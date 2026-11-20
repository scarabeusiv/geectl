//! geectl — Geeko Controller, a machine-first CLI for agents.
//!
//! Non-interactive is the only mode: no prompts, no editor, no pager.
//! Missing required input fails fast with a structured error naming the
//! missing flag. Stdout carries only the result payload (JSON by default);
//! every diagnostic, warning, and error goes to stderr. There is exactly
//! one exit path: the error kind's exit code.

use clap::{Parser, Subcommand, ValueEnum};
use geectl::auth::{AuthProvider, Credentials, EnvGiteaAuth, EnvObsAuth, GiteaSshKey};
use geectl::config;
use geectl::error::{ErrorKind, GeectlError};
use serde_json::json;
use std::io::Write;
use std::process::ExitCode;

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

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print version information as the result payload.
    Version,
    /// Inspect or initialise the config file.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
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

fn run(cli: Cli) -> Result<String, GeectlError> {
    let (payload, text) = match cli.command {
        Some(Command::Version) | None => {
            let payload = json!({
                "name": "geectl",
                "version": env!("CARGO_PKG_VERSION"),
            });
            let text = format!("geectl {}", env!("CARGO_PKG_VERSION"));
            (payload, text)
        }
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
    };
    Ok(match cli.format {
        Format::Json => payload.to_string(),
        Format::Text => text,
    })
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

/// Write a payload to stdout. A closed pipe (`BrokenPipe`, e.g. the
/// output piped to `head`) is not an error — the consumer just stopped
/// reading. Any other write failure is an internal error. `println!`
/// would panic on either; this is the single stdout writer.
fn write_payload(payload: &str) -> Result<(), GeectlError> {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    match writeln!(handle, "{payload}") {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(GeectlError::new(ErrorKind::InternalError, e.to_string())),
    }
}

/// Write a JSON error payload to stderr. A failure to write to stderr
/// is ignored: there is nowhere else to report it, and panicking while
/// already handling an error helps no one.
fn write_error(err: &GeectlError) {
    let stderr = std::io::stderr();
    let mut handle = stderr.lock();
    let _ = writeln!(handle, "{}", err.to_json());
}

fn main() -> ExitCode {
    // Parse errors go through the single JSON error path, not clap's
    // default (usage on stderr, exit 2, no JSON). Help and version are
    // the exception: they print clap's text to stdout and exit 0.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            use clap::error::ErrorKind as ClapErrorKind;
            match e.kind() {
                ClapErrorKind::DisplayHelp | ClapErrorKind::DisplayVersion => {
                    // Help/version text is the payload here. It already
                    // ends with a newline, so trim it before writeln!
                    // adds exactly one. A write failure is handled like
                    // the normal path (InternalError + its exit code),
                    // not swallowed: ENOSPC/EIO must not exit 0.
                    match write_payload(e.render().to_string().trim_end()) {
                        Ok(()) => return ExitCode::from(0),
                        Err(err) => {
                            write_error(&err);
                            return ExitCode::from(err.exit_code() as u8);
                        }
                    }
                }
                _ => {
                    let err = GeectlError::new(ErrorKind::InvalidInput, e.to_string());
                    write_error(&err);
                    return ExitCode::from(err.exit_code() as u8);
                }
            }
        }
    };
    match run(cli) {
        Ok(payload) => match write_payload(&payload) {
            Ok(()) => ExitCode::from(0),
            Err(err) => {
                write_error(&err);
                ExitCode::from(err.exit_code() as u8)
            }
        },
        Err(err) => {
            write_error(&err);
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
        let payload = run(cli).unwrap();
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(value["name"], "geectl");
        assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn no_subcommand_prints_version_payload() {
        let cli = Cli::parse_from(["geectl"]);
        assert!(run(cli).is_ok());
    }
}
