//! The auth seam and credential validation.
//!
//! This module is the credential design: the providers that supply
//! credentials from environment variables or mounted secret files,
//! and the validation that keeps a malformed credential from
//! reaching an Authorization header. It deliberately stores no
//! credentials and reads none from a config file, a prompt, or
//! `/dev/tty`: providers supply credentials from environment
//! variables or mounted secret files, and the HTTP client applies
//! them. A `.env` file in the working directory is an
//! environment-variable source, not a config file: with explicit
//! opt-in (see [`load_dotenv`]) it merges into the environment
//! snapshot (see [`snapshot_env`]). There is no
//! plaintext fallback and no obfuscation option, and a
//! credential-shaped value in a config file is a refusal to run
//! (see the design doc and `crate::config`).
//!
//! Credential types covered:
//! - OBS: API token (Bearer) or username/password (Basic auth).
//!   The server accepts Basic today; token support is upstream
//!   work, so a configured token is sent as `Authorization: Bearer <token>`;
//!   the server rejects it until that lands.
//! - Gitea: API token (Bearer), plus SSH keys for git operations.
//!
//! [`NoAuth`] is the only provider that supplies no credentials.
//! It exists so the HTTP client compiles and runs without
//! credentials; it is not a credential store.

use crate::error::{ErrorKind, GeectlError};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The credentials a provider supplies, as data. The HTTP client applies
/// them to a request; the provider never touches a request itself.
///
/// This type deliberately does NOT derive `Debug`: it carries secrets
/// (password, token), and a derived `Debug` would print them into any
/// log line or error that formats the value. The manual `Debug` below
/// shows the variant and the non-secret fields, and redacts the secrets.
#[derive(Clone)]
pub enum Credentials {
    /// No credentials. Public reads only.
    None,
    /// Username and password, sent as HTTP Basic auth.
    Basic { username: String, password: String },
    /// An API token, sent as a Bearer token.
    Bearer { token: String },
    /// An SSH key, for git operations only. It cannot authenticate an
    /// HTTP request — the HTTP client refuses it (see `http.rs`).
    SshKey { path: PathBuf },
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Credentials::None => write!(f, "None"),
            Credentials::Basic { username, .. } => f
                .debug_struct("Basic")
                .field("username", username)
                .field("password", &"[redacted]")
                .finish(),
            Credentials::Bearer { .. } => f
                .debug_struct("Bearer")
                .field("token", &"[redacted]")
                .finish(),
            Credentials::SshKey { path } => f.debug_struct("SshKey").field("path", path).finish(),
        }
    }
}

impl Credentials {
    /// The Basic username, when these credentials carry one: the OBS
    /// login the server records as a request's creator. None for
    /// `None`, Bearer, and SSH credentials — the credential value itself
    /// carries no username.
    pub fn username(&self) -> Option<&str> {
        match self {
            Credentials::Basic { username, .. } => Some(username),
            Credentials::None | Credentials::Bearer { .. } | Credentials::SshKey { .. } => None,
        }
    }
}

/// Supplies credentials to the HTTP client. Implementations read from
/// environment variables or mounted secret files only — never from a
/// config file, never from a prompt, never from `/dev/tty`.
pub trait AuthProvider: Send + Sync {
    fn credentials(&self) -> Result<Credentials, GeectlError>;
}

/// No credentials. Public reads only.
pub struct NoAuth;

impl AuthProvider for NoAuth {
    fn credentials(&self) -> Result<Credentials, GeectlError> {
        Ok(Credentials::None)
    }
}

/// True when the value carries a control character (or DEL). One in a
/// credential would corrupt the Authorization header or let a second
/// header be smuggled in, so any credential carrying one is rejected.
fn has_control_chars(value: &str) -> bool {
    value.chars().any(|c| c.is_control())
}

/// Validate a Bearer token. Trailing CR/LF is trimmed (a pasted token
/// with a trailing newline must not corrupt auth); any control character
/// anywhere else is rejected. Only CR/LF is stripped: a trailing space
/// is legitimate token content, and stripping all whitespace would
/// corrupt a token that ends in one (and empty a whitespace-only
/// token). The token itself is never included in the error — it is a
/// secret.
pub fn validate_bearer_token(token: &str) -> Result<String, GeectlError> {
    let token = token.trim_end_matches(['\r', '\n']);
    if token.trim().is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "bearer token is empty or whitespace-only",
        ));
    }
    if has_control_chars(token) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "bearer token contains control characters",
        ));
    }
    Ok(token.to_string())
}

/// Validate a Basic username: non-empty, no control characters, no
/// `:` — RFC 7617 reserves the colon as the separator between user-id
/// and password, so one in the username would corrupt the credential.
/// Trailing CR/LF is trimmed (a paste artifact, not part of the
/// credential); a trailing space is legitimate content and is kept.
fn validate_basic_username(username: &str) -> Result<String, GeectlError> {
    let username = username.trim_end_matches(['\r', '\n']);
    if username.trim().is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "basic auth username is empty or whitespace-only",
        ));
    }
    if has_control_chars(username) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "basic auth username contains control characters",
        ));
    }
    if username.contains(':') {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "basic auth username must not contain ':'",
        ));
    }
    Ok(username.to_string())
}

/// Validate a Basic password: non-empty, no control characters.
/// Trailing CR/LF is trimmed; a trailing space is legitimate content
/// and is kept. An empty (or whitespace-only) password is never a real
/// credential: fail fast here instead of sending "user:" and getting a
/// 401 at the first request.
fn validate_basic_password(password: &str) -> Result<String, GeectlError> {
    let password = password.trim_end_matches(['\r', '\n']);
    if password.trim().is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "basic auth password is empty or whitespace-only",
        ));
    }
    if has_control_chars(password) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "basic auth password contains control characters",
        ));
    }
    Ok(password.to_string())
}

/// Validate Basic credentials (username + password). The two fields
/// are validated separately so a failure names the field that failed;
/// see the per-field validators.
pub fn validate_basic(username: &str, password: &str) -> Result<(String, String), GeectlError> {
    Ok((
        validate_basic_username(username)?,
        validate_basic_password(password)?,
    ))
}

/// Environment variable holding the OBS username (Basic auth).
pub const OBS_USERNAME_VAR: &str = "GEECTL_OBS_USERNAME";
/// Environment variable holding the OBS password (Basic auth). Set
/// exactly one of this and [`OBS_PASSWORD_FILE_VAR`].
pub const OBS_PASSWORD_VAR: &str = "GEECTL_OBS_PASSWORD";
/// Environment variable holding the path to a file whose whole content
/// is the OBS password. Set exactly one of this and
/// [`OBS_PASSWORD_VAR`].
pub const OBS_PASSWORD_FILE_VAR: &str = "GEECTL_OBS_PASSWORD_FILE";
/// Environment variable holding the OBS API token (Bearer). Set
/// exactly one of this and [`OBS_APIKEY_FILE_VAR`].
pub const OBS_APIKEY_VAR: &str = "GEECTL_OBS_APIKEY";
/// Environment variable holding the path to a file whose whole content
/// is the OBS API token. Set exactly one of this and
/// [`OBS_APIKEY_VAR`].
pub const OBS_APIKEY_FILE_VAR: &str = "GEECTL_OBS_APIKEY_FILE";
/// Environment variable holding the Gitea API token (Bearer). Set
/// exactly one of this and [`GITEA_TOKEN_FILE_VAR`].
pub const GITEA_TOKEN_VAR: &str = "GEECTL_GITEA_TOKEN";
/// Environment variable holding the path to a file whose whole content
/// is the Gitea API token. Set exactly one of this and
/// [`GITEA_TOKEN_VAR`].
pub const GITEA_TOKEN_FILE_VAR: &str = "GEECTL_GITEA_TOKEN_FILE";
/// Environment variable holding the path to the Gitea SSH private key,
/// for git operations.
pub const GITEA_SSH_KEY_VAR: &str = "GEECTL_GITEA_SSH_KEY";

/// Lookup for one environment variable. Production passes a closure
/// over `std::env::var`; tests pass a closure over a fixed map, so
/// resolution is hermetic: no process-global env mutation and no
/// parallel-test interference.
pub type VarLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Reader for one secret file. Production passes
/// `std::fs::read_to_string`; tests pass a closure over a fixed map.
pub type FileReader<'a> = &'a dyn Fn(&Path) -> std::io::Result<String>;

/// Open a secret file once and return its unix mode (or `None`
/// where the platform has no unix modes) together with its contents.
/// Production passes [`open_secret_file_production`]; tests pass a
/// closure returning a fixed mode and contents, so the permission
/// check is exercised without touching the filesystem.
///
/// One opener, not a separate mode lookup plus a reader: the mode
/// and the content come from the same open file, so the type and
/// permission checks and the read cannot be separated by a path
/// swap in between.
pub type SecretFileOpener<'a> = &'a dyn Fn(&Path) -> std::io::Result<(Option<u32>, String)>;

/// Re-attribute a credential validation failure to the configuration
/// that supplied it. The value came from config (an env var or a
/// file), so the kind is `ConfigError`, and the message names the
/// source — never the value.
fn config_error(source: &str, err: GeectlError) -> GeectlError {
    GeectlError::new(ErrorKind::ConfigError, format!("{source}: {}", err.context))
}

/// The largest secret file accepted (64 KiB). A password or token
/// is a few dozen bytes; anything larger is a misconfiguration (a
/// whole keyring dumped into the file, say), not a secret.
pub const MAX_SECRET_FILE_BYTES: usize = 64 * 1024;

/// Open `path` for reading without blocking on unix: a FIFO at the
/// path has no writer, so a plain open would block until one arrives;
/// `O_NONBLOCK` makes the open return at once, and the caller then
/// refuses non-regular files before reading. For a regular file the
/// flag is a no-op for reads. On non-unix this is a plain open.
pub(crate) fn open_file_nonblocking(path: &Path) -> std::io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::File::open(path)
    }
}

/// Open a secret file in production and return its unix mode
/// together with its contents. The file must be a regular file (not
/// a device, pipe, or directory — reading those could block or
/// return unbounded data) and at most [`MAX_SECRET_FILE_BYTES`]
/// long. The open itself is non-blocking on unix, so a FIFO at the
/// path cannot block the open either: the regular-file check below
/// refuses it before any read. The size check is enforced while
/// reading (`take(MAX + 1)`),
/// so a file that grows between the open and the read is still
/// capped.
///
/// The type and mode checks and the read are all on the one open
/// file, not on a path re-resolved in between: the mode comes from
/// fstat on the open descriptor, and on unix the path's lstat
/// identity (dev, ino) must match the fstat identity — so a symlink
/// (the open follows it; lstat is the link) and a path swapped
/// after the open are both refused rather than silently read.
pub fn open_secret_file_production(path: &Path) -> std::io::Result<(Option<u32>, String)> {
    use std::io::Read;
    // Non-blocking on unix, so a FIFO at the path cannot block the
    // open; the regular-file check below refuses it before any read.
    let file = open_file_nonblocking(path)?;
    let meta = file.metadata()?;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::MetadataExt;
        let link = std::fs::symlink_metadata(path)?;
        if (link.dev(), link.ino()) != (meta.dev(), meta.ino()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path does not name the file that was opened (symlink or swapped path)",
            ));
        }
        Some(meta.mode())
    };
    #[cfg(not(unix))]
    let mode = None;
    if !meta.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut contents = Vec::new();
    file.take((MAX_SECRET_FILE_BYTES + 1) as u64)
        .read_to_end(&mut contents)?;
    if contents.len() > MAX_SECRET_FILE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("secret file exceeds {MAX_SECRET_FILE_BYTES} bytes"),
        ));
    }
    let text = String::from_utf8(contents)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok((mode, text))
}

/// Read a secret file in production: the contents only (the mode is
/// not needed for passwords and tokens). See
/// [`open_secret_file_production`] for the guards.
pub fn read_secret_file_production(path: &Path) -> std::io::Result<String> {
    open_secret_file_production(path).map(|(_, content)| content)
}

/// Read a secret from a file: the whole file is the secret. The caller
/// validates it (`validate_basic` / `validate_bearer_token`), which
/// trims a trailing CR/LF. A failure names the path, never the content.
/// The size cap and regular-file guard live in
/// [`read_secret_file_production`], the production reader; the
/// injected reader exists so tests can supply file contents without
/// touching the filesystem.
fn read_secret_file(path: &Path, read_file: FileReader<'_>) -> Result<String, GeectlError> {
    read_file(path).map_err(|e| {
        GeectlError::new(
            ErrorKind::ConfigError,
            format!("cannot read secret file {}: {e}", path.display()),
        )
    })
}

/// Resolve OBS Basic credentials from the environment.
///
/// Sources (at most one password source):
/// - `GEECTL_OBS_USERNAME` + `GEECTL_OBS_PASSWORD`, or
/// - `GEECTL_OBS_USERNAME` + `GEECTL_OBS_PASSWORD_FILE` (a file whose
///   whole content is the password).
///
/// Returns [`Credentials::None`] when nothing is configured — public
/// reads need no credentials. A half-configured pair (a username with
/// no password source, or a password source with no username) is a
/// `ConfigError`: silently dropping half the credential would fail
/// later with a confusing auth error. Both password sources set is a
/// `ConfigError` too: the source must be unambiguous. A malformed
/// value fails here, as `ConfigError` naming the variable, instead of
/// at the first request.
pub fn resolve_obs_basic(
    vars: VarLookup<'_>,
    read_file: FileReader<'_>,
) -> Result<Credentials, GeectlError> {
    resolve_obs_basic_with(
        OBS_USERNAME_VAR,
        OBS_PASSWORD_VAR,
        OBS_PASSWORD_FILE_VAR,
        vars,
        read_file,
    )
}

/// The same resolution as [`resolve_obs_basic`], but from the named
/// variables: the plain names for the configured instance, the
/// alias names (`GEECTL_OBS_USERNAME_<ALIAS>` and friends) for one
/// other instance. The error messages name the variables actually
/// read, so an alias misconfiguration points at the alias names.
fn resolve_obs_basic_with(
    username_var: &str,
    password_var: &str,
    password_file_var: &str,
    vars: VarLookup<'_>,
    read_file: FileReader<'_>,
) -> Result<Credentials, GeectlError> {
    let username = vars(username_var);
    let password = vars(password_var);
    let password_file = vars(password_file_var);
    if password.is_some() && password_file.is_some() {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("both {password_var} and {password_file_var} are set; set exactly one"),
        ));
    }
    let password_from_file = password_file.is_some();
    let password = if let Some(password) = password {
        Some(password)
    } else if let Some(file) = password_file {
        Some(read_secret_file(Path::new(&file), read_file)?)
    } else {
        None
    };
    match (username, password) {
        (None, None) => Ok(Credentials::None),
        (Some(username), Some(password)) => {
            // Each field names its own source on failure: a bad
            // password must not be attributed to the username
            // variable (or to the file variable when it came from
            // the environment, and vice versa).
            let username = validate_basic_username(&username)
                .map_err(|e| config_error(username_var, e))?;
            let password_source = if password_from_file {
                password_file_var
            } else {
                password_var
            };
            let password = validate_basic_password(&password)
                .map_err(|e| config_error(password_source, e))?;
            Ok(Credentials::Basic { username, password })
        }
        (Some(_), None) => Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!(
                "{username_var} is set but no password source is ({password_var} or {password_file_var})"
            ),
        )),
        (None, Some(_)) => Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!(
                "a password source is set ({password_var} or {password_file_var}) but {username_var} is not"
            ),
        )),
    }
}

/// Resolve the OBS API token (Bearer) from the environment.
///
/// Sources (at most one): `GEECTL_OBS_APIKEY`, or
/// `GEECTL_OBS_APIKEY_FILE` (a file whose whole content is the token).
/// Returns [`Credentials::None`] when neither is set. Both set is a
/// `ConfigError`: the source must be unambiguous. A malformed token
/// fails here, as `ConfigError` naming the variable.
pub fn resolve_obs_bearer(
    vars: VarLookup<'_>,
    read_file: FileReader<'_>,
) -> Result<Credentials, GeectlError> {
    resolve_obs_bearer_with(OBS_APIKEY_VAR, OBS_APIKEY_FILE_VAR, vars, read_file)
}

/// The same resolution as [`resolve_obs_bearer`], but from the named
/// variables: the plain names for the configured instance, the
/// alias names (`GEECTL_OBS_APIKEY_<ALIAS>` and friends) for one
/// other instance. The error messages name the variables actually
/// read, so an alias misconfiguration points at the alias names.
fn resolve_obs_bearer_with(
    apikey_var: &str,
    apikey_file_var: &str,
    vars: VarLookup<'_>,
    read_file: FileReader<'_>,
) -> Result<Credentials, GeectlError> {
    let token = vars(apikey_var);
    let token_file = vars(apikey_file_var);
    if token.is_some() && token_file.is_some() {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("both {apikey_var} and {apikey_file_var} are set; set exactly one"),
        ));
    }
    let token_from_file = token_file.is_some();
    let token = if let Some(token) = token {
        Some(token)
    } else if let Some(file) = token_file {
        Some(read_secret_file(Path::new(&file), read_file)?)
    } else {
        None
    };
    match token {
        None => Ok(Credentials::None),
        Some(token) => {
            // Each field names its own source on failure: a bad token
            // must not be attributed to the env variable when it came
            // from the file (or vice versa).
            let token_source = if token_from_file {
                apikey_file_var
            } else {
                apikey_var
            };
            let token = validate_bearer_token(&token).map_err(|e| config_error(token_source, e))?;
            Ok(Credentials::Bearer { token })
        }
    }
}

/// Split an API base URL into (scheme, host, explicit port): the
/// scheme and host lowercased, the port the explicit one when the URL
/// carries one. `http` and `https` only; a missing scheme, a missing
/// host, userinfo (`@` in the authority — a credential in the URL),
/// a query or fragment after the authority, and a malformed port
/// are `InvalidInput`. The URL itself is never
/// echoed in the error: it could carry userinfo. IPv6 hosts
/// arrive without their brackets.
fn split_api_url(url: &str) -> Result<(String, String, Option<u16>), GeectlError> {
    // The URL itself is never echoed: it could carry userinfo
    // (`ftp://user:secret@host` fails the scheme check before
    // the `@` check), and an error must never carry a credential.
    let invalid =
        |what: &str| GeectlError::new(ErrorKind::InvalidInput, format!("invalid API URL: {what}"));
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| invalid("expected scheme://host"))?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(invalid("scheme must be http or https"));
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        return Err(invalid("must have a host"));
    }
    // A query or fragment after the authority would ride along
    // verbatim into every request built from the base: the path
    // would land inside the query. Refuse it here — this is the
    // read boundary — so a mistyped `[instances]` URL fails at
    // read time instead of going out as a mangled request.
    if rest[authority.len()..].contains(['?', '#']) {
        return Err(invalid("must not contain a query or fragment"));
    }
    if authority.contains('@') {
        // The URL is not echoed here: the authority before the `@`
        // is a credential, and the error must never carry one.
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "invalid API URL: must not contain userinfo ('@')".to_string(),
        ));
    }
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, rest) = bracketed
            .split_once(']')
            .ok_or_else(|| invalid("unclosed IPv6 bracket"))?;
        let port = match rest.strip_prefix(':') {
            Some(port) => Some(parse_port(port).map_err(|_| invalid("invalid port"))?),
            None if rest.is_empty() => None,
            None => return Err(invalid("unexpected text after IPv6 host")),
        };
        (host.to_string(), port)
    } else {
        let parts: Vec<&str> = authority.split(':').collect();
        match parts.as_slice() {
            [host] => (host.to_string(), None),
            [host, port] => (
                host.to_string(),
                Some(parse_port(port).map_err(|_| invalid("invalid port"))?),
            ),
            _ => return Err(invalid("too many ':' in authority")),
        }
    };
    if host.is_empty() {
        return Err(invalid("must have a host"));
    }
    Ok((scheme, host.to_ascii_lowercase(), port))
}

/// Parse a port from an authority: digits only, 1-65535. Zero is not
/// a port a server listens on, and a non-numeric value is a typo.
fn parse_port(text: &str) -> Result<u16, ()> {
    if text.is_empty() || !text.chars().all(|c| c.is_ascii_digit()) {
        return Err(());
    }
    match text.parse::<u16>() {
        Ok(0) | Err(_) => Err(()),
        Ok(port) => Ok(port),
    }
}

/// The effective port of a split URL: the explicit one, else the
/// scheme default (443 for https, 80 for http).
fn effective_port(scheme: &str, port: Option<u16>) -> u16 {
    port.unwrap_or(if scheme == "https" { 443 } else { 80 })
}

/// True when two API base URLs name the same instance: same scheme,
/// same host (case-insensitive), same effective port. The spelling
/// does not matter — `https://api.opensuse.org` and
/// `https://api.opensuse.org:443` are the same instance, as are
/// `https://API.OPENSUSE.ORG` and the lowercase form.
pub fn same_api_url(a: &str, b: &str) -> Result<bool, GeectlError> {
    let (scheme_a, host_a, port_a) = split_api_url(a)?;
    let (scheme_b, host_b, port_b) = split_api_url(b)?;
    Ok(scheme_a == scheme_b
        && host_a == host_b
        && effective_port(&scheme_a, port_a) == effective_port(&scheme_b, port_b))
}

/// The alias whose configured URL matches `url` under
/// [`same_api_url`] — the first match in alias order — or `None`
/// when no alias names this URL. The alias is returned as written
/// in the config; the env-var names derive from its uppercased
/// form (see [`obs_alias_vars`] and [`gitea_alias_vars`]).
///
/// A malformed lookup URL is the caller's error, reported as
/// before. A malformed entry in the table matches nothing, so one
/// bad `[instances]` entry never poisons the lookup for the other
/// instances: a caller that reads per instance records that
/// entry's failure for the entry alone.
pub fn alias_for_url<'a>(
    url: &str,
    instances: &'a [(String, String)],
) -> Result<Option<&'a str>, GeectlError> {
    split_api_url(url)?;
    for (alias, instance_url) in instances {
        if same_api_url(url, instance_url).unwrap_or(false) {
            return Ok(Some(alias.as_str()));
        }
    }
    Ok(None)
}

/// The credential variable names for one instance alias: the alias
/// as written in the config, uppercased (env vars are conventionally
/// uppercase). Basic first, then the API-key pair.
pub fn obs_alias_vars(alias: &str) -> (String, String, String, String, String) {
    let upper = alias.to_ascii_uppercase();
    (
        format!("{OBS_USERNAME_VAR}_{upper}"),
        format!("{OBS_PASSWORD_VAR}_{upper}"),
        format!("{OBS_PASSWORD_FILE_VAR}_{upper}"),
        format!("{OBS_APIKEY_VAR}_{upper}"),
        format!("{OBS_APIKEY_FILE_VAR}_{upper}"),
    )
}

/// Resolve OBS Basic credentials for one API URL.
///
/// Precedence, in order:
/// 1. The matching alias's variables (`GEECTL_OBS_USERNAME_<ALIAS>`
///    / `GEECTL_OBS_PASSWORD[_FILE]_<ALIAS>`, with `<ALIAS>` the
///    alias as written in the config file's `[instances]` table,
///    uppercased) win when any of them is set — resolved strictly
///    from that set, with the same rules as [`resolve_obs_basic`].
///    The alias is the one whose configured URL matches this URL
///    under [`same_api_url`].
/// 2. Otherwise the plain variables apply when the URL is the
///    configured default (`default_url`, compared with
///    [`same_api_url`]): one configured URL works exactly as before.
/// 3. Otherwise — a different URL, credentials configured for the
///    default, none for this URL — a `ConfigError` naming what to
///    set: the alias variables when an alias matches this URL, or
///    the `[instances]` entry to add when none does. The default's
///    credentials are never forwarded to another instance.
///
/// When nothing is configured at all the result is [`Credentials::None`]:
/// public reads and dry runs need no credentials.
pub fn resolve_obs_basic_for_url(
    url: &str,
    default_url: &str,
    instances: &[(String, String)],
    vars: VarLookup<'_>,
    read_file: FileReader<'_>,
) -> Result<Credentials, GeectlError> {
    if let Some(alias) = alias_for_url(url, instances)? {
        let (username_var, password_var, password_file_var, _, _) = obs_alias_vars(alias);
        let alias_set = vars(&username_var).is_some()
            || vars(&password_var).is_some()
            || vars(&password_file_var).is_some();
        if alias_set {
            return resolve_obs_basic_with(
                &username_var,
                &password_var,
                &password_file_var,
                vars,
                read_file,
            );
        }
        // No alias variables set: the URL may still be the
        // configured default, which takes the plain variables.
        if same_api_url(url, default_url)? {
            return resolve_obs_basic(vars, read_file);
        }
        return resolve_unmatched_url(
            url,
            &format!(
                "instance alias '{alias}' matches this URL — set {username_var} and {password_var} (or {password_file_var})"
            ),
            vars,
            read_file,
        );
    }
    if same_api_url(url, default_url)? {
        return resolve_obs_basic(vars, read_file);
    }
    // The example is rendered by the TOML serializer, not
    // `Debug`: `Debug` renders control characters as `\u{...}`,
    // which TOML rejects, so the copied example would not parse.
    let example = toml::Value::String(url.to_string());
    resolve_unmatched_url(
        url,
        &format!(
            "no instance alias matches it; add an [instances] entry for it in the config file (e.g. [instances] myinstance = {example}) and set GEECTL_OBS_USERNAME_<ALIAS> and GEECTL_OBS_PASSWORD[_FILE]_<ALIAS> for that alias"
        ),
        vars,
        read_file,
    )
}

/// Resolve one scope's OBS credentials: an API token when any token
/// variable is set, a Basic pair when any Basic variable is set,
/// [`Credentials::None`] when neither is. Both kinds set is a
/// `ConfigError`: the credential source must be unambiguous, the
/// same rule as both password sources set.
fn resolve_obs_scope(
    apikey_var: &str,
    apikey_file_var: &str,
    username_var: &str,
    password_var: &str,
    password_file_var: &str,
    vars: VarLookup<'_>,
    read_file: FileReader<'_>,
) -> Result<Credentials, GeectlError> {
    let token_set = vars(apikey_var).is_some() || vars(apikey_file_var).is_some();
    let basic_set = vars(username_var).is_some()
        || vars(password_var).is_some()
        || vars(password_file_var).is_some();
    if token_set && basic_set {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!(
                "both an OBS API key ({apikey_var} or {apikey_file_var}) and Basic credentials ({username_var} and {password_var} or {password_file_var}) are set; set exactly one credential type"
            ),
        ));
    }
    if token_set {
        return resolve_obs_bearer_with(apikey_var, apikey_file_var, vars, read_file);
    }
    if basic_set {
        return resolve_obs_basic_with(
            username_var,
            password_var,
            password_file_var,
            vars,
            read_file,
        );
    }
    Ok(Credentials::None)
}

/// The plain-variable scope: the same token-or-Basic rule from the
/// plain variable names.
fn resolve_obs_plain(
    vars: VarLookup<'_>,
    read_file: FileReader<'_>,
) -> Result<Credentials, GeectlError> {
    resolve_obs_scope(
        OBS_APIKEY_VAR,
        OBS_APIKEY_FILE_VAR,
        OBS_USERNAME_VAR,
        OBS_PASSWORD_VAR,
        OBS_PASSWORD_FILE_VAR,
        vars,
        read_file,
    )
}

/// Resolve OBS credentials — an API token (Bearer) or a
/// username/password pair (Basic) — for one API URL.
///
/// Precedence, in order:
/// 1. The matching alias's variables win when any of them is set —
///    resolved strictly from that set: a token when any token
///    variable is set, a Basic pair when any Basic variable is
///    set. Both kinds set for one alias is a `ConfigError`.
///    The alias is the one whose configured URL matches this URL
///    under [`same_api_url`].
/// 2. Otherwise the plain variables apply when the URL is the
///    configured default (`default_url`, compared with
///    [`same_api_url`]): one configured URL works exactly as before.
/// 3. Otherwise — a different URL, credentials configured for the
///    default, none for this URL — a `ConfigError` naming what to
///    set: the alias variables when an alias matches this URL, or
///    the `[instances]` entry to add when none does. The default's
///    credentials are never forwarded to another instance.
///
/// When nothing is configured at all the result is [`Credentials::None`]:
/// public reads and dry runs need no credentials.
pub fn resolve_obs_for_url(
    url: &str,
    default_url: &str,
    instances: &[(String, String)],
    vars: VarLookup<'_>,
    read_file: FileReader<'_>,
) -> Result<Credentials, GeectlError> {
    if let Some(alias) = alias_for_url(url, instances)? {
        let (username_var, password_var, password_file_var, apikey_var, apikey_file_var) =
            obs_alias_vars(alias);
        let alias_set = vars(&username_var).is_some()
            || vars(&password_var).is_some()
            || vars(&password_file_var).is_some()
            || vars(&apikey_var).is_some()
            || vars(&apikey_file_var).is_some();
        if alias_set {
            return resolve_obs_scope(
                &apikey_var,
                &apikey_file_var,
                &username_var,
                &password_var,
                &password_file_var,
                vars,
                read_file,
            );
        }
        // No alias variables set: the URL may still be the
        // configured default, which takes the plain variables.
        if same_api_url(url, default_url)? {
            return resolve_obs_plain(vars, read_file);
        }
        return resolve_obs_unmatched_url(
            url,
            &format!(
                "instance alias '{alias}' matches this URL — set {apikey_var} (or {apikey_file_var}), or {username_var} and {password_var} (or {password_file_var})"
            ),
            vars,
            read_file,
        );
    }
    if same_api_url(url, default_url)? {
        return resolve_obs_plain(vars, read_file);
    }
    // The example is rendered by the TOML serializer, not
    // `Debug`: `Debug` renders control characters as `\u{...}`,
    // which TOML rejects, so the copied example would not parse.
    let example = toml::Value::String(url.to_string());
    resolve_obs_unmatched_url(
        url,
        &format!(
            "no instance alias matches it; add an [instances] entry for it in the config file (e.g. [instances] myinstance = {example}) and set GEECTL_OBS_APIKEY[_FILE]_<ALIAS>, or GEECTL_OBS_USERNAME_<ALIAS> and GEECTL_OBS_PASSWORD[_FILE]_<ALIAS> for that alias"
        ),
        vars,
        read_file,
    )
}

/// The fallback when neither alias nor plain variables apply to
/// this URL: `None` when nothing is configured at all, else a
/// `ConfigError` naming what to set (`guidance` — the alias
/// variables when an alias matches, the `[instances]` entry to add
/// when none does). The plain credentials, when configured, belong
/// to the default instance and are never forwarded.
fn resolve_obs_unmatched_url(
    url: &str,
    guidance: &str,
    vars: VarLookup<'_>,
    read_file: FileReader<'_>,
) -> Result<Credentials, GeectlError> {
    match resolve_obs_plain(vars, read_file) {
        Ok(Credentials::None) => Ok(Credentials::None),
        Ok(_) => Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!(
                "no credentials configured for {url}; {guidance} — the credentials configured for the default instance are never forwarded to another instance"
            ),
        )),
        // A half-configured plain pair, or both credential types
        // set, errors about the plain variables — but for a
        // different URL those are not the ones to fix, so the
        // error carries the guidance too.
        Err(err) => Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("{} — for {url}, {guidance}", err.context),
        )),
    }
}

/// The fallback when neither alias nor plain variables apply to
/// this URL: `None` when nothing is configured at all, else a
/// `ConfigError` naming what to set (`guidance` — the alias
/// variables when an alias matches, the `[instances]` entry to add
/// when none does). The plain pair, when configured, belongs to
/// the default instance and is never forwarded.
fn resolve_unmatched_url(
    url: &str,
    guidance: &str,
    vars: VarLookup<'_>,
    read_file: FileReader<'_>,
) -> Result<Credentials, GeectlError> {
    match resolve_obs_basic(vars, read_file) {
        Ok(Credentials::None) => Ok(Credentials::None),
        Ok(_) => Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!(
                "no credentials configured for {url}; {guidance} — the credentials configured for the default instance are never forwarded to another instance"
            ),
        )),
        // A half-configured plain pair errors about the plain
        // variables — but for a different URL those are not the ones
        // to fix, so the error carries the guidance too.
        Err(err) => Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("{} — for {url}, {guidance}", err.context),
        )),
    }
}

/// Resolve the Gitea API token (Bearer) from the environment.
///
/// Sources (at most one): `GEECTL_GITEA_TOKEN`, or
/// `GEECTL_GITEA_TOKEN_FILE` (a file whose whole content is the token).
/// Returns [`Credentials::None`] when neither is set. Both set is a
/// `ConfigError`: the source must be unambiguous. A malformed token
/// fails here, as `ConfigError` naming the variable.
pub fn resolve_gitea_bearer(
    vars: VarLookup<'_>,
    read_file: FileReader<'_>,
) -> Result<Credentials, GeectlError> {
    resolve_gitea_bearer_with(GITEA_TOKEN_VAR, GITEA_TOKEN_FILE_VAR, vars, read_file)
}

/// The same resolution as [`resolve_gitea_bearer`], but from the named
/// variables: the plain names for the configured instance, the
/// alias names (`GEECTL_GITEA_TOKEN_<ALIAS>` and friends) for one
/// other instance. The error messages name the variables actually
/// read, so an alias misconfiguration points at the alias names.
fn resolve_gitea_bearer_with(
    token_var: &str,
    token_file_var: &str,
    vars: VarLookup<'_>,
    read_file: FileReader<'_>,
) -> Result<Credentials, GeectlError> {
    let token = vars(token_var);
    let token_file = vars(token_file_var);
    if token.is_some() && token_file.is_some() {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("both {token_var} and {token_file_var} are set; set exactly one"),
        ));
    }
    let token_from_file = token_file.is_some();
    let token = if let Some(token) = token {
        Some(token)
    } else if let Some(file) = token_file {
        Some(read_secret_file(Path::new(&file), read_file)?)
    } else {
        None
    };
    match token {
        None => Ok(Credentials::None),
        Some(token) => {
            // Each field names its own source on failure: a bad token
            // must not be attributed to the env variable when it came
            // from the file (or vice versa).
            let token_source = if token_from_file {
                token_file_var
            } else {
                token_var
            };
            let token = validate_bearer_token(&token).map_err(|e| config_error(token_source, e))?;
            Ok(Credentials::Bearer { token })
        }
    }
}

/// The credential variable names for one instance alias: the alias
/// as written in the config, uppercased (env vars are conventionally
/// uppercase).
pub fn gitea_alias_vars(alias: &str) -> (String, String) {
    let upper = alias.to_ascii_uppercase();
    (
        format!("{GITEA_TOKEN_VAR}_{upper}"),
        format!("{GITEA_TOKEN_FILE_VAR}_{upper}"),
    )
}

/// Resolve the Gitea API token for one Gitea URL.
///
/// Precedence, in order:
/// 1. The matching alias's variables (`GEECTL_GITEA_TOKEN_<ALIAS>`
///    / `GEECTL_GITEA_TOKEN_FILE_<ALIAS>`, with `<ALIAS>` the
///    alias as written in the config file's `[instances]` table,
///    uppercased) win when any of them is set — resolved strictly
///    from that set, with the same rules as [`resolve_gitea_bearer`].
///    The alias is the one whose configured URL matches this URL
///    under [`same_api_url`].
/// 2. Otherwise the plain variables apply when the URL is the
///    configured default (`default_url`, compared with
///    [`same_api_url`]): one configured URL works exactly as before.
/// 3. Otherwise — a different URL, credentials configured for the
///    default, none for this URL — a `ConfigError` naming what to
///    set: the alias variables when an alias matches this URL, or
///    the `[instances]` entry to add when none does. The default's
///    credentials are never forwarded to another instance.
///
/// When nothing is configured at all the result is [`Credentials::None`]:
/// public reads and dry runs need no credentials.
pub fn resolve_gitea_bearer_for_url(
    url: &str,
    default_url: &str,
    instances: &[(String, String)],
    vars: VarLookup<'_>,
    read_file: FileReader<'_>,
) -> Result<Credentials, GeectlError> {
    if let Some(alias) = alias_for_url(url, instances)? {
        let (token_var, token_file_var) = gitea_alias_vars(alias);
        let alias_set = vars(&token_var).is_some() || vars(&token_file_var).is_some();
        if alias_set {
            return resolve_gitea_bearer_with(&token_var, &token_file_var, vars, read_file);
        }
        // No alias variables set: the URL may still be the
        // configured default, which takes the plain variables.
        if same_api_url(url, default_url)? {
            return resolve_gitea_bearer(vars, read_file);
        }
        return resolve_gitea_unmatched_url(
            url,
            &format!(
                "instance alias '{alias}' matches this URL — set {token_var} (or {token_file_var})"
            ),
            vars,
            read_file,
        );
    }
    if same_api_url(url, default_url)? {
        return resolve_gitea_bearer(vars, read_file);
    }
    // The example is rendered by the TOML serializer, not
    // `Debug`: `Debug` renders control characters as `\u{...}`,
    // which TOML rejects, so the copied example would not parse.
    let example = toml::Value::String(url.to_string());
    resolve_gitea_unmatched_url(
        url,
        &format!(
            "no instance alias matches it; add an [instances] entry for it in the config file (e.g. [instances] myinstance = {example}) and set GEECTL_GITEA_TOKEN_<ALIAS> (or GEECTL_GITEA_TOKEN_FILE_<ALIAS>) for that alias"
        ),
        vars,
        read_file,
    )
}

/// The fallback when neither alias nor plain variables apply to
/// this URL: `None` when nothing is configured at all, else a
/// `ConfigError` naming what to set (`guidance` — the alias
/// variables when an alias matches, the `[instances]` entry to add
/// when none does). The plain token, when configured, belongs to
/// the default instance and is never forwarded.
fn resolve_gitea_unmatched_url(
    url: &str,
    guidance: &str,
    vars: VarLookup<'_>,
    read_file: FileReader<'_>,
) -> Result<Credentials, GeectlError> {
    match resolve_gitea_bearer(vars, read_file) {
        Ok(Credentials::None) => Ok(Credentials::None),
        Ok(_) => Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!(
                "no credentials configured for {url}; {guidance} — the credentials configured for the default instance are never forwarded to another instance"
            ),
        )),
        // Both token sources set errors about the plain
        // variables — but for a different URL those are not the ones
        // to fix, so the error carries the guidance too.
        Err(err) => Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("{} — for {url}, {guidance}", err.context),
        )),
    }
}

/// Resolve the Gitea SSH private key from the environment
/// (`GEECTL_GITEA_SSH_KEY`, a path). Returns [`Credentials::None`]
/// when it is not set. The key must exist and be a regular file, must
/// not be readable by group or other (unix mode `& 0o077` must be
/// zero), and must look like a PEM private key (`-----BEGIN ...
/// PRIVATE KEY-----`). Any of those failing is a `ConfigError` naming
/// the path and the fix — a private key other users can read, or a
/// public key passed as a private key, is a misconfiguration, not a
/// key. The key authenticates git operations only; the HTTP client
/// refuses it.
pub fn resolve_gitea_ssh_key(
    vars: VarLookup<'_>,
    open_file: SecretFileOpener<'_>,
) -> Result<Credentials, GeectlError> {
    let Some(path) = vars(GITEA_SSH_KEY_VAR) else {
        return Ok(Credentials::None);
    };
    let path = PathBuf::from(path);
    // One open for the mode and the content: the mode is of the file
    // that was opened, so the type and permission checks and the read
    // are on the same file — no second path resolution in between.
    // A key other users can read is a misconfiguration, and it should
    // not even be read — hence the permission check before the PEM
    // validation below.
    let (mode, content) = open_file(&path).map_err(|e| {
        GeectlError::new(
            ErrorKind::ConfigError,
            format!("cannot read SSH key {}: {e}", path.display()),
        )
    })?;
    if let Some(mode) = mode {
        // The mode's file-type bits say what the path is: a symlink
        // (0o120000) or anything but a regular file (0o100000) is a
        // misconfiguration — the key must be the file itself, not a
        // link to one (a link's target can change under us). This
        // runs before the permission check, so a symlink is reported
        // as a symlink, not as "readable by group or other".
        if mode & 0o170000 != 0o100000 {
            let display = path.display();
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!(
                    "SSH key {display} is not a regular file (mode {:o}); expected a plain file",
                    mode & 0o777
                ),
            ));
        }
        if mode & 0o077 != 0 {
            let display = path.display();
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!(
                    "SSH key {display} is readable by group or other (mode {:o}); run `chmod 600 {display}`",
                    mode & 0o777
                ),
            ));
        }
    }
    // A leading BOM or whitespace (editors and copy-paste add both)
    // must not make a real key look invalid, and a BEGIN without an
    // END is a truncated file, not a key.
    let pem = content.trim_start_matches('\u{feff}').trim_start();
    if !pem.starts_with("-----BEGIN ")
        || !pem.contains("PRIVATE KEY-----")
        || !pem.contains("-----END ")
    {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!(
                "SSH key {} does not look like a private key (expected a PEM '-----BEGIN ... PRIVATE KEY-----' block)",
                path.display()
            ),
        ));
    }
    Ok(Credentials::SshKey { path })
}

/// The `.env` file name, looked up in the current working directory
/// only — never in a parent directory.
const DOTENV_FILE: &str = ".env";

/// The opt-in variable for `.env` loading: `GEECTL_DOTENV=1` in the
/// real process environment, never in the file itself. The file lives
/// in the working directory, which is a trust boundary — a `.env`
/// dropped into a checkout or unpacked from a tarball must not
/// redirect authed requests on its own. Only the literal value `1`
/// opts in, like `GEECTL_DEBUG` (see `resolve_debug` in `main.rs`).
const DOTENV_OPT_IN_VAR: &str = "GEECTL_DOTENV";

/// Whether the value of the opt-in variable opts into `.env` loading.
/// Pure, for tests: the real-environment read is in [`load_dotenv`].
fn dotenv_opted_in_from(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Whether the `.env` content carries any `GEECTL_*` assignment: a
/// line — after an optional `export` prefix, glued or spaced — whose
/// key starts with `GEECTL_`. Comments and blank lines do not count,
/// and neither does a key that merely mentions geectl. Pure, for
/// tests. A file with no such line is not a geectl file at all, so
/// strictness (mode, malformed lines) never applies to it — a
/// conventional project `.env` for other tools must not fail a run.
fn dotenv_carries_geectl_keys(content: &str) -> bool {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    content.lines().any(|raw_line| {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line).trim_start();
        if line.is_empty() || line.starts_with('#') {
            return false;
        }
        let line = line.strip_prefix("export").unwrap_or(line).trim_start();
        line.starts_with("GEECTL_")
    })
}

/// A `.env` error naming the file and the line, never the value: the
/// file holds secrets, so even a malformed line must not be echoed.
fn dotenv_error(path: &Path, lineno: usize, what: &str) -> GeectlError {
    GeectlError::new(
        ErrorKind::ConfigError,
        format!("{}: line {lineno}: {what}", path.display()),
    )
}

/// Parse `.env` content: one `KEY=value` per line, `#` comments,
/// blank lines, an optional `export` prefix followed by whitespace,
/// and optional single or double quotes around the value. No
/// variable interpolation and no multi-line values: a quote that does
/// not close on its line is malformed, and a `$` stays literal. Only
/// `GEECTL_*` keys are honoured — every other key is ignored without
/// parsing its value, so a project `.env` meant for other tools
/// cannot change geectl behaviour (a proxy or `GIT_*` variable, for
/// example). A key starting with `GEEC` that is not a valid
/// `GEECTL_*` key is a near-miss typo for a geectl variable, so it
/// fails rather than being silently ignored, like a malformed
/// `GEECTL_*` suffix. A key glued to `export` (`exportGEECTL_A`) is
/// the same kind of typo and fails too. A line with no `=`, an
/// unterminated quote, text after a quoted value, a near-miss `GEEC`
/// prefix, a key glued to `export`, or a `GEECTL_*` key with
/// anything but ASCII letters, digits, or `_` after the prefix is a
/// `ConfigError` naming the file and the line.
///
/// An unquoted value runs to the end of the line, except that a `#`
/// preceded by whitespace starts a comment; a `#` inside quotes, or
/// not preceded by whitespace, is literal. Quoted values are literal
/// inside the quotes (no escapes); unquoted values are trimmed. A
/// duplicate key keeps its last value.
fn parse_dotenv(content: &str, path: &Path) -> Result<HashMap<String, String>, GeectlError> {
    let mut map = HashMap::new();
    // A leading BOM (editors and copy-paste add one) must not make
    // the first key look invalid.
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    for (index, raw_line) in content.lines().enumerate() {
        let lineno = index + 1;
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let line = line.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // `export` followed by any whitespace (space or tab),
        // not exactly one space: `export\tKEY=value` is the same
        // prefix. A key glued to `export` (`exportGEECTL_A`) is not.
        let line = line
            .strip_prefix("export")
            .filter(|rest| rest.starts_with(char::is_whitespace))
            .map(str::trim_start)
            .unwrap_or(line);
        let Some(eq) = line.find('=') else {
            return Err(dotenv_error(path, lineno, "expected KEY=value"));
        };
        let key = line[..eq].trim_end();
        if key.is_empty() {
            return Err(dotenv_error(path, lineno, "expected KEY=value"));
        }
        // A key glued to `export` (`exportGEECTL_A`) is a typo for
        // an exported geectl variable, so it fails rather than being
        // silently ignored like any other non-geectl key.
        if let Some(rest) = key.strip_prefix("export") {
            if rest.starts_with("GEEC") {
                return Err(dotenv_error(path, lineno, "invalid variable name"));
            }
        }
        if !key.starts_with("GEECTL_") {
            // A near-miss prefix is a typo for a geectl variable, so
            // it fails rather than being silently ignored: the user
            // meant to configure geectl.
            if key.starts_with("GEEC") {
                return Err(dotenv_error(path, lineno, "invalid variable name"));
            }
            continue;
        }
        // A GEECTL_* key that is not a plain variable name is a typo
        // for a geectl variable, so it fails rather than being
        // silently ignored: the user meant to configure geectl.
        let suffix = &key["GEECTL_".len()..];
        if suffix.is_empty()
            || !suffix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(dotenv_error(path, lineno, "invalid variable name"));
        }
        let raw_value = line[eq + 1..].trim_start();
        let value = if let Some(rest) = raw_value.strip_prefix('\'') {
            let Some(end) = rest.find('\'') else {
                return Err(dotenv_error(path, lineno, "unterminated quote"));
            };
            check_dotenv_trailing(&rest[end + 1..], path, lineno)?;
            rest[..end].to_string()
        } else if let Some(rest) = raw_value.strip_prefix('"') {
            let Some(end) = rest.find('"') else {
                return Err(dotenv_error(path, lineno, "unterminated quote"));
            };
            check_dotenv_trailing(&rest[end + 1..], path, lineno)?;
            rest[..end].to_string()
        } else {
            // Unquoted: to the end of the line, minus a comment
            // starting at a `#` preceded by whitespace — or at the
            // start of the value, where there is no value before it.
            let comment = raw_value
                .char_indices()
                .find(|(i, c)| {
                    *c == '#'
                        && (*i == 0 || raw_value.as_bytes()[i - 1].is_ascii_whitespace())
                })
                .map(|(i, _)| i);
            match comment {
                Some(comment) => raw_value[..comment].trim_end().to_string(),
                None => raw_value.trim_end().to_string(),
            }
        };
        map.insert(key.to_string(), value);
    }
    Ok(map)
}

/// The text after a quoted `.env` value must be nothing but
/// whitespace and an optional `#` comment; anything else is a
/// malformed line.
fn check_dotenv_trailing(text: &str, path: &Path, lineno: usize) -> Result<(), GeectlError> {
    let trailing = text.trim_start();
    if trailing.is_empty() || trailing.starts_with('#') {
        Ok(())
    } else {
        Err(dotenv_error(
            path,
            lineno,
            "unexpected text after quoted value",
        ))
    }
}

/// Load and parse the `.env` file at `path`: `Ok(None)` when the
/// file is missing, or when it carries no `GEECTL_*` key — a missing
/// file is not an error, and a file for other tools is ignored whole,
/// whatever its mode or syntax. Any other failure is a `ConfigError`
/// naming the file (and the line, for a malformed line) and never the
/// value. The file is read through [`open_secret_file_production`],
/// so it must be a regular file, at most [`MAX_SECRET_FILE_BYTES`]
/// long, and not a symlink; and a file readable by group or other is
/// refused, like the SSH key in [`resolve_gitea_ssh_key`] — it holds
/// secrets, so a warning would leave the secret readable. On
/// non-Unix there is no mode to check (`None`): the explicit opt-in
/// in [`load_dotenv`] is the guard there, not a permission check.
fn load_dotenv_at(path: &Path) -> Result<Option<HashMap<String, String>>, GeectlError> {
    let (mode, content) = match open_secret_file_production(path) {
        Ok(found) => found,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!("cannot read {}: {err}", path.display()),
            ));
        }
    };
    // Strictness applies only to a file that carries geectl keys:
    // a project `.env` for other tools is ignored whole, whatever
    // its mode or syntax, so a conventional 0644 file never fails a
    // run that does not use it.
    if !dotenv_carries_geectl_keys(&content) {
        return Ok(None);
    }
    if let Some(mode) = mode {
        if mode & 0o077 != 0 {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!(
                    "{} is readable by group or other (mode {:o}); run `chmod 600 {}`",
                    path.display(),
                    mode & 0o777,
                    path.display()
                ),
            ));
        }
    }
    parse_dotenv(&content, path).map(Some)
}

/// The `.env` file, loaded once per process: the first caller reads
/// it, every later caller reuses the same snapshot, so the file
/// cannot change under a run between the startup load in `main.rs`
/// and a later [`snapshot_env`] call. [`load_dotenv_at`] stays
/// uncached, for tests that load explicit paths.
static DOTENV_CACHE: OnceLock<Result<Option<HashMap<String, String>>, GeectlError>> =
    OnceLock::new();

/// Load the `.env` file from the current working directory, if
/// present — see [`load_dotenv_at`]. The file is looked up in the
/// working directory only, never in a parent directory, and only
/// when the real process environment opts in with `GEECTL_DOTENV=1`:
/// without the opt-in the file is never read, so a stray `.env` in a
/// working copy cannot change a run. The opt-in is read from the real
/// environment only — a `GEECTL_DOTENV=1` line in the file itself
/// never opts in. The loaded snapshot is cached for the process (see
/// `DOTENV_CACHE`).
pub fn load_dotenv() -> Result<Option<HashMap<String, String>>, GeectlError> {
    DOTENV_CACHE.get_or_init(load_dotenv_uncached).clone()
}

/// Read the working-directory `.env` file once, uncached: the
/// `OnceLock` initializer for [`load_dotenv`].
fn load_dotenv_uncached() -> Result<Option<HashMap<String, String>>, GeectlError> {
    if !dotenv_opted_in_from(std::env::var(DOTENV_OPT_IN_VAR).ok().as_deref()) {
        return Ok(None);
    }
    load_dotenv_at(Path::new(DOTENV_FILE))
}

/// Merge one environment snapshot: the real process environment
/// wins, and the `.env` file fills only the names the environment
/// leaves unset. Pure, for tests: the process environment is not
/// touched here.
fn merge_snapshot(
    names: &[&str],
    real: &HashMap<String, String>,
    dotenv: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for name in names {
        if let Some(value) = real.get(*name).or_else(|| dotenv.get(*name)) {
            map.insert(name.to_string(), value.clone());
        }
    }
    map
}

/// Read the named environment variables once, into a map, merging
/// the `.env` file from the current working directory underneath:
/// a real process environment variable wins over the same key in the
/// file, and the file fills only the names the environment leaves
/// unset (see [`load_dotenv`]). A set variable that is not valid
/// UTF-8 is a `ConfigError` naming the variable — it must not
/// silently become "not set": the user configured a credential and
/// would get a confusing "nothing configured" outcome instead of an
/// error. The `.env` file loads only with explicit opt-in (see
/// [`load_dotenv`]) and only when it carries `GEECTL_*` keys; it
/// loads even when every requested variable is already set: once
/// opted into, a malformed file is a misconfiguration, not a value
/// the caller happens not to need. The file is read once per process
/// and the snapshot reused, so every call in a run sees the same
/// file.
///
/// Reading each variable once (instead of a check pass plus a read
/// pass) means resolution sees one consistent view of the
/// environment: a variable cannot be checked, then changed, then
/// read as a different value. Consumers read from the returned map
/// (see the `VarLookup` closures in the providers, and
/// `crate::config`).
pub(crate) fn snapshot_env(names: &[&str]) -> Result<HashMap<String, String>, GeectlError> {
    let mut real = HashMap::new();
    for name in names {
        if let Some(value) = std::env::var_os(name) {
            let value = value.into_string().map_err(|_| {
                GeectlError::new(ErrorKind::ConfigError, format!("{name} is not valid UTF-8"))
            })?;
            real.insert(name.to_string(), value);
        }
    }
    let dotenv = load_dotenv()?.unwrap_or_default();
    Ok(merge_snapshot(names, &real, &dotenv))
}

/// OBS credentials from the process environment. See
/// [`resolve_obs_for_url`] for the variables and the rules.
pub struct EnvObsAuth;

impl EnvObsAuth {
    /// OBS credentials for one API URL from the process
    /// environment: an API token (Bearer) or a username/password
    /// pair (Basic). See [`resolve_obs_for_url`] for the
    /// precedence and the fail-clear rule. `instances` is the
    /// config file's `[instances]` table: the alias variables for
    /// the matching alias are snapshotted alongside the plain ones,
    /// so resolution sees one consistent view of the environment.
    pub fn credentials_for_url(
        url: &str,
        default_url: &str,
        instances: &[(String, String)],
    ) -> Result<Credentials, GeectlError> {
        let mut names: Vec<String> = vec![
            OBS_USERNAME_VAR.to_string(),
            OBS_PASSWORD_VAR.to_string(),
            OBS_PASSWORD_FILE_VAR.to_string(),
            OBS_APIKEY_VAR.to_string(),
            OBS_APIKEY_FILE_VAR.to_string(),
        ];
        if let Some(alias) = alias_for_url(url, instances)? {
            let (username_var, password_var, password_file_var, apikey_var, apikey_file_var) =
                obs_alias_vars(alias);
            names.push(username_var);
            names.push(password_var);
            names.push(password_file_var);
            names.push(apikey_var);
            names.push(apikey_file_var);
        }
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let env = snapshot_env(&refs)?;
        resolve_obs_for_url(
            url,
            default_url,
            instances,
            &|name| env.get(name).cloned(),
            &read_secret_file_production,
        )
    }
}

/// Gitea Bearer auth from the process environment. See
/// [`resolve_gitea_bearer`].
pub struct EnvGiteaAuth;

impl AuthProvider for EnvGiteaAuth {
    fn credentials(&self) -> Result<Credentials, GeectlError> {
        let env = snapshot_env(&[GITEA_TOKEN_VAR, GITEA_TOKEN_FILE_VAR])?;
        resolve_gitea_bearer(&|name| env.get(name).cloned(), &read_secret_file_production)
    }
}

impl EnvGiteaAuth {
    /// Gitea Bearer auth for one Gitea URL from the process
    /// environment. See [`resolve_gitea_bearer_for_url`] for the
    /// precedence and the fail-clear rule. `default_url` is the
    /// effective configured Gitea URL — environment, then config
    /// file, then default — never the bare
    /// [`crate::config::DEFAULT_GITEA_URL`] constant when a custom
    /// URL is configured, or a custom single URL with a plain
    /// token would fail resolution. `instances` is the config
    /// file's `[instances]` table: the alias variables for the
    /// matching alias are snapshotted alongside the plain ones,
    /// so resolution sees one consistent view of the environment.
    pub fn credentials_for_url(
        url: &str,
        default_url: &str,
        instances: &[(String, String)],
    ) -> Result<Credentials, GeectlError> {
        let mut names: Vec<String> = vec![
            GITEA_TOKEN_VAR.to_string(),
            GITEA_TOKEN_FILE_VAR.to_string(),
        ];
        if let Some(alias) = alias_for_url(url, instances)? {
            let (token_var, token_file_var) = gitea_alias_vars(alias);
            names.push(token_var);
            names.push(token_file_var);
        }
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let env = snapshot_env(&refs)?;
        resolve_gitea_bearer_for_url(
            url,
            default_url,
            instances,
            &|name| env.get(name).cloned(),
            &read_secret_file_production,
        )
    }
}

/// Gitea SSH key from the process environment. See
/// [`resolve_gitea_ssh_key`].
pub struct GiteaSshKey;

impl AuthProvider for GiteaSshKey {
    fn credentials(&self) -> Result<Credentials, GeectlError> {
        let env = snapshot_env(&[GITEA_SSH_KEY_VAR])?;
        resolve_gitea_ssh_key(&|name| env.get(name).cloned(), &open_secret_file_production)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_auth_supplies_no_credentials() {
        let auth = NoAuth;
        assert!(matches!(auth.credentials().unwrap(), Credentials::None));
    }

    #[test]
    fn debug_output_redacts_secrets() {
        // A derived Debug would print the password and token. The
        // manual impl must never let a secret reach a log line.
        let basic = Credentials::Basic {
            username: "user".to_string(),
            password: "s3cr3t".to_string(),
        };
        // Exact rendering, not a substring check: a contains() check
        // passes even if the secret leaks in a field the test never
        // looks at.
        assert_eq!(
            format!("{basic:?}"),
            r#"Basic { username: "user", password: "[redacted]" }"#
        );

        let bearer = Credentials::Bearer {
            token: "tok3n".to_string(),
        };
        assert_eq!(format!("{bearer:?}"), r#"Bearer { token: "[redacted]" }"#);

        let key = Credentials::SshKey {
            path: PathBuf::from("/home/user/.ssh/id_ed25519"),
        };
        let rendered = format!("{key:?}");
        assert!(rendered.contains("id_ed25519"));
    }

    #[test]
    fn bearer_token_trailing_newline_is_trimmed() {
        assert_eq!(validate_bearer_token("abc123\n").unwrap(), "abc123");
        assert_eq!(validate_bearer_token("abc123\r\n").unwrap(), "abc123");
        // A trailing space is legitimate token content, not an
        // artifact: it must be preserved, not trimmed.
        assert_eq!(validate_bearer_token("abc123  ").unwrap(), "abc123  ");
    }

    #[test]
    fn bearer_token_with_control_char_is_rejected() {
        // A control character in the middle must be rejected, not
        // trimmed: it would corrupt the Authorization header.
        assert!(validate_bearer_token("abc\rdef").is_err());
        assert!(validate_bearer_token("abc\ndef").is_err());
        assert!(validate_bearer_token("").is_err());
        // Only CR/LF is trimmed, so a CR/LF-only token is empty after
        // trimming and rejected.
        assert!(validate_bearer_token("\r\n").is_err());
        // A whitespace-only token is never a real credential.
        assert!(validate_bearer_token("   ").is_err());
    }

    #[test]
    fn basic_username_with_colon_is_rejected() {
        // RFC 7617: the colon separates user-id from password.
        assert!(validate_basic("user:name", "pass").is_err());
    }

    #[test]
    fn basic_credentials_are_trimmed_and_checked() {
        let (username, password) = validate_basic("user\n", "pass\n").unwrap();
        assert_eq!(username, "user");
        assert_eq!(password, "pass");
        let (username, password) = validate_basic("user\r\n", "pass\r\n").unwrap();
        assert_eq!(username, "user");
        assert_eq!(password, "pass");
        // A trailing space is legitimate credential content.
        let (username, password) = validate_basic("user ", "pass ").unwrap();
        assert_eq!(username, "user ");
        assert_eq!(password, "pass ");
        // A whitespace-only username is never a real credential.
        assert!(validate_basic("   ", "pass").is_err());
        assert!(validate_basic("user", "pa\0ss").is_err());
        assert!(validate_basic("", "pass").is_err());
    }

    // --- Provider resolution tests (injected env/files, no real env) ---

    use std::collections::HashMap;

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn instances(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(alias, url)| (alias.to_string(), url.to_string()))
            .collect()
    }

    fn lookup(map: &HashMap<String, String>) -> impl Fn(&str) -> Option<String> + '_ {
        move |name| map.get(name).cloned()
    }

    fn files(pairs: &[(&str, &str)]) -> HashMap<PathBuf, String> {
        pairs
            .iter()
            .map(|(k, v)| (PathBuf::from(k), v.to_string()))
            .collect()
    }

    fn reader(map: &HashMap<PathBuf, String>) -> impl Fn(&Path) -> std::io::Result<String> + '_ {
        move |path| {
            map.get(path)
                .cloned()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no such file"))
        }
    }

    fn opener(
        map: &HashMap<PathBuf, String>,
        mode: Option<u32>,
    ) -> impl Fn(&Path) -> std::io::Result<(Option<u32>, String)> + '_ {
        move |path| {
            map.get(path)
                .cloned()
                .map(|content| (mode, content))
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no such file"))
        }
    }

    fn empty_files() -> HashMap<PathBuf, String> {
        HashMap::new()
    }

    #[test]
    fn same_api_url_matches_spelling_variants() {
        // The spelling does not matter: the default port and the
        // case of the host are the same instance.
        assert!(same_api_url("https://api.example.org", "https://api.example.org:443").unwrap());
        assert!(same_api_url("https://API.EXAMPLE.ORG", "https://api.example.org").unwrap());
        // A different scheme is a different instance even on the
        // same host and port: http and https never share credentials.
        assert!(!same_api_url("http://api.example.org", "https://api.example.org").unwrap());
        assert!(!same_api_url("http://api.example.org:443", "https://api.example.org").unwrap());
        // A different host or a different effective port is a
        // different instance.
        assert!(!same_api_url("https://build.example.org", "https://api.example.org").unwrap());
        assert!(!same_api_url("https://api.example.org:8443", "https://api.example.org").unwrap());
        assert!(!same_api_url("http://127.0.0.1:3001", "http://127.0.0.1:80").unwrap());
        // IPv6 hosts compare without their brackets.
        assert!(same_api_url("https://[::1]", "https://[::1]:443").unwrap());
        assert!(!same_api_url("https://[::1]", "https://[::2]").unwrap());
    }

    #[test]
    fn same_api_url_rejects_malformed_urls() {
        for bad in [
            "notaurl",
            "ftp://api.opensuse.org",
            "ftp://user:secret@api.opensuse.org",
            "https://",
            "https://user:secret@api.opensuse.org",
            "https://api.opensuse.org:0",
            "https://api.opensuse.org:notaport",
            "https://[::1",
            "https://api.opensuse.org?x=1",
            "https://api.opensuse.org#frag",
            "https://api.opensuse.org/path?x=1",
        ] {
            let err = same_api_url(bad, "https://api.opensuse.org").unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput, "{bad}");
            assert!(!err.context.contains("secret"), "{bad}");
        }
    }

    #[test]
    fn alias_for_url_matches_under_same_api_url() {
        let table = instances(&[("example", "https://api.example.org")]);
        // The spelling does not matter: the default port and the
        // case of the host are the same instance.
        assert_eq!(
            alias_for_url("https://api.example.org:443", &table).unwrap(),
            Some("example")
        );
        assert_eq!(
            alias_for_url("https://API.EXAMPLE.ORG", &table).unwrap(),
            Some("example")
        );
        // A different scheme or host is a different instance.
        assert_eq!(
            alias_for_url("http://api.example.org", &table).unwrap(),
            None
        );
        assert_eq!(
            alias_for_url("https://build.opensuse.org", &table).unwrap(),
            None
        );
        // Two aliases for one URL: the first in alias order wins.
        let table = instances(&[
            ("alpha", "https://api.example.org"),
            ("beta", "https://api.example.org"),
        ]);
        assert_eq!(
            alias_for_url("https://api.example.org", &table).unwrap(),
            Some("alpha")
        );
    }

    #[test]
    fn alias_for_url_skips_a_malformed_entry() {
        // One bad `[instances]` entry never poisons the lookup for
        // the others: it matches nothing, and a later entry still
        // matches. A query or fragment in the entry is refused at
        // this read boundary too: the URL would otherwise ride
        // along into every request built from the base.
        let table = instances(&[
            ("broken", "notaurl"),
            ("query", "https://api.example.org?x=1"),
            ("example", "https://api.example.org"),
        ]);
        assert_eq!(
            alias_for_url("https://api.example.org", &table).unwrap(),
            Some("example")
        );
        // A malformed lookup URL is still the caller's error.
        assert_eq!(
            alias_for_url("notaurl", &table).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            alias_for_url("https://api.example.org?x=1", &table)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn gitea_alias_vars_uppercase_the_alias_as_written() {
        assert_eq!(
            gitea_alias_vars("MyInstance"),
            (
                "GEECTL_GITEA_TOKEN_MYINSTANCE".to_string(),
                "GEECTL_GITEA_TOKEN_FILE_MYINSTANCE".to_string(),
            )
        );
    }

    #[test]
    fn gitea_alias_vars_win_over_plain_vars_for_their_url() {
        let map = vars(&[
            (GITEA_TOKEN_VAR, "plain-token"),
            ("GEECTL_GITEA_TOKEN_EXAMPLE", "target-token"),
        ]);
        let creds = resolve_gitea_bearer_for_url(
            "https://gitea.example.org",
            "https://src.opensuse.org",
            &instances(&[("example", "https://gitea.example.org")]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap();
        assert!(matches!(creds, Credentials::Bearer { ref token } if token == "target-token"));
    }

    #[test]
    fn gitea_alias_token_from_file_is_read() {
        let map = vars(&[(
            "GEECTL_GITEA_TOKEN_FILE_EXAMPLE",
            "/run/secrets/gitea-token",
        )]);
        let files = files(&[("/run/secrets/gitea-token", "file-token\n")]);
        let creds = resolve_gitea_bearer_for_url(
            "https://gitea.example.org",
            "https://src.opensuse.org",
            &instances(&[("example", "https://gitea.example.org")]),
            &lookup(&map),
            &reader(&files),
        )
        .unwrap();
        assert!(matches!(creds, Credentials::Bearer { ref token } if token == "file-token"));
    }

    #[test]
    fn gitea_alias_both_token_sources_is_config_error() {
        let map = vars(&[
            ("GEECTL_GITEA_TOKEN_EXAMPLE", "a"),
            (
                "GEECTL_GITEA_TOKEN_FILE_EXAMPLE",
                "/run/secrets/gitea-token",
            ),
        ]);
        let err = resolve_gitea_bearer_for_url(
            "https://gitea.example.org",
            "https://src.opensuse.org",
            &instances(&[("example", "https://gitea.example.org")]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("GEECTL_GITEA_TOKEN_EXAMPLE"));
        assert!(err.context.contains("GEECTL_GITEA_TOKEN_FILE_EXAMPLE"));
    }

    #[test]
    fn gitea_default_url_falls_back_to_plain_vars() {
        let map = vars(&[(GITEA_TOKEN_VAR, "plain-token")]);
        let creds = resolve_gitea_bearer_for_url(
            "https://src.opensuse.org",
            "https://src.opensuse.org",
            &instances(&[]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap();
        assert!(matches!(creds, Credentials::Bearer { ref token } if token == "plain-token"));
        // The same instance spelled with its default port still
        // takes the plain variables.
        let creds = resolve_gitea_bearer_for_url(
            "https://src.opensuse.org:443",
            "https://src.opensuse.org",
            &instances(&[]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap();
        assert!(matches!(creds, Credentials::Bearer { .. }));
    }

    #[test]
    fn gitea_alias_defined_but_unset_with_plain_vars_configured_names_the_alias_vars() {
        let map = vars(&[(GITEA_TOKEN_VAR, "plain-token")]);
        let err = resolve_gitea_bearer_for_url(
            "https://gitea.example.org",
            "https://src.opensuse.org",
            &instances(&[("example", "https://gitea.example.org")]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(
            err.context.contains("GEECTL_GITEA_TOKEN_EXAMPLE"),
            "alias token var name missing from the error"
        );
        // The default's credentials never leak into the message.
        assert!(!err.context.contains("plain-token"));
    }

    #[test]
    fn gitea_other_url_with_no_alias_names_the_instances_entry() {
        let map = vars(&[(GITEA_TOKEN_VAR, "plain-token")]);
        let err = resolve_gitea_bearer_for_url(
            "https://gitea.example.org",
            "https://src.opensuse.org",
            &instances(&[]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(
            err.context.contains("[instances]"),
            "instances entry missing from the error"
        );
        assert!(
            err.context.contains("https://gitea.example.org"),
            "URL missing from the error"
        );
        assert!(!err.context.contains("plain-token"));
    }

    #[test]
    fn gitea_unmatched_url_with_nothing_configured_is_none() {
        // Nothing configured anywhere: a second instance with no
        // token is public reads, not an error — the failure only
        // bites when credentials exist for another instance.
        let map = vars(&[]);
        let creds = resolve_gitea_bearer_for_url(
            "https://gitea.example.org",
            "https://src.opensuse.org",
            &instances(&[]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap();
        assert!(matches!(creds, Credentials::None));
    }

    /// The TOML string literal at the start of `text`: from the
    /// opening quote to its closing quote. A basic string (`"`)
    /// skips backslash escapes; a literal string (`'`, which the
    /// serializer uses when the value contains `"`) has none —
    /// so a `)` inside the value cannot truncate it either way.
    fn toml_string_literal(text: &str) -> &str {
        let bytes = text.as_bytes();
        let quote = *bytes.first().expect("empty TOML string literal");
        assert!(quote == b'"' || quote == b'\'');
        let mut index = 1;
        while index < bytes.len() {
            match bytes[index] {
                b'\\' if quote == b'"' => index += 2,
                b if b == quote => return &text[..=index],
                _ => index += 1,
            }
        }
        panic!("unterminated TOML string literal");
    }

    #[test]
    fn gitea_no_alias_guidance_example_is_valid_toml() {
        // The example must parse as TOML and round-trip the URL —
        // including a control character, which `Debug` would render
        // as `\u{...}` (not valid TOML), and a quote, which the
        // serializer renders as a literal string.
        for url in [
            "http://127.0.0.1\u{1}x:3000",
            "http://127.0.0.1\"x:3000",
            "http://127.0.0.1)x:3000",
        ] {
            let map = vars(&[(GITEA_TOKEN_VAR, "plain-token")]);
            let err = resolve_gitea_bearer_for_url(
                url,
                "https://src.opensuse.org",
                &instances(&[]),
                &lookup(&map),
                &reader(&empty_files()),
            )
            .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ConfigError);
            let example = toml_string_literal(
                err.context
                    .split("myinstance = ")
                    .nth(1)
                    .expect("guidance example missing"),
            );
            let doc: toml::Table = format!("myinstance = {example}").parse().unwrap();
            assert_eq!(doc["myinstance"].as_str(), Some(url));
        }
    }

    #[test]
    fn obs_basic_resolves_from_env() {
        let map = vars(&[(OBS_USERNAME_VAR, "alice"), (OBS_PASSWORD_VAR, "s3cr3t")]);
        let creds = resolve_obs_basic(&lookup(&map), &reader(&empty_files())).unwrap();
        assert!(
            matches!(creds, Credentials::Basic { ref username, ref password }
            if username == "alice" && password == "s3cr3t")
        );
    }

    #[test]
    fn obs_basic_resolves_password_from_file() {
        let map = vars(&[
            (OBS_USERNAME_VAR, "alice"),
            (OBS_PASSWORD_FILE_VAR, "/run/secrets/obs-password"),
        ]);
        let files = files(&[("/run/secrets/obs-password", "file-secret\n")]);
        let creds = resolve_obs_basic(&lookup(&map), &reader(&files)).unwrap();
        // The trailing newline in the file is an artifact, not the secret.
        assert!(
            matches!(creds, Credentials::Basic { ref username, ref password }
            if username == "alice" && password == "file-secret")
        );
    }

    #[test]
    fn obs_basic_unconfigured_is_none() {
        let map = vars(&[]);
        let creds = resolve_obs_basic(&lookup(&map), &reader(&empty_files())).unwrap();
        assert!(matches!(creds, Credentials::None));
    }

    #[test]
    fn obs_basic_both_password_sources_is_config_error() {
        let map = vars(&[
            (OBS_USERNAME_VAR, "alice"),
            (OBS_PASSWORD_VAR, "a"),
            (OBS_PASSWORD_FILE_VAR, "/run/secrets/obs-password"),
        ]);
        let err = resolve_obs_basic(&lookup(&map), &reader(&empty_files())).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains(OBS_PASSWORD_VAR));
        assert!(err.context.contains(OBS_PASSWORD_FILE_VAR));
    }

    #[test]
    fn obs_basic_username_without_password_is_config_error() {
        let map = vars(&[(OBS_USERNAME_VAR, "alice")]);
        let err = resolve_obs_basic(&lookup(&map), &reader(&empty_files())).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains(OBS_USERNAME_VAR));
    }

    #[test]
    fn obs_basic_password_without_username_is_config_error() {
        let map = vars(&[(OBS_PASSWORD_VAR, "s3cr3t")]);
        let err = resolve_obs_basic(&lookup(&map), &reader(&empty_files())).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains(OBS_USERNAME_VAR));
    }

    #[test]
    fn obs_basic_malformed_value_is_config_error_naming_the_source() {
        // A colon in the username fails validation; the provider reports
        // it as a config error naming the variable, not an auth failure
        // at the first request — and never the value.
        let map = vars(&[(OBS_USERNAME_VAR, "al:ice"), (OBS_PASSWORD_VAR, "s3cr3t")]);
        let err = resolve_obs_basic(&lookup(&map), &reader(&empty_files())).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains(OBS_USERNAME_VAR));
        assert!(!err.context.contains("al:ice"));
        assert!(!err.context.contains("s3cr3t"));
    }

    #[test]
    fn obs_basic_malformed_password_names_the_password_source() {
        // A bad password must be attributed to the password variable,
        // not the username variable (the pair's first var).
        let map = vars(&[(OBS_USERNAME_VAR, "alice"), (OBS_PASSWORD_VAR, "   ")]);
        let err = resolve_obs_basic(&lookup(&map), &reader(&empty_files())).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains(OBS_PASSWORD_VAR));
        assert!(!err.context.contains(OBS_USERNAME_VAR));
        assert!(!err.context.contains("alice"));
        // From a file, the file variable is named instead.
        let map = vars(&[
            (OBS_USERNAME_VAR, "alice"),
            (OBS_PASSWORD_FILE_VAR, "/run/secrets/obs-password"),
        ]);
        let files = files(&[("/run/secrets/obs-password", "   ")]);
        let err = resolve_obs_basic(&lookup(&map), &reader(&files)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        // The file variable is named. (The plain password variable
        // name is a prefix of the file variable name, so a negative
        // contains-check for it is meaningless here.)
        assert!(err.context.contains(OBS_PASSWORD_FILE_VAR));
    }

    #[test]
    fn obs_basic_unreadable_password_file_is_config_error() {
        let map = vars(&[
            (OBS_USERNAME_VAR, "alice"),
            (OBS_PASSWORD_FILE_VAR, "/run/secrets/missing"),
        ]);
        let err = resolve_obs_basic(&lookup(&map), &reader(&empty_files())).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("/run/secrets/missing"));
    }

    #[test]
    fn gitea_bearer_resolves_from_env() {
        let map = vars(&[(GITEA_TOKEN_VAR, "tok3n")]);
        let creds = resolve_gitea_bearer(&lookup(&map), &reader(&empty_files())).unwrap();
        assert!(matches!(creds, Credentials::Bearer { ref token } if token == "tok3n"));
    }

    #[test]
    fn gitea_bearer_resolves_token_from_file() {
        let map = vars(&[(GITEA_TOKEN_FILE_VAR, "/run/secrets/gitea-token")]);
        let files = files(&[("/run/secrets/gitea-token", "file-token\r\n")]);
        let creds = resolve_gitea_bearer(&lookup(&map), &reader(&files)).unwrap();
        assert!(matches!(creds, Credentials::Bearer { ref token } if token == "file-token"));
    }

    #[test]
    fn gitea_bearer_unconfigured_is_none() {
        let map = vars(&[]);
        let creds = resolve_gitea_bearer(&lookup(&map), &reader(&empty_files())).unwrap();
        assert!(matches!(creds, Credentials::None));
    }

    #[test]
    fn gitea_bearer_both_token_sources_is_config_error() {
        let map = vars(&[
            (GITEA_TOKEN_VAR, "a"),
            (GITEA_TOKEN_FILE_VAR, "/run/secrets/gitea-token"),
        ]);
        let err = resolve_gitea_bearer(&lookup(&map), &reader(&empty_files())).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains(GITEA_TOKEN_VAR));
    }

    #[test]
    fn gitea_bearer_empty_token_is_config_error() {
        let map = vars(&[(GITEA_TOKEN_VAR, "   ")]);
        let err = resolve_gitea_bearer(&lookup(&map), &reader(&empty_files())).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains(GITEA_TOKEN_VAR));
        assert!(!err.context.contains("tok3n"));
    }

    #[test]
    fn gitea_bearer_unreadable_token_file_is_config_error() {
        let map = vars(&[(GITEA_TOKEN_FILE_VAR, "/run/secrets/missing")]);
        let err = resolve_gitea_bearer(&lookup(&map), &reader(&empty_files())).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("/run/secrets/missing"));
    }

    #[test]
    fn obs_bearer_resolves_from_env() {
        let map = vars(&[(OBS_APIKEY_VAR, "tok3n")]);
        let creds = resolve_obs_bearer(&lookup(&map), &reader(&empty_files())).unwrap();
        assert!(matches!(creds, Credentials::Bearer { ref token } if token == "tok3n"));
    }

    #[test]
    fn obs_bearer_resolves_token_from_file() {
        let map = vars(&[(OBS_APIKEY_FILE_VAR, "/run/secrets/obs-token")]);
        let files = files(&[("/run/secrets/obs-token", "file-token\r\n")]);
        let creds = resolve_obs_bearer(&lookup(&map), &reader(&files)).unwrap();
        assert!(matches!(creds, Credentials::Bearer { ref token } if token == "file-token"));
    }

    #[test]
    fn obs_bearer_unconfigured_is_none() {
        let map = vars(&[]);
        let creds = resolve_obs_bearer(&lookup(&map), &reader(&empty_files())).unwrap();
        assert!(matches!(creds, Credentials::None));
    }

    #[test]
    fn obs_bearer_both_token_sources_is_config_error() {
        let map = vars(&[
            (OBS_APIKEY_VAR, "a"),
            (OBS_APIKEY_FILE_VAR, "/run/secrets/obs-token"),
        ]);
        let err = resolve_obs_bearer(&lookup(&map), &reader(&empty_files())).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains(OBS_APIKEY_VAR));
    }

    #[test]
    fn obs_bearer_empty_token_is_config_error() {
        let map = vars(&[(OBS_APIKEY_VAR, "   ")]);
        let err = resolve_obs_bearer(&lookup(&map), &reader(&empty_files())).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains(OBS_APIKEY_VAR));
    }

    #[test]
    fn obs_bearer_unreadable_token_file_is_config_error() {
        let map = vars(&[(OBS_APIKEY_FILE_VAR, "/run/secrets/missing")]);
        let err = resolve_obs_bearer(&lookup(&map), &reader(&empty_files())).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("/run/secrets/missing"));
    }

    #[test]
    fn basic_empty_password_is_rejected() {
        // An empty password is never a real credential: fail fast here
        // instead of sending "user:" and getting a 401 at the first
        // request.
        assert!(validate_basic("user", "").is_err());
        assert!(validate_basic("user", "\n").is_err());
        assert!(validate_basic("user", "   ").is_err());
        // A password with real content and a trailing space is fine.
        assert!(validate_basic("user", "pass ").is_ok());
    }

    const SSH_KEY_PEM: &str =
        "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n";
    const SSH_KEY_PATH: &str = "/home/user/.ssh/id_ed25519";

    #[test]
    fn gitea_ssh_key_resolves_a_valid_key() {
        let map = vars(&[(GITEA_SSH_KEY_VAR, SSH_KEY_PATH)]);
        let files = files(&[(SSH_KEY_PATH, SSH_KEY_PEM)]);
        let creds = resolve_gitea_ssh_key(&lookup(&map), &opener(&files, Some(0o100600))).unwrap();
        assert!(
            matches!(creds, Credentials::SshKey { ref path } if path == Path::new(SSH_KEY_PATH))
        );
    }

    #[test]
    fn gitea_ssh_key_unconfigured_is_none() {
        let map = vars(&[]);
        let creds = resolve_gitea_ssh_key(&lookup(&map), &opener(&empty_files(), None)).unwrap();
        assert!(matches!(creds, Credentials::None));
    }

    #[test]
    fn gitea_ssh_key_group_readable_is_config_error() {
        let map = vars(&[(GITEA_SSH_KEY_VAR, SSH_KEY_PATH)]);
        let files = files(&[(SSH_KEY_PATH, SSH_KEY_PEM)]);
        let err =
            resolve_gitea_ssh_key(&lookup(&map), &opener(&files, Some(0o100644))).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains(SSH_KEY_PATH));
        assert!(err.context.contains("chmod 600"));
    }

    #[test]
    fn gitea_ssh_key_owner_read_only_is_accepted() {
        let map = vars(&[(GITEA_SSH_KEY_VAR, SSH_KEY_PATH)]);
        let files = files(&[(SSH_KEY_PATH, SSH_KEY_PEM)]);
        let creds = resolve_gitea_ssh_key(&lookup(&map), &opener(&files, Some(0o100400))).unwrap();
        assert!(matches!(creds, Credentials::SshKey { .. }));
    }

    #[test]
    fn gitea_ssh_key_non_pem_content_is_config_error() {
        // A public key (or any non-key file) passed as a private key is
        // a misconfiguration, not a key.
        let map = vars(&[(GITEA_SSH_KEY_VAR, SSH_KEY_PATH)]);
        let files = files(&[(SSH_KEY_PATH, "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5 user@host\n")]);
        let err =
            resolve_gitea_ssh_key(&lookup(&map), &opener(&files, Some(0o100600))).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains(SSH_KEY_PATH));
    }

    #[test]
    fn gitea_ssh_key_unreadable_file_is_config_error() {
        let map = vars(&[(GITEA_SSH_KEY_VAR, SSH_KEY_PATH)]);
        let err = resolve_gitea_ssh_key(&lookup(&map), &opener(&empty_files(), None)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains(SSH_KEY_PATH));
    }

    #[test]
    fn gitea_ssh_key_without_unix_mode_skips_the_permission_check() {
        // On a platform without unix modes there is nothing to check.
        let map = vars(&[(GITEA_SSH_KEY_VAR, SSH_KEY_PATH)]);
        let files = files(&[(SSH_KEY_PATH, SSH_KEY_PEM)]);
        let creds = resolve_gitea_ssh_key(&lookup(&map), &opener(&files, None)).unwrap();
        assert!(matches!(creds, Credentials::SshKey { .. }));
    }

    #[test]
    fn gitea_token_from_file_error_names_the_file_variable() {
        // A malformed token from the file must name the file
        // variable, not the env variable it never came from.
        let map = vars(&[(GITEA_TOKEN_FILE_VAR, "/run/secrets/gitea-token")]);
        let files = files(&[("/run/secrets/gitea-token", "   \n")]);
        let err = resolve_gitea_bearer(&lookup(&map), &reader(&files)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        // The error names the file variable as the source — the
        // context starts with it (config_error formats
        // "{source}: ..."). A whitespace-only token is rejected by
        // validate_bearer_token.
        assert!(
            err.context.starts_with(GITEA_TOKEN_FILE_VAR),
            "{}",
            err.context
        );
    }

    #[test]
    fn gitea_ssh_key_symlink_is_rejected() {
        // A symlink is not the key itself: its target can change
        // under us. The mode of a symlink is 0o120777.
        let map = vars(&[(GITEA_SSH_KEY_VAR, SSH_KEY_PATH)]);
        let files = files(&[(SSH_KEY_PATH, SSH_KEY_PEM)]);
        let err =
            resolve_gitea_ssh_key(&lookup(&map), &opener(&files, Some(0o120777))).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(
            err.context.contains("not a regular file"),
            "{}",
            err.context
        );
    }

    #[test]
    fn gitea_ssh_key_mode_diagnostic_shows_permission_bits_only() {
        // The diagnostic must show the permission bits (644), not
        // the full mode including the file-type bits (100644).
        let map = vars(&[(GITEA_SSH_KEY_VAR, SSH_KEY_PATH)]);
        let files = files(&[(SSH_KEY_PATH, SSH_KEY_PEM)]);
        let err =
            resolve_gitea_ssh_key(&lookup(&map), &opener(&files, Some(0o100644))).unwrap_err();
        assert!(err.context.contains("mode 644"), "{}", err.context);
        assert!(!err.context.contains("100644"), "{}", err.context);
    }

    #[test]
    fn gitea_ssh_key_pem_with_bom_and_leading_space_is_accepted() {
        let map = vars(&[(GITEA_SSH_KEY_VAR, SSH_KEY_PATH)]);
        let files = files(&[(SSH_KEY_PATH, &format!("\u{feff}  \n{SSH_KEY_PEM}"))]);
        let creds = resolve_gitea_ssh_key(&lookup(&map), &opener(&files, Some(0o100600))).unwrap();
        assert!(matches!(creds, Credentials::SshKey { .. }));
    }

    #[test]
    fn gitea_ssh_key_pem_without_end_marker_is_rejected() {
        // A BEGIN without an END is a truncated file, not a key.
        let map = vars(&[(GITEA_SSH_KEY_VAR, SSH_KEY_PATH)]);
        let files = files(&[(SSH_KEY_PATH, "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n")]);
        let err =
            resolve_gitea_ssh_key(&lookup(&map), &opener(&files, Some(0o100600))).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
    }

    // --- Production secret-file reader tests (real temp files) ---

    use std::sync::atomic::{AtomicU64, Ordering};

    static SECRET_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A temp file under the system temp dir, removed on drop.
    struct TempSecretFile(PathBuf);

    impl TempSecretFile {
        fn write(contents: &[u8]) -> Self {
            let n = SECRET_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("geectl-secret-test-{}-{n}", std::process::id()));
            std::fs::write(&path, contents).expect("write temp secret file");
            TempSecretFile(path)
        }
    }

    impl Drop for TempSecretFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn production_secret_reader_accepts_a_small_file() {
        let file = TempSecretFile::write(b"tok3n\n");
        assert_eq!(read_secret_file_production(&file.0).unwrap(), "tok3n\n");
    }

    #[test]
    fn production_secret_reader_rejects_an_oversize_file() {
        let file = TempSecretFile::write(&vec![b'x'; MAX_SECRET_FILE_BYTES + 1]);
        let err = read_secret_file_production(&file.0).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    #[cfg(unix)]
    fn production_secret_reader_rejects_a_non_regular_file() {
        // /dev/null opens fine but is not a regular file.
        let err = read_secret_file_production(Path::new("/dev/null")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    #[cfg(unix)]
    fn production_secret_opener_returns_the_open_files_mode() {
        use std::os::unix::fs::PermissionsExt;
        let file = TempSecretFile::write(b"tok3n\n");
        std::fs::set_permissions(&file.0, std::fs::Permissions::from_mode(0o600)).unwrap();
        let (mode, content) = open_secret_file_production(&file.0).unwrap();
        assert_eq!(content, "tok3n\n");
        assert_eq!(mode, Some(0o100600));
    }

    #[test]
    #[cfg(unix)]
    fn production_secret_opener_refuses_a_symlink() {
        // The opener follows the link, but the lstat identity is the
        // link, not the file it opened — refused, not silently read.
        let file = TempSecretFile::write(b"tok3n\n");
        let link = std::env::temp_dir().join(format!(
            "geectl-secret-link-{}-{}",
            std::process::id(),
            SECRET_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::os::unix::fs::symlink(&file.0, &link).unwrap();
        let err = open_secret_file_production(&link).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        let _ = std::fs::remove_file(&link);
    }

    #[test]
    #[cfg(unix)]
    fn production_secret_opener_refuses_a_directory() {
        let err = open_secret_file_production(&std::env::temp_dir()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    #[cfg(unix)]
    fn production_secret_opener_does_not_block_on_a_fifo() {
        // A FIFO with no writer blocks a plain open forever; the
        // non-blocking open returns at once, and the regular-file
        // check refuses it. If the open ever blocked, this test
        // would hang rather than fail — that is the signal.
        let path = std::env::temp_dir().join(format!(
            "geectl-secret-fifo-{}-{}",
            std::process::id(),
            SECRET_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let status = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .expect("mkfifo must be available to create the test FIFO");
        assert!(status.success(), "mkfifo must create the test FIFO");

        let err = open_secret_file_production(&path).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn obs_alias_vars_uppercase_the_alias_as_written() {
        assert_eq!(
            obs_alias_vars("MyInstance"),
            (
                "GEECTL_OBS_USERNAME_MYINSTANCE".to_string(),
                "GEECTL_OBS_PASSWORD_MYINSTANCE".to_string(),
                "GEECTL_OBS_PASSWORD_FILE_MYINSTANCE".to_string(),
                "GEECTL_OBS_APIKEY_MYINSTANCE".to_string(),
                "GEECTL_OBS_APIKEY_FILE_MYINSTANCE".to_string(),
            )
        );
    }

    #[test]
    fn alias_vars_win_over_plain_vars_for_their_url() {
        let map = vars(&[
            (OBS_USERNAME_VAR, "plain-user"),
            (OBS_PASSWORD_VAR, "plain-secret"),
            ("GEECTL_OBS_USERNAME_EXAMPLE", "target-user"),
            ("GEECTL_OBS_PASSWORD_EXAMPLE", "target-secret"),
        ]);
        let creds = resolve_obs_basic_for_url(
            "https://api.example.org",
            "https://api.opensuse.org",
            &instances(&[("example", "https://api.example.org")]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap();
        assert!(
            matches!(creds, Credentials::Basic { ref username, ref password }
            if username == "target-user" && password == "target-secret")
        );
    }

    #[test]
    fn default_url_falls_back_to_plain_vars() {
        let map = vars(&[
            (OBS_USERNAME_VAR, "plain-user"),
            (OBS_PASSWORD_VAR, "plain-secret"),
        ]);
        let creds = resolve_obs_basic_for_url(
            "https://api.opensuse.org",
            "https://api.opensuse.org",
            &instances(&[]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap();
        assert!(
            matches!(creds, Credentials::Basic { ref username, ref password }
            if username == "plain-user" && password == "plain-secret")
        );
        // The same instance spelled with its default port still
        // takes the plain variables.
        let creds = resolve_obs_basic_for_url(
            "https://api.opensuse.org:443",
            "https://api.opensuse.org",
            &instances(&[]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap();
        assert!(matches!(creds, Credentials::Basic { .. }));
    }

    #[test]
    fn alias_defined_but_unset_with_plain_vars_configured_names_the_alias_vars() {
        let map = vars(&[
            (OBS_USERNAME_VAR, "plain-user"),
            (OBS_PASSWORD_VAR, "plain-secret"),
        ]);
        let err = resolve_obs_basic_for_url(
            "https://api.example.org",
            "https://api.opensuse.org",
            &instances(&[("example", "https://api.example.org")]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(
            err.context.contains("GEECTL_OBS_USERNAME_EXAMPLE"),
            "alias username var name missing from the error"
        );
        assert!(
            err.context.contains("GEECTL_OBS_PASSWORD_EXAMPLE"),
            "alias password var name missing from the error"
        );
        // The default's credentials never leak into the message.
        assert!(!err.context.contains("plain-user"));
        assert!(!err.context.contains("plain-secret"));
    }

    #[test]
    fn other_url_with_no_alias_names_the_instances_entry() {
        let map = vars(&[
            (OBS_USERNAME_VAR, "plain-user"),
            (OBS_PASSWORD_VAR, "plain-secret"),
        ]);
        let err = resolve_obs_basic_for_url(
            "https://api.example.org",
            "https://api.opensuse.org",
            &instances(&[]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(
            err.context.contains("[instances]"),
            "instances entry missing from the error"
        );
        assert!(
            err.context.contains("https://api.example.org"),
            "URL missing from the error"
        );
        assert!(!err.context.contains("plain-user"));
        assert!(!err.context.contains("plain-secret"));
    }

    #[test]
    fn no_alias_guidance_quotes_the_url_for_toml() {
        // A quote in the URL must not break the TOML example in
        // the error: the TOML serializer renders it as a literal
        // string, so the quote needs no escape and the line stays
        // one valid TOML assignment.
        let map = vars(&[
            (OBS_USERNAME_VAR, "plain-user"),
            (OBS_PASSWORD_VAR, "plain-secret"),
        ]);
        let err = resolve_obs_basic_for_url(
            "http://127.0.0.1\"x:3000",
            "https://api.opensuse.org",
            &instances(&[]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        // The TOML serializer renders a string containing a
        // quote as a literal string ('...'), not a basic string
        // with an escape — still valid TOML (see
        // no_alias_guidance_example_is_valid_toml).
        assert!(
            err.context
                .contains("myinstance = 'http://127.0.0.1\"x:3000'"),
            "TOML example did not use the expected literal-string rendering: {}",
            err.context
        );
    }

    #[test]
    fn no_alias_guidance_example_is_valid_toml() {
        // The example must parse as TOML and round-trip the URL —
        // including a control character, which `Debug` would render
        // as `\u{...}` (not valid TOML), and a quote, which the
        // serializer renders as a literal string.
        for url in ["http://127.0.0.1\u{1}x:3000", "http://127.0.0.1\"x:3000"] {
            let map = vars(&[
                (OBS_USERNAME_VAR, "plain-user"),
                (OBS_PASSWORD_VAR, "plain-secret"),
            ]);
            let err = resolve_obs_basic_for_url(
                url,
                "https://api.opensuse.org",
                &instances(&[]),
                &lookup(&map),
                &reader(&empty_files()),
            )
            .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ConfigError);
            let example = err
                .context
                .split("myinstance = ")
                .nth(1)
                .expect("guidance example missing")
                .split(") and set")
                .next()
                .expect("guidance example unterminated");
            let doc = format!("[instances]\nmyinstance = {example}\n");
            let value: toml::Value = toml::from_str(&doc).expect("example must parse as TOML");
            assert_eq!(value["instances"]["myinstance"].as_str(), Some(url));
        }
    }

    #[test]
    fn other_url_with_nothing_configured_is_none() {
        let map = vars(&[]);
        let creds = resolve_obs_basic_for_url(
            "https://api.example.org",
            "https://api.opensuse.org",
            &instances(&[("example", "https://api.example.org")]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap();
        assert!(matches!(creds, Credentials::None));
    }

    #[test]
    fn alias_half_configured_is_config_error_naming_the_alias_var() {
        let map = vars(&[("GEECTL_OBS_USERNAME_EXAMPLE", "target-user")]);
        let err = resolve_obs_basic_for_url(
            "https://api.example.org",
            "https://api.opensuse.org",
            &instances(&[("example", "https://api.example.org")]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(
            err.context.contains("GEECTL_OBS_USERNAME_EXAMPLE"),
            "alias username var name missing from the error"
        );
    }

    #[test]
    fn different_url_half_configured_plain_pair_names_the_alias_vars() {
        let map = vars(&[(OBS_USERNAME_VAR, "plain-user")]);
        let err = resolve_obs_basic_for_url(
            "https://api.example.org",
            "https://api.opensuse.org",
            &instances(&[("example", "https://api.example.org")]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(
            err.context.contains("GEECTL_OBS_USERNAME_EXAMPLE"),
            "alias username var name missing from the error"
        );
        assert!(
            err.context.contains("GEECTL_OBS_PASSWORD_EXAMPLE"),
            "alias password var name missing from the error"
        );
    }

    #[test]
    fn alias_password_file_is_read() {
        let map = vars(&[
            ("GEECTL_OBS_USERNAME_TARGET", "admin"),
            (
                "GEECTL_OBS_PASSWORD_FILE_TARGET",
                "/run/secrets/target-password",
            ),
        ]);
        let files = files(&[("/run/secrets/target-password", "target-secret\n")]);
        let creds = resolve_obs_basic_for_url(
            "http://127.0.0.1:3001",
            "http://127.0.0.1:3000",
            &instances(&[("target", "http://127.0.0.1:3001")]),
            &lookup(&map),
            &reader(&files),
        )
        .unwrap();
        assert!(
            matches!(creds, Credentials::Basic { ref username, ref password }
            if username == "admin" && password == "target-secret")
        );
    }

    #[test]
    fn ports_distinguish_instances_on_one_host() {
        // The two live stacks share 127.0.0.1 and differ only by
        // port: the alias variables for one must not apply to the
        // other.
        let table = instances(&[
            ("live1", "http://127.0.0.1:3000"),
            ("live2", "http://127.0.0.1:3001"),
        ]);
        let map = vars(&[
            ("GEECTL_OBS_USERNAME_LIVE2", "target-user"),
            ("GEECTL_OBS_PASSWORD_LIVE2", "target-secret"),
        ]);
        // Port 3000 sees none of the 3001 variables.
        let creds = resolve_obs_basic_for_url(
            "http://127.0.0.1:3000",
            "http://127.0.0.1:3000",
            &table,
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap();
        assert!(matches!(creds, Credentials::None));
        // Port 3001 sees its own.
        let creds = resolve_obs_basic_for_url(
            "http://127.0.0.1:3001",
            "http://127.0.0.1:3000",
            &table,
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap();
        assert!(
            matches!(creds, Credentials::Basic { ref username, ref password }
            if username == "target-user" && password == "target-secret")
        );
    }

    #[test]
    fn obs_token_alias_vars_win_over_plain_vars_for_their_url() {
        let map = vars(&[
            (OBS_USERNAME_VAR, "plain-user"),
            (OBS_PASSWORD_VAR, "plain-secret"),
            ("GEECTL_OBS_APIKEY_EXAMPLE", "alias-token"),
        ]);
        let creds = resolve_obs_for_url(
            "https://api.example.org",
            "https://api.opensuse.org",
            &instances(&[("example", "https://api.example.org")]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap();
        assert!(matches!(creds, Credentials::Bearer { ref token } if token == "alias-token"));
    }

    #[test]
    fn obs_token_plain_token_for_default_url() {
        let map = vars(&[(OBS_APIKEY_VAR, "plain-token")]);
        let creds = resolve_obs_for_url(
            "https://api.opensuse.org",
            "https://api.opensuse.org",
            &instances(&[]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap();
        assert!(matches!(creds, Credentials::Bearer { ref token } if token == "plain-token"));
    }

    #[test]
    fn obs_token_and_basic_both_set_for_one_alias_is_config_error() {
        let map = vars(&[
            ("GEECTL_OBS_APIKEY_EXAMPLE", "alias-token"),
            ("GEECTL_OBS_USERNAME_EXAMPLE", "alias-user"),
            ("GEECTL_OBS_PASSWORD_EXAMPLE", "alias-secret"),
        ]);
        let err = resolve_obs_for_url(
            "https://api.example.org",
            "https://api.opensuse.org",
            &instances(&[("example", "https://api.example.org")]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("GEECTL_OBS_APIKEY_EXAMPLE"));
        assert!(err.context.contains("GEECTL_OBS_USERNAME_EXAMPLE"));
        assert!(!err.context.contains("alias-token"));
        assert!(!err.context.contains("alias-secret"));
    }

    #[test]
    fn obs_token_alias_defined_but_unset_names_the_alias_token_var() {
        let map = vars(&[(OBS_APIKEY_VAR, "plain-token")]);
        let err = resolve_obs_for_url(
            "https://api.example.org",
            "https://api.opensuse.org",
            &instances(&[("example", "https://api.example.org")]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(
            err.context.contains("GEECTL_OBS_APIKEY_EXAMPLE"),
            "alias token var name missing from the error"
        );
        // The default's token never leaks into the message.
        assert!(!err.context.contains("plain-token"));
    }

    #[test]
    fn obs_token_unmatched_url_names_the_alias_token_var() {
        let map = vars(&[(OBS_APIKEY_VAR, "plain-token")]);
        let err = resolve_obs_for_url(
            "https://api.example.org",
            "https://api.opensuse.org",
            &instances(&[]),
            &lookup(&map),
            &reader(&empty_files()),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(
            err.context.contains("GEECTL_OBS_APIKEY[_FILE]_<ALIAS>"),
            "alias token var form missing from the error"
        );
        assert!(
            err.context.contains("[instances]"),
            "instances entry missing from the error"
        );
        assert!(!err.context.contains("plain-token"));
    }

    // --- `.env` file tests ---

    #[test]
    fn dotenv_parses_minimal_syntax() {
        let content = "# a comment\n\
            \n\
            GEECTL_OBS_URL=https://api.opensuse.org\n\
            export GEECTL_FORMAT=text\n\
            GEECTL_GITEA_TOKEN='tok en'\n\
            GEECTL_OBS_USERNAME=\"user name\"\n\
            GEECTL_OBS_PASSWORD=secret # trailing comment\n\
            GEECTL_DEBUG=1\n\
            GEECTL_DEBUG=0\n";
        let map = parse_dotenv(content, Path::new(".env")).unwrap();
        assert_eq!(map["GEECTL_OBS_URL"], "https://api.opensuse.org");
        assert_eq!(map["GEECTL_FORMAT"], "text");
        assert_eq!(map["GEECTL_GITEA_TOKEN"], "tok en");
        assert_eq!(map["GEECTL_OBS_USERNAME"], "user name");
        assert_eq!(map["GEECTL_OBS_PASSWORD"], "secret");
        // A duplicate key keeps its last value.
        assert_eq!(map["GEECTL_DEBUG"], "0");
    }

    #[test]
    fn dotenv_ignores_non_geectl_keys() {
        let content = "OTHER=1\n\
            GIT_DIR=/tmp/repo\n\
            https_proxy=http://proxy.example.org:8080\n\
            geectl_lower=bar\n\
            export OTHER=2\n\
            FOO BAR=baz\n";
        // `exportGEECTL_A` is not here: a key glued to `export` is a
        // malformed geectl line, not an ignored key (see
        // `dotenv_glued_export_is_malformed`).
        let map = parse_dotenv(content, Path::new(".env")).unwrap();
        assert!(map.is_empty(), "non-GEECTL keys must be ignored: {map:?}");
    }

    #[test]
    fn dotenv_hash_and_dollar_are_literal() {
        let map = parse_dotenv(
            "GEECTL_A=a#b\nGEECTL_B=\"a#b\"\nGEECTL_C='a$b'\nGEECTL_D=a=b=c\n",
            Path::new(".env"),
        )
        .unwrap();
        // A `#` not preceded by whitespace is part of the value,
        // in and out of quotes; a `$` is never interpolated.
        assert_eq!(map["GEECTL_A"], "a#b");
        assert_eq!(map["GEECTL_B"], "a#b");
        assert_eq!(map["GEECTL_C"], "a$b");
        // The value splits on the first `=` only.
        assert_eq!(map["GEECTL_D"], "a=b=c");
    }

    #[test]
    fn dotenv_export_prefix_accepts_a_tab() {
        let map = parse_dotenv("export\tGEECTL_A=1\n", Path::new(".env")).unwrap();
        assert_eq!(map["GEECTL_A"], "1");
    }

    #[test]
    fn dotenv_tab_before_hash_starts_a_comment() {
        let map = parse_dotenv("GEECTL_A=value\t# comment\n", Path::new(".env")).unwrap();
        assert_eq!(map["GEECTL_A"], "value");
    }

    #[test]
    fn dotenv_hash_after_equals_is_empty() {
        let map = parse_dotenv(
            "GEECTL_A=# comment\nGEECTL_B=#comment\n",
            Path::new(".env"),
        )
        .unwrap();
        assert_eq!(map["GEECTL_A"], "");
        assert_eq!(map["GEECTL_B"], "");
    }

    #[test]
    fn dotenv_trims_whitespace_bom_crlf_and_empty_values() {
        let content = "\u{feff}  GEECTL_A = spaced  \r\n\
            export  GEECTL_B=\"quoted\" # note\r\n\
            GEECTL_C=\r\n\
            GEECTL_D=''\n";
        let map = parse_dotenv(content, Path::new(".env")).unwrap();
        assert_eq!(map["GEECTL_A"], "spaced");
        assert_eq!(map["GEECTL_B"], "quoted");
        assert_eq!(map["GEECTL_C"], "");
        assert_eq!(map["GEECTL_D"], "");
    }

    #[test]
    fn dotenv_line_without_equals_is_a_config_error_naming_file_and_line() {
        let err =
            parse_dotenv("GEECTL_A=sekrit-value\nNOT_A_PAIR\n", Path::new(".env")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains(".env"), "{}", err.context);
        assert!(err.context.contains("line 2"), "{}", err.context);
        assert!(!err.context.contains("sekrit-value"), "{}", err.context);
    }

    #[test]
    fn dotenv_unterminated_quote_is_a_config_error() {
        let err = parse_dotenv("GEECTL_A=\"abc\n", Path::new(".env")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("line 1"), "{}", err.context);
        assert!(
            err.context.contains("unterminated quote"),
            "{}",
            err.context
        );
        assert!(!err.context.contains("abc"), "{}", err.context);
    }

    #[test]
    fn dotenv_text_after_a_quoted_value_is_a_config_error() {
        let err = parse_dotenv("GEECTL_A=\"v\" junk\n", Path::new(".env")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("line 1"), "{}", err.context);
        assert!(
            err.context.contains("unexpected text after quoted value"),
            "{}",
            err.context
        );
    }

    #[test]
    fn dotenv_invalid_geectl_key_is_a_config_error() {
        for bad in [
            "GEECTL_FOO-BAR=sekrit\n",
            "GEECTL_=sekrit\n",
            // Near-miss prefixes: a typo for a geectl variable.
            "GEECCTL_FOO=sekrit\n",
            "GEECT_FOO=sekrit\n",
            "GEECTL=sekrit\n",
            "GEECTLX=sekrit\n",
            "GEECTLFOO=sekrit\n",
            "GEEC-FOO=sekrit\n",
        ] {
            let err = parse_dotenv(bad, Path::new(".env")).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ConfigError);
            assert!(err.context.contains("line 1"), "{}", err.context);
            assert!(
                err.context.contains("invalid variable name"),
                "{}",
                err.context
            );
            assert!(!err.context.contains("sekrit"), "{}", err.context);
        }
    }

    #[test]
    fn merge_snapshot_prefers_real_env_over_dotenv() {
        let real = vars(&[("GEECTL_OBS_URL", "https://real.example.org")]);
        let dotenv = vars(&[
            ("GEECTL_OBS_URL", "https://file.example.org"),
            ("GEECTL_FORMAT", "text"),
        ]);
        let map = merge_snapshot(
            &["GEECTL_OBS_URL", "GEECTL_FORMAT", "GEECTL_DEBUG"],
            &real,
            &dotenv,
        );
        assert_eq!(map["GEECTL_OBS_URL"], "https://real.example.org");
        // The file fills only the names the environment leaves unset.
        assert_eq!(map["GEECTL_FORMAT"], "text");
        assert!(!map.contains_key("GEECTL_DEBUG"));
    }

    fn write_dotenv(dir: &tempfile::TempDir, content: &[u8]) -> PathBuf {
        let path = dir.path().join(".env");
        std::fs::write(&path, content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        path
    }

    #[test]
    fn load_dotenv_at_returns_none_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_dotenv_at(&dir.path().join(".env")).unwrap().is_none());
    }

    #[test]
    fn load_dotenv_at_loads_a_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_dotenv(
            &dir,
            b"GEECTL_OBS_URL=https://api.opensuse.org\nOTHER=ignored\n",
        );
        let map = load_dotenv_at(&path).unwrap().unwrap();
        assert_eq!(map["GEECTL_OBS_URL"], "https://api.opensuse.org");
        assert_eq!(map.len(), 1, "only GEECTL_* keys are honoured");
    }

    #[test]
    #[cfg(unix)]
    fn load_dotenv_at_refuses_a_group_readable_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = write_dotenv(&dir, b"GEECTL_OBS_URL=https://api.opensuse.org\n");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = load_dotenv_at(&path).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(
            err.context.contains(&path.display().to_string()),
            "{}",
            err.context
        );
        assert!(
            err.context.contains("readable by group or other"),
            "{}",
            err.context
        );
        assert!(
            !err.context.contains("https://api.opensuse.org"),
            "{}",
            err.context
        );
    }

    #[test]
    fn load_dotenv_at_reports_a_malformed_file_with_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_dotenv(&dir, b"GEECTL_A=sekrit-value\nBROKEN LINE\n");
        let err = load_dotenv_at(&path).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(
            err.context.contains(&path.display().to_string()),
            "{}",
            err.context
        );
        assert!(err.context.contains("line 2"), "{}", err.context);
        assert!(!err.context.contains("sekrit-value"), "{}", err.context);
    }

    #[test]
    #[cfg(unix)]
    fn load_dotenv_at_refuses_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let target = write_dotenv(&dir, b"GEECTL_A=sekrit-value\n");
        let link = dir.path().join(".env-link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let err = load_dotenv_at(&link).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(
            err.context.contains(&link.display().to_string()),
            "{}",
            err.context
        );
        assert!(!err.context.contains("sekrit-value"), "{}", err.context);
    }

    #[test]
    fn load_dotenv_at_rejects_non_utf8_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_dotenv(&dir, b"GEECTL_A=\xff\n");
        let err = load_dotenv_at(&path).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(
            err.context.contains(&path.display().to_string()),
            "{}",
            err.context
        );
    }

    #[test]
    fn load_dotenv_at_refuses_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let err = load_dotenv_at(dir.path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(
            err.context.contains(&dir.path().display().to_string()),
            "{}",
            err.context
        );
    }

    #[test]
    fn load_dotenv_at_refuses_an_oversize_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_dotenv(&dir, &vec![b'x'; MAX_SECRET_FILE_BYTES + 1]);
        let err = load_dotenv_at(&path).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(
            err.context.contains(&path.display().to_string()),
            "{}",
            err.context
        );
    }

    #[test]
    fn dotenv_opt_in_requires_the_literal_one() {
        assert!(dotenv_opted_in_from(Some("1")));
        assert!(!dotenv_opted_in_from(None));
        assert!(!dotenv_opted_in_from(Some("")));
        assert!(!dotenv_opted_in_from(Some("0")));
        assert!(!dotenv_opted_in_from(Some("yes")));
    }

    #[test]
    fn dotenv_carries_geectl_keys_spots_assignments_only() {
        assert!(dotenv_carries_geectl_keys("GEECTL_A=1\n"));
        assert!(dotenv_carries_geectl_keys("export GEECTL_A=1\n"));
        assert!(dotenv_carries_geectl_keys("export\tGEECTL_A=1\n"));
        assert!(dotenv_carries_geectl_keys("exportGEECTL_A=1\n"));
        assert!(dotenv_carries_geectl_keys("OTHER=1\nGEECTL_A\n"));
        assert!(!dotenv_carries_geectl_keys("OTHER=1\n"));
        assert!(!dotenv_carries_geectl_keys("# GEECTL_A=1\n"));
        assert!(!dotenv_carries_geectl_keys("GEECTLX=1\n"));
        assert!(!dotenv_carries_geectl_keys(""));
    }

    #[test]
    fn dotenv_glued_export_is_malformed() {
        for bad in ["exportGEECTL_A=1\n", "exportGEEC_A=1\n"] {
            let err = parse_dotenv(bad, Path::new(".env")).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ConfigError);
            assert!(
                err.context.contains("invalid variable name"),
                "{}",
                err.context
            );
        }
        // The spaced prefix stays valid.
        let map = parse_dotenv("export GEECTL_A=1\n", Path::new(".env")).unwrap();
        assert_eq!(map["GEECTL_A"], "1");
    }

    #[test]
    fn load_dotenv_at_ignores_a_file_without_geectl_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_dotenv(&dir, b"OTHER=1\nBROKEN LINE\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        // No GEECTL_* key: neither the loose mode nor the malformed
        // line matters — the file is not a geectl file.
        assert!(load_dotenv_at(&path).unwrap().is_none());
    }
}
