//! `meta` — the unified meta write: one operation for both backends.
//!
//! Unified for the end user, separate backend logic underneath:
//! the caller names a target — an OBS project, an OBS package, or
//! a Gitea repository — and the same `set_meta` call writes it.
//! The OBS arm is the `set-project-meta` / `set-package-meta`
//! leaves' write (read the meta, edit the fields in place, PUT
//! only when something changed); the Gitea arm is the
//! repo-settings write (`GiteaClient::repo_edit`: PATCH only the
//! fields that are set).
//!
//! The fields that converge are the description and the URL (the
//! Gitea `website`): those are the fields this operation sets, on
//! either backend. What does not converge stays on its own leaf:
//! the OBS title, scmsync, devel project, and persons live on
//! `remote set-project-meta` / `remote set-package-meta`, and a
//! Gitea rename, privacy change, or default-branch change is not
//! a meta field at all: they stay out of this operation by scope —
//! the convergent field set is description + URL. A field this
//! operation does not set is never
//! touched on either backend, and there is no flag to clear a
//! field.
//!
//! The client holds the two backends side by side, like
//! `PullRequestClient`: each arm is present only when its client
//! was built, and an arm without its client fails closed naming
//! the credentials it needs. The client itself resolves no
//! credentials — the caller builds the arm clients, so the
//! unified surface works with whatever credential resolution the
//! caller has; it assumes no login scheme beyond the clients it
//! is given.

use crate::error::{ErrorKind, GeectlError};
use crate::gitea::{GiteaClient, RepoEditOptions};
use crate::obs::{DryRun, ObsClient, PackageMeta, ProjectMeta};

/// The backend a meta write targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaBackend {
    /// An OBS project or package.
    Obs,
    /// A Gitea repository.
    Gitea,
}

impl MetaBackend {
    /// The backend name as it appears in the result payload.
    pub fn as_str(&self) -> &'static str {
        match self {
            MetaBackend::Obs => "obs",
            MetaBackend::Gitea => "gitea",
        }
    }
}

/// The target of a meta write: an OBS project, an OBS package, or
/// a Gitea repository. The target selects the backend — no flag
/// does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaTarget {
    /// An OBS project's meta.
    ObsProject {
        /// The OBS project.
        project: String,
    },
    /// An OBS package's meta.
    ObsPackage {
        /// The OBS project the package lives in.
        project: String,
        /// The OBS package.
        package: String,
    },
    /// A Gitea repository's settings.
    GiteaRepo {
        /// The repository owner.
        owner: String,
        /// The repository name.
        repo: String,
    },
}

impl MetaTarget {
    /// The backend this target writes on.
    pub fn backend(&self) -> MetaBackend {
        match self {
            MetaTarget::ObsProject { .. } | MetaTarget::ObsPackage { .. } => MetaBackend::Obs,
            MetaTarget::GiteaRepo { .. } => MetaBackend::Gitea,
        }
    }

    /// The target as the result line names it: `project P`,
    /// `package P/K`, `repo O/R`.
    pub fn describe(&self) -> String {
        match self {
            MetaTarget::ObsProject { project } => format!("project {project}"),
            MetaTarget::ObsPackage { project, package } => {
                format!("package {project}/{package}")
            }
            MetaTarget::GiteaRepo { owner, repo } => format!("repo {owner}/{repo}"),
        }
    }
}

/// The fields a meta write sets: the description and the URL. An
/// absent field keeps the current value on either backend; on
/// Gitea an absent field is not sent at all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetaFields {
    /// A short description; omitted when unset.
    pub description: Option<String>,
    /// A URL with more information about the target; the Gitea
    /// `website` on the Gitea arm.
    pub url: Option<String>,
}

impl MetaFields {
    /// True when at least one field is set: a write with nothing
    /// to write is a no-op, refused before any call.
    pub fn has_fields(&self) -> bool {
        self.description.is_some() || self.url.is_some()
    }

    /// Validate the fields: at least one set, and no field empty
    /// — there is no flag to clear a field, so an empty value is
    /// an input error, not a clear.
    pub fn validate(&self) -> Result<(), GeectlError> {
        if !self.has_fields() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "meta set needs at least one of --description, --url",
            ));
        }
        if self.description.as_deref() == Some("") {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "--description must not be empty: there is no flag to clear a field",
            ));
        }
        if self.url.as_deref() == Some("") {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "--url must not be empty: there is no flag to clear a field",
            ));
        }
        Ok(())
    }

    /// The fields a `meta set` call sets, as `name: value` entries
    /// for the result line — the same shape as the `remote`
    /// set-meta leaves' field list.
    pub fn field_list(&self) -> Vec<String> {
        let mut fields = Vec::new();
        if let Some(description) = &self.description {
            fields.push(format!("description: {description}"));
        }
        if let Some(url) = &self.url {
            fields.push(format!("url: {url}"));
        }
        fields
    }
}

/// The meta a `set_meta` call wrote: the backend, the target, and
/// the resulting description and URL — the merged values on the
/// OBS arm, the edited repo's values on the Gitea arm, the flag
/// values in dry-run (where the current values are unread).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetMeta {
    /// The backend the write ran on.
    pub backend: MetaBackend,
    /// The target the write ran on.
    pub target: MetaTarget,
    /// The resulting description, when the meta carries one.
    pub description: Option<String>,
    /// The resulting URL, when the meta carries one.
    pub url: Option<String>,
}

/// The unified meta-write client: the OBS client and the Gitea
/// client side by side, each optional. A target's arm runs only
/// when its client is present; an arm without its client fails
/// closed naming the credentials it needs, like the
/// pull-request client's arms.
pub struct MetaClient<'a> {
    obs: Option<&'a ObsClient<'a>>,
    gitea: Option<&'a GiteaClient<'a>>,
}

impl<'a> MetaClient<'a> {
    /// Build the client from the arm clients: either may be
    /// absent, and the arms that are absent fail closed when
    /// their target is written.
    pub fn new(obs: Option<&'a ObsClient<'a>>, gitea: Option<&'a GiteaClient<'a>>) -> Self {
        Self { obs, gitea }
    }

    /// The OBS arm client, when one was built.
    pub fn obs(&self) -> Option<&ObsClient<'a>> {
        self.obs
    }

    /// The Gitea arm client, when one was built.
    pub fn gitea(&self) -> Option<&GiteaClient<'a>> {
        self.gitea
    }

    /// Write the meta of `target`: the description and/or URL in
    /// `fields`. On the OBS arm the meta is read, the fields are
    /// merged onto it, and the merged meta is written — the same
    /// in-place terms as `set-project-meta` / `set-package-meta`,
    /// so settings outside the typed meta survive untouched and
    /// nothing is PUT when nothing changed. The read is the
    /// authenticated read: the public endpoint 404s on a hidden
    /// target and answers a reduced view where it answers at all,
    /// so a merge base from the public read would refuse the write
    /// or wipe the fields the public view omits. A live read without
    /// OBS credentials fails as a configuration error before any
    /// request, like `set-project-meta` / `set-package-meta`.
    /// In dry-run the merge
    /// starts from the empty meta, like the `remote` leaves, and
    /// the write is recorded in the plan. On the Gitea arm the
    /// fields go through the repo edit, which sends only the
    /// fields that are set. The fields are validated before any
    /// arm runs, so an empty write fails as InvalidInput whether
    /// or not the arm's credentials are configured.
    pub fn set_meta(
        &self,
        target: &MetaTarget,
        fields: &MetaFields,
        dry_run: &DryRun,
    ) -> Result<SetMeta, GeectlError> {
        fields.validate()?;
        match target {
            MetaTarget::ObsProject { project } => {
                let obs = self.obs.ok_or_else(|| missing_obs_arm(target))?;
                let current = if dry_run.enabled() {
                    empty_project_meta(project)
                } else {
                    // The live read needs credentials: without
                    // them the unauthenticated GET 404s on a hidden
                    // target, so the write would report NotFound
                    // instead of the missing-credentials error.
                    // The same fail-fast guard `set_project_meta`
                    // applies before its own read.
                    if !obs.has_auth() {
                        return Err(missing_obs_arm(target));
                    }
                    obs.project_meta_authenticated(project)?
                };
                let merged = merge_project_meta(&current, fields);
                obs.set_project_meta(project, &merged, dry_run)?;
                Ok(SetMeta {
                    backend: MetaBackend::Obs,
                    target: target.clone(),
                    description: merged.description,
                    url: merged.url,
                })
            }
            MetaTarget::ObsPackage { project, package } => {
                let obs = self.obs.ok_or_else(|| missing_obs_arm(target))?;
                let current = if dry_run.enabled() {
                    empty_package_meta(project, package)
                } else {
                    // The live read needs credentials: without
                    // them the unauthenticated GET 404s on a hidden
                    // target, so the write would report NotFound
                    // instead of the missing-credentials error.
                    // The same fail-fast guard `set_package_meta`
                    // applies before its own read.
                    if !obs.has_auth() {
                        return Err(missing_obs_arm(target));
                    }
                    obs.package_meta_authenticated(project, package)?
                };
                let merged = merge_package_meta(&current, fields);
                obs.set_package_meta(project, package, &merged, dry_run)?;
                Ok(SetMeta {
                    backend: MetaBackend::Obs,
                    target: target.clone(),
                    description: merged.description,
                    url: merged.url,
                })
            }
            MetaTarget::GiteaRepo { owner, repo } => {
                let gitea = self.gitea.ok_or_else(missing_gitea_arm)?;
                let options = RepoEditOptions {
                    description: fields.description.clone(),
                    website: fields.url.clone(),
                };
                let edited = gitea.repo_edit(owner, repo, &options, dry_run)?;
                let (description, url) = match edited {
                    Some(repository) => (repository.description, repository.website),
                    None => (fields.description.clone(), fields.url.clone()),
                };
                Ok(SetMeta {
                    backend: MetaBackend::Gitea,
                    target: target.clone(),
                    description,
                    url,
                })
            }
        }
    }
}

/// The missing-OBS-arm error for `target`: the write needs OBS
/// credentials, named for the target it ran on — a package write
/// without them reports "OBS package", not "OBS project".
fn missing_obs_arm(target: &MetaTarget) -> GeectlError {
    // Only the OBS arms call this; a Gitea target never reaches
    // it, so it names the project wording as a defensive default.
    let what = match target {
        MetaTarget::ObsProject { .. } => "project",
        MetaTarget::ObsPackage { .. } => "package",
        MetaTarget::GiteaRepo { .. } => "project",
    };
    GeectlError::new(
        ErrorKind::ConfigError,
        format!(
            "cannot set meta on an OBS {what} without OBS credentials (GEECTL_OBS_APIKEY[_FILE] or GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD[_FILE])"
        ),
    )
}

/// The missing-Gitea-arm error: the write needs a Gitea token.
fn missing_gitea_arm() -> GeectlError {
    GeectlError::new(
        ErrorKind::ConfigError,
        "cannot set meta on a Gitea repository without a Gitea token (GEECTL_GITEA_TOKEN or GEECTL_GITEA_TOKEN_FILE)",
    )
}

/// An empty project meta: the dry-run merge base, like the
/// `remote` leaves' — the flags are the whole meta in dry-run,
/// since the current values are unread.
fn empty_project_meta(project: &str) -> ProjectMeta {
    ProjectMeta {
        name: project.to_string(),
        title: None,
        description: None,
        url: None,
        scmsync: None,
        devel: None,
        persons: Vec::new(),
        groups: Vec::new(),
        repositories: Vec::new(),
    }
}

/// An empty package meta: the dry-run merge base, like the
/// `remote` leaves'.
fn empty_package_meta(project: &str, package: &str) -> PackageMeta {
    PackageMeta {
        name: package.to_string(),
        project: project.to_string(),
        title: None,
        description: None,
        url: None,
        scmsync: None,
        releasename: None,
        devel: None,
        persons: Vec::new(),
        groups: Vec::new(),
    }
}

/// Merge the fields onto the current project meta: a field
/// replaces its value, an absent field keeps the current value.
fn merge_project_meta(current: &ProjectMeta, fields: &MetaFields) -> ProjectMeta {
    let mut merged = current.clone();
    if let Some(description) = &fields.description {
        merged.description = Some(description.clone());
    }
    if let Some(url) = &fields.url {
        merged.url = Some(url.clone());
    }
    merged
}

/// Merge the fields onto the current package meta: a field
/// replaces its value, an absent field keeps the current value.
fn merge_package_meta(current: &PackageMeta, fields: &MetaFields) -> PackageMeta {
    let mut merged = current.clone();
    if let Some(description) = &fields.description {
        merged.description = Some(description.clone());
    }
    if let Some(url) = &fields.url {
        merged.url = Some(url.clone());
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Credentials;
    use crate::gitea::GiteaClient;
    use crate::obs::test_support::{basic_credentials, FakeHttp};

    const PROJECT_META: &str = "<project name=\"home:scarabeus_iv\"><title>Old title</title><description>Old description</description><lock><enable/></lock><person userid=\"alice\" role=\"maintainer\"/><repository name=\"openSUSE_Tumbleweed\"><arch>x86_64</arch><path project=\"openSUSE:Factory\" repository=\"snapshot\"/></repository><build><disable/></build></project>";

    const PACKAGE_META: &str = "<package name=\"foo\" project=\"home:scarabeus_iv\"><title>Old title</title><description>Old description</description><url>https://example.com/old</url><scmsync>https://github.com/openSUSE/foo.git</scmsync><releasename>foo</releasename><devel project=\"devel:languages:python\"/><person userid=\"alice\" role=\"maintainer\"/><lock><enable/></lock></package>";

    const GITEA_BASE: &str = "https://src.opensuse.org";

    fn bearer() -> Credentials {
        Credentials::Bearer {
            token: ["test", "token"].join("-"),
        }
    }

    fn obs_client(http: &FakeHttp) -> ObsClient<'_> {
        ObsClient::with_credentials(http, basic_credentials()).unwrap()
    }

    fn obs_client_without_credentials(http: &FakeHttp) -> ObsClient<'_> {
        ObsClient::with_credentials(http, Credentials::None).unwrap()
    }

    fn gitea_client(http: &FakeHttp) -> GiteaClient<'_> {
        GiteaClient::with_base(http, GITEA_BASE, bearer()).unwrap()
    }

    /// A repo JSON body, shaped like Gitea 1.24.7 answers, with
    /// the given description and website: the edited repo the
    /// PATCH response carries.
    fn repo_json(description: &str, website: &str) -> String {
        format!(
            r#"{{"id": 7, "owner": {{"login": "pool"}}, "name": "systemd", "full_name": "pool/systemd", "description": "{description}", "website": "{website}", "empty": false, "private": false, "fork": false, "parent": null, "html_url": "{GITEA_BASE}/pool/systemd", "ssh_url": "gitea@src.opensuse.org:pool/systemd.git", "clone_url": "{GITEA_BASE}/pool/systemd.git", "default_branch": "factory"}}"#
        )
    }

    fn fields(description: Option<&str>, url: Option<&str>) -> MetaFields {
        MetaFields {
            description: description.map(str::to_string),
            url: url.map(str::to_string),
        }
    }

    #[test]
    fn obs_project_arm_sets_description_and_keeps_the_rest() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                PROJECT_META,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                "<status code=\"ok\" />",
            );
        let obs = obs_client(&http);
        let client = MetaClient::new(Some(&obs), None);
        let target = MetaTarget::ObsProject {
            project: "home:scarabeus_iv".to_string(),
        };
        let set = client
            .set_meta(
                &target,
                &fields(Some("New description"), None),
                &DryRun::new(false),
            )
            .unwrap();
        assert_eq!(set.backend, MetaBackend::Obs);
        assert_eq!(set.target, target);
        assert_eq!(set.description.as_deref(), Some("New description"));
        assert_eq!(set.url, None);
        // The merge-base read and the leaf's re-read, both on the
        // authenticated endpoint: the public endpoint is never read
        // on a write path.
        assert_eq!(
            http.get_calls(),
            vec![
                (
                    "https://api.opensuse.org/source/home:scarabeus_iv/_meta".to_string(),
                    true
                ),
                (
                    "https://api.opensuse.org/source/home:scarabeus_iv/_meta".to_string(),
                    true
                ),
            ]
        );
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        let rendered = &puts[0].1;
        assert!(rendered.contains("<description>New description</description>"));
        assert!(rendered.contains("<title>Old title</title>"));
        assert!(rendered.contains("<lock><enable/></lock>"));
        assert!(rendered.contains("<person userid=\"alice\" role=\"maintainer\"/>"));
        assert!(rendered.contains("<repository name=\"openSUSE_Tumbleweed\">"));
    }

    #[test]
    fn obs_project_arm_writes_a_hidden_project_the_public_read_404s_on() {
        // A hidden project 404s on the public endpoint while the
        // authenticated read succeeds: the OBS arm reads its merge
        // base authenticated, so the write goes through and nothing
        // beyond the given field changes.
        let http = FakeHttp::new()
            .with_get_error(
                "https://api.opensuse.org/public/source/home:scarabeus_iv/_meta",
                ErrorKind::NotFound,
            )
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                PROJECT_META,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
                "<status code=\"ok\" />",
            );
        let obs = obs_client(&http);
        let client = MetaClient::new(Some(&obs), None);
        let set = client
            .set_meta(
                &MetaTarget::ObsProject {
                    project: "home:scarabeus_iv".to_string(),
                },
                &fields(Some("New description"), None),
                &DryRun::new(false),
            )
            .unwrap();
        assert_eq!(set.description.as_deref(), Some("New description"));
        assert_eq!(set.url, None);
        assert!(
            http.get_calls().iter().all(|(url, authed)| {
                url == "https://api.opensuse.org/source/home:scarabeus_iv/_meta" && *authed
            }),
            "no read may touch the public endpoint: {:?}",
            http.get_calls()
        );
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        let rendered = &puts[0].1;
        assert!(rendered.contains("<description>New description</description>"));
        assert!(rendered.contains("<title>Old title</title>"));
        assert!(rendered.contains("<lock><enable/></lock>"));
        assert!(rendered.contains("<person userid=\"alice\" role=\"maintainer\"/>"));
        assert!(rendered.contains("<repository name=\"openSUSE_Tumbleweed\">"));
    }

    #[test]
    fn obs_package_arm_sets_url_and_keeps_the_rest() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta",
                PACKAGE_META,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta",
                "<status code=\"ok\" />",
            );
        let obs = obs_client(&http);
        let client = MetaClient::new(Some(&obs), None);
        let target = MetaTarget::ObsPackage {
            project: "home:scarabeus_iv".to_string(),
            package: "foo".to_string(),
        };
        let set = client
            .set_meta(
                &target,
                &fields(None, Some("https://example.com/new")),
                &DryRun::new(false),
            )
            .unwrap();
        assert_eq!(set.description.as_deref(), Some("Old description"));
        assert_eq!(set.url.as_deref(), Some("https://example.com/new"));
        // The merge-base read and the leaf's re-read, both on the
        // authenticated endpoint: the public endpoint is never read
        // on a write path.
        assert_eq!(
            http.get_calls(),
            vec![
                (
                    "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta".to_string(),
                    true
                ),
                (
                    "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta".to_string(),
                    true
                ),
            ]
        );
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        let rendered = &puts[0].1;
        assert!(rendered.contains("<url>https://example.com/new</url>"));
        assert!(rendered.contains("<description>Old description</description>"));
        assert!(rendered.contains("<title>Old title</title>"));
    }

    #[test]
    fn obs_package_arm_writes_a_hidden_package_the_public_read_404s_on() {
        // A hidden package 404s on the public endpoint while the
        // authenticated read succeeds: the OBS arm reads its merge
        // base authenticated, so the write goes through and nothing
        // beyond the given field changes.
        let http = FakeHttp::new()
            .with_get_error(
                "https://api.opensuse.org/public/source/home:scarabeus_iv/foo/_meta",
                ErrorKind::NotFound,
            )
            .with_get(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta",
                PACKAGE_META,
            )
            .with_put(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta",
                "<status code=\"ok\" />",
            );
        let obs = obs_client(&http);
        let client = MetaClient::new(Some(&obs), None);
        let set = client
            .set_meta(
                &MetaTarget::ObsPackage {
                    project: "home:scarabeus_iv".to_string(),
                    package: "foo".to_string(),
                },
                &fields(None, Some("https://example.com/new")),
                &DryRun::new(false),
            )
            .unwrap();
        assert_eq!(set.description.as_deref(), Some("Old description"));
        assert_eq!(set.url.as_deref(), Some("https://example.com/new"));
        assert!(
            http.get_calls().iter().all(|(url, authed)| {
                url == "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta" && *authed
            }),
            "no read may touch the public endpoint: {:?}",
            http.get_calls()
        );
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        let rendered = &puts[0].1;
        assert!(rendered.contains("<url>https://example.com/new</url>"));
        assert!(rendered.contains("<description>Old description</description>"));
        assert!(rendered.contains("<title>Old title</title>"));
        assert!(rendered.contains("devel:languages:python"));
        assert!(rendered.contains("<person userid=\"alice\" role=\"maintainer\"/>"));
    }

    #[test]
    fn obs_arms_fail_fast_without_credentials_before_any_request() {
        // A live write without OBS credentials fails as a
        // configuration error before any request: the merge-base
        // read must not go out unauthenticated, where a hidden
        // target would answer 404 and the write would report
        // NotFound instead of the missing credentials.
        for target in [
            MetaTarget::ObsProject {
                project: "home:scarabeus_iv".to_string(),
            },
            MetaTarget::ObsPackage {
                project: "home:scarabeus_iv".to_string(),
                package: "foo".to_string(),
            },
        ] {
            let http = FakeHttp::new();
            let obs = obs_client_without_credentials(&http);
            let client = MetaClient::new(Some(&obs), None);
            let err = client
                .set_meta(
                    &target,
                    &fields(Some("New description"), None),
                    &DryRun::new(false),
                )
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ConfigError);
            let message = err.to_string();
            let noun = match target {
                MetaTarget::ObsProject { .. } => "OBS project",
                MetaTarget::ObsPackage { .. } => "OBS package",
                MetaTarget::GiteaRepo { .. } => unreachable!(),
            };
            assert!(
                message.contains(&format!("cannot set meta on an {noun}")),
                "message: {message}"
            );
            assert!(http.get_calls().is_empty());
        }
    }

    #[test]
    fn obs_project_arm_dry_run_plans_without_credentials() {
        // The guard runs on the live read only: a dry-run merge
        // starts from the empty meta, so the plan prints without
        // credentials, like the `remote` leaves.
        let http = FakeHttp::new();
        let obs = obs_client_without_credentials(&http);
        let client = MetaClient::new(Some(&obs), None);
        let dry_run = DryRun::new(true);
        let set = client
            .set_meta(
                &MetaTarget::ObsProject {
                    project: "home:scarabeus_iv".to_string(),
                },
                &fields(Some("New description"), None),
                &dry_run,
            )
            .unwrap();
        assert_eq!(set.description.as_deref(), Some("New description"));
        assert!(http.get_calls().is_empty());
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: set project meta of home:scarabeus_iv".to_string()]
        );
    }

    #[test]
    fn obs_arm_dry_run_plans_and_writes_nothing() {
        let http = FakeHttp::new();
        let obs = obs_client(&http);
        let client = MetaClient::new(Some(&obs), None);
        let dry_run = DryRun::new(true);
        let set = client
            .set_meta(
                &MetaTarget::ObsProject {
                    project: "home:scarabeus_iv".to_string(),
                },
                &fields(Some("New description"), None),
                &dry_run,
            )
            .unwrap();
        assert_eq!(set.description.as_deref(), Some("New description"));
        assert_eq!(set.url, None);
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: set project meta of home:scarabeus_iv".to_string()]
        );
    }

    #[test]
    fn obs_project_arm_unchanged_writes_nothing() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/_meta",
            PROJECT_META,
        );
        let obs = obs_client(&http);
        let client = MetaClient::new(Some(&obs), None);
        let set = client
            .set_meta(
                &MetaTarget::ObsProject {
                    project: "home:scarabeus_iv".to_string(),
                },
                &fields(Some("Old description"), None),
                &DryRun::new(false),
            )
            .unwrap();
        assert_eq!(set.description.as_deref(), Some("Old description"));
        assert_eq!(set.url, None);
        // The merge-base read plus the leaf's re-read, both
        // authenticated, and no PUT: the requested description is
        // already the current one, so the merged meta equals the
        // fetched meta.
        assert_eq!(http.get_calls().len(), 2);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn obs_package_arm_unchanged_writes_nothing() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:scarabeus_iv/foo/_meta",
            PACKAGE_META,
        );
        let obs = obs_client(&http);
        let client = MetaClient::new(Some(&obs), None);
        let set = client
            .set_meta(
                &MetaTarget::ObsPackage {
                    project: "home:scarabeus_iv".to_string(),
                    package: "foo".to_string(),
                },
                &fields(Some("Old description"), Some("https://example.com/old")),
                &DryRun::new(false),
            )
            .unwrap();
        assert_eq!(set.description.as_deref(), Some("Old description"));
        assert_eq!(set.url.as_deref(), Some("https://example.com/old"));
        assert_eq!(http.get_calls().len(), 2);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn obs_package_arm_dry_run_plans_and_writes_nothing() {
        let http = FakeHttp::new();
        let obs = obs_client(&http);
        let client = MetaClient::new(Some(&obs), None);
        let dry_run = DryRun::new(true);
        let set = client
            .set_meta(
                &MetaTarget::ObsPackage {
                    project: "home:scarabeus_iv".to_string(),
                    package: "foo".to_string(),
                },
                &fields(None, Some("https://example.com/new")),
                &dry_run,
            )
            .unwrap();
        assert_eq!(set.description, None);
        assert_eq!(set.url.as_deref(), Some("https://example.com/new"));
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: set package meta of home:scarabeus_iv/foo".to_string()]
        );
    }

    #[test]
    fn gitea_arm_sets_description_and_website() {
        let http = FakeHttp::new().with_patch(
            "https://src.opensuse.org/api/v1/repos/pool/systemd",
            &repo_json("New description", "https://example.com/systemd"),
        );
        let gitea = gitea_client(&http);
        let client = MetaClient::new(None, Some(&gitea));
        let target = MetaTarget::GiteaRepo {
            owner: "pool".to_string(),
            repo: "systemd".to_string(),
        };
        let set = client
            .set_meta(
                &target,
                &fields(Some("New description"), Some("https://example.com/systemd")),
                &DryRun::new(false),
            )
            .unwrap();
        assert_eq!(set.backend, MetaBackend::Gitea);
        assert_eq!(set.target, target);
        assert_eq!(set.description.as_deref(), Some("New description"));
        assert_eq!(set.url.as_deref(), Some("https://example.com/systemd"));
        assert_eq!(
            http.patch_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/repos/pool/systemd".to_string(),
                r#"{"description":"New description","website":"https://example.com/systemd"}"#
                    .to_string(),
                true
            )]
        );
    }

    #[test]
    fn gitea_arm_dry_run_plans_and_patches_nothing() {
        let http = FakeHttp::new();
        let gitea = gitea_client(&http);
        let client = MetaClient::new(None, Some(&gitea));
        let dry_run = DryRun::new(true);
        let set = client
            .set_meta(
                &MetaTarget::GiteaRepo {
                    owner: "pool".to_string(),
                    repo: "systemd".to_string(),
                },
                &fields(Some("New description"), None),
                &dry_run,
            )
            .unwrap();
        assert_eq!(set.description.as_deref(), Some("New description"));
        assert_eq!(set.url, None);
        assert!(http.patch_calls().is_empty());
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: edit repository pool/systemd".to_string()]
        );
    }

    #[test]
    fn gitea_arm_sets_website_only() {
        let http = FakeHttp::new().with_patch(
            "https://src.opensuse.org/api/v1/repos/pool/systemd",
            &repo_json("Old description", "https://example.com/systemd"),
        );
        let gitea = gitea_client(&http);
        let client = MetaClient::new(None, Some(&gitea));
        let target = MetaTarget::GiteaRepo {
            owner: "pool".to_string(),
            repo: "systemd".to_string(),
        };
        let set = client
            .set_meta(
                &target,
                &fields(None, Some("https://example.com/systemd")),
                &DryRun::new(false),
            )
            .unwrap();
        assert_eq!(set.backend, MetaBackend::Gitea);
        assert_eq!(set.target, target);
        assert_eq!(set.description.as_deref(), Some("Old description"));
        assert_eq!(set.url.as_deref(), Some("https://example.com/systemd"));
        assert_eq!(
            http.patch_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/repos/pool/systemd".to_string(),
                r#"{"website":"https://example.com/systemd"}"#.to_string(),
                true
            )]
        );
    }

    #[test]
    fn set_meta_needs_at_least_one_field() {
        let http = FakeHttp::new();
        let obs = obs_client(&http);
        let client = MetaClient::new(Some(&obs), None);
        let err = client
            .set_meta(
                &MetaTarget::ObsProject {
                    project: "home:scarabeus_iv".to_string(),
                },
                &MetaFields::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn set_meta_refuses_an_empty_field() {
        let http = FakeHttp::new();
        let obs = obs_client(&http);
        let client = MetaClient::new(Some(&obs), None);
        let err = client
            .set_meta(
                &MetaTarget::ObsProject {
                    project: "home:scarabeus_iv".to_string(),
                },
                &fields(Some(""), None),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn set_meta_refuses_an_empty_url() {
        let http = FakeHttp::new();
        let obs = obs_client(&http);
        let client = MetaClient::new(Some(&obs), None);
        let err = client
            .set_meta(
                &MetaTarget::ObsProject {
                    project: "home:scarabeus_iv".to_string(),
                },
                &fields(None, Some("")),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn set_meta_without_an_arm_is_a_configuration_error() {
        let client = MetaClient::new(None, None);
        for target in [
            MetaTarget::ObsProject {
                project: "home:scarabeus_iv".to_string(),
            },
            MetaTarget::ObsPackage {
                project: "home:scarabeus_iv".to_string(),
                package: "foo".to_string(),
            },
            MetaTarget::GiteaRepo {
                owner: "pool".to_string(),
                repo: "systemd".to_string(),
            },
        ] {
            let err = client
                .set_meta(
                    &target,
                    &fields(Some("New description"), None),
                    &DryRun::new(false),
                )
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ConfigError);
            let message = err.to_string();
            let phrase = match target {
                MetaTarget::ObsProject { .. } => "cannot set meta on an OBS project",
                MetaTarget::ObsPackage { .. } => "cannot set meta on an OBS package",
                MetaTarget::GiteaRepo { .. } => "cannot set meta on a Gitea repository",
            };
            assert!(message.contains(phrase), "message: {message}");
        }
    }

    #[test]
    fn set_meta_without_an_arm_refuses_an_empty_write_as_invalid_input() {
        let client = MetaClient::new(None, None);
        for target in [
            MetaTarget::ObsProject {
                project: "home:scarabeus_iv".to_string(),
            },
            MetaTarget::ObsPackage {
                project: "home:scarabeus_iv".to_string(),
                package: "foo".to_string(),
            },
            MetaTarget::GiteaRepo {
                owner: "pool".to_string(),
                repo: "systemd".to_string(),
            },
        ] {
            // Validation runs before any arm runs: an empty write
            // is InvalidInput even with no arm configured, not
            // the missing-arm configuration error — and with no
            // arm there is no call to make.
            for write in [MetaFields::default(), fields(Some(""), None)] {
                let err = client
                    .set_meta(&target, &write, &DryRun::new(false))
                    .unwrap_err();
                assert_eq!(err.kind(), ErrorKind::InvalidInput);
            }
        }
    }
}
