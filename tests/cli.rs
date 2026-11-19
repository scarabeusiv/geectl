//! Integration tests: the CLI as a binary (help, version, output shape).
//!
//! These run the built binary via `cargo test` (using `CARGO_BIN_EXE_geectl`,
//! set by Cargo for integration tests). They assert the machine contract:
//! `--help` and `--version` exit 0, the version payload is valid JSON on
//! stdout, and errors go to stderr with a structured shape.

use std::process::Command;

fn geectl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_geectl"))
}

#[test]
fn help_exits_zero() {
    let out = geectl().arg("--help").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("geectl"));
}

#[test]
fn version_flag_exits_zero() {
    let out = geectl().arg("--version").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("geectl"));
}

#[test]
fn version_subcommand_prints_json_on_stdout() {
    let out = geectl().arg("version").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(value["name"], "geectl");
    assert!(value["version"].is_string());
    // Nothing but the payload on stdout.
    assert_eq!(stdout.trim().lines().count(), 1);
}

#[test]
fn no_subcommand_prints_version_payload() {
    let out = geectl().output().unwrap();
    assert!(out.status.success());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["name"], "geectl");
}

#[test]
fn text_format_prints_human_readable_version() {
    let out = geectl()
        .args(["--format", "text", "version"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.trim().starts_with("geectl "));
}

#[test]
fn unknown_flag_fails_with_usage_on_stderr() {
    let out = geectl().arg("--no-such-flag").output().unwrap();
    assert!(!out.status.success());
    // clap writes usage to stderr and exits 2.
    assert_eq!(out.status.code(), Some(2));
    assert!(!String::from_utf8_lossy(&out.stderr).is_empty());
}
