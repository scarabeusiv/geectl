//! The HTTP client trait, with bounded timeouts.
//!
//! Every request geectl makes carries a bounded connect timeout, a
//! bounded read timeout, and a bounded total (global) deadline,
//! unconditional on library version. There is no unbounded request
//! anywhere. The timeout values are documented defaults, overridable
//! by explicit config, never by accident.
//!
//! The response body is capped (`MAX_BODY_BYTES`): a server that
//! answers with an unbounded body cannot grow geectl to OOM. A body
//! over the cap fails with `ServerError`.
//!
//! Retry policy (a single documented constant, not per-call-site
//! improvisation): only idempotent methods are retried (GET, HEAD, PUT,
//! DELETE). POST is never retried without an explicit idempotency key.
//! The scaffold applies the policy to GET only — mutations do not exist
//! yet, so there is nothing to retry — but the rule is already the law.
//! Within a retryable method, only transient failures are retried
//! (timeouts, transport errors, 429, 5xx). Client errors (other 4xx),
//! auth failures, and TLS failures are returned immediately.
//!
//! Non-2xx statuses are converted to a [`crate::error::GeectlError`] at
//! this layer, before any caller sees the body, so callers never have to
//! remember to check the status.

use crate::auth::{validate_basic, validate_bearer_token, AuthProvider, Credentials};
use crate::error::{ErrorKind, GeectlError};
use std::io::Read;
use std::time::Duration;

/// Default connect timeout: 10 seconds.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Default read timeout: 30 seconds.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Default total deadline for one request: 60 seconds. This bounds a
/// single attempt end to end (DNS, connect, TLS, headers, body), so a
/// slow drip that keeps resetting the per-read timeout still ends.
pub const DEFAULT_TOTAL_TIMEOUT: Duration = Duration::from_secs(60);
/// Maximum response body accepted: 10 MiB. A body over this fails with
/// `ServerError` instead of growing the process without bound.
pub const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

/// Bounded timeouts for a request. All three are always set; there is
/// no "no timeout" state.
#[derive(Debug, Clone, Copy)]
pub struct Timeout {
    pub connect: Duration,
    pub read: Duration,
    /// End-to-end deadline for one attempt (see `DEFAULT_TOTAL_TIMEOUT`).
    pub total: Duration,
}

impl Default for Timeout {
    fn default() -> Self {
        Timeout {
            connect: DEFAULT_CONNECT_TIMEOUT,
            read: DEFAULT_READ_TIMEOUT,
            total: DEFAULT_TOTAL_TIMEOUT,
        }
    }
}

impl Timeout {
    /// Create a timeout, enforcing the invariants `Default` upholds:
    /// every timeout is non-zero, and the total deadline exceeds a
    /// single read. A manually built `Timeout` with a zero, or with a
    /// total no larger than a read, would compile to an immediate
    /// deadline, so it fails here instead of at the first request.
    pub fn new(connect: Duration, read: Duration, total: Duration) -> Result<Self, GeectlError> {
        let timeout = Timeout {
            connect,
            read,
            total,
        };
        timeout.validate()?;
        Ok(timeout)
    }

    /// Enforce the timeout invariants: non-zero, and total exceeds read.
    pub fn validate(&self) -> Result<(), GeectlError> {
        if self.connect.is_zero() || self.read.is_zero() || self.total.is_zero() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "timeouts must be non-zero",
            ));
        }
        if self.total <= self.read {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "total timeout must exceed the read timeout",
            ));
        }
        Ok(())
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
    pub fn new(timeout: Timeout) -> Result<Self, GeectlError> {
        UreqClient::with_auth(timeout, None)
    }

    pub fn with_auth(
        timeout: Timeout,
        auth: Option<Box<dyn AuthProvider>>,
    ) -> Result<Self, GeectlError> {
        // A manually built Timeout bypasses `Timeout::new`, so validate
        // here too: an invalid timeout would otherwise compile to an
        // immediate deadline at the first request.
        timeout.validate()?;
        let config = ureq::Agent::config_builder()
            .timeout_connect(Some(timeout.connect))
            .timeout_per_read(Some(timeout.read))
            .timeout_global(Some(timeout.total))
            .build();
        let agent: ureq::Agent = config.into();
        Ok(UreqClient { agent, auth })
    }

    fn apply_auth<B>(
        &self,
        request: ureq::RequestBuilder<B>,
        credentials: &Credentials,
    ) -> Result<ureq::RequestBuilder<B>, GeectlError> {
        match credentials {
            Credentials::None => Ok(request),
            Credentials::Basic { username, password } => {
                let (username, password) = validate_basic(username, password)?;
                let token = format!("{username}:{password}");
                use base64::Engine;
                let encoded = base64::engine::general_purpose::STANDARD.encode(token.as_bytes());
                Ok(request.header("Authorization", format!("Basic {encoded}")))
            }
            Credentials::Bearer { token } => {
                let token = validate_bearer_token(token)?;
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
    let rest = &url[after_scheme..];
    // The authority ends at the first `/`, `?`, or `#`. Userinfo only
    // exists inside the authority; an `@` in the path, query, or
    // fragment is data, not a credential separator, and must be left
    // alone (stripping at it would misname the resource).
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    // Userinfo ends at the LAST `@` in the authority (RFC 3986): a
    // password may itself contain `@` (e.g. user:p@ss@host), so
    // stripping at the first `@` would leave part of the password in
    // the sanitized URL.
    let Some(at) = rest[..authority_end].rfind('@') else {
        return url.to_string();
    };
    let mut out = String::with_capacity(url.len());
    out.push_str(&url[..after_scheme]);
    out.push_str(&rest[at + 1..]);
    out
}

/// Read a response body with a cap. Reads at most `MAX_BODY_BYTES + 1`
/// bytes; a body longer than the cap fails with `ServerError` instead
/// of growing the process without bound. The `+ 1` distinguishes "at
/// the cap" (accepted) from "over the cap" (rejected). Bytes are read
/// first and converted to UTF-8 after the cap check: converting while
/// reading would split a multi-byte character at the cap (and report
/// the over-cap body as a network error), and an in-cap body that is
/// not valid UTF-8 is a server error, not a transport failure.
fn read_body_capped(reader: impl Read) -> Result<String, GeectlError> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_BODY_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| GeectlError::new(ErrorKind::NetworkError, e.to_string()))?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err(GeectlError::new(
            ErrorKind::ServerError,
            format!("response body exceeds the {MAX_BODY_BYTES}-byte limit"),
        ));
    }
    String::from_utf8(bytes).map_err(|e| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("response body is not valid UTF-8: {e}"),
        )
    })
}

/// What to do with a non-2xx status. `Return` ends the call with this
/// error; `Retry` records it and retries (transient only).
enum StatusAction {
    Return(GeectlError),
    Retry(GeectlError),
}

/// Map a non-2xx status to an error and a retry decision. Only
/// transient failures retry (408, 429, 5xx). Anything the caller can fix
/// (other 4xx) is returned immediately as `InvalidInput` with the
/// right exit code, instead of collapsing into `ServerError`. 407 is
/// an auth failure and 410 is a not-found, like 401/403 and 404.
fn status_action(code: u16, safe_url: &str) -> StatusAction {
    match code {
        404 => StatusAction::Return(GeectlError::new(
            ErrorKind::NotFound,
            format!("{safe_url} returned 404"),
        )),
        401 | 403 | 407 => StatusAction::Return(GeectlError::new(
            ErrorKind::AuthFailed,
            format!("{safe_url} returned {code}"),
        )),
        // 408 is the server timing out on the request: retry it as a
        // timeout, not as a caller error.
        408 => StatusAction::Retry(GeectlError::new(
            ErrorKind::Timeout,
            format!("{safe_url} returned 408"),
        )),
        // 410 means the resource is gone for good: the same terminal
        // answer as 404, not a caller error to fix and retry.
        410 => StatusAction::Return(GeectlError::new(
            ErrorKind::NotFound,
            format!("{safe_url} returned 410"),
        )),
        // 429 and 5xx are transient: the server is telling us to wait,
        // or is itself at fault. Retry; if retries run out the caller
        // sees ServerError.
        429 | 500..=599 => StatusAction::Retry(GeectlError::new(
            ErrorKind::ServerError,
            format!("{safe_url} returned {code}"),
        )),
        // Any other 4xx is the caller's fault: wrong name, wrong flag,
        // wrong method. Retrying would not help.
        400..=499 => StatusAction::Return(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("{safe_url} returned {code}"),
        )),
        _ => StatusAction::Return(GeectlError::new(
            ErrorKind::ServerError,
            format!("{safe_url} returned {code}"),
        )),
    }
}

impl HttpClient for UreqClient {
    fn get(&self, url: &str) -> Result<String, GeectlError> {
        // Resolve credentials once per call, not once per retry
        // attempt: re-resolving per attempt would re-read the secret
        // source on every retry for no reason.
        let credentials = match &self.auth {
            Some(auth) => auth.credentials()?,
            None => Credentials::None,
        };
        let mut last_error: Option<GeectlError> = None;
        for attempt in 0..=RETRY_POLICY.max_retries {
            let request = self.apply_auth(self.agent.get(url), &credentials)?;
            match request.call() {
                Ok(resp) => {
                    return read_body_capped(resp.into_body().into_reader());
                }
                Err(ureq::Error::StatusCode(code)) => {
                    let safe_url = sanitize_url_for_error(url);
                    match status_action(code, &safe_url) {
                        StatusAction::Return(err) => return Err(err),
                        StatusAction::Retry(err) => {
                            last_error = Some(err);
                        }
                    }
                }
                Err(ureq::Error::Tls(msg)) => {
                    // TLS failures are never retried: retrying a failed
                    // handshake or an untrusted certificate cannot
                    // succeed, and it would hammer a server that
                    // already refused us.
                    return Err(GeectlError::new(
                        ErrorKind::TlsError,
                        format!("TLS error: {msg}"),
                    ));
                }
                Err(ureq::Error::Rustls(e)) => {
                    return Err(GeectlError::new(
                        ErrorKind::TlsError,
                        format!("TLS error: {e}"),
                    ));
                }
                Err(ureq::Error::Pem(e)) => {
                    return Err(GeectlError::new(
                        ErrorKind::TlsError,
                        format!("TLS error: {e}"),
                    ));
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
            // Only idempotent methods retry. This function only ever
            // issues GET (which is retryable), so the gate is a
            // debug_assert: it documents the policy and fails a debug
            // build if the method ever changes, without dead code in
            // release. A future method must gate on its real method.
            debug_assert!(
                is_retryable_method("GET"),
                "GET must stay retryable; a new method needs its own gate"
            );
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
        assert!(t.total.as_secs() > 0);
        // The total deadline must exceed a single read: a slow drip
        // that keeps resetting the per-read timeout still has to end,
        // but a healthy read must never hit the global deadline first.
        assert!(t.total > t.read);
    }

    #[test]
    fn timeout_new_rejects_invalid() {
        assert!(Timeout::new(
            Duration::ZERO,
            Duration::from_secs(30),
            Duration::from_secs(60)
        )
        .is_err());
        assert!(Timeout::new(
            Duration::from_secs(10),
            Duration::ZERO,
            Duration::from_secs(60)
        )
        .is_err());
        assert!(Timeout::new(
            Duration::from_secs(10),
            Duration::from_secs(30),
            Duration::ZERO
        )
        .is_err());
        // Total must exceed a single read.
        assert!(Timeout::new(
            Duration::from_secs(10),
            Duration::from_secs(30),
            Duration::from_secs(30)
        )
        .is_err());
        assert!(Timeout::new(
            Duration::from_secs(10),
            Duration::from_secs(30),
            Duration::from_secs(60)
        )
        .is_ok());
        assert!(Timeout::default().validate().is_ok());
    }

    #[test]
    fn client_rejects_invalid_timeout() {
        let bad = Timeout {
            connect: Duration::ZERO,
            read: Duration::from_secs(30),
            total: Duration::from_secs(60),
        };
        assert!(UreqClient::new(bad).is_err());
        assert!(UreqClient::with_auth(Timeout::default(), None).is_ok());
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

    #[test]
    fn at_sign_in_password_is_stripped_with_last_at() {
        // RFC 3986: userinfo ends at the LAST `@` in the authority — a
        // password may itself contain `@`. Stripping at the first `@`
        // would leave part of the password in the sanitized URL.
        assert_eq!(
            sanitize_url_for_error("https://user:p@ss@example.com/path"),
            "https://example.com/path"
        );
    }

    #[test]
    fn at_sign_outside_the_authority_is_left_alone() {
        // An `@` in the path, query, or fragment is data, not a
        // credential separator. Stripping at it would misname the
        // resource (and hide the real path from the error).
        assert_eq!(
            sanitize_url_for_error("https://example.com/path@x"),
            "https://example.com/path@x"
        );
        assert_eq!(
            sanitize_url_for_error("https://example.com/search?q=a@b"),
            "https://example.com/search?q=a@b"
        );
        assert_eq!(
            sanitize_url_for_error("https://example.com/path#frag@x"),
            "https://example.com/path#frag@x"
        );
        // Only the authority userinfo is stripped; the query `@` stays.
        assert_eq!(
            sanitize_url_for_error("https://user:secret@example.com/path?x=@y"),
            "https://example.com/path?x=@y"
        );
    }

    #[test]
    fn body_at_the_cap_is_accepted() {
        let data = vec![b'a'; MAX_BODY_BYTES];
        let body = read_body_capped(std::io::Cursor::new(data)).unwrap();
        assert_eq!(body.len(), MAX_BODY_BYTES);
    }

    #[test]
    fn body_over_the_cap_is_rejected() {
        let data = vec![b'a'; MAX_BODY_BYTES + 1];
        let err = read_body_capped(std::io::Cursor::new(data)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn body_over_cap_with_split_utf8_is_server_error() {
        // A multi-byte character split by the cap must not be reported
        // as a network error: the body is over the cap, so it is a
        // server error. (Reading into a String first produced an
        // invalid-UTF-8 I/O error mapped to NetworkError.)
        let mut data = vec![b'a'; MAX_BODY_BYTES];
        data.push(0xC3); // first byte of a 2-byte UTF-8 sequence
        let err = read_body_capped(std::io::Cursor::new(data)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn in_cap_invalid_utf8_is_server_error() {
        // A body within the cap that is not valid UTF-8 is a server
        // error, not a transport failure.
        let data = vec![0xFF; 4];
        let err = read_body_capped(std::io::Cursor::new(data)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn valid_multibyte_body_is_accepted() {
        let data = "h\u{e9}llo w\u{f6}rld".repeat(10);
        let body = read_body_capped(std::io::Cursor::new(data.clone())).unwrap();
        assert_eq!(body, data);
    }

    #[test]
    fn status_codes_map_to_the_right_action() {
        // Not-found and auth failures return immediately.
        assert!(
            matches!(status_action(404, "u"), StatusAction::Return(e) if e.kind() == ErrorKind::NotFound)
        );
        assert!(
            matches!(status_action(401, "u"), StatusAction::Return(e) if e.kind() == ErrorKind::AuthFailed)
        );
        assert!(
            matches!(status_action(403, "u"), StatusAction::Return(e) if e.kind() == ErrorKind::AuthFailed)
        );
        // 407 is an auth failure; 410 is gone (not-found); 408 retries.
        assert!(
            matches!(status_action(407, "u"), StatusAction::Return(e) if e.kind() == ErrorKind::AuthFailed)
        );
        assert!(
            matches!(status_action(410, "u"), StatusAction::Return(e) if e.kind() == ErrorKind::NotFound)
        );
        assert!(
            matches!(status_action(408, "u"), StatusAction::Retry(e) if e.kind() == ErrorKind::Timeout)
        );
        // Other 4xx are the caller's fault: InvalidInput, no retry.
        assert!(
            matches!(status_action(400, "u"), StatusAction::Return(e) if e.kind() == ErrorKind::InvalidInput)
        );
        assert!(
            matches!(status_action(405, "u"), StatusAction::Return(e) if e.kind() == ErrorKind::InvalidInput)
        );
        assert!(
            matches!(status_action(422, "u"), StatusAction::Return(e) if e.kind() == ErrorKind::InvalidInput)
        );
        // 429 and 5xx are transient: retry as ServerError.
        assert!(
            matches!(status_action(429, "u"), StatusAction::Retry(e) if e.kind() == ErrorKind::ServerError)
        );
        assert!(
            matches!(status_action(500, "u"), StatusAction::Retry(e) if e.kind() == ErrorKind::ServerError)
        );
        assert!(
            matches!(status_action(503, "u"), StatusAction::Retry(e) if e.kind() == ErrorKind::ServerError)
        );
    }
}
