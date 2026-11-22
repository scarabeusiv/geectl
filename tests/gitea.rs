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
//! skipped unless `GEECTL_GITEA_URL` is set to the instance's base URL
//! (e.g. `http://127.0.0.1:3001`), so `cargo test` stays green without
//! an instance. They are fast enough for the gate (see
//! `tests/gitea/README.md` for the measured timings): a single prebuilt
//! image, ~20s end to end, where the OBS live job takes minutes.
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
use geectl::debug::Debug;
use geectl::error::ErrorKind;
use geectl::gitea::{ForkRepoOptions, GiteaClient, RepoCreateOptions};
use geectl::http::{HttpClient, Timeout, UreqClient};
use geectl::obs::DryRun;

/// Whether `url` points at a loopback host over plain HTTP.
///
/// The harness only ever runs against the plain-HTTP disposable
/// instance, so the scheme must be `http://`. The host parsing is the
/// production check (`geectl::http::is_loopback_url`), shared rather
/// than copied: the copy split on the first colon and missed userinfo,
/// so `http://localhost:3001@evil.com` read as loopback.
fn is_loopback_url(url: &str) -> bool {
    url.starts_with("http://") && geectl::http::is_loopback_url(url)
}

/// The Gitea instance's base URL, from the environment.
///
/// Returns `None` (skip the test) when `GEECTL_GITEA_URL` is unset or
/// empty — unless the run is in a job that provisions an instance (see
/// below). Rejects any URL that is not loopback-only: the instance is
/// plain HTTP, so pointing the harness at a non-localhost server would
/// send the token in cleartext over a real network. That must fail
/// fast, not silently.
fn gitea_url() -> Option<String> {
    let url = match std::env::var("GEECTL_GITEA_URL") {
        Ok(url) if !url.trim().is_empty() => url,
        _ => {
            // Only a job that provisions an instance expects the URL:
            // the `test` job sets GEECTL_GITEA_EXPECTED (see test.yml),
            // so a missing URL there means provisioning failed to run
            // or to export it — fail loudly instead of silently
            // skipping the whole live suite. Other jobs (beta, macOS)
            // do not provision and skip quietly: `CI` is set in every
            // GitHub job, so gating on it panicked there too.
            if std::env::var_os("GEECTL_GITEA_EXPECTED").is_some_and(|v| !v.is_empty()) {
                panic!("GEECTL_GITEA_URL is not set: the provision step must export it");
            }
            return None;
        }
    };
    let url = url.trim_end_matches('/').to_string();
    assert!(
        is_loopback_url(&url),
        "GEECTL_GITEA_URL must be a loopback URL (http://127.0.0.1, http://localhost, or http://[::1]); got {}",
        geectl::http::sanitize_url_for_error(&url)
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
            "GEECTL_GITEA_URL is set but no Gitea token is configured; \
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

/// The Gitea user provision.sh created (`GITEA_USER`, default
/// `geectl`): the same variable the provision script reads, so an
/// overridden user does not make the harness assert against a user
/// that does not exist.
fn gitea_user() -> String {
    std::env::var("GITEA_USER")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "geectl".to_string())
}

/// A process-wide counter, so two tests that read the clock in
/// the same nanosecond still get different names: the timestamp
/// alone collided under parallel runs (two `create_repo_raw` calls
/// with one name, the second answering 409 — reproduced twice in
/// the live loop, both times in `gitea_fork_round_trip`).
fn unique_counter() -> u64 {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A repository name unique to this run, so the test can run twice
/// against one instance (e.g. a local re-run without teardown) without
/// colliding with its own previous creation. The timestamp is
/// nanoseconds: two calls in the same process within a second do not
/// collide (whole seconds did). The process id is included so two
/// test processes do not collide either. The counter is included so
/// two calls in the same nanosecond do not collide either.
fn unique_repo_name() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time is after the epoch")
        .as_nanos();
    format!(
        "geectl-live-{}-{}-{}",
        std::process::id(),
        nanos,
        unique_counter()
    )
}

#[test]
fn gitea_user_is_the_provisioned_user() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEECTL_GITEA_URL is not set");
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
        Some(gitea_user().as_str()),
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
        eprintln!("skipping Gitea live test: GEECTL_GITEA_URL is not set");
        return;
    };
    let client = gitea_client();
    let repo = unique_repo_name();
    let expected_full_name = format!("{}/{repo}", gitea_user());
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
    // The limit is explicit: relying on the server's default page
    // size would let the assertion silently change meaning.
    let list = client
        .get(&format!("{base}/api/v1/user/repos?limit=50"))
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

/// A typed `GiteaClient` for the instance, authenticated through
/// `EnvGiteaAuth` — the same token the raw harness uses, so the typed
/// operations are exercised through the production auth path. The
/// client borrows the HTTP client: both live in the test body.
fn typed_client<'a>(http: &'a UreqClient, base: &str) -> GiteaClient<'a> {
    let credentials = EnvGiteaAuth
        .credentials()
        .expect("reading the Gitea token from the environment");
    GiteaClient::with_base(http, base, credentials).expect("typed Gitea client construction")
}

/// Fork through the typed operation in live mode (dry-run off)
/// and return the fork. A live fork always returns `Some`.
fn fork_live(
    client: &GiteaClient,
    owner: &str,
    repo: &str,
    options: &ForkRepoOptions,
) -> geectl::gitea::GiteaRepository {
    client
        .fork_repo(owner, repo, options, &DryRun::new(false))
        .expect("typed fork_repo forks the repository")
        .expect("a non-dry-run fork returns the fork")
}

/// Delete through the typed operation in live mode and return the
/// deleted name. A live delete always returns `Some`; force is
/// passed explicitly — the delete is destructive, so the caller
/// confirms it.
fn delete_live(client: &GiteaClient, owner: &str, repo: &str) -> String {
    client
        .repo_delete(owner, repo, true, &DryRun::new(false))
        .expect("typed repo_delete deletes the repository")
        .expect("a non-dry-run delete returns the name")
}

/// Create a repository through the raw API and return its full name.
/// Repo creation is not a phase-1 operation, so the setup stays raw;
/// the assertions then go through the typed operations.
fn create_repo_raw(http: &UreqClient, base: &str, name: &str) -> String {
    let body = serde_json::json!({
        "name": name,
        "private": false,
        "description": "geectl live-test repository",
    })
    .to_string();
    let created = http
        .post_json(&format!("{base}/api/v1/user/repos"), &body)
        .expect("POST /api/v1/user/repos on the Gitea instance");
    let created: serde_json::Value =
        serde_json::from_str(&created).expect("POST /api/v1/user/repos returns valid JSON");
    created["full_name"]
        .as_str()
        .expect("the created repository has a full_name")
        .to_string()
}

/// Create a repository through the typed phase-2 operation and
/// return the created repo. A live create always returns `Some`.
/// The default branch is named explicitly: the branch tests commit
/// onto it, so it must be the branch the repo records.
fn create_repo_typed(
    client: &GiteaClient,
    name: &str,
    default_branch: &str,
) -> geectl::gitea::GiteaRepository {
    client
        .repo_create(
            name,
            &RepoCreateOptions {
                private: false,
                description: None,
                default_branch: Some(default_branch.to_string()),
            },
            &DryRun::new(false),
        )
        .expect("typed repo_create creates the repository")
        .expect("a non-dry-run create returns the repo")
}

/// The address of a repository on the live instance, for the raw
/// setup helpers — base URL + owner + repo in one value, so the
/// helpers stay under clippy's argument limit.
struct RepoAddr<'a> {
    base: &'a str,
    owner: &'a str,
    repo: &'a str,
}

/// Create a file (and its commit) through the raw contents API
/// and return the commit SHA. `new_branch` creates the branch from
/// `branch` instead of committing onto it — the branch-creation
/// setup the branch tests use, since branch creation itself is a
/// phase-4 operation. Raw setup, like `create_repo_raw`: the
/// assertions then go through the typed operations.
fn create_commit_raw(
    http: &UreqClient,
    at: &RepoAddr,
    path: &str,
    content: &str,
    branch: &str,
    new_branch: Option<&str>,
) -> String {
    use base64::Engine as _;
    let mut body = serde_json::json!({
        "message": format!("add {path}"),
        "content": base64::engine::general_purpose::STANDARD.encode(content),
        "branch": branch,
    });
    if let Some(new_branch) = new_branch {
        body["new_branch"] = serde_json::Value::String(new_branch.to_string());
    }
    let created = http
        .post_json(
            &format!(
                "{}/api/v1/repos/{}/{}/contents/{path}",
                at.base, at.owner, at.repo
            ),
            &body.to_string(),
        )
        .expect("POST the contents API on the Gitea instance");
    let created: serde_json::Value =
        serde_json::from_str(&created).expect("POST the contents API returns valid JSON");
    created["commit"]["sha"]
        .as_str()
        .expect("the contents API returns the commit SHA")
        .to_string()
}

/// An organization name unique to this run. Shorter than
/// `unique_repo_name`: Gitea caps usernames at 39 characters, and
/// `geectl-live-<pid>-<nanos>` can exceed it. The counter keeps
/// two calls in the same second apart.
fn unique_org_name() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time is after the epoch")
        .as_secs();
    format!(
        "geectl-org-{}-{secs}-{}",
        std::process::id(),
        unique_counter()
    )
}

#[test]
fn gitea_repo_get_round_trip_and_missing() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEECTL_GITEA_URL is not set");
        return;
    };
    let http = gitea_client();
    let client = typed_client(&http, &base);
    let repo = unique_repo_name();
    let full_name = create_repo_raw(&http, &base, &repo);
    // The typed read shows the creation on the server, with the
    // fields the phase-1 operations rely on.
    let read = client
        .repo_get(&gitea_user(), &repo)
        .expect("typed repo_get reads the created repository");
    assert_eq!(read.full_name, full_name);
    assert_eq!(read.owner.login, gitea_user());
    assert_eq!(read.name, repo);
    assert!(
        read.parent.is_none(),
        "a freshly created repository is not a fork; body: {read:?}"
    );
    assert!(
        !read.default_branch.is_empty(),
        "a freshly created repository has a default branch; body: {read:?}"
    );
    // And a missing repository maps to NotFound through the typed
    // operation, not just the raw transport.
    let err = client
        .repo_get(&gitea_user(), "geectl-live-no-such-repo")
        .expect_err("a missing repository must not return a value");
    assert_eq!(
        err.kind(),
        ErrorKind::NotFound,
        "a missing repository must map to NotFound; got {err:?}"
    );
}

#[test]
fn gitea_repo_delete_removes_the_repo() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEECTL_GITEA_URL is not set");
        return;
    };
    let http = gitea_client();
    let client = typed_client(&http, &base);
    let repo = unique_repo_name();
    create_repo_raw(&http, &base, &repo);
    // Without force the delete is refused client-side, before any
    // server read — and the repository survives the refusal.
    let err = client
        .repo_delete(&gitea_user(), &repo, false, &DryRun::new(false))
        .expect_err("deleting without force must fail");
    assert_eq!(
        err.kind(),
        ErrorKind::InvalidInput,
        "deleting without force must map to InvalidInput; got {err:?}"
    );
    client
        .repo_get(&gitea_user(), &repo)
        .expect("the refused delete must leave the repository in place");
    let deleted = delete_live(&client, &gitea_user(), &repo);
    assert_eq!(deleted, repo);
    // The deletion shows up on the server: the repo no longer reads.
    let err = client
        .repo_get(&gitea_user(), &repo)
        .expect_err("the deleted repository must not read back");
    assert_eq!(
        err.kind(),
        ErrorKind::NotFound,
        "the deleted repository must map to NotFound; got {err:?}"
    );
    // Deleting a missing repository is NotFound too.
    let err = client
        .repo_delete(
            &gitea_user(),
            "geectl-live-no-such-repo",
            true,
            &DryRun::new(false),
        )
        .expect_err("deleting a missing repository must fail");
    assert_eq!(
        err.kind(),
        ErrorKind::NotFound,
        "deleting a missing repository must map to NotFound; got {err:?}"
    );
}

#[test]
fn gitea_repo_list_forms() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEECTL_GITEA_URL is not set");
        return;
    };
    let http = gitea_client();
    let client = typed_client(&http, &base);
    // The osc `git-repo-list.feature` forms, against the real
    // layout: the org form lists the populated mirror (among
    // whatever the org holds — the org is shared across the
    // parallel live tests), the user form lists the token owner's
    // repos, and the combined form lists the user's repos first,
    // then the org's.
    let org_repos = client
        .repo_list(None, Some("pool"))
        .expect("typed repo_list lists the pool org");
    // Contains, not exact equality: the org is shared mutable
    // state across the parallel live tests, and a future test
    // creating a pool repo must not break this one. The mirror is
    // the only pool repo today.
    assert!(
        org_repos.iter().any(|r| r.full_name == "pool/systemd"),
        "the pool org's repository list must contain the mirror repo; got {} repos",
        org_repos.len()
    );
    // Fork the mirror so the user owns a repo to list: a re-run
    // reuses the existing fork (same assertions hold).
    let forked = fork_live(&client, "pool", "systemd", &ForkRepoOptions::default());
    assert_eq!(forked.full_name, format!("{}/systemd", gitea_user()));
    let user_repos = client
        .repo_list(Some(&gitea_user()), None)
        .expect("typed repo_list lists the user");
    let user_names: Vec<&str> = user_repos.iter().map(|r| r.full_name.as_str()).collect();
    // The check is hoisted out of the assert so the failure
    // message is a static string: no repository name reaches the
    // log on failure. (CodeQL's cleartext-logging flagged the
    // names in the message, and the count form was still flagged.)
    let has_fork = user_names.contains(&forked.full_name.as_str());
    assert!(has_fork, "the user's repository list must contain the fork");
    // Both: the user's repos first, then the org's.
    let both = client
        .repo_list(Some(&gitea_user()), Some("pool"))
        .expect("typed repo_list lists user and org");
    let both_names: Vec<&str> = both.iter().map(|r| r.full_name.as_str()).collect();
    let user_at = both_names
        .iter()
        .position(|n| *n == forked.full_name.as_str())
        .expect("the combined list contains the user's fork");
    let org_at = both_names
        .iter()
        .position(|n| *n == "pool/systemd")
        .expect("the combined list contains the org repo");
    assert!(
        user_at < org_at,
        "the combined list must name the user's repos before the org's; got {} repos",
        both_names.len()
    );
    // Neither filter: the token owner's own repos — the same list
    // the user form returns for the token owner, modulo other
    // tests' repos created between the two calls (contains, not
    // equality).
    let own = client
        .repo_list(None, None)
        .expect("typed repo_list lists the token owner's repos");
    let own_names: Vec<&str> = own.iter().map(|r| r.full_name.as_str()).collect();
    assert!(
        own_names.contains(&forked.full_name.as_str()),
        "the owner's repository list must contain the fork; got {} repos",
        own_names.len()
    );
}

#[test]
fn gitea_repo_create_round_trip_and_collision() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEECTL_GITEA_URL is not set");
        return;
    };
    let http = gitea_client();
    let client = typed_client(&http, &base);
    let repo = unique_repo_name();
    let created = create_repo_typed(&client, &repo, "factory");
    assert_eq!(created.full_name, format!("{}/{repo}", gitea_user()));
    assert_eq!(created.name, repo);
    assert_eq!(created.default_branch, "factory");
    assert!(
        created.parent.is_none(),
        "a freshly created repository is not a fork; body: {created:?}"
    );
    // The creation shows up on the server: the repo reads back,
    // and the owner's list contains it.
    let read = client
        .repo_get(&gitea_user(), &repo)
        .expect("typed repo_get reads the created repository");
    assert_eq!(read.full_name, created.full_name);
    let own = client
        .repo_list(None, None)
        .expect("typed repo_list lists the token owner's repos");
    assert!(
        own.iter().any(|r| r.full_name == created.full_name),
        "the owner's repository list must contain the created repo"
    );
    // Creating the same name again is a collision, not a second
    // repo.
    let err = client
        .repo_create(&repo, &RepoCreateOptions::default(), &DryRun::new(false))
        .expect_err("creating a repository of an existing name must fail");
    assert_eq!(
        err.kind(),
        ErrorKind::AlreadyExists,
        "creating a repository of an existing name must be AlreadyExists; got {err:?}"
    );
    // Clean up: the created repo is removed again.
    let deleted = delete_live(&client, &gitea_user(), &repo);
    assert_eq!(deleted, repo);
}

#[test]
fn gitea_repo_create_dry_run_plans_and_creates_nothing() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEECTL_GITEA_URL is not set");
        return;
    };
    let http = gitea_client();
    let client = typed_client(&http, &base);
    let repo = unique_repo_name();
    let planned = client
        .repo_create(&repo, &RepoCreateOptions::default(), &DryRun::new(true))
        .expect("a dry-run create records the plan");
    assert_eq!(planned, None, "a dry-run create returns no repo");
    // And nothing was created on the server.
    let err = client
        .repo_get(&gitea_user(), &repo)
        .expect_err("the dry-run create must not create the repository");
    assert_eq!(
        err.kind(),
        ErrorKind::NotFound,
        "the dry-run create must not create the repository; got {err:?}"
    );
}

#[test]
fn gitea_branch_read_round_trip() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEECTL_GITEA_URL is not set");
        return;
    };
    let http = gitea_client();
    let client = typed_client(&http, &base);
    let repo = unique_repo_name();
    create_repo_typed(&client, &repo, "factory");
    // The first commit creates the default branch; the second
    // commit creates a branch off it; the third proves a slashed
    // branch name travels through the encoded segment.
    let factory_sha = create_commit_raw(
        &http,
        &RepoAddr {
            base: &base,
            owner: &gitea_user(),
            repo: &repo,
        },
        "README.md",
        "phase-2 live test\n",
        "factory",
        None,
    );
    let slfo_sha = create_commit_raw(
        &http,
        &RepoAddr {
            base: &base,
            owner: &gitea_user(),
            repo: &repo,
        },
        "SLFO.md",
        "phase-2 live test\n",
        "factory",
        Some("slfo-main"),
    );
    let feature_sha = create_commit_raw(
        &http,
        &RepoAddr {
            base: &base,
            owner: &gitea_user(),
            repo: &repo,
        },
        "FEATURE.md",
        "phase-2 live test\n",
        "factory",
        Some("feature/x"),
    );
    // The branch list carries all three, with their head commits.
    let branches = client
        .branch_list(&gitea_user(), &repo)
        .expect("typed branch_list lists the branches");
    let mut names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["factory", "feature/x", "slfo-main"]);
    let head_of = |name: &str| {
        branches
            .iter()
            .find(|b| b.name == name)
            .expect("the branch is listed")
            .commit
            .id
            .clone()
    };
    assert_eq!(head_of("factory"), factory_sha);
    assert_eq!(head_of("slfo-main"), slfo_sha);
    assert_eq!(head_of("feature/x"), feature_sha);
    // The single-branch read agrees with the list, including the
    // commit message — and the head is the same SHA the contents
    // API returned on creation, which is the sync-verification
    // comparison the filer will make.
    let factory = client
        .branch_get(&gitea_user(), &repo, "factory")
        .expect("typed branch_get reads the default branch");
    assert_eq!(factory.commit.id, factory_sha);
    // Gitea stores the commit message with a trailing newline, so
    // the comparison trims it — the message itself is the pin.
    assert_eq!(factory.commit.message.trim_end(), "add README.md");
    assert_eq!(
        client
            .branch_head(&gitea_user(), &repo, "factory")
            .expect("typed branch_head reads the default branch head"),
        factory_sha
    );
    let slfo = client
        .branch_get(&gitea_user(), &repo, "slfo-main")
        .expect("typed branch_get reads the slfo branch");
    assert_eq!(slfo.commit.id, slfo_sha);
    assert_eq!(
        client
            .branch_head(&gitea_user(), &repo, "slfo-main")
            .expect("typed branch_head reads the slfo branch head"),
        slfo_sha
    );
    // The slashed branch reads back through the encoded segment.
    let feature = client
        .branch_get(&gitea_user(), &repo, "feature/x")
        .expect("typed branch_get reads the slashed branch");
    assert_eq!(feature.commit.id, feature_sha);
    assert_eq!(
        client
            .branch_head(&gitea_user(), &repo, "feature/x")
            .expect("typed branch_head reads the slashed branch head"),
        feature_sha
    );
    // A missing branch maps to NotFound, and so does a missing repo.
    let err = client
        .branch_get(&gitea_user(), &repo, "leap-16.0")
        .expect_err("a missing branch must not return a value");
    assert_eq!(
        err.kind(),
        ErrorKind::NotFound,
        "a missing branch must map to NotFound; got {err:?}"
    );
    let err = client
        .branch_list(&gitea_user(), "geectl-live-no-such-repo")
        .expect_err("a missing repository must not list branches");
    assert_eq!(
        err.kind(),
        ErrorKind::NotFound,
        "a missing repository must map to NotFound; got {err:?}"
    );
    // Clean up: the created repo is removed again, so the live
    // runs do not grow the token owner's repository list.
    let deleted = delete_live(&client, &gitea_user(), &repo);
    assert_eq!(deleted, repo);
}

#[test]
fn gitea_fork_round_trip() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEECTL_GITEA_URL is not set");
        return;
    };
    let http = gitea_client();
    let client = typed_client(&http, &base);
    let source = unique_repo_name();
    let source_full = create_repo_raw(&http, &base, &source);
    let source_read = client
        .repo_get(&gitea_user(), &source)
        .expect("typed repo_get reads the source repository");
    // A self-fork needs a different name: forking onto the source's
    // own name would collide with the source itself (409).
    let fork_name = unique_repo_name();
    let forked = fork_live(
        &client,
        &gitea_user(),
        &source,
        &ForkRepoOptions {
            new_name: Some(fork_name.clone()),
            ..Default::default()
        },
    );
    assert_eq!(forked.owner.login, gitea_user());
    assert_eq!(forked.name, fork_name);
    assert_eq!(forked.full_name, format!("{}/{fork_name}", gitea_user()));
    // The fork's parent is the source, and the default branch is
    // inherited — the two facts the composite's re-read verifies.
    let parent = forked
        .parent
        .as_ref()
        .expect("the fork must name its parent");
    assert_eq!(parent.full_name, source_full);
    assert_eq!(forked.default_branch, source_read.default_branch);
    // And the fork reads back on the server under its own name.
    let read_back = client
        .repo_get(&gitea_user(), &fork_name)
        .expect("typed repo_get reads the fork back");
    assert_eq!(read_back.full_name, forked.full_name);
}

#[test]
fn gitea_fork_twice_with_the_same_name_reuses_the_fork() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEECTL_GITEA_URL is not set");
        return;
    };
    let http = gitea_client();
    let client = typed_client(&http, &base);
    let source = unique_repo_name();
    let source_full = create_repo_raw(&http, &base, &source);
    let fork_name = unique_repo_name();
    let first = fork_live(
        &client,
        &gitea_user(),
        &source,
        &ForkRepoOptions {
            new_name: Some(fork_name.clone()),
            ..Default::default()
        },
    );
    // The second fork, same name, reuses the existing fork instead
    // of failing: Gitea allows one fork per user per source, and the
    // pre-check finds it with the source as its parent.
    let second = fork_live(
        &client,
        &gitea_user(),
        &source,
        &ForkRepoOptions {
            new_name: Some(fork_name.clone()),
            ..Default::default()
        },
    );
    assert_eq!(second.full_name, first.full_name);
    assert_eq!(
        second
            .parent
            .as_ref()
            .expect("the reused fork has a parent")
            .full_name,
        source_full
    );
}

#[test]
fn gitea_fork_twice_with_a_different_name_reports_the_collision() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEECTL_GITEA_URL is not set");
        return;
    };
    let http = gitea_client();
    let client = typed_client(&http, &base);
    let source = unique_repo_name();
    create_repo_raw(&http, &base, &source);
    let first_name = unique_repo_name();
    client
        .fork_repo(
            &gitea_user(),
            &source,
            &ForkRepoOptions {
                new_name: Some(first_name.clone()),
                ..Default::default()
            },
            &DryRun::new(false),
        )
        .expect("the first fork creates the fork");
    // A second fork under a different name cannot reuse the first:
    // Gitea's 409 names the existing fork's path, but the transport
    // discards the server's error body, and listing the existing
    // forks is a phase-2 operation. So the collision surfaces as the
    // server's 409 (AlreadyExists), and no repo appears under the
    // second name.
    let second_name = unique_repo_name();
    let err = client
        .fork_repo(
            &gitea_user(),
            &source,
            &ForkRepoOptions {
                new_name: Some(second_name.clone()),
                ..Default::default()
            },
            &DryRun::new(false),
        )
        .expect_err("a second fork under a different name must fail");
    assert_eq!(
        err.kind(),
        ErrorKind::AlreadyExists,
        "a renamed re-fork must surface the server's 409; got {err:?}"
    );
    let err = client
        .repo_get(&gitea_user(), &second_name)
        .expect_err("no repository must exist under the second name");
    assert_eq!(
        err.kind(),
        ErrorKind::NotFound,
        "no repository must exist under the second name; got {err:?}"
    );
}

#[test]
fn gitea_fork_of_fork_parent_is_the_direct_parent() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEECTL_GITEA_URL is not set");
        return;
    };
    let http = gitea_client();
    let client = typed_client(&http, &base);
    let source = unique_repo_name();
    let source_full = create_repo_raw(&http, &base, &source);
    let first_name = unique_repo_name();
    let first = fork_live(
        &client,
        &gitea_user(),
        &source,
        &ForkRepoOptions {
            new_name: Some(first_name.clone()),
            ..Default::default()
        },
    );
    assert_eq!(
        first
            .parent
            .as_ref()
            .expect("the first fork has a parent")
            .full_name,
        source_full
    );
    // Forking the fork: the second fork's parent is the first fork
    // (the direct parent), not the root — that is what the
    // composite's re-read check verifies.
    let second_name = unique_repo_name();
    let second = fork_live(
        &client,
        &gitea_user(),
        &first_name,
        &ForkRepoOptions {
            new_name: Some(second_name.clone()),
            ..Default::default()
        },
    );
    assert_eq!(
        second
            .parent
            .as_ref()
            .expect("the second fork has a parent")
            .full_name,
        first.full_name
    );
}

#[test]
fn gitea_fork_name_collision_retries_with_a_new_name() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEECTL_GITEA_URL is not set");
        return;
    };
    let http = gitea_client();
    let client = typed_client(&http, &base);
    // A plain repository already holds the target name — not a fork
    // of the source, so the fork is refused, not reused.
    let blocking = unique_repo_name();
    create_repo_raw(&http, &base, &blocking);
    let source = unique_repo_name();
    create_repo_raw(&http, &base, &source);
    let err = client
        .fork_repo(
            &gitea_user(),
            &source,
            &ForkRepoOptions {
                new_name: Some(blocking.clone()),
                ..Default::default()
            },
            &DryRun::new(false),
        )
        .expect_err("forking onto an existing non-fork name must fail");
    assert_eq!(
        err.kind(),
        ErrorKind::AlreadyExists,
        "forking onto an existing non-fork name must be AlreadyExists; got {err:?}"
    );
    // The retry with a fresh name succeeds.
    let retry = unique_repo_name();
    let forked = fork_live(
        &client,
        &gitea_user(),
        &source,
        &ForkRepoOptions {
            new_name: Some(retry.clone()),
            ..Default::default()
        },
    );
    assert_eq!(forked.name, retry);
}

#[test]
fn gitea_fork_into_org_refuses_when_a_non_fork_exists() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEECTL_GITEA_URL is not set");
        return;
    };
    let http = gitea_client();
    let client = typed_client(&http, &base);
    let org = unique_org_name();
    // Org creation is not a phase-1 operation, so it stays raw.
    // The token needs `write:organization` (provision.sh).
    let org_body = serde_json::json!({ "username": org }).to_string();
    let created = http
        .post_json(&format!("{base}/api/v1/orgs"), &org_body)
        .expect("POST /api/v1/orgs on the Gitea instance");
    let created: serde_json::Value =
        serde_json::from_str(&created).expect("POST /api/v1/orgs returns valid JSON");
    assert_eq!(
        created["username"].as_str(),
        Some(org.as_str()),
        "POST /api/v1/orgs did not return the created organization; body: {created}"
    );
    // A plain repository of the target name already exists in the
    // org — not a fork of the source — so forking into the org is
    // refused, not reused.
    let source = unique_repo_name();
    let org_repo_body = serde_json::json!({
        "name": source,
        "private": false,
    })
    .to_string();
    http.post_json(&format!("{base}/api/v1/orgs/{org}/repos"), &org_repo_body)
        .expect("POST /api/v1/orgs/{org}/repos on the Gitea instance");
    create_repo_raw(&http, &base, &source);
    let err = client
        .fork_repo(
            &gitea_user(),
            &source,
            &ForkRepoOptions {
                organization: Some(org.clone()),
                ..Default::default()
            },
            &DryRun::new(false),
        )
        .expect_err("forking into an org that holds a non-fork of that name must fail");
    assert_eq!(
        err.kind(),
        ErrorKind::AlreadyExists,
        "forking into an org that holds a non-fork of that name must be AlreadyExists; got {err:?}"
    );
}

#[test]
fn gitea_typed_call_emits_a_debug_attempt_line() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEECTL_GITEA_URL is not set");
        return;
    };
    let repo = unique_repo_name();
    // The attempt lines are emitted by the transport, so the same
    // sink goes on the transport and on the typed client (the unit
    // test wires it the same way): the client alone only carries
    // the decision lines.
    let (debug, lines) = Debug::captured(true, false);
    let http = UreqClient::with_auth(Timeout::default(), Some(Box::new(EnvGiteaAuth)))
        .expect("default timeout is valid")
        .with_debug(debug.clone());
    create_repo_raw(&http, &base, &repo);
    let client = typed_client(&http, &base).with_debug(debug);
    client
        .repo_get(&gitea_user(), &repo)
        .expect("typed repo_get with a debug sink reads the repository");
    let lines = lines.lock().expect("the captured debug lines are readable");
    let expected = format!("http: GET /api/v1/repos/{}/{} -> 200", gitea_user(), repo);
    assert!(
        lines.iter().any(|line| line.contains(&expected)),
        "a typed call must emit an attempt line; lines: {lines:?}"
    );
}

/// The mirrored `pool/systemd` fixture must already exist on the
/// live instance: it is populated by `tests/gitea/populate-fixtures.sh`,
/// which CI runs as its own step after provisioning (and the local
/// gate runs before the test step). Fail clearly — naming the
/// populate step — instead of creating the fixture inline, so a
/// missing populate step can never silently pass as a passing
/// mirror test.
fn assert_pool_systemd_mirror(http: &UreqClient, base: &str) {
    match http.get(&format!("{base}/api/v1/repos/pool/systemd")) {
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {
            panic!(
                "pool/systemd is missing on the live Gitea instance: run \
                 tests/gitea/populate-fixtures.sh after provisioning \
                 (CI runs it as its own step; see tests/gitea/README.md)"
            );
        }
        Err(e) => {
            panic!(
                "could not read pool/systemd from the live Gitea instance: {e}; \
                 check the instance is up and the token is valid — the fixture \
                 may exist, so this is not a missing-populate failure"
            );
        }
    }
}

#[test]
fn gitea_pool_systemd_layout_mirrored() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEECTL_GITEA_URL is not set");
        return;
    };
    let http = gitea_client();
    // The real layout, mirrored exactly: org `pool`, repo
    // `systemd`, default branch `factory` — fetched live
    // 2026-11-21 from https://src.opensuse.org (see the layout
    // note in src/gitea.rs) and populated by
    // tests/gitea/populate-fixtures.sh, which runs as its own
    // step after provisioning. The fork owner is the disposable
    // instance's own user, not a real forker: the token owner is
    // always the forker, and the instance has one user. The
    // clone/ssh URLs are the instance's own (the server derives
    // them from its base), so only their tails are asserted.
    assert_pool_systemd_mirror(&http, &base);
    let client = typed_client(&http, &base);
    let source = client
        .repo_get("pool", "systemd")
        .expect("typed repo_get reads pool/systemd");
    assert_eq!(source.full_name, "pool/systemd");
    assert_eq!(source.owner.login, "pool");
    assert!(
        source.parent.is_none(),
        "pool/systemd is not a fork; body: {source:?}"
    );
    assert_eq!(
        source.default_branch, "factory",
        "pool/systemd's default branch is factory; body: {source:?}"
    );
    assert!(
        source.clone_url.ends_with("/pool/systemd.git"),
        "the clone URL names pool/systemd; body: {source:?}"
    );
    assert!(
        source.ssh_url.ends_with("pool/systemd.git"),
        "the SSH URL names pool/systemd; body: {source:?}"
    );
    // Fork it: the fork keeps the source name under the token
    // owner, its parent is pool/systemd, and it inherits the
    // factory default branch — the shapes the composite relies on.
    // A re-run reuses the existing fork (same assertions hold).
    let forked = fork_live(&client, "pool", "systemd", &ForkRepoOptions::default());
    assert_eq!(forked.name, "systemd");
    assert_eq!(forked.owner.login, gitea_user());
    assert_eq!(
        forked
            .parent
            .as_ref()
            .expect("the fork must name its parent")
            .full_name,
        "pool/systemd"
    );
    assert_eq!(forked.default_branch, "factory");
}

#[test]
fn gitea_missing_repo_is_not_found() {
    let Some(base) = gitea_url() else {
        eprintln!("skipping Gitea live test: GEECTL_GITEA_URL is not set");
        return;
    };
    let client = gitea_client();
    // A repository that was never created must map to geectl's
    // NotFound, not to a generic failure: the status→error mapping is
    // exercised against a real server, not a fixture.
    let err = client
        .get(&format!(
            "{base}/api/v1/repos/{}/geectl-live-no-such-repo",
            gitea_user()
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
        // Userinfo must not change the host: the host after the last
        // `@` is what counts.
        assert!(is_loopback_url("http://user:pass@127.0.0.1:3001"));
        assert!(!is_loopback_url("http://user:pass@127.0.0.1.evil.com"));
        assert!(!is_loopback_url("http://user@evil.com"));
        assert!(!is_loopback_url("http://[::1]evil.com"));
    }

    #[test]
    fn unique_repo_name_has_pid_and_timestamp_suffix() {
        let first = unique_repo_name();
        assert!(first.starts_with("geectl-live-"));
        // The suffix is
        // "<process-id>-<nanoseconds-since-epoch>-<counter>": all
        // numeric, so the name is deterministic within a process,
        // and two processes still differ.
        let suffix = &first["geectl-live-".len()..];
        let parts: Vec<&str> = suffix.split('-').collect();
        assert_eq!(parts.len(), 3);
        assert!(parts[0].parse::<u32>().is_ok());
        assert!(parts[1].parse::<u128>().is_ok());
        assert!(parts[2].parse::<u64>().is_ok());
    }

    #[test]
    fn unique_repo_name_differs_between_calls() {
        // The counter: two calls in the same process do not
        // collide even in the same nanosecond, so a re-run against
        // one instance cannot clash with its own previous creation.
        assert_ne!(unique_repo_name(), unique_repo_name());
    }
}
