//! geectl — Geeko Controller, a machine-first CLI for agents.
//!
//! Non-interactive is the only mode: no prompts, no editor, no pager.
//! Missing required input fails fast with a structured error naming the
//! missing flag. Stdout carries only the result payload (JSON by default);
//! every diagnostic, warning, and error goes to stderr. There is exactly
//! one exit path: the error kind's exit code.

use clap::{Parser, Subcommand, ValueEnum};
use geectl::error::GeectlError;
use serde_json::json;
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

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(payload) => {
            println!("{payload}");
            ExitCode::from(0)
        }
        Err(err) => {
            eprintln!("{}", err.to_json());
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
