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

/// The credentials a provider supplies, as data. The HTTP client applies
/// them to a request; the provider never touches a request itself.
#[derive(Debug, Clone)]
pub enum Credentials {
    /// No credentials. Public reads only.
    None,
    /// Username and password, sent as HTTP Basic auth.
    Basic { username: String, password: String },
    /// An API token, sent as a Bearer token.
    Bearer { token: String },
    /// An SSH key, for git operations only. It cannot authenticate an
    /// HTTP request — the HTTP client refuses it (see `http.rs`).
    SshKey { path: String },
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
}
