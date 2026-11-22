use super::build_url;
use super::find_element_tag;
use super::parse_status_ok;
use super::tag_attr;
use super::valid_obs_name;
use super::DryRun;
use super::MaintainedInstance;
use super::MaintenanceBranchOptions;
use super::MaintenanceBranchResult;
use super::ObsClient;
use crate::error::ErrorKind;
use crate::error::GeectlError;

/// The maintained instances in a dry-run branch report:
/// `<collection><package project package><devel project package/>?
/// <target project package/></package>...</collection>`. None when
/// the body is not a collection at all, or when a package element is
/// malformed (missing source names, or no target). An empty
/// collection reads as an empty list, not None. The report's target
/// package carries the source project appended, colons intact; the
/// parser translates `:` to `_`, matching the live branch name.
pub(crate) fn parse_maintained_instances(body: &str) -> Option<Vec<MaintainedInstance>> {
    if !body.contains("<collection") {
        return None;
    }
    let mut instances = Vec::new();
    let mut rest = body;
    while let Some(pos) = rest.find("<package") {
        let after_tag = &rest[pos + "<package".len()..];
        let first = after_tag.chars().next()?;
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let end = after_tag.find('>')?;
        let tag = &after_tag[..end];
        let source_project = tag_attr(tag, "project")?;
        let source_package = tag_attr(tag, "package")?;
        if tag.trim_end().ends_with('/') {
            // Self-closing: no children, so no target — malformed.
            return None;
        }
        let after_open = &after_tag[end + 1..];
        let close = after_open.find("</package>")?;
        let inner = &after_open[..close];
        // Optional <devel project package/>: only when the package
        // has a devel package to copy newer sources from.
        let (devel_project, devel_package) = match find_element_tag(inner, "devel") {
            Some(devel_tag) => (
                Some(tag_attr(devel_tag, "project")?),
                Some(tag_attr(devel_tag, "package")?),
            ),
            None => (None, None),
        };
        let target_tag = find_element_tag(inner, "target")?;
        let target_project = tag_attr(target_tag, "project")?;
        // The report names the target package with the source project
        // appended, colons intact (`foo.openSUSE:Leap:15.6:Update`);
        // the live branch translates `:` to `_`, so the listing does
        // the same — otherwise it would report a package name that
        // never lands.
        let target_package = tag_attr(target_tag, "package")?.replace(':', "_");
        instances.push(MaintainedInstance {
            source_project,
            source_package,
            devel_project,
            devel_package,
            target_project,
            target_package,
        });
        rest = &after_open[close + "</package>".len()..];
    }
    Some(instances)
}

/// The branch result in a `render_ok` branch response:
/// `<status code="ok"><data name="target_project">...</data>...`.
/// `target_project` is required; the rest is present only for a
/// single-package branch. None when the status is not ok or the
/// target project is missing.
pub(crate) fn parse_maintenance_branch_result(body: &str) -> Option<MaintenanceBranchResult> {
    if !parse_status_ok(body) {
        return None;
    }
    let mut target_project = None;
    let mut target_package = None;
    let mut source_project = None;
    let mut source_package = None;
    let mut rest = body;
    while let Some(pos) = rest.find("<data") {
        let after_tag = &rest[pos + "<data".len()..];
        let first = after_tag.chars().next()?;
        if !(first.is_whitespace() || first == '>') {
            rest = after_tag;
            continue;
        }
        let end = after_tag.find('>')?;
        let tag = &after_tag[..end];
        let name = tag_attr(tag, "name")?;
        let inner = &after_tag[end + 1..];
        let close = inner.find("</data>")?;
        let value = &inner[..close];
        if !value.is_empty() {
            match name.as_str() {
                "target_project" => target_project = Some(value.to_string()),
                "target_package" => target_package = Some(value.to_string()),
                "project" => source_project = Some(value.to_string()),
                "package" => source_package = Some(value.to_string()),
                _ => {}
            }
        }
        rest = &inner[close + "</data>".len()..];
    }
    Some(MaintenanceBranchResult {
        target_project: target_project?,
        target_package,
        source_project,
        source_package,
    })
}

impl<'a> ObsClient<'a> {
    /// List the maintained instances of `package`: the projects
    /// carrying `options`' attribute that the package exists in,
    /// with where a branch would land. This is the dry-run branch
    /// call (`POST /source?cmd=branch&dryrun=1`) — it reports, it
    /// does not branch. The update project attribute in `options`
    /// decides which update project the report branches from.
    /// Requires credentials; an unexpected response is an error,
    /// never an empty list.
    pub fn maintained_instances(
        &self,
        package: &str,
        options: &MaintenanceBranchOptions,
    ) -> Result<Vec<MaintainedInstance>, GeectlError> {
        if !valid_obs_name(package) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS package name: {package:?}"),
            ));
        }
        let url = build_url(
            &self.base,
            &["source"],
            &self.branch_query(package, options, true),
        );
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "listing maintained instances requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let body = self.http.post(&url, "", &self.credentials)?;
        parse_maintained_instances(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS branch dry-run response",
            )
        })
    }

    /// Branch `package` into its maintenance branch project and
    /// return where it landed. A mutation: the package name is
    /// validated first, then the DryRun guard — in dry-run nothing is
    /// branched and None is returned. The branch project defaults to
    /// `home:<user>:branches:OBS_Maintained:<package>` (the attribute,
    /// with `:` → `_`); pass a target project in `options` to override
    /// it. Requires credentials; an unexpected response is an error.
    /// The server refuses to branch a package whose sources live in
    /// scmsync ("Fork it instead") — that refusal surfaces as the
    /// server's error.
    pub fn maintenance_branch(
        &self,
        package: &str,
        options: &MaintenanceBranchOptions,
        dry_run: &DryRun,
    ) -> Result<Option<MaintenanceBranchResult>, GeectlError> {
        if !valid_obs_name(package) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS package name: {package:?}"),
            ));
        }
        let what = match &options.target_project {
            Some(target) => format!("branch maintained package {package} to {target}"),
            None => format!("branch maintained package {package}"),
        };
        if !dry_run.allow(&what) {
            return Ok(None);
        }
        let url = build_url(
            &self.base,
            &["source"],
            &self.branch_query(package, options, false),
        );
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "branching a maintained package requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let body = self.http.post(&url, "", &self.credentials)?;
        let result = parse_maintenance_branch_result(&body).ok_or_else(|| {
            GeectlError::new(ErrorKind::ServerError, "unexpected OBS branch response")
        })?;
        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use crate::obs::test_support::{basic_credentials, FakeHttp};

    fn dry_run_report() -> &'static str {
        "<collection><package project=\"openSUSE:Leap:15.6:Update\" package=\"foo\"><devel project=\"devel:languages:python\" package=\"foo\" /><target project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo.openSUSE:Leap:15.6:Update\" /></package><package project=\"openSUSE:Leap:15.5:Update\" package=\"foo\"><target project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo.openSUSE:Leap:15.5:Update\" /></package></collection>"
    }

    #[test]
    fn maintained_instances_parses_dry_run_report() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source?cmd=branch&attribute=OBS%3AMaintained&update_project_attribute=OBS%3AUpdateProject&package=foo&dryrun=1",
            dry_run_report(),
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let instances = client
            .maintained_instances("foo", &MaintenanceBranchOptions::new())
            .unwrap();
        assert_eq!(
            instances,
            vec![
                MaintainedInstance {
                    source_project: "openSUSE:Leap:15.6:Update".to_string(),
                    source_package: "foo".to_string(),
                    devel_project: Some("devel:languages:python".to_string()),
                    devel_package: Some("foo".to_string()),
                    target_project: "home:alice:branches:OBS_Maintained:foo".to_string(),
                    target_package: "foo.openSUSE_Leap_15.6_Update".to_string(),
                },
                MaintainedInstance {
                    source_project: "openSUSE:Leap:15.5:Update".to_string(),
                    source_package: "foo".to_string(),
                    devel_project: None,
                    devel_package: None,
                    target_project: "home:alice:branches:OBS_Maintained:foo".to_string(),
                    target_package: "foo.openSUSE_Leap_15.5_Update".to_string(),
                },
            ]
        );
        // The listing is a POST (the branch command), with an empty
        // body, and it carried the credentials.
        let posted = http.post_calls();
        assert_eq!(posted.len(), 1);
        assert_eq!(posted[0].1, "");
        assert!(posted[0].2);
    }

    #[test]
    fn maintained_instances_empty_collection_is_empty() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source?cmd=branch&attribute=OBS%3AMaintained&update_project_attribute=OBS%3AUpdateProject&package=foo&dryrun=1",
            "<collection></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .maintained_instances("foo", &MaintenanceBranchOptions::new())
                .unwrap(),
            Vec::<MaintainedInstance>::new()
        );
    }

    #[test]
    fn maintained_instances_malformed_is_server_error_not_empty() {
        // Not a collection at all: a failed lookup, never "no
        // maintained instances".
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source?cmd=branch&attribute=OBS%3AMaintained&update_project_attribute=OBS%3AUpdateProject&package=foo&dryrun=1",
            "not xml",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .maintained_instances("foo", &MaintenanceBranchOptions::new())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        // A package element without a target is malformed too.
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source?cmd=branch&attribute=OBS%3AMaintained&update_project_attribute=OBS%3AUpdateProject&package=foo&dryrun=1",
            "<collection><package project=\"openSUSE:Leap:15.6:Update\" package=\"foo\"></package></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .maintained_instances("foo", &MaintenanceBranchOptions::new())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn maintained_instances_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let err = client
            .maintained_instances("foo", &MaintenanceBranchOptions::new())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn maintained_instances_rejects_invalid_package_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .maintained_instances("foo/bar", &MaintenanceBranchOptions::new())
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn maintenance_branch_live_returns_target() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source?cmd=branch&attribute=OBS%3AMaintained&update_project_attribute=OBS%3AUpdateProject&package=foo",
            "<status code=\"ok\"><summary>Ok</summary><data name=\"target_project\">home:alice:branches:OBS_Maintained:foo</data><data name=\"target_package\">foo</data><data name=\"project\">openSUSE:Leap:15.6:Update</data><data name=\"package\">foo</data></status>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        assert_eq!(
            client
                .maintenance_branch("foo", &MaintenanceBranchOptions::new(), &dry)
                .unwrap(),
            Some(MaintenanceBranchResult {
                target_project: "home:alice:branches:OBS_Maintained:foo".to_string(),
                target_package: Some("foo".to_string()),
                source_project: Some("openSUSE:Leap:15.6:Update".to_string()),
                source_package: Some("foo".to_string()),
            })
        );
        // No dryrun flag on the live call, empty body, credentials sent.
        let posted = http.post_calls();
        assert_eq!(posted.len(), 1);
        assert_eq!(
            posted[0].0,
            "https://api.opensuse.org/source?cmd=branch&attribute=OBS%3AMaintained&update_project_attribute=OBS%3AUpdateProject&package=foo"
        );
        assert_eq!(posted[0].1, "");
        assert!(posted[0].2);
    }

    #[test]
    fn maintenance_branch_multi_package_reports_only_project() {
        // Several packages branched: the server reports only the
        // target project (plus the deprecated targetproject alias,
        // which the parser ignores).
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source?cmd=branch&attribute=OBS%3AMaintained&update_project_attribute=OBS%3AUpdateProject&package=foo",
            "<status code=\"ok\"><summary>Ok</summary><data name=\"target_project\">home:alice:branches:OBS_Maintained:foo</data><data name=\"targetproject\">home:alice:branches:OBS_Maintained:foo</data></status>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        assert_eq!(
            client
                .maintenance_branch("foo", &MaintenanceBranchOptions::new(), &dry)
                .unwrap(),
            Some(MaintenanceBranchResult {
                target_project: "home:alice:branches:OBS_Maintained:foo".to_string(),
                target_package: None,
                source_project: None,
                source_package: None,
            })
        );
    }

    #[test]
    fn maintenance_branch_dry_run_returns_none_and_posts_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::default();
        assert_eq!(
            client
                .maintenance_branch("foo", &MaintenanceBranchOptions::new(), &dry)
                .unwrap(),
            None
        );
        assert!(http.post_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec!["[dry-run] would: branch maintained package foo".to_string()]
        );
    }

    #[test]
    fn maintenance_branch_dry_run_rejects_invalid_package() {
        // Validation runs before the DryRun guard: a dry-run with an
        // invalid package name fails, like the filing methods.
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::default();
        assert!(client
            .maintenance_branch("foo/bar", &MaintenanceBranchOptions::new(), &dry)
            .is_err());
        assert!(http.post_calls().is_empty());
        assert!(dry.plan().is_empty());
    }

    #[test]
    fn maintenance_branch_with_target_project_override() {
        let options = MaintenanceBranchOptions::new()
            .with_target_project("home:alice:branches:custom")
            .unwrap();
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source?cmd=branch&attribute=OBS%3AMaintained&update_project_attribute=OBS%3AUpdateProject&package=foo&target_project=home%3Aalice%3Abranches%3Acustom",
            "<status code=\"ok\"><summary>Ok</summary><data name=\"target_project\">home:alice:branches:custom</data><data name=\"target_package\">foo</data></status>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        assert_eq!(
            client
                .maintenance_branch("foo", &options, &dry)
                .unwrap()
                .unwrap()
                .target_project,
            "home:alice:branches:custom".to_string()
        );
    }

    #[test]
    fn maintenance_branch_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let dry = DryRun::new(false);
        let err = client
            .maintenance_branch("foo", &MaintenanceBranchOptions::new(), &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn parse_maintenance_branch_result_rejects_non_ok_and_missing_target() {
        assert!(parse_maintenance_branch_result(
            "<status code=\"error\"><summary>nope</summary></status>"
        )
        .is_none());
        assert!(parse_maintenance_branch_result(
            "<status code=\"ok\"><summary>Ok</summary></status>"
        )
        .is_none());
        assert!(parse_maintenance_branch_result("not xml").is_none());
    }
}
