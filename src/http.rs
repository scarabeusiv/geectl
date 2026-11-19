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

use crate::auth::{AuthProvider, Credentials};
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
            .timeout_global(Some(timeout.total))
            .build();
        let agent: ureq::Agent = config.into();
        UreqClient { agent, auth }
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

/// True when the value carries a control character (or DEL). One in a
/// credential would corrupt the Authorization header or let a second
/// header be smuggled in, so any credential carrying one is rejected.
fn has_control_chars(value: &str) -> bool {
    value.chars().any(|c| c.is_control())
}

/// Validate a Bearer token. Trailing whitespace/control is trimmed (a
/// pasted token with a trailing newline must not corrupt auth); any
/// control character anywhere else is rejected. The token itself is
/// never included in the error — it is a secret.
fn validate_bearer_token(token: &str) -> Result<String, GeectlError> {
    let token = token.trim_end();
    if token.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "bearer token is empty",
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

/// Validate Basic credentials. Trailing whitespace/control is trimmed
/// from both values (credentials come from environment variables or
/// mounted secret files, where a trailing newline is a paste artifact,
/// not part of the secret); any control character anywhere else is
/// rejected. The username must not contain `:` — RFC 7617 reserves the
/// colon as the separator between user-id and password, so one in the
/// username would corrupt the credential.
fn validate_basic(username: &str, password: &str) -> Result<(String, String), GeectlError> {
    let username = username.trim_end();
    let password = password.trim_end();
    if username.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "basic auth username is empty",
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
    let Some(at) = rest[..authority_end].find('@') else {
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
/// the cap" (accepted) from "over the cap" (rejected).
fn read_body_capped(reader: impl Read) -> Result<String, GeectlError> {
    let mut body = String::new();
    reader
        .take(MAX_BODY_BYTES as u64 + 1)
        .read_to_string(&mut body)
        .map_err(|e| GeectlError::new(ErrorKind::NetworkError, e.to_string()))?;
    if body.len() > MAX_BODY_BYTES {
        return Err(GeectlError::new(
            ErrorKind::ServerError,
            format!("response body exceeds the {MAX_BODY_BYTES}-byte limit"),
        ));
    }
    Ok(body)
}

/// What to do with a non-2xx status. `Return` ends the call with this
/// error; `Retry` records it and retries (transient only).
enum StatusAction {
    Return(GeectlError),
    Retry(GeectlError),
}

/// Map a non-2xx status to an error and a retry decision. Only
/// transient failures retry (429, 5xx). Anything the caller can fix
/// (other 4xx) is returned immediately as `InvalidInput` with the
/// right exit code, instead of collapsing into `ServerError`.
fn status_action(code: u16, safe_url: &str) -> StatusAction {
    match code {
        404 => StatusAction::Return(GeectlError::new(
            ErrorKind::NotFound,
            format!("{safe_url} returned 404"),
        )),
        401 | 403 => StatusAction::Return(GeectlError::new(
            ErrorKind::AuthFailed,
            format!("{safe_url} returned {code}"),
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
            // Only idempotent methods retry (GET is one). The gate is
            // here so the policy is enforced in one place, not assumed
            // by the caller.
            if !is_retryable_method("GET") {
                break;
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
        assert!(t.total.as_secs() > 0);
        // The total deadline must exceed a single read: a slow drip
        // that keeps resetting the per-read timeout still has to end,
        // but a healthy read must never hit the global deadline first.
        assert!(t.total > t.read);
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
    fn bearer_token_trailing_newline_is_trimmed() {
        assert_eq!(validate_bearer_token("abc123\n").unwrap(), "abc123");
        assert_eq!(validate_bearer_token("abc123  ").unwrap(), "abc123");
    }

    #[test]
    fn bearer_token_with_control_char_is_rejected() {
        // A control character in the middle must be rejected, not
        // trimmed: it would corrupt the Authorization header.
        assert!(validate_bearer_token("abc\rdef").is_err());
        assert!(validate_bearer_token("abc\ndef").is_err());
        assert!(validate_bearer_token("").is_err());
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
        assert!(validate_basic("user", "pa\0ss").is_err());
        assert!(validate_basic("", "pass").is_err());
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
