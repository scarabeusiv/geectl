use super::RequestSearch;
use super::SrPayload;
use crate::auth::Credentials;
use crate::error::ErrorKind;
use crate::error::GeectlError;
use crate::http::{follow_mirror_redirects, HttpClient, MirrorHop, MAX_BODY_BYTES};
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
/// A fake HTTP client with canned responses, for fixtures. GET,
/// POST, PATCH, and DELETE are keyed separately by URL; a URL with
/// no canned response is a transport error. Calls are recorded so
/// tests can assert on the method, the URL, whether auth was
/// sent, and (for POST and PATCH) the body.
pub(crate) struct FakeHttp {
    gets: RefCell<HashMap<String, String>>,
    get_sequences: RefCell<HashMap<String, VecDeque<String>>>,
    get_errors: RefCell<HashMap<String, ErrorKind>>,
    get_error_sequences: RefCell<HashMap<String, VecDeque<ErrorKind>>>,
    posts: RefCell<HashMap<String, String>>,
    post_sequences: RefCell<HashMap<String, VecDeque<String>>>,
    post_errors: RefCell<HashMap<String, VecDeque<ErrorKind>>>,
    patches: RefCell<HashMap<String, String>>,
    patch_errors: RefCell<HashMap<String, VecDeque<ErrorKind>>>,
    deletes: RefCell<HashMap<String, String>>,
    delete_errors: RefCell<HashMap<String, ErrorKind>>,
    puts: RefCell<HashMap<String, String>>,
    put_errors: RefCell<HashMap<String, VecDeque<ErrorKind>>>,
    get_calls: RefCell<Vec<(String, bool)>>,
    get_bearer_tokens: RefCell<Vec<(String, Option<String>)>>,
    get_bytes: RefCell<HashMap<String, Vec<u8>>>,
    get_bytes_error_sequences: RefCell<HashMap<String, VecDeque<ErrorKind>>>,
    get_bytes_cap_errors: RefCell<HashSet<String>>,
    get_to_file: RefCell<HashMap<String, Vec<u8>>>,
    get_to_file_errors: RefCell<HashMap<String, ErrorKind>>,
    get_to_file_error_writes: RefCell<HashMap<String, Vec<u8>>>,
    get_to_file_error_sequences: RefCell<HashMap<String, VecDeque<ErrorKind>>>,
    get_to_file_calls: RefCell<Vec<(String, bool)>>,
    mirror_files: RefCell<HashMap<String, Vec<u8>>>,
    mirror_redirects: RefCell<HashMap<String, String>>,
    mirror_errors: RefCell<HashMap<String, ErrorKind>>,
    mirror_error_writes: RefCell<HashMap<String, Vec<u8>>>,
    mirror_calls: RefCell<Vec<(String, bool)>>,
    get_bytes_calls: RefCell<Vec<(String, bool)>>,
    post_calls: RefCell<Vec<(String, String, bool)>>,
    delete_calls: RefCell<Vec<(String, bool)>>,
    delete_json_calls: RefCell<Vec<(String, String, bool)>>,
    patch_calls: RefCell<Vec<(String, String, bool)>>,
    mutation_calls: RefCell<Vec<(String, String)>>,
    put_calls: RefCell<Vec<(String, String, bool)>>,
    put_bytes: RefCell<HashMap<String, String>>,
    put_bytes_calls: RefCell<Vec<(String, Vec<u8>, bool)>>,
}

impl FakeHttp {
    pub(crate) fn new() -> Self {
        FakeHttp {
            gets: RefCell::new(HashMap::new()),
            get_sequences: RefCell::new(HashMap::new()),
            get_errors: RefCell::new(HashMap::new()),
            get_error_sequences: RefCell::new(HashMap::new()),
            posts: RefCell::new(HashMap::new()),
            post_sequences: RefCell::new(HashMap::new()),
            post_errors: RefCell::new(HashMap::new()),
            patches: RefCell::new(HashMap::new()),
            patch_errors: RefCell::new(HashMap::new()),
            deletes: RefCell::new(HashMap::new()),
            delete_errors: RefCell::new(HashMap::new()),
            puts: RefCell::new(HashMap::new()),
            put_errors: RefCell::new(HashMap::new()),
            get_calls: RefCell::new(Vec::new()),
            get_bearer_tokens: RefCell::new(Vec::new()),
            get_bytes: RefCell::new(HashMap::new()),
            get_bytes_error_sequences: RefCell::new(HashMap::new()),
            get_bytes_cap_errors: RefCell::new(HashSet::new()),
            get_to_file: RefCell::new(HashMap::new()),
            get_to_file_errors: RefCell::new(HashMap::new()),
            get_to_file_error_writes: RefCell::new(HashMap::new()),
            get_to_file_error_sequences: RefCell::new(HashMap::new()),
            get_to_file_calls: RefCell::new(Vec::new()),
            mirror_files: RefCell::new(HashMap::new()),
            mirror_redirects: RefCell::new(HashMap::new()),
            mirror_errors: RefCell::new(HashMap::new()),
            mirror_error_writes: RefCell::new(HashMap::new()),
            mirror_calls: RefCell::new(Vec::new()),
            get_bytes_calls: RefCell::new(Vec::new()),
            post_calls: RefCell::new(Vec::new()),
            delete_calls: RefCell::new(Vec::new()),
            delete_json_calls: RefCell::new(Vec::new()),
            patch_calls: RefCell::new(Vec::new()),
            mutation_calls: RefCell::new(Vec::new()),
            put_calls: RefCell::new(Vec::new()),
            put_bytes: RefCell::new(HashMap::new()),
            put_bytes_calls: RefCell::new(Vec::new()),
        }
    }

    pub(crate) fn with_get(self, url: &str, body: &str) -> Self {
        self.gets
            .borrow_mut()
            .insert(url.to_string(), body.to_string());
        self
    }

    /// Answer successive GETs of `url` with `bodies` in order;
    /// once the queue is empty, the plain canned response (if
    /// any) answers.
    pub(crate) fn with_get_sequence(self, url: &str, bodies: &[&str]) -> Self {
        self.get_sequences.borrow_mut().insert(
            url.to_string(),
            bodies.iter().map(|body| body.to_string()).collect(),
        );
        self
    }

    pub(crate) fn with_get_bytes(self, url: &str, body: &[u8]) -> Self {
        self.get_bytes
            .borrow_mut()
            .insert(url.to_string(), body.to_vec());
        self
    }

    pub(crate) fn with_get_error(self, url: &str, kind: ErrorKind) -> Self {
        self.get_errors.borrow_mut().insert(url.to_string(), kind);
        self
    }

    /// Answer the next GETs of `url` with these errors, in order;
    /// once the queue is empty the canned `gets` body answers. The
    /// GET twin of `with_post_error_sequence`: lets a test model a
    /// resource that does not exist yet and then appears (a fork
    /// pre-check 404 followed by the re-read 200).
    pub(crate) fn with_get_error_sequence(self, url: &str, kinds: &[ErrorKind]) -> Self {
        self.get_error_sequences
            .borrow_mut()
            .insert(url.to_string(), kinds.iter().copied().collect());
        self
    }

    /// Answer the next byte GETs of `url` with these errors, in order;
    /// once the queue is empty the canned `get_bytes` body answers.
    /// The byte twin of `with_get_error_sequence`: lets a test model
    /// a download that does not exist yet and then appears (a
    /// download-on-demand binary fetched after the versions listing
    /// warms it).
    pub(crate) fn with_get_bytes_error_sequence(self, url: &str, kinds: &[ErrorKind]) -> Self {
        self.get_bytes_error_sequences
            .borrow_mut()
            .insert(url.to_string(), kinds.iter().copied().collect());
        self
    }

    /// Answer byte GETs of `url` with the transport's over-cap
    /// rejection: the response body exceeds the `MAX_BODY_BYTES`
    /// limit. Models a body the real transport refuses, so a caller
    /// that falls back to streaming can be tested without serving a
    /// 10 MiB body.
    pub(crate) fn with_get_bytes_cap_error(self, url: &str) -> Self {
        self.get_bytes_cap_errors
            .borrow_mut()
            .insert(url.to_string());
        self
    }

    /// Answer streamed downloads of `url` with `body`: the bytes the
    /// fake writes to the destination file.
    pub(crate) fn with_get_to_file(self, url: &str, body: &[u8]) -> Self {
        self.get_to_file
            .borrow_mut()
            .insert(url.to_string(), body.to_vec());
        self
    }

    /// Answer streamed downloads of `url` with this error after
    /// writing `body` first: models a stream that fails mid-download,
    /// leaving a partial file for the caller to delete. An
    /// `AlreadyExists` error writes nothing: a server 409 never
    /// creates the file.
    pub(crate) fn with_get_to_file_error_after_write(
        self,
        url: &str,
        body: &[u8],
        kind: ErrorKind,
    ) -> Self {
        self.get_to_file_error_writes
            .borrow_mut()
            .insert(url.to_string(), body.to_vec());
        self.get_to_file_errors
            .borrow_mut()
            .insert(url.to_string(), kind);
        self
    }

    /// Answer the next streamed downloads of `url` with these errors,
    /// in order; once the queue is empty the canned `get_to_file` body
    /// answers. The streamed twin of `with_get_bytes_error_sequence`.
    /// An `AlreadyExists` entry models a temp-name collision: the
    /// destination is created, as if another downloader won the name
    /// between the name allocation and this open, and the error is
    /// returned. The caller retries with the next name and must not
    /// delete a file it did not create. One file per entry: the queue
    /// is popped, so a retry never recreates it.
    pub(crate) fn with_get_to_file_error_sequence(self, url: &str, kinds: &[ErrorKind]) -> Self {
        self.get_to_file_error_sequences
            .borrow_mut()
            .insert(url.to_string(), kinds.iter().copied().collect());
        self
    }

    /// Answer mirror downloads of `url` with `body`: the bytes the
    /// fake writes to the destination file. The mirror twin of
    /// `with_get_to_file`, for the single-shot redirect-following
    /// mirror download.
    pub(crate) fn with_mirror_file(self, url: &str, body: &[u8]) -> Self {
        self.mirror_files
            .borrow_mut()
            .insert(url.to_string(), body.to_vec());
        self
    }

    /// Answer mirror downloads of `url` with a redirect to
    /// `location`: the fake follows it through the same
    /// redirect-resolution as the real client, so a test can model
    /// the 302 a download mirror answers with.
    pub(crate) fn with_mirror_redirect(self, url: &str, location: &str) -> Self {
        self.mirror_redirects
            .borrow_mut()
            .insert(url.to_string(), location.to_string());
        self
    }

    /// Answer mirror downloads of `url` with this error: models a
    /// mirror 404, 5xx, or transport failure, answered once — the
    /// mirror download never retries, so the error is returned at
    /// once.
    pub(crate) fn with_mirror_error(self, url: &str, kind: ErrorKind) -> Self {
        self.mirror_errors
            .borrow_mut()
            .insert(url.to_string(), kind);
        self
    }

    /// Answer mirror downloads of `url` with this error after
    /// writing `body` first: models a mirror stream that fails
    /// mid-download, leaving a partial file for the caller to
    /// delete. The mirror twin of
    /// `with_get_to_file_error_after_write`.
    pub(crate) fn with_mirror_error_after_write(
        self,
        url: &str,
        body: &[u8],
        kind: ErrorKind,
    ) -> Self {
        self.mirror_error_writes
            .borrow_mut()
            .insert(url.to_string(), body.to_vec());
        self.mirror_errors
            .borrow_mut()
            .insert(url.to_string(), kind);
        self
    }

    pub(crate) fn with_post_error(self, url: &str, kind: ErrorKind) -> Self {
        self.post_errors
            .borrow_mut()
            .insert(url.to_string(), VecDeque::from([kind]));
        self
    }

    /// Answer the next POSTs to `url` with these errors, in order;
    /// once the queue is empty the canned `posts` body answers.
    /// The POST twin of `with_put_error_sequence`: lets a test
    /// model a backend that is not ready yet and then catches up.
    pub(crate) fn with_post_error_sequence(self, url: &str, kinds: &[ErrorKind]) -> Self {
        self.post_errors
            .borrow_mut()
            .insert(url.to_string(), kinds.iter().copied().collect());
        self
    }

    pub(crate) fn with_post(self, url: &str, body: &str) -> Self {
        self.posts
            .borrow_mut()
            .insert(url.to_string(), body.to_string());
        self
    }

    /// Answer successive POSTs of `url` with `bodies` in order;
    /// once the queue is empty, the plain canned response (if any)
    /// answers. Lets a test model a multi-pass exchange on one URL
    /// (a validate pass that reports missing files, then a clean
    /// re-validate after the stage).
    pub(crate) fn with_post_sequence(self, url: &str, bodies: &[&str]) -> Self {
        self.post_sequences.borrow_mut().insert(
            url.to_string(),
            bodies.iter().map(|body| body.to_string()).collect(),
        );
        self
    }

    pub(crate) fn with_patch(self, url: &str, body: &str) -> Self {
        self.patches
            .borrow_mut()
            .insert(url.to_string(), body.to_string());
        self
    }

    pub(crate) fn with_patch_error(self, url: &str, kind: ErrorKind) -> Self {
        self.patch_errors
            .borrow_mut()
            .insert(url.to_string(), VecDeque::from([kind]));
        self
    }

    pub(crate) fn with_delete(self, url: &str, body: &str) -> Self {
        self.deletes
            .borrow_mut()
            .insert(url.to_string(), body.to_string());
        self
    }

    pub(crate) fn with_delete_error(self, url: &str, kind: ErrorKind) -> Self {
        self.delete_errors
            .borrow_mut()
            .insert(url.to_string(), kind);
        self
    }

    pub(crate) fn with_put(self, url: &str, body: &str) -> Self {
        self.puts
            .borrow_mut()
            .insert(url.to_string(), body.to_string());
        self
    }

    /// Answer the next PUTs to `url` with these errors, in order;
    /// once the queue is empty the canned `puts` body answers.
    /// Lets a test model a backend that is not ready yet and then
    /// catches up, instead of a single static answer.
    /// Answer a byte PUT of `url` with `body` (the revision
    /// document the server returns after storing the file).
    pub(crate) fn with_put_bytes(self, url: &str, body: &str) -> Self {
        self.put_bytes
            .borrow_mut()
            .insert(url.to_string(), body.to_string());
        self
    }

    pub(crate) fn with_put_error_sequence(self, url: &str, kinds: &[ErrorKind]) -> Self {
        self.put_errors
            .borrow_mut()
            .insert(url.to_string(), kinds.iter().copied().collect());
        self
    }

    pub(crate) fn get_calls(&self) -> Vec<(String, bool)> {
        self.get_calls.borrow().clone()
    }

    /// The Bearer token each GET carried, in call order: (url,
    /// token). The `bool` in `get_calls` only says whether any
    /// auth was sent, so a test that two instances send different
    /// tokens asserts here — `None` when the call carried no
    /// Bearer token. Test-only fake values, never real secrets.
    pub(crate) fn get_bearer_tokens(&self) -> Vec<(String, Option<String>)> {
        self.get_bearer_tokens.borrow().clone()
    }

    pub(crate) fn get_bytes_calls(&self) -> Vec<(String, bool)> {
        self.get_bytes_calls.borrow().clone()
    }

    pub(crate) fn get_to_file_calls(&self) -> Vec<(String, bool)> {
        self.get_to_file_calls.borrow().clone()
    }

    /// The mirror downloads in order (url, authed): one entry per
    /// hop, so a test can assert a redirect was followed (two
    /// entries) or a dead mirror was tried once (one entry).
    pub(crate) fn mirror_calls(&self) -> Vec<(String, bool)> {
        self.mirror_calls.borrow().clone()
    }

    pub(crate) fn post_calls(&self) -> Vec<(String, String, bool)> {
        self.post_calls.borrow().clone()
    }

    pub(crate) fn delete_calls(&self) -> Vec<(String, bool)> {
        self.delete_calls.borrow().clone()
    }

    pub(crate) fn delete_json_calls(&self) -> Vec<(String, String, bool)> {
        self.delete_json_calls.borrow().clone()
    }

    pub(crate) fn patch_calls(&self) -> Vec<(String, String, bool)> {
        self.patch_calls.borrow().clone()
    }

    /// The mutation calls in order (method, url): POST, PATCH,
    /// PUT and DELETE all record here, so a test can assert one
    /// mutation happened before another - the decline comment
    /// before the close PATCH, for instance.
    pub(crate) fn mutation_calls(&self) -> Vec<(String, String)> {
        self.mutation_calls.borrow().clone()
    }

    pub(crate) fn put_calls(&self) -> Vec<(String, String, bool)> {
        self.put_calls.borrow().clone()
    }

    pub(crate) fn put_bytes_calls(&self) -> Vec<(String, Vec<u8>, bool)> {
        self.put_bytes_calls.borrow().clone()
    }

    fn patch(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        // Fixtures do not distinguish content types: the transport
        // tests in `http.rs` assert the header PATCH sends.
        let authed = !matches!(credentials, Credentials::None);
        self.patch_calls
            .borrow_mut()
            .push((url.to_string(), body.to_string(), authed));
        self.mutation_calls
            .borrow_mut()
            .push(("PATCH".to_string(), url.to_string()));
        if let Some(kind) = self
            .patch_errors
            .borrow_mut()
            .get_mut(url)
            .and_then(|queue| queue.pop_front())
        {
            return Err(GeectlError::new(
                kind,
                format!("canned PATCH error for {url}"),
            ));
        }
        self.patches.borrow().get(url).cloned().ok_or_else(|| {
            GeectlError::new(
                ErrorKind::NetworkError,
                format!("no canned PATCH response for {url}"),
            )
        })
    }
}

impl HttpClient for FakeHttp {
    fn get(&self, url: &str) -> Result<String, GeectlError> {
        self.get_with(url, &Credentials::None)
    }

    fn get_with(&self, url: &str, credentials: &Credentials) -> Result<String, GeectlError> {
        let authed = !matches!(credentials, Credentials::None);
        self.get_calls.borrow_mut().push((url.to_string(), authed));
        let bearer = match credentials {
            Credentials::Bearer { token } => Some(token.clone()),
            _ => None,
        };
        self.get_bearer_tokens
            .borrow_mut()
            .push((url.to_string(), bearer));
        if let Some(kind) = self
            .get_error_sequences
            .borrow_mut()
            .get_mut(url)
            .and_then(|queue| queue.pop_front())
        {
            return Err(GeectlError::new(
                kind,
                format!("canned GET error for {url}"),
            ));
        }
        if let Some(kind) = self.get_errors.borrow().get(url).cloned() {
            return Err(GeectlError::new(
                kind,
                format!("canned GET error for {url}"),
            ));
        }
        if let Some(body) = self
            .get_sequences
            .borrow_mut()
            .get_mut(url)
            .and_then(|queue| queue.pop_front())
        {
            return Ok(body);
        }
        self.gets.borrow().get(url).cloned().ok_or_else(|| {
            GeectlError::new(
                ErrorKind::NetworkError,
                format!("no canned GET response for {url}"),
            )
        })
    }

    fn get_bytes(&self, url: &str, credentials: &Credentials) -> Result<Vec<u8>, GeectlError> {
        let authed = !matches!(credentials, Credentials::None);
        self.get_bytes_calls
            .borrow_mut()
            .push((url.to_string(), authed));
        if self.get_bytes_cap_errors.borrow().contains(url) {
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("response body exceeds the {MAX_BODY_BYTES}-byte limit"),
            ));
        }
        if let Some(kind) = self
            .get_bytes_error_sequences
            .borrow_mut()
            .get_mut(url)
            .and_then(|queue| queue.pop_front())
        {
            return Err(GeectlError::new(
                kind,
                format!("canned GET error for {url}"),
            ));
        }
        if let Some(kind) = self.get_errors.borrow().get(url).cloned() {
            return Err(GeectlError::new(
                kind,
                format!("canned GET error for {url}"),
            ));
        }
        self.get_bytes.borrow().get(url).cloned().ok_or_else(|| {
            GeectlError::new(
                ErrorKind::NetworkError,
                format!("no canned GET bytes response for {url}"),
            )
        })
    }

    fn get_to_file(
        &self,
        url: &str,
        credentials: &Credentials,
        path: &std::path::Path,
    ) -> Result<u64, GeectlError> {
        let authed = !matches!(credentials, Credentials::None);
        self.get_to_file_calls
            .borrow_mut()
            .push((url.to_string(), authed));
        if let Some(kind) = self
            .get_to_file_error_sequences
            .borrow_mut()
            .get_mut(url)
            .and_then(|queue| queue.pop_front())
        {
            if kind == ErrorKind::AlreadyExists {
                // A temp-name collision: the destination was created
                // by another downloader between the name allocation
                // and this open. The file is created once, by this
                // entry only, and left in place: the caller retries
                // with the next name and must not delete a file it
                // did not create.
                let _ = std::fs::File::options()
                    .write(true)
                    .create_new(true)
                    .open(path);
            }
            return Err(GeectlError::new(
                kind,
                format!("canned GET error for {url}"),
            ));
        }
        if let Some(kind) = self.get_to_file_errors.borrow().get(url).cloned() {
            // A server 409 reports `AlreadyExists` without creating
            // the file: writing a partial first would make the caller
            // read the error as a temp-name collision and retry,
            // creating a fresh temp file on every attempt.
            let partial = if kind == ErrorKind::AlreadyExists {
                None
            } else {
                self.get_to_file_error_writes.borrow().get(url).cloned()
            };
            if let Some(partial) = partial {
                // The partial file is created the way the real client
                // creates it: exclusively. A pre-existing destination
                // is an `AlreadyExists` error, not a truncation.
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
                std::io::Write::write_all(&mut file, &partial).map_err(|e| {
                    GeectlError::new(
                        ErrorKind::InternalError,
                        format!("cannot write download file {}: {e}", path.display()),
                    )
                })?;
            }
            return Err(GeectlError::new(
                kind,
                format!("canned GET error for {url}"),
            ));
        }
        let body = self.get_to_file.borrow().get(url).cloned().ok_or_else(|| {
            GeectlError::new(
                ErrorKind::NetworkError,
                format!("no canned GET bytes response for {url}"),
            )
        })?;
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
        std::io::Write::write_all(&mut file, &body).map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot write download file {}: {e}", path.display()),
            )
        })?;
        Ok(body.len() as u64)
    }

    fn get_to_file_mirror(
        &self,
        url: &str,
        credentials: &Credentials,
        path: &std::path::Path,
        max_bytes: u64,
    ) -> Result<u64, GeectlError> {
        // The redirect chain runs through the same
        // `follow_mirror_redirects` as the real client, so a canned
        // redirect is resolved by the same rules (same scheme, no
        // userinfo, query, or fragment) and the same hop limit.
        // Each hop records its call, like `get_to_file` — and like
        // the real client, only the first hop records credentials.
        let mut first_hop = true;
        follow_mirror_redirects(url, |current| {
            let authed = if first_hop {
                first_hop = false;
                !matches!(credentials, Credentials::None)
            } else {
                false
            };
            self.mirror_calls
                .borrow_mut()
                .push((current.to_string(), authed));
            if let Some(kind) = self.mirror_errors.borrow().get(current).copied() {
                if let Some(partial) = self.mirror_error_writes.borrow().get(current).cloned() {
                    // A stream that fails mid-download leaves a
                    // partial file for the caller to delete, like
                    // the real client: created exclusively, so a
                    // pre-existing destination is an error, not a
                    // truncation.
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
                    std::io::Write::write_all(&mut file, &partial).map_err(|e| {
                        GeectlError::new(
                            ErrorKind::InternalError,
                            format!("cannot write download file {}: {e}", path.display()),
                        )
                    })?;
                }
                return Err(GeectlError::new(
                    kind,
                    format!("canned mirror error for {current}"),
                ));
            }
            if let Some(location) = self.mirror_redirects.borrow().get(current).cloned() {
                return Ok(MirrorHop::Redirect(location));
            }
            let body = self
                .mirror_files
                .borrow()
                .get(current)
                .cloned()
                .ok_or_else(|| {
                    GeectlError::new(
                        ErrorKind::NetworkError,
                        format!("no canned mirror response for {current}"),
                    )
                })?;
            if body.len() as u64 > max_bytes {
                return Err(GeectlError::new(
                    ErrorKind::ServerError,
                    format!("mirror download of {current} exceeds the {max_bytes}-byte cap"),
                ));
            }
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
            std::io::Write::write_all(&mut file, &body).map_err(|e| {
                GeectlError::new(
                    ErrorKind::InternalError,
                    format!("cannot write download file {}: {e}", path.display()),
                )
            })?;
            Ok(MirrorHop::Downloaded(body.len() as u64))
        })
    }

    fn post(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        let authed = !matches!(credentials, Credentials::None);
        self.post_calls
            .borrow_mut()
            .push((url.to_string(), body.to_string(), authed));
        self.mutation_calls
            .borrow_mut()
            .push(("POST".to_string(), url.to_string()));
        if let Some(kind) = self
            .post_errors
            .borrow_mut()
            .get_mut(url)
            .and_then(|queue| queue.pop_front())
        {
            return Err(GeectlError::new(
                kind,
                format!("canned POST error for {url}"),
            ));
        }
        if let Some(body) = self
            .post_sequences
            .borrow_mut()
            .get_mut(url)
            .and_then(|queue| queue.pop_front())
        {
            return Ok(body);
        }
        self.posts.borrow().get(url).cloned().ok_or_else(|| {
            GeectlError::new(
                ErrorKind::NetworkError,
                format!("no canned POST response for {url}"),
            )
        })
    }

    fn post_json(&self, url: &str, body: &str) -> Result<String, GeectlError> {
        // The fake has no auth seam, so post_json resolves to no
        // credentials — the same delegation as post_text. Fixtures
        // do not distinguish content types: the transport tests in
        // `http.rs` assert the header each method sends.
        self.post(url, body, &Credentials::None)
    }

    fn post_json_with(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        // The explicit-credentials twin of post_json: the recorded
        // call carries whether the caller authenticated, so a test
        // can assert the fork POST went out as the client's Bearer
        // instead of as no credentials at all.
        self.post(url, body, credentials)
    }

    fn put_json_with(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        // Fixtures do not distinguish content types: the transport
        // tests in `http.rs` assert the header each method sends.
        // The recorded PUT call carries the body, so a test can
        // assert the collaborator PUT carried the permission.
        self.put(url, body, credentials)
    }

    fn patch_json_with(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        // Fixtures do not distinguish content types: the transport
        // tests in `http.rs` assert the header each method sends.
        // The recorded PATCH call carries the body, so a test can
        // assert the close PATCH carried the state.
        self.patch(url, body, credentials)
    }

    fn post_text(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        // Fixtures do not distinguish content types: the transport
        // tests in `http.rs` assert the header each method sends.
        self.post(url, body, credentials)
    }

    fn delete(&self, url: &str, credentials: &Credentials) -> Result<String, GeectlError> {
        let authed = !matches!(credentials, Credentials::None);
        self.delete_calls
            .borrow_mut()
            .push((url.to_string(), authed));
        self.mutation_calls
            .borrow_mut()
            .push(("DELETE".to_string(), url.to_string()));
        if let Some(kind) = self.delete_errors.borrow().get(url).cloned() {
            return Err(GeectlError::new(
                kind,
                format!("canned DELETE error for {url}"),
            ));
        }
        self.deletes.borrow().get(url).cloned().ok_or_else(|| {
            GeectlError::new(
                ErrorKind::NetworkError,
                format!("no canned DELETE response for {url}"),
            )
        })
    }

    fn delete_json_with(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        // The JSON-body twin of delete: the recorded call carries
        // the body, so a test can assert the cancel DELETE carried
        // the reviewer list.
        let authed = !matches!(credentials, Credentials::None);
        self.delete_json_calls
            .borrow_mut()
            .push((url.to_string(), body.to_string(), authed));
        self.deletes.borrow().get(url).cloned().ok_or_else(|| {
            GeectlError::new(
                ErrorKind::NetworkError,
                format!("no canned DELETE response for {url}"),
            )
        })
    }

    fn put(&self, url: &str, body: &str, credentials: &Credentials) -> Result<String, GeectlError> {
        // Fixtures do not distinguish content types: the transport
        // tests in `http.rs` assert the header PUT sends.
        let authed = !matches!(credentials, Credentials::None);
        self.put_calls
            .borrow_mut()
            .push((url.to_string(), body.to_string(), authed));
        self.mutation_calls
            .borrow_mut()
            .push(("PUT".to_string(), url.to_string()));
        if let Some(kind) = self
            .put_errors
            .borrow_mut()
            .get_mut(url)
            .and_then(|queue| queue.pop_front())
        {
            return Err(GeectlError::new(
                kind,
                format!("canned PUT error for {url}"),
            ));
        }
        self.puts.borrow().get(url).cloned().ok_or_else(|| {
            GeectlError::new(
                ErrorKind::NetworkError,
                format!("no canned PUT response for {url}"),
            )
        })
    }

    fn put_file(
        &self,
        url: &str,
        body: &str,
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        // Fixtures do not distinguish content types: the transport
        // tests in `http.rs` assert the header each method sends.
        self.put(url, body, credentials)
    }

    fn put_bytes(
        &self,
        url: &str,
        body: &[u8],
        credentials: &Credentials,
    ) -> Result<String, GeectlError> {
        // Fixtures do not distinguish content types: the transport
        // tests in `http.rs` assert the header each method sends.
        // The body IS recorded as bytes, so a test can assert the
        // exact bytes a binary upload carried.
        let authed = !matches!(credentials, Credentials::None);
        self.put_bytes_calls
            .borrow_mut()
            .push((url.to_string(), body.to_vec(), authed));
        // A byte PUT is a PUT: the same canned error sequence
        // applies as for a text PUT.
        if let Some(kind) = self
            .put_errors
            .borrow_mut()
            .get_mut(url)
            .and_then(|queue| queue.pop_front())
        {
            return Err(GeectlError::new(
                kind,
                format!("canned PUT error for {url}"),
            ));
        }
        self.put_bytes.borrow().get(url).cloned().ok_or_else(|| {
            GeectlError::new(
                ErrorKind::NetworkError,
                format!("no canned PUT bytes response for {url}"),
            )
        })
    }
}

pub(crate) fn basic_credentials() -> Credentials {
    Credentials::Basic {
        username: "scarabeus_iv".to_string(),
        password: "secret".to_string(),
    }
}

pub(crate) fn payload() -> SrPayload {
    SrPayload::new(
            "home:scarabeus_iv",
            "foo",
            "devel:languages:python",
            "foo",
            "Update foo to 2.0",
            "-------------------------------------------------------------------\nFri Nov 20 07:00:00 UTC 2026 - Tomáš Chvátal <tomas.chvatal@gmail.com>\n\n- Update to version 2.0.\n",
            &[],
        )
        .unwrap()
}

pub(crate) fn search_without_states() -> RequestSearch {
    RequestSearch::new().states(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_file_over_the_cap_is_refused() {
        // A canned mirror body over the caller's cap fails instead
        // of landing in the file: the cap is enforced in the fake
        // the same way as in the real client.
        let http =
            FakeHttp::new().with_mirror_file("https://mirror.example.com/pkg.rpm", &[b'a'; 100]);
        let path = std::env::temp_dir().join(format!(
            "geectl-fake-mirror-over-cap-{}.rpm",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let err = http
            .get_to_file_mirror(
                "https://mirror.example.com/pkg.rpm",
                &Credentials::None,
                &path,
                10,
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("cap"), "{err:?}");
        assert!(!path.exists());
    }
}
