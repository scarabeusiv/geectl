//! The HTTP client trait, with a bounded timeout.
//!
//! Every request geectl makes carries a bounded connect timeout and a
//! bounded read timeout, unconditional on library version. There is no
//! unbounded request anywhere. The timeout values are documented defaults,
//! overridable by explicit config, never by accident.
//!
//! Retry policy (a single documented constant, not per-call-site
//! improvisation): only idempotent methods are retried (GET, HEAD, PUT,
//! DELETE). POST is never retried without an explicit idempotency key.
//! The scaffold applies the policy to GET only — mutations do not exist
//! yet, so there is nothing to retry — but the rule is already the law.
//!
//! Non-2xx statuses are converted to a [`crate::error::GeectlError`] at
//! this layer, before any caller sees the body, so callers never have to
//! remember to check the status.

use crate::auth::{AuthProvider, Credentials};
use crate::error::{ErrorKind, GeectlError};
use std::time::Duration;

/// Default connect timeout: 10 seconds.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Default read timeout: 30 seconds.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Bounded timeouts for a request. Both are always set; there is no
/// "no timeout" state.
#[derive(Debug, Clone, Copy)]
pub struct Timeout {
    pub connect: Duration,
    pub read: Duration,
}

impl Default for Timeout {
    fn default() -> Self {
        Timeout {
            connect: DEFAULT_CONNECT_TIMEOUT,
            read: DEFAULT_READ_TIMEOUT,
        }
    }
}

/// The retry policy, as a single documented constant. `max_retries` is the
/// number of retries after the first attempt; `backoff_base` is the delay
/// before the first retry, doubled on each subsequent retry.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub backoff_base: Duration,
}

/// Retry only idempotent methods, up to 3 times, starting at 1 second and
/// doubling. POST is never in this set.
pub const RETRY_POLICY: RetryPolicy = RetryPolicy {
    max_retries: 3,
    backoff_base: Duration::from_secs(1),
};

/// True when the method may be retried. POST (and PATCH) are never
/// retried: a retried mutation can create a duplicate.
pub fn is_retryable_method(method: &str) -> bool {
    matches!(method, "GET" | "HEAD" | "PUT" | "DELETE")
}

/// The HTTP client interface. Implementations are synchronous and return
/// the response body as a string on a 2xx status, or a [`GeectlError`].
pub trait HttpClient {
    fn get(&self, url: &str) -> Result<String, GeectlError>;
}

/// A [`HttpClient`] backed by ureq, with bounded timeouts and the retry
/// policy applied to GET. Auth, when provided, is applied from the
/// [`AuthProvider`] seam — the client never reads credentials itself.
pub struct UreqClient {
    agent: ureq::Agent,
    auth: Option<Box<dyn AuthProvider>>,
}

impl UreqClient {
    pub fn new(timeout: Timeout) -> Self {
        UreqClient::with_auth(timeout, None)
    }

    pub fn with_auth(timeout: Timeout, auth: Option<Box<dyn AuthProvider>>) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_connect(Some(timeout.connect))
            .timeout_per_read(Some(timeout.read))
            .build();
        let agent: ureq::Agent = config.into();
        UreqClient { agent, auth }
    }

    fn apply_auth<B>(
        &self,
        request: ureq::RequestBuilder<B>,
    ) -> Result<ureq::RequestBuilder<B>, GeectlError> {
        let Some(auth) = &self.auth else {
            return Ok(request);
        };
        match auth.credentials()? {
            Credentials::None => Ok(request),
            Credentials::Basic { username, password } => {
                let token = format!("{username}:{password}");
                use base64::Engine;
                let encoded = base64::engine::general_purpose::STANDARD.encode(token.as_bytes());
                Ok(request.header("Authorization", format!("Basic {encoded}")))
            }
            Credentials::Bearer { token } => {
                Ok(request.header("Authorization", format!("Bearer {token}")))
            }
            Credentials::SshKey { .. } => Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "an SSH key cannot authenticate an HTTP request",
            )),
        }
    }
}

/// Strip any userinfo (`user:pass@`) from a URL before it is used in an
/// error context. A URL with embedded credentials must never leak into
/// an error message, a log, or stdout — the context names the resource,
/// not the credential.
fn sanitize_url_for_error(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let after_scheme = scheme_end + 3;
    let Some(at) = url[after_scheme..].find('@') else {
        return url.to_string();
    };
    let mut out = String::with_capacity(url.len());
    out.push_str(&url[..after_scheme]);
    out.push_str(&url[after_scheme + at + 1..]);
    out
}

impl HttpClient for UreqClient {
    fn get(&self, url: &str) -> Result<String, GeectlError> {
        let mut last_error: Option<GeectlError> = None;
        for attempt in 0..=RETRY_POLICY.max_retries {
            let request = self.apply_auth(self.agent.get(url))?;
            match request.call() {
                Ok(resp) => {
                    return resp
                        .into_body()
                        .read_to_string()
                        .map_err(|e| GeectlError::new(ErrorKind::NetworkError, e.to_string()));
                }
                Err(ureq::Error::StatusCode(code)) => {
                    let safe_url = sanitize_url_for_error(url);
                    return Err(match code {
                        404 => GeectlError::new(
                            ErrorKind::NotFound,
                            format!("{safe_url} returned 404"),
                        ),
                        401 | 403 => GeectlError::new(
                            ErrorKind::AuthFailed,
                            format!("{safe_url} returned {code}"),
                        ),
                        _ => GeectlError::new(
                            ErrorKind::ServerError,
                            format!("{safe_url} returned {code}"),
                        ),
                    });
                }
                Err(ureq::Error::Timeout(_)) => {
                    last_error = Some(GeectlError::new(
                        ErrorKind::Timeout,
                        format!("request to {} timed out", sanitize_url_for_error(url)),
                    ));
                }
                Err(ureq::Error::Io(e)) => {
                    last_error = Some(GeectlError::new(ErrorKind::NetworkError, e.to_string()));
                }
                Err(e) => {
                    last_error = Some(GeectlError::new(ErrorKind::ServerError, e.to_string()));
                }
            }
            if attempt < RETRY_POLICY.max_retries {
                std::thread::sleep(RETRY_POLICY.backoff_base * 2u32.pow(attempt));
            }
        }
        Err(last_error.unwrap_or_else(|| {
            GeectlError::new(ErrorKind::InternalError, "GET failed without an error")
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_methods_are_idempotent_only() {
        assert!(is_retryable_method("GET"));
        assert!(is_retryable_method("HEAD"));
        assert!(is_retryable_method("PUT"));
        assert!(is_retryable_method("DELETE"));
        assert!(!is_retryable_method("POST"));
        assert!(!is_retryable_method("PATCH"));
    }

    #[test]
    fn default_timeout_is_bounded() {
        let t = Timeout::default();
        assert!(t.connect.as_secs() > 0);
        assert!(t.read.as_secs() > 0);
    }

    #[test]
    fn userinfo_is_stripped_from_error_urls() {
        // A URL with embedded credentials must never reach an error message.
        assert_eq!(
            sanitize_url_for_error("https://user:secret@example.com/path"),
            "https://example.com/path"
        );
        assert_eq!(
            sanitize_url_for_error("https://example.com/path"),
            "https://example.com/path"
        );
        assert_eq!(
            sanitize_url_for_error("https://user@example.com/"),
            "https://example.com/"
        );
    }
}
