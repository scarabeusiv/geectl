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
//! - Filing supersedes every open submit request for the package from
//!   the authenticated user. [`SrFiling::file`] files the new request
//!   first, then supersedes each open one with it; the supersede is
//!   unconditional. The open-request check is also available on its
//!   own as [`ObsClient::has_open_sr`].
//! - The SR description follows the guide's message format: one
//!   headline (at most 160 characters of prose), then `- ` lines for
//!   staging, needs, and rebuilds. The library validates the
//!   description is non-empty; the headline shape is the caller's
//!   responsibility (see [`SrPayload`]).
//! - A security fix's `.changes` entry must name every CVE it fixes.
//!   When
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
//! `GEECTL_OBS_PASSWORD_FILE`). The public reads need no credentials
//! (see Credentials in `docs/architecture.md`). The open-SR search and
//! the filing POST need them and fail loudly without them. No credential
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

pub(crate) mod branch;
pub(crate) mod build;
pub(crate) mod client;
pub(crate) mod common;
pub(crate) mod maintenance;
pub(crate) mod payload;
pub(crate) mod remote;
pub(crate) mod request;
pub(crate) mod services;
pub(crate) mod staging;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod types;

pub(crate) use self::branch::build_link_meta;
pub use self::build::BinaryFile;
pub use self::build::BuildDep;
pub use self::build::BuildHistoryEntry;
pub use self::build::BuildInfo;
pub use self::build::BuildLogQuery;
pub use self::build::BuildResult;
pub use self::build::BuildResults;
pub use self::build::ExtDependency;
pub use self::build::FileInfo;
pub use self::build::JobHistoryEntry;
pub use self::build::JobStatus;
pub use self::build::PackageBuildStatus;
pub use self::build::PackageDeps;
pub use self::build::PreinstallImage;
pub use self::build::ProvidedBy;
pub use self::build::ResultsQuery;
pub use self::client::ObsClient;
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
pub(crate) use self::common::parse_submit_request_ids_for_creator;
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
pub use self::payload::ChangeDevelPayload;
pub use self::payload::DeletePayload;
pub use self::payload::DeleteTarget;
pub use self::payload::FiledRequest;
pub use self::payload::MaintainedInstance;
pub use self::payload::MaintenanceBranchOptions;
pub use self::payload::MaintenanceBranchResult;
pub use self::payload::MaintenanceIncidentPayload;
pub use self::payload::SourceUpdate;
pub use self::payload::SrFiling;
pub use self::payload::SrPayload;
pub(crate) use self::remote::parse_directory;
pub(crate) use self::remote::parse_directory_rev;
pub(crate) use self::remote::parse_linkinfo;
pub(crate) use self::remote::parse_package_names;
pub(crate) use self::remote::parse_revision;
pub use self::request::parse_comments;
pub use self::request::parse_request;
pub use self::request::parse_request_collection;
pub(crate) use self::request::validate_request_id;
pub use self::request::AcceptInfo;
pub use self::request::Request;
pub use self::request::RequestAction;
pub use self::request::RequestComment;
pub use self::request::RequestHistory;
pub use self::request::RequestPriority;
pub use self::request::RequestReview;
pub use self::request::RequestSearch;
pub use self::request::RequestState;
pub use self::request::RequestStateInfo;
pub use self::request::ReviewState;
pub use self::request::ReviewTarget;
pub use self::request::XpathQuery;
pub use self::services::ServiceDef;
pub use self::services::ServiceParam;
pub use self::staging::StagedRequest;
pub use self::staging::StagingProject;
pub use self::staging::StagingReadOptions;
pub use self::types::changelog_names_cve;
pub(crate) use self::types::ActionShape;
pub(crate) use self::types::ActionTarget;
pub use self::types::AggregateOptions;
pub use self::types::BranchOptions;
pub use self::types::BranchResult;
pub use self::types::CopyOptions;
pub use self::types::CopyProjectOptions;
pub use self::types::CreatePackageOptions;
pub use self::types::CrossInstanceCopyResult;
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
pub use self::types::ServiceCode;
pub use self::types::ServiceState;
pub use self::types::SourceFile;
