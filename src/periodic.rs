//! `periodic` — the daily periodic cache cleanup: the scheduler entries
//! that run `local-build prune --apply` once a day, on each platform.
//!
//! The schedule is the same everywhere: daily at 03:00 local time, under
//! the same policy and safeguards as automatic eviction (the prune pass
//! itself enforces them). The three platforms differ only in where the
//! schedule lives:
//!
//! - Linux: a systemd user service + timer (`geectl-prune.service` /
//!   `geectl-prune.timer` in `~/.config/systemd/user/`, or
//!   `$XDG_CONFIG_HOME/systemd/user/`), enabled with `systemctl --user`.
//!   The same unit files are the ones the RPM package will ship
//!   (separate packaging track); the spec substitutes `@GEECTL_BIN@`
//!   with the packaged binary path.
//! - macOS: a launchd agent (`io.github.scarabeusiv.geectl.prune.plist`
//!   in `~/Library/LaunchAgents/`), loaded with `launchctl load` —
//!   a prior load is unloaded first, so a re-install picks up the
//!   new plist instead of failing on an already-loaded agent.
//! - Windows: a Task Scheduler task (`geectl-prune`), created with
//!   `schtasks /create /xml`.
//!
//! The unit files live as templates under `packaging/` and are the
//! single source of truth: the install path renders them with the
//! running binary's path substituted for `@GEECTL_BIN@`, and the RPM
//! will ship the same files. A template that still carries a
//! placeholder after rendering is a bug: rendering fails closed
//! instead of writing a broken unit.
//!
//! The plan builders are pure functions of an explicit platform, so
//! every platform's content is unit-tested on any host; only the thin
//! execution layer (write files, run the enable/disable/query
//! commands) touches the system, and every scheduler command runs
//! under a 60-second bound.

use crate::config;
use crate::error::{ErrorKind, GeectlError};
use fs2::FileExt;
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The systemd unit base name and the Task Scheduler task name.
pub const TASK_NAME: &str = "geectl-prune";
/// The launchd agent label.
pub const LAUNCHD_LABEL: &str = "io.github.scarabeusiv.geectl.prune";
/// The bound on every scheduler command (`systemctl`, `launchctl`,
/// `schtasks`): admin commands return promptly, a wedged service
/// manager must not hang geectl.
const SCHEDULER_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a failed auto-install suppresses the next attempt: a
/// day. Long enough to stop every build re-running the scheduler
/// probes for a failure that has not changed, short enough that a
/// fixed scheduler is picked up without any manual step.
const AUTO_INSTALL_RETRY_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

const SYSTEMD_SERVICE_TEMPLATE: &str = include_str!("../packaging/systemd/geectl-prune.service");
const SYSTEMD_TIMER_TEMPLATE: &str = include_str!("../packaging/systemd/geectl-prune.timer");
const LAUNCHD_TEMPLATE: &str =
    include_str!("../packaging/launchd/io.github.scarabeusiv.geectl.prune.plist");
const SCHEDULED_TASK_TEMPLATE: &str = include_str!("../packaging/windows/geectl-prune.xml");

/// The platforms a periodic schedule ships on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PeriodicOs {
    Linux,
    MacOs,
    Windows,
}

impl PeriodicOs {
    /// The current platform, or `None` where no schedule ships.
    pub fn current() -> Option<Self> {
        match std::env::consts::OS {
            "linux" => Some(PeriodicOs::Linux),
            "macos" => Some(PeriodicOs::MacOs),
            "windows" => Some(PeriodicOs::Windows),
            _ => None,
        }
    }

    /// The machine-readable name, as it appears in payloads.
    pub fn name(self) -> &'static str {
        match self {
            PeriodicOs::Linux => "linux",
            PeriodicOs::MacOs => "macos",
            PeriodicOs::Windows => "windows",
        }
    }
}

/// One file the schedule owns: where it is written, and its content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodicFile {
    pub path: PathBuf,
    pub content: String,
}

/// One scheduler command in a plan. `tolerate_missing` means the
/// command may fail with "already gone" and still count as run:
/// the enable step's macOS unload (a re-install unloads a prior
/// load first; a fresh install has nothing to unload) and every
/// disable step (uninstalling a schedule that is already gone is a
/// no-op, not an error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodicCommand {
    pub argv: Vec<String>,
    pub tolerate_missing: bool,
}

impl PeriodicCommand {
    /// A command that must succeed.
    fn strict(argv: Vec<String>) -> Self {
        PeriodicCommand {
            argv,
            tolerate_missing: false,
        }
    }

    /// A command that may fail with "already gone".
    fn tolerating(argv: Vec<String>) -> Self {
        PeriodicCommand {
            argv,
            tolerate_missing: true,
        }
    }
}

/// The install/uninstall plan for one platform: the files the
/// schedule owns, the commands that register it, the commands that
/// unregister it, and the label under which the scheduler knows it.
/// Install writes `files`, then runs `enable`; uninstall runs
/// `disable`, then removes `files`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodicPlan {
    pub os: PeriodicOs,
    pub label: String,
    pub files: Vec<PeriodicFile>,
    pub enable: Vec<PeriodicCommand>,
    pub disable: Vec<PeriodicCommand>,
}

/// The outcome of an install or uninstall run: the files that
/// persist after the run, and the commands that ran. In a dry run
/// nothing is written and nothing runs; the plan is the outcome. The
/// Windows import file is transient — written, read by schtasks,
/// then removed — so an applied Windows install reports no files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PeriodicOutcome {
    pub platform: PeriodicOs,
    pub label: String,
    pub dry_run: bool,
    pub files: Vec<PathBuf>,
    pub commands: Vec<Vec<String>>,
}

/// The status of the schedule on one platform. `installed` is
/// `None` on Windows when the query fails ambiguously: the query is
/// the only answer there, so a failed query is unknown, not absent.
/// `enabled` and `active` are `None` when the scheduler cannot be
/// asked or its answer names no known state: unknown, not disabled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PeriodicStatus {
    pub platform: PeriodicOs,
    pub label: String,
    pub installed: Option<bool>,
    pub enabled: Option<bool>,
    pub active: Option<bool>,
    pub files: Vec<PathBuf>,
}

/// Escape `&`, `<` and `>` for XML element text. The plist and the
/// scheduled-task XML carry the binary path as element text; a path
/// with a `&` in it must not break the document.
fn escape_xml_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Quote the first word of a systemd `ExecStart` line (the binary
/// path): systemd splits on
/// whitespace with no shell, so a path with a space in it is quoted,
/// with `"` and `\` escaped inside the quotes. `%` specifiers are
/// escaped here: they expand even in the first word, which is the
/// binary path here. `$` is left alone: the first word is
/// the literal path (never `$`-expanded), so doubling it would
/// point at a nonexistent `$$` path. A `'` would otherwise start
/// a quote of its own.
fn quote_systemd_word(word: &str) -> String {
    let escaped: String = word
        .chars()
        .flat_map(|ch| match ch {
            '%' => vec!['%', '%'],
            _ => vec![ch],
        })
        .collect();
    if word.contains(char::is_whitespace)
        || word.contains('"')
        || word.contains('\\')
        || word.contains('\'')
    {
        let mut out = String::with_capacity(escaped.len() + 2);
        out.push('"');
        for ch in escaped.chars() {
            if ch == '"' || ch == '\\' {
                out.push('\\');
            }
            out.push(ch);
        }
        out.push('"');
        out
    } else {
        escaped
    }
}

/// Substitute the placeholders in a template. `@GEECTL_BIN@` is the
/// binary path, `@GEECTL_HOME@` the home directory. A template that
/// still carries a placeholder after substitution is a bug: fail
/// closed instead of writing a broken unit file.
fn render_template(
    template: &str,
    exe: &Path,
    home: Option<&Path>,
    xml_escape: bool,
) -> Result<String, GeectlError> {
    let exe = exe.display().to_string();
    let rendered = template.replace("@GEECTL_BIN@", &{
        if xml_escape {
            escape_xml_text(&exe)
        } else {
            quote_systemd_word(&exe)
        }
    });
    let rendered = match home {
        Some(home) => {
            let home = home.display().to_string();
            rendered.replace("@GEECTL_HOME@", &{
                if xml_escape {
                    escape_xml_text(&home)
                } else {
                    home
                }
            })
        }
        None => rendered,
    };
    if rendered.contains("@GEECTL_") {
        return Err(GeectlError::new(
            ErrorKind::InternalError,
            "periodic template still carries an unsubstituted placeholder",
        ));
    }
    Ok(rendered)
}

/// The rendered systemd service unit for `exe`.
pub fn render_systemd_service(exe: &Path) -> Result<String, GeectlError> {
    render_template(SYSTEMD_SERVICE_TEMPLATE, exe, None, false)
}

/// The rendered systemd timer unit. It carries no binary path.
pub fn render_systemd_timer() -> Result<String, GeectlError> {
    let rendered = SYSTEMD_TIMER_TEMPLATE.to_string();
    if rendered.contains("@GEECTL_") {
        return Err(GeectlError::new(
            ErrorKind::InternalError,
            "periodic template still carries an unsubstituted placeholder",
        ));
    }
    Ok(rendered)
}

/// The rendered launchd agent plist for `exe` under `home`.
pub fn render_launchd_plist(exe: &Path, home: &Path) -> Result<String, GeectlError> {
    render_template(LAUNCHD_TEMPLATE, exe, Some(home), true)
}

/// The rendered Task Scheduler task XML for `exe`.
pub fn render_scheduled_task_xml(exe: &Path) -> Result<String, GeectlError> {
    render_template(SCHEDULED_TASK_TEMPLATE, exe, None, true)
}

/// The systemd user unit directory: `$XDG_CONFIG_HOME/systemd/user/`
/// when `XDG_CONFIG_HOME` is set and non-empty, else
/// `~/.config/systemd/user/`. A relative directory is a configuration
/// error: systemd never reads a CWD-relative unit dir.
pub fn systemd_user_dir(
    xdg_config_home: Option<&str>,
    home: &Path,
) -> Result<PathBuf, GeectlError> {
    let dir = match xdg_config_home {
        Some(xdg) if !xdg.trim().is_empty() => {
            PathBuf::from(xdg.trim()).join("systemd").join("user")
        }
        _ => home.join(".config").join("systemd").join("user"),
    };
    if !dir.is_absolute() {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!(
                "systemd user unit directory is not absolute: {}",
                dir.display()
            ),
        ));
    }
    Ok(dir)
}

/// The launchd agents directory: `~/Library/LaunchAgents/`.
pub fn launchd_agents_dir(home: &Path) -> PathBuf {
    home.join("Library").join("LaunchAgents")
}

/// Resolve the user's home directory: `HOME`, or `USERPROFILE` on
/// Windows when `HOME` is absent or (on Windows) not a
/// Windows-absolute path. No home, or a relative one, is a
/// configuration error, never a silent fallback.
pub fn resolve_home() -> Result<PathBuf, GeectlError> {
    // Trim before `home_dir` selects: on Windows its absolute
    // filter sees the raw `HOME`, so a leading-space `HOME` would
    // fall back to `USERPROFILE` instead of being accepted.
    let home_var = std::env::var("HOME").ok().map(|v| v.trim().to_owned());
    let userprofile_var = std::env::var("USERPROFILE")
        .ok()
        .map(|v| v.trim().to_owned());
    let no_home = if cfg!(windows) {
        "no HOME and no USERPROFILE: cannot resolve the home directory"
    } else {
        "no HOME: cannot resolve the home directory"
    };
    let home = config::home_dir(home_var.as_deref(), userprofile_var.as_deref())
        .map(|s| s.to_owned())
        .ok_or_else(|| GeectlError::new(ErrorKind::ConfigError, no_home))?;
    let home = home.trim();
    if home.is_empty() {
        return Err(GeectlError::new(ErrorKind::ConfigError, no_home));
    }
    let home = PathBuf::from(home);
    if !home.is_absolute() {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("home directory is not absolute: {}", home.display()),
        ));
    }
    Ok(home)
}

/// The install plan for `os`: the files to write and the commands
/// that register them. `home` is the user's home directory;
/// `xdg_config_home` is `$XDG_CONFIG_HOME` when set (Linux only).
pub fn install_plan(
    os: PeriodicOs,
    exe: &Path,
    home: &Path,
    xdg_config_home: Option<&str>,
) -> Result<PeriodicPlan, GeectlError> {
    match os {
        PeriodicOs::Linux => {
            let dir = systemd_user_dir(xdg_config_home, home)?;
            Ok(PeriodicPlan {
                os,
                label: format!("{TASK_NAME}.timer"),
                files: vec![
                    PeriodicFile {
                        path: dir.join(format!("{TASK_NAME}.service")),
                        content: render_systemd_service(exe)?,
                    },
                    PeriodicFile {
                        path: dir.join(format!("{TASK_NAME}.timer")),
                        content: render_systemd_timer()?,
                    },
                ],
                enable: vec![
                    PeriodicCommand::strict(vec![
                        "systemctl".to_string(),
                        "--user".to_string(),
                        "daemon-reload".to_string(),
                    ]),
                    PeriodicCommand::strict(vec![
                        "systemctl".to_string(),
                        "--user".to_string(),
                        "enable".to_string(),
                        "--now".to_string(),
                        format!("{TASK_NAME}.timer"),
                    ]),
                ],
                disable: vec![
                    PeriodicCommand::tolerating(vec![
                        "systemctl".to_string(),
                        "--user".to_string(),
                        "disable".to_string(),
                        "--now".to_string(),
                        format!("{TASK_NAME}.timer"),
                    ]),
                    PeriodicCommand::strict(vec![
                        "systemctl".to_string(),
                        "--user".to_string(),
                        "daemon-reload".to_string(),
                    ]),
                ],
            })
        }
        PeriodicOs::MacOs => {
            let dir = launchd_agents_dir(home);
            let plist = dir.join(format!("{LAUNCHD_LABEL}.plist"));
            Ok(PeriodicPlan {
                os,
                label: LAUNCHD_LABEL.to_string(),
                files: vec![PeriodicFile {
                    path: plist.clone(),
                    content: render_launchd_plist(exe, home)?,
                }],
                // Unload first: `launchctl load` fails on an
                // already-loaded agent, so a re-install unloads the
                // prior load before loading the new plist. The
                // unload tolerates a fresh install, which has
                // nothing loaded yet.
                enable: vec![
                    PeriodicCommand::tolerating(vec![
                        "launchctl".to_string(),
                        "unload".to_string(),
                        plist.display().to_string(),
                    ]),
                    PeriodicCommand::strict(vec![
                        "launchctl".to_string(),
                        "load".to_string(),
                        plist.display().to_string(),
                    ]),
                ],
                disable: vec![PeriodicCommand::tolerating(vec![
                    "launchctl".to_string(),
                    "unload".to_string(),
                    plist.display().to_string(),
                ])],
            })
        }
        PeriodicOs::Windows => {
            // The task XML lives only long enough for schtasks to
            // read it: schtasks owns the task afterwards. The temp
            // path carries a suffix unique to this install (pid,
            // wall-clock nanos, per-process counter), so two
            // concurrent installs never share one file — a pid
            // alone could collide across a pid reuse — and the
            // write itself is an O_EXCL temp file plus rename, so
            // a planted name fails the write instead of being
            // followed. The path is bound once so the file and
            // the command agree.
            let xml_path =
                std::env::temp_dir().join(format!("{TASK_NAME}-{}.xml", unique_suffix()));
            Ok(PeriodicPlan {
                os,
                label: TASK_NAME.to_string(),
                files: vec![PeriodicFile {
                    path: xml_path.clone(),
                    content: render_scheduled_task_xml(exe)?,
                }],
                enable: vec![PeriodicCommand::strict(vec![
                    "schtasks".to_string(),
                    "/create".to_string(),
                    "/tn".to_string(),
                    TASK_NAME.to_string(),
                    "/xml".to_string(),
                    xml_path.display().to_string(),
                    "/f".to_string(),
                ])],
                disable: vec![PeriodicCommand::tolerating(vec![
                    "schtasks".to_string(),
                    "/delete".to_string(),
                    "/tn".to_string(),
                    TASK_NAME.to_string(),
                    "/f".to_string(),
                ])],
            })
        }
    }
}

/// The uninstall plan for `os`: the files to remove and the commands
/// that unregister them first. The Windows task owns no file, so its
/// plan carries no files: only the `schtasks /delete`.
pub fn uninstall_plan(
    os: PeriodicOs,
    home: &Path,
    xdg_config_home: Option<&str>,
) -> Result<PeriodicPlan, GeectlError> {
    match os {
        PeriodicOs::Windows => Ok(PeriodicPlan {
            os,
            label: TASK_NAME.to_string(),
            files: vec![],
            enable: vec![],
            disable: vec![PeriodicCommand::tolerating(vec![
                "schtasks".to_string(),
                "/delete".to_string(),
                "/tn".to_string(),
                TASK_NAME.to_string(),
                "/f".to_string(),
            ])],
        }),
        _ => {
            // Only the paths and the disable commands are used: the
            // exe is a dummy, the content is never written.
            let mut plan = install_plan(os, Path::new("geectl"), home, xdg_config_home)?;
            plan.enable = vec![];
            Ok(plan)
        }
    }
}

/// Run one scheduler command, bounded by `SCHEDULER_TIMEOUT`.
/// A spawn failure names the missing command; a timeout kills the
/// command and reports it; a non-zero exit is returned, not raised —
/// the caller decides whether it is an error or an unknown state.
fn run_bounded(argv: &[String]) -> Result<std::process::Output, GeectlError> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            GeectlError::new(
                ErrorKind::ConfigError,
                format!("cannot run {}: {e}", argv[0]),
            )
        })?;
    // Drain both pipes on threads while the child runs: a child
    // that writes more than the pipe buffer would otherwise block
    // forever and be misreported as a timeout. A drain-thread
    // spawn failure must not leak the child: kill it and wait
    // before reporting, and join the stdout handle already
    // spawned when the stderr spawn fails.
    let stdout = match spawn_drain(child.stdout.take().expect("stdout piped")) {
        Ok(handle) => handle,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    let stderr = match spawn_drain(child.stderr.take().expect("stderr piped")) {
        Ok(handle) => handle,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout.join();
            return Err(e);
        }
    };
    let deadline = Instant::now() + SCHEDULER_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(std::process::Output {
                    status,
                    stdout: stdout.join().unwrap_or_default(),
                    stderr: stderr.join().unwrap_or_default(),
                });
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout.join();
                    let _ = stderr.join();
                    return Err(GeectlError::new(
                        ErrorKind::Timeout,
                        format!(
                            "{} timed out after {}s",
                            argv[0],
                            SCHEDULER_TIMEOUT.as_secs()
                        ),
                    ));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                // Never leak the child on this near-unreachable
                // path: kill it and drain the pipes before
                // reporting.
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout.join();
                let _ = stderr.join();
                return Err(GeectlError::new(
                    ErrorKind::InternalError,
                    format!("cannot wait for {}: {e}", argv[0]),
                ));
            }
        }
    }
}

/// Drain one piped stream on a thread, collecting its bytes. A
/// spawn failure is an error, never a panic.
fn spawn_drain<T: std::io::Read + Send + 'static>(
    mut pipe: T,
) -> Result<std::thread::JoinHandle<Vec<u8>>, GeectlError> {
    std::thread::Builder::new()
        .spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            buf
        })
        .map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot spawn output reader: {e}"),
            )
        })
}

/// The stderr phrases that mean "already gone" for one platform's
/// disable step (and the macOS enable unload): a schedule that is
/// already gone uninstalls as a no-op, not an error.
fn missing_phrases(os: PeriodicOs) -> &'static [&'static str] {
    match os {
        // "Failed to disable unit: Unit file geectl-prune.timer
        // does not exist."
        PeriodicOs::Linux => &["does not exist"],
        // "Could not find service \"...\" in domain for port"
        // (newer) / "Could not find specified service" (older).
        PeriodicOs::MacOs => &["Could not find service", "Could not find specified service"],
        // "ERROR: The system cannot find the file specified."
        PeriodicOs::Windows => &["cannot find the file"],
    }
}

/// Run one command. A failure stops the run and names the command,
/// its exit status and its stderr — unless the command tolerates a
/// missing target and its stderr says the target is already gone,
/// in which case it counts as run. The status is named because a
/// signal death (a scheduler command aborting) leaves stderr empty.
fn run_command(os: PeriodicOs, command: &PeriodicCommand) -> Result<Vec<String>, GeectlError> {
    let output = run_bounded(&command.argv)?;
    if output.status.success() {
        return Ok(command.argv.clone());
    }
    if command.tolerate_missing {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if missing_phrases(os)
            .iter()
            .any(|phrase| stderr.contains(phrase))
        {
            return Ok(command.argv.clone());
        }
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let detail = if stderr.is_empty() {
        output.status.to_string()
    } else {
        format!("{}: {stderr}", output.status)
    };
    Err(GeectlError::new(
        ErrorKind::ConfigError,
        format!("{} failed ({detail})", command.argv.join(" ")),
    ))
}

/// Run each command in order. A failure stops the run and names the
/// command, its exit status and its stderr; the commands before it
/// already ran.
fn run_all(os: PeriodicOs, commands: &[PeriodicCommand]) -> Result<Vec<Vec<String>>, GeectlError> {
    let mut ran = Vec::new();
    for command in commands {
        ran.push(run_command(os, command)?);
    }
    Ok(ran)
}

/// On macOS, ask launchctl whether the agent is loaded: `Some`
/// when the answer is known, `None` when the query itself fails
/// ambiguously. The query answers where load/unload cannot: over
/// ssh, `launchctl list` reports a missing label (rc 113 +
/// "Could not find service") while load/unload abort with an empty
/// error.
fn launchctl_loaded(label: &str) -> Option<bool> {
    match run_bounded(&[
        "launchctl".to_string(),
        "list".to_string(),
        label.to_string(),
    ]) {
        Ok(output) => parse_launchctl_list(
            output.status.success(),
            &String::from_utf8_lossy(&output.stderr),
        ),
        Err(_) => None,
    }
}

/// A Windows temp file that is removed on every exit of its scope,
/// success or failure: the task XML must not linger when a write
/// or the enable step fails halfway.
struct TempCleanup {
    paths: Vec<PathBuf>,
}

impl TempCleanup {
    fn new(paths: Vec<PathBuf>) -> Self {
        TempCleanup { paths }
    }
}

impl Drop for TempCleanup {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = fs::remove_file(path);
        }
    }
}

/// A suffix unique to this write: pid, wall-clock nanos, and a
/// per-process counter. No randomness source is needed: the temp
/// file is created with O_EXCL, so a guessed name only fails the
/// write instead of hijacking it, and the counter keeps two writes
/// in one process apart even in the same nanosecond.
fn unique_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// Write `content` to `path` atomically: a temp file in the same
/// directory, created with O_EXCL, fsynced, then renamed over the
/// destination, with the directory fsynced so the rename itself
/// is durable. A crash or a full disk never leaves a truncated
/// destination, and a planted temp name fails the write instead
/// of being followed. The temp file is removed on every failure
/// path, so a retry never trips over this run's leftover.
fn write_file_atomically(path: &Path, content: &str) -> Result<(), GeectlError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let temp_path = parent.join(format!(".{file_name}.tmp-{}", unique_suffix()));
    let write = (|| -> std::io::Result<()> {
        let mut file = fs::File::options()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(err) = write {
        let _ = fs::remove_file(&temp_path);
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("cannot write {}: {err}", path.display()),
        ));
    }
    if let Err(err) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("cannot write {}: {err}", path.display()),
        ));
    }
    sync_dir_after_rename(parent);
    Ok(())
}

/// Sync `dir` after a rename so the rename itself is durable.
/// Unix-only: opening a directory is not portable. Best-effort:
/// the rename already happened, so a failure here must not report
/// the write as failed.
#[cfg(unix)]
fn sync_dir_after_rename(dir: &Path) {
    if let Ok(dir) = fs::File::open(dir) {
        let _ = dir.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir_after_rename(_dir: &Path) {}

/// An exclusive lock on the file `path` would write, held while
/// the write runs: two concurrent installs serialize instead of
/// racing their renames. The lock file (`<path>.lock`) is an
/// anchor that is never removed — removing it would let a second
/// process lock a deleted inode while the first still holds the
/// live one. A lock held over 5 s fails the write instead of
/// waiting forever; a crashed holder's lock is released by the
/// OS when its process dies, so no stale-lock cleanup is needed.
struct WriteLock {
    _file: fs::File,
}

impl WriteLock {
    fn acquire(path: &Path) -> Result<Self, GeectlError> {
        let lock_path = PathBuf::from(format!("{}.lock", path.display()));
        let file = fs::File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&lock_path)
            .map_err(|e| {
                GeectlError::new(
                    ErrorKind::ConfigError,
                    format!("cannot lock {}: {e}", path.display()),
                )
            })?;
        let deadline = Instant::now() + Duration::from_millis(5_000);
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(WriteLock { _file: file }),
                Err(_) => {
                    if Instant::now() >= deadline {
                        return Err(GeectlError::new(
                            ErrorKind::ConfigError,
                            format!("cannot lock {}: timed out after 5s", path.display()),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        let _ = self._file.unlock();
    }
}

/// Write `content` to `path` under the exclusive lock, atomically:
/// the lock serializes concurrent writers, the temp file plus
/// rename keeps every reader on a complete file.
fn write_file_locked(path: &Path, content: &str) -> Result<(), GeectlError> {
    let _lock = WriteLock::acquire(path)?;
    write_file_atomically(path, content)
}

/// Install the schedule for `os`: write the files, then run the
/// enable commands. Every file is written atomically under an
/// exclusive lock, so concurrent installs never leave a torn file.
/// A dry run writes nothing and runs nothing, but
/// lists the full plan: on macOS a live run skips the unload step
/// when `launchctl list` reports nothing loaded. An enable failure
/// leaves the files written and reports it, so a retry only
/// re-runs the enable step.
pub fn install(
    os: PeriodicOs,
    exe: &Path,
    home: &Path,
    xdg_config_home: Option<&str>,
    dry_run: bool,
) -> Result<PeriodicOutcome, GeectlError> {
    let plan = install_plan(os, exe, home, xdg_config_home)?;
    let files: Vec<PathBuf> = plan.files.iter().map(|f| f.path.clone()).collect();
    if dry_run {
        // The Windows import XML is transient: the dry run names
        // no file the live run would keep, so it reports none.
        let files = if os == PeriodicOs::Windows {
            vec![]
        } else {
            files
        };
        return Ok(PeriodicOutcome {
            platform: os,
            label: plan.label,
            dry_run: true,
            files,
            commands: plan
                .enable
                .iter()
                .map(|command| command.argv.clone())
                .collect(),
        });
    }
    // The Windows task XML is schtasks' input, not the schedule:
    // the guard removes it on every exit, and the outcome reports
    // no files either way.
    let _temp_guard = if os == PeriodicOs::Windows {
        Some(TempCleanup::new(files.clone()))
    } else {
        None
    };
    for file in &plan.files {
        if let Some(parent) = file.path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                GeectlError::new(
                    ErrorKind::ConfigError,
                    format!("cannot create {}: {e}", parent.display()),
                )
            })?;
        }
        write_file_locked(&file.path, &file.content)?;
    }
    let mut enable = plan.enable;
    if os == PeriodicOs::MacOs && launchctl_loaded(&plan.label) == Some(false) {
        // Nothing loaded: the unload-first step is vacuous, and
        // over ssh it aborts before the load ever runs. Find the
        // unload by its argv, not its position: a plan reorder
        // must not silently drop the load instead.
        let Some(position) = enable
            .iter()
            .position(|c| c.argv.get(1).is_some_and(|arg| arg == "unload"))
        else {
            return Err(GeectlError::new(
                ErrorKind::InternalError,
                "macOS install plan carries no unload step",
            ));
        };
        enable.remove(position);
    }
    let commands = run_all(os, &enable)?;
    let files = if os == PeriodicOs::Windows {
        vec![]
    } else {
        files
    };
    Ok(PeriodicOutcome {
        platform: os,
        label: plan.label,
        dry_run: false,
        files,
        commands,
    })
}

/// Uninstall the schedule for `os`: run the disable commands, then
/// remove the files. A dry run removes nothing and runs nothing,
/// but lists the full disable plan: on macOS a live run skips the
/// unload when `launchctl list` reports nothing loaded. A disable
/// failure leaves the files in place: a timer that may still fire
/// must not lose its unit file first.
pub fn uninstall(
    os: PeriodicOs,
    home: &Path,
    xdg_config_home: Option<&str>,
    dry_run: bool,
) -> Result<PeriodicOutcome, GeectlError> {
    let plan = uninstall_plan(os, home, xdg_config_home)?;
    let files: Vec<PathBuf> = plan.files.iter().map(|f| f.path.clone()).collect();
    if dry_run {
        return Ok(PeriodicOutcome {
            platform: os,
            label: plan.label,
            dry_run: true,
            files,
            commands: plan
                .disable
                .iter()
                .map(|command| command.argv.clone())
                .collect(),
        });
    }
    let mut disable = plan.disable;
    if os == PeriodicOs::MacOs && launchctl_loaded(&plan.label) == Some(false) {
        // Nothing loaded: there is nothing to unload, and over ssh
        // the unload would abort before the files are even touched.
        disable.clear();
    }
    let commands = run_all(os, &disable)?;
    for file in &plan.files {
        match fs::remove_file(&file.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(GeectlError::new(
                    ErrorKind::ConfigError,
                    format!("cannot remove {}: {e}", file.path.display()),
                ));
            }
        }
    }
    // A deliberate uninstall opts out of the first-run auto-install:
    // record the decision so the next build does not silently put
    // the schedule back.
    record_auto_install_decided(home, xdg_config_home);
    Ok(PeriodicOutcome {
        platform: os,
        label: plan.label,
        dry_run: false,
        files,
        commands,
    })
}

/// What the scheduler reports about the task. `enabled` and
/// `active` are `None` when the scheduler cannot be asked or its
/// answer names no known state: unknown, not disabled. `exists` is
/// `Some` only where the query itself answers it (Windows: the
/// query succeeding means the task exists); elsewhere the files
/// answer it.
struct SchedulerState {
    enabled: Option<bool>,
    active: Option<bool>,
    exists: Option<bool>,
}

/// Read `systemctl --user is-enabled` output: the state word on
/// stdout names the answer. Only a known word counts: an empty or
/// unrecognised stdout (the user manager unreachable, D-Bus down)
/// is unknown, not disabled.
fn parse_is_enabled(stdout: &str) -> Option<bool> {
    match stdout.trim() {
        "enabled" | "enabled-runtime" | "alias" | "linked" | "linked-runtime" | "indirect"
        | "generated" | "transient" => Some(true),
        "disabled" | "static" | "masked" | "masked-runtime" | "bad" | "not-found" => Some(false),
        _ => None,
    }
}

/// Read `systemctl --user is-active` output the same way: only a
/// known state word counts. `reloading` and `refreshing` are active
/// states (the man page: the unit is active and reloading its
/// configuration, or active and activating a new mount).
fn parse_is_active(stdout: &str) -> Option<bool> {
    match stdout.trim() {
        "active" | "reloading" | "refreshing" => Some(true),
        "inactive" | "failed" | "activating" | "deactivating" | "maintenance" => Some(false),
        _ => None,
    }
}

/// Read `launchctl list <label>` output: success means loaded.
/// A failure that says the service cannot be found means not
/// loaded; any other failure is unknown.
fn parse_launchctl_list(success: bool, stderr: &str) -> Option<bool> {
    if success {
        Some(true)
    } else if stderr.contains("Could not find service")
        || stderr.contains("Could not find specified service")
    {
        Some(false)
    } else {
        None
    }
}

/// Read `schtasks /query /fo LIST /v` output: only a known `Status:`
/// value counts. A missing or unrecognised value — including a
/// non-English locale, where the line reads e.g. `Deaktiviert` —
/// is unknown, not enabled.
fn parse_schtasks_status(stdout: &str) -> (Option<bool>, Option<bool>) {
    let status = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Status:"))
        .map(str::trim);
    match status {
        Some("Disabled") => (Some(false), Some(false)),
        Some("Running") => (Some(true), Some(true)),
        Some("Ready") | Some("Queued") => (Some(true), Some(false)),
        _ => (None, None),
    }
}

/// Ask the scheduler whether the task is registered, and whether it
/// is currently running. A missing command or a failed query is
/// unknown (`None`), not disabled: the query failing must not report
/// a schedule that exists as absent.
fn query_enabled_active(os: PeriodicOs, label: &str) -> SchedulerState {
    match os {
        PeriodicOs::Linux => {
            let enabled = match run_bounded(&[
                "systemctl".to_string(),
                "--user".to_string(),
                "is-enabled".to_string(),
                label.to_string(),
            ]) {
                Ok(output) => parse_is_enabled(&String::from_utf8_lossy(&output.stdout)),
                Err(_) => None,
            };
            let active = match run_bounded(&[
                "systemctl".to_string(),
                "--user".to_string(),
                "is-active".to_string(),
                label.to_string(),
            ]) {
                Ok(output) => parse_is_active(&String::from_utf8_lossy(&output.stdout)),
                Err(_) => None,
            };
            SchedulerState {
                enabled,
                active,
                exists: None,
            }
        }
        PeriodicOs::MacOs => {
            // launchctl reports no persistent active state for a
            // calendar agent: loaded is the whole story.
            let enabled = match run_bounded(&[
                "launchctl".to_string(),
                "list".to_string(),
                label.to_string(),
            ]) {
                Ok(output) => parse_launchctl_list(
                    output.status.success(),
                    &String::from_utf8_lossy(&output.stderr),
                ),
                Err(_) => None,
            };
            SchedulerState {
                enabled,
                active: None,
                exists: None,
            }
        }
        PeriodicOs::Windows => {
            let query = vec![
                "schtasks".to_string(),
                "/query".to_string(),
                "/tn".to_string(),
                label.to_string(),
                "/fo".to_string(),
                "LIST".to_string(),
                "/v".to_string(),
            ];
            match run_bounded(&query) {
                Ok(output) if output.status.success() => {
                    // The query succeeding means the task exists,
                    // whatever the Status line says.
                    let (enabled, active) =
                        parse_schtasks_status(&String::from_utf8_lossy(&output.stdout));
                    SchedulerState {
                        enabled,
                        active,
                        exists: Some(true),
                    }
                }
                Ok(output) => {
                    // A failed query names a missing task only when
                    // schtasks says so; anything else is unknown.
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if stderr.to_lowercase().contains("cannot find the file") {
                        SchedulerState {
                            enabled: Some(false),
                            active: Some(false),
                            exists: Some(false),
                        }
                    } else {
                        SchedulerState {
                            enabled: None,
                            active: None,
                            exists: None,
                        }
                    }
                }
                Err(_) => SchedulerState {
                    enabled: None,
                    active: None,
                    exists: None,
                },
            }
        }
    }
}

/// The status of the schedule for `os`: whether its files exist,
/// and what the scheduler reports. On Windows the task owns no
/// file, so installed is the scheduler query itself — and the
/// transient import XML is not listed as a file.
pub fn status(
    os: PeriodicOs,
    home: &Path,
    xdg_config_home: Option<&str>,
) -> Result<PeriodicStatus, GeectlError> {
    let plan = install_plan(os, Path::new("geectl"), home, xdg_config_home)?;
    let state = query_enabled_active(os, &plan.label);
    let installed = match os {
        // The query is the only answer on Windows: a failed query
        // is unknown, not absent.
        PeriodicOs::Windows => state.exists,
        _ => Some(plan.files.iter().all(|f| f.path.is_file())),
    };
    let files = match os {
        PeriodicOs::Windows => vec![],
        _ => plan.files.iter().map(|f| f.path.clone()).collect(),
    };
    Ok(PeriodicStatus {
        platform: os,
        label: plan.label,
        installed,
        enabled: state.enabled,
        active: state.active,
        files,
    })
}

/// The auto-install marker: `$XDG_CONFIG_HOME/geectl/periodic-auto-install`,
/// else `~/.config/geectl/periodic-auto-install`. Its presence means the
/// first-run decision is made: the schedule was installed (by
/// auto-install or by hand), or the user opted out with
/// `periodic-uninstall`. It is never removed, so an uninstall stays
/// uninstalled until a manual `periodic-install`.
pub fn auto_install_marker_path(
    home: &Path,
    xdg_config_home: Option<&str>,
) -> Result<PathBuf, GeectlError> {
    let dir = match xdg_config_home {
        Some(xdg) if !xdg.trim().is_empty() => PathBuf::from(xdg.trim()).join("geectl"),
        _ => home.join(".config").join("geectl"),
    };
    if !dir.is_absolute() {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!(
                "periodic auto-install marker directory is not absolute: {}",
                dir.display()
            ),
        ));
    }
    Ok(dir.join("periodic-auto-install"))
}

/// The negative-cache marker: `$XDG_CONFIG_HOME/geectl/periodic-auto-install-failed`,
/// else `~/.config/geectl/periodic-auto-install-failed`. It carries
/// the unix time of the last failed auto-install attempt; while it
/// is younger than the retry window, later builds skip the
/// scheduler probes instead of retrying a failure that has not
/// changed. Recording the decision removes it.
pub fn auto_install_failed_marker_path(
    home: &Path,
    xdg_config_home: Option<&str>,
) -> Result<PathBuf, GeectlError> {
    let dir = match xdg_config_home {
        Some(xdg) if !xdg.trim().is_empty() => PathBuf::from(xdg.trim()).join("geectl"),
        _ => home.join(".config").join("geectl"),
    };
    if !dir.is_absolute() {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!(
                "periodic auto-install marker directory is not absolute: {}",
                dir.display()
            ),
        ));
    }
    Ok(dir.join("periodic-auto-install-failed"))
}

/// Record that the auto-install decision is made: create the marker
/// file (and its directory) when missing. Best effort by design: a
/// marker that cannot be written only means the next build asks
/// again, never a failure.
fn record_auto_install_decided(home: &Path, xdg_config_home: Option<&str>) {
    let Ok(marker) = auto_install_marker_path(home, xdg_config_home) else {
        return;
    };
    if let Some(parent) = marker.parent() {
        let _ = fs::create_dir_all(parent);
    }
    // Atomic under the lock, like every other periodic write: two
    // concurrent first builds record the same decision without
    // racing. A decision supersedes a failed attempt, so a stale
    // negative-cache marker goes with it.
    let _ = write_file_locked(&marker, "auto-install decided\n");
    if let Ok(failed) = auto_install_failed_marker_path(home, xdg_config_home) {
        let _ = fs::remove_file(failed);
    }
}

/// Record that the last auto-install attempt failed: write the
/// current unix time to the negative-cache marker (and its
/// directory) when missing. Best effort by design, like the
/// decision marker: a marker that cannot be written only means
/// the next build asks again, never a failure.
fn record_auto_install_failed(home: &Path, xdg_config_home: Option<&str>) {
    let Ok(marker) = auto_install_failed_marker_path(home, xdg_config_home) else {
        return;
    };
    if let Some(parent) = marker.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = write_file_locked(&marker, &format!("{now}\n"));
}

/// Whether the last failed auto-install attempt is still inside
/// the retry window: a missing, unreadable or unparsable marker
/// counts as no failure, so a corrupt marker never suppresses
/// the install forever.
fn auto_install_failed_recently(home: &Path, xdg_config_home: Option<&str>) -> bool {
    let Ok(marker) = auto_install_failed_marker_path(home, xdg_config_home) else {
        return false;
    };
    let Ok(content) = fs::read_to_string(&marker) else {
        return false;
    };
    let Ok(failed_at) = content.trim().parse::<u64>() else {
        return false;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    now.saturating_sub(failed_at) < AUTO_INSTALL_RETRY_AFTER.as_secs()
}

/// The one-line notice printed when the first-run install happens.
fn auto_install_notice(os: PeriodicOs, label: &str) -> String {
    format!(
        "periodic: installed the daily cache cleanup ({}, {label}); run geectl local-build periodic-uninstall to remove it",
        os.name()
    )
}

/// Whether the schedule counts as installed: the files exist (or the
/// query answers so on Windows), and the scheduler confirms it
/// enabled. A unit the scheduler reports disabled — or cannot be
/// asked about — is not installed: the enable step failed, never
/// ran, or cannot be verified, so the next build retries the
/// install instead of recording the decision.
fn already_installed(status: &PeriodicStatus) -> bool {
    status.installed == Some(true) && status.enabled == Some(true)
}

/// Install the schedule on the first live `local-build` run, unless
/// the decision is already made. Returns the one-line notice when an
/// install happened, `None` otherwise. Never errors: every failure
/// is a quiet skip, so a missing scheduler can never fail the build
/// that triggered it. An install that fails writes no marker, so a
/// later build retries — whether the files were never written or
/// the enable failed and left them behind: the decision is
/// recorded only once the scheduler confirms the unit enabled. An
/// install that fails writes the failed-attempt marker, so later
/// builds skip the probes until the retry window (a day) passes.
/// A deliberate uninstall records the opt-out, so it is never
/// re-installed.
pub fn maybe_auto_install(
    os: PeriodicOs,
    exe: &Path,
    home: &Path,
    xdg_config_home: Option<&str>,
) -> Option<String> {
    let marker = auto_install_marker_path(home, xdg_config_home).ok()?;
    if marker.is_file() {
        return None;
    }
    // Already installed by hand: record the decision and stop. The
    // files answer it on Linux/macOS; on Windows the query does. A
    // scheduler that cannot be asked leaves the decision unrecorded,
    // so a later build retries once the scheduler answers; a unit
    // the scheduler reports disabled is not installed either, so it
    // falls through to the install attempt below.
    if let Ok(status) = status(os, home, xdg_config_home) {
        if already_installed(&status) {
            record_auto_install_decided(home, xdg_config_home);
            return None;
        }
    }
    // A failed attempt is negatively cached: without this every
    // build re-runs the scheduler probes and the install attempt
    // for a failure that has not changed.
    if auto_install_failed_recently(home, xdg_config_home) {
        return None;
    }
    let outcome = match install(os, exe, home, xdg_config_home, false) {
        Ok(outcome) => outcome,
        Err(_) => {
            record_auto_install_failed(home, xdg_config_home);
            return None;
        }
    };
    record_auto_install_decided(home, xdg_config_home);
    Some(auto_install_notice(os, &outcome.label))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn exe() -> PathBuf {
        PathBuf::from("/usr/bin/geectl")
    }

    /// The fake home: absolute on every platform. A Unix-style
    /// `/home/tester` is drive-relative, not absolute, on Windows,
    /// so the absolute-path checks reject every plan built from it.
    fn home() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from("C:\\Users\\tester")
        } else {
            PathBuf::from("/home/tester")
        }
    }

    /// The fake `$XDG_CONFIG_HOME`: absolute on every platform, for
    /// the same reason as `home`.
    fn xdg_config_home() -> &'static str {
        if cfg!(windows) {
            "C:\\xdg\\config"
        } else {
            "/xdg/config"
        }
    }

    #[test]
    fn fixtures_are_absolute_on_every_platform() {
        assert!(home().is_absolute());
        assert!(PathBuf::from(xdg_config_home()).is_absolute());
    }

    #[test]
    fn systemd_units_carry_the_binary_and_0300() {
        let service = render_systemd_service(&exe()).unwrap();
        assert!(service.contains("ExecStart=/usr/bin/geectl local-build prune --apply"));
        assert!(service.contains("Type=oneshot"));
        let timer = render_systemd_timer().unwrap();
        assert!(timer.contains("OnCalendar=*-*-* 03:00:00"));
        assert!(timer.contains("Persistent=true"));
        assert!(timer.contains("WantedBy=timers.target"));
    }

    #[test]
    fn systemd_exec_quotes_and_escapes_paths() {
        let service = render_systemd_service(Path::new("/home/test user/bin/geectl")).unwrap();
        assert!(
            service.contains("ExecStart=\"/home/test user/bin/geectl\" local-build prune --apply")
        );
        // An apostrophe quotes the word; `%` escapes even
        // unquoted, or the unit would expand it — while `$` stays
        // literal in the first word, which is the binary path.
        let service = render_systemd_service(Path::new("/opt/o'brien/geectl")).unwrap();
        assert!(service.contains("ExecStart=\"/opt/o'brien/geectl\" local-build prune --apply"));
        let service = render_systemd_service(Path::new("/opt/a$b%c/geectl")).unwrap();
        assert!(service.contains("ExecStart=/opt/a$b%%c/geectl local-build prune --apply"));
        let service = render_systemd_service(Path::new("/opt/a\"b\\c/geectl")).unwrap();
        assert!(service.contains("ExecStart=\"/opt/a\\\"b\\\\c/geectl\" local-build prune --apply"));
    }

    #[test]
    fn launchd_plist_carries_the_binary_log_and_0300() {
        let plist = render_launchd_plist(&exe(), &home()).unwrap();
        assert!(plist.contains("<string>/usr/bin/geectl</string>"));
        assert!(plist.contains("<string>local-build</string>"));
        assert!(plist.contains("<string>--apply</string>"));
        assert!(plist.contains("<string>io.github.scarabeusiv.geectl.prune</string>"));
        // The template carries the `/Library/Logs/...` suffix
        // literally; only the home prefix is substituted.
        let log = format!(
            "<string>{}/Library/Logs/geectl-prune.log</string>",
            home().display()
        );
        assert!(plist.contains(&log));
        assert!(plist.contains("<key>Hour</key>\n\t\t<integer>3</integer>"));
        assert!(plist.contains("<key>Minute</key>\n\t\t<integer>0</integer>"));
    }

    #[test]
    fn scheduled_task_xml_carries_the_binary_and_daily_0300() {
        let xml = render_scheduled_task_xml(&exe()).unwrap();
        assert!(xml.contains("<Command>/usr/bin/geectl</Command>"));
        assert!(xml.contains("<Arguments>local-build prune --apply</Arguments>"));
        assert!(xml.contains("<StartBoundary>2026-01-01T03:00:00</StartBoundary>"));
        assert!(xml.contains("<DaysInterval>1</DaysInterval>"));
        assert!(xml.contains("<LogonType>InteractiveToken</LogonType>"));
    }

    #[test]
    fn xml_paths_are_escaped() {
        let plist = render_launchd_plist(Path::new("/opt/a&b/geectl"), &home()).unwrap();
        assert!(plist.contains("<string>/opt/a&amp;b/geectl</string>"));
        let xml = render_scheduled_task_xml(Path::new("/opt/a<b/geectl")).unwrap();
        assert!(xml.contains("<Command>/opt/a&lt;b/geectl</Command>"));
        let xml = render_scheduled_task_xml(Path::new("/opt/a>b/geectl")).unwrap();
        assert!(xml.contains("<Command>/opt/a&gt;b/geectl</Command>"));
    }

    #[test]
    fn systemd_user_dir_honours_xdg_config_home() {
        let xdg = xdg_config_home();
        let xdg_padded = format!("  {xdg}  ");
        assert_eq!(
            systemd_user_dir(Some(xdg), &home()).unwrap(),
            PathBuf::from(xdg).join("systemd").join("user")
        );
        assert_eq!(
            systemd_user_dir(Some("   "), &home()).unwrap(),
            home().join(".config").join("systemd").join("user")
        );
        assert_eq!(
            systemd_user_dir(None, &home()).unwrap(),
            home().join(".config").join("systemd").join("user")
        );
        assert_eq!(
            systemd_user_dir(Some(&xdg_padded), &home()).unwrap(),
            PathBuf::from(xdg).join("systemd").join("user")
        );
        assert!(systemd_user_dir(Some("relative/config"), &home()).is_err());
    }

    #[test]
    fn linux_plan_writes_both_units_and_enables_the_timer() {
        let plan = install_plan(PeriodicOs::Linux, &exe(), &home(), None).unwrap();
        assert_eq!(plan.label, "geectl-prune.timer");
        assert_eq!(plan.files.len(), 2);
        assert_eq!(
            plan.files[0].path,
            home()
                .join(".config")
                .join("systemd")
                .join("user")
                .join("geectl-prune.service")
        );
        assert_eq!(
            plan.files[1].path,
            home()
                .join(".config")
                .join("systemd")
                .join("user")
                .join("geectl-prune.timer")
        );
        assert!(plan.files[0]
            .content
            .contains("ExecStart=/usr/bin/geectl local-build prune --apply"));
        assert_eq!(plan.enable.len(), 2);
        assert_eq!(
            plan.enable[0].argv,
            vec!["systemctl", "--user", "daemon-reload"]
        );
        assert!(!plan.enable[0].tolerate_missing);
        assert_eq!(
            plan.enable[1].argv,
            vec![
                "systemctl",
                "--user",
                "enable",
                "--now",
                "geectl-prune.timer"
            ]
        );
        assert!(!plan.enable[1].tolerate_missing);
        assert_eq!(plan.disable.len(), 2);
        assert_eq!(
            plan.disable[0].argv,
            vec![
                "systemctl",
                "--user",
                "disable",
                "--now",
                "geectl-prune.timer"
            ]
        );
        assert!(plan.disable[0].tolerate_missing);
        assert_eq!(
            plan.disable[1].argv,
            vec!["systemctl", "--user", "daemon-reload"]
        );
        assert!(!plan.disable[1].tolerate_missing);
    }

    #[test]
    fn macos_plan_writes_the_plist_and_loads_it() {
        let plan = install_plan(PeriodicOs::MacOs, &exe(), &home(), None).unwrap();
        assert_eq!(plan.label, "io.github.scarabeusiv.geectl.prune");
        assert_eq!(plan.files.len(), 1);
        let plist = home()
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{LAUNCHD_LABEL}.plist"));
        assert_eq!(plan.files[0].path, plist);
        // Unload first, so a re-install picks up the new plist
        // instead of failing on an already-loaded agent; the
        // unload tolerates a fresh install with nothing loaded.
        let plist_arg = plist.display().to_string();
        assert_eq!(plan.enable.len(), 2);
        assert_eq!(
            plan.enable[0].argv,
            vec!["launchctl", "unload", plist_arg.as_str()]
        );
        assert!(plan.enable[0].tolerate_missing);
        assert_eq!(
            plan.enable[1].argv,
            vec!["launchctl", "load", plist_arg.as_str()]
        );
        assert!(!plan.enable[1].tolerate_missing);
        assert_eq!(plan.disable.len(), 1);
        assert_eq!(
            plan.disable[0].argv,
            vec!["launchctl", "unload", plist_arg.as_str()]
        );
        assert!(plan.disable[0].tolerate_missing);
    }

    #[test]
    fn windows_plan_writes_the_xml_to_temp_and_creates_the_task() {
        let plan = install_plan(PeriodicOs::Windows, &exe(), &home(), None).unwrap();
        assert_eq!(plan.label, "geectl-prune");
        assert_eq!(plan.files.len(), 1);
        let file_name = plan.files[0].path.file_name().unwrap().to_string_lossy();
        assert!(file_name.starts_with("geectl-prune-"));
        assert!(file_name.ends_with(".xml"));
        assert_eq!(plan.files[0].path.parent().unwrap(), std::env::temp_dir());
        assert!(plan.files[0]
            .content
            .contains("<Command>/usr/bin/geectl</Command>"));
        assert_eq!(plan.enable.len(), 1);
        assert_eq!(plan.enable[0].argv[0], "schtasks");
        assert_eq!(plan.enable[0].argv[1], "/create");
        assert_eq!(plan.enable[0].argv[3], "geectl-prune");
        assert!(!plan.enable[0].tolerate_missing);
        assert_eq!(plan.disable[0].argv[1], "/delete");
        assert!(plan.disable[0].tolerate_missing);
    }

    #[test]
    fn uninstall_plan_disables_first_and_carries_no_enable() {
        let plan = uninstall_plan(PeriodicOs::Linux, &home(), None).unwrap();
        assert!(plan.enable.is_empty());
        assert_eq!(plan.files.len(), 2);
        assert_eq!(plan.disable[0].argv[2], "disable");
        let win = uninstall_plan(PeriodicOs::Windows, &home(), None).unwrap();
        assert!(win.files.is_empty());
        assert_eq!(win.disable[0].argv[1], "/delete");
    }

    #[test]
    fn scheduler_state_parsers_read_known_states_only() {
        assert_eq!(parse_is_enabled("enabled\n"), Some(true));
        assert_eq!(parse_is_enabled("enabled-runtime\n"), Some(true));
        assert_eq!(parse_is_enabled("linked-runtime\n"), Some(true));
        assert_eq!(parse_is_enabled("alias\n"), Some(true));
        assert_eq!(parse_is_enabled("indirect\n"), Some(true));
        assert_eq!(parse_is_enabled("generated\n"), Some(true));
        assert_eq!(parse_is_enabled("transient\n"), Some(true));
        assert_eq!(parse_is_enabled("disabled\n"), Some(false));
        assert_eq!(parse_is_enabled("static\n"), Some(false));
        assert_eq!(parse_is_enabled("masked-runtime\n"), Some(false));
        assert_eq!(parse_is_enabled("bad\n"), Some(false));
        assert_eq!(parse_is_enabled("masked\n"), Some(false));
        assert_eq!(parse_is_enabled("not-found\n"), Some(false));
        assert_eq!(parse_is_enabled(""), None);
        assert_eq!(parse_is_active("active\n"), Some(true));
        assert_eq!(parse_is_active("reloading\n"), Some(true));
        assert_eq!(parse_is_active("refreshing\n"), Some(true));
        assert_eq!(parse_is_active("inactive\n"), Some(false));
        assert_eq!(parse_is_active("failed\n"), Some(false));
        assert_eq!(parse_is_active("activating\n"), Some(false));
        assert_eq!(parse_is_active("deactivating\n"), Some(false));
        assert_eq!(parse_is_active("maintenance\n"), Some(false));
        assert_eq!(parse_is_active(""), None);
        assert_eq!(parse_launchctl_list(true, ""), Some(true));
        assert_eq!(
            parse_launchctl_list(false, "Could not find service \"x\" in domain for port\n"),
            Some(false)
        );
        assert_eq!(
            parse_launchctl_list(false, "Could not find specified service\n"),
            Some(false)
        );
        assert_eq!(parse_launchctl_list(false, "some other failure\n"), None);
        assert_eq!(
            parse_schtasks_status("TaskName: geectl-prune\nStatus: Ready\n"),
            (Some(true), Some(false))
        );
        assert_eq!(
            parse_schtasks_status("Status: Running\n"),
            (Some(true), Some(true))
        );
        assert_eq!(
            parse_schtasks_status("Status: Queued\n"),
            (Some(true), Some(false))
        );
        assert_eq!(
            parse_schtasks_status("Status: Disabled\n"),
            (Some(false), Some(false))
        );
        assert_eq!(parse_schtasks_status("Status: Deaktiviert\n"), (None, None));
        assert_eq!(
            parse_schtasks_status("TaskName: geectl-prune\n"),
            (None, None)
        );
    }

    #[test]
    fn dry_run_install_and_uninstall_touch_nothing() {
        let home = std::env::temp_dir().join(format!("geectl-191-dry-run-{}", std::process::id()));
        assert!(!home.exists());
        let outcome = install(PeriodicOs::Linux, &exe(), &home, None, true).unwrap();
        assert!(outcome.dry_run);
        assert_eq!(outcome.files.len(), 2);
        assert_eq!(outcome.commands.len(), 2);
        assert!(!home.exists());
        let outcome = uninstall(PeriodicOs::Linux, &home, None, true).unwrap();
        assert!(outcome.dry_run);
        assert_eq!(outcome.files.len(), 2);
        assert_eq!(outcome.commands.len(), 2);
        assert!(!home.exists());
        // macOS: one file; install lists the unload-first pair.
        let outcome = install(PeriodicOs::MacOs, &exe(), &home, None, true).unwrap();
        assert_eq!(outcome.files.len(), 1);
        assert_eq!(outcome.commands.len(), 2);
        assert!(!home.exists());
        let outcome = uninstall(PeriodicOs::MacOs, &home, None, true).unwrap();
        assert_eq!(outcome.files.len(), 1);
        assert_eq!(outcome.commands.len(), 1);
        assert!(!home.exists());
        // Windows: the transient import XML is never reported, and
        // never written.
        let plan = install_plan(PeriodicOs::Windows, &exe(), &home, None).unwrap();
        assert!(!plan.files[0].path.exists());
        let outcome = install(PeriodicOs::Windows, &exe(), &home, None, true).unwrap();
        assert!(outcome.files.is_empty());
        assert_eq!(outcome.commands.len(), 1);
        assert!(!plan.files[0].path.exists());
        assert!(!home.exists());
        let outcome = uninstall(PeriodicOs::Windows, &home, None, true).unwrap();
        assert!(outcome.files.is_empty());
        assert_eq!(outcome.commands.len(), 1);
        assert!(!home.exists());
    }

    #[test]
    #[cfg(unix)]
    fn run_command_tolerates_a_missing_target() {
        let missing = PeriodicCommand::tolerating(vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo 'does not exist' >&2; exit 1".to_string(),
        ]);
        assert!(run_command(PeriodicOs::Linux, &missing).is_ok());
        let strict = PeriodicCommand::strict(vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo 'does not exist' >&2; exit 1".to_string(),
        ]);
        assert!(run_command(PeriodicOs::Linux, &strict).is_err());
        let other = PeriodicCommand::tolerating(vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo 'something else broke' >&2; exit 1".to_string(),
        ]);
        assert!(run_command(PeriodicOs::Linux, &other).is_err());
        // The macOS and Windows phrases are matched the same way,
        // whatever the host: the phrase, not the platform, decides.
        let macos = PeriodicCommand::tolerating(vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo 'Could not find service \"x\" in domain for port' >&2; exit 1".to_string(),
        ]);
        assert!(run_command(PeriodicOs::MacOs, &macos).is_ok());
        let windows = PeriodicCommand::tolerating(vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo 'ERROR: The system cannot find the file specified.' >&2; exit 1".to_string(),
        ]);
        assert!(run_command(PeriodicOs::Windows, &windows).is_ok());
    }

    #[test]
    #[cfg(unix)]
    fn run_command_error_names_the_signal() {
        let abort = PeriodicCommand::strict(vec![
            "sh".to_string(),
            "-c".to_string(),
            "kill -ABRT $$".to_string(),
        ]);
        let err = run_command(PeriodicOs::Linux, &abort).unwrap_err();
        assert!(err.to_string().contains("signal"), "{err}");
    }

    #[test]
    fn periodic_os_names() {
        assert_eq!(PeriodicOs::Linux.name(), "linux");
        assert_eq!(PeriodicOs::MacOs.name(), "macos");
        assert_eq!(PeriodicOs::Windows.name(), "windows");
    }

    #[test]
    fn run_bounded_reports_a_missing_command() {
        let err = run_bounded(&["geectl-no-such-command-xyz".to_string()]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
    }

    #[test]
    fn run_bounded_returns_output_for_a_fast_command() {
        #[cfg(unix)]
        let argv = vec!["echo".to_string(), "hi".to_string()];
        #[cfg(windows)]
        let argv = vec![
            "cmd".to_string(),
            "/C".to_string(),
            "echo".to_string(),
            "hi".to_string(),
        ];
        let output = run_bounded(&argv).unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hi");
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let dir = std::env::temp_dir().join(format!(
                "geectl-periodic-test-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn auto_install_marker_path_honours_xdg_config_home() {
        let xdg = xdg_config_home();
        assert_eq!(
            auto_install_marker_path(&home(), Some(xdg)).unwrap(),
            PathBuf::from(xdg)
                .join("geectl")
                .join("periodic-auto-install")
        );
        assert_eq!(
            auto_install_marker_path(&home(), None).unwrap(),
            home()
                .join(".config")
                .join("geectl")
                .join("periodic-auto-install")
        );
        assert!(auto_install_marker_path(&home(), Some("relative/xdg")).is_err());
    }

    #[test]
    fn auto_install_notice_names_the_platform_and_the_removal() {
        assert_eq!(
            auto_install_notice(PeriodicOs::Linux, "geectl-prune.timer"),
            "periodic: installed the daily cache cleanup (linux, geectl-prune.timer); run geectl local-build periodic-uninstall to remove it"
        );
    }

    #[test]
    fn already_installed_needs_the_scheduler_not_to_report_disabled() {
        let status_with_enabled = |enabled: Option<bool>| PeriodicStatus {
            platform: PeriodicOs::Linux,
            label: "geectl-prune.timer".to_string(),
            installed: Some(true),
            enabled,
            active: None,
            files: vec![],
        };
        assert!(already_installed(&status_with_enabled(Some(true))));
        assert!(!already_installed(&status_with_enabled(None)));
        assert!(!already_installed(&status_with_enabled(Some(false))));
        assert!(!already_installed(&PeriodicStatus {
            installed: Some(false),
            ..status_with_enabled(Some(true))
        }));
        assert!(!already_installed(&PeriodicStatus {
            installed: None,
            ..status_with_enabled(Some(true))
        }));
    }

    #[test]
    fn maybe_auto_install_skips_when_marker_present() {
        let tmp = TempDir::new("marker-present");
        let home = tmp.0.join("home");
        let xdg = tmp.0.join("xdg");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&xdg).unwrap();
        let xdg = xdg.to_str().unwrap();
        record_auto_install_decided(&home, Some(xdg));
        assert_eq!(
            maybe_auto_install(PeriodicOs::Linux, &exe(), &home, Some(xdg)),
            None
        );
        // No install was attempted: no unit files exist.
        assert!(!systemd_user_dir(Some(xdg), &home)
            .unwrap()
            .join("geectl-prune.timer")
            .exists());
    }

    #[test]
    fn maybe_auto_install_does_not_decide_when_the_scheduler_cannot_be_asked() {
        // Unit files exist but the scheduler cannot confirm the
        // unit enabled: no marker is recorded, so a later build
        // retries once the scheduler answers. The enabled answer
        // comes from the real scheduler — on Linux a unit absent
        // from the real home reads disabled (or unknown with no
        // user manager), elsewhere the query fails — but every path
        // ends in an install attempt that fails without writing the
        // marker.
        let tmp = TempDir::new("unconfirmed-installed");
        let home = tmp.0.join("home");
        let xdg = tmp.0.join("xdg");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&xdg).unwrap();
        let xdg = xdg.to_str().unwrap();
        let plan = install_plan(PeriodicOs::Linux, &exe(), &home, Some(xdg)).unwrap();
        for file in &plan.files {
            std::fs::create_dir_all(file.path.parent().unwrap()).unwrap();
            std::fs::write(&file.path, &file.content).unwrap();
        }
        assert_eq!(
            maybe_auto_install(PeriodicOs::Linux, &exe(), &home, Some(xdg)),
            None
        );
        assert!(!auto_install_marker_path(&home, Some(xdg))
            .unwrap()
            .is_file());
    }

    #[test]
    fn maybe_auto_install_skips_quietly_when_the_install_fails() {
        // XDG_CONFIG_HOME points at a regular file: no unit
        // directory can be created under it, so the install fails
        // before any scheduler is involved — on every platform.
        let tmp = TempDir::new("install-fails");
        let home = tmp.0.join("home");
        let blocker = tmp.0.join("blocker");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(&blocker, "not a directory").unwrap();
        let xdg = blocker.to_str().unwrap();
        assert_eq!(
            maybe_auto_install(PeriodicOs::Linux, &exe(), &home, Some(xdg)),
            None
        );
        // A failed install writes no decision marker — and the
        // failed-attempt marker cannot be written either, because
        // its directory is the same unwritable path: a later build
        // retries.
        assert!(!auto_install_marker_path(&home, Some(xdg))
            .unwrap()
            .is_file());
        assert!(!auto_install_failed_marker_path(&home, Some(xdg))
            .unwrap()
            .is_file());
    }

    #[test]
    fn atomic_write_replaces_content_and_leaves_no_temp_files() {
        let tmp = TempDir::new("atomic-write");
        let path = tmp.0.join("geectl-prune.timer");
        write_file_locked(&path, "one\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\n");
        write_file_locked(&path, "two\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two\n");
        // No temp file survives either write: only the destination
        // and its lock anchor remain.
        let mut names: Vec<String> = std::fs::read_dir(&tmp.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["geectl-prune.timer", "geectl-prune.timer.lock"]);
    }

    #[test]
    fn windows_plan_xml_names_differ_between_installs() {
        // The temp name carries a unique suffix, not just the pid:
        // two installs in one process never share one file.
        let first = install_plan(PeriodicOs::Windows, &exe(), &home(), None).unwrap();
        let second = install_plan(PeriodicOs::Windows, &exe(), &home(), None).unwrap();
        assert_ne!(first.files[0].path, second.files[0].path);
    }

    #[test]
    fn failed_auto_install_is_negatively_cached() {
        // A recent failed marker short-circuits the probes: no
        // unit files are written, and the decision marker stays
        // unwritten, so the retry window — not the failure — is
        // what suppresses the attempt.
        let tmp = TempDir::new("failed-recent");
        let home = tmp.0.join("home");
        let xdg = tmp.0.join("xdg");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&xdg).unwrap();
        let xdg = xdg.to_str().unwrap();
        record_auto_install_failed(&home, Some(xdg));
        assert!(auto_install_failed_recently(&home, Some(xdg)));
        assert_eq!(
            maybe_auto_install(PeriodicOs::Linux, &exe(), &home, Some(xdg)),
            None
        );
        assert!(!systemd_user_dir(Some(xdg), &home)
            .unwrap()
            .join("geectl-prune.timer")
            .exists());
        assert!(!auto_install_marker_path(&home, Some(xdg))
            .unwrap()
            .is_file());
    }

    #[test]
    fn stale_failed_marker_does_not_suppress_the_retry() {
        let tmp = TempDir::new("failed-stale");
        let home = tmp.0.join("home");
        let xdg = tmp.0.join("xdg");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&xdg).unwrap();
        let xdg = xdg.to_str().unwrap();
        let failed = auto_install_failed_marker_path(&home, Some(xdg)).unwrap();
        std::fs::create_dir_all(failed.parent().unwrap()).unwrap();
        std::fs::write(&failed, "1\n").unwrap();
        assert!(!auto_install_failed_recently(&home, Some(xdg)));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn failed_auto_install_writes_the_failed_marker() {
        // Linux install on macOS: the unit files write, then the
        // systemctl enable fails (no systemctl on macOS), so the
        // attempt fails and the failed marker records it.
        let tmp = TempDir::new("failed-marker-written");
        let home = tmp.0.join("home");
        let xdg = tmp.0.join("xdg");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&xdg).unwrap();
        let xdg = xdg.to_str().unwrap();
        assert_eq!(
            maybe_auto_install(PeriodicOs::Linux, &exe(), &home, Some(xdg)),
            None
        );
        assert!(!auto_install_marker_path(&home, Some(xdg))
            .unwrap()
            .is_file());
        let failed = auto_install_failed_marker_path(&home, Some(xdg)).unwrap();
        assert!(failed.is_file());
        let stored: u64 = std::fs::read_to_string(&failed)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(stored > 1_700_000_000);
        // Inside the retry window the next build skips the probes.
        assert_eq!(
            maybe_auto_install(PeriodicOs::Linux, &exe(), &home, Some(xdg)),
            None
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn uninstall_records_the_opt_out() {
        let tmp = TempDir::new("uninstall-opt-out");
        let home = tmp.0.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let outcome = uninstall(PeriodicOs::MacOs, &home, None, false).unwrap();
        assert!(!outcome.dry_run);
        assert!(auto_install_marker_path(&home, None).unwrap().is_file());
        // The next build does not put the schedule back.
        assert_eq!(
            maybe_auto_install(PeriodicOs::MacOs, &exe(), &home, None),
            None
        );
        // No install was attempted: the marker short-circuited the
        // call, so no plist file exists — a failed install would
        // have left one behind.
        assert!(!home
            .join("Library")
            .join("LaunchAgents")
            .join("io.github.scarabeusiv.geectl.prune.plist")
            .exists());
    }
}
