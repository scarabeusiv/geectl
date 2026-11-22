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
use std::io::{Read, Write};
use std::net::TcpListener;
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

/// One parseable request document for the stub-server test: the
/// library's own fixture shape, so the stub answers what the real
/// server sends.
const REQUEST_STUB_XML: &str = r#"<request creator="creator" id="42">
  <action type="submit">
    <source package="bar" project="foo" rev="1" />
    <target package="bar" project="foobar" />
  </action>
  <action type="delete">
    <target project="deleteme" />
  </action>
  <state name="accepted" when="2010-12-27T01:36:29" who="user1" />
  <history when="2010-12-13T13:02:03" who="creator">
    <description>Create Request</description>
    <comment>foobar</comment>
  </history>
  <title>title of the request</title>
  <description>this is a
very long
description</description>
</request>"#;

/// A one-shot stub OBS server: accepts a single connection, records
/// the request target, and answers with the given XML body.
fn stub_obs_server(body: &str) -> (u16, std::sync::mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind the stub server");
    let port = listener.local_addr().expect("stub address").port();
    let (tx, rx) = std::sync::mpsc::channel();
    let body = body.to_string();
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut request = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    request.extend_from_slice(&buf[..n]);
                    if request.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let text = String::from_utf8_lossy(&request).to_string();
        let target = text
            .lines()
            .next()
            .unwrap_or("")
            .split_whitespace()
            .nth(1)
            .unwrap_or("")
            .to_string();
        let _ = tx.send(target);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    });
    (port, rx)
}

/// Percent-decode a URL target (`%XX` and `+` as space).
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
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
        "list",
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
fn request_list_without_credentials_fails_with_config_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "list"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    assert_eq!(error_of(&out)["error"]["code"], "config_error");
}

#[test]
fn request_list_with_project_without_credentials_fails_with_config_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "list", "--project", "openSUSE:Factory"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    assert_eq!(error_of(&out)["error"]["code"], "config_error");
}

#[test]
fn request_list_with_package_without_project_fails_before_credentials() {
    // `--package` names a package inside a project, so it is
    // refused before any credential resolution.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "list", "--package", "foo"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(error_of(&out)["error"]["code"], "invalid_input");
}

#[test]
fn request_list_with_invalid_since_fails_before_credentials() {
    // The date is validated while the search is built, before any
    // credential resolution, so a bad date fails as InvalidInput
    // even with no credentials configured.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "list", "--since", "garbage"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(error_of(&out)["error"]["code"], "invalid_input");
}

#[test]
fn request_list_with_zero_limit_fails_before_credentials() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "list", "--limit", "0"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(error_of(&out)["error"]["code"], "invalid_input");
}

#[test]
fn request_list_against_a_stub_server_lists_with_the_open_states() {
    // The leaf runs the search end to end: the stub records the
    // match the leaf sent, so the test pins the default states
    // (new, review) in the request itself, not only in the helper.
    // The stub returns its canned collection without filtering, so
    // the payload assertions pin the shape (id, count), while the
    // state assertion reads the sent match.
    let collection = format!("<collection matches=\"1\">\n{REQUEST_STUB_XML}\n</collection>");
    let (port, target_rx) = stub_obs_server(&collection);
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .env("GEECTL_OBS_URL", format!("http://127.0.0.1:{port}"))
        .env("GEECTL_OBS_USERNAME", "tester")
        .env("GEECTL_OBS_PASSWORD", "secret")
        .args(["request", "list"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let payload: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).expect("stdout is JSON");
    assert_eq!(payload["count"], 1);
    assert_eq!(payload["requests"][0]["id"], 42);
    let target = target_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the stub saw the search request");
    let query = percent_decode(&target);
    assert!(
        query.contains("state[@name='new' or @name='review']"),
        "list sends the open states: {query}"
    );
    assert!(
        !query.contains("declined"),
        "list does not send declined: {query}"
    );
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
fn request_file_delete_accept_in_hours_names_the_timer() {
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
            "--accept-in-hours",
            "48",
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
    assert_eq!(value["accept_in_hours"], 48);
    assert_eq!(value["dry_run"], true);
    assert_plan_non_empty(&value);
    let text = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "request",
            "file-delete",
            "--target-project",
            "foo",
            "--package",
            "bar",
            "--description",
            "Drop bar",
            "--accept-in-hours",
            "48",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(
        text.contains("auto-accept in 48h"),
        "text names the timer: {text}"
    );
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
fn request_search_with_invalid_since_fails_before_credentials() {
    // The date is validated while the search is built, before any
    // credential resolution, so a bad date fails as InvalidInput
    // even with no credentials configured.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "search", "--since", "garbage"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(error_of(&out)["error"]["code"], "invalid_input");
}

#[test]
fn request_search_with_zero_limit_fails_before_credentials() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "search", "--limit", "0"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(error_of(&out)["error"]["code"], "invalid_input");
}

#[test]
fn request_search_with_since_and_limit_without_credentials_fails_with_config_error() {
    // Valid filters pass validation, so the run reaches credential
    // resolution and fails there, with a ConfigError (exit 9).
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "search", "--since", "2026-10-01", "--limit", "5"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    assert_eq!(error_of(&out)["error"]["code"], "config_error");
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
