//! geectl — Geeko Controller, a machine-first CLI for agents.
//!
//! Non-interactive is the only mode: no prompts, no editor, no pager.
//! Missing required input fails fast with a structured error naming the
//! missing flag. Stdout carries only the result payload (JSON by default);
//! every diagnostic, warning, and error goes to stderr. There is exactly
//! one exit path: the error kind's exit code.

use clap::{Parser, Subcommand, ValueEnum};
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
}

fn run(cli: Cli) -> Result<String, GeectlError> {
    let payload = match cli.command {
        Some(Command::Version) | None => json!({
            "name": "geectl",
            "version": env!("CARGO_PKG_VERSION"),
        }),
    };
    Ok(match cli.format {
        Format::Json => payload.to_string(),
        Format::Text => format!(
            "geectl {}",
            payload["version"]
                .as_str()
                .unwrap_or(env!("CARGO_PKG_VERSION"))
        ),
    })
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
                    // adds exactly one.
                    let _ = write_payload(e.render().to_string().trim_end());
                    return ExitCode::from(0);
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
