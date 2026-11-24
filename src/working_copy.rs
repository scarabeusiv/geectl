//! The git working copy: checkout, update, status, diff, add, rm,
//! addremove, commit, pull, and resolve over one OBS package or one
//! git-backed package.
//!
//! The working copy is a git repository. An OBS-native package gets a
//! synthesized repository: the base revision is the root commit, and
//! the commit message carries the base coordinates (`geectl base
//! <rev> <srcmd5>`). A git-backed package (its `_meta` carries a
//! scmsync URL) gets a real clone of that repository. Either way the
//! agent edits files, stages with `add`, and commits with `commit`:
//! the OBS-native commit sends the index state against the base
//! through the `commitfilelist` protocol, the git-backed commit
//! pushes the local commits and polls the OBS side until the sync
//! picks the pushed head up.
//!
//! The checkout record lives in the repository's own git config
//! (`geectl.*`): the API URL, the project, the package, the backend
//! kind, and the base revision and source md5. The record is the
//! checkout: a directory without it is not a working copy.
//!
//! Git runs through the `git` binary, argv-only, never through a
//! shell. The binary is the reference rebase implementation; a
//! native git binding would add a C build to geectl and to every
//! consumer that pins it as a library. Synthesized commits use the
//! repo-local identity `geectl` / `geectl@localhost`, never a real
//! person's identity.

use crate::config::Config;
use crate::error::{ErrorKind, GeectlError};
use crate::local_build::LocalBuild;
use crate::obs::{
    valid_obs_filename, CommitFileEntry, CommitFilelistOptions, DirectoryListing, DryRun,
    ObsClient, RevisionSpec, ServiceCode,
};
use crate::xml::XmlDoc;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The git binary's fixed timeout: generous, since a clone or a
/// fetch of a large repository takes a while and a local operation
/// never should.
const GIT_TIMEOUT: Duration = Duration::from_secs(300);

/// The timeout for network git operations (clone, fetch, push),
/// in seconds, from `GEectl_GIT_NETWORK_TIMEOUT_SECS`. A clone or
/// a fetch of a large repository takes a while; a single fixed
/// value cannot fit every network, so the network budget is
/// configurable while local operations keep the fixed timeout.
/// Invalid, zero, or unset values fall back to the default.
const DEFAULT_GIT_NETWORK_TIMEOUT: Duration = Duration::from_secs(300);

/// Parse the network git timeout from `GEectl_GIT_NETWORK_TIMEOUT_SECS`:
/// invalid, zero, or unset values fall back to the default. Split
/// out of `git_network_timeout` so the parsing is testable: the
/// cached value cannot be re-read in-process.
fn parse_git_network_timeout(value: Option<String>) -> Duration {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_GIT_NETWORK_TIMEOUT)
}

fn git_network_timeout() -> Duration {
    static TIMEOUT: OnceLock<Duration> = OnceLock::new();
    *TIMEOUT.get_or_init(|| {
        parse_git_network_timeout(std::env::var("GEectl_GIT_NETWORK_TIMEOUT_SECS").ok())
    })
}

/// The precommit gate's budget: source services plus
/// `pre_checkin.sh` before a commit.
const PRECOMMIT_TIMEOUT_SECS: u64 = 3600;

/// How long the git-backed commit poll and the service-settle poll
/// wait for the server side to catch up.
const POLL_TIMEOUT: Duration = Duration::from_secs(60);

/// How long each poll waits between reads.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// The git config keys that make up the checkout record.
const CONFIG_KEYS: [&str; 6] = [
    "apiurl",
    "project",
    "package",
    "backend",
    "base-rev",
    "base-srcmd5",
];

/// Which backend a working copy talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// An OBS-native package: the base is a synthesized root commit.
    Obs,
    /// A git-backed package: the base is the pushed head of the
    /// cloned repository.
    Git,
}

impl Backend {
    /// The config value for this backend.
    fn as_str(self) -> &'static str {
        match self {
            Backend::Obs => "obs",
            Backend::Git => "git",
        }
    }

    /// Parse a config value. Anything else is not a backend.
    fn parse(value: &str) -> Option<Backend> {
        match value {
            "obs" => Some(Backend::Obs),
            "git" => Some(Backend::Git),
            _ => None,
        }
    }
}

/// One checked-out package: its root, its coordinates, and its base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingCopy {
    /// The checkout root (`<dir>/<project>/<package>/`).
    pub root: PathBuf,
    /// The OBS project.
    pub project: String,
    /// The OBS package.
    pub package: String,
    /// The OBS API URL the checkout came from.
    pub apiurl: String,
    /// The backend kind.
    pub backend: Backend,
    /// The base revision: the OBS revision number, or the pushed
    /// head SHA for a git-backed checkout.
    pub base_rev: String,
    /// The base source md5. Empty for a git-backed checkout.
    pub base_srcmd5: String,
}

impl WorkingCopy {
    /// The git commit the base is: the synthesized root commit for
    /// an OBS-native checkout, the recorded head SHA for a
    /// git-backed one.
    pub fn base_commit(&self) -> Result<String, GeectlError> {
        match self.backend {
            Backend::Obs => root_commit(&self.root),
            Backend::Git => Ok(self.base_rev.clone()),
        }
    }
}

/// One file's state against the base, in `git status --porcelain`
/// vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StatusCode {
    /// Staged as new.
    Added,
    /// Changed against the base.
    Modified,
    /// Staged as deleted.
    Deleted,
    /// Not tracked.
    Unknown,
    /// Tracked, but missing from the worktree.
    Missing,
    /// Unmerged: a rebase or merge left the file conflicted.
    Conflicted,
    /// Staged as a rename.
    Renamed,
}

/// One file in a `status` listing: its path and its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    /// The path, relative to the checkout root.
    pub path: String,
    /// The state against the base.
    pub code: StatusCode,
}

/// The result of a `commit`: the new revision for an OBS-native
/// commit, the pushed head for a git-backed one, the files sent,
/// and the open requests the commit landed under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitReport {
    /// The new OBS revision number, for an OBS-native commit.
    pub rev: Option<u64>,
    /// The new source md5, for an OBS-native commit.
    pub srcmd5: Option<String>,
    /// The pushed head SHA, for a git-backed commit.
    pub pushed_head: Option<String>,
    /// The open requests involving the package after the commit, as
    /// request ids. Empty when the search finds none.
    pub open_requests: Vec<String>,
    /// The files the commit sent (deletions included).
    pub files: Vec<String>,
}

/// The result of an `update`: whether the base moved, the old and
/// new base coordinates, and the conflicts a stopped rebase left in
/// the tree (empty when the rebase finished).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateReport {
    /// The base moved and the rebase finished.
    pub updated: bool,
    /// The base revision before the update.
    pub old_base_rev: String,
    /// The base revision after the update — the new base when the
    /// update completed, the rebase target while a conflicted
    /// update is stopped.
    pub new_base_rev: String,
    /// The base source md5 before the update.
    pub old_base_srcmd5: String,
    /// The base source md5 after the update — the target's md5
    /// while a conflicted update is stopped (unchanged for a
    /// git-backed checkout, which records no srcmd5).
    pub new_base_srcmd5: String,
    /// The files a stopped rebase left conflicted, empty when the
    /// rebase finished. Non-empty means the rebase is still in
    /// progress: resolve the files, then run `resolve`.
    pub conflicts: Vec<String>,
    /// A stash entry from the update is still there: the worktree
    /// changes the rebase stashed could not be re-applied (they
    /// conflict with the new base). `resolve` restores them after
    /// the rebase continues; while set, the entry stays in the
    /// stash under the message `geectl autostash`.
    pub stash_remaining: bool,
}

/// The result of a `pull`: whether the link moved, the old and new
/// link revisions, and the conflicts a stopped rebase left in the
/// tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullReport {
    /// The link moved and the rebase finished.
    pub updated: bool,
    /// The link target's source md5 before the pull.
    pub old_baserev: String,
    /// The link target's source md5 after the pull.
    pub new_baserev: String,
    /// The base revision before the pull.
    pub old_base_rev: String,
    /// The base revision after the pull — the new base when the
    /// pull completed, the rebase target while a conflicted pull
    /// is stopped.
    pub new_base_rev: String,
    /// The files a stopped rebase left conflicted, empty when the
    /// rebase finished.
    pub conflicts: Vec<String>,
    /// A stash entry from the pull is still there: the worktree
    /// changes the rebase stashed could not be re-applied. See
    /// [`UpdateReport::stash_remaining`].
    pub stash_remaining: bool,
}

/// The outcome of one `git` run: whether it succeeded, and the
/// captured stdout and stderr.
struct GitOutput {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Environment variables scrubbed from every `git` call: a
/// caller's `GIT_DIR`, `GIT_WORK_TREE`, or `GIT_INDEX_FILE` would
/// redirect the call at another repository (or at the update
/// tree's temporary index), a `GIT_CONFIG_*` would inject config,
/// and `GIT_SSH_COMMAND`/`GIT_ASKPASS` would hand the transport to
/// the caller's helpers. The identity variables ARE scrubbed, so
/// the author/committer identity comes from the checkout's own
/// repo-local config, never the caller's environment. The date
/// variables are NOT scrubbed: the author/committer dates pin
/// deterministic synthesized commits. `extra_env` is applied after
/// the scrub, so the call's own values (the token header, the
/// temporary index) win.
const GIT_ENV_SCRUB: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_SYSTEM",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_NOSYSTEM",
    "GIT_CEILING_DIRECTORIES",
    "GIT_DISCOVERY_ACROSS_FILESYSTEM",
    "GIT_AUTHOR_NAME",
    "GIT_AUTHOR_EMAIL",
    "GIT_COMMITTER_NAME",
    "GIT_COMMITTER_EMAIL",
    "GIT_ASKPASS",
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "GIT_ALLOW_PROTOCOL",
    "GIT_PROTOCOL_FROM_USER",
    "GIT_LITERAL_PATHSPECS",
    "GIT_QUARANTINE_PATH",
    "GIT_EXEC_PATH",
    "GIT_TEMPLATE_DIR",
    "GIT_DEFAULT_HASH",
    "GIT_DEFAULT_BRANCH",
];

/// Build the `git` command: the fixed `-c` flags, the scrubbed
/// environment, and the fixed environment, before the subcommand.
/// No signing and no line-ending conversion: the working copy
/// holds the server bytes exactly on every platform. No hooks:
/// user hooks must not run on synthesized commits, rebases, or
/// pushes. No color: parsed output (`stash list` and friends)
/// must never carry ANSI escapes, whatever `color.ui` the user
/// set. `globals` are `git`-level options placed before the
/// subcommand (`--literal-pathspecs`); `extra_env` is applied
/// after the scrub.
fn git_command(
    dir: &Path,
    globals: &[&str],
    subcommand: &str,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-c")
        .arg("commit.gpgsign=false")
        .arg("-c")
        .arg("core.autocrlf=false")
        .arg("-c")
        .arg("protocol.ext.allow=never")
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .arg("-c")
        .arg("color.ui=false")
        .args(globals)
        .arg(subcommand)
        .args(args);
    cmd.current_dir(dir);
    for key in GIT_ENV_SCRUB {
        cmd.env_remove(key);
    }
    for n in 0..16 {
        cmd.env_remove(format!("GIT_CONFIG_KEY_{n}"));
        cmd.env_remove(format!("GIT_CONFIG_VALUE_{n}"));
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GIT_EDITOR", "true");
    cmd.env("GIT_SEQUENCE_EDITOR", "true");
    cmd.env("LC_ALL", "C");
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    cmd
}

/// Run the `git` binary in `dir` with `subcommand` and `args`,
/// argv-only, with a fixed environment and a fixed timeout. The
/// error carries the subcommand and the bounded stderr, never the
/// full argv: argv can carry a token (the clone header passes
/// through the environment, not argv, for the same reason).
fn run_git(
    dir: &Path,
    subcommand: &str,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> Result<GitOutput, GeectlError> {
    run_git_inner(dir, &[], subcommand, args, extra_env, GIT_TIMEOUT)
}

/// Run `git` with the network timeout (clone, fetch, push) and
/// `globals` before the subcommand.
fn run_git_inner(
    dir: &Path,
    globals: &[&str],
    subcommand: &str,
    args: &[&str],
    extra_env: &[(&str, &str)],
    timeout: Duration,
) -> Result<GitOutput, GeectlError> {
    if !dir.is_dir() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("git working directory does not exist: {}", dir.display()),
        ));
    }
    let mut cmd = git_command(dir, globals, subcommand, args, extra_env);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(GeectlError::new(
                ErrorKind::NoGit,
                "git is not installed or not on PATH",
            ));
        }
        Err(e) => {
            return Err(GeectlError::new(
                ErrorKind::InternalError,
                format!("failed to run git {subcommand}: {e}"),
            ));
        }
    };
    // The pipes are drained on reader threads while the main thread
    // polls: a child that fills a pipe with output (a `git show` of
    // a large file) must not block forever waiting for a reader.
    let stdout_thread = child.stdout.take().map(|mut out| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = out.read_to_end(&mut buf);
            buf
        })
    });
    let stderr_thread = child.stderr.take().map(|mut err| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = err.read_to_end(&mut buf);
            buf
        })
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = stdout_thread
                    .map(|thread| thread.join().unwrap_or_default())
                    .unwrap_or_default();
                let stderr = stderr_thread
                    .map(|thread| thread.join().unwrap_or_default())
                    .unwrap_or_default();
                return Ok(GitOutput {
                    success: status.success(),
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(GeectlError::new(
                        ErrorKind::Timeout,
                        format!(
                            "git {subcommand} timed out after {} seconds",
                            timeout.as_secs()
                        ),
                    ));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => {
                return Err(GeectlError::new(
                    ErrorKind::InternalError,
                    format!("failed to wait for git {subcommand}: {e}"),
                ));
            }
        }
    }
}

/// The bounded stderr of a failed `git` run, as the error text.
/// A bounded stderr excerpt, for error messages built from a
/// command's raw stderr instead of `git_error`.
fn bounded(text: &str) -> String {
    text.chars().take(2048).collect()
}

/// The error kind for a failed `git` run, from its stderr: an
/// auth failure, a network failure, a non-fast-forward, an
/// overwrite a rebase would do, or a missing repository are
/// distinct, actionable states — not an internal error. Anything
/// else stays `InternalError`: the failure is the tool's, not the
/// caller's or the network's. The HTTP codes match only as tokens
/// (`http 401`, `error: 401`, `401 unauthorized`): a bare `401` /
/// `403` digit run also matches a SHA, a file name, or a port.
fn classify_git_failure(stderr: &str) -> ErrorKind {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("not a git repository") {
        ErrorKind::WorkingCopyNotFound
    } else if lower.contains("authentication failed")
        || lower.contains("permission denied (publickey)")
        || lower.contains("permission denied, please try again")
        || lower.contains("could not read username")
        || lower.contains("http 401")
        || lower.contains("http 403")
        || lower.contains("status 401")
        || lower.contains("status 403")
        || lower.contains("error: 401")
        || lower.contains("error: 403")
        || lower.contains("code 401")
        || lower.contains("code 403")
        || lower.contains("401 unauthorized")
        || lower.contains("403 forbidden")
    {
        ErrorKind::AuthFailed
    } else if lower.contains("could not resolve host")
        || lower.contains("connection refused")
        || lower.contains("failed to connect")
        || lower.contains("network is unreachable")
        || lower.contains("connection reset")
        || lower.contains("connection timed out")
        || lower.contains("unable to connect")
    {
        ErrorKind::NetworkError
    } else if lower.contains("non-fast-forward")
        || lower.contains("fetch first")
        || lower.contains("updates were rejected because the remote contains work")
    {
        ErrorKind::WorkingCopyOutdated
    } else if lower.contains("untracked working tree files would be overwritten") {
        ErrorKind::FileConflict
    } else {
        ErrorKind::InternalError
    }
}

fn git_error(subcommand: &str, stderr: &[u8]) -> GeectlError {
    let text = String::from_utf8_lossy(stderr);
    let bounded: String = text.chars().take(2048).collect();
    let message = if bounded.trim().is_empty() {
        format!("git {subcommand} failed")
    } else {
        format!("git {subcommand} failed: {}", bounded.trim())
    };
    GeectlError::new(classify_git_failure(&text), message)
}

/// Run `git` and return its stdout as text. A non-zero exit is an
/// error carrying the subcommand and the bounded stderr.
fn git(
    dir: &Path,
    subcommand: &str,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> Result<String, GeectlError> {
    let output = run_git(dir, subcommand, args, extra_env)?;
    if output.success {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(git_error(subcommand, &output.stderr))
    }
}

/// Run `git` and return its stdout as bytes. A non-zero exit is an
/// error carrying the subcommand and the bounded stderr.
fn git_bytes(
    dir: &Path,
    subcommand: &str,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> Result<Vec<u8>, GeectlError> {
    let output = run_git(dir, subcommand, args, extra_env)?;
    if output.success {
        Ok(output.stdout)
    } else {
        Err(git_error(subcommand, &output.stderr))
    }
}

/// Run `git` for a network operation (clone, fetch, push) with the
/// configurable network timeout. A non-zero exit is an error
/// carrying the subcommand and the bounded stderr.
fn git_network(
    dir: &Path,
    subcommand: &str,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> Result<String, GeectlError> {
    let output = run_git_inner(dir, &[], subcommand, args, extra_env, git_network_timeout())?;
    if output.success {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(git_error(subcommand, &output.stderr))
    }
}

/// Run `git` with `--literal-pathspecs` before the subcommand:
/// user-supplied paths are literal paths, never globs
/// (`rm 'file[1].txt'` must not also remove `file1.txt`). The
/// option is a `git`-level option, so it goes before the
/// subcommand — after it, git rejects it as unknown.
fn git_literal(
    dir: &Path,
    subcommand: &str,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> Result<String, GeectlError> {
    let output = run_git_inner(
        dir,
        &["--literal-pathspecs"],
        subcommand,
        args,
        extra_env,
        GIT_TIMEOUT,
    )?;
    if output.success {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(git_error(subcommand, &output.stderr))
    }
}

/// The md5 of a file, streamed in 64 KiB chunks: a large file
/// never sits in memory whole.
fn md5_of_file(path: &Path) -> Result<String, GeectlError> {
    let file = std::fs::File::open(path).map_err(|e| {
        GeectlError::new(
            ErrorKind::InternalError,
            format!("failed to read {}: {e}", path.display()),
        )
    })?;
    let mut reader = BufReader::new(file);
    let mut context = md5::Context::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = reader.read(&mut buf).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("failed to read {}: {e}", path.display()),
            )
        })?;
        if n == 0 {
            break;
        }
        context.consume(&buf[..n]);
    }
    Ok(format!("{:x}", context.compute()))
}

/// The sha256 of a worktree file, streamed: `None` when the file
/// is missing, a symlink, or not a regular file. A tracked
/// symlink to `/dev/zero` or a FIFO must not hang the gate or
/// exhaust memory: symlinks are never followed here, and a FIFO
/// is never opened.
fn hash_worktree_file(root: &Path, path: &str) -> Option<String> {
    let full = root.join(path);
    let meta = std::fs::symlink_metadata(&full).ok()?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return None;
    }
    let file = std::fs::File::open(&full).ok()?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = reader.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(format!("{:x}", hasher.finalize()))
}

/// One `git cat-file --batch` process serving blob md5s for a
/// whole operation: the base and index blobs are hashed through a
/// single spawn, streamed, instead of one `git show` spawn per
/// file (each of which re-resolved the base commit and buffered
/// the whole blob).
struct BlobBatch {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl BlobBatch {
    fn new(dir: &Path) -> Result<Self, GeectlError> {
        let mut cmd = git_command(dir, &[], "cat-file", &["--batch"], &[]);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("failed to run git cat-file: {e}"),
            )
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            GeectlError::new(ErrorKind::InternalError, "git cat-file has no stdin")
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            GeectlError::new(ErrorKind::InternalError, "git cat-file has no stdout")
        })?;
        Ok(BlobBatch {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    /// The md5 of the blob `sha`, streamed through the batch
    /// process: the blob content is hashed in chunks and never
    /// buffered.
    fn md5(&mut self, sha: &str) -> Result<String, GeectlError> {
        writeln!(self.stdin, "{sha}").map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("failed to read blob {sha} from git cat-file: {e}"),
            )
        })?;
        self.stdin.flush().map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("failed to read blob {sha} from git cat-file: {e}"),
            )
        })?;
        let mut header = String::new();
        self.stdout.read_line(&mut header).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("failed to read blob {sha} from git cat-file: {e}"),
            )
        })?;
        let mut parts = header.split_whitespace();
        match (parts.next(), parts.next(), parts.next()) {
            (Some(found), Some(kind), Some(size)) if found == sha && kind == "blob" => {
                let size: u64 = size.parse().map_err(|_| {
                    GeectlError::new(
                        ErrorKind::InternalError,
                        format!("git cat-file returned a malformed header for blob {sha}"),
                    )
                })?;
                let mut context = md5::Context::new();
                let mut remaining = size;
                let mut buf = [0u8; 65536];
                while remaining > 0 {
                    let want = remaining.min(buf.len() as u64) as usize;
                    self.stdout.read_exact(&mut buf[..want]).map_err(|e| {
                        GeectlError::new(
                            ErrorKind::InternalError,
                            format!("failed to read blob {sha} from git cat-file: {e}"),
                        )
                    })?;
                    context.consume(&buf[..want]);
                    remaining -= want as u64;
                }
                // The batch protocol ends each blob with one LF:
                // anything else means the stream desynced, and the
                // next read would take a corrupt header for an
                // answer.
                let mut lf = [0u8; 1];
                self.stdout.read_exact(&mut lf).map_err(|e| {
                    GeectlError::new(
                        ErrorKind::InternalError,
                        format!("failed to read blob {sha} from git cat-file: {e}"),
                    )
                })?;
                if lf != *b"\n" {
                    return Err(GeectlError::new(
                        ErrorKind::InternalError,
                        format!("git cat-file returned a malformed blob trailer for {sha}"),
                    ));
                }
                Ok(format!("{:x}", context.compute()))
            }
            _ => Err(GeectlError::new(
                ErrorKind::InternalError,
                format!("git cat-file has no blob {sha}"),
            )),
        }
    }
}

impl Drop for BlobBatch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Stream the blob `sha` into `dest` without buffering it:
/// `git cat-file` piped straight to the file. The reuse path for
/// an update file whose listing md5 matches the base blob.
fn git_blob_to_file(dir: &Path, sha: &str, dest: &Path) -> Result<(), GeectlError> {
    let mut cmd = git_command(dir, &[], "cat-file", &["blob", sha], &[]);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        GeectlError::new(
            ErrorKind::InternalError,
            format!("failed to run git cat-file: {e}"),
        )
    })?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| GeectlError::new(ErrorKind::InternalError, "git cat-file has no stdout"))?;
    let mut file = std::fs::File::create(dest).map_err(|e| {
        GeectlError::new(
            ErrorKind::InternalError,
            format!("failed to create {}: {e}", dest.display()),
        )
    })?;
    let copied = std::io::copy(&mut stdout, &mut file);
    let status = child.wait().map_err(|e| {
        GeectlError::new(
            ErrorKind::InternalError,
            format!("failed to wait for git cat-file: {e}"),
        )
    })?;
    match (copied, status.success()) {
        (Ok(_), true) => Ok(()),
        _ => Err(GeectlError::new(
            ErrorKind::InternalError,
            format!("failed to read blob {sha} from git cat-file"),
        )),
    }
}

/// Run `git` and return the raw outcome: whether it succeeded, and
/// the stdout and stderr as text. For commands whose non-zero exit
/// is data, not failure (`config --get` on a missing key, a rebase
/// that stops on a conflict).
fn git_status(
    dir: &Path,
    subcommand: &str,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> Result<(bool, String, String), GeectlError> {
    let output = run_git(dir, subcommand, args, extra_env)?;
    Ok((
        output.success,
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

/// The environment that carries a token to git as an
/// `http.extraHeader`, without putting it in argv: the header is
/// never logged, and a git error never echoes it. `header` holds
/// the formatted header line and must outlive the call.
/// The Gitea credentials for a git-backed operation: the token and
/// the instance URL it belongs to. The URL scopes the token (see
/// `token_env`): git only sends it to that instance.
pub struct GiteaAuth<'a> {
    pub token: &'a str,
    pub url: &'a str,
}

/// The git environment carrying the Gitea token, scoped to the
/// Gitea instance: `http.<url>.extraHeader` is only sent to URLs
/// under that instance. An unscoped `http.extraHeader` would go to
/// whatever host the scmsync URL names — a maintainer-controlled
/// value — leaking the token there, and a bogus token would break a
/// public clone on a host that needs none.
fn token_env<'a>(
    auth: &GiteaAuth<'a>,
    header: &'a mut String,
    key: &'a mut String,
) -> [(&'a str, &'a str); 3] {
    *header = format!("Authorization: token {}", auth.token);
    *key = format!("http.{}.extraHeader", auth.url);
    [
        ("GIT_CONFIG_COUNT", "1"),
        ("GIT_CONFIG_KEY_0", key.as_str()),
        ("GIT_CONFIG_VALUE_0", header.as_str()),
    ]
}

/// Read one `geectl.*` config key. `Ok(None)` when the key is not
/// set (a missing key is silent on stderr); a stderr on failure
/// means the config itself is broken, and that is an error.
fn git_config_get(dir: &Path, key: &str) -> Result<Option<String>, GeectlError> {
    let (success, stdout, stderr) = git_status(
        dir,
        "config",
        &["--local", "--get", &format!("geectl.{key}")],
        &[],
    )?;
    if success {
        Ok(Some(stdout.trim_end_matches('\n').to_string()))
    } else if stderr.trim().is_empty() {
        Ok(None)
    } else {
        Err(git_error("config", stderr.as_bytes()))
    }
}

/// Write one `geectl.*` config key. Values never carry newlines:
/// the record stays one line per key.
fn git_config_set(dir: &Path, key: &str, value: &str) -> Result<(), GeectlError> {
    if value.contains(['\n', '\r']) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("geectl.{key} value must not contain a newline"),
        ));
    }
    git(
        dir,
        "config",
        &["--local", &format!("geectl.{key}"), value],
        &[],
    )?;
    Ok(())
}

/// The root commit of the repository in `dir`: the synthesized base
/// commit of an OBS-native checkout.
/// The root commit of the checkout: the one whose message parses
/// as a geectl base commit. A history with two roots (a manual
/// merge in the working copy) does not pick one arbitrarily: no
/// parsing root, or more than one, is an inconsistent working
/// copy.
fn root_commit(dir: &Path) -> Result<String, GeectlError> {
    let out = git(dir, "rev-list", &["--max-parents=0", "HEAD"], &[])?;
    let roots: Vec<&str> = out
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if roots.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::WorkingCopyInconsistent,
            "the working copy has no commits",
        ));
    }
    let mut found: Vec<String> = Vec::new();
    for root in &roots {
        let message = root_message(dir, root)?;
        if parse_base_message(&message).is_some() {
            found.push((*root).to_string());
        }
    }
    match found.len() {
        1 => Ok(found.into_iter().next().unwrap()),
        0 => Err(GeectlError::new(
            ErrorKind::WorkingCopyInconsistent,
            "the working copy root commit is not a geectl base commit",
        )),
        _ => Err(GeectlError::new(
            ErrorKind::WorkingCopyInconsistent,
            "the working copy has more than one root commit that parses as a geectl base commit",
        )),
    }
}

/// The base coordinates from a root commit message
/// (`geectl base <rev> <srcmd5>`). `None` when the message is not a
/// base message.
fn parse_base_message(message: &str) -> Option<(String, String)> {
    let rest = message.trim().strip_prefix("geectl base ")?;
    let mut parts = rest.split_whitespace();
    let rev = parts.next()?.to_string();
    let srcmd5 = parts.next()?.to_string();
    if parts.next().is_some() {
        return None;
    }
    Some((rev, srcmd5))
}

/// The root commit message for the repository in `dir`.
fn root_message(dir: &Path, root: &str) -> Result<String, GeectlError> {
    git(dir, "log", &["-1", "--format=%B", root], &[])
}

/// Open the working copy at `dir`: read the checkout record, and
/// check it against the base commit. A directory that is not a git
/// repository, or a repository without any `geectl.*` record, is
/// not a working copy. A record missing some keys, a backend that
/// does not parse, or a root commit that is not a base commit, is
/// an inconsistent checkout. When the root commit carries newer
/// base coordinates than the record (a rebase finished without the
/// record being updated), the record is rewritten from the root:
/// the history is the source of truth, the record follows it.
pub fn open(dir: &Path) -> Result<WorkingCopy, GeectlError> {
    open_inner(dir, true)
}

/// Open the working copy at `dir` without touching local state:
/// the record is read, never rewritten. Read paths (`status`,
/// `diff`) and dry-run plans use this: a read must not rewrite
/// `geectl.base-*` config. When the root commit disagrees with
/// the record, the root's coordinates are returned (the history
/// is the source of truth) and the record is left stale on disk.
pub fn open_read(dir: &Path) -> Result<WorkingCopy, GeectlError> {
    open_inner(dir, false)
}

fn open_inner(dir: &Path, heal: bool) -> Result<WorkingCopy, GeectlError> {
    if !dir.is_dir() {
        return Err(GeectlError::new(
            ErrorKind::WorkingCopyNotFound,
            "the directory is not a geectl working copy",
        ));
    }
    let (is_repo, _, _) = git_status(dir, "rev-parse", &["--git-dir"], &[])?;
    if !is_repo {
        return Err(GeectlError::new(
            ErrorKind::WorkingCopyNotFound,
            "the directory is not a geectl working copy",
        ));
    }
    // The checkout root is the repository top level: a
    // subdirectory is not a checkout. `diff --name-only` is
    // root-relative while `ls-files` and `:(literal)` pathspecs
    // are cwd-relative, so opening a subdirectory would read the
    // index wrong and treat changed files as deleted.
    let toplevel = git(dir, "rev-parse", &["--show-toplevel"], &[])?;
    let toplevel = toplevel.trim();
    let canonical_dir = std::fs::canonicalize(dir).map_err(|e| {
        GeectlError::new(
            ErrorKind::InternalError,
            format!("failed to resolve {}: {e}", dir.display()),
        )
    })?;
    let canonical_top = std::fs::canonicalize(toplevel).map_err(|e| {
        GeectlError::new(
            ErrorKind::InternalError,
            format!("failed to resolve the checkout root {toplevel}: {e}"),
        )
    })?;
    if canonical_dir != canonical_top {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!(
                "{} is a subdirectory of the checkout at {}: run working-copy commands from the checkout root",
                dir.display(),
                toplevel
            ),
        ));
    }
    let mut values: Vec<Option<String>> = Vec::new();
    for key in CONFIG_KEYS {
        values.push(git_config_get(dir, key)?);
    }
    let missing = values.iter().filter(|value| value.is_none()).count();
    if missing == CONFIG_KEYS.len() {
        return Err(GeectlError::new(
            ErrorKind::WorkingCopyNotFound,
            "the directory is not a geectl working copy",
        ));
    }
    if missing > 0 {
        return Err(GeectlError::new(
            ErrorKind::WorkingCopyInconsistent,
            "the working copy record is incomplete: some geectl.* keys are missing",
        ));
    }
    let values: Vec<String> = values.into_iter().map(|value| value.unwrap()).collect();
    let [apiurl, project, package, backend, base_rev, base_srcmd5] =
        <[String; 6]>::try_from(values).map_err(|_| {
            GeectlError::new(
                ErrorKind::WorkingCopyInconsistent,
                "the working copy record is incomplete",
            )
        })?;
    let backend = Backend::parse(&backend).ok_or_else(|| {
        GeectlError::new(
            ErrorKind::WorkingCopyInconsistent,
            format!("the working copy backend is not obs or git: {backend:?}"),
        )
    })?;
    let mut working_copy = WorkingCopy {
        root: dir.to_path_buf(),
        project,
        package,
        apiurl,
        backend,
        base_rev,
        base_srcmd5,
    };
    if backend == Backend::Obs {
        let root = root_commit(dir)?;
        let message = root_message(dir, &root)?;
        let (rev, srcmd5) = parse_base_message(&message).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::WorkingCopyInconsistent,
                "the working copy root commit is not a geectl base commit",
            )
        })?;
        if rev != working_copy.base_rev || srcmd5 != working_copy.base_srcmd5 {
            if heal {
                git_config_set(dir, "base-rev", &rev)?;
                git_config_set(dir, "base-srcmd5", &srcmd5)?;
            }
            working_copy.base_rev = rev;
            working_copy.base_srcmd5 = srcmd5;
        }
    }
    Ok(working_copy)
}

/// Split NUL-separated git output into paths, dropping the empty
/// tail after the final separator.
fn split_nul(out: &str) -> Vec<String> {
    out.split('\0')
        .filter(|field| !field.is_empty())
        .map(|field| field.to_string())
        .collect()
}

/// Whether `name` is a service-generated file (`_service:` or
/// `_service_*`): the file list skips these, since the server
/// regenerates them. The `_service` file itself is committed.
fn is_service_generated(name: &str) -> bool {
    name.starts_with("_service:") || name.starts_with("_service_")
}

/// The content of `path` as stored in the index. `--no-textconv`:
/// a textconv filter would return converted bytes, not the bytes
/// the commit sends.
fn index_content(wc: &WorkingCopy, path: &str) -> Result<Vec<u8>, GeectlError> {
    git_bytes(
        &wc.root,
        "show",
        &["--no-textconv", &format!(":{path}")],
        &[],
    )
}

/// The base tree's `(name, blob SHA)` entries, for the base
/// `base` (resolved once by the caller): one `ls-tree` for the
/// whole tree, so a per-file md5 never re-resolves the base.
fn ls_tree_entries(wc: &WorkingCopy, base: &str) -> Result<Vec<(String, String)>, GeectlError> {
    let out = git(&wc.root, "ls-tree", &["-r", "-z", base], &[])?;
    let mut entries = Vec::new();
    for field in split_nul(&out) {
        let Some((meta, name)) = field.split_once('\t') else {
            return Err(GeectlError::new(
                ErrorKind::WorkingCopyInconsistent,
                "git ls-tree output is malformed: an entry has no name",
            ));
        };
        let mut parts = meta.split_whitespace();
        let _mode = parts.next();
        let _kind = parts.next();
        let Some(sha) = parts.next() else {
            return Err(GeectlError::new(
                ErrorKind::WorkingCopyInconsistent,
                "git ls-tree output is malformed: an entry has no blob SHA",
            ));
        };
        entries.push((name.to_string(), sha.to_string()));
    }
    Ok(entries)
}

/// The index's `name -> blob SHA` map: one `ls-files` for the
/// whole index, so a per-file md5 never re-resolves the base.
fn ls_files_entries(
    wc: &WorkingCopy,
) -> Result<std::collections::HashMap<String, String>, GeectlError> {
    let out = git(&wc.root, "ls-files", &["-s", "-z"], &[])?;
    let mut entries = std::collections::HashMap::new();
    for field in split_nul(&out) {
        let Some((meta, name)) = field.split_once('\t') else {
            return Err(GeectlError::new(
                ErrorKind::WorkingCopyInconsistent,
                "git ls-files output is malformed: an entry has no name",
            ));
        };
        let mut parts = meta.split_whitespace();
        let _mode = parts.next();
        let Some(sha) = parts.next() else {
            return Err(GeectlError::new(
                ErrorKind::WorkingCopyInconsistent,
                "git ls-files output is malformed: an entry has no blob SHA",
            ));
        };
        entries.insert(name.to_string(), sha.to_string());
    }
    Ok(entries)
}

/// Whether `path` is in the index.
fn index_has(wc: &WorkingCopy, path: &str) -> Result<bool, GeectlError> {
    let (success, stdout, _) = git_status(
        &wc.root,
        "ls-files",
        &["--cached", "--", &format!(":(literal){path}")],
        &[],
    )?;
    Ok(success && !stdout.trim().is_empty())
}

/// The files whose index state differs from the base commit: the
/// staged plus locally committed changes. Unstaged worktree changes
/// are not in this list: a commit sends the index state, never the
/// worktree. `--no-renames`: a rename lists both paths (the old
/// path deleted, the new path added) — without it, only the new
/// path is listed and the old path is re-added from the base, so
/// the commit sends both files.
fn index_file_list(wc: &WorkingCopy) -> Result<Vec<String>, GeectlError> {
    let base = wc.base_commit()?;
    let out = git(
        &wc.root,
        "diff",
        &[
            "--cached",
            "--no-renames",
            "--no-ext-diff",
            "--no-color",
            "--no-textconv",
            "--name-only",
            "-z",
            &base,
        ],
        &[],
    )?;
    Ok(split_nul(&out))
}

/// The files a rebase or merge left unmerged.
fn unmerged_files(wc: &WorkingCopy) -> Result<Vec<String>, GeectlError> {
    let out = git(
        &wc.root,
        "diff",
        &[
            "--name-only",
            "--diff-filter=U",
            "--no-ext-diff",
            "--no-color",
            "--no-textconv",
            "-z",
        ],
        &[],
    )?;
    Ok(split_nul(&out))
}

/// Whether a rebase is in progress in the repository.
fn rebase_in_progress(wc: &WorkingCopy) -> Result<bool, GeectlError> {
    let git_dir = git(&wc.root, "rev-parse", &["--absolute-git-dir"], &[])?;
    let git_dir = Path::new(git_dir.trim());
    Ok(git_dir.join("rebase-merge").is_dir() || git_dir.join("rebase-apply").is_dir())
}

/// A guard that removes temporary files and directories on drop:
/// the update tree build's temp index and download directory never
/// survive the call, on success or on failure.
struct TempCleanup {
    paths: Vec<PathBuf>,
}

impl Drop for TempCleanup {
    fn drop(&mut self) {
        for path in &self.paths {
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(path);
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// A path segment for a project or package name: `:` is legal in
/// an OBS name but not in a Windows path, so it becomes `_`. The
/// record in the checkout's git config keeps the real name; the
/// directory name never names the project or package back. Two
/// names can sanitize to the same directory (`home:foo` and
/// `home_foo`); the second checkout fails on the existing target.
fn path_segment(name: &str) -> String {
    name.replace(':', "_")
}

/// Refuse a project or package name that is not a single path
/// segment: `.` and `..` would escape the checkout directory
/// (and a failed checkout's cleanup would delete outside it),
/// and a separator or NUL is never a name. OBS names allow `.`
/// (so `..` passes the server's own name check) — this is the
/// check that stops it.
fn check_name_segment(kind: &str, name: &str) -> Result<(), GeectlError> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid OBS {kind} name: {name:?}"),
        ));
    }
    Ok(())
}

/// The checkout directory for a project and package:
/// `<dir>/<project>/<package>/` with `:` sanitized (see
/// `path_segment`). The CLI renders the same target for a dry-run
/// checkout, so the plan names the directory the live checkout
/// creates.
pub fn checkout_target(dir: &Path, project: &str, package: &str) -> Result<PathBuf, GeectlError> {
    check_name_segment("project", project)?;
    check_name_segment("package", package)?;
    Ok(dir.join(path_segment(project)).join(path_segment(package)))
}

/// Refuse a directory listing whose entries are not plain file
/// names: a name with a separator or a leading dot (for example
/// `../evil`) would write outside the checkout when downloaded.
/// The server's own file-name rule is the check, so a legitimate
/// listing always passes; `\` is refused too, since it separates
/// paths on Windows. Two entries that collide case-insensitively
/// (`Foo.spec` vs `foo.spec`) are refused too: a case-insensitive
/// checkout cannot hold both.
fn check_listing_names(listing: &DirectoryListing) -> Result<(), GeectlError> {
    let mut seen: std::collections::HashMap<String, &str> = std::collections::HashMap::new();
    for entry in &listing.entries {
        if !valid_obs_filename(&entry.name) || entry.name.contains('\\') {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "invalid file name in the directory listing: {:?}",
                    entry.name
                ),
            ));
        }
        // A case-insensitive checkout (macOS, Windows) cannot hold
        // both `Foo.spec` and `foo.spec`: the second download
        // overwrites the first. The fold is Unicode-aware, not
        // ASCII-only, so it holds for non-ASCII names too.
        let folded = entry.name.to_lowercase();
        if let Some(first) = seen.get(&folded) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "file names in the directory listing collide case-insensitively: {first:?} and {:?}",
                    entry.name
                ),
            ));
        }
        seen.insert(folded, entry.name.as_str());
    }
    Ok(())
}

/// Check out package `package` in project `project` into
/// `<dir>/<project>/<package>/`. A git-backed package (its `_meta`
/// carries a scmsync URL) is cloned; an OBS-native package is
/// downloaded file by file and committed as a synthesized root
/// commit carrying the base coordinates. `rev` pins the base
/// revision for an OBS-native checkout; a git-backed checkout does
/// not take one. Returns `None` in dry-run.
pub fn checkout(
    client: &ObsClient,
    project: &str,
    package: &str,
    rev: Option<&RevisionSpec>,
    dir: &Path,
    gitea_auth: Option<&GiteaAuth<'_>>,
    dry_run: &DryRun,
) -> Result<Option<WorkingCopy>, GeectlError> {
    // The segment check runs before anything else: a `..`
    // project would otherwise put the target (and the failure
    // cleanup below) outside `dir`.
    let target = checkout_target(dir, project, package)?;
    if target.exists() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!(
                "checkout target already exists: {}; on a case-insensitive filesystem a checkout with the same name in different case counts as existing",
                target.display()
            ),
        ));
    }
    let meta = client.package_meta(project, package)?;
    if rev.is_some() && meta.scmsync.is_some() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "checkout --rev is not supported for a git-backed package",
        ));
    }
    if !dry_run.allow(&format!(
        "checkout {project}/{package} into {}",
        target.display()
    )) {
        // The dry-run plan, per file, after the summary line: the
        // files the checkout would download, or the repository it
        // would clone. The --rev refusal above fires before any
        // plan line, the same as in live mode.
        match &meta.scmsync {
            Some(url) => {
                dry_run.note(&format!("would clone {url} into {}", target.display()));
            }
            None => {
                let (listing, _) = client.directory_listing_rev(project, package, rev)?;
                for entry in &listing.entries {
                    dry_run.note(&format!(
                        "would download {} into {}",
                        entry.name,
                        target.display()
                    ));
                }
            }
        }
        return Ok(None);
    }
    // A failed checkout leaves no partial target behind: the
    // target did not exist on entry (refused above), so anything
    // there on failure is this call's, and the next attempt must
    // not trip over it.
    let checked_out = match meta.scmsync {
        Some(url) => checkout_git(client, project, package, &url, dir, gitea_auth),
        None => checkout_obs(client, project, package, rev, dir),
    };
    match checked_out {
        Ok(wc) => Ok(Some(wc)),
        Err(err) => {
            let _ = std::fs::remove_dir_all(&target);
            Err(err)
        }
    }
}

/// Download `name` at the pinned `rev` and verify it against the
/// listing's md5: the bytes are hashed as they are read back from
/// disk, so a revision that landed between the listing and the
/// download (or a corrupt transfer) is refused before the file is
/// used. `expected_md5` is `None` when the listing carries no md5:
/// the download is still pinned, just not verified.
#[allow(clippy::too_many_arguments)]
fn download_verified(
    client: &ObsClient,
    project: &str,
    package: &str,
    name: &str,
    rev: Option<&RevisionSpec>,
    dest: &Path,
    expected_md5: Option<&str>,
    expanded: bool,
) -> Result<(), GeectlError> {
    if expanded {
        client.download_source_expanded_to_file(project, package, name, rev, dest)?;
    } else {
        client.download_source_to_file(project, package, name, rev, dest)?;
    }
    if let Some(expected) = expected_md5 {
        let actual = md5_of_file(dest)?;
        if actual != expected {
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "downloaded {name} does not match the directory listing (md5 {actual} != {expected}): the server moved under the download; retry the operation",
                ),
            ));
        }
    }
    Ok(())
}

/// The OBS-native checkout: download every file at the base
/// revision, then commit the tree as the root commit.
fn checkout_obs(
    client: &ObsClient,
    project: &str,
    package: &str,
    rev: Option<&RevisionSpec>,
    dir: &Path,
) -> Result<WorkingCopy, GeectlError> {
    let target = checkout_target(dir, project, package)?;
    let (listing, (rev_number, _, srcmd5)) = client.directory_listing_rev(project, package, rev)?;
    let rev_number = rev_number.ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("the server did not return a revision for {project}/{package}"),
        )
    })?;
    let srcmd5 = srcmd5.ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("the server did not return a source md5 for {project}/{package}"),
        )
    })?;
    // The downloads are pinned to the listing's revision: a
    // revision landing between the listing and the download is
    // recorded under the older rev otherwise.
    let pinned_rev = rev_number.parse::<u64>().map_err(|_| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("the server returned a non-numeric revision for {project}/{package}"),
        )
    })?;
    let pinned = RevisionSpec::Rev(pinned_rev);
    check_listing_names(&listing)?;
    std::fs::create_dir_all(&target).map_err(|e| {
        GeectlError::new(
            ErrorKind::InternalError,
            format!("failed to create {}: {e}", target.display()),
        )
    })?;
    for entry in &listing.entries {
        download_verified(
            client,
            project,
            package,
            &entry.name,
            Some(&pinned),
            &target.join(&entry.name),
            entry.md5.as_deref(),
            false,
        )?;
    }
    git(&target, "init", &["--quiet"], &[])?;
    write_record(
        &target,
        &client.base,
        project,
        package,
        Backend::Obs,
        &rev_number,
        &srcmd5,
    )?;
    // Forced: `git add -A` honours the user's global excludes, so
    // a `*.patch` in `core.excludesFile` would leave server files
    // out of the base and the next commit would delete them on the
    // server.
    git(&target, "add", &["-f", "-A"], &[])?;
    // An empty package has nothing to add: the root commit is
    // created over the empty tree, so the checkout still opens.
    git(
        &target,
        "commit",
        &[
            "--quiet",
            "--no-verify",
            "--allow-empty",
            "-m",
            &format!("geectl base {rev_number} {srcmd5}"),
        ],
        &[],
    )?;
    Ok(WorkingCopy {
        root: target,
        project: project.to_string(),
        package: package.to_string(),
        apiurl: client.base.clone(),
        backend: Backend::Obs,
        base_rev: rev_number,
        base_srcmd5: srcmd5,
    })
}

/// Split a scmsync URL (`url#branch`) into the clone URL and the
/// branch the fragment names, when one is present.
fn split_scmsync_url(url: &str) -> (&str, Option<&str>) {
    match url.split_once('#') {
        Some((repo, branch)) => (repo, Some(branch)),
        None => (url, None),
    }
}

/// The git-backed checkout: clone the scmsync repository. The
/// clone's HEAD is the base.
fn checkout_git(
    client: &ObsClient,
    project: &str,
    package: &str,
    url: &str,
    dir: &Path,
    gitea_auth: Option<&GiteaAuth<'_>>,
) -> Result<WorkingCopy, GeectlError> {
    let target = checkout_target(dir, project, package)?;
    // The `#branch` fragment names the branch to check out, but
    // git does not understand it: passing it to `git clone` makes
    // the fragment part of the path, and the clone fails. Strip it
    // and check the branch out with `--branch` instead.
    let (clone_url, branch) = split_scmsync_url(url);
    // A scmsync URL comes from the server (`_meta`), so it is
    // maintainer-controlled: only http(s) is accepted. A `file://`
    // URL, a local path, `ssh://` (the user's keys and agent), or
    // `git://` would hand the clone to a transport the checkout
    // never asked for. The refusal runs before anything is created
    // on disk.
    if !(clone_url.starts_with("http://") || clone_url.starts_with("https://")) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("refusing to clone a scmsync URL that is not http(s): {clone_url:?}"),
        ));
    }
    std::fs::create_dir_all(target.parent().unwrap_or(dir)).map_err(|e| {
        GeectlError::new(
            ErrorKind::InternalError,
            format!(
                "failed to create {}: {e}",
                target.parent().unwrap_or(dir).display()
            ),
        )
    })?;
    let mut header = String::new();
    let mut key = String::new();
    let mut env: Vec<(&str, &str)> = match gitea_auth {
        Some(auth) => token_env(auth, &mut header, &mut key).to_vec(),
        None => Vec::new(),
    };
    // Belt and braces with the scheme check above: only http(s)
    // transports are allowed for the clone, whatever the URL
    // names.
    env.push(("GIT_ALLOW_PROTOCOL", "http:https"));
    let target_str = target.to_string_lossy().into_owned();
    let mut args: Vec<&str> = Vec::new();
    if let Some(branch) = branch {
        if !branch.is_empty() {
            args.push("--branch");
            args.push(branch);
        }
    }
    // The `--` keeps the URL a repository, never an option: the
    // scheme check above already refuses a `-`-prefixed URL, and
    // this stops any other option-shaped value from being parsed
    // as one.
    args.push("--");
    args.push(clone_url);
    args.push(&target_str);
    git_network(dir, "clone", &args, &env)?;
    let head = git(&target, "rev-parse", &["HEAD"], &[])?
        .trim()
        .to_string();
    write_record(
        &target,
        &client.base,
        project,
        package,
        Backend::Git,
        &head,
        "",
    )?;
    Ok(WorkingCopy {
        root: target,
        project: project.to_string(),
        package: package.to_string(),
        apiurl: client.base.clone(),
        backend: Backend::Git,
        base_rev: head,
        base_srcmd5: String::new(),
    })
}

/// Write the checkout record: the repo-local identity for
/// synthesized commits, then the `geectl.*` keys.
fn write_record(
    dir: &Path,
    apiurl: &str,
    project: &str,
    package: &str,
    backend: Backend,
    base_rev: &str,
    base_srcmd5: &str,
) -> Result<(), GeectlError> {
    git(dir, "config", &["--local", "user.name", "geectl"], &[])?;
    git(
        dir,
        "config",
        &["--local", "user.email", "geectl@localhost"],
        &[],
    )?;
    git_config_set(dir, "apiurl", apiurl)?;
    git_config_set(dir, "project", project)?;
    git_config_set(dir, "package", package)?;
    git_config_set(dir, "backend", backend.as_str())?;
    git_config_set(dir, "base-rev", base_rev)?;
    git_config_set(dir, "base-srcmd5", base_srcmd5)?;
    Ok(())
}

/// The files' state against the base commit, from the index and
/// the worktree diffs against the base — not from `git status`,
/// which compares against HEAD and so reports a clean tree after a
/// local commit ahead of the base. `status` and `diff` read the
/// same base, so they never disagree about the checkout.
pub fn status(wc: &WorkingCopy) -> Result<Vec<StatusEntry>, GeectlError> {
    let base = wc.base_commit()?;
    let cached = name_status(&wc.root, &["--cached", &base])?;
    let worktree = name_status(&wc.root, &[])?;
    let untracked = git(
        &wc.root,
        "ls-files",
        &["--others", "--exclude-standard", "-z"],
        &[],
    )?;
    let unmerged = unmerged_files(wc)?;
    // One map for the merged `XY` pair per path (a linear
    // find per entry is O(n^2) on a large checkout), one set
    // for the entries already reported.
    let mut codes: std::collections::HashMap<String, (char, char)> =
        std::collections::HashMap::new();
    let put = |path: String,
               x: char,
               y: char,
               codes: &mut std::collections::HashMap<String, (char, char)>| {
        let slot = codes.entry(path).or_insert((' ', ' '));
        if x != ' ' {
            slot.0 = x;
        }
        if y != ' ' {
            slot.1 = y;
        }
    };
    for (code, path) in cached {
        put(path, code, ' ', &mut codes);
    }
    for (code, path) in worktree {
        put(path, ' ', code, &mut codes);
    }
    let mut entries: Vec<StatusEntry> = codes
        .into_iter()
        .filter_map(|(path, (x, y))| status_code(x, y).map(|code| StatusEntry { path, code }))
        .collect();
    let mut seen: std::collections::HashSet<String> =
        entries.iter().map(|entry| entry.path.clone()).collect();
    for path in split_nul(&untracked) {
        // A file staged for deletion that is still on disk is
        // both in the cached diff (Deleted) and untracked: report
        // it once, as the staged state.
        if seen.insert(path.clone()) {
            entries.push(StatusEntry {
                path,
                code: StatusCode::Unknown,
            });
        }
    }
    for path in unmerged {
        if let Some(entry) = entries.iter_mut().find(|entry| entry.path == path) {
            entry.code = StatusCode::Conflicted;
        } else {
            entries.push(StatusEntry {
                path,
                code: StatusCode::Conflicted,
            });
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

/// One file's state from an `XY` pair: X is the index against the
/// base, Y is the worktree against the index. `None` is a clean
/// pair, which a diff never reports.
fn status_code(x: char, y: char) -> Option<StatusCode> {
    if is_unmerged(x, y) {
        Some(StatusCode::Conflicted)
    } else if x == '?' && y == '?' {
        Some(StatusCode::Unknown)
    } else if x == 'R' {
        Some(StatusCode::Renamed)
    } else if x == 'A' {
        Some(StatusCode::Added)
    } else if x == 'D' {
        Some(StatusCode::Deleted)
    } else if y == 'D' {
        Some(StatusCode::Missing)
    } else if x == 'M' || y == 'M' || x == 'T' || y == 'T' {
        Some(StatusCode::Modified)
    } else {
        None
    }
}

/// `git diff --name-status -z` with `args`: one `(code, path)` pair
/// per changed file. A rename names its new path; the old path is
/// skipped (the record itself carries the new path, as in
/// porcelain `-z`).
fn name_status(dir: &Path, args: &[&str]) -> Result<Vec<(char, String)>, GeectlError> {
    // No `--no-renames` here: `status` reports renames as `R`,
    // and this parser reads the rename record (old path skipped,
    // new path kept). The `--no-renames` fix is for the diffs
    // that derive file lists for a commit.
    let mut full = vec![
        "diff",
        "--name-status",
        "--no-ext-diff",
        "--no-color",
        "--no-textconv",
        "-z",
    ];
    full.extend_from_slice(args);
    let out = git(dir, full[0], &full[1..], &[])?;
    let mut entries = Vec::new();
    let mut fields = out.split('\0');
    while let Some(status_field) = fields.next() {
        if status_field.is_empty() {
            continue;
        }
        let code = status_field.chars().next().unwrap_or(' ');
        if code == 'R' {
            // `R100\0<old>\0<new>\0`: the record names the new path.
            let _old = fields.next();
            let Some(new) = fields.next() else {
                return Err(GeectlError::new(
                    ErrorKind::WorkingCopyInconsistent,
                    "git diff output is malformed: a rename has no new path",
                ));
            };
            entries.push((code, new.to_string()));
        } else {
            let Some(path) = fields.next() else {
                return Err(GeectlError::new(
                    ErrorKind::WorkingCopyInconsistent,
                    "git diff output is malformed: a change has no path",
                ));
            };
            entries.push((code, path.to_string()));
        }
    }
    Ok(entries)
}

/// Whether an `XY` status pair is an unmerged (conflicted) state.
fn is_unmerged(x: char, y: char) -> bool {
    matches!(
        (x, y),
        ('D', 'D') | ('A', 'U') | ('U', 'D') | ('U', 'A') | ('D', 'U') | ('A', 'A') | ('U', 'U')
    )
}

/// The diff of the worktree and the index against the base commit,
/// plus the untracked files (listed, not diffed). The staged half
/// (the index against the base) plus the unstaged half (the
/// worktree against the index) cover every change exactly once: a
/// single `git diff <base>` sees only the worktree, so a
/// staged-only change would be invisible.
pub fn diff(wc: &WorkingCopy) -> Result<String, GeectlError> {
    let base = wc.base_commit()?;
    let mut out = git(
        &wc.root,
        "diff",
        &[
            "--cached",
            "--no-ext-diff",
            "--no-color",
            "--no-textconv",
            &base,
        ],
        &[],
    )?;
    out.push_str(&git(
        &wc.root,
        "diff",
        &["--no-ext-diff", "--no-color", "--no-textconv"],
        &[],
    )?);
    let untracked = git(
        &wc.root,
        "ls-files",
        &["--others", "--exclude-standard", "-z"],
        &[],
    )?;
    let untracked = split_nul(&untracked);
    if !untracked.is_empty() {
        out.push_str("\nUntracked:\n");
        for path in untracked {
            out.push_str(&path);
            out.push('\n');
        }
    }
    Ok(out)
}

/// Refuse paths that are absolute or escape the checkout root.
fn check_relative_paths(files: &[String]) -> Result<(), GeectlError> {
    for file in files {
        let path = Path::new(file);
        if path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("path escapes the working copy: {file:?}"),
            ));
        }
    }
    Ok(())
}

/// Stage `files` for commit. Returns `None` in dry-run.
pub fn add(
    wc: &WorkingCopy,
    files: &[String],
    dry_run: &DryRun,
) -> Result<Option<()>, GeectlError> {
    check_relative_paths(files)?;
    if !dry_run.allow(&format!(
        "stage {} in {}",
        files.join(", "),
        wc.root.display()
    )) {
        return Ok(None);
    }
    for file in files {
        // A file is stageable when it is in the index or on disk:
        // a tracked file deleted from the worktree stages as a
        // deletion, the same as `git add` does.
        if !index_has(wc, file)? && !wc.root.join(file).exists() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("no such file in the working copy: {file:?}"),
            ));
        }
    }
    let mut args = vec!["--"];
    args.extend(files.iter().map(String::as_str));
    // Literal pathspecs: a user path is a path, never a glob.
    git_literal(&wc.root, "add", &args, &[])?;
    Ok(Some(()))
}

/// Refuse a path that traverses a symlink: removing through one
/// would delete outside the checkout (`rm link/evil.txt` where
/// `link` points elsewhere). Removing the link itself is fine —
/// only traversal is refused.
fn check_no_symlink_parents(wc: &WorkingCopy, files: &[String]) -> Result<(), GeectlError> {
    for file in files {
        let mut path = wc.root.clone();
        let components: Vec<_> = Path::new(file).components().collect();
        for component in &components[..components.len().saturating_sub(1)] {
            path.push(component);
            if std::fs::symlink_metadata(&path)
                .map(|meta| meta.file_type().is_symlink())
                .unwrap_or(false)
            {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("path traverses a symlink: {file:?}"),
                ));
            }
        }
    }
    Ok(())
}

/// Remove `files` from the working copy: a tracked file is removed
/// with `git rm` (staged as a deletion), an untracked file is
/// deleted. Returns `None` in dry-run.
pub fn rm(wc: &WorkingCopy, files: &[String], dry_run: &DryRun) -> Result<Option<()>, GeectlError> {
    check_relative_paths(files)?;
    check_no_symlink_parents(wc, files)?;
    if !dry_run.allow(&format!(
        "remove {} from {}",
        files.join(", "),
        wc.root.display()
    )) {
        return Ok(None);
    }
    // Every file is validated before anything is removed: a
    // missing file errors before a tracked file is removed or an
    // untracked file is deleted, so a failed `rm` leaves the
    // checkout untouched.
    let mut tracked: Vec<String> = Vec::new();
    let mut untracked: Vec<String> = Vec::new();
    for file in files {
        if index_has(wc, file)? {
            tracked.push(file.clone());
        } else if wc.root.join(file).exists() {
            if wc.root.join(file).is_dir() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("rm removes files, not directories: {file:?}"),
                ));
            }
            untracked.push(file.clone());
        } else {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("no such file in the working copy: {file:?}"),
            ));
        }
    }
    // The tracked removal runs first: an untracked file deleted
    // before a failing `git rm` would leave a partial state (the
    // untracked file gone, the tracked file still there).
    if !tracked.is_empty() {
        // Forced: the agent asked for the removal, and the file's
        // modifications go with it — the same as the existing
        // tool's remove.
        let mut args = vec!["-f", "--"];
        args.extend(tracked.iter().map(String::as_str));
        // Literal pathspecs: a user path is a path, never a glob.
        git_literal(&wc.root, "rm", &args, &[])?;
    }
    for file in &untracked {
        std::fs::remove_file(wc.root.join(file)).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("failed to remove {file}: {e}"),
            )
        })?;
    }
    Ok(Some(()))
}

/// Stage new files and worktree deletions: `status` + add of
/// `?` + rm of `!`, as the design doc says. `status` lists
/// untracked files with `--exclude-standard`, so a file the user
/// excluded (global excludes, `.gitignore`, `.git/info/exclude`)
/// is never in the `?` set and is never staged — unlike
/// `git add -f -A`, which would stage it and the next commit
/// would ship it to OBS. Tracked files are unaffected by ignore
/// rules either way. Modifications to tracked files are staged
/// with `add`, not here. Returns `None` in dry-run.
pub fn addremove(wc: &WorkingCopy, dry_run: &DryRun) -> Result<Option<()>, GeectlError> {
    if !dry_run.allow(&format!(
        "stage new files and worktree deletions in {}",
        wc.root.display()
    )) {
        return Ok(None);
    }
    let entries = status(wc)?;
    let untracked: Vec<&str> = entries
        .iter()
        .filter(|entry| entry.code == StatusCode::Unknown)
        .map(|entry| entry.path.as_str())
        .collect();
    // A staged-new file deleted from the worktree is `Added`,
    // and a staged rename whose new path is deleted is
    // `Renamed`, not `Missing` (the cached diff wins), but both
    // stage as a deletion all the same, as `git add -A` did:
    // otherwise the next commit would send a file that no
    // longer exists.
    // `symlink_metadata`, not `exists`: a broken symlink is
    // still in the worktree.
    let missing: Vec<&str> = entries
        .iter()
        .filter(|entry| {
            entry.code == StatusCode::Missing
                || ((entry.code == StatusCode::Added || entry.code == StatusCode::Renamed)
                    && wc.root.join(&entry.path).symlink_metadata().is_err())
        })
        .map(|entry| entry.path.as_str())
        .collect();
    if !untracked.is_empty() {
        // Literal pathspecs, no `-f`: the paths are the `?` set,
        // never an ignored file.
        let mut args = vec!["--"];
        args.extend(untracked);
        git_literal(&wc.root, "add", &args, &[])?;
    }
    if !missing.is_empty() {
        // Forced: a missing file with staged changes still stages
        // as a deletion (the `rm` leaf does the same). The `-f`
        // cannot stage an ignored file: the `!` set holds tracked
        // files only, and ignore rules never apply to those.
        let mut args = vec!["-f", "--"];
        args.extend(missing);
        git_literal(&wc.root, "rm", &args, &[])?;
    }
    Ok(Some(()))
}

/// A snapshot of the tracked files' worktree content before the
/// service gate, so the gate's outputs can be staged afterwards
/// without staging unrelated unstaged changes.
struct GateSnapshot {
    hashes: Vec<(String, Option<String>)>,
}

fn snapshot_gate_inputs(wc: &WorkingCopy) -> Result<GateSnapshot, GeectlError> {
    let tracked = git(&wc.root, "ls-files", &["-z"], &[])?;
    let mut hashes = Vec::new();
    for path in split_nul(&tracked) {
        // Streamed, and never through a symlink: a tracked
        // symlink to `/dev/zero` or a FIFO would hang or exhaust
        // memory on `fs::read`, and a symlink's target bytes are
        // not the file's content.
        hashes.push((path.clone(), hash_worktree_file(&wc.root, &path)));
    }
    Ok(GateSnapshot { hashes })
}

/// Stage what the service gate changed: a tracked file whose
/// worktree content differs from the snapshot, or one the gate
/// deleted. New files are never staged here — the gate only
/// generates `_service:*` files, which the commit skips. A file
/// the gate changed is staged whole, including any pre-existing
/// unstaged edit to the same file; other unstaged changes are not
/// staged.
fn stage_gate_changes(wc: &WorkingCopy, before: &GateSnapshot) -> Result<Vec<String>, GeectlError> {
    let mut changed = Vec::new();
    for (path, hash) in &before.hashes {
        let now = hash_worktree_file(&wc.root, path);
        if now.as_deref() != hash.as_deref() {
            changed.push(path.clone());
        }
    }
    if !changed.is_empty() {
        let mut args = vec!["--"];
        args.extend(changed.iter().map(String::as_str));
        // Literal pathspecs: a gate-changed path is a path, never
        // a glob.
        git_literal(&wc.root, "add", &args, &[])?;
    }
    Ok(changed)
}

/// The service gate: run the source services and `pre_checkin.sh`
/// in the checkout before a commit. A failure refuses the commit:
/// the gate is hard.
fn run_service_gate(client: &ObsClient, config: &Config, dir: &Path) -> Result<(), GeectlError> {
    let vertical = LocalBuild::from_config(client, config)?;
    vertical.precommit(dir, PRECOMMIT_TIMEOUT_SECS, false)?;
    Ok(())
}

/// The file list for an OBS-native commit: the full list derived
/// from the base plus the staged diff — every base file with the
/// base content's md5, overridden by the staged diff (a changed or
/// added file carries the index content's md5, a deleted file is
/// omitted). The server builds the new revision from exactly the
/// list it receives. Service-generated files are never sent. The
/// second value is every changed file the commit sends, deletions
/// included.
fn build_commit_entries(
    wc: &WorkingCopy,
) -> Result<(Vec<CommitFileEntry>, Vec<String>), GeectlError> {
    let base = wc.base_commit()?;
    let changed = index_file_list(wc)?;
    let changed_names: std::collections::HashSet<&str> =
        changed.iter().map(String::as_str).collect();
    // One `ls-tree`, one `ls-files`, one `cat-file --batch` for
    // the whole file list: the md5s stream through the batch
    // process instead of one `git show` spawn (and one
    // base-commit re-resolution) per file, buffering each blob.
    let base_entries = ls_tree_entries(wc, &base)?;
    let index_entries = ls_files_entries(wc)?;
    let mut batch = BlobBatch::new(&wc.root)?;
    let mut entries = Vec::new();
    let mut files = Vec::new();
    for (name, sha) in &base_entries {
        if is_service_generated(name) || changed_names.contains(name.as_str()) {
            continue;
        }
        let md5 = batch.md5(sha)?;
        entries.push(CommitFileEntry::new(name, &md5));
    }
    for name in changed {
        if is_service_generated(&name) {
            continue;
        }
        files.push(name.clone());
        if let Some(sha) = index_entries.get(&name) {
            let md5 = batch.md5(sha)?;
            entries.push(CommitFileEntry::new(&name, &md5));
        }
    }
    Ok((entries, files))
}

/// Move the base to the committed revision: a new root commit from
/// the index tree, then a soft reset onto it (the worktree and the
/// index stay as they are; the old local commits' identities go).
/// The record follows the new root.
fn refresh_base(wc: &mut WorkingCopy, rev: &str, srcmd5: &str) -> Result<(), GeectlError> {
    let tree = git(&wc.root, "write-tree", &[], &[])?.trim().to_string();
    let message = format!("geectl base {rev} {srcmd5}");
    let commit = git(&wc.root, "commit-tree", &[&tree, "-m", &message], &[])?;
    git(&wc.root, "reset", &["--soft", commit.trim()], &[])?;
    git_config_set(&wc.root, "base-rev", rev)?;
    git_config_set(&wc.root, "base-srcmd5", srcmd5)?;
    wc.base_rev = rev.to_string();
    wc.base_srcmd5 = srcmd5.to_string();
    Ok(())
}

/// Commit the working copy: the index state against the base, with
/// the service gate (unless `noservice`). An OBS-native commit
/// sends the file list through the `commitfilelist` protocol and
/// refreshes the base to the new revision; a git-backed commit
/// pushes the local commits and polls the OBS side until the sync
/// picks the pushed head up. A successful commit updates this
/// checkout's recorded base, on disk and in the struct. Returns
/// `None` in dry-run.
#[allow(clippy::too_many_arguments)]
pub fn commit(
    wc: &mut WorkingCopy,
    client: &ObsClient,
    config: &Config,
    message: &str,
    noservice: bool,
    gitea_auth: Option<&GiteaAuth<'_>>,
    dry_run: &DryRun,
) -> Result<Option<CommitReport>, GeectlError> {
    // An empty message aborts `git commit` on the git path and
    // sends an empty comment on the OBS path: refuse it up front,
    // on both.
    if message.trim().is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "commit needs a non-empty --message",
        ));
    }
    if !dry_run.allow(&format!(
        "commit working copy {}/{}: {message}",
        wc.project, wc.package
    )) {
        record_commit_plan(wc, client, dry_run)?;
        return Ok(None);
    }
    match wc.backend {
        Backend::Obs => commit_obs(wc, client, config, message, noservice, dry_run).map(Some),
        Backend::Git => commit_git(wc, client, config, message, noservice, gitea_auth).map(Some),
    }
}

/// Refuse to commit a package whose `_link` points at a
/// different project: geectl does not auto-branch a link to a
/// different project (the existing tool branches); the agent
/// decides, and the error names the `branch_package` it needs —
/// the link target branched into this project and package.
fn refuse_link_to_other_project(wc: &WorkingCopy) -> Result<(), GeectlError> {
    if !index_has(wc, "_link")? {
        return Ok(());
    }
    let (link_project, link_package) = read_link_target(wc)?;
    if link_project != wc.project {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!(
                "refusing to commit a link to a different project: _link points at {link_project}/{link_package}; branch it first (branch_package {link_project}/{link_package} into {}/{})",
                wc.project, wc.package
            ),
        ));
    }
    Ok(())
}

/// The dry-run plan for a commit: the files it would send, and the
/// files it would delete (a staged deletion is sent by omission).
/// A git-backed commit lists the staged files it would commit and
/// the local commits it would push.
fn record_commit_plan(
    wc: &WorkingCopy,
    client: &ObsClient,
    dry_run: &DryRun,
) -> Result<(), GeectlError> {
    match wc.backend {
        Backend::Obs => {
            // The outdated check live commit_obs runs first: the
            // plan refuses a commit the live path would refuse.
            refuse_link_to_other_project(wc)?;
            refuse_when_outdated_obs(wc, client)?;
            refuse_on_conflicts(wc)?;
            let (entries, files) = build_commit_entries(wc)?;
            let sent: std::collections::HashSet<&str> =
                entries.iter().map(|entry| entry.name.as_str()).collect();
            for name in &files {
                if sent.contains(name.as_str()) {
                    dry_run.note(&format!("would send {name}"));
                } else {
                    dry_run.note(&format!("would delete {name}"));
                }
            }
            Ok(())
        }
        Backend::Git => {
            // The outdated check live commit_git runs after its
            // fetch, against the last-fetched upstream: the plan
            // does not fetch, so it refuses only when the checkout
            // is behind what was last fetched.
            refuse_when_remote_ahead(wc)?;
            refuse_on_conflicts(wc)?;
            let staged = git(
                &wc.root,
                "diff",
                &[
                    "--cached",
                    "--no-renames",
                    "--no-ext-diff",
                    "--no-color",
                    "--no-textconv",
                    "--name-only",
                    "-z",
                    "HEAD",
                ],
                &[],
            )?;
            for name in split_nul(&staged) {
                dry_run.note(&format!("would commit {name}"));
            }
            let head = git(&wc.root, "rev-parse", &["HEAD"], &[])?
                .trim()
                .to_string();
            if head != wc.base_rev {
                let count = git(
                    &wc.root,
                    "rev-list",
                    &["--count", &format!("{}..HEAD", wc.base_rev)],
                    &[],
                )?
                .trim()
                .to_string();
                dry_run.note(&format!("would push {count} local commit(s) ({head})"));
            }
            Ok(())
        }
    }
}

/// Attach the sha256 of `content` to the file-list entry for `name`:
/// the server verifies it against the staged bytes.
fn attach_sha256(entries: &mut [CommitFileEntry], name: &str, content: &[u8]) {
    let sha256 = format!("{:x}", Sha256::digest(content));
    if let Some(entry) = entries.iter_mut().find(|entry| entry.name == name) {
        entry.sha256 = Some(sha256);
    }
}

/// The OBS-native commit: the validate/stage/final sequence, the
/// service-settle poll, and the base refresh. The refusals run
/// before the service gate, in order: outdated base, conflicts,
/// nothing to commit. The gate runs last of the checks, and its
/// outputs are staged into the index, so the commit sends them.
fn commit_obs(
    wc: &mut WorkingCopy,
    client: &ObsClient,
    config: &Config,
    message: &str,
    noservice: bool,
    dry_run: &DryRun,
) -> Result<CommitReport, GeectlError> {
    // The upstream check: the checkout's base must equal the
    // server's latest revision, else the commit would land on top
    // of a revision the checkout never saw.
    refuse_link_to_other_project(wc)?;
    refuse_when_outdated_obs(wc, client)?;
    refuse_on_conflicts(wc)?;
    let (_, committed_files) = build_commit_entries(wc)?;
    if committed_files.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::NothingToCommit,
            "there is nothing to commit: the index matches the base",
        ));
    }
    let snapshot = snapshot_gate_inputs(wc)?;
    if !noservice {
        run_service_gate(client, config, &wc.root)?;
    }
    stage_gate_changes(wc, &snapshot)?;
    let (mut entries, committed_files) = build_commit_entries(wc)?;
    if committed_files.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::NothingToCommit,
            "there is nothing to commit: the index matches the base",
        ));
    }
    let keeplink = index_has(wc, "_link")?;
    // One validate pass: learn what the server lacks and any sha256
    // challenges. A validate pass with a complete file list commits
    // — the server commits whenever nothing is missing, and
    // `withvalidate` only adds the sha validation — so the sequence
    // never sends a second one. When the validate pass answers with
    // a revision, the commit already landed: no staging, no final
    // pass. Otherwise, after staging, the final pass runs, and it
    // reports anything still missing itself instead of committing.
    let mut options = CommitFilelistOptions::new(message);
    options.validate = true;
    options.keeplink = keeplink;
    options.noservice = noservice;
    let answer = client
        .commit_filelist(&wc.project, &wc.package, &entries, &options, dry_run)?
        .ok_or_else(|| {
            GeectlError::new(
                ErrorKind::InternalError,
                "the commit file list returned no answer in live mode",
            )
        })?;
    let (mut new_rev, mut new_srcmd5, service_running) = match answer.rev {
        Some(rev) => {
            let srcmd5 = answer.srcmd5.ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    "the commit did not return a new source md5",
                )
            })?;
            (rev, srcmd5, answer.service_running)
        }
        None => {
            for name in &answer.hash_challenge {
                let content = index_content(wc, name)?;
                attach_sha256(&mut entries, name, &content);
            }
            // A staged PUT buffers one whole file in memory:
            // streaming PUTs need an `ObsClient` API change.
            for name in &answer.missing {
                let content = index_content(wc, name)?;
                client.put_file_staged_bytes(&wc.project, &wc.package, name, &content, dry_run)?;
            }
            // The final pass: no validation flag, the sha256 answers
            // attached, and no server-side services when `noservice` is
            // set. One revision for the whole file list. The final pass
            // reports files still missing instead of committing, so a
            // stage that raced is retried (three final passes at most).
            let mut attempts = 0;
            loop {
                attempts += 1;
                let mut options = CommitFilelistOptions::new(message);
                options.keeplink = keeplink;
                options.noservice = noservice;
                let answer = client
                    .commit_filelist(&wc.project, &wc.package, &entries, &options, dry_run)?
                    .ok_or_else(|| {
                        GeectlError::new(
                            ErrorKind::InternalError,
                            "the commit file list returned no answer in live mode",
                        )
                    })?;
                if answer.missing.is_empty() {
                    let rev = answer.rev.ok_or_else(|| {
                        GeectlError::new(
                            ErrorKind::ServerError,
                            "the commit did not return a new revision",
                        )
                    })?;
                    let srcmd5 = answer.srcmd5.ok_or_else(|| {
                        GeectlError::new(
                            ErrorKind::ServerError,
                            "the commit did not return a new source md5",
                        )
                    })?;
                    break (rev, srcmd5, answer.service_running);
                }
                if attempts >= 3 {
                    return Err(GeectlError::new(
                        ErrorKind::ServerError,
                        format!(
                            "the server still reports files missing after staging them: {}",
                            answer.missing.join(", ")
                        ),
                    ));
                }
                for name in &answer.missing {
                    let content = index_content(wc, name)?;
                    client.put_file_staged_bytes(
                        &wc.project,
                        &wc.package,
                        name,
                        &content,
                        dry_run,
                    )?;
                    attach_sha256(&mut entries, name, &content);
                }
            }
        }
    };
    if service_running {
        // A server-side service run is still in progress: poll
        // until it settles, since it can create a new revision
        // after the commit. A failed run refuses the base refresh:
        // the committed revision is not the settled state.
        let deadline = Instant::now() + POLL_TIMEOUT;
        loop {
            // A read failure inside the poll names the landed
            // revision too: the commit is on the server even
            // though its settlement could not be read.
            let state = client
                .service_state(&wc.project, &wc.package)
                .map_err(|e| {
                    GeectlError::new(
                        e.kind(),
                        format!(
                            "the commit landed as revision {new_rev}, but the service run could not be read: {e}; run update, then check status"
                        ),
                    )
                })?;
            match state {
                Some(state) if state.code == ServiceCode::Running => {
                    if Instant::now() >= deadline {
                        return Err(GeectlError::new(
                            ErrorKind::Timeout,
                            format!(
                                "the commit landed as revision {new_rev}, but the server-side service run did not settle within 60 seconds; run update, then check status"
                            ),
                        ));
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
                Some(state) if state.code == ServiceCode::Failed => {
                    return Err(GeectlError::new(
                        ErrorKind::ServiceFailed,
                        format!(
                            "the commit landed as revision {new_rev}, but the server-side service run failed: {}; run update, then check status",
                            state.error.unwrap_or_default()
                        ),
                    ));
                }
                _ => break,
            }
        }
        // The re-read names the landed revision on failure too:
        // the commit is on the server even though the settled
        // revision could not be read.
        let (rev, _, srcmd5) = client
            .directory_rev(&wc.project, &wc.package)
            .map_err(|e| {
                GeectlError::new(
                    e.kind(),
                    format!(
                        "the commit landed as revision {new_rev}, but the settled revision could not be read: {e}; run update, then check status"
                    ),
                )
            })?;
        new_rev = rev
            .ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    format!(
                        "the server did not return a revision for {}/{}",
                        wc.project, wc.package
                    ),
                )
            })?
            .parse::<u64>()
            .map_err(|_| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    format!(
                        "the server returned a non-numeric revision for {}/{}",
                        wc.project, wc.package
                    ),
                )
            })?;
        new_srcmd5 = srcmd5.ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "the server did not return a source md5 for {}/{}",
                    wc.project, wc.package
                ),
            )
        })?;
    }
    // The base refresh runs after the commit landed: a failure
    // here must name the landed revision, like the settle-poll
    // errors do — otherwise the agent cannot tell the commit
    // landed and retries it, creating a second revision.
    refresh_base(wc, &new_rev.to_string(), &new_srcmd5).map_err(|e| {
        GeectlError::new(
            e.kind(),
            format!(
                "the commit landed as revision {new_rev}, but the base could not be refreshed: {e}; run update, then check status"
            ),
        )
    })?;
    // The open-requests report is informational: the commit landed,
    // so a failing search does not fail the commit.
    let open_requests = client
        .open_requests_for_package(&wc.project, &wc.package)
        .unwrap_or_default();
    Ok(CommitReport {
        rev: Some(new_rev),
        srcmd5: Some(new_srcmd5),
        pushed_head: None,
        open_requests,
        files: committed_files,
    })
}

/// The git-backed commit: commit staged changes, push, and poll the
/// OBS side until the synced commit matches the pushed head. The
/// refusals run before the service gate, in order: the remote has
/// commits the checkout lacks, conflicts, nothing to commit. The
/// gate runs last of the checks, and its outputs are staged into
/// the index, so the commit sends them.
fn commit_git(
    wc: &mut WorkingCopy,
    client: &ObsClient,
    config: &Config,
    message: &str,
    noservice: bool,
    gitea_auth: Option<&GiteaAuth<'_>>,
) -> Result<CommitReport, GeectlError> {
    let auth = gitea_auth.ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ConfigError,
            "a git-backed commit needs a Gitea token: the push authenticates with it",
        )
    })?;
    let mut header = String::new();
    let mut key = String::new();
    let env = token_env(auth, &mut header, &mut key);
    // The outdated check: fetch first, then refuse when the
    // upstream has commits the checkout does not have.
    git_network(&wc.root, "fetch", &["origin"], &env)?;
    refuse_when_remote_ahead(wc)?;
    refuse_on_conflicts(wc)?;
    let staged = git(
        &wc.root,
        "diff",
        &[
            "--cached",
            "--no-renames",
            "--no-ext-diff",
            "--no-color",
            "--no-textconv",
            "--name-only",
            "-z",
            "HEAD",
        ],
        &[],
    )?;
    let head = git(&wc.root, "rev-parse", &["HEAD"], &[])?
        .trim()
        .to_string();
    if split_nul(&staged).is_empty() && head == wc.base_rev {
        return Err(GeectlError::new(
            ErrorKind::NothingToCommit,
            "there is nothing to commit: no local commits ahead of the base",
        ));
    }
    let snapshot = snapshot_gate_inputs(wc)?;
    if !noservice {
        run_service_gate(client, config, &wc.root)?;
    }
    stage_gate_changes(wc, &snapshot)?;
    // Staged changes become one synthesized commit. Unstaged
    // worktree changes are not sent: the commit sends the index
    // state plus the local commits, never the worktree.
    let staged = git(
        &wc.root,
        "diff",
        &[
            "--cached",
            "--no-renames",
            "--no-ext-diff",
            "--no-color",
            "--no-textconv",
            "--name-only",
            "-z",
            "HEAD",
        ],
        &[],
    )?;
    if !split_nul(&staged).is_empty() {
        git(
            &wc.root,
            "commit",
            &["--quiet", "--no-verify", "-m", message],
            &[],
        )?;
    }
    let head = git(&wc.root, "rev-parse", &["HEAD"], &[])?
        .trim()
        .to_string();
    if head == wc.base_rev {
        return Err(GeectlError::new(
            ErrorKind::NothingToCommit,
            "there is nothing to commit: no local commits ahead of the base",
        ));
    }
    git_network(&wc.root, "push", &["--no-verify", "origin", "HEAD"], &env)?;
    // The push landed; the OBS side picks it up through scmsync.
    // Poll `_scmsync.obsinfo` until its `commit:` matches the
    // pushed head, bounded: a sync that never arrives is a timeout,
    // not a successful commit. A read that fails (a 401 or 404
    // while the sync is pending) is kept, not swallowed: the
    // timeout names the last failure instead of hiding it.
    let deadline = Instant::now() + POLL_TIMEOUT;
    let mut last_err: Option<String>;
    loop {
        match client.cat(&wc.project, &wc.package, "_scmsync.obsinfo", None) {
            Ok(body) => {
                // A successful read clears the last error: a stale
                // one would otherwise name a read that recovered.
                last_err = None;
                if parse_obsinfo_commit(&body).as_deref() == Some(head.as_str()) {
                    break;
                }
            }
            Err(e) => last_err = Some(e.to_string()),
        }
        if Instant::now() >= deadline {
            return Err(GeectlError::new(
                ErrorKind::Timeout,
                match last_err {
                    Some(last) => format!(
                        "the OBS side did not sync the pushed head within 60 seconds; the last sync read failed: {last}"
                    ),
                    None => {
                        "the OBS side did not sync the pushed head within 60 seconds"
                            .to_string()
                    }
                },
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    let old_base = wc.base_rev.clone();
    // The push landed: a failure here must name the pushed head,
    // like the OBS path names the landed revision — otherwise the
    // agent cannot tell the push landed and pushes again.
    git_config_set(&wc.root, "base-rev", &head).map_err(|e| {
        GeectlError::new(
            e.kind(),
            format!(
                "the push landed (head {head}), but the base record could not be updated: {e}; run update, then check status"
            ),
        )
    })?;
    wc.base_rev = head.clone();
    let changed = git(
        &wc.root,
        "diff",
        &[
            "--name-only",
            "--no-renames",
            "--no-ext-diff",
            "--no-color",
            "--no-textconv",
            "-z",
            &old_base,
            "HEAD",
        ],
        &[],
    )?;
    Ok(CommitReport {
        rev: None,
        srcmd5: None,
        pushed_head: Some(head),
        open_requests: Vec::new(),
        files: split_nul(&changed),
    })
}

/// The `commit:` line of a `_scmsync.obsinfo` body
/// (`key: value` lines). `None` when the body carries no commit.
fn parse_obsinfo_commit(body: &str) -> Option<String> {
    body.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if key.trim() == "commit" {
            Some(value.trim().to_string())
        } else {
            None
        }
    })
}

/// Add a downloaded file to the temporary index backing a new base
/// tree.
fn add_to_tmp_index(
    root: &Path,
    index_env: &[(&str, &str)],
    dest: &Path,
    name: &str,
) -> Result<(), GeectlError> {
    let dest_str = dest.to_string_lossy().into_owned();
    let blob = git(root, "hash-object", &["-w", &dest_str], index_env)?;
    let blob = blob.trim();
    let cacheinfo = format!("100644,{blob},{name}");
    git(
        root,
        "update-index",
        &["--add", "--cacheinfo", &cacheinfo],
        index_env,
    )?;
    Ok(())
}

/// Build the base tree for an update or a pull: download every file
/// of `listing` into a temporary directory, hash each into the
/// object store, and write the tree into a temporary index. Returns
/// the tree SHA. `link_baserev` rewrites the `_link` file's baserev
/// before hashing (a pull moves the link to the target's latest
/// revision). The expanded listing never carries `_link`, so a pull
/// fetches it through the unexpanded route, pinned to `link_rev`:
/// the link package's own revision, not the expanded pin.
/// `link_rev` must be `Some` whenever `link_baserev` is `Some`.
fn build_base_tree(
    client: &ObsClient,
    wc: &WorkingCopy,
    listing: &DirectoryListing,
    rev: Option<&RevisionSpec>,
    link_rev: Option<&RevisionSpec>,
    link_baserev: Option<&str>,
    expand: bool,
) -> Result<String, GeectlError> {
    check_listing_names(listing)?;
    let git_dir = git(&wc.root, "rev-parse", &["--absolute-git-dir"], &[])?;
    let git_dir = PathBuf::from(git_dir.trim());
    let tmp_dir = git_dir.join("geectl-tmp");
    let tmp_index = git_dir.join("geectl-tmp-index");
    let _cleanup = TempCleanup {
        paths: vec![tmp_dir.clone(), tmp_index.clone()],
    };
    // A stale temporary directory from a killed run blocks the
    // next one: remove it (and the stale index) before creating
    // them, instead of only removing the index.
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let _ = std::fs::remove_file(&tmp_index);
    std::fs::create_dir_all(&tmp_dir).map_err(|e| {
        GeectlError::new(
            ErrorKind::InternalError,
            format!("failed to create {}: {e}", tmp_dir.display()),
        )
    })?;
    let tmp_index_str = tmp_index.to_string_lossy().into_owned();
    let index_env = [("GIT_INDEX_FILE", tmp_index_str.as_str())];
    git(&wc.root, "read-tree", &["--empty"], &index_env)?;
    // The base tree's blob SHAs, resolved once: a file whose
    // listing md5 matches the base blob is copied out of the
    // object store instead of re-downloaded.
    let base = wc.base_commit()?;
    let base_blobs: std::collections::HashMap<String, String> =
        ls_tree_entries(wc, &base)?.into_iter().collect();
    let mut batch = BlobBatch::new(&wc.root)?;
    for entry in &listing.entries {
        let dest = tmp_dir.join(&entry.name);
        let reused = match (&entry.md5, base_blobs.get(&entry.name)) {
            (Some(listing_md5), Some(sha)) => batch.md5(sha)? == *listing_md5,
            _ => false,
        };
        if reused {
            let sha = base_blobs.get(&entry.name).unwrap();
            git_blob_to_file(&wc.root, sha, &dest)?;
        } else {
            // An expanded listing names the link target's files:
            // they resolve only through the link expansion, so
            // they download through the authenticated source
            // endpoint. Either way the download is pinned to
            // `rev` and verified against the listing's md5.
            download_verified(
                client,
                &wc.project,
                &wc.package,
                &entry.name,
                rev,
                &dest,
                entry.md5.as_deref(),
                expand,
            )?;
        }
        add_to_tmp_index(&wc.root, &index_env, &dest, &entry.name)?;
    }
    if let Some(baserev) = link_baserev {
        // The expanded listing never carries `_link`: the server's
        // expanded tree is the link target's files plus the link
        // package's own files except `_link`. Fetch it through the
        // unexpanded route — it is the link package's own file —
        // and rewrite its baserev to the pulled target revision.
        let dest = tmp_dir.join("_link");
        // Pinned to the link package's own revision, not the
        // expanded pin: the expanded md5 names the expanded tree,
        // not a revision of the link package, so it cannot pin the
        // unexpanded `_link` read. No md5 to verify against — the
        // expanded listing never carries `_link`.
        download_verified(
            client,
            &wc.project,
            &wc.package,
            "_link",
            link_rev,
            &dest,
            None,
            false,
        )?;
        let text = std::fs::read_to_string(&dest).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("failed to read the downloaded _link file: {e}"),
            )
        })?;
        let mut doc = XmlDoc::parse(&text)?;
        doc.set_root_attr("baserev", baserev);
        std::fs::write(&dest, doc.render()).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("failed to write the downloaded _link file: {e}"),
            )
        })?;
        add_to_tmp_index(&wc.root, &index_env, &dest, "_link")?;
    }
    let tree = git(&wc.root, "write-tree", &[], &index_env)?;
    Ok(tree.trim().to_string())
}

/// The outcome of a rebase: the conflicts it stopped on (if
/// any), and whether the stash entry holding the pre-rebase
/// worktree changes is still there.
struct RebaseOutcome {
    conflicts: Option<Vec<String>>,
    stash_remaining: bool,
}

/// The `stash@{n}` refs of the `geectl autostash` entries, newest
/// first: `git stash list` orders newest first, so the first
/// match is the latest update's entry.
fn autostash_entries(wc: &WorkingCopy) -> Result<Vec<String>, GeectlError> {
    let list = git(&wc.root, "stash", &["list"], &[])?;
    let mut entries = Vec::new();
    for line in list.lines() {
        // `stash@{0}: On main: geectl autostash`: the message is
        // the last `: `-separated field.
        if line.rsplit(": ").next() == Some("geectl autostash") {
            if let Some(entry) = line.split(':').next() {
                entries.push(entry.to_string());
            }
        }
    }
    Ok(entries)
}

/// Rebase the local commits onto `new_root`, with the worktree
/// stashed and restored. A clean rebase returns no conflicts; a
/// rebase stopped on conflicts returns the conflicted files, and
/// a stash entry that could not be re-applied is reported
/// (`stash_remaining`): a rebase that exits 0 can still leave
/// unmerged files behind when the stash re-apply conflicts, so
/// exit 0 is never taken as clean on its own. The stash is done
/// by hand, not with `--autostash`: autostash restores without
/// `--index`, so staged changes come back unstaged and the staged
/// intent is lost; the pop below uses `--index`, which restores
/// staged changes as staged.
fn rebase_onto(wc: &WorkingCopy, new_root: &str) -> Result<RebaseOutcome, GeectlError> {
    let old_root = wc.base_commit()?;
    let stashes_before = autostash_entries(wc)?.len();
    let unstaged = git(
        &wc.root,
        "diff",
        &[
            "--name-only",
            "--no-ext-diff",
            "--no-color",
            "--no-textconv",
            "-z",
        ],
        &[],
    )?;
    let staged = git(
        &wc.root,
        "diff",
        &[
            "--cached",
            "--name-only",
            "--no-ext-diff",
            "--no-color",
            "--no-textconv",
            "-z",
            "HEAD",
        ],
        &[],
    )?;
    let stashed = !split_nul(&unstaged).is_empty() || !split_nul(&staged).is_empty();
    if stashed {
        git(&wc.root, "stash", &["push", "-m", "geectl autostash"], &[])?;
    }
    let (success, _, stderr) = git_status(
        &wc.root,
        "rebase",
        &["--no-verify", "--onto", new_root, &old_root],
        &[],
    )?;
    if !success {
        let conflicts = unmerged_files(wc)?;
        if conflicts.is_empty() {
            // The rebase failed without leaving conflicts — an
            // untracked file in the way of the new base, for
            // example. The stash still holds the pre-rebase
            // worktree changes: restore them (best effort) so the
            // failure does not strand them silently, and name the
            // stash in the error when the restore does not take.
            if stashed {
                if let Some(entry) = autostash_entries(wc)?.into_iter().next() {
                    let _ = git_status(&wc.root, "stash", &["pop", "--index", &entry], &[]);
                }
            }
            let mut err = git_error("rebase", stderr.as_bytes());
            if autostash_entries(wc)?.len() > stashes_before {
                err.context.push_str(
                    "; the stashed worktree changes could not be restored (they stay in the stash)",
                );
            }
            return Err(err);
        }
        return Ok(RebaseOutcome {
            conflicts: Some(conflicts),
            stash_remaining: autostash_entries(wc)?.len() > stashes_before,
        });
    }
    // The rebase finished: restore the stashed worktree changes
    // with `--index`, so staged changes stay staged.
    if stashed {
        if let Some(entry) = autostash_entries(wc)?.into_iter().next() {
            // A failed pop leaves the entry in the stash and the
            // conflicting files unmerged: both are reported below,
            // not swallowed.
            let _ = git_status(&wc.root, "stash", &["pop", "--index", &entry], &[]);
        }
    }
    let conflicts = unmerged_files(wc)?;
    let stash_remaining = autostash_entries(wc)?.len() > stashes_before;
    Ok(RebaseOutcome {
        conflicts: if conflicts.is_empty() {
            None
        } else {
            Some(conflicts)
        },
        stash_remaining,
    })
}

/// Update the working copy to the latest base (or to `rev`): the
/// new base tree is built, and the local commits are rebased onto
/// it. Conflicts stay in the tree: the report carries them, and the
/// rebase stays in progress until `resolve` finishes it. A
/// successful update updates this checkout's recorded base, on disk
/// and in the struct. Returns `None` in dry-run.
pub fn update(
    wc: &mut WorkingCopy,
    client: &ObsClient,
    rev: Option<&str>,
    gitea_auth: Option<&GiteaAuth<'_>>,
    dry_run: &DryRun,
) -> Result<Option<UpdateReport>, GeectlError> {
    if !dry_run.allow(&format!(
        "update working copy {}/{}",
        wc.project, wc.package
    )) {
        record_update_plan(wc, client, rev, dry_run)?;
        return Ok(None);
    }
    match wc.backend {
        Backend::Obs => update_obs(wc, client, rev).map(Some),
        Backend::Git => update_git(wc, rev, gitea_auth).map(Some),
    }
}

/// Note the would-change lines for a listing against the base:
/// `would change` for a file whose base content differs from the
/// listing's md5, `would add` for a file not in the base,
/// `would delete` for a base file the listing drops. Shared by
/// the update and pull dry-run plans. A file named in `retain` is
/// never reported as a delete: pull retains `_link`, which the
/// expanded listing never carries.
fn note_listing_changes(
    wc: &WorkingCopy,
    listing: &DirectoryListing,
    dry_run: &DryRun,
    retain: &[&str],
) -> Result<(), GeectlError> {
    let base = wc.base_commit()?;
    // The base tree's entries, resolved once: the SHAs feed the
    // md5 comparison, the names feed the delete check below.
    let base_blobs: std::collections::HashMap<String, String> =
        ls_tree_entries(wc, &base)?.into_iter().collect();
    let base_names: std::collections::HashSet<&str> =
        base_blobs.keys().map(String::as_str).collect();
    let listing_names: std::collections::HashSet<&str> = listing
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    // The base blob md5s stream through one `cat-file --batch`:
    // one spawn for the whole listing, not one `git show` (and
    // one base-commit re-resolution) per file.
    let mut batch = BlobBatch::new(&wc.root)?;
    for entry in &listing.entries {
        match base_blobs.get(&entry.name) {
            Some(sha) => {
                // The listing carries no md5 for this file: its
                // content cannot be compared without a download,
                // so no line is recorded.
                if let Some(server_md5) = &entry.md5 {
                    let md5 = batch.md5(sha)?;
                    if *server_md5 != md5 {
                        dry_run.note(&format!("would change {}", entry.name));
                    }
                }
            }
            None => {
                dry_run.note(&format!("would add {}", entry.name));
            }
        }
    }
    for name in base_names.difference(&listing_names) {
        // A retained file stays even though the listing drops it:
        // the expanded listing never carries `_link`, and pull
        // rewrites it instead of deleting it.
        if retain.contains(name) {
            continue;
        }
        dry_run.note(&format!("would delete {name}"));
    }
    Ok(())
}

/// The dry-run plan for an update: the new base revision, and the
/// files it would add, delete, or change, from the server listing's
/// md5s against the base content. A git-backed update lists the
/// upstream it would fetch and rebase onto.
fn record_update_plan(
    wc: &WorkingCopy,
    client: &ObsClient,
    rev: Option<&str>,
    dry_run: &DryRun,
) -> Result<(), GeectlError> {
    match wc.backend {
        Backend::Obs => {
            refuse_on_rebase(wc)?;
            let spec = match rev {
                Some(rev) => Some(RevisionSpec::parse(rev)?),
                None => None,
            };
            let (listing, (rev_number, _, srcmd5)) =
                client.directory_listing_rev(&wc.project, &wc.package, spec.as_ref())?;
            let rev_number = rev_number.ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    format!(
                        "the server did not return a revision for {}/{}",
                        wc.project, wc.package
                    ),
                )
            })?;
            let srcmd5 = srcmd5.ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    format!(
                        "the server did not return a source md5 for {}/{}",
                        wc.project, wc.package
                    ),
                )
            })?;
            dry_run.note(&format!(
                "would update {}/{} to revision {} ({})",
                wc.project, wc.package, rev_number, srcmd5
            ));
            if rev_number == wc.base_rev && srcmd5 == wc.base_srcmd5 {
                dry_run.note("already at the latest base: nothing to update");
                return Ok(());
            }
            note_listing_changes(wc, &listing, dry_run, &[])?;
            Ok(())
        }
        Backend::Git => {
            if rev.is_some() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "update --rev is not supported for a git-backed package",
                ));
            }
            refuse_on_rebase(wc)?;
            // The upstream SHA is not named: the plan does not
            // fetch, so the last-fetched head may not be the head
            // the fetch would rebase onto.
            dry_run.note("would fetch origin and rebase onto the upstream head");
            Ok(())
        }
    }
}

/// Refuse when a rebase is already in progress: a second update or
/// pull must not rebase on top of the stopped one. The agent runs
/// `resolve` to continue it first.
fn refuse_on_rebase(wc: &WorkingCopy) -> Result<(), GeectlError> {
    if rebase_in_progress(wc)? {
        return Err(GeectlError::new(
            ErrorKind::FileConflict,
            "a rebase is in progress in the working copy: run resolve to continue it first",
        ));
    }
    Ok(())
}

/// Refuse when files are unmerged: a commit must not send a tree
/// with unresolved conflicts.
fn refuse_on_conflicts(wc: &WorkingCopy) -> Result<(), GeectlError> {
    let conflicts = unmerged_files(wc)?;
    if !conflicts.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::FileConflict,
            format!(
                "the working copy has unresolved conflicts in: {}; resolve them and run resolve first",
                conflicts.join(", ")
            ),
        ));
    }
    refuse_on_rebase(wc)
}

/// The outdated check for an OBS-native checkout: the checkout's
/// base must equal the server's latest revision, else a commit
/// would land on top of a revision the checkout never saw.
fn refuse_when_outdated_obs(wc: &WorkingCopy, client: &ObsClient) -> Result<(), GeectlError> {
    let (rev, _, srcmd5) = client.directory_rev(&wc.project, &wc.package)?;
    let rev = rev.ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "the server did not return a revision for {}/{}",
                wc.project, wc.package
            ),
        )
    })?;
    let server_srcmd5 = srcmd5.ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "the server did not return a source md5 for {}/{}",
                wc.project, wc.package
            ),
        )
    })?;
    if rev != wc.base_rev || server_srcmd5 != wc.base_srcmd5 {
        return Err(GeectlError::new(
            ErrorKind::WorkingCopyOutdated,
            format!(
                "the checkout's base revision {}/{} is behind the server's {}/{}; run update first",
                wc.base_rev, wc.base_srcmd5, rev, server_srcmd5
            ),
        ));
    }
    Ok(())
}

/// Refuse when the remote has commits the checkout does not have,
/// judged against the last-fetched upstream: `@{u}` when it
/// resolves, and a checkout with no upstream skips the check.
/// `commit_git` fetches first, so its upstream is current; the
/// dry-run plan does not fetch, so it refuses only when the
/// checkout is behind what was last fetched.
fn refuse_when_remote_ahead(wc: &WorkingCopy) -> Result<(), GeectlError> {
    if let Ok(upstream) = git(&wc.root, "rev-parse", &["@{u}"], &[]) {
        let upstream = upstream.trim();
        let (is_ancestor, _, stderr) = git_status(
            &wc.root,
            "merge-base",
            &["--is-ancestor", upstream, "HEAD"],
            &[],
        )?;
        if !stderr.trim().is_empty() {
            return Err(git_error("merge-base", stderr.as_bytes()));
        }
        if !is_ancestor {
            return Err(GeectlError::new(
                ErrorKind::WorkingCopyOutdated,
                "the remote has commits the checkout does not have: run update first",
            ));
        }
    }
    Ok(())
}

/// The OBS-native update: the new base comes from the server's
/// listing at the requested or latest revision.
fn update_obs(
    wc: &mut WorkingCopy,
    client: &ObsClient,
    rev: Option<&str>,
) -> Result<UpdateReport, GeectlError> {
    refuse_on_rebase(wc)?;
    let spec = match rev {
        Some(rev) => Some(RevisionSpec::parse(rev)?),
        None => None,
    };
    let (listing, (rev_number, _, srcmd5)) =
        client.directory_listing_rev(&wc.project, &wc.package, spec.as_ref())?;
    let rev_number = rev_number.ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "the server did not return a revision for {}/{}",
                wc.project, wc.package
            ),
        )
    })?;
    let srcmd5 = srcmd5.ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "the server did not return a source md5 for {}/{}",
                wc.project, wc.package
            ),
        )
    })?;
    if rev_number == wc.base_rev && srcmd5 == wc.base_srcmd5 {
        return Ok(UpdateReport {
            updated: false,
            old_base_rev: wc.base_rev.clone(),
            new_base_rev: rev_number,
            old_base_srcmd5: wc.base_srcmd5.clone(),
            new_base_srcmd5: srcmd5,
            conflicts: Vec::new(),
            stash_remaining: false,
        });
    }
    // The new base tree is built at the listing's revision, not
    // at the requested (or latest) spec: a revision landing
    // between the listing and the download would otherwise be
    // recorded under the older rev.
    let pinned_rev = rev_number.parse::<u64>().map_err(|_| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "the server returned a non-numeric revision for {}/{}",
                wc.project, wc.package
            ),
        )
    })?;
    let pinned = RevisionSpec::Rev(pinned_rev);
    let tree = build_base_tree(client, wc, &listing, Some(&pinned), None, None, false)?;
    let new_root = git(
        &wc.root,
        "commit-tree",
        &[&tree, "-m", &format!("geectl base {rev_number} {srcmd5}")],
        &[],
    )?
    .trim()
    .to_string();
    let outcome = rebase_onto(wc, &new_root)?;
    match outcome.conflicts {
        None => {
            git_config_set(&wc.root, "base-rev", &rev_number)?;
            git_config_set(&wc.root, "base-srcmd5", &srcmd5)?;
            let report = UpdateReport {
                updated: true,
                old_base_rev: wc.base_rev.clone(),
                new_base_rev: rev_number.clone(),
                old_base_srcmd5: wc.base_srcmd5.clone(),
                new_base_srcmd5: srcmd5.clone(),
                conflicts: Vec::new(),
                stash_remaining: outcome.stash_remaining,
            };
            wc.base_rev = rev_number;
            wc.base_srcmd5 = srcmd5;
            Ok(report)
        }
        Some(conflicts) => Ok(UpdateReport {
            updated: false,
            old_base_rev: wc.base_rev.clone(),
            new_base_rev: rev_number,
            old_base_srcmd5: wc.base_srcmd5.clone(),
            new_base_srcmd5: srcmd5,
            conflicts,
            stash_remaining: outcome.stash_remaining,
        }),
    }
}

/// The git-backed update: fetch, then rebase onto the upstream
/// head. `--rev` is refused above: a git-backed package names no
/// server revision to update to.
fn update_git(
    wc: &mut WorkingCopy,
    rev: Option<&str>,
    gitea_auth: Option<&GiteaAuth<'_>>,
) -> Result<UpdateReport, GeectlError> {
    if rev.is_some() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "update --rev is not supported for a git-backed package",
        ));
    }
    refuse_on_rebase(wc)?;
    let mut header = String::new();
    let mut key = String::new();
    let env: Vec<(&str, &str)> = match gitea_auth {
        Some(auth) => token_env(auth, &mut header, &mut key).to_vec(),
        None => Vec::new(),
    };
    git_network(&wc.root, "fetch", &["origin"], &env)?;
    let target = match git(&wc.root, "rev-parse", &["@{u}"], &[]) {
        Ok(sha) => sha.trim().to_string(),
        Err(_) => {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "the working copy has no upstream branch to update from",
            ));
        }
    };
    if target == wc.base_rev {
        return Ok(UpdateReport {
            updated: false,
            old_base_rev: wc.base_rev.clone(),
            new_base_rev: target,
            old_base_srcmd5: wc.base_srcmd5.clone(),
            new_base_srcmd5: wc.base_srcmd5.clone(),
            conflicts: Vec::new(),
            stash_remaining: false,
        });
    }
    let outcome = rebase_onto(wc, &target)?;
    match outcome.conflicts {
        None => {
            git_config_set(&wc.root, "base-rev", &target)?;
            let report = UpdateReport {
                updated: true,
                old_base_rev: wc.base_rev.clone(),
                new_base_rev: target.clone(),
                old_base_srcmd5: wc.base_srcmd5.clone(),
                new_base_srcmd5: wc.base_srcmd5.clone(),
                conflicts: Vec::new(),
                stash_remaining: outcome.stash_remaining,
            };
            wc.base_rev = target;
            Ok(report)
        }
        Some(conflicts) => Ok(UpdateReport {
            updated: false,
            old_base_rev: wc.base_rev.clone(),
            new_base_rev: target,
            old_base_srcmd5: wc.base_srcmd5.clone(),
            new_base_srcmd5: wc.base_srcmd5.clone(),
            conflicts,
            stash_remaining: outcome.stash_remaining,
        }),
    }
}

/// The link a linked package points at: the target project and
/// package, and the baserev the link pins.
fn read_link(wc: &WorkingCopy) -> Result<(String, String, String), GeectlError> {
    let link_content = index_content(wc, "_link")?;
    let link_text = String::from_utf8_lossy(&link_content);
    let doc = XmlDoc::parse(&link_text)?;
    let (link_project, link_package) = link_target(&doc)?;
    let old_baserev = doc.root_attr("baserev").ok_or_else(|| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            "the link has no baserev: pull needs a pinned link revision",
        )
    })?;
    Ok((link_project, link_package, old_baserev))
}

/// The link target: the project and package a `_link` points at.
/// No baserev is required: a plain link (committed with
/// `keeplink`) has none, and the commit-time check only needs
/// the target.
fn read_link_target(wc: &WorkingCopy) -> Result<(String, String), GeectlError> {
    let link_content = index_content(wc, "_link")?;
    let link_text = String::from_utf8_lossy(&link_content);
    let doc = XmlDoc::parse(&link_text)?;
    link_target(&doc)
}

/// The project and package a parsed `_link` document points at.
fn link_target(doc: &XmlDoc) -> Result<(String, String), GeectlError> {
    let link_project = doc.root_attr("project").ok_or_else(|| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            "the _link file has no project attribute",
        )
    })?;
    let link_package = doc.root_attr("package").ok_or_else(|| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            "the _link file has no package attribute",
        )
    })?;
    Ok((link_project, link_package))
}

/// How long pull waits for a directory read to return a revision:
/// a read can come back without a `rev`. Separate from the
/// commit/service-settle poll budget by design: the pull wait is
/// its own budget, tuned independently.
const PULL_REVISION_TIMEOUT: Duration = Duration::from_secs(60);

/// How long pull waits between revision reads.
const PULL_REVISION_INTERVAL: Duration = Duration::from_secs(2);

/// Re-run `fetch` until it returns a revision: a directory read can
/// come back without a `rev`. Only a missing revision retries — a
/// fetch failure, and a missing anything else once a revision is
/// present, fail at once. Bounded by `timeout`: after it is spent
/// the error names the package and the wait, so a genuinely broken
/// link is not masked as a transient.
fn wait_for_revision<T>(
    mut fetch: impl FnMut() -> Result<(T, Option<String>), GeectlError>,
    project: &str,
    package: &str,
    timeout: Duration,
    interval: Duration,
) -> Result<(T, String), GeectlError> {
    let deadline = Instant::now() + timeout;
    loop {
        let (value, rev) = fetch()?;
        if let Some(rev) = rev {
            return Ok((value, rev));
        }
        if Instant::now() >= deadline {
            // A sub-second budget still names a nonzero wait:
            // `as_secs` reads 0 for one.
            let budget = if timeout >= Duration::from_secs(1) {
                format!("{} seconds", timeout.as_secs())
            } else {
                format!("{} milliseconds", timeout.as_millis())
            };
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "the server did not return a revision for {project}/{package} within {budget}"
                ),
            ));
        }
        std::thread::sleep(interval);
    }
}

/// The dry-run plan for a pull: the link baserev it would move to,
/// and the files the expanded listing would add, change, or delete.
fn record_pull_plan(
    wc: &WorkingCopy,
    client: &ObsClient,
    dry_run: &DryRun,
) -> Result<(), GeectlError> {
    let (link_project, link_package, old_baserev) = read_link(wc)?;
    let (_, _, target_srcmd5) = client.directory_rev(&link_project, &link_package)?;
    let new_baserev = target_srcmd5.ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("the server did not return a source md5 for {link_project}/{link_package}"),
        )
    })?;
    let ((_, own_srcmd5), own_rev) = wait_for_revision(
        || {
            client
                .directory_rev(&wc.project, &wc.package)
                .map(|(rev, vrev, srcmd5)| ((vrev, srcmd5), rev))
        },
        &wc.project,
        &wc.package,
        PULL_REVISION_TIMEOUT,
        PULL_REVISION_INTERVAL,
    )?;
    let own_srcmd5 = own_srcmd5.ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "the server did not return a source md5 for {}/{}",
                wc.project, wc.package
            ),
        )
    })?;
    dry_run.note(&format!(
        "would pull {}/{}: link baserev {} -> {}",
        wc.project, wc.package, old_baserev, new_baserev
    ));
    if new_baserev == old_baserev && own_rev == wc.base_rev && own_srcmd5 == wc.base_srcmd5 {
        dry_run.note("already at the latest base: nothing to pull");
        return Ok(());
    }
    let (listing, _) = client.directory_listing_expanded(&wc.project, &wc.package)?;
    note_listing_changes(wc, &listing, dry_run, &["_link"])
}

/// Pull a linked package: move the link to the target's latest
/// revision, expanding the link at the new base and rebasing the
/// local commits onto it. The working copy must be clean, and the
/// package must link another package with a baserev. A successful
/// pull updates this checkout's recorded base, on disk and in the
/// struct. Returns `None` in dry-run. A directory read that comes
/// back without a revision is waited for (bounded); only the
/// missing revision retries.
pub fn pull(
    wc: &mut WorkingCopy,
    client: &ObsClient,
    dry_run: &DryRun,
) -> Result<Option<PullReport>, GeectlError> {
    // The refusals run before the dry-run guard, so a dry-run pull
    // on a git-backed checkout or an unlinked package refuses
    // instead of planning a pull that live would never do.
    if wc.backend != Backend::Obs {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "pull is for linked OBS packages: this working copy is git-backed",
        ));
    }
    if !index_has(wc, "_link")? {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "the package is not linked: no _link file",
        ));
    }
    let (link_project, link_package, old_baserev) = read_link(wc)?;
    refuse_on_rebase(wc)?;
    // The clean-tree check must not honour user excludes:
    // `status --porcelain` hides untracked files the user's
    // excludes match, so the untracked half is read with plain
    // `ls-files --others` (no `--exclude-standard`). An ignored
    // file blocks the pull like any other untracked file: the
    // contract is a clean tree.
    let status_out = git(&wc.root, "status", &["--porcelain=v1", "-z"], &[])?;
    let untracked_out = git(&wc.root, "ls-files", &["--others", "-z"], &[])?;
    if !split_nul(&status_out).is_empty() || !split_nul(&untracked_out).is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "the working copy has uncommitted changes: commit or stash them before pull",
        ));
    }
    if !dry_run.allow(&format!("pull working copy {}/{}", wc.project, wc.package)) {
        record_pull_plan(wc, client, dry_run)?;
        return Ok(None);
    }
    let (_, _, target_srcmd5) = client.directory_rev(&link_project, &link_package)?;
    let new_baserev = target_srcmd5.ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("the server did not return a source md5 for {link_project}/{link_package}"),
        )
    })?;
    // The record keeps the link package's own, unexpanded
    // coordinates: the expanded listing's srcmd5 names the expanded
    // tree, and the next commit's outdated check reads the
    // unexpanded revision — storing the expanded srcmd5 would make
    // every commit after a pull refuse as outdated. The expanded
    // listing below is only for building the new base tree.
    let ((_, own_srcmd5), own_rev) = wait_for_revision(
        || {
            client
                .directory_rev(&wc.project, &wc.package)
                .map(|(rev, vrev, srcmd5)| ((vrev, srcmd5), rev))
        },
        &wc.project,
        &wc.package,
        PULL_REVISION_TIMEOUT,
        PULL_REVISION_INTERVAL,
    )?;
    let own_srcmd5 = own_srcmd5.ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "the server did not return a source md5 for {}/{}",
                wc.project, wc.package
            ),
        )
    })?;
    if new_baserev == old_baserev && own_rev == wc.base_rev && own_srcmd5 == wc.base_srcmd5 {
        // The link already points at the target's latest revision,
        // and the link package itself has not moved: nothing to pull.
        return Ok(Some(PullReport {
            updated: false,
            old_baserev,
            new_baserev,
            old_base_rev: wc.base_rev.clone(),
            new_base_rev: own_rev,
            conflicts: Vec::new(),
            stash_remaining: false,
        }));
    }
    // The expanded listing can come back without a revision: wait
    // for it, like the own-revision read above.
    let (listing, expanded_rev) = wait_for_revision(
        || {
            client
                .directory_listing_expanded(&wc.project, &wc.package)
                .map(|(listing, (rev, _, _))| (listing, rev))
        },
        &wc.project,
        &wc.package,
        PULL_REVISION_TIMEOUT,
        PULL_REVISION_INTERVAL,
    )?;
    // The expanded downloads are pinned to the expanded
    // listing's revision, like the update path pins to its
    // listing's revision: an unpinned pull would download files
    // the listing never named, so a listing with an unparsable
    // revision errors instead of pulling. The expanded revision
    // is the expanded content's md5 for a linked package, not a
    // number, so the pin is a revision spec, which accepts both.
    let pinned = RevisionSpec::parse(&expanded_rev).map_err(|_| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "the expanded listing for {}/{} carries an invalid revision: {expanded_rev:?}",
                wc.project, wc.package
            ),
        )
    })?;
    // The `_link` file is not in the expanded tree: it downloads
    // through the unexpanded route, pinned to the link package's
    // own revision — the expanded md5 names the expanded tree,
    // not a revision of the link package.
    let own_pinned = RevisionSpec::parse(&own_rev).map_err(|_| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "the server returned an invalid own revision for {}/{}: {own_rev:?}",
                wc.project, wc.package
            ),
        )
    })?;
    let tree = build_base_tree(
        client,
        wc,
        &listing,
        Some(&pinned),
        Some(&own_pinned),
        Some(&new_baserev),
        true,
    )?;
    let new_root = git(
        &wc.root,
        "commit-tree",
        &[&tree, "-m", &format!("geectl base {own_rev} {own_srcmd5}")],
        &[],
    )?
    .trim()
    .to_string();
    let outcome = rebase_onto(wc, &new_root)?;
    match outcome.conflicts {
        None => {
            git_config_set(&wc.root, "base-rev", &own_rev)?;
            git_config_set(&wc.root, "base-srcmd5", &own_srcmd5)?;
            let report = PullReport {
                updated: true,
                old_baserev,
                new_baserev,
                old_base_rev: wc.base_rev.clone(),
                new_base_rev: own_rev.clone(),
                conflicts: Vec::new(),
                stash_remaining: outcome.stash_remaining,
            };
            wc.base_rev = own_rev;
            wc.base_srcmd5 = own_srcmd5;
            Ok(Some(report))
        }
        Some(conflicts) => Ok(Some(PullReport {
            updated: false,
            old_baserev,
            new_baserev,
            old_base_rev: wc.base_rev.clone(),
            new_base_rev: own_rev,
            conflicts,
            stash_remaining: outcome.stash_remaining,
        })),
    }
}

/// The commit a stopped rebase is rebasing onto, from the rebase
/// state directory (`rebase-merge/onto` or `rebase-apply/onto`).
/// `None` when no rebase is in progress.
fn rebase_target(wc: &WorkingCopy) -> Result<Option<String>, GeectlError> {
    let git_dir = git(&wc.root, "rev-parse", &["--absolute-git-dir"], &[])?;
    let git_dir = Path::new(git_dir.trim());
    for state in ["rebase-merge", "rebase-apply"] {
        let onto = git_dir.join(state).join("onto");
        if onto.is_file() {
            let sha = std::fs::read_to_string(&onto)
                .map_err(|e| {
                    GeectlError::new(
                        ErrorKind::InternalError,
                        format!("failed to read {}: {e}", onto.display()),
                    )
                })?
                .trim()
                .to_string();
            if sha.is_empty() {
                return Err(GeectlError::new(
                    ErrorKind::InternalError,
                    format!("the rebase state in {} records no target", onto.display()),
                ));
            }
            return Ok(Some(sha));
        }
    }
    Ok(None)
}

/// Whether the worktree file `path` still contains conflict
/// markers: a line starting with `<<<<<<<` or `>>>>>>>`. A lone
/// line that is exactly `=======` is not a marker: a conflicted
/// file that legitimately contains such a line (a separator, a
/// test fixture) must still resolve. Read as bytes, streamed: the
/// file may be large, and may not be UTF-8. A missing file holds
/// no markers (a deletion in a conflict is a resolution).
fn file_has_conflict_markers(root: &Path, path: &str) -> Result<bool, GeectlError> {
    let full = root.join(path);
    let Ok(file) = std::fs::File::open(&full) else {
        return Ok(false);
    };
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("failed to read {}: {e}", full.display()),
            )
        })?;
        if n == 0 {
            break;
        }
        let mut line = line.as_slice();
        if line.ends_with(b"\n") {
            line = &line[..line.len() - 1];
        }
        if line.ends_with(b"\r") {
            line = &line[..line.len() - 1];
        }
        if line.starts_with(b"<<<<<<<") || line.starts_with(b">>>>>>>") {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Refuse to stage files that still contain conflict markers:
/// the markers would end up in base history and be sent on the
/// next commit. The agent edits the file to remove them, then
/// runs `resolve` again.
fn refuse_on_conflict_markers(wc: &WorkingCopy, files: &[String]) -> Result<(), GeectlError> {
    let mut marked = Vec::new();
    for file in files {
        if file_has_conflict_markers(&wc.root, file)? {
            marked.push(file.clone());
        }
    }
    if marked.is_empty() {
        return Ok(());
    }
    Err(GeectlError::new(
        ErrorKind::FileConflict,
        format!(
            "these files still contain conflict markers (<<<<<<< / ======= / >>>>>>>): {}; edit them to remove the markers, then run resolve again",
            marked.join(", ")
        ),
    ))
}

/// Restore a `geectl autostash` entry left by an update whose
/// rebase stopped: the entry holds the pre-update worktree
/// changes. Popped with `--index`, so staged changes stay
/// staged. A pop that conflicts leaves the entry in the stash:
/// that is an error naming the stash, not a silent loss.
fn restore_autostash(wc: &WorkingCopy) -> Result<(), GeectlError> {
    let Some(entry) = autostash_entries(wc)?.into_iter().next() else {
        return Ok(());
    };
    let (pop_ok, _, pop_stderr) = git_status(&wc.root, "stash", &["pop", "--index", &entry], &[])?;
    if pop_ok {
        return Ok(());
    }
    let still = unmerged_files(wc)?;
    if still.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InternalError,
            format!(
                "the stashed worktree changes could not be restored (they stay in the stash as {entry}): {}",
                bounded(&pop_stderr).trim()
            ),
        ));
    }
    Err(GeectlError::new(
        ErrorKind::FileConflict,
        format!(
            "the stashed worktree changes could not be restored (they stay in the stash as {entry}): {}; unresolved conflicts remain in: {}",
            bounded(&pop_stderr).trim(),
            still.join(", ")
        ),
    ))
}

/// The dry-run plan for a resolve: the files it would stage, and
/// whether it would continue the rebase.
fn record_resolve_plan(wc: &WorkingCopy, dry_run: &DryRun) -> Result<(), GeectlError> {
    let before = unmerged_files(wc)?;
    let in_progress = rebase_in_progress(wc)?;
    if before.is_empty() && !in_progress {
        dry_run.note("nothing to resolve: no conflicts and no rebase in progress");
        return Ok(());
    }
    // The plan refuses what live would refuse: markers still in
    // the tree are not stageable.
    refuse_on_conflict_markers(wc, &before)?;
    for file in &before {
        dry_run.note(&format!("would stage {file}"));
    }
    if in_progress {
        dry_run.note("would continue the rebase");
    }
    Ok(())
}

/// Resolve the conflicts a stopped rebase left in the tree: stage
/// the resolved files, and continue the rebase when every conflict
/// is resolved. When the resolutions are already staged (the agent
/// added them by hand), there is nothing to stage and the rebase is
/// continued directly. A successful continue adopts the new base,
/// on disk and in this checkout: the record stops pointing at the
/// old base. Returns the files that were resolved, `None` in
/// dry-run. A conflict still in the tree after staging is an
/// error naming the files.
pub fn resolve(wc: &mut WorkingCopy, dry_run: &DryRun) -> Result<Option<Vec<String>>, GeectlError> {
    if !dry_run.allow(&format!("resolve conflicts in {}", wc.root.display())) {
        record_resolve_plan(wc, dry_run)?;
        return Ok(None);
    }
    let before = unmerged_files(wc)?;
    let in_progress = rebase_in_progress(wc)?;
    if before.is_empty() && !in_progress {
        // No conflicts and no rebase — but an autostash entry
        // from an earlier stopped update may still be waiting:
        // restore it now, instead of leaving it stranded.
        restore_autostash(wc)?;
        return Ok(Some(Vec::new()));
    }
    if !before.is_empty() {
        // A file that still holds conflict markers is not a
        // resolution: staging it would send the markers.
        refuse_on_conflict_markers(wc, &before)?;
        // Stage only the files that were conflicted: unrelated
        // unstaged edits stay unstaged, and the next commit sends the
        // index state only.
        let mut args = vec!["--"];
        args.extend(before.iter().map(String::as_str));
        // Literal pathspecs: a resolved path is a path, never a
        // glob.
        git_literal(&wc.root, "add", &args, &[])?;
        let remaining = unmerged_files(wc)?;
        if !remaining.is_empty() {
            return Err(GeectlError::new(
                ErrorKind::FileConflict,
                format!("unresolved conflicts remain in: {}", remaining.join(", ")),
            ));
        }
    }
    if in_progress {
        // The target the rebase is onto is read before the
        // continue: the state directory is gone once the rebase
        // finishes, and it is the new base for a git-backed
        // checkout.
        let target = rebase_target(wc)?.ok_or_else(|| {
            GeectlError::new(
                ErrorKind::InternalError,
                "the rebase is in progress but records no target",
            )
        })?;
        // `git rebase --continue` refuses when the worktree has
        // unstaged modifications to tracked files, even ones
        // unrelated to the conflict. Stash the unstaged changes
        // — keeping the staged resolutions in the index —
        // continue, then restore them.
        let unstaged = git(
            &wc.root,
            "diff",
            &[
                "--name-only",
                "--no-ext-diff",
                "--no-color",
                "--no-textconv",
                "-z",
            ],
            &[],
        )?;
        let stashed = !split_nul(&unstaged).is_empty();
        if stashed {
            git(
                &wc.root,
                "stash",
                &["push", "--keep-index", "-m", "geectl resolve"],
                &[],
            )?;
        }
        let (success, _, stderr) =
            // `git rebase --continue` rejects `--no-verify`
            // alongside the action flag; the fixed `-c
            // core.hooksPath=/dev/null` already keeps hooks out.
            git_status(&wc.root, "rebase", &["--continue"], &[])?;
        if !success {
            // Restore the agent's worktree state before reporting
            // the failure. A pop that fails strands the edits in
            // the stash: that is reported, not swallowed — the
            // agent would otherwise only see the conflict and
            // never know the stash entry is still there.
            let pop_failed = if stashed {
                git(&wc.root, "stash", &["pop"], &[]).is_err()
            } else {
                false
            };
            let still = unmerged_files(wc)?;
            if pop_failed && !still.is_empty() {
                return Err(GeectlError::new(
                    ErrorKind::InternalError,
                    format!(
                        "git rebase --continue failed: {}; the rebase is still stopped with unresolved conflicts in: {}; the stashed worktree changes could not be restored (they stay in the stash)",
                        bounded(&stderr).trim(),
                        still.join(", ")
                    ),
                ));
            }
            if pop_failed {
                return Err(GeectlError::new(
                    ErrorKind::InternalError,
                    format!(
                        "git rebase --continue failed: {}; the stashed worktree changes could not be restored (they stay in the stash)",
                        bounded(&stderr).trim()
                    ),
                ));
            }
            if still.is_empty() {
                // The continue failed but no file is unmerged:
                // the failure is git's, classified like every
                // other git failure — the same state as the
                // conflicts-remain case below is never reported
                // under two kinds.
                return Err(git_error("rebase --continue", stderr.as_bytes()));
            }
            return Err(GeectlError::new(
                ErrorKind::FileConflict,
                format!("unresolved conflicts remain in: {}", still.join(", ")),
            ));
        }
        // The rebase finished: adopt the new base now, before
        // restoring the stashed worktree changes — the base move
        // is done regardless of whether the pop succeeds. For OBS
        // the new root commit's message carries the coordinates;
        // for Git the rebase target is the new base (the upstream
        // head the rebase was onto).
        match wc.backend {
            Backend::Obs => {
                let root = root_commit(&wc.root)?;
                let message = root_message(&wc.root, &root)?;
                let (rev, srcmd5) = parse_base_message(&message).ok_or_else(|| {
                    GeectlError::new(
                        ErrorKind::WorkingCopyInconsistent,
                        "the working copy root commit is not a geectl base commit",
                    )
                })?;
                git_config_set(&wc.root, "base-rev", &rev)?;
                git_config_set(&wc.root, "base-srcmd5", &srcmd5)?;
                wc.base_rev = rev;
                wc.base_srcmd5 = srcmd5;
            }
            Backend::Git => {
                git_config_set(&wc.root, "base-rev", &target)?;
                wc.base_rev = target;
            }
        }
        if stashed {
            let (pop_ok, _, pop_stderr) = git_status(&wc.root, "stash", &["pop"], &[])?;
            if !pop_ok {
                return Err(GeectlError::new(
                    ErrorKind::InternalError,
                    format!(
                        "the rebase finished, but the stashed worktree changes could not be restored (they stay in the stash): {}",
                        bounded(&pop_stderr).trim()
                    ),
                ));
            }
        }
    }
    // An update whose rebase stopped left the pre-update worktree
    // changes in the `geectl autostash` entry: restore them now
    // that the rebase is done (or was never in progress).
    restore_autostash(wc)?;
    Ok(Some(before))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obs::test_support::{basic_credentials, FakeHttp};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A temp directory removed on drop.
    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(name: &str) -> Self {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir()
                .join(format!("geectl-wc-test-{}-{name}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDirGuard { path }
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn git_ok(dir: &Path, args: &[&str]) {
        git(dir, args[0], &args[1..], &[]).unwrap();
    }

    fn write_file(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    fn read_file(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(name)).unwrap()
    }

    fn init_repo(dir: &Path) {
        git(dir, "init", &["--quiet"], &[]).unwrap();
        git(dir, "config", &["--local", "user.name", "geectl"], &[]).unwrap();
        git(
            dir,
            "config",
            &["--local", "user.email", "geectl@localhost"],
            &[],
        )
        .unwrap();
    }

    /// A recorded OBS-native checkout: a root commit with the base
    /// message, and the `geectl.*` record.
    fn record_obs(dir: &Path, rev: &str, srcmd5: &str) {
        write_record(
            dir,
            "https://api.opensuse.org",
            "home:alice",
            "foo",
            Backend::Obs,
            rev,
            srcmd5,
        )
        .unwrap();
    }

    fn base_commit_with(dir: &Path, rev: &str, srcmd5: &str) {
        write_file(dir, "foo.spec", "Name: foo\n");
        git_ok(dir, &["add", "-A"]);
        git_ok(
            dir,
            &[
                "commit",
                "--quiet",
                "-m",
                &format!("geectl base {rev} {srcmd5}"),
            ],
        );
        record_obs(dir, rev, srcmd5);
    }

    const REV1: &str = "1";
    const SRCMD5_1: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SRCMD5_2: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn listing_rev1() -> String {
        format!(
            "<directory name=\"foo\" rev=\"1\" srcmd5=\"{SRCMD5_1}\" vrev=\"1\">\
            <entry md5=\"9444ad22a238286bca9bf649b2aef01d\" mtime=\"1282047302\" name=\"foo.spec\" size=\"10\" />\
            <entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" mtime=\"1282047302\" name=\"foo.tar.gz\" size=\"0\" />\
            </directory>"
        )
    }

    fn listing_rev2() -> String {
        format!(
            "<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\">\
            <entry md5=\"d3fed47ff119d916236560718c57971c\" mtime=\"1282047303\" name=\"foo.spec\" size=\"13\" />\
            <entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" mtime=\"1282047302\" name=\"foo.tar.gz\" size=\"0\" />\
            </directory>"
        )
    }

    fn checkout_http() -> FakeHttp {
        FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo/_meta",
                "<package name=\"foo\" project=\"home:alice\"></package>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo",
                &listing_rev1(),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=1",
                b"Name: foo\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=1",
                b"",
            )
    }

    fn do_checkout(http: &FakeHttp) -> (TempDirGuard, WorkingCopy) {
        let dir = TempDirGuard::new("checkout");
        let client = ObsClient::with_credentials(http, basic_credentials()).unwrap();
        let wc = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        (dir, wc)
    }

    /// A checkout whose package holds `g.txt` (three lines) plus
    /// the usual `foo.spec` and `foo.tar.gz`: revision 2 changes
    /// only the first line of `g.txt`.
    /// The six-line twin of `gtxt_http`: the local commits and
    /// the remote change touch lines far enough apart (1 and 5)
    /// that the rebase merges them instead of conflicting.
    fn gtxt6_http() -> FakeHttp {
        // The listing carries the real md5s: the rev-1 `g.txt`
        // is "one\ntwo\nthree\nfour\nfive\nsix\n", the rev-2
        // one is "uno\ntwo\nthree\nfour\nfive\nsix\n".
        let listing = |rev: &str, srcmd5: &str, gtxt_md5: &str| {
            format!(
                "<directory name=\"foo\" rev=\"{rev}\" srcmd5=\"{srcmd5}\" vrev=\"{rev}\">\
                <entry md5=\"9444ad22a238286bca9bf649b2aef01d\" mtime=\"1282047302\" name=\"foo.spec\" size=\"10\" />\
                <entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" mtime=\"1282047302\" name=\"foo.tar.gz\" size=\"0\" />\
                <entry md5=\"{gtxt_md5}\" mtime=\"1282047302\" name=\"g.txt\" size=\"28\" />\
                </directory>"
            )
        };
        FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo/_meta",
                "<package name=\"foo\" project=\"home:alice\"></package>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo",
                &listing("1", SRCMD5_1, "b256e7fa23f105539085c7c895557c41"),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=1",
                b"Name: foo\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=1",
                b"",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/g.txt?rev=1",
                b"one\ntwo\nthree\nfour\nfive\nsix\n",
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
                &listing("2", SRCMD5_2, "779daf71cbd68fa63f678839b4f16b5e"),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=2",
                b"Name: foo\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=2",
                b"",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/g.txt?rev=2",
                b"uno\ntwo\nthree\nfour\nfive\nsix\n",
            )
    }

    fn gtxt_http() -> FakeHttp {
        // The listing carries the real md5s: the rev-1 `g.txt`
        // is "one\ntwo\nthree\n", the rev-2 one is
        // "uno\ntwo\nthree\n".
        let listing = |rev: &str, srcmd5: &str, gtxt_md5: &str| {
            format!(
                "<directory name=\"foo\" rev=\"{rev}\" srcmd5=\"{srcmd5}\" vrev=\"{rev}\">\
                <entry md5=\"9444ad22a238286bca9bf649b2aef01d\" mtime=\"1282047302\" name=\"foo.spec\" size=\"10\" />\
                <entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" mtime=\"1282047302\" name=\"foo.tar.gz\" size=\"0\" />\
                <entry md5=\"{gtxt_md5}\" mtime=\"1282047302\" name=\"g.txt\" size=\"14\" />\
                </directory>"
            )
        };
        FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo/_meta",
                "<package name=\"foo\" project=\"home:alice\"></package>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo",
                &listing("1", SRCMD5_1, "deed54b823522e0525693b090363f9df"),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=1",
                b"Name: foo\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=1",
                b"",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/g.txt?rev=1",
                b"one\ntwo\nthree\n",
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
                &listing("2", SRCMD5_2, "61ff98d33c7c7c175d294d25cb949b7b"),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=2",
                b"Name: foo\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=2",
                b"",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/g.txt?rev=2",
                b"uno\ntwo\nthree\n",
            )
    }

    #[test]
    fn open_rejects_a_plain_repo() {
        let dir = TempDirGuard::new("plain");
        init_repo(&dir.path);
        write_file(&dir.path, "foo.spec", "Name: foo\n");
        let err = open(&dir.path).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::WorkingCopyNotFound);
    }

    #[test]
    fn open_accepts_a_recorded_checkout() {
        let dir = TempDirGuard::new("recorded");
        init_repo(&dir.path);
        base_commit_with(&dir.path, REV1, SRCMD5_1);
        let wc = open(&dir.path).unwrap();
        assert_eq!(wc.project, "home:alice");
        assert_eq!(wc.package, "foo");
        assert_eq!(wc.backend, Backend::Obs);
        assert_eq!(wc.base_rev, REV1);
        assert_eq!(wc.base_srcmd5, SRCMD5_1);
    }

    #[test]
    fn open_heals_a_stale_record_from_the_root_commit() {
        let dir = TempDirGuard::new("heal");
        init_repo(&dir.path);
        base_commit_with(&dir.path, REV1, SRCMD5_1);
        // The record says revision 2; the root commit says 1. The
        // root is the history, so the record follows it.
        git_config_set(&dir.path, "base-rev", "2").unwrap();
        let wc = open(&dir.path).unwrap();
        assert_eq!(wc.base_rev, REV1);
        assert_eq!(
            git_config_get(&dir.path, "base-rev").unwrap(),
            Some("1".to_string())
        );
    }

    #[test]
    fn open_picks_the_base_root_in_a_two_root_history() {
        let http = checkout_http();
        let (dir, wc) = do_checkout(&http);
        let branch = git(&wc.root, "branch", &["--show-current"], &[])
            .unwrap()
            .trim()
            .to_string();
        // A second, unrelated root: an orphan commit merged in.
        git_ok(&wc.root, &["checkout", "--quiet", "--orphan", "other"]);
        write_file(&wc.root, "other.txt", "other\n");
        git_ok(&wc.root, &["add", "other.txt"]);
        git_ok(&wc.root, &["commit", "--quiet", "-m", "unrelated root"]);
        git_ok(&wc.root, &["checkout", "--quiet", &branch]);
        git_ok(
            &wc.root,
            &["merge", "--quiet", "--allow-unrelated-histories", "other"],
        );
        // Two roots now; the base is still the geectl base commit,
        // whichever order rev-list returns them in.
        let roots = git(&wc.root, "rev-list", &["--max-parents=0", "HEAD"], &[]).unwrap();
        assert_eq!(roots.lines().count(), 2);
        let reopened = open(&wc.root).unwrap();
        assert_eq!(reopened.base_rev, REV1);
        assert_eq!(reopened.base_srcmd5, SRCMD5_1);
        let _ = dir;
    }

    #[test]
    fn open_rejects_a_root_commit_that_is_not_a_base_commit() {
        let dir = TempDirGuard::new("badroot");
        init_repo(&dir.path);
        write_file(&dir.path, "foo.spec", "Name: foo\n");
        git_ok(&dir.path, &["add", "-A"]);
        git_ok(&dir.path, &["commit", "--quiet", "-m", "not a base commit"]);
        record_obs(&dir.path, REV1, SRCMD5_1);
        let err = open(&dir.path).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::WorkingCopyInconsistent);
    }

    #[test]
    fn status_maps_porcelain_codes() {
        let dir = TempDirGuard::new("status");
        init_repo(&dir.path);
        for (name, content) in [
            ("a.txt", "alpha\n"),
            ("b.txt", "bravo\n"),
            ("c.txt", "charlie\n"),
            ("d.txt", "delta\n"),
        ] {
            write_file(&dir.path, name, content);
        }
        git_ok(&dir.path, &["add", "-A"]);
        git_ok(
            &dir.path,
            &[
                "commit",
                "--quiet",
                "-m",
                &format!("geectl base {REV1} {SRCMD5_1}"),
            ],
        );
        record_obs(&dir.path, REV1, SRCMD5_1);
        let wc = open(&dir.path).unwrap();
        write_file(&dir.path, "a.txt", "changed\n");
        git_ok(&dir.path, &["rm", "--quiet", "b.txt"]);
        std::fs::remove_file(dir.path.join("c.txt")).unwrap();
        git_ok(&dir.path, &["mv", "d.txt", "d2.txt"]);
        write_file(&dir.path, "e.txt", "new\n");
        git_ok(&dir.path, &["add", "e.txt"]);
        let mut found: Vec<(String, StatusCode)> = status(&wc)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.path, entry.code))
            .collect();
        found.sort();
        assert_eq!(
            found,
            vec![
                ("a.txt".to_string(), StatusCode::Modified),
                ("b.txt".to_string(), StatusCode::Deleted),
                ("c.txt".to_string(), StatusCode::Missing),
                ("d2.txt".to_string(), StatusCode::Renamed),
                ("e.txt".to_string(), StatusCode::Added),
            ]
        );
    }

    #[test]
    fn status_lists_a_cached_deletion_once() {
        let dir = TempDirGuard::new("status-rmcached");
        init_repo(&dir.path);
        base_commit_with(&dir.path, REV1, SRCMD5_1);
        let wc = open(&dir.path).unwrap();
        // Removed from the index but still on disk: the cached
        // diff says Deleted and `ls-files --others` also names
        // the file. It is one entry, as the staged state.
        git_ok(&dir.path, &["rm", "--quiet", "--cached", "foo.spec"]);
        let found: Vec<(String, StatusCode)> = status(&wc)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.path, entry.code))
            .collect();
        assert_eq!(found, vec![("foo.spec".to_string(), StatusCode::Deleted)]);
    }

    #[test]
    fn diff_shows_tracked_changes_and_lists_untracked() {
        let dir = TempDirGuard::new("diff");
        init_repo(&dir.path);
        base_commit_with(&dir.path, REV1, SRCMD5_1);
        let wc = open(&dir.path).unwrap();
        write_file(&dir.path, "foo.spec", "Name: bar\n");
        write_file(&dir.path, "u.txt", "untracked\n");
        let out = diff(&wc).unwrap();
        assert!(out.contains("Name: foo"));
        assert!(out.contains("Name: bar"));
        assert!(out.contains("Untracked:\nu.txt"));
    }

    #[test]
    fn diff_shows_staged_only_changes() {
        let dir = TempDirGuard::new("diff-staged");
        init_repo(&dir.path);
        base_commit_with(&dir.path, REV1, SRCMD5_1);
        let wc = open(&dir.path).unwrap();
        write_file(&dir.path, "foo.spec", "Name: bar\n");
        git_ok(&dir.path, &["add", "foo.spec"]);
        // The worktree goes back to the base content: a plain
        // `git diff <base>` is empty here, and only the staged
        // half shows the change.
        write_file(&dir.path, "foo.spec", "Name: foo\n");
        let out = diff(&wc).unwrap();
        assert!(out.contains("Name: bar"), "diff: {out}");
        let found: Vec<(String, StatusCode)> = status(&wc)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.path, entry.code))
            .collect();
        assert_eq!(found, vec![("foo.spec".to_string(), StatusCode::Modified)]);
    }

    #[test]
    fn diff_shows_a_staged_new_file_deleted_from_the_worktree() {
        let dir = TempDirGuard::new("diff-staged-new");
        init_repo(&dir.path);
        base_commit_with(&dir.path, REV1, SRCMD5_1);
        let wc = open(&dir.path).unwrap();
        write_file(&dir.path, "new.txt", "new\n");
        git_ok(&dir.path, &["add", "new.txt"]);
        std::fs::remove_file(dir.path.join("new.txt")).unwrap();
        let out = diff(&wc).unwrap();
        assert!(out.contains("new.txt"), "diff: {out}");
        let found: Vec<(String, StatusCode)> = status(&wc)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.path, entry.code))
            .collect();
        assert_eq!(found, vec![("new.txt".to_string(), StatusCode::Added)]);
    }

    #[test]
    fn add_rm_addremove_stage_changes() {
        let dir = TempDirGuard::new("addrm");
        init_repo(&dir.path);
        base_commit_with(&dir.path, REV1, SRCMD5_1);
        let wc = open(&dir.path).unwrap();
        write_file(&dir.path, "new.txt", "new\n");
        add(&wc, &["new.txt".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        let codes: Vec<StatusCode> = status(&wc)
            .unwrap()
            .into_iter()
            .map(|entry| entry.code)
            .collect();
        assert_eq!(codes, vec![StatusCode::Added]);
        rm(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(!dir.path.join("foo.spec").exists());
        // A fresh untracked file plus a worktree deletion: addremove
        // stages the new file; the staged-but-never-committed
        // new.txt just leaves the index when deleted from the
        // worktree (it never existed in the base).
        write_file(&dir.path, "other.txt", "other\n");
        std::fs::remove_file(dir.path.join("new.txt")).unwrap();
        addremove(&wc, &DryRun::new(false)).unwrap().unwrap();
        let mut found: Vec<(String, StatusCode)> = status(&wc)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.path, entry.code))
            .collect();
        found.sort();
        assert_eq!(
            found,
            vec![
                ("foo.spec".to_string(), StatusCode::Deleted),
                ("other.txt".to_string(), StatusCode::Added),
            ]
        );
    }

    #[test]
    fn addremove_stages_a_renamed_then_deleted_file_as_a_deletion() {
        let dir = TempDirGuard::new("addrm-rename");
        init_repo(&dir.path);
        base_commit_with(&dir.path, REV1, SRCMD5_1);
        let wc = open(&dir.path).unwrap();
        // A staged rename whose new path is deleted from the
        // worktree: `status` reports the new path as `Renamed`
        // (the cached diff wins over the worktree deletion), but
        // `addremove` stages it as a deletion — otherwise the
        // next commit would send a file that no longer exists
        // while deleting the old path.
        git_ok(&dir.path, &["mv", "foo.spec", "bar.spec"]);
        std::fs::remove_file(dir.path.join("bar.spec")).unwrap();
        addremove(&wc, &DryRun::new(false)).unwrap().unwrap();
        let mut found: Vec<(String, StatusCode)> = status(&wc)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.path, entry.code))
            .collect();
        found.sort();
        assert_eq!(found, vec![("foo.spec".to_string(), StatusCode::Deleted)]);
    }

    #[test]
    fn parse_obsinfo_commit_reads_the_commit_line() {
        let body = "mtime: 1763900000\ncommit: abcdef1234567890abcdef1234567890abcdef12\nurl: https://code.opensuse.org/foo\nrevision: main\n";
        assert_eq!(
            parse_obsinfo_commit(body),
            Some("abcdef1234567890abcdef1234567890abcdef12".to_string())
        );
        assert_eq!(parse_obsinfo_commit("mtime: 1\n"), None);
    }

    #[test]
    fn checkout_obs_downloads_files_and_records_the_base() {
        let http = checkout_http();
        let (dir, wc) = do_checkout(&http);
        assert_eq!(read_file(&wc.root, "foo.spec"), "Name: foo\n");
        assert!(wc.root.join("foo.tar.gz").exists());
        assert_eq!(wc.project, "home:alice");
        assert_eq!(wc.package, "foo");
        assert_eq!(wc.backend, Backend::Obs);
        assert_eq!(wc.base_rev, REV1);
        assert_eq!(wc.base_srcmd5, SRCMD5_1);
        assert_eq!(wc.apiurl, "https://api.opensuse.org");
        let root = root_commit(&wc.root).unwrap();
        assert_eq!(
            root_message(&wc.root, &root).unwrap().trim(),
            format!("geectl base {REV1} {SRCMD5_1}")
        );
        // The checkout opens again: the record round-trips.
        let reopened = open(&wc.root).unwrap();
        assert_eq!(reopened, wc);
        let _ = dir;
    }

    #[test]
    fn checkout_refuses_a_traversal_listing_name() {
        // A listing entry that escapes the checkout (`../evil`)
        // is refused before anything is downloaded: the file
        // never materializes next to the checkout.
        let listing = format!(
            "<directory name=\"foo\" rev=\"1\" srcmd5=\"{SRCMD5_1}\" vrev=\"1\">\
            <entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" mtime=\"1282047302\" name=\"../evil\" size=\"10\" />\
            </directory>"
        );
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo/_meta",
                "<package name=\"foo\" project=\"home:alice\"></package>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo",
                &listing,
            );
        let dir = TempDirGuard::new("checkout-traversal");
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(!dir.path.join("home_alice").join("evil").exists());
        assert!(!dir.path.join("home_alice").join("foo").exists());
        let _ = dir;
    }

    #[test]
    fn checkout_refuses_a_backslash_listing_name() {
        // `\` separates paths on Windows, so a listing entry
        // carrying one (`a\..\..\evil`) would escape the checkout
        // there: it is refused on every platform, before anything
        // is downloaded.
        let listing = format!(
            "<directory name=\"foo\" rev=\"1\" srcmd5=\"{SRCMD5_1}\" vrev=\"1\">\
            <entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" mtime=\"1282047302\" name=\"a\\..\\..\\evil\" size=\"10\" />\
            </directory>"
        );
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo/_meta",
                "<package name=\"foo\" project=\"home:alice\"></package>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo",
                &listing,
            );
        let dir = TempDirGuard::new("checkout-backslash");
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(!dir.path.join("home_alice").join("evil").exists());
        assert!(!dir.path.join("home_alice").join("foo").exists());
        let _ = dir;
    }

    #[test]
    fn checkout_refuses_a_case_insensitive_listing_collision() {
        // `Foo.spec` and `foo.spec` cannot coexist on a
        // case-insensitive checkout (macOS, Windows): the second
        // download would overwrite the first, so the listing is
        // refused before anything is downloaded.
        let listing = format!(
            "<directory name=\"foo\" rev=\"1\" srcmd5=\"{SRCMD5_1}\" vrev=\"1\">\
            <entry md5=\"9444ad22a238286bca9bf649b2aef01d\" mtime=\"1282047302\" name=\"Foo.spec\" size=\"10\" />\
            <entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" mtime=\"1282047302\" name=\"foo.spec\" size=\"10\" />\
            </directory>"
        );
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo/_meta",
                "<package name=\"foo\" project=\"home:alice\"></package>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo",
                &listing,
            );
        let dir = TempDirGuard::new("checkout-case-collision");
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(!dir.path.join("home_alice").join("foo").exists());
        let _ = dir;
    }

    #[test]
    fn checkout_sanitizes_colons_in_the_directory_name() {
        // `:` is legal in an OBS name but not in a Windows path:
        // the checkout lands under the sanitized name, and the
        // record still carries the real project and package.
        let dir = TempDirGuard::new("checkout-colon");
        let http = checkout_http();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let wc = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(wc.root, dir.path.join("home_alice").join("foo"));
        assert_eq!(wc.project, "home:alice");
        assert_eq!(wc.package, "foo");
        let reopened = open(&wc.root).unwrap();
        assert_eq!(reopened.project, "home:alice");
        assert_eq!(reopened.package, "foo");
        let _ = dir;
    }

    #[test]
    fn checkout_succeeds_for_an_empty_package() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo/_meta",
                "<package name=\"foo\" project=\"home:alice\"></package>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo",
                &format!("<directory name=\"foo\" rev=\"1\" srcmd5=\"{SRCMD5_1}\" vrev=\"1\"></directory>"),
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dir = TempDirGuard::new("empty-checkout");
        let wc = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(wc.base_rev, REV1);
        // The root commit exists over the empty tree: the
        // checkout re-opens.
        let root = root_commit(&wc.root).unwrap();
        assert_eq!(
            root_message(&wc.root, &root).unwrap().trim(),
            format!("geectl base {REV1} {SRCMD5_1}")
        );
        assert_eq!(open(&wc.root).unwrap().base_rev, REV1);
        let _ = dir;
    }

    #[test]
    fn commit_obs_sends_the_index_state() {
        // The validate pass reports bar.txt missing; after the
        // stage, the final pass creates revision 2 — one revision,
        // exactly: a second validate pass would commit too.
        let validate_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv&withvalidate=1";
        let final_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv";
        let http = checkout_http()
            .with_post(
                validate_url,
                    "<directory error=\"missing\" name=\"foo\"><entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" name=\"bar.txt\" /></directory>",
                )
            .with_post(
                final_url,
                &format!("<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\" />"),
            )
            .with_put_bytes(
                "https://api.opensuse.org/source/home:alice/foo/bar.txt?rev=repository",
                "<revision rev=\"repository\"><srcmd5>empty</srcmd5></revision>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = client
            .package_requests_search_url("home:alice", "foo")
            .unwrap();
        let http = http.with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"42\" /></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "foo.spec", "Name: bar\n");
        write_file(&wc.root, "bar.txt", "new file\n");
        add(
            &wc,
            &["foo.spec".to_string(), "bar.txt".to_string()],
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        let report = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            false,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(report.rev, Some(2));
        assert_eq!(report.srcmd5, Some(SRCMD5_2.to_string()));
        assert_eq!(report.open_requests, vec!["42".to_string()]);
        assert_eq!(
            report.files,
            vec!["bar.txt".to_string(), "foo.spec".to_string()]
        );
        // Exactly one validate pass and one final pass, in that
        // order: a second validate pass would commit a second
        // revision.
        let posts = http.post_calls();
        assert_eq!(
            posts
                .iter()
                .map(|(url, _, _)| url.as_str())
                .collect::<Vec<_>>(),
            vec![validate_url, final_url]
        );
        // Both passes send the full list: the edited spec and the
        // added file, plus the unchanged tarball from the base —
        // an unchanged file left out would be deleted server-side.
        let full_list = "<directory><entry name=\"bar.txt\" md5=\"18519bfbd592b4e6cb238c5ccdbc209f\" /><entry name=\"foo.spec\" md5=\"cf0d04a286833ebd00a5f5b2d54474a9\" /><entry name=\"foo.tar.gz\" md5=\"d41d8cd98f00b204e9800998ecf8427e\" /></directory>";
        assert_eq!(posts[0].1.as_str(), full_list);
        assert_eq!(posts[1].1.as_str(), full_list);
        // The base refreshed to the new root commit.
        let root = root_commit(&wc.root).unwrap();
        assert_eq!(
            root_message(&wc.root, &root).unwrap().trim(),
            format!("geectl base 2 {SRCMD5_2}")
        );
        assert_eq!(open(&wc.root).unwrap().base_rev, "2");
        // The staged PUT went out for the missing file only, with
        // the index content and the credentials.
        let puts = http.put_bytes_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].0,
            "https://api.opensuse.org/source/home:alice/foo/bar.txt?rev=repository"
        );
        assert_eq!(puts[0].1, b"new file\n".to_vec());
        assert!(puts[0].2);
        let _ = dir;
    }

    #[test]
    fn commit_obs_base_refresh_failure_names_the_landed_revision() {
        // The commit lands revision 2, then the base refresh
        // fails: a stale `.git/config.lock` makes `git config
        // --local` refuse the write. The error names the landed
        // revision: the agent must not retry the commit and land
        // a second revision.
        let validate_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv&withvalidate=1";
        let final_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv";
        let http = checkout_http()
            .with_post(
                validate_url,
                    "<directory error=\"missing\" name=\"foo\"><entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" name=\"bar.txt\" /></directory>",
                )
            .with_post(
                final_url,
                &format!("<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\" />"),
            )
            .with_put_bytes(
                "https://api.opensuse.org/source/home:alice/foo/bar.txt?rev=repository",
                "<revision rev=\"repository\"><srcmd5>empty</srcmd5></revision>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = client
            .package_requests_search_url("home:alice", "foo")
            .unwrap();
        let http = http.with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"42\" /></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "foo.spec", "Name: bar\n");
        write_file(&wc.root, "bar.txt", "new file\n");
        add(
            &wc,
            &["foo.spec".to_string(), "bar.txt".to_string()],
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        std::fs::write(wc.root.join(".git").join("config.lock"), "").unwrap();
        let err = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            false,
            None,
            &DryRun::new(false),
        )
        .unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("the commit landed as revision 2"),
            "message: {message}"
        );
        // The base is unrefreshed in the struct: the commit did
        // not complete. (On disk the reset already moved HEAD to
        // the new root before the config write failed, so the
        // record is stale. That is why the error must name the
        // landed revision.)
        assert_eq!(wc.base_rev, "1");
        let _ = dir;
    }

    #[test]
    fn commit_obs_validate_pass_that_commits_skips_the_final_pass() {
        // A validate pass with nothing missing commits: the server
        // commits whenever the file list is complete. Its answer
        // carries the landed revision — no final pass runs.
        let validate_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv&withvalidate=1";
        let http = checkout_http().with_post(
            validate_url,
            &format!("<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\" />"),
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "foo.spec", "Name: bar\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        let report = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            false,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(report.rev, Some(2));
        assert_eq!(report.srcmd5, Some(SRCMD5_2.to_string()));
        assert_eq!(report.files, vec!["foo.spec".to_string()]);
        // Exactly one POST: the validate pass committed, so no final
        // pass runs — a final pass would commit a second revision.
        // Nothing was missing, so nothing was staged either.
        let posts = http.post_calls();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].0.as_str(), validate_url);
        assert!(http.put_bytes_calls().is_empty());
        // The validate pass sent the full list: the unchanged
        // tarball is in it, so the server keeps it.
        assert!(
            posts[0].1.contains("foo.tar.gz"),
            "validate body: {}",
            posts[0].1
        );
        assert_eq!(open(&wc.root).unwrap().base_rev, "2");
        let _ = dir;
    }

    #[test]
    fn commit_obs_deletion_only_commit_sends_the_full_list() {
        // Deleting a file is the sharpest case for both rules: the
        // sent list must still carry every unchanged file (omission
        // deletes), and the validate pass commits (nothing missing),
        // so no final pass runs.
        let validate_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv&withvalidate=1";
        let http = checkout_http().with_post(
            validate_url,
            &format!("<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\" />"),
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        rm(&wc, &["foo.tar.gz".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        let report = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            false,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(report.rev, Some(2));
        assert_eq!(report.files, vec!["foo.tar.gz".to_string()]);
        let posts = http.post_calls();
        assert_eq!(posts.len(), 1);
        // The full list: the unchanged spec is sent, the deleted
        // tarball is omitted.
        assert_eq!(
            posts[0].1.as_str(),
            "<directory><entry name=\"foo.spec\" md5=\"9444ad22a238286bca9bf649b2aef01d\" /></directory>"
        );
        assert_eq!(open(&wc.root).unwrap().base_rev, "2");
        let _ = dir;
    }

    #[test]
    fn commit_obs_retries_the_final_pass_when_files_are_still_missing() {
        // The final pass reports bar.txt still missing (the stage
        // raced): it did not commit, so the client stages again
        // and retries the final pass. One revision, exactly.
        let validate_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv&withvalidate=1";
        let final_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv";
        let final_ok =
            format!("<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\" />");
        let http = checkout_http()
            .with_post(
                validate_url,
                "<directory error=\"missing\" name=\"foo\"><entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" name=\"bar.txt\" /></directory>",
            )
            .with_post_sequence(
                final_url,
                &[
                    "<directory error=\"missing\" name=\"foo\"><entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" name=\"bar.txt\" /></directory>",
                    &final_ok,
                ],
            )
            .with_put_bytes(
                "https://api.opensuse.org/source/home:alice/foo/bar.txt?rev=repository",
                "<revision rev=\"repository\"><srcmd5>empty</srcmd5></revision>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "foo.spec", "Name: bar\n");
        write_file(&wc.root, "bar.txt", "new file\n");
        add(
            &wc,
            &["foo.spec".to_string(), "bar.txt".to_string()],
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        let report = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            false,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(report.rev, Some(2));
        // The stage ran twice (validate + final retry), the final
        // pass ran twice, the validate pass once.
        assert_eq!(http.put_bytes_calls().len(), 2);
        let posts = http.post_calls();
        assert_eq!(
            posts
                .iter()
                .map(|(url, _, _)| url.as_str())
                .collect::<Vec<_>>(),
            vec![validate_url, final_url, final_url]
        );
        // The retried final pass carries the sha256 answer for the
        // file it re-staged, and still the full list.
        assert!(
            posts[2].1.contains(
                "hash=\"sha256:0f15384d18789b1ebf3043dc7b6bc27273c8576373fbeb6f3e15854b588141c0\""
            ),
            "second final body: {}",
            posts[2].1
        );
        assert!(
            posts[2].1.contains("foo.tar.gz"),
            "second final body: {}",
            posts[2].1
        );
        let _ = dir;
    }

    #[test]
    fn commit_obs_settle_failure_names_the_landed_revision() {
        // The final pass lands revision 2 and reports a
        // server-side service run still in progress; the settle
        // poll then reads a failed run. The error names the landed
        // revision: the next commit would refuse as outdated, and
        // the content is already on the server.
        let validate_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv&withvalidate=1";
        let final_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv";
        let listing_failed = format!(
            "<directory name=\"foo\" rev=\"1\" srcmd5=\"{SRCMD5_1}\" vrev=\"1\">\
            <entry md5=\"9444ad22a238286bca9bf649b2aef01d\" mtime=\"1282047302\" name=\"foo.spec\" size=\"10\" />\
            <entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" mtime=\"1282047302\" name=\"foo.tar.gz\" size=\"0\" />\
            <serviceinfo code=\"failed\" xsrcmd5=\"aaa\" lsrcmd5=\"bbb\"><error>service run failed: download_url: 404 Not Found</error></serviceinfo></directory>"
        );
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo/_meta",
                "<package name=\"foo\" project=\"home:alice\"></package>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo",
                &listing_failed,
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=1",
                b"Name: foo\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=1",
                b"",
            )
            .with_post(validate_url, "<directory name=\"foo\" />")
            .with_post(
                final_url,
                &format!(
                    "<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\"><serviceinfo code=\"running\"/></directory>"
                ),
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        // The checkout reads the listing with the failed
        // serviceinfo: checkout ignores it, the commit's settle
        // poll reads it.
        let dir = TempDirGuard::new("settle-fail");
        let mut wc = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        write_file(&wc.root, "foo.spec", "Name: bar\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        let err = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            false,
            None,
            &DryRun::new(false),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServiceFailed);
        let message = err.to_string();
        assert!(message.contains("revision 2"), "message: {message}");
        // The base is unrefreshed: the commit did not complete.
        assert_eq!(open(&wc.root).unwrap().base_rev, "1");
        let _ = dir;
    }

    #[test]
    fn commit_obs_settle_reread_failure_names_the_landed_revision() {
        // The final pass lands revision 2 and reports a
        // server-side service run still in progress. The settle
        // poll reads a listing with no service run recorded, so it
        // breaks — and the re-read of the settled revision then
        // fails (the canned listing is exhausted). The error names
        // the landed revision: the content is already on the
        // server, and the checkout must update before committing
        // again.
        let validate_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv&withvalidate=1";
        let final_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv";
        let listing = listing_rev1();
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo/_meta",
                "<package name=\"foo\" project=\"home:alice\"></package>",
            )
            .with_get_sequence(
                "https://api.opensuse.org/public/source/home:alice/foo",
                &[&listing, &listing, &listing],
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=1",
                b"Name: foo\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=1",
                b"",
            )
            .with_post(validate_url, "<directory name=\"foo\" />")
            .with_post(
                final_url,
                &format!(
                    "<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\"><serviceinfo code=\"running\"/></directory>"
                ),
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dir = TempDirGuard::new("settle-reread-fail");
        let mut wc = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        write_file(&wc.root, "foo.spec", "Name: bar\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        let err = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            false,
            None,
            &DryRun::new(false),
        )
        .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("revision 2"), "message: {message}");
        // The base is unrefreshed: the commit did not complete.
        assert_eq!(open(&wc.root).unwrap().base_rev, "1");
        let _ = dir;
    }

    #[test]
    fn commit_obs_refuses_an_outdated_base() {
        // The listing read answers rev 1 at checkout, rev 2 at
        // commit time: the server moved past the checkout's base.
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo/_meta",
                "<package name=\"foo\" project=\"home:alice\"></package>",
            )
            .with_get_sequence(
                "https://api.opensuse.org/public/source/home:alice/foo",
                &[&listing_rev1(), &listing_rev2()],
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=1",
                b"Name: foo\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=1",
                b"",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "foo.spec", "Name: bar\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        let err = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            false,
            None,
            &DryRun::new(false),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::WorkingCopyOutdated);
        let _ = dir;
    }

    #[test]
    fn commit_obs_refuses_nothing_to_commit() {
        let http = checkout_http();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        let err = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            false,
            None,
            &DryRun::new(false),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NothingToCommit);
        let _ = dir;
    }

    #[test]
    fn update_obs_rebases_local_commits_onto_the_new_base() {
        let http = checkout_http()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
                &listing_rev2(),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=2",
                b"Name: server\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=2",
                b"",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        // A local commit on an untouched file: the rebase applies
        // it cleanly onto the new base.
        write_file(&wc.root, "bar.txt", "local\n");
        add(&wc, &["bar.txt".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local bar"]);
        let report = update(&mut wc, &client, Some("2"), None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(report.updated);
        assert_eq!(report.old_base_rev, REV1);
        assert_eq!(report.new_base_rev, "2");
        assert!(report.conflicts.is_empty());
        assert_eq!(read_file(&wc.root, "foo.spec"), "Name: server\n");
        assert_eq!(read_file(&wc.root, "bar.txt"), "local\n");
        assert_eq!(open(&wc.root).unwrap().base_rev, "2");
        assert_eq!(wc.base_rev, "2");
        assert_eq!(wc.base_srcmd5, SRCMD5_2);
        let _ = dir;
    }

    #[test]
    fn update_refuses_a_traversal_listing_name() {
        // The new base's listing carries an entry that escapes
        // the checkout (`../evil`): the update is refused before
        // anything is downloaded, and the base does not move.
        let listing = format!(
            "<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\">\
            <entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" mtime=\"1282047303\" name=\"../evil\" size=\"12\" />\
            </directory>"
        );
        let http = checkout_http().with_get(
            "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
            &listing,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        let err = update(&mut wc, &client, Some("2"), None, &DryRun::new(false)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert_eq!(wc.base_rev, REV1);
        assert!(!dir.path.join("home_alice").join("evil").exists());
        let _ = dir;
    }

    #[test]
    fn resolve_finishes_a_conflicted_rebase() {
        let http = checkout_http()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
                &listing_rev2(),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=2",
                b"Name: server\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=2",
                b"",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        // A local commit on the same line the new base changed:
        // the rebase stops with a conflict.
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local spec"]);
        let report = update(&mut wc, &client, Some("2"), None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(!report.updated);
        assert_eq!(report.conflicts, vec!["foo.spec".to_string()]);
        write_file(&wc.root, "foo.spec", "Name: resolved\n");
        let resolved = resolve(&mut wc, &DryRun::new(false)).unwrap().unwrap();
        assert_eq!(resolved, vec!["foo.spec".to_string()]);
        assert_eq!(read_file(&wc.root, "foo.spec"), "Name: resolved\n");
        assert!(!rebase_in_progress(&wc).unwrap());
        // The record adopted the new base, on disk and in this
        // checkout.
        assert_eq!(open(&wc.root).unwrap().base_rev, "2");
        assert_eq!(wc.base_rev, "2");
        assert_eq!(wc.base_srcmd5, SRCMD5_2);
        let _ = dir;
    }

    #[test]
    fn resolve_dry_run_plans_the_staging_and_the_continue() {
        let http = checkout_http()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
                &listing_rev2(),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=2",
                b"Name: server\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=2",
                b"",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local spec"]);
        let report = update(&mut wc, &client, Some("2"), None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(!report.updated);
        assert_eq!(report.conflicts, vec!["foo.spec".to_string()]);
        write_file(&wc.root, "foo.spec", "Name: resolved\n");
        let dry_run = DryRun::new(true);
        let out = resolve(&mut wc, &dry_run).unwrap();
        assert!(out.is_none());
        let plan = dry_run.plan();
        assert!(
            plan.iter()
                .any(|line| line.contains("would stage foo.spec")),
            "plan: {plan:?}"
        );
        assert!(
            plan.iter()
                .any(|line| line.contains("would continue the rebase")),
            "plan: {plan:?}"
        );
        // Nothing was staged or continued: the rebase is still
        // stopped on the same conflict, against the new base the
        // update already moved the record to.
        assert!(rebase_in_progress(&wc).unwrap());
        assert_eq!(unmerged_files(&wc).unwrap(), vec!["foo.spec".to_string()]);
        assert_eq!(open(&wc.root).unwrap().base_rev, "2");
        let _ = dir;
    }

    #[test]
    fn resolve_dry_run_on_a_clean_tree_plans_nothing() {
        let http = checkout_http();
        let (dir, mut wc) = do_checkout(&http);
        let dry_run = DryRun::new(true);
        let out = resolve(&mut wc, &dry_run).unwrap();
        assert!(out.is_none());
        let plan = dry_run.plan();
        assert!(
            plan.iter().any(|line| line.contains("nothing to resolve")),
            "plan: {plan:?}"
        );
        let _ = dir;
    }

    #[test]
    fn resolve_continues_a_rebase_with_already_staged_resolutions() {
        let http = checkout_http()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
                &listing_rev2(),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=2",
                b"Name: server\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=2",
                b"",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local spec"]);
        let report = update(&mut wc, &client, Some("2"), None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(!report.updated);
        assert_eq!(report.conflicts, vec!["foo.spec".to_string()]);
        // The agent resolves and stages by hand, the natural git
        // flow: resolve must still finish the stopped rebase
        // instead of reporting nothing to resolve.
        write_file(&wc.root, "foo.spec", "Name: resolved\n");
        git_ok(&wc.root, &["add", "foo.spec"]);
        let resolved = resolve(&mut wc, &DryRun::new(false)).unwrap().unwrap();
        assert!(resolved.is_empty());
        assert!(!rebase_in_progress(&wc).unwrap());
        assert_eq!(open(&wc.root).unwrap().base_rev, "2");
        assert_eq!(wc.base_rev, "2");
        assert_eq!(wc.base_srcmd5, SRCMD5_2);
        let _ = dir;
    }

    #[test]
    fn resolve_reports_a_stranded_stash_when_continue_fails() {
        let http = gtxt_http();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        // Two local commits on g.txt: the first changes line 1,
        // the second line 2.
        write_file(&wc.root, "g.txt", "ONE\ntwo\nthree\n");
        git_ok(&wc.root, &["add", "g.txt"]);
        git_ok(&wc.root, &["commit", "--quiet", "-m", "first line"]);
        write_file(&wc.root, "g.txt", "ONE\nTWO\nthree\n");
        git_ok(&wc.root, &["add", "g.txt"]);
        git_ok(&wc.root, &["commit", "--quiet", "-m", "second line"]);
        // Revision 2 changes line 1 too: the rebase stops on the
        // first commit.
        let report = update(&mut wc, &client, Some("2"), None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(!report.updated);
        assert_eq!(report.conflicts, vec!["g.txt".to_string()]);
        // The resolution rewrites lines 1 and 2, is staged by
        // hand, and the agent then makes a further unstaged edit
        // to the same file.
        write_file(&wc.root, "g.txt", "RESOLVED1\nRESOLVED2\nthree\n");
        git_ok(&wc.root, &["add", "g.txt"]);
        write_file(&wc.root, "g.txt", "RESOLVED1\nRESOLVED2\nthree\nextra\n");
        // The continue applies the second commit, whose line-2
        // change conflicts with the resolution — and the stashed
        // edit cannot be restored over the conflicted file.
        let err = resolve(&mut wc, &DryRun::new(false)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InternalError);
        let message = err.to_string();
        assert!(message.contains("stash"), "message: {message}");
        assert!(message.contains("g.txt"), "message: {message}");
        // The stash entry is still there, holding the edit.
        let stash = git(&wc.root, "stash", &["list"], &[]).unwrap();
        assert!(stash.contains("geectl resolve"), "stash: {stash}");
        let _ = dir;
    }

    #[test]
    fn resolve_adopts_the_new_base_when_the_stash_pop_fails() {
        let http = gtxt6_http();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        // Two local commits on g.txt: the first changes line 1,
        // the second line 5.
        write_file(&wc.root, "g.txt", "ONE\ntwo\nthree\nfour\nfive\nsix\n");
        git_ok(&wc.root, &["add", "g.txt"]);
        git_ok(&wc.root, &["commit", "--quiet", "-m", "first line"]);
        write_file(&wc.root, "g.txt", "ONE\ntwo\nthree\nfour\nFIVE\nsix\n");
        git_ok(&wc.root, &["add", "g.txt"]);
        git_ok(&wc.root, &["commit", "--quiet", "-m", "fifth line"]);
        // Revision 2 changes line 1 too: the rebase stops on the
        // first commit.
        let report = update(&mut wc, &client, Some("2"), None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(!report.updated);
        assert_eq!(report.conflicts, vec!["g.txt".to_string()]);
        // The resolution rewrites line 1 only, is staged by hand,
        // and the agent then makes a further unstaged edit to
        // line 5 — the line the second commit changes.
        write_file(
            &wc.root,
            "g.txt",
            "RESOLVED1\ntwo\nthree\nfour\nfive\nsix\n",
        );
        git_ok(&wc.root, &["add", "g.txt"]);
        write_file(
            &wc.root,
            "g.txt",
            "RESOLVED1\ntwo\nthree\nfour\nfive-edited\nsix\n",
        );
        // The continue applies the second commit cleanly (its
        // line-5 change is far from the line-1 resolution), so the
        // rebase finishes — but the stashed edit cannot be
        // restored over the second commit's line-5 change, and the
        // pop fails.
        let err = resolve(&mut wc, &DryRun::new(false)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InternalError);
        let message = err.to_string();
        assert!(message.contains("stash"), "message: {message}");
        // The rebase finished, so the new base was adopted even
        // though the stash is stranded.
        assert!(!rebase_in_progress(&wc).unwrap());
        assert_eq!(open(&wc.root).unwrap().base_rev, "2");
        assert_eq!(wc.base_rev, "2");
        assert_eq!(wc.base_srcmd5, SRCMD5_2);
        // The stash entry is still there, holding the edit.
        let stash = git(&wc.root, "stash", &["list"], &[]).unwrap();
        assert!(stash.contains("geectl resolve"), "stash: {stash}");
        let _ = dir;
    }

    #[test]
    fn status_reports_a_local_commit_and_an_untracked_file() {
        let dir = TempDirGuard::new("status-local");
        init_repo(&dir.path);
        for (name, content) in [("a.txt", "alpha\n"), ("b.txt", "bravo\n")] {
            write_file(&dir.path, name, content);
        }
        git_ok(&dir.path, &["add", "-A"]);
        git_ok(
            &dir.path,
            &[
                "commit",
                "--quiet",
                "-m",
                &format!("geectl base {REV1} {SRCMD5_1}"),
            ],
        );
        record_obs(&dir.path, REV1, SRCMD5_1);
        let wc = open(&dir.path).unwrap();
        // A local commit ahead of the base: status compares
        // against the base, not HEAD, so the change still shows.
        write_file(&dir.path, "a.txt", "changed\n");
        git_ok(&dir.path, &["add", "a.txt"]);
        git_ok(&dir.path, &["commit", "--quiet", "-m", "local change"]);
        write_file(&dir.path, "u.txt", "untracked\n");
        let mut found: Vec<(String, StatusCode)> = status(&wc)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.path, entry.code))
            .collect();
        found.sort();
        assert_eq!(
            found,
            vec![
                ("a.txt".to_string(), StatusCode::Modified),
                ("u.txt".to_string(), StatusCode::Unknown),
            ]
        );
    }

    #[test]
    fn status_reports_conflicts() {
        let http = checkout_http()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
                &listing_rev2(),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=2",
                b"Name: server\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=2",
                b"",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local spec"]);
        let report = update(&mut wc, &client, Some("2"), None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(!report.updated);
        assert_eq!(report.conflicts, vec!["foo.spec".to_string()]);
        let found: Vec<(String, StatusCode)> = status(&wc)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.path, entry.code))
            .collect();
        assert_eq!(
            found,
            vec![("foo.spec".to_string(), StatusCode::Conflicted)]
        );
        let _ = dir;
    }

    #[test]
    fn commit_refuses_with_conflicts() {
        let http = checkout_http()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
                &listing_rev2(),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=2",
                b"Name: server\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=2",
                b"",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local spec"]);
        update(&mut wc, &client, Some("2"), None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        let err = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            false,
            None,
            &DryRun::new(false),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::FileConflict);
        let _ = dir;
    }

    #[test]
    fn commit_obs_noservice_skips_server_services() {
        let validate_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv&withvalidate=1&noservice=1";
        let final_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv&noservice=1";
        let http = checkout_http()
            .with_post(
                validate_url,
                    "<directory error=\"missing\" name=\"foo\"><entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" name=\"bar.txt\" /></directory>",
                )
            .with_post(
                final_url,
                &format!("<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\" />"),
            )
            .with_put_bytes(
                "https://api.opensuse.org/source/home:alice/foo/bar.txt?rev=repository",
                "<revision rev=\"repository\"><srcmd5>empty</srcmd5></revision>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = client
            .package_requests_search_url("home:alice", "foo")
            .unwrap();
        let http = http.with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"42\" /></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "foo.spec", "Name: bar\n");
        write_file(&wc.root, "bar.txt", "new file\n");
        add(
            &wc,
            &["foo.spec".to_string(), "bar.txt".to_string()],
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        let report = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            true,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(report.rev, Some(2));
        assert_eq!(report.srcmd5, Some(SRCMD5_2.to_string()));
        // The final pass, not the validate pass, carries
        // `noservice=1`: the server runs no services for it.
        let posts = http.post_calls();
        assert!(posts.iter().any(|(url, _, _)| url == final_url));
        assert_eq!(open(&wc.root).unwrap().base_rev, "2");
        let _ = dir;
    }

    #[test]
    fn commit_obs_answers_a_sha256_challenge() {
        let validate_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv&withvalidate=1";
        let final_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv";
        let http = checkout_http()
            .with_post(
                validate_url,
                    "<directory error=\"missing\" name=\"foo\"><entry md5=\"cf0d04a286833ebd00a5f5b2d54474a9\" name=\"foo.spec\" hash=\"new\" /></directory>",
                )
            .with_post(
                final_url,
                &format!("<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\" />"),
            )
            .with_put_bytes(
                "https://api.opensuse.org/source/home:alice/foo/foo.spec?rev=repository",
                "<revision rev=\"repository\"><srcmd5>empty</srcmd5></revision>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "foo.spec", "Name: bar\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        let report = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            false,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(report.rev, Some(2));
        // The final pass carries the sha256 of the index content
        // of the challenged file.
        let posts = http.post_calls();
        let final_body = posts
            .iter()
            .find(|(url, _, _)| url == final_url)
            .map(|(_, body, _)| body.clone())
            .unwrap();
        assert!(
            final_body.contains(
                "name=\"foo.spec\" md5=\"cf0d04a286833ebd00a5f5b2d54474a9\" hash=\"sha256:7ab81544975cc1bb4841ecf814a41513e77e9bf58c958f5d63a1c6cefc5cb58e\""
            ),
            "final body: {final_body}"
        );
        let _ = dir;
    }

    #[test]
    fn commit_obs_sends_keeplink_when_the_link_is_staged() {
        let validate_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv&withvalidate=1&keeplink=1";
        let final_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv&keeplink=1";
        let http = checkout_http()
            .with_post(
                validate_url,
                    "<directory error=\"missing\" name=\"foo\"><entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" name=\"_link\" /></directory>",
                )
            .with_post(
                final_url,
                &format!("<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\" />"),
            )
            .with_put_bytes(
                "https://api.opensuse.org/source/home:alice/foo/_link?rev=repository",
                "<revision rev=\"repository\"><srcmd5>empty</srcmd5></revision>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = client
            .package_requests_search_url("home:alice", "foo")
            .unwrap();
        let http = http.with_get(&search_url, "<collection matches=\"0\"></collection>");
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(
            &wc.root,
            "_link",
            "<link project=\"home:alice\" package=\"bar\"/>",
        );
        add(&wc, &["_link".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        let report = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            false,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(report.rev, Some(2));
        let posts = http.post_calls();
        assert!(posts.iter().any(|(url, _, _)| url == validate_url));
        assert!(posts.iter().any(|(url, _, _)| url == final_url));
        let _ = dir;
    }

    #[test]
    fn commit_obs_refuses_when_the_service_gate_fails() {
        let http = checkout_http();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "_service", "<notservices/>");
        add(&wc, &["_service".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        let err = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            false,
            None,
            &DryRun::new(false),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let _ = dir;
    }

    #[test]
    fn commit_obs_skips_the_service_gate_with_noservice() {
        let validate_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv&withvalidate=1&noservice=1";
        let final_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv&noservice=1";
        let http = checkout_http()
            .with_post(
                validate_url,
                    "<directory error=\"missing\" name=\"foo\"><entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" name=\"_service\" /></directory>",
                )
            .with_post(
                final_url,
                &format!("<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\" />"),
            )
            .with_put_bytes(
                "https://api.opensuse.org/source/home:alice/foo/_service?rev=repository",
                "<revision rev=\"repository\"><srcmd5>empty</srcmd5></revision>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "_service", "<notservices/>");
        add(&wc, &["_service".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        let report = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            true,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(report.rev, Some(2));
        let _ = dir;
    }

    #[test]
    fn stage_gate_changes_stages_only_the_changed_files() {
        let dir = TempDirGuard::new("gate-stage");
        init_repo(&dir.path);
        base_commit_with(&dir.path, REV1, SRCMD5_1);
        let wc = open(&dir.path).unwrap();
        // A pre-existing unstaged edit, present before the
        // snapshot; and a staged new file the gate does not touch.
        write_file(&dir.path, "foo.spec", "Name: preexisting\n");
        write_file(&dir.path, "bar.txt", "bar\n");
        git_ok(&dir.path, &["add", "bar.txt"]);
        let snapshot = snapshot_gate_inputs(&wc).unwrap();
        // The gate changes foo.spec further.
        write_file(&dir.path, "foo.spec", "Name: gated\n");
        let staged = stage_gate_changes(&wc, &snapshot).unwrap();
        assert_eq!(staged, vec!["foo.spec".to_string()]);
        // foo.spec is staged whole, including the pre-existing edit.
        assert_eq!(
            String::from_utf8(index_content(&wc, "foo.spec").unwrap()).unwrap(),
            "Name: gated\n"
        );
        let cached = git(
            &wc.root,
            "diff",
            &["--cached", "--name-only", "-z", "HEAD"],
            &[],
        )
        .unwrap();
        assert_eq!(
            split_nul(&cached),
            vec!["bar.txt".to_string(), "foo.spec".to_string()]
        );
        // Nothing unstaged remains.
        let worktree = git(&wc.root, "diff", &["--name-only", "-z"], &[]).unwrap();
        assert!(split_nul(&worktree).is_empty());
    }

    #[test]
    fn add_stages_a_worktree_deletion() {
        let dir = TempDirGuard::new("add-del");
        init_repo(&dir.path);
        base_commit_with(&dir.path, REV1, SRCMD5_1);
        let wc = open(&dir.path).unwrap();
        std::fs::remove_file(dir.path.join("foo.spec")).unwrap();
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        let found: Vec<(String, StatusCode)> = status(&wc)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.path, entry.code))
            .collect();
        assert_eq!(found, vec![("foo.spec".to_string(), StatusCode::Deleted)]);
    }

    #[test]
    fn rm_validates_all_files_before_removing_any() {
        let dir = TempDirGuard::new("rm-atomic");
        init_repo(&dir.path);
        base_commit_with(&dir.path, REV1, SRCMD5_1);
        let wc = open(&dir.path).unwrap();
        write_file(&dir.path, "new.txt", "new\n");
        // An untracked file plus a missing one: nothing is removed.
        let err = rm(
            &wc,
            &["new.txt".to_string(), "missing.txt".to_string()],
            &DryRun::new(false),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(dir.path.join("new.txt").exists());
        // A tracked file plus a missing one: nothing is removed.
        let err = rm(
            &wc,
            &["foo.spec".to_string(), "missing.txt".to_string()],
            &DryRun::new(false),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(dir.path.join("foo.spec").exists());
        // The refused removals left no trace: the only status
        // entry is the untracked file the first call refused to
        // remove.
        let found: Vec<(String, StatusCode)> = status(&wc)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.path, entry.code))
            .collect();
        assert_eq!(found, vec![("new.txt".to_string(), StatusCode::Unknown)]);
    }

    #[test]
    fn rm_refuses_an_untracked_directory() {
        let dir = TempDirGuard::new("rm-dir");
        init_repo(&dir.path);
        base_commit_with(&dir.path, REV1, SRCMD5_1);
        let wc = open(&dir.path).unwrap();
        std::fs::create_dir(dir.path.join("sub")).unwrap();
        write_file(&dir.path, "sub/file.txt", "in a directory\n");
        // A directory is not a file to remove: the refusal is a
        // validation error, and nothing is removed.
        let err = rm(&wc, &["sub".to_string()], &DryRun::new(false)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(dir.path.join("sub/file.txt").exists());
        let _ = dir;
    }

    #[test]
    #[cfg(unix)]
    fn rm_refuses_a_path_through_a_symlink() {
        // Removing through a symlink would delete outside the
        // checkout: the refusal is a validation error, and the
        // outside file survives.
        let dir = TempDirGuard::new("rm-symlink");
        let outside_dir = TempDirGuard::new("rm-symlink-outside");
        init_repo(&dir.path);
        base_commit_with(&dir.path, REV1, SRCMD5_1);
        let wc = open(&dir.path).unwrap();
        write_file(&outside_dir.path, "evil.txt", "outside\n");
        std::os::unix::fs::symlink(&outside_dir.path, dir.path.join("link")).unwrap();
        let err = rm(&wc, &["link/evil.txt".to_string()], &DryRun::new(false)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(outside_dir.path.join("evil.txt").exists());
        let _ = dir;
        let _ = outside_dir;
    }

    #[test]
    fn rm_removes_a_dirty_tracked_file() {
        let dir = TempDirGuard::new("rm-dirty");
        init_repo(&dir.path);
        base_commit_with(&dir.path, REV1, SRCMD5_1);
        let wc = open(&dir.path).unwrap();
        write_file(&dir.path, "foo.spec", "Name: dirty\n");
        git_ok(&dir.path, &["add", "foo.spec"]);
        rm(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(!dir.path.join("foo.spec").exists());
        let found: Vec<(String, StatusCode)> = status(&wc)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.path, entry.code))
            .collect();
        assert_eq!(found, vec![("foo.spec".to_string(), StatusCode::Deleted)]);
    }

    #[test]
    fn checkout_removes_a_partial_target_on_failure() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo/_meta",
                "<package name=\"foo\" project=\"home:alice\"></package>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo",
                &listing_rev1(),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=1",
                b"Name: foo\n",
            );
        // foo.tar.gz has no download mapping: the checkout fails
        // mid-download.
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dir = TempDirGuard::new("checkout-fail");
        let err = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap_err();
        let _ = err;
        assert!(!dir.path.join("home:alice").join("foo").exists());
    }

    /// A `git http-backend` server rooted at `root`, serving smart
    /// HTTP on 127.0.0.1. The git-backed checkout tests clone,
    /// fetch, and push through it: `checkout_git` only accepts
    /// http(s) scmsync URLs, so the test origins must be served
    /// over HTTP — a local path would be refused, and a dumb
    /// static server cannot serve pushes. Each request spawns
    /// `git http-backend` with the CGI environment (all request
    /// headers as `HTTP_*`, so `Git-Protocol: version=2` reaches
    /// it), and the response is written back with
    /// `Connection: close`.
    struct HttpBackendServer {
        port: u16,
        shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl HttpBackendServer {
        fn new(root: PathBuf) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = shutdown.clone();
            // `git --exec-path` locates `git-http-backend` for
            // this git installation.
            let exec_path = std::process::Command::new("git")
                .arg("--exec-path")
                .output()
                .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
                .unwrap_or_default();
            let backend = PathBuf::from(exec_path).join("git-http-backend");
            std::thread::spawn(move || {
                // A blocking accept: the wake-up connect in
                // `Drop` unblocks it for shutdown. An accept
                // error is never fatal — under a full test load
                // the process can briefly hit its file-descriptor
                // limit (`EMFILE`), and dropping the listener
                // over it would reset every later git connection;
                // the error is retried until fds free up or the
                // server is shut down.
                while !flag.load(std::sync::atomic::Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let root = root.clone();
                            let backend = backend.clone();
                            std::thread::spawn(move || {
                                serve_http_backend(stream, &root, &backend);
                            });
                        }
                        Err(_) => {
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                    }
                }
            });
            HttpBackendServer { port, shutdown }
        }
    }

    impl Drop for HttpBackendServer {
        fn drop(&mut self) {
            self.shutdown
                .store(true, std::sync::atomic::Ordering::SeqCst);
            // Wake the accept loop so the thread exits.
            let _ = std::net::TcpStream::connect(("127.0.0.1", self.port));
        }
    }

    fn serve_http_backend(stream: std::net::TcpStream, root: &Path, backend: &Path) {
        use std::io::{Read, Write};
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(60)));
        // One handle for the whole connection: the reader is
        // turned back into the stream for the response, so no
        // second file descriptor (and no failing clone) per
        // request.
        let mut reader = std::io::BufReader::new(stream);
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while head.len() < 65536 {
            match reader.read(&mut byte) {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    head.push(byte[0]);
                    if head.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
            }
        }
        let head_text = String::from_utf8_lossy(&head).into_owned();
        let mut lines = head_text.lines();
        let Some(request_line) = lines.next() else {
            return;
        };
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("GET").to_string();
        let target = parts.next().unwrap_or("/").to_string();
        let (path, query) = match target.split_once('?') {
            Some((path, query)) => (path.to_string(), query.to_string()),
            None => (target.clone(), String::new()),
        };
        let mut content_length = 0usize;
        let mut content_type = String::new();
        let mut header_env: Vec<(String, String)> = Vec::new();
        for line in lines {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let name = name.trim();
            let value = value.trim().to_string();
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.parse().unwrap_or(0);
            } else if name.eq_ignore_ascii_case("content-type") {
                content_type = value.clone();
            }
            let env_name = format!("HTTP_{}", name.to_ascii_uppercase().replace('-', "_"));
            header_env.push((env_name, value));
        }
        let mut body = vec![0u8; content_length];
        // Reading zero bytes is a no-op, so no length guard is
        // needed here.
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        // The request is fully read: the reader turns back
        // into the stream for the response.
        let mut stream = reader.into_inner();
        let mut cmd = std::process::Command::new(backend);
        cmd.env("GIT_PROJECT_ROOT", root);
        cmd.env("GIT_HTTP_EXPORT_ALL", "1");
        cmd.env("PATH_INFO", &path);
        cmd.env("QUERY_STRING", &query);
        cmd.env("REQUEST_METHOD", &method);
        cmd.env("REMOTE_ADDR", "127.0.0.1");
        if !content_type.is_empty() {
            cmd.env("CONTENT_TYPE", &content_type);
        }
        cmd.env("CONTENT_LENGTH", content_length.to_string());
        for (name, value) in header_env {
            cmd.env(name, value);
        }
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::null());
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(_) => {
                let _ = stream.write_all(
                    b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                return;
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(&body);
        }
        let output = child.wait_with_output();
        let Ok(output) = output else {
            let _ = stream.write_all(
                b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            return;
        };
        let stdout = output.stdout;
        let (headers, body) = match find_subsequence(&stdout, b"\r\n\r\n") {
            Some(pos) => (&stdout[..pos], &stdout[pos + 4..]),
            None => match find_subsequence(&stdout, b"\n\n") {
                Some(pos) => (&stdout[..pos], &stdout[pos + 2..]),
                None => (&stdout[..], &stdout[stdout.len()..]),
            },
        };
        let headers_text = String::from_utf8_lossy(headers).into_owned();
        let mut status = "200 OK".to_string();
        let mut header_lines = Vec::new();
        for line in headers_text.lines() {
            if let Some(value) = line.strip_prefix("Status:") {
                status = value.trim().to_string();
            } else if !line.trim().is_empty() {
                header_lines.push(line.to_string());
            }
        }
        let mut response = format!("HTTP/1.1 {status}\r\n");
        for line in header_lines {
            response.push_str(&line);
            response.push_str("\r\n");
        }
        response.push_str(&format!("Content-Length: {}\r\n", body.len()));
        response.push_str("Connection: close\r\n\r\n");
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.write_all(body);
    }

    fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    /// A bare origin repository with a `main` branch holding one
    /// commit, plus the work clone that made it — served over
    /// smart HTTP, the way a real scmsync URL serves it. Returns
    /// the origin path, the HTTP clone URL, and the main head SHA;
    /// the server lives as long as the returned value does.
    struct GitOrigin {
        path: PathBuf,
        url: String,
        head: String,
        _server: HttpBackendServer,
    }

    /// The bare origin repository with a `main` branch holding one
    /// commit, plus the work clone that made it, served over
    /// smart HTTP.
    fn git_origin(dir: &Path) -> GitOrigin {
        let origin = dir.join("origin");
        git(dir, "init", &["--bare", "-b", "main", "origin"], &[]).unwrap();
        // `git http-backend` refuses pushes unless the repo
        // enables receive-pack; a real scmsync host does the same.
        git(&origin, "config", &["http.receivepack", "true"], &[]).unwrap();
        let work = dir.join("work");
        git(dir, "clone", &["--quiet", "origin", "work"], &[]).unwrap();
        git(&work, "config", &["--local", "user.name", "geectl"], &[]).unwrap();
        git(
            &work,
            "config",
            &["--local", "user.email", "geectl@localhost"],
            &[],
        )
        .unwrap();
        write_file(&work, "foo.spec", "Name: foo\n");
        git_ok(&work, &["add", "-A"]);
        git_ok(&work, &["commit", "--quiet", "-m", "initial"]);
        git_ok(&work, &["push", "--quiet", "origin", "main"]);
        let head = git(&work, "rev-parse", &["HEAD"], &[])
            .unwrap()
            .trim()
            .to_string();
        let server = HttpBackendServer::new(dir.to_path_buf());
        let url = format!("http://127.0.0.1:{}/origin", server.port);
        GitOrigin {
            path: origin,
            url,
            head,
            _server: server,
        }
    }

    /// The `_meta` for a git-backed package whose scmsync URL is
    /// the origin's HTTP URL, on branch `main`.
    fn git_checkout_http(origin_url: &str) -> FakeHttp {
        FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/home:alice/foo/_meta",
            &format!(
                "<package name=\"foo\" project=\"home:alice\"><scmsync>{origin_url}#main</scmsync></package>",
            ),
        )
    }

    /// The Gitea auth for tests: a token against the default
    /// instance.
    fn test_auth() -> GiteaAuth<'static> {
        GiteaAuth {
            token: "token",
            url: "https://src.opensuse.org",
        }
    }

    fn do_git_checkout(http: &FakeHttp, dir: &TempDirGuard) -> WorkingCopy {
        let client = ObsClient::with_credentials(http, basic_credentials()).unwrap();
        checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    fn token_env_scopes_the_header_to_the_gitea_instance() {
        let auth = GiteaAuth {
            token: "SECRET",
            url: "https://src.opensuse.org",
        };
        let mut header = String::new();
        let mut key = String::new();
        let env = token_env(&auth, &mut header, &mut key);
        assert_eq!(env[0], ("GIT_CONFIG_COUNT", "1"));
        assert_eq!(
            env[1],
            (
                "GIT_CONFIG_KEY_0",
                "http.https://src.opensuse.org.extraHeader"
            )
        );
        assert_eq!(
            env[2],
            ("GIT_CONFIG_VALUE_0", "Authorization: token SECRET")
        );
    }

    /// A one-shot HTTP server on 127.0.0.1 that records the
    /// Authorization header of the first request it sees, then
    /// answers 404. The recorded value arrives on the channel
    /// after the request is handled.
    fn header_recorder() -> (u16, std::sync::mpsc::Receiver<Option<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut found = None;
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
                let mut buf = [0u8; 4096];
                if let Ok(n) = stream.read(&mut buf) {
                    let request = String::from_utf8_lossy(&buf[..n]);
                    for line in request.lines() {
                        if let Some(value) = line.strip_prefix("Authorization: ") {
                            found = Some(value.to_string());
                            break;
                        }
                    }
                }
                let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
            }
            let _ = tx.send(found);
        });
        (port, rx)
    }

    #[test]
    fn checkout_git_sends_the_token_only_to_the_gitea_host() {
        // The token is scoped to the Gitea instance: a scmsync URL
        // on another host gets no Authorization header, and a
        // scmsync URL on the Gitea host does. The server is not a
        // git server, so the clone fails either way; the request it
        // received is what the test asserts on.
        let dir = TempDirGuard::new("git-token-scope");
        let (port, rx) = header_recorder();
        let other = format!("http://127.0.0.1:{port}/other");
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/home:alice/foo/_meta",
            &format!(
                "<package name=\"foo\" project=\"home:alice\"><scmsync>{other}</scmsync></package>"
            ),
        );
        let auth = GiteaAuth {
            token: "SECRET-TOKEN-123",
            url: "https://src.opensuse.org",
        };
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let _ = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            Some(&auth),
            &DryRun::new(false),
        )
        .unwrap_err();
        let header = rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap();
        assert_eq!(header, None, "the token was sent to a non-Gitea host");
        let _ = dir;
    }

    #[test]
    fn checkout_git_sends_the_token_to_the_gitea_host() {
        let dir = TempDirGuard::new("git-token-gitea");
        let (port, rx) = header_recorder();
        let repo = format!("http://127.0.0.1:{port}/repo");
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/home:alice/foo/_meta",
            &format!(
                "<package name=\"foo\" project=\"home:alice\"><scmsync>{repo}</scmsync></package>"
            ),
        );
        let auth = GiteaAuth {
            token: "SECRET-TOKEN-123",
            url: &format!("http://127.0.0.1:{port}"),
        };
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let _ = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            Some(&auth),
            &DryRun::new(false),
        )
        .unwrap_err();
        let header = rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap();
        assert_eq!(header, Some("token SECRET-TOKEN-123".to_string()));
        let _ = dir;
    }

    #[test]
    fn split_scmsync_url_strips_the_fragment() {
        assert_eq!(
            split_scmsync_url("https://code.opensuse.org/foo/bar#main"),
            ("https://code.opensuse.org/foo/bar", Some("main"))
        );
        assert_eq!(split_scmsync_url("/tmp/origin"), ("/tmp/origin", None));
        assert_eq!(
            split_scmsync_url("https://example.com/x#"),
            ("https://example.com/x", Some(""))
        );
    }

    #[test]
    fn checkout_git_refuses_a_dash_prefixed_scmsync_url() {
        // A URL starting with `-` would be parsed as a git
        // option (`--upload-pack=...`), not a repository: the
        // clone never runs.
        let dir = TempDirGuard::new("git-checkout-dash");
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/home:alice/foo/_meta",
            "<package name=\"foo\" project=\"home:alice\"><scmsync>--upload-pack=touch /tmp/geectl-pwn#main</scmsync></package>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(!dir.path.join("home_alice").join("foo").exists());
        let _ = dir;
    }

    #[test]
    fn checkout_git_clones_the_scmsync_branch() {
        let dir = TempDirGuard::new("git-checkout");
        let origin = git_origin(&dir.path);
        let main_head = origin.head.clone();
        // A second branch, `other`, becomes the origin's default:
        // the clone must still check out `main`, the fragment's
        // branch, not the default.
        let work = dir.path.join("work");
        git_ok(&work, &["checkout", "--quiet", "-b", "other"]);
        write_file(&work, "foo.spec", "Name: other\n");
        git_ok(&work, &["add", "-A"]);
        git_ok(&work, &["commit", "--quiet", "-m", "other branch"]);
        git_ok(&work, &["push", "--quiet", "origin", "other"]);
        git(
            &origin.path,
            "symbolic-ref",
            &["HEAD", "refs/heads/other"],
            &[],
        )
        .unwrap();
        let http = git_checkout_http(&origin.url);
        let wc = do_git_checkout(&http, &dir);
        assert_eq!(wc.backend, Backend::Git);
        assert_eq!(wc.base_rev, main_head);
        assert_eq!(read_file(&wc.root, "foo.spec"), "Name: foo\n");
        let reopened = open(&wc.root).unwrap();
        assert_eq!(reopened, wc);
    }

    #[test]
    fn checkout_git_clones_with_an_empty_fragment() {
        let dir = TempDirGuard::new("git-checkout-empty-frag");
        let origin = git_origin(&dir.path);
        let main_head = origin.head.clone();
        // An empty fragment names no branch: the clone takes the
        // origin's default, like a URL with no fragment at all.
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/home:alice/foo/_meta",
            &format!(
                "<package name=\"foo\" project=\"home:alice\"><scmsync>{}#</scmsync></package>",
                origin.url
            ),
        );
        let wc = do_git_checkout(&http, &dir);
        assert_eq!(wc.backend, Backend::Git);
        assert_eq!(wc.base_rev, main_head);
        assert_eq!(read_file(&wc.root, "foo.spec"), "Name: foo\n");
        let _ = origin;
    }

    #[test]
    fn checkout_refuses_rev_for_a_git_backed_package() {
        let dir = TempDirGuard::new("git-checkout-rev");
        let origin = git_origin(&dir.path);
        let http = git_checkout_http(&origin.url);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = checkout(
            &client,
            "home:alice",
            "foo",
            Some(&RevisionSpec::parse("2").unwrap()),
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let _ = origin;
    }

    #[test]
    fn commit_git_pushes_and_polls_the_sync() {
        let dir = TempDirGuard::new("git-commit");
        let origin = git_origin(&dir.path);
        let http = git_checkout_http(&origin.url);
        let mut wc = do_git_checkout(&http, &dir);
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local change"]);
        let head = git(&wc.root, "rev-parse", &["HEAD"], &[])
            .unwrap()
            .trim()
            .to_string();
        // The OBS side already synced the pushed head: the poll
        // finds it on the first read.
        let http = http.with_get(
            "https://api.opensuse.org/public/source/home:alice/foo/_scmsync.obsinfo",
            &format!("mtime: 1\ncommit: {head}\nurl: https://example.com/foo\n"),
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let auth = test_auth();
        let report = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            false,
            Some(&auth),
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(report.pushed_head, Some(head.clone()));
        assert_eq!(report.rev, None);
        assert_eq!(open(&wc.root).unwrap().base_rev, head);
        // The push landed in the origin.
        let origin_head = git(&origin.path, "rev-parse", &["main"], &[])
            .unwrap()
            .trim()
            .to_string();
        assert_eq!(origin_head, head);
    }

    #[test]
    fn commit_git_base_record_failure_names_the_pushed_head() {
        // The push lands, then the base record update fails: a
        // stale `.git/config.lock` makes `git config --local`
        // refuse the write. The error names the pushed head: the
        // agent must not push again.
        let dir = TempDirGuard::new("git-commit-lock");
        let origin = git_origin(&dir.path);
        let http = git_checkout_http(&origin.url);
        let mut wc = do_git_checkout(&http, &dir);
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local change"]);
        let head = git(&wc.root, "rev-parse", &["HEAD"], &[])
            .unwrap()
            .trim()
            .to_string();
        // The OBS side already synced the pushed head: the poll
        // finds it on the first read.
        let http = http.with_get(
            "https://api.opensuse.org/public/source/home:alice/foo/_scmsync.obsinfo",
            &format!("mtime: 1\ncommit: {head}\nurl: https://example.com/foo\n"),
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let auth = test_auth();
        let old_base = wc.base_rev.clone();
        std::fs::write(wc.root.join(".git").join("config.lock"), "").unwrap();
        let err = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            false,
            Some(&auth),
            &DryRun::new(false),
        )
        .unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains(&format!("the push landed (head {head})")),
            "message: {message}"
        );
        // The base is unrefreshed: the commit did not complete.
        assert_eq!(open(&wc.root).unwrap().base_rev, old_base);
        let _ = dir;
    }

    /// Sets an environment variable, restoring the previous value
    /// (or removing it) on drop: a fixed git date must not leak
    /// into later tests.
    struct RestoreVar(&'static str, Option<std::ffi::OsString>);

    impl RestoreVar {
        fn set(name: &'static str, value: &str) -> Self {
            let old = std::env::var_os(name);
            std::env::set_var(name, value);
            RestoreVar(name, old)
        }
    }

    impl Drop for RestoreVar {
        fn drop(&mut self) {
            match self.1.take() {
                Some(old) => std::env::set_var(self.0, old),
                None => std::env::remove_var(self.0),
            }
        }
    }

    /// Serializes the tests that pin `GIT_AUTHOR_DATE` /
    /// `GIT_COMMITTER_DATE`: the dates are process-global, so a
    /// guard drop in one test would remove them mid-commit in
    /// the other and change its SHA.
    static GIT_DATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn commit_git_synthesizes_a_commit_for_staged_changes() {
        // Fixed author and committer dates: the synthesized
        // commit's SHA is deterministic, so the OBS-side sync
        // read can name it in advance. The dates are restored on
        // drop, so no later test inherits them. The lock
        // serializes the two date-pinning tests: the dates are
        // process-global, so a guard drop in one test would
        // otherwise remove them mid-commit in the other.
        let _date_lock = GIT_DATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _author_date = RestoreVar::set("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z");
        let _committer_date = RestoreVar::set("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z");
        let dir = TempDirGuard::new("git-commit-synth");
        let origin = git_origin(&dir.path);
        let http = git_checkout_http(&origin.url);
        let mut wc = do_git_checkout(&http, &dir);
        write_file(&wc.root, "foo.spec", "Name: staged\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        // The synthesized commit, computed in advance: the staged
        // tree over the base head, with the fixed dates and the
        // message the commit call will use.
        let head = git(&wc.root, "rev-parse", &["HEAD"], &[])
            .unwrap()
            .trim()
            .to_string();
        let tree = git(&wc.root, "write-tree", &[], &[])
            .unwrap()
            .trim()
            .to_string();
        let expected = git(
            &wc.root,
            "commit-tree",
            &[&tree, "-p", &head, "-m", "test"],
            &[],
        )
        .unwrap()
        .trim()
        .to_string();
        let http = http.with_get(
            "https://api.opensuse.org/public/source/home:alice/foo/_scmsync.obsinfo",
            &format!("mtime: 1\ncommit: {expected}\nurl: https://example.com/foo\n"),
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let auth = test_auth();
        let report = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            true,
            Some(&auth),
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(report.pushed_head, Some(expected.clone()));
        assert_eq!(open(&wc.root).unwrap().base_rev, expected);
        let origin_head = git(&origin.path, "rev-parse", &["main"], &[])
            .unwrap()
            .trim()
            .to_string();
        assert_eq!(origin_head, expected);
    }

    #[test]
    fn commit_git_skips_the_outdated_check_without_an_upstream() {
        let dir = TempDirGuard::new("git-no-upstream");
        let origin = git_origin(&dir.path);
        let http = git_checkout_http(&origin.url);
        let mut wc = do_git_checkout(&http, &dir);
        // The branch has no upstream: the fetch still works, but
        // `@{u}` does not resolve, so the outdated check is
        // skipped instead of failing.
        git_ok(&wc.root, &["branch", "--unset-upstream"]);
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local change"]);
        let head = git(&wc.root, "rev-parse", &["HEAD"], &[])
            .unwrap()
            .trim()
            .to_string();
        let http = http.with_get(
            "https://api.opensuse.org/public/source/home:alice/foo/_scmsync.obsinfo",
            &format!("mtime: 1\ncommit: {head}\nurl: https://example.com/foo\n"),
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let auth = test_auth();
        let report = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            true,
            Some(&auth),
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(report.pushed_head, Some(head));
        let _ = origin;
    }

    #[test]
    fn commit_git_refuses_when_the_remote_moved() {
        let dir = TempDirGuard::new("git-outdated");
        let origin = git_origin(&dir.path);
        let http = git_checkout_http(&origin.url);
        let mut wc = do_git_checkout(&http, &dir);
        // The remote moves: a second clone pushes a new commit.
        let work = dir.path.join("work");
        write_file(&work, "foo.spec", "Name: remote\n");
        git_ok(&work, &["add", "-A"]);
        git_ok(&work, &["commit", "--quiet", "-m", "remote change"]);
        git_ok(&work, &["push", "--quiet", "origin", "main"]);
        // The checkout diverges locally instead of updating.
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local change"]);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let auth = test_auth();
        let err = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            true,
            Some(&auth),
            &DryRun::new(false),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::WorkingCopyOutdated);
    }

    #[test]
    fn update_git_rebases_onto_the_upstream_head() {
        let dir = TempDirGuard::new("git-update");
        let origin = git_origin(&dir.path);
        let http = git_checkout_http(&origin.url);
        let mut wc = do_git_checkout(&http, &dir);
        // The remote moves: a second clone pushes a new commit.
        let work = dir.path.join("work");
        write_file(&work, "foo.spec", "Name: remote\n");
        git_ok(&work, &["add", "-A"]);
        git_ok(&work, &["commit", "--quiet", "-m", "remote change"]);
        git_ok(&work, &["push", "--quiet", "origin", "main"]);
        let new_head = git(&work, "rev-parse", &["HEAD"], &[])
            .unwrap()
            .trim()
            .to_string();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let report = update(&mut wc, &client, None, None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(report.updated);
        assert_eq!(report.new_base_rev, new_head);
        assert!(report.conflicts.is_empty());
        assert_eq!(read_file(&wc.root, "foo.spec"), "Name: remote\n");
        assert_eq!(open(&wc.root).unwrap().base_rev, new_head);
        let _ = origin;
    }

    #[test]
    fn update_git_stops_on_a_conflict() {
        let dir = TempDirGuard::new("git-update-conflict");
        let origin = git_origin(&dir.path);
        let initial = origin.head.clone();
        let http = git_checkout_http(&origin.url);
        let mut wc = do_git_checkout(&http, &dir);
        // The remote moves on the same line the local commit
        // changed: the rebase stops with the conflict in the tree.
        let work = dir.path.join("work");
        write_file(&work, "foo.spec", "Name: remote\n");
        git_ok(&work, &["add", "-A"]);
        git_ok(&work, &["commit", "--quiet", "-m", "remote change"]);
        git_ok(&work, &["push", "--quiet", "origin", "main"]);
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local change"]);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let report = update(&mut wc, &client, None, None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(!report.updated);
        assert_eq!(report.conflicts, vec!["foo.spec".to_string()]);
        assert!(rebase_in_progress(&wc).unwrap());
        assert_eq!(open(&wc.root).unwrap().base_rev, initial);
        let _ = origin;
    }

    #[test]
    fn resolve_adopts_the_new_base_on_a_git_backed_checkout() {
        let dir = TempDirGuard::new("git-resolve-base");
        let origin = git_origin(&dir.path);
        let http = git_checkout_http(&origin.url);
        let mut wc = do_git_checkout(&http, &dir);
        // The remote moves on the same line the local commit
        // changed, and adds bar.txt on its own: the rebase stops
        // with the conflict.
        let work = dir.path.join("work");
        write_file(&work, "foo.spec", "Name: remote\n");
        write_file(&work, "bar.txt", "bar\n");
        git_ok(&work, &["add", "-A"]);
        git_ok(&work, &["commit", "--quiet", "-m", "remote change"]);
        git_ok(&work, &["push", "--quiet", "origin", "main"]);
        let new_head = git(&work, "rev-parse", &["HEAD"], &[])
            .unwrap()
            .trim()
            .to_string();
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local change"]);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let report = update(&mut wc, &client, None, None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert_eq!(report.conflicts, vec!["foo.spec".to_string()]);
        write_file(&wc.root, "foo.spec", "Name: resolved\n");
        let resolved = resolve(&mut wc, &DryRun::new(false)).unwrap().unwrap();
        assert_eq!(resolved, vec!["foo.spec".to_string()]);
        assert!(!rebase_in_progress(&wc).unwrap());
        // The record adopted the new base, on disk and in this
        // checkout: the upstream-only bar.txt is not local work.
        assert_eq!(open(&wc.root).unwrap().base_rev, new_head);
        assert_eq!(wc.base_rev, new_head);
        let found: Vec<(String, StatusCode)> = status(&wc)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.path, entry.code))
            .collect();
        assert_eq!(found, vec![("foo.spec".to_string(), StatusCode::Modified)]);
        let _ = origin;
    }

    #[test]
    fn update_git_keeps_the_reused_checkout_current() {
        let dir = TempDirGuard::new("git-update-struct");
        let origin = git_origin(&dir.path);
        let http = git_checkout_http(&origin.url);
        let mut wc = do_git_checkout(&http, &dir);
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local change"]);
        // The remote adds bar.txt on its own: the update rebases
        // the local commit onto it without conflict.
        let work = dir.path.join("work");
        write_file(&work, "bar.txt", "bar\n");
        git_ok(&work, &["add", "-A"]);
        git_ok(&work, &["commit", "--quiet", "-m", "remote file"]);
        git_ok(&work, &["push", "--quiet", "origin", "main"]);
        let new_head = git(&work, "rev-parse", &["HEAD"], &[])
            .unwrap()
            .trim()
            .to_string();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let report = update(&mut wc, &client, None, None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(report.updated);
        // The reused checkout's recorded base moved with the
        // update: status compares against the new base, so the
        // upstream-only bar.txt is not local work.
        assert_eq!(wc.base_rev, new_head);
        assert_eq!(open(&wc.root).unwrap().base_rev, new_head);
        let found: Vec<(String, StatusCode)> = status(&wc)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.path, entry.code))
            .collect();
        assert_eq!(found, vec![("foo.spec".to_string(), StatusCode::Modified)]);
        let _ = origin;
    }

    #[test]
    fn update_git_refuses_rev() {
        let dir = TempDirGuard::new("git-update-rev");
        let origin = git_origin(&dir.path);
        let http = git_checkout_http(&origin.url);
        let mut wc = do_git_checkout(&http, &dir);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = update(&mut wc, &client, Some("2"), None, &DryRun::new(false)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let _ = origin;
    }

    #[test]
    fn update_refuses_during_a_rebase() {
        let http = checkout_http()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
                &listing_rev2(),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=2",
                b"Name: server\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=2",
                b"",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local spec"]);
        update(&mut wc, &client, Some("2"), None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(rebase_in_progress(&wc).unwrap());
        let err = update(&mut wc, &client, Some("2"), None, &DryRun::new(false)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::FileConflict);
        let _ = dir;
    }

    #[test]
    fn resolve_stages_only_the_conflicted_files() {
        let http = checkout_http()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
                &listing_rev2(),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=2",
                b"Name: server\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=2",
                b"",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "bar.txt", "local\n");
        add(&wc, &["bar.txt".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local bar"]);
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local spec"]);
        let report = update(&mut wc, &client, Some("2"), None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert_eq!(report.conflicts, vec!["foo.spec".to_string()]);
        // An unrelated unstaged edit to a tracked file.
        write_file(&wc.root, "bar.txt", "local\nunrelated\n");
        write_file(&wc.root, "foo.spec", "Name: resolved\n");
        let resolved = resolve(&mut wc, &DryRun::new(false)).unwrap().unwrap();
        assert_eq!(resolved, vec!["foo.spec".to_string()]);
        assert!(!rebase_in_progress(&wc).unwrap());
        // The resolution was committed by the continued rebase,
        // and bar.txt's unrelated edit stayed unstaged: with the
        // old stage-everything behaviour it would be committed.
        assert_eq!(
            git(&wc.root, "show", &["HEAD:foo.spec"], &[]).unwrap(),
            "Name: resolved\n"
        );
        let cached = git(
            &wc.root,
            "diff",
            &["--cached", "--name-only", "-z", "HEAD"],
            &[],
        )
        .unwrap();
        assert!(split_nul(&cached).is_empty());
        let worktree = git(&wc.root, "diff", &["--name-only", "-z"], &[]).unwrap();
        assert_eq!(split_nul(&worktree), vec!["bar.txt".to_string()]);
        assert_eq!(open(&wc.root).unwrap().base_rev, "2");
        let _ = dir;
    }

    const LINK_OLD_BASEREV: &str = "dddddddddddddddddddddddddddddddd";
    const TARGET_SRCMD5: &str = "cccccccccccccccccccccccccccccccc";
    const EXPANDED_SRCMD5: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

    fn link_xml(baserev: &str) -> String {
        format!("<link project=\"home:alice\" package=\"bar\" baserev=\"{baserev}\"/>")
    }

    fn pull_http() -> FakeHttp {
        let foo_rev1 = format!(
            "<directory name=\"foo\" rev=\"1\" srcmd5=\"{SRCMD5_1}\" vrev=\"1\">\
            <entry md5=\"235ee045c185e683ce770f81607ade06\" mtime=\"1282047302\" name=\"_link\" size=\"95\" />\
            </directory>"
        );
        // The expanded listing never carries `_link`: the server's
        // expanded tree excludes it, so pull fetches it through the
        // unexpanded route.
        let foo_expanded = format!(
            "<directory name=\"foo\" rev=\"1\" srcmd5=\"{EXPANDED_SRCMD5}\" vrev=\"1\">\
            <entry md5=\"cf0d04a286833ebd00a5f5b2d54474a9\" mtime=\"1282047303\" name=\"bar.spec\" size=\"10\" />\
            </directory>"
        );
        // The pulled checkout holds `_link`, so the commit after
        // the pull sends `keeplink=1` on both passes.
        let validate_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv&withvalidate=1&keeplink=1&noservice=1";
        let final_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv&keeplink=1&noservice=1";
        FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo/_meta",
                "<package name=\"foo\" project=\"home:alice\"></package>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo",
                &foo_rev1,
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/_link?rev=1",
                link_xml(LINK_OLD_BASEREV).as_bytes(),
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/bar",
                &format!(
                    "<directory name=\"bar\" rev=\"5\" srcmd5=\"{TARGET_SRCMD5}\"></directory>"
                ),
            )
            .with_get(
                "https://api.opensuse.org/source/home:alice/foo?expand=1",
                &foo_expanded,
            )
            .with_get_to_file(
                "https://api.opensuse.org/source/home:alice/foo/bar.spec?expand=1&rev=1",
                b"Name: bar\n",
            )
            .with_post(
                validate_url,
                    "<directory error=\"missing\" name=\"foo\"><entry md5=\"cf0d04a286833ebd00a5f5b2d54474a9\" name=\"bar.spec\" /><entry md5=\"90b55d54a3d954ca5d8b6985adcd8d46\" name=\"_link\" /></directory>",
                )
            .with_post(
                final_url,
                &format!("<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\" />"),
            )
            .with_put_bytes(
                "https://api.opensuse.org/source/home:alice/foo/bar.spec?rev=repository",
                "<revision rev=\"repository\"><srcmd5>empty</srcmd5></revision>",
            )
            .with_put_bytes(
                "https://api.opensuse.org/source/home:alice/foo/_link?rev=repository",
                "<revision rev=\"repository\"><srcmd5>empty</srcmd5></revision>",
            )
    }

    #[test]
    fn wait_for_revision_retries_until_a_revision_appears() {
        let mut attempts = 0;
        let (value, rev) = wait_for_revision(
            || -> Result<(i32, Option<String>), GeectlError> {
                attempts += 1;
                Ok(if attempts < 3 {
                    (attempts, None)
                } else {
                    (attempts, Some("7".to_string()))
                })
            },
            "home:alice",
            "foo",
            Duration::from_secs(10),
            Duration::from_millis(1),
        )
        .unwrap();
        assert_eq!(value, 3);
        assert_eq!(rev, "7");
        assert_eq!(attempts, 3);
    }

    #[test]
    fn wait_for_revision_errors_after_the_budget_is_spent() {
        let err = wait_for_revision(
            || Ok::<((), Option<String>), GeectlError>(((), None)),
            "home:alice",
            "foo",
            Duration::from_millis(10),
            Duration::from_millis(1),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        let message = err.to_string();
        assert!(
            message.contains("did not return a revision for home:alice/foo"),
            "message: {message}"
        );
    }

    #[test]
    fn wait_for_revision_does_not_retry_a_fetch_failure() {
        // A fetch failure fails at once: only a missing revision
        // retries, so one attempt is all a failing fetch gets.
        let mut attempts = 0;
        let err = wait_for_revision(
            || -> Result<((), Option<String>), GeectlError> {
                attempts += 1;
                Err(GeectlError::new(ErrorKind::ServerError, "boom"))
            },
            "home:alice",
            "foo",
            Duration::from_secs(60),
            Duration::from_secs(2),
        )
        .unwrap_err();
        assert_eq!(attempts, 1);
        assert_eq!(err.kind(), ErrorKind::ServerError);
        let message = err.to_string();
        assert!(message.contains("boom"), "message: {message}");
    }

    #[test]
    fn pull_moves_the_link_and_keeps_the_unexpanded_base() {
        let http = pull_http();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dir = TempDirGuard::new("pull");
        let mut wc = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        let report = pull(&mut wc, &client, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(report.updated);
        assert_eq!(report.old_baserev, LINK_OLD_BASEREV);
        assert_eq!(report.new_baserev, TARGET_SRCMD5);
        assert_eq!(report.old_base_rev, "1");
        assert_eq!(report.new_base_rev, "1");
        // The record keeps the link package's own, unexpanded
        // coordinates — not the expanded listing's srcmd5.
        let reopened = open(&wc.root).unwrap();
        assert_eq!(reopened.base_rev, "1");
        assert_eq!(reopened.base_srcmd5, SRCMD5_1);
        assert_eq!(wc.base_rev, "1");
        assert_eq!(wc.base_srcmd5, SRCMD5_1);
        // The downloaded _link carries the new baserev.
        let link = String::from_utf8(index_content(&wc, "_link").unwrap()).unwrap();
        assert!(
            link.contains(&format!("baserev=\"{TARGET_SRCMD5}\"")),
            "link: {link}"
        );
        assert_eq!(read_file(&wc.root, "bar.spec"), "Name: bar\n");
        // A second pull is a no-op: the link already points at the
        // target's latest revision.
        let again = pull(&mut wc, &client, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(!again.updated);
        // And the next commit does not refuse as outdated: the
        // recorded base still agrees with the server's unexpanded
        // revision.
        write_file(&wc.root, "bar.spec", "Name: bar2\n");
        add(&wc, &["bar.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        let report = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            true,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(report.rev, Some(2));
        assert_eq!(open(&wc.root).unwrap().base_rev, "2");
        assert_eq!(wc.base_rev, "2");
        assert_eq!(wc.base_srcmd5, SRCMD5_2);
        // The commit stages what the validate pass reported missing:
        // the edited spec and the rewritten link.
        let puts = http.put_bytes_calls();
        assert_eq!(puts.len(), 2);
        assert_eq!(
            puts[0].0,
            "https://api.opensuse.org/source/home:alice/foo/bar.spec?rev=repository"
        );
        assert_eq!(puts[0].1, b"Name: bar2\n".to_vec());
        assert!(puts[0].2);
        assert_eq!(
            puts[1].0,
            "https://api.opensuse.org/source/home:alice/foo/_link?rev=repository"
        );
        assert_eq!(puts[1].1, link_xml(TARGET_SRCMD5).into_bytes());
        assert!(puts[1].2);
    }

    #[test]
    fn pull_retries_an_expanded_listing_without_a_revision() {
        // The expanded listing can come back without a revision:
        // the first read has none, the retry reads it, and the
        // pull completes instead of failing on the first read.
        // Once the sequence is spent the canned listing answers.
        let no_rev = format!(
            "<directory name=\"foo\" srcmd5=\"{EXPANDED_SRCMD5}\" vrev=\"1\">\
            <entry md5=\"cf0d04a286833ebd00a5f5b2d54474a9\" mtime=\"1282047303\" name=\"bar.spec\" size=\"10\" />\
            </directory>"
        );
        let http = pull_http().with_get_sequence(
            "https://api.opensuse.org/source/home:alice/foo?expand=1",
            &[&no_rev],
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dir = TempDirGuard::new("pull-no-rev");
        let mut wc = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        let report = pull(&mut wc, &client, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(report.updated);
        assert_eq!(report.new_baserev, TARGET_SRCMD5);
        // The expanded listing was read twice — the missing
        // revision, then the retry.
        let expanded_reads = http
            .get_calls()
            .into_iter()
            .filter(|(url, _)| url == "https://api.opensuse.org/source/home:alice/foo?expand=1")
            .count();
        assert_eq!(expanded_reads, 2);
        let _ = dir;
    }

    #[test]
    fn pull_pins_the_expanded_downloads_by_an_md5_revision() {
        // The expanded listing of a linked package carries the
        // expanded content's md5 as its revision, not a number:
        // the pull pins the expanded downloads by revision spec,
        // which accepts the md5, and the `_link` download stays
        // pinned to the link package's own numeric revision.
        let md5_expanded = format!(
            "<directory name=\"foo\" rev=\"{EXPANDED_SRCMD5}\" srcmd5=\"{EXPANDED_SRCMD5}\" vrev=\"1\">\
            <entry md5=\"cf0d04a286833ebd00a5f5b2d54474a9\" mtime=\"1282047303\" name=\"bar.spec\" size=\"10\" />\
            </directory>"
        );
        let http = pull_http()
            .with_get_sequence(
                "https://api.opensuse.org/source/home:alice/foo?expand=1",
                &[&md5_expanded],
            )
            .with_get_to_file(
                &format!(
                    "https://api.opensuse.org/source/home:alice/foo/bar.spec?expand=1&rev={EXPANDED_SRCMD5}"
                ),
                b"Name: bar\n",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dir = TempDirGuard::new("pull-md5-rev");
        let mut wc = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        let report = pull(&mut wc, &client, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(report.updated);
        assert_eq!(report.new_baserev, TARGET_SRCMD5);
        // The expanded file downloaded pinned to the md5, and the
        // `_link` download stayed pinned to the own revision:
        // checkout downloaded it once, the pull once more.
        let file_downloads: Vec<String> = http
            .get_to_file_calls()
            .into_iter()
            .map(|(url, _)| url)
            .collect();
        assert!(
            file_downloads.contains(
                &format!(
                    "https://api.opensuse.org/source/home:alice/foo/bar.spec?expand=1&rev={EXPANDED_SRCMD5}"
                )
            ),
            "expanded download was not pinned to the md5: {file_downloads:?}"
        );
        assert_eq!(
            file_downloads
                .iter()
                .filter(|url| url.as_str()
                    == "https://api.opensuse.org/public/source/home:alice/foo/_link?rev=1")
                .count(),
            2,
            "_link download was not pinned to the own revision: {file_downloads:?}"
        );
        let _ = dir;
    }

    #[test]
    fn pull_retries_a_missing_own_revision() {
        // The checkout reads the link package's directory first, so
        // the sequence leads with the revision present: the pull's
        // first own read then comes back without one, and the retry
        // reads it. Once the sequence is spent the canned directory
        // answers.
        let foo_rev1 = format!(
            "<directory name=\"foo\" rev=\"1\" srcmd5=\"{SRCMD5_1}\" vrev=\"1\">\
            <entry md5=\"235ee045c185e683ce770f81607ade06\" mtime=\"1282047302\" name=\"_link\" size=\"95\" />\
            </directory>"
        );
        let foo_no_rev = format!("<directory name=\"foo\" srcmd5=\"{SRCMD5_1}\"></directory>");
        let http = pull_http().with_get_sequence(
            "https://api.opensuse.org/public/source/home:alice/foo",
            &[&foo_rev1, &foo_no_rev],
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dir = TempDirGuard::new("pull-own-no-rev");
        let mut wc = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        let report = pull(&mut wc, &client, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(report.updated);
        assert_eq!(report.new_baserev, TARGET_SRCMD5);
        // The checkout read the directory once; the pull read it
        // twice — the missing revision, then the retry.
        let own_reads = http
            .get_calls()
            .into_iter()
            .filter(|(url, _)| url == "https://api.opensuse.org/public/source/home:alice/foo")
            .count();
        assert_eq!(own_reads, 3);
        let _ = dir;
    }

    #[test]
    fn pull_dry_run_refuses_a_git_backed_checkout() {
        let dir = TempDirGuard::new("pull-dry-git");
        let origin = git_origin(&dir.path);
        let http = git_checkout_http(&origin.url);
        let mut wc = do_git_checkout(&http, &dir);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = pull(&mut wc, &client, &DryRun::new(true)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let _ = origin;
    }

    #[test]
    fn pull_dry_run_refuses_an_unlinked_package() {
        let http = checkout_http();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        let err = pull(&mut wc, &client, &DryRun::new(true)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let _ = dir;
    }

    #[test]
    fn pull_dry_run_lists_the_files_it_would_change() {
        let http = pull_http();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dir = TempDirGuard::new("pull-dry");
        let mut wc = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        let dry_run = DryRun::new(true);
        let out = pull(&mut wc, &client, &dry_run).unwrap();
        assert!(out.is_none());
        let plan = dry_run.plan();
        assert!(
            plan.iter().any(|line| line.contains(&format!(
                "would pull home:alice/foo: link baserev {LINK_OLD_BASEREV} -> {TARGET_SRCMD5}"
            ))),
            "plan: {plan:?}"
        );
        assert!(
            plan.iter().any(|line| line.contains("would add bar.spec")),
            "plan: {plan:?}"
        );
        // The link itself is rewritten, not deleted: the expanded
        // listing never carries it, so it must not show as a delete.
        assert!(
            !plan.iter().any(|line| line.contains("would delete _link")),
            "plan: {plan:?}"
        );
        // Nothing was pulled: the working tree still holds only
        // the link.
        assert!(!wc.root.join("bar.spec").exists());
        let _ = dir;
    }

    #[test]
    fn pull_refuses_during_a_rebase() {
        let http = checkout_http()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
                &listing_rev2(),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=2",
                b"Name: server\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=2",
                b"",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        // The package links bar: the link is committed first, so
        // the rebase refusal is the one pull hits, not the
        // not-linked refusal.
        write_file(&wc.root, "_link", &link_xml(LINK_OLD_BASEREV));
        add(&wc, &["_link".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "link bar"]);
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local spec"]);
        let report = update(&mut wc, &client, Some("2"), None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert_eq!(report.conflicts, vec!["foo.spec".to_string()]);
        let err = pull(&mut wc, &client, &DryRun::new(false)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::FileConflict);
        let _ = dir;
    }

    #[test]
    fn checkout_dry_run_lists_the_files_it_would_download() {
        let http = checkout_http();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dir = TempDirGuard::new("dry-checkout");
        let dry_run = DryRun::new(true);
        let out = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            None,
            &dry_run,
        )
        .unwrap();
        assert!(out.is_none());
        let plan = dry_run.plan();
        assert!(
            plan.iter()
                .any(|line| line.contains("would download foo.spec")),
            "plan: {plan:?}"
        );
        assert!(
            plan.iter()
                .any(|line| line.contains("would download foo.tar.gz")),
            "plan: {plan:?}"
        );
        assert!(!dir.path.join("home:alice").join("foo").exists());
    }

    #[test]
    fn commit_dry_run_lists_files_to_send_and_delete() {
        let http = checkout_http();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "foo.spec", "Name: bar\n");
        write_file(&wc.root, "bar.txt", "new file\n");
        add(
            &wc,
            &["foo.spec".to_string(), "bar.txt".to_string()],
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        std::fs::remove_file(wc.root.join("foo.tar.gz")).unwrap();
        add(&wc, &["foo.tar.gz".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        let dry_run = DryRun::new(true);
        let out = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            false,
            None,
            &dry_run,
        )
        .unwrap();
        assert!(out.is_none());
        let plan = dry_run.plan();
        assert!(
            plan.iter().any(|line| line.contains("would send bar.txt")),
            "plan: {plan:?}"
        );
        assert!(
            plan.iter().any(|line| line.contains("would send foo.spec")),
            "plan: {plan:?}"
        );
        assert!(
            plan.iter()
                .any(|line| line.contains("would delete foo.tar.gz")),
            "plan: {plan:?}"
        );
        let _ = dir;
    }

    #[test]
    fn commit_dry_run_refuses_an_outdated_base() {
        // The checkout is at revision 1; the server has moved to
        // revision 2 since. The dry-run commit refuses, as the live
        // commit would, instead of planning a commit that would be
        // refused as outdated.
        let http = checkout_http().with_get_sequence(
            "https://api.opensuse.org/public/source/home:alice/foo",
            &[&listing_rev1(), &listing_rev2()],
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "foo.spec", "Name: bar\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        let dry_run = DryRun::new(true);
        let err = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            false,
            None,
            &dry_run,
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::WorkingCopyOutdated);
        let _ = dir;
    }

    #[test]
    fn commit_git_dry_run_refuses_when_behind_the_last_fetched_upstream() {
        let dir = TempDirGuard::new("git-commit-dry-outdated");
        let origin = git_origin(&dir.path);
        let http = git_checkout_http(&origin.url);
        let mut wc = do_git_checkout(&http, &dir);
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        // The remote moves after the checkout: fetch the new head
        // without updating, so the checkout is behind what was last
        // fetched.
        let work = dir.path.join("work");
        write_file(&work, "foo.spec", "Name: remote\n");
        git_ok(&work, &["add", "-A"]);
        git_ok(&work, &["commit", "--quiet", "-m", "remote change"]);
        git_ok(&work, &["push", "--quiet", "origin", "main"]);
        git_ok(&wc.root, &["fetch", "origin"]);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        let err = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            true,
            None,
            &dry_run,
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::WorkingCopyOutdated);
        let _ = origin;
    }

    #[test]
    fn update_dry_run_lists_the_files_it_would_change() {
        let http = checkout_http().with_get(
            "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
            &listing_rev2(),
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        let dry_run = DryRun::new(true);
        let out = update(&mut wc, &client, Some("2"), None, &dry_run).unwrap();
        assert!(out.is_none());
        let plan = dry_run.plan();
        assert!(
            plan.iter()
                .any(|line| line.contains("would update home:alice/foo to revision 2")),
            "plan: {plan:?}"
        );
        assert!(
            plan.iter()
                .any(|line| line.contains("would change foo.spec")),
            "plan: {plan:?}"
        );
        assert!(!plan.iter().any(|line| line.contains("foo.tar.gz")));
        assert_eq!(open(&wc.root).unwrap().base_rev, REV1);
        let _ = dir;
    }

    #[test]
    fn update_dry_run_with_a_local_commit_keeps_it() {
        let http = checkout_http().with_get(
            "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
            &listing_rev2(),
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        // A local commit adds a file the server listing does not
        // have: the plan must not claim the update would delete
        // it — the update rebases it onto the new base instead.
        write_file(&wc.root, "bar.txt", "local\n");
        add(&wc, &["bar.txt".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local file"]);
        let dry_run = DryRun::new(true);
        let out = update(&mut wc, &client, Some("2"), None, &dry_run).unwrap();
        assert!(out.is_none());
        let plan = dry_run.plan();
        assert!(
            !plan
                .iter()
                .any(|line| line.contains("would delete bar.txt")),
            "plan: {plan:?}"
        );
        assert_eq!(open(&wc.root).unwrap().base_rev, REV1);
        let _ = dir;
    }

    #[test]
    fn update_dry_run_with_a_file_added_on_both_sides() {
        // Revision 2 adds bar.txt on the server, and a local
        // commit added bar.txt too. The plan names the server's
        // action — "would add" — not "would change": the file is
        // not in the base, and the live update would rebase
        // against the server's addition, not change it.
        let http = checkout_http().with_get(
            "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
            &format!(
                "<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\">\
                <entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" mtime=\"1282047303\" name=\"foo.spec\" size=\"12\" />\
                <entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" mtime=\"1282047302\" name=\"foo.tar.gz\" size=\"0\" />\
                <entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" mtime=\"1282047303\" name=\"bar.txt\" size=\"6\" />\
                </directory>"
            ),
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "bar.txt", "local\n");
        add(&wc, &["bar.txt".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local file"]);
        let dry_run = DryRun::new(true);
        let out = update(&mut wc, &client, Some("2"), None, &dry_run).unwrap();
        assert!(out.is_none());
        let plan = dry_run.plan();
        assert!(
            plan.iter().any(|line| line.contains("would add bar.txt")),
            "plan: {plan:?}"
        );
        assert!(
            !plan
                .iter()
                .any(|line| line.contains("would change bar.txt")),
            "plan: {plan:?}"
        );
        assert_eq!(open(&wc.root).unwrap().base_rev, REV1);
        let _ = dir;
    }

    #[test]
    fn update_dry_run_with_a_locally_modified_file_ignores_it() {
        // Revision 2 keeps foo.spec identical to revision 1
        // (its md5 is the md5 of "Name: foo\n"): the server never
        // touched it.
        let http = checkout_http().with_get(
            "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
            &format!(
                "<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\">\
                <entry md5=\"9444ad22a238286bca9bf649b2aef01d\" mtime=\"1282047303\" name=\"foo.spec\" size=\"10\" />\
                <entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" mtime=\"1282047302\" name=\"foo.tar.gz\" size=\"0\" />\
                </directory>"
            ),
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        // A local commit changes foo.spec, which the server never
        // touched: the plan must not claim the update would change
        // it — the update leaves its content alone.
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local spec"]);
        let dry_run = DryRun::new(true);
        let out = update(&mut wc, &client, Some("2"), None, &dry_run).unwrap();
        assert!(out.is_none());
        let plan = dry_run.plan();
        assert!(
            !plan
                .iter()
                .any(|line| line.contains("would change foo.spec")),
            "plan: {plan:?}"
        );
        assert_eq!(open(&wc.root).unwrap().base_rev, REV1);
        let _ = dir;
    }
    #[test]
    fn commit_rename_deletes_the_old_path_on_the_server() {
        // A rename staged with `git mv` is two changes under
        // `--no-renames` — the old path deleted, the new path
        // added — so the posted file list carries the new path
        // and drops the old one: the server deletes it instead
        // of keeping a stale file.
        let validate_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv&withvalidate=1";
        let final_url = "https://api.opensuse.org/source/home:alice/foo?cmd=commitfilelist&comment=test&user=scarabeus_iv";
        let http = checkout_http()
            .with_post(
                validate_url,
                "<directory error=\"missing\" name=\"foo\"><entry md5=\"9444ad22a238286bca9bf649b2aef01d\" name=\"bar.spec\" /></directory>",
            )
            .with_post(
                final_url,
                &format!("<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\" />"),
            )
            .with_put_bytes(
                "https://api.opensuse.org/source/home:alice/foo/bar.spec?rev=repository",
                "<revision rev=\"repository\"><srcmd5>empty</srcmd5></revision>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = client
            .package_requests_search_url("home:alice", "foo")
            .unwrap();
        let http = http.with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"42\" /></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        git_ok(&wc.root, &["mv", "foo.spec", "bar.spec"]);
        let report = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            false,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(report.rev, Some(2));
        assert_eq!(
            report.files,
            vec!["bar.spec".to_string(), "foo.spec".to_string()]
        );
        // Both passes send the new path and the unchanged
        // tarball, never the renamed-away path.
        let full_list = "<directory><entry name=\"bar.spec\" md5=\"9444ad22a238286bca9bf649b2aef01d\" /><entry name=\"foo.tar.gz\" md5=\"d41d8cd98f00b204e9800998ecf8427e\" /></directory>";
        let posts = http.post_calls();
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].1.as_str(), full_list);
        assert_eq!(posts[1].1.as_str(), full_list);
        let _ = dir;
    }

    #[test]
    fn update_reports_a_conflicting_autostash_and_resolve_restores_it() {
        // A dirty file the server also changed: the update
        // stashes it, rebases, and the stash pop conflicts — the
        // report carries the conflict and the remaining stash,
        // and the update is not reported as done.
        let http = checkout_http()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
                &listing_rev2(),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=2",
                b"Name: server\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=2",
                b"",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "foo.spec", "Name: local\n");
        let report = update(&mut wc, &client, Some("2"), None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(!report.updated);
        assert_eq!(report.conflicts, vec!["foo.spec".to_string()]);
        assert!(report.stash_remaining);
        // Resolve by keeping the dirty change: the autostash
        // entry pops cleanly (its change is the resolution) and
        // the stash is empty afterwards.
        write_file(&wc.root, "foo.spec", "Name: local\n");
        let resolved = resolve(&mut wc, &DryRun::new(false)).unwrap().unwrap();
        assert_eq!(resolved, vec!["foo.spec".to_string()]);
        assert_eq!(read_file(&wc.root, "foo.spec"), "Name: local\n");
        assert!(autostash_entries(&wc).unwrap().is_empty());
        let _ = dir;
    }

    #[test]
    fn addremove_skips_a_globally_excluded_file() {
        // `status` lists untracked files with `--exclude-standard`,
        // so a file matching the excludes is never in the `?` set:
        // `addremove` leaves it unstaged, and the next commit
        // does not ship it to OBS. A non-excluded new file is
        // staged, and a tracked file deleted from the worktree
        // stages as a deletion.
        let (dir, wc) = do_checkout(&checkout_http());
        std::fs::write(
            wc.root.join(".git").join("info").join("exclude"),
            "*.patch\n",
        )
        .unwrap();
        write_file(&wc.root, "fix.patch", "patch\n");
        write_file(&wc.root, "new.txt", "new\n");
        std::fs::remove_file(wc.root.join("foo.tar.gz")).unwrap();
        addremove(&wc, &DryRun::new(false)).unwrap().unwrap();
        let tracked = git(&wc.root, "ls-files", &["-z"], &[]).unwrap();
        let tracked = split_nul(&tracked);
        assert!(!tracked.contains(&"fix.patch".to_string()));
        assert!(tracked.contains(&"new.txt".to_string()));
        assert!(!tracked.contains(&"foo.tar.gz".to_string()));
        let _ = dir;
    }

    #[test]
    fn checkout_includes_a_globally_excluded_file_in_the_root_commit() {
        // The root commit is built with `git add -f -A`: a
        // downloaded file matching the user's global excludes
        // (here `*.patch`) is committed, not dropped by the
        // excludes.
        let home = TempDirGuard::new("fake-home");
        std::fs::write(home.path.join("excludes"), "*.patch\n").unwrap();
        // Gitconfig values treat `\` as an escape, so a Windows
        // path (`C:\...`) fails to parse; git on Windows accepts
        // forward slashes. HOME is process-wide, so this config is
        // visible to every test in this process: it must parse
        // wherever a parallel test's `git init` reads it.
        let excludes = home
            .path
            .join("excludes")
            .to_string_lossy()
            .replace('\\', "/");
        std::fs::write(
            home.path.join(".gitconfig"),
            format!(
                "[user]\n\tname = geectl\n\temail = geectl@localhost\n[core]\n\texcludesFile = {excludes}\n"
            ),
        )
        .unwrap();
        let _home_var = RestoreVar::set("HOME", home.path.to_str().unwrap());
        let listing = format!(
            "<directory name=\"foo\" rev=\"1\" srcmd5=\"{SRCMD5_1}\" vrev=\"1\">\
            <entry md5=\"9444ad22a238286bca9bf649b2aef01d\" mtime=\"1282047302\" name=\"foo.spec\" size=\"10\" />\
            <entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" mtime=\"1282047302\" name=\"foo.tar.gz\" size=\"0\" />\
            <entry md5=\"60291f7f68053968761025e399094dbe\" mtime=\"1282047302\" name=\"fix.patch\" size=\"6\" />\
            </directory>"
        );
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo/_meta",
                "<package name=\"foo\" project=\"home:alice\"></package>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo",
                &listing,
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=1",
                b"Name: foo\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=1",
                b"",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/fix.patch?rev=1",
                b"patch\n",
            );
        let (dir, wc) = do_checkout(&http);
        let tracked = git(&wc.root, "ls-files", &["-z"], &[]).unwrap();
        assert!(
            split_nul(&tracked).contains(&"fix.patch".to_string()),
            "tracked: {tracked:?}"
        );
        let _ = dir;
        let _ = home;
    }

    #[test]
    fn resolve_refuses_files_still_holding_conflict_markers() {
        // The conflicted file still holds its markers: resolve
        // refuses to stage it instead of sending the markers on
        // the next commit.
        let http = checkout_http()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
                &listing_rev2(),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=2",
                b"Name: server\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=2",
                b"",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local spec"]);
        let report = update(&mut wc, &client, Some("2"), None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert_eq!(report.conflicts, vec!["foo.spec".to_string()]);
        assert!(read_file(&wc.root, "foo.spec").contains("<<<<<<<"));
        let err = resolve(&mut wc, &DryRun::new(false)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::FileConflict);
        let message = err.to_string();
        assert!(message.contains("foo.spec"), "message: {message}");
        assert!(message.contains("conflict markers"), "message: {message}");
        assert!(rebase_in_progress(&wc).unwrap());
        let _ = dir;
    }

    #[test]
    fn resolve_accepts_a_file_holding_only_a_separator_line() {
        // A lone `=======` line is not a conflict marker: a
        // conflicted file that legitimately contains one (a
        // separator, a test fixture) still resolves.
        let http = checkout_http()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
                &listing_rev2(),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=2",
                b"Name: server\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=2",
                b"",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "foo.spec", "Name: local\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        git_ok(&wc.root, &["commit", "--quiet", "-m", "local spec"]);
        let report = update(&mut wc, &client, Some("2"), None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert_eq!(report.conflicts, vec!["foo.spec".to_string()]);
        assert!(read_file(&wc.root, "foo.spec").contains("<<<<<<<"));
        // The resolution keeps a lone separator line: no angle
        // marker is left, so resolve stages it and continues.
        write_file(&wc.root, "foo.spec", "Name: local\n=======\n");
        let resolved = resolve(&mut wc, &DryRun::new(false)).unwrap().unwrap();
        assert_eq!(resolved, vec!["foo.spec".to_string()]);
        assert!(!rebase_in_progress(&wc).unwrap());
        let _ = dir;
    }

    #[test]
    fn open_rejects_a_subdirectory_of_a_checkout() {
        let (dir, wc) = do_checkout(&checkout_http());
        let sub = wc.root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let err = open(&sub).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let message = err.to_string();
        assert!(
            message.contains("subdirectory of the checkout"),
            "message: {message}"
        );
        let _ = dir;
    }

    #[test]
    fn rm_with_a_glob_like_name_removes_only_that_file() {
        // A staged file whose name looks like a glob
        // (`file[1].txt`) is a literal pathspec: removing it
        // must not touch `file1.txt`.
        let (dir, wc) = do_checkout(&checkout_http());
        write_file(&wc.root, "file1.txt", "one\n");
        write_file(&wc.root, "file[1].txt", "bracket\n");
        add(
            &wc,
            &["file1.txt".to_string(), "file[1].txt".to_string()],
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        rm(&wc, &["file[1].txt".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(!wc.root.join("file[1].txt").exists());
        assert!(wc.root.join("file1.txt").exists());
        let _ = dir;
    }

    #[test]
    fn update_reuses_unchanged_files_from_the_base() {
        // The rev-2 listing keeps `foo.spec` and `foo.tar.gz`
        // byte-identical and adds `new.txt`: only the new file
        // is downloaded — the unchanged files come out of the
        // base object store.
        let listing_rev2 = format!(
            "<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\">\
            <entry md5=\"9444ad22a238286bca9bf649b2aef01d\" mtime=\"1282047303\" name=\"foo.spec\" size=\"10\" />\
            <entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" mtime=\"1282047302\" name=\"foo.tar.gz\" size=\"0\" />\
            <entry md5=\"9cd599a3523898e6a12e13ec787da50a\" mtime=\"1282047303\" name=\"new.txt\" size=\"4\" />\
            </directory>"
        );
        let http = checkout_http()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
                &listing_rev2,
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/new.txt?rev=2",
                b"new\n",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        let report = update(&mut wc, &client, Some("2"), None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(report.updated);
        let downloads = http.get_to_file_calls();
        assert!(
            !downloads
                .iter()
                .any(|(url, _)| url.contains("foo.spec?rev=2")),
            "downloads: {downloads:?}"
        );
        assert!(
            !downloads
                .iter()
                .any(|(url, _)| url.contains("foo.tar.gz?rev=2")),
            "downloads: {downloads:?}"
        );
        assert!(
            downloads.iter().any(|(url, _)| url
                == "https://api.opensuse.org/public/source/home:alice/foo/new.txt?rev=2"),
            "downloads: {downloads:?}"
        );
        assert_eq!(read_file(&wc.root, "new.txt"), "new\n");
        let _ = dir;
    }

    #[test]
    fn pull_refuses_an_ignored_untracked_file() {
        // The untracked file is ignored by the repo's excludes,
        // so `git status` hides it — the pull still refuses: the
        // contract is a clean tree, not a clean status.
        let http = pull_http();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dir = TempDirGuard::new("pull-dirty");
        let mut wc = checkout(
            &client,
            "home:alice",
            "foo",
            None,
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        std::fs::write(
            wc.root.join(".git").join("info").join("exclude"),
            "ignored.txt\n",
        )
        .unwrap();
        write_file(&wc.root, "ignored.txt", "junk\n");
        let err = pull(&mut wc, &client, &DryRun::new(false)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let message = err.to_string();
        assert!(
            message.contains("uncommitted changes"),
            "message: {message}"
        );
        let _ = dir;
    }

    #[test]
    fn checkout_target_rejects_dot_segments() {
        let dir = TempDirGuard::new("dot-target");
        for segment in [".", ".."] {
            let err = checkout_target(&dir.path, segment, "foo").unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
        }
        let err = checkout_target(&dir.path, "home:alice", "..").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        // The live checkout validates before computing the
        // target (and before any download): a `..` project
        // never resolves to a directory outside `dir`.
        let fake = FakeHttp::new();
        let client = ObsClient::with_credentials(&fake, basic_credentials()).unwrap();
        let err = checkout(
            &client,
            "..",
            "foo",
            None,
            &dir.path,
            None,
            &DryRun::new(false),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(!dir.path.join("foo").exists());
        let _ = dir;
    }

    #[test]
    fn commit_refuses_an_empty_message() {
        let (dir, mut wc) = do_checkout(&checkout_http());
        let http = checkout_http();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        write_file(&wc.root, "foo.spec", "Name: bar\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        for message in ["", "   "] {
            let err = commit(
                &mut wc,
                &client,
                &Config::default(),
                message,
                false,
                None,
                &DryRun::new(false),
            )
            .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(err.to_string().contains("non-empty --message"));
        }
        let _ = dir;
    }

    #[test]
    fn commit_refuses_a_link_to_a_different_project() {
        // The `_link` points at home:bob/bar while the checkout
        // is home:alice/foo: the commit refuses and names the
        // `branch_package` the agent needs, live and in the
        // dry-run plan alike.
        let link = "<link project=\"home:bob\" package=\"bar\"/>";
        let listing = format!(
            "<directory name=\"foo\" rev=\"1\" srcmd5=\"{SRCMD5_1}\" vrev=\"1\">\
            <entry md5=\"b3b44f408df58466c24a78bccde615ec\" mtime=\"1282047302\" name=\"_link\" size=\"{}\" />\
            </directory>",
            link.len()
        );
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo/_meta",
                "<package name=\"foo\" project=\"home:alice\"></package>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo",
                &listing,
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/_link?rev=1",
                link.as_bytes(),
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        for dry in [false, true] {
            let err = commit(
                &mut wc,
                &client,
                &Config::default(),
                "test",
                false,
                None,
                &DryRun::new(dry),
            )
            .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput, "dry: {dry}");
            let message = err.to_string();
            assert!(
                message.contains("branch_package home:bob/bar into home:alice/foo"),
                "dry: {dry}, message: {message}"
            );
        }
        let _ = dir;
    }

    #[test]
    fn git_error_classifies_common_failures() {
        assert_eq!(
            classify_git_failure(
                "fatal: not a git repository (or any of the parent directories): .git"
            ),
            ErrorKind::WorkingCopyNotFound
        );
        assert_eq!(
            classify_git_failure("fatal: Authentication failed for 'https://example.com/'"),
            ErrorKind::AuthFailed
        );
        assert_eq!(
            classify_git_failure(
                "ssh: Could not resolve hostname example.com: Name or service not known"
            ),
            ErrorKind::NetworkError
        );
        assert_eq!(
            classify_git_failure("fatal: unable to connect to example.com: Connection refused"),
            ErrorKind::NetworkError
        );
        assert_eq!(
            classify_git_failure("! [rejected] main -> main (non-fast-forward)"),
            ErrorKind::WorkingCopyOutdated
        );
        assert_eq!(
            classify_git_failure(
                "error: untracked working tree files would be overwritten by merge"
            ),
            ErrorKind::FileConflict
        );
        assert_eq!(
            classify_git_failure("fatal: some unknown failure"),
            ErrorKind::InternalError
        );
        assert_eq!(
            classify_git_failure("The requested URL returned error: 401"),
            ErrorKind::AuthFailed
        );
        assert_eq!(
            classify_git_failure("fatal: unable to access 'https://example.com/': The requested URL returned error: 403"),
            ErrorKind::AuthFailed
        );
        assert_eq!(
            classify_git_failure("curl: (22) The requested URL returned error: 401 Unauthorized"),
            ErrorKind::AuthFailed
        );
        assert_eq!(
            classify_git_failure("Received HTTP code 401 from proxy after CONNECT"),
            ErrorKind::AuthFailed
        );
        // A bare `401`/`403` digit run — a SHA, a file name, a
        // port — is not an auth failure.
        assert_eq!(
            classify_git_failure(
                "fatal: unable to read 401a2b3c4d5e6f7890abcdef1234567890abcdef12"
            ),
            ErrorKind::InternalError
        );
        assert_eq!(
            classify_git_failure(
                "error: pathspec 'fix-403.patch' did not match any file(s) known to git"
            ),
            ErrorKind::InternalError
        );
    }

    #[test]
    fn git_network_timeout_parsing_falls_back_to_the_default() {
        // Unset, invalid, zero, and negative values fall back to
        // the 300s default; a valid value is used as-is. The
        // parsing is tested directly: the cached value cannot be
        // re-read in-process.
        assert_eq!(parse_git_network_timeout(None), Duration::from_secs(300));
        assert_eq!(
            parse_git_network_timeout(Some("abc".to_string())),
            Duration::from_secs(300)
        );
        assert_eq!(
            parse_git_network_timeout(Some("0".to_string())),
            Duration::from_secs(300)
        );
        assert_eq!(
            parse_git_network_timeout(Some("-5".to_string())),
            Duration::from_secs(300)
        );
        assert_eq!(
            parse_git_network_timeout(Some("".to_string())),
            Duration::from_secs(300)
        );
        assert_eq!(
            parse_git_network_timeout(Some("60".to_string())),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn git_calls_ignore_a_bogus_git_dir_in_the_environment() {
        // A bogus `GIT_DIR` in the environment is scrubbed
        // before any git spawn: the calls still find this
        // repository instead of failing on the bogus path.
        let (dir, wc) = do_checkout(&checkout_http());
        {
            let _guard = RestoreVar::set("GIT_DIR", "/nonexistent/geectl-test");
            let head = git(&wc.root, "rev-parse", &["HEAD"], &[]).unwrap();
            assert!(!head.trim().is_empty());
            assert!(status(&wc).unwrap().is_empty());
        }
        let _ = dir;
    }

    #[test]
    #[cfg(unix)]
    fn commit_git_ignores_a_failing_pre_commit_hook() {
        // A failing pre-commit hook in the checkout's own hooks
        // directory: the synthesized commit skips it
        // (`--no-verify` plus the forced
        // `core.hooksPath=/dev/null`), so the commit still lands
        // and pushes. Serialized with the other date-pinning
        // test: see GIT_DATE_LOCK.
        let _date_lock = GIT_DATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _author_date = RestoreVar::set("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z");
        let _committer_date = RestoreVar::set("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z");
        let dir = TempDirGuard::new("git-hook");
        let origin = git_origin(&dir.path);
        let http = git_checkout_http(&origin.url);
        let mut wc = do_git_checkout(&http, &dir);
        write_file(&wc.root, "foo.spec", "Name: staged\n");
        add(&wc, &["foo.spec".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        let hooks = wc.root.join(".git").join("hooks");
        std::fs::write(hooks.join("pre-commit"), "#!/bin/sh\nexit 1\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                hooks.join("pre-commit"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        let head = git(&wc.root, "rev-parse", &["HEAD"], &[])
            .unwrap()
            .trim()
            .to_string();
        let tree = git(&wc.root, "write-tree", &[], &[])
            .unwrap()
            .trim()
            .to_string();
        let expected = git(
            &wc.root,
            "commit-tree",
            &[&tree, "-p", &head, "-m", "test"],
            &[],
        )
        .unwrap()
        .trim()
        .to_string();
        let http = http.with_get(
            "https://api.opensuse.org/public/source/home:alice/foo/_scmsync.obsinfo",
            &format!("mtime: 1\ncommit: {expected}\nurl: https://example.com/foo\n"),
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let auth = test_auth();
        let report = commit(
            &mut wc,
            &client,
            &Config::default(),
            "test",
            true,
            Some(&auth),
            &DryRun::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(report.pushed_head, Some(expected));
        let _ = origin;
        let _ = dir;
    }

    #[test]
    fn update_clears_a_stale_temp_directory() {
        // A leftover temp directory (and index) from a killed
        // run blocks `create_dir_all`: the next update removes
        // both before building the new base tree.
        let http = checkout_http()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
                &listing_rev2(),
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=2",
                b"Name: server\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=2",
                b"",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        let git_dir = wc.root.join(".git");
        let stale = git_dir.join("geectl-tmp");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("foo.spec"), "stale\n").unwrap();
        std::fs::write(git_dir.join("geectl-tmp-index"), "stale\n").unwrap();
        let report = update(&mut wc, &client, Some("2"), None, &DryRun::new(false))
            .unwrap()
            .unwrap();
        assert!(report.updated);
        assert_eq!(read_file(&wc.root, "foo.spec"), "Name: server\n");
        let _ = dir;
    }

    #[test]
    #[cfg(unix)]
    fn tracked_symlink_is_not_followed_by_the_service_gate() {
        // A tracked symlink to `/dev/zero`: the gate snapshot
        // records no hash for it (it hashes file content, never
        // a link target) instead of reading through the link.
        let (dir, wc) = do_checkout(&checkout_http());
        std::os::unix::fs::symlink("/dev/zero", wc.root.join("zero-link")).unwrap();
        add(&wc, &["zero-link".to_string()], &DryRun::new(false))
            .unwrap()
            .unwrap();
        let snapshot = snapshot_gate_inputs(&wc).unwrap();
        let entry = snapshot
            .hashes
            .iter()
            .find(|(name, _)| name == "zero-link")
            .unwrap();
        assert_eq!(entry.1, None);
        let _ = dir;
    }

    #[test]
    fn checkout_git_refuses_non_http_scmsync_urls() {
        for url in [
            "file:///tmp/origin#main",
            "ssh://git@example.com/foo.git#main",
            "git://example.com/foo.git#main",
            "/tmp/origin#main",
        ] {
            let dir = TempDirGuard::new("git-bad-scheme");
            let http = FakeHttp::new().with_get(
                "https://api.opensuse.org/public/source/home:alice/foo/_meta",
                &format!(
                    "<package name=\"foo\" project=\"home:alice\"><scmsync>{url}</scmsync></package>",
                ),
            );
            let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
            let err = checkout(
                &client,
                "home:alice",
                "foo",
                None,
                &dir.path,
                None,
                &DryRun::new(false),
            )
            .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput, "url: {url}");
            let message = err.to_string();
            assert!(
                message.contains("not http(s)"),
                "url: {url}, message: {message}"
            );
            let _ = dir;
        }
    }
    #[test]
    fn update_restores_the_autostash_when_the_rebase_fails_without_conflicts() {
        // The new base adds `new.txt`, which already exists
        // untracked in the checkout, and the worktree holds a dirty
        // change: the rebase fails on the untracked file with no
        // conflicts left behind. The failed update restores the
        // dirty change instead of stranding it in the stash.
        let listing_rev2 = format!(
            "<directory name=\"foo\" rev=\"2\" srcmd5=\"{SRCMD5_2}\" vrev=\"2\">\
            <entry md5=\"d3fed47ff119d916236560718c57971c\" mtime=\"1282047303\" name=\"foo.spec\" size=\"13\" />\
            <entry md5=\"d41d8cd98f00b204e9800998ecf8427e\" mtime=\"1282047302\" name=\"foo.tar.gz\" size=\"0\" />\
            <entry md5=\"9cd599a3523898e6a12e13ec787da50a\" mtime=\"1282047303\" name=\"new.txt\" size=\"4\" />\
            </directory>"
        );
        let http = checkout_http()
            .with_get(
                "https://api.opensuse.org/public/source/home:alice/foo?rev=2",
                &listing_rev2,
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.spec?rev=2",
                b"Name: server\n",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/foo.tar.gz?rev=2",
                b"",
            )
            .with_get_to_file(
                "https://api.opensuse.org/public/source/home:alice/foo/new.txt?rev=2",
                b"new\n",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let (dir, mut wc) = do_checkout(&http);
        write_file(&wc.root, "foo.spec", "Name: local\n");
        write_file(&wc.root, "new.txt", "mine\n");
        let err = update(&mut wc, &client, Some("2"), None, &DryRun::new(false)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::FileConflict);
        let message = err.to_string();
        assert!(
            message.contains("untracked working tree files would be overwritten"),
            "message: {message}"
        );
        assert!(!message.contains("stay in the stash"), "message: {message}");
        // The dirty change is back in the worktree and the stash
        // is empty: the failed update did not strand it.
        assert_eq!(read_file(&wc.root, "foo.spec"), "Name: local\n");
        assert!(autostash_entries(&wc).unwrap().is_empty());
        assert_eq!(wc.base_rev, REV1);
        let _ = dir;
    }
}
