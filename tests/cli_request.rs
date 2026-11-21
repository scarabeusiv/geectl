//! Integration tests: the `request` command group as a binary.
//!
//! These run the built binary via `cargo test` (using
//! `CARGO_BIN_EXE_geectl`, set by Cargo for integration tests).
//! Dry-run mutations need no network and no credentials — the
//! library records the plan before any HTTP — so they run fully
//! offline in an isolated environment. Reads refuse without
//! credentials before any request, with a ConfigError (exit 9).

use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Counter for temp dir names: the pid plus a nanosecond timestamp
/// alone collide when two tests run in the same nanosecond, so each
/// call also takes a distinct counter value.
static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temp dir under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "geectl-cli-request-test-{}-{nanos}-{n}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The geectl binary with an isolated environment: HOME and
/// XDG_CONFIG_HOME point at the temp dir (so no real config is read
/// or written), and every GEECTL_* variable is removed (so the
/// developer's own environment cannot leak into the test).
fn geectl_isolated(dir: &TempDir) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_geectl"));
    cmd.env("HOME", &dir.0)
        .env("XDG_CONFIG_HOME", &dir.0)
        .env_remove("GEECTL_OBS_URL")
        .env_remove("GEECTL_OBS_USERNAME")
        .env_remove("GEECTL_OBS_PASSWORD")
        .env_remove("GEECTL_OBS_PASSWORD_FILE")
        .env_remove("GEECTL_GITEA_URL")
        .env_remove("GEECTL_GITEA_TOKEN")
        .env_remove("GEECTL_GITEA_TOKEN_FILE")
        .env_remove("GEECTL_GITEA_SSH_KEY")
        .env_remove("GEECTL_DEBUG");
    cmd
}

/// The dry-run payload of a mutation carries a non-empty `plan`:
/// the recorded lines of what the run would have done.
fn assert_plan_non_empty(value: &serde_json::Value) {
    let plan = value["plan"].as_array().expect("payload carries a plan");
    assert!(!plan.is_empty(), "plan must not be empty: {value}");
}

/// The stderr JSON error of a failed run: code and message.
fn error_of(out: &std::process::Output) -> serde_json::Value {
    serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim())
        .expect("stderr is a JSON error")
}

#[test]
fn request_help_lists_the_subcommands() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    for command in [
        "show",
        "log",
        "search",
        "diff",
        "list-comments",
        "accept",
        "decline",
        "revoke",
        "change-state",
        "try-change-state",
        "add-review",
        "add-comment",
        "file",
        "refile",
        "supersede",
        "file-change-devel",
        "file-delete",
        "file-maintenance-incident",
        "refile-maintenance-incident",
    ] {
        assert!(text.contains(command), "help names {command}");
    }
}

#[test]
fn request_show_without_credentials_fails_with_config_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "show", "42"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    assert_eq!(error_of(&out)["error"]["code"], "config_error");
}

#[test]
fn request_search_without_credentials_fails_with_config_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "search", "--creator", "alice"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    assert_eq!(error_of(&out)["error"]["code"], "config_error");
}

#[test]
fn request_accept_with_global_live_reaches_the_handler() {
    // `--live` is global now: given before the subcommand, it must
    // reach the handler as live, so the run resolves credentials up
    // front and fails with ConfigError when none are configured —
    // instead of printing a dry-run plan.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "request", "accept", "42"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    assert_eq!(error_of(&out)["error"]["code"], "config_error");
}

#[test]
fn request_accept_with_global_live_after_the_leaf_reaches_the_handler() {
    // A global flag is accepted at any level, including after the
    // leaf — the same live run, the same ConfigError.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "accept", "42", "--live"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    assert_eq!(error_of(&out)["error"]["code"], "config_error");
}

#[test]
fn request_accept_dry_run_prints_plan_and_changes_nothing() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "accept", "42", "--comment", "looks good"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["id"], json!(42));
    assert_eq!(value["state"], "accepted");
    assert_eq!(value["dry_run"], true);
    assert_plan_non_empty(&value);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("[dry-run]"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn request_accept_dry_run_text_renders_the_plan() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--format", "text", "request", "accept", "42"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "dry-run: would change request 42 to accepted"
    );
}

#[test]
fn request_change_state_rejects_a_final_state() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "change-state", "42", "superseded"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(error_of(&out)["error"]["code"], "invalid_input");
}

#[test]
fn request_file_dry_run_returns_a_null_id() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "request",
            "file",
            "--source-project",
            "home:scarabeus_iv:branches:devel:languages:python",
            "--source-package",
            "foo",
            "--target-project",
            "devel:languages:python",
            "--target-package",
            "foo",
            "--description",
            "Submit foo",
            "--changes-entry",
            "- Fix CVE-2026-12345.",
            "--cve",
            "CVE-2026-12345",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["id"], serde_json::Value::Null);
    assert_eq!(value["dry_run"], true);
    assert_plan_non_empty(&value);
    assert_eq!(
        value["source_project"],
        "home:scarabeus_iv:branches:devel:languages:python"
    );
    assert_eq!(value["target_project"], "devel:languages:python");
    assert_eq!(value["description"], "Submit foo");
    assert_eq!(value["changes_entry"], "- Fix CVE-2026-12345.");
    assert_eq!(value["cve"], json!(["CVE-2026-12345"]));
    assert_eq!(value["without_cleanup"], false);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("[dry-run]"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn request_file_refuses_a_cve_missing_from_the_changes_entry() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "request",
            "file",
            "--source-project",
            "home:scarabeus_iv:branches:devel:languages:python",
            "--source-package",
            "foo",
            "--target-project",
            "devel:languages:python",
            "--target-package",
            "foo",
            "--description",
            "Submit foo",
            "--changes-entry",
            "- Some other fix.",
            "--cve",
            "CVE-2026-12345",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(error_of(&out)["error"]["code"], "invalid_input");
}

#[test]
fn request_file_delete_dry_run_names_the_target() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "request",
            "file-delete",
            "--target-project",
            "foo",
            "--package",
            "bar",
            "--description",
            "Drop bar",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["target"], "foo/bar");
    assert_eq!(value["description"], "Drop bar");
    assert_eq!(value["dry_run"], true);
    assert_plan_non_empty(&value);
}

#[test]
fn request_file_change_devel_dry_run_names_the_description() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "request",
            "file-change-devel",
            "--source-project",
            "home:foo",
            "--source-package",
            "bar",
            "--target-project",
            "devel:foo",
            "--target-package",
            "bar",
            "--description",
            "Move bar to devel",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["id"], serde_json::Value::Null);
    assert_eq!(value["dry_run"], true);
    assert_plan_non_empty(&value);
    assert_eq!(value["description"], "Move bar to devel");
}

#[test]
fn request_file_delete_refuses_both_targets() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "request",
            "file-delete",
            "--target-project",
            "foo",
            "--package",
            "bar",
            "--repository",
            "baz",
            "--description",
            "Drop bar",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(error_of(&out)["error"]["code"], "invalid_input");
}

#[test]
fn request_add_review_dry_run_names_the_reviewer() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "request",
            "add-review",
            "42",
            "--group",
            "factory-staging",
            "--comment",
            "please review",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["review"]["by_group"], "factory-staging");
    assert_eq!(value["dry_run"], true);
    assert_plan_non_empty(&value);
}

#[test]
fn request_file_maintenance_incident_dry_run_returns_a_null_id() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "request",
            "file-maintenance-incident",
            "--source-project",
            "home:scarabeus_iv:branches:openSUSE:Maintenance",
            "--source-package",
            "foo",
            "--source-package",
            "bar",
            "--target-project",
            "openSUSE:Maintenance",
            "--description",
            "Fix foo and bar",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["id"], serde_json::Value::Null);
    assert_eq!(value["dry_run"], true);
    assert_plan_non_empty(&value);
    assert_eq!(value["source_packages"], json!(["foo", "bar"]));
    assert_eq!(value["target_project"], "openSUSE:Maintenance");
    assert_eq!(value["description"], "Fix foo and bar");
    assert_eq!(value["with_cleanup"], false);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("[dry-run]"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn request_file_maintenance_incident_refuses_no_source_package() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "request",
            "file-maintenance-incident",
            "--source-project",
            "home:scarabeus_iv:branches:openSUSE:Maintenance",
            "--target-project",
            "openSUSE:Maintenance",
            "--description",
            "Fix nothing",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(error_of(&out)["error"]["code"], "invalid_input");
}

#[test]
fn request_refile_maintenance_incident_dry_run_returns_a_null_id() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "request",
            "refile-maintenance-incident",
            "--source-project",
            "home:scarabeus_iv:branches:openSUSE:Maintenance",
            "--source-package",
            "foo",
            "--target-project",
            "openSUSE:Maintenance",
            "--description",
            "Fix foo again",
            "--supersede",
            "42",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["id"], serde_json::Value::Null);
    assert_eq!(value["dry_run"], true);
    assert_plan_non_empty(&value);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("[dry-run]"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn request_supersede_without_credentials_fails_with_config_error() {
    // With --live the no-dry-run guard passes and the library fails
    // closed: a supersede needs OBS Basic credentials. Without
    // --live the command refuses as invalid input instead (see
    // tests/cli.rs).
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "request", "supersede", "42", "43"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    assert_eq!(error_of(&out)["error"]["code"], "config_error");
}

#[test]
fn request_show_without_credentials_text_error_renders_as_text() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--format", "text", "request", "show", "42"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error:"), "stderr: {stderr}");
    assert!(stderr.contains("remediation:"), "stderr: {stderr}");
    assert!(
        !stderr.trim_start().starts_with('{'),
        "stderr is not JSON in text mode: {stderr}"
    );
}

#[test]
fn request_show_with_half_configured_credentials_names_the_misconfiguration() {
    // Guards the premise of the dry-run test below: the username
    // really reaches the binary, so a partial credential set fails
    // resolution with the precise error instead of silently
    // resolving to no credentials.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .env("GEECTL_OBS_USERNAME", "alice")
        .args(["request", "show", "42"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let err = error_of(&out);
    assert!(err["error"]["code"].as_str() == Some("config_error"));
    assert!(
        err["error"]["context"]
            .as_str()
            .unwrap()
            .contains("GEECTL_OBS_USERNAME is set but no password source is"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn request_accept_dry_run_with_half_configured_credentials_prints_plan() {
    // A dry run never touches the server, so the half-configured
    // credential set above must not fail it.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .env("GEECTL_OBS_USERNAME", "alice")
        .args(["request", "accept", "42", "--comment", "looks good"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["dry_run"], true);
    assert_eq!(value["comment"], "looks good");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("[dry-run]"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn request_accept_dry_run_text_echoes_the_comment() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "request",
            "accept",
            "42",
            "--comment",
            "looks good",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "dry-run: would change request 42 to accepted \u{2014} looks good"
    );
}

#[test]
fn request_try_change_state_dry_run_names_target_state() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "request",
            "try-change-state",
            "42",
            "accepted",
            "--comment",
            "looks good",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["target_state"], "accepted");
    assert_eq!(value["changed"], true);
    assert_eq!(value["dry_run"], true);
    assert_plan_non_empty(&value);
    assert_eq!(value["comment"], "looks good");
}

#[test]
fn request_try_change_state_dry_run_text_echoes_the_comment() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "request",
            "try-change-state",
            "42",
            "accepted",
            "--comment",
            "looks good",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "dry-run: would try to change request 42 to accepted \u{2014} looks good"
    );
}

#[test]
fn request_add_comment_dry_run_text_echoes_the_body() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "request",
            "add-comment",
            "42",
            "--body",
            "please rebase",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "dry-run: would comment on request 42: please rebase"
    );
}

#[test]
fn request_search_with_unknown_type_fails_before_credentials() {
    // The search is built and validated before any credential
    // resolution, so an invalid filter fails as InvalidInput even
    // with no credentials configured.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "search", "--type", "bogus"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(error_of(&out)["error"]["code"], "invalid_input");
}

#[test]
fn request_file_change_devel_dry_run_text_names_source_and_target() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "request",
            "file-change-devel",
            "--source-project",
            "home:foo",
            "--source-package",
            "bar",
            "--target-project",
            "devel:foo",
            "--target-package",
            "bar",
            "--description",
            "Move bar to devel",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "dry-run: would file change-devel request for home:foo/bar to devel:foo/bar"
    );
}

#[test]
fn request_file_maintenance_incident_dry_run_text_names_the_release_project() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "request",
            "file-maintenance-incident",
            "--source-project",
            "home:foo",
            "--source-package",
            "bar",
            "--target-project",
            "openSUSE:Maintenance",
            "--release-project",
            "openSUSE:Maintenance:123",
            "--description",
            "Fix bar",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("releasing into openSUSE:Maintenance:123"),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn request_decline_dry_run_carries_a_plan() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "decline", "42", "--comment", "not yet"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["id"], json!(42));
    assert_eq!(value["state"], "declined");
    assert_eq!(value["dry_run"], true);
    assert_plan_non_empty(&value);
}

#[test]
fn request_revoke_dry_run_carries_a_plan() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "revoke", "42"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["id"], json!(42));
    assert_eq!(value["state"], "revoked");
    assert_eq!(value["dry_run"], true);
    assert_plan_non_empty(&value);
}

#[test]
fn request_change_state_dry_run_carries_a_plan() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "change-state", "42", "accepted"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["id"], json!(42));
    assert_eq!(value["state"], "accepted");
    assert_eq!(value["dry_run"], true);
    assert_plan_non_empty(&value);
}

#[test]
fn request_add_comment_dry_run_carries_a_plan() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "add-comment", "42", "--body", "please rebase"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["id"], json!(42));
    assert_eq!(value["dry_run"], true);
    assert_plan_non_empty(&value);
}
