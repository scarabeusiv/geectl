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
fn unknown_flag_fails_with_json_error_on_stderr() {
    let out = geectl().arg("--no-such-flag").output().unwrap();
    assert!(!out.status.success());
    // Parse errors go through the single JSON error path (exit 2,
    // InvalidInput), not clap's default usage text. Stdout stays
    // empty: it carries only the result payload.
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert!(value["error"]["message"].is_string());
}

// --- config show / config init integration tests (isolated env) ---

use std::fs;
use std::path::PathBuf;
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
            "geectl-cli-test-{}-{nanos}-{n}",
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
    let mut cmd = geectl();
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

#[test]
fn config_show_with_no_config_reports_defaults() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["config", "show"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["obs_url"], "https://api.opensuse.org");
    assert_eq!(value["obs_url_source"], "default");
    assert_eq!(value["gitea_url"], "https://src.opensuse.org");
    assert_eq!(value["gitea_url_source"], "default");
    assert_eq!(value["obs_auth"]["status"], "not_configured");
    assert_eq!(value["gitea_auth"]["status"], "not_configured");
    assert_eq!(value["gitea_ssh_key"]["status"], "not_configured");
    // The exact path, not a suffix: a suffix check passes for a
    // config file in the wrong directory.
    assert_eq!(
        value["config_file"].as_str().unwrap(),
        dir.0.join("geectl").join("config.toml").to_str().unwrap()
    );
}

#[test]
fn config_show_reports_config_file_source() {
    let dir = TempDir::new();
    let config_dir = dir.0.join("geectl");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "obs_url = \"https://obs.example.com\"\n",
    )
    .unwrap();
    let out = geectl_isolated(&dir)
        .args(["config", "show"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["obs_url"], "https://obs.example.com");
    assert_eq!(value["obs_url_source"], "config");
    assert_eq!(value["gitea_url_source"], "default");
}

#[test]
fn config_show_reports_environment_source() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["config", "show"])
        .env("GEECTL_OBS_URL", "https://obs.example.com")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["obs_url"], "https://obs.example.com");
    assert_eq!(value["obs_url_source"], "environment");
}

#[test]
fn config_show_reports_obs_username_but_never_the_password() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["config", "show"])
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(value["obs_auth"]["status"], "configured");
    assert_eq!(value["obs_auth"]["username"], "alice");
    // The password must not appear anywhere in the output.
    assert!(!stdout.contains("s3cr3t"));
    assert!(!String::from_utf8_lossy(&out.stderr).contains("s3cr3t"));
}

#[test]
fn config_show_refuses_a_credential_in_the_config_file() {
    let dir = TempDir::new();
    let config_dir = dir.0.join("geectl");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("config.toml"), "password = \"s3cr3t\"\n").unwrap();
    let out = geectl_isolated(&dir)
        .args(["config", "show"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    // A config error: exit 9, structured JSON on stderr, nothing on
    // stdout, and the secret value nowhere in the output.
    assert_eq!(out.status.code(), Some(9));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    let value: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    assert!(!stderr.contains("s3cr3t"));
}

#[test]
fn config_init_creates_the_template_0600_and_refuses_to_overwrite() {
    let dir = TempDir::new();
    let path = dir.0.join("geectl").join("config.toml");
    let out = geectl_isolated(&dir)
        .args(["config", "init"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["created"], true);
    // The exact path, not a suffix: a suffix check passes for a
    // config file in the wrong directory.
    assert_eq!(
        value["config_file"].as_str().unwrap(),
        path.to_str().unwrap()
    );
    let contents = fs::read_to_string(&path).unwrap();
    assert!(contents.contains("obs_url = \"https://api.opensuse.org\""));
    assert!(contents.contains("Credentials are NEVER stored here"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "config file must be 0600 from the first byte");
    }
    // A second init refuses to overwrite.
    let out = geectl_isolated(&dir)
        .args(["config", "init"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(9));
    assert_eq!(fs::read_to_string(&path).unwrap(), contents);
}

#[test]
fn config_show_text_format_renders_urls_and_sources() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--format", "text", "config", "show"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("obs_url: https://api.opensuse.org (default)"));
    assert!(stdout.contains("gitea_url: https://src.opensuse.org (default)"));
    assert!(stdout.contains("obs_auth: not_configured"));
}

#[test]
fn config_show_rejects_an_http_env_url() {
    // Env endpoint URLs are validated like file values: http is not
    // an endpoint URL.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["config", "show"])
        .env("GEECTL_OBS_URL", "http://api.opensuse.org")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(9));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    let value: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    // The variable name is in "context" (the specific detail);
    // "message" is the generic kind text.
    assert!(
        value["error"]["context"]
            .as_str()
            .unwrap()
            .contains("GEECTL_OBS_URL"),
        "{stderr}"
    );
}

#[test]
fn config_show_rejects_a_userinfo_env_url_without_echoing_the_secret() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["config", "show"])
        .env("GEECTL_OBS_URL", "https://user:secret@api.opensuse.org")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(9));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stderr.contains("secret"), "{stderr}");
    assert!(!stdout.contains("secret"), "{stdout}");
    let value: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    // The detail is in "context"; "message" is the generic kind text.
    assert!(
        value["error"]["context"]
            .as_str()
            .unwrap()
            .contains("userinfo"),
        "{stderr}"
    );
}

#[test]
fn config_show_reports_auth_error_with_code_and_remediation() {
    // A half-configured OBS pair (username, no password source) is a
    // config error, reported in the auth status — not as a failed
    // `config show`: the URLs are fine, the credential is not.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["config", "show"])
        .env("GEECTL_OBS_USERNAME", "alice")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["obs_auth"]["status"], "error");
    assert_eq!(value["obs_auth"]["code"], "config_error");
    assert!(
        value["obs_auth"]["message"]
            .as_str()
            .unwrap()
            .contains("GEECTL_OBS_USERNAME"),
        "{}",
        value["obs_auth"]
    );
    assert!(value["obs_auth"]["remediation"].is_string());
    assert!(!value["obs_auth"]["remediation"]
        .as_str()
        .unwrap()
        .is_empty());
}

#[test]
fn config_show_text_format_renders_an_auth_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--format", "text", "config", "show"])
        .env("GEECTL_OBS_USERNAME", "alice")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("obs_auth: error:"), "{stdout}");
    assert!(stdout.contains("GEECTL_OBS_USERNAME"), "{stdout}");
}

// --- debug channel ---

/// The two debug lines for an isolated `version` run: the startup
/// line for the given arguments, then the config line for the
/// default endpoints with no credentials.
fn expected_debug_lines(args: &str) -> String {
    format!(
        "geectl {} {args}\nconfig: obs api.opensuse.org (auth: none), gitea src.opensuse.org (auth: none)\n",
        env!("CARGO_PKG_VERSION")
    )
}

#[test]
fn debug_off_by_default_emits_no_debug_lines() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir).arg("version").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).is_empty());
}

#[test]
fn debug_flag_emits_startup_and_config_lines() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--debug", "version"])
        .output()
        .unwrap();
    assert!(out.status.success());
    // Stdout is exactly the JSON payload: debug never changes it.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(value["name"], "geectl");
    assert_eq!(stdout.trim().lines().count(), 1);
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        expected_debug_lines("--debug version")
    );
}

#[test]
fn debug_env_enables_without_flag() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .env("GEECTL_DEBUG", "1")
        .arg("version")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        expected_debug_lines("version")
    );
}

#[test]
fn debug_env_values_other_than_one_disable_without_flag() {
    for value in ["", "0", "yes"] {
        let dir = TempDir::new();
        let out = geectl_isolated(&dir)
            .env("GEECTL_DEBUG", value)
            .arg("version")
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stderr).is_empty(),
            "GEECTL_DEBUG={value:?} must not enable the channel"
        );
    }
}

#[test]
fn debug_flag_after_subcommand_works() {
    // The flags are global, so they parse after the subcommand too.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["version", "--debug"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        expected_debug_lines("version --debug")
    );
}

#[test]
fn debug_body_implies_debug() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--debug-body", "version"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        expected_debug_lines("--debug-body version")
    );
}

#[test]
fn debug_lines_are_identical_in_text_format() {
    // Composition with --format is orthogonal: the same debug lines
    // in both modes, and stdout carries the text payload.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--format", "text", "--debug", "version"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.trim(),
        format!("geectl {}", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        expected_debug_lines("--format text --debug version")
    );
}

#[test]
fn debug_config_line_never_shows_a_secret() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--debug", "version"])
        .env("GEECTL_OBS_USERNAME", "alice-test")
        .env("GEECTL_OBS_PASSWORD", "hunter2-fake")
        .env("GEECTL_GITEA_TOKEN", "tok-fake")
        .output()
        .unwrap();
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The config line names the auth kinds, never the values.
    assert!(stderr.contains(
        "config: obs api.opensuse.org (auth: basic), gitea src.opensuse.org (auth: bearer)"
    ));
    for secret in ["alice-test", "hunter2-fake", "tok-fake"] {
        assert!(!stderr.contains(secret), "debug line leaked {secret}");
    }
}

// --- remote-read integration tests ---

#[test]
fn remote_help_lists_every_command() {
    let out = geectl().args(["remote", "--help"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    for command in [
        "cat",
        "list-files",
        "list-packages",
        "package-log",
        "revision-patch",
        "project-meta",
        "package-meta",
        "search-packages",
        "search-projects",
        "owners",
        "owners-for-binary",
        "maintainers",
        "whois",
        "link-target",
        "project-diff",
        "devel-project",
    ] {
        assert!(
            stdout.contains(command),
            "remote --help is missing {command}"
        );
    }
}

#[test]
fn remote_cat_help_names_the_rev_flag() {
    let out = geectl().args(["remote", "cat", "--help"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--rev"));
    assert!(stdout.contains("project"));
    assert!(stdout.contains("package"));
    assert!(stdout.contains("file"));
}

#[test]
fn remote_unknown_command_fails_with_json_error() {
    let out = geectl().args(["remote", "frobnicate"]).output().unwrap();
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
}

#[test]
fn remote_invalid_name_fails_fast_without_network() {
    // An invalid OBS name is rejected by the library before any HTTP
    // call: the error is InvalidInput (exit 2) and the run never
    // touches the network, so no server is needed.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "list-packages", "!!not-a-name"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert!(value["error"]["message"].is_string());
    assert!(value["error"]["remediation"].is_string());
}

#[test]
fn remote_invalid_name_in_text_mode_renders_text_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "remote",
            "list-packages",
            "!!not-a-name",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error: the input is invalid"));
    assert!(stderr.contains("remediation:"));
    assert!(stderr.contains("context:"));
    // Text mode carries no JSON error payload.
    assert!(!stderr.contains("\"error\""));
}

#[test]
fn remote_invalid_range_fails_fast_without_network() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "package-log",
            "openSUSE:Factory",
            "vim",
            "--range",
            "not-a-range",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
}

#[test]
fn remote_unreachable_server_fails_with_network_error() {
    // The base URL points at a closed loopback port: no server
    // answers, so the library fails with a network error and the CLI
    // reports it structurally, without ever reaching a real OBS.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "list-packages", "openSUSE:Factory"])
        .env("GEECTL_OBS_URL", "http://127.0.0.1:1")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "network_error");
    assert!(value["error"]["message"].is_string());
    assert!(value["error"]["remediation"].is_string());
}

// --- remote-read success-path fixture tests ---

/// A one-shot HTTP fixture: serves a single canned response to the
/// first connection, then the thread ends. Lets a remote command run
/// its success path offline. The thread is deliberately not joined:
/// if the binary never connects, the test fails on its own assertion
/// and the blocked thread dies with the test process.
fn serve_once(body: &str) -> std::net::SocketAddr {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let body = body.to_string();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    });
    addr
}

#[test]
fn remote_list_packages_text_mode_renders_the_table() {
    let addr = serve_once("<directory><entry name=\"curl\"/><entry name=\"vim\"/></directory>");
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "remote",
            "list-packages",
            "openSUSE:Factory",
        ])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "NAME\ncurl\nvim");
}

#[test]
fn remote_list_packages_json_mode_renders_the_payload() {
    let addr = serve_once("<directory><entry name=\"curl\"/><entry name=\"vim\"/></directory>");
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "list-packages", "openSUSE:Factory"])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["project"], "openSUSE:Factory");
    assert_eq!(value["packages"], serde_json::json!(["curl", "vim"]));
}

#[test]
fn remote_unknown_command_in_text_mode_renders_text_error() {
    // The format was selected on the command line, so even a parse
    // error renders in it (decision 52.3).
    let out = geectl()
        .args(["--format", "text", "remote", "frobnicate"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error: the input is invalid"));
    assert!(stderr.contains("remediation:"));
    assert!(!stderr.contains("\"error\""));
}
