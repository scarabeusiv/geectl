//! XDG config file: endpoint URLs and the output-format default,
//! never credentials.
//!
//! The config file lives at `$XDG_CONFIG_HOME/geectl/config.toml`
//! (default `~/.config/geectl/config.toml`). It holds only the two
//! endpoint URLs (`obs_url`, `gitea_url`), the output-format
//! default (`format`, `json` or `text`), and the named-instance
//! table (`[instances]`, alias → URL — see [`Config::instances`]).
//! Credentials are never
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
    open_file_nonblocking, same_api_url, snapshot_env, GITEA_SSH_KEY_VAR, GITEA_TOKEN_FILE_VAR,
    GITEA_TOKEN_VAR, OBS_PASSWORD_FILE_VAR, OBS_PASSWORD_VAR, OBS_USERNAME_VAR,
};
use crate::error::{ErrorKind, GeectlError};
use crate::http::is_loopback_url;
use crate::url::UrlBuilder;
use std::collections::{HashMap, HashSet};

/// Environment variable overriding the OBS endpoint URL.
pub const OBS_URL_VAR: &str = "GEECTL_OBS_URL";
/// Environment variable overriding the Gitea endpoint URL.
pub const GITEA_URL_VAR: &str = "GEECTL_GITEA_URL";
/// Environment variable overriding the output-format default.
pub const FORMAT_VAR: &str = "GEECTL_FORMAT";

/// The built-in OBS endpoint (the OBS API, not the web UI).
pub const DEFAULT_OBS_URL: &str = "https://api.opensuse.org";
/// The built-in Gitea endpoint (src.opensuse.org serves its API at
/// the same host).
pub const DEFAULT_GITEA_URL: &str = "https://src.opensuse.org";

/// The effective (non-secret) configuration: endpoint URLs and
/// the output-format default. A field is `None` when neither the
/// config file nor the environment set it; [`effective_config`]
/// fills in the built-in defaults. The `[build]` table carries
/// the local-build vertical's tunables; both are `None` when the
/// table is absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub obs_url: Option<String>,
    pub gitea_url: Option<String>,
    pub format: Option<String>,
    /// Named instances from the config file's `[instances]`
    /// table, sorted by alias: `(alias, url)` pairs, e.g.
    /// `example = "https://api.opensuse.org"`. The alias
    /// names the instance's credential variables
    /// (`GEECTL_OBS_APIKEY[_FILE]_<ALIAS>` or
    /// `GEECTL_OBS_USERNAME_<ALIAS>` and friends — see
    /// [`crate::auth::resolve_obs_for_url`] — and
    /// `GEECTL_GITEA_TOKEN[_FILE]_<ALIAS>` — see
    /// [`crate::auth::resolve_gitea_bearer_for_url`]); the table
    /// itself carries no secret, only the alias → URL mapping. The
    /// shape is backend-neutral (alias → URL, not OBS- or
    /// Gitea-specific). The first alias whose URL matches a call's
    /// URL (under [`crate::auth::same_api_url`]) wins. Two aliases
    /// for the same URL are refused at load (see
    /// [`as_instances_table`]).
    pub instances: Vec<(String, String)>,
    /// The local-build cache directory (`$XDG_CACHE_HOME/geectl/`
    /// by default). A relative path resolves against the current
    /// directory at use time; an empty value is refused.
    pub build_cache_dir: Option<String>,
    /// The local-build cache cap in GiB (20 by default). Zero is
    /// refused: a zero cap would evict everything at every build.
    pub build_cache_cap_gib: Option<u64>,
}

/// The effective endpoints and output format, plus where each
/// value came from. The source is computed once, here, with the
/// values — callers must not re-derive it by re-reading the
/// environment (a second precedence copy drifts). The `[instances]`
/// table rides through unchanged: it has no environment override
/// and no source to report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveConfig {
    pub config: Config,
    pub obs_url_source: &'static str,
    pub gitea_url_source: &'static str,
    pub format_source: &'static str,
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

/// The effective endpoints and output format: env vars
/// (`GEECTL_OBS_URL` / `GEECTL_GITEA_URL` / `GEECTL_FORMAT`) win
/// over the config file, which wins over the built-in defaults.
/// URLs only — never a secret. An env value is validated like a
/// file value (see [`validate_endpoint_url`]); a bad one is a
/// `ConfigError` naming the variable. The format value is validated
/// the same way (see [`validate_format`]).
pub fn effective_config(config: &Config) -> Result<EffectiveConfig, GeectlError> {
    // One snapshot for the URLs and the format: the resolutions see
    // the same environment.
    let env = snapshot_env(&[OBS_URL_VAR, GITEA_URL_VAR, FORMAT_VAR])?;
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
    let (format, format_source) = match env_url(&env, FORMAT_VAR) {
        Some(value) => (validate_format(&value, FORMAT_VAR)?, "environment"),
        None => match &config.format {
            // Validated like the env value: parse_config validates
            // too, but effective_config takes any Config, and a value
            // built without the parse path must not slip through.
            Some(value) => (validate_format(value, "config file")?, "config"),
            None => ("json".to_string(), "default"),
        },
    };
    Ok(EffectiveConfig {
        config: Config {
            obs_url: Some(obs_url),
            gitea_url: Some(gitea_url),
            format: Some(format),
            instances: config.instances.clone(),
            build_cache_dir: None,
            build_cache_cap_gib: None,
        },
        obs_url_source,
        gitea_url_source,
        format_source,
    })
}

/// Validate an output-format value (from the config file or the
/// environment): `json` or `text`, exactly. A format value is not a
/// secret, so the error names the value — the user typed it, and the
/// fix is to spell it right. `source` names where the value came
/// from (an env var or the config file).
fn validate_format(value: &str, source: &str) -> Result<String, GeectlError> {
    let value = value.trim();
    match value {
        "json" | "text" => Ok(value.to_string()),
        _ => Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("{source}: invalid output format {value:?}; expected \"json\" or \"text\""),
        )),
    }
}

/// The two candidate config paths: the XDG one (only when
/// `XDG_CONFIG_HOME` is set and absolute — a relative value is
/// ignored per the XDG spec) and the `~/.config` default (only when
/// `HOME` is absolute — a relative HOME would make a CWD-relative
/// config path). On Windows, `USERPROFILE` stands in for `HOME`.
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

/// The home directory for path resolution: `HOME`, except on
/// Windows, where Git Bash sets `HOME` to a POSIX-style path
/// (`/c/Users/u`) that is not a Windows-absolute path — there
/// `HOME` counts only when absolute, and `USERPROFILE` is the
/// fallback.
pub(crate) fn home_dir<'a>(home: Option<&'a str>, userprofile: Option<&'a str>) -> Option<&'a str> {
    #[cfg(windows)]
    {
        home.filter(|h| Path::new(*h).is_absolute()).or(userprofile)
    }
    #[cfg(not(windows))]
    {
        let _ = userprofile;
        home
    }
}

/// Resolve the config file path from the environment. The XDG path
/// wins when set; if both candidates exist and differ, that is an
/// error — two live config files with one answer is a
/// misconfiguration, and silently picking one would hide it. A
/// missing file is not an error: `load_config` treats it as empty.
pub fn config_path() -> Result<PathBuf, GeectlError> {
    let env = snapshot_env(&[
        "XDG_CONFIG_HOME",
        "HOME",
        #[cfg(windows)]
        "USERPROFILE",
    ])?;
    let xdg = env.get("XDG_CONFIG_HOME").map(String::as_str);
    let home = home_dir(
        env.get("HOME").map(String::as_str),
        env.get("USERPROFILE").map(String::as_str),
    );
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

/// Parse config TOML. Only `obs_url` / `gitea_url` / `format` /
/// `instances` are recognised; anything else is an error (a typo
/// must not silently do nothing), and a credential-shaped key
/// anywhere in the file is a refusal to run. URL values must be
/// `https://` with a host — validated here, at load time, not at
/// the first request. The `format` value must be `json` or `text`.
/// The `[instances]` table maps an alias to an endpoint URL (see
/// [`as_instances_table`]).
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
        if key != "obs_url"
            && key != "gitea_url"
            && key != "format"
            && key != "instances"
            && key != "build"
        {
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
    if let Some(value) = table.get("format") {
        config.format = Some(as_format_string(value, "format", path)?);
    }
    if let Some(value) = table.get("instances") {
        config.instances = as_instances_table(value, path)?;
    }
    if let Some(value) = table.get("build") {
        let build = value.as_table().ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ConfigError,
                format!("{}: 'build' must be a table", path.display()),
            )
        })?;
        for key in build.keys() {
            if key != "cache_dir" && key != "cache_cap_gib" {
                return Err(GeectlError::new(
                    ErrorKind::ConfigError,
                    format!("{}: unknown config key 'build.{key}'", path.display()),
                ));
            }
        }
        if let Some(value) = build.get("cache_dir") {
            let text = value.as_str().ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ConfigError,
                    format!("{}: 'build.cache_dir' must be a string", path.display()),
                )
            })?;
            let text = text.trim();
            if text.is_empty() {
                return Err(GeectlError::new(
                    ErrorKind::ConfigError,
                    format!("{}: 'build.cache_dir' must not be empty", path.display()),
                ));
            }
            config.build_cache_dir = Some(text.to_string());
        }
        if let Some(value) = build.get("cache_cap_gib") {
            let cap = value.as_integer().ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ConfigError,
                    format!(
                        "{}: 'build.cache_cap_gib' must be a positive integer",
                        path.display()
                    ),
                )
            })?;
            if cap <= 0 {
                return Err(GeectlError::new(
                    ErrorKind::ConfigError,
                    format!(
                        "{}: 'build.cache_cap_gib' must be a positive integer",
                        path.display()
                    ),
                ));
            }
            config.build_cache_cap_gib = Some(cap as u64);
        }
    }
    Ok(config)
}

/// A format value must be a string and a valid output format (see
/// [`validate_format`]).
fn as_format_string(value: &toml::Value, key: &str, path: &Path) -> Result<String, GeectlError> {
    let text = value.as_str().ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ConfigError,
            format!("{}: '{key}' must be a string", path.display()),
        )
    })?;
    validate_format(text, &format!("{}: '{key}'", path.display()))
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

/// The `[instances]` table: alias → endpoint URL. The value must
/// be a table of strings and each alias like an env-var segment
/// (see [`validate_alias`]) — those are load errors. The URL
/// itself is not validated here, with one exception: a URL
/// carrying userinfo (`@` in the authority — a credential in the
/// URL) is refused at load. It can never resolve, and the raw
/// string would otherwise echo in full through the merged
/// results render (see [`crate::obs::InstanceBuildResults`]),
/// leaking the credential; tolerance for genuinely malformed
/// shapes is untouched. A malformed URL loads fine and fails
/// only when that instance is read, so one bad entry never
/// blocks the other instances from loading (a malformed entry
/// matches nothing under [`crate::auth::same_api_url`], and the
/// merged results read records the failure for that instance
/// alone). Only the scheme, host and effective port identify the
/// instance — a path in the URL is ignored by matching (see
/// [`crate::auth::same_api_url`]). The pairs come back sorted by
/// alias, so alias matching (see [`crate::auth::alias_for_url`])
/// is deterministic regardless of the file's key order.
fn as_instances_table(
    value: &toml::Value,
    path: &Path,
) -> Result<Vec<(String, String)>, GeectlError> {
    let table = value.as_table().ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ConfigError,
            format!("{}: 'instances' must be a table", path.display()),
        )
    })?;
    let mut seen_upper: HashSet<String> = HashSet::new();
    let mut instances = Vec::new();
    for (alias, url) in table {
        validate_alias(alias, &mut seen_upper, path)?;
        // The value must be a string, but the URL is not
        // validated here: a malformed URL loads fine and fails
        // only when that instance is read, so one bad entry
        // never blocks the other instances from loading.
        let text = url.as_str().ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ConfigError,
                format!("{}: '[instances] {alias}' must be a string", path.display()),
            )
        })?;
        let text = text.trim().to_string();
        // One shape is refused at load, not tolerated: userinfo
        // (`@` in the authority — a credential in the URL). It
        // can never resolve, and the loaded string would echo in
        // full through the merged results render. The URL itself
        // is never echoed here: the part before the `@` is a
        // credential. The authority ends at the first `/`, `?`,
        // or `#`, as in `validate_endpoint_url`.
        let after_scheme = text.split("://").nth(1).unwrap_or("");
        let authority_end = after_scheme
            .find(['/', '?', '#'])
            .unwrap_or(after_scheme.len());
        if after_scheme[..authority_end].contains('@') {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!(
                    "{}: '[instances] {alias}' URL must not contain userinfo ('@')",
                    path.display()
                ),
            ));
        }
        instances.push((alias.clone(), text));
    }
    instances.sort();
    // Two aliases for the same URL name the same instance twice:
    // resolution could only ever use the first one's variables,
    // so the duplicate is refused at load like the case
    // collision — name each instance once. A malformed URL
    // matches nothing under `same_api_url`, so it never
    // collides: the comparison failure is a non-match, not a
    // load failure.
    for (index, (alias, url)) in instances.iter().enumerate() {
        for (other_alias, other_url) in &instances[index + 1..] {
            if same_api_url(url, other_url).unwrap_or(false) {
                return Err(GeectlError::new(
                    ErrorKind::ConfigError,
                    format!(
                        "{}: instance aliases {alias:?} and {other_alias:?} name the same URL ({url}); name each instance once",
                        path.display()
                    ),
                ));
            }
        }
    }
    Ok(instances)
}

/// Validate one instance alias: non-empty, ASCII alphanumeric plus
/// underscore only — the alias becomes an env-var segment
/// (`GEECTL_OBS_USERNAME_<ALIAS>` / `GEECTL_GITEA_TOKEN_<ALIAS>`,
/// uppercased), so anything else cannot name a variable. Two
/// aliases that differ only by case name the same variables and
/// are refused too. `file` is reserved: it would collide with the
/// plain `*_FILE` variables (see below). A bad alias is a
/// `ConfigError` at load time naming the alias.
fn validate_alias(
    alias: &str,
    seen_upper: &mut HashSet<String>,
    path: &Path,
) -> Result<(), GeectlError> {
    if alias.is_empty() || !alias.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!(
                "{}: invalid instance alias {alias:?}; aliases must be ASCII alphanumeric plus underscore",
                path.display()
            ),
        ));
    }
    // `file` is reserved: `GEECTL_GITEA_TOKEN_<ALIAS>` with
    // `<ALIAS>` = `FILE` is `GEECTL_GITEA_TOKEN_FILE` — the plain
    // file variable — so one variable would mean two things
    // depending on which URL is called (the same holds for the
    // OBS password pair). It is the only such collision: the
    // `_FILE_<ALIAS>` form can never equal a plain variable, and
    // an empty alias is refused above.
    if alias.eq_ignore_ascii_case("file") {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!(
                "{}: instance alias {alias:?} is reserved; 'file' would collide with the plain GEECTL_GITEA_TOKEN_FILE / GEECTL_OBS_PASSWORD_FILE variables",
                path.display()
            ),
        ));
    }
    if !seen_upper.insert(alias.to_ascii_uppercase()) {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!(
                "{}: instance alias {alias:?} collides with another alias after uppercasing; aliases must differ after uppercasing",
                path.display()
            ),
        ));
    }
    Ok(())
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
/// never the value. The `[instances]` table is exempt: its keys are
/// aliases, not config keys, so an alias like `my_token` is valid
/// there even though `token` is a credential-shaped key.
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
            // The `[instances]` table's keys are aliases, not
            // config keys, so the refusal does not apply below it.
            toml::Value::Table(inner) if key != "instances" => check_no_credentials(inner, path)?,
            toml::Value::Table(_) => {}
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

/// The template `config init` writes: endpoints, the output-format
/// default, and named instances, with the credential rule spelled
/// out in a comment.
pub const CONFIG_TEMPLATE: &str = r#"# geectl configuration — endpoint URLs, the output-format default, and named instances.
#
# Credentials are NEVER stored here. Pass them via the environment
# (or mounted secret files):
#   OBS:   GEECTL_OBS_APIKEY (or GEECTL_OBS_APIKEY_FILE)
#          or GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD
#          (or GEECTL_OBS_PASSWORD_FILE)
#   Gitea: GEECTL_GITEA_TOKEN (or GEECTL_GITEA_TOKEN_FILE)
#          GEECTL_GITEA_SSH_KEY for git operations
# A credential-shaped key in this file is a refusal to run.
# A second instance gets one line in [instances] (alias → URL) and
# its own variables, with the alias uppercased:
#   [instances]
#   myinstance = "https://gitea.example.org"
#   GEECTL_GITEA_TOKEN_MYINSTANCE (or GEECTL_GITEA_TOKEN_FILE_MYINSTANCE)
# 'file' is reserved as an alias (it would collide with the plain
# GEECTL_GITEA_TOKEN_FILE variable).
obs_url = "https://api.opensuse.org"
gitea_url = "https://src.opensuse.org"
# The output format default: "json" (default) or "text". The
# --format flag wins over it, and GEECTL_FORMAT wins over this key.
# format = "json"
#
# Local builds (one podman vertical) keep their cache under
# $XDG_CACHE_HOME/geectl/ by default, capped at 20 GiB. Uncomment
# to override:
# [build]
# cache_dir = "/srv/geectl-cache"
# cache_cap_gib = 20
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

/// The outcome of adding an instance to the config file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddInstance {
    /// A new `[instances]` entry was written.
    Added,
    /// The alias already named the same URL; the file is untouched.
    AlreadyPresent,
    /// The alias existed with a different URL and `force` replaced it.
    Replaced,
}

/// Check whether an instance can be added: the alias and URL pass the
/// same validation the loader applies, the alias does not already
/// name another URL (unless `force`), and no other alias already
/// names this URL (the loader refuses two aliases for one URL).
/// Returns what a write would do. No file is touched.
pub fn check_add_instance(
    path: &Path,
    alias: &str,
    url: &str,
    force: bool,
) -> Result<AddInstance, GeectlError> {
    let mut seen_upper = HashSet::new();
    validate_alias(alias, &mut seen_upper, path)?;
    let url = validate_endpoint_url(url, &format!("{}: '[instances] {alias}'", path.display()))?;
    let current = load_config_at(path)?;
    if let Some((_, existing_url)) = current
        .instances
        .iter()
        .find(|(a, _)| a.eq_ignore_ascii_case(alias))
    {
        if same_api_url(existing_url, &url).unwrap_or(false) {
            return Ok(AddInstance::AlreadyPresent);
        }
        if !force {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!(
                    "{}: instance alias {alias:?} already names {existing_url}; pass --force to replace it",
                    path.display()
                ),
            ));
        }
        return Ok(AddInstance::Replaced);
    }
    if let Some((other, _)) = current
        .instances
        .iter()
        .find(|(a, u)| !a.eq_ignore_ascii_case(alias) && same_api_url(u, &url).unwrap_or(false))
    {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!(
                "{}: instance alias {other:?} already names {url}; use that alias instead of adding {alias:?}",
                path.display()
            ),
        ));
    }
    Ok(AddInstance::Added)
}

/// Add an instance to the config file's `[instances]` table: the
/// alias to URL mapping only, never a credential. The existing file
/// is validated through the loader first, so a malformed config
/// fails closed instead of gaining a spliced entry; the spliced text
/// is validated through the loader again before the atomic write, so
/// a splice bug never lands in the file. The rest of the file is
/// preserved apart from the inserted (or replaced) line — comments
/// and key order survive. A missing file gains a minimal
/// `[instances]` section only; endpoint lines are never invented.
pub fn add_instance(
    path: &Path,
    alias: &str,
    url: &str,
    force: bool,
) -> Result<AddInstance, GeectlError> {
    let outcome = check_add_instance(path, alias, url, force)?;
    if outcome == AddInstance::AlreadyPresent {
        return Ok(outcome);
    }
    let url = validate_endpoint_url(url, &format!("{}: '[instances] {alias}'", path.display()))?;
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(io_error(path, e)),
    };
    let new_text = splice_instance(&text, alias, &url, outcome == AddInstance::Replaced, path)?;
    // Validate the spliced text before writing: a splice bug must
    // fail here, never land in the file.
    parse_config(&new_text, path)?;
    secure_write(path, new_text.as_bytes())?;
    Ok(outcome)
}

/// Derive an instance alias from an endpoint URL: the host,
/// lowercased, with every character outside ASCII alphanumerics
/// replaced by `_`, so the alias is valid under [`validate_alias`]
/// and names the env-var segment (`GEECTL_OBS_USERNAME_<ALIAS>`).
/// A URL with no host is an error naming the URL.
pub fn alias_from_url(url: &str) -> Result<String, GeectlError> {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let host = if let Some(rest) = authority.strip_prefix('[') {
        // Bracketed host (IPv6): up to the closing bracket.
        rest.split(']').next().unwrap_or("")
    } else {
        // Strip a trailing `:port` (a port is all digits after the
        // last colon).
        match authority.rfind(':') {
            Some(i)
                if !authority[i + 1..].is_empty()
                    && authority[i + 1..].chars().all(|c| c.is_ascii_digit()) =>
            {
                &authority[..i]
            }
            _ => authority,
        }
    };
    let alias: String = host
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let alias = alias.trim_matches('_').to_string();
    if alias.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("cannot derive an instance alias from {url:?}; pass --alias"),
        ));
    }
    Ok(alias)
}

/// Splice one `[instances]` entry into config TOML text: insert
/// `alias = "url"` after the `[instances]` header, replace the
/// alias's line when `replace`, or append a new `[instances]`
/// section when the table is absent. The entry value is TOML-escaped
/// through the `toml` crate, never interpolated raw. An `[instances]`
/// entry written inline or with dotted keys (no section header) is
/// refused with a clear error: appending a section there would
/// duplicate the table, so the file needs the section form first.
fn splice_instance(
    text: &str,
    alias: &str,
    url: &str,
    replace: bool,
    path: &Path,
) -> Result<String, GeectlError> {
    let entry = format!("{alias} = {}", toml::Value::String(url.to_string()));
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    // Find the `[instances]` header: a line whose bracketed name is
    // `instances` (quotes stripped). A comment line never matches:
    // it does not start with `[`.
    let header = lines.iter().position(|line| {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('[') {
            return false;
        }
        let Some(rest) = trimmed.strip_prefix('[') else {
            return false;
        };
        let Some(inner) = rest.split(']').next() else {
            return false;
        };
        let name = inner.trim().trim_matches(|c| c == '"' || c == '\'');
        name == "instances"
    });
    match header {
        Some(h) => {
            // The section runs to the next header line or EOF.
            let end = lines[h + 1..]
                .iter()
                .position(|line| line.trim_start().starts_with('['))
                .map(|i| h + 1 + i)
                .unwrap_or(lines.len());
            if replace {
                let entry_line = lines[h + 1..end].iter().position(|line| {
                    entry_key(line).is_some_and(|key| key.eq_ignore_ascii_case(alias))
                });
                let Some(i) = entry_line else {
                    return Err(GeectlError::new(
                        ErrorKind::ConfigError,
                        format!(
                            "{}: cannot replace '[instances] {alias}': the entry is not a plain line in the section",
                            path.display()
                        ),
                    ));
                };
                lines[h + 1 + i] = entry;
            } else {
                lines.insert(h + 1, entry);
            }
        }
        None => {
            // No section header: the table must be absent entirely —
            // an inline or dotted `[instances]` would duplicate the
            // table if a section were appended.
            if !text.trim().is_empty() {
                let parsed = parse_config(text, path)?;
                if !parsed.instances.is_empty() {
                    return Err(GeectlError::new(
                        ErrorKind::ConfigError,
                        format!(
                            "{}: the '[instances]' table is written inline or with dotted keys; rewrite it as an [instances] section and retry",
                            path.display()
                        ),
                    ));
                }
                if !text.ends_with('\n') {
                    lines.push(String::new());
                }
                lines.push(String::new());
            }
            lines.push("[instances]".to_string());
            lines.push(entry);
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    Ok(out)
}

/// The key of an `[instances]` entry line (`alias = "url"`, quotes
/// stripped), or `None` for a comment, blank, or header line.
fn entry_key(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('[') {
        return None;
    }
    let before_eq = trimmed.split('=').next()?;
    let key = before_eq.trim();
    if key.is_empty() {
        return None;
    }
    let key = key.trim_matches(|c| c == '"' || c == '\'');
    Some(key.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_path() -> PathBuf {
        PathBuf::from("/x/geectl/config.toml")
    }

    /// Absolute fake homes for the path-resolution tests: unix
    /// paths on unix, drive-letter paths on Windows — a bare
    /// `/home/u` is not absolute on Windows.
    #[cfg(unix)]
    const TEST_HOME: &str = "/home/u";
    #[cfg(windows)]
    const TEST_HOME: &str = r"C:\Users\u";
    #[cfg(unix)]
    const TEST_XDG: &str = "/xdg";
    #[cfg(windows)]
    const TEST_XDG: &str = r"C:\xdg";

    fn home_config_path() -> PathBuf {
        PathBuf::from(TEST_HOME)
            .join(".config")
            .join("geectl")
            .join("config.toml")
    }

    fn xdg_config_path() -> PathBuf {
        PathBuf::from(TEST_XDG).join("geectl").join("config.toml")
    }

    #[test]
    fn home_dir_selection() {
        // An absolute HOME always wins.
        #[cfg(unix)]
        {
            assert_eq!(home_dir(Some("/home/u"), Some("/other")), Some("/home/u"));
            assert_eq!(home_dir(None, Some("/other")), None);
            assert_eq!(
                home_dir(Some("relative/home"), Some("/other")),
                Some("relative/home")
            );
        }
        #[cfg(windows)]
        {
            assert_eq!(
                home_dir(Some(r"C:\Users\u"), Some(r"C:\Other")),
                Some(r"C:\Users\u")
            );
            assert_eq!(home_dir(None, Some(r"C:\Users\u")), Some(r"C:\Users\u"));
            // A POSIX-style HOME (Git Bash) is not Windows-absolute,
            // so USERPROFILE wins.
            assert_eq!(
                home_dir(Some("/c/Users/u"), Some(r"C:\Users\u")),
                Some(r"C:\Users\u")
            );
            assert_eq!(home_dir(None, None), None);
        }
    }

    #[test]
    fn xdg_path_wins_when_set() {
        assert_eq!(
            resolve_config_path(Some(TEST_XDG), Some(TEST_HOME), false, false).unwrap(),
            xdg_config_path()
        );
    }

    #[test]
    fn default_path_when_xdg_unset() {
        assert_eq!(
            resolve_config_path(None, Some(TEST_HOME), false, false).unwrap(),
            home_config_path()
        );
    }

    #[test]
    fn relative_xdg_is_ignored() {
        // A relative XDG_CONFIG_HOME is invalid per the XDG spec.
        assert_eq!(
            resolve_config_path(Some("relative/xdg"), Some(TEST_HOME), false, false).unwrap(),
            home_config_path()
        );
    }

    #[test]
    fn two_existing_files_are_an_error() {
        let err = resolve_config_path(Some(TEST_XDG), Some(TEST_HOME), true, true).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err
            .context
            .contains(&xdg_config_path().display().to_string()));
        assert!(err
            .context
            .contains(&home_config_path().display().to_string()));
    }

    #[test]
    fn one_existing_file_is_not_ambiguous() {
        // Only the default exists: the XDG path still wins (it is the
        // location), and the default file is simply not the config.
        assert_eq!(
            resolve_config_path(Some(TEST_XDG), Some(TEST_HOME), false, true).unwrap(),
            xdg_config_path()
        );
        assert_eq!(
            resolve_config_path(Some(TEST_XDG), Some(TEST_HOME), true, false).unwrap(),
            xdg_config_path()
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
    fn instances_table_parses_sorted_by_alias() {
        let config = parse_config(
            "[instances]\nexample = \"https://api.opensuse.org\"\nlocal = \"http://127.0.0.1:3001\"\n",
            &config_path(),
        )
        .unwrap();
        assert_eq!(
            config.instances,
            vec![
                (
                    "example".to_string(),
                    "https://api.opensuse.org".to_string()
                ),
                ("local".to_string(), "http://127.0.0.1:3001".to_string()),
            ]
        );
    }

    #[test]
    fn instances_alias_must_be_an_env_var_segment() {
        for alias in ["my-instance", "my instance", "example.instance"] {
            let text = format!("[instances]\n\"{alias}\" = \"https://api.opensuse.org\"\n");
            let err = parse_config(&text, &config_path()).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ConfigError, "alias {alias:?}");
            assert!(
                err.context.contains(&format!("{alias:?}")),
                "alias {alias:?}"
            );
        }
        // An empty alias (a quoted empty key) is not a segment either.
        let err = parse_config(
            "[instances]\n\"\" = \"https://api.opensuse.org\"\n",
            &config_path(),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
    }

    #[test]
    fn instances_alias_collision_after_uppercasing_is_an_error() {
        let err = parse_config(
            "[instances]\nmyalias = \"https://api.opensuse.org\"\nMYALIAS = \"https://build.opensuse.org\"\n",
            &config_path(),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        // The message names the collision, not a specific alias:
        // which alias is reported second depends on table order.
        assert!(err
            .context
            .contains("collides with another alias after uppercasing"));
    }

    #[test]
    fn instances_alias_file_is_reserved() {
        // `GEECTL_GITEA_TOKEN_FILE` is the plain file variable:
        // an alias `file` would give it a second meaning.
        for alias in ["file", "FILE", "File"] {
            let text = format!("[instances]\n{alias} = \"https://gitea.example.org\"\n");
            let err = parse_config(&text, &config_path()).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ConfigError, "alias {alias:?}");
            assert!(err.context.contains("reserved"), "alias {alias:?}");
            assert!(
                err.context.contains(&format!("{alias:?}")),
                "alias {alias:?}"
            );
        }
    }

    #[test]
    fn instances_alias_may_look_like_a_credential_key() {
        // An alias is any key under `[instances]`; the
        // credential-key refusal is for config keys, not aliases.
        let config = parse_config(
            "[instances]\nmy_token = \"https://api.opensuse.org\"\ntoken = \"https://build.opensuse.org\"\n",
            &config_path(),
        )
        .unwrap();
        assert_eq!(config.instances.len(), 2);
    }

    #[test]
    fn instances_duplicate_url_is_an_error() {
        // The same instance spelled twice — explicit `:443` is
        // the same URL under `same_api_url` — names one instance
        // twice and is refused at load.
        let err = parse_config(
            "[instances]\nalpha = \"https://api.example.org\"\nbeta = \"https://api.example.org:443\"\n",
            &config_path(),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("alpha"));
        assert!(err.context.contains("beta"));
    }

    #[test]
    fn instances_malformed_url_loads_and_is_kept_trimmed() {
        // A malformed `[instances]` URL is not a load error any
        // more: the entry loads and is kept trimmed, and its
        // failure surfaces only when that instance is read.
        for bad in [
            "notaurl",
            "https://api.example.org:notaport",
            "https://api.example.org:0",
            "https://api.example.org:65536",
            "https://",
            "https://api.example.org?x=1",
        ] {
            let text = format!("[instances]\nalpha = \"{bad}\"\n");
            let config = parse_config(&text, &config_path()).unwrap();
            assert_eq!(
                config.instances,
                vec![("alpha".to_string(), bad.to_string())],
                "{bad}"
            );
        }
    }

    #[test]
    fn instances_userinfo_url_is_a_config_error() {
        // One shape is refused at load, not tolerated like the
        // other malformed shapes: a userinfo-bearing URL can
        // never resolve, and the loaded string would echo in
        // full through the merged results render, leaking the
        // credential. The failure names the file and the alias,
        // never the URL itself.
        for bad in [
            "https://user:secret@api.example.org",
            "https://user@api.example.org",
            "http://user:secret@127.0.0.1:3001",
        ] {
            let text = format!("[instances]\nalpha = \"{bad}\"\n");
            let err = parse_config(&text, &config_path()).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ConfigError, "{bad}");
            assert!(err.context.contains("/x/geectl/config.toml"), "{bad}");
            assert!(err.context.contains("alpha"), "{bad}");
            assert!(err.context.contains("userinfo"), "{bad}");
            assert!(!err.context.contains("secret"), "{bad}");
        }
        // An `@` outside the authority is data, not userinfo:
        // a path carrying one still loads.
        let config = parse_config(
            "[instances]\nalpha = \"https://api.example.org/path@x\"\n",
            &config_path(),
        )
        .unwrap();
        assert_eq!(
            config.instances,
            vec![(
                "alpha".to_string(),
                "https://api.example.org/path@x".to_string()
            )]
        );
    }

    #[test]
    fn instances_url_value_is_trimmed() {
        // The loader trims the value: surrounding whitespace in
        // the file does not become part of the URL.
        let config = parse_config(
            "[instances]\nalpha = \"  https://api.example.org  \"\n",
            &config_path(),
        )
        .unwrap();
        assert_eq!(
            config.instances,
            vec![("alpha".to_string(), "https://api.example.org".to_string())]
        );
    }

    #[test]
    fn instances_bad_port_entry_loads_and_matches_nothing() {
        // A bad port no longer fails the load: the entry is kept
        // trimmed and matches nothing, so it cannot duplicate
        // the valid entry either — its failure surfaces when
        // that instance is read.
        let config = parse_config(
            "[instances]\nalpha = \"https://api.example.org:notaport\"\nbeta = \"https://api.example.org\"\n",
            &config_path(),
        )
        .unwrap();
        assert_eq!(
            config.instances,
            vec![
                (
                    "alpha".to_string(),
                    "https://api.example.org:notaport".to_string()
                ),
                ("beta".to_string(), "https://api.example.org".to_string()),
            ]
        );
        // Two aliases for the same malformed URL load too: a
        // malformed URL matches nothing, so neither entry can
        // shadow the other's variables at read time.
        let config = parse_config(
            "[instances]\nalpha = \"https://api.example.org:notaport\"\nbeta = \"https://api.example.org:notaport\"\n",
            &config_path(),
        )
        .unwrap();
        assert_eq!(config.instances.len(), 2);
    }

    #[test]
    fn instances_value_must_be_a_url_string() {
        let err = parse_config("[instances]\nexample = 42\n", &config_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("example"));
    }

    #[test]
    fn instances_must_be_a_table() {
        let err =
            parse_config("instances = \"https://api.opensuse.org\"\n", &config_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("instances"));
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
    fn build_table_parses() {
        let config = parse_config(
            "[build]\ncache_dir = \"/srv/geectl-cache\"\ncache_cap_gib = 40\n",
            &config_path(),
        )
        .unwrap();
        assert_eq!(config.build_cache_dir.as_deref(), Some("/srv/geectl-cache"));
        assert_eq!(config.build_cache_cap_gib, Some(40));
        // Absent table reads as defaults, like the endpoints.
        let config = parse_config("", &config_path()).unwrap();
        assert_eq!(config.build_cache_dir, None);
        assert_eq!(config.build_cache_cap_gib, None);
    }

    #[test]
    fn build_table_rejects_bad_values() {
        // Unknown key inside [build].
        let err = parse_config("[build]\nccache = true\n", &config_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("build.ccache"));
        // Not a table.
        let err = parse_config("build = \"/srv\"\n", &config_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("'build' must be a table"));
        // Empty cache dir.
        let err = parse_config("[build]\ncache_dir = \"  \"\n", &config_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("build.cache_dir"));
        // Non-string cache dir.
        let err = parse_config("[build]\ncache_dir = 42\n", &config_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        // Zero cap: a cap that evicts everything is not a cap.
        let err = parse_config("[build]\ncache_cap_gib = 0\n", &config_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("build.cache_cap_gib"));
        // Negative and non-integer caps.
        let err = parse_config("[build]\ncache_cap_gib = -1\n", &config_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        let err = parse_config("[build]\ncache_cap_gib = \"20\"\n", &config_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
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
            format: None,
            instances: vec![],
            build_cache_dir: None,
            build_cache_cap_gib: None,
        };
        let err = effective_config(&config).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(!err.context.contains("secret"));
        // A valid file value still flows through.
        let config = Config {
            obs_url: Some("https://api.opensuse.org".to_string()),
            gitea_url: None,
            format: None,
            instances: vec![],
            build_cache_dir: None,
            build_cache_cap_gib: None,
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
            resolve_config_path(Some(TEST_XDG), Some("relative/home"), false, false).unwrap(),
            xdg_config_path()
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

    #[test]
    fn format_parses_from_config_file() {
        let config = parse_config("format = \"text\"\n", Path::new("config.toml")).unwrap();
        assert_eq!(config.format.as_deref(), Some("text"));
    }

    #[test]
    fn format_rejects_unknown_values() {
        let err = parse_config("format = \"yaml\"\n", Path::new("config.toml")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("yaml"));
        let err = parse_config("format = 3\n", Path::new("config.toml")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("format"));
    }

    #[test]
    fn effective_config_resolves_format_from_file() {
        let config = Config {
            obs_url: None,
            gitea_url: None,
            format: Some("text".to_string()),
            instances: vec![],
            build_cache_dir: None,
            build_cache_cap_gib: None,
        };
        let effective = effective_config(&config).unwrap();
        assert_eq!(effective.config.format.as_deref(), Some("text"));
        assert_eq!(effective.format_source, "config");
    }

    #[test]
    fn effective_config_revalidates_format() {
        // A Config built without going through parse_config must not
        // smuggle an unvalidated format past effective_config.
        let config = Config {
            obs_url: None,
            gitea_url: None,
            format: Some("yaml".to_string()),
            instances: vec![],
            build_cache_dir: None,
            build_cache_cap_gib: None,
        };
        let err = effective_config(&config).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("yaml"));
    }

    #[test]
    fn effective_config_defaults_format_to_json() {
        let effective = effective_config(&Config::default()).unwrap();
        assert_eq!(effective.config.format.as_deref(), Some("json"));
        assert_eq!(effective.format_source, "default");
    }

    // --- add_instance / splice_instance / alias_from_url ---

    #[test]
    fn add_instance_writes_section_to_new_file() {
        let dir = TempDir::new();
        let path = dir.0.join("config.toml");
        let outcome =
            add_instance(&path, "api_opensuse_org", "https://api.opensuse.org", false).unwrap();
        assert_eq!(outcome, AddInstance::Added);
        let config = load_config_at(&path).unwrap();
        assert_eq!(
            config.instances,
            vec![(
                "api_opensuse_org".to_string(),
                "https://api.opensuse.org".to_string()
            )]
        );
        // A minimal file: only the section, no invented endpoints.
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[instances]\napi_opensuse_org = \"https://api.opensuse.org\"\n"
        );
    }

    #[test]
    fn add_instance_inserts_into_existing_section_preserving_rest() {
        let dir = TempDir::new();
        let path = dir.0.join("config.toml");
        fs::write(
            &path,
            "# a user comment\nobs_url = \"https://api.opensuse.org\"\n[instances]\nexample = \"https://obs.example.com\"\n",
        )
        .unwrap();
        let outcome = add_instance(&path, "local", "http://127.0.0.1:3001", false).unwrap();
        assert_eq!(outcome, AddInstance::Added);
        let text = fs::read_to_string(&path).unwrap();
        // The comment, the endpoint line and the old entry survive.
        assert!(text.contains("# a user comment\n"), "{text}");
        assert!(
            text.contains("obs_url = \"https://api.opensuse.org\"\n"),
            "{text}"
        );
        assert!(
            text.contains("example = \"https://obs.example.com\"\n"),
            "{text}"
        );
        assert!(
            text.contains("local = \"http://127.0.0.1:3001\"\n"),
            "{text}"
        );
        let config = load_config_at(&path).unwrap();
        assert_eq!(config.instances.len(), 2);
    }

    #[test]
    fn add_instance_refuses_existing_alias_without_force() {
        let dir = TempDir::new();
        let path = dir.0.join("config.toml");
        fs::write(
            &path,
            "[instances]\napi_opensuse_org = \"https://other.example.com\"\n",
        )
        .unwrap();
        let before = fs::read_to_string(&path).unwrap();
        let err =
            add_instance(&path, "api_opensuse_org", "https://api.opensuse.org", false).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("already names"), "{}", err.context);
        assert!(err.context.contains("--force"), "{}", err.context);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn add_instance_force_replaces_alias() {
        let dir = TempDir::new();
        let path = dir.0.join("config.toml");
        fs::write(
            &path,
            "[instances]\napi_opensuse_org = \"https://other.example.com\"\nother = \"https://obs.example.com\"\n",
        )
        .unwrap();
        let outcome =
            add_instance(&path, "api_opensuse_org", "https://api.opensuse.org", true).unwrap();
        assert_eq!(outcome, AddInstance::Replaced);
        let config = load_config_at(&path).unwrap();
        assert_eq!(
            config.instances,
            vec![
                (
                    "api_opensuse_org".to_string(),
                    "https://api.opensuse.org".to_string()
                ),
                ("other".to_string(), "https://obs.example.com".to_string()),
            ]
        );
        assert!(
            !fs::read_to_string(&path)
                .unwrap()
                .contains("other.example.com"),
            "old URL must be gone"
        );
    }

    #[test]
    fn add_instance_already_present_is_noop() {
        let dir = TempDir::new();
        let path = dir.0.join("config.toml");
        fs::write(
            &path,
            "[instances]\napi_opensuse_org = \"https://api.opensuse.org\"\n",
        )
        .unwrap();
        let before = fs::read_to_string(&path).unwrap();
        let outcome =
            add_instance(&path, "api_opensuse_org", "https://api.opensuse.org", false).unwrap();
        assert_eq!(outcome, AddInstance::AlreadyPresent);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn add_instance_refuses_url_named_by_another_alias() {
        let dir = TempDir::new();
        let path = dir.0.join("config.toml");
        fs::write(
            &path,
            "[instances]\nexample = \"https://api.opensuse.org\"\n",
        )
        .unwrap();
        let before = fs::read_to_string(&path).unwrap();
        let err = add_instance(&path, "other", "https://api.opensuse.org", false).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("example"), "{}", err.context);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn add_instance_malformed_existing_config_fails_closed() {
        let dir = TempDir::new();
        let path = dir.0.join("config.toml");
        fs::write(
            &path,
            "obs_url = \"https://api.opensuse.org\"\n[instances\n",
        )
        .unwrap();
        let before = fs::read_to_string(&path).unwrap();
        let err =
            add_instance(&path, "api_opensuse_org", "https://api.opensuse.org", false).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn add_instance_inline_instances_refused() {
        let dir = TempDir::new();
        let path = dir.0.join("config.toml");
        fs::write(
            &path,
            "instances = { example = \"https://obs.example.com\" }\n",
        )
        .unwrap();
        let before = fs::read_to_string(&path).unwrap();
        let err = add_instance(&path, "local", "http://127.0.0.1:3001", false).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("section"), "{}", err.context);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn add_instance_validates_alias_and_url() {
        let dir = TempDir::new();
        let path = dir.0.join("config.toml");
        // A bad alias and a userinfo URL are both refused, and
        // neither writes anything.
        assert!(add_instance(&path, "bad-alias", "https://api.opensuse.org", false).is_err());
        assert!(add_instance(&path, "file", "https://api.opensuse.org", false).is_err());
        assert!(add_instance(&path, "ok", "https://user:secret@api.opensuse.org", false).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn alias_from_url_derives_host() {
        assert_eq!(
            alias_from_url("https://api.opensuse.org").unwrap(),
            "api_opensuse_org"
        );
        assert_eq!(
            alias_from_url("https://api.opensuse.org:443").unwrap(),
            "api_opensuse_org"
        );
        assert_eq!(
            alias_from_url("http://127.0.0.1:3001").unwrap(),
            "127_0_0_1"
        );
        assert_eq!(
            alias_from_url("https://obs.example.com/base").unwrap(),
            "obs_example_com"
        );
        assert_eq!(alias_from_url("https://[::1]:3001").unwrap(), "1");
        assert!(alias_from_url("https://").is_err());
        assert!(alias_from_url("").is_err());
    }

    #[test]
    fn splice_instance_replaces_quoted_key() {
        let text = "[instances]\n\"api_opensuse_org\" = \"https://other.example.com\"\n";
        let out = splice_instance(
            text,
            "api_opensuse_org",
            "https://api.opensuse.org",
            true,
            &config_path(),
        )
        .unwrap();
        assert_eq!(
            out,
            "[instances]\napi_opensuse_org = \"https://api.opensuse.org\"\n"
        );
    }
}
