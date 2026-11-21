//! geectl — Geeko Controller, a machine-first CLI for agents.
//!
//! Non-interactive is the only mode: no prompts, no editor, no pager.
//! Missing required input fails fast with a structured error naming the
//! missing flag. Stdout carries only the result payload (JSON by default);
//! every diagnostic, warning, and error goes to stderr. There is exactly
//! one exit path: the error kind's exit code.

mod cli_branch;
mod cli_remote;
mod cli_request;
mod maintenance;

use clap::{Parser, Subcommand, ValueEnum};
use geectl::auth::{AuthProvider, Credentials, EnvGiteaAuth, EnvObsAuth, GiteaSshKey};
use geectl::config;
use geectl::debug::Debug;
use geectl::error::{ErrorKind, GeectlError};
use serde_json::json;
use std::io::Write;
use std::process::ExitCode;

mod cli_build;

/// The output format. JSON is the default; it is the only format an agent
/// should parse. `text` is a human-readable rendering of the same data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    Json,
    Text,
}

#[derive(Debug, Parser)]
#[command(
    name = "geectl",
    version,
    about = "Geeko Controller — a machine-first CLI for agents (OBS + src.opensuse.org)",
    long_about = None
)]
struct Cli {
    /// Output format for the result payload. Applies to every command.
    #[arg(long, global = true, value_enum, default_value = "json")]
    format: Format,

    /// Enable the debug channel: human-readable diagnostic lines on
    /// stderr (HTTP attempts, retries, library decisions). Off by
    /// default; `GEECTL_DEBUG=1` enables it without the flag.
    #[arg(long, global = true)]
    debug: bool,

    /// Also print request and response bodies on the debug channel
    /// (capped at 2048 bytes, redacted, never headers). Implies
    /// `--debug`.
    #[arg(long, global = true)]
    debug_body: bool,

    /// Actually perform mutations. Without it, only the plan is
    /// printed: dry-run is the default for mutations.
    #[arg(long, global = true)]
    live: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print version information as the result payload.
    Version,
    /// Build-status commands: results, logs, histories, binaries.
    Build {
        #[command(subcommand)]
        command: cli_build::BuildCommand,
    },
    /// Inspect or initialise the config file.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Remote reads: inspect what the distribution holds.
    Remote {
        #[command(subcommand)]
        command: cli_remote::RemoteCommand,
    },
    /// Read and act on OBS submit requests, and file new ones.
    Request {
        #[command(subcommand)]
        command: Box<cli_request::RequestCommand>,
    },
    /// Branch, copy, and link packages.
    Branch {
        #[command(subcommand)]
        // Boxed: the ten branch subcommands make BranchCommand far
        // larger than the other variants (clippy::large_enum_variant).
        command: Box<cli_branch::BranchCommand>,
    },
    /// Maintenance operations: maintained-instances listing and
    /// maintenance branching.
    Maintenance {
        #[command(subcommand)]
        command: maintenance::MaintenanceCommand,
    },
}

/// The `config` subcommands.
#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Show the effective config: endpoint URLs with their source,
    /// and credential status (never a secret value).
    Show,
    /// Write the config template (endpoints only). Refuses if the
    /// file already exists.
    Init,
}

fn run(cli: Cli, debug: &Debug) -> Result<String, GeectlError> {
    let (payload, text) = match cli.command {
        Some(Command::Version) | None => {
            let payload = json!({
                "name": "geectl",
                "version": env!("CARGO_PKG_VERSION"),
            });
            let text = format!("geectl {}", env!("CARGO_PKG_VERSION"));
            (payload, text)
        }
        Some(Command::Request { command }) => cli_request::run(*command, debug, cli.live)?,
        Some(Command::Build { command }) => cli_build::run_build(debug, command)?,
        Some(Command::Config { command }) => match command {
            ConfigCommand::Show => {
                let payload = config_show_payload()?;
                let text = render_config_show_text(&payload);
                (payload, text)
            }
            ConfigCommand::Init => {
                let payload = config_init_payload()?;
                let text = render_config_init_text(&payload);
                (payload, text)
            }
        },
        Some(Command::Remote { command }) => cli_remote::run_remote(command, debug)?,
        Some(Command::Branch { command }) => cli_branch::run_branch(*command, debug, cli.live)?,
        Some(Command::Maintenance { command }) => {
            let (payload, text) = maintenance::run(command, debug, cli.live)?;
            (payload, text)
        }
    };
    Ok(match cli.format {
        Format::Json => payload.to_string(),
        Format::Text => text,
    })
}

/// Resolve the debug channel from the `--debug` / `--debug-body`
/// flags and the `GEECTL_DEBUG` environment variable. The flag wins
/// over the environment; `--debug-body` implies `--debug`. Only the
/// exact value `1` enables the channel via the environment; unset,
/// empty, or anything else disables it. Returns `(enabled, bodies)`.
fn resolve_debug(debug_flag: bool, debug_body_flag: bool, env: Option<&str>) -> (bool, bool) {
    let enabled = debug_flag || debug_body_flag || env == Some("1");
    (enabled, debug_body_flag)
}

/// Collect argv without panicking on non-UTF-8 arguments:
/// `std::env::args()` panics on them, so go through `args_os()`
/// and convert lossily (an invalid sequence becomes U+FFFD rather
/// than crashing the run before it starts).
fn lossy_argv(args: impl Iterator<Item = std::ffi::OsString>) -> Vec<String> {
    args.map(|arg| arg.to_string_lossy().into_owned()).collect()
}

/// The startup line for the debug channel: `geectl <version>` plus
/// the arguments as invoked (skipping argv[0], the binary path).
/// There is no secret-valued flag today; a future one must be masked
/// here, by construction, before it reaches this line.
fn startup_line(argv: &[String]) -> String {
    let mut parts = vec!["geectl".to_string(), env!("CARGO_PKG_VERSION").to_string()];
    parts.extend(argv.iter().skip(1).cloned());
    parts.join(" ")
}

/// The kind of credentials a provider yields, for the debug config
/// line: `basic`, `bearer`, `ssh`, `none`, or `error`. Never a
/// username, never a value — the channel is stricter than `config
/// show`.
fn auth_kind(provider: &dyn AuthProvider) -> &'static str {
    match provider.credentials() {
        Ok(Credentials::None) => "none",
        Ok(Credentials::Basic { .. }) => "basic",
        Ok(Credentials::Bearer { .. }) => "bearer",
        Ok(Credentials::SshKey { .. }) => "ssh",
        Err(_) => "error",
    }
}

/// The Gitea auth kind for the debug config line. Gitea has two
/// providers, so the kind prefers bearer, then ssh, then error,
/// else none: a bearer token wins over a key, and a key wins over a
/// misconfiguration in the other provider.
fn gitea_auth_kind() -> &'static str {
    let bearer = EnvGiteaAuth.credentials();
    let key = GiteaSshKey.credentials();
    if matches!(bearer, Ok(Credentials::Bearer { .. })) {
        "bearer"
    } else if matches!(key, Ok(Credentials::SshKey { .. })) {
        "ssh"
    } else if bearer.is_err() || key.is_err() {
        "error"
    } else {
        "none"
    }
}

/// The config line for the debug channel: the effective OBS and
/// Gitea base hosts with their auth kinds. None on any config-load
/// failure, so a debug run never fails for want of a config line —
/// including `version`, which does not otherwise touch config.
fn debug_config_line() -> Option<String> {
    let path = config::config_path().ok()?;
    let file_config = config::load_config_at(&path).ok()?;
    let effective = config::effective_config(&file_config).ok()?;
    // effective_config resolves env > file > default, so both URLs
    // are present by construction; the fallback is only for the
    // type, never expected to fire.
    let obs_url = effective.config.obs_url.as_deref().unwrap_or("");
    let gitea_url = effective.config.gitea_url.as_deref().unwrap_or("");
    Some(format!(
        "config: obs {} (auth: {}), gitea {} (auth: {})",
        geectl::debug::url_host(obs_url),
        auth_kind(&EnvObsAuth),
        geectl::debug::url_host(gitea_url),
        gitea_auth_kind()
    ))
}

/// The `config show` payload: the effective endpoint URLs with the
/// source of each value (environment / config / default), and the
/// credential status for each provider. A status is never a secret:
/// Basic auth shows the username only, Bearer shows "configured"
/// only, and an SSH key shows its path.
fn config_show_payload() -> Result<serde_json::Value, GeectlError> {
    // The config path is resolved once and passed to the load: a
    // second `config_path()` call here could diverge from the one
    // inside `load_config()` if the environment changed in between.
    // The URL sources come from `effective_config` itself — they are
    // computed with the values, not re-derived by re-reading the
    // environment.
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let effective = config::effective_config(&file_config)?;
    Ok(json!({
        "config_file": path.to_string_lossy(),
        "obs_url": effective.config.obs_url,
        "obs_url_source": effective.obs_url_source,
        "gitea_url": effective.config.gitea_url,
        "gitea_url_source": effective.gitea_url_source,
        "obs_auth": auth_status(&EnvObsAuth),
        "gitea_auth": auth_status(&EnvGiteaAuth),
        "gitea_ssh_key": auth_status(&GiteaSshKey),
    }))
}

/// The credential status as JSON: `{"status": "configured"}` plus,
/// for Basic auth, the username (never the password); for an SSH key,
/// the path; for a misconfiguration, `{"status": "error", "message":
/// ...}` naming the variable, never the value.
fn auth_status(provider: &dyn AuthProvider) -> serde_json::Value {
    match provider.credentials() {
        Ok(Credentials::None) => json!({"status": "not_configured"}),
        Ok(Credentials::Basic { username, .. }) => {
            json!({"status": "configured", "username": username})
        }
        Ok(Credentials::Bearer { .. }) => json!({"status": "configured"}),
        Ok(Credentials::SshKey { path }) => {
            json!({"status": "configured", "path": path.to_string_lossy()})
        }
        Err(e) => json!({
            "status": "error",
            "code": e.kind().code(),
            "message": e.context,
            "remediation": e.kind().remediation(),
        }),
    }
}

/// The `config init` payload: the path created. The file itself is
/// written by `write_config_template`, which refuses to overwrite.
fn config_init_payload() -> Result<serde_json::Value, GeectlError> {
    let path = config::config_path()?;
    config::write_config_template(&path)?;
    Ok(json!({
        "config_file": path.to_string_lossy(),
        "created": true,
        "note": "Edit the endpoint URLs as needed. Credentials are never stored here — use the GEECTL_* environment variables.",
    }))
}

fn render_config_show_text(payload: &serde_json::Value) -> String {
    [
        format!(
            "config file: {}",
            payload["config_file"].as_str().unwrap_or("")
        ),
        format!(
            "obs_url: {} ({})",
            payload["obs_url"].as_str().unwrap_or(""),
            payload["obs_url_source"].as_str().unwrap_or("")
        ),
        format!(
            "gitea_url: {} ({})",
            payload["gitea_url"].as_str().unwrap_or(""),
            payload["gitea_url_source"].as_str().unwrap_or("")
        ),
        format!("obs_auth: {}", render_auth_text(&payload["obs_auth"])),
        format!("gitea_auth: {}", render_auth_text(&payload["gitea_auth"])),
        format!(
            "gitea_ssh_key: {}",
            render_auth_text(&payload["gitea_ssh_key"])
        ),
    ]
    .join("\n")
}

fn render_auth_text(auth: &serde_json::Value) -> String {
    match auth["status"].as_str().unwrap_or("") {
        "configured" => {
            if let Some(user) = auth.get("username").and_then(|v| v.as_str()) {
                format!("configured (username {user})")
            } else if let Some(path) = auth.get("path").and_then(|v| v.as_str()) {
                format!("configured ({path})")
            } else {
                "configured".to_string()
            }
        }
        "error" => format!("error: {}", auth["message"].as_str().unwrap_or("")),
        _ => "not_configured".to_string(),
    }
}

fn render_config_init_text(payload: &serde_json::Value) -> String {
    format!("created {}", payload["config_file"].as_str().unwrap_or(""))
}

/// Write a payload to stdout, terminated with a trailing newline
/// (an empty payload writes nothing).
/// A closed pipe (`BrokenPipe`, e.g. the output piped to `head`) is
/// not an error — the consumer just stopped reading. Any other write
/// failure is an internal error. `println!` would panic on either;
/// this is the single stdout writer.
fn write_payload(payload: &str) -> Result<(), GeectlError> {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    match write!(handle, "{}", terminated(payload)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(GeectlError::new(ErrorKind::InternalError, e.to_string())),
    }
}

/// Terminate a payload with a trailing newline: a payload that does
/// not end with one gains one; an already-terminated payload is
/// returned untouched (so a `cat` file ending in two newlines keeps
/// both — verbatim wins over tidiness). JSON payloads never end with
/// a newline. An empty payload stays empty, so `remote cat` of an
/// empty file prints zero bytes instead of one newline.
fn terminated(payload: &str) -> String {
    if payload.is_empty() || payload.ends_with('\n') {
        payload.to_string()
    } else {
        format!("{payload}\n")
    }
}

/// Write an error to stderr, in the selected format: the JSON error
/// payload by default, `error:` + `remediation:` lines in text mode.
/// A failure to write to stderr is ignored: there is nowhere else to
/// report it, and panicking while already handling an error helps no
/// one.
fn write_error(err: &GeectlError, format: Format) {
    let stderr = std::io::stderr();
    let mut handle = stderr.lock();
    let rendered = match format {
        Format::Json => err.to_json().to_string(),
        Format::Text => render_error_text(err),
    };
    let _ = writeln!(handle, "{rendered}");
}

/// The text rendering of an error: `error: <message>` plus
/// `remediation: <...>` lines. The message is the error's context
/// when it names the failure, else the kind's one-sentence message.
/// The exit code is unchanged.
fn render_error_text(err: &GeectlError) -> String {
    let message = if err.context.is_empty() {
        err.kind.message().to_string()
    } else {
        err.context.clone()
    };
    format!(
        "error: {message}
remediation: {}",
        err.kind.remediation()
    )
}

fn main() -> ExitCode {
    // Parse errors go through the single error path, not clap's
    // default (usage on stderr, exit 2, no JSON). Help and version are
    // the exception: they print clap's text to stdout and exit 0.
    // The argv is collected once, lossily, for the debug startup line
    // below. Parsing itself uses the raw `OsString` argv, so a
    // non-UTF-8 argument still gets clap's invalid-UTF-8 error instead
    // of being silently replaced by U+FFFD and sent on as a value.
    let argv = lossy_argv(std::env::args_os());
    let cli = match Cli::try_parse_from(std::env::args_os()) {
        Ok(cli) => cli,
        Err(e) => {
            use clap::error::ErrorKind as ClapErrorKind;
            match e.kind() {
                ClapErrorKind::DisplayHelp | ClapErrorKind::DisplayVersion => {
                    // Help/version text is the payload here. It already
                    // ends with a newline, so trim it before
                    // write_payload adds exactly one. A write failure
                    // is handled like the normal path (InternalError +
                    // its exit code), not swallowed: ENOSPC/EIO must
                    // not exit 0.
                    match write_payload(e.render().to_string().trim_end()) {
                        Ok(()) => return ExitCode::from(0),
                        Err(err) => {
                            // No parsed CLI here, so no selected format:
                            // parse-time errors are always JSON.
                            write_error(&err, Format::Json);
                            return ExitCode::from(err.exit_code() as u8);
                        }
                    }
                }
                _ => {
                    // No `cli` exists yet — the format flag itself may
                    // be the parse failure — so this stays JSON.
                    let err = GeectlError::new(ErrorKind::InvalidInput, e.to_string());
                    write_error(&err, Format::Json);
                    return ExitCode::from(err.exit_code() as u8);
                }
            }
        }
    };
    // The debug channel: one backend install for the process,
    // then the startup and config lines on stderr. Off by default,
    // so without `--debug` / `GEECTL_DEBUG=1` this is a no-op.
    // Debug never fails a run: a config line that cannot be built
    // is skipped, and a stderr write failure is ignored like any
    // other diagnostic write. The backend installs only when the
    // channel is on: installing it unconditionally would raise the
    // global max level for every run, and ureq's own `debug!`
    // records would print on the first real HTTP command.
    let (debug_enabled, debug_bodies) = resolve_debug(
        cli.debug,
        cli.debug_body,
        std::env::var("GEECTL_DEBUG").ok().as_deref(),
    );
    if debug_enabled {
        geectl::debug::install();
    }
    let debug = Debug::new(debug_enabled, debug_bodies);
    if debug.enabled() {
        debug.emit(&startup_line(&argv));
        if let Some(line) = debug_config_line() {
            debug.emit(&line);
        }
    }
    // The format is read before `run` moves `cli`: the error path
    // needs it to render in the selected format.
    let format = cli.format;
    match run(cli, &debug) {
        Ok(payload) => match write_payload(&payload) {
            Ok(()) => ExitCode::from(0),
            Err(err) => {
                write_error(&err, format);
                ExitCode::from(err.exit_code() as u8)
            }
        },
        Err(err) => {
            write_error(&err, format);
            ExitCode::from(err.exit_code() as u8)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_payload_is_json_with_name_and_version() {
        let cli = Cli::parse_from(["geectl", "version"]);
        let payload = run(cli, &Debug::disabled()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(value["name"], "geectl");
        assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn no_subcommand_prints_version_payload() {
        let cli = Cli::parse_from(["geectl"]);
        assert!(run(cli, &Debug::disabled()).is_ok());
    }

    #[test]
    fn text_error_renders_message_context_and_remediation() {
        // Unified rendering (the request group owns write_error):
        // the context names the failure when present, else the kind's
        // message; there is no separate `context:` line.
        let err = GeectlError::new(ErrorKind::InvalidInput, "bad name".to_string());
        let text = render_error_text(&err);
        assert!(text.starts_with("error: bad name"));
        assert!(text.contains("remediation: fix the input and retry"));
    }

    #[test]
    fn text_error_omits_empty_context() {
        let err = GeectlError::new(ErrorKind::NotFound, String::new());
        let text = render_error_text(&err);
        assert!(!text.contains("context:"));
        assert!(text.contains("error: the requested resource does not exist"));
    }

    #[test]
    fn terminated_ensures_a_trailing_newline() {
        assert_eq!(terminated("abc"), "abc\n");
        assert_eq!(terminated("abc\n"), "abc\n");
        // Already-terminated payloads are untouched, even with more
        // than one trailing newline: verbatim wins over tidiness.
        assert_eq!(terminated("abc\n\n"), "abc\n\n");
        assert_eq!(terminated(""), "");
    }

    #[test]
    fn error_text_uses_context_when_present() {
        let with_context = GeectlError::new(ErrorKind::ConfigError, "missing credentials");
        let text = render_error_text(&with_context);
        assert!(text.starts_with("error: missing credentials"), "{text}");
        assert!(text.contains("remediation: "), "{text}");
        let bare = GeectlError::new(ErrorKind::ConfigError, "");
        assert_eq!(
            render_error_text(&bare),
            "error: the configuration is invalid\nremediation: check the configuration (flag, environment variable, or config file) and retry"
        );
    }

    #[test]
    fn build_subcommand_parses() {
        let cli = Cli::parse_from([
            "geectl",
            "build",
            "results",
            "openSUSE:Factory",
            "--code",
            "failed",
        ]);
        assert!(matches!(cli.command, Some(Command::Build { .. })));
    }

    #[test]
    fn error_text_renders_code_message_and_remediation() {
        // Unified rendering (the request group owns write_error):
        // the context names the failure in the `error:` line itself.
        let err = GeectlError::new(
            ErrorKind::ConfigError,
            "build reads need GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD",
        );
        let text = render_error_text(&err);
        assert!(text.starts_with("error: build reads need "));
        assert!(text.contains("GEECTL_OBS_USERNAME"));
        assert!(text.contains("remediation: check the configuration"));
    }

    #[test]
    fn resolve_debug_flag_enables_without_env() {
        assert_eq!(resolve_debug(true, false, None), (true, false));
    }

    #[test]
    fn resolve_debug_env_one_enables_without_flag() {
        assert_eq!(resolve_debug(false, false, Some("1")), (true, false));
    }

    #[test]
    fn resolve_debug_env_other_values_disable_without_flag() {
        assert_eq!(resolve_debug(false, false, None), (false, false));
        assert_eq!(resolve_debug(false, false, Some("")), (false, false));
        assert_eq!(resolve_debug(false, false, Some("0")), (false, false));
        assert_eq!(resolve_debug(false, false, Some("yes")), (false, false));
    }

    #[test]
    fn resolve_debug_flag_wins_over_env() {
        assert_eq!(resolve_debug(true, false, Some("0")), (true, false));
    }

    #[test]
    fn resolve_debug_body_implies_debug() {
        assert_eq!(resolve_debug(false, true, None), (true, true));
        assert_eq!(resolve_debug(false, true, Some("0")), (true, true));
    }

    #[test]
    fn resolve_debug_flag_without_body_prints_no_bodies() {
        assert_eq!(resolve_debug(true, false, Some("1")), (true, false));
    }

    #[test]
    fn startup_line_joins_the_arguments() {
        assert_eq!(
            startup_line(&[
                "geectl".to_string(),
                "--debug".to_string(),
                "version".to_string()
            ]),
            format!("geectl {} --debug version", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn startup_line_without_arguments_has_no_trailing_space() {
        assert_eq!(
            startup_line(&["geectl".to_string()]),
            format!("geectl {}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    #[cfg(unix)]
    fn lossy_argv_replaces_non_utf8_arguments() {
        use std::os::unix::ffi::OsStringExt;
        let argv = lossy_argv(
            vec![
                std::ffi::OsString::from("geectl"),
                std::ffi::OsString::from_vec(vec![0xff, 0xfe]),
            ]
            .into_iter(),
        );
        assert_eq!(argv[0], "geectl");
        assert!(argv[1].contains('\u{FFFD}'), "argv[1]: {:?}", argv[1]);
    }
}
