use super::build_url;
use super::elements;
use super::parse_package_meta;
use super::parse_project_meta;
use super::parse_status_ok;
use super::repository_element;
use super::tag_attr;
use super::valid_obs_filename;
use super::valid_obs_name;
use super::DryRun;
use super::NamedRole;
use super::ObsClient;
use super::PackageMeta;
use super::ProjectMeta;
use super::Repository;
use crate::error::ErrorKind;
use crate::error::GeectlError;
use crate::xml::skip_xml_prolog;
use crate::xml::xml_escape;
use crate::xml::XmlDoc;

/// Set the text element `name` in `doc` to `value`, or remove it
/// when `value` is None.
fn set_text_element(
    doc: &mut XmlDoc,
    name: &str,
    value: &Option<String>,
) -> Result<(), GeectlError> {
    match value {
        Some(text) => {
            doc.replace_or_insert_element(name, &format!("<{name}>{}</{name}>", xml_escape(text)))
        }
        None => {
            doc.remove_elements(name);
            Ok(())
        }
    }
}

/// Replace the role elements in `doc` with `persons` and `groups`.
/// The caller compares first and only calls this when the roles
/// differ, so a no-change write keeps the roles where they were.
fn set_roles(
    doc: &mut XmlDoc,
    persons: &[NamedRole],
    groups: &[NamedRole],
) -> Result<(), GeectlError> {
    doc.remove_elements("person");
    doc.remove_elements("group");
    for person in persons {
        doc.append_element(&format!(
            "<person userid=\"{}\" role=\"{}\"/>",
            xml_escape(&person.name),
            xml_escape(&person.role),
        ))?;
    }
    for group in groups {
        doc.append_element(&format!(
            "<group groupid=\"{}\" role=\"{}\"/>",
            xml_escape(&group.name),
            xml_escape(&group.role),
        ))?;
    }
    Ok(())
}

/// Validate the roles of a meta write: every name must be an OBS
/// name and every role non-empty.
fn validate_roles(persons: &[NamedRole], groups: &[NamedRole]) -> Result<(), GeectlError> {
    for role in persons.iter().chain(groups.iter()) {
        if !valid_obs_name(&role.name) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS role name: {:?}", role.name),
            ));
        }
        if role.role.is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("role must not be empty for {:?}", role.name),
            ));
        }
    }
    Ok(())
}

/// Validate the repositories of a project meta write: each name,
/// architecture, and path entry must be a valid OBS name, and no
/// name may repeat. Deliberately lenient about shape: valid server
/// state can have no architectures or no paths (a hostsystem-only
/// path parses to no paths; a download-only repository has no
/// architectures), so this write-path check requires neither. The
/// add path keeps its own stricter check.
fn validate_repositories(repositories: &[Repository]) -> Result<(), GeectlError> {
    let mut names = Vec::new();
    for repository in repositories {
        if !valid_obs_name(&repository.name) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS repository name: {:?}", repository.name),
            ));
        }
        for arch in &repository.arch {
            if !valid_obs_name(arch) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS architecture name: {arch:?}"),
                ));
            }
        }
        for (project, repo) in &repository.paths {
            if !valid_obs_name(project) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS project name: {project:?}"),
                ));
            }
            if !valid_obs_name(repo) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS repository name: {repo:?}"),
                ));
            }
        }
        if names.contains(&repository.name) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("duplicate repository {:?} in project meta", repository.name),
            ));
        }
        names.push(repository.name.clone());
    }
    Ok(())
}

/// The repository settings a meta write does not model, as named in
/// the refusal: the `rebuild`, `block`, and `linkedbuild`
/// attributes and the `<download>`, `<releasetarget>`, and
/// `<hostsystem>` children.
fn unmodeled_repository_content(open_tag: &str, inner: &str) -> Vec<&'static str> {
    let mut found = Vec::new();
    if tag_attr(open_tag, "rebuild").is_some() {
        found.push("rebuild attribute");
    }
    if tag_attr(open_tag, "block").is_some() {
        found.push("block attribute");
    }
    if tag_attr(open_tag, "linkedbuild").is_some() {
        found.push("linkedbuild attribute");
    }
    for (child, label) in [
        ("download", "download elements"),
        ("releasetarget", "releasetarget elements"),
        ("hostsystem", "hostsystem elements"),
    ] {
        if elements(inner, child)
            .map(|found| !found.is_empty())
            .unwrap_or(false)
        {
            found.push(label);
        }
    }
    found
}

/// Refuse a repository rewrite when the fetched document carries
/// repository settings the writer does not model: rendering the
/// repositories from the typed meta would silently delete them.
/// Runs only when the repositories would actually change — a write
/// that keeps them is an in-place edit and drops nothing.
fn refuse_unmodeled_repositories(body: &str) -> Result<(), GeectlError> {
    let body = skip_xml_prolog(body).map_err(|_| {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS response: malformed project meta",
        )
    })?;
    for (open_tag, inner) in elements(body, "repository")? {
        let unmodeled = unmodeled_repository_content(&open_tag, &inner);
        if !unmodeled.is_empty() {
            let name = tag_attr(&open_tag, "name").unwrap_or_default();
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "repository {name:?} carries settings this writer does not model ({}); refusing to rewrite its repositories",
                    unmodeled.join(", ")
                ),
            ));
        }
    }
    Ok(())
}

/// The title, URL, and description read from a spec file.
struct SpecMeta {
    title: Option<String>,
    url: Option<String>,
    description: Option<String>,
}

/// Read the title, URL, and description from a spec file, like osc's
/// `read_meta_from_spec`: the first `Summary:` and `Url:` lines
/// (case insensitive) and the first bare `%description` section,
/// whose lines run until a line starting with `%`. An absent or
/// empty value clears the field, like osc.
fn parse_spec_meta(spec: &str) -> SpecMeta {
    let mut title = None;
    let mut url = None;
    for line in spec.lines() {
        if title.is_none() {
            title = spec_tag(line, "Summary");
        }
        if url.is_none() {
            url = spec_tag(line, "Url");
        }
    }
    let mut description = None;
    let mut in_description = false;
    let mut collected = Vec::new();
    for line in spec.lines() {
        if !in_description {
            if is_bare_section(line, "%description") {
                in_description = true;
            }
        } else if line.starts_with('%') {
            break;
        } else {
            collected.push(line);
        }
    }
    if in_description {
        description = Some(collected.join("\n").trim().to_string());
    }
    SpecMeta {
        title: title.filter(|v| !v.is_empty()),
        url: url.filter(|v| !v.is_empty()),
        description: description.filter(|v| !v.is_empty()),
    }
}

/// The value of a spec tag on `line` (`Summary: value`, whitespace
/// allowed before the colon, case insensitive), or None when the
/// line is not that tag. The tag must start the line, like osc's
/// `^Summary` match.
fn spec_tag(line: &str, tag: &str) -> Option<String> {
    let head = line.get(..tag.len())?;
    if !head.eq_ignore_ascii_case(tag) {
        return None;
    }
    let rest = line[tag.len()..].trim_start();
    let value = rest.strip_prefix(':')?;
    Some(value.trim().to_string())
}

/// True when `line` is exactly the bare section header `section`
/// (trailing whitespace allowed), case insensitive: `%description`
/// starts the section, `%description -n foo` does not.
fn is_bare_section(line: &str, section: &str) -> bool {
    line.trim_end().eq_ignore_ascii_case(section)
}

impl<'a> ObsClient<'a> {
    /// Write the project meta of `project` (`PUT
    /// /source/<project>/_meta` with the meta edited in place):
    /// title, description, URL, scmsync, devel, persons, groups, and
    /// repositories. The fetched document is edited in place, so
    /// settings outside the typed meta (lock, publish, build flags)
    /// survive untouched; nothing is PUT when nothing changed. Only
    /// the fields that differ are touched, so an unchanged write
    /// keeps the document order it fetched. A write that leaves
    /// `scmsync` set has a server side effect: the server declines
    /// the project's open requests and cleans up its packages, even
    /// when the write changed something else. Requires credentials;
    /// without them the call fails loudly before any request. In
    /// dry-run the write is recorded in the plan and nothing is
    /// written — the guard runs before the credentials check, like
    /// `create_package`.
    ///
    /// An agent writes project meta to correct a project's title or
    /// description, point it at its git source, or change who
    /// maintains it.
    pub fn set_project_meta(
        &self,
        project: &str,
        meta: &ProjectMeta,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        if meta.name != project {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("project meta is for {:?}, not {project:?}", meta.name),
            ));
        }
        validate_roles(&meta.persons, &meta.groups)?;
        validate_repositories(&meta.repositories)?;
        if !dry_run.allow(&format!("set project meta of {project}")) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot set project meta without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(&self.base, &["source", project, "_meta"], &[]);
        let body = self.http.get_with(&url, &self.credentials)?;
        let current = parse_project_meta(&body)?;
        let mut doc = XmlDoc::parse(&body)?;
        // The document as fetched, for the no-change check below:
        // comparing parsed documents (not rendered text) means a
        // prolog in the fetched meta, or a reorder from an edit that
        // changes nothing, does not read as a change.
        let before = doc.clone();
        if current.title != meta.title {
            set_text_element(&mut doc, "title", &meta.title)?;
        }
        if current.description != meta.description {
            set_text_element(&mut doc, "description", &meta.description)?;
        }
        if current.url != meta.url {
            set_text_element(&mut doc, "url", &meta.url)?;
        }
        if current.scmsync != meta.scmsync {
            set_text_element(&mut doc, "scmsync", &meta.scmsync)?;
        }
        if current.devel != meta.devel {
            match &meta.devel {
                Some(devel) => doc.replace_or_insert_element(
                    "devel",
                    &format!("<devel project=\"{}\"/>", xml_escape(devel)),
                )?,
                None => doc.remove_elements("devel"),
            }
        }
        if current.persons != meta.persons || current.groups != meta.groups {
            set_roles(&mut doc, &meta.persons, &meta.groups)?;
        }
        if current.repositories != meta.repositories {
            refuse_unmodeled_repositories(&body)?;
            doc.remove_elements("repository");
            for repository in &meta.repositories {
                doc.append_element(&repository_element(repository))?;
            }
        }
        // No change, no PUT: writing the meta that is already there
        // writes nothing. The comparison is on the parsed documents,
        // so a prolog in the fetched meta does not read as a change.
        if doc == before {
            return Ok(());
        }
        self.put_meta_doc(
            &url,
            &doc.render(),
            &format!("project meta for project {project}"),
        )
    }

    /// Write the package meta of `package` in `project` (`PUT
    /// /source/<project>/<package>/_meta` with the meta edited in
    /// place): title, description, URL, scmsync, release name, devel,
    /// persons, and groups. Same in-place terms as
    /// [`ObsClient::set_project_meta`]: settings outside the typed
    /// meta survive untouched, and nothing is PUT when nothing
    /// changed.
    ///
    /// An agent writes package meta to correct a package's title or
    /// description, point it at its git source, or change who
    /// maintains it. The devel package attribute is omitted when the
    /// devel package keeps this package's name, like
    /// `set_devel_project`.
    pub fn set_package_meta(
        &self,
        project: &str,
        package: &str,
        meta: &PackageMeta,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if meta.name != package || meta.project != project {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "package meta is for {:?}/{:?}, not {project:?}/{package:?}",
                    meta.project, meta.name
                ),
            ));
        }
        validate_roles(&meta.persons, &meta.groups)?;
        if !dry_run.allow(&format!("set package meta of {project}/{package}")) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot set package meta without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(&self.base, &["source", project, package, "_meta"], &[]);
        let body = self.http.get_with(&url, &self.credentials)?;
        self.write_package_meta(project, package, &body, meta)
    }

    /// Edit a fetched package meta document in place and PUT it back
    /// when anything changed: the shared core of
    /// `set_package_meta` and `update_package_meta_from_spec`.
    fn write_package_meta(
        &self,
        project: &str,
        package: &str,
        body: &str,
        meta: &PackageMeta,
    ) -> Result<(), GeectlError> {
        let current = parse_package_meta(body)?;
        let mut doc = XmlDoc::parse(body)?;
        // The document as fetched, for the no-change check below:
        // comparing parsed documents (not rendered text) means a
        // prolog in the fetched meta, or a reorder from an edit that
        // changes nothing, does not read as a change.
        let before = doc.clone();
        if current.title != meta.title {
            set_text_element(&mut doc, "title", &meta.title)?;
        }
        if current.description != meta.description {
            set_text_element(&mut doc, "description", &meta.description)?;
        }
        if current.url != meta.url {
            set_text_element(&mut doc, "url", &meta.url)?;
        }
        if current.scmsync != meta.scmsync {
            set_text_element(&mut doc, "scmsync", &meta.scmsync)?;
        }
        if current.releasename != meta.releasename {
            set_text_element(&mut doc, "releasename", &meta.releasename)?;
        }
        if current.devel != meta.devel {
            match &meta.devel {
                Some((devel_project, devel_package)) => {
                    let element = if devel_package == package {
                        format!("<devel project=\"{}\"/>", xml_escape(devel_project))
                    } else {
                        format!(
                            "<devel project=\"{}\" package=\"{}\"/>",
                            xml_escape(devel_project),
                            xml_escape(devel_package),
                        )
                    };
                    doc.replace_or_insert_element("devel", &element)?;
                }
                None => doc.remove_elements("devel"),
            }
        }
        if current.persons != meta.persons || current.groups != meta.groups {
            set_roles(&mut doc, &meta.persons, &meta.groups)?;
        }
        // No change, no PUT: writing the meta that is already there
        // writes nothing. The comparison is on the parsed documents,
        // so a prolog in the fetched meta does not read as a change.
        if doc == before {
            return Ok(());
        }
        let url = build_url(&self.base, &["source", project, package, "_meta"], &[]);
        self.put_meta_doc(
            &url,
            &doc.render(),
            &format!("package meta for package {project}/{package}"),
        )
    }

    /// Update the package meta of `package` in `project` from its
    /// spec file (`updatepacmetafromspec`): the spec's `Summary`
    /// becomes the title, its `Url` the URL, and its `%description`
    /// section the description — like osc. The spec file is found by
    /// listing the package files when exactly one `*.spec` exists;
    /// otherwise the caller names it with `spec`. The listing and
    /// the spec read go through the authenticated source endpoints,
    /// so restricted projects and the `_project` pseudo-package
    /// work: the public source routes cannot address them. The other meta
    /// fields (release name, devel, persons, groups, scmsync) are
    /// kept as they are, and nothing is PUT when nothing changed.
    /// Requires credentials; without them the call fails loudly
    /// before any request. In dry-run the update is recorded in the
    /// plan and nothing is written — the guard runs before the
    /// credentials check, like `create_package`.
    ///
    /// An agent updates package meta from the spec after editing the
    /// spec, so the meta shown on the server matches the package
    /// without hand-editing XML.
    pub fn update_package_meta_from_spec(
        &self,
        project: &str,
        package: &str,
        spec: Option<&str>,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if let Some(spec) = spec {
            if !valid_obs_filename(spec) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS file name: {spec:?}"),
                ));
            }
        }
        if !dry_run.allow(&format!(
            "update package meta of {project}/{package} from spec"
        )) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot update package meta from spec without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let spec = match spec {
            Some(spec) => spec.to_string(),
            None => self.discover_spec_file(project, package)?,
        };
        let text = self.authed_cat(project, package, &spec)?;
        let parsed = parse_spec_meta(&text);
        let url = build_url(&self.base, &["source", project, package, "_meta"], &[]);
        let body = self.http.get_with(&url, &self.credentials)?;
        let mut meta = parse_package_meta(&body)?;
        meta.title = parsed.title;
        meta.url = parsed.url;
        meta.description = parsed.description;
        self.write_package_meta(project, package, &body, &meta)
    }

    /// The single `*.spec` file of a package: the spec file is found
    /// by listing the package files when exactly one exists.
    fn discover_spec_file(&self, project: &str, package: &str) -> Result<String, GeectlError> {
        let specs: Vec<String> = self
            .authed_directory_listing(project, package)?
            .entries
            .into_iter()
            .map(|file| file.name)
            .filter(|name| name.ends_with(".spec"))
            .collect();
        match specs.len() {
            1 => Ok(specs.into_iter().next().unwrap()),
            0 => Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("no spec file in {project}/{package}; name one"),
            )),
            _ => Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "multiple spec files in {project}/{package}: {}; name one",
                    specs.join(", ")
                ),
            )),
        }
    }

    /// PUT a rendered meta document back to the server, and fail
    /// closed when the server does not answer `<status code="ok"/>`.
    fn put_meta_doc(&self, url: &str, rendered: &str, what: &str) -> Result<(), GeectlError> {
        let body = self.http.put(url, rendered, &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            let preview = crate::debug::format_body_preview(&body);
            let received = if preview.is_empty() {
                "<empty response>"
            } else {
                preview.as_str()
            };
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS {what} response; received: {received}"),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use crate::obs::test_support::{basic_credentials, FakeHttp};

    const PROJECT_META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><description>Old description</description><lock><enable/></lock><person userid=\"alice\" role=\"maintainer\"/><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><path project=\"openSUSE:Factory\" repository=\"snapshot\"/></repository><build><disable/></build></project>";

    const PACKAGE_META: &str = "<package name=\"foo\" project=\"home:scarabeus_iv\"><title>Old title</title><description>Old description</description><url>https://example.com/old</url><scmsync>https://github.com/openSUSE/foo.git</scmsync><releasename>foo</releasename><devel project=\"devel:languages:python\"/><person userid=\"alice\" role=\"maintainer\"/><lock><enable/></lock></package>";

    fn project_client(http: &FakeHttp) -> ObsClient<'_> {
        ObsClient::with_credentials(http, basic_credentials()).unwrap()
    }

    #[test]
    fn set_project_meta_puts_only_the_changed_title_and_keeps_the_rest() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                PROJECT_META,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                "<status code=\"ok\" />",
            );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META).unwrap();
        meta.title = Some("New title".to_string());
        assert!(client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].0,
            "https://api.opensuse.org/source/home:scarabeus_iv/_meta"
        );
        assert!(puts[0].2);
        let rendered = &puts[0].1;
        assert!(rendered.contains("<title>New title</title>"));
        assert!(rendered.contains("<description>Old description</description>"));
        assert!(rendered.contains("<lock><enable/></lock>"));
        assert!(rendered.contains("<person userid=\"alice\" role=\"maintainer\"/>"));
        assert!(rendered.contains("<repository name=\"openSUSE_Tumbleweed\">"));
        assert!(rendered.contains("<build><disable/></build>"));
    }

    const PROJECT_META_WITH_UNMODELED_REPOSITORY: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\" rebuild=\"local\"><arch>x86_64</arch><path project=\"openSUSE:Factory\" repository=\"snapshot\"/><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpm-md\"/></repository></project>";

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_that_would_drop_settings() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
            PROJECT_META_WITH_UNMODELED_REPOSITORY,
        );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META_WITH_UNMODELED_REPOSITORY).unwrap();
        meta.repositories = vec![Repository {
            name: "openSUSE_Leap_15.6".to_string(),
            arch: vec!["x86_64".to_string()],
            paths: vec![("openSUSE:Leap:15.6".to_string(), "standard".to_string())],
        }];
        let err = client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_project_meta_keeps_unmodeled_repository_settings_when_they_do_not_change() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                PROJECT_META_WITH_UNMODELED_REPOSITORY,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                "<status code=\"ok\" />",
            );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META_WITH_UNMODELED_REPOSITORY).unwrap();
        meta.title = Some("New title".to_string());
        assert!(client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert!(puts[0].1.contains("rebuild=\"local\""));
        assert!(puts[0].1.contains("<download"));
    }

    #[test]
    fn set_project_meta_accepts_a_repository_without_architectures_or_paths() {
        const META_HOSTSYSTEM_ONLY: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"standard\"><hostsystem project=\"openSUSE:Factory\" repository=\"snapshot\"/></repository></project>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                META_HOSTSYSTEM_ONLY,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                "<status code=\"ok\" />",
            );
        let client = project_client(&http);
        let mut meta = parse_project_meta(META_HOSTSYSTEM_ONLY).unwrap();
        assert!(meta.repositories[0].arch.is_empty());
        assert!(meta.repositories[0].paths.is_empty());
        meta.title = Some("New title".to_string());
        assert!(client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .is_ok());
        assert_eq!(http.put_calls().len(), 1);
    }

    #[test]
    fn set_project_meta_unchanged_writes_nothing() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
            PROJECT_META,
        );
        let client = project_client(&http);
        let meta = parse_project_meta(PROJECT_META).unwrap();
        assert!(client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .is_ok());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_project_meta_replaces_roles_and_repositories() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                PROJECT_META,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                "<status code=\"ok\" />",
            );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META).unwrap();
        meta.persons = vec![NamedRole {
            name: "bob".to_string(),
            role: "bugowner".to_string(),
        }];
        meta.groups = vec![NamedRole {
            name: "factory-maintainers".to_string(),
            role: "maintainer".to_string(),
        }];
        meta.repositories = vec![Repository {
            name: "openSUSE_Leap_15.6".to_string(),
            arch: vec!["x86_64".to_string()],
            paths: vec![("openSUSE:Leap:15.6".to_string(), "standard".to_string())],
        }];
        assert!(client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        let rendered = &puts[0].1;
        assert!(rendered.contains("<person userid=\"bob\" role=\"bugowner\"/>"));
        assert!(!rendered.contains("alice"));
        assert!(rendered.contains("<group groupid=\"factory-maintainers\" role=\"maintainer\"/>"));
        assert!(rendered.contains("<repository name=\"openSUSE_Leap_15.6\">"));
        assert!(!rendered.contains("openSUSE_Tumbleweed"));
        assert!(rendered.contains("<lock><enable/></lock>"));
    }

    #[test]
    fn set_project_meta_dry_run_records_the_write_and_sends_nothing() {
        let http = FakeHttp::new();
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META).unwrap();
        meta.title = Some("New title".to_string());
        let dry = DryRun::new(true);
        assert!(client
            .set_project_meta("home:scarabeus_iv", &meta, &dry)
            .is_ok());
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec!["[dry-run] would: set project meta of home:scarabeus_iv".to_string()]
        );
    }

    #[test]
    fn set_project_meta_rejects_a_mismatched_meta_and_missing_credentials_without_http() {
        let http = FakeHttp::new();
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META).unwrap();
        meta.name = "home:other".to_string();
        let err = client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
        use crate::auth::Credentials;
        let client = ObsClient::with_credentials(&http, Credentials::None).unwrap();
        let meta = parse_project_meta(PROJECT_META).unwrap();
        let err = client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn set_package_meta_puts_the_changed_fields_and_keeps_the_lock() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta",
                PACKAGE_META,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta",
                "<status code=\"ok\" />",
            );
        let client = project_client(&http);
        let mut meta = parse_package_meta(PACKAGE_META).unwrap();
        meta.title = Some("New title".to_string());
        meta.url = None;
        meta.devel = Some(("devel:languages:python".to_string(), "bar".to_string()));
        assert!(client
            .set_package_meta("home:scarabeus_iv", "foo", &meta, &DryRun::new(false))
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        let rendered = &puts[0].1;
        assert!(rendered.contains("<title>New title</title>"));
        assert!(!rendered.contains("<url>"));
        assert!(rendered.contains("<devel project=\"devel:languages:python\" package=\"bar\"/>"));
        assert!(rendered.contains("<scmsync>https://github.com/openSUSE/foo.git</scmsync>"));
        assert!(rendered.contains("<releasename>foo</releasename>"));
        assert!(rendered.contains("<lock><enable/></lock>"));
    }

    #[test]
    fn set_package_meta_devel_keeping_the_package_name_omits_the_package_attribute() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta",
                PACKAGE_META,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta",
                "<status code=\"ok\" />",
            );
        let client = project_client(&http);
        let mut meta = parse_package_meta(PACKAGE_META).unwrap();
        meta.devel = Some(("devel:other".to_string(), "foo".to_string()));
        assert!(client
            .set_package_meta("home:scarabeus_iv", "foo", &meta, &DryRun::new(false))
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert!(puts[0].1.contains("<devel project=\"devel:other\"/>"));
        assert!(!puts[0].1.contains("package=\"foo\""));
    }

    #[test]
    fn set_package_meta_put_body_round_trips_as_a_no_op_write() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta",
                PACKAGE_META,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta",
                "<status code=\"ok\" />",
            );
        let client = project_client(&http);
        let mut meta = parse_package_meta(PACKAGE_META).unwrap();
        meta.devel = Some(("devel:other".to_string(), "foo".to_string()));
        assert!(client
            .set_package_meta("home:scarabeus_iv", "foo", &meta, &DryRun::new(false))
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert!(puts[0].1.contains("<devel project=\"devel:other\"/>"));
        // The written document, read back, is the meta already
        // there: the same write is a no-op.
        let written = puts[0].1.clone();
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta",
            &written,
        );
        let client = project_client(&http);
        assert!(client
            .set_package_meta("home:scarabeus_iv", "foo", &meta, &DryRun::new(false))
            .is_ok());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_package_meta_unchanged_writes_nothing() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta",
            PACKAGE_META,
        );
        let client = project_client(&http);
        let meta = parse_package_meta(PACKAGE_META).unwrap();
        assert!(client
            .set_package_meta("home:scarabeus_iv", "foo", &meta, &DryRun::new(false))
            .is_ok());
        assert!(http.put_calls().is_empty());
    }

    const SPEC: &str = "Name:           foo\nVersion:        1.0\nSummary:        A foo package\nLicense:        MIT\nUrl:            https://example.com/foo\n\n%description\nFoo does foo things.\n\nIt does them well.\n\n%package -n foo-devel\nSummary:        Development files for foo\n\n%description -n foo-devel\nDevelopment headers.\n\n%prep\n%autosetup\n";

    const DIRECTORY_ONE_SPEC: &str = "<directory name=\"foo\"><entry name=\"foo.spec\" md5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" size=\"1234\" mtime=\"1700000000\"/><entry name=\"foo.changes\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"567\" mtime=\"1700000001\"/></directory>";

    fn update_client(http: &FakeHttp) -> ObsClient<'_> {
        ObsClient::with_credentials(http, basic_credentials()).unwrap()
    }

    fn update_stubs(http: FakeHttp, spec: &str) -> FakeHttp {
        http.with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/foo",
            DIRECTORY_ONE_SPEC,
        )
        .with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/foo/foo.spec",
            spec,
        )
        .with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta",
            PACKAGE_META,
        )
        .with_put(
            "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta",
            "<status code=\"ok\" />",
        )
    }

    #[test]
    fn parse_spec_meta_reads_the_first_summary_url_and_bare_description() {
        let parsed = parse_spec_meta(SPEC);
        assert_eq!(parsed.title.as_deref(), Some("A foo package"));
        assert_eq!(parsed.url.as_deref(), Some("https://example.com/foo"));
        assert_eq!(
            parsed.description.as_deref(),
            Some("Foo does foo things.\n\nIt does them well.")
        );
    }

    #[test]
    fn parse_spec_meta_absent_or_empty_values_clear_the_field() {
        let parsed = parse_spec_meta("Name: foo\nVersion: 1.0\n");
        assert_eq!(parsed.title, None);
        assert_eq!(parsed.url, None);
        assert_eq!(parsed.description, None);
        let parsed = parse_spec_meta("Summary:\nUrl:  \n%description\n");
        assert_eq!(parsed.title, None);
        assert_eq!(parsed.url, None);
        assert_eq!(parsed.description, None);
    }

    #[test]
    fn parse_spec_meta_tags_are_case_insensitive_and_allow_space_before_the_colon() {
        let parsed = parse_spec_meta("summary:   lower case\nURL : https://example.com/x\n");
        assert_eq!(parsed.title.as_deref(), Some("lower case"));
        assert_eq!(parsed.url.as_deref(), Some("https://example.com/x"));
    }

    #[test]
    fn parse_spec_meta_a_named_description_section_does_not_start_the_section() {
        let parsed =
            parse_spec_meta("%description -n foo\nNamed only.\n%description\nBare one.\n%prep\n");
        assert_eq!(parsed.description.as_deref(), Some("Bare one."));
    }

    #[test]
    fn update_package_meta_from_spec_discovers_the_spec_and_puts_the_spec_fields() {
        let http = update_stubs(FakeHttp::new(), SPEC);
        let client = update_client(&http);
        assert!(client
            .update_package_meta_from_spec("home:scarabeus_iv", "foo", None, &DryRun::new(false))
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        let rendered = &puts[0].1;
        assert!(rendered.contains("<title>A foo package</title>"));
        assert!(rendered.contains("<url>https://example.com/foo</url>"));
        assert!(rendered
            .contains("<description>Foo does foo things.\n\nIt does them well.</description>"));
        assert!(rendered.contains("<releasename>foo</releasename>"));
        assert!(rendered.contains("<devel project=\"devel:languages:python\"/>"));
        assert!(rendered.contains("<person userid=\"alice\" role=\"maintainer\"/>"));
        assert!(rendered.contains("<lock><enable/></lock>"));
        assert!(rendered.contains("<scmsync>https://github.com/openSUSE/foo.git</scmsync>"));
    }

    #[test]
    fn update_package_meta_from_spec_with_a_named_spec_skips_the_listing() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo/other.spec",
                SPEC,
            )
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta",
                PACKAGE_META,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta",
                "<status code=\"ok\" />",
            );
        let client = update_client(&http);
        assert!(client
            .update_package_meta_from_spec(
                "home:scarabeus_iv",
                "foo",
                Some("other.spec"),
                &DryRun::new(false)
            )
            .is_ok());
        assert_eq!(http.put_calls().len(), 1);
        assert_eq!(http.get_calls().len(), 2);
    }

    #[test]
    fn update_package_meta_from_spec_unchanged_writes_nothing() {
        let spec =
            "Summary: Old title\nUrl: https://example.com/old\n%description\nOld description\n";
        let http = update_stubs(FakeHttp::new(), spec);
        let client = update_client(&http);
        assert!(client
            .update_package_meta_from_spec("home:scarabeus_iv", "foo", None, &DryRun::new(false))
            .is_ok());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn update_package_meta_from_spec_refuses_zero_or_many_specs() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/foo",
            "<directory name=\"foo\"><entry name=\"foo.changes\"/></directory>",
        );
        let client = update_client(&http);
        let err = client
            .update_package_meta_from_spec("home:scarabeus_iv", "foo", None, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/foo",
            "<directory name=\"foo\"><entry name=\"foo.spec\"/><entry name=\"bar.spec\"/></directory>",
        );
        let client = update_client(&http);
        let err = client
            .update_package_meta_from_spec("home:scarabeus_iv", "foo", None, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn update_package_meta_from_spec_dry_run_records_the_update_and_sends_nothing() {
        let http = FakeHttp::new();
        let client = update_client(&http);
        let dry = DryRun::new(true);
        assert!(client
            .update_package_meta_from_spec("home:scarabeus_iv", "foo", None, &dry)
            .is_ok());
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec![
                "[dry-run] would: update package meta of home:scarabeus_iv/foo from spec"
                    .to_string()
            ]
        );
    }

    #[test]
    fn update_package_meta_from_spec_requires_credentials() {
        let http = FakeHttp::new();
        use crate::auth::Credentials;
        let client = ObsClient::with_credentials(&http, Credentials::None).unwrap();
        let err = client
            .update_package_meta_from_spec("home:scarabeus_iv", "foo", None, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }
}
