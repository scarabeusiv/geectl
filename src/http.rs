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
//! improvisation): only idempotent methods are retried (GET, HEAD, PUT).
//! POST is never retried without an explicit idempotency key, and
//! DELETE is never retried either: a retried delete cannot tell "the
//! first attempt never ran" from "the first attempt deleted it and the
//! response was lost", so the second attempt 404s and a successful
//! delete reads as a failure. `post_json` never retries — a retried
//! mutation can create a duplicate.
//! Within a retryable method, only transient failures are retried
//! (timeouts, transport errors, 429, 5xx). Client errors (other 4xx),
//! auth failures, and TLS failures are returned immediately.
//!
//! Non-2xx statuses are converted to a [`crate::error::GeectlError`] at
//! this layer, before any caller sees the body, so callers never have to
//! remember to check the status.

use crate::auth::{validate_basic, validate_bearer_token, AuthProvider, Credentials};
use crate::debug::Debug;
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
/// retried: a retried mutation can create a duplicate. DELETE is not
/// retried either, even though it is idempotent in the HTTP sense: a
/// retried delete cannot tell "the first attempt never ran" from "the
/// first attempt deleted it and the response was lost", so the second
/// attempt 404s and a successful delete reads as a failure. DELETE is
/// single-attempt by design (see `UreqClient::delete`).
pub fn is_retryable_method(method: &str) -> bool {
    matches!(method, "GET" | "HEAD" | "PUT")
}

/// The HTTP client interface. Implementations are synchronous and return
/// the response body as a string on a 2xx status, or a [`GeectlError`].
/// `get` uses the client's configured credentials (from its
/// [`AuthProvider`], or none); `get_with`, `get_bytes`, `post`,
/// `post_text`, and `delete` take explicit credentials, for callers
/// that resolve them
/// per request (e.g. the OBS operations, which read public endpoints
/// without credentials and authenticated endpoints with OBS Basic).
/// `get` and `get_with` ask for `application/xml`: the API answers
/// XML, and a path the WebUI also serves (the owner search proved it)
/// would otherwise come back as an HTML page. `get_bytes` sends no
/// `Accept` header: a binary download is not an XML document, and
/// asking for XML could invite an XML error document in place of the
/// file.
/// `post` sends the body as `application/xml`, `post_text` as
/// `text/plain` (the request `changestate` comment, which the server
/// reads as the raw body); both, and `delete`, are never retried.
/// `put` sends the body as `application/xml` (a project or package
/// `_meta` document) and IS retried: PUT is idempotent — re-PUTting
/// the same `_meta` returns the same 200 — so a transient failure
/// retries like GET. `put_file` sends the body as
/// `application/octet-stream` (a source file) and IS retried like
/// `put`: re-PUTting the same file stores the same bytes.
/// `post_json` sends the body as `application/json` and resolves
/// credentials from the client's auth seam instead;
/// `post_json_with` is the same POST with explicit credentials, for
/// callers that hold their own credentials (the Gitea client, whose
/// Bearer must be the single source for every call it makes —
/// a `post_json` here would silently fork as whoever the HTTP
/// client's seam resolves, not as the Gitea client's token).
/// The mutation methods (`post`, `post_text`, `post_json`,
/// `post_json_with`, `put`, `put_json_with`, `put_file`, `delete`) all ask for
/// `application/xml` (`Accept` header): a path that exists in both
/// the API and WebUI route sets serves the WebUI HTML page unless
/// the request asks for XML.
pub trait HttpClient {
    fn get(&self, url: &str) -> Result<String, GeectlError>;
    fn get_with(&self, url: &str, credentials: &Credentials) -> Result<String, GeectlError>;
    /// GET a URL and return the raw response bytes. The byte
    /// counterpart of [`HttpClient::get_with`]: same transport guard,
    /// same retry policy, same body cap — but no UTF-8 check, so it
    /// can download a binary (an RPM), and no `Accept` header: the
    /// bytes are a file, not an XML document. The default implementation
    /// refuses: a client that only reads text cannot download
    /// binaries, and a caller that needs bytes gets an honest error
    /// instead of a mojibake string. [`UreqClient`] overrides it.
    fn get_bytes(&self, url: &str, credentials: &Credentials) -> Result<Vec<u8>, GeectlError> {
        let _ = (url, credentials);
        Err(GeectlError::new(
            ErrorKind::InternalError,
            "this HTTP client cannot download binary bodies",
        ))
    }
    fn post(&self, url: &str, body: &str, credentials: &Credentials)
        -> Result<String, GeectlError>;

    /// POST a JSON body. The body is sent as-is with
    /// `Content-Type: application/json`; the caller builds the JSON.
    /// Unlike GET, POST is never retried: a retried mutation can create
    /// a duplicate, so every non-2xx status (and every transport
    /// failure) is returned immediately. Auth and the transport guard
    /// work exactly as for GET.
    fn post_json(&self, url: &str, body: &str) -> Result<String, GeectlError>;
    /// POST a JSON body with explicit credentials. The `post_json`
    /// counterpart of [`HttpClient::post`]: same single-attempt POST,
    /// same `application/json` body, but the credentials come from
    /// the caller instead of the client's auth seam — so a client
    /// that holds its own credentials (the Gitea client) never
    /// authenticates a mutation as someone else.
    fn post_json_with(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError>;
    /// PUT a JSON body with explicit credentials. The `put`
    /// counterpart of [`HttpClient::post_json_with`]: the same
    /// `application/json` body and the credentials from the caller
    /// instead of the client's auth seam — so a client that holds
    /// its own credentials (the Gitea client) never authenticates
    /// a mutation as someone else. Unlike POST, PUT is retried:
    /// it is idempotent — re-PUTting the same collaborator or team
    /// member stores the same state — so a transient failure
    /// retries like GET.
    fn put_json_with(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError>;
    fn post_text(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError>;
    /// PUT a body as `application/xml` (a project or package `_meta`
    /// document). Unlike POST, PUT is retried: it is idempotent —
    /// re-PUTting the same `_meta` returns the same 200 — so a
    /// transient failure retries like GET.
    fn put(&self, url: &str, body: &str, credentials: &Credentials) -> Result<String, GeectlError>;
    /// PUT a file body as `application/octet-stream` (a source file in
    /// a package). Retried like `put`: re-PUTting the same file stores
    /// the same bytes, so a transient failure cannot corrupt anything
    /// (a retry records a second revision with identical content).
    /// The server reads the raw body as the file content (see
    /// `SourcePackageController#update_file`); the content type
    /// carries no meaning to it beyond being the file's bytes.
    fn put_file(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError>;
    /// PUT a binary body as `application/octet-stream` (a source file
    /// in a package whose content is not UTF-8 text). The byte
    /// counterpart of [`HttpClient::put_file`]: same transport guard,
    /// same retry policy — but the body is bytes, not a string, so a
    /// binary file (a tarball, an RPM) survives the transfer instead
    /// of failing the UTF-8 check a text body would hit. The default
    /// implementation refuses: a client that only PUTs text cannot
    /// upload binaries, and a caller that needs bytes gets an honest
    /// error instead of a mojibake upload. [`UreqClient`] overrides it.
    fn put_bytes(
        &self,
        url: &str,
        body: &[u8],
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        let _ = (url, body, credentials);
        Err(GeectlError::new(
            ErrorKind::InternalError,
            "this HTTP client cannot upload binary bodies",
        ))
    }
    fn delete(&self, url: &str, credentials: &Credentials) -> Result<String, GeectlError>;
}

/// A [`HttpClient`] backed by ureq, with bounded timeouts and the retry
/// policy applied to GET. Auth, when provided, is applied from the
/// [`AuthProvider`] seam — the client never reads credentials itself.
pub struct UreqClient {
    agent: ureq::Agent,
    auth: Option<Box<dyn AuthProvider>>,
    debug: Debug,
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
        Ok(UreqClient {
            agent,
            auth,
            debug: Debug::disabled(),
        })
    }

    /// Attach the debug sink: HTTP attempts, retries, and endpoint
    /// selection are reported on the channel when it is on. The
    /// default is off, so the existing constructors are unaffected.
    pub fn with_debug(mut self, debug: Debug) -> Self {
        self.debug = debug;
        self
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

    /// One GET, retried: the retry loop shared by `get_with` (text)
    /// and `get_bytes` (raw bytes). Only the body reader differs:
    /// text goes through the UTF-8 check, bytes do not. `accept` is
    /// the `Accept` header to send, if any: text reads ask for
    /// `application/xml` (a dual-routed path would otherwise serve the
    /// WebUI HTML page), a binary download sends none — the bytes are
    /// a file, not an XML document. `read_body` is `FnOnce`: the body
    /// is read at most once, on the first successful response, so a
    /// closure that consumes its captures is fine. A redirect is
    /// refused, like POST: automatic redirects are disabled, so a 3xx
    /// is an error, not a body.
    ///
    /// `emit_debug_attempts` selects the debug channel's per-attempt
    /// lines: text reads emit them, binary downloads do not. The debug
    /// change instrumented text reads only, so a byte download stays
    /// silent even though it shares this loop.
    fn get_with_body<T>(
        &self,
        url: &str,
        credentials: &Credentials,
        accept: Option<&str>,
        emit_debug_attempts: bool,
        read_body: impl FnOnce(ureq::Body) -> Result<T, GeectlError>,
    ) -> Result<T, GeectlError> {
        // Credentials never travel over cleartext http, except to a
        // loopback host (see require_credentials_transport).
        require_credentials_transport(url, credentials)?;
        let mut last_error: Option<GeectlError> = None;
        let mut last_desc = String::new();
        for attempt in 0..=RETRY_POLICY.max_retries {
            let started = std::time::Instant::now();
            // Ask for XML when the caller wants it: the deployment
            // serves the WebUI HTML page for a path that exists in
            // both the API and WebUI route sets (the owner search
            // proved it) unless the request asks for XML. A binary
            // download sends no `Accept` header (see `get_bytes`).
            let mut request = self.apply_auth(self.agent.get(url), credentials)?;
            if let Some(accept) = accept {
                request = request.header("Accept", accept);
            }
            match request.call() {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if resp.status().is_redirection() {
                        let location = resp.headers().get("location").and_then(|v| v.to_str().ok());
                        if emit_debug_attempts {
                            self.debug_attempt(
                                "GET",
                                url,
                                &status.to_string(),
                                started.elapsed(),
                                attempt + 1,
                                None,
                            );
                        }
                        return Err(redirect_error(status, location));
                    }
                    if emit_debug_attempts {
                        self.debug_attempt(
                            "GET",
                            url,
                            &status.to_string(),
                            started.elapsed(),
                            attempt + 1,
                            None,
                        );
                    }
                    return read_body(resp.into_body());
                }
                Err(ureq::Error::StatusCode(code)) => {
                    let safe_url = sanitize_url_for_error(url);
                    match status_action(code, &safe_url) {
                        StatusAction::Return(err) => {
                            if emit_debug_attempts {
                                self.debug_attempt(
                                    "GET",
                                    url,
                                    &code.to_string(),
                                    started.elapsed(),
                                    attempt + 1,
                                    None,
                                );
                            }
                            return Err(err);
                        }
                        StatusAction::Retry(err) => {
                            if emit_debug_attempts {
                                let backoff = RETRY_POLICY.backoff_base * 2u32.pow(attempt);
                                self.debug_attempt(
                                    "GET",
                                    url,
                                    &code.to_string(),
                                    started.elapsed(),
                                    attempt + 1,
                                    Some((backoff, &format!("transient status {code}"))),
                                );
                            }
                            last_desc = code.to_string();
                            last_error = Some(err);
                        }
                    }
                }
                Err(ureq::Error::Tls(msg)) => {
                    // TLS failures are never retried: retrying a failed
                    // handshake or an untrusted certificate cannot
                    // succeed, and it would hammer a server that
                    // already refused us.
                    if emit_debug_attempts {
                        self.debug_attempt(
                            "GET",
                            url,
                            "TLS error",
                            started.elapsed(),
                            attempt + 1,
                            None,
                        );
                    }
                    return Err(GeectlError::new(
                        ErrorKind::TlsError,
                        format!("TLS error: {msg}"),
                    ));
                }
                Err(ureq::Error::Rustls(e)) => {
                    if emit_debug_attempts {
                        self.debug_attempt(
                            "GET",
                            url,
                            "TLS error",
                            started.elapsed(),
                            attempt + 1,
                            None,
                        );
                    }
                    return Err(GeectlError::new(
                        ErrorKind::TlsError,
                        format!("TLS error: {e}"),
                    ));
                }
                Err(ureq::Error::Pem(e)) => {
                    if emit_debug_attempts {
                        self.debug_attempt(
                            "GET",
                            url,
                            "TLS error",
                            started.elapsed(),
                            attempt + 1,
                            None,
                        );
                    }
                    return Err(GeectlError::new(
                        ErrorKind::TlsError,
                        format!("TLS error: {e}"),
                    ));
                }
                Err(ureq::Error::Timeout(_)) => {
                    if emit_debug_attempts {
                        let backoff = RETRY_POLICY.backoff_base * 2u32.pow(attempt);
                        self.debug_attempt(
                            "GET",
                            url,
                            "timeout",
                            started.elapsed(),
                            attempt + 1,
                            Some((backoff, "timeout")),
                        );
                    }
                    last_desc = "timeout".to_string();
                    last_error = Some(GeectlError::new(
                        ErrorKind::Timeout,
                        format!("request to {} timed out", sanitize_url_for_error(url)),
                    ));
                }
                Err(ureq::Error::Io(e)) => {
                    if emit_debug_attempts {
                        let backoff = RETRY_POLICY.backoff_base * 2u32.pow(attempt);
                        self.debug_attempt(
                            "GET",
                            url,
                            "network error",
                            started.elapsed(),
                            attempt + 1,
                            Some((backoff, "network error")),
                        );
                    }
                    last_desc = "network error".to_string();
                    last_error = Some(GeectlError::new(ErrorKind::NetworkError, e.to_string()));
                }
                Err(e) => {
                    if emit_debug_attempts {
                        let backoff = RETRY_POLICY.backoff_base * 2u32.pow(attempt);
                        self.debug_attempt(
                            "GET",
                            url,
                            "server error",
                            started.elapsed(),
                            attempt + 1,
                            Some((backoff, "server error")),
                        );
                    }
                    last_desc = "server error".to_string();
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
        if emit_debug_attempts && last_error.is_some() {
            self.debug_exhausted("GET", url, RETRY_POLICY.max_retries + 1, &last_desc);
        }
        Err(last_error.unwrap_or_else(|| {
            GeectlError::new(ErrorKind::InternalError, "GET failed without an error")
        }))
    }

    /// Emit one HTTP attempt line for a retried method: method,
    /// path, status or error kind, duration, attempt against the
    /// retry budget. On a retryable failure `retry` carries the
    /// backoff and the reason. Nothing is formatted when the channel
    /// is off.
    fn debug_attempt(
        &self,
        method: &str,
        url: &str,
        outcome: &str,
        duration: Duration,
        attempt: u32,
        retry: Option<(Duration, &str)>,
    ) {
        if !self.debug.enabled() {
            return;
        }
        let total = RETRY_POLICY.max_retries + 1;
        let path = crate::debug::url_path(url);
        // The last attempt has no retry after it, so its line must
        // not promise one: a caller that computed a backoff for a
        // failed final attempt still reports a plain attempt line.
        let retry = retry.filter(|_| attempt < total);
        match retry {
            Some((backoff, reason)) => self.debug.emit(&format!(
                "http: {method} {path} -> {outcome} in {duration:?} (attempt {attempt}/{total}, retry in {backoff:?}: {reason})"
            )),
            None => self.debug.emit(&format!(
                "http: {method} {path} -> {outcome} in {duration:?} (attempt {attempt}/{total})"
            )),
        }
    }

    /// Emit one attempt line for a method that never retries
    /// (POST, DELETE): a single attempt, so the budget is 1 of 1.
    /// Nothing is formatted when the channel is off.
    fn debug_single_attempt(&self, method: &str, url: &str, outcome: &str, duration: Duration) {
        if !self.debug.enabled() {
            return;
        }
        let path = crate::debug::url_path(url);
        self.debug.emit(&format!(
            "http: {method} {path} -> {outcome} in {duration:?} (attempt 1/1)"
        ));
    }

    /// Emit the exhaustion line after the retry budget runs out.
    fn debug_exhausted(&self, method: &str, url: &str, attempts: u32, last: &str) {
        if !self.debug.enabled() {
            return;
        }
        let path = crate::debug::url_path(url);
        self.debug.emit(&format!(
            "http: {method} {path} failed after {attempts} attempts (last: {last})"
        ));
    }

    /// Emit a request or response body line, behind `--debug-body`
    /// only. The body is capped and redacted by
    /// [`crate::debug::format_body_preview`]; headers are never
    /// printed.
    fn debug_body(&self, direction: &str, method: &str, url: &str, body: &str) {
        if !self.debug.bodies_enabled() {
            return;
        }
        let path = crate::debug::url_path(url);
        let preview = crate::debug::format_body_preview(body);
        self.debug.emit(&format!(
            "http: {method} {path} {direction} body ({} bytes): {preview}",
            body.len()
        ));
    }

    /// Emit the binary marker for a response body that failed UTF-8
    /// decoding, behind `--debug-body` only. The caller still
    /// returns the exact `ServerError` for the undecodable body, so
    /// the marker is the only difference: the transcript shows the
    /// body was binary rather than absent.
    fn debug_binary_body(&self, method: &str, url: &str, len: usize) {
        if !self.debug.bodies_enabled() {
            return;
        }
        let path = crate::debug::url_path(url);
        self.debug.emit(&format!(
            "http: {method} {path} response body ({len} bytes): <binary body, {len} bytes>"
        ));
    }

    /// Read a response body for a `method` request to `url`. On
    /// success the caller still emits the `response body` line (after
    /// the attempt line). A body that is not valid UTF-8
    /// still errors (`ServerError`, same message);
    /// the binary marker line is emitted behind `--debug-body` only,
    /// so a non-UTF-8 response is visible in the transcript instead
    /// of vanishing at the `?`. The caller emits the attempt line
    /// before calling this: on a read error the marker is the only
    /// body line, and it must follow its attempt line rather than
    /// standing alone. The cap, the error kind, and the message are
    /// unchanged.
    fn read_response_body(
        &self,
        reader: impl Read,
        method: &str,
        url: &str,
    ) -> Result<String, GeectlError> {
        let bytes = read_body_bytes_capped(reader)?;
        match String::from_utf8(bytes) {
            Ok(body) => Ok(body),
            Err(e) => {
                let len = e.as_bytes().len();
                self.debug_binary_body(method, url, len);
                Err(GeectlError::new(
                    ErrorKind::ServerError,
                    format!("response body is not valid UTF-8: {e}"),
                ))
            }
        }
    }

    /// Emit the endpoint-selection decision for a read: which route
    /// the request takes, and whether it carries credentials. The
    /// route is the first path segment (`/public/source` for public
    /// reads, `/source` or `/request` otherwise); an SSH key never
    /// authenticates HTTP, so it reports no selection.
    fn debug_endpoint(&self, url: &str, credentials: &Credentials) {
        if !self.debug.enabled() {
            return;
        }
        let auth = match credentials {
            Credentials::None => "no credentials",
            Credentials::Basic { .. } | Credentials::Bearer { .. } => "authenticated",
            Credentials::SshKey { .. } => return,
        };
        let path = crate::debug::url_path(url);
        let mut segments = path.split('/').filter(|s| !s.is_empty());
        let route = match (segments.next(), segments.next()) {
            (Some("public"), Some(second)) => format!("/public/{second}"),
            (Some(first), _) => format!("/{first}"),
            _ => "/".to_string(),
        };
        self.debug
            .emit(&format!("decision: read via {route} ({auth})"));
    }

    /// One POST attempt with the given content type. POST is never
    /// retried: a retried mutation can create a duplicate (e.g. a
    /// second submit request). A transient failure (429, 5xx) is
    /// returned as an error immediately, not retried. This is a
    /// single attempt, end to end. A redirect is refused, like GET:
    /// automatic redirects are disabled, so a 3xx is an error, not a body.
    fn post_with_content_type(
        &self,
        url: &str,
        body: &str,
        content_type: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        require_credentials_transport(url, credentials)?;
        debug_assert!(
            !is_retryable_method("POST"),
            "POST must stay non-retryable; a retried mutation can duplicate"
        );
        // Ask for XML, the same defence as GET: the owner search proved
        // a dual-routed path serves the WebUI HTML page unless the request
        // asks for XML. Mutations ask for XML too, as a preventive
        // measure.
        let request = self
            .agent
            .post(url)
            .header("Content-Type", content_type)
            .header("Accept", "application/xml");
        let request = self.apply_auth(request, credentials)?;
        // A single attempt: POST is never retried.
        let started = std::time::Instant::now();
        self.debug_body("request", "POST", url, body);
        match request.send(body) {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if resp.status().is_redirection() {
                    let location = resp.headers().get("location").and_then(|v| v.to_str().ok());
                    self.debug_single_attempt("POST", url, &status.to_string(), started.elapsed());
                    return Err(redirect_error(status, location));
                }
                // Attempt line first (see read_response_body).
                self.debug_single_attempt("POST", url, &status.to_string(), started.elapsed());
                let body = self.read_response_body(resp.into_body().into_reader(), "POST", url)?;
                self.debug_body("response", "POST", url, &body);
                Ok(body)
            }
            Err(ureq::Error::StatusCode(code)) => {
                let safe_url = sanitize_url_for_error(url);
                self.debug_single_attempt("POST", url, &code.to_string(), started.elapsed());
                match status_action(code, &safe_url) {
                    // No retry for POST: a Retry action is returned
                    // as the error, immediately.
                    StatusAction::Return(err) | StatusAction::Retry(err) => Err(err),
                }
            }
            Err(ureq::Error::Tls(msg)) => {
                self.debug_single_attempt("POST", url, "TLS error", started.elapsed());
                Err(GeectlError::new(
                    ErrorKind::TlsError,
                    format!("TLS error: {msg}"),
                ))
            }
            Err(ureq::Error::Rustls(e)) => {
                self.debug_single_attempt("POST", url, "TLS error", started.elapsed());
                Err(GeectlError::new(
                    ErrorKind::TlsError,
                    format!("TLS error: {e}"),
                ))
            }
            Err(ureq::Error::Pem(e)) => {
                self.debug_single_attempt("POST", url, "TLS error", started.elapsed());
                Err(GeectlError::new(
                    ErrorKind::TlsError,
                    format!("TLS error: {e}"),
                ))
            }
            Err(ureq::Error::Timeout(_)) => {
                self.debug_single_attempt("POST", url, "timeout", started.elapsed());
                Err(GeectlError::new(
                    ErrorKind::Timeout,
                    format!("request to {} timed out", sanitize_url_for_error(url)),
                ))
            }
            Err(ureq::Error::Io(e)) => {
                self.debug_single_attempt("POST", url, "network error", started.elapsed());
                Err(GeectlError::new(ErrorKind::NetworkError, e.to_string()))
            }
            Err(e) => {
                self.debug_single_attempt("POST", url, "server error", started.elapsed());
                Err(GeectlError::new(ErrorKind::ServerError, e.to_string()))
            }
        }
    }

    fn put_with_content_type_body(
        &self,
        url: &str,
        body: &[u8],
        content_type: &str,
        credentials: &Credentials,
        debug_request_body: impl Fn(&Self),
    ) -> Result<String, GeectlError> {
        // PUT is idempotent (re-PUTting the same `_meta` returns the
        // same 200; re-PUTting the same file stores the same bytes),
        // so a transient failure retries like GET: the retry loop is
        // the same shape as `get_with`, with the method changed. A
        // lost response cannot leave a half-applied document behind —
        // the `_meta` document and the file content are both applied
        // atomically — so unlike DELETE there is no 404 ambiguity on
        // retry. A file retry records a second revision with identical
        // content; the bytes on the server are the same either way.
        //
        // The loop is shared by the text and byte callers
        // (`put_with_content_type` / `put_bytes_with_content_type`):
        // the body is always bytes — a text body arrives as
        // `body.as_bytes()` — and the caller supplies the per-attempt
        // debug line for the request body, because the loop never
        // looks at the body: a text preview, or a byte-count marker
        // for binary. The response is always text (a revision
        // document), so the response handling is shared unchanged.
        require_credentials_transport(url, credentials)?;
        let mut last_error: Option<GeectlError> = None;
        let mut last_desc = String::new();
        for attempt in 0..=RETRY_POLICY.max_retries {
            let started = std::time::Instant::now();
            // Ask for XML, the same defence as GET: the owner search proved
            // a dual-routed path serves the WebUI HTML page unless the request
            // asks for XML. Mutations ask for XML too, as a preventive
            // measure.
            let request = self
                .agent
                .put(url)
                .header("Content-Type", content_type)
                .header("Accept", "application/xml");
            let request = self.apply_auth(request, credentials)?;
            debug_request_body(self);
            match request.send(body) {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if resp.status().is_redirection() {
                        let location = resp.headers().get("location").and_then(|v| v.to_str().ok());
                        self.debug_attempt(
                            "PUT",
                            url,
                            &status.to_string(),
                            started.elapsed(),
                            attempt + 1,
                            None,
                        );
                        return Err(redirect_error(status, location));
                    }
                    // Attempt line first (see read_response_body).
                    self.debug_attempt(
                        "PUT",
                        url,
                        &status.to_string(),
                        started.elapsed(),
                        attempt + 1,
                        None,
                    );
                    let body =
                        self.read_response_body(resp.into_body().into_reader(), "PUT", url)?;
                    self.debug_body("response", "PUT", url, &body);
                    return Ok(body);
                }
                Err(ureq::Error::StatusCode(code)) => {
                    let safe_url = sanitize_url_for_error(url);
                    match status_action(code, &safe_url) {
                        StatusAction::Return(err) => {
                            self.debug_attempt(
                                "PUT",
                                url,
                                &code.to_string(),
                                started.elapsed(),
                                attempt + 1,
                                None,
                            );
                            return Err(err);
                        }
                        StatusAction::Retry(err) => {
                            let backoff = RETRY_POLICY.backoff_base * 2u32.pow(attempt);
                            self.debug_attempt(
                                "PUT",
                                url,
                                &code.to_string(),
                                started.elapsed(),
                                attempt + 1,
                                Some((backoff, &format!("transient status {code}"))),
                            );
                            last_desc = code.to_string();
                            last_error = Some(err);
                        }
                    }
                }
                Err(ureq::Error::Tls(msg)) => {
                    // TLS failures are never retried: retrying a failed
                    // handshake or an untrusted certificate cannot
                    // succeed, and it would hammer a server that
                    // already refused us.
                    self.debug_attempt(
                        "PUT",
                        url,
                        "TLS error",
                        started.elapsed(),
                        attempt + 1,
                        None,
                    );
                    return Err(GeectlError::new(
                        ErrorKind::TlsError,
                        format!("TLS error: {msg}"),
                    ));
                }
                Err(ureq::Error::Rustls(e)) => {
                    self.debug_attempt(
                        "PUT",
                        url,
                        "TLS error",
                        started.elapsed(),
                        attempt + 1,
                        None,
                    );
                    return Err(GeectlError::new(
                        ErrorKind::TlsError,
                        format!("TLS error: {e}"),
                    ));
                }
                Err(ureq::Error::Pem(e)) => {
                    self.debug_attempt(
                        "PUT",
                        url,
                        "TLS error",
                        started.elapsed(),
                        attempt + 1,
                        None,
                    );
                    return Err(GeectlError::new(
                        ErrorKind::TlsError,
                        format!("TLS error: {e}"),
                    ));
                }
                Err(ureq::Error::Timeout(_)) => {
                    let backoff = RETRY_POLICY.backoff_base * 2u32.pow(attempt);
                    self.debug_attempt(
                        "PUT",
                        url,
                        "timeout",
                        started.elapsed(),
                        attempt + 1,
                        Some((backoff, "timeout")),
                    );
                    last_desc = "timeout".to_string();
                    last_error = Some(GeectlError::new(
                        ErrorKind::Timeout,
                        format!("request to {} timed out", sanitize_url_for_error(url)),
                    ));
                }
                Err(ureq::Error::Io(e)) => {
                    let backoff = RETRY_POLICY.backoff_base * 2u32.pow(attempt);
                    self.debug_attempt(
                        "PUT",
                        url,
                        "network error",
                        started.elapsed(),
                        attempt + 1,
                        Some((backoff, "network error")),
                    );
                    last_desc = "network error".to_string();
                    last_error = Some(GeectlError::new(ErrorKind::NetworkError, e.to_string()));
                }
                Err(e) => {
                    let backoff = RETRY_POLICY.backoff_base * 2u32.pow(attempt);
                    self.debug_attempt(
                        "PUT",
                        url,
                        "server error",
                        started.elapsed(),
                        attempt + 1,
                        Some((backoff, "server error")),
                    );
                    last_desc = "server error".to_string();
                    last_error = Some(GeectlError::new(ErrorKind::ServerError, e.to_string()));
                }
            }
            // Only idempotent methods retry. This function only ever
            // issues PUT (which is retryable), so the gate is a
            // debug_assert: it documents the policy and fails a debug
            // build if the method ever changes, without dead code in
            // release. A future method must gate on its real method.
            debug_assert!(
                is_retryable_method("PUT"),
                "PUT must stay retryable; a new method needs its own gate"
            );
            if attempt < RETRY_POLICY.max_retries {
                std::thread::sleep(RETRY_POLICY.backoff_base * 2u32.pow(attempt));
            }
        }
        if last_error.is_some() {
            self.debug_exhausted("PUT", url, RETRY_POLICY.max_retries + 1, &last_desc);
        }
        Err(last_error.unwrap_or_else(|| {
            GeectlError::new(ErrorKind::InternalError, "PUT failed without an error")
        }))
    }

    /// PUT a text body with an explicit content type: the text caller
    /// of the shared retry loop. The request body gets its preview
    /// in the `--debug-body` channel, like every text mutation.
    fn put_with_content_type(
        &self,
        url: &str,
        body: &str,
        content_type: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        self.put_with_content_type_body(url, body.as_bytes(), content_type, credentials, |this| {
            this.debug_body("request", "PUT", url, body);
        })
    }

    /// PUT a binary body with an explicit content type: the byte
    /// caller of the shared retry loop. The request body is never
    /// dumped to the `--debug-body` channel — it is not text, and a
    /// preview would be mojibake at best; the channel records the
    /// byte count only. The response is still text (a revision
    /// document), so its preview is shared with the text caller.
    fn put_bytes_with_content_type(
        &self,
        url: &str,
        body: &[u8],
        content_type: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        self.put_with_content_type_body(url, body, content_type, credentials, |this| {
            if this.debug.bodies_enabled() {
                this.debug.emit(&format!(
                    "http: PUT {} request body ({} bytes): <binary, not shown>",
                    crate::debug::url_path(url),
                    body.len()
                ));
            }
        })
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

/// Read a response body with a cap, as raw bytes. Reads at most
/// `MAX_BODY_BYTES + 1` bytes; a body longer than the cap fails with
/// `ServerError` instead of growing the process without bound. The
/// `+ 1` distinguishes "at the cap" (accepted) from "over the cap"
/// (rejected). Bytes are read first and converted to UTF-8 after the
/// cap check: converting while reading would split a multi-byte
/// character at the cap (and report the over-cap body as a network
/// error), and an in-cap body that is not valid UTF-8 is a server
/// error, not a transport failure.
fn read_body_bytes_capped(reader: impl Read) -> Result<Vec<u8>, GeectlError> {
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
    Ok(bytes)
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
        // 409 is a conflict with existing state: the resource the
        // caller tried to create already exists (a fork, a repo).
        // It maps to AlreadyExists, not InvalidInput, so a caller
        // can match the conflict specifically — a 400/422 InvalidInput
        // must never be mistaken for one (the fork race re-reads
        // its target only on AlreadyExists).
        409 => StatusAction::Return(GeectlError::new(
            ErrorKind::AlreadyExists,
            format!("{safe_url} returned 409"),
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
        // wrong method — 409 is handled above, as AlreadyExists.
        // Retrying would not help.
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
        self.get_with(url, &credentials)
    }

    fn get_with(&self, url: &str, credentials: &Credentials) -> Result<String, GeectlError> {
        // The endpoint selection for this read: public or
        // authenticated, reported once per call, not per attempt.
        self.debug_endpoint(url, credentials);
        self.get_with_body(url, credentials, Some("application/xml"), true, |body| {
            let text = self.read_response_body(body.into_reader(), "GET", url)?;
            self.debug_body("response", "GET", url, &text);
            Ok(text)
        })
    }

    fn get_bytes(&self, url: &str, credentials: &Credentials) -> Result<Vec<u8>, GeectlError> {
        self.get_with_body(url, credentials, None, false, |body| {
            read_body_bytes_capped(body.into_reader())
        })
    }

    fn post_json(&self, url: &str, body: &str) -> Result<String, GeectlError> {
        // Resolve credentials once: re-resolving per attempt would
        // re-read the secret source for no reason — and there is only
        // one attempt, because POST never retries.
        let credentials = match &self.auth {
            Some(auth) => auth.credentials()?,
            None => Credentials::None,
        };
        // The JSON body goes through the shared single-attempt POST:
        // same transport guard, same error mapping, Content-Type
        // application/json.
        self.post_with_content_type(url, body, "application/json", &credentials)
    }

    fn post_json_with(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        // The caller's credentials, not the seam's: a client that
        // holds its own Bearer (the Gitea client) authenticates its
        // mutation as itself even when the HTTP client was built
        // with a different provider — or with none at all.
        self.post_with_content_type(url, body, "application/json", credentials)
    }

    fn put_json_with(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        // The JSON PUT twin of post_json_with: the same explicit
        // credentials, but PUT semantics — retried, because the
        // collaborator and team PUTs are idempotent.
        self.put_with_content_type(url, body, "application/json", credentials)
    }

    fn post(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        // The filing POST carries request XML.
        self.post_with_content_type(url, body, "application/xml", credentials)
    }

    fn post_text(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        // The changestate POST carries a plain-text comment.
        self.post_with_content_type(url, body, "text/plain", credentials)
    }

    fn put(&self, url: &str, body: &str, credentials: &Credentials) -> Result<String, GeectlError> {
        self.put_with_content_type(url, body, "application/xml", credentials)
    }

    fn put_file(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        self.put_with_content_type(url, body, "application/octet-stream", credentials)
    }

    fn put_bytes(
        &self,
        url: &str,
        body: &[u8],
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        self.put_bytes_with_content_type(url, body, "application/octet-stream", credentials)
    }

    fn delete(&self, url: &str, credentials: &Credentials) -> Result<String, GeectlError> {
        // DELETE is never retried, like POST — and `is_retryable_method`
        // agrees: DELETE is not in the retryable set. A retried delete
        // cannot tell "the first attempt never ran" from "the first
        // attempt deleted it and the response was lost": the second
        // attempt then 404s, and the retry loop would report a
        // successful delete as NotFound. A single attempt keeps the
        // outcome honest: a timeout means "unknown, check the server
        // state", never a false failure. The caller re-issues the
        // delete itself once it has checked.
        require_credentials_transport(url, credentials)?;
        // DELETE is single-attempt by design: gate on the retryable set like POST/PUT/GET.
        debug_assert!(
            !is_retryable_method("DELETE"),
            "DELETE must stay non-retryable; a retried delete cannot tell a lost response from a delete that never ran"
        );
        // Ask for XML, the same defence as GET: the owner search proved
        // a dual-routed path serves the WebUI HTML page unless the request
        // asks for XML. Mutations ask for XML too, as a preventive
        // measure.
        let request = self.agent.delete(url).header("Accept", "application/xml");
        let request = self.apply_auth(request, credentials)?;
        // A single attempt: DELETE is never retried.
        let started = std::time::Instant::now();
        match request.call() {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if resp.status().is_redirection() {
                    let location = resp.headers().get("location").and_then(|v| v.to_str().ok());
                    self.debug_single_attempt(
                        "DELETE",
                        url,
                        &status.to_string(),
                        started.elapsed(),
                    );
                    return Err(redirect_error(status, location));
                }
                // Attempt line first (see read_response_body).
                self.debug_single_attempt("DELETE", url, &status.to_string(), started.elapsed());
                let body =
                    self.read_response_body(resp.into_body().into_reader(), "DELETE", url)?;
                self.debug_body("response", "DELETE", url, &body);
                Ok(body)
            }
            Err(ureq::Error::StatusCode(code)) => {
                let safe_url = sanitize_url_for_error(url);
                self.debug_single_attempt("DELETE", url, &code.to_string(), started.elapsed());
                match status_action(code, &safe_url) {
                    // No retry for DELETE: a Retry action is returned
                    // as the error, immediately.
                    StatusAction::Return(err) | StatusAction::Retry(err) => Err(err),
                }
            }
            Err(ureq::Error::Tls(msg)) => {
                self.debug_single_attempt("DELETE", url, "TLS error", started.elapsed());
                Err(GeectlError::new(
                    ErrorKind::TlsError,
                    format!("TLS error: {msg}"),
                ))
            }
            Err(ureq::Error::Rustls(e)) => {
                self.debug_single_attempt("DELETE", url, "TLS error", started.elapsed());
                Err(GeectlError::new(
                    ErrorKind::TlsError,
                    format!("TLS error: {e}"),
                ))
            }
            Err(ureq::Error::Pem(e)) => {
                self.debug_single_attempt("DELETE", url, "TLS error", started.elapsed());
                Err(GeectlError::new(
                    ErrorKind::TlsError,
                    format!("TLS error: {e}"),
                ))
            }
            Err(ureq::Error::Timeout(_)) => {
                self.debug_single_attempt("DELETE", url, "timeout", started.elapsed());
                Err(GeectlError::new(
                    ErrorKind::Timeout,
                    format!("request to {} timed out", sanitize_url_for_error(url)),
                ))
            }
            Err(ureq::Error::Io(e)) => {
                self.debug_single_attempt("DELETE", url, "network error", started.elapsed());
                Err(GeectlError::new(ErrorKind::NetworkError, e.to_string()))
            }
            Err(e) => {
                self.debug_single_attempt("DELETE", url, "server error", started.elapsed());
                Err(GeectlError::new(ErrorKind::ServerError, e.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn retryable_methods_are_idempotent_only() {
        assert!(is_retryable_method("GET"));
        assert!(is_retryable_method("HEAD"));
        assert!(is_retryable_method("PUT"));
        assert!(!is_retryable_method("POST"));
        assert!(!is_retryable_method("PATCH"));
        // DELETE is idempotent in the HTTP sense but single-attempt
        // by design: a retried delete cannot tell a lost response
        // from a delete that never ran.
        assert!(!is_retryable_method("DELETE"));
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
        let body = read_body_bytes_capped(std::io::Cursor::new(data)).unwrap();
        assert_eq!(body.len(), MAX_BODY_BYTES);
    }

    #[test]
    fn body_over_the_cap_is_rejected() {
        let data = vec![b'a'; MAX_BODY_BYTES + 1];
        let err = read_body_bytes_capped(std::io::Cursor::new(data)).unwrap_err();
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
        let err = read_body_bytes_capped(std::io::Cursor::new(data)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn in_cap_invalid_utf8_is_server_error() {
        // A body within the cap that is not valid UTF-8 is a server
        // error, not a transport failure.
        let (debug, _) = Debug::captured(false, false);
        let client = UreqClient::new(Timeout::default())
            .unwrap()
            .with_debug(debug);
        let data = vec![0xFF; 4];
        let err = client
            .read_response_body(std::io::Cursor::new(data), "GET", "https://example.com/x")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn valid_multibyte_body_is_accepted() {
        let data = "h\u{e9}llo w\u{f6}rld".repeat(10);
        let (debug, _) = Debug::captured(false, false);
        let client = UreqClient::new(Timeout::default())
            .unwrap()
            .with_debug(debug);
        let body = client
            .read_response_body(
                std::io::Cursor::new(data.clone().into_bytes()),
                "GET",
                "https://example.com/x",
            )
            .unwrap();
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
        // 409 is a conflict with existing state, not a caller
        // fault: AlreadyExists, so a caller can match it
        // specifically instead of treating every InvalidInput as
        // a conflict.
        assert!(
            matches!(status_action(409, "u"), StatusAction::Return(e) if e.kind() == ErrorKind::AlreadyExists)
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

    /// Read one request (headers + body) from a loopback connection:
    /// the body length comes from Content-Length (a request without
    /// one has no body). Reading headers only would leave a POST body
    /// unread and the recorded request incomplete. Shared by the
    /// one-shot and counting servers.
    fn read_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
        use std::io::Read;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    // Search the raw bytes for the header/body
                    // boundary: decoding first would let a multi-byte
                    // body sequence mimic (or hide) the boundary in
                    // the lossy text. Only the header slice is
                    // decoded, for the Content-Length parse.
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
        buf
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
        fn serve_once(response: Vec<u8>) -> (u16, std::thread::JoinHandle<Option<String>>) {
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
                use std::io::Write;
                // On macOS/BSD, the accepted stream inherits the
                // listener's non-blocking mode: without this
                // the read below returns WouldBlock at once
                // (and the read timeout is a no-op), so the
                // server answers a request it never read.
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                let buf = read_request(&mut stream);
                let request = String::from_utf8_lossy(&buf).into_owned();
                let _ = stream.write_all(&response);
                Some(request)
            });
            (port, handle)
        }

        /// Answer one request with raw `response` bytes: for
        /// bodies that are not valid UTF-8, which cannot be a `String`.
        fn serve_raw(response: Vec<u8>) -> Self {
            let (port, handle) = Self::serve_once(response);
            LoopbackServer { port, handle }
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
            let (port, handle) = Self::serve_once(response.into_bytes());
            LoopbackServer { port, handle }
        }

        /// Answer 302 to `location`: the client must follow it, and
        /// must not carry the Authorization header to the target.
        fn serve_redirect(location: &str) -> Self {
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let (port, handle) = Self::serve_once(response.into_bytes());
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
                // Headers only: a body line that happens to start
                // with "authorization:" is not a header.
                let headers = match request.split_once("\r\n\r\n") {
                    Some((headers, _)) => headers,
                    None => &request,
                };
                headers
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("authorization:"))
                    .map(|l| l["authorization:".len()..].trim().to_string())
            })
        }
    }

    /// A loopback server that counts requests instead of serving once:
    /// for the retry tests, which must prove the client made exactly
    /// one attempt against a 429 or a 500. The accept loop runs until
    /// the server is dropped; dropping connects once to unblock the
    /// accept, then joins the thread.
    struct CountingServer {
        port: u16,
        hits: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl CountingServer {
        fn serve_status(body: &'static str, status: u16) -> Self {
            let reason = if status == 200 { "OK" } else { "Error" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let listener =
                std::net::TcpListener::bind("127.0.0.1:0").expect("bind counting server");
            listener.set_nonblocking(true).expect("nonblocking");
            let port = listener.local_addr().expect("local addr").port();
            let hits = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let hits_thread = Arc::clone(&hits);
            let stop_thread = Arc::clone(&stop);
            let handle = std::thread::spawn(move || {
                loop {
                    if stop_thread.load(Ordering::Relaxed) {
                        break;
                    }
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            use std::io::Write;
                            // On macOS/BSD, the accepted stream inherits the
                            // listener's non-blocking mode: without this
                            // the read below returns WouldBlock at once
                            // (and the read timeout is a no-op), so the
                            // server answers a request it never read.
                            let _ = stream.set_nonblocking(false);
                            let _ =
                                stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                            // The content is discarded; only the count
                            // matters. Reading it fully keeps the client
                            // from blocking on a half-read write.
                            let _ = read_request(&mut stream);
                            hits_thread.fetch_add(1, Ordering::Relaxed);
                            let _ = stream.write_all(response.as_bytes());
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            });
            CountingServer {
                port,
                hits,
                stop,
                handle: Some(handle),
            }
        }

        fn url(&self) -> String {
            format!("http://127.0.0.1:{}/test", self.port)
        }

        fn hits(&self) -> usize {
            self.hits.load(Ordering::Relaxed)
        }
    }

    impl Drop for CountingServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            // Unblock the accept loop, then join.
            let _ = std::net::TcpStream::connect(format!("127.0.0.1:{}", self.port));
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    /// A recorded request with the Authorization header value
    /// redacted, for assert messages: the raw request carries the
    /// fixture token, which must not be printed on failure.
    fn redact_authorization(request: &str) -> String {
        request
            .lines()
            .map(|l| {
                if l.to_ascii_lowercase().starts_with("authorization:") {
                    "Authorization: [redacted]"
                } else {
                    l
                }
            })
            .collect::<Vec<_>>()
            .join("\r\n")
    }

    #[test]
    fn post_json_does_not_retry_transient_failures() {
        // A 429 or a 500 must surface as ServerError after exactly
        // one attempt: post_json is documented single-attempt, and a
        // retried POST could create the resource twice.
        let client = UreqClient::with_auth(Timeout::default(), None).unwrap();
        for status in [429, 500] {
            let server = CountingServer::serve_status("error", status);
            let err = client.post_json(&server.url(), "{}").unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ServerError, "status {status}");
            assert_eq!(server.hits(), 1, "status {status}: exactly one attempt");
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
    fn get_requests_xml_from_the_api() {
        let server = LoopbackServer::serve("hello");
        let client = UreqClient::new(Timeout::default()).unwrap();
        assert_eq!(client.get(&server.url()).unwrap(), "hello");
        let request = server.request().expect("a request arrived");
        let headers = match request.split_once("\r\n\r\n") {
            Some((headers, _)) => headers,
            None => &request,
        };
        let accept = headers
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("accept:"))
            .map(|l| l["accept:".len()..].trim().to_string());
        assert_eq!(accept.as_deref(), Some("application/xml"));
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
            "POST method line wrong; request: {}",
            redact_authorization(&request)
        );
        assert!(
            request
                .lines()
                .any(|l| l.eq_ignore_ascii_case("content-type: application/json")),
            "POST must carry Content-Type: application/json; request: {}",
            redact_authorization(&request)
        );
        let (_, sent_body) = request.split_once("\r\n\r\n").expect("request has a body");
        assert_eq!(sent_body, body);
        assert!(
            request
                .lines()
                .any(|l| l.eq_ignore_ascii_case("authorization: Bearer tok-123")),
            "POST must carry the Bearer <redacted> request: {}",
            redact_authorization(&request)
        );
    }

    #[test]
    fn post_json_with_sends_the_explicit_credentials_not_the_seam() {
        // The split-brain case: the HTTP client was built with one
        // provider (the seam token) while the caller passes another.
        // The wire must carry the explicit credentials — a Gitea
        // fork authenticated as the seam token would fork as the
        // wrong account while every read/delete went as the caller.
        let server = LoopbackServer::serve("created");
        let client = UreqClient::with_auth(
            Timeout::default(),
            Some(Box::new(FixedAuth(Credentials::Bearer {
                token: "seam-token".to_string(),
            }))),
        )
        .unwrap();
        let body = "{\"name\":\"geectl-live\"}";
        let explicit = Credentials::Bearer {
            token: "explicit-token".to_string(),
        };
        assert_eq!(
            client
                .post_json_with(&server.url(), body, &explicit)
                .unwrap(),
            "created"
        );
        let request = server.request().expect("server recorded the request");
        assert!(
            request
                .lines()
                .any(|l| l.eq_ignore_ascii_case("authorization: Bearer explicit-token")),
            "POST must carry the explicit Bearer <redacted>, not the seam one; request: {}",
            redact_authorization(&request)
        );
        assert!(
            !request.contains("seam-token"),
            "the seam token must not reach the wire; request: {}",
            redact_authorization(&request)
        );
    }

    #[test]
    fn post_json_with_maps_409_to_already_exists() {
        let server = LoopbackServer::serve_status("already exists", 409);
        let client = UreqClient::with_auth(Timeout::default(), None).unwrap();
        let err = client
            .post_json_with(&server.url(), "{}", &Credentials::None)
            .expect_err("a 409 POST must fail");
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
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
        assert!(
            first.starts_with("GET /test HTTP/1.1\r\n"),
            "redirector saw a different request; request: {}",
            redact_authorization(&first)
        );
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
        // A bracketed host counts only when the closing bracket is
        // followed by end of authority or a `:port`: `http://[::1]evil.com`
        // is not loopback, while genuine `[::1]` with or without a port is.
        assert!(!is_loopback_url("http://[::1]evil.com"));
        assert!(!is_loopback_url("http://[::1]evil.com:3000"));
        assert!(!is_loopback_url("http://[::1"));
        assert!(is_loopback_url("http://[::1]:8080/x"));
        assert!(is_loopback_url("http://[::1]/x"));
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

    // --- POST tests (OBS submit-request filing) ---

    /// A one-shot loopback server that records the request it receives:
    /// method, Content-Type and Authorization headers, and the body.
    /// Answers 200 with `response`. Loopback only, fixture
    /// credentials only, never a real endpoint and never a real
    /// credential.
    ///
    /// The captured request: (method, Content-Type, Authorization, body).
    type CapturedRequest = (String, Option<String>, Option<String>, String);

    struct RequestCapture {
        port: u16,
        handle: std::thread::JoinHandle<Option<CapturedRequest>>,
    }

    impl RequestCapture {
        fn serve(response: &'static str) -> Self {
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
                // On macOS/BSD, the accepted stream inherits the
                // listener's non-blocking mode: without this
                // the read below returns WouldBlock at once
                // (and the read timeout is a no-op), so the
                // server answers a request it never read.
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            // The body follows the header terminator;
                            // stop once the declared Content-Length
                            // bytes have arrived.
                            if let Some(headers_end) = find_subsequence(&buf, b"\r\n\r\n") {
                                let headers =
                                    String::from_utf8_lossy(&buf[..headers_end]).to_string();
                                let content_length = headers
                                    .lines()
                                    .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                                    .and_then(|l| {
                                        l["content-length:".len()..].trim().parse::<usize>().ok()
                                    })
                                    .unwrap_or(0);
                                if buf.len() >= headers_end + 4 + content_length {
                                    break;
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let (head, body) = match text.split_once("\r\n\r\n") {
                    Some((head, body)) => (head.to_string(), body.to_string()),
                    None => (text.clone(), String::new()),
                };
                let method = head
                    .lines()
                    .next()
                    .unwrap_or("")
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string();
                let header = |name: &str| {
                    head.lines()
                        .find(|l| {
                            l.to_ascii_lowercase()
                                .starts_with(&name.to_ascii_lowercase())
                        })
                        .map(|l| l[name.len() + 1..].trim().to_string())
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
                    response.len(),
                    response
                );
                let _ = stream.write_all(response.as_bytes());
                Some((
                    method,
                    header("Content-Type:"),
                    header("Authorization:"),
                    body,
                ))
            });
            RequestCapture { port, handle }
        }

        fn url(&self) -> String {
            format!("http://127.0.0.1:{}/request?cmd=create", self.port)
        }

        fn url_for(&self, path: &str) -> String {
            format!("http://127.0.0.1:{}{}", self.port, path)
        }

        fn captured(self) -> CapturedRequest {
            self.handle
                .join()
                .expect("server thread")
                .expect("a request arrived")
        }
    }

    fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    #[test]
    fn post_sends_body_content_type_and_auth() {
        let server = RequestCapture::serve("created");
        let client = UreqClient::with_auth(
            Timeout::default(),
            Some(Box::new(FixedAuth(Credentials::Basic {
                username: "alice".to_string(),
                password: "s3cr3t".to_string(),
            }))),
        )
        .unwrap();
        let body = "<request><action type=\"submit\"/></request>";
        assert_eq!(
            client
                .post(
                    &server.url(),
                    body,
                    &Credentials::Basic {
                        username: "alice".to_string(),
                        password: "s3cr3t".to_string(),
                    }
                )
                .unwrap(),
            "created"
        );
        let (method, content_type, auth, received_body) = server.captured();
        assert_eq!(method, "POST");
        assert_eq!(content_type.as_deref(), Some("application/xml"));
        use base64::Engine;
        let expected = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("alice:s3cr3t")
        );
        assert_eq!(auth.as_deref(), Some(expected.as_str()));
        assert_eq!(received_body, body);
    }

    /// A loopback server that answers one request with `body` as raw
    /// bytes, then exits. Returns the port and the server thread. The
    /// accept loop is nonblocking with a 5s deadline, so a test that
    /// never connects does not hang the suite.
    fn serve_bytes_once(body: Vec<u8>) -> (u16, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback server");
        listener.set_nonblocking(true).expect("nonblocking");
        let port = listener.local_addr().expect("local addr").port();
        let handle = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // On macOS/BSD, the accepted stream inherits the
                        // listener's non-blocking mode: without this
                        // the read below returns WouldBlock at once
                        // (and the read timeout is a no-op), so the
                        // server answers a request it never read.
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                        let mut buf = [0u8; 4096];
                        let _ = stream.read(&mut buf);
                        let mut response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        response.extend_from_slice(&body);
                        let _ = stream.write_all(&response);
                        return;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() > deadline {
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(_) => return,
                }
            }
        });
        (port, handle)
    }

    #[test]
    fn get_bytes_returns_raw_bytes_that_get_with_refuses() {
        // RPM magic plus non-UTF-8 bytes: a binary body.
        let bytes: Vec<u8> = vec![0xED, 0xAB, 0xEE, 0xDB, 0x03, 0x00, 0x00, 0x00, 0xFF, 0xFE];
        let client = UreqClient::new(Timeout::default()).unwrap();
        let (port, server) = serve_bytes_once(bytes.clone());
        let url = format!("http://127.0.0.1:{port}/pkg.rpm");
        let got = client.get_bytes(&url, &Credentials::None).unwrap();
        assert_eq!(got, bytes);
        server.join().expect("server thread");
        // The same bytes through get_with are a server error, not a
        // mojibake string: a text read must not silently corrupt a
        // binary body.
        let (port, server) = serve_bytes_once(bytes.clone());
        let url = format!("http://127.0.0.1:{port}/pkg.rpm");
        let err = client.get_with(&url, &Credentials::None).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        server.join().expect("server thread");
    }

    /// A loopback server that answers 200 with `body` once and
    /// captures the request head (method line + headers) it received.
    /// Returns the port, the server thread, and the captured head.
    fn serve_bytes_once_capture_request(
        body: Vec<u8>,
    ) -> (
        u16,
        std::thread::JoinHandle<()>,
        std::sync::Arc<std::sync::Mutex<Option<String>>>,
    ) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback server");
        listener.set_nonblocking(true).expect("nonblocking");
        let port = listener.local_addr().expect("local addr").port();
        let captured: std::sync::Arc<std::sync::Mutex<Option<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        let captured_clone = std::sync::Arc::clone(&captured);
        let handle = std::thread::spawn(move || {
            use std::io::Write;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // On macOS/BSD, the accepted stream inherits the
                        // listener's non-blocking mode: without this
                        // the read below returns WouldBlock at once
                        // (and the read timeout is a no-op), so the
                        // server answers a request it never read.
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                        // Read until the full request head has
                        // arrived: one read can return a partial head,
                        // or nothing at all.
                        // read_request loops until the header/body
                        // boundary, plus any Content-Length body; a
                        // GET has no body, so this is exactly the head.
                        let buf = read_request(&mut stream);
                        let head = String::from_utf8_lossy(&buf).into_owned();
                        *captured_clone.lock().expect("capture lock") = Some(head);
                        let mut response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        response.extend_from_slice(&body);
                        let _ = stream.write_all(&response);
                        return;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() > deadline {
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(_) => return,
                }
            }
        });
        (port, handle, captured)
    }

    #[test]
    fn get_bytes_sends_no_accept_xml_header() {
        // A binary download must not ask for XML: the bytes are a
        // file, not a document. The text path still asks for it.
        let bytes: Vec<u8> = vec![0xED, 0xAB, 0xEE, 0xDB];
        let client = UreqClient::new(Timeout::default()).unwrap();
        let (port, server, request) = serve_bytes_once_capture_request(bytes.clone());
        let url = format!("http://127.0.0.1:{port}/pkg.rpm");
        let got = client.get_bytes(&url, &Credentials::None).unwrap();
        assert_eq!(got, bytes);
        server.join().expect("server thread");
        let head = request
            .lock()
            .expect("request lock")
            .clone()
            .expect("request seen");
        assert!(
            !head.contains("accept: application/xml"),
            "binary download asked for XML: {head}"
        );
        let (port, server, request) = serve_bytes_once_capture_request(b"<collection/>".to_vec());
        let url = format!("http://127.0.0.1:{port}/collection");
        let _ = client.get_with(&url, &Credentials::None).unwrap();
        server.join().expect("server thread");
        let head = request
            .lock()
            .expect("request lock")
            .clone()
            .expect("request seen");
        assert!(
            head.contains("accept: application/xml"),
            "text read did not ask for XML: {head}"
        );
    }

    /// A loopback server that counts requests and answers 500 to each.
    /// It stays up 1.5s past the last request (longer than the 1s
    /// first-retry backoff), so a retry would be observed as a second
    /// request. Returns the port, the count, and the server thread.
    fn serve_counting_500() -> (
        u16,
        std::sync::Arc<std::sync::atomic::AtomicU32>,
        std::thread::JoinHandle<()>,
    ) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback server");
        listener.set_nonblocking(true).expect("nonblocking");
        let port = listener.local_addr().expect("local addr").port();
        let count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let count_clone = count.clone();
        let handle = std::thread::spawn(move || {
            use std::io::Write;
            let mut last_request: Option<std::time::Instant> = None;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        count_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        last_request = Some(std::time::Instant::now());
                        // On macOS/BSD, the accepted stream inherits the
                        // listener's non-blocking mode: without this
                        // the read below returns WouldBlock at once
                        // (and the read timeout is a no-op), so the
                        // server answers a request it never read.
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                        // Read the whole request (headers + body):
                        // a single read can return only the headers
                        // when the body arrives in a later segment,
                        // and answering before the body is read resets
                        // the client's body write, which the client
                        // reads as a transport failure and retries.
                        let _ = read_request(&mut stream);
                        let _ = stream.write_all(
                            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if let Some(last) = last_request {
                            if last.elapsed() > std::time::Duration::from_millis(1500) {
                                break;
                            }
                        }
                        if std::time::Instant::now() > deadline {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        (port, count, handle)
    }

    /// Answer 500 to the first request, then 200: the server a
    /// retrying client must recover from. Like `serve_counting_500`
    /// but with a success after the failure, so a test can assert the
    /// client retried (it got the 200 body) instead of counting
    /// attempts against a server that never succeeds.
    fn serve_500_then_ok() -> (
        u16,
        std::sync::Arc<std::sync::atomic::AtomicU32>,
        std::thread::JoinHandle<()>,
    ) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback server");
        listener.set_nonblocking(true).expect("nonblocking");
        let port = listener.local_addr().expect("local addr").port();
        let count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let count_clone = count.clone();
        let handle = std::thread::spawn(move || {
            use std::io::Write;
            let mut last_request: Option<std::time::Instant> = None;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let n = count_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        last_request = Some(std::time::Instant::now());
                        // On macOS/BSD, the accepted stream inherits the
                        // listener's non-blocking mode: without this
                        // the read below returns WouldBlock at once
                        // (and the read timeout is a no-op), so the
                        // server answers a request it never read.
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                        // Read the whole request (headers + body):
                        // a single read can return only the headers
                        // when the body arrives in a later segment,
                        // and answering before the body is read resets
                        // the client's body write, which the client
                        // reads as a transport failure and retries.
                        let _ = read_request(&mut stream);
                        // First request fails; the retry succeeds.
                        let response = if n == 0 {
                            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
                        } else {
                            let body = "<status code=\"ok\" />";
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            )
                        };
                        let _ = stream.write_all(response.as_bytes());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if let Some(last) = last_request {
                            // 3s, not the 1.5s `serve_counting_500`
                            // uses: the client sleeps 1s before its
                            // retry, and the server timestamps the
                            // request before the client reads the 500,
                            // so 1.5s leaves only ~0.5s of margin.
                            if last.elapsed() > std::time::Duration::from_millis(3000) {
                                break;
                            }
                        }
                        if std::time::Instant::now() > deadline {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        (port, count, handle)
    }

    #[test]
    fn post_is_never_retried() {
        // A 500 must come back as a ServerError after exactly ONE
        // attempt: a retried POST could file a duplicate submit
        // request.
        let (port, count, handle) = serve_counting_500();
        let client = UreqClient::new(Timeout::default()).unwrap();
        let url = format!("http://127.0.0.1:{port}/request?cmd=create");
        let err = client
            .post(&url, "<request/>", &Credentials::None)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        handle.join().expect("server thread");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn put_sends_xml_body() {
        let server = LoopbackServer::serve("<status code=\"ok\" />");
        let client = UreqClient::new(Timeout::default()).unwrap();
        let body =
            "<project name=\"home:alice\"><title>t</title><description>d</description></project>";
        assert_eq!(
            client.put(&server.url(), body, &Credentials::None).unwrap(),
            "<status code=\"ok\" />"
        );
        let request = server.request().expect("server recorded the request");
        // Method, content type, and body — exactly what a `_meta` PUT needs.
        assert!(
            request.starts_with("PUT /test HTTP/1.1\r\n"),
            "PUT method line wrong; request: {request}"
        );
        assert!(
            request
                .lines()
                .any(|l| l.eq_ignore_ascii_case("content-type: application/xml")),
            "PUT must carry Content-Type: application/xml; request: {request}"
        );
        let sent_body = request
            .split("\r\n\r\n")
            .nth(1)
            .expect("request has a body");
        assert_eq!(sent_body, body);
    }

    #[test]
    fn put_json_with_sends_json_body() {
        let server = LoopbackServer::serve("");
        let client = UreqClient::with_auth(
            Timeout::default(),
            Some(Box::new(FixedAuth(Credentials::Bearer {
                token: "seam-token".to_string(),
            }))),
        )
        .unwrap();
        let body = "{\"permission\":\"write\"}";
        let explicit = Credentials::Bearer {
            token: "explicit-token".to_string(),
        };
        assert_eq!(
            client
                .put_json_with(&server.url(), body, &explicit)
                .unwrap(),
            ""
        );
        let request = server.request().expect("server recorded the request");
        // Method, content type, body, and auth — exactly what a
        // Gitea collaborator PUT needs: the explicit Bearer, not
        // the seam one.
        assert!(
            request.starts_with("PUT /test HTTP/1.1\r\n"),
            "PUT method line wrong; request: {}",
            redact_authorization(&request)
        );
        assert!(
            request
                .lines()
                .any(|l| l.eq_ignore_ascii_case("content-type: application/json")),
            "PUT must carry Content-Type: application/json; request: {}",
            redact_authorization(&request)
        );
        let (_, sent_body) = request.split_once("\r\n\r\n").expect("request has a body");
        assert_eq!(sent_body, body);
        assert!(
            request
                .lines()
                .any(|l| l.eq_ignore_ascii_case("authorization: Bearer explicit-token")),
            "PUT must carry the explicit Bearer, not the seam one; request: {}",
            redact_authorization(&request)
        );
        assert!(
            !request.contains("seam-token"),
            "the seam token must not reach the wire; request: {}",
            redact_authorization(&request)
        );
    }

    #[test]
    fn put_file_sends_octet_stream_body() {
        let server = LoopbackServer::serve("<status code=\"ok\" />");
        let client = UreqClient::new(Timeout::default()).unwrap();
        let body = "Name: foo\nVersion: 1.0\n";
        assert_eq!(
            client
                .put_file(&server.url(), body, &Credentials::None)
                .unwrap(),
            "<status code=\"ok\" />"
        );
        let request = server.request().expect("server recorded the request");
        // Method, content type, and body — exactly what a file PUT needs.
        assert!(
            request.starts_with("PUT /test HTTP/1.1\r\n"),
            "PUT method line wrong; request: {request}"
        );
        assert!(
            request
                .lines()
                .any(|l| l.eq_ignore_ascii_case("content-type: application/octet-stream")),
            "PUT file must carry Content-Type: application/octet-stream; request: {request}"
        );
        let sent_body = request
            .split("\r\n\r\n")
            .nth(1)
            .expect("request has a body");
        assert_eq!(sent_body, body);
    }

    #[test]
    fn put_retries_after_500() {
        // A 500 on the first attempt must be retried, and the retry
        // must return the 200 body: PUT is idempotent (re-PUTting the
        // same `_meta` returns the same 200), so unlike POST a retry
        // cannot duplicate anything. The counting 500 server cannot
        // serve this: its idle break (1.5s) is shorter than the
        // backoff between the second and third attempts, so it exits
        // mid-retry — hence the 500-then-200 server below.
        let (port, count, handle) = serve_500_then_ok();
        let client = UreqClient::new(Timeout::default()).unwrap();
        let url = format!("http://127.0.0.1:{port}/source/home:alice/_meta");
        assert_eq!(
            client.put(&url, "<project/>", &Credentials::None).unwrap(),
            "<status code=\"ok\" />"
        );
        handle.join().expect("server thread");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn put_file_retries_after_500() {
        // A 500 on the first attempt must be retried, and the retry
        // must return the 200 body: PUT is idempotent (re-PUTting the
        // same file stores the same bytes), so unlike POST a retry
        // cannot duplicate anything. The retry records a second
        // revision with identical content on a real server; the
        // counting server here only proves the retry happens.
        let (port, count, handle) = serve_500_then_ok();
        let client = UreqClient::new(Timeout::default()).unwrap();
        let url = format!("http://127.0.0.1:{port}/source/home:alice/foo/foo.spec");
        assert_eq!(
            client
                .put_file(&url, "Name: foo\n", &Credentials::None)
                .unwrap(),
            "<status code=\"ok\" />"
        );
        handle.join().expect("server thread");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn put_bytes_sends_octet_stream_body_verbatim() {
        let server = LoopbackServer::serve("<revision>1</revision>");
        let client = UreqClient::new(Timeout::default()).unwrap();
        // Bytes a text body could not carry in the general case — a
        // NUL byte and a multi-byte UTF-8 sequence — but still valid
        // UTF-8, so the loopback server's lossy recording preserves
        // them exactly (a non-UTF-8 byte flow is pinned at the
        // FakeHttp level, where the recorded body is a Vec<u8>, not
        // a String).
        let body: &[u8] = b"PK\x03\x04\x00binary\xc3\xa9";
        assert_eq!(
            client
                .put_bytes(&server.url(), body, &Credentials::None)
                .unwrap(),
            "<revision>1</revision>"
        );
        let request = server.request().expect("server recorded the request");
        assert!(
            request.starts_with("PUT /test HTTP/1.1\r\n"),
            "PUT method line wrong; request: {request}"
        );
        assert!(
            request
                .lines()
                .any(|l| l.eq_ignore_ascii_case("content-type: application/octet-stream")),
            "PUT bytes must carry Content-Type: application/octet-stream; request: {request}"
        );
        let sent_body = request
            .split("\r\n\r\n")
            .nth(1)
            .expect("request has a body");
        assert_eq!(sent_body.as_bytes(), body);
    }

    #[test]
    fn put_bytes_retries_after_500() {
        // A 500 on the first attempt must be retried, and the retry
        // must return the 200 body: PUT is idempotent (re-PUTting the
        // same file stores the same bytes), so like `put_file` a
        // transient failure cannot corrupt anything. The retry
        // records a second revision with identical content on a real
        // server; the counting server here only proves the retry
        // happens.
        let (port, count, handle) = serve_500_then_ok();
        let client = UreqClient::new(Timeout::default()).unwrap();
        let url = format!("http://127.0.0.1:{port}/source/home:alice/foo/foo.bin");
        assert_eq!(
            client
                .put_bytes(&url, b"PK\x03\x04", &Credentials::None)
                .unwrap(),
            "<status code=\"ok\" />"
        );
        handle.join().expect("server thread");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn post_over_non_loopback_http_is_refused() {
        // Cleartext http to a real host must never carry a credential
        // — and a POST body must not reach it either. The refusal
        // happens before connecting.
        let client = UreqClient::new(Timeout::default()).unwrap();
        let err = client
            .post(
                "http://example.com/request?cmd=create",
                "<request/>",
                &Credentials::Basic {
                    username: "alice".to_string(),
                    password: "s3cr3t".to_string(),
                },
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn post_text_sends_plain_text_content_type() {
        // The changestate comment is plain text, not XML: the server
        // reads the raw body as the comment, so the header must say
        // what the body is.
        let server = RequestCapture::serve("<status code=\"ok\" />");
        let client = UreqClient::new(Timeout::default()).unwrap();
        let url = server
            .url_for("/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1397897");
        assert_eq!(
            client
                .post_text(
                    &url,
                    "superseded by 1397897",
                    &Credentials::Basic {
                        username: "alice".to_string(),
                        password: "s3cr3t".to_string(),
                    }
                )
                .unwrap(),
            "<status code=\"ok\" />"
        );
        let (method, content_type, auth, received_body) = server.captured();
        assert_eq!(method, "POST");
        assert_eq!(content_type.as_deref(), Some("text/plain"));
        assert!(auth.is_some());
        assert_eq!(received_body, "superseded by 1397897");
    }

    // --- DELETE tests (branch-cleanup mutation) ---

    #[test]
    fn delete_sends_auth_and_returns_body() {
        let server = RequestCapture::serve("<status code=\"ok\" />");
        let client = UreqClient::new(Timeout::default()).unwrap();
        let url = server.url_for("/source/home:scarabeus_iv/foo");
        assert_eq!(
            client
                .delete(
                    &url,
                    &Credentials::Basic {
                        username: "alice".to_string(),
                        password: "s3cr3t".to_string(),
                    }
                )
                .unwrap(),
            "<status code=\"ok\" />"
        );
        let (method, content_type, auth, received_body) = server.captured();
        assert_eq!(method, "DELETE");
        // A DELETE carries no body, so no Content-Type is sent.
        assert_eq!(content_type, None);
        use base64::Engine;
        let expected = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("alice:s3cr3t")
        );
        assert_eq!(auth.as_deref(), Some(expected.as_str()));
        assert_eq!(received_body, "");
    }

    #[test]
    fn mutations_request_xml_from_the_api() {
        // Every mutation asks for XML, the same defence as GET: the owner
        // search proved a dual-routed path serves the WebUI HTML page
        // unless the request asks for XML. Mutations ask for XML too,
        // as a preventive measure.
        let client = UreqClient::new(Timeout::default()).unwrap();
        let accept_of = |request: &str| {
            let headers = match request.split_once("\r\n\r\n") {
                Some((headers, _)) => headers,
                None => request,
            };
            headers
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("accept:"))
                .map(|l| l["accept:".len()..].trim().to_string())
        };

        let server = LoopbackServer::serve("created");
        assert_eq!(
            client
                .post(&server.url(), "<request/>", &Credentials::None)
                .unwrap(),
            "created"
        );
        let recorded = server.request().expect("a request arrived");
        assert_eq!(
            accept_of(&recorded).as_deref(),
            Some("application/xml"),
            "mutation must ask for XML; recorded request: {recorded}"
        );

        let server = LoopbackServer::serve("<status code=\"ok\" />");
        assert_eq!(
            client
                .put(&server.url(), "<project/>", &Credentials::None)
                .unwrap(),
            "<status code=\"ok\" />"
        );
        let recorded = server.request().expect("a request arrived");
        assert_eq!(
            accept_of(&recorded).as_deref(),
            Some("application/xml"),
            "mutation must ask for XML; recorded request: {recorded}"
        );

        let server = LoopbackServer::serve("<status code=\"ok\" />");
        assert_eq!(
            client.delete(&server.url(), &Credentials::None).unwrap(),
            "<status code=\"ok\" />"
        );
        let recorded = server.request().expect("a request arrived");
        assert_eq!(
            accept_of(&recorded).as_deref(),
            Some("application/xml"),
            "mutation must ask for XML; recorded request: {recorded}"
        );

        let server = LoopbackServer::serve("created");
        assert_eq!(
            client
                .post_text(&server.url(), "superseded by 42", &Credentials::None)
                .unwrap(),
            "created"
        );
        let recorded = server.request().expect("a request arrived");
        assert_eq!(
            accept_of(&recorded).as_deref(),
            Some("application/xml"),
            "mutation must ask for XML; recorded request: {recorded}"
        );

        let server = LoopbackServer::serve("<status code=\"ok\" />");
        assert_eq!(
            client
                .put_file(
                    &server.url(),
                    "Name: foo\nVersion: 1.0\n",
                    &Credentials::None
                )
                .unwrap(),
            "<status code=\"ok\" />"
        );
        let recorded = server.request().expect("a request arrived");
        assert_eq!(
            accept_of(&recorded).as_deref(),
            Some("application/xml"),
            "mutation must ask for XML; recorded request: {recorded}"
        );

        let server = LoopbackServer::serve("created");
        assert_eq!(client.post_json(&server.url(), "{}").unwrap(), "created");
        let recorded = server.request().expect("a request arrived");
        assert_eq!(
            accept_of(&recorded).as_deref(),
            Some("application/xml"),
            "mutation must ask for XML; recorded request: {recorded}"
        );
    }

    #[test]
    fn delete_is_never_retried() {
        // A 500 must come back as a ServerError after exactly ONE
        // attempt: a retried DELETE cannot tell "the first attempt
        // never ran" from "the first attempt deleted it and the
        // response was lost", so a second attempt would 404 and read
        // a successful delete as a failure.
        let (port, count, handle) = serve_counting_500();
        let client = UreqClient::new(Timeout::default()).unwrap();
        let url = format!("http://127.0.0.1:{port}/source/home:scarabeus_iv/foo");
        let err = client.delete(&url, &Credentials::None).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        handle.join().expect("server thread");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn delete_over_non_loopback_http_is_refused() {
        // Cleartext http to a real host must never carry a credential.
        // The refusal happens before connecting.
        let client = UreqClient::new(Timeout::default()).unwrap();
        let err = client
            .delete(
                "http://example.com/source/home:scarabeus_iv/foo",
                &Credentials::Basic {
                    username: "alice".to_string(),
                    password: "s3cr3t".to_string(),
                },
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    // --- Debug channel tests (fixture transports, never a live server) ---

    /// The lines a captured sink recorded, as owned strings.
    fn take_debug_lines(lines: &std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> Vec<String> {
        lines.lock().unwrap().clone()
    }

    /// Debug lines with the per-attempt duration stripped: durations
    /// are real timing, so an exact-line assertion must not depend
    /// on them.
    fn debug_lines_without_durations(lines: &[String]) -> Vec<String> {
        lines
            .iter()
            .map(|line| match (line.find(" in "), line.find(" (attempt")) {
                (Some(start), Some(end)) if start < end => {
                    format!("{}{}", &line[..start], &line[end..])
                }
                _ => line.clone(),
            })
            .collect()
    }

    #[test]
    fn debug_off_records_no_lines() {
        let server = LoopbackServer::serve("hello");
        let (debug, lines) = Debug::captured(false, false);
        let client = UreqClient::new(Timeout::default())
            .unwrap()
            .with_debug(debug);
        assert_eq!(client.get(&server.url()).unwrap(), "hello");
        assert!(take_debug_lines(&lines).is_empty());
    }

    #[test]
    fn debug_on_records_attempt_lines_for_500_then_200() {
        let (port, count, handle) = serve_500_then_ok();
        let (debug, lines) = Debug::captured(true, false);
        let client = UreqClient::new(Timeout::default())
            .unwrap()
            .with_debug(debug);
        let url = format!("http://127.0.0.1:{port}/public/source/openSUSE:Factory/foo");
        assert_eq!(client.get(&url).unwrap(), "<status code=\"ok\" />");
        handle.join().expect("server thread");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(
            debug_lines_without_durations(&take_debug_lines(&lines)),
            vec![
                "decision: read via /public/source (no credentials)".to_string(),
                "http: GET /public/source/openSUSE:Factory/foo -> 500 (attempt 1/4, retry in 1s: transient status 500)"
                    .to_string(),
                "http: GET /public/source/openSUSE:Factory/foo -> 200 (attempt 2/4)".to_string(),
            ]
        );
    }

    #[test]
    fn debug_on_records_one_line_for_404() {
        // A 404 is not retryable: one attempt line, no retry note,
        // no exhaustion line.
        let server = LoopbackServer::serve_status("missing", 404);
        let (debug, lines) = Debug::captured(true, false);
        let client = UreqClient::new(Timeout::default())
            .unwrap()
            .with_debug(debug);
        let url = format!(
            "http://127.0.0.1:{}/public/source/openSUSE:Factory/foo",
            server.port
        );
        let err = client.get(&url).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert_eq!(
            debug_lines_without_durations(&take_debug_lines(&lines)),
            vec![
                "decision: read via /public/source (no credentials)".to_string(),
                "http: GET /public/source/openSUSE:Factory/foo -> 404 (attempt 1/4)".to_string(),
            ]
        );
    }

    #[test]
    fn debug_on_records_exhaustion_after_retries() {
        // Four attempts (1s + 2s + 4s of backoff), then the
        // exhaustion line naming the last status.
        let server = CountingServer::serve_status("error", 500);
        let (debug, lines) = Debug::captured(true, false);
        let client = UreqClient::new(Timeout::default())
            .unwrap()
            .with_debug(debug);
        let url = format!(
            "http://127.0.0.1:{}/public/source/openSUSE:Factory/foo",
            server.port
        );
        let err = client.get(&url).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert_eq!(server.hits(), 4);
        assert_eq!(
            debug_lines_without_durations(&take_debug_lines(&lines)),
            vec![
                "decision: read via /public/source (no credentials)".to_string(),
                "http: GET /public/source/openSUSE:Factory/foo -> 500 (attempt 1/4, retry in 1s: transient status 500)"
                    .to_string(),
                "http: GET /public/source/openSUSE:Factory/foo -> 500 (attempt 2/4, retry in 2s: transient status 500)"
                    .to_string(),
                "http: GET /public/source/openSUSE:Factory/foo -> 500 (attempt 3/4, retry in 4s: transient status 500)"
                    .to_string(),
                "http: GET /public/source/openSUSE:Factory/foo -> 500 (attempt 4/4)".to_string(),
                "http: GET /public/source/openSUSE:Factory/foo failed after 4 attempts (last: 500)"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn debug_endpoint_reports_authenticated_reads() {
        let server = LoopbackServer::serve("hello");
        let (debug, lines) = Debug::captured(true, false);
        let client = UreqClient::with_auth(
            Timeout::default(),
            Some(Box::new(FixedAuth(Credentials::Basic {
                username: "alice".to_string(),
                password: "s3cr3t".to_string(),
            }))),
        )
        .unwrap()
        .with_debug(debug);
        assert_eq!(client.get(&server.url()).unwrap(), "hello");
        let lines = take_debug_lines(&lines);
        assert!(
            lines
                .iter()
                .any(|l| l == "decision: read via /test (authenticated)"),
            "lines: {lines:?}"
        );
        // The fixture credential never reaches a debug line.
        assert!(!lines.iter().any(|l| l.contains("s3cr3t")));
    }

    #[test]
    fn debug_body_absent_by_default() {
        // Default debug never prints bodies: the response body (and
        // the fake credential in it) stays out of the channel.
        let server = LoopbackServer::serve("<auth password=\"hunter2-fake\" />");
        let (debug, lines) = Debug::captured(true, false);
        let client = UreqClient::new(Timeout::default())
            .unwrap()
            .with_debug(debug);
        assert_eq!(
            client.get(&server.url()).unwrap(),
            "<auth password=\"hunter2-fake\" />"
        );
        let lines = take_debug_lines(&lines);
        assert!(!lines.iter().any(|l| l.contains("body")));
        assert!(!lines.iter().any(|l| l.contains("hunter2-fake")));
    }

    #[test]
    fn debug_body_present_only_behind_debug_body() {
        let server = LoopbackServer::serve("<auth password=\"hunter2-fake\" />");
        let (debug, lines) = Debug::captured(true, true);
        let client = UreqClient::new(Timeout::default())
            .unwrap()
            .with_debug(debug);
        assert_eq!(
            client.get(&server.url()).unwrap(),
            "<auth password=\"hunter2-fake\" />"
        );
        let lines = take_debug_lines(&lines);
        assert!(
            lines.iter().any(|l| l.contains("response body")),
            "lines: {lines:?}"
        );
        // The fake credential appears nowhere; the redaction marker does.
        assert!(!lines.iter().any(|l| l.contains("hunter2-fake")));
        assert!(lines.iter().any(|l| l.contains("[redacted]")));
    }

    #[test]
    fn debug_body_redacts_request_bodies() {
        let server = LoopbackServer::serve("<status code=\"ok\" />");
        let (debug, lines) = Debug::captured(true, true);
        let client = UreqClient::new(Timeout::default())
            .unwrap()
            .with_debug(debug);
        let body = "<request><auth password=\"hunter2-fake\" /></request>";
        assert_eq!(
            client
                .post(&server.url(), body, &Credentials::None)
                .unwrap(),
            "<status code=\"ok\" />"
        );
        let lines = take_debug_lines(&lines);
        assert!(
            lines.iter().any(|l| l.contains("request body")),
            "lines: {lines:?}"
        );
        assert!(!lines.iter().any(|l| l.contains("hunter2-fake")));
    }

    #[test]
    fn debug_body_never_dumps_binary_bodies() {
        let body = format!("ok{}binary", char::from(0));
        let server = LoopbackServer::serve(Box::leak(body.clone().into_boxed_str()));
        let (debug, lines) = Debug::captured(true, true);
        let client = UreqClient::new(Timeout::default())
            .unwrap()
            .with_debug(debug);
        assert_eq!(client.get(&server.url()).unwrap(), body);
        let lines = take_debug_lines(&lines);
        assert!(
            lines.iter().any(|l| l.contains("<binary body, 9 bytes>")),
            "lines: {lines:?}"
        );
    }

    #[test]
    fn debug_body_marks_non_utf8_response_bodies() {
        // A non-UTF-8 response still errors exactly as before
        // (ServerError), but the binary marker line is emitted first,
        // so the transcript shows the body was binary rather than
        // absent.
        let (debug, lines) = Debug::captured(true, true);
        let client = UreqClient::new(Timeout::default())
            .unwrap()
            .with_debug(debug);
        let err = client
            .read_response_body(
                std::io::Cursor::new(vec![0xFF; 4]),
                "GET",
                "https://api.opensuse.org/source/foo",
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        let lines = take_debug_lines(&lines);
        assert!(
            lines
                .iter()
                .any(|l| l
                    == "http: GET /source/foo response body (4 bytes): <binary body, 4 bytes>"),
            "lines: {lines:?}"
        );
    }

    #[test]
    fn debug_attempt_line_precedes_non_utf8_response_body_error() {
        // F-H probe: a 200 whose body is not valid UTF-8 errors
        // (ServerError), but the attempt line must come before the
        // binary marker, so the transcript shows which attempt the
        // marker belongs to. The caller emits the attempt line
        // before the body read: on a read error the marker is the
        // only body line and follows its attempt line.
        let mut response = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n"
            .to_vec();
        response.extend_from_slice(&[0xFF; 4]);
        let server = LoopbackServer::serve_raw(response);
        let (debug, lines) = Debug::captured(true, true);
        let client = UreqClient::new(Timeout::default())
            .unwrap()
            .with_debug(debug);
        let err = client.get(&server.url()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert_eq!(
            debug_lines_without_durations(&take_debug_lines(&lines)),
            vec![
                "decision: read via /test (no credentials)".to_string(),
                "http: GET /test -> 200 (attempt 1/4)".to_string(),
                "http: GET /test response body (4 bytes): <binary body, 4 bytes>".to_string(),
            ]
        );
    }

    #[test]
    fn debug_body_marker_absent_without_debug_body() {
        // The marker is a body line: without `--debug-body` there is
        // no marker, and the error is still the same ServerError.
        let (debug, lines) = Debug::captured(true, false);
        let client = UreqClient::new(Timeout::default())
            .unwrap()
            .with_debug(debug);
        let err = client
            .read_response_body(
                std::io::Cursor::new(vec![0xFF; 4]),
                "GET",
                "https://api.opensuse.org/source/foo",
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(take_debug_lines(&lines).is_empty());
    }
}
