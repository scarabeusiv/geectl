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
//! The policy applies to GET; `post_json` never retries — a retried
//! mutation can create a duplicate.
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

    /// POST a JSON body. The body is sent as-is with
    /// `Content-Type: application/json`; the caller builds the JSON.
    /// Unlike GET, POST is never retried: a retried mutation can create
    /// a duplicate, so every non-2xx status (and every transport
    /// failure) is returned immediately. Auth and the transport guard
    /// work exactly as for GET.
    fn post_json(&self, url: &str, body: &str) -> Result<String, GeectlError>;
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
            // Never follow a redirect automatically: a redirect
            // would send the request (path, query) to a host geectl
            // never validated. A 3xx is an error instead (see
            // redirect_error); the caller retries or reports it.
            // With max_redirects(0) ureq returns the 3xx as
            // Ok(response), so get()/post_json() must refuse it
            // explicitly — reading the body would return the
            // redirect body as a success.
            .max_redirects(0)
            // Never forward the Authorization header on a redirect.
            // The ureq default is already `Never`, but it is set
            // explicitly so a future ureq default change cannot
            // silently start leaking credentials to the redirect
            // target (which may be a different host entirely).
            .redirect_auth_headers(ureq::config::RedirectAuthHeaders::Never)
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
///
/// Public so the live-test harness strips userinfo the same way before
/// printing a URL.
pub fn sanitize_url_for_error(url: &str) -> String {
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

/// Refuse a redirect response: automatic redirects are disabled, so
/// a 3xx is an error, not a body. The Location is named (sanitized —
/// it can carry userinfo) so the caller can see where the server
/// pointed. Plain arguments: ureq's Response type is private, so the
/// call sites pass the status and header value.
fn redirect_error(status: u16, location: Option<&str>) -> GeectlError {
    let location = location
        .map(sanitize_url_for_error)
        .unwrap_or_else(|| "(no Location header)".to_string());
    GeectlError::new(
        ErrorKind::ServerError,
        format!("server returned {status} and redirected to {location}; automatic redirects are disabled"),
    )
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

/// True when the URL's authority carries userinfo (`user` or
/// `user:pass` before the last `@`). Only the authority is inspected:
/// an `@` in the path or query is not userinfo. The last `@`
/// separates userinfo from the host, so an `@` inside a password
/// does not end the userinfo.
fn url_has_userinfo(url: &str) -> bool {
    let Some(after_scheme) = url.split("://").nth(1) else {
        return false;
    };
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    after_scheme[..authority_end].contains('@')
}

/// Refuse to send credentials over cleartext http, except to a loopback
/// host (127.0.0.0/8, ::1, localhost) where the traffic never leaves
/// the host. Real servers are addressed through `UrlBuilder`, which
/// requires `https://`; this is the second guard, for callers that pass
/// a raw URL. The loopback exception exists so tests can exercise auth
/// against a local server with fixture credentials — never a real
/// endpoint, never a real credential. A userinfo URL carries a
/// credential in the URL itself, so the transport check applies to it
/// even when no `Credentials` are configured.
fn require_credentials_transport(url: &str, credentials: &Credentials) -> Result<(), GeectlError> {
    if url_has_userinfo(url) && !url.starts_with("https://") && !is_loopback_url(url) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "a URL with userinfo requires an https:// URL (http:// is allowed only for loopback hosts)",
        ));
    }
    if matches!(credentials, Credentials::None) {
        return Ok(());
    }
    if url.starts_with("https://") {
        return Ok(());
    }
    if is_loopback_url(url) {
        return Ok(());
    }
    Err(GeectlError::new(
        ErrorKind::InvalidInput,
        "credentials require an https:// URL (http:// is allowed only for loopback hosts)",
    ))
}

/// True when the URL's host is loopback (localhost, 127.0.0.0/8, ::1).
/// Only the authority is inspected; a loopback string in the path or
/// query does not count, and a DNS name that merely starts with
/// "127." (e.g. `127.0.0.1.evil.com`) is not loopback — the host must
/// parse as an IP in the loopback range, or be localhost.
///
/// Public so the live-test harness (`tests/gitea.rs`) shares the one
/// check the transport guard uses, instead of a copy that can drift.
pub fn is_loopback_url(url: &str) -> bool {
    let Some(after_scheme) = url.split("://").nth(1) else {
        return false;
    };
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    // Strip any userinfo first: an `@` in it must not be mistaken for
    // the host separator.
    let without_userinfo = authority.rsplit('@').next().unwrap_or(authority);
    // A bracketed host is only a bracketed host if the closing bracket
    // is followed by the end of the authority or a `:port`: without
    // that, `http://[::1]evil.com` would read as `::1` and be classified
    // loopback while the real host is `[::1]evil.com`. An unterminated
    // bracket is not a host at all.
    let host = if let Some(bracketed) = without_userinfo.strip_prefix('[') {
        match bracketed.split_once(']') {
            Some((inside, rest)) if rest.is_empty() || rest.starts_with(':') => inside,
            _ => "",
        }
    } else {
        without_userinfo.split(':').next().unwrap_or("")
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    matches!(host.parse::<std::net::IpAddr>(), Ok(ip) if ip.is_loopback())
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
        // Credentials never travel over cleartext http, except to a
        // loopback host (see require_credentials_transport).
        require_credentials_transport(url, &credentials)?;
        let mut last_error: Option<GeectlError> = None;
        for attempt in 0..=RETRY_POLICY.max_retries {
            let request = self.apply_auth(self.agent.get(url), &credentials)?;
            match request.call() {
                Ok(resp) => {
                    if resp.status().is_redirection() {
                        let location = resp.headers().get("location").and_then(|v| v.to_str().ok());
                        return Err(redirect_error(resp.status().as_u16(), location));
                    }
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

    fn post_json(&self, url: &str, body: &str) -> Result<String, GeectlError> {
        // Resolve credentials once: re-resolving per attempt would
        // re-read the secret source for no reason — and there is only
        // one attempt, because POST never retries.
        let credentials = match &self.auth {
            Some(auth) => auth.credentials()?,
            None => Credentials::None,
        };
        // Credentials never travel over cleartext http, except to a
        // loopback host (see require_credentials_transport).
        require_credentials_transport(url, &credentials)?;
        let request = self.apply_auth(self.agent.post(url), &credentials)?;
        let request = request.header("Content-Type", "application/json");
        match request.send(body) {
            Ok(resp) => {
                if resp.status().is_redirection() {
                    let location = resp.headers().get("location").and_then(|v| v.to_str().ok());
                    return Err(redirect_error(resp.status().as_u16(), location));
                }
                read_body_capped(resp.into_body().into_reader())
            }
            Err(ureq::Error::StatusCode(code)) => {
                let safe_url = sanitize_url_for_error(url);
                // No retry for POST: return the mapped error
                // immediately, even for a transient status — a retried
                // mutation can create a duplicate.
                let err = match status_action(code, &safe_url) {
                    StatusAction::Return(err) | StatusAction::Retry(err) => err,
                };
                Err(err)
            }
            Err(ureq::Error::Tls(msg)) => Err(GeectlError::new(
                ErrorKind::TlsError,
                format!("TLS error: {msg}"),
            )),
            Err(ureq::Error::Rustls(e)) => Err(GeectlError::new(
                ErrorKind::TlsError,
                format!("TLS error: {e}"),
            )),
            Err(ureq::Error::Pem(e)) => Err(GeectlError::new(
                ErrorKind::TlsError,
                format!("TLS error: {e}"),
            )),
            Err(ureq::Error::Timeout(_)) => Err(GeectlError::new(
                ErrorKind::Timeout,
                format!("request to {} timed out", sanitize_url_for_error(url)),
            )),
            Err(ureq::Error::Io(e)) => {
                Err(GeectlError::new(ErrorKind::NetworkError, e.to_string()))
            }
            Err(e) => Err(GeectlError::new(ErrorKind::ServerError, e.to_string())),
        }
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

    // --- Authenticated-request tests (loopback server, fixture creds) ---

    /// An [`AuthProvider`] returning fixed credentials, for tests. The
    /// real providers read the process environment; this one lets the
    /// HTTP tests supply exact credentials without touching env vars.
    struct FixedAuth(Credentials);

    impl AuthProvider for FixedAuth {
        fn credentials(&self) -> Result<Credentials, GeectlError> {
            Ok(self.0.clone())
        }
    }

    /// The offset of the first occurrence of `needle` in `haystack`,
    /// if any. A byte search, not a text search: the header/body
    /// boundary must be found in the raw request bytes (see
    /// `serve_once`), not in a lossy decoding of them.
    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    /// A one-shot loopback HTTP server for auth tests: accepts a single
    /// request, records it (method line, headers, body), and answers
    /// with the given body. The client talks to it over plain http:// —
    /// loopback only, fixture credentials only, never a real endpoint
    /// and never a real credential. The accept/read are bounded (5s)
    /// so a test that never connects fails instead of hanging the suite.
    struct LoopbackServer {
        port: u16,
        handle: std::thread::JoinHandle<Option<String>>,
    }

    impl LoopbackServer {
        /// Accept one request, record it (method line, headers, body),
        /// answer with `response`, and return the port plus the handle
        /// that yields the recorded request. The read is
        /// Content-Length-aware: reading headers only would leave a
        /// POST body unread and the recorded request incomplete.
        fn serve_once(response: String) -> (u16, std::thread::JoinHandle<Option<String>>) {
            let listener =
                std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback server");
            listener.set_nonblocking(true).expect("nonblocking");
            let port = listener.local_addr().expect("local addr").port();
            let handle = std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break Some(stream),
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            if std::time::Instant::now() > deadline {
                                break None;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Err(_) => break None,
                    }
                }?;
                use std::io::{Read, Write};
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                // Read the headers, then the body: the body length comes
                // from Content-Length (a request without one has no
                // body). Reading headers only would leave a POST body
                // unread and the recorded request incomplete.
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            // Search the raw bytes for the header/body
                            // boundary: decoding first would let a
                            // multi-byte body sequence mimic (or hide)
                            // the boundary in the lossy text. Only the
                            // header slice is decoded, for the
                            // Content-Length parse.
                            if let Some(end) = find_subslice(&buf, b"\r\n\r\n") {
                                let header_end = end + 4;
                                let header_text = String::from_utf8_lossy(&buf[..end]);
                                let content_length: usize = header_text
                                    .lines()
                                    .find_map(|l| {
                                        let (k, v) = l.split_once(':')?;
                                        if k.trim().eq_ignore_ascii_case("content-length") {
                                            v.trim().parse().ok()
                                        } else {
                                            None
                                        }
                                    })
                                    .unwrap_or(0);
                                if buf.len() >= header_end + content_length {
                                    break;
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
                let request = String::from_utf8_lossy(&buf).into_owned();
                let _ = stream.write_all(response.as_bytes());
                Some(request)
            });
            (port, handle)
        }

        fn serve(body: &'static str) -> Self {
            Self::serve_status(body, 200)
        }

        fn serve_status(body: &'static str, status: u16) -> Self {
            let reason = if status == 200 { "OK" } else { "Error" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let (port, handle) = Self::serve_once(response);
            LoopbackServer { port, handle }
        }

        /// Answer 302 to `location`: the client must follow it, and
        /// must not carry the Authorization header to the target.
        fn serve_redirect(location: &str) -> Self {
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let (port, handle) = Self::serve_once(response);
            LoopbackServer { port, handle }
        }

        fn url(&self) -> String {
            format!("http://127.0.0.1:{}/test", self.port)
        }

        /// The recorded request (method line, headers, body), if the
        /// client connected.
        fn request(self) -> Option<String> {
            self.handle.join().ok().flatten()
        }

        fn authorization(self) -> Option<String> {
            self.request().and_then(|request| {
                request
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("authorization:"))
                    .map(|l| l["authorization:".len()..].trim().to_string())
            })
        }
    }

    #[test]
    fn basic_auth_is_sent_as_authorization_header() {
        let server = LoopbackServer::serve("hello");
        let client = UreqClient::with_auth(
            Timeout::default(),
            Some(Box::new(FixedAuth(Credentials::Basic {
                username: "alice".to_string(),
                password: "s3cr3t".to_string(),
            }))),
        )
        .unwrap();
        assert_eq!(client.get(&server.url()).unwrap(), "hello");
        use base64::Engine;
        let expected = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("alice:s3cr3t")
        );
        assert_eq!(server.authorization().as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn bearer_auth_is_sent_as_authorization_header() {
        let server = LoopbackServer::serve("hello");
        let client = UreqClient::with_auth(
            Timeout::default(),
            Some(Box::new(FixedAuth(Credentials::Bearer {
                token: "tok3n".to_string(),
            }))),
        )
        .unwrap();
        assert_eq!(client.get(&server.url()).unwrap(), "hello");
        assert_eq!(server.authorization().as_deref(), Some("Bearer tok3n"));
    }

    #[test]
    fn post_json_sends_json_body_with_auth() {
        let server = LoopbackServer::serve("created");
        let client = UreqClient::with_auth(
            Timeout::default(),
            Some(Box::new(FixedAuth(Credentials::Bearer {
                token: "tok-123".to_string(),
            }))),
        )
        .unwrap();
        let body = "{\"name\":\"geectl-live\"}";
        assert_eq!(client.post_json(&server.url(), body).unwrap(), "created");
        let request = server.request().expect("server recorded the request");
        // Method, content type, body, and auth — all four must be
        // exactly what a Gitea create-repo call needs.
        assert!(
            request.starts_with("POST /test HTTP/1.1\r\n"),
            "POST method line wrong; request: {request}"
        );
        assert!(
            request
                .lines()
                .any(|l| l.eq_ignore_ascii_case("content-type: application/json")),
            "POST must carry Content-Type: application/json; request: {request}"
        );
        let sent_body = request
            .split("\r\n\r\n")
            .nth(1)
            .expect("request has a body");
        assert_eq!(sent_body, body);
        assert!(
            request
                .lines()
                .any(|l| l.eq_ignore_ascii_case("authorization: Bearer tok-123")),
            "POST must carry the Bearer <redacted> request: {request}"
        );
    }

    #[test]
    fn post_json_maps_404_to_not_found() {
        let server = LoopbackServer::serve_status("no such repo", 404);
        let client = UreqClient::with_auth(Timeout::default(), None).unwrap();
        let err = client
            .post_json(&server.url(), "{}")
            .expect_err("a 404 POST must fail");
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn post_json_maps_401_to_auth_failed() {
        let server = LoopbackServer::serve_status("bad token", 401);
        let client = UreqClient::with_auth(
            Timeout::default(),
            Some(Box::new(FixedAuth(Credentials::Bearer {
                token: "bad-token".to_string(),
            }))),
        )
        .unwrap();
        let err = client
            .post_json(&server.url(), "{}")
            .expect_err("a 401 POST must fail");
        assert_eq!(err.kind(), ErrorKind::AuthFailed);
    }

    #[test]
    fn post_json_over_non_loopback_http_is_refused() {
        // Cleartext http to a real host must never carry a credential —
        // the same transport guard as GET, for the mutation path.
        let client = UreqClient::with_auth(
            Timeout::default(),
            Some(Box::new(FixedAuth(Credentials::Bearer {
                token: "tok-123".to_string(),
            }))),
        )
        .unwrap();
        let err = client
            .post_json("http://example.com/api/v1/user/repos", "{}")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn ssh_key_credentials_are_refused_for_http() {
        // An SSH key cannot authenticate an HTTP request: the refusal
        // happens before any network traffic.
        let client = UreqClient::with_auth(
            Timeout::default(),
            Some(Box::new(FixedAuth(Credentials::SshKey {
                path: std::path::PathBuf::from("/home/user/.ssh/id_ed25519"),
            }))),
        )
        .unwrap();
        let err = client.get("https://example.com/").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn credentials_over_non_loopback_http_are_refused() {
        // Cleartext http to a real host must never carry a credential.
        // The refusal happens before connecting.
        let client = UreqClient::with_auth(
            Timeout::default(),
            Some(Box::new(FixedAuth(Credentials::Basic {
                username: "alice".to_string(),
                password: "s3cr3t".to_string(),
            }))),
        )
        .unwrap();
        let err = client.get("http://example.com/").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn userinfo_detection_is_authority_only() {
        assert!(url_has_userinfo("http://user:pass@example.com/"));
        assert!(url_has_userinfo("http://user@example.com"));
        assert!(!url_has_userinfo("http://example.com/@user"));
        assert!(!url_has_userinfo("http://example.com/?x=user@host"));
        assert!(!url_has_userinfo("http://example.com/"));
        assert!(!url_has_userinfo("example.com"));
    }

    #[test]
    fn userinfo_url_without_credentials_is_still_transport_checked() {
        // A userinfo URL carries a credential in the URL itself, so
        // the transport check applies even with Credentials::None:
        // http://user:pass@example.com/ would otherwise send the
        // password in cleartext.
        assert!(
            require_credentials_transport("http://user:pass@example.com/", &Credentials::None)
                .is_err()
        );
        // Loopback and https userinfo URLs are not a cleartext leak.
        assert!(
            require_credentials_transport("http://user:pass@127.0.0.1/x", &Credentials::None)
                .is_ok()
        );
        assert!(require_credentials_transport(
            "https://user:pass@example.com/",
            &Credentials::None
        )
        .is_ok());
        // An `@` in the path is not userinfo.
        assert!(
            require_credentials_transport("http://example.com/@user", &Credentials::None).is_ok()
        );
        // And with credentials configured, a plain http URL is still
        // refused (the pre-existing rule).
        assert!(require_credentials_transport(
            "http://example.com/",
            &Credentials::Basic {
                username: "alice".to_string(),
                password: "s3cr3t".to_string(),
            }
        )
        .is_err());
    }

    #[test]
    fn redirect_is_not_followed() {
        // Automatic redirects are disabled: a 302 is an error, and
        // the redirect target sees no request at all — no metadata
        // and no Authorization header reach it.
        let target = LoopbackServer::serve("target-body");
        let redirect = LoopbackServer::serve_redirect(&target.url());
        let client = UreqClient::with_auth(
            Timeout::default(),
            Some(Box::new(FixedAuth(Credentials::Bearer {
                token: "tok3n".to_string(),
            }))),
        )
        .unwrap();
        let err = client.get(&redirect.url()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("redirect"), "{err:?}");
        // The redirector saw exactly one request (the initial GET);
        // the target saw none (joining its handle waits out the
        // 5s accept deadline, so this test is slow by design).
        let first = redirect.request().expect("redirector recorded the request");
        assert!(first.starts_with("GET /test HTTP/1.1\r\n"), "{first}");
        assert_eq!(target.request(), None);
    }

    #[test]
    fn loopback_detection_is_authority_only() {
        assert!(is_loopback_url("http://127.0.0.1:8080/x"));
        assert!(is_loopback_url("http://localhost/x"));
        assert!(is_loopback_url("http://LOCALHOST:8080/x"));
        assert!(is_loopback_url("http://[::1]:8080/x"));
        assert!(is_loopback_url("http://user:pass@127.0.0.1/x"));
        assert!(!is_loopback_url("http://example.com/"));
        // A DNS name that merely starts with "127." is not loopback.
        assert!(!is_loopback_url("http://127.0.0.1.evil.com/"));
        // A loopback string in the path or query does not count.
        assert!(!is_loopback_url("http://example.com/127.0.0.1"));
        assert!(!is_loopback_url("http://example.com/?x=127.0.0.1"));
        // A bracketed host needs its closing bracket followed by the
        // end of the authority or a port: `[::1]evil.com` is a
        // different host, not loopback; an unterminated bracket is
        // not a host at all.
        assert!(!is_loopback_url("http://[::1]evil.com/"));
        assert!(!is_loopback_url("http://[::1/x"));
        // No scheme names no host.
        assert!(!is_loopback_url("127.0.0.1/x"));
    }
}
