//! Live tests against a disposable Gitea instance.
//!
//! These are the Gitea half of the live-test design (see
//! `docs/architecture.md`): a real (temporary) Gitea instance, asserting
//! on **server state** — whatever geectl does must actually show up on
//! the server. That is what catches drift as the server changes
//! underneath us, and — the reason this file exists — what validates
//! geectl's auth against a real server. The unit tests prove geectl
//! sends the right `Authorization` header to a loopback server; these
//! prove a real Gitea accepts the token and returns valid responses.
//! A `Bearer` token that Gitea rejected would fail here with
//! `AuthFailed`, not in a unit test.
//!
//! These tests run in the per-PR gate (the `test` job), against the
//! instance the provisioning in `tests/gitea/` spins up. They are
//! skipped unless `GEETCL_GITEA_URL` is set to the instance's base URL
//! (e.g. `http://127.0.0.1:3001`), so `cargo test` stays green without
//! an instance. They are fast enough for the gate (see
//! `tests/gitea/README.md` for the measured timings): a single prebuilt
//! image, ~20s end to end, where the OBS temporal job takes minutes.
//!
//! The instance is plain HTTP on localhost. That is the single
//! sanctioned exception to geectl's https-only URL rule, and it is safe
//! because:
//! - the instance is loopback-only (the harness rejects any
//!   non-loopback URL outright — see `gitea_url`),
//! - the instance is ephemeral (created fresh per run, destroyed after),
//! - the credentials are ephemeral (a user + token created by
//!   `provision.sh`, never a real credential), and
//! - the harness authenticates through geectl's own `EnvGiteaAuth`
//!   (Bearer token), so the token goes to the same loopback-only
//!   ephemeral instance and nowhere else.
//!
//! The harness talks to the instance through geectl's own `UreqClient`
//! with `EnvGiteaAuth` — the production auth path, not a hand-rolled
//! header. That is the point: the auth resolution, the transport guard,
//! and the status mapping are all exercised against a real server.

use geectl::auth::{AuthProvider, Credentials, EnvGiteaAuth};
use geectl::error::ErrorKind;
use geectl::http::{HttpClient, Timeout, UreqClient};

/// Whether `url` points at a loopback host.
///
/// The host is extracted exactly: the authority after `http://` up to
/// the next `/`, minus any `:port`, minus IPv6 brackets. A prefix check
/// would also accept `http://127.0.0.1.evil.com`, which is not loopback.
fn is_loopback_url(url: &str) -> bool {
    let authority = url
        .strip_prefix("http://")
        .unwrap_or("")
        .split('/')
        .next()
        .unwrap_or("");
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    host == "127.0.0.1" || host == "localhost" || host == "::1"
}

/// The Gitea instance's base URL, from the environment.
///
/// Returns `None` (skip the test) when `GEETCL_GITEA_URL` is unset or
/// empty. Rejects any URL that is not loopback-only: the instance is
/// plain HTTP, so pointing the harness at a non-localhost server would
/// send the token in cleartext over a real network. That must fail
/// fast, not silently.
fn gitea_url() -> Option<String> {
    let url = std::env::var("GEETCL_GITEA_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())?;
    let url = url.trim_end_matches('/').to_string();
    assert!(
        is_loopback_url(&url),
        "GEETCL_GITEA_URL must be a loopback URL (http://127.0.0.1, http://localhost, or http://[::1]); got {url}"
    );
    Some(url)
}

/// A client for the Gitea instance, authenticated through geectl's own
/// `EnvGiteaAuth` (Bearer token from `GEECTL_GITEA_TOKEN` or
/// `GEECTL_GITEA_TOKEN_FILE`).
///
/// Call only after `gitea_url()` returned `Some` (the URL is set).
/// Panics when no token is configured — the provision script always
/// creates one, so a missing token is a broken setup, not a reason to
/// skip. A config error (both token sources set, malformed token) also
/// panics: that is the auth layer failing, and the test must say so.
fn gitea_client() -> UreqClient {
    let credentials = EnvGiteaAuth
        .credentials()
        .expect("reading the Gitea token from the environment");
    match credentials {
        Credentials::None => panic!(
            "GEETCL_GITEA_URL is set but no Gitea token is configured; \
             set GEECTL_GITEA_TOKEN or GEECTL_GITEA_TOKEN_FILE \
             (provision.sh writes the token to tests/gitea/.gitea-token)"
        ),
        Credentials::Bearer { .. } => {
            UreqClient::with_auth(Timeout::default(), Some(Box::new(EnvGiteaAuth)))
                .expect("default timeout is valid")
        }
        // EnvGiteaAuth only ever resolves None or Bearer; anything else
        // would be a bug in the auth layer.
        _ => panic!("the Gitea token resolved to unexpected credentials"),
    }
}

/// A repository name unique to this run, so the test can run twice
/// against one instance (e.g. a local re-run without teardown) without
/// colliding with its own previous creation. The process id is included
/// so two test processes in the same second do not collide either.
fn unique_repo_name() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time is after the epoch")
        .as_secs();
    format!("geectl-live-{}-{}", std::process::id(), secs)
}

#[test]
fn gitea_user_is_the_provisioned_user() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEETCL_GITEA_URL is not set");
        return;
    };
    let client = gitea_client();
    // The token is accepted and the response is a valid user: this is
    // the auth validation the unit tests cannot do. A `Bearer` token
    // Gitea rejected would fail here with AuthFailed.
    let body = client
        .get(&format!("{base}/api/v1/user"))
        .expect("GET /api/v1/user on the Gitea instance");
    let user: serde_json::Value =
        serde_json::from_str(&body).expect("GET /api/v1/user returns valid JSON");
    assert_eq!(
        user["login"].as_str(),
        Some("geectl"),
        "GET /api/v1/user did not return the provisioned user; body: {body}"
    );
    assert_eq!(
        user["is_admin"].as_bool(),
        Some(true),
        "the provisioned user must be an admin; body: {body}"
    );
}

#[test]
fn gitea_create_repo_shows_up_on_server() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEETCL_GITEA_URL is not set");
        return;
    };
    let client = gitea_client();
    let repo = unique_repo_name();
    let expected_full_name = format!("geectl/{repo}");
    // Create the repository through geectl (POST with the Bearer
    // token). A 201 means the server accepted the creation.
    let create_body = serde_json::json!({
        "name": repo,
        "private": false,
        "description": "geectl live-test repository",
    })
    .to_string();
    let created = client
        .post_json(&format!("{base}/api/v1/user/repos"), &create_body)
        .expect("POST /api/v1/user/repos on the Gitea instance");
    let created: serde_json::Value =
        serde_json::from_str(&created).expect("POST /api/v1/user/repos returns valid JSON");
    assert_eq!(
        created["full_name"].as_str(),
        Some(expected_full_name.as_str()),
        "POST /api/v1/user/repos did not return the created repository; body: {created}"
    );
    // Read it back: the server state must show the creation.
    let read_back = client
        .get(&format!("{base}/api/v1/repos/{expected_full_name}"))
        .expect("GET the created repository on the Gitea instance");
    let read_back: serde_json::Value =
        serde_json::from_str(&read_back).expect("GET the repository returns valid JSON");
    assert_eq!(
        read_back["name"].as_str(),
        Some(repo.as_str()),
        "GET the created repository did not return it; body: {read_back}"
    );
    // And it must be in the authenticated user's repository list.
    let list = client
        .get(&format!("{base}/api/v1/user/repos"))
        .expect("GET /api/v1/user/repos on the Gitea instance");
    let list: serde_json::Value =
        serde_json::from_str(&list).expect("GET /api/v1/user/repos returns valid JSON");
    let names: Vec<&str> = list
        .as_array()
        .expect("GET /api/v1/user/repos returns an array")
        .iter()
        .filter_map(|r| r["full_name"].as_str())
        .collect();
    assert!(
        names.contains(&expected_full_name.as_str()),
        "the created repository is not in the user's repository list; list: {names:?}"
    );
}

#[test]
fn gitea_missing_repo_is_not_found() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEETCL_GITEA_URL is not set");
        return;
    };
    let client = gitea_client();
    // A repository that was never created must map to geectl's
    // NotFound, not to a generic failure: the status→error mapping is
    // exercised against a real server, not a fixture.
    let err = client
        .get(&format!(
            "{base}/api/v1/repos/geectl/geectl-live-no-such-repo"
        ))
        .expect_err("a missing repository must not return 200");
    assert_eq!(
        err.kind(),
        ErrorKind::NotFound,
        "a missing repository must map to NotFound; got {err:?}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_check_is_exact() {
        assert!(is_loopback_url("http://127.0.0.1:3001"));
        assert!(is_loopback_url("http://127.0.0.1"));
        assert!(is_loopback_url("http://localhost:3001"));
        assert!(is_loopback_url("http://localhost"));
        assert!(is_loopback_url("http://[::1]:3001"));
        assert!(is_loopback_url("http://[::1]"));
        assert!(is_loopback_url("http://127.0.0.1:3001/api/v1/user"));
        // A host that merely starts with a loopback address is not one.
        assert!(!is_loopback_url("http://127.0.0.1.evil.com"));
        assert!(!is_loopback_url("http://localhost.evil.com"));
        assert!(!is_loopback_url("http://example.com"));
        // HTTPS is not loopback-HTTP either: the exception is for the
        // plain-HTTP disposable instance only.
        assert!(!is_loopback_url("https://127.0.0.1:3001"));
        assert!(!is_loopback_url(""));
    }

    #[test]
    fn unique_repo_name_is_stable_within_a_second() {
        let first = unique_repo_name();
        assert!(first.starts_with("geectl-live-"));
        // The suffix is "<process-id>-<seconds-since-epoch>": both
        // numeric, so the name is deterministic within a process and a
        // second, and two processes in the same second still differ.
        let suffix = &first["geectl-live-".len()..];
        let parts: Vec<&str> = suffix.split('-').collect();
        assert_eq!(parts.len(), 2);
        assert!(parts[0].parse::<u32>().is_ok());
        assert!(parts[1].parse::<u64>().is_ok());
    }
}
