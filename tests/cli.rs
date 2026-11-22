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
        .env_remove("GEECTL_DEBUG")
        .env_remove("GEECTL_FORMAT");
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
fn config_show_reports_alias_token_for_the_configured_instance() {
    let dir = TempDir::new();
    let config_dir = dir.0.join("geectl");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "[instances]\nmyinstance = \"https://src.opensuse.org\"\n",
    )
    .unwrap();
    let out = geectl_isolated(&dir)
        .args(["config", "show"])
        .env("GEECTL_GITEA_TOKEN_MYINSTANCE", "alias-token")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    // The default URL is named in [instances], so the alias token
    // is the credential it uses — not "not_configured".
    assert_eq!(value["gitea_auth"]["status"], "configured");
    assert_eq!(value["instances"]["myinstance"], "https://src.opensuse.org");
    // The token must not appear anywhere in the output.
    assert!(!stdout.contains("alias-token"));
    assert!(!String::from_utf8_lossy(&out.stderr).contains("alias-token"));
}

#[test]
fn config_show_text_format_renders_instances() {
    let dir = TempDir::new();
    let config_dir = dir.0.join("geectl");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "[instances]\nmyinstance = \"https://src.opensuse.org\"\n",
    )
    .unwrap();
    let out = geectl_isolated(&dir)
        .args(["--format", "text", "config", "show"])
        .env("GEECTL_GITEA_TOKEN_MYINSTANCE", "alias-token")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("instances: myinstance -> https://src.opensuse.org"));
    assert!(stdout.contains("gitea_auth: configured"));
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

#[test]
fn debug_config_line_reports_alias_credentials() {
    // Alias-only credentials (no plain variables): the debug
    // config line must report the OBS auth kind for the effective
    // URL, not the plain variables only.
    let dir = TempDir::new();
    let config_dir = dir.0.join("geectl");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "[instances]\nmyloveinstance = \"https://api.opensuse.org\"\n",
    )
    .unwrap();
    let out = geectl_isolated(&dir)
        .args(["--debug", "version"])
        .env("GEECTL_OBS_USERNAME_MYLOVEINSTANCE", "target-user")
        .env("GEECTL_OBS_PASSWORD_MYLOVEINSTANCE", "target-secret")
        .output()
        .unwrap();
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains(
        "config: obs api.opensuse.org (auth: basic), gitea src.opensuse.org (auth: none)"
    ));
    assert!(!stderr.contains("target-secret"));
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
        "diff-packages",
        "devel-project",
        "repo-add",
        "repo-remove",
        "list-projects",
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
    // Unified error rendering (the request group owns write_error):
    // the context names the failure in the `error:` line itself;
    // there is no separate `context:` line.
    assert!(stderr.contains("error: invalid OBS project name: \"!!not-a-name\""));
    assert!(stderr.contains("remediation:"));
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
/// A one-shot HTTP fixture serving an explicit status line: the
/// same single-connection shape as `serve_once`, for the failure
/// paths (a credentialed read answered 401).
fn serve_once_status(status: u16, reason: &str, body: &str) -> std::net::SocketAddr {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let body = body.to_string();
    let reason = reason.to_string();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    });
    addr
}

fn serve_once(body: &str) -> std::net::SocketAddr {
    serve_once_status(200, "OK", body)
}

/// A many-shot HTTP fixture: serves the same canned response to
/// every connection, until the test process ends. For commands that
/// fan out (a diff lists both sides, then reads files), where one
/// canned body answers every request. The thread is deliberately
/// not joined, like `serve_once`: if the binary never connects, the
/// test fails on its own assertion and the thread dies with the
/// test process.
fn serve_many(body: &str) -> std::net::SocketAddr {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let body = body.to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
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
    assert_eq!(stdout.trim(), "project: openSUSE:Factory\nNAME\ncurl\nvim");
}

#[test]
fn remote_list_files_text_mode_opens_with_the_coordinate_header() {
    let addr = serve_once(
        "<directory><entry name=\"vim.spec\" size=\"1234\" mtime=\"1700000000\" md5=\"deadbeef\"/></directory>",
    );
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "remote",
            "list-files",
            "openSUSE:Factory",
            "vim",
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
    let mut lines = stdout.lines();
    assert_eq!(
        lines.next(),
        Some("project: openSUSE:Factory package: vim"),
        "{stdout}"
    );
    assert!(stdout.contains("vim.spec"), "{stdout}");
    // The md5 stays JSON-only.
    assert!(!stdout.contains("deadbeef"), "{stdout}");
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
fn remote_unknown_command_in_text_mode_stays_json() {
    // A parse error has no parsed Cli yet, so it stays JSON even in
    // text mode: the parse-error path is JSON, never a guess.
    let out = geectl()
        .args(["--format", "text", "remote", "frobnicate"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
}

// --- remote lazy credential resolution (issue #82) ---
//
// The public reads never resolve credentials, so a half-set env
// (username without password) must not break them; the credentialed
// reads and a live mutation fail fast with a config error. The
// fixture-backed tests prove the public read actually runs, offline:
// a half-set env would fail before the first request.

#[test]
fn remote_list_packages_ignores_half_set_credentials() {
    let addr = serve_once("<directory><entry name=\"curl\"/><entry name=\"vim\"/></directory>");
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "list-packages", "openSUSE:Factory"])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
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
    assert_eq!(value["packages"], serde_json::json!(["curl", "vim"]));
}

#[test]
fn remote_list_packages_needs_no_credentials() {
    // With nothing configured the public read runs with no
    // credentials at all — this guards the no-credential read
    // path, not the resolution timing.
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
    assert_eq!(value["packages"], serde_json::json!(["curl", "vim"]));
}

#[test]
fn remote_cat_ignores_half_set_credentials() {
    let addr = serve_once("hello from the fixture");
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "cat", "openSUSE:Factory", "vim", "vim.spec"])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
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
    assert_eq!(value["content"], "hello from the fixture");
}

#[test]
fn remote_public_reads_ignore_half_set_credentials() {
    // Every remaining public read, one case each: a half-set env must
    // not fail it, and the fixture body lets the command succeed, so
    // success proves the read ran past credential resolution. The
    // fan-out commands (`diff-packages`, `project-diff`) get the
    // many-shot fixture — one canned body answers every request —
    // and an empty listing on both sides diffs to nothing.
    let cases: &[(&[&str], &str, bool)] = &[
        (
            &["remote", "list-files", "openSUSE:Factory", "vim"],
            "<directory><entry name=\"vim.spec\" size=\"1234\" mtime=\"1700000000\" md5=\"deadbeef\"/></directory>",
            false,
        ),
        (
            &["remote", "project-meta", "openSUSE:Factory"],
            "<project name=\"openSUSE:Factory\"><title>T</title><description>D</description></project>",
            false,
        ),
        (
            &["remote", "package-meta", "openSUSE:Factory", "vim"],
            "<package name=\"vim\" project=\"openSUSE:Factory\"><title>T</title><description>D</description></package>",
            false,
        ),
        (
            &["remote", "link-target", "openSUSE:Factory", "vim"],
            "<directory><linkinfo project=\"openSUSE:Factory\" package=\"vim\"/></directory>",
            false,
        ),
        (
            &["remote", "devel-project", "foo"],
            "<package name=\"foo\" project=\"openSUSE:Factory\"><devel project=\"devel:languages:python\" package=\"python-foo\"/></package>",
            false,
        ),
        (
            &["remote", "revision-patch", "openSUSE:Factory", "vim", "1"],
            "<directory/>",
            false,
        ),
        (
            &[
                "remote",
                "diff-packages",
                "openSUSE:Factory",
                "vim",
                "openSUSE:Factory",
                "vim",
            ],
            "<directory/>",
            true,
        ),
        (
            &["remote", "project-diff", "openSUSE:Factory", "openSUSE:Factory"],
            "<directory/>",
            true,
        ),
    ];
    for (args, body, many) in cases {
        let addr = if *many {
            serve_many(body)
        } else {
            serve_once(body)
        };
        let dir = TempDir::new();
        let out = geectl_isolated(&dir)
            .args(*args)
            .env("GEECTL_OBS_URL", format!("http://{addr}"))
            .env("GEECTL_OBS_USERNAME", "alice")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "args {args:?} failed: stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        // The payload parses: the command ran to completion.
        let _: serde_json::Value =
            serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim())
                .unwrap_or_else(|_| panic!("args {args:?}: stdout is not JSON"));
    }
}

#[test]
fn remote_search_packages_with_half_set_credentials_fails_with_config_error() {
    // The credentialed reads still resolve up front and fail fast on
    // a half-set env — the asymmetry with the public reads is
    // intended.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "search-packages", "vim"])
        .env("GEECTL_OBS_USERNAME", "alice")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
}

#[test]
fn remote_list_projects_with_half_set_credentials_fails_with_config_error() {
    // `list-projects` has no public route: it reads with the
    // client's credentials, so it fails fast like the searches.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "list-projects"])
        .env("GEECTL_OBS_USERNAME", "alice")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
}

// --- unconfigured credentialed read (issue #108) ---
//
// With nothing configured the credentialed read goes out without
// credentials: the server answers 401 and the CLI reports the auth
// failure structurally. The half-set case fails fast with a config
// error; the unconfigured case reaches the server and fails on its
// 401 — both fail closed, never an empty result.

#[test]
fn remote_list_projects_without_credentials_fails_with_auth_error() {
    // `list-projects` has no public route, so with no credentials
    // configured the read goes out unauthenticated and the server
    // 401s. The CLI reports the auth failure (exit 4), never an
    // empty project list.
    let addr = serve_once_status(
        401,
        "Unauthorized",
        "<status code=\"401\"><summary>Unauthorized</summary></status>",
    );
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "list-projects"])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "auth_failed");
    assert!(value["error"]["message"].is_string());
    assert!(value["error"]["remediation"].is_string());
}

#[test]
fn remote_project_diff_annotate_requests_with_half_set_credentials_fails_with_config_error() {
    // The diff itself is public, but `--annotate-requests` needs
    // credentials for the open-request search, so it fails fast.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "project-diff",
            "openSUSE:Factory",
            "openSUSE:Factory",
            "--annotate-requests",
        ])
        .env("GEECTL_OBS_USERNAME", "alice")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
}

#[test]
fn remote_repo_add_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "repo-add",
            "openSUSE:Factory",
            "standard",
            "--arch",
            "x86_64",
            "--path",
            "openSUSE:Factory/standard",
        ])
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
    assert_eq!(value["dry_run"], true);
}

#[test]
fn remote_repo_remove_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "repo-remove", "openSUSE:Factory", "standard"])
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
    assert_eq!(value["dry_run"], true);
}

#[test]
fn remote_repo_add_live_with_half_set_credentials_fails_with_config_error() {
    // A live mutation does need credentials, so it fails fast.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "remote",
            "repo-add",
            "openSUSE:Factory",
            "standard",
            "--arch",
            "x86_64",
            "--path",
            "openSUSE:Factory/standard",
        ])
        .env("GEECTL_OBS_USERNAME", "alice")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
}

#[test]
fn remote_repo_remove_live_with_half_set_credentials_fails_with_config_error() {
    // A live mutation does need credentials, so it fails fast.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "remote",
            "repo-remove",
            "openSUSE:Factory",
            "standard",
        ])
        .env("GEECTL_OBS_USERNAME", "alice")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    // The context names the half-set pair, which only the
    // CLI-level fail-fast produces: the library's own
    // credentials check reports a missing-credentials message
    // instead. This pins the fail-fast layer, not just the code.
    assert!(
        value["error"]["context"]
            .as_str()
            .unwrap()
            .contains("is set but no password source is"),
        "context: {}",
        value["error"]["context"]
    );
}

// --- branch/copy/link integration tests (isolated env, dry-run) ---
//
// The dry-run guard runs before the first HTTP call in all ten
// operations, so these need neither credentials nor a server: the
// isolated env has none of either. The plan lives in the JSON
// payload in JSON mode and on stdout in text mode; the guard also
// prints each plan line to stderr as it records it.

/// Run an isolated dry-run branch command and return its JSON
/// payload: exit 0, stdout is the payload with `dry_run: true` and
/// a non-empty `[dry-run]` plan. The plan lines are also printed to
/// stderr, one per line, as the dry-run guard records them — that
/// is the `[dry-run]` output, not the debug channel.
fn branch_dry_json(dir: &TempDir, args: &[&str]) -> serde_json::Value {
    let out = geectl_isolated(dir)
        .args(["branch"])
        .args(args)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "args: {args:?}\nstderr: {stderr}");
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["dry_run"], true, "payload: {value}");
    let plan = value["plan"].as_array().expect("payload carries a plan");
    assert!(!plan.is_empty(), "plan must not be empty: {value}");
    let stderr_lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(
        stderr_lines.len(),
        plan.len(),
        "stderr lines must match the plan: {stderr:?} vs {plan:?}"
    );
    for (line, expected) in stderr_lines.iter().zip(plan.iter()) {
        assert!(line.contains("[dry-run]"), "stderr line: {line}");
        assert_eq!(*line, expected.as_str().expect("plan lines are strings"));
    }
    value
}

/// A branch command with missing required flags: exit 2, a JSON
/// error on stderr, empty stdout.
fn branch_missing_flags(args: &[&str]) {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["branch"])
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "args: {args:?}");
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert!(value["error"]["message"].is_string());
}

#[test]
fn branch_help_exits_zero() {
    let out = geectl().args(["branch", "--help"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("branch-package"), "{stdout}");
    assert!(stdout.contains("copy-package"), "{stdout}");
}

#[test]
fn branch_commands_reject_missing_required_flags() {
    for args in [
        ["branch-package"],
        ["copy-package"],
        ["copy-project"],
        ["set-devel-project"],
        ["link-package"],
        ["repair-link"],
        ["set-link-rev"],
        ["set-link-rev-project"],
        ["link-to-branch"],
        ["detach-branch"],
    ] {
        branch_missing_flags(&args);
    }
}

#[test]
fn branch_package_dry_run() {
    let dir = TempDir::new();
    let value = branch_dry_json(
        &dir,
        &[
            "branch-package",
            "--project",
            "test:factory",
            "--package",
            "foo",
        ],
    );
    assert_eq!(value["source_project"], "test:factory");
    assert_eq!(value["source_package"], "foo");
    // The target is the requested one when the flags named it, else
    // null: the server resolves it only when live.
    assert_eq!(value["target_project"], serde_json::Value::Null);
    assert_eq!(value["target_package"], serde_json::Value::Null);
}

#[test]
fn copy_package_dry_run() {
    let dir = TempDir::new();
    let value = branch_dry_json(
        &dir,
        &[
            "copy-package",
            "--source-project",
            "test:factory",
            "--source-package",
            "foo",
            "--target-project",
            "test:target",
            "--target-package",
            "foo",
        ],
    );
    assert_eq!(value["source_project"], "test:factory");
    assert_eq!(value["target_project"], "test:target");
    assert_eq!(value["revision"], serde_json::Value::Null);
}

#[test]
fn copy_package_to_apiurl_dry_run() {
    // A valid URL value: loopback http is a valid base, and the
    // dry-run guard runs before any server read, so no second
    // instance needs to exist.
    let dir = TempDir::new();
    let value = branch_dry_json(
        &dir,
        &[
            "copy-package",
            "--source-project",
            "test:factory",
            "--source-package",
            "foo",
            "--target-project",
            "test:target",
            "--target-package",
            "foo",
            "--to-apiurl",
            "http://127.0.0.1:3000",
        ],
    );
    assert_eq!(value["target_api_url"], "http://127.0.0.1:3000");
    assert_eq!(value["revision"], serde_json::Value::Null);
    // Dry text never reports counts: the payload omits them.
    assert!(value.get("copied_files").is_none());
    assert!(value.get("skipped_files").is_none());
}

#[test]
fn copy_project_dry_run() {
    let dir = TempDir::new();
    let value = branch_dry_json(
        &dir,
        &[
            "copy-project",
            "--source-project",
            "test:factory",
            "--target-project",
            "test:target",
        ],
    );
    assert_eq!(value["source_project"], "test:factory");
    assert_eq!(value["target_project"], "test:target");
}

#[test]
fn set_devel_project_dry_run() {
    let dir = TempDir::new();
    let value = branch_dry_json(
        &dir,
        &[
            "set-devel-project",
            "--project",
            "test:factory",
            "--package",
            "foo",
            "--devel-project",
            "devel:languages:python",
            "--devel-package",
            "foo",
        ],
    );
    assert_eq!(value["project"], "test:factory");
    assert_eq!(value["package"], "foo");
    assert_eq!(value["devel_project"], "devel:languages:python");
    assert_eq!(value["devel_package"], "foo");
}

#[test]
fn set_devel_project_removal_dry_run() {
    // Without --devel-project the devel project is removed: the
    // payload's devel fields are null.
    let dir = TempDir::new();
    let value = branch_dry_json(
        &dir,
        &[
            "set-devel-project",
            "--project",
            "test:factory",
            "--package",
            "foo",
        ],
    );
    assert_eq!(value["devel_project"], serde_json::Value::Null);
    assert_eq!(value["devel_package"], serde_json::Value::Null);
}

#[test]
fn link_package_dry_run() {
    let dir = TempDir::new();
    let value = branch_dry_json(
        &dir,
        &[
            "link-package",
            "--source-project",
            "test:factory",
            "--source-package",
            "foo",
            "--target-project",
            "test:target",
            "--target-package",
            "foo",
        ],
    );
    assert_eq!(value["source_project"], "test:factory");
    assert_eq!(value["target_package"], "foo");
}

#[test]
fn repair_link_dry_run() {
    let dir = TempDir::new();
    let value = branch_dry_json(
        &dir,
        &[
            "repair-link",
            "--project",
            "test:factory",
            "--package",
            "foo",
        ],
    );
    assert_eq!(value["project"], "test:factory");
    assert_eq!(value["package"], "foo");
}

#[test]
fn set_link_rev_dry_run() {
    let dir = TempDir::new();
    let value = branch_dry_json(
        &dir,
        &[
            "set-link-rev",
            "--project",
            "test:factory",
            "--package",
            "foo",
            "--revision",
            "latest",
        ],
    );
    assert_eq!(value["project"], "test:factory");
    assert_eq!(value["package"], "foo");
    // The requested revision is not echoed in dry text: the payload
    // revision is null until the set happens.
    assert_eq!(value["revision"], serde_json::Value::Null);
}

#[test]
fn set_link_rev_project_dry_run() {
    let dir = TempDir::new();
    let value = branch_dry_json(
        &dir,
        &[
            "set-link-rev-project",
            "--project",
            "test:factory",
            "--revision",
            "latest",
        ],
    );
    assert_eq!(value["project"], "test:factory");
    // The set is empty by construction in dry-run.
    assert_eq!(value["set"], serde_json::json!([]));
}

#[test]
fn link_to_branch_dry_run() {
    let dir = TempDir::new();
    let value = branch_dry_json(
        &dir,
        &[
            "link-to-branch",
            "--project",
            "test:factory",
            "--package",
            "foo",
        ],
    );
    assert_eq!(value["project"], "test:factory");
    assert_eq!(value["package"], "foo");
}

#[test]
fn detach_branch_dry_run() {
    let dir = TempDir::new();
    let value = branch_dry_json(
        &dir,
        &[
            "detach-branch",
            "--project",
            "test:factory",
            "--package",
            "foo",
        ],
    );
    assert_eq!(value["project"], "test:factory");
    assert_eq!(value["package"], "foo");
    assert_eq!(value["revision"], serde_json::Value::Null);
}

#[test]
fn branch_package_dry_run_text_echoes_the_header() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "branch",
            "branch-package",
            "--project",
            "test:factory",
            "--package",
            "foo",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // The header line echoes the query coordinates; the JSON
    // payload's names appear in it.
    assert!(
        stdout.starts_with("branch-package test:factory/foo\ndry_run: true\n"),
        "{stdout}"
    );
    assert!(stdout.contains("[dry-run]"), "{stdout}");
}

#[test]
fn set_link_rev_project_dry_run_text_echoes_the_header() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "branch",
            "set-link-rev-project",
            "--project",
            "test:factory",
            "--revision",
            "latest",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("set-link-rev-project test:factory\ndry_run: true\n"),
        "{stdout}"
    );
    assert!(stdout.contains("[dry-run]"), "{stdout}");
    // Dry: the set is empty by construction, so there is no
    // `no links set` line.
    assert!(!stdout.contains("no links set"), "{stdout}");
}

#[test]
fn copy_package_to_apiurl_rejects_a_malformed_url_in_dry_run() {
    // The target client is built (and its base validated) before the
    // copy runs, so a malformed URL fails fast in dry-run too —
    // before any read, with no network touched.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "branch",
            "copy-package",
            "--source-project",
            "test:factory",
            "--source-package",
            "foo",
            "--target-project",
            "test:target",
            "--target-package",
            "foo",
            "--to-apiurl",
            "notaurl",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert!(value["error"]["message"].is_string());
}

#[test]
fn copy_package_to_apiurl_dry_run_text_echoes_the_target_instance() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "branch",
            "copy-package",
            "--source-project",
            "test:factory",
            "--source-package",
            "foo",
            "--target-project",
            "test:target",
            "--target-package",
            "foo",
            "--to-apiurl",
            "http://127.0.0.1:3000",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // The target instance rides the header after `@`.
    assert!(
        stdout.starts_with(
            "copy-package test:factory/foo -> test:target/foo @ http://127.0.0.1:3000\ndry_run: true\n"
        ),
        "{stdout}"
    );
    assert!(stdout.contains("[dry-run]"), "{stdout}");
}

// --- build commands integration tests (isolated env) ---

#[test]
fn build_help_lists_all_fifteen_subcommands() {
    let out = geectl().args(["build", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    for name in [
        "results",
        "wait-for-results",
        "build-log",
        "build-status",
        "job-status",
        "buildinfo",
        "build-history",
        "job-history",
        "list-binaries",
        "download-binary",
        "fileinfo",
        "dependson",
        "whatdependson",
        "check-constraints",
        "list-repositories",
    ] {
        assert!(text.contains(name), "build --help misses {name}");
    }
}

#[test]
fn service_help_lists_both_subcommands() {
    let out = geectl().args(["service", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    for name in ["list", "state"] {
        assert!(text.contains(name), "service --help misses {name}");
    }
}

#[test]
fn top_help_lists_the_service_group() {
    let out = geectl().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("service"),
        "top --help misses the service group"
    );
}

#[test]
fn gitea_help_lists_all_eight_subcommands() {
    let out = geectl().args(["gitea", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    for name in [
        "repo-get",
        "repo-list",
        "repo-create",
        "repo-delete",
        "repo-fork",
        "branch-list",
        "branch-get",
        "branch-head",
    ] {
        assert!(text.contains(name), "gitea --help misses {name}");
    }
}

#[test]
fn top_help_lists_the_gitea_group() {
    let out = geectl().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("gitea"), "top --help misses the gitea group");
}

#[test]
fn build_results_without_credentials_fails_with_config_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["build", "results", "openSUSE:Factory"])
        .output()
        .unwrap();
    // ConfigError exits 9; the build reads require OBS Basic
    // credentials and fail loudly without them.
    assert_eq!(out.status.code(), Some(9));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    assert!(value["error"]["context"]
        .as_str()
        .unwrap()
        .contains("GEECTL_OBS_USERNAME"));
}

#[test]
fn build_results_without_credentials_renders_text_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--format", "text", "build", "results", "openSUSE:Factory"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Text mode renders `error:` + `remediation:` lines, not JSON.
    assert!(stderr.contains("error: reading build state requires OBS credentials"));
    assert!(stderr.contains("remediation: check the configuration"));
    assert!(serde_json::from_str::<serde_json::Value>(stderr.trim()).is_err());
}

#[test]
fn build_download_binary_without_output_fails_with_parse_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "build",
            "download-binary",
            "openSUSE:Factory",
            "foo",
            "standard",
            "x86_64",
            "foo-1.0-1.x86_64.rpm",
        ])
        .output()
        .unwrap();
    // `--output` is required: a missing required argument is a parse
    // error (exit 2, InvalidInput) with a JSON error on stderr.
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
}

#[test]
fn build_list_repositories_needs_no_credentials() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["build", "list-repositories", "openSUSE:Factory"])
        .output()
        .unwrap();
    // The public read: whatever the outcome (success, or a network /
    // server failure in a sandbox without OBS access), it must not be
    // the ConfigError the credentialed reads fail with.
    assert_ne!(out.status.code(), Some(9));
    if !out.status.success() {
        let value: serde_json::Value =
            serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
        assert_ne!(value["error"]["code"], "config_error");
    }
}

#[test]
fn build_list_repositories_ignores_half_set_credentials() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["build", "list-repositories", "openSUSE:Factory"])
        .env("GEECTL_OBS_USERNAME", "alice")
        .output()
        .unwrap();
    // Username without password is a ConfigError for the credentialed
    // reads — but the public read never resolves credentials, so it
    // must not fail that way.
    assert_ne!(out.status.code(), Some(9));
    if !out.status.success() {
        let value: serde_json::Value =
            serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
        assert_ne!(value["error"]["code"], "config_error");
    }
}

#[test]
fn build_results_with_half_set_credentials_fails_with_config_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["build", "results", "openSUSE:Factory"])
        .env("GEECTL_OBS_USERNAME", "alice")
        .output()
        .unwrap();
    // The credentialed reads resolve credentials eagerly and fail loud
    // on a half-set env — the asymmetry with the public read is
    // intended.
    assert_eq!(out.status.code(), Some(9));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
}

#[test]
fn build_unknown_subcommand_fails_with_parse_error() {
    let out = geectl().args(["build", "frobnicate"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
}

// --- Docs golden test, alias credentials,
// supersede guard, whole-project guard, output-format default ---

/// Extract the `--help` blocks from docs/cli.md: each ``` fence
/// whose content holds a `Usage: geectl ...` line. Returns
/// `(argv, expected_stdout)` per block, where argv is the path from
/// the Usage line (the tokens after `geectl`, up to the first `[` or
/// `<` token). The ```sh and ```toml fences are usage/config
/// snippets, not --help output, and are skipped (they hold no Usage
/// line).
fn help_blocks(docs: &str) -> Vec<(Vec<String>, String)> {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in docs.lines() {
        // Any fenced block toggles: ```, ```sh, ```toml — a
        // fenced snippet that is not a --help block is skipped below
        // for holding no Usage line.
        if line.trim().starts_with("```") {
            if let Some(lines) = current.take() {
                let text = lines.join("\n");
                if let Some(usage) = text.lines().find(|l| l.starts_with("Usage: geectl")) {
                    let argv: Vec<String> = usage
                        .trim_start_matches("Usage: geectl")
                        .split_whitespace()
                        .take_while(|t| !t.starts_with('[') && !t.starts_with('<'))
                        .map(str::to_string)
                        .collect();
                    blocks.push((argv, format!("{text}\n")));
                }
            } else {
                current = Some(Vec::new());
            }
        } else if let Some(lines) = &mut current {
            lines.push(line);
        }
    }
    blocks
}

#[test]
fn docs_help_blocks_match_the_binary() {
    // The docs carry the binary's --help output verbatim; this test
    // makes that permanent. A block that drifts fails here — regenerate
    // it from the built binary (`geectl <path> --help`), never by hand.
    // The docs carry 15 blocks (top level, 7 groups, 7 leaves) — a
    // sampled surface, not every leaf: a leaf with no docs block can
    // change --help undetected. That is deliberate — the docs show the
    // groups and one leaf each — and the count assertion guards the
    // docs side only.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let docs = fs::read_to_string(manifest.join("docs/cli.md")).expect("docs/cli.md");
    let blocks = help_blocks(&docs);
    assert_eq!(
        blocks.len(),
        15,
        "docs/cli.md should carry 15 --help blocks (top level, 7 groups, 7 leaves);          update this count when a block is deliberately added or removed"
    );
    for (argv, expected) in blocks {
        let mut cmd = geectl();
        cmd.args(&argv).arg("--help");
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "geectl {argv:?} --help failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let actual = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            actual,
            expected,
            "docs/cli.md --help block drifted for `geectl {}`",
            argv.join(" ")
        );
    }
}

#[test]
fn copy_package_to_apiurl_with_plain_credentials_fails_clearly() {
    // Plain (default-instance) credentials must never be forwarded
    // to the target: a different-URL target with only plain creds
    // configured fails with a config error naming the `[instances]`
    // entry to add (no alias matches the target URL yet).
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "branch",
            "copy-package",
            "--source-project",
            "test:factory",
            "--source-package",
            "foo",
            "--target-project",
            "test:target",
            "--target-package",
            "foo",
            "--to-apiurl",
            "http://127.0.0.1:3000",
        ])
        .env("GEECTL_OBS_USERNAME", "plain-user")
        .env("GEECTL_OBS_PASSWORD", "plain-secret")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let value: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    // The specific failure is in `context`; `message` is the kind's
    // one-sentence summary.
    let message = value["error"]["context"].as_str().unwrap();
    assert!(message.contains("[instances]"), "stderr: {stderr}");
    assert!(
        message.contains("http://127.0.0.1:3000"),
        "stderr: {stderr}"
    );
    // The default's credentials never leak into the message.
    assert!(!stderr.contains("plain-user"));
    assert!(!stderr.contains("plain-secret"));
}

#[test]
fn copy_package_to_apiurl_with_alias_defined_but_unset_names_the_alias_vars() {
    // An alias matches the target URL but its variables are not
    // set, while plain credentials exist: the error names the alias
    // variables, not the `[instances]` entry (the alias exists).
    let dir = TempDir::new();
    let config_dir = dir.0.join("geectl");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "[instances]\ntarget = \"http://127.0.0.1:3000\"\n",
    )
    .unwrap();
    let out = geectl_isolated(&dir)
        .args([
            "branch",
            "copy-package",
            "--source-project",
            "test:factory",
            "--source-package",
            "foo",
            "--target-project",
            "test:target",
            "--target-package",
            "foo",
            "--to-apiurl",
            "http://127.0.0.1:3000",
        ])
        .env("GEECTL_OBS_USERNAME", "plain-user")
        .env("GEECTL_OBS_PASSWORD", "plain-secret")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let value: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    let message = value["error"]["context"].as_str().unwrap();
    assert!(
        message.contains("GEECTL_OBS_USERNAME_TARGET"),
        "stderr: {stderr}"
    );
    assert!(
        message.contains("GEECTL_OBS_PASSWORD_TARGET"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("plain-user"));
    assert!(!stderr.contains("plain-secret"));
}

#[test]
fn copy_package_to_apiurl_with_alias_credentials_dry_runs() {
    // Alias credentials for the target let the dry run proceed;
    // the source needs none in dry-run (the guard runs before any
    // server read).
    let dir = TempDir::new();
    let config_dir = dir.0.join("geectl");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "[instances]\ntarget = \"http://127.0.0.1:3000\"\n",
    )
    .unwrap();
    let out = geectl_isolated(&dir)
        .args([
            "branch",
            "copy-package",
            "--source-project",
            "test:factory",
            "--source-package",
            "foo",
            "--target-project",
            "test:target",
            "--target-package",
            "foo",
            "--to-apiurl",
            "http://127.0.0.1:3000",
        ])
        .env("GEECTL_OBS_USERNAME_TARGET", "target-user")
        .env("GEECTL_OBS_PASSWORD_TARGET", "target-secret")
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
    assert_eq!(value["target_api_url"], "http://127.0.0.1:3000");
}

#[test]
fn copy_package_to_apiurl_same_instance_refuses() {
    // `--to-apiurl` means another instance: pointing it at the
    // configured instance would silently take the degraded
    // client-side copy instead of the same-instance server copy,
    // so it is refused with the fix (omit the flag).
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "branch",
            "copy-package",
            "--source-project",
            "test:factory",
            "--source-package",
            "foo",
            "--target-project",
            "test:target",
            "--target-package",
            "foo",
            "--to-apiurl",
            "https://api.opensuse.org",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert!(
        value["error"]["context"]
            .as_str()
            .unwrap()
            .contains("omit --to-apiurl"),
        "{}",
        value["error"]["context"]
    );
}

#[test]
fn request_supersede_without_live_refuses() {
    // The library's raw supersede has no dry-run, so without --live
    // the command refuses and points at the dry-runnable flow.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["request", "supersede", "123", "124"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    let value: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert!(
        value["error"]["context"]
            .as_str()
            .unwrap()
            .contains("request refile --supersede"),
        "stderr: {stderr}"
    );
}

#[test]
fn request_file_delete_whole_project_without_flag_refuses() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "request",
            "file-delete",
            "--target-project",
            "test:delete-me",
            "--description",
            "removing",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let value: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert!(
        value["error"]["context"]
            .as_str()
            .unwrap()
            .contains("--whole-project"),
        "stderr: {stderr}"
    );
}

#[test]
fn request_file_delete_whole_project_dry_run_with_flag() {
    // With --whole-project the dry run files nothing, returns no id,
    // and carries the plan (B3: request payloads include `plan` like
    // the other groups).
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
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
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["dry_run"], true);
    assert_eq!(value["id"], serde_json::Value::Null);
    let plan = value["plan"].as_array().expect("payload carries a plan");
    assert!(!plan.is_empty(), "plan must not be empty: {value}");
}

#[test]
fn geectl_format_env_selects_text_output() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["version"])
        .env("GEECTL_FORMAT", "text")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .starts_with("geectl "),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn geectl_format_env_invalid_fails_with_config_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["version"])
        .env("GEECTL_FORMAT", "yaml")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let value: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    assert!(
        value["error"]["context"]
            .as_str()
            .unwrap()
            .contains("GEECTL_FORMAT"),
        "stderr: {stderr}"
    );
}

#[test]
fn config_show_reports_format_from_env() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["config", "show"])
        .env("GEECTL_FORMAT", "text")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The env selects text output, so the format line is in the text
    // rendering, not a JSON field.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("format: text (environment)"), "{stdout}");
}

#[test]
fn config_show_reports_format_from_config_file() {
    let dir = TempDir::new();
    let config_dir = dir.0.join("geectl");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("config.toml"), "format = \"text\"\n").unwrap();
    let out = geectl_isolated(&dir)
        .args(["config", "show"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("format: text (config)"), "{stdout}");
}

#[test]
fn config_show_reports_obs_auth_from_alias_credentials() {
    // Alias-only credentials (no plain variables): config show
    // must report the OBS status for the effective URL, not the
    // plain variables only.
    let dir = TempDir::new();
    let config_dir = dir.0.join("geectl");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "[instances]\nmyloveinstance = \"https://api.opensuse.org\"\n",
    )
    .unwrap();
    let out = geectl_isolated(&dir)
        .args(["config", "show"])
        .env("GEECTL_OBS_USERNAME_MYLOVEINSTANCE", "target-user")
        .env("GEECTL_OBS_PASSWORD_MYLOVEINSTANCE", "target-secret")
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
    assert_eq!(value["obs_auth"]["username"], "target-user");
    assert_eq!(
        value["instances"]["myloveinstance"],
        "https://api.opensuse.org"
    );
    assert!(!stdout.contains("target-secret"));
    assert!(!String::from_utf8_lossy(&out.stderr).contains("target-secret"));
}

#[test]
fn config_show_reports_instances_table() {
    let dir = TempDir::new();
    let config_dir = dir.0.join("geectl");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "[instances]\nmyloveinstance = \"https://api.opensuse.org\"\nlocal = \"http://127.0.0.1:3001\"\n",
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
    assert_eq!(
        value["instances"]["myloveinstance"],
        "https://api.opensuse.org"
    );
    assert_eq!(value["instances"]["local"], "http://127.0.0.1:3001");
    // And in text form, as `alias -> url` pairs sorted by alias.
    let out = geectl_isolated(&dir)
        .args(["--format", "text", "config", "show"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(
            "instances: local -> http://127.0.0.1:3001, myloveinstance -> https://api.opensuse.org"
        ),
        "{stdout}"
    );
}

#[test]
fn config_show_reports_build_defaults_when_unset() {
    let dir = TempDir::new();
    // Pin XDG_CACHE_HOME: the default dir is derived from it (else
    // $HOME/.cache), and the ambient environment must not leak in.
    let out = geectl_isolated(&dir)
        .env("XDG_CACHE_HOME", &dir.0)
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
    assert_eq!(
        value["build_cache_dir"],
        dir.0.join("geectl").to_str().unwrap()
    );
    assert_eq!(value["build_cache_dir_source"], "default");
    assert_eq!(value["build_cache_cap_gib"], 20);
    assert_eq!(value["build_cache_cap_gib_source"], "default");
    // And in text form, each line states the default that applies.
    let out = geectl_isolated(&dir)
        .env("XDG_CACHE_HOME", &dir.0)
        .args(["--format", "text", "config", "show"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!(
            "build_cache_dir: {} (default)",
            dir.0.join("geectl").to_str().unwrap()
        )),
        "{stdout}"
    );
    assert!(
        stdout.contains("build_cache_cap_gib: 20 GiB (default)"),
        "{stdout}"
    );
}

#[test]
fn config_show_reports_build_values_from_config_file() {
    let dir = TempDir::new();
    let config_dir = dir.0.join("geectl");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "[build]\ncache_dir = \"/srv/geectl-cache\"\ncache_cap_gib = 40\n",
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
    assert_eq!(value["build_cache_dir"], "/srv/geectl-cache");
    assert_eq!(value["build_cache_dir_source"], "config");
    assert_eq!(value["build_cache_cap_gib"], 40);
    assert_eq!(value["build_cache_cap_gib_source"], "config");
    let out = geectl_isolated(&dir)
        .args(["--format", "text", "config", "show"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("build_cache_dir: /srv/geectl-cache (config)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("build_cache_cap_gib: 40 GiB (config)"),
        "{stdout}"
    );
}

#[test]
fn config_show_reports_build_cache_dir_from_config_file_when_cap_unset() {
    let dir = TempDir::new();
    let config_dir = dir.0.join("geectl");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "[build]\ncache_dir = \"relative/geectl-cache\"\n",
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
    // A relative configured dir is reported absolutized, the way
    // the build resolves it, not verbatim.
    let expected = std::env::current_dir()
        .unwrap()
        .join("relative/geectl-cache");
    assert_eq!(value["build_cache_dir"], expected.to_str().unwrap());
    assert_eq!(value["build_cache_dir_source"], "config");
    assert_eq!(value["build_cache_cap_gib"], 20);
    assert_eq!(value["build_cache_cap_gib_source"], "default");
    let out = geectl_isolated(&dir)
        .args(["--format", "text", "config", "show"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!(
            "build_cache_dir: {} (config)",
            expected.to_str().unwrap()
        )),
        "{stdout}"
    );
    assert!(
        stdout.contains("build_cache_cap_gib: 20 GiB (default)"),
        "{stdout}"
    );
}

#[test]
fn config_show_reports_build_cap_from_config_file_when_dir_unset() {
    let dir = TempDir::new();
    let config_dir = dir.0.join("geectl");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "[build]\ncache_cap_gib = 40\n",
    )
    .unwrap();
    let out = geectl_isolated(&dir)
        .env("XDG_CACHE_HOME", &dir.0)
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
    assert_eq!(
        value["build_cache_dir"],
        dir.0.join("geectl").to_str().unwrap()
    );
    assert_eq!(value["build_cache_dir_source"], "default");
    assert_eq!(value["build_cache_cap_gib"], 40);
    assert_eq!(value["build_cache_cap_gib_source"], "config");
    let out = geectl_isolated(&dir)
        .env("XDG_CACHE_HOME", &dir.0)
        .args(["--format", "text", "config", "show"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!(
            "build_cache_dir: {} (default)",
            dir.0.join("geectl").to_str().unwrap()
        )),
        "{stdout}"
    );
    assert!(
        stdout.contains("build_cache_cap_gib: 40 GiB (config)"),
        "{stdout}"
    );
}

#[test]
fn config_show_reports_error_source_when_default_cache_dir_unresolvable() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .env_remove("XDG_CACHE_HOME")
        .env("HOME", "")
        .args(["config", "show"])
        .output()
        .unwrap();
    // The command still succeeds: one unresolvable tunable must
    // not fail the whole diagnostic.
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert!(value["build_cache_dir"].is_null());
    assert_eq!(value["build_cache_dir_source"], "error");
    assert_eq!(value["build_cache_cap_gib"], 20);
    assert_eq!(value["build_cache_cap_gib_source"], "default");
    let out = geectl_isolated(&dir)
        .env_remove("XDG_CACHE_HOME")
        .env("HOME", "")
        .args(["--format", "text", "config", "show"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("build_cache_dir: - (error)"), "{stdout}");
}
