//! Integration tests: the working-copy leaves as a binary.
//!
//! These run the built binary via `cargo test` (using
//! `CARGO_BIN_EXE_geectl`, set by Cargo for integration tests).
//! The live `checkout-pr` test runs against a loopback server that
//! serves both halves of the checkout — the Gitea pull-request
//! read and the git smart-HTTP clone — and enforces the alias
//! token on both, so it pins that `--instance` authenticates the
//! clone as the same alias `pr_get` uses.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Counter for temp dir names: the pid plus a nanosecond timestamp
/// alone collide when two tests run in the same nanosecond, so each
/// call also takes a distinct counter value.
static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temp dir under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "geectl-cli-working-copy-test-{}-{nanos}-{n}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The geectl binary with an isolated environment: HOME and
/// XDG_CONFIG_HOME point at the temp dir (so no real config is read
/// or written), and every GEECTL_* variable is removed (so the
/// developer's own environment cannot leak into the test).
fn geectl_isolated(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_geectl"));
    cmd.env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir)
        .env_remove("GEECTL_OBS_URL")
        .env_remove("GEECTL_OBS_USERNAME")
        .env_remove("GEECTL_OBS_PASSWORD")
        .env_remove("GEECTL_OBS_PASSWORD_FILE")
        .env_remove("GEECTL_GITEA_URL")
        .env_remove("GEECTL_GITEA_TOKEN")
        .env_remove("GEECTL_GITEA_TOKEN_FILE")
        .env_remove("GEECTL_GITEA_SSH_KEY")
        .env_remove("GEECTL_DEBUG");
    cmd
}

/// Run git in `dir`, failing the test on a non-zero exit.
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Run git in `dir` and return its trimmed stdout.
fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write_file(dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).expect("write fixture file");
}

/// A bare origin repository with a `main` branch holding one
/// commit and a `feature` branch holding one more, at
/// `dir/bob/foo.git` — the head repo the pull-request JSON names,
/// stored with the `.git` suffix the way Gitea stores it.
/// Served over smart HTTP by [`PrOriginServer`].
struct PrOrigin {
    head: String,
}

fn git_pr_origin(dir: &Path) -> PrOrigin {
    let repo = dir.join("bob").join("foo.git");
    git(dir, &["init", "--bare", "-b", "main", "bob/foo.git"]);
    git(&repo, &["config", "http.receivepack", "true"]);
    let work = dir.join("work-origin");
    git(dir, &["clone", "--quiet", "bob/foo.git", "work-origin"]);
    git(&work, &["config", "--local", "user.name", "geectl"]);
    git(
        &work,
        &["config", "--local", "user.email", "geectl@localhost"],
    );
    write_file(&work, "foo.spec", "Name: foo\n");
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "--quiet", "-m", "initial"]);
    git(&work, &["push", "--quiet", "origin", "main"]);
    git(&work, &["checkout", "-b", "feature"]);
    write_file(&work, "feature.txt", "feature\n");
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "--quiet", "-m", "feature"]);
    git(&work, &["push", "--quiet", "origin", "feature"]);
    let head = git_out(&work, &["rev-parse", "HEAD"]);
    PrOrigin { head }
}

/// The Gitea pull-request JSON for `pr_get`: head repo `bob/foo`
/// at branch `feature`, base repo `alice/foo`.
fn pr_json(head_sha: &str) -> String {
    format!(
        r#"{{"id": 7, "number": 7, "user": {{"login": "alice"}}, "title": "trivial fix", "body": null, "labels": [], "state": "open", "draft": false, "merged": false, "mergeable": true, "head": {{"label": "bob:feature", "ref": "feature", "sha": "{head_sha}", "repo": {{"full_name": "bob/foo"}}}}, "base": {{"label": "alice:main", "ref": "main", "sha": "{head_sha}", "repo": {{"full_name": "alice/foo"}}}}, "future_field": "tolerated"}}"#
    )
}

/// One observed request: its path and the `Authorization` header
/// it carried, or `"<missing>"` when it carried none.
type AuthRecord = (String, String);

/// A `git http-backend` server rooted at `root`, serving smart
/// HTTP on 127.0.0.1 — plus the one Gitea route `checkout-pr`
/// calls before cloning. Every request must carry the alias token:
/// `Bearer <token>` on the API route (the Gitea client) and
/// `token <token>` on the git routes (the clone's extra header).
/// Anything else gets a 401, and every header seen is recorded,
/// so the test proves both halves authenticated as the same alias.
/// Each request spawns `git http-backend` with the CGI environment
/// (all request headers as `HTTP_*`, so `Git-Protocol: version=2`
/// reaches it), and the response is written back with
/// `Connection: close`.
struct PrOriginServer {
    port: u16,
    shutdown: Arc<AtomicBool>,
    records: Arc<Mutex<Vec<AuthRecord>>>,
}

impl PrOriginServer {
    fn new(root: PathBuf, pr_body: String, token: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the origin server");
        let port = listener.local_addr().expect("origin address").port();
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = shutdown.clone();
        let records = Arc::new(Mutex::new(Vec::new()));
        let seen = records.clone();
        // `git --exec-path` locates `git-http-backend` for this git
        // installation.
        let exec_path = Command::new("git")
            .arg("--exec-path")
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .unwrap_or_default();
        let backend = PathBuf::from(exec_path).join("git-http-backend");
        std::thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let root = root.clone();
                        let backend = backend.clone();
                        let pr_body = pr_body.clone();
                        let token = token.clone();
                        let seen = seen.clone();
                        std::thread::spawn(move || {
                            serve_origin(stream, &root, &backend, &pr_body, &token, &seen);
                        });
                    }
                    Err(_) => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                }
            }
        });
        PrOriginServer {
            port,
            shutdown,
            records,
        }
    }

    fn records(&self) -> Vec<AuthRecord> {
        self.records.lock().expect("read auth records").clone()
    }
}

impl Drop for PrOriginServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // Wake the accept loop so the thread exits.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn serve_origin(
    stream: TcpStream,
    root: &Path,
    backend: &Path,
    pr_body: &str,
    token: &str,
    records: &Arc<Mutex<Vec<AuthRecord>>>,
) {
    use std::io::BufReader;
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(60)));
    // One handle for the whole connection: the reader is turned
    // back into the stream for the response, so no second file
    // descriptor (and no failing clone) per request.
    let mut reader = BufReader::new(stream);
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while head.len() < 65536 {
        match reader.read(&mut byte) {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
        }
    }
    let head_text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = head_text.lines();
    let Some(request_line) = lines.next() else {
        return;
    };
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_string(), query.to_string()),
        None => (target.clone(), String::new()),
    };
    let mut content_length = 0usize;
    let mut content_type = String::new();
    let mut authorization = "<missing>".to_string();
    let mut header_env: Vec<(String, String)> = Vec::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim().to_string();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.parse().unwrap_or(0);
        } else if name.eq_ignore_ascii_case("content-type") {
            content_type = value.clone();
        } else if name.eq_ignore_ascii_case("authorization") {
            authorization = value.clone();
        }
        let env_name = format!("HTTP_{}", name.to_ascii_uppercase().replace('-', "_"));
        header_env.push((env_name, value));
    }
    let mut body = vec![0u8; content_length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }
    // The request is fully read: the reader turns back into the
    // stream for the response.
    let mut stream = reader.into_inner();
    records
        .lock()
        .expect("record the auth header")
        .push((path.clone(), authorization.clone()));

    // The pull-request read: the Gitea client authenticates with a
    // Bearer token.
    if path == "/api/v1/repos/alice/foo/pulls/7" && method == "GET" {
        if authorization == format!("Bearer {token}") {
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                pr_body.len(),
                pr_body
            );
            let _ = stream.write_all(response.as_bytes());
        } else {
            let _ = stream.write_all(
                b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
        return;
    }
    // The clone: git sends the token as an extra header.
    if path.starts_with("/bob/foo.git/") {
        if authorization != format!("token {token}") {
            let _ = stream.write_all(
                b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            return;
        }
        serve_http_backend(
            stream,
            root,
            backend,
            &method,
            &path,
            &query,
            content_length,
            &content_type,
            body,
            header_env,
        );
        return;
    }
    let _ = stream
        .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
}

#[allow(clippy::too_many_arguments)]
fn serve_http_backend(
    mut stream: TcpStream,
    root: &Path,
    backend: &Path,
    method: &str,
    path: &str,
    query: &str,
    content_length: usize,
    content_type: &str,
    body: Vec<u8>,
    header_env: Vec<(String, String)>,
) {
    let mut cmd = Command::new(backend);
    cmd.env("GIT_PROJECT_ROOT", root);
    cmd.env("GIT_HTTP_EXPORT_ALL", "1");
    cmd.env("PATH_INFO", path);
    cmd.env("QUERY_STRING", query);
    cmd.env("REQUEST_METHOD", method);
    cmd.env("REMOTE_ADDR", "127.0.0.1");
    if !content_type.is_empty() {
        cmd.env("CONTENT_TYPE", content_type);
    }
    cmd.env("CONTENT_LENGTH", content_length.to_string());
    for (name, value) in header_env {
        cmd.env(name, value);
    }
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::null());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(_) => {
            let _ = stream.write_all(
                b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            return;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&body);
    }
    let output = child.wait_with_output();
    let Ok(output) = output else {
        let _ = stream.write_all(
            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return;
    };
    let stdout = output.stdout;
    let (headers, body) = match find_subsequence(&stdout, b"\r\n\r\n") {
        Some(pos) => (&stdout[..pos], &stdout[pos + 4..]),
        None => match find_subsequence(&stdout, b"\n\n") {
            Some(pos) => (&stdout[..pos], &stdout[pos + 2..]),
            None => (&stdout[..], &stdout[stdout.len()..]),
        },
    };
    let headers_text = String::from_utf8_lossy(headers).into_owned();
    let mut status = "200 OK".to_string();
    let mut header_lines = Vec::new();
    for line in headers_text.lines() {
        if let Some(value) = line.strip_prefix("Status:") {
            status = value.trim().to_string();
        } else if !line.trim().is_empty() {
            header_lines.push(line.to_string());
        }
    }
    let mut response = format!("HTTP/1.1 {status}\r\n");
    for line in header_lines {
        response.push_str(&line);
        response.push_str("\r\n");
    }
    response.push_str(&format!("Content-Length: {}\r\n", body.len()));
    response.push_str("Connection: close\r\n\r\n");
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.write_all(body);
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// The alias token: the only Gitea credential in the test
/// environment. The plain `GEECTL_GITEA_TOKEN` is removed, so a
/// clone that borrowed it — or ran anonymous — is visible: the
/// server 401s it, and the leaf fails.
const ALIAS_TOKEN: &str = "alias-token-foo";

#[test]
fn checkout_pr_with_instance_authenticates_the_clone_as_the_alias() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let work = dir.0.join("work");
    fs::create_dir_all(home.join("geectl")).expect("create config dir");
    fs::create_dir_all(&work).expect("create checkout dir");
    // The alias maps to a dummy OBS URL: `checkout-pr` makes no
    // OBS call, the client only names the instance the record is
    // associated with.
    fs::write(
        home.join("geectl").join("config.toml"),
        "[instances]\nfoo = \"https://api.example.org\"\n",
    )
    .expect("write config");
    let origin = git_pr_origin(&dir.0);
    let server = PrOriginServer::new(
        dir.0.clone(),
        pr_json(&origin.head),
        ALIAS_TOKEN.to_string(),
    );
    let base = format!("http://127.0.0.1:{}", server.port);

    let out = geectl_isolated(&home)
        .env("GEECTL_GITEA_URL", &base)
        .env("GEECTL_GITEA_TOKEN_FOO", ALIAS_TOKEN)
        .current_dir(&work)
        .args([
            "--live",
            "--instance",
            "foo",
            "checkout-pr",
            "gitea:alice/foo#7",
        ])
        .output()
        .expect("run geectl");
    assert!(
        out.status.success(),
        "checkout-pr failed: stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // The default format is JSON: the payload carries the
    // checkout record.
    let payload: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("parse stdout payload");
    assert_eq!(payload["reference"], "gitea:alice/foo#7");
    assert_eq!(payload["dry_run"], false);
    assert_eq!(payload["pr_head_repo"], "bob/foo");
    assert_eq!(payload["pr_head_branch"], "feature");
    // The head branch, not the shared `main` tip: `feature.txt`
    // exists only on it.
    let checkout = work.join("alice").join("foo");
    assert_eq!(
        fs::read_to_string(checkout.join("feature.txt")).expect("read feature.txt"),
        "feature\n"
    );
    assert_eq!(
        fs::read_to_string(checkout.join("foo.spec")).expect("read foo.spec"),
        "Name: foo\n"
    );

    // Both halves authenticated as the alias: `pr_get` with the
    // Bearer token, the clone with the token extra header. A clone
    // that ran anonymous (or borrowed the plain token) would have
    // been 401d by the server, and the leaf would have failed
    // above — the records pin which token each half actually sent.
    let records = server.records();
    let api: Vec<&AuthRecord> = records
        .iter()
        .filter(|(path, _)| path.starts_with("/api/"))
        .collect();
    assert_eq!(api.len(), 1, "api requests: {records:?}");
    assert_eq!(
        api[0].1,
        format!("Bearer {ALIAS_TOKEN}"),
        "api auth: {records:?}"
    );
    let git: Vec<&AuthRecord> = records
        .iter()
        .filter(|(path, _)| path.starts_with("/bob/foo.git/"))
        .collect();
    assert!(
        !git.is_empty(),
        "the clone made no git request: {records:?}"
    );
    assert!(
        git.iter()
            .all(|(_, header)| header == &format!("token {ALIAS_TOKEN}")),
        "git auth: {git:?}"
    );
}

/// Under `--instance`, an existing leaf resolves the Gitea token
/// strictly from the flagged alias's variables: a plain token set
/// in the environment is never borrowed. `checkout --live` reaches
/// `gitea_auth` before any network, so the refusal names the alias
/// variables with no server behind it.
#[test]
fn checkout_with_instance_needs_the_alias_token() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let work = dir.0.join("work");
    fs::create_dir_all(home.join("geectl")).expect("create config dir");
    fs::create_dir_all(&work).expect("create checkout dir");
    fs::write(
        home.join("geectl").join("config.toml"),
        "[instances]\nfoo = \"https://api.example.org\"\n",
    )
    .expect("write config");
    let out = geectl_isolated(&home)
        .env("GEECTL_GITEA_TOKEN", "plain-token")
        .env_remove("GEECTL_GITEA_TOKEN_FOO")
        .env_remove("GEECTL_GITEA_TOKEN_FILE_FOO")
        .current_dir(&work)
        .args([
            "--live",
            "--instance",
            "foo",
            "checkout",
            "home:alice",
            "foo",
        ])
        .output()
        .expect("run geectl");
    assert!(
        !out.status.success(),
        "checkout succeeded without an alias token"
    );
    assert_eq!(
        out.status.code(),
        Some(9),
        "exit code: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("no Gitea token configured for instance"),
        "output: {combined}"
    );
    assert!(
        combined.contains("GEECTL_GITEA_TOKEN_FOO"),
        "output: {combined}"
    );
}
