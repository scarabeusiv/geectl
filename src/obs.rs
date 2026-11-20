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
//! (the same reason the temporal harness builds its URLs as strings).
//!
//! The live executor that actually branches, bumps, cleans, builds,
//! and runs the package gates is NOT part of this module: the library
//! files, supersedes, and deletes; the caller owns the package work.
//! Branch deletion stays a separate explicit call — the caller decides
//! when a branch is safe to delete — but the full sequence (file ->
//! supersede -> delete branch) is documented on [`SrFiling::refile`].

use crate::auth::Credentials;
use crate::error::{ErrorKind, GeectlError};
use crate::http::{is_loopback_url, HttpClient};
use crate::url::{encode_path_segment, encode_query_value, UrlBuilder};
use std::cell::RefCell;

/// The OBS API base (production).
pub const OBS_API_BASE: &str = "https://api.opensuse.org";
/// The Factory project: the devel lookup starts here.
pub const FACTORY_PROJECT: &str = "openSUSE:Factory";

/// True when `name` is a valid OBS package or project name
/// (`[A-Za-z0-9_.+-:]`). Anything else is rejected rather than quoted
/// into a URL or an XPath literal, where a `/`, `?`, or quote would
/// rewrite the request.
pub fn valid_obs_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+' | ':'))
}

/// True when `name` is a valid OBS source filename for upload.
/// Mirrors the server's rule (`PackageService::FileVerifier#file_name_invalid?`:
/// `^[^./][^/]+$` — first character not `.` or `/`, no `/` anywhere;
/// the `+` needs at least one more character, so a one-character name
/// is invalid and the minimum length is two), plus the two names the
/// routes reserve: `_meta` (the package-meta route, not a file) and
/// `_attribute` (rejected by `Package.verify_file!`). Anything else is
/// rejected rather than quoted into a URL, where a `/` would rewrite
/// the request.
pub fn valid_obs_filename(name: &str) -> bool {
    name.chars().count() >= 2
        && !name.starts_with('.')
        && !name.contains('/')
        && name != "_meta"
        && name != "_attribute"
}

/// Split a `project/package` path on `/`, dropping empty segments:
/// a leading, trailing, or doubled slash contributes no segment, so
/// `"//"` and `"/////////"` split to nothing and `"a//b"` to
/// `["a", "b"]`. The segments are returned verbatim (no trimming);
/// callers validate them as OBS names.
pub fn split_slash_segments(s: &str) -> Vec<String> {
    s.split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_string())
        .collect()
}

/// Split a `project/package` path into its two parts. Exactly two
/// non-empty segments, or an error: a bare project, a bare package,
/// or more than one slash is not a project/package pair.
pub fn parse_project_package(s: &str) -> Result<(String, String), GeectlError> {
    let segments = split_slash_segments(s);
    if segments.len() != 2 {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("expected a project/package path, got {s:?}"),
        ));
    }
    Ok((segments[0].clone(), segments[1].clone()))
}

/// Build a request URL from a base, path segments, and query pairs,
/// using the encoders from `url.rs`. Segments are encoded as exactly
/// one element each; query pairs as `name=value`.
fn build_url(base: &str, segments: &[&str], query: &[(&str, &str)]) -> String {
    let mut url = base.trim_end_matches('/').to_string();
    for segment in segments {
        url.push('/');
        url.push_str(&encode_path_segment(segment));
    }
    if !query.is_empty() {
        url.push('?');
        for (i, (name, value)) in query.iter().enumerate() {
            if i > 0 {
                url.push('&');
            }
            url.push_str(&encode_query_value(name));
            url.push('=');
            url.push_str(&encode_query_value(value));
        }
    }
    url
}

/// Validate an OBS API base URL. It must be a bare `https://` URL
/// (scheme plus authority, no path, query, fragment, or userinfo)
/// with a host (the production API), or a bare `http://` URL on a
/// loopback host (a test instance only). Anything else is refused at
/// construction, not at the first request. A trailing slash is
/// stripped, not a path.
fn validate_base(base: &str) -> Result<(), GeectlError> {
    // A base is scheme plus authority only: anything after the
    // authority would be string-concatenated into every OBS URL (see
    // `build_url`), so a base such as `https://api.opensuse.org/evil`
    // would rewrite them all. The temporal harness enforces the same
    // bare shape for its base URL.
    let base = base.trim_end_matches('/');
    if let Some(authority) = base.strip_prefix("https://") {
        if !is_bare_authority(authority) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "OBS base URL must be a bare https:// URL (scheme + host, no path, query, fragment, or userinfo)",
            ));
        }
        // Reuse the production builder's validation (https + host).
        UrlBuilder::new(base).map(|_| ())
    } else if let Some(authority) = base.strip_prefix("http://") {
        if !is_bare_authority(authority) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "OBS base URL must be a bare http:// URL (scheme + host, no path, query, fragment, or userinfo)",
            ));
        }
        if is_loopback_url(base) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "an http:// OBS base must be a loopback URL (a test instance only)",
            ))
        }
    } else {
        Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "OBS base URL must be https:// (or http:// loopback for a test instance)",
        ))
    }
}

/// True when `authority` is a bare authority: non-empty, with no path,
/// query, fragment, or userinfo. The authority ends at the first `/`,
/// `?`, or `#`; an `@` inside it is userinfo, which a base must never
/// carry (credentials belong in `Credentials`, not in the URL).
fn is_bare_authority(authority: &str) -> bool {
    !authority.is_empty() && !authority.contains(['/', '?', '#', '@'])
}

/// A CVE identifier, e.g. `CVE-2026-12345`. Stored in canonical
/// uppercase form.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CveId(String);

impl CveId {
    /// Parse and validate a CVE id: `CVE-<year>-<number>`, year 1999 or
    /// later, number at least 4 digits. Accepts lowercase input and
    /// normalizes it to uppercase.
    pub fn parse(s: &str) -> Result<Self, GeectlError> {
        let upper = s.to_ascii_uppercase();
        let parts: Vec<&str> = upper.split('-').collect();
        if parts.len() != 3 || parts[0] != "CVE" {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid CVE id: {s:?}"),
            ));
        }
        let year = parts[1];
        let number = parts[2];
        let year_ok = year.len() == 4
            && year.chars().all(|c| c.is_ascii_digit())
            && year.parse::<u32>().map(|y| y >= 1999).unwrap_or(false);
        let number_ok = number.len() >= 4 && number.chars().all(|c| c.is_ascii_digit());
        if !year_ok || !number_ok {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid CVE id: {s:?}"),
            ));
        }
        Ok(CveId(upper))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CveId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Does the changelog text reference this CVE?
///
/// Case-insensitive on the id. The match must be a standalone id: an
/// alphanumeric right after means the changelog names a longer token
/// that merely starts with this id (`CVE-2026-123` does not match a
/// changelog that names `CVE-2026-1234`, and `CVE-2026-12345a` does
/// not name `CVE-2026-12345`), and an alphanumeric right before means
/// the match is part of a longer token (`XCVE-2026-12345` does not
/// match `CVE-2026-12345`). Returns false for an empty changelog.
pub fn changelog_names_cve(changelog: &str, cve: &CveId) -> bool {
    let haystack = changelog.to_ascii_lowercase();
    let needle = cve.as_str().to_ascii_lowercase();
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(&needle) {
        let match_start = start + pos;
        let after = match_start + needle.len();
        // Leading boundary: the char before must not be alphanumeric.
        let leading_ok =
            match_start == 0 || !haystack.as_bytes()[match_start - 1].is_ascii_alphanumeric();
        // Trailing boundary: an alphanumeric right after means the
        // changelog names a longer token that merely starts with this
        // id (`CVE-2026-123456` and `CVE-2026-12345a` do not name
        // `CVE-2026-12345`).
        let trailing_ok =
            after >= haystack.len() || !haystack.as_bytes()[after].is_ascii_alphanumeric();
        if leading_ok && trailing_ok {
            return true;
        }
        start = after;
    }
    false
}

/// Dry-run mode: the "say what you would do, do nothing" switch.
///
/// Every mutating operation goes through the single guard here
/// (`DryRun::allow`) before any write or API call. Reads are NOT
/// gated: the caller still reads (it has to, to say what it WOULD
/// do), and the dedup check still runs in live mode. Only writes
/// are gated. Dry-run is the default: a `DryRun` built with
/// `Default` files nothing.
#[derive(Debug)]
pub struct DryRun {
    enabled: bool,
    plan: RefCell<Vec<String>>,
}

impl DryRun {
    /// A dry-run mode with the given setting. `true` = dry-run (do
    /// nothing, print the plan); `false` = live.
    pub fn new(enabled: bool) -> Self {
        DryRun {
            enabled,
            plan: RefCell::new(Vec::new()),
        }
    }

    /// True when dry-run is on (nothing is written or filed).
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// True when live (dry-run off).
    pub fn live(&self) -> bool {
        !self.enabled
    }

    /// The would-do plan recorded so far (in order). Tests assert on
    /// this; production prints each entry to stderr as it is recorded.
    pub fn plan(&self) -> Vec<String> {
        self.plan.borrow().clone()
    }

    /// The guard. Call before any mutating operation. Returns true
    /// when the operation may proceed (live mode). In dry-run,
    /// records and prints the would-do action and returns false.
    pub fn allow(&self, action: &str) -> bool {
        if self.enabled {
            self.note(&format!("would: {action}"));
            false
        } else {
            true
        }
    }

    /// Record (and print) a would-do decision, e.g. a would-skip with
    /// its reason. Does not gate anything; use `allow` for that.
    pub fn note(&self, message: &str) {
        if self.enabled {
            let line = format!("[dry-run] {message}");
            eprintln!("{line}");
            self.plan.borrow_mut().push(line);
        }
    }
}

impl Default for DryRun {
    /// Dry-run is the default: a mode built without a setting files
    /// nothing. Live mode must be asked for explicitly.
    fn default() -> Self {
        DryRun::new(true)
    }
}

/// A single revision specifier: a revision number, or a source md5.
/// The `rev` parameter the API takes on source reads (`?rev=N`) is a
/// number; an md5 names the same revision by content hash. Parsed by
/// [`RevisionSpec::parse`]; a string that is neither a positive number
/// nor a 32-hex md5 is invalid, never silently a revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionSpec {
    /// Revision number (`rev` attribute).
    Rev(u64),
    /// Source md5 (32 hex digits, normalized to lowercase).
    Md5(String),
}

impl RevisionSpec {
    /// Parse a single revision specifier: `"1"` → `Rev(1)`; a 32-hex
    /// md5 → `Md5`. Anything else (empty, `0`, a short hash, a
    /// non-hex string) is an error.
    pub fn parse(s: &str) -> Result<Self, GeectlError> {
        if s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(RevisionSpec::Md5(s.to_ascii_lowercase()));
        }
        match s.parse::<u64>() {
            Ok(0) | Err(_) => Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid revision specifier: {s:?}"),
            )),
            Ok(n) => Ok(RevisionSpec::Rev(n)),
        }
    }
}

/// A revision range: an optional lower and upper bound, each a
/// [`RevisionSpec`]. Parsed by [`RevisionRange::parse`] from the
/// `rev` option form: `""` or `":"` → no bounds; `"1"` → only rev 1;
/// `"1:2"` → revs 1 through 2; a 32-hex md5 → only that md5;
/// `"hash1:hash2"` → the revisions between the two hashes, endpoints
/// included. More than one `:` or a part that is neither a revision
/// number nor a 32-hex md5 is an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionRange {
    /// Lower bound (inclusive), or `None` for the oldest revision.
    pub from: Option<RevisionSpec>,
    /// Upper bound (inclusive), or `None` for the newest revision.
    pub to: Option<RevisionSpec>,
}

impl RevisionRange {
    /// Parse a revision range: `""`/`":"` → `(None, None)`; `"1"` →
    /// `(Rev(1), None)`; `"1:2"` → `(Rev(1), Rev(2))`; a 32-hex md5
    /// → `(Md5, None)`; `"hash1:hash2"` → `(Md5, Md5)`. `":::::"`,
    /// `"1:2:3:4:5"`, and three hashes are errors (more than one
    /// `:`); a part that is neither a revision number nor a 32-hex
    /// md5 is an error.
    pub fn parse(s: &str) -> Result<Self, GeectlError> {
        let invalid = || {
            GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid revision range: {s:?}"),
            )
        };
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() > 2 {
            return Err(invalid());
        }
        let parse_part = |part: &str| -> Result<Option<RevisionSpec>, GeectlError> {
            if part.is_empty() {
                Ok(None)
            } else {
                RevisionSpec::parse(part).map(Some).map_err(|_| invalid())
            }
        };
        let from = parse_part(parts[0])?;
        let to = if parts.len() == 2 {
            parse_part(parts[1])?
        } else {
            None
        };
        Ok(RevisionRange { from, to })
    }
}

/// A file in an OBS source directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// File name.
    pub name: String,
    /// File size in bytes, when the listing carries it.
    pub size: Option<u64>,
    /// Modification time (seconds since the epoch), when the listing
    /// carries it.
    pub mtime: Option<u64>,
    /// Source md5, when the listing carries it.
    pub md5: Option<String>,
}

/// A parsed OBS source directory listing: the files plus the link
/// target, when the package links another package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DirectoryListing {
    /// The files in the listing, in server order.
    pub entries: Vec<SourceFile>,
    /// `(project, package)` from `<linkinfo project package/>`, when
    /// the listing carries one.
    pub linkinfo: Option<(String, String)>,
}

/// One entry in a package's history (`<revision>` in
/// `<revisionlist>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    /// Revision number (`rev` attribute).
    pub rev: u64,
    /// Versioned revision (`vrev` attribute).
    pub vrev: u64,
    /// Source md5 of this revision, when the history carries it.
    pub srcmd5: Option<String>,
    /// Package version at this revision, when the history carries it.
    pub version: Option<String>,
    /// Commit time (seconds since the epoch), when the history
    /// carries it.
    pub time: Option<u64>,
    /// Who committed this revision, when the history carries it.
    pub user: Option<String>,
    /// Commit comment, when the history carries it.
    pub comment: Option<String>,
    /// The submit request that filed this revision, when the history
    /// carries it.
    pub request_id: Option<String>,
}

/// Whether a file was added, modified, or deleted between two
/// revisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilePatchStatus {
    /// Present in the new revision only.
    Added,
    /// Present in both, with different contents.
    Modified,
    /// Present in the old revision only.
    Deleted,
}

/// One file's diff between two revisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatch {
    /// File name.
    pub filename: String,
    /// Whether the file was added, modified, or deleted.
    pub status: FilePatchStatus,
    /// Unified diff of the file's contents (`---`/`+++`/`@@`
    /// form); empty for an unchanged file, which never appears here.
    pub diff: String,
}

/// The per-file diff of one revision against the previous one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionPatch {
    /// The revision the patch describes.
    pub rev: u64,
    /// The changed files, sorted by name.
    pub files: Vec<FilePatch>,
}

/// A person or group with a role on a project or package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedRole {
    /// User login (`person`) or group id (`group`).
    pub name: String,
    /// Role (`maintainer`, `bugowner`, `reviewer`, ...).
    pub role: String,
}

/// A repository in a project meta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    /// Repository name.
    pub name: String,
    /// Architectures the repository builds for.
    pub arch: Vec<String>,
    /// Rebuild sources: `(project, repository)` pairs.
    pub paths: Vec<(String, String)>,
}

/// A project meta (`<project>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectMeta {
    /// Project name.
    pub name: String,
    /// Title, when the meta carries one.
    pub title: Option<String>,
    /// Description, when the meta carries one.
    pub description: Option<String>,
    /// URL, when the meta carries one.
    pub url: Option<String>,
    /// The git source the project syncs from, when it is a
    /// git-backed (scmsync) project.
    pub scmsync: Option<String>,
    /// The devel project, when the meta names one.
    pub devel: Option<String>,
    /// Persons with roles on the project.
    pub persons: Vec<NamedRole>,
    /// Groups with roles on the project.
    pub groups: Vec<NamedRole>,
    /// Repositories.
    pub repositories: Vec<Repository>,
}

/// A package meta (`<package>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageMeta {
    /// Package name.
    pub name: String,
    /// The project the package lives in.
    pub project: String,
    /// Title, when the meta carries one.
    pub title: Option<String>,
    /// Description, when the meta carries one.
    pub description: Option<String>,
    /// URL, when the meta carries one.
    pub url: Option<String>,
    /// The git source the package syncs from, when it is a
    /// git-backed (scmsync) package.
    pub scmsync: Option<String>,
    /// Release name, when the meta carries one.
    pub releasename: Option<String>,
    /// The devel project and package, when the meta names one.
    pub devel: Option<(String, String)>,
    /// Persons with roles on the package.
    pub persons: Vec<NamedRole>,
    /// Groups with roles on the package.
    pub groups: Vec<NamedRole>,
}

/// A package match from a package search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageMatch {
    /// Package name.
    pub name: String,
    /// The project the package lives in.
    pub project: String,
    /// Title, when the match carries one.
    pub title: Option<String>,
    /// Description, when the match carries one.
    pub description: Option<String>,
}

/// A project match from a project search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectMatch {
    /// Project name.
    pub name: String,
    /// Title, when the match carries one.
    pub title: Option<String>,
    /// Description, when the match carries one.
    pub description: Option<String>,
}

/// An owner of a package or project: the project (and package, when
/// the ownership is package-scoped) the person or group owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    /// The root project the ownership resolves under.
    pub root_project: String,
    /// The project the ownership is in.
    pub project: String,
    /// The package, when the ownership is package-scoped.
    pub package: Option<String>,
    /// Persons with roles on it.
    pub persons: Vec<NamedRole>,
    /// Groups with roles on it.
    pub groups: Vec<NamedRole>,
}

/// A person (`<person>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    /// Login name.
    pub login: String,
    /// Email address.
    pub email: String,
    /// Real name, when the person carries one.
    pub realname: Option<String>,
    /// Account state (`confirmed`, `locked`, ...).
    pub state: String,
}

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
}

impl<'a> ObsClient<'a> {
    /// A client for the production OBS API, with no credentials
    /// (public reads only).
    pub fn new(http: &'a dyn HttpClient) -> Self {
        ObsClient {
            http,
            base: OBS_API_BASE.to_string(),
            credentials: Credentials::None,
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
            }),
            Credentials::Bearer { .. } | Credentials::SshKey { .. } => Err(GeectlError::new(
                ErrorKind::ConfigError,
                "OBS accepts Basic credentials only; a Bearer token or SSH key cannot authenticate it",
            )),
        }
    }

    /// The credentials, if any.
    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    /// True when OBS Basic credentials are configured.
    fn has_basic(&self) -> bool {
        matches!(self.credentials, Credentials::Basic { .. })
    }

    /// Return the devel project for `package` on Factory, or None
    /// when no devel project is registered. A 404 is None as well:
    /// the package is not there, so it has no devel project. Any
    /// other failed lookup is an error, never None.
    pub fn devel_project(&self, package: &str) -> Result<Option<String>, GeectlError> {
        if !valid_obs_name(package) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS package name: {package:?}"),
            ));
        }
        let url = build_url(
            &self.base,
            &["public", "source", FACTORY_PROJECT, package],
            &[],
        );
        let body = match self.http.get_with(&url, &Credentials::None) {
            Ok(body) => body,
            // A 404 means the package (or Factory itself) is not
            // there, so it has no devel project — that is None, not
            // a failed lookup. Any other failure stays an error.
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let project = parse_devel_project(&body)?;
        // The project comes from network XML, so it is validated at
        // the source like any other interpolated value: a devel
        // project that is not a valid OBS name is a server response
        // that cannot be trusted, not a project.
        if let Some(project) = &project {
            if !valid_obs_name(project) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS project name: {project:?}"),
                ));
            }
        }
        Ok(project)
    }

    /// Return the devel project's Version of `package`, or None when
    /// the package has no devel project or its spec carries no
    /// Version. A failed lookup is an error, never None.
    pub fn devel_version(&self, package: &str) -> Result<Option<String>, GeectlError> {
        let project = match self.devel_project(package)? {
            Some(project) => project,
            None => return Ok(None),
        };
        // The project comes from network XML (`<devel project>`), so
        // it is validated and encoded like any other interpolated
        // value before it goes into a URL.
        if !valid_obs_name(&project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        let url = build_url(
            &self.base,
            &[
                "public",
                "source",
                &project,
                package,
                &format!("{package}.spec"),
            ],
            &[],
        );
        let spec = self.http.get_with(&url, &Credentials::None)?;
        Ok(parse_spec_version(&spec))
    }

    /// True when an open submit request targets `package`.
    ///
    /// Uses the OBS search API, matching on actions targeting the
    /// package in the `new`/`review` states, then keeping only the
    /// requests that carry a submit action: the search engine cannot
    /// filter by action type itself. The
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
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot file a submit request without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(&self.base, &["request"], &[("cmd", "create")]);
        let body = self.http.post(&url, &payload.to_xml(), &self.credentials)?;
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

    /// Delete `project`/`package` from OBS (`DELETE
    /// /source/<project>/<package>`): the manual branch cleanup.
    /// Requests filed through this library clean up on accept by
    /// default (see [`SrPayload`]), so this is for the branches the
    /// server-side cleanup never reaches: a superseded request's
    /// branch, a declined request's branch, and any branch whose
    /// request was filed without the cleanup attribute. Requires
    /// credentials; without them the call fails loudly before any
    /// request. Takes the caller's [`DryRun`]: in dry-run the delete
    /// is recorded in the plan and nothing is deleted, so a dry-run
    /// caller cannot mutate the server by invoking this directly.
    ///
    /// The delete is refused while requests in `new`, `review`, or
    /// `declined` state involve the package as a submit source or
    /// target — unless `force` is set. That refusal is the library's
    /// own guard, and it exists because the API does NOT refuse: a
    /// package delete revokes the requests that have it as source and
    /// declines the ones that have it as target
    /// (`Package#revoke_requests_with_self_as_source` /
    /// `#decline_requests_with_self_as_target`), with the comment
    /// "The package 'X/Y' has been removed". Deleting a branch with a
    /// live submit request would silently kill that request, so the
    /// guard names the blocking requests and refuses instead. The
    /// state set is exactly the set the API touches — osc's
    /// `delete_package` pre-check covers the same set, so here osc and
    /// the API agree; they differ only in mechanism (osc refuses, the
    /// API closes).
    ///
    /// `force` bypasses the guard AND is forwarded as `force=1` to
    /// the API, which also bypasses the server's weak-dependency
    /// guard (`check_weak_dependencies!`: refuse when other packages
    /// use this one as their devel package). That is osc's `force`
    /// semantics, and it is the only way a delete with blocking
    /// requests proceeds — the caller has verified the requests are
    /// abandonable. A review by the package on an open request is NOT
    /// covered by the guard: the delete only obsoletes that review
    /// (`obsolete_reviews_for_self`) without changing the request's
    /// state.
    ///
    /// Success is the `<status code="ok"/>` body: any other body is
    /// a server error, never success. A delete that times out leaves
    /// the state unknown (the package may or may not be gone) — the
    /// caller checks the server state instead of assuming.
    pub fn delete_package(
        &self,
        project: &str,
        package: &str,
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
        // The dry-run guard: in dry-run the delete is recorded in the
        // plan and nothing is deleted — not even the pre-check search
        // runs, since a dry-run delete has nothing it must read.
        // Without this, a dry-run caller invoking delete directly
        // would mutate the server.
        if !dry_run.allow(&format!("delete package {project}/{package}")) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot delete a package without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        if !force {
            let blocking = self.open_requests_for_package(project, package)?;
            if !blocking.is_empty() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "refusing to delete {project}/{package}: open submit request(s) {} involve it (a delete would revoke them as source / decline them as target); supersede, accept, or decline them first, or pass force",
                        blocking.join(", ")
                    ),
                ));
            }
        }
        let mut query: Vec<(&str, &str)> = Vec::new();
        if force {
            query.push(("force", "1"));
        }
        let url = build_url(&self.base, &["source", project, package], &query);
        let body = self.http.delete(&url, &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS delete response for package {project}/{package}"),
            ))
        }
    }

    /// Create project `project` on OBS (`PUT /source/<project>/_meta`)
    /// with the given title and description. Requires credentials;
    /// without them the call fails loudly before any request. In
    /// dry-run the creation is recorded in the plan and nothing is
    /// created — the guard runs before the credentials check, like
    /// `delete_package`, so planning a creation needs no credentials.
    ///
    /// An agent creates a branch project before filing a submit
    /// request — a fresh project per update, so the accepted branch
    /// dies on accept and never piles up — so project creation
    /// belongs in the library, not in test setup: freshcrab needs
    /// exactly this operation when it migrates its branch creation
    /// onto geectl. Success is the `<status code="ok"/>` body: any
    /// other body is a server error, never success.
    pub fn create_project(
        &self,
        project: &str,
        title: &str,
        description: &str,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        // The dry-run guard: in dry-run the creation is recorded in
        // the plan and nothing is created — not even the credentials
        // check runs. Without this, a dry-run caller invoking create
        // directly would mutate the server.
        if !dry_run.allow(&format!("create project {project}")) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot create a project without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let meta = format!(
            "<project name=\"{}\"><title>{}</title><description>{}</description></project>",
            xml_escape(project),
            xml_escape(title),
            xml_escape(description),
        );
        let url = build_url(&self.base, &["source", project, "_meta"], &[]);
        let body = self.http.put(&url, &meta, &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS create-project response for project {project}"),
            ))
        }
    }

    /// Create package `package` in project `project` on OBS (`PUT
    /// /source/<project>/<package>/_meta`) with the given title and
    /// description. Requires credentials; without them the call fails
    /// loudly before any request. In dry-run the creation is recorded
    /// in the plan and nothing is created — the guard runs before the
    /// credentials check, like `delete_package`.
    ///
    /// An agent creates the package it submits from — the branch
    /// package — so package creation belongs in the library next to
    /// [`ObsClient::create_project`]: freshcrab needs exactly this
    /// operation when it migrates its branch creation onto geectl.
    /// Success is the `<status code="ok"/>` body: any other body is
    /// a server error, never success.
    pub fn create_package(
        &self,
        project: &str,
        package: &str,
        title: &str,
        description: &str,
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
        // The dry-run guard: in dry-run the creation is recorded in
        // the plan and nothing is created — not even the credentials
        // check runs. Without this, a dry-run caller invoking create
        // directly would mutate the server.
        if !dry_run.allow(&format!("create package {project}/{package}")) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot create a package without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let meta = format!(
            "<package name=\"{}\" project=\"{}\"><title>{}</title><description>{}</description></package>",
            xml_escape(package),
            xml_escape(project),
            xml_escape(title),
            xml_escape(description),
        );
        let url = build_url(&self.base, &["source", project, package, "_meta"], &[]);
        let body = self.http.put(&url, &meta, &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS create-package response for package {project}/{package}"),
            ))
        }
    }

    /// Upload file `filename` with `content` into package `package`
    /// in project `project` on OBS (`PUT
    /// /source/<project>/<package>/<filename>`). Requires credentials;
    /// without them the call fails loudly before any request. In
    /// dry-run the upload is recorded in the plan and nothing is
    /// uploaded — the guard runs before the credentials check, like
    /// `delete_package` — and the result is `None`, like
    /// `SrFiling::file` in dry-run.
    ///
    /// An agent uploads the files it submits — the spec, the
    /// `.changes` entry — into the branch package before filing, so
    /// file upload belongs in the library next to
    /// [`ObsClient::create_project`]: freshcrab needs exactly this
    /// operation when it migrates its branch creation onto geectl.
    /// The content is text (a spec, a `.changes` entry, a service
    /// file); binary content is out of scope for this operation.
    /// Returns the new revision number. Success is the backend's
    /// `<revision rev="N" .../>` document, not a `<status code="ok"/>`:
    /// the API streams the backend's answer back
    /// (`SourcePackageController#update_file` passes the request
    /// through to `bs_srcserver::putfile`, which returns the new
    /// revision), so any other body is a server error, never success.
    pub fn upload_file(
        &self,
        project: &str,
        package: &str,
        filename: &str,
        content: &str,
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
        if !valid_obs_filename(filename) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS filename: {filename:?}"),
            ));
        }
        // The dry-run guard: in dry-run the upload is recorded in the
        // plan and nothing is uploaded — not even the credentials
        // check runs. Without this, a dry-run caller invoking upload
        // directly would mutate the server.
        if !dry_run.allow(&format!("upload file {project}/{package}/{filename}")) {
            return Ok(None);
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot upload a file without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(&self.base, &["source", project, package, filename], &[]);
        let body = self.http.put_file(&url, content, &self.credentials)?;
        match parse_revision(&body) {
            Some(rev) => Ok(Some(rev)),
            None => Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS upload response for file {project}/{package}/{filename}"),
            )),
        }
    }

    /// GET a public directory listing (`/public/source/...`) and
    /// parse it. A public read: no credentials. The path is the
    /// segments after `/public/source` (a project, or a
    /// project/package pair).
    fn public_directory(&self, path: &[&str]) -> Result<DirectoryListing, GeectlError> {
        let mut segments: Vec<&str> = vec!["public", "source"];
        segments.extend_from_slice(path);
        let url = build_url(&self.base, &segments, &[]);
        let body = self.http.get_with(&url, &Credentials::None)?;
        parse_directory(&body)
    }

    /// Read file `file` of package `package` in project `project`
    /// (`GET /public/source/<project>/<package>/<file>`), optionally
    /// at revision `rev` (`?rev=N`, which the backend forwards to the
    /// file at that revision). A public read: no credentials. A
    /// missing file is an error, never an empty string.
    pub fn cat(
        &self,
        project: &str,
        package: &str,
        file: &str,
        rev: Option<u64>,
    ) -> Result<String, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if !valid_obs_filename(file) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS filename: {file:?}"),
            ));
        }
        if let Some(0) = rev {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "invalid revision: 0",
            ));
        }
        let mut query: Vec<(&str, &str)> = Vec::new();
        let rev_text;
        if let Some(rev) = rev {
            rev_text = rev.to_string();
            query.push(("rev", &rev_text));
        }
        let url = build_url(
            &self.base,
            &["public", "source", project, package, file],
            &query,
        );
        self.http.get_with(&url, &Credentials::None)
    }

    /// List the files of package `package` in project `project`
    /// (`GET /public/source/<project>/<package>`). A public read: no
    /// credentials. The listing carries each file's name, size,
    /// modification time, and md5, in server order.
    pub fn list_files(&self, project: &str, package: &str) -> Result<Vec<SourceFile>, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        Ok(self.public_directory(&[project, package])?.entries)
    }

    /// List the packages of project `project` (`GET
    /// /public/source/<project>`). A public read: no credentials.
    /// Multibuild flavors (`pkg:flavor1`, `pkg:flavor2`) are separate
    /// entries, exactly as the listing carries them.
    pub fn list_packages(&self, project: &str) -> Result<Vec<String>, GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        let listing = self.public_directory(&[project])?;
        Ok(listing
            .entries
            .into_iter()
            .map(|entry| entry.name)
            .collect())
    }

    /// List the files of a package at a revision (`GET
    /// /public/source/<project>/<package>?rev=N`): the `rev`-carrying
    /// form of [`ObsClient::list_files`], for diffing two revisions.
    fn list_files_rev(
        &self,
        project: &str,
        package: &str,
        rev: Option<u64>,
    ) -> Result<Vec<SourceFile>, GeectlError> {
        let mut query: Vec<(&str, &str)> = Vec::new();
        let rev_text;
        if let Some(rev) = rev {
            rev_text = rev.to_string();
            query.push(("rev", &rev_text));
        }
        let url = build_url(&self.base, &["public", "source", project, package], &query);
        let body = self.http.get_with(&url, &Credentials::None)?;
        Ok(parse_directory(&body)?.entries)
    }

    /// Read the history of package `package` in project `project`
    /// (`GET /source/<project>/<package>/_history`), optionally
    /// restricted to `range`. The history has no public route, so the
    /// read uses the client's credentials (whatever they are — no
    /// fail-closed pre-check: a client without credentials gets the
    /// server's 401 as an error, like any other authenticated read).
    /// Entries come back newest first: the server lists them oldest
    /// first, so the log is reversed before it is returned; a range
    /// filter preserves the newest-first order.
    pub fn package_log(
        &self,
        project: &str,
        package: &str,
        range: Option<&RevisionRange>,
    ) -> Result<Vec<HistoryEntry>, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        let url = build_url(&self.base, &["source", project, package, "_history"], &[]);
        let body = self.http.get_with(&url, &self.credentials)?;
        let entries = parse_history(&body)?;
        let mut entries = match range {
            None => entries,
            Some(range) => filter_history(entries, range)?,
        };
        // The server lists oldest first; a log reads newest first.
        entries.reverse();
        Ok(entries)
    }

    /// The per-file diff of revision `rev` of package `package` in
    /// project `project` against the previous revision: the files
    /// added, modified, or deleted, each with a unified diff.
    /// Revision 1 has no previous revision, so its patch is empty.
    /// File contents are read as text; a binary file that differs
    /// fails the call (its contents cannot be rendered as a text
    /// diff). Added and deleted files always appear; a modified file
    /// appears only when its contents actually differ.
    pub fn revision_patch(
        &self,
        project: &str,
        package: &str,
        rev: u64,
    ) -> Result<RevisionPatch, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if rev == 0 {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "invalid revision: 0",
            ));
        }
        if rev == 1 {
            return Ok(RevisionPatch {
                rev,
                files: Vec::new(),
            });
        }
        let new_files = self.list_files_rev(project, package, Some(rev))?;
        let old_files = self.list_files_rev(project, package, Some(rev - 1))?;
        let mut patches = Vec::new();
        for new in &new_files {
            match old_files.iter().find(|old| old.name == new.name) {
                None => {
                    // Added: the whole file is new.
                    let content = self.cat(project, package, &new.name, Some(rev))?;
                    patches.push(FilePatch {
                        filename: new.name.clone(),
                        status: FilePatchStatus::Added,
                        diff: unified_diff("/dev/null", "", &format!("b/{}", new.name), &content),
                    });
                }
                Some(old) if old.md5 != new.md5 => {
                    let old_content = self.cat(project, package, &new.name, Some(rev - 1))?;
                    let new_content = self.cat(project, package, &new.name, Some(rev))?;
                    let diff = unified_diff(
                        &format!("a/{}", new.name),
                        &old_content,
                        &format!("b/{}", new.name),
                        &new_content,
                    );
                    // The md5s differ but the contents may not (a
                    // missing md5 reads as a difference): only a
                    // non-empty diff is a modification.
                    if !diff.is_empty() {
                        patches.push(FilePatch {
                            filename: new.name.clone(),
                            status: FilePatchStatus::Modified,
                            diff,
                        });
                    }
                }
                Some(_) => {}
            }
        }
        for old in &old_files {
            if !new_files.iter().any(|new| new.name == old.name) {
                let content = self.cat(project, package, &old.name, Some(rev - 1))?;
                patches.push(FilePatch {
                    filename: old.name.clone(),
                    status: FilePatchStatus::Deleted,
                    diff: unified_diff(&format!("a/{}", old.name), &content, "/dev/null", ""),
                });
            }
        }
        patches.sort_by(|a, b| a.filename.cmp(&b.filename));
        Ok(RevisionPatch {
            rev,
            files: patches,
        })
    }

    /// Read the meta of project `project` (`GET
    /// /public/source/<project>/_meta`). A public read: no
    /// credentials. The meta carries the title, description, url,
    /// scmsync, devel project, persons, groups, and repositories —
    /// the same operation regardless of whether the project is a
    /// plain project or a git-backed one: a scmsync project returns
    /// its typed `scmsync` element through this call.
    pub fn project_meta(&self, project: &str) -> Result<ProjectMeta, GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        let url = build_url(&self.base, &["public", "source", project, "_meta"], &[]);
        let body = self.http.get_with(&url, &Credentials::None)?;
        parse_project_meta(&body)
    }

    /// Read the meta of package `package` in project `project` (`GET
    /// /public/source/<project>/<package>/_meta`). A public read: no
    /// credentials. The meta carries the title, description, url,
    /// scmsync, releasename, devel project and package, persons, and
    /// groups — the same operation regardless of whether the package
    /// is a plain package or a git-backed one: a scmsync package
    /// returns its typed `scmsync` element through this call.
    pub fn package_meta(&self, project: &str, package: &str) -> Result<PackageMeta, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        let url = build_url(
            &self.base,
            &["public", "source", project, package, "_meta"],
            &[],
        );
        let body = self.http.get_with(&url, &Credentials::None)?;
        parse_package_meta(&body)
    }

    /// Search packages by substring (`GET
    /// /search/package?match=...`): the match is
    /// `contains(@name,'term') or contains(title,'term') or
    /// contains(description,'term')` — a substring match, not the
    /// exact match osc defaults to. Requires credentials; without
    /// them the search fails loudly before any request. An unexpected
    /// response (no `matches` count) is an error, never an empty
    /// list.
    pub fn search_packages(&self, term: &str) -> Result<Vec<PackageMatch>, GeectlError> {
        let term = validate_search_term(term)?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "searching packages requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let match_expr = format!(
            "contains(@name,'{term}') or contains(title,'{term}') or contains(description,'{term}')"
        );
        let url = build_url(
            &self.base,
            &["search", "package"],
            &[("match", &match_expr)],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_package_search(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS package search response for {term:?}"),
            )
        })
    }

    /// Search projects by substring (`GET
    /// /search/project?match=...`): the same substring match as
    /// [`ObsClient::search_packages`], over project names, titles,
    /// and descriptions. Requires credentials; an unexpected response
    /// is an error, never an empty list.
    pub fn search_projects(&self, term: &str) -> Result<Vec<ProjectMatch>, GeectlError> {
        let term = validate_search_term(term)?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "searching projects requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let match_expr = format!(
            "contains(@name,'{term}') or contains(title,'{term}') or contains(description,'{term}')"
        );
        let url = build_url(
            &self.base,
            &["search", "project"],
            &[("match", &match_expr)],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_project_search(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS project search response for {term:?}"),
            )
        })
    }

    /// The owners of package `package` in project `project` (`GET
    /// /search/owner?project=...&package=...`). Requires credentials;
    /// without them the search fails loudly before any request. An
    /// unexpected response (not a collection) is an error, never an
    /// empty list.
    pub fn owners(&self, project: &str, package: &str) -> Result<Vec<Owner>, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "searching owners requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(
            &self.base,
            &["search", "owner"],
            &[("project", project), ("package", package)],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_owners(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS owner search response for package {project}/{package}"),
            )
        })
    }

    /// The owners of binary `binary` (`GET
    /// /search/owner?binary=...`). Requires credentials; an
    /// unexpected response is an error, never an empty list.
    pub fn owners_for_binary(&self, binary: &str) -> Result<Vec<Owner>, GeectlError> {
        if !valid_obs_name(binary) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS binary name: {binary:?}"),
            ));
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "searching owners requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(&self.base, &["search", "owner"], &[("binary", binary)]);
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_owners(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS owner search response for binary {binary:?}"),
            )
        })
    }

    /// The maintainers of package `package` in project `project`:
    /// the owners filtered to role `maintainer`, as `(name, role)`
    /// pairs (persons and groups flattened).
    pub fn maintainers(&self, project: &str, package: &str) -> Result<Vec<NamedRole>, GeectlError> {
        let owners = self.owners(project, package)?;
        let mut maintainers = Vec::new();
        for owner in owners {
            maintainers.extend(
                owner
                    .persons
                    .into_iter()
                    .filter(|person| person.role == "maintainer"),
            );
            maintainers.extend(
                owner
                    .groups
                    .into_iter()
                    .filter(|group| group.role == "maintainer"),
            );
        }
        Ok(maintainers)
    }

    /// Read the person `login` (`GET /person/<login>`). Requires
    /// credentials; without them the read fails loudly before any
    /// request. An unknown login is an error, never an empty person.
    pub fn whois(&self, login: &str) -> Result<Person, GeectlError> {
        validate_login(login)?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "reading a person requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(&self.base, &["person", login], &[]);
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_person(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS person response for login {login:?}"),
            )
        })
    }

    /// Build the OBS search URL for open submit requests targeting
    /// `package`. The match expression is XPath over the request XML,
    /// restricted to `submit` actions: a request of another type that
    /// targets the package does not block filing.
    /// The package name goes into a single-quoted XPath string
    /// literal, so it must be a valid OBS package name
    /// (`[A-Za-z0-9_.+-:]`); anything else is rejected rather than
    /// quoted, because XPath has no string escape and a quote in the
    /// name would break out of the literal.
    pub fn search_url(&self, package: &str) -> Result<String, GeectlError> {
        if !valid_obs_name(package) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS package name: {package:?}"),
            ));
        }
        // The match narrows to actions targeting the package in the
        // open states. It does NOT filter by action type: the OBS
        // search engine has no `action/@type` in its whitelist and
        // refuses a match that uses it with 400. The submit filter
        // lives client-side in `parse_submit_request_ids`, on the
        // response the server actually returns.
        let match_expr = format!(
            "action/target/@package='{package}' and (state/@name='new' or state/@name='review')"
        );
        Ok(build_url(
            &self.base,
            &["search", "request"],
            &[("match", &match_expr)],
        ))
    }
}

/// Extract the devel project from an OBS directory listing
/// (`<devel project="..." package="..."/>`). None when the listing
/// carries no `<devel>` element. The element is found by tag and the
/// `project` attribute read within it, so the attribute order does
/// not matter.
///
/// The body must be a `<directory>` document: a body that is not one
/// (empty, truncated, or some other document) is an error, never
/// None — a truncated 200 body must not read as "no devel project".
/// A `<devel>` element without a `project` attribute is skipped, not
/// an error: only the directory envelope itself is validated here.
pub(crate) fn parse_devel_project(body: &str) -> Result<Option<String>, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS directory response: not a <directory> document",
        )
    };
    let body = body.trim();
    let Some(after_open) = body.strip_prefix("<directory") else {
        return Err(malformed());
    };
    // The tag name must end here: `<directorylisting>` is not a
    // directory element. The next character is whitespace, `/`, or `>`.
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    // The opening tag ends at the first `>`.
    let Some(end) = after_open.find('>') else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<directory .../>`: no children, so no devel.
        return Ok(None);
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</directory>") else {
        return Err(malformed());
    };
    // Nothing but whitespace may follow the closing tag.
    if !inner[close + "</directory>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let mut rest = &inner[..close];
    while let Some(pos) = rest.find("<devel") {
        let after_tag = &rest[pos + "<devel".len()..];
        // The tag name must end here: `<development>` is not a devel
        // element. The next character is whitespace, `/`, or `>`.
        let Some(first) = after_tag.chars().next() else {
            return Err(malformed());
        };
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        // The element ends at the first `>`: an unterminated devel
        // element is a malformed listing, not "no devel project".
        let Some(end) = after_tag.find('>') else {
            return Err(malformed());
        };
        let element = &after_tag[..end];
        if let Some(attr_pos) = element.find("project=\"") {
            let start = attr_pos + "project=\"".len();
            if let Some(quote) = element[start..].find('"') {
                return Ok(Some(element[start..start + quote].to_string()));
            }
        }
        rest = &after_tag[end..];
    }
    Ok(None)
}

/// Extract the Version field from a spec file. Returns None if absent.
pub(crate) fn parse_spec_version(spec_text: &str) -> Option<String> {
    for line in spec_text.lines() {
        if let Some(rest) = line.strip_prefix("Version:") {
            let version = rest.trim().to_string();
            if !version.is_empty() {
                return Some(version);
            }
        }
    }
    None
}

/// Extract the `matches` count from an OBS search response
/// (`<collection matches="N">...</collection>`). None when the
/// response does not carry a count.
pub(crate) fn parse_search_matches(body: &str) -> Option<u32> {
    let marker = "matches=\"";
    let pos = body.find(marker)?;
    let start = pos + marker.len();
    let end = body[start..].find('"')?;
    body[start..start + end].parse::<u32>().ok()
}

/// The numeric `id` attribute of a `<request ...>` opening tag's
/// text (the text between `<request` and `>`), read order-tolerantly:
/// the attribute may sit anywhere in the tag, in either quote style.
/// None when the tag carries no numeric id. The attribute name must
/// stand on its own — `valid="..."` or `grid="..."` must not match —
/// so the character before `id=` has to be whitespace.
fn request_tag_id(element: &str) -> Option<String> {
    let mut rest = element;
    while let Some(pos) = rest.find("id=") {
        if pos > 0 && !rest.as_bytes()[pos - 1].is_ascii_whitespace() {
            rest = &rest[pos + "id=".len()..];
            continue;
        }
        let after_eq = &rest[pos + "id=".len()..];
        let quote = after_eq.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let close = after_eq[1..].find(quote)?;
        let id = &after_eq[1..1 + close];
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        return Some(id.to_string());
    }
    None
}

/// Extract the request id from an OBS create-request response
/// (`<request id="1234567" ...>`). The id is read from inside the
/// request tag, so the attribute order does not matter and either
/// quote style works. None when the response carries no numeric id.
pub(crate) fn parse_request_id(body: &str) -> Option<String> {
    let mut rest = body;
    while let Some(pos) = rest.find("<request") {
        let after_tag = &rest[pos + "<request".len()..];
        // The tag name must end here: `<requests>` is not a request
        // element. The next character is whitespace, `/`, or `>`.
        let first = after_tag.chars().next()?;
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let end = after_tag.find('>')?;
        return request_tag_id(&after_tag[..end]);
    }
    None
}

/// Extract every request id from an OBS search response
/// (`<collection matches="N"><request id="N"/>...</collection>`).
/// None when the response is not a collection at all (no
/// `<collection` tag), when the collection carries no `matches`
/// count, when any request tag carries no numeric id, or when the
/// count says there are requests but none parsed: an unexpected
/// response is an error, never an empty list. An empty collection
/// (with its count) reads as an empty list. A non-numeric id anywhere
/// in the collection is also unexpected: dropping it silently could
/// hide a request that blocks a delete. The ids are the returned
/// page: the count is required so a malformed collection cannot read
/// as empty, but it is not compared against the number of ids — a
/// search that paginates would return fewer elements than the total,
/// and refusing on that would be a false refusal. The id is read
/// from inside each request tag, so the attribute order does not
/// matter and either quote style works.
pub(crate) fn parse_request_ids(body: &str) -> Option<Vec<String>> {
    if !body.contains("<collection") {
        return None;
    }
    let matches = parse_search_matches(body)?;
    let mut ids = Vec::new();
    let mut rest = body;
    while let Some(pos) = rest.find("<request") {
        let after_tag = &rest[pos + "<request".len()..];
        // The tag name must end here: `<requests>` is not a request
        // element. The next character is whitespace, `/`, or `>`.
        let first = after_tag.chars().next()?;
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        // The tag ends at the first `>`: an unterminated request tag
        // is a malformed collection, not a skippable element.
        let end = after_tag.find('>')?;
        let id = request_tag_id(&after_tag[..end])?;
        ids.push(id);
        rest = &after_tag[end..];
    }
    // The count says there are requests but none parsed: the server
    // changed shape underneath us, and an empty list here would let
    // the filing dedup pass and the delete pre-check proceed.
    if matches > 0 && ids.is_empty() {
        return None;
    }
    Some(ids)
}
/// The ids of the open SUBMIT requests in an OBS search response
/// (`<collection matches="N">` of full `<request>` elements): a
/// request counts only when one of its `<action>` elements has
/// `type="submit"`. The OBS search engine cannot filter by action
/// type — `action/@type` is not in its whitelist, and a match that
/// uses it is refused with 400 — so the type filter lives here, on
/// the response the server actually returns. The search match still
/// narrows to the package and the open states; this only drops the
/// non-submit requests it also returns. A request element is kept
/// only when its own block contains a submit action; a self-closing
/// request tag carries no action and never counts. None when the
/// response is not a collection at all, has no matches count, when
/// any request tag carries no numeric id, or when the count says
/// there are requests but none parsed — same fail-closed discipline
/// as [`parse_request_ids`]. A collection whose requests are all
/// non-submit reads as an empty list, not an error: that is the
/// answer "no open submit request".
pub(crate) fn parse_submit_request_ids(body: &str) -> Option<Vec<String>> {
    if !body.contains("<collection") {
        return None;
    }
    let matches = parse_search_matches(body)?;
    let mut ids = Vec::new();
    let mut parsed = 0usize;
    let mut rest = body;
    while let Some(pos) = rest.find("<request") {
        let after_tag = &rest[pos + "<request".len()..];
        // The tag name must end here: `<requests>` is not a request
        // element. The next character is whitespace, `/`, or `>`.
        let first = after_tag.chars().next()?;
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        // The tag ends at the first `>`: an unterminated request tag
        // is a malformed collection, not a skippable element.
        let end = after_tag.find('>')?;
        let tag = &after_tag[..end];
        let id = request_tag_id(tag)?;
        parsed += 1;
        if tag.trim_end().ends_with('/') {
            // Self-closing: no children, so no action.
            rest = &after_tag[end..];
            continue;
        }
        let inner = &after_tag[end + 1..];
        let close = inner.find("</request>")?;
        if request_block_has_submit_action(&inner[..close]) {
            ids.push(id);
        }
        rest = &inner[close + "</request>".len()..];
    }
    // The count says there are requests but none parsed: the server
    // changed shape underneath us, and an empty list here would let
    // the filing dedup pass. Non-submit requests do parse, so a
    // collection of only those reads as an empty list, not None.
    if matches > 0 && parsed == 0 {
        return None;
    }
    Some(ids)
}

/// True when a `<request>` element's inner XML contains an `<action>`
/// element whose `type` attribute is `submit`. The `type` is read
/// from inside the action tag, so its position does not matter and
/// either quote style works; a mention of "submit" elsewhere in the
/// block (a description, say) does not count.
fn request_block_has_submit_action(block: &str) -> bool {
    let mut rest = block;
    while let Some(pos) = rest.find("<action") {
        let after_tag = &rest[pos + "<action".len()..];
        let Some(first) = after_tag.chars().next() else {
            return false;
        };
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let Some(end) = after_tag.find('>') else {
            return false;
        };
        if action_tag_type(&after_tag[..end]).as_deref() == Some("submit") {
            return true;
        }
        rest = &after_tag[end..];
    }
    false
}

/// The `type` attribute of an `<action ...>` opening tag's text,
/// read order-tolerantly in either quote style. None when the tag
/// carries no `type` attribute. The attribute name must stand on its
/// own — `subtype="..."` must not match — so the character before
/// `type=` has to be whitespace.
fn action_tag_type(element: &str) -> Option<String> {
    let mut rest = element;
    while let Some(pos) = rest.find("type=") {
        if pos > 0 && !rest.as_bytes()[pos - 1].is_ascii_whitespace() {
            rest = &rest[pos + "type=".len()..];
            continue;
        }
        let after_eq = &rest[pos + "type=".len()..];
        let quote = after_eq.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let close = after_eq[1..].find(quote)?;
        let value = &after_eq[1..1 + close];
        if value.is_empty() {
            return None;
        }
        return Some(value.to_string());
    }
    None
}

/// True when an OBS response body carries `<status code="ok" ...>`.
/// The write endpoints (request `changestate`, source delete) answer
/// `render_ok` on success; any other status code in the body — or no
/// status element at all — reads as failure, never success.
pub(crate) fn parse_status_ok(body: &str) -> bool {
    let Some(pos) = body.find("<status") else {
        return false;
    };
    let after_tag = &body[pos + "<status".len()..];
    // The tag name must end here: `<statuscode>` is not a status
    // element. The next character is whitespace, `/`, or `>`.
    let first = match after_tag.chars().next() {
        Some(c) => c,
        None => return false,
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return false;
    }
    // The code must be an attribute of this element, not text after it
    // — and it must be the `code` attribute, not a longer name that
    // ends in it (`mycode="ok"` is not `code="ok"`).
    let Some(end) = after_tag.find('>') else {
        return false;
    };
    let element = &after_tag[..end];
    let mut rest = element;
    while let Some(pos) = rest.find("code=") {
        if pos > 0 && !rest.as_bytes()[pos - 1].is_ascii_whitespace() {
            rest = &rest[pos + "code=".len()..];
            continue;
        }
        let after_eq = &rest[pos + "code=".len()..];
        let Some(quote) = after_eq.chars().next() else {
            return false;
        };
        if quote != '"' && quote != '\'' {
            return false;
        }
        let Some(close) = after_eq[1..].find(quote) else {
            return false;
        };
        return &after_eq[1..1 + close] == "ok";
    }
    false
}

/// Parse the revision number out of a backend `<revision>` document
/// (the response to a file upload). Returns the revision number, or
/// None when the body is not a revision document. The backend returns
/// the new revision after storing a file (`bs_srcserver::putfile`
/// returns `($rev, $BSXML::revision)`); the API streams it back
/// through `pass_to_backend`, so the upload response is a revision
/// document, not a `<status code="ok"/>` — `parse_status_ok` would
/// read every successful upload as a failure.
pub(crate) fn parse_revision(body: &str) -> Option<u64> {
    let pos = body.find("<revision")?;
    let after_tag = &body[pos + "<revision".len()..];
    // The tag name must end here: `<revisions>` is not a revision
    // element. The next character is whitespace, `/`, or `>`.
    let first = after_tag.chars().next()?;
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return None;
    }
    // The revision number is the `rev` attribute of this element, not
    // text after it — and it must be the `rev` attribute, not a longer
    // name that ends in it (`vrev="3"` is not `rev`).
    let end = after_tag.find('>')?;
    let element = &after_tag[..end];
    let mut rest = element;
    while let Some(pos) = rest.find("rev=") {
        if pos > 0 && !rest.as_bytes()[pos - 1].is_ascii_whitespace() {
            rest = &rest[pos + "rev=".len()..];
            continue;
        }
        let after_eq = &rest[pos + "rev=".len()..];
        let quote = after_eq.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let close = after_eq[1..].find(quote)?;
        let value = &after_eq[1..1 + close];
        let number: u64 = value.parse().ok()?;
        if number == 0 {
            return None;
        }
        return Some(number);
    }
    None
}

/// The value of attribute `name` in an opening tag's text (the text
/// between `<tag` and `>`), read order-tolerantly in either quote
/// style. None when the tag carries no such attribute. The attribute
/// name must stand on its own — `myname="..."` must not match — so
/// the character before `name=` has to be whitespace.
pub(crate) fn tag_attr(element: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=");
    let mut rest = element;
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
        let value = &after_eq[1..1 + close];
        if value.is_empty() {
            return None;
        }
        return Some(value.to_string());
    }
    None
}

/// Parse an OBS source directory listing (`<directory>...</directory>`
/// with `<entry name md5 size mtime .../>` children and an optional
/// `<linkinfo project package/>`). The body must be a `<directory>`
/// document: a body that is not one (empty, truncated, or some other
/// document) is an error, never an empty listing — a truncated 200
/// body must not read as "no files". An `<entry>` without a `name`
/// attribute, or with a `size`/`mtime` that is not a number, is a
/// malformed listing, not a skippable entry. Other children (`<devel
/// .../>` and the like) are ignored: this parser reads files and the
/// link target only.
pub(crate) fn parse_directory(body: &str) -> Result<DirectoryListing, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS directory response: not a <directory> document",
        )
    };
    let body = body.trim();
    let Some(after_open) = body.strip_prefix("<directory") else {
        return Err(malformed());
    };
    // The tag name must end here: `<directorylisting>` is not a
    // directory element. The next character is whitespace, `/`, or `>`.
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    // The opening tag ends at the first `>`.
    let Some(end) = after_open.find('>') else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<directory .../>`: no children, so no files.
        return Ok(DirectoryListing {
            entries: Vec::new(),
            linkinfo: None,
        });
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</directory>") else {
        return Err(malformed());
    };
    // Nothing but whitespace may follow the closing tag.
    if !inner[close + "</directory>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut entries = Vec::new();
    let mut rest = inner;
    while let Some(pos) = rest.find("<entry") {
        let after_tag = &rest[pos + "<entry".len()..];
        // The tag name must end here: `<entryname>` is not an entry
        // element. The next character is whitespace, `/`, or `>`.
        let Some(first) = after_tag.chars().next() else {
            return Err(malformed());
        };
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        // The element ends at the first `>`: an unterminated entry is
        // a malformed listing, not a skippable element.
        let Some(end) = after_tag.find('>') else {
            return Err(malformed());
        };
        let element = &after_tag[..end];
        let name = tag_attr(element, "name").ok_or_else(malformed)?;
        let size = match tag_attr(element, "size") {
            Some(value) => Some(value.parse::<u64>().map_err(|_| malformed())?),
            None => None,
        };
        let mtime = match tag_attr(element, "mtime") {
            Some(value) => Some(value.parse::<u64>().map_err(|_| malformed())?),
            None => None,
        };
        let md5 = tag_attr(element, "md5").map(|value| xml_unescape(&value));
        entries.push(SourceFile {
            name: xml_unescape(&name),
            size,
            mtime,
            md5,
        });
        rest = &after_tag[end..];
    }
    let mut linkinfo = None;
    let mut rest = inner;
    while let Some(pos) = rest.find("<linkinfo") {
        let after_tag = &rest[pos + "<linkinfo".len()..];
        let Some(first) = after_tag.chars().next() else {
            return Err(malformed());
        };
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let Some(end) = after_tag.find('>') else {
            return Err(malformed());
        };
        let element = &after_tag[..end];
        if let (Some(project), Some(package)) =
            (tag_attr(element, "project"), tag_attr(element, "package"))
        {
            // The first linkinfo wins: a listing carries at most one.
            linkinfo = Some((xml_unescape(&project), xml_unescape(&package)));
            break;
        }
        rest = &after_tag[end..];
    }
    Ok(DirectoryListing { entries, linkinfo })
}

/// Restrict `entries` (server order, oldest first) to `range`.
/// A numeric bound compares revision numbers; an md5 bound resolves
/// to the revision carrying that md5 (an unknown md5 is an error,
/// never an empty list). A one-sided range from a single specifier
/// matches exactly that revision; `"a:b"` matches `a..=b`, and a
/// reversed range is an error.
fn filter_history(
    entries: Vec<HistoryEntry>,
    range: &RevisionRange,
) -> Result<Vec<HistoryEntry>, GeectlError> {
    let resolve = |spec: &RevisionSpec| -> Result<u64, GeectlError> {
        match spec {
            RevisionSpec::Rev(number) => Ok(*number),
            RevisionSpec::Md5(md5) => entries
                .iter()
                .find(|entry| entry.srcmd5.as_deref() == Some(md5.as_str()))
                .map(|entry| entry.rev)
                .ok_or_else(|| {
                    GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!("revision md5 {md5:?} not found in the package history"),
                    )
                }),
        }
    };
    let (low, high) = match (&range.from, &range.to) {
        (None, None) => return Ok(entries),
        (Some(from), None) => {
            let rev = resolve(from)?;
            (rev, rev)
        }
        (None, Some(to)) => {
            let rev = resolve(to)?;
            (0, rev)
        }
        (Some(from), Some(to)) => {
            let from = resolve(from)?;
            let to = resolve(to)?;
            if from > to {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("reversed revision range: {from}..{to}"),
                ));
            }
            (from, to)
        }
    };
    Ok(entries
        .into_iter()
        .filter(|entry| entry.rev >= low && entry.rev <= high)
        .collect())
}

/// The text of the first `<tag>...</tag>` element in `block`, or
/// None when there is none. The text is XML-unescaped. A
/// self-closing `<tag/>` carries no text and reads as None.
fn element_text(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let mut rest = block;
    while let Some(pos) = rest.find(&open) {
        let after_tag = &rest[pos + open.len()..];
        let first = after_tag.chars().next()?;
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let end = after_tag.find('>')?;
        let open_text = &after_tag[..end];
        if open_text.trim_end().ends_with('/') {
            return None;
        }
        let inner = &after_tag[end + 1..];
        let close = format!("</{tag}>");
        let close_pos = inner.find(&close)?;
        return Some(xml_unescape(&inner[..close_pos]));
    }
    None
}

/// Parse an OBS package history (`<revisionlist>` with `<revision
/// rev vrev>` children carrying `<srcmd5>`, `<version>`, `<time>`,
/// `<user>`, `<comment>`, and `<requestid>` elements). The body must
/// be a `<revisionlist>` document: a body that is not one is an
/// error, never an empty history. A `<revision>` without numeric
/// `rev`/`vrev` attributes, or with a non-numeric `<time>`, is a
/// malformed history. The entries come back in server order (oldest
/// first); the parser preserves it.
pub(crate) fn parse_history(body: &str) -> Result<Vec<HistoryEntry>, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS history response: not a <revisionlist> document",
        )
    };
    let body = body.trim();
    let Some(after_open) = body.strip_prefix("<revisionlist") else {
        return Err(malformed());
    };
    // The tag name must end here: `<revisionlists>` is not a
    // revision list. The next character is whitespace, `/`, or `>`.
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = after_open.find('>') else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<revisionlist/>`: no revisions.
        return Ok(Vec::new());
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</revisionlist>") else {
        return Err(malformed());
    };
    if !inner[close + "</revisionlist>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut entries = Vec::new();
    let mut rest = inner;
    while let Some(pos) = rest.find("<revision") {
        let after_tag = &rest[pos + "<revision".len()..];
        // The tag name must end here: `<revisions>` is not a
        // revision element. The next character is whitespace, `/`,
        // or `>`.
        let Some(first) = after_tag.chars().next() else {
            return Err(malformed());
        };
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let Some(end) = after_tag.find('>') else {
            return Err(malformed());
        };
        let element = &after_tag[..end];
        let rev = match tag_attr(element, "rev") {
            Some(value) => value.parse::<u64>().map_err(|_| malformed())?,
            None => return Err(malformed()),
        };
        let vrev = match tag_attr(element, "vrev") {
            Some(value) => value.parse::<u64>().map_err(|_| malformed())?,
            None => return Err(malformed()),
        };
        let inner = &after_tag[end + 1..];
        let Some(close) = inner.find("</revision>") else {
            return Err(malformed());
        };
        let block = &inner[..close];
        let time = match element_text(block, "time") {
            Some(value) => Some(value.trim().parse::<u64>().map_err(|_| malformed())?),
            None => None,
        };
        entries.push(HistoryEntry {
            rev,
            vrev,
            srcmd5: element_text(block, "srcmd5"),
            version: element_text(block, "version"),
            time,
            user: element_text(block, "user"),
            comment: element_text(block, "comment"),
            request_id: element_text(block, "requestid"),
        });
        rest = &inner[close + "</revision>".len()..];
    }
    Ok(entries)
}

/// A unified diff of `old` against `new`, in the standard form:
/// `--- <old_path>` / `+++ <new_path>` headers, `@@ -a,b +c,d @@`
/// hunks with three lines of context, `-`/`+`/context lines. The
/// diff is computed on lines (LCS-based); a file that exists on one
/// side only is rendered with `/dev/null` on the missing side. The
/// LCS table is quadratic in the file sizes, so very large files are
/// expensive to diff — the callers diff package text files, which are
/// small. An empty diff (identical contents) is an empty string:
/// there is nothing to show.
pub(crate) fn unified_diff(old_path: &str, old: &str, new_path: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let n = old_lines.len();
    let m = new_lines.len();
    // LCS table: `table[i][j]` is the LCS length of the suffixes from
    // `i` and `j`.
    let mut table = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = if old_lines[i] == new_lines[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Op {
        Equal,
        Delete,
        Add,
    }
    let mut ops: Vec<(Op, &str)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old_lines[i] == new_lines[j] {
            ops.push((Op::Equal, old_lines[i]));
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            ops.push((Op::Delete, old_lines[i]));
            i += 1;
        } else {
            ops.push((Op::Add, new_lines[j]));
            j += 1;
        }
    }
    while i < n {
        ops.push((Op::Delete, old_lines[i]));
        i += 1;
    }
    while j < m {
        ops.push((Op::Add, new_lines[j]));
        j += 1;
    }
    if ops.iter().all(|(op, _)| *op == Op::Equal) {
        return String::new();
    }
    // Group into hunks with three lines of context: hunks merge when
    // the equal run between two changes is at most twice the context.
    const CONTEXT: usize = 3;
    let mut hunks: Vec<(usize, usize)> = Vec::new();
    let mut k = 0;
    while k < ops.len() {
        while k < ops.len() && ops[k].0 == Op::Equal {
            k += 1;
        }
        if k >= ops.len() {
            break;
        }
        let hunk_start = k.saturating_sub(CONTEXT);
        let mut end = k;
        while end < ops.len() {
            if ops[end].0 == Op::Equal {
                let run_start = end;
                while end < ops.len() && ops[end].0 == Op::Equal {
                    end += 1;
                }
                if end >= ops.len() {
                    // Trailing equals: only CONTEXT of them belong.
                    end = run_start + CONTEXT.min(end - run_start);
                    break;
                }
                if end - run_start > 2 * CONTEXT {
                    // Gap too wide: the hunk ends CONTEXT after the
                    // last change.
                    end = run_start + CONTEXT;
                    break;
                }
                // Short gap: absorbed, keep going.
            } else {
                end += 1;
            }
        }
        hunks.push((hunk_start, end));
        k = end;
    }
    let mut out = String::new();
    out.push_str(&format!("--- {old_path}\n+++ {new_path}\n"));
    for (start, end) in hunks {
        // Line numbers before the hunk, then the counts within it.
        let (mut old_line, mut new_line) = (0, 0);
        for (op, _) in &ops[..start] {
            match op {
                Op::Equal => {
                    old_line += 1;
                    new_line += 1;
                }
                Op::Delete => old_line += 1,
                Op::Add => new_line += 1,
            }
        }
        let (mut old_count, mut new_count) = (0, 0);
        for (op, _) in &ops[start..end] {
            match op {
                Op::Equal => {
                    old_count += 1;
                    new_count += 1;
                }
                Op::Delete => old_count += 1,
                Op::Add => new_count += 1,
            }
        }
        let fmt_range = |start: usize, count: usize| -> String {
            if count == 0 {
                // An empty range starts at the line before it.
                format!("{start},0")
            } else if count == 1 {
                format!("{}", start + 1)
            } else {
                format!("{},{}", start + 1, count)
            }
        };
        out.push_str(&format!(
            "@@ -{} +{} @@\n",
            fmt_range(old_line, old_count),
            fmt_range(new_line, new_count)
        ));
        for (op, line) in &ops[start..end] {
            let marker = match op {
                Op::Equal => ' ',
                Op::Delete => '-',
                Op::Add => '+',
            };
            out.push(marker);
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Each `<tag ...>...</tag>` (or self-closing `<tag .../>`) element
/// in `block`, as `(opening-tag text, inner text)`. The tag name must
/// stand on its own. An unterminated element is an error, never a
/// skipped element.
fn elements(block: &str, tag: &str) -> Result<Vec<(String, String)>, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected OBS response: malformed <{tag}> element"),
        )
    };
    let open = format!("<{tag}");
    let mut out = Vec::new();
    let mut rest = block;
    while let Some(pos) = rest.find(&open) {
        let after_tag = &rest[pos + open.len()..];
        let Some(first) = after_tag.chars().next() else {
            return Err(malformed());
        };
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let Some(end) = after_tag.find('>') else {
            return Err(malformed());
        };
        let element = &after_tag[..end];
        if element.trim_end().ends_with('/') {
            out.push((element.to_string(), String::new()));
            rest = &after_tag[end + 1..];
        } else {
            let inner = &after_tag[end + 1..];
            let close = format!("</{tag}>");
            let Some(close_pos) = inner.find(&close) else {
                return Err(malformed());
            };
            out.push((element.to_string(), inner[..close_pos].to_string()));
            rest = &inner[close_pos + close.len()..];
        }
    }
    Ok(out)
}

/// The `<person userid role/>` and `<group groupid role/>` children
/// of a meta document. An element without its name or role attribute
/// is skipped, not an error: only the meta envelope itself is
/// validated here.
fn meta_roles(block: &str) -> Result<(Vec<NamedRole>, Vec<NamedRole>), GeectlError> {
    let mut persons = Vec::new();
    for (element, _) in elements(block, "person")? {
        if let (Some(name), Some(role)) = (tag_attr(&element, "userid"), tag_attr(&element, "role"))
        {
            persons.push(NamedRole {
                name: xml_unescape(&name),
                role: xml_unescape(&role),
            });
        }
    }
    let mut groups = Vec::new();
    for (element, _) in elements(block, "group")? {
        if let (Some(name), Some(role)) =
            (tag_attr(&element, "groupid"), tag_attr(&element, "role"))
        {
            groups.push(NamedRole {
                name: xml_unescape(&name),
                role: xml_unescape(&role),
            });
        }
    }
    Ok((persons, groups))
}

/// Split a meta document into its root opening-tag text and inner
/// text. The body must be a `<root>` document: a body that is not
/// one (empty, truncated, or some other document) is an error, never
/// a meta with no name. A self-closing root reads as an empty inner
/// text.
fn meta_envelope<'a>(body: &'a str, root: &str) -> Result<(String, &'a str), GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected OBS meta response: not a <{root}> document"),
        )
    };
    let body = body.trim();
    let open = format!("<{root}");
    let Some(after_open) = body.strip_prefix(&open) else {
        return Err(malformed());
    };
    // The tag name must end here: `<packages>` is not a package
    // element. The next character is whitespace, `/`, or `>`.
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = after_open.find('>') else {
        return Err(malformed());
    };
    let open_tag = after_open[..end].to_string();
    if open_tag.trim_end().ends_with('/') {
        return Ok((open_tag, ""));
    }
    let inner = &after_open[end + 1..];
    let close = format!("</{root}>");
    let Some(close_pos) = inner.find(&close) else {
        return Err(malformed());
    };
    // Nothing but whitespace may follow the closing tag.
    if !inner[close_pos + close.len()..].trim().is_empty() {
        return Err(malformed());
    }
    Ok((open_tag, &inner[..close_pos]))
}

/// Parse a project meta (`<project>` with `title`, `description`,
/// `url?`, `scmsync?`, `devel?`, persons, groups, and
/// `<repository name><arch>*<path project repository/>` children).
/// The body must be a `<project>` document with a `name` attribute:
/// either is missing and the parse is an error, never a meta with
/// no name.
pub(crate) fn parse_project_meta(body: &str) -> Result<ProjectMeta, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS project meta response: not a <project> document",
        )
    };
    let (open_tag, inner) = meta_envelope(body, "project")?;
    let name = tag_attr(&open_tag, "name").ok_or_else(malformed)?;
    let devel = {
        let mut found = None;
        for (element, _) in elements(inner, "devel")? {
            if let Some(project) = tag_attr(&element, "project") {
                found = Some(xml_unescape(&project));
                break;
            }
        }
        found
    };
    let mut repositories = Vec::new();
    for (element, repo_inner) in elements(inner, "repository")? {
        let Some(repo_name) = tag_attr(&element, "name") else {
            continue;
        };
        let mut arch = Vec::new();
        for (_, text) in elements(&repo_inner, "arch")? {
            let arch_name = text.trim().to_string();
            if !arch_name.is_empty() {
                arch.push(xml_unescape(&arch_name));
            }
        }
        let mut paths = Vec::new();
        for (path_element, _) in elements(&repo_inner, "path")? {
            if let (Some(project), Some(repository)) = (
                tag_attr(&path_element, "project"),
                tag_attr(&path_element, "repository"),
            ) {
                paths.push((xml_unescape(&project), xml_unescape(&repository)));
            }
        }
        repositories.push(Repository {
            name: xml_unescape(&repo_name),
            arch,
            paths,
        });
    }
    let (persons, groups) = meta_roles(inner)?;
    Ok(ProjectMeta {
        name: xml_unescape(&name),
        title: element_text(inner, "title"),
        description: element_text(inner, "description"),
        url: element_text(inner, "url"),
        scmsync: element_text(inner, "scmsync"),
        devel,
        persons,
        groups,
        repositories,
    })
}

/// Parse a package meta (`<package>` with `title`, `description`,
/// `url?`, `scmsync?`, `releasename?`, `devel project package?`,
/// `<person userid role/>`, and `<group groupid role/>` children).
/// The body must be a `<package>` document with `name` and `project`
/// attributes: either is missing and the parse is an error, never a
/// meta with no name.
pub(crate) fn parse_package_meta(body: &str) -> Result<PackageMeta, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS package meta response: not a <package> document",
        )
    };
    let (open_tag, inner) = meta_envelope(body, "package")?;
    let name = tag_attr(&open_tag, "name").ok_or_else(malformed)?;
    let project = tag_attr(&open_tag, "project").ok_or_else(malformed)?;
    let devel = {
        let mut found = None;
        for (element, _) in elements(inner, "devel")? {
            if let (Some(devel_project), Some(devel_package)) =
                (tag_attr(&element, "project"), tag_attr(&element, "package"))
            {
                found = Some((xml_unescape(&devel_project), xml_unescape(&devel_package)));
                break;
            }
        }
        found
    };
    let (persons, groups) = meta_roles(inner)?;
    Ok(PackageMeta {
        name: xml_unescape(&name),
        project: xml_unescape(&project),
        title: element_text(inner, "title"),
        description: element_text(inner, "description"),
        url: element_text(inner, "url"),
        scmsync: element_text(inner, "scmsync"),
        releasename: element_text(inner, "releasename"),
        devel,
        persons,
        groups,
    })
}

/// True when `body` is an OBS search collection: it carries a
/// `<collection` tag and a `matches` count. The count is required so
/// a malformed collection cannot read as empty.
fn is_search_collection(body: &str) -> bool {
    body.contains("<collection") && parse_search_matches(body).is_some()
}

/// Parse a package search response (`<collection matches="N">` of
/// `<package name project>` elements with `<title>` and
/// `<description>` children). None when the response is not a
/// collection at all, carries no `matches` count, or says there are
/// matches but none parsed: an unexpected response is an error,
/// never an empty list. An empty collection (with its count) reads
/// as an empty list. A package element without a `name` or `project`
/// attribute is skipped, not an error: only the collection envelope
/// itself is validated here.
pub(crate) fn parse_package_search(body: &str) -> Option<Vec<PackageMatch>> {
    if !is_search_collection(body) {
        return None;
    }
    let matches_count = parse_search_matches(body)?;
    let mut matches = Vec::new();
    for (element, inner) in elements(body, "package").ok()? {
        let (Some(name), Some(project)) =
            (tag_attr(&element, "name"), tag_attr(&element, "project"))
        else {
            continue;
        };
        matches.push(PackageMatch {
            name: xml_unescape(&name),
            project: xml_unescape(&project),
            title: element_text(&inner, "title"),
            description: element_text(&inner, "description"),
        });
    }
    // The count says there are matches but none parsed: the server
    // changed shape underneath us, and an empty list here would hide
    // results.
    if matches_count > 0 && matches.is_empty() {
        return None;
    }
    Some(matches)
}

/// Parse a project search response (`<collection matches="N">` of
/// `<project name>` elements with `<title>` and `<description>`
/// children). None on the same fail-closed terms as
/// [`parse_package_search`].
pub(crate) fn parse_project_search(body: &str) -> Option<Vec<ProjectMatch>> {
    if !is_search_collection(body) {
        return None;
    }
    let matches_count = parse_search_matches(body)?;
    let mut matches = Vec::new();
    for (element, inner) in elements(body, "project").ok()? {
        let Some(name) = tag_attr(&element, "name") else {
            continue;
        };
        matches.push(ProjectMatch {
            name: xml_unescape(&name),
            title: element_text(&inner, "title"),
            description: element_text(&inner, "description"),
        });
    }
    if matches_count > 0 && matches.is_empty() {
        return None;
    }
    Some(matches)
}

/// Validate a search term: non-empty, and without a single quote —
/// the term goes into a single-quoted XPath string literal, where a
/// quote would break out of the literal (XPath has no string
/// escape).
fn validate_search_term(term: &str) -> Result<&str, GeectlError> {
    if term.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "search term must not be empty",
        ));
    }
    if term.contains('\'') {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("search term must not contain a single quote: {term:?}"),
        ));
    }
    Ok(term)
}

/// Parse an owner search response (`<collection>` of `<owner
/// rootproject project package?>` elements with `<person name
/// role/>` and `<group name role/>` children — `name`, not
/// `userid`). None when the response is not a collection at all, or
/// when a `matches` count is present but does not parse: an
/// unexpected response is an error, never an empty list. An owner
/// element without its `rootproject` or `project` attribute is
/// skipped, not an error: only the collection envelope itself is
/// validated here. The `matches` count is not required: the owner
/// search renders a collection either way, and refusing on a missing
/// count would refuse a valid empty answer.
pub(crate) fn parse_owners(body: &str) -> Option<Vec<Owner>> {
    if !body.contains("<collection") {
        return None;
    }
    if body.contains("matches=") && parse_search_matches(body).is_none() {
        return None;
    }
    let mut owners = Vec::new();
    for (element, inner) in elements(body, "owner").ok()? {
        let (Some(root_project), Some(project)) = (
            tag_attr(&element, "rootproject"),
            tag_attr(&element, "project"),
        ) else {
            continue;
        };
        let package = tag_attr(&element, "package").map(|value| xml_unescape(&value));
        let mut persons = Vec::new();
        for (person_element, _) in elements(&inner, "person").ok()? {
            if let (Some(name), Some(role)) = (
                tag_attr(&person_element, "name"),
                tag_attr(&person_element, "role"),
            ) {
                persons.push(NamedRole {
                    name: xml_unescape(&name),
                    role: xml_unescape(&role),
                });
            }
        }
        let mut groups = Vec::new();
        for (group_element, _) in elements(&inner, "group").ok()? {
            if let (Some(name), Some(role)) = (
                tag_attr(&group_element, "name"),
                tag_attr(&group_element, "role"),
            ) {
                groups.push(NamedRole {
                    name: xml_unescape(&name),
                    role: xml_unescape(&role),
                });
            }
        }
        owners.push(Owner {
            root_project: xml_unescape(&root_project),
            project: xml_unescape(&project),
            package,
            persons,
            groups,
        });
    }
    Some(owners)
}

/// Parse a person document (`<person>` with `<login>`, `<email>`,
/// `<realname?>`, `<state>`, and `<globalrole>*` children). None
/// when the body is not a `<person>` document, or when it carries no
/// login, email, or state: a person without an identity is an
/// unexpected response, never a person with empty fields.
pub(crate) fn parse_person(body: &str) -> Option<Person> {
    let (_, inner) = meta_envelope(body, "person").ok()?;
    let token = |tag: &str| -> Option<String> {
        element_text(inner, tag)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    Some(Person {
        login: token("login")?,
        email: token("email")?,
        realname: token("realname"),
        state: token("state")?,
    })
}

/// Validate a login name against the server's format: 2–100
/// characters, from `[\w $\^\-.#*+&'"]` (word characters plus space,
/// `$`, `^`, `-`, `.`, `#`, `*`, `+`, `&`, `'`, `"`).
fn validate_login(login: &str) -> Result<(), GeectlError> {
    let invalid = || {
        GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid OBS login name: {login:?}"),
        )
    };
    let len = login.chars().count();
    if !(2..=100).contains(&len) {
        return Err(invalid());
    }
    let allowed = |c: char| {
        c.is_ascii_alphanumeric()
            || c == '_'
            || matches!(
                c,
                ' ' | '$' | '^' | '-' | '.' | '#' | '*' | '+' | '&' | '\'' | '"'
            )
    };
    if !login.chars().all(allowed) {
        return Err(invalid());
    }
    Ok(())
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
        let xml = self.to_xml();
        check_well_formed(&xml)?;
        // The rendered XML must carry exactly one <sourceupdate> with
        // a value the server accepts. The value comes from the
        // `SourceUpdate` enum, so it is always one of the three —
        // this check exists so a future edit to `to_xml` that drops
        // or mangles the attribute fails here, at payload build time,
        // instead of at the API.
        match sourceupdate_value(&xml) {
            Some(value) if ["update", "noupdate", "cleanup"].contains(&value) => Ok(()),
            Some(value) => Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("malformed SR XML: unknown <sourceupdate> value {value:?}"),
            )),
            // A missing <sourceupdate> is refused by check_well_formed
            // above, so None here means the element is duplicated.
            None => Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "malformed SR XML: duplicated <sourceupdate> in <action>",
            )),
        }
    }
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

/// Escape text for an XML element or attribute value.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Unescape XML text or an attribute value: the five entities
/// [`xml_escape`] produces, plus numeric character references
/// (`&#65;`, `&#x41;`). Anything else is left verbatim: an unknown
/// entity is a server response the library does not understand, and
/// passing it through beats inventing a character.
pub(crate) fn xml_unescape(s: &str) -> String {
    // Fast path: nothing to unescape.
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        match after.find(';') {
            Some(semi) => {
                let entity = &after[..semi];
                match entity {
                    "amp" => out.push('&'),
                    "lt" => out.push('<'),
                    "gt" => out.push('>'),
                    "quot" => out.push('"'),
                    "apos" => out.push('\''),
                    _ => {
                        let numeric = entity
                            .strip_prefix("#x")
                            .or_else(|| entity.strip_prefix("#X"))
                            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                            .or_else(|| {
                                entity
                                    .strip_prefix('#')
                                    .and_then(|dec| dec.parse::<u32>().ok())
                            })
                            .and_then(char::from_u32);
                        match numeric {
                            Some(c) => out.push(c),
                            // Unknown entity: keep it verbatim.
                            None => {
                                out.push('&');
                                out.push_str(entity);
                                out.push(';');
                            }
                        }
                    }
                }
                rest = &after[semi + 1..];
            }
            None => {
                // A bare `&` with no `;` after it: keep it verbatim.
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Check that `xml` is well-formed: every tag is balanced and properly
/// nested, there is exactly one root element, and it is `request` with
/// the submit action, source, target, options, and description in the
/// expected nesting. Returns the first problem found.
fn check_well_formed(xml: &str) -> Result<(), GeectlError> {
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
    // Required structure: exactly one root `request`, containing an
    // `action` (which holds `source`, `target`, and `options` with the
    // source-update value) and a `description`.
    let has = |path: &str| paths.iter().any(|p| p == path);
    if !has("request") {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "malformed SR XML: root element is not <request>",
        ));
    }
    if !has("request/action") {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "malformed SR XML: missing <action> in <request>",
        ));
    }
    if !has("request/action/source") {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "malformed SR XML: missing <source> in <action>",
        ));
    }
    if !has("request/action/target") {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "malformed SR XML: missing <target> in <action>",
        ));
    }
    if !has("request/action/options") {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "malformed SR XML: missing <options> in <action>",
        ));
    }
    if !has("request/action/options/sourceupdate") {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "malformed SR XML: missing <sourceupdate> in <options>",
        ));
    }
    if !has("request/description") {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "malformed SR XML: missing <description> in <request>",
        ));
    }
    Ok(())
}

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
}

impl<'a> SrFiling<'a> {
    pub fn new(obs: &'a ObsClient<'a>, dry_run: &'a DryRun) -> Self {
        SrFiling { obs, dry_run }
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
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "an open submit request already targets {} ({blocking}); not filing a duplicate",
                    payload.target_package
                ),
            ));
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// A fake HTTP client with canned responses, for fixtures. GET,
    /// POST, and DELETE are keyed separately by URL; a URL with no
    /// canned response is a transport error. Calls are recorded so
    /// tests can assert on the method, the URL, whether auth was
    /// sent, and (for POST) the body.
    struct FakeHttp {
        gets: RefCell<HashMap<String, String>>,
        get_errors: RefCell<HashMap<String, ErrorKind>>,
        posts: RefCell<HashMap<String, String>>,
        deletes: RefCell<HashMap<String, String>>,
        puts: RefCell<HashMap<String, String>>,
        get_calls: RefCell<Vec<(String, bool)>>,
        post_calls: RefCell<Vec<(String, String, bool)>>,
        delete_calls: RefCell<Vec<(String, bool)>>,
        put_calls: RefCell<Vec<(String, String, bool)>>,
    }

    impl FakeHttp {
        fn new() -> Self {
            FakeHttp {
                gets: RefCell::new(HashMap::new()),
                get_errors: RefCell::new(HashMap::new()),
                posts: RefCell::new(HashMap::new()),
                deletes: RefCell::new(HashMap::new()),
                puts: RefCell::new(HashMap::new()),
                get_calls: RefCell::new(Vec::new()),
                post_calls: RefCell::new(Vec::new()),
                delete_calls: RefCell::new(Vec::new()),
                put_calls: RefCell::new(Vec::new()),
            }
        }

        fn with_get(self, url: &str, body: &str) -> Self {
            self.gets
                .borrow_mut()
                .insert(url.to_string(), body.to_string());
            self
        }

        fn with_get_error(self, url: &str, kind: ErrorKind) -> Self {
            self.get_errors.borrow_mut().insert(url.to_string(), kind);
            self
        }

        fn with_post(self, url: &str, body: &str) -> Self {
            self.posts
                .borrow_mut()
                .insert(url.to_string(), body.to_string());
            self
        }

        fn with_delete(self, url: &str, body: &str) -> Self {
            self.deletes
                .borrow_mut()
                .insert(url.to_string(), body.to_string());
            self
        }

        fn with_put(self, url: &str, body: &str) -> Self {
            self.puts
                .borrow_mut()
                .insert(url.to_string(), body.to_string());
            self
        }

        fn get_calls(&self) -> Vec<(String, bool)> {
            self.get_calls.borrow().clone()
        }

        fn post_calls(&self) -> Vec<(String, String, bool)> {
            self.post_calls.borrow().clone()
        }

        fn delete_calls(&self) -> Vec<(String, bool)> {
            self.delete_calls.borrow().clone()
        }

        fn put_calls(&self) -> Vec<(String, String, bool)> {
            self.put_calls.borrow().clone()
        }
    }

    impl HttpClient for FakeHttp {
        fn get(&self, url: &str) -> Result<String, GeectlError> {
            self.get_with(url, &Credentials::None)
        }

        fn get_with(&self, url: &str, credentials: &Credentials) -> Result<String, GeectlError> {
            let authed = !matches!(credentials, Credentials::None);
            self.get_calls.borrow_mut().push((url.to_string(), authed));
            if let Some(kind) = self.get_errors.borrow().get(url).cloned() {
                return Err(GeectlError::new(
                    kind,
                    format!("canned GET error for {url}"),
                ));
            }
            self.gets.borrow().get(url).cloned().ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::NetworkError,
                    format!("no canned GET response for {url}"),
                )
            })
        }

        fn post(
            &self,
            url: &str,
            body: &str,
            credentials: &Credentials,
        ) -> Result<String, GeectlError> {
            let authed = !matches!(credentials, Credentials::None);
            self.post_calls
                .borrow_mut()
                .push((url.to_string(), body.to_string(), authed));
            self.posts.borrow().get(url).cloned().ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::NetworkError,
                    format!("no canned POST response for {url}"),
                )
            })
        }

        fn post_json(&self, url: &str, body: &str) -> Result<String, GeectlError> {
            // The fake has no auth seam, so post_json resolves to no
            // credentials — the same delegation as post_text. Fixtures
            // do not distinguish content types: the transport tests in
            // `http.rs` assert the header each method sends.
            self.post(url, body, &Credentials::None)
        }

        fn post_text(
            &self,
            url: &str,
            body: &str,
            credentials: &Credentials,
        ) -> Result<String, GeectlError> {
            // Fixtures do not distinguish content types: the transport
            // tests in `http.rs` assert the header each method sends.
            self.post(url, body, credentials)
        }

        fn delete(&self, url: &str, credentials: &Credentials) -> Result<String, GeectlError> {
            let authed = !matches!(credentials, Credentials::None);
            self.delete_calls
                .borrow_mut()
                .push((url.to_string(), authed));
            self.deletes.borrow().get(url).cloned().ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::NetworkError,
                    format!("no canned DELETE response for {url}"),
                )
            })
        }

        fn put(
            &self,
            url: &str,
            body: &str,
            credentials: &Credentials,
        ) -> Result<String, GeectlError> {
            // Fixtures do not distinguish content types: the transport
            // tests in `http.rs` assert the header PUT sends.
            let authed = !matches!(credentials, Credentials::None);
            self.put_calls
                .borrow_mut()
                .push((url.to_string(), body.to_string(), authed));
            self.puts.borrow().get(url).cloned().ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::NetworkError,
                    format!("no canned PUT response for {url}"),
                )
            })
        }

        fn put_file(
            &self,
            url: &str,
            body: &str,
            credentials: &Credentials,
        ) -> Result<String, GeectlError> {
            // Fixtures do not distinguish content types: the transport
            // tests in `http.rs` assert the header each method sends.
            self.put(url, body, credentials)
        }
    }

    fn basic_credentials() -> Credentials {
        Credentials::Basic {
            username: "scarabeus_iv".to_string(),
            password: "secret".to_string(),
        }
    }

    fn payload() -> SrPayload {
        SrPayload::new(
            "home:scarabeus_iv",
            "foo",
            "devel:languages:python",
            "foo",
            "Update foo to 2.0",
            "-------------------------------------------------------------------\nFri Nov 20 07:00:00 UTC 2026 - Tomáš Chvátal <tomas.chvatal@gmail.com>\n\n- Update to version 2.0.\n",
            &[],
        )
        .unwrap()
    }

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

    // --- valid_obs_name ---

    #[test]
    fn valid_obs_name_accepts_and_rejects() {
        assert!(valid_obs_name("python3-requests"));
        assert!(valid_obs_name("foo:bar"));
        assert!(valid_obs_name("gcc-c++"));
        assert!(!valid_obs_name(""));
        assert!(!valid_obs_name("foo/bar"));
        assert!(!valid_obs_name("foo?x=1"));
        assert!(!valid_obs_name("foo' or '1'='1"));
    }

    // --- devel_project ---

    #[test]
    fn devel_project_extracted() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo",
            "<directory name=\"foo\"><devel project=\"devel:languages:python\" package=\"foo\"/></directory>",
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.devel_project("foo").unwrap(),
            Some("devel:languages:python".to_string())
        );
        // The public read carries no credentials.
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo".to_string(),
                false
            )]
        );
    }

    #[test]
    fn devel_project_missing_is_none() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo",
            "<directory name=\"foo\"></directory>",
        );
        let client = ObsClient::new(&http);
        assert_eq!(client.devel_project("foo").unwrap(), None);
    }

    #[test]
    fn devel_project_missing_package_is_none() {
        // A 404 means the package is not there, so it has no devel
        // project — that is None, not a failed lookup. Any other
        // failure stays an error (see the transport-failure test).
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo",
            ErrorKind::NotFound,
        );
        let client = ObsClient::new(&http);
        assert_eq!(client.devel_project("foo").unwrap(), None);
    }

    #[test]
    fn devel_project_attribute_order_does_not_matter() {
        // The package attribute first: the project must still be found.
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo",
            "<directory name=\"foo\"><devel package=\"foo\" project=\"devel:languages:python\"/></directory>",
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.devel_project("foo").unwrap(),
            Some("devel:languages:python".to_string())
        );
    }

    #[test]
    fn devel_project_rejects_invalid_package_name_without_http() {
        // An invalid name is rejected before any request: no canned
        // response exists, so reaching the HTTP client would error
        // differently (FakeHttp: "no canned response").
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let err = client.devel_project("foo/bar").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn devel_project_malformed_body_is_error_not_none() {
        // A truncated 200 body must not read as "no devel project".
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo",
            "<directory name=\"foo\"><devel project=\"devel:languages:python\"",
        );
        let client = ObsClient::new(&http);
        let err = client.devel_project("foo").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn devel_project_invalid_project_from_xml_is_error() {
        // The network-sourced project is validated at the source: a
        // name that cannot go into a URL is an error, not a project.
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo",
            "<directory name=\"foo\"><devel project=\"devel/foo\" package=\"foo\"/></directory>",
        );
        let client = ObsClient::new(&http);
        let err = client.devel_project("foo").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn devel_project_transport_failure_is_error_not_none() {
        // A failed lookup must never read as "no devel project": no
        // canned response is a transport error, not None.
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let err = client.devel_project("foo").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NetworkError);
    }

    // --- devel_version ---

    #[test]
    fn devel_version_reads_the_spec() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo",
                "<directory name=\"foo\"><devel project=\"devel:languages:python\" package=\"foo\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/devel:languages:python/foo/foo.spec",
                "Name: foo\nVersion: 2.0\nRelease: 1\n",
            );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.devel_version("foo").unwrap(),
            Some("2.0".to_string())
        );
    }

    #[test]
    fn devel_version_none_when_no_devel_project() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo",
            "<directory name=\"foo\"></directory>",
        );
        let client = ObsClient::new(&http);
        assert_eq!(client.devel_version("foo").unwrap(), None);
    }

    #[test]
    fn devel_version_rejects_invalid_project_from_xml() {
        // A project name from network XML is validated too: a value
        // that cannot be a project name errors rather than being
        // interpolated into a URL.
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo",
            "<directory name=\"foo\"><devel project=\"weird/project\" package=\"foo\"/></directory>",
        );
        let client = ObsClient::new(&http);
        let err = client.devel_version("foo").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    // --- has_open_sr ---

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
        // The search match cannot filter by action type (the engine
        // 400s on it), so a delete request targeting the package
        // must not read as an open submit request.
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
    fn search_url_rejects_invalid_package_name() {
        // A quote in the name would break out of the XPath string literal.
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client.search_url("foo' or '1'='1").is_err());
        assert!(client.search_url("").is_err());
        assert!(client.search_url("foo bar").is_err());
        // Valid OBS names pass.
        assert!(client.search_url("python3-requests").is_ok());
        assert!(client.search_url("foo:bar").is_ok());
    }

    #[test]
    fn parse_search_matches_counts() {
        assert_eq!(
            parse_search_matches("<collection matches=\"3\"></collection>"),
            Some(3)
        );
        assert_eq!(
            parse_search_matches("<collection matches=\"0\"></collection>"),
            Some(0)
        );
        assert_eq!(parse_search_matches("<collection></collection>"), None);
    }

    // --- create_request ---

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
    fn parse_request_id_extracts_numeric_id() {
        assert_eq!(
            parse_request_id("<request id=\"1400001\" creator=\"scarabeus_iv\"></request>"),
            Some("1400001".to_string())
        );
        assert_eq!(parse_request_id("<request></request>"), None);
        assert_eq!(parse_request_id("<request id=\"abc\"></request>"), None);
        assert_eq!(parse_request_id(""), None);
        // The attribute order does not matter, and either quote style
        // works: the id is read from inside the request tag.
        assert_eq!(
            parse_request_id("<request creator=\"scarabeus_iv\" id=\"1400001\"></request>"),
            Some("1400001".to_string())
        );
        assert_eq!(
            parse_request_id("<request id='1400001'></request>"),
            Some("1400001".to_string())
        );
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

    // --- SrPayload ---

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
        assert!(check_well_formed(&payload().to_xml()).is_ok());
    }

    #[test]
    fn check_well_formed_rejects_mismatched_tags() {
        assert!(check_well_formed("<request><action></request>").is_err());
        assert!(check_well_formed("<request><action>").is_err());
        assert!(check_well_formed("<request></request><request></request>").is_err());
        assert!(check_well_formed("").is_err());
    }

    #[test]
    fn check_well_formed_rejects_missing_elements() {
        // Well-formed XML but not an SR: no action/source/target.
        assert!(check_well_formed("<request><description>x</description></request>").is_err());
        assert!(check_well_formed(
            "<request><action><source project=\"a\" package=\"b\" /></action></request>"
        )
        .is_err());
        // Well-formed SR XML, but without the options block: the
        // cleanup attribute is part of the payload now.
        assert!(check_well_formed(
            "<request><action><source project=\"a\" package=\"b\" /><target project=\"c\" package=\"b\" /></action><description>x</description></request>"
        )
        .is_err());
    }

    // --- CVE rule ---

    #[test]
    fn cve_id_parse_validates_shape() {
        assert_eq!(
            CveId::parse("CVE-2026-12345").unwrap().as_str(),
            "CVE-2026-12345"
        );
        // Lowercase input normalizes to uppercase.
        assert_eq!(
            CveId::parse("cve-2026-12345").unwrap().as_str(),
            "CVE-2026-12345"
        );
        assert!(CveId::parse("CVE-1998-12345").is_err());
        assert!(CveId::parse("CVE-2026-123").is_err());
        assert!(CveId::parse("CVE-2026-abc").is_err());
        assert!(CveId::parse("2026-12345").is_err());
        assert!(CveId::parse("CVE-2026").is_err());
    }

    #[test]
    fn changelog_names_cve_present_absent_and_boundaries() {
        let cve = CveId::parse("CVE-2026-12345").unwrap();
        let changelog = "- CVE-2026-12345: fix the overflow (bsc#1234567)\n";
        assert!(changelog_names_cve(changelog, &cve));
        assert!(!changelog_names_cve("", &cve));
        assert!(!changelog_names_cve("- Update to version 2.0.\n", &cve));
        // Case-insensitive.
        assert!(changelog_names_cve("- fixed cve-2026-12345\n", &cve));
        // A longer number that merely starts with this id does not match.
        assert!(!changelog_names_cve(
            "- Fix CVE-2026-123456.\n",
            &CveId::parse("CVE-2026-12345").unwrap()
        ));
        // A longer token that merely contains the id does not match.
        assert!(!changelog_names_cve("- Fix XCVE-2026-12345.\n", &cve));
        // A trailing letter means a longer token, not this CVE.
        assert!(!changelog_names_cve("- Fix CVE-2026-12345a.\n", &cve));
        assert!(changelog_names_cve("- Fix (CVE-2026-12345).\n", &cve));
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

    // --- SrFiling / DryRun ---

    #[test]
    fn dry_run_is_the_default() {
        assert!(DryRun::default().enabled());
        assert!(!DryRun::default().live());
        assert!(DryRun::new(false).live());
    }

    #[test]
    fn dry_run_allow_blocks_and_records() {
        let dry = DryRun::new(true);
        assert!(!dry.allow("write the queue file"));
        assert_eq!(dry.plan(), vec!["[dry-run] would: write the queue file"]);
        let live = DryRun::new(false);
        assert!(live.allow("write the queue file"));
        assert!(live.plan().is_empty());
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
    fn parse_devel_project_handles_missing_and_malformed() {
        assert_eq!(
            parse_devel_project("<directory></directory>").unwrap(),
            None
        );
        assert_eq!(parse_devel_project("<directory/>").unwrap(), None);
        // A `<development>` tag is not a devel element.
        assert_eq!(
            parse_devel_project("<directory><development project=\"x\"/></directory>").unwrap(),
            None
        );
        // Not a directory document: an error, never None.
        assert!(parse_devel_project("").is_err());
        assert!(parse_devel_project("<collection></collection>").is_err());
        // A truncated body is malformed, not "no devel project".
        assert!(
            parse_devel_project("<directory name=\"foo\"><devel project=\"devel:foo\"").is_err()
        );
        assert!(parse_devel_project("<directory name=\"foo\">").is_err());
        // A devel element whose opening tag is complete still yields
        // its project, even without a closing tag: the parser reads
        // the tag, not the element balance.
        assert_eq!(
            parse_devel_project("<directory><devel project=\"devel:foo\"></directory>").unwrap(),
            Some("devel:foo".to_string())
        );
        // Trailing junk after the closing tag is malformed.
        assert!(parse_devel_project("<directory></directory>junk").is_err());
    }

    #[test]
    fn parse_spec_version_variants() {
        assert_eq!(
            parse_spec_version("Name: foo\nVersion: 1.2.3\nRelease: 1\n"),
            Some("1.2.3".to_string())
        );
        assert_eq!(parse_spec_version("Name: foo\nRelease: 1\n"), None);
        assert_eq!(
            parse_spec_version("Version: 2.0~rc1\n"),
            Some("2.0~rc1".to_string())
        );
    }

    // --- validate_request_id / parse_status_ok / parse_request_ids ---

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
    fn parse_request_ids_requires_a_matches_count() {
        // A collection without its matches count is unexpected, not
        // empty: without the count a malformed response would read as
        // "no requests", which is exactly the answer that lets a
        // delete or a refile proceed.
        assert_eq!(parse_request_ids("<collection></collection>"), None);
        assert_eq!(parse_request_ids(""), None);
        assert_eq!(
            parse_request_ids("<collection matches=\"0\"></collection>"),
            Some(vec![])
        );
        // The count says there are requests but none parsed: the
        // server changed shape, and an empty list would let the
        // filing dedup pass and the delete pre-check proceed.
        assert_eq!(
            parse_request_ids("<collection matches=\"1\"></collection>"),
            None
        );
        assert_eq!(
            parse_request_ids(
                "<collection matches=\"1\"><request creator=\"scarabeus_iv\" /></collection>"
            ),
            None
        );
    }

    #[test]
    fn parse_status_ok_reads_the_status_code() {
        assert!(parse_status_ok("<status code=\"ok\" />"));
        assert!(parse_status_ok("<status code=\"ok\"/>"));
        assert!(!parse_status_ok(
            "<status code=\"not_found\"><summary>no such request</summary></status>"
        ));
        assert!(!parse_status_ok("<status code=\"deleted\" />"));
        assert!(!parse_status_ok(""));
        assert!(!parse_status_ok("<request id=\"1398492\" />"));
        // A `code="ok"` outside the status element does not count.
        assert!(!parse_status_ok("<other code=\"ok\" />"));
        // A `code="ok"` inside a longer attribute name is not the
        // status code.
        assert!(!parse_status_ok("<status mycode=\"ok\" />"));
        assert!(parse_status_ok("<status code='ok' />"));
    }

    #[test]
    fn parse_submit_request_ids_keeps_only_submit_requests() {
        // The search response carries full request elements: a
        // submit action counts, a delete action does not, and a
        // self-closing request tag carries no action at all.
        assert_eq!(
            parse_submit_request_ids(
                "<collection matches=\"3\"><request id=\"1\"><action type=\"submit\"><source project=\"a\" package=\"foo\" /><target project=\"b\" package=\"foo\" /></action><state name=\"new\" /></request><request id=\"2\"><action type=\"delete\"><target project=\"b\" package=\"foo\" /></action><state name=\"new\" /></request><request id=\"3\" /></collection>"
            ),
            Some(vec!["1".to_string()])
        );
        // All non-submit still parses: the answer is an empty list,
        // not an error — that is "no open submit request".
        assert_eq!(
            parse_submit_request_ids(
                "<collection matches=\"1\"><request id=\"2\"><action type=\"delete\"><target project=\"b\" package=\"foo\" /></action><state name=\"new\" /></request></collection>"
            ),
            Some(vec![])
        );
        // Not a collection, or no count, or a count with nothing
        // parsed, is unexpected, never empty.
        assert_eq!(parse_submit_request_ids(""), None);
        assert_eq!(parse_submit_request_ids("<collection></collection>"), None);
        assert_eq!(
            parse_submit_request_ids("<collection matches=\"1\"></collection>"),
            None
        );
        // The action type is read from the tag, not the raw text: a
        // description mentioning submit does not count.
        assert_eq!(
            parse_submit_request_ids(
                "<collection matches=\"1\"><request id=\"2\"><action type=\"delete\" /><description>submit this please</description></request></collection>"
            ),
            Some(vec![])
        );
        // Either quote style works, and the type may sit anywhere in
        // the action tag.
        assert_eq!(
            parse_submit_request_ids(
                "<collection matches=\"1\"><request id=\"2\"><action foo=\"x\" type='submit' /></request></collection>"
            ),
            Some(vec!["2".to_string()])
        );
    }

    #[test]
    fn parse_request_ids_collects_all_ids() {
        assert_eq!(
            parse_request_ids(
                "<collection matches=\"2\"><request id=\"1398492\" /><request id=\"1398493\" /></collection>"
            ),
            Some(vec!["1398492".to_string(), "1398493".to_string()])
        );
        assert_eq!(
            parse_request_ids("<collection matches=\"0\"></collection>"),
            Some(vec![])
        );
        // Not a collection at all: unexpected, not empty.
        assert_eq!(parse_request_ids("<request id=\"1398492\" />"), None);
        assert_eq!(parse_request_ids(""), None);
        // A malformed id hides a request: unexpected, not partial.
        assert_eq!(
            parse_request_ids("<collection matches=\"1\"><request id=\"abc\" /></collection>"),
            None
        );
        // The id is read from inside the request tag: the attribute
        // order does not matter, and either quote style works.
        assert_eq!(
            parse_request_ids(
                "<collection matches=\"2\"><request creator=\"scarabeus_iv\" id=\"1398492\" /><request id='1398493' /></collection>"
            ),
            Some(vec!["1398492".to_string(), "1398493".to_string()])
        );
    }

    // --- supersede_request ---

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

    // --- create_project / create_package ---

    #[test]
    fn create_project_puts_meta_and_returns_ok() {
        let http = FakeHttp::new().with_put(
            "https://api.opensuse.org/source/home:alice/_meta",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .create_project(
                "home:alice",
                "Alice's branch",
                "Temporal test branch",
                &DryRun::new(false)
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].0,
            "https://api.opensuse.org/source/home:alice/_meta"
        );
        assert_eq!(
            puts[0].1,
            "<project name=\"home:alice\"><title>Alice&apos;s branch</title><description>Temporal test branch</description></project>"
        );
        // The creation PUT carried the credentials.
        assert!(puts[0].2);
    }

    #[test]
    fn create_project_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_put(
            "https://api.opensuse.org/source/home:alice/_meta",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .create_project(
                "home:alice",
                "Alice's branch",
                "Temporal test branch",
                &DryRun::new(false)
            )
            .is_err());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn create_project_rejects_invalid_name_without_http() {
        // An invalid name is rejected before any request: no canned
        // response exists, so reaching the HTTP client would error
        // differently (FakeHttp: "no canned response").
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .create_project(
                "home/alice",
                "Alice's branch",
                "Temporal test branch",
                &DryRun::new(false)
            )
            .is_err());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn create_project_non_ok_body_is_error_not_success() {
        // The server answered 2xx but not ok: a failed creation must
        // never read as success.
        let http = FakeHttp::new().with_put(
            "https://api.opensuse.org/source/home:alice/_meta",
            "<status code=\"not_found\"><summary>no such project</summary></status>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .create_project(
                "home:alice",
                "Alice's branch",
                "Temporal test branch",
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn create_project_escapes_meta_text() {
        // Title and description go into the `_meta` XML, so markup in
        // them must be escaped, not interpolated raw.
        let http = FakeHttp::new().with_put(
            "https://api.opensuse.org/source/home:alice/_meta",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .create_project(
                "home:alice",
                "A <branch> & friends",
                "x\"y",
                &DryRun::new(false)
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert!(
            puts[0]
                .1
                .contains("<title>A &lt;branch&gt; &amp; friends</title>"),
            "title must be escaped in the _meta XML; got {}",
            puts[0].1
        );
        assert!(
            puts[0].1.contains("<description>x&quot;y</description>"),
            "description must be escaped in the _meta XML; got {}",
            puts[0].1
        );
    }

    #[test]
    fn create_project_dry_run_records_plan_and_creates_nothing() {
        // A dry-run caller invoking create directly must not mutate
        // the server: the creation is recorded in the plan and no PUT
        // runs (no canned response exists, so a PUT would fail). No
        // credentials are needed either: the guard runs before the
        // credentials check, like delete_package — so the client here
        // deliberately has none.
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let dry = DryRun::default();
        assert!(client
            .create_project("home:alice", "Alice's branch", "Temporal test branch", &dry)
            .is_ok());
        assert!(http.put_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec!["[dry-run] would: create project home:alice"]
        );
    }

    #[test]
    fn create_package_puts_meta_and_returns_ok() {
        let http = FakeHttp::new().with_put(
            "https://api.opensuse.org/source/home:alice/foo/_meta",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .create_package(
                "home:alice",
                "foo",
                "Foo",
                "Temporal test package",
                &DryRun::new(false)
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].0,
            "https://api.opensuse.org/source/home:alice/foo/_meta"
        );
        assert_eq!(
            puts[0].1,
            "<package name=\"foo\" project=\"home:alice\"><title>Foo</title><description>Temporal test package</description></package>"
        );
        assert!(puts[0].2);
    }

    #[test]
    fn create_package_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_put(
            "https://api.opensuse.org/source/home:alice/foo/_meta",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .create_package(
                "home:alice",
                "foo",
                "Foo",
                "Temporal test package",
                &DryRun::new(false)
            )
            .is_err());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn create_package_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .create_package(
                "home/alice",
                "foo",
                "Foo",
                "Temporal test package",
                &DryRun::new(false)
            )
            .is_err());
        assert!(client
            .create_package(
                "home:alice",
                "foo/bar",
                "Foo",
                "Temporal test package",
                &DryRun::new(false)
            )
            .is_err());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn create_package_dry_run_records_plan_and_creates_nothing() {
        // As create_project: the creation is recorded in the plan, no
        // PUT runs, and no credentials are needed.
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let dry = DryRun::default();
        assert!(client
            .create_package("home:alice", "foo", "Foo", "Temporal test package", &dry)
            .is_ok());
        assert!(http.put_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec!["[dry-run] would: create package home:alice/foo"]
        );
    }

    // --- upload_file ---

    #[test]
    fn upload_file_puts_file_and_returns_revision() {
        let http = FakeHttp::new().with_put(
            "https://api.opensuse.org/source/home:alice/foo/foo.spec",
            "<revision rev=\"1\" vrev=\"1\" srcmd5=\"abc\" version=\"1\" time=\"123\" user=\"Admin\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .upload_file(
                    "home:alice",
                    "foo",
                    "foo.spec",
                    "Name: foo\n",
                    &DryRun::new(false)
                )
                .unwrap(),
            Some(1)
        );
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].0,
            "https://api.opensuse.org/source/home:alice/foo/foo.spec"
        );
        assert_eq!(puts[0].1, "Name: foo\n");
        // The upload PUT carried the credentials.
        assert!(puts[0].2);
    }

    #[test]
    fn upload_file_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_put(
            "https://api.opensuse.org/source/home:alice/foo/foo.spec",
            "<revision rev=\"1\" />",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .upload_file(
                "home:alice",
                "foo",
                "foo.spec",
                "Name: foo\n",
                &DryRun::new(false)
            )
            .is_err());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn upload_file_rejects_invalid_names_and_filenames_without_http() {
        // An invalid name or filename is rejected before any request:
        // no canned responses exist, so reaching the HTTP client would
        // error differently (FakeHttp: "no canned response").
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let live = DryRun::new(false);
        assert!(client
            .upload_file("home/alice", "foo", "foo.spec", "x", &live)
            .is_err());
        assert!(client
            .upload_file("home:alice", "foo/bar", "foo.spec", "x", &live)
            .is_err());
        // The server's filename rule (^[^./][^/]+$): a slash anywhere,
        // a leading dot, and a one-character name are all invalid.
        for bad in ["foo/bar.spec", ".hidden", "x", "", "_meta", "_attribute"] {
            assert!(
                client
                    .upload_file("home:alice", "foo", bad, "x", &live)
                    .is_err(),
                "filename {bad:?} must be rejected"
            );
        }
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn upload_file_non_revision_body_is_error_not_success() {
        // The upload response is a revision document, not a status:
        // a `<status code="ok"/>` body (the meta-PUT success shape)
        // must never read as a successful upload.
        let http = FakeHttp::new().with_put(
            "https://api.opensuse.org/source/home:alice/foo/foo.spec",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .upload_file(
                "home:alice",
                "foo",
                "foo.spec",
                "Name: foo\n",
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn upload_file_dry_run_records_plan_and_uploads_nothing() {
        // As create_project: the upload is recorded in the plan, no
        // PUT runs, and no credentials are needed.
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let dry = DryRun::default();
        assert_eq!(
            client
                .upload_file("home:alice", "foo", "foo.spec", "Name: foo\n", &dry)
                .unwrap(),
            None
        );
        assert!(http.put_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec!["[dry-run] would: upload file home:alice/foo/foo.spec"]
        );
    }

    #[test]
    fn parse_revision_reads_the_rev_attribute() {
        // The backend's revision document, full shape.
        assert_eq!(
            parse_revision("<revision rev=\"3\" vrev=\"3\" srcmd5=\"abc\" version=\"1\" time=\"123\" user=\"Admin\" />"),
            Some(3)
        );
        // `vrev` comes first: the `rev=` inside `vrev=` is not the
        // revision (it is preceded by `v`, not whitespace).
        assert_eq!(parse_revision("<revision vrev=\"3\" rev=\"7\" />"), Some(7));
        // Not a revision document: a status-ok body (the meta-PUT
        // success shape), a revision list, a zero revision, and a
        // `<revisions>` tag all read as none.
        assert_eq!(parse_revision("<status code=\"ok\" />"), None);
        assert_eq!(
            parse_revision("<revisionlist><revision rev=\"1\" /></revisionlist>"),
            None
        );
        assert_eq!(parse_revision("<revision rev=\"0\" />"), None);
        assert_eq!(parse_revision("<revisions rev=\"1\" />"), None);
        assert_eq!(parse_revision(""), None);
    }

    // --- open_requests_for_package / package_requests_search_url ---

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

    // --- delete_package ---

    #[test]
    fn delete_package_deletes_when_no_open_requests() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs
            .package_requests_search_url("home:scarabeus_iv", "foo")
            .unwrap();
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_delete(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo",
                "<status code=\"ok\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(obs
            .delete_package("home:scarabeus_iv", "foo", false, &DryRun::new(false))
            .is_ok());
        // The pre-check search ran first, then the DELETE.
        assert_eq!(http.get_calls().len(), 1);
        let deleted = http.delete_calls();
        assert_eq!(deleted.len(), 1);
        assert_eq!(
            deleted[0].0,
            "https://api.opensuse.org/source/home:scarabeus_iv/foo"
        );
        assert!(deleted[0].1);
    }

    #[test]
    fn delete_package_refuses_when_open_requests_exist() {
        // A live request involves the package: the delete is refused
        // and no DELETE is made, so the request is not silently
        // revoked/declined by the API.
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs
            .package_requests_search_url("home:scarabeus_iv", "foo")
            .unwrap();
        let http = FakeHttp::new()
            .with_get(
                &search_url,
                "<collection matches=\"1\"><request id=\"1398492\" /></collection>",
            )
            .with_delete(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo",
                "<status code=\"ok\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = obs
            .delete_package("home:scarabeus_iv", "foo", false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        // The error names the blocking request.
        assert!(err.context.contains("1398492"));
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn delete_package_force_bypasses_precheck_and_forwards_force() {
        // Force skips the search entirely (no canned GET exists, so a
        // search would fail) and forwards force=1 to the API.
        let http = FakeHttp::new().with_delete(
            "https://api.opensuse.org/source/home:scarabeus_iv/foo?force=1",
            "<status code=\"ok\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(obs
            .delete_package("home:scarabeus_iv", "foo", true, &DryRun::new(false))
            .is_ok());
        assert!(http.get_calls().is_empty());
        assert_eq!(http.delete_calls().len(), 1);
        assert_eq!(
            http.delete_calls()[0].0,
            "https://api.opensuse.org/source/home:scarabeus_iv/foo?force=1"
        );
    }

    #[test]
    fn delete_package_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client
            .delete_package("home:scarabeus_iv", "foo", false, &DryRun::new(false))
            .is_err());
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn delete_package_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .delete_package("home:scarabeus_iv", "foo/bar", false, &DryRun::new(false))
            .is_err());
        assert!(client
            .delete_package("", "foo", false, &DryRun::new(false))
            .is_err());
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn delete_package_non_ok_body_is_error_not_success() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs
            .package_requests_search_url("home:scarabeus_iv", "foo")
            .unwrap();
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_delete(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo",
                "<status code=\"not_found\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = obs
            .delete_package("home:scarabeus_iv", "foo", false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn delete_package_dry_run_records_plan_and_deletes_nothing() {
        // A dry-run caller invoking delete directly must not mutate
        // the server: the delete is recorded in the plan, and neither
        // the pre-check search nor the DELETE runs (no canned
        // responses exist, so either would fail).
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::default();
        assert!(obs
            .delete_package("home:scarabeus_iv", "foo", false, &dry)
            .is_ok());
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec!["[dry-run] would: delete package home:scarabeus_iv/foo"]
        );
    }

    // --- SrFiling::refile ---

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

    // --- split_slash_segments / parse_project_package ---

    #[test]
    fn slash_split_drops_empty_segments() {
        // No slash, leading/trailing slash, middle slash, `//` and
        // `/////////` collapse (empty segments dropped).
        assert_eq!(split_slash_segments("foo"), vec!["foo"]);
        assert_eq!(split_slash_segments("a/b"), vec!["a", "b"]);
        assert_eq!(split_slash_segments("/a/b"), vec!["a", "b"]);
        assert_eq!(split_slash_segments("a/b/"), vec!["a", "b"]);
        assert_eq!(split_slash_segments("a//b"), vec!["a", "b"]);
        assert!(split_slash_segments("//").is_empty());
        assert!(split_slash_segments("/////////").is_empty());
        assert!(split_slash_segments("").is_empty());
    }

    #[test]
    fn project_package_split_needs_exactly_two_segments() {
        assert_eq!(
            parse_project_package("devel:languages:python/foo").unwrap(),
            ("devel:languages:python".to_string(), "foo".to_string())
        );
        assert_eq!(
            parse_project_package("/a/b/").unwrap(),
            ("a".to_string(), "b".to_string())
        );
        assert!(parse_project_package("foo").is_err());
        assert!(parse_project_package("").is_err());
        assert!(parse_project_package("a/b/c").is_err());
        assert!(parse_project_package("a//b/c").is_err());
    }

    // --- RevisionSpec / RevisionRange ---

    #[test]
    fn revision_spec_parses_numbers_and_md5s() {
        assert_eq!(RevisionSpec::parse("1").unwrap(), RevisionSpec::Rev(1));
        assert_eq!(RevisionSpec::parse("123").unwrap(), RevisionSpec::Rev(123));
        let md5 = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            RevisionSpec::parse(md5).unwrap(),
            RevisionSpec::Md5(md5.to_string())
        );
        // An uppercase hex md5 normalizes to lowercase.
        assert_eq!(
            RevisionSpec::parse("0123456789ABCDEF0123456789ABCDEF").unwrap(),
            RevisionSpec::Md5(md5.to_string())
        );
        assert!(RevisionSpec::parse("").is_err());
        assert!(RevisionSpec::parse("0").is_err());
        assert!(RevisionSpec::parse("abc").is_err());
        assert!(RevisionSpec::parse("0123456789abcdef").is_err());
        assert!(RevisionSpec::parse("1:2").is_err());
    }

    #[test]
    fn revision_range_parses_the_option_form() {
        // `""` / `":"` → (None, None).
        assert_eq!(
            RevisionRange::parse("").unwrap(),
            RevisionRange {
                from: None,
                to: None
            }
        );
        assert_eq!(
            RevisionRange::parse(":").unwrap(),
            RevisionRange {
                from: None,
                to: None
            }
        );
        // `"1"` → ("1", None); `"1:2"` → ("1", "2").
        assert_eq!(
            RevisionRange::parse("1").unwrap(),
            RevisionRange {
                from: Some(RevisionSpec::Rev(1)),
                to: None
            }
        );
        assert_eq!(
            RevisionRange::parse("1:2").unwrap(),
            RevisionRange {
                from: Some(RevisionSpec::Rev(1)),
                to: Some(RevisionSpec::Rev(2))
            }
        );
        // 32-hex md5 → (hash, None); hash1:hash2.
        let hash1 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hash2 = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        assert_eq!(
            RevisionRange::parse(hash1).unwrap(),
            RevisionRange {
                from: Some(RevisionSpec::Md5(hash1.to_string())),
                to: None
            }
        );
        assert_eq!(
            RevisionRange::parse(&format!("{hash1}:{hash2}")).unwrap(),
            RevisionRange {
                from: Some(RevisionSpec::Md5(hash1.to_string())),
                to: Some(RevisionSpec::Md5(hash2.to_string()))
            }
        );
    }

    // --- xml_unescape ---

    #[test]
    fn xml_unescape_reverses_escape() {
        assert_eq!(xml_unescape("plain"), "plain");
        assert_eq!(xml_unescape("a &amp; b"), "a & b");
        assert_eq!(xml_unescape("&lt;tag&gt;"), "<tag>");
        assert_eq!(xml_unescape("&quot;q&quot;"), "\"q\"");
        assert_eq!(xml_unescape("&apos;a&apos;"), "'a'");
        assert_eq!(xml_unescape("&#65;"), "A");
        assert_eq!(xml_unescape("&#x41;"), "A");
        // Unknown entities and a bare `&` pass through verbatim.
        assert_eq!(xml_unescape("&nbsp;"), "&nbsp;");
        assert_eq!(xml_unescape("a & b"), "a & b");
        assert_eq!(xml_unescape("a &"), "a &");
    }

    // --- tag_attr ---

    #[test]
    fn tag_attr_reads_either_quote_style_in_any_order() {
        assert_eq!(
            tag_attr("entry name=\"foo\" md5=\"abc\"", "name").as_deref(),
            Some("foo")
        );
        assert_eq!(
            tag_attr("entry md5='abc' name='foo'", "name").as_deref(),
            Some("foo")
        );
        // The name must stand on its own: `myname` does not match.
        assert_eq!(tag_attr("entry myname=\"foo\"", "name"), None);
        assert_eq!(tag_attr("entry name=\"\"", "name"), None);
        assert_eq!(tag_attr("entry", "name"), None);
    }

    // --- parse_directory ---

    const DIRECTORY_FIXTURE: &str = "<directory name=\"foo\" rev=\"3\" vrev=\"1\" srcmd5=\"0123456789abcdef0123456789abcdef\">\
        <entry name=\"foo.spec\" md5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" size=\"1234\" mtime=\"1700000000\"/>\
        <entry name=\"foo.changes\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"567\" mtime=\"1700000001\"/>\
        <entry name=\"_link\"/>\
        <linkinfo project=\"home:Admin\" package=\"foo\"/>\
        <devel project=\"devel:languages:python\" package=\"foo\"/>\
        </directory>";

    #[test]
    fn directory_listing_parses_entries_and_linkinfo() {
        let listing = parse_directory(DIRECTORY_FIXTURE).unwrap();
        assert_eq!(
            listing.entries,
            vec![
                SourceFile {
                    name: "foo.spec".to_string(),
                    size: Some(1234),
                    mtime: Some(1_700_000_000),
                    md5: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
                },
                SourceFile {
                    name: "foo.changes".to_string(),
                    size: Some(567),
                    mtime: Some(1_700_000_001),
                    md5: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
                },
                SourceFile {
                    name: "_link".to_string(),
                    size: None,
                    mtime: None,
                    md5: None,
                },
            ]
        );
        assert_eq!(
            listing.linkinfo,
            Some(("home:Admin".to_string(), "foo".to_string()))
        );
    }

    #[test]
    fn directory_listing_rejects_malformed_bodies() {
        // Not a directory document at all.
        assert!(parse_directory("").is_err());
        assert!(parse_directory("<collection matches=\"0\"></collection>").is_err());
        assert!(parse_directory("<directorylisting></directorylisting>").is_err());
        // Unterminated, or a missing close tag.
        assert!(parse_directory("<directory name=\"foo\">").is_err());
        assert!(parse_directory("<directory name=\"foo\"><entry name=\"x\"/>").is_err());
        // Trailing garbage after the closing tag.
        assert!(parse_directory("<directory></directory> junk").is_err());
        // An entry without a name, or with a non-numeric size/mtime.
        assert!(parse_directory("<directory><entry md5=\"abc\"/></directory>").is_err());
        assert!(
            parse_directory("<directory><entry name=\"x\" size=\"big\"/></directory>").is_err()
        );
        assert!(
            parse_directory("<directory><entry name=\"x\" mtime=\"soon\"/></directory>").is_err()
        );
        // An unterminated entry tag.
        assert!(parse_directory("<directory><entry name=\"x\"</directory>").is_err());
    }

    #[test]
    fn directory_listing_self_closing_is_empty() {
        let listing = parse_directory("<directory name=\"foo\"/>").unwrap();
        assert!(listing.entries.is_empty());
        assert_eq!(listing.linkinfo, None);
    }

    #[test]
    fn directory_listing_without_linkinfo_is_none() {
        let listing = parse_directory("<directory><entry name=\"foo.spec\"/></directory>").unwrap();
        assert_eq!(listing.linkinfo, None);
    }

    // --- cat ---

    #[test]
    fn cat_reads_the_file_without_credentials() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/foo.spec",
            "Name: foo\nVersion: 1.0\n",
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .cat("openSUSE:Factory", "foo", "foo.spec", None)
                .unwrap(),
            "Name: foo\nVersion: 1.0\n"
        );
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo/foo.spec".to_string(),
                false
            )]
        );
    }

    #[test]
    fn cat_at_revision_passes_rev() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/foo.spec?rev=2",
            "Name: foo\nVersion: 1.0\n",
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .cat("openSUSE:Factory", "foo", "foo.spec", Some(2))
                .unwrap(),
            "Name: foo\nVersion: 1.0\n"
        );
    }

    #[test]
    fn cat_rejects_bad_inputs() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client
            .cat("openSUSE:Factory", "foo/bar", "foo.spec", None)
            .is_err());
        assert!(client
            .cat("openSUSE:Factory", "foo", ".hidden", None)
            .is_err());
        assert!(client.cat("openSUSE:Factory", "foo", "a/b", None).is_err());
        assert!(client
            .cat("openSUSE:Factory", "foo", "foo.spec", Some(0))
            .is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn cat_missing_file_is_an_error() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/nope.spec",
            ErrorKind::NotFound,
        );
        let client = ObsClient::new(&http);
        let err = client
            .cat("openSUSE:Factory", "foo", "nope.spec", None)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    // --- list_files / list_packages ---

    #[test]
    fn list_files_reads_the_listing_without_credentials() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo",
            DIRECTORY_FIXTURE,
        );
        let client = ObsClient::new(&http);
        let files = client.list_files("openSUSE:Factory", "foo").unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].name, "foo.spec");
        assert_eq!(files[2].name, "_link");
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo".to_string(),
                false
            )]
        );
    }

    #[test]
    fn list_packages_reads_entry_names() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory",
            "<directory><entry name=\"foo\"/><entry name=\"multibuild-pkg:flavor1\"/><entry name=\"multibuild-pkg:flavor2\"/></directory>",
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.list_packages("openSUSE:Factory").unwrap(),
            vec![
                "foo".to_string(),
                "multibuild-pkg:flavor1".to_string(),
                "multibuild-pkg:flavor2".to_string(),
            ]
        );
    }

    #[test]
    fn list_calls_reject_bad_names() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client.list_files("openSUSE:Factory", "foo/bar").is_err());
        assert!(client.list_packages("openSUSE:Factory/x").is_err());
        assert!(http.get_calls().is_empty());
    }

    // --- unified_diff ---

    #[test]
    fn unified_diff_of_identical_contents_is_empty() {
        assert_eq!(unified_diff("a/x", "one\ntwo\n", "b/x", "one\ntwo\n"), "");
        assert_eq!(unified_diff("a/x", "", "b/x", ""), "");
    }

    #[test]
    fn unified_diff_shows_a_single_change_with_context() {
        let diff = unified_diff(
            "a/x",
            "one\ntwo\nthree\nfour\nfive\n",
            "b/x",
            "one\nTWO\nthree\nfour\nfive\n",
        );
        assert_eq!(
            diff,
            "--- a/x\n+++ b/x\n@@ -1,5 +1,5 @@\n one\n-two\n+TWO\n three\n four\n five\n"
        );
    }

    #[test]
    fn unified_diff_of_a_new_file_is_all_additions() {
        let diff = unified_diff("/dev/null", "", "b/new.txt", "hello\nworld\n");
        assert_eq!(
            diff,
            "--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+hello\n+world\n"
        );
    }

    #[test]
    fn unified_diff_of_a_deleted_file_is_all_deletions() {
        let diff = unified_diff("a/old.txt", "hello\nworld\n", "/dev/null", "");
        assert_eq!(
            diff,
            "--- a/old.txt\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-hello\n-world\n"
        );
    }

    #[test]
    fn unified_diff_splits_distant_changes_into_hunks() {
        // Two changes ten lines apart: two hunks, three context lines
        // each, and no line from the middle in either hunk.
        let old: String = (1..=20).map(|n| format!("line {n}\n")).collect();
        let mut new = old.clone();
        new = new.replacen("line 2\n", "LINE 2\n", 1);
        new = new.replacen("line 18\n", "LINE 18\n", 1);
        let diff = unified_diff("a/x", &old, "b/x", &new);
        assert!(diff.contains("@@ -1,5 +1,5 @@\n"));
        assert!(diff.contains("@@ -15,6 +15,6 @@\n"));
        assert!(!diff.contains("line 10\n"));
        assert_eq!(diff.matches("@@").count(), 4);
    }

    // --- parse_history ---

    const HISTORY_FIXTURE: &str = "<revisionlist>\
        <revision rev=\"1\" vrev=\"1\">\
        <srcmd5>aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa</srcmd5>\
        <version>1.0</version><time>1700000000</time><user>Admin</user>\
        <comment>Initial import</comment>\
        </revision>\
        <revision rev=\"2\" vrev=\"1\">\
        <srcmd5>bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb</srcmd5>\
        <version>1.1</version><time>1700000001</time><user>Admin</user>\
        <comment>Update to 1.1</comment>\
        </revision>\
        <revision rev=\"3\" vrev=\"1\">\
        <srcmd5>cccccccccccccccccccccccccccccccc</srcmd5>\
        <version>1.2</version><time>1700000002</time><user>Admin</user>\
        <comment>Update to 1.2</comment><requestid>123</requestid>\
        </revision>\
        </revisionlist>";

    #[test]
    fn history_parses_in_server_order() {
        let entries = parse_history(HISTORY_FIXTURE).unwrap();
        assert_eq!(entries.len(), 3);
        // Server order is oldest first: the parser preserves it.
        assert_eq!(entries[0].rev, 1);
        assert_eq!(entries[2].rev, 3);
        assert_eq!(entries[2].vrev, 1);
        assert_eq!(
            entries[2].srcmd5.as_deref(),
            Some("cccccccccccccccccccccccccccccccc")
        );
        assert_eq!(entries[2].version.as_deref(), Some("1.2"));
        assert_eq!(entries[2].time, Some(1_700_000_002));
        assert_eq!(entries[2].user.as_deref(), Some("Admin"));
        assert_eq!(entries[2].comment.as_deref(), Some("Update to 1.2"));
        assert_eq!(entries[2].request_id.as_deref(), Some("123"));
        // The second entry carries no request id.
        assert_eq!(entries[1].rev, 2);
        assert_eq!(entries[1].request_id, None);
    }

    #[test]
    fn history_rejects_malformed_bodies() {
        assert!(parse_history("").is_err());
        assert!(parse_history("<collection matches=\"0\"></collection>").is_err());
        assert!(parse_history("<revisionlists></revisionlists>").is_err());
        assert!(parse_history("<revisionlist>").is_err());
        assert!(
            parse_history("<revisionlist><revision vrev=\"1\"></revision></revisionlist>").is_err()
        );
        assert!(parse_history(
            "<revisionlist><revision rev=\"x\" vrev=\"1\"></revision></revisionlist>"
        )
        .is_err());
        assert!(
            parse_history("<revisionlist><revision rev=\"1\" vrev=\"1\"><time>soon</time></revision></revisionlist>").is_err()
        );
        assert!(
            parse_history("<revisionlist><revision rev=\"1\" vrev=\"1\"></revisionlist>").is_err()
        );
        assert!(parse_history("<revisionlist/>").unwrap().is_empty());
    }

    // --- package_log ---

    #[test]
    fn package_log_returns_newest_first() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/openSUSE:Factory/foo/_history",
            HISTORY_FIXTURE,
        );
        let client = ObsClient::new(&http);
        let entries = client.package_log("openSUSE:Factory", "foo", None).unwrap();
        assert_eq!(
            entries.iter().map(|entry| entry.rev).collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
        // No credentials on this client: the read is unauthenticated.
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/source/openSUSE:Factory/foo/_history".to_string(),
                false
            )]
        );
    }

    #[test]
    fn package_log_range_filters_preserving_order() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/openSUSE:Factory/foo/_history",
            HISTORY_FIXTURE,
        );
        let client = ObsClient::new(&http);
        // `--revision=2` returns only r2.
        let entries = client
            .package_log(
                "openSUSE:Factory",
                "foo",
                Some(&RevisionRange::parse("2").unwrap()),
            )
            .unwrap();
        assert_eq!(
            entries.iter().map(|entry| entry.rev).collect::<Vec<_>>(),
            vec![2]
        );
        // `--revision=1:2` returns r2 then r1.
        let entries = client
            .package_log(
                "openSUSE:Factory",
                "foo",
                Some(&RevisionRange::parse("1:2").unwrap()),
            )
            .unwrap();
        assert_eq!(
            entries.iter().map(|entry| entry.rev).collect::<Vec<_>>(),
            vec![2, 1]
        );
        // An md5 range resolves through the history.
        let entries = client
            .package_log(
                "openSUSE:Factory",
                "foo",
                Some(
                    &RevisionRange::parse(
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    )
                    .unwrap(),
                ),
            )
            .unwrap();
        assert_eq!(
            entries.iter().map(|entry| entry.rev).collect::<Vec<_>>(),
            vec![2, 1]
        );
    }

    #[test]
    fn package_log_range_errors_are_fail_closed() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/openSUSE:Factory/foo/_history",
            HISTORY_FIXTURE,
        );
        let client = ObsClient::new(&http);
        // An unknown md5 is an error, never an empty list.
        assert!(client
            .package_log(
                "openSUSE:Factory",
                "foo",
                Some(&RevisionRange::parse("dddddddddddddddddddddddddddddddd").unwrap()),
            )
            .is_err());
        // A reversed range is an error.
        assert!(client
            .package_log(
                "openSUSE:Factory",
                "foo",
                Some(&RevisionRange::parse("2:1").unwrap()),
            )
            .is_err());
    }

    // --- revision_patch ---

    fn patch_client() -> FakeHttp {
        let rev1 = "<directory><entry name=\"foo.spec\" md5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"/><entry name=\"old.txt\" md5=\"oooooooooooooooooooooooooooooooo\"/></directory>";
        let rev2 = "<directory><entry name=\"foo.spec\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"/><entry name=\"new.txt\" md5=\"nnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnn\"/></directory>";
        FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo?rev=1",
                rev1,
            )
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo?rev=2",
                rev2,
            )
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo/foo.spec?rev=1",
                "Name: foo\nVersion: 1.0\n",
            )
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo/foo.spec?rev=2",
                "Name: foo\nVersion: 1.1\n",
            )
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo/new.txt?rev=2",
                "fresh\n",
            )
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo/old.txt?rev=1",
                "stale\n",
            )
    }

    #[test]
    fn revision_patch_rev1_is_empty() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let patch = client.revision_patch("openSUSE:Factory", "foo", 1).unwrap();
        assert_eq!(patch.rev, 1);
        assert!(patch.files.is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn revision_patch_classifies_added_modified_deleted() {
        let http = patch_client();
        let client = ObsClient::new(&http);
        let patch = client.revision_patch("openSUSE:Factory", "foo", 2).unwrap();
        assert_eq!(patch.rev, 2);
        assert_eq!(
            patch
                .files
                .iter()
                .map(|file| (file.filename.as_str(), file.status))
                .collect::<Vec<_>>(),
            vec![
                ("foo.spec", FilePatchStatus::Modified),
                ("new.txt", FilePatchStatus::Added),
                ("old.txt", FilePatchStatus::Deleted),
            ]
        );
        let modified = &patch.files[0];
        assert!(modified.diff.contains("--- a/foo.spec\n+++ b/foo.spec\n"));
        assert!(modified.diff.contains("-Version: 1.0\n+Version: 1.1\n"));
        // A new file renders with `/dev/null` on the old side.
        assert!(patch.files[1]
            .diff
            .contains("--- /dev/null\n+++ b/new.txt\n"));
        assert!(patch.files[1].diff.contains("+fresh\n"));
        assert!(patch.files[2]
            .diff
            .contains("--- a/old.txt\n+++ /dev/null\n"));
        assert!(patch.files[2].diff.contains("-stale\n"));
    }

    #[test]
    fn revision_patch_rejects_rev_zero() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client.revision_patch("openSUSE:Factory", "foo", 0).is_err());
        assert!(http.get_calls().is_empty());
    }

    // --- parse_project_meta / parse_package_meta ---

    const PROJECT_META_FIXTURE: &str = "<project name=\"openSUSE:Factory\">\
        <title>Factory</title>\
        <description>The Factory project</description>\
        <url>https://example.com</url>\
        <scmsync>https://src.opensuse.org/pool/factory</scmsync>\
        <devel project=\"openSUSE:Factory:Devel\"/>\
        <person userid=\"Admin\" role=\"maintainer\"/>\
        <group groupid=\"factory-maintainers\" role=\"maintainer\"/>\
        <repository name=\"standard\">\
        <arch>x86_64</arch><arch>aarch64</arch>\
        <path project=\"openSUSE:Factory:ARM\" repository=\"standard\"/>\
        <path project=\"openSUSE:Factory\" repository=\"snapshot\"/>\
        </repository>\
        <repository name=\"images\"><arch>x86_64</arch></repository>\
        </project>";

    const PACKAGE_META_FIXTURE: &str = "<package name=\"foo\" project=\"openSUSE:Factory\">\
        <title>Foo package</title>\
        <description>A test package &amp; more</description>\
        <url>https://example.com/foo</url>\
        <scmsync>https://src.opensuse.org/pool/foo</scmsync>\
        <releasename>foo</releasename>\
        <devel project=\"devel:languages:python\" package=\"foo\"/>\
        <person userid=\"Admin\" role=\"maintainer\"/>\
        <person userid=\"scarabeus_iv\" role=\"bugowner\"/>\
        <group groupid=\"factory-maintainers\" role=\"maintainer\"/>\
        </package>";

    #[test]
    fn project_meta_parses_everything() {
        let meta = parse_project_meta(PROJECT_META_FIXTURE).unwrap();
        assert_eq!(meta.name, "openSUSE:Factory");
        assert_eq!(meta.title.as_deref(), Some("Factory"));
        assert_eq!(meta.description.as_deref(), Some("The Factory project"));
        assert_eq!(meta.url.as_deref(), Some("https://example.com"));
        assert_eq!(
            meta.scmsync.as_deref(),
            Some("https://src.opensuse.org/pool/factory")
        );
        assert_eq!(meta.devel.as_deref(), Some("openSUSE:Factory:Devel"));
        assert_eq!(
            meta.persons,
            vec![NamedRole {
                name: "Admin".to_string(),
                role: "maintainer".to_string(),
            }]
        );
        assert_eq!(
            meta.groups,
            vec![NamedRole {
                name: "factory-maintainers".to_string(),
                role: "maintainer".to_string(),
            }]
        );
        assert_eq!(
            meta.repositories,
            vec![
                Repository {
                    name: "standard".to_string(),
                    arch: vec!["x86_64".to_string(), "aarch64".to_string()],
                    paths: vec![
                        ("openSUSE:Factory:ARM".to_string(), "standard".to_string()),
                        ("openSUSE:Factory".to_string(), "snapshot".to_string()),
                    ],
                },
                Repository {
                    name: "images".to_string(),
                    arch: vec!["x86_64".to_string()],
                    paths: vec![],
                },
            ]
        );
    }

    #[test]
    fn package_meta_parses_everything() {
        let meta = parse_package_meta(PACKAGE_META_FIXTURE).unwrap();
        assert_eq!(meta.name, "foo");
        assert_eq!(meta.project, "openSUSE:Factory");
        assert_eq!(meta.title.as_deref(), Some("Foo package"));
        assert_eq!(meta.description.as_deref(), Some("A test package & more"));
        assert_eq!(meta.url.as_deref(), Some("https://example.com/foo"));
        assert_eq!(
            meta.scmsync.as_deref(),
            Some("https://src.opensuse.org/pool/foo")
        );
        assert_eq!(meta.releasename.as_deref(), Some("foo"));
        assert_eq!(
            meta.devel,
            Some(("devel:languages:python".to_string(), "foo".to_string()))
        );
        assert_eq!(
            meta.persons,
            vec![
                NamedRole {
                    name: "Admin".to_string(),
                    role: "maintainer".to_string(),
                },
                NamedRole {
                    name: "scarabeus_iv".to_string(),
                    role: "bugowner".to_string(),
                },
            ]
        );
        assert_eq!(
            meta.groups,
            vec![NamedRole {
                name: "factory-maintainers".to_string(),
                role: "maintainer".to_string(),
            }]
        );
    }

    #[test]
    fn meta_parses_minimal_documents() {
        let meta = parse_project_meta("<project name=\"home:Admin\"/>").unwrap();
        assert_eq!(meta.name, "home:Admin");
        assert_eq!(meta.title, None);
        assert!(meta.repositories.is_empty());
        let meta = parse_package_meta(
            "<package name=\"foo\" project=\"home:Admin\"><title>Foo</title></package>",
        )
        .unwrap();
        assert_eq!(meta.name, "foo");
        assert_eq!(meta.project, "home:Admin");
        assert_eq!(meta.title.as_deref(), Some("Foo"));
        assert_eq!(meta.devel, None);
    }

    #[test]
    fn meta_rejects_malformed_bodies() {
        assert!(parse_project_meta("").is_err());
        assert!(parse_project_meta("<package name=\"foo\" project=\"p\"/>").is_err());
        assert!(parse_project_meta("<projects name=\"p\"></projects>").is_err());
        assert!(parse_project_meta("<project></project>").is_err());
        assert!(parse_project_meta("<project name=\"p\">").is_err());
        assert!(parse_package_meta("").is_err());
        assert!(parse_package_meta("<project name=\"p\"/>").is_err());
        assert!(parse_package_meta("<package name=\"foo\"></package>").is_err());
        assert!(parse_package_meta("<package name=\"foo\" project=\"p\">").is_err());
    }

    // --- project_meta / package_meta ---

    #[test]
    fn project_meta_reads_without_credentials() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/_meta",
            PROJECT_META_FIXTURE,
        );
        let client = ObsClient::new(&http);
        let meta = client.project_meta("openSUSE:Factory").unwrap();
        assert_eq!(meta.name, "openSUSE:Factory");
        assert_eq!(meta.repositories.len(), 2);
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/public/source/openSUSE:Factory/_meta".to_string(),
                false
            )]
        );
    }

    #[test]
    fn package_meta_reads_without_credentials() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/_meta",
            PACKAGE_META_FIXTURE,
        );
        let client = ObsClient::new(&http);
        let meta = client.package_meta("openSUSE:Factory", "foo").unwrap();
        assert_eq!(meta.name, "foo");
        assert_eq!(meta.project, "openSUSE:Factory");
        assert_eq!(meta.persons.len(), 2);
    }

    #[test]
    fn meta_reads_reject_bad_names() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client.project_meta("openSUSE:Factory/x").is_err());
        assert!(client.package_meta("openSUSE:Factory", "foo/bar").is_err());
        assert!(http.get_calls().is_empty());
    }

    // --- parse_package_search / parse_project_search ---

    const PACKAGE_SEARCH_FIXTURE: &str = "<collection matches=\"2\">\
        <package name=\"foo\" project=\"openSUSE:Factory\">\
        <title>Foo package</title><description>A test package</description>\
        </package>\
        <package name=\"foobar\" project=\"devel:languages:python\"/>\
        </collection>";

    const PROJECT_SEARCH_FIXTURE: &str = "<collection matches=\"1\">\
        <project name=\"devel:languages:python\">\
        <title>Python</title><description>Python packages</description>\
        </project>\
        </collection>";

    #[test]
    fn package_search_parses_matches() {
        let matches = parse_package_search(PACKAGE_SEARCH_FIXTURE).unwrap();
        assert_eq!(
            matches,
            vec![
                PackageMatch {
                    name: "foo".to_string(),
                    project: "openSUSE:Factory".to_string(),
                    title: Some("Foo package".to_string()),
                    description: Some("A test package".to_string()),
                },
                PackageMatch {
                    name: "foobar".to_string(),
                    project: "devel:languages:python".to_string(),
                    title: None,
                    description: None,
                },
            ]
        );
    }

    #[test]
    fn project_search_parses_matches() {
        let matches = parse_project_search(PROJECT_SEARCH_FIXTURE).unwrap();
        assert_eq!(
            matches,
            vec![ProjectMatch {
                name: "devel:languages:python".to_string(),
                title: Some("Python".to_string()),
                description: Some("Python packages".to_string()),
            }]
        );
    }

    #[test]
    fn search_parsers_are_fail_closed() {
        // Not a collection, or no matches count: None.
        assert_eq!(parse_package_search(""), None);
        assert_eq!(parse_package_search("<directory></directory>"), None);
        assert_eq!(
            parse_package_search("<collection><package name=\"x\" project=\"y\"/></collection>"),
            None
        );
        assert_eq!(parse_project_search(""), None);
        // An empty collection (with its count) reads as an empty list.
        assert_eq!(
            parse_package_search("<collection matches=\"0\"></collection>"),
            Some(vec![])
        );
        assert_eq!(
            parse_project_search("<collection matches=\"0\"></collection>"),
            Some(vec![])
        );
        // The count says there are matches but none parsed: None.
        assert_eq!(
            parse_package_search("<collection matches=\"2\"></collection>"),
            None
        );
        assert_eq!(
            parse_project_search("<collection matches=\"1\"></collection>"),
            None
        );
        // An element without its name is skipped, not an error.
        assert_eq!(
            parse_package_search(
                "<collection matches=\"2\"><package project=\"y\"/><package name=\"x\" project=\"y\"/></collection>"
            ),
            Some(vec![PackageMatch {
                name: "x".to_string(),
                project: "y".to_string(),
                title: None,
                description: None,
            }])
        );
    }

    // --- search_packages / search_projects ---

    #[test]
    fn search_packages_sends_the_substring_match_authed() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/package?match=contains%28%40name%2C%27foo%27%29%20or%20contains%28title%2C%27foo%27%29%20or%20contains%28description%2C%27foo%27%29",
            PACKAGE_SEARCH_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let matches = client.search_packages("foo").unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].name, "foo");
        // The search carries credentials.
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/search/package?match=contains%28%40name%2C%27foo%27%29%20or%20contains%28title%2C%27foo%27%29%20or%20contains%28description%2C%27foo%27%29"
                    .to_string(),
                true
            )]
        );
    }

    #[test]
    fn search_projects_sends_the_substring_match_authed() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/project?match=contains%28%40name%2C%27python%27%29%20or%20contains%28title%2C%27python%27%29%20or%20contains%28description%2C%27python%27%29",
            PROJECT_SEARCH_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let matches = client.search_projects("python").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "devel:languages:python");
    }

    #[test]
    fn search_requires_credentials() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client.search_packages("foo").is_err());
        assert!(client.search_projects("foo").is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn search_rejects_bad_terms() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client.search_packages("").is_err());
        assert!(client.search_packages("foo' or '1'='1").is_err());
        assert!(client.search_projects("").is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn search_unexpected_response_is_an_error() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/package?match=contains%28%40name%2C%27foo%27%29%20or%20contains%28title%2C%27foo%27%29%20or%20contains%28description%2C%27foo%27%29",
            "<collection><package name=\"x\" project=\"y\"/></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client.search_packages("foo").is_err());
    }

    // --- parse_owners / parse_person ---

    const OWNER_SEARCH_FIXTURE: &str = "<collection matches=\"2\">\
        <owner rootproject=\"openSUSE:Factory\" project=\"devel:languages:python\" package=\"foo\">\
        <person name=\"Admin\" role=\"maintainer\"/>\
        <person name=\"scarabeus_iv\" role=\"bugowner\"/>\
        <group name=\"factory-maintainers\" role=\"maintainer\"/>\
        </owner>\
        <owner rootproject=\"openSUSE:Factory\" project=\"openSUSE:Factory\">\
        <person name=\"Admin\" role=\"maintainer\"/>\
        </owner>\
        </collection>";

    const PERSON_FIXTURE: &str = "<person>\
        <login>Admin</login><email>admin@example.com</email>\
        <realname>Admin User</realname><state>confirmed</state>\
        <globalrole>Admin</globalrole>\
        </person>";

    #[test]
    fn owner_search_parses_owners() {
        let owners = parse_owners(OWNER_SEARCH_FIXTURE).unwrap();
        assert_eq!(
            owners,
            vec![
                Owner {
                    root_project: "openSUSE:Factory".to_string(),
                    project: "devel:languages:python".to_string(),
                    package: Some("foo".to_string()),
                    persons: vec![
                        NamedRole {
                            name: "Admin".to_string(),
                            role: "maintainer".to_string(),
                        },
                        NamedRole {
                            name: "scarabeus_iv".to_string(),
                            role: "bugowner".to_string(),
                        },
                    ],
                    groups: vec![NamedRole {
                        name: "factory-maintainers".to_string(),
                        role: "maintainer".to_string(),
                    }],
                },
                Owner {
                    root_project: "openSUSE:Factory".to_string(),
                    project: "openSUSE:Factory".to_string(),
                    package: None,
                    persons: vec![NamedRole {
                        name: "Admin".to_string(),
                        role: "maintainer".to_string(),
                    }],
                    groups: vec![],
                },
            ]
        );
    }

    #[test]
    fn owner_search_parsers_are_fail_closed() {
        assert_eq!(parse_owners(""), None);
        assert_eq!(parse_owners("<directory></directory>"), None);
        // A `matches` count that does not parse is unexpected.
        assert_eq!(
            parse_owners("<collection matches=\"many\"></collection>"),
            None
        );
        // An empty collection reads as an empty list.
        assert_eq!(
            parse_owners("<collection matches=\"0\"></collection>"),
            Some(vec![])
        );
        // An owner without its project is skipped, not an error.
        assert_eq!(
            parse_owners("<collection><owner rootproject=\"p\"/></collection>"),
            Some(vec![])
        );
    }

    #[test]
    fn person_parses_everything() {
        let person = parse_person(PERSON_FIXTURE).unwrap();
        assert_eq!(
            person,
            Person {
                login: "Admin".to_string(),
                email: "admin@example.com".to_string(),
                realname: Some("Admin User".to_string()),
                state: "confirmed".to_string(),
            }
        );
    }

    #[test]
    fn person_without_realname_is_none() {
        let person = parse_person(
            "<person><login>Admin</login><email>a@b.c</email><state>confirmed</state></person>",
        )
        .unwrap();
        assert_eq!(person.realname, None);
    }

    #[test]
    fn person_rejects_malformed_bodies() {
        assert_eq!(parse_person(""), None);
        assert_eq!(parse_person("<collection></collection>"), None);
        assert_eq!(
            parse_person("<person><email>a@b.c</email><state>confirmed</state></person>"),
            None
        );
        assert_eq!(
            parse_person("<person><login>Admin</login><state>confirmed</state></person>"),
            None
        );
        assert_eq!(
            parse_person("<person><login>Admin</login><email>a@b.c</email></person>"),
            None
        );
        assert_eq!(
            parse_person(
                "<person><login>Admin</login><email>a@b.c</email><state>confirmed</state>"
            ),
            None
        );
    }

    #[test]
    fn login_validation_matches_the_server_format() {
        assert!(validate_login("Admin").is_ok());
        assert!(validate_login("ab").is_ok());
        assert!(validate_login("a b").is_ok());
        assert!(validate_login("a$b^c-d.e#f*g+h&i'j\"k").is_ok());
        assert!(validate_login("a").is_err());
        assert!(validate_login("").is_err());
        assert!(validate_login("a/b").is_err());
        assert!(validate_login("a?b").is_err());
        assert!(validate_login("a:b").is_err());
        assert!(validate_login(&"a".repeat(101)).is_err());
        assert!(validate_login(&"a".repeat(100)).is_ok());
    }

    // --- owners / owners_for_binary / maintainers / whois ---

    #[test]
    fn owners_searches_by_project_and_package_authed() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/owner?project=openSUSE%3AFactory&package=foo",
            OWNER_SEARCH_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let owners = client.owners("openSUSE:Factory", "foo").unwrap();
        assert_eq!(owners.len(), 2);
        assert_eq!(owners[0].package.as_deref(), Some("foo"));
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/search/owner?project=openSUSE%3AFactory&package=foo"
                    .to_string(),
                true
            )]
        );
    }

    #[test]
    fn owners_for_binary_searches_by_binary() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/owner?binary=python3.11",
            OWNER_SEARCH_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let owners = client.owners_for_binary("python3.11").unwrap();
        assert_eq!(owners.len(), 2);
    }

    #[test]
    fn maintainers_filters_to_the_maintainer_role() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/owner?project=openSUSE%3AFactory&package=foo",
            OWNER_SEARCH_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client.maintainers("openSUSE:Factory", "foo").unwrap(),
            vec![
                NamedRole {
                    name: "Admin".to_string(),
                    role: "maintainer".to_string(),
                },
                NamedRole {
                    name: "factory-maintainers".to_string(),
                    role: "maintainer".to_string(),
                },
                NamedRole {
                    name: "Admin".to_string(),
                    role: "maintainer".to_string(),
                },
            ]
        );
    }

    #[test]
    fn whois_reads_the_person_authed() {
        let http =
            FakeHttp::new().with_get("https://api.opensuse.org/person/Admin", PERSON_FIXTURE);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let person = client.whois("Admin").unwrap();
        assert_eq!(person.login, "Admin");
        assert_eq!(person.state, "confirmed");
        assert_eq!(
            http.get_calls(),
            vec![("https://api.opensuse.org/person/Admin".to_string(), true)]
        );
    }

    #[test]
    fn owner_and_whois_reads_require_credentials() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client.owners("openSUSE:Factory", "foo").is_err());
        assert!(client.owners_for_binary("python3.11").is_err());
        assert!(client.maintainers("openSUSE:Factory", "foo").is_err());
        assert!(client.whois("Admin").is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn owner_and_whois_reads_reject_bad_inputs() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client.owners("openSUSE:Factory", "foo/bar").is_err());
        assert!(client.owners_for_binary("python3.11/x").is_err());
        assert!(client.whois("a").is_err());
        assert!(client.whois("a/b").is_err());
        assert!(http.get_calls().is_empty());
    }
}
