//! Live smoketest for the container service runs: every service
//! of the acceptance list runs for real in the podman build
//! vertical's container, with real side effects.
//!
//! These tests are NOT part of the per-PR gate. They run only when
//! `GEECTL_LOCAL_BUILD_LIVE=1` is set, inside the runner container
//! (the `geectl-gate` image plus `podman`, `openssh`, and `git`,
//! with `CONTAINER_HOST` pointing at the podman machine and the
//! worktree mounted at its host path). They are skipped otherwise,
//! so `cargo test` stays green without a container runtime.
//!
//! Fixture directories live under `target/live-fixtures`: the
//! target directory is a host volume in the runner, so the nested
//! service container (started by the host podman) mounts the same
//! host paths the test process sees. A fixture directory anywhere
//! else would not exist for the nested container.
//!
//! The `file://` URLs in the scm fixtures point at `/work/...`:
//! the nested container sees the source directory as `/work`, so
//! only container paths resolve there. The `download_url` fixtures
//! use a tiny HTTP server started by the test itself, reached at
//! the runner container's bridge IP: the nested container shares
//! the podman machine's network with the runner.

use geectl::auth::Credentials;
use geectl::debug::Debug;
use geectl::error::ErrorKind;
use geectl::http::{Timeout, UreqClient};
use geectl::local_build::{LocalBuild, ServiceRunMode, ServiceRunRequest};
use geectl::obs::{ObsClient, OBS_API_BASE};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Whether the live tests run: `GEECTL_LOCAL_BUILD_LIVE=1`.
/// Otherwise the test prints a skip line and returns. When
/// `GEECTL_LOCAL_BUILD_LIVE_REQUIRED` is set, an unset value
/// panics instead: the live runner sets that marker, so its
/// absence means the wiring broke, and a silent skip would go
/// green while hiding drift.
fn live_enabled() -> bool {
    match std::env::var("GEECTL_LOCAL_BUILD_LIVE") {
        Ok(v) if v == "1" => true,
        _ => {
            if std::env::var_os("GEECTL_LOCAL_BUILD_LIVE_REQUIRED").is_some() {
                panic!("GEECTL_LOCAL_BUILD_LIVE is not set to 1: the live runner must wire it");
            }
            eprintln!("skipping live local-build test: GEECTL_LOCAL_BUILD_LIVE is not 1");
            false
        }
    }
}

fn fixture_root() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("live-fixtures");
    fs::create_dir_all(&dir).expect("create the live fixture root");
    dir
}

/// The shared image cache: one directory for all live tests, so
/// the image is pulled once per run, not once per test.
fn live_cache_dir() -> PathBuf {
    let dir = fixture_root().join("cache");
    fs::create_dir_all(&dir).expect("create the live cache dir");
    dir
}

static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fixture source directory, removed on drop.
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let n = FIXTURE_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = fixture_root().join(format!("{name}-{}-{n}", std::process::id()));
        fs::create_dir_all(&dir).expect("create the fixture dir");
        Fixture { dir }
    }

    fn src(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Run `f` with a vertical on the shared cache. The client makes
/// no server calls, so it carries no credentials.
fn with_vertical<R>(f: impl FnOnce(&LocalBuild) -> R) -> R {
    let http = UreqClient::new(Timeout::default()).expect("default timeout is valid");
    let client = ObsClient::with_base_and_credentials(&http, OBS_API_BASE, Credentials::None)
        .expect("a client without credentials builds");
    let vertical = LocalBuild::new(&client, live_cache_dir()).with_debug(Debug::disabled());
    f(&vertical)
}

/// Run a command, failing the test with its stderr when it fails.
fn run(cmd: &mut Command) {
    let output = cmd
        .output()
        .unwrap_or_else(|e| panic!("cannot run {cmd:?}: {e}"));
    assert!(
        output.status.success(),
        "command {cmd:?} failed with exit {}: {}",
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git(dir: &Path, args: &[&str]) {
    run(Command::new("git").args(args).current_dir(dir));
}

fn init_repo(dir: &Path) {
    fs::create_dir_all(dir).expect("create the repo dir");
    git(dir, &["init", "-b", "main"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "user.name", "Test"]);
    fs::write(dir.join("file.txt"), "version one\n").expect("write the repo file");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "first"]);
}

fn git_commit(dir: &Path, content: &str) {
    fs::write(dir.join("file.txt"), content).expect("write the repo file");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "second"]);
}

fn git_head(dir: &Path) -> String {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .expect("git rev-parse");
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// The runner container's bridge IP, from the route to the
/// outside: a UDP connect transmits nothing, it only resolves the
/// source address. The nested service container reaches the file
/// server at this IP.
fn runner_ip() -> std::net::IpAddr {
    let socket = UdpSocket::bind("0.0.0.0:0").expect("bind a udp socket");
    socket.connect("8.8.8.8:80").expect("route to the outside");
    socket.local_addr().expect("local address").ip()
}

/// A tiny HTTP server for the live fixtures: serves a
/// path-to-bytes map, or a directory on disk read per request,
/// 404 otherwise. One thread accepts, one thread per connection
/// serves.
struct FileServer {
    port: u16,
    shutdown: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// What a `FileServer` serves: an in-memory path-to-bytes map,
/// or a directory on disk, read per request so later writes are
/// visible to later requests.
enum FileSource {
    Map(Arc<HashMap<String, Vec<u8>>>),
    Dir(PathBuf),
}

impl FileSource {
    fn lookup(&self, path: &str) -> Option<Vec<u8>> {
        match self {
            FileSource::Map(files) => files.get(path).cloned(),
            FileSource::Dir(root) => {
                if path.split('/').any(|part| part == "..") {
                    return None;
                }
                let file = root.join(path);
                if !file.is_file() {
                    return None;
                }
                fs::read(file).ok()
            }
        }
    }
}

fn serve_forever(
    listener: TcpListener,
    source: FileSource,
) -> (Arc<AtomicBool>, std::thread::JoinHandle<()>) {
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_loop = shutdown.clone();
    let source = Arc::new(source);
    let handle = std::thread::spawn(move || {
        while !shutdown_loop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let source = source.clone();
                    std::thread::spawn(move || serve(stream, &source));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    });
    (shutdown, handle)
}

impl FileServer {
    fn start(files: &[(&str, &str)]) -> Self {
        let listener = TcpListener::bind("0.0.0.0:0").expect("bind the file server");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let port = listener.local_addr().expect("server address").port();
        let source = FileSource::Map(Arc::new(
            files
                .iter()
                .map(|(path, body)| (path.to_string(), body.as_bytes().to_vec()))
                .collect(),
        ));
        let (shutdown, handle) = serve_forever(listener, source);
        FileServer {
            port,
            shutdown,
            handle: Some(handle),
        }
    }

    /// Serve a directory on disk, read per request: a bare git
    /// repository served this way sees later commits, so a clone
    /// after a new commit fetches it.
    fn start_dir(root: &Path) -> Self {
        let listener = TcpListener::bind("0.0.0.0:0").expect("bind the file server");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let port = listener.local_addr().expect("server address").port();
        let (shutdown, handle) = serve_forever(listener, FileSource::Dir(root.to_path_buf()));
        FileServer {
            port,
            shutdown,
            handle: Some(handle),
        }
    }

    /// `host:port` for a `download_url` host parameter.
    fn host_port(&self) -> String {
        format!("{}:{}", runner_ip(), self.port)
    }
}

impl Drop for FileServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve(mut stream: std::net::TcpStream, source: &FileSource) {
    let mut buf = [0u8; 8192];
    let mut request = Vec::new();
    loop {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                request.extend_from_slice(&buf[..n]);
                if request.windows(4).any(|w| w == b"\r\n\r\n") || request.len() > 8192 {
                    break;
                }
            }
        }
    }
    let text = String::from_utf8_lossy(&request);
    let target = text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let path = target
        .trim_start_matches('/')
        .split('?')
        .next()
        .unwrap_or("");
    match source.lookup(path) {
        Some(bytes) => {
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                    bytes.len()
                )
                .as_bytes(),
            );
            let _ = stream.write_all(&bytes);
        }
        None => {
            let _ = stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    }
    let _ = stream.flush();
}

fn prefixed_outputs(src: &Path, service: &str, needle: &str) -> Vec<String> {
    let prefix = format!("_service:{service}:");
    fs::read_dir(src)
        .expect("list the source dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(&prefix) && name.contains(needle))
        .collect()
}

/// The `_servicedata` file, plain or prefixed: `obs_scm` writes it
/// to its outdir, and placement decides the name.
fn read_servicedata(src: &Path) -> Option<String> {
    for name in ["_servicedata", "_service:obs_scm:_servicedata"] {
        if let Ok(body) = fs::read_to_string(src.join(name)) {
            return Some(body);
        }
    }
    None
}

fn spec_with(version: &str, source: &str) -> String {
    format!(
        "Name:           foo\nVersion:        {version}\nRelease:        0\nSummary:        Foo\nLicense:        MIT\nSource0:        {source}\nBuildArch:      noarch\n\n%description\nFoo.\n"
    )
}

#[test]
fn live_mode_filter_with_download_url() {
    if !live_enabled() {
        return;
    }
    let server = FileServer::start(&[
        ("file1.txt", "one\n"),
        ("file2.txt", "two\n"),
        ("file3.txt", "three\n"),
        ("file4.txt", "four\n"),
        ("file5.txt", "five\n"),
        ("file6.txt", "six\n"),
        ("file7.txt", "seven\n"),
    ]);
    let host = server.host_port();
    // One `download_url` service per stored mode, each fetching a
    // distinct file: the mode filter decides which files appear.
    let modes = [
        ("", "file1.txt"),
        ("manual", "file2.txt"),
        ("disabled", "file3.txt"),
        ("trylocal", "file4.txt"),
        ("localonly", "file5.txt"),
        ("serveronly", "file6.txt"),
        ("buildtime", "file7.txt"),
    ];
    let xml = {
        let mut xml = String::from("<services>");
        for (mode, file) in &modes {
            if mode.is_empty() {
                xml.push_str("<service name=\"download_url\">");
            } else {
                xml.push_str(&format!("<service name=\"download_url\" mode=\"{mode}\">"));
            }
            xml.push_str("<param name=\"protocol\">http</param>");
            xml.push_str(&format!("<param name=\"host\">{host}</param>"));
            xml.push_str(&format!("<param name=\"path\">/{file}</param>"));
            xml.push_str("</service>");
        }
        xml.push_str("</services>");
        xml
    };
    // (run mode, expected files with their placement: plain or
    // prefixed `_service:download_url:`).
    type ModeCase<'a> = (&'a str, ServiceRunMode, &'a [(&'a str, bool)]);
    let cases: &[ModeCase] = &[
        (
            "run",
            ServiceRunMode::Run,
            &[
                ("file1.txt", false),
                ("file4.txt", true),
                ("file5.txt", true),
            ],
        ),
        (
            "manualrun",
            ServiceRunMode::ManualRun,
            &[("file2.txt", true)],
        ),
        (
            "disabledrun",
            ServiceRunMode::DisabledRun,
            &[("file3.txt", true)],
        ),
        (
            "precommit",
            ServiceRunMode::Precommit,
            &[("file4.txt", true), ("file5.txt", true)],
        ),
        (
            "localrun",
            ServiceRunMode::LocalRun,
            &[
                ("file1.txt", true),
                ("file4.txt", true),
                ("file5.txt", true),
                ("file6.txt", true),
            ],
        ),
    ];
    for (label, mode, expected) in cases {
        let fixture = Fixture::new("mode-filter");
        fs::write(fixture.src().join("_service"), &xml).expect("write _service");
        with_vertical(|vertical| {
            let req = ServiceRunRequest::new(fixture.src().to_path_buf(), *mode);
            let record = vertical
                .service_run(&req)
                .unwrap_or_else(|e| panic!("{label} service run failed: {e:?}"));
            assert_eq!(
                record.ran.len(),
                expected.len(),
                "{label}: ran {:?}",
                record.ran.iter().map(|r| &r.name).collect::<Vec<_>>()
            );
        });
        for (file, plain) in *expected {
            let path = if *plain {
                fixture.src().join(file)
            } else {
                fixture.src().join(format!("_service:download_url:{file}"))
            };
            assert!(path.exists(), "{label}: {file} (plain={plain}) missing");
        }
        for n in 1..=7 {
            let file = format!("file{n}.txt");
            if expected.iter().any(|(f, _)| *f == file) {
                continue;
            }
            assert!(
                !fixture.src().join(&file).exists(),
                "{label}: {file} must not appear plain"
            );
            assert!(
                !fixture
                    .src()
                    .join(format!("_service:download_url:{file}"))
                    .exists(),
                "{label}: {file} must not appear prefixed"
            );
        }
    }
}

#[test]
fn live_tar_scm_obs_scm_and_servicedata_advances() {
    if !live_enabled() {
        return;
    }
    let fixture = Fixture::new("scm");
    // The upstream is a bare repository served over dumb HTTP:
    // tar_scm refuses file:// URLs, and the directory server
    // reads per request, so the second commit is visible to the
    // re-run. The changesgenerate parameter is covered here too:
    // obs_scm writes the generated .changes file and
    // _servicedata; there is no standalone changesgenerate
    // service binary in the distribution.
    let work = fixture.src().join("upstream-work");
    init_repo(&work);
    // obs_scm finds the base revision for the generated .changes
    // file with `git log --skip=10`: fewer than eleven commits
    // leave it empty, and the entry lists no commits.
    for i in 2..=11 {
        fs::write(work.join("file.txt"), format!("version {i}\n")).expect("write the repo file");
        let message = format!("commit {i}");
        git(&work, &["add", "."]);
        git(&work, &["commit", "-m", &message]);
    }
    // The bare upstream lives outside the source directory: the
    // server serves its own directory, not the source directory.
    // With OSC_VERSION set, the scm services clone into the source
    // directory, and a directory there with the repository's name
    // is reused as the clone — a bare repository there fails the
    // run.
    let server_fixture = Fixture::new("scm-server");
    let upstream = server_fixture.src().join("upstream");
    git(
        fixture.src(),
        &[
            "clone",
            "--bare",
            work.to_str().expect("work path is utf-8"),
            upstream.to_str().expect("upstream path is utf-8"),
        ],
    );
    git(&upstream, &["update-server-info"]);
    let head1 = git_head(&upstream);
    let server = FileServer::start_dir(server_fixture.src());
    let url = format!("http://{}/upstream", server.host_port());
    fs::write(
        fixture.src().join("foo.changes"),
        "Sun Nov 22 2026 - Test <test@example.com>\n\n- Initial version\n\n-------------------------------------------------------------------\n",
    )
    .expect("write the changes file");
    fs::write(
        fixture.src().join("_service"),
        format!(
            "<services><service name=\"tar_scm\"><param name=\"url\">{url}</param><param name=\"scm\">git</param><param name=\"revision\">main</param></service>\
            <service name=\"obs_scm\"><param name=\"url\">{url}</param><param name=\"scm\">git</param><param name=\"revision\">main</param><param name=\"changesgenerate\">enable</param><param name=\"changesauthor\">Test &lt;test@example.com&gt;</param></service></services>"
        ),
    )
    .expect("write _service");
    with_vertical(|vertical| {
        let req = ServiceRunRequest::new(fixture.src().to_path_buf(), ServiceRunMode::Run);
        let record = vertical
            .service_run(&req)
            .unwrap_or_else(|e| panic!("scm service run failed: {e:?}"));
        assert_eq!(record.ran.len(), 2);
    });
    assert!(
        !prefixed_outputs(fixture.src(), "tar_scm", ".tar").is_empty(),
        "tar_scm produced no tarball: {:?}",
        fs::read_dir(fixture.src())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect::<Vec<_>>()
    );
    assert!(
        !prefixed_outputs(fixture.src(), "obs_scm", ".obscpio").is_empty(),
        "obs_scm produced no cpio archive"
    );
    let changes = fs::read_to_string(fixture.src().join("_service:obs_scm:foo.changes"))
        .expect("obs_scm writes the generated .changes file");
    assert!(
        changes.contains("  * commit 11"),
        "the generated .changes file names the latest commit: {changes}"
    );
    assert!(
        changes.contains("- Initial version"),
        "the generated .changes file keeps the old entry: {changes}"
    );
    let data1 = read_servicedata(fixture.src()).expect("obs_scm writes _servicedata");
    assert!(
        data1.contains(&head1),
        "_servicedata names the first commit {head1}: {data1}"
    );
    // A new upstream commit advances `_servicedata` on re-run.
    git_commit(&work, "version two\n");
    git(
        &work,
        &[
            "push",
            upstream.to_str().expect("upstream path is utf-8"),
            "main",
        ],
    );
    git(&upstream, &["update-server-info"]);
    let head2 = git_head(&upstream);
    assert_ne!(head1, head2);
    with_vertical(|vertical| {
        let mut req = ServiceRunRequest::new(fixture.src().to_path_buf(), ServiceRunMode::Run);
        req.service = Some("obs_scm".to_string());
        let record = vertical
            .service_run(&req)
            .unwrap_or_else(|e| panic!("obs_scm re-run failed: {e:?}"));
        assert_eq!(record.ran.len(), 1);
    });
    let data2 = read_servicedata(fixture.src()).expect("obs_scm writes _servicedata");
    assert!(
        data2.contains(&head2),
        "_servicedata names the second commit {head2}: {data2}"
    );
    assert_ne!(data1, data2, "_servicedata did not advance");
}

#[test]
fn live_set_version_recompress() {
    if !live_enabled() {
        return;
    }
    // set_version: the spec's Version follows the tarball name.
    let fixture = Fixture::new("set-version");
    fs::write(
        fixture.src().join("foo.spec"),
        spec_with("0.1", "foo-1.2.3.tar.gz"),
    )
    .expect("write the spec");
    run(Command::new("tar")
        .args(["-czf", "foo-1.2.3.tar.gz", "foo.spec"])
        .current_dir(fixture.src()));
    fs::write(
        fixture.src().join("_service"),
        "<services><service name=\"set_version\"/></services>",
    )
    .expect("write _service");
    with_vertical(|vertical| {
        let req = ServiceRunRequest::new(fixture.src().to_path_buf(), ServiceRunMode::Run);
        vertical
            .service_run(&req)
            .unwrap_or_else(|e| panic!("set_version run failed: {e:?}"));
    });
    // set_version writes the updated spec to its outdir: under `run`
    // the output lands prefixed, the source spec is untouched.
    let spec = fs::read_to_string(fixture.src().join("_service:set_version:foo.spec"))
        .expect("read the set_version output");
    let version_line = spec
        .lines()
        .find(|line| line.starts_with("Version:"))
        .expect("the spec keeps a Version line");
    assert!(
        version_line.contains("1.2.3"),
        "set_version left {version_line}"
    );
    // recompress: the plain tar becomes an xz tarball.
    let fixture = Fixture::new("recompress");
    fs::write(fixture.src().join("file.txt"), "content\n").expect("write the file");
    run(Command::new("tar")
        .args(["-cf", "foo.tar", "file.txt"])
        .current_dir(fixture.src()));
    fs::write(
        fixture.src().join("_service"),
        "<services><service name=\"recompress\"><param name=\"file\">*.tar</param><param name=\"compression\">xz</param></service></services>",
    )
    .expect("write _service");
    with_vertical(|vertical| {
        let req = ServiceRunRequest::new(fixture.src().to_path_buf(), ServiceRunMode::Run);
        vertical
            .service_run(&req)
            .unwrap_or_else(|e| panic!("recompress run failed: {e:?}"));
    });
    assert!(
        fixture
            .src()
            .join("_service:recompress:foo.tar.xz")
            .exists(),
        "recompress produced no foo.tar.xz"
    );
}

#[test]
fn live_source_validator() {
    if !live_enabled() {
        return;
    }
    // A spec with its sources present validates.
    let fixture = Fixture::new("source-validator-ok");
    fs::write(
        fixture.src().join("foo.spec"),
        spec_with("1.0", "foo-1.0.tar.gz"),
    )
    .expect("write the spec");
    run(Command::new("tar")
        .args(["-czf", "foo-1.0.tar.gz", "foo.spec"])
        .current_dir(fixture.src()));
    fs::write(
        fixture.src().join("_service"),
        "<services><service name=\"source_validator\"/></services>",
    )
    .expect("write _service");
    with_vertical(|vertical| {
        let req = ServiceRunRequest::new(fixture.src().to_path_buf(), ServiceRunMode::Run);
        let record = vertical
            .service_run(&req)
            .unwrap_or_else(|e| panic!("source_validator run failed: {e:?}"));
        assert_eq!(record.ran.len(), 1);
        assert_eq!(record.ran[0].exit_code, 0);
    });
    // A spec with an empty patch file fails the run: the validator
    // reports the zero-byte patch as an error. (A missing Source is
    // not a failure while a _service file exists: the check for
    // missing sources is skipped there, the sources being
    // service-generated.)
    let fixture = Fixture::new("source-validator-bad");
    fs::write(
        fixture.src().join("foo.spec"),
        spec_with("1.0", "foo-1.0.tar.gz")
            .replace("Source0:", "Patch0:        fix.patch\nSource0:"),
    )
    .expect("write the spec");
    fs::write(fixture.src().join("fix.patch"), "").expect("write the patch");
    run(Command::new("tar")
        .args(["-czf", "foo-1.0.tar.gz", "foo.spec"])
        .current_dir(fixture.src()));
    fs::write(
        fixture.src().join("_service"),
        "<services><service name=\"source_validator\"/></services>",
    )
    .expect("write _service");
    with_vertical(|vertical| {
        let req = ServiceRunRequest::new(fixture.src().to_path_buf(), ServiceRunMode::Run);
        let err = vertical
            .service_run(&req)
            .expect_err("source_validator must fail on an empty patch");
        assert_eq!(err.kind(), ErrorKind::ServiceFailed);
        assert!(
            err.context.contains("fix.patch"),
            "validator error names the patch: {}",
            err.context
        );
    });
}

#[test]
fn live_format_spec_file_and_spec_cleaner() {
    if !live_enabled() {
        return;
    }
    let messy = "Name:foo\nVersion: 1.0\nRelease: 0\nSummary: Foo   \nLicense: MIT\nSource0: foo-1.0.tar.gz\nBuildArch: noarch\n\n%description\nFoo.\n";
    // format_spec_file: the service cleans the spec in place.
    let fixture = Fixture::new("format-spec-file");
    fs::write(fixture.src().join("foo.spec"), messy).expect("write the spec");
    fs::write(
        fixture.src().join("_service"),
        "<services><service name=\"format_spec_file\"/></services>",
    )
    .expect("write _service");
    with_vertical(|vertical| {
        let req = ServiceRunRequest::new(fixture.src().to_path_buf(), ServiceRunMode::Run);
        vertical
            .service_run(&req)
            .unwrap_or_else(|e| panic!("format_spec_file run failed: {e:?}"));
    });
    // format_spec_file writes the cleaned spec to its outdir: under
    // `run` the output lands prefixed, the source spec is untouched.
    let cleaned = fs::read_to_string(fixture.src().join("_service:format_spec_file:foo.spec"))
        .expect("read the format_spec_file output");
    assert_ne!(cleaned, messy, "format_spec_file left the spec untouched");
    assert!(
        !cleaned
            .lines()
            .any(|line| line.ends_with(' ') || line.ends_with('\t')),
        "format_spec_file left trailing whitespace: {cleaned}"
    );
    // The spec-cleaner operation itself, with -i.
    let fixture = Fixture::new("spec-cleaner");
    fs::write(fixture.src().join("foo.spec"), messy).expect("write the spec");
    with_vertical(|vertical| {
        let result = vertical
            .spec_cleaner(
                fixture.src(),
                Path::new("foo.spec"),
                &["-i".to_string()],
                3600,
                false,
            )
            .unwrap_or_else(|e| panic!("spec-cleaner run failed: {e:?}"));
        assert_eq!(result.exit_code, 0);
    });
    let cleaned = fs::read_to_string(fixture.src().join("foo.spec")).expect("read the spec");
    assert_ne!(cleaned, messy, "spec-cleaner left the spec untouched");
}

#[test]
fn live_precommit_runs_script_in_the_container() {
    if !live_enabled() {
        return;
    }
    let server = FileServer::start(&[("file.txt", "hello\n")]);
    let fixture = Fixture::new("precommit");
    fs::write(
        fixture.src().join("_service"),
        format!(
            "<services><service name=\"download_url\" mode=\"trylocal\"><param name=\"protocol\">http</param><param name=\"host\">{}</param><param name=\"path\">/file.txt</param></service></services>",
            server.host_port()
        ),
    )
    .expect("write _service");
    fs::write(
        fixture.src().join("pre_checkin.sh"),
        "#!/bin/sh\ngrep ^ID= /etc/os-release > container-id.txt\necho generated > generated.txt\n",
    )
    .expect("write pre_checkin.sh");
    with_vertical(|vertical| {
        let record = vertical
            .precommit(fixture.src(), 3600, false)
            .unwrap_or_else(|e| panic!("precommit failed: {e:?}"));
        assert_eq!(record.services.ran.len(), 1);
        assert!(record.pre_checkin.ran);
        assert_eq!(record.pre_checkin.exit_code, Some(0));
    });
    assert!(
        fixture.src().join("file.txt").exists(),
        "the trylocal service did not download file.txt"
    );
    assert!(
        fixture.src().join("generated.txt").exists(),
        "pre_checkin.sh did not run"
    );
    let id =
        fs::read_to_string(fixture.src().join("container-id.txt")).expect("read container-id.txt");
    assert!(
        id.contains("opensuse"),
        "pre_checkin.sh did not run in the container: {id}"
    );
    // A failing script blocks the gate.
    let fixture = Fixture::new("precommit-fail");
    fs::write(fixture.src().join("pre_checkin.sh"), "#!/bin/sh\nexit 1\n")
        .expect("write pre_checkin.sh");
    with_vertical(|vertical| {
        let err = vertical
            .precommit(fixture.src(), 3600, false)
            .expect_err("a failing pre_checkin.sh must block precommit");
        assert_eq!(err.kind(), ErrorKind::ServiceFailed);
    });
}
