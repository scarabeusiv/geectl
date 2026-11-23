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

/// The binary must run under a 1 MiB main-thread stack: the Windows
/// main thread carries 1 MiB, and the unoptimized CLI parse needs
/// more than that. The limit is set in a shell wrapper so the test
/// needs no unsafe code.
#[test]
#[cfg(unix)]
fn version_flag_exits_zero_under_one_mib_stack() {
    let out = Command::new("sh")
        .args([
            "-c",
            "ulimit -s 1024; exec \"$@\"",
            "sh",
            env!("CARGO_BIN_EXE_geectl"),
            "--version",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("geectl"));
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
        "download",
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
        "upload-file",
        "create-project",
        "create-package",
        "delete-package",
        "delete-project",
        "undelete-package",
        "undelete-project",
        "aggregate-package",
        "release-package",
        "release-project",
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
fn remote_download_help_names_the_rev_and_output_flags() {
    let out = geectl()
        .args(["remote", "download", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--rev"));
    assert!(stdout.contains("--output"));
    assert!(stdout.contains("project"));
    assert!(stdout.contains("package"));
    assert!(stdout.contains("file"));
}

#[test]
fn remote_download_without_output_fails_with_parse_error() {
    let out = geectl()
        .args([
            "remote",
            "download",
            "openSUSE:Factory",
            "foo",
            "foo-1.0.tar.gz",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(2));
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
fn remote_invalid_list_files_rev_fails_fast_without_network() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "list-files",
            "openSUSE:Factory",
            "vim",
            "--rev",
            "not-a-rev",
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
fn remote_package_log_limit_returns_only_the_latest_entry() {
    let addr = serve_once(
        "<revisionlist>\
        <revision rev=\"1\" vrev=\"1\">\
        <srcmd5>aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa</srcmd5>\
        <version>1.0</version><time>1700000000</time><user>Admin</user>\
        <comment>Initial import</comment>\
        </revision>\
        <revision rev=\"2\" vrev=\"1\">\
        <srcmd5>bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb</srcmd5>\
        <version>1.1</version><time>1700000001</time><user>Admin</user>\
        <comment>Update to 1.1</comment>\
        </revision>\
        <revision rev=\"3\" vrev=\"1\">\
        <srcmd5>cccccccccccccccccccccccccccccccc</srcmd5>\
        <version>1.2</version><time>1700000002</time><user>Admin</user>\
        <comment>Update to 1.2</comment><requestid>123</requestid>\
        </revision>\
        </revisionlist>",
    );
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "package-log",
            "openSUSE:Factory",
            "foo",
            "--limit",
            "1",
        ])
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
    assert_eq!(value["limit"].as_u64(), Some(1));
    assert_eq!(
        value["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["rev"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![3]
    );
}

#[test]
fn remote_package_log_limit_zero_fails_fast_without_network() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "package-log",
            "openSUSE:Factory",
            "vim",
            "--limit",
            "0",
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
/// its success path offline. The thread is not joined:
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

/// A one-shot HTTP fixture serving raw bytes: the same
/// single-connection shape as `serve_once`, for downloads whose body
/// is not valid UTF-8.
fn serve_once_bytes(body: &[u8]) -> std::net::SocketAddr {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let body = body.to_vec();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.write_all(&body);
        let _ = stream.flush();
    });
    addr
}

/// A many-shot HTTP fixture: serves the same canned response to
/// every connection, until the test process ends. For commands that
/// fan out (a diff lists both sides, then reads files), where one
/// canned body answers every request. The thread is not joined,
/// like `serve_once`: if the binary never connects, the
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
fn remote_list_files_rev_carries_rev_in_the_payload_and_keeps_the_text_table() {
    // The fixture answers the one request whatever its query is; the
    // library unit tests pin `?rev=` on the wire. This pins the CLI
    // half: the payload carries the parsed revision — a number as a
    // number, an md5 as a string — and the text stays the design
    // table under the coordinate header.
    let body = "<directory><entry name=\"vim.spec\" size=\"1234\" mtime=\"1700000000\" md5=\"deadbeef\"/></directory>";
    let dir = TempDir::new();
    let addr = serve_once(body);
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "list-files",
            "openSUSE:Factory",
            "vim",
            "--rev",
            "2",
        ])
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
    assert_eq!(value["rev"].as_u64(), Some(2));
    assert_eq!(value["files"][0]["name"], "vim.spec");
    let addr = serve_once(body);
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "list-files",
            "openSUSE:Factory",
            "vim",
            "--rev",
            "0123456789abcdef0123456789abcdef",
        ])
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
    assert_eq!(
        value["rev"].as_str(),
        Some("0123456789abcdef0123456789abcdef")
    );
    let addr = serve_once(body);
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "remote",
            "list-files",
            "openSUSE:Factory",
            "vim",
            "--rev",
            "2",
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
    assert_eq!(
        stdout.lines().next(),
        Some("project: openSUSE:Factory package: vim"),
        "{stdout}"
    );
    assert!(stdout.contains("vim.spec"), "{stdout}");
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
fn remote_download_writes_binary_bytes_to_disk() {
    // A binary body (NULs, non-UTF-8) downloads to --output and the
    // payload names the file instead of carrying the bytes.
    let bytes: &[u8] = &[0xED, 0xAB, 0x00, 0xEE, 0xDB, 0xFF];
    let addr = serve_once_bytes(bytes);
    let dir = TempDir::new();
    let output = dir.0.join("foo-1.0.tar.gz");
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "download",
            "openSUSE:Factory",
            "foo",
            "foo-1.0.tar.gz",
            "--output",
            output.to_str().unwrap(),
        ])
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
    assert_eq!(value["package"], "foo");
    assert_eq!(value["file"], "foo-1.0.tar.gz");
    assert_eq!(value["rev"], serde_json::Value::Null);
    assert_eq!(value["path"], output.to_str().unwrap());
    assert_eq!(value["size_bytes"], 6);
    assert_eq!(std::fs::read(&output).unwrap(), bytes);
}

#[test]
fn remote_download_text_mode_renders_the_confirmation_line() {
    let bytes: &[u8] = &[1, 2, 3];
    let addr = serve_once_bytes(bytes);
    let dir = TempDir::new();
    let output = dir.0.join("foo-1.0.tar.gz");
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "remote",
            "download",
            "openSUSE:Factory",
            "foo",
            "foo-1.0.tar.gz",
            "--output",
            output.to_str().unwrap(),
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
    assert_eq!(
        stdout.trim(),
        format!(
            "project: openSUSE:Factory package: foo file: foo-1.0.tar.gz\ndownloaded foo-1.0.tar.gz (3 bytes) to {}",
            output.display()
        )
    );
    assert_eq!(std::fs::read(&output).unwrap(), bytes);
}

#[test]
fn remote_download_rev_forms_are_accepted() {
    // Both rev forms reach the server: the exact `?rev=` value is
    // pinned at the library level; here the CLI accepts the forms
    // and the payload echoes the normalized spec.
    for rev in ["2", "aabbccddaabbccddaabbccddaabbccdd"] {
        let addr = serve_once_bytes(&[1]);
        let dir = TempDir::new();
        let output = dir.0.join("foo-1.0.tar.gz");
        let out = geectl_isolated(&dir)
            .args([
                "remote",
                "download",
                "openSUSE:Factory",
                "foo",
                "foo-1.0.tar.gz",
                "--rev",
                rev,
                "--output",
                output.to_str().unwrap(),
            ])
            .env("GEECTL_OBS_URL", format!("http://{addr}"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "rev {rev}: stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let value: serde_json::Value =
            serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
        assert_eq!(value["rev"], rev);
        assert_eq!(std::fs::read(&output).unwrap(), vec![1]);
    }
}

#[test]
fn remote_download_invalid_rev_fails_fast_without_network() {
    let dir = TempDir::new();
    let output = dir.0.join("foo-1.0.tar.gz");
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "download",
            "openSUSE:Factory",
            "foo",
            "foo-1.0.tar.gz",
            "--rev",
            "not-a-rev",
            "--output",
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert!(!output.exists());
}

#[test]
fn remote_download_missing_file_is_an_error() {
    let addr = serve_once_status(404, "Not Found", "");
    let dir = TempDir::new();
    let output = dir.0.join("nope.tar.gz");
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "download",
            "openSUSE:Factory",
            "foo",
            "nope.tar.gz",
            "--output",
            output.to_str().unwrap(),
        ])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "not_found");
    assert!(!output.exists());
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
    // a half-set env.
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

#[test]
fn remote_list_staging_projects_with_half_set_credentials_fails_with_config_error() {
    // The staging read needs credentials, so it fails fast like the
    // other credentialed reads.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "list-staging-projects", "openSUSE:Factory"])
        .env("GEECTL_OBS_USERNAME", "alice")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
}

#[test]
fn remote_staging_project_with_half_set_credentials_fails_with_config_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "staging-project", "openSUSE:Factory", "A"])
        .env("GEECTL_OBS_USERNAME", "alice")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
}

#[test]
fn remote_search_projects_by_person_with_half_set_credentials_fails_with_config_error() {
    // The person search needs credentials, so it fails fast like the
    // other credentialed reads.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "search-projects-by-person", "alice"])
        .env("GEECTL_OBS_USERNAME", "alice")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
}

#[test]
fn remote_count_scmsync_projects_with_half_set_credentials_fails_with_config_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "count-scmsync-projects",
            "https://github.com/openSUSE/",
        ])
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
fn remote_set_package_meta_devel_package_alone_fails_in_dry_run() {
    // `--devel-package` without `--devel-project` needs a devel
    // project already set on the package; dry-run never reads the
    // current meta, so the pair fails before any plan is printed.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-package-meta",
            "P",
            "K",
            "--devel-package",
            "other",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert_eq!(
        value["error"]["context"],
        "--devel-package needs --devel-project or a devel project already set on the package"
    );
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
fn remote_upload_file_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let from = dir.0.join("vim.spec");
    std::fs::write(&from, "Name: vim\n").unwrap();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "upload-file",
            "openSUSE:Factory",
            "vim",
            "vim.spec",
            "--from",
        ])
        .arg(&from)
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
    assert_eq!(value["project"], "openSUSE:Factory");
    assert_eq!(value["package"], "vim");
    assert_eq!(value["file"], "vim.spec");
    assert_eq!(value["rev"], serde_json::Value::Null);
}

#[test]
fn remote_create_project_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "create-project",
            "home:alice",
            "--title",
            "Alice",
            "--description",
            "Alice's project",
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
    assert_eq!(value["project"], "home:alice");
    assert_eq!(value["title"], "Alice");
}

#[test]
fn remote_create_project_live_with_half_set_credentials_fails_with_config_error() {
    // A live mutation does need credentials, so it fails fast.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "remote",
            "create-project",
            "home:alice",
            "--title",
            "Alice",
            "--description",
            "Alice's project",
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
fn remote_create_package_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all. Both
    // creation forms are covered: the direct form and the options
    // form (`--scmsync` selects it).
    let dir = TempDir::new();
    for args in [
        vec![
            "remote",
            "create-package",
            "home:alice",
            "vim",
            "--title",
            "Vi",
            "--description",
            "Vi IMproved",
        ],
        vec![
            "remote",
            "create-package",
            "home:alice",
            "vim",
            "--title",
            "Vi",
            "--description",
            "Vi IMproved",
            "--scmsync",
            "https://example.com/vim#main",
            "--force",
        ],
    ] {
        let out = geectl_isolated(&dir)
            .args(&args)
            .env("GEECTL_OBS_USERNAME", "alice")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "args {args:?} stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let value: serde_json::Value =
            serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
        assert_eq!(value["dry_run"], true);
        assert_eq!(value["project"], "home:alice");
        assert_eq!(value["package"], "vim");
    }
}

#[test]
fn remote_delete_package_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "delete-package", "home:alice", "vim", "--force"])
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
    assert_eq!(value["project"], "home:alice");
    assert_eq!(value["package"], "vim");
    assert_eq!(value["force"], true);
}

#[test]
fn remote_delete_link_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "delete-link", "home:alice", "vim"])
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
    assert_eq!(value["project"], "home:alice");
    assert_eq!(value["package"], "vim");
    assert_eq!(
        value["plan"][0],
        "[dry-run] would: delete link of home:alice/vim"
    );
}

#[test]
fn remote_delete_link_live_with_half_set_credentials_fails_with_config_error() {
    // A live mutation does need credentials, so it fails fast.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "remote", "delete-link", "home:alice", "vim"])
        .env("GEECTL_OBS_USERNAME", "alice")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
}

#[test]
fn remote_set_project_meta_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-project-meta",
            "home:alice",
            "--title",
            "Alice",
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
    assert_eq!(value["name"], "home:alice");
    assert_eq!(value["title"], "Alice");
    // Unmentioned fields are absent from the dry-run payload, not cleared.
    assert!(value.get("description").is_none());
    assert!(value.get("repositories").is_none());
    assert_eq!(
        value["plan"][0],
        "[dry-run] would: set project meta of home:alice"
    );
}

#[test]
fn remote_set_project_meta_live_with_half_set_credentials_fails_with_config_error() {
    // A live mutation does need credentials, so it fails fast.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "remote",
            "set-project-meta",
            "home:alice",
            "--title",
            "Alice",
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
fn remote_set_package_meta_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-package-meta",
            "home:alice",
            "vim",
            "--title",
            "Vim",
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
    assert_eq!(value["project"], "home:alice");
    assert_eq!(value["name"], "vim");
    assert_eq!(value["title"], "Vim");
    // Unmentioned fields are absent from the dry-run payload, not cleared.
    assert!(value.get("description").is_none());
    assert!(value.get("persons").is_none());
    assert_eq!(
        value["plan"][0],
        "[dry-run] would: set package meta of home:alice/vim"
    );
}

#[test]
fn remote_set_package_meta_live_with_half_set_credentials_fails_with_config_error() {
    // A live mutation does need credentials, so it fails fast.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "remote",
            "set-package-meta",
            "home:alice",
            "vim",
            "--title",
            "Vim",
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
fn remote_update_package_meta_from_spec_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "update-package-meta-from-spec",
            "home:alice",
            "vim",
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
    assert_eq!(value["project"], "home:alice");
    assert_eq!(value["package"], "vim");
    assert_eq!(
        value["plan"][0],
        "[dry-run] would: update package meta of home:alice/vim from spec"
    );
}

#[test]
fn remote_update_package_meta_from_spec_live_with_half_set_credentials_fails_with_config_error() {
    // A live mutation does need credentials, so it fails fast.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "remote",
            "update-package-meta-from-spec",
            "home:alice",
            "vim",
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
fn remote_delete_project_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "delete-project",
            "home:alice",
            "--recursive",
            "--force",
            "--comment",
            "cleanup",
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
    assert_eq!(value["project"], "home:alice");
    assert_eq!(value["recursive"], true);
    assert_eq!(value["force"], true);
    assert_eq!(value["comment"], "cleanup");
}

#[test]
fn remote_undelete_package_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "undelete-package", "home:alice", "vim"])
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
    assert_eq!(value["project"], "home:alice");
    assert_eq!(value["package"], "vim");
}

#[test]
fn remote_undelete_project_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "undelete-project", "home:alice"])
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
    assert_eq!(value["project"], "home:alice");
}

#[test]
fn remote_aggregate_package_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "aggregate-package",
            "openSUSE:Factory",
            "vim",
            "home:alice",
            "vim",
            "--repository-map",
            "standard=standard",
            "--no-sources",
            "--disable-publish",
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
    assert_eq!(value["source_project"], "openSUSE:Factory");
    assert_eq!(value["source_package"], "vim");
    assert_eq!(value["target_project"], "home:alice");
    assert_eq!(value["target_package"], "vim");
    assert_eq!(value["repository_map"][0]["source"], "standard");
    assert_eq!(value["no_sources"], true);
    assert_eq!(value["disable_publish"], true);
}

#[test]
fn remote_release_package_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "release-package",
            "home:alice",
            "vim",
            "--repository",
            "standard",
            "--arch",
            "x86_64",
            "--target-project",
            "openSUSE:Factory",
            "--target-repository",
            "standard",
            "--set-release",
            "1.0",
            "--no-delay",
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
    assert_eq!(value["project"], "home:alice");
    assert_eq!(value["package"], "vim");
    assert_eq!(value["repository"], "standard");
    assert_eq!(value["architecture"], "x86_64");
    assert_eq!(value["target_project"], "openSUSE:Factory");
    assert_eq!(value["target_repository"], "standard");
    assert_eq!(value["set_release"], "1.0");
    assert_eq!(value["no_delay"], true);
}

#[test]
fn remote_release_project_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "release-project",
            "home:alice",
            "--repository",
            "standard",
            "--arch",
            "x86_64",
            "--target-project",
            "openSUSE:Factory",
            "--target-repository",
            "standard",
            "--set-release",
            "1.0",
            "--no-delay",
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
    assert_eq!(value["project"], "home:alice");
    assert_eq!(value["repository"], "standard");
    assert_eq!(value["architecture"], "x86_64");
    assert_eq!(value["target_project"], "openSUSE:Factory");
    assert_eq!(value["target_repository"], "standard");
    assert_eq!(value["set_release"], "1.0");
    assert_eq!(value["no_delay"], true);
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

// --- remote mutation CLI pins (issue #129) ---
//
// The live-creds failure and the invalid-input rejections,
// parametrized across the remote mutation leaves. Credentials resolve
// before the command runs, and the input checks run before any
// network, so none of these needs a server.

/// A live remote mutation with half-set credentials fails fast with
/// a config error (exit 9). The context names the half-set pair,
/// which only the CLI-level fail-fast produces: the library's own
/// credentials check reports a missing-credentials message instead.
/// This pins the fail-fast layer, not just the code.
fn remote_live_mutation_with_half_set_credentials_fails(args: &[String]) {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "remote"])
        .args(args)
        .env("GEECTL_OBS_USERNAME", "alice")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9), "args: {args:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).is_empty(),
        "args: {args:?}"
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    assert!(
        value["error"]["context"]
            .as_str()
            .unwrap()
            .contains("is set but no password source is"),
        "context: {}",
        value["error"]["context"]
    );
}

#[test]
fn remote_live_mutations_with_half_set_credentials_fail_with_config_error() {
    // The create-project, repo-add and repo-remove leaves already
    // have their own live-creds tests above; this loop pins the
    // remaining live mutations the same way.
    let dir = TempDir::new();
    let from = dir.0.join("vim.spec");
    std::fs::write(&from, "Name: vim\n").unwrap();
    let from = from.to_string_lossy().into_owned();
    let cases: Vec<Vec<String>> = vec![
        vec![
            "upload-file".into(),
            "openSUSE:Factory".into(),
            "vim".into(),
            "vim.spec".into(),
            "--from".into(),
            from,
        ],
        vec![
            "create-package".into(),
            "home:alice".into(),
            "vim".into(),
            "--title".into(),
            "Vi".into(),
            "--description".into(),
            "Vi IMproved".into(),
        ],
        vec![
            "delete-package".into(),
            "home:alice".into(),
            "vim".into(),
            "--force".into(),
        ],
        vec![
            "delete-project".into(),
            "home:alice".into(),
            "--recursive".into(),
            "--force".into(),
            "--comment".into(),
            "cleanup".into(),
        ],
        vec!["undelete-package".into(), "home:alice".into(), "vim".into()],
        vec!["undelete-project".into(), "home:alice".into()],
        vec![
            "aggregate-package".into(),
            "openSUSE:Factory".into(),
            "vim".into(),
            "home:alice".into(),
            "vim".into(),
            "--repository-map".into(),
            "standard=standard".into(),
            "--no-sources".into(),
            "--disable-publish".into(),
        ],
        vec![
            "release-package".into(),
            "home:alice".into(),
            "vim".into(),
            "--repository".into(),
            "standard".into(),
            "--arch".into(),
            "x86_64".into(),
            "--target-project".into(),
            "openSUSE:Factory".into(),
            "--target-repository".into(),
            "standard".into(),
            "--set-release".into(),
            "1.0".into(),
            "--no-delay".into(),
        ],
        vec![
            "release-project".into(),
            "home:alice".into(),
            "--repository".into(),
            "standard".into(),
            "--arch".into(),
            "x86_64".into(),
            "--target-project".into(),
            "openSUSE:Factory".into(),
            "--target-repository".into(),
            "standard".into(),
            "--set-release".into(),
            "1.0".into(),
            "--no-delay".into(),
        ],
    ];
    for args in cases {
        remote_live_mutation_with_half_set_credentials_fails(&args);
    }
}

/// A remote mutation with invalid input fails fast (exit 2) with a
/// structured `invalid_input` error on stderr and empty stdout.
fn remote_mutation_invalid_input_fails(args: &[String], context: &str) {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote"])
        .args(args)
        .output()
        .unwrap();
    assert!(!out.status.success(), "args: {args:?}");
    assert_eq!(out.status.code(), Some(2), "args: {args:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).is_empty(),
        "args: {args:?}"
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert!(
        value["error"]["context"]
            .as_str()
            .unwrap()
            .contains(context),
        "context: {}",
        value["error"]["context"]
    );
}

#[test]
fn remote_upload_file_missing_from_fails_with_invalid_input() {
    let dir = TempDir::new();
    let missing = dir.0.join("missing.spec");
    // The operating system text for a missing file differs per
    // platform; derive it from the platform.
    let context = format!(
        "cannot read {}: {}",
        missing.display(),
        std::io::Error::from_raw_os_error(2)
    );
    remote_mutation_invalid_input_fails(
        &[
            "upload-file".to_string(),
            "openSUSE:Factory".to_string(),
            "vim".to_string(),
            "vim.spec".to_string(),
            "--from".to_string(),
            missing.to_string_lossy().into_owned(),
        ],
        &context,
    );
}

#[test]
fn remote_upload_file_non_utf8_from_fails_with_invalid_input() {
    let dir = TempDir::new();
    let bad = dir.0.join("bad.spec");
    std::fs::write(&bad, [0xff, 0xfe, b'b', b'a', b'd']).unwrap();
    let context = format!(
        "cannot read {}: stream did not contain valid UTF-8",
        bad.display()
    );
    remote_mutation_invalid_input_fails(
        &[
            "upload-file".to_string(),
            "openSUSE:Factory".to_string(),
            "vim".to_string(),
            "vim.spec".to_string(),
            "--from".to_string(),
            bad.to_string_lossy().into_owned(),
        ],
        &context,
    );
}

#[test]
fn remote_upload_file_non_utf8_from_binary_dry_run_succeeds() {
    // The binary upload takes bytes, so the same file the text
    // upload rejects uploads in dry-run.
    let dir = TempDir::new();
    let bad = dir.0.join("bad.spec");
    std::fs::write(&bad, [0xff, 0xfe, b'b', b'a', b'd']).unwrap();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "upload-file",
            "openSUSE:Factory",
            "vim",
            "vim.spec",
            "--binary",
            "--from",
        ])
        .arg(&bad)
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
    assert_eq!(value["rev"], serde_json::Value::Null);
}

#[test]
fn remote_repo_add_bad_path_fails_with_invalid_input() {
    // `--path` is the `PROJECT/REPOSITORY` coordinate: a value with
    // no `/`, or an empty side, is an input error naming the form.
    for path in ["no-slash-here", "openSUSE:Factory/", "/standard"] {
        let context = format!("invalid repository path {path:?}: expected PROJECT/REPOSITORY");
        remote_mutation_invalid_input_fails(
            &[
                "repo-add".to_string(),
                "openSUSE:Factory".to_string(),
                "standard".to_string(),
                "--arch".to_string(),
                "x86_64".to_string(),
                "--path".to_string(),
                path.to_string(),
            ],
            &context,
        );
    }
}

#[test]
fn remote_aggregate_package_bad_repository_map_fails_with_invalid_input() {
    for mapping in ["no-equals", "=target", "source="] {
        let context = format!("invalid repository mapping {mapping:?}: expected SOURCE=TARGET");
        remote_mutation_invalid_input_fails(
            &[
                "aggregate-package".to_string(),
                "openSUSE:Factory".to_string(),
                "vim".to_string(),
                "home:alice".to_string(),
                "vim".to_string(),
                "--repository-map".to_string(),
                mapping.to_string(),
            ],
            &context,
        );
    }
}

#[test]
fn remote_release_target_repository_without_target_project_fails_with_invalid_input() {
    // Both release leaves validate the pair the same way.
    let context = "releasing to a target repository needs a target project";
    remote_mutation_invalid_input_fails(
        &[
            "release-package".to_string(),
            "home:alice".to_string(),
            "vim".to_string(),
            "--target-repository".to_string(),
            "standard".to_string(),
        ],
        context,
    );
    remote_mutation_invalid_input_fails(
        &[
            "release-project".to_string(),
            "home:alice".to_string(),
            "--target-repository".to_string(),
            "standard".to_string(),
        ],
        context,
    );
    // The second branch of the same validation: a target project
    // needs a repository and a target repository.
    remote_mutation_invalid_input_fails(
        &[
            "release-package".to_string(),
            "home:alice".to_string(),
            "vim".to_string(),
            "--target-project".to_string(),
            "openSUSE:Factory".to_string(),
        ],
        "releasing to a target project needs a repository and a target repository",
    );
}

#[test]
fn remote_mutation_bad_project_name_fails_with_invalid_input() {
    // The library validates names before the dry-run guard in every
    // mutation leaf; aggregate-package pins it here.
    remote_mutation_invalid_input_fails(
        &[
            "aggregate-package".to_string(),
            "!!bad".to_string(),
            "vim".to_string(),
            "home:alice".to_string(),
            "vim".to_string(),
        ],
        "invalid OBS project name: \"!!bad\"",
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
fn build_help_lists_all_sixteen_subcommands() {
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
        "buildconfig",
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
fn pull_request_help_lists_the_subcommands() {
    let out = geectl().args(["pull-request", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    for name in [
        "show",
        "search",
        "approve",
        "request-changes",
        "comment",
        "comments",
        "cancel-review-request",
        "merge",
        "decline",
        "withdraw",
        "reopen",
    ] {
        assert!(text.contains(name), "pull-request --help misses {name}");
    }
}

#[test]
fn top_help_lists_the_pull_request_group() {
    let out = geectl().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("pull-request"),
        "top --help misses the pull-request group"
    );
}

#[test]
fn pull_request_merge_obs_dry_run_without_credentials_prints_plan() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["pull-request", "merge", "obs:123"])
        .output()
        .unwrap();
    // A dry-run level act on OBS prints without credentials: the
    // plan names no acting user.
    assert!(out.status.success());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["reference"], "obs:123");
    assert_eq!(value["status"], serde_json::Value::Null);
    assert_eq!(value["dry_run"], true);
    assert_eq!(
        value["plan"][0],
        "[dry-run] would: change state of submit request 123 to accepted"
    );
}

#[test]
fn pull_request_merge_obs_text_dry_run_without_credentials_prints_plan() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--format", "text", "pull-request", "merge", "obs:123"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "dry-run: would merge obs:123"
    );
}

#[test]
fn pull_request_merge_obs_live_without_credentials_fails_with_config_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "pull-request", "merge", "obs:123"])
        .output()
        .unwrap();
    // ConfigError exits 9; a live merge needs OBS Basic
    // credentials and fails loudly without them.
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
fn pull_request_merge_gitea_without_credentials_fails_with_config_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["pull-request", "merge", "gitea:openSUSE/geectl#122"])
        .output()
        .unwrap();
    // ConfigError exits 9; a Gitea act needs the Bearer <redacted>
    // even in dry-run and fails loudly without it.
    assert_eq!(out.status.code(), Some(9));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    assert!(value["error"]["context"]
        .as_str()
        .unwrap()
        .contains("GEECTL_GITEA_TOKEN"));
}

#[test]
fn pull_request_merge_obs_with_a_method_is_invalid_input() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["pull-request", "merge", "obs:123", "--method", "squash"])
        .output()
        .unwrap();
    // InvalidInput exits 2; --method does not apply to OBS.
    // The refusal precedes credential resolution, so the dry-run
    // credential warning never appears; the assertions are
    // substrings, not a JSON parse.
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("invalid_input"), "stderr: {stderr}");
    assert!(stderr.contains("--method squash"), "stderr: {stderr}");
}

#[test]
fn pull_request_merge_gitea_with_force_is_invalid_input_without_credentials() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "pull-request",
            "merge",
            "gitea:openSUSE/geectl#122",
            "--force",
        ])
        .output()
        .unwrap();
    // InvalidInput exits 2; --force does not apply to Gitea, and
    // the refusal precedes credential resolution, so it fires
    // without a token where a plain merge would fail with
    // ConfigError.
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("invalid_input"), "stderr: {stderr}");
    assert!(stderr.contains("--force"), "stderr: {stderr}");
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
fn pull_request_show_obs_without_credentials_fails_with_config_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["pull-request", "show", "obs:123"])
        .output()
        .unwrap();
    // ConfigError exits 9; an `obs:` read needs OBS Basic
    // credentials and fails loudly without them.
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
fn pull_request_show_gitea_without_credentials_fails_with_config_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["pull-request", "show", "gitea:openSUSE/geectl#122"])
        .output()
        .unwrap();
    // ConfigError exits 9; a `gitea:` read needs the Bearer token
    // and fails loudly without it.
    assert_eq!(out.status.code(), Some(9));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    assert!(value["error"]["context"]
        .as_str()
        .unwrap()
        .contains("GEECTL_GITEA_TOKEN"));
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
fn build_log_last_and_lastsucceeded_conflict() {
    // The two selectors name different logs: clap refuses the
    // combination as a parse error (exit 2) before any work runs.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "build",
            "build-log",
            "openSUSE:Factory",
            "foo",
            "standard",
            "x86_64",
            "--last",
            "--lastsucceeded",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("lastsucceeded"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
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
fn build_download_binary_refuses_an_existing_destination_before_downloading() {
    // The refusal precedes the download: the output already exists
    // and the server is unreachable, so a download-first order would
    // fail with a transport error instead of AlreadyExists.
    let dir = TempDir::new();
    let output = dir.0.join("foo-1.0-1.x86_64.rpm");
    fs::write(&output, b"existing").unwrap();
    let out = geectl_isolated(&dir)
        .args([
            "build",
            "download-binary",
            "openSUSE:Factory",
            "foo",
            "standard",
            "x86_64",
            "foo-1.0-1.x86_64.rpm",
            "--output",
            output.to_str().unwrap(),
        ])
        .env("GEECTL_OBS_URL", "http://127.0.0.1:1")
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(10));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "already_exists");
    assert_eq!(fs::read(&output).unwrap(), b"existing");
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
fn build_rebuild_restart_abort_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    for (leaf, plan) in [
        (
            "rebuild-package",
            "[dry-run] would: rebuild package openSUSE:Factory/vim (standard/x86_64)",
        ),
        (
            "restart-build",
            "[dry-run] would: restart the build of openSUSE:Factory/vim (standard/x86_64)",
        ),
        (
            "abort-build",
            "[dry-run] would: abort the build of openSUSE:Factory/vim (standard/x86_64)",
        ),
    ] {
        let out = geectl_isolated(&dir)
            .args([
                "build",
                leaf,
                "openSUSE:Factory",
                "vim",
                "standard",
                "x86_64",
            ])
            .env("GEECTL_OBS_USERNAME", "alice")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "leaf {leaf} stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let value: serde_json::Value =
            serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
        assert_eq!(value["dry_run"], true);
        assert_eq!(value["project"], "openSUSE:Factory");
        assert_eq!(value["package"], "vim");
        assert_eq!(value["repository"], "standard");
        assert_eq!(value["arch"], "x86_64");
        assert_eq!(value["plan"][0], plan);
    }
}

#[test]
fn build_rebuild_package_live_with_half_set_credentials_fails_with_config_error() {
    // A live mutation does need credentials, so it fails fast.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "build",
            "rebuild-package",
            "openSUSE:Factory",
            "vim",
            "standard",
            "x86_64",
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
fn build_restart_build_live_with_half_set_credentials_fails_with_config_error() {
    // A live mutation does need credentials, so it fails fast.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "build",
            "restart-build",
            "openSUSE:Factory",
            "vim",
            "standard",
            "x86_64",
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
fn build_abort_build_live_with_half_set_credentials_fails_with_config_error() {
    // A live mutation does need credentials, so it fails fast.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "build",
            "abort-build",
            "openSUSE:Factory",
            "vim",
            "standard",
            "x86_64",
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
fn build_results_with_half_set_credentials_fails_with_config_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["build", "results", "openSUSE:Factory"])
        .env("GEECTL_OBS_USERNAME", "alice")
        .output()
        .unwrap();
    // The credentialed reads resolve credentials eagerly and fail loud
    // on a half-set env.
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
    // The docs carry 20 blocks (top level, 9 groups, 10 leaves) — a
    // sampled surface, not every leaf: a leaf with no docs block can
    // change --help undetected. That is deliberate — the docs show the
    // groups and one leaf each, plus `remote download` — and the count
    // assertion guards the docs side only.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let docs = fs::read_to_string(manifest.join("docs/cli.md")).expect("docs/cli.md");
    let blocks = help_blocks(&docs);
    assert_eq!(
        blocks.len(),
        20,
        "docs/cli.md should carry 20 --help blocks (top level, 9 groups, 10 leaves);          update this count when a block is deliberately added or removed"
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
    // The binary absolutizes the configured dir with
    // `std::path::absolute`, which on Windows adds the current drive
    // and normalizes the separators; the expectation uses the same call.
    let expected = std::path::absolute("/srv/geectl-cache")
        .unwrap_or_else(|_| std::path::PathBuf::from("/srv/geectl-cache"));
    assert_eq!(value["build_cache_dir"], expected.to_str().unwrap());
    assert_eq!(value["build_cache_dir_source"], "config");
    assert_eq!(value["build_cache_cap_gib"], 40);
    assert_eq!(value["build_cache_cap_gib_source"], "config");
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
    // the build resolves it, not verbatim. The expectation uses the
    // same `std::path::absolute` call the binary uses, so the
    // separators match on every platform.
    let expected = std::path::absolute("relative/geectl-cache")
        .unwrap_or_else(|_| std::path::PathBuf::from("relative/geectl-cache"));
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
    // USERPROFILE is the Windows home fallback: without clearing
    // it the cache dir resolves on Windows and the error is not
    // exercised.
    let out = geectl_isolated(&dir)
        .env_remove("XDG_CACHE_HOME")
        .env_remove("USERPROFILE")
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
        .env_remove("USERPROFILE")
        .env("HOME", "")
        .args(["--format", "text", "config", "show"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("build_cache_dir: - (error)"), "{stdout}");
}
