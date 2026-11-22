use super::validate_base;
use super::OBS_API_BASE;
use crate::auth::Credentials;
use crate::debug::Debug;
use crate::error::ErrorKind;
use crate::error::GeectlError;
use crate::http::HttpClient;

/// The OBS client: the devel-project lookup, the devel version, the
/// open-submit-request check, submit-request filing, branch setup,
/// and the remote reads (file and directory reads, history, meta,
/// search, owners, person, link target, project diff).
///
/// The client holds the HTTP client and the OBS Basic credentials
/// (resolved by the caller through geectl's auth surface, e.g.
/// `EnvObsAuth`). The public reads need no credentials; the
/// credentialed reads and the filing POST need them and fail loudly
/// without them.
pub struct ObsClient<'a> {
    pub(crate) http: &'a dyn HttpClient,
    pub(crate) base: String,
    pub(crate) credentials: Credentials,
    pub(crate) debug: Debug,
}

impl<'a> ObsClient<'a> {
    /// A client for the production OBS API, with no credentials
    /// (public reads only).
    pub fn new(http: &'a dyn HttpClient) -> Self {
        ObsClient {
            http,
            base: OBS_API_BASE.to_string(),
            credentials: Credentials::None,
            debug: Debug::disabled(),
        }
    }

    /// A client for a custom OBS API base (a test instance), with no
    /// credentials. The base must be `https://` with a host, or
    /// `http://` on a loopback host.
    pub fn with_base(http: &'a dyn HttpClient, base: &str) -> Result<Self, GeectlError> {
        validate_base(base)?;
        Ok(ObsClient {
            http,
            base: base.trim_end_matches('/').to_string(),
            credentials: Credentials::None,
            debug: Debug::disabled(),
        })
    }

    /// A client with OBS Basic credentials (for the authenticated
    /// endpoints: the open-SR search and filing a submit request).
    /// Accepts `Credentials::Basic` and `Credentials::None`; a Bearer
    /// token or an SSH key cannot authenticate OBS and is refused.
    pub fn with_credentials(
        http: &'a dyn HttpClient,
        credentials: Credentials,
    ) -> Result<Self, GeectlError> {
        match &credentials {
            Credentials::None | Credentials::Basic { .. } => Ok(ObsClient {
                http,
                base: OBS_API_BASE.to_string(),
                credentials,
                debug: Debug::disabled(),
            }),
            Credentials::Bearer { .. } | Credentials::SshKey { .. } => Err(GeectlError::new(
                ErrorKind::ConfigError,
                "OBS accepts Basic credentials only; a Bearer token or SSH key cannot authenticate it",
            )),
        }
    }

    /// A client for a custom OBS API base with OBS Basic credentials.
    pub fn with_base_and_credentials(
        http: &'a dyn HttpClient,
        base: &str,
        credentials: Credentials,
    ) -> Result<Self, GeectlError> {
        validate_base(base)?;
        match &credentials {
            Credentials::None | Credentials::Basic { .. } => Ok(ObsClient {
                http,
                base: base.trim_end_matches('/').to_string(),
                credentials,
                debug: Debug::disabled(),
            }),
            Credentials::Bearer { .. } | Credentials::SshKey { .. } => Err(GeectlError::new(
                ErrorKind::ConfigError,
                "OBS accepts Basic credentials only; a Bearer token or SSH key cannot authenticate it",
            )),
        }
    }

    /// Report the channel's decision and retry lines to
    /// `debug` instead of dropping them. The sink is off by
    /// default; the caller that owns the debug channel passes its
    /// sink here. (The HTTP attempt lines come from the HTTP client
    /// itself, which the caller configures separately.)
    pub fn with_debug(mut self, debug: Debug) -> Self {
        self.debug = debug;
        self
    }

    /// The credentials, if any.
    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    /// True when OBS Basic credentials are configured.
    pub(crate) fn has_basic(&self) -> bool {
        matches!(self.credentials, Credentials::Basic { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Credentials;
    use crate::obs::common::is_bare_authority;
    use crate::obs::test_support::{basic_credentials, FakeHttp};

    #[test]
    fn with_credentials_rejects_bearer_and_ssh_key() {
        // OBS accepts Basic only: a Bearer token or SSH key cannot
        // authenticate it, so it is refused at construction.
        let http = FakeHttp::new();
        assert!(ObsClient::with_credentials(
            &http,
            Credentials::Bearer {
                token: "tok3n".to_string()
            }
        )
        .is_err());
        assert!(ObsClient::with_credentials(
            &http,
            Credentials::SshKey {
                path: std::path::PathBuf::from("/home/user/.ssh/id_ed25519"),
            }
        )
        .is_err());
        // Basic and None are accepted.
        assert!(ObsClient::with_credentials(&http, basic_credentials()).is_ok());
        assert!(ObsClient::with_credentials(&http, Credentials::None).is_ok());
    }

    #[test]
    fn custom_base_must_be_https_or_loopback_http() {
        let http = FakeHttp::new();
        assert!(ObsClient::with_base(&http, "https://api.opensuse.org").is_ok());
        assert!(ObsClient::with_base(&http, "http://127.0.0.1:3000").is_ok());
        assert!(ObsClient::with_base(&http, "http://localhost:3000").is_ok());
        assert!(ObsClient::with_base(&http, "http://example.com").is_err());
        assert!(ObsClient::with_base(&http, "http://127.0.0.1.evil.com").is_err());
        // A bracketed-host spoof is not loopback.
        assert!(ObsClient::with_base(&http, "http://[::1]evil.com").is_err());
        assert!(ObsClient::with_base(&http, "api.opensuse.org").is_err());
    }

    #[test]
    fn custom_base_must_be_bare() {
        let http = FakeHttp::new();
        // A path, query, fragment, or userinfo after the authority
        // would be concatenated into every OBS URL.
        for base in [
            "https://api.opensuse.org/evil",
            "https://api.opensuse.org?x=1",
            "https://api.opensuse.org#frag",
            "https://user@api.opensuse.org",
            "https://user:pass@api.opensuse.org",
            "https://api.opensuse.org ",
            "https://:443",
            "http://127.0.0.1:3000/evil",
            "http://user@127.0.0.1:3000",
        ] {
            assert!(
                ObsClient::with_base(&http, base).is_err(),
                "{base} must be refused"
            );
        }
        // A trailing slash is stripped, not a path: the client built
        // from it points at the bare base.
        let client = ObsClient::with_base(&http, "https://api.opensuse.org/").unwrap();
        let url = client.search_url("foo").unwrap();
        assert!(url.starts_with("https://api.opensuse.org/search/request?match="));
        assert!(!url.contains("/evil"));
    }

    #[test]
    fn bare_authority_rejects_whitespace_and_empty_host() {
        assert!(is_bare_authority("api.opensuse.org"));
        assert!(is_bare_authority("127.0.0.1:3000"));
        assert!(is_bare_authority("[::1]:3000"));
        assert!(is_bare_authority("[::1]"));
        assert!(is_bare_authority("[::1]:8080"));
        // Whitespace anywhere in the authority is not a host.
        assert!(!is_bare_authority("api.opensuse.org "));
        assert!(!is_bare_authority("api .opensuse.org"));
        assert!(!is_bare_authority("api\t.opensuse.org"));
        // An empty host: a bare port, or empty brackets.
        assert!(!is_bare_authority(":3000"));
        assert!(!is_bare_authority("[]"));
        assert!(!is_bare_authority("[]:3000"));
        // An unterminated or spoofed bracketed host is not a host.
        assert!(!is_bare_authority("["));
        assert!(!is_bare_authority("[::1"));
        assert!(!is_bare_authority("[::1]evil.com"));
        assert!(!is_bare_authority(""));
        // A port, when present, is a decimal port number.
        assert!(is_bare_authority("api.opensuse.org:443"));
        assert!(is_bare_authority("api.opensuse.org:65535"));
        // An empty port means the default port: accepted, as before.
        assert!(is_bare_authority("api.opensuse.org:"));
        assert!(!is_bare_authority("api.opensuse.org:abc"));
        assert!(!is_bare_authority("api.opensuse.org:80x"));
        assert!(!is_bare_authority("api.opensuse.org:99999"));
        assert!(!is_bare_authority("[::1]:abc"));
    }
}
