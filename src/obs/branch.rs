use super::build_url;
use super::parse_directory_rev;
use super::parse_linkinfo;
use super::parse_package_names;
use super::parse_revision;
use super::parse_status_invoked;
use super::parse_status_ok;
use super::status_data;
use super::valid_obs_name;
use super::BranchOptions;
use super::BranchResult;
use super::CopyOptions;
use super::CopyProjectOptions;
use super::DryRun;
use super::LinkOptions;
use super::LinkRevision;
use super::MaintenanceBranchOptions;
use super::ObsClient;
use crate::error::ErrorKind;
use crate::error::GeectlError;
use crate::xml::xml_escape;
use crate::xml::XmlDoc;

/// Parse the revision number out of a backend `<revision>` document
/// (the response to a file upload). Returns the revision number, or
/// None when the body is not a revision document. The backend returns
/// the new revision after storing a file (`bs_srcserver::putfile`
/// returns `($rev, $BSXML::revision)`); the API streams it back
/// through `pass_to_backend`, so the upload response is a revision
/// document, not a `<status code="ok"/>` — `parse_status_ok` would
/// read every successful upload as a failure.
/// The option suffix for a branch plan line: the set options, in the
/// order they appear in [`BranchOptions`], so a dry-run plan reads
/// what the branch would have done. The target project and package
/// are not listed: the plan line already names the resolved target.
fn branch_option_suffix(options: &BranchOptions) -> String {
    let mut parts: Vec<String> = Vec::new();
    if options.no_devel_project {
        parts.push("no devel project".to_string());
    }
    if options.new_package {
        parts.push("new package".to_string());
    }
    if options.force {
        parts.push("force".to_string());
    }
    if let Some(revision) = &options.revision {
        parts.push(format!("revision {revision}"));
    }
    if let Some(linkrev) = &options.linkrev {
        parts.push(format!("link revision {linkrev}"));
    }
    if let Some(comment) = &options.comment {
        parts.push(format!("comment {comment}"));
    }
    if options.noaccess {
        parts.push("no access".to_string());
    }
    if options.add_repositories {
        parts.push("add repositories".to_string());
    }
    if let Some(block) = &options.add_repositories_block {
        parts.push(format!("add repositories block {block}"));
    }
    if let Some(rebuild) = &options.add_repositories_rebuild {
        parts.push(format!("add repositories rebuild {rebuild}"));
    }
    if options.extend_package_names {
        parts.push("extend package names".to_string());
    }
    if options.maintenance {
        parts.push("maintenance".to_string());
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

/// Parse the branch target out of the server's `cmd=branch`
/// response: a `<status code="ok"/>` carrying the source and the
/// target as `<data>` elements. Returns None when the body is not
/// one (a non-ok status, a missing element, an empty value).
///
/// The server renders the names two ways: the current
/// `target_project`, `target_package`, `project`, `package`, and the
/// deprecated `targetproject`, `targetpackage`, `sourceproject`,
/// `sourcepackage`. The current names win when both are present; the
/// deprecated ones are the fallback for an older server.
pub(crate) fn parse_branch_result(body: &str) -> Option<BranchResult> {
    if !parse_status_ok(body) {
        return None;
    }
    let target_project =
        status_data(body, "target_project").or_else(|| status_data(body, "targetproject"))?;
    let target_package =
        status_data(body, "target_package").or_else(|| status_data(body, "targetpackage"))?;
    let source_project =
        status_data(body, "project").or_else(|| status_data(body, "sourceproject"))?;
    let source_package =
        status_data(body, "package").or_else(|| status_data(body, "sourcepackage"))?;
    Some(BranchResult {
        source_project,
        source_package,
        target_project,
        target_package,
    })
}

/// Parse the `<status code="ok">` answer to a `cmd=fork` POST into a
/// [`BranchResult`]. Like [`parse_branch_result`], except a project
/// fork (package `"_project"`) answers only `targetproject` /
/// `sourceproject` — the server's `create_fork` returns no package
/// names for it — so the package fields default to `"_project"`
/// there, while a package fork without them is malformed and
/// returns `None`.
pub(crate) fn parse_fork_result(body: &str, package: &str) -> Option<BranchResult> {
    if !parse_status_ok(body) {
        return None;
    }
    let target_project =
        status_data(body, "target_project").or_else(|| status_data(body, "targetproject"))?;
    let source_project =
        status_data(body, "project").or_else(|| status_data(body, "sourceproject"))?;
    let is_project_fork = package == "_project";
    let target_package =
        match status_data(body, "target_package").or_else(|| status_data(body, "targetpackage")) {
            Some(name) => name,
            None if is_project_fork => "_project".to_string(),
            None => return None,
        };
    let source_package =
        match status_data(body, "package").or_else(|| status_data(body, "sourcepackage")) {
            Some(name) => name,
            None if is_project_fork => "_project".to_string(),
            None => return None,
        };
    Some(BranchResult {
        source_project,
        source_package,
        target_project,
        target_package,
    })
}

/// The option suffix for a copy plan line: the set options, in the
/// order they appear in [`CopyOptions`], so a dry-run plan reads what
/// the copy would have done.
fn copy_option_suffix(options: &CopyOptions) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(revision) = &options.revision {
        parts.push(format!("revision {revision}"));
    }
    if options.keep_link {
        parts.push("keep link".to_string());
    }
    if options.expand {
        parts.push("expand".to_string());
    }
    if let Some(comment) = &options.comment {
        parts.push(format!("comment {comment}"));
    }
    if options.noservice {
        parts.push("no service".to_string());
    }
    if options.dont_update_source {
        parts.push("do not update source".to_string());
    }
    if options.with_vrev {
        parts.push("with vrev".to_string());
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

/// The option suffix for a project-copy plan line: the set options, in
/// the order they appear in [`CopyProjectOptions`].
fn copy_project_option_suffix(options: &CopyProjectOptions) -> String {
    let mut parts: Vec<String> = Vec::new();
    if options.with_binaries {
        parts.push("with binaries".to_string());
    }
    if options.with_history {
        parts.push("with history".to_string());
    }
    if options.make_older {
        parts.push("make older".to_string());
    }
    if options.make_origin_older {
        parts.push("make origin older".to_string());
    }
    if options.noservice {
        parts.push("no service".to_string());
    }
    if options.resign {
        parts.push("resign".to_string());
    }
    if let Some(comment) = &options.comment {
        parts.push(format!("comment {comment}"));
    }
    if options.no_delay {
        parts.push("no delay".to_string());
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

/// Build the target meta for a fresh link from the source meta: the
/// source meta with the project and name set to the target, and the
/// entries a link never inherits stripped (devel, person, group,
/// lock, release name, scmsync). The title and description carry
/// over; anything else in the source meta survives untouched.
pub(crate) fn build_link_meta(
    source_meta: &str,
    target_project: &str,
    target_package: &str,
) -> Result<XmlDoc, GeectlError> {
    let mut doc = XmlDoc::parse(source_meta)?;
    doc.set_root_attr("project", target_project);
    doc.set_root_attr("name", target_package);
    for name in ["devel", "person", "group", "lock", "releasename", "scmsync"] {
        doc.remove_elements(name);
    }
    Ok(doc)
}

/// The option suffix for a link plan line: the set options, in the
/// order they appear in [`LinkOptions`], so a dry-run plan reads what
/// the link would have done.
fn link_option_suffix(options: &LinkOptions) -> String {
    let mut parts: Vec<String> = Vec::new();
    if options.force {
        parts.push("force".to_string());
    }
    if let Some(revision) = &options.revision {
        parts.push(format!("revision {revision}"));
    }
    if let Some(vrev) = &options.vrev {
        parts.push(format!("vrev {vrev}"));
    }
    if let Some(cicount) = &options.cicount {
        parts.push(format!("cicount {cicount}"));
    }
    if options.missing_target {
        parts.push("missing target".to_string());
    }
    if options.disable_build {
        parts.push("disable build".to_string());
    }
    if options.disable_publish {
        parts.push("disable publish".to_string());
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

/// Strip the apply/delete patches from a parsed `_link` document,
/// mirroring what the server's repair does to the link: the first
/// apply or delete entry and everything after it is dropped — later
/// entries are based on the broken state — while entries before it
/// (add, topadd) survive. Sets `baserev` when `new_baserev` is
/// given. Returns true when a patch was stripped; false when the
/// link has no apply/delete patch at all, which means the breakage
/// is not a patch failure and this repair cannot fix it.
fn repair_link_doc(doc: &mut XmlDoc, new_baserev: Option<&str>) -> Result<bool, GeectlError> {
    let Some(patches) = doc.child("patches") else {
        return Ok(false);
    };
    let mut inner = XmlDoc::parse(&patches)?;
    if !inner.truncate_at_element(&["apply", "delete"]) {
        return Ok(false);
    }
    if inner.has_elements() {
        doc.replace_or_insert_element("patches", &inner.render())?;
    } else {
        doc.remove_elements("patches");
    }
    if let Some(baserev) = new_baserev {
        doc.set_root_attr("baserev", baserev);
    }
    Ok(true)
}

impl<'a> ObsClient<'a> {
    /// The query parameters for the branch command
    /// (`POST /source?cmd=branch`), shared by the dry-run listing
    /// and the live branch: the attribute, the update project
    /// attribute, the package, and the target project override when
    /// set. The dry-run flag is added only for the listing.
    pub(crate) fn branch_query<'b>(
        &self,
        package: &'b str,
        options: &'b MaintenanceBranchOptions,
        dry_run: bool,
    ) -> Vec<(&'b str, &'b str)> {
        let mut query: Vec<(&str, &str)> = vec![
            ("cmd", "branch"),
            ("attribute", &options.attribute),
            (
                "update_project_attribute",
                &options.update_project_attribute,
            ),
            ("package", package),
        ];
        if let Some(target_project) = &options.target_project {
            query.push(("target_project", target_project));
        }
        if dry_run {
            query.push(("dryrun", "1"));
        }
        query
    }

    /// Branch package `package` in project `project` on OBS
    /// (`POST /source/<project>/<package>?cmd=branch`) and return
    /// where the branch landed. Requires credentials; without them the
    /// call fails loudly before any request. In dry-run the branch is
    /// recorded in the plan and nothing is branched — the guard runs
    /// before the credentials check, like `create_package` — and the
    /// result is `None`, like `upload_file` in dry-run.
    ///
    /// An agent branches the package it is about to change, so that
    /// the change is a package like any other: the branch is where the
    /// spec and `.changes` edits land before a submit request is
    /// filed from it. The link source resolves through the package's
    /// devel project when one is registered, unless
    /// `no_devel_project` is set; a package inherited through a
    /// project link branches from the project it is inherited from.
    ///
    /// `new_package` is the server's two-step for a package that does
    /// not exist yet (or only exists through a project link): when the
    /// source does not exist yet the call sends `missingok`, when it
    /// already exists it sends `newinstance`. A `newinstance` branch
    /// also copies the source's devel package sources in when the
    /// source has a devel project. The two cases are told
    /// apart by reading the source first, because the transport
    /// reports a failed call by status only and the server's
    /// `not_missing` error code does not survive it. The read runs
    /// only on the `new_package` path; a plain branch makes a single
    /// POST.
    ///
    /// Success is the server's `<status code="ok"/>` carrying the
    /// branch target as `<data>` elements; any other body is a server
    /// error, never success.
    pub fn branch_package(
        &self,
        project: &str,
        package: &str,
        options: &BranchOptions,
        dry_run: &DryRun,
    ) -> Result<Option<BranchResult>, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if let Some(target) = &options.target_project {
            if !valid_obs_name(target) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS project name: {target:?}"),
                ));
            }
        }
        if let Some(target) = &options.target_package {
            if !valid_obs_name(target) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS package name: {target:?}"),
                ));
            }
        }
        for value in [
            &options.add_repositories_block,
            &options.add_repositories_rebuild,
        ]
        .into_iter()
        .flatten()
        {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS repository name: {value:?}"),
                ));
            }
        }
        // The dry-run guard: in dry-run the branch is recorded in the
        // plan and nothing is branched — not even the credentials
        // check runs. The target is named when it is known: the
        // explicit target, or the server default when the username is
        // available from the credentials.
        // The dry-run plan names the target only when the caller
        // gave one: otherwise it says the server default instead of
        // guessing it, and the request omits the keys below so the
        // server applies its own default.
        let target_project = options
            .target_project
            .as_deref()
            .unwrap_or("the server default branch project");
        let target_package = options.target_package.as_deref().unwrap_or(package);
        if !dry_run.allow(&format!(
            "branch package {project}/{package} to {target_project}/{target_package}{}",
            branch_option_suffix(options),
        )) {
            return Ok(None);
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot branch a package without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let mut query: Vec<(&str, &str)> = vec![("cmd", "branch")];
        if let Some(target_project) = &options.target_project {
            query.push(("target_project", target_project));
        }
        if let Some(target_package) = &options.target_package {
            query.push(("target_package", target_package));
        }
        if options.no_devel_project {
            query.push(("ignoredevel", "1"));
        }
        if options.new_package {
            // The source's existence decides the flag: `missingok`
            // when the source does not exist yet, `newinstance`
            // when it already does (the server answers
            // `not_missing` to a `missingok` branch of an existing
            // source, and the transport does not carry the error
            // code back). A source inherited through a project link
            // reads as existing — the branch creates a new instance
            // of it.
            let source_url = build_url(&self.base, &["source", project, package], &[]);
            match self.http.get_with(&source_url, &self.credentials) {
                Ok(_) => query.push(("newinstance", "1")),
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    query.push(("missingok", "1"));
                }
                Err(err) => return Err(err),
            }
        }
        if options.force {
            query.push(("force", "1"));
        }
        if let Some(rev) = &options.revision {
            query.push(("rev", rev));
        }
        if let Some(linkrev) = &options.linkrev {
            query.push(("linkrev", linkrev));
        }
        if let Some(comment) = &options.comment {
            query.push(("comment", comment));
        }
        if options.noaccess {
            query.push(("noaccess", "1"));
        }
        if options.add_repositories {
            query.push(("add_repositories", "1"));
            if let Some(block) = &options.add_repositories_block {
                query.push(("add_repositories_block", block));
            }
            if let Some(rebuild) = &options.add_repositories_rebuild {
                query.push(("add_repositories_rebuild", rebuild));
            }
        }
        if options.extend_package_names {
            query.push(("extend_package_names", "1"));
        }
        if options.maintenance {
            query.push(("maintenance", "1"));
        }
        let url = build_url(&self.base, &["source", project, package], &query);
        let body = self.http.post(&url, "", &self.credentials)?;
        match parse_branch_result(&body) {
            Some(result) => Ok(Some(result)),
            None => Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS branch response for package {project}/{package}"),
            )),
        }
    }

    /// Fork a git-backed package on OBS (`POST
    /// /source/<project>/<package>?cmd=fork&scmsync=<url>`): the
    /// server creates the target package (or project, for package
    /// `"_project"`) wired to the fork's scmsync URL, with no source
    /// copy — the sync backend clones the URL. Requires OBS
    /// credentials; without them the call fails loudly before any
    /// request. This is the OBS half of the `branch` composite
    /// (`GiteaClient::branch`); it is not a standalone operation,
    /// so it carries no DryRun of its own — the composite's guard
    /// runs first and the composite never calls this in dry-run.
    /// `target_project` / `target_package` are omitted when `None`,
    /// so the server applies its defaults (`home:<user>:branches:<project>`
    /// and the package name); the answer names the actual targets.
    pub(crate) fn fork_package(
        &self,
        project: &str,
        package: &str,
        scmsync: &str,
        target_project: Option<&str>,
        target_package: Option<&str>,
    ) -> Result<BranchResult, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if let Some(target) = target_project {
            if !valid_obs_name(target) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS target project name: {target:?}"),
                ));
            }
        }
        if let Some(target) = target_package {
            if !valid_obs_name(target) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS target package name: {target:?}"),
                ));
            }
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot fork a package without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let mut query: Vec<(&str, &str)> = vec![("cmd", "fork"), ("scmsync", scmsync)];
        if let Some(target_project) = target_project {
            query.push(("target_project", target_project));
        }
        if let Some(target_package) = target_package {
            query.push(("target_package", target_package));
        }
        let url = build_url(&self.base, &["source", project, package], &query);
        let body = self.http.post(&url, "", &self.credentials)?;
        match parse_fork_result(&body, package) {
            Some(result) => Ok(result),
            None => Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS fork response for package {project}/{package}"),
            )),
        }
    }

    /// Copy package `source_package` in project `source_project` to
    /// `target_package` in project `target_project` on OBS
    /// (`POST /source/<target>/<target package>?cmd=copy`) and return
    /// the new revision number. Requires credentials; without them the
    /// call fails loudly before any request. In dry-run the copy is
    /// recorded in the plan and nothing is copied — the guard runs
    /// before the credentials check, like `create_package` — and the
    /// result is `None`, like `upload_file` in dry-run.
    ///
    /// An agent copies a package to fork its content into another
    /// project (or under another name) without the link a branch
    /// carries: the copy is a standalone package, and the server
    /// builds the target meta from the source meta itself, so the
    /// caller never writes one. Copying a package onto itself needs
    /// `expand` or a revision — without either the server has nothing
    /// to copy, and the call is refused client-side with that reason
    /// instead of failing on the server.
    ///
    /// Success is the backend's `<revision>` document carrying the
    /// new revision number, not a `<status code="ok"/>`: the
    /// controller passes the backend's answer back, so a status check
    /// would read every successful copy as a failure (the same shape
    /// as `upload_file`).
    pub fn copy_package(
        &self,
        source_project: &str,
        source_package: &str,
        target_project: &str,
        target_package: &str,
        options: &CopyOptions,
        dry_run: &DryRun,
    ) -> Result<Option<u64>, GeectlError> {
        for (label, value) in [
            ("project", source_project),
            ("package", source_package),
            ("project", target_project),
            ("package", target_package),
        ] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        // A self-copy with nothing to copy from: the server dies
        // with "need a revision to copy", so refuse it here with the
        // reason instead of failing on the server.
        if source_project == target_project
            && source_package == target_package
            && !options.expand
            && options.revision.is_none()
        {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "copying a package onto itself needs expand or a revision",
            ));
        }
        let mut query: Vec<(&str, &str)> = vec![
            ("cmd", "copy"),
            ("oproject", source_project),
            ("opackage", source_package),
        ];
        if let Some(rev) = &options.revision {
            query.push(("orev", rev));
        }
        if options.keep_link {
            query.push(("keeplink", "1"));
        }
        if options.expand {
            query.push(("expand", "1"));
        }
        if let Some(comment) = &options.comment {
            query.push(("comment", comment));
        }
        if options.noservice {
            query.push(("noservice", "1"));
        }
        if options.dont_update_source {
            query.push(("dontupdatesource", "1"));
        }
        if options.with_vrev {
            query.push(("withvrev", "1"));
        }
        self.copy_to(
            target_project,
            target_package,
            &query,
            &format!(
                "copy package {source_project}/{source_package} to {target_project}/{target_package}{}",
                copy_option_suffix(options),
            ),
            dry_run,
        )
    }

    /// Copy project `source_project` to project `target_project` on
    /// OBS (`POST /source/<target>?cmd=copy`). Requires credentials;
    /// without them the call fails loudly before any request. In
    /// dry-run the copy is recorded in the plan and nothing is
    /// copied — the guard runs before the credentials check, like
    /// `create_package`.
    ///
    /// An agent copies a project to fork a whole project (its
    /// packages, flags, links, and repositories — the server copies
    /// them from the source project itself) under another name. The
    /// server runs the copy as a background job by default and
    /// answers `invoked`; with `no_delay` it runs synchronously and
    /// answers `ok`. Both are success here — the copy is accepted
    /// either way, and a queued copy lands asynchronously, so the
    /// caller polls for the target project when it needs it.
    pub fn copy_project(
        &self,
        source_project: &str,
        target_project: &str,
        options: &CopyProjectOptions,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        for (label, value) in [("project", source_project), ("project", target_project)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        // The dry-run guard: in dry-run the copy is recorded in the
        // plan and nothing is copied — not even the credentials check
        // runs.
        if !dry_run.allow(&format!(
            "copy project {source_project} to {target_project}{}",
            copy_project_option_suffix(options),
        )) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot copy a project without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let mut query: Vec<(&str, &str)> = vec![("cmd", "copy"), ("oproject", source_project)];
        if options.with_binaries {
            query.push(("withbinaries", "1"));
        }
        if options.with_history {
            query.push(("withhistory", "1"));
        }
        if options.make_older {
            query.push(("makeolder", "1"));
        }
        if options.make_origin_older {
            query.push(("makeoriginolder", "1"));
        }
        if options.noservice {
            query.push(("noservice", "1"));
        }
        if options.resign {
            query.push(("resign", "1"));
        }
        if let Some(comment) = &options.comment {
            query.push(("comment", comment));
        }
        if options.no_delay {
            query.push(("nodelay", "1"));
        }
        let url = build_url(&self.base, &["source", target_project], &query);
        let body = self.http.post(&url, "", &self.credentials)?;
        if parse_status_ok(&body) || parse_status_invoked(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS copy-project response for project {target_project}"),
            ))
        }
    }

    /// POST a `cmd=copy` to the target package and return the new
    /// revision number. Shared by [`ObsClient::copy_package`] and the
    /// repair path, which copies with its own flags. The dry-run
    /// guard and the credentials check live here, so both callers get
    /// them: in dry-run the copy is recorded in the plan and nothing
    /// is copied, and the result is `None`.
    fn copy_to(
        &self,
        target_project: &str,
        target_package: &str,
        query: &[(&str, &str)],
        plan_action: &str,
        dry_run: &DryRun,
    ) -> Result<Option<u64>, GeectlError> {
        if !dry_run.allow(plan_action) {
            return Ok(None);
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot copy a package without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(
            &self.base,
            &["source", target_project, target_package],
            query,
        );
        let body = self.http.post(&url, "", &self.credentials)?;
        match parse_revision(&body) {
            Some(rev) => Ok(Some(rev)),
            None => Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected OBS copy response for package {target_project}/{target_package}"
                ),
            )),
        }
    }

    /// Set the devel project of package `package` in project `project`
    /// on OBS (`GET` then `PUT /source/<project>/<package>/_meta`),
    /// or remove it when `devel_project` is None. `devel_package`
    /// names a different package in the devel project (a cross-package
    /// mapping); it is ignored when `devel_project` is None. Requires
    /// credentials; without them the call fails loudly before any
    /// request. In dry-run the change is recorded in the plan and
    /// nothing is changed — the guard runs before the credentials
    /// check, like `create_package`.
    ///
    /// An agent sets the devel project so the package resolves its
    /// branch source (and its submit target) through the right
    /// project: branching follows the devel project, so a package
    /// whose devel is wrong branches from the wrong place. The meta is
    /// PUT back only when the edit changes it — setting the devel that
    /// is already there makes no request.
    ///
    /// Success is the `<status code="ok"/>` body on the PUT; any
    /// other body is a server error, never success. A missing package
    /// reads as `NotFound` from the GET, before anything is written.
    /// The server refuses a devel whose package does not exist in
    /// the devel project, so the devel package must be created
    /// there first.
    pub fn set_devel_project(
        &self,
        project: &str,
        package: &str,
        devel_project: Option<&str>,
        devel_package: Option<&str>,
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
        if let Some(devel) = devel_project {
            if !valid_obs_name(devel) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS project name: {devel:?}"),
                ));
            }
        }
        if let Some(devel) = devel_package {
            if !valid_obs_name(devel) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS package name: {devel:?}"),
                ));
            }
        }
        // The dry-run guard: in dry-run the change is recorded in the
        // plan and nothing is changed — not even the credentials check
        // runs.
        let action = match devel_project {
            Some(devel) => match devel_package {
                Some(pkg) => format!("set devel project of {project}/{package} to {devel}/{pkg}"),
                None => format!("set devel project of {project}/{package} to {devel}"),
            },
            None => format!("unset devel project of {project}/{package}"),
        };
        if !dry_run.allow(&action) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot set a devel project without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(&self.base, &["source", project, package, "_meta"], &[]);
        let meta = self.http.get_with(&url, &self.credentials)?;
        let mut doc = XmlDoc::parse(&meta)?;
        // The document as fetched, for the no-change check below:
        // comparing parsed documents (not rendered text) means a
        // prolog in the fetched meta, or a reorder from an edit that
        // changes nothing, does not read as a change.
        let before = doc.clone();
        match devel_project {
            Some(devel) => {
                let element = match devel_package {
                    Some(pkg) => format!(
                        "<devel project=\"{}\" package=\"{}\"/>",
                        xml_escape(devel),
                        xml_escape(pkg),
                    ),
                    None => format!("<devel project=\"{}\"/>", xml_escape(devel)),
                };
                doc.replace_or_insert_element("devel", &element)?;
            }
            None => doc.remove_elements("devel"),
        }
        let rendered = doc.render();
        // No change, no PUT: setting the devel that is already there
        // (or unsetting one that is not) writes nothing. The
        // comparison is on the parsed documents, so a prolog in the
        // fetched meta does not read as a change.
        if doc == before {
            return Ok(());
        }
        let body = self.http.put(&url, &rendered, &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected OBS set-devel-project response for package {project}/{package}"
                ),
            ))
        }
    }

    /// Link package `source_package` in project `source_project`
    /// into `target_package` in project `target_project` on OBS: the
    /// target package becomes a link at the source (a `PUT` of its
    /// `_meta` and its `_link`). Requires credentials; without them
    /// the call fails loudly before any request. In dry-run the link
    /// is recorded in the plan and nothing is linked — the guard runs
    /// before the credentials check, like `create_package`.
    ///
    /// An agent links a package when it wants the target to track the
    /// source (the link expands to the source's files on read) without
    /// copying the content: a devel package linked into a test
    /// project, a package linked under another name. The target meta
    /// is built from the source meta when the target does not exist
    /// yet (with the devel, person, group, lock, release name,
    /// and scmsync entries stripped — a link never inherits those),
    /// and edited in place when it does — the target keeps its own
    /// scmsync there, so linking onto a git-backed package does not
    /// drop its sync config. The `_link` always names
    /// the source package explicitly, with the project attribute
    /// omitted when the link stays in the same project.
    ///
    /// The meta is PUT back only when the edit changes it, and the
    /// link is refused when the target already has one unless `force`
    /// is set — the refusal happens before anything is written, so a
    /// refused link leaves no half-done meta behind. A target package
    /// that only exists through a project link does not count as
    /// having a link: the shadowed meta's project differs from the
    /// target project, so the link is created explicitly (and the
    /// meta is built from the source, not from the shadow).
    ///
    /// Success is the `<status code="ok"/>` body on the meta PUT and
    /// the backend's `<revision>` document on the `_link` PUT; any
    /// other body is a server error, never success. A missing source
    /// package reads as `NotFound` from the source meta GET, before
    /// anything is written.
    pub fn link_package(
        &self,
        source_project: &str,
        source_package: &str,
        target_project: &str,
        target_package: &str,
        options: &LinkOptions,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        for (label, value) in [
            ("project", source_project),
            ("package", source_package),
            ("project", target_project),
            ("package", target_package),
        ] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        // The dry-run guard: in dry-run the link is recorded in the
        // plan and nothing is linked — not even the credentials check
        // runs.
        if !dry_run.allow(&format!(
            "link package {source_project}/{source_package} to {target_project}/{target_package}{}",
            link_option_suffix(options),
        )) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot link a package without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let source_url = build_url(
            &self.base,
            &["source", source_project, source_package, "_meta"],
            &[],
        );
        let source_meta = self.http.get_with(&source_url, &self.credentials)?;
        let target_url = build_url(
            &self.base,
            &["source", target_project, target_package, "_meta"],
            &[],
        );
        let target_meta = match self.http.get_with(&target_url, &self.credentials) {
            Ok(meta) => Some(meta),
            Err(err) if err.kind() == ErrorKind::NotFound => None,
            Err(err) => return Err(err),
        };
        // The target meta is built from the source meta when the
        // target does not exist yet, or when the target only exists
        // through a project link (its meta names another project):
        // the shadow must not supply the target's meta.
        let (mut doc, put_meta, target_exists_here, before) = match &target_meta {
            Some(meta) => {
                let doc = XmlDoc::parse(meta)?;
                if doc.root_attr("project").as_deref() == Some(target_project) {
                    // The document as fetched, for the no-change
                    // check below: comparing parsed documents (not
                    // rendered text) means a prolog in the fetched
                    // meta, or a reorder from an edit that changes
                    // nothing, does not read as a change.
                    let before = doc.clone();
                    (doc, false, true, Some(before))
                } else {
                    (
                        build_link_meta(&source_meta, target_project, target_package)?,
                        true,
                        false,
                        None,
                    )
                }
            }
            None => (
                build_link_meta(&source_meta, target_project, target_package)?,
                true,
                false,
                None,
            ),
        };
        // A link never inherits the source's scmsync — but only the
        // meta built from the source is stripped (build_link_meta
        // already does that). On the edit-in-place path the target
        // keeps its own sync config: dropping it would silently
        // un-sync a git-backed package. The disable flags are applied
        // to the target meta either way.
        if put_meta {
            doc.remove_elements("scmsync");
        }
        if options.disable_build {
            doc.replace_or_insert_element("build", "<build><disable/></build>")?;
        }
        if options.disable_publish {
            doc.replace_or_insert_element("publish", "<publish><disable/></publish>")?;
        }
        // Refuse an existing link before writing anything, unless
        // forced — but only when the target package really lives in
        // the target project (see above): a shadowed package has no
        // explicit link to refuse.
        if target_exists_here && !options.force {
            let link_url = build_url(
                &self.base,
                &["source", target_project, target_package, "_link"],
                &[],
            );
            match self.http.get_with(&link_url, &self.credentials) {
                Ok(_) => {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "package {target_project}/{target_package} already has a link; pass force to replace it"
                        ),
                    ));
                }
                Err(err) if err.kind() == ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }
        }
        let rendered = doc.render();
        // A meta built from the source is always PUT; an edited
        // target meta only when the edit changed it — compared on the
        // parsed documents, so a prolog in the fetched meta does not
        // read as a change.
        let changed = before.as_ref().is_some_and(|orig| *orig != doc);
        if put_meta || changed {
            let body = self.http.put(&target_url, &rendered, &self.credentials)?;
            if !parse_status_ok(&body) {
                let preview = crate::debug::format_body_preview(&body);
                let received = if preview.is_empty() {
                    "<empty response>"
                } else {
                    preview.as_str()
                };
                return Err(GeectlError::new(
                    ErrorKind::ServerError,
                    format!(
                        "unexpected OBS link meta response for package {target_project}/{target_package}; received: {received}"
                    ),
                ));
            }
        }
        let mut link = String::from("<link");
        if source_project != target_project {
            link.push_str(&format!(" project=\"{}\"", xml_escape(source_project)));
        }
        link.push_str(&format!(" package=\"{}\"", xml_escape(source_package)));
        if let Some(rev) = &options.revision {
            link.push_str(&format!(" rev=\"{}\"", xml_escape(rev)));
        }
        if let Some(vrev) = &options.vrev {
            link.push_str(&format!(" vrev=\"{}\"", xml_escape(vrev)));
        }
        if let Some(cicount) = &options.cicount {
            link.push_str(&format!(" cicount=\"{}\"", xml_escape(cicount)));
        }
        if options.missing_target {
            link.push_str(" missingok=\"1\"");
        }
        link.push_str("/>");
        let link_url = build_url(
            &self.base,
            &["source", target_project, target_package, "_link"],
            &[],
        );
        let body = self.http.put_file(&link_url, &link, &self.credentials)?;
        match parse_revision(&body) {
            Some(_) => Ok(()),
            None => {
                let preview = crate::debug::format_body_preview(&body);
                let received = if preview.is_empty() {
                    "<empty response>"
                } else {
                    preview.as_str()
                };
                Err(GeectlError::new(
                    ErrorKind::ServerError,
                    format!(
                        "unexpected OBS link response for package {target_project}/{target_package}; received: {received}"
                    ),
                ))
            }
        }
    }

    /// The `srcmd5` the link in `project`/`package` last expanded
    /// at, from its own previous revisions: the first revision
    /// before the current one whose directory listing expands
    /// without an error. Up to five revisions back; a revision
    /// that cannot be read stops the walk. None when no previous
    /// revision expands, or when the current revision cannot be
    /// read from the directory.
    fn last_working_baserev(
        &self,
        project: &str,
        package: &str,
        directory: &str,
    ) -> Result<Option<String>, GeectlError> {
        let Some(current) = XmlDoc::parse(directory)
            .ok()
            .and_then(|doc| doc.root_attr("rev"))
            .and_then(|rev| rev.parse::<u64>().ok())
        else {
            return Ok(None);
        };
        for rev in (1..current).rev().take(5) {
            let rev_text = rev.to_string();
            let url = build_url(
                &self.base,
                &["source", project, package],
                &[("rev", rev_text.as_str())],
            );
            let Ok(prior_directory) = self.http.get_with(&url, &self.credentials) else {
                break;
            };
            let Ok(prior_info) = parse_linkinfo(&prior_directory) else {
                break;
            };
            let Some(prior_info) = prior_info else {
                continue;
            };
            if prior_info.error.is_some() {
                continue;
            }
            if let Some(srcmd5) = prior_info.srcmd5 {
                return Ok(Some(srcmd5));
            }
        }
        Ok(None)
    }

    /// Repair a broken source link in `project`/`package` on OBS:
    /// strip the apply/delete patches that fail to apply and pin
    /// the link at a revision where it expands, by editing its
    /// `_link` and PUTting it back. Requires credentials; without
    /// them the call fails loudly before any request. In dry-run
    /// the repair is recorded in the plan and nothing is repaired
    /// — the guard runs before the credentials check, like
    /// `create_package`.
    ///
    /// An agent repairs a link when a linked package stops
    /// expanding — usually because the link target moved on and a
    /// patch no longer applies — and the package must work again
    /// without a merge: the patch is dropped rather than fixed,
    /// and the link is pinned at the last revision where it
    /// expanded. The patch files themselves stay in the package
    /// as inert files; deleting them is a separate step.
    ///
    /// The breakage is confirmed first from the directory
    /// listing: its `<linkinfo>` carries the expansion error, and
    /// a package that is not a link, or whose link is not broken,
    /// is refused without writing anything. The pin revision is
    /// then resolved the way the server's own repair does: when
    /// the link still expands at its base, the existing baserev
    /// stands — it already names the target revision the link was
    /// based at. Otherwise the last revision where the link
    /// worked supplies it, through the target revision that
    /// expansion resolved to (the `srcmd5` of the linkinfo at the
    /// last-working revision — a target revision, not the linked
    /// package's expanded one). When the server records no
    /// last-working revision — the breakage came from the link
    /// itself, and the server only records workings of the
    /// current `_link` — the link's own previous revisions are
    /// walked back instead, and the first that still expands
    /// supplies the pin. A link with no apply/delete patch
    /// is not a patch failure and is refused: stripping cannot
    /// fix it, and the server's error is reported instead.
    ///
    /// Success is the backend's `<revision>` document on the
    /// `_link` PUT; any other body is a server error, never
    /// success. A missing package reads as `NotFound` from the
    /// directory GET, before anything is written.
    pub fn repair_link(
        &self,
        project: &str,
        package: &str,
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
        // The dry-run guard: in dry-run the repair is recorded in
        // the plan and nothing is repaired — not even the
        // credentials check runs.
        if !dry_run.allow(&format!("repair link {project}/{package}")) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot repair a link without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let directory_url = build_url(
            &self.base,
            &["source", project, package],
            &[("rev", "latest")],
        );
        let directory = self.http.get_with(&directory_url, &self.credentials)?;
        let linkinfo = parse_linkinfo(&directory)?.ok_or_else(|| {
            GeectlError::new(
                ErrorKind::InvalidInput,
                format!("package {project}/{package} is not a source link"),
            )
        })?;
        let Some(server_error) = linkinfo.error.clone() else {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("source link {project}/{package} is not broken"),
            ));
        };
        // The baserev the repaired link is pinned at, when it
        // changes: None keeps the link's existing baserev.
        let mut new_baserev: Option<String> = None;
        let mut base_ok = false;
        if linkinfo.baserev.is_some() {
            let base_url = build_url(
                &self.base,
                &["source", project, package],
                &[("rev", "latest"), ("linkrev", "base")],
            );
            let base_directory = self.http.get_with(&base_url, &self.credentials)?;
            if let Some(base_info) = parse_linkinfo(&base_directory)? {
                base_ok = base_info.error.is_none();
            }
        }
        if !base_ok {
            let lastworking_url = build_url(
                &self.base,
                &["source", project, package],
                &[("lastworking", "1")],
            );
            let lastworking_directory = self.http.get_with(&lastworking_url, &self.credentials)?;
            let lastworking_info = parse_linkinfo(&lastworking_directory)?.ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    format!("unexpected OBS directory response for package {project}/{package}"),
                )
            })?;
            if lastworking_info.error.is_none() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("source link {project}/{package} is not broken"),
                ));
            }
            new_baserev = Some(match lastworking_info.lastworking {
                Some(lastworking) => {
                    let working_url = build_url(
                        &self.base,
                        &["source", project, package],
                        &[("rev", lastworking.as_str())],
                    );
                    let working_directory = self.http.get_with(&working_url, &self.credentials)?;
                    let working_info = parse_linkinfo(&working_directory)?.ok_or_else(|| {
                        GeectlError::new(
                            ErrorKind::ServerError,
                            format!(
                                "unexpected OBS directory response for package {project}/{package}"
                            ),
                        )
                    })?;
                    if working_info.error.is_some() {
                        return Err(GeectlError::new(
                            ErrorKind::ServerError,
                            format!(
                                "working revision of source link {project}/{package} is not working"
                            ),
                        ));
                    }
                    working_info.srcmd5.ok_or_else(|| {
                        GeectlError::new(
                            ErrorKind::ServerError,
                            format!(
                                "unexpected OBS directory response for package {project}/{package}"
                            ),
                        )
                    })?
                }
                None => {
                    // No last-working revision is recorded: the
                    // breakage came from the link itself. Walk back
                    // through the link's own revisions and take the
                    // first that still expands.
                    let Some(pin) = self.last_working_baserev(project, package, &directory)? else {
                        return Err(GeectlError::new(
                            ErrorKind::InvalidInput,
                            format!("source link {project}/{package} never worked"),
                        ));
                    };
                    pin
                }
            });
        }
        let link_url = build_url(&self.base, &["source", project, package, "_link"], &[]);
        let link_body = self.http.get_with(&link_url, &self.credentials)?;
        let mut doc = XmlDoc::parse(&link_body)?;
        if !repair_link_doc(&mut doc, new_baserev.as_deref())? {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "source link {project}/{package} has no apply/delete patch to repair (server error: {server_error})"
                ),
            ));
        }
        let body = self
            .http
            .put_file(&link_url, &doc.render(), &self.credentials)?;
        if parse_revision(&body).is_none() {
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS link response for package {project}/{package}"),
            ));
        }
        Ok(())
    }

    /// Set the revision of the source link in `project`/`package`
    /// on OBS: write the `rev` (and, for an md5 revision, the
    /// `vrev`) onto its `_link` and PUT it back. Requires
    /// credentials; without them the call fails loudly before any
    /// request. In dry-run the revision is recorded in the plan
    /// and nothing is set — the guard runs before the credentials
    /// check, like `create_package`.
    ///
    /// An agent sets a link revision when a linked package must
    /// stop following its target: pinning it at the revision that
    /// was reviewed, or unsetting it so it follows again. `Latest`
    /// resolves the target's current revision, `Expanded` the
    /// target's current expanded revision (the expanded srcmd5,
    /// with its `vrev`), `Pinned` sets exactly the revision
    /// given without fetching anything, and `Unset` strips the
    /// revision (and the `vrev`) so the link follows its target.
    /// The revision is always PUT back, even when it is unchanged:
    /// the link revision is re-asserted, not deduplicated.
    ///
    /// The `vrev` counts versions within an md5 revision, so it
    /// is only ever set from a directory fetch, and only when the
    /// new revision is an md5; a `vrev` left from another revision
    /// is stale and is dropped. The PUT carries a comment
    /// ("Set link revision to {rev}" / "Unset link revision") so
    /// the package history says what changed.
    ///
    /// Returns the revision now on the link, or None when it was
    /// unset — and None in dry-run, where nothing is set. Success is
    /// the backend's `<revision>` document on
    /// the `_link` PUT; any other body is a server error, never
    /// success. A missing link reads as `NotFound` from the
    /// `_link` GET, before anything is written; expanding a
    /// broken link reads as the server's 400, propagated.
    pub fn set_link_rev(
        &self,
        project: &str,
        package: &str,
        revision: &LinkRevision,
        dry_run: &DryRun,
    ) -> Result<Option<String>, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        let plan = match revision {
            LinkRevision::Unset => format!("unset link revision of {project}/{package}"),
            LinkRevision::Latest => {
                format!("set link revision of {project}/{package} to latest")
            }
            LinkRevision::Pinned(rev) => {
                format!("set link revision of {project}/{package} to {rev}")
            }
            LinkRevision::Expanded => {
                format!("set link revision of {project}/{package} to expanded")
            }
            LinkRevision::PinnedExpanded(rev) => {
                format!("set link revision of {project}/{package} to expanded {rev}")
            }
        };
        // The dry-run guard: in dry-run the revision is recorded
        // in the plan and nothing is set — not even the
        // credentials check runs.
        if !dry_run.allow(&plan) {
            return Ok(None);
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot set a link revision without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let link_url = build_url(&self.base, &["source", project, package, "_link"], &[]);
        let link_body = self.http.get_with(&link_url, &self.credentials)?;
        self.set_link_rev_with_link(project, package, revision, &link_body)
    }

    /// Set the link revision of every linked package in `project`
    /// on OBS, with the same modes as [`ObsClient::set_link_rev`]
    /// except a pinned revision: pinning every link in a project
    /// to one revision is almost certainly a mistake, so a pinned
    /// revision needs a package (use `set_link_rev` for one
    /// package at a time). Packages without a link are skipped.
    /// Requires credentials; without them the call fails loudly
    /// before any request. In dry-run the revision is recorded
    /// in the plan and nothing is set.
    ///
    /// An agent sets a project's link revisions when a whole
    /// project of links must stop following its targets — a test
    /// project frozen at the revisions that were reviewed. The
    /// operation applies per package and may partially apply: a
    /// failure leaves the earlier packages set and is reported
    /// with the package it stopped at.
    ///
    /// Returns the packages whose links were set, in project
    /// order, with the revision now on each link (or None when
    /// unset). In dry-run the returned list is empty: nothing is
    /// set. Success is the backend's `<revision>` document
    /// on each `_link` PUT.
    pub fn set_link_rev_project(
        &self,
        project: &str,
        revision: &LinkRevision,
        dry_run: &DryRun,
    ) -> Result<Vec<(String, Option<String>)>, GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        if matches!(
            revision,
            LinkRevision::Pinned(_) | LinkRevision::PinnedExpanded(_)
        ) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "setting a pinned link revision needs a package: use set_link_rev for one package at a time",
            ));
        }
        let plan = match revision {
            LinkRevision::Unset => {
                format!("unset link revision of every linked package in project {project}")
            }
            LinkRevision::Latest => {
                format!("set link revision of every linked package in project {project} to latest")
            }
            LinkRevision::Expanded => format!(
                "set link revision of every linked package in project {project} to expanded"
            ),
            LinkRevision::Pinned(_) | LinkRevision::PinnedExpanded(_) => {
                unreachable!("pinned revisions are refused above")
            }
        };
        if !dry_run.allow(&plan) {
            return Ok(Vec::new());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot set a link revision without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(&self.base, &["source", project], &[]);
        let directory = self.http.get_with(&url, &self.credentials)?;
        let packages = parse_package_names(&directory)?;
        let mut set = Vec::new();
        for package in packages {
            let link_url = build_url(
                &self.base,
                &["source", project, package.as_str(), "_link"],
                &[],
            );
            let link_body = match self.http.get_with(&link_url, &self.credentials) {
                Ok(body) => body,
                Err(err) if err.kind() == ErrorKind::NotFound => continue,
                Err(err) => return Err(err),
            };
            let rev = self.set_link_rev_with_link(project, &package, revision, &link_body)?;
            set.push((package, rev));
        }
        Ok(set)
    }

    /// Set the link revision on an already-fetched `_link`
    /// document: resolve the revision when the mode needs it,
    /// apply the `rev`/`vrev` attributes, and PUT the link back.
    /// The credentials check and the `_link` fetch live in the
    /// callers, so the project form can skip packages without a
    /// link instead of failing on them.
    fn set_link_rev_with_link(
        &self,
        project: &str,
        package: &str,
        revision: &LinkRevision,
        link_body: &str,
    ) -> Result<Option<String>, GeectlError> {
        let mut doc = XmlDoc::parse(link_body)?;
        // The link target: the link's own project/package
        // attributes, defaulting to the package the link lives in.
        let target_project = doc
            .root_attr("project")
            .unwrap_or_else(|| project.to_string());
        let target_package = doc
            .root_attr("package")
            .unwrap_or_else(|| package.to_string());
        let (new_rev, new_vrev): (Option<String>, Option<String>) = match revision {
            LinkRevision::Unset => (None, None),
            LinkRevision::Pinned(rev) => (Some(rev.clone()), None),
            LinkRevision::Latest => {
                let (rev, vrev) =
                    self.resolve_link_rev(&target_project, &target_package, None, false)?;
                (Some(rev), vrev)
            }
            LinkRevision::Expanded => {
                let (rev, vrev) =
                    self.resolve_link_rev(&target_project, &target_package, None, true)?;
                (Some(rev), vrev)
            }
            LinkRevision::PinnedExpanded(rev) => {
                let (resolved, vrev) =
                    self.resolve_link_rev(&target_project, &target_package, Some(rev), true)?;
                (Some(resolved), vrev)
            }
        };
        match &new_rev {
            Some(rev) => doc.set_root_attr("rev", rev),
            None => doc.remove_root_attr("rev"),
        }
        // A vrev counts versions within an md5 revision: it is
        // only ever set from a directory fetch, and only when the
        // new revision is an md5. Any vrev left from another
        // revision is stale and is dropped.
        match &new_vrev {
            Some(vrev) => doc.set_root_attr("vrev", vrev),
            None => doc.remove_root_attr("vrev"),
        }
        let comment = match &new_rev {
            Some(rev) => format!("Set link revision to {rev}"),
            None => "Unset link revision".to_string(),
        };
        let link_url = build_url(
            &self.base,
            &["source", project, package, "_link"],
            &[("comment", comment.as_str())],
        );
        let body = self
            .http
            .put_file(&link_url, &doc.render(), &self.credentials)?;
        if parse_revision(&body).is_none() {
            let preview = crate::debug::format_body_preview(&body);
            let received = if preview.is_empty() {
                "<empty response>"
            } else {
                preview.as_str()
            };
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected OBS link response for package {project}/{package}; received: {received}"
                ),
            ));
        }
        Ok(new_rev)
    }

    /// Resolve the revision a link is set to from the link
    /// target's directory: the directory's `rev` attribute, with
    /// its `vrev` when the revision is an md5 (exactly 32 hex
    /// characters — a vrev counts versions within an md5 revision, so
    /// a numeric revision has none). `at` pins the directory
    /// revision; `expand` asks for the expanded source. A failed
    /// directory read (expanding a broken link reads as a 400)
    /// propagates: there is no revision to set.
    fn resolve_link_rev(
        &self,
        target_project: &str,
        target_package: &str,
        at: Option<&str>,
        expand: bool,
    ) -> Result<(String, Option<String>), GeectlError> {
        let mut query: Vec<(&str, &str)> = vec![("rev", at.unwrap_or("latest"))];
        if expand {
            query.push(("expand", "1"));
        }
        let url = build_url(
            &self.base,
            &["source", target_project, target_package],
            &query,
        );
        let directory = self.http.get_with(&url, &self.credentials)?;
        let (rev, vrev, srcmd5) = parse_directory_rev(&directory)?;
        // An expanded directory reports the revision number in
        // `rev` and the expanded content's md5 in `srcmd5`; the
        // link revision is the md5. (The server rewrites `rev` to
        // the srcmd5 only when the listed package itself is a link
        // being expanded, which the target here is not.)
        let rev = if expand { srcmd5 } else { rev };
        let Some(rev) = rev else {
            let preview = crate::debug::format_body_preview(&directory);
            let received = if preview.is_empty() {
                "<empty response>"
            } else {
                preview.as_str()
            };
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected OBS directory response for package {target_project}/{target_package}; received: {received}"
                ),
            ));
        };
        // A vrev counts versions within an md5 revision: it is only
        // kept when the revision is exactly 32 hex characters. A
        // longer hex string is not an md5, and a numeric revision has
        // no vrev.
        let vrev = if rev.len() == 32 && rev.chars().all(|c| c.is_ascii_hexdigit()) {
            vrev
        } else {
            None
        };
        Ok((rev, vrev))
    }

    /// Convert a package holding a classic source link into a branch
    /// on OBS (`POST /source/<project>/<package>?cmd=linktobranch`).
    /// Requires credentials; without them the call fails loudly
    /// before any request. In dry-run the conversion is recorded in
    /// the plan and nothing is converted — the guard runs before the
    /// credentials check, like `create_package`.
    ///
    /// An agent converts a link to a branch when it wants to work on
    /// the linked sources as its own package while keeping the
    /// connection to the link target: the server expands the link into
    /// the package's own files, sets the link's baserev to the target
    /// revision it expanded at, and replaces the link's patches with a
    /// single `<branch/>` entry — the package keeps pointing at its
    /// target, so a later submit diffs against it. The link target
    /// itself is unchanged.
    ///
    /// The server refuses a package that is not a link ("package is
    /// not a link") and a link that is already a branch ("package is
    /// already a branch"); both surface as server errors, so no
    /// client-side pre-check is needed. The route also takes `rev`,
    /// `user`, `comment`, `linkrev`, and `noservice`, but they are not
    /// exposed here: the conversion uses the current revision and the
    /// server's defaults.
    ///
    /// Success is the `<status code="ok"/>` body; any other body is a
    /// server error, never success.
    pub fn link_to_branch(
        &self,
        project: &str,
        package: &str,
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
        // The dry-run guard: in dry-run the conversion is recorded
        // in the plan and nothing is converted — not even the
        // credentials check runs.
        if !dry_run.allow(&format!("convert link {project}/{package} to a branch")) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot convert a link to a branch without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(
            &self.base,
            &["source", project, package],
            &[("cmd", "linktobranch")],
        );
        let body = self.http.post(&url, "", &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS linktobranch response for package {project}/{package}"),
            ))
        }
    }

    /// Replace a link with its expanded sources on OBS: copy the
    /// package onto itself with the link expanded, so the link is
    /// gone and the package holds the sources as its own files
    /// (`POST /source/<project>/<package>?cmd=copy&...&expand=1`).
    /// Requires credentials; without them the call fails loudly
    /// before any request. In dry-run the detach is recorded in the
    /// plan and nothing is detached — the guard runs before the
    /// credentials check, like `create_package`. Returns the new
    /// revision, or None in dry-run — and None when the link is
    /// dropped instead of copied, because the file delete reports
    /// only status-ok and no revision.
    ///
    /// An agent detaches a branch when it no longer wants the package
    /// tied to its link target: after the detach the package builds
    /// from its own files and there is no target left to submit
    /// against. The package meta is untouched.
    ///
    /// When the copy fails, the failure is triaged from the package's
    /// link state, because the common cause is a broken link: when
    /// the link is broken and its target is gone, the link is dropped
    /// instead (the expanded sources are already lost with the
    /// target, so there is nothing to keep) and the detach still
    /// succeeds. A package that is not a link at all is refused —
    /// there is nothing to detach — and any other copy failure
    /// propagates unchanged.
    pub fn detach_branch(
        &self,
        project: &str,
        package: &str,
        dry_run: &DryRun,
    ) -> Result<Option<u64>, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        // The dry-run guard: in dry-run the detach is recorded in
        // the plan and nothing is detached — not even the credentials
        // check runs.
        if !dry_run.allow(&format!(
            "detach branch {project}/{package} (replace the link with its expanded sources)"
        )) {
            return Ok(None);
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot detach a branch without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let options = CopyOptions {
            expand: true,
            ..CopyOptions::default()
        };
        match self.copy_package(project, package, project, package, &options, dry_run) {
            Ok(rev) => Ok(rev),
            Err(copy_err) => self.detach_branch_on_copy_failure(project, package, copy_err),
        }
    }

    /// Triage a failed detach copy from the package's link state. A
    /// broken link whose target is gone is dropped instead; a package
    /// that is not a link is refused; anything else keeps the
    /// original copy error.
    fn detach_branch_on_copy_failure(
        &self,
        project: &str,
        package: &str,
        copy_err: GeectlError,
    ) -> Result<Option<u64>, GeectlError> {
        let directory_url = build_url(
            &self.base,
            &["source", project, package],
            &[("rev", "latest")],
        );
        let Ok(directory) = self.http.get_with(&directory_url, &self.credentials) else {
            return Err(copy_err);
        };
        let Ok(linkinfo) = parse_linkinfo(&directory) else {
            return Err(copy_err);
        };
        let Some(linkinfo) = linkinfo else {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("package {project}/{package} is not a source link"),
            ));
        };
        if linkinfo.error.is_none() {
            return Err(copy_err);
        }
        // The link target: the linkinfo's own project/package
        // attributes, defaulting to the package the link lives in.
        let target_project = linkinfo.project.as_deref().unwrap_or(project);
        let target_package = linkinfo.package.as_deref().unwrap_or(package);
        let target_meta_url = build_url(
            &self.base,
            &["source", target_project, target_package, "_meta"],
            &[],
        );
        match self.http.get_with(&target_meta_url, &self.credentials) {
            Ok(_) => Err(copy_err),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                // The link target is gone: the expanded sources are
                // already lost with it, so drop the link. The API's
                // file delete reports only status-ok, not a revision,
                // so the detach reports no revision here.
                self.delete_file(project, package, "_link")?;
                Ok(None)
            }
            Err(_) => Err(copy_err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use crate::obs::test_support::{basic_credentials, FakeHttp};
    use crate::xml::XmlDoc;

    #[test]
    fn branch_package_posts_branch_command_and_returns_target() {
        let body = "<status code=\"ok\"><summary>Ok</summary>\
            <data name=\"target_project\">home:scarabeus_iv:branches:test:factory</data>\
            <data name=\"target_package\">test-pkgA</data>\
            <data name=\"project\">test:factory</data>\
            <data name=\"package\">test-pkgA</data></status>";
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory/test-pkgA?cmd=branch",
            body,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let result = client
            .branch_package(
                "test:factory",
                "test-pkgA",
                &BranchOptions::default(),
                &DryRun::new(false),
            )
            .unwrap();
        assert_eq!(
            result,
            Some(BranchResult {
                source_project: "test:factory".to_string(),
                source_package: "test-pkgA".to_string(),
                target_project: "home:scarabeus_iv:branches:test:factory".to_string(),
                target_package: "test-pkgA".to_string(),
            })
        );
        let posts = http.post_calls();
        assert_eq!(posts.len(), 1);
        // No target was given, so the request omits both keys and
        // lets the server apply its default.
        assert_eq!(
            posts[0].0,
            "https://api.opensuse.org/source/test:factory/test-pkgA?cmd=branch"
        );
        // The branch POST carries no body (the parameters are the
        // query) and the credentials.
        assert_eq!(posts[0].1, "");
        assert!(posts[0].2);
        // A plain branch makes a single POST: no target pre-check.
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn branch_package_new_package_missing_source_sends_missingok() {
        let body = "<status code=\"ok\"><summary>Ok</summary>\
            <data name=\"target_project\">home:scarabeus_iv:branches:test:factory</data>\
            <data name=\"target_package\">test-pkgA</data>\
            <data name=\"project\">test:factory</data>\
            <data name=\"package\">test-pkgA</data></status>";
        let http = FakeHttp::new()
            .with_get_error(
                "https://api.opensuse.org/source/test:factory/test-pkgA",
                ErrorKind::NotFound,
            )
            .with_post(
                "https://api.opensuse.org/source/test:factory/test-pkgA?cmd=branch&missingok=1",
                body,
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = BranchOptions {
            new_package: true,
            ..BranchOptions::default()
        };
        let result = client
            .branch_package("test:factory", "test-pkgA", &options, &DryRun::new(false))
            .unwrap();
        assert!(result.is_some());
        // The source was read first, then branched with missingok.
        assert_eq!(http.get_calls().len(), 1);
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn branch_package_new_package_existing_source_sends_newinstance() {
        let body = "<status code=\"ok\"><summary>Ok</summary>\
            <data name=\"target_project\">home:scarabeus_iv:branches:test:factory</data>\
            <data name=\"target_package\">test-pkgA</data>\
            <data name=\"project\">test:factory</data>\
            <data name=\"package\">test-pkgA</data></status>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA",
                "<directory name=\"test-pkgA\" />",
            )
            .with_post(
                "https://api.opensuse.org/source/test:factory/test-pkgA?cmd=branch&newinstance=1",
                body,
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = BranchOptions {
            new_package: true,
            ..BranchOptions::default()
        };
        let result = client
            .branch_package("test:factory", "test-pkgA", &options, &DryRun::new(false))
            .unwrap();
        assert!(result.is_some());
        assert_eq!(http.get_calls().len(), 1);
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn branch_package_no_devel_project_sends_ignoredevel() {
        let body = "<status code=\"ok\"><summary>Ok</summary>\
            <data name=\"target_project\">home:scarabeus_iv:branches:test:factory</data>\
            <data name=\"target_package\">test-pkgA</data>\
            <data name=\"project\">test:factory</data>\
            <data name=\"package\">test-pkgA</data></status>";
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory/test-pkgA?cmd=branch&ignoredevel=1",
            body,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = BranchOptions {
            no_devel_project: true,
            ..BranchOptions::default()
        };
        assert!(client
            .branch_package("test:factory", "test-pkgA", &options, &DryRun::new(false))
            .is_ok());
    }

    #[test]
    fn branch_package_explicit_target_and_options() {
        let body = "<status code=\"ok\"><summary>Ok</summary>\
            <data name=\"target_project\">home:alice:branches:test:factory</data>\
            <data name=\"target_package\">renamed</data>\
            <data name=\"project\">test:factory</data>\
            <data name=\"package\">test-pkgA</data></status>";
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory/test-pkgA?cmd=branch&target_project=home%3Aalice%3Abranches%3Atest%3Afactory&target_package=renamed&ignoredevel=1&force=1&rev=42&linkrev=abc123&comment=test-branch&noaccess=1&add_repositories=1&add_repositories_block=standard&add_repositories_rebuild=standard&extend_package_names=1&maintenance=1",
            body,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = BranchOptions {
            target_project: Some("home:alice:branches:test:factory".to_string()),
            target_package: Some("renamed".to_string()),
            no_devel_project: true,
            force: true,
            revision: Some("42".to_string()),
            linkrev: Some("abc123".to_string()),
            comment: Some("test-branch".to_string()),
            noaccess: true,
            add_repositories: true,
            add_repositories_block: Some("standard".to_string()),
            add_repositories_rebuild: Some("standard".to_string()),
            extend_package_names: true,
            maintenance: true,
            ..BranchOptions::default()
        };
        let result = client
            .branch_package("test:factory", "test-pkgA", &options, &DryRun::new(false))
            .unwrap();
        assert_eq!(result.unwrap().target_package, "renamed".to_string());
    }

    #[test]
    fn branch_package_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory/test-pkgA?cmd=branch",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .branch_package(
                "test:factory",
                "test-pkgA",
                &BranchOptions::default(),
                &DryRun::new(false)
            )
            .is_err());
        assert!(http.post_calls().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn branch_package_rejects_invalid_names_without_http() {
        // An invalid name is rejected before any request: no canned
        // response exists, so reaching the HTTP client would error
        // differently (FakeHttp: "no canned response").
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .branch_package(
                "test/factory",
                "test-pkgA",
                &BranchOptions::default(),
                &DryRun::new(false)
            )
            .is_err());
        let options = BranchOptions {
            target_project: Some("home/alice".to_string()),
            ..BranchOptions::default()
        };
        assert!(client
            .branch_package("test:factory", "test-pkgA", &options, &DryRun::new(false))
            .is_err());
        assert!(http.post_calls().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn branch_package_dry_run_records_plan_and_branches_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        let result = client
            .branch_package(
                "test:factory",
                "test-pkgA",
                &BranchOptions::default(),
                &dry_run,
            )
            .unwrap();
        assert_eq!(result, None);
        assert_eq!(
            dry_run.plan(),
            vec![
                "[dry-run] would: branch package test:factory/test-pkgA to the server default branch project/test-pkgA"
                    .to_string()
            ]
        );
        assert!(http.post_calls().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn branch_package_dry_run_names_revision_linkrev_and_comment() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        let options = BranchOptions {
            revision: Some("42".to_string()),
            linkrev: Some("7".to_string()),
            comment: Some("backport".to_string()),
            ..BranchOptions::default()
        };
        let result = client
            .branch_package("test:factory", "test-pkgA", &options, &dry_run)
            .unwrap();
        assert_eq!(result, None);
        assert_eq!(
            dry_run.plan(),
            vec![
                "[dry-run] would: branch package test:factory/test-pkgA to the server default branch project/test-pkgA (revision 42, link revision 7, comment backport)"
                    .to_string()
            ]
        );
        assert!(http.post_calls().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn branch_package_dry_run_without_credentials_names_no_target() {
        // Without credentials the default branch project cannot be
        // named, so the plan says so instead of guessing — and still
        // makes no request.
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let dry_run = DryRun::new(true);
        let result = client
            .branch_package(
                "test:factory",
                "test-pkgA",
                &BranchOptions::default(),
                &dry_run,
            )
            .unwrap();
        assert_eq!(result, None);
        assert_eq!(
            dry_run.plan(),
            vec![
                "[dry-run] would: branch package test:factory/test-pkgA to the server default branch project/test-pkgA"
                    .to_string()
            ]
        );
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn branch_package_non_ok_body_is_error_not_success() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory/test-pkgA?cmd=branch",
            "<status code=\"error\"><summary>nope</summary></status>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .branch_package(
                "test:factory",
                "test-pkgA",
                &BranchOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn parse_branch_result_prefers_current_data_names() {
        let body = "<status code=\"ok\"><summary>Ok</summary>\
            <data name=\"targetproject\">old:project</data>\
            <data name=\"target_project\">home:alice:branches:test:factory</data>\
            <data name=\"targetpackage\">old-pkg</data>\
            <data name=\"target_package\">test-pkgA</data>\
            <data name=\"sourceproject\">old:source</data>\
            <data name=\"project\">test:factory</data>\
            <data name=\"sourcepackage\">old-src</data>\
            <data name=\"package\">test-pkgA</data></status>";
        assert_eq!(
            parse_branch_result(body),
            Some(BranchResult {
                source_project: "test:factory".to_string(),
                source_package: "test-pkgA".to_string(),
                target_project: "home:alice:branches:test:factory".to_string(),
                target_package: "test-pkgA".to_string(),
            })
        );
    }

    #[test]
    fn parse_branch_result_falls_back_to_deprecated_data_names() {
        let body = "<status code=\"ok\"><summary>Ok</summary>\
            <data name=\"targetproject\">home:alice:branches:test:factory</data>\
            <data name=\"targetpackage\">test-pkgA</data>\
            <data name=\"sourceproject\">test:factory</data>\
            <data name=\"sourcepackage\">test-pkgA</data></status>";
        assert_eq!(
            parse_branch_result(body),
            Some(BranchResult {
                source_project: "test:factory".to_string(),
                source_package: "test-pkgA".to_string(),
                target_project: "home:alice:branches:test:factory".to_string(),
                target_package: "test-pkgA".to_string(),
            })
        );
    }

    #[test]
    fn parse_branch_result_rejects_non_ok_and_incomplete_bodies() {
        assert_eq!(
            parse_branch_result("<status code=\"error\"><summary>nope</summary></status>"),
            None
        );
        // A missing data element is not a result.
        assert_eq!(
            parse_branch_result(
                "<status code=\"ok\"><summary>Ok</summary>\
                <data name=\"target_project\">home:alice:branches:test:factory</data></status>"
            ),
            None
        );
        // An empty value is not a result.
        assert_eq!(
            parse_branch_result(
                "<status code=\"ok\"><summary>Ok</summary>\
                <data name=\"target_project\"></data>\
                <data name=\"target_package\">test-pkgA</data>\
                <data name=\"project\">test:factory</data>\
                <data name=\"package\">test-pkgA</data></status>"
            ),
            None
        );
        assert_eq!(parse_branch_result(""), None);
    }

    #[test]
    fn parse_fork_result_parses_a_package_fork() {
        // The shape mirrors the real fork of jengelh/descent3:
        // the OBS fork of games/descent3 lands in
        // home:fbui:branches:games (see the layout note in
        // src/gitea.rs).
        let body = "<status code=\"ok\"><summary>Ok</summary>\
            <data name=\"targetproject\">home:fbui:branches:games</data>\
            <data name=\"targetpackage\">descent3</data>\
            <data name=\"sourceproject\">games</data>\
            <data name=\"sourcepackage\">descent3</data></status>";
        assert_eq!(
            parse_fork_result(body, "descent3"),
            Some(BranchResult {
                source_project: "games".to_string(),
                source_package: "descent3".to_string(),
                target_project: "home:fbui:branches:games".to_string(),
                target_package: "descent3".to_string(),
            })
        );
    }

    #[test]
    fn parse_fork_result_defaults_a_project_fork_to_project() {
        // A project fork answers only targetproject /
        // sourceproject — no package names at all. The shape
        // mirrors the real fork of the network:messaging:xmpp
        // project.
        let body = "<status code=\"ok\"><summary>Ok</summary>\
            <data name=\"targetproject\">home:fbui:branches:network:messaging:xmpp</data>\
            <data name=\"sourceproject\">network:messaging:xmpp</data></status>";
        assert_eq!(
            parse_fork_result(body, "_project"),
            Some(BranchResult {
                source_project: "network:messaging:xmpp".to_string(),
                source_package: "_project".to_string(),
                target_project: "home:fbui:branches:network:messaging:xmpp".to_string(),
                target_package: "_project".to_string(),
            })
        );
        // The same answer for a package fork is malformed: the
        // package names are missing, not defaulted.
        assert_eq!(parse_fork_result(body, "test-pkgA"), None);
        assert_eq!(
            parse_fork_result(
                "<status code=\"error\"><summary>nope</summary></status>",
                "_project"
            ),
            None
        );
    }

    #[test]
    fn copy_package_posts_copy_command_and_returns_revision() {
        // The copy response is the backend's revision document, not
        // a status: the revision number is the result.
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:target/test-pkgA?cmd=copy&oproject=test%3Afactory&opackage=test-pkgA",
            "<revision rev=\"3\" vrev=\"1\" srcmd5=\"abc\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let result = client
            .copy_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &CopyOptions::default(),
                &DryRun::new(false),
            )
            .unwrap();
        assert_eq!(result, Some(3));
        let posts = http.post_calls();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].1, "");
        assert!(posts[0].2);
    }

    #[test]
    fn copy_package_with_rename_and_options() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:target/renamed?cmd=copy&oproject=test%3Afactory&opackage=test-pkgA&orev=42&keeplink=1&expand=1&comment=copy-for-test&noservice=1&dontupdatesource=1&withvrev=1",
            "<revision rev=\"5\" vrev=\"2\" srcmd5=\"def\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = CopyOptions {
            revision: Some("42".to_string()),
            keep_link: true,
            expand: true,
            comment: Some("copy-for-test".to_string()),
            noservice: true,
            dont_update_source: true,
            with_vrev: true,
        };
        let result = client
            .copy_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "renamed",
                &options,
                &DryRun::new(false),
            )
            .unwrap();
        assert_eq!(result, Some(5));
    }

    #[test]
    fn copy_package_self_copy_without_flags_is_refused() {
        // A self-copy with nothing to copy from is refused
        // client-side, before any request: no canned response exists,
        // so reaching the HTTP client would error differently.
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .copy_package(
                "test:factory",
                "test-pkgA",
                "test:factory",
                "test-pkgA",
                &CopyOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn copy_package_self_copy_with_expand_is_allowed() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory/test-pkgA?cmd=copy&oproject=test%3Afactory&opackage=test-pkgA&expand=1",
            "<revision rev=\"2\" vrev=\"1\" srcmd5=\"abc\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = CopyOptions {
            expand: true,
            ..CopyOptions::default()
        };
        assert!(client
            .copy_package(
                "test:factory",
                "test-pkgA",
                "test:factory",
                "test-pkgA",
                &options,
                &DryRun::new(false),
            )
            .is_ok());
    }

    #[test]
    fn copy_package_self_copy_with_revision_is_allowed() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory/test-pkgA?cmd=copy&oproject=test%3Afactory&opackage=test-pkgA&orev=1",
            "<revision rev=\"2\" vrev=\"1\" srcmd5=\"abc\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = CopyOptions {
            revision: Some("1".to_string()),
            ..CopyOptions::default()
        };
        assert!(client
            .copy_package(
                "test:factory",
                "test-pkgA",
                "test:factory",
                "test-pkgA",
                &options,
                &DryRun::new(false),
            )
            .is_ok());
    }

    #[test]
    fn copy_package_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:target/test-pkgA?cmd=copy&oproject=test%3Afactory&opackage=test-pkgA",
            "<revision rev=\"3\" vrev=\"1\" srcmd5=\"abc\" />",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .copy_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &CopyOptions::default(),
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn copy_package_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .copy_package(
                "test/factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &CopyOptions::default(),
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn copy_package_dry_run_records_plan_and_copies_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        let result = client
            .copy_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &CopyOptions::default(),
                &dry_run,
            )
            .unwrap();
        assert_eq!(result, None);
        assert_eq!(
            dry_run.plan(),
            vec![
                "[dry-run] would: copy package test:factory/test-pkgA to test:target/test-pkgA"
                    .to_string()
            ]
        );
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn copy_package_dry_run_names_revision_and_comment() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        let options = CopyOptions {
            revision: Some("1".to_string()),
            comment: Some("pick".to_string()),
            ..CopyOptions::default()
        };
        let result = client
            .copy_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &options,
                &dry_run,
            )
            .unwrap();
        assert_eq!(result, None);
        assert_eq!(
            dry_run.plan(),
            vec![
                "[dry-run] would: copy package test:factory/test-pkgA to test:target/test-pkgA (revision 1, comment pick)"
                    .to_string()
            ]
        );
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn copy_package_non_revision_body_is_error_not_success() {
        // A status-ok body is not a copy result: the copy response
        // must be a revision document.
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:target/test-pkgA?cmd=copy&oproject=test%3Afactory&opackage=test-pkgA",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .copy_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &CopyOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn copy_project_posts_copy_command_and_returns_ok() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:target?cmd=copy&oproject=test%3Afactory",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .copy_project(
                "test:factory",
                "test:target",
                &CopyProjectOptions::default(),
                &DryRun::new(false),
            )
            .is_ok());
        let posts = http.post_calls();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].1, "");
        assert!(posts[0].2);
    }

    #[test]
    fn copy_project_invoked_is_success() {
        // The default (queued) copy answers invoked, not ok: both
        // are acceptance.
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:target?cmd=copy&oproject=test%3Afactory",
            "<status code=\"invoked\"><summary>Job invoked</summary></status>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .copy_project(
                "test:factory",
                "test:target",
                &CopyProjectOptions::default(),
                &DryRun::new(false),
            )
            .is_ok());
    }

    #[test]
    fn copy_project_no_delay_and_options() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:target?cmd=copy&oproject=test%3Afactory&withbinaries=1&withhistory=1&makeolder=1&makeoriginolder=1&noservice=1&resign=1&comment=project-copy&nodelay=1",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = CopyProjectOptions {
            with_binaries: true,
            with_history: true,
            make_older: true,
            make_origin_older: true,
            noservice: true,
            resign: true,
            comment: Some("project-copy".to_string()),
            no_delay: true,
        };
        assert!(client
            .copy_project("test:factory", "test:target", &options, &DryRun::new(false))
            .is_ok());
    }

    #[test]
    fn copy_project_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:target?cmd=copy&oproject=test%3Afactory",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .copy_project(
                "test:factory",
                "test:target",
                &CopyProjectOptions::default(),
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn copy_project_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .copy_project(
                "test/factory",
                "test:target",
                &CopyProjectOptions::default(),
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn copy_project_dry_run_records_plan_and_copies_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        assert!(client
            .copy_project(
                "test:factory",
                "test:target",
                &CopyProjectOptions::default(),
                &dry_run
            )
            .is_ok());
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: copy project test:factory to test:target".to_string()]
        );
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn copy_project_dry_run_names_comment() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        let options = CopyProjectOptions {
            comment: Some("move".to_string()),
            ..CopyProjectOptions::default()
        };
        assert!(client
            .copy_project("test:factory", "test:target", &options, &dry_run)
            .is_ok());
        assert_eq!(
            dry_run.plan(),
            vec![
                "[dry-run] would: copy project test:factory to test:target (comment move)"
                    .to_string()
            ]
        );
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn copy_project_non_ok_body_is_error_not_success() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:target?cmd=copy&oproject=test%3Afactory",
            "<status code=\"error\"><summary>nope</summary></status>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .copy_project(
                "test:factory",
                "test:target",
                &CopyProjectOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn link_package_creates_link_from_source_meta() {
        let source_meta = "<package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description><devel project=\"test:devel\"/><person userid=\"alice\" role=\"maintainer\"/><lock><enable/></lock><scmsync url=\"https://example.com#main\"/></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
                source_meta,
            )
            .with_get_error(
                "https://api.opensuse.org/source/test:target/test-pkgA/_meta",
                ErrorKind::NotFound,
            )
            .with_put(
                "https://api.opensuse.org/source/test:target/test-pkgA/_meta",
                "<status code=\"ok\" />",
            )
            .with_put(
                "https://api.opensuse.org/source/test:target/test-pkgA/_link",
                "<revision rev=\"1\" vrev=\"1\" srcmd5=\"abc\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .link_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &LinkOptions::default(),
                &DryRun::new(false),
            )
            .is_ok());
        // The meta PUT comes first (built from the source meta, with
        // the devel, person, lock, and scmsync entries stripped),
        // then the _link PUT.
        let puts = http.put_calls();
        assert_eq!(puts.len(), 2);
        assert_eq!(
            puts[0].1,
            "<package name=\"test-pkgA\" project=\"test:target\"><title>Test</title><description>A test package</description></package>"
        );
        assert_eq!(
            puts[1].1,
            "<link project=\"test:factory\" package=\"test-pkgA\"/>"
        );
        assert!(puts[0].2 && puts[1].2);
        assert_eq!(http.get_calls().len(), 2);
    }

    #[test]
    fn link_package_existing_target_meta_is_edited() {
        let source_meta = "<package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description></package>";
        let target_meta = "<package name=\"test-pkgA\" project=\"test:target\"><title>Target</title><description>Old</description><scmsync url=\"https://example.com#main\"/></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
                source_meta,
            )
            .with_get(
                "https://api.opensuse.org/source/test:target/test-pkgA/_meta",
                target_meta,
            )
            .with_get_error(
                "https://api.opensuse.org/source/test:target/test-pkgA/_link",
                ErrorKind::NotFound,
            )
            .with_put(
                "https://api.opensuse.org/source/test:target/test-pkgA/_meta",
                "<status code=\"ok\" />",
            )
            .with_put(
                "https://api.opensuse.org/source/test:target/test-pkgA/_link",
                "<revision rev=\"1\" vrev=\"1\" srcmd5=\"abc\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = LinkOptions {
            disable_build: true,
            ..LinkOptions::default()
        };
        assert!(client
            .link_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &options,
                &DryRun::new(false),
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 2);
        // The target keeps its own scmsync and the build disable is
        // added; the target's own title and description survive.
        assert_eq!(
            puts[0].1,
            "<package name=\"test-pkgA\" project=\"test:target\"><title>Target</title><description>Old</description><scmsync url=\"https://example.com#main\"/><build><disable/></build></package>"
        );
    }

    #[test]
    fn link_package_disable_publish_adds_publish_disable() {
        let source_meta = "<package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
                source_meta,
            )
            .with_get_error(
                "https://api.opensuse.org/source/test:target/test-pkgA/_meta",
                ErrorKind::NotFound,
            )
            .with_put(
                "https://api.opensuse.org/source/test:target/test-pkgA/_meta",
                "<status code=\"ok\" />",
            )
            .with_put(
                "https://api.opensuse.org/source/test:target/test-pkgA/_link",
                "<revision rev=\"1\" vrev=\"1\" srcmd5=\"abc\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = LinkOptions {
            disable_publish: true,
            ..LinkOptions::default()
        };
        assert!(client
            .link_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &options,
                &DryRun::new(false),
            )
            .is_ok());
        assert!(http.put_calls()[0]
            .1
            .contains("<publish><disable/></publish>"));
    }

    #[test]
    fn link_package_refuses_existing_link_without_force() {
        let source_meta = "<package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description></package>";
        let target_meta = "<package name=\"test-pkgA\" project=\"test:target\"><title>Target</title><description>Old</description></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
                source_meta,
            )
            .with_get(
                "https://api.opensuse.org/source/test:target/test-pkgA/_meta",
                target_meta,
            )
            .with_get(
                "https://api.opensuse.org/source/test:target/test-pkgA/_link",
                "<link project=\"test:factory\" package=\"test-pkgA\"/>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .link_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &LinkOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        // The refusal happens before anything is written: no PUT at
        // all, not even the meta.
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn link_package_force_overwrites_existing_link() {
        let source_meta = "<package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description></package>";
        let target_meta = "<package name=\"test-pkgA\" project=\"test:target\"><title>Target</title><description>Old</description></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
                source_meta,
            )
            .with_get(
                "https://api.opensuse.org/source/test:target/test-pkgA/_meta",
                target_meta,
            )
            .with_get(
                "https://api.opensuse.org/source/test:target/test-pkgA/_link",
                "<link project=\"test:factory\" package=\"test-pkgA\"/>",
            )
            .with_put(
                "https://api.opensuse.org/source/test:target/test-pkgA/_link",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"def\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = LinkOptions {
            force: true,
            ..LinkOptions::default()
        };
        assert!(client
            .link_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &options,
                &DryRun::new(false),
            )
            .is_ok());
        // No meta PUT (nothing changed there); the _link PUT happened.
        assert_eq!(http.put_calls().len(), 1);
        assert_eq!(
            http.put_calls()[0].0,
            "https://api.opensuse.org/source/test:target/test-pkgA/_link"
        );
    }

    #[test]
    fn link_package_same_project_omits_project_attr() {
        let source_meta = "<package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
                source_meta,
            )
            .with_get_error(
                "https://api.opensuse.org/source/test:factory/test-pkgB/_meta",
                ErrorKind::NotFound,
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/test-pkgB/_meta",
                "<status code=\"ok\" />",
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/test-pkgB/_link",
                "<revision rev=\"1\" vrev=\"1\" srcmd5=\"abc\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .link_package(
                "test:factory",
                "test-pkgA",
                "test:factory",
                "test-pkgB",
                &LinkOptions::default(),
                &DryRun::new(false),
            )
            .is_ok());
        assert_eq!(http.put_calls()[1].1, "<link package=\"test-pkgA\"/>");
    }

    #[test]
    fn link_package_project_link_shadowing_builds_from_source() {
        // The target meta names another project: the package only
        // exists through a project link, so the meta is built from
        // the source (not from the shadow) and no _link check runs.
        let source_meta = "<package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description></package>";
        let shadow_meta = "<package name=\"test-pkgA\" project=\"test:factory\"><title>Shadow</title><description>Shadowed</description></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
                source_meta,
            )
            .with_get(
                "https://api.opensuse.org/source/test:target/test-pkgA/_meta",
                shadow_meta,
            )
            .with_put(
                "https://api.opensuse.org/source/test:target/test-pkgA/_meta",
                "<status code=\"ok\" />",
            )
            .with_put(
                "https://api.opensuse.org/source/test:target/test-pkgA/_link",
                "<revision rev=\"1\" vrev=\"1\" srcmd5=\"abc\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .link_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &LinkOptions::default(),
                &DryRun::new(false),
            )
            .is_ok());
        // Only the two meta GETs: no _link existence check for a
        // shadowed package.
        assert_eq!(http.get_calls().len(), 2);
        assert_eq!(
            http.put_calls()[0].1,
            "<package name=\"test-pkgA\" project=\"test:target\"><title>Test</title><description>A test package</description></package>"
        );
    }

    #[test]
    fn link_package_no_change_to_existing_meta_skips_put() {
        let source_meta = "<package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description></package>";
        let target_meta = "<package name=\"test-pkgA\" project=\"test:target\"><title>Test</title><description>A test package</description></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
                source_meta,
            )
            .with_get(
                "https://api.opensuse.org/source/test:target/test-pkgA/_meta",
                target_meta,
            )
            .with_get_error(
                "https://api.opensuse.org/source/test:target/test-pkgA/_link",
                ErrorKind::NotFound,
            )
            .with_put(
                "https://api.opensuse.org/source/test:target/test-pkgA/_link",
                "<revision rev=\"1\" vrev=\"1\" srcmd5=\"abc\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .link_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &LinkOptions::default(),
                &DryRun::new(false),
            )
            .is_ok());
        // Only the _link PUT: the meta needed no change.
        assert_eq!(http.put_calls().len(), 1);
        assert_eq!(
            http.put_calls()[0].0,
            "https://api.opensuse.org/source/test:target/test-pkgA/_link"
        );
    }

    #[test]
    fn link_package_no_change_with_prolog_skips_meta_put() {
        // The fetched target meta may carry an XML prolog, which the
        // rendered document never does: a string comparison would
        // read a change where there is none. The no-change check
        // compares parsed documents, so only the _link is PUT.
        let source_meta = "<package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description></package>";
        let target_meta = "<?xml version=\"1.0\"?><package name=\"test-pkgA\" project=\"test:target\"><title>Test</title><description>A test package</description></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
                source_meta,
            )
            .with_get(
                "https://api.opensuse.org/source/test:target/test-pkgA/_meta",
                target_meta,
            )
            .with_get_error(
                "https://api.opensuse.org/source/test:target/test-pkgA/_link",
                ErrorKind::NotFound,
            )
            .with_put(
                "https://api.opensuse.org/source/test:target/test-pkgA/_link",
                "<revision rev=\"1\" vrev=\"1\" srcmd5=\"abc\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .link_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &LinkOptions::default(),
                &DryRun::new(false),
            )
            .is_ok());
        // Only the _link PUT: the meta needed no change.
        assert_eq!(http.put_calls().len(), 1);
        assert_eq!(
            http.put_calls()[0].0,
            "https://api.opensuse.org/source/test:target/test-pkgA/_link"
        );
    }

    #[test]
    fn link_package_missing_source_is_not_found() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .link_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &LinkOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn link_package_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
            "<package name=\"test-pkgA\" project=\"test:factory\"/>",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .link_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &LinkOptions::default(),
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn link_package_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .link_package(
                "test/factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &LinkOptions::default(),
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn link_package_dry_run_records_plan_and_links_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        assert!(client
            .link_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &LinkOptions::default(),
                &dry_run,
            )
            .is_ok());
        assert_eq!(
            dry_run.plan(),
            vec![
                "[dry-run] would: link package test:factory/test-pkgA to test:target/test-pkgA"
                    .to_string()
            ]
        );
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn link_package_dry_run_names_revision_vrev_and_cicount() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        let options = LinkOptions {
            revision: Some("abc123".to_string()),
            vrev: Some("2".to_string()),
            cicount: Some("3".to_string()),
            ..LinkOptions::default()
        };
        assert!(client
            .link_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &options,
                &dry_run,
            )
            .is_ok());
        assert_eq!(
            dry_run.plan(),
            vec![
                "[dry-run] would: link package test:factory/test-pkgA to test:target/test-pkgA (revision abc123, vrev 2, cicount 3)"
                    .to_string()
            ]
        );
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn link_package_with_revision_and_vrev_and_cicount() {
        let source_meta = "<package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
                source_meta,
            )
            .with_get_error(
                "https://api.opensuse.org/source/test:target/test-pkgA/_meta",
                ErrorKind::NotFound,
            )
            .with_put(
                "https://api.opensuse.org/source/test:target/test-pkgA/_meta",
                "<status code=\"ok\" />",
            )
            .with_put(
                "https://api.opensuse.org/source/test:target/test-pkgA/_link",
                "<revision rev=\"1\" vrev=\"1\" srcmd5=\"abc\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = LinkOptions {
            revision: Some("abc123".to_string()),
            vrev: Some("2".to_string()),
            cicount: Some("3".to_string()),
            missing_target: true,
            ..LinkOptions::default()
        };
        assert!(client
            .link_package(
                "test:factory",
                "test-pkgA",
                "test:target",
                "test-pkgA",
                &options,
                &DryRun::new(false),
            )
            .is_ok());
        assert_eq!(
            http.put_calls()[1].1,
            "<link project=\"test:factory\" package=\"test-pkgA\" rev=\"abc123\" vrev=\"2\" cicount=\"3\" missingok=\"1\"/>"
        );
    }

    #[test]
    fn repair_link_repairs_broken_link_at_base() {
        let broken = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><entry name=\"bad.patch\" md5=\"cccccccccccccccccccccccccccccccc\" size=\"40\" mtime=\"1700000001\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"dddddddddddddddddddddddddddddddd\" baserev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" xsrcmd5=\"ffffffffffffffffffffffffffffffff\" error=\"patch bad.patch failed to apply\"/></directory>";
        let base_ok = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" baserev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" xsrcmd5=\"11111111111111111111111111111111\"/></directory>";
        let link = "<link project=\"test:factory\" package=\"test-pkgA\" baserev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\"><patches><apply name=\"bad.patch\"/></patches></link>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?rev=latest",
                broken,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?rev=latest&linkrev=base",
                base_ok,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_link",
                link,
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_link",
                "<revision rev=\"3\" vrev=\"1\" srcmd5=\"22222222222222222222222222222222\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .repair_link("test:factory", "test-pkgA", &DryRun::new(false))
            .is_ok());
        // The patches are stripped; the existing baserev stands.
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].1,
            "<link project=\"test:factory\" package=\"test-pkgA\" baserev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\"></link>"
        );
    }

    #[test]
    fn repair_link_repairs_via_lastworking_without_baserev() {
        let broken = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"dddddddddddddddddddddddddddddddd\" xsrcmd5=\"ffffffffffffffffffffffffffffffff\" error=\"patch bad.patch failed to apply\"/></directory>";
        let lastworking = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"dddddddddddddddddddddddddddddddd\" xsrcmd5=\"ffffffffffffffffffffffffffffffff\" error=\"patch bad.patch failed to apply\" lastworking=\"33333333333333333333333333333333\"/></directory>";
        let working = "<directory name=\"test:factory/test-pkgA\" rev=\"1\" vrev=\"1\" srcmd5=\"33333333333333333333333333333333\"><entry name=\"_link\" md5=\"44444444444444444444444444444444\" size=\"100\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"55555555555555555555555555555555\" xsrcmd5=\"33333333333333333333333333333333\"/></directory>";
        let link = "<link project=\"test:factory\" package=\"test-pkgA\"><patches><apply name=\"bad.patch\"/></patches></link>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?rev=latest",
                broken,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?lastworking=1",
                lastworking,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?rev=33333333333333333333333333333333",
                working,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_link",
                link,
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_link",
                "<revision rev=\"3\" vrev=\"1\" srcmd5=\"22222222222222222222222222222222\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .repair_link("test:factory", "test-pkgA", &DryRun::new(false))
            .is_ok());
        // The repaired link is pinned at the target revision the
        // last working expansion resolved to.
        assert_eq!(
            http.put_calls()[0].1,
            "<link project=\"test:factory\" package=\"test-pkgA\" baserev=\"55555555555555555555555555555555\"></link>"
        );
    }

    #[test]
    fn repair_link_repairs_via_previous_revision_without_lastworking() {
        let broken = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"dddddddddddddddddddddddddddddddd\" xsrcmd5=\"ffffffffffffffffffffffffffffffff\" error=\"patch bad.patch failed to apply\"/></directory>";
        let prior = "<directory name=\"test:factory/test-pkgA\" rev=\"1\" vrev=\"1\" srcmd5=\"33333333333333333333333333333333\"><entry name=\"_link\" md5=\"44444444444444444444444444444444\" size=\"100\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"55555555555555555555555555555555\" xsrcmd5=\"33333333333333333333333333333333\"/></directory>";
        let link = "<link project=\"test:factory\" package=\"test-pkgA\"><patches><apply name=\"bad.patch\"/></patches></link>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?rev=latest",
                broken,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?lastworking=1",
                broken,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?rev=1",
                prior,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_link",
                link,
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_link",
                "<revision rev=\"3\" vrev=\"1\" srcmd5=\"22222222222222222222222222222222\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .repair_link("test:factory", "test-pkgA", &DryRun::new(false))
            .is_ok());
        // No last-working revision is recorded (the breakage came
        // from the link itself), so the pin comes from the link's
        // previous revision, which still expands.
        assert_eq!(
            http.put_calls()[0].1,
            "<link project=\"test:factory\" package=\"test-pkgA\" baserev=\"55555555555555555555555555555555\"></link>"
        );
    }

    #[test]
    fn repair_link_broken_base_falls_back_to_lastworking() {
        let broken = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"dddddddddddddddddddddddddddddddd\" baserev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" xsrcmd5=\"ffffffffffffffffffffffffffffffff\" error=\"patch bad.patch failed to apply\"/></directory>";
        let base_broken = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"dddddddddddddddddddddddddddddddd\" baserev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" error=\"patch bad.patch failed to apply\"/></directory>";
        let lastworking = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"dddddddddddddddddddddddddddddddd\" error=\"patch bad.patch failed to apply\" lastworking=\"33333333333333333333333333333333\"/></directory>";
        let working = "<directory name=\"test:factory/test-pkgA\" rev=\"1\" vrev=\"1\" srcmd5=\"33333333333333333333333333333333\"><entry name=\"_link\" md5=\"44444444444444444444444444444444\" size=\"100\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"55555555555555555555555555555555\" xsrcmd5=\"33333333333333333333333333333333\"/></directory>";
        let link = "<link project=\"test:factory\" package=\"test-pkgA\" baserev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\"><patches><apply name=\"bad.patch\"/></patches></link>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?rev=latest",
                broken,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?rev=latest&linkrev=base",
                base_broken,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?lastworking=1",
                lastworking,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?rev=33333333333333333333333333333333",
                working,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_link",
                link,
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_link",
                "<revision rev=\"3\" vrev=\"1\" srcmd5=\"22222222222222222222222222222222\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .repair_link("test:factory", "test-pkgA", &DryRun::new(false))
            .is_ok());
        // The base was broken too, so the pin comes from the last
        // working expansion, replacing the old baserev.
        assert_eq!(
            http.put_calls()[0].1,
            "<link project=\"test:factory\" package=\"test-pkgA\" baserev=\"55555555555555555555555555555555\"></link>"
        );
    }

    #[test]
    fn repair_link_not_a_link_is_refused() {
        let directory = "<directory name=\"test:factory/test-pkgA\" rev=\"1\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"test-pkgA.spec\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"100\" mtime=\"1700000000\"/></directory>";
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/test:factory/test-pkgA?rev=latest",
            directory,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .repair_link("test:factory", "test-pkgA", &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn repair_link_unbroken_link_is_refused() {
        let directory = "<directory name=\"test:factory/test-pkgA\" rev=\"1\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"100\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"dddddddddddddddddddddddddddddddd\" xsrcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"/></directory>";
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/test:factory/test-pkgA?rev=latest",
            directory,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .repair_link("test:factory", "test-pkgA", &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn repair_link_without_patches_is_not_a_patch_failure() {
        let broken = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"dddddddddddddddddddddddddddddddd\" baserev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" error=\"linked package does not exist\"/></directory>";
        let base_ok = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" baserev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" xsrcmd5=\"11111111111111111111111111111111\"/></directory>";
        let link = "<link project=\"test:factory\" package=\"test-pkgA\" baserev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\"/>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?rev=latest",
                broken,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?rev=latest&linkrev=base",
                base_ok,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_link",
                link,
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .repair_link("test:factory", "test-pkgA", &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn repair_link_never_worked_is_refused() {
        let broken = "<directory name=\"test:factory/test-pkgA\" rev=\"1\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"dddddddddddddddddddddddddddddddd\" error=\"patch bad.patch failed to apply\"/></directory>";
        let lastworking = "<directory name=\"test:factory/test-pkgA\" rev=\"1\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"dddddddddddddddddddddddddddddddd\" error=\"patch bad.patch failed to apply\"/></directory>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?rev=latest",
                broken,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?lastworking=1",
                lastworking,
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .repair_link("test:factory", "test-pkgA", &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn repair_link_working_revision_broken_is_error() {
        let broken = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"dddddddddddddddddddddddddddddddd\" error=\"patch bad.patch failed to apply\"/></directory>";
        let lastworking = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"dddddddddddddddddddddddddddddddd\" error=\"patch bad.patch failed to apply\" lastworking=\"33333333333333333333333333333333\"/></directory>";
        let working_broken = "<directory name=\"test:factory/test-pkgA\" rev=\"1\" vrev=\"1\" srcmd5=\"33333333333333333333333333333333\"><entry name=\"_link\" md5=\"44444444444444444444444444444444\" size=\"100\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"55555555555555555555555555555555\" error=\"patch bad.patch failed to apply\"/></directory>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?rev=latest",
                broken,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?lastworking=1",
                lastworking,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?rev=33333333333333333333333333333333",
                working_broken,
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .repair_link("test:factory", "test-pkgA", &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn repair_link_topadd_before_apply_survives() {
        let broken = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"dddddddddddddddddddddddddddddddd\" baserev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" error=\"patch bad.patch failed to apply\"/></directory>";
        let base_ok = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" baserev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" xsrcmd5=\"11111111111111111111111111111111\"/></directory>";
        let link = "<link project=\"test:factory\" package=\"test-pkgA\" baserev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\"><patches><topadd name=\"top.patch\"/><apply name=\"bad.patch\"/><delete name=\"gone\"/></patches></link>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?rev=latest",
                broken,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?rev=latest&linkrev=base",
                base_ok,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_link",
                link,
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_link",
                "<revision rev=\"3\" vrev=\"1\" srcmd5=\"22222222222222222222222222222222\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .repair_link("test:factory", "test-pkgA", &DryRun::new(false))
            .is_ok());
        assert_eq!(
            http.put_calls()[0].1,
            "<link project=\"test:factory\" package=\"test-pkgA\" baserev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\"><patches><topadd name=\"top.patch\"/></patches></link>"
        );
    }

    #[test]
    fn repair_link_dry_run_records_plan_and_repairs_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        assert!(client
            .repair_link("test:factory", "test-pkgA", &dry_run)
            .is_ok());
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: repair link test:factory/test-pkgA".to_string()]
        );
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn repair_link_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/test:factory/test-pkgA?rev=latest",
            "<directory name=\"test:factory/test-pkgA\"/>",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .repair_link("test:factory", "test-pkgA", &DryRun::new(false))
            .is_err());
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn repair_link_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .repair_link("test/factory", "test-pkgA", &DryRun::new(false))
            .is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn repair_link_missing_package_is_not_found() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/source/test:factory/test-pkgA?rev=latest",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .repair_link("test:factory", "test-pkgA", &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn repair_link_non_revision_body_is_error_not_success() {
        let broken = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"dddddddddddddddddddddddddddddddd\" baserev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" error=\"patch bad.patch failed to apply\"/></directory>";
        let base_ok = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" baserev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" xsrcmd5=\"11111111111111111111111111111111\"/></directory>";
        let link = "<link project=\"test:factory\" package=\"test-pkgA\" baserev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\"><patches><apply name=\"bad.patch\"/></patches></link>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?rev=latest",
                broken,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA?rev=latest&linkrev=base",
                base_ok,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_link",
                link,
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_link",
                "<status code=\"ok\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .repair_link("test:factory", "test-pkgA", &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn repair_link_doc_strips_patches_and_sets_baserev() {
        let mut doc = XmlDoc::parse("<link project=\"test:factory\" package=\"test-pkgA\"><patches><apply name=\"bad.patch\"/><delete name=\"gone\"/></patches></link>").unwrap();
        assert!(repair_link_doc(&mut doc, Some("55555555555555555555555555555555")).unwrap());
        assert_eq!(
            doc.render(),
            "<link project=\"test:factory\" package=\"test-pkgA\" baserev=\"55555555555555555555555555555555\"></link>"
        );
    }

    #[test]
    fn repair_link_doc_false_without_apply_or_delete() {
        let mut doc = XmlDoc::parse("<link project=\"test:factory\" package=\"test-pkgA\"><patches><topadd name=\"top.patch\"/></patches></link>").unwrap();
        assert!(!repair_link_doc(&mut doc, None).unwrap());
        assert_eq!(
            doc.render(),
            "<link project=\"test:factory\" package=\"test-pkgA\"><patches><topadd name=\"top.patch\"/></patches></link>"
        );
        let mut doc =
            XmlDoc::parse("<link project=\"test:factory\" package=\"test-pkgA\"/>").unwrap();
        assert!(!repair_link_doc(&mut doc, None).unwrap());
    }

    #[test]
    fn set_link_rev_latest_resolves_target_revision() {
        let link = "<link package=\"srcpkg\" project=\"srcprj\" />";
        let directory = "<directory name=\"srcpkg\" rev=\"42\" srcmd5=\"ffffffffffffffffffffffffffffffff\" vrev=\"1\"><entry md5=\"0d62ceea6020d75154078a20d8c9f9ba\" mtime=\"1282047302\" name=\"foo\" size=\"23\" /></directory>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/osctest/simple/_link",
                link,
            )
            .with_get(
                "https://api.opensuse.org/source/srcprj/srcpkg?rev=latest",
                directory,
            )
            .with_put(
                "https://api.opensuse.org/source/osctest/simple/_link?comment=Set%20link%20revision%20to%2042",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .set_link_rev(
                    "osctest",
                    "simple",
                    &LinkRevision::Latest,
                    &DryRun::new(false),
                )
                .unwrap(),
            Some("42".to_string())
        );
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].1,
            "<link package=\"srcpkg\" project=\"srcprj\" rev=\"42\"/>"
        );
    }

    #[test]
    fn set_link_rev_latest_keeps_vrev_for_an_md5_revision() {
        let link = "<link package=\"srcpkg\" project=\"srcprj\" />";
        let md5 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let directory = format!(
            "<directory name=\"srcpkg\" rev=\"{md5}\" srcmd5=\"{md5}\" vrev=\"3\"><entry md5=\"0d62ceea6020d75154078a20d8c9f9ba\" mtime=\"1282047302\" name=\"foo\" size=\"23\" /></directory>"
        );
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/osctest/simple/_link",
                link,
            )
            .with_get(
                "https://api.opensuse.org/source/srcprj/srcpkg?rev=latest",
                &directory,
            )
            .with_put(
                &format!(
                    "https://api.opensuse.org/source/osctest/simple/_link?comment=Set%20link%20revision%20to%20{md5}"
                ),
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .set_link_rev(
                    "osctest",
                    "simple",
                    &LinkRevision::Latest,
                    &DryRun::new(false),
                )
                .unwrap(),
            Some(md5.to_string())
        );
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].1,
            format!("<link package=\"srcpkg\" project=\"srcprj\" rev=\"{md5}\" vrev=\"3\"/>")
        );
    }

    #[test]
    fn set_link_rev_latest_drops_vrev_for_a_longer_hex_revision() {
        // A 40-hex revision is not an md5: the directory's vrev must
        // not survive onto the link. The old `rev.len() >= 32` kept
        // it.
        let link = "<link package=\"srcpkg\" project=\"srcprj\" />";
        let rev = "a".repeat(40);
        let directory = format!(
            "<directory name=\"srcpkg\" rev=\"{rev}\" srcmd5=\"ffffffffffffffffffffffffffffffff\" vrev=\"3\"><entry md5=\"0d62ceea6020d75154078a20d8c9f9ba\" mtime=\"1282047302\" name=\"foo\" size=\"23\" /></directory>"
        );
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/osctest/simple/_link",
                link,
            )
            .with_get(
                "https://api.opensuse.org/source/srcprj/srcpkg?rev=latest",
                &directory,
            )
            .with_put(
                &format!(
                    "https://api.opensuse.org/source/osctest/simple/_link?comment=Set%20link%20revision%20to%20{rev}"
                ),
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .set_link_rev(
                    "osctest",
                    "simple",
                    &LinkRevision::Latest,
                    &DryRun::new(false),
                )
                .unwrap(),
            Some(rev.clone())
        );
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].1,
            format!("<link package=\"srcpkg\" project=\"srcprj\" rev=\"{rev}\"/>")
        );
    }

    #[test]
    fn set_link_rev_missing_revision_error_carries_response_preview() {
        // A directory with no revision is a server error, and the
        // error names what was received, so the caller can diagnose
        // it.
        let link = "<link package=\"srcpkg\" project=\"srcprj\" />";
        let directory = "<directory name=\"srcpkg\" srcmd5=\"ffffffffffffffffffffffffffffffff\"><entry md5=\"0d62ceea6020d75154078a20d8c9f9ba\" mtime=\"1282047302\" name=\"foo\" size=\"23\" /></directory>";
        let http = FakeHttp::new()
            .with_get("https://api.opensuse.org/source/osctest/simple/_link", link)
            .with_get(
                "https://api.opensuse.org/source/srcprj/srcpkg?rev=latest",
                directory,
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .set_link_rev(
                "osctest",
                "simple",
                &LinkRevision::Latest,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(
            err.context.contains("received: <directory"),
            "{}",
            err.context
        );
    }

    #[test]
    fn set_link_rev_pinned_puts_without_fetching() {
        let link = "<link package=\"srcpkg\" project=\"srcprj\" />";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/osctest/simple/_link",
                link,
            )
            .with_put(
                "https://api.opensuse.org/source/osctest/simple/_link?comment=Set%20link%20revision%20to%2042",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .set_link_rev(
                    "osctest",
                    "simple",
                    &LinkRevision::Pinned("42".to_string()),
                    &DryRun::new(false),
                )
                .unwrap(),
            Some("42".to_string())
        );
        // Only the _link GET: no directory fetch for a pinned rev.
        assert_eq!(http.get_calls().len(), 1);
        assert_eq!(http.put_calls().len(), 1);
    }

    #[test]
    fn set_link_rev_expanded_resolves_expanded_revision() {
        let link = "<link package=\"srcpkg\" />";
        let directory = "<directory name=\"srcpkg\" rev=\"1\" srcmd5=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" vrev=\"1\"><linkinfo project=\"srcsrcprj\" package=\"srcsrcpkg\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" baserev=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" xsrcmd5=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" lsrcmd5=\"cccccccccccccccccccccccccccccccc\" /><entry md5=\"0d62ceea6020d75154078a20d8c9f9ba\" mtime=\"1282047302\" name=\"_link\" size=\"23\" /></directory>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/osctest/simple/_link",
                link,
            )
            .with_get(
                "https://api.opensuse.org/source/osctest/srcpkg?rev=latest&expand=1",
                directory,
            )
            .with_put(
                "https://api.opensuse.org/source/osctest/simple/_link?comment=Set%20link%20revision%20to%20eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .set_link_rev(
                    "osctest",
                    "simple",
                    &LinkRevision::Expanded,
                    &DryRun::new(false),
                )
                .unwrap(),
            Some("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string())
        );
        assert_eq!(
            http.put_calls()[0].1,
            "<link package=\"srcpkg\" rev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" vrev=\"1\"/>"
        );
    }

    #[test]
    fn set_link_rev_latest_updates_existing_revision() {
        let link = "<link package=\"srcpkg\" project=\"srcprj\" rev=\"7\" />";
        let directory = "<directory name=\"srcpkg\" rev=\"42\" srcmd5=\"ffffffffffffffffffffffffffffffff\" vrev=\"1\"><entry md5=\"0d62ceea6020d75154078a20d8c9f9ba\" mtime=\"1282047302\" name=\"foo\" size=\"23\" /></directory>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/osctest/simple/_link",
                link,
            )
            .with_get(
                "https://api.opensuse.org/source/srcprj/srcpkg?rev=latest",
                directory,
            )
            .with_put(
                "https://api.opensuse.org/source/osctest/simple/_link?comment=Set%20link%20revision%20to%2042",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .set_link_rev(
                "osctest",
                "simple",
                &LinkRevision::Latest,
                &DryRun::new(false),
            )
            .is_ok());
        assert_eq!(
            http.put_calls()[0].1,
            "<link package=\"srcpkg\" project=\"srcprj\" rev=\"42\"/>"
        );
    }

    #[test]
    fn set_link_rev_expanded_updates_existing_revision() {
        let link = "<link package=\"srcpkg\" project=\"srcprj\" rev=\"7\" />";
        let directory = "<directory name=\"srcpkg\" rev=\"1\" srcmd5=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" vrev=\"1\"><linkinfo project=\"srcsrcprj\" package=\"srcsrcpkg\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" baserev=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" xsrcmd5=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" lsrcmd5=\"cccccccccccccccccccccccccccccccc\" /><entry md5=\"0d62ceea6020d75154078a20d8c9f9ba\" mtime=\"1282047302\" name=\"_link\" size=\"23\" /></directory>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/osctest/simple/_link",
                link,
            )
            .with_get(
                "https://api.opensuse.org/source/srcprj/srcpkg?rev=latest&expand=1",
                directory,
            )
            .with_put(
                "https://api.opensuse.org/source/osctest/simple/_link?comment=Set%20link%20revision%20to%20eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .set_link_rev(
                "osctest",
                "simple",
                &LinkRevision::Expanded,
                &DryRun::new(false),
            )
            .is_ok());
        assert_eq!(
            http.put_calls()[0].1,
            "<link package=\"srcpkg\" project=\"srcprj\" rev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" vrev=\"1\"/>"
        );
    }

    #[test]
    fn set_link_rev_expanding_broken_link_propagates_the_error() {
        let link = "<link package=\"srcpkg\" project=\"srcprj\" />";
        let http = FakeHttp::new()
            .with_get("https://api.opensuse.org/source/osctest/simple/_link", link)
            .with_get_error(
                "https://api.opensuse.org/source/srcprj/srcpkg?rev=latest&expand=1",
                ErrorKind::ServerError,
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .set_link_rev(
                "osctest",
                "simple",
                &LinkRevision::Expanded,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_link_rev_unset_strips_the_revision() {
        let link = "<link package=\"srcpkg\" project=\"srcprj\" rev=\"42\" />";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/osctest/simple/_link",
                link,
            )
            .with_put(
                "https://api.opensuse.org/source/osctest/simple/_link?comment=Unset%20link%20revision",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .set_link_rev(
                    "osctest",
                    "simple",
                    &LinkRevision::Unset,
                    &DryRun::new(false),
                )
                .unwrap(),
            None
        );
        assert_eq!(
            http.put_calls()[0].1,
            "<link package=\"srcpkg\" project=\"srcprj\"/>"
        );
    }

    #[test]
    fn set_link_rev_unset_strips_revision_and_vrev() {
        let link = "<link package=\"srcpkg\" project=\"srcprj\" rev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" vrev=\"1\" />";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/osctest/simple/_link",
                link,
            )
            .with_put(
                "https://api.opensuse.org/source/osctest/simple/_link?comment=Unset%20link%20revision",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .set_link_rev(
                "osctest",
                "simple",
                &LinkRevision::Unset,
                &DryRun::new(false),
            )
            .is_ok());
        assert_eq!(
            http.put_calls()[0].1,
            "<link package=\"srcpkg\" project=\"srcprj\"/>"
        );
    }

    #[test]
    fn set_link_rev_unset_puts_even_without_a_revision() {
        let link = "<link package=\"srcpkg\" project=\"srcprj\" />";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/osctest/simple/_link",
                link,
            )
            .with_put(
                "https://api.opensuse.org/source/osctest/simple/_link?comment=Unset%20link%20revision",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .set_link_rev(
                "osctest",
                "simple",
                &LinkRevision::Unset,
                &DryRun::new(false),
            )
            .is_ok());
        // Unsetting a missing revision still PUTs the unchanged link.
        assert_eq!(http.put_calls().len(), 1);
        assert_eq!(
            http.put_calls()[0].1,
            "<link package=\"srcpkg\" project=\"srcprj\"/>"
        );
    }

    #[test]
    fn set_link_rev_pinned_expanded_resolves_the_pinned_revision() {
        let link = "<link package=\"srcpkg\" project=\"srcprj\" />";
        let directory = "<directory name=\"srcpkg\" rev=\"2\" srcmd5=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" vrev=\"1\"><entry md5=\"0d62ceea6020d75154078a20d8c9f9ba\" mtime=\"1282047302\" name=\"foo\" size=\"23\" /></directory>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/osctest/simple/_link",
                link,
            )
            .with_get(
                "https://api.opensuse.org/source/srcprj/srcpkg?rev=2&expand=1",
                directory,
            )
            .with_put(
                "https://api.opensuse.org/source/osctest/simple/_link?comment=Set%20link%20revision%20to%20eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .set_link_rev(
                    "osctest",
                    "simple",
                    &LinkRevision::PinnedExpanded("2".to_string()),
                    &DryRun::new(false),
                )
                .unwrap(),
            Some("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string())
        );
        assert_eq!(
            http.put_calls()[0].1,
            "<link package=\"srcpkg\" project=\"srcprj\" rev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" vrev=\"1\"/>"
        );
    }

    #[test]
    fn set_link_rev_latest_short_revision_drops_stale_vrev() {
        let link = "<link package=\"srcpkg\" project=\"srcprj\" rev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" vrev=\"1\" />";
        let directory = "<directory name=\"srcpkg\" rev=\"42\" srcmd5=\"ffffffffffffffffffffffffffffffff\" vrev=\"1\"><entry md5=\"0d62ceea6020d75154078a20d8c9f9ba\" mtime=\"1282047302\" name=\"foo\" size=\"23\" /></directory>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/osctest/simple/_link",
                link,
            )
            .with_get(
                "https://api.opensuse.org/source/srcprj/srcpkg?rev=latest",
                directory,
            )
            .with_put(
                "https://api.opensuse.org/source/osctest/simple/_link?comment=Set%20link%20revision%20to%2042",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .set_link_rev(
                "osctest",
                "simple",
                &LinkRevision::Latest,
                &DryRun::new(false),
            )
            .is_ok());
        // The numeric revision has no versions within it: the vrev
        // from the old md5 revision is stale and is dropped.
        assert_eq!(
            http.put_calls()[0].1,
            "<link package=\"srcpkg\" project=\"srcprj\" rev=\"42\"/>"
        );
    }

    #[test]
    fn set_link_rev_pinned_md5_drops_stale_vrev() {
        let link = "<link package=\"srcpkg\" project=\"srcprj\" rev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" vrev=\"1\" />";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/osctest/simple/_link",
                link,
            )
            .with_put(
                "https://api.opensuse.org/source/osctest/simple/_link?comment=Set%20link%20revision%20to%20aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .set_link_rev(
                "osctest",
                "simple",
                &LinkRevision::Pinned("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
                &DryRun::new(false),
            )
            .is_ok());
        // Pinning resolves nothing, so there is no vrev to set — and
        // the vrev from the old revision is stale.
        assert_eq!(
            http.put_calls()[0].1,
            "<link package=\"srcpkg\" project=\"srcprj\" rev=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"/>"
        );
    }

    #[test]
    fn set_link_rev_latest_without_directory_revision_is_error() {
        let link = "<link package=\"srcpkg\" project=\"srcprj\" />";
        let directory = "<directory name=\"srcpkg\" srcmd5=\"ffffffffffffffffffffffffffffffff\"><entry md5=\"0d62ceea6020d75154078a20d8c9f9ba\" mtime=\"1282047302\" name=\"foo\" size=\"23\" /></directory>";
        let http = FakeHttp::new()
            .with_get("https://api.opensuse.org/source/osctest/simple/_link", link)
            .with_get(
                "https://api.opensuse.org/source/srcprj/srcpkg?rev=latest",
                directory,
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .set_link_rev(
                "osctest",
                "simple",
                &LinkRevision::Latest,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_link_rev_missing_link_is_not_found() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/source/osctest/simple/_link",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .set_link_rev(
                "osctest",
                "simple",
                &LinkRevision::Latest,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_link_rev_dry_run_records_plan_and_sets_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        assert_eq!(
            client
                .set_link_rev("osctest", "simple", &LinkRevision::Latest, &dry_run,)
                .unwrap(),
            None
        );
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: set link revision of osctest/simple to latest".to_string()]
        );
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_link_rev_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/osctest/simple/_link",
            "<link package=\"srcpkg\" project=\"srcprj\" />",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .set_link_rev(
                "osctest",
                "simple",
                &LinkRevision::Latest,
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_link_rev_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .set_link_rev(
                "osctest",
                "simple/pkg",
                &LinkRevision::Latest,
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn set_link_rev_project_sets_every_linked_package() {
        let project = "<directory name=\"test:factory\"><entry name=\"pkgA\"/><entry name=\"pkgB\"/><entry name=\"pkgC\"/></directory>";
        let link = "<link package=\"srcpkg\" project=\"srcprj\" />";
        let directory = "<directory name=\"srcpkg\" rev=\"42\" srcmd5=\"ffffffffffffffffffffffffffffffff\" vrev=\"1\"><entry md5=\"0d62ceea6020d75154078a20d8c9f9ba\" mtime=\"1282047302\" name=\"foo\" size=\"23\" /></directory>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory",
                project,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/pkgA/_link",
                link,
            )
            .with_get_error(
                "https://api.opensuse.org/source/test:factory/pkgB/_link",
                ErrorKind::NotFound,
            )
            .with_get(
                "https://api.opensuse.org/source/test:factory/pkgC/_link",
                link,
            )
            .with_get(
                "https://api.opensuse.org/source/srcprj/srcpkg?rev=latest",
                directory,
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/pkgA/_link?comment=Set%20link%20revision%20to%2042",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/pkgC/_link?comment=Set%20link%20revision%20to%2042",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .set_link_rev_project("test:factory", &LinkRevision::Latest, &DryRun::new(false),)
                .unwrap(),
            vec![
                ("pkgA".to_string(), Some("42".to_string())),
                ("pkgC".to_string(), Some("42".to_string())),
            ]
        );
        // pkgB has no link and is skipped; pkgA and pkgC are set.
        assert_eq!(http.put_calls().len(), 2);
    }

    #[test]
    fn set_link_rev_project_pinned_is_refused() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .set_link_rev_project(
                "test:factory",
                &LinkRevision::Pinned("42".to_string()),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn set_link_rev_project_dry_run_records_plan_and_sets_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        assert_eq!(
            client
                .set_link_rev_project("test:factory", &LinkRevision::Latest, &dry_run)
                .unwrap(),
            Vec::<(String, Option<String>)>::new()
        );
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: set link revision of every linked package in project test:factory to latest".to_string()]
        );
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn link_to_branch_posts_the_command_and_succeeds() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/osctest/simple?cmd=linktobranch",
            "<status code=\"ok\"/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .link_to_branch("osctest", "simple", &DryRun::new(false))
            .is_ok());
        let posts = http.post_calls();
        assert_eq!(posts.len(), 1);
        // The linktobranch POST carries no body (the command is the
        // query) and the credentials.
        assert_eq!(posts[0].1, "");
        assert!(posts[0].2);
    }

    #[test]
    fn link_to_branch_non_ok_body_is_error_not_success() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/osctest/simple?cmd=linktobranch",
            "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .link_to_branch("osctest", "simple", &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn link_to_branch_dry_run_records_plan_and_converts_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        assert!(client.link_to_branch("osctest", "simple", &dry_run).is_ok());
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: convert link osctest/simple to a branch".to_string()]
        );
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn link_to_branch_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/osctest/simple?cmd=linktobranch",
            "<status code=\"ok\"/>",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .link_to_branch("osctest", "simple", &DryRun::new(false))
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn link_to_branch_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .link_to_branch("osctest", "simple/pkg", &DryRun::new(false))
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn detach_branch_self_copy_with_expand_succeeds() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/osctest/simple?cmd=copy&oproject=osctest&opackage=simple&expand=1",
            "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .detach_branch("osctest", "simple", &DryRun::new(false))
                .unwrap(),
            Some(2)
        );
        let posts = http.post_calls();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].1, "");
        assert!(posts[0].2);
        // No triage reads: the copy succeeded.
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn detach_branch_copy_failure_on_plain_package_is_refused() {
        let directory =
            "<directory name=\"osctest/simple\" rev=\"2\"><entry name=\"foo.spec\"/></directory>";
        let http = FakeHttp::new()
            .with_post(
                "https://api.opensuse.org/source/osctest/simple?cmd=copy&oproject=osctest&opackage=simple&expand=1",
                "<status code=\"ok\"/>",
            )
            .with_get(
                "https://api.opensuse.org/source/osctest/simple?rev=latest",
                directory,
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .detach_branch("osctest", "simple", &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn detach_branch_copy_failure_on_unbroken_link_propagates() {
        let directory = "<directory name=\"osctest/simple\" rev=\"2\"><entry name=\"_link\"/><linkinfo project=\"srcprj\" package=\"srcpkg\" srcmd5=\"dddddddddddddddddddddddddddddddd\"/></directory>";
        let http = FakeHttp::new()
            .with_post(
                "https://api.opensuse.org/source/osctest/simple?cmd=copy&oproject=osctest&opackage=simple&expand=1",
                "<status code=\"ok\"/>",
            )
            .with_get(
                "https://api.opensuse.org/source/osctest/simple?rev=latest",
                directory,
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .detach_branch("osctest", "simple", &DryRun::new(false))
            .unwrap_err();
        // The link expands, so the copy failed for another reason:
        // the original copy error propagates.
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn detach_branch_copy_failure_broken_link_target_exists_propagates() {
        let directory = "<directory name=\"osctest/simple\" rev=\"2\"><entry name=\"_link\"/><linkinfo project=\"srcprj\" package=\"srcpkg\" srcmd5=\"dddddddddddddddddddddddddddddddd\" error=\"404 package 'srcprj/srcpkg' does not exist\"/></directory>";
        let target_meta = "<package name=\"srcpkg\" project=\"srcprj\"><title>Source</title><description>Source package</description></package>";
        let http = FakeHttp::new()
            .with_post(
                "https://api.opensuse.org/source/osctest/simple?cmd=copy&oproject=osctest&opackage=simple&expand=1",
                "<status code=\"ok\"/>",
            )
            .with_get(
                "https://api.opensuse.org/source/osctest/simple?rev=latest",
                directory,
            )
            .with_get(
                "https://api.opensuse.org/source/srcprj/srcpkg/_meta",
                target_meta,
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .detach_branch("osctest", "simple", &DryRun::new(false))
            .unwrap_err();
        // The target still exists, so the broken link is not the
        // target-gone case: the original copy error propagates.
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn detach_branch_copy_failure_broken_link_target_gone_drops_the_link() {
        let directory = "<directory name=\"osctest/simple\" rev=\"2\"><entry name=\"_link\"/><linkinfo project=\"srcprj\" package=\"srcpkg\" srcmd5=\"dddddddddddddddddddddddddddddddd\" error=\"404 package 'srcprj/srcpkg' does not exist\"/></directory>";
        let http = FakeHttp::new()
            .with_post(
                "https://api.opensuse.org/source/osctest/simple?cmd=copy&oproject=osctest&opackage=simple&expand=1",
                "<status code=\"ok\"/>",
            )
            .with_get(
                "https://api.opensuse.org/source/osctest/simple?rev=latest",
                directory,
            )
            .with_get_error(
                "https://api.opensuse.org/source/srcprj/srcpkg/_meta",
                ErrorKind::NotFound,
            )
            .with_delete(
                "https://api.opensuse.org/source/osctest/simple/_link?comment=removed%20_link",
                "<status code=\"ok\"/>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        // The file delete reports only status-ok, so the detach
        // reports no revision for the dropped link.
        assert_eq!(
            client
                .detach_branch("osctest", "simple", &DryRun::new(false))
                .unwrap(),
            None
        );
        let deletes = http.delete_calls();
        assert_eq!(deletes.len(), 1);
        assert_eq!(
            deletes[0].0,
            "https://api.opensuse.org/source/osctest/simple/_link?comment=removed%20_link"
        );
        assert!(deletes[0].1);
    }

    #[test]
    fn detach_branch_copy_failure_directory_read_failure_propagates() {
        let http = FakeHttp::new()
            .with_post(
                "https://api.opensuse.org/source/osctest/simple?cmd=copy&oproject=osctest&opackage=simple&expand=1",
                "<status code=\"ok\"/>",
            )
            .with_get_error(
                "https://api.opensuse.org/source/osctest/simple?rev=latest",
                ErrorKind::ServerError,
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .detach_branch("osctest", "simple", &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn detach_branch_dry_run_records_plan_and_detaches_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        assert_eq!(
            client.detach_branch("osctest", "simple", &dry_run).unwrap(),
            None
        );
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: detach branch osctest/simple (replace the link with its expanded sources)".to_string()]
        );
        assert!(http.post_calls().is_empty());
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn detach_branch_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/osctest/simple?cmd=copy&oproject=osctest&opackage=simple&expand=1",
            "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .detach_branch("osctest", "simple", &DryRun::new(false))
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn detach_branch_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .detach_branch("osctest", "simple/pkg", &DryRun::new(false))
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn set_devel_project_sets_devel_in_meta() {
        let meta = "<package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
                meta,
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
                "<status code=\"ok\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .set_devel_project(
                "test:factory",
                "test-pkgA",
                Some("test:devel"),
                None,
                &DryRun::new(false),
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].1,
            "<package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description><devel project=\"test:devel\"/></package>"
        );
        assert!(puts[0].2);
    }

    #[test]
    fn set_devel_project_sets_cross_package_devel() {
        let meta = "<package name=\"test-pkgB\" project=\"test:factory\"><title>Test</title><description>A test package</description></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgB/_meta",
                meta,
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/test-pkgB/_meta",
                "<status code=\"ok\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .set_devel_project(
                "test:factory",
                "test-pkgB",
                Some("test:devel"),
                Some("test-pkgA"),
                &DryRun::new(false),
            )
            .is_ok());
        assert_eq!(http.put_calls().len(), 1);
        assert!(http.put_calls()[0]
            .1
            .contains("<devel project=\"test:devel\" package=\"test-pkgA\"/>"));
    }

    #[test]
    fn set_devel_project_unsets_devel() {
        let meta = "<package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description><devel project=\"test:devel\"/></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
                meta,
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
                "<status code=\"ok\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .set_devel_project("test:factory", "test-pkgA", None, None, &DryRun::new(false))
            .is_ok());
        assert_eq!(http.put_calls().len(), 1);
        assert!(!http.put_calls()[0].1.contains("<devel"));
    }

    #[test]
    fn set_devel_project_no_change_makes_no_put() {
        // The devel is already what was asked: nothing is written.
        let meta = "<package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description><devel project=\"test:devel\"/></package>";
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
            meta,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .set_devel_project(
                "test:factory",
                "test-pkgA",
                Some("test:devel"),
                None,
                &DryRun::new(false),
            )
            .is_ok());
        assert_eq!(http.get_calls().len(), 1);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_devel_project_no_change_with_prolog_makes_no_put() {
        // The fetched meta may carry an XML prolog, which the
        // rendered document never does: a string comparison would
        // read a change where there is none. The no-change check
        // compares parsed documents, so nothing is written.
        let meta = "<?xml version=\"1.0\"?><package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description><devel project=\"test:devel\"/></package>";
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
            meta,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .set_devel_project(
                "test:factory",
                "test-pkgA",
                Some("test:devel"),
                None,
                &DryRun::new(false),
            )
            .is_ok());
        assert_eq!(http.get_calls().len(), 1);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_devel_project_missing_package_is_not_found() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .set_devel_project(
                "test:factory",
                "test-pkgA",
                Some("test:devel"),
                None,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_devel_project_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
            "<package name=\"test-pkgA\" project=\"test:factory\"/>",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .set_devel_project(
                "test:factory",
                "test-pkgA",
                Some("test:devel"),
                None,
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_devel_project_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .set_devel_project(
                "test/factory",
                "test-pkgA",
                Some("test:devel"),
                None,
                &DryRun::new(false),
            )
            .is_err());
        assert!(client
            .set_devel_project(
                "test:factory",
                "test-pkgA",
                Some("test/devel"),
                None,
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn set_devel_project_dry_run_records_plan_and_changes_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        assert!(client
            .set_devel_project(
                "test:factory",
                "test-pkgA",
                Some("test:devel"),
                None,
                &dry_run,
            )
            .is_ok());
        assert_eq!(
            dry_run.plan(),
            vec![
                "[dry-run] would: set devel project of test:factory/test-pkgA to test:devel"
                    .to_string()
            ]
        );
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_devel_project_non_ok_body_is_error_not_success() {
        let meta = "<package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
                meta,
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/test-pkgA/_meta",
                "<status code=\"error\"><summary>nope</summary></status>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .set_devel_project(
                "test:factory",
                "test-pkgA",
                Some("test:devel"),
                None,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }
}
