//! OBS operations: the devel-project lookup, the open-submit-request
//! check, and submit-request filing.
//!
//! This is the library surface a tool uses to move a package update
//! through the distribution. An agent (for example freshcrab) drives
//! it like this: resolve the package's devel project, read the devel
//! version, check no open submit request already targets the package,
//! build a validated submit-request payload, and file it — or, in
//! dry-run, log exactly what would be filed and file nothing.
//!
//! Behaviour rules, from the openSUSE packaging guide
//! (`references/osc-usage.md` "Requests" and "Lookup",
//! `references/request-text.md` "SR and PR messages",
//! `references/git-workflow.md` "Submitting changes", and
//! `references/changelog-rules.md"):
//! - A submit request targets the DEVEL project, never Factory
//!   directly. A payload naming `openSUSE:Factory` as its target is
//!   refused at build time.
//! - An open submit request for the package must be checked before
//!   filing (dedup). [`SrFiling::file`] performs that check itself in
//!   live mode and refuses to file a duplicate; the check is also
//!   available on its own as [`ObsClient::has_open_sr`].
//! - The SR description follows the guide's message format: one
//!   headline (at most 160 characters of prose), then `- ` lines for
//!   staging, needs, and rebuilds. The library validates the
//!   description is non-empty; the headline shape is the caller's
//!   responsibility (see [`SrPayload`]).
//! - A security fix's `.changes` entry must name every CVE it fixes
//!   (the changelog is the verification that the fix landed). When
//!   the caller supplies CVE ids, [`SrPayload::new`] refuses a
//!   `.changes` entry that does not name each of them.
//! - A failed lookup must never read as good news: a lookup that
//!   errors (transport, auth, unexpected response) returns an error,
//!   never a silent "not found" or "no open SR". Only a successful
//!   response that carries no result reads as "none" / "false".
//! - The full submit-request lifecycle is file -> supersede -> delete
//!   branch, and the branch dies by default: every payload carries
//!   `opt_sourceupdate="cleanup"`, so OBS deletes the source branch
//!   when the request is accepted (and the source project with it,
//!   when that was its last package and it is not a top-level home
//!   project). A superseded request always points at a real
//!   replacement: [`SrFiling::refile`] files the new request first and
//!   only then supersedes the old one with it. The old branch still
//!   needs an explicit delete — a superseded request never reaches
//!   accept, so the server-side cleanup never runs for it — and so
//!   does any branch whose request was filed without the attribute or
//!   was declined. Deleting a branch refuses while requests in `new`,
//!   `review`, or `declined` state involve the package as a submit
//!   source or target, because the API would silently revoke/decline
//!   them; the caller resolves those requests first, or forces the
//!   delete explicitly (see [`ObsClient::delete_package`]).
//! - The supersede and delete semantics are validated against the OBS
//!   API server code, not copied from osc. osc's flow is a reference
//!   to check against, not ground truth: where osc and the API
//!   disagree, the API wins and the discrepancy is documented on the
//!   method. Concretely: `superseded_by` is required by the server
//!   (a supersede without it is refused); superseding is allowed from
//!   `new`, `review`, and `declined` but refused from any final state
//!   (`accepted`, `superseded`, `revoked`); the server does not verify
//!   that `superseded_by` names a real request, so the file-first
//!   ordering is what makes the link real; and a package delete does
//!   not fail on open requests — it revokes them as source and
//!   declines them as target — which is why the library refuses the
//!   delete instead of letting the API close them silently.
//! - The source-update attribute is validated against the OBS API
//!   server code, not copied from osc: the only values the server
//!   accepts are `update`, `noupdate`, and `cleanup`
//!   (`BsRequestAction::VALID_SOURCEUPDATE_OPTIONS`); the library
//!   emits `cleanup` by default and refuses any other value. On
//!   accept the server deletes the source package
//!   (`BsRequestActionSubmit` calls `source_cleanup`), and the source
//!   project with it when it was its last package — except a
//!   top-level `home:<user>` project, which is never deleted. The
//!   server also defaults to cleanup itself when no value is given
//!   and the source is the filer's branch project for the target, but
//!   the library sends the value explicitly: the server default only
//!   covers branch projects, and pr-guard refuses a `home:`
//!   subproject filing without it. Cleanup is refused at creation
//!   when the source project is not local, or when the source package
//!   is used as a devel package by other packages; and the filer must
//!   be able to modify the source. A forward from a devel project
//!   must NOT carry cleanup — accept would delete the devel package
//!   — so that case uses [`SrPayload::without_cleanup`].
//!
//! Credentials come from geectl's auth surface (`EnvObsAuth`, OBS
//! Basic via `GEECTL_OBS_USERNAME` + `GEECTL_OBS_PASSWORD` or
//! `GEECTL_OBS_PASSWORD_FILE`). The public reads (devel project,
//! devel version) need no credentials. The open-SR search and the
//! filing POST need them and fail loudly without them. No credential
//! is stored here, and no request carries one over cleartext http
//! except to a loopback host (the transport guard in `http.rs`).
//!
//! The OBS API base is `https://api.opensuse.org` by default. A
//! custom base is accepted for a test instance (see
//! [`ObsClient::with_base`]): it must be `https://` with a host, or
//! `http://` on a loopback host. URLs are built with the encoders
//! from `url.rs` (one encoder per context); the base itself is
//! carried as a string because the URL builder rightly rejects
//! `http://` bases, and the test instance is plain http on loopback
//! (the same reason the live harness builds its URLs as strings).
//!
//! The live executor that actually branches, bumps, cleans, builds,
//! and runs the package gates is NOT part of this module: the library
//! files, supersedes, and deletes; the caller owns the package work.
//! Branch deletion stays a separate explicit call — the caller decides
//! when a branch is safe to delete — but the full sequence (file ->
//! supersede -> delete branch) is documented on [`SrFiling::refile`].

use crate::auth::Credentials;
use crate::debug::Debug;
use crate::error::{ErrorKind, GeectlError};
use crate::http::HttpClient;
use crate::xml::{find_tag_end, xml_child_elements, xml_elements, xml_escape, xml_unescape};
use std::fmt;

pub(crate) mod branch;
pub(crate) mod common;
pub(crate) mod maintenance;
pub(crate) mod remote;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod types;

pub(crate) use self::branch::build_link_meta;
pub(crate) use self::common::action_tag_type;
pub(crate) use self::common::build_url;
pub(crate) use self::common::element_text;
pub(crate) use self::common::elements;
pub(crate) use self::common::filter_history;
pub(crate) use self::common::find_element_tag;
pub(crate) use self::common::meta_envelope;
pub(crate) use self::common::meta_roles;
pub use self::common::parse_project_package;
pub(crate) use self::common::parse_request_id;
pub(crate) use self::common::parse_request_ids;
pub(crate) use self::common::parse_request_ids_for_action;
pub(crate) use self::common::parse_search_matches;
pub(crate) use self::common::parse_status_invoked;
pub(crate) use self::common::parse_status_ok;
pub(crate) use self::common::parse_submit_request_ids;
pub(crate) use self::common::quoted_attr_value;
pub(crate) use self::common::request_block_matches_action;
pub use self::common::split_slash_segments;
pub(crate) use self::common::status_data;
pub(crate) use self::common::tag_attr;
pub use self::common::valid_attribute_name;
pub use self::common::valid_obs_filename;
pub use self::common::valid_obs_name;
pub(crate) use self::common::validate_base;
pub use self::common::FACTORY_PROJECT;
pub use self::common::OBS_API_BASE;
pub(crate) use self::remote::parse_directory_rev;
pub(crate) use self::remote::parse_linkinfo;
pub(crate) use self::remote::parse_package_names;
pub(crate) use self::remote::parse_revision;
pub use self::types::changelog_names_cve;
pub(crate) use self::types::ActionShape;
pub(crate) use self::types::ActionTarget;
pub use self::types::AggregateOptions;
pub use self::types::BranchOptions;
pub use self::types::BranchResult;
pub use self::types::CopyOptions;
pub use self::types::CopyProjectOptions;
pub use self::types::CreatePackageOptions;
pub use self::types::CveId;
pub use self::types::DeleteProjectOptions;
pub(crate) use self::types::DirectoryListing;
pub use self::types::DryRun;
pub use self::types::FilePatch;
pub use self::types::FilePatchStatus;
pub use self::types::HistoryEntry;
pub use self::types::LinkOptions;
pub use self::types::LinkRevision;
pub use self::types::NamedRole;
pub use self::types::Owner;
pub use self::types::PackageDiff;
pub use self::types::PackageDiffStatus;
pub use self::types::PackageMatch;
pub use self::types::PackageMeta;
pub use self::types::Person;
pub use self::types::ProjectMatch;
pub use self::types::ProjectMeta;
pub use self::types::ReleaseOptions;
pub use self::types::Repository;
pub use self::types::RevisionPatch;
pub use self::types::RevisionRange;
pub use self::types::RevisionSpec;
pub use self::types::SourceFile;
/// The OBS client: the devel-project lookup, the devel version, the
/// open-submit-request check, submit-request filing, branch setup,
/// and the remote reads (file and directory reads, history, meta,
/// search, owners, person, link target, project diff).
///
/// The client holds the HTTP client and the OBS Basic credentials
/// (resolved by the caller through geectl's auth surface, e.g.
/// `EnvObsAuth`). The public reads need no credentials; the
/// credentialed reads and the filing POST need them and fail loudly
/// without them.
pub struct ObsClient<'a> {
    http: &'a dyn HttpClient,
    base: String,
    credentials: Credentials,
    debug: Debug,
}

impl<'a> ObsClient<'a> {
    /// A client for the production OBS API, with no credentials
    /// (public reads only).
    pub fn new(http: &'a dyn HttpClient) -> Self {
        ObsClient {
            http,
            base: OBS_API_BASE.to_string(),
            credentials: Credentials::None,
            debug: Debug::disabled(),
        }
    }

    /// A client for a custom OBS API base (a test instance), with no
    /// credentials. The base must be `https://` with a host, or
    /// `http://` on a loopback host.
    pub fn with_base(http: &'a dyn HttpClient, base: &str) -> Result<Self, GeectlError> {
        validate_base(base)?;
        Ok(ObsClient {
            http,
            base: base.trim_end_matches('/').to_string(),
            credentials: Credentials::None,
            debug: Debug::disabled(),
        })
    }

    /// A client with OBS Basic credentials (for the authenticated
    /// endpoints: the open-SR search and filing a submit request).
    /// Accepts `Credentials::Basic` and `Credentials::None`; a Bearer
    /// token or an SSH key cannot authenticate OBS and is refused.
    pub fn with_credentials(
        http: &'a dyn HttpClient,
        credentials: Credentials,
    ) -> Result<Self, GeectlError> {
        match &credentials {
            Credentials::None | Credentials::Basic { .. } => Ok(ObsClient {
                http,
                base: OBS_API_BASE.to_string(),
                credentials,
                debug: Debug::disabled(),
            }),
            Credentials::Bearer { .. } | Credentials::SshKey { .. } => Err(GeectlError::new(
                ErrorKind::ConfigError,
                "OBS accepts Basic credentials only; a Bearer token or SSH key cannot authenticate it",
            )),
        }
    }

    /// A client for a custom OBS API base with OBS Basic credentials.
    pub fn with_base_and_credentials(
        http: &'a dyn HttpClient,
        base: &str,
        credentials: Credentials,
    ) -> Result<Self, GeectlError> {
        validate_base(base)?;
        match &credentials {
            Credentials::None | Credentials::Basic { .. } => Ok(ObsClient {
                http,
                base: base.trim_end_matches('/').to_string(),
                credentials,
                debug: Debug::disabled(),
            }),
            Credentials::Bearer { .. } | Credentials::SshKey { .. } => Err(GeectlError::new(
                ErrorKind::ConfigError,
                "OBS accepts Basic credentials only; a Bearer token or SSH key cannot authenticate it",
            )),
        }
    }

    /// Report the channel's decision and retry lines to
    /// `debug` instead of dropping them. The sink is off by
    /// default; the caller that owns the debug channel passes its
    /// sink here. (The HTTP attempt lines come from the HTTP client
    /// itself, which the caller configures separately.)
    pub fn with_debug(mut self, debug: Debug) -> Self {
        self.debug = debug;
        self
    }

    /// The credentials, if any.
    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    /// True when OBS Basic credentials are configured.
    fn has_basic(&self) -> bool {
        matches!(self.credentials, Credentials::Basic { .. })
    }

    /// True when an open submit request targets `package`.
    ///
    /// Uses the OBS search API, matching on actions targeting the
    /// package in the `new`/`review` states, then keeping only the
    /// requests that carry a submit action: the type filter lives
    /// client-side in `parse_submit_request_ids` as defence-in-depth
    /// — the engine does whitelist `action/@type`, so filtering
    /// server-side is a follow-up, not this change. The
    /// search has no public equivalent: without credentials it is
    /// refused here, before any request, instead of made and 401'd —
    /// a failed check is fail-closed (an error), never "no open SR".
    /// An unexpected response (no `matches` count) is an error, not
    /// false.
    pub fn has_open_sr(&self, package: &str) -> Result<bool, GeectlError> {
        Ok(!self.open_target_request_ids(package)?.is_empty())
    }

    /// The ids of the open submit requests targeting `package` (state
    /// `new` or `review`): the id-returning form of the
    /// [`ObsClient::has_open_sr`] search. A caller that needs to
    /// exempt one request — [`SrFiling::refile`] exempting the request
    /// it supersedes — uses this instead of the boolean, so the
    /// exemption is decided on the ids the server actually returned.
    /// Requires credentials; an unexpected response is an error,
    /// never an empty list.
    fn open_target_request_ids(&self, package: &str) -> Result<Vec<String>, GeectlError> {
        let url = self.search_url(package)?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "checking for open submit requests requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_submit_request_ids(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS search response for package {package:?}"),
            )
        })
    }

    /// The ids of the open requests (`new` or `review` state) whose
    /// single action matches `target` exactly. The search filters on
    /// source/target, state, and action type (`action/@type`); the
    /// parser re-checks the type and narrows by shape where the
    /// search cannot — a delete search cannot match "no target
    /// package", and the incident identity includes the release
    /// project. The match mirrors the server's action identity —
    /// delete shapes are compared by subsumption, not by key (a
    /// whole-project target subsumes package/repository targets):
    /// the incident keys on target and source package — and on the
    /// release project when one is given, while a filing without one
    /// matches any stored release project — and change-devel keys on
    /// the target alone. A declined request is closed
    /// and does not block a re-file, so it is not in the state set —
    /// only genuinely open requests block a filing. Requires
    /// credentials; an unexpected response is an error, never an
    /// empty list.
    fn open_action_request_ids(
        &self,
        target: &ActionTarget<'_>,
    ) -> Result<Vec<String>, GeectlError> {
        let states = "(state/@name='new' or state/@name='review')";
        let mut action_parts: Vec<String> = Vec::new();
        if let Some(source_project) = target.source_project {
            if !valid_obs_name(source_project) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS project name for source project: {source_project:?}"),
                ));
            }
            action_parts.push(format!("source/@project='{source_project}'"));
        }
        if let Some(source_package) = target.source_package {
            if !valid_obs_name(source_package) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS package name for source package: {source_package:?}"),
                ));
            }
            action_parts.push(format!("source/@package='{source_package}'"));
        }
        if !valid_obs_name(target.target_project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "invalid OBS project name for target project: {:?}",
                    target.target_project
                ),
            ));
        }
        action_parts.push(format!("target/@project='{}'", target.target_project));
        if let Some(target_package) = target.target_package {
            if !valid_obs_name(target_package) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS package name for target package: {target_package:?}"),
                ));
            }
            // A delete search leaves the package out: the whole-project
            // form has no target package, and a match on one would
            // never see an open whole-project delete. The parser
            // narrows by shape instead.
            if !matches!(target.shape, ActionShape::DeleteTarget { .. }) {
                action_parts.push(format!("target/@package='{target_package}'"));
            }
        }
        if let Some(target_repository) = target.target_repository {
            if !valid_obs_name(target_repository) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "invalid OBS repository name for target repository: {target_repository:?}"
                    ),
                ));
            }
            // Validated but never added to the match: the search
            // whitelist has no `action/target/@repository`, and a
            // delete dedup matches the project — the parser narrows
            // by shape instead.
        }
        let match_expr = format!(
            "{states} and action/@type='{}' and action[{}]",
            target.action_type,
            action_parts.join(" and ")
        );
        let url = build_url(
            &self.base,
            &["search", "request"],
            &[("match", &match_expr)],
        );
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "checking for open requests requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_request_ids_for_action(&body, target.action_type, target.shape).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected OBS search response for {} requests",
                    target.action_type
                ),
            )
        })
    }

    /// File `payload` as a submit request: render it and POST it to
    /// the OBS API (`POST /request?cmd=create`) and return the new
    /// request id. Requires credentials; without them the filing
    /// fails loudly before any request.
    ///
    /// Private: [`SrFiling`] is the only caller. Filing goes through
    /// the guarded workflow there (payload validation, dry-run, and
    /// the dedup check) — a direct call would bypass all three, so
    /// there is no direct call to bypass them with. The request XML
    /// is rendered here from the typed payload; no caller passes XML.
    fn create_request(&self, payload: &SrPayload) -> Result<String, GeectlError> {
        self.create_request_with_query(&payload.to_xml(), &[("cmd", "create")], "submit")
    }

    /// File a maintenance incident request: the same filing POST as
    /// [`ObsClient::create_request`], with `addrevision=1`, which pins
    /// `source_rev` to the current expanded srcmd5 at filing time and
    /// refuses to file when the source is broken. The server applies
    /// it to submit, release, maintenance_incident and
    /// maintenance_release actions (`RequestController#create` sets it,
    /// `BsRequest#set_add_revision`, enforced in
    /// `BsRequestAction#check_for_expand_errors!`); this library sends
    /// it only for maintenance incidents, where the pin is the point —
    /// an incident must freeze the exact source it was filed against.
    /// Same convention as `create_request`: [`SrFiling`] is the single
    /// permitted caller in this crate.
    fn create_request_with_add_revision(&self, xml: &str) -> Result<String, GeectlError> {
        self.create_request_with_query(
            xml,
            &[("cmd", "create"), ("addrevision", "1")],
            "maintenance incident",
        )
    }

    /// The shared filing POST: `POST /request` with the given query
    /// parameters and the request XML as the body, returning the new
    /// request id. Requires credentials; without them the filing
    /// fails loudly before any request. `kind` names the request
    /// type in the credentials error only.
    fn create_request_with_query(
        &self,
        xml: &str,
        query: &[(&str, &str)],
        kind: &str,
    ) -> Result<String, GeectlError> {
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!(
                    "cannot file a {kind} request without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)"
                ),
            ));
        }
        let url = build_url(&self.base, &["request"], query);
        let body = self.http.post(&url, xml, &self.credentials)?;
        parse_request_id(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS create-request response: no request id",
            )
        })
    }

    /// Supersede submit request `old_id` with `new_id`: the old
    /// request moves to state `superseded`, with `superseded_by` set
    /// to the new id. Requires credentials; without them the call
    /// fails loudly before any request.
    ///
    /// Semantics, verified against the OBS API server
    /// (`BsRequest#change_state` and
    /// `BsRequestPermissionCheck#cmd_changestate_permissions`) and
    /// the live API — not taken from osc:
    /// - `superseded_by` is REQUIRED: the server refuses a supersede
    ///   without it (`PostRequestMissingParameter`), and the model
    ///   requires it to be a positive number. The library always
    ///   sends it.
    /// - Allowed from `new`, `review`, and `declined`. Refused from a
    ///   final state: `accepted` cannot change state at all, and a
    ///   final state cannot move to another final state, so
    ///   `accepted`, `superseded`, or `revoked` -> `superseded` is
    ///   refused. The library does not pre-check the old request's
    ///   state, so the server stays the authority on the transition
    ///   and its 403 is surfaced as-is.
    /// - The server does NOT verify that `superseded_by` points at a
    ///   real request — it only checks the number is positive. The
    ///   ordering guarantee (the new request exists first) belongs to
    ///   the caller; [`SrFiling::refile`] provides it.
    /// - Who may supersede: an admin, the request creator, or anyone
    ///   with write permission in the target or the source project.
    ///   Anyone else gets the server's 403.
    /// - The comment sent with the supersede is `superseded by
    ///   <new>`, the form osc sends and the form the live API shows
    ///   on superseded requests.
    /// - Success is the `<status code="ok"/>` body: any other body
    ///   is a server error, never success.
    pub fn supersede_request(&self, old_id: &str, new_id: &str) -> Result<(), GeectlError> {
        let old = validate_request_id(old_id)?;
        let new = validate_request_id(new_id)?;
        if old == new {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("cannot supersede submit request {old_id} with itself"),
            ));
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot supersede a submit request without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(
            &self.base,
            &["request", &old.to_string()],
            &[
                ("cmd", "changestate"),
                ("newstate", "superseded"),
                ("superseded_by", &new.to_string()),
            ],
        );
        // The comment travels as the request body, as plain text:
        // the server's `RequestController#request_command` takes
        // `request.raw_post` as the comment
        // (openSUSE/open-build-service,
        // `src/api/app/controllers/request_controller.rb`), so the body
        // is the comment verbatim and its content type is its actual
        // type (`text/plain`), not the `application/xml` the filing
        // POST sends. The server reads the raw body regardless of
        // content type, so either would reach it — but the header
        // must say what the body is.
        let body = self
            .http
            .post_text(&url, &format!("superseded by {new}"), &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS supersede response for submit request {old}"),
            ))
        }
    }

    /// The request ids in `new`, `review`, or `declined` state that
    /// involve `project`/`package` as a submit source or target.
    /// Used by [`ObsClient::delete_package`]: deleting a package
    /// revokes/declines exactly these requests, so they block the
    /// delete until they are resolved or the caller forces it.
    /// Requires credentials; an unexpected response is an error,
    /// never an empty list.
    pub fn open_requests_for_package(
        &self,
        project: &str,
        package: &str,
    ) -> Result<Vec<String>, GeectlError> {
        let url = self.package_requests_search_url(project, package)?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "searching for requests involving a package requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_request_ids(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS search response for package {project}/{package}"),
            )
        })
    }

    /// Build the OBS search URL for requests involving a package:
    /// source or target is this project/package, in `new`, `review`,
    /// or `declined` state. The state set is exactly the set the
    /// API's package-delete callbacks touch
    /// (`Package#open_requests_with_package_as_source` /
    /// `#open_requests_with_package_as_target`): a delete revokes
    /// those as source and declines those as target. `superseded`,
    /// `revoked`, and `accepted` requests are already closed and are
    /// not touched by a delete.
    pub fn package_requests_search_url(
        &self,
        project: &str,
        package: &str,
    ) -> Result<String, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        let match_expr = format!(
            "(action/source/@project='{project}' and action/source/@package='{package}' or action/target/@project='{project}' and action/target/@package='{package}') and (state/@name='new' or state/@name='review' or state/@name='declined')"
        );
        Ok(build_url(
            &self.base,
            &["search", "request"],
            &[("match", &match_expr)],
        ))
    }

    /// Read submit request `id` (`GET /request/:id`): what it does,
    /// the state it is in, its reviews, and its history. An agent
    /// reads a request before acting on it — to see what it changes,
    /// who filed it, what state it is in, and what has already been
    /// said about it — and after acting on it, to confirm the state
    /// it landed in. Requires credentials; without them the call
    /// fails loudly before any request. A request that does not exist
    /// reads as `NotFound`, and a document the parser does not
    /// recognise in full is a server error, never a partial request.
    ///
    /// The read asks for the history with the request
    /// (`withhistory=1`): the creation entry and the state changes
    /// are part of reading a request, not a separate operation — the
    /// separate operation ([`ObsClient::request_log`]) is for when
    /// the history alone is what the caller wants.
    ///
    /// The operation is the same whether the request's packages are
    /// OBS-native or git-backed: a request is a server object, and
    /// its actions name projects and packages without saying how
    /// their sources are stored, so nothing here branches on the
    /// backend.
    pub fn show_request(&self, id: &str) -> Result<Request, GeectlError> {
        self.fetch_request(id)
    }

    /// Read one request's history (`GET /request/:id` with
    /// `withhistory=1`, returning only the history entries): the
    /// creation entry first, then each state change in order, with
    /// who made it, when, and what it was. An agent reads the log to
    /// audit what happened to a request — who declined it and why,
    /// when it was superseded — without pulling the whole request.
    /// Requires credentials; without them the call fails loudly
    /// before any request. The history the server sends with
    /// `withhistory` is the request history (creation plus state
    /// changes); review history is not included, by the server's own
    /// choice.
    pub fn request_log(&self, id: &str) -> Result<Vec<RequestHistory>, GeectlError> {
        Ok(self.fetch_request(id)?.history)
    }

    /// The shared read behind [`ObsClient::show_request`] and
    /// [`ObsClient::request_log`]: fetch the request document with
    /// its history and parse it in full.
    fn fetch_request(&self, id: &str) -> Result<Request, GeectlError> {
        let id = validate_request_id(id)?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot read a submit request without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(
            &self.base,
            &["request", &id.to_string()],
            &[("withhistory", "1")],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_request(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS request response for submit request {id}"),
            )
        })
    }

    /// Format a retry budget for a probe line: whole seconds as
    /// `30s`, anything else in milliseconds (`1500ms`).
    /// `Duration::as_secs` truncates, so a 1500 ms budget used to
    /// print as `1s`.
    fn format_budget(budget: std::time::Duration) -> String {
        let millis = budget.as_millis();
        if millis.is_multiple_of(1000) {
            format!("{}s", millis / 1000)
        } else {
            format!("{millis}ms")
        }
    }

    /// POST the changestate command at `url` with `comment` as the
    /// body, retrying transient failures while `budget` lasts and
    /// checking the request's state before any re-issue: the
    /// server's API replies before its backend has registered a
    /// just-filed request, so the changestate POST that follows a
    /// filing can fail while the request is about to exist (seen on
    /// live run 44880250442: the accept POST returned 500, while
    /// the same accept passed on earlier runs). Only transient
    /// failures retry — a 5xx, a timeout, a network error, or a
    /// not-found (the just-filed request is not visible yet while
    /// the backend registers it, the same race as the package-create
    /// retry). A deterministic failure fails fast instead of
    /// burning the budget — bad credentials, or a malformed id
    /// refused client-side. A well-formed id that does not exist is
    /// different: it 404s, which is in the retry set, so it takes
    /// up to 60 s to fail, the same tradeoff as `create_package`'s
    /// genuinely-missing project. A failed state read is just
    /// "state unknown" —
    /// the same backend that 500s the POST can 500 the follow-up
    /// GET — and the retry keeps going until the deadline, which
    /// still fails loudly. The retry sleeps 500 ms between
    /// attempts, so a request that never changes state still fails
    /// loudly once the budget runs out instead of hanging.
    ///
    /// The state check is what makes this retry safe where the
    /// HTTP layer's POST policy is not: a POST is never retried
    /// there, because a retried mutation can duplicate — but here
    /// the read-back proves whether the change already landed, so
    /// a mutation that went through is never re-issued. The check is
    /// end-state-based, not attribution-based: it proves the request
    /// is in the target state, not that this call's POST put it
    /// there — if another actor moved it first, this call's comment
    /// is never posted, and the request is already where the caller
    /// wanted it. There is a narrow window between the state read
    /// and the re-POST: if another actor moves the request in
    /// between, the re-POST can fail fast (a 403 on an
    /// already-accepted request, for example) even though the request
    /// is in the target state; the window is one sleep wide and the
    /// caller's flow is sequential, so the retry documents it rather
    /// than closing it. A 200 with a non-ok body keeps the
    /// pre-existing fail-fast: the retry covers transport-level
    /// transient failures, and a non-ok body is read as the server
    /// refusing the change. Callers pass the budget;
    /// [`ObsClient::change_state`] passes 60 s.
    fn post_changestate_with_retry(
        &self,
        url: &str,
        comment: &str,
        id: u64,
        state: RequestState,
        budget: std::time::Duration,
    ) -> Result<(), GeectlError> {
        let deadline = std::time::Instant::now() + budget;
        let mut probe = 0;
        loop {
            match self.http.post_text(url, comment, &self.credentials) {
                Ok(body) => {
                    if parse_status_ok(&body) {
                        return Ok(());
                    }
                    return Err(GeectlError::new(
                        ErrorKind::ServerError,
                        format!("unexpected OBS changestate response for submit request {id}"),
                    ));
                }
                Err(err)
                    if matches!(
                        err.kind(),
                        ErrorKind::ServerError
                            | ErrorKind::Timeout
                            | ErrorKind::NetworkError
                            | ErrorKind::NotFound
                    ) =>
                {
                    // The 500 can come after the state change
                    // landed, so check before re-issuing: a request
                    // already in the target state means the change
                    // went through, and the retry returns success
                    // without posting again. The check is a decision
                    // point on the debug channel, and the probe that
                    // carries it reports its number against the
                    // retry budget.
                    match self.show_request(&id.to_string()) {
                        Ok(request) if request.state.name == state => {
                            self.debug.emit(&format!(
                                "decision: request {id} already {}; change landed, not re-issuing",
                                state.as_str()
                            ));
                            return Ok(());
                        }
                        Ok(request) => {
                            probe += 1;
                            self.debug.emit(&format!(
                                "retry: change_state request {id} probe {probe}/{}: still {}",
                                Self::format_budget(budget),
                                request.state.name.as_str()
                            ));
                            self.debug.emit(&format!(
                                "decision: request {id} still {}, re-issuing {}",
                                request.state.name.as_str(),
                                state.as_str()
                            ));
                        }
                        Err(_) => {
                            probe += 1;
                            self.debug.emit(&format!(
                                "retry: change_state request {id} probe {probe}/{}: state unknown",
                                Self::format_budget(budget)
                            ));
                            self.debug.emit(&format!(
                                "decision: request {id} state unknown, re-issuing {}",
                                state.as_str()
                            ));
                        }
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err(err);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// Change submit request `id` to `state` (`POST
    /// /request/:id?cmd=changestate&newstate=<state>`), with
    /// `comment` as the message left with the change. Requires
    /// credentials; without them the call fails loudly before any
    /// request. Takes the caller's [`DryRun`]: in dry-run the change
    /// is recorded in the plan and nothing changes, so a dry-run
    /// caller cannot mutate the server by invoking this directly.
    ///
    /// Semantics, verified against the OBS API server
    /// (`BsRequest#change_state` and
    /// `BsRequestPermissionCheck#cmd_changestate_permissions`) and
    /// the live API — not taken from anywhere else:
    /// - The server does not pre-check here, and neither does the
    ///   library: the server stays the authority on the transition,
    ///   and its 403 is surfaced as-is. A request in `accepted`
    ///   cannot change state at all, and a final state cannot move to
    ///   another final state.
    /// - Accepting requires write permission on the target, and the
    ///   server can forbid self-accept outright (the
    ///   `OBS:CreatorCannotAcceptOwnRequests` attribute, set on the
    ///   target package or project). An agent accepts its own
    ///   request only when the target allows it; the server's 403
    ///   says so when it does not.
    /// - Accepting a request in `review` state requires `force`
    ///   (`force=1`): without it the server refuses. `force` is only
    ///   honoured for accept; it is ignored for the other
    ///   transitions, so the wrappers for decline and revoke do not
    ///   take one.
    /// - `superseded` is refused here: superseding needs a
    ///   replacement id, and [`ObsClient::supersede_request`] is the
    ///   path that carries it. `deleted` is refused here as well:
    ///   deleting a request is an admin-only server operation, and
    ///   revoking is the agent's way to withdraw one.
    /// - Re-opening a request to `new` or `review` resets its
    ///   non-accepted reviews to `new` and forces the state to
    ///   `review` when reviews are still open; that is the server's
    ///   behaviour, not the library's.
    /// - Success is the `<status code="ok"/>` body: any other body is
    ///   a server error, never success.
    /// - The POST retries transient failures for a bounded 60 s:
    ///   the server's API replies before its backend has
    ///   registered a just-filed request, so the changestate POST
    ///   that follows a filing can fail while the request is about
    ///   to exist (seen on live run 44880250442: the accept
    ///   POST returned 500, while the same accept passed on
    ///   earlier runs). Before any re-issue the retry reads the
    ///   request back, so a change that already landed is never
    ///   posted again; a request that never changes state still
    ///   fails loudly once the budget runs out. Only transient
    ///   failures retry — anything else fails fast, except a
    ///   well-formed id that does not exist, which 404s and retries
    ///   for up to 60 s (the same tradeoff as a genuinely-missing
    ///   project for `create_package`).
    pub fn change_state(
        &self,
        id: &str,
        state: RequestState,
        comment: &str,
        force: bool,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        let id = validate_request_id(id)?;
        let state_name = match state {
            RequestState::New
            | RequestState::Review
            | RequestState::Accepted
            | RequestState::Declined
            | RequestState::Revoked => state.as_str(),
            RequestState::Superseded => {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "superseding a submit request needs a replacement id; use supersede_request",
                ));
            }
            RequestState::Deleted => {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "deleting a submit request is an admin-only server operation; revoke it instead",
                ));
            }
        };
        if !dry_run.allow(&format!(
            "change state of submit request {id} to {state_name}"
        )) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot change a submit request's state without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let mut query: Vec<(&str, &str)> = vec![("cmd", "changestate"), ("newstate", state_name)];
        if force {
            query.push(("force", "1"));
        }
        let url = build_url(&self.base, &["request", &id.to_string()], &query);
        // The comment travels as the request body, as plain text:
        // the server's `RequestController#request_command` takes
        // `request.raw_post` as the comment (see
        // `supersede_request`), so the body is the comment verbatim
        // and its content type is its actual type (`text/plain`).
        self.post_changestate_with_retry(
            &url,
            comment,
            id,
            state,
            std::time::Duration::from_secs(60),
        )
    }

    /// Accept submit request `id` (`POST
    /// /request/:id?cmd=changestate&newstate=accepted`), with
    /// `comment` as the message left with the accept. An agent
    /// accepts a request it reviewed and agrees with — the change
    /// lands, and the source branch dies on accept when the request
    /// was filed with cleanup (the default). `force` is for
    /// accepting a request that is still in `review` state: the
    /// server refuses that without `force=1`, so passing it is a
    /// deliberate override of the open reviews, not a default. See
    /// [`ObsClient::change_state`] for the semantics.
    pub fn accept_request(
        &self,
        id: &str,
        comment: &str,
        force: bool,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        self.change_state(id, RequestState::Accepted, comment, force, dry_run)
    }

    /// Decline submit request `id` (`POST
    /// /request/:id?cmd=changestate&newstate=declined`), with
    /// `comment` as the reason. An agent declines a request it
    /// reviewed and disagrees with; the reason is the review, so a
    /// decline without one is possible but the caller should say
    /// why. See [`ObsClient::change_state`] for the semantics.
    pub fn decline_request(
        &self,
        id: &str,
        comment: &str,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        self.change_state(id, RequestState::Declined, comment, false, dry_run)
    }

    /// Revoke submit request `id` (`POST
    /// /request/:id?cmd=changestate&newstate=revoked`), with
    /// `comment` as the reason. Revoking is how a request is
    /// withdrawn — by its creator, or by anyone the server allows to
    /// withdraw it. See [`ObsClient::change_state`] for the semantics.
    pub fn revoke_request(
        &self,
        id: &str,
        comment: &str,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        self.change_state(id, RequestState::Revoked, comment, false, dry_run)
    }

    /// Best-effort [`ObsClient::change_state`]: try the state change,
    /// and return whether it happened. A transport or server failure
    /// — the request is already closed, the credentials do not allow
    /// the change, the server is unreachable — reports the failure
    /// on the debug channel (gated on `--debug`) and returns `false`
    /// instead of failing, so a batch
    /// triaging many requests does not stop at the first one that
    /// cannot move. A caller bug does not get swallowed: a malformed
    /// id or state, and missing credentials, still return an error.
    /// In dry-run the change is recorded in the plan (as
    /// [`ObsClient::change_state`] does) and this returns `true`.
    pub fn try_change_state(
        &self,
        id: &str,
        state: RequestState,
        comment: &str,
        force: bool,
        dry_run: &DryRun,
    ) -> Result<bool, GeectlError> {
        match self.change_state(id, state, comment, force, dry_run) {
            Ok(()) => Ok(true),
            Err(err)
                if matches!(
                    err.kind(),
                    ErrorKind::NotFound
                        | ErrorKind::AuthFailed
                        | ErrorKind::ServerError
                        | ErrorKind::Timeout
                        | ErrorKind::NetworkError
                        | ErrorKind::TlsError
                ) =>
            {
                self.debug.emit(&format!(
                    "decision: could not change state of submit request {id}: {err}"
                ));
                Ok(false)
            }
            Err(err) => Err(err),
        }
    }

    /// Ask `target` to review submit request `id` (`POST
    /// /request/:id?cmd=addreview`), with `comment` as the reason the
    /// review is asked for. The review moves the request to `review`
    /// state. An agent asks for a review when a request needs eyes it
    /// does not have — a maintainer of the target, a group that owns
    /// the area — and the reason is the ask, so an empty comment is
    /// refused: a review that says nothing about what it wants is not
    /// a review. Requires credentials; without them the call fails
    /// loudly before any request. Takes the caller's [`DryRun`]: in
    /// dry-run the review is recorded in the plan and nothing is
    /// asked.
    ///
    /// The target is validated up front ([`ReviewTarget::new`]): at
    /// least one of a user, a group, and a project must be named, and
    /// a package review needs its project. Success is the `<status
    /// code="ok"/>` body: any other body is a server error, never
    /// success.
    pub fn add_review(
        &self,
        id: &str,
        target: &ReviewTarget,
        comment: &str,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        let id = validate_request_id(id)?;
        if comment.is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "a review needs a reason; an empty comment asks for a review that says nothing",
            ));
        }
        if !dry_run.allow(&format!(
            "add a review by {} to submit request {id}",
            target.describe()
        )) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot add a review to a submit request without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let mut query: Vec<(&str, &str)> = vec![("cmd", "addreview")];
        if let Some(by_user) = &target.by_user {
            query.push(("by_user", by_user));
        }
        if let Some(by_group) = &target.by_group {
            query.push(("by_group", by_group));
        }
        if let Some(by_project) = &target.by_project {
            query.push(("by_project", by_project));
        }
        if let Some(by_package) = &target.by_package {
            query.push(("by_package", by_package));
        }
        let url = build_url(&self.base, &["request", &id.to_string()], &query);
        // The reason travels as the request body, as plain text:
        // the server's `RequestController#request_command` takes
        // `request.raw_post` as the comment (see
        // `supersede_request`), so the body is the reason verbatim
        // and its content type is its actual type (`text/plain`).
        let body = self.http.post_text(&url, comment, &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS addreview response for submit request {id}"),
            ))
        }
    }

    /// List the comments on submit request `id` (`GET
    /// /comments/request/:id`): who said what, when, and which
    /// comment each reply answers. The list covers the comments on
    /// the request and the comments on its actions, in id order.
    /// Requires credentials; without them the call fails loudly
    /// before any request. A request with no comments reads as an
    /// empty list, not an error; a document the parser does not
    /// recognise in full is a server error.
    pub fn list_comments(&self, id: &str) -> Result<Vec<RequestComment>, GeectlError> {
        let id = validate_request_id(id)?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot list a submit request's comments without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(&self.base, &["comments", "request", &id.to_string()], &[]);
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_comments(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS comments response for submit request {id}"),
            )
        })
    }

    /// Comment on submit request `id` (`POST
    /// /comments/request/:id`), with `parent_id` naming the comment
    /// being replied to, when this is a reply. An agent comments to
    /// discuss a request — asking a question, leaving a note for the
    /// reviewer — without changing its state. The comment text
    /// travels as the request body, as plain text; an empty comment
    /// is refused, since a comment that says nothing is not a
    /// comment. Requires credentials; without them the call fails
    /// loudly before any request. Takes the caller's [`DryRun`]: in
    /// dry-run the comment is recorded in the plan and nothing is
    /// posted. Success is the `<status code="ok"/>` body: any other
    /// body is a server error, never success.
    pub fn add_comment(
        &self,
        id: &str,
        body: &str,
        parent_id: Option<&str>,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        let id = validate_request_id(id)?;
        // The parent is a comment id: the same positive-number shape
        // as a request id, so the same check applies.
        let parent = match parent_id {
            Some(parent) => Some(validate_request_id(parent)?),
            None => None,
        };
        if body.is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "a comment with no text says nothing; refusing an empty comment",
            ));
        }
        if !dry_run.allow(&format!("comment on submit request {id}")) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot comment on a submit request without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let parent_string;
        let mut query: Vec<(&str, &str)> = Vec::new();
        if let Some(parent) = parent {
            parent_string = parent.to_string();
            query.push(("parent_id", &parent_string));
        }
        let url = build_url(
            &self.base,
            &["comments", "request", &id.to_string()],
            &query,
        );
        let response = self.http.post_text(&url, body, &self.credentials)?;
        if parse_status_ok(&response) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS comment response for submit request {id}"),
            ))
        }
    }

    /// Read the diff of submit request `id` (`POST
    /// /request/:id?cmd=diff`): what the request changes, as a
    /// unified diff — or, with `view_xml`, as the request XML with
    /// the diff injected into it. An agent reads the diff to review
    /// a request before acting on it: accepting or declining without
    /// seeing the change is not a review. Requires credentials;
    /// without them the call fails loudly before any request. The
    /// diff is returned as the raw response body, unparsed: it is
    /// caller-consumed text, not a structure the library acts on.
    ///
    /// The diff is fetched with a POST because that is the only way
    /// the API offers it — but it changes nothing on the server, so
    /// it takes no [`DryRun`]: a dry-run caller reading a diff is
    /// still only reading.
    pub fn request_diff(&self, id: &str, view_xml: bool) -> Result<String, GeectlError> {
        let id = validate_request_id(id)?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot read a submit request's diff without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let mut query: Vec<(&str, &str)> = vec![("cmd", "diff")];
        if view_xml {
            query.push(("view", "xml"));
        }
        let url = build_url(&self.base, &["request", &id.to_string()], &query);
        self.http.post_text(&url, "", &self.credentials)
    }

    /// List the submit requests matching `search` (`GET
    /// /search/request`): the typed list-my-requests operation. An
    /// agent lists requests to triage — its own open requests, the
    /// open requests targeting a package, the requests waiting on a
    /// reviewer — and each result is a whole [`Request`], parsed in
    /// full, so the caller can filter further without another round
    /// trip. Requires credentials; without them the call fails loudly
    /// before any request. An unexpected response is an error, never
    /// an empty list.
    pub fn search_requests(&self, search: &RequestSearch) -> Result<Vec<Request>, GeectlError> {
        let match_expr = search.match_expr()?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot search submit requests without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let mut query: Vec<(&str, &str)> = vec![("match", &match_expr)];
        if search.wants_history() {
            query.push(("withhistory", "1"));
        }
        if search.wants_full_history() {
            query.push(("withfullhistory", "1"));
        }
        let url = build_url(&self.base, &["search", "request"], &query);
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_request_collection(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS request search response",
            )
        })
    }

    /// Refuse a credentialed build read without OBS Basic
    /// credentials, before any request: every `/build` endpoint
    /// requires Basic authentication, so a call without credentials
    /// would 401 — a failed lookup must fail closed, never read as
    /// "no build state".
    fn require_build_credentials(&self) -> Result<(), GeectlError> {
        if self.has_basic() {
            return Ok(());
        }
        Err(GeectlError::new(
            ErrorKind::ConfigError,
            "reading build state requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
        ))
    }

    /// The build-results URL for `project` and `query`: `GET
    /// /build/<project>/_result` with the query's filters, in a fixed
    /// order (package, repositories, architectures, code, multibuild,
    /// locallink, lastbuild, oldstate).
    fn results_url(&self, project: &str, query: &ResultsQuery) -> String {
        let mut pairs: Vec<(&str, &str)> = Vec::new();
        if let Some(package) = &query.package {
            pairs.push(("package", package));
        }
        for repository in &query.repositories {
            pairs.push(("repository", repository));
        }
        for arch in &query.arches {
            pairs.push(("arch", arch));
        }
        if let Some(code) = &query.code {
            pairs.push(("code", code));
        }
        if query.multibuild {
            pairs.push(("multibuild", "1"));
        }
        if query.locallink {
            pairs.push(("locallink", "1"));
        }
        if query.lastbuild {
            pairs.push(("lastbuild", "1"));
        }
        if let Some(oldstate) = &query.oldstate {
            pairs.push(("oldstate", oldstate));
        }
        build_url(&self.base, &["build", project, "_result"], &pairs)
    }

    /// The build results for `project`: one entry per
    /// repository/architecture cell, each with the per-package build
    /// codes. The query filters by package, repository, architecture,
    /// or package build code; `multibuild` + `locallink` expand
    /// multibuild flavors into their own statuses, which is the
    /// per-package form. A package-filtered query omits a filtered
    /// package with no status instead of reporting it `unknown` (a
    /// server-side typo in `getresult`), so a never-scheduled
    /// package silently drops out of a filtered read. A filtered
    /// [`ObsClient::wait_for_results`] does not treat that absence as
    /// settled: it keeps polling until the package appears or the
    /// deadline passes.
    /// Requires OBS Basic credentials. That is a client policy, not
    /// something the endpoint forces: the endpoint allows anonymous
    /// reads when the instance permits them, so without credentials
    /// the call fails closed instead of reading anonymously.
    /// A failed lookup is an error, never an empty matrix.
    pub fn results(
        &self,
        project: &str,
        query: &ResultsQuery,
    ) -> Result<BuildResults, GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        if let Some(package) = &query.package {
            if !valid_obs_name(package) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS package name: {package:?}"),
                ));
            }
        }
        for (label, values) in [
            ("repository", &query.repositories),
            ("architecture", &query.arches),
        ] {
            for value in values {
                if !valid_obs_name(value) {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!("invalid OBS {label} name: {value:?}"),
                    ));
                }
            }
        }
        if let Some(oldstate) = &query.oldstate {
            if oldstate.is_empty() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "invalid OBS results state hash: empty oldstate".to_string(),
                ));
            }
        }
        self.require_build_credentials()?;
        let url = self.results_url(project, query);
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_build_results(&body)
    }

    /// Poll the build results for `project` until they settle — no
    /// cell dirty, no cell or package in a waiting state (`blocked`,
    /// `scheduled`, `dispatching`, `building`, `signing`, `finished`)
    /// — or `deadline` passes. With a package filter, the package
    /// must also appear in the matrix: a never-scheduled package drops
    /// out of a filtered read, and that absence is not settled, so the
    /// poll keeps going until the package appears or the deadline
    /// passes. The poll reads the matrix every two seconds (less when
    /// the deadline is nearer, so a sleep never overshoots it); it
    /// deliberately does not send `oldstate`, even though
    /// the server holds an unchanged `oldstate` reply for up to five
    /// minutes: past the transport's 30-second read timeout, so an
    /// `oldstate` poll would time out instead of settling. Plain
    /// polling keeps the deadline honest; use
    /// `ResultsQuery.oldstate` with [`ObsClient::results`] directly
    /// for a hand-rolled long poll. On a deadline the error is a
    /// timeout: call [`ObsClient::results`] again for the current
    /// state instead of assuming.
    pub fn wait_for_results(
        &self,
        project: &str,
        query: &ResultsQuery,
        deadline: std::time::Duration,
    ) -> Result<BuildResults, GeectlError> {
        // Plain polling only: a caller-set `oldstate` would make the
        // server hold an unchanged reply for up to five minutes, past
        // the transport's read timeout.
        let query = ResultsQuery {
            oldstate: None,
            ..query.clone()
        };
        let start = std::time::Instant::now();
        loop {
            let fetched = self.results(project, &query)?;
            if results_settled(&fetched, query.package.as_deref()) {
                return Ok(fetched);
            }
            if start.elapsed() >= deadline {
                return Err(GeectlError::new(
                    ErrorKind::Timeout,
                    format!("build results for {project} did not settle within {deadline:?}"),
                ));
            }
            // Sleep at most two seconds, and never past the deadline:
            // a fixed sleep would overshoot a near deadline.
            let remaining = deadline.saturating_sub(start.elapsed());
            std::thread::sleep(std::cmp::min(std::time::Duration::from_secs(2), remaining));
        }
    }

    /// Validate the coordinates of a build read (project, package,
    /// repository, architecture): each must be a valid OBS name, so a
    /// `/`, `?`, or quote can never rewrite the URL. The package is
    /// optional: the repository-wide reads take None.
    fn validate_build_coordinates(
        project: &str,
        package: Option<&str>,
        repository: &str,
        arch: &str,
    ) -> Result<(), GeectlError> {
        let mut names: Vec<(&str, &str)> = vec![
            ("project", project),
            ("repository", repository),
            ("architecture", arch),
        ];
        if let Some(package) = package {
            names.push(("package", package));
        }
        for (label, value) in names {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        Ok(())
    }

    /// The build log for `package` in `repository`/`arch`: the raw
    /// log text. `nostream=1` reads the whole log at once (no
    /// streaming); the query selects the slice. None when there is no
    /// log (404): the package was never built, or the package/project
    /// does not exist — the server answers 404 for both, so the caller
    /// checks the package exists when the distinction matters.
    /// Requires OBS Basic credentials. The log is capped by the
    /// transport's body limit, like every read.
    pub fn build_log(
        &self,
        project: &str,
        package: &str,
        repository: &str,
        arch: &str,
        query: &BuildLogQuery,
    ) -> Result<Option<String>, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        self.require_build_credentials()?;
        let start = query.start.map(|offset| offset.to_string());
        let mut pairs: Vec<(&str, &str)> = vec![("nostream", "1")];
        if let Some(start) = &start {
            pairs.push(("start", start));
        }
        if query.last {
            pairs.push(("last", "1"));
        }
        if query.lastsucceeded {
            pairs.push(("lastsucceeded", "1"));
        }
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, package, "_log"],
            &pairs,
        );
        match self.http.get_with(&url, &self.credentials) {
            Ok(body) => Ok(Some(body)),
            // A 404 means no log: never built, or the package/project
            // does not exist. Any other failure stays an error.
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The build status code for `package` in `repository`/`arch`
    /// (`succeeded`, `failed`, `unresolvable`, ...), or None when the
    /// package has no status: the server answers `code="unknown"`
    /// with 200 when nothing is known, and 404 when the
    /// package/project does not exist. Requires OBS Basic credentials.
    pub fn build_status(
        &self,
        project: &str,
        package: &str,
        repository: &str,
        arch: &str,
    ) -> Result<Option<String>, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        self.require_build_credentials()?;
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, package, "_status"],
            &[],
        );
        let body = match self.http.get_with(&url, &self.credentials) {
            Ok(body) => body,
            // A 404 means the package/project does not exist. Any
            // other failure stays an error.
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let code = parse_build_status(&body)?;
        // The server's "no status known" is `code="unknown"` with a
        // 200, not a 404: it reads as None, like the 404 above.
        if code == "unknown" {
            return Ok(None);
        }
        Ok(Some(code))
    }

    /// The running build job for `package` in `repository`/`arch`,
    /// or None when no job is running. Requires OBS Basic credentials.
    /// A 404 is an error (unknown project/package): unlike the log and
    /// the status, a missing job reads as an empty jobstatus element,
    /// not a 404, so a 404 here names something that does not exist.
    pub fn job_status(
        &self,
        project: &str,
        package: &str,
        repository: &str,
        arch: &str,
    ) -> Result<Option<JobStatus>, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        self.require_build_credentials()?;
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, package, "_jobstatus"],
            &[],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_job_status(&body)
    }

    /// The buildinfo for `package` in `repository`/`arch`: the error
    /// blocking the build, if any, and the dependencies the build
    /// pulls in. An unresolvable build surfaces its missing
    /// dependencies as the error, as a comma-separated list. Requires
    /// OBS Basic credentials. A 404 is an error (unknown
    /// project/package): a buildinfo with no error and no dependencies
    /// reads as an empty document, not a 404.
    pub fn buildinfo(
        &self,
        project: &str,
        package: &str,
        repository: &str,
        arch: &str,
    ) -> Result<BuildInfo, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        self.require_build_credentials()?;
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, package, "_buildinfo"],
            &[],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_buildinfo(&body)
    }

    /// The build history for `package` in `repository`/`arch`: the
    /// completed builds, newest first, up to `limit` entries. An empty
    /// history (never built) reads as an empty list. Requires OBS
    /// Basic credentials. A 404 is an error (unknown project/package).
    pub fn build_history(
        &self,
        project: &str,
        package: &str,
        repository: &str,
        arch: &str,
        limit: Option<u32>,
    ) -> Result<Vec<BuildHistoryEntry>, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        self.require_build_credentials()?;
        let limit = limit.map(|n| n.to_string());
        let mut pairs: Vec<(&str, &str)> = Vec::new();
        if let Some(limit) = &limit {
            pairs.push(("limit", limit));
        }
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, package, "_history"],
            &pairs,
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_build_history(&body)
    }

    /// The job history for `repository`/`arch`: the build jobs that
    /// ran there, newest first, optionally limited to `package`,
    /// `limit` entries, and a result `code`. `lastfailures` is
    /// rejected: it is a special server mode returning per-package
    /// success-then-failure pairs, not a chronological history. Use
    /// it to check a green build is from the
    /// current source revision, not a stale rebuild: match the entry's
    /// `srcmd5` against the package's current revision. An empty
    /// history reads as an empty list. Requires OBS Basic credentials.
    /// A 404 is an error (unknown project/repository/architecture).
    pub fn job_history(
        &self,
        project: &str,
        repository: &str,
        arch: &str,
        package: Option<&str>,
        limit: Option<u32>,
        code: Option<&str>,
    ) -> Result<Vec<JobHistoryEntry>, GeectlError> {
        Self::validate_build_coordinates(project, package, repository, arch)?;
        self.require_build_credentials()?;
        if let Some(code) = code {
            if code.is_empty() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "invalid OBS job history code: empty".to_string(),
                ));
            }
            if code == "lastfailures" {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "invalid OBS job history code: lastfailures is a special server mode, not a result filter".to_string(),
                ));
            }
        }
        let limit = limit.map(|n| n.to_string());
        let mut pairs: Vec<(&str, &str)> = Vec::new();
        if let Some(package) = package {
            pairs.push(("package", package));
        }
        if let Some(limit) = &limit {
            pairs.push(("limit", limit));
        }
        if let Some(code) = code {
            pairs.push(("code", code));
        }
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, "_jobhistory"],
            &pairs,
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_job_history(&body)
    }

    /// The built files for `package` in `repository`/`arch`: the
    /// binary RPMs plus the build artifacts (`_buildenv`,
    /// `_statistics`, `rpmlint.log`, the `.src.rpm`, `-debuginfo` and
    /// `-debugsource` when built). With `package` None, the whole
    /// repository's files (`_repository`). An empty list means no
    /// binaries: a never-built existing package answers 200 with an
    /// empty list. A 404 is an error (the package or project does not
    /// exist), never an empty list: a misspelled package must not read
    /// as "nothing built".
    /// Requires OBS Basic credentials. A multibuild flavor travels as
    /// `pkg:flavor`.
    pub fn list_binaries(
        &self,
        project: &str,
        package: Option<&str>,
        repository: &str,
        arch: &str,
    ) -> Result<Vec<BinaryFile>, GeectlError> {
        Self::validate_build_coordinates(project, package, repository, arch)?;
        self.require_build_credentials()?;
        let package = package.unwrap_or("_repository");
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, package],
            &[],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_binary_list(&body)
    }

    /// Download one built file (`filename`) for `package` in
    /// `repository`/`arch`: the raw bytes. Use it for a binary RPM or
    /// a build artifact by name (`rpmlint.log`). The download is
    /// capped at 10 MiB by the transport's body limit, like every
    /// read: a file larger than 10 MiB is an error, not a truncated
    /// download. Requires OBS Basic credentials. A 404 is an error
    /// (the file does not exist).
    pub fn download_binary(
        &self,
        project: &str,
        package: &str,
        repository: &str,
        arch: &str,
        filename: &str,
    ) -> Result<Vec<u8>, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        if !valid_obs_filename(filename) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS binary filename: {filename:?}"),
            ));
        }
        self.require_build_credentials()?;
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, package, filename],
            &[],
        );
        self.http.get_bytes(&url, &self.credentials)
    }

    /// The metadata of a built file (`filename`) for `package` in
    /// `repository`/`arch`: the RPM header fields plus the dependency
    /// lists. With `ext`, the `fileinfo_ext` view: each required
    /// dependency names the packages providing it, and each provided
    /// dependency names the packages requiring it. Requires OBS Basic
    /// credentials. A 404 is an error (the file does not exist).
    pub fn fileinfo(
        &self,
        project: &str,
        package: &str,
        repository: &str,
        arch: &str,
        filename: &str,
        ext: bool,
    ) -> Result<FileInfo, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        if !valid_obs_filename(filename) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS binary filename: {filename:?}"),
            ));
        }
        self.require_build_credentials()?;
        let view = if ext { "fileinfo_ext" } else { "fileinfo" };
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, package, filename],
            &[("view", view)],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_fileinfo(&body)
    }

    /// The build dependencies of the packages in `repository`/`arch`:
    /// which packages each package depends on for its build
    /// (whole-package dependencies, not individual binaries). With
    /// `package`, only that package's dependencies. Requires OBS Basic
    /// credentials. A 404 is an error (unknown
    /// project/repository/architecture). An empty list means the
    /// repository has no packages with recorded dependencies.
    pub fn dependson(
        &self,
        project: &str,
        repository: &str,
        arch: &str,
        package: Option<&str>,
    ) -> Result<Vec<PackageDeps>, GeectlError> {
        Self::validate_build_coordinates(project, package, repository, arch)?;
        self.require_build_credentials()?;
        let mut pairs: Vec<(&str, &str)> = vec![("view", "pkgnames")];
        if let Some(package) = package {
            pairs.push(("package", package));
        }
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, "_builddepinfo"],
            &pairs,
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_builddepinfo(&body)
    }

    /// The packages in `repository`/`arch` that depend on `package`
    /// for their build: a rebuild of `package` triggers a rebuild of
    /// each of them. The reverse of [`ObsClient::dependson`]: each
    /// entry's `dependencies` are the packages depending on the
    /// queried package, not the queried package's own dependencies.
    /// Requires OBS Basic credentials. A 404 is an error (unknown
    /// project/repository/architecture).
    pub fn whatdependson(
        &self,
        project: &str,
        repository: &str,
        arch: &str,
        package: &str,
    ) -> Result<Vec<PackageDeps>, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        self.require_build_credentials()?;
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, "_builddepinfo"],
            &[("view", "revpkgnames"), ("package", package)],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_builddepinfo(&body)
    }

    /// The worker ids that satisfy `constraints` for `package` in
    /// `repository`/`arch`: a query, not a build — it creates or
    /// modifies nothing, so it takes no dry-run guard. `constraints`
    /// is the package's constraints XML (`_constraints`), passed
    /// through as authored: there is no typed constraints model yet,
    /// so this is the one place a raw XML document crosses the
    /// public API. None posts an empty body, which the server
    /// accepts. An empty list means no worker satisfies the
    /// constraints (or no worker is registered). Requires OBS Basic
    /// credentials.
    pub fn check_constraints(
        &self,
        project: &str,
        package: &str,
        repository: &str,
        arch: &str,
        constraints: Option<&str>,
    ) -> Result<Vec<String>, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        if let Some(constraints) = constraints {
            if constraints.is_empty() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "invalid OBS constraints: empty".to_string(),
                ));
            }
        }
        self.require_build_credentials()?;
        let url = build_url(
            &self.base,
            &["worker"],
            &[
                ("cmd", "checkconstraints"),
                ("project", project),
                ("package", package),
                ("repository", repository),
                ("arch", arch),
            ],
        );
        let body = self
            .http
            .post(&url, constraints.unwrap_or(""), &self.credentials)?;
        // The response is a `<directory>` of the matching workers,
        // one `<entry name="workerid"/>` each.
        parse_directory_entry_names(&body)
    }

    /// The repositories of `project`: their names, architectures, and
    /// the repositories they build against. A public read: project
    /// meta is public, so no credentials are needed. A 404 is an
    /// error (unknown project). An empty list means the project has no
    /// repositories.
    pub fn list_repositories(&self, project: &str) -> Result<Vec<Repository>, GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        let url = build_url(&self.base, &["public", "source", project, "_meta"], &[]);
        let body = self.http.get_with(&url, &Credentials::None)?;
        parse_repositories(&body)
    }
}

/// Validate an OBS request id: a non-empty run of ASCII digits that
/// reads as a positive number. Returns the canonical number, so
/// `old != new` is a numeric comparison (a leading-zero spelling of
/// the same id still counts as the same request). The API only checks
/// the id is a positive number (`check_supersede_state`); the shape
/// check here keeps a malformed id from ever reaching a URL.
fn validate_request_id(id: &str) -> Result<u64, GeectlError> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid OBS request id: {id:?}"),
        ));
    }
    let number = id.parse::<u64>().map_err(|_| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid OBS request id: {id:?}"),
        )
    })?;
    if number == 0 {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid OBS request id: {id:?}"),
        ));
    }
    Ok(number)
}

/// The build-results matrix for a project: one entry per
/// repository/architecture cell, plus the state hash for polling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildResults {
    /// The state hash: pass it back as [`ResultsQuery::oldstate`] to
    /// poll for changes. None when the server sent no state.
    pub state: Option<String>,
    /// One entry per repository/architecture cell.
    pub results: Vec<BuildResult>,
}

/// One repository/architecture cell of a build-results matrix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildResult {
    /// The project the cell belongs to.
    pub project: String,
    /// The repository the cell belongs to.
    pub repository: String,
    /// The architecture the cell belongs to.
    pub arch: String,
    /// The cell's build state (`published`, `broken`, `blocked`,
    /// `scheduling`, ...). This is the authoritative state; `state`
    /// is its legacy name.
    pub code: String,
    /// The legacy name of `code`, still sent by the server; equal to
    /// `code` when the server omits it.
    pub state: String,
    /// The cell is marked for re-scheduling: its state may already be
    /// stale. A result is settled only once no cell is dirty.
    pub dirty: bool,
    /// The server's detail text for the cell (e.g. why a repository
    /// is `broken`), if any.
    pub details: Option<String>,
    /// The per-package build codes in this cell.
    pub statuses: Vec<PackageBuildStatus>,
}

/// One package's build code inside a [`BuildResult`] cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageBuildStatus {
    /// The package.
    pub package: String,
    /// The package's build code (`succeeded`, `failed`,
    /// `unresolvable`, `disabled`, ...).
    pub code: String,
    /// The server's detail text for the package (e.g. the missing
    /// dependency for an `unresolvable` package), if any.
    pub details: Option<String>,
}

/// Query options for [`ObsClient::results`]. Every field is an
/// optional filter; an empty query returns the whole project's
/// matrix. `multibuild` + `locallink` are the per-package form: they
/// expand multibuild flavors into their own statuses, which is what
/// an agent asking about one package wants.
#[derive(Debug, Default, Clone)]
pub struct ResultsQuery {
    /// Only this package's statuses (the repository/architecture
    /// cells still come back whole).
    pub package: Option<String>,
    /// Only these repositories (one `repository` pair per entry).
    pub repositories: Vec<String>,
    /// Only these architectures (one `arch` pair per entry).
    pub arches: Vec<String>,
    /// Only packages with this build code (e.g. `failed`). The
    /// filter applies to each package's code, not to the
    /// repository/architecture cell's state.
    pub code: Option<String>,
    /// Expand multibuild flavors into their own statuses. Older
    /// servers without multibuild support reject the query; geectl
    /// does not fall back to a plain query.
    pub multibuild: bool,
    /// Follow local links when expanding multibuild flavors.
    pub locallink: bool,
    /// The last build's results instead of the current state.
    pub lastbuild: bool,
    /// Poll for changes since this state hash (from a previous
    /// [`BuildResults::state`]); set it only to poll by hand. A hand
    /// poll with `oldstate` may time out at the transport level when
    /// the state does not change within the read timeout: the server
    /// holds the reply for up to five minutes.
    pub oldstate: Option<String>,
}

/// Build states that are still in flight: a cell or a package in one
/// of these states — or a cell marked dirty — means the build has not
/// settled yet. The set is exactly the waiting set the reference
/// client's `--watch` loop polls on.
const WAITING_BUILD_CODES: [&str; 6] = [
    "blocked",
    "scheduled",
    "dispatching",
    "building",
    "signing",
    "finished",
];

/// True when no cell is dirty and no cell or package sits in a
/// waiting state: the build has settled. With `package`, the package
/// must also appear in the matrix: a package-filtered read drops a
/// never-scheduled package instead of reporting it `unknown`, so
/// absence is not settled — the wait keeps polling until the package
/// appears or the deadline passes. Without a filter, an empty matrix
/// (a project with no repositories) is settled.
fn results_settled(results: &BuildResults, package: Option<&str>) -> bool {
    if let Some(package) = package {
        if !results
            .results
            .iter()
            .flat_map(|result| result.statuses.iter())
            .any(|status| status.package == package)
        {
            return false;
        }
    }
    results.results.iter().all(|result| {
        !result.dirty
            && !WAITING_BUILD_CODES.contains(&result.code.as_str())
            && result
                .statuses
                .iter()
                .all(|status| !WAITING_BUILD_CODES.contains(&status.code.as_str()))
    })
}

/// Parse a `<resultlist>` document into typed build results. A
/// document that is not a resultlist — or a `<result>` / `<status>`
/// missing a required attribute — is a server error, never an empty
/// matrix: a failed lookup must not read as "nothing is building".
pub(crate) fn parse_build_results(body: &str) -> Result<BuildResults, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS build results response: not a <resultlist> document",
        )
    };
    let body = body.trim();
    let Some(after_open) = body.strip_prefix("<resultlist") else {
        return Err(malformed());
    };
    // The tag name must end here: `<resultlistx>` is not a resultlist.
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    let state = xml_attr(open_tag, "state");
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<resultlist .../>`: a project with no
        // repositories, so no cells.
        return Ok(BuildResults {
            state,
            results: Vec::new(),
        });
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</resultlist>") else {
        return Err(malformed());
    };
    // Nothing but whitespace may follow the closing tag.
    if !inner[close + "</resultlist>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut results = Vec::new();
    for element in xml_elements(inner, "result")? {
        let (tag, content) = xml_split_element(element, "result");
        let required = |name: &str| {
            xml_attr(tag, name).ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    format!("unexpected OBS build results response: <result> without {name}"),
                )
            })
        };
        let project = required("project")?;
        let repository = required("repository")?;
        let arch = required("arch")?;
        let code = required("code")?;
        let state = xml_attr(tag, "state").unwrap_or_else(|| code.clone());
        let dirty = xml_attr(tag, "dirty").as_deref() == Some("true");
        let details = xml_attr(tag, "details");
        let mut statuses = Vec::new();
        for status_element in xml_elements(content, "status")? {
            let (status_tag, status_content) = xml_split_element(status_element, "status");
            let package = xml_attr(status_tag, "package").ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    "unexpected OBS build results response: <status> without package",
                )
            })?;
            let code = xml_attr(status_tag, "code").ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    "unexpected OBS build results response: <status> without code",
                )
            })?;
            let details = xml_element_text(status_content, "details");
            statuses.push(PackageBuildStatus {
                package,
                code,
                details,
            });
        }
        results.push(BuildResult {
            project,
            repository,
            arch,
            code,
            state,
            dirty,
            details,
            statuses,
        });
    }
    Ok(BuildResults { state, results })
}

/// Options for [`ObsClient::build_log`]: which slice of the log to
/// read. The default reads the current log from the start.
#[derive(Debug, Default, Clone)]
pub struct BuildLogQuery {
    /// Start reading at this byte offset; a negative offset reads
    /// from the end of the log.
    pub start: Option<i64>,
    /// The last build's log instead of the current one. This skips
    /// the live-worker handoff and reads the local logfile, so on a
    /// currently-building package with no prior local log it reads
    /// as no log (None) rather than the live log.
    pub last: bool,
    /// The last succeeded build's log.
    pub lastsucceeded: bool,
}

/// A running build job, from `GET .../_jobstatus`. `code` is the
/// only field guaranteed present: a jobstatus element with no `code`
/// means no job is running (see [`ObsClient::job_status`]). Only
/// `code`, `result`, and `details` are attributes; the rest are
/// child elements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobStatus {
    /// The job's state (`building`, `scheduled`, ...).
    pub code: String,
    /// The job's result (`succeeded`, `failed`, `unchanged`), if the
    /// server reported one.
    pub result: Option<String>,
    /// The server's detail text for the job, if any.
    pub details: Option<String>,
    /// When the job started (unix time).
    pub starttime: Option<u64>,
    /// When the job ended (unix time), if it ended.
    pub endtime: Option<u64>,
    /// How long the previous build took (seconds).
    pub lastduration: Option<u64>,
    /// The worker id running the job.
    pub workerid: Option<String>,
    /// The architecture of the machine running the job.
    pub hostarch: Option<String>,
    /// The worker URI the job runs on.
    pub uri: Option<String>,
    /// The architecture the job builds for.
    pub arch: Option<String>,
    /// The job name.
    pub job: Option<String>,
    /// The job id (md5 of the job info file).
    pub jobid: Option<String>,
    /// The number of attempts to build the job.
    pub attempt: Option<u64>,
}

/// Parse a `<jobstatus ...>...</jobstatus>` document. An element
/// with no `code` means no job is running (None); a document with no
/// jobstatus element at all — or a non-numeric numeric child — is a
/// server error, never "no job". Only `code`, `result`, and `details`
/// are attributes; the rest (`starttime`, `endtime`, `lastduration`,
/// `workerid`, `hostarch`, `uri`, `arch`, `job`, `jobid`, `attempt`)
/// are child elements.
pub(crate) fn parse_job_status(body: &str) -> Result<Option<JobStatus>, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS job status response: not a <jobstatus> document",
        )
    };
    let body = body.trim();
    // The document must start with the root element: a `<jobstatus>`
    // found later in the body is not a jobstatus document.
    let Some(after_tag) = body.strip_prefix("<jobstatus") else {
        return Err(malformed());
    };
    let Some(first) = after_tag.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_tag) else {
        return Err(malformed());
    };
    let tag = &after_tag[..end];
    // The child elements, if the element is not self-closing.
    // Nothing but whitespace may follow the root element either way.
    let inner = if tag.trim_end().ends_with('/') {
        if !after_tag[end + 1..].trim().is_empty() {
            return Err(malformed());
        }
        ""
    } else {
        let after = &after_tag[end + 1..];
        let Some(close) = after.find("</jobstatus>") else {
            return Err(malformed());
        };
        if !after[close + "</jobstatus>".len()..].trim().is_empty() {
            return Err(malformed());
        }
        &after[..close]
    };
    let Some(code) = xml_attr(tag, "code") else {
        // An empty `<jobstatus/>`: no job is running.
        return Ok(None);
    };
    if code.is_empty() {
        return Ok(None);
    }
    let number = |name: &str| -> Result<Option<u64>, GeectlError> {
        match xml_element_text(inner, name) {
            None => Ok(None),
            Some(value) => value.parse::<u64>().map(Some).map_err(|_| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    format!(
                        "unexpected OBS job status response: <jobstatus> <{name}> is not a number"
                    ),
                )
            }),
        }
    };
    Ok(Some(JobStatus {
        code,
        result: xml_attr(tag, "result"),
        details: xml_attr(tag, "details"),
        starttime: number("starttime")?,
        endtime: number("endtime")?,
        lastduration: number("lastduration")?,
        workerid: xml_element_text(inner, "workerid"),
        hostarch: xml_element_text(inner, "hostarch"),
        uri: xml_element_text(inner, "uri"),
        arch: xml_element_text(inner, "arch"),
        job: xml_element_text(inner, "job"),
        jobid: xml_element_text(inner, "jobid"),
        attempt: number("attempt")?,
    }))
}

/// Parse a `<status package code/>` document (the `.../_status`
/// response) into the status code. A document without a `code`
/// attribute is a server error, never "no status": no status reads
/// as `code="unknown"` on the call, not as an empty document.
pub(crate) fn parse_build_status(body: &str) -> Result<String, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS build status response: not a <status> document",
        )
    };
    let body = body.trim();
    // The document must start with the root element: a `<status>`
    // found later in the body is not a status document, and nothing
    // but whitespace may follow the root element.
    let Some(after_tag) = body.strip_prefix("<status") else {
        return Err(malformed());
    };
    let Some(first) = after_tag.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_tag) else {
        return Err(malformed());
    };
    let tag = &after_tag[..end];
    if tag.trim_end().ends_with('/') {
        if !after_tag[end + 1..].trim().is_empty() {
            return Err(malformed());
        }
    } else {
        let after = &after_tag[end + 1..];
        let Some(close) = after.find("</status>") else {
            return Err(malformed());
        };
        if !after[close + "</status>".len()..].trim().is_empty() {
            return Err(malformed());
        }
    }
    match xml_attr(tag, "code") {
        Some(code) if !code.is_empty() => Ok(code),
        _ => Err(malformed()),
    }
}

/// The buildinfo for a package in a repository/architecture: the
/// error that blocks the build, if any, and the dependencies the
/// build pulls in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInfo {
    /// The error blocking the build (e.g. `unresolvable:
    /// DOES-NOT-EXIST`), if the build cannot start.
    pub error: Option<String>,
    /// The names of the build dependencies, in build order.
    pub bdeps: Vec<String>,
}

/// One entry of a package's build history: a completed build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildHistoryEntry {
    /// The source revision the build ran on.
    pub rev: String,
    /// The source md5 the build ran on.
    pub srcmd5: String,
    /// The version-release the build produced.
    pub versrel: String,
    /// The build count for that version-release.
    pub bcnt: u32,
    /// When the build ran (unix time).
    pub time: u64,
    /// How long the build took (seconds), if recorded.
    pub duration: Option<u64>,
}

/// One entry of a repository/architecture's job history: a build job
/// that ran, succeeded or failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobHistoryEntry {
    /// The repository the job ran in.
    pub repository: Option<String>,
    /// The architecture the job ran on.
    pub arch: Option<String>,
    /// The package the job built.
    pub package: String,
    /// The source revision the job ran on.
    pub rev: Option<String>,
    /// The source md5 the job ran on.
    pub srcmd5: Option<String>,
    /// The version-release the job produced.
    pub versrel: Option<String>,
    /// The build count for that version-release.
    pub bcnt: Option<u32>,
    /// When the job became ready (unix time).
    pub readytime: Option<u64>,
    /// When the job started (unix time).
    pub starttime: Option<u64>,
    /// When the job ended (unix time).
    pub endtime: Option<u64>,
    /// The job's result (`succeeded`, `failed`, ...).
    pub code: String,
    /// The worker URI the job ran on.
    pub uri: Option<String>,
    /// The worker id the job ran on.
    pub workerid: Option<String>,
    /// The architecture of the machine that ran the job.
    pub hostarch: Option<String>,
    /// The reason the job ran.
    pub reason: Option<String>,
    /// The md5 the job verified against.
    pub verifymd5: Option<String>,
}

/// Parse a `<buildinfo ...>...</buildinfo>` document into the error
/// blocking the build, if any, and the build dependency names. A
/// document that is not a buildinfo — or a `<bdep>` without a name —
/// is a server error, never an empty buildinfo.
pub(crate) fn parse_buildinfo(body: &str) -> Result<BuildInfo, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS buildinfo response: not a <buildinfo> document",
        )
    };
    let body = body.trim();
    let Some(after_open) = body.strip_prefix("<buildinfo") else {
        return Err(malformed());
    };
    // The tag name must end here: `<buildinfox>` is not a buildinfo.
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<buildinfo .../>`: no error, no dependencies.
        return Ok(BuildInfo {
            error: None,
            bdeps: Vec::new(),
        });
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</buildinfo>") else {
        return Err(malformed());
    };
    // Nothing but whitespace may follow the closing tag.
    if !inner[close + "</buildinfo>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let error = xml_element_text(inner, "error");
    let mut bdeps = Vec::new();
    for element in xml_elements(inner, "bdep")? {
        let (tag, _) = xml_split_element(element, "bdep");
        let name = xml_attr(tag, "name").ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS buildinfo response: <bdep> without name",
            )
        })?;
        bdeps.push(name);
    }
    Ok(BuildInfo { error, bdeps })
}

/// Parse a `<buildhistory>...</buildhistory>` document into the
/// completed builds. A document that is not a buildhistory — or an
/// `<entry>` missing a required attribute or carrying a non-numeric
/// one — is a server error, never an empty history.
pub(crate) fn parse_build_history(body: &str) -> Result<Vec<BuildHistoryEntry>, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS build history response: not a <buildhistory> document",
        )
    };
    let body = body.trim();
    let Some(after_open) = body.strip_prefix("<buildhistory") else {
        return Err(malformed());
    };
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<buildhistory/>`: never built.
        return Ok(Vec::new());
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</buildhistory>") else {
        return Err(malformed());
    };
    if !inner[close + "</buildhistory>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut entries = Vec::new();
    for element in xml_elements(inner, "entry")? {
        let (tag, _) = xml_split_element(element, "entry");
        let required = |name: &str| {
            xml_attr(tag, name).ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    format!("unexpected OBS build history response: <entry> without {name}"),
                )
            })
        };
        entries.push(BuildHistoryEntry {
            rev: required("rev")?,
            srcmd5: required("srcmd5")?,
            versrel: required("versrel")?,
            bcnt: xml_attr_number_required(tag, "bcnt")?,
            time: xml_attr_number_required(tag, "time")?,
            duration: xml_attr_number(tag, "duration")?,
        });
    }
    // The wire order is oldest-first (the backend reads the
    // append-only history file backwards, then reverses it), so the
    // newest build is first.
    entries.reverse();
    Ok(entries)
}

/// Parse a `<jobhistlist>...</jobhistlist>` document into the build
/// jobs that ran. A document that is not a jobhistlist — or a
/// `<jobhist>` missing its package or code, or carrying a non-numeric
/// number — is a server error, never an empty history.
pub(crate) fn parse_job_history(body: &str) -> Result<Vec<JobHistoryEntry>, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS job history response: not a <jobhistlist> document",
        )
    };
    let body = body.trim();
    let Some(after_open) = body.strip_prefix("<jobhistlist") else {
        return Err(malformed());
    };
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<jobhistlist/>`: no jobs ran.
        return Ok(Vec::new());
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</jobhistlist>") else {
        return Err(malformed());
    };
    if !inner[close + "</jobhistlist>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut entries = Vec::new();
    for element in xml_elements(inner, "jobhist")? {
        let (tag, _) = xml_split_element(element, "jobhist");
        let package = xml_attr(tag, "package").ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS job history response: <jobhist> without package",
            )
        })?;
        let code = xml_attr(tag, "code").ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS job history response: <jobhist> without code",
            )
        })?;
        entries.push(JobHistoryEntry {
            repository: xml_attr(tag, "repository"),
            arch: xml_attr(tag, "arch"),
            package,
            rev: xml_attr(tag, "rev"),
            srcmd5: xml_attr(tag, "srcmd5"),
            versrel: xml_attr(tag, "versrel"),
            bcnt: xml_attr_number(tag, "bcnt")?,
            readytime: xml_attr_number(tag, "readytime")?,
            starttime: xml_attr_number(tag, "starttime")?,
            endtime: xml_attr_number(tag, "endtime")?,
            code,
            uri: xml_attr(tag, "uri"),
            workerid: xml_attr(tag, "workerid"),
            hostarch: xml_attr(tag, "hostarch"),
            reason: xml_attr(tag, "reason"),
            verifymd5: xml_attr(tag, "verifymd5"),
        });
    }
    // The wire order is oldest-first, like the build history, so the
    // newest job is first.
    entries.reverse();
    Ok(entries)
}

/// One built file in a build result directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryFile {
    /// The file name (e.g. `foo-1.0-1.x86_64.rpm`, `_buildenv`).
    pub name: String,
    /// The file size in bytes, if the server reported it.
    pub size: Option<u64>,
    /// The file mtime (unix time), if the server reported it.
    pub mtime: Option<u64>,
}

/// The metadata of a built file: the RPM header fields (name,
/// epoch, version, release, arch, source, summary, description,
/// disturl, size, mtime) plus the dependency lists (plain view) and
/// the resolved dependencies (`fileinfo_ext` view).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    /// The file name.
    pub filename: String,
    /// The RPM name.
    pub name: Option<String>,
    /// The RPM epoch.
    pub epoch: Option<u64>,
    /// The RPM version.
    pub version: Option<String>,
    /// The RPM release.
    pub release: Option<String>,
    /// The RPM architecture.
    pub arch: Option<String>,
    /// The source package, for a source RPM.
    pub source: Option<String>,
    /// The RPM summary.
    pub summary: Option<String>,
    /// The RPM description.
    pub description: Option<String>,
    /// The dist URL identifying where the file was built.
    pub disturl: Option<String>,
    /// The file size in bytes.
    pub size: Option<u64>,
    /// The file mtime (unix time).
    pub mtime: Option<u64>,
    /// The dependencies the file provides (plain view).
    pub provides: Vec<String>,
    /// The dependencies the file requires (plain view).
    pub requires: Vec<String>,
    /// The pre-requires (plain view).
    pub prerequires: Vec<String>,
    /// The conflicts (plain view).
    pub conflicts: Vec<String>,
    /// The obsoletes (plain view).
    pub obsoletes: Vec<String>,
    /// The recommends (plain view).
    pub recommends: Vec<String>,
    /// The supplements (plain view).
    pub supplements: Vec<String>,
    /// The suggests (plain view).
    pub suggests: Vec<String>,
    /// The enhances (plain view).
    pub enhances: Vec<String>,
    /// The dependencies the file provides, with the packages that
    /// require each one (`fileinfo_ext` view only).
    pub provides_ext: Vec<ExtDependency>,
    /// The dependencies the file requires, with the package providing
    /// each one (`fileinfo_ext` view only).
    pub requires_ext: Vec<ExtDependency>,
    /// The dependencies the file recommends, with the package
    /// providing each one (`fileinfo_ext` view only).
    pub recommends_ext: Vec<ExtDependency>,
    /// The dependencies the file supplements, with the package
    /// providing each one (`fileinfo_ext` view only).
    pub supplements_ext: Vec<ExtDependency>,
}

/// One dependency in a `fileinfo_ext` view, with its resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtDependency {
    /// The dependency (e.g. `libc.so.6()(64bit)`).
    pub dep: String,
    /// For a required dependency: the packages providing it, if
    /// any were found in the repository.
    pub provided_by: Vec<ProvidedBy>,
    /// For a provided dependency: the packages requiring it.
    pub required_by: Vec<ProvidedBy>,
}

/// A package named by a `fileinfo_ext` resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvidedBy {
    /// The package name.
    pub name: String,
    /// The package epoch.
    pub epoch: Option<u64>,
    /// The package version.
    pub version: Option<String>,
    /// The package release.
    pub release: Option<String>,
    /// The package architecture.
    pub arch: Option<String>,
    /// The project the package comes from.
    pub project: Option<String>,
    /// The repository the package comes from.
    pub repository: Option<String>,
}

/// Parse a `<binarylist>...</binarylist>` document into the built
/// files. A document that is not a binarylist — or a `<binary>`
/// without a filename, or with a non-numeric size/mtime — is a server
/// error, never an empty list.
pub(crate) fn parse_binary_list(body: &str) -> Result<Vec<BinaryFile>, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS binary list response: not a <binarylist> document",
        )
    };
    let body = body.trim();
    let Some(after_open) = body.strip_prefix("<binarylist") else {
        return Err(malformed());
    };
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<binarylist/>`: no binaries built.
        return Ok(Vec::new());
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</binarylist>") else {
        return Err(malformed());
    };
    if !inner[close + "</binarylist>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut files = Vec::new();
    for element in xml_elements(inner, "binary")? {
        let (tag, _) = xml_split_element(element, "binary");
        let name = xml_attr(tag, "filename").ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS binary list response: <binary> without filename",
            )
        })?;
        files.push(BinaryFile {
            name,
            size: xml_attr_number(tag, "size")?,
            mtime: xml_attr_number(tag, "mtime")?,
        });
    }
    Ok(files)
}

/// Parse one `<provided_by>` / `<requiredby>` resolution element
/// (both carry the same attributes) into the package it names.
fn parse_provided_by(tag: &str) -> Result<ProvidedBy, GeectlError> {
    let name = xml_attr(tag, "name").ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS fileinfo response: a dependency resolution without name",
        )
    })?;
    Ok(ProvidedBy {
        name,
        epoch: xml_attr_number(tag, "epoch")?,
        version: xml_attr(tag, "version"),
        release: xml_attr(tag, "release"),
        arch: xml_attr(tag, "arch"),
        project: xml_attr(tag, "project"),
        repository: xml_attr(tag, "repository"),
    })
}

/// Parse one `<*_ext>` dependency element into its dependency
/// and its resolution: the packages requiring a provided dependency
/// (`<requiredby>` children) and the packages providing a required,
/// recommended, or supplemented dependency (`<provided_by>` under
/// both spellings). An element without a `dep` is a server error.
fn parse_ext_dependency(element: &str, name: &str) -> Result<ExtDependency, GeectlError> {
    let (tag, content) = xml_split_element(element, name);
    let dep = xml_attr(tag, "dep").ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected OBS fileinfo response: <{name}> without dep"),
        )
    })?;
    let mut required_by = Vec::new();
    for child in xml_elements(content, "requiredby")? {
        let (child_tag, _) = xml_split_element(child, "requiredby");
        required_by.push(parse_provided_by(child_tag)?);
    }
    let mut provided_by = Vec::new();
    for spelling in ["provided_by", "providedby"] {
        for child in xml_elements(content, spelling)? {
            let (child_tag, _) = xml_split_element(child, spelling);
            provided_by.push(parse_provided_by(child_tag)?);
        }
    }
    Ok(ExtDependency {
        dep,
        provided_by,
        required_by,
    })
}

/// Parse a `<fileinfo ...>...</fileinfo>` document into the file's
/// metadata. The plain view carries the dependency lists
/// (`provides`, `requires`, `prerequires`, `conflicts`,
/// `obsoletes`, `recommends`, `supplements`, `suggests`,
/// `enhances`) as repeated text elements; the `fileinfo_ext` view
/// carries `provides_ext` / `requires_ext` / `recommends_ext` /
/// `supplements_ext` with one element per dependency. A document
/// that is not a fileinfo — or a fileinfo without a filename, an ext
/// dependency without a `dep`, or a non-numeric size/mtime — is a
/// server error, never an empty fileinfo. The `provided_by` element
/// is read under both spellings: the backend writes `<providedby>`,
/// the API docs name it `<provided_by>`. `filelist` is only present
/// with `withfilelist`, which geectl never requests, so it is not
/// read.
pub(crate) fn parse_fileinfo(body: &str) -> Result<FileInfo, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS fileinfo response: not a <fileinfo> document",
        )
    };
    let body = body.trim();
    let Some(after_open) = body.strip_prefix("<fileinfo") else {
        return Err(malformed());
    };
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    let filename = xml_attr(open_tag, "filename").ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS fileinfo response: <fileinfo> without filename",
        )
    })?;
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<fileinfo .../>`: a file with no metadata.
        return Ok(FileInfo {
            filename,
            name: None,
            epoch: None,
            version: None,
            release: None,
            arch: None,
            source: None,
            summary: None,
            description: None,
            disturl: None,
            size: None,
            mtime: None,
            provides: Vec::new(),
            requires: Vec::new(),
            prerequires: Vec::new(),
            conflicts: Vec::new(),
            obsoletes: Vec::new(),
            recommends: Vec::new(),
            supplements: Vec::new(),
            suggests: Vec::new(),
            enhances: Vec::new(),
            provides_ext: Vec::new(),
            requires_ext: Vec::new(),
            recommends_ext: Vec::new(),
            supplements_ext: Vec::new(),
        });
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</fileinfo>") else {
        return Err(malformed());
    };
    if !inner[close + "</fileinfo>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let number_text = |name: &str| -> Result<Option<u64>, GeectlError> {
        match xml_element_text(inner, name) {
            None => Ok(None),
            Some(value) => value.parse::<u64>().map(Some).map_err(|_| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    format!("unexpected OBS fileinfo response: <{name}> is not a number"),
                )
            }),
        }
    };
    let list = |name: &str| -> Result<Vec<String>, GeectlError> {
        Ok(xml_elements(inner, name)?
            .iter()
            .map(|element| {
                let (_, text) = xml_split_element(element, name);
                xml_unescape(text.trim())
            })
            .filter(|text| !text.is_empty())
            .collect())
    };
    let mut provides_ext = Vec::new();
    for element in xml_elements(inner, "provides_ext")? {
        provides_ext.push(parse_ext_dependency(element, "provides_ext")?);
    }
    let mut requires_ext = Vec::new();
    for element in xml_elements(inner, "requires_ext")? {
        requires_ext.push(parse_ext_dependency(element, "requires_ext")?);
    }
    let mut recommends_ext = Vec::new();
    for element in xml_elements(inner, "recommends_ext")? {
        recommends_ext.push(parse_ext_dependency(element, "recommends_ext")?);
    }
    let mut supplements_ext = Vec::new();
    for element in xml_elements(inner, "supplements_ext")? {
        supplements_ext.push(parse_ext_dependency(element, "supplements_ext")?);
    }
    Ok(FileInfo {
        filename,
        name: xml_element_text(inner, "name"),
        epoch: number_text("epoch")?,
        version: xml_element_text(inner, "version"),
        release: xml_element_text(inner, "release"),
        arch: xml_element_text(inner, "arch"),
        source: xml_element_text(inner, "source"),
        summary: xml_element_text(inner, "summary"),
        description: xml_element_text(inner, "description"),
        disturl: xml_element_text(inner, "disturl"),
        size: number_text("size")?,
        mtime: number_text("mtime")?,
        provides: list("provides")?,
        requires: list("requires")?,
        prerequires: list("prerequires")?,
        conflicts: list("conflicts")?,
        obsoletes: list("obsoletes")?,
        recommends: list("recommends")?,
        supplements: list("supplements")?,
        suggests: list("suggests")?,
        enhances: list("enhances")?,
        provides_ext,
        requires_ext,
        recommends_ext,
        supplements_ext,
    })
}

/// One package's build dependencies, from `_builddepinfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageDeps {
    /// The package.
    pub name: String,
    /// The source package, if the server reported one.
    pub source: Option<String>,
    /// The packages it depends on for the build (whole-package
    /// dependencies, not individual binaries). In the reverse
    /// (`revpkgnames`) view returned by [`ObsClient::whatdependson`],
    /// these are instead the packages depending on the queried
    /// package.
    pub dependencies: Vec<String>,
    /// The subpackages it produces.
    pub subpackages: Vec<String>,
}

/// Parse a `<builddepinfo>...</builddepinfo>` document into the
/// per-package build dependencies. Only the direct `<package>`
/// children count: the document also carries `<cycle>` and `<scc>`
/// elements whose `<package>` children are cycle members, not
/// dependencies. A document that is not a builddepinfo — or a
/// `<package>` without a name — is a server error, never an empty
/// list.
pub(crate) fn parse_builddepinfo(body: &str) -> Result<Vec<PackageDeps>, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS builddepinfo response: not a <builddepinfo> document",
        )
    };
    let body = body.trim();
    let Some(after_open) = body.strip_prefix("<builddepinfo") else {
        return Err(malformed());
    };
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<builddepinfo/>`: no packages, no deps.
        return Ok(Vec::new());
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</builddepinfo>") else {
        return Err(malformed());
    };
    if !inner[close + "</builddepinfo>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut packages = Vec::new();
    for element in xml_child_elements(inner, "package")? {
        let (tag, content) = xml_split_element(element, "package");
        let name = xml_attr(tag, "name").ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS builddepinfo response: <package> without name",
            )
        })?;
        let dependencies = xml_elements(content, "pkgdep")?
            .iter()
            .map(|dep| {
                let (_, text) = xml_split_element(dep, "pkgdep");
                xml_unescape(text.trim())
            })
            .filter(|text| !text.is_empty())
            .collect();
        let subpackages = xml_elements(content, "subpkg")?
            .iter()
            .map(|sub| {
                let (_, text) = xml_split_element(sub, "subpkg");
                xml_unescape(text.trim())
            })
            .filter(|text| !text.is_empty())
            .collect();
        packages.push(PackageDeps {
            name,
            source: xml_element_text(content, "source"),
            dependencies,
            subpackages,
        });
    }
    Ok(packages)
}

/// Parse a `<project ...>...</project>` document (`_meta`) into the
/// project's repositories. A document that is not a project — or a
/// `<repository>` without a name, or a `<path>` without a project —
/// is a server error, never an empty list.
pub(crate) fn parse_repositories(body: &str) -> Result<Vec<Repository>, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS project meta response: not a <project> document",
        )
    };
    let body = body.trim();
    let Some(after_open) = body.strip_prefix("<project") else {
        return Err(malformed());
    };
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<project .../>`: no repositories.
        return Ok(Vec::new());
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</project>") else {
        return Err(malformed());
    };
    if !inner[close + "</project>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut repositories = Vec::new();
    for element in xml_child_elements(inner, "repository")? {
        let (tag, content) = xml_split_element(element, "repository");
        let name = xml_attr(tag, "name").ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS project meta response: <repository> without name",
            )
        })?;
        let arch = xml_elements(content, "arch")?
            .iter()
            .map(|arch| {
                let (_, text) = xml_split_element(arch, "arch");
                xml_unescape(text.trim())
            })
            .filter(|text| !text.is_empty())
            .collect();
        let mut paths = Vec::new();
        for path in xml_elements(content, "path")? {
            let (path_tag, _) = xml_split_element(path, "path");
            let project = xml_attr(path_tag, "project").ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    "unexpected OBS project meta response: <path> without project",
                )
            })?;
            // A path without a repository attribute names no
            // (project, repository) pair, so it is dropped - the
            // same rule project_meta applies to its paths.
            if let Some(repository) = xml_attr(path_tag, "repository") {
                paths.push((project, repository));
            }
        }
        repositories.push(Repository { name, arch, paths });
    }
    Ok(repositories)
}

/// Parse a `<directory>...</directory>` document into the `name`
/// attributes of its `<entry>` children. A document that is not a
/// directory is a server error, never an empty list: an empty
/// directory is the "nothing matches" answer.
pub(crate) fn parse_directory_entry_names(body: &str) -> Result<Vec<String>, GeectlError> {
    let body = body.trim();
    // A malformed response still fails closed, but the error carries a
    // bounded preview of what was received, so the failure is
    // diagnosable instead of a bare "not a directory document". The
    // preview is the debug channel's redacting preview: credential
    // values are masked, binary bodies are never dumped, and the text
    // is capped at DEBUG_BODY_CAP bytes.
    let malformed = || {
        let preview = crate::debug::format_body_preview(body);
        let received = if preview.is_empty() {
            "<empty response>".to_string()
        } else {
            preview
        };
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected OBS response: not a <directory> document; received: {received}"),
        )
    };
    // The server may emit an XML prolog (`<?xml version=\"1.0\"?>`)
    // before the directory element; skip it. The prolog must
    // terminate (`?>`) — an unterminated prolog is malformed, not a
    // missing directory.
    let body = if let Some(after) = body.strip_prefix("<?xml") {
        let first = after.chars().next();
        if !matches!(first, Some(c) if c.is_whitespace() || c == '?') {
            return Err(malformed());
        }
        let Some(end) = after.find("?>") else {
            return Err(malformed());
        };
        after[end + 2..].trim_start()
    } else {
        body
    };
    let Some(after_open) = body.strip_prefix("<directory") else {
        return Err(malformed());
    };
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<directory/>`: no entries.
        return Ok(Vec::new());
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</directory>") else {
        return Err(malformed());
    };
    if !inner[close + "</directory>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut names = Vec::new();
    for element in xml_elements(inner, "entry")? {
        let (tag, _) = xml_split_element(element, "entry");
        if let Some(name) = xml_attr(tag, "name") {
            names.push(name);
        }
    }
    Ok(names)
}

/// What OBS does with the source branch when a submit request is
/// accepted. The values are exactly the server's
/// `BsRequestAction::VALID_SOURCEUPDATE_OPTIONS`; anything else is
/// refused by the server at creation, so this enum is the only way to
/// name one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceUpdate {
    /// Delete the source package on accept (and the source project
    /// with it, when it was its last package and it is not a
    /// top-level `home:<user>` project). This is the default: the
    /// branch dies when its request lands, like a GitHub head branch
    /// on merge, so accepted branches never pile up as leftovers.
    #[default]
    Cleanup,
    /// Keep the source package, updated to the accepted state. This
    /// is what [`SrPayload::without_cleanup`] selects, and what
    /// `osc sr --no-cleanup` sends.
    Update,
    /// Leave the source package untouched on accept.
    NoUpdate,
}

impl SourceUpdate {
    /// The server's spelling of this value, as it appears in
    /// `<sourceupdate>`.
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceUpdate::Cleanup => "cleanup",
            SourceUpdate::Update => "update",
            SourceUpdate::NoUpdate => "noupdate",
        }
    }
}

/// The validated submit-request payload: where the change comes from,
/// where it goes, what it says, and the one `.changes` entry it carries.
///
/// The `.changes` entry is not part of the OBS request XML (it lives
/// in the package's `.changes` file, in the branch); it is carried
/// here so the filing step can log the complete submission and the
/// caller can validate the changelog rules before anything is filed.
///
/// The description follows the guide's SR message format
/// (`references/request-text.md` "SR and PR messages"): one headline
/// of at most 160 characters of prose (`Update <pkg> to <ver>: <what
/// changes>`), then `- ` lines for staging, needs, and rebuilds. The
/// library validates the description is non-empty; the headline
/// shape is the caller's responsibility.
///
/// Branch cleanup is the default: the payload carries
/// `opt_sourceupdate="cleanup"`, so OBS deletes the source branch on
/// accept. A caller that needs the branch to survive — a forward from
/// a devel project, where the source is a package that must keep
/// existing — opts out with [`SrPayload::without_cleanup`].
///
/// The fields are private: a payload can only be built through
/// [`SrPayload::new`], so a struct literal cannot bypass the
/// validation below. Read them through the accessors; [`SrFiling`]
/// re-validates the semantics on the filing path as well.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrPayload {
    /// The project the change comes from (the branch, e.g.
    /// `home:scarabeus_iv`).
    source_project: String,
    /// The package in the source project.
    source_package: String,
    /// The project the SR targets (the devel project). Never
    /// `openSUSE:Factory` — a submit request targets the devel
    /// project; Factory comes via a forward.
    target_project: String,
    /// The package in the target project.
    target_package: String,
    /// The SR description text (the human-readable summary).
    description: String,
    /// The one `.changes` entry for this submission.
    changes_entry: String,
    /// The CVEs this submission claims to fix. The `.changes` entry
    /// must name each of them (see [`changelog_names_cve`]).
    cves: Vec<CveId>,
    /// What OBS does with the source branch when this request is
    /// accepted. The default is [`SourceUpdate::Cleanup`]: the branch
    /// dies on accept, so accepted branches never pile up as
    /// leftovers. See [`SrPayload::without_cleanup`] for the opt-out.
    source_update: SourceUpdate,
}

impl SrPayload {
    /// Build and validate a payload. Every name must be a valid OBS
    /// project/package name, the target must not be
    /// `openSUSE:Factory`, the description and `.changes` entry must
    /// be non-empty, and the `.changes` entry must name every CVE in
    /// `cves`. The payload carries `opt_sourceupdate="cleanup"` by
    /// default, so OBS deletes the source branch on accept; use
    /// [`SrPayload::without_cleanup`] when the branch must survive.
    /// Returns Err instead of building a payload that could not be
    /// filed.
    pub fn new(
        source_project: &str,
        source_package: &str,
        target_project: &str,
        target_package: &str,
        description: &str,
        changes_entry: &str,
        cves: &[CveId],
    ) -> Result<Self, GeectlError> {
        let payload = SrPayload {
            source_project: source_project.to_string(),
            source_package: source_package.to_string(),
            target_project: target_project.to_string(),
            target_package: target_package.to_string(),
            description: description.to_string(),
            changes_entry: changes_entry.to_string(),
            cves: cves.to_vec(),
            source_update: SourceUpdate::default(),
        };
        payload.validate_semantics()?;
        // The payload must render to well-formed XML, or it is not a
        // payload at all — check now, not at filing time.
        payload.validate_xml()?;
        Ok(payload)
    }

    /// The semantic checks [`SrPayload::new`] runs: every name must
    /// be a valid OBS project/package name, the target must not be
    /// `openSUSE:Factory`, the description and `.changes` entry must
    /// be non-empty, and the `.changes` entry must name every CVE in
    /// `cves`. [`SrFiling::file`] runs the same checks again on the
    /// filing path, so a payload that reaches filing is validated
    /// there too, not only at construction.
    pub fn validate_semantics(&self) -> Result<(), GeectlError> {
        for (label, value) in [
            ("source project", self.source_project.as_str()),
            ("target project", self.target_project.as_str()),
        ] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS project name for {label}: {value:?}"),
                ));
            }
        }
        for (label, value) in [
            ("source package", self.source_package.as_str()),
            ("target package", self.target_package.as_str()),
        ] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS package name for {label}: {value:?}"),
                ));
            }
        }
        // A submit request targets the devel project, never Factory
        // directly: the Factory step is a forward after the devel
        // request lands.
        if self.target_project == FACTORY_PROJECT {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "submit requests target the devel project, never openSUSE:Factory directly",
            ));
        }
        if self.description.trim().is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "SR description must not be empty",
            ));
        }
        if self.changes_entry.trim().is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "SR .changes entry must not be empty",
            ));
        }
        for cve in &self.cves {
            if !changelog_names_cve(&self.changes_entry, cve) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "the .changes entry does not name {cve}, which this submission claims to fix"
                    ),
                ));
            }
        }
        Ok(())
    }

    /// The project the change comes from (the branch).
    pub fn source_project(&self) -> &str {
        &self.source_project
    }

    /// The package in the source project.
    pub fn source_package(&self) -> &str {
        &self.source_package
    }

    /// The project the SR targets (the devel project).
    pub fn target_project(&self) -> &str {
        &self.target_project
    }

    /// The package in the target project.
    pub fn target_package(&self) -> &str {
        &self.target_package
    }

    /// The SR description text (the human-readable summary).
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The one `.changes` entry for this submission.
    pub fn changes_entry(&self) -> &str {
        &self.changes_entry
    }

    /// The CVEs this submission claims to fix.
    pub fn cves(&self) -> &[CveId] {
        &self.cves
    }

    /// What OBS does with the source branch when this request is
    /// accepted.
    pub fn source_update(&self) -> SourceUpdate {
        self.source_update
    }

    /// Opt out of the default branch cleanup: the source branch is
    /// kept (updated) instead of deleted on accept. Use this only
    /// when the branch must survive the request — the case the
    /// packaging guide names is a forward from a devel project (a
    /// Factory forward), where the source is a package that must keep
    /// existing. From a `home:` subproject the default stays:
    /// pr-guard refuses the SR without cleanup, and without it
    /// accepted branches pile up as leftovers. This selects the
    /// `update` value (what `osc sr --no-cleanup` sends), not
    /// `noupdate`: the source is still updated to the accepted state,
    /// it is just not deleted.
    pub fn without_cleanup(mut self) -> Self {
        self.source_update = SourceUpdate::Update;
        self
    }

    /// Render the OBS request XML for this payload. All text is
    /// XML-escaped, so a description containing `&` or `<` cannot break
    /// out of its element. The action carries
    /// `<options><sourceupdate>…</sourceupdate></options>` with the
    /// payload's source-update value (`cleanup` by default).
    ///
    /// Crate-internal: the rendering is used by [`SrFiling`] (the
    /// dry-run note) and by the unit tests. Consumers never see XML;
    /// they file the typed payload through [`SrFiling::file`].
    pub(crate) fn to_xml(&self) -> String {
        format!(
            "<request>\n  <action type=\"submit\">\n    <source project=\"{}\" package=\"{}\" />\n    <target project=\"{}\" package=\"{}\" />\n    <options><sourceupdate>{}</sourceupdate></options>\n  </action>\n  <description>{}</description>\n</request>",
            xml_escape(&self.source_project),
            xml_escape(&self.source_package),
            xml_escape(&self.target_project),
            xml_escape(&self.target_package),
            self.source_update.as_str(),
            xml_escape(&self.description),
        )
    }

    /// Verify the rendered XML is well-formed: tags are balanced and
    /// properly nested, there is exactly one root element (`request`),
    /// and the required elements are present in the expected nesting
    /// (`request > action[type=submit] > source, target, options >
    /// sourceupdate` and `request > description`). The
    /// `<sourceupdate>` value is also checked against the values the
    /// server accepts. This is a structural check on the generated
    /// XML, not a re-derivation of the payload.
    pub fn validate_xml(&self) -> Result<(), GeectlError> {
        validate_sr_xml(&self.to_xml())
    }
}

/// Verify rendered SR XML: well-formed, with the required
/// structure, a `<sourceupdate>` value the server accepts, and an
/// `<action>` carrying `type="submit"`. The action-type check exists
/// so a future edit to `to_xml` that changes the action type fails
/// here, at payload build time, instead of at the API.
fn validate_sr_xml(xml: &str) -> Result<(), GeectlError> {
    check_well_formed(xml, SUBMIT_REQUIRED_PATHS)?;
    // The rendered XML must carry exactly one <sourceupdate> with
    // a value the server accepts. The value comes from the
    // `SourceUpdate` enum, so it is always one of the three —
    // this check exists so a future edit to `to_xml` that drops
    // or mangles the attribute fails here, at payload build time,
    // instead of at the API.
    match sourceupdate_value(xml) {
        Some(value) if ["update", "noupdate", "cleanup"].contains(&value) => {}
        Some(value) => {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("malformed SR XML: unknown <sourceupdate> value {value:?}"),
            ));
        }
        // A missing <sourceupdate> is refused by check_well_formed
        // above, so None here means the element is duplicated.
        None => {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "malformed SR XML: duplicated <sourceupdate> in <action>",
            ));
        }
    }
    if !request_block_matches_action(xml, "submit", ActionShape::Any) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "malformed SR XML: <action> is not type=\"submit\"",
        ));
    }
    Ok(())
}

/// The `<sourceupdate>` value the rendered XML carries, if exactly one
/// `<sourceupdate>…</sourceupdate>` element is present. Any other
/// shape (none, or more than one) reads as absent, so the caller
/// refuses it rather than guessing which one applies.
fn sourceupdate_value(xml: &str) -> Option<&str> {
    let open = "<sourceupdate>";
    let close = "</sourceupdate>";
    let start = xml.find(open)? + open.len();
    let rest = &xml[start..];
    let end = rest.find(close)?;
    let value = &rest[..end];
    // Exactly one element: a second opening or closing tag anywhere
    // means the document carries two values, not one.
    if xml.matches(open).count() != 1 || xml.matches(close).count() != 1 {
        return None;
    }
    Some(value)
}

/// The validated maintenance-incident payload: which packages from
/// which project go into which maintenance project, and what the
/// incident says.
///
/// One action per source package: a single request carries the whole
/// incident, so the packages land together. The target is the
/// maintenance project itself (`openSUSE:Maintenance`, or the
/// incident project for a follow-up); the target package is never
/// specified — the server forbids it (`IllegalRequest`); the incident
/// package is named at accept time, when the server merges the source
/// into the incident project. The optional release project is the maintained product
/// the update releases into; the server resolves it to its update
/// instance and requires `maintenance_release` kind.
///
/// No source-update by default: unlike a submit request, whose
/// branch is a throwaway by default, the incident source is the
/// maintenance working area, which the maintainer may reuse for the
/// next incident. Opt in with [`MaintenanceIncidentPayload::with_cleanup`]
/// when the branch must die on accept.
///
/// A project-level incident (no packages at all) is legal on the
/// server but out of scope here: the skill files incidents per
/// package, and the dedup check keys on the source package. The
/// fields are private: a payload can only be built through
/// [`MaintenanceIncidentPayload::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceIncidentPayload {
    /// The project the update comes from (the maintenance branch).
    source_project: String,
    /// The packages in the source project, one action each.
    source_packages: Vec<String>,
    /// The maintenance project the incident targets.
    target_project: String,
    /// The maintained product the update releases into, if any.
    target_releaseproject: Option<String>,
    /// What OBS does with the source branch when the incident is
    /// accepted. None renders no `<options>` at all.
    source_update: Option<SourceUpdate>,
    /// The incident description text.
    description: String,
}

impl MaintenanceIncidentPayload {
    /// Build and validate a payload. Every name must be a valid OBS
    /// project/package name, at least one source package is required,
    /// with no duplicates, and the description must be non-empty.
    /// Returns Err instead of
    /// building a payload that could not be filed.
    pub fn new(
        source_project: &str,
        source_packages: &[&str],
        target_project: &str,
        description: &str,
    ) -> Result<Self, GeectlError> {
        let payload = MaintenanceIncidentPayload {
            source_project: source_project.to_string(),
            source_packages: source_packages.iter().map(|p| p.to_string()).collect(),
            target_project: target_project.to_string(),
            target_releaseproject: None,
            source_update: None,
            description: description.to_string(),
        };
        payload.validate_semantics()?;
        payload.validate_xml()?;
        Ok(payload)
    }

    /// Set the release project the update releases into. The name
    /// must be a valid OBS project name; the server resolves it to
    /// its update instance at filing time, requires
    /// `maintenance_release` kind, and stores the resolved name. The
    /// dedup keys on the release project as filed, so a release
    /// project that resolves elsewhere may slip past a later dedup;
    /// a `maintenance_release` project resolves to itself, so the
    /// normal case is unaffected.
    pub fn with_release_project(mut self, release_project: &str) -> Result<Self, GeectlError> {
        if !valid_obs_name(release_project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name for release project: {release_project:?}"),
            ));
        }
        self.target_releaseproject = Some(release_project.to_string());
        self.validate_xml()?;
        Ok(self)
    }

    /// Opt in to branch cleanup: the source branch is deleted when
    /// the incident is accepted. Use this only when the branch is a
    /// throwaway; by default the branch survives the incident.
    pub fn with_cleanup(mut self) -> Result<Self, GeectlError> {
        self.source_update = Some(SourceUpdate::Cleanup);
        self.validate_xml()?;
        Ok(self)
    }

    /// The project the update comes from.
    pub fn source_project(&self) -> &str {
        &self.source_project
    }

    /// The packages in the source project, one action each.
    pub fn source_packages(&self) -> &[String] {
        &self.source_packages
    }

    /// The maintenance project the incident targets.
    pub fn target_project(&self) -> &str {
        &self.target_project
    }

    /// The maintained product the update releases into, if any.
    pub fn target_releaseproject(&self) -> Option<&str> {
        self.target_releaseproject.as_deref()
    }

    /// What OBS does with the source branch on accept, if any.
    pub fn source_update(&self) -> Option<SourceUpdate> {
        self.source_update
    }

    /// The incident description text.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The semantic checks [`MaintenanceIncidentPayload::new`] runs:
    /// every name must be a valid OBS project/package name, at least
    /// one source package is required, with no duplicates, and the
    /// description must be non-empty.
    /// [`SrFiling::file_maintenance_incident`] runs the
    /// same checks again on the filing path.
    pub fn validate_semantics(&self) -> Result<(), GeectlError> {
        if !valid_obs_name(&self.source_project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "invalid OBS project name for source project: {:?}",
                    self.source_project
                ),
            ));
        }
        if self.source_packages.is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "a maintenance incident needs at least one source package",
            ));
        }
        // One action per source package: a duplicate would render two
        // identical actions, which the server refuses
        // (`ConflictingActions` on the incident `uniq_key`).
        let mut seen: Vec<&String> = Vec::new();
        for package in &self.source_packages {
            if !valid_obs_name(package) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS package name for source package: {package:?}"),
                ));
            }
            if seen.contains(&package) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("a maintenance incident lists source package {package:?} twice"),
                ));
            }
            seen.push(package);
        }
        if !valid_obs_name(&self.target_project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "invalid OBS project name for target project: {:?}",
                    self.target_project
                ),
            ));
        }
        if let Some(release_project) = &self.target_releaseproject {
            if !valid_obs_name(release_project) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS project name for release project: {release_project:?}"),
                ));
            }
        }
        if self.description.trim().is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "maintenance incident description must not be empty",
            ));
        }
        Ok(())
    }

    /// Render the OBS request XML for this payload: one
    /// `maintenance_incident` action per source package, each with
    /// its source and the shared target (plus the release project,
    /// when set), and an `<options>` block only when cleanup is
    /// opted in. All text is XML-escaped.
    pub fn to_xml(&self) -> String {
        let mut actions = String::new();
        for package in &self.source_packages {
            actions.push_str("  <action type=\"maintenance_incident\">\n");
            actions.push_str(&format!(
                "    <source project=\"{}\" package=\"{}\" />\n",
                xml_escape(&self.source_project),
                xml_escape(package),
            ));
            actions.push_str(&format!(
                "    <target project=\"{}\"{} />\n",
                xml_escape(&self.target_project),
                match &self.target_releaseproject {
                    Some(release_project) =>
                        format!(" releaseproject=\"{}\"", xml_escape(release_project)),
                    None => String::new(),
                },
            ));
            if let Some(source_update) = self.source_update {
                actions.push_str(&format!(
                    "    <options><sourceupdate>{}</sourceupdate></options>\n",
                    source_update.as_str(),
                ));
            }
            actions.push_str("  </action>\n");
        }
        format!(
            "<request>\n{actions}  <description>{}</description>\n</request>",
            xml_escape(&self.description),
        )
    }

    /// Verify the rendered XML is well-formed and carries exactly
    /// one `maintenance_incident` action per source package, each
    /// with its source and target, plus the description — and a
    /// `<sourceupdate>` in every action exactly when cleanup is
    /// opted in, with a value the server accepts.
    pub fn validate_xml(&self) -> Result<(), GeectlError> {
        let xml = self.to_xml();
        check_well_formed(
            &xml,
            &[
                "request/action",
                "request/action/source",
                "request/action/target",
                "request/description",
            ],
        )?;
        let types = action_types(&xml)?;
        if types.len() != self.source_packages.len()
            || types.iter().any(|t| t != "maintenance_incident")
        {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "malformed maintenance incident XML: expected {} maintenance_incident action(s), found {} action(s) of type(s) {types:?}",
                    self.source_packages.len(),
                    types.len(),
                ),
            ));
        }
        let expected = if self.source_update.is_some() {
            self.source_packages.len()
        } else {
            0
        };
        if xml.matches("<sourceupdate>cleanup</sourceupdate>").count() != expected
            || xml.matches("<sourceupdate>").count() != expected
        {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "malformed maintenance incident XML: <sourceupdate> does not match the opted-in cleanup",
            ));
        }
        Ok(())
    }
}

/// The validated change-devel payload: which package's devel
/// project changes, and to what.
///
/// The source is the NEW devel (project + package); the target is
/// the package whose devel definition changes. On accept the server
/// points the target package's devel at the source. No options are
/// ever sent: the server forbids `sourceupdate`, `source_rev`,
/// `target_releaseproject`, and `target_repository` on this action
/// type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeDevelPayload {
    /// The project holding the new devel package.
    source_project: String,
    /// The new devel package.
    source_package: String,
    /// The project holding the package whose devel changes.
    target_project: String,
    /// The package whose devel changes.
    target_package: String,
    /// The request description text.
    description: String,
}

impl ChangeDevelPayload {
    /// Build and validate a payload. Every name must be a valid OBS
    /// project/package name and the description must be non-empty.
    pub fn new(
        source_project: &str,
        source_package: &str,
        target_project: &str,
        target_package: &str,
        description: &str,
    ) -> Result<Self, GeectlError> {
        let payload = ChangeDevelPayload {
            source_project: source_project.to_string(),
            source_package: source_package.to_string(),
            target_project: target_project.to_string(),
            target_package: target_package.to_string(),
            description: description.to_string(),
        };
        payload.validate_semantics()?;
        payload.validate_xml()?;
        Ok(payload)
    }

    /// The project holding the new devel package.
    pub fn source_project(&self) -> &str {
        &self.source_project
    }

    /// The new devel package.
    pub fn source_package(&self) -> &str {
        &self.source_package
    }

    /// The project holding the package whose devel changes.
    pub fn target_project(&self) -> &str {
        &self.target_project
    }

    /// The package whose devel changes.
    pub fn target_package(&self) -> &str {
        &self.target_package
    }

    /// The request description text.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The semantic checks [`ChangeDevelPayload::new`] runs: every
    /// name must be a valid OBS project/package name and the
    /// description must be non-empty.
    pub fn validate_semantics(&self) -> Result<(), GeectlError> {
        for (label, value) in [
            ("source project", self.source_project.as_str()),
            ("target project", self.target_project.as_str()),
        ] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS project name for {label}: {value:?}"),
                ));
            }
        }
        for (label, value) in [
            ("source package", self.source_package.as_str()),
            ("target package", self.target_package.as_str()),
        ] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS package name for {label}: {value:?}"),
                ));
            }
        }
        if self.description.trim().is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "change-devel description must not be empty",
            ));
        }
        Ok(())
    }

    /// Render the OBS request XML for this payload: one
    /// `change_devel` action with its source and target. All text
    /// is XML-escaped. No `<options>`: the server forbids them here.
    pub fn to_xml(&self) -> String {
        format!(
            "<request>\n  <action type=\"change_devel\">\n    <source project=\"{}\" package=\"{}\" />\n    <target project=\"{}\" package=\"{}\" />\n  </action>\n  <description>{}</description>\n</request>",
            xml_escape(&self.source_project),
            xml_escape(&self.source_package),
            xml_escape(&self.target_project),
            xml_escape(&self.target_package),
            xml_escape(&self.description),
        )
    }

    /// Verify the rendered XML is well-formed and carries exactly
    /// one `change_devel` action with its source and target, plus
    /// the description — and no `<options>` block.
    pub fn validate_xml(&self) -> Result<(), GeectlError> {
        let xml = self.to_xml();
        check_well_formed(
            &xml,
            &[
                "request/action",
                "request/action/source",
                "request/action/target",
                "request/description",
            ],
        )?;
        if action_types(&xml)? != ["change_devel"] {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "malformed change-devel XML: expected exactly one change_devel action",
            ));
        }
        if xml.contains("<options>") {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "malformed change-devel XML: <options> is forbidden on this action type",
            ));
        }
        Ok(())
    }
}

/// What a delete request targets: one package, one repository, or
/// the whole project. The server forbids a target package and a
/// target repository together, so the shape is an enum, not two
/// optional fields — the forbidden combination is unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteTarget {
    /// Delete one package from the target project.
    Package(String),
    /// Delete one repository from the target project.
    Repository(String),
    /// Delete the whole target project.
    WholeProject,
}

/// One maintained instance of a package: a project carrying the
/// maintenance attribute, with the package it would branch from and
/// where the branch would land. From the dry-run branch report
/// (`POST /source?cmd=branch&dryrun=1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintainedInstance {
    /// The project the package is branched from.
    pub source_project: String,
    /// The package that would be branched.
    pub source_package: String,
    /// The devel project the branch copies newer sources from, when
    /// the package has a devel package.
    pub devel_project: Option<String>,
    /// The devel package the branch copies newer sources from.
    pub devel_package: Option<String>,
    /// The project the branch would land in.
    pub target_project: String,
    /// The package the branch would land as — the report's name with
    /// `:` → `_`, matching the live branch.
    pub target_package: String,
}

/// Options for branching a maintained package (`POST
/// /source?cmd=branch`). The attribute marks which projects maintain
/// the package; the update project attribute points each of them at
/// its update project; the target project overrides the
/// server-derived branch project.
#[derive(Debug, Clone)]
pub struct MaintenanceBranchOptions {
    attribute: String,
    update_project_attribute: String,
    target_project: Option<String>,
}

impl MaintenanceBranchOptions {
    /// Defaults: attribute `OBS:Maintained`, update project
    /// attribute `OBS:UpdateProject`, no target project override
    /// (the server derives
    /// `home:<user>:branches:OBS_Maintained:<package>` — the attribute
    /// with `:` → `_`).
    pub fn new() -> Self {
        MaintenanceBranchOptions {
            attribute: "OBS:Maintained".to_string(),
            update_project_attribute: "OBS:UpdateProject".to_string(),
            target_project: None,
        }
    }

    /// The attribute that marks a project as maintaining the package
    /// (`NAMESPACE:NAME`, e.g. `OBS:Maintained`).
    pub fn with_attribute(mut self, attribute: &str) -> Result<Self, GeectlError> {
        if !valid_attribute_name(attribute) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS attribute name: {attribute:?} (expected NAMESPACE:NAME)"),
            ));
        }
        self.attribute = attribute.to_string();
        Ok(self)
    }

    /// The attribute that points a project at its update project
    /// (`NAMESPACE:NAME`, e.g. `OBS:UpdateProject`). The server
    /// refuses a value that is not in that shape.
    pub fn with_update_project_attribute(mut self, attribute: &str) -> Result<Self, GeectlError> {
        if !valid_attribute_name(attribute) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS attribute name: {attribute:?} (expected NAMESPACE:NAME)"),
            ));
        }
        self.update_project_attribute = attribute.to_string();
        Ok(self)
    }

    /// Branch into this project instead of the server-derived one.
    pub fn with_target_project(mut self, target_project: &str) -> Result<Self, GeectlError> {
        if !valid_obs_name(target_project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name for target project: {target_project:?}"),
            ));
        }
        self.target_project = Some(target_project.to_string());
        Ok(self)
    }
}

impl Default for MaintenanceBranchOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Where a maintenance branch landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceBranchResult {
    /// The project the branch landed in.
    pub target_project: String,
    /// The package the branch landed as. None when several packages
    /// were branched: the server then reports only the project.
    pub target_package: Option<String>,
    /// The project the package was branched from (single-package
    /// branches only).
    pub source_project: Option<String>,
    /// The package that was branched (single-package branches only).
    pub source_package: Option<String>,
}

/// The validated delete-request payload: what gets deleted, and
/// what the request says.
///
/// The target project is always required; the target (package,
/// repository, or whole project) comes from [`DeleteTarget`]. On
/// accept the server deletes exactly that — a package, a repository,
/// or the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletePayload {
    /// The project the deletion targets.
    target_project: String,
    /// What inside it gets deleted.
    target: DeleteTarget,
    /// The request description text.
    description: String,
}

impl DeletePayload {
    /// Build and validate a payload. The target project and (for a
    /// package or repository target) the target name must be valid
    /// OBS names, and the description must be non-empty.
    pub fn new(
        target_project: &str,
        target: DeleteTarget,
        description: &str,
    ) -> Result<Self, GeectlError> {
        let payload = DeletePayload {
            target_project: target_project.to_string(),
            target,
            description: description.to_string(),
        };
        payload.validate_semantics()?;
        payload.validate_xml()?;
        Ok(payload)
    }

    /// The project the deletion targets.
    pub fn target_project(&self) -> &str {
        &self.target_project
    }

    /// What inside it gets deleted.
    pub fn target(&self) -> &DeleteTarget {
        &self.target
    }

    /// The request description text.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The semantic checks [`DeletePayload::new`] runs: the target
    /// project and the target name (when there is one) must be valid
    /// OBS names, and the description must be non-empty.
    pub fn validate_semantics(&self) -> Result<(), GeectlError> {
        if !valid_obs_name(&self.target_project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "invalid OBS project name for target project: {:?}",
                    self.target_project
                ),
            ));
        }
        match &self.target {
            DeleteTarget::Package(package) => {
                if !valid_obs_name(package) {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!("invalid OBS package name for target package: {package:?}"),
                    ));
                }
            }
            DeleteTarget::Repository(repository) => {
                if !valid_obs_name(repository) {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "invalid OBS repository name for target repository: {repository:?}"
                        ),
                    ));
                }
            }
            DeleteTarget::WholeProject => {}
        }
        if self.description.trim().is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "delete description must not be empty",
            ));
        }
        Ok(())
    }

    /// Render the OBS request XML for this payload: one `delete`
    /// action with its target (package, repository, or neither for
    /// a whole-project delete). All text is XML-escaped.
    pub fn to_xml(&self) -> String {
        let target_attrs = match &self.target {
            DeleteTarget::Package(package) => {
                format!(" package=\"{}\"", xml_escape(package))
            }
            DeleteTarget::Repository(repository) => {
                format!(" repository=\"{}\"", xml_escape(repository))
            }
            DeleteTarget::WholeProject => String::new(),
        };
        format!(
            "<request>\n  <action type=\"delete\">\n    <target project=\"{}\"{} />\n  </action>\n  <description>{}</description>\n</request>",
            xml_escape(&self.target_project),
            target_attrs,
            xml_escape(&self.description),
        )
    }

    /// Verify the rendered XML is well-formed and carries exactly
    /// one `delete` action with its target, plus the description.
    pub fn validate_xml(&self) -> Result<(), GeectlError> {
        let xml = self.to_xml();
        check_well_formed(
            &xml,
            &[
                "request/action",
                "request/action/target",
                "request/description",
            ],
        )?;
        if action_types(&xml)? != ["delete"] {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "malformed delete XML: expected exactly one delete action",
            ));
        }
        Ok(())
    }
}

/// The `type` attributes of the `<action>` elements in `xml`, in
/// order. Err when an action tag is unterminated or carries no type
/// — a generated document is never that shape, so it is malformed,
/// not skippable. Used by the maintenance payloads' `validate_xml`
/// to prove the rendered actions are the ones the payload claims.
fn action_types(xml: &str) -> Result<Vec<String>, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::InvalidInput,
            "malformed request XML: action element without a type",
        )
    };
    let mut types = Vec::new();
    let mut rest = xml;
    while let Some(pos) = rest.find("<action") {
        let after_tag = &rest[pos + "<action".len()..];
        let Some(first) = after_tag.chars().next() else {
            break;
        };
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let Some(end) = after_tag.find('>') else {
            return Err(malformed());
        };
        let Some(action_type) = action_tag_type(&after_tag[..end]) else {
            return Err(malformed());
        };
        types.push(action_type);
        rest = &after_tag[end..];
    }
    Ok(types)
}

pub(crate) fn xml_attr(element: &str, name: &str) -> Option<String> {
    // Quote-aware: a `name=` inside an earlier attribute's value
    // (e.g. `details="x dirty=y"`) must not match. The scan tracks
    // whether it is inside a quoted value and only matches outside
    // one. The match position is always an ASCII byte (the first byte
    // of the ASCII needle), so the slicing below stays on UTF-8
    // boundaries.
    let needle = format!("{name}=");
    let bytes = element.as_bytes();
    let first = needle.as_bytes()[0];
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
        if b == first
            && element[i..].starts_with(&needle)
            && (i == 0 || bytes[i - 1].is_ascii_whitespace())
        {
            let after_eq = &element[i + needle.len()..];
            let quote = after_eq.chars().next()?;
            if quote != '"' && quote != '\'' {
                return None;
            }
            let close = after_eq[1..].find(quote)?;
            return Some(xml_unescape(&after_eq[1..1 + close]));
        }
        i += 1;
    }
    None
}

/// The text between `<name>` and `</name>` in `body` (first
/// occurrence), trimmed and XML-unescaped. The opening tag may carry
/// attributes (`<name lang="en">`); a `<namex>` prefix name is
/// skipped, not read as `<name>`. None when the element is absent or
/// unterminated. A self-closing `<name/>` carries no text, so it
/// reads as None.
fn xml_element_text(body: &str, name: &str) -> Option<String> {
    let open_marker = format!("<{name}");
    let close_marker = format!("</{name}>");
    let mut offset = 0;
    while offset < body.len() {
        let rest = &body[offset..];
        let pos = rest.find(&open_marker)?;
        let after_tag = &rest[pos + open_marker.len()..];
        let first = after_tag.chars().next()?;
        if !(first.is_whitespace() || first == '/' || first == '>') {
            offset += pos + open_marker.len();
            continue;
        }
        let end = find_tag_end(after_tag)?;
        let tag = &after_tag[..end];
        if tag.trim_end().ends_with('/') {
            // Self-closing: no text.
            return None;
        }
        let start = offset + pos + open_marker.len() + end + 1;
        let end = body[start..].find(&close_marker)?;
        let end = end + start;
        return Some(xml_unescape(body[start..end].trim()));
    }
    None
}

/// Split a full element text (`<name ...>...</name>` or `<name .../>`,
/// as returned by [`xml_elements`]) into its opening tag (without the
/// brackets) and its inner content. The inner content is empty for a
/// self-closing element.
pub(crate) fn xml_split_element<'a>(element: &'a str, name: &str) -> (&'a str, &'a str) {
    let after_lt = element.strip_prefix('<').unwrap_or(element);
    let Some(end) = find_tag_end(after_lt) else {
        return (after_lt, "");
    };
    let tag = &after_lt[..end];
    if tag.trim_end().ends_with('/') {
        return (tag, "");
    }
    let inner = &after_lt[end + 1..];
    let close_marker = format!("</{name}>");
    let inner = inner.strip_suffix(&close_marker).unwrap_or(inner);
    (tag, inner)
}

/// Parse an unsigned attribute value: None when the attribute is
/// absent, an error when it is present but not a number.
fn xml_attr_number<T>(tag: &str, name: &str) -> Result<Option<T>, GeectlError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match xml_attr(tag, name) {
        None => Ok(None),
        Some(value) => value.parse::<T>().map(Some).map_err(|_| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS response: attribute {name} is not a number"),
            )
        }),
    }
}

/// Parse a required unsigned attribute value: an error when it is
/// absent or not a number.
fn xml_attr_number_required<T>(tag: &str, name: &str) -> Result<T, GeectlError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    xml_attr_number(tag, name)?.ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected OBS response: missing attribute {name}"),
        )
    })
}

/// Check that `xml` is well-formed: every tag is balanced and properly
/// nested, there is exactly one root element, and the element paths in
/// `required` are all present. Returns the first problem found. The
/// required paths are the caller's contract — a submit payload needs
/// its action/source/target/options/sourceupdate/description, a
/// maintenance incident its own shape — so the balance check is
/// shared and the structure check is per payload.
fn check_well_formed(xml: &str, required: &[&str]) -> Result<(), GeectlError> {
    let mut stack: Vec<String> = Vec::new();
    // The element path of each element, for the required-structure check.
    let mut paths: Vec<String> = Vec::new();
    let bytes = xml.as_bytes();
    let mut i = 0;
    let mut root_seen = false;
    while i < bytes.len() {
        let Some(open) = xml[i..].find('<') else {
            break;
        };
        // `start` is the position of this tag; `i` (the scan position)
        // only advances at the end of the iteration, after the tag.
        let start = i + open;
        let Some(close_rel) = xml[start..].find('>') else {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "malformed XML: unterminated tag",
            ));
        };
        let close = start + close_rel;
        let raw = &xml[start + 1..close];
        if raw.starts_with('?') || raw.starts_with('!') {
            // Declaration / comment / doctype: not generated here, but
            // tolerated so a strict checker does not reject a harmless
            // prefix.
            i = close + 1;
            continue;
        }
        if let Some(name) = raw.strip_prefix('/') {
            // Close tag: must match the innermost open tag.
            let name = name.trim();
            match stack.pop() {
                Some(open_name) if open_name == name => {}
                Some(open_name) => {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "malformed XML: closing </{name}> does not match open <{open_name}>"
                        ),
                    ));
                }
                None => {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!("malformed XML: closing </{name}> with nothing open"),
                    ));
                }
            }
        } else if let Some(without_slash) = raw.strip_suffix('/') {
            // Self-closing tag: no stack change. It is a leaf; record
            // its path for the structure check.
            let name = without_slash.split_whitespace().next().unwrap_or("");
            if name.is_empty() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "malformed XML: empty tag",
                ));
            }
            let mut path = stack.join("/");
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(name);
            paths.push(path);
        } else {
            // Open tag: push it. The tag name is the first word.
            let name = raw.split_whitespace().next().unwrap_or("");
            if name.is_empty() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "malformed XML: empty tag",
                ));
            }
            if stack.is_empty() {
                if root_seen {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        "malformed XML: more than one root element",
                    ));
                }
                root_seen = true;
            }
            stack.push(name.to_string());
            let path = stack.join("/");
            paths.push(path);
        }
        i = close + 1;
    }
    if !stack.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("malformed XML: unclosed tag(s): {}", stack.join(", ")),
        ));
    }
    if !root_seen {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "malformed XML: no root element",
        ));
    }
    // Required structure: exactly one root `request`, plus the
    // element paths the caller requires.
    let has = |path: &str| paths.iter().any(|p| p == path);
    if !has("request") {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "malformed request XML: root element is not <request>",
        ));
    }
    for path in required {
        if !has(path) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("malformed request XML: missing <{path}>"),
            ));
        }
    }
    Ok(())
}

/// The element paths a submit-request payload must carry, for
/// [`check_well_formed`].
const SUBMIT_REQUIRED_PATHS: &[&str] = &[
    "request/action",
    "request/action/source",
    "request/action/target",
    "request/action/options",
    "request/action/options/sourceupdate",
    "request/description",
];

/// Files a submit request through the OBS API, and refiles one over
/// an existing request.
///
/// The only filing path. In dry-run, [`SrFiling::file`] logs exactly
/// what would be filed and verifies the payload, and files nothing. In
/// live mode it first checks no open submit request already targets
/// the package (dedup — an open SR is refused, not filed over), then
/// POSTs the payload XML and returns the new request id.
/// [`SrFiling::refile`] extends that path: it files the new request
/// and supersedes the old one with it, in that order.
pub struct SrFiling<'a> {
    obs: &'a ObsClient<'a>,
    dry_run: &'a DryRun,
    debug: Debug,
}

impl<'a> SrFiling<'a> {
    pub fn new(obs: &'a ObsClient<'a>, dry_run: &'a DryRun) -> Self {
        SrFiling {
            obs,
            dry_run,
            debug: Debug::disabled(),
        }
    }

    /// Report the channel's dedup decision lines to `debug`
    /// instead of dropping them. The sink is off by default; the
    /// caller that owns the debug channel passes its sink here.
    pub fn with_debug(mut self, debug: Debug) -> Self {
        self.debug = debug;
        self
    }

    /// File `payload`. Returns the new OBS request id in live mode, or
    /// None in dry-run (nothing was filed). The payload is validated
    /// before either path is taken. In live mode an open submit
    /// request already targeting the package is refused (dedup). The
    /// payload carries `opt_sourceupdate="cleanup"` by default, so
    /// OBS deletes the source branch when the request is accepted;
    /// see [`SrPayload::without_cleanup`] for the opt-out.
    pub fn file(&self, payload: &SrPayload) -> Result<Option<String>, GeectlError> {
        self.file_with_except(payload, None)
    }

    /// File `payload`, exempting `except` from the dedup check: an
    /// open submit request with that id targeting the package does
    /// not block the filing. [`SrFiling::refile`] passes the request
    /// it is about to supersede, because that request is open by
    /// definition — without the exemption a refile could never file.
    /// Any OTHER open request targeting the package still blocks the
    /// filing, and is named in the error. A caller that passes an id
    /// that is not actually open gains nothing: the exemption only
    /// skips a request that is there.
    fn file_with_except(
        &self,
        payload: &SrPayload,
        except: Option<u64>,
    ) -> Result<Option<String>, GeectlError> {
        // The payload is validated on the filing path, not only at
        // construction: the semantic checks run here as well as the
        // XML check, so a payload that reaches filing is a valid
        // payload by both measures.
        payload.validate_semantics()?;
        payload.validate_xml()?;
        if !self.dry_run.allow(&format!(
            "file submit request for {} to {}",
            payload.target_package, payload.target_project
        )) {
            // Dry-run: log exactly what would be filed, in full, so
            // the filing path is exercised end to end without filing.
            // (The `allow` call above already recorded the would-do
            // line in the plan.) The detail lines go through `note`,
            // not `eprintln`, so the plan carries them too.
            self.dry_run
                .note("--- would-be submit request (dry-run) ---");
            self.dry_run.note(&format!(
                "source: {}/{}",
                payload.source_project, payload.source_package
            ));
            self.dry_run.note(&format!(
                "target: {}/{}",
                payload.target_project, payload.target_package
            ));
            self.dry_run
                .note(&format!("description: {}", payload.description));
            self.dry_run
                .note(&format!("changes entry: {}", payload.changes_entry));
            self.dry_run.note(&payload.to_xml());
            self.dry_run.note("--- end of would-be submit request ---");
            return Ok(None);
        }
        // Live: the dedup check is part of filing. An open submit
        // request for the package must be checked before filing, so a
        // duplicate is refused here instead of filed. (The check
        // itself is also available as `ObsClient::has_open_sr`.)
        let open = self.obs.open_target_request_ids(&payload.target_package)?;
        let blocking: Vec<&String> = open
            .iter()
            .filter(|id| match except {
                None => true,
                // An id that does not parse cannot be the exempted
                // one: it blocks, rather than being skipped on a
                // failed comparison.
                Some(n) => id.parse::<u64>() != Ok(n),
            })
            .collect();
        if !blocking.is_empty() {
            let blocking = blocking
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<&str>>()
                .join(", ");
            self.debug.emit(&format!(
                "decision: open submit request(s) {blocking} already target {}; refusing to file",
                payload.target_package
            ));
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "an open submit request already targets {} ({blocking}); not filing a duplicate",
                    payload.target_package
                ),
            ));
        }
        self.debug.emit(&format!(
            "decision: no open submit request targets {}; filing",
            payload.target_package
        ));

        let id = self.obs.create_request(payload)?;
        Ok(Some(id))
    }

    /// File `payload` as a new submit request and supersede `old_id`
    /// with it: the full refile flow. Returns the new request id in
    /// live mode, or None in dry-run (nothing was filed or
    /// superseded).
    ///
    /// The order is the guarantee: the new request is filed FIRST,
    /// and only then is the old one superseded pointing at it, so
    /// `superseded_by` always names a request that exists. (The API
    /// does not verify the replacement exists — it only checks the
    /// number is positive — so filing first is what makes the link
    /// real. osc files first too, in `submitreq --supersede`.)
    ///
    /// The filing step exempts the old request from the dedup check:
    /// the old request is open by definition, and without the
    /// exemption a refile could never file. Any OTHER open request
    /// targeting the package still blocks the filing.
    ///
    /// If superseding fails after the new request is filed, the error
    /// names the new id: the old request is still open and the new one
    /// unlinked, and the caller finishes the supersede manually. That
    /// is a deliberate divergence from osc, which supersedes with
    /// `can_fail=True` and only warns — a half-done refile passing
    /// silently is a failed refile that reads as a success.
    ///
    /// Branch deletion of the OLD branch is NOT part of this method:
    /// the old branch stays until the caller deletes it explicitly
    /// with [`ObsClient::delete_package`], once the new request is
    /// resolved (or the old branch is otherwise safe to drop). A
    /// superseded request never reaches accept, so the server-side
    /// cleanup never runs for the old branch — only the NEW branch
    /// dies on accept, by the payload's default cleanup. The full
    /// sequence is: file -> supersede -> delete old branch.
    pub fn refile(&self, payload: &SrPayload, old_id: &str) -> Result<Option<String>, GeectlError> {
        // Validate the old id before filing anything: a malformed id
        // must not leave a filed-but-unlinked new request behind.
        let old = validate_request_id(old_id)?;
        let new_id = self.file_with_except(payload, Some(old))?;
        let Some(new_id) = new_id else {
            if !self.dry_run.enabled() {
                return Err(GeectlError::new(
                    ErrorKind::InternalError,
                    "filing in live mode returned no request id",
                ));
            }
            // Dry-run: `file` logged the would-file and returned
            // None. Record the would-supersede against the request
            // that would be filed, and file nothing.
            self.dry_run.note(&format!(
                "would: supersede submit request {old} with the new request"
            ));
            return Ok(None);
        };
        match self.obs.supersede_request(old_id, &new_id) {
            Ok(()) => Ok(Some(new_id)),
            Err(err) => Err(GeectlError::new(
                err.kind(),
                format!(
                    "filed submit request {new_id} but could not supersede {old}: {}; the old request is still open — supersede it manually with the new id",
                    err.context
                ),
            )),
        }
    }

    /// File `payload` as a maintenance incident request. Returns the
    /// new OBS request id in live mode, or None in dry-run (nothing
    /// was filed). The payload is validated before either path is
    /// taken. In live mode an open maintenance incident request
    /// already targeting any of the payload's sources is refused
    /// (dedup, one check per source package); a filing without a
    /// release project is refused by any open incident for the same
    /// source package and target, whatever release project it
    /// resolved to — that over-blocks the narrow cases where the
    /// link-derived release project differs. The filing pins
    /// `source_rev` (`addrevision=1`), so the incident freezes the
    /// exact source it was filed against. The payload carries no
    /// sourceupdate by default; see
    /// [`MaintenanceIncidentPayload::with_cleanup`] for the opt-in
    /// branch cleanup on accept.
    pub fn file_maintenance_incident(
        &self,
        payload: &MaintenanceIncidentPayload,
    ) -> Result<Option<String>, GeectlError> {
        self.file_maintenance_incident_with_except(payload, None)
    }

    fn file_maintenance_incident_with_except(
        &self,
        payload: &MaintenanceIncidentPayload,
        except: Option<u64>,
    ) -> Result<Option<String>, GeectlError> {
        payload.validate_semantics()?;
        payload.validate_xml()?;
        let sources = payload
            .source_packages
            .iter()
            .map(|package| format!("{}/{}", payload.source_project, package))
            .collect::<Vec<String>>()
            .join(", ");
        if !self.dry_run.allow(&format!(
            "file maintenance incident request for {sources} to {}",
            payload.target_project
        )) {
            self.dry_run
                .note("--- would-be maintenance incident request (dry-run) ---");
            for package in &payload.source_packages {
                self.dry_run
                    .note(&format!("source: {}/{}", payload.source_project, package));
            }
            self.dry_run
                .note(&format!("target: {}", payload.target_project));
            self.dry_run
                .note(&format!("description: {}", payload.description));
            self.dry_run.note(&payload.to_xml());
            self.dry_run
                .note("--- end of would-be maintenance incident request ---");
            return Ok(None);
        }
        // Live: the dedup check is part of filing, one check per
        // source package — an incident carries one action per source,
        // and an open incident for any of them blocks the filing.
        // The match mirrors the server's incident identity
        // (`uniq_key`: target, source package, release project) — the
        // source project is not part of it, so an equivalent incident
        // from another branch blocks too, and the server would not
        // catch it (no cross-request duplicate check).
        for package in &payload.source_packages {
            self.check_no_open_action_request(
                &ActionTarget {
                    action_type: "maintenance_incident",
                    source_project: None,
                    source_package: Some(package),
                    target_project: &payload.target_project,
                    target_package: None,
                    target_repository: None,
                    shape: ActionShape::ReleaseProject(payload.target_releaseproject.as_deref()),
                },
                &format!("{} for {}", payload.target_project, package),
                except,
            )?;
        }
        let xml = payload.to_xml();
        let id = self.obs.create_request_with_add_revision(&xml)?;
        Ok(Some(id))
    }

    /// File `payload` as a new maintenance incident request and
    /// supersede `old_id` with it: the full refile flow, same order
    /// guarantee as [`SrFiling::refile`] — file first, supersede
    /// second, so `superseded_by` always names a request that
    /// exists. Returns the new request id in live mode, or None in
    /// dry-run (nothing was filed or superseded).
    pub fn refile_maintenance_incident(
        &self,
        payload: &MaintenanceIncidentPayload,
        old_id: &str,
    ) -> Result<Option<String>, GeectlError> {
        self.refile_with(
            |except| self.file_maintenance_incident_with_except(payload, except),
            old_id,
            "maintenance incident",
        )
    }

    /// File `payload` as a change devel request: move the devel
    /// project of a package. Returns the new OBS request id in live
    /// mode, or None in dry-run (nothing was filed). The payload is
    /// validated before either path is taken. In live mode an open
    /// change devel request already targeting the same target
    /// is refused (dedup); the dedup keys on the target alone, and
    /// the source is not part of the identity. The server sets the
    /// devel project on accept; there is no branch cleanup to opt into.
    pub fn file_change_devel(
        &self,
        payload: &ChangeDevelPayload,
    ) -> Result<Option<String>, GeectlError> {
        payload.validate_semantics()?;
        payload.validate_xml()?;
        if !self.dry_run.allow(&format!(
            "file change devel request for {}/{} to {}/{}",
            payload.source_project,
            payload.source_package,
            payload.target_project,
            payload.target_package
        )) {
            self.dry_run
                .note("--- would-be change devel request (dry-run) ---");
            self.dry_run.note(&format!(
                "source: {}/{}",
                payload.source_project, payload.source_package
            ));
            self.dry_run.note(&format!(
                "target: {}/{}",
                payload.target_project, payload.target_package
            ));
            self.dry_run
                .note(&format!("description: {}", payload.description));
            self.dry_run.note(&payload.to_xml());
            self.dry_run
                .note("--- end of would-be change devel request ---");
            return Ok(None);
        }
        // The match mirrors the server's change-devel identity
        // (`uniq_key`: target project and package only) — the source
        // is not part of it, so a request naming another new devel
        // for the same target blocks too, and the server would not
        // catch it (no cross-request duplicate check).
        self.check_no_open_action_request(
            &ActionTarget {
                action_type: "change_devel",
                source_project: None,
                source_package: None,
                target_project: &payload.target_project,
                target_package: Some(&payload.target_package),
                target_repository: None,
                shape: ActionShape::Any,
            },
            &format!("{}/{}", payload.target_project, payload.target_package),
            None,
        )?;
        let xml = payload.to_xml();
        let id = self
            .obs
            .create_request_with_query(&xml, &[("cmd", "create")], "submit")?;
        Ok(Some(id))
    }

    /// File `payload` as a delete request: delete a package, a
    /// repository, or a whole project. Returns the new OBS request id
    /// in live mode, or None in dry-run (nothing was filed). The
    /// payload is validated before either path is taken. In live mode
    /// an open delete request already targeting the same project (and
    /// package or repository, when the payload names one) is refused
    /// (dedup). The server deletes the target on accept.
    pub fn file_delete_request(
        &self,
        payload: &DeletePayload,
    ) -> Result<Option<String>, GeectlError> {
        payload.validate_semantics()?;
        payload.validate_xml()?;
        let target = match &payload.target {
            DeleteTarget::Package(package) => {
                format!("{}/{}", payload.target_project, package)
            }
            DeleteTarget::Repository(repository) => {
                format!("{} (repository {repository})", payload.target_project)
            }
            DeleteTarget::WholeProject => payload.target_project.clone(),
        };
        if !self
            .dry_run
            .allow(&format!("file delete request for {target}"))
        {
            self.dry_run
                .note("--- would-be delete request (dry-run) ---");
            self.dry_run.note(&format!("target: {target}"));
            self.dry_run
                .note(&format!("description: {}", payload.description));
            self.dry_run.note(&payload.to_xml());
            self.dry_run.note("--- end of would-be delete request ---");
            return Ok(None);
        }
        let (target_package, target_repository) = match &payload.target {
            DeleteTarget::Package(package) => (Some(package.as_str()), None),
            DeleteTarget::Repository(repository) => (None, Some(repository.as_str())),
            DeleteTarget::WholeProject => (None, None),
        };
        self.check_no_open_action_request(
            &ActionTarget {
                action_type: "delete",
                source_project: None,
                source_package: None,
                target_project: &payload.target_project,
                target_package,
                target_repository,
                shape: ActionShape::DeleteTarget {
                    package: target_package,
                    repository: target_repository,
                },
            },
            &target,
            None,
        )?;
        let xml = payload.to_xml();
        let id = self
            .obs
            .create_request_with_query(&xml, &[("cmd", "create")], "submit")?;
        Ok(Some(id))
    }

    /// Refuse when an open request already matches `target` exactly.
    /// The search filters on the action type server-side; the parser
    /// re-checks the type and narrows by shape (delete target shape,
    /// incident release project) where the search cannot. `except`,
    /// when set, is the request the refile flow is about to
    /// supersede: it is open by definition and does not block.
    /// `what` names the target in the refusal, e.g.
    /// `home:alice:branches:foo/foo`. A declined request is closed
    /// and does not block — only `new`/`review` requests are in the
    /// search.
    fn check_no_open_action_request(
        &self,
        target: &ActionTarget<'_>,
        what: &str,
        except: Option<u64>,
    ) -> Result<(), GeectlError> {
        let open = self.obs.open_action_request_ids(target)?;
        let blocking: Vec<&String> = open
            .iter()
            .filter(|id| match except {
                None => true,
                // An id that does not parse cannot be the exempted
                // one: it blocks, rather than being skipped on a
                // failed comparison.
                Some(n) => id.parse::<u64>() != Ok(n),
            })
            .collect();
        if !blocking.is_empty() {
            let blocking = blocking
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<&str>>()
                .join(", ");
            self.debug.emit(&format!(
                "decision: open {} request(s) {blocking} already target {what}; refusing to file",
                target.action_type
            ));
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "an open {} request already targets {what} ({blocking}); not filing a duplicate",
                    target.action_type
                ),
            ));
        }
        self.debug.emit(&format!(
            "decision: no open {} request targets {what}; filing",
            target.action_type
        ));
        Ok(())
    }

    /// The shared refile flow: file first, then supersede the old
    /// request pointing at the new one, so `superseded_by` always
    /// names a request that exists. `file` is the filing step (with
    /// the old request exempted from dedup); `kind` names the
    /// request type in messages ("maintenance incident").
    /// If superseding fails after the new request is filed, the error
    /// names the new id: the old request is still open and the new
    /// one unlinked, and the caller finishes the supersede manually.
    /// A half-done refile passing silently is a failed refile that
    /// reads as a success.
    fn refile_with<F>(
        &self,
        file: F,
        old_id: &str,
        kind: &str,
    ) -> Result<Option<String>, GeectlError>
    where
        F: FnOnce(Option<u64>) -> Result<Option<String>, GeectlError>,
    {
        let old = validate_request_id(old_id)?;
        let new_id = file(Some(old))?;
        let Some(new_id) = new_id else {
            if !self.dry_run.enabled() {
                return Err(GeectlError::new(
                    ErrorKind::InternalError,
                    "filing in live mode returned no request id",
                ));
            }
            self.dry_run.note(&format!(
                "would: supersede {kind} request {old} with the new request"
            ));
            return Ok(None);
        };
        match self.obs.supersede_request(old_id, &new_id) {
            Ok(()) => Ok(Some(new_id)),
            Err(err) => Err(GeectlError::new(
                err.kind(),
                format!(
                    "filed {kind} request {new_id} but could not supersede {old}: {}; the old request is still open — supersede it manually with the new id",
                    err.context
                ),
            )),
        }
    }
}

/// A single XML element found by scanning: its tag name, the raw
/// attribute text between the tag name and the closing `>` (without a
/// trailing `/` for a self-closing tag), and its inner XML (empty for
/// a self-closing tag). Borrowed from the document being parsed.
struct Element<'a> {
    name: &'a str,
    attrs: &'a str,
    inner: &'a str,
}

/// Split `xml` — the inner XML of one parent element — into its
/// top-level child elements, in document order. Text between elements
/// must be whitespace only: a request document never carries bare text
/// under a parent, so anything else is a shape change and the split
/// fails. A child is found by its opening tag and its matching close
/// tag (plain search, the same discipline as
/// [`parse_submit_request_ids`]): these documents never nest an
/// element inside another of the same name, so no depth counting is
/// needed, and a missing close tag fails the whole document rather
/// than yielding a partial element list.
fn child_elements(xml: &str) -> Option<Vec<Element<'_>>> {
    let mut elements = Vec::new();
    let mut rest = xml;
    loop {
        let text_end = rest.find('<').unwrap_or(rest.len());
        if !rest[..text_end].trim().is_empty() {
            return None;
        }
        rest = &rest[text_end..];
        if rest.is_empty() {
            return Some(elements);
        }
        let after_lt = &rest[1..];
        if after_lt.starts_with('/') || after_lt.starts_with('!') || after_lt.starts_with('?') {
            return None;
        }
        let name_end = after_lt
            .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .unwrap_or(after_lt.len());
        let name = &after_lt[..name_end];
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
        {
            return None;
        }
        let after_name = &after_lt[name_end..];
        let tag_end = after_name.find('>')?;
        let tag = &after_name[..tag_end];
        if let Some(attrs) = tag.trim_end().strip_suffix('/') {
            elements.push(Element {
                name,
                attrs: attrs.trim_end(),
                inner: "",
            });
            rest = &after_name[tag_end + 1..];
            continue;
        }
        let inner_start = &after_name[tag_end + 1..];
        let close_tag = format!("</{name}>");
        let close = inner_start.find(&close_tag)?;
        elements.push(Element {
            name,
            attrs: tag,
            inner: &inner_start[..close],
        });
        rest = &inner_start[close + close_tag.len()..];
    }
}

/// The value of the `name` attribute in an opening tag's attribute
/// text, in either quote style, order-tolerant. The attribute name
/// must stand on its own — `subtype="..."` must not match a lookup
/// for `type` — so the character before `name=` has to be whitespace
/// or the start of the text. The value is returned raw: the attribute
/// values this parser reads (ids, logins, project and package names,
/// timestamps, state and role names) cannot carry XML entities, so
/// there is nothing to unescape in them. None when the attribute is
/// absent or unterminated.
fn attr_value<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=");
    let mut rest = attrs;
    while let Some(pos) = rest.find(&needle) {
        if pos > 0 && !rest.as_bytes()[pos - 1].is_ascii_whitespace() {
            rest = &rest[pos + needle.len()..];
            continue;
        }
        let after_eq = &rest[pos + needle.len()..];
        let quote = after_eq.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let close = after_eq[1..].find(quote)?;
        return Some(&after_eq[1..1 + close]);
    }
    None
}

/// The text of a leaf element's inner XML, unescaped. The inner XML
/// must be pure text: any `<` in it means the element carries markup
/// the model does not expect, and the leaf fails. An empty element
/// reads as the empty string.
fn leaf_text(inner: &str) -> Option<String> {
    if inner.contains('<') {
        return None;
    }
    Some(xml_unescape(inner))
}

/// The state a submit request is in. The values are exactly the
/// server's `VALID_REQUEST_STATES` (`BsRequest`): a request the
/// library reads can be in any of them, including `deleted`, which
/// only an admin can produce — the library never sends it (see
/// `ObsClient::change_state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestState {
    /// Freshly filed; nobody has acted on it yet.
    New,
    /// Under review; reviewers have been asked.
    Review,
    /// Accepted; the change landed.
    Accepted,
    /// Declined; the change was refused.
    Declined,
    /// Revoked by its creator; withdrawn.
    Revoked,
    /// Superseded by a newer request.
    Superseded,
    /// Deleted by an admin.
    Deleted,
}

impl RequestState {
    /// The server's spelling of this state, as it appears in
    /// `<state name="…">` and in `newstate=` query values.
    pub fn as_str(&self) -> &'static str {
        match self {
            RequestState::New => "new",
            RequestState::Review => "review",
            RequestState::Accepted => "accepted",
            RequestState::Declined => "declined",
            RequestState::Revoked => "revoked",
            RequestState::Superseded => "superseded",
            RequestState::Deleted => "deleted",
        }
    }

    /// Parse a state name the server sent. None for anything else:
    /// an unknown state is a server shape change, not a state the
    /// library can act on.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "new" => Some(RequestState::New),
            "review" => Some(RequestState::Review),
            "accepted" => Some(RequestState::Accepted),
            "declined" => Some(RequestState::Declined),
            "revoked" => Some(RequestState::Revoked),
            "superseded" => Some(RequestState::Superseded),
            "deleted" => Some(RequestState::Deleted),
            _ => None,
        }
    }
}

/// The state of one review on a request. The values are exactly the
/// server's `VALID_REVIEW_STATES` (`Review`) — a review's own states,
/// not the request's: `obsoleted` is set when the reviewer is removed
/// and never appears on a request, and a request's `review` and
/// `revoked` never appear on a review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewState {
    /// Asked for; nobody has acted on it yet.
    New,
    /// Accepted by the reviewer.
    Accepted,
    /// Declined by the reviewer.
    Declined,
    /// Superseded by a newer review of the same target.
    Superseded,
    /// The reviewer was removed, so the review no longer applies.
    Obsoleted,
}

impl ReviewState {
    /// The server's spelling of this state, as it appears in
    /// `<review state="…">`.
    pub fn as_str(&self) -> &'static str {
        match self {
            ReviewState::New => "new",
            ReviewState::Accepted => "accepted",
            ReviewState::Declined => "declined",
            ReviewState::Superseded => "superseded",
            ReviewState::Obsoleted => "obsoleted",
        }
    }

    /// Parse a review state name the server sent. None for anything
    /// else: an unknown state is a server shape change, not a state
    /// the library can act on.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "new" => Some(ReviewState::New),
            "accepted" => Some(ReviewState::Accepted),
            "declined" => Some(ReviewState::Declined),
            "superseded" => Some(ReviewState::Superseded),
            "obsoleted" => Some(ReviewState::Obsoleted),
            _ => None,
        }
    }
}

/// A request's priority. The values are exactly the server's
/// `VALID_REQUEST_PRIORITIES` (`BsRequest`): anything else is refused
/// by the server at creation, so a priority the parser does not
/// recognise is a shape change, not a priority. The server omits the
/// `<priority>` element for the default `moderate` on its own output;
/// the serializer renders the model as given, so a `Moderate` it was
/// told about is written out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestPriority {
    /// Below the default; handled when there is time.
    Low,
    /// The default; the server omits this value from its output.
    Moderate,
    /// Above the default; handled soon.
    Important,
    /// The highest; handled first.
    Critical,
}

impl RequestPriority {
    /// The server's spelling of this priority.
    pub fn as_str(&self) -> &'static str {
        match self {
            RequestPriority::Low => "low",
            RequestPriority::Moderate => "moderate",
            RequestPriority::Important => "important",
            RequestPriority::Critical => "critical",
        }
    }

    /// Parse a priority the server sent. None for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "low" => Some(RequestPriority::Low),
            "moderate" => Some(RequestPriority::Moderate),
            "important" => Some(RequestPriority::Important),
            "critical" => Some(RequestPriority::Critical),
            _ => None,
        }
    }
}

/// The `<acceptinfo>` of a submit action: the revision the request
/// was accepted at, when it was accepted. Only present on a request
/// that has been accepted; a request that is still open carries none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptInfo {
    /// The accepted source revision number.
    pub rev: String,
    /// The accepted source md5.
    pub srcmd5: String,
    /// The origin project, when the accepted revision came from
    /// elsewhere (a forwarded request).
    pub oproject: Option<String>,
    /// The origin package, when the accepted revision came from
    /// elsewhere.
    pub opackage: Option<String>,
    /// The origin source md5, when the accepted revision came from
    /// elsewhere.
    pub osrcmd5: Option<String>,
    /// The expanded source md5 at accept time.
    pub xsrcmd5: Option<String>,
    /// The origin expanded source md5, when the accepted revision
    /// came from elsewhere.
    pub oxsrcmd5: Option<String>,
}

/// One action of a submit request: what the request does. The
/// variants are the action types the server renders
/// (`BsRequestAction`): a document carrying an action type the
/// library does not know is refused at parse time, so a new server
/// action type surfaces as an error instead of silently dropping the
/// action that carries the change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestAction {
    /// Submit a package's changes from a source to a target. The
    /// target is optional: a submit action the server returns can
    /// carry no `<target>` element at all.
    Submit {
        /// The project the change comes from.
        source_project: String,
        /// The package in the source project.
        source_package: String,
        /// The source revision, when the request pins one.
        source_rev: Option<String>,
        /// The project the change goes to, when the action names one.
        target_project: Option<String>,
        /// The package in the target project, when the action names
        /// one.
        target_package: Option<String>,
        /// What happens to the source branch on accept, when the
        /// request sets it.
        sourceupdate: Option<SourceUpdate>,
        /// Whether the request updates the devel-project link on
        /// accept.
        updatelink: bool,
        /// Whether the request makes the origin older on accept.
        makeoriginolder: bool,
        /// The accept revision, when the request was accepted.
        acceptinfo: Option<AcceptInfo>,
    },
    /// Delete a project or a package.
    Delete {
        /// The project to delete.
        target_project: String,
        /// The package to delete; absent when the whole project goes.
        target_package: Option<String>,
        /// The repository to delete, when the request names one.
        target_repository: Option<String>,
    },
    /// Grant a person or a group a role on a project or a package.
    AddRole {
        /// The project the role is granted on.
        target_project: String,
        /// The package the role is granted on; absent for a
        /// project-level grant.
        target_package: Option<String>,
        /// The person and the role granted, when the action names one.
        person: Option<(String, String)>,
        /// The group and the role granted, when the action names one.
        group: Option<(String, String)>,
    },
    /// Set the bugowner of a project or a package.
    SetBugowner {
        /// The project the bugowner is set on.
        target_project: String,
        /// The package the bugowner is set on; absent for a
        /// project-level setting.
        target_package: Option<String>,
        /// The person set as bugowner, when the action names one.
        person_name: Option<String>,
        /// The group set as bugowner, when the action names one.
        group_name: Option<String>,
    },
    /// Change the devel project of a package.
    ChangeDevel {
        /// The project the package lives in.
        source_project: String,
        /// The package whose devel project changes.
        source_package: String,
        /// The project that becomes the devel project.
        target_project: String,
        /// The package in the devel project.
        target_package: String,
    },
    /// Create a maintenance incident from a source package.
    MaintenanceIncident {
        /// The project the incident comes from.
        source_project: String,
        /// The package the incident is built from.
        source_package: String,
        /// The maintenance project the incident goes to.
        target_project: String,
        /// The package in the target project, when the action names
        /// one.
        target_package: Option<String>,
    },
    /// Release a maintenance incident to the maintenance project.
    MaintenanceRelease {
        /// The project the release comes from.
        source_project: String,
        /// The package the release is built from.
        source_package: String,
        /// The project the release goes to.
        target_project: String,
        /// The package in the target project, when the action names
        /// one.
        target_package: Option<String>,
    },
    /// Release a package to another project.
    Release {
        /// The project the release comes from.
        source_project: String,
        /// The package being released.
        source_package: String,
        /// The project the release goes to.
        target_project: String,
        /// The package in the target project, when the action names
        /// one.
        target_package: Option<String>,
    },
}

impl RequestAction {
    /// The server's spelling of this action's type, as it appears in
    /// `<action type="…">`.
    pub fn type_name(&self) -> &'static str {
        match self {
            RequestAction::Submit { .. } => "submit",
            RequestAction::Delete { .. } => "delete",
            RequestAction::AddRole { .. } => "add_role",
            RequestAction::SetBugowner { .. } => "set_bugowner",
            RequestAction::ChangeDevel { .. } => "change_devel",
            RequestAction::MaintenanceIncident { .. } => "maintenance_incident",
            RequestAction::MaintenanceRelease { .. } => "maintenance_release",
            RequestAction::Release { .. } => "release",
        }
    }
}

/// The `<state>` element of a request: the state it is in now, who
/// put it there, and when. The `comment` is the message left with the
/// last state change; it is empty when the change carried none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestStateInfo {
    /// The state the request is in.
    pub name: RequestState,
    /// Who made the last state change.
    pub who: String,
    /// When the last state change happened.
    pub when: String,
    /// When the request was created. The server always sends this;
    /// older documents may not carry it.
    pub created: Option<String>,
    /// The request this one was superseded by, when it was superseded.
    pub superseded_by: Option<u64>,
    /// Who approved the request, when it carries an approval.
    pub approver: Option<String>,
    /// The message left with the last state change; empty when none.
    pub comment: String,
}

/// One `<review>` on a request: a review asked of a user, a group, a
/// project, or a package. The `comment` is the reason the review was
/// asked for, or the message left with it. The `history` is what
/// happened to the review since it was asked — accepted, declined,
/// assigned, reopened, obsoleted — one entry per change, in order;
/// a review nobody has acted on yet has none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestReview {
    /// The review's state.
    pub state: ReviewState,
    /// When the review was asked for or last changed.
    pub when: Option<String>,
    /// Who asked for or last changed the review.
    pub who: Option<String>,
    /// The user the review is asked of, when it names one.
    pub by_user: Option<String>,
    /// The group the review is asked of, when it names one.
    pub by_group: Option<String>,
    /// The project the review is asked of, when it names one.
    pub by_project: Option<String>,
    /// The package the review is asked of, when it names one.
    pub by_package: Option<String>,
    /// The reason for the review; empty when none was given.
    pub comment: String,
    /// What happened to the review since it was asked, in order.
    pub history: Vec<RequestHistory>,
}

/// One `<history>` entry of a request: a state change, who made it
/// and when, and what it was. The first entry is always the creation
/// (`description` "Request created", `comment` the request's
/// description); the rest are the state changes in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestHistory {
    /// Who made the change.
    pub who: String,
    /// When the change happened.
    pub when: String,
    /// What the change was; empty when the entry carries none.
    pub description: String,
    /// The message left with the change; empty when none.
    pub comment: String,
}

/// A submit request as the server renders it: its id and creator,
/// what it does, the state it is in, its reviews and history, and its
/// message. Read with `ObsClient::show_request`; a document the
/// parser does not recognise in full is refused rather than read
/// partially, so a caller never acts on a half-parsed request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The request number.
    pub id: u64,
    /// Who filed the request.
    pub creator: String,
    /// The request's title, when the document carries one. The server
    /// does not render a title on its own output; older documents may
    /// carry one.
    pub title: Option<String>,
    /// The request's description (its message); empty when none.
    pub description: String,
    /// What the request does, in order.
    pub actions: Vec<RequestAction>,
    /// The request's priority, when the document carries one. The
    /// server omits the default `moderate` from its output.
    pub priority: Option<RequestPriority>,
    /// The state the request is in now.
    pub state: RequestStateInfo,
    /// The reviews asked on the request, in order.
    pub reviews: Vec<RequestReview>,
    /// The request's history, in order, starting with its creation.
    /// Only present when the document was fetched with history (see
    /// `ObsClient::show_request`); empty otherwise.
    pub history: Vec<RequestHistory>,
    /// When the request is scheduled to be accepted, when one is set.
    pub accept_at: Option<String>,
}

impl Request {
    /// The actions of the given types, in order. An empty type list
    /// returns every action. A type name that matches no action
    /// simply returns nothing for it — filtering is the caller's
    /// choice, not a parse error.
    pub fn actions_of(&self, types: &[&str]) -> Vec<&RequestAction> {
        if types.is_empty() {
            return self.actions.iter().collect();
        }
        self.actions
            .iter()
            .filter(|action| types.contains(&action.type_name()))
            .collect()
    }

    /// Render this request as the XML the server renders: the same
    /// elements, in the server's order, with the same attribute
    /// order. Empty comments and descriptions are dropped (an empty
    /// `<comment></comment>` reads back as the empty string, so
    /// writing it out would add nothing). The round-trip is
    /// structural — `parse_request(&request.to_xml())` reads back the
    /// same request — not byte-identical to a server document, which
    /// may spell attributes in another order.
    pub fn to_xml(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "<request creator=\"{}\" id=\"{}\">\n",
            xml_escape(&self.creator),
            self.id
        ));
        for action in &self.actions {
            out.push_str(&action_to_xml(action));
        }
        if let Some(priority) = self.priority {
            out.push_str(&format!("  <priority>{}</priority>\n", priority.as_str()));
        }
        out.push_str(&state_to_xml(&self.state));
        for review in &self.reviews {
            out.push_str(&review_to_xml(review));
        }
        for entry in &self.history {
            out.push_str(&history_to_xml(entry, 2));
        }
        if let Some(accept_at) = &self.accept_at {
            out.push_str(&format!(
                "  <accept_at>{}</accept_at>\n",
                xml_escape(accept_at)
            ));
        }
        if let Some(title) = &self.title {
            out.push_str(&format!("  <title>{}</title>\n", xml_escape(title)));
        }
        if !self.description.is_empty() {
            out.push_str(&format!(
                "  <description>{}</description>\n",
                xml_escape(&self.description)
            ));
        }
        out.push_str("</request>");
        out
    }
}

/// Parse a `<request>` document into a [`Request`]. The document
/// must be a whole request: an `id` and a `creator` on the root, at
/// least one action, exactly one `<state>`. Anything the parser does
/// not recognise — an unknown action type, an unknown child element,
/// a missing required attribute, a duplicated singleton — fails the
/// whole document (None), never a partial request. An old-style
/// `<request type="submit"><submit>…` document parses to the modern
/// action form, and re-serialises as `<action>`; the `type`
/// attribute on the root is dropped on the way out.
pub fn parse_request(xml: &str) -> Option<Request> {
    let xml = xml.trim();
    let root = parse_root(xml, "request")?;
    let id = attr_value(root.attrs, "id")?;
    let id = id.parse::<u64>().ok().filter(|id| *id > 0)?;
    let creator = attr_value(root.attrs, "creator")?;
    if creator.is_empty() {
        return None;
    }
    let mut actions = Vec::new();
    let mut has_action_element = false;
    let mut has_submit_element = false;
    let mut state: Option<RequestStateInfo> = None;
    let mut reviews = Vec::new();
    let mut history = Vec::new();
    let mut title: Option<String> = None;
    let mut description = String::new();
    let mut seen_description = false;
    let mut priority: Option<RequestPriority> = None;
    let mut accept_at: Option<String> = None;
    for child in child_elements(root.inner)? {
        match child.name {
            "action" => {
                has_action_element = true;
                actions.push(parse_action(&child)?);
            }
            "submit" => {
                has_submit_element = true;
                actions.push(parse_legacy_submit(&child)?);
            }
            "state" => {
                if state.is_some() {
                    return None;
                }
                state = Some(parse_state(&child)?);
            }
            "review" => reviews.push(parse_review(&child)?),
            "history" => history.push(parse_request_history(&child)?),
            "title" => {
                if title.is_some() {
                    return None;
                }
                title = Some(leaf_text(child.inner)?);
            }
            "description" => {
                if seen_description {
                    return None;
                }
                seen_description = true;
                description = leaf_text(child.inner)?;
            }
            "priority" => {
                if priority.is_some() {
                    return None;
                }
                priority = Some(RequestPriority::parse(&leaf_text(child.inner)?)?);
            }
            "accept_at" => {
                if accept_at.is_some() {
                    return None;
                }
                accept_at = Some(leaf_text(child.inner)?);
            }
            _ => return None,
        }
    }
    // The legacy form carries its submit action as `<submit>` and no
    // `<action>` at all; a document mixing the two shapes is neither,
    // so it fails rather than guessing which half to keep.
    if has_action_element && has_submit_element {
        return None;
    }
    if actions.is_empty() {
        return None;
    }
    Some(Request {
        id,
        creator: creator.to_string(),
        title,
        description,
        actions,
        priority,
        state: state?,
        reviews,
        history,
        accept_at,
    })
}

/// The root element of a document: its attribute text and inner XML.
/// The document must start with `<name` and end with `</name>` (after
/// trimming). A self-closing root reads as an empty document of this
/// shape. A leading XML declaration (`<?xml …?>`) is skipped: the
/// server does not send one on these responses, but a document that
/// carries one is the same document.
fn parse_root<'a>(xml: &'a str, name: &'a str) -> Option<Element<'a>> {
    let xml = xml.trim();
    let xml = match xml.strip_prefix("<?xml") {
        Some(rest) => {
            // The prolog must start here, exactly as in
            // `parse_devel_project`: `<?xmlfoo` is not a prolog, so it
            // is not stripped — the `<` handling below rejects `<?...`
            // as not an element.
            let first = rest.chars().next();
            if !matches!(first, Some(c) if c.is_whitespace() || c == '?') {
                xml
            } else {
                let end = rest.find("?>")?;
                rest[end + 2..].trim_start()
            }
        }
        None => xml,
    };
    let after_lt = xml.strip_prefix('<')?;
    if after_lt.starts_with('/') || after_lt.starts_with('!') || after_lt.starts_with('?') {
        return None;
    }
    if !after_lt.starts_with(name) {
        return None;
    }
    let after_name = &after_lt[name.len()..];
    match after_name.chars().next() {
        Some(c) if c.is_whitespace() || c == '>' => {}
        _ => return None,
    }
    let tag_end = after_name.find('>')?;
    let tag = &after_name[..tag_end];
    if let Some(attrs) = tag.trim_end().strip_suffix('/') {
        // A self-closing root has no children: it is an empty
        // document of this shape (an empty `<comments />` reads as an
        // empty list; an empty `<request />` fails later, on its
        // missing actions and state).
        return Some(Element {
            name,
            attrs: attrs.trim_end(),
            inner: "",
        });
    }
    let inner_start = &after_name[tag_end + 1..];
    let close_tag = format!("</{name}>");
    let close = inner_start.rfind(&close_tag)?;
    let inner = &inner_start[..close];
    let after_close = &inner_start[close + close_tag.len()..];
    if !after_close.trim().is_empty() {
        return None;
    }
    Some(Element {
        name,
        attrs: tag,
        inner,
    })
}

/// Parse one `<action>` element. The `type` attribute names the
/// variant; anything else fails. Each variant takes exactly the
/// children the server renders for it — a missing required child, a
/// duplicated singleton, or an unknown child fails the action.
fn parse_action(element: &Element<'_>) -> Option<RequestAction> {
    let action_type = attr_value(element.attrs, "type")?;
    match action_type {
        "submit" => parse_submit_action(element),
        "delete" => parse_delete_action(element),
        "add_role" => parse_add_role_action(element),
        "set_bugowner" => parse_set_bugowner_action(element),
        "change_devel" => parse_change_devel_action(element),
        "maintenance_incident" | "maintenance_release" | "release" => {
            parse_source_target_action(element, action_type)
        }
        _ => None,
    }
}

/// Parse the `<source>` child of an action element: the project and
/// package are required, the revision optional. Exactly one source;
/// a missing or duplicated one fails.
fn action_source(element: &Element<'_>) -> Option<(String, String, Option<String>)> {
    let mut source: Option<(String, String, Option<String>)> = None;
    for child in child_elements(element.inner)? {
        if child.name != "source" {
            continue;
        }
        if source.is_some() {
            return None;
        }
        let project = attr_value(child.attrs, "project")?;
        let package = attr_value(child.attrs, "package")?;
        if project.is_empty() || package.is_empty() {
            return None;
        }
        source = Some((
            project.to_string(),
            package.to_string(),
            attr_value(child.attrs, "rev").map(str::to_string),
        ));
    }
    source
}

/// A target reference: the project, the package (when the action
/// names one), and the repository (when it pins one).
type TargetRef = (String, Option<String>, Option<String>);

/// Parse the `<target>` child of an action element: the project is
/// required, the package and repository optional. At most one target;
/// a duplicated one fails. None when the action carries no target.
fn action_target(element: &Element<'_>) -> Option<Option<TargetRef>> {
    let mut target: Option<TargetRef> = None;
    for child in child_elements(element.inner)? {
        if child.name != "target" {
            continue;
        }
        if target.is_some() {
            return None;
        }
        let project = attr_value(child.attrs, "project")?;
        if project.is_empty() {
            return None;
        }
        target = Some((
            project.to_string(),
            attr_value(child.attrs, "package").map(str::to_string),
            attr_value(child.attrs, "repository").map(str::to_string),
        ));
    }
    Some(target)
}

/// Parse a `<submit>` action: a source, an optional target, optional
/// options, and an optional acceptinfo. Any other child fails.
fn parse_submit_action(element: &Element<'_>) -> Option<RequestAction> {
    let mut sourceupdate: Option<SourceUpdate> = None;
    let mut updatelink = false;
    let mut seen_updatelink = false;
    let mut makeoriginolder = false;
    let mut seen_makeoriginolder = false;
    let mut acceptinfo: Option<AcceptInfo> = None;
    let mut seen_options = false;
    let mut seen_acceptinfo = false;
    for child in child_elements(element.inner)? {
        match child.name {
            "source" | "target" => {}
            "options" => {
                if seen_options {
                    return None;
                }
                seen_options = true;
                for option in child_elements(child.inner)? {
                    match option.name {
                        "sourceupdate" => {
                            if sourceupdate.is_some() {
                                return None;
                            }
                            sourceupdate = Some(parse_sourceupdate(&leaf_text(option.inner)?)?);
                        }
                        "updatelink" => {
                            if seen_updatelink {
                                return None;
                            }
                            seen_updatelink = true;
                            updatelink = parse_option_bool(&leaf_text(option.inner)?)?;
                        }
                        "makeoriginolder" => {
                            if seen_makeoriginolder {
                                return None;
                            }
                            seen_makeoriginolder = true;
                            makeoriginolder = parse_option_bool(&leaf_text(option.inner)?)?;
                        }
                        _ => return None,
                    }
                }
            }
            "acceptinfo" => {
                if seen_acceptinfo {
                    return None;
                }
                seen_acceptinfo = true;
                if !child.inner.trim().is_empty() {
                    return None;
                }
                let rev = attr_value(child.attrs, "rev")?;
                let srcmd5 = attr_value(child.attrs, "srcmd5")?;
                if rev.is_empty() || srcmd5.is_empty() {
                    return None;
                }
                acceptinfo = Some(AcceptInfo {
                    rev: rev.to_string(),
                    srcmd5: srcmd5.to_string(),
                    oproject: attr_value(child.attrs, "oproject").map(str::to_string),
                    opackage: attr_value(child.attrs, "opackage").map(str::to_string),
                    osrcmd5: attr_value(child.attrs, "osrcmd5").map(str::to_string),
                    xsrcmd5: attr_value(child.attrs, "xsrcmd5").map(str::to_string),
                    oxsrcmd5: attr_value(child.attrs, "oxsrcmd5").map(str::to_string),
                });
            }
            _ => return None,
        }
    }
    let (source_project, source_package, source_rev) = action_source(element)?;
    let (target_project, target_package) = match action_target(element)? {
        Some((project, package, _)) => (Some(project), package),
        None => (None, None),
    };
    Some(RequestAction::Submit {
        source_project,
        source_package,
        source_rev,
        target_project,
        target_package,
        sourceupdate,
        updatelink,
        makeoriginolder,
        acceptinfo,
    })
}

/// Parse a `<sourceupdate>` value: exactly the three values the
/// server accepts, as [`SourceUpdate`]. Anything else fails.
fn parse_sourceupdate(value: &str) -> Option<SourceUpdate> {
    match value {
        "cleanup" => Some(SourceUpdate::Cleanup),
        "update" => Some(SourceUpdate::Update),
        "noupdate" => Some(SourceUpdate::NoUpdate),
        _ => None,
    }
}

/// Parse an `<updatelink>` / `<makeoriginolder>` value. The server
/// writes `true`; documents produced elsewhere may write `1`. Both
/// spellings read as true, `false` and `0` as false; anything else
/// fails.
fn parse_option_bool(value: &str) -> Option<bool> {
    match value {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// Parse a `<delete>` action: a target naming the project, and the
/// package when a package (rather than the whole project) goes.
fn parse_delete_action(element: &Element<'_>) -> Option<RequestAction> {
    for child in child_elements(element.inner)? {
        if child.name != "target" {
            return None;
        }
    }
    let (target_project, target_package, target_repository) = action_target(element)??;
    Some(RequestAction::Delete {
        target_project,
        target_package,
        target_repository,
    })
}

/// Parse an `<add_role>` action: a target, and a person and/or a
/// group with the role each is granted. At least one of person and
/// group is required.
fn parse_add_role_action(element: &Element<'_>) -> Option<RequestAction> {
    let mut person: Option<(String, String)> = None;
    let mut group: Option<(String, String)> = None;
    for child in child_elements(element.inner)? {
        match child.name {
            "target" => {}
            "person" => {
                if person.is_some() {
                    return None;
                }
                let name = attr_value(child.attrs, "name")?;
                let role = attr_value(child.attrs, "role")?;
                if name.is_empty() || role.is_empty() {
                    return None;
                }
                person = Some((name.to_string(), role.to_string()));
            }
            "group" => {
                if group.is_some() {
                    return None;
                }
                let name = attr_value(child.attrs, "name")?;
                let role = attr_value(child.attrs, "role")?;
                if name.is_empty() || role.is_empty() {
                    return None;
                }
                group = Some((name.to_string(), role.to_string()));
            }
            _ => return None,
        }
    }
    if person.is_none() && group.is_none() {
        return None;
    }
    let (target_project, target_package, _) = action_target(element)??;
    Some(RequestAction::AddRole {
        target_project,
        target_package,
        person,
        group,
    })
}

/// Parse a `<set_bugowner>` action: a target, and a person and/or a
/// group set as bugowner. The person and group carry a name only, no
/// role. At least one of person and group is required.
fn parse_set_bugowner_action(element: &Element<'_>) -> Option<RequestAction> {
    let mut person_name: Option<String> = None;
    let mut group_name: Option<String> = None;
    for child in child_elements(element.inner)? {
        match child.name {
            "target" => {}
            "person" => {
                if person_name.is_some() {
                    return None;
                }
                let name = attr_value(child.attrs, "name")?;
                if name.is_empty() {
                    return None;
                }
                person_name = Some(name.to_string());
            }
            "group" => {
                if group_name.is_some() {
                    return None;
                }
                let name = attr_value(child.attrs, "name")?;
                if name.is_empty() {
                    return None;
                }
                group_name = Some(name.to_string());
            }
            _ => return None,
        }
    }
    if person_name.is_none() && group_name.is_none() {
        return None;
    }
    let (target_project, target_package, _) = action_target(element)??;
    Some(RequestAction::SetBugowner {
        target_project,
        target_package,
        person_name,
        group_name,
    })
}

/// Parse a `<change_devel>` action: a source and a target, both
/// naming a project and a package.
fn parse_change_devel_action(element: &Element<'_>) -> Option<RequestAction> {
    for child in child_elements(element.inner)? {
        if child.name != "source" && child.name != "target" {
            return None;
        }
    }
    let (source_project, source_package, _) = action_source(element)?;
    let (target_project, target_package, _) = action_target(element)??;
    let target_package = target_package?;
    Some(RequestAction::ChangeDevel {
        source_project,
        source_package,
        target_project,
        target_package,
    })
}

/// Parse a maintenance or release action: a source and a target,
/// both naming a project; the target package is optional.
fn parse_source_target_action(element: &Element<'_>, action_type: &str) -> Option<RequestAction> {
    for child in child_elements(element.inner)? {
        if child.name != "source" && child.name != "target" {
            return None;
        }
    }
    let (source_project, source_package, _) = action_source(element)?;
    let (target_project, target_package, _) = action_target(element)??;
    match action_type {
        "maintenance_incident" => Some(RequestAction::MaintenanceIncident {
            source_project,
            source_package,
            target_project,
            target_package,
        }),
        "maintenance_release" => Some(RequestAction::MaintenanceRelease {
            source_project,
            source_package,
            target_project,
            target_package,
        }),
        "release" => Some(RequestAction::Release {
            source_project,
            source_package,
            target_project,
            target_package,
        }),
        _ => None,
    }
}

/// Parse an old-style `<submit>` element (the legacy request form):
/// a source and a target, no options. The caller lifts it into a
/// modern submit action.
fn parse_legacy_submit(element: &Element<'_>) -> Option<RequestAction> {
    for child in child_elements(element.inner)? {
        if child.name != "source" && child.name != "target" {
            return None;
        }
    }
    let (source_project, source_package, source_rev) = action_source(element)?;
    let (target_project, target_package) = match action_target(element)? {
        Some((project, package, _)) => (Some(project), package),
        None => (None, None),
    };
    Some(RequestAction::Submit {
        source_project,
        source_package,
        source_rev,
        target_project,
        target_package,
        sourceupdate: None,
        updatelink: false,
        makeoriginolder: false,
        acceptinfo: None,
    })
}

/// Parse a `<state>` element: the name, who, and when are required;
/// `created`, `superseded_by`, and `approver` are optional. The
/// comment child is optional and reads as empty when absent.
fn parse_state(element: &Element<'_>) -> Option<RequestStateInfo> {
    let name = RequestState::parse(attr_value(element.attrs, "name")?)?;
    let who = attr_value(element.attrs, "who")?;
    let when = attr_value(element.attrs, "when")?;
    if who.is_empty() || when.is_empty() {
        return None;
    }
    let created = attr_value(element.attrs, "created").map(str::to_string);
    let superseded_by = match attr_value(element.attrs, "superseded_by") {
        Some(value) => Some(value.parse::<u64>().ok().filter(|id| *id > 0)?),
        None => None,
    };
    let approver = attr_value(element.attrs, "approver").map(str::to_string);
    let mut comment = String::new();
    let mut seen_comment = false;
    for child in child_elements(element.inner)? {
        if child.name != "comment" {
            return None;
        }
        if seen_comment {
            return None;
        }
        seen_comment = true;
        comment = leaf_text(child.inner)?;
    }
    Some(RequestStateInfo {
        name,
        who: who.to_string(),
        when: when.to_string(),
        created,
        superseded_by,
        approver,
        comment,
    })
}

/// Parse a `<review>` element: the state is required; when, who, and
/// the `by_*` fields are optional. The comment child is optional and
/// reads as empty when absent. A review that was acted on carries
/// one `<history>` child per change, each with a description of what
/// happened; they read like request history entries.
fn parse_review(element: &Element<'_>) -> Option<RequestReview> {
    let state = ReviewState::parse(attr_value(element.attrs, "state")?)?;
    let when = attr_value(element.attrs, "when").map(str::to_string);
    let who = attr_value(element.attrs, "who").map(str::to_string);
    let by_user = attr_value(element.attrs, "by_user").map(str::to_string);
    let by_group = attr_value(element.attrs, "by_group").map(str::to_string);
    let by_project = attr_value(element.attrs, "by_project").map(str::to_string);
    let by_package = attr_value(element.attrs, "by_package").map(str::to_string);
    let mut comment = String::new();
    let mut seen_comment = false;
    let mut history = Vec::new();
    for child in child_elements(element.inner)? {
        match child.name {
            "comment" => {
                if seen_comment {
                    return None;
                }
                seen_comment = true;
                comment = leaf_text(child.inner)?;
            }
            // A review that was acted on carries one `<history>`
            // child per change (accepted, declined, assigned,
            // reopened, obsoleted), each with a description of what
            // happened — never empty, and never a placeholder. The
            // entries read like request history entries.
            "history" => history.push(parse_request_history(&child)?),
            _ => return None,
        }
    }
    Some(RequestReview {
        state,
        when,
        who,
        by_user,
        by_group,
        by_project,
        by_package,
        comment,
        history,
    })
}

/// Parse a `<history>` element: who and when are required; the
/// description and comment children are optional and read as empty
/// when absent.
fn parse_request_history(element: &Element<'_>) -> Option<RequestHistory> {
    let who = attr_value(element.attrs, "who")?;
    let when = attr_value(element.attrs, "when")?;
    if who.is_empty() || when.is_empty() {
        return None;
    }
    let mut description = String::new();
    let mut comment = String::new();
    let mut seen_description = false;
    let mut seen_comment = false;
    for child in child_elements(element.inner)? {
        match child.name {
            "description" => {
                if seen_description {
                    return None;
                }
                seen_description = true;
                description = leaf_text(child.inner)?;
            }
            "comment" => {
                if seen_comment {
                    return None;
                }
                seen_comment = true;
                comment = leaf_text(child.inner)?;
            }
            _ => return None,
        }
    }
    Some(RequestHistory {
        who: who.to_string(),
        when: when.to_string(),
        description,
        comment,
    })
}

/// Render one action as XML, in the server's shape: the same
/// elements, in the server's order, with the same attribute order
/// (`project` before `package`, `rev` last on a source).
fn action_to_xml(action: &RequestAction) -> String {
    let mut out = String::new();
    match action {
        RequestAction::Submit {
            source_project,
            source_package,
            source_rev,
            target_project,
            target_package,
            sourceupdate,
            updatelink,
            makeoriginolder,
            acceptinfo,
        } => {
            out.push_str("  <action type=\"submit\">\n");
            out.push_str(&source_to_xml(
                source_project,
                source_package,
                source_rev.as_deref(),
            ));
            if let Some(target_project) = target_project {
                out.push_str(&target_to_xml(
                    target_project,
                    target_package.as_deref(),
                    None,
                ));
            }
            if sourceupdate.is_some() || *updatelink || *makeoriginolder {
                out.push_str("    <options>\n");
                if let Some(update) = sourceupdate {
                    out.push_str(&format!(
                        "      <sourceupdate>{}</sourceupdate>\n",
                        update.as_str()
                    ));
                }
                if *updatelink {
                    out.push_str("      <updatelink>true</updatelink>\n");
                }
                if *makeoriginolder {
                    out.push_str("      <makeoriginolder>true</makeoriginolder>\n");
                }
                out.push_str("    </options>\n");
            }
            if let Some(info) = acceptinfo {
                out.push_str(&format!(
                    "    <acceptinfo rev=\"{}\" srcmd5=\"{}\"",
                    xml_escape(&info.rev),
                    xml_escape(&info.srcmd5)
                ));
                for (name, value) in [
                    ("oproject", &info.oproject),
                    ("opackage", &info.opackage),
                    ("osrcmd5", &info.osrcmd5),
                    ("xsrcmd5", &info.xsrcmd5),
                    ("oxsrcmd5", &info.oxsrcmd5),
                ] {
                    if let Some(value) = value {
                        out.push_str(&format!(" {name}=\"{}\"", xml_escape(value)));
                    }
                }
                out.push_str(" />\n");
            }
            out.push_str("  </action>\n");
        }
        RequestAction::Delete {
            target_project,
            target_package,
            target_repository,
        } => {
            out.push_str("  <action type=\"delete\">\n");
            out.push_str(&target_to_xml(
                target_project,
                target_package.as_deref(),
                target_repository.as_deref(),
            ));
            out.push_str("  </action>\n");
        }
        RequestAction::AddRole {
            target_project,
            target_package,
            person,
            group,
        } => {
            out.push_str("  <action type=\"add_role\">\n");
            out.push_str(&target_to_xml(
                target_project,
                target_package.as_deref(),
                None,
            ));
            if let Some((name, role)) = person {
                out.push_str(&format!(
                    "    <person name=\"{}\" role=\"{}\" />\n",
                    xml_escape(name),
                    xml_escape(role)
                ));
            }
            if let Some((name, role)) = group {
                out.push_str(&format!(
                    "    <group name=\"{}\" role=\"{}\" />\n",
                    xml_escape(name),
                    xml_escape(role)
                ));
            }
            out.push_str("  </action>\n");
        }
        RequestAction::SetBugowner {
            target_project,
            target_package,
            person_name,
            group_name,
        } => {
            out.push_str("  <action type=\"set_bugowner\">\n");
            out.push_str(&target_to_xml(
                target_project,
                target_package.as_deref(),
                None,
            ));
            if let Some(name) = person_name {
                out.push_str(&format!("    <person name=\"{}\" />\n", xml_escape(name)));
            }
            if let Some(name) = group_name {
                out.push_str(&format!("    <group name=\"{}\" />\n", xml_escape(name)));
            }
            out.push_str("  </action>\n");
        }
        RequestAction::ChangeDevel {
            source_project,
            source_package,
            target_project,
            target_package,
        } => {
            out.push_str("  <action type=\"change_devel\">\n");
            out.push_str(&source_to_xml(source_project, source_package, None));
            out.push_str(&target_to_xml(target_project, Some(target_package), None));
            out.push_str("  </action>\n");
        }
        RequestAction::MaintenanceIncident {
            source_project,
            source_package,
            target_project,
            target_package,
        }
        | RequestAction::MaintenanceRelease {
            source_project,
            source_package,
            target_project,
            target_package,
        }
        | RequestAction::Release {
            source_project,
            source_package,
            target_project,
            target_package,
        } => {
            out.push_str(&format!("  <action type=\"{}\">\n", action.type_name()));
            out.push_str(&source_to_xml(source_project, source_package, None));
            out.push_str(&target_to_xml(
                target_project,
                target_package.as_deref(),
                None,
            ));
            out.push_str("  </action>\n");
        }
    }
    out
}

/// Render a `<source>` element: `project`, then `package`, then
/// `rev` when one is pinned.
fn source_to_xml(project: &str, package: &str, rev: Option<&str>) -> String {
    let mut out = format!(
        "    <source project=\"{}\" package=\"{}\"",
        xml_escape(project),
        xml_escape(package)
    );
    if let Some(rev) = rev {
        out.push_str(&format!(" rev=\"{}\"", xml_escape(rev)));
    }
    out.push_str(" />\n");
    out
}

/// Render a `<target>` element: `project`, then `package`, then
/// `repository`, each only when present.
fn target_to_xml(project: &str, package: Option<&str>, repository: Option<&str>) -> String {
    let mut out = format!("    <target project=\"{}\"", xml_escape(project));
    if let Some(package) = package {
        out.push_str(&format!(" package=\"{}\"", xml_escape(package)));
    }
    if let Some(repository) = repository {
        out.push_str(&format!(" repository=\"{}\"", xml_escape(repository)));
    }
    out.push_str(" />\n");
    out
}

/// Render a `<state>` element: `name`, `who`, `when`, then `created`,
/// `superseded_by`, and `approver` when present. The comment child is
/// written only when non-empty; an empty comment reads back as the
/// empty string, so writing it out would add nothing.
fn state_to_xml(state: &RequestStateInfo) -> String {
    let mut out = format!(
        "  <state name=\"{}\" who=\"{}\" when=\"{}\"",
        state.name.as_str(),
        xml_escape(&state.who),
        xml_escape(&state.when)
    );
    if let Some(created) = &state.created {
        out.push_str(&format!(" created=\"{}\"", xml_escape(created)));
    }
    if let Some(superseded_by) = state.superseded_by {
        out.push_str(&format!(" superseded_by=\"{superseded_by}\""));
    }
    if let Some(approver) = &state.approver {
        out.push_str(&format!(" approver=\"{}\"", xml_escape(approver)));
    }
    if state.comment.is_empty() {
        out.push_str(" />\n");
    } else {
        out.push_str(&format!(
            ">\n    <comment>{}</comment>\n  </state>\n",
            xml_escape(&state.comment)
        ));
    }
    out
}

/// Render a `<review>` element: `state`, `when`, `who`, then the
/// `by_*` fields when present. The comment child is written only when
/// non-empty; the review's history entries follow it, in order.
fn review_to_xml(review: &RequestReview) -> String {
    let mut out = format!("  <review state=\"{}\"", review.state.as_str());
    if let Some(when) = &review.when {
        out.push_str(&format!(" when=\"{}\"", xml_escape(when)));
    }
    if let Some(who) = &review.who {
        out.push_str(&format!(" who=\"{}\"", xml_escape(who)));
    }
    if let Some(by_group) = &review.by_group {
        out.push_str(&format!(" by_group=\"{}\"", xml_escape(by_group)));
    }
    if let Some(by_user) = &review.by_user {
        out.push_str(&format!(" by_user=\"{}\"", xml_escape(by_user)));
    }
    if let Some(by_package) = &review.by_package {
        out.push_str(&format!(" by_package=\"{}\"", xml_escape(by_package)));
    }
    if let Some(by_project) = &review.by_project {
        out.push_str(&format!(" by_project=\"{}\"", xml_escape(by_project)));
    }
    out.push_str(">\n");
    if !review.comment.is_empty() {
        out.push_str(&format!(
            "    <comment>{}</comment>\n",
            xml_escape(&review.comment)
        ));
    }
    for entry in &review.history {
        out.push_str(&history_to_xml(entry, 4));
    }
    out.push_str("  </review>\n");
    out
}

/// Render a `<history>` element at `indent` spaces: `who`, then
/// `when`. The description and comment children are written only
/// when non-empty, indented two further spaces.
fn history_to_xml(entry: &RequestHistory, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let child_pad = " ".repeat(indent + 2);
    if entry.description.is_empty() && entry.comment.is_empty() {
        return format!(
            "{pad}<history who=\"{}\" when=\"{}\" />\n",
            xml_escape(&entry.who),
            xml_escape(&entry.when)
        );
    }
    let mut out = format!(
        "{pad}<history who=\"{}\" when=\"{}\">\n",
        xml_escape(&entry.who),
        xml_escape(&entry.when)
    );
    if !entry.description.is_empty() {
        out.push_str(&format!(
            "{child_pad}<description>{}</description>\n",
            xml_escape(&entry.description)
        ));
    }
    if !entry.comment.is_empty() {
        out.push_str(&format!(
            "{child_pad}<comment>{}</comment>\n",
            xml_escape(&entry.comment)
        ));
    }
    out.push_str(&format!("{pad}</history>\n"));
    out
}

/// Parse a `<collection>` of whole `<request>` elements — the shape
/// the request search returns — into the requests it carries. Each
/// element is parsed with [`parse_request`]; any element that fails
/// fails the whole collection, since a search result the library
/// cannot read in full is not a result it can act on. The number of
/// elements parsed must equal the `matches` count: the server returns
/// every match in one response (over the limit it refuses with 403
/// rather than truncating), so a short collection is a server shape
/// change, not a partial result.
pub fn parse_request_collection(body: &str) -> Option<Vec<Request>> {
    if !body.contains("<collection") {
        return None;
    }
    let matches = parse_search_matches(body)?;
    let mut requests = Vec::new();
    let mut parsed = 0usize;
    let mut rest = body;
    while let Some(pos) = rest.find("<request") {
        let after_tag = &rest[pos + "<request".len()..];
        let first = after_tag.chars().next()?;
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let end = after_tag.find('>')?;
        let tag = &after_tag[..end];
        parsed += 1;
        if tag.trim_end().ends_with('/') {
            // Self-closing: no children, so not a whole request.
            return None;
        }
        let inner = &after_tag[end + 1..];
        let close = inner.find("</request>")?;
        let block = &inner[..close];
        let element = format!("<request{tag}>{block}</request>");
        requests.push(parse_request(&element)?);
        rest = &inner[close + "</request>".len()..];
    }
    if parsed != matches as usize {
        return None;
    }
    Some(requests)
}

/// One comment on a request: who left it, when, what it says, and
/// which comment it replies to, when it is a reply. Comments on a
/// request and comments on its actions are listed together, in id
/// order — the server sorts them, and a threaded reply carries its
/// parent's id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestComment {
    /// The comment's id, for threading replies under it.
    pub id: u64,
    /// Who left the comment.
    pub who: String,
    /// When the comment was left.
    pub when: String,
    /// The comment this one replies to, when it is a reply.
    pub parent_id: Option<u64>,
    /// The comment's text.
    pub body: String,
}

/// Parse a `<comments>` document — the shape the comment list returns
/// — into the comments it carries, in document order. An empty
/// document reads as an empty list: a request with no comments is a
/// normal request. Anything the parser does not recognise — an
/// unknown child element, a missing required attribute — fails the
/// whole document.
pub fn parse_comments(body: &str) -> Option<Vec<RequestComment>> {
    if !body.contains("<comments") {
        return None;
    }
    let root = parse_root(body.trim(), "comments")?;
    let mut comments = Vec::new();
    for child in child_elements(root.inner)? {
        if child.name != "comment" {
            return None;
        }
        let id = attr_value(child.attrs, "id")?;
        let id = id.parse::<u64>().ok().filter(|id| *id > 0)?;
        let who = attr_value(child.attrs, "who")?;
        let when = attr_value(child.attrs, "when")?;
        if who.is_empty() || when.is_empty() {
            return None;
        }
        let parent_id = match attr_value(child.attrs, "parent") {
            Some(value) => Some(value.parse::<u64>().ok().filter(|id| *id > 0)?),
            None => None,
        };
        let body = leaf_text(child.inner)?;
        comments.push(RequestComment {
            id,
            who: who.to_string(),
            when: when.to_string(),
            parent_id,
            body,
        });
    }
    Some(comments)
}

/// Who a review is asked of: a user, a group, a project, or a
/// project/package pair. At least one of user, group, and project is
/// required — the server refuses a review asked of nobody — and a
/// package review needs its project, since a package name alone
/// names nothing. Built through [`ReviewTarget::new`], so an invalid
/// target fails before any request is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewTarget {
    /// The user the review is asked of, when it names one.
    pub by_user: Option<String>,
    /// The group the review is asked of, when it names one.
    pub by_group: Option<String>,
    /// The project the review is asked of, when it names one.
    pub by_project: Option<String>,
    /// The package the review is asked of, when it names one.
    pub by_package: Option<String>,
}

impl ReviewTarget {
    /// Build a review target from its parts. Each part is optional;
    /// an empty string for a part is refused rather than silently
    /// dropped, and a project or package name must be a valid OBS
    /// name.
    pub fn new(
        by_user: Option<&str>,
        by_group: Option<&str>,
        by_project: Option<&str>,
        by_package: Option<&str>,
    ) -> Result<Self, GeectlError> {
        let by_user = validate_review_party("user", by_user)?;
        let by_group = validate_review_party("group", by_group)?;
        let by_project = match by_project {
            Some("") => {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "a review project name must not be empty",
                ));
            }
            Some(value) if !valid_obs_name(value) => {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS project name: {value:?}"),
                ));
            }
            Some(value) => Some(value.to_string()),
            None => None,
        };
        let by_package = match by_package {
            Some("") => {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "a review package name must not be empty",
                ));
            }
            Some(value) if !valid_obs_name(value) => {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS package name: {value:?}"),
                ));
            }
            Some(value) => Some(value.to_string()),
            None => None,
        };
        if by_user.is_none() && by_group.is_none() && by_project.is_none() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "a review must be asked of a user, a group, or a project",
            ));
        }
        if by_package.is_some() && by_project.is_none() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "a review asked of a package needs its project",
            ));
        }
        Ok(ReviewTarget {
            by_user,
            by_group,
            by_project,
            by_package,
        })
    }
}

/// Validate one review party (a user or a group login): absent is
/// fine, present must be non-empty.
fn validate_review_party(label: &str, value: Option<&str>) -> Result<Option<String>, GeectlError> {
    match value {
        Some("") => Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("a review {label} name must not be empty"),
        )),
        Some(value) => Ok(Some(value.to_string())),
        None => Ok(None),
    }
}

impl ReviewTarget {
    /// The target as a short phrase for logs and dry-run plans, e.g.
    /// `user Admin`, `group reviewers`, `project openSUSE:Factory`,
    /// `project openSUSE:Factory/package foo`. The parts present are
    /// joined with `, `.
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(by_user) = &self.by_user {
            parts.push(format!("user {by_user}"));
        }
        if let Some(by_group) = &self.by_group {
            parts.push(format!("group {by_group}"));
        }
        if let Some(by_project) = &self.by_project {
            match &self.by_package {
                Some(by_package) => {
                    parts.push(format!("project {by_project}/package {by_package}"));
                }
                None => parts.push(format!("project {by_project}")),
            }
        }
        parts.join(", ")
    }
}

/// One XPath predicate over the request XML, for the `match=`
/// parameter of the request search. The tree carries the precedence:
/// `and` binds tighter than `or`, and rendering parenthesises a child
/// whose operator differs from its parent's, so the string reads back
/// with the same grouping the tree has. A leaf is a single comparison
/// (`@name='foo'`, `contains(@name, 'foo')`); `Pred` is an element
/// predicate (`state[@name='new']`) — the brackets scope the inner
/// query, so it never needs parens of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XpathQuery {
    /// No predicate; renders as the empty string.
    Empty,
    /// `path='value'`.
    Eq(String, String),
    /// `not(inner)`.
    Not(Box<XpathQuery>),
    /// `contains(path, 'value')`.
    Contains(String, String),
    /// `left and right`.
    And(Box<XpathQuery>, Box<XpathQuery>),
    /// `left or right`.
    Or(Box<XpathQuery>, Box<XpathQuery>),
    /// `element[inner]`.
    Pred(String, Box<XpathQuery>),
}

impl XpathQuery {
    /// The `and` of several queries, left-associated, skipping empty
    /// ones. No queries reads as the empty query.
    pub fn and_all(queries: Vec<XpathQuery>) -> XpathQuery {
        queries
            .into_iter()
            .filter(|query| *query != XpathQuery::Empty)
            .reduce(|left, right| XpathQuery::And(Box::new(left), Box::new(right)))
            .unwrap_or(XpathQuery::Empty)
    }

    /// The `or` of several queries, left-associated, skipping empty
    /// ones. No queries reads as the empty query.
    pub fn or_all(queries: Vec<XpathQuery>) -> XpathQuery {
        queries
            .into_iter()
            .filter(|query| *query != XpathQuery::Empty)
            .reduce(|left, right| XpathQuery::Or(Box::new(left), Box::new(right)))
            .unwrap_or(XpathQuery::Empty)
    }

    /// `path='value1' or path='value2' …` for a list of values. An
    /// empty list reads as the empty query.
    pub fn eq_list(path: &str, values: &[String]) -> XpathQuery {
        XpathQuery::or_all(
            values
                .iter()
                .map(|value| XpathQuery::Eq(path.to_string(), value.clone()))
                .collect(),
        )
    }
}

impl fmt::Display for XpathQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XpathQuery::Empty => Ok(()),
            XpathQuery::Eq(path, value) => write!(f, "{path}='{value}'"),
            XpathQuery::Not(inner) => write!(f, "not({inner})"),
            XpathQuery::Contains(path, value) => write!(f, "contains({path}, '{value}')"),
            XpathQuery::And(left, right) => {
                let rendered_left = render_xpath_child(left, true);
                let rendered_right = render_xpath_child(right, true);
                write!(f, "{rendered_left} and {rendered_right}")
            }
            XpathQuery::Or(left, right) => {
                let rendered_left = render_xpath_child(left, false);
                let rendered_right = render_xpath_child(right, false);
                write!(f, "{rendered_left} or {rendered_right}")
            }
            XpathQuery::Pred(element, inner) => write!(f, "{element}[{inner}]"),
        }
    }
}

/// Render a child of an `and` (`is_and`) or `or` node: a child whose
/// operator differs from its parent's is parenthesised, so the
/// precedence the tree carries survives the flat string; a child with
/// the same operator, or a leaf, renders bare. An `and` of two `and`s
/// reads flat (`a and b and c`); an `or` under an `and` reads
/// parenthesised (`(a or b) and c`).
fn render_xpath_child(child: &XpathQuery, is_and: bool) -> String {
    let differs = match child {
        XpathQuery::And(..) => !is_and,
        XpathQuery::Or(..) => is_and,
        _ => false,
    };
    if differs {
        format!("({child})")
    } else {
        format!("{child}")
    }
}

/// The action types the request search can filter by: exactly the
/// types [`RequestAction`] parses. An unknown type in a search would
/// match nothing on the server, so the builder refuses it instead.
const KNOWN_ACTION_TYPES: &[&str] = &[
    "submit",
    "delete",
    "add_role",
    "set_bugowner",
    "change_devel",
    "maintenance_incident",
    "maintenance_release",
    "release",
];

/// A typed request search: the filters for `GET /search/request`,
/// built into the `match=` expression the server evaluates. Every
/// filter is a list, and a list matches when any of its values
/// matches; the filters themselves are `and`-joined. The clause order
/// is fixed (ids, projects, packages, source, target, creators, who,
/// states, types, exclusions), so the same search always builds the
/// same expression.
///
/// The states default to the open states — `new`, `review`,
/// `declined` — the set a tool triaging open work wants. Pass an
/// explicit (possibly empty) state list to override: an empty list
/// means no state filter at all.
///
/// The `source_packages` / `target_packages` filters use the
/// singular attribute (`source[@package='…']`): the search engine's
/// whitelist has `source/@package` and `target/@package` and no
/// plural form, so the plural is refused by the server with 400.
///
/// History: `with_history` adds the creation entry and the request
/// history to each result; `with_full_history` adds all history
/// elements. When both are set the server prefers the full history.
#[derive(Debug, Clone, Default)]
pub struct RequestSearch {
    /// Match requests with one of these numbers.
    request_ids: Vec<u64>,
    /// Match requests whose source or target project is one of these.
    projects: Vec<String>,
    /// Match requests whose source or target package is one of these.
    packages: Vec<String>,
    /// Match requests whose source project is one of these.
    source_projects: Vec<String>,
    /// Match requests whose source package is one of these.
    source_packages: Vec<String>,
    /// Match requests whose target project is one of these.
    target_projects: Vec<String>,
    /// Match requests whose target package is one of these.
    target_packages: Vec<String>,
    /// Match requests created by one of these users.
    creators: Vec<String>,
    /// Match requests one of these users interacted with (as the last
    /// actor on the state, or in the history).
    who: Vec<String>,
    /// Match requests in one of these states. `None` is the default
    /// open states; `Some(vec![])` is no state filter.
    states: Option<Vec<RequestState>>,
    /// Match requests with an action of one of these types.
    types: Vec<String>,
    /// Exclude requests targeting one of these projects.
    exclude_target_projects: Vec<String>,
    /// Include the creation entry and the request history in each
    /// result.
    with_history: bool,
    /// Include all history elements in each result.
    with_full_history: bool,
}

impl RequestSearch {
    /// An empty search: the default open states, no other filter, no
    /// history.
    pub fn new() -> Self {
        RequestSearch::default()
    }

    /// Match requests with one of these numbers.
    pub fn request_ids(mut self, request_ids: Vec<u64>) -> Self {
        self.request_ids = request_ids;
        self
    }

    /// Match requests whose source or target project is one of these.
    pub fn projects(mut self, projects: Vec<String>) -> Self {
        self.projects = projects;
        self
    }

    /// Match requests whose source or target package is one of these.
    pub fn packages(mut self, packages: Vec<String>) -> Self {
        self.packages = packages;
        self
    }

    /// Match requests whose source project is one of these.
    pub fn source_projects(mut self, source_projects: Vec<String>) -> Self {
        self.source_projects = source_projects;
        self
    }

    /// Match requests whose source package is one of these.
    pub fn source_packages(mut self, source_packages: Vec<String>) -> Self {
        self.source_packages = source_packages;
        self
    }

    /// Match requests whose target project is one of these.
    pub fn target_projects(mut self, target_projects: Vec<String>) -> Self {
        self.target_projects = target_projects;
        self
    }

    /// Match requests whose target package is one of these.
    pub fn target_packages(mut self, target_packages: Vec<String>) -> Self {
        self.target_packages = target_packages;
        self
    }

    /// Match requests created by one of these users.
    pub fn creators(mut self, creators: Vec<String>) -> Self {
        self.creators = creators;
        self
    }

    /// Match requests one of these users interacted with.
    pub fn who(mut self, who: Vec<String>) -> Self {
        self.who = who;
        self
    }

    /// Match requests in one of these states. An empty list means no
    /// state filter at all.
    pub fn states(mut self, states: Vec<RequestState>) -> Self {
        self.states = Some(states);
        self
    }

    /// Match requests with an action of one of these types.
    pub fn types(mut self, types: Vec<String>) -> Self {
        self.types = types;
        self
    }

    /// Exclude requests targeting one of these projects.
    pub fn exclude_target_projects(mut self, exclude_target_projects: Vec<String>) -> Self {
        self.exclude_target_projects = exclude_target_projects;
        self
    }

    /// Include the creation entry and the request history in each
    /// result.
    pub fn with_history(mut self, with_history: bool) -> Self {
        self.with_history = with_history;
        self
    }

    /// Include all history elements in each result.
    pub fn with_full_history(mut self, with_full_history: bool) -> Self {
        self.with_full_history = with_full_history;
        self
    }

    /// Whether the search asks for the request history.
    pub fn wants_history(&self) -> bool {
        self.with_history
    }

    /// Whether the search asks for the full history.
    pub fn wants_full_history(&self) -> bool {
        self.with_full_history
    }

    /// Build the `match=` expression for this search. Every value is
    /// validated first: project and package names must be valid OBS
    /// names, user names must be non-empty and quote-free, and action
    /// types must be ones the server renders. A value that would
    /// break out of its XPath string literal is refused, because
    /// XPath has no string escape and a quote in a value would
    /// rewrite the query. A search with no filter at all — no
    /// clause, not even a state — is refused too: the server answers
    /// an empty `match=` with 400 (`empty_match`), so there is no
    /// unfiltered search.
    pub fn match_expr(&self) -> Result<String, GeectlError> {
        for (label, values) in [
            ("project", &self.projects),
            ("package", &self.packages),
            ("source project", &self.source_projects),
            ("source package", &self.source_packages),
            ("target project", &self.target_projects),
            ("target package", &self.target_packages),
            ("excluded target project", &self.exclude_target_projects),
        ] {
            for value in values {
                if !valid_obs_name(value) {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!("invalid OBS {label} name: {value:?}"),
                    ));
                }
            }
        }
        for (label, values) in [("creator", &self.creators), ("user", &self.who)] {
            for value in values {
                validate_xpath_value(label, value)?;
            }
        }
        for action_type in &self.types {
            if !KNOWN_ACTION_TYPES.contains(&action_type.as_str()) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "unknown request action type: {action_type:?} (known: {})",
                        KNOWN_ACTION_TYPES.join(", ")
                    ),
                ));
            }
        }
        let mut clauses: Vec<XpathQuery> = Vec::new();
        if !self.request_ids.is_empty() {
            let ids: Vec<String> = self.request_ids.iter().map(|id| id.to_string()).collect();
            clauses.push(XpathQuery::eq_list("@id", &ids));
        }
        if !self.projects.is_empty() {
            clauses.push(XpathQuery::Or(
                Box::new(XpathQuery::Pred(
                    "source".to_string(),
                    Box::new(XpathQuery::eq_list("@project", &self.projects)),
                )),
                Box::new(XpathQuery::Pred(
                    "target".to_string(),
                    Box::new(XpathQuery::eq_list("@project", &self.projects)),
                )),
            ));
        }
        if !self.packages.is_empty() {
            clauses.push(XpathQuery::Or(
                Box::new(XpathQuery::Pred(
                    "source".to_string(),
                    Box::new(XpathQuery::eq_list("@package", &self.packages)),
                )),
                Box::new(XpathQuery::Pred(
                    "target".to_string(),
                    Box::new(XpathQuery::eq_list("@package", &self.packages)),
                )),
            ));
        }
        if !self.source_projects.is_empty() {
            clauses.push(XpathQuery::Pred(
                "source".to_string(),
                Box::new(XpathQuery::eq_list("@project", &self.source_projects)),
            ));
        }
        if !self.source_packages.is_empty() {
            clauses.push(XpathQuery::Pred(
                "source".to_string(),
                Box::new(XpathQuery::eq_list("@package", &self.source_packages)),
            ));
        }
        if !self.target_projects.is_empty() {
            clauses.push(XpathQuery::Pred(
                "target".to_string(),
                Box::new(XpathQuery::eq_list("@project", &self.target_projects)),
            ));
        }
        if !self.target_packages.is_empty() {
            clauses.push(XpathQuery::Pred(
                "target".to_string(),
                Box::new(XpathQuery::eq_list("@package", &self.target_packages)),
            ));
        }
        if !self.creators.is_empty() {
            clauses.push(XpathQuery::eq_list("@creator", &self.creators));
        }
        if !self.who.is_empty() {
            clauses.push(XpathQuery::Or(
                Box::new(XpathQuery::Pred(
                    "state".to_string(),
                    Box::new(XpathQuery::eq_list("@who", &self.who)),
                )),
                Box::new(XpathQuery::Pred(
                    "history".to_string(),
                    Box::new(XpathQuery::eq_list("@who", &self.who)),
                )),
            ));
        }
        let states = match &self.states {
            Some(states) => states.clone(),
            None => vec![
                RequestState::New,
                RequestState::Review,
                RequestState::Declined,
            ],
        };
        if !states.is_empty() {
            let values: Vec<String> = states
                .iter()
                .map(|state| state.as_str().to_string())
                .collect();
            clauses.push(XpathQuery::Pred(
                "state".to_string(),
                Box::new(XpathQuery::eq_list("@name", &values)),
            ));
        }
        if !self.types.is_empty() {
            clauses.push(XpathQuery::Pred(
                "action".to_string(),
                Box::new(XpathQuery::eq_list("@type", &self.types)),
            ));
        }
        for project in &self.exclude_target_projects {
            // `target[not(@project='X')]`: no target of the request
            // is project X. The outer form `not(target[@project='X'])`
            // is refused by the server with 400 — verified live
            // against the production API — so this predicate form is
            // the only one the dialect accepts. One caveat: the
            // server evaluates the match per joined action row, so a
            // multi-action request targeting both X and another
            // project still matches via the other row, and a request
            // with no target at all is not excluded. The form is
            // exact for the overwhelmingly common single-action case.
            clauses.push(XpathQuery::Pred(
                "target".to_string(),
                Box::new(XpathQuery::Not(Box::new(XpathQuery::Eq(
                    "@project".to_string(),
                    project.clone(),
                )))),
            ));
        }
        if clauses.is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "a request search needs at least one filter; the server refuses an empty match",
            ));
        }
        Ok(XpathQuery::and_all(clauses).to_string())
    }
}

/// Validate a value that goes into a single-quoted XPath string
/// literal: it must be non-empty, and it must not contain a single
/// quote, which would break out of the literal (XPath has no string
/// escape).
fn validate_xpath_value(label: &str, value: &str) -> Result<(), GeectlError> {
    if value.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("an XPath {label} value must not be empty"),
        ));
    }
    if value.contains('\'') {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("an XPath {label} value must not contain a single quote: {value:?}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obs::common::is_bare_authority;
    use crate::obs::test_support::{basic_credentials, payload, search_without_states, FakeHttp};

    fn payload_with_cve() -> SrPayload {
        SrPayload::new(
            "home:scarabeus_iv",
            "foo",
            "devel:languages:python",
            "foo",
            "Update foo to 2.0: fix CVE-2026-12345",
            "-------------------------------------------------------------------\nFri Nov 20 07:00:00 UTC 2026 - Tomáš Chvátal <tomas.chvatal@gmail.com>\n\n- CVE-2026-12345: fix the overflow (bsc#1234567)\n",
            &[CveId::parse("CVE-2026-12345").unwrap()],
        )
        .unwrap()
    }

    #[test]
    fn has_open_sr_detects_request() {
        // The search URL is built by the client; can it with the
        // expected response.
        let http_for_url = FakeHttp::new();
        let client = ObsClient::with_credentials(&http_for_url, basic_credentials()).unwrap();
        let url = client.search_url("foo").unwrap();
        let http = FakeHttp::new().with_get(
            &url,
            "<collection matches=\"1\"><request id=\"1398492\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client.has_open_sr("foo").unwrap());
        // The search carried the credentials.
        assert_eq!(http.get_calls().len(), 1);
        assert!(http.get_calls()[0].1);
    }

    #[test]
    fn has_open_sr_ignores_non_submit_requests() {
        // The search match does not filter by action type (the
        // filter lives client-side), so a delete request targeting
        // the package must not read as an open submit request.
        let http_for_url = FakeHttp::new();
        let client = ObsClient::with_credentials(&http_for_url, basic_credentials()).unwrap();
        let url = client.search_url("foo").unwrap();
        let http = FakeHttp::new().with_get(
            &url,
            "<collection matches=\"1\"><request id=\"1398492\" creator=\"scarabeus_iv\"><action type=\"delete\"><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(!client.has_open_sr("foo").unwrap());
    }

    #[test]
    fn has_open_sr_empty_is_false() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let url = client.search_url("foo").unwrap();
        let http = FakeHttp::new().with_get(&url, "<collection matches=\"0\"></collection>");
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(!client.has_open_sr("foo").unwrap());
    }

    #[test]
    fn has_open_sr_unexpected_response_is_error_not_false() {
        // A response with no matches count is a failed lookup: it must
        // never read as "no open SR".
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let url = client.search_url("foo").unwrap();
        let http = FakeHttp::new().with_get(&url, "<collection></collection>");
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client.has_open_sr("foo").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn has_open_sr_without_credentials_fails_closed() {
        // No credentials: refused before any request, not made and 401'd.
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let err = client.has_open_sr("foo").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn create_request_posts_payload_and_returns_id() {
        let payload = payload();
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request?cmd=create",
            "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let id = client.create_request(&payload).unwrap();
        assert_eq!(id, "1400001");
        let posted = http.post_calls();
        assert_eq!(posted.len(), 1);
        assert_eq!(posted[0].0, "https://api.opensuse.org/request?cmd=create");
        // The posted body is the payload's own rendering: the typed
        // path is the only path, and no XML is ever handed in.
        assert_eq!(posted[0].1, payload.to_xml());
        // The filing POST carried the credentials.
        assert!(posted[0].2);
    }

    #[test]
    fn create_request_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request?cmd=create",
            "<request id=\"1400001\"></request>",
        );
        let client = ObsClient::new(&http);
        assert!(client.create_request(&payload()).is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn create_request_transport_error_fails() {
        // No canned POST response: the transport fails, so filing fails
        // instead of silently reporting success.
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client.create_request(&payload()).is_err());
    }

    #[test]
    fn with_credentials_rejects_bearer_and_ssh_key() {
        // OBS accepts Basic only: a Bearer token or SSH key cannot
        // authenticate it, so it is refused at construction.
        let http = FakeHttp::new();
        assert!(ObsClient::with_credentials(
            &http,
            Credentials::Bearer {
                token: "tok3n".to_string()
            }
        )
        .is_err());
        assert!(ObsClient::with_credentials(
            &http,
            Credentials::SshKey {
                path: std::path::PathBuf::from("/home/user/.ssh/id_ed25519"),
            }
        )
        .is_err());
        // Basic and None are accepted.
        assert!(ObsClient::with_credentials(&http, basic_credentials()).is_ok());
        assert!(ObsClient::with_credentials(&http, Credentials::None).is_ok());
    }

    #[test]
    fn custom_base_must_be_https_or_loopback_http() {
        let http = FakeHttp::new();
        assert!(ObsClient::with_base(&http, "https://api.opensuse.org").is_ok());
        assert!(ObsClient::with_base(&http, "http://127.0.0.1:3000").is_ok());
        assert!(ObsClient::with_base(&http, "http://localhost:3000").is_ok());
        assert!(ObsClient::with_base(&http, "http://example.com").is_err());
        assert!(ObsClient::with_base(&http, "http://127.0.0.1.evil.com").is_err());
        // A bracketed-host spoof is not loopback.
        assert!(ObsClient::with_base(&http, "http://[::1]evil.com").is_err());
        assert!(ObsClient::with_base(&http, "api.opensuse.org").is_err());
    }

    #[test]
    fn custom_base_must_be_bare() {
        let http = FakeHttp::new();
        // A path, query, fragment, or userinfo after the authority
        // would be concatenated into every OBS URL.
        for base in [
            "https://api.opensuse.org/evil",
            "https://api.opensuse.org?x=1",
            "https://api.opensuse.org#frag",
            "https://user@api.opensuse.org",
            "https://user:pass@api.opensuse.org",
            "https://api.opensuse.org ",
            "https://:443",
            "http://127.0.0.1:3000/evil",
            "http://user@127.0.0.1:3000",
        ] {
            assert!(
                ObsClient::with_base(&http, base).is_err(),
                "{base} must be refused"
            );
        }
        // A trailing slash is stripped, not a path: the client built
        // from it points at the bare base.
        let client = ObsClient::with_base(&http, "https://api.opensuse.org/").unwrap();
        let url = client.search_url("foo").unwrap();
        assert!(url.starts_with("https://api.opensuse.org/search/request?match="));
        assert!(!url.contains("/evil"));
    }

    #[test]
    fn bare_authority_rejects_whitespace_and_empty_host() {
        assert!(is_bare_authority("api.opensuse.org"));
        assert!(is_bare_authority("127.0.0.1:3000"));
        assert!(is_bare_authority("[::1]:3000"));
        assert!(is_bare_authority("[::1]"));
        assert!(is_bare_authority("[::1]:8080"));
        // Whitespace anywhere in the authority is not a host.
        assert!(!is_bare_authority("api.opensuse.org "));
        assert!(!is_bare_authority("api .opensuse.org"));
        assert!(!is_bare_authority("api\t.opensuse.org"));
        // An empty host: a bare port, or empty brackets.
        assert!(!is_bare_authority(":3000"));
        assert!(!is_bare_authority("[]"));
        assert!(!is_bare_authority("[]:3000"));
        // An unterminated or spoofed bracketed host is not a host.
        assert!(!is_bare_authority("["));
        assert!(!is_bare_authority("[::1"));
        assert!(!is_bare_authority("[::1]evil.com"));
        assert!(!is_bare_authority(""));
        // A port, when present, is a decimal port number.
        assert!(is_bare_authority("api.opensuse.org:443"));
        assert!(is_bare_authority("api.opensuse.org:65535"));
        // An empty port means the default port: accepted, as before.
        assert!(is_bare_authority("api.opensuse.org:"));
        assert!(!is_bare_authority("api.opensuse.org:abc"));
        assert!(!is_bare_authority("api.opensuse.org:80x"));
        assert!(!is_bare_authority("api.opensuse.org:99999"));
        assert!(!is_bare_authority("[::1]:abc"));
    }

    #[test]
    fn to_xml_renders_submit_action() {
        let xml = payload().to_xml();
        assert!(xml.contains("<request>"));
        assert!(xml.contains("<action type=\"submit\">"));
        assert!(xml.contains("<source project=\"home:scarabeus_iv\" package=\"foo\" />"));
        assert!(xml.contains("<target project=\"devel:languages:python\" package=\"foo\" />"));
        assert!(xml.contains("<options><sourceupdate>cleanup</sourceupdate></options>"));
        assert!(xml.contains("<description>Update foo to 2.0</description>"));
        assert!(xml.contains("</request>"));
    }

    #[test]
    fn validate_xml_rejects_a_non_submit_action() {
        // A future `to_xml` edit that changes the action type must
        // fail at payload build time, not at the API.
        let xml = payload().to_xml();
        assert!(validate_sr_xml(&xml).is_ok());
        let delete = xml.replace("<action type=\"submit\">", "<action type=\"delete\">");
        let err = validate_sr_xml(&delete).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.context.contains("type=\"submit\""), "{}", err.context);
        // Either quote style counts: a single-quoted submit passes.
        let single = xml.replace("<action type=\"submit\">", "<action type='submit'>");
        assert!(validate_sr_xml(&single).is_ok());
    }

    #[test]
    fn source_update_values_are_the_server_values() {
        assert_eq!(SourceUpdate::default(), SourceUpdate::Cleanup);
        assert_eq!(SourceUpdate::Cleanup.as_str(), "cleanup");
        assert_eq!(SourceUpdate::Update.as_str(), "update");
        assert_eq!(SourceUpdate::NoUpdate.as_str(), "noupdate");
    }

    #[test]
    fn new_payload_defaults_to_cleanup() {
        assert_eq!(payload().source_update, SourceUpdate::Cleanup);
        assert!(payload()
            .to_xml()
            .contains("<options><sourceupdate>cleanup</sourceupdate></options>"));
        assert!(payload().validate_xml().is_ok());
    }

    #[test]
    fn without_cleanup_keeps_the_branch() {
        let xml = payload().without_cleanup().to_xml();
        assert!(xml.contains("<options><sourceupdate>update</sourceupdate></options>"));
        assert!(!xml.contains("cleanup"));
        assert!(payload().without_cleanup().validate_xml().is_ok());
    }

    #[test]
    fn payload_fields_are_read_through_accessors() {
        let payload = payload();
        assert_eq!(payload.source_project(), "home:scarabeus_iv");
        assert_eq!(payload.source_package(), "foo");
        assert_eq!(payload.target_project(), "devel:languages:python");
        assert_eq!(payload.target_package(), "foo");
        assert_eq!(payload.description(), "Update foo to 2.0");
        assert!(payload.changes_entry().contains("Update to version 2.0."));
        assert!(payload.cves().is_empty());
        assert_eq!(payload.source_update(), SourceUpdate::Cleanup);
    }

    #[test]
    fn validate_semantics_accepts_a_valid_payload() {
        assert!(payload().validate_semantics().is_ok());
        assert!(payload_with_cve().validate_semantics().is_ok());
    }

    #[test]
    fn noupdate_renders_and_validates() {
        let mut p = payload();
        p.source_update = SourceUpdate::NoUpdate;
        assert!(p
            .to_xml()
            .contains("<options><sourceupdate>noupdate</sourceupdate></options>"));
        assert!(p.validate_xml().is_ok());
    }

    #[test]
    fn sourceupdate_value_extracts_and_rejects() {
        assert_eq!(sourceupdate_value(&payload().to_xml()), Some("cleanup"));
        assert_eq!(sourceupdate_value("<request></request>"), None);
        // Two values is not a value.
        assert_eq!(
            sourceupdate_value(
                "<options><sourceupdate>cleanup</sourceupdate><sourceupdate>update</sourceupdate></options>"
            ),
            None
        );
        // An unknown value extracts, so the caller can refuse it.
        assert_eq!(
            sourceupdate_value("<options><sourceupdate>wipe</sourceupdate></options>"),
            Some("wipe")
        );
    }

    #[test]
    fn to_xml_escapes_description() {
        let p = SrPayload::new(
            "home:scarabeus_iv",
            "foo",
            "devel:languages:python",
            "foo",
            "Update foo to 2.0 & fix <bugs>",
            "- Update to version 2.0.\n",
            &[],
        )
        .unwrap();
        let xml = p.to_xml();
        assert!(xml.contains("Update foo to 2.0 &amp; fix &lt;bugs&gt;"));
        assert!(!xml.contains("fix <bugs>"));
        assert!(p.validate_xml().is_ok());
    }

    #[test]
    fn new_rejects_invalid_names_and_empty_text() {
        assert!(SrPayload::new(
            "home:scarabeus_iv",
            "foo/bar",
            "devel:languages:python",
            "foo",
            "Update foo to 2.0",
            "- Update to version 2.0.\n",
            &[],
        )
        .is_err());
        assert!(SrPayload::new(
            "home:scarabeus_iv",
            "foo",
            "devel:languages:python",
            "foo",
            "   ",
            "- Update to version 2.0.\n",
            &[],
        )
        .is_err());
        assert!(SrPayload::new(
            "home:scarabeus_iv",
            "foo",
            "devel:languages:python",
            "foo",
            "Update foo to 2.0",
            "  ",
            &[],
        )
        .is_err());
    }

    #[test]
    fn new_rejects_factory_as_target() {
        // A submit request targets the devel project, never Factory
        // directly.
        assert!(SrPayload::new(
            "home:scarabeus_iv",
            "foo",
            "openSUSE:Factory",
            "foo",
            "Update foo to 2.0",
            "- Update to version 2.0.\n",
            &[],
        )
        .is_err());
    }

    #[test]
    fn new_rejects_unicode_names() {
        // OBS names are ASCII-only; a unicode package name is rejected
        // at payload build time, not at the API.
        assert!(SrPayload::new(
            "home:scarabeus_iv",
            "föö",
            "devel:languages:python",
            "foo",
            "Update foo to 2.0",
            "- Update to version 2.0.\n",
            &[],
        )
        .is_err());
    }

    #[test]
    fn check_well_formed_accepts_generated_xml() {
        assert!(check_well_formed(&payload().to_xml(), SUBMIT_REQUIRED_PATHS).is_ok());
    }

    #[test]
    fn check_well_formed_rejects_mismatched_tags() {
        assert!(check_well_formed("<request><action></request>", SUBMIT_REQUIRED_PATHS).is_err());
        assert!(check_well_formed("<request><action>", SUBMIT_REQUIRED_PATHS).is_err());
        assert!(check_well_formed(
            "<request></request><request></request>",
            SUBMIT_REQUIRED_PATHS
        )
        .is_err());
        assert!(check_well_formed("", SUBMIT_REQUIRED_PATHS).is_err());
    }

    #[test]
    fn check_well_formed_rejects_missing_elements() {
        // Well-formed XML but not an SR: no action/source/target.
        assert!(check_well_formed(
            "<request><description>x</description></request>",
            SUBMIT_REQUIRED_PATHS
        )
        .is_err());
        assert!(check_well_formed(
            "<request><action><source project=\"a\" package=\"b\" /></action></request>",
            SUBMIT_REQUIRED_PATHS
        )
        .is_err());
        // Well-formed SR XML, but without the options block: the
        // cleanup attribute is part of the payload now.
        assert!(check_well_formed(
            "<request><action><source project=\"a\" package=\"b\" /><target project=\"c\" package=\"b\" /></action><description>x</description></request>",
            SUBMIT_REQUIRED_PATHS
        )
        .is_err());
    }

    fn incident_payload() -> MaintenanceIncidentPayload {
        MaintenanceIncidentPayload::new(
            "home:alice:branches:OBS_Maintained:foo",
            &["foo"],
            "openSUSE:Maintenance",
            "Maintenance incident for foo",
        )
        .expect("test incident payload")
    }

    #[test]
    fn maintenance_incident_to_xml_renders_one_action_per_package() {
        assert_eq!(
            incident_payload().to_xml(),
            "<request>\n  <action type=\"maintenance_incident\">\n    <source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" />\n    <target project=\"openSUSE:Maintenance\" />\n  </action>\n  <description>Maintenance incident for foo</description>\n</request>",
        );
    }

    #[test]
    fn maintenance_incident_to_xml_with_release_project_and_cleanup() {
        let payload = MaintenanceIncidentPayload::new(
            "home:alice:branches:OBS_Maintained:foo",
            &["foo", "bar"],
            "openSUSE:Maintenance",
            "Maintenance incident for foo and bar",
        )
        .expect("test incident payload")
        .with_release_project("openSUSE:Leap:15.6:Update")
        .expect("release project")
        .with_cleanup()
        .expect("cleanup");
        assert_eq!(
            payload.to_xml(),
            "<request>\n  <action type=\"maintenance_incident\">\n    <source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" />\n    <target project=\"openSUSE:Maintenance\" releaseproject=\"openSUSE:Leap:15.6:Update\" />\n    <options><sourceupdate>cleanup</sourceupdate></options>\n  </action>\n  <action type=\"maintenance_incident\">\n    <source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"bar\" />\n    <target project=\"openSUSE:Maintenance\" releaseproject=\"openSUSE:Leap:15.6:Update\" />\n    <options><sourceupdate>cleanup</sourceupdate></options>\n  </action>\n  <description>Maintenance incident for foo and bar</description>\n</request>",
        );
        assert!(payload.validate_xml().is_ok());
    }

    #[test]
    fn maintenance_incident_validation_refuses_bad_input() {
        // No source packages: a project-level incident is out of scope.
        assert!(MaintenanceIncidentPayload::new(
            "home:alice:branches:OBS_Maintained:foo",
            &[],
            "openSUSE:Maintenance",
            "Maintenance incident",
        )
        .is_err());
        // Bad names anywhere.
        assert!(MaintenanceIncidentPayload::new(
            "home:alice/bad",
            &["foo"],
            "openSUSE:Maintenance",
            "Maintenance incident",
        )
        .is_err());
        assert!(MaintenanceIncidentPayload::new(
            "home:alice:branches:OBS_Maintained:foo",
            &["foo/bar"],
            "openSUSE:Maintenance",
            "Maintenance incident",
        )
        .is_err());
        assert!(MaintenanceIncidentPayload::new(
            "home:alice:branches:OBS_Maintained:foo",
            &["foo"],
            "openSUSE:Maintenance?",
            "Maintenance incident",
        )
        .is_err());
        // Empty description.
        assert!(MaintenanceIncidentPayload::new(
            "home:alice:branches:OBS_Maintained:foo",
            &["foo"],
            "openSUSE:Maintenance",
            "   ",
        )
        .is_err());
        // Bad release project.
        assert!(incident_payload()
            .with_release_project("openSUSE:Leap/15.6")
            .is_err());
    }

    #[test]
    fn maintenance_incident_validation_refuses_duplicate_source_packages() {
        // Two identical actions would render, which the server refuses
        // (`ConflictingActions` on the incident `uniq_key`).
        assert!(MaintenanceIncidentPayload::new(
            "home:alice:branches:OBS_Maintained:foo",
            &["foo", "foo"],
            "openSUSE:Maintenance",
            "Maintenance incident",
        )
        .is_err());
    }

    #[test]
    fn maintenance_incident_xml_escapes_description() {
        let payload = MaintenanceIncidentPayload::new(
            "home:alice:branches:OBS_Maintained:foo",
            &["foo"],
            "openSUSE:Maintenance",
            "Fix foo & bar <baz>",
        )
        .expect("test incident payload");
        assert!(payload.to_xml().contains("Fix foo &amp; bar &lt;baz&gt;"));
        assert!(payload.validate_xml().is_ok());
    }

    #[test]
    fn change_devel_to_xml_and_validation() {
        let payload = ChangeDevelPayload::new(
            "devel:languages:python",
            "python-foo",
            "openSUSE:Factory",
            "python-foo",
            "Change the devel project for python-foo",
        )
        .expect("test change-devel payload");
        assert_eq!(
            payload.to_xml(),
            "<request>\n  <action type=\"change_devel\">\n    <source project=\"devel:languages:python\" package=\"python-foo\" />\n    <target project=\"openSUSE:Factory\" package=\"python-foo\" />\n  </action>\n  <description>Change the devel project for python-foo</description>\n</request>",
        );
        assert!(payload.validate_xml().is_ok());
        assert!(ChangeDevelPayload::new(
            "devel:languages:python",
            "python-foo",
            "openSUSE:Factory",
            "python-foo?",
            "Change the devel project",
        )
        .is_err());
        assert!(ChangeDevelPayload::new(
            "devel:languages:python",
            "python-foo",
            "openSUSE:Factory",
            "python-foo",
            "",
        )
        .is_err());
    }

    #[test]
    fn delete_to_xml_and_validation() {
        let package = DeletePayload::new(
            "openSUSE:Factory",
            DeleteTarget::Package("foo".to_string()),
            "Drop foo",
        )
        .expect("test delete payload");
        assert_eq!(
            package.to_xml(),
            "<request>\n  <action type=\"delete\">\n    <target project=\"openSUSE:Factory\" package=\"foo\" />\n  </action>\n  <description>Drop foo</description>\n</request>",
        );
        assert!(package.validate_xml().is_ok());
        let repository = DeletePayload::new(
            "openSUSE:Factory",
            DeleteTarget::Repository("standard".to_string()),
            "Drop the standard repository",
        )
        .expect("test delete payload");
        assert!(repository
            .to_xml()
            .contains("<target project=\"openSUSE:Factory\" repository=\"standard\" />"));
        assert!(repository.validate_xml().is_ok());
        let project = DeletePayload::new(
            "home:alice:branches:foo",
            DeleteTarget::WholeProject,
            "Drop the branch project",
        )
        .expect("test delete payload");
        assert!(project
            .to_xml()
            .contains("<target project=\"home:alice:branches:foo\" />"));
        assert!(project.validate_xml().is_ok());
        assert!(DeletePayload::new(
            "openSUSE:Factory",
            DeleteTarget::Package("foo/bar".to_string()),
            "Drop foo",
        )
        .is_err());
        assert!(DeletePayload::new(
            "openSUSE:Factory",
            DeleteTarget::Package("foo".to_string()),
            "   ",
        )
        .is_err());
    }

    #[test]
    fn payload_cve_rule_requires_the_entry_to_name_each_cve() {
        // The entry names the CVE: accepted.
        assert!(payload_with_cve().to_xml().contains("CVE-2026-12345"));
        // The entry does not name the CVE: refused.
        let cve = CveId::parse("CVE-2026-12345").unwrap();
        assert!(SrPayload::new(
            "home:scarabeus_iv",
            "foo",
            "devel:languages:python",
            "foo",
            "Update foo to 2.0: fix CVE-2026-12345",
            "- Update to version 2.0.\n",
            &[cve],
        )
        .is_err());
    }

    #[test]
    fn file_dry_run_returns_none_and_makes_no_requests() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::default();
        let filing = SrFiling::new(&obs, &dry);
        let payload = payload();
        assert_eq!(filing.file(&payload).unwrap(), None);
        // Nothing was read or filed: no GET (dedup) and no POST.
        assert!(http.get_calls().is_empty());
        assert!(http.post_calls().is_empty());
        // The would-do line plus the full would-be request, recorded
        // in the plan (not just printed).
        assert_eq!(
            dry.plan(),
            vec![
                "[dry-run] would: file submit request for foo to devel:languages:python"
                    .to_string(),
                "[dry-run] --- would-be submit request (dry-run) ---".to_string(),
                "[dry-run] source: home:scarabeus_iv/foo".to_string(),
                "[dry-run] target: devel:languages:python/foo".to_string(),
                "[dry-run] description: Update foo to 2.0".to_string(),
                format!("[dry-run] changes entry: {}", payload.changes_entry()),
                format!("[dry-run] {}", payload.to_xml()),
                "[dry-run] --- end of would-be submit request ---".to_string(),
            ]
        );
    }

    #[test]
    fn file_live_returns_id_when_no_open_sr() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs.search_url("foo").unwrap();
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing.file(&payload()).unwrap(),
            Some("1400001".to_string())
        );
        // The dedup search ran first, then the filing POST.
        assert_eq!(http.get_calls().len(), 1);
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn file_live_refuses_when_open_sr_exists() {
        // Dedup: an open SR already targets the package, so filing is
        // refused and no POST is made.
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs.search_url("foo").unwrap();
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"1398492\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing.file(&payload()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn file_live_transport_failure_is_error() {
        // Live mode, credentials present, dedup passes, but the POST
        // itself fails (no canned response = transport error): filing
        // returns Err, it does not silently report a filed id.
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs.search_url("foo").unwrap();
        let http = FakeHttp::new().with_get(&search_url, "<collection matches=\"0\"></collection>");
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert!(filing.file(&payload()).is_err());
    }

    #[test]
    fn validate_request_id_accepts_and_rejects() {
        assert_eq!(validate_request_id("1398492").unwrap(), 1398492);
        // A leading-zero spelling is the same request, canonicalized.
        assert_eq!(validate_request_id("01398492").unwrap(), 1398492);
        assert!(validate_request_id("").is_err());
        assert!(validate_request_id("abc").is_err());
        assert!(validate_request_id("13984a2").is_err());
        assert!(validate_request_id("0").is_err());
        // Overflow: more digits than a u64 holds.
        assert!(validate_request_id("99999999999999999999999").is_err());
    }

    #[test]
    fn supersede_request_posts_changestate_with_superseded_by() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1397897",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client.supersede_request("1397357", "1397897").is_ok());
        let posted = http.post_calls();
        assert_eq!(posted.len(), 1);
        assert_eq!(
            posted[0].0,
            "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1397897"
        );
        // The comment is the form osc sends and the live API shows.
        assert_eq!(posted[0].1, "superseded by 1397897");
        assert!(posted[0].2);
    }

    #[test]
    fn supersede_request_rejects_invalid_ids_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        for (old, new) in [
            ("", "1397897"),
            ("1397357", ""),
            ("abc", "1397897"),
            ("1397357", "def"),
            ("0", "1397897"),
            ("1397357", "1397357"),
        ] {
            assert!(client.supersede_request(old, new).is_err());
        }
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn supersede_request_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1397897",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::new(&http);
        assert!(client.supersede_request("1397357", "1397897").is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn supersede_request_non_ok_body_is_error_not_success() {
        // The server answered 2xx but not ok: a failed supersede must
        // never read as success.
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1397897",
            "<status code=\"not_found\"><summary>no such request</summary></status>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client.supersede_request("1397357", "1397897").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn supersede_request_transport_failure_is_error() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client.supersede_request("1397357", "1397897").is_err());
    }

    #[test]
    fn parse_directory_entry_names_accepts_an_xml_prolog() {
        // The server may emit an XML prolog before the directory
        // element; the result is the same as without one.
        let body = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><directory name=\"test:factory\"><entry name=\"pkgA\"/><entry name=\"pkgB\"/></directory>";
        assert_eq!(
            parse_directory_entry_names(body).unwrap(),
            vec!["pkgA".to_string(), "pkgB".to_string()]
        );
        // Bare prolog, and whitespace/newline between prolog and element.
        assert_eq!(
            parse_directory_entry_names("<?xml?><directory name=\"test:factory\"/>").unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            parse_directory_entry_names(
                "<?xml version=\"1.0\"?>\n<directory name=\"test:factory\"><entry name=\"pkgA\"/></directory>"
            )
            .unwrap(),
            vec!["pkgA".to_string()]
        );
        // A prolog that never terminates is malformed, not an empty listing.
        assert!(
            parse_directory_entry_names("<?xml version=\"1.0\" <directory></directory>").is_err()
        );
        assert!(parse_directory_entry_names("<?xml version=\"1.0\"").is_err());
        // `<?xmlfoo` is not a prolog at all: the guard rejects it as
        // malformed rather than skipping a prefix that is not one.
        assert!(
            parse_directory_entry_names("<?xmlfoo version=\"1.0\"?><directory></directory>")
                .is_err()
        );
    }

    #[test]
    fn parse_directory_entry_names_malformed_error_carries_response_preview() {
        // The failure stays fail-closed, but the error names what was
        // received, so the caller can diagnose it.
        let err = parse_directory_entry_names("garbage").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("garbage"), "{}", err.context);
        // A long body is capped: the preview carries the head, not the tail.
        let long = format!("<collection>{}</collection>", "x".repeat(3000));
        let err = parse_directory_entry_names(&long).unwrap_err();
        assert!(err.context.contains("... [truncated,"), "{}", err.context);
        assert!(!err.context.contains(&"x".repeat(3000)), "{}", err.context);
        // An empty body still fails closed, and says so.
        let err = parse_directory_entry_names("").unwrap_err();
        assert!(err.context.contains("<empty response>"), "{}", err.context);
    }

    #[test]
    fn package_requests_search_url_matches_source_or_target() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .package_requests_search_url("home:scarabeus_iv", "foo")
                .unwrap(),
            "https://api.opensuse.org/search/request?match=%28action%2Fsource%2F%40project%3D%27home%3Ascarabeus_iv%27%20and%20action%2Fsource%2F%40package%3D%27foo%27%20or%20action%2Ftarget%2F%40project%3D%27home%3Ascarabeus_iv%27%20and%20action%2Ftarget%2F%40package%3D%27foo%27%29%20and%20%28state%2F%40name%3D%27new%27%20or%20state%2F%40name%3D%27review%27%20or%20state%2F%40name%3D%27declined%27%29"
        );
        // Invalid names are rejected before any request.
        assert!(client
            .package_requests_search_url("home:foo", "foo/bar")
            .is_err());
        assert!(client.package_requests_search_url("", "foo").is_err());
    }

    #[test]
    fn open_requests_for_package_returns_ids() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let url = client
            .package_requests_search_url("home:scarabeus_iv", "foo")
            .unwrap();
        let http = FakeHttp::new().with_get(
            &url,
            "<collection matches=\"1\"><request id=\"1398492\" /></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .open_requests_for_package("home:scarabeus_iv", "foo")
                .unwrap(),
            vec!["1398492".to_string()]
        );
        assert_eq!(http.get_calls().len(), 1);
        assert!(http.get_calls()[0].1);
    }

    #[test]
    fn open_requests_for_package_empty_is_empty() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let url = client
            .package_requests_search_url("home:scarabeus_iv", "foo")
            .unwrap();
        let http = FakeHttp::new().with_get(&url, "<collection matches=\"0\"></collection>");
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .open_requests_for_package("home:scarabeus_iv", "foo")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn open_requests_for_package_unexpected_response_is_error() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let url = client
            .package_requests_search_url("home:scarabeus_iv", "foo")
            .unwrap();
        let http = FakeHttp::new().with_get(&url, "<request id=\"1398492\" />");
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .open_requests_for_package("home:scarabeus_iv", "foo")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn open_requests_for_package_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let err = client
            .open_requests_for_package("home:scarabeus_iv", "foo")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    fn refile_http() -> FakeHttp {
        // The canned responses for a live refile: the dedup search is
        // empty, filing creates 1400001, superseding 1397357 with it
        // succeeds.
        let probe_http = FakeHttp::new();
        let obs_probe = ObsClient::with_credentials(&probe_http, basic_credentials()).unwrap();
        let search_url = obs_probe.search_url("foo").unwrap();
        FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
            )
            .with_post(
                "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1400001",
                "<status code=\"ok\" />",
            )
    }

    #[test]
    fn refile_live_files_then_supersedes_in_order() {
        let http = refile_http();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing.refile(&payload(), "1397357").unwrap(),
            Some("1400001".to_string())
        );
        // Dedup search first, then the filing POST, then the
        // supersede POST — file before supersede, so superseded_by
        // always names a request that exists.
        assert_eq!(http.get_calls().len(), 1);
        let posted = http.post_calls();
        assert_eq!(posted.len(), 2);
        assert_eq!(posted[0].0, "https://api.opensuse.org/request?cmd=create");
        assert_eq!(
            posted[1].0,
            "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1400001"
        );
        assert_eq!(posted[1].1, "superseded by 1400001");
    }

    #[test]
    fn refile_live_files_when_old_is_the_only_open_sr() {
        // The dedup exemption: the old request is open for the same
        // package, and a refile must still file. Without the
        // exemption the dedup check would refuse the filing, and a
        // refile over an open request could never happen.
        let probe_http = FakeHttp::new();
        let obs_probe = ObsClient::with_credentials(&probe_http, basic_credentials()).unwrap();
        let search_url = obs_probe.search_url("foo").unwrap();
        let http = FakeHttp::new()
            .with_get(
                &search_url,
                "<collection matches=\"1\"><request id=\"1397357\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request></collection>",
            )
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
            )
            .with_post(
                "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1400001",
                "<status code=\"ok\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing.refile(&payload(), "1397357").unwrap(),
            Some("1400001".to_string())
        );
        assert_eq!(http.post_calls().len(), 2);
    }

    #[test]
    fn refile_live_refuses_when_another_open_sr_exists() {
        // The exemption covers only the request being superseded: a
        // second open request for the package still blocks the filing,
        // and no POST is made.
        let probe_http = FakeHttp::new();
        let obs_probe = ObsClient::with_credentials(&probe_http, basic_credentials()).unwrap();
        let search_url = obs_probe.search_url("foo").unwrap();
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"2\"><request id=\"1397357\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request><request id=\"1398492\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing.refile(&payload(), "1397357").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.context.contains("1398492"));
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn refile_dry_run_returns_none_and_records_both_steps() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::default();
        let filing = SrFiling::new(&obs, &dry);
        let payload = payload();
        assert_eq!(filing.refile(&payload, "1397357").unwrap(), None);
        assert!(http.get_calls().is_empty());
        assert!(http.post_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec![
                "[dry-run] would: file submit request for foo to devel:languages:python"
                    .to_string(),
                "[dry-run] --- would-be submit request (dry-run) ---".to_string(),
                "[dry-run] source: home:scarabeus_iv/foo".to_string(),
                "[dry-run] target: devel:languages:python/foo".to_string(),
                "[dry-run] description: Update foo to 2.0".to_string(),
                format!("[dry-run] changes entry: {}", payload.changes_entry()),
                format!("[dry-run] {}", payload.to_xml()),
                "[dry-run] --- end of would-be submit request ---".to_string(),
                "[dry-run] would: supersede submit request 1397357 with the new request"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn refile_supersede_failure_names_the_new_id() {
        // Filing succeeds but the supersede does not: the error must
        // carry the new id so the caller can finish manually — the
        // old request is still open and the new one unlinked.
        let probe_http = FakeHttp::new();
        let obs_probe = ObsClient::with_credentials(&probe_http, basic_credentials()).unwrap();
        let search_url = obs_probe.search_url("foo").unwrap();
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
            )
            .with_post(
                "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1400001",
                "<status code=\"not_found\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing.refile(&payload(), "1397357").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("1400001"), "error names the new id");
        assert!(err.context.contains("1397357"), "error names the old id");
    }

    #[test]
    fn refile_rejects_invalid_old_id_without_http() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert!(filing.refile(&payload(), "not-an-id").is_err());
        assert!(http.get_calls().is_empty());
        assert!(http.post_calls().is_empty());
    }

    const REQUEST1_XML: &str = r#"<request creator="creator" id="42">
  <action type="submit">
    <source package="bar" project="foo" rev="1" />
    <target package="bar" project="foobar" />
  </action>
  <action type="delete">
    <target project="deleteme" />
  </action>
  <state name="accepted" when="2010-12-27T01:36:29" who="user1" />
  <history when="2010-12-13T13:02:03" who="creator">
    <description>Create Request</description>
    <comment>foobar</comment>
  </history>
  <title>title of the request</title>
  <description>this is a
very long
description</description>
</request>"#;

    const REQUEST2_XML: &str = r#"<request creator="creator" id="123">
  <action type="submit">
    <source package="abc" project="xyz" />
    <options>
      <sourceupdate>cleanup</sourceupdate>
      <updatelink>1</updatelink>
    </options>
  </action>
  <action type="add_role">
    <target project="home:foo" />
    <person name="bar" role="maintainer" />
    <group name="groupxyz" role="reader" />
  </action>
  <state approver="someone" name="review" when="2010-12-27T01:36:29" who="abc" />
  <review by_group="group1" state="new" when="2010-12-28T00:11:22" who="abc">
    <comment>review start</comment>
  </review>
  <history when="2010-12-11T00:00:00" who="creator">
    <description>Created request</description>
  </history>
</request>"#;

    const LIST_VIEW1_XML: &str = r#"<request creator="Admin" id="62">
  <action type="set_bugowner">
    <target project="foo" />
    <person name="buguser" />
  </action>
  <action type="add_role">
    <target project="foobar" />
    <person name="xyz" role="maintainer" />
    <group name="group1" role="reader" />
  </action>
  <action type="add_role">
    <target project="foo" package="bar" />
    <person name="abc" role="reviewer" />
  </action>
  <action type="change_devel">
    <source project="devprj" package="devpkg" />
    <target project="foo" package="bar" />
  </action>
  <action type="submit">
    <source project="srcprj" package="srcpackage" />
    <target project="tgtprj" package="tgtpackage" />
  </action>
  <action type="submit">
    <source project="foo" package="bar" />
    <target project="baz" />
  </action>
  <action type="delete">
    <target project="deleteme" />
  </action>
  <action type="delete">
    <target project="foo" package="bar" />
  </action>
  <state name="new" when="2010-12-29T14:57:25" who="Admin">
    <comment></comment>
  </state>
</request>"#;

    /// A minimal request carrying one action, for the construction
    /// tests: id 42, creator `creator`, state `new`.
    fn request_with_action(action: RequestAction) -> Request {
        Request {
            id: 42,
            creator: "creator".to_string(),
            title: None,
            description: String::new(),
            actions: vec![action],
            priority: None,
            state: RequestStateInfo {
                name: RequestState::New,
                who: "creator".to_string(),
                when: "2010-12-13T13:02:03".to_string(),
                created: None,
                superseded_by: None,
                approver: None,
                comment: String::new(),
            },
            reviews: Vec::new(),
            history: Vec::new(),
            accept_at: None,
        }
    }

    /// A request document carrying one action fragment, for the
    /// action-parsing tests.
    fn request_document_with_action(action_xml: &str) -> String {
        format!(
            "<request creator=\"creator\" id=\"42\">\n  {action_xml}\n  <state name=\"new\" when=\"2010-12-27T01:36:29\" who=\"creator\" />\n</request>"
        )
    }

    #[test]
    fn request_state_names_round_trip() {
        for (name, state) in [
            ("new", RequestState::New),
            ("review", RequestState::Review),
            ("accepted", RequestState::Accepted),
            ("declined", RequestState::Declined),
            ("revoked", RequestState::Revoked),
            ("superseded", RequestState::Superseded),
            ("deleted", RequestState::Deleted),
        ] {
            assert_eq!(state.as_str(), name);
            assert_eq!(RequestState::parse(name), Some(state));
        }
        assert_eq!(RequestState::parse("exploded"), None);
    }

    #[test]
    fn request_priority_names_round_trip() {
        for (name, priority) in [
            ("low", RequestPriority::Low),
            ("moderate", RequestPriority::Moderate),
            ("important", RequestPriority::Important),
            ("critical", RequestPriority::Critical),
        ] {
            assert_eq!(priority.as_str(), name);
            assert_eq!(RequestPriority::parse(name), Some(priority));
        }
        assert_eq!(RequestPriority::parse("urgent"), None);
    }

    #[test]
    fn request_createsr_serialises_a_submit_action() {
        let action = RequestAction::Submit {
            source_project: "foo".to_string(),
            source_package: "bar".to_string(),
            source_rev: Some("42".to_string()),
            target_project: Some("foobar".to_string()),
            target_package: Some("bar".to_string()),
            sourceupdate: None,
            updatelink: false,
            makeoriginolder: false,
            acceptinfo: None,
        };
        assert_eq!(action.type_name(), "submit");
        let request = request_with_action(action);
        assert_eq!(
            request.to_xml(),
            "<request creator=\"creator\" id=\"42\">\n  <action type=\"submit\">\n    <source project=\"foo\" package=\"bar\" rev=\"42\" />\n    <target project=\"foobar\" package=\"bar\" />\n  </action>\n  <state name=\"new\" who=\"creator\" when=\"2010-12-13T13:02:03\" />\n</request>"
        );
        // The serialised form parses back to the same request.
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_createsr_with_option_serialises_options() {
        let action = RequestAction::Submit {
            source_project: "foo".to_string(),
            source_package: "bar".to_string(),
            source_rev: None,
            target_project: Some("foobar".to_string()),
            target_package: Some("bar".to_string()),
            sourceupdate: Some(SourceUpdate::Cleanup),
            updatelink: true,
            makeoriginolder: false,
            acceptinfo: None,
        };
        let request = request_with_action(action);
        assert_eq!(
            request.to_xml(),
            "<request creator=\"creator\" id=\"42\">\n  <action type=\"submit\">\n    <source project=\"foo\" package=\"bar\" />\n    <target project=\"foobar\" package=\"bar\" />\n    <options>\n      <sourceupdate>cleanup</sourceupdate>\n      <updatelink>true</updatelink>\n    </options>\n  </action>\n  <state name=\"new\" who=\"creator\" when=\"2010-12-13T13:02:03\" />\n</request>"
        );
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_createsr_missing_tgt_package_serialises_project_only() {
        let action = RequestAction::Submit {
            source_project: "foo".to_string(),
            source_package: "bar".to_string(),
            source_rev: None,
            target_project: Some("foobar".to_string()),
            target_package: None,
            sourceupdate: None,
            updatelink: false,
            makeoriginolder: false,
            acceptinfo: None,
        };
        let request = request_with_action(action);
        assert!(request.to_xml().contains("<target project=\"foobar\" />"));
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_create_add_role_person_serialises() {
        let action = RequestAction::AddRole {
            target_project: "foo".to_string(),
            target_package: Some("bar".to_string()),
            person: Some(("user".to_string(), "reader".to_string())),
            group: None,
        };
        let request = request_with_action(action);
        assert_eq!(
            request.to_xml(),
            "<request creator=\"creator\" id=\"42\">\n  <action type=\"add_role\">\n    <target project=\"foo\" package=\"bar\" />\n    <person name=\"user\" role=\"reader\" />\n  </action>\n  <state name=\"new\" who=\"creator\" when=\"2010-12-13T13:02:03\" />\n</request>"
        );
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_create_add_role_group_serialises() {
        let action = RequestAction::AddRole {
            target_project: "foo".to_string(),
            target_package: Some("bar".to_string()),
            person: None,
            group: Some(("group".to_string(), "reviewer".to_string())),
        };
        let request = request_with_action(action);
        assert!(request
            .to_xml()
            .contains("<group name=\"group\" role=\"reviewer\" />"));
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_create_add_role_person_group_serialises_both() {
        let action = RequestAction::AddRole {
            target_project: "foo".to_string(),
            target_package: Some("bar".to_string()),
            person: Some(("user".to_string(), "reader".to_string())),
            group: Some(("group".to_string(), "reviewer".to_string())),
        };
        let request = request_with_action(action);
        let xml = request.to_xml();
        assert!(xml.contains("<person name=\"user\" role=\"reader\" />"));
        assert!(xml.contains("<group name=\"group\" role=\"reviewer\" />"));
        assert_eq!(parse_request(&xml), Some(request));
    }

    #[test]
    fn request_create_set_bugowner_project_serialises() {
        let action = RequestAction::SetBugowner {
            target_project: "foobar".to_string(),
            target_package: None,
            person_name: Some("buguser".to_string()),
            group_name: None,
        };
        let request = request_with_action(action);
        assert_eq!(
            request.to_xml(),
            "<request creator=\"creator\" id=\"42\">\n  <action type=\"set_bugowner\">\n    <target project=\"foobar\" />\n    <person name=\"buguser\" />\n  </action>\n  <state name=\"new\" who=\"creator\" when=\"2010-12-13T13:02:03\" />\n</request>"
        );
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_create_set_bugowner_package_serialises() {
        let action = RequestAction::SetBugowner {
            target_project: "foobar".to_string(),
            target_package: Some("baz".to_string()),
            person_name: Some("buguser".to_string()),
            group_name: None,
        };
        let request = request_with_action(action);
        assert!(request
            .to_xml()
            .contains("<target project=\"foobar\" package=\"baz\" />"));
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_create_delete_project_serialises() {
        let action = RequestAction::Delete {
            target_project: "foo".to_string(),
            target_package: None,
            target_repository: None,
        };
        let request = request_with_action(action);
        assert!(request.to_xml().contains("<target project=\"foo\" />"));
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_create_delete_package_serialises() {
        let action = RequestAction::Delete {
            target_project: "foo".to_string(),
            target_package: Some("deleteme".to_string()),
            target_repository: None,
        };
        let request = request_with_action(action);
        assert!(request
            .to_xml()
            .contains("<target project=\"foo\" package=\"deleteme\" />"));
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_create_change_devel_serialises() {
        let action = RequestAction::ChangeDevel {
            source_project: "foo".to_string(),
            source_package: "bar".to_string(),
            target_project: "devprj".to_string(),
            target_package: "devpkg".to_string(),
        };
        let request = request_with_action(action);
        assert_eq!(
            request.to_xml(),
            "<request creator=\"creator\" id=\"42\">\n  <action type=\"change_devel\">\n    <source project=\"foo\" package=\"bar\" />\n    <target project=\"devprj\" package=\"devpkg\" />\n  </action>\n  <state name=\"new\" who=\"creator\" when=\"2010-12-13T13:02:03\" />\n</request>"
        );
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_action_from_xml_add_role_parses_and_round_trips() {
        let xml = request_document_with_action(
            "<action type=\"add_role\">\n  <target package=\"bar\" project=\"foo\" />\n  <person name=\"user\" role=\"reader\" />\n  <group name=\"group\" role=\"reviewer\" />\n</action>",
        );
        let request = parse_request(&xml).expect("add_role parses");
        assert_eq!(
            request.actions[0],
            RequestAction::AddRole {
                target_project: "foo".to_string(),
                target_package: Some("bar".to_string()),
                person: Some(("user".to_string(), "reader".to_string())),
                group: Some(("group".to_string(), "reviewer".to_string())),
            }
        );
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_action_from_xml_submit_with_options_parses_and_round_trips() {
        let xml = request_document_with_action(
            "<action type=\"submit\">\n  <source package=\"bar\" project=\"foo\" />\n  <target package=\"bar\" project=\"foobar\" />\n  <options>\n    <sourceupdate>cleanup</sourceupdate>\n    <updatelink>1</updatelink>\n  </options>\n</action>",
        );
        let request = parse_request(&xml).expect("submit with options parses");
        assert_eq!(
            request.actions[0],
            RequestAction::Submit {
                source_project: "foo".to_string(),
                source_package: "bar".to_string(),
                source_rev: None,
                target_project: Some("foobar".to_string()),
                target_package: Some("bar".to_string()),
                sourceupdate: Some(SourceUpdate::Cleanup),
                updatelink: true,
                makeoriginolder: false,
                acceptinfo: None,
            }
        );
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_action_from_xml_submit_with_acceptinfo_parses_and_round_trips() {
        let xml = request_document_with_action(
            "<action type=\"submit\">\n  <source package=\"bar\" project=\"testprj\" />\n  <target package=\"baz\" project=\"foobar\" />\n  <acceptinfo rev=\"5\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" xsrcmd5=\"ffffffffffffffffffffffffffffffff\" />\n</action>",
        );
        let request = parse_request(&xml).expect("submit with acceptinfo parses");
        assert_eq!(
            request.actions[0],
            RequestAction::Submit {
                source_project: "testprj".to_string(),
                source_package: "bar".to_string(),
                source_rev: None,
                target_project: Some("foobar".to_string()),
                target_package: Some("baz".to_string()),
                sourceupdate: None,
                updatelink: false,
                makeoriginolder: false,
                acceptinfo: Some(AcceptInfo {
                    rev: "5".to_string(),
                    srcmd5: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                    oproject: None,
                    opackage: None,
                    osrcmd5: None,
                    xsrcmd5: Some("ffffffffffffffffffffffffffffffff".to_string()),
                    oxsrcmd5: None,
                }),
            }
        );
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_parse_rejects_unknown_action_type() {
        // An unknown action type at construction is unrepresentable in
        // the typed model, and an unknown action kwarg cannot be
        // passed to a typed constructor at all — so the parse refusal
        // is the adoption of both invalid-argument cases: a document
        // carrying an action the library does not know fails whole.
        let xml = request_document_with_action(
            "<action type=\"foo\"><source package=\"bar\" project=\"foo\" /></action>",
        );
        assert_eq!(parse_request(&xml), None);
    }

    #[test]
    fn request_read_request1_parses_and_round_trips() {
        let request = parse_request(REQUEST1_XML).expect("request 1 parses");
        assert_eq!(request.id, 42);
        assert_eq!(request.creator, "creator");
        assert_eq!(
            request.actions[0],
            RequestAction::Submit {
                source_project: "foo".to_string(),
                source_package: "bar".to_string(),
                source_rev: Some("1".to_string()),
                target_project: Some("foobar".to_string()),
                target_package: Some("bar".to_string()),
                sourceupdate: None,
                updatelink: false,
                makeoriginolder: false,
                acceptinfo: None,
            }
        );
        assert_eq!(
            request.actions[1],
            RequestAction::Delete {
                target_project: "deleteme".to_string(),
                target_package: None,
                target_repository: None,
            }
        );
        assert_eq!(request.state.name, RequestState::Accepted);
        assert_eq!(request.state.when, "2010-12-27T01:36:29");
        assert_eq!(request.state.who, "user1");
        assert_eq!(request.state.approver, None);
        assert_eq!(request.state.comment, "");
        assert_eq!(request.history.len(), 1);
        assert_eq!(request.history[0].when, "2010-12-13T13:02:03");
        assert_eq!(request.history[0].who, "creator");
        assert_eq!(request.history[0].description, "Create Request");
        assert_eq!(request.history[0].comment, "foobar");
        assert_eq!(request.title.as_deref(), Some("title of the request"));
        assert_eq!(request.description, "this is a\nvery long\ndescription");
        assert!(request.reviews.is_empty());
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_read_request2_parses_and_round_trips() {
        let request = parse_request(REQUEST2_XML).expect("request 2 parses");
        assert_eq!(request.id, 123);
        assert_eq!(request.creator, "creator");
        // The submit action carries no target element at all.
        assert_eq!(
            request.actions[0],
            RequestAction::Submit {
                source_project: "xyz".to_string(),
                source_package: "abc".to_string(),
                source_rev: None,
                target_project: None,
                target_package: None,
                sourceupdate: Some(SourceUpdate::Cleanup),
                updatelink: true,
                makeoriginolder: false,
                acceptinfo: None,
            }
        );
        assert_eq!(
            request.actions[1],
            RequestAction::AddRole {
                target_project: "home:foo".to_string(),
                target_package: None,
                person: Some(("bar".to_string(), "maintainer".to_string())),
                group: Some(("groupxyz".to_string(), "reader".to_string())),
            }
        );
        assert_eq!(request.state.name, RequestState::Review);
        assert_eq!(request.state.when, "2010-12-27T01:36:29");
        assert_eq!(request.state.approver.as_deref(), Some("someone"));
        assert_eq!(request.state.who, "abc");
        assert_eq!(request.state.comment, "");
        assert_eq!(request.reviews.len(), 1);
        assert_eq!(request.reviews[0].state, ReviewState::New);
        assert_eq!(request.reviews[0].by_group.as_deref(), Some("group1"));
        assert_eq!(
            request.reviews[0].when.as_deref(),
            Some("2010-12-28T00:11:22")
        );
        assert_eq!(request.reviews[0].who.as_deref(), Some("abc"));
        assert_eq!(request.reviews[0].comment, "review start");
        assert_eq!(request.reviews[0].by_user, None);
        assert_eq!(request.history.len(), 1);
        assert_eq!(request.history[0].when, "2010-12-11T00:00:00");
        assert_eq!(request.history[0].who, "creator");
        assert_eq!(request.history[0].comment, "");
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_review_with_history_parses_and_round_trips() {
        // A review that was acted on carries one `<history>` child
        // per change, each with a description of what happened —
        // never empty, never a placeholder. The parser must read it,
        // not refuse the whole request.
        let xml = "<request creator=\"creator\" id=\"7\">\n  <action type=\"submit\">\n    <source package=\"bar\" project=\"foo\" />\n    <target package=\"bar\" project=\"foobar\" />\n  </action>\n  <state name=\"review\" when=\"2010-12-27T01:36:29\" who=\"abc\" />\n  <review by_user=\"Admin\" state=\"accepted\" when=\"2010-12-28T00:11:22\" who=\"abc\">\n    <comment>looks good</comment>\n    <history when=\"2010-12-28T00:12:00\" who=\"Admin\">\n      <description>Review got accepted</description>\n    </history>\n  </review>\n</request>";
        let request = parse_request(xml).expect("review with history parses");
        assert_eq!(request.reviews.len(), 1);
        assert_eq!(request.reviews[0].state, ReviewState::Accepted);
        assert_eq!(request.reviews[0].history.len(), 1);
        assert_eq!(request.reviews[0].history[0].who, "Admin");
        assert_eq!(
            request.reviews[0].history[0].description,
            "Review got accepted"
        );
        assert_eq!(request.reviews[0].history[0].comment, "");
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_review_obsoleted_parses() {
        // `obsoleted` is a review-only state, set when the reviewer
        // is removed; it never appears on a request itself.
        let xml = "<request creator=\"creator\" id=\"8\">\n  <action type=\"submit\">\n    <source package=\"bar\" project=\"foo\" />\n    <target package=\"bar\" project=\"foobar\" />\n  </action>\n  <state name=\"new\" when=\"2010-12-27T01:36:29\" who=\"abc\" />\n  <review by_user=\"gone\" state=\"obsoleted\" when=\"2010-12-28T00:11:22\" who=\"abc\">\n    <history when=\"2010-12-28T00:12:00\" who=\"abc\">\n      <description>Review was updated</description>\n    </history>\n  </review>\n</request>";
        let request = parse_request(xml).expect("obsoleted review parses");
        assert_eq!(request.reviews.len(), 1);
        assert_eq!(request.reviews[0].state, ReviewState::Obsoleted);
        assert_eq!(request.reviews[0].history.len(), 1);
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_read_request3_empty_comment_and_description_drop_on_serialise() {
        let xml = "<request creator=\"xyz\" id=\"2\">\n  <action type=\"set_bugowner\">\n    <target project=\"foo\" />\n    <person name=\"buguser\" />\n  </action>\n  <state name=\"new\" when=\"2010-12-28T12:36:29\" who=\"xyz\">\n    <comment></comment>\n  </state>\n  <description></description>\n</request>";
        let request = parse_request(xml).expect("request 3 parses");
        assert_eq!(request.id, 2);
        assert_eq!(request.creator, "xyz");
        assert_eq!(request.state.name, RequestState::New);
        assert_eq!(request.state.comment, "");
        assert_eq!(request.description, "");
        assert!(request.history.is_empty());
        assert!(request.reviews.is_empty());
        let serialised = request.to_xml();
        assert!(
            !serialised.contains("<comment>"),
            "empty comment drops: {serialised}"
        );
        assert!(
            !serialised.contains("<description>"),
            "empty description drops: {serialised}"
        );
        assert_eq!(parse_request(&serialised), Some(request));
    }

    #[test]
    fn request_legacy_form_parses_and_reserialises_modern() {
        let xml = "<request creator=\"olduser\" id=\"1234\" type=\"submit\">\n  <submit>\n    <source package=\"baz\" project=\"foobar\" />\n    <target package=\"baz\" project=\"foo\" />\n  </submit>\n  <state name=\"new\" when=\"2010-12-30T02:11:22\" who=\"olduser\" />\n</request>";
        let request = parse_request(xml).expect("legacy request parses");
        assert_eq!(request.id, 1234);
        assert_eq!(request.creator, "olduser");
        assert_eq!(
            request.actions[0],
            RequestAction::Submit {
                source_project: "foobar".to_string(),
                source_package: "baz".to_string(),
                source_rev: None,
                target_project: Some("foo".to_string()),
                target_package: Some("baz".to_string()),
                sourceupdate: None,
                updatelink: false,
                makeoriginolder: false,
                acceptinfo: None,
            }
        );
        assert_eq!(request.state.name, RequestState::New);
        assert_eq!(request.state.when, "2010-12-30T02:11:22");
        assert_eq!(request.state.who, "olduser");
        // The modern form: an `<action>` element, and no `type` on
        // the root.
        let serialised = request.to_xml();
        assert!(serialised.contains("<action type=\"submit\">"));
        assert!(serialised.contains("<request creator=\"olduser\" id=\"1234\">"));
        assert!(!serialised.contains("<submit>"));
        assert_eq!(parse_request(&serialised), Some(request));
    }

    #[test]
    fn request_get_actions_filters_by_type() {
        let request = parse_request(LIST_VIEW1_XML).expect("list view parses");
        assert_eq!(request.actions.len(), 8);
        assert_eq!(request.actions_of(&["submit"]).len(), 2);
        for action in request.actions_of(&["submit"]) {
            assert_eq!(action.type_name(), "submit");
        }
        assert_eq!(
            request
                .actions_of(&["submit", "delete", "change_devel"])
                .len(),
            5
        );
        assert_eq!(request.actions_of(&[]).len(), 8);
        assert_eq!(request.actions_of(&["delete"]).len(), 2);
    }

    #[test]
    fn request_parse_rejects_unknown_child_element() {
        let xml = "<request creator=\"creator\" id=\"42\">\n  <action type=\"delete\">\n    <target project=\"foo\" />\n  </action>\n  <state name=\"new\" when=\"2010-12-27T01:36:29\" who=\"creator\" />\n  <surprise>hello</surprise>\n</request>";
        assert_eq!(parse_request(xml), None);
    }

    #[test]
    fn request_parse_requires_a_state() {
        let xml = "<request creator=\"creator\" id=\"42\">\n  <action type=\"delete\">\n    <target project=\"foo\" />\n  </action>\n</request>";
        assert_eq!(parse_request(xml), None);
    }

    #[test]
    fn request_parse_rejects_missing_or_zero_id() {
        let no_id = "<request creator=\"creator\">\n  <action type=\"delete\">\n    <target project=\"foo\" />\n  </action>\n  <state name=\"new\" when=\"2010-12-27T01:36:29\" who=\"creator\" />\n</request>";
        assert_eq!(parse_request(no_id), None);
        let zero_id = no_id.replace("creator=\"creator\"", "creator=\"creator\" id=\"0\"");
        assert_eq!(parse_request(&zero_id), None);
    }

    #[test]
    fn open_action_request_ids_searches_by_source_target_and_type() {
        let match_expr = "(state/@name='new' or state/@name='review') and action/@type='maintenance_incident' and action[source/@project='home:alice:branches:OBS_Maintained:foo' and source/@package='foo' and target/@project='openSUSE:Maintenance']";
        let url = build_url(
            "https://api.opensuse.org",
            &["search", "request"],
            &[("match", match_expr)],
        );
        let http = FakeHttp::new().with_get(
            &url,
            "<collection matches=\"2\"><request id=\"2\" creator=\"alice\"><action type=\"maintenance_incident\"><source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" /><target project=\"openSUSE:Maintenance\" /></action><state name=\"review\" /></request><request id=\"9\" creator=\"alice\"><action type=\"submit\"><source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" /><target project=\"openSUSE:Maintenance\" /></action><state name=\"new\" /></request></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        // Only the maintenance_incident request blocks, not the
        // submit one that happens to match the same source/target.
        // The parser re-checks the type even though the search
        // already filtered on it.
        assert_eq!(
            client
                .open_action_request_ids(&ActionTarget {
                    action_type: "maintenance_incident",
                    source_project: Some("home:alice:branches:OBS_Maintained:foo"),
                    source_package: Some("foo"),
                    target_project: "openSUSE:Maintenance",
                    target_package: None,
                    target_repository: None,
                    shape: ActionShape::ReleaseProject(None),
                })
                .unwrap(),
            vec!["2".to_string()]
        );
        assert_eq!(http.get_calls().len(), 1);
        assert!(http.get_calls()[0].1);
    }

    #[test]
    fn open_action_request_ids_without_credentials_fails_loud() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let err = client
            .open_action_request_ids(&ActionTarget {
                action_type: "maintenance_incident",
                source_project: Some("home:alice:branches:OBS_Maintained:foo"),
                source_package: Some("foo"),
                target_project: "openSUSE:Maintenance",
                target_package: None,
                target_repository: None,
                shape: ActionShape::Any,
            })
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
    }

    #[test]
    fn request_parse_rejects_duplicate_state() {
        let xml = "<request creator=\"creator\" id=\"42\">\n  <action type=\"delete\">\n    <target project=\"foo\" />\n  </action>\n  <state name=\"new\" when=\"2010-12-27T01:36:29\" who=\"creator\" />\n  <state name=\"review\" when=\"2010-12-28T01:36:29\" who=\"creator\" />\n</request>";
        assert_eq!(parse_request(xml), None);
    }

    #[test]
    fn request_parse_accepts_priority_and_rejects_an_unknown_one() {
        let xml = "<request creator=\"creator\" id=\"42\">\n  <action type=\"delete\">\n    <target project=\"foo\" />\n  </action>\n  <priority>important</priority>\n  <state name=\"new\" when=\"2010-12-27T01:36:29\" who=\"creator\" />\n</request>";
        let request = parse_request(xml).expect("priority parses");
        assert_eq!(request.priority, Some(RequestPriority::Important));
        assert_eq!(parse_request(&request.to_xml()), Some(request));
        let bad = xml.replace("important", "urgent");
        assert_eq!(parse_request(&bad), None);
    }

    #[test]
    fn request_parse_accepts_accept_at() {
        let xml = "<request creator=\"creator\" id=\"42\">\n  <action type=\"delete\">\n    <target project=\"foo\" />\n  </action>\n  <state name=\"new\" when=\"2010-12-27T01:36:29\" who=\"creator\" />\n  <accept_at>2010-12-30T00:00:00</accept_at>\n</request>";
        let request = parse_request(xml).expect("accept_at parses");
        assert_eq!(request.accept_at.as_deref(), Some("2010-12-30T00:00:00"));
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_parse_unescapes_entities_in_text() {
        let xml = "<request creator=\"creator\" id=\"42\">\n  <action type=\"delete\">\n    <target project=\"foo\" />\n  </action>\n  <state name=\"new\" when=\"2010-12-27T01:36:29\" who=\"creator\" />\n  <title>fish &amp; chips</title>\n  <description>5 &lt; 6 &amp;&amp; 7 &gt; 6</description>\n</request>";
        let request = parse_request(xml).expect("entities parse");
        assert_eq!(request.title.as_deref(), Some("fish & chips"));
        assert_eq!(request.description, "5 < 6 && 7 > 6");
        // Re-serialising escapes them again, and re-parsing reads
        // the same request back.
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn review_target_describe_names_each_part() {
        let target = ReviewTarget::new(Some("Admin"), None, None, None).unwrap();
        assert_eq!(target.describe(), "user Admin");
        let target = ReviewTarget::new(None, Some("reviewers"), None, None).unwrap();
        assert_eq!(target.describe(), "group reviewers");
        let target = ReviewTarget::new(None, None, Some("openSUSE:Factory"), None).unwrap();
        assert_eq!(target.describe(), "project openSUSE:Factory");
        let target = ReviewTarget::new(None, None, Some("openSUSE:Factory"), Some("foo")).unwrap();
        assert_eq!(target.describe(), "project openSUSE:Factory/package foo");
        let target = ReviewTarget::new(
            Some("Admin"),
            Some("reviewers"),
            Some("openSUSE:Factory"),
            None,
        )
        .unwrap();
        assert_eq!(
            target.describe(),
            "user Admin, group reviewers, project openSUSE:Factory"
        );
    }

    #[test]
    fn xpath_noop_renders_a_bare_comparison() {
        let query = XpathQuery::Eq("@name".to_string(), "foo".to_string());
        assert_eq!(query.to_string(), "@name='foo'");
    }

    #[test]
    fn xpath_not_wraps_a_comparison() {
        let query = XpathQuery::Not(Box::new(XpathQuery::Eq(
            "@name".to_string(),
            "foo".to_string(),
        )));
        assert_eq!(query.to_string(), "not(@name='foo')");
    }

    #[test]
    fn xpath_contains_renders_a_contains_call() {
        let query = XpathQuery::Contains("@name".to_string(), "foo".to_string());
        assert_eq!(query.to_string(), "contains(@name, 'foo')");
    }

    #[test]
    fn xpath_and_chains_flat() {
        let query = XpathQuery::and_all(vec![
            XpathQuery::Eq("@name".to_string(), "foo".to_string()),
            XpathQuery::Eq("@name".to_string(), "bar".to_string()),
        ]);
        assert_eq!(query.to_string(), "@name='foo' and @name='bar'");
        let query = XpathQuery::and_all(vec![
            XpathQuery::Eq("@name".to_string(), "foo".to_string()),
            XpathQuery::Eq("@name".to_string(), "bar".to_string()),
            XpathQuery::Eq("@name".to_string(), "baz".to_string()),
        ]);
        assert_eq!(
            query.to_string(),
            "@name='foo' and @name='bar' and @name='baz'"
        );
    }

    #[test]
    fn xpath_or_chains_flat() {
        let query = XpathQuery::or_all(vec![
            XpathQuery::Eq("@name".to_string(), "foo".to_string()),
            XpathQuery::Eq("@name".to_string(), "bar".to_string()),
        ]);
        assert_eq!(query.to_string(), "@name='foo' or @name='bar'");
        let query = XpathQuery::or_all(vec![
            XpathQuery::Eq("@name".to_string(), "foo".to_string()),
            XpathQuery::Eq("@name".to_string(), "bar".to_string()),
            XpathQuery::Eq("@name".to_string(), "baz".to_string()),
        ]);
        assert_eq!(
            query.to_string(),
            "@name='foo' or @name='bar' or @name='baz'"
        );
    }

    #[test]
    fn xpath_and_or_parenthesises_the_and_side() {
        let and = XpathQuery::and_all(vec![
            XpathQuery::Eq("@name".to_string(), "foo".to_string()),
            XpathQuery::Eq("@name".to_string(), "bar".to_string()),
        ]);
        let query = XpathQuery::or_all(vec![
            and,
            XpathQuery::Eq("@name".to_string(), "baz".to_string()),
        ]);
        assert_eq!(
            query.to_string(),
            "(@name='foo' and @name='bar') or @name='baz'"
        );
        let query = XpathQuery::or_all(vec![
            query,
            XpathQuery::Eq("@name".to_string(), "xyz".to_string()),
        ]);
        assert_eq!(
            query.to_string(),
            "(@name='foo' and @name='bar') or @name='baz' or @name='xyz'"
        );
    }

    #[test]
    fn xpath_or_and_parenthesises_the_or_side() {
        let or = XpathQuery::or_all(vec![
            XpathQuery::Eq("@name".to_string(), "foo".to_string()),
            XpathQuery::Eq("@name".to_string(), "bar".to_string()),
        ]);
        let query = XpathQuery::and_all(vec![
            or,
            XpathQuery::Eq("@name".to_string(), "baz".to_string()),
        ]);
        assert_eq!(
            query.to_string(),
            "(@name='foo' or @name='bar') and @name='baz'"
        );
        let query = XpathQuery::and_all(vec![
            query,
            XpathQuery::Eq("@name".to_string(), "xyz".to_string()),
        ]);
        assert_eq!(
            query.to_string(),
            "(@name='foo' or @name='bar') and @name='baz' and @name='xyz'"
        );
    }

    #[test]
    fn xpath_and_or_and_parenthesises_both_and_sides() {
        let left = XpathQuery::and_all(vec![
            XpathQuery::Eq("@name".to_string(), "foo".to_string()),
            XpathQuery::Eq("@name".to_string(), "bar".to_string()),
        ]);
        let right = XpathQuery::and_all(vec![
            XpathQuery::Eq("@name".to_string(), "baz".to_string()),
            XpathQuery::Eq("@name".to_string(), "xyz".to_string()),
        ]);
        let query = XpathQuery::Or(Box::new(left), Box::new(right));
        assert_eq!(
            query.to_string(),
            "(@name='foo' and @name='bar') or (@name='baz' and @name='xyz')"
        );
    }

    #[test]
    fn xpath_or_and_or_parenthesises_both_or_sides() {
        let left = XpathQuery::or_all(vec![
            XpathQuery::Eq("@name".to_string(), "foo".to_string()),
            XpathQuery::Eq("@name".to_string(), "bar".to_string()),
        ]);
        let right = XpathQuery::or_all(vec![
            XpathQuery::Eq("@name".to_string(), "baz".to_string()),
            XpathQuery::Eq("@name".to_string(), "xyz".to_string()),
        ]);
        let query = XpathQuery::And(Box::new(left), Box::new(right));
        assert_eq!(
            query.to_string(),
            "(@name='foo' or @name='bar') and (@name='baz' or @name='xyz')"
        );
    }

    #[test]
    fn xpath_multiple_terms_and_join() {
        let query = XpathQuery::and_all(vec![
            XpathQuery::Eq("@name1".to_string(), "foo".to_string()),
            XpathQuery::Eq("@name2".to_string(), "bar".to_string()),
        ]);
        assert_eq!(query.to_string(), "@name1='foo' and @name2='bar'");
    }

    #[test]
    fn xpath_eq_list_or_joins_values() {
        let query = XpathQuery::eq_list(
            "@name",
            &["foo".to_string(), "bar".to_string(), "baz".to_string()],
        );
        assert_eq!(
            query.to_string(),
            "@name='foo' or @name='bar' or @name='baz'"
        );
    }

    #[test]
    fn xpath_not_eq_list_and_joins_nots() {
        let query = XpathQuery::and_all(
            ["foo", "bar", "baz"]
                .iter()
                .map(|value| {
                    XpathQuery::Not(Box::new(XpathQuery::Eq(
                        "@name".to_string(),
                        (*value).to_string(),
                    )))
                })
                .collect(),
        );
        assert_eq!(
            query.to_string(),
            "not(@name='foo') and not(@name='bar') and not(@name='baz')"
        );
    }

    #[test]
    fn xpath_element_predicate_scopes_its_inner_query() {
        let query = XpathQuery::Pred(
            "state".to_string(),
            Box::new(XpathQuery::Eq("@name".to_string(), "new".to_string())),
        );
        assert_eq!(query.to_string(), "state[@name='new']");
    }

    #[test]
    fn xpath_empty_renders_empty_and_is_skipped_by_and_all() {
        assert_eq!(XpathQuery::Empty.to_string(), "");
        let query = XpathQuery::and_all(vec![
            XpathQuery::Empty,
            XpathQuery::Eq("@name".to_string(), "foo".to_string()),
        ]);
        assert_eq!(query.to_string(), "@name='foo'");
        assert_eq!(XpathQuery::and_all(vec![]), XpathQuery::Empty);
    }

    #[test]
    fn search_default_matches_the_open_states() {
        assert_eq!(
            RequestSearch::new().match_expr().unwrap(),
            "state[@name='new' or @name='review' or @name='declined']"
        );
    }

    #[test]
    fn search_request_ids_or_join() {
        let search = search_without_states().request_ids(vec![1, 2]);
        assert_eq!(search.match_expr().unwrap(), "@id='1' or @id='2'");
    }

    #[test]
    fn search_source_projects_match_source_only() {
        let search =
            search_without_states().source_projects(vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "source[@project='foo' or @project='bar']"
        );
    }

    #[test]
    fn search_source_packages_use_the_singular_attribute() {
        // The search engine's whitelist has `source/@package` and
        // no plural form; the plural is refused by the server.
        let search =
            search_without_states().source_packages(vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "source[@package='foo' or @package='bar']"
        );
    }

    #[test]
    fn search_target_projects_match_target_only() {
        let search =
            search_without_states().target_projects(vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "target[@project='foo' or @project='bar']"
        );
    }

    #[test]
    fn search_target_packages_use_the_singular_attribute() {
        // As for source packages: the whitelist has
        // `target/@package`, singular; the plural is refused.
        let search =
            search_without_states().target_packages(vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "target[@package='foo' or @package='bar']"
        );
    }

    #[test]
    fn search_creators_or_join() {
        let search = search_without_states().creators(vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "@creator='foo' or @creator='bar'"
        );
    }

    #[test]
    fn search_who_matches_state_or_history() {
        let search = search_without_states().who(vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "state[@who='foo' or @who='bar'] or history[@who='foo' or @who='bar']"
        );
    }

    #[test]
    fn search_states_match_the_given_states() {
        let search = RequestSearch::new().states(vec![RequestState::New, RequestState::Review]);
        assert_eq!(
            search.match_expr().unwrap(),
            "state[@name='new' or @name='review']"
        );
    }

    #[test]
    fn search_types_match_action_types() {
        let search =
            search_without_states().types(vec!["submit".to_string(), "delete".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "action[@type='submit' or @type='delete']"
        );
    }

    #[test]
    fn search_exclude_target_projects_and_joins_nots() {
        // `target[not(@project='X')]`: no target of the request is
        // project X. The outer form `not(target[@project='X'])` is
        // refused by the server with 400 (verified live against the
        // production API), so this predicate form is the one emitted.
        let search = search_without_states()
            .exclude_target_projects(vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "target[not(@project='foo')] and target[not(@project='bar')]"
        );
    }

    #[test]
    fn search_with_no_filter_is_refused() {
        // No clause at all — not even a state — is refused: the
        // server answers an empty `match=` with 400 (`empty_match`).
        let err = search_without_states().match_expr().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn search_combined_filters_parenthesise_the_or_clause() {
        // An or-clause and-joined onto the states needs parens, or
        // the `and` would bind tighter than the `or` and change the
        // query's meaning.
        let search = RequestSearch::new().projects(vec!["foo".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "(source[@project='foo'] or target[@project='foo']) and state[@name='new' or @name='review' or @name='declined']"
        );
    }

    #[test]
    fn search_rejects_an_invalid_project_name() {
        let search = search_without_states().projects(vec!["foo' or '1'='1".to_string()]);
        let err = search.match_expr().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn search_rejects_a_quoted_creator() {
        let search = search_without_states().creators(vec!["o'brien".to_string()]);
        let err = search.match_expr().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn search_rejects_an_unknown_action_type() {
        let search = search_without_states().types(vec!["explode".to_string()]);
        let err = search.match_expr().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn show_request_reads_a_request_with_its_history() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/request/42?withhistory=1",
            REQUEST1_XML,
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let request = obs.show_request("42").expect("show reads the request");
        assert_eq!(request.id, 42);
        assert_eq!(request.state.name, RequestState::Accepted);
        assert_eq!(request.actions.len(), 2);
        assert_eq!(request.history.len(), 1);
        assert_eq!(request.history[0].comment, "foobar");
        let calls = http.get_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].0,
            "https://api.opensuse.org/request/42?withhistory=1"
        );
        assert!(calls[0].1, "the read is credentialed");
    }

    #[test]
    fn show_request_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let obs = ObsClient::new(&http);
        let err = obs.show_request("42").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn show_request_invalid_id_without_http() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(obs.show_request("0").is_err());
        assert!(obs.show_request("not-an-id").is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn show_request_malformed_response_is_error() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/request/42?withhistory=1",
            "<request this is not xml",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = obs.show_request("42").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn show_request_not_found_maps_to_not_found() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/request/42?withhistory=1",
            ErrorKind::NotFound,
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = obs.show_request("42").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn request_log_returns_only_the_history() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/request/42?withhistory=1",
            REQUEST1_XML,
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let log = obs.request_log("42").expect("log reads the history");
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].description, "Create Request");
        assert_eq!(log[0].comment, "foobar");
        assert_eq!(log[0].who, "creator");
    }

    #[test]
    fn change_state_accept_posts_changestate_with_the_comment() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
            "<status code=\"ok\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        obs.change_state("42", RequestState::Accepted, "looks good", false, &dry)
            .expect("accept posts");
        let calls = http.post_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].0,
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted"
        );
        assert_eq!(calls[0].1, "looks good");
        assert!(calls[0].2, "the change is credentialed");
    }

    #[test]
    fn change_state_force_adds_the_force_param() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted&force=1",
            "<status code=\"ok\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        obs.change_state(
            "42",
            RequestState::Accepted,
            "overriding review",
            true,
            &dry,
        )
        .expect("forced accept posts");
        assert_eq!(
            http.post_calls()[0].0,
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted&force=1"
        );
    }

    #[test]
    fn change_state_refuses_superseded_and_deleted() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let err = obs
            .change_state("42", RequestState::Superseded, "x", false, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let err = obs
            .change_state("42", RequestState::Deleted, "x", false, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn change_state_dry_run_records_the_plan_and_posts_nothing() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        obs.change_state("42", RequestState::Accepted, "looks good", false, &dry)
            .expect("dry-run accept records");
        assert!(http.post_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec!["[dry-run] would: change state of submit request 42 to accepted".to_string()]
        );
    }

    #[test]
    fn change_state_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let obs = ObsClient::new(&http);
        let dry = DryRun::new(false);
        let err = obs
            .change_state("42", RequestState::Accepted, "ok", false, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn change_state_unexpected_response_is_error() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=declined",
            "<status code=\"not_found\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let err = obs
            .change_state("42", RequestState::Declined, "no", false, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn change_state_retries_server_error_then_succeeds() {
        // The server's API replies before its backend has
        // registered a just-filed request, so the changestate POST
        // that follows a filing can 500 while the request is about
        // to exist (live run 44880250442). The library retries
        // a transient failure; the second POST lands. The state
        // check between attempts reads the request back as still
        // new, so the retry goes ahead.
        let http = FakeHttp::new()
            .with_post_error_sequence(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                &[ErrorKind::ServerError],
            )
            .with_post(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                "<status code=\"ok\" />",
            )
            .with_get(
                "https://api.opensuse.org/request/42?withhistory=1",
                &REQUEST1_XML.replace("name=\"accepted\"", "name=\"new\""),
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        obs.change_state("42", RequestState::Accepted, "looks good", false, &dry)
            .expect("accept retries and lands");
        assert_eq!(http.post_calls().len(), 2);
        assert_eq!(http.get_calls().len(), 1);
    }

    #[test]
    fn change_state_does_not_reissue_when_already_accepted() {
        // The 500 can come after the state change landed, so the
        // retry reads the request back first: a request already in
        // the target state means the change went through, and the
        // POST is never issued again.
        let http = FakeHttp::new()
            .with_post_error_sequence(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                &[ErrorKind::ServerError],
            )
            .with_post(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                "<status code=\"ok\" />",
            )
            .with_get(
                "https://api.opensuse.org/request/42?withhistory=1",
                REQUEST1_XML,
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        obs.change_state("42", RequestState::Accepted, "looks good", false, &dry)
            .expect("an already-accepted request reads as success");
        assert_eq!(http.post_calls().len(), 1);
        assert_eq!(http.get_calls().len(), 1);
    }

    #[test]
    fn change_state_persistent_server_error_surfaces_after_the_budget() {
        // A request that never changes state still fails loudly
        // once the budget runs out: with a 400 ms budget the helper
        // tries twice (the first failure, one 500 ms sleep, the
        // second failure past the deadline) and returns the server
        // error instead of hanging. The production budget is 60 s,
        // passed by change_state; the helper takes the budget so
        // this test does not have to wait it out. The state check
        // between attempts has no canned read, so it treats the
        // state as unknown and keeps retrying.
        let http = FakeHttp::new().with_post_error_sequence(
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
            &[
                ErrorKind::ServerError,
                ErrorKind::ServerError,
                ErrorKind::ServerError,
            ],
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = obs
            .post_changestate_with_retry(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                "looks good",
                42,
                RequestState::Accepted,
                std::time::Duration::from_millis(400),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert_eq!(http.post_calls().len(), 2);
    }

    #[test]
    fn change_state_does_not_retry_deterministic_errors() {
        // Only transient failures retry: bad credentials fail fast
        // on the first attempt instead of burning the budget, and
        // no state read runs.
        let http = FakeHttp::new()
            .with_post_error_sequence(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                &[ErrorKind::AuthFailed],
            )
            .with_post(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                "<status code=\"ok\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let err = obs
            .change_state("42", RequestState::Accepted, "looks good", false, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AuthFailed);
        assert_eq!(http.post_calls().len(), 1);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn accept_decline_revoke_post_the_right_newstate() {
        let http = FakeHttp::new()
            .with_post(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                "<status code=\"ok\" />",
            )
            .with_post(
                "https://api.opensuse.org/request/43?cmd=changestate&newstate=declined",
                "<status code=\"ok\" />",
            )
            .with_post(
                "https://api.opensuse.org/request/44?cmd=changestate&newstate=revoked",
                "<status code=\"ok\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        obs.accept_request("42", "landing it", false, &dry)
            .expect("accept posts");
        obs.decline_request("43", "not this way", &dry)
            .expect("decline posts");
        obs.revoke_request("44", "withdrawing", &dry)
            .expect("revoke posts");
        let calls = http.post_calls();
        assert_eq!(calls.len(), 3);
        assert_eq!(
            calls[0].0,
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted"
        );
        assert_eq!(
            calls[1].0,
            "https://api.opensuse.org/request/43?cmd=changestate&newstate=declined"
        );
        assert_eq!(
            calls[2].0,
            "https://api.opensuse.org/request/44?cmd=changestate&newstate=revoked"
        );
    }

    #[test]
    fn try_change_state_true_on_success() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
            "<status code=\"ok\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        assert!(obs
            .try_change_state("42", RequestState::Accepted, "ok", false, &dry)
            .unwrap());
    }

    #[test]
    fn try_change_state_false_on_a_refused_change() {
        // The server refuses the change (403 here): the failure is
        // swallowed with a warning and the call reports false, so a
        // batch triaging many requests does not stop at the first one
        // that cannot move.
        let http = FakeHttp::new().with_post_error(
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
            ErrorKind::AuthFailed,
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        assert!(!obs
            .try_change_state("42", RequestState::Accepted, "ok", false, &dry)
            .unwrap());
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn try_change_state_propagates_a_caller_bug() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let err = obs
            .try_change_state("not-an-id", RequestState::Accepted, "ok", false, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn try_change_state_true_in_dry_run() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        assert!(obs
            .try_change_state("42", RequestState::Accepted, "ok", false, &dry)
            .unwrap());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn add_review_posts_addreview_with_the_reason() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/42?cmd=addreview&by_user=Admin",
            "<status code=\"ok\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let target = ReviewTarget::new(Some("Admin"), None, None, None).unwrap();
        obs.add_review("42", &target, "please review", &dry)
            .expect("review posts");
        let calls = http.post_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].0,
            "https://api.opensuse.org/request/42?cmd=addreview&by_user=Admin"
        );
        assert_eq!(calls[0].1, "please review");
        assert!(calls[0].2, "the review is credentialed");
    }

    #[test]
    fn add_review_by_project_and_package_names_both() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/42?cmd=addreview&by_project=openSUSE%3AFactory&by_package=foo",
            "<status code=\"ok\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let target = ReviewTarget::new(None, None, Some("openSUSE:Factory"), Some("foo")).unwrap();
        obs.add_review("42", &target, "package review", &dry)
            .expect("project/package review posts");
        assert_eq!(
            http.post_calls()[0].0,
            "https://api.opensuse.org/request/42?cmd=addreview&by_project=openSUSE%3AFactory&by_package=foo"
        );
    }

    #[test]
    fn add_review_refuses_an_empty_reason() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let target = ReviewTarget::new(Some("Admin"), None, None, None).unwrap();
        let err = obs.add_review("42", &target, "", &dry).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn review_target_requires_someone_and_a_project_for_a_package() {
        assert!(ReviewTarget::new(None, None, None, None).is_err());
        assert!(ReviewTarget::new(None, None, None, Some("foo")).is_err());
        assert!(ReviewTarget::new(Some(""), None, None, None).is_err());
        assert!(ReviewTarget::new(None, None, Some("not a project!"), None).is_err());
        assert!(ReviewTarget::new(None, None, Some("openSUSE:Factory"), Some("foo")).is_ok());
    }

    #[test]
    fn add_review_dry_run_records_the_plan_and_posts_nothing() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        let target = ReviewTarget::new(Some("Admin"), None, None, None).unwrap();
        obs.add_review("42", &target, "please review", &dry)
            .expect("dry-run review records");
        assert!(http.post_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec!["[dry-run] would: add a review by user Admin to submit request 42".to_string()]
        );
    }

    #[test]
    fn add_review_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let obs = ObsClient::new(&http);
        let dry = DryRun::new(false);
        let target = ReviewTarget::new(Some("Admin"), None, None, None).unwrap();
        let err = obs
            .add_review("42", &target, "please review", &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn add_review_unexpected_response_is_error() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/42?cmd=addreview&by_user=Admin",
            "<status code=\"not_found\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let target = ReviewTarget::new(Some("Admin"), None, None, None).unwrap();
        let err = obs
            .add_review("42", &target, "please review", &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    const COMMENTS_XML: &str = r#"<comments request="42">
  <comment who="user1" when="2010-12-29T10:00:00" id="1">first comment</comment>
  <comment who="user2" when="2010-12-29T11:00:00" id="2" parent="1">a reply</comment>
</comments>"#;

    #[test]
    fn parse_comments_reads_comments_and_replies_in_order() {
        let comments = parse_comments(COMMENTS_XML).expect("comments parse");
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].id, 1);
        assert_eq!(comments[0].who, "user1");
        assert_eq!(comments[0].when, "2010-12-29T10:00:00");
        assert_eq!(comments[0].parent_id, None);
        assert_eq!(comments[0].body, "first comment");
        assert_eq!(comments[1].id, 2);
        assert_eq!(comments[1].who, "user2");
        assert_eq!(comments[1].parent_id, Some(1));
        assert_eq!(comments[1].body, "a reply");
    }

    #[test]
    fn parse_comments_empty_is_empty() {
        assert_eq!(
            parse_comments("<comments request=\"42\"></comments>"),
            Some(Vec::new())
        );
        assert_eq!(
            parse_comments("<comments request=\"42\" />"),
            Some(Vec::new())
        );
    }

    #[test]
    fn parse_comments_rejects_a_malformed_document() {
        assert_eq!(parse_comments("not xml at all"), None);
        assert_eq!(
            parse_comments("<comments request=\"42\"><note>hi</note></comments>"),
            None
        );
        assert_eq!(
            parse_comments(
                "<comments request=\"42\"><comment who=\"user1\">no id</comment></comments>"
            ),
            None
        );
    }

    #[test]
    fn parse_root_strips_only_a_well_formed_xml_prolog() {
        // A well-formed prolog is stripped.
        assert_eq!(
            parse_comments("<?xml version=\"1.0\"?><comments request=\"42\"></comments>"),
            Some(Vec::new())
        );
        // `<?xmlfoo` is not a prolog: the document does not parse.
        assert_eq!(
            parse_comments("<?xmlfoo version=\"1.0\"?><comments request=\"42\"></comments>"),
            None
        );
        // An unterminated prolog is not stripped either.
        assert_eq!(
            parse_comments("<?xml version=\"1.0\"<comments request=\"42\"></comments>"),
            None
        );
    }

    #[test]
    fn list_comments_returns_the_parsed_comments() {
        let http =
            FakeHttp::new().with_get("https://api.opensuse.org/comments/request/42", COMMENTS_XML);
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let comments = obs.list_comments("42").expect("comments list");
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[1].parent_id, Some(1));
        let calls = http.get_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "https://api.opensuse.org/comments/request/42");
        assert!(calls[0].1, "the list is credentialed");
    }

    #[test]
    fn list_comments_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let obs = ObsClient::new(&http);
        let err = obs.list_comments("42").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn list_comments_unexpected_response_is_error() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/comments/request/42",
            "not comments at all",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = obs.list_comments("42").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn add_comment_posts_the_comment_as_plain_text() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/comments/request/42",
            "<status code=\"ok\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        obs.add_comment("42", "looks good to me", None, &dry)
            .expect("comment posts");
        let calls = http.post_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "https://api.opensuse.org/comments/request/42");
        assert_eq!(calls[0].1, "looks good to me");
        assert!(calls[0].2, "the comment is credentialed");
    }

    #[test]
    fn add_comment_reply_names_its_parent() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/comments/request/42?parent_id=1",
            "<status code=\"ok\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        obs.add_comment("42", "agreed", Some("1"), &dry)
            .expect("reply posts");
        assert_eq!(
            http.post_calls()[0].0,
            "https://api.opensuse.org/comments/request/42?parent_id=1"
        );
    }

    #[test]
    fn add_comment_refuses_an_empty_body() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let err = obs.add_comment("42", "", None, &dry).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn add_comment_dry_run_records_the_plan_and_posts_nothing() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        obs.add_comment("42", "looks good to me", None, &dry)
            .expect("dry-run comment records");
        assert!(http.post_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec!["[dry-run] would: comment on submit request 42".to_string()]
        );
    }

    #[test]
    fn add_comment_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let obs = ObsClient::new(&http);
        let dry = DryRun::new(false);
        let err = obs
            .add_comment("42", "looks good to me", None, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn add_comment_unexpected_response_is_error() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/comments/request/42",
            "<status code=\"not_found\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let err = obs
            .add_comment("42", "looks good to me", None, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn request_diff_plain_posts_diff_and_returns_the_body() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/42?cmd=diff",
            "diff --git a/foo b/foo\n",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let diff = obs.request_diff("42", false).expect("diff reads");
        assert_eq!(diff, "diff --git a/foo b/foo\n");
        let calls = http.post_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "https://api.opensuse.org/request/42?cmd=diff");
        assert_eq!(calls[0].1, "");
        assert!(calls[0].2, "the diff read is credentialed");
    }

    #[test]
    fn request_diff_xml_adds_the_view_param() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/42?cmd=diff&view=xml",
            "<request id=\"42\"></request>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let diff = obs.request_diff("42", true).expect("xml diff reads");
        assert_eq!(diff, "<request id=\"42\"></request>");
        assert_eq!(
            http.post_calls()[0].0,
            "https://api.opensuse.org/request/42?cmd=diff&view=xml"
        );
    }

    #[test]
    fn request_diff_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let obs = ObsClient::new(&http);
        let err = obs.request_diff("42", false).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn search_requests_returns_parsed_requests() {
        let collection =
            format!("<collection matches=\"2\">\n{REQUEST1_XML}\n{REQUEST2_XML}\n</collection>");
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/request?match=%28%40id%3D%2742%27%20or%20%40id%3D%27123%27%29%20and%20state%5B%40name%3D%27new%27%20or%20%40name%3D%27review%27%20or%20%40name%3D%27declined%27%5D",
            &collection,
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search = RequestSearch::new().request_ids(vec![42, 123]);
        let requests = obs.search_requests(&search).expect("search parses");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].id, 42);
        assert_eq!(requests[1].id, 123);
        let calls = http.get_calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].1, "the search is credentialed");
    }

    #[test]
    fn search_requests_empty_collection_is_empty() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/request?match=state%5B%40name%3D%27new%27%20or%20%40name%3D%27review%27%20or%20%40name%3D%27declined%27%5D",
            "<collection matches=\"0\"></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let requests = obs
            .search_requests(&RequestSearch::new())
            .expect("empty search parses");
        assert!(requests.is_empty());
    }

    #[test]
    fn search_requests_unexpected_response_is_error() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/request?match=state%5B%40name%3D%27new%27%20or%20%40name%3D%27review%27%20or%20%40name%3D%27declined%27%5D",
            "not a collection at all",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = obs.search_requests(&RequestSearch::new()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn search_requests_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let obs = ObsClient::new(&http);
        let err = obs.search_requests(&RequestSearch::new()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn search_requests_with_history_adds_the_history_param() {
        // The search is the default open-states search with the
        // history flag: an all-empty search is refused (the server
        // answers an empty match with 400), so the flag rides on a
        // real filter.
        let search = RequestSearch::new().with_history(true);
        let url = build_url(
            "https://api.opensuse.org",
            &["search", "request"],
            &[
                ("match", &search.match_expr().unwrap()),
                ("withhistory", "1"),
            ],
        );
        let http = FakeHttp::new().with_get(&url, "<collection matches=\"0\"></collection>");
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let requests = obs.search_requests(&search).expect("history search parses");
        assert!(requests.is_empty());
        assert_eq!(http.get_calls()[0].0, url);
    }

    #[test]
    fn search_requests_with_full_history_adds_the_full_history_param() {
        // As above, with the full-history flag instead.
        let search = RequestSearch::new().with_full_history(true);
        let url = build_url(
            "https://api.opensuse.org",
            &["search", "request"],
            &[
                ("match", &search.match_expr().unwrap()),
                ("withfullhistory", "1"),
            ],
        );
        let http = FakeHttp::new().with_get(&url, "<collection matches=\"0\"></collection>");
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let requests = obs.search_requests(&search).expect("full history parses");
        assert!(requests.is_empty());
        assert_eq!(http.get_calls()[0].0, url);
    }

    #[test]
    fn parse_request_collection_rejects_a_malformed_request() {
        let collection = "<collection matches=\"1\">\n<request creator=\"creator\" id=\"42\">\n  <action type=\"explode\">\n    <target project=\"foo\" />\n  </action>\n  <state name=\"new\" when=\"2010-12-27T01:36:29\" who=\"creator\" />\n</request>\n</collection>";
        assert_eq!(parse_request_collection(collection), None);
    }

    #[test]
    fn parse_request_collection_rejects_a_count_with_no_requests() {
        assert_eq!(
            parse_request_collection("<collection matches=\"2\"></collection>"),
            None
        );
    }

    #[test]
    fn parse_request_collection_rejects_a_short_collection() {
        // The server returns every match in one response (over the
        // limit it refuses with 403 rather than truncating), so a
        // collection carrying fewer requests than its `matches`
        // count is a shape change, not a partial result.
        let collection = format!("<collection matches=\"2\">\n{REQUEST1_XML}\n</collection>");
        assert_eq!(parse_request_collection(&collection), None);
    }

    /// The search URL for an action-type dedup check, built from the
    /// same match expression the client builds: state plus the
    /// action's source/target parts, in production order.
    fn action_match_url(action_type: &str, parts: &[&str]) -> String {
        let match_expr = format!(
            "(state/@name='new' or state/@name='review') and action/@type='{action_type}' and action[{}]",
            parts.join(" and ")
        );
        build_url(
            "https://api.opensuse.org",
            &["search", "request"],
            &[("match", &match_expr)],
        )
    }

    fn change_devel_payload() -> ChangeDevelPayload {
        ChangeDevelPayload::new(
            "devel:languages:python",
            "foo",
            "openSUSE:Factory",
            "foo",
            "Move foo to Factory",
        )
        .expect("test change devel payload")
    }

    fn delete_package_payload() -> DeletePayload {
        DeletePayload::new(
            "openSUSE:Factory",
            DeleteTarget::Package("foo".to_string()),
            "Drop foo",
        )
        .expect("test delete payload")
    }

    fn delete_repository_payload() -> DeletePayload {
        DeletePayload::new(
            "openSUSE:Factory",
            DeleteTarget::Repository("standard".to_string()),
            "Drop the standard repository",
        )
        .expect("test delete payload")
    }

    fn delete_project_payload() -> DeletePayload {
        DeletePayload::new(
            "openSUSE:Factory",
            DeleteTarget::WholeProject,
            "Drop Factory",
        )
        .expect("test delete payload")
    }

    fn incident_payload_with_release_project() -> MaintenanceIncidentPayload {
        incident_payload()
            .with_release_project("openSUSE:Leap:15.6:Update")
            .expect("test incident release project")
    }

    #[test]
    fn file_maintenance_incident_dry_run_returns_none_and_makes_no_requests() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::default();
        let filing = SrFiling::new(&obs, &dry);
        let payload = incident_payload();
        assert_eq!(filing.file_maintenance_incident(&payload).unwrap(), None);
        assert!(http.get_calls().is_empty());
        assert!(http.post_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec![
                "[dry-run] would: file maintenance incident request for home:alice:branches:OBS_Maintained:foo/foo to openSUSE:Maintenance"
                    .to_string(),
                "[dry-run] --- would-be maintenance incident request (dry-run) ---".to_string(),
                "[dry-run] source: home:alice:branches:OBS_Maintained:foo/foo".to_string(),
                "[dry-run] target: openSUSE:Maintenance".to_string(),
                "[dry-run] description: Maintenance incident for foo".to_string(),
                format!("[dry-run] {}", payload.to_xml()),
                "[dry-run] --- end of would-be maintenance incident request ---".to_string(),
            ]
        );
    }

    #[test]
    fn xml_attr_reads_either_quote_style_and_requires_a_standalone_name() {
        assert_eq!(
            xml_attr("project=\"foo\" repository='bar'", "project"),
            Some("foo".to_string())
        );
        assert_eq!(
            xml_attr("project=\"foo\" repository='bar'", "repository"),
            Some("bar".to_string())
        );
        // `valid="..."` must not match a lookup for `id`.
        assert_eq!(
            xml_attr("valid=\"1\" id=\"42\"", "id"),
            Some("42".to_string())
        );
        assert_eq!(xml_attr("valid=\"1\"", "id"), None);
        // Unquoted values are not read.
        assert_eq!(xml_attr("code=failed", "code"), None);
        // Missing attribute is None.
        assert_eq!(xml_attr("code=\"failed\"", "state"), None);
    }

    #[test]
    fn xml_attr_ignores_a_name_inside_an_earlier_quoted_value() {
        // `dirty=` inside the details value must not match: the real
        // `dirty` attribute follows it.
        assert_eq!(
            xml_attr("details=\"x dirty=y\" dirty=\"true\"", "dirty"),
            Some("true".to_string())
        );
        assert_eq!(
            xml_attr("details=\"code=failed\" code=\"building\"", "code"),
            Some("building".to_string())
        );
    }

    #[test]
    fn xml_element_text_reads_an_attributed_open_tag() {
        assert_eq!(
            xml_element_text("<summary lang=\"en\">hello</summary>", "summary").as_deref(),
            Some("hello")
        );
        assert_eq!(xml_element_text("<name attr=\"x\"/>", "name"), None);
        assert_eq!(
            xml_element_text("<namex>no</namex><name>yes</name>", "name").as_deref(),
            Some("yes")
        );
    }

    const RESULTS_FIXTURE: &str = "<resultlist state=\"f1cfebfd13354bcb8e792cb85f6218d8\">\n  <result project=\"testproject\" repository=\"openSUSE_Leap_42.3\" arch=\"x86_64\" code=\"published\" state=\"published\">\n    <status package=\"python-MarkupSafe\" code=\"disabled\" />\n  </result>\n  <result project=\"testproject\" repository=\"openSUSE_Leap_15.0\" arch=\"x86_64\" code=\"published\" state=\"published\">\n    <status package=\"python-MarkupSafe\" code=\"disabled\" />\n  </result>\n  <result project=\"testproject\" repository=\"SLE_12_SP4\" arch=\"x86_64\" code=\"published\" state=\"published\">\n    <status package=\"python-MarkupSafe\" code=\"disabled\" />\n  </result>\n  <result project=\"testproject\" repository=\"SLE_12_SP3\" arch=\"x86_64\" code=\"published\" state=\"published\">\n    <status package=\"python-MarkupSafe\" code=\"disabled\" />\n  </result>\n</resultlist>";

    const RESULTS_DIRTY_FIXTURE: &str = "<resultlist state=\"c57e2ee592dbbf26ebf19cc4f1bc1e83\">\n  <result project=\"testproject\" repository=\"openSUSE_Leap_42.3\" arch=\"x86_64\" code=\"published\" state=\"published\">\n    <status package=\"python-MarkupSafe\" code=\"disabled\" />\n  </result>\n  <result project=\"testproject\" repository=\"openSUSE_Leap_15.0\" arch=\"x86_64\" code=\"broken\" state=\"broken\" details=\"repository 'inheritedproject/openSUSE_Leap_15.0' is unavailable\">\n    <status package=\"python-MarkupSafe\" code=\"disabled\" />\n  </result>\n  <result project=\"testproject\" repository=\"SLE_12_SP4\" arch=\"x86_64\" code=\"publishing\" state=\"publishing\">\n    <status package=\"python-MarkupSafe\" code=\"disabled\" />\n  </result>\n  <result project=\"testproject\" repository=\"SLE_12_SP3\" arch=\"x86_64\" code=\"blocked\" state=\"blocked\" details=\"suspendproject\" dirty=\"true\">\n    <status package=\"python-MarkupSafe\" code=\"disabled\" />\n  </result>\n</resultlist>";

    #[test]
    fn parse_build_results_reads_the_matrix() {
        let parsed = parse_build_results(RESULTS_FIXTURE).unwrap();
        assert_eq!(
            parsed.state.as_deref(),
            Some("f1cfebfd13354bcb8e792cb85f6218d8")
        );
        assert_eq!(parsed.results.len(), 4);
        let first = &parsed.results[0];
        assert_eq!(first.project, "testproject");
        assert_eq!(first.repository, "openSUSE_Leap_42.3");
        assert_eq!(first.arch, "x86_64");
        assert_eq!(first.code, "published");
        assert_eq!(first.state, "published");
        assert!(!first.dirty);
        assert_eq!(first.details, None);
        assert_eq!(first.statuses.len(), 1);
        assert_eq!(first.statuses[0].package, "python-MarkupSafe");
        assert_eq!(first.statuses[0].code, "disabled");
        assert_eq!(first.statuses[0].details, None);
        assert!(results_settled(&parsed, None));
    }

    #[test]
    fn parse_build_results_reads_dirty_and_broken_cells() {
        let parsed = parse_build_results(RESULTS_DIRTY_FIXTURE).unwrap();
        assert_eq!(parsed.results.len(), 4);
        let broken = &parsed.results[1];
        assert_eq!(broken.code, "broken");
        assert_eq!(
            broken.details.as_deref(),
            Some("repository 'inheritedproject/openSUSE_Leap_15.0' is unavailable")
        );
        assert!(!broken.dirty);
        let blocked = &parsed.results[3];
        assert_eq!(blocked.code, "blocked");
        assert_eq!(blocked.details.as_deref(), Some("suspendproject"));
        assert!(blocked.dirty);
        assert!(!results_settled(&parsed, None));
    }

    #[test]
    fn parse_build_results_reads_an_empty_project() {
        let parsed = parse_build_results("<resultlist state=\"abc\"/>").unwrap();
        assert_eq!(parsed.state.as_deref(), Some("abc"));
        assert!(parsed.results.is_empty());
        assert!(results_settled(&parsed, None));
    }

    #[test]
    fn parse_build_results_rejects_malformed_documents() {
        assert!(parse_build_results("").is_err());
        assert!(parse_build_results("<collection></collection>").is_err());
        assert!(parse_build_results("<resultlist state=\"abc\">").is_err());
        assert!(parse_build_results("<resultlist>junk</resultlist>junk").is_err());
        // An unterminated <result> is malformed, not a shorter matrix.
        assert!(parse_build_results(
            "<resultlist><result project=\"p\" repository=\"r\" arch=\"x86_64\" code=\"succeeded\"></resultlist>"
        )
        .is_err());
        // A result without a required attribute is malformed, not
        // skipped: a partial matrix would read as a settled one.
        assert!(parse_build_results(
            "<resultlist><result project=\"p\" repository=\"r\" arch=\"x86_64\"/></resultlist>"
        )
        .is_err());
    }

    #[test]
    fn results_settled_requires_no_dirty_and_no_waiting_code() {
        let settled = BuildResults {
            state: None,
            results: vec![BuildResult {
                project: "p".to_string(),
                repository: "r".to_string(),
                arch: "x86_64".to_string(),
                code: "published".to_string(),
                state: "published".to_string(),
                dirty: false,
                details: None,
                statuses: vec![PackageBuildStatus {
                    package: "pkg".to_string(),
                    code: "succeeded".to_string(),
                    details: None,
                }],
            }],
        };
        assert!(results_settled(&settled, None));
        let mut dirty = settled.clone();
        dirty.results[0].dirty = true;
        assert!(!results_settled(&dirty, None));
        let mut building = settled.clone();
        building.results[0].statuses[0].code = "building".to_string();
        assert!(!results_settled(&building, None));
        let mut scheduled = settled.clone();
        scheduled.results[0].code = "scheduled".to_string();
        assert!(!results_settled(&scheduled, None));
    }

    #[test]
    fn results_settled_requires_the_filtered_package_to_appear() {
        let settled = BuildResults {
            state: None,
            results: vec![BuildResult {
                project: "p".to_string(),
                repository: "r".to_string(),
                arch: "x86_64".to_string(),
                code: "published".to_string(),
                state: "published".to_string(),
                dirty: false,
                details: None,
                statuses: vec![PackageBuildStatus {
                    package: "pkg".to_string(),
                    code: "succeeded".to_string(),
                    details: None,
                }],
            }],
        };
        // The package is present and settled.
        assert!(results_settled(&settled, Some("pkg")));
        // A never-scheduled package drops out of a filtered read:
        // absence is not settled, even when every present cell is.
        assert!(!results_settled(&settled, Some("other")));
        // Without a filter the same matrix is settled.
        assert!(results_settled(&settled, None));
        // An empty matrix is settled without a filter, not with one.
        let empty = BuildResults {
            state: None,
            results: Vec::new(),
        };
        assert!(results_settled(&empty, None));
        assert!(!results_settled(&empty, Some("pkg")));
    }

    #[test]
    fn results_builds_the_per_package_url() {
        let query = ResultsQuery {
            package: Some("python-MarkupSafe".to_string()),
            multibuild: true,
            locallink: true,
            ..ResultsQuery::default()
        };
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/_result?package=python-MarkupSafe&multibuild=1&locallink=1",
            RESULTS_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let parsed = client.results("testproject", &query).unwrap();
        assert_eq!(parsed.results.len(), 4);
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/build/testproject/_result?package=python-MarkupSafe&multibuild=1&locallink=1"
                    .to_string(),
                true
            )]
        );
    }

    #[test]
    fn file_maintenance_incident_live_files_with_add_revision() {
        let search_url = action_match_url(
            "maintenance_incident",
            &[
                "source/@package='foo'",
                "target/@project='openSUSE:Maintenance'",
            ],
        );
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create&addrevision=1",
                "<request id=\"1400001\" creator=\"alice\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let payload = incident_payload();
        assert_eq!(
            filing.file_maintenance_incident(&payload).unwrap(),
            Some("1400001".to_string())
        );
        // The dedup search ran first, then the filing POST — with
        // addrevision=1, which pins source_rev to the filed source.
        assert_eq!(http.get_calls().len(), 1);
        let posted = http.post_calls();
        assert_eq!(posted.len(), 1);
        assert_eq!(
            posted[0].0,
            "https://api.opensuse.org/request?cmd=create&addrevision=1"
        );
        assert_eq!(posted[0].1, payload.to_xml());
    }

    #[test]
    fn file_maintenance_incident_live_refuses_when_open_incident_exists() {
        let search_url = action_match_url(
            "maintenance_incident",
            &[
                "source/@package='foo'",
                "target/@project='openSUSE:Maintenance'",
            ],
        );
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"1399001\" creator=\"alice\"><action type=\"maintenance_incident\"><source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" /><target project=\"openSUSE:Maintenance\" /></action><state name=\"new\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing
            .file_maintenance_incident(&incident_payload())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn file_maintenance_incident_live_refuses_when_open_incident_from_other_branch_exists() {
        // The server's incident identity has no source project, so an
        // equivalent incident filed from another branch blocks too.
        let search_url = action_match_url(
            "maintenance_incident",
            &[
                "source/@package='foo'",
                "target/@project='openSUSE:Maintenance'",
            ],
        );
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"1399001\" creator=\"bob\"><action type=\"maintenance_incident\"><source project=\"home:bob:branches:OBS_Maintained:foo\" package=\"foo\" /><target project=\"openSUSE:Maintenance\" /></action><state name=\"new\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing
            .file_maintenance_incident(&incident_payload())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn file_maintenance_incident_live_files_when_open_incident_has_other_release_project() {
        // The open incident releases elsewhere: the server's incident
        // identity includes the release project, so this is a
        // different incident, not a duplicate.
        let search_url = action_match_url(
            "maintenance_incident",
            &[
                "source/@package='foo'",
                "target/@project='openSUSE:Maintenance'",
            ],
        );
        let http = FakeHttp::new()
            .with_get(
                &search_url,
                "<collection matches=\"1\"><request id=\"1399001\" creator=\"alice\"><action type=\"maintenance_incident\"><source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" /><target project=\"openSUSE:Maintenance\" /></action><state name=\"new\" /></request></collection>",
            )
            .with_post(
                "https://api.opensuse.org/request?cmd=create&addrevision=1",
                "<request id=\"1400001\" creator=\"alice\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing
                .file_maintenance_incident(&incident_payload_with_release_project())
                .unwrap(),
            Some("1400001".to_string())
        );
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn refile_maintenance_incident_live_files_then_supersedes_in_order() {
        let search_url = action_match_url(
            "maintenance_incident",
            &[
                "source/@package='foo'",
                "target/@project='openSUSE:Maintenance'",
            ],
        );
        let http = FakeHttp::new()
            .with_get(
                &search_url,
                "<collection matches=\"1\"><request id=\"1397357\" creator=\"alice\"><action type=\"maintenance_incident\"><source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" /><target project=\"openSUSE:Maintenance\" /></action><state name=\"new\" /></request></collection>",
            )
            .with_post(
                "https://api.opensuse.org/request?cmd=create&addrevision=1",
                "<request id=\"1400001\" creator=\"alice\"></request>",
            )
            .with_post(
                "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1400001",
                "<status code=\"ok\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing
                .refile_maintenance_incident(&incident_payload(), "1397357")
                .unwrap(),
            Some("1400001".to_string())
        );
        // The old incident is open for the same source package,
        // but the refile exempts it: dedup first, then the filing
        // POST, then the supersede POST.
        assert_eq!(http.get_calls().len(), 1);
        let posted = http.post_calls();
        assert_eq!(posted.len(), 2);
        assert_eq!(
            posted[0].0,
            "https://api.opensuse.org/request?cmd=create&addrevision=1"
        );
        assert_eq!(
            posted[1].0,
            "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1400001"
        );
    }

    #[test]
    fn refile_maintenance_incident_dry_run_returns_none_and_records_both_steps() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::default();
        let filing = SrFiling::new(&obs, &dry);
        let payload = incident_payload();
        assert_eq!(
            filing
                .refile_maintenance_incident(&payload, "1397357")
                .unwrap(),
            None
        );
        assert!(http.get_calls().is_empty());
        assert!(http.post_calls().is_empty());
        assert!(dry.plan().iter().any(|line| line
            == "[dry-run] would: file maintenance incident request for home:alice:branches:OBS_Maintained:foo/foo to openSUSE:Maintenance"));
        assert!(dry.plan().iter().any(|line| line
            == "[dry-run] would: supersede maintenance incident request 1397357 with the new request"));
    }

    #[test]
    fn file_change_devel_live_returns_id() {
        let search_url = action_match_url(
            "change_devel",
            &[
                "target/@project='openSUSE:Factory'",
                "target/@package='foo'",
            ],
        );
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"alice\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing.file_change_devel(&change_devel_payload()).unwrap(),
            Some("1400001".to_string())
        );
        assert_eq!(http.get_calls().len(), 1);
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn file_change_devel_live_refuses_when_open_request_exists() {
        let search_url = action_match_url(
            "change_devel",
            &[
                "target/@project='openSUSE:Factory'",
                "target/@package='foo'",
            ],
        );
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"1399002\" creator=\"alice\"><action type=\"change_devel\"><source project=\"devel:languages:python\" package=\"foo\" /><target project=\"openSUSE:Factory\" package=\"foo\" /></action><state name=\"review\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing
            .file_change_devel(&change_devel_payload())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn file_change_devel_live_refuses_when_open_request_names_other_source() {
        // The server's change-devel identity is the target alone, so
        // an open request naming another new devel blocks too.
        let search_url = action_match_url(
            "change_devel",
            &[
                "target/@project='openSUSE:Factory'",
                "target/@package='foo'",
            ],
        );
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"1399002\" creator=\"alice\"><action type=\"change_devel\"><source project=\"devel:languages:go\" package=\"foo\" /><target project=\"openSUSE:Factory\" package=\"foo\" /></action><state name=\"review\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing
            .file_change_devel(&change_devel_payload())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn file_delete_request_live_returns_id_for_package() {
        let search_url = action_match_url("delete", &["target/@project='openSUSE:Factory'"]);
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"alice\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing
                .file_delete_request(&delete_package_payload())
                .unwrap(),
            Some("1400001".to_string())
        );
        assert_eq!(http.get_calls().len(), 1);
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn file_delete_request_live_refuses_when_open_delete_exists() {
        let search_url = action_match_url("delete", &["target/@project='openSUSE:Factory'"]);
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"1399003\" creator=\"alice\"><action type=\"delete\"><target project=\"openSUSE:Factory\" package=\"foo\" /></action><state name=\"new\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing
            .file_delete_request(&delete_package_payload())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn file_delete_request_live_files_repository_target() {
        // A repository delete has no package in the target: the
        // dedup match carries only the target project, and the parser
        // narrows by shape instead.
        let search_url = action_match_url("delete", &["target/@project='openSUSE:Factory'"]);
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"alice\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing
                .file_delete_request(&delete_repository_payload())
                .unwrap(),
            Some("1400001".to_string())
        );
        assert_eq!(http.get_calls().len(), 1);
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn file_delete_request_live_refuses_when_open_whole_project_delete_exists() {
        // An open whole-project delete covers every package in the
        // project: filing a package delete under it is refused.
        let search_url = action_match_url("delete", &["target/@project='openSUSE:Factory'"]);
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"1399004\" creator=\"alice\"><action type=\"delete\"><target project=\"openSUSE:Factory\" /></action><state name=\"new\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing
            .file_delete_request(&delete_package_payload())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.context.contains("1399004"));
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn file_delete_request_live_files_whole_project_when_open_package_delete_exists() {
        // The reverse is not a conflict: an open package delete does
        // not block deleting the whole project.
        let search_url = action_match_url("delete", &["target/@project='openSUSE:Factory'"]);
        let http = FakeHttp::new()
            .with_get(
                &search_url,
                "<collection matches=\"1\"><request id=\"1399004\" creator=\"alice\"><action type=\"delete\"><target project=\"openSUSE:Factory\" package=\"foo\" /></action><state name=\"new\" /></request></collection>",
            )
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"alice\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing
                .file_delete_request(&delete_project_payload())
                .unwrap(),
            Some("1400001".to_string())
        );
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn file_delete_request_live_refuses_whole_project_when_open_repository_delete_exists() {
        // A repository delete and a whole-project delete share the
        // server's delete identity, so the open repository delete
        // blocks deleting the whole project.
        let search_url = action_match_url("delete", &["target/@project='openSUSE:Factory'"]);
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"1399004\" creator=\"alice\"><action type=\"delete\"><target project=\"openSUSE:Factory\" repository=\"standard\" /></action><state name=\"new\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing
            .file_delete_request(&delete_project_payload())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn create_request_with_add_revision_posts_add_revision() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request?cmd=create&addrevision=1",
            "<request id=\"1400001\" creator=\"alice\"></request>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .create_request_with_add_revision("<request></request>")
                .unwrap(),
            "1400001".to_string()
        );
    }

    #[test]
    fn maintenance_branch_options_reject_invalid_values() {
        assert!(MaintenanceBranchOptions::new()
            .with_attribute("no-colon")
            .is_err());
        assert!(MaintenanceBranchOptions::new()
            .with_attribute("OBS:")
            .is_err());
        assert!(MaintenanceBranchOptions::new()
            .with_attribute(":Maintained")
            .is_err());
        assert!(MaintenanceBranchOptions::new()
            .with_update_project_attribute("a:b:c")
            .is_err());
        assert!(MaintenanceBranchOptions::new()
            .with_target_project("foo/bar")
            .is_err());
        // The defaults are the server defaults.
        let options = MaintenanceBranchOptions::new();
        assert_eq!(options.attribute, "OBS:Maintained");
        assert_eq!(options.update_project_attribute, "OBS:UpdateProject");
        assert_eq!(options.target_project, None);
    }

    #[test]
    fn results_builds_the_project_url_without_filters() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/_result",
            RESULTS_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let parsed = client
            .results("testproject", &ResultsQuery::default())
            .unwrap();
        assert_eq!(parsed.results.len(), 4);
    }

    #[test]
    fn results_builds_repeated_repository_and_arch_pairs() {
        let query = ResultsQuery {
            repositories: vec!["standard".to_string(), "ports".to_string()],
            arches: vec!["x86_64".to_string(), "aarch64".to_string()],
            code: Some("failed".to_string()),
            lastbuild: true,
            ..ResultsQuery::default()
        };
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/_result?repository=standard&repository=ports&arch=x86_64&arch=aarch64&code=failed&lastbuild=1",
            "<resultlist state=\"abc\"/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let parsed = client.results("testproject", &query).unwrap();
        assert!(parsed.results.is_empty());
    }

    #[test]
    fn results_fails_closed_without_credentials() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/_result",
            RESULTS_FIXTURE,
        );
        let client = ObsClient::new(&http);
        let err = client
            .results("testproject", &ResultsQuery::default())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn results_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .results("bad/project", &ResultsQuery::default())
            .is_err());
        let query = ResultsQuery {
            package: Some("bad/package".to_string()),
            ..ResultsQuery::default()
        };
        assert!(client.results("testproject", &query).is_err());
        let query = ResultsQuery {
            repositories: vec!["bad repo".to_string()],
            ..ResultsQuery::default()
        };
        assert!(client.results("testproject", &query).is_err());
        let query = ResultsQuery {
            oldstate: Some(String::new()),
            ..ResultsQuery::default()
        };
        assert!(client.results("testproject", &query).is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn results_404_is_an_error_not_an_empty_matrix() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/build/nosuchproject/_result",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .results("nosuchproject", &ResultsQuery::default())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn wait_for_results_polls_plain_until_settled() {
        let http = FakeHttp::new().with_get_sequence(
            "https://api.opensuse.org/build/testproject/_result",
            &[RESULTS_DIRTY_FIXTURE, RESULTS_FIXTURE],
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let parsed = client
            .wait_for_results(
                "testproject",
                &ResultsQuery::default(),
                std::time::Duration::from_secs(30),
            )
            .unwrap();
        assert_eq!(parsed.results.len(), 4);
        assert!(results_settled(&parsed, None));
        assert_eq!(
            http.get_calls(),
            vec![
                (
                    "https://api.opensuse.org/build/testproject/_result".to_string(),
                    true
                ),
                (
                    "https://api.opensuse.org/build/testproject/_result".to_string(),
                    true
                ),
            ]
        );
    }

    #[test]
    fn debug_change_state_retry_reports_probe_and_decision() {
        // The changestate POST 500s, the state check reads the
        // request back as still new, and the retry goes ahead: one
        // probe line and one accept-state decision line, in time
        // order.
        let (debug, lines) = Debug::captured(true, false);
        let http = FakeHttp::new()
            .with_post_error_sequence(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                &[ErrorKind::ServerError],
            )
            .with_post(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                "<status code=\"ok\" />",
            )
            .with_get(
                "https://api.opensuse.org/request/42?withhistory=1",
                &REQUEST1_XML.replace("name=\"accepted\"", "name=\"new\""),
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials())
            .unwrap()
            .with_debug(debug);
        let dry = DryRun::new(false);
        obs.change_state("42", RequestState::Accepted, "looks good", false, &dry)
            .expect("accept retries and lands");
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            [
                "retry: change_state request 42 probe 1/60s: still new".to_string(),
                "decision: request 42 still new, re-issuing accepted".to_string(),
            ]
        );
    }

    #[test]
    fn wait_for_results_ignores_a_caller_set_oldstate() {
        let query = ResultsQuery {
            oldstate: Some("c57e2ee592dbbf26ebf19cc4f1bc1e83".to_string()),
            ..ResultsQuery::default()
        };
        let http = FakeHttp::new().with_get_sequence(
            "https://api.opensuse.org/build/testproject/_result",
            &[RESULTS_DIRTY_FIXTURE, RESULTS_FIXTURE],
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let parsed = client
            .wait_for_results("testproject", &query, std::time::Duration::from_secs(30))
            .unwrap();
        assert!(results_settled(&parsed, None));
        assert_eq!(
            http.get_calls(),
            vec![
                (
                    "https://api.opensuse.org/build/testproject/_result".to_string(),
                    true
                ),
                (
                    "https://api.opensuse.org/build/testproject/_result".to_string(),
                    true
                ),
            ]
        );
    }

    #[test]
    fn debug_change_state_already_accepted_reports_the_decision() {
        let (debug, lines) = Debug::captured(true, false);
        let http = FakeHttp::new()
            .with_post_error_sequence(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                &[ErrorKind::ServerError],
            )
            .with_post(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                "<status code=\"ok\" />",
            )
            .with_get(
                "https://api.opensuse.org/request/42?withhistory=1",
                REQUEST1_XML,
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials())
            .unwrap()
            .with_debug(debug);
        let dry = DryRun::new(false);
        obs.change_state("42", RequestState::Accepted, "looks good", false, &dry)
            .expect("an already-accepted request reads as success");
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            ["decision: request 42 already accepted; change landed, not re-issuing".to_string()]
        );
    }

    #[test]
    fn wait_for_results_returns_immediately_when_settled() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/_result",
            RESULTS_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let parsed = client
            .wait_for_results(
                "testproject",
                &ResultsQuery::default(),
                std::time::Duration::from_secs(30),
            )
            .unwrap();
        assert!(results_settled(&parsed, None));
        assert_eq!(http.get_calls().len(), 1);
    }

    #[test]
    fn wait_for_results_times_out_on_a_never_settling_build() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/_result",
            RESULTS_DIRTY_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .wait_for_results(
                "testproject",
                &ResultsQuery::default(),
                std::time::Duration::ZERO,
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Timeout);
        assert_eq!(http.get_calls().len(), 1);
    }

    #[test]
    fn wait_for_results_polls_a_package_filtered_wait_until_the_package_appears() {
        // First poll: settled cells, but the filtered package is
        // absent (never scheduled) — not settled. Second poll: the
        // package appears, succeeded — settled.
        let http = FakeHttp::new().with_get_sequence(
            "https://api.opensuse.org/build/testproject/_result?package=foo",
            &[
                "<resultlist state=\"abc\"><result project=\"testproject\" repository=\"standard\" arch=\"x86_64\" code=\"published\" state=\"published\"></result></resultlist>",
                "<resultlist state=\"abc\"><result project=\"testproject\" repository=\"standard\" arch=\"x86_64\" code=\"published\" state=\"published\"><status package=\"foo\" code=\"succeeded\"/></result></resultlist>",
            ],
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let query = ResultsQuery {
            package: Some("foo".to_string()),
            ..ResultsQuery::default()
        };
        let parsed = client
            .wait_for_results("testproject", &query, std::time::Duration::from_secs(30))
            .unwrap();
        assert_eq!(parsed.results.len(), 1);
        assert_eq!(parsed.results[0].statuses.len(), 1);
        assert_eq!(parsed.results[0].statuses[0].package, "foo");
        assert_eq!(http.get_calls().len(), 2);
    }

    #[test]
    fn wait_for_results_times_out_on_a_never_scheduled_filtered_package() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/_result?package=foo",
            "<resultlist state=\"abc\"><result project=\"testproject\" repository=\"standard\" arch=\"x86_64\" code=\"published\" state=\"published\"></result></resultlist>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let query = ResultsQuery {
            package: Some("foo".to_string()),
            ..ResultsQuery::default()
        };
        let err = client
            .wait_for_results("testproject", &query, std::time::Duration::ZERO)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Timeout);
        assert_eq!(http.get_calls().len(), 1);
    }

    #[test]
    fn parse_job_status_reads_a_running_job() {
        let parsed = parse_job_status(
            "<jobstatus code=\"building\" details=\"building on sheep87:16\"><starttime>1618222900</starttime><lastduration>43</lastduration><workerid>sheep87:16</workerid><hostarch>x86_64</hostarch><uri>http://172.18.0.5:38963</uri><arch>x86_64</arch><job>home:alice::foo::1</job><jobid>17c8d25b92da67f6ec428ea51a9b5342</jobid><attempt>1</attempt></jobstatus>",
        )
        .unwrap();
        assert_eq!(
            parsed,
            Some(JobStatus {
                code: "building".to_string(),
                result: None,
                details: Some("building on sheep87:16".to_string()),
                starttime: Some(1_618_222_900),
                endtime: None,
                lastduration: Some(43),
                workerid: Some("sheep87:16".to_string()),
                hostarch: Some("x86_64".to_string()),
                uri: Some("http://172.18.0.5:38963".to_string()),
                arch: Some("x86_64".to_string()),
                job: Some("home:alice::foo::1".to_string()),
                jobid: Some("17c8d25b92da67f6ec428ea51a9b5342".to_string()),
                attempt: Some(1),
            })
        );
    }

    #[test]
    fn parse_job_status_empty_means_no_job_running() {
        assert_eq!(parse_job_status("<jobstatus/>").unwrap(), None);
        assert_eq!(parse_job_status("<jobstatus></jobstatus>").unwrap(), None);
        assert_eq!(parse_job_status("<jobstatus code=\"\"/>").unwrap(), None);
    }

    #[test]
    fn parse_job_status_rejects_malformed_documents() {
        assert!(parse_job_status("").is_err());
        assert!(parse_job_status("<collection></collection>").is_err());
        assert!(parse_job_status("<jobstatus code=\"building\"").is_err());
        assert!(parse_job_status(
            "<jobstatus code=\"building\"><starttime>soon</starttime></jobstatus>"
        )
        .is_err());
        assert!(parse_job_status("<jobstatus code=\"building\"><starttime>1</starttime>").is_err());
        assert!(parse_job_status("junk<jobstatus/>").is_err());
        assert!(parse_job_status("<jobstatus/>junk").is_err());
        assert!(parse_job_status("<jobstatus code=\"building\"></jobstatus>junk").is_err());
        assert!(parse_job_status("<jobstatusx/>").is_err());
    }

    #[test]
    fn parse_build_status_reads_the_code() {
        assert_eq!(
            parse_build_status("<status package=\"foo\" code=\"failed\"/>").unwrap(),
            "failed"
        );
        assert_eq!(
            parse_build_status("<status code=\"succeeded\" package=\"foo\"></status>").unwrap(),
            "succeeded"
        );
    }

    #[test]
    fn parse_build_status_rejects_a_code_less_document() {
        assert!(parse_build_status("").is_err());
        assert!(parse_build_status("<status package=\"foo\"/>").is_err());
        assert!(parse_build_status("<status package=\"foo\" code=\"\"/>").is_err());
        assert!(parse_build_status("<collection></collection>").is_err());
        assert!(parse_build_status("junk<status code=\"failed\"/>").is_err());
        assert!(parse_build_status("<status code=\"failed\"/>junk").is_err());
        assert!(parse_build_status("<statusx code=\"failed\"/>").is_err());
    }

    #[test]
    fn build_log_builds_the_url_and_returns_the_text() {
        let query = BuildLogQuery {
            start: Some(100),
            last: true,
            ..BuildLogQuery::default()
        };
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_log?nostream=1&start=100&last=1",
            "build log text",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .build_log("testproject", "foo", "standard", "x86_64", &query)
                .unwrap(),
            Some("build log text".to_string())
        );
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_log?nostream=1&start=100&last=1"
                    .to_string(),
                true
            )]
        );
    }

    #[test]
    fn build_log_keeps_the_multibuild_flavor_in_the_path() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo:flavor1/_log?nostream=1&lastsucceeded=1",
            "flavor log",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let query = BuildLogQuery {
            lastsucceeded: true,
            ..BuildLogQuery::default()
        };
        assert_eq!(
            client
                .build_log("testproject", "foo:flavor1", "standard", "x86_64", &query)
                .unwrap(),
            Some("flavor log".to_string())
        );
    }

    #[test]
    fn build_log_404_is_none_not_an_error() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_log?nostream=1",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .build_log(
                    "testproject",
                    "foo",
                    "standard",
                    "x86_64",
                    &BuildLogQuery::default()
                )
                .unwrap(),
            None
        );
    }

    #[test]
    fn build_log_fails_closed_without_credentials() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let err = client
            .build_log(
                "testproject",
                "foo",
                "standard",
                "x86_64",
                &BuildLogQuery::default(),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn build_status_returns_the_code_and_none_on_404() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_status",
                "<status package=\"foo\" code=\"failed\"/>",
            )
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/baz/_status",
                "<status package=\"baz\" code=\"unknown\"/>",
            )
            .with_get_error(
                "https://api.opensuse.org/build/testproject/standard/x86_64/bar/_status",
                ErrorKind::NotFound,
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .build_status("testproject", "foo", "standard", "x86_64")
                .unwrap(),
            Some("failed".to_string())
        );
        assert_eq!(
            client
                .build_status("testproject", "bar", "standard", "x86_64")
                .unwrap(),
            None
        );
        assert_eq!(
            client
                .build_status("testproject", "baz", "standard", "x86_64")
                .unwrap(),
            None
        );
    }

    #[test]
    fn build_status_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .build_status("testproject", "bad/package", "standard", "x86_64")
            .is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn job_status_returns_the_job_and_none_when_idle() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_jobstatus",
                "<jobstatus code=\"building\"><starttime>1618222900</starttime></jobstatus>",
            )
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/bar/_jobstatus",
                "<jobstatus/>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let job = client
            .job_status("testproject", "foo", "standard", "x86_64")
            .unwrap();
        assert_eq!(job.as_ref().unwrap().code, "building");
        assert_eq!(job.as_ref().unwrap().starttime, Some(1_618_222_900));
        assert_eq!(
            client
                .job_status("testproject", "bar", "standard", "x86_64")
                .unwrap(),
            None
        );
    }

    #[test]
    fn job_status_404_is_an_error_not_none() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/build/testproject/standard/x86_64/nosuch/_jobstatus",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .job_status("testproject", "nosuch", "standard", "x86_64")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn parse_buildinfo_reads_the_error_and_the_dependencies() {
        let parsed = parse_buildinfo(
            "<buildinfo project=\"p\" repository=\"r\" package=\"foo\" arch=\"x86_64\"><error>unresolvable: DOES-NOT-EXIST</error><bdep name=\"gcc\"/><bdep name=\"make\"/></buildinfo>",
        )
        .unwrap();
        assert_eq!(
            parsed.error.as_deref(),
            Some("unresolvable: DOES-NOT-EXIST")
        );
        assert_eq!(parsed.bdeps, vec!["gcc".to_string(), "make".to_string()]);
    }

    #[test]
    fn parse_buildinfo_without_an_error_is_empty() {
        let parsed = parse_buildinfo("<buildinfo project=\"p\"/>").unwrap();
        assert_eq!(parsed.error, None);
        assert!(parsed.bdeps.is_empty());
        let parsed = parse_buildinfo("<buildinfo/>").unwrap();
        assert_eq!(parsed.error, None);
        assert!(parsed.bdeps.is_empty());
    }

    #[test]
    fn parse_buildinfo_rejects_malformed_documents() {
        assert!(parse_buildinfo("").is_err());
        assert!(parse_buildinfo("<collection></collection>").is_err());
        assert!(parse_buildinfo("<buildinfo>").is_err());
        assert!(parse_buildinfo("<buildinfo><bdep version=\"1\"/></buildinfo>").is_err());
    }

    #[test]
    fn parse_build_history_reads_entries() {
        let parsed = parse_build_history(
            "<buildhistory><entry rev=\"2\" srcmd5=\"def\" versrel=\"1.0-1\" bcnt=\"1\" time=\"1699900000\"/><entry rev=\"3\" srcmd5=\"abc\" versrel=\"1.0-1\" bcnt=\"2\" time=\"1700000000\" duration=\"120\"/></buildhistory>",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].rev, "3");
        assert_eq!(parsed[0].srcmd5, "abc");
        assert_eq!(parsed[0].versrel, "1.0-1");
        assert_eq!(parsed[0].bcnt, 2);
        assert_eq!(parsed[0].time, 1_700_000_000);
        assert_eq!(parsed[0].duration, Some(120));
        assert_eq!(parsed[1].duration, None);
    }

    #[test]
    fn parse_build_history_empty_is_an_empty_list() {
        assert_eq!(
            parse_build_history("<buildhistory/>").unwrap(),
            Vec::<BuildHistoryEntry>::new()
        );
        assert_eq!(
            parse_build_history("<buildhistory></buildhistory>").unwrap(),
            Vec::<BuildHistoryEntry>::new()
        );
    }

    #[test]
    fn parse_build_history_rejects_malformed_documents() {
        assert!(parse_build_history("").is_err());
        assert!(parse_build_history("<collection></collection>").is_err());
        assert!(
            parse_build_history(
                "<buildhistory><entry rev=\"3\" srcmd5=\"abc\" versrel=\"1.0-1\" bcnt=\"x\" time=\"1\"/></buildhistory>"
            )
            .is_err()
        );
        assert!(
            parse_build_history(
                "<buildhistory><entry srcmd5=\"abc\" versrel=\"1.0-1\" bcnt=\"1\" time=\"1\"/></buildhistory>"
            )
            .is_err()
        );
    }

    #[test]
    fn parse_job_history_reads_entries() {
        let parsed = parse_job_history(
            "<jobhistlist><jobhist repository=\"standard\" arch=\"x86_64\" package=\"foo\" rev=\"3\" srcmd5=\"abc\" versrel=\"1.0-1\" bcnt=\"2\" readytime=\"1700000000\" starttime=\"1700000060\" endtime=\"1700000120\" code=\"succeeded\" uri=\"http://worker:3000\" workerid=\"w1\" hostarch=\"x86_64\" reason=\"source change\" verifymd5=\"abc\"/></jobhistlist>",
        )
        .unwrap();
        assert_eq!(parsed.len(), 1);
        let entry = &parsed[0];
        assert_eq!(entry.repository.as_deref(), Some("standard"));
        assert_eq!(entry.package, "foo");
        assert_eq!(entry.code, "succeeded");
        assert_eq!(entry.bcnt, Some(2));
        assert_eq!(entry.readytime, Some(1_700_000_000));
        assert_eq!(entry.starttime, Some(1_700_000_060));
        assert_eq!(entry.endtime, Some(1_700_000_120));
        assert_eq!(entry.reason.as_deref(), Some("source change"));
    }

    #[test]
    fn parse_job_history_reads_newest_first() {
        let parsed = parse_job_history(
            "<jobhistlist><jobhist package=\"foo\" code=\"failed\" endtime=\"1700000000\"/><jobhist package=\"foo\" code=\"succeeded\" endtime=\"1700000120\"/></jobhistlist>",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].code, "succeeded");
        assert_eq!(parsed[1].code, "failed");
    }

    #[test]
    fn parse_job_history_minimal_entry_is_enough() {
        let parsed = parse_job_history(
            "<jobhistlist><jobhist package=\"foo\" code=\"failed\"/></jobhistlist>",
        )
        .unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].package, "foo");
        assert_eq!(parsed[0].code, "failed");
        assert_eq!(parsed[0].bcnt, None);
    }

    #[test]
    fn parse_job_history_rejects_malformed_documents() {
        assert!(parse_job_history("").is_err());
        assert!(parse_job_history("<collection></collection>").is_err());
        assert!(
            parse_job_history("<jobhistlist><jobhist code=\"failed\"/></jobhistlist>").is_err()
        );
        assert!(
            parse_job_history("<jobhistlist><jobhist package=\"foo\"/></jobhistlist>").is_err()
        );
        assert!(
            parse_job_history(
                "<jobhistlist><jobhist package=\"foo\" code=\"failed\" starttime=\"soon\"/></jobhistlist>"
            )
            .is_err()
        );
    }

    #[test]
    fn buildinfo_returns_the_error_and_dependencies() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_buildinfo",
            "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"x86_64\"><error>unresolvable: DOES-NOT-EXIST</error><bdep name=\"gcc\"/></buildinfo>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let info = client
            .buildinfo("testproject", "foo", "standard", "x86_64")
            .unwrap();
        assert_eq!(info.error.as_deref(), Some("unresolvable: DOES-NOT-EXIST"));
        assert_eq!(info.bdeps, vec!["gcc".to_string()]);
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_buildinfo"
                    .to_string(),
                true
            )]
        );
    }

    #[test]
    fn buildinfo_fails_closed_without_credentials() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let err = client
            .buildinfo("testproject", "foo", "standard", "x86_64")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn build_history_builds_the_url_with_and_without_a_limit() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_history?limit=1",
                "<buildhistory><entry rev=\"3\" srcmd5=\"abc\" versrel=\"1.0-1\" bcnt=\"2\" time=\"1700000000\"/></buildhistory>",
            )
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_history",
                "<buildhistory/>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let entries = client
            .build_history("testproject", "foo", "standard", "x86_64", Some(1))
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].rev, "3");
        let entries = client
            .build_history("testproject", "foo", "standard", "x86_64", None)
            .unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn build_history_404_is_an_error_not_an_empty_list() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/build/testproject/standard/x86_64/nosuch/_history",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .build_history("testproject", "nosuch", "standard", "x86_64", None)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn job_history_builds_the_url_with_filters() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/x86_64/_jobhistory?package=foo&limit=10&code=succeeded",
            "<jobhistlist><jobhist package=\"foo\" code=\"succeeded\"/></jobhistlist>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let entries = client
            .job_history(
                "testproject",
                "standard",
                "x86_64",
                Some("foo"),
                Some(10),
                Some("succeeded"),
            )
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].code, "succeeded");
    }

    #[test]
    fn job_history_builds_the_url_without_filters() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/x86_64/_jobhistory",
            "<jobhistlist/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let entries = client
            .job_history("testproject", "standard", "x86_64", None, None, None)
            .unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn job_history_rejects_an_empty_code_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .job_history("testproject", "standard", "x86_64", None, None, Some(""))
            .is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn job_history_rejects_lastfailures_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .job_history(
                "testproject",
                "standard",
                "x86_64",
                None,
                None,
                Some("lastfailures")
            )
            .is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn parse_binary_list_reads_the_whole_build_output() {
        let parsed = parse_binary_list(
            "<binarylist><binary filename=\"foo-1.0-1.x86_64.rpm\" size=\"58352\" mtime=\"1617270174\"/><binary filename=\"foo-1.0-1.src.rpm\" size=\"1234\" mtime=\"1617270174\"/><binary filename=\"foo-debuginfo-1.0-1.x86_64.rpm\" size=\"999\" mtime=\"1617270174\"/><binary filename=\"_buildenv\" size=\"10\" mtime=\"1617270174\"/><binary filename=\"_statistics\" size=\"20\" mtime=\"1617270174\"/><binary filename=\"rpmlint.log\" size=\"30\" mtime=\"1617270174\"/></binarylist>",
        )
        .unwrap();
        assert_eq!(parsed.len(), 6);
        assert_eq!(parsed[0].name, "foo-1.0-1.x86_64.rpm");
        assert_eq!(parsed[0].size, Some(58_352));
        assert_eq!(parsed[0].mtime, Some(1_617_270_174));
        assert_eq!(parsed[1].name, "foo-1.0-1.src.rpm");
        assert_eq!(parsed[3].name, "_buildenv");
        assert_eq!(parsed[5].name, "rpmlint.log");
    }

    #[test]
    fn parse_binary_list_empty_is_an_empty_list() {
        assert_eq!(
            parse_binary_list("<binarylist/>").unwrap(),
            Vec::<BinaryFile>::new()
        );
        assert_eq!(
            parse_binary_list("<binarylist></binarylist>").unwrap(),
            Vec::<BinaryFile>::new()
        );
    }

    #[test]
    fn parse_binary_list_rejects_malformed_documents() {
        assert!(parse_binary_list("").is_err());
        assert!(parse_binary_list("<directory></directory>").is_err());
        assert!(parse_binary_list("<binarylist><binary size=\"1\"/></binarylist>").is_err());
        assert!(parse_binary_list(
            "<binarylist><binary filename=\"x\" size=\"big\"/></binarylist>"
        )
        .is_err());
    }

    #[test]
    fn parse_fileinfo_reads_the_plain_view() {
        let parsed = parse_fileinfo(
            "<fileinfo filename=\"foo-1.0-1.x86_64.rpm\"><name>foo</name><epoch>1</epoch><version>1.0</version><release>1</release><arch>x86_64</arch><summary>Foo tool</summary><description>Does foo.</description><disturl>obs://build.opensuse.org/openSUSE:Factory/standard/abc-foo-1.0-1.x86_64.rpm</disturl><size>58352</size><mtime>1617270174</mtime><provides>foo = 1.0-1</provides><provides>libc.so.6()(64bit)</provides><requires>libc.so.6()(64bit)</requires><prerequires>/bin/sh</prerequires><conflicts>oldfoo</conflicts><obsoletes>oldfoo</obsoletes><recommends>foo-doc</recommends><supplements>foo</supplements><suggests>foo-extra</suggests><enhances>foo-base</enhances></fileinfo>",
        )
        .unwrap();
        assert_eq!(parsed.filename, "foo-1.0-1.x86_64.rpm");
        assert_eq!(parsed.name.as_deref(), Some("foo"));
        assert_eq!(parsed.version.as_deref(), Some("1.0"));
        assert_eq!(parsed.release.as_deref(), Some("1"));
        assert_eq!(parsed.arch.as_deref(), Some("x86_64"));
        assert_eq!(parsed.summary.as_deref(), Some("Foo tool"));
        assert_eq!(parsed.size, Some(58_352));
        assert_eq!(parsed.mtime, Some(1_617_270_174));
        assert_eq!(
            parsed.provides,
            vec!["foo = 1.0-1".to_string(), "libc.so.6()(64bit)".to_string()]
        );
        assert_eq!(parsed.requires, vec!["libc.so.6()(64bit)".to_string()]);
        assert_eq!(parsed.epoch, Some(1));
        assert_eq!(
            parsed.disturl.as_deref(),
            Some("obs://build.opensuse.org/openSUSE:Factory/standard/abc-foo-1.0-1.x86_64.rpm")
        );
        assert_eq!(parsed.prerequires, vec!["/bin/sh".to_string()]);
        assert_eq!(parsed.conflicts, vec!["oldfoo".to_string()]);
        assert_eq!(parsed.obsoletes, vec!["oldfoo".to_string()]);
        assert_eq!(parsed.recommends, vec!["foo-doc".to_string()]);
        assert_eq!(parsed.supplements, vec!["foo".to_string()]);
        assert_eq!(parsed.suggests, vec!["foo-extra".to_string()]);
        assert_eq!(parsed.enhances, vec!["foo-base".to_string()]);
        assert!(parsed.provides_ext.is_empty());
        assert!(parsed.requires_ext.is_empty());
        assert!(parsed.recommends_ext.is_empty());
        assert!(parsed.supplements_ext.is_empty());
    }

    #[test]
    fn parse_fileinfo_reads_the_ext_view_under_both_spellings() {
        let parsed = parse_fileinfo(
            "<fileinfo filename=\"foo-1.0-1.x86_64.rpm\"><name>foo</name><provides_ext dep=\"libc.so.6()(64bit)\"><requiredby name=\"bar\" version=\"2.0\" release=\"1\" arch=\"x86_64\" project=\"p\" repository=\"r\"/></provides_ext><requires_ext dep=\"libc.so.6()(64bit)\"><provided_by name=\"glibc\" epoch=\"2\" version=\"2.38\" release=\"1\" arch=\"x86_64\" project=\"p\" repository=\"r\"/></requires_ext><requires_ext dep=\"libz.so.1()(64bit)\"><providedby name=\"zlib\" version=\"1.3\" release=\"1\" arch=\"x86_64\" project=\"p\" repository=\"r\"/><providedby name=\"zlib-32bit\" version=\"1.3\" release=\"1\" arch=\"i586\" project=\"p\" repository=\"r\"/></requires_ext><recommends_ext dep=\"foo-doc\"><provided_by name=\"foo-doc\" epoch=\"1\" version=\"1.0\" release=\"1\" arch=\"x86_64\" project=\"p\" repository=\"r\"/></recommends_ext><supplements_ext dep=\"foo-base\"><requiredby name=\"bar\" version=\"2.0\" release=\"1\" arch=\"x86_64\" project=\"p\" repository=\"r\"/></supplements_ext></fileinfo>",
        )
        .unwrap();
        assert_eq!(parsed.provides_ext.len(), 1);
        assert_eq!(parsed.provides_ext[0].dep, "libc.so.6()(64bit)");
        assert_eq!(parsed.provides_ext[0].required_by.len(), 1);
        assert_eq!(parsed.provides_ext[0].required_by[0].name, "bar");
        assert_eq!(parsed.requires_ext.len(), 2);
        assert_eq!(parsed.requires_ext[0].dep, "libc.so.6()(64bit)");
        assert_eq!(parsed.requires_ext[0].provided_by[0].name, "glibc");
        assert_eq!(parsed.requires_ext[1].provided_by[0].name, "zlib");
        assert_eq!(parsed.requires_ext[1].provided_by.len(), 2);
        assert_eq!(parsed.requires_ext[1].provided_by[1].name, "zlib-32bit");
        assert_eq!(parsed.requires_ext[0].provided_by[0].epoch, Some(2));
        assert_eq!(parsed.recommends_ext.len(), 1);
        assert_eq!(parsed.recommends_ext[0].dep, "foo-doc");
        assert_eq!(parsed.recommends_ext[0].provided_by[0].name, "foo-doc");
        assert_eq!(parsed.recommends_ext[0].provided_by[0].epoch, Some(1));
        assert_eq!(parsed.supplements_ext.len(), 1);
        assert_eq!(parsed.supplements_ext[0].dep, "foo-base");
        assert_eq!(parsed.supplements_ext[0].required_by.len(), 1);
        assert_eq!(parsed.supplements_ext[0].required_by[0].name, "bar");
    }

    #[test]
    fn parse_fileinfo_rejects_malformed_documents() {
        assert!(parse_fileinfo("").is_err());
        assert!(parse_fileinfo("<collection></collection>").is_err());
        assert!(parse_fileinfo("<fileinfo><name>foo</name></fileinfo>").is_err());
        assert!(
            parse_fileinfo("<fileinfo filename=\"x\"><provides_ext><requiredby name=\"a\"/></provides_ext></fileinfo>")
                .is_err()
        );
        assert!(parse_fileinfo("<fileinfo filename=\"x\"><size>big</size></fileinfo>").is_err());
    }

    #[test]
    fn list_binaries_lists_the_package_and_the_whole_repository() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo",
                "<binarylist><binary filename=\"foo-1.0-1.x86_64.rpm\" size=\"58352\" mtime=\"1617270174\"/><binary filename=\"_buildenv\" size=\"10\" mtime=\"1617270174\"/></binarylist>",
            )
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/_repository",
                "<binarylist><binary filename=\"foo-1.0-1.x86_64.rpm\" size=\"58352\" mtime=\"1617270174\"/></binarylist>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let files = client
            .list_binaries("testproject", Some("foo"), "standard", "x86_64")
            .unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].name, "_buildenv");
        let files = client
            .list_binaries("testproject", None, "standard", "x86_64")
            .unwrap();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn list_binaries_404_is_an_error() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .list_binaries("testproject", Some("foo"), "standard", "x86_64")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn list_binaries_keeps_the_multibuild_flavor_in_the_path() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo:flavor1",
            "<binarylist/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .list_binaries("testproject", Some("foo:flavor1"), "standard", "x86_64")
                .unwrap(),
            Vec::<BinaryFile>::new()
        );
    }

    #[test]
    fn download_binary_returns_raw_bytes_not_text() {
        // Non-UTF-8 bytes: the download must not go through the
        // string path, which would refuse them.
        let bytes: &[u8] = &[0xED, 0xAB, 0xEE, 0xDB, 0xFF, 0xFE];
        let http = FakeHttp::new().with_get_bytes(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo/foo-1.0-1.x86_64.rpm",
            bytes,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .download_binary(
                    "testproject",
                    "foo",
                    "standard",
                    "x86_64",
                    "foo-1.0-1.x86_64.rpm"
                )
                .unwrap(),
            bytes.to_vec()
        );
        assert_eq!(
            http.get_bytes_calls(),
            vec![(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/foo-1.0-1.x86_64.rpm"
                    .to_string(),
                true
            )]
        );
    }

    #[test]
    fn download_binary_404_is_an_error_not_empty_bytes() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo/nosuch.rpm",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .download_binary("testproject", "foo", "standard", "x86_64", "nosuch.rpm")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn download_binary_rejects_an_unsafe_filename_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .download_binary("testproject", "foo", "standard", "x86_64", "../evil")
            .is_err());
        assert!(http.get_bytes_calls().is_empty());
    }

    #[test]
    fn fileinfo_reads_both_views() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/foo-1.0-1.x86_64.rpm?view=fileinfo",
                "<fileinfo filename=\"foo-1.0-1.x86_64.rpm\"><name>foo</name><version>1.0</version></fileinfo>",
            )
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/foo-1.0-1.x86_64.rpm?view=fileinfo_ext",
                "<fileinfo filename=\"foo-1.0-1.x86_64.rpm\"><name>foo</name><requires_ext dep=\"libc.so.6()(64bit)\"><provided_by name=\"glibc\"/></requires_ext></fileinfo>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let info = client
            .fileinfo(
                "testproject",
                "foo",
                "standard",
                "x86_64",
                "foo-1.0-1.x86_64.rpm",
                false,
            )
            .unwrap();
        assert_eq!(info.name.as_deref(), Some("foo"));
        assert!(info.requires_ext.is_empty());
        let info = client
            .fileinfo(
                "testproject",
                "foo",
                "standard",
                "x86_64",
                "foo-1.0-1.x86_64.rpm",
                true,
            )
            .unwrap();
        assert_eq!(info.requires_ext.len(), 1);
        assert_eq!(info.requires_ext[0].provided_by.len(), 1);
        assert_eq!(info.requires_ext[0].provided_by[0].name, "glibc");
    }

    #[test]
    fn fileinfo_404_is_an_error_not_an_empty_fileinfo() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo/nosuch.rpm?view=fileinfo",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .fileinfo(
                "testproject",
                "foo",
                "standard",
                "x86_64",
                "nosuch.rpm",
                false,
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn parse_builddepinfo_reads_packages_and_ignores_cycles() {
        let parsed = parse_builddepinfo(
            "<builddepinfo><package name=\"memcached\"><source>memcached</source><pkgdep>gzip</pkgdep><pkgdep>obs-server</pkgdep><subpkg>memcached-devel</subpkg><subpkg>memcached-debugsource</subpkg></package><package name=\"gzip\"><source>gzip</source><pkgdep>zlib</pkgdep><subpkg>gzip</subpkg></package><cycle><package>memcached</package><package>gzip</package></cycle><scc><package>memcached</package></scc></builddepinfo>",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "memcached");
        assert_eq!(parsed[0].source.as_deref(), Some("memcached"));
        assert_eq!(
            parsed[0].dependencies,
            vec!["gzip".to_string(), "obs-server".to_string()]
        );
        assert_eq!(
            parsed[0].subpackages,
            vec![
                "memcached-devel".to_string(),
                "memcached-debugsource".to_string()
            ]
        );
        assert_eq!(parsed[1].name, "gzip");
        assert_eq!(parsed[1].source.as_deref(), Some("gzip"));
        assert_eq!(parsed[1].dependencies, vec!["zlib".to_string()]);
    }

    #[test]
    fn parse_builddepinfo_empty_is_an_empty_list() {
        assert_eq!(
            parse_builddepinfo("<builddepinfo/>").unwrap(),
            Vec::<PackageDeps>::new()
        );
        assert_eq!(
            parse_builddepinfo("<builddepinfo></builddepinfo>").unwrap(),
            Vec::<PackageDeps>::new()
        );
    }

    #[test]
    fn parse_builddepinfo_rejects_malformed_documents() {
        assert!(parse_builddepinfo("").is_err());
        assert!(parse_builddepinfo("<collection></collection>").is_err());
        assert!(parse_builddepinfo(
            "<builddepinfo><package><pkgdep>x</pkgdep></package></builddepinfo>"
        )
        .is_err());
    }

    #[test]
    fn parse_builddepinfo_rejects_an_inner_unclosed_package() {
        // The package close is missing but the outer close is present:
        // this used to read as an empty list instead of an error.
        assert!(parse_builddepinfo(
            "<builddepinfo><package name=\"a\"><pkgdep>x</pkgdep></builddepinfo>"
        )
        .is_err());
    }

    #[test]
    fn parse_repositories_reads_repos_arches_and_paths() {
        let parsed = parse_repositories(
            "<project name=\"testproject\"><title>t</title><repository name=\"standard\"><arch>x86_64</arch><arch>i586</arch><path project=\"openSUSE.org:openSUSE:Tumbleweed\" repository=\"standard\"/></repository><repository name=\"ports\"><arch>aarch64</arch><path project=\"openSUSE.org:openSUSE:Tumbleweed\" repository=\"ports\"/></repository></project>",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "standard");
        assert_eq!(
            parsed[0].arch,
            vec!["x86_64".to_string(), "i586".to_string()]
        );
        assert_eq!(parsed[0].paths.len(), 1);
        assert_eq!(parsed[0].paths[0].0, "openSUSE.org:openSUSE:Tumbleweed");
        assert_eq!(parsed[0].paths[0].1, "standard");
        assert_eq!(parsed[1].name, "ports");
        assert_eq!(parsed[1].arch, vec!["aarch64".to_string()]);
    }

    #[test]
    fn parse_repositories_empty_is_an_empty_list() {
        assert_eq!(
            parse_repositories("<project name=\"p\"/>").unwrap(),
            Vec::<Repository>::new()
        );
        assert_eq!(
            parse_repositories("<project name=\"p\"><title>t</title></project>").unwrap(),
            Vec::<Repository>::new()
        );
    }

    #[test]
    fn parse_repositories_rejects_malformed_documents() {
        assert!(parse_repositories("").is_err());
        assert!(parse_repositories("<collection></collection>").is_err());
        assert!(parse_repositories(
            "<project><repository><arch>x86_64</arch></repository></project>"
        )
        .is_err());
        assert!(
            parse_repositories(
                "<project><repository name=\"r\"><path repository=\"standard\"/></repository></project>"
            )
            .is_err()
        );
    }

    #[test]
    fn parse_repositories_rejects_an_inner_unclosed_repository() {
        // The repository close is missing but the outer close is
        // present: this used to read as an empty list instead of an
        // error.
        assert!(parse_repositories(
            "<project name=\"p\"><repository name=\"r\"><path project=\"q\" repository=\"s\"/></project>"
        )
        .is_err());
    }

    #[test]
    fn dependson_builds_the_url_with_and_without_a_package() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/_builddepinfo?view=pkgnames&package=foo",
                "<builddepinfo><package name=\"foo\"><pkgdep>bar</pkgdep><subpkg>foo</subpkg></package></builddepinfo>",
            )
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/_builddepinfo?view=pkgnames",
                "<builddepinfo/>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let deps = client
            .dependson("testproject", "standard", "x86_64", Some("foo"))
            .unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].dependencies, vec!["bar".to_string()]);
        let deps = client
            .dependson("testproject", "standard", "x86_64", None)
            .unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn whatdependson_builds_the_reverse_url() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/x86_64/_builddepinfo?view=revpkgnames&package=bar",
            "<builddepinfo><package name=\"foo\"><pkgdep>bar</pkgdep></package></builddepinfo>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let deps = client
            .whatdependson("testproject", "standard", "x86_64", "bar")
            .unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "foo");
    }

    #[test]
    fn dependson_404_is_an_error_not_an_empty_list() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/build/testproject/standard/x86_64/_builddepinfo?view=pkgnames",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .dependson("testproject", "standard", "x86_64", None)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn check_constraints_posts_the_constraints_and_reads_worker_ids() {
        let constraints = "<constraints><hardware><disk><size unit=\"G\">10</size></disk></hardware></constraints>";
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/worker?cmd=checkconstraints&project=testproject&package=foo&repository=standard&arch=x86_64",
            "<directory><entry name=\"worker1\"/><entry name=\"worker2\"/></directory>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let workers = client
            .check_constraints(
                "testproject",
                "foo",
                "standard",
                "x86_64",
                Some(constraints),
            )
            .unwrap();
        assert_eq!(workers, vec!["worker1".to_string(), "worker2".to_string()]);
        assert_eq!(
            http.post_calls(),
            vec![(
                "https://api.opensuse.org/worker?cmd=checkconstraints&project=testproject&package=foo&repository=standard&arch=x86_64"
                    .to_string(),
                constraints.to_string(),
                true
            )]
        );
    }

    #[test]
    fn check_constraints_non_directory_body_is_an_error() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/worker?cmd=checkconstraints&project=testproject&package=foo&repository=standard&arch=x86_64",
            "<collection></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .check_constraints("testproject", "foo", "standard", "x86_64", None)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn check_constraints_without_constraints_posts_an_empty_body() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/worker?cmd=checkconstraints&project=testproject&package=foo&repository=standard&arch=x86_64",
            "<directory/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let workers = client
            .check_constraints("testproject", "foo", "standard", "x86_64", None)
            .unwrap();
        assert!(workers.is_empty());
        assert_eq!(http.post_calls()[0].1, String::new());
    }

    #[test]
    fn check_constraints_rejects_empty_constraints_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .check_constraints("testproject", "foo", "standard", "x86_64", Some(""))
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn list_repositories_reads_the_public_meta() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/testproject/_meta",
            "<project name=\"testproject\"><repository name=\"standard\"><arch>x86_64</arch><arch>i586</arch><path project=\"openSUSE.org:openSUSE:Tumbleweed\" repository=\"standard\"/></repository></project>",
        );
        let client = ObsClient::new(&http);
        let repos = client.list_repositories("testproject").unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "standard");
        assert_eq!(
            repos[0].arch,
            vec!["x86_64".to_string(), "i586".to_string()]
        );
        // The public read carries no credentials.
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/public/source/testproject/_meta".to_string(),
                false
            )]
        );
    }

    #[test]
    fn debug_change_state_unknown_state_reports_probe_and_decision() {
        // No canned read for the state check, so every probe reads
        // the state as unknown and keeps retrying: with a 1500 ms
        // budget the helper tries four times (three 500 ms sleeps).
        let (debug, lines) = Debug::captured(true, false);
        let http = FakeHttp::new().with_post_error_sequence(
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
            &[
                ErrorKind::ServerError,
                ErrorKind::ServerError,
                ErrorKind::ServerError,
                ErrorKind::ServerError,
            ],
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials())
            .unwrap()
            .with_debug(debug);
        let err = obs
            .post_changestate_with_retry(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                "looks good",
                42,
                RequestState::Accepted,
                std::time::Duration::from_millis(1500),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert_eq!(http.post_calls().len(), 4);
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            [
                "retry: change_state request 42 probe 1/1500ms: state unknown".to_string(),
                "decision: request 42 state unknown, re-issuing accepted".to_string(),
                "retry: change_state request 42 probe 2/1500ms: state unknown".to_string(),
                "decision: request 42 state unknown, re-issuing accepted".to_string(),
                "retry: change_state request 42 probe 3/1500ms: state unknown".to_string(),
                "decision: request 42 state unknown, re-issuing accepted".to_string(),
                "retry: change_state request 42 probe 4/1500ms: state unknown".to_string(),
                "decision: request 42 state unknown, re-issuing accepted".to_string(),
            ]
        );
    }

    #[test]
    fn format_budget_prints_whole_seconds_and_millis() {
        assert_eq!(
            ObsClient::format_budget(std::time::Duration::from_secs(30)),
            "30s"
        );
        assert_eq!(
            ObsClient::format_budget(std::time::Duration::from_secs(60)),
            "60s"
        );
        assert_eq!(
            ObsClient::format_budget(std::time::Duration::from_millis(1500)),
            "1500ms"
        );
        assert_eq!(
            ObsClient::format_budget(std::time::Duration::from_millis(500)),
            "500ms"
        );
        assert_eq!(
            ObsClient::format_budget(std::time::Duration::from_millis(0)),
            "0s"
        );
    }

    #[test]
    fn debug_try_change_state_reports_a_refused_change() {
        let (debug, lines) = Debug::captured(true, false);
        let http = FakeHttp::new().with_post_error(
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
            ErrorKind::AuthFailed,
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials())
            .unwrap()
            .with_debug(debug);
        let dry = DryRun::new(false);
        assert!(!obs
            .try_change_state("42", RequestState::Accepted, "ok", false, &dry)
            .unwrap());
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            ["decision: could not change state of submit request 42: auth_failed: authentication failed (canned POST error for https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted)"
                .to_string()]
        );
    }

    #[test]
    fn debug_file_reports_the_dedup_decision() {
        let (debug, lines) = Debug::captured(true, false);
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs.search_url("foo").unwrap();
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry).with_debug(debug);
        assert_eq!(
            filing.file(&payload()).unwrap(),
            Some("1400001".to_string())
        );
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            ["decision: no open submit request targets foo; filing".to_string()]
        );
    }

    #[test]
    fn debug_file_reports_the_dedup_refusal() {
        let (debug, lines) = Debug::captured(true, false);
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs.search_url("foo").unwrap();
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"1398492\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry).with_debug(debug);
        let err = filing.file(&payload()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            [
                "decision: open submit request(s) 1398492 already target foo; refusing to file"
                    .to_string()
            ]
        );
    }

    #[test]
    fn list_repositories_404_is_an_error_not_an_empty_list() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/public/source/nosuchproject/_meta",
            ErrorKind::NotFound,
        );
        let client = ObsClient::new(&http);
        let err = client.list_repositories("nosuchproject").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn list_repositories_rejects_an_invalid_name_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client.list_repositories("bad/project").is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn debug_file_reports_nothing_when_debug_is_off() {
        let (debug, lines) = Debug::captured(false, false);
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs.search_url("foo").unwrap();
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry).with_debug(debug);
        assert_eq!(
            filing.file(&payload()).unwrap(),
            Some("1400001".to_string())
        );
        assert!(lines.lock().unwrap().is_empty());
    }

    #[test]
    fn debug_maintenance_incident_reports_the_dedup_decision() {
        let (debug, lines) = Debug::captured(true, false);
        let search_url = action_match_url(
            "maintenance_incident",
            &[
                "source/@package='foo'",
                "target/@project='openSUSE:Maintenance'",
            ],
        );
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create&addrevision=1",
                "<request id=\"1400001\" creator=\"alice\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry).with_debug(debug);
        let payload = incident_payload();
        assert_eq!(
            filing.file_maintenance_incident(&payload).unwrap(),
            Some("1400001".to_string())
        );
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            ["decision: no open maintenance_incident request targets openSUSE:Maintenance for foo; filing"
                .to_string()]
        );
    }

    #[test]
    fn debug_maintenance_incident_reports_the_dedup_refusal() {
        let (debug, lines) = Debug::captured(true, false);
        let search_url = action_match_url(
            "maintenance_incident",
            &[
                "source/@package='foo'",
                "target/@project='openSUSE:Maintenance'",
            ],
        );
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"1399001\" creator=\"alice\"><action type=\"maintenance_incident\"><source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" /><target project=\"openSUSE:Maintenance\" /></action><state name=\"new\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry).with_debug(debug);
        let err = filing
            .file_maintenance_incident(&incident_payload())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            ["decision: open maintenance_incident request(s) 1399001 already target openSUSE:Maintenance for foo; refusing to file"
                .to_string()]
        );
    }
}
