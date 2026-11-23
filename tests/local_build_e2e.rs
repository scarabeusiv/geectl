//! End-to-end local build: real engine, real podman, real local OBS.
//!
//! Runs only when `GEECTL_LOCAL_BUILD_E2E` is `1` (the local-build-e2e CI
//! job sets it). Without the marker the test skips, so `cargo test` stays
//! green without podman, the engine, or a local OBS instance.
//!
//! With the marker set, the test builds the fixture package
//! (`tests/local_build_e2e/geectl-e2e-hello.spec`) against the local OBS instance
//! the job provisions: the instance serves the buildinfo, buildconfig,
//! and base-system RPMs for the fixture project through a
//! download-on-demand repository backed by the Tumbleweed OSS
//! repository. The instance's bootstrap admin credentials (`Admin` /
//! `opensuse`, the OBS development defaults) authenticate the client —
//! no real OBS credential is used anywhere. A missing `GEECTL_LIVE_URL`
//! with the marker set fails the test: the job must wire it, so a
//! silent skip would hide a broken fixture.

use geectl::auth::Credentials;
use geectl::debug::Debug;
use geectl::http::{is_loopback_url, sanitize_url_for_error, Timeout, UreqClient};
use geectl::local_build::{BuildRequest, LocalBuild};
use geectl::obs::ObsClient;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Marker enabling the test.
fn enabled() -> bool {
    std::env::var("GEECTL_LOCAL_BUILD_E2E").as_deref() == Ok("1")
}

/// Validate a live-instance base URL: `http://`, bare (no path, query,
/// or fragment), no userinfo, and loopback. The same rule as the live
/// harness, on the same shared primitives — the bootstrap credentials
/// must never be sent to a remote host.
fn validate_live_base(raw: &str) -> Result<String, String> {
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
            "must be an http:// URL (the live instance is plain HTTP on loopback); got {shown}"
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
    // No userinfo in the base: credentials go through the auth
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

/// The local OBS instance's base URL. Panics when the marker is set
/// and the URL is missing or invalid: in the e2e job the URL must be
/// wired, so a silent skip would hide a broken fixture.
fn live_base() -> String {
    match std::env::var("GEECTL_LIVE_URL") {
        Ok(url) if !url.trim().is_empty() => match validate_live_base(&url) {
            Ok(base) => base,
            Err(reason) => panic!("GEECTL_LIVE_URL {reason}"),
        },
        _ => {
            panic!("GEECTL_LIVE_URL is not set or empty: the local-build-e2e workflow must wire it")
        }
    }
}

/// The local instance's bootstrap admin credentials (the OBS
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

/// A temp dir unique per test, removed on drop.
struct TempDir(PathBuf);

static COUNTER: AtomicU64 = AtomicU64::new(0);

impl TempDir {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "geectl-local-build-e2e-test-{}-{n}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Whether `binary` resolves on `PATH`.
fn on_path(binary: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(binary).is_file())
}

#[test]
fn real_podman_build_of_fixture_package_succeeds() {
    if !enabled() {
        eprintln!("skipping local-build e2e test: GEECTL_LOCAL_BUILD_E2E is not set to 1");
        return;
    }
    for binary in ["build", "podman"] {
        assert!(
            on_path(binary),
            "{binary} is not on PATH: the local-build-e2e workflow installs it"
        );
    }
    // The debug channel is on in this test: the run talks to a
    // live backend, and a failure without the channel is
    // undiagnosable — the download-on-demand warm failure in
    // `download_dep` reports only through it.
    let debug = Debug::new(true, false);
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default())
            .expect("default timeout is valid")
            .with_debug(debug.clone()),
    ));
    let base = live_base();
    let client = ObsClient::with_base_and_credentials(http, &base, live_credentials())
        .expect("the live base is a loopback http URL")
        .with_debug(debug.clone());
    let cache = TempDir::new();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/local_build_e2e");
    let request = BuildRequest::new(source, "geectl:e2e", "standard", std::env::consts::ARCH);
    let record = LocalBuild::new(&client, cache.path().to_path_buf())
        .with_debug(debug)
        .build(&request)
        .expect("local build succeeds");
    assert_eq!(record.verdict, "green");
    assert_eq!(record.exit_code, 0);
    assert!(
        record.engine_version.is_some(),
        "engine answered no version; log: {}",
        record.log_path.display()
    );
    assert!(
        record.image_digest.is_some(),
        "podman answered no image digest; log: {}",
        record.log_path.display()
    );
    assert!(
        !record.rpms.is_empty(),
        "build record carries no RPMs; log: {}",
        record.log_path.display()
    );
    for rpm in &record.rpms {
        assert!(rpm.is_file(), "built RPM missing: {}", rpm.display());
        assert!(
            rpm.file_name()
                .is_some_and(|name| name.to_string_lossy().contains("geectl-e2e-hello")),
            "unexpected RPM built: {}",
            rpm.display()
        );
    }
}
