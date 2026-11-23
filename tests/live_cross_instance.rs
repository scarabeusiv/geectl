//! Live cross-instance copy test: two disposable OBS instances.
//!
//! The proof for `branch copy-package --to-apiurl` (osc `copypac
//! -t`): a package copied from one OBS instance to another, client-
//! side, through geectl's own `ObsClient::copy_package_to_instance`.
//! The assertions are on **server state on the target instance** —
//! the files it holds, their md5s and bytes, and the carried-over
//! meta — which is what catches drift as the server changes
//! underneath us.
//!
//! These tests are NOT part of the per-PR gate. They run only in
//! the scheduled `live` CI job, against the two instances the
//! provisioning in `tests/live/` spins up (the second stack on port
//! 3001 — see `tests/live/README.md`). They are skipped unless BOTH
//! `GEECTL_LIVE_URL` and `GEECTL_LIVE_URL_2` are set, so `cargo
//! test` stays green without any instance.
//!
//! The harness helpers are copied from `tests/live.rs` (as
//! `tests/live_obs.rs` does): one copy per live test target, since
//! integration targets cannot share modules. Their unit tests live
//! in `tests/live.rs` and are not duplicated here — the code is
//! identical, so testing it twice adds no signal — with one
//! exception: the local `validate_live_base` also rejects a
//! non-decimal or out-of-range port, which `live.rs` does not check.
//! This target needs it: `live_instance_key` maps a missing port to
//! 80, and without the check a garbage port would silently key as
//! 80. The same loopback-only, ephemeral-instance,
//! ephemeral-credential rules apply (see `tests/live.rs`): the
//! instances are plain HTTP on localhost, and the bootstrap admin
//! credentials go only to them.

use geectl::auth::Credentials;
use geectl::error::ErrorKind;
use geectl::http::{HttpClient, Timeout, UreqClient};
use geectl::obs::{CopyOptions, DryRun, LinkOptions, ObsClient, SourceFile};

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

/// Validate a live base URL: trim whitespace, strip trailing
/// slashes, require a bare base (no path, query, or fragment), reject
/// userinfo, and require a loopback host. Returns the cleaned URL, or
/// the reason it is invalid. Pure, so it is unit-testable;
/// `live_urls` turns the error into a panic at the boundary.
/// Error messages echo the URL with userinfo stripped, never the raw
/// input: a password in the base must not land in a panic message.
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
    // A `:port`, when present, must be decimal and in range:
    // `http://127.0.0.1:abc` names no server, and the instance key
    // in `live_urls` maps a missing port to 80 — without this check
    // a garbage port would silently key as 80 and the inequality
    // check would blame the wrong URL. (`live.rs` does not check
    // the port; this target needs the check, see the module docs.)
    // The authority is bare (no path/query/fragment) at this point.
    // A bracketed host (`[::1]`) carries its colons inside the
    // brackets, so the port splits off after the closing bracket.
    let port = if let Some(bracketed) = after_scheme.strip_prefix('[') {
        bracketed
            .split_once(']')
            .and_then(|(_, rest)| rest.strip_prefix(':'))
    } else {
        after_scheme.split_once(':').map(|(_, port)| port)
    };
    if let Some(port) = port {
        if port.parse::<u16>().is_err() {
            return Err(format!(
                "port must be a decimal number (0-65535); got {shown}"
            ));
        }
    }
    Ok(url)
}

/// The identity of a live instance for the two-URL inequality check:
/// the normalized loopback host plus the port. `localhost` and `::1`
/// count as `127.0.0.1`, and a missing port is 80: two URLs naming
/// the same instance must compare equal even when spelled
/// differently — `http://127.0.0.1:3000` and
/// `http://localhost:3000` are one instance, not two. A bracketed
/// host (`[::1]`) carries its colons inside the brackets, so the
/// port splits off after the closing bracket, not at the first `:`.
/// The URLs are already validated bare — loopback host, decimal
/// port when present — by `validate_live_base` when this runs, so
/// a missing port is the only reason the fallback to 80 fires.
fn live_instance_key(url: &str) -> (String, u16) {
    let authority = url.strip_prefix("http://").unwrap_or(url);
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        match bracketed.split_once(']') {
            Some((inside, rest)) => (
                inside,
                rest.strip_prefix(':')
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(80),
            ),
            None => (bracketed, 80),
        }
    } else {
        match authority.split_once(':') {
            Some((host, port)) => (host, port.parse().unwrap_or(80)),
            None => (authority, 80),
        }
    };
    let host = match host.to_ascii_lowercase().as_str() {
        "localhost" | "::1" => "127.0.0.1".to_string(),
        other => other.to_string(),
    };
    (host, port)
}

/// The two live instances' base URLs, from the environment.
///
/// Returns `None` (skip the test) when either `GEECTL_LIVE_URL` or
/// `GEECTL_LIVE_URL_2` is unset or empty. When
/// `GEECTL_LIVE_REQUIRED` is set, a missing or empty value panics
/// instead: the live workflow sets that marker and wires both URLs,
/// so an absence means the wiring broke, and a silent skip would go
/// green while hiding drift. A present but invalid value panics in
/// both cases, as in `tests/live.rs`. The two URLs must name
/// different instances (compared by `live_instance_key`, so
/// same-instance aliases count as one): both pointing at the same
/// instance would prove a same-instance copy, not a cross-instance
/// one, and would pass while proving nothing.
fn live_urls() -> Option<(String, String)> {
    let first = match std::env::var("GEECTL_LIVE_URL") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            if std::env::var_os("GEECTL_LIVE_REQUIRED").is_some() {
                panic!(
                    "GEECTL_LIVE_URL is not set or empty: the live workflow must wire it (see .github/workflows/live.yml)"
                );
            }
            eprintln!("skipping live cross-instance test: GEECTL_LIVE_URL is not set");
            return None;
        }
    };
    let second = match std::env::var("GEECTL_LIVE_URL_2") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            if std::env::var_os("GEECTL_LIVE_REQUIRED").is_some() {
                panic!(
                    "GEECTL_LIVE_URL_2 is not set or empty: the live workflow must wire the second stack (see .github/workflows/live.yml)"
                );
            }
            eprintln!("skipping live cross-instance test: GEECTL_LIVE_URL_2 is not set");
            return None;
        }
    };
    match (validate_live_base(&first), validate_live_base(&second)) {
        (Ok(a), Ok(b)) => {
            assert_ne!(
                live_instance_key(&a),
                live_instance_key(&b),
                "GEECTL_LIVE_URL and GEECTL_LIVE_URL_2 must point at different instances"
            );
            Some((a, b))
        }
        (Err(reason), _) => panic!("GEECTL_LIVE_URL {reason}"),
        (_, Err(reason)) => panic!("GEECTL_LIVE_URL_2 {reason}"),
    }
}

/// A client for a live instance. No auth: only used for the
/// no-credential reads below.
fn live_client() -> UreqClient {
    UreqClient::new(Timeout::default()).expect("default timeout is valid")
}

/// The live instances' bootstrap admin credentials (the OBS
/// development defaults, ephemeral, never a real credential).
/// Overridable via `GEECTL_LIVE_OBS_USERNAME` /
/// `GEECTL_LIVE_OBS_PASSWORD` if the provisioning ever changes
/// them. Both stacks seed the same defaults.
fn live_credentials() -> Credentials {
    let username =
        std::env::var("GEECTL_LIVE_OBS_USERNAME").unwrap_or_else(|_| "Admin".to_string());
    let password =
        std::env::var("GEECTL_LIVE_OBS_PASSWORD").unwrap_or_else(|_| "opensuse".to_string());
    Credentials::Basic { username, password }
}

/// A client for a live instance, authenticated with the
/// bootstrap admin credentials.
fn live_client_authed(base: &str) -> ObsClient<'_> {
    let http = Box::leak(Box::new(
        UreqClient::new(Timeout::default()).expect("default timeout is valid"),
    ));
    ObsClient::with_base_and_credentials(http, base, live_credentials())
        .expect("the live base is a loopback http URL")
}

/// A raw GET against a live instance with the bootstrap admin
/// credentials, returning the body as text.
fn live_authed_get(url: &str) -> Result<String, geectl::error::GeectlError> {
    live_client().get_with(url, &live_credentials())
}

/// A raw GET against a live instance with the bootstrap admin
/// credentials, returning the body as bytes. The file read-back
/// must compare bytes, not lossy text: the binary fixture is not
/// valid UTF-8.
fn live_authed_get_bytes(url: &str) -> Result<Vec<u8>, geectl::error::GeectlError> {
    live_client().get_bytes(url, &live_credentials())
}

/// Extract the text of `<tag>...</tag>` from a small XML body. This is a
/// test-harness helper, not a parser: it is enough to assert the server
/// returned the element with non-empty content.
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

/// A unique stamp per run (process id plus a nanosecond timestamp):
/// the live tests run in parallel, and the instances are shared with
/// the `live` and `live_obs` targets' tests. The projects are left
/// behind — the instances are destroyed after the run.
fn unique_stamp() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time is after the epoch")
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

/// A directory listing reduced to sorted `(name, md5)` pairs, for
/// comparing what two instances hold. The md5 is the server's own
/// record of the file content: identical content means an identical
/// md5, so equal pairs mean equal files.
fn files_by_name(files: Vec<SourceFile>) -> Vec<(String, Option<String>)> {
    let mut pairs: Vec<(String, Option<String>)> =
        files.into_iter().map(|f| (f.name, f.md5)).collect();
    pairs.sort();
    pairs
}

/// Copy a package from one live OBS instance to another and assert
/// on the target instance's server state: the files it holds, their
/// md5s and bytes, and the carried-over title and description. Then
/// copy again (everything already identical: nothing copied, nothing
/// uploaded) and copy a link package (expanded: no `_link` file on
/// the target). No `_service` case runs live: services do not execute
/// on the development instance without the service daemon, so there
/// are no `_service:` outputs to skip — that skip is unit-tested
/// against fixtures, not here.
#[test]
fn live_cross_instance_copy() {
    let Some((base_a, base_b)) = live_urls() else {
        return;
    };
    let source = live_client_authed(&base_a);
    let target = live_client_authed(&base_b);
    let live = DryRun::new(false);
    let stamp = unique_stamp();
    let src_project = format!("geectl-xinst-src-{stamp}");
    let tgt_project = format!("geectl-xinst-tgt-{stamp}");
    source
        .create_project(
            &src_project,
            "Cross-instance source",
            "Created by the geectl cross-instance live test; the instances are destroyed after the run.",
            &live,
        )
        .expect("create the source project on instance A");
    target
        .create_project(
            &tgt_project,
            "Cross-instance target",
            "Created by the geectl cross-instance live test; the instances are destroyed after the run.",
            &live,
        )
        .expect("create the target project on instance B");
    source
        .create_package(
            &src_project,
            "foo",
            "Cross-instance source package",
            "The source package for the cross-instance copy proof.",
            &live,
        )
        .expect("create the source package on instance A");
    let spec = "#\n# spec file for the geectl cross-instance copy proof\n#\n\nName:           foo\nVersion:        1\nRelease:        0\nSummary:        Cross-instance copy proof package\nLicense:        MIT\nBuildArch:      noarch\n\n%description\nA package that exists only to prove the cross-instance copy.\n\n%files\n";
    source
        .upload_file(&src_project, "foo", "hello.spec", spec, &live)
        .expect("upload the text file to instance A");
    // NUL plus non-UTF-8 bytes: the copy must move them verbatim,
    // never through a text round-trip.
    let binary: &[u8] = b"PK\x03\x04\x00\xed\xab\xee\xdb\x00PKhello";
    source
        .upload_file_bytes(&src_project, "foo", "data.bin", binary, &live)
        .expect("upload the binary file to instance A");
    // A link package on A, so the second copy proves the link is
    // expanded rather than copied as a link.
    source
        .link_package(
            &src_project,
            "foo",
            &src_project,
            "linkpkg",
            &LinkOptions {
                force: false,
                revision: None,
                vrev: None,
                cicount: None,
                missing_target: false,
                disable_build: false,
                disable_publish: false,
            },
            &live,
        )
        .expect("create the link package on instance A");
    let options = CopyOptions {
        revision: None,
        keep_link: false,
        expand: false,
        comment: None,
        noservice: false,
        dont_update_source: false,
        with_vrev: false,
    };
    let result = source
        .copy_package_to_instance(
            &target,
            &src_project,
            "foo",
            &tgt_project,
            "foo",
            &options,
            &live,
        )
        .expect("copy the package from instance A to instance B");
    assert_eq!(result.copied_files, 2, "both files must be copied");
    assert_eq!(result.skipped_files, 0, "nothing can be skipped yet");
    assert!(
        result.revision.is_some(),
        "the copy must return the target revision"
    );
    // What B holds must be exactly what A holds: same files, same
    // md5s — and nothing else (no `_link`, no `_service:` output).
    let src_files = source
        .list_files(&src_project, "foo", None)
        .expect("list the source package on instance A");
    let tgt_files = target
        .list_files(&tgt_project, "foo", None)
        .expect("list the copied package on instance B");
    let tgt_pairs = files_by_name(tgt_files);
    assert_eq!(
        files_by_name(src_files),
        tgt_pairs,
        "instance B must hold exactly the files instance A holds"
    );
    assert_eq!(
        tgt_pairs
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["data.bin", "hello.spec"],
        "the copied package holds exactly the two files"
    );
    // The bytes themselves, read back from B over the authenticated
    // route: identical to what was uploaded to A.
    for (name, expected) in [("hello.spec", spec.as_bytes()), ("data.bin", binary)] {
        let got = live_authed_get_bytes(&format!("{base_b}/source/{tgt_project}/foo/{name}"))
            .expect("download the copied file from instance B");
        assert_eq!(
            got, expected,
            "the copied file {name} must read back byte-identical"
        );
    }
    // The title and description travel with the package.
    let meta = live_authed_get(&format!("{base_b}/source/{tgt_project}/foo/_meta"))
        .expect("read the copied package meta from instance B");
    assert_eq!(
        xml_text(&meta, "title").as_deref(),
        Some("Cross-instance source package"),
        "the title must carry over; meta: {meta}"
    );
    assert_eq!(
        xml_text(&meta, "description").as_deref(),
        Some("The source package for the cross-instance copy proof."),
        "the description must carry over; meta: {meta}"
    );
    // Copy again: everything is already identical, so nothing is
    // downloaded or uploaded and there is no new revision.
    let again = source
        .copy_package_to_instance(
            &target,
            &src_project,
            "foo",
            &tgt_project,
            "foo",
            &options,
            &live,
        )
        .expect("copy the package a second time");
    assert_eq!(again.copied_files, 0, "nothing to copy the second time");
    assert_eq!(again.skipped_files, 2, "both files skipped as identical");
    assert_eq!(again.revision, None, "no new revision when nothing changed");
    // The link package copies as its resolved files, never as a link:
    // no `_link` file on B, and the same two files with the same md5s.
    let link_result = source
        .copy_package_to_instance(
            &target,
            &src_project,
            "linkpkg",
            &tgt_project,
            "linkcopy",
            &options,
            &live,
        )
        .expect("copy the link package from instance A to instance B");
    assert_eq!(
        link_result.copied_files, 2,
        "the link must expand to two files"
    );
    assert_eq!(link_result.skipped_files, 0);
    let link_files = target
        .list_files(&tgt_project, "linkcopy", None)
        .expect("list the copied link package on instance B");
    let link_pairs = files_by_name(link_files);
    assert_eq!(
        link_pairs
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["data.bin", "hello.spec"],
        "the expanded link copy holds exactly the two resolved files, no _link"
    );
    let src_pairs = source
        .list_files(&src_project, "foo", None)
        .expect("list the source package on instance A");
    assert_eq!(
        files_by_name(src_pairs),
        link_pairs,
        "the expanded link copy must hold the same files with the same md5s"
    );
    for (name, expected) in [("hello.spec", spec.as_bytes()), ("data.bin", binary)] {
        let got = live_authed_get_bytes(&format!("{base_b}/source/{tgt_project}/linkcopy/{name}"))
            .expect("download the expanded link file from instance B");
        assert_eq!(
            got, expected,
            "the expanded link file {name} must read back byte-identical"
        );
    }
    // A missing target package maps to geectl's NotFound, not to a
    // generic failure: the error mapping is exercised against a real
    // server, not a fixture.
    let err = live_client()
        .get(&format!(
            "{base_b}/public/source/{tgt_project}/geectl-xinst-no-such-package"
        ))
        .expect_err("a missing package must not return 200");
    assert_eq!(
        err.kind(),
        ErrorKind::NotFound,
        "a missing package must map to NotFound; got {err:?}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_instance_key_normalizes_loopback_spellings() {
        assert_eq!(
            live_instance_key("http://127.0.0.1:3000"),
            ("127.0.0.1".to_string(), 3000)
        );
        assert_eq!(
            live_instance_key("http://localhost:3000"),
            ("127.0.0.1".to_string(), 3000)
        );
        assert_eq!(
            live_instance_key("http://LOCALHOST:3000"),
            ("127.0.0.1".to_string(), 3000)
        );
        assert_eq!(
            live_instance_key("http://[::1]:3000"),
            ("127.0.0.1".to_string(), 3000)
        );
        // A missing port is 80 — the only case the fallback may
        // fire for, since `validate_live_base` rejects a garbage
        // port before this runs.
        assert_eq!(
            live_instance_key("http://127.0.0.1"),
            ("127.0.0.1".to_string(), 80)
        );
        assert_ne!(
            live_instance_key("http://127.0.0.1:3000"),
            live_instance_key("http://127.0.0.1:3001")
        );
        // A garbage port never reaches the key: it is rejected at
        // the boundary, not silently coerced to 80.
        assert!(validate_live_base("http://127.0.0.1:abc").is_err());
        assert!(validate_live_base("http://127.0.0.1:99999").is_err());
        assert!(validate_live_base("http://127.0.0.1:3000:4000").is_err());
        assert!(validate_live_base("http://127.0.0.1:3000").is_ok());
        assert!(validate_live_base("http://127.0.0.1").is_ok());
    }
}
