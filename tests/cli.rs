//! Integration tests: the CLI as a binary (help, version, output shape).
//!
//! These run the built binary via `cargo test` (using `CARGO_BIN_EXE_geectl`,
//! set by Cargo for integration tests). They assert the machine contract:
//! `--help` and `--version` exit 0, the version payload is valid JSON on
//! stdout, and errors go to stderr with a structured shape.

use geectl::changelog::changes::{parse_changes, parse_header};
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
        .env_remove("GEECTL_DOTENV")
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

/// Write a `.env` file into the temp dir (0600 on unix, so the mode
/// check passes) and return its path.
fn write_dotenv(dir: &TempDir, content: &str) -> PathBuf {
    let path = dir.0.join(".env");
    fs::write(&path, content).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    path
}

#[test]
fn config_show_reports_dotenv_source() {
    let dir = TempDir::new();
    write_dotenv(&dir, "GEECTL_OBS_URL=https://obs.example.com\n");
    let out = geectl_isolated(&dir)
        .env("GEECTL_DOTENV", "1")
        .args(["config", "show"])
        .current_dir(&dir.0)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    // A value from the file resolves like the equivalent environment
    // variable, and reports the same source.
    assert_eq!(value["obs_url"], "https://obs.example.com");
    assert_eq!(value["obs_url_source"], "environment");
}

#[test]
fn dotenv_credentials_resolve_like_environment_variables() {
    let dir = TempDir::new();
    write_dotenv(
        &dir,
        "GEECTL_OBS_USERNAME=alice\nGEECTL_OBS_PASSWORD=s3cr3t\n",
    );
    let out = geectl_isolated(&dir)
        .env("GEECTL_DOTENV", "1")
        .args(["config", "show"])
        .current_dir(&dir.0)
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
fn real_environment_overrides_dotenv() {
    let dir = TempDir::new();
    write_dotenv(&dir, "GEECTL_OBS_URL=https://file.example.com\n");
    let out = geectl_isolated(&dir)
        .env("GEECTL_DOTENV", "1")
        .args(["config", "show"])
        .current_dir(&dir.0)
        .env("GEECTL_OBS_URL", "https://real.example.com")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["obs_url"], "https://real.example.com");
    assert_eq!(value["obs_url_source"], "environment");
}

#[test]
fn dotenv_malformed_file_fails_fast() {
    let dir = TempDir::new();
    write_dotenv(&dir, "GEECTL_OBS_PASSWORD=s3cr3t-value\nBROKEN LINE\n");
    let out = geectl_isolated(&dir)
        .env("GEECTL_DOTENV", "1")
        .arg("version")
        .current_dir(&dir.0)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    let value: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    let context = value["error"]["context"].as_str().unwrap();
    assert!(context.contains(".env"), "{context}");
    assert!(context.contains("line 2"), "{context}");
    assert!(!stderr.contains("s3cr3t-value"));
}

#[test]
#[cfg(unix)]
fn dotenv_group_readable_file_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new();
    let path = write_dotenv(&dir, "GEECTL_OBS_PASSWORD=s3cr3t-value\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let out = geectl_isolated(&dir)
        .env("GEECTL_DOTENV", "1")
        .arg("version")
        .current_dir(&dir.0)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    let value: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    let context = value["error"]["context"].as_str().unwrap();
    assert!(context.contains("readable by group or other"), "{context}");
    assert!(context.contains("chmod 600"), "{context}");
    assert!(!stderr.contains("s3cr3t-value"));
}

#[test]
fn dotenv_without_opt_in_is_ignored() {
    let dir = TempDir::new();
    // A 0644 file carrying geectl keys — and a malformed line — must
    // not fail a run that never opted into `.env` loading.
    let path = write_dotenv(
        &dir,
        "GEECTL_OBS_URL=https://file.example.com\nBROKEN LINE\n",
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    }
    let out = geectl_isolated(&dir)
        .arg("version")
        .current_dir(&dir.0)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn dotenv_opted_in_ignores_a_file_without_geectl_keys() {
    let dir = TempDir::new();
    let path = write_dotenv(&dir, "OTHER=1\nBROKEN LINE\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    }
    let out = geectl_isolated(&dir)
        .env("GEECTL_DOTENV", "1")
        .arg("version")
        .current_dir(&dir.0)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn dotenv_glued_export_is_malformed() {
    let dir = TempDir::new();
    write_dotenv(&dir, "exportGEECTL_OBS_URL=https://file.example.com\n");
    let out = geectl_isolated(&dir)
        .env("GEECTL_DOTENV", "1")
        .arg("version")
        .current_dir(&dir.0)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    let value: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    let context = value["error"]["context"].as_str().unwrap();
    assert!(context.contains(".env"), "{context}");
    assert!(context.contains("line 1"), "{context}");
    assert!(!stderr.contains("file.example.com"));
}

#[test]
fn dotenv_opt_in_from_the_file_itself_does_not_count() {
    let dir = TempDir::new();
    // GEECTL_DOTENV=1 in the file must not opt the file in.
    write_dotenv(
        &dir,
        "GEECTL_DOTENV=1\nGEECTL_OBS_URL=https://file.example.com\n",
    );
    let out = geectl_isolated(&dir)
        .args(["config", "show"])
        .current_dir(&dir.0)
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
}

#[test]
fn debug_reports_dotenv_loaded_without_values() {
    let dir = TempDir::new();
    write_dotenv(&dir, "GEECTL_GITEA_TOKEN=sekrit-token-value\n");
    let out = geectl_isolated(&dir)
        .env("GEECTL_DOTENV", "1")
        .args(["--debug", "version"])
        .current_dir(&dir.0)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("loaded .env"), "{stderr}");
    assert!(!stderr.contains("sekrit-token-value"), "{stderr}");
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
fn dotenv_debug_enables_without_flag() {
    let dir = TempDir::new();
    write_dotenv(&dir, "GEECTL_DEBUG=1\n");
    let out = geectl_isolated(&dir)
        .env("GEECTL_DOTENV", "1")
        .arg("version")
        .current_dir(&dir.0)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The file supplies GEECTL_DEBUG through the same path as the
    // flag: the startup line, the loaded-file line, then the config
    // line, all on stderr.
    let expected = format!(
        "geectl {} version\nloaded .env\nconfig: obs api.opensuse.org (auth: none), gitea src.opensuse.org (auth: none)\n",
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(String::from_utf8_lossy(&out.stderr), expected);
}

#[test]
fn real_debug_env_overrides_dotenv() {
    let dir = TempDir::new();
    write_dotenv(&dir, "GEECTL_DEBUG=1\n");
    let out = geectl_isolated(&dir)
        .env("GEECTL_DOTENV", "1")
        .env("GEECTL_DEBUG", "0")
        .arg("version")
        .current_dir(&dir.0)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).is_empty(),
        "real GEECTL_DEBUG=0 must win over GEECTL_DEBUG=1 in the file"
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
        "[instances]\nexample = \"https://api.opensuse.org\"\n",
    )
    .unwrap();
    let out = geectl_isolated(&dir)
        .args(["--debug", "version"])
        .env("GEECTL_OBS_USERNAME_EXAMPLE", "target-user")
        .env("GEECTL_OBS_PASSWORD_EXAMPLE", "target-secret")
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
        "owners-for-package",
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
        "create-token",
        "delete-package",
        "delete-project",
        "undelete-package",
        "undelete-project",
        "aggregate-package",
        "release-package",
        "release-project",
        "lock",
        "unlock",
        "add-channels",
        "add-containers",
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

#[test]
fn remote_owners_for_package_needs_credentials() {
    // No credentials configured: the credential gate fires before
    // any request, so no server is needed.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "owners-for-package", "vim"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(9));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
}

#[test]
fn remote_owners_for_package_rejects_an_invalid_name() {
    // An invalid OBS name is rejected by the library before any HTTP
    // call — and before the credential gate — so no credentials (and
    // no server) are needed.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "owners-for-package", "foo/bar"])
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

const OWNER_SEARCH_BODY: &str = "<collection matches=\"2\"><owner rootproject=\"openSUSE:Factory\" project=\"devel:languages:python\" package=\"foo\"><person name=\"Admin\" role=\"maintainer\"/><person name=\"scarabeus_iv\" role=\"bugowner\"/><group name=\"factory-maintainers\" role=\"maintainer\"/></owner><owner rootproject=\"openSUSE:Factory\" project=\"openSUSE:Factory\"><person name=\"Admin\" role=\"maintainer\"/></owner></collection>";
const ADMIN_PERSON_BODY: &str =
    "<person><login>Admin</login><email>admin@example.com</email><state>confirmed</state></person>";
const SCARABEUS_PERSON_BODY: &str = "<person><login>scarabeus_iv</login><email>scarabeus@example.com</email><state>confirmed</state></person>";

#[test]
fn remote_owners_for_package_json_reports_package_and_owners() {
    // The search plus one whois per distinct person: the fixture
    // answers each path, so the enrichment runs offline.
    let addr = serve_routes(&[
        ("/search/owner", OWNER_SEARCH_BODY),
        ("/person/Admin", ADMIN_PERSON_BODY),
        ("/person/scarabeus_iv", SCARABEUS_PERSON_BODY),
    ]);
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "owners-for-package", "foo"])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "secret")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["package"], "foo");
    assert_eq!(value["owners"].as_array().unwrap().len(), 2);
    assert_eq!(value["owners"][0]["root_project"], "openSUSE:Factory");
    assert_eq!(value["owners"][0]["project"], "devel:languages:python");
    assert_eq!(value["owners"][0]["package"], "foo");
    assert_eq!(
        value["owners"][0]["persons"][0]["email"],
        "admin@example.com"
    );
    assert_eq!(
        value["owners"][0]["persons"][1]["email"],
        "scarabeus@example.com"
    );
}

#[test]
fn remote_owners_for_package_text_mode_opens_with_the_package_header() {
    let addr = serve_routes(&[
        ("/search/owner", OWNER_SEARCH_BODY),
        ("/person/Admin", ADMIN_PERSON_BODY),
        ("/person/scarabeus_iv", SCARABEUS_PERSON_BODY),
    ]);
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--format", "text", "remote", "owners-for-package", "foo"])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "secret")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("package: foo\n"), "{stdout}");
    assert!(
        stdout.contains("openSUSE:Factory devel:languages:python foo: persons: Admin (maintainer) <admin@example.com>, scarabeus_iv (bugowner) <scarabeus@example.com>; groups: factory-maintainers (maintainer)"),
        "{stdout}"
    );
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
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let body = body.to_string();
    let reason = reason.to_string();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream);
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
        let _ = stream.shutdown(std::net::Shutdown::Write);
    });
    addr
}

fn serve_once(body: &str) -> std::net::SocketAddr {
    serve_once_status(200, "OK", body)
}

/// A one-shot HTTP fixture that records the request target: the same
/// single-connection shape as `serve_once`, for commands whose
/// behaviour is which URL they hit. The thread is not joined, like
/// `serve_once`.
fn serve_once_recording(body: &str) -> (std::net::SocketAddr, std::sync::mpsc::Receiver<String>) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let body = body.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        let target = String::from_utf8_lossy(&request)
            .lines()
            .next()
            .unwrap_or("")
            .split_whitespace()
            .nth(1)
            .unwrap_or("")
            .to_string();
        let _ = tx.send(target);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
        let _ = stream.shutdown(std::net::Shutdown::Write);
    });
    (addr, rx)
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

/// A one-shot HTTP fixture serving raw bytes: the same
/// single-connection shape as `serve_once`, for downloads whose body
/// is not valid UTF-8.
fn serve_once_bytes(body: &[u8]) -> std::net::SocketAddr {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let body = body.to_vec();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.write_all(&body);
        let _ = stream.flush();
        let _ = stream.shutdown(std::net::Shutdown::Write);
    });
    addr
}

/// The origin repo of the submit fixture: a fork of pool/repo
/// whose default branch is main.
const SUBMIT_FORK_REPO_JSON: &str = r#"{"owner": {"login": "owner"}, "name": "repo", "full_name": "owner/repo", "parent": {"owner": {"login": "pool"}, "name": "repo", "full_name": "pool/repo"}, "default_branch": "main", "private": false, "clone_url": "http://127.0.0.1/owner/repo.git", "ssh_url": "gitea@127.0.0.1:owner/repo.git"}"#;

/// The parent repo of the submit fixture: no parent, default
/// branch devel (the fallback base).
const SUBMIT_PARENT_REPO_JSON: &str = r#"{"owner": {"login": "pool"}, "name": "repo", "full_name": "pool/repo", "parent": null, "default_branch": "devel", "private": false, "clone_url": "http://127.0.0.1/pool/repo.git", "ssh_url": "gitea@127.0.0.1:pool/repo.git"}"#;

/// A repo that is not a fork: the parent default has nothing to
/// resolve.
const SUBMIT_NOFORK_REPO_JSON: &str = r#"{"owner": {"login": "owner"}, "name": "repo", "full_name": "owner/repo", "parent": null, "default_branch": "main", "private": false, "clone_url": "http://127.0.0.1/owner/repo.git", "ssh_url": "gitea@127.0.0.1:owner/repo.git"}"#;

/// A many-shot Gitea fixture for the submit reads: repo bodies by
/// exact path, and a branch body built from the requested branch
/// name (the branch parse requires the name to match the request)
/// for the branches listed; any other branch answers 404.
/// Anything else answers 404 too. For commands whose reads fan out
/// (the parent resolution, then the base verification), where one
/// canned body cannot answer every request.
fn serve_gitea_submit(repos: &[(&str, &str)], branches: &[&str]) -> std::net::SocketAddr {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let repos: Vec<(String, String)> = repos
        .iter()
        .map(|(path, body)| (path.to_string(), body.to_string()))
        .collect();
    let branches: Vec<String> = branches.iter().map(|name| name.to_string()).collect();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let request = read_request(&mut stream);
            let target = String::from_utf8_lossy(&request)
                .lines()
                .next()
                .unwrap_or("")
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .to_string();
            let (status, body) = match repos.iter().find(|(path, _)| path == &target) {
                Some((_, body)) => ("200 OK".to_string(), body.clone()),
                None => match target.split_once("/branches/") {
                    Some((_, name)) if branches.iter().any(|branch| branch == name) => (
                        "200 OK".to_string(),
                        format!(
                            r#"{{"name": "{name}", "commit": {{"id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "message": "initial", "url": "http://127.0.0.1/api/v1/repos/pool/repo/git/commits/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}, "protected": false}}"#
                        ),
                    ),
                    _ => ("404 Not Found".to_string(), String::new()),
                },
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Write);
        }
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
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let body = body.to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let _ = read_request(&mut stream);
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
    addr
}

/// A multi-connection HTTP fixture answering by request path:
/// each route is a path prefix and the body served for it, so one
/// command that fans out over several endpoints (an owner search
/// plus a `whois` per person) can run offline. An unmatched path
/// gets a 404; the thread is not joined, like `serve_once`.
fn serve_routes(routes: &[(&str, &str)]) -> std::net::SocketAddr {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let routes: Vec<(String, String)> = routes
        .iter()
        .map(|(path, body)| (path.to_string(), body.to_string()))
        .collect();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let request = read_request(&mut stream);
            let target = String::from_utf8_lossy(&request)
                .lines()
                .next()
                .unwrap_or("")
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .to_string();
            let path = target.split('?').next().unwrap_or("");
            let (status, reason, body) = match routes
                .iter()
                .find(|(prefix, _)| path.starts_with(prefix.as_str()))
            {
                Some((_, body)) => (200, "OK", body.clone()),
                None => (404, "Not Found", "<status code=\"not_found\"/>".to_string()),
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Write);
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
fn remote_devel_project_target_project_carries_the_target_in_payload_and_text() {
    // The fixture answers the one request whatever its path is; the
    // library unit tests pin the meta read, and the live tests pin
    // the pair on a non-Factory project. This pins the CLI half:
    // `--target-project` reaches the payload and the header, and the
    // default stays Factory.
    let body = "<package name=\"foo\" project=\"test:factory\"><devel project=\"test:devel\" package=\"foo\"/></package>";
    let dir = TempDir::new();
    let addr = serve_once(body);
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "devel-project",
            "foo",
            "--target-project",
            "test:factory",
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
    assert_eq!(value["project"], "test:factory");
    assert_eq!(value["package"], "foo");
    assert_eq!(value["devel_project"], "test:devel");
    assert_eq!(value["devel_package"], "foo");
    let addr = serve_once(body);
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "remote",
            "devel-project",
            "foo",
            "--target-project",
            "test:factory",
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
        Some("project: test:factory package: foo"),
        "{stdout}"
    );
    assert!(
        stdout.contains("devel project: test:devel package: foo"),
        "{stdout}"
    );
    // The default target is Factory.
    let addr = serve_once(body);
    let out = geectl_isolated(&dir)
        .args(["remote", "devel-project", "foo"])
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
}

#[test]
fn remote_list_files_include_deleted_carries_flag_in_payload_and_keeps_the_text_table() {
    // The fixture answers the one request whatever its query is; the
    // library unit tests pin `?deleted=1` on the wire. This pins the
    // CLI half: the payload carries the flag, and the text stays the
    // design table under the coordinate header. The deleted listing
    // is a credentialed read, so the fixture calls carry credentials.
    let body = "<directory><entry name=\"vim.spec\" size=\"1234\" mtime=\"1700000000\" md5=\"deadbeef\"/></directory>";
    let dir = TempDir::new();
    let addr = serve_once(body);
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "list-files",
            "openSUSE:Factory",
            "vim",
            "--include-deleted",
        ])
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
    assert_eq!(value["include_deleted"], true);
    assert_eq!(value["files"][0]["name"], "vim.spec");
    let addr = serve_once(body);
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "remote",
            "list-files",
            "openSUSE:Factory",
            "vim",
            "--include-deleted",
        ])
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
fn remote_list_files_include_deleted_with_half_set_credentials_fails_with_config_error() {
    // The deleted listing is a credentialed read, so it fails fast
    // on a half-set env like the other credentialed reads — the
    // plain listing stays public.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "list-files",
            "openSUSE:Factory",
            "vim",
            "--include-deleted",
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
fn remote_list_files_include_deleted_without_credentials_fails_closed() {
    // No credentials at all: the deleted listing fails fast with a
    // config error instead of sending an unauthenticated request
    // for the server to 401 — the plain listing stays public.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "list-files",
            "openSUSE:Factory",
            "vim",
            "--include-deleted",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    assert!(value["error"]["context"]
        .as_str()
        .unwrap()
        .contains("without OBS credentials"));
}

#[test]
fn remote_search_packages_by_devel_project_with_half_set_credentials_fails_with_config_error() {
    // The devel-project form is a credentialed read too, so it fails
    // fast like the term form.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "search-packages",
            "--devel-project",
            "devel:languages:python",
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
fn remote_search_packages_exact_without_term_fails_with_invalid_input() {
    // `--exact` needs a term to filter on: clap refuses it with
    // --devel-project alone through the single JSON error path.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "search-packages",
            "--exact",
            "--devel-project",
            "devel:languages:python",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
}

#[test]
fn remote_search_packages_without_term_or_devel_project_fails_with_invalid_input() {
    // A term is required unless --devel-project is given: clap
    // refuses the bare leaf through the single JSON error path.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "search-packages"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
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
fn remote_search_packages_by_person_with_half_set_credentials_fails_with_config_error() {
    // The package person search needs credentials, so it fails fast
    // like the project form.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "search-packages-by-person", "alice"])
        .env("GEECTL_OBS_USERNAME", "alice")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
}

#[test]
fn person_search_both_arms_returns_projects_and_users() {
    // Both arms run against fixture servers: the OBS
    // person-match answers one project, the Gitea user search
    // one user. The JSON payload carries both lists.
    let obs_addr = serve_once(
        "<collection matches=\"1\"><project name=\"devel:languages:python\"><title>Python</title><description>Python packages</description></project></collection>",
    );
    let gitea_addr = serve_once(
        r#"{"ok": true, "data": [{"login": "alice", "full_name": "Alice", "email": "alice@example.com"}]}"#,
    );
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["person", "search", "alice"])
        .env("GEECTL_OBS_URL", format!("http://{obs_addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "test-password")
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
    assert_eq!(value["query"], "alice");
    assert!(value["backend"].is_null());
    assert_eq!(value["projects"][0]["name"], "devel:languages:python");
    assert_eq!(value["projects"][0]["title"], "Python");
    assert_eq!(value["users"][0]["login"], "alice");
    assert_eq!(value["users"][0]["full_name"], "Alice");
    assert_eq!(value["users"][0]["email"], "alice@example.com");
}

#[test]
fn person_search_text_mode_renders_both_sections() {
    let obs_addr = serve_once(
        "<collection matches=\"1\"><project name=\"devel:languages:python\"><title>Python</title><description>Python packages</description></project></collection>",
    );
    let gitea_addr = serve_once(
        r#"{"ok": true, "data": [{"login": "alice", "full_name": "Alice", "email": "alice@example.com"}]}"#,
    );
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--format", "text", "person", "search", "alice"])
        .env("GEECTL_OBS_URL", format!("http://{obs_addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "test-password")
        .env("GEECTL_GITEA_URL", format!("http://{gitea_addr}"))
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("query: alice"), "stdout: {stdout}");
    assert!(stdout.contains("OBS projects:"), "stdout: {stdout}");
    assert!(
        stdout.contains("devel:languages:python"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("Gitea users:"), "stdout: {stdout}");
    assert!(stdout.contains("alice@example.com"), "stdout: {stdout}");
}

#[test]
fn person_search_with_no_credentials_fails_with_config_error() {
    // No arm has credentials: both clients build before the
    // search runs, so the missing Gitea token fails first — the
    // same construction order as `pull-request search`.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["person", "search", "alice"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let value: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    assert!(
        stderr.contains("no Gitea token configured"),
        "stderr: {stderr}"
    );
}

#[test]
fn person_search_gitea_scoped_with_no_token_fails_with_config_error() {
    // Scoped to Gitea, the OBS arm never runs: only the missing
    // token fails the search.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["person", "search", "alice", "--backend", "gitea"])
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "test-password")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let value: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    assert!(stderr.contains("Gitea token"), "stderr: {stderr}");
}

#[test]
fn person_search_both_arms_with_only_a_gitea_token_fails_on_obs_credentials() {
    // Only the Gitea token is configured: both clients construct
    // (OBS construction accepts Credentials::None by design), and
    // the failure surfaces at the OBS arm, before any HTTP — the
    // Gitea arm never runs, and no OBS request is attempted. The
    // recording stub proves the last point: a request would land
    // in its channel before the process exits.
    let (obs_addr, obs_requests) = serve_once_recording(
        "<collection matches=\"1\"><project name=\"devel:languages:python\"><title>Python</title><description>Python packages</description></project></collection>",
    );
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["person", "search", "alice"])
        .env("GEECTL_OBS_URL", format!("http://{obs_addr}"))
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let value: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    assert!(stderr.contains("OBS credentials"), "stderr: {stderr}");
    assert!(
        obs_requests
            .recv_timeout(std::time::Duration::from_secs(1))
            .is_err(),
        "the OBS arm must fail before any OBS request is attempted"
    );
}

#[test]
fn person_search_obs_scoped_returns_only_projects() {
    // Scoped to OBS, the Gitea arm never runs: no token is
    // configured, and its construction would fail if touched.
    // The payload names the backend and carries only the OBS list.
    let obs_addr = serve_once(
        "<collection matches=\"1\"><project name=\"devel:languages:python\"><title>Python</title><description>Python packages</description></project></collection>",
    );
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["person", "search", "alice", "--backend", "obs"])
        .env("GEECTL_OBS_URL", format!("http://{obs_addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "test-password")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["query"], "alice");
    assert_eq!(value["backend"], "obs");
    assert_eq!(value["projects"][0]["name"], "devel:languages:python");
    assert_eq!(value["projects"][0]["title"], "Python");
    assert!(value["users"].as_array().unwrap().is_empty());
}

#[test]
fn person_search_gitea_scoped_returns_only_users() {
    // Scoped to Gitea, the OBS arm never runs: no OBS credentials
    // are configured, and its arm would fail if touched. The
    // payload names the backend and carries only the Gitea list.
    let gitea_addr = serve_once(
        r#"{"ok": true, "data": [{"login": "alice", "full_name": "Alice", "email": "alice@example.com"}]}"#,
    );
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["person", "search", "alice", "--backend", "gitea"])
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
    assert_eq!(value["query"], "alice");
    assert_eq!(value["backend"], "gitea");
    assert_eq!(value["users"][0]["login"], "alice");
    assert_eq!(value["users"][0]["full_name"], "Alice");
    assert_eq!(value["users"][0]["email"], "alice@example.com");
    assert!(value["projects"].as_array().unwrap().is_empty());
}

#[test]
fn person_search_obs_scoped_with_no_obs_credentials_fails_with_config_error() {
    // Scoped to OBS, the Gitea arm never runs: only the missing
    // OBS credentials fail the search.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["person", "search", "alice", "--backend", "obs"])
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let value: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    assert!(stderr.contains("OBS credentials"), "stderr: {stderr}");
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

#[test]
fn remote_list_scmsync_projects_with_half_set_credentials_fails_with_config_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "list-scmsync-projects",
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

#[test]
fn remote_list_scmsync_projects_with_a_quote_in_the_prefix_fails_with_invalid_input() {
    // The quote is rejected before any request, even with
    // credentials configured.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "list-scmsync-projects", "https://example.com/x'"])
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
}

#[test]
fn remote_list_scmsync_projects_with_limit_zero_fails_with_invalid_input() {
    // A limit of 0 is rejected before any request, even with
    // credentials configured.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "list-scmsync-projects",
            "--limit",
            "0",
            "https://github.com/openSUSE/",
        ])
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
}

#[test]
fn remote_list_scmsync_projects_reports_the_payload_shape() {
    let addr = serve_once(
        "<collection matches=\"2\"><project name=\"home:alice:branches:foo\"/><project name=\"home:alice:branches:bar\"/></collection>",
    );
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "list-scmsync-projects",
            "https://github.com/openSUSE/",
        ])
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
    assert_eq!(value["url_prefix"], "https://github.com/openSUSE/");
    assert_eq!(value["matches"].as_u64(), Some(2));
    assert_eq!(value["truncated"].as_bool(), Some(false));
    assert_eq!(
        value["projects"],
        serde_json::json!(["home:alice:branches:foo", "home:alice:branches:bar"])
    );
}

#[test]
fn remote_list_scmsync_projects_text_mode_opens_with_the_url_prefix_header() {
    let addr = serve_once(
        "<collection matches=\"2\"><project name=\"home:alice:branches:foo\"/><project name=\"home:alice:branches:bar\"/></collection>",
    );
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "remote",
            "list-scmsync-projects",
            "https://github.com/openSUSE/",
        ])
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
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.trim(),
        "url-prefix: https://github.com/openSUSE/\nNAME\nhome:alice:branches:foo\nhome:alice:branches:bar"
    );
}

#[test]
fn remote_list_scmsync_projects_empty_result_reports_an_empty_list() {
    let addr = serve_once("<collection matches=\"0\"/>");
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "list-scmsync-projects",
            "https://github.com/openSUSE/",
        ])
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
    assert_eq!(value["url_prefix"], "https://github.com/openSUSE/");
    assert_eq!(value["matches"].as_u64(), Some(0));
    assert_eq!(value["truncated"].as_bool(), Some(false));
    assert_eq!(value["projects"], serde_json::json!([]));
}

#[test]
fn remote_list_scmsync_projects_past_the_end_reports_an_empty_page() {
    // An offset past the last match is an empty page, not an error:
    // the server's total says 3 matches exist and the page starts
    // at 5, so the JSON payload carries an empty list with the total.
    let addr = serve_once("<collection matches=\"3\"/>");
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "list-scmsync-projects",
            "--limit",
            "2",
            "--offset",
            "5",
            "https://github.com/openSUSE/",
        ])
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
    assert_eq!(value["url_prefix"], "https://github.com/openSUSE/");
    assert_eq!(value["limit"].as_u64(), Some(2));
    assert_eq!(value["offset"].as_u64(), Some(5));
    assert_eq!(value["matches"].as_u64(), Some(3));
    assert_eq!(value["truncated"].as_bool(), Some(false));
    assert_eq!(value["projects"], serde_json::json!([]));
}

#[test]
fn remote_list_scmsync_projects_past_the_end_text_mode_flags_the_empty_page() {
    // The same past-end page in text mode renders the designed
    // empty-page line, not a no-matches line and not an error.
    let addr = serve_once("<collection matches=\"3\"/>");
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "remote",
            "list-scmsync-projects",
            "--limit",
            "2",
            "--offset",
            "5",
            "https://github.com/openSUSE/",
        ])
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
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("no projects on this page"));
    assert!(stdout.contains("3 matches in total"));
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
fn remote_delete_file_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "delete-file", "home:alice", "vim", "foo.spec"])
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
    assert_eq!(value["file"], "foo.spec");
    assert_eq!(
        value["plan"][0],
        "[dry-run] would: delete file home:alice/vim/foo.spec"
    );
}

#[test]
fn remote_delete_file_live_with_half_set_credentials_fails_with_config_error() {
    // A live mutation does need credentials, so it fails fast.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "remote",
            "delete-file",
            "home:alice",
            "vim",
            "foo.spec",
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
fn remote_delete_file_invalid_name_fails_with_json_error() {
    // Client-side validation: no server is contacted, so the name
    // fails in dry-run and live alike; live is the path a user hits.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "remote",
            "delete-file",
            "home:alice",
            "vim",
            "../x",
        ])
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
}

#[test]
fn remote_delete_file_missing_file_is_not_found() {
    // The server 404s a delete of a file that is not there: exit 3,
    // `not_found` on stderr. The loopback server answers one
    // connection; DELETE is never retried, so one is enough.
    let addr = serve_once_status(404, "Not Found", "");
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "remote",
            "delete-file",
            "home:alice",
            "vim",
            "missing.spec",
        ])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "not_found");
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
fn remote_set_project_meta_add_person_dry_run_ignores_half_set_credentials() {
    // A person-only change writes no meta itself — the meta write
    // runs only when a field flag is set — so the plan carries only
    // the role entry, and the payload carries the granted person.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-project-meta",
            "home:alice",
            "--add-person",
            "alice:maintainer",
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
    assert_eq!(value["persons"][0]["name"], "alice");
    assert_eq!(value["persons"][0]["role"], "maintainer");
    assert_eq!(
        value["plan"],
        serde_json::json!([
            "[dry-run] would: add maintainer role for person alice on project home:alice"
        ])
    );
}

#[test]
fn remote_set_project_meta_field_and_person_flags_dry_run() {
    // A field flag and a person flag combine: the meta write plans
    // first, then the role grant.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-project-meta",
            "home:alice",
            "--title",
            "Alice",
            "--add-person",
            "alice:maintainer",
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
    assert_eq!(value["title"], "Alice");
    assert_eq!(value["persons"][0]["name"], "alice");
    assert_eq!(
        value["plan"],
        serde_json::json!([
            "[dry-run] would: set project meta of home:alice",
            "[dry-run] would: add maintainer role for person alice on project home:alice"
        ])
    );
}

#[test]
fn remote_set_package_meta_person_flags_dry_run_records_removal_before_addition() {
    // Removal applies before addition: the plan lists the removal
    // first, so the pair moves a person to a new role.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-package-meta",
            "home:alice",
            "vim",
            "--add-person",
            "alice:maintainer",
            "--remove-person",
            "bob",
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
    assert_eq!(
        value["plan"],
        serde_json::json!([
            "[dry-run] would: remove person bob from package home:alice/vim (unverified: dry-run does not read the live meta)",
            "[dry-run] would: add maintainer role for person alice on package home:alice/vim"
        ])
    );
    assert_eq!(value["persons"][0]["name"], "alice");
    assert_eq!(value["persons"][0]["role"], "maintainer");
}

#[test]
fn remote_set_project_meta_add_person_without_colon_fails() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-project-meta",
            "home:alice",
            "--add-person",
            "alice",
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
        "invalid person role \"alice\": expected LOGIN:ROLE"
    );
}

#[test]
fn remote_set_project_meta_add_person_with_invalid_role_fails_in_dry_run() {
    // The role is validated by the library call before the dry-run
    // guard, so an invalid role fails without a server.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-project-meta",
            "home:alice",
            "--add-person",
            "alice:admin",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert_eq!(
        value["error"]["context"],
        "invalid OBS role \"admin\" for project home:alice; roles are maintainer, bugowner, reviewer, downloader, reader"
    );
}

#[test]
fn remote_set_project_meta_add_person_with_invalid_login_fails_in_dry_run() {
    // The login is validated up front, so a bad login fails without
    // a server, as a login error rather than a role error.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-project-meta",
            "home:alice",
            "--add-person",
            "a/b:maintainer",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert_eq!(
        value["error"]["context"],
        "invalid OBS person name: \"a/b\""
    );
}

#[test]
fn remote_set_project_meta_handover_dry_run_grants_the_maintainer_first() {
    // A maintainer grant to someone who is not being removed applies
    // before the removal, so a handover in one call keeps a
    // maintainer; the removal itself is unverified in dry-run.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-project-meta",
            "home:alice",
            "--add-person",
            "bob:maintainer",
            "--remove-person",
            "alice",
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
    assert_eq!(
        value["plan"],
        serde_json::json!([
            "[dry-run] would: add maintainer role for person bob on project home:alice",
            "[dry-run] would: remove person alice from project home:alice (unverified: dry-run does not read the live meta)"
        ])
    );
    assert_eq!(value["persons"][0]["name"], "bob");
    assert_eq!(value["persons"][0]["role"], "maintainer");
}

#[test]
fn remote_set_project_meta_same_person_move_dry_run_removes_before_adding() {
    // The same-login move removes first, then adds: the plan lists
    // the removal before the grant, so the person ends on the new
    // role only.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-project-meta",
            "home:alice",
            "--remove-person",
            "alice",
            "--add-person",
            "alice:reviewer",
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
    assert_eq!(
        value["plan"],
        serde_json::json!([
            "[dry-run] would: remove person alice from project home:alice (unverified: dry-run does not read the live meta)",
            "[dry-run] would: add reviewer role for person alice on project home:alice"
        ])
    );
    assert_eq!(value["persons"][0]["name"], "alice");
    assert_eq!(value["persons"][0]["role"], "reviewer");
}

#[test]
fn remote_set_project_meta_remove_person_with_invalid_login_fails() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-project-meta",
            "home:alice",
            "--remove-person",
            "bad login",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert_eq!(
        value["error"]["context"],
        "invalid OBS person name: \"bad login\""
    );
}

#[test]
fn remote_set_project_meta_without_any_flag_fails() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "set-project-meta", "home:alice"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert_eq!(
        value["error"]["context"],
        "set-project-meta needs at least one of --from-file, --title, --description, --url, --scmsync, --devel-project, --add-person, --remove-person"
    );
}

#[test]
fn remote_set_package_meta_without_any_flag_fails() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "set-package-meta", "home:alice", "vim"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert_eq!(
        value["error"]["context"],
        "set-package-meta needs at least one of --from-file, --title, --description, --url, --scmsync, --releasename, --devel-project, --devel-package, --add-person, --remove-person"
    );
}

#[test]
fn remote_set_project_meta_from_file_dry_run() {
    // The whole-file write plans the file's meta: title,
    // description, URL, devel project, person, and repository.
    let dir = TempDir::new();
    let file = dir.0.join("meta.xml");
    fs::write(
        &file,
        "<project name=\"home:alice\">\
         <title>Alice</title>\
         <description>Home project</description>\
         <url>https://example.org</url>\
         <devel project=\"openSUSE:Factory\"/>\
         <person userid=\"alice\" role=\"maintainer\"/>\
         <repository name=\"standard\">\
         <path project=\"openSUSE:Factory\" repository=\"standard\"/>\
         <arch>x86_64</arch>\
         </repository></project>",
    )
    .unwrap();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-project-meta",
            "home:alice",
            "--from-file",
            file.to_str().unwrap(),
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
    assert_eq!(value["name"], "home:alice");
    assert_eq!(value["title"], "Alice");
    assert_eq!(value["description"], "Home project");
    assert_eq!(value["url"], "https://example.org");
    assert_eq!(value["devel"], "openSUSE:Factory");
    assert_eq!(value["persons"][0]["name"], "alice");
    assert_eq!(value["repositories"][0]["name"], "standard");
    assert_eq!(
        value["plan"],
        serde_json::json!(["[dry-run] would: set project meta of home:alice"])
    );
    // The text result line names the file it was written from.
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "remote",
            "set-project-meta",
            "home:alice",
            "--from-file",
            file.to_str().unwrap(),
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
        stdout.contains(&format!(
            "dry-run: would set project meta of home:alice (from-file: {})",
            file.display()
        )),
        "stdout: {stdout}"
    );
}

#[test]
fn remote_set_package_meta_from_file_dry_run() {
    // The whole-file write plans the file's meta: title, URL,
    // release name, devel pair, and person.
    let dir = TempDir::new();
    let file = dir.0.join("meta.xml");
    fs::write(
        &file,
        "<package name=\"vim\" project=\"home:alice\">\
         <title>Vim</title>\
         <url>https://example.org/vim</url>\
         <releasename>vim</releasename>\
         <devel project=\"devel:languages:vim\" package=\"vim\"/>\
         <person userid=\"alice\" role=\"maintainer\"/>\
         </package>",
    )
    .unwrap();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-package-meta",
            "home:alice",
            "vim",
            "--from-file",
            file.to_str().unwrap(),
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
    assert_eq!(value["project"], "home:alice");
    assert_eq!(value["name"], "vim");
    assert_eq!(value["title"], "Vim");
    assert_eq!(value["url"], "https://example.org/vim");
    assert_eq!(value["releasename"], "vim");
    assert_eq!(value["devel"]["project"], "devel:languages:vim");
    assert_eq!(value["devel"]["package"], "vim");
    assert_eq!(value["persons"][0]["name"], "alice");
    assert_eq!(
        value["plan"],
        serde_json::json!(["[dry-run] would: set package meta of home:alice/vim"])
    );
}

#[test]
fn remote_set_project_meta_from_file_with_field_flag_fails() {
    // Bimodal: `--from-file` and a field flag together are a
    // usage error, before any file is read.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-project-meta",
            "home:alice",
            "--from-file",
            "meta.xml",
            "--title",
            "Alice",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert!(
        value["error"]["context"]
            .as_str()
            .unwrap()
            .contains("cannot be used with"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn remote_set_package_meta_from_file_with_person_flag_fails() {
    // Bimodal: `--from-file` and a person flag together are a
    // usage error too — the file already carries the persons.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-package-meta",
            "home:alice",
            "vim",
            "--from-file",
            "meta.xml",
            "--add-person",
            "alice:maintainer",
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
            .contains("cannot be used with"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn remote_set_project_meta_from_file_missing_file_fails() {
    let dir = TempDir::new();
    let file = dir.0.join("no-such-meta.xml");
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-project-meta",
            "home:alice",
            "--from-file",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    let context = value["error"]["context"].as_str().unwrap();
    assert!(context.contains("cannot read "));
    assert!(
        context.contains(&file.display().to_string()),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn remote_set_project_meta_from_file_malformed_fails() {
    let dir = TempDir::new();
    let file = dir.0.join("meta.xml");
    fs::write(&file, "not a meta document").unwrap();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-project-meta",
            "home:alice",
            "--from-file",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert_eq!(
        value["error"]["context"],
        format!(
            "cannot parse meta file {}: not a <project> document",
            file.display()
        )
    );
}

#[test]
fn remote_set_package_meta_from_file_wrong_project_fails() {
    // The file's project must match the package: a file written
    // for another project fails in dry-run, before any write.
    let dir = TempDir::new();
    let file = dir.0.join("meta.xml");
    fs::write(
        &file,
        "<package name=\"vim\" project=\"other\"><title>Vim</title></package>",
    )
    .unwrap();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-package-meta",
            "home:alice",
            "vim",
            "--from-file",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert_eq!(
        value["error"]["context"],
        "package meta is for \"other\"/\"vim\", not \"home:alice\"/\"vim\""
    );
}

#[test]
fn remote_set_project_meta_from_file_wrong_project_fails() {
    // The file's project must match the project: a file written
    // for another project fails in dry-run, before any write.
    let dir = TempDir::new();
    let file = dir.0.join("meta.xml");
    fs::write(
        &file,
        "<project name=\"other\"><title>Other</title></project>",
    )
    .unwrap();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-project-meta",
            "home:alice",
            "--from-file",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert_eq!(
        value["error"]["context"],
        "project meta is for \"other\", not \"home:alice\""
    );
}

#[test]
fn remote_set_package_meta_from_file_missing_file_fails() {
    let dir = TempDir::new();
    let file = dir.0.join("no-such-meta.xml");
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-package-meta",
            "home:alice",
            "vim",
            "--from-file",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    let context = value["error"]["context"].as_str().unwrap();
    assert!(context.contains("cannot read "));
    assert!(
        context.contains(&file.display().to_string()),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn remote_set_package_meta_from_file_malformed_fails() {
    let dir = TempDir::new();
    let file = dir.0.join("meta.xml");
    fs::write(&file, "not a meta document").unwrap();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-package-meta",
            "home:alice",
            "vim",
            "--from-file",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert_eq!(
        value["error"]["context"],
        format!(
            "cannot parse meta file {}: not a <package> document",
            file.display()
        )
    );
}

#[test]
fn remote_set_project_meta_from_file_package_document_fails() {
    // The project leaf takes a `<project>` document, not a
    // `<package>` one.
    let dir = TempDir::new();
    let file = dir.0.join("meta.xml");
    fs::write(
        &file,
        "<package name=\"vim\" project=\"home:alice\"><title>Vim</title></package>",
    )
    .unwrap();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "set-project-meta",
            "home:alice",
            "--from-file",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert_eq!(
        value["error"]["context"],
        format!(
            "cannot parse meta file {}: not a <project> document",
            file.display()
        )
    );
}

#[test]
fn remote_set_project_meta_add_person_live_with_half_set_credentials_fails_with_config_error() {
    // A live person change needs credentials: the library's
    // credentials check runs after the dry-run guard.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "remote",
            "set-project-meta",
            "home:alice",
            "--add-person",
            "alice:maintainer",
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
fn remote_lock_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "lock",
            "openSUSE:Factory",
            "vim",
            "--message",
            "freeze for release",
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
    assert_eq!(value["project"], "openSUSE:Factory");
    assert_eq!(value["package"], "vim");
    assert_eq!(value["message"], "freeze for release");
    assert_eq!(
        value["plan"][0],
        "[dry-run] would: lock package openSUSE:Factory/vim (message freeze for release)"
    );
    // A project lock without a message plans with no suffix.
    let out = geectl_isolated(&dir)
        .args(["remote", "lock", "openSUSE:Factory"])
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
    assert_eq!(
        value["plan"][0],
        "[dry-run] would: lock project openSUSE:Factory"
    );
}

#[test]
fn remote_unlock_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "unlock",
            "openSUSE:Factory",
            "vim",
            "--message",
            "release done",
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
    assert_eq!(value["project"], "openSUSE:Factory");
    assert_eq!(value["package"], "vim");
    assert_eq!(value["message"], "release done");
    assert_eq!(
        value["plan"][0],
        "[dry-run] would: unlock package openSUSE:Factory/vim (message release done)"
    );
}

#[test]
fn remote_unlock_empty_message_fails_with_invalid_input() {
    // The server requires a comment, so an empty message is refused
    // before the dry-run guard — a dry-run preview must never
    // promise an unlock the server would refuse.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["remote", "unlock", "openSUSE:Factory", "--message", ""])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
}

#[test]
fn remote_add_channels_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "add-channels",
            "openSUSE:Factory",
            "vim",
            "--enable-all",
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
    assert_eq!(value["project"], "openSUSE:Factory");
    assert_eq!(value["package"], "vim");
    assert_eq!(value["enable_all"], true);
    assert_eq!(
        value["plan"][0],
        "[dry-run] would: add channels to package openSUSE:Factory/vim (enable all)"
    );
}

#[test]
fn remote_add_channels_both_modes_fails_with_invalid_input() {
    // `--enable-all` and `--skip-disabled` conflict at parse time:
    // clap refuses the combination through the single JSON error path.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "add-channels",
            "openSUSE:Factory",
            "--enable-all",
            "--skip-disabled",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
}

#[test]
fn remote_add_containers_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "remote",
            "add-containers",
            "openSUSE:Factory",
            "vim",
            "--extend-package-names",
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
    assert_eq!(value["project"], "openSUSE:Factory");
    assert_eq!(value["package"], "vim");
    assert_eq!(value["extend_package_names"], true);
    assert_eq!(
        value["plan"][0],
        "[dry-run] would: add containers to package openSUSE:Factory/vim (extend package names)"
    );
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
fn build_buildconfig_help_lists_the_alternative_project_flag() {
    let out = geectl()
        .args(["build", "buildconfig", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("--alternative-project <ALTERNATIVE_PROJECT>"),
        "{text}"
    );
    assert!(text.contains("another project instead"), "{text}");
}

#[test]
fn build_buildconfig_reads_the_alternative_project() {
    // The alternative project is the fetch project: the request goes
    // to the alternative project's `_buildconfig`, while the payload
    // still names the project the config was read for. The stub
    // answers whatever it is asked, so the recorded target is the
    // proof of which project was fetched.
    let dir = TempDir::new();
    let (addr, target) = serve_once_recording("Macro: test 1\n");
    let out = geectl_isolated(&dir)
        .args([
            "build",
            "buildconfig",
            "testproject",
            "standard",
            "--alternative-project",
            "openSUSE:Factory",
        ])
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
    assert_eq!(
        target
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the stub saw one request"),
        "/build/openSUSE:Factory/standard/_buildconfig"
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["project"], "testproject");
    assert_eq!(value["repository"], "standard");
    assert_eq!(value["alternative_project"], "openSUSE:Factory");
    assert_eq!(value["buildconfig"], "Macro: test 1\n");
    // Without the flag the same project is fetched, with a null
    // alternative project.
    let (addr, target) = serve_once_recording("Macro: test 1\n");
    let out = geectl_isolated(&dir)
        .args(["build", "buildconfig", "testproject", "standard"])
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
    assert_eq!(
        target
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the stub saw one request"),
        "/build/testproject/standard/_buildconfig"
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["project"], "testproject");
    assert_eq!(value["alternative_project"], serde_json::Value::Null);
    assert_eq!(value["buildconfig"], "Macro: test 1\n");
}

#[test]
fn build_buildconfig_rejects_a_bad_alternative_project_before_any_request() {
    // The name validation runs in the library before any HTTP: with
    // the base URL on a closed port the failure is invalid_input, not
    // a network error — no request ever goes out.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "build",
            "buildconfig",
            "testproject",
            "standard",
            "--alternative-project",
            "bad name!",
        ])
        .env("GEECTL_OBS_URL", "http://127.0.0.1:1")
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "s3cr3t")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
}

#[test]
fn service_help_lists_all_subcommands() {
    let out = geectl().args(["service", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    for name in ["list", "state", "trigger"] {
        assert!(text.contains(name), "service --help misses {name}");
    }
}

#[test]
fn service_trigger_dry_run_reports_plan_without_credentials() {
    // The trigger is a mutation with dry-run as the default: the
    // plan is printed and nothing goes out, with no credentials at
    // all — the library's dry-run guard runs before its
    // credentials check — and the JSON payload carries the plan.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["service", "trigger", "P", "K"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["triggered"], false);
    assert_eq!(value["queued"], false);
    assert_eq!(value["dry_run"], true);
    assert_eq!(value["service"], serde_json::Value::Null);
    assert_eq!(value["code"], serde_json::Value::Null);
    assert_eq!(
        value["plan"],
        serde_json::json!(["[dry-run] would: trigger services for P/K"])
    );
    // The named form plans the service it is for.
    let out = geectl_isolated(&dir)
        .args(["service", "trigger", "P", "K", "--service", "download_url"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["triggered"], false);
    assert_eq!(value["queued"], false);
    assert_eq!(value["dry_run"], true);
    assert_eq!(value["service"], "download_url");
    assert_eq!(value["code"], serde_json::Value::Null);
    assert_eq!(
        value["plan"],
        serde_json::json!(["[dry-run] would: trigger services for P/K (service download_url)"])
    );
}

#[test]
fn service_trigger_live_against_closed_port_fails_with_network_error() {
    // `--live` flows through main.rs into the trigger: with
    // credentials set and the base URL on a closed loopback port,
    // the POST fails with a network error instead of silently
    // dry-running.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "service", "trigger", "P", "K"])
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
fn service_trigger_dry_run_fails_fast_on_half_set_credentials() {
    // The service group resolves credentials up front: a
    // half-configured pair fails fast before any work, even for a
    // dry-run trigger that needs no credentials itself.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["service", "trigger", "P", "K"])
        .env("GEECTL_OBS_USERNAME", "alice")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
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
fn gitea_help_lists_all_nine_subcommands() {
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
        "maintainers",
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
        "submit",
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
fn top_help_lists_the_meta_group() {
    let out = geectl().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("meta"), "top --help misses the meta group");
}

#[test]
fn meta_help_lists_the_set_subcommand() {
    let out = geectl().args(["meta", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("set"), "meta --help misses set");
}

#[test]
fn meta_set_help_names_the_target_and_field_flags() {
    let out = geectl().args(["meta", "set", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    for flag in ["--project", "--package", "--repo", "--description", "--url"] {
        assert!(text.contains(flag), "meta set --help misses {flag}");
    }
}

#[test]
fn top_help_lists_the_working_copy_leaves() {
    let out = geectl().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    for name in [
        "checkout",
        "update",
        "status",
        "diff",
        "add",
        "rm",
        "addremove",
        "commit",
        "pull",
        "resolve",
    ] {
        assert!(text.contains(name), "top --help misses {name}");
    }
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
        "dry-run: would merge obs:123; would forward after merge"
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
fn pull_request_merge_obs_with_no_forward_is_invalid_input() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["pull-request", "merge", "obs:123", "--no-forward"])
        .output()
        .unwrap();
    // InvalidInput exits 2; --no-forward does not apply to OBS.
    // The refusal precedes credential resolution, so the dry-run
    // credential warning never appears; the assertions are
    // substrings, not a JSON parse.
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("invalid_input"), "stderr: {stderr}");
    assert!(stderr.contains("--no-forward"), "stderr: {stderr}");
}

#[test]
fn pull_request_merge_gitea_dry_run_prints_forward_plan() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["pull-request", "merge", "gitea:openSUSE/geectl#122"])
        .env("GEECTL_GITEA_TOKEN", "tok-fake")
        .output()
        .unwrap();
    // A dry-run Gitea merge prints without a server: the plan
    // names the merge, the forward that would follow it, and the
    // note that the forward preflight was skipped without OBS
    // credentials — the forward is planned unverified.
    assert!(out.status.success());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["reference"], "gitea:openSUSE/geectl#122");
    assert_eq!(value["status"], serde_json::Value::Null);
    assert_eq!(value["dry_run"], true);
    let plan = value["plan"].as_array().expect("payload carries a plan");
    assert_eq!(plan.len(), 3);
    assert!(
        plan[0]
            .as_str()
            .unwrap()
            .contains("merge pull request openSUSE/geectl#122"),
        "plan: {plan:?}"
    );
    assert_eq!(
        plan[1],
        "[dry-run] would: forward gitea:openSUSE/geectl#122 after merge"
    );
    assert_eq!(
        plan[2],
        "[dry-run] note: forward preflight skipped: no OBS credentials, so the forward was not verified"
    );
}

#[test]
fn pull_request_merge_gitea_text_dry_run_prints_forward_plan() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "pull-request",
            "merge",
            "gitea:openSUSE/geectl#122",
        ])
        .env("GEECTL_GITEA_TOKEN", "tok-fake")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "dry-run: would merge gitea:openSUSE/geectl#122; would forward after merge"
    );
}

#[test]
fn pull_request_merge_gitea_dry_run_with_no_forward_prints_no_forward_plan() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "pull-request",
            "merge",
            "gitea:openSUSE/geectl#122",
            "--no-forward",
        ])
        .env("GEECTL_GITEA_TOKEN", "tok-fake")
        .output()
        .unwrap();
    assert!(out.status.success());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    let plan = value["plan"].as_array().expect("payload carries a plan");
    assert_eq!(plan.len(), 1);
    assert!(
        plan[0]
            .as_str()
            .unwrap()
            .contains("merge pull request openSUSE/geectl#122"),
        "plan: {plan:?}"
    );
    let out = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "pull-request",
            "merge",
            "gitea:openSUSE/geectl#122",
            "--no-forward",
        ])
        .env("GEECTL_GITEA_TOKEN", "tok-fake")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "dry-run: would merge gitea:openSUSE/geectl#122"
    );
}

/// A routing stub Gitea server for the live merge-then-forward:
/// the merge POST, the pull-request read, the reviews read, and
/// the repo read each get their canned answer, by method and
/// request target. Anything else is a 404. The PR merges into
/// `pool/systemd` on `factory`; the repo's clone URL spells the
/// scmsync repo the OBS stub's package syncs from.
fn serve_gitea_merge_forward() -> (
    std::net::SocketAddr,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_in_thread = std::sync::Arc::clone(&seen);
    let pr_json = r#"{"number":7,"title":"Update systemd to 257","body":"Update systemd to 257","state":"closed","draft":false,"merged":true,"user":{"login":"scarabeus_iv"},"labels":[],"head":{"label":"scarabeus_iv:systemd-257","ref":"systemd-257","sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","repo":{"full_name":"scarabeus_iv/systemd"}},"base":{"label":"factory","ref":"factory","sha":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","repo":{"full_name":"pool/systemd"}}}"#;
    let repo_json = format!(
        r#"{{"owner":{{"login":"pool"}},"name":"systemd","full_name":"pool/systemd","default_branch":"factory","private":false,"clone_url":"http://{addr}/pool/systemd.git","ssh_url":"git@{addr}:pool/systemd.git"}}"#
    );
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
            let (status, body) = if method == "POST" && target.contains("/pulls/7/merge") {
                (200, "{}".to_string())
            } else if target.contains("/pulls/7/reviews") {
                (200, "[]".to_string())
            } else if target.contains("/pulls/7") {
                (200, pr_json.to_string())
            } else if target == "/api/v1/repos/pool/systemd" {
                (200, repo_json.clone())
            } else {
                (404, "{}".to_string())
            };
            let response = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
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

/// A routing stub OBS server for the live Gitea merge-then-forward:
/// the scmsync package search, the candidate meta, the target
/// package's directory listing, the link parent's meta, the
/// maintenance search, the filing dedup search, and the create
/// POST each get their canned answer. The candidate package
/// `devel:systemd/systemd` syncs from the Gitea stub's repo on
/// `factory` and links to `openSUSE:Factory/systemd`.
fn serve_obs_gitea_merge_forward(
    gitea_addr: std::net::SocketAddr,
) -> (
    std::net::SocketAddr,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_in_thread = std::sync::Arc::clone(&seen);
    let search_xml = "<collection matches=\"1\"><package name=\"systemd\" project=\"devel:systemd\"><title>systemd</title><description>System and Session Manager</description></package></collection>";
    let candidate_meta = format!(
        "<package name=\"systemd\" project=\"devel:systemd\"><scmsync>http://{gitea_addr}/pool/systemd#factory</scmsync></package>"
    );
    let parent_meta = "<package name=\"systemd\" project=\"openSUSE:Factory\"><devel project=\"devel:systemd\" package=\"systemd\"/></package>";
    let directory_xml =
        "<directory><linkinfo project=\"openSUSE:Factory\" package=\"systemd\" /></directory>";
    let empty_collection = "<collection matches=\"0\"></collection>";
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
            let (status, body) = if method == "POST" && target.contains("cmd=create") {
                (
                    200,
                    "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>".to_string(),
                )
            } else if target.contains("/_meta") && target.contains("openSUSE:Factory") {
                (200, parent_meta.to_string())
            } else if target.contains("/_meta") {
                (200, candidate_meta.clone())
            } else if target.contains("search/package") {
                (200, search_xml.to_string())
            } else if target.contains("search/project/id") || target.contains("search/request") {
                (200, empty_collection.to_string())
            } else if target.starts_with("/source/") {
                (200, directory_xml.to_string())
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

/// A routing stub OBS server for the live Gitea merge-then-forward
/// with an ambiguous scmsync repo: the package search returns two
/// packages synced from the same repo, so the forward must refuse
/// to guess which one the merge targeted. The candidate metas both
/// name the Gitea stub's repo on `factory`; everything else answers
/// like `serve_obs_gitea_merge_forward`.
fn serve_obs_gitea_merge_forward_ambiguous(
    gitea_addr: std::net::SocketAddr,
) -> (
    std::net::SocketAddr,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_in_thread = std::sync::Arc::clone(&seen);
    let search_xml = "<collection matches=\"2\"><package name=\"systemd\" project=\"devel:systemd\"><title>systemd</title><description>System and Session Manager</description></package><package name=\"systemd\" project=\"home:alice\"><title>systemd</title><description>System and Session Manager</description></package></collection>";
    let candidate_meta = format!(
        "<package name=\"systemd\" project=\"devel:systemd\"><scmsync>http://{gitea_addr}/pool/systemd#factory</scmsync></package>"
    );
    let parent_meta = "<package name=\"systemd\" project=\"openSUSE:Factory\"><devel project=\"devel:systemd\" package=\"systemd\"/></package>";
    let directory_xml =
        "<directory><linkinfo project=\"openSUSE:Factory\" package=\"systemd\" /></directory>";
    let empty_collection = "<collection matches=\"0\"></collection>";
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
            let (status, body) = if method == "POST" && target.contains("cmd=create") {
                (
                    200,
                    "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>".to_string(),
                )
            } else if target.contains("/_meta") && target.contains("openSUSE:Factory") {
                (200, parent_meta.to_string())
            } else if target.contains("/_meta") {
                (200, candidate_meta.clone())
            } else if target.contains("search/package") {
                (200, search_xml.to_string())
            } else if target.contains("search/project/id") || target.contains("search/request") {
                (200, empty_collection.to_string())
            } else if target.starts_with("/source/") {
                (200, directory_xml.to_string())
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
fn pull_request_merge_gitea_live_forwards_after_merge() {
    let (gitea_addr, gitea_seen) = serve_gitea_merge_forward();
    let (obs_addr, obs_seen) = serve_obs_gitea_merge_forward(gitea_addr);
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "pull-request", "merge", "gitea:pool/systemd#7"])
        .env("GEECTL_GITEA_URL", format!("http://{gitea_addr}"))
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .env("GEECTL_OBS_URL", format!("http://{obs_addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "test-password")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["reference"], "gitea:pool/systemd#7");
    assert_eq!(value["status"], "merged");
    assert_eq!(value["dry_run"], false);
    assert_eq!(value["forward"]["actions"][0]["outcome"], "forwarded");
    assert_eq!(
        value["forward"]["actions"][0]["source"],
        "devel:systemd/systemd"
    );
    assert_eq!(
        value["forward"]["actions"][0]["target"],
        "openSUSE:Factory/systemd"
    );
    assert_eq!(value["forward"]["actions"][0]["new_id"], 1400001);
    // The guards must drop before the next run: the stub threads
    // take the same lock per request, and holding it across the
    // run would stall every response.
    {
        let gitea_seen = gitea_seen.lock().unwrap();
        assert!(
            gitea_seen
                .iter()
                .any(|r| r.contains("POST") && r.contains("/pulls/7/merge")),
            "gitea requests: {gitea_seen:?}"
        );
        assert!(
            gitea_seen.iter().any(|r| r.contains("/pulls/7")),
            "gitea requests: {gitea_seen:?}"
        );
    }
    {
        let obs_seen = obs_seen.lock().unwrap();
        assert!(
            obs_seen.iter().any(|r| r.contains("search/package")),
            "obs requests: {obs_seen:?}"
        );
        assert!(
            obs_seen.iter().any(|r| r.contains("cmd=create")),
            "obs requests: {obs_seen:?}"
        );
    }
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "--format",
            "text",
            "pull-request",
            "merge",
            "gitea:pool/systemd#7",
        ])
        .env("GEECTL_GITEA_URL", format!("http://{gitea_addr}"))
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .env("GEECTL_OBS_URL", format!("http://{obs_addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "test-password")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "merged gitea:pool/systemd#7 (now merged); forwarded devel:systemd/systemd to openSUSE:Factory/systemd as request 1400001"
    );
}

#[test]
fn pull_request_merge_gitea_live_preflight_refuses_before_the_merge() {
    // The preflight finds two OBS packages synced from the merged
    // repo and refuses to guess which one the merge targeted — the
    // merge never lands, and nothing is filed.
    let (gitea_addr, gitea_seen) = serve_gitea_merge_forward();
    let (obs_addr, obs_seen) = serve_obs_gitea_merge_forward_ambiguous(gitea_addr);
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "pull-request", "merge", "gitea:pool/systemd#7"])
        .env("GEECTL_GITEA_URL", format!("http://{gitea_addr}"))
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .env("GEECTL_OBS_URL", format!("http://{obs_addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "test-password")
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "the ambiguous merge exited 0: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        stderr.contains("refusing to guess which one the merge targeted"),
        "stderr: {stderr}"
    );
    // The guards must drop before the next run: the stub threads
    // take the same lock per request, and holding it across the
    // run would stall every response.
    {
        let gitea_seen = gitea_seen.lock().unwrap();
        assert!(
            !gitea_seen
                .iter()
                .any(|r| r.contains("POST") && r.contains("/pulls/7/merge")),
            "the merge landed before the preflight refused: {gitea_seen:?}"
        );
    }
    {
        let obs_seen = obs_seen.lock().unwrap();
        assert!(
            !obs_seen.iter().any(|r| r.contains("cmd=create")),
            "a forward was filed for an ambiguous merge: {obs_seen:?}"
        );
    }
}

#[test]
fn pull_request_merge_gitea_dry_run_verifies_the_forward_when_credentials_are_available() {
    // With both servers up, the dry-run preflight resolves the
    // hop: the plan names the verified forward, with no skip note.
    let (gitea_addr, _gitea_seen) = serve_gitea_merge_forward();
    let (obs_addr, _obs_seen) = serve_obs_gitea_merge_forward(gitea_addr);
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["pull-request", "merge", "gitea:pool/systemd#7"])
        .env("GEECTL_GITEA_URL", format!("http://{gitea_addr}"))
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .env("GEECTL_OBS_URL", format!("http://{obs_addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "test-password")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["reference"], "gitea:pool/systemd#7");
    assert_eq!(value["status"], serde_json::Value::Null);
    assert_eq!(value["dry_run"], true);
    let plan = value["plan"].as_array().expect("payload carries a plan");
    assert_eq!(plan.len(), 2);
    assert_eq!(
        plan[1],
        "[dry-run] would: forward gitea:pool/systemd#7 after merge"
    );
}

#[test]
fn pull_request_merge_gitea_dry_run_preflight_refuses_an_ambiguous_forward() {
    // The same ambiguous repo in dry-run: the preflight refuses,
    // so the dry-run fails instead of printing "would forward".
    let (gitea_addr, gitea_seen) = serve_gitea_merge_forward();
    let (obs_addr, _obs_seen) = serve_obs_gitea_merge_forward_ambiguous(gitea_addr);
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["pull-request", "merge", "gitea:pool/systemd#7"])
        .env("GEECTL_GITEA_URL", format!("http://{gitea_addr}"))
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .env("GEECTL_OBS_URL", format!("http://{obs_addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "test-password")
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "the ambiguous dry-run exited 0: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        stderr.contains("refusing to guess which one the merge targeted"),
        "stderr: {stderr}"
    );
    {
        let gitea_seen = gitea_seen.lock().unwrap();
        assert!(
            !gitea_seen
                .iter()
                .any(|r| r.contains("POST") && r.contains("/pulls/7/merge")),
            "a dry-run merged: {gitea_seen:?}"
        );
    }
}

#[test]
fn pull_request_merge_gitea_live_without_obs_credentials_fails_before_the_merge() {
    // A live Gitea merge without OBS credentials fails at the
    // preflight, before the merge: the forward cannot be
    // validated, so the pull request stays open.
    let (gitea_addr, gitea_seen) = serve_gitea_merge_forward();
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "pull-request", "merge", "gitea:pool/systemd#7"])
        .env("GEECTL_GITEA_URL", format!("http://{gitea_addr}"))
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(9));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("GEECTL_OBS_USERNAME"), "stderr: {stderr}");
    {
        let gitea_seen = gitea_seen.lock().unwrap();
        assert!(
            !gitea_seen
                .iter()
                .any(|r| r.contains("POST") && r.contains("/pulls/7/merge")),
            "the merge landed without OBS credentials: {gitea_seen:?}"
        );
    }
}

/// A routing stub OBS server for the live Gitea merge-then-forward
/// whose filing fails: every read answers like
/// `serve_obs_gitea_merge_forward`, but the create POST answers
/// 500, so the forward fails after the merge lands. The dry-run
/// preflight never files, so it verifies the hop and passes —
/// the failure surfaces only in the live forward.
fn serve_obs_gitea_merge_forward_create_fails(
    gitea_addr: std::net::SocketAddr,
) -> (
    std::net::SocketAddr,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_in_thread = std::sync::Arc::clone(&seen);
    let search_xml = "<collection matches=\"1\"><package name=\"systemd\" project=\"devel:systemd\"><title>systemd</title><description>System and Session Manager</description></package></collection>";
    let candidate_meta = format!(
        "<package name=\"systemd\" project=\"devel:systemd\"><scmsync>http://{gitea_addr}/pool/systemd#factory</scmsync></package>"
    );
    let parent_meta = "<package name=\"systemd\" project=\"openSUSE:Factory\"><devel project=\"devel:systemd\" package=\"systemd\"/></package>";
    let directory_xml =
        "<directory><linkinfo project=\"openSUSE:Factory\" package=\"systemd\" /></directory>";
    let empty_collection = "<collection matches=\"0\"></collection>";
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
            let (status, body) = if method == "POST" && target.contains("cmd=create") {
                (500, "<status code=\"internal_error\" />".to_string())
            } else if target.contains("/_meta") && target.contains("openSUSE:Factory") {
                (200, parent_meta.to_string())
            } else if target.contains("/_meta") {
                (200, candidate_meta.clone())
            } else if target.contains("search/package") {
                (200, search_xml.to_string())
            } else if target.contains("search/project/id") || target.contains("search/request") {
                (200, empty_collection.to_string())
            } else if target.starts_with("/source/") {
                (200, directory_xml.to_string())
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

/// A routing stub OBS server for the live Gitea merge-then-forward
/// with no package synced from the merged repo: the scmsync
/// package search answers empty, so there is nothing to forward.
/// Everything else answers like `serve_obs_gitea_merge_forward`.
fn serve_obs_gitea_merge_forward_no_package(
    gitea_addr: std::net::SocketAddr,
) -> (
    std::net::SocketAddr,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_in_thread = std::sync::Arc::clone(&seen);
    let candidate_meta = format!(
        "<package name=\"systemd\" project=\"devel:systemd\"><scmsync>http://{gitea_addr}/pool/systemd#factory</scmsync></package>"
    );
    let parent_meta = "<package name=\"systemd\" project=\"openSUSE:Factory\"><devel project=\"devel:systemd\" package=\"systemd\"/></package>";
    let directory_xml =
        "<directory><linkinfo project=\"openSUSE:Factory\" package=\"systemd\" /></directory>";
    let empty_collection = "<collection matches=\"0\"></collection>";
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
            let (status, body) = if method == "POST" && target.contains("cmd=create") {
                (
                    200,
                    "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>".to_string(),
                )
            } else if target.contains("/_meta") && target.contains("openSUSE:Factory") {
                (200, parent_meta.to_string())
            } else if target.contains("/_meta") {
                (200, candidate_meta.clone())
            } else if target.contains("search/package")
                || target.contains("search/project/id")
                || target.contains("search/request")
            {
                (200, empty_collection.to_string())
            } else if target.starts_with("/source/") {
                (200, directory_xml.to_string())
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
fn pull_request_merge_gitea_live_merge_stands_when_the_forward_fails() {
    // The filing POST 500s: the preflight verifies the hop (the
    // dry-run forward never files), the merge lands, and the live
    // forward then fails — the failure is reported with the merge
    // standing, and nothing is filed.
    let (gitea_addr, gitea_seen) = serve_gitea_merge_forward();
    let (obs_addr, obs_seen) = serve_obs_gitea_merge_forward_create_fails(gitea_addr);
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "pull-request", "merge", "gitea:pool/systemd#7"])
        .env("GEECTL_GITEA_URL", format!("http://{gitea_addr}"))
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .env("GEECTL_OBS_URL", format!("http://{obs_addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "test-password")
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "the failed-forward merge exited 0: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(out.status.code(), Some(6));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        stderr.contains("merged gitea:pool/systemd#7, but the forward failed"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("returned 500"), "stderr: {stderr}");
    // The guards must drop before the next run: the stub threads
    // take the same lock per request, and holding it across the
    // run would stall every response.
    {
        let gitea_seen = gitea_seen.lock().unwrap();
        assert!(
            gitea_seen
                .iter()
                .any(|r| r.contains("POST") && r.contains("/pulls/7/merge")),
            "the merge never landed: {gitea_seen:?}"
        );
    }
    {
        let obs_seen = obs_seen.lock().unwrap();
        assert!(
            obs_seen.iter().any(|r| r.contains("cmd=create")),
            "the forward never attempted the filing: {obs_seen:?}"
        );
    }
}

#[test]
fn pull_request_merge_gitea_live_no_package_forwards_nothing() {
    // No OBS package syncs from the merged repo: the merge lands,
    // the forward reports the hop skipped, and nothing is filed.
    let (gitea_addr, gitea_seen) = serve_gitea_merge_forward();
    let (obs_addr, obs_seen) = serve_obs_gitea_merge_forward_no_package(gitea_addr);
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["--live", "pull-request", "merge", "gitea:pool/systemd#7"])
        .env("GEECTL_GITEA_URL", format!("http://{gitea_addr}"))
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .env("GEECTL_OBS_URL", format!("http://{obs_addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "test-password")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["reference"], "gitea:pool/systemd#7");
    assert_eq!(value["status"], "merged");
    assert_eq!(value["dry_run"], false);
    assert_eq!(value["forward"]["actions"][0]["outcome"], "skipped");
    assert_eq!(
        value["forward"]["actions"][0]["source"],
        "pool/systemd#factory"
    );
    assert_eq!(
        value["forward"]["actions"][0]["reason"],
        format!("no OBS package is synced from http://{gitea_addr}/pool/systemd#factory")
    );
    {
        let gitea_seen = gitea_seen.lock().unwrap();
        assert!(
            gitea_seen
                .iter()
                .any(|r| r.contains("POST") && r.contains("/pulls/7/merge")),
            "gitea requests: {gitea_seen:?}"
        );
    }
    {
        let obs_seen = obs_seen.lock().unwrap();
        assert!(
            !obs_seen.iter().any(|r| r.contains("cmd=create")),
            "a forward was filed with no package synced: {obs_seen:?}"
        );
    }
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "--format",
            "text",
            "pull-request",
            "merge",
            "gitea:pool/systemd#7",
        ])
        .env("GEECTL_GITEA_URL", format!("http://{gitea_addr}"))
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .env("GEECTL_OBS_URL", format!("http://{obs_addr}"))
        .env("GEECTL_OBS_USERNAME", "alice")
        .env("GEECTL_OBS_PASSWORD", "test-password")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!(
            "merged gitea:pool/systemd#7 (now merged); skipped pool/systemd#factory: no OBS package is synced from http://{gitea_addr}/pool/systemd#factory"
        )
    );
}

#[test]
fn pull_request_merge_gitea_live_with_no_forward_merges_without_forwarding() {
    // `--no-forward` skips the preflight and the forward: the merge
    // lands with no OBS server involved at all, so no OBS
    // credentials are needed and the payload carries no forward.
    let (gitea_addr, gitea_seen) = serve_gitea_merge_forward();
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "--live",
            "pull-request",
            "merge",
            "gitea:pool/systemd#7",
            "--no-forward",
        ])
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
    assert_eq!(value["reference"], "gitea:pool/systemd#7");
    assert_eq!(value["status"], "merged");
    assert_eq!(value["dry_run"], false);
    assert!(value.get("forward").is_none(), "payload: {value}");
    {
        let gitea_seen = gitea_seen.lock().unwrap();
        assert!(
            gitea_seen
                .iter()
                .any(|r| r.contains("POST") && r.contains("/pulls/7/merge")),
            "gitea requests: {gitea_seen:?}"
        );
    }
}
/// Run git in `dir`, asserting success.
fn git_in(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
        .status;
    assert!(
        status.success(),
        "git {} failed in {}",
        args.join(" "),
        dir.display()
    );
}

/// A hand-built OBS-native checkout at `<dir>/checkout`: a root
/// commit with the base message and the `geectl.*` record, base
/// rev 1, srcmd5 of all `a`s, apiurl the fixture URL.
fn obs_submit_checkout(dir: &TempDir, apiurl: &str) -> std::path::PathBuf {
    let checkout = dir.0.join("checkout");
    std::fs::create_dir_all(&checkout).unwrap();
    git_in(&checkout, &["init", "--quiet"]);
    git_in(&checkout, &["config", "--local", "user.name", "geectl"]);
    git_in(
        &checkout,
        &["config", "--local", "user.email", "geectl@localhost"],
    );
    std::fs::write(checkout.join("foo.spec"), "Name: foo\n").unwrap();
    git_in(&checkout, &["add", "-A"]);
    git_in(
        &checkout,
        &[
            "commit",
            "--quiet",
            "-m",
            "geectl base 1 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ],
    );
    for (key, value) in [
        ("geectl.apiurl", apiurl),
        ("geectl.project", "home:alice"),
        ("geectl.package", "foo"),
        ("geectl.backend", "obs"),
        ("geectl.base-rev", "1"),
        ("geectl.base-srcmd5", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    ] {
        git_in(&checkout, &["config", "--local", key, value]);
    }
    checkout
}

/// A hand-built git-backed checkout at `<dir>/checkout`: branch
/// `main`, origin `https://src.opensuse.org/owner/repo` — the
/// default instance, so the origin host matches — base the head
/// commit. The shape a real `checkout` records.
fn git_submit_checkout(dir: &TempDir) -> std::path::PathBuf {
    let checkout = dir.0.join("checkout");
    std::fs::create_dir_all(&checkout).unwrap();
    git_in(&checkout, &["init", "--quiet", "-b", "main"]);
    git_in(&checkout, &["config", "--local", "user.name", "geectl"]);
    git_in(
        &checkout,
        &["config", "--local", "user.email", "geectl@localhost"],
    );
    std::fs::write(checkout.join("foo.spec"), "Name: foo\n").unwrap();
    git_in(&checkout, &["add", "-A"]);
    git_in(&checkout, &["commit", "--quiet", "-m", "initial"]);
    let head = String::from_utf8(
        Command::new("git")
            .current_dir(&checkout)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let head = head.trim().to_string();
    git_in(
        &checkout,
        &[
            "config",
            "--local",
            "remote.origin.url",
            "https://src.opensuse.org/owner/repo",
        ],
    );
    for (key, value) in [
        ("geectl.apiurl", "https://api.opensuse.org"),
        ("geectl.project", "home:alice"),
        ("geectl.package", "foo"),
        ("geectl.backend", "git"),
        ("geectl.base-rev", head.as_str()),
        ("geectl.base-srcmd5", ""),
    ] {
        git_in(&checkout, &["config", "--local", key, value]);
    }
    checkout
}

#[test]
fn pull_request_submit_obs_dry_run_prints_plan() {
    let dir = TempDir::new();
    let addr = serve_once(
        "<directory name=\"foo\" rev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" vrev=\"1\">\
        <entry md5=\"9444ad22a238286bca9bf649b2aef01d\" mtime=\"1282047302\" name=\"foo.spec\" size=\"10\" />\
        </directory>",
    );
    let checkout = obs_submit_checkout(&dir, &format!("http://{addr}"));
    let out = geectl_isolated(&dir)
        .current_dir(&checkout)
        .args([
            "pull-request",
            "submit",
            "--target-project",
            "home:devel",
            "--description",
            "Update foo",
            "--changes-entry",
            "- Update to 1.1.",
        ])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .output()
        .unwrap();
    // A dry-run submit on OBS prints without credentials: the
    // plan names no acting user, and the only network read is the
    // base revision for the freshness check.
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["reference"], serde_json::Value::Null);
    assert_eq!(value["superseded"], serde_json::Value::Null);
    assert_eq!(value["dry_run"], true);
    assert_eq!(
        value["plan"][0],
        "[dry-run] would: file submit request for foo to home:devel"
    );
}

#[test]
fn pull_request_submit_obs_text_dry_run_prints_plan() {
    let dir = TempDir::new();
    let addr = serve_once(
        "<directory name=\"foo\" rev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" vrev=\"1\">\
        <entry md5=\"9444ad22a238286bca9bf649b2aef01d\" mtime=\"1282047302\" name=\"foo.spec\" size=\"10\" />\
        </directory>",
    );
    let checkout = obs_submit_checkout(&dir, &format!("http://{addr}"));
    let out = geectl_isolated(&dir)
        .current_dir(&checkout)
        .args([
            "--format",
            "text",
            "pull-request",
            "submit",
            "--target-project",
            "home:devel",
            "--description",
            "Update foo",
            "--changes-entry",
            "- Update to 1.1.",
        ])
        .env("GEECTL_OBS_URL", format!("http://{addr}"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "dry-run: would file a submit request for home:alice/foo to home:devel"
    );
}

#[test]
fn pull_request_submit_gitea_dry_run_prints_plan() {
    let dir = TempDir::new();
    let checkout = git_submit_checkout(&dir);
    // The fixture instance answers the parent resolution and the
    // base verification: the origin is a fork of pool/repo, and
    // main exists on the target.
    let addr = serve_gitea_submit(
        &[("/api/v1/repos/owner/repo", SUBMIT_FORK_REPO_JSON)],
        &["main"],
    );
    let origin = format!("http://{addr}/owner/repo");
    git_in(
        &checkout,
        &["config", "--local", "remote.origin.url", origin.as_str()],
    );
    let out = geectl_isolated(&dir)
        .current_dir(&checkout)
        .args([
            "pull-request",
            "submit",
            "--base",
            "main",
            "--title",
            "Update foo",
        ])
        .env("GEECTL_GITEA_URL", format!("http://{addr}"))
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .output()
        .unwrap();
    // A dry-run submit on Gitea prints with a token configured
    // (the client needs one to build); the plan reads the parent
    // and the base but files nothing — the token is sent only on
    // those reads, never on a filing POST. The dry-run plan does
    // not fetch, and pr_create posts nothing.
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["reference"], serde_json::Value::Null);
    assert_eq!(value["superseded"], serde_json::Value::Null);
    assert_eq!(value["dry_run"], true);
    assert_eq!(
        value["plan"][0],
        "[dry-run] would: create pull request in pool/repo from owner:main to main"
    );
}

#[test]
fn pull_request_submit_gitea_text_dry_run_prints_plan() {
    let dir = TempDir::new();
    let checkout = git_submit_checkout(&dir);
    let addr = serve_gitea_submit(
        &[("/api/v1/repos/owner/repo", SUBMIT_FORK_REPO_JSON)],
        &["main"],
    );
    let origin = format!("http://{addr}/owner/repo");
    git_in(
        &checkout,
        &["config", "--local", "remote.origin.url", origin.as_str()],
    );
    let out = geectl_isolated(&dir)
        .current_dir(&checkout)
        .args([
            "--format",
            "text",
            "pull-request",
            "submit",
            "--base",
            "main",
            "--title",
            "Update foo",
        ])
        .env("GEECTL_GITEA_URL", format!("http://{addr}"))
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "dry-run: would open a pull request for pool/repo from owner:main to main"
    );
}

#[test]
fn pull_request_submit_obs_with_repo_is_invalid_input() {
    let dir = TempDir::new();
    let checkout = obs_submit_checkout(&dir, "https://api.opensuse.org");
    let out = geectl_isolated(&dir)
        .current_dir(&checkout)
        .args([
            "pull-request",
            "submit",
            "--target-project",
            "home:devel",
            "--description",
            "Update foo",
            "--changes-entry",
            "- Update to 1.1.",
            "--repo",
            "owner/repo",
        ])
        .output()
        .unwrap();
    // InvalidInput exits 2; --repo does not apply to an
    // OBS-native checkout. The refusal precedes credential
    // resolution and any network read, so no fixture is needed.
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("invalid_input"), "stderr: {stderr}");
    assert!(stderr.contains("--repo"), "stderr: {stderr}");
}

#[test]
fn pull_request_submit_gitea_with_target_project_is_invalid_input() {
    let dir = TempDir::new();
    let checkout = git_submit_checkout(&dir);
    let out = geectl_isolated(&dir)
        .current_dir(&checkout)
        .args([
            "pull-request",
            "submit",
            "--target-project",
            "home:devel",
            "--base",
            "main",
            "--title",
            "Update foo",
        ])
        .output()
        .unwrap();
    // InvalidInput exits 2; --target-project does not apply to a
    // git-backed checkout. The refusal precedes credential
    // resolution, so it fires without a token.
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("invalid_input"), "stderr: {stderr}");
    assert!(stderr.contains("--target-project"), "stderr: {stderr}");
}

/// A submit invocation that must fail with InvalidInput (exit 2):
/// the args, and the substring the error must name. Every case
/// runs without credentials and without a fixture: the refusal
/// precedes credential resolution and any network read.
fn assert_submit_invalid_input(
    dir: &TempDir,
    checkout: &std::path::Path,
    args: &[&str],
    needle: &str,
) {
    let out = geectl_isolated(dir)
        .current_dir(checkout)
        .args(["pull-request", "submit"])
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "args: {args:?}");
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("invalid_input"),
        "args: {args:?} stderr: {stderr}"
    );
    assert!(stderr.contains(needle), "args: {args:?} stderr: {stderr}");
}

#[test]
fn pull_request_submit_obs_missing_required_flags_are_invalid_input() {
    let dir = TempDir::new();
    let checkout = obs_submit_checkout(&dir, "https://api.opensuse.org");
    // Each required flag missing in turn; the other two present.
    assert_submit_invalid_input(
        &dir,
        &checkout,
        &[
            "--description",
            "Update foo",
            "--changes-entry",
            "- Update to 1.1.",
        ],
        "--target-project",
    );
    assert_submit_invalid_input(
        &dir,
        &checkout,
        &[
            "--target-project",
            "home:devel",
            "--changes-entry",
            "- Update to 1.1.",
        ],
        "--description",
    );
    assert_submit_invalid_input(
        &dir,
        &checkout,
        &[
            "--target-project",
            "home:devel",
            "--description",
            "Update foo",
        ],
        "--changes-entry",
    );
}

#[test]
fn pull_request_submit_gitea_missing_required_flags_are_invalid_input() {
    let dir = TempDir::new();
    let checkout = git_submit_checkout(&dir);
    // --base now defaults (the scmsync branch, else the target's
    // default branch); only --title stays required.
    assert_submit_invalid_input(&dir, &checkout, &["--base", "main"], "--title");
}

#[test]
fn pull_request_submit_obs_foreign_flags_are_invalid_input() {
    let dir = TempDir::new();
    let checkout = obs_submit_checkout(&dir, "https://api.opensuse.org");
    let base = [
        "--target-project",
        "home:devel",
        "--description",
        "Update foo",
        "--changes-entry",
        "- Update to 1.1.",
    ];
    for (flag, value) in [
        ("--repo", "owner/repo"),
        ("--head", "feature"),
        ("--base", "main"),
        ("--title", "Update foo"),
        ("--body", "The body."),
    ] {
        let mut args: Vec<&str> = base.to_vec();
        args.push(flag);
        args.push(value);
        assert_submit_invalid_input(&dir, &checkout, &args, flag);
    }
    let mut args: Vec<&str> = base.to_vec();
    args.push("--draft");
    assert_submit_invalid_input(&dir, &checkout, &args, "--draft");
}

#[test]
fn pull_request_submit_gitea_foreign_flags_are_invalid_input() {
    let dir = TempDir::new();
    let checkout = git_submit_checkout(&dir);
    let base = ["--base", "main", "--title", "Update foo"];
    for (flag, value) in [
        ("--target-project", "home:devel"),
        ("--target-package", "foo"),
        ("--description", "Update foo"),
        ("--changes-entry", "- Update to 1.1."),
        ("--cve", "CVE-2026-1234"),
    ] {
        let mut args: Vec<&str> = base.to_vec();
        args.push(flag);
        args.push(value);
        assert_submit_invalid_input(&dir, &checkout, &args, flag);
    }
    let mut args: Vec<&str> = base.to_vec();
    args.push("--without-cleanup");
    assert_submit_invalid_input(&dir, &checkout, &args, "--without-cleanup");
}

#[test]
fn pull_request_submit_gitea_foreign_host_is_invalid_input() {
    let dir = TempDir::new();
    let checkout = git_submit_checkout(&dir);
    // The fixture origin is the default instance; point it at a
    // foreign host instead: same owner/repo shape, wrong instance.
    git_in(
        &checkout,
        &[
            "config",
            "--local",
            "remote.origin.url",
            "https://codeberg.org/owner/repo",
        ],
    );
    let out = geectl_isolated(&dir)
        .current_dir(&checkout)
        .args([
            "pull-request",
            "submit",
            "--base",
            "main",
            "--title",
            "Update foo",
        ])
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .output()
        .unwrap();
    // InvalidInput exits 2; the origin host is not the configured
    // instance. The refusal precedes any filing, so the token is
    // never sent anywhere.
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("invalid_input"), "stderr: {stderr}");
    assert!(
        stderr.contains("not the configured Gitea instance"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("pass --repo"), "stderr: {stderr}");
}

#[test]
fn pull_request_submit_gitea_alias_token_dry_run_prints_plan() {
    let dir = TempDir::new();
    // A custom instance under an alias: the token lives only in
    // the alias variable, never in the plain one. The instance is
    // a loopback fixture, so the parent resolution and the base
    // verification answer.
    let addr = serve_gitea_submit(
        &[("/api/v1/repos/owner/repo", SUBMIT_FORK_REPO_JSON)],
        &["main"],
    );
    let config_dir = dir.0.join("geectl");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.toml"),
        format!("[instances]\ncustom = \"http://{addr}\"\n"),
    )
    .unwrap();
    let checkout = git_submit_checkout(&dir);
    let origin = format!("http://{addr}/owner/repo");
    git_in(
        &checkout,
        &["config", "--local", "remote.origin.url", origin.as_str()],
    );
    let out = geectl_isolated(&dir)
        .current_dir(&checkout)
        .args([
            "pull-request",
            "submit",
            "--base",
            "main",
            "--title",
            "Update foo",
        ])
        .env("GEECTL_GITEA_URL", format!("http://{addr}"))
        .env("GEECTL_GITEA_TOKEN_CUSTOM", "alias-token")
        .output()
        .unwrap();
    // The alias token resolves for the client and the fetch alike:
    // one resolution, shared. The dry-run plan prints; the reads
    // resolve the parent and the base, nothing is filed.
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["reference"], serde_json::Value::Null);
    assert_eq!(value["dry_run"], true);
    assert_eq!(
        value["plan"][0],
        "[dry-run] would: create pull request in pool/repo from owner:main to main"
    );
}

#[test]
fn pull_request_submit_gitea_base_with_path_reports_the_base() {
    let dir = TempDir::new();
    let checkout = git_submit_checkout(&dir);
    // The origin host matches the configured base; the base itself
    // is malformed (a path), and no token is configured at all.
    git_in(
        &checkout,
        &[
            "config",
            "--local",
            "remote.origin.url",
            "https://host/owner/repo",
        ],
    );
    let out = geectl_isolated(&dir)
        .current_dir(&checkout)
        .args([
            "pull-request",
            "submit",
            "--base",
            "main",
            "--title",
            "Update foo",
        ])
        .env("GEECTL_GITEA_URL", "https://host/evil")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The base validates first: the error names the base, never
    // the missing token.
    assert!(stderr.contains("invalid_input"), "stderr: {stderr}");
    assert!(stderr.contains("Gitea base URL"), "stderr: {stderr}");
    assert!(
        !stderr.contains("no Gitea token configured"),
        "stderr: {stderr}"
    );
}

#[test]
fn pull_request_submit_gitea_factory_base_is_invalid_input() {
    let dir = TempDir::new();
    let checkout = git_submit_checkout(&dir);
    // The refusal precedes credential resolution and any network
    // read: the factory branch takes no pull requests.
    assert_submit_invalid_input(
        &dir,
        &checkout,
        &["--base", "factory", "--title", "Update foo"],
        "the factory branch does not take pull requests",
    );
}

#[test]
fn pull_request_submit_gitea_base_defaults_from_the_scmsync_branch() {
    let dir = TempDir::new();
    let checkout = git_submit_checkout(&dir);
    git_in(
        &checkout,
        &["config", "--local", "geectl.scmsync-branch", "devel"],
    );
    let addr = serve_gitea_submit(
        &[("/api/v1/repos/owner/repo", SUBMIT_FORK_REPO_JSON)],
        &["devel"],
    );
    let origin = format!("http://{addr}/owner/repo");
    git_in(
        &checkout,
        &["config", "--local", "remote.origin.url", origin.as_str()],
    );
    let out = geectl_isolated(&dir)
        .current_dir(&checkout)
        .args(["pull-request", "submit", "--title", "Update foo"])
        .env("GEECTL_GITEA_URL", format!("http://{addr}"))
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
    assert_eq!(
        value["plan"][0],
        "[dry-run] would: create pull request in pool/repo from owner:main to devel"
    );
}

#[test]
fn pull_request_submit_gitea_base_defaults_from_the_target_default_branch() {
    let dir = TempDir::new();
    let checkout = git_submit_checkout(&dir);
    // No scmsync branch on this checkout: the base falls back to
    // the target repo's default branch (devel on pool/repo).
    let addr = serve_gitea_submit(
        &[
            ("/api/v1/repos/owner/repo", SUBMIT_FORK_REPO_JSON),
            ("/api/v1/repos/pool/repo", SUBMIT_PARENT_REPO_JSON),
        ],
        &["devel"],
    );
    let origin = format!("http://{addr}/owner/repo");
    git_in(
        &checkout,
        &["config", "--local", "remote.origin.url", origin.as_str()],
    );
    let out = geectl_isolated(&dir)
        .current_dir(&checkout)
        .args(["pull-request", "submit", "--title", "Update foo"])
        .env("GEECTL_GITEA_URL", format!("http://{addr}"))
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
    assert_eq!(
        value["plan"][0],
        "[dry-run] would: create pull request in pool/repo from owner:main to devel"
    );
}

#[test]
fn pull_request_submit_gitea_origin_not_a_fork_is_invalid_input() {
    let dir = TempDir::new();
    let checkout = git_submit_checkout(&dir);
    let addr = serve_gitea_submit(
        &[("/api/v1/repos/owner/repo", SUBMIT_NOFORK_REPO_JSON)],
        &[],
    );
    let origin = format!("http://{addr}/owner/repo");
    git_in(
        &checkout,
        &["config", "--local", "remote.origin.url", origin.as_str()],
    );
    let out = geectl_isolated(&dir)
        .current_dir(&checkout)
        .args(["pull-request", "submit", "--title", "Update foo"])
        .env("GEECTL_GITEA_URL", format!("http://{addr}"))
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("invalid_input"), "stderr: {stderr}");
    assert!(stderr.contains("not a fork"), "stderr: {stderr}");
}

#[test]
fn pull_request_submit_gitea_target_not_parent_is_invalid_input() {
    let dir = TempDir::new();
    let checkout = git_submit_checkout(&dir);
    let addr = serve_gitea_submit(&[("/api/v1/repos/owner/repo", SUBMIT_FORK_REPO_JSON)], &[]);
    let origin = format!("http://{addr}/owner/repo");
    git_in(
        &checkout,
        &["config", "--local", "remote.origin.url", origin.as_str()],
    );
    let out = geectl_isolated(&dir)
        .current_dir(&checkout)
        .args([
            "pull-request",
            "submit",
            "--repo",
            "other/repo",
            "--base",
            "main",
            "--title",
            "Update foo",
        ])
        .env("GEECTL_GITEA_URL", format!("http://{addr}"))
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("invalid_input"), "stderr: {stderr}");
    assert!(stderr.contains("not the parent"), "stderr: {stderr}");
}

#[test]
fn pull_request_submit_gitea_missing_base_is_invalid_input() {
    let dir = TempDir::new();
    let checkout = git_submit_checkout(&dir);
    // The fixture answers the parent resolution only: the base
    // branch read answers 404.
    let addr = serve_gitea_submit(&[("/api/v1/repos/owner/repo", SUBMIT_FORK_REPO_JSON)], &[]);
    let origin = format!("http://{addr}/owner/repo");
    git_in(
        &checkout,
        &["config", "--local", "remote.origin.url", origin.as_str()],
    );
    let out = geectl_isolated(&dir)
        .current_dir(&checkout)
        .args([
            "pull-request",
            "submit",
            "--base",
            "nope",
            "--title",
            "Update foo",
        ])
        .env("GEECTL_GITEA_URL", format!("http://{addr}"))
        .env("GEECTL_GITEA_TOKEN", "test-token")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("invalid_input"), "stderr: {stderr}");
    assert!(
        stderr.contains("does not exist on pool/repo"),
        "stderr: {stderr}"
    );
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
fn gitea_maintainers_without_credentials_fails_with_config_error() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["gitea", "maintainers", "pool", "systemd"])
        .output()
        .unwrap();
    // ConfigError exits 9; a `gitea:` read needs the Bearer <redacted>
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
fn build_results_all_instances_conflicts_with_oldstate() {
    // A state hash is per instance, so the merged read refuses it:
    // clap refuses the combination as a parse error (exit 2) before
    // any work runs.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "build",
            "results",
            "openSUSE:Factory",
            "--all-instances",
            "--oldstate",
            "abc123",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("all-instances"), "stderr: {stderr}");
    assert!(stderr.contains("oldstate"), "stderr: {stderr}");
}

#[test]
fn build_results_all_instances_without_credentials_fails_with_config_error() {
    // One configured instance only: the merged read reports that
    // instance's own failure unchanged (exit 9, the missing
    // credentials named).
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["build", "results", "openSUSE:Factory", "--all-instances"])
        .output()
        .unwrap();
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
fn build_results_all_instances_merges_two_stub_instances() {
    // The merged read itself (`results_all_instances`) against two
    // stub instances: the default instance answers one matrix, the
    // `[instances]` entry another, and one view carries both — text
    // and JSON alike.
    let factory = serve_many("<resultlist state=\"aaa\"><result project=\"testproject\" repository=\"standard\" arch=\"x86_64\" code=\"published\" state=\"published\"><status package=\"foo\" code=\"succeeded\" /></result></resultlist>");
    let local = serve_many("<resultlist state=\"bbb\"><result project=\"testproject\" repository=\"standard\" arch=\"x86_64\" code=\"published\" state=\"published\"><status package=\"foo\" code=\"failed\" /></result></resultlist>");
    let dir = TempDir::new();
    let config_dir = dir.0.join("geectl");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        format!("[instances]\nlocal = \"http://{local}\"\n"),
    )
    .unwrap();
    let run = |format: &str| {
        geectl_isolated(&dir)
            .args([
                "--format",
                format,
                "build",
                "results",
                "testproject",
                "--all-instances",
            ])
            .env("GEECTL_OBS_URL", format!("http://{factory}"))
            .env("GEECTL_OBS_USERNAME", "factory-user")
            .env("GEECTL_OBS_PASSWORD", "factory-secret")
            .env("GEECTL_OBS_USERNAME_LOCAL", "local-user")
            .env("GEECTL_OBS_PASSWORD_LOCAL", "local-secret")
            .output()
            .unwrap()
    };
    let out = run("text");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!(
            "project: testproject\ninstance: default (http://{factory})\nREPOSITORY  ARCH    PACKAGE  CODE\nstandard    x86_64  foo      succeeded\n1 succeeded\nstate: aaa\ninstance: local (http://{local})\nREPOSITORY  ARCH    PACKAGE  CODE\nstandard    x86_64  foo      failed\n1 failed\nstate: bbb"
        )
    );
    let out = run("json");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["project"], "testproject");
    assert_eq!(value["instances"][0]["alias"], "default");
    assert_eq!(
        value["instances"][0]["api_url"],
        format!("http://{factory}")
    );
    assert_eq!(value["instances"][0]["state"], "aaa");
    assert_eq!(
        value["instances"][0]["results"][0]["statuses"][0]["code"],
        "succeeded"
    );
    assert_eq!(value["instances"][1]["alias"], "local");
    assert_eq!(value["instances"][1]["api_url"], format!("http://{local}"));
    assert_eq!(value["instances"][1]["state"], "bbb");
    assert_eq!(
        value["instances"][1]["results"][0]["statuses"][0]["code"],
        "failed"
    );
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
fn build_log_single_positional_that_is_not_a_url_fails_with_invalid_input() {
    // A single positional must be a build-log URL: a bare project
    // name fails in resolution (exit 2, invalid_input) before any
    // credential or network work.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["build", "build-log", "openSUSE:Factory"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert!(
        value["error"]["context"]
            .as_str()
            .unwrap()
            .contains("not a build-log URL"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn build_log_url_with_coordinates_fails_with_invalid_input() {
    // A pasted URL takes no other coordinates.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "build",
            "build-log",
            "https://api.opensuse.org/build/p/r/a/k/_log",
            "k",
            "r",
            "a",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert!(
        value["error"]["context"]
            .as_str()
            .unwrap()
            .contains("no other coordinates"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn build_log_multibuild_on_flavored_package_fails_with_invalid_input() {
    // --multibuild appends a flavor; a package that already carries
    // one refuses instead of gaining a second.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "build",
            "build-log",
            "p",
            "k:flavor1",
            "r",
            "a",
            "--multibuild",
            "flavor2",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert!(
        value["error"]["context"]
            .as_str()
            .unwrap()
            .contains("already carries a flavor"),
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
fn build_wipe_binaries_dry_run_ignores_half_set_credentials() {
    // A dry-run mutation preview needs no credentials — the
    // library's dry-run guard runs before its credentials check —
    // so it runs with a half-set env and no server at all.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["build", "wipe-binaries", "openSUSE:Factory", "--all"])
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
    assert_eq!(value["all"], true);
    assert_eq!(
        value["plan"][0],
        "[dry-run] would: wipe binaries in project openSUSE:Factory (all)"
    );
}

#[test]
fn build_wipe_binaries_without_scope_fails_with_invalid_input() {
    // An unscoped wipe would delete every binary in the project, so
    // the leaf refuses it before the dry-run guard.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["build", "wipe-binaries", "openSUSE:Factory"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
}

#[test]
fn build_wipe_binaries_all_with_status_filter_fails_with_invalid_input() {
    // `all` combines with no status filter: the combination is
    // refused instead of silently dropping the filter.
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args([
            "build",
            "wipe-binaries",
            "openSUSE:Factory",
            "--all",
            "--build-failed",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
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

// --- Docs no-help-blocks guard, alias credentials,
// supersede guard, whole-project guard, output-format default ---

#[test]
fn docs_carry_no_verbatim_help_output() {
    // The docs carry a per-command reference, not the binary's --help
    // output: verbatim blocks drifted with nothing verifying them, so
    // they are gone. This test guards the removal — a `Usage: geectl`
    // line anywhere in docs/ fails here. The binary's --help is the
    // source of truth for the full text.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let docs_dir = manifest.join("docs");
    for entry in fs::read_dir(&docs_dir).expect("docs/") {
        let path = entry.expect("docs entry").path();
        if path.extension().is_some_and(|e| e == "md") {
            let text = fs::read_to_string(&path).expect("read doc");
            assert!(
                !text.contains("Usage: geectl"),
                "{} carries verbatim --help output; the docs reference commands in one line each and point at `geectl <group> --help` instead",
                path.display()
            );
        }
    }
}

#[test]
fn local_build_chroot_dry_run_reads_commands_from_stdin() {
    // The dry run prints the plan — the resolved root plus the
    // piped commands — and touches nothing: no server call, no
    // root, no credentials. The blank line in the piped input is
    // dropped, not run.
    use std::io::Write;
    let dir = TempDir::new();
    let source = dir.0.join("source");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("foo.spec"), "Name: foo\n").unwrap();
    let mut child = geectl_isolated(&dir)
        .args([
            "--format",
            "text",
            "local-build",
            "chroot",
            source.to_str().unwrap(),
            "--project",
            "testproject",
            "--repo",
            "standard",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"echo hi\n\nrpm -qa\n").unwrap();
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    // The root prints with the platform-native separator, so
    // normalize before matching a forward-slash path fragment.
    let slashes = text.replace('\\', "/");
    assert!(
        slashes.contains("build-roots/testproject/foo/standard-"),
        "stdout: {text}"
    );
    assert!(text.contains("run: echo hi"), "stdout: {text}");
    assert!(text.contains("run: rpm -qa"), "stdout: {text}");
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
        "[instances]\nexample = \"https://api.opensuse.org\"\n",
    )
    .unwrap();
    let out = geectl_isolated(&dir)
        .args(["config", "show"])
        .env("GEECTL_OBS_USERNAME_EXAMPLE", "target-user")
        .env("GEECTL_OBS_PASSWORD_EXAMPLE", "target-secret")
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
    assert_eq!(value["instances"]["example"], "https://api.opensuse.org");
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
        "[instances]\nexample = \"https://api.opensuse.org\"\nlocal = \"http://127.0.0.1:3001\"\n",
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
    assert_eq!(value["instances"]["example"], "https://api.opensuse.org");
    assert_eq!(value["instances"]["local"], "http://127.0.0.1:3001");
    // And in text form, as `alias -> url` pairs sorted by alias.
    let out = geectl_isolated(&dir)
        .args(["--format", "text", "config", "show"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(
            "instances: example -> https://api.opensuse.org, local -> http://127.0.0.1:3001"
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

// --- changelog validate / changelog generate integration tests ---

/// A package directory fixture: spec + `.changes` (+ optional NEWS
/// tarball for the 2.0 version). `changes_entry` is the body of the
/// newest entry.
fn changelog_fixture(dir: &TempDir, changes_entry: &str, with_news: bool) -> PathBuf {
    let pkg = dir.0.join("foo");
    fs::create_dir_all(&pkg).unwrap();
    fs::write(
        pkg.join("foo.spec"),
        "Name:           foo\nVersion:        2.0\nLicense:        GPL-3.0-only\nSource0:        foo-2.0.tar.gz\nPatch0:         new.patch\nBuildRequires:  pkgconfig\n\n%changelog\n",
    )
    .unwrap();
    let sep = "-".repeat(67);
    fs::write(
        pkg.join("foo.changes"),
        format!(
            "{sep}\nWed May 27 16:31:48 UTC 2026 - Jane Packager <jane@example.com>\n\n{changes_entry}\n\n{sep}\nTue May 26 10:00:00 UTC 2026 - Jane Packager <jane@example.com>\n\n- Initial package\n\n"
        ),
    )
    .unwrap();
    fs::write(pkg.join("new.patch"), "new patch content\n").unwrap();
    if with_news {
        let news_dir = pkg.join("foo-2.0");
        fs::create_dir_all(&news_dir).unwrap();
        fs::write(
            news_dir.join("NEWS"),
            "## 1.0\n\n- Old news\n\n## 2.0\n\n- Fix a crash in the parser\n- Add a new subcommand\n",
        )
        .unwrap();
        let status = std::process::Command::new("tar")
            .args(["-czf", "foo-2.0.tar.gz", "foo-2.0"])
            .current_dir(&pkg)
            .status()
            .unwrap();
        assert!(status.success());
    }
    pkg
}

#[test]
fn changelog_validate_clean_fixture_exits_zero() {
    let dir = TempDir::new();
    let pkg = changelog_fixture(
        &dir,
        "- Update to 2.0\n- Fix a crash in the parser (boo#1234567)",
        false,
    );
    let out = geectl_isolated(&dir)
        .args(["changelog", "validate"])
        .arg(&pkg)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(value["verdict"], "pass");
    assert!(value["findings"].as_array().unwrap().is_empty());
    // The text rendering opens with the coordinate header and ends
    // with the verdict line.
    let out = geectl_isolated(&dir)
        .args(["--format", "text", "changelog", "validate"])
        .arg(&pkg)
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("dir: "));
    assert!(text.contains("OK: clean"));
}

#[test]
fn changelog_validate_bare_update_with_news_exits_24() {
    let dir = TempDir::new();
    let pkg = changelog_fixture(&dir, "- Update to 2.0", true);
    let out = geectl_isolated(&dir)
        .args(["changelog", "validate"])
        .arg(&pkg)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(24));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "validation_failed");
    let context = value["error"]["context"].as_str().unwrap();
    assert!(context.contains("V9"), "context: {context}");
    assert!(context.contains("foo.changes:4:"), "context: {context}");
}

#[test]
fn changelog_validate_missing_dir_fails_with_invalid_input() {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir)
        .args(["changelog", "validate"])
        .arg(dir.0.join("no-such-package"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
}

/// Replace the header line's date-time with a fixed placeholder, so two
/// runs stamped from separate `SystemTime::now()` reads compare equal
/// across a UTC second/minute/hour/day boundary. The author is kept.
fn normalize_header_time(entry: &str) -> String {
    let mut out = String::with_capacity(entry.len());
    for line in entry.split_inclusive('\n') {
        let stripped = line.strip_suffix('\n').unwrap_or(line);
        match parse_header(stripped) {
            Some(header) => {
                out.push_str(&format!("HEADER - {}", header.author));
                if line.ends_with('\n') {
                    out.push('\n');
                }
            }
            None => out.push_str(line),
        }
    }
    out
}

#[test]
fn changelog_generate_prints_draft_with_news_slots() {
    // The bump shape: the spec is at 2.0 while the newest
    // `.changes` entry is still at 1.0, so the version is new and
    // the NEWS section is extracted as a highlights slot.
    let dir = TempDir::new();
    let pkg = changelog_fixture(&dir, "- Update to 1.0", true);
    let out = geectl_isolated(&dir)
        .args(["changelog", "generate"])
        .arg(&pkg)
        .args(["--author", "Jane Packager <jane@example.com>"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(value["needs_detail"], true);
    assert_eq!(value["news_found"], true);
    let entry = value["entry"].as_str().unwrap();
    assert!(entry.contains("- Update to 2.0"));
    assert!(entry.contains("TODO"));
    assert!(entry.contains("Fix a crash in the parser"));
    // The text rendering is the entry itself, verbatim.
    let out = geectl_isolated(&dir)
        .args(["--format", "text", "changelog", "generate"])
        .arg(&pkg)
        .args(["--author", "Jane Packager <jane@example.com>"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    // The two runs stamp the header from separate `SystemTime::now()`
    // reads, so the seconds can differ by one across a UTC second
    // boundary; the timestamps must still be within a second.
    let json_header = entry
        .lines()
        .find_map(parse_header)
        .expect("generated entry carries a header");
    let text_header = text
        .lines()
        .find_map(parse_header)
        .expect("generated entry carries a header");
    assert!((json_header.timestamp - text_header.timestamp).abs() <= 1);
    // The text rendering is the entry itself, verbatim, apart from the
    // wall-clock time in the header line.
    assert_eq!(normalize_header_time(&text), normalize_header_time(entry));
}

#[test]
fn changelog_generate_write_refuses_incomplete_draft() {
    // The bump shape (see `changelog_generate_prints_draft_with_news_slots`):
    // the draft carries the NEWS highlights slot, so `--write` refuses it.
    let dir = TempDir::new();
    let pkg = changelog_fixture(&dir, "- Update to 1.0", true);
    let before = fs::read_to_string(pkg.join("foo.changes")).unwrap();
    let out = geectl_isolated(&dir)
        .args(["changelog", "generate"])
        .arg(&pkg)
        .args(["--author", "Jane Packager <jane@example.com>", "--write"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(24));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "validation_failed");
    assert!(value["error"]["context"]
        .as_str()
        .unwrap()
        .contains("refusing to write an incomplete entry"));
    // The file is untouched.
    assert_eq!(fs::read_to_string(pkg.join("foo.changes")).unwrap(), before);
}

#[test]
fn changelog_generate_write_refuses_slot_only_draft() {
    // No NEWS tarball: no detail is demanded, but the draft still
    // carries the no-news TODO slot — `--write` refuses it. (The
    // `--write` success path, a slot-free draft, needs a target diff
    // and is covered at the lib level by
    // `generate_license_only_change_has_no_slots`.) The fixture is
    // the bump shape, so the slot exists at all.
    let dir = TempDir::new();
    let pkg = changelog_fixture(&dir, "- Update to 1.0", false);
    let before = fs::read_to_string(pkg.join("foo.changes")).unwrap();
    let out = geectl_isolated(&dir)
        .args(["changelog", "generate"])
        .arg(&pkg)
        .args(["--author", "Jane Packager <jane@example.com>", "--write"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(24));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "validation_failed");
    assert!(
        value["error"]["context"]
            .as_str()
            .unwrap()
            .contains("unfilled TODO slot"),
        "context: {}",
        value["error"]["context"]
    );
    // The file is untouched.
    assert_eq!(fs::read_to_string(pkg.join("foo.changes")).unwrap(), before);
}

#[test]
fn changelog_generate_message_hotfix_carries_only_the_message() {
    // The hotfix shape: the spec is at 2.0 and the newest
    // `.changes` entry already documents 2.0, so the version is
    // unchanged — no version line, no summarize slot. The message
    // is the whole entry.
    let dir = TempDir::new();
    let pkg = changelog_fixture(&dir, "- Update to 2.0", false);
    let out = geectl_isolated(&dir)
        .args(["changelog", "generate"])
        .arg(&pkg)
        .args([
            "--author",
            "Jane Packager <jane@example.com>",
            "-m",
            "Fix the parser crash",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["needs_detail"], false);
    assert_eq!(value["slots"], 0);
    let entry = value["entry"].as_str().unwrap();
    assert!(entry.contains("- Fix the parser crash"));
    assert!(!entry.contains("Update to"));
    assert!(!entry.contains("TODO"));
}

#[test]
fn changelog_generate_message_hotfix_skips_news() {
    // The hotfix shape with upstream NEWS: the version is
    // unchanged, so no NEWS is extracted — no highlights slot, no
    // version line, no detail demanded. The message is the entry.
    // (The bump shape, where the message replaces the highlights
    // slot, is pinned at the lib level by
    // `message_replaces_the_highlights_slot_when_news_exists`.)
    let dir = TempDir::new();
    let pkg = changelog_fixture(&dir, "- Update to 2.0", true);
    let out = geectl_isolated(&dir)
        .args(["changelog", "generate"])
        .arg(&pkg)
        .args([
            "--author",
            "Jane Packager <jane@example.com>",
            "-m",
            "Rework the parser error path",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(value["needs_detail"], false);
    assert_eq!(value["news_found"], false);
    assert_eq!(value["slots"], 0);
    let entry = value["entry"].as_str().unwrap();
    assert!(entry.contains("- Rework the parser error path"));
    assert!(!entry.contains("Update to"));
    assert!(!entry.contains("Upstream highlights"));
    assert!(!entry.contains("Fix a crash"));
    assert!(!entry.contains("TODO"));
}

#[test]
fn changelog_generate_hotfix_message_write_prepends_and_validates() {
    // The hotfix shape, end to end: the written entry is just the
    // message — no version line, no NEWS — and it validates clean.
    let dir = TempDir::new();
    let pkg = changelog_fixture(&dir, "- Update to 2.0", true);
    let before = fs::read_to_string(pkg.join("foo.changes")).unwrap();
    let out = geectl_isolated(&dir)
        .args(["changelog", "generate"])
        .arg(&pkg)
        .args([
            "--author",
            "Jane Packager <jane@example.com>",
            "-m",
            "fix bug123",
            "--write",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let after = fs::read_to_string(pkg.join("foo.changes")).unwrap();
    assert!(after.ends_with(&before));
    let written = parse_changes(&after);
    let first = written.entries.first().expect("a written entry");
    let bullets: Vec<String> = first
        .bullets()
        .into_iter()
        .map(|(_, text, _)| text)
        .collect();
    assert_eq!(bullets, vec!["- fix bug123"]);
    // The written entry validates clean on its own.
    let out = geectl_isolated(&dir)
        .args(["changelog", "validate"])
        .arg(&pkg)
        .output()
        .unwrap();
    assert!(out.status.success());
}

#[test]
fn changelog_generate_hotfix_without_message_is_nothing_to_generate() {
    // The hotfix shape without a message or a target diff: there
    // is nothing to generate, so the command refuses loudly
    // instead of printing a bare header.
    let dir = TempDir::new();
    let pkg = changelog_fixture(&dir, "- Update to 2.0", true);
    let out = geectl_isolated(&dir)
        .args(["changelog", "generate"])
        .arg(&pkg)
        .args(["--author", "Jane Packager <jane@example.com>"])
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
            .contains("nothing to generate"),
        "context: {}",
        value["error"]["context"]
    );
}

#[test]
fn changelog_generate_message_write_prepends_and_validates() {
    let dir = TempDir::new();
    let pkg = changelog_fixture(&dir, "- Update to 2.0", false);
    let before = fs::read_to_string(pkg.join("foo.changes")).unwrap();
    let out = geectl_isolated(&dir)
        .args(["changelog", "generate"])
        .arg(&pkg)
        .args([
            "--author",
            "Jane Packager <jane@example.com>",
            "-m",
            "Fix the parser crash",
            "--write",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let after = fs::read_to_string(pkg.join("foo.changes")).unwrap();
    assert!(after.contains("- Fix the parser crash"));
    assert!(after.ends_with(&before));
    // The written entry validates clean on its own.
    let out = geectl_isolated(&dir)
        .args(["changelog", "validate"])
        .arg(&pkg)
        .output()
        .unwrap();
    assert!(out.status.success());
}

#[test]
fn changelog_generate_message_multiline_and_repeatable() {
    let dir = TempDir::new();
    let pkg = changelog_fixture(&dir, "- Update to 2.0", false);
    let out = geectl_isolated(&dir)
        .args(["changelog", "generate"])
        .arg(&pkg)
        .args([
            "--author",
            "Jane Packager <jane@example.com>",
            "-m",
            "First line\nsecond line",
            "-m",
            "Second bullet",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    let entry = value["entry"].as_str().unwrap();
    assert!(entry.contains("- First line\n  second line\n"));
    assert!(entry.contains("- Second bullet\n"));
}

#[test]
fn changelog_generate_message_empty_is_refused() {
    let dir = TempDir::new();
    let pkg = changelog_fixture(&dir, "- Update to 2.0", false);
    let out = geectl_isolated(&dir)
        .args(["changelog", "generate"])
        .arg(&pkg)
        .args(["--author", "Jane Packager <jane@example.com>", "-m", ""])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert!(value["error"]["context"]
        .as_str()
        .unwrap()
        .contains("--message must carry text"));
}

#[test]
fn changelog_target_flags_go_together() {
    let dir = TempDir::new();
    let pkg = changelog_fixture(&dir, "- Update to 2.0", false);
    let out = geectl_isolated(&dir)
        .args(["changelog", "validate"])
        .arg(&pkg)
        .args(["--target-project", "openSUSE:Factory"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
}

#[test]
fn changelog_generate_without_author_fails() {
    let dir = TempDir::new();
    let pkg = changelog_fixture(&dir, "- Update to 2.0", false);
    let out = geectl_isolated(&dir)
        .args(["changelog", "generate"])
        .arg(&pkg)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
}

#[test]
fn changelog_validate_entries_flag_lints_older_entries() {
    // The older entry opens with a `* ` bullet at the top level: the
    // default `--entries 1` lints only the newest entry (pass), while
    // `--entries 2` lints it too (exit 24).
    let dir = TempDir::new();
    let pkg = dir.0.join("foo");
    fs::create_dir_all(&pkg).unwrap();
    fs::write(
        pkg.join("foo.spec"),
        "Name:           foo\nVersion:        2.0\nLicense:        GPL-3.0-only\n\n%changelog\n",
    )
    .unwrap();
    let sep = "-".repeat(67);
    fs::write(
        pkg.join("foo.changes"),
        format!(
            "{sep}\nWed May 27 16:31:48 UTC 2026 - Jane Packager <jane@example.com>\n\n- Update to 2.0\n\n{sep}\nTue May 26 10:00:00 UTC 2026 - Jane Packager <jane@example.com>\n\n* Bad top star\n\n"
        ),
    )
    .unwrap();
    let out = geectl_isolated(&dir)
        .args(["changelog", "validate"])
        .arg(&pkg)
        .output()
        .unwrap();
    assert!(out.status.success());
    let out = geectl_isolated(&dir)
        .args(["changelog", "validate"])
        .arg(&pkg)
        .args(["--entries", "2"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(24));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "validation_failed");
    assert!(
        value["error"]["context"].as_str().unwrap().contains("V11"),
        "context: {}",
        value["error"]["context"]
    );
}

#[test]
fn prjconf_help_lists_the_subcommands() {
    let out = geectl().args(["prjconf", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    for name in ["show", "validate", "clean"] {
        assert!(text.contains(name), "prjconf --help misses {name}");
    }
}

#[test]
fn top_help_lists_the_prjconf_group() {
    let out = geectl().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("prjconf"),
        "top --help misses the prjconf group"
    );
}

#[test]
fn prjconf_validate_help_exits_zero() {
    let out = geectl()
        .args(["prjconf", "validate", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("geectl prjconf validate"), "{text}");
    assert!(text.contains("<PROJECT>"), "{text}");
}

#[test]
fn prjconf_clean_help_lists_the_path_dependent_opt_in() {
    let out = geectl()
        .args(["prjconf", "clean", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("--include-path-dependent"), "{text}");
}

/// The prjconf reads fail closed without OBS credentials: no
/// anonymous request is attempted.
fn prjconf_fails_closed_without_credentials(args: &[&str]) {
    let dir = TempDir::new();
    let out = geectl_isolated(&dir).args(args).output().unwrap();
    assert!(
        !out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.status.code(),
        Some(9),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stderr).trim()).unwrap();
    assert_eq!(value["error"]["code"], "config_error");
    assert!(value["error"]["context"]
        .as_str()
        .unwrap()
        .contains("without OBS credentials"));
}

#[test]
fn prjconf_show_fails_closed_without_credentials() {
    prjconf_fails_closed_without_credentials(&["prjconf", "show", "openSUSE:Factory"]);
}

#[test]
fn prjconf_validate_fails_closed_without_credentials() {
    prjconf_fails_closed_without_credentials(&["prjconf", "validate", "openSUSE:Factory"]);
}

#[test]
fn prjconf_clean_fails_closed_without_credentials() {
    prjconf_fails_closed_without_credentials(&["prjconf", "clean", "openSUSE:Factory"]);
}

// --- local-build log integration tests ---

/// Plant a build root's log under the isolated cache dir: the root
/// the build's own coordinates name, with a spec source dir naming
/// the package. Returns the source dir.
fn local_build_log_fixture(dir: &TempDir) -> PathBuf {
    let source = dir.0.join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("foo.spec"), "Name: foo\n").unwrap();
    let root = dir
        .0
        .join("geectl")
        .join("build-roots")
        // The library percent-encodes `:` in cache paths, so the
        // fixture plants the encoded name the log read looks up.
        .join("home%3Afoo")
        .join("foo")
        .join(format!("standard-{}", std::env::consts::ARCH));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join(".build.log"),
        "first\nfinished \"build foo.spec\"\n",
    )
    .unwrap();
    source
}

#[test]
fn local_build_log_prints_the_last_build_log() {
    let dir = TempDir::new();
    let source = local_build_log_fixture(&dir);
    let arch = std::env::consts::ARCH;
    // Text is the log verbatim (design rule 4): no header line.
    let out = geectl_isolated(&dir)
        .env("XDG_CACHE_HOME", &dir.0)
        .args([
            "--format",
            "text",
            "local-build",
            "log",
            "standard",
            arch,
            source.to_str().unwrap(),
            "--project",
            "home:foo",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "first\nfinished \"build foo.spec\"\n"
    );
    // The tail arm prints only the last lines.
    let out = geectl_isolated(&dir)
        .env("XDG_CACHE_HOME", &dir.0)
        .args([
            "--format",
            "text",
            "local-build",
            "log",
            "standard",
            arch,
            source.to_str().unwrap(),
            "--project",
            "home:foo",
            "--tail",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "finished \"build foo.spec\"\n"
    );
    // JSON (the default format) carries the record: the lines, the
    // truncation flag, and the verdict (null: no build record here).
    let out = geectl_isolated(&dir)
        .env("XDG_CACHE_HOME", &dir.0)
        .args([
            "local-build",
            "log",
            "standard",
            arch,
            source.to_str().unwrap(),
            "--project",
            "home:foo",
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
    assert_eq!(value["project"], "home:foo");
    assert_eq!(value["repo"], "standard");
    assert_eq!(value["arch"], arch);
    assert_eq!(value["log"]["truncated"], false);
    assert_eq!(value["log"]["verdict"], serde_json::Value::Null);
    assert_eq!(
        value["log"]["lines"],
        serde_json::json!(["first", "finished \"build foo.spec\""])
    );
}

#[test]
fn local_build_log_missing_log_fails_with_invalid_input() {
    // A source whose package never built: no log file, so the read
    // fails closed naming the log path.
    let dir = TempDir::new();
    let source = dir.0.join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("foo.spec"), "Name: foo\n").unwrap();
    let out = geectl_isolated(&dir)
        .env("XDG_CACHE_HOME", &dir.0)
        .args([
            "local-build",
            "log",
            "standard",
            std::env::consts::ARCH,
            source.to_str().unwrap(),
            "--project",
            "home:foo",
        ])
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
            .contains("no build log at"),
        "stderr: {stderr}"
    );
}
