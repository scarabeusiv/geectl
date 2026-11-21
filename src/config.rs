//! XDG config file: endpoint URLs only, never credentials.
//!
//! The config file lives at `$XDG_CONFIG_HOME/geectl/config.toml`
//! (default `~/.config/geectl/config.toml`). It holds only the two
//! endpoint URLs (`obs_url`, `gitea_url`). Credentials are never
//! stored here: a key that looks like a credential is a refusal to
//! run, naming the key, the file, and the environment variable to use
//! instead. (This is the plaintext-fallback failure the design doc's
//! risk analysis names: a config file must never become a silent
//! credential store.)
//!
//! Writes go through [`secure_write`]: the file is created 0600 from
//! the first byte (unix), via a unique temp file with `O_EXCL`, fsync,
//! rename, then a directory fsync — the secure state is reached at
//! creation, not at a later chmod. There is no option to disable
//! certificate verification, anywhere: the HTTP client always
//! verifies, and no config key can change that.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::auth::{
    open_file_nonblocking, snapshot_env, GITEA_SSH_KEY_VAR, GITEA_TOKEN_FILE_VAR, GITEA_TOKEN_VAR,
    OBS_PASSWORD_FILE_VAR, OBS_PASSWORD_VAR, OBS_USERNAME_VAR,
};
use crate::error::{ErrorKind, GeectlError};
use crate::http::is_loopback_url;
use crate::url::UrlBuilder;
use std::collections::HashMap;

/// Environment variable overriding the OBS endpoint URL.
pub const OBS_URL_VAR: &str = "GEECTL_OBS_URL";
/// Environment variable overriding the Gitea endpoint URL.
pub const GITEA_URL_VAR: &str = "GEECTL_GITEA_URL";

/// The built-in OBS endpoint (the OBS API, not the web UI).
pub const DEFAULT_OBS_URL: &str = "https://api.opensuse.org";
/// The built-in Gitea endpoint (src.opensuse.org serves its API at
/// the same host).
pub const DEFAULT_GITEA_URL: &str = "https://src.opensuse.org";

/// The effective (non-secret) configuration: endpoint URLs only.
/// A field is `None` when neither the config file nor the environment
/// set it; [`effective_config`] fills in the built-in defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub obs_url: Option<String>,
    pub gitea_url: Option<String>,
}

/// The effective endpoints plus where each value came from. The
/// source is computed once, here, with the values — callers must not
/// re-derive it by re-reading the environment (a second precedence
/// copy drifts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveConfig {
    pub config: Config,
    pub obs_url_source: &'static str,
    pub gitea_url_source: &'static str,
}

/// Read one endpoint URL from a snapshot (see [`snapshot_env`]):
/// `None` when unset or empty (after trimming). Non-UTF-8 values
/// are already refused by the snapshot, so this cannot fail.
fn env_url(env: &HashMap<String, String>, var: &str) -> Option<String> {
    env.get(var)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Validate an endpoint URL value (from the config file or the
/// environment): require `https://` with a host, reject userinfo
/// (`@` in the authority) and a `?` or `#` in the base. A userinfo
/// URL would carry a credential into the endpoint and echo it from
/// `config show`; a `?`/`#` breaks `UrlBuilder` joining. Returns
/// the trimmed, validated URL. `source` names the value in errors
/// (a config key or an env var) — never the value itself.
fn validate_endpoint_url(text: &str, source: &str) -> Result<String, GeectlError> {
    let text = text.trim();
    // Leading/trailing whitespace is trimmed above by design, but
    // whitespace or a control character inside the URL is never
    // legitimate — reject it explicitly rather than relying on the
    // URL parser to catch every shape.
    if text.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("{source}: endpoint URL must not contain whitespace or control characters"),
        ));
    }
    // An http endpoint is allowed only to a loopback host — the same
    // rule the transport guard enforces before any credential is sent
    // (see require_credentials_transport): cleartext is safe only on
    // loopback, never to a remote host. Anything else must be https.
    // is_loopback_url is reused verbatim, so its bracketed-host
    // hardening applies here too: `http://[::1]evil.com` is not
    // loopback and stays rejected.
    let http_loopback = text.starts_with("http://") && is_loopback_url(text);
    if http_loopback {
        // Same host check as UrlBuilder, minus the scheme rule.
        let after_scheme = &text["http://".len()..];
        let host_end = after_scheme
            .find(['/', '?', '#'])
            .unwrap_or(after_scheme.len());
        if after_scheme[..host_end].is_empty() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!("{source}: URL base must have a host"),
            ));
        }
    } else {
        UrlBuilder::new(text).map_err(|e| {
            GeectlError::new(ErrorKind::ConfigError, format!("{source}: {}", e.context))
        })?;
    }
    // The authority ends at the first `/`, `?`, or `#`. A `@` inside
    // it is userinfo (a credential in the endpoint).
    let after_scheme = text.split("://").nth(1).unwrap_or("");
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    if after_scheme[..authority_end].contains('@') {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("{source}: endpoint URL must not contain userinfo ('@')"),
        ));
    }
    if text.contains('?') || text.contains('#') {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("{source}: endpoint URL must not contain '?' or '#'"),
        ));
    }
    Ok(text.to_string())
}

/// The effective endpoints: env vars (`GEECTL_OBS_URL` /
/// `GEECTL_GITEA_URL`) win over the config file, which wins over the
/// built-in defaults. URLs only — never a secret. An env value is
/// validated like a file value (see [`validate_endpoint_url`]); a bad
/// one is a `ConfigError` naming the variable.
pub fn effective_config(config: &Config) -> Result<EffectiveConfig, GeectlError> {
    // One snapshot for both URLs: the two resolutions see the same
    // environment.
    let env = snapshot_env(&[OBS_URL_VAR, GITEA_URL_VAR])?;
    let resolve = |var: &str, from_file: &Option<String>, default: &str| {
        if let Some(value) = env_url(&env, var) {
            let value = validate_endpoint_url(&value, var)?;
            return Ok((value, "environment"));
        }
        if let Some(value) = from_file {
            // Validated like the env value: parse_config validates
            // too, but effective_config takes any Config, and a value
            // built without the parse path must not slip through.
            let value = validate_endpoint_url(value, "config file")?;
            return Ok((value, "config"));
        }
        Ok((default.to_string(), "default"))
    };
    let (obs_url, obs_url_source) = resolve(OBS_URL_VAR, &config.obs_url, DEFAULT_OBS_URL)?;
    let (gitea_url, gitea_url_source) =
        resolve(GITEA_URL_VAR, &config.gitea_url, DEFAULT_GITEA_URL)?;
    Ok(EffectiveConfig {
        config: Config {
            obs_url: Some(obs_url),
            gitea_url: Some(gitea_url),
        },
        obs_url_source,
        gitea_url_source,
    })
}

/// The two candidate config paths: the XDG one (only when
/// `XDG_CONFIG_HOME` is set and absolute — a relative value is
/// ignored per the XDG spec) and the `~/.config` default (only when
/// `HOME` is absolute — a relative HOME would make a CWD-relative
/// config path).
fn candidate_paths(
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> (Option<PathBuf>, Option<PathBuf>) {
    let xdg = xdg_config_home
        .filter(|p| Path::new(p).is_absolute())
        .map(|p| PathBuf::from(p).join("geectl").join("config.toml"));
    let default = home.filter(|p| Path::new(p).is_absolute()).map(|h| {
        PathBuf::from(h)
            .join(".config")
            .join("geectl")
            .join("config.toml")
    });
    (xdg, default)
}

/// Resolve the config file path from the environment. The XDG path
/// wins when set; if both candidates exist and differ, that is an
/// error — two live config files with one answer is a
/// misconfiguration, and silently picking one would hide it. A
/// missing file is not an error: `load_config` treats it as empty.
pub fn config_path() -> Result<PathBuf, GeectlError> {
    let env = snapshot_env(&["XDG_CONFIG_HOME", "HOME"])?;
    let xdg = env.get("XDG_CONFIG_HOME").map(String::as_str);
    let home = env.get("HOME").map(String::as_str);
    let (xdg_path, default_path) = candidate_paths(xdg, home);
    let xdg_exists = xdg_path.as_ref().is_some_and(|p| p.exists());
    let default_exists = default_path.as_ref().is_some_and(|p| p.exists());
    resolve_config_path(xdg, home, xdg_exists, default_exists)
}

/// Pure path resolution, for tests: given the environment values and
/// whether each candidate file exists, pick the config path.
pub fn resolve_config_path(
    xdg_config_home: Option<&str>,
    home: Option<&str>,
    xdg_file_exists: bool,
    default_file_exists: bool,
) -> Result<PathBuf, GeectlError> {
    let (xdg_path, default_path) = candidate_paths(xdg_config_home, home);
    if let (Some(x), Some(d)) = (&xdg_path, &default_path) {
        if x != d && xdg_file_exists && default_file_exists {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!(
                    "two geectl config files found ({} and {}); remove one",
                    x.display(),
                    d.display()
                ),
            ));
        }
    }
    if let Some(x) = xdg_path {
        return Ok(x);
    }
    if let Some(d) = default_path {
        return Ok(d);
    }
    Err(GeectlError::new(
        ErrorKind::ConfigError,
        "cannot locate a config directory: set XDG_CONFIG_HOME or HOME",
    ))
}

/// Load the config file if it exists; a missing file is an empty
/// config, not an error.
pub fn load_config() -> Result<Config, GeectlError> {
    let path = config_path()?;
    load_config_at(&path)
}

/// Load the config file at `path` (a missing file is an empty config).
/// The file must be a regular file: a FIFO or device at the path is
/// refused, not read. The largest config file accepted (64 KiB, the
/// same cap as secret files). A config is two URL lines; anything
/// larger is a misconfiguration, not a config.
pub const MAX_CONFIG_FILE_BYTES: usize = 64 * 1024;

pub fn load_config_at(path: &Path) -> Result<Config, GeectlError> {
    use std::io::Read;
    // The open is non-blocking on unix (see `open_file_nonblocking`
    // in auth.rs), so a FIFO at the path cannot block the open; the
    // regular-file check below refuses it before any byte is read.
    let file = match open_file_nonblocking(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!("{}: cannot read config: {e}", path.display()),
            ));
        }
    };
    match file.metadata() {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!("{}: not a regular file", path.display()),
            ));
        }
        Err(e) => {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!("{}: cannot read config: {e}", path.display()),
            ));
        }
    }
    // Read at most MAX + 1 bytes, so a file that grows while being
    // read is still capped.
    let mut bytes = Vec::new();
    if let Err(e) = file
        .take((MAX_CONFIG_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
    {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("{}: cannot read config: {e}", path.display()),
        ));
    }
    if bytes.len() > MAX_CONFIG_FILE_BYTES {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!(
                "{}: config exceeds {MAX_CONFIG_FILE_BYTES} bytes",
                path.display()
            ),
        ));
    }
    let text = String::from_utf8(bytes).map_err(|_| {
        GeectlError::new(
            ErrorKind::ConfigError,
            format!("{}: config is not valid UTF-8", path.display()),
        )
    })?;
    parse_config(&text, path)
}

/// The (1-based) line and column of a byte offset in `text`, for
/// a parse error position. No content is included — only the
/// position.
fn toml_position(text: &str, offset: usize) -> String {
    let before = text.get(..offset).unwrap_or(text);
    let line = before.matches('\n').count() + 1;
    let col = before.len() - before.rfind('\n').map(|i| i + 1).unwrap_or(0) + 1;
    format!(" at line {line}, column {col}")
}

/// Parse config TOML. Only `obs_url` / `gitea_url` are recognised;
/// anything else is an error (a typo must not silently do nothing),
/// and a credential-shaped key anywhere in the file is a refusal to
/// run. URL values must be `https://` with a host — validated here,
/// at load time, not at the first request.
pub fn parse_config(text: &str, path: &Path) -> Result<Config, GeectlError> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| {
        // The parser error embeds the offending line, which could be
        // a mistyped credential pasted into the config. Report the
        // path and the position only — never the parser snippet.
        let position = e
            .span()
            .map(|span| toml_position(text, span.start))
            .unwrap_or_default();
        GeectlError::new(
            ErrorKind::ConfigError,
            format!("{}: invalid TOML{position}", path.display()),
        )
    })?;
    check_no_credentials(&table, path)?;
    for key in table.keys() {
        if key != "obs_url" && key != "gitea_url" {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!("{}: unknown config key '{key}'", path.display()),
            ));
        }
    }
    let mut config = Config::default();
    if let Some(value) = table.get("obs_url") {
        config.obs_url = Some(as_url_string(value, "obs_url", path)?);
    }
    if let Some(value) = table.get("gitea_url") {
        config.gitea_url = Some(as_url_string(value, "gitea_url", path)?);
    }
    Ok(config)
}

/// A URL value must be a string and a usable `https://` endpoint
/// (see [`validate_endpoint_url`]).
fn as_url_string(value: &toml::Value, key: &str, path: &Path) -> Result<String, GeectlError> {
    let text = value.as_str().ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ConfigError,
            format!("{}: '{key}' must be a string", path.display()),
        )
    })?;
    validate_endpoint_url(text, &format!("{}: '{key}'", path.display()))
}

/// Credential-shaped keys, refused anywhere in the config file: an
/// exact match, or a key ending in `_<name>` — so `api_key`,
/// `obs_password` and `gitea_token` all match. The match is
/// case-insensitive and normalizes separators and case: `api-key`,
/// `apiKey`, `APIKey`, `TOKEN`, and plurals like `tokens` all count.
const CREDENTIAL_KEYS: &[&str] = &[
    "password",
    "passwd",
    "pass",
    "passphrase",
    "token",
    "api_key",
    "apikey",
    "secret",
    "credentials",
    "credential",
    "bearer_token",
    "access_token",
    "private_key",
    "client_secret",
];

/// Split a camelCase key into `_`-separated lowercase words:
/// a hump starts a new word when the previous character is lowercase
/// or a digit (`apiKey` -> `api_key`), or when an uppercase run is
/// followed by a lowercase letter — the last capital of the run
/// starts the next word (`APIKey` -> `api_key`). A single pass that
/// inserts `_` before every uppercase letter would mangle all-caps
/// keys (`TOKEN` -> `t_o_k_e_n`), so the flat case-insensitive
/// match in [`is_credential_key`] runs first and this is only the
/// second stage.
fn split_camel_case(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    let mut out = String::with_capacity(key.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == '-' {
            out.push('_');
        } else if c.is_uppercase() {
            if i > 0 {
                let prev = chars[i - 1];
                let next_is_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
                if prev.is_lowercase()
                    || prev.is_ascii_digit()
                    || (prev.is_uppercase() && next_is_lower)
                {
                    out.push('_');
                }
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn matches_credential_key(normalized: &str) -> bool {
    // Also try the singular, for the plural forms (`tokens`,
    // `passwords`).
    let singular = normalized.strip_suffix('s').unwrap_or(normalized);
    CREDENTIAL_KEYS.iter().any(|name| {
        normalized == *name
            || normalized.ends_with(&format!("_{name}"))
            || singular == *name
            || singular.ends_with(&format!("_{name}"))
    })
}

fn is_credential_key(key: &str) -> bool {
    // Stage 1: case-insensitive, hyphen-insensitive — the original
    // rule (`TOKEN`, `Api_Key`), extended to hyphens (`api-key`).
    let flat = key.to_ascii_lowercase().replace('-', "_");
    if matches_credential_key(&flat) {
        return true;
    }
    // Stage 2: camelCase humps become separators (`apiKey` ->
    // `api_key`, `APIKey` -> `api_key`).
    let split = split_camel_case(key);
    split != flat && matches_credential_key(&split)
}

/// Recursively refuse credential-shaped keys anywhere in the table
/// (nested tables and tables inside arrays included). The error names
/// the key, the file, and the environment variable to use instead —
/// never the value.
fn check_no_credentials(table: &toml::Table, path: &Path) -> Result<(), GeectlError> {
    for (key, value) in table {
        if is_credential_key(key) {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!(
                    "{}: credential-shaped key '{key}' is not allowed in the config file; pass credentials via {} instead",
                    path.display(),
                    credential_env_hint(key),
                ),
            ));
        }
        match value {
            toml::Value::Table(inner) => check_no_credentials(inner, path)?,
            toml::Value::Array(items) => {
                for item in items {
                    if let toml::Value::Table(inner) = item {
                        check_no_credentials(inner, path)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Which environment variable to use for a refused key, by name.
/// The hint is approximate (a bare `token` is most likely Gitea's,
/// a bare `password` most likely OBS's); the exact variable is the
/// caller's to confirm.
fn credential_env_hint(key: &str) -> String {
    let key = key.to_ascii_lowercase();
    if key.contains("private_key") || key.contains("ssh") {
        return GITEA_SSH_KEY_VAR.to_string();
    }
    if key.contains("gitea") {
        return format!("{GITEA_TOKEN_VAR} (or {GITEA_TOKEN_FILE_VAR})");
    }
    if key.contains("obs") {
        return format!("{OBS_USERNAME_VAR} + {OBS_PASSWORD_VAR} (or {OBS_PASSWORD_FILE_VAR})");
    }
    if key.contains("token") || key.contains("api_key") || key.contains("apikey") {
        return format!("{GITEA_TOKEN_VAR} (or {GITEA_TOKEN_FILE_VAR})");
    }
    if key.contains("password") || key.contains("passwd") || key.contains("pass") {
        return format!("{OBS_USERNAME_VAR} + {OBS_PASSWORD_VAR} (or {OBS_PASSWORD_FILE_VAR})");
    }
    format!("the {OBS_USERNAME_VAR} / {GITEA_TOKEN_VAR} environment variables")
}

/// The template `config init` writes: endpoints only, with the
/// credential rule spelled out in a comment.
pub const CONFIG_TEMPLATE: &str = r#"# geectl configuration — endpoint URLs only.
#
# Credentials are NEVER stored here. Pass them via the environment
# (or mounted secret files):
#   OBS:   GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD
#          (or GEECTL_OBS_PASSWORD_FILE)
#   Gitea: GEECTL_GITEA_TOKEN (or GEECTL_GITEA_TOKEN_FILE)
#          GEECTL_GITEA_SSH_KEY for git operations
# A credential-shaped key in this file is a refusal to run.
obs_url = "https://api.opensuse.org"
gitea_url = "https://src.opensuse.org"
"#;

/// Write the template to `path`, refusing if the file already exists
/// (never clobber a user's config).
pub fn write_config_template(path: &Path) -> Result<(), GeectlError> {
    if path.exists() {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!(
                "{} already exists; refusing to overwrite it",
                path.display()
            ),
        ));
    }
    // The exists check is the clear error for the common case;
    // the hard link inside `secure_write_new` is the atomic guarantee
    // against a concurrent creator in between.
    secure_write_new(path, CONFIG_TEMPLATE.as_bytes())
}

/// Create a unique temp file next to `path`, write `contents`
/// into it (0600 from the first byte on unix, `O_EXCL`), and fsync
/// it. Returns the temp path; on failure the temp file is removed.
/// The caller publishes it (rename or hard link) and fsyncs the
/// directory. A killed process never leaves a world-readable file
/// behind. The parent directory is created 0700 (unix) when missing.
fn write_secure_temp(path: &Path, contents: &[u8]) -> Result<PathBuf, GeectlError> {
    let parent = path.parent().ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ConfigError,
            format!("{}: no parent directory", path.display()),
        )
    })?;
    ensure_dir_0700(parent)?;
    let mut last_error: Option<std::io::Error> = None;
    for _ in 0..10 {
        let tmp = unique_tmp_path(path);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        match options.open(&tmp) {
            Ok(mut file) => {
                if let Err(e) = file.write_all(contents).and_then(|()| file.sync_all()) {
                    let _ = fs::remove_file(&tmp);
                    return Err(io_error(path, e));
                }
                drop(file);
                return Ok(tmp);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                last_error = Some(e);
                break;
            }
        }
    }
    let reason = last_error
        .map(|e| e.to_string())
        .unwrap_or_else(|| "all temp file names collided".to_string());
    Err(GeectlError::new(
        ErrorKind::ConfigError,
        format!(
            "{}: cannot create a secure temp file: {reason}",
            path.display()
        ),
    ))
}

/// Fsync the directory containing `path`, so a publish into it is
/// durable. A failure to open or fsync the directory is ignored:
/// the file itself is already fsynced, and a directory fsync
/// failure must not fail the write.
fn fsync_parent_dir(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
}

/// Write bytes to a file atomically and securely: the file is created
/// 0600 from the first byte (unix), via a unique temp file in the
/// same directory (`O_EXCL`, random suffix), fsynced, renamed into
/// place, and the directory fsynced. The temp file gets the same 0600
/// mode, and a failed write removes it — a killed process never
/// leaves a world-readable file behind. A rename overwrites an
/// existing destination; for create-new semantics see
/// [`secure_write_new`].
pub fn secure_write(path: &Path, contents: &[u8]) -> Result<(), GeectlError> {
    let tmp = write_secure_temp(path, contents)?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(io_error(path, e));
    }
    fsync_parent_dir(path);
    Ok(())
}

/// Write bytes atomically and securely, refusing if the destination
/// already exists. Same 0600-from-first-byte, temp+fsync flow as
/// [`secure_write`], but the final publish is a hard link (which
/// fails when the destination exists) instead of a rename (which
/// would overwrite it). The link is atomic: two concurrent creators
/// cannot both succeed, so `config init` never clobbers a config
/// another process just created.
pub fn secure_write_new(path: &Path, contents: &[u8]) -> Result<(), GeectlError> {
    let tmp = write_secure_temp(path, contents)?;
    match fs::hard_link(&tmp, path) {
        Ok(()) => {
            let _ = fs::remove_file(&tmp);
            fsync_parent_dir(path);
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                return Err(GeectlError::new(
                    ErrorKind::ConfigError,
                    format!(
                        "{} already exists; refusing to overwrite it",
                        path.display()
                    ),
                ));
            }
            Err(io_error(path, e))
        }
    }
}

fn io_error(path: &Path, err: std::io::Error) -> GeectlError {
    GeectlError::new(ErrorKind::ConfigError, format!("{}: {err}", path.display()))
}

/// Create the directory (and parents) with 0700 on unix when it does
/// not exist. An existing directory is left alone — its mode is the
/// user's to manage, and the file inside is 0600 regardless.
fn ensure_dir_0700(dir: &Path) -> Result<(), GeectlError> {
    if dir.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|e| io_error(dir, e))?;
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
    }
    Ok(())
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A unique temp path in the same directory as `path`: dot-prefixed
/// (hidden), with the pid, a nanosecond timestamp, and a counter.
/// Uniqueness is a belt-and-braces measure — `O_EXCL` in
/// [`secure_write`] is what makes a pre-existing path fail instead of
/// being followed.
fn unique_tmp_path(path: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".to_string());
    path.with_file_name(format!(".{name}.tmp-{pid}-{nanos}-{n}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_path() -> PathBuf {
        PathBuf::from("/x/geectl/config.toml")
    }

    #[test]
    fn xdg_path_wins_when_set() {
        assert_eq!(
            resolve_config_path(Some("/xdg"), Some("/home/u"), false, false).unwrap(),
            PathBuf::from("/xdg/geectl/config.toml")
        );
    }

    #[test]
    fn default_path_when_xdg_unset() {
        assert_eq!(
            resolve_config_path(None, Some("/home/u"), false, false).unwrap(),
            PathBuf::from("/home/u/.config/geectl/config.toml")
        );
    }

    #[test]
    fn relative_xdg_is_ignored() {
        // A relative XDG_CONFIG_HOME is invalid per the XDG spec.
        assert_eq!(
            resolve_config_path(Some("relative/xdg"), Some("/home/u"), false, false).unwrap(),
            PathBuf::from("/home/u/.config/geectl/config.toml")
        );
    }

    #[test]
    fn two_existing_files_are_an_error() {
        let err = resolve_config_path(Some("/xdg"), Some("/home/u"), true, true).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("/xdg/geectl/config.toml"));
        assert!(err.context.contains("/home/u/.config/geectl/config.toml"));
    }

    #[test]
    fn one_existing_file_is_not_ambiguous() {
        // Only the default exists: the XDG path still wins (it is the
        // location), and the default file is simply not the config.
        assert_eq!(
            resolve_config_path(Some("/xdg"), Some("/home/u"), false, true).unwrap(),
            PathBuf::from("/xdg/geectl/config.toml")
        );
        assert_eq!(
            resolve_config_path(Some("/xdg"), Some("/home/u"), true, false).unwrap(),
            PathBuf::from("/xdg/geectl/config.toml")
        );
    }

    #[test]
    fn no_home_is_an_error() {
        let err = resolve_config_path(None, None, false, false).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
    }

    #[test]
    fn valid_config_parses() {
        let config = parse_config(
            "obs_url = \"https://api.opensuse.org\"\ngitea_url = \"https://src.opensuse.org\"\n",
            &config_path(),
        )
        .unwrap();
        assert_eq!(config.obs_url.as_deref(), Some("https://api.opensuse.org"));
        assert_eq!(
            config.gitea_url.as_deref(),
            Some("https://src.opensuse.org")
        );
    }

    #[test]
    fn empty_config_parses_to_defaults() {
        assert_eq!(parse_config("", &config_path()).unwrap(), Config::default());
        assert_eq!(
            parse_config("# only a comment\n", &config_path()).unwrap(),
            Config::default()
        );
    }

    #[test]
    fn credential_key_is_refused() {
        let err = parse_config("password = \"s3cr3t\"\n", &config_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("password"));
        assert!(err.context.contains("GEECTL_OBS_PASSWORD"));
        // The value never appears in the error.
        assert!(!err.context.contains("s3cr3t"));
    }

    #[test]
    fn credential_key_suffix_is_refused() {
        // `obs_password`, `gitea_token`, `api_key`: the `_<name>` form.
        for key in [
            "obs_password",
            "gitea_token",
            "api_key",
            "TOKEN",
            "obs_passwd",
            "passphrase",
        ] {
            let text = format!("{key} = \"x\"\n");
            let err = parse_config(&text, &config_path()).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ConfigError, "key {key}");
            assert!(err.context.contains(key), "key {key}");
        }
    }

    #[test]
    fn credential_key_in_nested_table_is_refused() {
        let err = parse_config("[obs]\npassword = \"x\"\n", &config_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("password"));
    }

    #[test]
    fn unknown_key_is_an_error() {
        let err =
            parse_config("obz_url = \"https://api.opensuse.org\"\n", &config_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("obz_url"));
    }

    #[test]
    fn non_string_url_is_an_error() {
        let err = parse_config("obs_url = 42\n", &config_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("obs_url"));
    }

    #[test]
    fn http_url_is_an_error() {
        // An http endpoint would send credentials in cleartext.
        let err =
            parse_config("obs_url = \"http://api.opensuse.org\"\n", &config_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("obs_url"));
    }

    #[test]
    fn hostless_url_is_an_error() {
        let err = parse_config("obs_url = \"https://\"\n", &config_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
    }

    #[test]
    fn invalid_toml_is_an_error() {
        let err = parse_config("obs_url = \n", &config_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
    }

    /// Counter for temp dir names: the pid plus a nanosecond
    /// timestamp alone collide when two tests run in the same
    /// nanosecond, so each call also takes a distinct counter value.
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
                "geectl-config-test-{}-{nanos}-{n}",
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

    #[test]
    fn secure_write_creates_0600_and_no_temp_leftovers() {
        let dir = TempDir::new();
        let path = dir.0.join("config.toml");
        secure_write(&path, b"obs_url = \"https://api.opensuse.org\"\n").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "obs_url = \"https://api.opensuse.org\"\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "config file must be 0600 from the first byte");
        }
        // No temp file is left behind in the directory.
        let entries: Vec<_> = fs::read_dir(&dir.0).unwrap().collect();
        assert_eq!(entries.len(), 1, "only the config file should exist");
    }

    #[test]
    fn secure_write_replaces_atomically() {
        let dir = TempDir::new();
        let path = dir.0.join("config.toml");
        secure_write(&path, b"first\n").unwrap();
        secure_write(&path, b"second\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn config_template_refuses_to_overwrite() {
        let dir = TempDir::new();
        let path = dir.0.join("config.toml");
        write_config_template(&path).unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("obs_url"));
        let err = write_config_template(&path).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        // The existing file is untouched.
        assert!(fs::read_to_string(&path).unwrap().contains("obs_url"));
    }

    #[test]
    fn load_config_at_missing_file_is_empty() {
        let dir = TempDir::new();
        assert_eq!(
            load_config_at(&dir.0.join("missing.toml")).unwrap(),
            Config::default()
        );
    }

    #[test]
    fn load_config_at_reads_a_real_file() {
        let dir = TempDir::new();
        let path = dir.0.join("config.toml");
        secure_write(&path, b"gitea_url = \"https://src.opensuse.org\"\n").unwrap();
        let config = load_config_at(&path).unwrap();
        assert_eq!(
            config.gitea_url.as_deref(),
            Some("https://src.opensuse.org")
        );
        assert_eq!(config.obs_url, None);
    }

    #[test]
    #[cfg(unix)]
    fn load_config_at_does_not_block_on_a_fifo() {
        // A FIFO with no writer blocks a plain open forever; the
        // non-blocking open returns at once, and the regular-file
        // check refuses it before reading.
        let dir = TempDir::new();
        let path = dir.0.join("config.toml");
        let status = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .expect("mkfifo must be available to create the test FIFO");
        assert!(status.success(), "mkfifo must create the test FIFO");
        let err = load_config_at(&path).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(
            err.context.contains("not a regular file"),
            "{}",
            err.context
        );
    }

    #[test]
    fn load_config_at_rejects_an_oversize_file() {
        let dir = TempDir::new();
        let path = dir.0.join("config.toml");
        secure_write(&path, &vec![b'x'; MAX_CONFIG_FILE_BYTES + 1]).unwrap();
        let err = load_config_at(&path).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("exceeds"), "{}", err.context);
    }

    #[test]
    fn endpoint_url_validation_rejects_userinfo_and_markers() {
        // A userinfo URL would carry a credential into the endpoint.
        for bad in [
            "https://user:secret@api.opensuse.org",
            "https://user@api.opensuse.org",
            "https://api.opensuse.org?x=1",
            "https://api.opensuse.org#frag",
            "https://api.opensuse.org/path?x=1",
        ] {
            let err = validate_endpoint_url(bad, "obs_url").unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ConfigError, "url {bad}");
        }
        // http and hostless are rejected by the UrlBuilder check.
        assert!(validate_endpoint_url("http://api.opensuse.org", "obs_url").is_err());
        assert!(validate_endpoint_url("https://", "obs_url").is_err());
        // http to a loopback host is allowed — the same rule the
        // transport guard enforces (see require_credentials_transport):
        // the local Gitea instance lives at http://127.0.0.1:3001.
        for ok in [
            "http://127.0.0.1:3001",
            "http://localhost:3001",
            "http://[::1]:3001",
            "http://127.0.0.1:3001/base",
        ] {
            assert_eq!(
                validate_endpoint_url(ok, "obs_url").unwrap(),
                ok,
                "url {ok}"
            );
        }
        // ...but loopback does not excuse userinfo, and a bracketed
        // host that is not really loopback stays rejected.
        assert!(validate_endpoint_url("http://user:secret@127.0.0.1:3001", "obs_url").is_err());
        assert!(validate_endpoint_url("http://[::1]evil.com", "obs_url").is_err());
        // A bare https host (with an optional path) is fine.
        assert_eq!(
            validate_endpoint_url("https://api.opensuse.org", "obs_url").unwrap(),
            "https://api.opensuse.org"
        );
        assert_eq!(
            validate_endpoint_url("https://api.opensuse.org/base", "obs_url").unwrap(),
            "https://api.opensuse.org/base"
        );
    }

    #[test]
    fn endpoint_url_rejects_embedded_whitespace_and_control() {
        // Leading/trailing whitespace is trimmed by design (see
        // endpoint_url_is_trimmed); whitespace or control characters
        // inside the URL are rejected.
        for bad in [
            "https://api.open suse.org",
            "https://api.opensuse.org/pa th",
            "https://api.opensuse.org\u{7f}",
            "https://api.opensuse.org\u{0}",
        ] {
            let err = validate_endpoint_url(bad, "obs_url").unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ConfigError, "url {bad:?}");
            assert!(err.context.contains("whitespace"), "{}", err.context);
        }
    }

    #[test]
    fn endpoint_url_is_trimmed() {
        // Leading/trailing whitespace is stored trimmed, not passed
        // on to fail confusingly at request time.
        assert_eq!(
            validate_endpoint_url("  https://api.opensuse.org  \n", "obs_url").unwrap(),
            "https://api.opensuse.org"
        );
        let config = parse_config(
            "obs_url = \"  https://api.opensuse.org  \"\n",
            &config_path(),
        )
        .unwrap();
        assert_eq!(config.obs_url.as_deref(), Some("https://api.opensuse.org"));
    }

    #[test]
    fn effective_config_revalidates_file_values() {
        // A Config built without going through parse_config must
        // not smuggle an unvalidated URL past effective_config:
        // the endpoint is validated at the point of use, not only
        // at parse time.
        let config = Config {
            obs_url: Some("https://user:secret@api.opensuse.org".to_string()),
            gitea_url: None,
        };
        let err = effective_config(&config).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(!err.context.contains("secret"));
        // A valid file value still flows through.
        let config = Config {
            obs_url: Some("https://api.opensuse.org".to_string()),
            gitea_url: None,
        };
        let effective = effective_config(&config).unwrap();
        assert_eq!(
            effective.config.obs_url.as_deref(),
            Some("https://api.opensuse.org")
        );
        assert_eq!(effective.obs_url_source, "config");
    }

    #[test]
    fn file_url_with_userinfo_is_an_error() {
        let err = parse_config(
            "obs_url = \"https://user:secret@api.opensuse.org\"\n",
            &config_path(),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("obs_url"));
        // The credential never appears in the error.
        assert!(!err.context.contains("secret"));
    }

    #[test]
    fn toml_parse_error_names_position_not_content() {
        // A mistyped credential line must not be echoed into the error.
        let text = "obs_url = \"https://api.opensuse.org\"\nfoo = \"s3cr3t-value\"\nbar = \n";
        let err = parse_config(text, &config_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("invalid TOML"), "{}", err.context);
        assert!(err.context.contains("line"), "{}", err.context);
        assert!(!err.context.contains("s3cr3t-value"), "{}", err.context);
        assert!(!err.context.contains("foo"), "{}", err.context);
    }

    #[test]
    fn credential_key_matcher_covers_hyphen_camel_and_plural() {
        for key in [
            "api-key",
            "apiKey",
            "tokens",
            "passwords",
            "obs-passwords",
            "giteaToken",
            // All-caps and acronym forms must be refused as
            // credential-shaped, not fall through to "unknown key":
            // a single-pass camelCase splitter mangles them.
            "TOKEN",
            "API_KEY",
            "APIKey",
            "Api_Key",
        ] {
            let text = format!("{key} = \"x\"\n");
            let err = parse_config(&text, &config_path()).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ConfigError, "key {key}");
            assert!(err.context.contains(key), "key {key}");
            assert!(
                err.context.contains("credential-shaped"),
                "key {key} was not refused as credential-shaped: {}",
                err.context
            );
        }
        // Non-credential keys still fail as unknown, not as refused.
        let err = parse_config("api_version = \"1\"\n", &config_path()).unwrap_err();
        assert!(err.context.contains("unknown config key"));
    }

    #[test]
    fn relative_home_is_ignored() {
        // A relative HOME must not make a CWD-relative config path.
        assert_eq!(
            resolve_config_path(None, Some("relative/home"), false, false)
                .unwrap_err()
                .kind(),
            ErrorKind::ConfigError
        );
        // With an absolute XDG set, the relative HOME is irrelevant.
        assert_eq!(
            resolve_config_path(Some("/xdg"), Some("relative/home"), false, false).unwrap(),
            PathBuf::from("/xdg/geectl/config.toml")
        );
    }

    #[test]
    fn secure_write_new_refuses_an_existing_destination() {
        let dir = TempDir::new();
        let path = dir.0.join("config.toml");
        secure_write_new(&path, b"first\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "first\n");
        let err = secure_write_new(&path, b"second\n").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("already exists"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "first\n");
    }
}
