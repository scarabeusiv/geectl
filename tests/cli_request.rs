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
        "change-review-state",
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
fn request_search_with_reviewer_without_credentials_fails_with_config_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "search", "--reviewer", "alice"])
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
        value["plan"].as_array().unwrap().iter().any(|line| line
            .as_str()
            .unwrap_or("")
            .contains("[dry-run] would: forward request 42 after accept")),
        "plan: {}",
        value["plan"]
    );
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
        "dry-run: would change request 42 to accepted; branch cleanup not run; would forward after accept"
    );
}

#[test]
fn request_accept_dry_run_forward_message_names_the_changes_entry() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "accept", "42", "-m", "- custom entry"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert!(
        value["plan"].as_array().unwrap().iter().any(|line| line
            .as_str()
            .unwrap_or("")
            .contains("[dry-run] would: forward request 42 after accept with changes entry: - custom entry")),
        "plan: {}",
        value["plan"]
    );
}

#[test]
fn request_accept_dry_run_text_forward_message_names_the_changes_entry() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "request",
            "accept",
            "42",
            "-m",
            "- custom entry",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "dry-run: would change request 42 to accepted; branch cleanup not run; would forward after accept with changes entry: - custom entry"
    );
}
#[test]
fn request_accept_dry_run_with_no_forward_omits_the_forward() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "request",
            "accept",
            "42",
            "--no-forward",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "dry-run: would change request 42 to accepted; branch cleanup not run"
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

/// A routing stub HTTP server: answers each request with the body
/// of the first route whose needle the request target contains,
/// recording every request target. Unmatched requests get a 404, so
/// an unexpected request fails the test loudly instead of passing
/// silently. The thread is not joined: if the binary never connects,
/// the test fails on its own assertion and the thread dies with the
/// test process.
fn serve_routes(
    routes: Vec<(String, String)>,
) -> (
    std::net::SocketAddr,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_in_thread = std::sync::Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let request = read_request(&mut stream);
            let text = String::from_utf8_lossy(&request).into_owned();
            let target = text
                .lines()
                .next()
                .unwrap_or("")
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .to_string();
            seen_in_thread.lock().unwrap().push(target.clone());
            let body = routes
                .iter()
                .find(|(needle, _)| target.contains(needle.as_str()))
                .map(|(_, body)| body.clone());
            let (status, body) = match body {
                Some(body) => ("200 OK", body),
                None => ("404 Not Found", "<status code=\"not_found\" />".to_string()),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Write);
        }
    });
    (addr, seen)
}

/// The `science/gsl` obsinfo body, shaped like the live file.
fn gsl_obsinfo(commit: &str) -> String {
    format!(
        "mtime: 1774534342\ncommit: {commit}\nurl: https://src.opensuse.org/adamm/gsl.git\nrevision: factory\n"
    )
}

/// The `science/gsl` obsinfo commit observed live at implementation
/// time; the branch-head answer for the current-source test.
const CLI_GSL_HEAD: &str = "2ec4b574be01c905f83a91b026682b839cfce0521a5b24d164c818d54d0880ec";
/// A different commit: the branch-head answer for the stale test.
const CLI_OTHER_HEAD: &str = "abcdef1234567890abcdef1234567890abcdef12";

/// The `request file --live` arguments for the git-backed
/// `science/gsl` source.
fn gsl_file_args() -> Vec<&'static str> {
    vec![
        "--live",
        "request",
        "file",
        "--source-project",
        "science",
        "--source-package",
        "gsl",
        "--target-project",
        "devel:languages:python",
        "--target-package",
        "gsl",
        "--description",
        "Update gsl to 2.0",
        "--changes-entry",
        "- Update to version 2.0.",
    ]
}

/// A live `request file` from a stale git-backed source refuses
/// before filing: the OBS side lags the git head, so no submit
/// request is created. The Gitea stub answers the branch head; the
/// OBS stub answers the source meta and the obsinfo.
#[test]
fn request_file_live_stale_git_backed_source_refuses() {
    let (gitea_addr, gitea_seen) = serve_routes(vec![(
        "/api/v1/repos/adamm/gsl/branches/factory".to_string(),
        format!(
            r#"{{"name": "factory", "commit": {{"id": "{CLI_OTHER_HEAD}", "message": "OBS bridge sync"}}, "protected": false}}"#
        ),
    )]);
    let (obs_addr, obs_seen) = serve_routes(vec![
        (
            "/source/science/gsl/_meta".to_string(),
            format!(
                "<package name=\"gsl\" project=\"science\"><scmsync>http://{gitea_addr}/adamm/gsl.git#factory</scmsync></package>"
            ),
        ),
        (
            "/source/science/gsl/_scmsync.obsinfo".to_string(),
            gsl_obsinfo(CLI_GSL_HEAD),
        ),
        (
            "/search/request".to_string(),
            "<collection matches=\"0\"></collection>".to_string(),
        ),
        (
            "/request?cmd=create".to_string(),
            "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>".to_string(),
        ),
    ]);
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(gsl_file_args())
        .env("GEECTL_OBS_URL", format!("http://{obs_addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .env("GEECTL_GITEA_URL", format!("http://{gitea_addr}"))
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = error_of(&out);
    assert_eq!(err["error"]["code"], "invalid_input");
    assert!(
        err["error"]["context"]
            .as_str()
            .unwrap()
            .contains("refusing to submit science/gsl"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        err["error"]["context"]
            .as_str()
            .unwrap()
            .contains("has not synced the git head yet"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The branch head was read, but nothing was filed: no create
    // POST reached the OBS stub.
    assert!(
        gitea_seen
            .lock()
            .unwrap()
            .iter()
            .any(|t| t.contains("/api/v1/repos/adamm/gsl/branches/factory")),
        "gitea requests: {:?}",
        gitea_seen.lock().unwrap()
    );
    assert!(
        !obs_seen
            .lock()
            .unwrap()
            .iter()
            .any(|t| t.contains("cmd=create")),
        "obs requests: {:?}",
        obs_seen.lock().unwrap()
    );
}

/// A live `request file` from a current git-backed source files:
/// the OBS side carries the git head, so the submit request is
/// created.
#[test]
fn request_file_live_current_git_backed_source_files() {
    let (gitea_addr, _) = serve_routes(vec![(
        "/api/v1/repos/adamm/gsl/branches/factory".to_string(),
        format!(
            r#"{{"name": "factory", "commit": {{"id": "{CLI_GSL_HEAD}", "message": "OBS bridge sync"}}, "protected": false}}"#
        ),
    )]);
    let (obs_addr, obs_seen) = serve_routes(vec![
        (
            "/source/science/gsl/_meta".to_string(),
            format!(
                "<package name=\"gsl\" project=\"science\"><scmsync>http://{gitea_addr}/adamm/gsl.git#factory</scmsync></package>"
            ),
        ),
        (
            "/source/science/gsl/_scmsync.obsinfo".to_string(),
            gsl_obsinfo(CLI_GSL_HEAD),
        ),
        (
            "/search/request".to_string(),
            "<collection matches=\"0\"></collection>".to_string(),
        ),
        (
            "/request?cmd=create".to_string(),
            "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>".to_string(),
        ),
    ]);
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(gsl_file_args())
        .env("GEECTL_OBS_URL", format!("http://{obs_addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .env("GEECTL_GITEA_URL", format!("http://{gitea_addr}"))
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["id"], json!(1400001));
    assert_eq!(value["dry_run"], false);
    assert!(
        obs_seen
            .lock()
            .unwrap()
            .iter()
            .any(|t| t.contains("cmd=create")),
        "obs requests: {:?}",
        obs_seen.lock().unwrap()
    );
}

/// A live `request file` from a git-backed source files with no
/// Gitea token configured: the freshness check resolves its client
/// lazily, after the source meta says `<scmsync>` — and with no
/// token the check is off, so the filing proceeds exactly as
/// before, the Gitea stub sees no request, and the debug channel
/// records the skip.
#[test]
fn request_file_live_without_gitea_token_files_with_skip_recorded() {
    let (gitea_addr, gitea_seen) = serve_routes(vec![(
        "/api/v1/repos/adamm/gsl/branches/factory".to_string(),
        format!(
            r#"{{"name": "factory", "commit": {{"id": "{CLI_GSL_HEAD}", "message": "OBS bridge sync"}}, "protected": false}}"#
        ),
    )]);
    let (obs_addr, obs_seen) = serve_routes(vec![
        (
            "/source/science/gsl/_meta".to_string(),
            format!(
                "<package name=\"gsl\" project=\"science\"><scmsync>http://{gitea_addr}/adamm/gsl.git#factory</scmsync></package>"
            ),
        ),
        (
            "/source/science/gsl/_scmsync.obsinfo".to_string(),
            gsl_obsinfo(CLI_GSL_HEAD),
        ),
        (
            "/search/request".to_string(),
            "<collection matches=\"0\"></collection>".to_string(),
        ),
        (
            "/request?cmd=create".to_string(),
            "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>".to_string(),
        ),
    ]);
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--debug"])
        .args(gsl_file_args())
        .env("GEECTL_OBS_URL", format!("http://{obs_addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .env("GEECTL_GITEA_URL", format!("http://{gitea_addr}"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["id"], json!(1400001));
    assert_eq!(value["dry_run"], false);
    // The Gitea stub saw nothing: the client never resolved, so no
    // branch head was read.
    assert!(
        gitea_seen.lock().unwrap().is_empty(),
        "gitea requests: {:?}",
        gitea_seen.lock().unwrap()
    );
    // The create POST did reach the OBS stub.
    assert!(
        obs_seen
            .lock()
            .unwrap()
            .iter()
            .any(|t| t.contains("cmd=create")),
        "obs requests: {:?}",
        obs_seen.lock().unwrap()
    );
    // The skip is recorded on the debug channel.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(
            "decision: no Gitea token configured; the filing-path freshness check is off"
        ),
        "{stderr}"
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
fn request_file_delete_dry_run_text_names_the_target() {
    // The dry-run text names the target, like the JSON payload's
    // `target` field; the description stays JSON-only.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "request",
            "file-delete",
            "--target-project",
            "test:delete-me",
            "--whole-project",
            "--description",
            "removing",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "dry-run: would file delete request for test:delete-me"
    );
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
fn request_change_review_state_dry_run_names_the_reviewer_and_state() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "request",
            "change-review-state",
            "42",
            "declined",
            "--group",
            "factory-staging",
            "--comment",
            "not this way",
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
    assert_eq!(value["target_state"], "declined");
    assert_eq!(value["review"]["by_group"], "factory-staging");
    assert_eq!(value["comment"], "not this way");
    assert_eq!(value["dry_run"], true);
    assert_plan_non_empty(&value);
}

#[test]
fn request_change_review_state_dry_run_text_renders_the_plan() {
    // The agreement test: every identifying field the JSON payload
    // carries appears in the text rendering.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "request",
            "change-review-state",
            "42",
            "declined",
            "--group",
            "factory-staging",
            "--comment",
            "not this way",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "dry-run: would change the review by group factory-staging on request 42 to declined — not this way"
    );
}

#[test]
fn request_change_review_state_rejects_final_states() {
    for state in ["superseded", "obsoleted"] {
        let dir = TempDir::new();
        let out = geectl_isolated(&dir)
            .args([
                "request",
                "change-review-state",
                "42",
                state,
                "--user",
                "Admin",
            ])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert_eq!(error_of(&out)["error"]["code"], "invalid_input");
    }
}

#[test]
fn request_change_review_state_without_a_target_fails() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "change-review-state", "42", "accepted"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(error_of(&out)["error"]["code"], "invalid_input");
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

/// Read one request (headers + body) from a loopback connection:
/// the body length comes from Content-Length (a request without one
/// has no body). A single read can return only the headers when the
/// body arrives in a later segment, and answering before the body is
/// read resets the client's write on Windows, which the client reads
/// as a transport failure.
fn read_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
    use std::io::Read;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header_end = end + 4;
                    let header_text = String::from_utf8_lossy(&buf[..end]);
                    let content_length: usize = header_text
                        .lines()
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            if k.trim().eq_ignore_ascii_case("content-length") {
                                v.trim().parse().ok()
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    if buf.len() >= header_end + content_length {
                        break;
                    }
                }
            }
            Err(_) => break,
        }
    }
    buf
}

/// A many-shot HTTP fixture answering every request with a status-ok
/// body: the supersede POST needs one canned answer, and the test
/// runs the command twice (JSON and text) against the same server.
/// The thread is not joined: if the binary never connects, the test
/// fails on its own assertion and the thread dies with the test
/// process.
fn serve_supersede_ok() -> (
    std::net::SocketAddr,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let body = "<status code=\"ok\" />";
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_in_thread = std::sync::Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let request = read_request(&mut stream);
            seen_in_thread
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&request).into_owned());
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Write);
        }
    });
    (addr, seen)
}

#[test]
fn request_supersede_live_shape_carries_ids_and_dry_run_false() {
    // With --live the command performs the supersede and reports the
    // pair with dry_run false, in both formats.
    let (addr, seen) = serve_supersede_ok();
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "request", "supersede", "42", "43"])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["old_id"], 42);
    assert_eq!(value["new_id"], 43);
    assert_eq!(value["dry_run"], false);
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "--live",
            "request",
            "supersede",
            "42",
            "43",
        ])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "superseded request 42 with 43"
    );
    let seen = seen.lock().unwrap();
    assert!(
        seen.iter()
            .any(|r| r.contains("request/42") && r.contains("superseded_by=43")),
        "recorded requests: {seen:?}"
    );
}

/// A routing stub OBS server for the live accept-then-forward: the
/// changestate POST, the request read, the target package's
/// directory listing, the maintenance search, the filing dedup
/// search, and the create POST each get their canned answer, by
/// method and request target. Anything else is a 404. The seen
/// request lines are recorded so the test can prove the accept
/// landed before the forward was filed.
/// What a real OBS returns for a project-meta read: a `<project>`
/// document. The stubs used to answer package meta for every `/_meta`
/// URL, which made the accept-time cleanup read a package document
/// as a project and report a failure; the project carries no branch
/// marker, so the cleanup skips it.
const PROJECT_META_XML: &str = "<project name=\"home:scarabeus_iv\"><description>Test project for the master branch of home:scarabeus_iv/foo</description></project>";

/// A project-meta URL carries one path segment between `/source/`
/// and `/_meta`; a package-meta URL carries two.
fn is_project_meta_url(target: &str) -> bool {
    target
        .strip_prefix("/source/")
        .and_then(|rest| rest.strip_suffix("/_meta"))
        .is_some_and(|middle| !middle.contains('/'))
}

fn serve_accept_forward_live() -> (
    std::net::SocketAddr,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_in_thread = std::sync::Arc::clone(&seen);
    let request_xml = "<request id=\"42\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"accepted\" who=\"alice\" when=\"2026-11-24T00:00:00Z\" /><description>Update foo to 2.0</description></request>";
    let directory_xml =
        "<directory><linkinfo project=\"openSUSE:Factory\" package=\"foo\" /></directory>";
    let meta_xml = "<package name=\"foo\" project=\"openSUSE:Factory\"><devel project=\"devel:languages:python\" package=\"foo\"/></package>";
    let empty_collection = "<collection matches=\"0\"></collection>";
    let ok = "<status code=\"ok\" />";
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let request = read_request(&mut stream);
            let text = String::from_utf8_lossy(&request).into_owned();
            seen_in_thread
                .lock()
                .unwrap()
                .push(text.lines().next().unwrap_or("").to_string());
            let mut parts = text.lines().next().unwrap_or("").split_whitespace();
            let method = parts.next().unwrap_or("");
            let target = parts.next().unwrap_or("");
            let (status, body) = if method == "POST" && target.contains("changestate") {
                (200, ok.to_string())
            } else if method == "POST" && target.contains("cmd=create") {
                (
                    200,
                    "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>".to_string(),
                )
            } else if is_project_meta_url(target) {
                (200, PROJECT_META_XML.to_string())
            } else if target.contains("/_meta") {
                (200, meta_xml.to_string())
            } else if target.contains("search/project/id") || target.contains("search/request") {
                (200, empty_collection.to_string())
            } else if target.starts_with("/source/") {
                (200, directory_xml.to_string())
            } else if target.starts_with("/request/") {
                (200, request_xml.to_string())
            } else {
                (404, "<status code=\"not_found\" />".to_string())
            };
            let response = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Write);
        }
    });
    (addr, seen)
}

#[test]
fn request_accept_live_forwards_after_accept() {
    let (addr, seen) = serve_accept_forward_live();
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "request", "accept", "42"])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["state"], "accepted");
    assert_eq!(value["dry_run"], false);
    assert_eq!(value["forward"]["actions"][0]["outcome"], "forwarded");
    assert_eq!(
        value["forward"]["actions"][0]["target"],
        "openSUSE:Factory/foo"
    );
    assert_eq!(value["forward"]["actions"][0]["new_id"], 1400001);
    // The guard must drop before the second run: the stub thread
    // takes the same lock per request, and holding it across the
    // run would stall every response.
    {
        let seen = seen.lock().unwrap();
        let change = seen
            .iter()
            .position(|r| r.contains("changestate") && r.contains("newstate=accepted"))
            .expect("no changestate POST recorded");
        let create = seen
            .iter()
            .position(|r| r.contains("cmd=create"))
            .expect("no create POST recorded");
        assert!(
            change < create,
            "the forward was filed before the accept landed: {seen:?}"
        );
    }
    let out = geectl_isolated(&dir)
        .args(["--live", "--format", "text", "request", "accept", "42"])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "changed request 42 to accepted; forwarded request 42\nforwarded devel:languages:python/foo to openSUSE:Factory/foo as request 1400001"
    );
}

/// A routing stub OBS server for a live accept whose forward is
/// refused: the link parent's meta records a different devel
/// package (a rename). The request read answers the post-accept
/// state, like the live server would after the accept lands. The
/// seen request lines are recorded so the test can prove the
/// accept landed and nothing was declined or filed.
fn serve_accept_forward_refused() -> (
    std::net::SocketAddr,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_in_thread = std::sync::Arc::clone(&seen);
    let request_xml = "<request id=\"42\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"accepted\" who=\"alice\" when=\"2026-11-24T00:00:00Z\" /><description>Update foo to 2.0</description></request>";
    let directory_xml =
        "<directory><linkinfo project=\"openSUSE:Factory\" package=\"foo\" /></directory>";
    let meta_xml = "<package name=\"foo\" project=\"openSUSE:Factory\"><devel project=\"devel:languages:python\" package=\"bar\"/></package>";
    let empty_collection = "<collection matches=\"0\"></collection>";
    let ok = "<status code=\"ok\" />";
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let request = read_request(&mut stream);
            let text = String::from_utf8_lossy(&request).into_owned();
            seen_in_thread
                .lock()
                .unwrap()
                .push(text.lines().next().unwrap_or("").to_string());
            let mut parts = text.lines().next().unwrap_or("").split_whitespace();
            let method = parts.next().unwrap_or("");
            let target = parts.next().unwrap_or("");
            let (status, body) = if method == "POST" && target.contains("changestate") {
                (200, ok.to_string())
            } else if method == "POST" && target.contains("cmd=create") {
                (
                    200,
                    "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>".to_string(),
                )
            } else if is_project_meta_url(target) {
                (200, PROJECT_META_XML.to_string())
            } else if target.contains("/_meta") {
                (200, meta_xml.to_string())
            } else if target.contains("search/project/id") || target.contains("search/request") {
                (200, empty_collection.to_string())
            } else if target.starts_with("/source/") {
                (200, directory_xml.to_string())
            } else if target.starts_with("/request/") {
                (200, request_xml.to_string())
            } else {
                (404, "<status code=\"not_found\" />".to_string())
            };
            let response = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Write);
        }
    });
    (addr, seen)
}
/// A routing stub OBS server for a live accept whose forward
/// fails transiently: the link parent's directory listing answers
/// 500, so the forward fails after the accept lands. The request
/// read answers the post-accept state. The seen request lines are
/// recorded so the test can prove the accept landed and nothing
/// was declined or filed.
fn serve_accept_forward_transient_error() -> (
    std::net::SocketAddr,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_in_thread = std::sync::Arc::clone(&seen);
    let request_xml = "<request id=\"42\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"accepted\" who=\"alice\" when=\"2026-11-24T00:00:00Z\" /><description>Update foo to 2.0</description></request>";
    let meta_xml = "<package name=\"foo\" project=\"openSUSE:Factory\"><devel project=\"devel:languages:python\" package=\"bar\"/></package>";
    let empty_collection = "<collection matches=\"0\"></collection>";
    let ok = "<status code=\"ok\" />";
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let request = read_request(&mut stream);
            let text = String::from_utf8_lossy(&request).into_owned();
            seen_in_thread
                .lock()
                .unwrap()
                .push(text.lines().next().unwrap_or("").to_string());
            let mut parts = text.lines().next().unwrap_or("").split_whitespace();
            let method = parts.next().unwrap_or("");
            let target = parts.next().unwrap_or("");
            let (status, body) = if method == "POST" && target.contains("changestate") {
                (200, ok.to_string())
            } else if method == "POST" && target.contains("cmd=create") {
                (
                    200,
                    "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>".to_string(),
                )
            } else if is_project_meta_url(target) {
                (200, PROJECT_META_XML.to_string())
            } else if target.contains("/_meta") {
                (200, meta_xml.to_string())
            } else if target.contains("search/project/id") || target.contains("search/request") {
                (200, empty_collection.to_string())
            } else if target.starts_with("/source/") {
                (500, "<status code=\"internal_error\" />".to_string())
            } else if target.starts_with("/request/") {
                (200, request_xml.to_string())
            } else {
                (404, "<status code=\"not_found\" />".to_string())
            };
            let response = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Write);
        }
    });
    (addr, seen)
}

/// A routing stub OBS server for a live accept whose forward
/// fails transiently at the devel record: the hop resolves from the
/// link parent, but the link parent's meta read answers 500, so the
/// forward fails after the accept lands. The same shape as
/// `serve_accept_forward_refused`, with a `ServerError` check
/// failure instead of a refusal. The request read answers the
/// post-accept state. The seen request lines are recorded so the
/// test can prove the accept landed and nothing was declined or
/// filed.
fn serve_accept_forward_transient_check_error() -> (
    std::net::SocketAddr,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_in_thread = std::sync::Arc::clone(&seen);
    let request_xml = "<request id=\"42\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"accepted\" who=\"alice\" when=\"2026-11-24T00:00:00Z\" /><description>Update foo to 2.0</description></request>";
    let directory_xml =
        "<directory><linkinfo project=\"openSUSE:Factory\" package=\"foo\" /></directory>";
    let empty_collection = "<collection matches=\"0\"></collection>";
    let ok = "<status code=\"ok\" />";
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let request = read_request(&mut stream);
            let text = String::from_utf8_lossy(&request).into_owned();
            seen_in_thread
                .lock()
                .unwrap()
                .push(text.lines().next().unwrap_or("").to_string());
            let mut parts = text.lines().next().unwrap_or("").split_whitespace();
            let method = parts.next().unwrap_or("");
            let target = parts.next().unwrap_or("");
            let (status, body) = if method == "POST" && target.contains("changestate") {
                (200, ok.to_string())
            } else if method == "POST" && target.contains("cmd=create") {
                (
                    200,
                    "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>".to_string(),
                )
            } else if is_project_meta_url(target) {
                (200, PROJECT_META_XML.to_string())
            } else if target.contains("/_meta") {
                (500, "<status code=\"internal_error\" />".to_string())
            } else if target.contains("search/project/id") || target.contains("search/request") {
                (200, empty_collection.to_string())
            } else if target.starts_with("/source/") {
                (200, directory_xml.to_string())
            } else if target.starts_with("/request/") {
                (200, request_xml.to_string())
            } else {
                (404, "<status code=\"not_found\" />".to_string())
            };
            let response = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Write);
        }
    });
    (addr, seen)
}

#[test]
fn request_accept_live_accepts_when_the_forward_is_refused() {
    // The refused fixture: the accept lands, and the forward then
    // refuses on the renamed devel — the refusal is reported, the
    // accepted request stands, and nothing is declined or filed.
    let (addr, seen) = serve_accept_forward_refused();
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "request", "accept", "42"])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "the refused accept exited 0: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        stderr.contains("accepted request 42, but the forward failed"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("devel:languages:python/bar"),
        "stderr: {stderr}"
    );
    {
        let seen = seen.lock().unwrap();
        assert!(
            seen.iter()
                .any(|r| r.contains("changestate") && r.contains("newstate=accepted")),
            "no accept changestate recorded: {seen:?}"
        );
        assert!(
            !seen.iter().any(|r| r.contains("newstate=declined")),
            "the refusal declined the accepted request: {seen:?}"
        );
        assert!(
            !seen.iter().any(|r| r.contains("cmd=create")),
            "a forward was filed after the refusal: {seen:?}"
        );
    }
}

#[test]
fn request_accept_live_transient_check_failure_accepts_and_reports() {
    // The same refused fixture shape, but the hop read fails
    // transiently (500) instead of refusing: the accept lands,
    // the forward fails with the server's error, and the request
    // stays accepted with the work queued — nothing is declined
    // or filed.
    let (addr, seen) = serve_accept_forward_transient_error();
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "request", "accept", "42"])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "the transient accept exited 0: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(out.status.code(), Some(6));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        stderr.contains("accepted request 42, but the forward failed"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("returned 500"), "stderr: {stderr}");
    {
        let seen = seen.lock().unwrap();
        assert!(
            seen.iter()
                .any(|r| r.contains("changestate") && r.contains("newstate=accepted")),
            "no accept changestate recorded: {seen:?}"
        );
        assert!(
            !seen.iter().any(|r| r.contains("newstate=declined")),
            "the transient failure declined the accepted request: {seen:?}"
        );
        assert!(
            !seen.iter().any(|r| r.contains("cmd=create")),
            "a forward was filed after the transient failure: {seen:?}"
        );
    }
}

#[test]
fn request_accept_live_transient_devel_check_failure_accepts_and_reports() {
    // The refused fixture shape, but the devel-record read fails
    // transiently (500) instead of returning a mismatched devel:
    // the hop resolves, the forward fails with a ServerError
    // after the accept lands, and the request stays accepted with
    // the work queued — nothing is declined or filed.
    let (addr, seen) = serve_accept_forward_transient_check_error();
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "request", "accept", "42"])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "the transient accept exited 0: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(out.status.code(), Some(6));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        stderr.contains("accepted request 42, but the forward failed"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("returned 500"), "stderr: {stderr}");
    {
        let seen = seen.lock().unwrap();
        assert!(
            seen.iter()
                .any(|r| r.contains("changestate") && r.contains("newstate=accepted")),
            "no accept changestate recorded: {seen:?}"
        );
        assert!(
            !seen.iter().any(|r| r.contains("newstate=declined")),
            "the transient failure declined the accepted request: {seen:?}"
        );
        assert!(
            !seen.iter().any(|r| r.contains("cmd=create")),
            "a forward was filed after the transient check failure: {seen:?}"
        );
    }
}

#[test]
fn request_accept_live_no_forward_skips_the_gate() {
    // The same refused fixture: with --no-forward the accept
    // runs without the devel check, so the request is accepted
    // and nothing is declined or filed.
    let (addr, seen) = serve_accept_forward_refused();
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "request", "accept", "42", "--no-forward"])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["state"], "accepted");
    assert_eq!(value["dry_run"], false);
    {
        let seen = seen.lock().unwrap();
        assert!(
            seen.iter()
                .any(|r| r.contains("changestate") && r.contains("newstate=accepted")),
            "no accept changestate recorded: {seen:?}"
        );
        assert!(
            !seen.iter().any(|r| r.contains("newstate=declined")),
            "the gate declined a --no-forward accept: {seen:?}"
        );
        assert!(
            !seen.iter().any(|r| r.contains("cmd=create")),
            "a forward was filed for a --no-forward accept: {seen:?}"
        );
    }
}

#[test]
fn pull_request_merge_obs_live_forwards_after_merge() {
    // The OBS arm of `pull-request merge` merges with the
    // accept-coupled forward: the same routing stub serves it, and
    // the merge must land before the forward is filed.
    let (addr, seen) = serve_accept_forward_live();
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "pull-request", "merge", "obs:42"])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["reference"], "obs:42");
    assert_eq!(value["status"], "merged");
    assert_eq!(value["dry_run"], false);
    assert_eq!(value["forward"]["actions"][0]["outcome"], "forwarded");
    assert_eq!(
        value["forward"]["actions"][0]["target"],
        "openSUSE:Factory/foo"
    );
    assert_eq!(value["forward"]["actions"][0]["new_id"], 1400001);
    // The guard must drop before the second run: the stub thread
    // takes the same lock per request, and holding it across the
    // run would stall every response.
    {
        let seen = seen.lock().unwrap();
        let change = seen
            .iter()
            .position(|r| r.contains("changestate") && r.contains("newstate=accepted"))
            .expect("no changestate POST recorded");
        let create = seen
            .iter()
            .position(|r| r.contains("cmd=create"))
            .expect("no create POST recorded");
        assert!(
            change < create,
            "the forward was filed before the merge landed: {seen:?}"
        );
    }
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "--format",
            "text",
            "pull-request",
            "merge",
            "obs:42",
        ])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "merged obs:42 (now merged); forwarded request 42\nforwarded devel:languages:python/foo to openSUSE:Factory/foo as request 1400001"
    );
}

#[test]
fn pull_request_merge_obs_live_merges_when_the_forward_is_refused() {
    // The same rename fixture: the merge lands, and the forward
    // then refuses — the refusal is reported, the merged request
    // stands, and nothing is declined or filed.
    let (addr, seen) = serve_accept_forward_refused();
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "pull-request", "merge", "obs:42"])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "the refused merge exited 0: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        stderr.contains("merged request 42, but the forward failed"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("devel:languages:python/bar"),
        "stderr: {stderr}"
    );
    {
        let seen = seen.lock().unwrap();
        assert!(
            seen.iter()
                .any(|r| r.contains("changestate") && r.contains("newstate=accepted")),
            "no accept changestate recorded: {seen:?}"
        );
        assert!(
            !seen.iter().any(|r| r.contains("newstate=declined")),
            "the refusal declined the merged request: {seen:?}"
        );
        assert!(
            !seen.iter().any(|r| r.contains("cmd=create")),
            "a forward was filed after the refusal: {seen:?}"
        );
    }
}

/// A routing stub OBS server for a live merge whose target package
/// has no link parent: the merge lands and the forward reports
/// the hop skipped, filing nothing.
fn serve_merge_no_link_parent() -> (
    std::net::SocketAddr,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_in_thread = std::sync::Arc::clone(&seen);
    let request_xml = "<request id=\"42\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"accepted\" who=\"alice\" when=\"2026-11-24T00:00:00Z\" /><description>Update foo to 2.0</description></request>";
    let directory_xml = "<directory></directory>";
    let meta_xml = "<package name=\"foo\" project=\"openSUSE:Factory\"><devel project=\"devel:languages:python\" package=\"foo\"/></package>";
    let empty_collection = "<collection matches=\"0\"></collection>";
    let ok = "<status code=\"ok\" />";
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let request = read_request(&mut stream);
            let text = String::from_utf8_lossy(&request).into_owned();
            seen_in_thread
                .lock()
                .unwrap()
                .push(text.lines().next().unwrap_or("").to_string());
            let mut parts = text.lines().next().unwrap_or("").split_whitespace();
            let method = parts.next().unwrap_or("");
            let target = parts.next().unwrap_or("");
            let (status, body) = if method == "POST" && target.contains("changestate") {
                (200, ok.to_string())
            } else if method == "POST" && target.contains("cmd=create") {
                (
                    200,
                    "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>".to_string(),
                )
            } else if is_project_meta_url(target) {
                (200, PROJECT_META_XML.to_string())
            } else if target.contains("/_meta") {
                (200, meta_xml.to_string())
            } else if target.contains("search/project/id") || target.contains("search/request") {
                (200, empty_collection.to_string())
            } else if target.starts_with("/source/") {
                (200, directory_xml.to_string())
            } else if target.starts_with("/request/") {
                (200, request_xml.to_string())
            } else {
                (404, "<status code=\"not_found\" />".to_string())
            };
            let response = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Write);
        }
    });
    (addr, seen)
}

#[test]
fn pull_request_merge_obs_live_no_link_parent_forwards_nothing() {
    // No link parent means no hop: the merge lands, the forward
    // reports the action skipped, and nothing is filed.
    let (addr, seen) = serve_merge_no_link_parent();
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "pull-request", "merge", "obs:42"])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["reference"], "obs:42");
    assert_eq!(value["status"], "merged");
    assert_eq!(value["dry_run"], false);
    assert_eq!(value["forward"]["actions"][0]["outcome"], "skipped");
    assert_eq!(
        value["forward"]["actions"][0]["reason"],
        "the package has no link parent"
    );
    {
        let seen = seen.lock().unwrap();
        assert!(
            seen.iter()
                .any(|r| r.contains("changestate") && r.contains("newstate=accepted")),
            "no merge changestate recorded: {seen:?}"
        );
        assert!(
            !seen.iter().any(|r| r.contains("cmd=create")),
            "a forward was filed with no link parent: {seen:?}"
        );
    }
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "--format",
            "text",
            "pull-request",
            "merge",
            "obs:42",
        ])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "merged obs:42 (now merged); request 42: nothing forwarded\nskipped devel:languages:python/foo: the package has no link parent"
    );
}

#[test]
fn pull_request_merge_obs_dry_run_plans_the_forward() {
    // No server: the dry run plans the merge and the forward
    // after it, and changes nothing.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["pull-request", "merge", "obs:42"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["reference"], "obs:42");
    assert!(value["status"].is_null());
    assert_eq!(value["dry_run"], true);
    assert_plan_non_empty(&value);
    assert!(
        value["plan"].as_array().unwrap().iter().any(|line| line
            .as_str()
            .unwrap_or("")
            .contains("[dry-run] would: forward request 42 after merge")),
        "plan: {}",
        value["plan"]
    );
    let out = geectl_isolated(&dir)
        .args(["--format", "text", "pull-request", "merge", "obs:42"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "dry-run: would merge obs:42; branch cleanup not run; would forward after merge"
    );
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
        "dry-run: would change request 42 to accepted \u{2014} looks good; branch cleanup not run; would forward after accept"
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
fn request_search_with_quoted_reviewer_fails_before_credentials() {
    // As for an unknown type: the reviewer value is validated when
    // the search is built, before any credential resolution.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "search", "--reviewer", "o'brien"])
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
fn request_file_add_role_dry_run_names_the_grantee() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "request",
            "file-add-role",
            "--target-project",
            "foo",
            "--target-package",
            "bar",
            "--person",
            "alice",
            "--person-role",
            "maintainer",
            "--description",
            "Grant alice the maintainer role on bar",
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
    assert_eq!(value["target_project"], "foo");
    assert_eq!(value["target_package"], "bar");
    assert_eq!(
        value["person"],
        json!({"name": "alice", "role": "maintainer"})
    );
    assert_eq!(value["group"], serde_json::Value::Null);
    assert_eq!(
        value["description"],
        "Grant alice the maintainer role on bar"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("[dry-run]"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn request_file_add_role_group_only_dry_run_names_the_group() {
    // Project-level grant: no package, group only.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "request",
            "file-add-role",
            "--target-project",
            "foo",
            "--group",
            "factory-maintainers",
            "--group-role",
            "reviewer",
            "--description",
            "Grant the group the reviewer role on foo",
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
    assert_eq!(value["target_project"], "foo");
    assert_eq!(value["target_package"], serde_json::Value::Null);
    assert_eq!(value["person"], serde_json::Value::Null);
    assert_eq!(
        value["group"],
        json!({"name": "factory-maintainers", "role": "reviewer"})
    );
    assert_eq!(
        value["description"],
        "Grant the group the reviewer role on foo"
    );
}

#[test]
fn request_file_add_role_person_and_group_dry_run_names_both() {
    // Person and group together carry the same role: the server
    // keeps one role per action.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "request",
            "file-add-role",
            "--target-project",
            "foo",
            "--target-package",
            "bar",
            "--person",
            "alice",
            "--person-role",
            "maintainer",
            "--group",
            "factory-maintainers",
            "--group-role",
            "maintainer",
            "--description",
            "Grant roles on bar",
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
    assert_eq!(value["dry_run"], true);
    assert_plan_non_empty(&value);
    assert_eq!(
        value["person"],
        json!({"name": "alice", "role": "maintainer"})
    );
    assert_eq!(
        value["group"],
        json!({"name": "factory-maintainers", "role": "maintainer"})
    );
}

#[test]
fn request_file_add_role_dry_run_text_names_the_grantee_and_target() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "request",
            "file-add-role",
            "--target-project",
            "foo",
            "--target-package",
            "bar",
            "--person",
            "alice",
            "--person-role",
            "maintainer",
            "--description",
            "Grant alice the maintainer role on bar",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "dry-run: would file add-role request for person alice as maintainer on foo/bar"
    );
}

#[test]
fn request_file_add_role_refuses_different_roles() {
    // The payload refuses a person and a group with different
    // roles before any credential resolution, like the invalid
    // search filters: the server keeps one role per action.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "request",
            "file-add-role",
            "--target-project",
            "foo",
            "--target-package",
            "bar",
            "--person",
            "alice",
            "--person-role",
            "maintainer",
            "--group",
            "factory-maintainers",
            "--group-role",
            "reviewer",
            "--description",
            "Grant roles on bar",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(error_of(&out)["error"]["code"], "invalid_input");
}

#[test]
fn request_file_add_role_refuses_no_grantee() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "request",
            "file-add-role",
            "--target-project",
            "foo",
            "--description",
            "Grant a role",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(error_of(&out)["error"]["code"], "invalid_input");
}

#[test]
fn request_file_add_role_live_against_closed_port_fails_with_network_error() {
    // `--live` flows through main.rs into the filing: with
    // credentials set and the base URL on a closed loopback port,
    // the dedup search fails with a network error instead of
    // silently dry-running.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "request",
            "file-add-role",
            "--target-project",
            "foo",
            "--person",
            "alice",
            "--person-role",
            "maintainer",
            "--description",
            "Grant a role",
        ])
        .env("GEECTL_OBS_URL", "http://127.0.0.1:1")
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "network_error");
}

#[test]
fn request_file_add_role_live_with_half_configured_credentials_fails_fast() {
    // A live run resolves credentials up front: a username without
    // a password source fails before any filing work.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .env("GEECTL_OBS_USERNAME", "alice")
        .args([
            "--live",
            "request",
            "file-add-role",
            "--target-project",
            "foo",
            "--person",
            "alice",
            "--person-role",
            "maintainer",
            "--description",
            "Grant a role",
        ])
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
fn request_file_add_role_dry_run_with_half_configured_credentials_prints_plan() {
    // A dry run never touches the server, so the half-configured
    // credential set above must not fail it.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .env("GEECTL_OBS_USERNAME", "alice")
        .args([
            "request",
            "file-add-role",
            "--target-project",
            "foo",
            "--person",
            "alice",
            "--person-role",
            "maintainer",
            "--description",
            "Grant a role",
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
    assert_eq!(value["dry_run"], true);
    assert_plan_non_empty(&value);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("[dry-run]"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
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
