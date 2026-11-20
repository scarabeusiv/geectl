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
use crate::error::{ErrorKind, GeectlError};
use crate::http::{is_loopback_url, HttpClient};
use crate::url::{encode_path_segment, encode_query_value, UrlBuilder};
use crate::xml::XmlDoc;
use std::cell::RefCell;
use std::fmt;

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
    // would rewrite them all. The live harness enforces the same
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

/// Whether a package is the same, different, or only on one side
/// of a project diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageDiffStatus {
    /// Same files, same contents.
    Identical,
    /// Files added, removed, or changed.
    Differs,
    /// Only in the old project.
    OnlyInOld,
    /// Only in the new project.
    OnlyInNew,
}

/// One package's diff between two projects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageDiff {
    /// Package name.
    pub name: String,
    /// Whether the package is identical, differs, or is only on one
    /// side.
    pub status: PackageDiffStatus,
    /// The changed files, sorted by name (empty for identical
    /// packages and for packages only on one side).
    pub files: Vec<FilePatch>,
    /// The ids of the open submit requests involving this package in
    /// the new project, when the diff was annotated (empty
    /// otherwise).
    pub requests: Vec<String>,
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

/// Options for [`ObsClient::branch_package`]. All fields are optional
/// except through the method arguments: an unset target means the
/// server default, an unset flag means off. The defaults branch the
/// package into the caller's `home:<user>:branches:<project>` project
/// (the server default), resolving the link source through the devel
/// project when one is registered.
///
/// The flags are the server's `cmd=branch` parameters, named after the
/// behaviour rather than the wire spelling: `no_devel_project` is the
/// server's `ignoredevel`, `new_package` is `missingok` when the source
/// does not exist yet and `newinstance` when it already does (the source
/// is read first, see [`ObsClient::branch_package`]).
#[derive(Debug, Clone, Default)]
pub struct BranchOptions {
    /// The branch project. None means the server default — usually
    /// `home:<user>:branches:<source project>`, but the server may
    /// pick another project (an incident project when branching as
    /// maintenance), so the key is omitted and the client never
    /// guesses it.
    pub target_project: Option<String>,
    /// The branch package name. None means the source package name
    /// (the key is omitted and the server applies its default).
    pub target_package: Option<String>,
    /// Ignore the devel project when resolving the link source: the
    /// branch links at the project branched from instead of the devel
    /// project. The server's `ignoredevel`.
    pub no_devel_project: bool,
    /// Branch a package that does not exist yet (or that only exists
    /// through a project link): the server's `missingok` when the
    /// source does not exist yet, `newinstance` when it already
    /// does. See [`ObsClient::branch_package`].
    pub new_package: bool,
    /// Overwrite an existing target package. The server's `force`.
    pub force: bool,
    /// Branch from this source revision instead of the latest. The
    /// server's `rev`.
    pub revision: Option<String>,
    /// The link revision the branch records. The server's `linkrev`.
    pub linkrev: Option<String>,
    /// A comment recorded on the branch. The server's `comment`.
    pub comment: Option<String>,
    /// Do not give the branch read access to the source package's
    /// project. The server's `noaccess`.
    pub noaccess: bool,
    /// Add the source project's repositories to the branch project.
    /// The server's `add_repositories`.
    pub add_repositories: bool,
    /// Block the added repositories. Only sent with
    /// `add_repositories`. The server's `add_repositories_block`.
    pub add_repositories_block: Option<String>,
    /// Rebuild the added repositories. Only sent with
    /// `add_repositories`. The server's `add_repositories_rebuild`.
    pub add_repositories_rebuild: Option<String>,
    /// Extend the branch package names with the source project name.
    /// The server's `extend_package_names`.
    pub extend_package_names: bool,
    /// Branch as a maintenance incident. The server's `maintenance`.
    pub maintenance: bool,
}

/// Where a branch landed: the source package and the branch package
/// the server created for it, as reported by the server's
/// `cmd=branch` response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchResult {
    /// The project branched from.
    pub source_project: String,
    /// The package branched from.
    pub source_package: String,
    /// The project the branch lives in.
    pub target_project: String,
    /// The branch package name.
    pub target_package: String,
}
/// Options for [`ObsClient::copy_package`]. All fields are optional:
/// an unset revision means the latest, an unset flag means off.
#[derive(Debug, Clone, Default)]
pub struct CopyOptions {
    /// The source revision to copy (a number or an md5). None means
    /// the latest. The server's `orev`.
    pub revision: Option<String>,
    /// Keep the source package's link when copying a link package.
    /// The server's `keeplink`.
    pub keep_link: bool,
    /// Copy the expanded source (the link resolved) instead of the
    /// link itself. The server's `expand`.
    pub expand: bool,
    /// A comment recorded on the copy. The server's `comment`.
    pub comment: Option<String>,
    /// Do not run source services on the copied files. The server's
    /// `noservice`.
    pub noservice: bool,
    /// Do not update the source package when copying a link onto its
    /// target. The server's `dontupdatesource`.
    pub dont_update_source: bool,
    /// Carry the source revision's vrev over to the copy. The
    /// server's `withvrev`.
    pub with_vrev: bool,
}

/// Options for [`ObsClient::copy_project`]. All fields are optional:
/// an unset flag means off. The copy runs as a background job unless
/// `no_delay` is set.
#[derive(Debug, Clone, Default)]
pub struct CopyProjectOptions {
    /// Copy the built binaries as well. Admin-only on the server.
    /// The server's `withbinaries`.
    pub with_binaries: bool,
    /// Copy the full history. The server's `withhistory`.
    pub with_history: bool,
    /// Make the copied project older than the source (for rebuild
    /// ordering). The server's `makeolder`.
    pub make_older: bool,
    /// Make the origin older than the copy. The server's
    /// `makeoriginolder`.
    pub make_origin_older: bool,
    /// Do not run source services on the copied files. The server's
    /// `noservice`.
    pub noservice: bool,
    /// Re-sign the copied packages. The server's `resign`.
    pub resign: bool,
    /// A comment recorded on the copy. The server's `comment`.
    pub comment: Option<String>,
    /// Run the copy synchronously and wait for it, instead of the
    /// server default of queueing it as a job (which answers
    /// `invoked`). The server's `nodelay`.
    pub no_delay: bool,
}

/// Options for [`ObsClient::link_package`]. All fields are optional:
/// an unset revision means no pin, an unset flag means off.
#[derive(Debug, Clone, Default)]
pub struct LinkOptions {
    /// Replace an existing link in the target package. Without it,
    /// linking onto a package that already has a link is refused.
    pub force: bool,
    /// Pin the link at this source revision (a number or an md5).
    /// The server's `rev` attribute on the link.
    pub revision: Option<String>,
    /// The link's vrev attribute.
    pub vrev: Option<String>,
    /// The link's cicount attribute.
    pub cicount: Option<String>,
    /// Allow the link to point at a package that does not exist yet.
    /// The server's `missingok` attribute on the link.
    pub missing_target: bool,
    /// Disable building the target package. The server's
    /// `<build><disable/></build>` in the target meta.
    pub disable_build: bool,
    /// Disable publishing the target package. The server's
    /// `<publish><disable/></publish>` in the target meta.
    pub disable_publish: bool,
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
                    // without posting again.
                    if let Ok(request) = self.show_request(&id.to_string()) {
                        if request.state.name == state {
                            return Ok(());
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
    /// the change, the server is unreachable — prints one warning to
    /// stderr and returns `false` instead of failing, so a batch
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
                eprintln!("warning: could not change state of submit request {id}: {err}");
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

    /// PUT `url` with `body`, retrying a not-found while `budget`
    /// lasts: the server's API answers before its backend has
    /// registered a just-created parent, so a PUT that follows a
    /// creation can 404 on a parent that is about to exist. Only a
    /// not-found retries — a 410 maps to the same kind — and the
    /// retry sleeps 500 ms between attempts, so a genuinely missing
    /// parent still fails loudly once the budget runs out instead of
    /// hanging. The PUT is idempotent, so a retry cannot duplicate
    /// anything. Callers pass the budget; [`ObsClient::create_package`]
    /// passes 30 s.
    fn put_with_registration_retry(
        &self,
        url: &str,
        body: &str,
        budget: std::time::Duration,
    ) -> Result<String, GeectlError> {
        let deadline = std::time::Instant::now() + budget;
        loop {
            match self.http.put(url, body, &self.credentials) {
                Ok(body) => return Ok(body),
                Err(err)
                    if err.kind() == ErrorKind::NotFound
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                Err(err) => return Err(err),
            }
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
    ///
    /// The PUT retries a not-found for a bounded 30 s: the server's
    /// API and backend are separate processes, and the API replies to
    /// the project creation before the backend has the project
    /// registered — the immediately following package creation can
    /// then 404 on the just-created project (seen on live run
    /// 44876734097). Only a not-found retries, so anything else still
    /// fails loudly; a genuinely missing project takes about 30 s to
    /// fail instead of failing fast.
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
        let body =
            self.put_with_registration_retry(&url, &meta, std::time::Duration::from_secs(30))?;
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

    /// The link target of package `package` in project `project`
    /// (`GET /public/source/<project>/<package>`, reading the
    /// listing's `<linkinfo project package/>`), or None when the
    /// package links nothing. A public read: no credentials. A
    /// missing package is an error, never None — only a listing
    /// without linkinfo reads as "no link".
    pub fn link_target(
        &self,
        project: &str,
        package: &str,
    ) -> Result<Option<(String, String)>, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        Ok(self.public_directory(&[project, package])?.linkinfo)
    }

    /// The per-file diff between two listings of the same package:
    /// files added, modified (contents differ), or deleted, each
    /// with a unified diff. Unchanged files do not appear.
    fn diff_files(
        &self,
        old_project: &str,
        package: &str,
        old_files: &[SourceFile],
        new_project: &str,
        new_files: &[SourceFile],
    ) -> Result<Vec<FilePatch>, GeectlError> {
        let mut patches = Vec::new();
        for new in new_files {
            match old_files.iter().find(|old| old.name == new.name) {
                None => {
                    // Added: the whole file is new.
                    let content = self.cat(new_project, package, &new.name, None)?;
                    patches.push(FilePatch {
                        filename: new.name.clone(),
                        status: FilePatchStatus::Added,
                        diff: unified_diff("/dev/null", "", &format!("b/{}", new.name), &content),
                    });
                }
                Some(old) if old.md5 != new.md5 => {
                    let old_content = self.cat(old_project, package, &new.name, None)?;
                    let new_content = self.cat(new_project, package, &new.name, None)?;
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
        for old in old_files {
            if !new_files.iter().any(|new| new.name == old.name) {
                let content = self.cat(old_project, package, &old.name, None)?;
                patches.push(FilePatch {
                    filename: old.name.clone(),
                    status: FilePatchStatus::Deleted,
                    diff: unified_diff(&format!("a/{}", old.name), &content, "/dev/null", ""),
                });
            }
        }
        patches.sort_by(|a, b| a.filename.cmp(&b.filename));
        Ok(patches)
    }

    /// Diff two projects package by package (`old` vs `new`): every
    /// package in either project is classified — identical (same
    /// files, same contents), differs (files added, removed, or
    /// changed), only-in-old, or only-in-new — with a per-file
    /// unified diff for the packages that differ. `include` narrows
    /// the diff to the named packages (empty means all); `exclude`
    /// drops the named packages. With `annotate_requests`, each
    /// differing package is annotated with the ids of the open
    /// submit requests involving it in the `new` project (which
    /// requires credentials, like
    /// [`ObsClient::open_requests_for_package`]).
    ///
    /// A package only in the new project is its own status
    /// (`OnlyInNew`), not folded into identical: the caller decides
    /// what a new package means. The file comparison is by md5, so a
    /// file that only changed metadata does not count as changed.
    pub fn project_diff(
        &self,
        old: &str,
        new: &str,
        include: &[&str],
        exclude: &[&str],
        annotate_requests: bool,
    ) -> Result<Vec<PackageDiff>, GeectlError> {
        for (label, value) in [("old project", old), ("new project", new)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        for name in include.iter().chain(exclude.iter()) {
            if !valid_obs_name(name) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS package name: {name:?}"),
                ));
            }
        }
        let old_packages = self.list_packages(old)?;
        let new_packages = self.list_packages(new)?;
        let mut names: Vec<String> = old_packages
            .iter()
            .chain(new_packages.iter())
            .cloned()
            .collect();
        names.sort();
        names.dedup();
        let mut diffs = Vec::new();
        for name in names {
            if !include.is_empty() && !include.contains(&name.as_str()) {
                continue;
            }
            if exclude.contains(&name.as_str()) {
                continue;
            }
            let in_old = old_packages.contains(&name);
            let in_new = new_packages.contains(&name);
            let (status, files) = match (in_old, in_new) {
                (true, false) => (PackageDiffStatus::OnlyInOld, Vec::new()),
                (false, true) => (PackageDiffStatus::OnlyInNew, Vec::new()),
                (true, true) => {
                    let old_files = self.list_files(old, &name)?;
                    let new_files = self.list_files(new, &name)?;
                    let files = self.diff_files(old, &name, &old_files, new, &new_files)?;
                    let status = if files.is_empty() {
                        PackageDiffStatus::Identical
                    } else {
                        PackageDiffStatus::Differs
                    };
                    (status, files)
                }
                (false, false) => unreachable!("package in neither project"),
            };
            let requests = if annotate_requests && status == PackageDiffStatus::Differs {
                self.open_requests_for_package(new, &name)?
            } else {
                Vec::new()
            };
            diffs.push(PackageDiff {
                name,
                status,
                files,
                requests,
            });
        }
        Ok(diffs)
    }

    /// Build the OBS search URL for open submit requests targeting
    /// `package`. The match expression is XPath over the request XML;
    /// the submit restriction is applied client-side in
    /// `parse_submit_request_ids`, so a request of another type that
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
        // open states. It does NOT filter by action type: the submit
        // filter lives client-side in `parse_submit_request_ids`, on
        // the response the server actually returns. The search
        // engine does whitelist `action/@type`, so the match could
        // filter server-side — that is a follow-up, not this change;
        // the client-side filter stays as defence-in-depth.
        let match_expr = format!(
            "action/target/@package='{package}' and (state/@name='new' or state/@name='review')"
        );
        Ok(build_url(
            &self.base,
            &["search", "request"],
            &[("match", &match_expr)],
        ))
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
        // (or unsetting one that is not) writes nothing.
        if rendered == meta.trim() {
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
        let (mut doc, put_meta, target_exists_here) = match &target_meta {
            Some(meta) => {
                let doc = XmlDoc::parse(meta)?;
                if doc.root_attr("project").as_deref() == Some(target_project) {
                    (doc, false, true)
                } else {
                    (
                        build_link_meta(&source_meta, target_project, target_package)?,
                        true,
                        false,
                    )
                }
            }
            None => (
                build_link_meta(&source_meta, target_project, target_package)?,
                true,
                false,
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
        if put_meta || target_meta.as_deref() != Some(rendered.as_str()) {
            let body = self.http.put(&target_url, &rendered, &self.credentials)?;
            if !parse_status_ok(&body) {
                return Err(GeectlError::new(
                    ErrorKind::ServerError,
                    format!(
                        "unexpected OBS link meta response for package {target_project}/{target_package}"
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
            None => Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected OBS link response for package {target_project}/{target_package}"
                ),
            )),
        }
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
/// `type="submit"`. The search engine does whitelist `action/@type`,
/// so the type filter could live in the match — it lives here
/// instead, on the response the server actually returns, as
/// defence-in-depth. The search match still
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

/// The text of the first `<data name="NAME">VALUE</data>` element in
/// a `<status>` document. None when there is no such element, or it
/// is self-closing (an empty value is not a result). The name is
/// matched exactly, quoted: `<data name="target_project">` does not
/// match a search for `project`.
pub(crate) fn status_data(body: &str, name: &str) -> Option<String> {
    let open = format!("<data name=\"{name}\">");
    let pos = body.find(&open)?;
    let after = &body[pos + open.len()..];
    let end = after.find("</data>")?;
    let value = &after[..end];
    if value.is_empty() {
        return None;
    }
    Some(value.to_string())
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
/// True when `body` is a `<status code="invoked"/>` document: the
/// server queued the operation as a job instead of running it (a
/// project copy or a project release without `nodelay`). The same
/// tag-boundary discipline as [`parse_status_ok`]: the code must be
/// the `code` attribute of the `<status>` element itself, not text
/// after it and not a longer attribute name ending in it.
pub(crate) fn parse_status_invoked(body: &str) -> bool {
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
    // ends in it (`mycode="invoked"` is not `code="invoked"`).
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
        return &after_eq[1..1 + close] == "invoked";
    }
    false
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
fn build_link_meta(
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
            let end = rest.find("?>")?;
            rest[end + 2..].trim_start()
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
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};

    /// A fake HTTP client with canned responses, for fixtures. GET,
    /// POST, and DELETE are keyed separately by URL; a URL with no
    /// canned response is a transport error. Calls are recorded so
    /// tests can assert on the method, the URL, whether auth was
    /// sent, and (for POST) the body.
    struct FakeHttp {
        gets: RefCell<HashMap<String, String>>,
        get_errors: RefCell<HashMap<String, ErrorKind>>,
        posts: RefCell<HashMap<String, String>>,
        post_errors: RefCell<HashMap<String, VecDeque<ErrorKind>>>,
        deletes: RefCell<HashMap<String, String>>,
        puts: RefCell<HashMap<String, String>>,
        put_errors: RefCell<HashMap<String, VecDeque<ErrorKind>>>,
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
                post_errors: RefCell::new(HashMap::new()),
                deletes: RefCell::new(HashMap::new()),
                puts: RefCell::new(HashMap::new()),
                put_errors: RefCell::new(HashMap::new()),
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

        fn with_post_error(self, url: &str, kind: ErrorKind) -> Self {
            self.post_errors
                .borrow_mut()
                .insert(url.to_string(), VecDeque::from([kind]));
            self
        }

        /// Answer the next POSTs to `url` with these errors, in order;
        /// once the queue is empty the canned `posts` body answers.
        /// The POST twin of `with_put_error_sequence`: lets a test
        /// model a backend that is not ready yet and then catches up.
        fn with_post_error_sequence(self, url: &str, kinds: &[ErrorKind]) -> Self {
            self.post_errors
                .borrow_mut()
                .insert(url.to_string(), kinds.iter().copied().collect());
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

        /// Answer the next PUTs to `url` with these errors, in order;
        /// once the queue is empty the canned `puts` body answers.
        /// Lets a test model a backend that is not ready yet and then
        /// catches up, instead of a single static answer.
        fn with_put_error_sequence(self, url: &str, kinds: &[ErrorKind]) -> Self {
            self.put_errors
                .borrow_mut()
                .insert(url.to_string(), kinds.iter().copied().collect());
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
            if let Some(kind) = self
                .post_errors
                .borrow_mut()
                .get_mut(url)
                .and_then(|queue| queue.pop_front())
            {
                return Err(GeectlError::new(
                    kind,
                    format!("canned POST error for {url}"),
                ));
            }
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
            if let Some(kind) = self
                .put_errors
                .borrow_mut()
                .get_mut(url)
                .and_then(|queue| queue.pop_front())
            {
                return Err(GeectlError::new(
                    kind,
                    format!("canned PUT error for {url}"),
                ));
            }
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
    fn parse_status_invoked_reads_the_status_code() {
        assert!(parse_status_invoked(
            "<status code=\"invoked\"><summary>Job invoked</summary></status>"
        ));
        assert!(parse_status_invoked("<status code=\"invoked\" />"));
        assert!(!parse_status_invoked("<status code=\"ok\" />"));
        assert!(!parse_status_invoked("<status code=\"error\" />"));
        assert!(!parse_status_invoked(""));
        assert!(!parse_status_invoked("<other code=\"invoked\" />"));
        assert!(!parse_status_invoked("<status mycode=\"invoked\" />"));
        assert!(parse_status_invoked("<status code='invoked' />"));
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
                "Live test branch",
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
            "<project name=\"home:alice\"><title>Alice&apos;s branch</title><description>Live test branch</description></project>"
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
                "Live test branch",
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
                "Live test branch",
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
                "Live test branch",
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
            .create_project("home:alice", "Alice's branch", "Live test branch", &dry)
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
                "Live test package",
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
            "<package name=\"foo\" project=\"home:alice\"><title>Foo</title><description>Live test package</description></package>"
        );
        assert!(puts[0].2);
    }

    #[test]
    fn create_package_retries_not_found_then_succeeds() {
        // The server's API replies to the project creation before
        // its backend has the project registered, so the immediately
        // following package PUT can 404 on the just-created project
        // (live run 44876734097). The library retries a
        // not-found; the second PUT lands.
        let http = FakeHttp::new()
            .with_put_error_sequence(
                "https://api.opensuse.org/source/home:alice/foo/_meta",
                &[ErrorKind::NotFound],
            )
            .with_put(
                "https://api.opensuse.org/source/home:alice/foo/_meta",
                "<status code=\"ok\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .create_package(
                "home:alice",
                "foo",
                "Foo",
                "Live test package",
                &DryRun::new(false)
            )
            .is_ok());
        assert_eq!(http.put_calls().len(), 2);
    }

    #[test]
    fn create_package_persistent_not_found_surfaces_after_the_budget() {
        // A genuinely missing project still fails loudly once the
        // budget runs out: with a 400 ms budget the helper tries
        // twice (the first failure, one 500 ms sleep, the second
        // failure past the deadline) and returns the not-found
        // instead of hanging. The production budget is 30 s, passed
        // by create_package; the helper takes the budget so this
        // test does not have to wait it out.
        let http = FakeHttp::new().with_put_error_sequence(
            "https://api.opensuse.org/source/home:alice/foo/_meta",
            &[
                ErrorKind::NotFound,
                ErrorKind::NotFound,
                ErrorKind::NotFound,
            ],
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .put_with_registration_retry(
                "https://api.opensuse.org/source/home:alice/foo/_meta",
                "<package name=\"foo\" project=\"home:alice\" />",
                std::time::Duration::from_millis(400),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert_eq!(http.put_calls().len(), 2);
    }

    #[test]
    fn create_package_does_not_retry_other_errors() {
        // Only a not-found retries: a server error fails fast on
        // the first attempt instead of burning the budget.
        let http = FakeHttp::new()
            .with_put_error_sequence(
                "https://api.opensuse.org/source/home:alice/foo/_meta",
                &[ErrorKind::ServerError],
            )
            .with_put(
                "https://api.opensuse.org/source/home:alice/foo/_meta",
                "<status code=\"ok\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .create_package(
                "home:alice",
                "foo",
                "Foo",
                "Live test package",
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert_eq!(http.put_calls().len(), 1);
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
                "Live test package",
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
                "Live test package",
                &DryRun::new(false)
            )
            .is_err());
        assert!(client
            .create_package(
                "home:alice",
                "foo/bar",
                "Foo",
                "Live test package",
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
            .create_package("home:alice", "foo", "Foo", "Live test package", &dry)
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

    // --- link_target ---

    #[test]
    fn link_target_reads_linkinfo_without_credentials() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo",
            DIRECTORY_FIXTURE,
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.link_target("openSUSE:Factory", "foo").unwrap(),
            Some(("home:Admin".to_string(), "foo".to_string()))
        );
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo".to_string(),
                false
            )]
        );
    }

    #[test]
    fn link_target_without_linkinfo_is_none() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo",
            "<directory><entry name=\"foo.spec\"/></directory>",
        );
        let client = ObsClient::new(&http);
        assert_eq!(client.link_target("openSUSE:Factory", "foo").unwrap(), None);
    }

    #[test]
    fn link_target_missing_package_is_an_error() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo",
            ErrorKind::NotFound,
        );
        let client = ObsClient::new(&http);
        let err = client.link_target("openSUSE:Factory", "foo").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn link_target_rejects_bad_names() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client.link_target("openSUSE:Factory", "foo/bar").is_err());
        assert!(http.get_calls().is_empty());
    }

    // --- project_diff ---

    /// The request-search URL `open_requests_for_package` builds,
    /// for the FakeHttp fixture keys. The URL builder itself is
    /// tested separately; here it only supplies the fixture key.
    fn requests_url(project: &str, package: &str) -> String {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        client
            .package_requests_search_url(project, package)
            .unwrap()
    }

    const OLD_PACKAGES_FIXTURE: &str = "<directory>\
        <entry name=\"common-one\"/><entry name=\"common-two\"/>\
        <entry name=\"common-three\"/><entry name=\"old-only\"/>\
        </directory>";
    const NEW_PACKAGES_FIXTURE: &str = "<directory>\
        <entry name=\"common-one\"/><entry name=\"common-two\"/>\
        <entry name=\"common-three\"/><entry name=\"new-only\"/>\
        </directory>";

    /// A FakeHttp with the two project listings and the file
    /// listings for the shared packages: `common-one` and
    /// `common-three` identical on both sides, `common-two` with a
    /// changed `foo.spec`.
    fn diff_http() -> FakeHttp {
        FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/old",
                OLD_PACKAGES_FIXTURE,
            )
            .with_get(
                "https://api.opensuse.org/public/source/new",
                NEW_PACKAGES_FIXTURE,
            )
            .with_get(
                "https://api.opensuse.org/public/source/old/common-one",
                "<directory><entry name=\"a.spec\" md5=\"111\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/new/common-one",
                "<directory><entry name=\"a.spec\" md5=\"111\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/old/common-two",
                "<directory><entry name=\"foo.spec\" md5=\"aaa\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/new/common-two",
                "<directory><entry name=\"foo.spec\" md5=\"bbb\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/old/common-three",
                "<directory><entry name=\"c.spec\" md5=\"222\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/new/common-three",
                "<directory><entry name=\"c.spec\" md5=\"222\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/old/common-two/foo.spec",
                "Version: 1.0\n",
            )
            .with_get(
                "https://api.opensuse.org/public/source/new/common-two/foo.spec",
                "Version: 1.1\n",
            )
    }

    #[test]
    fn project_diff_classifies_packages() {
        let http = diff_http();
        let client = ObsClient::new(&http);
        let diffs = client.project_diff("old", "new", &[], &[], false).unwrap();
        assert_eq!(
            diffs
                .iter()
                .map(|d| (d.name.as_str(), d.status))
                .collect::<Vec<_>>(),
            vec![
                ("common-one", PackageDiffStatus::Identical),
                ("common-three", PackageDiffStatus::Identical),
                ("common-two", PackageDiffStatus::Differs),
                ("new-only", PackageDiffStatus::OnlyInNew),
                ("old-only", PackageDiffStatus::OnlyInOld),
            ]
        );
        assert_eq!(diffs[0].files, Vec::<FilePatch>::new());
        assert_eq!(diffs[0].requests, Vec::<String>::new());
        let changed = &diffs[2];
        assert_eq!(
            changed.files,
            vec![FilePatch {
                filename: "foo.spec".to_string(),
                status: FilePatchStatus::Modified,
                diff: "--- a/foo.spec\n+++ b/foo.spec\n@@ -1 +1 @@\n-Version: 1.0\n+Version: 1.1\n"
                    .to_string(),
            }]
        );
    }

    #[test]
    fn project_diff_include_and_exclude_narrow_the_diff() {
        let http = diff_http();
        let client = ObsClient::new(&http);
        let diffs = client
            .project_diff("old", "new", &["common-two"], &[], false)
            .unwrap();
        assert_eq!(
            diffs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            vec!["common-two"]
        );
        let http = diff_http();
        let client = ObsClient::new(&http);
        let diffs = client
            .project_diff("old", "new", &[], &["common-two"], false)
            .unwrap();
        assert_eq!(
            diffs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            vec!["common-one", "common-three", "new-only", "old-only"]
        );
        let http = diff_http();
        let client = ObsClient::new(&http);
        let diffs = client
            .project_diff("old", "new", &["common-one"], &["common-one"], false)
            .unwrap();
        assert_eq!(diffs, Vec::<PackageDiff>::new());
    }

    #[test]
    fn project_diff_annotations_carry_open_request_ids() {
        let http = diff_http().with_get(
            &requests_url("new", "common-two"),
            "<collection matches=\"1\"><request id=\"42\"/></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let diffs = client.project_diff("old", "new", &[], &[], true).unwrap();
        let changed = diffs
            .iter()
            .find(|d| d.name == "common-two")
            .expect("common-two differs");
        assert_eq!(changed.requests, vec!["42".to_string()]);
        let identical = diffs
            .iter()
            .find(|d| d.name == "common-one")
            .expect("common-one identical");
        assert_eq!(identical.requests, Vec::<String>::new());
    }

    #[test]
    fn project_diff_annotation_requires_credentials() {
        let http = diff_http();
        let client = ObsClient::new(&http);
        let err = client
            .project_diff("old", "new", &[], &[], true)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
    }

    #[test]
    fn project_diff_rejects_bad_names_without_requesting() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client
            .project_diff("old", "new", &["bad/name"], &[], false)
            .is_err());
        assert!(client
            .project_diff("old", "new", &[], &["bad/name"], false)
            .is_err());
        assert!(client.project_diff("o/l", "new", &[], &[], false).is_err());
        assert!(http.get_calls().is_empty());
    }

    // -- Request XML build/parse/round-trip (adopted from the
    // request XML tests: submit/add_role/set_bugowner/delete/
    // change_devel construction, action parsing, full-request reads,
    // empty comment/description, legacy form, action filtering).

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
    fn xml_unescape_decodes_named_and_numeric_entities() {
        assert_eq!(
            xml_unescape("a &amp; b &lt; c &gt; d &quot;e&quot; &apos;f&apos;"),
            "a & b < c > d \"e\" 'f'"
        );
        assert_eq!(xml_unescape("&#65;&#x42;"), "AB");
        assert_eq!(xml_unescape("&amp;lt;"), "&lt;");
        assert_eq!(xml_unescape("plain text"), "plain text");
        assert_eq!(xml_unescape("&bogus; stays"), "&bogus; stays");
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

    // -- XPath query construction (adopted from the `match=` string
    // forms: noop, not, eq, not-eq, contains, and/or with the
    // parenthesisation rules, eq lists, and element predicates).

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

    // -- Request search `match=` construction (adopted from the
    // search rules: default states, ids, projects, packages, source
    // / target, creators, who, states, types, exclusions, history).

    fn search_without_states() -> RequestSearch {
        RequestSearch::new().states(Vec::new())
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
    fn search_projects_match_source_or_target() {
        let search = search_without_states().projects(vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "source[@project='foo' or @project='bar'] or target[@project='foo' or @project='bar']"
        );
    }

    #[test]
    fn search_packages_match_source_or_target() {
        let search = search_without_states().packages(vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "source[@package='foo' or @package='bar'] or target[@package='foo' or @package='bar']"
        );
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

    // -- Request read: show / log / search.

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

    // -- Request state changes: change_state / accept / decline /
    // revoke / try_change_state (the can_fail adoption: a failed
    // change is swallowed with one warning, a caller bug is not).

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

    // -- Review add.

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

    // -- Comments: list and create.

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

    // -- Request diff.

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
}
