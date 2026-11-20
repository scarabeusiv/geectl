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
//! The SR tests (`temporal_filed_request_shows_up_on_server`,
//! `temporal_refile_supersedes_and_deletes_on_server`, and
//! `temporal_accept_deletes_source_branch_on_server`) file real
//! submit requests against the instance: they create a source branch
//! and a target project first, through geectl's own
//! `ObsClient::create_project` / `create_package` with the bootstrap
//! admin credentials, then assert on the server state the filing
//! leaves behind. The request XML is read back through geectl's own
//! HTTP client (there is no library read-a-request operation), and
//! the accept is a maintainer-side `changestate` POST, likewise
//! through the harness — an agent files requests; accepting one is a
//! maintainer action, so neither belongs in the library.

use geectl::auth::Credentials;
use geectl::error::ErrorKind;
use geectl::http::{HttpClient, Timeout, UreqClient};
use geectl::obs::{parse_status_ok, DryRun, ObsClient, SrFiling, SrPayload};

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
/// occurrence). Used for `superseded_by`, which is not tied to one
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

/// The request id this request was superseded by, if any: the
/// `superseded_by` attribute when the server renders it on the
/// request element, or the `<superseded_by>` element otherwise.
fn superseded_by(body: &str) -> Option<String> {
    if let Some(id) = xml_attr(body, "superseded_by") {
        return Some(id);
    }
    let open = "<superseded_by>";
    let close = "</superseded_by>";
    let start = body.find(open)? + open.len();
    let end = body[start..].find(close)? + start;
    Some(body[start..end].trim().to_string())
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

/// Accept submit request `id` as the admin: the maintainer-side
/// state change the accept-cleanup test exercises. There is no
/// library accept operation (an agent files requests; accepting one
/// is a maintainer action), so the harness POSTs the changestate
/// directly, the same `cmd=changestate` form `supersede_request`
/// uses.
fn accept_request(base: &str, id: &str) {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    let body = http
        .post_text(
            &format!("{base}/request/{id}?cmd=changestate&newstate=accepted"),
            "accepted by the geectl temporal test",
            &temporal_credentials(),
        )
        .expect("accept the request on the temporal instance");
    assert!(
        parse_status_ok(&body),
        "accepting request {id} did not return <status code=\"ok\"/>; body: {body}"
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
    let body = request_xml(&base, &id);
    // The filed request is open, from the branch, to the target, and
    // carries the default cleanup: whatever the server holds must be
    // what was filed.
    assert_eq!(
        request_state(&body).as_deref(),
        Some("new"),
        "filed request {id} must be new; body: {body}"
    );
    assert_eq!(
        xml_attr_after(&body, "<source ", "project").as_deref(),
        Some(source_project.as_str()),
        "filed request {id} must come from the branch; body: {body}"
    );
    assert_eq!(
        xml_attr_after(&body, "<source ", "package").as_deref(),
        Some(package.as_str()),
        "filed request {id} must submit the branch package; body: {body}"
    );
    assert_eq!(
        xml_attr_after(&body, "<target ", "project").as_deref(),
        Some(target_project.as_str()),
        "filed request {id} must target the target project; body: {body}"
    );
    assert_eq!(
        xml_attr_after(&body, "<target ", "package").as_deref(),
        Some(package.as_str()),
        "filed request {id} must target the package; body: {body}"
    );
    assert!(
        body.contains("<sourceupdate>cleanup</sourceupdate>"),
        "filed request {id} must carry the default cleanup; body: {body}"
    );
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
    let old = request_xml(&base, &id1);
    assert_eq!(
        request_state(&old).as_deref(),
        Some("superseded"),
        "the old request {id1} must be superseded; body: {old}"
    );
    assert_eq!(
        superseded_by(&old).as_deref(),
        Some(id2.as_str()),
        "the old request {id1} must point at the new one; body: {old}"
    );
    let new = request_xml(&base, &id2);
    assert_eq!(
        request_state(&new).as_deref(),
        Some("new"),
        "the new request {id2} must be new; body: {new}"
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
