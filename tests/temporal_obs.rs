//! Live tests for the OBS operations against a temporal OBS instance.
//!
//! These are layer 2 of the testing design (see `docs/architecture.md`):
//! scheduled live tests against a real (temporary) OBS instance, asserting
//! on **server state** — whatever geectl reads must reflect what the
//! server actually holds. That is what catches drift as the server changes
//! underneath us, which the per-PR fixture tests cannot do.
//!
//! These tests are NOT part of the per-PR gate. They run only in the
//! scheduled `temporal` CI job, against the instance the provisioning in
//! `tests/temporal/` spins up. They are skipped unless the
//! `GEECTL_TEMPORAL_URL` environment variable is set to the instance's
//! base URL (e.g. `http://127.0.0.1:3000`), so `cargo test` stays green
//! without an instance. When `GEECTL_TEMPORAL_REQUIRED` is set, a missing
//! or empty URL panics instead (the temporal workflow sets that marker
//! and wires the URL, so its absence means the wiring broke).
//!
//! The instance is plain HTTP on localhost. That is the single sanctioned
//! exception to geectl's https-only URL rule, and it is safe because:
//! - the instance is loopback-only (the harness rejects any
//!   non-loopback URL outright — see `validate_temporal_base`),
//! - the instance is ephemeral (created fresh per run, destroyed after),
//! - the credentials are the ephemeral OBS development defaults
//!   (Admin / opensuse), never a real credential, and
//! - the public reads use no auth at all; the authenticated reads send
//!   only those ephemeral defaults to the same loopback-only instance.
//!
//! The harness talks to the instance through geectl's own `ObsClient`
//! (over `UreqClient`), with the instance's base URL — the client
//! accepts an `http://` loopback base for exactly this case, and the
//! URL builder's https-only rule is untouched (the OBS operations build
//! their URLs with the encoders from `url.rs`, as `docs/architecture.md`
//! records).
//!
//! The upload test (`temporal_upload_file_reads_back_on_server`)
//! uploads a spec file into a fresh branch package and reads it back,
//! asserting the bytes and the revision number.
//!
//! The remote-read tests (`temporal_cat_reads_back_uploaded_file`,
//! `temporal_list_files_and_packages_read_back`,
//! `temporal_package_log_newest_first_and_range`,
//! `temporal_revision_patch_diffs_revisions`,
//! `temporal_meta_reads_back`,
//! `temporal_search_finds_package_and_project`,
//! `temporal_owners_call_succeeds`, `temporal_whois_reads_admin`,
//! `temporal_link_target_reads_linkinfo`, and
//! `temporal_project_diff_classifies`) exercise the read operations
//! the same way: create through geectl's own write operations, then
//! read back through the new operations and assert on server state.
//! The log and link-target tests poll for up to a minute: the history
//! and the linkinfo are written by the backend, so they must not
//! assert the instant the upload returns.
//!
//! The SR tests file real submit requests against the instance: they
//! create a source branch and a target project first, through geectl's
//! own `ObsClient::create_project` / `create_package` with the
//! bootstrap admin credentials, then assert on the server state the
//! filing leaves behind — read back through the library's own
//! `ObsClient::show_request`, and changed through its
//! `accept_request` / `decline_request` / `revoke_request`,
//! `add_review`, comment, and search operations. Accepting is a
//! maintainer action, and the library exposes it for automation all
//! the same: the own-requests boundary (an agent accepts its own
//! requests unless explicitly authorised) is documented on
//! `ObsClient::change_state` in `src/obs.rs`, and the server stays
//! the authority on who may move a request.
use geectl::auth::Credentials;
use geectl::error::ErrorKind;
use geectl::http::{HttpClient, Timeout, UreqClient};
use geectl::obs::{
    DryRun, FilePatchStatus, HistoryEntry, ObsClient, PackageDiffStatus, Request, RequestAction,
    RequestSearch, RequestState, ReviewState, ReviewTarget, RevisionRange, SourceUpdate, SrFiling,
    SrPayload,
};

/// Whether `url` points at a loopback host.
///
/// The host is extracted exactly: the authority after `http://` up to
/// the next `/`, `?`, or `#`, minus any userinfo (the text before the
/// LAST `@` — a password may itself contain `@`), minus any `:port`,
/// minus IPv6 brackets. A prefix check would also accept
/// `http://127.0.0.1.evil.com`, which is not loopback, and without the
/// userinfo strip `http://127.0.0.1:80@evil.com` would be classified
/// loopback while ureq resolves the real host to evil.com. A bracketed
/// host counts only when the closing bracket is followed by the end of
/// the authority or a `:port`, so `http://[::1]evil.com` is not
/// loopback. The hostname comparison is case-insensitive (DNS), so
/// `LOCALHOST` is loopback.
fn is_loopback_url(url: &str) -> bool {
    let after_scheme = url.strip_prefix("http://").unwrap_or("");
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    let hostport = match authority.rfind('@') {
        Some(at) => &authority[at + 1..],
        None => authority,
    };
    // A bracketed host is only a bracketed host if the closing bracket
    // is followed by the end of the authority or a `:port`: without
    // that, `http://[::1]evil.com` would read as `::1` and be classified
    // loopback while the real host is `[::1]evil.com`. An unterminated
    // bracket is not a host at all.
    let host = if let Some(bracketed) = hostport.strip_prefix('[') {
        match bracketed.split_once(']') {
            Some((inside, rest)) if rest.is_empty() || rest.starts_with(':') => inside,
            _ => "",
        }
    } else {
        hostport.split(':').next().unwrap_or("")
    };
    // DNS is case-insensitive: `LOCALHOST` is loopback too. The
    // 127.0.0.0/8 range is loopback, not just 127.0.0.1 — the same
    // check `http.rs` uses (`IpAddr::is_loopback`), so the harness
    // and the transport agree on what loopback means.
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

/// Validate a temporal base URL: trim whitespace, strip trailing
/// slashes, require a bare base (no path, query, or fragment), and
/// require a loopback host. Returns the cleaned URL, or the reason it
/// is invalid. Pure, so it is unit-testable; `temporal_url` turns the
/// error into a panic at the boundary.
fn validate_temporal_base(raw: &str) -> Result<String, String> {
    // Trim, strip trailing slashes, and trim again: a space before a
    // trailing slash (`...:3000 /`) would otherwise survive the strip
    // and leave a trailing space in the base.
    let url = raw.trim().trim_end_matches('/').trim().to_string();
    // The scheme is checked first: the bare-base check below sees the
    // `//` in any `scheme://` URL, so a non-http URL such as
    // `https://127.0.0.1:3000` would otherwise be reported as a
    // bare-base violation instead of a scheme violation.
    if !url.starts_with("http://") {
        return Err(format!(
            "must be an http:// URL (the temporal instance is plain HTTP on loopback); got {raw}"
        ));
    }
    // The base must be bare (scheme + authority only): the tests
    // append paths themselves (`{base}/about`), so a base with a path,
    // query, or fragment would double the path or silently drop it.
    // Reject at the boundary with a clear message, not a confusing
    // loopback-assert failure downstream.
    let after_scheme = url.strip_prefix("http://").unwrap_or(&url);
    if after_scheme.contains(['/', '?', '#']) {
        return Err(format!(
            "must be a bare base URL (http://127.0.0.1:3000), with no path, query, or fragment; got {raw}"
        ));
    }
    if !is_loopback_url(&url) {
        return Err(format!(
            "must be a loopback URL (http://127.0.0.1, http://localhost, or http://[::1]); got {raw}"
        ));
    }
    Ok(url)
}

/// The temporal instance's base URL, from the environment.
///
/// Returns `None` (skip the test) when `GEECTL_TEMPORAL_URL` is unset or
/// empty. When `GEECTL_TEMPORAL_REQUIRED` is set, a missing or empty
/// value panics instead: the temporal workflow sets that marker and
/// wires the URL, so its absence means the wiring broke, and a silent
/// skip would go green while hiding drift. The marker (not `CI` or
/// `GITHUB_ACTIONS`) is the signal: the per-PR `test` job also runs
/// under CI, but with no temporal instance, so the tests must skip
/// there. A present but invalid value panics in both cases: the
/// instance is plain HTTP, so pointing the harness at a non-localhost
/// server would send requests (and, once auth lands, credentials) in
/// cleartext over a real network. That must fail fast, not silently.
fn temporal_url() -> Option<String> {
    let raw = match std::env::var("GEECTL_TEMPORAL_URL") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            if std::env::var_os("GEECTL_TEMPORAL_REQUIRED").is_some() {
                panic!(
                    "GEECTL_TEMPORAL_URL is not set or empty: the temporal workflow must wire it (see .github/workflows/temporal.yml)"
                );
            }
            eprintln!("skipping temporal test: GEECTL_TEMPORAL_URL is not set");
            return None;
        }
    };
    match validate_temporal_base(&raw) {
        Ok(url) => Some(url),
        Err(reason) => panic!("GEECTL_TEMPORAL_URL {reason}"),
    }
}

/// The temporal instance's bootstrap admin credentials (the OBS
/// development defaults, ephemeral, never a real credential).
/// Overridable via `GEECTL_TEMPORAL_OBS_USERNAME` /
/// `GEECTL_TEMPORAL_OBS_PASSWORD` if the provisioning ever changes
/// them.
fn temporal_credentials() -> Credentials {
    let username =
        std::env::var("GEECTL_TEMPORAL_OBS_USERNAME").unwrap_or_else(|_| "Admin".to_string());
    let password =
        std::env::var("GEECTL_TEMPORAL_OBS_PASSWORD").unwrap_or_else(|_| "opensuse".to_string());
    Credentials::Basic { username, password }
}

/// A client for the temporal instance, with no credentials (public
/// reads only).
fn temporal_client(base: &str) -> ObsClient<'_> {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    ObsClient::with_base(http, base).expect("the temporal base is a loopback http URL")
}

/// A client for the temporal instance, authenticated with the
/// bootstrap admin credentials.
fn temporal_client_authed(base: &str) -> ObsClient<'_> {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    ObsClient::with_base_and_credentials(http, base, temporal_credentials())
        .expect("the temporal base is a loopback http URL")
}

/// A suffix that makes a test's projects unique: the process id plus
/// a nanosecond timestamp. The temporal tests run in parallel in one
/// process, and the instance is shared with the `temporal` target's
/// tests, so two tests must never create the same project — a name
/// collision would file one test's request against another's branch.
/// Only digits and hyphens: a valid OBS name fragment.
fn unique_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time is after the epoch")
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

/// A package name unique to this test run. The projects already
/// differ per test, so the package entities never collide — but the
/// name must still be unique: the open-SR search is scoped by target
/// package name only, so a shared name lets one test's open request
/// block another test's filing (the refile test hit exactly that:
/// the filing test's open request targeted the same package name).
fn temporal_package() -> String {
    format!("geectl-temporal-pkg-{}", unique_suffix())
}

/// Create a uniquely-named source project plus the package in it.
/// Returns the source project name. The project is deliberately NOT
/// a `home:<user>` project: the accept-cleanup test asserts the
/// server deletes it once it is empty, and the server keeps
/// top-level `home:<user>` projects (see `SourceUpdate::Cleanup`).
///
/// The package creation goes through the library, which retries a
/// not-found for a bounded 30 s: the server's API and backend are
/// separate processes, and the API replies to the project creation
/// before the backend has the project registered — the immediately
/// following package creation can then 404 on the just-created
/// project (seen on run 44876734097). The retry lives in the library
/// so sequential consumers get it too; the harness no longer retries
/// around the call.
fn create_source_branch(client: &ObsClient<'_>, package: &str) -> String {
    let source_project = format!("geectl-tmp-src-{}", unique_suffix());
    client
        .create_project(
            &source_project,
            "geectl temporal test branch",
            "Created by the geectl temporal live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create source project on the temporal instance");
    client
        .create_package(
            &source_project,
            package,
            "geectl temporal test package",
            "Created by the geectl temporal live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create source package on the temporal instance");
    source_project
}

/// Create a source branch and a target project, all uniquely named
/// for this test run. Returns `(source_project, target_project,
/// package)`. The target project only needs to exist: the submitted
/// package is created in it on accept.
fn setup_branch(client: &ObsClient<'_>) -> (String, String, String) {
    let package = temporal_package();
    let source_project = create_source_branch(client, &package);
    let target_project = format!("geectl-tmp-tgt-{}", unique_suffix());
    client
        .create_project(
            &target_project,
            "geectl temporal test target",
            "Created by the geectl temporal live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create target project on the temporal instance");
    (source_project, target_project, package)
}

/// A submit-request payload for the temporal tests: source branch to
/// target, with the default cleanup (`opt_sourceupdate="cleanup"`),
/// so the accept test exercises the server-side branch deletion.
fn temporal_payload(
    source_project: &str,
    package: &str,
    target_project: &str,
    version: &str,
) -> SrPayload {
    SrPayload::new(
        source_project,
        package,
        target_project,
        package,
        &format!("Update {package} to {version}: temporal test submission"),
        &format!(
            "-------------------------------------------------------------------\nFri Nov 20 07:00:00 UTC 2026 - Tomáš Chvátal <tomas.chvatal@gmail.com>\n\n- Update to version {version}.\n"
        ),
        &[],
    )
    .expect("temporal SR payload")
}

/// The submit action of a filed request: its source project and
/// package, its target project and package (when the action names
/// them), and what the request does with the source branch on
/// accept. Panics naming the request when it carries no submit
/// action — the live tests file submits, so a missing one means the
/// filing did not do what the test asked.
fn submit_action(
    request: &Request,
) -> (&str, &str, Option<&str>, Option<&str>, Option<SourceUpdate>) {
    let Some(RequestAction::Submit {
        source_project,
        source_package,
        target_project,
        target_package,
        sourceupdate,
        ..
    }) = request.actions.first()
    else {
        panic!("request {} must carry a submit action", request.id);
    };
    (
        source_project,
        source_package,
        target_project.as_deref(),
        target_package.as_deref(),
        *sourceupdate,
    )
}

/// File a submit request on the temporal instance and return its id
/// and package. Each call files from a fresh, uniquely-named branch
/// (see `setup_branch`), so parallel tests never share a target
/// package name — the open-SR search is scoped by target package
/// name only.
fn file_temporal_request(client: &ObsClient<'_>, version: &str) -> (String, String) {
    let (source_project, target_project, package) = setup_branch(client);
    let dry = DryRun::new(false);
    let filing = SrFiling::new(client, &dry);
    let payload = temporal_payload(&source_project, &package, &target_project, version);
    let id = filing
        .file(&payload)
        .expect("file a submit request on the temporal instance")
        .expect("live filing returns a request id");
    (id, package)
}

/// File a submit request from a branch holding one file: the diff of
/// an empty submit fails on the server (the backend diff has nothing
/// to compute from, and the unrescued diff error renders as 404), so
/// the diff test files from a branch with content. The other tests
/// keep the empty branch — they never ask the server for a diff.
fn file_temporal_request_with_file(
    client: &ObsClient<'_>,
    version: &str,
    filename: &str,
    content: &str,
) -> (String, String) {
    let (source_project, target_project, package) = setup_branch(client);
    let dry = DryRun::new(false);
    client
        .upload_file(&source_project, &package, filename, content, &dry)
        .expect("upload a file into the source branch on the temporal instance");
    let filing = SrFiling::new(client, &dry);
    let payload = temporal_payload(&source_project, &package, &target_project, version);
    let id = filing
        .file(&payload)
        .expect("file a submit request on the temporal instance")
        .expect("live filing returns a request id");
    (id, package)
}

/// The value of attribute `attr` in the first element starting at
/// `marker` (e.g. `<state name="new">` via marker `<state `, attr
/// `name`). None when the marker or the attribute is absent.
/// Attribute order does not matter: the search starts at the marker,
/// not at a fixed offset. The marker must end in a space (or the
/// attribute itself): `<source ` does not match `<sourceupdate>`.
fn xml_attr_after(body: &str, marker: &str, attr: &str) -> Option<String> {
    let after = body.split_once(marker)?.1;
    let needle = format!("{attr}=\"");
    let value = after.split_once(&needle)?.1;
    Some(value.split('"').next()?.to_string())
}

/// The request's state name (`new`, `superseded`, `accepted`, ...):
/// the first `<state name="...">` in the document, which is the
/// request's current state.
fn request_state(body: &str) -> Option<String> {
    xml_attr_after(body, "<state ", "name")
}

/// The raw request XML for `id`, read back from the server. The
/// library has no read-a-request operation (an agent files and
/// supersedes; reading one back is test verification), so the harness
/// reads it through geectl's own HTTP client with the admin
/// credentials.
fn request_xml(base: &str, id: &str) -> String {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    http.get_with(&format!("{base}/request/{id}"), &temporal_credentials())
        .expect("GET the filed request back from the server")
}

/// The request's state, read back best-effort: `None` when the read
/// itself fails or the body carries no state. The accept retry uses
/// this instead of `request_xml`, because the same flaky backend
/// that 500s the changestate POST can 500 the follow-up GET — a
/// panicking read there would kill the retry and lose the original
/// POST error. A failed read is just "state unknown", and the retry
/// keeps going until the deadline, which still fails loudly.
fn request_state_or_none(base: &str, id: &str) -> Option<String> {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    let body = http
        .get_with(&format!("{base}/request/{id}"), &temporal_credentials())
        .ok()?;
    request_state(&body)
}

/// True when `body` is a `<status code="ok"/>` document. The harness
/// cannot use the crate-internal `parse_status_ok` (integration tests
/// cannot see `pub(crate)` items), so it carries this small check:
/// the `<status` tag must carry the `code="ok"` attribute.
fn response_is_status_ok(body: &str) -> bool {
    let Some(pos) = body.find("<status") else {
        return false;
    };
    let after_tag = &body[pos + "<status".len()..];
    let first = match after_tag.chars().next() {
        Some(c) => c,
        None => return false,
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return false;
    }
    let Some(end) = after_tag.find('>') else {
        return false;
    };
    // The token must be exactly the attribute: `code="okx"` must not
    // match, so a bare prefix check is not enough.
    after_tag[..end].split_whitespace().any(|token| {
        token == "code=\"ok\""
            || token.starts_with("code=\"ok\"/")
            || token.starts_with("code=\"ok\">")
    })
}

/// Change the state of one review on a request, as the admin, by
/// driving the raw `changereviewstate` command: the library has no
/// review-state operation in this batch, so this is harness-only
/// setup for the readback test, not a library operation. The comment
/// travels as the POST body, the same form the change-state command
/// uses.
fn change_review_state(base: &str, id: &str, newstate: &str, by_user: &str) {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    let body = http
        .post_text(
            &format!(
                "{base}/request/{id}?cmd=changereviewstate&newstate={newstate}&by_user={by_user}"
            ),
            "accepted by the geectl temporal test",
            &temporal_credentials(),
        )
        .expect("change the review state on the temporal instance");
    assert!(
        response_is_status_ok(&body),
        "changing the review state of request {id} did not return <status code=\"ok\"/>; body: {body}"
    );
}

/// The accept retries on a failed POST for a bounded time: accepting
/// runs the server's backend copy and cleanup, and on a fresh
/// instance those can fail while the backend is still registering
/// the just-created source (seen on run 44880250442: the changestate
/// POST returned 500, while the same accept passed on earlier runs).
/// A retry first checks whether the request already reads back as
/// accepted — the 500 can come after the state change landed — so a
/// mutation that already went through is never re-issued blindly.
/// The retry is bounded (60 s); a request that never accepts still
/// fails loudly instead of hanging. Only transient failures are
/// retried — a 5xx, a timeout, a network error, or a not-found (the
/// just-filed request is not visible yet while the backend registers
/// it, the same race as the package-create retry). A deterministic
/// failure (bad credentials, a bad request id, an invalid state)
/// fails fast instead of burning the whole minute.
fn accept_request(base: &str, id: &str) {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        match http.post_text(
            &format!("{base}/request/{id}?cmd=changestate&newstate=accepted"),
            "accepted by the geectl temporal test",
            &temporal_credentials(),
        ) {
            Ok(body) => {
                assert!(
                    response_is_status_ok(&body),
                    "accepting request {id} did not return <status code=\"ok\"/>; body: {body}"
                );
                return;
            }
            Err(err) => {
                if !matches!(
                    err.kind(),
                    ErrorKind::ServerError
                        | ErrorKind::Timeout
                        | ErrorKind::NetworkError
                        | ErrorKind::NotFound
                ) {
                    panic!("accept the request on the temporal instance: {err:?}");
                }
                if request_state_or_none(base, id).as_deref() == Some("accepted") {
                    return;
                }
                if std::time::Instant::now() >= deadline {
                    panic!("accept the request on the temporal instance: {err:?}");
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
    }
}

/// True when the public directory listing for `path` (`project`, or
/// `project/package`) answers 200. A 404 reads as false (the path is
/// gone); anything else is a failure, not an answer. The reads are
/// public, so no credentials.
fn public_path_exists(base: &str, path: &str) -> bool {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    match http.get(&format!("{base}/public/source/{path}")) {
        Ok(_) => true,
        Err(err) if err.kind() == ErrorKind::NotFound => false,
        Err(err) => panic!("GET /public/source/{path} failed unexpectedly: {err}"),
    }
}

/// Poll until the public path is gone, for up to a minute: the accept
/// cleanup deletes the source package — and the empty source project
/// — but the deletion runs through the backend, so the test must not
/// assert the instant the accept returns. Panics naming what is
/// still there when the deadline passes.
fn wait_until_gone(base: &str, path: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        if !public_path_exists(base, path) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    panic!("{path} still exists on the server 60s after the accept");
}

/// Poll until the public path exists, for up to a minute: the target
/// package lands through the backend on accept, like the cleanup.
/// Panics naming what is still missing when the deadline passes.
fn wait_until_present(base: &str, path: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        if public_path_exists(base, path) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    panic!("{path} is still missing on the server 60s after the accept");
}

#[test]
fn temporal_obs_devel_project_missing_is_none() {
    let Some(base) = temporal_url() else {
        return;
    };
    // A package that was never created has no devel project on the
    // instance: the lookup must return None from the server's own
    // directory listing, not from a fixture.
    let client = temporal_client(&base);
    assert_eq!(
        client
            .devel_project("geectl-temporal-no-such-package")
            .expect("devel lookup on the temporal instance"),
        None
    );
}

#[test]
fn temporal_obs_has_open_sr_false_on_fresh_instance() {
    let Some(base) = temporal_url() else {
        return;
    };
    // A fresh instance holds no submit requests at all, so the dedup
    // search must return false — and the search must succeed against
    // the real server with the bootstrap admin credentials, not 401.
    let client = temporal_client_authed(&base);
    assert!(
        !client
            .has_open_sr("geectl-temporal-no-such-package")
            .expect("open-SR search on the temporal instance"),
        "a fresh temporal instance must have no open submit request"
    );
}

/// Filing a real submit request and reading it back from the server.
#[test]
fn temporal_filed_request_shows_up_on_server() {
    let Some(base) = temporal_url() else {
        return;
    };
    let client = temporal_client_authed(&base);
    let (source_project, target_project, package) = setup_branch(&client);
    let dry = DryRun::new(false);
    let filing = SrFiling::new(&client, &dry);
    let payload = temporal_payload(&source_project, &package, &target_project, "2.0");
    let id = filing
        .file(&payload)
        .expect("file a submit request on the temporal instance")
        .expect("live filing returns a request id");
    let request = client
        .show_request(&id)
        .expect("read the filed request back from the server");
    // The filed request is open, from the branch, to the target, and
    // carries the default cleanup: whatever the server holds must be
    // what was filed.
    assert_eq!(request.id.to_string(), id);
    assert_eq!(request.creator, "Admin");
    assert_eq!(request.state.name, RequestState::New);
    assert_eq!(
        request.description,
        format!("Update {package} to 2.0: temporal test submission")
    );
    let (src_project, src_package, tgt_project, tgt_package, sourceupdate) =
        submit_action(&request);
    assert_eq!(src_project, source_project);
    assert_eq!(src_package, package);
    assert_eq!(tgt_project, Some(target_project.as_str()));
    assert_eq!(tgt_package, Some(package.as_str()));
    assert_eq!(sourceupdate, Some(SourceUpdate::Cleanup));
}

#[test]
fn temporal_obs_open_requests_for_package_empty_on_fresh_instance() {
    let Some(base) = temporal_url() else {
        return;
    };
    // A fresh instance holds no requests at all, so the search for
    // requests involving a never-created package must return an empty
    // list — and the search must succeed against the real server with
    // the bootstrap admin credentials, not 401. This exercises the
    // delete pre-check's search URL and response parsing against
    // server state, not a fixture.
    let client = temporal_client_authed(&base);
    assert!(
        client
            .open_requests_for_package(
                "geectl-temporal-no-such-project",
                "geectl-temporal-no-such-package"
            )
            .expect("involved-requests search on the temporal instance")
            .is_empty(),
        "a fresh temporal instance must have no requests involving the package"
    );
}

/// The full refile lifecycle against the server: file, supersede,
/// delete.
#[test]
fn temporal_refile_supersedes_and_deletes_on_server() {
    let Some(base) = temporal_url() else {
        return;
    };
    let client = temporal_client_authed(&base);
    let (source_a, target_project, package) = setup_branch(&client);
    let dry = DryRun::new(false);
    let filing = SrFiling::new(&client, &dry);
    let id1 = filing
        .file(&temporal_payload(
            &source_a,
            &package,
            &target_project,
            "2.0",
        ))
        .expect("file the first submit request on the temporal instance")
        .expect("live filing returns a request id");
    // The refile comes from a second branch, same target.
    let source_b = create_source_branch(&client, &package);
    let id2 = filing
        .refile(
            &temporal_payload(&source_b, &package, &target_project, "2.1"),
            &id1,
        )
        .expect("refile on the temporal instance")
        .expect("live refile returns a request id");
    assert_ne!(id1, id2, "refile must return a new request id");
    let old = client
        .show_request(&id1)
        .expect("read the old request back from the server");
    assert_eq!(
        old.state.name,
        RequestState::Superseded,
        "the old request {id1} must be superseded"
    );
    assert_eq!(
        old.state.superseded_by,
        Some(id2.parse().expect("the new request id is a number")),
        "the old request {id1} must point at the new one"
    );
    let new = client
        .show_request(&id2)
        .expect("read the new request back from the server");
    assert_eq!(
        new.state.name,
        RequestState::New,
        "the new request {id2} must be new"
    );
    // The old branch deletes now that its request is closed...
    client
        .delete_package(&source_a, &package, false, &dry)
        .expect("delete the superseded branch");
    assert!(
        !public_path_exists(&base, &format!("{source_a}/{package}")),
        "the superseded branch {source_a}/{package} must be gone"
    );
    // ...but the new branch is still guarded while its request is open.
    let err = client
        .delete_package(&source_b, &package, false, &dry)
        .expect_err("deleting a branch with an open request must be refused");
    assert!(
        format!("{err}").contains(&id2),
        "the refusal must name the blocking request {id2}; got {err}"
    );
}

/// Accept-time cleanup: a request filed with the default cleanup
/// deletes its source branch when it lands.
#[test]
fn temporal_accept_deletes_source_branch_on_server() {
    let Some(base) = temporal_url() else {
        return;
    };
    let client = temporal_client_authed(&base);
    let (source_project, target_project, package) = setup_branch(&client);
    let dry = DryRun::new(false);
    let filing = SrFiling::new(&client, &dry);
    let payload = temporal_payload(&source_project, &package, &target_project, "2.0");
    let id = filing
        .file(&payload)
        .expect("file a submit request on the temporal instance")
        .expect("live filing returns a request id");
    let filed = request_xml(&base, &id);
    assert!(
        filed.contains("<sourceupdate>cleanup</sourceupdate>"),
        "filed request {id} must carry the default cleanup; body: {filed}"
    );
    accept_request(&base, &id);
    let accepted = request_xml(&base, &id);
    assert_eq!(
        request_state(&accepted).as_deref(),
        Some("accepted"),
        "request {id} must be accepted; body: {accepted}"
    );
    // The server deletes the source branch on accept: the package
    // first, then the project, which was its last package. The
    // source project is not a `home:<user>` project, so the cleanup
    // takes it too (see `SourceUpdate::Cleanup`). Both deletions run
    // through the backend, so both waits poll.
    wait_until_gone(&base, &format!("{source_project}/{package}"));
    wait_until_gone(&base, &source_project);
    // ...and the target package lands.
    wait_until_present(&base, &format!("{target_project}/{package}"));
}

/// The content of a file in a package, read back from the server. The
/// library has no read-a-file operation (an agent uploads files; reading
/// one back is test verification), so the harness reads it through
/// geectl's own HTTP client with the admin credentials.
fn file_content(base: &str, project: &str, package: &str, filename: &str) -> String {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    http.get_with(
        &format!("{base}/source/{project}/{package}/{filename}"),
        &temporal_credentials(),
    )
    .expect("GET the uploaded file back from the server")
}

/// Upload a file through geectl and read it back from the server,
/// asserting the bytes the server holds are the ones uploaded. The
/// upload returns the new revision number: the first upload to a fresh
/// package is revision 1.
#[test]
fn temporal_upload_file_reads_back_on_server() {
    let Some(base) = temporal_url() else {
        return;
    };
    let client = temporal_client_authed(&base);
    let package = temporal_package();
    let source_project = create_source_branch(&client, &package);
    let content = "Name: geectl-temporal-pkg\nVersion: 1.0\n";
    let rev = client
        .upload_file(
            &source_project,
            &package,
            "geectl-temporal-pkg.spec",
            content,
            &DryRun::new(false),
        )
        .expect("upload a spec file to the temporal instance");
    assert_eq!(
        rev,
        Some(1),
        "first upload to a fresh package is revision 1"
    );
    let body = file_content(&base, &source_project, &package, "geectl-temporal-pkg.spec");
    assert_eq!(
        body, content,
        "the file the server holds must be the uploaded content"
    );
}

/// Create package `package` in existing project `project` and upload
/// a spec file with `content` into it. Returns the revision the
/// upload returns.
fn add_package(client: &ObsClient<'_>, project: &str, package: &str, content: &str) -> Option<u64> {
    client
        .create_package(
            project,
            package,
            "geectl temporal test package",
            "Created by the geectl temporal live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create package on the temporal instance");
    client
        .upload_file(
            project,
            package,
            &format!("{package}.spec"),
            content,
            &DryRun::new(false),
        )
        .expect("upload a spec file to the temporal instance")
}

/// Poll until `package_log` holds at least `count` entries, for up
/// to a minute: the history is written by the backend, so the test
/// must not assert the instant the upload returns. Returns the log.
fn wait_until_log(
    client: &ObsClient<'_>,
    project: &str,
    package: &str,
    count: usize,
) -> Vec<HistoryEntry> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let log = client
            .package_log(project, package, None)
            .expect("read the package log on the temporal instance");
        if log.len() >= count {
            return log;
        }
        if std::time::Instant::now() >= deadline {
            panic!("package log for {project}/{package} still holds fewer than {count} entries after 60s");
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

/// Poll until `link_target` finds a link, for up to a minute: the
/// backend computes the linkinfo from the `_link` file, so the test
/// must not assert the instant the upload returns.
fn wait_until_link_target(
    client: &ObsClient<'_>,
    project: &str,
    package: &str,
) -> (String, String) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if let Some(target) = client
            .link_target(project, package)
            .expect("read the link target on the temporal instance")
        {
            return target;
        }
        if std::time::Instant::now() >= deadline {
            panic!("{project}/{package} still links nothing after 60s");
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

/// A file uploaded through geectl reads back through `cat`, at the
/// current revision and at an earlier one. The reads are public: no
/// credentials.
#[test]
fn temporal_cat_reads_back_uploaded_file() {
    let Some(base) = temporal_url() else {
        return;
    };
    let authed = temporal_client_authed(&base);
    let package = temporal_package();
    let source_project = create_source_branch(&authed, &package);
    let v1 = "Name: geectl-temporal-pkg\nVersion: 1.0\n";
    let v2 = "Name: geectl-temporal-pkg\nVersion: 1.1\n";
    authed
        .upload_file(
            &source_project,
            &package,
            "geectl-temporal-pkg.spec",
            v1,
            &DryRun::new(false),
        )
        .expect("upload the first spec on the temporal instance");
    authed
        .upload_file(
            &source_project,
            &package,
            "geectl-temporal-pkg.spec",
            v2,
            &DryRun::new(false),
        )
        .expect("upload the second spec on the temporal instance");
    let public = temporal_client(&base);
    assert_eq!(
        public
            .cat(&source_project, &package, "geectl-temporal-pkg.spec", None)
            .expect("cat the current revision on the temporal instance"),
        v2
    );
    assert_eq!(
        public
            .cat(
                &source_project,
                &package,
                "geectl-temporal-pkg.spec",
                Some(1)
            )
            .expect("cat revision 1 on the temporal instance"),
        v1
    );
    assert_eq!(
        public
            .cat(
                &source_project,
                &package,
                "geectl-temporal-pkg.spec",
                Some(2)
            )
            .expect("cat revision 2 on the temporal instance"),
        v2
    );
}

/// The directory listing and the project listing read back what was
/// uploaded and created, through the public reads.
#[test]
fn temporal_list_files_and_packages_read_back() {
    let Some(base) = temporal_url() else {
        return;
    };
    let authed = temporal_client_authed(&base);
    let package = temporal_package();
    let source_project = create_source_branch(&authed, &package);
    authed
        .upload_file(
            &source_project,
            &package,
            "a.spec",
            "Name: a\nVersion: 1.0\n",
            &DryRun::new(false),
        )
        .expect("upload a.spec on the temporal instance");
    authed
        .upload_file(
            &source_project,
            &package,
            "b.spec",
            "Name: b\nVersion: 1.0\n",
            &DryRun::new(false),
        )
        .expect("upload b.spec on the temporal instance");
    let public = temporal_client(&base);
    let files = public
        .list_files(&source_project, &package)
        .expect("list the package files on the temporal instance");
    let names: Vec<&str> = files.iter().map(|file| file.name.as_str()).collect();
    assert!(
        names.contains(&"a.spec") && names.contains(&"b.spec"),
        "the listing must hold both uploaded files; got {names:?}"
    );
    let packages = public
        .list_packages(&source_project)
        .expect("list the project packages on the temporal instance");
    assert!(
        packages.contains(&package),
        "the project listing must hold the created package {package}; got {packages:?}"
    );
}

/// The package log comes back newest-first, and a revision range
/// narrows it to the revisions in the range.
#[test]
fn temporal_package_log_newest_first_and_range() {
    let Some(base) = temporal_url() else {
        return;
    };
    let authed = temporal_client_authed(&base);
    let package = temporal_package();
    let source_project = create_source_branch(&authed, &package);
    authed
        .upload_file(
            &source_project,
            &package,
            "geectl-temporal-pkg.spec",
            "Name: geectl-temporal-pkg\nVersion: 1.0\n",
            &DryRun::new(false),
        )
        .expect("upload the first spec on the temporal instance");
    authed
        .upload_file(
            &source_project,
            &package,
            "geectl-temporal-pkg.spec",
            "Name: geectl-temporal-pkg\nVersion: 1.1\n",
            &DryRun::new(false),
        )
        .expect("upload the second spec on the temporal instance");
    let log = wait_until_log(&authed, &source_project, &package, 2);
    assert_eq!(log.len(), 2, "two uploads must leave two revisions");
    assert_eq!(log[0].rev, 2, "the log must be newest-first");
    assert_eq!(log[1].rev, 1);
    let first_only = authed
        .package_log(
            &source_project,
            &package,
            Some(&RevisionRange::parse("1").unwrap()),
        )
        .expect("read the ranged log on the temporal instance");
    assert_eq!(
        first_only.iter().map(|entry| entry.rev).collect::<Vec<_>>(),
        vec![1],
        "the range must narrow the log to revision 1"
    );
}

/// The revision patch diffs one revision against the previous one,
/// and revision 1 has nothing to diff against.
#[test]
fn temporal_revision_patch_diffs_revisions() {
    let Some(base) = temporal_url() else {
        return;
    };
    let authed = temporal_client_authed(&base);
    let package = temporal_package();
    let source_project = create_source_branch(&authed, &package);
    authed
        .upload_file(
            &source_project,
            &package,
            "geectl-temporal-pkg.spec",
            "Name: geectl-temporal-pkg\nVersion: 1.0\n",
            &DryRun::new(false),
        )
        .expect("upload the first spec on the temporal instance");
    authed
        .upload_file(
            &source_project,
            &package,
            "geectl-temporal-pkg.spec",
            "Name: geectl-temporal-pkg\nVersion: 1.1\n",
            &DryRun::new(false),
        )
        .expect("upload the second spec on the temporal instance");
    let public = temporal_client(&base);
    let patch = public
        .revision_patch(&source_project, &package, 2)
        .expect("diff revision 2 on the temporal instance");
    assert_eq!(patch.rev, 2);
    assert_eq!(patch.files.len(), 1, "one file changed between revisions");
    assert_eq!(
        patch.files[0].status,
        FilePatchStatus::Modified,
        "the spec was modified, not added or deleted"
    );
    assert!(
        patch.files[0].diff.contains("-Version: 1.0"),
        "the diff must drop the old version; got {}",
        patch.files[0].diff
    );
    assert!(
        patch.files[0].diff.contains("+Version: 1.1"),
        "the diff must add the new version; got {}",
        patch.files[0].diff
    );
    let first = public
        .revision_patch(&source_project, &package, 1)
        .expect("diff revision 1 on the temporal instance");
    assert!(
        first.files.is_empty(),
        "revision 1 has nothing to diff against"
    );
}

/// The project and package meta read back what creation put there.
#[test]
fn temporal_meta_reads_back() {
    let Some(base) = temporal_url() else {
        return;
    };
    let authed = temporal_client_authed(&base);
    let package = temporal_package();
    let source_project = create_source_branch(&authed, &package);
    let public = temporal_client(&base);
    let project_meta = public
        .project_meta(&source_project)
        .expect("read the project meta on the temporal instance");
    assert_eq!(project_meta.name, source_project);
    assert_eq!(
        project_meta.title.as_deref(),
        Some("geectl temporal test branch")
    );
    let package_meta = public
        .package_meta(&source_project, &package)
        .expect("read the package meta on the temporal instance");
    assert_eq!(package_meta.name, package);
    assert_eq!(package_meta.project, source_project);
    assert_eq!(
        package_meta.title.as_deref(),
        Some("geectl temporal test package")
    );
}

/// The package and project search find what was just created.
#[test]
fn temporal_search_finds_package_and_project() {
    let Some(base) = temporal_url() else {
        return;
    };
    let authed = temporal_client_authed(&base);
    let package = temporal_package();
    let source_project = create_source_branch(&authed, &package);
    let matches = authed
        .search_packages(&package)
        .expect("search packages on the temporal instance");
    assert!(
        matches
            .iter()
            .any(|found| found.name == package && found.project == source_project),
        "the search must find the created package {package}; got {matches:?}"
    );
    let projects = authed
        .search_projects(&source_project)
        .expect("search projects on the temporal instance");
    assert!(
        projects.iter().any(|found| found.name == source_project),
        "the search must find the created project {source_project}; got {projects:?}"
    );
}

/// The owner search runs against the real server: a fresh package
/// has no explicit owners, so the call succeeding (not 401, not
/// 404) is the assertion, plus the shape of whatever it returns.
#[test]
fn temporal_owners_call_succeeds() {
    let Some(base) = temporal_url() else {
        return;
    };
    let authed = temporal_client_authed(&base);
    let package = temporal_package();
    let source_project = create_source_branch(&authed, &package);
    let owners = authed
        .owners(&source_project, &package)
        .expect("owner search on the temporal instance");
    assert!(
        owners.iter().all(|owner| !owner.project.is_empty()),
        "every returned owner must name a project; got {owners:?}"
    );
}

/// `whois` reads the bootstrap admin back from the server.
#[test]
fn temporal_whois_reads_admin() {
    let Some(base) = temporal_url() else {
        return;
    };
    let authed = temporal_client_authed(&base);
    let person = authed
        .whois("Admin")
        .expect("whois on the temporal instance");
    assert_eq!(person.login, "Admin");
    assert_eq!(person.state, "confirmed");
}

/// A `_link` file uploaded into a package reads back as its link
/// target. The target is a second package in the same project, so
/// the link resolves.
#[test]
fn temporal_link_target_reads_linkinfo() {
    let Some(base) = temporal_url() else {
        return;
    };
    let authed = temporal_client_authed(&base);
    let package = temporal_package();
    let source_project = create_source_branch(&authed, &package);
    let target_package = format!("geectl-temporal-link-tgt-{}", unique_suffix());
    add_package(
        &authed,
        &source_project,
        &target_package,
        "Name: target\nVersion: 1.0\n",
    );
    let link_package = format!("geectl-temporal-link-{}", unique_suffix());
    authed
        .create_package(
            &source_project,
            &link_package,
            "geectl temporal test package",
            "Created by the geectl temporal live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create the link package on the temporal instance");
    authed
        .upload_file(
            &source_project,
            &link_package,
            "_link",
            &format!("<link project=\"{source_project}\" package=\"{target_package}\"/>"),
            &DryRun::new(false),
        )
        .expect("upload the _link file on the temporal instance");
    let public = temporal_client(&base);
    assert_eq!(
        wait_until_link_target(&public, &source_project, &link_package),
        (source_project, target_package)
    );
}

/// Two projects diffed package by package: identical, differs,
/// only-in-old, and only-in-new all classify.
#[test]
fn temporal_project_diff_classifies() {
    let Some(base) = temporal_url() else {
        return;
    };
    let authed = temporal_client_authed(&base);
    let suffix = unique_suffix();
    let old_project = format!("geectl-tmp-diff-old-{suffix}");
    let new_project = format!("geectl-tmp-diff-new-{suffix}");
    for project in [&old_project, &new_project] {
        authed
            .create_project(
                project,
                "geectl temporal test project",
                "Created by the geectl temporal live test; the instance is destroyed after the run.",
                &DryRun::new(false),
            )
            .expect("create a diff project on the temporal instance");
    }
    let shared = format!("pkg-shared-{suffix}");
    let changed = format!("pkg-changed-{suffix}");
    let old_only = format!("pkg-old-only-{suffix}");
    let new_only = format!("pkg-new-only-{suffix}");
    add_package(
        &authed,
        &old_project,
        &shared,
        "Name: shared\nVersion: 1.0\n",
    );
    add_package(
        &authed,
        &new_project,
        &shared,
        "Name: shared\nVersion: 1.0\n",
    );
    add_package(
        &authed,
        &old_project,
        &changed,
        "Name: changed\nVersion: 1.0\n",
    );
    add_package(
        &authed,
        &new_project,
        &changed,
        "Name: changed\nVersion: 1.1\n",
    );
    add_package(
        &authed,
        &old_project,
        &old_only,
        "Name: old-only\nVersion: 1.0\n",
    );
    add_package(
        &authed,
        &new_project,
        &new_only,
        "Name: new-only\nVersion: 1.0\n",
    );
    let public = temporal_client(&base);
    let diffs = public
        .project_diff(&old_project, &new_project, &[], &[], false)
        .expect("diff the two projects on the temporal instance");
    let status = |name: &str| {
        diffs
            .iter()
            .find(|diff| diff.name == name)
            .map(|diff| diff.status)
    };
    assert_eq!(status(&shared), Some(PackageDiffStatus::Identical));
    assert_eq!(status(&changed), Some(PackageDiffStatus::Differs));
    assert_eq!(status(&old_only), Some(PackageDiffStatus::OnlyInOld));
    assert_eq!(status(&new_only), Some(PackageDiffStatus::OnlyInNew));
    let changed_diff = diffs
        .iter()
        .find(|diff| diff.name == changed)
        .expect("the changed package must be in the diff");
    assert_eq!(changed_diff.files.len(), 1, "one file changed");
    assert!(
        changed_diff.files[0].diff.contains("+Version: 1.1"),
        "the diff must add the new version; got {}",
        changed_diff.files[0].diff
    );
}

/// Reading a filed request back through the library: the id,
/// creator, state, description, action, and creation history must be
/// what was filed.
#[test]
fn temporal_show_request_reads_back_the_filed_request() {
    let Some(base) = temporal_url() else {
        return;
    };
    let client = temporal_client_authed(&base);
    let (id, package) = file_temporal_request(&client, "2.0");
    let request = client
        .show_request(&id)
        .expect("read the filed request back from the server");
    assert_eq!(request.id.to_string(), id);
    assert_eq!(request.creator, "Admin");
    assert_eq!(request.state.name, RequestState::New);
    assert_eq!(
        request.description,
        format!("Update {package} to 2.0: temporal test submission")
    );
    let (_, src_package, _, tgt_package, sourceupdate) = submit_action(&request);
    assert_eq!(src_package, package);
    assert_eq!(tgt_package, Some(package.as_str()));
    assert_eq!(sourceupdate, Some(SourceUpdate::Cleanup));
    // The creation history entry is the filing itself.
    assert_eq!(request.history.len(), 1);
    assert_eq!(request.history[0].description, "Request created");
    assert_eq!(request.history[0].comment, request.description);
}

/// The log of a fresh request is its creation entry alone.
#[test]
fn temporal_request_log_lists_the_creation_entry() {
    let Some(base) = temporal_url() else {
        return;
    };
    let client = temporal_client_authed(&base);
    let (id, package) = file_temporal_request(&client, "2.0");
    let log = client
        .request_log(&id)
        .expect("read the filed request's log from the server");
    assert_eq!(
        log.len(),
        1,
        "a fresh request has one log entry: its creation"
    );
    assert_eq!(log[0].description, "Request created");
    assert_eq!(
        log[0].comment,
        format!("Update {package} to 2.0: temporal test submission")
    );
}

/// Accepting a request through the library moves it to `accepted` on
/// the server.
#[test]
fn temporal_accept_request_changes_state_on_server() {
    let Some(base) = temporal_url() else {
        return;
    };
    let client = temporal_client_authed(&base);
    let (id, _) = file_temporal_request(&client, "2.0");
    let dry = DryRun::new(false);
    client
        .accept_request(&id, "accepted by the geectl temporal test", false, &dry)
        .expect("accept the request on the temporal instance");
    let request = client
        .show_request(&id)
        .expect("read the accepted request back from the server");
    assert_eq!(request.state.name, RequestState::Accepted);
}

/// Declining a request through the library moves it to `declined` on
/// the server.
#[test]
fn temporal_decline_request_changes_state_on_server() {
    let Some(base) = temporal_url() else {
        return;
    };
    let client = temporal_client_authed(&base);
    let (id, _) = file_temporal_request(&client, "2.0");
    let dry = DryRun::new(false);
    client
        .decline_request(&id, "declined by the geectl temporal test", &dry)
        .expect("decline the request on the temporal instance");
    let request = client
        .show_request(&id)
        .expect("read the declined request back from the server");
    assert_eq!(request.state.name, RequestState::Declined);
}

/// Revoking a request through the library moves it to `revoked` on
/// the server.
#[test]
fn temporal_revoke_request_changes_state_on_server() {
    let Some(base) = temporal_url() else {
        return;
    };
    let client = temporal_client_authed(&base);
    let (id, _) = file_temporal_request(&client, "2.0");
    let dry = DryRun::new(false);
    client
        .revoke_request(&id, "revoked by the geectl temporal test", &dry)
        .expect("revoke the request on the temporal instance");
    let request = client
        .show_request(&id)
        .expect("read the revoked request back from the server");
    assert_eq!(request.state.name, RequestState::Revoked);
}

/// Asking for a review through the library moves the request to
/// `review` on the server, with the review carrying who it was asked
/// of and why.
#[test]
fn temporal_add_review_marks_the_request_for_review() {
    let Some(base) = temporal_url() else {
        return;
    };
    let client = temporal_client_authed(&base);
    let (id, _) = file_temporal_request(&client, "2.0");
    let dry = DryRun::new(false);
    let target = ReviewTarget::new(Some("Admin"), None, None, None)
        .expect("a review asked of a user is a valid target");
    client
        .add_review(&id, &target, "please review", &dry)
        .expect("ask Admin to review the request on the temporal instance");
    let request = client
        .show_request(&id)
        .expect("read the reviewed request back from the server");
    assert_eq!(request.state.name, RequestState::Review);
    assert_eq!(request.reviews.len(), 1);
    assert_eq!(request.reviews[0].by_user.as_deref(), Some("Admin"));
    assert_eq!(request.reviews[0].who.as_deref(), Some("Admin"));
    assert_eq!(request.reviews[0].comment, "please review");
    assert_eq!(request.reviews[0].state, ReviewState::New);
}

/// A review that was acted on reads back with its history: the
/// parser must read the review's `<history>` children, not refuse
/// the whole request. The library has no review-state operation in
/// this batch, so the harness drives the raw `changereviewstate`
/// command to put the review into the accepted state the parser
/// must then read back — the test's subject is the readback, and the
/// state change is only its setup.
#[test]
fn temporal_review_with_history_reads_back() {
    let Some(base) = temporal_url() else {
        return;
    };
    let client = temporal_client_authed(&base);
    let (id, _) = file_temporal_request(&client, "2.0");
    let dry = DryRun::new(false);
    let target = ReviewTarget::new(Some("Admin"), None, None, None)
        .expect("a review asked of a user is a valid target");
    client
        .add_review(&id, &target, "please review", &dry)
        .expect("ask Admin to review the request on the temporal instance");
    change_review_state(&base, &id, "accepted", "Admin");
    let request = client
        .show_request(&id)
        .expect("read the reviewed request back from the server");
    assert_eq!(request.reviews.len(), 1);
    assert_eq!(request.reviews[0].state, ReviewState::Accepted);
    assert_eq!(
        request.reviews[0].history.len(),
        1,
        "an accepted review must carry its history entry"
    );
    assert_eq!(
        request.reviews[0].history[0].description,
        "Review got accepted"
    );
}

/// Commenting on a request through the library, then listing its
/// comments back: the comment and its threaded reply must be what
/// was posted.
#[test]
fn temporal_comment_create_and_list_round_trips() {
    let Some(base) = temporal_url() else {
        return;
    };
    let client = temporal_client_authed(&base);
    let (id, _) = file_temporal_request(&client, "2.0");
    let dry = DryRun::new(false);
    client
        .add_comment(&id, "first comment", None, &dry)
        .expect("comment on the request on the temporal instance");
    let comments = client
        .list_comments(&id)
        .expect("list the request's comments from the server");
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].body, "first comment");
    assert_eq!(comments[0].who, "Admin");
    assert_eq!(comments[0].parent_id, None);
    let first_id = comments[0].id;
    let first_id_str = first_id.to_string();
    client
        .add_comment(&id, "reply to the first", Some(&first_id_str), &dry)
        .expect("reply to the comment on the temporal instance");
    let comments = client
        .list_comments(&id)
        .expect("list the request's comments from the server");
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[1].body, "reply to the first");
    assert_eq!(comments[1].parent_id, Some(first_id));
}

/// The diff of a filed request, plain and wrapped in the request
/// XML. The branch holds one file, so the plain diff shows it added;
/// the XML form must wrap a request element.
#[test]
fn temporal_request_diff_returns_the_submit_diff() {
    let Some(base) = temporal_url() else {
        return;
    };
    let client = temporal_client_authed(&base);
    let (id, _) = file_temporal_request_with_file(
        &client,
        "2.0",
        "geectl-temporal.txt",
        "geectl temporal diff fixture\n",
    );
    let plain = client
        .request_diff(&id, false)
        .expect("read the plain diff of the filed request");
    assert!(
        plain.contains("geectl-temporal.txt"),
        "the plain diff must show the added file; got {plain}"
    );
    let xml = client
        .request_diff(&id, true)
        .expect("read the XML diff of the filed request");
    assert!(
        xml.contains("<request"),
        "the XML diff must wrap a request element; got {xml}"
    );
}

/// Searching by id and action type finds the filed submit, and
/// searching by creator contains it among the parallel tests'
/// requests. The types filter is the live proof for the
/// `action/@type` question: the server accepts it in the match
/// expression.
#[test]
fn temporal_search_requests_finds_the_filed_request() {
    let Some(base) = temporal_url() else {
        return;
    };
    let client = temporal_client_authed(&base);
    let (id, package) = file_temporal_request(&client, "2.0");
    let search = RequestSearch::new()
        .request_ids(vec![id.parse().expect("the filed request id is a number")])
        .types(vec!["submit".to_string()]);
    let found = client
        .search_requests(&search)
        .expect("search by id and type on the temporal instance");
    assert_eq!(
        found.len(),
        1,
        "the search must find exactly the filed request"
    );
    assert_eq!(found[0].id.to_string(), id);
    assert_eq!(found[0].state.name, RequestState::New);
    // The source/target package filters use the singular attribute
    // the search engine whitelists; the plural is refused with 400.
    // The exclude filter is `not(target[@project='X'])`: excluding
    // the request's own target project hides it, excluding another
    // project does not.
    let target_project = {
        let request = client
            .show_request(&id)
            .expect("read the filed request back for its target project");
        let (_, _, tgt_project, _, _) = submit_action(&request);
        tgt_project
            .expect("the filed submit names a target project")
            .to_string()
    };
    let by_source_package = client
        .search_requests(
            &RequestSearch::new()
                .request_ids(vec![id.parse().expect("the filed request id is a number")])
                .source_packages(vec![package.clone()]),
        )
        .expect("search by id and source package on the temporal instance");
    assert_eq!(
        by_source_package.len(),
        1,
        "the source-package search must find exactly the filed request"
    );
    let by_target_package = client
        .search_requests(
            &RequestSearch::new()
                .request_ids(vec![id.parse().expect("the filed request id is a number")])
                .target_packages(vec![package.clone()]),
        )
        .expect("search by id and target package on the temporal instance");
    assert_eq!(
        by_target_package.len(),
        1,
        "the target-package search must find exactly the filed request"
    );
    let excluded = client
        .search_requests(
            &RequestSearch::new()
                .request_ids(vec![id.parse().expect("the filed request id is a number")])
                .exclude_target_projects(vec![target_project.to_string()]),
        )
        .expect("search excluding the filed request's target on the temporal instance");
    assert!(
        excluded.is_empty(),
        "excluding the filed request's target project must hide it"
    );
    let not_excluded = client
        .search_requests(
            &RequestSearch::new()
                .request_ids(vec![id.parse().expect("the filed request id is a number")])
                .exclude_target_projects(vec!["home:Admin".to_string()]),
        )
        .expect("search excluding another project on the temporal instance");
    assert_eq!(
        not_excluded.len(),
        1,
        "excluding another project must not hide the filed request"
    );
    let by_creator = client
        .search_requests(&RequestSearch::new().creators(vec!["Admin".to_string()]))
        .expect("search by creator on the temporal instance");
    assert!(
        by_creator
            .iter()
            .any(|request| request.id.to_string() == id),
        "the creator search must contain the filed request {id}"
    );
}

/// The lenient state change on a request the server refuses to move:
/// accepting an accepted request reports `false` instead of failing,
/// so a batch triaging many requests does not stop here.
#[test]
fn temporal_try_change_state_false_on_an_accepted_request() {
    let Some(base) = temporal_url() else {
        return;
    };
    let client = temporal_client_authed(&base);
    let (id, _) = file_temporal_request(&client, "2.0");
    let dry = DryRun::new(false);
    client
        .accept_request(&id, "accepted by the geectl temporal test", false, &dry)
        .expect("accept the request on the temporal instance");
    assert!(
        !client
            .try_change_state(&id, RequestState::Accepted, "accept again", false, &dry)
            .expect("the lenient change reports false, not an error"),
        "accepting an accepted request must report false"
    );
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_check_is_exact() {
        assert!(is_loopback_url("http://127.0.0.1:3000"));
        assert!(is_loopback_url("http://127.0.0.1"));
        assert!(is_loopback_url("http://localhost:3000"));
        assert!(is_loopback_url("http://localhost"));
        assert!(is_loopback_url("http://[::1]:3000"));
        assert!(is_loopback_url("http://[::1]"));
        assert!(is_loopback_url("http://127.0.0.1:3000/about"));
        // The whole 127.0.0.0/8 range is loopback, not just 127.0.0.1.
        assert!(is_loopback_url("http://127.0.0.2:3000"));
        assert!(is_loopback_url("http://127.1.2.3:3000"));
        // A host that merely starts with a loopback address is not one.
        assert!(!is_loopback_url("http://127.0.0.1.evil.com"));
        assert!(!is_loopback_url("http://localhost.evil.com"));
        assert!(!is_loopback_url("http://example.com"));
        // Userinfo must not change the host: ureq resolves the host
        // after the LAST `@`, so `http://127.0.0.1:80@evil.com` is
        // evil.com, not loopback — even though the text before the
        // `@` looks loopback. A password may itself contain `@`.
        assert!(!is_loopback_url("http://127.0.0.1:80@evil.com"));
        assert!(!is_loopback_url("http://user@evil.com"));
        assert!(!is_loopback_url("http://user:p@ss@evil.com"));
        assert!(!is_loopback_url("http://user@[::1]:3000@evil.com"));
        // A genuine loopback host behind userinfo is still loopback.
        assert!(is_loopback_url("http://user@127.0.0.1:3000"));
        assert!(is_loopback_url("http://user:p@ss@127.0.0.1:3000"));
        assert!(is_loopback_url("http://user@[::1]:3000"));
        // A bracketed host counts only when the closing bracket is
        // followed by end of authority or a `:port`: `http://[::1]evil.com`
        // is not loopback, while genuine `[::1]` with or without a port is.
        assert!(!is_loopback_url("http://[::1]evil.com"));
        assert!(!is_loopback_url("http://[::1]evil.com:3000"));
        assert!(!is_loopback_url("http://[::1"));
        assert!(is_loopback_url("http://[::1]:3000"));
        assert!(is_loopback_url("http://[::1]"));
        // DNS is case-insensitive: uppercase LOCALHOST is loopback.
        assert!(is_loopback_url("http://LOCALHOST:3000"));
        assert!(is_loopback_url("http://Localhost"));
        assert!(!is_loopback_url("http://LOCALHOST.evil.com"));
        // HTTPS is not loopback-HTTP either: the exception is for the
        // plain-HTTP development frontend only.
        assert!(!is_loopback_url("https://127.0.0.1:3000"));
        assert!(!is_loopback_url(""));
    }

    #[test]
    fn temporal_base_validation() {
        // Bare loopback bases pass, trimmed and slash-stripped.
        assert_eq!(
            validate_temporal_base("http://127.0.0.1:3000").as_deref(),
            Ok("http://127.0.0.1:3000")
        );
        assert_eq!(
            validate_temporal_base("  http://127.0.0.1:3000/  ").as_deref(),
            Ok("http://127.0.0.1:3000")
        );
        // A space before the trailing slash is stripped too.
        assert_eq!(
            validate_temporal_base("http://127.0.0.1:3000 /").as_deref(),
            Ok("http://127.0.0.1:3000")
        );
        // A base with a path, query, or fragment is rejected: the
        // tests append paths themselves, so a non-bare base would
        // double them.
        assert!(validate_temporal_base("http://127.0.0.1:3000/about").is_err());
        assert!(validate_temporal_base("http://127.0.0.1:3000?x=1").is_err());
        assert!(validate_temporal_base("http://127.0.0.1:3000#frag").is_err());
        // Non-loopback is rejected, including behind userinfo.
        assert!(validate_temporal_base("http://example.com").is_err());
        assert!(validate_temporal_base("http://127.0.0.1:80@evil.com").is_err());
        // A non-http URL reports a scheme error, not a bare-base one:
        // the bare-base check would otherwise see the `//` in `https://`.
        let scheme_err = validate_temporal_base("https://127.0.0.1:3000").unwrap_err();
        assert!(
            scheme_err.contains("must be an http:// URL"),
            "a non-http URL must report a scheme error; got {scheme_err}"
        );
        // A bracketed-host spoof is rejected by the loopback check.
        assert!(validate_temporal_base("http://[::1]evil.com").is_err());
    }

    #[test]
    fn temporal_obs_client_uses_the_instance_base() {
        // The client built for the temporal base must point its reads
        // at that base (not at the production API), and must refuse a
        // non-loopback http base.
        let http = UreqClient::new(Timeout::default()).expect("default timeout is valid");
        let client = ObsClient::with_base(&http, "http://127.0.0.1:3000").unwrap();
        assert_eq!(
            client.search_url("foo").unwrap(),
            "http://127.0.0.1:3000/search/request?match=action%2Ftarget%2F%40package%3D%27foo%27%20and%20%28state%2F%40name%3D%27new%27%20or%20state%2F%40name%3D%27review%27%29"
        );
        assert!(ObsClient::with_base(&http, "http://example.com").is_err());
    }

    #[test]
    fn temporal_filing_path_is_wired_but_not_filed() {
        // The filing path is wired end to end (payload → SrFiling →
        // dry-run) without a live server: dry-run files nothing and
        // returns None. The live path against the instance is the
        // live tests above.
        let http = UreqClient::new(Timeout::default()).expect("default timeout is valid");
        let client = ObsClient::with_base(&http, "http://127.0.0.1:3000").unwrap();
        let dry = DryRun::default();
        let filing = SrFiling::new(&client, &dry);
        let payload = SrPayload::new(
            "home:geectl-temporal",
            "foo",
            "devel:languages:python",
            "foo",
            "Update foo to 2.0",
            "- Update to version 2.0.\n",
            &[],
        )
        .unwrap();
        assert_eq!(filing.file(&payload).unwrap(), None);
    }
}
