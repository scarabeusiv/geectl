//! The local-build vertical: one podman build for quick debug.
//!
//! The skill needs a local build, and OBS supports too many build
//! backends to mirror, so geectl grows exactly one: podman, native
//! arch, for quick debug. The engine is the openSUSE `build` script,
//! reused exactly as the existing tool uses it — geectl prepares the
//! inputs (buildinfo, buildconfig, the resolved dep list, the rpmlist)
//! and reads the result, it does not reimplement container mechanics,
//! dependency installation, or RPM creation.
//!
//! The vertical owns service runs, tarball fetching, and spec-cleaner
//! runs: all three happen inside the one podman container, before
//! anything goes to the server. The server stays the source of truth
//! for service results; local is for iterating fast.
//!
//! Layout (one geectl cache dir, `$XDG_CACHE_HOME/geectl/` by
//! default, overridable by config `build.cache_dir`):
//! - `pkg/<project>/<repo>/<arch>/` — the shared package cache, hdrmd5-verified
//!   on every use, written via temp file + atomic rename.
//! - `sccache/<project>/<package>/<repo>-<arch>[-<flavor>]/` — the
//!   compiler cache, one store per build root (never shared across
//!   roots, so two concurrent builds of the same package in
//!   different roots never write one tar).
//! - `build-roots/<project>/<package>/<repo>-<arch>[-<flavor>]/` — the
//!   build root, with a `.geectl-build-root` marker file.
//!
//! Eviction is a size cap (default 20 GiB, low-water 80%) enforced at
//! build start, LRU within class, pkg files first, then sccache
//! stores, then roots, then superseded images. Locked roots, the most
//! recent root per (project, package), the current build's dep set,
//! anything used within 24 h, and the current image digest are never
//! evicted.
//!
//! Test story: PR-level tests use a stub engine (a script that records
//! its argv and emits a canned log with a `finished` line and fake
//! RPMs) plus a stub podman — no real build runs in CI. The scheduled
//! live run proves the real engine + image + cache end to end.
//!
//! Known limits, recorded here rather than hidden:
//! - The library transport caps response bodies at 10 MiB, so a dep
//!   RPM over the cap fails closed with a structured error: streaming
//!   downloads are not supported yet. Small dep sets build end to end.

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use fs2::FileExt;
use serde::Serialize;

use crate::auth::Credentials;
use crate::config::Config;
use crate::debug::Debug;
use crate::error::ErrorKind;
use crate::error::GeectlError;
use crate::obs::services::parse_services;
use crate::obs::valid_obs_name;
use crate::obs::BuildDep;
use crate::obs::ObsClient;
use crate::obs::ServiceDef;
use crate::obs::ServiceParam;

/// The default build image: Tumbleweed `latest`. One image, one
/// target — a Leap pool build stays OBS-ONLY.
pub const DEFAULT_IMAGE: &str = "registry.opensuse.org/opensuse/tumbleweed:latest";
/// The repository the default image lives in: image eviction only
/// ever touches digests of this repository, never the machine's
/// other images.
const IMAGE_REPOSITORY: &str = "registry.opensuse.org/opensuse/tumbleweed";
/// The default cache cap in GiB (low-water is 80% of it).
pub const DEFAULT_CACHE_CAP_GIB: u64 = 20;
/// The low-water mark as a fraction of the cap.
pub const LOW_WATER_NUM: u64 = 4;
/// The low-water mark as a fraction of the cap (4/5 = 80%).
pub const LOW_WATER_DEN: u64 = 5;
/// An image older than this is refreshed (days).
pub const IMAGE_REFRESH_DAYS: u64 = 7;
/// The eviction grace period: anything used within this long is never
/// evicted (seconds).
pub const GRACE_PERIOD_SECS: u64 = 24 * 3600;
/// The default build timeout (seconds).
pub const DEFAULT_BUILD_TIMEOUT_SECS: u64 = 3600;
/// How long the timeout kill waits after SIGTERM before SIGKILL.
/// Unix only — the Windows kill is immediate.
#[cfg(unix)]
const KILL_GRACE_SECS: u64 = 5;
/// How long a one-shot engine/podman probe may take (seconds):
/// `--version`, image inspect, image listing, image size, image
/// remove. A probe that cannot answer in this long is hung, not
/// slow. The image pull is not a probe — it takes the build's own
/// timeout (see `podman_pull`).
const PROBE_TIMEOUT_SECS: u64 = 60;
/// How long a `podman kill` of the service container may take
/// (seconds): the run has already failed, so a hung kill must not
/// delay the timeout error. The bound matches the 10s `stop`
/// passes to the container.
const KILL_TIMEOUT_SECS: u64 = 10;
/// How long one package-owner version probe may take (seconds):
/// the query answers in ms on a healthy host; the budget only
/// bounds a hung package manager. At most three run in sequence,
/// after `--version`, so the chain stays bounded.
const PACKAGE_PROBE_TIMEOUT_SECS: u64 = 10;
/// How long the caller waits for a drain thread to finish after
/// the child exits (seconds): the drain only has to read what is
/// already in the pipe, so this is generous — but a descendant
/// that inherited the pipe and outlives the child holds EOF open
/// forever, and the caller must not hang on it (see `take_drain`).
const DRAIN_JOIN_TIMEOUT_SECS: u64 = 2;
/// How much of a build log is read at most (bytes):
/// `finish_build` reads only the tail for its verdict parsing,
/// and `build_log` reads at most the same window unless an
/// uncapped read is asked for — a verbose build's whole log must
/// never be read into memory twice over (see `read_log_tail`).
const LOG_TAIL_CAP_BYTES: u64 = 4 * 1024 * 1024;
/// The hdrmd5 a download-on-demand binary carries in buildinfo in
/// place of the real header digest: the file has not been fetched
/// into the repository yet, so the pool has no real digest for it.
const DOD_PLACEHOLDER_HDRMD5: &str = "d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0";
/// The most the engine's captured stdout/stderr may hold (bytes);
/// past it the capture is truncated, never unbounded.
const OUTPUT_CAP_BYTES: usize = 1024 * 1024;
/// The marker file that makes a directory a geectl build root.
const ROOT_MARKER: &str = ".geectl-build-root";
/// The flock file inside a build root.
const ROOT_LOCK: &str = ".geectl-build.lock";
/// The build log file inside a build root.
const BUILD_LOG: &str = ".build.log";
/// The rpmlist file the engine takes.
const RPMLIST: &str = ".rpmlist";
/// The buildconfig file the engine takes as `--dist`.
const BUILDCONFIG: &str = ".buildconfig";
/// The build record file inside a build root.
const BUILD_RECORD: &str = ".geectl-build-record.json";
/// The image sidecar in the cache dir (last refresh per image).
const IMAGE_SIDECAR: &str = "image.json";
/// The global eviction lock in the cache dir.
const EVICTION_LOCK: &str = "eviction.lock";

/// A build request: a source directory plus an explicit repo and
/// arch. The project is explicit too: the buildinfo fetch needs it,
/// and there is no working-copy layer yet to derive it from.
#[derive(Debug, Clone)]
pub struct BuildRequest {
    /// The source directory (spec + sources).
    pub source_dir: PathBuf,
    /// The OBS project the package belongs to.
    pub project: String,
    /// The repository to build for (e.g. `standard`).
    pub repo: String,
    /// The architecture to build for; must equal the host arch.
    pub arch: String,
    /// The multibuild flavor; required when the source carries a
    /// `_multibuild` file. One invocation builds one flavor.
    pub flavor: Option<String>,
    /// Wipe the root before building (a failed root is dirty and
    /// reusing it masks the fix — the caller passes this explicitly).
    pub clean: bool,
    /// The release number, passed through to the engine (sets
    /// `BUILD_RELEASE`, which removes the local-only
    /// `python-bytecode-inconsistent-mtime` artifact).
    pub release: Option<String>,
    /// Allow build dependencies from other users' `home:` projects
    /// (their packages run code in the build root): skip the
    /// foreign-`home:` refusal for the dep set the buildinfo
    /// enumerates — the build proceeds with those deps, and the
    /// accepted foreign projects are reported on the debug channel.
    /// Without it, a foreign `home:` dep refuses with
    /// `UntrustedProject`.
    pub trust_all_projects: bool,
    /// Force an image pull even when the local image is fresh.
    pub refresh_image: bool,
    /// The alternative project the build runs against: buildinfo,
    /// buildconfig, the trust check, and dep resolution use it
    /// instead of `project`. The root and sccache stay keyed by
    /// `project`: the root is local state, and the engine
    /// reinstalls the dep set from the fresh rpmlist on every
    /// non-`--noinit` build.
    pub alternative_project: Option<String>,
    /// A build root of its own, instead of the derived cache-dir
    /// root. Parallel builds of the same package do not share one.
    pub root: Option<PathBuf>,
    /// Skip the root initialization: the engine starts the build
    /// immediately in the existing root, installing nothing. Needs
    /// an initialized root (a previous build wrote the rpmlist and
    /// buildconfig there).
    pub noinit: bool,
    /// Copy the built RPMs into this directory on a green build;
    /// the directory is created when missing.
    pub keep_rpms: Option<PathBuf>,
    /// Prefer RPMs from this directory when installing the build
    /// root: a dep whose file is there is used instead of the
    /// download, hdrmd5-verified when the buildinfo carries one.
    pub prefer_packages: Option<PathBuf>,
    /// The build timeout in seconds (default 3600): one budget for
    /// the whole build op — the image pull and the engine share it,
    /// so a slow pull leaves less time for the engine.
    pub timeout_secs: u64,
    /// Print the plan and touch nothing.
    pub dry_run: bool,
}

impl BuildRequest {
    /// A request with the defaults: no flavor, no clean, no
    /// release, no trust flag, no refresh, no alternative project,
    /// no custom root, init on, no keep/prefer dirs, the default
    /// timeout, a real build.
    pub fn new(source_dir: PathBuf, project: &str, repo: &str, arch: &str) -> Self {
        BuildRequest {
            source_dir,
            project: project.to_string(),
            repo: repo.to_string(),
            arch: arch.to_string(),
            flavor: None,
            clean: false,
            release: None,
            trust_all_projects: false,
            refresh_image: false,
            alternative_project: None,
            root: None,
            noinit: false,
            keep_rpms: None,
            prefer_packages: None,
            timeout_secs: DEFAULT_BUILD_TIMEOUT_SECS,
            dry_run: false,
        }
    }

    /// The project the server-side fetches run against: the
    /// alternative project when one was passed, else the package's
    /// own project.
    pub fn fetch_project(&self) -> &str {
        self.alternative_project.as_deref().unwrap_or(&self.project)
    }
}

/// The outcome of a build-root wipe: the root and whether it
/// carried the build-root marker. In dry-run nothing is wiped and
/// `existed` reports whether a live wipe would find a root there.
/// `entries`/`bytes` are the plan: what the wipe deletes (live) or
/// would delete (dry-run), counted recursively, excluding the lock
/// file that stays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct WipeOutcome {
    /// The root that was wiped, or would be wiped in dry-run.
    pub root: PathBuf,
    /// Whether the root carried the build-root marker.
    pub existed: bool,
    /// Whether this was a dry run (nothing wiped).
    pub dry_run: bool,
    /// Entries deleted (live) or to delete (dry-run), excluding
    /// the lock file.
    pub entries: usize,
    /// Total bytes of regular files among `entries`.
    pub bytes: u64,
}

/// The rpmlint summary read from the end of the build log. The
/// errors/warnings come from rpmlint's own summary line when the log
/// carries one; the badness is best-effort (the sum of the `Badness:`
/// annotations in the log, 0 when the log carries none).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct RpmlintSummary {
    pub errors: u64,
    pub warnings: u64,
    pub badness: u64,
}

/// The `%check` summary, when the log carries a parseable one
/// (ctest's `N% tests passed, M tests failed out of K`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct CheckSummary {
    pub passed: u64,
    pub failed: u64,
    pub total: u64,
}

/// Cache statistics for one build: pkg cache hits/misses,
/// preferred-package uses, and the sccache hit count when the log
/// carries one (best-effort, None when the engine reported none).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct CacheStats {
    pub pkg_hits: u64,
    pub pkg_misses: u64,
    /// Deps taken from the `--prefer-packages` directory instead of
    /// downloaded.
    pub pkg_preferred: u64,
    pub sccache_hits: Option<u64>,
}

/// The cache usage: total bytes and per-area bytes, plus the cap and
/// low-water mark the eviction enforces. The image size is the sum
/// of every digest of this vertical's repository, from podman when
/// it answers; when it does not, the images count as 0.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CacheUsage {
    pub cap_bytes: u64,
    pub low_water_bytes: u64,
    pub pkg_bytes: u64,
    pub sccache_bytes: u64,
    pub roots_bytes: u64,
    pub image_bytes: u64,
    pub total_bytes: u64,
}

/// One evicted item: what was deleted, how big it was, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct EvictedItem {
    /// `pkg`, `sccache`, `root`, or `image`.
    pub kind: String,
    /// The path that was deleted (an image digest for `image`).
    pub path: String,
    /// The bytes freed.
    pub bytes: u64,
    /// `lru` or `superseded-image`.
    pub reason: String,
}

/// The eviction report attached to every build record (and to a
/// `build-prune` run). `ran` is false when usage was already under
/// the cap, or when another eviction held the lock and this run
/// skipped.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct EvictionReport {
    pub ran: bool,
    pub cap_bytes: u64,
    pub low_water_bytes: u64,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub freed_bytes: u64,
    pub evicted: Vec<EvictedItem>,
    /// Deletions that failed, each naming the item and the error.
    /// A failure never blocks the build: the report carries it and
    /// the eviction continues with the next candidate.
    #[serde(default)]
    pub failures: Vec<String>,
    /// The bytes of items skipped because a protection rule applied
    /// (locked, recent, current deps, current digest), capped at the
    /// bytes above the low-water mark: the portion of the excess
    /// that could not be evicted. 0 when nothing was skipped, or
    /// when usage was already under the cap.
    pub protected_bytes: u64,
    /// The bytes still above the low-water mark after eviction (0
    /// when the cap was met).
    pub shortfall_bytes: u64,
}

/// The engine plan a dry run returns: the invocation as data, so the
/// caller can inspect it without anything being touched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct EnginePlan {
    pub image: String,
    pub root: PathBuf,
    pub repo: String,
    pub arch: String,
    pub flavor: Option<String>,
    /// The alternative project the buildinfo was fetched for, when
    /// `--alternative-project` was passed.
    pub alternative_project: Option<String>,
    /// Whether `--trust-all-projects` was passed: the plan records
    /// the flag, but a dry run evaluates no trust — the live build
    /// still refuses foreign `home:` deps without it.
    #[serde(default)]
    pub trust_all_projects: bool,
    /// The directory built RPMs are copied to on a green build,
    /// when `--keep-rpms` was passed.
    pub keep_rpms: Option<PathBuf>,
    /// The directory preferred RPMs are taken from, when
    /// `--prefer-packages` was passed.
    pub prefer_packages: Option<PathBuf>,
    /// The engine argv, engine binary first, spec last.
    pub argv: Vec<String>,
}

/// The build record: the typed result of one build. Written to
/// `<root>/.geectl-build-record.json` on every build — success,
/// failure, and timeout alike — so `build-log` can always report
/// the verdict that was read.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct BuildRecord {
    /// `green`, `failed`, or `dry-run`.
    pub verdict: String,
    pub exit_code: i32,
    pub log_path: PathBuf,
    /// The built RPMs (empty when the build failed).
    pub rpms: Vec<PathBuf>,
    /// The built RPMs copied to the `--keep-rpms` directory on a
    /// green build (empty otherwise).
    pub kept_rpms: Vec<PathBuf>,
    pub rpmlint: RpmlintSummary,
    pub checks: Option<CheckSummary>,
    /// The resolved image digest, when podman answered.
    pub image_digest: Option<String>,
    /// The engine version, when the engine reports one: its `--version`
    /// answer, else the version of the package owning the engine binary.
    pub engine_version: Option<String>,
    pub root_path: PathBuf,
    /// True when the build reused an existing root.
    pub root_reused: bool,
    pub cache: CacheStats,
    pub eviction: EvictionReport,
    /// The preinstall image filename the server offered, if any.
    /// Dropped for a rootless podman build, never installed.
    pub preinstall_image: Option<String>,
    /// The foreign `home:` projects `--trust-all-projects` accepted
    /// for this build (empty otherwise): the override is recorded
    /// here so a run that trusted foreign projects does not look
    /// like a clean one. Empty on a dry run — the plan carries the
    /// flag instead, and trust is not evaluated.
    #[serde(default)]
    pub trusted_foreign_projects: Vec<String>,
    /// True for a dry run: nothing was touched, `plan` holds the
    /// invocation.
    pub dry_run: bool,
    pub plan: Option<EnginePlan>,
}

/// The result of `build-shell`: the in-container command's exit
/// code and its captured output as data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShellResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// The result of `build-log`: the log lines plus the verdict the
/// build record holds, when a record exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuildLog {
    pub log_path: PathBuf,
    pub lines: Vec<String>,
    /// True when the log file is larger than the read cap, so the
    /// returned lines come only from its last 4 MiB. Never set on
    /// an uncapped read.
    pub truncated: bool,
    /// The verdict from the build record (`green`, `failed`), when
    /// the root carries one.
    pub verdict: Option<String>,
    pub exit_code: Option<i32>,
}

/// A local service run mode: the `osc service` command it mirrors.
/// `runall` is not a mode: it runs every service regardless of
/// mode, including `buildtime` services, and is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ServiceRunMode {
    /// `run`: every service except `buildtime`, `serveronly`,
    /// `manual`, and `disabled` — a service with an unrecognised
    /// stored mode runs.
    Run,
    /// `manualrun`: services with mode `manual` only.
    ManualRun,
    /// `precommit`: services with mode `trylocal` or `localonly`
    /// only — the set a commit runs.
    Precommit,
    /// `localrun`: `run` plus services with mode `serveronly`.
    LocalRun,
    /// `disabledrun`: services with mode `disabled` only.
    DisabledRun,
}

impl ServiceRunMode {
    /// The `osc service` command name for this mode.
    pub fn command_name(self) -> &'static str {
        match self {
            ServiceRunMode::Run => "run",
            ServiceRunMode::ManualRun => "manualrun",
            ServiceRunMode::Precommit => "precommit",
            ServiceRunMode::LocalRun => "localrun",
            ServiceRunMode::DisabledRun => "disabledrun",
        }
    }

    /// Parse an `osc service` command name. `runall` is refused:
    /// it runs every service regardless of mode.
    pub fn from_command_name(name: &str) -> Result<Self, GeectlError> {
        match name {
            "run" | "r" => Ok(ServiceRunMode::Run),
            "manualrun" | "mr" => Ok(ServiceRunMode::ManualRun),
            "precommit" => Ok(ServiceRunMode::Precommit),
            "localrun" | "lr" => Ok(ServiceRunMode::LocalRun),
            "disabledrun" | "dr" => Ok(ServiceRunMode::DisabledRun),
            "runall" | "ra" => Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "runall is refused: it runs every service regardless of mode, including buildtime services",
            )),
            _ => Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("unknown service run mode {name:?}; expected run, manualrun, precommit, localrun, or disabledrun"),
            )),
        }
    }

    /// The mode filter value the selection applies, mirroring the
    /// reference implementation's `callmode`: `None` for `run`,
    /// `"local"` for `localrun`, `"manual"` for `manualrun`,
    /// `"disabled"` for `disabledrun`, `"trylocal"` for `precommit`.
    fn callmode(self) -> Option<&'static str> {
        match self {
            ServiceRunMode::Run => None,
            ServiceRunMode::ManualRun => Some("manual"),
            ServiceRunMode::Precommit => Some("trylocal"),
            ServiceRunMode::LocalRun => Some("local"),
            ServiceRunMode::DisabledRun => Some("disabled"),
        }
    }
}

/// A service run request: a source directory (a checkout carrying
/// a `_service` file) plus the mode filter. A service run needs no
/// OBS client call: it reads the local `_service` file and runs the
/// binaries in the container.
#[derive(Debug, Clone)]
pub struct ServiceRunRequest {
    /// The source directory (the checkout the services run in).
    pub source_dir: PathBuf,
    /// The mode filter.
    pub mode: ServiceRunMode,
    /// Run only this service, with its stored mode forced to the
    /// run mode. `None` runs every service the mode selects.
    pub service: Option<String>,
    /// The service timeout in seconds (default 3600): one budget
    /// for the whole run — the image pull, the package install,
    /// and every service share it.
    pub timeout_secs: u64,
    /// Print the plan and touch nothing.
    pub dry_run: bool,
}

impl ServiceRunRequest {
    /// A request with the defaults: the default timeout, a real run.
    pub fn new(source_dir: PathBuf, mode: ServiceRunMode) -> Self {
        ServiceRunRequest {
            source_dir,
            mode,
            service: None,
            timeout_secs: DEFAULT_BUILD_TIMEOUT_SECS,
            dry_run: false,
        }
    }
}

/// The outcome of one service binary run: its exit code and its
/// captured output (capped, as in the engine run).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServiceRunOutcome {
    /// The service name from the `_service` file.
    pub name: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// One service as the dry run plans it: the name, the stored mode,
/// and the binary argv the container would run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlannedService {
    /// The service name from the `_service` file.
    pub name: String,
    /// The mode as stored (empty is the default mode).
    pub mode: String,
    /// The binary argv, service binary first, `--outdir` last.
    pub argv: Vec<String>,
}

/// The dry-run plan for a service run: the image and the services
/// that would run, in `_service` file order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServiceRunPlan {
    pub image: String,
    pub services: Vec<PlannedService>,
}

/// The record of one service run: the mode, one outcome per service
/// that ran (in `_service` file order), and the resolved image
/// digest when podman answered. A run that selects no services
/// records an empty `ran` list and starts no container; stale
/// service files are still removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServiceRunRecord {
    /// The `osc service` command name for the mode that ran.
    pub mode: String,
    pub ran: Vec<ServiceRunOutcome>,
    /// The resolved image digest, when podman answered.
    pub image_digest: Option<String>,
    /// True for a dry run: nothing was touched, `plan` holds the
    /// invocation.
    pub dry_run: bool,
    pub plan: Option<ServiceRunPlan>,
}

/// The outcome of the `pre_checkin.sh` step of a precommit run:
/// whether the script exists and ran, its exit code, and its
/// captured output (capped, as in the engine run). A missing script
/// records `ran: false` and no exit code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrecheckinOutcome {
    /// True when `pre_checkin.sh` exists in the source directory
    /// and ran.
    pub ran: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// The record of one precommit run: the service run in `precommit`
/// mode, then `pre_checkin.sh` when the source directory carries
/// one. A failing service or a failing `pre_checkin.sh` fails the
/// whole run with [`ErrorKind::ServiceFailed`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrecommitRecord {
    pub services: ServiceRunRecord,
    pub pre_checkin: PrecheckinOutcome,
    /// True for a dry run: nothing was touched.
    pub dry_run: bool,
}

/// The result of a `spec-cleaner` run: the exit code and the
/// captured output (capped, as in the engine run).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpecCleanerResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    /// The resolved image digest, when podman answered.
    pub image_digest: Option<String>,
    /// True for a dry run: nothing was touched, `plan` holds the
    /// invocation.
    pub dry_run: bool,
    pub plan: Option<Vec<String>>,
}

/// The root marker: the identity of a build root, written on first
/// creation and updated after every build. The eviction policy
/// orders roots by `last_build_unix` and protects the most recent
/// root per key and any root whose last successful build used a
/// digest an image eviction would remove.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct RootMarker {
    pub project: String,
    pub package: String,
    pub repo: String,
    pub arch: String,
    pub flavor: Option<String>,
    pub image_digest: Option<String>,
    pub engine_version: Option<String>,
    pub created_unix: u64,
    pub last_build_unix: u64,
    /// The digest of the last build that succeeded in this root, if
    /// any. An image digest is superseded only once a newer digest
    /// has a successful build behind it.
    pub last_success_digest: Option<String>,
}

/// The coordinates a source-derived wipe expects the root's marker
/// to carry. A named-root wipe has none: any marker that parses
/// proves the root.
struct ExpectedMarker<'a> {
    project: &'a str,
    package: &'a str,
    repo: &'a str,
    arch: &'a str,
    flavor: &'a Option<String>,
}

/// The wipe plan: what the wipe deletes (live) or would delete
/// (dry-run). Entries are counted recursively, excluding the lock
/// file that stays; bytes sum the regular files among them.
struct WipePlan {
    entries: usize,
    bytes: u64,
}

/// The image sidecar: when each image was last pulled, so the 7-day
/// refresh does not need to parse podman's date formats.
#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
struct ImageSidecar {
    #[serde(default)]
    refreshed: HashMap<String, u64>,
}

/// A held build-root lock: the open file keeps the flock alive until
/// this guard drops. The lock file lives inside the root, so wiping
/// the root removes it with the root.
struct BuildLock {
    _file: File,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The local-build vertical: one podman build for quick debug.
///
/// The client holds the OBS client (buildinfo, buildconfig, dep
/// downloads), the cache dir, and the engine/podman binaries (both
/// overridable, so tests can point at stubs). The engine and podman
/// paths default to the bare names, resolved via `PATH`.
pub struct LocalBuild<'a> {
    obs: &'a ObsClient<'a>,
    cache_dir: PathBuf,
    cache_cap_bytes: u64,
    engine: PathBuf,
    podman: PathBuf,
    debug: Debug,
}

impl<'a> LocalBuild<'a> {
    /// A vertical for `obs` with the cache at `cache_dir` and the
    /// default 20 GiB cap. The cache dir is resolved to an absolute
    /// path here: the engine runs with its cwd in the source dir,
    /// so a relative cache dir would resolve inside the source tree
    /// (and a relative root would point at the wrong place).
    pub fn new(obs: &'a ObsClient<'a>, cache_dir: PathBuf) -> Self {
        LocalBuild {
            obs,
            cache_dir: std::path::absolute(&cache_dir).unwrap_or(cache_dir),
            cache_cap_bytes: DEFAULT_CACHE_CAP_GIB * 1024 * 1024 * 1024,
            engine: PathBuf::from("build"),
            podman: PathBuf::from("podman"),
            debug: Debug::disabled(),
        }
    }

    /// A vertical resolved from the config file: `build.cache_dir`
    /// and `build.cache_cap_gib` when set, the XDG default and the
    /// 20 GiB cap otherwise.
    pub fn from_config(obs: &'a ObsClient<'a>, config: &Config) -> Result<Self, GeectlError> {
        let cache_dir = match &config.build_cache_dir {
            Some(dir) => PathBuf::from(dir),
            None => default_cache_dir()?,
        };
        let mut vertical = LocalBuild::new(obs, cache_dir);
        if let Some(cap_gib) = config.build_cache_cap_gib {
            // Saturating: a huge cap clamps to u64::MAX instead of
            // wrapping to a tiny cap and mass-evicting.
            vertical.cache_cap_bytes = cap_gib
                .saturating_mul(1024)
                .saturating_mul(1024)
                .saturating_mul(1024);
        }
        Ok(vertical)
    }

    /// The engine binary (`build` by default). Tests point it at a
    /// stub script.
    pub fn with_engine(mut self, engine: PathBuf) -> Self {
        self.engine = engine;
        self
    }

    /// The podman binary (`podman` by default). Tests point it at a
    /// stub script.
    pub fn with_podman(mut self, podman: PathBuf) -> Self {
        self.podman = podman;
        self
    }

    /// The cache cap in bytes (default 20 GiB).
    pub fn with_cache_cap_bytes(mut self, cap_bytes: u64) -> Self {
        self.cache_cap_bytes = cap_bytes;
        self
    }

    /// Report decision lines to `debug` instead of dropping them.
    pub fn with_debug(mut self, debug: Debug) -> Self {
        self.debug = debug;
        self
    }

    /// The cache directory this vertical uses.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// The cache cap in bytes.
    pub fn cache_cap_bytes(&self) -> u64 {
        self.cache_cap_bytes
    }
}

/// The default cache directory: `$XDG_CACHE_HOME/geectl/`, or
/// `~/.cache/geectl/` when `XDG_CACHE_HOME` is unset. No home is a
/// configuration error, never a silent fallback. On Windows, `HOME`
/// counts only when it is a Windows-absolute path — Git Bash sets a
/// POSIX-style `HOME` — and `USERPROFILE` is the fallback.
pub fn default_cache_dir() -> Result<PathBuf, GeectlError> {
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        let xdg = xdg.trim();
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg).join("geectl"));
        }
    }
    let home_var = std::env::var("HOME").ok();
    let userprofile_var = std::env::var("USERPROFILE").ok();
    // On Windows the home lookup also tries USERPROFILE; name it
    // so the error lists every variable that was actually consulted.
    let no_home = if cfg!(windows) {
        "no HOME, no USERPROFILE and no XDG_CACHE_HOME: cannot resolve the local-build cache directory"
    } else {
        "no HOME and no XDG_CACHE_HOME: cannot resolve the local-build cache directory"
    };
    let home = crate::config::home_dir(home_var.as_deref(), userprofile_var.as_deref())
        .map(|s| s.to_owned())
        .ok_or_else(|| GeectlError::new(ErrorKind::ConfigError, no_home))?;
    if home.trim().is_empty() {
        return Err(GeectlError::new(ErrorKind::ConfigError, no_home));
    }
    Ok(PathBuf::from(home).join(".cache").join("geectl"))
}

/// The custom-root `--clean` gate: a user directory is wiped only
/// when it is empty or carries the geectl marker. The lock file is
/// geectl's own and never counts: under the root lock it is normally
/// present (this build just created it) — an external unlink in
/// between is possible but harmless, the file is ours either way —
/// and a stale one at validation time is equally ours. The wipe
/// skips it too, so the gate and the wipe agree on what "empty"
/// means. A read failure counts as nonempty (fail closed).
fn check_clean_custom_root(root: &Path) -> Result<(), GeectlError> {
    if root.is_dir() {
        let nonempty = fs::read_dir(root)
            .map(|mut entries| {
                entries.any(|entry| match entry {
                    Ok(entry) => entry.file_name().as_os_str() != ROOT_LOCK,
                    Err(_) => true,
                })
            })
            .unwrap_or(true);
        if nonempty && !root.join(ROOT_MARKER).exists() {
            return Err(GeectlError::new(
                ErrorKind::NotAGeectlBuildRoot,
                format!(
                    "--clean refuses a custom root without the geectl marker: {} is not a geectl build root; drop --clean or build there first",
                    root.display()
                ),
            ));
        }
    }
    Ok(())
}

/// The request-level checks a build needs before anything runs:
/// the project and repository names, the cache-dir components, the
/// host architecture, and a non-zero timeout. Pure input checks —
/// the spec and flavor checks need the source directory, so they
/// stay in `build`. The CLI calls this before the pre-steps, so a
/// bad coordinate fails before any service runs or script executes.
pub fn validate_build_request(req: &BuildRequest) -> Result<(), GeectlError> {
    if !valid_obs_name(&req.project) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid OBS project name: {:?}", req.project),
        ));
    }
    if !valid_obs_name(&req.repo) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid OBS repository name: {:?}", req.repo),
        ));
    }
    if let Some(alt) = &req.alternative_project {
        if !valid_obs_name(alt) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {alt:?}"),
            ));
        }
        // The alternative project is a cache-dir component too:
        // `fetch_project()` feeds the pkg-cache path in
        // `ensure_cached` (and the API path in
        // `resolve_dep_filename`), so a `..` here would escape
        // the cache dir the same way a `..` project would.
        validate_path_component("alternative project", alt)?;
    }
    // The path components under the cache dir must be single
    // components: `valid_obs_name` allows `.`, so `..` would
    // pass it and escape the cache dir (a `build --clean` would
    // then wipe an arbitrary directory). The package component
    // is checked in `build` once it is known from the spec.
    validate_cache_components(&req.project, &req.repo, &req.arch, &req.flavor)?;
    if req.arch != std::env::consts::ARCH {
        return Err(GeectlError::new(
            ErrorKind::UnsupportedArch,
            format!(
                "local build for {} on a {} host; other architectures go to the server (OBS-ONLY)",
                req.arch,
                std::env::consts::ARCH
            ),
        ));
    }
    // `--clean` wipes the root `--noinit` needs to build in.
    if req.clean && req.noinit {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "--clean and --noinit conflict: a wiped root has nothing to build in without initialization",
        ));
    }
    // `--noinit` installs nothing, so `--prefer-packages` would be
    // a silent no-op: refuse the combination instead.
    if req.noinit && req.prefer_packages.is_some() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "--noinit and --prefer-packages conflict: no deps are installed without initialization",
        ));
    }
    // A custom root is a user directory, not a cache dir: `--clean`
    // wipes it, so it must be a geectl root (marker) or empty — the
    // same rule `build-wipe` enforces. A path naming a file is not
    // a directory at all. Both fail here, before any fetch or build
    // work, not after the engine ran.
    if let Some(root) = &req.root {
        if root.exists() && !root.is_dir() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "build root {} exists and is not a directory",
                    root.display()
                ),
            ));
        }
        if req.clean {
            check_clean_custom_root(root)?;
        }
    }
    // `--keep-rpms` is written after the build: a path naming a
    // file would fail there, after the whole engine run.
    if let Some(dir) = &req.keep_rpms {
        if dir.exists() && !dir.is_dir() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "keep-rpms directory {} exists and is not a directory",
                    dir.display()
                ),
            ));
        }
    }
    // The prefer dir is read during the build: a missing one fails
    // here, before any service runs or script executes.
    if let Some(dir) = &req.prefer_packages {
        if !dir.is_dir() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "prefer-packages directory {} does not exist or is not a directory",
                    dir.display()
                ),
            ));
        }
    }
    // A zero timeout is an instant timeout, never a build.
    if req.timeout_secs == 0 {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "build timeout must be greater than 0 seconds",
        ));
    }
    Ok(())
}

/// The build flow: validate, fetch buildinfo + buildconfig, check
/// trust, resolve the image, evict, prepare the deps, run the
/// engine, read the verdict. A dry run stops after the plan is
/// built and touches nothing.
impl<'a> LocalBuild<'a> {
    /// Build the package in `req.source_dir` for `req.repo` /
    /// `req.arch`. On a non-zero engine exit this returns
    /// [`ErrorKind::BuildFailed`] (exit code + log path + tail in
    /// the context); on a timeout [`ErrorKind::BuildTimeout`]. A
    /// zero exit with no `finished` line or no RPMs is a record with
    /// verdict `failed`, not an error — the record states what was
    /// read. The record is written to the root on every run.
    pub fn build(&self, req: &BuildRequest) -> Result<BuildRecord, GeectlError> {
        validate_build_request(req)?;
        // One budget for the whole build op: the image pull and the
        // engine share it (see below) — a slow pull leaves less
        // time for the engine, instead of each phase taking a fresh
        // full timeout.
        let deadline = std::time::Instant::now() + Duration::from_secs(req.timeout_secs);
        self.build_with_deadline(req, deadline)
    }

    /// The build against a fixed deadline: the caller owns the
    /// deadline - the `build` leaf passes one deadline for the
    /// service run, the precommit gate, and the engine build, so a
    /// slow pre-step leaves less time for the build instead of
    /// each phase taking a fresh full timeout.
    pub fn build_with_deadline(
        &self,
        req: &BuildRequest,
        deadline: std::time::Instant,
    ) -> Result<BuildRecord, GeectlError> {
        validate_build_request(req)?;
        // The source dir is resolved to an absolute path: the engine
        // runs with its cwd in the source dir, so a relative source
        // dir would make the spec path resolve to
        // `<source_dir>/<source_dir>/foo.spec`.
        let source_dir = std::path::absolute(&req.source_dir).map_err(|e| {
            GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "cannot resolve source directory {}: {e}",
                    req.source_dir.display()
                ),
            )
        })?;
        let spec = find_spec(&source_dir)?;
        let package = spec
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if !valid_obs_name(&package) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "spec file name {:?} is not a valid package name",
                    spec.file_name().unwrap_or_default().to_string_lossy()
                ),
            ));
        }
        // The package is a cache-dir component too (see above).
        validate_path_component("package", &package)?;
        let flavors = multibuild_flavors(&source_dir)?;
        if let Some(flavors) = &flavors {
            match &req.flavor {
                None => {
                    return Err(GeectlError::new(
                        ErrorKind::MultibuildFlavorRequired,
                        format!(
                            "package {package} is multibuild; pass one flavor ({})",
                            flavors.join(", ")
                        ),
                    ));
                }
                Some(flavor) if !flavors.iter().any(|f| f == flavor) => {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "unknown multibuild flavor {flavor:?} for {package}; known flavors: {}",
                            flavors.join(", ")
                        ),
                    ));
                }
                _ => {}
            }
        }
        // A dry run prints the plan and touches nothing, so it does
        // not need the engine installed: a missing engine records
        // as an unknown version instead of failing the plan.
        let engine_version = if req.dry_run {
            self.engine_version().ok().flatten()
        } else {
            self.engine_version()?
        };
        // A custom root replaces the derived cache-dir root: the
        // marker, lock, log, rpmlist, buildconfig, and the sccache
        // store all live under it. It is absolute: the engine runs
        // with its cwd in the source dir, so a relative root would
        // resolve against the source dir.
        let root = match &req.root {
            Some(dir) => std::path::absolute(dir).map_err(|e| {
                GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("cannot resolve build root {}: {e}", dir.display()),
                )
            })?,
            None => self.root_path(&req.project, &package, &req.repo, &req.arch, &req.flavor),
        };
        let log_path = root.join(BUILD_LOG);
        let root_existed = root.join(ROOT_MARKER).exists();
        // The dry run stops here: the plan is data, and nothing
        // below it (no fetch, no root, no eviction) is touched.
        if req.dry_run {
            // The plan records absolute paths, like the root: a
            // relative keep/prefer dir would resolve against the
            // invoker's cwd, not the build.
            let keep_rpms = match &req.keep_rpms {
                Some(dir) => Some(std::path::absolute(dir).map_err(|e| {
                    GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!("cannot resolve keep-rpms directory {}: {e}", dir.display()),
                    )
                })?),
                None => None,
            };
            let prefer_packages = match &req.prefer_packages {
                Some(dir) => Some(std::path::absolute(dir).map_err(|e| {
                    GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "cannot resolve prefer-packages directory {}: {e}",
                            dir.display()
                        ),
                    )
                })?),
                None => None,
            };
            let argv = self.engine_argv(
                &root,
                &spec,
                req,
                &root.join(RPMLIST),
                &root.join(BUILDCONFIG),
                &self.sccache_uri_for_req(req, &package, &root),
            );
            self.debug.emit(&format!(
                "local-build dry-run: {} {} {}",
                req.project, package, req.repo
            ));
            return Ok(BuildRecord {
                verdict: "dry-run".to_string(),
                exit_code: 0,
                log_path,
                rpms: Vec::new(),
                kept_rpms: Vec::new(),
                rpmlint: RpmlintSummary::default(),
                checks: None,
                image_digest: None,
                engine_version,
                root_path: root.clone(),
                root_reused: root_existed,
                cache: CacheStats::default(),
                eviction: EvictionReport {
                    cap_bytes: self.cache_cap_bytes,
                    low_water_bytes: self.low_water(),
                    ..EvictionReport::default()
                },
                preinstall_image: None,
                trusted_foreign_projects: Vec::new(),
                dry_run: true,
                plan: Some(EnginePlan {
                    image: DEFAULT_IMAGE.to_string(),
                    root: root.clone(),
                    repo: req.repo.clone(),
                    arch: req.arch.clone(),
                    flavor: req.flavor.clone(),
                    alternative_project: req.alternative_project.clone(),
                    trust_all_projects: req.trust_all_projects,
                    keep_rpms,
                    prefer_packages,
                    argv,
                }),
            });
        }

        // `--noinit` builds in the existing root without installing
        // anything: it needs the rpmlist and buildconfig a previous
        // build wrote there (the existing tool refuses the same way
        // when no local buildinfo/buildconfig file exists). The check
        // sits after the dry-run return: the plan is data and needs
        // neither file, so a dry run prints it without consulting
        // the root.
        if req.noinit {
            for file in [RPMLIST, BUILDCONFIG] {
                if !root.join(file).exists() {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "--noinit needs an initialized root: {} is missing; run a normal build first",
                            root.join(file).display()
                        ),
                    ));
                }
            }
        }

        // The buildinfo fetch is the prerequisite: the dep set, the
        // trust check, and the eviction's current-dep protection all
        // come from it. A buildinfo error means the build cannot
        // start — fail closed with the server's own text.
        let info = self
            .obs
            .buildinfo(req.fetch_project(), &package, &req.repo, &req.arch)?;
        if let Some(error) = &info.error {
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("build cannot start: {error}"),
            ));
        }
        // Dep fields are server data: validate them before the trust
        // check, so a project name carrying control characters or an
        // ANSI escape never reaches the terminal through the
        // `UntrustedProject` error or the debug line.
        for dep in &info.bdeps {
            validate_dep(dep)?;
        }
        let trusted_foreign_projects =
            self.check_trust(req.fetch_project(), &info, req.trust_all_projects)?;
        let buildconfig = self.obs.buildconfig(req.fetch_project(), &req.repo)?;

        // The image is resolved before eviction: the current digest
        // is protected, and the record names the digest actually used.
        let image = self.resolve_image(req.refresh_image, req.timeout_secs, deadline)?;
        self.debug.emit(&format!(
            "local-build image: {} digest {}",
            DEFAULT_IMAGE,
            image.digest.as_deref().unwrap_or("<unknown>")
        ));

        // The root is created before eviction so the current build's
        // root is never its own eviction candidate. The lock is taken
        // before any wipe: a `--clean` build must never wipe a root
        // another build holds.
        fs::create_dir_all(&root).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot create build root {}: {e}", root.display()),
            )
        })?;
        // The lock is held for the whole build: a second invocation
        // for the same root refuses immediately with BuildInProgress.
        let _lock = self.acquire_root_lock(&root, &package)?;
        if req.clean {
            // Re-check under the lock: the validation-time gate
            // raced the wipe — a marker removed, or files added to
            // an empty root, in between would be wiped unchecked.
            // Cache-dir roots are geectl-managed and need no gate.
            if req.root.is_some() {
                check_clean_custom_root(&root)?;
            }
            self.wipe_root_contents(&root)?;
        }
        self.ensure_marker(&root, req, &package, &image, &engine_version)?;
        let root_reused = root_existed && !req.clean;

        // Eviction runs at build start, after the buildinfo fetch
        // (the current dep set is known) and before any download.
        // The dep set is protected twice: by file name (deps that
        // carry a `binary` attribute) and by dep name (deps without
        // one are resolved from the binary list only later, so their
        // file names are not known yet — a name prefix protects them
        // until then).
        let current_dep_files: Vec<String> = info
            .bdeps
            .iter()
            .filter_map(|dep| dep.binary.clone())
            .collect();
        let current_dep_names: Vec<String> =
            info.bdeps.iter().map(|dep| dep.name.clone()).collect();
        let eviction = self.evict(
            &current_dep_files,
            &current_dep_names,
            image.digest.as_deref(),
            Some(&self.sccache_dir_for_req(req, &package, &root)),
        )?;

        // The deps go into the pkg cache, hdrmd5-verified on every
        // use, then the rpmlist names them for the engine.
        // `--noinit` skips that: the engine installs nothing, so
        // the root's existing rpmlist and buildconfig (checked
        // above) are reused as-is and nothing is downloaded.
        let cache = if req.noinit {
            CacheStats::default()
        } else {
            let (cache, rpmlist) = self.prepare_deps(req, &info)?;
            let rpmlist_path = root.join(RPMLIST);
            write_atomic(&rpmlist_path, rpmlist.as_bytes()).map_err(|e| {
                GeectlError::new(
                    ErrorKind::InternalError,
                    format!("cannot write rpmlist {}: {e}", rpmlist_path.display()),
                )
            })?;
            let buildconfig_path = root.join(BUILDCONFIG);
            write_atomic(&buildconfig_path, buildconfig.as_bytes()).map_err(|e| {
                GeectlError::new(
                    ErrorKind::InternalError,
                    format!(
                        "cannot write buildconfig {}: {e}",
                        buildconfig_path.display()
                    ),
                )
            })?;
            cache
        };
        let rpmlist_path = root.join(RPMLIST);
        let buildconfig_path = root.join(BUILDCONFIG);

        // The sccache store is per root: repeat builds of the same
        // root share compiled objects; different roots never do —
        // two concurrent builds of the same package in different
        // roots must not share a tar.
        let sccache_dir = self.sccache_dir_for_req(req, &package, &root);
        fs::create_dir_all(&sccache_dir).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot create sccache store {}: {e}", sccache_dir.display()),
            )
        })?;
        let sccache_uri = self.sccache_uri_for_req(req, &package, &root);
        let argv = self.engine_argv(
            &root,
            &spec,
            req,
            &rpmlist_path,
            &buildconfig_path,
            &sccache_uri,
        );
        self.debug
            .emit(&format!("local-build engine: {}", argv.join(" ")));
        let (outcome, _, _) = run_engine(
            &argv,
            &source_dir,
            deadline.saturating_duration_since(std::time::Instant::now()),
        )?;
        let record = self.finish_build(
            req,
            &package,
            &root,
            &log_path,
            &image,
            &engine_version,
            root_reused,
            cache,
            eviction,
            info.preinstall_image
                .as_ref()
                .and_then(|i| i.filename.clone()),
            trusted_foreign_projects,
            outcome,
        )?;
        Ok(record)
    }

    /// The low-water mark in bytes (80% of the cap). Divide first:
    /// `cap * 4` can overflow for a huge configured cap.
    fn low_water(&self) -> u64 {
        self.cache_cap_bytes / LOW_WATER_DEN * LOW_WATER_NUM
    }

    /// A path segment for a project, package, repository, arch, or
    /// flavor name: `:` is legal in an OBS name but not in a Windows
    /// path, so it is percent-encoded (`:` becomes `%3A`, `%`
    /// becomes `%25`). The encoding is injective — distinct names
    /// always map to distinct directories — and reversible (see
    /// `decode_path_segment`); a name without `%` or `:` is
    /// unchanged. Only `:` needs encoding: it is the one
    /// Windows-illegal character a validated OBS name
    /// (`[A-Za-z0-9_.+-:]`) can carry, and `%` is escaped so the
    /// encoding stays injective even for unvalidated input. The
    /// marker in the build root keeps the real name; the directory
    /// name never names the project or package back. This differs
    /// from the working copy's lossy `:` to `_` mapping on purpose:
    /// a second checkout there fails on the existing target, while a
    /// build here reuses a root by path alone — the marker is never
    /// compared against the requested build — so a lossy mapping
    /// would silently run one project's tree as another's.
    fn encode_path_segment(name: &str) -> String {
        let mut out = String::with_capacity(name.len());
        for c in name.chars() {
            match c {
                '%' => out.push_str("%25"),
                ':' => out.push_str("%3A"),
                c => out.push(c),
            }
        }
        out
    }

    /// The inverse of `encode_path_segment`: `%25` becomes `%`,
    /// `%3A` becomes `:`. A `%` not followed by `25`, `3A`, or `3a`
    /// passes through unchanged, so decoding a directory name the
    /// encoder never wrote still yields a usable name for a lock
    /// lookup, never a failure.
    fn decode_path_segment(segment: &str) -> String {
        let mut out = String::with_capacity(segment.len());
        let mut rest = segment;
        while let Some(at) = rest.find('%') {
            out.push_str(&rest[..at]);
            let after = &rest[at + 1..];
            if let Some(tail) = after.strip_prefix("25") {
                out.push('%');
                rest = tail;
            } else if let Some(tail) = after
                .strip_prefix("3A")
                .or_else(|| after.strip_prefix("3a"))
            {
                out.push(':');
                rest = tail;
            } else {
                out.push('%');
                rest = after;
            }
        }
        out.push_str(rest);
        out
    }

    /// The root's name inside its project/package dirs:
    /// `<repo>-<arch>` plus `-<flavor>` when the build carries one.
    /// Shared by the build-root path and the sccache store path, so
    /// a root and its store always name the same build.
    fn root_name(repo: &str, arch: &str, flavor: &Option<String>) -> String {
        let mut name = format!(
            "{}-{}",
            Self::encode_path_segment(repo),
            Self::encode_path_segment(arch)
        );
        if let Some(flavor) = flavor {
            name.push('-');
            name.push_str(&Self::encode_path_segment(flavor));
        }
        name
    }

    /// The build root for a (project, package, repo, arch, flavor).
    fn root_path(
        &self,
        project: &str,
        package: &str,
        repo: &str,
        arch: &str,
        flavor: &Option<String>,
    ) -> PathBuf {
        self.cache_dir
            .join("build-roots")
            .join(Self::encode_path_segment(project))
            .join(Self::encode_path_segment(package))
            .join(Self::root_name(repo, arch, flavor))
    }

    /// The sccache store directory for one build root: one store per
    /// root, never shared across roots. Two concurrent builds of the
    /// same package in different roots (different repos, arches or
    /// flavors) hold different root locks, so a per-package tar
    /// would be written by both at once and corrupt; a per-root tar
    /// is guarded by its own root's flock, exactly like the root
    /// itself. Repeat builds of the same root share compiled
    /// objects; different roots never do.
    fn sccache_dir(
        &self,
        project: &str,
        package: &str,
        repo: &str,
        arch: &str,
        flavor: &Option<String>,
    ) -> PathBuf {
        self.cache_dir
            .join("sccache")
            .join(Self::encode_path_segment(project))
            .join(Self::encode_path_segment(package))
            .join(Self::root_name(repo, arch, flavor))
    }

    /// The sccache store directory for one build request: under a
    /// custom `--root` it lives inside the root, so two builds of
    /// the same package in different roots never share a tar (the
    /// tar is guarded by its root's lock, and different roots hold
    /// different locks). Otherwise the derived cache-dir store.
    fn sccache_dir_for_req(&self, req: &BuildRequest, package: &str, root: &Path) -> PathBuf {
        match &req.root {
            Some(_) => root.join("sccache"),
            None => self.sccache_dir(&req.project, package, &req.repo, &req.arch, &req.flavor),
        }
    }

    /// The sccache URI for one build request, matching
    /// `sccache_dir_for_req`.
    fn sccache_uri_for_req(&self, req: &BuildRequest, package: &str, root: &Path) -> PathBuf {
        self.sccache_dir_for_req(req, package, root)
            .join("sccache.tar")
    }

    /// The (project, package) of a sccache store path, for the
    /// locked-root check: the store layout is
    /// `sccache/<project>/<package>/<repo>-<arch>[-<flavor>]/`, so
    /// the last component is the root name, not the package. None
    /// for a path of any other shape — an unrecognised store is
    /// never attributed to the wrong package, so a locked-root
    /// check on it cannot match the wrong (project, package) and
    /// the caller protects it instead. Both the plan-time retain
    /// and the apply-time re-probe use this one helper, so the two
    /// parses cannot drift apart again. The pair is the encoded
    /// directory form (see `encode_path_segment`): a store for
    /// `home:foo` reads back as `home%3Afoo`, matching the encoded
    /// keys `locked_root_keys` returns.
    fn store_project_package(&self, path: &Path) -> Option<(String, String)> {
        let rel = path.strip_prefix(self.cache_dir.join("sccache")).ok()?;
        let mut parts = rel.components();
        let project = parts.next()?.as_os_str().to_string_lossy().to_string();
        let package = parts.next()?.as_os_str().to_string_lossy().to_string();
        let _root = parts.next()?;
        if parts.next().is_some() {
            return None;
        }
        Some((project, package))
    }

    /// The build root for a sccache store path: the store layout is
    /// `sccache/<project>/<package>/<root-name>/`, and the root of
    /// the same build is `build-roots/<project>/<package>/<root-name>/`.
    /// The components are the encoded directory forms (see
    /// `encode_path_segment`); they re-join as-is, so no decode is
    /// needed — the root of the same build sits at the same encoded
    /// path. None for a path of any other shape — the caller never deletes
    /// a store it cannot attribute to a root.
    fn store_root_path(&self, path: &Path) -> Option<PathBuf> {
        let rel = path.strip_prefix(self.cache_dir.join("sccache")).ok()?;
        let mut parts = rel.components();
        let project = parts.next()?.as_os_str();
        let package = parts.next()?.as_os_str();
        let root_name = parts.next()?.as_os_str();
        if parts.next().is_some() {
            return None;
        }
        Some(
            self.cache_dir
                .join("build-roots")
                .join(project)
                .join(package)
                .join(root_name),
        )
    }

    /// Delete a build root while its flock is held: the contents go
    /// first (except the lock file), then the lock file, then the
    /// root directory itself. The lock file goes last on purpose:
    /// unlinking it first would let a concurrent build create a
    /// fresh lock file (new inode) and run inside a root being
    /// deleted. With the lock file last, a build that slips in
    /// between the unlink and the rmdir makes the rmdir fail, so
    /// the delete is reported as failed and the root survives with
    /// the build in it, instead of being deleted under the build.
    fn remove_root_dir(&self, root: &Path) -> std::io::Result<()> {
        self.wipe_root_contents(root)
            .map_err(|e| std::io::Error::other(format!("{e}")))?;
        // The lock file may already be gone (a root that never had
        // one, or one whose lock was never created): its absence is
        // not a delete failure.
        match fs::remove_file(root.join(ROOT_LOCK)) {
            Ok(()) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        fs::remove_dir(root)
    }

    /// The engine argv for a build, in the shape the existing tool
    /// uses: root, rpmlist, dist, arch, the podman VM options, the
    /// build flags, the sccache URI, and the spec last.
    // Six arguments: the engine invocation's inputs — root, spec,
    // request, rpmlist, buildconfig, sccache URI. Grouping them
    // would hide the argv shape.
    #[allow(clippy::too_many_arguments)]
    fn engine_argv(
        &self,
        root: &Path,
        spec: &Path,
        req: &BuildRequest,
        rpmlist: &Path,
        buildconfig: &Path,
        sccache_uri: &Path,
    ) -> Vec<String> {
        let mut argv = vec![
            self.engine.to_string_lossy().to_string(),
            format!("--root={}", root.join(".mount").display()),
            format!("--rpmlist={}", rpmlist.display()),
            format!("--dist={}", buildconfig.display()),
            format!("--arch={}", req.arch),
            "--vm-type=podman".to_string(),
            format!("--vm-disk={}", root.join("img").display()),
            format!("--vm-swap={}", root.join("swap").display()),
            format!("--logfile={}", root.join(BUILD_LOG).display()),
            "--statistics".to_string(),
            "--norootforbuild".to_string(),
        ];
        if req.clean {
            argv.push("--clean".to_string());
        }
        if req.noinit {
            argv.push("--noinit".to_string());
        }
        // The engine passes `--changelog` by default (unless
        // `--no-changelog`): without it the locally built RPMs lack
        // `%changelog` and differ from server builds.
        argv.push("--changelog".to_string());
        if let Some(flavor) = &req.flavor {
            argv.push(format!("--buildflavor={flavor}"));
        }
        if let Some(release) = &req.release {
            argv.push("--release".to_string());
            argv.push(release.clone());
        }
        argv.push(format!("--sccache-uri={}", sccache_uri.display()));
        argv.push(spec.to_string_lossy().to_string());
        argv
    }

    /// The trust check: the `<bdep project="…">` set must contain
    /// only non-`home:` projects and the caller's own `home:`, which
    /// comes from the Basic credentials' username. With a Bearer
    /// token (or no credentials) the caller's own `home:` is
    /// unknown — the OBS API has no whoami endpoint to resolve it
    /// from — so every `home:` project is treated as foreign and
    /// the refusal says so. The
    /// build project itself is excluded from the set: a dep
    /// resolving from the project being built is the build's own
    /// output, not a foreign origin — the existing tool's
    /// `check_trusted_projects` excludes it (`if not i == prj`),
    /// exact match only, never a subproject. A foreign `home:`
    /// project refuses with `UntrustedProject`
    /// naming it, unless `--trust-all-projects` is set: the flag
    /// overrides the refusal for the dep set the buildinfo
    /// enumerates, and the accepted foreign projects are recorded
    /// in the build record and reported on the debug channel.
    /// Returns the foreign `home:` projects: empty when the dep set
    /// holds none, the accepted set when the flag overrode the
    /// refusal. A dep without
    /// a `project` carries no origin to enumerate: it is a virtual
    /// provide or a not-installed flag, not a package the build
    /// would install, so it joins no trust set — the existing tool's
    /// `check_trusted_projects` only considers deps that carry a
    /// project, and a `noinstall` dep must not refuse a build.
    fn check_trust(
        &self,
        build_project: &str,
        info: &crate::obs::BuildInfo,
        trust_all_projects: bool,
    ) -> Result<Vec<String>, GeectlError> {
        let username = match self.obs.credentials() {
            Credentials::Basic { username, .. } => Some(username.as_str()),
            _ => None,
        };
        let own_home = username.map(|u| format!("home:{u}"));
        let mut foreign: Vec<String> = Vec::new();
        for dep in &info.bdeps {
            let Some(project) = &dep.project else {
                continue;
            };
            // The build project itself, exactly: a dep resolving
            // from the project being built is trusted, whatever
            // that project is — including a foreign `home:`.
            // Exact match only, never a subproject: a foreign
            // `home:foo:branches:…` dep is still judged on its own.
            if project == build_project {
                continue;
            }
            // The caller's own home includes its subtree
            // (`home:{user}:branches:…`, where OBS branch work
            // lives): an exact match or a `home:{user}:` prefix.
            let is_own_home = own_home
                .as_ref()
                .is_some_and(|own| project == own || project.starts_with(&format!("{own}:")));
            if project.starts_with("home:") && !is_own_home && !foreign.contains(project) {
                foreign.push(project.clone());
            }
        }
        if foreign.is_empty() {
            return Ok(Vec::new());
        }
        if trust_all_projects {
            self.debug.emit(&format!(
                "local-build trust: accepting foreign home: project(s) via --trust-all-projects: {}",
                foreign.join(", ")
            ));
            return Ok(foreign);
        }
        // With non-Basic credentials the caller's own home: is
        // unknown: a Bearer token carries no username, and the OBS
        // API has no whoami endpoint to resolve one from, so the
        // projects cannot be told apart from a foreign home: — say
        // so, instead of telling the user to pass the broad flag
        // for their own home.
        if own_home.is_none() {
            return Err(GeectlError::new(
                ErrorKind::UntrustedProject,
                format!(
                    "build pulls dependencies from home: project(s): {}; the caller's own home: project cannot be determined from the configured credentials (only Basic username/password credentials carry a username), so they are treated as foreign; pass --trust-all-projects to trust them, or authenticate with a username and password so the caller's own home: is recognized",
                    foreign.join(", ")
                ),
            ));
        }
        Err(GeectlError::new(
            ErrorKind::UntrustedProject,
            format!(
                "build pulls dependencies from foreign home: project(s): {}; only non-home: projects and the caller's own home: are trusted; pass --trust-all-projects to trust them",
                foreign.join(", ")
            ),
        ))
    }

    /// The engine version, when the engine reports one: the engine's
    /// own `--version` answer (first line), else the version of the
    /// package owning the engine binary. The build script answers no
    /// `--version`, so the package version is the version it reports.
    /// An engine that reports no version either way reads as unknown
    /// rather than failing the build — the record carries it when
    /// known.
    fn engine_version(&self) -> Result<Option<String>, GeectlError> {
        let engine = which(&self.engine).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::NoBuildEngine,
                format!(
                    "build engine {:?} not found on PATH; install the openSUSE build package (zypper install build) or put the build script on PATH",
                    self.engine.to_string_lossy()
                ),
            )
        })?;
        let mut cmd = Command::new(&engine);
        cmd.arg("--version");
        let output = run_with_timeout(&mut cmd, Duration::from_secs(PROBE_TIMEOUT_SECS));
        if let Ok(output) = output {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                let first = text.lines().next().map(|l| l.trim().to_string());
                if let Some(first) = first.filter(|l| !l.is_empty()) {
                    return Ok(Some(first));
                }
            }
        }
        if let Some(version) = engine_package_version(&engine) {
            return Ok(Some(version));
        }
        self.debug
            .emit("local-build: engine reported no version; recording unknown");
        Ok(None)
    }

    /// Create the root marker on first build, or refresh the
    /// last-build time on reuse. `--clean` wipes the root contents
    /// first (the marker is rewritten), so a cleaned root reads as
    /// new. A marker that exists but does not parse fails closed:
    /// silently resetting it would drop `created_unix` and
    /// `last_success_digest` — the root's most-recent and image
    /// protections — and a root whose marker is corrupt is a cache
    /// problem to fix (build-wipe), not to paper over.
    fn ensure_marker(
        &self,
        root: &Path,
        req: &BuildRequest,
        package: &str,
        image: &ImageInfo,
        engine_version: &Option<String>,
    ) -> Result<RootMarker, GeectlError> {
        let path = root.join(ROOT_MARKER);
        let now = now_unix();
        let mut marker = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<RootMarker>(&text).map_err(|e| {
                GeectlError::new(
                    ErrorKind::CacheCorrupt,
                    format!("build root marker {} is corrupt ({}); wipe the root with build-wipe and rebuild", path.display(), e),
                )
            })?,
            Err(_) => RootMarker {
                project: req.project.clone(),
                package: package.to_string(),
                repo: req.repo.clone(),
                arch: req.arch.clone(),
                flavor: req.flavor.clone(),
                image_digest: image.digest.clone(),
                engine_version: engine_version.clone(),
                created_unix: now,
                last_build_unix: now,
                last_success_digest: None,
            },
        };
        marker.image_digest = image.digest.clone();
        marker.engine_version = engine_version.clone();
        marker.last_build_unix = now;
        let text = serde_json::to_string(&marker).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot serialize build root marker: {e}"),
            )
        })?;
        write_atomic(&path, text.as_bytes()).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot write build root marker {}: {e}", path.display()),
            )
        })?;
        Ok(marker)
    }

    /// Canonicalize `path`, falling back to the nearest existing
    /// ancestor when the path itself does not exist yet: the
    /// ancestor canonicalizes, the remainder re-appends. A
    /// build-roots base under a fresh cache may not exist, and a
    /// bare `absolute()` would keep a `/tmp` symlink unresolved on
    /// macOS, breaking the containment comparison.
    fn canonicalize_or_absolute(path: &Path) -> PathBuf {
        if let Ok(canonical) = fs::canonicalize(path) {
            return canonical;
        }
        match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => {
                let base = Self::canonicalize_or_absolute(parent);
                match path.file_name() {
                    Some(name) => base.join(name),
                    None => base,
                }
            }
            _ => std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()),
        }
    }

    /// Read and parse the root marker. Missing marker: `None`.
    /// Present but unparsable: fail closed — a corrupt marker
    /// proves nothing about the directory, and wiping on the
    /// strength of a planted file would trust it.
    fn read_root_marker(root: &Path) -> Result<Option<RootMarker>, GeectlError> {
        let path = root.join(ROOT_MARKER);
        // The marker must be a regular file: a symlink to a valid
        // marker elsewhere would authorize the wipe on borrowed
        // identity.
        match fs::symlink_metadata(&path) {
            Ok(meta) if !meta.file_type().is_file() => {
                return Err(GeectlError::new(
                    ErrorKind::NotAGeectlBuildRoot,
                    format!(
                        "build root marker {} is not a regular file; refusing to wipe an unverified directory",
                        path.display()
                    ),
                ));
            }
            _ => {}
        }
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(GeectlError::new(
                    ErrorKind::InternalError,
                    format!("cannot read build root marker {}: {e}", path.display()),
                ));
            }
        };
        match serde_json::from_str::<RootMarker>(&text) {
            Ok(marker) => Ok(Some(marker)),
            Err(e) => Err(GeectlError::new(
                ErrorKind::NotAGeectlBuildRoot,
                format!(
                    "build root marker {} is corrupt ({}); refusing to wipe an unverified directory",
                    path.display(),
                    e
                ),
            )),
        }
    }

    /// Whether the marker carries the coordinates a
    /// source-derived wipe expects.
    fn marker_matches(marker: &RootMarker, expected: &ExpectedMarker) -> bool {
        marker.project == expected.project
            && marker.package == expected.package
            && marker.repo == expected.repo
            && marker.arch == expected.arch
            && marker.flavor == *expected.flavor
    }

    /// Read the root's marker and, for a source-derived root,
    /// check it against the expected coordinates. Missing marker:
    /// `false`. Corrupt marker or coordinate mismatch: fail closed
    /// with `NotAGeectlBuildRoot` — the directory is unverified.
    fn check_marker(
        root: &Path,
        expected: Option<&ExpectedMarker<'_>>,
    ) -> Result<bool, GeectlError> {
        let marker = match Self::read_root_marker(root)? {
            Some(marker) => marker,
            None => return Ok(false),
        };
        if let Some(expected) = expected {
            if !Self::marker_matches(&marker, expected) {
                return Err(GeectlError::new(
                    ErrorKind::NotAGeectlBuildRoot,
                    format!(
                        "build root marker at {} does not match {} / {} / {} / {} / {}; refusing to wipe an unverified directory",
                        root.join(ROOT_MARKER).display(),
                        expected.project,
                        expected.package,
                        expected.repo,
                        expected.arch,
                        expected.flavor.as_deref().unwrap_or("<none>")
                    ),
                ));
            }
        }
        Ok(true)
    }

    /// The root's device + inode, for the post-lock identity
    /// re-check. Unix only: elsewhere there is no stable pair to
    /// compare, so the marker re-check alone guards the lock race.
    #[cfg(unix)]
    fn root_identity(root: &Path) -> Result<(u64, u64), GeectlError> {
        use std::os::unix::fs::MetadataExt;
        let meta = fs::symlink_metadata(root).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot stat build root {}: {e}", root.display()),
            )
        })?;
        Ok((meta.dev(), meta.ino()))
    }

    /// Refuse a wipe across a device boundary: the root must not
    /// be a mount point, and no entry inside it may live on
    /// another device. A mount inside the root would be deleted
    /// with the root's contents, and a root that is itself a mount
    /// point is not a build root's directory. Counts entries and
    /// sums regular-file bytes for the plan on the way. Symlinks
    /// are never followed: their own device is the parent's, and
    /// they count as one entry. The device comparison is Unix
    /// only: without `dev` there is no boundary to check, so the
    /// check is a no-op elsewhere and the count still runs. A bind
    /// mount of the same filesystem shares the device, so it is
    /// invisible here: only cross-device mounts are caught.
    fn inspect_wipe_plan(root: &Path) -> Result<WipePlan, GeectlError> {
        let mut plan = WipePlan {
            entries: 0,
            bytes: 0,
        };
        #[cfg(unix)]
        let root_dev = {
            use std::os::unix::fs::MetadataExt;
            let meta = fs::symlink_metadata(root).map_err(|e| {
                GeectlError::new(
                    ErrorKind::InternalError,
                    format!("cannot stat build root {}: {e}", root.display()),
                )
            })?;
            if let Some(parent) = root.parent() {
                let parent_meta = fs::symlink_metadata(parent).map_err(|e| {
                    GeectlError::new(
                        ErrorKind::InternalError,
                        format!("cannot stat {}: {e}", parent.display()),
                    )
                })?;
                if parent_meta.dev() != meta.dev() {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "build root {} is a mount point; refusing to wipe",
                            root.display()
                        ),
                    ));
                }
            }
            meta.dev()
        };
        let mut stack: Vec<(PathBuf, bool)> = vec![(root.to_path_buf(), true)];
        while let Some((dir, top)) = stack.pop() {
            let entries = fs::read_dir(&dir).map_err(|e| {
                GeectlError::new(
                    ErrorKind::InternalError,
                    format!("cannot list build root {}: {e}", dir.display()),
                )
            })?;
            for entry in entries {
                let entry = entry.map_err(|e| {
                    GeectlError::new(
                        ErrorKind::InternalError,
                        format!("cannot list build root {}: {e}", dir.display()),
                    )
                })?;
                let path = entry.path();
                // The lock file stays, so it is not part of the plan.
                if top && path.file_name().is_some_and(|n| n == ROOT_LOCK) {
                    continue;
                }
                let meta = fs::symlink_metadata(&path).map_err(|e| {
                    GeectlError::new(
                        ErrorKind::InternalError,
                        format!("cannot stat {}: {e}", path.display()),
                    )
                })?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if meta.dev() != root_dev {
                        return Err(GeectlError::new(
                            ErrorKind::InvalidInput,
                            format!(
                                "entry {} is on a different device (mount boundary); refusing to wipe",
                                path.display()
                            ),
                        ));
                    }
                }
                plan.entries += 1;
                if meta.is_file() {
                    plan.bytes += meta.len();
                } else if meta.file_type().is_dir() {
                    stack.push((path, false));
                }
            }
        }
        Ok(plan)
    }

    /// Remove everything inside the root (for `--clean`), keeping
    /// the root directory itself. The marker is rewritten by the
    /// caller. The lock file is NOT removed: the caller holds its
    /// flock for the whole build, and deleting the file would leave
    /// the lock on an unlinked inode — a second build could then
    /// create a fresh lock file and run in the same root.
    ///
    /// Deletion never follows symlinks: each entry's type is
    /// checked with `symlink_metadata` at removal time, and a
    /// symlink is unlinked, never traversed — an entry swapped for
    /// a symlink after the listing is unlinked, not followed. The
    /// lock only excludes cooperating geectl builds and wipes; a
    /// hostile local writer with access to the root could still
    /// swap entries, but the swap can only delete entries inside
    /// the root, never traverse out of it.
    fn wipe_root_contents(&self, root: &Path) -> Result<(), GeectlError> {
        let entries = fs::read_dir(root).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot list build root {}: {e}", root.display()),
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|e| {
                GeectlError::new(
                    ErrorKind::InternalError,
                    format!("cannot list build root {}: {e}", root.display()),
                )
            })?;
            let path = entry.path();
            if path.file_name().is_some_and(|n| n == ROOT_LOCK) {
                continue;
            }
            let file_type = fs::symlink_metadata(&path)
                .map_err(|e| {
                    GeectlError::new(
                        ErrorKind::InternalError,
                        format!("cannot stat {}: {e}", path.display()),
                    )
                })?
                .file_type();
            if file_type.is_dir() {
                fs::remove_dir_all(&path).map_err(|e| {
                    GeectlError::new(
                        ErrorKind::InternalError,
                        format!("cannot wipe {}: {e}", path.display()),
                    )
                })?;
            } else {
                fs::remove_file(&path).map_err(|e| {
                    GeectlError::new(
                        ErrorKind::InternalError,
                        format!("cannot wipe {}: {e}", path.display()),
                    )
                })?;
            }
        }
        Ok(())
    }

    /// Take the root's flock (non-blocking). A held lock refuses
    /// with `BuildInProgress`, naming the holder from the lock file.
    /// Lock a build root for a build, refusing while another build
    /// holds it. A lock that frees within a few milliseconds is
    /// retried, not refused: a `Command::spawn` in another thread
    /// forks the process, and the forked child holds a reference
    /// to the lock file's open file description until exec closes
    /// it — so a flock dropped just before that fork still reads
    /// as held for a few milliseconds (proven with a standalone
    /// probe: `Command` reproduces it, raw `posix_spawnp` does
    /// not). A real concurrent build holds the lock for the whole
    /// build, far past this budget, so it still refuses promptly
    /// with the holder named.
    fn acquire_root_lock(&self, root: &Path, package: &str) -> Result<BuildLock, GeectlError> {
        let path = root.join(ROOT_LOCK);
        // The lock file is opened with create(true) and then
        // written: a planted symlink here would truncate and
        // overwrite the link's target. Refuse a lock path that is
        // not a regular file. (Checked before opening; a swap
        // between the check and the open is the same residual race
        // the wipe documents for the lock.)
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if !meta.file_type().is_file() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "build root lock {} is not a regular file; refusing to lock",
                        path.display()
                    ),
                ));
            }
        }
        // No truncate: the holder line is rewritten after the lock
        // is held (seek + write + set_len). Truncating on open
        // would let a refused caller wipe the holder's line before
        // reading it.
        let open_lock = || {
            File::options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
        };
        // A just-released lock can read as held for a few
        // milliseconds (see above): retry briefly, then refuse.
        let deadline = std::time::Instant::now() + Duration::from_millis(100);
        loop {
            let file = match open_lock() {
                Ok(file) => file,
                // The root was deleted between the caller's create and
                // this open (an eviction rmdir landing in between):
                // recreate the root and open again, instead of failing
                // a build that should start in a fresh root.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    fs::create_dir_all(root).map_err(|e| {
                        GeectlError::new(
                            ErrorKind::InternalError,
                            format!("cannot create build root {}: {e}", root.display()),
                        )
                    })?;
                    open_lock().map_err(|e| {
                        GeectlError::new(
                            ErrorKind::InternalError,
                            format!("cannot open build root lock {}: {e}", path.display()),
                        )
                    })?
                }
                Err(e) => {
                    return Err(GeectlError::new(
                        ErrorKind::InternalError,
                        format!("cannot open build root lock {}: {e}", path.display()),
                    ));
                }
            };
            // Lock first, then write the holder line: writing before
            // locking would let a refused caller overwrite the holder's
            // line with its own pid and then name itself as the holder.
            match file.try_lock_exclusive() {
                Ok(()) => {
                    let holder = format!(
                        "pid={} started={} package={}\n",
                        std::process::id(),
                        now_unix(),
                        package
                    );
                    {
                        use std::io::Seek;
                        use std::io::SeekFrom;
                        let mut file = &file;
                        let _ = file.seek(SeekFrom::Start(0));
                        let _ = file.write_all(holder.as_bytes());
                        let _ = file.set_len(holder.len() as u64);
                    }
                    return Ok(BuildLock { _file: file });
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    drop(file);
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    let holder = fs::read_to_string(&path)
                        .unwrap_or_default()
                        .lines()
                        .next()
                        .unwrap_or("unknown holder")
                        .to_string();
                    return Err(GeectlError::new(
                        ErrorKind::BuildInProgress,
                        format!(
                            "build root {} is locked ({holder}); a build of this package is already running",
                            root.display()
                        ),
                    ));
                }
                Err(e) => {
                    return Err(GeectlError::new(
                        ErrorKind::InternalError,
                        format!("cannot lock build root {}: {e}", path.display()),
                    ));
                }
            }
        }
    }
}

/// The resolved image: the digest the build records, and whether
/// this build pulled or refreshed it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ImageInfo {
    digest: Option<String>,
}

/// Image resolution and dep preparation.
impl<'a> LocalBuild<'a> {
    /// Resolve the build image: the local digest when present, a
    /// pull when absent or older than 7 days, a forced pull with
    /// `--refresh-image`. The digest is recorded in the build record,
    /// so a build is reproducible from the record. Podman missing or
    /// a pull failing is `ImageUnavailable` — the engine cannot run
    /// without it. A pull refused because the build budget is
    /// already exhausted is `BuildTimeout` instead — podman and the
    /// image are fine, the build ran out of time before the pull.
    /// A pull that starts and then times out is `BuildTimeout` too
    /// (see `podman_pull`): the same kind as an engine timeout.
    fn resolve_image(
        &self,
        refresh_image: bool,
        timeout_secs: u64,
        deadline: std::time::Instant,
    ) -> Result<ImageInfo, GeectlError> {
        let podman = which(&self.podman).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ImageUnavailable,
                format!(
                    "podman {:?} not found on PATH; install podman to run local builds",
                    self.podman.to_string_lossy()
                ),
            )
        })?;
        let digest = self.podman_digest(&podman)?;
        let sidecar = self.read_image_sidecar();
        let refreshed = sidecar.refreshed.get(DEFAULT_IMAGE).copied().unwrap_or(0);
        let stale =
            refreshed == 0 || now_unix().saturating_sub(refreshed) > IMAGE_REFRESH_DAYS * 86400;
        if digest.is_none() || refresh_image || stale {
            // The pull gets the build's remaining budget — but a
            // budget already exhausted fails as a timeout, not as
            // an image failure: handing a 0s timeout to the pull
            // reported its "command timed out after 0s" as
            // `ImageUnavailable`, although podman and the image
            // were fine. The refusal fires only when no budget
            // remains at all: a pull due with time left runs with
            // the remaining budget, however small — a floor above
            // zero would refuse the pull on every build whose
            // budget is that small (the pre-pull steps always
            // consume some of it).
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(GeectlError::new(
                    ErrorKind::BuildTimeout,
                    format!(
                        "image pull refused: build budget of {}s already exhausted ({}s remaining); podman and the image are fine, the build ran out of time before the pull",
                        timeout_secs,
                        remaining.as_secs()
                    ),
                ));
            }
            self.debug.emit(&format!(
                "local-build image pull: {} (absent={}, refresh={}, stale={})",
                DEFAULT_IMAGE,
                digest.is_none(),
                refresh_image,
                stale
            ));
            self.podman_pull(&podman, remaining)?;
            let mut sidecar = self.read_image_sidecar();
            sidecar
                .refreshed
                .insert(DEFAULT_IMAGE.to_string(), now_unix());
            self.write_image_sidecar(&sidecar)?;
            let digest = self.podman_digest(&podman)?.ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ImageUnavailable,
                    format!(
                        "image {DEFAULT_IMAGE} pulled but has no digest; refusing to build from an unidentified image"
                    ),
                )
            })?;
            return Ok(ImageInfo {
                digest: Some(digest),
            });
        }
        Ok(ImageInfo { digest })
    }

    /// The digest of the local image, or None when it is not pulled.
    fn podman_digest(&self, podman: &Path) -> Result<Option<String>, GeectlError> {
        let mut cmd = Command::new(podman);
        cmd.args(["image", "inspect", "--format", "{{.Digest}}", DEFAULT_IMAGE]);
        let output =
            run_with_timeout(&mut cmd, Duration::from_secs(PROBE_TIMEOUT_SECS)).map_err(|e| {
                GeectlError::new(
                    ErrorKind::ImageUnavailable,
                    format!("cannot run podman image inspect: {e}"),
                )
            })?;
        if !output.status.success() {
            return Ok(None);
        }
        let digest = String::from_utf8_lossy(&output.stdout).trim().to_string();
        // Podman prints `<none>` when the image has no digest: that
        // is "absent", not a digest to record and protect — callers
        // would otherwise treat it as a real digest and never pull.
        if digest.is_empty() || digest == "<none>" {
            return Ok(None);
        }
        Ok(Some(digest))
    }

    /// Pull the image. The command's stderr is bounded in the error:
    /// a pull failure must say why, not dump a log. The timeout is
    /// the build's remaining budget: a pull is part of the build,
    /// and the pull and the engine share one budget — a pull that
    /// outlasts the whole build budget is hung, not slow. A pull
    /// that times out is `BuildTimeout`, not `ImageUnavailable`:
    /// the engine and the image are fine, the build ran out of
    /// time during the pull.
    fn podman_pull(&self, podman: &Path, timeout: Duration) -> Result<(), GeectlError> {
        let mut cmd = Command::new(podman);
        cmd.args(["pull", DEFAULT_IMAGE]);
        let output = run_with_timeout(&mut cmd, timeout).map_err(|e| {
            if e.kind() == std::io::ErrorKind::TimedOut {
                // A pull that times out is a timeout, not an image
                // failure: podman and the image are fine, the build
                // ran out of time during the pull — the same kind
                // as an engine timeout, naming the budget it had.
                GeectlError::new(
                    ErrorKind::BuildTimeout,
                    format!(
                        "podman pull {DEFAULT_IMAGE} timed out after {timeout:?} of remaining build budget; podman and the image are fine, the build ran out of time during the pull"
                    ),
                )
            } else {
                GeectlError::new(
                    ErrorKind::ImageUnavailable,
                    format!("cannot run podman pull: {e}"),
                )
            }
        })?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr: String = stderr.chars().take(500).collect();
        Err(GeectlError::new(
            ErrorKind::ImageUnavailable,
            format!("podman pull {DEFAULT_IMAGE} failed: {}", stderr.trim()),
        ))
    }

    /// The image size in bytes for a digest, best-effort (None when
    /// podman will not answer — the cap then counts filesystem bytes
    /// only, and the record says so by omission).
    fn podman_image_size(&self, digest: &str) -> Option<u64> {
        let podman = which(&self.podman)?;
        let mut cmd = Command::new(podman);
        cmd.args(["image", "inspect", "--format", "{{.Size}}", digest]);
        let output = run_with_timeout(&mut cmd, Duration::from_secs(PROBE_TIMEOUT_SECS)).ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u64>()
            .ok()
    }

    fn read_image_sidecar(&self) -> ImageSidecar {
        let path = self.cache_dir.join(IMAGE_SIDECAR);
        fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<ImageSidecar>(&text).ok())
            .unwrap_or_default()
    }

    fn write_image_sidecar(&self, sidecar: &ImageSidecar) -> Result<(), GeectlError> {
        let path = self.cache_dir.join(IMAGE_SIDECAR);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                GeectlError::new(
                    ErrorKind::InternalError,
                    format!("cannot create cache dir {}: {e}", parent.display()),
                )
            })?;
        }
        let text = serde_json::to_string(sidecar).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot serialize image sidecar: {e}"),
            )
        })?;
        write_atomic(&path, text.as_bytes()).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot write image sidecar {}: {e}", path.display()),
            )
        })
    }

    /// The repository a dep resolves from: its own, or the build
    /// repo when the server did not say. The existing tool keys its
    /// cache dir and its download URL by the dep's own
    /// project/repository/repoarch, not the build's — a dep from a
    /// linked or module repo must not be fetched from (or cached
    /// under) the build repo: that would 404, or worse, install a
    /// wrong-repo file into the build repo's cache dir.
    fn dep_repo<'b>(dep: &'b BuildDep, req: &'b BuildRequest) -> &'b str {
        dep.repository.as_deref().unwrap_or(req.repo.as_str())
    }

    /// The repository architecture a dep resolves for: its own, or
    /// the build arch when the server did not say.
    fn dep_arch<'b>(dep: &'b BuildDep, req: &'b BuildRequest) -> &'b str {
        dep.repoarch.as_deref().unwrap_or(req.arch.as_str())
    }

    /// Prepare the resolved deps: each one lands in the pkg cache
    /// (hdrmd5-verified on every use) and the rpmlist names it for
    /// the engine. Returns the cache statistics and the rpmlist text.
    /// A dep the server resolved without a binary name is resolved
    /// from the binary list of its package; a dep with no installable
    /// file refuses (fail closed, naming the dep). Every dep is
    /// downloaded and listed, noinstall included: for a spec build
    /// the existing tool lists every dep as `name path` (the
    /// noinstall filter applies only to kiwi/docker/podman image
    /// build types), and the rpmlist references the file, so it must
    /// exist. The `noinstall:` and `installonly:` lines are emitted
    /// only when non-empty, and a sysroot dep's line carries the
    /// `sysroot: ` prefix.
    fn prepare_deps(
        &self,
        req: &BuildRequest,
        info: &crate::obs::BuildInfo,
    ) -> Result<(CacheStats, String), GeectlError> {
        let mut stats = CacheStats::default();
        let mut lines: Vec<String> = Vec::new();
        let mut preinstall: Vec<String> = Vec::new();
        let mut vminstall: Vec<String> = Vec::new();
        let mut runscripts: Vec<String> = Vec::new();
        let mut noinstall: Vec<String> = Vec::new();
        let mut installonly: Vec<String> = Vec::new();
        for dep in &info.bdeps {
            validate_dep(dep)?;
            if dep.preinstall {
                preinstall.push(dep.name.clone());
            }
            if dep.vminstall {
                vminstall.push(dep.name.clone());
            }
            if dep.runscripts {
                runscripts.push(dep.name.clone());
            }
            if dep.noinstall {
                noinstall.push(dep.name.clone());
            }
            if dep.installonly {
                installonly.push(dep.name.clone());
            }
            let filename = match &dep.binary {
                Some(binary) => binary.clone(),
                None => self.resolve_dep_filename(req, dep)?,
            };
            let cached = self.ensure_cached(req, dep, &filename, &mut stats)?;
            if dep.sysroot {
                lines.push(format!("sysroot: {} {}", dep.name, cached.display()));
            } else {
                lines.push(format!("{} {}", dep.name, cached.display()));
            }
        }
        // The rpmlist shape: one `<name> <file>` line per dep, then
        // the install lists. The preinstall image lines are absent
        // on purpose: a rootless podman build drops the image,
        // exactly as the existing tool does.
        lines.push(format!("preinstall: {}", preinstall.join(" ")));
        lines.push(format!("vminstall: {}", vminstall.join(" ")));
        lines.push(format!("runscripts: {}", runscripts.join(" ")));
        if !noinstall.is_empty() {
            lines.push(format!("noinstall: {}", noinstall.join(" ")));
        }
        if !installonly.is_empty() {
            lines.push(format!("installonly: {}", installonly.join(" ")));
        }
        Ok((stats, lines.join("\n") + "\n"))
    }

    /// Resolve a dep's binary file name. The scheduler's buildinfo
    /// carries no `binary` attribute, so the file name is computed
    /// locally (`name-version-release.arch.rpm`, the epoch is never
    /// part of a file name) when the server sent version, release
    /// and arch — the same shape the existing tool uses. Without
    /// all three, the binary list of the dep's *source* package
    /// resolves it: exactly one RPM whose parsed package name
    /// equals the dep name must match — a `starts_with("{name}-")`
    /// filter picks the wrong file for any package that builds
    /// subpackages (`helper-devel-…` would match a `helper` dep).
    /// The list is of the source package (`dep.package`), never
    /// the binary name: a subpackage binary (`libz1`) lives under
    /// its source package (`zlib`), and listing by the binary name
    /// 404s (the binary name is not a package). Without a source package the
    /// whole repository is listed (`_repository`) — the server did
    /// not say where the binary comes from, so no package segment
    /// can be assumed.
    fn resolve_dep_filename(
        &self,
        req: &BuildRequest,
        dep: &BuildDep,
    ) -> Result<String, GeectlError> {
        if let (Some(version), Some(release), Some(arch)) = (
            dep.version.as_deref().filter(|v| !v.is_empty()),
            dep.release.as_deref().filter(|v| !v.is_empty()),
            dep.arch.as_deref().filter(|v| !v.is_empty()),
        ) {
            return Ok(format!("{}-{version}-{release}.{arch}.rpm", dep.name));
        }
        let project = dep.project.as_deref().unwrap_or(req.fetch_project());
        let package = dep.package.as_deref().unwrap_or("_repository");
        let files = self.obs.list_binaries(
            project,
            Some(package),
            Self::dep_repo(dep, req),
            Self::dep_arch(dep, req),
        )?;
        let candidates: Vec<&str> = files
            .iter()
            .map(|f| f.name.as_str())
            .filter(|name| rpm_package_name(name).as_deref() == Some(dep.name.as_str()))
            .collect();
        match candidates.len() {
            1 => Ok(candidates[0].to_string()),
            0 => Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "cannot resolve a binary file for dependency {} (project {project}, package {package}); the binary list carries no main RPM",
                    dep.name
                ),
            )),
            _ => Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "cannot resolve a binary file for dependency {} (project {project}, package {package}); ambiguous: {}",
                    dep.name,
                    candidates.join(", ")
                ),
            )),
        }
    }

    /// Ensure one dep RPM is in the pkg cache, verified against the
    /// buildinfo hdrmd5 when the server sent one. A cached file that
    /// fails verification is re-downloaded once, then refused with
    /// `CacheCorrupt`. Returns the cache path. Every use touches the
    /// file (mtime = last use) for the eviction order.
    fn ensure_cached(
        &self,
        req: &BuildRequest,
        dep: &BuildDep,
        filename: &str,
        stats: &mut CacheStats,
    ) -> Result<PathBuf, GeectlError> {
        // The resolved file name is the one dep-derived path
        // component `validate_dep` cannot see: it comes from the
        // server's binary list, not from a dep field, so a `/` or
        // `..` in it would escape the cache dir at the `dir.join`
        // below (and the API path when it is downloaded). Fail
        // closed here, before any path is built from it: this is
        // the sink every file name reaches, whichever of the two
        // sources (`dep.binary` or the list resolution) produced it.
        validate_path_component("dependency binary", filename)?;
        // The dep's own project/repository/repoarch, not the
        // build's: the cache dir is one per project+repo+arch, so a
        // dep from another project or repo can never be installed
        // into this repo's root (or vice versa) — and the same repo
        // name plus the same file name from two projects never
        // collide in one dir, the way they would under a
        // repo+arch-only key. The lock is the same
        // project-repo-arch lock eviction takes while deleting that
        // dir's pkg files.
        let project = dep.project.as_deref().unwrap_or(req.fetch_project());
        let repo = Self::dep_repo(dep, req);
        let arch = Self::dep_arch(dep, req);
        let dir = self.pkg_dir(project, repo, arch);
        fs::create_dir_all(&dir).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot create package cache {}: {e}", dir.display()),
            )
        })?;
        // The per-file download/write path takes the
        // project-repo-arch lock (blocking, briefly): eviction
        // takes the same lock while deleting that dir's pkg files,
        // so a delete can never race a download's atomic rename.
        // The lock is held for the whole ensure — check, download,
        // write, verify — so a concurrent eviction cannot delete
        // the file between the existence check and the verify.
        let _pkg_lock = self.acquire_pkg_lock(project, repo, arch)?;
        let path = dir.join(filename);
        // Preferred packages win over the cache and the download:
        // an explicit local file overrides whatever the cache
        // holds — a rebuilt RPM in the prefer dir is used on the
        // next run instead of the stale cached copy — and is copied
        // into the cache instead of downloaded, hdrmd5-verified
        // when the buildinfo carries one. A preferred file that
        // fails verification refuses the build rather than falling
        // back to the cache or the download: installing a different
        // package than the dep names is silent wrongness, not a
        // cache miss.
        if let Some(prefer) = &req.prefer_packages {
            let candidate = prefer.join(filename);
            if candidate.is_file() {
                self.verify_hdrmd5(&candidate, Self::effective_hdrmd5(dep, None))
                    .map_err(|e| {
                        GeectlError::new(
                            ErrorKind::CacheCorrupt,
                            format!(
                                "preferred package {} failed its hdrmd5 check ({}); remove it from the prefer dir or fix the buildinfo digest",
                                candidate.display(),
                                e.context
                            ),
                        )
                    })?;
                let tmp = dir.join(format!(".{filename}.preferred"));
                fs::copy(&candidate, &tmp).map_err(|e| {
                    GeectlError::new(
                        ErrorKind::InternalError,
                        format!("cannot copy preferred package {}: {e}", candidate.display()),
                    )
                })?;
                // Verify the copy, not just the source: the file
                // could change between the check and the copy.
                if let Err(e) = self.verify_hdrmd5(&tmp, Self::effective_hdrmd5(dep, None)) {
                    let _ = fs::remove_file(&tmp);
                    return Err(GeectlError::new(
                        ErrorKind::CacheCorrupt,
                        format!(
                            "preferred package {} failed its hdrmd5 check after copying ({}); copy discarded",
                            candidate.display(),
                            e.context
                        ),
                    ));
                }
                fs::rename(&tmp, &path).map_err(|e| {
                    let _ = fs::remove_file(&tmp);
                    GeectlError::new(
                        ErrorKind::InternalError,
                        format!("cannot place preferred package {}: {e}", path.display()),
                    )
                })?;
                stats.pkg_preferred += 1;
                return Ok(path);
            }
        }
        if path.exists() {
            match self.verify_hdrmd5(&path, Self::effective_hdrmd5(dep, None)) {
                Ok(()) => {
                    stats.pkg_hits += 1;
                    touch(&path);
                    return Ok(path);
                }
                Err(e) => {
                    // A corrupt cached file is re-downloaded once;
                    // the download path below verifies again.
                    self.debug.emit(&format!(
                        "local-build cache: {} failed verification ({}); re-downloading",
                        filename, e
                    ));
                    let _ = fs::remove_file(&path);
                }
            }
        }
        stats.pkg_misses += 1;
        match self.download_dep(req, dep, project, filename, &dir)? {
            DownloadedDep::InMemory(bytes, warmed_hdrmd5) => {
                write_atomic(&path, &bytes).map_err(|e| {
                    GeectlError::new(
                        ErrorKind::InternalError,
                        format!("cannot write package cache {}: {e}", path.display()),
                    )
                })?;
                self.verify_hdrmd5(&path, Self::effective_hdrmd5(dep, warmed_hdrmd5.as_deref()))
                    .map_err(|e| {
                        let _ = fs::remove_file(&path);
                        GeectlError::new(
                            ErrorKind::CacheCorrupt,
                            format!(
                                "cached package {filename} failed its hdrmd5 check after re-download ({}); deleted from the cache",
                                e.context
                            ),
                        )
                    })?;
            }
            DownloadedDep::Streamed(tmp, warmed_hdrmd5) => {
                // The streamed file is verified before it lands in
                // the cache: a corrupt stream never becomes a cached
                // package, and the temp file is deleted on failure.
                if let Err(e) =
                    self.verify_hdrmd5(&tmp, Self::effective_hdrmd5(dep, warmed_hdrmd5.as_deref()))
                {
                    let _ = fs::remove_file(&tmp);
                    return Err(GeectlError::new(
                        ErrorKind::CacheCorrupt,
                        format!(
                            "cached package {filename} failed its hdrmd5 check after re-download ({}); streamed download discarded",
                            e.context
                        ),
                    ));
                }
                fs::rename(&tmp, &path).map_err(|e| {
                    let _ = fs::remove_file(&tmp);
                    GeectlError::new(
                        ErrorKind::InternalError,
                        format!("cannot write package cache {}: {e}", path.display()),
                    )
                })?;
            }
        }
        touch(&path);
        Ok(path)
    }

    /// Download one dep RPM. A dep over the transport's 10 MiB body
    /// cap streams to a temp file in the cache directory instead of
    /// failing: the bytes never sit in memory at once, and the
    /// streamed file is verified before it lands in the cache (see
    /// `ensure_cached`).
    /// The download URL keys on the dep's *source* package, never
    /// the binary name: a subpackage binary lives under its source
    /// package, and the binary name as the package segment 404s.
    /// A 404 retries after warming the binary: on a
    /// download-on-demand repository the daemon fetches repository
    /// metadata only, so a binary nobody has asked for is not in
    /// the repository yet and a direct file download 404s. Asking
    /// the server for the binary's versions makes it fetch the
    /// binary (the listing blocks until the fetch completes). The
    /// retry downloads under the name the listing reports
    /// (`<binary>.<suffix>`): the fetched file is stored under
    /// that name. A package in a repodata module is stored under
    /// the full EVRA filename instead, so the EVRA filename is the
    /// second retry candidate. A warm failure keeps the original
    /// 404; a retry failure is returned as-is. The retry runs even
    /// when the warm reports the binary unavailable: the listing only
    /// triggers the fetch, the download is the check. The
    /// returned digest is the one the warm reported for the
    /// binary, when the download went through the warm; it is
    /// the digest to verify the downloaded file against.
    fn download_dep(
        &self,
        req: &BuildRequest,
        dep: &BuildDep,
        project: &str,
        filename: &str,
        dir: &Path,
    ) -> Result<DownloadedDep, GeectlError> {
        let package = dep.package.as_deref().unwrap_or("_repository");
        let repo = Self::dep_repo(dep, req);
        let arch = Self::dep_arch(dep, req);
        match self
            .obs
            .download_binary(project, package, repo, arch, filename)
        {
            Ok(bytes) => Ok(DownloadedDep::InMemory(bytes, None)),
            Err(e) if is_transport_cap_error(&e) => {
                self.stream_dep(req, dep, project, filename, None, dir)
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                match self.obs.binary_versions(project, repo, arch, &dep.name) {
                    Ok(versions) => {
                        // The listing reports the name the server
                        // stored the fetched binary under
                        // (`<binary>.<suffix>`): try that first. A
                        // package in a repodata module is stored
                        // under the full EVRA filename instead, so
                        // the EVRA filename is the second candidate.
                        // No usable name in the listing leaves only
                        // the EVRA filename.
                        let warmed = versions.iter().find(|version| version.error.is_none());
                        let mut candidates = Vec::new();
                        if let Some(reported) = warmed.map(|version| version.name.clone()) {
                            candidates.push(reported);
                        }
                        if !candidates.iter().any(|name| name == filename) {
                            candidates.push(filename.to_string());
                        }
                        let mut last_err = None;
                        for name in &candidates {
                            match self.obs.download_binary(project, package, repo, arch, name) {
                                Ok(bytes) => {
                                    return Ok(DownloadedDep::InMemory(
                                        bytes,
                                        warmed.and_then(|version| version.hdrmd5.clone()),
                                    ));
                                }
                                Err(err) if is_transport_cap_error(&err) => {
                                    return self.stream_dep(
                                        req,
                                        dep,
                                        project,
                                        name,
                                        warmed.and_then(|version| version.hdrmd5.clone()),
                                        dir,
                                    );
                                }
                                Err(err) => last_err = Some(err),
                            }
                        }
                        match last_err {
                            Some(err) => Err(err),
                            None => Err(e),
                        }
                    }
                    Err(warm) => {
                        self.debug.emit(&format!(
                            "local-build: warming dependency {} failed ({}); keeping the original 404",
                            dep.name, warm
                        ));
                        Err(e)
                    }
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Stream one dep RPM to a temp file in the cache directory: the
    /// over-cap fallback for `download_dep`. The temp name is
    /// allocated by `fresh_temp_path` and created exclusively by the
    /// download, so a name that already exists is retried with the
    /// next name instead of deleting a file this call did not create.
    /// A 409 from the server reports the same kind without creating
    /// the file, so it is returned, not retried. Any other failure
    /// deletes the temp file; on success the caller verifies it and
    /// renames it into the cache.
    fn stream_dep(
        &self,
        req: &BuildRequest,
        dep: &BuildDep,
        project: &str,
        filename: &str,
        warmed_hdrmd5: Option<String>,
        dir: &Path,
    ) -> Result<DownloadedDep, GeectlError> {
        let package = dep.package.as_deref().unwrap_or("_repository");
        let repo = Self::dep_repo(dep, req);
        let arch = Self::dep_arch(dep, req);
        for _ in 0..1000 {
            let tmp = fresh_temp_path(dir).map_err(|e| {
                let kind = if e.kind() == std::io::ErrorKind::AlreadyExists {
                    ErrorKind::AlreadyExists
                } else {
                    ErrorKind::InternalError
                };
                GeectlError::new(
                    kind,
                    format!("cannot allocate a temp file in {}: {e}", dir.display()),
                )
            })?;
            match self
                .obs
                .download_binary_to_file(project, package, repo, arch, filename, &tmp)
            {
                Ok(_) => return Ok(DownloadedDep::Streamed(tmp, warmed_hdrmd5)),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                    // `AlreadyExists` has two sources: a temp name
                    // that already exists (retry with the next name),
                    // and a 409 from the server, which never created
                    // the file (return it: retrying a 409 a thousand
                    // times ends in a misleading exhaustion error).
                    if tmp.exists() {
                        continue;
                    }
                    return Err(e);
                }
                Err(e) => {
                    let _ = fs::remove_file(&tmp);
                    return Err(e);
                }
            }
        }
        Err(GeectlError::new(
            ErrorKind::AlreadyExists,
            "no free temp file name for streamed download",
        ))
    }

    /// The hdrmd5 to verify a downloaded dep against. The warmed
    /// value wins when the download went through the warm: it is
    /// computed from the file the server fetched. Otherwise the
    /// buildinfo value. The download-on-demand placeholder is never
    /// a real digest, so a dep carrying only it verifies as absent,
    /// and a warmed placeholder never overrides a real buildinfo
    /// digest.
    fn effective_hdrmd5<'h>(dep: &'h BuildDep, warmed: Option<&'h str>) -> Option<&'h str> {
        let warmed = warmed.filter(|value| !value.eq_ignore_ascii_case(DOD_PLACEHOLDER_HDRMD5));
        match warmed.or(dep.hdrmd5.as_deref()) {
            Some(value) if value.eq_ignore_ascii_case(DOD_PLACEHOLDER_HDRMD5) => None,
            other => other,
        }
    }

    /// Verify a cached RPM against the expected hdrmd5. No expected
    /// value (the server did not send one) verifies as ok — the
    /// existing tool only verifies when the buildinfo carries one.
    fn verify_hdrmd5(&self, path: &Path, expected: Option<&str>) -> Result<(), GeectlError> {
        let Some(expected) = expected else {
            return Ok(());
        };
        let actual = rpm_hdrmd5(path).map_err(|e| {
            GeectlError::new(
                ErrorKind::CacheCorrupt,
                format!("cannot read hdrmd5 of {}: {e}", path.display()),
            )
        })?;
        if actual.eq_ignore_ascii_case(expected) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::CacheCorrupt,
                format!(
                    "hdrmd5 mismatch for {}: expected {expected}, got {actual}",
                    path.display()
                ),
            ))
        }
    }

    /// Read the build log, parse the verdict, and write the build
    /// record. A non-zero engine exit is `BuildFailed` (exit code +
    /// log path + tail); a zero exit with no `finished` line or no
    /// RPMs is a record with verdict `failed`.
    // Twelve arguments: the build record's fields, assembled here
    // from the build's inputs — there is no earlier record to pass.
    #[allow(clippy::too_many_arguments)]
    fn finish_build(
        &self,
        req: &BuildRequest,
        package: &str,
        root: &Path,
        log_path: &Path,
        image: &ImageInfo,
        engine_version: &Option<String>,
        root_reused: bool,
        cache: CacheStats,
        eviction: EvictionReport,
        preinstall_image: Option<String>,
        trusted_foreign_projects: Vec<String>,
        outcome: EngineOutcome,
    ) -> Result<BuildRecord, GeectlError> {
        // Lossy on purpose: a non-UTF-8 log must still parse (a
        // strict read would empty the log and fail a good build on
        // its verdict). Only the tail is read (see `read_log_tail`).
        let log_text = read_log_tail(log_path, false);
        let rpms = collect_rpms(root);
        let rpmlint = parse_rpmlint_summary(&log_text);
        let checks = parse_check_summary(&log_text);
        let sccache_hits = parse_sccache_hits(&log_text);
        let cache = CacheStats {
            sccache_hits,
            ..cache
        };
        let finished = has_finished_line(&log_text, package);
        // Green is exit code 0, the finished line, and RPMs present:
        // a `--keep-rpms` copy only ever carries built RPMs, never
        // stale ones from a previous build in a reused root.
        let green = !outcome.timed_out && outcome.exit_code == 0 && finished && !rpms.is_empty();
        let kept_rpms = if green {
            self.keep_rpms(req, &rpms)?
        } else {
            Vec::new()
        };
        let record = BuildRecord {
            verdict: if green {
                "green".to_string()
            } else {
                "failed".to_string()
            },
            exit_code: outcome.exit_code,
            log_path: log_path.to_path_buf(),
            rpms,
            kept_rpms,
            rpmlint,
            checks,
            image_digest: image.digest.clone(),
            engine_version: engine_version.clone(),
            root_path: root.to_path_buf(),
            root_reused,
            cache,
            eviction,
            preinstall_image,
            trusted_foreign_projects,
            dry_run: false,
            plan: None,
        };
        // The record is written even on failure: `build-log` reads
        // the verdict from it, and the next build reuses the root. A
        // failed write is debug output, not a build failure — the
        // record is an audit trail, and losing it must not fail a
        // build that already ran.
        let record_path = root.join(BUILD_RECORD);
        match serde_json::to_string_pretty(&record) {
            Ok(text) => {
                if let Err(e) = write_atomic(&record_path, text.as_bytes()) {
                    self.debug.emit(&format!(
                        "local-build: cannot write build record {}: {e}",
                        record_path.display()
                    ));
                }
            }
            Err(e) => {
                self.debug
                    .emit(&format!("local-build: cannot serialize build record: {e}"));
            }
        }
        // The marker records the last successful build's digest: an
        // image digest is superseded only once a newer digest has a
        // successful build behind it. The marker is re-read from
        // disk here (it may have been refreshed since the build
        // started) rather than trusting the in-memory copy.
        if record.verdict == "green" {
            if let Ok(text) = fs::read_to_string(root.join(ROOT_MARKER)) {
                if let Ok(mut marker) = serde_json::from_str::<RootMarker>(&text) {
                    marker.last_success_digest = image.digest.clone();
                    marker.last_build_unix = now_unix();
                    if let Ok(text) = serde_json::to_string(&marker) {
                        let _ = write_atomic(&root.join(ROOT_MARKER), text.as_bytes());
                    }
                }
            }
        }
        // Touch the sccache store: eviction orders stores by last
        // use. This runs before the timeout/failure returns below:
        // the store is used on every build, and a package that
        // fails repeatedly must not lose its warm cache to eviction
        // for aging while it is still being built.
        touch(&self.sccache_dir_for_req(req, package, root));
        if outcome.timed_out {
            // On unix the timeout kills the engine's process group;
            // on Windows only the direct child is killed.
            #[cfg(unix)]
            let killed = "the engine's process group was killed";
            #[cfg(not(unix))]
            let killed = "the engine process was killed; its children may still be running";
            return Err(GeectlError::new(
                ErrorKind::BuildTimeout,
                format!(
                    "build of {package} timed out after {}s; partial log at {}; {killed}, but the container it started may still be running",
                    req_timeout(req),
                    log_path.display()
                ),
            ));
        }
        if outcome.exit_code != 0 {
            let tail = last_lines(&log_text, 20).join("\n");
            return Err(GeectlError::new(
                ErrorKind::BuildFailed,
                format!(
                    "build of {package} failed with exit code {}; log at {}; tail:\n{tail}",
                    outcome.exit_code,
                    log_path.display()
                ),
            ));
        }
        Ok(record)
    }

    /// Copy the built RPMs into the `--keep-rpms` directory
    /// (created when missing), returning the copied paths. Only
    /// called on a green build. A copy failure fails the build: the
    /// caller asked for the RPMs kept, and silently dropping them
    /// would report a green build whose outputs are not where they
    /// were asked for.
    fn keep_rpms(&self, req: &BuildRequest, rpms: &[PathBuf]) -> Result<Vec<PathBuf>, GeectlError> {
        let Some(dir) = &req.keep_rpms else {
            return Ok(Vec::new());
        };
        let dir = std::path::absolute(dir).map_err(|e| {
            GeectlError::new(
                ErrorKind::InvalidInput,
                format!("cannot resolve keep-rpms directory {}: {e}", dir.display()),
            )
        })?;
        fs::create_dir_all(&dir).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot create keep-rpms directory {}: {e}", dir.display()),
            )
        })?;
        let mut kept = Vec::new();
        for rpm in rpms {
            let Some(name) = rpm.file_name() else {
                continue;
            };
            let dest = dir.join(name);
            fs::copy(rpm, &dest).map_err(|e| {
                GeectlError::new(
                    ErrorKind::InternalError,
                    format!(
                        "cannot copy built RPM {} to {}: {e}",
                        rpm.display(),
                        dest.display()
                    ),
                )
            })?;
            kept.push(dest);
        }
        Ok(kept)
    }
}

/// Whether `e` is the transport's over-cap rejection: the response
/// body exceeded the 10 MiB read cap (`MAX_BODY_BYTES` in
/// `src/http.rs`, whose message reads "response body exceeds the
/// N-byte limit"). The match is on the message because the
/// transport reports the cap as a plain `ServerError`; it is the
/// full phrase, not the bare "exceeds the" — any other error
/// containing those words must not be misreported as a cap refusal.
fn is_transport_cap_error(e: &GeectlError) -> bool {
    e.context.contains("response body exceeds the")
}

fn req_timeout(req: &BuildRequest) -> u64 {
    req.timeout_secs
}

/// The engine's exit: code, and whether the timeout killed it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EngineOutcome {
    exit_code: i32,
    timed_out: bool,
}

/// Run the engine with a bounded timeout, draining its stdout and
/// stderr (capped) while waiting. Returns the exit, whether the
/// timeout killed it, and the captured output (stdout, stderr).
/// On unix, the engine spawns in its own process group
/// (`process_group(0)`), so on timeout the kill reaches the engine
/// and its children in the host process group — killing the wrapper
/// alone would leave the engine running. The container the engine
/// started is NOT stopped by that kill: it lives in its own pid
/// namespace, and conmon keeps it running after podman dies —
/// stopping it needs the container id, which the engine does not
/// report (see the design doc's "Timeout and abort"). On unix, the
/// child gets SIGTERM, then SIGKILL after a grace period (see
/// [`kill_child`]); on Windows only the direct child is killed.
fn run_engine(
    argv: &[String],
    cwd: &Path,
    timeout: Duration,
) -> Result<(EngineOutcome, Vec<u8>, Vec<u8>), GeectlError> {
    if argv.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InternalError,
            "cannot run the build engine: empty argv",
        ));
    }
    let mut child = spawn_with_retry(|| {
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Its own process group: the timeout kill signals the group
        // (`kill -- -$pgid`), not just the wrapper — the engine and
        // its host-side children die with it. The container does
        // not: it lives in its own pid namespace (see the doc above).
        // `process_group` is safe std, no `unsafe`, so the crate's
        // `unsafe` forbid still holds. Unix-only: Windows has no
        // process groups in std.
        #[cfg(unix)]
        cmd.process_group(0);
        cmd.spawn()
    })
    .map_err(|e| {
        GeectlError::new(
            ErrorKind::NoBuildEngine,
            format!("cannot run the build engine {}: {e}", argv[0]),
        )
    })?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let stdout_drain = spawn_drain(stdout.take());
    let stderr_drain = spawn_drain(stderr.take());
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok((
                    EngineOutcome {
                        exit_code: status.code().unwrap_or(-1),
                        timed_out: false,
                    },
                    take_drain(stdout_drain),
                    take_drain(stderr_drain),
                ));
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    kill_child(&mut child);
                    let _ = child.wait();
                    // The timeout path discards the captured output
                    // (the log file is the record): detach the
                    // drains instead of waiting out the join timeout.
                    drop(stdout_drain);
                    drop(stderr_drain);
                    return Ok((
                        EngineOutcome {
                            exit_code: -1,
                            timed_out: true,
                        },
                        Vec::new(),
                        Vec::new(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                return Err(GeectlError::new(
                    ErrorKind::InternalError,
                    format!("cannot wait for the build engine: {e}"),
                ));
            }
        }
    }
}

/// On unix, SIGTERM the child's process group, then SIGKILL after a
/// grace period. The engine runs in its own process group (see
/// [`run_engine`]), so the signal reaches the engine and its
/// host-side children — killing the wrapper alone would leave the
/// engine running. It does NOT stop the container the engine
/// started (own pid namespace; conmon keeps it running) — see
/// [`run_engine`]. `kill` is the system binary (present on Linux
/// and macOS); when it is missing the SIGKILL below is the whole
/// kill, and only the direct child dies. On Windows there is no
/// `kill` binary and no process group to signal, so the child is
/// killed directly instead (see below).
#[cfg(unix)]
fn kill_child(child: &mut std::process::Child) {
    let pgid = child.id();
    let _ = Command::new("kill")
        .args(["-TERM", "--", &format!("-{pgid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let deadline = std::time::Instant::now() + Duration::from_secs(KILL_GRACE_SECS);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => break,
        }
    }
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{pgid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
}

/// Kill the direct child immediately. Windows has no `kill`
/// binary and no process group to signal, so the grace-period
/// wait would only burn time polling a child nothing terminates.
/// Grandchildren are not stopped.
#[cfg(not(unix))]
fn kill_child(child: &mut std::process::Child) {
    let _ = child.kill();
}

/// Spawn attempts, at most: a momentary fork failure clears in
/// milliseconds, so a short backoff is enough.
const SPAWN_RETRY_ATTEMPTS: u32 = 3;
/// Pause between spawn retries.
const SPAWN_RETRY_DELAY: Duration = Duration::from_millis(10);

/// Whether an I/O error from spawning a child is transient and
/// worth one more attempt: resource exhaustion (EAGAIN/ENOMEM, or
/// a full file-descriptor table, EMFILE/ENFILE) under parallel
/// load, not a missing binary or a denied exec. A spawn refused
/// for descriptors clears as soon as the other in-flight
/// children exit, so the same short backoff applies.
fn is_transient_spawn_error(e: &std::io::Error) -> bool {
    if matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::ResourceBusy
            | std::io::ErrorKind::OutOfMemory
    ) {
        return true;
    }
    // Descriptor exhaustion is named by the raw errno, not the
    // kind: the dedicated kind is still unstable, so a kind match
    // would not compile — EMFILE (per-process table full) and
    // ENFILE (system table full) both retry.
    matches!(
        e.raw_os_error(),
        Some(code) if code == libc::EMFILE || code == libc::ENFILE
    )
}

/// Spawn a child, retrying a transient fork failure instead of
/// surfacing it: a momentary resource failure (EAGAIN, or a full
/// descriptor table) while the test suite runs in parallel must
/// not fail a build whose podman is fine. Permanent failures
/// return on the first attempt, and the original error is kept
/// when the retries run out.
fn spawn_with_retry(
    mut spawn: impl FnMut() -> std::io::Result<std::process::Child>,
) -> std::io::Result<std::process::Child> {
    let mut attempts = 0;
    let mut first_err: Option<std::io::Error> = None;
    loop {
        match spawn() {
            Ok(child) => return Ok(child),
            Err(e) if is_transient_spawn_error(&e) && attempts + 1 < SPAWN_RETRY_ATTEMPTS => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
                attempts += 1;
                std::thread::sleep(SPAWN_RETRY_DELAY);
            }
            Err(e) => return Err(first_err.take().unwrap_or(e)),
        }
    }
}

/// Run a one-shot command with a deadline, capturing stdout/stderr
/// (capped, as in [`run_engine`]). On expiry the child is killed
/// and `TimedOut` is returned — a probe must never hang the caller.
/// The engine run has its own deadline loop ([`run_engine`]); this
/// is for the probes (`--version` and package-owner version,
/// image inspect/listing/size, image remove, image pull).
fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> std::io::Result<std::process::Output> {
    let mut child = spawn_with_retry(|| {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    })?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let stdout_drain = spawn_drain(stdout.take());
    let stderr_drain = spawn_drain(stderr.take());
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(std::process::Output {
                    status,
                    stdout: take_drain(stdout_drain),
                    stderr: take_drain(stderr_drain),
                });
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    drop(stdout_drain);
                    drop(stderr_drain);
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("command timed out after {}s", timeout.as_secs()),
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    }
}

/// A drain in flight: the buffer the drain thread appends to, and
/// a signal that it finished. The buffer is shared so the caller
/// can take what was captured so far when the join times out — the
/// child has already written it; only EOF is missing.
struct Drain {
    buf: Arc<Mutex<Vec<u8>>>,
    done: mpsc::Receiver<()>,
}

/// Start draining a piped stream into a shared buffer (capped, as
/// below): the thread reads until EOF and then signals completion.
/// If a descendant inherited the pipe and outlives the child, EOF
/// never comes — the thread is left detached and the caller takes
/// the partial buffer after `DRAIN_JOIN_TIMEOUT_SECS` instead of
/// hanging on the join.
fn spawn_drain(stream: Option<impl Read + Send + 'static>) -> Drain {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let (done_tx, done_rx) = mpsc::channel();
    let shared = Arc::clone(&buf);
    std::thread::spawn(move || {
        drain_into(stream, &shared);
        let _ = done_tx.send(());
    });
    Drain { buf, done: done_rx }
}

/// Take the drained output, waiting at most the join timeout for
/// the drain to finish. On timeout the partial buffer is returned —
/// the child already wrote it; only EOF is missing. Dropping a
/// `Drain` without taking it detaches the thread: it finishes on
/// its own when the pipe closes.
fn take_drain(drain: Drain) -> Vec<u8> {
    let _ = drain
        .done
        .recv_timeout(Duration::from_secs(DRAIN_JOIN_TIMEOUT_SECS));
    drain
        .buf
        .lock()
        .map(|mut buf| std::mem::take(&mut *buf))
        .unwrap_or_default()
}

/// Drain a piped stream into a shared buffer, capped: past the cap
/// the rest is read and discarded, never unbounded, and never left
/// unread — stopping at the cap would let a verbose child block on
/// a full pipe and be killed as a timeout for talking too much.
fn drain_into(stream: Option<impl Read + Send + 'static>, buf: &Arc<Mutex<Vec<u8>>>) {
    let Some(mut stream) = stream else {
        return;
    };
    let mut chunk = [0u8; 8192];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if let Ok(mut buf) = buf.lock() {
                    let room = OUTPUT_CAP_BYTES.saturating_sub(buf.len());
                    if room > 0 {
                        buf.extend_from_slice(&chunk[..n.min(room)]);
                    }
                }
            }
            Err(_) => break,
        }
    }
}

/// The version of the package owning `engine`, when the host package
/// manager can tell: `rpm -qf` on rpm systems, `dpkg-query` on
/// Debian systems. `None` when neither answers — the engine is not
/// installed from a package, or no package manager is present.
fn engine_package_version(engine: &Path) -> Option<String> {
    let engine = engine.to_str()?;
    let mut cmd = Command::new("rpm");
    cmd.args(["-qf", "--queryformat", "%{VERSION}-%{RELEASE}", engine]);
    if let Ok(output) = run_with_timeout(&mut cmd, Duration::from_secs(PACKAGE_PROBE_TIMEOUT_SECS))
    {
        if output.status.success() {
            let version = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if !version.is_empty() {
                return Some(version);
            }
        }
    }
    let mut cmd = Command::new("dpkg-query");
    cmd.args(["-S", engine]);
    let owner = run_with_timeout(&mut cmd, Duration::from_secs(PACKAGE_PROBE_TIMEOUT_SECS)).ok()?;
    if !owner.status.success() {
        return None;
    }
    // The owner line is `package: <path>` (or `package:arch: <path>`);
    // a diverted path also prints `local diversion ...` lines naming
    // the same path, so select the line whose package field has no
    // whitespace — a package name never does.
    let stdout = String::from_utf8_lossy(&owner.stdout);
    let package = stdout
        .lines()
        .filter(|line| line.ends_with(&format!(": {engine}")))
        .filter_map(|line| line.split(':').next().map(str::trim))
        .find(|package| !package.is_empty() && !package.contains(char::is_whitespace));
    let package = package?;
    let mut cmd = Command::new("dpkg-query");
    cmd.args(["-W", "-f", "${Version}", package]);
    let output =
        run_with_timeout(&mut cmd, Duration::from_secs(PACKAGE_PROBE_TIMEOUT_SECS)).ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

/// Find the engine (or podman) binary: an absolute path as-is, a
/// bare name resolved via `PATH`.
fn which(path: &Path) -> Option<PathBuf> {
    if path.components().count() > 1 {
        return if path.exists() {
            Some(path.to_path_buf())
        } else {
            None
        };
    }
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(path);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// What `download_dep` brought back: bytes in memory, or a file
/// already streamed to a temp file in the cache directory (with the
/// warmed hdrmd5, when the download went through the warm).
#[derive(Debug)]
enum DownloadedDep {
    InMemory(Vec<u8>, Option<String>),
    Streamed(PathBuf, Option<String>),
}

/// A temp file name in `parent` that does not exist yet, in the same
/// `.geectl-tmp-<pid>-<n>` scheme `write_atomic` uses, so a streamed
/// download lands beside its final path (an atomic rename needs the
/// same filesystem). The name is only allocated, not created: the
/// writer creates it exclusively.
fn fresh_temp_path(parent: &Path) -> std::io::Result<PathBuf> {
    for n in 0..1000 {
        let tmp = parent.join(format!(".geectl-tmp-{}-{n}", std::process::id()));
        if !tmp.exists() {
            return Ok(tmp);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no free temp file name",
    ))
}

/// Write a file via temp file + atomic rename, so a half-written
/// file is never visible (a reader either sees the old file or the
/// new one). The temp file is removed on failure: no crust.
fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?;
    fs::create_dir_all(parent)?;
    // The temp name must be unique per call, not just per second:
    // two writes in the same second would collide.
    for n in 0..1000 {
        let tmp = parent.join(format!(".geectl-tmp-{}-{n}", std::process::id()));
        match File::options().write(true).create_new(true).open(&tmp) {
            Ok(mut file) => {
                if let Err(e) = file.write_all(contents).and_then(|()| file.sync_all()) {
                    let _ = fs::remove_file(&tmp);
                    return Err(e);
                }
                drop(file);
                return match fs::rename(&tmp, path) {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        let _ = fs::remove_file(&tmp);
                        Err(e)
                    }
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "cannot allocate a temp file for atomic write",
    ))
}

/// Touch a path (update its mtime to now) for the eviction order.
/// A missing path is not an error: an absent sccache store starts
/// empty. Opening for append does NOT update mtime on its own — the
/// `FileTimes` set is what moves it.
fn touch(path: &Path) {
    if let Ok(file) = File::open(path) {
        let _ = file.set_times(std::fs::FileTimes::new().set_modified(SystemTime::now()));
    }
}

/// One path component under the cache dir: a single name, never a
/// traversal. `valid_obs_name` allows `.`, so `..` (and `/`, `\`,
/// NUL, empty) needs this separate check — without it a flavor of
/// `../../x` (or a `..` project/repo) would escape the cache dir,
/// and a `build --clean` would wipe an arbitrary directory.
/// Control characters are rejected too: an ANSI escape rides on
/// ESC, so a server-supplied name carrying one must fail here,
/// never reach the terminal.
fn validate_path_component(label: &str, value: &str) -> Result<(), GeectlError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
        || value.chars().any(|c| c.is_control())
    {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid {label} {value:?}: must be a single path component"),
        ));
    }
    Ok(())
}

/// Every dep field that becomes a cache path component or an API
/// URL segment: server data, so a `..` project (or a `/` in a file
/// name) must fail closed here, not escape the cache dir or the API
/// path when the dep is downloaded. The build's own components are
/// validated in `build`; the dep's were not. An empty attribute is
/// no attribute (see `resolve_dep_filename`), so empty values are
/// skipped, not rejected.
fn validate_dep(dep: &BuildDep) -> Result<(), GeectlError> {
    validate_path_component("dependency name", &dep.name)?;
    for (label, value) in [
        ("dependency project", &dep.project),
        ("dependency package", &dep.package),
        ("dependency binary", &dep.binary),
        ("dependency version", &dep.version),
        ("dependency release", &dep.release),
        ("dependency architecture", &dep.arch),
        ("dependency repository", &dep.repository),
        ("dependency repoarch", &dep.repoarch),
    ] {
        if let Some(value) = value.as_deref().filter(|v| !v.is_empty()) {
            validate_path_component(label, value)?;
        }
    }
    Ok(())
}

/// All five cache-dir components of a build root: project, repo,
/// arch, and the flavor when present. The package is checked by the
/// caller once it is known (from the spec in `build`, from the
/// argument in `build_shell`/`build_wipe`/`build_log`).
fn validate_cache_components(
    project: &str,
    repo: &str,
    arch: &str,
    flavor: &Option<String>,
) -> Result<(), GeectlError> {
    validate_path_component("project", project)?;
    validate_path_component("repository", repo)?;
    validate_path_component("architecture", arch)?;
    if let Some(flavor) = flavor {
        validate_path_component("flavor", flavor)?;
    }
    Ok(())
}

/// Find the single spec file in a source directory. Zero or several
/// is invalid input, naming what was found.
fn find_spec(source_dir: &Path) -> Result<PathBuf, GeectlError> {
    let entries = fs::read_dir(source_dir).map_err(|e| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            format!("cannot read source directory {}: {e}", source_dir.display()),
        )
    })?;
    let mut specs: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot list source directory {}: {e}", source_dir.display()),
            )
        })?;
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "spec") {
            specs.push(path);
        }
    }
    match specs.len() {
        1 => Ok(specs.pop().unwrap()),
        0 => Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("no .spec file in source directory {}", source_dir.display()),
        )),
        _ => {
            let names: Vec<String> = specs
                .iter()
                .map(|p| {
                    p.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                })
                .collect();
            Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "several .spec files in source directory {}: {}; pass a directory with one package",
                    source_dir.display(),
                    names.join(", ")
                ),
            ))
        }
    }
}

/// The multibuild flavors of a source directory, from `_multibuild`
/// (`<multibuild><package>flavor</package>…</multibuild>`). None when
/// the file is absent (not a multibuild package). A present but
/// unreadable or empty file is invalid input, not an empty flavor
/// list — a flavorless build of a multibuild package dies with a
/// fake architecture error, which is exactly what this refuses to
/// re-learn.
fn multibuild_flavors(source_dir: &Path) -> Result<Option<Vec<String>>, GeectlError> {
    let path = source_dir.join("_multibuild");
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).map_err(|e| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            format!("cannot read _multibuild file {}: {e}", path.display()),
        )
    })?;
    let mut flavors: Vec<String> = Vec::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find("<package") {
        let Some(tag_end) = rest[start..].find('>') else {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("malformed _multibuild file {}", path.display()),
            ));
        };
        let content_start = start + tag_end + 1;
        let Some(end) = rest[content_start..].find("</package>") else {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("malformed _multibuild file {}", path.display()),
            ));
        };
        let flavor = rest[content_start..content_start + end].trim().to_string();
        if !flavor.is_empty() {
            flavors.push(flavor);
        }
        rest = &rest[content_start + end + "</package>".len()..];
    }
    if flavors.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!(
                "_multibuild file {} carries no flavors; remove it or name one",
                path.display()
            ),
        ));
    }
    Ok(Some(flavors))
}

/// The package name of an RPM filename (`name-version-release.arch.rpm`
/// -> `name`): the version and release are split off from the right,
/// and the arch after the last dot. A source RPM has no binary name
/// to resolve, so it never matches. None when the filename is not a
/// shaped RPM name at all.
fn rpm_package_name(filename: &str) -> Option<String> {
    if filename.ends_with(".src.rpm") {
        return None;
    }
    let stem = filename.strip_suffix(".rpm")?;
    let without_arch = stem.rsplit_once('.')?.0;
    let without_release = without_arch.rsplit_once('-')?.0;
    let name = without_release.rsplit_once('-')?.0;
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

/// The RPMs a build produced: `*.rpm` files under the root's
/// `.mount/.build.packages`, recursing into `RPMS/` and `SRPMS/` —
/// the engine puts its output there, not at the top level (the
/// existing tool's `pacdir` is `<build_root>/.build.packages` where
/// the build root already ends in `/.mount`). The top level is never
/// scanned: a stale `.build.packages` there would have its RPMs
/// reported as fresh outputs of every later build. A source RPM
/// counts as a built RPM: it is output the build produced. The list
/// is sorted for a stable record.
fn collect_rpms(root: &Path) -> Vec<PathBuf> {
    let mut rpms: Vec<PathBuf> = Vec::new();
    collect_rpms_under(&root.join(".mount").join(".build.packages"), &mut rpms);
    rpms.sort();
    rpms
}

/// Recurse `dir` for `*.rpm` files, deduplicating by path.
fn collect_rpms_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rpms_under(&path, out);
        } else if path.is_file()
            && path.extension().is_some_and(|e| e == "rpm")
            && !out.contains(&path)
        {
            out.push(path);
        }
    }
}

/// True when the log carries the engine's `finished "build <spec>"`
/// line for this package: the success signal, alongside exit code 0
/// and the RPMs present. The built target is the quoted name after
/// `finished "build `, compared by file name exactly: a suffix of
/// another spec name (`barfoo.spec` for package `foo`) must not
/// match, while a path prefix (`/some/dir/foo.spec`) still does.
fn has_finished_line(log: &str, package: &str) -> bool {
    let spec = format!("{package}.spec");
    log.lines().any(|line| {
        let Some((_, rest)) = line.split_once("finished \"build ") else {
            return false;
        };
        let Some(target) = rest.split('"').next() else {
            return false;
        };
        Path::new(target)
            .file_name()
            .is_some_and(|name| name == spec.as_str())
    })
}

/// The rpmlint summary from a build log: rpmlint's own summary line
/// (`N packages and M specfiles checked; X errors, Y warnings.`)
/// when present, else the count of `E:`/`W:` lines in the log.
/// Badness is the sum of the `Badness:` annotations, 0 when the log
/// carries none. The summary line is read from the whole log (its
/// shape is distinctive); the fallback count and the badness sum
/// cover only the rpmlint section — the lines after the last line
/// that names rpmlint — so `%check` output cannot be miscounted as
/// rpmlint findings. With no such line the whole log is the section.
fn parse_rpmlint_summary(log: &str) -> RpmlintSummary {
    let lines: Vec<&str> = log.lines().collect();
    let mut summary = RpmlintSummary::default();
    let mut saw_summary = false;
    for line in &lines {
        if let Some((errors, warnings)) = parse_rpmlint_summary_line(line) {
            saw_summary = true;
            summary.errors = errors;
            summary.warnings = warnings;
        }
    }
    // The rpmlint section starts after the last line naming rpmlint
    // (its header or invocation, which precedes the findings). A log
    // that never names rpmlint has no section to bound.
    let section_start = lines
        .iter()
        .rposition(|l| l.to_ascii_lowercase().contains("rpmlint"))
        .map(|i| i + 1)
        .unwrap_or(0);
    for line in &lines[section_start..] {
        if let Some(badness) = parse_badness(line) {
            summary.badness += badness;
        }
    }
    if !saw_summary {
        // No summary line: fall back to counting the finding lines.
        // A finding reads `<pkg>: E: ...` (or a bare `E: ...` when
        // the log carries no package prefix).
        for line in &lines[section_start..] {
            let trimmed = line.trim_start();
            if line.contains(": E:") || trimmed.starts_with("E:") {
                summary.errors += 1;
            } else if line.contains(": W:") || trimmed.starts_with("W:") {
                summary.warnings += 1;
            }
        }
    }
    summary
}

/// Parse `… checked; 3 errors, 2 warnings.` — rpmlint's summary
/// line. Returns None when the line is not a summary line. The
/// singular forms ("1 error, 1 warning") are accepted too.
fn parse_rpmlint_summary_line(line: &str) -> Option<(u64, u64)> {
    let (_, rest) = line.split_once("checked;")?;
    let rest = rest.trim().trim_end_matches('.');
    let (errors, rest) = rest
        .split_once(" errors,")
        .or_else(|| rest.split_once(" error,"))?;
    let warnings = rest
        .trim()
        .strip_suffix(" warnings")
        .or_else(|| rest.trim().strip_suffix(" warning"))?;
    Some((
        errors.trim().parse::<u64>().ok()?,
        warnings.trim().parse::<u64>().ok()?,
    ))
}

/// Parse a `Badness: N` annotation on a log line.
fn parse_badness(line: &str) -> Option<u64> {
    let (_, rest) = line.split_once("Badness:")?;
    rest.trim()
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|n| n.parse::<u64>().ok())
}

/// The `%check` summary from a build log: ctest's `N% tests passed,
/// M tests failed out of K`. None when the log carries no such line.
fn parse_check_summary(log: &str) -> Option<CheckSummary> {
    for line in log.lines() {
        if let Some(summary) = parse_ctest_line(line) {
            return Some(summary);
        }
    }
    None
}

fn parse_ctest_line(line: &str) -> Option<CheckSummary> {
    let (_, rest) = line.split_once("% tests passed,")?;
    let (failed, rest) = rest.trim().split_once(" tests failed out of ")?;
    let total = rest.trim().parse::<u64>().ok()?;
    let failed = failed.trim().parse::<u64>().ok()?;
    // The percent is the pass share; derive passed from the total.
    if total < failed {
        return None;
    }
    Some(CheckSummary {
        passed: total - failed,
        failed,
        total,
    })
}

/// The sccache hit count from a build log, best-effort: the engine
/// prints `sccache … <n> cache hits` in some versions, nothing in
/// others. The count is the digit run immediately before
/// "cache hits" — the first digit run on the line may be a version.
fn parse_sccache_hits(log: &str) -> Option<u64> {
    for line in log.lines() {
        let lower = line.to_ascii_lowercase();
        let Some(pos) = lower.find("cache hits") else {
            continue;
        };
        if !lower[..pos].contains("sccache") {
            continue;
        }
        let before = lower[..pos].trim_end();
        let digits: String = before
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect::<Vec<char>>()
            .into_iter()
            .rev()
            .collect();
        if let Ok(n) = digits.parse::<u64>() {
            return Some(n);
        }
    }
    None
}

/// The last `n` lines of a log, for the `BuildFailed` tail.
/// Read a build log (at most `LOG_TAIL_CAP_BYTES`, lossy UTF-8):
/// the tail by default, the whole log when `uncapped` is set. The
/// verdict, the rpmlint/check/sccache summaries and the `finished`
/// line all live at the end of the log, so the tail is what the
/// parsers need. A missing or unreadable log
/// reads as empty, as before — a strict read would fail a good
/// build on its verdict. When the head was skipped, the tail
/// starts at a line boundary: the partial first line is dropped —
/// unless the window starts exactly on a line boundary, in which
/// case its first line is complete and is kept — so the result is
/// whole lines.
fn read_log_tail(path: &Path, uncapped: bool) -> String {
    let Ok(mut file) = File::open(path) else {
        return String::new();
    };
    use std::io::Seek;
    use std::io::SeekFrom;
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let skipped = !uncapped && len > LOG_TAIL_CAP_BYTES;
    // When the head was skipped, the tail window starts at
    // `len - LOG_TAIL_CAP_BYTES`. When the byte before the window
    // is `\n`, the window starts exactly on a line boundary and
    // its first line is complete — keep it. Otherwise the window
    // starts mid-line and the partial first line is dropped.
    // Dropping unconditionally would delete a complete first
    // line on the boundary, so a log whose last line is the
    // `finished` line would read as an empty tail and a green
    // build would be reported failed.
    let on_boundary = skipped
        && file
            .seek(SeekFrom::Start(len - LOG_TAIL_CAP_BYTES - 1))
            .is_ok()
        && {
            let mut probe = [0u8; 1];
            file.read_exact(&mut probe).is_ok() && probe[0] == b'\n'
        };
    if skipped {
        let _ = file.seek(SeekFrom::Start(len - LOG_TAIL_CAP_BYTES));
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    if skipped && !on_boundary {
        match bytes.iter().position(|&b| b == b'\n') {
            Some(pos) => {
                bytes.drain(..=pos);
            }
            None => return String::new(),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn last_lines(log: &str, n: usize) -> Vec<&str> {
    let lines: Vec<&str> = log.lines().collect();
    let skip = lines.len().saturating_sub(n);
    lines[skip..].to_vec()
}

/// The hdrmd5 of an RPM file: the MD5 of the header section plus the
/// payload — the bytes from the main-header start to the end of the
/// file. That is what the signature header's tag 1004 carries, and
/// what the backend puts in the buildinfo: the name is misleading,
/// the payload is included (verified against real Tumbleweed RPMs).
/// The file is lead (96 bytes) + signature header + header + payload.
///
/// The digest needs only the main-header *offset*, which the
/// signature header's prologue declares — so the probe reads the
/// 112-byte prologue, seeks to the main header, and streams the
/// rest through the hasher. No header store is buffered: a real
/// main header can be megabytes (kernel-obs-build's main-header
/// store is 5,503,288 bytes), and a signature store can be just
/// as large, so a probe that buffers the header up to a cap
/// rejects real RPMs. The header length is validated against the file
/// length instead: a file truncated inside its header still fails
/// correctly, its digest never matching the expected hdrmd5.
fn rpm_hdrmd5(path: &Path) -> Result<String, String> {
    use std::io::{Seek, SeekFrom};
    let mut file = File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let file_len = file
        .metadata()
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        .len();
    // The lead (96) plus the signature header's 16-byte prologue:
    // the prologue declares the signature header's length, which
    // is all the offset needs. A file shorter than that is not an
    // RPM at all.
    let mut prologue = [0u8; 112];
    file.read_exact(&mut prologue).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            "file too short to be an RPM".to_string()
        } else {
            format!("cannot read {}: {e}", path.display())
        }
    })?;
    let header_start = rpm_header_start(&prologue, file_len)?;
    // The main header's own prologue: its length must fit in the
    // file, or the file is truncated inside its header.
    let mut main_prologue = [0u8; 16];
    file.seek(SeekFrom::Start(header_start as u64))
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    file.read_exact(&mut main_prologue).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            "truncated RPM header".to_string()
        } else {
            format!("cannot read {}: {e}", path.display())
        }
    })?;
    let (header_len, _) =
        rpm_header_len(&main_prologue).ok_or_else(|| "malformed RPM header".to_string())?;
    if header_start
        .checked_add(header_len)
        .is_none_or(|end| end as u64 > file_len)
    {
        return Err("truncated RPM header".to_string());
    }
    file.seek(SeekFrom::Start(header_start as u64))
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut ctx = md5::Context::new();
    let mut chunk = [0u8; 8192];
    loop {
        match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => ctx.consume(&chunk[..n]),
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        }
    }
    Ok(format!("{:x}", ctx.compute()))
}

/// The offset of the main header in an RPM file, from the lead
/// and the signature header's prologue alone: lead magic
/// `ED AB EE DB`, then the signature header (magic `8E AD E8`,
/// 8-byte aligned). The signature header's store is never read —
/// only its declared length is used — so a store of any size is
/// fine. `file_len` is the file's length: the main header must
/// start inside the file. Every slice is bounds-checked: a corrupt
/// cache file must error, never panic.
fn rpm_header_start(prologue: &[u8; 112], file_len: u64) -> Result<usize, String> {
    if prologue[0..4] != [0xed, 0xab, 0xee, 0xdb] {
        return Err("not an RPM file (bad lead magic)".to_string());
    }
    let sig_start = 96;
    let (sig_len, _) = rpm_header_len(&prologue[sig_start..])
        .ok_or_else(|| "malformed RPM signature header".to_string())?;
    let header_start = sig_start
        .checked_add(sig_len)
        .ok_or_else(|| "malformed RPM signature header (size overflows)".to_string())?;
    if header_start as u64 > file_len {
        return Err("malformed RPM signature header (header past end of file)".to_string());
    }
    Ok(header_start)
}

/// The length of one RPM header section starting at `bytes`:
/// 16 bytes of prologue + index + store, padded to a multiple of 8.
/// Returns (total length, entry count).
fn rpm_header_len(bytes: &[u8]) -> Option<(usize, usize)> {
    if bytes.len() < 16 {
        return None;
    }
    if bytes[0..3] != [0x8e, 0xad, 0xe8] {
        return None;
    }
    let nindex = u32::from_be_bytes(bytes[8..12].try_into().ok()?) as usize;
    let hsize = u32::from_be_bytes(bytes[12..16].try_into().ok()?) as usize;
    // Checked: a corrupt file's huge counts must error, never
    // overflow (debug) or wrap (release) into a bogus length.
    let len = 16usize
        .checked_add(16usize.checked_mul(nindex)?)?
        .checked_add(hsize)?;
    let aligned = len.div_ceil(8).checked_mul(8)?;
    Some((aligned, nindex))
}

/// Eviction and the companion operations.
impl<'a> LocalBuild<'a> {
    /// Take the shared eviction lock (non-blocking). None when
    /// another build holds it — the caller skips eviction instead of
    /// deleting under a concurrent eviction.
    fn acquire_eviction_lock(&self) -> Option<BuildLock> {
        let path = self.cache_dir.join(EVICTION_LOCK);
        let _ = fs::create_dir_all(&self.cache_dir);
        // No truncate: the file carries no content, and truncating
        // on open would be a write to a file another build may hold.
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .ok()?;
        match file.try_lock_exclusive() {
            Ok(()) => Some(BuildLock { _file: file }),
            Err(_) => None,
        }
    }

    /// The package cache dir for a project, repository and arch:
    /// `pkg/<project>/<repo>/<arch>/`, with every component encoded
    /// (see `encode_path_segment`). The download/write path builds
    /// its dir through this helper; the lock path builds the same
    /// encoded components itself, so the two can never disagree
    /// about where a project's files live.
    fn pkg_dir(&self, project: &str, repo: &str, arch: &str) -> PathBuf {
        self.cache_dir
            .join("pkg")
            .join(Self::encode_path_segment(project))
            .join(Self::encode_path_segment(repo))
            .join(Self::encode_path_segment(arch))
    }

    /// The project-repo-arch lock file:
    /// `pkg/<project>/<repo>/<arch>.lock`, next to the pkg files it
    /// guards. The project is part of the key: the same repo name
    /// in two projects is two different cache dirs, so it needs two
    /// different locks. Every component is encoded (see
    /// `encode_path_segment`), including the arch in the file name,
    /// so the lock path is the encoded form of the real coordinates.
    fn pkg_lock_path(&self, project: &str, repo: &str, arch: &str) -> PathBuf {
        self.cache_dir
            .join("pkg")
            .join(Self::encode_path_segment(project))
            .join(Self::encode_path_segment(repo))
            .join(format!("{}.lock", Self::encode_path_segment(arch)))
    }

    /// Open a project-repo-arch lock file (no truncate: the file
    /// carries no content).
    fn open_pkg_lock(&self, project: &str, repo: &str, arch: &str) -> Result<File, GeectlError> {
        let path = self.pkg_lock_path(project, repo, arch);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                GeectlError::new(
                    ErrorKind::InternalError,
                    format!("cannot create package cache {}: {e}", parent.display()),
                )
            })?;
        }
        File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| {
                GeectlError::new(
                    ErrorKind::InternalError,
                    format!("cannot open package cache lock {}: {e}", path.display()),
                )
            })
    }

    /// Take a project-repo-arch lock, blocking: the per-file
    /// download/write path holds it for the whole ensure (check,
    /// download, write, verify). A download is bounded by the
    /// transport's global timeout plus verify time, so the hold is
    /// bounded; a build that waits here is waiting on another
    /// build's download of the same project-repo-arch, not on a
    /// build.
    fn acquire_pkg_lock(
        &self,
        project: &str,
        repo: &str,
        arch: &str,
    ) -> Result<BuildLock, GeectlError> {
        let file = self.open_pkg_lock(project, repo, arch)?;
        file.lock_exclusive().map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot lock package cache {project}/{repo}/{arch}: {e}"),
            )
        })?;
        Ok(BuildLock { _file: file })
    }

    /// Take a project-repo-arch lock, non-blocking: eviction holds
    /// it while deleting that dir's pkg files. None when a download
    /// holds it — the file is skipped, not deleted under the
    /// download.
    fn try_pkg_lock(&self, project: &str, repo: &str, arch: &str) -> Option<BuildLock> {
        let file = self.open_pkg_lock(project, repo, arch).ok()?;
        match file.try_lock_exclusive() {
            Ok(()) => Some(BuildLock { _file: file }),
            Err(_) => None,
        }
    }

    /// The cache usage: total bytes and per-area bytes, plus the cap
    /// and low-water mark. The image size is every digest of this
    /// vertical's repository, from podman when it answers; when it
    /// does not, the images count as 0 and the record says so by
    /// the missing size, not by a guess.
    pub fn cache_usage(&self) -> Result<CacheUsage, GeectlError> {
        let images = self.vertical_images();
        self.cache_usage_with_images(&images)
    }

    /// `cache_usage` against a listing already taken: the plan
    /// path takes the listing once and reuses it for the pre-check,
    /// the usage and the image step, instead of shelling out to
    /// `podman images` plus one `inspect` per digest up to four
    /// times per over-cap build. The apply path still re-reads
    /// fresh (`cache_usage`), so a delete that freed bytes another
    /// build added is not reported against a stale total.
    fn cache_usage_with_images(&self, images: &[(String, u64)]) -> Result<CacheUsage, GeectlError> {
        let mut usage = CacheUsage {
            cap_bytes: self.cache_cap_bytes,
            low_water_bytes: self.low_water(),
            ..CacheUsage::default()
        };
        usage.pkg_bytes = dir_size(&self.cache_dir.join("pkg"));
        usage.sccache_bytes = dir_size(&self.cache_dir.join("sccache"));
        usage.roots_bytes = dir_size(&self.cache_dir.join("build-roots"));
        usage.total_bytes = usage.pkg_bytes + usage.sccache_bytes + usage.roots_bytes;
        // The images are the one area not on the cache filesystem:
        // count every digest of this vertical's repository, not
        // just the current one — eviction's image step subtracts
        // superseded digests from the same total, so the two must
        // agree. When podman will not answer, the images count as 0.
        usage.image_bytes = images.iter().map(|(_, size)| *size).sum();
        usage.total_bytes += usage.image_bytes;
        Ok(usage)
    }

    /// The digests of this vertical's image in the local podman
    /// store, with sizes: only the Tumbleweed repository, never the
    /// whole local image set. A digest is listed once even when
    /// podman tags it twice (`latest` + a dated tag): one row per
    /// repo:tag would double-count its bytes and list it twice for
    /// deletion. The current digest is resolved by inspecting
    /// `DEFAULT_IMAGE` directly, not only from the listing: if
    /// podman spells the repository differently (a mirror, another
    /// reference), the listing's exact-match filter would drop the
    /// image entirely and the cap would stop covering images.
    fn vertical_images(&self) -> Vec<(String, u64)> {
        let Some(podman) = which(&self.podman) else {
            return Vec::new();
        };
        let mut out: Vec<(String, u64)> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        // The current digest first, as podman resolves it.
        if let Ok(Some(digest)) = self.podman_digest(&podman) {
            let size = self.podman_image_size(&digest).unwrap_or(0);
            seen.push(digest.clone());
            out.push((digest, size));
        }
        let mut cmd = Command::new(podman);
        cmd.args(["images", "--format", "{{.Repository}} {{.Digest}}"]);
        let output = run_with_timeout(&mut cmd, Duration::from_secs(PROBE_TIMEOUT_SECS));
        let Ok(output) = output else {
            return out;
        };
        if !output.status.success() {
            return out;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            let (Some(repo), Some(digest)) = (parts.next(), parts.next()) else {
                continue;
            };
            // No repository filter here would delete every local
            // image on the machine (a dev database, another
            // project's image) — only this vertical's repository.
            if repo != IMAGE_REPOSITORY || digest == "<none>" {
                continue;
            }
            if seen.contains(&digest.to_string()) {
                continue;
            }
            seen.push(digest.to_string());
            let size = self.podman_image_size(digest).unwrap_or(0);
            out.push((digest.to_string(), size));
        }
        out
    }

    /// Every build root's marker, parsed: the basis for the
    /// last-success digests and the most-recent-build digest. A
    /// marker that does not parse is skipped — its root's key and
    /// times are unknown, so eviction cannot reason about it.
    fn root_markers(&self) -> Vec<RootMarker> {
        let roots = self.cache_dir.join("build-roots");
        let mut markers: Vec<RootMarker> = Vec::new();
        let mut stack: Vec<PathBuf> = vec![roots];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.file_name().is_some_and(|n| n == ROOT_MARKER) {
                    if let Ok(text) = fs::read_to_string(&path) {
                        if let Ok(marker) = serde_json::from_str::<RootMarker>(&text) {
                            markers.push(marker);
                        }
                    }
                }
            }
        }
        markers
    }

    /// The digest the most recent build recorded (the marker with
    /// the latest last-build time): image eviction never removes it.
    fn most_recent_build_digest(&self) -> Option<String> {
        self.root_markers()
            .iter()
            .max_by_key(|m| m.last_build_unix)
            .and_then(|m| m.image_digest.clone())
    }

    /// The (project, package) pairs with a build root whose lock is
    /// held: a sccache store for such a package is never evicted.
    /// The pairs are the encoded directory forms (see
    /// `encode_path_segment`), matching what
    /// `store_project_package` reads from a store path — the marker
    /// keeps the real names, so the marker side encodes here.
    fn locked_root_keys(&self) -> std::collections::HashSet<(String, String)> {
        let mut locked = std::collections::HashSet::new();
        let roots = self.cache_dir.join("build-roots");
        let mut stack: Vec<PathBuf> = vec![roots];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.file_name().is_some_and(|n| n == ROOT_MARKER) {
                    let Ok(text) = fs::read_to_string(&path) else {
                        continue;
                    };
                    let Ok(marker) = serde_json::from_str::<RootMarker>(&text) else {
                        continue;
                    };
                    let Some(root) = path.parent() else {
                        continue;
                    };
                    if root_lock_held(root) {
                        locked.insert((
                            Self::encode_path_segment(&marker.project),
                            Self::encode_path_segment(&marker.package),
                        ));
                    }
                }
            }
        }
        locked
    }

    /// The current image digest from the sidecar's refresh record,
    /// via podman. None when no image is recorded or podman will
    /// not answer.
    fn current_image_digest(&self) -> Option<String> {
        let podman = which(&self.podman)?;
        self.podman_digest(&podman).ok().flatten()
    }

    /// Plan the eviction: when usage is above the cap, select which
    /// cache contents go, in the settled order, until the usage is
    /// at or below the low-water mark — pkg files (oldest use
    /// first), then sccache stores, then build roots, then
    /// superseded images. The cap is the trigger, the low-water
    /// mark is the target: a cache between the two is left alone.
    /// Never selected: locked roots, the most recent root per
    /// (project, package), the current dep set,
    /// the current build's sccache store, a store whose package
    /// has a locked build root, the 24h grace period, the current
    /// image digest, the digest the most recent build recorded.
    /// This only plans: it deletes nothing. The caller deletes what
    /// the plan lists (see [`Self::evict`] and [`Self::prune_apply`]).
    /// `protected_bytes` is the bytes skipped by a protection rule
    /// while planning, capped at the bytes above the low-water mark.
    fn evict_plan(
        &self,
        current_dep_files: &[String],
        current_dep_names: &[String],
        current_digest: Option<&str>,
        current_sccache: Option<&Path>,
        images: &[(String, u64)],
    ) -> Result<EvictionReport, GeectlError> {
        let mut report = EvictionReport {
            cap_bytes: self.cache_cap_bytes,
            low_water_bytes: self.low_water(),
            ..EvictionReport::default()
        };
        let usage = self.cache_usage_with_images(images)?;
        report.bytes_before = usage.total_bytes;
        report.bytes_after = usage.total_bytes;
        if usage.total_bytes <= report.cap_bytes {
            return Ok(report);
        }
        report.ran = true;
        // The simulated usage: candidates are selected until this
        // reaches the low-water mark. Nothing is deleted here.
        let mut remaining = usage.total_bytes;
        let mut protected: u64 = 0;
        let now = now_unix();

        // 1. Package files, oldest use (mtime) first, across every
        // repo/arch in the cache. The current dep set and anything
        // used in the last 24h never evict.
        let mut pkg_files: Vec<(PathBuf, u64, u64)> = Vec::new();
        let mut pkg_skipped: u64 = 0;
        collect_pkg_files(
            &self.cache_dir.join("pkg"),
            current_dep_files,
            current_dep_names,
            now,
            &mut pkg_files,
            &mut pkg_skipped,
        );
        protected += pkg_skipped;
        pkg_files.sort_by_key(|(_, _, mtime)| *mtime);
        for (path, size, _) in pkg_files {
            if remaining <= report.low_water_bytes {
                break;
            }
            remaining = remaining.saturating_sub(size);
            report.evicted.push(EvictedItem {
                path: path.display().to_string(),
                bytes: size,
                kind: "pkg".to_string(),
                reason: "lru".to_string(),
            });
        }

        // 2. Sccache stores, oldest use first. The current build's
        // store, a store whose package has a locked build root, and
        // anything in the 24h grace period never evict.
        if remaining > report.low_water_bytes {
            let mut stores: Vec<(PathBuf, u64, u64)> = Vec::new();
            let mut stores_skipped: u64 = 0;
            if let Ok(entries) = fs::read_dir(self.cache_dir.join("sccache")) {
                for entry in entries.flatten() {
                    collect_sccache_stores(&entry.path(), &mut stores, now, &mut stores_skipped);
                }
            }
            protected += stores_skipped;
            let locked_keys = self.locked_root_keys();
            stores.retain(|(path, size, _)| {
                if current_sccache.is_some_and(|keep| path == keep) {
                    protected += *size;
                    return false;
                }
                // The store's (project, package) from its path
                // (see `store_project_package`): an unrecognised
                // shape is protected, never evicted — a locked
                // root must not be missed because the path parsed
                // wrong.
                match self.store_project_package(path) {
                    Some(key) => {
                        if locked_keys.contains(&key) {
                            protected += *size;
                            return false;
                        }
                    }
                    None => {
                        protected += *size;
                        return false;
                    }
                }
                true
            });
            stores.sort_by_key(|(_, _, mtime)| *mtime);
            for (path, size, _) in stores {
                if remaining <= report.low_water_bytes {
                    break;
                }
                remaining = remaining.saturating_sub(size);
                report.evicted.push(EvictedItem {
                    path: path.display().to_string(),
                    bytes: size,
                    kind: "sccache".to_string(),
                    reason: "lru".to_string(),
                });
            }
        }

        // 3. Build roots, oldest last-build first. A locked root, the
        // most recent root per (project, package), and anything in
        // the 24h grace period never evict. The grouping is by
        // project+package, keyed on the marker's real coordinates —
        // deliberately not the encoded directory form: grouping is
        // by project identity, so two distinct projects must never
        // group together (`home:foo` and `home_foo` are different
        // projects with different roots), and the marker is the only
        // source of the real coordinates. The key is never compared
        // against a directory-derived key, so no encoding is needed
        // here. One root per 5-tuple key exists (the path is the
        // key), so grouping by the full key protected every
        // root that carries a parseable marker and roots effectively
        // never evicted. Only the newest root of the package is
        // protected, and an older root in another repo, arch or
        // flavor is regenerable by reinstalling its deps (from the
        // pkg cache when still cached, else re-downloaded from OBS)
        // — root reuse
        // itself stays per 5-tuple, so that repo's next build
        // recreates its root rather than reusing the newest one.
        if remaining > report.low_water_bytes {
            let mut roots =
                collect_root_entries(&self.cache_dir.join("build-roots"), now, &mut protected);
            // The most recent root per (project, package) is
            // protected, whatever its last build's outcome: a failed
            // root is still the root the next build reuses.
            let mut newest: HashMap<(String, String), u64> = HashMap::new();
            for entry in &roots {
                let key = (entry.project.clone(), entry.package.clone());
                let best = newest.entry(key).or_insert(0);
                *best = (*best).max(entry.last_build_unix);
            }
            for entry in &roots {
                let key = (entry.project.clone(), entry.package.clone());
                if newest.get(&key).copied().unwrap_or(0) == entry.last_build_unix {
                    protected += dir_size(&entry.path);
                }
            }
            roots.retain(|entry| {
                let key = (entry.project.clone(), entry.package.clone());
                newest.get(&key).copied().unwrap_or(0) != entry.last_build_unix
            });
            roots.sort_by_key(|entry| entry.last_build_unix);
            for entry in roots {
                if remaining <= report.low_water_bytes {
                    break;
                }
                let size = dir_size(&entry.path);
                remaining = remaining.saturating_sub(size);
                report.evicted.push(EvictedItem {
                    path: entry.path.display().to_string(),
                    bytes: size,
                    kind: "root".to_string(),
                    reason: "lru".to_string(),
                });
            }
        }

        // 4. Superseded images: an older digest of the image, once a
        // newer digest has been pulled and a build has succeeded
        // with it. Never the current digest, never the digest the
        // most recent build recorded.
        if remaining > report.low_water_bytes {
            self.plan_superseded_images(
                current_digest,
                &mut remaining,
                &mut report,
                &mut protected,
                images,
            );
        }

        report.bytes_after = remaining;
        report.freed_bytes = report.evicted.iter().map(|item| item.bytes).sum();
        report.shortfall_bytes = remaining.saturating_sub(report.low_water_bytes);
        report.protected_bytes =
            protected.min(report.bytes_before.saturating_sub(report.low_water_bytes));
        Ok(report)
    }

    /// Evict at build start: plan under the shared eviction lock,
    /// then delete what the plan lists. A failure to delete one
    /// candidate is recorded and the eviction continues with the
    /// next — a stuck file must not block the build. When another
    /// build holds the eviction lock, this run skips eviction
    /// (`ran` stays false, nothing deleted) rather than deleting
    /// under a concurrent eviction.
    fn evict(
        &self,
        current_dep_files: &[String],
        current_dep_names: &[String],
        current_digest: Option<&str>,
        current_sccache: Option<&Path>,
    ) -> Result<EvictionReport, GeectlError> {
        // Cheap pre-check: no lock when already under the cap. The
        // images listing is taken once here and handed to the plan,
        // so one over-cap build shells out to podman once, not
        // once per usage/image computation.
        let images = self.vertical_images();
        let usage = self.cache_usage_with_images(&images)?;
        if usage.total_bytes <= self.cache_cap_bytes {
            return Ok(EvictionReport {
                cap_bytes: self.cache_cap_bytes,
                low_water_bytes: self.low_water(),
                bytes_before: usage.total_bytes,
                bytes_after: usage.total_bytes,
                ..EvictionReport::default()
            });
        }
        let Some(_evict_lock) = self.acquire_eviction_lock() else {
            return Ok(EvictionReport {
                cap_bytes: self.cache_cap_bytes,
                low_water_bytes: self.low_water(),
                bytes_before: usage.total_bytes,
                bytes_after: usage.total_bytes,
                failures: vec!["eviction lock held by another build; eviction skipped".to_string()],
                ..EvictionReport::default()
            });
        };
        let mut report = self.evict_plan(
            current_dep_files,
            current_dep_names,
            current_digest,
            current_sccache,
            &images,
        )?;
        self.apply_eviction_plan(&mut report);
        Ok(report)
    }

    /// Delete what a plan lists. Each deletion is recorded; a
    /// failure to delete one entry is recorded and the rest still
    /// go. `bytes_after` is re-read after the deletes so it states
    /// the cache as it stands, not as the plan assumed. The plan is
    /// stale by the time it applies, so every root's lock is held
    /// across its delete (a root locked between plan and apply is
    /// never deleted under a running build), every sccache store's
    /// root lock is held across its delete the same way, and every
    /// pkg file's project-repo-arch lock is taken non-blocking
    /// before its delete (a file being downloaded is skipped, not
    /// deleted under the download).
    fn apply_eviction_plan(&self, report: &mut EvictionReport) {
        let planned = std::mem::take(&mut report.evicted);
        let mut freed = 0u64;
        for item in planned {
            let result = if item.kind == "image" {
                self.remove_image(Path::new(&item.path))
            } else if item.kind == "root" {
                let path = Path::new(&item.path);
                // Hold the root's flock across the delete: the
                // plan-time probe is stale by now, and a build that
                // locks between probe and delete would have its root
                // removed under it. The lock file itself goes last
                // (see `remove_root_dir`): unlinking it first would
                // let a concurrent build create a fresh lock file
                // and run inside the root being deleted.
                let held = match try_hold_root_lock(path) {
                    Ok(held) => held,
                    Err(()) => {
                        report.failures.push(format!(
                            "cannot delete {}: the root was locked after the plan was made",
                            item.path
                        ));
                        continue;
                    }
                };
                let _held = held;
                self.remove_root_dir(path)
            } else if item.kind == "sccache" {
                // Hold the store's root lock across the delete: the
                // plan-time locked-root snapshot is stale by now, and
                // a store whose package started a build after the
                // plan must not be deleted under it. The store is
                // per root, so the root's flock is the guard — a
                // build of that root holds it from before the store
                // is created until the build ends. An unrecognised
                // store path is never deleted: it cannot be
                // attributed to a root, so no lock could guard it.
                // A store whose root is already gone has no build
                // in it (a build creates the root before the store),
                // so it deletes unguarded.
                let store_path = Path::new(&item.path);
                let Some(root) = self.store_root_path(store_path) else {
                    report.failures.push(format!(
                        "cannot delete {}: its store path has an unrecognised shape",
                        item.path
                    ));
                    continue;
                };
                let held = if root.exists() {
                    match try_hold_root_lock(&root) {
                        Ok(held) => held,
                        Err(()) => {
                            report.failures.push(format!(
                                "cannot delete {}: its package has a locked build root",
                                item.path
                            ));
                            continue;
                        }
                    }
                } else {
                    None
                };
                let _held = held;
                fs::remove_dir_all(&item.path)
            } else if item.kind == "pkg" {
                match self.pkg_lock_for_path(Path::new(&item.path)) {
                    Some(_lock) => fs::remove_file(&item.path),
                    None => {
                        // A `None` is either a lock held by a
                        // download or a path of an unrecognised
                        // shape — a legacy file the collector
                        // recursed over, which no lock could ever
                        // guard. Report them apart: the shape
                        // case is not a concurrent download.
                        if self.pkg_path_has_current_shape(Path::new(&item.path)) {
                            report.failures.push(format!(
                                "cannot delete {}: its project-repo-arch lock is held by a download",
                                item.path
                            ));
                        } else {
                            report.failures.push(format!(
                                "cannot delete {}: its path has an unrecognised shape",
                                item.path
                            ));
                        }
                        continue;
                    }
                }
            } else if Path::new(&item.path).is_dir() {
                fs::remove_dir_all(&item.path)
            } else {
                fs::remove_file(&item.path)
            };
            match result {
                Ok(()) => {
                    freed += item.bytes;
                    report.evicted.push(item);
                }
                Err(e) => {
                    report
                        .failures
                        .push(format!("cannot delete {}: {e}", item.path));
                }
            }
        }
        report.freed_bytes = freed;
        if let Ok(usage) = self.cache_usage() {
            report.bytes_after = usage.total_bytes;
        } else {
            report.bytes_after = report.bytes_before.saturating_sub(freed);
        }
        report.shortfall_bytes = report.bytes_after.saturating_sub(report.low_water_bytes);
    }

    /// Whether `path` is a current-layout pkg file: exactly
    /// `pkg/<project>/<repo>/<arch>/<file>` under the cache dir.
    /// The apply arm uses it to tell an unrecognised shape (a
    /// legacy file that can never be evicted, and is not a
    /// concurrent download) from a lock held by a download.
    fn pkg_path_has_current_shape(&self, path: &Path) -> bool {
        let Ok(rel) = path.strip_prefix(self.cache_dir.join("pkg")) else {
            return false;
        };
        rel.components().count() == 4
    }

    /// The project-repo-arch lock for a pkg file path
    /// (`pkg/<project>/<repo>/<arch>/<file>`), non-blocking. None
    /// when the path is not under the pkg cache, has an
    /// unrecognised shape, or a download holds the lock — the
    /// caller skips the file. The shape is exact: a legacy
    /// `pkg/<repo>/<arch>/<file>` path must not be read as
    /// project/repo/arch, or the lock taken would guard nothing
    /// and the file would be deleted unguarded.
    fn pkg_lock_for_path(&self, path: &Path) -> Option<BuildLock> {
        let rel = path.strip_prefix(self.cache_dir.join("pkg")).ok()?;
        let mut parts = rel.components();
        let project = parts.next()?.as_os_str().to_string_lossy().to_string();
        let repo = parts.next()?.as_os_str().to_string_lossy().to_string();
        let arch = parts.next()?.as_os_str().to_string_lossy().to_string();
        let _file = parts.next()?;
        if parts.next().is_some() {
            return None;
        }
        // The components on disk are the encoded forms: decode
        // them back to the real coordinates before the lock lookup,
        // or `pkg_lock_path` would encode the `%` a second time
        // (`home%3Afoo` becoming `home%253Afoo`) and the lock would
        // never match the one the download holds.
        self.try_pkg_lock(
            &Self::decode_path_segment(&project),
            &Self::decode_path_segment(&repo),
            &Self::decode_path_segment(&arch),
        )
    }

    /// Select superseded images for the plan: digests of this
    /// vertical's repository that are neither the current digest
    /// nor the digest the most recent build recorded. An older
    /// digest may go only once a newer digest has been pulled and a
    /// build has succeeded with it — here that is the current digest
    /// with a last-success marker behind it. Without that evidence
    /// no image evicts: deleting the only working image would leave
    /// the next build pulling again. Digests skipped by a
    /// protection rule add their sizes to `protected`.
    fn plan_superseded_images(
        &self,
        current_digest: Option<&str>,
        remaining: &mut u64,
        report: &mut EvictionReport,
        protected: &mut u64,
        images: &[(String, u64)],
    ) {
        let Some(current) = current_digest else {
            return;
        };
        // The gate: the current (newer) digest must have a
        // successful build behind it.
        if !self.last_success_digests().iter().any(|d| d == current) {
            return;
        }
        let most_recent = self.most_recent_build_digest();
        for (digest, size) in images {
            if *remaining <= report.low_water_bytes {
                break;
            }
            if digest.as_str() == current {
                *protected += *size;
                continue;
            }
            if most_recent.as_deref() == Some(digest.as_str()) {
                *protected += *size;
                continue;
            }
            *remaining = remaining.saturating_sub(*size);
            report.evicted.push(EvictedItem {
                path: digest.clone(),
                bytes: *size,
                kind: "image".to_string(),
                reason: "superseded-image".to_string(),
            });
        }
    }

    /// Every digest with a last-success marker behind it, across all
    /// build roots.
    fn last_success_digests(&self) -> Vec<String> {
        let mut digests: Vec<String> = Vec::new();
        for marker in self.root_markers() {
            if let Some(digest) = marker.last_success_digest {
                if !digests.contains(&digest) {
                    digests.push(digest);
                }
            }
        }
        digests
    }

    /// Build the plan for `build-prune` without deleting anything:
    /// the same eviction order the build start uses, against the
    /// current usage. No dep set is protected (no build is running),
    /// but locked roots, most-recent roots, the grace period and the
    /// current digest still are.
    pub fn prune_plan(&self) -> Result<EvictionReport, GeectlError> {
        let images = self.vertical_images();
        self.evict_plan(
            &[],
            &[],
            self.current_image_digest().as_deref(),
            None,
            &images,
        )
    }

    /// Delete what `prune_plan` lists: plan under the shared
    /// eviction lock, then delete. Each deletion is recorded; a
    /// failure to delete one entry is recorded and the rest still
    /// go. Returns the report of what was (and was not) deleted.
    pub fn prune_apply(&self) -> Result<EvictionReport, GeectlError> {
        self.evict(&[], &[], self.current_image_digest().as_deref(), None)
    }

    /// Remove one image by digest via podman.
    fn remove_image(&self, digest: &Path) -> std::io::Result<()> {
        let Some(podman) = which(&self.podman) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "podman not found on PATH",
            ));
        };
        let mut cmd = Command::new(podman);
        cmd.args(["rmi", digest.to_string_lossy().as_ref()]);
        let output = run_with_timeout(&mut cmd, Duration::from_secs(PROBE_TIMEOUT_SECS))?;
        let status = output.status;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "podman rmi {} failed",
                digest.display()
            )))
        }
    }

    /// Open a shell in the build container for a built root: the
    /// engine's `--shell-cmd` path with the given commands joined by
    /// ` && `. The commands are shell fragments executed inside the
    /// build container, not on the host: they are joined into one
    /// `--shell-cmd` string and the engine runs it in the container.
    /// The root must exist and carry a marker; a missing
    /// root is `NotAGeectlBuildRoot`. The shell is scripted only: a
    /// command array, no TTY, a bounded timeout, stdout/stderr
    /// captured as data in the result — never attached, never
    /// unbounded. An empty command list is invalid input (no
    /// interactive fallback). The argv is the build's shape minus
    /// `--logfile`: `--rpmlist`/`--dist`/`--arch` are passed exactly
    /// as the existing tool passes them for a `--shell-cmd` run
    /// (they come from the root, which the build wrote). `--logfile`
    /// is dropped on purpose: passing it would overwrite the last
    /// build's log that `build-log` reads; the output rides in the
    /// result instead. The spec itself is not passed: it lives in
    /// the source dir, not the root, and the shell enters the
    /// preserved root whose rpmlist/dist carry the resolved inputs.
    // Seven arguments: the build-root coordinates plus the command
    // array and the timeout.
    #[allow(clippy::too_many_arguments)]
    pub fn build_shell(
        &self,
        project: &str,
        package: &str,
        repo: &str,
        arch: &str,
        flavor: &Option<String>,
        commands: &[String],
        timeout_secs: u64,
    ) -> Result<ShellResult, GeectlError> {
        validate_cache_components(project, repo, arch, flavor)?;
        validate_path_component("package", package)?;
        if commands.is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "build-shell takes a scripted command array; an empty list is not an interactive shell",
            ));
        }
        if timeout_secs == 0 {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "build-shell timeout must be greater than 0 seconds",
            ));
        }
        let root = self.root_path(project, package, repo, arch, flavor);
        let marker_path = root.join(ROOT_MARKER);
        if !marker_path.exists() {
            return Err(GeectlError::new(
                ErrorKind::NotAGeectlBuildRoot,
                format!(
                    "no geectl build root at {}; build the package first",
                    root.display()
                ),
            ));
        }
        // The shell needs the build's rpmlist and buildconfig: a
        // root whose build never got that far has no shell to open.
        let rpmlist_path = root.join(RPMLIST);
        let buildconfig_path = root.join(BUILDCONFIG);
        for path in [&rpmlist_path, &buildconfig_path] {
            if !path.exists() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "no {} in build root {}; build the package first",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        root.display()
                    ),
                ));
            }
        }
        let _lock = self.acquire_root_lock(&root, package)?;
        // The marker and the shell's inputs were checked before the
        // lock; re-check after it, mirroring `build_wipe`: an
        // eviction may have deleted the root in between (the lock
        // open recreates an empty directory), and a shell in a
        // markerless empty root would fail confusingly after
        // populating it.
        if !marker_path.exists() {
            return Err(GeectlError::new(
                ErrorKind::NotAGeectlBuildRoot,
                format!(
                    "no geectl build root at {}; build the package first",
                    root.display()
                ),
            ));
        }
        for path in [&rpmlist_path, &buildconfig_path] {
            if !path.exists() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "no {} in build root {}; build the package first",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        root.display()
                    ),
                ));
            }
        }
        let shell_cmd = commands.join(" && ");
        let argv = [
            self.engine.to_string_lossy().to_string(),
            format!("--root={}", root.join(".mount").display()),
            format!("--rpmlist={}", rpmlist_path.display()),
            format!("--dist={}", buildconfig_path.display()),
            format!("--arch={arch}"),
            "--vm-type=podman".to_string(),
            format!("--vm-disk={}", root.join("img").display()),
            format!("--vm-swap={}", root.join("swap").display()),
            "--shell-cmd".to_string(),
            shell_cmd,
        ];
        self.debug
            .emit(&format!("local-build shell: {}", argv.join(" ")));
        let (outcome, stdout, stderr) =
            run_engine(&argv, &root, Duration::from_secs(timeout_secs))?;
        if outcome.timed_out {
            // On unix the timeout kills the engine's process group;
            // on Windows only the direct child is killed.
            #[cfg(unix)]
            let killed = "the engine's process group was killed";
            #[cfg(not(unix))]
            let killed = "the engine process was killed; its children may still be running";
            return Err(GeectlError::new(
                ErrorKind::BuildTimeout,
                format!(
                    "shell in build root {} timed out after {timeout_secs}s; {killed}, but the container it started may still be running",
                    root.display()
                ),
            ));
        }
        Ok(ShellResult {
            exit_code: outcome.exit_code,
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    }

    /// Wipe a build root: the marker must be present and the root
    /// must not be locked by another build. Refuses with
    /// `NotAGeectlBuildRoot` when the marker is absent (never wipe
    /// an unknown directory) and with `BuildRootInUse` when another
    /// build holds the lock — a destructive op refused while the
    /// root is in use, not a second build (`BuildInProgress`). The
    /// lock file itself is created on demand by the lock attempt,
    /// so it is not a precondition.
    pub fn build_wipe(
        &self,
        project: &str,
        package: &str,
        repo: &str,
        arch: &str,
        flavor: &Option<String>,
    ) -> Result<(), GeectlError> {
        validate_cache_components(project, repo, arch, flavor)?;
        validate_path_component("package", package)?;
        let root = self.root_path(project, package, repo, arch, flavor);
        let expected = ExpectedMarker {
            project,
            package,
            repo,
            arch,
            flavor,
        };
        self.wipe_root_at(&root, package, Some(&expected), false)
            .map(|_| ())
    }

    /// Wipe the build root for the source in `source_dir`: the
    /// package comes from the spec file in it, like the build, and
    /// the root is the one the build's own coordinates name. No
    /// host mounts exist under a root (the engine's mounts live
    /// inside the container), so there is nothing to unmount.
    /// There is no force flag: the two guards a force could
    /// bypass (the marker check, the in-use lock) are both safety
    /// invariants.
    pub fn wipe_source_root(
        &self,
        source_dir: &Path,
        project: &str,
        repo: &str,
        arch: &str,
        flavor: &Option<String>,
        dry_run: bool,
    ) -> Result<WipeOutcome, GeectlError> {
        let source_dir = std::path::absolute(source_dir).map_err(|e| {
            GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "cannot resolve source directory {}: {e}",
                    source_dir.display()
                ),
            )
        })?;
        let spec = find_spec(&source_dir)?;
        let package = spec
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if !valid_obs_name(&package) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "spec file name {:?} is not a valid package name",
                    spec.file_name().unwrap_or_default().to_string_lossy()
                ),
            ));
        }
        // The package is a cache-dir component too (see above).
        validate_path_component("package", &package)?;
        let flavors = multibuild_flavors(&source_dir)?;
        if let Some(flavors) = &flavors {
            match flavor {
                None => {
                    return Err(GeectlError::new(
                        ErrorKind::MultibuildFlavorRequired,
                        format!(
                            "package {package} is multibuild; pass one flavor ({})",
                            flavors.join(", ")
                        ),
                    ));
                }
                Some(flavor) if !flavors.iter().any(|f| f == flavor) => {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "unknown multibuild flavor {flavor:?} for {package}; known flavors: {}",
                            flavors.join(", ")
                        ),
                    ));
                }
                _ => {}
            }
        }
        validate_cache_components(project, repo, arch, flavor)?;
        let root = self.root_path(project, &package, repo, arch, flavor);
        let expected = ExpectedMarker {
            project,
            package: &package,
            repo,
            arch,
            flavor,
        };
        self.wipe_root_at(&root, &package, Some(&expected), dry_run)
    }

    /// Wipe the build root at `root`, named directly: the marker must
    /// be present and parse (never wipe an unknown directory) and
    /// the root must not be locked by another build. The package
    /// for the lock's holder line comes from the root's own marker
    /// when it parses, else the root's own file name. A root with
    /// no file name (`/`) is refused outright.
    pub fn wipe_named_root(&self, root: &Path, dry_run: bool) -> Result<WipeOutcome, GeectlError> {
        let root = std::path::absolute(root).map_err(|e| {
            GeectlError::new(
                ErrorKind::InvalidInput,
                format!("cannot resolve build root {}: {e}", root.display()),
            )
        })?;
        let holder = fs::read_to_string(root.join(ROOT_MARKER))
            .ok()
            .and_then(|text| serde_json::from_str::<RootMarker>(&text).ok())
            .map(|marker| marker.package)
            .or_else(|| root.file_name().map(|n| n.to_string_lossy().to_string()))
            .ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("cannot wipe {}; a root needs a file name", root.display()),
                )
            })?;
        self.wipe_root_at(&root, &holder, None, dry_run)
    }

    /// Wipe the root at `root`, returning the outcome with the
    /// plan (entries, bytes). The root as named is reported in the
    /// outcome; the checks and the wipe itself run on the
    /// canonicalized path, so a symlinked root is judged by its
    /// target. A live wipe refuses a root without a valid marker
    /// (`NotAGeectlBuildRoot`); a dry run reports the absence
    /// instead and touches nothing. A marker that does not parse,
    /// or (for a source-derived root) does not match the
    /// coordinates the root was derived from, refuses in both
    /// modes: the directory is unverified, and the plan must not
    /// claim it would be wiped.
    ///
    /// The lock excludes cooperating geectl builds and wipes
    /// only. After the lock, the root's identity (device + inode,
    /// Unix) and the marker are re-verified: an eviction may have
    /// deleted the root in between (the lock open recreates an
    /// empty directory — see `acquire_root_lock`), and wiping that
    /// empty directory would report success for a root that is
    /// already gone; a replaced directory would wipe the wrong
    /// tree. The dry-run plan takes no lock, so it is advisory: a
    /// root held by a running build still plans, and the live wipe
    /// refuses it with `BuildRootInUse`.
    fn wipe_root_at(
        &self,
        root: &Path,
        holder: &str,
        expected: Option<&ExpectedMarker<'_>>,
        dry_run: bool,
    ) -> Result<WipeOutcome, GeectlError> {
        let absent = || WipeOutcome {
            root: root.to_path_buf(),
            existed: false,
            dry_run,
            entries: 0,
            bytes: 0,
        };
        let canonical = match fs::canonicalize(root) {
            Ok(path) => path,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if dry_run {
                    return Ok(absent());
                }
                return Err(GeectlError::new(
                    ErrorKind::NotAGeectlBuildRoot,
                    format!(
                        "no geectl build root at {}; refusing to wipe an unknown directory",
                        root.display()
                    ),
                ));
            }
            Err(e) => {
                return Err(GeectlError::new(
                    ErrorKind::InternalError,
                    format!("cannot resolve build root {}: {e}", root.display()),
                ));
            }
        };
        if !canonical.is_dir() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "{} is not a directory; refusing to wipe",
                    canonical.display()
                ),
            ));
        }
        if canonical.file_name().is_none() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "cannot wipe {}; a root needs a file name",
                    canonical.display()
                ),
            ));
        }
        let base = Self::canonicalize_or_absolute(&self.cache_dir.join("build-roots"));
        if canonical == base || !canonical.starts_with(&base) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "build root {} is outside the build-roots cache {}; refusing to wipe",
                    canonical.display(),
                    base.display()
                ),
            ));
        }
        // A build root is exactly one leaf below the cache:
        // `build-roots/<project>/<package>/<repo>-<arch>[-<flavor>]`.
        // A named root higher up (a project or package directory)
        // would wipe a whole subtree on one planted marker.
        let depth = canonical
            .strip_prefix(&base)
            .map(|rest| rest.components().count())
            .unwrap_or(0);
        if depth != 3 {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "build root {} is not a single build root under {}; refusing to wipe",
                    canonical.display(),
                    base.display()
                ),
            ));
        }
        if !Self::check_marker(&canonical, expected)? {
            if dry_run {
                return Ok(absent());
            }
            return Err(GeectlError::new(
                ErrorKind::NotAGeectlBuildRoot,
                format!(
                    "no geectl build root at {}; refusing to wipe an unknown directory",
                    canonical.display()
                ),
            ));
        }
        let plan = Self::inspect_wipe_plan(&canonical)?;
        if dry_run {
            return Ok(WipeOutcome {
                root: root.to_path_buf(),
                existed: true,
                dry_run,
                entries: plan.entries,
                bytes: plan.bytes,
            });
        }
        #[cfg(unix)]
        let identity = Self::root_identity(&canonical)?;
        let _lock = self.acquire_root_lock(&canonical, holder).map_err(|e| {
            if e.kind() == ErrorKind::BuildInProgress {
                GeectlError::new(
                    ErrorKind::BuildRootInUse,
                    format!("cannot wipe build root {}: {e}", canonical.display()),
                )
            } else {
                e
            }
        })?;
        #[cfg(unix)]
        {
            let now = Self::root_identity(&canonical)?;
            if now != identity {
                return Err(GeectlError::new(
                    ErrorKind::NotAGeectlBuildRoot,
                    format!(
                        "build root {} changed while locking; refusing to wipe",
                        canonical.display()
                    ),
                ));
            }
        }
        if !Self::check_marker(&canonical, expected)? {
            return Err(GeectlError::new(
                ErrorKind::NotAGeectlBuildRoot,
                format!(
                    "no geectl build root at {}; refusing to wipe an unknown directory",
                    canonical.display()
                ),
            ));
        }
        // Re-inspect after the lock: the pre-lock plan is stale
        // once the lock is held, and a mount created in between
        // must refuse the wipe rather than be deleted with it.
        let plan = Self::inspect_wipe_plan(&canonical)?;
        self.wipe_root_contents(&canonical)?;
        Ok(WipeOutcome {
            root: root.to_path_buf(),
            existed: true,
            dry_run,
            entries: plan.entries,
            bytes: plan.bytes,
        })
    }

    /// Read a build log: the last `tail` lines, or the whole log when
    /// `tail` is None. Both arms read at most the last 4 MiB (see
    /// `read_log_tail`) unless `uncapped` is set. A larger log under
    /// the default read returns its last 4 MiB of whole lines with
    /// `truncated` set; an uncapped read returns the whole log in
    /// the whole-log arm — or the last `tail` lines of the whole
    /// log in the tail arm — with `truncated` unset.
    /// The record's verdict rides along when a record exists;
    /// a missing log is `InvalidInput` (the root may exist but has
    /// never built, so there is no log to read).
    #[allow(clippy::too_many_arguments)]
    pub fn build_log(
        &self,
        project: &str,
        package: &str,
        repo: &str,
        arch: &str,
        flavor: &Option<String>,
        tail: Option<usize>,
        uncapped: bool,
    ) -> Result<BuildLog, GeectlError> {
        validate_cache_components(project, repo, arch, flavor)?;
        validate_path_component("package", package)?;
        let root = self.root_path(project, package, repo, arch, flavor);
        let log_path = root.join(BUILD_LOG);
        if !log_path.exists() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "no build log at {}; build the package first",
                    log_path.display()
                ),
            ));
        }
        let truncated = !uncapped
            && fs::metadata(&log_path)
                .map(|m| m.len() > LOG_TAIL_CAP_BYTES)
                .unwrap_or(false);
        let text = match tail {
            Some(_) => read_log_tail(&log_path, uncapped),
            None => {
                // `read_log_tail` reads an unreadable log as empty;
                // the whole-log arm keeps the old contract and
                // surfaces a read failure instead.
                File::open(&log_path).map_err(|e| {
                    GeectlError::new(
                        ErrorKind::InternalError,
                        format!("cannot read build log {}: {e}", log_path.display()),
                    )
                })?;
                read_log_tail(&log_path, uncapped)
            }
        };
        let lines: Vec<&str> = text.lines().collect();
        let shown: Vec<String> = match tail {
            Some(n) => lines
                .iter()
                .skip(lines.len().saturating_sub(n))
                .map(|l| l.to_string())
                .collect(),
            None => lines.iter().map(|l| l.to_string()).collect(),
        };
        let record = fs::read_to_string(root.join(BUILD_RECORD))
            .ok()
            .and_then(|text| serde_json::from_str::<BuildRecord>(&text).ok());
        Ok(BuildLog {
            log_path: log_path.to_path_buf(),
            verdict: record.as_ref().map(|r| r.verdict.clone()),
            exit_code: record.as_ref().map(|r| r.exit_code),
            lines: shown,
            truncated,
        })
    }
}

/// Service runs and the precommit gate.
///
/// A service run executes the `_service` file's binaries in the
/// vertical's container, not on the host: the container fixes the
/// host divergence (a macOS or Windows host cannot run the Linux
/// service binaries). The binaries come from the distribution, not
/// from the image: the plain image carries no `obs-service-*`
/// package, and the vertical never builds an image — each run
/// installs the selected services' packages inside one ephemeral
/// container, then runs the binaries there. The mode filter, the
/// single-service forcing, the outdir handling, and the output
/// placement mirror the reference implementation's
/// `Serviceinfo._execute` (`osc/obs_scm/serviceinfo.py`) and the
/// command-to-mode mapping of its `do_service`
/// (`osc/commandline.py`).
impl<'a> LocalBuild<'a> {
    /// Run the services of the `_service` file in `req.source_dir`
    /// in the container, filtered by `req.mode`. One container for
    /// the whole run: the package install and every service share
    /// it, and the run's timeout budget. The first failing service
    /// aborts the run with [`ErrorKind::ServiceFailed`]; a run
    /// that selects no services starts no container and records an
    /// empty `ran` list, and stale service files are still removed.
    /// A dry run returns the plan and touches nothing.
    pub fn service_run(&self, req: &ServiceRunRequest) -> Result<ServiceRunRecord, GeectlError> {
        if req.timeout_secs == 0 {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "service run timeout must be greater than 0 seconds",
            ));
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(req.timeout_secs);
        self.service_run_with_deadline(req, deadline)
    }

    /// The precommit gate: the `precommit`-mode service run, then
    /// `pre_checkin.sh` when the source directory carries one.
    /// The script step is part of the gate, not of the service
    /// mode: the mode filter alone runs services only. Both steps
    /// share one timeout budget. A failing service or a failing
    /// `pre_checkin.sh` fails the whole run with
    /// [`ErrorKind::ServiceFailed`]. A dry run returns the plan
    /// and touches nothing.
    pub fn precommit(
        &self,
        source_dir: &Path,
        timeout_secs: u64,
        dry_run: bool,
    ) -> Result<PrecommitRecord, GeectlError> {
        if timeout_secs == 0 {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "precommit timeout must be greater than 0 seconds",
            ));
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
        self.precommit_with_deadline(source_dir, timeout_secs, deadline, dry_run)
    }

    /// The precommit gate against a fixed deadline: the caller
    /// owns the deadline - the `build` leaf passes one deadline
    /// for the service run, the precommit gate, and the engine
    /// build, so a slow service run leaves less time for the
    /// script instead of the gate taking a fresh full timeout.
    /// `timeout_secs` is the whole-run budget the
    /// exhausted-budget message names.
    pub fn precommit_with_deadline(
        &self,
        source_dir: &Path,
        timeout_secs: u64,
        deadline: std::time::Instant,
        dry_run: bool,
    ) -> Result<PrecommitRecord, GeectlError> {
        if timeout_secs == 0 {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "precommit timeout must be greater than 0 seconds",
            ));
        }
        let req = ServiceRunRequest {
            source_dir: source_dir.to_path_buf(),
            mode: ServiceRunMode::Precommit,
            service: None,
            timeout_secs,
            dry_run,
        };
        let services = self.service_run_with_deadline(&req, deadline)?;
        let source_dir = std::path::absolute(&req.source_dir).map_err(|e| {
            GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "cannot resolve source directory {}: {e}",
                    req.source_dir.display()
                ),
            )
        })?;
        let pre_checkin = self.run_precheckin(&source_dir, deadline, dry_run)?;
        Ok(PrecommitRecord {
            services,
            pre_checkin,
            dry_run,
        })
    }

    /// Run `spec-cleaner` on `spec` in the container. `spec` is a
    /// path inside `source_dir` (relative, or absolute under it).
    /// `extra_args` pass through to the binary as argv elements.
    /// A non-zero exit fails with [`ErrorKind::ServiceFailed`].
    /// A dry run returns the plan and touches nothing.
    pub fn spec_cleaner(
        &self,
        source_dir: &Path,
        spec: &Path,
        extra_args: &[String],
        timeout_secs: u64,
        dry_run: bool,
    ) -> Result<SpecCleanerResult, GeectlError> {
        if timeout_secs == 0 {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "spec-cleaner timeout must be greater than 0 seconds",
            ));
        }
        let source_dir = std::path::absolute(source_dir).map_err(|e| {
            GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "cannot resolve source directory {}: {e}",
                    source_dir.display()
                ),
            )
        })?;
        if !source_dir.is_dir() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "source directory {} is not a directory",
                    source_dir.display()
                ),
            ));
        }
        let spec_path = if spec.is_absolute() {
            spec.to_path_buf()
        } else {
            source_dir.join(spec)
        };
        let spec_name = spec_path.strip_prefix(&source_dir).map_err(|_| {
            GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "spec {} is outside source directory {}",
                    spec_path.display(),
                    source_dir.display()
                ),
            )
        })?;
        // A lexically joined `..` passes the prefix check above:
        // reject it here, so an escaping spec fails here instead
        // of confusingly in the container.
        if spec_name
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "spec {} is outside source directory {}",
                    spec_path.display(),
                    source_dir.display()
                ),
            ));
        }
        if spec_name.as_os_str().is_empty() || !spec_path.is_file() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("spec {} is not a file", spec_path.display()),
            ));
        }
        let mut argv: Vec<String> = vec!["spec-cleaner".to_string()];
        argv.extend(extra_args.iter().cloned());
        argv.push(spec_name.to_string_lossy().into_owned());
        if dry_run {
            return Ok(SpecCleanerResult {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
                image_digest: None,
                dry_run: true,
                plan: Some(argv),
            });
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
        ServiceContainer::check_mount_source(&source_dir)?;
        let image = self.resolve_image(false, timeout_secs, deadline)?;
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let container =
            ServiceContainer::start(self, &source_dir, DEFAULT_IMAGE, remaining.as_secs())?;
        let install = container.exec(
            &service_install_argv(&["spec-cleaner".to_string()]),
            deadline,
        )?;
        if !install.status.success() {
            return Err(GeectlError::new(
                ErrorKind::ServiceFailed,
                format!(
                    "cannot install spec-cleaner in the service container: {}",
                    last_lines(&String::from_utf8_lossy(&install.stderr), 20).join("\n")
                ),
            ));
        }
        let output = container.exec(&argv, deadline)?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if !output.status.success() {
            return Err(GeectlError::new(
                ErrorKind::ServiceFailed,
                format!(
                    "spec-cleaner failed with exit {}: {}",
                    output.status.code().unwrap_or(-1),
                    last_lines(&stderr, 20).join("\n")
                ),
            ));
        }
        Ok(SpecCleanerResult {
            exit_code: output.status.code().unwrap_or(-1),
            stdout,
            stderr,
            image_digest: image.digest,
            dry_run: false,
            plan: None,
        })
    }

    /// The service run against a fixed deadline: the caller owns
    /// the deadline - `precommit` passes its own deadline so the
    /// services and `pre_checkin.sh` share one budget, and the
    /// `build` leaf passes one deadline for the service run, the
    /// precommit gate, and the engine build.
    pub fn service_run_with_deadline(
        &self,
        req: &ServiceRunRequest,
        deadline: std::time::Instant,
    ) -> Result<ServiceRunRecord, GeectlError> {
        if req.timeout_secs == 0 {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "service run timeout must be greater than 0 seconds",
            ));
        }
        let source_dir = std::path::absolute(&req.source_dir).map_err(|e| {
            GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "cannot resolve source directory {}: {e}",
                    req.source_dir.display()
                ),
            )
        })?;
        if !source_dir.is_dir() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "source directory {} is not a directory",
                    source_dir.display()
                ),
            ));
        }
        let definitions = read_local_services(&source_dir)?;
        let selected = select_services(&definitions, req.mode, req.service.as_deref())?;
        let outdirs: Vec<PathBuf> = selected
            .iter()
            .enumerate()
            .map(|(index, service)| service_outdir(&source_dir, &service.name, index))
            .collect();
        if req.dry_run {
            let plan = ServiceRunPlan {
                image: DEFAULT_IMAGE.to_string(),
                services: selected
                    .iter()
                    .enumerate()
                    .map(|(index, service)| PlannedService {
                        name: service.name.clone(),
                        mode: service.effective_mode.clone(),
                        argv: service_argv(
                            service,
                            &service_outdir_container(&service.name, index),
                        ),
                    })
                    .collect(),
            };
            return Ok(ServiceRunRecord {
                mode: req.mode.command_name().to_string(),
                ran: Vec::new(),
                image_digest: None,
                dry_run: true,
                plan: Some(plan),
            });
        }
        // Stale generated files move aside before the run and are
        // deleted after it, whatever the outcome: a service that
        // no longer generates a file must not leave a stale
        // `_service:` file behind. The stash runs even when the
        // selection is empty, so an empty run still clears stale
        // files.
        let stash = stash_service_files(&source_dir)?;
        if selected.is_empty() {
            self.debug.emit(&format!(
                "service-run {}: no services selected",
                req.mode.command_name()
            ));
            remove_stash(&stash, &self.debug);
            return Ok(ServiceRunRecord {
                mode: req.mode.command_name().to_string(),
                ran: Vec::new(),
                image_digest: None,
                dry_run: false,
                plan: None,
            });
        }
        ServiceContainer::check_mount_source(&source_dir).inspect_err(|_| {
            remove_stash(&stash, &self.debug);
        })?;
        let image = self
            .resolve_image(false, req.timeout_secs, deadline)
            .inspect_err(|_| {
                remove_stash(&stash, &self.debug);
            })?;
        let result =
            self.service_run_inner(req, &source_dir, &selected, &outdirs, &image, deadline);
        remove_stash(&stash, &self.debug);
        result
    }

    /// The service run body: the container, the package install,
    /// and one binary run per selected service, in file order.
    /// The stash cleanup lives in the caller, so it runs whatever
    /// this returns.
    fn service_run_inner(
        &self,
        req: &ServiceRunRequest,
        source_dir: &Path,
        selected: &[SelectedService],
        outdirs: &[PathBuf],
        image: &ImageInfo,
        deadline: std::time::Instant,
    ) -> Result<ServiceRunRecord, GeectlError> {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let container =
            ServiceContainer::start(self, source_dir, DEFAULT_IMAGE, remaining.as_secs())?;
        let packages = service_packages(selected);
        let install_argv = service_install_argv(&packages);
        let install = container.exec(&install_argv, deadline)?;
        if !install.status.success() {
            return Err(GeectlError::new(
                ErrorKind::ServiceFailed,
                format!(
                    "cannot install {} in the service container: {}",
                    packages.join(", "),
                    last_lines(&String::from_utf8_lossy(&install.stderr), 20).join("\n")
                ),
            ));
        }
        let mut ran = Vec::new();
        for (index, (service, outdir)) in selected.iter().zip(outdirs.iter()).enumerate() {
            fs::create_dir(outdir).map_err(|e| {
                GeectlError::new(
                    ErrorKind::InternalError,
                    format!("cannot create service outdir {}: {e}", outdir.display()),
                )
            })?;
            let argv = service_argv(service, &service_outdir_container(&service.name, index));
            self.debug.emit(&format!(
                "service-run {}: {}",
                req.mode.command_name(),
                argv.join(" ")
            ));
            let output = container.exec(&argv, deadline)?;
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            let exit_code = output.status.code().unwrap_or(-1);
            if !output.status.success() {
                let _ = fs::remove_dir_all(outdir);
                return Err(GeectlError::new(
                    ErrorKind::ServiceFailed,
                    format!(
                        "service {} failed with exit {exit_code}: {}",
                        service.name,
                        last_lines(&stderr, 20).join("\n")
                    ),
                ));
            }
            place_outputs(
                outdir,
                source_dir,
                &service.name,
                service_plain_output(&service.effective_mode, req.mode.callmode()),
            )?;
            let _ = fs::remove_dir(outdir);
            ran.push(ServiceRunOutcome {
                name: service.name.clone(),
                exit_code,
                stdout,
                stderr,
            });
        }
        Ok(ServiceRunRecord {
            mode: req.mode.command_name().to_string(),
            ran,
            image_digest: image.digest.clone(),
            dry_run: false,
            plan: None,
        })
    }

    /// Run `pre_checkin.sh` in the container when the source
    /// directory carries one. A missing script is not an error:
    /// most packages carry none. A failing script fails with
    /// [`ErrorKind::ServiceFailed`]. The script runs under `bash`
    /// with the source directory as its working directory, in the
    /// bare base image: no packages are installed for it, so a
    /// script needing tools beyond the base image fails. A script
    /// that shells out to `osc` cannot work here (no osc working
    /// copy, no osc in the container).
    fn run_precheckin(
        &self,
        source_dir: &Path,
        deadline: std::time::Instant,
        dry_run: bool,
    ) -> Result<PrecheckinOutcome, GeectlError> {
        let script = source_dir.join("pre_checkin.sh");
        if !script.is_file() {
            return Ok(PrecheckinOutcome {
                ran: false,
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
            });
        }
        if dry_run {
            return Ok(PrecheckinOutcome {
                ran: false,
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
            });
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.as_secs() == 0 {
            return Err(GeectlError::new(
                ErrorKind::BuildTimeout,
                "precommit budget exhausted before pre_checkin.sh; the services ran, the script did not",
            ));
        }
        ServiceContainer::check_mount_source(source_dir)?;
        let image = self.resolve_image(false, remaining.as_secs(), deadline)?;
        self.debug.emit(&format!(
            "precommit image: {} digest {}",
            DEFAULT_IMAGE,
            image.digest.as_deref().unwrap_or("<unknown>")
        ));
        let container =
            ServiceContainer::start(self, source_dir, DEFAULT_IMAGE, remaining.as_secs())?;
        let output = container.exec(
            &["bash".to_string(), "./pre_checkin.sh".to_string()],
            deadline,
        )?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let exit_code = output.status.code().unwrap_or(-1);
        if !output.status.success() {
            return Err(GeectlError::new(
                ErrorKind::ServiceFailed,
                format!(
                    "pre_checkin.sh failed with exit {exit_code}: {}",
                    last_lines(&stderr, 20).join("\n")
                ),
            ));
        }
        Ok(PrecheckinOutcome {
            ran: true,
            exit_code: Some(exit_code),
            stdout,
            stderr,
        })
    }
}

/// Capabilities the service container keeps: `DAC_OVERRIDE` for
/// the cpio writes of the package install, `SETGID`/`SETUID` for
/// the `setpriv` elevation of the install exec. Every other
/// capability is dropped.
const SERVICE_CONTAINER_CAPS: &str = "DAC_OVERRIDE,SETGID,SETUID";

/// Memory cap for the service container, for RAM and for RAM
/// plus swap together.
const SERVICE_CONTAINER_MEMORY: &str = "4g";

/// Process cap for the service container.
const SERVICE_CONTAINER_PIDS_LIMIT: &str = "1024";

/// One ephemeral container for a service run, a precommit, or a
/// `spec-cleaner` invocation: started detached on `sleep` so the
/// steps run as `podman exec` calls in the same container, stopped
/// on drop. The `sleep` duration bounds a container orphaned by a
/// killed client: it exits on its own when the budget lapses. The
/// container runs as the invoking user (`--userns=keep-id` maps
/// that user into the container, so the bind-mounted source
/// directory is owned by the container user and service writes
/// land there). Only the package install elevates, via
/// `setpriv --reuid=0 --regid=0 --clear-groups` under the
/// granted capabilities, to the container's root user,
/// unprivileged on the host. Capabilities are dropped except the
/// install set,
/// new privileges are denied, and memory and process counts are
/// capped; the network stays open — `download_url` fetches over
/// it. A timed-out `exec` kills the container: the exec'd process
/// ends with it instead of running on to the sleep budget. The
/// container carries the runner version as `OSC_VERSION` (the
/// value is the geectl version); the `OBS_SERVICE_*` variables
/// stay unset — a local run carries no project context. No CPU cap
/// is set: the wall-clock timeout already bounds CPU time, and a
/// fixed CPU count does not scale portably across hosts.
/// A `,` in the source path is refused: the `--mount` form
/// splits on `,` with no escape, so the mount cannot carry it.
struct ServiceContainer {
    podman: PathBuf,
    name: String,
}

static SERVICE_CONTAINER_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The install exec argv for the service container: `zypper`
/// under a scoped `setpriv` elevation to the container's root
/// user. The container runs as the invoking user, and `zypper`
/// refuses a non-root install, so only this exec elevates.
fn service_install_argv(packages: &[String]) -> Vec<String> {
    let mut argv = vec![
        "setpriv".to_string(),
        "--reuid=0".to_string(),
        "--regid=0".to_string(),
        "--clear-groups".to_string(),
        "zypper".to_string(),
        "--non-interactive".to_string(),
        "install".to_string(),
        "--no-recommends".to_string(),
    ];
    argv.extend(packages.iter().cloned());
    argv
}

impl ServiceContainer {
    /// A source directory the `--mount` form can carry: it holds
    /// no `,`, which splits the comma-separated value with no
    /// escape — podman refuses the fragment as an invalid mount
    /// option. Refusing here names the path instead of letting
    /// the container fail to start.
    fn check_mount_source(source_dir: &Path) -> Result<(), GeectlError> {
        if source_dir.to_string_lossy().contains(',') {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "source directory {} contains a ','; the --mount form cannot carry a comma in the path",
                    source_dir.display()
                ),
            ));
        }
        Ok(())
    }

    fn start(
        vertical: &LocalBuild<'_>,
        source_dir: &Path,
        image: &str,
        timeout_secs: u64,
    ) -> Result<Self, GeectlError> {
        // Checked at the entry points before any image work; kept
        // here so a future caller cannot build the mount unchecked.
        Self::check_mount_source(source_dir)?;
        let podman = which(&vertical.podman).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ImageUnavailable,
                format!(
                    "podman {:?} not found on PATH; install podman to run local builds",
                    vertical.podman.to_string_lossy()
                ),
            )
        })?;
        let n = SERVICE_CONTAINER_COUNTER.fetch_add(1, Ordering::SeqCst);
        let name = format!("geectl-service-{}-{n}", std::process::id());
        let osc_version = format!("OSC_VERSION={}", env!("CARGO_PKG_VERSION"));
        let mut cmd = Command::new(&podman);
        cmd.args([
            "run",
            "-d",
            "--rm",
            "--name",
            &name,
            "--userns",
            "keep-id",
            "--cap-drop",
            "ALL",
            "--cap-add",
            SERVICE_CONTAINER_CAPS,
            "--security-opt",
            "no-new-privileges",
            "--memory",
            SERVICE_CONTAINER_MEMORY,
            "--memory-swap",
            SERVICE_CONTAINER_MEMORY,
            "--pids-limit",
            SERVICE_CONTAINER_PIDS_LIMIT,
            "-e",
            &osc_version,
            "--mount",
            &format!("type=bind,src={},target=/work", source_dir.display()),
            "-w",
            "/work",
            image,
            "sleep",
            &timeout_secs.to_string(),
        ]);
        let output =
            run_with_timeout(&mut cmd, Duration::from_secs(PROBE_TIMEOUT_SECS)).map_err(|e| {
                GeectlError::new(
                    ErrorKind::ImageUnavailable,
                    format!("cannot start the service container: {e}"),
                )
            })?;
        if !output.status.success() {
            return Err(GeectlError::new(
                ErrorKind::ImageUnavailable,
                format!(
                    "cannot start the service container: {}",
                    last_lines(&String::from_utf8_lossy(&output.stderr), 20).join("\n")
                ),
            ));
        }
        if String::from_utf8_lossy(&output.stdout).trim().is_empty() {
            return Err(GeectlError::new(
                ErrorKind::ImageUnavailable,
                "podman run started no container: empty container id",
            ));
        }
        Ok(ServiceContainer { podman, name })
    }

    /// Run `argv` in the container, bounded by the deadline. The
    /// argv form carries no shell: `_service` parameter values
    /// reach the binary as single arguments, whatever they contain.
    /// Every call sets the working directory explicitly instead of
    /// relying on the create-time value.
    fn exec(
        &self,
        argv: &[String],
        deadline: std::time::Instant,
    ) -> Result<std::process::Output, GeectlError> {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(GeectlError::new(
                ErrorKind::BuildTimeout,
                "service container budget exhausted before the command started",
            ));
        }
        let budget_secs = remaining.as_secs();
        let mut cmd = Command::new(&self.podman);
        cmd.arg("exec")
            .arg("-w")
            .arg("/work")
            .arg(&self.name)
            .args(argv);
        run_with_timeout(&mut cmd, remaining).map_err(|e| {
            if e.kind() == std::io::ErrorKind::TimedOut {
                self.kill();
                GeectlError::new(
                    ErrorKind::BuildTimeout,
                    format!(
                        "service container command timed out after {budget_secs}s: {}",
                        argv.first().map(String::as_str).unwrap_or("")
                    ),
                )
            } else {
                GeectlError::new(
                    ErrorKind::InternalError,
                    format!("cannot run a command in the service container: {e}"),
                )
            }
        })
    }

    fn stop(&self) {
        let mut cmd = Command::new(&self.podman);
        cmd.args(["stop", "--time", "10", &self.name]);
        let _ = run_with_timeout(&mut cmd, Duration::from_secs(PROBE_TIMEOUT_SECS));
    }

    /// Kill the container at once: the run has already failed,
    /// so nothing in the container has anything left to finish.
    /// SIGKILL to the container's init ends the pid namespace,
    /// which ends the exec'd process with it.
    fn kill(&self) {
        let mut cmd = Command::new(&self.podman);
        cmd.args(["kill", &self.name]);
        let _ = run_with_timeout(&mut cmd, Duration::from_secs(KILL_TIMEOUT_SECS));
    }
}

impl Drop for ServiceContainer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A service selected for a run: the file's definition with its
/// mode replaced by the run mode's callmode when the reference
/// implementation forces it (a single-service run, or a mode whose
/// callmode a service's stored mode cannot override).
#[derive(Debug)]
struct SelectedService {
    name: String,
    effective_mode: String,
    params: Vec<ServiceParam>,
}

/// Read the `_service` file of a source directory. A missing file
/// selects nothing; a malformed file is a caller error.
fn read_local_services(source_dir: &Path) -> Result<Vec<ServiceDef>, GeectlError> {
    let path = source_dir.join("_service");
    let body = match fs::read_to_string(&path) {
        Ok(body) => body,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("_service in {} is not valid UTF-8", source_dir.display()),
            ));
        }
        Err(e) => {
            return Err(GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot read _service in {}: {e}", source_dir.display()),
            ));
        }
    };
    parse_services(&body).map_err(|e| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            format!(
                "malformed _service in {}: {}",
                source_dir.display(),
                e.context
            ),
        )
    })
}

/// Select the services a run executes, in file order. A
/// single-service run executes only that service: when the file
/// defines it, with its parameters and the run mode forced as its
/// mode; when it does not, with no parameters and the run mode as
/// its mode. Any other run filters the file's services by the run
/// mode.
fn select_services(
    definitions: &[ServiceDef],
    mode: ServiceRunMode,
    single: Option<&str>,
) -> Result<Vec<SelectedService>, GeectlError> {
    let callmode = mode.callmode();
    if let Some(name) = single {
        if name.chars().count() < 3 || name.contains('/') {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid service name {name:?}: at least 3 characters, no '/'",),
            ));
        }
        let effective_mode = callmode.unwrap_or_default().to_string();
        if let Some(def) = definitions.iter().find(|d| d.name == name) {
            return Ok(vec![SelectedService {
                name: def.name.clone(),
                effective_mode,
                params: def.params.clone(),
            }]);
        }
        return Ok(vec![SelectedService {
            name: name.to_string(),
            effective_mode,
            params: Vec::new(),
        }]);
    }
    Ok(definitions
        .iter()
        .filter(|def| service_selected(&def.mode, callmode))
        .map(|def| SelectedService {
            name: def.name.clone(),
            effective_mode: def.mode.clone(),
            params: def.params.clone(),
        })
        .collect())
}

/// The mode filter, mirroring the reference implementation's
/// skip chain (`Serviceinfo._execute`,
/// `osc/obs_scm/serviceinfo.py`): `buildtime` never runs outside
/// a `runall` run; `serveronly` runs only under `localrun`;
/// `manual` and `disabled` run only under their own mode; under
/// `precommit` only `trylocal` and `localonly` run (a service with
/// no stored mode does not run there). Any other stored mode runs
/// under `run` and `localrun`.
fn service_selected(mode: &str, callmode: Option<&str>) -> bool {
    // `all` is runall's callmode: no constructed mode yields it,
    // since runall is refused. The arm keeps the mirrored filter
    // complete, and the exhaustive test pins it.
    if callmode == Some("all") {
        return true;
    }
    if mode == "buildtime" {
        return false;
    }
    if mode == "serveronly" && callmode != Some("local") {
        return false;
    }
    if mode == "manual" && callmode != Some("manual") {
        return false;
    }
    if mode != "manual" && callmode == Some("manual") {
        return false;
    }
    if mode == "disabled" && callmode != Some("disabled") {
        return false;
    }
    if mode != "disabled" && callmode == Some("disabled") {
        return false;
    }
    if mode != "trylocal" && mode != "localonly" && callmode == Some("trylocal") {
        return false;
    }
    true
}

/// Whether a service's outputs land as plain filenames in the
/// source directory, rather than prefixed `_service:<name>:`:
/// plain when the service's mode is `manual`, `disabled`,
/// `trylocal`, or `localonly`, or when the run mode is `local`,
/// `trylocal`, or `all`. The reference implementation's placement
/// condition, verbatim. The `all` callmode is runall's: no
/// constructed mode yields it, since runall is refused; the arm
/// keeps the mirrored rule complete, and the exhaustive test pins
/// it.
fn service_plain_output(mode: &str, callmode: Option<&str>) -> bool {
    matches!(mode, "manual" | "disabled" | "trylocal" | "localonly")
        || matches!(callmode, Some("local") | Some("trylocal") | Some("all"))
}

/// The binary argv for one service: the distribution binary path,
/// one `--<param> <value>` pair per parameter, and the `--outdir`.
/// Argv elements, never a shell string: parameter values reach the
/// binary verbatim.
fn service_argv(service: &SelectedService, outdir: &str) -> Vec<String> {
    let mut argv = vec![format!("/usr/lib/obs/service/{}", service.name)];
    for param in &service.params {
        argv.push(format!("--{}", param.name));
        argv.push(param.value.clone());
    }
    argv.push("--outdir".to_string());
    argv.push(outdir.to_string());
    argv
}

/// The distribution packages a run installs in its container: one
/// `obs-service-<name>` per selected service, plus `spec-cleaner`
/// when `format_spec_file` runs and `cpio` when `obs_scm` runs
/// (each binary shells out to it). Deduplicated and sorted, so
/// the install argv is stable.
fn service_packages(selected: &[SelectedService]) -> Vec<String> {
    let mut packages: BTreeSet<String> = selected
        .iter()
        .map(|s| format!("obs-service-{}", s.name))
        .collect();
    if selected.iter().any(|s| s.name == "format_spec_file") {
        packages.insert("spec-cleaner".to_string());
    }
    if selected.iter().any(|s| s.name == "obs_scm") {
        packages.insert("cpio".to_string());
    }
    packages.into_iter().collect()
}

/// The host-side temporary directory for one service's outputs:
/// a dot-directory in the source directory, named per service,
/// process, and position in the run. The position keeps the name
/// stable between the dry-run plan and the real run within one
/// process: the process id in the name differs across processes.
fn service_outdir(source_dir: &Path, service: &str, index: usize) -> PathBuf {
    source_dir.join(service_outdir_name(service, index))
}

/// The same directory, as the container sees it (`/work` is the
/// source directory's mount point).
fn service_outdir_container(service: &str, index: usize) -> String {
    format!("/work/{}", service_outdir_name(service, index))
}

fn service_outdir_name(service: &str, index: usize) -> String {
    format!(".geectl-service-{service}-{}-{index}", std::process::id())
}

/// Move the `_service:*` and `_service_*` files aside into
/// `.geectl-service-old` before a run, so a service that no longer
/// generates a file leaves no stale output behind. A pre-existing
/// stash directory fails the run: it is the previous run's leftover
/// and the run never removes it — remove it before running services
/// again. The run waits briefly for a concurrent run to finish
/// removing it before failing.
fn stash_service_files(source_dir: &Path) -> Result<Option<PathBuf>, GeectlError> {
    // A pre-existing stash fails the run before anything moves:
    // it is the previous run's leftover, and the run leaves it in
    // place — remove it before running services again. Wait
    // briefly first: a concurrent run may be removing it.
    let stash = source_dir.join(".geectl-service-old");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while stash.exists() {
        if std::time::Instant::now() >= deadline {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "stale service stash {} exists; remove it before running services",
                    stash.display()
                ),
            ));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let mut stale = Vec::new();
    for entry in fs::read_dir(source_dir).map_err(|e| {
        GeectlError::new(
            ErrorKind::InternalError,
            format!("cannot list source directory {}: {e}", source_dir.display()),
        )
    })? {
        let entry = entry.map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot list source directory {}: {e}", source_dir.display()),
            )
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("_service:") || name.starts_with("_service_") {
            stale.push(entry.path());
        }
    }
    if stale.is_empty() {
        return Ok(None);
    }
    fs::create_dir(&stash).map_err(|e| {
        GeectlError::new(
            ErrorKind::InternalError,
            format!("cannot create service stash {}: {e}", stash.display()),
        )
    })?;
    for path in stale {
        let name = path.file_name().unwrap_or_default().to_os_string();
        fs::rename(&path, stash.join(name)).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot stash service file {}: {e}", path.display()),
            )
        })?;
    }
    Ok(Some(stash))
}

/// Delete a stash directory after a run, whatever the run's
/// outcome. A failure to delete is a debug note, never a run
/// failure: the outputs are already placed.
fn remove_stash(stash: &Option<PathBuf>, debug: &Debug) {
    if let Some(stash) = stash {
        if let Err(e) = fs::remove_dir_all(stash) {
            debug.emit(&format!(
                "service-run: cannot remove service stash {}: {e}",
                stash.display()
            ));
        }
    }
}

/// Move a service's outputs from its outdir into the source
/// directory: plain filenames, or prefixed `_service:<name>:`.
/// The outdir itself is removed by the caller.
fn place_outputs(
    outdir: &Path,
    source_dir: &Path,
    service: &str,
    plain: bool,
) -> Result<(), GeectlError> {
    for entry in fs::read_dir(outdir).map_err(|e| {
        GeectlError::new(
            ErrorKind::InternalError,
            format!("cannot list service outdir {}: {e}", outdir.display()),
        )
    })? {
        let entry = entry.map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot list service outdir {}: {e}", outdir.display()),
            )
        })?;
        let name = entry.file_name();
        let dest = if plain {
            source_dir.join(&name)
        } else {
            source_dir.join(format!("_service:{service}:{}", name.to_string_lossy()))
        };
        fs::rename(entry.path(), &dest).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot place service output {}: {e}", dest.display()),
            )
        })?;
    }
    Ok(())
}

/// The size of a directory tree in bytes, best-effort: unreadable
/// entries count as 0 rather than failing the whole usage read.
fn dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack: Vec<PathBuf> = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => stack.push(path),
                Ok(_) => {
                    if let Ok(meta) = fs::metadata(&path) {
                        total += meta.len();
                    }
                }
                Err(_) => {}
            }
        }
    }
    total
}

/// Collect sccache stores under `dir` (project/package/root
/// layout): each store is a directory with a size and a last-use
/// mtime. The current build's store is skipped by the caller (its
/// mtime was just touched); stores in the 24h grace period never
/// evict, and their sizes are added to `skipped` (protected bytes).
fn collect_sccache_stores(
    dir: &Path,
    out: &mut Vec<(PathBuf, u64, u64)>,
    now: u64,
    skipped: &mut u64,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // A store is a project dir (packages inside), a package
        // dir (root names inside), or a root-name dir (the
        // sccache.tar inside): walk one level deeper when the dir
        // carries no tar.
        if path.join("sccache.tar").exists() {
            let size = dir_size(&path);
            let mtime = fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if now.saturating_sub(mtime) >= GRACE_PERIOD_SECS {
                out.push((path, size, mtime));
            } else {
                *skipped += size;
            }
        } else {
            collect_sccache_stores(&path, out, now, skipped);
        }
    }
}

/// A build root as an eviction candidate: path, last-build time,
/// and the project/package it belongs to (from the marker, not the
/// path — the path layout is an implementation detail). The
/// repo/arch/flavor are not carried: the most-recent protection
/// groups by (project, package), so they are not read. Whether the
/// last build succeeded is not carried: the most-recent protection
/// applies whatever the outcome was.
struct RootEntry {
    path: PathBuf,
    last_build_unix: u64,
    project: String,
    package: String,
}

/// Whether a build root's lock is held by a running build: a
/// non-blocking `flock` probe. A missing lock file reads as
/// unlocked — the root has no build in it.
fn root_lock_held(root: &Path) -> bool {
    let Ok(lock_file) = File::options()
        .read(true)
        .write(true)
        .open(root.join(ROOT_LOCK))
    else {
        return false;
    };
    // The probe lock releases on drop; the probe itself is the check.
    lock_file.try_lock_exclusive().is_err()
}

/// Hold a build root's flock (non-blocking) across a delete.
/// `Ok(Some)` holds the lock; `Ok(None)` means there is no lock
/// file to hold (the root was never locked — nothing to guard);
/// `Err(())` means another build holds it, so the caller must not
/// delete. The eviction apply arm uses this instead of a probe: a
/// probe taken before the delete is stale by the time the delete
/// runs, and a build starting in between would lose its root (or
/// its sccache store) under it. Holding the lock across the delete
/// is the guard — the lock is the thing a build takes before it
/// touches either.
fn try_hold_root_lock(root: &Path) -> Result<Option<BuildLock>, ()> {
    // Create the lock file when it is missing: a root without one
    // has no build in it, but creating it here (and holding the
    // flock across the delete) keeps a build that starts between
    // the probe and the delete from entering the root.
    let Ok(file) = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(ROOT_LOCK))
    else {
        return Ok(None);
    };
    match file.try_lock_exclusive() {
        Ok(()) => Ok(Some(BuildLock { _file: file })),
        Err(_) => Err(()),
    }
}

/// Collect package files under `dir` (the whole `pkg/` tree, every
/// repo/arch): (path, size, mtime). The current dep set (by file
/// name, and by dep name for deps whose file name is not known yet)
/// and anything in the 24h grace period are never candidates; their
/// sizes are added to `skipped` (protected bytes). The
/// project-repo-arch lock files (`*.lock`) are not cache data and
/// are never candidates.
fn collect_pkg_files(
    dir: &Path,
    current_dep_files: &[String],
    current_dep_names: &[String],
    now: u64,
    out: &mut Vec<(PathBuf, u64, u64)>,
    skipped: &mut u64,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_pkg_files(
                &path,
                current_dep_files,
                current_dep_names,
                now,
                out,
                skipped,
            );
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if name.ends_with(".lock") {
            continue;
        }
        if let Ok(meta) = fs::metadata(&path) {
            let size = meta.len();
            // Exact file match (deps that carry a `binary`), or a
            // dep-name prefix (deps resolved from the binary list
            // only later — over-protects `helper-devel-…` for a
            // `helper` dep, which costs a re-download at most).
            if current_dep_files.iter().any(|d| d == &name)
                || current_dep_names
                    .iter()
                    .any(|d| name.starts_with(&format!("{d}-")))
            {
                *skipped += size;
                continue;
            }
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if now.saturating_sub(mtime) < GRACE_PERIOD_SECS {
                *skipped += size;
                continue;
            }
            out.push((path, size, mtime));
        }
    }
}

/// Collect build roots under `dir` (project/package/repo-arch
/// layout) as eviction candidates. A locked root (another build
/// holds it) and anything in the 24h grace period never evict; their
/// sizes are added to `skipped` (protected bytes). A root whose
/// marker does not parse is not a candidate at all: its key and
/// times are unknown, so eviction cannot reason about it (it is
/// still a geectl root for `build-wipe`). The most-recent
/// protection is applied by the caller, which sees the whole set.
fn collect_root_entries(dir: &Path, now: u64, skipped: &mut u64) -> Vec<RootEntry> {
    let mut out: Vec<RootEntry> = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let marker_path = path.join(ROOT_MARKER);
        if marker_path.exists() {
            // A locked root is in use: never evict it.
            if root_lock_held(&path) {
                *skipped += dir_size(&path);
                continue;
            }
            let Some(marker) = fs::read_to_string(&marker_path)
                .ok()
                .and_then(|text| serde_json::from_str::<RootMarker>(&text).ok())
            else {
                // Unparsable marker: not a candidate (see above).
                continue;
            };
            if now.saturating_sub(marker.last_build_unix) < GRACE_PERIOD_SECS {
                *skipped += dir_size(&path);
                continue;
            }
            out.push(RootEntry {
                path,
                last_build_unix: marker.last_build_unix,
                project: marker.project,
                package: marker.package,
            });
        } else {
            out.extend(collect_root_entries(&path, now, skipped));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obs::test_support::{basic_credentials, FakeHttp};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A temp dir unique per test (tests run in parallel threads of
    /// one process, so the process id alone is not unique). Removed
    /// on drop: a test that leaves files behind fails the no-crust
    /// assertion in the end-to-end test, and this guard keeps the
    /// machine clean either way.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = std::env::temp_dir().join(format!("geectl-local-build-test-{n}"));
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn host_arch() -> &'static str {
        std::env::consts::ARCH
    }

    fn other_arch() -> &'static str {
        if host_arch() == "x86_64" {
            "aarch64"
        } else {
            "x86_64"
        }
    }

    fn buildinfo_url() -> String {
        format!(
            "https://api.opensuse.org/build/testproject/standard/{}/foo/_buildinfo",
            host_arch()
        )
    }

    fn buildconfig_url() -> String {
        "https://api.opensuse.org/build/testproject/standard/_buildconfig".to_string()
    }

    fn write_source(dir: &Path, spec_name: &str) -> PathBuf {
        let source = dir.join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join(spec_name),
            "Name: foo\nVersion: 1.0\nRelease: 1\nSummary: test\nLicense: MIT\n",
        )
        .unwrap();
        source
    }

    #[test]
    fn new_defaults_to_twenty_gib_cap() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let dir = TempDir::new();
        let vertical = LocalBuild::new(&client, dir.path().join("cache"));
        assert_eq!(vertical.cache_cap_bytes(), 20 * 1024 * 1024 * 1024);
        assert_eq!(vertical.cache_dir(), dir.path().join("cache"));
    }

    #[test]
    fn find_spec_accepts_exactly_one() {
        let dir = TempDir::new();
        let source = write_source(dir.path(), "foo.spec");
        let found = find_spec(&source).unwrap();
        assert_eq!(found.file_name().unwrap(), "foo.spec");
    }

    #[test]
    fn find_spec_refuses_zero_or_many() {
        let dir = TempDir::new();
        let empty = dir.path().join("empty");
        fs::create_dir_all(&empty).unwrap();
        let err = find_spec(&empty).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.to_string().contains("no .spec file"));

        let two = dir.path().join("two");
        fs::create_dir_all(&two).unwrap();
        fs::write(two.join("a.spec"), "").unwrap();
        fs::write(two.join("b.spec"), "").unwrap();
        let err = find_spec(&two).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.to_string().contains("several .spec files"));
    }

    #[test]
    fn multibuild_flavors_parses_packages() {
        let dir = TempDir::new();
        let source = write_source(dir.path(), "foo.spec");
        assert_eq!(multibuild_flavors(&source).unwrap(), None);
        fs::write(
            source.join("_multibuild"),
            "<multibuild><package>flavor-a</package><package>flavor-b</package></multibuild>",
        )
        .unwrap();
        assert_eq!(
            multibuild_flavors(&source).unwrap(),
            Some(vec!["flavor-a".to_string(), "flavor-b".to_string()])
        );
    }

    #[test]
    fn multibuild_flavors_refuses_empty_or_malformed() {
        let dir = TempDir::new();
        let source = write_source(dir.path(), "foo.spec");
        fs::write(source.join("_multibuild"), "<multibuild></multibuild>").unwrap();
        assert!(multibuild_flavors(&source).is_err());
        fs::write(source.join("_multibuild"), "<multibuild><package>flavor-a").unwrap();
        assert!(multibuild_flavors(&source).is_err());
    }

    #[test]
    fn parse_rpmlint_summary_reads_the_summary_line() {
        let log = "some build output\n2 packages and 1 specfiles checked; 3 errors, 2 warnings.\n";
        let summary = parse_rpmlint_summary(log);
        assert_eq!(summary.errors, 3);
        assert_eq!(summary.warnings, 2);
        assert_eq!(summary.badness, 0);
    }

    #[test]
    fn parse_rpmlint_summary_accepts_singular_and_badness() {
        let log = "foo.x86_64: E: bad-thing (Badness: 5)\n1 packages and 1 specfiles checked; 1 error, 1 warning.\n";
        let summary = parse_rpmlint_summary(log);
        assert_eq!(summary.errors, 1);
        assert_eq!(summary.warnings, 1);
        assert_eq!(summary.badness, 5);
    }

    #[test]
    fn parse_rpmlint_summary_falls_back_to_counting_lines() {
        let log = "foo.x86_64: E: one\nfoo.x86_64: W: two\nfoo.x86_64: W: three\n";
        let summary = parse_rpmlint_summary(log);
        assert_eq!(summary.errors, 1);
        assert_eq!(summary.warnings, 2);
    }

    #[test]
    fn parse_check_summary_reads_ctest_line() {
        let log = "100% tests passed, 0 tests failed out of 5\n";
        let summary = parse_check_summary(log).unwrap();
        assert_eq!(summary.passed, 5);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.total, 5);
        assert!(parse_check_summary("no tests here").is_none());
    }

    #[test]
    fn has_finished_line_matches_the_spec() {
        assert!(has_finished_line(
            "building...\nfinished \"build foo.spec\"\n",
            "foo"
        ));
        assert!(!has_finished_line(
            "building...\nfinished \"build bar.spec\"\n",
            "foo"
        ));
        assert!(!has_finished_line("building...\n", "foo"));
        // A suffix of another spec name is not the spec: `barfoo`
        // must not match package `foo`.
        assert!(!has_finished_line(
            "building...\nfinished \"build barfoo.spec\"\n",
            "foo"
        ));
        // A path prefix still matches: the engine prints the spec
        // path, not just its name.
        assert!(has_finished_line(
            "building...\nfinished \"build /some/dir/foo.spec\"\n",
            "foo"
        ));
        // A longer file name sharing the prefix is not the spec.
        assert!(!has_finished_line(
            "building...\nfinished \"build foo.spec.bak\"\n",
            "foo"
        ));
    }

    /// A minimal RPM: 96-byte lead + empty signature header +
    /// empty header + payload. The hdrmd5 is the MD5 of the header
    /// section plus the payload (bytes from the main-header start
    /// to the end of the file) — the fixture carries a payload so
    /// the test discriminates: a header-only MD5 does not match.
    fn make_rpm(payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; 96];
        bytes[0..4].copy_from_slice(&[0xed, 0xab, 0xee, 0xdb]);
        let header = [0x8eu8, 0xad, 0xe8, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(payload);
        bytes
    }

    /// An RPM whose signature header carries a `store_len`-byte
    /// store: the main header starts at 96 + 16 + `store_len` (for
    /// a multiple-of-8 store; the store is all zeros, the lengths
    /// are what the header parser reads).
    fn make_rpm_with_sig_store(store_len: usize, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; 96];
        bytes[0..4].copy_from_slice(&[0xed, 0xab, 0xee, 0xdb]);
        let header = [0x8eu8, 0xad, 0xe8, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let mut sig = header;
        sig[12..16].copy_from_slice(&(store_len as u32).to_be_bytes());
        bytes.extend_from_slice(&sig);
        bytes.extend_from_slice(&vec![0u8; store_len]);
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(payload);
        bytes
    }

    /// An RPM whose main header carries a `store_len`-byte store:
    /// the header end is 112 + 16 + `store_len` (the store is all
    /// zeros; the lengths are what the header parser reads).
    fn make_rpm_with_header_store(store_len: usize, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; 96];
        bytes[0..4].copy_from_slice(&[0xed, 0xab, 0xee, 0xdb]);
        let header = [0x8eu8, 0xad, 0xe8, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        bytes.extend_from_slice(&header);
        let mut main = header;
        main[12..16].copy_from_slice(&(store_len as u32).to_be_bytes());
        bytes.extend_from_slice(&main);
        bytes.extend_from_slice(&vec![0u8; store_len]);
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn rpm_hdrmd5_covers_header_and_payload() {
        let dir = TempDir::new();
        let bytes = make_rpm(b"some payload bytes");
        let path = dir.path().join("foo-1.0-1.x86_64.rpm");
        fs::write(&path, &bytes).unwrap();
        // The header starts after the 96-byte lead and the 16-byte
        // signature header; the hdrmd5 covers it and the payload.
        // The digests are literals computed from that layout, not
        // from the implementation's own slice: computing the
        // expected value with the same expression the
        // implementation uses would let an off-by-offset error in
        // both pass silently.
        let expected = "8697390d2d06fb94dd00c1cf2581c323";
        assert_eq!(rpm_hdrmd5(&path).unwrap(), expected);
        // And it is not the header-only MD5.
        let header_only = "000fc93ea19f678d44aa75e8251333d6";
        assert_ne!(rpm_hdrmd5(&path).unwrap(), header_only);
    }

    #[test]
    fn rpm_hdrmd5_streams_a_large_file() {
        // A file larger than the header probe: the digest must still
        // cover the whole header+payload, and the streaming read
        // must not drop or duplicate a byte. The oracle is the
        // one-shot MD5 over the full in-memory bytes; the offset
        // (112) is pinned by the literal test above, not by this
        // one.
        let dir = TempDir::new();
        let payload = vec![0xabu8; 200 * 1024];
        let bytes = make_rpm(&payload);
        assert!(bytes.len() > 64 * 1024);
        let path = dir.path().join("big-1.0-1.x86_64.rpm");
        fs::write(&path, &bytes).unwrap();
        let expected = format!("{:x}", md5::compute(&bytes[112..]));
        assert_eq!(rpm_hdrmd5(&path).unwrap(), expected);
    }

    #[test]
    fn rpm_hdrmd5_accepts_a_large_main_header() {
        // A main header can end past a fixed 64 KiB probe
        // (glibc's ends at 66,656). The probe must grow until the
        // header is found. The fixture's main header carries a 70
        // KiB store, so its end (70,128) is past one probe chunk.
        let dir = TempDir::new();
        let bytes = make_rpm_with_header_store(70_000, b"payload");
        let path = dir.path().join("big-header-1.0-1.x86_64.rpm");
        fs::write(&path, &bytes).unwrap();
        let expected = format!("{:x}", md5::compute(&bytes[112..]));
        assert_eq!(rpm_hdrmd5(&path).unwrap(), expected);
    }

    #[test]
    fn read_log_tail_returns_only_the_tail() {
        // A log larger than the cap: only the tail is read, and the
        // `finished` line at the end survives it.
        let dir = TempDir::new();
        let path = dir.path().join("build.log");
        let mut content = vec![b'x'; LOG_TAIL_CAP_BYTES as usize + 1024];
        content.extend_from_slice(b"\nfinished \"build foo.spec\"\n");
        fs::write(&path, &content).unwrap();
        let tail = read_log_tail(&path, false);
        // The tail starts at a line boundary: the partial head line
        // is dropped, so only the complete `finished` line remains.
        assert_eq!(tail, "finished \"build foo.spec\"\n");
        // And a short log reads whole, and a missing log reads as
        // empty.
        let short = dir.path().join("short.log");
        fs::write(&short, b"hello\n").unwrap();
        assert_eq!(read_log_tail(&short, false), "hello\n");
        assert_eq!(read_log_tail(&dir.path().join("missing.log"), false), "");
    }

    #[test]
    fn read_log_tail_keeps_a_complete_first_line_on_a_line_boundary() {
        // When the tail window starts exactly on a line boundary
        // (the byte before it is `\n`), the first line in the
        // window is complete and must be kept.
        let dir = TempDir::new();
        let path = dir.path().join("build.log");
        // 26 bytes of head, then exactly one cap of body: the
        // window starts at offset 26, right after the `\n`, and
        // holds the whole body — the `finished` line included.
        let mut content = vec![b'x'; 25];
        content.push(b'\n');
        content.extend_from_slice(&vec![b'y'; LOG_TAIL_CAP_BYTES as usize - 26]);
        content.extend_from_slice(b"finished \"build foo.spec\"\n");
        fs::write(&path, &content).unwrap();
        let tail = read_log_tail(&path, false);
        assert_eq!(tail.len(), LOG_TAIL_CAP_BYTES as usize);
        assert!(tail.ends_with("finished \"build foo.spec\"\n"));
    }

    #[test]
    fn rpm_package_name_parses_name_version_release_arch() {
        assert_eq!(
            rpm_package_name("foo-1.0-1.x86_64.rpm").as_deref(),
            Some("foo")
        );
        assert_eq!(
            rpm_package_name("helper-devel-1.0-1.x86_64.rpm").as_deref(),
            Some("helper-devel")
        );
        assert_eq!(
            rpm_package_name("glibc-2.38-1.aarch64.rpm").as_deref(),
            Some("glibc")
        );
        // A source RPM has no binary name to resolve.
        assert_eq!(rpm_package_name("foo-1.0-1.src.rpm"), None);
        assert_eq!(rpm_package_name("not-an-rpm"), None);
        assert_eq!(rpm_package_name("foo.rpm"), None);
    }

    #[test]
    fn is_transport_cap_error_matches_the_cap_message_only() {
        let cap = GeectlError::new(
            ErrorKind::ServerError,
            "response body exceeds the 10485760-byte limit",
        );
        assert!(is_transport_cap_error(&cap));
        let other = GeectlError::new(ErrorKind::ServerError, "status 500 from the server");
        assert!(!is_transport_cap_error(&other));
        // The bare substring "exceeds the" in another message is
        // not the cap error.
        let bare = GeectlError::new(ErrorKind::ServerError, "the build exceeds the limit");
        assert!(!is_transport_cap_error(&bare));
    }

    #[test]
    fn rpm_hdrmd5_rejects_non_rpm_and_truncated() {
        let dir = TempDir::new();
        let path = dir.path().join("not-an-rpm.rpm");
        fs::write(&path, b"hello").unwrap();
        let err = rpm_hdrmd5(&path).unwrap_err();
        assert!(err.contains("file too short to be an RPM"), "got: {err}");
        let mut bytes = vec![0u8; 96];
        bytes[0..4].copy_from_slice(&[0xed, 0xab, 0xee, 0xdb]);
        fs::write(&path, &bytes).unwrap();
        let err = rpm_hdrmd5(&path).unwrap_err();
        assert!(err.contains("file too short to be an RPM"), "got: {err}");
        // A full prologue with bad lead magic names the magic,
        // not the length.
        let bytes = vec![0u8; 112];
        fs::write(&path, &bytes).unwrap();
        let err = rpm_hdrmd5(&path).unwrap_err();
        assert!(
            err.contains("not an RPM file (bad lead magic)"),
            "got: {err}"
        );
    }

    #[test]
    fn rpm_hdrmd5_accepts_a_main_header_past_four_mib() {
        // The fixture carries a 5,503,288 byte main-header store,
        // the size measured from kernel-obs-build-7.3.5-1.1.x86_64.rpm,
        // so its header ends at offset 5,503,416 — past the old 4 MiB
        // probe cap, which refused it as "RPM header exceeds the probe
        // cap" and failed the e2e build. The offset comes from the
        // signature prologue and the length is checked against the
        // file length, so no header store is buffered and the digest
        // covers it.
        let dir = TempDir::new();
        let bytes = make_rpm_with_header_store(5_503_288, b"payload");
        let path = dir.path().join("huge-header-1.0-1.x86_64.rpm");
        fs::write(&path, &bytes).unwrap();
        let expected = format!("{:x}", md5::compute(&bytes[112..]));
        assert_eq!(rpm_hdrmd5(&path).unwrap(), expected);
    }

    #[test]
    fn rpm_hdrmd5_accepts_a_signature_header_past_four_mib() {
        // A signature store past the old 4 MiB cap: the main header
        // starts past it, but the offset comes from the signature
        // prologue, so the store is never read.
        let dir = TempDir::new();
        let bytes = make_rpm_with_sig_store(5 * 1024 * 1024, b"payload");
        let path = dir.path().join("huge-sig-header-1.0-1.x86_64.rpm");
        fs::write(&path, &bytes).unwrap();
        // 96 lead + 16 signature prologue + 5 MiB store.
        let header_start = 96 + 16 + 5 * 1024 * 1024;
        let expected = format!("{:x}", md5::compute(&bytes[header_start..]));
        assert_eq!(rpm_hdrmd5(&path).unwrap(), expected);
    }

    #[test]
    fn rpm_hdrmd5_still_rejects_a_truncated_header() {
        // A file genuinely truncated inside its header still says
        // truncated: same header shape, cut short of its end.
        let dir = TempDir::new();
        let mut short = make_rpm_with_header_store(1000, b"payload");
        short.truncate(500);
        let path = dir.path().join("short-header-1.0-1.x86_64.rpm");
        fs::write(&path, &short).unwrap();
        let err = rpm_hdrmd5(&path).unwrap_err();
        assert!(err.contains("truncated"), "got: {err}");
    }

    #[test]
    fn collect_rpms_ignores_a_stale_top_level_packages_dir() {
        // The engine writes under `.mount/.build.packages`; a
        // `.build.packages` at the root top level is stale (a
        // previous layout, a stray file), and its RPMs must never
        // be reported as this build's outputs.
        let dir = TempDir::new();
        let root = dir.path();
        fs::create_dir_all(root.join(".build.packages")).unwrap();
        fs::write(
            root.join(".build.packages").join("stale-1.0-1.x86_64.rpm"),
            b"stale",
        )
        .unwrap();
        fs::create_dir_all(root.join(".mount").join(".build.packages").join("RPMS")).unwrap();
        let real = root
            .join(".mount")
            .join(".build.packages")
            .join("RPMS")
            .join("foo-1.0-1.x86_64.rpm");
        fs::write(&real, b"real").unwrap();
        assert_eq!(collect_rpms(root), vec![real]);
    }

    /// The stub tests below run shell scripts, so they are unix-only.
    /// They prove the flow end to end against a stub engine and a
    /// stub podman — no real build runs in CI.
    #[cfg(unix)]
    mod unix_tests {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        fn write_script(path: &Path, body: &str) {
            fs::write(path, body).unwrap();
            let mut perms = fs::metadata(path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).unwrap();
        }

        /// A stub engine: answers `--version`, records its argv,
        /// writes a canned log with a `finished` line, and drops a
        /// fake RPM into the root's `.build.packages/RPMS/` — the
        /// subdir the real engine uses. With `--shell-cmd` it
        /// echoes the command to stdout (the captured shell output).
        fn stub_engine(dir: &Path) -> (PathBuf, PathBuf) {
            let script = dir.join("build-stub");
            let argv_log = dir.join("engine-argv.log");
            write_script(
                &script,
                &format!(
                    r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo "build stub 1.0"; exit 0; fi
echo "$@" >> "{argv_log}"
root=""
logfile=""
spec=""
shell_cmd=""
prev=""
for arg in "$@"; do
  case "$arg" in
    --root=*) root="${{arg#--root=}}" ;;
    --logfile=*) logfile="${{arg#--logfile=}}" ;;
    *.spec) spec="$arg" ;;
  esac
  if [ "$prev" = "--shell-cmd" ]; then shell_cmd="$arg"; fi
  prev="$arg"
done
if [ -n "$shell_cmd" ]; then echo "shell output: $shell_cmd"; fi
if [ -n "$spec" ]; then
  pkg=$(basename "$spec" .spec)
  mkdir -p "$root/.build.packages/RPMS"
  touch "$root/.build.packages/RPMS/$pkg-1.0-1.{arch}.rpm"
  if [ -n "$logfile" ]; then printf 'build log\nfinished "build %s.spec"\n' "$pkg" > "$logfile"; fi
fi
"#,
                    argv_log = argv_log.display(),
                    arch = host_arch(),
                ),
            );
            (script, argv_log)
        }

        /// A stub podman: `image inspect` reports a digest (or
        /// nothing, when `with_image` is false), `pull` succeeds,
        /// `images` lists the digest with its repository, `rmi`
        /// succeeds.
        fn stub_podman(dir: &Path, with_image: bool) -> (PathBuf, PathBuf) {
            let script = dir.join("podman-stub");
            let call_log = dir.join("podman-calls.log");
            let digest_line = if with_image {
                "echo \"sha256:fake-digest-1234\""
            } else {
                "exit 1"
            };
            write_script(
                &script,
                &format!(
                    r#"#!/bin/sh
echo "$@" >> "{call_log}"
if [ "$1" = "image" ] && [ "$2" = "inspect" ]; then
  case "$4" in
    *Digest*) {digest_line} ;;
    *Size*) echo "123456789" ;;
  esac
  exit 0
fi
if [ "$1" = "pull" ]; then exit 0; fi
if [ "$1" = "images" ]; then
  if {with_image}; then echo "registry.opensuse.org/opensuse/tumbleweed sha256:fake-digest-1234"; fi
  exit 0
fi
if [ "$1" = "rmi" ]; then exit 0; fi
exit 0
"#,
                    call_log = call_log.display(),
                    with_image = with_image,
                ),
            );
            (script, call_log)
        }

        fn client_with(http: &FakeHttp) -> ObsClient<'_> {
            ObsClient::with_credentials(http, basic_credentials()).unwrap()
        }

        fn buildinfo_ok() -> String {
            format!(
                "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                <bdep name=\"glibc\" project=\"testproject\" repository=\"standard\" binary=\"glibc-2.38-1.{arch}.rpm\"/>\
                <bdep name=\"ownpkg\" project=\"home:scarabeus_iv\" repository=\"standard\" binary=\"ownpkg-1.0-1.{arch}.rpm\"/>\
                <bdep name=\"pre\" project=\"testproject\" repository=\"standard\" binary=\"pre-1.0-1.{arch}.rpm\" preinstall=\"1\"/>\
                <bdep name=\"helper\" project=\"testproject\" repository=\"standard\"/>\
                </buildinfo>",
                arch = host_arch()
            )
        }

        /// Build the full stubbed setup: buildinfo + buildconfig +
        /// dep downloads + binary list for the binary-less dep.
        /// Returns the client and the http recorder (the client
        /// borrows the recorder, so both must outlive the build).
        struct Setup {
            http: FakeHttp,
        }

        impl Setup {
            fn new() -> Self {
                let arch = host_arch();
                let http = FakeHttp::new()
                    .with_get(&buildinfo_url(), &buildinfo_ok())
                    .with_get(&buildconfig_url(), "Macro: test 1\n")
                    .with_get_bytes(
                        &format!(
                            "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/glibc-2.38-1.{arch}.rpm"
                        ),
                        b"fake glibc rpm",
                    )
                    .with_get_bytes(
                        &format!(
                            "https://api.opensuse.org/build/home:scarabeus_iv/standard/{arch}/_repository/ownpkg-1.0-1.{arch}.rpm"
                        ),
                        b"fake ownpkg rpm",
                    )
                    .with_get_bytes(
                        &format!(
                            "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/pre-1.0-1.{arch}.rpm"
                        ),
                        b"fake pre rpm",
                    )
                    .with_get(
                        &format!(
                            "https://api.opensuse.org/build/testproject/standard/{arch}/_repository"
                        ),
                        &format!(
                            "<binarylist><binary filename=\"helper-1.0-1.{arch}.rpm\" size=\"10\" mtime=\"1\"/><binary filename=\"helper-debuginfo-1.0-1.{arch}.rpm\" size=\"10\" mtime=\"1\"/></binarylist>"
                        ),
                    )
                    .with_get_bytes(
                        &format!(
                            "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/helper-1.0-1.{arch}.rpm"
                        ),
                        b"fake helper rpm",
                    );
                Setup { http }
            }

            fn client(&self) -> ObsClient<'_> {
                ObsClient::with_credentials(&self.http, basic_credentials()).unwrap()
            }
        }

        fn vertical<'a>(
            client: &'a ObsClient<'a>,
            dir: &Path,
            engine: PathBuf,
            podman: PathBuf,
        ) -> LocalBuild<'a> {
            LocalBuild::new(client, dir.join("cache"))
                .with_engine(engine)
                .with_podman(podman)
        }

        /// Apply a one-item eviction plan, retrying while the item's
        /// root reads as locked only because of the fork window
        /// described on `acquire_root_lock` (eviction probes the
        /// lock instead of retrying it, so the retry lives here):
        /// the first apply may record a lock failure for a lock that
        /// is already released, so retry until the item evicts or
        /// the deadline passes.
        fn apply_one_item_soon(vertical: &LocalBuild<'_>, item: EvictedItem) -> EvictionReport {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                let mut report = EvictionReport {
                    evicted: vec![item.clone()],
                    ..Default::default()
                };
                vertical.apply_eviction_plan(&mut report);
                if !report.evicted.is_empty() || std::time::Instant::now() >= deadline {
                    return report;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        #[test]
        fn dry_run_returns_the_plan_and_touches_nothing() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.dry_run = true;
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "dry-run");
            assert!(record.dry_run);
            let plan = record.plan.unwrap();
            assert!(plan.argv.iter().any(|a| a == "--vm-type=podman"));
            assert!(plan.argv.iter().any(|a| a.starts_with("--sccache-uri=")));
            assert!(plan.argv.last().unwrap().ends_with("foo.spec"));
            // Nothing was created: no root, no cache, no server call.
            assert!(!dir.path().join("cache").join("build-roots").exists());
            assert!(setup.http.get_calls().is_empty());
            assert!(setup.http.get_bytes_calls().is_empty());
        }

        #[test]
        fn validate_build_request_rejects_a_bad_alternative_project() {
            let dir = TempDir::new();
            let source = write_source(dir.path(), "foo.spec");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.alternative_project = Some("bad name!".to_string());
            let err = validate_build_request(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(err.to_string().contains("invalid OBS project name"));
        }

        #[test]
        fn validate_build_request_rejects_a_traversal_alternative_project() {
            let dir = TempDir::new();
            let source = write_source(dir.path(), "foo.spec");
            for alt in ["..", "."] {
                let mut req =
                    BuildRequest::new(source.clone(), "testproject", "standard", host_arch());
                req.alternative_project = Some(alt.to_string());
                let err = validate_build_request(&req).unwrap_err();
                assert_eq!(err.kind(), ErrorKind::InvalidInput);
                assert!(
                    err.to_string().contains("alternative project"),
                    "got: {err}"
                );
            }
        }

        #[test]
        fn validate_build_request_rejects_clean_with_noinit() {
            let dir = TempDir::new();
            let source = write_source(dir.path(), "foo.spec");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.clean = true;
            req.noinit = true;
            let err = validate_build_request(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(err.to_string().contains("--clean and --noinit conflict"));
        }

        #[test]
        fn validate_build_request_rejects_a_missing_prefer_dir() {
            let dir = TempDir::new();
            let source = write_source(dir.path(), "foo.spec");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.prefer_packages = Some(dir.path().join("no-such-dir"));
            let err = validate_build_request(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(err.to_string().contains("prefer-packages directory"));
        }

        #[test]
        fn validate_build_request_rejects_clean_on_a_markerless_custom_root() {
            let dir = TempDir::new();
            let source = write_source(dir.path(), "foo.spec");
            let root = dir.path().join("custom-root");
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join("some-file"), "user data").unwrap();
            let mut req = BuildRequest::new(source.clone(), "testproject", "standard", host_arch());
            req.root = Some(root.clone());
            req.clean = true;
            let err = validate_build_request(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NotAGeectlBuildRoot);
            assert!(err.to_string().contains("--clean refuses a custom root"));
            // The marker makes it a geectl root: clean is allowed.
            fs::write(root.join(ROOT_MARKER), "").unwrap();
            validate_build_request(&req).unwrap();
            // An empty root without a marker is allowed too: the
            // wipe has nothing to destroy.
            let empty = dir.path().join("empty-root");
            fs::create_dir_all(&empty).unwrap();
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.root = Some(empty);
            req.clean = true;
            validate_build_request(&req).unwrap();
        }

        #[test]
        fn check_clean_custom_root_gate() {
            // The gate the wipe re-checks under the root lock: a
            // markerless root with user data refuses, a marked one
            // and an empty one pass, and the lock file never counts
            // (under the lock it is always present — this build just
            // created it — and the wipe skips it).
            let dir = TempDir::new();
            let root = dir.path().join("custom-root");
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join("some-file"), "user data").unwrap();
            let err = check_clean_custom_root(&root).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NotAGeectlBuildRoot);
            assert!(err.to_string().contains("--clean refuses a custom root"));
            fs::write(root.join(ROOT_MARKER), "").unwrap();
            check_clean_custom_root(&root).unwrap();
            fs::remove_file(root.join(ROOT_MARKER)).unwrap();
            fs::remove_file(root.join("some-file")).unwrap();
            check_clean_custom_root(&root).unwrap();
            // The lock file alone is not user data.
            fs::write(root.join(ROOT_LOCK), "pid=1 started=0 package=foo\n").unwrap();
            check_clean_custom_root(&root).unwrap();
            fs::write(root.join("some-file"), "user data").unwrap();
            check_clean_custom_root(&root).unwrap_err();
        }

        #[test]
        fn validate_build_request_rejects_noinit_with_prefer_packages() {
            let dir = TempDir::new();
            let source = write_source(dir.path(), "foo.spec");
            let prefer = dir.path().join("prefer");
            fs::create_dir_all(&prefer).unwrap();
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.noinit = true;
            req.prefer_packages = Some(prefer);
            let err = validate_build_request(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(err
                .to_string()
                .contains("--noinit and --prefer-packages conflict"));
        }

        #[test]
        fn validate_build_request_rejects_a_root_naming_a_file() {
            let dir = TempDir::new();
            let source = write_source(dir.path(), "foo.spec");
            let file = dir.path().join("not-a-dir");
            fs::write(&file, "x").unwrap();
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.root = Some(file);
            let err = validate_build_request(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(err.to_string().contains("is not a directory"));
        }

        #[test]
        fn validate_build_request_rejects_a_keep_dir_naming_a_file() {
            let dir = TempDir::new();
            let source = write_source(dir.path(), "foo.spec");
            let file = dir.path().join("not-a-dir");
            fs::write(&file, "x").unwrap();
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.keep_rpms = Some(file);
            let err = validate_build_request(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(err.to_string().contains("is not a directory"));
        }

        #[test]
        fn dry_run_plan_carries_the_new_flags() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let root = dir.path().join("custom-root");
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join(RPMLIST), "").unwrap();
            fs::write(root.join(BUILDCONFIG), "").unwrap();
            let prefer = dir.path().join("prefer");
            fs::create_dir_all(&prefer).unwrap();
            let keep = dir.path().join("keep");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.dry_run = true;
            req.trust_all_projects = true;
            req.alternative_project = Some("openSUSE:Factory".to_string());
            req.root = Some(root.clone());
            req.keep_rpms = Some(keep.clone());
            req.prefer_packages = Some(prefer.clone());
            let record = vertical.build(&req).unwrap();
            let plan = record.plan.unwrap();
            assert_eq!(
                plan.alternative_project.as_deref(),
                Some("openSUSE:Factory")
            );
            assert!(plan.trust_all_projects);
            assert_eq!(plan.root, root);
            assert_eq!(plan.keep_rpms, Some(keep));
            assert_eq!(plan.prefer_packages, Some(prefer));
            // The engine builds inside the root's mount dir.
            assert!(
                plan.argv
                    .iter()
                    .any(|a| a == &format!("--root={}/.mount", root.display())),
                "argv: {:?}",
                plan.argv
            );
            assert!(
                plan.argv
                    .iter()
                    .any(|a| a == &format!("--sccache-uri={}/sccache/sccache.tar", root.display())),
                "argv: {:?}",
                plan.argv
            );
            assert!(setup.http.get_calls().is_empty());
        }

        #[test]
        fn dry_run_plan_carries_noinit() {
            // --noinit takes no --prefer-packages, so its argv pin
            // lives in its own test with an initialized root.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let root = dir.path().join("custom-root");
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join(RPMLIST), "").unwrap();
            fs::write(root.join(BUILDCONFIG), "").unwrap();
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.dry_run = true;
            req.root = Some(root.clone());
            req.noinit = true;
            let record = vertical.build(&req).unwrap();
            let plan = record.plan.unwrap();
            assert!(plan.argv.iter().any(|a| a == "--noinit"));
            assert!(
                plan.argv
                    .iter()
                    .any(|a| a == &format!("--root={}/.mount", root.display())),
                "argv: {:?}",
                plan.argv
            );
        }

        #[test]
        fn dry_run_noinit_on_a_fresh_root_prints_the_plan() {
            // The plan is data and needs neither rpmlist nor
            // buildconfig, so a dry run prints it without consulting
            // the root; the refusal stays a real-build check.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.dry_run = true;
            req.root = Some(dir.path().join("fresh-root"));
            req.noinit = true;
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "dry-run");
            assert!(record.dry_run);
            let plan = record.plan.unwrap();
            assert!(plan.argv.iter().any(|a| a == "--noinit"));
            assert!(setup.http.get_calls().is_empty());
        }

        #[test]
        fn noinit_build_on_a_fresh_root_is_refused() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let root = dir.path().join("fresh-root");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.root = Some(root.clone());
            req.noinit = true;
            let err = vertical.build(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(err
                .to_string()
                .contains("--noinit needs an initialized root"));
            assert!(setup.http.get_calls().is_empty());
            // The refusal fires before the root is created.
            assert!(!root.exists());
        }

        /// The Setup registrations with the buildinfo/buildconfig
        /// served from another project: the dep downloads stay the
        /// same, only the fetch project moves.
        fn alt_setup(buildinfo: &str) -> Setup {
            let arch = host_arch();
            let http = FakeHttp::new()
                .with_get(
                    &format!(
                        "https://api.opensuse.org/build/openSUSE:Factory/standard/{arch}/foo/_buildinfo"
                    ),
                    buildinfo,
                )
                .with_get(
                    "https://api.opensuse.org/build/openSUSE:Factory/standard/_buildconfig",
                    "Macro: test 1\n",
                )
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/glibc-2.38-1.{arch}.rpm"
                    ),
                    b"fake glibc rpm",
                )
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/home:scarabeus_iv/standard/{arch}/_repository/ownpkg-1.0-1.{arch}.rpm"
                    ),
                    b"fake ownpkg rpm",
                )
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/pre-1.0-1.{arch}.rpm"
                    ),
                    b"fake pre rpm",
                )
                .with_get(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{arch}/_repository"
                    ),
                    &format!(
                        "<binarylist><binary filename=\"helper-1.0-1.{arch}.rpm\" size=\"10\" mtime=\"1\"/><binary filename=\"helper-debuginfo-1.0-1.{arch}.rpm\" size=\"10\" mtime=\"1\"/></binarylist>"
                    ),
                )
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/helper-1.0-1.{arch}.rpm"
                    ),
                    b"fake helper rpm",
                );
            Setup { http }
        }

        #[test]
        fn build_fetches_buildinfo_for_the_alternative_project() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = alt_setup(&buildinfo_ok());
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.alternative_project = Some("openSUSE:Factory".to_string());
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");
            let calls = setup.http.get_calls();
            let alt_buildinfo = format!(
                "https://api.opensuse.org/build/openSUSE:Factory/standard/{}/foo/_buildinfo",
                host_arch()
            );
            assert!(
                calls.iter().any(|(u, _)| u == &alt_buildinfo),
                "calls: {calls:?}"
            );
            assert!(
                !calls.iter().any(|(u, _)| u == &buildinfo_url()),
                "the own project's buildinfo must not be fetched: {calls:?}"
            );
        }

        #[test]
        fn build_resolves_a_project_less_dep_from_the_alternative_project() {
            // A dep without a `project` carries no origin of its own:
            // under --alternative-project it resolves from the fetch
            // project, not the package's own project.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let http = FakeHttp::new()
                .with_get(
                    &format!(
                        "https://api.opensuse.org/build/openSUSE:Factory/standard/{arch}/foo/_buildinfo"
                    ),
                    &format!(
                        "<buildinfo project=\"openSUSE:Factory\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                        <bdep name=\"noproject\" binary=\"noproject-1.0-1.{arch}.rpm\"/>\
                        </buildinfo>"
                    ),
                )
                .with_get(
                    "https://api.opensuse.org/build/openSUSE:Factory/standard/_buildconfig",
                    "Macro: test 1\n",
                )
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/openSUSE:Factory/standard/{arch}/_repository/noproject-1.0-1.{arch}.rpm"
                    ),
                    b"fake noproject rpm",
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.alternative_project = Some("openSUSE:Factory".to_string());
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");
            assert!(
                http.get_bytes_calls()
                    .iter()
                    .any(|(u, _)| u.contains("openSUSE:Factory")),
                "calls: {:?}",
                http.get_bytes_calls()
            );
        }

        /// A buildinfo like `buildinfo_ok` with the glibc dep
        /// carrying an hdrmd5, for the prefer-dir tests.
        fn buildinfo_with_glibc_digest(digest: &str) -> String {
            let arch = host_arch();
            format!(
                "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                <bdep name=\"glibc\" project=\"testproject\" repository=\"standard\" binary=\"glibc-2.38-1.{arch}.rpm\" hdrmd5=\"{digest}\"/>\
                <bdep name=\"ownpkg\" project=\"home:scarabeus_iv\" repository=\"standard\" binary=\"ownpkg-1.0-1.{arch}.rpm\"/>\
                <bdep name=\"pre\" project=\"testproject\" repository=\"standard\" binary=\"pre-1.0-1.{arch}.rpm\" preinstall=\"1\"/>\
                <bdep name=\"helper\" project=\"testproject\" repository=\"standard\"/>\
                </buildinfo>"
            )
        }

        #[test]
        fn build_prefers_a_package_from_the_prefer_dir() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let prefer = dir.path().join("prefer");
            fs::create_dir_all(&prefer).unwrap();
            let rpm_path = prefer.join(format!("glibc-2.38-1.{}.rpm", host_arch()));
            fs::write(&rpm_path, make_rpm(b"preferred glibc")).unwrap();
            let digest = rpm_hdrmd5(&rpm_path).unwrap();
            let http = FakeHttp::new()
                .with_get(&buildinfo_url(), &buildinfo_with_glibc_digest(&digest))
                .with_get(&buildconfig_url(), "Macro: test 1\n")
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{}/_repository/glibc-2.38-1.{}.rpm",
                        host_arch(),
                        host_arch()
                    ),
                    b"fake glibc rpm",
                )
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/home:scarabeus_iv/standard/{}/_repository/ownpkg-1.0-1.{}.rpm",
                        host_arch(),
                        host_arch()
                    ),
                    b"fake ownpkg rpm",
                )
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{}/_repository/pre-1.0-1.{}.rpm",
                        host_arch(),
                        host_arch()
                    ),
                    b"fake pre rpm",
                )
                .with_get(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{}/_repository",
                        host_arch()
                    ),
                    &format!(
                        "<binarylist><binary filename=\"helper-1.0-1.{}.rpm\" size=\"10\" mtime=\"1\"/><binary filename=\"helper-debuginfo-1.0-1.{}.rpm\" size=\"10\" mtime=\"1\"/></binarylist>",
                        host_arch(),
                        host_arch()
                    ),
                )
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{}/_repository/helper-1.0-1.{}.rpm",
                        host_arch(),
                        host_arch()
                    ),
                    b"fake helper rpm",
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.prefer_packages = Some(prefer);
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");
            assert_eq!(record.cache.pkg_preferred, 1);
            assert!(
                !http
                    .get_bytes_calls()
                    .iter()
                    .any(|(u, _)| u.contains("glibc-2.38-1")),
                "the preferred dep must not be downloaded: {:?}",
                http.get_bytes_calls()
            );
        }

        #[test]
        fn build_prefers_a_package_over_a_warm_cache() {
            // An explicit prefer file overrides the cached copy: a
            // rebuilt RPM in the prefer dir is used on the next run
            // instead of the stale cached one.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let rpm_bytes = make_rpm(b"preferred glibc");
            let prefer = dir.path().join("prefer");
            fs::create_dir_all(&prefer).unwrap();
            let rpm_path = prefer.join(format!("glibc-2.38-1.{}.rpm", host_arch()));
            fs::write(&rpm_path, &rpm_bytes).unwrap();
            let digest = rpm_hdrmd5(&rpm_path).unwrap();
            let http = FakeHttp::new()
                .with_get(&buildinfo_url(), &buildinfo_with_glibc_digest(&digest))
                .with_get(&buildconfig_url(), "Macro: test 1\n")
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{}/_repository/glibc-2.38-1.{}.rpm",
                        host_arch(),
                        host_arch()
                    ),
                    &rpm_bytes,
                )
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/home:scarabeus_iv/standard/{}/_repository/ownpkg-1.0-1.{}.rpm",
                        host_arch(),
                        host_arch()
                    ),
                    b"fake ownpkg rpm",
                )
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{}/_repository/pre-1.0-1.{}.rpm",
                        host_arch(),
                        host_arch()
                    ),
                    b"fake pre rpm",
                )
                .with_get(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{}/_repository",
                        host_arch()
                    ),
                    &format!(
                        "<binarylist><binary filename=\"helper-1.0-1.{}.rpm\" size=\"10\" mtime=\"1\"/><binary filename=\"helper-debuginfo-1.0-1.{}.rpm\" size=\"10\" mtime=\"1\"/></binarylist>",
                        host_arch(),
                        host_arch()
                    ),
                )
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{}/_repository/helper-1.0-1.{}.rpm",
                        host_arch(),
                        host_arch()
                    ),
                    b"fake helper rpm",
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            // First build, no prefer: the download is cached.
            let req = BuildRequest::new(source.clone(), "testproject", "standard", host_arch());
            let first = vertical.build(&req).unwrap();
            assert_eq!(first.verdict, "green");
            assert_eq!(first.cache.pkg_preferred, 0);
            // Second build, prefer set: the cached copy is not used.
            let mut second = BuildRequest::new(source, "testproject", "standard", host_arch());
            second.prefer_packages = Some(prefer);
            let record = vertical.build(&second).unwrap();
            assert_eq!(record.verdict, "green");
            assert_eq!(record.cache.pkg_preferred, 1);
            assert_eq!(
                http.get_bytes_calls()
                    .iter()
                    .filter(|(u, _)| u.contains("glibc-2.38-1"))
                    .count(),
                1,
                "the preferred dep is downloaded once, then preferred: {:?}",
                http.get_bytes_calls()
            );
        }

        #[test]
        fn build_fails_closed_on_a_preferred_package_with_a_bad_digest() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let prefer = dir.path().join("prefer");
            fs::create_dir_all(&prefer).unwrap();
            let rpm_path = prefer.join(format!("glibc-2.38-1.{}.rpm", host_arch()));
            fs::write(&rpm_path, make_rpm(b"preferred glibc")).unwrap();
            let http = FakeHttp::new()
                .with_get(
                    &buildinfo_url(),
                    &buildinfo_with_glibc_digest("00000000000000000000000000000000"),
                )
                .with_get(&buildconfig_url(), "Macro: test 1\n");
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.prefer_packages = Some(prefer);
            let err = vertical.build(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::CacheCorrupt);
            assert!(err.to_string().contains("hdrmd5"), "got: {err}");
        }

        #[test]
        fn build_keeps_rpms_in_the_keep_dir() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let keep = dir.path().join("keep");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.keep_rpms = Some(keep.clone());
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");
            let expected = keep.join(format!("foo-1.0-1.{}.rpm", host_arch()));
            assert_eq!(record.kept_rpms, vec![expected.clone()]);
            assert!(expected.is_file());
        }

        #[test]
        fn build_keeps_nothing_on_a_failed_build() {
            // An engine that exits 0 but never writes the finished
            // line: verdict failed, so nothing is kept.
            let dir = TempDir::new();
            let script = dir.path().join("build-stub-nofinish");
            let argv_log = dir.path().join("engine-argv-nofinish.log");
            write_script(
                &script,
                &format!(
                    r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo "build stub 1.0"; exit 0; fi
echo "$@" >> "{argv_log}"
root=""
logfile=""
spec=""
for arg in "$@"; do
  case "$arg" in
    --root=*) root="${{arg#--root=}}" ;;
    --logfile=*) logfile="${{arg#--logfile=}}" ;;
    *.spec) spec="$arg" ;;
  esac
done
if [ -n "$spec" ]; then
  pkg=$(basename "$spec" .spec)
  mkdir -p "$root/.build.packages/RPMS"
  touch "$root/.build.packages/RPMS/$pkg-1.0-1.{arch}.rpm"
  if [ -n "$logfile" ]; then printf 'build log\nno finished line\n' > "$logfile"; fi
fi
"#,
                    argv_log = argv_log.display(),
                    arch = host_arch(),
                ),
            );
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), script, podman);
            let source = write_source(dir.path(), "foo.spec");
            let keep = dir.path().join("keep");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.keep_rpms = Some(keep.clone());
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "failed");
            assert!(record.kept_rpms.is_empty());
            assert!(!keep.exists());
        }

        #[test]
        fn build_fails_when_a_kept_rpm_cannot_be_copied() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let keep = dir.path().join("keep");
            // A directory where the RPM would be copied: the copy
            // fails, and the build fails with it.
            fs::create_dir_all(keep.join(format!("foo-1.0-1.{}.rpm", host_arch()))).unwrap();
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.keep_rpms = Some(keep);
            let err = vertical.build(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InternalError);
            assert!(
                err.to_string().contains("cannot copy built RPM"),
                "got: {err}"
            );
        }

        #[test]
        fn noinit_build_after_a_normal_build_is_green() {
            let dir = TempDir::new();
            let (engine, argv_log) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source.clone(), "testproject", "standard", host_arch());
            let first = vertical.build(&req).unwrap();
            assert_eq!(first.verdict, "green");
            // Dep prep must not run again: the rpmlist/buildconfig
            // a sentinel overwrites stay untouched, and nothing is
            // downloaded a second time.
            let rpmlist = first.root_path.join(RPMLIST);
            let buildconfig = first.root_path.join(BUILDCONFIG);
            fs::write(&rpmlist, "sentinel rpmlist").unwrap();
            fs::write(&buildconfig, "sentinel buildconfig").unwrap();
            let downloads_before = setup.http.get_bytes_calls().len();
            let mut second = BuildRequest::new(source, "testproject", "standard", host_arch());
            second.noinit = true;
            let record = vertical.build(&second).unwrap();
            assert_eq!(record.verdict, "green");
            assert_eq!(fs::read_to_string(&rpmlist).unwrap(), "sentinel rpmlist");
            assert_eq!(
                fs::read_to_string(&buildconfig).unwrap(),
                "sentinel buildconfig"
            );
            assert_eq!(setup.http.get_bytes_calls().len(), downloads_before);
            let argv = fs::read_to_string(&argv_log).unwrap();
            assert!(
                argv.lines().any(|line| line.contains("--noinit")),
                "engine argv: {argv}"
            );
        }

        #[test]
        fn clean_build_on_a_marked_custom_root_wipes_under_the_lock() {
            // A clean build on a marked custom root runs the wipe
            // under the lock: the marker passes the gate and the
            // stale file is gone. (The lock exclusion itself is
            // pinned by the unit test and the empty-root test.)
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let root = dir.path().join("custom-root");
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join(ROOT_MARKER), "").unwrap();
            fs::write(root.join("stale-output"), b"stale").unwrap();
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.root = Some(root.clone());
            req.clean = true;
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");
            assert!(!root.join("stale-output").exists());
        }

        #[test]
        fn clean_build_on_an_empty_markerless_custom_root_succeeds() {
            // The under-lock re-check must not refuse an empty
            // markerless root: the only entry under the lock is the
            // lock file itself, which never counts. Without the
            // exclusion this build would refuse on the file the lock
            // just created.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let root = dir.path().join("custom-root");
            fs::create_dir_all(&root).unwrap();
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.root = Some(root.clone());
            req.clean = true;
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");
        }

        #[test]
        fn build_refuses_another_architecture() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", other_arch());
            let err = vertical.build(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::UnsupportedArch);
            assert!(setup.http.get_calls().is_empty());
        }

        #[test]
        fn validate_build_request_rejects_bad_coordinates() {
            let dir = TempDir::new();
            let source = dir.path().join("pkg");
            let good = BuildRequest::new(
                source.clone(),
                "testproject",
                "standard",
                std::env::consts::ARCH,
            );
            assert!(validate_build_request(&good).is_ok());
            let bad_arch =
                BuildRequest::new(source.clone(), "testproject", "standard", other_arch());
            assert_eq!(
                validate_build_request(&bad_arch).unwrap_err().kind(),
                ErrorKind::UnsupportedArch
            );
            let bad_project = BuildRequest::new(
                source.clone(),
                "bad project",
                "standard",
                std::env::consts::ARCH,
            );
            assert_eq!(
                validate_build_request(&bad_project).unwrap_err().kind(),
                ErrorKind::InvalidInput
            );
            let bad_repo = BuildRequest::new(
                source.clone(),
                "testproject",
                "bad/repo",
                std::env::consts::ARCH,
            );
            assert_eq!(
                validate_build_request(&bad_repo).unwrap_err().kind(),
                ErrorKind::InvalidInput
            );
            let mut zero_timeout =
                BuildRequest::new(source, "testproject", "standard", std::env::consts::ARCH);
            zero_timeout.timeout_secs = 0;
            assert_eq!(
                validate_build_request(&zero_timeout).unwrap_err().kind(),
                ErrorKind::InvalidInput
            );
        }

        #[test]
        fn build_refuses_a_foreign_home_project() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let http = FakeHttp::new()
                .with_get(
                    &buildinfo_url(),
                    &format!(
                        "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                        <bdep name=\"glibc\" project=\"home:someoneelse\" repository=\"standard\" binary=\"glibc-2.38-1.{arch}.rpm\"/>\
                        </buildinfo>",
                        arch = host_arch()
                    ),
                )
                .with_get(&buildconfig_url(), "Macro: test 1\n");
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", host_arch());
            let err = vertical.build(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::UntrustedProject);
            assert!(err.to_string().contains("home:someoneelse"));
            assert!(err.to_string().contains("--trust-all-projects"));
        }

        #[test]
        fn build_rejects_an_escape_sequence_project_before_the_trust_check() {
            // A project name carrying an ANSI escape (as a numeric
            // character reference, the only way one survives the
            // buildinfo parse) must fail in `validate_dep`, not reach
            // the terminal through the `UntrustedProject` error or
            // the debug line: validation runs before `check_trust`.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let http = FakeHttp::new()
                .with_get(
                    &buildinfo_url(),
                    &format!(
                        "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                        <bdep name=\"glibc\" project=\"home:evil&#27;[2J\" repository=\"standard\" binary=\"glibc-2.38-1.{arch}.rpm\"/>\
                        </buildinfo>",
                        arch = host_arch()
                    ),
                )
                .with_get(&buildconfig_url(), "Macro: test 1\n");
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", host_arch());
            let err = vertical.build(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(err.to_string().contains("dependency project"));
            assert!(!err.to_string().contains('\x1b'));
        }

        #[test]
        fn build_with_bearer_credentials_says_the_own_home_is_unknown() {
            // A Bearer token carries no username, and the OBS API
            // has no whoami endpoint, so the caller's own home:
            // cannot be determined: a dep from the caller's own
            // home: is refused as foreign, and the refusal says
            // why instead of telling the user to pass the broad
            // flag for their own home.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let http = FakeHttp::new()
                .with_get(
                    &buildinfo_url(),
                    &format!(
                        "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                        <bdep name=\"ownpkg\" project=\"home:scarabeus_iv\" repository=\"standard\" binary=\"ownpkg-1.0-1.{arch}.rpm\"/>\
                        </buildinfo>",
                        arch = host_arch()
                    ),
                )
                .with_get(&buildconfig_url(), "Macro: test 1\n");
            let client = ObsClient::with_credentials(
                &http,
                Credentials::Bearer {
                    token: "test-token".to_string(),
                },
            )
            .unwrap();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", host_arch());
            let err = vertical.build(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::UntrustedProject);
            assert!(err.to_string().contains("home:scarabeus_iv"));
            assert!(err.to_string().contains("cannot be determined"));
            assert!(err.to_string().contains("--trust-all-projects"));
        }

        #[test]
        fn build_with_trust_all_projects_and_nothing_foreign_records_nothing() {
            // The flag with no foreign dep changes nothing: the
            // build is green and the record lists no accepted
            // project.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let http = FakeHttp::new()
                .with_get(
                    &buildinfo_url(),
                    &format!(
                        "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                        <bdep name=\"glibc\" project=\"testproject\" repository=\"standard\" binary=\"glibc-2.38-1.{arch}.rpm\"/>\
                        <bdep name=\"ownpkg\" project=\"home:scarabeus_iv\" repository=\"standard\" binary=\"ownpkg-1.0-1.{arch}.rpm\"/>\
                        </buildinfo>",
                        arch = host_arch()
                    ),
                )
                .with_get(&buildconfig_url(), "Macro: test 1\n")
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{}/_repository/glibc-2.38-1.{}.rpm",
                        host_arch(),
                        host_arch()
                    ),
                    b"fake glibc rpm",
                )
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/home:scarabeus_iv/standard/{}/_repository/ownpkg-1.0-1.{}.rpm",
                        host_arch(),
                        host_arch()
                    ),
                    b"fake ownpkg rpm",
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.trust_all_projects = true;
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");
            assert!(record.trusted_foreign_projects.is_empty());
        }

        #[test]
        fn build_with_trust_all_projects_lists_only_the_foreign_projects() {
            // A mix of the caller's own home subtree, a foreign
            // home:, and a non-home: project: only the foreign one
            // is listed as accepted.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let http = FakeHttp::new()
                .with_get(
                    &buildinfo_url(),
                    &format!(
                        "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                        <bdep name=\"ownpkg\" project=\"home:scarabeus_iv:branches:foo\" repository=\"standard\" binary=\"ownpkg-1.0-1.{arch}.rpm\"/>\
                        <bdep name=\"glibc\" project=\"home:someoneelse\" repository=\"standard\" binary=\"glibc-2.38-1.{arch}.rpm\"/>\
                        <bdep name=\"make\" project=\"openSUSE:Factory\" repository=\"standard\" binary=\"make-4.4-1.{arch}.rpm\"/>\
                        </buildinfo>",
                        arch = host_arch()
                    ),
                )
                .with_get(&buildconfig_url(), "Macro: test 1\n")
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/home:scarabeus_iv:branches:foo/standard/{}/_repository/ownpkg-1.0-1.{}.rpm",
                        host_arch(),
                        host_arch()
                    ),
                    b"fake ownpkg rpm",
                )
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/home:someoneelse/standard/{}/_repository/glibc-2.38-1.{}.rpm",
                        host_arch(),
                        host_arch()
                    ),
                    b"fake glibc rpm",
                )
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/openSUSE:Factory/standard/{}/_repository/make-4.4-1.{}.rpm",
                        host_arch(),
                        host_arch()
                    ),
                    b"fake make rpm",
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.trust_all_projects = true;
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");
            assert_eq!(record.trusted_foreign_projects, ["home:someoneelse"]);
        }

        #[test]
        fn build_treats_a_foreign_branch_subproject_as_foreign() {
            // `home:someoneelse:branches:foo` is not the caller's
            // own subtree: without the flag it refuses, with the
            // flag it is accepted and listed.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let http = FakeHttp::new()
                .with_get(
                    &buildinfo_url(),
                    &format!(
                        "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                        <bdep name=\"glibc\" project=\"home:someoneelse:branches:foo\" repository=\"standard\" binary=\"glibc-2.38-1.{arch}.rpm\"/>\
                        </buildinfo>",
                        arch = host_arch()
                    ),
                )
                .with_get(&buildconfig_url(), "Macro: test 1\n")
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/home:someoneelse:branches:foo/standard/{}/_repository/glibc-2.38-1.{}.rpm",
                        host_arch(),
                        host_arch()
                    ),
                    b"fake glibc rpm",
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source.clone(), "testproject", "standard", host_arch());
            let err = vertical.build(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::UntrustedProject);
            assert!(err.to_string().contains("home:someoneelse:branches:foo"));
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.trust_all_projects = true;
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");
            assert_eq!(
                record.trusted_foreign_projects,
                ["home:someoneelse:branches:foo"]
            );
        }

        #[test]
        fn build_with_trust_all_projects_still_rejects_a_traversal_project() {
            // The flag overrides trust, not validation: a `..`
            // project still fails in `validate_dep`.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let http = FakeHttp::new()
                .with_get(
                    &buildinfo_url(),
                    &format!(
                        "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                        <bdep name=\"evil\" project=\"..\" repository=\"standard\" binary=\"evil-1.0-1.{arch}.rpm\"/>\
                        </buildinfo>",
                        arch = host_arch()
                    ),
                )
                .with_get(&buildconfig_url(), "Macro: test 1\n");
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.trust_all_projects = true;
            let err = vertical.build(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(err.to_string().contains("dependency project"));
        }

        #[test]
        fn build_allows_a_dep_without_a_project() {
            // A dep without a `project` carries no origin to
            // enumerate: it is a virtual provide or a not-installed
            // flag, not a package the build would install, so it
            // joins no trust set and must not refuse the build.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let http = FakeHttp::new()
                .with_get(
                    &buildinfo_url(),
                    &format!(
                        "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                        <bdep name=\"glibc\" binary=\"glibc-2.38-1.{arch}.rpm\"/>\
                        </buildinfo>",
                        arch = host_arch()
                    ),
                )
                .with_get(&buildconfig_url(), "Macro: test 1\n")
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{}/_repository/glibc-2.38-1.{}.rpm",
                        host_arch(),
                        host_arch()
                    ),
                    b"fake glibc rpm",
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", host_arch());
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");
        }

        #[test]
        fn build_trusts_a_foreign_home_project_with_trust_all_projects() {
            // `--trust-all-projects` overrides the foreign `home:`
            // refusal for the dep set the buildinfo enumerates: the
            // build proceeds, and the accepted project is recorded
            // in the build record and reported on the debug channel.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let http = FakeHttp::new()
                .with_get(
                    &buildinfo_url(),
                    &format!(
                        "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                        <bdep name=\"glibc\" project=\"home:someoneelse\" repository=\"standard\" binary=\"glibc-2.38-1.{arch}.rpm\"/>\
                        </buildinfo>",
                        arch = host_arch()
                    ),
                )
                .with_get(&buildconfig_url(), "Macro: test 1\n")
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/home:someoneelse/standard/{}/_repository/glibc-2.38-1.{}.rpm",
                        host_arch(),
                        host_arch()
                    ),
                    b"fake glibc rpm",
                );
            let client = client_with(&http);
            let (debug, lines) = Debug::captured(true, false);
            let vertical = vertical(&client, dir.path(), engine, podman).with_debug(debug);
            let source = write_source(dir.path(), "foo.spec");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.trust_all_projects = true;
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");
            assert_eq!(record.trusted_foreign_projects, ["home:someoneelse"]);
            let lines = lines.lock().expect("debug lines readable");
            assert!(
                lines.iter().any(|line| line.contains(
                    "local-build trust: accepting foreign home: project(s) via --trust-all-projects: home:someoneelse"
                )),
                "debug misses the trust override:\n{}",
                lines.join("\n")
            );
        }

        #[test]
        fn build_fails_closed_on_an_unresolvable_buildinfo() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let http = FakeHttp::new().with_get(
                &buildinfo_url(),
                "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"x86_64\"><error>unresolvable: DOES-NOT-EXIST</error></buildinfo>",
            );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", host_arch());
            let err = vertical.build(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ServerError);
            assert!(err.to_string().contains("DOES-NOT-EXIST"));
        }

        #[test]
        fn multibuild_requires_a_flavor() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            fs::write(
                source.join("_multibuild"),
                "<multibuild><package>flavor-a</package><package>flavor-b</package></multibuild>",
            )
            .unwrap();
            let req = BuildRequest::new(source.clone(), "testproject", "standard", host_arch());
            let err = vertical.build(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::MultibuildFlavorRequired);
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.flavor = Some("flavor-z".to_string());
            let err = vertical.build(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
        }

        #[test]
        fn stub_build_end_to_end_and_second_run_reuses_the_root() {
            let dir = TempDir::new();
            let (engine, argv_log) = stub_engine(dir.path());
            let (podman, podman_log) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", host_arch());

            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");
            assert_eq!(record.exit_code, 0);
            assert!(!record.rpms.is_empty());
            assert!(!record.root_reused);
            assert_eq!(
                record.image_digest.as_deref(),
                Some("sha256:fake-digest-1234")
            );
            assert_eq!(record.engine_version.as_deref(), Some("build stub 1.0"));
            assert_eq!(record.cache.pkg_misses, 4);
            assert_eq!(record.cache.pkg_hits, 0);
            assert!(record.log_path.exists());
            // The root carries the marker, the record, the rpmlist
            // (with the preinstall line) and the buildconfig.
            let root = &record.root_path;
            assert!(root.join(ROOT_MARKER).exists());
            assert!(root.join(BUILD_RECORD).exists());
            let rpmlist = fs::read_to_string(root.join(RPMLIST)).unwrap();
            assert!(rpmlist.contains("preinstall: pre"));
            assert!(rpmlist.contains("glibc-2.38-1."));
            assert!(root.join(BUILDCONFIG).exists());
            // The engine got the shape the existing tool uses.
            let argv = fs::read_to_string(&argv_log).unwrap();
            assert!(argv.contains("--vm-type=podman"));
            assert!(argv.contains("--sccache-uri="));
            assert!(argv.contains("--norootforbuild"));
            assert!(argv.contains("--changelog"));
            assert!(argv.contains("foo.spec"));
            // The image was pulled (absent at first use).
            let podman_calls = fs::read_to_string(&podman_log).unwrap();
            assert!(podman_calls.contains("pull"));
            // No crust: no temp files left anywhere in the cache.
            let mut stack = vec![dir.path().join("cache")];
            let mut crust = Vec::new();
            while let Some(d) = stack.pop() {
                for entry in fs::read_dir(d).unwrap() {
                    let entry = entry.unwrap();
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .starts_with(".geectl-tmp-")
                    {
                        crust.push(path);
                    }
                }
            }
            assert!(crust.is_empty(), "crust left behind: {crust:?}");

            // The second run reuses the root and hits the pkg cache.
            let record2 = vertical.build(&req).unwrap();
            assert_eq!(record2.verdict, "green");
            assert!(record2.root_reused);
            assert_eq!(record2.cache.pkg_hits, 4);
            assert_eq!(record2.cache.pkg_misses, 0);
            // The image was not pulled again: the sidecar records
            // the refresh, so the second build reuses the digest.
            let podman_calls = fs::read_to_string(&podman_log).unwrap();
            assert_eq!(
                podman_calls.matches("pull").count(),
                1,
                "podman calls:\n{podman_calls}"
            );
        }

        #[test]
        fn dep_from_another_repo_uses_its_own_repo() {
            // A dep resolved from a linked repo must be fetched
            // from (and cached under) its own repository, not the
            // build repo: the existing tool keys its cache and its
            // download URL by the dep's own
            // project/repository/repoarch.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let http = FakeHttp::new()
                .with_get(
                    &buildinfo_url(),
                    &format!(
                        "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                        <bdep name=\"glibc\" project=\"testproject\" repository=\"other-repo\" binary=\"glibc-2.38-1.{arch}.rpm\"/>\
                        </buildinfo>",
                    ),
                )
                .with_get(&buildconfig_url(), "Macro: test 1\n")
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/testproject/other-repo/{arch}/_repository/glibc-2.38-1.{arch}.rpm"
                    ),
                    b"fake glibc rpm",
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", arch);
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");
            assert!(dir
                .path()
                .join("cache")
                .join("pkg")
                .join("testproject")
                .join("other-repo")
                .join(arch)
                .join(format!("glibc-2.38-1.{arch}.rpm"))
                .exists());
            // Nothing was cached under the build repo.
            assert!(!dir
                .path()
                .join("cache")
                .join("pkg")
                .join("testproject")
                .join("standard")
                .exists());
            let rpmlist = fs::read_to_string(record.root_path.join(RPMLIST)).unwrap();
            assert!(rpmlist.contains("other-repo"));
        }

        #[test]
        fn rpmlist_lists_every_dep_for_a_spec_build() {
            // For a spec build the existing tool lists every dep
            // (noinstall included) as `name path`, with a
            // `noinstall:` / `installonly:` line when non-empty
            // and a `sysroot: ` prefix for sysroot deps.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let http = FakeHttp::new()
                .with_get(
                    &buildinfo_url(),
                    &format!(
                        "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                        <bdep name=\"glibc\" project=\"testproject\" repository=\"standard\" binary=\"glibc-2.38-1.{arch}.rpm\"/>\
                        <bdep name=\"helper\" project=\"testproject\" repository=\"standard\" binary=\"helper-1.0-1.{arch}.rpm\" noinstall=\"1\"/>\
                        <bdep name=\"syspkg\" project=\"testproject\" repository=\"standard\" binary=\"syspkg-1.0-1.{arch}.rpm\" installonly=\"1\"/>\
                        <bdep name=\"sysdep\" project=\"testproject\" repository=\"standard\" binary=\"sysdep-1.0-1.{arch}.rpm\" sysroot=\"1\"/>\
                        </buildinfo>",
                    ),
                )
                .with_get(&buildconfig_url(), "Macro: test 1\n")
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/glibc-2.38-1.{arch}.rpm"
                    ),
                    b"fake glibc rpm",
                )
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/helper-1.0-1.{arch}.rpm"
                    ),
                    b"fake helper rpm",
                )
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/syspkg-1.0-1.{arch}.rpm"
                    ),
                    b"fake syspkg rpm",
                )
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/sysdep-1.0-1.{arch}.rpm"
                    ),
                    b"fake sysdep rpm",
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", arch);
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");
            assert_eq!(record.cache.pkg_misses, 4);
            let rpmlist = fs::read_to_string(record.root_path.join(RPMLIST)).unwrap();
            // The noinstall dep is downloaded and listed like any
            // dep: the rpmlist references the file, so it must exist.
            assert!(rpmlist.contains("helper-1.0-1."));
            assert!(rpmlist.contains("noinstall: helper"));
            assert!(rpmlist.contains("installonly: syspkg"));
            assert!(rpmlist.contains("sysroot: sysdep "));
        }

        #[test]
        fn build_allows_the_callers_own_home_subproject() {
            // The caller's own home subtree (branches,
            // subprojects) is trusted exactly like the home
            // project itself.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let http = FakeHttp::new()
                .with_get(
                    &buildinfo_url(),
                    &format!(
                        "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                        <bdep name=\"ownpkg\" project=\"home:scarabeus_iv:branches:foo\" repository=\"standard\" binary=\"ownpkg-1.0-1.{arch}.rpm\"/>\
                        </buildinfo>",
                    ),
                )
                .with_get(&buildconfig_url(), "Macro: test 1\n")
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/home:scarabeus_iv:branches:foo/standard/{arch}/_repository/ownpkg-1.0-1.{arch}.rpm"
                    ),
                    b"fake ownpkg rpm",
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", arch);
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");
        }

        #[test]
        fn build_allows_deps_from_the_foreign_build_project_itself() {
            // The build project itself is trusted, exactly: a dep
            // resolving from the project being built is the build's
            // own output, not a foreign origin — even when that
            // project is a foreign `home:`. A subproject of it
            // would still be judged on its own; only the exact
            // project is excluded.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let http = FakeHttp::new()
                .with_get(
                    &format!(
                        "https://api.opensuse.org/build/home:someoneelse/standard/{arch}/foo/_buildinfo"
                    ),
                    &format!(
                        "<buildinfo project=\"home:someoneelse\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                        <bdep name=\"ownpkg\" project=\"home:someoneelse\" repository=\"standard\" binary=\"ownpkg-1.0-1.{arch}.rpm\"/>\
                        </buildinfo>",
                    ),
                )
                .with_get(
                    "https://api.opensuse.org/build/home:someoneelse/standard/_buildconfig",
                    "Macro: test 1\n",
                )
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/home:someoneelse/standard/{arch}/_repository/ownpkg-1.0-1.{arch}.rpm"
                    ),
                    b"fake ownpkg rpm",
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "home:someoneelse", "standard", arch);
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");
        }

        #[test]
        fn zero_timeout_is_invalid_input() {
            // A zero timeout is an instant timeout, never a build
            // — and never an instant shell.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.timeout_secs = 0;
            let err = vertical.build(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(setup.http.get_calls().is_empty());
            let req = BuildRequest::new(
                write_source(dir.path(), "foo.spec"),
                "testproject",
                "standard",
                host_arch(),
            );
            vertical.build(&req).unwrap();
            let err = vertical
                .build_shell(
                    "testproject",
                    "foo",
                    "standard",
                    host_arch(),
                    &None,
                    &["echo hi".to_string()],
                    0,
                )
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
        }

        #[test]
        fn concurrent_build_of_the_same_root_refuses() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", host_arch());
            // Hold the root's lock before the build starts.
            let root = dir
                .path()
                .join("cache")
                .join("build-roots")
                .join("testproject")
                .join("foo")
                .join(format!("standard-{}", host_arch()));
            fs::create_dir_all(&root).unwrap();
            let lock_file = File::options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(root.join(ROOT_LOCK))
                .unwrap();
            lock_file.try_lock_exclusive().unwrap();
            let err = vertical.build(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::BuildInProgress);
        }

        #[test]
        fn clean_wipe_keeps_the_root_lock() {
            // The --clean wipe once deleted the root's own lock
            // file, so the held flock sat on an unlinked inode and
            // a second build for the same root could lock a fresh
            // file and run concurrently — defeating the one-build-
            // per-root rule on exactly the path taken after every
            // failure. The wipe must preserve the lock file, so the
            // held flock still refuses a second acquirer afterwards.
            // (The fake HTTP client is RefCell-based, so the
            // vertical cannot move into a thread for a live
            // two-build race; this pins the exact invariant the
            // race depends on instead.)
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let root = dir
                .path()
                .join("cache")
                .join("build-roots")
                .join("testproject")
                .join("foo")
                .join(format!("standard-{}", host_arch()));
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join("stale-output"), b"stale").unwrap();
            // `BuildLock` carries no Debug, so the acquires go
            // through matches rather than unwrap/unwrap_err.
            let lock = match vertical.acquire_root_lock(&root, "foo") {
                Ok(lock) => lock,
                Err(e) => panic!("first lock acquire failed: {e}"),
            };
            vertical.wipe_root_contents(&root).unwrap();
            assert!(!root.join("stale-output").exists());
            assert!(root.join(ROOT_LOCK).exists());
            let err = match vertical.acquire_root_lock(&root, "foo") {
                Ok(_) => panic!("second lock acquire should have refused"),
                Err(e) => e,
            };
            assert_eq!(err.kind(), ErrorKind::BuildInProgress);
            drop(lock);
            // The lock file survives the wipe and is usable again.
            assert!(vertical.acquire_root_lock(&root, "foo").is_ok());
        }

        /// A valid marker for the wipe fixture coordinates.
        fn valid_marker() -> RootMarker {
            RootMarker {
                project: "testproject".to_string(),
                package: "foo".to_string(),
                repo: "standard".to_string(),
                arch: host_arch().to_string(),
                flavor: None,
                image_digest: None,
                engine_version: None,
                created_unix: 0,
                last_build_unix: 0,
                last_success_digest: None,
            }
        }

        /// The root for the wipe tests: `cache/build-roots/<project>/        /// <package>/<repo>-<arch>`, with the marker and one stale
        /// file in it. The source carries the spec the package name
        /// comes from.
        fn wipe_fixture(dir: &TempDir) -> (PathBuf, PathBuf) {
            let root = dir
                .path()
                .join("cache")
                .join("build-roots")
                .join("testproject")
                .join("foo")
                .join(format!("standard-{}", host_arch()));
            fs::create_dir_all(&root).unwrap();
            fs::write(
                root.join(ROOT_MARKER),
                serde_json::to_string(&valid_marker()).unwrap(),
            )
            .unwrap();
            fs::write(root.join("stale-output"), b"stale").unwrap();
            let source = write_source(dir.path(), "foo.spec");
            (root, source)
        }

        #[test]
        fn wipe_source_root_wipes_a_marked_root() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let (root, source) = wipe_fixture(&dir);
            let outcome = vertical
                .wipe_source_root(
                    &source,
                    "testproject",
                    "standard",
                    host_arch(),
                    &None,
                    false,
                )
                .unwrap();
            assert_eq!(outcome.root, root);
            assert!(outcome.existed);
            assert!(!outcome.dry_run);
            assert!(!root.join("stale-output").exists());
            assert!(!root.join(ROOT_MARKER).exists());
            // A wiped root is no longer a build root: a second wipe
            // refuses it.
            let err = vertical
                .wipe_source_root(
                    &source,
                    "testproject",
                    "standard",
                    host_arch(),
                    &None,
                    false,
                )
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NotAGeectlBuildRoot);
        }

        #[test]
        fn wipe_source_root_dry_run_wipes_nothing() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let (root, source) = wipe_fixture(&dir);
            let outcome = vertical
                .wipe_source_root(&source, "testproject", "standard", host_arch(), &None, true)
                .unwrap();
            assert_eq!(outcome.root, root);
            assert!(outcome.existed);
            assert!(outcome.dry_run);
            assert!(root.join("stale-output").exists());
            assert!(root.join(ROOT_MARKER).exists());
            // A dry run on an unknown root reports the absence
            // instead of failing the plan.
            let outcome = vertical
                .wipe_source_root(
                    &source,
                    "otherproject",
                    "standard",
                    host_arch(),
                    &None,
                    true,
                )
                .unwrap();
            assert!(!outcome.existed);
            assert!(outcome.dry_run);
        }

        #[test]
        fn wipe_named_root_wipes_by_path_and_reads_the_marker() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let (root, _) = wipe_fixture(&dir);
            // The marker carries the package: the lock file the wipe
            // leaves behind names it in the holder line.
            let marker = RootMarker {
                project: "testproject".to_string(),
                package: "foo".to_string(),
                repo: "standard".to_string(),
                arch: host_arch().to_string(),
                flavor: None,
                image_digest: None,
                engine_version: None,
                created_unix: 0,
                last_build_unix: 0,
                last_success_digest: None,
            };
            fs::write(
                root.join(ROOT_MARKER),
                serde_json::to_string(&marker).unwrap(),
            )
            .unwrap();
            let outcome = vertical.wipe_named_root(&root, false).unwrap();
            assert_eq!(outcome.root, root);
            assert!(outcome.existed);
            assert!(!root.join("stale-output").exists());
            let holder = fs::read_to_string(root.join(ROOT_LOCK)).unwrap();
            assert!(holder.contains("package=foo"), "holder: {holder}");
            // A corrupt marker refuses the wipe: the directory is
            // unverified, and no holder fallback resurrects it.
            // The stale file survives the refusal.
            fs::write(root.join(ROOT_MARKER), b"not json").unwrap();
            fs::write(root.join("stale-output"), b"stale").unwrap();
            let err = vertical.wipe_named_root(&root, false).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NotAGeectlBuildRoot);
            assert!(root.join("stale-output").exists());
            let err = vertical.wipe_named_root(&root, true).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NotAGeectlBuildRoot);
            // A root without the marker refuses the wipe.
            let err = vertical
                .wipe_named_root(
                    &dir.path().join("cache").join("build-roots").join("nobody"),
                    false,
                )
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NotAGeectlBuildRoot);
        }

        #[test]
        fn wipe_source_root_refuses_a_marker_that_does_not_match() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let (root, source) = wipe_fixture(&dir);
            // A marker for another project: the root is not the
            // one the coordinates name.
            let mut marker = valid_marker();
            marker.project = "otherproject".to_string();
            fs::write(
                root.join(ROOT_MARKER),
                serde_json::to_string(&marker).unwrap(),
            )
            .unwrap();
            let err = vertical
                .wipe_source_root(
                    &source,
                    "testproject",
                    "standard",
                    host_arch(),
                    &None,
                    false,
                )
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NotAGeectlBuildRoot);
            assert!(root.join("stale-output").exists());
            // The dry run refuses too: the plan must not claim an
            // unverified directory would be wiped.
            let err = vertical
                .wipe_source_root(&source, "testproject", "standard", host_arch(), &None, true)
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NotAGeectlBuildRoot);
            // Every coordinate field mismatches the same way.
            let mutators: [fn(&mut RootMarker); 4] = [
                |m| m.package = "bar".to_string(),
                |m| m.repo = "other".to_string(),
                |m| m.arch = "sparc64".to_string(),
                |m| m.flavor = Some("lib".to_string()),
            ];
            for mutate in mutators {
                let mut marker = valid_marker();
                mutate(&mut marker);
                fs::write(
                    root.join(ROOT_MARKER),
                    serde_json::to_string(&marker).unwrap(),
                )
                .unwrap();
                let err = vertical
                    .wipe_source_root(
                        &source,
                        "testproject",
                        "standard",
                        host_arch(),
                        &None,
                        false,
                    )
                    .unwrap_err();
                assert_eq!(err.kind(), ErrorKind::NotAGeectlBuildRoot);
            }
        }

        #[test]
        fn wipe_root_contents_unlinks_symlinks_without_traversing() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let (root, source) = wipe_fixture(&dir);
            // Symlink entries pointing outside the root: the wipe
            // unlinks the links, never the targets.
            let outside_file = dir.path().join("outside-file");
            fs::write(&outside_file, b"outside").unwrap();
            std::os::unix::fs::symlink(&outside_file, root.join("link-file")).unwrap();
            let outside_dir = dir.path().join("outside-dir");
            fs::create_dir_all(&outside_dir).unwrap();
            fs::write(outside_dir.join("kept"), b"kept").unwrap();
            std::os::unix::fs::symlink(&outside_dir, root.join("link-dir")).unwrap();
            // A symlink nested in a subdirectory, pointing outside.
            let sub = root.join("sub");
            fs::create_dir_all(&sub).unwrap();
            std::os::unix::fs::symlink(&outside_file, sub.join("nested-link")).unwrap();
            // The plan counts every entry once: the marker, the
            // stale file, both links, the subdirectory and the
            // nested link. Symlinks count as entries with no bytes.
            let marker_bytes = fs::read(root.join(ROOT_MARKER)).unwrap().len() as u64;
            let outcome = vertical
                .wipe_source_root(&source, "testproject", "standard", host_arch(), &None, true)
                .unwrap();
            assert_eq!(outcome.entries, 6);
            assert_eq!(outcome.bytes, marker_bytes + 5);
            vertical
                .wipe_source_root(
                    &source,
                    "testproject",
                    "standard",
                    host_arch(),
                    &None,
                    false,
                )
                .unwrap();
            assert!(fs::symlink_metadata(root.join("link-file")).is_err());
            assert!(fs::symlink_metadata(root.join("link-dir")).is_err());
            assert!(fs::symlink_metadata(sub.join("nested-link")).is_err());
            assert!(!root.join("sub").exists());
            assert!(!root.join(ROOT_MARKER).exists());
            assert_eq!(fs::read_to_string(&outside_file).unwrap(), "outside");
            assert_eq!(
                fs::read_to_string(outside_dir.join("kept")).unwrap(),
                "kept"
            );
        }

        #[test]
        fn wipe_named_root_refuses_roots_outside_the_cache() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            // A valid marker planted outside the cache: the marker
            // alone never proves a root.
            let outside = dir.path().join("outside");
            fs::create_dir_all(&outside).unwrap();
            fs::write(
                outside.join(ROOT_MARKER),
                serde_json::to_string(&valid_marker()).unwrap(),
            )
            .unwrap();
            fs::write(outside.join("kept"), b"kept").unwrap();
            let err = vertical.wipe_named_root(&outside, false).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(outside.join("kept").exists());
            // The same root reached through a symlink inside the
            // cache: containment judges the target, not the link.
            let link = dir.path().join("cache").join("build-roots").join("link");
            fs::create_dir_all(link.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            let err = vertical.wipe_named_root(&link, false).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(outside.join("kept").exists());
        }

        #[test]
        fn wipe_named_root_refuses_a_root_with_no_file_name() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let err = vertical.wipe_named_root(Path::new("/"), false).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
        }

        #[test]
        fn wipe_plan_counts_entries_and_bytes() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let (root, source) = wipe_fixture(&dir);
            let marker_bytes = fs::read(root.join(ROOT_MARKER)).unwrap().len() as u64;
            // The lock file stays, so it is not part of the plan.
            fs::write(root.join(ROOT_LOCK), b"pid=1 started=0 package=foo\n").unwrap();
            let outcome = vertical
                .wipe_source_root(&source, "testproject", "standard", host_arch(), &None, true)
                .unwrap();
            assert!(outcome.existed);
            // The marker plus the one stale file.
            assert_eq!(outcome.entries, 2);
            assert_eq!(outcome.bytes, marker_bytes + 5);
            assert!(root.join("stale-output").exists());
        }

        #[test]
        fn wipe_named_root_refuses_a_project_or_package_level_root() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let (root, _) = wipe_fixture(&dir);
            // One valid marker planted at the project level must
            // not buy a whole-subtree wipe.
            let project_dir = dir
                .path()
                .join("cache")
                .join("build-roots")
                .join("testproject");
            fs::write(
                project_dir.join(ROOT_MARKER),
                serde_json::to_string(&valid_marker()).unwrap(),
            )
            .unwrap();
            let err = vertical.wipe_named_root(&project_dir, false).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(root.join("stale-output").exists());
            // Same at the package level.
            let package_dir = project_dir.join("foo");
            fs::write(
                package_dir.join(ROOT_MARKER),
                serde_json::to_string(&valid_marker()).unwrap(),
            )
            .unwrap();
            let err = vertical.wipe_named_root(&package_dir, false).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(root.join("stale-output").exists());
        }

        #[test]
        fn wipe_refuses_a_lock_file_symlink() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let (root, _) = wipe_fixture(&dir);
            // A planted lock symlink to a victim file: the wipe
            // must refuse before the lock open can truncate it.
            let victim = dir.path().join("victim");
            fs::write(&victim, b"victim").unwrap();
            std::os::unix::fs::symlink(&victim, root.join(ROOT_LOCK)).unwrap();
            let err = vertical.wipe_named_root(&root, false).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert_eq!(fs::read_to_string(&victim).unwrap(), "victim");
            assert!(root.join("stale-output").exists());
        }

        #[test]
        fn wipe_refuses_a_marker_symlink() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let (root, _) = wipe_fixture(&dir);
            // A marker symlink to a valid marker elsewhere: the
            // wipe must refuse on borrowed identity.
            let elsewhere = dir.path().join("elsewhere-marker");
            fs::write(&elsewhere, serde_json::to_string(&valid_marker()).unwrap()).unwrap();
            fs::remove_file(root.join(ROOT_MARKER)).unwrap();
            std::os::unix::fs::symlink(&elsewhere, root.join(ROOT_MARKER)).unwrap();
            let err = vertical.wipe_named_root(&root, false).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NotAGeectlBuildRoot);
            assert!(root.join("stale-output").exists());
            let err = vertical.wipe_named_root(&root, true).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NotAGeectlBuildRoot);
        }

        #[test]
        fn shell_wipe_and_log_round_trip() {
            let dir = TempDir::new();
            let (engine, argv_log) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", host_arch());

            // Unknown roots refuse before any build.
            let err = vertical
                .build_shell(
                    "testproject",
                    "foo",
                    "standard",
                    host_arch(),
                    &None,
                    &["echo hi".to_string()],
                    60,
                )
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NotAGeectlBuildRoot);
            let err = vertical
                .build_wipe("testproject", "foo", "standard", host_arch(), &None)
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NotAGeectlBuildRoot);

            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");

            let log = vertical
                .build_log(
                    "testproject",
                    "foo",
                    "standard",
                    host_arch(),
                    &None,
                    Some(1),
                    false,
                )
                .unwrap();
            assert_eq!(log.verdict.as_deref(), Some("green"));
            assert_eq!(log.exit_code, Some(0));
            assert_eq!(log.lines.len(), 1);
            assert!(log.lines[0].contains("finished"));

            // An empty command list is invalid input, not an
            // interactive shell.
            let err = vertical
                .build_shell(
                    "testproject",
                    "foo",
                    "standard",
                    host_arch(),
                    &None,
                    &[],
                    60,
                )
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);

            // The shell is scripted and captured: its output rides
            // in the result, and it writes no log file (the build
            // log still reads the build's `finished` line).
            let shell = vertical
                .build_shell(
                    "testproject",
                    "foo",
                    "standard",
                    host_arch(),
                    &None,
                    &["echo hi".to_string()],
                    60,
                )
                .unwrap();
            assert_eq!(shell.exit_code, 0);
            assert!(shell.stdout.contains("shell output: echo hi"));
            // The argv log accumulates every engine invocation (the
            // build above carried --logfile=); only the shell's own
            // line — the last — must be free of it.
            let argv = fs::read_to_string(&argv_log).unwrap();
            let shell_argv = argv.lines().last().unwrap();
            assert!(shell_argv.contains("--shell-cmd echo hi"));
            assert!(!shell_argv.contains("--logfile="));
            // The shell passes the build's argv shape: the rpmlist,
            // the buildconfig and the arch from the root, exactly as
            // the existing tool does for a --shell-cmd run.
            assert!(shell_argv.contains("--rpmlist="));
            assert!(shell_argv.contains("--dist="));
            assert!(shell_argv.contains(&format!("--arch={}", host_arch())));
            let log = vertical
                .build_log(
                    "testproject",
                    "foo",
                    "standard",
                    host_arch(),
                    &None,
                    Some(1),
                    false,
                )
                .unwrap();
            assert!(log.lines[0].contains("finished"));

            vertical
                .build_wipe("testproject", "foo", "standard", host_arch(), &None)
                .unwrap();
            assert!(!record.root_path.join(ROOT_MARKER).exists());
            assert!(record.root_path.exists());
        }

        #[test]
        fn build_log_full_read_is_capped() {
            // A log larger than the read cap: the whole-log read
            // returns only the last 4 MiB of whole lines and
            // reports truncated, instead of reading the whole file.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let root = dir
                .path()
                .join("cache")
                .join("build-roots")
                .join("testproject")
                .join("foo")
                .join(format!("standard-{}", host_arch()));
            fs::create_dir_all(&root).unwrap();
            // build_log reads only the log file, so no marker is
            // needed here.
            let mut content = vec![b'x'; LOG_TAIL_CAP_BYTES as usize + 1024];
            content.extend_from_slice(b"\nfinished \"build foo.spec\"\n");
            fs::write(root.join(BUILD_LOG), &content).unwrap();
            let log = vertical
                .build_log(
                    "testproject",
                    "foo",
                    "standard",
                    host_arch(),
                    &None,
                    None,
                    false,
                )
                .unwrap();
            assert!(log.truncated);
            assert_eq!(log.lines, vec!["finished \"build foo.spec\"".to_string()]);
            // The returned text stays within the cap.
            let bytes: usize = log.lines.iter().map(|l| l.len() + 1).sum();
            assert!(bytes <= LOG_TAIL_CAP_BYTES as usize);

            // The tail arm reports truncation the same way.
            let log = vertical
                .build_log(
                    "testproject",
                    "foo",
                    "standard",
                    host_arch(),
                    &None,
                    Some(1),
                    false,
                )
                .unwrap();
            assert!(log.truncated);
            assert_eq!(log.lines, vec!["finished \"build foo.spec\"".to_string()]);

            // A short log reads whole and reports no truncation.
            fs::write(root.join(BUILD_LOG), b"first\nsecond\n").unwrap();
            let log = vertical
                .build_log(
                    "testproject",
                    "foo",
                    "standard",
                    host_arch(),
                    &None,
                    None,
                    false,
                )
                .unwrap();
            assert!(!log.truncated);
            assert_eq!(log.lines, vec!["first".to_string(), "second".to_string()]);
        }

        #[test]
        fn build_log_uncapped_read_returns_the_whole_log() {
            // A log larger than the read cap: an explicit uncapped
            // read returns the whole log uncapped and reports no
            // truncation, while the default read still caps.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let root = dir
                .path()
                .join("cache")
                .join("build-roots")
                .join("testproject")
                .join("foo")
                .join(format!("standard-{}", host_arch()));
            fs::create_dir_all(&root).unwrap();
            // Head line, then filler past the cap, then the last
            // line: only an uncapped read sees the head.
            let mut content = b"first line\n".to_vec();
            content.extend(std::iter::repeat_n(b'x', LOG_TAIL_CAP_BYTES as usize));
            content.extend_from_slice(b"\nfinished \"build foo.spec\"\n");
            fs::write(root.join(BUILD_LOG), &content).unwrap();
            let log = vertical
                .build_log(
                    "testproject",
                    "foo",
                    "standard",
                    host_arch(),
                    &None,
                    None,
                    true,
                )
                .unwrap();
            assert!(!log.truncated);
            assert_eq!(log.lines.first().unwrap(), "first line");
            assert_eq!(log.lines.last().unwrap(), "finished \"build foo.spec\"");
            let bytes: usize = log.lines.iter().map(|l| l.len() + 1).sum();
            assert!(bytes > LOG_TAIL_CAP_BYTES as usize);

            // The tail arm of an uncapped read reports no truncation
            // either; the default tail read still caps. The tail
            // lines are the same either way — the last line sits at
            // the end of the file — so this arm pins the flag, not
            // the read; the whole-log arm above proves the uncapped
            // read.
            let log = vertical
                .build_log(
                    "testproject",
                    "foo",
                    "standard",
                    host_arch(),
                    &None,
                    Some(1),
                    true,
                )
                .unwrap();
            assert!(!log.truncated);
            assert_eq!(log.lines, vec!["finished \"build foo.spec\"".to_string()]);
            let log = vertical
                .build_log(
                    "testproject",
                    "foo",
                    "standard",
                    host_arch(),
                    &None,
                    Some(1),
                    false,
                )
                .unwrap();
            assert!(log.truncated);
        }

        #[test]
        fn build_shell_joins_commands_into_one_shell_fragment() {
            // Two commands reach the engine as one `--shell-cmd`
            // string joined by ` && `: a shell fragment for the
            // container, not two host commands.
            let dir = TempDir::new();
            let (engine, argv_log) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", host_arch());
            vertical.build(&req).unwrap();
            let shell = vertical
                .build_shell(
                    "testproject",
                    "foo",
                    "standard",
                    host_arch(),
                    &None,
                    &["echo a".to_string(), "echo b".to_string()],
                    60,
                )
                .unwrap();
            assert_eq!(shell.exit_code, 0);
            assert!(shell.stdout.contains("shell output: echo a && echo b"));
            let argv = fs::read_to_string(&argv_log).unwrap();
            let shell_argv = argv.lines().last().unwrap();
            assert!(shell_argv.contains("--shell-cmd echo a && echo b"));
        }

        #[test]
        fn shell_refuses_a_root_without_a_build() {
            // A marker-only root has no rpmlist/buildconfig for
            // the shell to pass: the shell refuses rather than
            // running a build shape with missing inputs.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let root = dir
                .path()
                .join("cache")
                .join("build-roots")
                .join("testproject")
                .join("foo")
                .join(format!("standard-{}", host_arch()));
            fs::create_dir_all(&root).unwrap();
            let marker = RootMarker {
                project: "testproject".to_string(),
                package: "foo".to_string(),
                repo: "standard".to_string(),
                arch: host_arch().to_string(),
                flavor: None,
                image_digest: None,
                engine_version: None,
                created_unix: now_unix(),
                last_build_unix: now_unix(),
                last_success_digest: None,
            };
            fs::write(
                root.join(ROOT_MARKER),
                serde_json::to_string(&marker).unwrap(),
            )
            .unwrap();
            let err = vertical
                .build_shell(
                    "testproject",
                    "foo",
                    "standard",
                    host_arch(),
                    &None,
                    &["echo hi".to_string()],
                    60,
                )
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
        }

        #[test]
        fn prune_evicts_old_pkg_files_and_nothing_under_cap() {
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), false);
            let setup = Setup::new();
            let client = setup.client();
            let vertical =
                vertical(&client, dir.path(), engine, podman).with_cache_cap_bytes(1024 * 1024);
            // Nothing cached yet: the plan is empty.
            let plan = vertical.prune_plan().unwrap();
            assert!(plan.evicted.is_empty());

            // An old pkg file (mtime 2 days ago) over the cap.
            let pkg_dir = dir
                .path()
                .join("cache")
                .join("pkg")
                .join("testproject")
                .join("standard")
                .join(host_arch());
            fs::create_dir_all(&pkg_dir).unwrap();
            let pkg = pkg_dir.join("old-1.0-1.x86_64.rpm");
            fs::write(&pkg, vec![0u8; 2 * 1024 * 1024]).unwrap();
            let two_days_ago = SystemTime::now() - Duration::from_secs(2 * 86400);
            let file = File::open(&pkg).unwrap();
            file.set_times(std::fs::FileTimes::new().set_modified(two_days_ago))
                .unwrap();
            drop(file);
            // A fresh pkg file (within the grace period) never evicts.
            let fresh = pkg_dir.join("fresh-1.0-1.x86_64.rpm");
            fs::write(&fresh, vec![0u8; 1024]).unwrap();

            let plan = vertical.prune_plan().unwrap();
            assert_eq!(plan.evicted.len(), 1);
            assert_eq!(plan.evicted[0].kind, "pkg");
            assert!(plan.evicted[0].path.ends_with("old-1.0-1.x86_64.rpm"));
            assert!(plan.failures.is_empty());

            let report = vertical.prune_apply().unwrap();
            assert_eq!(report.evicted.len(), 1);
            assert!(!pkg.exists());
            assert!(fresh.exists());
            assert!(report.bytes_after <= report.low_water_bytes);
        }

        #[test]
        fn dry_run_without_an_engine_returns_the_plan() {
            // A dry run prints the plan and touches nothing, so it
            // does not need the engine installed: a missing engine
            // records as an unknown version instead of failing.
            let dir = TempDir::new();
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = LocalBuild::new(&client, dir.path().join("cache"))
                .with_engine(PathBuf::from("/nonexistent/build"))
                .with_podman(podman);
            let source = write_source(dir.path(), "foo.spec");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.dry_run = true;
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "dry-run");
            assert_eq!(record.engine_version, None);
            assert!(record.plan.is_some());
        }

        /// Serializes the tests that read or mutate PATH for the
        /// package-manager probes: a stub manager on PATH must not
        /// leak into a test expecting the real ones.
        static PATH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

        /// Prepends `dir` to PATH, restoring it on drop.
        struct RestorePath(Option<std::ffi::OsString>);

        impl RestorePath {
            fn prepend(dir: &Path) -> Self {
                let old = std::env::var_os("PATH");
                let mut paths =
                    std::env::split_paths(&old.clone().unwrap_or_default()).collect::<Vec<_>>();
                paths.insert(0, dir.to_path_buf());
                std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
                RestorePath(old)
            }
        }

        impl Drop for RestorePath {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(old) => std::env::set_var("PATH", old),
                    None => std::env::remove_var("PATH"),
                }
            }
        }

        #[test]
        fn engine_version_falls_back_to_the_package_version() {
            // A --version-refusing engine plus a stub rpm answering
            // -qf: the record carries the package version.
            let _path = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = TempDir::new();
            let engine = dir.path().join("silent-engine");
            write_script(&engine, "#!/bin/sh\nexit 1\n");
            write_script(
                &dir.path().join("rpm"),
                "#!/bin/sh\nif [ \"$1\" = \"-qf\" ]; then echo \"9.9-9\"; exit 0; fi\nexit 1\n",
            );
            let _restore = RestorePath::prepend(dir.path());
            let setup = Setup::new();
            let client = setup.client();
            let vertical = LocalBuild::new(&client, dir.path().join("cache"))
                .with_engine(engine)
                .with_podman(PathBuf::from("podman"));
            assert_eq!(vertical.engine_version().unwrap().as_deref(), Some("9.9-9"));
        }

        #[test]
        fn engine_version_falls_back_to_dpkg_when_rpm_refuses() {
            // rpm refuses and dpkg-query answers -S and -W: the record
            // carries the dpkg version. The rpm stub masks a real rpm
            // on PATH so the dpkg branch is the one that runs.
            let _path = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = TempDir::new();
            let engine = dir.path().join("silent-engine");
            write_script(&engine, "#!/bin/sh\nexit 1\n");
            write_script(&dir.path().join("rpm"), "#!/bin/sh\nexit 1\n");
            write_script(
                &dir.path().join("dpkg-query"),
                "#!/bin/sh\nif [ \"$1\" = \"-S\" ]; then echo \"obs-build: $2\"; exit 0; fi\nif [ \"$1\" = \"-W\" ]; then echo \"8.8-8\"; exit 0; fi\nexit 1\n",
            );
            let _restore = RestorePath::prepend(dir.path());
            let setup = Setup::new();
            let client = setup.client();
            let vertical = LocalBuild::new(&client, dir.path().join("cache"))
                .with_engine(engine)
                .with_podman(PathBuf::from("podman"));
            assert_eq!(vertical.engine_version().unwrap().as_deref(), Some("8.8-8"));
        }

        /// Whether a package manager on PATH owns `binary`: `Some(true)`
        /// when one answers for it, `Some(false)` when both ran (or
        /// are absent) and neither owns it, `None` when an ownership
        /// probe could not run (spawn failure or timeout) — no
        /// evidence either way, so the caller retries instead of
        /// concluding. A manager present without owning the binary
        /// (a Homebrew rpm, whose files are not in the rpm
        /// database) answers neither.
        fn manager_owns_binary(binary: &Path) -> Option<bool> {
            let binary = binary.to_str()?;
            let mut rpm = Command::new("rpm");
            rpm.args(["-qf", "--queryformat", "%{VERSION}-%{RELEASE}", binary]);
            let rpm_unknown =
                match run_with_timeout(&mut rpm, Duration::from_secs(PACKAGE_PROBE_TIMEOUT_SECS)) {
                    Ok(output)
                        if output.status.success()
                            && !String::from_utf8_lossy(&output.stdout).trim().is_empty() =>
                    {
                        return Some(true);
                    }
                    Ok(_) => false,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                    Err(_) => true,
                };
            let mut dpkg = Command::new("dpkg-query");
            dpkg.args(["-S", binary]);
            let owner_line = |stdout: &[u8]| {
                String::from_utf8_lossy(stdout)
                    .lines()
                    .any(|line| line.ends_with(&format!(": {binary}")))
            };
            match run_with_timeout(&mut dpkg, Duration::from_secs(PACKAGE_PROBE_TIMEOUT_SECS)) {
                Ok(output) if output.status.success() && owner_line(&output.stdout) => Some(true),
                Ok(_) if !rpm_unknown => Some(false),
                Ok(_) => None,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    if rpm_unknown {
                        None
                    } else {
                        Some(false)
                    }
                }
                Err(_) => None,
            }
        }

        /// The live-manager check, factored so the stub tests can
        /// drive it too. Callers hold PATH_LOCK: the probes read
        /// PATH, and a PATH-mutating probe test must not swap the
        /// managers under this one.
        fn check_engine_package_version() {
            // The package manager on the host answers for a binary it
            // owns: rpm on rpm systems, dpkg-query on Debian systems.
            // The check skips when neither is on PATH (the case on a
            // stock macOS host).
            let rpm = which(Path::new("rpm"));
            let dpkg = which(Path::new("dpkg-query"));
            let binary = match (rpm, dpkg) {
                (Some(path), _) => path,
                (None, Some(path)) => path,
                (None, None) => {
                    eprintln!("skipping: no rpm or dpkg-query on PATH");
                    return;
                }
            };
            // One probe attempt can fail transiently under parallel
            // load while the managers are healthy, so retry a few
            // times before concluding. Each attempt costs at most
            // five probe timeouts when the managers hang (three in
            // the version probe, two in the ownership probe); the
            // common case is milliseconds. When no manager owns the
            // binary (a Homebrew rpm, whose files are not in the
            // rpm database), there is nothing to answer: skip. A
            // manager that owns the binary while the probe still
            // answers nothing after retries is a broken probe: fail.
            // An ownership probe that could not run is no evidence
            // either way: retry instead of concluding. When ownership
            // could not be determined at all after retries, fail too:
            // a host whose managers never answer is broken, and
            // skipping would hide it.
            const ATTEMPTS: u32 = 3;
            const RETRY_DELAY: Duration = Duration::from_millis(100);
            let mut owned = false;
            for attempt in 0..ATTEMPTS {
                if engine_package_version(&binary).is_some_and(|version| !version.is_empty()) {
                    return;
                }
                match manager_owns_binary(&binary) {
                    Some(true) => {
                        owned = true;
                    }
                    Some(false) => {
                        eprintln!("skipping: no package manager owns {}", binary.display());
                        return;
                    }
                    None => {
                        eprintln!(
                            "ownership probe could not run for {}; retrying",
                            binary.display()
                        );
                    }
                }
                if attempt + 1 < ATTEMPTS {
                    std::thread::sleep(RETRY_DELAY);
                }
            }
            if owned {
                panic!(
                    "a package manager owns {} but the probe answered no version",
                    binary.display()
                );
            }
            panic!(
                "the probe answered no version for {} and ownership could not be determined",
                binary.display()
            );
        }

        #[test]
        fn engine_package_version_reads_the_owning_package() {
            let _path = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            check_engine_package_version();
        }

        #[test]
        fn engine_package_version_skips_when_no_manager_owns_the_binary() {
            // Hold the lock: the ownership check reads PATH, and a
            // PATH-mutating probe test must not swap the managers.
            let _path = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = TempDir::new();
            // rpm refuses -qf and no dpkg-query stub answers: no
            // manager owns the binary, so the check skips instead
            // of failing.
            write_script(&dir.path().join("rpm"), "#!/bin/sh\nexit 1\n");
            let _restore = RestorePath::prepend(dir.path());
            check_engine_package_version();
        }

        #[test]
        fn engine_package_version_retries_a_transient_probe_failure() {
            // Hold the lock: the ownership check reads PATH, and a
            // PATH-mutating probe test must not swap the managers.
            let _path = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = TempDir::new();
            // rpm refuses -qf exactly once, then answers: the first
            // probe attempt returns nothing while a manager owns
            // the binary, and the retry answers it.
            write_script(
                &dir.path().join("rpm"),
                &format!(
                    "#!/bin/sh\nif [ \"$1\" = \"-qf\" ]; then if [ -f \"{}/rpm-failed-once-{}\" ]; then echo \"9.9-9\"; exit 0; fi; touch \"{}/rpm-failed-once-{}\"; exit 1; fi\nexit 1\n",
                    dir.path().display(),
                    std::process::id(),
                    dir.path().display(),
                    std::process::id()
                ),
            );
            let _restore = RestorePath::prepend(dir.path());
            check_engine_package_version();
        }

        #[test]
        #[should_panic(expected = "but the probe answered no version")]
        fn engine_package_version_fails_when_a_manager_owns_the_binary_but_the_probe_answers_nothing(
        ) {
            // Hold the lock: the ownership check reads PATH, and a
            // PATH-mutating probe test must not swap the managers.
            let _path = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = TempDir::new();
            // rpm answers -qf with an empty version and dpkg-query
            // -S names an owner but -W refuses: the probe answers
            // nothing while a manager owns the binary.
            write_script(
                &dir.path().join("rpm"),
                "#!/bin/sh\nif [ \"$1\" = \"-qf\" ]; then exit 0; fi\nexit 1\n",
            );
            write_script(
                &dir.path().join("dpkg-query"),
                "#!/bin/sh\nif [ \"$1\" = \"-S\" ]; then echo \"obs-build: $2\"; exit 0; fi\nexit 1\n",
            );
            let _restore = RestorePath::prepend(dir.path());
            check_engine_package_version();
        }

        #[test]
        fn engine_version_is_unknown_when_nothing_answers() {
            // Hold the lock: the fallback reads PATH, and a
            // PATH-mutating probe test must not swap the managers.
            let _path = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            // A stub engine that refuses --version and is owned by no
            // package records an unknown version instead of failing.
            let dir = TempDir::new();
            let script = dir.path().join("silent-stub");
            write_script(&script, "#!/bin/sh\nexit 1\n");
            let setup = Setup::new();
            let client = setup.client();
            let vertical = LocalBuild::new(&client, dir.path().join("cache"))
                .with_engine(script)
                .with_podman(PathBuf::from("podman"));
            assert_eq!(vertical.engine_version().unwrap(), None);
        }

        #[test]
        fn build_times_out_on_a_sleeping_engine() {
            let dir = TempDir::new();
            // A stub engine that answers --version and then sleeps:
            // the build must time out instead of hanging.
            let script = dir.path().join("sleep-stub");
            write_script(
                &script,
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"build stub 1.0\"; exit 0; fi\nsleep 5\n",
            );
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), script, podman);
            let source = write_source(dir.path(), "foo.spec");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.timeout_secs = 1;
            let err = vertical.build(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::BuildTimeout);
            // The timeout message must not claim the container was
            // killed: the group kill does not stop it.
            assert!(err.context.contains("may still be running"));
        }

        #[test]
        fn build_pull_and_engine_share_one_timeout_budget() {
            // The timeout is one budget for the whole op: a pull
            // that takes 2 s of a 3 s budget leaves the engine 1 s,
            // not a fresh 3 s. Without the shared deadline this
            // build takes 2 + 3 = 5 s; with it, ~3 s.
            let dir = TempDir::new();
            let script = dir.path().join("sleep-stub");
            write_script(
                &script,
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"build stub 1.0\"; exit 0; fi\nsleep 30\n",
            );
            let slow_podman = dir.path().join("podman-slow-pull");
            write_script(
                &slow_podman,
                "#!/bin/sh\nif [ \"$1\" = \"pull\" ]; then sleep 2; exit 0; fi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then echo \"sha256:fake-digest-1234\"; exit 0; fi\nexit 0\n",
            );
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), script, slow_podman);
            let source = write_source(dir.path(), "foo.spec");
            let mut req = BuildRequest::new(source, "testproject", "standard", host_arch());
            req.timeout_secs = 3;
            req.refresh_image = true;
            let started = std::time::Instant::now();
            let err = vertical.build(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::BuildTimeout);
            assert!(started.elapsed() < Duration::from_secs(4));
        }

        /// A podman stub whose service runs take 2 s each: the
        /// service binary sleeps before writing its output, and
        /// `pre_checkin.sh` runs on the host as the stub's `bash`
        /// arm runs it. `image inspect` reports a digest, `pull`
        /// exits at once, `run` reports a fake container id.
        fn stub_slow_service_podman(dir: &Path, src: &Path) -> PathBuf {
            let script = dir.join("podman-slow-services");
            let body = r##"#!/bin/sh
if [ "$1" = "image" ] && [ "$2" = "inspect" ]; then echo "sha256:fake-digest-1234"; exit 0; fi
if [ "$1" = "pull" ]; then exit 0; fi
if [ "$1" = "run" ]; then echo "fake-container-id"; exit 0; fi
if [ "$1" = "stop" ]; then exit 0; fi
if [ "$1" = "exec" ]; then
  shift 1
  if [ "$1" = "-w" ]; then shift 2; fi
  shift 1
  # The install exec elevates via setpriv: skip the prefix to
  # reach the installed command.
  if [ "$1" = "setpriv" ]; then shift 4; fi
  if [ "$1" = "zypper" ]; then exit 0; fi
  if [ "$1" = "bash" ]; then cd "{SRC}" && bash ./pre_checkin.sh; exit $?; fi
  case "$1" in
    /usr/lib/obs/service/*)
      sleep 2
      name="${1#/usr/lib/obs/service/}"
      outdir=""
      prev=""
      for arg in "$@"; do
        if [ "$prev" = "--outdir" ]; then outdir="$arg"; fi
        prev="$arg"
      done
      outdir="{SRC}/$(basename "$outdir")"
      echo "output of $name" > "$outdir/$name.out"
      exit 0
      ;;
  esac
  exit 0
fi
exit 0
"##
            .replace("{SRC}", &src.display().to_string());
            write_script(&script, &body);
            script
        }

        #[test]
        fn build_leaf_phases_share_one_timeout_budget() {
            // One budget for the whole `build` leaf: the run-mode
            // service run, the precommit gate, and the engine build
            // share a single deadline. Without it each phase takes
            // a fresh full timeout, so the whole run can take ~3x
            // the documented budget: 4 s of services, 4 s of
            // precommit, then a fresh 10 s for the engine.
            let dir = TempDir::new();
            let script = dir.path().join("sleep-stub");
            write_script(
                &script,
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"build stub 1.0\"; exit 0; fi\nsleep 30\n",
            );
            let src = write_source(dir.path(), "foo.spec");
            fs::write(
                src.join("_service"),
                "<services><service name=\"alpha\"/><service name=\"beta\" mode=\"trylocal\"/></services>",
            )
            .unwrap();
            write_script(&src.join("pre_checkin.sh"), "#!/bin/sh\nsleep 2\n");
            let slow_podman = stub_slow_service_podman(dir.path(), &src);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), script, slow_podman);
            let mut service_req = ServiceRunRequest::new(src.clone(), ServiceRunMode::Run);
            service_req.timeout_secs = 10;
            let mut build_req =
                BuildRequest::new(src.clone(), "testproject", "standard", host_arch());
            build_req.timeout_secs = 10;
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let started = std::time::Instant::now();
            let err = (|| -> Result<BuildRecord, GeectlError> {
                vertical.service_run_with_deadline(&service_req, deadline)?;
                vertical.precommit_with_deadline(&src, 10, deadline, false)?;
                vertical.build_with_deadline(&build_req, deadline)
            })()
            .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::BuildTimeout);
            assert!(started.elapsed() < Duration::from_secs(13));
        }

        #[test]
        fn build_leaf_precommit_shares_the_timeout_budget() {
            // The precommit gate shares the leaf's deadline too:
            // its work here takes 10 s against a 10 s budget with
            // 6 s already spent on services, so the gate itself
            // times out at the deadline. A fresh deadline for the
            // gate would let it finish and push the run past the
            // bound.
            let dir = TempDir::new();
            let script = dir.path().join("sleep-stub");
            write_script(
                &script,
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"build stub 1.0\"; exit 0; fi\nsleep 30\n",
            );
            let src = write_source(dir.path(), "foo.spec");
            fs::write(
                src.join("_service"),
                "<services><service name=\"alpha\"/><service name=\"beta\" mode=\"trylocal\"/><service name=\"gamma\" mode=\"trylocal\"/></services>",
            )
            .unwrap();
            write_script(&src.join("pre_checkin.sh"), "#!/bin/sh\nsleep 6\n");
            let slow_podman = stub_slow_service_podman(dir.path(), &src);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), script, slow_podman);
            let mut service_req = ServiceRunRequest::new(src.clone(), ServiceRunMode::Run);
            service_req.timeout_secs = 10;
            let mut build_req =
                BuildRequest::new(src.clone(), "testproject", "standard", host_arch());
            build_req.timeout_secs = 10;
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let started = std::time::Instant::now();
            let err = (|| -> Result<BuildRecord, GeectlError> {
                vertical.service_run_with_deadline(&service_req, deadline)?;
                vertical.precommit_with_deadline(&src, 10, deadline, false)?;
                vertical.build_with_deadline(&build_req, deadline)
            })()
            .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::BuildTimeout);
            assert!(started.elapsed() < Duration::from_secs(13));
        }

        #[test]
        fn spawn_with_retry_retries_a_transient_spawn_failure() {
            // A momentary fork failure (EAGAIN under parallel load)
            // is retried, not surfaced: the third attempt spawns
            // fine, so the caller never sees the transient error.
            let attempts = std::cell::Cell::new(0);
            let mut child = spawn_with_retry(|| {
                let n = attempts.get() + 1;
                attempts.set(n);
                if n < 3 {
                    Err(std::io::Error::from(std::io::ErrorKind::WouldBlock))
                } else {
                    // /usr/bin/true, not /bin/true: the latter
                    // does not exist on macOS.
                    Command::new("/usr/bin/true").spawn()
                }
            })
            .unwrap();
            assert_eq!(attempts.get(), 3);
            assert!(child.wait().unwrap().success());
        }

        #[test]
        fn spawn_with_retry_gives_up_after_the_attempts() {
            // A spawn failure that never clears returns the original
            // error once the attempts run out: the first attempt's
            // WouldBlock, not the later attempts' OutOfMemory — and
            // not a success, and not an endless retry.
            let attempts = std::cell::Cell::new(0);
            let err = spawn_with_retry(|| {
                let n = attempts.get() + 1;
                attempts.set(n);
                Err::<std::process::Child, _>(std::io::Error::from(if n == 1 {
                    std::io::ErrorKind::WouldBlock
                } else {
                    std::io::ErrorKind::OutOfMemory
                }))
            })
            .unwrap_err();
            assert_eq!(attempts.get(), 3);
            assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);
        }

        #[test]
        fn spawn_with_retry_does_not_retry_a_permanent_failure() {
            // A missing binary is permanent: one attempt, same error,
            // no pointless retries.
            let attempts = std::cell::Cell::new(0);
            let err = spawn_with_retry(|| {
                attempts.set(attempts.get() + 1);
                Err::<std::process::Child, _>(std::io::Error::from(std::io::ErrorKind::NotFound))
            })
            .unwrap_err();
            assert_eq!(attempts.get(), 1);
            assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        }

        #[test]
        fn spawn_with_retry_retries_too_many_open_files() {
            // A full descriptor table (EMFILE under parallel load)
            // is retried like any other resource exhaustion: the
            // first two attempts fail, the third spawns fine, so
            // the caller never sees the transient error.
            let attempts = std::cell::Cell::new(0);
            let mut child = spawn_with_retry(|| {
                let n = attempts.get() + 1;
                attempts.set(n);
                if n < 3 {
                    Err(std::io::Error::from_raw_os_error(libc::EMFILE))
                } else {
                    // /usr/bin/true, not /bin/true: the latter
                    // does not exist on macOS.
                    Command::new("/usr/bin/true").spawn()
                }
            })
            .unwrap();
            assert_eq!(attempts.get(), 3);
            assert!(child.wait().unwrap().success());
        }

        #[test]
        fn spawn_with_retry_gives_up_after_fd_exhaustion() {
            // Descriptor exhaustion that never clears returns the
            // original error once the attempts run out: the first
            // attempt's EMFILE, not a success, and not an endless
            // retry.
            let attempts = std::cell::Cell::new(0);
            let err = spawn_with_retry(|| {
                let n = attempts.get() + 1;
                attempts.set(n);
                Err::<std::process::Child, _>(std::io::Error::from_raw_os_error(if n == 1 {
                    libc::EMFILE
                } else {
                    libc::ENFILE
                }))
            })
            .unwrap_err();
            assert_eq!(attempts.get(), 3);
            assert_eq!(err.raw_os_error(), Some(libc::EMFILE));
        }

        #[test]
        fn service_run_with_deadline_honours_a_short_deadline() {
            // A deadline the services cannot meet fails the run
            // with a timeout instead of running them to completion:
            // the passed deadline is the authority, not a fresh
            // budget. A method that minted its own deadline would
            // return Ok here.
            let dir = TempDir::new();
            let src = dir.path().join("source");
            fs::create_dir_all(&src).unwrap();
            fs::write(
                src.join("_service"),
                "<services><service name=\"alpha\"/></services>",
            )
            .unwrap();
            let slow_podman = stub_slow_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), slow_podman);
            let req = ServiceRunRequest::new(src.clone(), ServiceRunMode::Run);
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            let err = vertical
                .service_run_with_deadline(&req, deadline)
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::BuildTimeout);
        }

        #[test]
        fn resolve_image_refuses_a_pull_on_an_exhausted_budget() {
            // A pull due on an exhausted budget is a timeout, and
            // it names the budget — and the pull is never started.
            let dir = TempDir::new();
            let (podman, call_log) = stub_podman(dir.path(), false);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", host_arch());
            let past = std::time::Instant::now() - Duration::from_secs(1);
            let err = vertical
                .resolve_image(req.refresh_image, req.timeout_secs, past)
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::BuildTimeout);
            assert!(
                err.context.contains("already exhausted"),
                "got: {}",
                err.context
            );
            assert!(
                !fs::read_to_string(&call_log).unwrap().contains("pull"),
                "the pull must never be attempted"
            );
        }

        #[test]
        fn resolve_image_reports_a_pull_timeout_as_build_timeout() {
            // Regression (LOW-2 residual): a tiny-but-nonzero
            // remaining budget still ran the pull, which timed out
            // ("command timed out after 0s") and was mapped to
            // `ImageUnavailable`, although podman and the image
            // were fine. A pull timeout is a `BuildTimeout` naming
            // the budget, the same kind as an engine timeout.
            let dir = TempDir::new();
            let slow_podman = dir.path().join("podman-slow-pull");
            write_script(
                &slow_podman,
                "#!/bin/sh\nif [ \"$1\" = \"pull\" ]; then sleep 5; exit 0; fi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then echo \"sha256:fake-digest-1234\"; exit 0; fi\nexit 0\n",
            );
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), slow_podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", host_arch());
            let deadline = std::time::Instant::now() + Duration::from_millis(500);
            let err = vertical
                .resolve_image(req.refresh_image, req.timeout_secs, deadline)
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::BuildTimeout, "got: {err}");
            assert!(err.context.contains("timed out"), "got: {}", err.context);
        }

        #[test]
        fn resolve_dep_filename_picks_the_exact_package_name() {
            // A package that builds subpackages: the binary list
            // carries helper, helper-devel and helper-lang, plus a
            // source RPM — only the exact package name resolves.
            // No source package was sent, so the whole repository
            // is listed (`_repository`): the list must not assume
            // the binary name is the package.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let http = FakeHttp::new().with_get(
                &format!(
                    "https://api.opensuse.org/build/testproject/standard/{}/_repository",
                    host_arch()
                ),
                &format!(
                    "<binarylist><binary filename=\"helper-1.0-1.{arch}.rpm\" size=\"10\" mtime=\"1\"/><binary filename=\"helper-devel-1.0-1.{arch}.rpm\" size=\"10\" mtime=\"1\"/><binary filename=\"helper-lang-1.0-1.{arch}.rpm\" size=\"10\" mtime=\"1\"/><binary filename=\"helper-1.0-1.{arch}.src.rpm\" size=\"10\" mtime=\"1\"/></binarylist>",
                    arch = host_arch()
                ),
            );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let req = BuildRequest::new(
                dir.path().join("source"),
                "testproject",
                "standard",
                host_arch(),
            );
            let dep = BuildDep {
                name: "helper".to_string(),
                project: Some("testproject".to_string()),
                repository: None,
                package: None,
                binary: None,
                version: None,
                release: None,
                arch: None,
                epoch: None,
                hdrmd5: None,
                preinstall: false,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let filename = vertical.resolve_dep_filename(&req, &dep).unwrap();
            assert_eq!(filename, format!("helper-1.0-1.{}.rpm", host_arch()));
        }

        #[test]
        fn resolve_dep_filename_computes_the_noarch_name() {
            // A noarch dep with no `binary` attribute: the file name
            // is computed from version/release/arch, and the expected
            // name is written out literally — the test must not
            // compute it with the same format string the
            // implementation uses.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let req = BuildRequest::new(
                dir.path().join("source"),
                "testproject",
                "standard",
                host_arch(),
            );
            let dep = BuildDep {
                name: "foo-data".to_string(),
                project: Some("testproject".to_string()),
                repository: None,
                package: None,
                binary: None,
                version: Some("2.0".to_string()),
                release: Some("3.1".to_string()),
                arch: Some("noarch".to_string()),
                epoch: None,
                hdrmd5: None,
                preinstall: false,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let filename = vertical.resolve_dep_filename(&req, &dep).unwrap();
            assert_eq!(filename, "foo-data-2.0-3.1.noarch.rpm");
        }

        #[test]
        fn subpackage_dep_without_binary_attr_resolves_from_its_source_package() {
            // Real scheduler buildinfo carries no `binary`
            // attribute: the file name is computed locally from
            // version/release/arch, and the download URL keys on
            // the source package (`zlib`), never the binary name
            // (`libz1`) — the binary name as the package segment
            // 404s on the server. End to end: the build goes
            // green, and the RPM lands in the cache under the
            // dep's own project/repo/arch.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let http = FakeHttp::new()
                .with_get(
                    &buildinfo_url(),
                    &format!(
                        "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                        <bdep name=\"libz1\" project=\"testproject\" repository=\"standard\" package=\"zlib\" version=\"1.3.1\" release=\"1\" arch=\"{arch}\"/>\
                        </buildinfo>",
                    ),
                )
                .with_get(&buildconfig_url(), "Macro: test 1\n")
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{arch}/zlib/libz1-1.3.1-1.{arch}.rpm"
                    ),
                    b"fake libz1 rpm",
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", arch);
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");
            assert_eq!(record.cache.pkg_misses, 1);
            assert!(dir
                .path()
                .join("cache")
                .join("pkg")
                .join("testproject")
                .join("standard")
                .join(arch)
                .join(format!("libz1-1.3.1-1.{arch}.rpm"))
                .exists());
            let rpmlist = fs::read_to_string(record.root_path.join(RPMLIST)).unwrap();
            assert!(rpmlist.contains("libz1-1.3.1-1."));
        }

        #[test]
        fn dep_with_dotdot_project_is_refused() {
            // Server data is not trusted as a path component: a dep
            // whose project is `..` would escape the pkg cache dir
            // when its file is cached (and the API path when it is
            // downloaded), so the build refuses with `InvalidInput`
            // before any path is built from it.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let http = FakeHttp::new()
                .with_get(
                    &buildinfo_url(),
                    &format!(
                        "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                        <bdep name=\"foo\" project=\"..\" repository=\"standard\" package=\"foo\" version=\"1.0\" release=\"1\" arch=\"{arch}\"/>\
                        </buildinfo>",
                    ),
                )
                .with_get(&buildconfig_url(), "Macro: test 1\n");
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", arch);
            let err = vertical.build(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
        }

        #[test]
        fn dep_with_dotdot_list_filename_is_refused() {
            // The binary-list file name is server data, not a dep
            // field, so `validate_dep` never sees it: `ensure_cached`
            // must refuse a `/` or `..` in it itself, before any path
            // is built from it. The download path already refuses it
            // (`download_binary` validates the file name), so the
            // hole is the cache-hit path: a file already sitting at
            // the escaped location would be installed as the dep
            // without any download. The escape resolves only when
            // the intermediate directory exists, so plant it with
            // the file — and require `InvalidInput` from the sink.
            // Without the check the planted file is returned as the
            // cached dep.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let inter = dir
                .path()
                .join("cache")
                .join("pkg")
                .join("testproject")
                .join("standard")
                .join(arch)
                .join(format!("helper-1.0-1.{arch}"));
            fs::create_dir_all(&inter).unwrap();
            let escaped = dir
                .path()
                .join("cache")
                .join("pkg")
                .join("testproject")
                .join("standard")
                .join("evil.rpm");
            fs::write(&escaped, make_rpm(b"planted")).unwrap();
            let req = BuildRequest::new(dir.path().join("source"), "testproject", "standard", arch);
            let dep = BuildDep {
                name: "helper".to_string(),
                project: Some("testproject".to_string()),
                repository: Some("standard".to_string()),
                package: None,
                binary: None,
                version: None,
                release: None,
                arch: None,
                epoch: None,
                hdrmd5: None,
                preinstall: false,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let mut stats = CacheStats::default();
            let filename = format!("helper-1.0-1.{arch}/../../evil.rpm");
            let err = vertical
                .ensure_cached(&req, &dep, &filename, &mut stats)
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
        }

        #[test]
        fn ensure_cached_verifies_a_dod_binary_against_the_warmed_hdrmd5() {
            // buildinfo carries the download-on-demand placeholder
            // hdrmd5 for a binary nobody has fetched yet, not the
            // real header digest: the direct download 404s, the warm
            // fetches the file and reports its real hdrmd5, and the
            // retry downloads it. The cached file is verified against
            // the warmed digest — the placeholder would reject the
            // real file as corrupt.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let rpm = make_rpm(b"bash payload");
            let real_hdrmd5 = format!("{:x}", md5::compute(&rpm[112..]));
            let download_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/bash-5.3.20-11.1.{arch}.rpm"
            );
            let warm_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository?view=binaryversions&binary=bash"
            );
            let warmed_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/bash.rpm"
            );
            let http = FakeHttp::new()
                .with_get_bytes_error_sequence(&download_url, &[ErrorKind::NotFound])
                .with_get_bytes(&warmed_url, &rpm)
                .with_get(
                    &warm_url,
                    &format!(
                        "<binaryversionlist><binary name=\"bash.rpm\" hdrmd5=\"{real_hdrmd5}\" sizek=\"10\"/></binaryversionlist>"
                    ),
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let req = BuildRequest::new(dir.path().join("source"), "testproject", "standard", arch);
            let dep = BuildDep {
                name: "bash".to_string(),
                project: Some("testproject".to_string()),
                repository: Some("standard".to_string()),
                package: None,
                binary: None,
                version: None,
                release: None,
                arch: None,
                epoch: None,
                hdrmd5: Some(DOD_PLACEHOLDER_HDRMD5.to_string()),
                preinstall: true,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let mut stats = CacheStats::default();
            let filename = format!("bash-5.3.20-11.1.{arch}.rpm");
            let path = vertical
                .ensure_cached(&req, &dep, &filename, &mut stats)
                .unwrap();
            assert_eq!(fs::read(&path).unwrap(), rpm);
            assert_eq!(stats.pkg_misses, 1);
            assert_eq!(
                http.get_calls()
                    .iter()
                    .filter(|(url, _)| url == &warm_url)
                    .count(),
                1
            );
            assert_eq!(
                http.get_bytes_calls()
                    .iter()
                    .filter(|(url, _)| url == &download_url)
                    .count(),
                1
            );
            assert_eq!(
                http.get_bytes_calls()
                    .iter()
                    .filter(|(url, _)| url == &warmed_url)
                    .count(),
                1
            );
        }

        #[test]
        fn ensure_cached_does_not_reject_a_cached_dod_binary() {
            // A download-on-demand binary verified at download time
            // against the warmed hdrmd5: a later build finds it in
            // the cache and must not reject it for carrying the
            // placeholder hdrmd5 in buildinfo — there is no real
            // digest to check it against, so the hit verifies as ok.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let rpm = make_rpm(b"bash payload");
            let filename = format!("bash-5.3.20-11.1.{arch}.rpm");
            let cached = dir
                .path()
                .join("cache")
                .join("pkg")
                .join("testproject")
                .join("standard")
                .join(arch)
                .join(&filename);
            fs::create_dir_all(cached.parent().unwrap()).unwrap();
            fs::write(&cached, &rpm).unwrap();
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let req = BuildRequest::new(dir.path().join("source"), "testproject", "standard", arch);
            let dep = BuildDep {
                name: "bash".to_string(),
                project: Some("testproject".to_string()),
                repository: Some("standard".to_string()),
                package: None,
                binary: None,
                version: None,
                release: None,
                arch: None,
                epoch: None,
                hdrmd5: Some(DOD_PLACEHOLDER_HDRMD5.to_string()),
                preinstall: true,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let mut stats = CacheStats::default();
            let path = vertical
                .ensure_cached(&req, &dep, &filename, &mut stats)
                .unwrap();
            assert_eq!(fs::read(&path).unwrap(), rpm);
            assert_eq!(stats.pkg_hits, 1);
            assert!(http.get_bytes_calls().is_empty());
        }

        #[test]
        fn ensure_cached_rejects_a_dod_binary_when_the_warmed_hdrmd5_mismatches() {
            // The warm reports the digest of a different payload than
            // the downloaded file: the warmed digest is the expected
            // value, so the mismatch is a corrupt cache entry — the
            // file is deleted and the build fails, not silently kept.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let rpm = make_rpm(b"bash payload");
            let other = make_rpm(b"other payload");
            let other_hdrmd5 = format!("{:x}", md5::compute(&other[112..]));
            let download_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/bash-5.3.20-11.1.{arch}.rpm"
            );
            let warm_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository?view=binaryversions&binary=bash"
            );
            let warmed_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/bash.rpm"
            );
            let http = FakeHttp::new()
                .with_get_bytes_error_sequence(&download_url, &[ErrorKind::NotFound])
                .with_get_bytes(&warmed_url, &rpm)
                .with_get(
                    &warm_url,
                    &format!(
                        "<binaryversionlist><binary name=\"bash.rpm\" hdrmd5=\"{other_hdrmd5}\" sizek=\"10\"/></binaryversionlist>"
                    ),
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let req = BuildRequest::new(dir.path().join("source"), "testproject", "standard", arch);
            let dep = BuildDep {
                name: "bash".to_string(),
                project: Some("testproject".to_string()),
                repository: Some("standard".to_string()),
                package: None,
                binary: None,
                version: None,
                release: None,
                arch: None,
                epoch: None,
                hdrmd5: Some(DOD_PLACEHOLDER_HDRMD5.to_string()),
                preinstall: true,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let mut stats = CacheStats::default();
            let filename = format!("bash-5.3.20-11.1.{arch}.rpm");
            let err = vertical
                .ensure_cached(&req, &dep, &filename, &mut stats)
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::CacheCorrupt);
            let cached = dir
                .path()
                .join("cache")
                .join("pkg")
                .join("testproject")
                .join("standard")
                .join(arch)
                .join(&filename);
            assert!(!cached.exists());
        }

        #[test]
        fn ensure_cached_prefers_the_warmed_hdrmd5_over_buildinfo() {
            // The warmed digest is the authority: buildinfo agrees
            // with the downloaded file, but the warm reports a
            // different digest for the fetched file, so the file is
            // not the one the server fetched and is rejected.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let rpm = make_rpm(b"bash payload");
            let real_hdrmd5 = format!("{:x}", md5::compute(&rpm[112..]));
            let other = make_rpm(b"other payload");
            let other_hdrmd5 = format!("{:x}", md5::compute(&other[112..]));
            let download_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/bash-5.3.20-11.1.{arch}.rpm"
            );
            let warm_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository?view=binaryversions&binary=bash"
            );
            let warmed_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/bash.rpm"
            );
            let http = FakeHttp::new()
                .with_get_bytes_error_sequence(&download_url, &[ErrorKind::NotFound])
                .with_get_bytes(&warmed_url, &rpm)
                .with_get(
                    &warm_url,
                    &format!(
                        "<binaryversionlist><binary name=\"bash.rpm\" hdrmd5=\"{other_hdrmd5}\" sizek=\"10\"/></binaryversionlist>"
                    ),
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let req = BuildRequest::new(dir.path().join("source"), "testproject", "standard", arch);
            let dep = BuildDep {
                name: "bash".to_string(),
                project: Some("testproject".to_string()),
                repository: Some("standard".to_string()),
                package: None,
                binary: None,
                version: None,
                release: None,
                arch: None,
                epoch: None,
                hdrmd5: Some(real_hdrmd5),
                preinstall: true,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let mut stats = CacheStats::default();
            let filename = format!("bash-5.3.20-11.1.{arch}.rpm");
            let err = vertical
                .ensure_cached(&req, &dep, &filename, &mut stats)
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::CacheCorrupt);
        }

        #[test]
        fn effective_hdrmd5_prefers_the_warmed_digest_and_ignores_the_placeholder() {
            // The warmed digest wins over buildinfo, and the
            // placeholder is never a digest: a warmed placeholder
            // leaves a real buildinfo digest in place, and a dep
            // carrying only the placeholder verifies as absent. The
            // placeholder matches case-insensitively, like the digest
            // comparison in `verify_hdrmd5`.
            let mut dep = BuildDep {
                name: "bash".to_string(),
                project: Some("testproject".to_string()),
                repository: Some("standard".to_string()),
                package: None,
                binary: None,
                version: None,
                release: None,
                arch: None,
                epoch: None,
                hdrmd5: Some("abc".to_string()),
                preinstall: true,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            assert_eq!(LocalBuild::effective_hdrmd5(&dep, Some("def")), Some("def"));
            assert_eq!(
                LocalBuild::effective_hdrmd5(&dep, Some(DOD_PLACEHOLDER_HDRMD5)),
                Some("abc")
            );
            assert_eq!(LocalBuild::effective_hdrmd5(&dep, None), Some("abc"));
            dep.hdrmd5 = Some(DOD_PLACEHOLDER_HDRMD5.to_string());
            assert_eq!(LocalBuild::effective_hdrmd5(&dep, None), None);
            assert_eq!(LocalBuild::effective_hdrmd5(&dep, Some("def")), Some("def"));
            assert_eq!(
                LocalBuild::effective_hdrmd5(&dep, Some("D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0")),
                None
            );
        }

        #[test]
        fn ensure_cached_streams_an_over_cap_dep_to_disk() {
            // A dep RPM over the transport's 10 MiB body cap streams
            // to disk instead of failing: the direct download hits
            // the cap, the stream writes the file, the hdrmd5
            // verifies, and the cache holds the whole file — with no
            // temp file left behind.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let rpm = make_rpm(&vec![b'k'; crate::http::MAX_BODY_BYTES]);
            let real_hdrmd5 = format!("{:x}", md5::compute(&rpm[112..]));
            let download_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/kernel-obs-build-7.3.5-1.1.{arch}.rpm"
            );
            let http = FakeHttp::new()
                .with_get_bytes_cap_error(&download_url)
                .with_get_to_file(&download_url, &rpm);
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let req = BuildRequest::new(dir.path().join("source"), "testproject", "standard", arch);
            let dep = BuildDep {
                name: "kernel-obs-build".to_string(),
                project: Some("testproject".to_string()),
                repository: Some("standard".to_string()),
                package: None,
                binary: None,
                version: None,
                release: None,
                arch: None,
                epoch: None,
                hdrmd5: Some(real_hdrmd5),
                preinstall: true,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let mut stats = CacheStats::default();
            let filename = format!("kernel-obs-build-7.3.5-1.1.{arch}.rpm");
            let path = vertical
                .ensure_cached(&req, &dep, &filename, &mut stats)
                .unwrap();
            assert_eq!(fs::read(&path).unwrap(), rpm);
            assert_eq!(stats.pkg_misses, 1);
            assert_eq!(http.get_to_file_calls().len(), 1);
            let cache_dir = dir
                .path()
                .join("cache")
                .join("pkg")
                .join("testproject")
                .join("standard")
                .join(arch);
            for entry in fs::read_dir(&cache_dir).unwrap() {
                let name = entry.unwrap().file_name();
                assert!(
                    !name.to_string_lossy().starts_with(".geectl-tmp-"),
                    "temp file left in the cache dir: {name:?}"
                );
            }
        }

        #[test]
        fn ensure_cached_streamed_dep_with_mismatched_warmed_hdrmd5_is_rejected() {
            // The warm reports a digest that is not the streamed
            // file's: the streamed file is verified before it lands
            // in the cache, so the ensure fails `CacheCorrupt` and
            // neither the package nor the temp file is left behind.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let rpm = make_rpm(b"big payload");
            let download_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/big-1.0-1.{arch}.rpm"
            );
            let warm_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository?view=binaryversions&binary=big"
            );
            let streamed_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/big.rpm"
            );
            let http = FakeHttp::new()
                .with_get_bytes_error_sequence(&download_url, &[ErrorKind::NotFound])
                .with_get_bytes_cap_error(&streamed_url)
                .with_get_to_file(&streamed_url, &rpm)
                .with_get(
                    &warm_url,
                    "<binaryversionlist><binary name=\"big.rpm\" hdrmd5=\"00000000000000000000000000000000\" sizek=\"10\"/></binaryversionlist>",
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let req = BuildRequest::new(dir.path().join("source"), "testproject", "standard", arch);
            let dep = BuildDep {
                name: "big".to_string(),
                project: Some("testproject".to_string()),
                repository: Some("standard".to_string()),
                package: None,
                binary: None,
                version: None,
                release: None,
                arch: None,
                epoch: None,
                hdrmd5: Some(DOD_PLACEHOLDER_HDRMD5.to_string()),
                preinstall: true,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let mut stats = CacheStats::default();
            let filename = format!("big-1.0-1.{arch}.rpm");
            let err = vertical
                .ensure_cached(&req, &dep, &filename, &mut stats)
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::CacheCorrupt);
            let cache_dir = dir
                .path()
                .join("cache")
                .join("pkg")
                .join("testproject")
                .join("standard")
                .join(arch);
            assert!(!cache_dir.join(&filename).exists());
            for entry in fs::read_dir(&cache_dir).unwrap() {
                let name = entry.unwrap().file_name();
                assert!(
                    !name.to_string_lossy().starts_with(".geectl-tmp-"),
                    "temp file left in the cache dir: {name:?}"
                );
            }
        }

        #[test]
        fn download_dep_404_warms_the_binary_and_retries_once() {
            // A direct file download 404s for a download-on-demand
            // binary nobody has asked for: the daemon fetches
            // repository metadata only. The download asks the server
            // for the binary's versions — that listing makes the
            // server fetch it — and retries once, under the name the
            // listing reports (`glibc.rpm`): the backend stores the
            // fetched binary under that name, and a retry under the
            // full EVRA filename 404s again. Without the retry the
            // build fails on the first 404.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let download_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/glibc-2.38-1.{arch}.rpm"
            );
            let warm_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository?view=binaryversions&binary=glibc"
            );
            let warmed_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/glibc.rpm"
            );
            let http = FakeHttp::new()
                .with_get_bytes_error_sequence(
                    &download_url,
                    &[ErrorKind::NotFound, ErrorKind::NotFound],
                )
                .with_get_bytes(&warmed_url, b"fake glibc rpm")
                .with_get(
                    &warm_url,
                    "<binaryversionlist><binary name=\"glibc.rpm\" hdrmd5=\"abc\" sizek=\"10\"/></binaryversionlist>",
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let req = BuildRequest::new(dir.path().join("source"), "testproject", "standard", arch);
            let dep = BuildDep {
                name: "glibc".to_string(),
                project: Some("testproject".to_string()),
                repository: Some("standard".to_string()),
                package: None,
                binary: None,
                version: None,
                release: None,
                arch: None,
                epoch: None,
                hdrmd5: None,
                preinstall: true,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let DownloadedDep::InMemory(bytes, warmed) = vertical
                .download_dep(
                    &req,
                    &dep,
                    "testproject",
                    &format!("glibc-2.38-1.{arch}.rpm"),
                    dir.path(),
                )
                .unwrap()
            else {
                panic!("expected an in-memory download");
            };
            assert_eq!(bytes, b"fake glibc rpm".to_vec());
            assert_eq!(warmed.as_deref(), Some("abc"));
            assert_eq!(
                http.get_calls()
                    .iter()
                    .filter(|(url, _)| url == &warm_url)
                    .count(),
                1
            );
            assert_eq!(
                http.get_bytes_calls()
                    .iter()
                    .filter(|(url, _)| url == &download_url)
                    .count(),
                1
            );
            assert_eq!(
                http.get_bytes_calls()
                    .iter()
                    .filter(|(url, _)| url == &warmed_url)
                    .count(),
                1
            );
        }

        #[test]
        fn download_dep_warmed_name_404_falls_back_to_evra_filename() {
            // The warm reports the stored name, but a package in a
            // repodata module is stored under the full EVRA filename
            // instead: the retry under the reported name 404s, and
            // one final retry under the EVRA filename finds it.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let download_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/glibc-2.38-1.{arch}.rpm"
            );
            let warmed_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/glibc.rpm"
            );
            let warm_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository?view=binaryversions&binary=glibc"
            );
            let http = FakeHttp::new()
                .with_get_bytes_error_sequence(&download_url, &[ErrorKind::NotFound])
                .with_get_bytes(&download_url, b"fake glibc rpm")
                .with_get_bytes_error_sequence(&warmed_url, &[ErrorKind::NotFound])
                .with_get(
                    &warm_url,
                    "<binaryversionlist><binary name=\"glibc.rpm\" hdrmd5=\"abc\" sizek=\"10\"/></binaryversionlist>",
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let req = BuildRequest::new(dir.path().join("source"), "testproject", "standard", arch);
            let dep = BuildDep {
                name: "glibc".to_string(),
                project: Some("testproject".to_string()),
                repository: Some("standard".to_string()),
                package: None,
                binary: None,
                version: None,
                release: None,
                arch: None,
                epoch: None,
                hdrmd5: None,
                preinstall: true,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let DownloadedDep::InMemory(bytes, warmed) = vertical
                .download_dep(
                    &req,
                    &dep,
                    "testproject",
                    &format!("glibc-2.38-1.{arch}.rpm"),
                    dir.path(),
                )
                .unwrap()
            else {
                panic!("expected an in-memory download");
            };
            assert_eq!(bytes, b"fake glibc rpm".to_vec());
            assert_eq!(warmed.as_deref(), Some("abc"));
            assert_eq!(
                http.get_calls()
                    .iter()
                    .filter(|(url, _)| url == &warm_url)
                    .count(),
                1
            );
            assert_eq!(
                http.get_bytes_calls()
                    .iter()
                    .filter(|(url, _)| url == &download_url)
                    .count(),
                2
            );
            assert_eq!(
                http.get_bytes_calls()
                    .iter()
                    .filter(|(url, _)| url == &warmed_url)
                    .count(),
                1
            );
        }

        #[test]
        fn download_dep_404_twice_is_still_a_404_after_one_warm() {
            // A binary the repository does not carry stays a 404:
            // the warm listing reports it unavailable and the retry
            // 404s again. Exactly one warm and one retry: no loop.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let download_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/glibc-2.38-1.{arch}.rpm"
            );
            let warm_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository?view=binaryversions&binary=glibc"
            );
            let http = FakeHttp::new()
                .with_get_bytes_error_sequence(
                    &download_url,
                    &[ErrorKind::NotFound, ErrorKind::NotFound],
                )
                .with_get(
                    &warm_url,
                    "<binaryversionlist><binary name=\"glibc\" error=\"not available\"/></binaryversionlist>",
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let req = BuildRequest::new(dir.path().join("source"), "testproject", "standard", arch);
            let dep = BuildDep {
                name: "glibc".to_string(),
                project: Some("testproject".to_string()),
                repository: Some("standard".to_string()),
                package: None,
                binary: None,
                version: None,
                release: None,
                arch: None,
                epoch: None,
                hdrmd5: None,
                preinstall: true,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let err = vertical
                .download_dep(
                    &req,
                    &dep,
                    "testproject",
                    &format!("glibc-2.38-1.{arch}.rpm"),
                    dir.path(),
                )
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NotFound);
            assert_eq!(
                http.get_calls()
                    .iter()
                    .filter(|(url, _)| url == &warm_url)
                    .count(),
                1
            );
            assert_eq!(
                http.get_bytes_calls()
                    .iter()
                    .filter(|(url, _)| url == &download_url)
                    .count(),
                2
            );
        }

        #[test]
        fn download_dep_warm_failure_keeps_the_original_404() {
            // A warm that itself fails keeps the original 404: the
            // caller sees the missing binary, not the warm failure.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let download_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/glibc-2.38-1.{arch}.rpm"
            );
            let warm_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository?view=binaryversions&binary=glibc"
            );
            let http = FakeHttp::new()
                .with_get_bytes_error_sequence(&download_url, &[ErrorKind::NotFound])
                .with_get_error(&warm_url, ErrorKind::ServerError);
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let req = BuildRequest::new(dir.path().join("source"), "testproject", "standard", arch);
            let dep = BuildDep {
                name: "glibc".to_string(),
                project: Some("testproject".to_string()),
                repository: Some("standard".to_string()),
                package: None,
                binary: None,
                version: None,
                release: None,
                arch: None,
                epoch: None,
                hdrmd5: None,
                preinstall: true,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let err = vertical
                .download_dep(
                    &req,
                    &dep,
                    "testproject",
                    &format!("glibc-2.38-1.{arch}.rpm"),
                    dir.path(),
                )
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NotFound);
            assert_eq!(
                http.get_calls()
                    .iter()
                    .filter(|(url, _)| url == &warm_url)
                    .count(),
                1
            );
            assert_eq!(
                http.get_bytes_calls()
                    .iter()
                    .filter(|(url, _)| url == &download_url)
                    .count(),
                1
            );
        }

        #[test]
        fn download_dep_other_errors_do_not_warm() {
            // Only a 404 warms: a server error fails straight away
            // with no listing request.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let download_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/glibc-2.38-1.{arch}.rpm"
            );
            let http = FakeHttp::new().with_get_error(&download_url, ErrorKind::ServerError);
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let req = BuildRequest::new(dir.path().join("source"), "testproject", "standard", arch);
            let dep = BuildDep {
                name: "glibc".to_string(),
                project: Some("testproject".to_string()),
                repository: Some("standard".to_string()),
                package: None,
                binary: None,
                version: None,
                release: None,
                arch: None,
                epoch: None,
                hdrmd5: None,
                preinstall: true,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let err = vertical
                .download_dep(
                    &req,
                    &dep,
                    "testproject",
                    &format!("glibc-2.38-1.{arch}.rpm"),
                    dir.path(),
                )
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ServerError);
            assert!(http.get_calls().is_empty());
        }

        #[test]
        fn download_dep_cap_error_streams_instead_of_refusing() {
            // A direct download over the transport cap does not
            // refuse: the dep streams to a temp file in the given
            // directory, and the caller owns the temp file from there.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let rpm = make_rpm(b"streamed payload");
            let download_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/glibc-2.38-1.{arch}.rpm"
            );
            let http = FakeHttp::new()
                .with_get_bytes_cap_error(&download_url)
                .with_get_to_file(&download_url, &rpm);
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let req = BuildRequest::new(dir.path().join("source"), "testproject", "standard", arch);
            let dep = BuildDep {
                name: "glibc".to_string(),
                project: Some("testproject".to_string()),
                repository: Some("standard".to_string()),
                package: None,
                binary: None,
                version: None,
                release: None,
                arch: None,
                epoch: None,
                hdrmd5: None,
                preinstall: true,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let downloaded = vertical
                .download_dep(
                    &req,
                    &dep,
                    "testproject",
                    &format!("glibc-2.38-1.{arch}.rpm"),
                    dir.path(),
                )
                .unwrap();
            let DownloadedDep::Streamed(tmp, warmed) = downloaded else {
                panic!("expected a streamed download");
            };
            assert_eq!(warmed, None);
            assert!(tmp.starts_with(dir.path()));
            assert_eq!(fs::read(&tmp).unwrap(), rpm);
        }

        #[test]
        fn download_dep_stream_failure_deletes_the_temp_file() {
            // The stream fails mid-download: the error propagates,
            // and no partial temp file is left in the directory.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let download_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/glibc-2.38-1.{arch}.rpm"
            );
            let http = FakeHttp::new()
                .with_get_bytes_cap_error(&download_url)
                .with_get_to_file_error_after_write(
                    &download_url,
                    b"partial rpm",
                    ErrorKind::NetworkError,
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let req = BuildRequest::new(dir.path().join("source"), "testproject", "standard", arch);
            let dep = BuildDep {
                name: "glibc".to_string(),
                project: Some("testproject".to_string()),
                repository: Some("standard".to_string()),
                package: None,
                binary: None,
                version: None,
                release: None,
                arch: None,
                epoch: None,
                hdrmd5: None,
                preinstall: true,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let err = vertical
                .download_dep(
                    &req,
                    &dep,
                    "testproject",
                    &format!("glibc-2.38-1.{arch}.rpm"),
                    dir.path(),
                )
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NetworkError);
            for entry in fs::read_dir(dir.path()).unwrap() {
                let name = entry.unwrap().file_name();
                assert!(
                    !name.to_string_lossy().starts_with(".geectl-tmp-"),
                    "temp file left in the directory: {name:?}"
                );
            }
        }

        #[test]
        fn download_dep_stream_409_returns_without_retry() {
            // A 409 from the server reports `AlreadyExists` without
            // creating the file: the streamed download returns it
            // instead of retrying with the next temp name. Retrying
            // a 409 would end in a misleading exhaustion error, and
            // the retry loop must not spin on an error that never
            // created a file.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let download_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/glibc-2.38-1.{arch}.rpm"
            );
            let http = FakeHttp::new()
                .with_get_bytes_cap_error(&download_url)
                .with_get_to_file_error_after_write(
                    &download_url,
                    b"partial rpm",
                    ErrorKind::AlreadyExists,
                );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let req = BuildRequest::new(dir.path().join("source"), "testproject", "standard", arch);
            let dep = BuildDep {
                name: "glibc".to_string(),
                project: Some("testproject".to_string()),
                repository: Some("standard".to_string()),
                package: None,
                binary: None,
                version: None,
                release: None,
                arch: None,
                epoch: None,
                hdrmd5: None,
                preinstall: true,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let err = vertical
                .download_dep(
                    &req,
                    &dep,
                    "testproject",
                    &format!("glibc-2.38-1.{arch}.rpm"),
                    dir.path(),
                )
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::AlreadyExists);
            assert_eq!(http.get_to_file_calls().len(), 1);
            for entry in fs::read_dir(dir.path()).unwrap() {
                let name = entry.unwrap().file_name();
                assert!(
                    !name.to_string_lossy().starts_with(".geectl-tmp-"),
                    "temp file left in the directory: {name:?}"
                );
            }
        }

        #[test]
        fn download_dep_stream_temp_name_collision_retries_with_the_next_name() {
            // The first temp name is already taken by another
            // downloader: the streamed download retries with the
            // next name instead of failing, and leaves the file it
            // did not create alone.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let arch = host_arch();
            let rpm = make_rpm(b"streamed payload");
            let download_url = format!(
                "https://api.opensuse.org/build/testproject/standard/{arch}/_repository/glibc-2.38-1.{arch}.rpm"
            );
            let http = FakeHttp::new()
                .with_get_bytes_cap_error(&download_url)
                .with_get_to_file_error_sequence(&download_url, &[ErrorKind::AlreadyExists])
                .with_get_to_file(&download_url, &rpm);
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let req = BuildRequest::new(dir.path().join("source"), "testproject", "standard", arch);
            let dep = BuildDep {
                name: "glibc".to_string(),
                project: Some("testproject".to_string()),
                repository: Some("standard".to_string()),
                package: None,
                binary: None,
                version: None,
                release: None,
                arch: None,
                epoch: None,
                hdrmd5: None,
                preinstall: true,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let downloaded = vertical
                .download_dep(
                    &req,
                    &dep,
                    "testproject",
                    &format!("glibc-2.38-1.{arch}.rpm"),
                    dir.path(),
                )
                .unwrap();
            let DownloadedDep::Streamed(tmp, warmed) = downloaded else {
                panic!("expected a streamed download");
            };
            assert_eq!(warmed, None);
            assert_eq!(http.get_to_file_calls().len(), 2);
            assert_eq!(
                tmp.file_name().unwrap().to_string_lossy(),
                format!(".geectl-tmp-{}-1", std::process::id())
            );
            assert_eq!(fs::read(&tmp).unwrap(), rpm);
            // The collided name is still there: the retry must not
            // delete a file it did not create.
            let collided = dir
                .path()
                .join(format!(".geectl-tmp-{}-0", std::process::id()));
            assert!(collided.exists());
        }

        #[test]
        fn resolve_dep_filename_falls_back_to_the_list_for_empty_version() {
            // An empty attribute is no attribute: `version=""`
            // must not compute `name--.arch.rpm` and 404 the
            // download — the binary list of the whole repository
            // resolves it instead, exactly like a missing one.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let http = FakeHttp::new().with_get(
                &format!(
                    "https://api.opensuse.org/build/testproject/standard/{}/_repository",
                    host_arch()
                ),
                &format!(
                    "<binarylist><binary filename=\"helper-1.0-1.{arch}.rpm\" size=\"10\" mtime=\"1\"/><binary filename=\"helper-devel-1.0-1.{arch}.rpm\" size=\"10\" mtime=\"1\"/></binarylist>",
                    arch = host_arch()
                ),
            );
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let req = BuildRequest::new(
                dir.path().join("source"),
                "testproject",
                "standard",
                host_arch(),
            );
            let dep = BuildDep {
                name: "helper".to_string(),
                project: Some("testproject".to_string()),
                repository: None,
                package: None,
                binary: None,
                version: Some(String::new()),
                release: Some(String::new()),
                arch: Some(String::new()),
                epoch: None,
                hdrmd5: None,
                preinstall: false,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let filename = vertical.resolve_dep_filename(&req, &dep).unwrap();
            assert_eq!(filename, format!("helper-1.0-1.{}.rpm", host_arch()));
            // One empty attribute alone must fall back too: a
            // lone empty release must not compute
            // `helper-1.0-.arch.rpm`.
            let dep = BuildDep {
                name: "helper".to_string(),
                project: Some("testproject".to_string()),
                repository: None,
                package: None,
                binary: None,
                version: Some("1.0".to_string()),
                release: Some(String::new()),
                arch: Some(host_arch().to_string()),
                epoch: None,
                hdrmd5: None,
                preinstall: false,
                vminstall: false,
                runscripts: false,
                noinstall: false,
                repoarch: None,
                installonly: false,
                sysroot: false,
            };
            let filename = vertical.resolve_dep_filename(&req, &dep).unwrap();
            assert_eq!(filename, format!("helper-1.0-1.{}.rpm", host_arch()));
        }

        /// A single-dep buildinfo carrying an hdrmd5, with the dep's
        /// download canned to `download_bytes`.
        fn hdrmd5_setup(download_bytes: &[u8]) -> (TempDir, FakeHttp, String) {
            let dir = TempDir::new();
            let good = make_rpm(b"the real payload");
            let hdrmd5 = format!("{:x}", md5::compute(&good[112..]));
            let http = FakeHttp::new()
                .with_get(
                    &buildinfo_url(),
                    &format!(
                        "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"{arch}\">\
                        <bdep name=\"glibc\" project=\"testproject\" repository=\"standard\" binary=\"glibc-2.38-1.{arch}.rpm\" hdrmd5=\"{hdrmd5}\"/>\
                        </buildinfo>",
                        arch = host_arch()
                    ),
                )
                .with_get(&buildconfig_url(), "Macro: test 1\n")
                .with_get_bytes(
                    &format!(
                        "https://api.opensuse.org/build/testproject/standard/{}/_repository/glibc-2.38-1.{}.rpm",
                        host_arch(),
                        host_arch()
                    ),
                    download_bytes,
                );
            (dir, http, hdrmd5)
        }

        #[test]
        fn corrupt_cached_dep_is_redownloaded_then_refused() {
            // A cached dep that fails its hdrmd5 is re-downloaded
            // once; when the download fails the check too, the build
            // refuses with CacheCorrupt and the bad file is gone.
            let (dir, http, _) = hdrmd5_setup(b"also corrupt bytes");
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let cache_file = dir
                .path()
                .join("cache")
                .join("pkg")
                .join("testproject")
                .join("standard")
                .join(host_arch())
                .join(format!("glibc-2.38-1.{}.rpm", host_arch()));
            fs::create_dir_all(cache_file.parent().unwrap()).unwrap();
            fs::write(&cache_file, b"corrupt cached bytes").unwrap();
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", host_arch());
            let err = vertical.build(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::CacheCorrupt);
            assert!(!cache_file.exists());
        }

        #[test]
        fn corrupt_cached_dep_recovers_on_redownload() {
            // The same corrupt cache, but the download carries the
            // real RPM: the build proceeds and the cache now holds
            // the verified bytes.
            let good = make_rpm(b"the real payload");
            let (dir, http, _) = hdrmd5_setup(&good);
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), engine, podman);
            let cache_file = dir
                .path()
                .join("cache")
                .join("pkg")
                .join("testproject")
                .join("standard")
                .join(host_arch())
                .join(format!("glibc-2.38-1.{}.rpm", host_arch()));
            fs::create_dir_all(cache_file.parent().unwrap()).unwrap();
            fs::write(&cache_file, b"corrupt cached bytes").unwrap();
            let source = write_source(dir.path(), "foo.spec");
            let req = BuildRequest::new(source, "testproject", "standard", host_arch());
            let record = vertical.build(&req).unwrap();
            assert_eq!(record.verdict, "green");
            assert_eq!(fs::read(&cache_file).unwrap(), good);
        }

        #[test]
        fn prune_protects_locked_and_most_recent_roots() {
            // The real layout, not a fabricated one: each root here
            // is the most recent of its own (project, package) —
            // foo, bar and baz are different packages — so every
            // root stays protected and the cap is met from the pkg
            // file. The pkg file goes; every root stays; the
            // shortfall and the protected bytes are reported.
            // Grouping is by project+package: an older root of the
            // same package in another repo can evict — see
            // prune_evicts_the_older_root_of_the_same_package.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), false);
            let setup = Setup::new();
            let client = setup.client();
            let vertical =
                vertical(&client, dir.path(), engine, podman).with_cache_cap_bytes(1024 * 1024);
            let now = now_unix();
            let roots_base = dir.path().join("cache").join("build-roots");
            let mk_root = |package: &str, last_build: u64| -> PathBuf {
                let root = roots_base
                    .join("testproject")
                    .join(package)
                    .join(format!("standard-{}", host_arch()));
                fs::create_dir_all(&root).unwrap();
                fs::write(root.join("blob"), vec![0u8; 2 * 1024 * 1024]).unwrap();
                let marker = RootMarker {
                    project: "testproject".to_string(),
                    package: package.to_string(),
                    repo: "standard".to_string(),
                    arch: host_arch().to_string(),
                    flavor: None,
                    image_digest: None,
                    engine_version: None,
                    created_unix: last_build,
                    last_build_unix: last_build,
                    last_success_digest: None,
                };
                fs::write(
                    root.join(ROOT_MARKER),
                    serde_json::to_string(&marker).unwrap(),
                )
                .unwrap();
                root
            };
            let foo = mk_root("foo", now - 3 * 86400);
            let bar = mk_root("bar", now - 2 * 86400);
            let locked = mk_root("baz", now - 4 * 86400);
            // Hold the locked root's lock for the test.
            let lock_file = File::options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(locked.join(ROOT_LOCK))
                .unwrap();
            lock_file.try_lock_exclusive().unwrap();
            // An old pkg file over the cap.
            let pkg_dir = dir
                .path()
                .join("cache")
                .join("pkg")
                .join("testproject")
                .join("standard")
                .join(host_arch());
            fs::create_dir_all(&pkg_dir).unwrap();
            let pkg = pkg_dir.join("old-1.0-1.x86_64.rpm");
            fs::write(&pkg, vec![0u8; 2 * 1024 * 1024]).unwrap();
            let two_days_ago = SystemTime::now() - Duration::from_secs(2 * 86400);
            let file = File::open(&pkg).unwrap();
            file.set_times(std::fs::FileTimes::new().set_modified(two_days_ago))
                .unwrap();
            drop(file);

            let plan = vertical.prune_plan().unwrap();
            assert!(plan.ran);
            let evicted_paths: Vec<&str> = plan.evicted.iter().map(|i| i.path.as_str()).collect();
            assert_eq!(plan.evicted.len(), 1, "plan: {plan:?}");
            assert!(
                evicted_paths
                    .iter()
                    .any(|p| p.ends_with("old-1.0-1.x86_64.rpm")),
                "plan: {plan:?}"
            );
            // No root evicts: each is most-recent of its own
            // (project, package), and the locked one is protected
            // as in use.
            for root in [&foo, &bar, &locked] {
                assert!(
                    !evicted_paths.iter().any(|p| Path::new(p) == root),
                    "plan: {plan:?}"
                );
            }
            assert!(plan.shortfall_bytes > 0);
            // Protected: the locked root (2 MiB) plus the two
            // most-recent roots (2 MiB each) — just over 6 MiB,
            // the excess being the markers and lock files inside
            // the roots.
            assert!(plan.protected_bytes > 6 * 1024 * 1024, "plan: {plan:?}");
            assert!(plan.protected_bytes < 7 * 1024 * 1024, "plan: {plan:?}");

            let report = vertical.prune_apply().unwrap();
            assert_eq!(report.evicted.len(), 1);
            assert!(!pkg.exists());
            assert!(foo.exists());
            assert!(bar.exists());
            assert!(locked.exists());
        }

        #[test]
        fn prune_evicts_the_older_root_of_the_same_package() {
            // Grouping is by (project, package): an older root of a
            // package in another repo can evict once a newer root
            // of that same package exists — the next build reuses
            // the newest root of its package, not every root it
            // ever had. A root of a different package stays
            // protected as its own package's most recent.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), false);
            let setup = Setup::new();
            let client = setup.client();
            let vertical =
                vertical(&client, dir.path(), engine, podman).with_cache_cap_bytes(1024 * 1024);
            let now = now_unix();
            let roots_base = dir.path().join("cache").join("build-roots");
            let mk_root = |package: &str, repo: &str, last_build: u64| -> PathBuf {
                let root = roots_base
                    .join("testproject")
                    .join(package)
                    .join(format!("{repo}-{}", host_arch()));
                fs::create_dir_all(&root).unwrap();
                fs::write(root.join("blob"), vec![0u8; 2 * 1024 * 1024]).unwrap();
                let marker = RootMarker {
                    project: "testproject".to_string(),
                    package: package.to_string(),
                    repo: repo.to_string(),
                    arch: host_arch().to_string(),
                    flavor: None,
                    image_digest: None,
                    engine_version: None,
                    created_unix: last_build,
                    last_build_unix: last_build,
                    last_success_digest: None,
                };
                fs::write(
                    root.join(ROOT_MARKER),
                    serde_json::to_string(&marker).unwrap(),
                )
                .unwrap();
                root
            };
            let old_foo = mk_root("foo", "standard", now - 3 * 86400);
            let new_foo = mk_root("foo", "other-repo", now - 2 * 86400);
            let bar = mk_root("bar", "standard", now - 2 * 86400);

            let plan = vertical.prune_plan().unwrap();
            assert!(plan.ran);
            let evicted_paths: Vec<&str> = plan.evicted.iter().map(|i| i.path.as_str()).collect();
            assert_eq!(plan.evicted.len(), 1, "plan: {plan:?}");
            assert_eq!(plan.evicted[0].kind, "root");
            assert!(
                evicted_paths.iter().any(|p| Path::new(p) == old_foo),
                "plan: {plan:?}"
            );
            for root in [&new_foo, &bar] {
                assert!(
                    !evicted_paths.iter().any(|p| Path::new(p) == root),
                    "plan: {plan:?}"
                );
            }

            let report = vertical.prune_apply().unwrap();
            assert_eq!(report.evicted.len(), 1);
            assert!(!old_foo.exists());
            assert!(new_foo.exists());
            assert!(bar.exists());
        }

        #[test]
        fn store_project_package_parses_only_the_three_level_shape() {
            // The (project, package) of a store path is read for
            // the locked-root check: a legacy per-package store
            // (two levels, no root name) has no (project, package)
            // to read, so it is never attributed to the wrong
            // package — and neither is a deeper path.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let cache = dir.path().join("cache");
            assert_eq!(
                vertical.store_project_package(
                    &cache
                        .join("sccache")
                        .join("testproject")
                        .join("foo")
                        .join("standard-x86_64")
                ),
                Some(("testproject".to_string(), "foo".to_string()))
            );
            assert_eq!(
                vertical
                    .store_project_package(&cache.join("sccache").join("testproject").join("foo")),
                None
            );
            assert_eq!(
                vertical.store_project_package(
                    &cache
                        .join("sccache")
                        .join("a")
                        .join("b")
                        .join("c")
                        .join("d")
                ),
                None
            );
            assert_eq!(
                vertical.store_project_package(&cache.join("pkg").join("testproject")),
                None
            );
            // Bare `sccache/` has no (project, package) to read.
            assert_eq!(vertical.store_project_package(&cache.join("sccache")), None);
            // A trailing slash is normalized away by `components`,
            // so the store still parses.
            assert_eq!(
                vertical.store_project_package(
                    &cache
                        .join("sccache")
                        .join("testproject")
                        .join("foo")
                        .join("standard-x86_64/")
                ),
                Some(("testproject".to_string(), "foo".to_string()))
            );
            // A relative path never strips against the absolute
            // cache dir.
            assert_eq!(
                vertical
                    .store_project_package(Path::new("sccache/testproject/foo/standard-x86_64")),
                None
            );
        }

        #[test]
        fn cache_paths_encode_colon_in_obs_names() {
            // `:` is legal in an OBS name but not in a Windows path:
            // every cache path encodes it as `%3A`, on every
            // platform, so a `home:foo` build creates usable
            // directories on Windows too. The encoding is injective:
            // `home:foo` and `home_foo` get distinct roots.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let cache = dir.path().join("cache");
            let no_flavor: Option<String> = None;
            assert_eq!(
                vertical.root_path("home:foo", "foo", "standard", "x86_64", &no_flavor),
                cache
                    .join("build-roots")
                    .join("home%3Afoo")
                    .join("foo")
                    .join("standard-x86_64")
            );
            assert_eq!(
                vertical.sccache_dir("home:foo", "foo", "standard", "x86_64", &no_flavor),
                cache
                    .join("sccache")
                    .join("home%3Afoo")
                    .join("foo")
                    .join("standard-x86_64")
            );
            assert_eq!(
                vertical.pkg_lock_path("home:foo", "standard", "x86_64"),
                cache
                    .join("pkg")
                    .join("home%3Afoo")
                    .join("standard")
                    .join("x86_64.lock")
            );
            // The pkg dir helper encodes the project too:
            // `ensure_cached` builds its dir through `pkg_dir`, so
            // this pins the dir a `home:foo` dep's files land in.
            assert_eq!(
                vertical.pkg_dir("home:foo", "standard", "x86_64"),
                cache
                    .join("pkg")
                    .join("home%3Afoo")
                    .join("standard")
                    .join("x86_64")
            );
            // The root name encodes a colon in the flavor too.
            assert_eq!(
                LocalBuild::root_name("standard", "x86_64", &Some("a:b".to_string())),
                "standard-x86_64-a%3Ab"
            );
            // Names without `%` or `:` are unchanged.
            assert_eq!(LocalBuild::encode_path_segment("home_foo"), "home_foo");
            assert_eq!(LocalBuild::encode_path_segment("home:foo"), "home%3Afoo");
            // `%` itself is escaped, so a literal `%3A` in a name
            // cannot collide with an encoded `:`.
            assert_eq!(
                LocalBuild::encode_path_segment("home%3Afoo"),
                "home%253Afoo"
            );
            // The encoding round-trips.
            for name in ["home:foo", "home%3Afoo", "home_foo", "plain"] {
                assert_eq!(
                    LocalBuild::decode_path_segment(&LocalBuild::encode_path_segment(name)),
                    name
                );
            }
            // Distinct names stay distinct: `home:foo` and
            // `home_foo` are different projects with different
            // roots — the second build never reuses the first root.
            assert_ne!(
                vertical.root_path("home:foo", "foo", "standard", "x86_64", &no_flavor),
                vertical.root_path("home_foo", "foo", "standard", "x86_64", &no_flavor)
            );
        }

        #[test]
        fn encoded_locked_keys_match_store_paths() {
            // The eviction locked-root check compares the
            // (project, package) read from a store path against the
            // keys read from root markers: the marker keeps the real
            // name (`home:foo`) while the store path carries the
            // encoded one (`home%3Afoo`), so the marker side
            // encodes too — otherwise a locked root's store
            // evicts while the root is in use.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let no_flavor: Option<String> = None;
            let root = vertical.root_path("home:foo", "foo", "standard", "x86_64", &no_flavor);
            fs::create_dir_all(&root).unwrap();
            let marker = RootMarker {
                project: "home:foo".to_string(),
                package: "foo".to_string(),
                repo: "standard".to_string(),
                arch: "x86_64".to_string(),
                flavor: None,
                image_digest: None,
                engine_version: None,
                created_unix: 0,
                last_build_unix: 0,
                last_success_digest: None,
            };
            fs::write(
                root.join(ROOT_MARKER),
                serde_json::to_string(&marker).unwrap(),
            )
            .unwrap();
            let _lock = vertical.acquire_root_lock(&root, "foo").unwrap();
            let store = vertical.sccache_dir("home:foo", "foo", "standard", "x86_64", &no_flavor);
            assert_eq!(
                vertical.store_project_package(&store),
                Some(("home%3Afoo".to_string(), "foo".to_string()))
            );
            assert!(vertical
                .locked_root_keys()
                .contains(&("home%3Afoo".to_string(), "foo".to_string())));
        }

        #[test]
        fn colon_names_get_distinct_roots_with_distinct_markers() {
            // `home:foo` and `home_foo` are different projects: the
            // encoding gives them different roots, and the marker
            // check tells them apart — wiping one project's root
            // never touches the other's, and a marker written for
            // one never validates the other.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let no_flavor: Option<String> = None;
            let colon_root =
                vertical.root_path("home:foo", "foo", "standard", "x86_64", &no_flavor);
            let plain_root =
                vertical.root_path("home_foo", "foo", "standard", "x86_64", &no_flavor);
            assert_ne!(colon_root, plain_root);
            let marker = |project: &str| RootMarker {
                project: project.to_string(),
                package: "foo".to_string(),
                repo: "standard".to_string(),
                arch: "x86_64".to_string(),
                flavor: None,
                image_digest: None,
                engine_version: None,
                created_unix: 0,
                last_build_unix: 0,
                last_success_digest: None,
            };
            for (root, project) in [(&colon_root, "home:foo"), (&plain_root, "home_foo")] {
                fs::create_dir_all(root).unwrap();
                fs::write(
                    root.join(ROOT_MARKER),
                    serde_json::to_string(&marker(project)).unwrap(),
                )
                .unwrap();
            }
            let colon_expected = ExpectedMarker {
                project: "home:foo",
                package: "foo",
                repo: "standard",
                arch: "x86_64",
                flavor: &no_flavor,
            };
            let plain_expected = ExpectedMarker {
                project: "home_foo",
                package: "foo",
                repo: "standard",
                arch: "x86_64",
                flavor: &no_flavor,
            };
            assert!(LocalBuild::check_marker(&colon_root, Some(&colon_expected)).unwrap());
            assert!(LocalBuild::check_marker(&plain_root, Some(&plain_expected)).unwrap());
            assert!(LocalBuild::check_marker(&colon_root, Some(&plain_expected)).is_err());
            assert!(LocalBuild::check_marker(&plain_root, Some(&colon_expected)).is_err());
            // And the wipe itself: wiping `home_foo` removes only
            // its root's contents (the directory stays — the wipe
            // clears contents, not the root), and wiping `home:foo`
            // then removes its own.
            vertical
                .build_wipe("home_foo", "foo", "standard", "x86_64", &no_flavor)
                .unwrap();
            assert!(plain_root.is_dir());
            assert!(!plain_root.join(ROOT_MARKER).exists());
            assert!(colon_root.is_dir());
            assert!(colon_root.join(ROOT_MARKER).exists());
            vertical
                .build_wipe("home:foo", "foo", "standard", "x86_64", &no_flavor)
                .unwrap();
            assert!(colon_root.is_dir());
            assert!(!colon_root.join(ROOT_MARKER).exists());
        }

        #[test]
        fn pkg_lock_for_path_rejects_a_legacy_two_level_path() {
            // A legacy pkg/<repo>/<arch>/<file> path must not be
            // read as project/repo/arch: the lock it would take
            // guards nothing, and the file would be deleted
            // unguarded. The current four-level shape still takes
            // its lock.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let cache = dir.path().join("cache");
            let legacy = cache
                .join("pkg")
                .join("standard")
                .join(host_arch())
                .join("old-1.0-1.x86_64.rpm");
            assert!(vertical.pkg_lock_for_path(&legacy).is_none());
            assert!(!cache
                .join("pkg")
                .join("standard")
                .join(host_arch())
                .join("old-1.0-1.x86_64.rpm.lock")
                .exists());
            let current = cache
                .join("pkg")
                .join("testproject")
                .join("standard")
                .join(host_arch())
                .join("glibc-2.38-1.x86_64.rpm");
            assert!(vertical.pkg_lock_for_path(&current).is_some());
        }

        #[test]
        fn pkg_lock_for_path_matches_the_download_lock_for_a_colon_project() {
            // The lock `pkg_lock_for_path` takes for an encoded
            // `home:foo` file must be the same lock `acquire_pkg_lock`
            // takes for the real coordinates: the path components
            // are decoded before the lookup, or the lookup would
            // encode the `%` a second time and take a different
            // lock — a download in flight would not block the delete.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let cache = dir.path().join("cache");
            let current = cache
                .join("pkg")
                .join("home%3Afoo")
                .join("standard")
                .join(host_arch())
                .join("glibc-2.38-1.x86_64.rpm");
            // Hold the download lock for the real coordinates: the
            // lookup for the encoded file must contend with it.
            let held = vertical
                .acquire_pkg_lock("home:foo", "standard", host_arch())
                .unwrap();
            assert!(vertical.pkg_lock_for_path(&current).is_none());
            drop(held);
            assert!(vertical.pkg_lock_for_path(&current).is_some());
        }

        #[test]
        fn apply_eviction_plan_reports_pkg_shape_and_held_lock_apart() {
            // The apply arm must not report a legacy pkg file as
            // a concurrent download: a legacy path has no lock to
            // hold, so it is an unrecognised shape. A
            // current-layout file whose lock is held really is a
            // download in flight. Both are refused, neither is
            // deleted — only the diagnosis differs.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let cache = dir.path().join("cache");
            let legacy = cache
                .join("pkg")
                .join("standard")
                .join(host_arch())
                .join("old-1.0-1.x86_64.rpm");
            fs::create_dir_all(legacy.parent().unwrap()).unwrap();
            fs::write(&legacy, b"x").unwrap();
            let current = cache
                .join("pkg")
                .join("testproject")
                .join("standard")
                .join(host_arch())
                .join("glibc-2.38-1.x86_64.rpm");
            fs::create_dir_all(current.parent().unwrap()).unwrap();
            fs::write(&current, b"x").unwrap();
            // Hold the current file's project-repo-arch lock: the
            // apply run must see it as held, not as a shape
            // problem.
            let _held = vertical.pkg_lock_for_path(&current).unwrap();
            let mut report = EvictionReport {
                evicted: vec![
                    EvictedItem {
                        kind: "pkg".to_string(),
                        path: legacy.display().to_string(),
                        bytes: 1,
                        reason: "lru".to_string(),
                    },
                    EvictedItem {
                        kind: "pkg".to_string(),
                        path: current.display().to_string(),
                        bytes: 1,
                        reason: "lru".to_string(),
                    },
                ],
                ..Default::default()
            };
            vertical.apply_eviction_plan(&mut report);
            assert!(report.evicted.is_empty());
            assert_eq!(report.failures.len(), 2);
            assert!(report.failures[0].contains("unrecognised shape"));
            assert!(report.failures[1].contains("held by a download"));
            assert!(legacy.exists());
            assert!(current.exists());
        }

        #[test]
        fn apply_eviction_plan_holds_the_store_root_lock() {
            // The apply-time check for a sccache store must hold the
            // store's root lock across the delete, not re-probe a
            // snapshot: a build that locks the root after the plan
            // would otherwise lose its store under it. While the
            // root is locked the store survives with a failure
            // recorded; once the lock releases the same plan
            // deletes it.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let cache = dir.path().join("cache");
            let root_name = format!("standard-{}", host_arch());
            let root = cache
                .join("build-roots")
                .join("testproject")
                .join("foo")
                .join(&root_name);
            fs::create_dir_all(&root).unwrap();
            let store = cache
                .join("sccache")
                .join("testproject")
                .join("foo")
                .join(&root_name);
            fs::create_dir_all(&store).unwrap();
            fs::write(store.join("sccache.tar"), b"x").unwrap();
            let lock_file = File::options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(root.join(ROOT_LOCK))
                .unwrap();
            lock_file.try_lock_exclusive().unwrap();
            let item = || EvictedItem {
                kind: "sccache".to_string(),
                path: store.display().to_string(),
                bytes: 1,
                reason: "lru".to_string(),
            };
            let mut report = EvictionReport {
                evicted: vec![item()],
                ..Default::default()
            };
            vertical.apply_eviction_plan(&mut report);
            assert!(report.evicted.is_empty());
            assert_eq!(report.failures.len(), 1);
            assert!(report.failures[0].contains("locked build root"));
            assert!(store.exists());
            drop(lock_file);
            let report = apply_one_item_soon(&vertical, item());
            assert_eq!(report.evicted.len(), 1);
            assert!(report.failures.is_empty());
            assert!(!store.exists());
        }

        #[test]
        fn apply_eviction_plan_deletes_the_root_and_its_lock_file() {
            // A root delete removes the whole directory, lock file
            // included — and a locked root survives: the lock file
            // goes last, so a build that slips in between the
            // unlink and the rmdir makes the delete fail instead
            // of running inside a half-deleted root. Afterwards a
            // build can lock a fresh root at the same path.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let cache = dir.path().join("cache");
            let root = cache
                .join("build-roots")
                .join("testproject")
                .join("foo")
                .join(format!("standard-{}", host_arch()));
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join("blob"), b"x").unwrap();
            let lock_file = File::options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(root.join(ROOT_LOCK))
                .unwrap();
            lock_file.try_lock_exclusive().unwrap();
            let item = || EvictedItem {
                kind: "root".to_string(),
                path: root.display().to_string(),
                bytes: 1,
                reason: "lru".to_string(),
            };
            let mut report = EvictionReport {
                evicted: vec![item()],
                ..Default::default()
            };
            vertical.apply_eviction_plan(&mut report);
            assert!(report.evicted.is_empty());
            assert_eq!(report.failures.len(), 1);
            assert!(root.exists());
            drop(lock_file);
            let report = apply_one_item_soon(&vertical, item());
            assert_eq!(report.evicted.len(), 1);
            assert!(report.failures.is_empty());
            assert!(!root.exists());
            assert!(!root.join(ROOT_LOCK).exists());
            assert!(vertical.acquire_root_lock(&root, "foo").is_ok());
        }

        #[test]
        fn build_wipe_refuses_a_locked_root_with_build_root_in_use() {
            // A destructive op refused while a build holds the root
            // is `BuildRootInUse`, not a second build
            // (`BuildInProgress`): the caller asked to wipe, not to
            // build.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman);
            let root = dir
                .path()
                .join("cache")
                .join("build-roots")
                .join("testproject")
                .join("foo")
                .join(format!("standard-{}", host_arch()));
            fs::create_dir_all(&root).unwrap();
            fs::write(
                root.join(ROOT_MARKER),
                serde_json::to_string(&valid_marker()).unwrap(),
            )
            .unwrap();
            let lock_file = File::options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(root.join(ROOT_LOCK))
                .unwrap();
            lock_file.try_lock_exclusive().unwrap();
            let err = vertical
                .build_wipe("testproject", "foo", "standard", host_arch(), &None)
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::BuildRootInUse);
        }

        #[test]
        fn podman_digest_treats_none_as_absent() {
            // Podman prints `<none>` when the image has no digest: that
            // is "absent", not a digest to record and protect — a build
            // would otherwise never pull.
            let dir = TempDir::new();
            let (engine, _) = stub_engine(dir.path());
            let (podman, _) = stub_podman(dir.path(), true);
            let setup = Setup::new();
            let client = setup.client();
            let vertical = vertical(&client, dir.path(), engine, podman.clone());
            let none_podman = dir.path().join("podman-none");
            write_script(
                &none_podman,
                "#!/bin/sh\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then echo \"<none>\"; exit 0; fi\nexit 0\n",
            );
            assert_eq!(vertical.podman_digest(&none_podman).unwrap(), None);
            // And a real digest still reads.
            assert_eq!(
                vertical.podman_digest(&podman).unwrap().as_deref(),
                Some("sha256:fake-digest-1234")
            );
        }

        #[test]
        fn run_with_timeout_kills_a_hung_command() {
            // The probe deadline is the point: a hung command returns
            // `TimedOut` promptly instead of blocking the caller.
            let mut cmd = Command::new("sleep");
            cmd.arg("30");
            let err = run_with_timeout(&mut cmd, Duration::from_millis(100)).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
            // And a fast command returns its output.
            let mut cmd = Command::new("echo");
            cmd.arg("hi");
            let output = run_with_timeout(&mut cmd, Duration::from_secs(10)).unwrap();
            assert!(output.status.success());
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hi");
        }

        #[test]
        fn run_with_timeout_returns_despite_a_pipe_holding_grandchild() {
            // A child that exits while a grandchild holds its stdout
            // pipe open: the drain never sees EOF, so an unbounded
            // join would hang the caller until the grandchild exits
            // (30 s). The bounded join returns the child's output
            // after the drain timeout instead.
            let dir = TempDir::new();
            let pidfile = dir.path().join("grandchild.pid");
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(format!(
                "sleep 30 >&1 & echo $! > \"{}\"; echo done",
                pidfile.display()
            ));
            let started = std::time::Instant::now();
            let output = run_with_timeout(&mut cmd, Duration::from_secs(30)).unwrap();
            assert!(output.status.success());
            // The partial buffer is the child's whole output: the
            // drain already read it; only EOF was missing.
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "done");
            assert!(started.elapsed() < Duration::from_secs(10));
            // Reap the grandchild so it does not outlive the test.
            if let Ok(pid) = fs::read_to_string(&pidfile) {
                if let Ok(pid) = pid.trim().parse::<u32>() {
                    let _ = Command::new("kill")
                        .args(["-KILL", &pid.to_string()])
                        .status();
                }
            }
        }

        /// A stub podman for service runs: image inspect and pull
        /// as `stub_podman`, plus `run -d` (a fake container id),
        /// `exec` (the `-w /work` prefix is skipped; zypper
        /// succeeds; a service binary writes a canned `<name>.out`
        /// into its `--outdir`, except `failme`, which fails;
        /// `bash ./pre_checkin.sh` runs the real script in the
        /// source directory; `spec-cleaner` appends a marker to
        /// the spec), and `stop`.
        fn stub_service_podman(dir: &Path, src: &Path) -> (PathBuf, PathBuf) {
            let script = dir.join("podman-service-stub");
            let call_log = dir.join("podman-service-calls.log");
            let body = r##"#!/bin/sh
echo "$@" >> "{CALL_LOG}"
if [ "$1" = "image" ] && [ "$2" = "inspect" ]; then
  case "$4" in
    *Digest*) echo "sha256:fake-digest-1234" ;;
    *Size*) echo "123456789" ;;
  esac
  exit 0
fi
if [ "$1" = "pull" ]; then exit 0; fi
if [ "$1" = "run" ]; then echo "fake-container-id"; exit 0; fi
if [ "$1" = "stop" ]; then exit 0; fi
if [ "$1" = "exec" ]; then
  shift 1
  if [ "$1" = "-w" ]; then shift 2; fi
  shift 1
  # The install exec elevates via setpriv: skip the prefix to
  # reach the installed command.
  if [ "$1" = "setpriv" ]; then shift 4; fi
  if [ "$1" = "zypper" ]; then exit 0; fi
  if [ "$1" = "bash" ]; then cd "{SRC}" && bash ./pre_checkin.sh; exit $?; fi
  if [ "$1" = "spec-cleaner" ]; then
    for last; do true; done
    echo "# spec-cleaner ran" >> "{SRC}/$last"
    exit 0
  fi
  case "$1" in
    /usr/lib/obs/service/*)
      name="${1#/usr/lib/obs/service/}"
      if [ "$name" = "failme" ]; then echo "boom: service failed" >&2; exit 1; fi
      if [ "$name" = "slowpoke" ]; then sleep 5; fi
      outdir=""
      prev=""
      for arg in "$@"; do
        if [ "$prev" = "--outdir" ]; then outdir="$arg"; fi
        prev="$arg"
      done
      # The outdir arrives as the container path (/work/...): the
      # stub runs on the host, so the file lands in the source
      # directory's outdir of the same name.
      outdir="{SRC}/$(basename "$outdir")"
      echo "output of $name" > "$outdir/$name.out"
      exit 0
      ;;
  esac
  exit 0
fi
exit 0
"##
            .replace("{CALL_LOG}", &call_log.display().to_string())
            .replace("{SRC}", &src.display().to_string());
            write_script(&script, &body);
            (script, call_log)
        }

        fn service_defs() -> Vec<ServiceDef> {
            vec![
                ServiceDef {
                    name: "plain".to_string(),
                    mode: String::new(),
                    params: vec![],
                    comments: vec![],
                },
                ServiceDef {
                    name: "manualsvc".to_string(),
                    mode: "manual".to_string(),
                    params: vec![],
                    comments: vec![],
                },
                ServiceDef {
                    name: "disabledsvc".to_string(),
                    mode: "disabled".to_string(),
                    params: vec![],
                    comments: vec![],
                },
                ServiceDef {
                    name: "trylocalsvc".to_string(),
                    mode: "trylocal".to_string(),
                    params: vec![],
                    comments: vec![],
                },
                ServiceDef {
                    name: "buildtimesvc".to_string(),
                    mode: "buildtime".to_string(),
                    params: vec![],
                    comments: vec![],
                },
                ServiceDef {
                    name: "serversvc".to_string(),
                    mode: "serveronly".to_string(),
                    params: vec![],
                    comments: vec![],
                },
            ]
        }

        #[test]
        fn service_selected_covers_every_mode_and_callmode() {
            // The mode filter, verified against every case it
            // covers: eight stored modes against six callmodes.
            // `runall` runs everything, including `buildtime`;
            // `precommit` (trylocal) runs only `trylocal` and
            // `localonly` — a service with no stored mode does not
            // run there.
            let modes = [
                "",
                "buildtime",
                "serveronly",
                "manual",
                "disabled",
                "trylocal",
                "localonly",
                "weird",
            ];
            let cases: &[(Option<&str>, [bool; 8])] = &[
                (None, [true, false, false, false, false, true, true, true]),
                (
                    Some("local"),
                    [true, false, true, false, false, true, true, true],
                ),
                (
                    Some("manual"),
                    [false, false, false, true, false, false, false, false],
                ),
                (
                    Some("disabled"),
                    [false, false, false, false, true, false, false, false],
                ),
                (
                    Some("trylocal"),
                    [false, false, false, false, false, true, true, false],
                ),
                (
                    Some("all"),
                    [true, true, true, true, true, true, true, true],
                ),
            ];
            for (callmode, expected) in cases {
                for (mode, want) in modes.iter().zip(expected.iter()) {
                    assert_eq!(
                        service_selected(mode, *callmode),
                        *want,
                        "mode {mode:?} under callmode {callmode:?}"
                    );
                }
            }
        }

        #[test]
        fn service_plain_output_covers_every_mode_and_callmode() {
            // The placement rule, verified against every case it
            // covers: plain when the stored mode is manual,
            // disabled, trylocal, or localonly, or when the run
            // mode is local, trylocal, or all.
            let cases: &[(&str, Option<&str>, bool)] = &[
                ("", None, false),
                ("manual", None, true),
                ("disabled", None, true),
                ("trylocal", None, true),
                ("localonly", None, true),
                ("serveronly", None, false),
                ("buildtime", None, false),
                ("weird", None, false),
                ("", Some("local"), true),
                ("", Some("trylocal"), true),
                ("", Some("all"), true),
                ("", Some("manual"), false),
                ("", Some("disabled"), false),
                ("weird", Some("local"), true),
                ("buildtime", Some("all"), true),
            ];
            for (mode, callmode, want) in cases {
                assert_eq!(
                    service_plain_output(mode, *callmode),
                    *want,
                    "mode {mode:?} under callmode {callmode:?}"
                );
            }
        }

        #[test]
        fn service_run_mode_names_round_trip() {
            for mode in [
                ServiceRunMode::Run,
                ServiceRunMode::ManualRun,
                ServiceRunMode::Precommit,
                ServiceRunMode::LocalRun,
                ServiceRunMode::DisabledRun,
            ] {
                assert_eq!(
                    ServiceRunMode::from_command_name(mode.command_name()).unwrap(),
                    mode
                );
            }
            assert!(ServiceRunMode::from_command_name("runall").is_err());
            assert!(ServiceRunMode::from_command_name("ra").is_err());
            assert!(ServiceRunMode::from_command_name("bogus").is_err());
        }

        #[test]
        fn service_argv_passes_params_verbatim() {
            let service = SelectedService {
                name: "download_url".to_string(),
                effective_mode: String::new(),
                params: vec![
                    ServiceParam {
                        name: "protocol".to_string(),
                        value: "https".to_string(),
                    },
                    ServiceParam {
                        name: "path".to_string(),
                        value: "/a b/c?x=1&y=2".to_string(),
                    },
                ],
            };
            assert_eq!(
                service_argv(&service, "/work/.geectl-service-download_url-1-0"),
                vec![
                    "/usr/lib/obs/service/download_url",
                    "--protocol",
                    "https",
                    "--path",
                    "/a b/c?x=1&y=2",
                    "--outdir",
                    "/work/.geectl-service-download_url-1-0",
                ]
            );
        }

        #[test]
        fn service_packages_dedups_and_adds_spec_cleaner() {
            let selected = vec![
                SelectedService {
                    name: "format_spec_file".to_string(),
                    effective_mode: String::new(),
                    params: vec![],
                },
                SelectedService {
                    name: "obs_scm".to_string(),
                    effective_mode: String::new(),
                    params: vec![],
                },
                SelectedService {
                    name: "tar_scm".to_string(),
                    effective_mode: String::new(),
                    params: vec![],
                },
                SelectedService {
                    name: "tar_scm".to_string(),
                    effective_mode: String::new(),
                    params: vec![],
                },
            ];
            assert_eq!(
                service_packages(&selected),
                vec![
                    "cpio",
                    "obs-service-format_spec_file",
                    "obs-service-obs_scm",
                    "obs-service-tar_scm",
                    "spec-cleaner",
                ]
            );
            let selected = vec![SelectedService {
                name: "tar_scm".to_string(),
                effective_mode: String::new(),
                params: vec![],
            }];
            assert_eq!(service_packages(&selected), vec!["obs-service-tar_scm"]);
        }

        #[test]
        fn select_services_filters_by_mode_and_forces_single() {
            let defs = service_defs();
            let names = |services: Vec<SelectedService>| {
                services.into_iter().map(|s| s.name).collect::<Vec<_>>()
            };
            assert_eq!(
                names(select_services(&defs, ServiceRunMode::Run, None).unwrap()),
                vec!["plain", "trylocalsvc"]
            );
            assert_eq!(
                names(select_services(&defs, ServiceRunMode::Precommit, None).unwrap()),
                vec!["trylocalsvc"]
            );
            assert_eq!(
                names(select_services(&defs, ServiceRunMode::ManualRun, None).unwrap()),
                vec!["manualsvc"]
            );
            assert_eq!(
                names(select_services(&defs, ServiceRunMode::DisabledRun, None).unwrap()),
                vec!["disabledsvc"]
            );
            assert_eq!(
                names(select_services(&defs, ServiceRunMode::LocalRun, None).unwrap()),
                vec!["plain", "trylocalsvc", "serversvc"]
            );
            // A single-service run forces the run mode, keeping the
            // stored parameters: a manual service runs under a
            // plain `run` when named.
            let single = select_services(&defs, ServiceRunMode::Run, Some("manualsvc")).unwrap();
            assert_eq!(single.len(), 1);
            assert_eq!(single[0].name, "manualsvc");
            assert_eq!(single[0].effective_mode, "");
            // A service the file does not define runs with no
            // parameters and the run mode as its mode.
            let single = select_services(&defs, ServiceRunMode::ManualRun, Some("newsvc")).unwrap();
            assert_eq!(single.len(), 1);
            assert_eq!(single[0].name, "newsvc");
            assert_eq!(single[0].effective_mode, "manual");
            assert!(single[0].params.is_empty());
            // A short or slash-carrying name is a caller error.
            assert!(
                select_services(&defs, ServiceRunMode::Run, Some("ab"))
                    .unwrap_err()
                    .kind()
                    == ErrorKind::InvalidInput
            );
            assert!(
                select_services(&defs, ServiceRunMode::Run, Some("a/b"))
                    .unwrap_err()
                    .kind()
                    == ErrorKind::InvalidInput
            );
        }

        #[test]
        fn read_local_services_handles_missing_malformed_and_valid() {
            let dir = TempDir::new();
            let src = dir.path().join("source");
            fs::create_dir_all(&src).unwrap();
            assert!(read_local_services(&src).unwrap().is_empty());
            fs::write(src.join("_service"), "not a services document").unwrap();
            let err = read_local_services(&src).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            fs::write(
                src.join("_service"),
                "<services><service name=\"tar_scm\"><param name=\"url\">https://example.com/r.git</param></service>\
                <service name=\"set_version\" mode=\"buildtime\"/></services>",
            )
            .unwrap();
            let defs = read_local_services(&src).unwrap();
            assert_eq!(defs.len(), 2);
            assert_eq!(defs[0].name, "tar_scm");
            assert_eq!(defs[0].mode, "");
            assert_eq!(defs[0].params.len(), 1);
            assert_eq!(defs[0].params[0].name, "url");
            assert_eq!(defs[0].params[0].value, "https://example.com/r.git");
            assert_eq!(defs[1].name, "set_version");
            assert_eq!(defs[1].mode, "buildtime");
        }

        #[test]
        fn stash_and_place_move_files() {
            let dir = TempDir::new();
            let src = dir.path().join("source");
            fs::create_dir_all(&src).unwrap();
            fs::write(src.join("_service:alpha:alpha.tar.gz"), "tarball").unwrap();
            fs::write(src.join("_service_beta"), "old").unwrap();
            fs::write(src.join("keep.spec"), "spec").unwrap();
            let stash = stash_service_files(&src).unwrap().unwrap();
            assert!(!src.join("_service:alpha:alpha.tar.gz").exists());
            assert!(!src.join("_service_beta").exists());
            assert!(src.join("keep.spec").exists());
            assert!(stash.join("_service:alpha:alpha.tar.gz").exists());
            // A second stash while one exists fails closed.
            fs::write(src.join("_service:gamma:x"), "x").unwrap();
            let err = stash_service_files(&src).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            remove_stash(&Some(stash), &Debug::disabled());
            assert!(!src.join(".geectl-service-old").exists());
            // Placement: plain filenames, or prefixed.
            let outdir = src.join("out");
            fs::create_dir_all(&outdir).unwrap();
            fs::write(outdir.join("plain.txt"), "p").unwrap();
            place_outputs(&outdir, &src, "alpha", true).unwrap();
            assert!(src.join("plain.txt").exists());
            fs::write(outdir.join("prefixed.txt"), "q").unwrap();
            place_outputs(&outdir, &src, "alpha", false).unwrap();
            assert!(src.join("_service:alpha:prefixed.txt").exists());
        }

        #[test]
        fn service_run_end_to_end_stubbed() {
            let dir = TempDir::new();
            let src = dir.path().join("source");
            fs::create_dir_all(&src).unwrap();
            fs::write(
                src.join("_service"),
                "<services><service name=\"alpha\"><param name=\"file\">x</param></service>\
                <service name=\"beta\" mode=\"trylocal\"/></services>",
            )
            .unwrap();
            fs::write(src.join("_service:alpha:stale"), "stale").unwrap();
            let (podman, call_log) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let req = ServiceRunRequest::new(src.clone(), ServiceRunMode::Run);
            let record = vertical.service_run(&req).unwrap();
            assert_eq!(record.mode, "run");
            assert!(!record.dry_run);
            assert!(record.plan.is_none());
            assert_eq!(
                record.image_digest.as_deref(),
                Some("sha256:fake-digest-1234")
            );
            assert_eq!(record.ran.len(), 2);
            assert_eq!(record.ran[0].name, "alpha");
            assert_eq!(record.ran[1].name, "beta");
            // The plain service's output is prefixed; the trylocal
            // service's output lands plain.
            assert_eq!(
                fs::read_to_string(src.join("_service:alpha:alpha.out")).unwrap(),
                "output of alpha\n"
            );
            assert_eq!(
                fs::read_to_string(src.join("beta.out")).unwrap(),
                "output of beta\n"
            );
            // The stale prefixed file is gone, and no outdir or
            // stash is left behind.
            assert!(!src.join("_service:alpha:stale").exists());
            assert!(fs::read_dir(&src).unwrap().flatten().all(|e| !e
                .file_name()
                .to_string_lossy()
                .starts_with(".geectl-service-")));
            // The install ran once, for both services.
            let calls = fs::read_to_string(&call_log).unwrap();
            assert!(calls.contains("setpriv --reuid=0 --regid=0 --clear-groups zypper --non-interactive install --no-recommends obs-service-alpha obs-service-beta"));
            assert!(calls.contains("run -d --rm --name geectl-service-"));
        }

        #[test]
        fn service_run_dry_run_touches_nothing() {
            let dir = TempDir::new();
            let src = dir.path().join("source");
            fs::create_dir_all(&src).unwrap();
            fs::write(
                src.join("_service"),
                "<services><service name=\"alpha\"/></services>",
            )
            .unwrap();
            let (podman, call_log) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let mut req = ServiceRunRequest::new(src.clone(), ServiceRunMode::Run);
            req.dry_run = true;
            let record = vertical.service_run(&req).unwrap();
            assert!(record.dry_run);
            assert!(record.ran.is_empty());
            let plan = record.plan.unwrap();
            assert_eq!(plan.image, DEFAULT_IMAGE);
            assert_eq!(plan.services.len(), 1);
            assert_eq!(plan.services[0].name, "alpha");
            assert_eq!(plan.services[0].argv[0], "/usr/lib/obs/service/alpha");
            assert!(plan.services[0].argv.contains(&"--outdir".to_string()));
            // No container was started and no file appeared: the
            // call log does not exist when podman never ran.
            assert!(!fs::read_to_string(&call_log)
                .unwrap_or_default()
                .contains("run -d"));
            assert!(!src.join("_service:alpha:alpha.out").exists());
        }

        #[test]
        fn service_run_missing_service_file_is_a_noop() {
            let dir = TempDir::new();
            let src = dir.path().join("source");
            fs::create_dir_all(&src).unwrap();
            let (podman, call_log) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let req = ServiceRunRequest::new(src.clone(), ServiceRunMode::Run);
            let record = vertical.service_run(&req).unwrap();
            assert!(record.ran.is_empty());
            assert!(!fs::read_to_string(&call_log)
                .unwrap_or_default()
                .contains("run -d"));
        }

        #[test]
        fn service_run_empty_selection_clears_stale_files() {
            let dir = TempDir::new();
            let src = dir.path().join("source");
            fs::create_dir_all(&src).unwrap();
            fs::write(
                src.join("_service"),
                "<services><service name=\"alpha\" mode=\"buildtime\"/></services>",
            )
            .unwrap();
            fs::write(src.join("_service:alpha:old"), "stale").unwrap();
            let (podman, call_log) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let req = ServiceRunRequest::new(src.clone(), ServiceRunMode::Run);
            let record = vertical.service_run(&req).unwrap();
            assert!(record.ran.is_empty());
            assert!(!src.join("_service:alpha:old").exists());
            assert!(!fs::read_to_string(&call_log)
                .unwrap_or_default()
                .contains("run -d"));
        }

        #[test]
        fn service_container_exec_sets_workdir_and_runner_env() {
            let dir = TempDir::new();
            let src = dir.path().join("source");
            fs::create_dir_all(&src).unwrap();
            fs::write(
                src.join("_service"),
                "<services><service name=\"alpha\"/></services>",
            )
            .unwrap();
            let (podman, call_log) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let req = ServiceRunRequest::new(src.clone(), ServiceRunMode::Run);
            let record = vertical.service_run(&req).unwrap();
            assert_eq!(record.ran.len(), 1);
            let log = fs::read_to_string(&call_log).unwrap_or_default();
            assert!(
                log.lines()
                    .any(|l| l.starts_with("exec ") && l.contains("-w /work")),
                "exec calls carry no -w /work: {log}"
            );
            assert!(
                log.contains(&format!("OSC_VERSION={}", env!("CARGO_PKG_VERSION"))),
                "container carries no runner version: {log}"
            );
        }

        #[test]
        fn service_container_mount_survives_a_colon_in_the_source_path() {
            // The `--mount` form carries the path as one value, so
            // a `:` in the source path cannot split the mount.
            let dir = TempDir::new();
            let src = dir.path().join("so:urce");
            fs::create_dir_all(&src).unwrap();
            fs::write(
                src.join("_service"),
                "<services><service name=\"alpha\"/></services>",
            )
            .unwrap();
            let (podman, call_log) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let req = ServiceRunRequest::new(src.clone(), ServiceRunMode::Run);
            let record = vertical.service_run(&req).unwrap();
            assert_eq!(record.ran.len(), 1);
            let log = fs::read_to_string(&call_log).unwrap_or_default();
            let run_line = log.lines().find(|l| l.starts_with("run "));
            let Some(run_line) = run_line else {
                panic!("no run call in log: {log}");
            };
            assert!(
                run_line.contains(&format!(
                    "--mount type=bind,src={},target=/work",
                    src.display()
                )),
                "run line carries no --mount form: {run_line}"
            );
            assert!(
                !run_line.contains(" -v "),
                "run line still uses the -v string form: {run_line}"
            );
            let mount_pos = run_line
                .find("--mount")
                .expect("run line carries no --mount");
            let image_pos = run_line
                .find(DEFAULT_IMAGE)
                .expect("run line carries no image");
            assert!(
                mount_pos < image_pos,
                "mount flag follows the image name: {run_line}"
            );
        }

        #[test]
        fn service_container_mount_refuses_a_comma_in_the_source_path() {
            // The `--mount` value is comma-separated with no
            // escape: a `,` in the source path splits the mount
            // and podman refuses the fragment. The run is refused
            // before any podman call, naming the path.
            let dir = TempDir::new();
            let src = dir.path().join("so,urce");
            fs::create_dir_all(&src).unwrap();
            fs::write(
                src.join("_service"),
                "<services><service name=\"alpha\"/></services>",
            )
            .unwrap();
            fs::write(src.join("_service:alpha:old"), "stale").unwrap();
            let (podman, call_log) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let req = ServiceRunRequest::new(src.clone(), ServiceRunMode::Run);
            let err = vertical.service_run(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(
                err.context.contains(&src.display().to_string()),
                "refusal names no path: {err}"
            );
            let log = fs::read_to_string(&call_log).unwrap_or_default();
            assert!(
                !log.lines()
                    .any(|l| l.starts_with("run ") || l.starts_with("image ")),
                "podman ran before the refusal: {log}"
            );
            assert!(
                !src.join(".geectl-service-old").exists(),
                "refusal left the moved-aside stash behind"
            );
        }

        #[test]
        fn service_container_mount_survives_an_equals_in_the_source_path() {
            // The run line carries the `=` in the source path
            // as part of the mount value.
            let dir = TempDir::new();
            let src = dir.path().join("so=urce");
            fs::create_dir_all(&src).unwrap();
            fs::write(
                src.join("_service"),
                "<services><service name=\"alpha\"/></services>",
            )
            .unwrap();
            let (podman, call_log) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let req = ServiceRunRequest::new(src.clone(), ServiceRunMode::Run);
            let record = vertical.service_run(&req).unwrap();
            assert_eq!(record.ran.len(), 1);
            let log = fs::read_to_string(&call_log).unwrap_or_default();
            let run_line = log.lines().find(|l| l.starts_with("run "));
            let Some(run_line) = run_line else {
                panic!("no run call in log: {log}");
            };
            assert!(
                run_line.contains(&format!(
                    "--mount type=bind,src={},target=/work",
                    src.display()
                )),
                "run line carries no --mount form: {run_line}"
            );
        }

        #[test]
        fn service_container_run_carries_hardening_flags() {
            // The container runs hardened: as the invoking user
            // (`--userns=keep-id`), capabilities dropped except
            // the install set, no new privileges, memory and
            // process counts capped.
            let dir = TempDir::new();
            let src = dir.path().join("source");
            fs::create_dir_all(&src).unwrap();
            fs::write(
                src.join("_service"),
                "<services><service name=\"alpha\"/></services>",
            )
            .unwrap();
            let (podman, call_log) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let req = ServiceRunRequest::new(src.clone(), ServiceRunMode::Run);
            let record = vertical.service_run(&req).unwrap();
            assert_eq!(record.ran.len(), 1);
            let log = fs::read_to_string(&call_log).unwrap_or_default();
            let run_line = log.lines().find(|l| l.starts_with("run "));
            let Some(run_line) = run_line else {
                panic!("no run call in log: {log}");
            };
            let image_pos = run_line
                .find(DEFAULT_IMAGE)
                .expect("run line carries no image");
            for flag in [
                "--userns keep-id",
                "--cap-drop ALL",
                "--cap-add DAC_OVERRIDE,SETGID,SETUID",
                "--security-opt no-new-privileges",
                "--memory 4g",
                "--memory-swap 4g",
                "--pids-limit 1024",
            ] {
                assert!(run_line.contains(flag), "run line lacks {flag}: {run_line}");
                assert!(
                    run_line.find(flag).unwrap() < image_pos,
                    "flag {flag} follows the image name: {run_line}"
                );
            }
        }

        #[test]
        fn service_container_exec_timeout_kills_the_container() {
            // A service that runs past the deadline: the error is
            // a timeout, and a kill is issued for the container.
            let dir = TempDir::new();
            let src = dir.path().join("source");
            fs::create_dir_all(&src).unwrap();
            fs::write(
                src.join("_service"),
                "<services><service name=\"slowpoke\"/></services>",
            )
            .unwrap();
            let (podman, call_log) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let mut req = ServiceRunRequest::new(src.clone(), ServiceRunMode::Run);
            req.timeout_secs = 2;
            let started = std::time::Instant::now();
            let err = vertical.service_run(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::BuildTimeout);
            assert!(started.elapsed() < Duration::from_secs(10));
            let log = fs::read_to_string(&call_log).unwrap_or_default();
            let Some(exec_pos) = log.find("exec ") else {
                panic!("no exec call in log: {log}");
            };
            let Some(kill_pos) = log.find("kill geectl-service-") else {
                panic!("no kill call in log: {log}");
            };
            assert!(
                kill_pos > exec_pos,
                "kill precedes the timed-out exec: {log}"
            );
        }

        #[test]
        fn service_run_stops_at_the_first_failure() {
            let dir = TempDir::new();
            let src = dir.path().join("source");
            fs::create_dir_all(&src).unwrap();
            fs::write(
                src.join("_service"),
                "<services><service name=\"alpha\"/><service name=\"failme\"/><service name=\"beta\"/></services>",
            )
            .unwrap();
            let (podman, _) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let req = ServiceRunRequest::new(src.clone(), ServiceRunMode::Run);
            let err = vertical.service_run(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ServiceFailed);
            assert!(err.context.contains("failme"), "got: {}", err.context);
            assert!(err.context.contains("boom"), "got: {}", err.context);
            // The earlier service's output is placed; the later
            // service never ran; no outdir or stash is left.
            assert!(src.join("_service:alpha:alpha.out").exists());
            assert!(!src.join("_service:beta:beta.out").exists());
            assert!(!src.join("beta.out").exists());
            assert!(fs::read_dir(&src).unwrap().flatten().all(|e| !e
                .file_name()
                .to_string_lossy()
                .starts_with(".geectl-service-")));
        }

        #[test]
        fn service_run_single_service_runs_only_that_one() {
            let dir = TempDir::new();
            let src = dir.path().join("source");
            fs::create_dir_all(&src).unwrap();
            fs::write(
                src.join("_service"),
                "<services><service name=\"alpha\"/><service name=\"beta\"/></services>",
            )
            .unwrap();
            let (podman, _) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let mut req = ServiceRunRequest::new(src.clone(), ServiceRunMode::Run);
            req.service = Some("beta".to_string());
            let record = vertical.service_run(&req).unwrap();
            assert_eq!(record.ran.len(), 1);
            assert_eq!(record.ran[0].name, "beta");
            assert!(!src.join("_service:alpha:alpha.out").exists());
            assert!(src.join("_service:beta:beta.out").exists());
        }

        #[test]
        fn service_run_rejects_a_zero_timeout() {
            let dir = TempDir::new();
            let src = dir.path().join("source");
            fs::create_dir_all(&src).unwrap();
            let (podman, _) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let mut req = ServiceRunRequest::new(src.clone(), ServiceRunMode::Run);
            req.timeout_secs = 0;
            let err = vertical.service_run(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
        }

        #[test]
        fn service_run_refuses_a_stale_stash() {
            let dir = TempDir::new();
            let src = dir.path().join("source");
            fs::create_dir_all(&src).unwrap();
            fs::write(
                src.join("_service"),
                "<services><service name=\"alpha\"/></services>",
            )
            .unwrap();
            fs::create_dir(src.join(".geectl-service-old")).unwrap();
            let (podman, call_log) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let req = ServiceRunRequest::new(src.clone(), ServiceRunMode::Run);
            let err = vertical.service_run(&req).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            // The refused run calls no podman: the call log does
            // not exist when podman never ran.
            assert!(!fs::read_to_string(&call_log)
                .unwrap_or_default()
                .contains("run -d"));
        }

        #[test]
        fn precommit_runs_trylocal_services_and_the_script() {
            let dir = TempDir::new();
            let src = dir.path().join("source");
            fs::create_dir_all(&src).unwrap();
            fs::write(
                src.join("_service"),
                "<services><service name=\"alpha\"/><service name=\"beta\" mode=\"trylocal\"/><service name=\"gamma\" mode=\"manual\"/></services>",
            )
            .unwrap();
            write_script(
                &src.join("pre_checkin.sh"),
                "#!/bin/sh\necho ran > pre_checkin.marker\n",
            );
            let (podman, _) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let record = vertical.precommit(&src, 3600, false).unwrap();
            assert!(!record.dry_run);
            assert_eq!(record.services.ran.len(), 1);
            assert_eq!(record.services.ran[0].name, "beta");
            assert!(record.pre_checkin.ran);
            assert_eq!(record.pre_checkin.exit_code, Some(0));
            assert!(src.join("pre_checkin.marker").exists());
            assert!(src.join("beta.out").exists());
            assert!(!src.join("_service:alpha:alpha.out").exists());
            assert!(!src.join("gamma.out").exists());
        }

        #[test]
        fn precommit_without_a_script_reports_it_did_not_run() {
            let dir = TempDir::new();
            let src = dir.path().join("source");
            fs::create_dir_all(&src).unwrap();
            let (podman, _) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let record = vertical.precommit(&src, 3600, false).unwrap();
            assert!(record.services.ran.is_empty());
            assert!(!record.pre_checkin.ran);
            assert_eq!(record.pre_checkin.exit_code, None);
        }

        #[test]
        fn precheckin_refuses_a_comma_in_the_source_path() {
            // The mount refusal fires before the image resolve:
            // a comma path fails here, not in the container.
            let dir = TempDir::new();
            let src = dir.path().join("so,urce");
            fs::create_dir_all(&src).unwrap();
            write_script(&src.join("pre_checkin.sh"), "#!/bin/sh\nexit 0\n");
            let (podman, call_log) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let deadline = std::time::Instant::now() + Duration::from_secs(3600);
            let err = vertical.run_precheckin(&src, deadline, false).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(
                err.context.contains(&src.display().to_string()),
                "refusal names no path: {err}"
            );
            let log = fs::read_to_string(&call_log).unwrap_or_default();
            assert!(
                !log.lines().any(|l| l.starts_with("image ")),
                "podman ran before the refusal: {log}"
            );
        }

        #[test]
        fn precommit_failing_script_blocks_the_run() {
            let dir = TempDir::new();
            let src = dir.path().join("source");
            fs::create_dir_all(&src).unwrap();
            write_script(&src.join("pre_checkin.sh"), "#!/bin/sh\nexit 3\n");
            let (podman, _) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let err = vertical.precommit(&src, 3600, false).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ServiceFailed);
            assert!(
                err.context.contains("pre_checkin.sh"),
                "got: {}",
                err.context
            );
        }

        #[test]
        fn spec_cleaner_refuses_a_comma_in_the_source_path() {
            // The mount refusal fires before the image resolve:
            // a comma path fails here, not in the container.
            let dir = TempDir::new();
            let src = dir.path().join("so,urce");
            fs::create_dir_all(&src).unwrap();
            fs::write(src.join("foo.spec"), "Name: foo\n").unwrap();
            let (podman, call_log) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let err = vertical
                .spec_cleaner(&src, Path::new("foo.spec"), &[], 3600, false)
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(
                err.context.contains(&src.display().to_string()),
                "refusal names no path: {err}"
            );
            let log = fs::read_to_string(&call_log).unwrap_or_default();
            assert!(
                !log.lines().any(|l| l.starts_with("image ")),
                "podman ran before the refusal: {log}"
            );
        }

        #[test]
        fn spec_cleaner_edits_the_spec_in_place_stubbed() {
            let dir = TempDir::new();
            let src = dir.path().join("source");
            fs::create_dir_all(&src).unwrap();
            fs::write(src.join("foo.spec"), "Name: foo\n").unwrap();
            let (podman, _) = stub_service_podman(dir.path(), &src);
            let http = FakeHttp::new();
            let client = client_with(&http);
            let vertical = vertical(&client, dir.path(), dir.path().join("engine"), podman);
            let result = vertical
                .spec_cleaner(&src, Path::new("foo.spec"), &[], 3600, false)
                .unwrap();
            assert_eq!(result.exit_code, 0);
            assert!(!result.dry_run);
            assert!(result.plan.is_none());
            assert!(fs::read_to_string(src.join("foo.spec"))
                .unwrap()
                .contains("# spec-cleaner ran"));
            // A dry run returns the plan and touches nothing.
            fs::write(src.join("foo.spec"), "Name: foo\n").unwrap();
            let result = vertical
                .spec_cleaner(&src, Path::new("foo.spec"), &[], 3600, true)
                .unwrap();
            assert!(result.dry_run);
            assert_eq!(result.plan.as_ref().unwrap().last().unwrap(), "foo.spec");
            assert_eq!(
                fs::read_to_string(src.join("foo.spec")).unwrap(),
                "Name: foo\n"
            );
            // A spec outside the source directory is a caller error.
            let err = vertical
                .spec_cleaner(&src, Path::new("/etc/passwd"), &[], 3600, false)
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            // A relative spec escaping the source directory is a
            // caller error too, not a confusing in-container failure.
            let other = dir.path().join("other");
            fs::create_dir_all(&other).unwrap();
            fs::write(other.join("foo.spec"), "Name: foo\n").unwrap();
            let err = vertical
                .spec_cleaner(&src, Path::new("../other/foo.spec"), &[], 3600, false)
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(
                err.context.contains("outside source directory"),
                "got: {}",
                err.context
            );
        }
    }
}
