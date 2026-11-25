use super::find_element_tag;
use super::tag_attr;
use crate::debug::Debug;
use crate::error::ErrorKind;
use crate::error::GeectlError;
use clap::ValueEnum;
use std::cell::RefCell;

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
    debug: Debug,
}

impl DryRun {
    /// A dry-run mode with the given setting. `true` = dry-run (do
    /// nothing, print the plan); `false` = live.
    pub fn new(enabled: bool) -> Self {
        DryRun {
            enabled,
            plan: RefCell::new(Vec::new()),
            debug: Debug::disabled(),
        }
    }

    /// Report the channel's decision lines to `debug` instead of
    /// dropping them. The sink is off by default; the caller that
    /// owns the debug channel passes its sink here.
    pub fn with_debug(mut self, debug: Debug) -> Self {
        self.debug = debug;
        self
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
            self.debug.emit(&format!(
                "decision: dry-run: mutation refused, plan only: {action}"
            ));
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

impl std::fmt::Display for RevisionSpec {
    /// Render the specifier the way it goes into a `rev` query
    /// parameter: a number renders as itself, an md5 renders
    /// lowercase (already normalized by [`RevisionSpec::parse`]).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RevisionSpec::Rev(n) => write!(f, "{n}"),
            RevisionSpec::Md5(md5) => write!(f, "{md5}"),
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

/// One entry of a `commitfilelist` file list: the file name, its
/// md5, and the sha256 the server asked for, when it challenged the
/// entry (`hash="sha256:..."` on the resent list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFileEntry {
    /// File name.
    pub name: String,
    /// md5 of the file content.
    pub md5: String,
    /// sha256 hex digest, when the server challenged this entry.
    pub sha256: Option<String>,
}

impl CommitFileEntry {
    /// An entry with a name and md5, no sha256 challenge answered.
    pub fn new(name: &str, md5: &str) -> Self {
        CommitFileEntry {
            name: name.to_string(),
            md5: md5.to_string(),
            sha256: None,
        }
    }
}

/// Options for one `commitfilelist` call: the commit message, the
/// user the commit is recorded as, and the link flags. `validate`
/// sends the `withvalidate=1` first pass; the final pass omits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFilelistOptions {
    /// Commit message (`comment`).
    pub comment: String,
    /// User the commit is recorded as. `None` resolves to the OBS
    /// Basic username on the client.
    pub user: Option<String>,
    /// First (validation) pass: `withvalidate=1`.
    pub validate: bool,
    /// Keep the link (`keeplink=1`): the package stays linked after
    /// the commit.
    pub keeplink: bool,
    /// Link revision to keep (`linkrev`): the expanded srcmd5.
    pub linkrev: Option<String>,
    /// Repair the link (`repairlink=1`).
    pub repairlink: bool,
    /// Skip server-side services after the commit (`noservice=1`).
    pub noservice: bool,
}

impl CommitFilelistOptions {
    /// Options for a commit with this message: validation pass off,
    /// no link flags, services run.
    pub fn new(comment: &str) -> Self {
        CommitFilelistOptions {
            comment: comment.to_string(),
            user: None,
            validate: false,
            keeplink: false,
            linkrev: None,
            repairlink: false,
            noservice: false,
        }
    }
}

/// The parsed answer to one `commitfilelist` call: the new revision
/// (final pass only), the files the server does not have yet, the
/// files it challenged for a sha256, and whether a server-side
/// service run is still in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFilelistResponse {
    /// New revision number, when the call created one.
    pub rev: Option<u64>,
    /// Source md5 of the new revision, when the call created one.
    pub srcmd5: Option<String>,
    /// Files the server does not have yet (`error="missing"`): stage
    /// them with `rev=repository` and resend the list.
    pub missing: Vec<String>,
    /// Files the server challenged for a sha256 (`hash="new"`):
    /// resend the list with `hash="sha256:..."` on these entries.
    pub hash_challenge: Vec<String>,
    /// A server-side service run is still in progress
    /// (`<serviceinfo code="running">`): poll the directory until
    /// it settles before refreshing the base.
    pub service_running: bool,
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
    /// The service run state from `<serviceinfo>`, when the listing
    /// carries one (a run is in progress, failed, or finished).
    pub serviceinfo: Option<ServiceState>,
    /// Source md5 of the listing (`srcmd5` attribute on the
    /// `<directory>` element), when the listing carries one. It pins
    /// the expanded revision the entries belong to: a download at
    /// this srcmd5 reads exactly the files listed, never a newer
    /// revision that landed between the listing and the download.
    pub srcmd5: Option<String>,
}

/// A service run's state, from `<serviceinfo>` in the directory
/// listing (`BSXML.pm` `$serviceinfo`): the run's code, the source
/// md5s, and the failure text, when the server carries them. Read by
/// `ObsClient::service_state`; intended as the settlement read for the
/// commit path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceCode {
    /// A run is in progress.
    Running,
    /// A run finished with an error (see [`ServiceState::error`]).
    Failed,
    /// A run finished successfully.
    Succeeded,
}

impl std::fmt::Display for ServiceCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ServiceCode::Running => "running",
            ServiceCode::Failed => "failed",
            ServiceCode::Succeeded => "succeeded",
        })
    }
}

/// A service run's state, from `<serviceinfo>` in the directory
/// listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceState {
    /// The run's code.
    pub code: ServiceCode,
    /// The expanded source md5, when the listing carries it:
    /// exactly 32 hex digits, validated at parse — a listing
    /// carrying anything else is malformed, never a state.
    pub xsrcmd5: Option<String>,
    /// The linked source md5, when the listing carries it:
    /// exactly 32 hex digits, validated at parse — a listing
    /// carrying anything else is malformed, never a state.
    pub lsrcmd5: Option<String>,
    /// The `<error>` text, on failure. Multi-line; terminal control
    /// characters are stripped at parse, the rest verbatim from the
    /// server.
    pub error: Option<String>,
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
    /// Email address, when the role came from an owner search: the
    /// search returns name and role only, so the email is filled in
    /// from `whois()`. `None` for groups and for roles read from
    /// package meta, which carry no email.
    pub email: Option<String>,
}

/// A role grant or removal: a person and/or a group with a role.
/// At least one of `person` and `group` names someone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleGrant {
    /// User login, when the role is granted to or removed from a person.
    pub person: Option<String>,
    /// Group id, when the role is granted to or removed from a group.
    pub group: Option<String>,
    /// Role: one of the local roles the server grants on projects
    /// and packages.
    pub role: String,
}

impl RoleGrant {
    /// A grant for a person and a group with the same role.
    pub fn new(person: Option<&str>, group: Option<&str>, role: &str) -> Self {
        RoleGrant {
            person: person.map(str::to_string),
            group: group.map(str::to_string),
            role: role.to_string(),
        }
    }

    /// A grant for one person.
    pub fn person(name: &str, role: &str) -> Self {
        RoleGrant::new(Some(name), None, role)
    }

    /// A grant for one group.
    pub fn group(name: &str, role: &str) -> Self {
        RoleGrant::new(None, Some(name), role)
    }
}

/// A download-on-demand source of a repository (`<download>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryDownload {
    /// Architecture the download applies to, when set.
    pub arch: Option<String>,
    /// Download URL.
    pub url: String,
    /// Repository type, when set.
    pub repotype: Option<String>,
    /// Architecture filter, when set.
    pub archfilter: Option<String>,
    /// The download's public key, when set.
    pub pubkey: Option<String>,
    /// The download master, when the download names one.
    pub master: Option<DownloadMaster>,
}

/// The master of a download (`<master>` child of `<download>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadMaster {
    /// Master URL.
    pub url: String,
    /// SSL fingerprint of the master, when set.
    pub sslfingerprint: Option<String>,
}

/// A release target of a repository (`<releasetarget>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseTarget {
    /// Target project.
    pub project: String,
    /// Target repository, when set.
    pub repository: Option<String>,
    /// Release trigger, when set.
    pub trigger: Option<String>,
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
    /// Rebuild mode (`rebuild` attribute), when set.
    pub rebuild: Option<String>,
    /// Block mode (`block` attribute), when set.
    pub block: Option<String>,
    /// Linked-build mode (`linkedbuild` attribute), when set.
    pub linkedbuild: Option<String>,
    /// Download-on-demand sources.
    pub downloads: Vec<RepositoryDownload>,
    /// Release targets.
    pub release_targets: Vec<ReleaseTarget>,
    /// Host systems (`<hostsystem>`).
    pub hostsystem: Vec<(String, String)>,
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

/// The projects whose scmsync URL starts with a prefix, as listed by
/// the id search: the project names on the returned page, the
/// server's total match count, and the page's offset in that total.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScmsyncProjectList {
    /// Project names on this page.
    pub projects: Vec<String>,
    /// The server's total match count.
    pub matches: u32,
    /// The offset of this page in the server's total: the page holds
    /// matches `offset..offset + projects.len()`.
    pub offset: u32,
}

impl ScmsyncProjectList {
    /// Whether more matches exist past this page: the page ends
    /// before the server's total.
    pub fn truncated(&self) -> bool {
        self.offset.saturating_add(self.projects.len() as u32) < self.matches
    }
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

/// The result of [`ObsClient::copy_package_to_instance`]: the target
/// revision when files were uploaded, plus how many files were
/// copied and how many were already identical on the target.
/// `_link` and `_service:` entries are excluded from both counts:
/// they are never copied, so they are neither copied nor skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossInstanceCopyResult {
    /// The target revision after the copy, or None when nothing was
    /// uploaded: in dry-run, or when every file was already
    /// identical on the target.
    pub revision: Option<u64>,
    /// Files downloaded from the source and uploaded to the target.
    pub copied_files: usize,
    /// Files already identical on the target (same name and md5),
    /// skipped without a download.
    pub skipped_files: usize,
}

/// Options for [`ObsClient::delete_project`]. All fields are
/// optional: an unset flag means off, an unset comment means the
/// server's default.
#[derive(Debug, Clone, Default)]
pub struct DeleteProjectOptions {
    /// Delete the project with all its packages. Without this, a
    /// project holding packages is refused — the server deletes a
    /// non-empty project without complaint, so the guard is
    /// client-side.
    pub recursive: bool,
    /// Ignore other projects depending on this one. The server's
    /// `force`.
    pub force: bool,
    /// A comment recorded on the delete. The server's `comment`.
    pub comment: Option<String>,
}

/// Options for [`ObsClient::delete_package`]. All fields are
/// optional: an unset flag means off, an unset message means the
/// server's default.
#[derive(Debug, Clone, Default)]
pub struct DeletePackageOptions {
    /// Ignore open submit requests involving the package. The
    /// server's `force`.
    pub force: bool,
    /// A message recorded on the delete. The server's `comment`.
    pub message: Option<String>,
}

/// Options for [`ObsClient::aggregate_package`]. All fields are
/// optional: an unset flag means off, an empty repository map means
/// no repository mappings.
#[derive(Debug, Clone, Default)]
pub struct AggregateOptions {
    /// Map source repositories to target repositories, as
    /// (source, target) pairs: `<repository target="..." source="..."/>`
    /// entries on the `_aggregate` file.
    pub repository_map: Vec<(String, String)>,
    /// Aggregate binaries only, not the sources. The server's
    /// `<nosources/>`.
    pub no_sources: bool,
    /// Disable publishing of the aggregate package: a
    /// `<publish><disable/></publish>` is added to the target meta.
    pub disable_publish: bool,
}

/// Options for [`ObsClient::release_package`] and
/// [`ObsClient::release_project`]. All fields are optional: an unset
/// value means the server's default.
#[derive(Debug, Clone, Default)]
pub struct ReleaseOptions {
    /// Release only binaries from this repository. The server's
    /// `repository`. Required when `target_project` is set.
    pub repository: Option<String>,
    /// Release only binaries for this architecture. The server's
    /// `arch`.
    pub architecture: Option<String>,
    /// Release only to this project. The server's `target_project`.
    /// Needs `repository` and `target_repository`.
    pub target_project: Option<String>,
    /// Release only to this repository in the target project. The
    /// server's `target_repository`. Needs `target_project`.
    pub target_repository: Option<String>,
    /// Rename binaries during the release using this release tag.
    /// The server's `setrelease`.
    pub set_release: Option<String>,
    /// Run the release immediately instead of queueing it as a job.
    /// The server's `nodelay`.
    pub no_delay: bool,
}

/// The operation a trigger token authorises: the server's
/// `operation` on `POST /person/<login>/token`. The API accepts
/// only these four; any other operation is refused by the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TokenOperation {
    /// Run source services: the `runservice` trigger.
    Runservice,
    /// Rebuild packages: the `rebuild` trigger.
    Rebuild,
    /// Release packages: the `release` trigger.
    Release,
    /// Run SCM/CI workflows: the `workflow` trigger.
    Workflow,
}

impl Default for TokenOperation {
    /// The default exists only so `CreateTokenOptions` can derive
    /// `Default` for struct-update syntax; `create_token` always
    /// sends an operation, so the server's default for a missing
    /// operation is never exercised.
    fn default() -> Self {
        TokenOperation::Runservice
    }
}

impl TokenOperation {
    /// The wire spelling: the server's `operation` value.
    pub fn as_str(self) -> &'static str {
        match self {
            TokenOperation::Runservice => "runservice",
            TokenOperation::Rebuild => "rebuild",
            TokenOperation::Release => "release",
            TokenOperation::Workflow => "workflow",
        }
    }
}

/// Options for [`ObsClient::create_token`]. The project and the
/// package go together: both scope the token to a package, neither
/// leaves it global.
#[derive(Debug, Clone, Default)]
pub struct CreateTokenOptions {
    /// The operation the token authorises. The server's
    /// `operation`.
    pub operation: TokenOperation,
    /// The project of the package the token is scoped to. The
    /// server's `project`; sent only with a package.
    pub project: Option<String>,
    /// The package the token is scoped to. The server's `package`.
    pub package: Option<String>,
    /// A description identifying the token, at most 64 characters.
    /// The server's `description`.
    pub description: Option<String>,
}

/// A token just created by [`ObsClient::create_token`]: the id and
/// the secret, which the server returns once, at creation — it is
/// never shown again.
#[derive(Clone, PartialEq, Eq)]
pub struct CreatedToken {
    /// The token's id, for listing and deleting it.
    pub id: u64,
    /// The token secret. Shown once, at creation.
    pub token: String,
    /// The operation the token authorises.
    pub operation: TokenOperation,
    /// The project of the package the token is scoped to, if any.
    pub project: Option<String>,
    /// The package the token is scoped to, if any.
    pub package: Option<String>,
    /// The description identifying the token, if any.
    pub description: Option<String>,
}

impl std::fmt::Debug for CreatedToken {
    /// The secret is never shown in debug output: it is printed
    /// once, at creation, and a `{created:?}` anywhere else must
    /// not leak it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreatedToken")
            .field("id", &self.id)
            .field("token", &"[redacted]")
            .field("operation", &self.operation)
            .field("project", &self.project)
            .field("package", &self.package)
            .field("description", &self.description)
            .finish()
    }
}

/// The status filter for [`ObsClient::wipe_binaries`]: wipe only
/// the binaries of packages in this build state. The server's
/// `code` parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WipeCode {
    /// Packages whose build is disabled. The server's
    /// `code=disabled`.
    Disabled,
    /// Packages whose build failed. The server's `code=failed`.
    Failed,
    /// Packages whose sources are bad. The server's `code=broken`.
    Broken,
    /// Packages with dependency errors. The server's
    /// `code=unresolvable`.
    Unresolvable,
}

impl WipeCode {
    /// The wire spelling: the server's `code` value.
    pub fn as_str(self) -> &'static str {
        match self {
            WipeCode::Disabled => "disabled",
            WipeCode::Failed => "failed",
            WipeCode::Broken => "broken",
            WipeCode::Unresolvable => "unresolvable",
        }
    }
}

/// Options for [`ObsClient::wipe_binaries`]. At least one scope
/// selector is required: `all`, `arch`, `repository`, or a
/// non-empty `codes` — an unscoped wipe would delete every binary
/// in the project, so osc refuses it too.
#[derive(Debug, Clone, Default)]
pub struct WipeBinariesOptions {
    /// The package to wipe. None wipes the whole project. The
    /// server's `package`.
    pub package: Option<String>,
    /// Wipe only binaries for this architecture. The server's
    /// `arch`.
    pub arch: Option<String>,
    /// Wipe only binaries for this repository. The server's
    /// `repository`.
    pub repository: Option<String>,
    /// Wipe all binaries regardless of package status. The
    /// unfiltered request: no `code` is sent. Combines with no
    /// status filter.
    pub all: bool,
    /// Wipe only binaries of packages in these build states: one
    /// request per code, in order. The server's `code` parameter.
    pub codes: Vec<WipeCode>,
}

/// Options for [`ObsClient::add_channels`]. Both fields are off
/// by default: the channels are added disabled, as the server's
/// default `add_disabled` mode.
#[derive(Debug, Clone, Default)]
pub struct AddChannelsOptions {
    /// Enable all added channels, including the ones disabled by
    /// default. The server's `mode=enable_all`. Mutually exclusive
    /// with `skip_disabled`.
    pub enable_all: bool,
    /// Skip disabled channels: add the channel packages but not
    /// their repositories. The server's `mode=skip_disabled`.
    /// Mutually exclusive with `enable_all`.
    pub skip_disabled: bool,
}

/// Options for [`ObsClient::add_containers`].
#[derive(Debug, Clone, Default)]
pub struct AddContainersOptions {
    /// Extend the package names with the container names when
    /// adding. The server's `extend_package_names=1`.
    pub extend_package_names: bool,
}

/// Options for [`ObsClient::create_package_with_options`]. All
/// fields are optional: no scmsync means an OBS-native package, no
/// force means an existing package is refused rather than
/// overwritten.
#[derive(Debug, Clone, Default)]
pub struct CreatePackageOptions {
    /// The git repository and branch the package syncs its sources
    /// from, as `url#branch`. The server's `<scmsync>`. The fragment
    /// (the branch name) is required.
    pub scmsync: Option<String>,
    /// Overwrite the package when it already exists. Without force
    /// an existing package is refused.
    pub force: bool,
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

/// Which revision a link is set to by [`ObsClient::set_link_rev`].
/// The revision is the link target's revision the link expands at:
/// a number, an md5, or nothing (the link follows the target).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkRevision {
    /// The link target's latest revision (its directory `rev`).
    Latest,
    /// Exactly this revision: a number or an md5. Set without
    /// fetching anything — the revision is taken as given.
    Pinned(String),
    /// The link target's latest revision, expanded: the directory
    /// `rev` at `expand=1` (the expanded srcmd5) with its `vrev`.
    Expanded,
    /// This revision, expanded: the directory `rev` at the pinned
    /// revision with `expand=1` (the expanded srcmd5) with its
    /// `vrev`.
    PinnedExpanded(String),
    /// No revision: the link follows its target. The `rev` and the
    /// `vrev` are both stripped from the link.
    Unset,
}

/// How a dedup search narrows the returned actions beyond their type,
/// checked client-side in [`parse_request_ids_for_action`]. The search
/// match cannot express these: a delete search cannot match "no target
/// package" (the whole-project form), so a package filing's search
/// would never see an open whole-project delete; and the server's
/// incident identity includes the release project
/// (`BsRequestActionMaintenanceIncident#uniq_key`), which the search
/// does not match on. The forward supersede search narrows the same
/// way, on the hop identity — source plus target in one action.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ActionShape<'a> {
    /// Any action of the type counts.
    Any,
    /// Only actions whose target release project is `release_project`;
    /// when it is `None`, any stored release project matches — the
    /// server stores the filing-time-resolved value even when the
    /// filed action named none, so a filing without a release project
    /// cannot tell stored values apart. That over-blocks the narrow
    /// cases where the link-derived release project differs (two
    /// branches linked at different release projects, or a patchinfo
    /// incident against a non-patchinfo filing); resolving it
    /// client-side would need a link read the library has no
    /// operation for.
    ReleaseProject(Option<&'a str>),
    /// Only delete actions whose target shape conflicts with the
    /// filing target: a whole-project open delete blocks any delete in
    /// the project; a package open delete blocks the same package; a
    /// repository open delete blocks the same repository and a
    /// whole-project filing.
    DeleteTarget {
        package: Option<&'a str>,
        repository: Option<&'a str>,
    },
    /// Only submit actions for one forward hop: the action's
    /// `<source>` names `source_project`/`source_package` and its
    /// `<target>` names `target_project`/`target_package`, in the
    /// same action. An action whose source or target cannot be read
    /// does not count: superseding a request that cannot be verified
    /// as the same hop is the overbroad supersede this shape exists
    /// to prevent, so an unverifiable action slips past instead of
    /// blocking fail-closed.
    SubmitHop {
        source_project: &'a str,
        source_package: &'a str,
        target_project: &'a str,
        target_package: &'a str,
    },
}

impl<'a> ActionShape<'a> {
    /// True when an action whose inner XML is `inner` (None when the
    /// action tag is self-closing, so it has no target) counts for
    /// this shape. `ReleaseProject(None)` counts every action of the
    /// type: the server stores the filing-time-resolved release
    /// project even when the filed action named none, so a filing
    /// without a release project cannot tell stored values apart. A
    /// target-less action blocks for every other shape: for
    /// `DeleteTarget` it reads as carrying nothing, and for
    /// `ReleaseProject(Some(_))` a missing target is malformed, so it
    /// blocks fail-closed rather than slipping past the dedup. For
    /// `SubmitHop` the direction is the other way: an action that
    /// cannot be verified as the hop never counts, because counting
    /// it would supersede a request that is not the same hop.
    pub(crate) fn matches_action(&self, inner: Option<&str>) -> bool {
        match self {
            ActionShape::Any => true,
            ActionShape::ReleaseProject(expected) => {
                // No release project expected: any stored value
                // matches — the server stores the filing-time-resolved
                // value even when the filed action named none.
                if expected.is_none() {
                    return true;
                }
                let target = inner.and_then(|inner| find_element_tag(inner, "target"));
                match target {
                    // No target element at all: malformed — block,
                    // fail-closed, rather than slipping past the dedup.
                    None => true,
                    Some(tag) => tag_attr(tag, "releaseproject").as_deref() == *expected,
                }
            }
            ActionShape::DeleteTarget {
                package,
                repository,
            } => {
                let target = inner.and_then(|inner| find_element_tag(inner, "target"));
                let open_package = target.and_then(|tag| tag_attr(tag, "package"));
                let open_repository = target.and_then(|tag| tag_attr(tag, "repository"));
                // A whole-project open delete (neither set) blocks any
                // delete; a package/repository open delete blocks only
                // the same target — and a repository open delete also
                // blocks a whole-project filing, whose target subsumes
                // it (the server's delete identity carries no
                // repository, so the two share the key).
                let open_whole_project = open_package.is_none() && open_repository.is_none();
                let same_package = open_package.is_some() && open_package.as_deref() == *package;
                let same_repository =
                    open_repository.is_some() && open_repository.as_deref() == *repository;
                let repo_blocks_whole_project =
                    open_repository.is_some() && package.is_none() && repository.is_none();
                open_whole_project || same_package || same_repository || repo_blocks_whole_project
            }
            ActionShape::SubmitHop {
                source_project,
                source_package,
                target_project,
                target_package,
            } => {
                let source = inner.and_then(|inner| find_element_tag(inner, "source"));
                let target = inner.and_then(|inner| find_element_tag(inner, "target"));
                let source_matches = source.is_some_and(|tag| {
                    tag_attr(tag, "project").as_deref() == Some(source_project)
                        && tag_attr(tag, "package").as_deref() == Some(source_package)
                });
                let target_matches = target.is_some_and(|tag| {
                    tag_attr(tag, "project").as_deref() == Some(target_project)
                        && tag_attr(tag, "package").as_deref() == Some(target_package)
                });
                source_matches && target_matches
            }
        }
    }
}

/// What a dedup check matches on: the action type plus the
/// source/target names of the action. Only the parts present are
/// matched — a delete request has no source, a whole-project delete
/// has neither a target package nor a repository, and a maintenance
/// incident target has no package. The `shape` narrows the returned
/// actions client-side where the search cannot. The struct keeps the
/// search and the filing-dedup helper from growing a parameter per part.
pub(crate) struct ActionTarget<'a> {
    pub(crate) action_type: &'a str,
    pub(crate) source_project: Option<&'a str>,
    pub(crate) source_package: Option<&'a str>,
    pub(crate) target_project: &'a str,
    pub(crate) target_package: Option<&'a str>,
    pub(crate) target_repository: Option<&'a str>,
    pub(crate) shape: ActionShape<'a>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug::Debug;

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

    #[test]
    fn revision_spec_renders_as_the_rev_query_value() {
        assert_eq!(RevisionSpec::Rev(2).to_string(), "2");
        assert_eq!(
            RevisionSpec::Md5("0123456789abcdef0123456789abcdef".to_string()).to_string(),
            "0123456789abcdef0123456789abcdef"
        );
        assert_eq!(
            RevisionSpec::parse("AABBCCDDAABBCCDDAABBCCDDAABBCCDD")
                .unwrap()
                .to_string(),
            "aabbccddaabbccddaabbccddaabbccdd"
        );
    }

    #[test]
    fn debug_dry_run_refusal_reports_the_guard() {
        let (debug, lines) = Debug::captured(true, false);
        let dry = DryRun::new(true).with_debug(debug);
        assert!(!dry.allow("file submit request for foo to devel:languages:python"));
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            ["decision: dry-run: mutation refused, plan only: file submit request for foo to devel:languages:python"
                .to_string()]
        );
        // The [dry-run] plan echo is unchanged: it is unconditional
        // narration, not a debug line.
        assert_eq!(
            dry.plan(),
            [
                "[dry-run] would: file submit request for foo to devel:languages:python"
                    .to_string()
            ]
        );
    }

    #[test]
    fn debug_dry_run_refusal_reports_nothing_when_debug_is_off() {
        let (debug, lines) = Debug::captured(false, false);
        let dry = DryRun::new(true).with_debug(debug);
        assert!(!dry.allow("file submit request for foo to devel:languages:python"));
        assert!(lines.lock().unwrap().is_empty());
        assert_eq!(dry.plan().len(), 1);
    }
}
