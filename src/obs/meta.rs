use super::build_url;
use super::elements;
use super::parse_package_meta;
use super::parse_project_meta;
use super::parse_status_ok;
use super::repository_element;
use super::tag_attr;
use super::tag_attr_names;
use super::valid_obs_filename;
use super::valid_obs_name;
use super::validate_repository_settings;
use super::DryRun;
use super::NamedRole;
use super::ObsClient;
use super::PackageMeta;
use super::ProjectMeta;
use super::Repository;
use super::RoleGrant;
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

/// The roles the server grants on projects and packages.
const LOCAL_ROLES: [&str; 5] = ["maintainer", "bugowner", "reviewer", "downloader", "reader"];

/// Validate a role for a direct grant on `what`: it must be one of
/// the local roles the server grants on projects and packages.
fn validate_role(role: &str, what: &str) -> Result<(), GeectlError> {
    if LOCAL_ROLES.contains(&role) {
        Ok(())
    } else {
        Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!(
                "invalid OBS role {role:?} for {what}; roles are {}",
                LOCAL_ROLES.join(", ")
            ),
        ))
    }
}

/// Validate the grantees of a role change in `grant`: at least one
/// of `person` and `group` must name someone, and every name must be
/// a valid OBS name. Returns the grantee phrase for the dry-run plan.
fn grantee_phrase(grant: &RoleGrant) -> Result<String, GeectlError> {
    for (label, name) in [
        ("person", grant.person.as_deref()),
        ("group", grant.group.as_deref()),
    ] {
        if let Some(name) = name {
            if !valid_obs_name(name) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {name:?}"),
                ));
            }
        }
    }
    match (grant.person.as_deref(), grant.group.as_deref()) {
        (Some(person), Some(group)) => Ok(format!("person {person} and group {group}")),
        (Some(person), None) => Ok(format!("person {person}")),
        (None, Some(group)) => Ok(format!("group {group}")),
        (None, None) => Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "a role change names no person or group: name a person or a group",
        )),
    }
}

/// Add the role in `grant` for its person and/or group to the role
/// lists. A pair already present stays where it is; a missing pair
/// is appended.
fn add_role_to_lists(
    persons: &[NamedRole],
    groups: &[NamedRole],
    grant: &RoleGrant,
) -> (Vec<NamedRole>, Vec<NamedRole>) {
    let role = grant.role.as_str();
    let mut persons = persons.to_vec();
    let mut groups = groups.to_vec();
    if let Some(name) = grant.person.as_deref() {
        if !persons
            .iter()
            .any(|entry| entry.name == name && entry.role == role)
        {
            persons.push(NamedRole {
                name: name.to_string(),
                role: role.to_string(),
            });
        }
    }
    if let Some(name) = grant.group.as_deref() {
        if !groups
            .iter()
            .any(|entry| entry.name == name && entry.role == role)
        {
            groups.push(NamedRole {
                name: name.to_string(),
                role: role.to_string(),
            });
        }
    }
    (persons, groups)
}

/// Remove the role in `grant` for its person and/or group from the
/// role lists. A pair that is not present is an error, never a
/// silent no-op.
fn remove_role_from_lists(
    persons: &[NamedRole],
    groups: &[NamedRole],
    grant: &RoleGrant,
    what: &str,
) -> Result<(Vec<NamedRole>, Vec<NamedRole>), GeectlError> {
    let role = grant.role.as_str();
    let mut persons = persons.to_vec();
    let mut groups = groups.to_vec();
    if let Some(name) = grant.person.as_deref() {
        if !persons
            .iter()
            .any(|entry| entry.name == name && entry.role == role)
        {
            return Err(GeectlError::new(
                ErrorKind::NotFound,
                format!("person {name} with role {role} not found on {what}"),
            ));
        }
        persons.retain(|entry| !(entry.name == name && entry.role == role));
    }
    if let Some(name) = grant.group.as_deref() {
        if !groups
            .iter()
            .any(|entry| entry.name == name && entry.role == role)
        {
            return Err(GeectlError::new(
                ErrorKind::NotFound,
                format!("group {name} with role {role} not found on {what}"),
            ));
        }
        groups.retain(|entry| !(entry.name == name && entry.role == role));
    }
    Ok((persons, groups))
}

/// Validate the repositories of a project meta write: each name,
/// architecture, and path entry must be a valid OBS name, each
/// repository's settings must be valid, and no name may repeat.
/// Deliberately lenient about shape: valid server
/// state can have no architectures or no paths (a hostsystem-only
/// repository has neither), so this write-path check requires
/// neither. The add path keeps its own stricter check.
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
        validate_repository_settings(repository)?;
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
/// the refusal: any attribute outside `name`, `rebuild`, `block`,
/// and `linkedbuild` on the repository itself; any child outside
/// `arch`, `path`, `download`, `releasetarget`, and `hostsystem` —
/// the expanded-repo `<base>` element, for one, is never part of a
/// project meta; any attribute outside the modeled set on a child
/// (`download`, `master`, `releasetarget`, `hostsystem`, `path`,
/// `arch`, `archfilter`, `pubkey`); any child of `<download>`
/// outside `archfilter`, `master`, and `pubkey`; a second
/// `<master>`, `<archfilter>`, or `<pubkey>` in one `<download>`;
/// and any content inside `<master>`, `<releasetarget>`,
/// `<hostsystem>`, or `<path>`, which carry attributes only. A child
/// missing a required value is malformed, not unmodeled: the parser
/// errors on it before this check runs.
fn unmodeled_repository_content(open_tag: &str, inner: &str) -> Result<Vec<String>, GeectlError> {
    let mut found = Vec::new();
    for attr in tag_attr_names(open_tag) {
        if !matches!(attr.as_str(), "name" | "rebuild" | "block" | "linkedbuild") {
            push_finding(&mut found, format!("{attr} attribute"));
        }
    }
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<repository .../>`: no children to check.
        return Ok(found);
    }
    // A comment or processing instruction inside the repository is
    // content the render drops: the typed model carries no opaque
    // entries, so a rewrite would delete it silently.
    if text_has_unquoted(inner, "<!--") {
        push_finding(&mut found, "XML comments in <repository>".to_string());
    }
    if text_has_unquoted(inner, "<?") {
        push_finding(
            &mut found,
            "processing instructions in <repository>".to_string(),
        );
    }
    let doc = XmlDoc::parse(&format!("<repository{open_tag}>{inner}</repository>"))?;
    for child in doc.child_names() {
        if !matches!(
            child,
            "arch" | "path" | "download" | "releasetarget" | "hostsystem"
        ) {
            push_finding(&mut found, format!("<{child}> elements"));
        }
    }
    for (download_tag, download_inner) in elements(inner, "download")? {
        for attr in tag_attr_names(&download_tag) {
            if !matches!(attr.as_str(), "arch" | "url" | "repotype") {
                push_finding(&mut found, format!("{attr} attribute on <download>"));
            }
        }
        if download_tag.trim_end().ends_with('/') {
            continue;
        }
        let download_doc = XmlDoc::parse(&format!(
            "<download{download_tag}>{download_inner}</download>"
        ))?;
        for child in ["master", "archfilter", "pubkey"] {
            let count = download_doc
                .child_names()
                .iter()
                .filter(|name| **name == child)
                .count();
            if count > 1 {
                push_finding(&mut found, format!("duplicate <{child}> in <download>"));
            }
        }
        for child in download_doc.child_names() {
            if !matches!(child, "archfilter" | "master" | "pubkey") {
                push_finding(&mut found, format!("<{child}> elements in <download>"));
            }
        }
        for (master_tag, master_inner) in elements(&download_inner, "master")? {
            if !master_inner.trim().is_empty() {
                push_finding(&mut found, "master elements with content".to_string());
            }
            for attr in tag_attr_names(&master_tag) {
                if !matches!(attr.as_str(), "url" | "sslfingerprint") {
                    push_finding(&mut found, format!("{attr} attribute on <master>"));
                }
            }
        }
        for name in ["archfilter", "pubkey"] {
            for (tag, _) in elements(&download_inner, name)? {
                for attr in tag_attr_names(&tag) {
                    push_finding(&mut found, format!("{attr} attribute on <{name}>"));
                }
            }
        }
    }
    for (target_tag, target_inner) in elements(inner, "releasetarget")? {
        if !target_inner.trim().is_empty() {
            push_finding(
                &mut found,
                "releasetarget elements with content".to_string(),
            );
        }
        for attr in tag_attr_names(&target_tag) {
            if !matches!(attr.as_str(), "project" | "repository" | "trigger") {
                push_finding(&mut found, format!("{attr} attribute on <releasetarget>"));
            }
        }
    }
    for (host_tag, host_inner) in elements(inner, "hostsystem")? {
        if !host_inner.trim().is_empty() {
            push_finding(&mut found, "hostsystem elements with content".to_string());
        }
        for attr in tag_attr_names(&host_tag) {
            if !matches!(attr.as_str(), "project" | "repository") {
                push_finding(&mut found, format!("{attr} attribute on <hostsystem>"));
            }
        }
    }
    for (path_tag, path_inner) in elements(inner, "path")? {
        if !path_inner.trim().is_empty() {
            push_finding(&mut found, "path elements with content".to_string());
        }
        for attr in tag_attr_names(&path_tag) {
            if !matches!(attr.as_str(), "project" | "repository") {
                push_finding(&mut found, format!("{attr} attribute on <path>"));
            }
        }
    }
    for (arch_tag, _) in elements(inner, "arch")? {
        for attr in tag_attr_names(&arch_tag) {
            push_finding(&mut found, format!("{attr} attribute on <arch>"));
        }
    }
    Ok(found)
}

/// True when `text` carries `marker` outside a quoted attribute
/// value: inside quotes the marker is literal text, outside it the
/// marker starts a comment or a processing instruction.
fn text_has_unquoted(text: &str, marker: &str) -> bool {
    let bytes = text.as_bytes();
    let mut in_quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = in_quote {
            if b == q {
                in_quote = None;
            }
            i += 1;
            continue;
        }
        if b == b'"' || b == b'\'' {
            in_quote = Some(b);
            i += 1;
            continue;
        }
        if text[i..].starts_with(marker) {
            return true;
        }
        i += 1;
    }
    false
}

/// Record a refusal reason once: the same dropped piece named twice
/// reads as two problems.
fn push_finding(found: &mut Vec<String>, finding: String) {
    if !found.contains(&finding) {
        found.push(finding);
    }
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
        let unmodeled = unmodeled_repository_content(&open_tag, &inner)?;
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
        self.write_project_meta(project, &body, meta)
    }

    /// Edit a fetched project meta document in place and PUT it back
    /// when anything changed: the shared core of
    /// `set_project_meta` and the role operations, which read the
    /// meta once and write from the same snapshot.
    fn write_project_meta(
        &self,
        project: &str,
        body: &str,
        meta: &ProjectMeta,
    ) -> Result<(), GeectlError> {
        let url = build_url(&self.base, &["source", project, "_meta"], &[]);
        let current = parse_project_meta(body)?;
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
            refuse_unmodeled_repositories(body)?;
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
    /// `set_package_meta`, `update_package_meta_from_spec`, and the
    /// package role operations, which read the meta once and write
    /// from the same snapshot.
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

    /// Read the project meta through the authenticated source route
    /// (`GET /source/<project>/_meta`), returning the fetched body
    /// with the parsed meta: the role operations write from the same
    /// snapshot they read, so a change between the read and the write
    /// cannot be overwritten.
    fn read_project_meta(&self, project: &str) -> Result<(String, ProjectMeta), GeectlError> {
        let url = build_url(&self.base, &["source", project, "_meta"], &[]);
        let body = self.http.get_with(&url, &self.credentials)?;
        let meta = parse_project_meta(&body)?;
        Ok((body, meta))
    }

    /// Read the package meta through the authenticated source route
    /// (`GET /source/<project>/<package>/_meta`), returning the
    /// fetched body with the parsed meta: the role operations write
    /// from the same snapshot they read, so a change between the read
    /// and the write cannot be overwritten.
    fn read_package_meta(
        &self,
        project: &str,
        package: &str,
    ) -> Result<(String, PackageMeta), GeectlError> {
        let url = build_url(&self.base, &["source", project, package, "_meta"], &[]);
        let body = self.http.get_with(&url, &self.credentials)?;
        let meta = parse_package_meta(&body)?;
        Ok((body, meta))
    }

    /// Grant the role in `grant` to its person and/or group on project `project`
    /// (direct grant: read the project meta, add the role, write it
    /// back). The role in `grant` must be one of the local roles the server
    /// grants on projects and packages: maintainer, bugowner,
    /// reviewer, downloader, reader. A pair already present is left
    /// alone, so granting a role that is already held writes
    /// nothing. Requires credentials; without them the call fails
    /// loudly before any request. In dry-run the grant is recorded
    /// in the plan and nothing is written — the guard runs before
    /// the credentials check, like `create_package`.
    ///
    /// An agent grants a role directly when the grant needs no
    /// review: the request-based `file_add_role` asks for a role
    /// instead.
    pub fn role_add_project(
        &self,
        project: &str,
        grant: &RoleGrant,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        let role = grant.role.as_str();
        validate_role(role, &format!("project {project}"))?;
        let grantees = grantee_phrase(grant)?;
        if !dry_run.allow(&format!(
            "add {role} role for {grantees} on project {project}"
        )) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!("cannot add a role on project {project} without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)"),
            ));
        }
        let (body, mut meta) = self.read_project_meta(project)?;
        let (persons, groups) = add_role_to_lists(&meta.persons, &meta.groups, grant);
        meta.persons = persons;
        meta.groups = groups;
        self.write_project_meta(project, &body, &meta)
    }

    /// Grant the role in `grant` to its person and/or group on package `package`
    /// in project `project` (direct grant: read the package meta,
    /// add the role, write it back). Same terms as
    /// [`ObsClient::role_add_project`].
    ///
    /// An agent grants a package role directly when the grant needs
    /// no review: the request-based `file_add_role` asks for a role
    /// instead.
    pub fn role_add_package(
        &self,
        project: &str,
        package: &str,
        grant: &RoleGrant,
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
        let role = grant.role.as_str();
        validate_role(role, &format!("package {project}/{package}"))?;
        let grantees = grantee_phrase(grant)?;
        if !dry_run.allow(&format!(
            "add {role} role for {grantees} on package {project}/{package}"
        )) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!("cannot add a role on package {project}/{package} without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)"),
            ));
        }
        let (body, mut meta) = self.read_package_meta(project, package)?;
        let (persons, groups) = add_role_to_lists(&meta.persons, &meta.groups, grant);
        meta.persons = persons;
        meta.groups = groups;
        self.write_package_meta(project, package, &body, &meta)
    }

    /// Remove the role in `grant` from its person and/or group on project
    /// `project` (direct removal: read the project meta, remove the
    /// role, write it back). Force-confirmed: without `force` the
    /// call refuses — removing access is destructive. A pair that is
    /// not present is an error, never a silent no-op. Removing the
    /// last maintainer is refused even with force: grant a maintainer
    /// role to someone else first, or ask an administrator to restore
    /// one — a project with no maintainer leaves a non-admin user
    /// unable to write it. The zero-maintainer state itself is
    /// reachable only by writing the project meta directly with
    /// [`ObsClient::set_project_meta`], which this guard does not
    /// block. Requires credentials;
    /// without them the call fails loudly before any request. In
    /// dry-run the removal is recorded in the plan and nothing is
    /// written — the guard runs before the credentials check, like
    /// `create_package`.
    ///
    /// An agent removes a role directly when the removal needs no
    /// review.
    pub fn role_remove_project(
        &self,
        project: &str,
        grant: &RoleGrant,
        force: bool,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        let role = grant.role.as_str();
        validate_role(role, &format!("project {project}"))?;
        let grantees = grantee_phrase(grant)?;
        if !force {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("refusing to remove {role} role for {grantees} from project {project} without force: removing access is destructive; pass force to confirm"),
            ));
        }
        if !dry_run.allow(&format!(
            "remove {role} role for {grantees} from project {project}"
        )) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!("cannot remove a role on project {project} without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)"),
            ));
        }
        let (body, mut meta) = self.read_project_meta(project)?;
        let (persons, groups) = remove_role_from_lists(
            &meta.persons,
            &meta.groups,
            grant,
            &format!("project {project}"),
        )?;
        if meta
            .persons
            .iter()
            .chain(meta.groups.iter())
            .any(|entry| entry.role == "maintainer")
            && !persons
                .iter()
                .chain(groups.iter())
                .any(|entry| entry.role == "maintainer")
        {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("refusing to remove the last maintainer of project {project}: the project would have no maintainer left; grant a maintainer role to someone else first"),
            ));
        }
        meta.persons = persons;
        meta.groups = groups;
        self.write_project_meta(project, &body, &meta)
    }

    /// Remove the role in `grant` from its person and/or group on package
    /// `package` in project `project` (direct removal: read the
    /// package meta, remove the role, write it back). Same terms as
    /// [`ObsClient::role_remove_project`], except removing the last
    /// package maintainer is allowed: the project maintainers still
    /// cover the package.
    ///
    /// An agent removes a package role directly when the removal
    /// needs no review.
    pub fn role_remove_package(
        &self,
        project: &str,
        package: &str,
        grant: &RoleGrant,
        force: bool,
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
        let role = grant.role.as_str();
        validate_role(role, &format!("package {project}/{package}"))?;
        let grantees = grantee_phrase(grant)?;
        if !force {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("refusing to remove {role} role for {grantees} from package {project}/{package} without force: removing access is destructive; pass force to confirm"),
            ));
        }
        if !dry_run.allow(&format!(
            "remove {role} role for {grantees} from package {project}/{package}"
        )) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!("cannot remove a role on package {project}/{package} without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)"),
            ));
        }
        let (body, mut meta) = self.read_package_meta(project, package)?;
        let (persons, groups) = remove_role_from_lists(
            &meta.persons,
            &meta.groups,
            grant,
            &format!("package {project}/{package}"),
        )?;
        meta.persons = persons;
        meta.groups = groups;
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
    use crate::obs::{DownloadMaster, ReleaseTarget, RepositoryDownload};

    const PROJECT_META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><description>Old description</description><lock><enable/></lock><person userid=\"alice\" role=\"maintainer\"/><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><path project=\"openSUSE:Factory\" repository=\"snapshot\"/></repository><build><disable/></build></project>";

    const PACKAGE_META: &str = "<package name=\"foo\" project=\"home:scarabeus_iv\"><title>Old title</title><description>Old description</description><url>https://example.com/old</url><scmsync>https://github.com/openSUSE/foo.git</scmsync><releasename>foo</releasename><devel project=\"devel:languages:python\"/><person userid=\"alice\" role=\"maintainer\"/><lock><enable/></lock></package>";

    fn project_client(http: &FakeHttp) -> ObsClient<'_> {
        ObsClient::with_credentials(http, basic_credentials()).unwrap()
    }

    fn replacement_repository() -> Repository {
        Repository {
            name: "openSUSE_Leap_15.6".to_string(),
            arch: vec!["x86_64".to_string()],
            paths: vec![("openSUSE:Leap:15.6".to_string(), "standard".to_string())],
            rebuild: None,
            block: None,
            linkedbuild: None,
            downloads: vec![],
            release_targets: vec![],
            hostsystem: vec![],
        }
    }

    fn set_project_meta_replacing_repositories_is_refused(fetched: &str) {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
            fetched,
        );
        let client = project_client(&http);
        let mut meta = parse_project_meta(fetched).unwrap();
        meta.repositories = vec![replacement_repository()];
        let err = client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
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

    const PROJECT_META_WITH_FULL_REPOSITORY: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\" rebuild=\"local\" block=\"never\" linkedbuild=\"all\"><arch>x86_64</arch><arch>aarch64</arch><path project=\"openSUSE:Factory\" repository=\"snapshot\"/><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><archfilter>x86_64</archfilter><master url=\"https://master.example.com\" sslfingerprint=\"AB:CD\"/><pubkey>key-material</pubkey></download><download arch=\"aarch64\" url=\"https://other.example.com\" repotype=\"rpmmd\"/><releasetarget project=\"openSUSE:Leap:15.6\" repository=\"standard\" trigger=\"manual\"/><releasetarget project=\"openSUSE:Leap:15.5\" repository=\"ports\"/><hostsystem project=\"openSUSE:Factory\" repository=\"snapshot\"/></repository></project>";

    const PROJECT_META_WITH_BASE_REPOSITORY: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><base project=\"openSUSE:Factory\" repository=\"snapshot\"/></repository></project>";

    const PROJECT_META_WITH_UNKNOWN_ATTRIBUTE: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\" foo=\"bar\"><arch>x86_64</arch></repository></project>";

    #[test]
    fn set_project_meta_replaces_a_repository_that_carries_all_settings() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                PROJECT_META_WITH_FULL_REPOSITORY,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                "<status code=\"ok\" />",
            );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META_WITH_FULL_REPOSITORY).unwrap();
        meta.repositories = vec![Repository {
            name: "openSUSE_Leap_15.6".to_string(),
            arch: vec!["x86_64".to_string()],
            paths: vec![("openSUSE:Leap:15.6".to_string(), "standard".to_string())],
            rebuild: Some("direct".to_string()),
            block: Some("local".to_string()),
            linkedbuild: Some("localdep".to_string()),
            downloads: vec![RepositoryDownload {
                arch: Some("x86_64".to_string()),
                url: "https://replacement.example.com".to_string(),
                repotype: Some("rpmmd".to_string()),
                archfilter: None,
                pubkey: None,
                master: Some(DownloadMaster {
                    url: "https://replacement-master.example.com".to_string(),
                    sslfingerprint: Some("AB:CD".to_string()),
                }),
            }],
            release_targets: vec![ReleaseTarget {
                project: "openSUSE:Leap:15.5".to_string(),
                repository: Some("ports".to_string()),
                trigger: Some("manual".to_string()),
            }],
            hostsystem: vec![("openSUSE:Factory:ARM".to_string(), "standard".to_string())],
        }];
        assert!(client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert!(puts[0].1.contains("rebuild=\"direct\""));
        assert!(puts[0].1.contains("block=\"local\""));
        assert!(puts[0].1.contains("linkedbuild=\"localdep\""));
        assert!(puts[0].1.contains(
            "<download arch=\"x86_64\" url=\"https://replacement.example.com\" repotype=\"rpmmd\">"
        ));
        assert!(puts[0].1.contains(
            "<master url=\"https://replacement-master.example.com\" sslfingerprint=\"AB:CD\"/>"
        ));
        assert!(puts[0].1.contains(
            "<releasetarget project=\"openSUSE:Leap:15.5\" repository=\"ports\" trigger=\"manual\"/>"
        ));
        assert!(puts[0]
            .1
            .contains("<hostsystem project=\"openSUSE:Factory:ARM\" repository=\"standard\"/>"));
    }

    #[test]
    fn set_project_meta_keeps_repository_settings_when_they_do_not_change() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                PROJECT_META_WITH_FULL_REPOSITORY,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                "<status code=\"ok\" />",
            );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META_WITH_FULL_REPOSITORY).unwrap();
        meta.title = Some("New title".to_string());
        assert!(client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert!(puts[0].1.contains("rebuild=\"local\""));
        assert!(puts[0].1.contains("block=\"never\""));
        assert!(puts[0].1.contains("linkedbuild=\"all\""));
        assert!(puts[0]
            .1
            .contains("<download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\">"));
        assert!(puts[0].1.contains("<archfilter>x86_64</archfilter>"));
        assert!(puts[0]
            .1
            .contains("<master url=\"https://master.example.com\" sslfingerprint=\"AB:CD\"/>"));
        assert!(puts[0].1.contains("<pubkey>key-material</pubkey>"));
        assert!(puts[0].1.contains(
            "<download arch=\"aarch64\" url=\"https://other.example.com\" repotype=\"rpmmd\"/>"
        ));
        assert!(puts[0].1.contains(
            "<releasetarget project=\"openSUSE:Leap:15.6\" repository=\"standard\" trigger=\"manual\"/>"
        ));
        assert!(puts[0]
            .1
            .contains("<releasetarget project=\"openSUSE:Leap:15.5\" repository=\"ports\"/>"));
        assert!(puts[0]
            .1
            .contains("<hostsystem project=\"openSUSE:Factory\" repository=\"snapshot\"/>"));
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_that_would_drop_settings() {
        set_project_meta_replacing_repositories_is_refused(PROJECT_META_WITH_BASE_REPOSITORY);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_an_unknown_attribute() {
        set_project_meta_replacing_repositories_is_refused(PROJECT_META_WITH_UNKNOWN_ATTRIBUTE);
    }

    #[test]
    fn parse_project_meta_rejects_a_download_without_url() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><download arch=\"x86_64\" repotype=\"rpmmd\"/></repository></project>";
        let err = parse_project_meta(META).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn set_project_meta_fails_closed_on_a_download_without_url() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><download arch=\"x86_64\" repotype=\"rpmmd\"/></repository></project>";
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
            META,
        );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META_WITH_FULL_REPOSITORY).unwrap();
        meta.title = Some("New title".to_string());
        let err = client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn parse_project_meta_rejects_a_releasetarget_without_project() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><releasetarget repository=\"standard\"/></repository></project>";
        let err = parse_project_meta(META).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn parse_project_meta_rejects_a_hostsystem_without_repository() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><hostsystem project=\"openSUSE:Factory\"/></repository></project>";
        let err = parse_project_meta(META).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn parse_project_meta_rejects_a_master_without_url() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><master sslfingerprint=\"AB:CD\"/></download></repository></project>";
        let err = parse_project_meta(META).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_an_unknown_download_attribute() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\" foo=\"bar\"/></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_an_unknown_download_child() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><foo/></download></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_a_duplicate_master() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><master url=\"https://a.example.com\"/><master url=\"https://b.example.com\"/></download></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_a_duplicate_archfilter() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><archfilter>x86_64</archfilter><archfilter>aarch64</archfilter></download></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_a_duplicate_pubkey() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><pubkey>key-one</pubkey><pubkey>key-two</pubkey></download></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_a_status_child() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><status>dirty</status></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_a_crosshostarch_child() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><crosshostarch>aarch64</crosshostarch></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_a_self_closing_repository_with_an_unknown_attribute(
    ) {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\" foo=\"bar\"/></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_an_unknown_releasetarget_attribute() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><releasetarget project=\"openSUSE:Leap:15.6\" repository=\"standard\" foo=\"bar\"/></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_an_unknown_hostsystem_attribute() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><hostsystem project=\"openSUSE:Factory\" repository=\"snapshot\" foo=\"bar\"/></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_an_unknown_path_attribute() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><path project=\"openSUSE:Factory\" repository=\"snapshot\" foo=\"bar\"/></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_an_unknown_arch_attribute() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch foo=\"bar\">x86_64</arch></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_an_unknown_master_attribute() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><master url=\"https://master.example.com\" foo=\"bar\"/></download></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_an_unknown_archfilter_attribute() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><archfilter foo=\"bar\">x86_64</archfilter></download></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn parse_project_meta_rejects_a_path_without_a_project() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><path repository=\"snapshot\"/></repository></project>";
        let err = parse_project_meta(META).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn parse_project_meta_rejects_a_path_without_a_repository() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><path project=\"openSUSE:Factory\"/></repository></project>";
        let err = parse_project_meta(META).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn parse_project_meta_rejects_a_repository_without_a_name() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository><arch>x86_64</arch></repository></project>";
        let err = parse_project_meta(META).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn parse_project_meta_rejects_a_repository_with_an_empty_name() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"\"><arch>x86_64</arch></repository></project>";
        let err = parse_project_meta(META).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_a_comment_inside_a_repository() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><!-- a note --><path project=\"openSUSE:Factory\" repository=\"snapshot\"/></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_a_comment_inside_a_download() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><!-- a note --></download></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_a_processing_instruction_inside_a_repository(
    ) {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><?pi data?><arch>x86_64</arch><path project=\"openSUSE:Factory\" repository=\"snapshot\"/></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_keeps_a_comment_outside_the_repositories() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><!-- a note --><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><path project=\"openSUSE:Factory\" repository=\"snapshot\"/></repository></project>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                META,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                "<status code=\"ok\" />",
            );
        let client = project_client(&http);
        let mut meta = parse_project_meta(META).unwrap();
        meta.repositories = vec![replacement_repository()];
        assert!(client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .is_ok());
        assert_eq!(http.put_calls().len(), 1);
        assert!(http.put_calls()[0].1.contains("<!-- a note -->"));
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_a_master_with_a_child() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><master url=\"https://master.example.com\"><foo/></master></download></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_a_master_with_text() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><master url=\"https://master.example.com\">junk</master></download></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_a_releasetarget_with_content() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><releasetarget project=\"openSUSE:Leap:15.6\" repository=\"standard\">junk</releasetarget></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_a_hostsystem_with_content() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><hostsystem project=\"openSUSE:Factory\" repository=\"snapshot\">junk</hostsystem></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_refuses_a_repository_rewrite_with_a_path_with_content() {
        const META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><path project=\"openSUSE:Factory\" repository=\"snapshot\">junk</path></repository></project>";
        set_project_meta_replacing_repositories_is_refused(META);
    }

    #[test]
    fn set_project_meta_rejects_an_invalid_rebuild_value() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
            PROJECT_META_WITH_FULL_REPOSITORY,
        );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META_WITH_FULL_REPOSITORY).unwrap();
        meta.repositories[0].rebuild = Some("bogus".to_string());
        let err = client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_project_meta_rejects_an_invalid_linkedbuild_value() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
            PROJECT_META_WITH_FULL_REPOSITORY,
        );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META_WITH_FULL_REPOSITORY).unwrap();
        meta.repositories[0].linkedbuild = Some("bogus".to_string());
        let err = client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_project_meta_rejects_an_empty_download_url() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
            PROJECT_META_WITH_FULL_REPOSITORY,
        );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META_WITH_FULL_REPOSITORY).unwrap();
        meta.repositories[0].downloads[0].url = String::new();
        let err = client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_project_meta_rejects_an_invalid_hostsystem() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
            PROJECT_META_WITH_FULL_REPOSITORY,
        );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META_WITH_FULL_REPOSITORY).unwrap();
        meta.repositories[0].hostsystem =
            vec![("bad project!".to_string(), "standard".to_string())];
        let err = client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_project_meta_accepts_blank_rebuild_and_linkedbuild_values() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                PROJECT_META_WITH_FULL_REPOSITORY,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                "<status code=\"ok\" />",
            );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META_WITH_FULL_REPOSITORY).unwrap();
        meta.repositories[0].rebuild = Some(String::new());
        meta.repositories[0].linkedbuild = Some(String::new());
        assert!(client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .is_ok());
        assert_eq!(http.put_calls().len(), 1);
    }

    #[test]
    fn set_project_meta_rejects_a_download_arch_outside_the_repository() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
            PROJECT_META_WITH_FULL_REPOSITORY,
        );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META_WITH_FULL_REPOSITORY).unwrap();
        meta.repositories[0].downloads[0].arch = Some("i586".to_string());
        let err = client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_project_meta_rejects_a_download_without_a_repotype() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
            PROJECT_META_WITH_FULL_REPOSITORY,
        );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META_WITH_FULL_REPOSITORY).unwrap();
        meta.repositories[0].downloads[0].repotype = None;
        let err = client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_project_meta_rejects_an_unknown_download_repotype() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
            PROJECT_META_WITH_FULL_REPOSITORY,
        );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META_WITH_FULL_REPOSITORY).unwrap();
        meta.repositories[0].downloads[0].repotype = Some("rpm-md".to_string());
        let err = client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_project_meta_rejects_a_duplicate_download_arch() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
            PROJECT_META_WITH_FULL_REPOSITORY,
        );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META_WITH_FULL_REPOSITORY).unwrap();
        meta.repositories[0].downloads.push(RepositoryDownload {
            arch: Some("x86_64".to_string()),
            url: "https://other.example.com".to_string(),
            repotype: Some("deb".to_string()),
            archfilter: None,
            pubkey: None,
            master: None,
        });
        let err = client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_project_meta_rejects_a_duplicate_download_arch_case_insensitively() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
            PROJECT_META_WITH_FULL_REPOSITORY,
        );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META_WITH_FULL_REPOSITORY).unwrap();
        meta.repositories[0].arch.push("X86_64".to_string());
        meta.repositories[0].downloads.push(RepositoryDownload {
            arch: Some("X86_64".to_string()),
            url: "https://other.example.com".to_string(),
            repotype: Some("deb".to_string()),
            archfilter: None,
            pubkey: None,
            master: None,
        });
        let err = client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_project_meta_rejects_a_download_url_without_a_scheme() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
            PROJECT_META_WITH_FULL_REPOSITORY,
        );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META_WITH_FULL_REPOSITORY).unwrap();
        meta.repositories[0].downloads[0].url = "example.com/no-scheme".to_string();
        let err = client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_project_meta_rejects_a_releasetarget_without_a_repository() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
            PROJECT_META_WITH_FULL_REPOSITORY,
        );
        let client = project_client(&http);
        let mut meta = parse_project_meta(PROJECT_META_WITH_FULL_REPOSITORY).unwrap();
        meta.repositories[0].release_targets[1].repository = None;
        let err = client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
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
        assert_eq!(
            meta.repositories[0].hostsystem,
            vec![("openSUSE:Factory".to_string(), "snapshot".to_string())]
        );
        meta.title = Some("New title".to_string());
        assert!(client
            .set_project_meta("home:scarabeus_iv", &meta, &DryRun::new(false))
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert!(puts[0]
            .1
            .contains("<hostsystem project=\"openSUSE:Factory\" repository=\"snapshot\"/>"));
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
            rebuild: None,
            block: None,
            linkedbuild: None,
            downloads: vec![],
            release_targets: vec![],
            hostsystem: vec![],
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

    #[test]
    fn role_add_project_adds_a_group_role_and_keeps_the_person() {
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
        assert!(client
            .role_add_project(
                "home:scarabeus_iv",
                &RoleGrant::group("editors", "reviewer"),
                &DryRun::new(false)
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(http.get_calls().len(), 1);
        assert!(puts[0]
            .1
            .contains("<person userid=\"alice\" role=\"maintainer\"/>"));
        assert!(puts[0]
            .1
            .contains("<group groupid=\"editors\" role=\"reviewer\"/>"));
        assert!(puts[0].1.contains("<title>Old title</title>"));
        assert!(puts[0].1.contains("<lock><enable/></lock>"));
    }

    #[test]
    fn role_add_project_granting_a_role_already_held_writes_nothing() {
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
        assert!(client
            .role_add_project(
                "home:scarabeus_iv",
                &RoleGrant::person("alice", "maintainer"),
                &DryRun::new(false)
            )
            .is_ok());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn role_add_project_rejects_a_role_outside_the_local_roles() {
        let http = FakeHttp::new();
        let client = project_client(&http);
        let err = client
            .role_add_project(
                "home:scarabeus_iv",
                &RoleGrant::person("alice", "admin"),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(
            err.context
                .contains("roles are maintainer, bugowner, reviewer, downloader, reader"),
            "{}",
            err.context
        );
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn role_add_project_rejects_an_invalid_role_before_the_missing_grantee() {
        let http = FakeHttp::new();
        let client = project_client(&http);
        let err = client
            .role_add_project(
                "home:scarabeus_iv",
                &RoleGrant {
                    person: None,
                    group: None,
                    role: "admin".to_string(),
                },
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.context.contains("invalid OBS role"), "{}", err.context);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn role_add_project_rejects_a_grant_with_no_grantee() {
        let http = FakeHttp::new();
        let client = project_client(&http);
        let err = client
            .role_add_project(
                "home:scarabeus_iv",
                &RoleGrant {
                    person: None,
                    group: None,
                    role: "maintainer".to_string(),
                },
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(
            err.context.contains("no person or group"),
            "{}",
            err.context
        );
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn role_add_project_dry_run_records_the_grant_and_sends_nothing() {
        let http = FakeHttp::new();
        let client = project_client(&http);
        let dry = DryRun::new(true);
        assert!(client
            .role_add_project(
                "home:scarabeus_iv",
                &RoleGrant::group("editors", "reviewer"),
                &dry
            )
            .is_ok());
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec![
                "[dry-run] would: add reviewer role for group editors on project home:scarabeus_iv"
                    .to_string()
            ]
        );
    }

    #[test]
    fn role_add_package_adds_a_person_and_a_group_role() {
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
        assert!(client
            .role_add_package(
                "home:scarabeus_iv",
                "foo",
                &RoleGrant::new(Some("bob"), Some("editors"), "reviewer"),
                &DryRun::new(false)
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(http.get_calls().len(), 1);
        assert!(puts[0]
            .1
            .contains("<person userid=\"alice\" role=\"maintainer\"/>"));
        assert!(puts[0]
            .1
            .contains("<person userid=\"bob\" role=\"reviewer\"/>"));
        assert!(puts[0]
            .1
            .contains("<group groupid=\"editors\" role=\"reviewer\"/>"));
        assert!(puts[0].1.contains("<title>Old title</title>"));
        assert!(puts[0].1.contains("<lock><enable/></lock>"));
    }

    #[test]
    fn role_remove_project_removes_the_role_and_keeps_the_rest() {
        const TWO_MAINTAINERS: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><person userid=\"alice\" role=\"maintainer\"/><person userid=\"bob\" role=\"maintainer\"/><group groupid=\"editors\" role=\"bugowner\"/></project>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                TWO_MAINTAINERS,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                "<status code=\"ok\" />",
            );
        let client = project_client(&http);
        assert!(client
            .role_remove_project(
                "home:scarabeus_iv",
                &RoleGrant::person("bob", "maintainer"),
                true,
                &DryRun::new(false)
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(http.get_calls().len(), 1);
        assert!(puts[0]
            .1
            .contains("<person userid=\"alice\" role=\"maintainer\"/>"));
        assert!(!puts[0].1.contains("bob"));
        assert!(puts[0]
            .1
            .contains("<group groupid=\"editors\" role=\"bugowner\"/>"));
        assert!(puts[0].1.contains("<title>Old title</title>"));
    }

    #[test]
    fn role_remove_project_refuses_without_force() {
        let http = FakeHttp::new();
        let client = project_client(&http);
        let err = client
            .role_remove_project(
                "home:scarabeus_iv",
                &RoleGrant::person("alice", "maintainer"),
                false,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.context.contains("without force"), "{}", err.context);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn role_remove_project_dry_run_without_force_errors() {
        let http = FakeHttp::new();
        let client = project_client(&http);
        let dry = DryRun::new(true);
        let err = client
            .role_remove_project(
                "home:scarabeus_iv",
                &RoleGrant::person("alice", "maintainer"),
                false,
                &dry,
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.context.contains("without force"), "{}", err.context);
        assert!(dry.plan().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn role_remove_project_refuses_a_pair_that_is_not_present() {
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
        let err = client
            .role_remove_project(
                "home:scarabeus_iv",
                &RoleGrant::person("carol", "reviewer"),
                true,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(
            err.context
                .contains("not found on project home:scarabeus_iv"),
            "{}",
            err.context
        );
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn role_remove_project_refuses_removing_the_last_maintainer() {
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
        let err = client
            .role_remove_project(
                "home:scarabeus_iv",
                &RoleGrant::person("alice", "maintainer"),
                true,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.context.contains("last maintainer"), "{}", err.context);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn role_remove_package_allows_removing_the_last_package_maintainer() {
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
        assert!(client
            .role_remove_package(
                "home:scarabeus_iv",
                "foo",
                &RoleGrant::person("alice", "maintainer"),
                true,
                &DryRun::new(false)
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(http.get_calls().len(), 1);
        assert!(!puts[0].1.contains("alice"));
        assert!(puts[0].1.contains("<title>Old title</title>"));
        assert!(puts[0].1.contains("<lock><enable/></lock>"));
    }

    #[test]
    fn role_remove_package_dry_run_records_the_removal_and_sends_nothing() {
        let http = FakeHttp::new();
        let client = project_client(&http);
        let dry = DryRun::new(true);
        assert!(client
            .role_remove_package(
                "home:scarabeus_iv",
                "foo",
                &RoleGrant::person("alice", "maintainer"),
                true,
                &dry
            )
            .is_ok());
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec!["[dry-run] would: remove maintainer role for person alice from package home:scarabeus_iv/foo"
                .to_string()]
        );
    }

    #[test]
    fn role_remove_package_dry_run_without_force_errors() {
        let http = FakeHttp::new();
        let client = project_client(&http);
        let dry = DryRun::new(true);
        let err = client
            .role_remove_package(
                "home:scarabeus_iv",
                "foo",
                &RoleGrant::person("alice", "maintainer"),
                false,
                &dry,
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.context.contains("without force"), "{}", err.context);
        assert!(dry.plan().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn role_add_project_requires_credentials() {
        let http = FakeHttp::new();
        use crate::auth::Credentials;
        let client = ObsClient::with_credentials(&http, Credentials::None).unwrap();
        let err = client
            .role_add_project(
                "home:scarabeus_iv",
                &RoleGrant::person("alice", "maintainer"),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn role_add_package_requires_credentials() {
        let http = FakeHttp::new();
        use crate::auth::Credentials;
        let client = ObsClient::with_credentials(&http, Credentials::None).unwrap();
        let err = client
            .role_add_package(
                "home:scarabeus_iv",
                "foo",
                &RoleGrant::person("alice", "maintainer"),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn role_remove_project_requires_credentials() {
        let http = FakeHttp::new();
        use crate::auth::Credentials;
        let client = ObsClient::with_credentials(&http, Credentials::None).unwrap();
        let err = client
            .role_remove_project(
                "home:scarabeus_iv",
                &RoleGrant::person("alice", "maintainer"),
                true,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn role_remove_package_requires_credentials() {
        let http = FakeHttp::new();
        use crate::auth::Credentials;
        let client = ObsClient::with_credentials(&http, Credentials::None).unwrap();
        let err = client
            .role_remove_package(
                "home:scarabeus_iv",
                "foo",
                &RoleGrant::person("alice", "maintainer"),
                true,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn role_remove_project_refuses_removing_the_last_group_maintainer() {
        const GROUP_MAINTAINER: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><group groupid=\"editors\" role=\"maintainer\"/></project>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                GROUP_MAINTAINER,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                "<status code=\"ok\" />",
            );
        let client = project_client(&http);
        let err = client
            .role_remove_project(
                "home:scarabeus_iv",
                &RoleGrant::group("editors", "maintainer"),
                true,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.context.contains("last maintainer"), "{}", err.context);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn role_remove_project_removing_a_non_maintainer_role_with_one_maintainer_succeeds() {
        const MAINTAINER_AND_BUGOWNER: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><person userid=\"alice\" role=\"maintainer\"/><person userid=\"bob\" role=\"bugowner\"/></project>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                MAINTAINER_AND_BUGOWNER,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                "<status code=\"ok\" />",
            );
        let client = project_client(&http);
        assert!(client
            .role_remove_project(
                "home:scarabeus_iv",
                &RoleGrant::person("bob", "bugowner"),
                true,
                &DryRun::new(false)
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert!(puts[0]
            .1
            .contains("<person userid=\"alice\" role=\"maintainer\"/>"));
        assert!(!puts[0].1.contains("bob"));
    }

    #[test]
    fn role_remove_project_refuses_removing_both_maintainers_in_one_call() {
        const PERSON_AND_GROUP_MAINTAINERS: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><person userid=\"alice\" role=\"maintainer\"/><group groupid=\"editors\" role=\"maintainer\"/></project>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                PERSON_AND_GROUP_MAINTAINERS,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                "<status code=\"ok\" />",
            );
        let client = project_client(&http);
        let err = client
            .role_remove_project(
                "home:scarabeus_iv",
                &RoleGrant::new(Some("alice"), Some("editors"), "maintainer"),
                true,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.context.contains("last maintainer"), "{}", err.context);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn role_remove_project_refuses_a_missing_pair_without_a_partial_write() {
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
        let err = client
            .role_remove_project(
                "home:scarabeus_iv",
                &RoleGrant::new(Some("alice"), Some("editors"), "maintainer"),
                true,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(err.context.contains("not found"), "{}", err.context);
        assert!(http.put_calls().is_empty());
    }
}
