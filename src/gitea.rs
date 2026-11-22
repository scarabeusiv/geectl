//! The Gitea operations for git-backed packages: repo read, fork,
//! repo delete, repo list, repo create, branch read and branch head,
//! and the `branch` composite (Gitea fork + OBS `cmd=fork`) behind
//! the uniform flow.
//!
//! The client is internal plumbing, not the user-facing surface:
//! the uniform flow verbs (`branch`, `submit`, …) are what an agent
//! calls, and the `Gitea*` types never appear in a uniform-flow
//! signature. The module is public so the live harness can exercise
//! the typed operations against a real server; the uniform flow
//! itself dispatches to this client internally on the backend kind
//! recorded at checkout.
//!
//! Wire rules: JSON via `serde` `Deserialize`, unknown fields
//! ignored — the structs tolerate a newer Gitea than the pinned
//! test image (1.24.7, `tests/gitea/provision.sh`) without
//! breaking. Only the fields each operation needs are modelled.
//! The `HttpClient` trait is reused as-is (`get_with`,
//! `post_json_with`, `delete`); the Gitea client adds no transport of
//! its own. Gitea answers fork creation with 202 and the created
//! fork is NOT verified by the response alone — the fork is
//! re-read and its `parent` checked before success is reported.
//!
//! Auth is a Bearer token (the Gitea API takes no other HTTP
//! auth); construction fails closed when no token is configured.
//! The credentials held here are the single source for every call
//! the client makes: `get_with`, `post_json_with`, and `delete` all
//! take them explicitly, so the client never consults the HTTP
//! client's own auth provider. A client built with
//! [`GiteaClient::with_base_resolved`] carries the credentials
//! resolved for its own base URL (see
//! [`crate::auth::resolve_gitea_bearer_for_url`]), so two clients
//! for two instances carry different credentials, and no call can
//! silently authenticate as someone else.

use crate::auth::{
    alias_for_url, gitea_alias_vars, resolve_gitea_bearer_for_url, same_api_url, Credentials,
    FileReader, VarLookup, GITEA_TOKEN_FILE_VAR, GITEA_TOKEN_VAR,
};
use crate::config::DEFAULT_GITEA_URL;
use crate::debug::Debug;
use crate::error::ErrorKind;
use crate::error::GeectlError;
use crate::http::is_loopback_url;
use crate::http::HttpClient;
use crate::obs::build_url;
use crate::obs::common::is_bare_authority;
use crate::obs::valid_obs_name;
use crate::obs::DryRun;
use crate::obs::ObsClient;
use crate::url::UrlBuilder;
use serde::Deserialize;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

/// A Gitea user: only the login is modelled (the fork target-owner
/// resolution reads it from `GET /api/v1/user`).
#[derive(Debug, Deserialize)]
struct GiteaUser {
    login: String,
}

/// A Gitea account (owner): only the login is modelled.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GiteaOwner {
    /// The login name.
    pub login: String,
}

/// The parent of a fork: only the identity is modelled — the fork
/// verification compares the parent's full name with the source.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GiteaParent {
    /// The parent's owner.
    pub owner: GiteaOwner,
    /// The parent's repo name.
    pub name: String,
    /// The parent's `owner/name`.
    pub full_name: String,
}

/// A Gitea repository: only the fields the operations need are modelled — identity, parent, default
/// branch, privacy, and the clone URLs the fork result returns. A
/// field the operation does not need is not parsed.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GiteaRepository {
    /// The repo's owner.
    pub owner: GiteaOwner,
    /// The repo name.
    pub name: String,
    /// `owner/name`.
    pub full_name: String,
    /// The repo this one was forked from, when it is a fork.
    #[serde(default)]
    pub parent: Option<GiteaParent>,
    /// The repo's default branch.
    pub default_branch: String,
    /// Whether the repo is private.
    pub private: bool,
    /// The HTTPS clone URL.
    pub clone_url: String,
    /// The SSH clone URL.
    pub ssh_url: String,
}

/// Options for the Gitea-side fork step (`fork_repo`).
#[derive(Debug, Clone, Default)]
pub struct ForkRepoOptions {
    /// Name for the fork; defaults to the source repo name.
    pub new_name: Option<String>,
    /// Organization to fork into; defaults to the token owner's
    /// account.
    pub organization: Option<String>,
}

/// Options for the `branch` composite (Gitea fork + OBS `cmd=fork`).
#[derive(Debug, Clone, Default)]
pub struct ForkOptions {
    /// Name for the fork; defaults to the source repo name.
    pub new_repo_name: Option<String>,
    /// Organization to fork into; defaults to the token owner's
    /// account.
    pub organization: Option<String>,
    /// Branch to fork from and name in the fork's scmsync URL (the
    /// explicit `--git-branch`); defaults to the source branch.
    pub fork_branch: Option<String>,
    /// Fork from the named project/package even when it has a devel
    /// project.
    pub no_devel_project: bool,
    /// OBS target project for the fork; defaults to the server
    /// default (`home:<user>:branches:<project>`).
    pub target_project: Option<String>,
    /// OBS target package for the fork; defaults to the package
    /// name.
    pub target_package: Option<String>,
}

/// The result of the `branch` composite: the fork repo, the scmsync
/// URL the OBS side syncs from, the OBS target project/package, and
/// the clone URLs for the agent's own `git clone`. No local clone
/// is made. Plain fields only — no `Gitea*` type leaks into the
/// uniform-flow result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkResult {
    /// The fork's owner (the token owner, or the organization).
    pub owner: String,
    /// The fork's repo name.
    pub repo: String,
    /// The branch named in the fork's scmsync URL.
    pub branch: String,
    /// The scmsync URL the OBS fork syncs from.
    pub scmsync: String,
    /// The OBS project the fork lives in.
    pub target_project: String,
    /// The OBS package the fork lives in.
    pub target_package: String,
    /// The HTTPS clone URL of the fork.
    pub clone_url: String,
    /// The SSH clone URL of the fork.
    pub ssh_url: String,
}

/// The body of `POST /api/v1/repos/{owner}/{repo}/forks`: both
/// fields optional, both omitted when unset (the server defaults
/// the name to the source repo and the target to the token owner).
#[derive(Debug, Serialize)]
struct ForkRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    organization: Option<String>,
}

/// The head commit of a branch: only the id (the sync-verification
/// primitive) and the message are modelled.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GiteaBranchCommit {
    /// The commit SHA.
    pub id: String,
    /// The commit message.
    #[serde(default)]
    pub message: String,
}

/// A Gitea branch: only the fields the operations need
/// are modelled — the name, the head commit, and whether the
/// branch is protected. A field the operation does not need is
/// not parsed.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GiteaBranch {
    /// The branch name.
    pub name: String,
    /// The head commit.
    pub commit: GiteaBranchCommit,
    /// Whether the branch is protected. Required, not
    /// defaulted: the server always emits it, and a response
    /// that omits it must fail the parse rather than read the
    /// branch as unprotected.
    pub protected: bool,
}

/// A Gitea pull request: only the fields the read operations need
/// are modelled — identity, title and body, state and merge flags,
/// author, labels, and the head/base endpoints. A field the
/// operation does not need is not parsed. Shaped like Gitea 1.24.7
/// answers (`GET /api/v1/repos/{owner}/{repo}/pulls/{n}` and the
/// list endpoint, which answers the same objects).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GiteaPullRequest {
    /// The pull request number, unique per repo.
    pub number: u64,
    /// The pull request title.
    pub title: String,
    /// The pull request body; `None` when the PR carries none.
    #[serde(default)]
    pub body: Option<String>,
    /// The wire state (`open`/`closed`).
    pub state: String,
    /// Whether the PR is a draft.
    #[serde(default)]
    pub draft: bool,
    /// Whether the PR is merged.
    #[serde(default)]
    pub merged: bool,
    /// Whether the PR is mergeable; `None` when the server has not
    /// computed it.
    #[serde(default)]
    pub mergeable: Option<bool>,
    /// The PR author.
    pub user: GiteaOwner,
    /// The label names on the PR.
    #[serde(default)]
    pub labels: Vec<GiteaLabel>,
    /// Where the change comes from.
    pub head: GiteaPullRequestRef,
    /// Where the change goes to.
    pub base: GiteaPullRequestRef,
}

/// One endpoint of a Gitea pull request: the repo, branch, and head
/// commit the change comes from or goes to.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GiteaPullRequestRef {
    /// The branch label (`owner:branch` on a fork, the branch name
    /// otherwise).
    pub label: String,
    /// The branch name.
    #[serde(rename = "ref")]
    pub branch: String,
    /// The head/base commit SHA.
    pub sha: String,
    /// The repo the branch lives in; `None` when the fork the
    /// branch lived in is deleted.
    #[serde(default)]
    pub repo: Option<GiteaPullRequestRepo>,
}

/// The repo of a pull-request endpoint: only its full name is
/// modelled.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GiteaPullRequestRepo {
    /// `owner/name`.
    pub full_name: String,
}

/// A Gitea label: only the name is modelled.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GiteaLabel {
    /// The label name.
    pub name: String,
}

/// A Gitea issue as the global pull-request search answers it
/// (`GET /api/v1/repos/issues/search?type=pulls`): the issue
/// fields plus the `pull_request` block carrying the merged flag
/// and the `repository` block carrying the repo. The head/base
/// endpoints are not in the search answer, so a search result
/// reads with empty endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GiteaIssue {
    /// The issue/PR number, unique per repo.
    pub number: u64,
    /// The issue title.
    pub title: String,
    /// The issue body; `None` when it carries none.
    #[serde(default)]
    pub body: Option<String>,
    /// The wire state (`open`/`closed`).
    pub state: String,
    /// The issue author.
    pub user: GiteaOwner,
    /// The label names on the issue.
    #[serde(default)]
    pub labels: Vec<GiteaLabel>,
    /// The pull-request block; `None` when the item is not a PR.
    #[serde(default)]
    pub pull_request: Option<GiteaIssuePullRequest>,
    /// The repo the issue lives in.
    pub repository: GiteaIssueRepository,
}

/// The `pull_request` block of a search item: only the merged
/// flag is modelled.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GiteaIssuePullRequest {
    /// Whether the PR is merged.
    #[serde(default)]
    pub merged: bool,
}

/// The `repository` block of a search item: only its full name is
/// modelled.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GiteaIssueRepository {
    /// `owner/name`.
    pub full_name: String,
}

/// A Gitea pull-request review: only the fields the read
/// operations need are modelled — the reviewer, the state, the
/// body, the dismissed flag, and when it was submitted. Shaped
/// like Gitea 1.24.7 answers (`GET
/// /api/v1/repos/{owner}/{repo}/pulls/{n}/reviews`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GiteaReview {
    /// The reviewer.
    pub user: GiteaOwner,
    /// The wire state (`PENDING`/`REQUEST_REVIEW`/`APPROVED`/
    /// `REQUEST_CHANGES`/`COMMENT`).
    pub state: String,
    /// The review body; `None` when it carries none.
    #[serde(default)]
    pub body: Option<String>,
    /// Whether a maintainer dismissed the review.
    #[serde(default)]
    pub dismissed: bool,
    /// When the review was submitted; `None` when the server does
    /// not record one.
    #[serde(default)]
    pub submitted_at: Option<String>,
}

/// The wire state filter for the pull-request list and search
/// endpoints (`open`/`closed`/`all`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GiteaPullRequestState {
    /// Open pull requests.
    Open,
    /// Closed pull requests (merged or closed unmerged).
    Closed,
    /// Every pull request, open or closed.
    All,
}

impl GiteaPullRequestState {
    /// The wire spelling of this state, as it appears in the
    /// `state` query parameter.
    pub fn as_str(&self) -> &'static str {
        match self {
            GiteaPullRequestState::Open => "open",
            GiteaPullRequestState::Closed => "closed",
            GiteaPullRequestState::All => "all",
        }
    }
}

/// A Gitea access level a caller grants: read, write, or admin.
/// The request-side permission — only these three are grantable.
/// The server reports two more (`owner`, `none`) that no grant
/// names, so they live on the response side
/// ([`GiteaAccessMode`]) only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GiteaAccess {
    Read,
    Write,
    Admin,
}

impl GiteaAccess {
    /// The wire string for this level, for dry-run plans and
    /// error messages.
    fn as_str(self) -> &'static str {
        match self {
            GiteaAccess::Read => "read",
            GiteaAccess::Write => "write",
            GiteaAccess::Admin => "admin",
        }
    }
}

/// The access mode the server reports for a collaborator or a
/// team: the three grantable levels, plus `owner` (the repo or
/// org owner) and `none` (no access), plus any mode a newer
/// server reports, kept verbatim instead of failing the parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GiteaAccessMode {
    None,
    Read,
    Write,
    Admin,
    Owner,
    Unknown(String),
}

impl<'de> Deserialize<'de> for GiteaAccessMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "none" => GiteaAccessMode::None,
            "read" => GiteaAccessMode::Read,
            "write" => GiteaAccessMode::Write,
            "admin" => GiteaAccessMode::Admin,
            "owner" => GiteaAccessMode::Owner,
            other => GiteaAccessMode::Unknown(other.to_string()),
        })
    }
}

impl GiteaAccessMode {
    /// Whether this reported mode grants at least the requested
    /// level: the modes order none < read < write < admin <
    /// owner, and a mode a newer server reports cannot be
    /// ordered, so it grants nothing.
    pub fn grants_at_least(&self, level: GiteaAccess) -> bool {
        let rank = match self {
            GiteaAccessMode::None => 0,
            GiteaAccessMode::Read => 1,
            GiteaAccessMode::Write => 2,
            GiteaAccessMode::Admin => 3,
            GiteaAccessMode::Owner => 4,
            GiteaAccessMode::Unknown(_) => -1,
        };
        let want = match level {
            GiteaAccess::Read => 1,
            GiteaAccess::Write => 2,
            GiteaAccess::Admin => 3,
        };
        rank >= want
    }
}

impl From<GiteaAccess> for GiteaAccessMode {
    fn from(level: GiteaAccess) -> Self {
        match level {
            GiteaAccess::Read => GiteaAccessMode::Read,
            GiteaAccess::Write => GiteaAccessMode::Write,
            GiteaAccess::Admin => GiteaAccessMode::Admin,
        }
    }
}

/// A collaborator's permission on a repository, as the server
/// reports it: the access mode, the role name, and the user.
/// Only the fields the permission read needs are modelled.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GiteaCollaboratorPermission {
    /// The access mode the user holds on the repository.
    pub permission: GiteaAccessMode,
    /// The role name the server reports for the permission.
    #[serde(default)]
    pub role_name: String,
    /// The user the permission belongs to.
    pub user: GiteaOwner,
}

/// The body of `PUT /api/v1/repos/{owner}/{repo}/collaborators/{user}`:
/// the access level to grant.
#[derive(Debug, Serialize)]
struct AddCollaboratorRequest {
    permission: GiteaAccess,
}

/// A Gitea organization, as a team response names it: only the
/// name is modelled.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GiteaTeamOrg {
    /// The organization's name.
    pub name: String,
}

/// A Gitea org team: only the fields the team operations need
/// are modelled — identity, permission, units, and flags. A
/// field the operation does not need is not parsed.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GiteaTeam {
    /// The team's numeric id: the address of every member and
    /// repo operation on the team.
    pub id: i64,
    /// The team's name.
    pub name: String,
    /// The team's description.
    #[serde(default)]
    pub description: String,
    /// The organization the team belongs to, when the server
    /// names it: the team list leaves it null, while create and
    /// get return it.
    #[serde(default)]
    pub organization: Option<GiteaTeamOrg>,
    /// The access level the team grants on its repositories, as
    /// the server reports it. The server reports `none` here for
    /// read and write teams; the grant lives per unit in
    /// `units_map`. Only admin and owner teams report their
    /// level here.
    pub permission: GiteaAccessMode,
    /// The repo units the team can access, when the server
    /// names them.
    #[serde(default)]
    pub units: Vec<String>,
    /// The access level per repo unit, as the server reports it:
    /// the grant a read or write team actually holds, since the
    /// top-level `permission` reports `none` for those teams.
    /// Empty when the server names no per-unit grant.
    #[serde(default)]
    pub units_map: HashMap<String, GiteaAccessMode>,
    /// Whether the team can access every repository of the org.
    #[serde(default)]
    pub includes_all_repositories: bool,
    /// Whether the team can create repositories in the org.
    #[serde(default)]
    pub can_create_org_repo: bool,
}

/// The repo units a team can access, as the server names them:
/// the full set, sent when `team_create` is not told otherwise.
/// A create with no units is refused by the server (500) for a
/// non-admin team — an admin team is created with all units —
/// so an unset units list means all of them, not none.
const GITEA_TEAM_ALL_UNITS: [&str; 10] = [
    "repo.code",
    "repo.issues",
    "repo.ext_issues",
    "repo.pulls",
    "repo.releases",
    "repo.wiki",
    "repo.ext_wiki",
    "repo.projects",
    "repo.packages",
    "repo.actions",
];

/// Options for `team_create`: the name is required; the rest
/// default to a read team over all repo units, no all-repo
/// access, and no repo creation.
#[derive(Debug, Clone, Default)]
pub struct TeamCreateOptions {
    /// A short description; omitted when unset.
    pub description: Option<String>,
    /// The access level the team grants; read when unset.
    pub permission: Option<GiteaAccess>,
    /// The repo units the team can access; all of them when unset — a create with no units is refused by the server for a non-admin team.
    pub units: Option<Vec<String>>,
    /// Whether the team can access every repository of the org.
    pub includes_all_repositories: bool,
    /// Whether the team can create repositories in the org.
    pub can_create_org_repo: bool,
}

/// The body of `POST /api/v1/orgs/{org}/teams`: the name, the
/// permission, the units, the per-unit grant, and the flags
/// always; the description only when set. The permission is
/// always sent: the server defaults an omitted permission to
/// `none`, not read. The units are always sent: the server
/// refuses a create with no units for a non-admin team. The
/// per-unit grant is sent alongside the units: `units` is
/// deprecated in favour of `units_map`.
#[derive(Debug, Serialize)]
struct TeamCreateRequest {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    permission: GiteaAccess,
    units: Vec<String>,
    units_map: BTreeMap<String, GiteaAccess>,
    includes_all_repositories: bool,
    can_create_org_repo: bool,
}
/// Options for `repo_create`: the repo is public and takes the
/// server's default branch unless told otherwise.
#[derive(Debug, Clone, Default)]
pub struct RepoCreateOptions {
    /// Create a private repository; public by default.
    pub private: bool,
    /// A short description; omitted when unset.
    pub description: Option<String>,
    /// The default branch; the server default when unset.
    pub default_branch: Option<String>,
}

/// The body of `POST /api/v1/user/repos`: the name and the
/// privacy flag always; the description and default branch only
/// when set.
#[derive(Debug, Serialize)]
struct RepoCreateRequest {
    name: String,
    private: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_branch: Option<String>,
}

/// The page size for Gitea list calls (`limit=50`, like osc's
/// fork list), and the page bound: past it the call fails instead
/// of reading unboundedly or silently truncating.
const LIST_PAGE_SIZE: u32 = 50;
const MAX_LIST_PAGES: u32 = 10;

/// A parsed scmsync URL, following the server's
/// `generate_scmsyncinfo` (backend `BSSrcServer/ScmsyncDB.pm`):
/// the repo is scheme + host + path with a trailing `.git`
/// stripped and a `git+` scheme prefix stripped; the branch is the
/// `#fragment`; the `trackingbranch` query value is a separate
/// field, honoured as the branch only when no fragment names one.
struct ScmsyncUrl {
    /// The authority (host, plus port when present) of the repo URL.
    authority: String,
    /// The repo's owner.
    owner: String,
    /// The repo name.
    repo: String,
    /// The branch: the `#fragment` when present, else the
    /// `trackingbranch` query value when present, else `None`.
    branch: Option<String>,
}

/// The Gitea client: the transport client for the git-backed
/// implementation of the uniform flow. Parallel to `ObsClient` —
/// a different base URL, different auth (Bearer vs OBS Basic), and
/// a different wire format (JSON, not XML).
pub struct GiteaClient<'a> {
    http: &'a dyn HttpClient,
    base: String,
    credentials: Credentials,
    debug: Debug,
}

impl<'a> GiteaClient<'a> {
    /// Build a client for the configured Gitea instance
    /// (`DEFAULT_GITEA_URL`) with the given Bearer credentials.
    /// Construction fails closed when no token is configured: the
    /// Gitea API takes a Bearer token and no other HTTP auth.
    pub fn new(http: &'a dyn HttpClient, credentials: Credentials) -> Result<Self, GeectlError> {
        Self::with_base(http, DEFAULT_GITEA_URL, credentials)
    }

    /// Build a client for `base` with the given Bearer
    /// credentials. The base is a bare `https://` URL (scheme +
    /// host, no path, query, fragment, or userinfo), or a bare
    /// `http://` URL on a loopback host (a test instance only).
    pub fn with_base(
        http: &'a dyn HttpClient,
        base: &str,
        credentials: Credentials,
    ) -> Result<Self, GeectlError> {
        validate_gitea_base(base)?;
        let credentials = match credentials {
            Credentials::Bearer { .. } => credentials,
            Credentials::None => {
                return Err(GeectlError::new(
                    ErrorKind::ConfigError,
                    "no Gitea token configured (GEECTL_GITEA_TOKEN or GEECTL_GITEA_TOKEN_FILE)",
                ));
            }
            Credentials::Basic { .. } | Credentials::SshKey { .. } => {
                return Err(GeectlError::new(
                    ErrorKind::ConfigError,
                    "Gitea takes a Bearer token (GEECTL_GITEA_TOKEN or GEECTL_GITEA_TOKEN_FILE), not a username/password or SSH key",
                ));
            }
        };
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            credentials,
            debug: Debug::disabled(),
        })
    }

    /// Build a client for `base` with the credentials resolved for
    /// that URL: the matching alias's variables from the config
    /// file's `[instances]` table, the plain variables for the
    /// configured default URL, and a clear error otherwise (see
    /// [`resolve_gitea_bearer_for_url`]). `default_url` is the
    /// effective configured Gitea URL — environment, then config
    /// file, then default — never the bare [`DEFAULT_GITEA_URL`]
    /// constant when a custom URL is configured, or a custom
    /// single URL with a plain token would fail resolution. An
    /// unauthenticated client cannot be built: when nothing is
    /// configured for this URL the error names what to set — the
    /// alias variables when an alias matches (plus the plain
    /// variables when the URL is also the default), the
    /// `[instances]` entry when none does, the plain variables
    /// for the default URL — instead of `with_base`'s plain-vars
    /// message. The base is validated first, exactly as
    /// [`with_base`] validates it, and the fail-closed
    /// construction of [`with_base`] applies unchanged.
    pub fn with_base_resolved(
        http: &'a dyn HttpClient,
        base: &str,
        default_url: &str,
        instances: &[(String, String)],
        vars: VarLookup<'_>,
        read_file: FileReader<'_>,
    ) -> Result<Self, GeectlError> {
        // Validate before resolving: an invalid base gets the
        // base-validation error, not a credential error that
        // misdirects the user to `[instances]` — and no secret
        // file is read for a base the client would reject.
        validate_gitea_base(base)?;
        let credentials =
            resolve_gitea_bearer_for_url(base, default_url, instances, vars, read_file)?;
        if matches!(credentials, Credentials::None) {
            // `with_base` rejects an unauthenticated client, and
            // its error names the plain variables — the wrong
            // guidance when the URL is an alias or an unknown
            // instance, so name what to set here instead. The
            // default URL falls through to `with_base`, whose
            // plain-vars message is the right one there.
            if let Some(alias) = alias_for_url(base, instances)? {
                let (token_var, token_file_var) = gitea_alias_vars(alias);
                // The URL can also be the configured default:
                // then the plain variables would fix it too, so
                // name both sets.
                let default_suffix = if same_api_url(base, default_url)? {
                    format!(
                        " — or the plain {GITEA_TOKEN_VAR} (or {GITEA_TOKEN_FILE_VAR}), since this URL is also the configured default"
                    )
                } else {
                    String::new()
                };
                return Err(GeectlError::new(
                    ErrorKind::ConfigError,
                    format!(
                        "no credentials configured for {base}; instance alias '{alias}' matches this URL — set {token_var} (or {token_file_var}){default_suffix}"
                    ),
                ));
            }
            if !same_api_url(base, default_url)? {
                return Err(GeectlError::new(
                    ErrorKind::ConfigError,
                    format!(
                        "no credentials configured for {base}; no instance alias matches it — add an [instances] entry for it in the config file and set GEECTL_GITEA_TOKEN_<ALIAS> (or GEECTL_GITEA_TOKEN_FILE_<ALIAS>) for that alias"
                    ),
                ));
            }
        }
        Self::with_base(http, base, credentials)
    }

    /// Report the channel's decision lines to `debug` instead of
    /// dropping them. The sink is off by default; the caller that
    /// owns the debug channel passes its sink here.
    pub fn with_debug(mut self, debug: Debug) -> Self {
        self.debug = debug;
        self
    }

    /// The authority (host, plus port when present) of the
    /// configured base, for the single-instance host check.
    fn authority(&self) -> &str {
        self.base
            .strip_prefix("https://")
            .or_else(|| self.base.strip_prefix("http://"))
            .unwrap_or(&self.base)
    }

    /// Read a repository (`GET /api/v1/repos/{owner}/{repo}`).
    /// The read the other operations build on: it resolves the
    /// default branch and verifies a fork's parent by re-reading.
    pub fn repo_get(&self, owner: &str, repo: &str) -> Result<GiteaRepository, GeectlError> {
        validate_gitea_name(owner, "owner")?;
        validate_gitea_name(repo, "repo")?;
        let url = build_url(&self.base, &["api", "v1", "repos", owner, repo], &[]);
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_repository(&body, owner, repo)
    }

    /// Read the token owner's login (`GET /api/v1/user`). A private
    /// helper: the fork target owner defaults to this login.
    fn current_user_login(&self) -> Result<String, GeectlError> {
        let url = build_url(&self.base, &["api", "v1", "user"], &[]);
        let body = self.http.get_with(&url, &self.credentials)?;
        let user: GiteaUser = serde_json::from_str(&body).map_err(|err| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected Gitea user response: {err}"),
            )
        })?;
        Ok(user.login)
    }

    /// Fork a repository (`POST
    /// /api/v1/repos/{owner}/{repo}/forks`): the Gitea-side step of
    /// the `branch` composite. `ForkExists` returns the existing
    /// fork instead of failing — the agent reuses it. A non-fork
    /// repo of the requested name is an error naming the
    /// alternative (new name, or delete and retry). Fork creation
    /// answers 202 and the created fork is NOT verified by the
    /// response alone: the fork is re-read and its `parent`
    /// checked before success is reported, so a fork-of-fork with
    /// a mismatched parent is a `ServerError` naming the re-read.
    /// In dry-run the fork is recorded in the plan and nothing is
    /// forked — the guard runs before any server read, like
    /// `branch_package` — and the result is `None`.
    pub fn fork_repo(
        &self,
        owner: &str,
        repo: &str,
        options: &ForkRepoOptions,
        dry_run: &DryRun,
    ) -> Result<Option<GiteaRepository>, GeectlError> {
        validate_gitea_name(owner, "owner")?;
        validate_gitea_name(repo, "repo")?;
        if let Some(name) = &options.new_name {
            validate_gitea_name(name, "fork name")?;
        }
        if let Some(organization) = &options.organization {
            validate_gitea_name(organization, "organization")?;
        }
        let target_name = options.new_name.as_deref().unwrap_or(repo);
        // The dry-run guard: in dry-run the fork is recorded in the
        // plan and nothing is forked — not even the target-owner
        // read runs. The plan names the target owner only when the
        // caller gave an organization; otherwise it says the
        // caller's own account instead of guessing it.
        let target_owner = options.organization.as_deref().unwrap_or("your account");
        if !dry_run.allow(&format!(
            "fork repository {owner}/{repo} to {target_owner}/{target_name}"
        )) {
            return Ok(None);
        }
        let target_owner = match &options.organization {
            Some(organization) => organization.clone(),
            None => self.current_user_login()?,
        };
        // Pre-check: a repo of the target name may already exist.
        // The reuse-or-refuse rule decides: a fork of this source
        // is reused, anything else is `AlreadyExists`.
        match self.repo_get(&target_owner, target_name) {
            Ok(existing) => return self.reuse_or_refuse(&existing, owner, repo),
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        let request = ForkRequest {
            name: options.new_name.clone(),
            organization: options.organization.clone(),
        };
        let body = serde_json::to_string(&request).map_err(|err| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot serialize the Gitea fork request: {err}"),
            )
        })?;
        let url = build_url(
            &self.base,
            &["api", "v1", "repos", owner, repo, "forks"],
            &[],
        );
        match self.http.post_json_with(&url, &body, &self.credentials) {
            Ok(_) => {}
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                // A 409 from the fork: a fork landed first (or the
                // target name was taken) between the pre-check and
                // the POST. Only a 409 — AlreadyExists — re-reads
                // the target; any other refusal (a 400, a 422)
                // propagates as-is, so a malformed request is never
                // mistaken for a race. Re-read the target and apply
                // the same reuse-or-refuse rule; when the target
                // still does not exist the fork lives under a
                // different name, so the original refusal stands.
                return match self.repo_get(&target_owner, target_name) {
                    Ok(existing) => self.reuse_or_refuse(&existing, owner, repo),
                    Err(re_err) if re_err.kind() == ErrorKind::NotFound => Err(err),
                    Err(re_err) => Err(re_err),
                };
            }
            Err(err) => return Err(err),
        };
        // The 202 answer is not the verification: re-read the
        // created fork by its known name and check its parent
        // before reporting success. The reported repo is the
        // re-read one — server state after creation, not the 202
        // body, which is never parsed (a malformed 202 must not
        // fail a fork the re-read proves exists).
        let forked = self.verify_fork_parent(&target_owner, target_name, owner, repo)?;
        Ok(Some(forked))
    }

    /// The ForkExists rule: an existing target repo that is a fork
    /// of the source is reused (a decision line records it);
    /// any other repo of the target name is an `AlreadyExists`
    /// naming the alternative.
    fn reuse_or_refuse(
        &self,
        existing: &GiteaRepository,
        source_owner: &str,
        source_repo: &str,
    ) -> Result<Option<GiteaRepository>, GeectlError> {
        let source = format!("{source_owner}/{source_repo}");
        match &existing.parent {
            Some(parent) if parent.full_name.eq_ignore_ascii_case(&source) => {
                self.debug.emit(&format!(
                    "decision: reusing existing fork {} for {source}",
                    existing.full_name
                ));
                Ok(Some(existing.clone()))
            }
            _ => Err(GeectlError::new(
                ErrorKind::AlreadyExists,
                format!(
                    "a repository named {} already exists and is not a fork of {source}; fork to a new name, or delete it and retry",
                    existing.full_name
                ),
            )),
        }
    }

    /// Verify a fork by re-reading it by its known name and return
    /// the re-read repo: the parent must be the source repo. A
    /// fork-of-fork whose parent is not the source, or a fork that
    /// is not yet readable, is a `ServerError` naming the re-read
    /// — the fork was just created, so a re-read failure is the
    /// server's fault, not a missing resource.
    fn verify_fork_parent(
        &self,
        target_owner: &str,
        target_name: &str,
        source_owner: &str,
        source_repo: &str,
    ) -> Result<GiteaRepository, GeectlError> {
        let source = format!("{source_owner}/{source_repo}");
        let target = format!("{target_owner}/{target_name}");
        let re_read = match self.repo_get(target_owner, target_name) {
            Ok(re_read) => re_read,
            Err(err) => {
                return Err(GeectlError::new(
                    ErrorKind::ServerError,
                    format!("re-read of the fork {target} after creation failed: {err}"),
                ));
            }
        };
        match &re_read.parent {
            Some(parent) if parent.full_name.eq_ignore_ascii_case(&source) => Ok(re_read),
            _ => Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "re-read of the fork {} after creation does not name {source} as its parent",
                    re_read.full_name
                ),
            )),
        }
    }

    /// Delete a repository (`DELETE /api/v1/repos/{owner}/{repo}`)
    /// and return the deleted name. DryRun-guarded like
    /// `delete_package`: the guard runs before the read, and the
    /// result is `None` in dry-run. The repo is GET-read first to
    /// confirm owner/name — the read is a guard, not just the return
    /// value: `parse_repository` refuses a response naming another
    /// repo, so a delete never runs against the wrong generation.
    /// `force` is the confirmation: without it the delete is
    /// refused client-side (`InvalidInput` naming force) before any
    /// server read, like `delete_package` refusing without force
    /// while open requests exist. A dry-run plan needs no force.
    pub fn repo_delete(
        &self,
        owner: &str,
        repo: &str,
        force: bool,
        dry_run: &DryRun,
    ) -> Result<Option<String>, GeectlError> {
        validate_gitea_name(owner, "owner")?;
        validate_gitea_name(repo, "repo")?;
        if !dry_run.allow(&format!("delete repository {owner}/{repo}")) {
            return Ok(None);
        }
        if !force {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "refusing to delete repository {owner}/{repo} without force: deleting a repository is destructive; pass force to confirm"
                ),
            ));
        }
        let existing = self.repo_get(owner, repo)?;
        let url = build_url(&self.base, &["api", "v1", "repos", owner, repo], &[]);
        self.http.delete(&url, &self.credentials)?;
        Ok(Some(existing.name))
    }

    /// List repositories: the token owner's own repos when neither
    /// filter is given (`GET /api/v1/user/repos`), one user's repos
    /// (`GET /api/v1/users/{owner}/repos`), one organization's
    /// repos (`GET /api/v1/orgs/{org}/repos`) — or both, the user's
    /// repos first and then the organization's, the `repo list
    /// --user --org` form (osc
    /// `behave/features/git-repo-list.feature`). The agent use case is finding its forks
    /// or an org's repos before acting on one — including finding
    /// the fork of a source under a different name, which the
    /// fork's 409 body does not name. Each list paginates
    /// internally and is bounded (see `list_paged`): past the bound
    /// the call fails naming the list and what the caller can do
    /// about it, never a silent truncation.
    pub fn repo_list(
        &self,
        owner: Option<&str>,
        org: Option<&str>,
    ) -> Result<Vec<GiteaRepository>, GeectlError> {
        if let Some(owner) = owner {
            validate_gitea_name(owner, "owner")?;
        }
        if let Some(org) = org {
            validate_gitea_name(org, "organization")?;
        }
        let mut repos = Vec::new();
        // The bound error's remedy is per form: only the own form
        // can be narrowed (list one user or org instead); a single
        // user, a single org, and a repo's branches have no
        // narrower filter, so their remedy says the list is
        // bounded.
        match (owner, org) {
            (None, None) => repos.extend(self.list_paged::<GiteaRepository>(
                &["api", "v1", "user", "repos"],
                &[],
                "your repositories",
                "list a single user or organization instead",
            )?),
            (Some(owner), None) => repos.extend(self.list_paged::<GiteaRepository>(
                &["api", "v1", "users", owner, "repos"],
                &[],
                &format!("repositories of user {owner}"),
                "the list is bounded at 500 entries",
            )?),
            (None, Some(org)) => repos.extend(self.list_paged::<GiteaRepository>(
                &["api", "v1", "orgs", org, "repos"],
                &[],
                &format!("repositories of organization {org}"),
                "the list is bounded at 500 entries",
            )?),
            (Some(owner), Some(org)) => {
                // The user's repos first, then the org's — osc's
                // `repo list --user --org` order (osc
                // `behave/features/git-repo-list.feature`,
                // "List repos owned by a user and an organization":
                // the user's repo first, then the org's). An org
                // is a user in Gitea, so when `owner` and `org`
                // name the same account the two lists are the same
                // repos — dedupe by full name (case-insensitive,
                // the repo-identity rule) instead of returning
                // every repo twice.
                repos.extend(self.list_paged::<GiteaRepository>(
                    &["api", "v1", "users", owner, "repos"],
                    &[],
                    &format!("repositories of user {owner}"),
                    "the list is bounded at 500 entries",
                )?);
                repos.extend(self.list_paged::<GiteaRepository>(
                    &["api", "v1", "orgs", org, "repos"],
                    &[],
                    &format!("repositories of organization {org}"),
                    "the list is bounded at 500 entries",
                )?);
                // A set, not a linear scan: the two lists hold
                // up to 500 entries each, and the Vec form was
                // O(n^2) on the combined list. `insert` returns
                // false for a duplicate, which is the retain
                // predicate directly; order is unchanged.
                let mut seen: HashSet<String> = HashSet::new();
                repos.retain(|repo| seen.insert(repo.full_name.to_lowercase()));
            }
        }
        Ok(repos)
    }

    /// Create a repository (`POST /api/v1/user/repos`) and return
    /// the created repo. The fixture primitive the live tests use,
    /// and the repo the git-backed new-package flow's sources will
    /// live in. DryRun-guarded like `fork_repo`: the guard runs
    /// before any server read, and the result is `None` in
    /// dry-run. The response is checked against what was asked
    /// for — the name, the privacy flag, and the default branch
    /// when one was named: the owner is the token owner, unknown
    /// here without a read, so the name (not the full name) is the
    /// identity guard.
    pub fn repo_create(
        &self,
        name: &str,
        options: &RepoCreateOptions,
        dry_run: &DryRun,
    ) -> Result<Option<GiteaRepository>, GeectlError> {
        validate_gitea_name(name, "repo")?;
        if let Some(branch) = &options.default_branch {
            validate_git_branch_name(branch)?;
        }
        let private = if options.private { " (private)" } else { "" };
        if !dry_run.allow(&format!("create repository {name}{private}")) {
            return Ok(None);
        }
        let request = RepoCreateRequest {
            name: name.to_string(),
            private: options.private,
            description: options.description.clone(),
            default_branch: options.default_branch.clone(),
        };
        let body = serde_json::to_string(&request).map_err(|err| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot serialize the Gitea repo-create request: {err}"),
            )
        })?;
        let url = build_url(&self.base, &["api", "v1", "user", "repos"], &[]);
        let response = self.http.post_json_with(&url, &body, &self.credentials)?;
        let created =
            parse_repository_body(&response, &format!("for the created repository {name}"))?;
        // The same rule as the read path (`parse_repository`):
        // Gitea logins are case-insensitive, so the created name
        // is compared case-insensitively too. A server that
        // answers with the name in another case created the repo
        // that was asked for; a different name is still an error.
        if !created.name.eq_ignore_ascii_case(name) {
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected Gitea repository response for the created repository {name}: the server answered for {}",
                    created.full_name
                ),
            ));
        }
        // The create is verified against what was asked for: a
        // server that silently created a public repo, or one with
        // another default branch, must not report success — the
        // options are the caller's instruction, not a hint.
        if let Some(branch) = &options.default_branch {
            if created.default_branch != *branch {
                return Err(GeectlError::new(
                    ErrorKind::ServerError,
                    format!(
                        "unexpected Gitea repository response for the created repository {name}: the server answered with default branch {}, not {branch}",
                        created.default_branch
                    ),
                ));
            }
        }
        if created.private != options.private {
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected Gitea repository response for the created repository {name}: the server answered with private={}, not {}",
                    created.private, options.private
                ),
            ));
        }
        Ok(Some(created))
    }

    /// List a repository's collaborators (`GET
    /// /api/v1/repos/{owner}/{repo}/collaborators`): the users
    /// with access to the repo. Paginates internally and is
    /// bounded like `repo_list`. The entries carry logins only —
    /// the access level of each collaborator comes from
    /// `collaborator_permission`, not from this list.
    pub fn collaborator_list(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<GiteaOwner>, GeectlError> {
        validate_gitea_name(owner, "owner")?;
        validate_gitea_name(repo, "repo")?;
        self.list_paged::<GiteaOwner>(
            &["api", "v1", "repos", owner, repo, "collaborators"],
            &[],
            &format!("collaborators of {owner}/{repo}"),
            "the list is bounded at 500 entries",
        )
    }

    /// Read one collaborator's permission (`GET
    /// /api/v1/repos/{owner}/{repo}/collaborators/{user}/permission`):
    /// the access mode the user holds on the repo. This is the
    /// effective access, not membership: a user who is not a
    /// collaborator still reads — `read` on a public repository,
    /// `none` on a private one — so it cannot tell whether the
    /// user is a collaborator at all.
    pub fn collaborator_permission(
        &self,
        owner: &str,
        repo: &str,
        user: &str,
    ) -> Result<GiteaCollaboratorPermission, GeectlError> {
        validate_gitea_name(owner, "owner")?;
        validate_gitea_name(repo, "repo")?;
        validate_gitea_name(user, "collaborator")?;
        let url = build_url(
            &self.base,
            &[
                "api",
                "v1",
                "repos",
                owner,
                repo,
                "collaborators",
                user,
                "permission",
            ],
            &[],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_collaborator_permission(&body, owner, repo, user)
    }

    /// Whether a user is a collaborator on a repository (`GET
    /// /api/v1/repos/{owner}/{repo}/collaborators/{user}`): 204
    /// when they are, 404 when they are not. The permission read
    /// cannot answer this: it reports the effective access, which
    /// reads even for a non-collaborator.
    pub fn collaborator_is_collaborator(
        &self,
        owner: &str,
        repo: &str,
        user: &str,
    ) -> Result<bool, GeectlError> {
        validate_gitea_name(owner, "owner")?;
        validate_gitea_name(repo, "repo")?;
        validate_gitea_name(user, "collaborator")?;
        let url = build_url(
            &self.base,
            &["api", "v1", "repos", owner, repo, "collaborators", user],
            &[],
        );
        match self.http.get_with(&url, &self.credentials) {
            Ok(_) => Ok(true),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err),
        }
    }

    /// Add a collaborator (`PUT
    /// /api/v1/repos/{owner}/{repo}/collaborators/{user}`) with
    /// the given access level, and return the permission the
    /// server reports afterwards. The PUT answers 204 with no
    /// body, so the grant is verified by re-reading the
    /// permission — like the fork re-reads the fork — and a
    /// server that records less than asked for fails here instead
    /// of reporting success. Access is hierarchical: a user who
    /// already holds more (an org owner reads back `owner`)
    /// satisfies the grant, and the returned permission says so.
    /// DryRun-guarded like `repo_create`: the guard runs before
    /// any server read, and the result is `None` in dry-run.
    /// `force` is the confirmation for an admin grant: without
    /// it an admin grant is refused client-side (`InvalidInput`
    /// naming force) before any server read. A dry-run plan
    /// needs no force.
    pub fn collaborator_add(
        &self,
        owner: &str,
        repo: &str,
        user: &str,
        permission: GiteaAccess,
        force: bool,
        dry_run: &DryRun,
    ) -> Result<Option<GiteaCollaboratorPermission>, GeectlError> {
        validate_gitea_name(owner, "owner")?;
        validate_gitea_name(repo, "repo")?;
        validate_gitea_name(user, "collaborator")?;
        if !dry_run.allow(&format!(
            "add collaborator {user} to {owner}/{repo} with {} access",
            permission.as_str()
        )) {
            return Ok(None);
        }
        if permission == GiteaAccess::Admin && !force {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "refusing to add collaborator {user} to {owner}/{repo} with admin access without force: granting admin access is a privilege grant; pass force to confirm"
                ),
            ));
        }
        let request = AddCollaboratorRequest { permission };
        let body = serde_json::to_string(&request).map_err(|err| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot serialize the Gitea collaborator-add request: {err}"),
            )
        })?;
        let url = build_url(
            &self.base,
            &["api", "v1", "repos", owner, repo, "collaborators", user],
            &[],
        );
        self.http.put_json_with(&url, &body, &self.credentials)?;
        let granted = self.collaborator_permission(owner, repo, user)?;
        if !granted.permission.grants_at_least(permission) {
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected Gitea collaborator permission for {user} on {owner}/{repo}: the server granted {:?}, not {} access or more",
                    granted.permission,
                    permission.as_str()
                ),
            ));
        }
        Ok(Some(granted))
    }

    /// Remove a collaborator (`DELETE
    /// /api/v1/repos/{owner}/{repo}/collaborators/{user}`) and
    /// return the removed login. Membership is read first: a
    /// user who is not a collaborator answers 404 there, so the
    /// remove refuses with `NotFound` naming the user before any
    /// delete runs — like `repo_delete` reads the repo first. The
    /// permission read cannot be the pre-check: it reports the
    /// effective access, which reads even for a non-collaborator.
    /// `force` is the confirmation: without it the remove is
    /// refused client-side (`InvalidInput` naming force) before
    /// any server read. A dry-run plan needs no force.
    pub fn collaborator_remove(
        &self,
        owner: &str,
        repo: &str,
        user: &str,
        force: bool,
        dry_run: &DryRun,
    ) -> Result<Option<String>, GeectlError> {
        validate_gitea_name(owner, "owner")?;
        validate_gitea_name(repo, "repo")?;
        validate_gitea_name(user, "collaborator")?;
        if !dry_run.allow(&format!("remove collaborator {user} from {owner}/{repo}")) {
            return Ok(None);
        }
        if !force {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "refusing to remove collaborator {user} from {owner}/{repo} without force: removing access is destructive; pass force to confirm"
                ),
            ));
        }
        if !self.collaborator_is_collaborator(owner, repo, user)? {
            return Err(GeectlError::new(
                ErrorKind::NotFound,
                format!("collaborator {user} not found on {owner}/{repo}"),
            ));
        }
        let url = build_url(
            &self.base,
            &["api", "v1", "repos", owner, repo, "collaborators", user],
            &[],
        );
        self.http.delete(&url, &self.credentials)?;
        Ok(Some(user.to_string()))
    }

    /// List an organization's teams (`GET
    /// /api/v1/orgs/{org}/teams`). Paginates internally and is
    /// bounded like `repo_list`.
    pub fn team_list(&self, org: &str) -> Result<Vec<GiteaTeam>, GeectlError> {
        validate_gitea_name(org, "organization")?;
        self.list_paged::<GiteaTeam>(
            &["api", "v1", "orgs", org, "teams"],
            &[],
            &format!("teams of organization {org}"),
            "the list is bounded at 500 entries",
        )
    }

    /// Read one team (`GET /api/v1/teams/{id}`).
    pub fn team_get(&self, id: i64) -> Result<GiteaTeam, GeectlError> {
        validate_team_id(id)?;
        let id_str = id.to_string();
        let url = build_url(&self.base, &["api", "v1", "teams", &id_str], &[]);
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_team(&body, id)
    }

    /// Create an org team (`POST /api/v1/orgs/{org}/teams`) and
    /// return the created team. DryRun-guarded like
    /// `repo_create`: the guard runs before any server read, and
    /// the result is `None` in dry-run. `force` is the
    /// confirmation for a privileged team: without it a create
    /// granting admin permission, access to all repositories,
    /// or repository creation is refused client-side
    /// (`InvalidInput` naming force) before any server read. A
    /// dry-run plan needs no force. The response is checked
    /// against what was asked for — the name, the organization
    /// when the server names it, the permission per unit in
    /// `units_map` (read when none was named), the units, and
    /// both flags: a server that silently created a different
    /// team must not report success.
    pub fn team_create(
        &self,
        org: &str,
        name: &str,
        options: &TeamCreateOptions,
        force: bool,
        dry_run: &DryRun,
    ) -> Result<Option<GiteaTeam>, GeectlError> {
        validate_gitea_name(org, "organization")?;
        validate_gitea_team_name(name)?;
        if let Some(description) = &options.description {
            if description.chars().count() > 255 {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "invalid Gitea team description: over 255 characters".to_string(),
                ));
            }
        }
        if let Some(units) = &options.units {
            for unit in units {
                if !GITEA_TEAM_ALL_UNITS.contains(&unit.as_str()) {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!("invalid Gitea team unit: {unit:?}"),
                    ));
                }
            }
        }
        if !dry_run.allow(&format!("create team {name} in organization {org}")) {
            return Ok(None);
        }
        let want_permission = options.permission.unwrap_or(GiteaAccess::Read);
        if !force
            && (want_permission == GiteaAccess::Admin
                || options.includes_all_repositories
                || options.can_create_org_repo)
        {
            let grant = if want_permission == GiteaAccess::Admin {
                "admin permission"
            } else if options.includes_all_repositories {
                "access to all repositories"
            } else {
                "repository creation"
            };
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "refusing to create team {name} in organization {org} without force: creating a team with {grant} is a privilege grant; pass force to confirm"
                ),
            ));
        }
        // Duplicates are dropped and an empty list means unset:
        // the server dedupes the units it grants, so the request
        // and the verification must agree on the deduped set.
        let want_units: Vec<String> = match &options.units {
            Some(units) if !units.is_empty() => {
                let mut seen = HashSet::new();
                units
                    .iter()
                    .filter(|u| seen.insert(u.as_str()))
                    .cloned()
                    .collect()
            }
            _ => GITEA_TEAM_ALL_UNITS.iter().map(|u| u.to_string()).collect(),
        };
        let request = TeamCreateRequest {
            name: name.to_string(),
            description: options.description.clone(),
            permission: want_permission,
            units: want_units.clone(),
            units_map: want_units
                .iter()
                .map(|u| (u.clone(), want_permission))
                .collect(),
            includes_all_repositories: options.includes_all_repositories,
            can_create_org_repo: options.can_create_org_repo,
        };
        let body = serde_json::to_string(&request).map_err(|err| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot serialize the Gitea team-create request: {err}"),
            )
        })?;
        let url = build_url(&self.base, &["api", "v1", "orgs", org, "teams"], &[]);
        let response = self.http.post_json_with(&url, &body, &self.credentials)?;
        let created = parse_team_body(&response, &format!("for the created team {name}"))?;
        if created.name != name {
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected Gitea team response for the created team {name}: the server answered for {}",
                    created.name
                ),
            ));
        }
        // The list endpoint leaves the organization null, so the
        // check runs only when the server names it.
        if let Some(org_name) = &created.organization {
            if !org_name.name.eq_ignore_ascii_case(org) {
                return Err(GeectlError::new(
                    ErrorKind::ServerError,
                    format!(
                        "unexpected Gitea team response for the created team {name}: the server answered for organization {}",
                        org_name.name
                    ),
                ));
            }
        }
        // An admin team is granted all units whatever subset was
        // asked for, so the verification expects the full set
        // for an admin team and the asked set otherwise.
        let verify_units: Vec<String> = if want_permission == GiteaAccess::Admin {
            GITEA_TEAM_ALL_UNITS.iter().map(|u| u.to_string()).collect()
        } else {
            want_units.clone()
        };
        // The server reports `none` as the team permission for
        // read and write teams — the grant lives per unit in
        // `units_map` — so the permission is verified there, not
        // on the top-level field. An admin team reports `admin`
        // on the top level too, and that is checked.
        if want_permission == GiteaAccess::Admin && created.permission != GiteaAccessMode::Admin {
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected Gitea team response for the created team {name}: the server answered with {:?} permission, not admin",
                    created.permission
                ),
            ));
        }
        let want_mode = GiteaAccessMode::from(want_permission);
        for unit in &verify_units {
            match created.units_map.get(unit) {
                Some(mode) if *mode == want_mode => {}
                _ => {
                    return Err(GeectlError::new(
                        ErrorKind::ServerError,
                        format!(
                            "unexpected Gitea team response for the created team {name}: the server answered with {:?} for unit {unit}, not {want_mode:?}",
                            created.units_map.get(unit)
                        ),
                    ));
                }
            }
        }
        // The server reorders the units list, so the set is
        // compared sorted, not in order.
        let mut got_units = created.units.clone();
        got_units.sort();
        let mut want_units_sorted = verify_units.clone();
        want_units_sorted.sort();
        if got_units != want_units_sorted {
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected Gitea team response for the created team {name}: the server answered with units {got_units:?}, not {want_units_sorted:?}"
                ),
            ));
        }
        if created.includes_all_repositories != options.includes_all_repositories {
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected Gitea team response for the created team {name}: the server answered with includes_all_repositories={}, not {}",
                    created.includes_all_repositories, options.includes_all_repositories
                ),
            ));
        }
        if created.can_create_org_repo != options.can_create_org_repo {
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected Gitea team response for the created team {name}: the server answered with can_create_org_repo={}, not {}",
                    created.can_create_org_repo, options.can_create_org_repo
                ),
            ));
        }
        Ok(Some(created))
    }

    /// Delete an org team (`DELETE /api/v1/teams/{id}`) and
    /// return the deleted team's name. The team is read first:
    /// a team that does not exist answers 404 there, so the
    /// delete refuses with `NotFound` naming the id before any
    /// delete runs — like `repo_delete` reads the repo first.
    /// `force` is the confirmation: without it the delete is
    /// refused client-side (`InvalidInput` naming force) before
    /// any server read. A dry-run plan needs no force.
    pub fn team_delete(
        &self,
        id: i64,
        force: bool,
        dry_run: &DryRun,
    ) -> Result<Option<String>, GeectlError> {
        validate_team_id(id)?;
        if !dry_run.allow(&format!("delete team {id}")) {
            return Ok(None);
        }
        if !force {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "refusing to delete team {id} without force: deleting a team is destructive; pass force to confirm"
                ),
            ));
        }
        let existing = self.team_get(id)?;
        let id_str = id.to_string();
        let url = build_url(&self.base, &["api", "v1", "teams", &id_str], &[]);
        self.http.delete(&url, &self.credentials)?;
        Ok(Some(existing.name))
    }

    /// List a team's members (`GET /api/v1/teams/{id}/members`).
    /// Paginates internally and is bounded like `repo_list`.
    pub fn team_list_members(&self, id: i64) -> Result<Vec<GiteaOwner>, GeectlError> {
        validate_team_id(id)?;
        let id_str = id.to_string();
        self.list_paged::<GiteaOwner>(
            &["api", "v1", "teams", &id_str, "members"],
            &[],
            &format!("members of team {id}"),
            "the list is bounded at 500 entries",
        )
    }

    /// Read one team member (`GET
    /// /api/v1/teams/{id}/members/{user}`): the member's login.
    /// A user who is not a member answers 404, which reads as
    /// `NotFound` naming the user — the same read the remove
    /// pre-check and the add verification use.
    pub fn team_member_get(&self, id: i64, user: &str) -> Result<GiteaOwner, GeectlError> {
        validate_team_id(id)?;
        validate_gitea_name(user, "member")?;
        let id_str = id.to_string();
        let url = build_url(
            &self.base,
            &["api", "v1", "teams", &id_str, "members", user],
            &[],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_team_member(&body, id, user)
    }

    /// Add a member to a team (`PUT
    /// /api/v1/teams/{id}/members/{user}`) and return the member
    /// read back. The PUT answers 204 with no body, so the add
    /// is verified by re-reading the member — like the
    /// collaborator add re-reads the permission. A server that
    /// answers the add without recording it fails here instead
    /// of reporting success. DryRun-guarded like `repo_create`:
    /// the guard runs before any server read, and the result is
    /// `None` in dry-run. `force` is the confirmation: without
    /// it the add is refused client-side (`InvalidInput` naming
    /// force) before any server read — the team's level is not
    /// known without a read, so every add confirms. A dry-run
    /// plan needs no force.
    pub fn team_add_member(
        &self,
        id: i64,
        user: &str,
        force: bool,
        dry_run: &DryRun,
    ) -> Result<Option<GiteaOwner>, GeectlError> {
        validate_team_id(id)?;
        validate_gitea_name(user, "member")?;
        if !dry_run.allow(&format!("add member {user} to team {id}")) {
            return Ok(None);
        }
        if !force {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "refusing to add member {user} to team {id} without force: adding a member grants the team's access; pass force to confirm"
                ),
            ));
        }
        let id_str = id.to_string();
        let url = build_url(
            &self.base,
            &["api", "v1", "teams", &id_str, "members", user],
            &[],
        );
        self.http.put_json_with(&url, "{}", &self.credentials)?;
        Ok(Some(self.team_member_get(id, user)?))
    }

    /// Remove a member from a team (`DELETE
    /// /api/v1/teams/{id}/members/{user}`) and return the removed
    /// login. The member is read first: a user who is not a
    /// member answers 404 there, so the remove refuses with
    /// `NotFound` naming the user before any delete runs.
    /// `force` is the confirmation: without it the remove is
    /// refused client-side (`InvalidInput` naming force) before
    /// any server read. A dry-run plan needs no force.
    pub fn team_remove_member(
        &self,
        id: i64,
        user: &str,
        force: bool,
        dry_run: &DryRun,
    ) -> Result<Option<String>, GeectlError> {
        validate_team_id(id)?;
        validate_gitea_name(user, "member")?;
        if !dry_run.allow(&format!("remove member {user} from team {id}")) {
            return Ok(None);
        }
        if !force {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "refusing to remove member {user} from team {id} without force: removing a member is destructive; pass force to confirm"
                ),
            ));
        }
        let existing = self.team_member_get(id, user)?;
        let id_str = id.to_string();
        let url = build_url(
            &self.base,
            &["api", "v1", "teams", &id_str, "members", user],
            &[],
        );
        self.http.delete(&url, &self.credentials)?;
        Ok(Some(existing.login))
    }

    /// List a team's repositories (`GET /api/v1/teams/{id}/repos`).
    /// Paginates internally and is bounded like `repo_list`.
    pub fn team_list_repos(&self, id: i64) -> Result<Vec<GiteaRepository>, GeectlError> {
        validate_team_id(id)?;
        let id_str = id.to_string();
        self.list_paged::<GiteaRepository>(
            &["api", "v1", "teams", &id_str, "repos"],
            &[],
            &format!("repositories of team {id}"),
            "the list is bounded at 500 entries",
        )
    }

    /// Read one team repository (`GET
    /// /api/v1/teams/{id}/repos/{org}/{repo}`): the repo entry.
    /// A repo the team cannot access answers 404, which reads as
    /// `NotFound` naming the repo — the same read the remove
    /// pre-check and the add verification use.
    pub fn team_repo_get(
        &self,
        id: i64,
        org: &str,
        repo: &str,
    ) -> Result<GiteaRepository, GeectlError> {
        validate_team_id(id)?;
        validate_gitea_name(org, "organization")?;
        validate_gitea_name(repo, "repo")?;
        let id_str = id.to_string();
        let url = build_url(
            &self.base,
            &["api", "v1", "teams", &id_str, "repos", org, repo],
            &[],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_repository(&body, org, repo)
    }

    /// Add a repository to a team (`PUT
    /// /api/v1/teams/{id}/repos/{org}/{repo}`) and return the repo
    /// entry read back. The PUT answers 204 with no body, so the
    /// add is verified by re-reading the repo entry. DryRun-guarded
    /// like `repo_create`: the guard runs before any server read,
    /// and the result is `None` in dry-run. `force` is the
    /// confirmation: without it the add is refused client-side
    /// (`InvalidInput` naming force) before any server read —
    /// the team's level is not known without a read, so every
    /// add confirms. A dry-run plan needs no force.
    pub fn team_add_repo(
        &self,
        id: i64,
        org: &str,
        repo: &str,
        force: bool,
        dry_run: &DryRun,
    ) -> Result<Option<GiteaRepository>, GeectlError> {
        validate_team_id(id)?;
        validate_gitea_name(org, "organization")?;
        validate_gitea_name(repo, "repo")?;
        if !dry_run.allow(&format!("add repository {org}/{repo} to team {id}")) {
            return Ok(None);
        }
        if !force {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "refusing to add repository {org}/{repo} to team {id} without force: adding a repository grants the team's access; pass force to confirm"
                ),
            ));
        }
        let id_str = id.to_string();
        let url = build_url(
            &self.base,
            &["api", "v1", "teams", &id_str, "repos", org, repo],
            &[],
        );
        self.http.put_json_with(&url, "{}", &self.credentials)?;
        Ok(Some(self.team_repo_get(id, org, repo)?))
    }

    /// Remove a repository from a team (`DELETE
    /// /api/v1/teams/{id}/repos/{org}/{repo}`) and return the
    /// removed repo's full name. The repo entry is read first: a
    /// repo the team cannot access answers 404 there, so the
    /// remove refuses with `NotFound` naming the repo before any
    /// delete runs. `force` is the confirmation: without it the
    /// remove is refused client-side (`InvalidInput` naming
    /// force) before any server read. A dry-run plan needs no
    /// force.
    pub fn team_remove_repo(
        &self,
        id: i64,
        org: &str,
        repo: &str,
        force: bool,
        dry_run: &DryRun,
    ) -> Result<Option<String>, GeectlError> {
        validate_team_id(id)?;
        validate_gitea_name(org, "organization")?;
        validate_gitea_name(repo, "repo")?;
        if !dry_run.allow(&format!("remove repository {org}/{repo} from team {id}")) {
            return Ok(None);
        }
        if !force {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "refusing to remove repository {org}/{repo} from team {id} without force: removing repo access is destructive; pass force to confirm"
                ),
            ));
        }
        let existing = self.team_repo_get(id, org, repo)?;
        let id_str = id.to_string();
        let url = build_url(
            &self.base,
            &["api", "v1", "teams", &id_str, "repos", org, repo],
            &[],
        );
        self.http.delete(&url, &self.credentials)?;
        Ok(Some(existing.full_name))
    }

    /// List a repository's branches (`GET
    /// /api/v1/repos/{owner}/{repo}/branches`): the names and head
    /// commits of what a fork carries. Paginates internally and is
    /// bounded like `repo_list`.
    pub fn branch_list(&self, owner: &str, repo: &str) -> Result<Vec<GiteaBranch>, GeectlError> {
        validate_gitea_name(owner, "owner")?;
        validate_gitea_name(repo, "repo")?;
        self.list_paged::<GiteaBranch>(
            &["api", "v1", "repos", owner, repo, "branches"],
            &[],
            &format!("branches of {owner}/{repo}"),
            "the list is bounded at 500 entries",
        )
    }

    /// Read one branch (`GET
    /// /api/v1/repos/{owner}/{repo}/branches/{branch}`): verify a
    /// branch exists before naming it in a scmsync URL. The branch
    /// is one encoded segment, so a slashed name (`feature/x`)
    /// travels as `feature%2Fx` and the server decodes it.
    pub fn branch_get(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<GiteaBranch, GeectlError> {
        validate_gitea_name(owner, "owner")?;
        validate_gitea_name(repo, "repo")?;
        validate_git_branch_name(branch)?;
        let url = build_url(
            &self.base,
            &["api", "v1", "repos", owner, repo, "branches", branch],
            &[],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_branch(&body, owner, repo, branch)
    }

    /// The head commit SHA of a branch: the sync-verification
    /// primitive — the SHA the filer compares against the OBS side
    /// (`_scmsync.obsinfo`) before filing from a git-backed
    /// package. A branch read; only the commit id is returned.
    pub fn branch_head(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<String, GeectlError> {
        Ok(self.branch_get(owner, repo, branch)?.commit.id)
    }

    /// Read one pull request (`GET
    /// /api/v1/repos/{owner}/{repo}/pulls/{number}`): the title and
    /// body, state and merge flags, author, labels, and the
    /// head/base endpoints. The number is verified against what
    /// was asked for: a server that answers for another PR is a
    /// server error, like `repo_get`'s full-name check.
    pub fn pr_get(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<GiteaPullRequest, GeectlError> {
        validate_gitea_name(owner, "owner")?;
        validate_gitea_name(repo, "repo")?;
        validate_pr_number(number)?;
        let url = build_url(
            &self.base,
            &[
                "api",
                "v1",
                "repos",
                owner,
                repo,
                "pulls",
                &number.to_string(),
            ],
            &[],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_pull_request(&body, owner, repo, number)
    }

    /// List a repository's pull requests (`GET
    /// /api/v1/repos/{owner}/{repo}/pulls?state=…`): the PRs in
    /// the given wire state. Paginates internally and is bounded
    /// like `repo_list`.
    pub fn pr_list(
        &self,
        owner: &str,
        repo: &str,
        state: GiteaPullRequestState,
    ) -> Result<Vec<GiteaPullRequest>, GeectlError> {
        validate_gitea_name(owner, "owner")?;
        validate_gitea_name(repo, "repo")?;
        self.list_paged::<GiteaPullRequest>(
            &["api", "v1", "repos", owner, repo, "pulls"],
            &[("state", state.as_str())],
            &format!("pull requests of {owner}/{repo}"),
            "the list is bounded at 500 entries",
        )
    }

    /// Search pull requests across the instance (`GET
    /// /api/v1/repos/issues/search?type=pulls&state=…`): the
    /// search answers issues, so the items carry the repo and the
    /// merged flag but no head/base endpoints. Paginates
    /// internally and is bounded like `repo_list`. The token
    /// needs the `read:issue` scope: the server gates the issues
    /// search on it (403 without it).
    pub fn pr_search(&self, state: GiteaPullRequestState) -> Result<Vec<GiteaIssue>, GeectlError> {
        self.list_paged::<GiteaIssue>(
            &["api", "v1", "repos", "issues", "search"],
            &[("type", "pulls"), ("state", state.as_str())],
            "pull requests on the instance",
            "the list is bounded at 500 entries",
        )
    }

    /// List the reviews on a pull request (`GET
    /// /api/v1/repos/{owner}/{repo}/pulls/{number}/reviews`): one
    /// entry per review, in order. Paginates internally and is
    /// bounded like `repo_list`.
    pub fn pr_reviews(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<Vec<GiteaReview>, GeectlError> {
        validate_gitea_name(owner, "owner")?;
        validate_gitea_name(repo, "repo")?;
        validate_pr_number(number)?;
        self.list_paged::<GiteaReview>(
            &[
                "api",
                "v1",
                "repos",
                owner,
                repo,
                "pulls",
                &number.to_string(),
                "reviews",
            ],
            &[],
            &format!("reviews of pull request {owner}/{repo}#{number}"),
            "the list is bounded at 500 entries",
        )
    }

    /// Read every page of a list endpoint (`page`/`limit` query)
    /// into one typed list. A page shorter than the page size ends
    /// the list; a page of exactly the page size fetches the next
    /// one. Past `MAX_LIST_PAGES` full pages the call reads one
    /// more page to prove the list is longer: an empty page 11
    /// means the list is exactly at the bound and is returned, a
    /// non-empty one fails with `InvalidInput` naming `scope` and
    /// the per-call-site `remedy` — what the caller can do about
    /// it — instead of silently truncating or reading unboundedly.
    /// The remedy is per call site because not every list can be
    /// narrowed: the token owner's own repos can (list one user or
    /// org instead), a single user, a single org, and a repo's
    /// branches cannot.
    fn list_paged<T>(
        &self,
        segments: &[&str],
        params: &[(&str, &str)],
        scope: &str,
        remedy: &str,
    ) -> Result<Vec<T>, GeectlError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let mut items = Vec::new();
        for page in 1..=MAX_LIST_PAGES {
            let page_arg = page.to_string();
            let limit_arg = LIST_PAGE_SIZE.to_string();
            let mut query: Vec<(&str, &str)> = params.to_vec();
            query.push(("page", &page_arg));
            query.push(("limit", &limit_arg));
            let url = build_url(&self.base, segments, &query);
            let body = self.http.get_with(&url, &self.credentials)?;
            // Gitea answers a JSON `null` body for an empty list —
            // verified live on the branches endpoint of
            // an empty repo (`pool/systemd` before its first
            // commit). Parse as an optional list so a null page is
            // an empty page, not a server error. Any other shape
            // still fails with the same message as before.
            let page_items: Vec<T> = serde_json::from_str::<Option<Vec<T>>>(&body)
                .map_err(|err| {
                    GeectlError::new(
                        ErrorKind::ServerError,
                        format!("unexpected Gitea {scope} response (page {page}): {err}"),
                    )
                })?
                .unwrap_or_default();
            let full = page_items.len() == LIST_PAGE_SIZE as usize;
            items.extend(page_items);
            if !full {
                return Ok(items);
            }
        }
        // Ten full pages: the list is at least at the bound. One
        // more read decides — an empty page 11 proves exactly 500
        // entries, which is at the bound and is returned; anything
        // on it proves more than 500, and only then does the call
        // fail. Without this read a list of exactly 500 would fail
        // on an unproven "more than 500 entries".
        let page = MAX_LIST_PAGES + 1;
        let page_arg = page.to_string();
        let limit_arg = LIST_PAGE_SIZE.to_string();
        let mut query: Vec<(&str, &str)> = params.to_vec();
        query.push(("page", &page_arg));
        query.push(("limit", &limit_arg));
        let url = build_url(&self.base, segments, &query);
        let body = self.http.get_with(&url, &self.credentials)?;
        let page_items: Vec<T> = serde_json::from_str::<Option<Vec<T>>>(&body)
            .map_err(|err| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    format!("unexpected Gitea {scope} response (page {page}): {err}"),
                )
            })?
            .unwrap_or_default();
        if page_items.is_empty() {
            return Ok(items);
        }
        Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!(
                "the list of {scope} has more than {} entries ({} pages of {LIST_PAGE_SIZE}); {remedy}",
                MAX_LIST_PAGES * LIST_PAGE_SIZE,
                MAX_LIST_PAGES
            ),
        ))
    }

    /// Give the agent a place to work on a git-backed package (or
    /// project): Gitea fork + OBS `cmd=fork` wired to the fork's
    /// scmsync URL, in one call. The user-facing verb `branch` —
    /// on OBS-native the same verb is `branch_package`; on
    /// git-backed it is this composite. No local clone is made.
    ///
    /// Steps: read the OBS package (or project, for package
    /// `"_project"`) meta and refuse with `InvalidInput` when it
    /// carries no `<scmsync>`; resolve the devel project unless
    /// `no_devel_project` — fork from the devel source, never
    /// silently from the named project when a devel exists; parse
    /// the scmsync URL and resolve the branch (the explicit
    /// `fork_branch`, else the source branch, else the source
    /// repo's default branch via `repo_get`); Gitea fork (reuse
    /// when it already exists); OBS fork with the full scmsync
    /// URL `{base}/{fork-owner}/{fork-repo}#{branch}`. The fork's
    /// scmsync URL is built from the configured Gitea base, and a
    /// source repo on another host fails with `ConfigError`
    /// naming the configured host (one instance per client).
    ///
    /// In dry-run the fork is recorded in the plan and nothing is
    /// created — the guard runs before any server read, like
    /// `branch_package` — and the result is `None`. The plan names
    /// what is known without a read; the rest is the server
    /// default.
    pub fn branch(
        &self,
        obs: &ObsClient,
        project: &str,
        package: &str,
        options: &ForkOptions,
        dry_run: &DryRun,
    ) -> Result<Option<ForkResult>, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if let Some(name) = &options.new_repo_name {
            validate_gitea_name(name, "fork name")?;
        }
        if let Some(organization) = &options.organization {
            validate_gitea_name(organization, "organization")?;
        }
        if let Some(branch) = &options.fork_branch {
            validate_git_branch_name(branch)?;
        }
        if let Some(target) = &options.target_project {
            if !valid_obs_name(target) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS target project name: {target:?}"),
                ));
            }
        }
        if let Some(target) = &options.target_package {
            if !valid_obs_name(target) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS target package name: {target:?}"),
                ));
            }
        }
        // The dry-run guard: in dry-run the fork is recorded in the
        // plan and nothing is created — not even the meta read
        // runs. The plan names the target only when the caller gave
        // one; otherwise it says the default target project instead
        // of guessing it, and the fork name likewise.
        let target_project = options
            .target_project
            .as_deref()
            .unwrap_or("the default target project");
        let target_package = options.target_package.as_deref().unwrap_or(package);
        let fork_name = options
            .new_repo_name
            .as_deref()
            .unwrap_or("the source repository name");
        let fork_owner = options.organization.as_deref().unwrap_or("your account");
        if !dry_run.allow(&format!(
            "fork git-backed package {project}/{package} (Gitea fork {fork_owner}/{fork_name} + OBS fork to {target_project}/{target_package})"
        )) {
            return Ok(None);
        }
        // Read the meta of the thing being forked and resolve the
        // source: the package meta, or the project meta for a
        // project fork ("_project").
        let (source_project, source_package, scmsync) = if package == "_project" {
            let meta = obs.project_meta(project)?;
            let source_project = match (&meta.devel, options.no_devel_project) {
                (Some(devel_project), false) => {
                    self.debug.emit(&format!(
                        "decision: forking project {project} from devel project {devel_project} instead"
                    ));
                    devel_project.clone()
                }
                _ => project.to_string(),
            };
            let scmsync = if source_project == project {
                meta.scmsync
            } else {
                // The devel project's meta carries its own scmsync.
                obs.project_meta(&source_project)?.scmsync
            };
            let scmsync = scmsync.ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "project {source_project} is not managed in Git; forking is possible only with packages managed in Git"
                    ),
                )
            })?;
            (source_project, "_project".to_string(), scmsync)
        } else {
            let meta = obs.package_meta(project, package)?;
            let (source_project, source_package) = match (&meta.devel, options.no_devel_project) {
                (Some((devel_project, devel_package)), false) => {
                    self.debug.emit(&format!(
                            "decision: forking from devel {devel_project}/{devel_package} instead of {project}/{package}"
                        ));
                    (devel_project.clone(), devel_package.clone())
                }
                _ => (project.to_string(), package.to_string()),
            };
            let scmsync = if source_project == project && source_package == package {
                meta.scmsync
            } else {
                // The devel source's meta carries its own scmsync.
                obs.package_meta(&source_project, &source_package)?.scmsync
            };
            let scmsync = match scmsync {
                Some(url) => url,
                None => {
                    // The package itself is not git-managed. When
                    // its project is, the actionable alternative
                    // is forking the project.
                    let hint = match obs.project_meta(project) {
                        Ok(project_meta) if project_meta.scmsync.is_some() => format!(
                            "; the project {project} is managed in Git — fork the project (package \"_project\") instead"
                        ),
                        _ => String::new(),
                    };
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "package {project}/{package} is not managed in Git; forking is possible only with packages managed in Git{hint}"
                        ),
                    ));
                }
            };
            (source_project, source_package, scmsync)
        };
        let parsed = parse_scmsync_url(&scmsync)?;
        // Decision 5: one configured Gitea instance per client. A
        // repo URL on another host fails with `ConfigError` naming
        // the configured host. Both sides are normalized the same
        // way, so a base that names the default port explicitly
        // (`https://src.opensuse.org:443`) still matches the bare
        // host in the scmsync URL.
        let scheme = if self.base.starts_with("https://") {
            "https"
        } else {
            "http"
        };
        if !parsed
            .authority
            .eq_ignore_ascii_case(&normalize_authority(self.authority(), scheme))
        {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!(
                    "the scmsync URL {scmsync:?} is on another Gitea host; this client is configured for {}",
                    self.base
                ),
            ));
        }
        // Resolve the branch: the explicit fork branch, else the
        // source branch, else the source repo's default branch.
        let source_branch = match (&options.fork_branch, &parsed.branch) {
            (Some(explicit), _) => explicit.clone(),
            (None, Some(branch)) => branch.clone(),
            (None, None) => {
                let default = self.repo_get(&parsed.owner, &parsed.repo)?.default_branch;
                self.debug.emit(&format!(
                    "decision: no branch named in the scmsync URL; using the default branch {default}"
                ));
                default
            }
        };
        // The resolved branch is validated before it is embedded
        // in the fork's scmsync URL: the explicit fork branch was
        // validated at entry, but the scmsync fragment (or the
        // trackingbranch value) and the repo's default branch come
        // from the server, untrusted here — a fragment carrying
        // `#` would truncate the scmsync URL the OBS side clones.
        validate_git_branch_name(&source_branch)?;
        let fork_options = ForkRepoOptions {
            new_name: options.new_repo_name.clone(),
            organization: options.organization.clone(),
        };
        let fork = match self.fork_repo(&parsed.owner, &parsed.repo, &fork_options, dry_run)? {
            Some(fork) => fork,
            None => {
                return Err(GeectlError::new(
                    ErrorKind::InternalError,
                    "fork_repo returned no fork in live mode",
                ));
            }
        };
        // The OBS fork's scmsync is the full URL: the server stores
        // it verbatim and the sync backend clones it.
        let fork_scmsync = format!(
            "{}/{}/{}#{source_branch}",
            self.base, fork.owner.login, fork.name
        );
        let forked = obs.fork_package(
            &source_project,
            &source_package,
            &fork_scmsync,
            options.target_project.as_deref(),
            options.target_package.as_deref(),
        )?;
        Ok(Some(ForkResult {
            owner: fork.owner.login,
            repo: fork.name,
            branch: source_branch,
            scmsync: fork_scmsync,
            target_project: forked.target_project,
            target_package: forked.target_package,
            clone_url: fork.clone_url,
            ssh_url: fork.ssh_url,
        }))
    }
}

/// Validate a Gitea API base URL. It must be a bare `https://` URL
/// (scheme plus authority, no path, query, fragment, or userinfo)
/// with a host (the production instance), or a bare `http://` URL
/// on a loopback host (a test instance only). Anything else is
/// refused at construction, not at the first request. A trailing
/// slash is stripped, not a path.
fn validate_gitea_base(base: &str) -> Result<(), GeectlError> {
    // A base is scheme plus authority only: anything after the
    // authority would be string-concatenated into every Gitea URL
    // (see `build_url`), so a base such as
    // `https://src.opensuse.org/evil` would rewrite them all.
    let base = base.trim_end_matches('/');
    if let Some(authority) = base.strip_prefix("https://") {
        if !is_bare_authority(authority) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "Gitea base URL must be a bare https:// URL (scheme + host, no path, query, fragment, or userinfo)",
            ));
        }
        // Reuse the production builder's validation (https + host).
        UrlBuilder::new(base).map(|_| ())
    } else if let Some(authority) = base.strip_prefix("http://") {
        if !is_bare_authority(authority) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "Gitea base URL must be a bare http:// URL (scheme + host, no path, query, fragment, or userinfo)",
            ));
        }
        if is_loopback_url(base) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "an http:// Gitea base must be a loopback URL (a test instance only)",
            ))
        }
    } else {
        Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "Gitea base URL must start with https:// (or http:// for a loopback test instance)",
        ))
    }
}

/// A Gitea owner, repo, organization, or fork name: non-empty,
/// ASCII alphanumerics plus `-_.`, never `.` or `..`. The server
/// validates the rest; this refuses only what is never a name.
fn valid_gitea_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

pub(crate) fn validate_gitea_name(name: &str, label: &str) -> Result<(), GeectlError> {
    if !valid_gitea_name(name) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid Gitea {label} name: {name:?}"),
        ));
    }
    Ok(())
}

/// Split an `owner/repo` coordinate into its parts, validating
/// both names. Anything else is an invalid input naming the
/// coordinate.
pub fn parse_gitea_repo(value: &str) -> Result<(String, String), GeectlError> {
    let (owner, repo) = value.split_once('/').ok_or_else(|| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid Gitea repository: {value:?} (expected owner/repo)"),
        )
    })?;
    validate_gitea_name(owner, "owner")?;
    validate_gitea_name(repo, "repo")?;
    Ok((owner.to_string(), repo.to_string()))
}

/// A Gitea pull-request number: positive, like an OBS request id.
fn validate_pr_number(number: u64) -> Result<(), GeectlError> {
    if number == 0 {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "invalid Gitea pull-request number: 0",
        ));
    }
    Ok(())
}

/// A Gitea team id: a positive integer. Zero and negatives name
/// no team.
fn validate_team_id(id: i64) -> Result<(), GeectlError> {
    if id <= 0 {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid Gitea team id: {id}"),
        ));
    }
    Ok(())
}

/// A Gitea team name: the server's rule is alphanumerics plus
/// `-_.`, at most 255 characters — spaces are refused with 422,
/// and a longer name too. The server is the authority on the
/// rest.
fn validate_gitea_team_name(name: &str) -> Result<(), GeectlError> {
    let valid = !name.is_empty()
        && name.chars().count() <= 255
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !valid {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid Gitea team name: {name:?}"),
        ));
    }
    Ok(())
}

/// A git branch name for the explicit fork branch — and for the
/// resolved branch before it is embedded in a fork's scmsync URL.
/// The git-check-ref-format rules, for one-level names
/// (`refs/heads/<name>`): no control characters, whitespace, `~`,
/// `^`, `:`, `?`, `*`, `[`, or `\`; no `..`; no `@{`; no leading
/// or trailing `/`, no `//`; no component starting with `.` or
/// ending with `.lock`; not `@` alone; no trailing `.`. `#` is
/// refused as well: it is legal in git, but the branch is embedded
/// verbatim in a `#fragment`, so a `#` inside it would truncate
/// the scmsync URL — the value is rejected, never percent-encoded,
/// because the server stores the scmsync URL verbatim and clones
/// exactly it. The server is the authority on the rest (slashes
/// are legal in branch names, so they are allowed here, unlike in
/// a repo name).
fn validate_git_branch_name(branch: &str) -> Result<(), GeectlError> {
    let valid = !branch.is_empty()
        && branch != "@"
        && !branch.ends_with('.')
        && !branch.contains("..")
        && !branch.contains("@{")
        && !branch.starts_with('/')
        && !branch.ends_with('/')
        && !branch.contains("//")
        && branch.chars().all(|c| {
            !c.is_control()
                && !c.is_whitespace()
                && !matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\' | '#')
        })
        && branch
            .split('/')
            .all(|component| !component.starts_with('.') && !component.ends_with(".lock"));
    if !valid {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid git branch name: {branch:?}"),
        ));
    }
    Ok(())
}

/// Parse a Gitea repository out of a response body. A body that is
/// not a repository is a server error naming the repo, never a
/// silent empty value — the caller asked for this repo. A body
/// that names another repo is a server error too: the callers
/// (fork verification, the delete pre-read) treat the read as the
/// requested repo, so a response for `other/name` must fail here
/// instead of being deleted or verified as `owner/repo`. Gitea
/// logins are case-insensitive, so the comparison canonicalizes
/// case on both sides.
fn parse_repository(body: &str, owner: &str, repo: &str) -> Result<GiteaRepository, GeectlError> {
    let parsed = parse_repository_body(body, &format!("for {owner}/{repo}"))?;
    let expected = format!("{owner}/{repo}");
    if !parsed.full_name.eq_ignore_ascii_case(&expected) {
        return Err(GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected Gitea repository response for {owner}/{repo}: the server answered for {}",
                parsed.full_name
            ),
        ));
    }
    Ok(parsed)
}

/// Deserialize a Gitea repository out of a response body, naming
/// `context` in the error when the body is not a repository. The
/// identity check lives in `parse_repository` (for reads of a known
/// repo) or in the caller (for creates, where the owner is the
/// token owner and unknown here); this only proves the body is a
/// repository at all.
fn parse_repository_body(body: &str, context: &str) -> Result<GiteaRepository, GeectlError> {
    serde_json::from_str(body).map_err(|err| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected Gitea repository response {context}: {err}"),
        )
    })
}

/// Parse a Gitea pull request out of a response body. A body that
/// is not a pull request is a server error naming the PR, never a
/// silent empty value — the caller asked for this PR. A body that
/// names another PR number is a server error too: numbers are
/// unique per repo, so the comparison is exact.
fn parse_pull_request(
    body: &str,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<GiteaPullRequest, GeectlError> {
    let parsed: GiteaPullRequest = serde_json::from_str(body).map_err(|err| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected Gitea pull-request response for {owner}/{repo}#{number}: {err}"),
        )
    })?;
    if parsed.number != number {
        return Err(GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected Gitea pull-request response for {owner}/{repo}#{number}: the server answered for pull request {}",
                parsed.number
            ),
        ));
    }
    Ok(parsed)
}

/// Parse a collaborator permission out of a response body. A body
/// that is not a permission is a server error naming the user and
/// the repo, never a silent empty value — the caller asked for
/// this user's permission. A body that names another user is a
/// server error too: the callers (the add verification, the
/// remove pre-read) treat the read as the requested user, so a
/// response for another user must fail here instead of being
/// verified or deleted as the requested one. Gitea logins are
/// case-insensitive, so the comparison canonicalizes case on
/// both sides.
fn parse_collaborator_permission(
    body: &str,
    owner: &str,
    repo: &str,
    user: &str,
) -> Result<GiteaCollaboratorPermission, GeectlError> {
    let parsed: GiteaCollaboratorPermission =
        serde_json::from_str(body).map_err(|err| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected Gitea collaborator permission response for {user} on {owner}/{repo}: {err}"
                ),
            )
        })?;
    if !parsed.user.login.eq_ignore_ascii_case(user) {
        return Err(GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected Gitea collaborator permission response for {user} on {owner}/{repo}: the server answered for {}",
                parsed.user.login
            ),
        ));
    }
    Ok(parsed)
}

/// Parse a Gitea team out of a response body. A body that is not
/// a team is a server error naming the team id, never a silent
/// empty value — the caller asked for this team. A body that
/// names another id is a server error too: the callers (the
/// create verification, the delete pre-read) treat the read as
/// the requested team, so a response for another team must fail
/// here instead of being verified or deleted as the requested one.
fn parse_team(body: &str, id: i64) -> Result<GiteaTeam, GeectlError> {
    let parsed = parse_team_body(body, &format!("for team {id}"))?;
    if parsed.id != id {
        return Err(GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected Gitea team response for team {id}: the server answered for team {}",
                parsed.id
            ),
        ));
    }
    Ok(parsed)
}

/// Parse a team body without an id check: the create response
/// names an id the caller does not know yet, so the id is
/// verified by the caller against the created name instead.
fn parse_team_body(body: &str, context: &str) -> Result<GiteaTeam, GeectlError> {
    serde_json::from_str(body).map_err(|err| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected Gitea team response {context}: {err}"),
        )
    })
}

/// Parse a team member out of a response body. A body that is not
/// a user is a server error naming the team and the user, never a
/// silent empty value. A body that names another user is a server
/// error too, for the same reason as
/// `parse_collaborator_permission`: the callers treat the read as
/// the requested user.
fn parse_team_member(body: &str, id: i64, user: &str) -> Result<GiteaOwner, GeectlError> {
    let parsed: GiteaOwner = serde_json::from_str(body).map_err(|err| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected Gitea team member response for {user} in team {id}: {err}"),
        )
    })?;
    if !parsed.login.eq_ignore_ascii_case(user) {
        return Err(GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected Gitea team member response for {user} in team {id}: the server answered for {}",
                parsed.login
            ),
        ));
    }
    Ok(parsed)
}

/// Parse a Gitea branch out of a response body. A body that is not
/// a branch is a server error naming the branch, never a silent
/// empty value — the caller asked for this branch. A body that
/// names another branch is a server error too: branch names are
/// case-sensitive, so the comparison is exact.
fn parse_branch(
    body: &str,
    owner: &str,
    repo: &str,
    branch: &str,
) -> Result<GiteaBranch, GeectlError> {
    let parsed: GiteaBranch = serde_json::from_str(body).map_err(|err| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected Gitea branch response for {owner}/{repo}@{branch}: {err}"),
        )
    })?;
    if parsed.name != branch {
        return Err(GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected Gitea branch response for {owner}/{repo}@{branch}: the server answered for {}",
                parsed.name
            ),
        ));
    }
    Ok(parsed)
}

/// Strip what does not change which instance an authority names:
/// userinfo (`user@host`) and the scheme's default port
/// (`:443` for https, `:80` for http). A non-default port is
/// kept — it names a different instance.
fn normalize_authority(authority: &str, scheme: &str) -> String {
    let hostport = authority
        .rsplit_once('@')
        .map(|(_, after)| after)
        .unwrap_or(authority);
    let default_port = match scheme {
        "https" => ":443",
        "http" => ":80",
        _ => "",
    };
    hostport
        .strip_suffix(default_port)
        .unwrap_or(hostport)
        .to_string()
}

/// Decode `%XX` sequences in a branch value (the fragment or the
/// `trackingbranch` value may carry an encoded branch name). A
/// malformed sequence is left as-is rather than failing the whole
/// parse — the branch is validated again by the server.
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(decoded) = u8::from_str_radix(hex, 16) {
                out.push(decoded);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse a scmsync URL (`url#branch`, `trackingbranch` query
/// honoured) into its parts, mirroring the server's
/// `generate_scmsyncinfo`: the repo is scheme + host + path with a
/// trailing `.git` stripped and a `git+` scheme prefix stripped;
/// the branch is the `#fragment`; the `trackingbranch` query value
/// is honoured as the branch only when no fragment names one.
fn parse_scmsync_url(scmsync: &str) -> Result<ScmsyncUrl, GeectlError> {
    let invalid = |reason: &str| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid scmsync URL {scmsync:?}: {reason}"),
        )
    };
    let (before_fragment, fragment) = match scmsync.split_once('#') {
        Some((before, fragment)) => (before, Some(fragment)),
        None => (scmsync, None),
    };
    let (repo_url, query) = match before_fragment.split_once('?') {
        Some((before, query)) => (before, Some(query)),
        None => (before_fragment, None),
    };
    let (scheme, rest) = repo_url
        .split_once("://")
        .ok_or_else(|| invalid("no scheme"))?;
    let scheme = scheme.strip_prefix("git+").unwrap_or(scheme);
    if scheme != "https" && scheme != "http" {
        return Err(invalid(
            "the scheme must be https (or http for a test instance)",
        ));
    }
    let (authority, path) = rest
        .split_once('/')
        .ok_or_else(|| invalid("no host/path"))?;
    if authority.is_empty() {
        return Err(invalid("no host"));
    }
    // Userinfo and the scheme's default port do not change which
    // instance the URL names, so both are stripped before the
    // authority is compared with the client's.
    let authority = normalize_authority(authority, scheme);
    // Strip a trailing `.git` from the path, like the server.
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    let owner = segments.next().ok_or_else(|| invalid("no owner"))?;
    let repo = segments.next().ok_or_else(|| invalid("no repo"))?;
    if segments.next().is_some() {
        return Err(invalid("the path must name one owner and one repository"));
    }
    if !valid_gitea_name(owner) {
        return Err(invalid("invalid owner name"));
    }
    if !valid_gitea_name(repo) {
        return Err(invalid("invalid repo name"));
    }
    let tracking_branch = query.and_then(|query| {
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            if key == "trackingbranch" && !value.is_empty() {
                Some(percent_decode(value))
            } else {
                None
            }
        })
    });
    // The fragment names the branch, like trackingbranch — and
    // like it, it is percent-decoded: `#feature%2Fx` is the branch
    // `feature/x`, not the literal `%2F`.
    let branch = fragment
        .filter(|fragment| !fragment.is_empty())
        .map(percent_decode)
        .or(tracking_branch);
    Ok(ScmsyncUrl {
        authority: authority.to_string(),
        owner: owner.to_string(),
        repo: repo.to_string(),
        branch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Credentials;
    use crate::http::Timeout;
    use crate::http::UreqClient;
    use crate::obs::test_support::basic_credentials;
    use crate::obs::test_support::FakeHttp;

    /// Fixture layout: the real openSUSE Gitea instance and the
    /// real OBS metas that point at it.
    ///
    /// Fetched from https://src.opensuse.org
    /// (code.opensuse.org) and from the OBS API via osc — the
    /// instance `DEFAULT_GITEA_URL` points at — and mirrored here
    /// instead of invented repo/branch names. `FakeHttp` cans the
    /// responses, so no network happens; the addresses are the
    /// real ones.
    /// - Source repos live in the `pool` org, named after the
    ///   package: `pool/systemd` (default branch `factory`;
    ///   branches `factory`, `slfo-main`, `slfo-1.0`, `slfo-1.1`,
    ///   `slfo-1.2`, `slfo-1.3`) and `pool/libstrophe` (default
    ///   `factory`). Clone
    ///   `https://src.opensuse.org/pool/<repo>.git`, SSH
    ///   `gitea@src.opensuse.org:pool/<repo>.git`.
    /// - Forks keep the source name under the forker: user fork
    ///   `fbui/systemd` (parent `pool/systemd`, default
    ///   `factory`), `lbsobolewski/systemd` (parent
    ///   `pool/systemd`, default `slfo-main`), org fork
    ///   `rpm/systemd` (parent `pool/systemd`, default
    ///   `factory`).
    /// - `xmpp/libstrophe` (parent `pool/libstrophe`, default
    ///   `main`) is the repo `network:messaging:xmpp/libstrophe`
    ///   syncs from (its `<scmsync>` pins a commit SHA).
    /// - The `pool` org is a mirror, not a sync source:
    ///   `pool/systemd` is fed by the OBS-to-Git bridge (commits
    ///   authored by "Git OBS Bridge", `OBS-URL:` in the
    ///   message); neither `openSUSE:Factory/systemd` nor
    ///   `Base:System/systemd` carries `<scmsync>`, so `branch`
    ///   on systemd correctly refuses. The `leap-16.0` branch
    ///   does not exist on `pool/systemd` — it lives on
    ///   `openSUSE/Leap` (branches `leap-16.0`, `leap-16.1`,
    ///   `leap-16.2`, default `leap-16.1`).
    /// - Per-package `<scmsync>` (the shape `branch` reads):
    ///   `games/descent3` carries `jengelh/descent3#master`
    ///   (repo default `master`, the only branch; no `<devel>`),
    ///   and `science/gsl` carries `adamm/gsl.git#factory`
    ///   (`adamm/gsl` branches `factory`, `slfo-1.2`,
    ///   `slfo-main`, default `factory`, itself a fork of
    ///   `pool/gsl`).
    /// - A real devel pair: `openSUSE:Factory/gsl` has no
    ///   `<scmsync>` of its own, only `<devel project="science"
    ///   package="gsl"/>` — the fork resolves to `science/gsl`.
    /// - Project-level `<scmsync>`: `network:messaging:xmpp`
    ///   carries `xmpp/_ObsPrj.git#master` (`xmpp/_ObsPrj`
    ///   default `master`), and `openSUSE:Leap:16.0` carries
    ///   `openSUSE/Leap#leap-16.0`. Packages in those projects
    ///   carry their own `<scmsync>` too (e.g. `xmpp/dino`
    ///   names `trackingbranch=main` with a SHA fragment;
    ///   `Leap:16.0/CoreFreq` names `trackingbranch=Leap-16.0`
    ///   with a SHA fragment; `Leap:16.0/000release-packages`
    ///   syncs via `?subdir=`).
    /// - `openSUSE:Factory/libstrophe` carries neither
    ///   `<scmsync>` nor `<devel>` — its git management lives
    ///   only in project scmsync / `_scmsync.obsinfo`, which
    ///   `branch` does not read, so `branch` on it refuses.
    ///
    /// Constructed cases (no live equivalent found): the renamed
    /// fork `systemd-slfo` (no renamed fork exists on the
    /// instance); the no-devel test, which combines
    /// `Factory/gsl`'s real `<devel>` with `science/gsl`'s real
    /// scmsync value in one synthetic meta — no real package
    /// carries both `<devel>` and `<scmsync>` (0 of the 9,046
    /// packages with `<scmsync>`); and the project-alternative
    /// hint, whose project scmsync is synthetic — every package
    /// checked in a scmsync-managed project (`xmpp`, `Leap:16.0`)
    /// carries its own `<scmsync>`.
    const BASE: &str = "https://src.opensuse.org";

    fn bearer() -> Credentials {
        Credentials::Bearer {
            token: "test-token".to_string(),
        }
    }

    fn client<'a>(http: &'a dyn HttpClient) -> GiteaClient<'a> {
        GiteaClient::with_base(http, BASE, bearer()).unwrap()
    }

    fn test_vars(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn test_lookup(
        map: &std::collections::HashMap<String, String>,
    ) -> impl Fn(&str) -> Option<String> + '_ {
        move |name| map.get(name).cloned()
    }

    fn no_secret_files() -> impl Fn(&std::path::Path) -> std::io::Result<String> {
        |_| Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no file"))
    }

    fn obs_client<'a>(http: &'a dyn HttpClient) -> ObsClient<'a> {
        ObsClient::with_credentials(http, basic_credentials()).unwrap()
    }

    /// A repo JSON body, shaped like Gitea 1.24.7 answers, with
    /// unknown fields (`future_field`) at the top level and inside
    /// `owner` / `parent` to prove they are ignored. The default
    /// branch is `factory` — the real default of the `pool/*`
    /// repos this helper is used for; repos whose default is
    /// `master` use `repo_json_branch` instead.
    fn repo_json(owner: &str, name: &str, parent: Option<(&str, &str)>) -> String {
        repo_json_branch(owner, name, parent, "factory", false)
    }

    /// Like `repo_json`, with the default branch given: the
    /// descent3 fork and the `_ObsPrj` fork inherit `master`.
    /// The privacy flag is given too: the create tests answer a
    /// private create with a private repo.
    fn repo_json_branch(
        owner: &str,
        name: &str,
        parent: Option<(&str, &str)>,
        default_branch: &str,
        private: bool,
    ) -> String {
        let parent_json = match parent {
            Some((parent_owner, parent_name)) => format!(
                r#"{{"owner": {{"login": "{parent_owner}", "future_field": 1}}, "name": "{parent_name}", "full_name": "{parent_owner}/{parent_name}", "future_field": true}}"#
            ),
            None => "null".to_string(),
        };
        let is_fork = parent.is_some();
        format!(
            r#"{{"id": 7, "owner": {{"login": "{owner}", "future_field": 1}}, "name": "{name}", "full_name": "{owner}/{name}", "description": "", "empty": true, "private": {private}, "fork": {is_fork}, "parent": {parent_json}, "future_field": "tolerated", "html_url": "{BASE}/{owner}/{name}", "ssh_url": "gitea@src.opensuse.org:{owner}/{name}.git", "clone_url": "{BASE}/{owner}/{name}.git", "default_branch": "{default_branch}", "created_at": "2026-11-21T00:00:00Z"}}"#
        )
    }

    /// A repository-list JSON body out of repo JSON bodies: the
    /// array shape the list endpoints answer.
    fn repo_list_json(repos: &[String]) -> String {
        format!("[{}]", repos.join(","))
    }

    /// Fifty repos under one owner, named `repo-00` … — the full
    /// page the pagination tests fill.
    fn fifty_repos(owner: &str) -> Vec<String> {
        (0..50)
            .map(|n| repo_json(owner, &format!("repo-{n:02}"), None))
            .collect()
    }

    /// A branch JSON body, shaped like Gitea 1.24.7 answers, with
    /// an unknown field (`future_field`) to prove it is ignored.
    /// The commit id is a real-looking SHA; `pool/systemd`'s real
    /// branches are `factory`, `slfo-main`, `slfo-1.0` …
    fn branch_json(name: &str, sha: &str, message: &str, protected: bool) -> String {
        format!(
            r#"{{"name": "{name}", "commit": {{"id": "{sha}", "message": "{message}", "url": "{BASE}/api/v1/repos/pool/systemd/git/commits/{sha}", "future_field": 1}}, "protected": {protected}, "future_field": "tolerated"}}"#
        )
    }

    /// A branch-list JSON body out of branch JSON bodies.
    fn branch_list_json(branches: &[String]) -> String {
        format!("[{}]", branches.join(","))
    }
    /// A pull-request JSON body, shaped like Gitea 1.24.7 answers,
    /// with an unknown field (`future_field`) to prove it is
    /// ignored. The PR is `openSUSE/geectl#122`: the head is the
    /// `change-review-state` branch on the `scarabeusiv/geectl`
    /// fork, the base is `main` on `openSUSE/geectl`.
    fn pr_json(number: u64, state: &str, merged: bool) -> String {
        let sha = "e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa";
        format!(
            r#"{{"id": 122, "number": {number}, "user": {{"login": "scarabeusiv", "future_field": 1}}, "title": "change_review_state", "body": "A test PR body.", "labels": [{{"id": 1, "name": "bug", "future_field": true}}], "state": "{state}", "draft": false, "merged": {merged}, "mergeable": true, "head": {{"label": "scarabeusiv:change-review-state", "ref": "change-review-state", "sha": "{sha}", "repo": {{"full_name": "scarabeusiv/geectl", "future_field": 1}}}}, "base": {{"label": "main", "ref": "main", "sha": "{sha}", "repo": {{"full_name": "openSUSE/geectl", "future_field": 1}}}}, "future_field": "tolerated"}}"#
        )
    }

    /// An issue-search JSON item, shaped like Gitea 1.24.7 answers
    /// for `GET /api/v1/repos/issues/search?type=pulls`: the issue
    /// fields plus the `pull_request` block and the `repository`
    /// block.
    fn issue_json(number: u64, state: &str, merged: bool) -> String {
        format!(
            r#"{{"id": 122, "number": {number}, "user": {{"login": "scarabeusiv"}}, "title": "change_review_state", "body": "A test PR body.", "labels": [], "state": "{state}", "pull_request": {{"merged": {merged}, "draft": false}}, "repository": {{"id": 1, "name": "geectl", "owner": "openSUSE", "full_name": "openSUSE/geectl"}}, "future_field": "tolerated"}}"#
        )
    }

    /// A review JSON body, shaped like Gitea 1.24.7 answers.
    fn review_json(login: &str, state: &str, body: &str) -> String {
        format!(
            r#"{{"id": 1, "user": {{"login": "{login}"}}, "state": "{state}", "body": "{body}", "dismissed": false, "submitted_at": "2026-11-22T19:52:58Z", "future_field": "tolerated"}}"#
        )
    }

    /// A package meta with a scmsync URL (and, when `devel` is set,
    /// a devel project/package).
    fn package_meta(
        project: &str,
        package: &str,
        scmsync: Option<&str>,
        devel: Option<&str>,
    ) -> String {
        let scmsync = scmsync
            .map(|url| format!("<scmsync>{url}</scmsync>"))
            .unwrap_or_default();
        let devel = devel
            .map(|devel| format!("<devel project=\"{devel}\" package=\"{package}\"/>"))
            .unwrap_or_default();
        format!(
            "<package name=\"{package}\" project=\"{project}\"><title>t</title>{scmsync}{devel}</package>"
        )
    }

    /// A project meta with a scmsync URL (and, when `devel` is set,
    /// a devel project).
    fn project_meta(project: &str, scmsync: Option<&str>, devel: Option<&str>) -> String {
        let scmsync = scmsync
            .map(|url| format!("<scmsync>{url}</scmsync>"))
            .unwrap_or_default();
        let devel = devel
            .map(|devel| format!("<devel project=\"{devel}\"/>"))
            .unwrap_or_default();
        format!("<project name=\"{project}\">{scmsync}{devel}</project>")
    }

    /// An OBS fork answer in the server's shape (deprecated data
    /// names, as `create_fork` renders them).
    fn fork_status(
        target_project: &str,
        target_package: &str,
        source_project: &str,
        source_package: &str,
    ) -> String {
        format!(
            "<status code=\"ok\"><summary>Ok</summary><data name=\"targetproject\">{target_project}</data><data name=\"targetpackage\">{target_package}</data><data name=\"sourceproject\">{source_project}</data><data name=\"sourcepackage\">{source_package}</data></status>"
        )
    }

    #[test]
    fn gitea_name_validation() {
        for valid in [
            "foo",
            "foo-bar",
            "foo_bar",
            "foo.bar",
            "a1",
            "UPSTREAM-1_x.y",
        ] {
            assert!(valid_gitea_name(valid), "{valid} should be valid");
        }
        for invalid in [
            "", ".", "..", "foo/bar", "foo bar", "foo@bar", "foo:bar", "foo?bar",
        ] {
            assert!(!valid_gitea_name(invalid), "{invalid} should be invalid");
        }
    }

    #[test]
    fn gitea_base_validation() {
        assert!(validate_gitea_base("https://src.opensuse.org").is_ok());
        assert!(validate_gitea_base("http://127.0.0.1:3001").is_ok());
        assert!(validate_gitea_base("http://localhost:3001").is_ok());
        assert!(validate_gitea_base("https://src.opensuse.org/evil").is_err());
        assert!(validate_gitea_base("http://example.com").is_err());
        assert!(validate_gitea_base("src.opensuse.org").is_err());
        assert!(validate_gitea_base("https://").is_err());
        // A trailing slash is stripped, not a path.
        let http = FakeHttp::new();
        let client = GiteaClient::with_base(&http, "http://127.0.0.1:3001/", bearer()).unwrap();
        assert_eq!(client.base, "http://127.0.0.1:3001");
    }

    #[test]
    fn git_branch_name_validation() {
        // Real branch names from the fixtures: one-level names,
        // slashed names, dotted releases, and a pinned commit SHA.
        for valid in [
            "factory",
            "master",
            "slfo-main",
            "leap-16.0",
            "feature/x",
            "release-1.2.3",
            "e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa",
        ] {
            assert!(
                validate_git_branch_name(valid).is_ok(),
                "{valid} should be valid"
            );
        }
        // git-check-ref-format refusals, plus `#`: legal in git,
        // but it would truncate the scmsync fragment the branch is
        // embedded in.
        for invalid in [
            "", "@", "a..b", "a b", "a\tb", "a\x01b", "a#b", "a?b", "a*b", "a[b", "a~b", "a^b",
            "a:b", "a\\b", "/a", "a/", "a//b", ".a", "a/.b", "a.lock", "a/b.lock", "a@{b", "a.",
        ] {
            assert!(
                validate_git_branch_name(invalid).is_err(),
                "{invalid:?} should be invalid"
            );
        }
    }

    #[test]
    fn construction_fails_closed_without_a_bearer_token() {
        // No `unwrap_err` here: `GiteaClient` carries `&dyn
        // HttpClient`, so it has no `Debug` (like `ObsClient`).
        let http = FakeHttp::new();
        for credentials in [
            Credentials::None,
            basic_credentials(),
            Credentials::SshKey {
                path: std::path::PathBuf::from("/tmp/key"),
            },
        ] {
            match GiteaClient::with_base(&http, BASE, credentials) {
                Ok(_) => panic!("construction without a Bearer token must fail"),
                Err(err) => assert_eq!(err.kind(), ErrorKind::ConfigError),
            }
        }
    }

    #[test]
    fn with_base_resolved_uses_the_alias_token_for_its_url() {
        // Two instances, two tokens: each client is built from the
        // credentials resolved for its own base URL, and each sends
        // them (the FakeHttp records an authed call). Which token
        // resolves for which URL is proven at the auth layer; this
        // test proves the constructor wires the resolution through.
        let vars = test_vars(&[
            ("GEECTL_GITEA_TOKEN", "plain-token"),
            ("GEECTL_GITEA_TOKEN_EXAMPLE", "alias-token"),
        ]);
        let instances = vec![(
            "example".to_string(),
            "https://gitea.example.org".to_string(),
        )];
        let http = FakeHttp::new().with_get(
            "https://gitea.example.org/api/v1/repos/pool/systemd",
            &repo_json("pool", "systemd", None),
        );
        let client = GiteaClient::with_base_resolved(
            &http,
            "https://gitea.example.org",
            BASE,
            &instances,
            &test_lookup(&vars),
            &no_secret_files(),
        )
        .unwrap();
        let repo = client.repo_get("pool", "systemd").unwrap();
        assert_eq!(repo.full_name, "pool/systemd");
        // The recorded bearer token proves the alias token went to
        // the alias URL — the `authed` flag alone cannot tell the
        // alias token from the plain one.
        assert_eq!(
            http.get_bearer_tokens(),
            vec![(
                "https://gitea.example.org/api/v1/repos/pool/systemd".to_string(),
                Some("alias-token".to_string())
            )]
        );
        // The default URL takes the plain variables, even with an
        // alias defined for another instance.
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/pool/systemd",
            &repo_json("pool", "systemd", None),
        );
        let client = GiteaClient::with_base_resolved(
            &http,
            BASE,
            BASE,
            &instances,
            &test_lookup(&vars),
            &no_secret_files(),
        )
        .unwrap();
        client.repo_get("pool", "systemd").unwrap();
        assert_eq!(
            http.get_bearer_tokens(),
            vec![(
                "https://src.opensuse.org/api/v1/repos/pool/systemd".to_string(),
                Some("plain-token".to_string())
            )]
        );
    }

    #[test]
    fn with_base_resolved_fails_clear_for_an_unmatched_url() {
        let vars = test_vars(&[("GEECTL_GITEA_TOKEN", "plain-token")]);
        let instances = vec![(
            "example".to_string(),
            "https://gitea.example.org".to_string(),
        )];
        // The default's token is configured, but the URL is another
        // instance with no alias: the error names the [instances]
        // entry, and the token never leaks into it.
        // No `unwrap_err` here: `GiteaClient` carries `&dyn
        // HttpClient`, so it has no `Debug` (like the construction
        // test above).
        let http = FakeHttp::new();
        // The lookup and the file reader are bound first: a
        // temporary closure passed inline would live to the end of
        // the `match` scrutinee, past the `vars` it borrows.
        let lookup = test_lookup(&vars);
        let files = no_secret_files();
        match GiteaClient::with_base_resolved(
            &http,
            "https://other.example.org",
            BASE,
            &instances,
            &lookup,
            &files,
        ) {
            Ok(_) => panic!("construction for an unmatched URL must fail"),
            Err(err) => {
                assert_eq!(err.kind(), ErrorKind::ConfigError);
                assert!(err.context.contains("[instances]"));
                assert!(!err.context.contains("plain-token"));
            }
        }
        // Nothing configured anywhere: construction still fails
        // closed, with no token to send — and the error names the
        // `[instances]` entry, not the plain variables.
        let vars = test_vars(&[]);
        let lookup = test_lookup(&vars);
        match GiteaClient::with_base_resolved(
            &http,
            "https://other.example.org",
            BASE,
            &[],
            &lookup,
            &files,
        ) {
            Ok(_) => panic!("construction without any token must fail"),
            Err(err) => {
                assert_eq!(err.kind(), ErrorKind::ConfigError);
                assert!(err.context.contains("[instances]"));
            }
        }
    }

    #[test]
    fn with_base_resolved_validates_the_base_first() {
        // An invalid base gets the base-validation error, not a
        // credential error: resolution must not run first and
        // misdirect the user to `[instances]`.
        let vars = test_vars(&[("GEECTL_GITEA_TOKEN", "plain-token")]);
        let http = FakeHttp::new();
        let lookup = test_lookup(&vars);
        let files = no_secret_files();
        match GiteaClient::with_base_resolved(
            &http,
            "http://example.com",
            BASE,
            &[],
            &lookup,
            &files,
        ) {
            Ok(_) => panic!("construction with a non-loopback http base must fail"),
            Err(err) => {
                assert_eq!(err.kind(), ErrorKind::InvalidInput);
                assert!(err.context.contains("loopback"));
                assert!(!err.context.contains("[instances]"));
            }
        }
    }

    #[test]
    fn with_base_resolved_names_the_alias_vars_when_the_alias_is_unset() {
        // An alias matches the URL but its variables are unset and
        // nothing else is configured: the error names the alias
        // variables, not the plain ones — those belong to the
        // default instance.
        let vars = test_vars(&[]);
        let instances = vec![(
            "example".to_string(),
            "https://gitea.example.org".to_string(),
        )];
        let http = FakeHttp::new();
        let lookup = test_lookup(&vars);
        let files = no_secret_files();
        match GiteaClient::with_base_resolved(
            &http,
            "https://gitea.example.org",
            BASE,
            &instances,
            &lookup,
            &files,
        ) {
            Ok(_) => panic!("construction without any token must fail"),
            Err(err) => {
                assert_eq!(err.kind(), ErrorKind::ConfigError);
                assert!(err.context.contains("GEECTL_GITEA_TOKEN_EXAMPLE"));
            }
        }
    }

    #[test]
    fn with_base_resolved_names_the_plain_vars_for_the_default_url() {
        // Nothing configured for the default URL: the error is
        // `with_base`'s plain-vars message — assert it exactly,
        // since the alias and unmatched messages also contain
        // the `GEECTL_GITEA_TOKEN` substring.
        let vars = test_vars(&[]);
        let http = FakeHttp::new();
        let lookup = test_lookup(&vars);
        let files = no_secret_files();
        match GiteaClient::with_base_resolved(&http, BASE, BASE, &[], &lookup, &files) {
            Ok(_) => panic!("construction without any token must fail"),
            Err(err) => {
                assert_eq!(err.kind(), ErrorKind::ConfigError);
                assert!(err.context.contains(
                    "no Gitea token configured (GEECTL_GITEA_TOKEN or GEECTL_GITEA_TOKEN_FILE)"
                ));
            }
        }
    }

    #[test]
    fn with_base_resolved_names_both_var_sets_when_the_alias_is_the_default_url() {
        // The URL is both the configured default and an aliased
        // instance, and nothing is set anywhere: either variable
        // set would fix it, so the error names both.
        let vars = test_vars(&[]);
        let instances = vec![("myinstance".to_string(), BASE.to_string())];
        let http = FakeHttp::new();
        let lookup = test_lookup(&vars);
        let files = no_secret_files();
        match GiteaClient::with_base_resolved(&http, BASE, BASE, &instances, &lookup, &files) {
            Ok(_) => panic!("construction without any token must fail"),
            Err(err) => {
                assert_eq!(err.kind(), ErrorKind::ConfigError);
                assert!(err.context.contains("GEECTL_GITEA_TOKEN_MYINSTANCE"));
                assert!(err
                    .context
                    .contains("the plain GEECTL_GITEA_TOKEN (or GEECTL_GITEA_TOKEN_FILE)"));
            }
        }
    }

    #[test]
    fn repo_get_parses_a_repo_and_ignores_unknown_fields() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/pool/systemd",
            &repo_json("pool", "systemd", None),
        );
        let repo = client(&http).repo_get("pool", "systemd").unwrap();
        assert_eq!(repo.owner.login, "pool");
        assert_eq!(repo.name, "systemd");
        assert_eq!(repo.full_name, "pool/systemd");
        assert_eq!(repo.parent, None);
        assert_eq!(repo.default_branch, "factory");
        assert_eq!(repo.clone_url, "https://src.opensuse.org/pool/systemd.git");
        assert_eq!(repo.ssh_url, "gitea@src.opensuse.org:pool/systemd.git");
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/repos/pool/systemd".to_string(),
                true
            )]
        );
    }

    #[test]
    fn repo_get_parses_a_fork_parent() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/fbui/systemd",
            &repo_json("fbui", "systemd", Some(("pool", "systemd"))),
        );
        let repo = client(&http).repo_get("fbui", "systemd").unwrap();
        assert_eq!(
            repo.parent,
            Some(GiteaParent {
                owner: GiteaOwner {
                    login: "pool".to_string()
                },
                name: "systemd".to_string(),
                full_name: "pool/systemd".to_string(),
            })
        );
    }

    #[test]
    fn repo_get_missing_field_is_a_server_error() {
        let body = r#"{"owner": {"login": "pool"}, "name": "systemd", "full_name": "pool/systemd", "parent": null, "clone_url": "x", "ssh_url": "y"}"#;
        let http =
            FakeHttp::new().with_get("https://src.opensuse.org/api/v1/repos/pool/systemd", body);
        let err = client(&http).repo_get("pool", "systemd").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn repo_get_not_found_propagates() {
        let http = FakeHttp::new().with_get_error(
            "https://src.opensuse.org/api/v1/repos/pool/systemd",
            ErrorKind::NotFound,
        );
        let err = client(&http).repo_get("pool", "systemd").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn repo_get_validates_names_before_any_call() {
        let http = FakeHttp::new();
        let err = client(&http).repo_get("up/stream", "foo").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn repo_get_rejects_a_response_for_another_repo() {
        // The read is a guard, not just a parse: a body naming
        // another repo fails here instead of being verified or
        // deleted as the requested one.
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/pool/systemd",
            &repo_json("pool", "libstrophe", None),
        );
        let err = client(&http).repo_get("pool", "systemd").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        // Case alone is not a mismatch: Gitea logins are
        // case-insensitive, so the comparison canonicalizes case.
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/pool/systemd",
            &repo_json("Pool", "systemd", None),
        );
        let repo = client(&http).repo_get("pool", "systemd").unwrap();
        assert_eq!(repo.full_name, "Pool/systemd");
    }

    #[test]
    fn repo_list_own_uses_user_repos_and_paginates() {
        // Page 1 full (50 repos) fetches page 2; page 2 short (1
        // repo) ends the list. Order is the server's order.
        let page1 = fifty_repos("geectl");
        let page2 = vec![repo_json("geectl", "repo-50", None)];
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/user/repos?page=1&limit=50",
                &repo_list_json(&page1),
            )
            .with_get(
                "https://src.opensuse.org/api/v1/user/repos?page=2&limit=50",
                &repo_list_json(&page2),
            );
        let repos = client(&http).repo_list(None, None).unwrap();
        assert_eq!(repos.len(), 51);
        assert_eq!(repos[0].full_name, "geectl/repo-00");
        assert_eq!(repos[50].full_name, "geectl/repo-50");
        assert_eq!(
            http.get_calls(),
            vec![
                (
                    "https://src.opensuse.org/api/v1/user/repos?page=1&limit=50".to_string(),
                    true
                ),
                (
                    "https://src.opensuse.org/api/v1/user/repos?page=2&limit=50".to_string(),
                    true
                ),
            ]
        );
    }

    #[test]
    fn repo_list_org_and_user_forms_and_both() {
        // The org form: pool's repos, the real mirror org.
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/orgs/pool/repos?page=1&limit=50",
            &repo_list_json(&[repo_json("pool", "systemd", None)]),
        );
        let repos = client(&http).repo_list(None, Some("pool")).unwrap();
        assert_eq!(
            repos
                .iter()
                .map(|r| r.full_name.as_str())
                .collect::<Vec<_>>(),
            vec!["pool/systemd"]
        );
        // The user form.
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/users/fbui/repos?page=1&limit=50",
            &repo_list_json(&[repo_json("fbui", "systemd", Some(("pool", "systemd")))]),
        );
        let repos = client(&http).repo_list(Some("fbui"), None).unwrap();
        assert_eq!(
            repos
                .iter()
                .map(|r| r.full_name.as_str())
                .collect::<Vec<_>>(),
            vec!["fbui/systemd"]
        );
        // Both: the user's repos first, then the org's — osc's
        // `repo list --user --org` order (osc
        // `behave/features/git-repo-list.feature`, "List repos
        // owned by a user and an organization").
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/users/Admin/repos?page=1&limit=50",
                &repo_list_json(&[repo_json(
                    "Admin",
                    "test-GitPkgA",
                    Some(("pool", "test-GitPkgA")),
                )]),
            )
            .with_get(
                "https://src.opensuse.org/api/v1/orgs/pool/repos?page=1&limit=50",
                &repo_list_json(&[
                    repo_json("pool", "new_package", None),
                    repo_json("pool", "test-GitPkgA", None),
                ]),
            );
        let repos = client(&http)
            .repo_list(Some("Admin"), Some("pool"))
            .unwrap();
        assert_eq!(
            repos
                .iter()
                .map(|r| r.full_name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Admin/test-GitPkgA",
                "pool/new_package",
                "pool/test-GitPkgA"
            ]
        );
    }

    #[test]
    fn repo_list_both_form_dedupes_the_same_account() {
        // An org is a user in Gitea: naming the same account as
        // owner and org lists the same repos twice, so the both
        // form returns each repo once.
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/users/pool/repos?page=1&limit=50",
                &repo_list_json(&[repo_json("pool", "systemd", None)]),
            )
            .with_get(
                "https://src.opensuse.org/api/v1/orgs/pool/repos?page=1&limit=50",
                &repo_list_json(&[repo_json("pool", "systemd", None)]),
            );
        let repos = client(&http).repo_list(Some("pool"), Some("pool")).unwrap();
        assert_eq!(
            repos
                .iter()
                .map(|r| r.full_name.as_str())
                .collect::<Vec<_>>(),
            vec!["pool/systemd"]
        );
    }

    #[test]
    fn repo_list_both_form_dedupes_mixed_case_names() {
        // The repo-identity rule is case-insensitive: the user
        // list naming the account `Pool` and the org list naming
        // it `pool` hold the same repo, so the both form returns
        // it once, keeping the first (user) spelling.
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/users/Pool/repos?page=1&limit=50",
                &repo_list_json(&[repo_json("Pool", "systemd", None)]),
            )
            .with_get(
                "https://src.opensuse.org/api/v1/orgs/pool/repos?page=1&limit=50",
                &repo_list_json(&[repo_json("pool", "systemd", None)]),
            );
        let repos = client(&http).repo_list(Some("Pool"), Some("pool")).unwrap();
        assert_eq!(
            repos
                .iter()
                .map(|r| r.full_name.as_str())
                .collect::<Vec<_>>(),
            vec!["Pool/systemd"]
        );
    }

    #[test]
    fn repo_list_bound_fails_instead_of_truncating() {
        // Ten full pages (500 repos) and page 11 still has one
        // more: the call fails naming the scope, instead
        // of silently returning the first 500.
        let page = repo_list_json(&fifty_repos("pool"));
        let mut http = FakeHttp::new();
        for n in 1..=10 {
            http = http.with_get(
                &format!("https://src.opensuse.org/api/v1/orgs/pool/repos?page={n}&limit=50"),
                &page,
            );
        }
        http = http.with_get(
            "https://src.opensuse.org/api/v1/orgs/pool/repos?page=11&limit=50",
            &repo_list_json(&[repo_json("pool", "repo-500", None)]),
        );
        let err = client(&http).repo_list(None, Some("pool")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(
            err.to_string().contains("organization pool"),
            "the bound error must name the organization; got {err}"
        );
        assert!(
            err.to_string()
                .contains("the list is bounded at 500 entries"),
            "an org list has no narrower filter, so the error must say the list is bounded; got {err}"
        );
        assert!(
            !err.to_string().contains("narrow"),
            "the error must not promise a narrower scope that does not exist; got {err}"
        );
    }

    #[test]
    fn repo_list_own_bound_names_the_narrower_form() {
        // The own form is the one list that CAN be narrowed: the
        // token owner's repos are reachable one account at a time,
        // so the bound error names that remedy instead of claiming
        // the list is merely bounded.
        let page = repo_list_json(&fifty_repos("geectl"));
        let mut http = FakeHttp::new();
        for n in 1..=10 {
            http = http.with_get(
                &format!("https://src.opensuse.org/api/v1/user/repos?page={n}&limit=50"),
                &page,
            );
        }
        http = http.with_get(
            "https://src.opensuse.org/api/v1/user/repos?page=11&limit=50",
            &repo_list_json(&[repo_json("geectl", "repo-500", None)]),
        );
        let err = client(&http).repo_list(None, None).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(
            err.to_string()
                .contains("list a single user or organization instead"),
            "the own-form bound error must name the narrower form; got {err}"
        );
    }

    #[test]
    fn repo_list_exactly_at_the_bound_is_returned() {
        // Ten full pages (500 repos) and page 11 is empty: the
        // list is exactly at the bound, not past it, so it is
        // returned — the page-11 read is what proves it.
        let page = repo_list_json(&fifty_repos("pool"));
        let mut http = FakeHttp::new();
        for n in 1..=10 {
            http = http.with_get(
                &format!("https://src.opensuse.org/api/v1/orgs/pool/repos?page={n}&limit=50"),
                &page,
            );
        }
        http = http.with_get(
            "https://src.opensuse.org/api/v1/orgs/pool/repos?page=11&limit=50",
            "[]",
        );
        let repos = client(&http).repo_list(None, Some("pool")).unwrap();
        assert_eq!(repos.len(), 500);
    }

    #[test]
    fn repo_list_validates_names_before_any_call() {
        let http = FakeHttp::new();
        let err = client(&http)
            .repo_list(Some("up/stream"), None)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let err = client(&http).repo_list(None, Some("")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn repo_create_posts_and_parses_the_created_repo() {
        let http = FakeHttp::new().with_post(
            "https://src.opensuse.org/api/v1/user/repos",
            &repo_json("geectl", "new-package", None),
        );
        let options = RepoCreateOptions {
            private: false,
            description: Some("sources for a new package".to_string()),
            default_branch: Some("factory".to_string()),
        };
        let created = client(&http)
            .repo_create("new-package", &options, &DryRun::new(false))
            .unwrap()
            .expect("a non-dry-run create returns the repo");
        assert_eq!(created.full_name, "geectl/new-package");
        assert_eq!(created.default_branch, "factory");
        assert_eq!(
            http.post_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/user/repos".to_string(),
                r#"{"name":"new-package","private":false,"description":"sources for a new package","default_branch":"factory"}"#.to_string(),
                true
            )]
        );
    }

    #[test]
    fn repo_create_posts_the_default_options_body() {
        // Default options: only the name and the privacy flag are
        // sent — `description` and `default_branch` are omitted
        // when unset, never sent as null.
        let http = FakeHttp::new().with_post(
            "https://src.opensuse.org/api/v1/user/repos",
            &repo_json("geectl", "new-package", None),
        );
        let created = client(&http)
            .repo_create(
                "new-package",
                &RepoCreateOptions::default(),
                &DryRun::new(false),
            )
            .unwrap()
            .expect("a non-dry-run create returns the repo");
        assert_eq!(created.full_name, "geectl/new-package");
        assert_eq!(
            http.post_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/user/repos".to_string(),
                r#"{"name":"new-package","private":false}"#.to_string(),
                true
            )]
        );
    }

    #[test]
    fn repo_create_verifies_the_requested_options() {
        // A private create answered with a private repo and the
        // requested default branch succeeds.
        let options = RepoCreateOptions {
            private: true,
            description: None,
            default_branch: Some("factory".to_string()),
        };
        let http = FakeHttp::new().with_post(
            "https://src.opensuse.org/api/v1/user/repos",
            &repo_json_branch("geectl", "new-package", None, "factory", true),
        );
        let created = client(&http)
            .repo_create("new-package", &options, &DryRun::new(false))
            .unwrap()
            .expect("a matching create returns the repo");
        assert!(created.private);
        assert_eq!(created.default_branch, "factory");
        // A public answer to a private create is a server error,
        // not a success.
        let http = FakeHttp::new().with_post(
            "https://src.opensuse.org/api/v1/user/repos",
            &repo_json_branch("geectl", "new-package", None, "factory", false),
        );
        let err = client(&http)
            .repo_create("new-package", &options, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        // And a different default branch than requested.
        let http = FakeHttp::new().with_post(
            "https://src.opensuse.org/api/v1/user/repos",
            &repo_json_branch("geectl", "new-package", None, "main", true),
        );
        let err = client(&http)
            .repo_create("new-package", &options, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn repo_create_dry_run_records_plan_and_creates_nothing() {
        let http = FakeHttp::new();
        let dry_run = DryRun::new(true);
        let result = client(&http)
            .repo_create("new-package", &RepoCreateOptions::default(), &dry_run)
            .unwrap();
        assert_eq!(result, None);
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: create repository new-package".to_string()]
        );
        assert!(http.post_calls().is_empty());
        // A private create names the privacy in the plan.
        let options = RepoCreateOptions {
            private: true,
            ..Default::default()
        };
        let dry_run = DryRun::new(true);
        let result = client(&http)
            .repo_create("new-package", &options, &dry_run)
            .unwrap();
        assert_eq!(result, None);
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: create repository new-package (private)".to_string()]
        );
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn repo_create_conflict_is_already_exists() {
        let http = FakeHttp::new().with_post_error(
            "https://src.opensuse.org/api/v1/user/repos",
            ErrorKind::AlreadyExists,
        );
        let err = client(&http)
            .repo_create(
                "systemd",
                &RepoCreateOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
    }

    #[test]
    fn repo_create_response_naming_another_repo_is_a_server_error() {
        let http = FakeHttp::new().with_post(
            "https://src.opensuse.org/api/v1/user/repos",
            &repo_json("geectl", "other-name", None),
        );
        let err = client(&http)
            .repo_create(
                "new-package",
                &RepoCreateOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn repo_create_accepts_the_name_in_a_different_case() {
        // The server preserves the requested name's case in
        // practice, but the read path compares case-insensitively
        // and the create check must match it: an answer for
        // `New-Package` is the `new-package` that was asked for.
        let http = FakeHttp::new().with_post(
            "https://src.opensuse.org/api/v1/user/repos",
            &repo_json("geectl", "New-Package", None),
        );
        let created = client(&http)
            .repo_create(
                "new-package",
                &RepoCreateOptions::default(),
                &DryRun::new(false),
            )
            .unwrap()
            .expect("a non-dry-run create returns the repo");
        assert_eq!(created.name, "New-Package");
    }

    #[test]
    fn repo_create_validates_names_before_any_call() {
        let http = FakeHttp::new();
        let err = client(&http)
            .repo_create(
                "up/stream",
                &RepoCreateOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let options = RepoCreateOptions {
            default_branch: Some("a#b".to_string()),
            ..Default::default()
        };
        let err = client(&http)
            .repo_create("new-package", &options, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn branch_list_parses_branches_and_paginates() {
        // pool/systemd's real branches: factory, slfo-main,
        // slfo-1.0 … — the first page is short, so the list ends
        // after one page.
        let sha = "e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa";
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/pool/systemd/branches?page=1&limit=50",
            &branch_list_json(&[
                branch_json("factory", sha, "OBS bridge sync", false),
                branch_json("slfo-main", sha, "OBS bridge sync", true),
                branch_json("slfo-1.0", sha, "OBS bridge sync", false),
            ]),
        );
        let branches = client(&http).branch_list("pool", "systemd").unwrap();
        assert_eq!(
            branches.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
            vec!["factory", "slfo-main", "slfo-1.0"]
        );
        assert_eq!(branches[0].commit.id, sha);
        assert_eq!(branches[0].commit.message, "OBS bridge sync");
        assert!(!branches[0].protected);
        assert!(branches[1].protected);
    }

    #[test]
    fn branch_list_null_page_is_an_empty_list() {
        // Gitea answers a JSON `null` body for an empty list —
        // verified live on the branches endpoint of the empty
        // `pool/systemd` fixture repo. The list is empty, not an
        // error.
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/pool/systemd/branches?page=1&limit=50",
            "null",
        );
        let branches = client(&http).branch_list("pool", "systemd").unwrap();
        assert!(branches.is_empty());
    }
    #[test]
    fn pr_get_round_trip() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/openSUSE/geectl/pulls/122",
            &pr_json(122, "open", false),
        );
        let pr = client(&http).pr_get("openSUSE", "geectl", 122).unwrap();
        assert_eq!(pr.number, 122);
        assert_eq!(pr.title, "change_review_state");
        assert_eq!(pr.body.as_deref(), Some("A test PR body."));
        assert_eq!(pr.state, "open");
        assert!(!pr.draft);
        assert!(!pr.merged);
        assert_eq!(pr.mergeable, Some(true));
        assert_eq!(pr.user.login, "scarabeusiv");
        assert_eq!(
            pr.labels
                .iter()
                .map(|l| l.name.as_str())
                .collect::<Vec<_>>(),
            vec!["bug"]
        );
        assert_eq!(pr.head.label, "scarabeusiv:change-review-state");
        assert_eq!(pr.head.branch, "change-review-state");
        assert_eq!(
            pr.head.repo.as_ref().map(|r| r.full_name.as_str()),
            Some("scarabeusiv/geectl")
        );
        assert_eq!(pr.base.branch, "main");
        assert_eq!(
            pr.base.repo.as_ref().map(|r| r.full_name.as_str()),
            Some("openSUSE/geectl")
        );
    }

    #[test]
    fn pr_get_wrong_number_is_a_server_error() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/openSUSE/geectl/pulls/122",
            &pr_json(123, "open", false),
        );
        let err = client(&http)
            .pr_get("openSUSE", "geectl", 122)
            .expect_err("a PR body naming another number must fail");
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn pr_get_missing_fields_are_a_server_error() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/openSUSE/geectl/pulls/122",
            "{}",
        );
        let err = client(&http)
            .pr_get("openSUSE", "geectl", 122)
            .expect_err("a PR body missing required fields must fail");
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn pr_get_tolerates_null_body_mergeable_and_deleted_fork_head() {
        // A PR with no body, mergeability not computed, and the
        // head fork deleted: the body and mergeable read as `None`,
        // the head repo as `None` — not a parse failure.
        let body = r#"{"number": 122, "title": "change_review_state", "body": null, "state": "open", "draft": false, "merged": false, "mergeable": null, "user": {"login": "scarabeusiv"}, "labels": [], "head": {"label": "change-review-state", "ref": "change-review-state", "sha": "e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa", "repo": null}, "base": {"label": "main", "ref": "main", "sha": "e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa", "repo": {"full_name": "openSUSE/geectl"}}}"#;
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/openSUSE/geectl/pulls/122",
            body,
        );
        let pr = client(&http).pr_get("openSUSE", "geectl", 122).unwrap();
        assert_eq!(pr.body, None);
        assert_eq!(pr.mergeable, None);
        assert_eq!(pr.head.repo, None);
    }

    #[test]
    fn pr_get_zero_is_an_invalid_input() {
        // The guard runs before any server read: no canned
        // response exists, so a request would fail differently.
        let http = FakeHttp::new();
        let err = client(&http)
            .pr_get("openSUSE", "geectl", 0)
            .expect_err("pull request number 0 must fail");
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn pr_list_parses_prs_and_sends_the_state() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/openSUSE/geectl/pulls?state=open&page=1&limit=50",
            &format!("[{}, {}]", pr_json(122, "open", false), pr_json(123, "open", false)),
        );
        let prs = client(&http)
            .pr_list("openSUSE", "geectl", GiteaPullRequestState::Open)
            .unwrap();
        assert_eq!(
            prs.iter().map(|p| p.number).collect::<Vec<_>>(),
            vec![122, 123]
        );
        assert_eq!(prs[0].title, "change_review_state");
    }

    #[test]
    fn pr_list_null_page_is_an_empty_list() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/openSUSE/geectl/pulls?state=all&page=1&limit=50",
            "null",
        );
        let prs = client(&http)
            .pr_list("openSUSE", "geectl", GiteaPullRequestState::All)
            .unwrap();
        assert!(prs.is_empty());
    }

    #[test]
    fn pr_search_parses_issue_items() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/issues/search?type=pulls&state=open&page=1&limit=50",
            &format!("[{}]", issue_json(122, "open", false)),
        );
        let items = client(&http)
            .pr_search(GiteaPullRequestState::Open)
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].number, 122);
        assert_eq!(items[0].title, "change_review_state");
        assert_eq!(items[0].state, "open");
        assert_eq!(items[0].user.login, "scarabeusiv");
        assert_eq!(items[0].repository.full_name, "openSUSE/geectl");
        assert_eq!(
            items[0].pull_request.as_ref().map(|p| p.merged),
            Some(false)
        );
    }

    #[test]
    fn pr_search_null_page_is_an_empty_list() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/issues/search?type=pulls&state=closed&page=1&limit=50",
            "null",
        );
        let items = client(&http)
            .pr_search(GiteaPullRequestState::Closed)
            .unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn pr_reviews_parses_reviews() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/openSUSE/geectl/pulls/122/reviews?page=1&limit=50",
            &format!(
                "[{}, {}]",
                review_json("plusky", "APPROVED", "Looks good."),
                review_json("mimi1vx", "COMMENT", "One nit.")
            ),
        );
        let reviews = client(&http).pr_reviews("openSUSE", "geectl", 122).unwrap();
        assert_eq!(reviews.len(), 2);
        assert_eq!(reviews[0].user.login, "plusky");
        assert_eq!(reviews[0].state, "APPROVED");
        assert_eq!(reviews[0].body.as_deref(), Some("Looks good."));
        assert!(!reviews[0].dismissed);
        assert_eq!(
            reviews[0].submitted_at.as_deref(),
            Some("2026-11-22T19:52:58Z")
        );
        assert_eq!(reviews[1].state, "COMMENT");
    }

    #[test]
    fn pr_reviews_missing_state_is_a_server_error() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/openSUSE/geectl/pulls/122/reviews?page=1&limit=50",
            r#"[{"user": {"login": "plusky"}, "body": "Looks good."}]"#,
        );
        let err = client(&http)
            .pr_reviews("openSUSE", "geectl", 122)
            .expect_err("a review missing its state must fail");
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn parse_gitea_repo_splits_and_validates() {
        assert_eq!(
            parse_gitea_repo("pool/systemd").unwrap(),
            ("pool".to_string(), "systemd".to_string())
        );
        for invalid in [
            "pool",
            "pool/",
            "/systemd",
            "a/b/c",
            "pool/systemd ",
            "",
            "pool//systemd",
        ] {
            assert!(
                parse_gitea_repo(invalid).is_err(),
                "{invalid:?} must not parse as a repository"
            );
        }
    }

    #[test]
    fn branch_list_bound_fails_instead_of_truncating() {
        let page = branch_list_json(
            &(0..50)
                .map(|n| {
                    branch_json(
                        &format!("branch-{n:02}"),
                        "e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa",
                        "x",
                        false,
                    )
                })
                .collect::<Vec<_>>(),
        );
        let mut http = FakeHttp::new();
        for n in 1..=10 {
            http = http.with_get(
                &format!(
                    "https://src.opensuse.org/api/v1/repos/pool/systemd/branches?page={n}&limit=50"
                ),
                &page,
            );
        }
        http = http.with_get(
            "https://src.opensuse.org/api/v1/repos/pool/systemd/branches?page=11&limit=50",
            &branch_list_json(&[branch_json(
                "branch-500",
                "e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa",
                "x",
                false,
            )]),
        );
        let err = client(&http).branch_list("pool", "systemd").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(
            err.to_string().contains("pool/systemd"),
            "the bound error must name the repo; got {err}"
        );
        assert!(
            err.to_string()
                .contains("the list is bounded at 500 entries"),
            "a branch list has no narrower filter, so the error must say the list is bounded; got {err}"
        );
        assert!(
            !err.to_string().contains("narrow"),
            "the error must not promise a narrower scope that does not exist; got {err}"
        );
    }

    #[test]
    fn branch_list_exactly_at_the_bound_is_returned() {
        // Ten full pages (500 branches) and page 11 is empty: the
        // list is exactly at the bound, so it is returned.
        let page = branch_list_json(
            &(0..50)
                .map(|n| {
                    branch_json(
                        &format!("branch-{n:02}"),
                        "e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa",
                        "x",
                        false,
                    )
                })
                .collect::<Vec<_>>(),
        );
        let mut http = FakeHttp::new();
        for n in 1..=10 {
            http = http.with_get(
                &format!(
                    "https://src.opensuse.org/api/v1/repos/pool/systemd/branches?page={n}&limit=50"
                ),
                &page,
            );
        }
        http = http.with_get(
            "https://src.opensuse.org/api/v1/repos/pool/systemd/branches?page=11&limit=50",
            "[]",
        );
        let branches = client(&http).branch_list("pool", "systemd").unwrap();
        assert_eq!(branches.len(), 500);
    }

    #[test]
    fn branch_get_parses_a_branch() {
        let sha = "e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa";
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/pool/systemd/branches/factory",
            &branch_json("factory", sha, "OBS bridge sync", false),
        );
        let branch = client(&http)
            .branch_get("pool", "systemd", "factory")
            .unwrap();
        assert_eq!(branch.name, "factory");
        assert_eq!(branch.commit.id, sha);
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/branches/factory".to_string(),
                true
            )]
        );
    }

    #[test]
    fn branch_without_protected_fails_to_parse() {
        // `protected` is required: a server that omits it must
        // fail the parse instead of reading the branch as
        // unprotected. Unknown fields stay tolerated. The failure
        // must name the missing field, so a different parse
        // breakage in the fixture cannot pass this test.
        let body = r#"{"name": "factory", "commit": {"id": "e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa", "message": "OBS bridge sync"}, "future_field": "tolerated"}"#;
        let err = serde_json::from_str::<GiteaBranch>(body)
            .expect_err("a branch without `protected` must fail to parse");
        assert!(
            err.to_string().contains("protected"),
            "the parse failure must name the missing `protected` field; got {err}"
        );
    }

    #[test]
    fn branch_get_encodes_a_slashed_branch_name() {
        let sha = "e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa";
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/pool/systemd/branches/feature%2Fx",
            &branch_json("feature/x", sha, "work in progress", false),
        );
        let branch = client(&http)
            .branch_get("pool", "systemd", "feature/x")
            .unwrap();
        assert_eq!(branch.name, "feature/x");
        assert_eq!(branch.commit.id, sha);
    }

    #[test]
    fn branch_get_missing_branch_is_not_found() {
        let http = FakeHttp::new().with_get_error(
            "https://src.opensuse.org/api/v1/repos/pool/systemd/branches/leap-16.0",
            ErrorKind::NotFound,
        );
        // The real layout: pool/systemd has no leap-16.0 branch —
        // it lives on openSUSE/Leap.
        let err = client(&http)
            .branch_get("pool", "systemd", "leap-16.0")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn branch_get_validates_names_before_any_call() {
        let http = FakeHttp::new();
        let err = client(&http)
            .branch_get("pool", "systemd", "a#b")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let err = client(&http)
            .branch_get("up/stream", "systemd", "factory")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn branch_get_rejects_a_response_for_another_branch() {
        let sha = "e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa";
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/pool/systemd/branches/factory",
            &branch_json("slfo-main", sha, "OBS bridge sync", false),
        );
        let err = client(&http)
            .branch_get("pool", "systemd", "factory")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn branch_head_returns_the_commit_id() {
        let sha = "e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa";
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/pool/systemd/branches/factory",
            &branch_json("factory", sha, "OBS bridge sync", false),
        );
        let head = client(&http)
            .branch_head("pool", "systemd", "factory")
            .unwrap();
        assert_eq!(head, sha);
    }

    #[test]
    fn repo_delete_dry_run_records_plan_and_deletes_nothing() {
        let http = FakeHttp::new();
        let dry_run = DryRun::new(true);
        // Planning a delete needs no force: the refusal guards the
        // live delete, not the plan.
        let result = client(&http)
            .repo_delete("pool", "systemd", false, &dry_run)
            .unwrap();
        assert_eq!(result, None);
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: delete repository pool/systemd".to_string()]
        );
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn repo_delete_refuses_without_force_and_reads_nothing() {
        // The confirmation is client-side: without force the delete
        // is refused before any server read, so a mistaken call
        // cannot delete the wrong generation — and the refusal
        // proves the pre-read never ran.
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/pool/systemd",
            &repo_json("pool", "systemd", None),
        );
        let err = client(&http)
            .repo_delete("pool", "systemd", false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn repo_delete_gets_first_then_deletes_and_returns_the_name() {
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/repos/pool/systemd",
                &repo_json("pool", "systemd", None),
            )
            .with_delete("https://src.opensuse.org/api/v1/repos/pool/systemd", "");
        let result = client(&http)
            .repo_delete("pool", "systemd", true, &DryRun::new(false))
            .unwrap();
        assert_eq!(result, Some("systemd".to_string()));
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/repos/pool/systemd".to_string(),
                true
            )]
        );
        assert_eq!(
            http.delete_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/repos/pool/systemd".to_string(),
                true
            )]
        );
    }

    #[test]
    fn repo_delete_missing_repo_fails_before_delete() {
        let http = FakeHttp::new().with_get_error(
            "https://src.opensuse.org/api/v1/repos/pool/systemd",
            ErrorKind::NotFound,
        );
        let err = client(&http)
            .repo_delete("pool", "systemd", true, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn fork_repo_dry_run_records_plan_and_forks_nothing() {
        let http = FakeHttp::new();
        let dry_run = DryRun::new(true);
        let options = ForkRepoOptions {
            new_name: None,
            organization: Some("rpm".to_string()),
        };
        let result = client(&http)
            .fork_repo("pool", "systemd", &options, &dry_run)
            .unwrap();
        assert_eq!(result, None);
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: fork repository pool/systemd to rpm/systemd".to_string()]
        );
        assert!(http.get_calls().is_empty());
        assert!(http.post_calls().is_empty());

        let dry_run = DryRun::new(true);
        let result = client(&http)
            .fork_repo("pool", "systemd", &ForkRepoOptions::default(), &dry_run)
            .unwrap();
        assert_eq!(result, None);
        assert_eq!(
            dry_run.plan(),
            vec![
                "[dry-run] would: fork repository pool/systemd to your account/systemd".to_string()
            ]
        );
    }

    /// The fixture for a fork creation: the pre-check 404s, the
    /// POST answers the fork, the re-read answers the same fork.
    fn fork_fixture(
        http: FakeHttp,
        source_owner: &str,
        source_repo: &str,
        target_url: &str,
        fork_body: &str,
    ) -> FakeHttp {
        http.with_get_error_sequence(target_url, &[ErrorKind::NotFound])
            .with_get(target_url, fork_body)
            .with_post(
                &format!("{BASE}/api/v1/repos/{source_owner}/{source_repo}/forks"),
                fork_body,
            )
    }

    #[test]
    fn fork_repo_creates_and_verifies_by_re_read() {
        let fork_body = repo_json("fbui", "systemd", Some(("pool", "systemd")));
        let http = fork_fixture(
            FakeHttp::new().with_get(
                "https://src.opensuse.org/api/v1/user",
                r#"{"login": "fbui", "future_field": 1}"#,
            ),
            "pool",
            "systemd",
            "https://src.opensuse.org/api/v1/repos/fbui/systemd",
            &fork_body,
        );
        let fork = client(&http)
            .fork_repo(
                "pool",
                "systemd",
                &ForkRepoOptions::default(),
                &DryRun::new(false),
            )
            .unwrap()
            .unwrap();
        assert_eq!(fork.full_name, "fbui/systemd");
        assert_eq!(fork.parent.as_ref().unwrap().full_name, "pool/systemd");
        // The POST body omits the name and organization when unset.
        // The POST is authenticated with the client's Bearer: the
        // third tuple element is the auth flag, and it must be true
        // — a fork POST without it would authenticate as whoever the
        // HTTP client's own provider resolves, not as this client.
        assert_eq!(
            http.post_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/forks".to_string(),
                "{}".to_string(),
                true
            )]
        );
        // The target owner came from GET /api/v1/user, then the
        // pre-check, then the re-read.
        assert_eq!(
            http.get_calls(),
            vec![
                ("https://src.opensuse.org/api/v1/user".to_string(), true),
                (
                    "https://src.opensuse.org/api/v1/repos/fbui/systemd".to_string(),
                    true
                ),
                (
                    "https://src.opensuse.org/api/v1/repos/fbui/systemd".to_string(),
                    true
                ),
            ]
        );
    }

    #[test]
    fn fork_repo_verify_accepts_a_parent_whose_case_differs() {
        // The re-read reports the parent as Pool/Systemd while the
        // source was requested as pool/systemd: the fork was created
        // fine, so the verification accepts it instead of failing
        // with a ServerError.
        let fork_body = repo_json("fbui", "systemd", Some(("Pool", "Systemd")));
        let http = fork_fixture(
            FakeHttp::new().with_get(
                "https://src.opensuse.org/api/v1/user",
                r#"{"login": "fbui", "future_field": 1}"#,
            ),
            "pool",
            "systemd",
            "https://src.opensuse.org/api/v1/repos/fbui/systemd",
            &fork_body,
        );
        let fork = client(&http)
            .fork_repo(
                "pool",
                "systemd",
                &ForkRepoOptions::default(),
                &DryRun::new(false),
            )
            .unwrap()
            .unwrap();
        assert_eq!(fork.full_name, "fbui/systemd");
        assert_eq!(fork.parent.as_ref().unwrap().full_name, "Pool/Systemd");
    }

    #[test]
    fn fork_repo_sends_name_and_organization_when_given() {
        let fork_body = repo_json("rpm", "systemd-slfo", Some(("pool", "systemd")));
        let http = FakeHttp::new()
            .with_get_error_sequence(
                "https://src.opensuse.org/api/v1/repos/rpm/systemd-slfo",
                &[ErrorKind::NotFound],
            )
            .with_get(
                "https://src.opensuse.org/api/v1/repos/rpm/systemd-slfo",
                &fork_body,
            )
            .with_post(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/forks",
                &fork_body,
            );
        let options = ForkRepoOptions {
            new_name: Some("systemd-slfo".to_string()),
            organization: Some("rpm".to_string()),
        };
        let fork = client(&http)
            .fork_repo("pool", "systemd", &options, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert_eq!(fork.full_name, "rpm/systemd-slfo");
        assert_eq!(
            http.post_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/forks".to_string(),
                r#"{"name":"systemd-slfo","organization":"rpm"}"#.to_string(),
                true
            )]
        );
        // An explicit organization means no /api/v1/user read.
        assert!(http
            .get_calls()
            .iter()
            .all(|(url, _)| !url.ends_with("/api/v1/user")));
    }

    #[test]
    fn fork_repo_reuses_the_existing_fork() {
        let fork_body = repo_json("fbui", "systemd", Some(("pool", "systemd")));
        let (debug, lines) = Debug::captured(true, false);
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/user",
                r#"{"login": "fbui"}"#,
            )
            .with_get(
                "https://src.opensuse.org/api/v1/repos/fbui/systemd",
                &fork_body,
            );
        let fork = client(&http)
            .with_debug(debug)
            .fork_repo(
                "pool",
                "systemd",
                &ForkRepoOptions::default(),
                &DryRun::new(false),
            )
            .unwrap()
            .unwrap();
        assert_eq!(fork.full_name, "fbui/systemd");
        assert!(http.post_calls().is_empty());
        assert_eq!(
            lines.lock().unwrap().clone(),
            vec!["decision: reusing existing fork fbui/systemd for pool/systemd".to_string()]
        );
    }

    #[test]
    fn fork_repo_reuses_the_existing_fork_when_the_parent_case_differs() {
        // The parent is reported as Pool/Systemd while the source was
        // requested as pool/systemd: repo identity is case-insensitive
        // (like parse_repository), so the fork is reused, not refused.
        let fork_body = repo_json("fbui", "systemd", Some(("Pool", "Systemd")));
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/user",
                r#"{"login": "fbui"}"#,
            )
            .with_get(
                "https://src.opensuse.org/api/v1/repos/fbui/systemd",
                &fork_body,
            );
        let fork = client(&http)
            .fork_repo(
                "pool",
                "systemd",
                &ForkRepoOptions::default(),
                &DryRun::new(false),
            )
            .unwrap()
            .unwrap();
        assert_eq!(fork.full_name, "fbui/systemd");
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn fork_repo_refuses_a_non_fork_repo_of_the_target_name() {
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/user",
                r#"{"login": "fbui"}"#,
            )
            .with_get(
                "https://src.opensuse.org/api/v1/repos/fbui/systemd",
                &repo_json("fbui", "systemd", None),
            );
        let err = client(&http)
            .fork_repo(
                "pool",
                "systemd",
                &ForkRepoOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn fork_repo_refuses_a_fork_of_another_source() {
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/user",
                r#"{"login": "fbui"}"#,
            )
            .with_get(
                "https://src.opensuse.org/api/v1/repos/fbui/systemd",
                &repo_json("fbui", "systemd", Some(("pool", "libstrophe"))),
            );
        let err = client(&http)
            .fork_repo(
                "pool",
                "systemd",
                &ForkRepoOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
    }

    #[test]
    fn fork_repo_409_race_re_reads_and_reuses() {
        let fork_body = repo_json("fbui", "systemd", Some(("pool", "systemd")));
        let http = fork_fixture(
            FakeHttp::new()
                .with_get(
                    "https://src.opensuse.org/api/v1/user",
                    r#"{"login": "fbui"}"#,
                )
                .with_post_error(
                    "https://src.opensuse.org/api/v1/repos/pool/systemd/forks",
                    ErrorKind::AlreadyExists,
                ),
            "pool",
            "systemd",
            "https://src.opensuse.org/api/v1/repos/fbui/systemd",
            &fork_body,
        );
        let fork = client(&http)
            .fork_repo(
                "pool",
                "systemd",
                &ForkRepoOptions::default(),
                &DryRun::new(false),
            )
            .unwrap()
            .unwrap();
        assert_eq!(fork.full_name, "fbui/systemd");
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn fork_repo_409_race_with_no_target_re_raises_the_refusal() {
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/user",
                r#"{"login": "fbui"}"#,
            )
            .with_get_error_sequence(
                "https://src.opensuse.org/api/v1/repos/fbui/systemd",
                &[ErrorKind::NotFound, ErrorKind::NotFound],
            )
            .with_post_error(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/forks",
                ErrorKind::AlreadyExists,
            );
        let err = client(&http)
            .fork_repo(
                "pool",
                "systemd",
                &ForkRepoOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
    }

    #[test]
    fn fork_repo_non_409_post_error_propagates_without_a_re_read() {
        // Only a 409 (AlreadyExists) re-reads the target: a 400
        // (InvalidInput) from the fork POST is the caller's fault,
        // not a race, so it propagates as-is and the target is
        // never re-read — the GETs are the user read and the
        // pre-check only.
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/user",
                r#"{"login": "fbui"}"#,
            )
            .with_get_error_sequence(
                "https://src.opensuse.org/api/v1/repos/fbui/systemd",
                &[ErrorKind::NotFound],
            )
            .with_post_error(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/forks",
                ErrorKind::InvalidInput,
            );
        let err = client(&http)
            .fork_repo(
                "pool",
                "systemd",
                &ForkRepoOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert_eq!(
            http.get_calls(),
            vec![
                ("https://src.opensuse.org/api/v1/user".to_string(), true),
                (
                    "https://src.opensuse.org/api/v1/repos/fbui/systemd".to_string(),
                    true
                ),
            ]
        );
    }

    #[test]
    fn fork_repo_re_read_parent_mismatch_is_a_server_error() {
        let fork_body = repo_json("fbui", "systemd", Some(("pool", "systemd")));
        let mismatched = repo_json("fbui", "systemd", Some(("pool", "libstrophe")));
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/user",
                r#"{"login": "fbui"}"#,
            )
            .with_get_error_sequence(
                "https://src.opensuse.org/api/v1/repos/fbui/systemd",
                &[ErrorKind::NotFound],
            )
            .with_get_sequence(
                "https://src.opensuse.org/api/v1/repos/fbui/systemd",
                &[&mismatched],
            )
            .with_post(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/forks",
                &fork_body,
            );
        let err = client(&http)
            .fork_repo(
                "pool",
                "systemd",
                &ForkRepoOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn fork_repo_re_read_missing_is_a_server_error() {
        let fork_body = repo_json("fbui", "systemd", Some(("pool", "systemd")));
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/user",
                r#"{"login": "fbui"}"#,
            )
            .with_get_error_sequence(
                "https://src.opensuse.org/api/v1/repos/fbui/systemd",
                &[ErrorKind::NotFound, ErrorKind::NotFound],
            )
            .with_post(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/forks",
                &fork_body,
            );
        let err = client(&http)
            .fork_repo(
                "pool",
                "systemd",
                &ForkRepoOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn fork_repo_ignores_a_malformed_202_body_when_the_re_read_proves_the_fork() {
        // The 202 answer is never parsed: the re-read by the known
        // target name is the verification, so a body that is not a
        // repository must not fail a fork that exists.
        let fork_body = repo_json("fbui", "systemd", Some(("pool", "systemd")));
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/user",
                r#"{"login": "fbui"}"#,
            )
            .with_get_error_sequence(
                "https://src.opensuse.org/api/v1/repos/fbui/systemd",
                &[ErrorKind::NotFound],
            )
            .with_get_sequence(
                "https://src.opensuse.org/api/v1/repos/fbui/systemd",
                &[&fork_body],
            )
            .with_post(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/forks",
                "this is not a repository",
            );
        let forked = client(&http)
            .fork_repo(
                "pool",
                "systemd",
                &ForkRepoOptions::default(),
                &DryRun::new(false),
            )
            .unwrap()
            .unwrap();
        assert_eq!(forked.full_name, "fbui/systemd");
    }

    #[test]
    fn parse_scmsync_url_cases() {
        // The real scmsync value from `games/descent3`.
        let parsed = parse_scmsync_url("https://src.opensuse.org/jengelh/descent3#master").unwrap();
        assert_eq!(parsed.authority, "src.opensuse.org");
        assert_eq!(parsed.owner, "jengelh");
        assert_eq!(parsed.repo, "descent3");
        assert_eq!(parsed.branch, Some("master".to_string()));
        // A trailing `.git` is stripped, like the server.
        let parsed =
            parse_scmsync_url("https://src.opensuse.org/jengelh/descent3.git#master").unwrap();
        assert_eq!(parsed.repo, "descent3");
        // A `git+` scheme prefix is stripped, like the server.
        let parsed =
            parse_scmsync_url("git+https://src.opensuse.org/jengelh/descent3#master").unwrap();
        assert_eq!(parsed.authority, "src.opensuse.org");
        // Userinfo and the default port name the same instance.
        let parsed =
            parse_scmsync_url("https://src.opensuse.org:443/jengelh/descent3#master").unwrap();
        assert_eq!(parsed.authority, "src.opensuse.org");
        let parsed =
            parse_scmsync_url("https://fbui@src.opensuse.org/jengelh/descent3#master").unwrap();
        assert_eq!(parsed.authority, "src.opensuse.org");
        // A non-default port is kept: it names another instance.
        let parsed =
            parse_scmsync_url("https://src.opensuse.org:8443/jengelh/descent3#master").unwrap();
        assert_eq!(parsed.authority, "src.opensuse.org:8443");
        // The trackingbranch query names the branch when no
        // fragment does.
        let parsed =
            parse_scmsync_url("https://src.opensuse.org/pool/systemd?trackingbranch=factory")
                .unwrap();
        assert_eq!(parsed.branch, Some("factory".to_string()));
        // The fragment wins over trackingbranch when both name one
        // — the real value from `network:messaging:xmpp/dino`,
        // which pins a commit while tracking `main`.
        let parsed = parse_scmsync_url(
            "https://src.opensuse.org/xmpp/dino?trackingbranch=main#e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa",
        )
        .unwrap();
        assert_eq!(parsed.owner, "xmpp");
        assert_eq!(parsed.repo, "dino");
        assert_eq!(
            parsed.branch,
            Some("e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa".to_string())
        );
        // A `subdir` query names the package inside a
        // project-synced repo — the real value from
        // `openSUSE:Leap:16.0/000release-packages`.
        let parsed = parse_scmsync_url(
            "https://src.opensuse.org/openSUSE/Leap?subdir=000release-packages#leap-16.0",
        )
        .unwrap();
        assert_eq!(parsed.owner, "openSUSE");
        assert_eq!(parsed.repo, "Leap");
        assert_eq!(parsed.branch, Some("leap-16.0".to_string()));
        // No branch named at all — the real value from
        // `home:adamm/git-quick-stats`, whose scmsync carries no
        // fragment.
        let parsed = parse_scmsync_url("https://src.opensuse.org/adamm/git-quick-stats").unwrap();
        assert_eq!(parsed.branch, None);
        // An empty fragment names no branch.
        let parsed = parse_scmsync_url("https://src.opensuse.org/pool/systemd#").unwrap();
        assert_eq!(parsed.branch, None);
        // An encoded trackingbranch value is decoded.
        let parsed =
            parse_scmsync_url("https://src.opensuse.org/pool/systemd?trackingbranch=feature%2Fx")
                .unwrap();
        assert_eq!(parsed.branch, Some("feature/x".to_string()));
        // An encoded fragment is decoded the same way (a decoding
        // probe — `feature/x` is not a real branch here).
        let parsed =
            parse_scmsync_url("https://src.opensuse.org/pool/systemd#feature%2Fx").unwrap();
        assert_eq!(parsed.branch, Some("feature/x".to_string()));
        for invalid in [
            "https://src.opensuse.org/pool",
            "https://src.opensuse.org/pool/systemd/extra#factory",
            "pool/systemd#factory",
            "ssh://git@src.opensuse.org/pool/systemd#factory",
            "https:///pool/systemd#factory",
        ] {
            assert!(
                parse_scmsync_url(invalid).is_err(),
                "{invalid} should be invalid"
            );
        }
    }

    #[test]
    fn branch_dry_run_records_plan_and_touches_nothing() {
        let gitea_http = FakeHttp::new();
        let obs_http = FakeHttp::new();
        let dry_run = DryRun::new(true);
        let result = client(&gitea_http)
            .branch(
                &obs_client(&obs_http),
                "openSUSE:Factory",
                "systemd",
                &ForkOptions::default(),
                &dry_run,
            )
            .unwrap();
        assert_eq!(result, None);
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: fork git-backed package openSUSE:Factory/systemd (Gitea fork your account/the source repository name + OBS fork to the default target project/systemd)".to_string()]
        );
        assert!(gitea_http.get_calls().is_empty());
        assert!(gitea_http.post_calls().is_empty());
        assert!(obs_http.get_calls().is_empty());
        assert!(obs_http.post_calls().is_empty());
    }

    /// The OBS half of a composite fixture: the package meta GET
    /// and the fork POST with the server's answer.
    fn obs_fork_fixture(
        http: FakeHttp,
        meta_url: &str,
        meta_body: &str,
        fork_url: &str,
        fork_body: &str,
    ) -> FakeHttp {
        http.with_get(meta_url, meta_body)
            .with_post(fork_url, fork_body)
    }

    #[test]
    fn branch_forks_a_package() {
        // The real per-package scmsync case: `games/descent3`
        // carries `jengelh/descent3#master` and no `<devel>`.
        let fork_body = repo_json_branch(
            "fbui",
            "descent3",
            Some(("jengelh", "descent3")),
            "master",
            false,
        );
        let gitea_http = fork_fixture(
            FakeHttp::new().with_get(
                "https://src.opensuse.org/api/v1/user",
                r#"{"login": "fbui"}"#,
            ),
            "jengelh",
            "descent3",
            "https://src.opensuse.org/api/v1/repos/fbui/descent3",
            &fork_body,
        );
        let meta = package_meta(
            "games",
            "descent3",
            Some("https://src.opensuse.org/jengelh/descent3#master"),
            None,
        );
        let obs_http = obs_fork_fixture(
            FakeHttp::new(),
            "https://api.opensuse.org/public/source/games/descent3/_meta",
            &meta,
            "https://api.opensuse.org/source/games/descent3?cmd=fork&scmsync=https%3A%2F%2Fsrc.opensuse.org%2Ffbui%2Fdescent3%23master",
            &fork_status(
                "home:fbui:branches:games",
                "descent3",
                "games",
                "descent3",
            ),
        );
        let result = client(&gitea_http)
            .branch(
                &obs_client(&obs_http),
                "games",
                "descent3",
                &ForkOptions::default(),
                &DryRun::new(false),
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            result,
            ForkResult {
                owner: "fbui".to_string(),
                repo: "descent3".to_string(),
                branch: "master".to_string(),
                scmsync: "https://src.opensuse.org/fbui/descent3#master".to_string(),
                target_project: "home:fbui:branches:games".to_string(),
                target_package: "descent3".to_string(),
                clone_url: "https://src.opensuse.org/fbui/descent3.git".to_string(),
                ssh_url: "gitea@src.opensuse.org:fbui/descent3.git".to_string(),
            }
        );
        // The OBS fork POST carries the full scmsync URL,
        // percent-encoded as a query value, and no target keys —
        // the server applies its defaults.
        assert_eq!(obs_http.post_calls().len(), 1);
        assert_eq!(
            obs_http.post_calls()[0].0,
            "https://api.opensuse.org/source/games/descent3?cmd=fork&scmsync=https%3A%2F%2Fsrc.opensuse.org%2Ffbui%2Fdescent3%23master"
        );
        assert!(obs_http.post_calls()[0].2);
    }

    #[test]
    fn branch_refuses_an_invalid_scmsync_branch_before_any_fork() {
        // The scmsync fragment names the branch, and it comes from
        // the server, untrusted here: a fragment carrying `#` would
        // truncate the fork's scmsync URL, so the resolved branch
        // is validated before anything is forked — no Gitea call,
        // no OBS fork.
        let gitea_http = FakeHttp::new();
        let meta = package_meta(
            "games",
            "descent3",
            Some("https://src.opensuse.org/jengelh/descent3#bad#branch"),
            None,
        );
        let obs_http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/games/descent3/_meta",
            &meta,
        );
        let err = client(&gitea_http)
            .branch(
                &obs_client(&obs_http),
                "games",
                "descent3",
                &ForkOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(gitea_http.get_calls().is_empty());
        assert!(gitea_http.post_calls().is_empty());
        assert!(obs_http.post_calls().is_empty());
    }

    #[test]
    fn branch_resolves_the_devel_source() {
        // The real devel pair: `openSUSE:Factory/gsl` has no
        // scmsync of its own, only a devel to `science/gsl`,
        // whose scmsync is `adamm/gsl.git#factory`.
        let fork_body = repo_json("fbui", "gsl", Some(("adamm", "gsl")));
        let gitea_http = fork_fixture(
            FakeHttp::new().with_get(
                "https://src.opensuse.org/api/v1/user",
                r#"{"login": "fbui"}"#,
            ),
            "adamm",
            "gsl",
            "https://src.opensuse.org/api/v1/repos/fbui/gsl",
            &fork_body,
        );
        let (debug, lines) = Debug::captured(true, false);
        let named_meta = package_meta("openSUSE:Factory", "gsl", None, Some("science"));
        let devel_meta = package_meta(
            "science",
            "gsl",
            Some("https://src.opensuse.org/adamm/gsl.git#factory"),
            None,
        );
        let obs_http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/gsl/_meta",
                &named_meta,
            )
            .with_get(
                "https://api.opensuse.org/public/source/science/gsl/_meta",
                &devel_meta,
            )
            .with_post(
                "https://api.opensuse.org/source/science/gsl?cmd=fork&scmsync=https%3A%2F%2Fsrc.opensuse.org%2Ffbui%2Fgsl%23factory",
                &fork_status("home:fbui:branches:science", "gsl", "science", "gsl"),
            );
        let result = client(&gitea_http)
            .with_debug(debug)
            .branch(
                &obs_client(&obs_http),
                "openSUSE:Factory",
                "gsl",
                &ForkOptions::default(),
                &DryRun::new(false),
            )
            .unwrap()
            .unwrap();
        assert_eq!(result.owner, "fbui");
        assert_eq!(result.repo, "gsl");
        assert_eq!(result.branch, "factory");
        assert_eq!(
            result.scmsync,
            "https://src.opensuse.org/fbui/gsl#factory".to_string()
        );
        assert_eq!(result.target_project, "home:fbui:branches:science");
        assert_eq!(
            lines.lock().unwrap().clone(),
            vec![
                "decision: forking from devel science/gsl instead of openSUSE:Factory/gsl"
                    .to_string()
            ]
        );
    }

    #[test]
    fn branch_with_no_devel_project_forks_the_named_package() {
        // SYNTHETIC: the named meta combines `Factory/gsl`'s real
        // `<devel>` with `science/gsl`'s real scmsync value — no
        // real package carries both (see the layout note). The
        // test covers the flag, not a live package.
        let fork_body = repo_json("fbui", "gsl", Some(("adamm", "gsl")));
        let gitea_http = fork_fixture(
            FakeHttp::new().with_get(
                "https://src.opensuse.org/api/v1/user",
                r#"{"login": "fbui"}"#,
            ),
            "adamm",
            "gsl",
            "https://src.opensuse.org/api/v1/repos/fbui/gsl",
            &fork_body,
        );
        let named_meta = package_meta(
            "openSUSE:Factory",
            "gsl",
            Some("https://src.opensuse.org/adamm/gsl.git#factory"),
            Some("science"),
        );
        let obs_http = obs_fork_fixture(
            FakeHttp::new(),
            "https://api.opensuse.org/public/source/openSUSE:Factory/gsl/_meta",
            &named_meta,
            "https://api.opensuse.org/source/openSUSE:Factory/gsl?cmd=fork&scmsync=https%3A%2F%2Fsrc.opensuse.org%2Ffbui%2Fgsl%23factory",
            &fork_status("home:fbui:branches:openSUSE:Factory", "gsl", "openSUSE:Factory", "gsl"),
        );
        let options = ForkOptions {
            no_devel_project: true,
            ..ForkOptions::default()
        };
        let result = client(&gitea_http)
            .branch(
                &obs_client(&obs_http),
                "openSUSE:Factory",
                "gsl",
                &options,
                &DryRun::new(false),
            )
            .unwrap()
            .unwrap();
        assert_eq!(result.target_project, "home:fbui:branches:openSUSE:Factory");
        // The devel package's meta was never read.
        assert_eq!(obs_http.get_calls().len(), 1);
    }

    #[test]
    fn branch_refuses_a_package_without_scmsync() {
        // The real `Factory/systemd` meta carries a `<devel>` to
        // `Base:System/systemd`, which carries no `<scmsync>`
        // either — the refusal runs through the devel leg. The
        // error names the named package, not the devel source
        // (the project-fork path is the inconsistent one: it
        // names the devel-resolved source).
        let gitea_http = FakeHttp::new();
        let obs_http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/systemd/_meta",
                &package_meta("openSUSE:Factory", "systemd", None, Some("Base:System")),
            )
            .with_get(
                "https://api.opensuse.org/public/source/Base:System/systemd/_meta",
                &package_meta("Base:System", "systemd", None, None),
            )
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/_meta",
                &project_meta("openSUSE:Factory", None, None),
            );
        let err = client(&gitea_http)
            .branch(
                &obs_client(&obs_http),
                "openSUSE:Factory",
                "systemd",
                &ForkOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(gitea_http.post_calls().is_empty());
        assert!(obs_http.post_calls().is_empty());
        // The devel leg ran: named meta, devel meta, then the
        // project meta for the hint.
        assert_eq!(
            obs_http
                .get_calls()
                .iter()
                .map(|(url, _)| url.as_str())
                .collect::<Vec<_>>(),
            vec![
                "https://api.opensuse.org/public/source/openSUSE:Factory/systemd/_meta",
                "https://api.opensuse.org/public/source/Base:System/systemd/_meta",
                "https://api.opensuse.org/public/source/openSUSE:Factory/_meta",
            ]
        );
    }

    #[test]
    fn branch_accepts_an_explicit_default_port_in_the_base() {
        // A base that names the default port explicitly
        // (`https://src.opensuse.org:443`) names the same instance
        // as the bare host, so a same-host scmsync URL is accepted
        // — the fork's scmsync keeps the base verbatim. The Gitea
        // cans use the `:443` base too: the client calls it.
        let base = "https://src.opensuse.org:443";
        let fork_body = repo_json_branch(
            "fbui",
            "descent3",
            Some(("jengelh", "descent3")),
            "master",
            false,
        );
        let gitea_http = FakeHttp::new()
            .with_get(&format!("{base}/api/v1/user"), r#"{"login": "fbui"}"#)
            .with_get_error_sequence(
                &format!("{base}/api/v1/repos/fbui/descent3"),
                &[ErrorKind::NotFound],
            )
            .with_get(&format!("{base}/api/v1/repos/fbui/descent3"), &fork_body)
            .with_post(
                &format!("{base}/api/v1/repos/jengelh/descent3/forks"),
                &fork_body,
            );
        let meta = package_meta(
            "games",
            "descent3",
            Some("https://src.opensuse.org/jengelh/descent3#master"),
            None,
        );
        let obs_http = obs_fork_fixture(
            FakeHttp::new(),
            "https://api.opensuse.org/public/source/games/descent3/_meta",
            &meta,
            "https://api.opensuse.org/source/games/descent3?cmd=fork&scmsync=https%3A%2F%2Fsrc.opensuse.org%3A443%2Ffbui%2Fdescent3%23master",
            &fork_status("home:fbui:branches:games", "descent3", "games", "descent3"),
        );
        let client =
            GiteaClient::with_base(&gitea_http, "https://src.opensuse.org:443", bearer()).unwrap();
        let result = client
            .branch(
                &obs_client(&obs_http),
                "games",
                "descent3",
                &ForkOptions::default(),
                &DryRun::new(false),
            )
            .unwrap()
            .unwrap();
        assert_eq!(result.branch, "master");
        assert_eq!(result.target_project, "home:fbui:branches:games");
        assert_eq!(
            result.scmsync,
            "https://src.opensuse.org:443/fbui/descent3#master".to_string()
        );
    }

    #[test]
    fn branch_names_the_project_alternative_when_the_project_is_git_managed() {
        // SYNTHETIC combination: the package half is real
        // (`Factory/systemd` carries no `<scmsync>`) and the
        // project scmsync value is real (the
        // `network:messaging:xmpp` project carries
        // `xmpp/_ObsPrj.git#master`), but no live pair was found
        // where the package lacks `<scmsync>` while its project
        // carries one (see the layout note). The test covers the
        // hint branch, not a live pair.
        let gitea_http = FakeHttp::new();
        let obs_http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/systemd/_meta",
                &package_meta("openSUSE:Factory", "systemd", None, None),
            )
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/_meta",
                &project_meta(
                    "openSUSE:Factory",
                    Some("https://src.opensuse.org/xmpp/_ObsPrj.git#master"),
                    None,
                ),
            );
        let err = client(&gitea_http)
            .branch(
                &obs_client(&obs_http),
                "openSUSE:Factory",
                "systemd",
                &ForkOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(format!("{err}").contains("fork the project"));
    }

    #[test]
    fn branch_refuses_a_repo_on_another_host() {
        let gitea_http = FakeHttp::new();
        let obs_http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/systemd/_meta",
            &package_meta(
                "openSUSE:Factory",
                "systemd",
                Some("https://codeberg.org/other/foo#main"),
                None,
            ),
        );
        let err = client(&gitea_http)
            .branch(
                &obs_client(&obs_http),
                "openSUSE:Factory",
                "systemd",
                &ForkOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(format!("{err}").contains("https://src.opensuse.org"));
    }

    #[test]
    fn branch_resolves_the_default_branch_when_none_is_named() {
        // The real fragment-less scmsync: `home:adamm/git-quick-stats`
        // carries `adamm/git-quick-stats` with no `#fragment`, and
        // the repo's default branch is `main`.
        let fork_body = repo_json_branch(
            "fbui",
            "git-quick-stats",
            Some(("adamm", "git-quick-stats")),
            "main",
            false,
        );
        let gitea_http = fork_fixture(
            FakeHttp::new()
                .with_get(
                    "https://src.opensuse.org/api/v1/user",
                    r#"{"login": "fbui"}"#,
                )
                .with_get(
                    "https://src.opensuse.org/api/v1/repos/adamm/git-quick-stats",
                    &repo_json_branch("adamm", "git-quick-stats", None, "main", false),
                ),
            "adamm",
            "git-quick-stats",
            "https://src.opensuse.org/api/v1/repos/fbui/git-quick-stats",
            &fork_body,
        );
        let (debug, lines) = Debug::captured(true, false);
        let meta = package_meta(
            "home:adamm",
            "git-quick-stats",
            Some("https://src.opensuse.org/adamm/git-quick-stats"),
            None,
        );
        let obs_http = obs_fork_fixture(
            FakeHttp::new(),
            "https://api.opensuse.org/public/source/home:adamm/git-quick-stats/_meta",
            &meta,
            "https://api.opensuse.org/source/home:adamm/git-quick-stats?cmd=fork&scmsync=https%3A%2F%2Fsrc.opensuse.org%2Ffbui%2Fgit-quick-stats%23main",
            &fork_status("home:fbui:branches:home:adamm", "git-quick-stats", "home:adamm", "git-quick-stats"),
        );
        let result = client(&gitea_http)
            .with_debug(debug)
            .branch(
                &obs_client(&obs_http),
                "home:adamm",
                "git-quick-stats",
                &ForkOptions::default(),
                &DryRun::new(false),
            )
            .unwrap()
            .unwrap();
        assert_eq!(result.branch, "main");
        assert_eq!(
            lines.lock().unwrap().clone(),
            vec![
                "decision: no branch named in the scmsync URL; using the default branch main"
                    .to_string()
            ]
        );
    }

    #[test]
    fn branch_honours_an_explicit_fork_branch() {
        // The explicit branch `slfo-main` is a real branch of
        // `adamm/gsl` (whose scmsync names `factory`); the fork
        // overrides it.
        let fork_body = repo_json("fbui", "gsl", Some(("adamm", "gsl")));
        let gitea_http = fork_fixture(
            FakeHttp::new().with_get(
                "https://src.opensuse.org/api/v1/user",
                r#"{"login": "fbui"}"#,
            ),
            "adamm",
            "gsl",
            "https://src.opensuse.org/api/v1/repos/fbui/gsl",
            &fork_body,
        );
        let meta = package_meta(
            "science",
            "gsl",
            Some("https://src.opensuse.org/adamm/gsl.git#factory"),
            None,
        );
        let obs_http = obs_fork_fixture(
            FakeHttp::new(),
            "https://api.opensuse.org/public/source/science/gsl/_meta",
            &meta,
            "https://api.opensuse.org/source/science/gsl?cmd=fork&scmsync=https%3A%2F%2Fsrc.opensuse.org%2Ffbui%2Fgsl%23slfo-main",
            &fork_status("home:fbui:branches:science", "gsl", "science", "gsl"),
        );
        let options = ForkOptions {
            fork_branch: Some("slfo-main".to_string()),
            ..ForkOptions::default()
        };
        let result = client(&gitea_http)
            .branch(
                &obs_client(&obs_http),
                "science",
                "gsl",
                &options,
                &DryRun::new(false),
            )
            .unwrap()
            .unwrap();
        assert_eq!(result.branch, "slfo-main");
        assert_eq!(
            result.scmsync,
            "https://src.opensuse.org/fbui/gsl#slfo-main".to_string()
        );
    }

    #[test]
    fn branch_forks_a_project() {
        // The real project-level scmsync: the
        // `network:messaging:xmpp` project carries
        // `xmpp/_ObsPrj.git#master`.
        let fork_body = repo_json_branch(
            "fbui",
            "_ObsPrj",
            Some(("xmpp", "_ObsPrj")),
            "master",
            false,
        );
        let gitea_http = fork_fixture(
            FakeHttp::new().with_get(
                "https://src.opensuse.org/api/v1/user",
                r#"{"login": "fbui"}"#,
            ),
            "xmpp",
            "_ObsPrj",
            "https://src.opensuse.org/api/v1/repos/fbui/_ObsPrj",
            &fork_body,
        );
        let meta = project_meta(
            "network:messaging:xmpp",
            Some("https://src.opensuse.org/xmpp/_ObsPrj.git#master"),
            None,
        );
        // A project fork answers only targetproject /
        // sourceproject — no package names.
        let fork_response = "<status code=\"ok\"><summary>Ok</summary><data name=\"targetproject\">home:fbui:branches:network:messaging:xmpp</data><data name=\"sourceproject\">network:messaging:xmpp</data></status>";
        let obs_http = obs_fork_fixture(
            FakeHttp::new(),
            "https://api.opensuse.org/public/source/network:messaging:xmpp/_meta",
            &meta,
            "https://api.opensuse.org/source/network:messaging:xmpp/_project?cmd=fork&scmsync=https%3A%2F%2Fsrc.opensuse.org%2Ffbui%2F_ObsPrj%23master",
            fork_response,
        );
        let result = client(&gitea_http)
            .branch(
                &obs_client(&obs_http),
                "network:messaging:xmpp",
                "_project",
                &ForkOptions::default(),
                &DryRun::new(false),
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            result.target_project,
            "home:fbui:branches:network:messaging:xmpp"
        );
        assert_eq!(result.target_package, "_project");
    }

    /// A one-shot loopback HTTP server: serves `body` once with
    /// `status`, then exits. The fixture transport for the debug
    /// test below (a Gitea call emits the same attempt lines as an
    /// OBS call).
    fn serve_once(body: &'static str, status: u16) -> (u16, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                use std::io::Read;
                use std::io::Write;
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (port, handle)
    }

    #[test]
    fn debug_on_records_attempt_lines_for_a_gitea_call() {
        let body = repo_json("pool", "systemd", None);
        // The body must outlive the server thread: leak it — the
        // test process exits right after.
        let body: &'static str = Box::leak(body.into_boxed_str());
        let (port, handle) = serve_once(body, 200);
        let (debug, lines) = Debug::captured(true, false);
        let transport = UreqClient::new(Timeout::default())
            .unwrap()
            .with_debug(debug.clone());
        let client =
            GiteaClient::with_base(&transport, &format!("http://127.0.0.1:{port}"), bearer())
                .unwrap()
                .with_debug(debug);
        let repo = client.repo_get("pool", "systemd").unwrap();
        handle.join().expect("server thread");
        assert_eq!(repo.name, "systemd");
        assert!(
            lines
                .lock()
                .unwrap()
                .iter()
                .any(|line| line.contains("http: GET /api/v1/repos/pool/systemd -> 200")),
            "attempt lines: {:?}",
            lines.lock().unwrap().clone()
        );
    }

    /// A collaborator-permission JSON body, shaped like Gitea
    /// 1.24.7 answers: the mode, the role name, and the user,
    /// with an unknown field to prove it is ignored.
    fn collaborator_permission_json(user: &str, permission: &str) -> String {
        let role = match permission {
            "read" => "Read",
            "write" => "Write",
            "admin" => "Admin",
            "owner" => "Owner",
            _ => "Read",
        };
        format!(
            r#"{{"permission": "{permission}", "role_name": "{role}", "user": {{"login": "{user}", "future_field": 1}}, "future_field": "tolerated"}}"#
        )
    }

    /// A collaborator-list JSON body out of login names.
    fn collaborator_list_json(users: &[&str]) -> String {
        let entries: Vec<String> = users
            .iter()
            .map(|user| format!(r#"{{"login": "{user}", "future_field": 1}}"#))
            .collect();
        format!("[{}]", entries.join(","))
    }

    #[test]
    fn collaborator_list_returns_the_collaborators() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators?page=1&limit=50",
            &collaborator_list_json(&["fbui", "lbsobolewski"]),
        );
        let users = client(&http).collaborator_list("pool", "systemd").unwrap();
        assert_eq!(
            users.iter().map(|u| u.login.as_str()).collect::<Vec<_>>(),
            vec!["fbui", "lbsobolewski"]
        );
    }

    #[test]
    fn collaborator_list_empty_list_is_empty() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators?page=1&limit=50",
            "null",
        );
        let users = client(&http).collaborator_list("pool", "systemd").unwrap();
        assert!(users.is_empty());
    }

    #[test]
    fn collaborator_permission_parses_the_mode() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators/fbui/permission",
            &collaborator_permission_json("fbui", "write"),
        );
        let permission = client(&http)
            .collaborator_permission("pool", "systemd", "fbui")
            .unwrap();
        assert_eq!(permission.permission, GiteaAccessMode::Write);
        assert_eq!(permission.user.login, "fbui");
    }

    #[test]
    fn collaborator_permission_for_another_user_is_a_server_error() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators/fbui/permission",
            &collaborator_permission_json("lbsobolewski", "write"),
        );
        let err = client(&http)
            .collaborator_permission("pool", "systemd", "fbui")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn collaborator_is_collaborator_reports_membership() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators/fbui",
            "",
        );
        assert!(client(&http)
            .collaborator_is_collaborator("pool", "systemd", "fbui")
            .unwrap());
        let http = FakeHttp::new().with_get_error(
            "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators/fbui",
            ErrorKind::NotFound,
        );
        assert!(!client(&http)
            .collaborator_is_collaborator("pool", "systemd", "fbui")
            .unwrap());
    }

    #[test]
    fn collaborator_add_puts_the_permission_and_verifies_it() {
        let http = FakeHttp::new()
            .with_put(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators/fbui",
                "",
            )
            .with_get(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators/fbui/permission",
                &collaborator_permission_json("fbui", "write"),
            );
        let granted = client(&http)
            .collaborator_add(
                "pool",
                "systemd",
                "fbui",
                GiteaAccess::Write,
                false,
                &DryRun::new(false),
            )
            .unwrap()
            .expect("a non-dry-run add returns the permission");
        assert_eq!(granted.permission, GiteaAccessMode::Write);
        assert_eq!(
            http.put_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators/fbui".to_string(),
                r#"{"permission":"write"}"#.to_string(),
                true
            )]
        );
    }

    #[test]
    fn collaborator_add_more_access_than_asked_satisfies_the_grant() {
        // An org owner reads back `owner` for any grant: the
        // grant holds, and the returned permission says so.
        let http = FakeHttp::new()
            .with_put(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators/fbui",
                "",
            )
            .with_get(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators/fbui/permission",
                &collaborator_permission_json("fbui", "owner"),
            );
        let granted = client(&http)
            .collaborator_add(
                "pool",
                "systemd",
                "fbui",
                GiteaAccess::Write,
                false,
                &DryRun::new(false),
            )
            .unwrap()
            .expect("a non-dry-run add returns the permission");
        assert_eq!(granted.permission, GiteaAccessMode::Owner);
    }

    #[test]
    fn collaborator_add_less_than_asked_is_a_server_error() {
        let http = FakeHttp::new()
            .with_put(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators/fbui",
                "",
            )
            .with_get(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators/fbui/permission",
                &collaborator_permission_json("fbui", "read"),
            );
        let err = client(&http)
            .collaborator_add(
                "pool",
                "systemd",
                "fbui",
                GiteaAccess::Write,
                false,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn collaborator_add_dry_run_plans_and_puts_nothing() {
        let http = FakeHttp::new();
        let planned = client(&http)
            .collaborator_add(
                "pool",
                "systemd",
                "fbui",
                GiteaAccess::Write,
                false,
                &DryRun::new(true),
            )
            .unwrap();
        assert!(planned.is_none());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn collaborator_add_admin_refuses_without_force() {
        let http = FakeHttp::new();
        let err = client(&http)
            .collaborator_add(
                "pool",
                "systemd",
                "fbui",
                GiteaAccess::Admin,
                false,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn collaborator_add_admin_with_force_puts_the_permission() {
        let http = FakeHttp::new()
            .with_put(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators/fbui",
                "",
            )
            .with_get(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators/fbui/permission",
                &collaborator_permission_json("fbui", "admin"),
            );
        let granted = client(&http)
            .collaborator_add(
                "pool",
                "systemd",
                "fbui",
                GiteaAccess::Admin,
                true,
                &DryRun::new(false),
            )
            .unwrap()
            .expect("a non-dry-run admin add returns the permission");
        assert_eq!(granted.permission, GiteaAccessMode::Admin);
    }

    #[test]
    fn collaborator_add_admin_dry_run_plans_and_puts_nothing() {
        let http = FakeHttp::new();
        let planned = client(&http)
            .collaborator_add(
                "pool",
                "systemd",
                "fbui",
                GiteaAccess::Admin,
                false,
                &DryRun::new(true),
            )
            .unwrap();
        assert!(planned.is_none());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn collaborator_add_refuses_an_invalid_user_name() {
        let http = FakeHttp::new();
        let err = client(&http)
            .collaborator_add(
                "pool",
                "systemd",
                "has space",
                GiteaAccess::Write,
                false,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn collaborator_remove_dry_run_plans_and_deletes_nothing() {
        let http = FakeHttp::new();
        let dry_run = DryRun::new(true);
        // Planning a remove needs no force: the refusal guards the
        // live remove, not the plan.
        let result = client(&http)
            .collaborator_remove("pool", "systemd", "fbui", false, &dry_run)
            .unwrap();
        assert_eq!(result, None);
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: remove collaborator fbui from pool/systemd".to_string()]
        );
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn collaborator_remove_refuses_an_invalid_repo_name() {
        let http = FakeHttp::new();
        let err = client(&http)
            .collaborator_remove("pool", "has space", "fbui", true, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn collaborator_remove_reads_then_deletes() {
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators/fbui",
                "",
            )
            .with_delete(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators/fbui",
                "",
            );
        let removed = client(&http)
            .collaborator_remove("pool", "systemd", "fbui", true, &DryRun::new(false))
            .unwrap()
            .expect("a non-dry-run remove returns the login");
        assert_eq!(removed, "fbui");
        assert_eq!(
            http.delete_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators/fbui".to_string(),
                true
            )]
        );
    }

    #[test]
    fn collaborator_remove_refuses_without_force() {
        let http = FakeHttp::new();
        let err = client(&http)
            .collaborator_remove("pool", "systemd", "fbui", false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.delete_calls().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn collaborator_remove_missing_collaborator_is_not_found() {
        let http = FakeHttp::new().with_get_error(
            "https://src.opensuse.org/api/v1/repos/pool/systemd/collaborators/fbui",
            ErrorKind::NotFound,
        );
        let err = client(&http)
            .collaborator_remove("pool", "systemd", "fbui", true, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(http.delete_calls().is_empty());
    }

    /// A team JSON body, shaped like Gitea 1.24.7 answers, with
    /// an unknown field to prove it is ignored. The top-level
    /// permission and the per-unit permission are separate on
    /// purpose: the server reports `none` on top for read and
    /// write teams, and the grant lives in `units_map`.
    #[allow(clippy::too_many_arguments)]
    fn team_json(
        id: i64,
        org: &str,
        name: &str,
        permission: &str,
        unit_permission: &str,
        units: &[&str],
        includes_all_repositories: bool,
        can_create_org_repo: bool,
    ) -> String {
        let units_json: Vec<String> = units.iter().map(|u| format!("\"{u}\"")).collect();
        let units_map_json: Vec<String> = units
            .iter()
            .map(|u| format!("\"{u}\": \"{unit_permission}\""))
            .collect();
        format!(
            r#"{{"id": {id}, "name": "{name}", "description": "", "organization": {{"name": "{org}"}}, "permission": "{permission}", "units": [{}], "units_map": {{{}}}, "includes_all_repositories": {includes_all_repositories}, "can_create_org_repo": {can_create_org_repo}, "future_field": "tolerated"}}"#,
            units_json.join(","),
            units_map_json.join(",")
        )
    }

    /// A team-list JSON body out of team JSON bodies.
    fn team_list_json(teams: &[String]) -> String {
        format!("[{}]", teams.join(","))
    }

    #[test]
    fn team_list_returns_the_teams() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/orgs/pool/teams?page=1&limit=50",
            &team_list_json(&[
                team_json(7, "pool", "reviewers", "none", "read", &["repo.code"], false, false),
                team_json(8, "pool", "owners", "admin", "admin", &["repo.code"], true, true),
                // The list leaves the organization null: it still parses.
                r#"{"id": 9, "name": "watchers", "description": "", "organization": null, "permission": "none", "units": ["repo.code"], "units_map": {"repo.code": "read"}, "includes_all_repositories": false, "can_create_org_repo": false, "future_field": "tolerated"}"#.to_string(),
            ]),
        );
        let teams = client(&http).team_list("pool").unwrap();
        assert_eq!(
            teams.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            vec!["reviewers", "owners", "watchers"]
        );
        assert_eq!(teams[1].permission, GiteaAccessMode::Admin);
        assert!(teams[1].includes_all_repositories);
        assert!(teams[2].organization.is_none());
    }

    #[test]
    fn team_list_empty_list_is_empty() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/orgs/pool/teams?page=1&limit=50",
            "null",
        );
        let teams = client(&http).team_list("pool").unwrap();
        assert!(teams.is_empty());
    }

    #[test]
    fn team_list_refuses_an_invalid_org_name() {
        let http = FakeHttp::new();
        let err = client(&http).team_list("has space").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn team_get_parses_the_team() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/teams/7",
            &team_json(
                7,
                "pool",
                "reviewers",
                "none",
                "read",
                &["repo.code"],
                false,
                false,
            ),
        );
        let team = client(&http).team_get(7).unwrap();
        assert_eq!(team.name, "reviewers");
        assert_eq!(
            team.organization
                .as_ref()
                .expect("a team names its organization")
                .name,
            "pool"
        );
        assert_eq!(team.permission, GiteaAccessMode::None);
        assert_eq!(
            team.units_map.get("repo.code"),
            Some(&GiteaAccessMode::Read)
        );
    }

    #[test]
    fn team_get_wrong_id_is_a_server_error() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/teams/7",
            &team_json(
                8,
                "pool",
                "owners",
                "admin",
                "admin",
                &["repo.code"],
                true,
                true,
            ),
        );
        let err = client(&http).team_get(7).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn team_get_refuses_an_invalid_id() {
        let http = FakeHttp::new();
        let err = client(&http).team_get(0).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn team_create_posts_and_verifies_the_created_team() {
        let http = FakeHttp::new().with_post(
            "https://src.opensuse.org/api/v1/orgs/pool/teams",
            &team_json(
                7,
                "pool",
                "reviewers",
                "none",
                "read",
                &GITEA_TEAM_ALL_UNITS,
                false,
                false,
            ),
        );
        let created = client(&http)
            .team_create(
                "pool",
                "reviewers",
                &TeamCreateOptions::default(),
                false,
                &DryRun::new(false),
            )
            .unwrap()
            .expect("a non-dry-run create returns the team");
        assert_eq!(created.id, 7);
        // A read team reports `none` on top; the grant is per unit.
        assert_eq!(created.permission, GiteaAccessMode::None);
        assert_eq!(created.units_map.len(), GITEA_TEAM_ALL_UNITS.len());
        assert!(created
            .units_map
            .values()
            .all(|m| *m == GiteaAccessMode::Read));
        assert_eq!(
            http.post_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/orgs/pool/teams".to_string(),
                r#"{"name":"reviewers","permission":"read","units":["repo.code","repo.issues","repo.ext_issues","repo.pulls","repo.releases","repo.wiki","repo.ext_wiki","repo.projects","repo.packages","repo.actions"],"units_map":{"repo.actions":"read","repo.code":"read","repo.ext_issues":"read","repo.ext_wiki":"read","repo.issues":"read","repo.packages":"read","repo.projects":"read","repo.pulls":"read","repo.releases":"read","repo.wiki":"read"},"includes_all_repositories":false,"can_create_org_repo":false}"#.to_string(),
                true
            )]
        );
    }

    #[test]
    fn team_create_with_options_posts_the_full_body() {
        let options = TeamCreateOptions {
            description: Some("reviews submissions".to_string()),
            permission: Some(GiteaAccess::Write),
            units: Some(vec!["repo.code".to_string(), "repo.issues".to_string()]),
            includes_all_repositories: true,
            can_create_org_repo: false,
        };
        let http = FakeHttp::new().with_post(
            "https://src.opensuse.org/api/v1/orgs/pool/teams",
            &team_json(
                7,
                "pool",
                "reviewers",
                "none",
                "write",
                &["repo.code", "repo.issues"],
                true,
                false,
            ),
        );
        let created = client(&http)
            .team_create("pool", "reviewers", &options, true, &DryRun::new(false))
            .unwrap()
            .expect("a non-dry-run create returns the team");
        assert_eq!(created.permission, GiteaAccessMode::None);
        assert_eq!(created.units, vec!["repo.code", "repo.issues"]);
        assert_eq!(
            created.units_map.get("repo.code"),
            Some(&GiteaAccessMode::Write)
        );
        assert_eq!(
            created.units_map.get("repo.issues"),
            Some(&GiteaAccessMode::Write)
        );
        assert_eq!(
            http.post_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/orgs/pool/teams".to_string(),
                r#"{"name":"reviewers","description":"reviews submissions","permission":"write","units":["repo.code","repo.issues"],"units_map":{"repo.code":"write","repo.issues":"write"},"includes_all_repositories":true,"can_create_org_repo":false}"#.to_string(),
                true
            )]
        );
    }

    #[test]
    fn team_create_wrong_permission_is_a_server_error() {
        let options = TeamCreateOptions {
            permission: Some(GiteaAccess::Write),
            ..Default::default()
        };
        let http = FakeHttp::new().with_post(
            "https://src.opensuse.org/api/v1/orgs/pool/teams",
            &team_json(
                7,
                "pool",
                "reviewers",
                "none",
                "read",
                &GITEA_TEAM_ALL_UNITS,
                false,
                false,
            ),
        );
        let err = client(&http)
            .team_create("pool", "reviewers", &options, false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn team_create_dry_run_plans_and_posts_nothing() {
        let http = FakeHttp::new();
        let planned = client(&http)
            .team_create(
                "pool",
                "reviewers",
                &TeamCreateOptions::default(),
                false,
                &DryRun::new(true),
            )
            .unwrap();
        assert!(planned.is_none());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn team_create_duplicate_units_are_deduped() {
        let options = TeamCreateOptions {
            units: Some(vec![
                "repo.code".to_string(),
                "repo.code".to_string(),
                "repo.issues".to_string(),
            ]),
            ..Default::default()
        };
        let http = FakeHttp::new().with_post(
            "https://src.opensuse.org/api/v1/orgs/pool/teams",
            &team_json(
                7,
                "pool",
                "reviewers",
                "none",
                "read",
                &["repo.code", "repo.issues"],
                false,
                false,
            ),
        );
        let created = client(&http)
            .team_create("pool", "reviewers", &options, false, &DryRun::new(false))
            .unwrap()
            .expect("a non-dry-run create returns the team");
        assert_eq!(created.units, vec!["repo.code", "repo.issues"]);
        assert_eq!(
            http.post_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/orgs/pool/teams".to_string(),
                r#"{"name":"reviewers","permission":"read","units":["repo.code","repo.issues"],"units_map":{"repo.code":"read","repo.issues":"read"},"includes_all_repositories":false,"can_create_org_repo":false}"#.to_string(),
                true
            )]
        );
    }

    #[test]
    fn team_create_empty_units_mean_all_units() {
        let options = TeamCreateOptions {
            units: Some(vec![]),
            ..Default::default()
        };
        let http = FakeHttp::new().with_post(
            "https://src.opensuse.org/api/v1/orgs/pool/teams",
            &team_json(
                7,
                "pool",
                "reviewers",
                "none",
                "read",
                &GITEA_TEAM_ALL_UNITS,
                false,
                false,
            ),
        );
        let created = client(&http)
            .team_create("pool", "reviewers", &options, false, &DryRun::new(false))
            .unwrap()
            .expect("a non-dry-run create returns the team");
        assert_eq!(created.units.len(), GITEA_TEAM_ALL_UNITS.len());
        assert_eq!(
            http.post_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/orgs/pool/teams".to_string(),
                r#"{"name":"reviewers","permission":"read","units":["repo.code","repo.issues","repo.ext_issues","repo.pulls","repo.releases","repo.wiki","repo.ext_wiki","repo.projects","repo.packages","repo.actions"],"units_map":{"repo.actions":"read","repo.code":"read","repo.ext_issues":"read","repo.ext_wiki":"read","repo.issues":"read","repo.packages":"read","repo.projects":"read","repo.pulls":"read","repo.releases":"read","repo.wiki":"read"},"includes_all_repositories":false,"can_create_org_repo":false}"#.to_string(),
                true
            )]
        );
    }

    #[test]
    fn team_create_admin_team_is_granted_all_units() {
        let options = TeamCreateOptions {
            permission: Some(GiteaAccess::Admin),
            units: Some(vec!["repo.code".to_string(), "repo.issues".to_string()]),
            ..Default::default()
        };
        let http = FakeHttp::new().with_post(
            "https://src.opensuse.org/api/v1/orgs/pool/teams",
            &team_json(
                7,
                "pool",
                "reviewers",
                "admin",
                "admin",
                &GITEA_TEAM_ALL_UNITS,
                false,
                false,
            ),
        );
        let created = client(&http)
            .team_create("pool", "reviewers", &options, true, &DryRun::new(false))
            .unwrap()
            .expect("a non-dry-run create returns the team");
        assert_eq!(created.permission, GiteaAccessMode::Admin);
        assert_eq!(created.units.len(), GITEA_TEAM_ALL_UNITS.len());
    }

    #[test]
    fn team_create_refuses_a_spaced_name() {
        let http = FakeHttp::new();
        let err = client(&http)
            .team_create(
                "pool",
                "has space",
                &TeamCreateOptions::default(),
                false,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn team_create_refuses_a_name_over_255_chars() {
        let http = FakeHttp::new();
        let err = client(&http)
            .team_create(
                "pool",
                &"n".repeat(256),
                &TeamCreateOptions::default(),
                false,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn team_create_refuses_a_description_over_255_chars() {
        let options = TeamCreateOptions {
            description: Some("d".repeat(256)),
            ..Default::default()
        };
        let http = FakeHttp::new();
        let err = client(&http)
            .team_create("pool", "reviewers", &options, false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn team_create_refuses_an_unknown_unit() {
        let options = TeamCreateOptions {
            units: Some(vec!["repo.code".to_string(), "repo.nope".to_string()]),
            ..Default::default()
        };
        let http = FakeHttp::new();
        let err = client(&http)
            .team_create("pool", "reviewers", &options, false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn team_create_admin_refuses_without_force() {
        let options = TeamCreateOptions {
            permission: Some(GiteaAccess::Admin),
            ..Default::default()
        };
        let http = FakeHttp::new();
        let err = client(&http)
            .team_create("pool", "reviewers", &options, false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn team_create_all_repositories_refuses_without_force() {
        let options = TeamCreateOptions {
            includes_all_repositories: true,
            ..Default::default()
        };
        let http = FakeHttp::new();
        let err = client(&http)
            .team_create("pool", "reviewers", &options, false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn team_create_can_create_repo_refuses_without_force() {
        let options = TeamCreateOptions {
            can_create_org_repo: true,
            ..Default::default()
        };
        let http = FakeHttp::new();
        let err = client(&http)
            .team_create("pool", "reviewers", &options, false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn team_create_admin_dry_run_plans_and_posts_nothing() {
        let options = TeamCreateOptions {
            permission: Some(GiteaAccess::Admin),
            ..Default::default()
        };
        let http = FakeHttp::new();
        let planned = client(&http)
            .team_create("pool", "reviewers", &options, false, &DryRun::new(true))
            .unwrap();
        assert!(planned.is_none());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn team_delete_reads_then_deletes() {
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/teams/7",
                &team_json(
                    7,
                    "pool",
                    "reviewers",
                    "none",
                    "read",
                    &["repo.code"],
                    false,
                    false,
                ),
            )
            .with_delete("https://src.opensuse.org/api/v1/teams/7", "");
        let deleted = client(&http)
            .team_delete(7, true, &DryRun::new(false))
            .unwrap()
            .expect("a non-dry-run delete returns the name");
        assert_eq!(deleted, "reviewers");
        assert_eq!(
            http.delete_calls(),
            vec![("https://src.opensuse.org/api/v1/teams/7".to_string(), true)]
        );
    }

    #[test]
    fn team_delete_refuses_without_force() {
        let http = FakeHttp::new();
        let err = client(&http)
            .team_delete(7, false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.delete_calls().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn team_delete_missing_team_is_not_found() {
        let http = FakeHttp::new().with_get_error(
            "https://src.opensuse.org/api/v1/teams/7",
            ErrorKind::NotFound,
        );
        let err = client(&http)
            .team_delete(7, true, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn team_delete_dry_run_plans_and_deletes_nothing() {
        let http = FakeHttp::new();
        let dry_run = DryRun::new(true);
        // Planning a delete needs no force: the refusal guards the
        // live delete, not the plan.
        let result = client(&http).team_delete(7, false, &dry_run).unwrap();
        assert_eq!(result, None);
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: delete team 7".to_string()]
        );
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn team_delete_refuses_an_invalid_id() {
        let http = FakeHttp::new();
        let err = client(&http)
            .team_delete(-1, true, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn team_list_members_returns_the_members() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/teams/7/members?page=1&limit=50",
            &collaborator_list_json(&["fbui", "lbsobolewski"]),
        );
        let members = client(&http).team_list_members(7).unwrap();
        assert_eq!(
            members.iter().map(|u| u.login.as_str()).collect::<Vec<_>>(),
            vec!["fbui", "lbsobolewski"]
        );
    }

    #[test]
    fn team_list_members_empty_list_is_empty() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/teams/7/members?page=1&limit=50",
            "null",
        );
        let members = client(&http).team_list_members(7).unwrap();
        assert!(members.is_empty());
    }

    #[test]
    fn team_member_get_parses_the_member() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/teams/7/members/fbui",
            r#"{"login": "fbui", "future_field": 1}"#,
        );
        let member = client(&http).team_member_get(7, "fbui").unwrap();
        assert_eq!(member.login, "fbui");
    }

    #[test]
    fn team_member_get_for_another_user_is_a_server_error() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/teams/7/members/fbui",
            r#"{"login": "lbsobolewski", "future_field": 1}"#,
        );
        let err = client(&http).team_member_get(7, "fbui").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn team_add_member_puts_and_verifies() {
        let http = FakeHttp::new()
            .with_put("https://src.opensuse.org/api/v1/teams/7/members/fbui", "")
            .with_get(
                "https://src.opensuse.org/api/v1/teams/7/members/fbui",
                r#"{"login": "fbui", "future_field": 1}"#,
            );
        let member = client(&http)
            .team_add_member(7, "fbui", true, &DryRun::new(false))
            .unwrap()
            .expect("a non-dry-run add returns the member");
        assert_eq!(member.login, "fbui");
        assert_eq!(
            http.put_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/teams/7/members/fbui".to_string(),
                "{}".to_string(),
                true
            )]
        );
    }

    #[test]
    fn team_add_member_unrecorded_is_a_server_error() {
        let http = FakeHttp::new()
            .with_put("https://src.opensuse.org/api/v1/teams/7/members/fbui", "")
            .with_get(
                "https://src.opensuse.org/api/v1/teams/7/members/fbui",
                r#"{"login": "lbsobolewski", "future_field": 1}"#,
            );
        let err = client(&http)
            .team_add_member(7, "fbui", true, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn team_add_member_dry_run_plans_and_puts_nothing() {
        let http = FakeHttp::new();
        let planned = client(&http)
            .team_add_member(7, "fbui", false, &DryRun::new(true))
            .unwrap();
        assert!(planned.is_none());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn team_add_member_refuses_without_force() {
        let http = FakeHttp::new();
        let err = client(&http)
            .team_add_member(7, "fbui", false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn team_add_member_refuses_an_invalid_id() {
        let http = FakeHttp::new();
        let err = client(&http)
            .team_add_member(0, "fbui", true, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn team_remove_member_reads_then_deletes() {
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/teams/7/members/fbui",
                r#"{"login": "fbui", "future_field": 1}"#,
            )
            .with_delete("https://src.opensuse.org/api/v1/teams/7/members/fbui", "");
        let removed = client(&http)
            .team_remove_member(7, "fbui", true, &DryRun::new(false))
            .unwrap()
            .expect("a non-dry-run remove returns the login");
        assert_eq!(removed, "fbui");
        assert_eq!(
            http.delete_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/teams/7/members/fbui".to_string(),
                true
            )]
        );
    }

    #[test]
    fn team_remove_member_refuses_without_force() {
        let http = FakeHttp::new();
        let err = client(&http)
            .team_remove_member(7, "fbui", false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.delete_calls().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn team_remove_member_missing_member_is_not_found() {
        let http = FakeHttp::new().with_get_error(
            "https://src.opensuse.org/api/v1/teams/7/members/fbui",
            ErrorKind::NotFound,
        );
        let err = client(&http)
            .team_remove_member(7, "fbui", true, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn team_remove_member_dry_run_plans_and_deletes_nothing() {
        let http = FakeHttp::new();
        let dry_run = DryRun::new(true);
        // Planning a remove needs no force: the refusal guards the
        // live remove, not the plan.
        let result = client(&http)
            .team_remove_member(7, "fbui", false, &dry_run)
            .unwrap();
        assert_eq!(result, None);
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: remove member fbui from team 7".to_string()]
        );
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn team_list_repos_returns_the_repos() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/teams/7/repos?page=1&limit=50",
            &repo_list_json(&[
                repo_json("pool", "systemd", None),
                repo_json("pool", "libstrophe", None),
            ]),
        );
        let repos = client(&http).team_list_repos(7).unwrap();
        assert_eq!(
            repos
                .iter()
                .map(|r| r.full_name.as_str())
                .collect::<Vec<_>>(),
            vec!["pool/systemd", "pool/libstrophe"]
        );
    }

    #[test]
    fn team_list_repos_empty_list_is_empty() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/teams/7/repos?page=1&limit=50",
            "null",
        );
        let repos = client(&http).team_list_repos(7).unwrap();
        assert!(repos.is_empty());
    }

    #[test]
    fn team_repo_get_parses_the_repo() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/teams/7/repos/pool/systemd",
            &repo_json("pool", "systemd", None),
        );
        let repo = client(&http).team_repo_get(7, "pool", "systemd").unwrap();
        assert_eq!(repo.full_name, "pool/systemd");
    }

    #[test]
    fn team_repo_get_for_another_repo_is_a_server_error() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/teams/7/repos/pool/systemd",
            &repo_json("pool", "libstrophe", None),
        );
        let err = client(&http)
            .team_repo_get(7, "pool", "systemd")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn team_add_repo_puts_and_verifies() {
        let http = FakeHttp::new()
            .with_put(
                "https://src.opensuse.org/api/v1/teams/7/repos/pool/systemd",
                "",
            )
            .with_get(
                "https://src.opensuse.org/api/v1/teams/7/repos/pool/systemd",
                &repo_json("pool", "systemd", None),
            );
        let repo = client(&http)
            .team_add_repo(7, "pool", "systemd", true, &DryRun::new(false))
            .unwrap()
            .expect("a non-dry-run add returns the repo");
        assert_eq!(repo.full_name, "pool/systemd");
        assert_eq!(
            http.put_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/teams/7/repos/pool/systemd".to_string(),
                "{}".to_string(),
                true
            )]
        );
    }

    #[test]
    fn team_add_repo_unrecorded_is_a_server_error() {
        let http = FakeHttp::new()
            .with_put(
                "https://src.opensuse.org/api/v1/teams/7/repos/pool/systemd",
                "",
            )
            .with_get(
                "https://src.opensuse.org/api/v1/teams/7/repos/pool/systemd",
                &repo_json("pool", "libstrophe", None),
            );
        let err = client(&http)
            .team_add_repo(7, "pool", "systemd", true, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn team_add_repo_dry_run_plans_and_puts_nothing() {
        let http = FakeHttp::new();
        let planned = client(&http)
            .team_add_repo(7, "pool", "systemd", false, &DryRun::new(true))
            .unwrap();
        assert!(planned.is_none());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn team_add_repo_refuses_without_force() {
        let http = FakeHttp::new();
        let err = client(&http)
            .team_add_repo(7, "pool", "systemd", false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn team_add_repo_refuses_an_invalid_repo_name() {
        let http = FakeHttp::new();
        let err = client(&http)
            .team_add_repo(7, "pool", "has space", true, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.put_calls().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn team_remove_repo_reads_then_deletes() {
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/teams/7/repos/pool/systemd",
                &repo_json("pool", "systemd", None),
            )
            .with_delete(
                "https://src.opensuse.org/api/v1/teams/7/repos/pool/systemd",
                "",
            );
        let removed = client(&http)
            .team_remove_repo(7, "pool", "systemd", true, &DryRun::new(false))
            .unwrap()
            .expect("a non-dry-run remove returns the full name");
        assert_eq!(removed, "pool/systemd");
        assert_eq!(
            http.delete_calls(),
            vec![(
                "https://src.opensuse.org/api/v1/teams/7/repos/pool/systemd".to_string(),
                true
            )]
        );
    }

    #[test]
    fn team_remove_repo_refuses_without_force() {
        let http = FakeHttp::new();
        let err = client(&http)
            .team_remove_repo(7, "pool", "systemd", false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.delete_calls().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn team_remove_repo_dry_run_plans_and_deletes_nothing() {
        let http = FakeHttp::new();
        let dry_run = DryRun::new(true);
        // Planning a remove needs no force: the refusal guards the
        // live remove, not the plan.
        let result = client(&http)
            .team_remove_repo(7, "pool", "systemd", false, &dry_run)
            .unwrap();
        assert_eq!(result, None);
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: remove repository pool/systemd from team 7".to_string()]
        );
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn team_remove_repo_missing_repo_is_not_found() {
        let http = FakeHttp::new().with_get_error(
            "https://src.opensuse.org/api/v1/teams/7/repos/pool/systemd",
            ErrorKind::NotFound,
        );
        let err = client(&http)
            .team_remove_repo(7, "pool", "systemd", true, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(http.delete_calls().is_empty());
    }
}
