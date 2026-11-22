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
//! Proxy: the client honours the process proxy environment.
//! `ALL_PROXY`, `all_proxy`, `HTTPS_PROXY`, `https_proxy`,
//! `HTTP_PROXY`, `http_proxy` are tried in that order; the first
//! set variable with a valid URI wins, and one proxy serves all
//! requests. A malformed URI is skipped in favour of the next
//! variable; no proxy only when every set variable is malformed.
//! `NO_PROXY` / `no_proxy` lists hosts that connect directly: exact hosts,
//! `*.suffix` / `.suffix` matches, `*` for all, trailing-`*` / trailing-dot forms.
//! Entries are matched verbatim: surrounding spaces are not
//! trimmed, so `NO_PROXY=localhost, 127.0.0.1` leaves the second
//! entry unmatched. Loopback targets go through the proxy too
//! unless `NO_PROXY` lists them — a cleartext loopback request
//! with credentials exposes its `Authorization` header to the
//! proxy. Credentials otherwise never travel to the proxy: auth
//! still comes only from the [`AuthProvider`] seam. A proxy URL
//! carrying userinfo is a different matter: ureq sends it to the
//! proxy as `Proxy-Authorization: Basic` on CONNECT. SOCKS proxies
//! (`socks5://`, `socks4://`) are honoured too.
//!
//! Non-2xx statuses are converted to a [`crate::error::GeectlError`] at
//! this layer, before any caller sees the body, so callers never have to
//! remember to check the status.

use crate::auth::{validate_basic, validate_bearer_token, AuthProvider, Credentials};
use crate::debug::Debug;
use crate::error::{ErrorKind, GeectlError};
use std::io::{Read, Write};
use std::path::Path;
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

/// True when a body-read failure may retry within [`RETRY_POLICY`]:
/// a timeout (a stalled transfer) or a transport failure (a severed
/// one). Anything else fails the same way on every attempt — an
/// over-cap body stays over the cap, a disk failure stays a disk
/// failure, an already-existing destination stays existing — so
/// retrying it only repeats the failure.
fn is_retryable_body_error(e: &GeectlError) -> bool {
    matches!(e.kind(), ErrorKind::Timeout | ErrorKind::NetworkError)
}

/// True when an I/O error from a body read is a timeout: a bare
/// `TimedOut`, or a ureq timeout wrapped as the error's source.
/// ureq's body reader converts its `Error::Timeout` into an
/// `io::Error` of kind `Other` carrying the timeout
/// (`Error::into_io`), so the kind alone does not say timeout —
/// the wrapped error is checked.
fn is_read_timeout(e: &std::io::Error) -> bool {
    if e.kind() == std::io::ErrorKind::TimedOut {
        return true;
    }
    e.get_ref()
        .and_then(|inner| inner.downcast_ref::<ureq::Error>())
        .is_some_and(|error| matches!(error, ureq::Error::Timeout(_)))
}

/// Map a body-read I/O error: a timeout is a timeout, anything
/// else a network error — so a stalled transfer is distinguishable
/// from a severed one wherever a body is read.
fn body_read_error(e: std::io::Error) -> GeectlError {
    let kind = if is_read_timeout(&e) {
        ErrorKind::Timeout
    } else {
        ErrorKind::NetworkError
    };
    GeectlError::new(kind, e.to_string())
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
/// `post_json_with`, `put`, `put_json_with`, `put_file`, `delete`,
/// `patch_json_with`) all ask for
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
    /// GET a URL and stream the response body to a file, without the
    /// `MAX_BODY_BYTES` cap. The byte-download counterpart of
    /// [`HttpClient::get_bytes`]: same transport guard, same retry
    /// policy, same timeouts, no `Accept` header — but the body is
    /// copied to disk as it arrives instead of buffered in memory,
    /// so a file larger than the cap downloads instead of failing.
    /// The file is created exclusively: an existing file is an
    /// `AlreadyExists` error, not a truncation. A failure mid-stream
    /// leaves a partial file for the caller to delete, unless a
    /// retry follows: a retried attempt deletes the partial file
    /// first, so the next attempt's exclusive create succeeds.
    /// Returns the bytes written. A failure reading the body is a
    /// network error, except a read timeout, which is a timeout
    /// error; a failure writing the file is an internal error. The
    /// default implementation refuses: a client that only reads text
    /// cannot stream downloads. [`UreqClient`] overrides it.
    ///
    /// Mid-stream failures retry like call failures, within the
    /// same [`RETRY_POLICY`] budget: a body read that times out or
    /// drops re-issues the GET from byte zero (restart, not resume —
    /// the backends are not required to answer Range requests) after
    /// deleting the partial file. The per-attempt total deadline
    /// still bounds each attempt, so a download that needs more
    /// than one deadline of continuous transfer still fails: the
    /// retry recovers a stalled or severed transfer, it does not
    /// stretch the deadline. A read timeout surfaces as `Timeout`,
    /// not `NetworkError`, so a stalled transfer is distinguishable
    /// from a severed one; other read failures stay `NetworkError`.
    /// Non-retryable body failures (an over-cap body, a disk write
    /// failure, an already-existing destination) return at once.
    fn get_to_file(
        &self,
        url: &str,
        credentials: &Credentials,
        path: &Path,
    ) -> Result<u64, GeectlError> {
        let _ = (url, credentials, path);
        Err(GeectlError::new(
            ErrorKind::InternalError,
            "this HTTP client cannot stream downloads to disk",
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
    /// PATCH a JSON body with explicit credentials. The
    /// `post_json_with` counterpart for a partial update: the same
    /// `application/json` body and the credentials from the caller
    /// instead of the client's auth seam — so a client that holds
    /// its own credentials (the Gitea client) never authenticates
    /// a mutation as someone else. Like POST, PATCH is never
    /// retried: a retried mutation can apply twice, so every
    /// non-2xx status (and every transport failure) is returned
    /// immediately.
    fn patch_json_with(
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
    /// DELETE with a JSON body and explicit credentials. The `delete`
    /// counterpart of `post_json_with`: the same `application/json`
    /// body and the credentials from the caller instead of the
    /// client's auth seam — so a client that holds its own Bearer
    /// (the Gitea client) never authenticates a mutation as someone
    /// else. Like `delete` it is never retried: a retried delete
    /// cannot tell a lost response from a delete that never ran.
    fn delete_json_with(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError>;
}

/// A [`HttpClient`] backed by ureq, with bounded timeouts and the retry
/// policy applied to GET. Auth, when provided, is applied from the
/// [`AuthProvider`] seam — the client never reads credentials itself.
pub struct UreqClient {
    agent: ureq::Agent,
    auth: Option<Box<dyn AuthProvider>>,
    debug: Debug,
}

/// Read the proxy from the process environment, if any: see the
/// module docs for the variable order and `no_proxy` matching.
/// A malformed proxy URI is skipped in favour of the next variable
/// (no proxy only when every set variable is malformed), never
/// failing client construction.
fn proxy_from_env() -> Option<ureq::Proxy> {
    ureq::Proxy::try_from_env()
}

impl UreqClient {
    pub fn new(timeout: Timeout) -> Result<Self, GeectlError> {
        UreqClient::with_auth(timeout, None)
    }

    pub fn with_auth(
        timeout: Timeout,
        auth: Option<Box<dyn AuthProvider>>,
    ) -> Result<Self, GeectlError> {
        UreqClient::with_auth_and_proxy(timeout, auth, proxy_from_env())
    }

    /// Build a client with an explicit proxy (or none). `with_auth`
    /// passes the proxy read from the process environment;
    /// callers that need a fixed proxy pass one here, and tests
    /// build one by hand so proxy behaviour is pinned without
    /// mutating process environment variables.
    pub fn with_auth_and_proxy(
        timeout: Timeout,
        auth: Option<Box<dyn AuthProvider>>,
        proxy: Option<ureq::Proxy>,
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
            // Error statuses arrive as responses, not errors: the
            // body of a 4xx/5xx carries the real cause (a 403 from a
            // search that exceeded the server's result limit names
            // the limit; a genuine auth failure does not), and the
            // status mapping needs the body to tell them apart.
            // Every call site maps a non-2xx response explicitly, so
            // no error status is ever read as a success.
            .http_status_as_error(false)
            // Never forward the Authorization header on a redirect.
            // The ureq default is already `Never`, but it is set
            // explicitly so a future ureq default change cannot
            // silently start leaking credentials to the redirect
            // target (which may be a different host entirely).
            .redirect_auth_headers(ureq::config::RedirectAuthHeaders::Never)
            // Honour the process proxy environment (see the module
            // docs): without this, ureq connects directly even
            // behind a proxy.
            .proxy(proxy)
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
    /// a file, not an XML document. `read_body` is `FnMut`: the body
    /// is read once per attempt — a mid-stream failure retries it on
    /// the next attempt, within the same budget — so a closure that
    /// consumes its captures does not fit. The second argument is
    /// the attempt index (0-based): a closure that owns a file uses
    /// it to tell a retried attempt, which must clean up after the
    /// failed one, from the final attempt, which must leave the
    /// partial file for the caller. A redirect is refused, like
    /// POST: automatic redirects are disabled, so a 3xx is an error,
    /// not a body.
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
        mut read_body: impl FnMut(ureq::Body, u32) -> Result<T, GeectlError>,
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
                    if resp.status().is_success() {
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
                        // The body is read inside the retry loop, not
                        // after it: a read that times out or drops
                        // mid-body retries like a failed call, within
                        // the same budget — the previous code returned
                        // the read error at once, so a stalled or
                        // severed download never retried. Only a
                        // timeout or a transport failure retries: an
                        // over-cap body, a disk failure, or a
                        // destination that already exists fails the
                        // same way on every attempt, so retrying it
                        // would only repeat the failure.
                        match read_body(resp.into_body(), attempt) {
                            Ok(body) => return Ok(body),
                            Err(e) if is_retryable_body_error(&e) => {
                                if emit_debug_attempts {
                                    let backoff = RETRY_POLICY.backoff_base * 2u32.pow(attempt);
                                    self.debug_attempt(
                                        "GET",
                                        url,
                                        if e.kind() == ErrorKind::Timeout {
                                            "timeout"
                                        } else {
                                            "network error"
                                        },
                                        started.elapsed(),
                                        attempt + 1,
                                        Some((backoff, "body read failed")),
                                    );
                                }
                                last_desc = if e.kind() == ErrorKind::Timeout {
                                    "timeout".to_string()
                                } else {
                                    "network error".to_string()
                                };
                                last_error = Some(e);
                            }
                            Err(e) => return Err(e),
                        }
                    } else {
                        // A non-2xx status arrives as a response (see
                        // http_status_as_error(false) in with_auth): map
                        // it with its body. The error body is read once,
                        // outside the retry loop: a transient status
                        // retries the whole request, not the error-body
                        // read.
                        let body = read_error_body(resp.into_body().into_reader());
                        let safe_url = sanitize_url_for_error(url);
                        match status_action_with_body(status, &safe_url, &body) {
                            StatusAction::Return(err) => {
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
                                return Err(err);
                            }
                            StatusAction::Retry(err) => {
                                if emit_debug_attempts {
                                    let backoff = RETRY_POLICY.backoff_base * 2u32.pow(attempt);
                                    self.debug_attempt(
                                        "GET",
                                        url,
                                        &status.to_string(),
                                        started.elapsed(),
                                        attempt + 1,
                                        Some((backoff, &format!("transient status {status}"))),
                                    );
                                }
                                last_desc = status.to_string();
                                last_error = Some(err);
                            }
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
                if !resp.status().is_success() {
                    // A non-2xx status arrives as a response (see
                    // http_status_as_error(false) in with_auth): map
                    // it with its body.
                    let body = read_error_body(resp.into_body().into_reader());
                    let safe_url = sanitize_url_for_error(url);
                    // No retry for POST: a Retry action is returned
                    // as the error, immediately.
                    return match status_action_with_body(status, &safe_url, &body) {
                        StatusAction::Return(err) | StatusAction::Retry(err) => Err(err),
                    };
                }
                let body = self.read_response_body(resp.into_body().into_reader(), "POST", url)?;
                self.debug_body("response", "POST", url, &body);
                Ok(body)
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

    /// One PATCH attempt with the given content type. PATCH is never
    /// retried, like POST: a retried partial update can apply twice
    /// (e.g. a second close of an already-closed pull request).
    /// A transient failure (429, 5xx) is returned as an error
    /// immediately, not retried. This is a single attempt, end to
    /// end. A redirect is refused, like POST: automatic redirects
    /// are disabled, so a 3xx is an error, not a body.
    fn patch_with_content_type(
        &self,
        url: &str,
        body: &str,
        content_type: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        require_credentials_transport(url, credentials)?;
        debug_assert!(
            !is_retryable_method("PATCH"),
            "PATCH must stay non-retryable; a retried mutation can apply twice"
        );
        // Ask for XML, the same defence as POST: a dual-routed path
        // serves the WebUI HTML page unless the request asks for XML.
        let request = self
            .agent
            .patch(url)
            .header("Content-Type", content_type)
            .header("Accept", "application/xml");
        let request = self.apply_auth(request, credentials)?;
        // A single attempt: PATCH is never retried.
        let started = std::time::Instant::now();
        self.debug_body("request", "PATCH", url, body);
        match request.send(body) {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if resp.status().is_redirection() {
                    let location = resp.headers().get("location").and_then(|v| v.to_str().ok());
                    self.debug_single_attempt("PATCH", url, &status.to_string(), started.elapsed());
                    return Err(redirect_error(status, location));
                }
                // Attempt line first (see read_response_body).
                self.debug_single_attempt("PATCH", url, &status.to_string(), started.elapsed());
                if !resp.status().is_success() {
                    // A non-2xx status arrives as a response (see
                    // http_status_as_error(false) in with_auth): map
                    // it with its body.
                    let body = read_error_body(resp.into_body().into_reader());
                    let safe_url = sanitize_url_for_error(url);
                    // No retry for PATCH: a Retry action is returned
                    // as the error, immediately.
                    return match status_action_with_body(status, &safe_url, &body) {
                        StatusAction::Return(err) | StatusAction::Retry(err) => Err(err),
                    };
                }
                let body = self.read_response_body(resp.into_body().into_reader(), "PATCH", url)?;
                self.debug_body("response", "PATCH", url, &body);
                Ok(body)
            }
            Err(ureq::Error::Tls(msg)) => {
                self.debug_single_attempt("PATCH", url, "TLS error", started.elapsed());
                Err(GeectlError::new(
                    ErrorKind::TlsError,
                    format!("TLS error: {msg}"),
                ))
            }
            Err(ureq::Error::Rustls(e)) => {
                self.debug_single_attempt("PATCH", url, "TLS error", started.elapsed());
                Err(GeectlError::new(
                    ErrorKind::TlsError,
                    format!("TLS error: {e}"),
                ))
            }
            Err(ureq::Error::Pem(e)) => {
                self.debug_single_attempt("PATCH", url, "TLS error", started.elapsed());
                Err(GeectlError::new(
                    ErrorKind::TlsError,
                    format!("TLS error: {e}"),
                ))
            }
            Err(ureq::Error::Timeout(_)) => {
                self.debug_single_attempt("PATCH", url, "timeout", started.elapsed());
                Err(GeectlError::new(
                    ErrorKind::Timeout,
                    format!("request to {} timed out", sanitize_url_for_error(url)),
                ))
            }
            Err(ureq::Error::Io(e)) => {
                self.debug_single_attempt("PATCH", url, "network error", started.elapsed());
                Err(GeectlError::new(ErrorKind::NetworkError, e.to_string()))
            }
            Err(e) => {
                self.debug_single_attempt("PATCH", url, "server error", started.elapsed());
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
                    if resp.status().is_success() {
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
                    // A non-2xx status arrives as a response (see
                    // http_status_as_error(false) in with_auth): map
                    // it with its body.
                    let body = read_error_body(resp.into_body().into_reader());
                    let safe_url = sanitize_url_for_error(url);
                    match status_action_with_body(status, &safe_url, &body) {
                        StatusAction::Return(err) => {
                            self.debug_attempt(
                                "PUT",
                                url,
                                &status.to_string(),
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
                                &status.to_string(),
                                started.elapsed(),
                                attempt + 1,
                                Some((backoff, &format!("transient status {status}"))),
                            );
                            last_desc = status.to_string();
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

    fn delete_inner(
        &self,
        url: &str,
        body: Option<&str>,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
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
        let mut request = self.agent.delete(url).header("Accept", "application/xml");
        if body.is_some() {
            request = request.header("Content-Type", "application/json");
        }
        let request = self.apply_auth(request, credentials)?;
        // A single attempt: DELETE is never retried.
        let started = std::time::Instant::now();
        // ureq types DELETE as bodyless, so a body-carrying
        // DELETE goes through force_send_body, its escape hatch
        // for exactly this shape.
        let result = match body {
            Some(body) => {
                self.debug_body("request", "DELETE", url, body);
                request.force_send_body().send(body)
            }
            None => request.call(),
        };
        match result {
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
                if !resp.status().is_success() {
                    // A non-2xx status arrives as a response (see
                    // http_status_as_error(false) in with_auth): map
                    // it with its body.
                    let body = read_error_body(resp.into_body().into_reader());
                    let safe_url = sanitize_url_for_error(url);
                    // No retry for DELETE: a Retry action is returned
                    // as the error, immediately.
                    return match status_action_with_body(status, &safe_url, &body) {
                        StatusAction::Return(err) | StatusAction::Retry(err) => Err(err),
                    };
                }
                let body =
                    self.read_response_body(resp.into_body().into_reader(), "DELETE", url)?;
                self.debug_body("response", "DELETE", url, &body);
                Ok(body)
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
                // A timeout means "unknown, check the server state",
                // never a false failure. The caller re-issues the
                // delete itself once it has checked.
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
        .map_err(body_read_error)?;
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

/// The OBS search result-limit marker: the `<status code>` the
/// server returns when a search exceeds its configured result
/// limit (HTTP 403). The code attribute is the discriminator — a
/// genuine auth 403 carries a different code — and the match is a
/// substring search for that attribute form.
const SEARCH_RESULT_LIMIT_CODE: &str = "code=\"search_results_exceed_configured_limit\"";

/// Map a non-2xx status with its response body to an error and a
/// retry decision. A 403 whose body carries the search result-limit
/// code is a result-limit refusal, not an auth failure: the caller
/// narrows the search instead of checking credentials. Any other
/// status, or a 403 without the marker, maps exactly as
/// [`status_action`] maps it — the body never changes another
/// mapping.
fn status_action_with_body(code: u16, safe_url: &str, body: &str) -> StatusAction {
    if code == 403 && body.contains(SEARCH_RESULT_LIMIT_CODE) {
        return StatusAction::Return(GeectlError::new(
            ErrorKind::ResultLimitExceeded,
            format!("{safe_url} returned 403"),
        ));
    }
    status_action(code, safe_url)
}

/// Read an error response body for the status mapping: capped like
/// any other body, lossy UTF-8 (an error body is only inspected
/// for a marker, never parsed), empty when unreadable or over the
/// cap — the code-only mapping still applies then.
fn read_error_body(reader: impl Read) -> String {
    match read_body_bytes_capped(reader) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => String::new(),
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
        self.get_with_body(
            url,
            credentials,
            Some("application/xml"),
            true,
            |body, _attempt| {
                let text = self.read_response_body(body.into_reader(), "GET", url)?;
                self.debug_body("response", "GET", url, &text);
                Ok(text)
            },
        )
    }

    fn get_bytes(&self, url: &str, credentials: &Credentials) -> Result<Vec<u8>, GeectlError> {
        self.get_with_body(url, credentials, None, false, |body, _attempt| {
            read_body_bytes_capped(body.into_reader())
        })
    }

    fn get_to_file(
        &self,
        url: &str,
        credentials: &Credentials,
        path: &Path,
    ) -> Result<u64, GeectlError> {
        self.get_with_body(url, credentials, None, false, |body, attempt| {
            let mut file = std::fs::File::options()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|e| {
                    let kind = if e.kind() == std::io::ErrorKind::AlreadyExists {
                        ErrorKind::AlreadyExists
                    } else {
                        ErrorKind::InternalError
                    };
                    GeectlError::new(
                        kind,
                        format!("cannot create download file {}: {e}", path.display()),
                    )
                })?;
            // Copy in a loop, not `io::copy`: a read failure is the
            // network, a write failure is the disk, and the two must
            // not share one error kind.
            let mut reader = body.into_reader();
            let mut buf = [0u8; 65536];
            let mut bytes: u64 = 0;
            loop {
                // `io::copy` retries `Interrupted` reads internally;
                // the manual loop must do the same, or a transient
                // EINTR fails the whole download.
                let n = match reader.read(&mut buf) {
                    Ok(n) => n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        // A read timeout is a timeout, not a network
                        // error: a stalled transfer must be
                        // distinguishable from a severed one. The
                        // error retries either way (see
                        // `get_with_body`), but the kind decides what
                        // the caller sees when the budget runs out.
                        let kind = if is_read_timeout(&e) {
                            ErrorKind::Timeout
                        } else {
                            ErrorKind::NetworkError
                        };
                        let message =
                            format!("cannot read download body for {}: {e}", path.display());
                        // A retry follows, so the partial file goes
                        // first: the next attempt creates it
                        // exclusively, and a stale partial would
                        // read as a name collision instead. On the
                        // final attempt the partial stays, as the
                        // trait documents — the caller deletes it.
                        // If the delete fails the retry cannot
                        // proceed, so the read error is reported at
                        // once instead of being masked behind the
                        // collision the next attempt would hit.
                        if attempt < RETRY_POLICY.max_retries && std::fs::remove_file(path).is_err()
                        {
                            return Err(GeectlError::new(kind, message));
                        }
                        return Err(GeectlError::new(kind, message));
                    }
                };
                if n == 0 {
                    break;
                }
                file.write_all(&buf[..n]).map_err(|e| {
                    GeectlError::new(
                        ErrorKind::InternalError,
                        format!("cannot write download file {}: {e}", path.display()),
                    )
                })?;
                bytes += n as u64;
            }
            file.sync_all().map_err(|e| {
                GeectlError::new(
                    ErrorKind::InternalError,
                    format!("cannot sync download file {}: {e}", path.display()),
                )
            })?;
            Ok(bytes)
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

    fn patch_json_with(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        // The JSON PATCH twin of post_json_with: the same explicit
        // credentials, but PATCH semantics — single-attempt, like
        // POST, because a retried partial update can apply twice.
        self.patch_with_content_type(url, body, "application/json", credentials)
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
        self.delete_inner(url, None, credentials)
    }

    fn delete_json_with(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        // The JSON-body twin of delete: same single attempt, same
        // explicit credentials — the cancel of a Gitea review
        // request carries the reviewer list in the body.
        self.delete_inner(url, Some(body), credentials)
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
    fn get_to_file_streams_a_body_over_the_cap_to_disk() {
        // A body over the cap streams to disk through `get_to_file`:
        // the bytes land whole, and the returned count is the file's
        // length.
        let data = vec![b'a'; MAX_BODY_BYTES + 1];
        let client = UreqClient::new(Timeout::default()).unwrap();
        let (port, server) = serve_bytes_once(data.clone());
        let url = format!("http://127.0.0.1:{port}/pkg.rpm");
        let path =
            std::env::temp_dir().join(format!("geectl-get-to-file-{}.rpm", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let written = client.get_to_file(&url, &Credentials::None, &path).unwrap();
        assert_eq!(written, data.len() as u64);
        assert_eq!(std::fs::read(&path).unwrap(), data);
        let _ = std::fs::remove_file(&path);
        server.join().expect("server thread");
    }

    #[test]
    fn get_to_file_refuses_a_pre_existing_destination() {
        // The destination is created exclusively: a pre-existing file
        // is an `AlreadyExists` error, and its contents are left
        // intact.
        let data = vec![b'a'; MAX_BODY_BYTES + 1];
        let client = UreqClient::new(Timeout::default()).unwrap();
        let (port, server) = serve_bytes_once(data);
        let url = format!("http://127.0.0.1:{port}/pkg.rpm");
        let path = std::env::temp_dir().join(format!(
            "geectl-get-to-file-exists-{}.rpm",
            std::process::id()
        ));
        std::fs::write(&path, b"existing").unwrap();
        let err = client
            .get_to_file(&url, &Credentials::None, &path)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&path).unwrap(), b"existing");
        let _ = std::fs::remove_file(&path);
        server.join().expect("server thread");
    }

    #[test]
    fn get_to_file_retries_a_severed_download_and_restarts() {
        // First attempt sends half the body then drops the
        // connection; the retry re-downloads from byte zero. The
        // halves differ, so the content pins the order: a resume
        // that appended the re-download to the partial and
        // truncated to length would repeat the first half.
        let mut body = vec![b'a'; 100 * 1024];
        body.extend_from_slice(&vec![b'b'; 100 * 1024]);
        let (port, count, server) = serve_download(body.clone(), DownloadBehavior::SeverOnce);
        let client = UreqClient::new(Timeout::default()).unwrap();
        let url = format!("http://127.0.0.1:{port}/pkg.rpm");
        let path = std::env::temp_dir().join(format!(
            "geectl-get-to-file-sever-{}.rpm",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let written = client.get_to_file(&url, &Credentials::None, &path).unwrap();
        assert_eq!(written, body.len() as u64);
        assert_eq!(std::fs::read(&path).unwrap(), body);
        let _ = std::fs::remove_file(&path);
        server.join().expect("server thread");
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn get_to_file_retries_a_stalled_download() {
        // First attempt stalls mid-body past the read timeout; the
        // retry downloads the whole body.
        let body = vec![b'b'; 200 * 1024];
        let (port, count, server) = serve_download(body.clone(), DownloadBehavior::StallOnce);
        let timeout = Timeout::new(
            std::time::Duration::from_secs(2),
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        let client = UreqClient::new(timeout).unwrap();
        let url = format!("http://127.0.0.1:{port}/pkg.rpm");
        let path = std::env::temp_dir().join(format!(
            "geectl-get-to-file-stall-{}.rpm",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let written = client.get_to_file(&url, &Credentials::None, &path).unwrap();
        assert_eq!(written, body.len() as u64);
        assert_eq!(std::fs::read(&path).unwrap(), body);
        let _ = std::fs::remove_file(&path);
        server.join().expect("server thread");
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn get_to_file_severed_download_stops_after_the_retry_budget() {
        // A server that severs every attempt: exactly 1 + 3
        // attempts, then the network error — the retry budget is
        // shared with the call-stage retries, not a second budget.
        let body = vec![b'b'; 200 * 1024];
        let (port, count, server) = serve_download(body, DownloadBehavior::SeverAlways);
        let client = UreqClient::new(Timeout::default()).unwrap();
        let url = format!("http://127.0.0.1:{port}/pkg.rpm");
        let path = std::env::temp_dir().join(format!(
            "geectl-get-to-file-sever-budget-{}.rpm",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let err = client
            .get_to_file(&url, &Credentials::None, &path)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NetworkError);
        // The final failure leaves the partial file for the caller.
        assert!(path.exists());
        let _ = std::fs::remove_file(&path);
        server.join().expect("server thread");
        assert_eq!(count.load(Ordering::SeqCst), RETRY_POLICY.max_retries + 1);
    }

    #[test]
    fn get_to_file_stalled_download_fails_as_a_timeout() {
        // A server that stalls every attempt: the budget runs out
        // on a Timeout, not a NetworkError — a stalled transfer is
        // distinguishable from a severed one.
        let body = vec![b'b'; 200 * 1024];
        let (port, count, server) = serve_download(body, DownloadBehavior::StallAlways);
        let timeout = Timeout::new(
            std::time::Duration::from_secs(2),
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        let client = UreqClient::new(timeout).unwrap();
        let url = format!("http://127.0.0.1:{port}/pkg.rpm");
        let path = std::env::temp_dir().join(format!(
            "geectl-get-to-file-stall-budget-{}.rpm",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let err = client
            .get_to_file(&url, &Credentials::None, &path)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Timeout);
        let _ = std::fs::remove_file(&path);
        server.join().expect("server thread");
        assert_eq!(count.load(Ordering::SeqCst), RETRY_POLICY.max_retries + 1);
    }

    #[test]
    fn over_cap_body_read_is_not_retried() {
        // An over-cap body fails as a ServerError on the first
        // attempt: retrying it would return the same body.
        let body = vec![b'a'; MAX_BODY_BYTES + 1];
        let (port, count, server) = serve_download(body, DownloadBehavior::AlwaysOk);
        let client = UreqClient::new(Timeout::default()).unwrap();
        let url = format!("http://127.0.0.1:{port}/pkg.rpm");
        let err = client.get_bytes(&url, &Credentials::None).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        server.join().expect("server thread");
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn body_read_failures_retry_only_on_timeout_or_network_error() {
        assert!(is_retryable_body_error(&GeectlError::new(
            ErrorKind::Timeout,
            "stalled"
        )));
        assert!(is_retryable_body_error(&GeectlError::new(
            ErrorKind::NetworkError,
            "severed"
        )));
        assert!(!is_retryable_body_error(&GeectlError::new(
            ErrorKind::ServerError,
            "over cap"
        )));
        assert!(!is_retryable_body_error(&GeectlError::new(
            ErrorKind::InternalError,
            "disk"
        )));
        assert!(!is_retryable_body_error(&GeectlError::new(
            ErrorKind::AlreadyExists,
            "exists"
        )));
    }

    #[test]
    fn read_timeout_is_recognized_through_the_ureq_wrapper() {
        // A bare TimedOut is a timeout.
        assert!(is_read_timeout(&std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "timed out"
        )));
        // ureq's body reader converts its Error::Timeout into an
        // io error of kind Other carrying the timeout — the kind
        // alone does not say timeout, the wrapped error does.
        assert!(is_read_timeout(&std::io::Error::other(
            ureq::Error::Timeout(ureq::Timeout::PerRead)
        )));
        // A severed connection is not a timeout, wrapped or not.
        assert!(!is_read_timeout(&std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "reset"
        )));
        assert!(!is_read_timeout(&std::io::Error::other(
            ureq::Error::StatusCode(500)
        )));
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

    #[test]
    fn result_limit_403_is_not_an_auth_failure() {
        // The OBS search result-limit body on a 403 is a
        // result-limit refusal, not an auth failure: the
        // credentials are fine, the search is too broad.
        let limit_body = "<status code=\"search_results_exceed_configured_limit\">\n  <summary>The number of results returned by the performed search exceeds the configured limit.</summary>\n</status>";
        assert!(matches!(
            status_action_with_body(403, "u", limit_body),
            StatusAction::Return(e) if e.kind() == ErrorKind::ResultLimitExceeded
        ));
        assert_eq!(
            ErrorKind::ResultLimitExceeded.code(),
            "result_limit_exceeded"
        );
        assert_eq!(ErrorKind::ResultLimitExceeded.exit_code(), 23);
        // A 403 without the marker is still an auth failure.
        assert!(matches!(
            status_action_with_body(403, "u", ""),
            StatusAction::Return(e) if e.kind() == ErrorKind::AuthFailed
        ));
        assert!(matches!(
            status_action_with_body(403, "u", "<status code=\"forbidden\"><summary>no</summary></status>"),
            StatusAction::Return(e) if e.kind() == ErrorKind::AuthFailed
        ));
        // The marker only matters on 403: other statuses map as
        // status_action maps them, body or not.
        assert!(matches!(
            status_action_with_body(404, "u", limit_body),
            StatusAction::Return(e) if e.kind() == ErrorKind::NotFound
        ));
        assert!(matches!(
            status_action_with_body(500, "u", limit_body),
            StatusAction::Retry(e) if e.kind() == ErrorKind::ServerError
        ));
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

    /// The exact 403 body the OBS search returns when a search
    /// exceeds its configured result limit (captured live from
    /// api.opensuse.org, superseded Factory search).
    const SEARCH_RESULT_LIMIT_BODY: &str = "<status code=\"search_results_exceed_configured_limit\">\n  <summary>The number of results returned by the performed search exceeds the configured limit.</summary>\n</status>";

    #[test]
    fn get_result_limit_403_reports_result_limit() {
        let server = LoopbackServer::serve_status(SEARCH_RESULT_LIMIT_BODY, 403);
        let client = UreqClient::new(Timeout::default()).unwrap();
        let err = client.get(&server.url()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ResultLimitExceeded);
        assert_eq!(err.exit_code(), 23);
    }

    #[test]
    fn get_auth_403_still_reports_auth_failed() {
        let server = LoopbackServer::serve_status(
            "<status code=\"forbidden\"><summary>no</summary></status>",
            403,
        );
        let client = UreqClient::new(Timeout::default()).unwrap();
        let err = client.get(&server.url()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AuthFailed);
    }

    #[test]
    fn post_result_limit_403_reports_result_limit() {
        let server = LoopbackServer::serve_status(SEARCH_RESULT_LIMIT_BODY, 403);
        let client = UreqClient::with_auth(Timeout::default(), None).unwrap();
        let err = client.post_json(&server.url(), "{}").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ResultLimitExceeded);
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

    /// How a download server fails a `get_to_file` download.
    /// `Sever` closes the socket mid-body; `Stall` holds it open
    /// past the client's read timeout, then closes it. `Once` fails
    /// only the first attempt, `Always` every attempt.
    enum DownloadBehavior {
        SeverOnce,
        StallOnce,
        SeverAlways,
        StallAlways,
        AlwaysOk,
    }

    /// A loopback server that answers `get_to_file` downloads with
    /// `body`, failing attempts per `behavior`. Returns the port,
    /// the request count, and the server thread. The server stays
    /// up past the longest backoff gap (4 s) so a retrying client
    /// still finds it on its last attempt.
    fn serve_download(
        body: Vec<u8>,
        behavior: DownloadBehavior,
    ) -> (
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
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
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
                        let _ = read_request(&mut stream);
                        let fail = match behavior {
                            DownloadBehavior::SeverOnce | DownloadBehavior::StallOnce => n == 0,
                            DownloadBehavior::SeverAlways | DownloadBehavior::StallAlways => true,
                            DownloadBehavior::AlwaysOk => false,
                        };
                        let headers = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(headers.as_bytes());
                        if fail {
                            let half = body.len() / 2;
                            let _ = stream.write_all(&body[..half]);
                            if matches!(
                                behavior,
                                DownloadBehavior::StallOnce | DownloadBehavior::StallAlways
                            ) {
                                // Hold the socket open past the
                                // client's read timeout (1 s in the
                                // stall tests) before closing it.
                                std::thread::sleep(std::time::Duration::from_secs(3));
                            }
                        } else {
                            let _ = stream.write_all(&body);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if let Some(last) = last_request {
                            // Past the longest backoff gap (4 s) with
                            // margin: the client may still be
                            // sleeping before its last attempt.
                            if last.elapsed() > std::time::Duration::from_secs(8) {
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
    fn delete_json_with_sends_json_body_and_auth() {
        let server = RequestCapture::serve("");
        let client = UreqClient::new(Timeout::default()).unwrap();
        let url = server.url_for("/repos/geectl/probe/pulls/1/requested_reviewers");
        assert_eq!(
            client
                .delete_json_with(
                    &url,
                    "{\"reviewers\":[\"bob\"],\"team_reviewers\":[]}",
                    &Credentials::Bearer {
                        token: "tok3n".to_string()
                    },
                )
                .unwrap(),
            ""
        );
        let (method, content_type, auth, received_body) = server.captured();
        assert_eq!(method, "DELETE");
        assert_eq!(content_type.as_deref(), Some("application/json"));
        // captured() carries the raw header, unlike
        // server.authorization() which redacts the token.
        assert_eq!(auth.as_deref(), Some("Bearer tok3n"));
        assert_eq!(
            received_body,
            "{\"reviewers\":[\"bob\"],\"team_reviewers\":[]}"
        );
    }

    #[test]
    fn delete_json_with_is_never_retried() {
        // The same single-attempt rule as delete: a 500 must come
        // back as a ServerError after exactly ONE attempt.
        let (port, count, handle) = serve_counting_500();
        let client = UreqClient::new(Timeout::default()).unwrap();
        let url = format!("http://127.0.0.1:{port}/repos/geectl/probe/pulls/1/requested_reviewers");
        let err = client
            .delete_json_with(&url, "{}", &Credentials::None)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        handle.join().expect("server thread");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn patch_json_with_sends_json_body_and_auth() {
        let server = RequestCapture::serve("");
        let client = UreqClient::new(Timeout::default()).unwrap();
        let url = server.url_for("/repos/openSUSE/geectl/issues/122");
        assert_eq!(
            client
                .patch_json_with(
                    &url,
                    "{\"state\":\"closed\"}",
                    &Credentials::Bearer {
                        token: "patch-test-token".to_string(),
                    },
                )
                .unwrap(),
            ""
        );
        let (method, content_type, auth, received_body) = server.captured();
        assert_eq!(method, "PATCH");
        assert_eq!(content_type.as_deref(), Some("application/json"));
        // captured() carries the raw header, unlike
        // server.authorization() which redacts the token.
        assert_eq!(auth.as_deref(), Some("Bearer patch-test-token"));
        assert_eq!(received_body, "{\"state\":\"closed\"}");
    }

    #[test]
    fn patch_json_with_maps_404_to_not_found() {
        let server = LoopbackServer::serve_status("no such issue", 404);
        let client = UreqClient::with_auth(Timeout::default(), None).unwrap();
        let err = client
            .patch_json_with(&server.url(), "{}", &Credentials::None)
            .expect_err("a 404 PATCH must fail");
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn patch_json_with_is_never_retried() {
        // The same single-attempt rule as POST and DELETE: a 500
        // must come back as a ServerError after exactly ONE attempt.
        let (port, count, handle) = serve_counting_500();
        let client = UreqClient::new(Timeout::default()).unwrap();
        let url = format!("http://127.0.0.1:{port}/repos/openSUSE/geectl/issues/122");
        let err = client
            .patch_json_with(&url, "{}", &Credentials::None)
            .unwrap_err();
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
    fn debug_on_records_body_failure_line_for_severed_read() {
        // A 200 whose body severs mid-read emits two lines for the
        // attempt: the status line when the headers arrive, then a
        // body-failure line naming the retry — the body read is a
        // separate event from the status, not a second attempt.
        let body = vec![b'b'; 200 * 1024];
        let (port, count, server) = serve_download(body, DownloadBehavior::SeverOnce);
        let (debug, lines) = Debug::captured(true, false);
        let client = UreqClient::new(Timeout::default())
            .unwrap()
            .with_debug(debug);
        let url = format!("http://127.0.0.1:{port}/public/source/openSUSE:Factory/foo");
        let read = client.get(&url).unwrap();
        assert_eq!(read.len(), 200 * 1024);
        server.join().expect("server thread");
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(
            debug_lines_without_durations(&take_debug_lines(&lines)),
            vec![
                "decision: read via /public/source (no credentials)".to_string(),
                "http: GET /public/source/openSUSE:Factory/foo -> 200 (attempt 1/4)".to_string(),
                "http: GET /public/source/openSUSE:Factory/foo -> network error (attempt 1/4, retry in 1s: body read failed)".to_string(),
                "http: GET /public/source/openSUSE:Factory/foo -> 200 (attempt 2/4)".to_string(),
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
        let url = format!("http://127.0.0.1:{}/source", server.port);
        assert_eq!(client.get(&url).unwrap(), "hello");
        let lines = take_debug_lines(&lines);
        assert!(
            lines
                .iter()
                .any(|l| l == "decision: read via /source (authenticated)"),
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

    /// A fake HTTP CONNECT proxy: accepts one connection, answers
    /// the CONNECT with `200 Connection Established`, then records
    /// the request sent through the tunnel and answers it. ureq
    /// tunnels every proxied request through CONNECT — even plain
    /// HTTP targets — so a single-shot server cannot stand in for
    /// a proxy: the handshake and the request are two exchanges on
    /// one connection.
    struct ConnectProxyServer {
        port: u16,
        handle: std::thread::JoinHandle<Option<(String, String)>>,
    }

    impl ConnectProxyServer {
        fn serve(body: &'static str) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind proxy server");
            listener.set_nonblocking(true).expect("nonblocking");
            let port = listener.local_addr().expect("local addr").port();
            let handle = std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
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
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                use std::io::Write;
                let connect = read_request(&mut stream);
                let connect = String::from_utf8_lossy(&connect).into_owned();
                let _ = stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n");
                let _ = stream.flush();
                let tunneled = read_request(&mut stream);
                let tunneled = String::from_utf8_lossy(&tunneled).into_owned();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                Some((connect, tunneled))
            });
            ConnectProxyServer { port, handle }
        }

        fn requests(self) -> Option<(String, String)> {
            self.handle.join().ok().flatten()
        }
    }

    #[test]
    fn proxy_is_used_for_requests() {
        // The client must tunnel through the proxy: a CONNECT to
        // the target host first, then the request itself through
        // the tunnel. The proxy's answer is what the client
        // returns. The proxy URL carries userinfo, which ureq sends
        // as `Proxy-Authorization: Basic` on CONNECT.
        let proxy = ConnectProxyServer::serve("proxied");
        let proxy_url = format!("http://user:pass@127.0.0.1:{}", proxy.port);
        let client = UreqClient::with_auth_and_proxy(
            Timeout::default(),
            None,
            Some(ureq::Proxy::new(&proxy_url).unwrap()),
        )
        .unwrap();
        assert_eq!(client.get("http://example.com/test").unwrap(), "proxied");
        let (connect, tunneled) = proxy.requests().expect("requests reached the proxy");
        assert!(
            connect.starts_with("CONNECT example.com:80 "),
            "unexpected CONNECT line: {connect}"
        );
        assert!(
            connect.contains("Proxy-Authorization: Basic dXNlcjpwYXNz"),
            "CONNECT lacks proxy credentials: {connect}"
        );
        assert!(
            tunneled.starts_with("GET /test "),
            "unexpected tunneled request: {tunneled}"
        );
    }

    #[test]
    fn no_proxy_bypasses_the_proxy() {
        // A dead proxy (nothing listening on port 9) with
        // `no_proxy` covering the loopback target: the request
        // must go direct and succeed.
        let server = LoopbackServer::serve("hello");
        let proxy = ureq::Proxy::builder(ureq::ProxyProtocol::Http)
            .host("127.0.0.1")
            .port(9)
            .no_proxy("127.0.0.1")
            .build()
            .unwrap();
        let client =
            UreqClient::with_auth_and_proxy(Timeout::default(), None, Some(proxy)).unwrap();
        assert_eq!(client.get(&server.url()).unwrap(), "hello");
        assert!(server.request().is_some());
    }

    #[test]
    fn default_client_reflects_proxy_env() {
        // The default client reflects the process environment.
        // No proxy variables, no proxy. The assertion is written
        // against the ambient environment rather than mutating
        // it: tests share one process, so setting a proxy variable
        // here would reroute other tests' clients mid-run. When
        // the environment does carry a proxy, the configured
        // proxy must be exactly the one the environment names.
        let client = UreqClient::new(Timeout::default()).unwrap();
        let env_has_proxy = [
            "ALL_PROXY",
            "all_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
        ]
        .iter()
        .any(|name| std::env::var_os(name).is_some());
        if env_has_proxy {
            // Independent of `try_from_env`: walk the variables in
            // the documented order and take the first set one with
            // a parseable URI.
            let mut expected = None;
            for name in [
                "ALL_PROXY",
                "all_proxy",
                "HTTPS_PROXY",
                "https_proxy",
                "HTTP_PROXY",
                "http_proxy",
            ] {
                let Ok(value) = std::env::var(name) else {
                    continue;
                };
                if let Ok(proxy) = ureq::Proxy::new(&value) {
                    expected = Some(proxy);
                    break;
                }
            }
            assert_eq!(
                client.agent.config().proxy().map(|p| p.uri()),
                expected.as_ref().map(|p| p.uri())
            );
        } else {
            assert!(client.agent.config().proxy().is_none());
        }
    }

    #[test]
    fn loopback_target_goes_through_the_proxy() {
        // A loopback target with no `NO_PROXY` entry: the request
        // must tunnel through the proxy, not go direct — the
        // proxy's answer is what the client returns, not the
        // loopback server's. The proxy is built by hand, so the
        // process environment cannot exempt the target.
        let proxy = ConnectProxyServer::serve("proxied");
        let proxy_url = format!("http://127.0.0.1:{}", proxy.port);
        let client = UreqClient::with_auth_and_proxy(
            Timeout::default(),
            None,
            Some(ureq::Proxy::new(&proxy_url).unwrap()),
        )
        .unwrap();
        let target = LoopbackServer::serve("hello");
        assert_eq!(client.get(&target.url()).unwrap(), "proxied");
        let (connect, tunneled) = proxy.requests().expect("requests reached the proxy");
        assert!(
            connect.starts_with("CONNECT 127.0.0.1:"),
            "unexpected CONNECT line: {connect}"
        );
        assert!(
            tunneled.starts_with("GET /test "),
            "unexpected tunneled request: {tunneled}"
        );
    }

    /// A minimal SOCKS5 server: one handshake, one CONNECT, then the
    /// tunneled HTTP request answered directly, mirroring
    /// `ConnectProxyServer` with the SOCKS5 greeting in front.
    struct Socks5Server {
        port: u16,
        handle: std::thread::JoinHandle<Option<(String, String)>>,
    }

    impl Socks5Server {
        fn serve(body: &'static str) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind socks server");
            listener.set_nonblocking(true).expect("nonblocking");
            let port = listener.local_addr().expect("local addr").port();
            let handle = std::thread::spawn(move || {
                use std::io::{Read, Write};
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
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
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                // SOCKS5 greeting: VER, NMETHODS, METHODS; no-auth only.
                let mut greeting = [0u8; 2];
                stream.read_exact(&mut greeting).ok()?;
                if greeting[0] != 0x05 {
                    return None;
                }
                let mut methods = vec![0u8; greeting[1] as usize];
                stream.read_exact(&mut methods).ok()?;
                stream.write_all(&[0x05, 0x00]).ok()?;
                // CONNECT request: VER CMD RSV ATYP, then address + port.
                let mut head = [0u8; 4];
                stream.read_exact(&mut head).ok()?;
                if head[0] != 0x05 || head[1] != 0x01 {
                    return None;
                }
                let target = match head[3] {
                    0x01 => {
                        let mut addr = [0u8; 6];
                        stream.read_exact(&mut addr).ok()?;
                        format!(
                            "{}.{}.{}.{}:{}",
                            addr[0],
                            addr[1],
                            addr[2],
                            addr[3],
                            u16::from_be_bytes([addr[4], addr[5]])
                        )
                    }
                    0x03 => {
                        let mut len = [0u8; 1];
                        stream.read_exact(&mut len).ok()?;
                        let mut domain = vec![0u8; len[0] as usize + 2];
                        stream.read_exact(&mut domain).ok()?;
                        let port = u16::from_be_bytes([
                            domain[len[0] as usize],
                            domain[len[0] as usize + 1],
                        ]);
                        format!(
                            "{}:{}",
                            String::from_utf8_lossy(&domain[..len[0] as usize]),
                            port
                        )
                    }
                    0x04 => {
                        let mut addr = [0u8; 18];
                        stream.read_exact(&mut addr).ok()?;
                        let mut ip = [0u8; 16];
                        ip.copy_from_slice(&addr[..16]);
                        format!(
                            "{}:{}",
                            std::net::Ipv6Addr::from(ip),
                            u16::from_be_bytes([addr[16], addr[17]])
                        )
                    }
                    _ => return None,
                };
                stream
                    .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                    .ok()?;
                let _ = stream.flush();
                let tunneled = read_request(&mut stream);
                let tunneled = String::from_utf8_lossy(&tunneled).into_owned();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                Some((target, tunneled))
            });
            Socks5Server { port, handle }
        }

        fn requests(self) -> Option<(String, String)> {
            self.handle.join().ok().flatten()
        }
    }

    #[test]
    fn socks5_proxy_is_used_for_requests() {
        // A SOCKS5 proxy used to be silently ignored: without the
        // `socks-proxy` feature ureq falls back to a direct
        // connection (and panics for a manually configured one).
        // The request must tunnel through the SOCKS handshake and
        // come back with the proxy's answer.
        let proxy = Socks5Server::serve("proxied");
        let proxy_url = format!("socks5://127.0.0.1:{}", proxy.port);
        let client = UreqClient::with_auth_and_proxy(
            Timeout::default(),
            None,
            Some(ureq::Proxy::new(&proxy_url).unwrap()),
        )
        .unwrap();
        assert_eq!(client.get("http://127.0.0.1:9/test").unwrap(), "proxied");
        let (target, tunneled) = proxy.requests().expect("requests reached the proxy");
        assert_eq!(target, "127.0.0.1:9");
        assert!(
            tunneled.starts_with("GET /test "),
            "unexpected tunneled request: {tunneled}"
        );
    }

    /// A minimal SOCKS4 server: one handshake, one CONNECT, then the
    /// tunneled HTTP request answered directly, mirroring
    /// `Socks5Server` with the SOCKS4 greeting in front.
    struct Socks4Server {
        port: u16,
        handle: std::thread::JoinHandle<Option<(String, String)>>,
    }

    impl Socks4Server {
        fn serve(body: &'static str) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind socks4 server");
            listener.set_nonblocking(true).expect("nonblocking");
            let port = listener.local_addr().expect("local addr").port();
            let handle = std::thread::spawn(move || {
                use std::io::{Read, Write};
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
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
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                // SOCKS4 CONNECT: VER CMD DSTPORT DSTIP USERID NUL.
                let mut head = [0u8; 8];
                stream.read_exact(&mut head).ok()?;
                if head[0] != 0x04 || head[1] != 0x01 {
                    return None;
                }
                let target = format!(
                    "{}.{}.{}.{}:{}",
                    head[4],
                    head[5],
                    head[6],
                    head[7],
                    u16::from_be_bytes([head[2], head[3]])
                );
                // Read the NUL-terminated USERID.
                let mut userid = [0u8; 1];
                loop {
                    stream.read_exact(&mut userid).ok()?;
                    if userid[0] == 0 {
                        break;
                    }
                }
                // Granted: VN 0x00, CD 0x5A, then DSTPORT DSTIP.
                stream
                    .write_all(&[
                        0x00, 0x5A, head[2], head[3], head[4], head[5], head[6], head[7],
                    ])
                    .ok()?;
                let _ = stream.flush();
                let tunneled = read_request(&mut stream);
                let tunneled = String::from_utf8_lossy(&tunneled).into_owned();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                Some((target, tunneled))
            });
            Socks4Server { port, handle }
        }

        fn requests(self) -> Option<(String, String)> {
            self.handle.join().ok().flatten()
        }
    }

    #[test]
    fn socks4_proxy_is_used_for_requests() {
        // SOCKS4 is honoured like SOCKS5: the request must go
        // through the SOCKS4 handshake and come back with the
        // proxy's answer.
        let proxy = Socks4Server::serve("proxied");
        let proxy_url = format!("socks4://127.0.0.1:{}", proxy.port);
        let client = UreqClient::with_auth_and_proxy(
            Timeout::default(),
            None,
            Some(ureq::Proxy::new(&proxy_url).unwrap()),
        )
        .unwrap();
        assert_eq!(client.get("http://127.0.0.1:9/test").unwrap(), "proxied");
        let (target, tunneled) = proxy.requests().expect("requests reached the proxy");
        assert_eq!(target, "127.0.0.1:9");
        assert!(
            tunneled.starts_with("GET /test "),
            "unexpected tunneled request: {tunneled}"
        );
    }

    #[test]
    fn socks5h_proxy_sends_domain_targets() {
        // A domain target reaches the SOCKS server as ATYP 0x03
        // with the domain. Plain SOCKS5 resolves locally first, so
        // the domain form is exercised through SOCKS5h (remote DNS).
        let proxy = Socks5Server::serve("proxied");
        let proxy_url = format!("socks5h://127.0.0.1:{}", proxy.port);
        let client = UreqClient::with_auth_and_proxy(
            Timeout::default(),
            None,
            Some(ureq::Proxy::new(&proxy_url).unwrap()),
        )
        .unwrap();
        assert_eq!(client.get("http://example.com/test").unwrap(), "proxied");
        let (target, tunneled) = proxy.requests().expect("requests reached the proxy");
        assert_eq!(target, "example.com:80");
        assert!(
            tunneled.starts_with("GET /test "),
            "unexpected tunneled request: {tunneled}"
        );
    }

    #[test]
    fn socks5_proxy_sends_ipv6_targets() {
        // An IPv6 literal target reaches the SOCKS5 server as ATYP
        // 0x04 with the address, not a mapped IPv4 address.
        let proxy = Socks5Server::serve("proxied");
        let proxy_url = format!("socks5://127.0.0.1:{}", proxy.port);
        let client = UreqClient::with_auth_and_proxy(
            Timeout::default(),
            None,
            Some(ureq::Proxy::new(&proxy_url).unwrap()),
        )
        .unwrap();
        assert_eq!(client.get("http://[::1]:9/test").unwrap(), "proxied");
        let (target, tunneled) = proxy.requests().expect("requests reached the proxy");
        assert_eq!(target, "::1:9");
        assert!(
            tunneled.starts_with("GET /test "),
            "unexpected tunneled request: {tunneled}"
        );
    }

    /// Helper run only as a subprocess by
    /// `proxy_env_precedence_and_malformed_in_subprocess`: it reads
    /// the case from `GEectl_PROXY_ENV_CASE` and asserts what the
    /// default client picks up. Run directly it does nothing, so a
    /// plain `cargo test` run never mutates the process
    /// environment.
    #[test]
    fn proxy_env_subprocess_helper() {
        let Ok(case) = std::env::var("GEectl_PROXY_ENV_CASE") else {
            return;
        };
        let client = UreqClient::new(Timeout::default()).unwrap();
        let proxy = client.agent.config().proxy();
        match case.as_str() {
            "all-beats-https" => {
                let expected = ureq::Proxy::new("http://127.0.0.1:9").unwrap();
                assert_eq!(proxy.map(|p| p.uri()), Some(expected.uri()));
            }
            "malformed-skipped" => {
                let expected = ureq::Proxy::new("http://127.0.0.1:10").unwrap();
                assert_eq!(proxy.map(|p| p.uri()), Some(expected.uri()));
            }
            "malformed-only" => {
                assert!(proxy.is_none());
            }
            "lowercase-only" => {
                let expected = ureq::Proxy::new("http://127.0.0.1:11").unwrap();
                assert_eq!(proxy.map(|p| p.uri()), Some(expected.uri()));
            }
            "http-only" => {
                let expected = ureq::Proxy::new("http://127.0.0.1:12").unwrap();
                assert_eq!(proxy.map(|p| p.uri()), Some(expected.uri()));
            }
            "all-lower-beats-https" => {
                let expected = ureq::Proxy::new("http://127.0.0.1:9").unwrap();
                assert_eq!(proxy.map(|p| p.uri()), Some(expected.uri()));
            }
            "no-proxy-bypasses-via-env" => {
                // `NO_PROXY` from the environment must exempt the
                // loopback target from a dead proxy: the request
                // goes direct and succeeds.
                let server = LoopbackServer::serve("hello");
                assert_eq!(client.get(&server.url()).unwrap(), "hello");
                assert!(server.request().is_some());
            }
            _ => panic!("unknown proxy env case: {case}"),
        }
    }

    #[test]
    fn proxy_env_precedence_and_malformed_in_subprocess() {
        // Env precedence and the malformed-URI fallback cannot be
        // pinned in-process: setting a proxy variable here would
        // reroute other tests' clients in the shared process. Each
        // case runs the helper above in a fresh process with a
        // controlled environment instead.
        let exe = std::env::current_exe().expect("current test binary");
        let cases: &[(&str, &[(&str, &str)])] = &[
            (
                "all-beats-https",
                &[
                    ("ALL_PROXY", "http://127.0.0.1:9"),
                    ("HTTPS_PROXY", "http://127.0.0.1:10"),
                ],
            ),
            (
                "malformed-skipped",
                &[
                    ("ALL_PROXY", "http://[::1"),
                    ("HTTPS_PROXY", "http://127.0.0.1:10"),
                ],
            ),
            ("malformed-only", &[("ALL_PROXY", "http://[::1")]),
            ("lowercase-only", &[("http_proxy", "http://127.0.0.1:11")]),
            ("http-only", &[("HTTP_PROXY", "http://127.0.0.1:12")]),
            (
                "all-lower-beats-https",
                &[
                    ("all_proxy", "http://127.0.0.1:9"),
                    ("HTTPS_PROXY", "http://127.0.0.1:10"),
                ],
            ),
            (
                "no-proxy-bypasses-via-env",
                &[
                    ("ALL_PROXY", "http://127.0.0.1:9"),
                    ("NO_PROXY", "127.0.0.1"),
                ],
            ),
        ];
        for (case, vars) in cases {
            let mut cmd = std::process::Command::new(&exe);
            cmd.arg("http::tests::proxy_env_subprocess_helper")
                .arg("--exact")
                .arg("--nocapture")
                .env("GEectl_PROXY_ENV_CASE", case);
            for name in [
                "ALL_PROXY",
                "all_proxy",
                "HTTPS_PROXY",
                "https_proxy",
                "HTTP_PROXY",
                "http_proxy",
                "NO_PROXY",
                "no_proxy",
            ] {
                cmd.env_remove(name);
            }
            for (name, value) in *vars {
                cmd.env(name, value);
            }
            let out = cmd.output().expect("spawn helper");
            assert!(
                out.status.success(),
                "proxy env case {case} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}
