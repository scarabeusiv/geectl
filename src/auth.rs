//! The auth seam and credential validation.
//!
//! This module is the credential design: the providers that supply
//! credentials from environment variables or mounted secret files,
//! and the validation that keeps a malformed credential from
//! reaching an Authorization header. It deliberately stores no
//! credentials and reads none from a config file, a prompt, or
//! `/dev/tty`: providers supply credentials from environment
//! variables or mounted secret files, and the HTTP client applies
//! them. There is no plaintext fallback and no obfuscation option,
//! and a credential-shaped value in a config file is a refusal to
//! run (see the design doc and `crate::config`).
//!
//! Credential types covered:
//! - OBS: username/password (Basic auth) — OBS has no API key.
//! - Gitea: API token (Bearer), plus SSH keys for git operations.
//!
//! [`NoAuth`] is the only provider that supplies no credentials.
//! It exists so the HTTP client compiles and runs without
//! credentials; it is not a credential store.

use crate::error::{ErrorKind, GeectlError};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

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
    let username = vars(OBS_USERNAME_VAR);
    let password = vars(OBS_PASSWORD_VAR);
    let password_file = vars(OBS_PASSWORD_FILE_VAR);
    if password.is_some() && password_file.is_some() {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("both {OBS_PASSWORD_VAR} and {OBS_PASSWORD_FILE_VAR} are set; set exactly one"),
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
                .map_err(|e| config_error(OBS_USERNAME_VAR, e))?;
            let password_source = if password_from_file {
                OBS_PASSWORD_FILE_VAR
            } else {
                OBS_PASSWORD_VAR
            };
            let password = validate_basic_password(&password)
                .map_err(|e| config_error(password_source, e))?;
            Ok(Credentials::Basic { username, password })
        }
        (Some(_), None) => Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!(
                "{OBS_USERNAME_VAR} is set but no password source is ({OBS_PASSWORD_VAR} or {OBS_PASSWORD_FILE_VAR})"
            ),
        )),
        (None, Some(_)) => Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!(
                "a password source is set ({OBS_PASSWORD_VAR} or {OBS_PASSWORD_FILE_VAR}) but {OBS_USERNAME_VAR} is not"
            ),
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
    let token = vars(GITEA_TOKEN_VAR);
    let token_file = vars(GITEA_TOKEN_FILE_VAR);
    if token.is_some() && token_file.is_some() {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("both {GITEA_TOKEN_VAR} and {GITEA_TOKEN_FILE_VAR} are set; set exactly one"),
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
                GITEA_TOKEN_FILE_VAR
            } else {
                GITEA_TOKEN_VAR
            };
            let token = validate_bearer_token(&token).map_err(|e| config_error(token_source, e))?;
            Ok(Credentials::Bearer { token })
        }
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

/// Read the named environment variables once, into a map. A set
/// variable that is not valid UTF-8 is a `ConfigError` naming the
/// variable — it must not silently become "not set": the user
/// configured a credential and would get a confusing "nothing
/// configured" outcome instead of an error.
///
/// Reading each variable once (instead of a check pass plus a read
/// pass) means resolution sees one consistent view of the
/// environment: a variable cannot be checked, then changed, then
/// read as a different value. Consumers read from the returned map
/// (see the `VarLookup` closures in the providers, and
/// `crate::config`).
pub(crate) fn snapshot_env(names: &[&str]) -> Result<HashMap<String, String>, GeectlError> {
    let mut map = HashMap::new();
    for name in names {
        if let Some(value) = std::env::var_os(name) {
            let value = value.into_string().map_err(|_| {
                GeectlError::new(ErrorKind::ConfigError, format!("{name} is not valid UTF-8"))
            })?;
            map.insert(name.to_string(), value);
        }
    }
    Ok(map)
}

/// OBS Basic auth from the process environment. See
/// [`resolve_obs_basic`] for the variables and the rules.
pub struct EnvObsAuth;

impl AuthProvider for EnvObsAuth {
    fn credentials(&self) -> Result<Credentials, GeectlError> {
        let env = snapshot_env(&[OBS_USERNAME_VAR, OBS_PASSWORD_VAR, OBS_PASSWORD_FILE_VAR])?;
        resolve_obs_basic(&|name| env.get(name).cloned(), &read_secret_file_production)
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
    fn basic_empty_password_is_rejected() {
        // An empty password is never a real credential: fail fast here
        // instead of sending "user:" and getting a 401 at the first
        // request. (validate_basic previously accepted it.)
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
}
