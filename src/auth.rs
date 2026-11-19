//! The auth seam.
//!
//! This module is a trait only — the seam the credential design plugs
//! into after scaffolding. It deliberately stores no credentials and
//! reads none: providers supply credentials from environment variables or
//! mounted secret files, and the HTTP client applies them. There is no
//! plaintext fallback and no obfuscation option, and a credential-shaped
//! value in a config file is a refusal to run (see the design doc).
//!
//! Credential types the design will cover (after scaffolding):
//! - OBS: username/password (Basic auth) — OBS has no API key.
//! - Gitea: API token (Bearer), plus SSH keys for git operations.
//!
//! [`NoAuth`] is the only provider in the scaffold. It exists so the HTTP
//! client compiles and runs without credentials; it is not a credential
//! store.

use crate::error::GeectlError;
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
}
