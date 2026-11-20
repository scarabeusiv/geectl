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
use std::fmt;
use std::path::PathBuf;

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

/// Validate Basic credentials. Trailing CR/LF is trimmed from both
/// values (credentials come from environment variables or mounted secret
/// files, where a trailing newline is a paste artifact, not part of the
/// secret); any control character anywhere else is rejected. Only CR/LF
/// is stripped: a trailing space is legitimate credential content. The
/// username must not contain `:` — RFC 7617 reserves the colon as the
/// separator between user-id and password, so one in the username would
/// corrupt the credential.
pub fn validate_basic(username: &str, password: &str) -> Result<(String, String), GeectlError> {
    let username = username.trim_end_matches(['\r', '\n']);
    let password = password.trim_end_matches(['\r', '\n']);
    if username.trim().is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "basic auth username is empty or whitespace-only",
        ));
    }
    if has_control_chars(username) || has_control_chars(password) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "basic auth credential contains control characters",
        ));
    }
    if username.contains(':') {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "basic auth username must not contain ':'",
        ));
    }
    Ok((username.to_string(), password.to_string()))
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
        let rendered = format!("{basic:?}");
        assert!(rendered.contains("user"));
        assert!(!rendered.contains("s3cr3t"));
        assert!(rendered.contains("[redacted]"));

        let bearer = Credentials::Bearer {
            token: "tok3n".to_string(),
        };
        let rendered = format!("{bearer:?}");
        assert!(!rendered.contains("tok3n"));
        assert!(rendered.contains("[redacted]"));

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
}
