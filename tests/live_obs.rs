//! Live tests for the OBS operations against a live OBS instance.
//!
//! These are layer 2 of the testing design (see `docs/architecture.md`):
//! scheduled live tests against a real (temporary) OBS instance, asserting
//! on **server state** — whatever geectl reads must reflect what the
//! server actually holds. That is what catches drift as the server changes
//! underneath us, which the per-PR fixture tests cannot do.
//!
//! These tests are NOT part of the per-PR gate. They run only in the
//! scheduled `live` CI job, against the instance the provisioning in
//! `tests/live/` spins up. They are skipped unless the
//! `GEECTL_LIVE_URL` environment variable is set to the instance's
//! base URL (e.g. `http://127.0.0.1:3000`), so `cargo test` stays green
//! without an instance. When `GEECTL_LIVE_REQUIRED` is set, a missing
//! or empty URL panics instead (the live workflow sets that marker
//! and wires the URL, so its absence means the wiring broke).
//!
//! The instance is plain HTTP on localhost. That is the single sanctioned
//! exception to geectl's https-only URL rule, and it is safe because:
//! - the instance is loopback-only (the harness rejects any
//!   non-loopback URL outright — see `validate_live_base`),
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
//! The upload test (`live_upload_file_reads_back_on_server`)
//! uploads a spec file into a fresh branch package and reads it back,
//! asserting the bytes and the revision number.
//!
//! The remote-read tests (`live_cat_reads_back_uploaded_file`,
//! `live_list_files_and_packages_read_back`,
//! `live_package_log_newest_first_and_range`,
//! `live_revision_patch_diffs_revisions`,
//! `live_meta_reads_back`,
//! `live_search_finds_package_and_project`,
//! `live_owners_call_succeeds`, `live_whois_reads_admin`,
//! `live_link_target_reads_linkinfo`, and
//! `live_project_diff_classifies`) exercise the read operations
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
use std::sync::atomic::{AtomicU64, Ordering};

use geectl::auth::Credentials;
use geectl::error::ErrorKind;
use geectl::http::{sanitize_url_for_error, HttpClient, Timeout, UreqClient};
use geectl::obs::{
    AggregateOptions, BranchOptions, BuildLogQuery, ChangeDevelPayload, CopyOptions,
    CopyProjectOptions, CreatePackageOptions, DeletePayload, DeleteProjectOptions, DeleteTarget,
    DryRun, FilePatchStatus, HistoryEntry, LinkOptions, LinkRevision, MaintenanceBranchOptions,
    MaintenanceIncidentPayload, ObsClient, PackageDiffStatus, ReleaseOptions, Request,
    RequestAction, RequestSearch, RequestState, ResultsQuery, ReviewState, ReviewTarget,
    RevisionRange, SourceUpdate, SrFiling, SrPayload,
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

/// Validate a live base URL: trim whitespace, strip trailing
/// slashes, require a bare base (no path, query, or fragment), and
/// require a loopback host. Returns the cleaned URL, or the reason it
/// is invalid. Pure, so it is unit-testable; `live_url` turns the
/// error into a panic at the boundary.
fn validate_live_base(raw: &str) -> Result<String, String> {
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
            "must be an http:// URL (the live instance is plain HTTP on loopback); got {}",
            sanitize_url_for_error(&url)
        ));
    }
    // Credentials go through the auth provider, never the URL: a base
    // carrying userinfo is rejected outright, before any other check
    // could report it.
    let after_scheme = url.strip_prefix("http://").unwrap_or(&url);
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    if after_scheme[..authority_end].contains('@') {
        return Err(format!(
            "must not contain userinfo (credentials go through the auth provider, not the URL); got {}",
            sanitize_url_for_error(&url)
        ));
    }
    // The base must be bare (scheme + authority only): the tests
    // append paths themselves (`{base}/about`), so a base with a path,
    // query, or fragment would double the path or silently drop it.
    // Reject at the boundary with a clear message, not a confusing
    // loopback-assert failure downstream.
    if after_scheme.contains(['/', '?', '#']) {
        return Err(format!(
            "must be a bare base URL (http://127.0.0.1:3000), with no path, query, or fragment; got {}",
            sanitize_url_for_error(&url)
        ));
    }
    if !is_loopback_url(&url) {
        return Err(format!(
            "must be a loopback URL (http://127.0.0.1, http://localhost, or http://[::1]); got {}",
            sanitize_url_for_error(&url)
        ));
    }
    Ok(url)
}

/// The live instance's base URL, from the environment.
///
/// Returns `None` (skip the test) when `GEECTL_LIVE_URL` is unset or
/// empty. When `GEECTL_LIVE_REQUIRED` is set, a missing or empty
/// value panics instead: the live workflow sets that marker and
/// wires the URL, so its absence means the wiring broke, and a silent
/// skip would go green while hiding drift. The marker (not `CI` or
/// `GITHUB_ACTIONS`) is the signal: the per-PR `test` job also runs
/// under CI, but with no live instance, so the tests must skip
/// there. A present but invalid value panics in both cases: the
/// instance is plain HTTP, so pointing the harness at a non-localhost
/// server would send requests (and, once auth lands, credentials) in
/// cleartext over a real network. That must fail fast, not silently.
fn live_url() -> Option<String> {
    let raw = match std::env::var("GEECTL_LIVE_URL") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            if std::env::var_os("GEECTL_LIVE_REQUIRED").is_some() {
                panic!(
                    "GEECTL_LIVE_URL is not set or empty: the live workflow must wire it (see .github/workflows/live.yml)"
                );
            }
            eprintln!("skipping live test: GEECTL_LIVE_URL is not set");
            return None;
        }
    };
    match validate_live_base(&raw) {
        Ok(url) => Some(url),
        Err(reason) => panic!("GEECTL_LIVE_URL {reason}"),
    }
}

/// The live instance's bootstrap admin credentials (the OBS
/// development defaults, ephemeral, never a real credential).
/// Overridable via `GEECTL_LIVE_OBS_USERNAME` /
/// `GEECTL_LIVE_OBS_PASSWORD` if the provisioning ever changes
/// them.
fn live_credentials() -> Credentials {
    let username =
        std::env::var("GEECTL_LIVE_OBS_USERNAME").unwrap_or_else(|_| "Admin".to_string());
    let password =
        std::env::var("GEECTL_LIVE_OBS_PASSWORD").unwrap_or_else(|_| "opensuse".to_string());
    Credentials::Basic { username, password }
}

/// A client for the live instance, with no credentials (public
/// reads only).
fn live_client(base: &str) -> ObsClient<'_> {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    ObsClient::with_base(http, base).expect("the live base is a loopback http URL")
}

/// A client for the live instance, authenticated with the
/// bootstrap admin credentials.
fn live_client_authed(base: &str) -> ObsClient<'_> {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    ObsClient::with_base_and_credentials(http, base, live_credentials())
        .expect("the live base is a loopback http URL")
}

/// A per-call counter folded into the suffix: the clock reading
/// alone is not enough — two parallel tests can read the same
/// nanosecond tick (the clock's granularity is coarser than a
/// nanosecond), and the dedup guard then trips on the shared name
/// (issue #39). The counter makes every call distinct within the
/// process; the pid keeps it distinct across processes.
static SUFFIX_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A suffix that makes a test's projects unique: the process id plus
/// a nanosecond timestamp plus a per-call counter. The live tests
/// run in parallel in one process, and the instance is shared with
/// the `live` target's tests, so two tests must never create the
/// same project — a name collision would file one test's request
/// against another's branch. Only digits and hyphens: a valid OBS
/// name fragment.
fn unique_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time is after the epoch")
        .as_nanos();
    let n = SUFFIX_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{nanos}-{n}", std::process::id())
}

/// A package name unique to this test run. The projects already
/// differ per test, so the package entities never collide — but the
/// name must still be unique: the open-SR search is scoped by target
/// package name only, so a shared name lets one test's open request
/// block another test's filing (the refile test hit exactly that:
/// the filing test's open request targeted the same package name).
fn live_package() -> String {
    format!("geectl-live-pkg-{}", unique_suffix())
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
            "geectl live test branch",
            "Created by the geectl live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create source project on the live instance");
    client
        .create_package(
            &source_project,
            package,
            "geectl live test package",
            "Created by the geectl live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create source package on the live instance");
    source_project
}

/// Create a source branch and a target project, all uniquely named
/// for this test run. Returns `(source_project, target_project,
/// package)`. The target project only needs to exist: the submitted
/// package is created in it on accept.
fn setup_branch(client: &ObsClient<'_>) -> (String, String, String) {
    let package = live_package();
    let source_project = create_source_branch(client, &package);
    let target_project = format!("geectl-tmp-tgt-{}", unique_suffix());
    client
        .create_project(
            &target_project,
            "geectl live test target",
            "Created by the geectl live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create target project on the live instance");
    (source_project, target_project, package)
}

/// A submit-request payload for the live tests: source branch to
/// target, with the default cleanup (`opt_sourceupdate="cleanup"`),
/// so the accept test exercises the server-side branch deletion.
fn live_payload(
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
        &format!("Update {package} to {version}: live test submission"),
        &format!(
            "-------------------------------------------------------------------\nFri Nov 20 07:00:00 UTC 2026 - Tomáš Chvátal <tomas.chvatal@gmail.com>\n\n- Update to version {version}.\n"
        ),
        &[],
    )
    .expect("live SR payload")
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

/// File a submit request on the live instance and return its id
/// and package. Each call files from a fresh, uniquely-named branch
/// (see `setup_branch`), so parallel tests never share a target
/// package name — the open-SR search is scoped by target package
/// name only.
fn file_live_request(client: &ObsClient<'_>, version: &str) -> (String, String) {
    let (source_project, target_project, package) = setup_branch(client);
    let dry = DryRun::new(false);
    let filing = SrFiling::new(client, &dry);
    let payload = live_payload(&source_project, &package, &target_project, version);
    let id = filing
        .file(&payload)
        .expect("file a submit request on the live instance")
        .expect("live filing returns a request id");
    (id, package)
}

/// File a submit request from a branch holding one file: the diff of
/// an empty submit fails on the server (the backend diff has nothing
/// to compute from, and the unrescued diff error renders as 404), so
/// the diff test files from a branch with content. The other tests
/// keep the empty branch — they never ask the server for a diff.
fn file_live_request_with_file(
    client: &ObsClient<'_>,
    version: &str,
    filename: &str,
    content: &str,
) -> (String, String) {
    let (source_project, target_project, package) = setup_branch(client);
    let dry = DryRun::new(false);
    client
        .upload_file(&source_project, &package, filename, content, &dry)
        .expect("upload a file into the source branch on the live instance");
    let filing = SrFiling::new(client, &dry);
    let payload = live_payload(&source_project, &package, &target_project, version);
    let id = filing
        .file(&payload)
        .expect("file a submit request on the live instance")
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

/// The value of attribute `attr` anywhere in `body` (first
/// occurrence). Used for link attributes, which are not tied to one
/// element the tests can name.
fn xml_attr(body: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let value = body.split_once(&needle)?.1;
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
    http.get_with(&format!("{base}/request/{id}"), &live_credentials())
        .expect("GET the filed request back from the server")
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
            "accepted by the geectl live test",
            &live_credentials(),
        )
        .expect("change the review state on the live instance");
    assert!(
        response_is_status_ok(&body),
        "changing the review state of request {id} did not return <status code=\"ok\"/>; body: {body}"
    );
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
fn live_obs_devel_project_missing_is_none() {
    let Some(base) = live_url() else {
        return;
    };
    // A package that was never created has no devel project on the
    // instance: the lookup must return None from the server's own
    // directory listing, not from a fixture.
    let client = live_client(&base);
    assert_eq!(
        client
            .devel_project("geectl-live-no-such-package")
            .expect("devel lookup on the live instance"),
        None
    );
}

#[test]
fn live_obs_has_open_sr_false_on_fresh_instance() {
    let Some(base) = live_url() else {
        return;
    };
    // A fresh instance holds no submit requests at all, so the dedup
    // search must return false — and the search must succeed against
    // the real server with the bootstrap admin credentials, not 401.
    let client = live_client_authed(&base);
    assert!(
        !client
            .has_open_sr("geectl-live-no-such-package")
            .expect("open-SR search on the live instance"),
        "a fresh live instance must have no open submit request"
    );
}

/// Filing a real submit request and reading it back from the server.
#[test]
fn live_filed_request_shows_up_on_server() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, target_project, package) = setup_branch(&client);
    let dry = DryRun::new(false);
    let filing = SrFiling::new(&client, &dry);
    let payload = live_payload(&source_project, &package, &target_project, "2.0");
    let id = filing
        .file(&payload)
        .expect("file a submit request on the live instance")
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
        format!("Update {package} to 2.0: live test submission")
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
fn live_obs_open_requests_for_package_empty_on_fresh_instance() {
    let Some(base) = live_url() else {
        return;
    };
    // A fresh instance holds no requests at all, so the search for
    // requests involving a never-created package must return an empty
    // list — and the search must succeed against the real server with
    // the bootstrap admin credentials, not 401. This exercises the
    // delete pre-check's search URL and response parsing against
    // server state, not a fixture.
    let client = live_client_authed(&base);
    assert!(
        client
            .open_requests_for_package("geectl-live-no-such-project", "geectl-live-no-such-package")
            .expect("involved-requests search on the live instance")
            .is_empty(),
        "a fresh live instance must have no requests involving the package"
    );
}

/// The full refile lifecycle against the server: file, supersede,
/// delete.
#[test]
fn live_refile_supersedes_and_deletes_on_server() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_a, target_project, package) = setup_branch(&client);
    let dry = DryRun::new(false);
    let filing = SrFiling::new(&client, &dry);
    let id1 = filing
        .file(&live_payload(&source_a, &package, &target_project, "2.0"))
        .expect("file the first submit request on the live instance")
        .expect("live filing returns a request id");
    // The refile comes from a second branch, same target.
    let source_b = create_source_branch(&client, &package);
    let id2 = filing
        .refile(
            &live_payload(&source_b, &package, &target_project, "2.1"),
            &id1,
        )
        .expect("refile on the live instance")
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
/// deletes its source branch when it lands. The accept goes through
/// the library, which retries transient failures for a bounded 60 s
/// (the server's backend can still be registering the just-filed
/// request), so the test proves the library path absorbs the race.
#[test]
fn live_accept_deletes_source_branch_on_server() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, target_project, package) = setup_branch(&client);
    let dry = DryRun::new(false);
    let filing = SrFiling::new(&client, &dry);
    let payload = live_payload(&source_project, &package, &target_project, "2.0");
    let id = filing
        .file(&payload)
        .expect("file a submit request on the live instance")
        .expect("live filing returns a request id");
    let filed = request_xml(&base, &id);
    assert!(
        filed.contains("<sourceupdate>cleanup</sourceupdate>"),
        "filed request {id} must carry the default cleanup; body: {filed}"
    );
    client
        .accept_request(&id, "accepted by the geectl live test", false, &dry)
        .expect("accept the request on the live instance");
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
        &live_credentials(),
    )
    .expect("GET the uploaded file back from the server")
}

/// Upload a file through geectl and read it back from the server,
/// asserting the bytes the server holds are the ones uploaded. The
/// upload returns the new revision number: the first upload to a fresh
/// package is revision 1.
#[test]
fn live_upload_file_reads_back_on_server() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&client, &package);
    let content = "Name: geectl-live-pkg\nVersion: 1.0\n";
    let rev = client
        .upload_file(
            &source_project,
            &package,
            "geectl-live-pkg.spec",
            content,
            &DryRun::new(false),
        )
        .expect("upload a spec file to the live instance");
    assert_eq!(
        rev,
        Some(1),
        "first upload to a fresh package is revision 1"
    );
    let body = file_content(&base, &source_project, &package, "geectl-live-pkg.spec");
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
            "geectl live test package",
            "Created by the geectl live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create package on the live instance");
    client
        .upload_file(
            project,
            package,
            &format!("{package}.spec"),
            content,
            &DryRun::new(false),
        )
        .expect("upload a spec file to the live instance")
}

/// PUT a project `_meta` carrying a project link at `link_project`,
/// so the project's packages are inherited from that project. The
/// harness writes the meta directly: creating a project link is not
/// a library operation (it is test setup, not an agent workflow).
fn set_project_link(base: &str, project: &str, link_project: &str) {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    let meta = format!(
        "<project name=\"{project}\"><title>geectl live test project</title><description>Created by the geectl live live test; the instance is destroyed after the run.</description><link project=\"{link_project}\"/></project>"
    );
    http.put(
        &format!("{base}/source/{project}/_meta"),
        &meta,
        &live_credentials(),
    )
    .expect("PUT the project meta with a project link");
}
/// PUT a package `_meta` carrying a `<devel project="..."/>`, so the
/// package resolves its branch source through that project. The
/// harness writes the meta directly (test setup, like
/// `set_project_link`); the library operation is `set_devel_project`.
fn set_package_devel(base: &str, project: &str, package: &str, devel_project: &str) {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    let meta = format!(
        "<package name=\"{package}\" project=\"{project}\"><title>geectl live test package</title><description>Created by the geectl live live test; the instance is destroyed after the run.</description><devel project=\"{devel_project}\"/></package>"
    );
    http.put(
        &format!("{base}/source/{project}/{package}/_meta"),
        &meta,
        &live_credentials(),
    )
    .expect("PUT the package meta with a devel project");
}
/// Read a package's `_link` back from the server, authenticated: the
/// branch projects are `home:Admin` projects, not public reads.
fn read_link(base: &str, project: &str, package: &str) -> String {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    http.get_with(
        &format!("{base}/source/{project}/{package}/_link"),
        &live_credentials(),
    )
    .expect("GET the _link back from the server")
}
/// Create a factory project with a package, an update project linked
/// at the factory (so the package is inherited there), and a devel
/// project that also holds the package. Returns
/// `(factory, update, devel, package)`. The package is created in
/// the devel project too, so a branch that resolves through devel
/// links at a package that exists.
fn setup_branch_projects(client: &ObsClient<'_>, base: &str) -> (String, String, String, String) {
    let package = live_package();
    let factory = format!("geectl-tmp-factory-{}", unique_suffix());
    client
        .create_project(
            &factory,
            "geectl live test factory",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create factory project on the live instance");
    client
        .create_package(
            &factory,
            &package,
            "geectl live test package",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create package in the factory project");
    let update = format!("geectl-tmp-update-{}", unique_suffix());
    client
        .create_project(
            &update,
            "geectl live test update",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create update project on the live instance");
    set_project_link(base, &update, &factory);
    let devel = format!("geectl-tmp-devel-{}", unique_suffix());
    client
        .create_project(
            &devel,
            "geectl live test devel",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create devel project on the live instance");
    client
        .create_package(
            &devel,
            &package,
            "geectl live test package",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create package in the devel project");
    (factory, update, devel, package)
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
            .expect("read the package log on the live instance");
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
            .expect("read the link target on the live instance")
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
fn live_cat_reads_back_uploaded_file() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&authed, &package);
    let v1 = "Name: geectl-live-pkg\nVersion: 1.0\n";
    let v2 = "Name: geectl-live-pkg\nVersion: 1.1\n";
    authed
        .upload_file(
            &source_project,
            &package,
            "geectl-live-pkg.spec",
            v1,
            &DryRun::new(false),
        )
        .expect("upload the first spec on the live instance");
    authed
        .upload_file(
            &source_project,
            &package,
            "geectl-live-pkg.spec",
            v2,
            &DryRun::new(false),
        )
        .expect("upload the second spec on the live instance");
    let public = live_client(&base);
    assert_eq!(
        public
            .cat(&source_project, &package, "geectl-live-pkg.spec", None)
            .expect("cat the current revision on the live instance"),
        v2
    );
    assert_eq!(
        public
            .cat(&source_project, &package, "geectl-live-pkg.spec", Some(1))
            .expect("cat revision 1 on the live instance"),
        v1
    );
    assert_eq!(
        public
            .cat(&source_project, &package, "geectl-live-pkg.spec", Some(2))
            .expect("cat revision 2 on the live instance"),
        v2
    );
}

/// The directory listing and the project listing read back what was
/// uploaded and created, through the public reads.
#[test]
fn live_list_files_and_packages_read_back() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&authed, &package);
    authed
        .upload_file(
            &source_project,
            &package,
            "a.spec",
            "Name: a\nVersion: 1.0\n",
            &DryRun::new(false),
        )
        .expect("upload a.spec on the live instance");
    authed
        .upload_file(
            &source_project,
            &package,
            "b.spec",
            "Name: b\nVersion: 1.0\n",
            &DryRun::new(false),
        )
        .expect("upload b.spec on the live instance");
    let public = live_client(&base);
    let files = public
        .list_files(&source_project, &package)
        .expect("list the package files on the live instance");
    let names: Vec<&str> = files.iter().map(|file| file.name.as_str()).collect();
    assert!(
        names.contains(&"a.spec") && names.contains(&"b.spec"),
        "the listing must hold both uploaded files; got {names:?}"
    );
    let packages = public
        .list_packages(&source_project)
        .expect("list the project packages on the live instance");
    assert!(
        packages.contains(&package),
        "the project listing must hold the created package {package}; got {packages:?}"
    );
}

/// The package log comes back newest-first, and a revision range
/// narrows it to the revisions in the range.
#[test]
fn live_package_log_newest_first_and_range() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&authed, &package);
    authed
        .upload_file(
            &source_project,
            &package,
            "geectl-live-pkg.spec",
            "Name: geectl-live-pkg\nVersion: 1.0\n",
            &DryRun::new(false),
        )
        .expect("upload the first spec on the live instance");
    authed
        .upload_file(
            &source_project,
            &package,
            "geectl-live-pkg.spec",
            "Name: geectl-live-pkg\nVersion: 1.1\n",
            &DryRun::new(false),
        )
        .expect("upload the second spec on the live instance");
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
        .expect("read the ranged log on the live instance");
    assert_eq!(
        first_only.iter().map(|entry| entry.rev).collect::<Vec<_>>(),
        vec![1],
        "the range must narrow the log to revision 1"
    );
}

/// The revision patch diffs one revision against the previous one,
/// and revision 1 has nothing to diff against.
#[test]
fn live_revision_patch_diffs_revisions() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&authed, &package);
    authed
        .upload_file(
            &source_project,
            &package,
            "geectl-live-pkg.spec",
            "Name: geectl-live-pkg\nVersion: 1.0\n",
            &DryRun::new(false),
        )
        .expect("upload the first spec on the live instance");
    authed
        .upload_file(
            &source_project,
            &package,
            "geectl-live-pkg.spec",
            "Name: geectl-live-pkg\nVersion: 1.1\n",
            &DryRun::new(false),
        )
        .expect("upload the second spec on the live instance");
    let public = live_client(&base);
    let patch = public
        .revision_patch(&source_project, &package, 2)
        .expect("diff revision 2 on the live instance");
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
        .expect("diff revision 1 on the live instance");
    assert!(
        first.files.is_empty(),
        "revision 1 has nothing to diff against"
    );
}

/// The project and package meta read back what creation put there.
#[test]
fn live_meta_reads_back() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&authed, &package);
    let public = live_client(&base);
    let project_meta = public
        .project_meta(&source_project)
        .expect("read the project meta on the live instance");
    assert_eq!(project_meta.name, source_project);
    assert_eq!(
        project_meta.title.as_deref(),
        Some("geectl live test branch")
    );
    let package_meta = public
        .package_meta(&source_project, &package)
        .expect("read the package meta on the live instance");
    assert_eq!(package_meta.name, package);
    assert_eq!(package_meta.project, source_project);
    assert_eq!(
        package_meta.title.as_deref(),
        Some("geectl live test package")
    );
}

/// The package and project search find what was just created.
#[test]
fn live_search_finds_package_and_project() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&authed, &package);
    let matches = authed
        .search_packages(&package)
        .expect("search packages on the live instance");
    assert!(
        matches
            .iter()
            .any(|found| found.name == package && found.project == source_project),
        "the search must find the created package {package}; got {matches:?}"
    );
    let projects = authed
        .search_projects(&source_project)
        .expect("search projects on the live instance");
    assert!(
        projects.iter().any(|found| found.name == source_project),
        "the search must find the created project {source_project}; got {projects:?}"
    );
}

/// The owner search runs against the real server: a fresh package
/// has no explicit owners, so the call succeeding (not 401, not
/// 404) is the assertion, plus the shape of whatever it returns.
#[test]
fn live_owners_call_succeeds() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&authed, &package);
    let owners = authed
        .owners(&source_project, &package)
        .expect("owner search on the live instance");
    assert!(
        owners.iter().all(|owner| !owner.project.is_empty()),
        "every returned owner must name a project; got {owners:?}"
    );
}

/// `whois` reads the bootstrap admin back from the server.
#[test]
fn live_whois_reads_admin() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let person = authed.whois("Admin").expect("whois on the live instance");
    assert_eq!(person.login, "Admin");
    assert_eq!(person.state, "confirmed");
}

/// A `_link` file uploaded into a package reads back as its link
/// target. The target is a second package in the same project, so
/// the link resolves.
#[test]
fn live_link_target_reads_linkinfo() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&authed, &package);
    let target_package = format!("geectl-live-link-tgt-{}", unique_suffix());
    add_package(
        &authed,
        &source_project,
        &target_package,
        "Name: target\nVersion: 1.0\n",
    );
    let link_package = format!("geectl-live-link-{}", unique_suffix());
    authed
        .create_package(
            &source_project,
            &link_package,
            "geectl live test package",
            "Created by the geectl live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create the link package on the live instance");
    authed
        .upload_file(
            &source_project,
            &link_package,
            "_link",
            &format!("<link project=\"{source_project}\" package=\"{target_package}\"/>"),
            &DryRun::new(false),
        )
        .expect("upload the _link file on the live instance");
    let public = live_client(&base);
    assert_eq!(
        wait_until_link_target(&public, &source_project, &link_package),
        (source_project, target_package)
    );
}

/// Two projects diffed package by package: identical, differs,
/// only-in-old, and only-in-new all classify.
#[test]
fn live_project_diff_classifies() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let suffix = unique_suffix();
    let old_project = format!("geectl-tmp-diff-old-{suffix}");
    let new_project = format!("geectl-tmp-diff-new-{suffix}");
    for project in [&old_project, &new_project] {
        authed
            .create_project(
                project,
                "geectl live test project",
                "Created by the geectl live test; the instance is destroyed after the run.",
                &DryRun::new(false),
            )
            .expect("create a diff project on the live instance");
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
    let public = live_client(&base);
    let diffs = public
        .project_diff(&old_project, &new_project, &[], &[], false)
        .expect("diff the two projects on the live instance");
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
fn live_show_request_reads_back_the_filed_request() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (id, package) = file_live_request(&client, "2.0");
    let request = client
        .show_request(&id)
        .expect("read the filed request back from the server");
    assert_eq!(request.id.to_string(), id);
    assert_eq!(request.creator, "Admin");
    assert_eq!(request.state.name, RequestState::New);
    assert_eq!(
        request.description,
        format!("Update {package} to 2.0: live test submission")
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
fn live_request_log_lists_the_creation_entry() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (id, package) = file_live_request(&client, "2.0");
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
        format!("Update {package} to 2.0: live test submission")
    );
}

/// Accepting a request through the library moves it to `accepted` on
/// the server.
#[test]
fn live_accept_request_changes_state_on_server() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (id, _) = file_live_request(&client, "2.0");
    let dry = DryRun::new(false);
    client
        .accept_request(&id, "accepted by the geectl live test", false, &dry)
        .expect("accept the request on the live instance");
    let request = client
        .show_request(&id)
        .expect("read the accepted request back from the server");
    assert_eq!(request.state.name, RequestState::Accepted);
}

/// Declining a request through the library moves it to `declined` on
/// the server.
#[test]
fn live_decline_request_changes_state_on_server() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (id, _) = file_live_request(&client, "2.0");
    let dry = DryRun::new(false);
    client
        .decline_request(&id, "declined by the geectl live test", &dry)
        .expect("decline the request on the live instance");
    let request = client
        .show_request(&id)
        .expect("read the declined request back from the server");
    assert_eq!(request.state.name, RequestState::Declined);
}

/// Revoking a request through the library moves it to `revoked` on
/// the server.
#[test]
fn live_revoke_request_changes_state_on_server() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (id, _) = file_live_request(&client, "2.0");
    let dry = DryRun::new(false);
    client
        .revoke_request(&id, "revoked by the geectl live test", &dry)
        .expect("revoke the request on the live instance");
    let request = client
        .show_request(&id)
        .expect("read the revoked request back from the server");
    assert_eq!(request.state.name, RequestState::Revoked);
}

/// Asking for a review through the library moves the request to
/// `review` on the server, with the review carrying who it was asked
/// of and why.
#[test]
fn live_add_review_marks_the_request_for_review() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (id, _) = file_live_request(&client, "2.0");
    let dry = DryRun::new(false);
    let target = ReviewTarget::new(Some("Admin"), None, None, None)
        .expect("a review asked of a user is a valid target");
    client
        .add_review(&id, &target, "please review", &dry)
        .expect("ask Admin to review the request on the live instance");
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
fn live_review_with_history_reads_back() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (id, _) = file_live_request(&client, "2.0");
    let dry = DryRun::new(false);
    let target = ReviewTarget::new(Some("Admin"), None, None, None)
        .expect("a review asked of a user is a valid target");
    client
        .add_review(&id, &target, "please review", &dry)
        .expect("ask Admin to review the request on the live instance");
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
fn live_comment_create_and_list_round_trips() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (id, _) = file_live_request(&client, "2.0");
    let dry = DryRun::new(false);
    client
        .add_comment(&id, "first comment", None, &dry)
        .expect("comment on the request on the live instance");
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
        .expect("reply to the comment on the live instance");
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
fn live_request_diff_returns_the_submit_diff() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (id, _) = file_live_request_with_file(
        &client,
        "2.0",
        "geectl-live.txt",
        "geectl live diff fixture\n",
    );
    let plain = client
        .request_diff(&id, false)
        .expect("read the plain diff of the filed request");
    assert!(
        plain.contains("geectl-live.txt"),
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
fn live_search_requests_finds_the_filed_request() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (id, package) = file_live_request(&client, "2.0");
    let search = RequestSearch::new()
        .request_ids(vec![id.parse().expect("the filed request id is a number")])
        .types(vec!["submit".to_string()]);
    let found = client
        .search_requests(&search)
        .expect("search by id and type on the live instance");
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
        .expect("search by id and source package on the live instance");
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
        .expect("search by id and target package on the live instance");
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
        .expect("search excluding the filed request's target on the live instance");
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
        .expect("search excluding another project on the live instance");
    assert_eq!(
        not_excluded.len(),
        1,
        "excluding another project must not hide the filed request"
    );
    let by_creator = client
        .search_requests(&RequestSearch::new().creators(vec!["Admin".to_string()]))
        .expect("search by creator on the live instance");
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
fn live_try_change_state_false_on_an_accepted_request() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (id, _) = file_live_request(&client, "2.0");
    let dry = DryRun::new(false);
    client
        .accept_request(&id, "accepted by the geectl live test", false, &dry)
        .expect("accept the request on the live instance");
    assert!(
        !client
            .try_change_state(&id, RequestState::Accepted, "accept again", false, &dry)
            .expect("the lenient change reports false, not an error"),
        "accepting an accepted request must report false"
    );
}

/// Read a source file back from the server, authenticated.
fn read_file(base: &str, project: &str, package: &str, filename: &str) -> String {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    http.get_with(
        &format!("{base}/source/{project}/{package}/{filename}"),
        &live_credentials(),
    )
    .expect("GET the file back from the server")
}

/// Read a source file back from the server through a link's
/// expansion, authenticated. A plain file GET serves the package's
/// own files only; the expanded read is what a link serves.
fn read_expanded_file(base: &str, project: &str, package: &str, filename: &str) -> String {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    http.get_with(
        &format!("{base}/source/{project}/{package}/{filename}?expand=1"),
        &live_credentials(),
    )
    .expect("GET the expanded file back from the server")
}

/// Read a package's `_meta` back from the server, authenticated.
fn read_meta(base: &str, project: &str, package: &str) -> String {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    http.get_with(
        &format!("{base}/source/{project}/{package}/_meta"),
        &live_credentials(),
    )
    .expect("GET the _meta back from the server")
}

/// Create a source project with a package holding one spec file.
/// Returns `(source_project, package, spec_filename, spec_content)`.
fn setup_copy_source(client: &ObsClient<'_>) -> (String, String, String, String) {
    let package = live_package();
    let source_project = create_source_branch(client, &package);
    let filename = format!("{package}.spec");
    let content =
        "Name: live-test\nVersion: 1.0\nRelease: 1\nSummary: geectl live copy test\nLicense: MIT\n";
    client
        .upload_file(
            &source_project,
            &package,
            &filename,
            content,
            &DryRun::new(false),
        )
        .expect("upload a spec file into the source package");
    (source_project, package, filename, content.to_string())
}

#[test]
fn live_copy_package_to_target_project() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, package, filename, content) = setup_copy_source(&client);
    let target_project = format!("geectl-tmp-copy-{}", unique_suffix());
    client
        .create_project(
            &target_project,
            "geectl live test copy target",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create copy target project on the live instance");
    // Copy to the target project under the same package name: the
    // file must read back, and the server must have built the target
    // meta from the source meta.
    let revision = client
        .copy_package(
            &source_project,
            &package,
            &target_project,
            &package,
            &CopyOptions::default(),
            &DryRun::new(false),
        )
        .expect("copy a package on the live instance")
        .expect("live copy returns a revision");
    assert!(revision >= 1, "the copy must record a revision");
    assert_eq!(
        read_file(&base, &target_project, &package, &filename),
        content,
        "the copied file must read back with its content"
    );
    assert!(
        read_meta(&base, &target_project, &package)
            .contains("<title>geectl live test package</title>"),
        "the target meta must carry the source title"
    );
}

#[test]
fn live_copy_package_with_rename() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, package, filename, content) = setup_copy_source(&client);
    let target_project = format!("geectl-tmp-copy-{}", unique_suffix());
    client
        .create_project(
            &target_project,
            "geectl live test copy target",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create copy target project on the live instance");
    // Copy under another name: the renamed package holds the file.
    let renamed = format!("{package}-renamed");
    client
        .copy_package(
            &source_project,
            &package,
            &target_project,
            &renamed,
            &CopyOptions::default(),
            &DryRun::new(false),
        )
        .expect("copy a package with a rename on the live instance");
    assert_eq!(
        read_file(&base, &target_project, &renamed, &filename),
        content,
        "the renamed copy must hold the file"
    );
}

#[test]
fn live_copy_self_with_keep_link_and_revision() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, package, _filename, _content) = setup_copy_source(&client);
    // A link package pointing at the source package, made by writing
    // the _link directly (test setup, like set_project_link): the
    // library link operation lands in a later commit.
    let link_package = format!("{package}-link");
    client
        .create_package(
            &source_project,
            &link_package,
            "geectl live test link package",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create link package on the live instance");
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    http.put_file(
        &format!("{base}/source/{source_project}/{link_package}/_link"),
        &format!("<link project=\"{source_project}\" package=\"{package}\"/>"),
        &live_credentials(),
    )
    .expect("PUT the _link into the link package");
    // Self-copy with keep-link and a revision succeeds, and the link
    // survives the copy.
    let options = CopyOptions {
        keep_link: true,
        revision: Some("1".to_string()),
        ..CopyOptions::default()
    };
    client
        .copy_package(
            &source_project,
            &link_package,
            &source_project,
            &link_package,
            &options,
            &DryRun::new(false),
        )
        .expect("self-copy a link package with keep-link and a revision");
    let link = read_link(&base, &source_project, &link_package);
    assert_eq!(
        xml_attr_after(&link, "<link ", "project").as_deref(),
        Some(source_project.as_str()),
        "the link must survive the self-copy; link: {link}"
    );
}

#[test]
fn live_copy_self_without_flags_is_refused() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, package, _filename, _content) = setup_copy_source(&client);
    // A self-copy with nothing to copy from is refused client-side,
    // before any request reaches the server.
    let err = client
        .copy_package(
            &source_project,
            &package,
            &source_project,
            &package,
            &CopyOptions::default(),
            &DryRun::new(false),
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

#[test]
fn live_copy_project_with_no_delay() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, package, filename, content) = setup_copy_source(&client);
    let target_project = format!("geectl-tmp-copy-{}", unique_suffix());
    // Copy the whole project synchronously: the target project, its
    // package, and the file must all be there afterwards.
    let options = CopyProjectOptions {
        no_delay: true,
        ..CopyProjectOptions::default()
    };
    client
        .copy_project(
            &source_project,
            &target_project,
            &options,
            &DryRun::new(false),
        )
        .expect("copy a project on the live instance");
    assert!(
        public_path_exists(&base, &format!("{target_project}/{package}")),
        "the copied project must hold the package"
    );
    assert_eq!(
        read_file(&base, &target_project, &package, &filename),
        content,
        "the copied project must hold the file"
    );
}

#[test]
fn live_set_devel_project_sets_devel() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let package = live_package();
    let factory = create_source_branch(&client, &package);
    let devel = format!("geectl-tmp-devel-{}", unique_suffix());
    client
        .create_project(
            &devel,
            "geectl live test devel",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create devel project on the live instance");
    // The server refuses a devel whose package does not exist in
    // the devel project, so the devel package is created there
    // first.
    client
        .create_package(
            &devel,
            &package,
            "geectl live test devel package",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create devel package on the live instance");
    client
        .set_devel_project(&factory, &package, Some(&devel), None, &DryRun::new(false))
        .expect("set the devel project on the live instance");
    let meta = read_meta(&base, &factory, &package);
    // The server records the devel with the package attribute too
    // (defaulting to the same package name), so match the project
    // attribute rather than the whole element.
    assert_eq!(
        xml_attr_after(&meta, "<devel ", "project").as_deref(),
        Some(devel.as_str()),
        "the meta must carry the devel project; meta: {meta}"
    );
}

#[test]
fn live_set_devel_project_cross_package() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let package = live_package();
    let factory = create_source_branch(&client, &package);
    let devel = format!("geectl-tmp-devel-{}", unique_suffix());
    client
        .create_project(
            &devel,
            "geectl live test devel",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create devel project on the live instance");
    // A cross-package mapping: this package's devel is another
    // package in the devel project.
    let devel_package = format!("{package}-devel");
    // The server refuses a devel whose package does not exist in
    // the devel project, so the devel package is created there
    // first.
    client
        .create_package(
            &devel,
            &devel_package,
            "geectl live test devel package",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create devel package on the live instance");
    client
        .set_devel_project(
            &factory,
            &package,
            Some(&devel),
            Some(&devel_package),
            &DryRun::new(false),
        )
        .expect("set the cross-package devel on the live instance");
    let meta = read_meta(&base, &factory, &package);
    assert!(
        meta.contains(&format!(
            "<devel project=\"{devel}\" package=\"{devel_package}\"/>"
        )),
        "the meta must carry the cross-package devel; meta: {meta}"
    );
}

#[test]
fn live_set_devel_project_unset() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let package = live_package();
    let factory = create_source_branch(&client, &package);
    let devel = format!("geectl-tmp-devel-{}", unique_suffix());
    client
        .create_project(
            &devel,
            "geectl live test devel",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create devel project on the live instance");
    // The server refuses a devel whose package does not exist in
    // the devel project, so the devel package is created there
    // first.
    client
        .create_package(
            &devel,
            &package,
            "geectl live test devel package",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create devel package on the live instance");
    client
        .set_devel_project(&factory, &package, Some(&devel), None, &DryRun::new(false))
        .expect("set the devel project on the live instance");
    client
        .set_devel_project(&factory, &package, None, None, &DryRun::new(false))
        .expect("unset the devel project on the live instance");
    let meta = read_meta(&base, &factory, &package);
    assert!(
        !meta.contains("<devel"),
        "the meta must carry no devel element after unsetting; meta: {meta}"
    );
}

/// PUT a package `_meta` carrying a `<lock><enable/></lock>`, so the
/// package is locked. The harness writes the meta directly (test
/// setup, like `set_package_devel`); the library operation under test
/// is `link_package`, which must not carry the lock into the target.
fn set_package_lock(base: &str, project: &str, package: &str) {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    let meta = format!(
        "<package name=\"{package}\" project=\"{project}\"><title>geectl live test package</title><description>Created by the geectl live live test; the instance is destroyed after the run.</description><lock><enable/></lock></package>"
    );
    http.put(
        &format!("{base}/source/{project}/{package}/_meta"),
        &meta,
        &live_credentials(),
    )
    .expect("PUT the package meta with a lock");
}

#[test]
fn live_link_package_creates_link() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, target_project, package) = setup_branch(&client);
    client
        .link_package(
            &source_project,
            &package,
            &target_project,
            &package,
            &LinkOptions::default(),
            &DryRun::new(false),
        )
        .expect("link a package on the live instance");
    let link = read_link(&base, &target_project, &package);
    assert_eq!(
        xml_attr(&link, "project").as_deref(),
        Some(source_project.as_str()),
        "the link must point at the source project; link: {link}"
    );
    assert_eq!(
        xml_attr(&link, "package").as_deref(),
        Some(package.as_str()),
        "the link must point at the source package; link: {link}"
    );
    // The target meta exists and carries the source's title: the
    // link was built from the source meta.
    let meta = read_meta(&base, &target_project, &package);
    assert!(
        meta.contains("<title>geectl live test package</title>"),
        "the target meta must carry the source title; meta: {meta}"
    );
}

#[test]
fn live_link_package_disable_build() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, target_project, package) = setup_branch(&client);
    let options = LinkOptions {
        disable_build: true,
        ..LinkOptions::default()
    };
    client
        .link_package(
            &source_project,
            &package,
            &target_project,
            &package,
            &options,
            &DryRun::new(false),
        )
        .expect("link a package with build disabled on the live instance");
    let meta = read_meta(&base, &target_project, &package);
    // The server pretty-prints the meta, so the elements are
    // whitespace-separated; compare with the whitespace removed.
    let flat: String = meta.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        flat.contains("<build><disable/></build>"),
        "the target meta must disable building; meta: {meta}"
    );
}

#[test]
fn live_link_package_disable_publish() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, target_project, package) = setup_branch(&client);
    let options = LinkOptions {
        disable_publish: true,
        ..LinkOptions::default()
    };
    client
        .link_package(
            &source_project,
            &package,
            &target_project,
            &package,
            &options,
            &DryRun::new(false),
        )
        .expect("link a package with publishing disabled on the live instance");
    let meta = read_meta(&base, &target_project, &package);
    // The server pretty-prints the meta, so the elements are
    // whitespace-separated; compare with the whitespace removed.
    let flat: String = meta.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        flat.contains("<publish><disable/></publish>"),
        "the target meta must disable publishing; meta: {meta}"
    );
}

#[test]
fn live_link_package_locked_source_not_inherited() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, target_project, package) = setup_branch(&client);
    set_package_lock(&base, &source_project, &package);
    client
        .link_package(
            &source_project,
            &package,
            &target_project,
            &package,
            &LinkOptions::default(),
            &DryRun::new(false),
        )
        .expect("link a locked package on the live instance");
    let meta = read_meta(&base, &target_project, &package);
    assert!(
        !meta.contains("<lock"),
        "the target meta must not inherit the source lock; meta: {meta}"
    );
}

#[test]
fn live_link_package_project_link_target_takes_explicit_link() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&client, &package);
    let target_project = format!("geectl-tmp-link-{}", unique_suffix());
    client
        .create_project(
            &target_project,
            "geectl live test link target",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create link target project on the live instance");
    // The target project links at the source project, so the package
    // is inherited there — the link must still be created explicitly.
    set_project_link(&base, &target_project, &source_project);
    client
        .link_package(
            &source_project,
            &package,
            &target_project,
            &package,
            &LinkOptions::default(),
            &DryRun::new(false),
        )
        .expect("link a package into a project-linked target on the live instance");
    let link = read_link(&base, &target_project, &package);
    assert_eq!(
        xml_attr(&link, "project").as_deref(),
        Some(source_project.as_str()),
        "the target must carry the explicit link; link: {link}"
    );
    assert_eq!(
        xml_attr(&link, "package").as_deref(),
        Some(package.as_str()),
        "the link must point at the source package; link: {link}"
    );
}

/// PUT a package's `_link` directly. The harness writes the link
/// (test setup, like `set_project_link`): breaking a link with a
/// bad patch is the setup; the library operation is `repair_link`.
fn put_link(base: &str, project: &str, package: &str, link: &str) {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    http.put_file(
        &format!("{base}/source/{project}/{package}/_link"),
        link,
        &live_credentials(),
    )
    .expect("PUT the _link on the live instance");
}

/// Read a package's directory listing back from the server,
/// authenticated, at the latest revision.
fn read_directory(base: &str, project: &str, package: &str) -> String {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    http.get_with(
        &format!("{base}/source/{project}/{package}?rev=latest"),
        &live_credentials(),
    )
    .expect("GET the directory back from the server")
}

#[test]
fn live_repair_link_repairs_broken_link() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, package, filename, content) = setup_copy_source(&client);
    let target_project = format!("geectl-tmp-repair-{}", unique_suffix());
    client
        .create_project(
            &target_project,
            "geectl live test repair target",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create repair target project on the live instance");
    client
        .link_package(
            &source_project,
            &package,
            &target_project,
            &package,
            &LinkOptions::default(),
            &DryRun::new(false),
        )
        .expect("link a package on the live instance");
    // Break the link: a patch file that is not a patch at all, plus
    // an apply entry for it. The link stops expanding.
    client
        .upload_file(
            &target_project,
            &package,
            "bad.patch",
            "this is not a patch\n",
            &DryRun::new(false),
        )
        .expect("upload a bad patch file into the linked package");
    put_link(
        &base,
        &target_project,
        &package,
        &format!(
            "<link project=\"{source_project}\" package=\"{package}\"><patches><apply name=\"bad.patch\"/></patches></link>"
        ),
    );
    let directory = read_directory(&base, &target_project, &package);
    assert!(
        directory.contains("error=\""),
        "the link must be broken before the repair; directory: {directory}"
    );
    client
        .repair_link(&target_project, &package, &DryRun::new(false))
        .expect("repair a broken link on the live instance");
    // The repaired link expands again, carries no apply entry, and
    // is pinned at a baserev.
    let directory = read_directory(&base, &target_project, &package);
    assert!(
        !directory.contains("error=\""),
        "the link must expand after the repair; directory: {directory}"
    );
    let link = read_link(&base, &target_project, &package);
    assert!(
        !link.contains("<apply"),
        "the repaired link must carry no apply patch; link: {link}"
    );
    assert!(
        link.contains("baserev=\""),
        "the repaired link must be pinned at a baserev; link: {link}"
    );
    // The spec file from the source still reads back through the
    // repaired link.
    assert_eq!(
        read_expanded_file(&base, &target_project, &package, &filename),
        content,
        "the source file must read back through the repaired link"
    );
}

/// Create a source project with a package holding one spec file
/// and link it into a fresh target project. Returns
/// `(source_project, target_project, package)`.
fn setup_link(client: &ObsClient<'_>) -> (String, String, String) {
    let (source_project, package, _, _) = setup_copy_source(client);
    let target_project = format!("geectl-tmp-setlinkrev-{}", unique_suffix());
    client
        .create_project(
            &target_project,
            "geectl live test link-revision target",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create link-revision target project on the live instance");
    client
        .link_package(
            &source_project,
            &package,
            &target_project,
            &package,
            &LinkOptions::default(),
            &DryRun::new(false),
        )
        .expect("link a package on the live instance");
    (source_project, target_project, package)
}

#[test]
fn live_set_link_rev_latest() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, target_project, package) = setup_link(&client);
    let source_dir = read_directory(&base, &source_project, &package);
    let expected = xml_attr(&source_dir, "rev").expect("the source directory carries a rev");
    let rev = client
        .set_link_rev(
            &target_project,
            &package,
            &LinkRevision::Latest,
            &DryRun::new(false),
        )
        .expect("set the link revision on the live instance");
    assert_eq!(
        rev.as_deref(),
        Some(expected.as_str()),
        "the resolved revision must be the target's revision"
    );
    let link = read_link(&base, &target_project, &package);
    assert_eq!(
        xml_attr(&link, "rev").as_deref(),
        Some(expected.as_str()),
        "the link must carry the target's revision; link: {link}"
    );
}

#[test]
fn live_set_link_rev_pinned() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, target_project, package) = setup_link(&client);
    // Pin to the source's actual current revision: the source is
    // created with a single revision, so a hardcoded "2" names a
    // revision that does not exist.
    let source_dir = read_directory(&base, &source_project, &package);
    let pinned = xml_attr(&source_dir, "rev").expect("the source directory carries a rev");
    let rev = client
        .set_link_rev(
            &target_project,
            &package,
            &LinkRevision::Pinned(pinned.clone()),
            &DryRun::new(false),
        )
        .expect("pin the link revision on the live instance");
    assert_eq!(rev.as_deref(), Some(pinned.as_str()));
    let link = read_link(&base, &target_project, &package);
    assert_eq!(
        xml_attr(&link, "rev").as_deref(),
        Some(pinned.as_str()),
        "the link must carry the pinned revision; link: {link}"
    );
    // The pinned revision is the target's current revision, so the
    // link still expands.
    let directory = read_directory(&base, &target_project, &package);
    assert!(
        !directory.contains("error=\""),
        "the pinned link must expand; directory: {directory}"
    );
}

#[test]
fn live_set_link_rev_unset() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (_, target_project, package) = setup_link(&client);
    client
        .set_link_rev(
            &target_project,
            &package,
            &LinkRevision::Latest,
            &DryRun::new(false),
        )
        .expect("set the link revision on the live instance");
    let rev = client
        .set_link_rev(
            &target_project,
            &package,
            &LinkRevision::Unset,
            &DryRun::new(false),
        )
        .expect("unset the link revision on the live instance");
    assert_eq!(rev, None);
    let link = read_link(&base, &target_project, &package);
    assert!(
        !link.contains(" rev=\""),
        "the link must carry no revision after unsetting; link: {link}"
    );
}

#[test]
fn live_set_link_rev_expanded() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (_, target_project, package) = setup_link(&client);
    let rev = client
        .set_link_rev(
            &target_project,
            &package,
            &LinkRevision::Expanded,
            &DryRun::new(false),
        )
        .expect("set the expanded link revision on the live instance")
        .expect("expanding sets a revision");
    assert_eq!(rev.len(), 32, "the expanded revision is an md5; rev: {rev}");
    let link = read_link(&base, &target_project, &package);
    assert_eq!(
        xml_attr(&link, "rev").as_deref(),
        Some(rev.as_str()),
        "the link must carry the expanded revision; link: {link}"
    );
    assert_eq!(
        xml_attr(&link, "vrev").as_deref(),
        Some("1"),
        "the expanded revision carries a vrev; link: {link}"
    );
}

#[test]
fn live_set_link_rev_project() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (_, target_project, package) = setup_link(&client);
    let set = client
        .set_link_rev_project(&target_project, &LinkRevision::Latest, &DryRun::new(false))
        .expect("set the project link revisions on the live instance");
    assert_eq!(set.len(), 1, "the project holds one linked package");
    assert_eq!(set[0].0, package);
    assert!(set[0].1.is_some());
    let link = read_link(&base, &target_project, &package);
    assert_eq!(
        xml_attr(&link, "rev").as_deref(),
        set[0].1.as_deref(),
        "the link must carry the resolved revision; link: {link}"
    );
}

#[test]
fn live_set_link_rev_project_unset() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (_, target_project, package) = setup_link(&client);
    client
        .set_link_rev(
            &target_project,
            &package,
            &LinkRevision::Latest,
            &DryRun::new(false),
        )
        .expect("set the link revision on the live instance");
    let set = client
        .set_link_rev_project(&target_project, &LinkRevision::Unset, &DryRun::new(false))
        .expect("unset the project link revisions on the live instance");
    assert_eq!(set.len(), 1, "the project holds one linked package");
    assert_eq!(set[0], (package.clone(), None));
    let link = read_link(&base, &target_project, &package);
    assert!(
        !link.contains(" rev=\""),
        "the link must carry no revision after unsetting; link: {link}"
    );
}

#[test]
fn live_link_to_branch() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, target_project, package) = setup_link(&client);
    client
        .link_to_branch(&target_project, &package, &DryRun::new(false))
        .expect("convert the link to a branch on the live instance");
    let link = read_link(&base, &target_project, &package);
    // The link still points at its target, now as a branch: the
    // server sets baserev and replaces the patches with a single
    // <branch/> entry.
    assert_eq!(
        xml_attr(&link, "project").as_deref(),
        Some(source_project.as_str()),
        "the branch link still points at the source project; link: {link}"
    );
    assert_eq!(
        xml_attr(&link, "package").as_deref(),
        Some(package.as_str()),
        "the branch link still points at the source package; link: {link}"
    );
    assert!(
        xml_attr(&link, "baserev").is_some(),
        "the branch link carries the baserev it expanded at; link: {link}"
    );
    assert!(
        link.contains("<branch"),
        "the branch link carries a branch patch entry; link: {link}"
    );
    // The linked sources are now the package's own files.
    let directory = read_directory(&base, &target_project, &package);
    assert!(
        directory.contains(&format!("name=\"{package}.spec\"")),
        "the expanded sources are the package's own files; directory: {directory}"
    );
}

#[test]
fn live_detach_branch() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (_, target_project, package) = setup_link(&client);
    let rev = client
        .detach_branch(&target_project, &package, &DryRun::new(false))
        .expect("detach the branch on the live instance");
    assert!(rev.is_some(), "detaching commits a new revision");
    let directory = read_directory(&base, &target_project, &package);
    assert!(
        directory.contains(&format!("name=\"{package}.spec\"")),
        "the expanded sources stay as the package's own files; directory: {directory}"
    );
    assert!(
        !directory.contains("name=\"_link\""),
        "the link is gone after the detach; directory: {directory}"
    );
}

#[test]
fn live_detach_branch_drops_link_when_target_removed() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, target_project, package) = setup_link(&client);
    client
        .delete_package(&source_project, &package, true, &DryRun::new(false))
        .expect("remove the link target on the live instance");
    // Dropping the link reports no revision: the API's file delete
    // answers status-ok, not a revision document.
    client
        .detach_branch(&target_project, &package, &DryRun::new(false))
        .expect("detach the branch with a removed target on the live instance");
    let directory = read_directory(&base, &target_project, &package);
    assert!(
        !directory.contains("name=\"_link\""),
        "the link is dropped when its target is gone; directory: {directory}"
    );
}

/// Whether the project still exists on the server, authenticated:
/// a GET of the project directory succeeds exactly when the project
/// is there.
fn project_exists(base: &str, project: &str) -> bool {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    match http.get_with(&format!("{base}/source/{project}"), &live_credentials()) {
        Ok(_) => true,
        Err(err) if err.kind() == ErrorKind::NotFound => false,
        Err(err) => panic!("GET /source/{project} failed unexpectedly: {err}"),
    }
}

/// Poll until the project is gone, for up to a minute: the project
/// itself is unlinked before the delete replies, but the poll covers
/// any lag. Panics naming the project when the deadline passes.
fn wait_until_project_gone(base: &str, project: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        if !project_exists(base, project) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    panic!("{project} still exists on the server 60s after the delete");
}

#[test]
fn live_delete_project_empty() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let project = format!("geectl-tmp-delproj-{}", unique_suffix());
    client
        .create_project(
            &project,
            "geectl live test delete target",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create the project on the live instance");
    client
        .delete_project(
            &project,
            &DeleteProjectOptions::default(),
            &DryRun::new(false),
        )
        .expect("delete the empty project on the live instance");
    wait_until_project_gone(&base, &project);
}

#[test]
fn live_delete_project_refuses_non_empty() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, package, _, _) = setup_copy_source(&client);
    let err = client
        .delete_project(
            &source_project,
            &DeleteProjectOptions::default(),
            &DryRun::new(false),
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    // The refusal deletes nothing: the package is still there.
    let meta = read_meta(&base, &source_project, &package);
    assert!(
        meta.contains(&format!("name=\"{package}\"")),
        "the package meta is still there after the refused delete; meta: {meta}"
    );
}

#[test]
fn live_delete_project_recursive() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, _, _, _) = setup_copy_source(&client);
    let options = DeleteProjectOptions {
        recursive: true,
        force: true,
        comment: Some("cleanup".to_string()),
    };
    client
        .delete_project(&source_project, &options, &DryRun::new(false))
        .expect("delete the project recursively on the live instance");
    wait_until_project_gone(&base, &source_project);
}

#[test]
fn live_undelete_package() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, package, filename, _) = setup_copy_source(&client);
    client
        .delete_package(&source_project, &package, true, &DryRun::new(false))
        .expect("delete the package on the live instance");
    client
        .undelete_package(&source_project, &package, &DryRun::new(false))
        .expect("restore the deleted package on the live instance");
    let meta = read_meta(&base, &source_project, &package);
    assert!(
        meta.contains(&format!("name=\"{package}\"")),
        "the package meta is back after the restore; meta: {meta}"
    );
    let spec = read_file(&base, &source_project, &package, &filename);
    assert!(
        spec.contains("Name: live-test"),
        "the package sources are back after the restore; spec: {spec}"
    );
}

#[test]
fn live_undelete_project() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, package, filename, _) = setup_copy_source(&client);
    let options = DeleteProjectOptions {
        recursive: true,
        ..DeleteProjectOptions::default()
    };
    client
        .delete_project(&source_project, &options, &DryRun::new(false))
        .expect("delete the project recursively on the live instance");
    // The server moves the packages to the deleted area after
    // replying to the delete, so the restore can race it: retry
    // until the project is restorable.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        match client.undelete_project(&source_project, &DryRun::new(false)) {
            Ok(()) => break,
            Err(err)
                if err.kind() == ErrorKind::NotFound && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
            Err(err) => panic!("restore the deleted project on the live instance: {err}"),
        }
    }
    let meta = read_meta(&base, &source_project, &package);
    assert!(
        meta.contains(&format!("name=\"{package}\"")),
        "the package meta is back after the restore; meta: {meta}"
    );
    let spec = read_file(&base, &source_project, &package, &filename);
    assert!(
        spec.contains("Name: live-test"),
        "the package sources are back after the restore; spec: {spec}"
    );
}

/// PUT a project `_meta` carrying a repository, so the project can
/// release from it. The harness writes the meta directly (test
/// setup, like `set_project_link`); the library operation under test
/// is `release_*`. `target` is the optional release target
/// (project, repository) the repository releases to.
fn set_project_repository(
    base: &str,
    project: &str,
    repository: &str,
    target: Option<(&str, &str)>,
) {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    let releasetarget = match target {
        Some((target_project, target_repository)) => format!(
            "<releasetarget project=\"{target_project}\" repository=\"{target_repository}\" trigger=\"manual\"/>"
        ),
        None => String::new(),
    };
    let meta = format!(
        "<project name=\"{project}\"><title>geectl live test project</title><description>Created by the geectl live live test; the instance is destroyed after the run.</description><repository name=\"{repository}\"><arch>x86_64</arch>{releasetarget}</repository></project>"
    );
    http.put(
        &format!("{base}/source/{project}/_meta"),
        &meta,
        &live_credentials(),
    )
    .expect("PUT the project meta with a repository");
}

/// The `aggregate_package` operation aggregates a package into a
/// fresh target package: the `_aggregate` file and the target meta
/// are read back from the server.
#[test]
fn live_aggregate_package() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, package, _filename, _content) = setup_copy_source(&client);
    let target_project = format!("geectl-tmp-agg-{}", unique_suffix());
    client
        .create_project(
            &target_project,
            "geectl live test aggregate target",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create aggregate target project on the live instance");
    client
        .aggregate_package(
            &source_project,
            &package,
            &target_project,
            &package,
            &AggregateOptions::default(),
            &DryRun::new(false),
        )
        .expect("aggregate a package on the live instance");
    let aggregate = read_file(&base, &target_project, &package, "_aggregate");
    assert!(aggregate.contains(&format!("<aggregate project=\"{source_project}\">")));
    assert!(aggregate.contains(&format!("<package>{package}</package>")));
    let meta = read_meta(&base, &target_project, &package);
    assert!(meta.contains(&format!("name=\"{package}\"")));
    assert!(meta.contains(&format!("project=\"{target_project}\"")));
    assert!(!meta.contains("scmsync"));
}

/// The `aggregate_package` operation aggregates into a renamed
/// target package: the `_aggregate` still names the source package,
/// the target meta carries the new name.
#[test]
fn live_aggregate_package_rename() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, package, _filename, _content) = setup_copy_source(&client);
    let target_project = format!("geectl-tmp-agg-{}", unique_suffix());
    client
        .create_project(
            &target_project,
            "geectl live test aggregate target",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create aggregate target project on the live instance");
    let target_package = format!("{package}-agg");
    client
        .aggregate_package(
            &source_project,
            &package,
            &target_project,
            &target_package,
            &AggregateOptions::default(),
            &DryRun::new(false),
        )
        .expect("aggregate a package under a new name on the live instance");
    let aggregate = read_file(&base, &target_project, &target_package, "_aggregate");
    assert!(aggregate.contains(&format!("<aggregate project=\"{source_project}\">")));
    assert!(aggregate.contains(&format!("<package>{package}</package>")));
    let meta = read_meta(&base, &target_project, &target_package);
    assert!(meta.contains(&format!("name=\"{target_package}\"")));
}

/// Aggregating a second time into a package that already has an
/// `_aggregate` file is refused and leaves the file unchanged.
#[test]
fn live_aggregate_package_refuses_existing() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, package, _filename, _content) = setup_copy_source(&client);
    let target_project = format!("geectl-tmp-agg-{}", unique_suffix());
    client
        .create_project(
            &target_project,
            "geectl live test aggregate target",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create aggregate target project on the live instance");
    client
        .aggregate_package(
            &source_project,
            &package,
            &target_project,
            &package,
            &AggregateOptions::default(),
            &DryRun::new(false),
        )
        .expect("aggregate a package on the live instance");
    let first = read_file(&base, &target_project, &package, "_aggregate");
    let err = client
        .aggregate_package(
            &source_project,
            &package,
            &target_project,
            &package,
            &AggregateOptions::default(),
            &DryRun::new(false),
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    let second = read_file(&base, &target_project, &package, "_aggregate");
    assert_eq!(second, first);
}

/// The `release_package` operation is accepted by the server for a
/// package with a repository and a release target.
#[test]
fn live_release_package() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, package, _filename, _content) = setup_copy_source(&client);
    let target_project = format!("geectl-tmp-rel-{}", unique_suffix());
    client
        .create_project(
            &target_project,
            "geectl live test release target",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create release target project on the live instance");
    // The target project's repository must exist before the
    // source project's meta names it as a release target.
    set_project_repository(&base, &target_project, "distro-1-ga", None);
    set_project_repository(
        &base,
        &source_project,
        "standard",
        Some((&target_project, "distro-1-ga")),
    );
    let options = ReleaseOptions {
        repository: Some("standard".to_string()),
        target_project: Some(target_project),
        target_repository: Some("distro-1-ga".to_string()),
        ..ReleaseOptions::default()
    };
    client
        .release_package(&source_project, &package, &options, &DryRun::new(false))
        .expect("release a package on the live instance");
}

/// The `release_project` operation is accepted by the server for a
/// project with a repository and a release target.
#[test]
fn live_release_project() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, _package, _filename, _content) = setup_copy_source(&client);
    let target_project = format!("geectl-tmp-rel-{}", unique_suffix());
    client
        .create_project(
            &target_project,
            "geectl live test release target",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create release target project on the live instance");
    // The target project's repository must exist before the
    // source project's meta names it as a release target.
    set_project_repository(&base, &target_project, "distro-1-ga", None);
    set_project_repository(
        &base,
        &source_project,
        "standard",
        Some((&target_project, "distro-1-ga")),
    );
    let options = ReleaseOptions {
        repository: Some("standard".to_string()),
        target_project: Some(target_project),
        target_repository: Some("distro-1-ga".to_string()),
        ..ReleaseOptions::default()
    };
    client
        .release_project(&source_project, &options, &DryRun::new(false))
        .expect("release a project on the live instance");
}

/// PUT a project `_meta` carrying a `<scmsync>`, so the project is
/// git-managed. The harness writes the meta directly (test setup,
/// like `set_project_link`); the library operation under test is
/// `create_package_with_options`.
fn set_project_scmsync(base: &str, project: &str, scmsync: &str) {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    let meta = format!(
        "<project name=\"{project}\"><title>geectl live test project</title><description>Created by the geectl live live test; the instance is destroyed after the run.</description><scmsync>{scmsync}</scmsync></project>"
    );
    http.put(
        &format!("{base}/source/{project}/_meta"),
        &meta,
        &live_credentials(),
    )
    .expect("PUT the project meta with a scmsync");
}

/// The `create_package_with_options` operation creates a package
/// wired to a git repository: the `<scmsync>` is read back from the
/// server meta.
#[test]
fn live_create_package_with_scmsync() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let project = format!("geectl-tmp-mkpkg-{}", unique_suffix());
    client
        .create_project(
            &project,
            "geectl live test package target",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create package target project on the live instance");
    let package = live_package();
    let options = CreatePackageOptions {
        scmsync: Some("https://mygitea1.com#main".to_string()),
        ..CreatePackageOptions::default()
    };
    client
        .create_package_with_options(&project, &package, "", "", &options, &DryRun::new(false))
        .expect("create a package with scmsync on the live instance");
    let meta = read_meta(&base, &project, &package);
    assert!(meta.contains("<scmsync>https://mygitea1.com#main</scmsync>"));
}

/// Creating the same package again with a different scmsync is
/// refused without force, and the first scmsync stays in place.
#[test]
fn live_create_package_with_options_refuses_existing() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let project = format!("geectl-tmp-mkpkg-{}", unique_suffix());
    client
        .create_project(
            &project,
            "geectl live test package target",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create package target project on the live instance");
    let package = live_package();
    let first = CreatePackageOptions {
        scmsync: Some("https://mygitea1.com#main".to_string()),
        ..CreatePackageOptions::default()
    };
    client
        .create_package_with_options(&project, &package, "", "", &first, &DryRun::new(false))
        .expect("create a package with scmsync on the live instance");
    let second = CreatePackageOptions {
        scmsync: Some("https://mygitea2.com#main".to_string()),
        ..CreatePackageOptions::default()
    };
    let err = client
        .create_package_with_options(&project, &package, "", "", &second, &DryRun::new(false))
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    let meta = read_meta(&base, &project, &package);
    assert!(meta.contains("<scmsync>https://mygitea1.com#main</scmsync>"));
    assert!(!meta.contains("mygitea2"));
}

/// Creating the same package again with force overwrites the
/// scmsync.
#[test]
fn live_create_package_with_options_force_overwrites() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let project = format!("geectl-tmp-mkpkg-{}", unique_suffix());
    client
        .create_project(
            &project,
            "geectl live test package target",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create package target project on the live instance");
    let package = live_package();
    let first = CreatePackageOptions {
        scmsync: Some("https://mygitea1.com#main".to_string()),
        ..CreatePackageOptions::default()
    };
    client
        .create_package_with_options(&project, &package, "", "", &first, &DryRun::new(false))
        .expect("create a package with scmsync on the live instance");
    let second = CreatePackageOptions {
        scmsync: Some("https://mygitea2.com#main".to_string()),
        force: true,
    };
    client
        .create_package_with_options(&project, &package, "", "", &second, &DryRun::new(false))
        .expect("overwrite a package with force on the live instance");
    let meta = read_meta(&base, &project, &package);
    assert!(meta.contains("<scmsync>https://mygitea2.com#main</scmsync>"));
    assert!(!meta.contains("mygitea1"));
}

/// The `create_package_with_options` operation creates a plain
/// package without a scmsync, like `create_package`.
#[test]
fn live_create_package_with_options_plain() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let project = format!("geectl-tmp-mkpkg-{}", unique_suffix());
    client
        .create_project(
            &project,
            "geectl live test package target",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create package target project on the live instance");
    let package = live_package();
    client
        .create_package_with_options(
            &project,
            &package,
            "",
            "",
            &CreatePackageOptions::default(),
            &DryRun::new(false),
        )
        .expect("create a plain package on the live instance");
    let meta = read_meta(&base, &project, &package);
    assert!(!meta.contains("scmsync"));
}

/// An empty package name is refused without any request.
#[test]
fn live_create_package_with_options_empty_name() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let project = format!("geectl-tmp-mkpkg-{}", unique_suffix());
    client
        .create_project(
            &project,
            "geectl live test package target",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create package target project on the live instance");
    let err = client
        .create_package_with_options(
            &project,
            "",
            "",
            "",
            &CreatePackageOptions::default(),
            &DryRun::new(false),
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

/// Creating a package in a project that does not exist fails, even
/// with force.
#[test]
fn live_create_package_with_options_missing_project() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let project = format!("geectl-tmp-mkpkg-nope-{}", unique_suffix());
    let package = live_package();
    for options in [
        CreatePackageOptions::default(),
        CreatePackageOptions {
            force: true,
            ..CreatePackageOptions::default()
        },
    ] {
        let err = client
            .create_package_with_options(&project, &package, "", "", &options, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }
}

/// Creating a package by hand in a git-managed project is refused:
/// the project's packages are created by the sync, not by hand.
#[test]
fn live_create_package_with_options_refuses_git_project() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let project = format!("geectl-tmp-mkpkg-{}", unique_suffix());
    client
        .create_project(
            &project,
            "geectl live test package target",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create package target project on the live instance");
    set_project_scmsync(&base, &project, "https://example.com#main");
    let package = live_package();
    let err = client
        .create_package_with_options(
            &project,
            &package,
            "",
            "",
            &CreatePackageOptions::default(),
            &DryRun::new(false),
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
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
    fn live_base_validation() {
        // Bare loopback bases pass, trimmed and slash-stripped.
        assert_eq!(
            validate_live_base("http://127.0.0.1:3000").as_deref(),
            Ok("http://127.0.0.1:3000")
        );
        assert_eq!(
            validate_live_base("  http://127.0.0.1:3000/  ").as_deref(),
            Ok("http://127.0.0.1:3000")
        );
        // A space before the trailing slash is stripped too.
        assert_eq!(
            validate_live_base("http://127.0.0.1:3000 /").as_deref(),
            Ok("http://127.0.0.1:3000")
        );
        // A base with a path, query, or fragment is rejected: the
        // tests append paths themselves, so a non-bare base would
        // double them.
        assert!(validate_live_base("http://127.0.0.1:3000/about").is_err());
        assert!(validate_live_base("http://127.0.0.1:3000?x=1").is_err());
        assert!(validate_live_base("http://127.0.0.1:3000#frag").is_err());
        // Non-loopback is rejected, including behind userinfo.
        assert!(validate_live_base("http://example.com").is_err());
        assert!(validate_live_base("http://127.0.0.1:80@evil.com").is_err());
        // A userinfo-bearing loopback base is rejected outright:
        // credentials go through the auth provider, never the URL.
        assert!(validate_live_base("http://user:pass@127.0.0.1:3000").is_err());
        // No error reason echoes credentials: the userinfo is
        // stripped from the reported URL.
        let userinfo_err = validate_live_base("http://user:s3cret@127.0.0.1:3000").unwrap_err();
        assert!(
            !userinfo_err.contains("s3cret"),
            "an error reason must not echo credentials; got {userinfo_err}"
        );
        assert!(
            userinfo_err.contains("http://127.0.0.1:3000"),
            "the stripped URL is still reported; got {userinfo_err}"
        );
        let userinfo_scheme_err =
            validate_live_base("https://user:s3cret@127.0.0.1:3000").unwrap_err();
        assert!(
            !userinfo_scheme_err.contains("s3cret"),
            "a scheme error must not echo credentials; got {userinfo_scheme_err}"
        );
        // A non-http URL reports a scheme error, not a bare-base one:
        // the bare-base check would otherwise see the `//` in `https://`.
        let scheme_err = validate_live_base("https://127.0.0.1:3000").unwrap_err();
        assert!(
            scheme_err.contains("must be an http:// URL"),
            "a non-http URL must report a scheme error; got {scheme_err}"
        );
        // A bracketed-host spoof is rejected by the loopback check.
        assert!(validate_live_base("http://[::1]evil.com").is_err());
    }

    #[test]
    fn live_obs_client_uses_the_instance_base() {
        // The client built for the live base must point its reads
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
    fn live_filing_path_is_wired_but_not_filed() {
        // The filing path is wired end to end (payload → SrFiling →
        // dry-run) without a live server: dry-run files nothing and
        // returns None. The live path against the instance is the
        // live tests above.
        let http = UreqClient::new(Timeout::default()).expect("default timeout is valid");
        let client = ObsClient::with_base(&http, "http://127.0.0.1:3000").unwrap();
        let dry = DryRun::default();
        let filing = SrFiling::new(&client, &dry);
        let payload = SrPayload::new(
            "home:geectl-live",
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

    #[test]
    fn unique_suffix_is_distinct_across_parallel_calls() {
        // The live tests run in parallel in one process, and two calls
        // in the same clock tick used to return the same suffix,
        // tripping the dedup guard (issue #39). Hammer the generator
        // from several threads and require every returned suffix
        // distinct.
        use std::collections::HashSet;
        let threads = 8;
        let per_thread = 1_000;
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                std::thread::spawn(move || {
                    (0..per_thread).map(|_| unique_suffix()).collect::<Vec<_>>()
                })
            })
            .collect();
        let mut seen = HashSet::new();
        for handle in handles {
            for suffix in handle.join().expect("generator thread panicked") {
                assert!(seen.insert(suffix), "duplicate suffix from parallel calls");
            }
        }
        assert_eq!(seen.len(), threads * per_thread);
    }
}

#[test]
fn live_branch_inherited_package_links_at_origin() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (factory, update, _devel, package) = setup_branch_projects(&client, &base);
    // Branch the package from the update project, where it is only
    // inherited through the project link: the branch must link at the
    // inherited source (the factory), not at the update project.
    let result = client
        .branch_package(
            &update,
            &package,
            &BranchOptions::default(),
            &DryRun::new(false),
        )
        .expect("branch an inherited package on the live instance")
        .expect("live branching returns a target");
    // The server reports the resolved origin as the source
    // project, not the project the branch was requested from.
    assert_eq!(result.source_project, factory);
    assert_eq!(result.source_package, package);
    let link = read_link(&base, &result.target_project, &result.target_package);
    assert_eq!(
        xml_attr_after(&link, "<link ", "project").as_deref(),
        Some(factory.as_str()),
        "the branch must link at the inherited source, not the update project; link: {link}"
    );
    // The server omits the package attribute when the link's package
    // is the target package itself (it is implied).
    assert!(
        xml_attr_after(&link, "<link ", "package")
            .as_deref()
            .is_none_or(|p| p == package.as_str()),
        "the branch must link at the package; link: {link}"
    );
}
#[test]
fn live_branch_no_devel_project_flag() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (factory, update, _devel, package) = setup_branch_projects(&client, &base);
    // The same inherited case with the no-devel-project flag, sent
    // for real: the branch still links at the inherited source.
    let options = BranchOptions {
        no_devel_project: true,
        ..BranchOptions::default()
    };
    let result = client
        .branch_package(&update, &package, &options, &DryRun::new(false))
        .expect("branch with no-devel-project on the live instance")
        .expect("live branching returns a target");
    let link = read_link(&base, &result.target_project, &result.target_package);
    assert_eq!(
        xml_attr_after(&link, "<link ", "project").as_deref(),
        Some(factory.as_str()),
        "the branch must link at the inherited source with the flag set; link: {link}"
    );
}
#[test]
fn live_branch_new_package_keeps_update_project() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (factory, update, _devel, package) = setup_branch_projects(&client, &base);
    // Branch the inherited package as a new package: the link keeps
    // the update project as its source instead of resolving to the
    // factory.
    let options = BranchOptions {
        new_package: true,
        ..BranchOptions::default()
    };
    let result = client
        .branch_package(&update, &package, &options, &DryRun::new(false))
        .expect("branch as a new package on the live instance")
        .expect("live branching returns a target");
    let link = read_link(&base, &result.target_project, &result.target_package);
    assert_eq!(
        xml_attr_after(&link, "<link ", "project").as_deref(),
        Some(update.as_str()),
        "a new-package branch must keep the update project as its source; link: {link}"
    );
    assert_ne!(
        update, factory,
        "the setup must distinguish the update project from the factory"
    );
}
#[test]
fn live_branch_new_package_copies_devel_sources() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (factory, _update, devel, package) = setup_branch_projects(&client, &base);
    // The devel package carries a file the source package does not
    // have; a new-package branch of a source with a devel project
    // copies the devel package's sources into the branch.
    client
        .upload_file(
            &devel,
            &package,
            "devel-marker.txt",
            "devel source\n",
            &DryRun::new(false),
        )
        .expect("upload a marker file to the devel package");
    client
        .set_devel_project(&factory, &package, Some(&devel), None, &DryRun::new(false))
        .expect("set the devel project on the live instance");
    let options = BranchOptions {
        new_package: true,
        ..BranchOptions::default()
    };
    let result = client
        .branch_package(&factory, &package, &options, &DryRun::new(false))
        .expect("branch as a new package on the live instance")
        .expect("live branching returns a target");
    let directory = read_directory(&base, &result.target_project, &result.target_package);
    assert!(
        directory.contains("name=\"devel-marker.txt\""),
        "a newinstance branch must copy the devel package's sources; directory: {directory}"
    );
}
#[test]
fn live_branch_with_devel_project_links_at_devel() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (factory, _update, devel, package) = setup_branch_projects(&client, &base);
    set_package_devel(&base, &factory, &package, &devel);
    // Branch the package with a devel project registered: the link
    // resolves through devel, not the factory branched from.
    let result = client
        .branch_package(
            &factory,
            &package,
            &BranchOptions::default(),
            &DryRun::new(false),
        )
        .expect("branch a package with a devel project on the live instance")
        .expect("live branching returns a target");
    let link = read_link(&base, &result.target_project, &result.target_package);
    assert_eq!(
        xml_attr_after(&link, "<link ", "project").as_deref(),
        Some(devel.as_str()),
        "the branch must link at the devel project; link: {link}"
    );
}
#[test]
fn live_branch_no_devel_project_overrides_devel() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (factory, _update, devel, package) = setup_branch_projects(&client, &base);
    set_package_devel(&base, &factory, &package, &devel);
    // The flag overrides the devel project: the branch links at the
    // factory it was branched from.
    let options = BranchOptions {
        no_devel_project: true,
        ..BranchOptions::default()
    };
    let result = client
        .branch_package(&factory, &package, &options, &DryRun::new(false))
        .expect("branch with no-devel-project on the live instance")
        .expect("live branching returns a target");
    let link = read_link(&base, &result.target_project, &result.target_package);
    assert_eq!(
        xml_attr_after(&link, "<link ", "project").as_deref(),
        Some(factory.as_str()),
        "the flag must override the devel project; link: {link}"
    );
}
#[test]
fn live_branch_inherited_with_devel_resolves_through_devel() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (factory, update, devel, package) = setup_branch_projects(&client, &base);
    set_package_devel(&base, &factory, &package, &devel);
    // An inherited package that HAS a devel project still resolves
    // through devel, even when branched from the update project.
    let result = client
        .branch_package(
            &update,
            &package,
            &BranchOptions::default(),
            &DryRun::new(false),
        )
        .expect("branch an inherited package with a devel project on the live instance")
        .expect("live branching returns a target");
    let link = read_link(&base, &result.target_project, &result.target_package);
    assert_eq!(
        xml_attr_after(&link, "<link ", "project").as_deref(),
        Some(devel.as_str()),
        "the branch must resolve through the devel project; link: {link}"
    );
}
#[test]
fn live_branch_inherited_no_devel_project_links_at_factory() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (factory, update, devel, package) = setup_branch_projects(&client, &base);
    set_package_devel(&base, &factory, &package, &devel);
    // The flag on the inherited package: the branch links at the
    // factory, not the devel project and not the update project.
    let options = BranchOptions {
        no_devel_project: true,
        ..BranchOptions::default()
    };
    let result = client
        .branch_package(&update, &package, &options, &DryRun::new(false))
        .expect("branch an inherited package with no-devel-project on the live instance")
        .expect("live branching returns a target");
    let link = read_link(&base, &result.target_project, &result.target_package);
    assert_eq!(
        xml_attr_after(&link, "<link ", "project").as_deref(),
        Some(factory.as_str()),
        "the branch must link at the factory with the flag set; link: {link}"
    );
}
#[test]
fn live_branch_new_package_nonexistent_succeeds() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (_factory, update, _devel, package) = setup_branch_projects(&client, &base);
    // Branch a package that does not exist anywhere as a new
    // package: it succeeds, and the link points at the project
    // branched from (the update project) for the not-yet-existing
    // package — the factory link in the borrowed case came from
    // branching from the factory itself.
    let missing = format!("{package}-missing");
    let options = BranchOptions {
        new_package: true,
        ..BranchOptions::default()
    };
    let result = client
        .branch_package(&update, &missing, &options, &DryRun::new(false))
        .expect("branch a not-yet-existing package on the live instance")
        .expect("live branching returns a target");
    let link = read_link(&base, &result.target_project, &result.target_package);
    assert_eq!(
        xml_attr_after(&link, "<link ", "project").as_deref(),
        Some(update.as_str()),
        "the branch must link at the update project for the missing package; link: {link}"
    );
    // The server omits the package attribute when the link's package
    // is the target package itself (it is implied).
    assert!(
        xml_attr_after(&link, "<link ", "package")
            .as_deref()
            .is_none_or(|p| p == missing.as_str()),
        "the branch must link at the missing package name; link: {link}"
    );
}

/// Create a uniquely-named project carrying `kind` in its `_meta`.
/// Returns the project name.
fn create_kind_project(client: &ObsClient<'_>, base: &str, kind: &str) -> String {
    let project = format!("geectl-tmp-{kind}-{}", unique_suffix());
    client
        .create_project(
            &project,
            "geectl live test",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create project on the live instance");
    set_project_kind(base, &project, kind);
    project
}

/// Create a project of `kind` whose name carries a colon, so the live
/// run exercises the `:` to `_` translation where the name feeds an
/// extended package name.
fn create_kind_project_colon(client: &ObsClient<'_>, base: &str, kind: &str) -> String {
    let project = format!("geectl-tmp-{kind}:{}", unique_suffix());
    client
        .create_project(
            &project,
            "geectl live test",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create project on the live instance");
    set_project_kind(base, &project, kind);
    project
}

/// A maintenance incident payload for the live tests: the source
/// branch to the maintenance project, releasing into the release
/// project.
fn incident_payload(
    source_project: &str,
    package: &str,
    target_project: &str,
    release_project: &str,
) -> MaintenanceIncidentPayload {
    MaintenanceIncidentPayload::new(
        source_project,
        &[package],
        target_project,
        &format!("Maintenance incident for {package}: live test"),
    )
    .expect("live maintenance incident payload")
    .with_release_project(release_project)
    .expect("live incident release project")
}

/// Accepting a change devel request re-points the target package's
/// devel at the source package.
#[test]
fn live_change_devel_accept_sets_devel_on_server() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let package = live_package();
    // The package whose devel changes, and the project that will be
    // its new devel. Both need to exist as packages.
    let target_project = create_source_branch(&client, &package);
    let devel_project = create_source_branch(&client, &package);
    let dry = DryRun::new(false);
    let filing = SrFiling::new(&client, &dry);
    let payload = ChangeDevelPayload::new(
        &devel_project,
        &package,
        &target_project,
        &package,
        &format!("Change devel of {package} to {devel_project}: live test"),
    )
    .expect("live change devel payload");
    let id = filing
        .file_change_devel(&payload)
        .expect("file a change devel request on the live instance")
        .expect("live filing returns a request id");
    let body = request_xml(&base, &id);
    assert_eq!(
        request_state(&body).as_deref(),
        Some("new"),
        "filed request {id} must be new; body: {body}"
    );
    assert_eq!(
        xml_attr_after(&body, "<action ", "type").as_deref(),
        Some("change_devel"),
        "filed request {id} must be a change devel; body: {body}"
    );
    assert_eq!(
        xml_attr_after(&body, "<source ", "project").as_deref(),
        Some(devel_project.as_str()),
        "filed request {id} must name the new devel project; body: {body}"
    );
    assert_eq!(
        xml_attr_after(&body, "<target ", "project").as_deref(),
        Some(target_project.as_str()),
        "filed request {id} must target the package's project; body: {body}"
    );
    client
        .accept_request(&id, "accepted by the geectl live test", false, &dry)
        .expect("accept the request on the live instance");
    let accepted = request_xml(&base, &id);
    assert_eq!(
        request_state(&accepted).as_deref(),
        Some("accepted"),
        "request {id} must be accepted; body: {accepted}"
    );
    // The target package's _meta now names the devel package.
    let meta = package_meta(&base, &target_project, &package);
    assert!(
        meta.contains(&format!(
            "<devel project=\"{devel_project}\" package=\"{package}\"/>"
        )),
        "the target package {target_project}/{package} must name its new devel; meta: {meta}"
    );
}

/// Give `project` a single repository `repo` / `arch` whose path
/// points at `path_project` / `path_repo` (see
/// [`project_meta_with_repo`]).
fn add_repository(
    base: &str,
    project: &str,
    repo: &str,
    arch: &str,
    path_project: &str,
    path_repo: &str,
) {
    set_project_meta(
        base,
        project,
        &project_meta_with_repo(project, repo, arch, path_project, path_repo),
    );
}

/// Create a uniquely-named empty project with a single `standard`
/// repository and no path, for use as a repository path target (see
/// [`project_meta_with_repo`]). The target's repository has no
/// packages, so anything building against it finds nothing to
/// resolve.
fn create_path_target(client: &ObsClient<'_>, base: &str) -> String {
    let target = format!("geectl-tmp-path-{}", unique_suffix());
    client
        .create_project(
            &target,
            "geectl live test path target",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create path target project on the live instance");
    set_project_meta(
        base,
        &target,
        &project_meta_with_bare_repo(&target, "standard", "x86_64"),
    );
    target
}

/// The binaries for a package that was never built: no files to
/// list, and neither a download nor a fileinfo exists.
#[test]
fn live_binaries_empty_for_a_never_built_package() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&client, &package);
    let path_target = create_path_target(&client, &base);
    add_repository(
        &base,
        &source_project,
        "standard",
        "x86_64",
        &path_target,
        "standard",
    );
    assert!(
        client
            .list_binaries(&source_project, Some(&package), "standard", "x86_64")
            .expect("binary list on the live instance")
            .is_empty(),
        "a package that was never built has no binaries"
    );
    let err = client
        .download_binary(
            &source_project,
            &package,
            "standard",
            "x86_64",
            "nosuch-1.0-1.x86_64.rpm",
        )
        .unwrap_err();
    assert_eq!(
        err.kind(),
        ErrorKind::NotFound,
        "downloading a binary that was never built must 404"
    );
    let err = client
        .fileinfo(
            &source_project,
            &package,
            "standard",
            "x86_64",
            "nosuch-1.0-1.x86_64.rpm",
            false,
        )
        .unwrap_err();
    assert_eq!(
        err.kind(),
        ErrorKind::NotFound,
        "fileinfo for a binary that was never built must 404"
    );
}

/// Accepting a delete request deletes the target package.
#[test]
fn live_delete_accept_deletes_package_on_server() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let package = live_package();
    let project = create_source_branch(&client, &package);
    let dry = DryRun::new(false);
    let filing = SrFiling::new(&client, &dry);
    let payload = DeletePayload::new(
        &project,
        DeleteTarget::Package(package.clone()),
        &format!("Delete {package}: live test"),
    )
    .expect("live delete payload");
    let id = filing
        .file_delete_request(&payload)
        .expect("file a delete request on the live instance")
        .expect("live filing returns a request id");
    let body = request_xml(&base, &id);
    assert_eq!(
        request_state(&body).as_deref(),
        Some("new"),
        "filed request {id} must be new; body: {body}"
    );
    assert_eq!(
        xml_attr_after(&body, "<action ", "type").as_deref(),
        Some("delete"),
        "filed request {id} must be a delete; body: {body}"
    );
    assert_eq!(
        xml_attr_after(&body, "<target ", "project").as_deref(),
        Some(project.as_str()),
        "filed request {id} must target the package's project; body: {body}"
    );
    assert_eq!(
        xml_attr_after(&body, "<target ", "package").as_deref(),
        Some(package.as_str()),
        "filed request {id} must target the package; body: {body}"
    );
    client
        .accept_request(&id, "accepted by the geectl live test", false, &dry)
        .expect("accept the request on the live instance");
    let accepted = request_xml(&base, &id);
    assert_eq!(
        request_state(&accepted).as_deref(),
        Some("accepted"),
        "request {id} must be accepted; body: {accepted}"
    );
    wait_until_gone(&base, &format!("{project}/{package}"));
}

/// Listing the maintained instances of a package and branching it:
/// the dry-run listing reports where the branch would land, and the
/// live branch lands there.
#[test]
fn live_maintenance_branch_lists_and_branches_on_server() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let package = live_package();
    let maintained_project = create_source_branch(&client, &package);
    upload_spec(&client, &maintained_project, &package);
    set_project_attribute(&base, &maintained_project, "OBS", "Maintained");
    let options = MaintenanceBranchOptions::new();
    let instances = client
        .maintained_instances(&package, &options)
        .expect("list maintained instances on the live instance");
    assert_eq!(
        instances.len(),
        1,
        "the maintained project must yield exactly one instance; got {instances:?}"
    );
    let instance = &instances[0];
    assert_eq!(instance.source_project, maintained_project);
    assert_eq!(instance.source_package, package);
    assert_eq!(instance.devel_project, None);
    assert_eq!(instance.devel_package, None);
    let branch_project = format!("home:{}:branches:OBS_Maintained:{package}", live_username());
    assert_eq!(instance.target_project, branch_project);
    assert_eq!(
        instance.target_package,
        format!("{package}.{maintained_project}")
    );
    let dry = DryRun::new(false);
    let result = client
        .maintenance_branch(&package, &options, &dry)
        .expect("branch the maintained package on the live instance")
        .expect("live branch returns a result");
    assert_eq!(result.target_project, branch_project);
    assert_eq!(
        result.target_package.as_deref(),
        Some(format!("{package}.{maintained_project}").as_str())
    );
    assert_eq!(
        result.source_project.as_deref(),
        Some(maintained_project.as_str())
    );
    assert_eq!(result.source_package.as_deref(), Some(package.as_str()));
    wait_until_present(
        &base,
        &format!("{branch_project}/{package}.{maintained_project}"),
    );
}

/// Accepting a maintenance incident creates the incident project
/// under the maintenance project and merges the package into it.
#[test]
fn live_maintenance_incident_accept_creates_incident_on_server() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, target_project, release_project, package) = setup_incident(&client, &base);
    let dry = DryRun::new(false);
    let filing = SrFiling::new(&client, &dry);
    let id = filing
        .file_maintenance_incident(&incident_payload(
            &source_project,
            &package,
            &target_project,
            &release_project,
        ))
        .expect("file a maintenance incident on the live instance")
        .expect("live filing returns a request id");
    client
        .accept_request(&id, "accepted by the geectl live test", false, &dry)
        .expect("accept the request on the live instance");
    let accepted = request_xml(&base, &id);
    assert_eq!(
        request_state(&accepted).as_deref(),
        Some("accepted"),
        "incident {id} must be accepted; body: {accepted}"
    );
    // The accept moves the target to the incident project it created
    // under the maintenance project.
    let incident_project = xml_attr_after(&accepted, "<target ", "project")
        .expect("the accepted incident must name its incident project");
    assert!(
        incident_project.starts_with(&format!("{target_project}:")),
        "the incident project {incident_project} must sit under the maintenance project {target_project}; body: {accepted}"
    );
    // The accept merges the package under the extended name
    // `<package>.<release-project>` — the plain source name never
    // lands. The accepted request's own target names it.
    let incident_package = xml_attr_after(&accepted, "<target ", "package")
        .expect("the accepted incident must name its incident package");
    assert_eq!(
        incident_package,
        format!("{package}.{release_project}").replace(':', "_"),
        "the incident package must be the source extended with the release project; body: {accepted}"
    );
    wait_until_present(&base, &incident_project);
    wait_until_present(&base, &format!("{incident_project}/{incident_package}"));
}

/// Filing a real maintenance incident request and reading it back
/// from the server, then filing the same incident again: the dedup
/// refuses the duplicate and names the open request.
#[test]
fn live_maintenance_incident_filed_and_dedup_on_server() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_project, target_project, release_project, package) = setup_incident(&client, &base);
    let dry = DryRun::new(false);
    let filing = SrFiling::new(&client, &dry);
    let payload = incident_payload(&source_project, &package, &target_project, &release_project);
    let id = filing
        .file_maintenance_incident(&payload)
        .expect("file a maintenance incident on the live instance")
        .expect("live filing returns a request id");
    let body = request_xml(&base, &id);
    // The filed incident is open, from the branch, to the maintenance
    // project, releasing into the release project — and the filing
    // pinned source_rev, so the source carries a rev.
    assert_eq!(
        request_state(&body).as_deref(),
        Some("new"),
        "filed incident {id} must be new; body: {body}"
    );
    assert_eq!(
        xml_attr_after(&body, "<action ", "type").as_deref(),
        Some("maintenance_incident"),
        "filed request {id} must be a maintenance incident; body: {body}"
    );
    assert_eq!(
        xml_attr_after(&body, "<source ", "project").as_deref(),
        Some(source_project.as_str()),
        "filed incident {id} must come from the branch; body: {body}"
    );
    assert_eq!(
        xml_attr_after(&body, "<source ", "package").as_deref(),
        Some(package.as_str()),
        "filed incident {id} must carry the branch package; body: {body}"
    );
    assert!(
        xml_attr_after(&body, "<source ", "rev").is_some(),
        "filed incident {id} must pin source_rev (addrevision=1); body: {body}"
    );
    assert_eq!(
        xml_attr_after(&body, "<target ", "project").as_deref(),
        Some(target_project.as_str()),
        "filed incident {id} must target the maintenance project; body: {body}"
    );
    assert_eq!(
        xml_attr_after(&body, "<target ", "releaseproject").as_deref(),
        Some(release_project.as_str()),
        "filed incident {id} must release into the release project; body: {body}"
    );
    // Filing the same incident again is refused by the dedup, and
    // the refusal names the open request.
    let err = filing
        .file_maintenance_incident(&incident_payload(
            &source_project,
            &package,
            &target_project,
            &release_project,
        ))
        .expect_err("filing a duplicate maintenance incident must be refused");
    assert!(
        format!("{err}").contains(&id),
        "the refusal must name the open incident {id}; got {err}"
    );
}

/// The maintenance incident refile lifecycle against the server:
/// file, then refile from a second branch and supersede.
#[test]
fn live_maintenance_incident_refile_supersedes_on_server() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let (source_a, target_project, release_project, package) = setup_incident(&client, &base);
    let dry = DryRun::new(false);
    let filing = SrFiling::new(&client, &dry);
    let id1 = filing
        .file_maintenance_incident(&incident_payload(
            &source_a,
            &package,
            &target_project,
            &release_project,
        ))
        .expect("file the first incident on the live instance")
        .expect("live filing returns a request id");
    // The refile comes from a second branch, same target.
    let source_b = create_source_branch(&client, &package);
    upload_spec(&client, &source_b, &package);
    let id2 = filing
        .refile_maintenance_incident(
            &incident_payload(&source_b, &package, &target_project, &release_project),
            &id1,
        )
        .expect("refile the incident on the live instance")
        .expect("live refile returns a request id");
    assert_ne!(id1, id2, "refile must return a new request id");
    let old = client
        .show_request(&id1)
        .expect("read the old incident back from the server");
    assert_eq!(
        old.state.name,
        RequestState::Superseded,
        "the old incident {id1} must be superseded"
    );
    assert_eq!(
        old.state.superseded_by,
        Some(id2.parse().expect("the new request id is a number")),
        "the old incident {id1} must point at the new one"
    );
    let new = client
        .show_request(&id2)
        .expect("read the new incident back from the server");
    assert_eq!(
        new.state.name,
        RequestState::New,
        "the new incident {id2} must be new"
    );
}

/// Filing and accepting a maintenance incident without a release
/// project: the release target resolves at filing time, through the
/// source package's link target — the server raises
/// `NoMaintenanceReleaseTarget` when the source is unlinked and no
/// release project is given, so the source here links at the release
/// project. The source also carries a bumped spec alongside the link:
/// a pure link diffs empty against the release package, so the server
/// would drop the action (`MissingAction`) at filing. The accept
/// merges the package under the extended name, like the
/// release-project case.
#[test]
fn live_maintenance_incident_without_release_project_resolves_via_link_on_server() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let package = live_package();
    // The release project holds the package; the source branch links
    // at it, so the filing resolves the release target through the link.
    let release_project = create_kind_project(&client, &base, "maintenance_release");
    client
        .create_package(
            &release_project,
            &package,
            "geectl live test package",
            "Created by the geectl live live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create package in the release project on the live instance");
    upload_spec(&client, &release_project, &package);
    let source_project = create_source_branch(&client, &package);
    client
        .upload_file(
            &source_project,
            &package,
            "_link",
            &format!("<link project=\"{release_project}\" package=\"{package}\"/>"),
            &DryRun::new(false),
        )
        .expect("upload a _link file to the live instance");
    // A pure link diffs empty against the release package, so the
    // server would drop the action at filing: the source also carries
    // a bumped spec, which overrides the linked one in expansion.
    client
        .upload_file(
            &source_project,
            &package,
            "geectl-live-pkg.spec",
            "Name: geectl-live-pkg\nVersion: 2.0\n",
            &DryRun::new(false),
        )
        .expect("upload a bumped spec file to the live instance");
    let target_project = create_kind_project(&client, &base, "maintenance");
    // The live instance is a fresh OBS with no project flagged as the
    // central maintenance project, and the accept path needs one (see
    // setup_incident): flag this test's maintenance project so the
    // instance models a production one.
    set_project_attribute(&base, &target_project, "OBS", "MaintenanceProject");
    let dry = DryRun::new(false);
    let filing = SrFiling::new(&client, &dry);
    let payload = MaintenanceIncidentPayload::new(
        &source_project,
        &[&package],
        &target_project,
        &format!("Maintenance incident for {package} without release project: live test"),
    )
    .expect("live maintenance incident payload");
    let id = filing
        .file_maintenance_incident(&payload)
        .expect("file a maintenance incident without a release project on the live instance")
        .expect("live filing returns a request id");
    let body = request_xml(&base, &id);
    assert_eq!(
        request_state(&body).as_deref(),
        Some("new"),
        "filed incident {id} must be new; body: {body}"
    );
    assert_eq!(
        xml_attr_after(&body, "<action ", "type").as_deref(),
        Some("maintenance_incident"),
        "filed request {id} must be a maintenance incident; body: {body}"
    );
    // Filing-time expansion stores the resolved release project on
    // the action, even though the filed XML named none.
    assert_eq!(
        xml_attr_after(&body, "<target ", "releaseproject").as_deref(),
        Some(release_project.as_str()),
        "filed incident {id} must resolve the release project through the link; body: {body}"
    );
    client
        .accept_request(&id, "accepted by the geectl live test", false, &dry)
        .expect("accept the request on the live instance");
    let accepted = request_xml(&base, &id);
    assert_eq!(
        request_state(&accepted).as_deref(),
        Some("accepted"),
        "incident {id} must be accepted; body: {accepted}"
    );
    let incident_project = xml_attr_after(&accepted, "<target ", "project")
        .expect("the accepted incident must name its incident project");
    assert!(
        incident_project.starts_with(&format!("{target_project}:")),
        "the incident project {incident_project} must sit under the maintenance project {target_project}; body: {accepted}"
    );
    // The accept merges the linked package under the extended name.
    let incident_package = xml_attr_after(&accepted, "<target ", "package")
        .expect("the accepted incident must name its incident package");
    assert_eq!(
        incident_package,
        format!("{package}.{release_project}").replace(':', "_"),
        "the incident package must be the source extended with the release project; body: {accepted}"
    );
    wait_until_present(&base, &incident_project);
    wait_until_present(&base, &format!("{incident_project}/{incident_package}"));
}

/// The login of the live credentials (the bootstrap admin by
/// default). The maintenance branch project is derived from it
/// (`home:<user>:branches:...`), so the branch test builds the
/// expected name from the same source as the credentials.
fn live_username() -> String {
    match live_credentials() {
        Credentials::Basic { username, .. } => username,
        _ => unreachable!("live credentials are Basic"),
    }
}

/// The `_meta` document of a package, read back from the server. The
/// library has no read-a-package-meta operation, so the harness
/// reads it through geectl's own HTTP client with the admin
/// credentials.
fn package_meta(base: &str, project: &str, package: &str) -> String {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    http.get_with(
        &format!("{base}/source/{project}/{package}/_meta"),
        &live_credentials(),
    )
    .expect("GET package _meta from the live instance")
}

/// Set attribute `namespace:name` on a project: POST the
/// `<attributes>` document to the project's `_attribute`.
fn set_project_attribute(base: &str, project: &str, namespace: &str, name: &str) {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    let xml =
        format!("<attributes><attribute namespace=\"{namespace}\" name=\"{name}\"/></attributes>");
    let body = http
        .post(
            &format!("{base}/source/{project}/_attribute"),
            &xml,
            &live_credentials(),
        )
        .expect("POST project attribute on the live instance");
    assert!(
        response_is_status_ok(&body),
        "setting attribute {namespace}:{name} on {project} did not return <status code=\"ok\"/>; body: {body}"
    );
}

/// Rewrite a project's `_meta` with the given kind (`maintenance`,
/// `maintenance_release`, ...). The library's `create_project`
/// writes no kind; the kind lives in the `_meta` document, so the
/// harness writes it directly, the same way the library writes
/// `_meta` for `create_project`.
fn set_project_kind(base: &str, project: &str, kind: &str) {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    let meta = format!(
        "<project name=\"{project}\" kind=\"{kind}\"><title>geectl live test</title><description>Created by the geectl live live test; the instance is destroyed after the run.</description></project>"
    );
    let body = http
        .put(
            &format!("{base}/source/{project}/_meta"),
            &meta,
            &live_credentials(),
        )
        .expect("PUT project _meta with kind on the live instance");
    assert!(
        response_is_status_ok(&body),
        "setting kind {kind} on {project} did not return <status code=\"ok\"/>; body: {body}"
    );
}

/// A maintenance incident setup: a source branch holding the package
/// (with a spec file), a maintenance target project, and a
/// maintenance_release project. Returns `(source_project,
/// target_project, release_project, package)`.
fn setup_incident(client: &ObsClient<'_>, base: &str) -> (String, String, String, String) {
    let package = live_package();
    let source_project = create_source_branch(client, &package);
    upload_spec(client, &source_project, &package);
    let target_project = create_kind_project(client, base, "maintenance");
    // The live instance is a fresh OBS with no project flagged as the
    // central maintenance project, and the accept path needs one: the
    // branch into the incident runs `add_channels`, which looks the
    // maintained projects up through `Project.get_maintenance_project!`
    // and 404s with `unknown_project` when none is flagged. Production
    // OBS instances have one; flag this test's maintenance project so
    // the instance models one.
    set_project_attribute(base, &target_project, "OBS", "MaintenanceProject");
    // The release project name carries a colon, so the accept test
    // exercises the `:` to `_` translation in the extended incident
    // package name live.
    let release_project = create_kind_project_colon(client, base, "maintenance_release");
    (source_project, target_project, release_project, package)
}

/// Upload a spec file into a package so the source has content: the
/// incident filing pins `source_rev`, which needs an expanded
/// source, and the branch copies the package's files.
fn upload_spec(client: &ObsClient<'_>, project: &str, package: &str) {
    client
        .upload_file(
            project,
            package,
            "geectl-live-pkg.spec",
            "Name: geectl-live-pkg\nVersion: 1.0\n",
            &DryRun::new(false),
        )
        .expect("upload a spec file to the live instance");
}

/// The build and job histories for a package that was never built:
/// both empty.
#[test]
fn live_build_history_empty_for_a_never_built_package() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&client, &package);
    let path_target = create_path_target(&client, &base);
    add_repository(
        &base,
        &source_project,
        "standard",
        "x86_64",
        &path_target,
        "standard",
    );
    assert!(
        client
            .build_history(&source_project, &package, "standard", "x86_64", None)
            .expect("build history on the live instance")
            .is_empty(),
        "a package that was never built has no build history"
    );
    assert!(
        client
            .job_history(
                &source_project,
                "standard",
                "x86_64",
                Some(&package),
                None,
                None
            )
            .expect("job history on the live instance")
            .is_empty(),
        "a repository that never built has no job history"
    );
}

/// The log, the status, and the job status for a package that was
/// never built: no log, no status, and no running job.
#[test]
fn live_build_log_and_status_none_for_a_never_built_package() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&client, &package);
    let path_target = create_path_target(&client, &base);
    add_repository(
        &base,
        &source_project,
        "standard",
        "x86_64",
        &path_target,
        "standard",
    );
    assert_eq!(
        client
            .build_log(
                &source_project,
                &package,
                "standard",
                "x86_64",
                &BuildLogQuery::default()
            )
            .expect("build log on the live instance"),
        None,
        "a package that was never built has no log"
    );
    assert_eq!(
        client
            .build_status(&source_project, &package, "standard", "x86_64")
            .expect("build status on the live instance"),
        None,
        "a package that was never built has no status"
    );
    assert_eq!(
        client
            .job_status(&source_project, &package, "standard", "x86_64")
            .expect("job status on the live instance"),
        None,
        "no job runs for a package that was never built"
    );
}

/// Build results on the server: a fresh project has no repositories,
/// so the matrix is empty and the wait settles at once; once a
/// repository exists, the scheduler reports it.
#[test]
fn live_build_results_empty_then_scheduler_reports_the_repo() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&client, &package);
    // A fresh project has no repositories: the matrix is empty.
    let results = client
        .results(&source_project, &ResultsQuery::default())
        .expect("results for a fresh project");
    assert!(
        results.results.is_empty(),
        "a project with no repositories must have an empty results matrix"
    );
    // The wait settles at once on an empty matrix: nothing is
    // building, nothing is dirty.
    let waited = client
        .wait_for_results(
            &source_project,
            &ResultsQuery::default(),
            std::time::Duration::from_secs(30),
        )
        .expect("wait on a fresh project");
    assert!(waited.results.is_empty());
    // Give the project a repository whose path points at an
    // empty repository: the scheduler reports the cell, but the
    // report arrives through the backend, so poll for it.
    let path_target = create_path_target(&client, &base);
    add_repository(
        &base,
        &source_project,
        "standard",
        "x86_64",
        &path_target,
        "standard",
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let results = loop {
        let results = client
            .results(&source_project, &ResultsQuery::default())
            .expect("results while polling for the repository");
        if !results.results.is_empty() {
            break results;
        }
        if std::time::Instant::now() > deadline {
            panic!("no build result for {source_project}/standard/x86_64 after 60s");
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    };
    assert_eq!(results.results.len(), 1);
    let cell = &results.results[0];
    assert_eq!(cell.project, source_project);
    assert_eq!(cell.repository, "standard");
    assert_eq!(cell.arch, "x86_64");
    assert!(
        !cell.code.is_empty(),
        "the scheduler must report a state for the repository"
    );
}

/// Buildinfo on the server: a spec whose BuildRequires names a
/// package that does not exist surfaces as an unresolvable error.
/// The buildinfo is computed on demand, so poll until the error
/// appears (the backend reads the fresh meta and spec directly).
/// The project config must define the build type (`Type: spec`):
/// without it the backend errors before dependency expansion.
#[test]
fn live_buildinfo_reports_the_unresolvable_dependency() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&client, &package);
    let spec = format!(
        "Name:           {package}\nVersion:        1.0\nRelease:        0\nSummary:        geectl live test package\nLicense:        MIT\nBuildRequires:  DOES-NOT-EXIST\n\n%description\ngeectl live test package.\n\n%prep\n%build\n%install\n"
    );
    client
        .upload_file(
            &source_project,
            &package,
            &format!("{package}.spec"),
            &spec,
            &DryRun::new(false),
        )
        .expect("upload a spec file to the live instance");
    // The repository builds against an empty repository: its
    // published state is empty, so nothing the spec requires can
    // resolve.
    let path_target = create_path_target(&client, &base);
    add_repository(
        &base,
        &source_project,
        "standard",
        "x86_64",
        &path_target,
        "standard",
    );
    // The build type comes from the project config, not the spec:
    // without it the backend reports "no build type defined or
    // detected" instead of expanding the dependencies.
    set_project_config(&base, &source_project, "Type: spec\n");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let info = loop {
        match client.buildinfo(&source_project, &package, "standard", "x86_64") {
            Ok(info) if info.error.is_some() => break info,
            Ok(info) => {
                if std::time::Instant::now() > deadline {
                    panic!(
                        "buildinfo for {package} never reported an error; last buildinfo: {info:?}"
                    );
                }
            }
            Err(err) => {
                if std::time::Instant::now() > deadline {
                    panic!("buildinfo for {package} kept failing: {err}");
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    };
    let error = info.error.expect("the polled buildinfo carries an error");
    assert!(
        error.contains("unresolvable"),
        "buildinfo must name the unresolvable dependency; got: {error}"
    );
}

/// The constraint check on the server: the live instance
/// ships no build worker by design (see
/// tests/live/docker-compose.yml), so no worker satisfies the
/// constraints. The empty result is an environment property, not a
/// property of the check: on an instance with a worker this test
/// would fail.
#[test]
fn live_check_constraints_reports_no_workers() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&client, &package);
    let path_target = create_path_target(&client, &base);
    add_repository(
        &base,
        &source_project,
        "standard",
        "x86_64",
        &path_target,
        "standard",
    );
    let workers = client
        .check_constraints(&source_project, &package, "standard", "x86_64", None)
        .expect("constraint check on the live instance");
    assert!(
        workers.is_empty(),
        "an instance with no workers must report no matching workers"
    );
}

/// The dependency info for a fresh repository: no packages have
/// recorded dependencies, in either direction.
#[test]
fn live_dependson_empty_for_a_fresh_repository() {
    let Some(base) = live_url() else {
        return;
    };
    let client = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&client, &package);
    let path_target = create_path_target(&client, &base);
    add_repository(
        &base,
        &source_project,
        "standard",
        "x86_64",
        &path_target,
        "standard",
    );
    assert!(
        client
            .dependson(&source_project, "standard", "x86_64", None)
            .expect("dependson on the live instance")
            .is_empty(),
        "a fresh repository has no recorded build dependencies"
    );
    assert!(
        client
            .whatdependson(&source_project, "standard", "x86_64", &package)
            .expect("whatdependson on the live instance")
            .is_empty(),
        "nothing in a fresh repository depends on the package"
    );
}

/// The repository list on the server, read as a public read: a fresh
/// project has none, and the added repository reads back with its
/// name, architectures, and path.
#[test]
fn live_list_repositories_reads_back_the_added_repo() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&authed, &package);
    // The repository list is a public read: no credentials.
    let client = live_client(&base);
    assert!(
        client
            .list_repositories(&source_project)
            .expect("repository list on the live instance")
            .is_empty(),
        "a fresh project has no repositories"
    );
    let path_target = create_path_target(&authed, &base);
    add_repository(
        &base,
        &source_project,
        "standard",
        "x86_64",
        &path_target,
        "standard",
    );
    let repos = client
        .list_repositories(&source_project)
        .expect("repository list after adding a repository");
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].name, "standard");
    assert_eq!(repos[0].arch, vec!["x86_64".to_string()]);
    assert_eq!(repos[0].paths.len(), 1);
    assert_eq!(repos[0].paths[0].0, path_target);
    assert_eq!(repos[0].paths[0].1, "standard");
}

/// A project meta document for `project` with a single repository
/// `repo` / `arch` and no path: it builds against nothing.
fn project_meta_with_bare_repo(project: &str, repo: &str, arch: &str) -> String {
    format!(
        "<project name=\"{project}\"><title>geectl live test project</title><description>Created by the geectl live live test; the instance is destroyed after the run.</description><repository name=\"{repo}\"><arch>{arch}</arch></repository></project>"
    )
}

/// A project meta document for `project` with a single repository
/// `repo` / `arch` whose path points at `path_project` /
/// `path_repo`. The path's project and repository must exist on the
/// instance — the API rejects an unknown path project with 404, and
/// an unknown path repository (or a path to the repository itself)
/// with 400 — so the target is a separate project created for the
/// test (see [`create_path_target`]).
fn project_meta_with_repo(
    project: &str,
    repo: &str,
    arch: &str,
    path_project: &str,
    path_repo: &str,
) -> String {
    format!(
        "<project name=\"{project}\"><title>geectl live test project</title><description>Created by the geectl live live test; the instance is destroyed after the run.</description><repository name=\"{repo}\"><arch>{arch}</arch><path project=\"{path_project}\" repository=\"{path_repo}\"/></repository></project>"
    )
}

/// PUT a project config (`_config`, the prjconf) for `project` on
/// the live instance.
fn set_project_config(base: &str, project: &str, config: &str) {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    let body = http
        .put(
            &format!("{base}/source/{project}/_config"),
            config,
            &live_credentials(),
        )
        .expect("PUT the project config on the live instance");
    assert!(
        response_is_status_ok(&body),
        "PUT project config for {project} did not return <status code=\"ok\"/>; body: {body}"
    );
}

/// Replace the project meta of `project` on the live instance
/// (PUT /source/<project>/_meta with the admin credentials). The
/// library has no repository-add operation (a mutation, outside this
/// batch's reads-only scope), so the harness writes the meta
/// directly, the same way it POSTs the accept `changestate`.
fn set_project_meta(base: &str, project: &str, meta: &str) {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    let body = http
        .put(
            &format!("{base}/source/{project}/_meta"),
            meta,
            &live_credentials(),
        )
        .expect("PUT the project meta on the live instance");
    assert!(
        response_is_status_ok(&body),
        "PUT project meta for {project} did not return <status code=\"ok\"/>; body: {body}"
    );
}

/// The services batch live tests: the two read-only operations
/// against the live instance. The instance's backend image may not
/// ship the service binaries, so no test asserts a live service RUN:
/// the positive live cases are round-trips of the stored `_service`
/// text, and the state cases read whatever the server recorded
/// (none on a fresh package).
const LIVE_SERVICE_MIXED: &str = r#"<services>
  <service name="download_url" mode="manual">
    <param name="host">example.com</param>
    <param name="protocol">https</param>
    <param name="query">a&amp;b</param>
  </service>
  <service name="set_version" mode="trylocal">
    <param name="file">version.txt</param>
    <param name="empty"></param>
    <param name="blank"/>
  </service>
  <service name="tar_scm" mode="disabled"/>
  <service name="obs_scm" mode="buildtime">
    <param name="url">https://example.com/repo.git</param>
  </service>
  <service name="format_spec_file">
    <param name="files">*.spec</param>
  </service>
  <service name="run_source_validator" mode="serveronly"/>
</services>
"#;

/// The `osc` invalid-`_service` fixture, exact bytes: well-formed
/// XML whose "invalid" is execution-time, so the typed view returns
/// the definition.
const LIVE_SERVICE_INVALID_OSC: &str = "<services>\n  <service name=\"set_version\" mode=\"manual\">\n    <param name=\"fromfile\">invalid</param>\n  </service>\n</services>\n";

/// The stored `_service` text round-trips as typed definitions:
/// all six modes, parameters verbatim including empty values and an
/// entity.
#[test]
fn live_list_services_round_trips_stored_file() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&authed, &package);
    authed
        .upload_file(
            &source_project,
            &package,
            "_service",
            LIVE_SERVICE_MIXED,
            &DryRun::new(false),
        )
        .expect("upload _service on the live instance");
    let public = live_client(&base);
    let services = public
        .list_services(&source_project, &package)
        .expect("list services on the live instance")
        .expect("the uploaded _service must be present");
    assert_eq!(services.len(), 6);
    assert_eq!(services[0].name, "download_url");
    assert_eq!(services[0].mode, "manual");
    assert_eq!(services[0].params[2].value, "a&b");
    assert_eq!(services[1].name, "set_version");
    assert_eq!(services[1].mode, "trylocal");
    assert_eq!(services[1].params[1].value, "");
    assert_eq!(services[1].params[2].value, "");
    assert_eq!(services[2].mode, "disabled");
    assert_eq!(services[3].mode, "buildtime");
    assert_eq!(services[4].name, "format_spec_file");
    assert_eq!(services[4].mode, "");
    assert_eq!(services[5].mode, "serveronly");
}

// No live round-trip for an unknown mode: the server rejects the
// upload with 400, so no such file can be stored via upload;
// the parse is covered by `unknown_mode_is_stored_verbatim`.

/// The `osc` invalid fixture parses on the live instance too: its
/// failure is a service run failing, not a parse failure.
#[test]
fn live_list_services_osc_fixture_parses() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&authed, &package);
    authed
        .upload_file(
            &source_project,
            &package,
            "_service",
            LIVE_SERVICE_INVALID_OSC,
            &DryRun::new(false),
        )
        .expect("upload the osc fixture _service on the live instance");
    let public = live_client(&base);
    let services = public
        .list_services(&source_project, &package)
        .expect("list services on the live instance")
        .expect("the uploaded _service must be present");
    assert_eq!(services.len(), 1);
    assert_eq!(services[0].name, "set_version");
    assert_eq!(services[0].mode, "manual");
    assert_eq!(services[0].params[0].value, "invalid");
}

/// A package with no `_service` file reads back as none.
#[test]
fn live_list_services_none_without_service_file() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&authed, &package);
    let public = live_client(&base);
    assert_eq!(
        public
            .list_services(&source_project, &package)
            .expect("list services on the live instance"),
        None
    );
}

/// A missing package is NotFound, not "no _service file".
#[test]
fn live_list_services_missing_package_is_not_found() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&authed, &package);
    let public = live_client(&base);
    assert_eq!(
        public
            .list_services(&source_project, "no-such-package")
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
}

/// The project's own `_service` file reads back the same way.
/// No `<link>` element here: the server rejects a link-bearing
/// `_service` upload with 400, so the link skip is covered by the
/// `project_link_is_skipped_not_a_service` unit test. The read uses
/// the authed client: the project `_service` has no public route.
#[test]
fn live_list_services_on_project_reads_project_file() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&authed, &package);
    let project_service =
        "<services>\n  <service name=\"download_url\" mode=\"manual\">\n    <param name=\"host\">example.com</param>\n  </service>\n</services>\n";
    authed
        .upload_file(
            &source_project,
            "_project",
            "_service",
            project_service,
            &DryRun::new(false),
        )
        .expect("upload the project _service on the live instance");
    let services = authed
        .list_services(&source_project, "_project")
        .expect("list project services on the live instance")
        .expect("the uploaded project _service must be present");
    assert_eq!(services.len(), 1);
    assert_eq!(services[0].name, "download_url");
}

/// A fresh package carries no service run: the state is none.
#[test]
fn live_service_state_none_on_fresh_package() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&authed, &package);
    let public = live_client(&base);
    assert_eq!(
        public
            .service_state(&source_project, &package)
            .expect("read service state on the live instance"),
        None
    );
}

/// A missing package is NotFound, not "no service run".
#[test]
fn live_service_state_missing_package_is_not_found() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&authed, &package);
    let public = live_client(&base);
    assert_eq!(
        public
            .service_state(&source_project, "no-such-package")
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
}

/// Probe (P4): the project's own service state has no asserted
/// expectation until observed — the outcome is printed for the run
/// log, whatever it is. The probe uses the authed client: the
/// project listing has no public route.
#[test]
fn live_service_state_on_project_probe() {
    let Some(base) = live_url() else {
        return;
    };
    let authed = live_client_authed(&base);
    let package = live_package();
    let source_project = create_source_branch(&authed, &package);
    let outcome = authed.service_state(&source_project, "_project");
    eprintln!("P4 probe: service_state on _project returned {outcome:?}");
}
