//! Live tests against a temporal OBS instance.
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
//! without an instance.
//!
//! The instance is plain HTTP on localhost. That is the single sanctioned
//! exception to geectl's https-only URL rule, and it is safe because:
//! - the instance is loopback-only (the harness rejects any
//!   non-loopback URL outright — see `temporal_url`),
//! - the instance is ephemeral (created fresh per run, destroyed after),
//! - the credentials are ephemeral (the OBS development default, never a
//!   real credential), and
//! - the public reads (`/about`, a missing project) run with no auth at
//!   all (`NoAuth`): nothing secret is ever on the wire for them. The
//!   project-list read and the mutation test send the bootstrap admin
//!   credentials to the same loopback-only ephemeral instance — the
//!   server requires auth for the project list (`GET /source` 401s
//!   without it).
//!
//! The harness talks to the instance through geectl's own `UreqClient`
//! (not `UrlBuilder`, which rightly rejects `http://` bases) — the HTTP
//! exception is confined to this file and `tests/temporal_obs.rs`.
//!
//! The mutation assertion is `temporal_create_project_shows_up_on_server`:
//! it creates a project through geectl's own `ObsClient::create_project`
//! with the bootstrap admin credentials and reads it back from the
//! server.

use geectl::auth::Credentials;
use geectl::error::ErrorKind;
use geectl::http::{HttpClient, Timeout, UreqClient};
use geectl::obs::{DryRun, ObsClient};

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
/// `LOCALHOST` is loopback. The userinfo handling follows
/// `sanitize_url_for_error` (src/http.rs).
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
    // DNS is case-insensitive: `LOCALHOST` is loopback too.
    host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost") || host == "::1"
}

/// Strip userinfo (username and password) from a URL for error
/// messages, mirroring `sanitize_url_for_error` in `src/http.rs`.
/// A password may itself contain `@`, so the strip is at the LAST
/// `@` in the authority; an `@` in the path, query, or fragment is
/// data, not a credential separator, and is left alone.
fn sanitize_url_for_error(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let after_scheme = scheme_end + 3;
    let rest = &url[after_scheme..];
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let Some(at) = rest[..authority_end].rfind('@') else {
        return url.to_string();
    };
    let mut out = String::with_capacity(url.len());
    out.push_str(&url[..after_scheme]);
    out.push_str(&rest[at + 1..]);
    out
}

/// Validate a temporal base URL: trim whitespace, strip trailing
/// slashes, require a bare base (no path, query, or fragment), reject
/// userinfo, and require a loopback host. Returns the cleaned URL, or
/// the reason it is invalid. Pure, so it is unit-testable;
/// `temporal_url` turns the error into a panic at the boundary.
/// Error messages echo the URL with userinfo stripped, never the raw
/// input: a password in the base must not land in a panic message.
fn validate_temporal_base(raw: &str) -> Result<String, String> {
    let url = raw.trim().trim_end_matches('/').to_string();
    // Shown in error messages: the URL with any userinfo stripped,
    // so a rejected base never echoes a password.
    let shown = sanitize_url_for_error(&url);
    // The scheme is checked first: the bare-base check below sees the
    // `//` in any `scheme://` URL, so a non-http URL such as
    // `https://127.0.0.1:3000` would otherwise be reported as a
    // bare-base violation instead of a scheme violation.
    if !url.starts_with("http://") {
        return Err(format!(
            "must be an http:// URL (the temporal instance is plain HTTP on loopback); got {shown}"
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
            "must be a bare base URL (http://127.0.0.1:3000), with no path, query, or fragment; got {shown}"
        ));
    }
    // No userinfo in the base: Phase-B credentials go through the auth
    // provider, not the URL. The authority is the whole remainder here
    // (the bare-base check above already rejected any path, query, or
    // fragment), so any `@` is a credential separator.
    if after_scheme.contains('@') {
        return Err(format!(
            "must not contain userinfo (no username or password in the URL); credentials go through the auth provider; got {shown}"
        ));
    }
    if !is_loopback_url(&url) {
        return Err(format!(
            "must be a loopback URL (http://127.0.0.1, http://localhost, or http://[::1]); got {shown}"
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

/// A client for the temporal instance. No auth: the public endpoints
/// these tests use need none, and the instance is a throwaway.
fn temporal_client() -> UreqClient {
    UreqClient::new(Timeout::default()).expect("default timeout is valid")
}

/// A raw GET against the temporal instance with the bootstrap admin
/// credentials. The project list (`GET /source`) requires auth, and
/// the created project's meta is read back the same way — the typed
/// `ObsClient` operations cover actions, not ad-hoc reads.
fn temporal_authed_get(url: &str) -> Result<String, geectl::error::GeectlError> {
    temporal_client().get_with(url, &temporal_credentials())
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

/// A client for the temporal instance, authenticated with the
/// bootstrap admin credentials.
fn temporal_client_authed(base: &str) -> ObsClient<'_> {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    ObsClient::with_base_and_credentials(http, base, temporal_credentials())
        .expect("the temporal base is a loopback http URL")
}

/// Extract the text of `<tag>...</tag>` from a small XML body. This is a
/// test-harness helper, not a parser: the full XML→JSON mapping is a
/// later module. It is enough to assert the server returned the element
/// with non-empty content.
fn xml_text(body: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = body.find(&open)? + open.len();
    let end = body[start..].find(&close)? + start;
    let text = body[start..end].trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

#[test]
fn temporal_about_reports_a_real_obs() {
    let Some(base) = temporal_url() else {
        return;
    };
    let body = temporal_client()
        .get(&format!("{base}/about"))
        .expect("GET /about on the temporal instance");
    // The OBS `/about` endpoint is public (no auth) and returns the
    // server's own version. If this body is not a real OBS answering,
    // the title and revision are absent — and if OBS ever changes the
    // shape, this fails, which is the drift this layer exists to catch.
    assert!(
        body.contains("<title>Open Build Service API</title>"),
        "GET /about did not return an OBS API title; body: {body}"
    );
    let revision = xml_text(&body, "revision");
    assert!(
        revision.is_some(),
        "GET /about returned no <revision>; body: {body}"
    );
    eprintln!("temporal instance OBS revision: {}", revision.unwrap());
}

#[test]
fn temporal_missing_project_is_not_found() {
    let Some(base) = temporal_url() else {
        return;
    };
    // A project that was never created must map to geectl's NotFound,
    // not to a generic failure: the status→error mapping is exercised
    // against a real server, not a fixture.
    let err = temporal_client()
        .get(&format!(
            "{base}/public/source/geectl-temporal-no-such-project"
        ))
        .expect_err("a missing project must not return 200");
    assert_eq!(
        err.kind(),
        ErrorKind::NotFound,
        "a missing project must map to NotFound; got {err:?}"
    );
}

#[test]
fn temporal_project_list_is_a_directory() {
    let Some(base) = temporal_url() else {
        return;
    };
    // The project list lives at GET /source and returns a <directory>
    // of <entry name="..."/> elements — there is no bare
    // /public/source route (the public controller serves only
    // /public/source/:project and below, so that path 404s), and the
    // list is a directory, not a <collection> envelope. The list
    // requires auth (401 without it), so this read uses the bootstrap
    // admin credentials like the mutation test. A fresh bootstrapped
    // instance may hold no projects yet; the assertion is on the
    // shape, not the contents — the contents are whatever the server
    // actually holds, which is the point.
    let body = temporal_authed_get(&format!("{base}/source"))
        .expect("GET /source on the temporal instance");
    assert!(
        body.contains("<directory"),
        "GET /source did not return a <directory>; body: {body}"
    );
}

/// Create a project through geectl and read it back from the server,
/// asserting the server state shows the creation. The project name is
/// unique per run (process id plus a nanosecond timestamp): the
/// temporal tests run in parallel, and the instance is shared with
/// the `temporal_obs` target's tests. The project is left behind —
/// the instance is destroyed after the run.
#[test]
fn temporal_create_project_shows_up_on_server() {
    let Some(base) = temporal_url() else {
        return;
    };
    let client = temporal_client_authed(&base);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time is after the epoch")
        .as_nanos();
    let name = format!("geectl-tmp-create-{}-{nanos}", std::process::id());
    client
        .create_project(
            &name,
            "geectl temporal test project",
            "Created by the geectl temporal live test; the instance is destroyed after the run.",
            &DryRun::new(false),
        )
        .expect("create a project on the temporal instance");
    // Read the project's stored meta back: a project directory
    // listing shows its packages (none yet), not the project itself,
    // so the name is asserted on the meta, which renders the stored
    // `<project name="...">` with its title.
    let body = temporal_authed_get(&format!("{base}/source/{name}/_meta"))
        .expect("GET the created project's meta back from the server");
    assert!(
        body.contains(&format!("name=\"{name}\"")),
        "the created project {name} must read back under its own name; body: {body}"
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
        // A base with a path, query, or fragment is rejected: the
        // tests append paths themselves, so a non-bare base would
        // double them.
        assert!(validate_temporal_base("http://127.0.0.1:3000/about").is_err());
        assert!(validate_temporal_base("http://127.0.0.1:3000?x=1").is_err());
        assert!(validate_temporal_base("http://127.0.0.1:3000#frag").is_err());
        // Non-loopback is rejected, including behind userinfo.
        assert!(validate_temporal_base("http://example.com").is_err());
        assert!(validate_temporal_base("http://127.0.0.1:80@evil.com").is_err());
        // Userinfo is rejected even on a loopback host: Phase-B
        // credentials go through the auth provider, not the URL.
        assert!(validate_temporal_base("http://user@127.0.0.1:3000").is_err());
        assert!(validate_temporal_base("http://user:p@ss@127.0.0.1:3000").is_err());
        // And a rejected base never echoes the password.
        let userinfo_err = validate_temporal_base("http://user:s3cret@127.0.0.1:3000").unwrap_err();
        assert!(
            !userinfo_err.contains("s3cret"),
            "a rejected base must not echo the password; got {userinfo_err}"
        );
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
    fn error_urls_have_no_userinfo() {
        assert_eq!(
            sanitize_url_for_error("http://user:p@ss@127.0.0.1:3000/about"),
            "http://127.0.0.1:3000/about"
        );
        assert_eq!(
            sanitize_url_for_error("http://127.0.0.1:3000/about"),
            "http://127.0.0.1:3000/about"
        );
        // An `@` in the path is data, not a credential separator.
        assert_eq!(
            sanitize_url_for_error("http://127.0.0.1:3000/@foo"),
            "http://127.0.0.1:3000/@foo"
        );
    }

    #[test]
    fn xml_text_extracts_element_content() {
        let body =
            "<about><title>Open Build Service API</title><revision>2.10.1</revision></about>";
        assert_eq!(xml_text(body, "revision").as_deref(), Some("2.10.1"));
        assert_eq!(
            xml_text(body, "title").as_deref(),
            Some("Open Build Service API")
        );
        assert_eq!(xml_text(body, "missing"), None);
        assert_eq!(xml_text("<a><b></b></a>", "b"), None);
    }
}
