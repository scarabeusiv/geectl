use super::build_url;
use super::parse_status_ok;
use super::valid_obs_filename;
use super::valid_obs_name;
use super::DownloadMaster;
use super::DryRun;
use super::ObsClient;
use super::ReleaseTarget;
use super::Repository;
use super::RepositoryDownload;
use crate::auth::Credentials;
use crate::error::ErrorKind;
use crate::error::GeectlError;
use crate::xml::find_tag_end;
use crate::xml::skip_xml_prolog;
use crate::xml::xml_child_elements;
use crate::xml::xml_elements;
use crate::xml::xml_escape;
use crate::xml::xml_unescape;
use crate::xml::XmlDoc;
use std::collections::HashMap;
use std::path::Path;

/// The build-results matrix for a project: one entry per
/// repository/architecture cell, plus the state hash for polling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildResults {
    /// The state hash: pass it back as [`ResultsQuery::oldstate`] to
    /// poll for changes. None when the server sent no state.
    pub state: Option<String>,
    /// One entry per repository/architecture cell.
    pub results: Vec<BuildResult>,
}

/// One repository/architecture cell of a build-results matrix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildResult {
    /// The project the cell belongs to.
    pub project: String,
    /// The repository the cell belongs to.
    pub repository: String,
    /// The architecture the cell belongs to.
    pub arch: String,
    /// The cell's build state (`published`, `broken`, `blocked`,
    /// `scheduling`, ...). This is the authoritative state; `state`
    /// is its legacy name.
    pub code: String,
    /// The legacy name of `code`, still sent by the server; equal to
    /// `code` when the server omits it.
    pub state: String,
    /// The cell is marked for re-scheduling: its state may already be
    /// stale. A result is settled only once no cell is dirty.
    pub dirty: bool,
    /// The server's detail text for the cell (e.g. why a repository
    /// is `broken`), if any.
    pub details: Option<String>,
    /// The per-package build codes in this cell.
    pub statuses: Vec<PackageBuildStatus>,
}

/// One package's build code inside a [`BuildResult`] cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageBuildStatus {
    /// The package.
    pub package: String,
    /// The package's build code (`succeeded`, `failed`,
    /// `unresolvable`, `disabled`, ...).
    pub code: String,
    /// The server's detail text for the package (e.g. the missing
    /// dependency for an `unresolvable` package), if any.
    pub details: Option<String>,
}

/// Query options for [`ObsClient::results`]. Every field is an
/// optional filter; an empty query returns the whole project's
/// matrix. `multibuild` + `locallink` are the per-package form: they
/// expand multibuild flavors into their own statuses, which is what
/// an agent asking about one package wants.
#[derive(Debug, Default, Clone)]
pub struct ResultsQuery {
    /// Only this package's statuses (the repository/architecture
    /// cells still come back whole).
    pub package: Option<String>,
    /// Only these repositories (one `repository` pair per entry).
    pub repositories: Vec<String>,
    /// Only these architectures (one `arch` pair per entry).
    pub arches: Vec<String>,
    /// Only packages with this build code (e.g. `failed`). The
    /// filter applies to each package's code, not to the
    /// repository/architecture cell's state.
    pub code: Option<String>,
    /// Expand multibuild flavors into their own statuses. Older
    /// servers without multibuild support reject the query; geectl
    /// does not fall back to a plain query.
    pub multibuild: bool,
    /// Follow local links when expanding multibuild flavors.
    pub locallink: bool,
    /// The last build's results instead of the current state.
    pub lastbuild: bool,
    /// Poll for changes since this state hash (from a previous
    /// [`BuildResults::state`]); set it only to poll by hand. A hand
    /// poll with `oldstate` may time out at the transport level when
    /// the state does not change within the read timeout: the server
    /// holds the reply for up to five minutes.
    pub oldstate: Option<String>,
}

/// Build states that are still in flight: a cell or a package in one
/// of these states — or a cell marked dirty — means the build has not
/// settled yet. The set is exactly the waiting set the reference
/// client's `--watch` loop polls on.
const WAITING_BUILD_CODES: [&str; 6] = [
    "blocked",
    "scheduled",
    "dispatching",
    "building",
    "signing",
    "finished",
];

/// True when no cell is dirty and no cell or package sits in a
/// waiting state: the build has settled. With `package`, the package
/// must also appear in the matrix: a package-filtered read drops a
/// never-scheduled package instead of reporting it `unknown`, so
/// absence is not settled — the wait keeps polling until the package
/// appears or the deadline passes. Without a filter, an empty matrix
/// (a project with no repositories) is settled.
fn results_settled(results: &BuildResults, package: Option<&str>) -> bool {
    if let Some(package) = package {
        if !results
            .results
            .iter()
            .flat_map(|result| result.statuses.iter())
            .any(|status| status.package == package)
        {
            return false;
        }
    }
    results.results.iter().all(|result| {
        !result.dirty
            && !WAITING_BUILD_CODES.contains(&result.code.as_str())
            && result
                .statuses
                .iter()
                .all(|status| !WAITING_BUILD_CODES.contains(&status.code.as_str()))
    })
}

/// Parse a `<resultlist>` document into typed build results. A
/// document that is not a resultlist — or a `<result>` / `<status>`
/// missing a required attribute — is a server error, never an empty
/// matrix: a failed lookup must not read as "nothing is building".
pub(crate) fn parse_build_results(body: &str) -> Result<BuildResults, GeectlError> {
    let body = body.trim();
    // A malformed build results response still fails closed, but the error
    // carries a bounded preview of what was received, so the failure
    // is diagnosable instead of a bare "not a <resultlist> document". The
    // preview is the debug channel's redacting preview: credential
    // values are masked, binary bodies are never dumped, and the text
    // is capped at DEBUG_BODY_CAP bytes.
    let malformed = || {
        let preview = crate::debug::format_body_preview(body);
        let received = if preview.is_empty() {
            "<empty response>".to_string()
        } else {
            preview
        };
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected OBS build results response: not a <resultlist> document; received: {received}"
            ),
        )
    };
    // The server may emit an XML prolog (`<?xml version="1.0"?>`)
    // before the <resultlist> element; skip it. The prolog must terminate
    // (`?>`) — an unterminated prolog is malformed, not a missing
    // <resultlist>.
    let body = match skip_xml_prolog(body) {
        Ok(body) => body,
        Err(()) => return Err(malformed()),
    };
    let Some(after_open) = body.strip_prefix("<resultlist") else {
        return Err(malformed());
    };
    // The tag name must end here: `<resultlistx>` is not a resultlist.
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    let state = xml_attr(open_tag, "state");
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<resultlist .../>`: a project with no
        // repositories, so no cells.
        return Ok(BuildResults {
            state,
            results: Vec::new(),
        });
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</resultlist>") else {
        return Err(malformed());
    };
    // Nothing but whitespace may follow the closing tag.
    if !inner[close + "</resultlist>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut results = Vec::new();
    for element in xml_elements(inner, "result")? {
        let (tag, content) = xml_split_element(element, "result");
        let required = |name: &str| {
            xml_attr(tag, name).ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    format!("unexpected OBS build results response: <result> without {name}"),
                )
            })
        };
        let project = required("project")?;
        let repository = required("repository")?;
        let arch = required("arch")?;
        let code = required("code")?;
        let state = xml_attr(tag, "state").unwrap_or_else(|| code.clone());
        let dirty = xml_attr(tag, "dirty").as_deref() == Some("true");
        let details = xml_attr(tag, "details");
        let mut statuses = Vec::new();
        for status_element in xml_elements(content, "status")? {
            let (status_tag, status_content) = xml_split_element(status_element, "status");
            let package = xml_attr(status_tag, "package").ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    "unexpected OBS build results response: <status> without package",
                )
            })?;
            let code = xml_attr(status_tag, "code").ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    "unexpected OBS build results response: <status> without code",
                )
            })?;
            let details = xml_element_text(status_content, "details");
            statuses.push(PackageBuildStatus {
                package,
                code,
                details,
            });
        }
        results.push(BuildResult {
            project,
            repository,
            arch,
            code,
            state,
            dirty,
            details,
            statuses,
        });
    }
    Ok(BuildResults { state, results })
}

/// Options for [`ObsClient::build_log`]: which slice of the log to
/// read. The default reads the current log from the start.
#[derive(Debug, Default, Clone)]
pub struct BuildLogQuery {
    /// Start reading at this byte offset; a negative offset reads
    /// from the end of the log.
    pub start: Option<i64>,
    /// The last build's log instead of the current one. This skips
    /// the live-worker handoff and reads the local logfile, so on a
    /// currently-building package with no prior local log it reads
    /// as no log (None) rather than the live log. Cannot combine
    /// with `lastsucceeded`.
    pub last: bool,
    /// The last succeeded build's log. Cannot combine with `last`.
    pub lastsucceeded: bool,
    /// Strip the leading `[timestamp] ` from each log line before
    /// returning the text. Client-side only: the server sends the
    /// same log either way.
    pub strip_time: bool,
}

/// Build-log coordinates from a pasted URL: project, package,
/// repository, arch. The package may carry a multibuild flavor
/// (`pkg:flavor`), as in the four-coordinate form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildLogCoordinates {
    /// The project the package is in.
    pub project: String,
    /// The package to read the log for, with an optional `:flavor`.
    pub package: String,
    /// The repository the package builds in.
    pub repository: String,
    /// The architecture the package builds for.
    pub arch: String,
}

/// The host (with port, if any) of an `http(s)://` URL, or None
/// when the URL has no `http(s)://` scheme.
fn url_host(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    Some(rest.split('/').next().unwrap_or(""))
}

/// Parse a pasted build-log URL into its coordinates. Two forms,
/// both fail closed: anything else is an [`ErrorKind::InvalidInput`]
/// naming the expected forms, never a guess.
///
/// * API form: `http(s)://<api-host>/build/<project>/<repository>/<arch>/<package>/_log`
///   — the host must be the configured API host (`api_base`).
/// * Web-UI form: `http(s)://build.opensuse.org/package/live_build_log/<project>/<package>/<repository>/<arch>`
///   — only for the public instance: the configured API host must
///   be `api.opensuse.org`, since no other web host maps to a known
///   API host.
///
/// The URL takes no query string or fragment: a pasted `?start=…`
/// or `#…` is refused rather than silently ignored.
pub fn parse_build_log_url(url: &str, api_base: &str) -> Result<BuildLogCoordinates, GeectlError> {
    let expected = "expected http(s)://<api-host>/build/<project>/<repository>/<arch>/<package>/_log \
        or http(s)://build.opensuse.org/package/live_build_log/<project>/<package>/<repository>/<arch>";
    let malformed = || {
        GeectlError::new(
            ErrorKind::InvalidInput,
            format!("not a build-log URL: {url:?}; {expected}"),
        )
    };
    if url.contains('?') || url.contains('#') {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("build-log URL takes no query string or fragment: {url:?}; {expected}"),
        ));
    }
    let Some(host) = url_host(url) else {
        return Err(malformed());
    };
    let api_host = url_host(api_base.trim_end_matches('/')).unwrap_or("");
    let Some((_, rest)) = url.split_once("://") else {
        return Err(malformed());
    };
    let segments: Vec<&str> = rest.split('/').collect();
    // segments[0] is the host; the rest is the path.
    let path = &segments[1..];
    let coords = match path {
        ["build", project, repository, arch, package, "_log"] => {
            if host != api_host {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "build-log URL points at {host}, but the configured API host is {api_host}"
                    ),
                ));
            }
            BuildLogCoordinates {
                project: (*project).to_string(),
                package: (*package).to_string(),
                repository: (*repository).to_string(),
                arch: (*arch).to_string(),
            }
        }
        ["package", "live_build_log", project, package, repository, arch] => {
            if host != "build.opensuse.org" || api_host != "api.opensuse.org" {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "the web-UI build-log form is only mapped for build.opensuse.org against api.opensuse.org; the URL host is {host} and the configured API host is {api_host}"
                    ),
                ));
            }
            BuildLogCoordinates {
                project: (*project).to_string(),
                package: (*package).to_string(),
                repository: (*repository).to_string(),
                arch: (*arch).to_string(),
            }
        }
        _ => return Err(malformed()),
    };
    ObsClient::validate_build_coordinates(
        &coords.project,
        Some(&coords.package),
        &coords.repository,
        &coords.arch,
    )?;
    Ok(coords)
}

/// Strip the leading `[timestamp] ` from each log line: the
/// client-side strip-time behaviour. A line is stripped only when
/// it starts with `[` and the first `]` is followed by a space —
/// the same rule as the `^\[[^\]]*\] ` pattern, without a regex
/// dependency. Lines without the prefix pass through unchanged, as
/// do the line endings.
pub(crate) fn strip_log_timestamps(log: &str) -> String {
    let mut out = String::with_capacity(log.len());
    for line in log.split_inclusive('\n') {
        let (text, newline) = match line.strip_suffix('\n') {
            Some(text) => (text, "\n"),
            None => (line, ""),
        };
        out.push_str(strip_timestamp_prefix(text));
        out.push_str(newline);
    }
    out
}

/// The text of one line after its `[…] ` prefix, or the line
/// unchanged when it carries no such prefix.
fn strip_timestamp_prefix(line: &str) -> &str {
    let Some(rest) = line.strip_prefix('[') else {
        return line;
    };
    let Some(end) = rest.find(']') else {
        return line;
    };
    if rest.as_bytes().get(end + 1) != Some(&b' ') {
        return line;
    }
    &rest[end + 2..]
}

/// A running build job, from `GET .../_jobstatus`. `code` is the
/// only field guaranteed present: a jobstatus element with no `code`
/// means no job is running (see [`ObsClient::job_status`]). Only
/// `code`, `result`, and `details` are attributes; the rest are
/// child elements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobStatus {
    /// The job's state (`building`, `scheduled`, ...).
    pub code: String,
    /// The job's result (`succeeded`, `failed`, `unchanged`), if the
    /// server reported one.
    pub result: Option<String>,
    /// The server's detail text for the job, if any.
    pub details: Option<String>,
    /// When the job started (unix time).
    pub starttime: Option<u64>,
    /// When the job ended (unix time), if it ended.
    pub endtime: Option<u64>,
    /// How long the previous build took (seconds).
    pub lastduration: Option<u64>,
    /// The worker id running the job.
    pub workerid: Option<String>,
    /// The architecture of the machine running the job.
    pub hostarch: Option<String>,
    /// The worker URI the job runs on.
    pub uri: Option<String>,
    /// The architecture the job builds for.
    pub arch: Option<String>,
    /// The job name.
    pub job: Option<String>,
    /// The job id (md5 of the job info file).
    pub jobid: Option<String>,
    /// The number of attempts to build the job.
    pub attempt: Option<u64>,
}

/// Parse a `<jobstatus ...>...</jobstatus>` document. An element
/// with no `code` means no job is running (None); a document with no
/// jobstatus element at all — or a non-numeric numeric child — is a
/// server error, never "no job". Only `code`, `result`, and `details`
/// are attributes; the rest (`starttime`, `endtime`, `lastduration`,
/// `workerid`, `hostarch`, `uri`, `arch`, `job`, `jobid`, `attempt`)
/// are child elements.
pub(crate) fn parse_job_status(body: &str) -> Result<Option<JobStatus>, GeectlError> {
    let body = body.trim();
    // A malformed job status response still fails closed, but the error
    // carries a bounded preview of what was received, so the failure
    // is diagnosable instead of a bare "not a <jobstatus> document". The
    // preview is the debug channel's redacting preview: credential
    // values are masked, binary bodies are never dumped, and the text
    // is capped at DEBUG_BODY_CAP bytes.
    let malformed = || {
        let preview = crate::debug::format_body_preview(body);
        let received = if preview.is_empty() {
            "<empty response>".to_string()
        } else {
            preview
        };
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected OBS job status response: not a <jobstatus> document; received: {received}"
            ),
        )
    };
    // The server may emit an XML prolog (`<?xml version="1.0"?>`)
    // before the <jobstatus> element; skip it. The prolog must terminate
    // (`?>`) — an unterminated prolog is malformed, not a missing
    // <jobstatus>.
    let body = match skip_xml_prolog(body) {
        Ok(body) => body,
        Err(()) => return Err(malformed()),
    };
    // The document must start with the root element: a `<jobstatus>`
    // found later in the body is not a jobstatus document.
    let Some(after_tag) = body.strip_prefix("<jobstatus") else {
        return Err(malformed());
    };
    let Some(first) = after_tag.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_tag) else {
        return Err(malformed());
    };
    let tag = &after_tag[..end];
    // The child elements, if the element is not self-closing.
    // Nothing but whitespace may follow the root element either way.
    let inner = if tag.trim_end().ends_with('/') {
        if !after_tag[end + 1..].trim().is_empty() {
            return Err(malformed());
        }
        ""
    } else {
        let after = &after_tag[end + 1..];
        let Some(close) = after.find("</jobstatus>") else {
            return Err(malformed());
        };
        if !after[close + "</jobstatus>".len()..].trim().is_empty() {
            return Err(malformed());
        }
        &after[..close]
    };
    let Some(code) = xml_attr(tag, "code") else {
        // An empty `<jobstatus/>`: no job is running.
        return Ok(None);
    };
    if code.is_empty() {
        return Ok(None);
    }
    let number = |name: &str| -> Result<Option<u64>, GeectlError> {
        match xml_element_text(inner, name) {
            None => Ok(None),
            Some(value) => value.parse::<u64>().map(Some).map_err(|_| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    format!(
                        "unexpected OBS job status response: <jobstatus> <{name}> is not a number"
                    ),
                )
            }),
        }
    };
    Ok(Some(JobStatus {
        code,
        result: xml_attr(tag, "result"),
        details: xml_attr(tag, "details"),
        starttime: number("starttime")?,
        endtime: number("endtime")?,
        lastduration: number("lastduration")?,
        workerid: xml_element_text(inner, "workerid"),
        hostarch: xml_element_text(inner, "hostarch"),
        uri: xml_element_text(inner, "uri"),
        arch: xml_element_text(inner, "arch"),
        job: xml_element_text(inner, "job"),
        jobid: xml_element_text(inner, "jobid"),
        attempt: number("attempt")?,
    }))
}

/// Parse a `<status package code/>` document (the `.../_status`
/// response) into the status code. A document without a `code`
/// attribute is a server error, never "no status": no status reads
/// as `code="unknown"` on the call, not as an empty document.
pub(crate) fn parse_build_status(body: &str) -> Result<String, GeectlError> {
    let body = body.trim();
    // A malformed build status response still fails closed, but the error
    // carries a bounded preview of what was received, so the failure
    // is diagnosable instead of a bare "not a <status> document". The
    // preview is the debug channel's redacting preview: credential
    // values are masked, binary bodies are never dumped, and the text
    // is capped at DEBUG_BODY_CAP bytes.
    let malformed = || {
        let preview = crate::debug::format_body_preview(body);
        let received = if preview.is_empty() {
            "<empty response>".to_string()
        } else {
            preview
        };
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected OBS build status response: not a <status> document; received: {received}"
            ),
        )
    };
    // The server may emit an XML prolog (`<?xml version="1.0"?>`)
    // before the <status> element; skip it. The prolog must terminate
    // (`?>`) — an unterminated prolog is malformed, not a missing
    // <status>.
    let body = match skip_xml_prolog(body) {
        Ok(body) => body,
        Err(()) => return Err(malformed()),
    };
    // The document must start with the root element: a `<status>`
    // found later in the body is not a status document, and nothing
    // but whitespace may follow the root element.
    let Some(after_tag) = body.strip_prefix("<status") else {
        return Err(malformed());
    };
    let Some(first) = after_tag.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_tag) else {
        return Err(malformed());
    };
    let tag = &after_tag[..end];
    if tag.trim_end().ends_with('/') {
        if !after_tag[end + 1..].trim().is_empty() {
            return Err(malformed());
        }
    } else {
        let after = &after_tag[end + 1..];
        let Some(close) = after.find("</status>") else {
            return Err(malformed());
        };
        if !after[close + "</status>".len()..].trim().is_empty() {
            return Err(malformed());
        }
    }
    match xml_attr(tag, "code") {
        Some(code) if !code.is_empty() => Ok(code),
        _ => Err(malformed()),
    }
}

/// One build dependency of a package: the name plus the origin the
/// server resolved it from. The project and hdrmd5 are what the
/// local-build vertical needs: the project set is the trust check,
/// the hdrmd5 is the cache verification. Either may be absent on a
/// dependency the server did not resolve to a binary (a virtual
/// provide, a not-installed flag) — the callers that need them treat
/// an absent value as "unknown", never as a match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildDep {
    /// The dependency name (the package name).
    pub name: String,
    /// The project the dependency resolves from, if the server said.
    pub project: Option<String>,
    /// The repository the dependency resolves from, if the server said.
    pub repository: Option<String>,
    /// The source package the dependency's binary is built from,
    /// if the server said. The binary list and download URLs use
    /// it, not the binary name: a subpackage binary (`libz1`) lives
    /// under its source package (`zlib`), and listing by the binary
    /// name 404s, like the download: the binary name is not a
    /// package. Absent, the whole repository is listed instead
    /// (`_repository`) — the server did not say where the binary
    /// comes from, so no package segment can be assumed.
    pub package: Option<String>,
    /// The binary file name (`name-version-release.arch.rpm`), if the
    /// server said. Without it the caller resolves the file name from
    /// the binary list of the dependency's package.
    pub binary: Option<String>,
    /// The resolved version, if the server said. With `release` and
    /// `arch` present and no `binary` name, the caller computes the
    /// file name locally (`name-version-release.arch.rpm`) instead
    /// of listing binaries: real scheduler buildinfo carries no
    /// `binary` attribute at all.
    pub version: Option<String>,
    /// The resolved release, if the server said (see `version`).
    pub release: Option<String>,
    /// The resolved architecture, if the server said (see `version`).
    /// An RPM file name never carries the epoch, so `epoch` is
    /// parsed but unused for the file name.
    pub arch: Option<String>,
    /// The resolved epoch, if the server said. Parsed for
    /// completeness; it is not part of the file name.
    pub epoch: Option<String>,
    /// The hdrmd5 of the resolved binary, if the server said. A cached
    /// package is verified against it on every use.
    pub hdrmd5: Option<String>,
    /// The dependency is installed before the build starts.
    pub preinstall: bool,
    /// The dependency is installed into the VM, not the build root.
    pub vminstall: bool,
    /// The dependency's scripts run before the build.
    pub runscripts: bool,
    /// The dependency is resolved but never installed: the rpmlist
    /// carries it on a `noinstall:` line, and the engine does not
    /// install it — but the file is still downloaded and listed,
    /// exactly as the existing tool does for spec builds.
    pub noinstall: bool,
    /// The repository architecture the dependency resolves for, if
    /// the server said (a dep can resolve from a different repoarch
    /// than the build arch, e.g. a noarch dep). The cache dir and
    /// the download URL use it; the build arch is the fallback.
    pub repoarch: Option<String>,
    /// The dependency is installed only (never removed by a later
    /// install): the rpmlist carries it on an `installonly:` line.
    pub installonly: bool,
    /// The dependency is installed into the sysroot subdirectory:
    /// the rpmlist prefixes its line with `sysroot: `.
    pub sysroot: bool,
}

/// The preinstall image the buildinfo offers: the image the server
/// would seed a fresh root with. Selected for a fresh root only,
/// and only when `--preinstall-image` is set: the covered
/// dependencies get no download and no `<name> <file>` line, and
/// the engine seeds the root from the image instead (see
/// `local_build::prepare_deps`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreinstallImage {
    /// The project the image lives in.
    pub project: Option<String>,
    /// The repository the image lives in.
    pub repository: Option<String>,
    /// The package the image is built from.
    pub package: Option<String>,
    /// The image file name.
    pub filename: Option<String>,
    /// The image's hdrmd5, if the server said.
    pub hdrmd5: Option<String>,
    /// The binaries the image covers, in the order the server
    /// listed them: a dep whose name is here gets no download and
    /// no `<name> <file>` line; it keeps its place in the install
    /// lists and is installed from the image instead.
    pub binaries: Vec<String>,
}

/// The buildinfo for a package in a repository/architecture: the
/// error that blocks the build, if any, and the dependencies the
/// build pulls in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInfo {
    /// The error blocking the build (e.g. `unresolvable:
    /// DOES-NOT-EXIST`), if the build cannot start.
    pub error: Option<String>,
    /// The names of the build dependencies, in build order.
    pub bdeps: Vec<BuildDep>,
    /// The preinstall image the server offers, if any.
    pub preinstall_image: Option<PreinstallImage>,
    /// The published-repo mirror URL per (project, repository),
    /// from the buildinfo `<path>` elements: the `url` attribute,
    /// e.g. `https://download.opensuse.org/tumbleweed/repo/oss/`,
    /// kept only when it passes [`crate::http::valid_mirror_url`]
    /// (public `https`, no userinfo, query, or fragment). A repo
    /// without a `url` has no entry — the dep fetch falls back to
    /// the API for it.
    pub urls: HashMap<(String, String), String>,
    /// The `<path>` elements skipped while parsing: malformed
    /// (no project or repository) or carrying a mirror URL that
    /// fails validation. Mirror data is only an optimisation, so a
    /// broken entry never fails the parse — the dep fetch falls
    /// back to the API for that repo — but the count is kept so the
    /// caller can log it.
    pub skipped_paths: usize,
}

/// One entry of a package's build history: a completed build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildHistoryEntry {
    /// The source revision the build ran on.
    pub rev: String,
    /// The source md5 the build ran on.
    pub srcmd5: String,
    /// The version-release the build produced.
    pub versrel: String,
    /// The build count for that version-release.
    pub bcnt: u32,
    /// When the build ran (unix time).
    pub time: u64,
    /// How long the build took (seconds), if recorded.
    pub duration: Option<u64>,
}

/// One entry of a repository/architecture's job history: a build job
/// that ran, succeeded or failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobHistoryEntry {
    /// The repository the job ran in.
    pub repository: Option<String>,
    /// The architecture the job ran on.
    pub arch: Option<String>,
    /// The package the job built.
    pub package: String,
    /// The source revision the job ran on.
    pub rev: Option<String>,
    /// The source md5 the job ran on.
    pub srcmd5: Option<String>,
    /// The version-release the job produced.
    pub versrel: Option<String>,
    /// The build count for that version-release.
    pub bcnt: Option<u32>,
    /// When the job became ready (unix time).
    pub readytime: Option<u64>,
    /// When the job started (unix time).
    pub starttime: Option<u64>,
    /// When the job ended (unix time).
    pub endtime: Option<u64>,
    /// The job's result (`succeeded`, `failed`, ...).
    pub code: String,
    /// The worker URI the job ran on.
    pub uri: Option<String>,
    /// The worker id the job ran on.
    pub workerid: Option<String>,
    /// The architecture of the machine that ran the job.
    pub hostarch: Option<String>,
    /// The reason the job ran.
    pub reason: Option<String>,
    /// The md5 the job verified against.
    pub verifymd5: Option<String>,
}

/// Parse a `<buildinfo ...>...</buildinfo>` document into the error
/// blocking the build, if any, and the build dependency names. A
/// document that is not a buildinfo — or a `<bdep>` without a name —
/// is a server error, never an empty buildinfo.
pub(crate) fn parse_buildinfo(body: &str) -> Result<BuildInfo, GeectlError> {
    let body = body.trim();
    // A malformed buildinfo response still fails closed, but the error
    // carries a bounded preview of what was received, so the failure
    // is diagnosable instead of a bare "not a <buildinfo> document". The
    // preview is the debug channel's redacting preview: credential
    // values are masked, binary bodies are never dumped, and the text
    // is capped at DEBUG_BODY_CAP bytes.
    let malformed = || {
        let preview = crate::debug::format_body_preview(body);
        let received = if preview.is_empty() {
            "<empty response>".to_string()
        } else {
            preview
        };
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected OBS buildinfo response: not a <buildinfo> document; received: {received}"
            ),
        )
    };
    // The server may emit an XML prolog (`<?xml version="1.0"?>`)
    // before the <buildinfo> element; skip it. The prolog must terminate
    // (`?>`) — an unterminated prolog is malformed, not a missing
    // <buildinfo>.
    let body = match skip_xml_prolog(body) {
        Ok(body) => body,
        Err(()) => return Err(malformed()),
    };
    let Some(after_open) = body.strip_prefix("<buildinfo") else {
        return Err(malformed());
    };
    // The tag name must end here: `<buildinfox>` is not a buildinfo.
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<buildinfo .../>`: no error, no dependencies.
        return Ok(BuildInfo {
            error: None,
            bdeps: Vec::new(),
            preinstall_image: None,
            urls: HashMap::new(),
            skipped_paths: 0,
        });
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</buildinfo>") else {
        return Err(malformed());
    };
    // Nothing but whitespace may follow the closing tag.
    if !inner[close + "</buildinfo>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let error = xml_element_text(inner, "error");
    let mut bdeps = Vec::new();
    for element in xml_elements(inner, "bdep")? {
        let (tag, _) = xml_split_element(element, "bdep");
        let name = xml_attr(tag, "name").ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS buildinfo response: <bdep> without name",
            )
        })?;
        bdeps.push(BuildDep {
            name,
            project: xml_attr(tag, "project"),
            repository: xml_attr(tag, "repository"),
            package: xml_attr(tag, "package"),
            binary: xml_attr(tag, "binary"),
            version: xml_attr(tag, "version"),
            release: xml_attr(tag, "release"),
            arch: xml_attr(tag, "arch"),
            epoch: xml_attr(tag, "epoch"),
            hdrmd5: xml_attr(tag, "hdrmd5"),
            preinstall: xml_flag(tag, "preinstall"),
            vminstall: xml_flag(tag, "vminstall"),
            runscripts: xml_flag(tag, "runscripts"),
            noinstall: xml_flag(tag, "noinstall"),
            repoarch: xml_attr(tag, "repoarch"),
            installonly: xml_flag(tag, "installonly"),
            sysroot: xml_flag(tag, "sysroot"),
        });
    }
    // The preinstall image, when the server offers one: a single
    // `<preinstallimage .../>` element. A second one is ignored —
    // the server sends at most one, and the first is the offer. A
    // malformed element is ignored rather than failing the parse:
    // the image is an optimization, so a broken image element must
    // not fail a build whose dependencies parsed fine. The covered
    // binaries are the `<binary>` children, in server order; a
    // malformed child list reads as no covered binaries, which
    // drops the image (nothing is covered, so it buys nothing).
    let preinstall_image = match xml_elements(inner, "preinstallimage") {
        Ok(elements) => elements.first().map(|element| {
            let (tag, inner) = xml_split_element(element, "preinstallimage");
            let binaries = xml_elements(inner, "binary")
                .map(|children| {
                    children
                        .iter()
                        .map(|child| xml_split_element(child, "binary").1.trim().to_string())
                        .filter(|name| !name.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            PreinstallImage {
                project: xml_attr(tag, "project"),
                repository: xml_attr(tag, "repository"),
                package: xml_attr(tag, "package"),
                filename: xml_attr(tag, "filename"),
                hdrmd5: xml_attr(tag, "hdrmd5"),
                binaries,
            }
        }),
        Err(_) => None,
    };
    // The published-repo mirror URLs, from the `<path>` elements:
    // one per (project, repository), keyed by that pair, carrying
    // the `url` attribute when it passes `valid_mirror_url`
    // (public `https`, no userinfo, query, or fragment). A `<path>`
    // without a project or repository is malformed, and a `<path>`
    // whose URL fails validation names no usable mirror — either
    // way the entry is skipped and counted, never fatal: mirror
    // data is only an optimisation, and the dep fetch falls back
    // to the API for that repo. A `<path>` without a `url` names a
    // repo that is not published: it carries no entry, and the dep
    // fetch falls back to the API for it (the existing tool reads
    // the URL the same way, skipping a path with no `url`). A
    // later element for the same pair wins, as in the existing
    // tool's mapping.
    let mut urls = HashMap::new();
    let mut skipped_paths = 0usize;
    for element in xml_elements(inner, "path")? {
        let (tag, _) = xml_split_element(element, "path");
        let Some(project) = xml_attr_non_empty(tag, "project") else {
            skipped_paths += 1;
            continue;
        };
        let Some(repository) = xml_attr_non_empty(tag, "repository") else {
            skipped_paths += 1;
            continue;
        };
        if let Some(url) = xml_attr(tag, "url").filter(|url| !url.is_empty()) {
            if crate::http::valid_mirror_url(&url) {
                urls.insert((project, repository), url);
            } else {
                skipped_paths += 1;
            }
        }
    }
    Ok(BuildInfo {
        error,
        bdeps,
        preinstall_image,
        urls,
        skipped_paths,
    })
}

/// A boolean flag attribute on a buildinfo element: present and
/// `"1"` or `"true"`. Absent, `"0"`, or anything else reads as false
/// — a flag the server did not set is off, never an error.
fn xml_flag(tag: &str, name: &str) -> bool {
    matches!(xml_attr(tag, name).as_deref(), Some("1") | Some("true"))
}

/// Parse a `<buildhistory>...</buildhistory>` document into the
/// completed builds. A document that is not a buildhistory — or an
/// `<entry>` missing a required attribute or carrying a non-numeric
/// one — is a server error, never an empty history.
pub(crate) fn parse_build_history(body: &str) -> Result<Vec<BuildHistoryEntry>, GeectlError> {
    let body = body.trim();
    // A malformed build history response still fails closed, but the error
    // carries a bounded preview of what was received, so the failure
    // is diagnosable instead of a bare "not a <buildhistory> document". The
    // preview is the debug channel's redacting preview: credential
    // values are masked, binary bodies are never dumped, and the text
    // is capped at DEBUG_BODY_CAP bytes.
    let malformed = || {
        let preview = crate::debug::format_body_preview(body);
        let received = if preview.is_empty() {
            "<empty response>".to_string()
        } else {
            preview
        };
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected OBS build history response: not a <buildhistory> document; received: {received}"
            ),
        )
    };
    // The server may emit an XML prolog (`<?xml version="1.0"?>`)
    // before the <buildhistory> element; skip it. The prolog must terminate
    // (`?>`) — an unterminated prolog is malformed, not a missing
    // <buildhistory>.
    let body = match skip_xml_prolog(body) {
        Ok(body) => body,
        Err(()) => return Err(malformed()),
    };
    let Some(after_open) = body.strip_prefix("<buildhistory") else {
        return Err(malformed());
    };
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<buildhistory/>`: never built.
        return Ok(Vec::new());
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</buildhistory>") else {
        return Err(malformed());
    };
    if !inner[close + "</buildhistory>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut entries = Vec::new();
    for element in xml_elements(inner, "entry")? {
        let (tag, _) = xml_split_element(element, "entry");
        let required = |name: &str| {
            xml_attr(tag, name).ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    format!("unexpected OBS build history response: <entry> without {name}"),
                )
            })
        };
        entries.push(BuildHistoryEntry {
            rev: required("rev")?,
            srcmd5: required("srcmd5")?,
            versrel: required("versrel")?,
            bcnt: xml_attr_number_required(tag, "bcnt")?,
            time: xml_attr_number_required(tag, "time")?,
            duration: xml_attr_number(tag, "duration")?,
        });
    }
    // The wire order is oldest-first (the backend reads the
    // append-only history file backwards, then reverses it), so the
    // newest build is first.
    entries.reverse();
    Ok(entries)
}

/// Parse a `<jobhistlist>...</jobhistlist>` document into the build
/// jobs that ran. A document that is not a jobhistlist — or a
/// `<jobhist>` missing its package or code, or carrying a non-numeric
/// number — is a server error, never an empty history.
pub(crate) fn parse_job_history(body: &str) -> Result<Vec<JobHistoryEntry>, GeectlError> {
    let body = body.trim();
    // A malformed job history response still fails closed, but the error
    // carries a bounded preview of what was received, so the failure
    // is diagnosable instead of a bare "not a <jobhistlist> document". The
    // preview is the debug channel's redacting preview: credential
    // values are masked, binary bodies are never dumped, and the text
    // is capped at DEBUG_BODY_CAP bytes.
    let malformed = || {
        let preview = crate::debug::format_body_preview(body);
        let received = if preview.is_empty() {
            "<empty response>".to_string()
        } else {
            preview
        };
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected OBS job history response: not a <jobhistlist> document; received: {received}"
            ),
        )
    };
    // The server may emit an XML prolog (`<?xml version="1.0"?>`)
    // before the <jobhistlist> element; skip it. The prolog must terminate
    // (`?>`) — an unterminated prolog is malformed, not a missing
    // <jobhistlist>.
    let body = match skip_xml_prolog(body) {
        Ok(body) => body,
        Err(()) => return Err(malformed()),
    };
    let Some(after_open) = body.strip_prefix("<jobhistlist") else {
        return Err(malformed());
    };
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<jobhistlist/>`: no jobs ran.
        return Ok(Vec::new());
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</jobhistlist>") else {
        return Err(malformed());
    };
    if !inner[close + "</jobhistlist>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut entries = Vec::new();
    for element in xml_elements(inner, "jobhist")? {
        let (tag, _) = xml_split_element(element, "jobhist");
        let package = xml_attr(tag, "package").ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS job history response: <jobhist> without package",
            )
        })?;
        let code = xml_attr(tag, "code").ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS job history response: <jobhist> without code",
            )
        })?;
        entries.push(JobHistoryEntry {
            repository: xml_attr(tag, "repository"),
            arch: xml_attr(tag, "arch"),
            package,
            rev: xml_attr(tag, "rev"),
            srcmd5: xml_attr(tag, "srcmd5"),
            versrel: xml_attr(tag, "versrel"),
            bcnt: xml_attr_number(tag, "bcnt")?,
            readytime: xml_attr_number(tag, "readytime")?,
            starttime: xml_attr_number(tag, "starttime")?,
            endtime: xml_attr_number(tag, "endtime")?,
            code,
            uri: xml_attr(tag, "uri"),
            workerid: xml_attr(tag, "workerid"),
            hostarch: xml_attr(tag, "hostarch"),
            reason: xml_attr(tag, "reason"),
            verifymd5: xml_attr(tag, "verifymd5"),
        });
    }
    // The wire order is oldest-first, like the build history, so the
    // newest job is first.
    entries.reverse();
    Ok(entries)
}

/// One built file in a build result directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryFile {
    /// The file name (e.g. `foo-1.0-1.x86_64.rpm`, `_buildenv`).
    pub name: String,
    /// The file size in bytes, if the server reported it.
    pub size: Option<u64>,
    /// The file mtime (unix time), if the server reported it.
    pub mtime: Option<u64>,
}

/// One binary version in a `<binaryversionlist>` document: the
/// server-reported name, the RPM header md5, and the size in KiB.
/// The name is `<binary>.<suffix>` (e.g. `foo.rpm`); an error entry
/// for a binary the repository does not carry carries the bare
/// queried name instead. `error` carries the server's per-binary
/// error (`not available` when the repository does not carry the
/// binary); a version entry for a binary the server had to fetch
/// carries the fetched file's hdrmd5.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryVersion {
    /// The server-reported name (`<binary>.<suffix>`, e.g. `foo.rpm`);
    /// error entries carry the bare queried name.
    pub name: String,
    /// The RPM header md5, if the server reported it.
    pub hdrmd5: Option<String>,
    /// The file size in KiB, if the server reported it.
    pub size_kb: Option<u64>,
    /// The server's per-binary error, if any.
    pub error: Option<String>,
}

/// The metadata of a built file: the RPM header fields (name,
/// epoch, version, release, arch, source, summary, description,
/// disturl, size, mtime) plus the dependency lists (plain view) and
/// the resolved dependencies (`fileinfo_ext` view).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    /// The file name.
    pub filename: String,
    /// The RPM name.
    pub name: Option<String>,
    /// The RPM epoch.
    pub epoch: Option<u64>,
    /// The RPM version.
    pub version: Option<String>,
    /// The RPM release.
    pub release: Option<String>,
    /// The RPM architecture.
    pub arch: Option<String>,
    /// The source package, for a source RPM.
    pub source: Option<String>,
    /// The RPM summary.
    pub summary: Option<String>,
    /// The RPM description.
    pub description: Option<String>,
    /// The dist URL identifying where the file was built.
    pub disturl: Option<String>,
    /// The file size in bytes.
    pub size: Option<u64>,
    /// The file mtime (unix time).
    pub mtime: Option<u64>,
    /// The dependencies the file provides (plain view).
    pub provides: Vec<String>,
    /// The dependencies the file requires (plain view).
    pub requires: Vec<String>,
    /// The pre-requires (plain view).
    pub prerequires: Vec<String>,
    /// The conflicts (plain view).
    pub conflicts: Vec<String>,
    /// The obsoletes (plain view).
    pub obsoletes: Vec<String>,
    /// The recommends (plain view).
    pub recommends: Vec<String>,
    /// The supplements (plain view).
    pub supplements: Vec<String>,
    /// The suggests (plain view).
    pub suggests: Vec<String>,
    /// The enhances (plain view).
    pub enhances: Vec<String>,
    /// The dependencies the file provides, with the packages that
    /// require each one (`fileinfo_ext` view only).
    pub provides_ext: Vec<ExtDependency>,
    /// The dependencies the file requires, with the package providing
    /// each one (`fileinfo_ext` view only).
    pub requires_ext: Vec<ExtDependency>,
    /// The dependencies the file recommends, with the package
    /// providing each one (`fileinfo_ext` view only).
    pub recommends_ext: Vec<ExtDependency>,
    /// The dependencies the file supplements, with the package
    /// providing each one (`fileinfo_ext` view only).
    pub supplements_ext: Vec<ExtDependency>,
}

/// One dependency in a `fileinfo_ext` view, with its resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtDependency {
    /// The dependency (e.g. `libc.so.6()(64bit)`).
    pub dep: String,
    /// For a required dependency: the packages providing it, if
    /// any were found in the repository.
    pub provided_by: Vec<ProvidedBy>,
    /// For a provided dependency: the packages requiring it.
    pub required_by: Vec<ProvidedBy>,
}

/// A package named by a `fileinfo_ext` resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvidedBy {
    /// The package name.
    pub name: String,
    /// The package epoch.
    pub epoch: Option<u64>,
    /// The package version.
    pub version: Option<String>,
    /// The package release.
    pub release: Option<String>,
    /// The package architecture.
    pub arch: Option<String>,
    /// The project the package comes from.
    pub project: Option<String>,
    /// The repository the package comes from.
    pub repository: Option<String>,
}

/// Parse a `<binarylist>...</binarylist>` document into the built
/// files. A document that is not a binarylist — or a `<binary>`
/// without a filename, or with a non-numeric size/mtime — is a server
/// error, never an empty list.
pub(crate) fn parse_binary_list(body: &str) -> Result<Vec<BinaryFile>, GeectlError> {
    let body = body.trim();
    // A malformed binary list response still fails closed, but the error
    // carries a bounded preview of what was received, so the failure
    // is diagnosable instead of a bare "not a <binarylist> document". The
    // preview is the debug channel's redacting preview: credential
    // values are masked, binary bodies are never dumped, and the text
    // is capped at DEBUG_BODY_CAP bytes.
    let malformed = || {
        let preview = crate::debug::format_body_preview(body);
        let received = if preview.is_empty() {
            "<empty response>".to_string()
        } else {
            preview
        };
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected OBS binary list response: not a <binarylist> document; received: {received}"
            ),
        )
    };
    // The server may emit an XML prolog (`<?xml version="1.0"?>`)
    // before the <binarylist> element; skip it. The prolog must terminate
    // (`?>`) — an unterminated prolog is malformed, not a missing
    // <binarylist>.
    let body = match skip_xml_prolog(body) {
        Ok(body) => body,
        Err(()) => return Err(malformed()),
    };
    let Some(after_open) = body.strip_prefix("<binarylist") else {
        return Err(malformed());
    };
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<binarylist/>`: no binaries built.
        return Ok(Vec::new());
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</binarylist>") else {
        return Err(malformed());
    };
    if !inner[close + "</binarylist>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut files = Vec::new();
    for element in xml_elements(inner, "binary")? {
        let (tag, _) = xml_split_element(element, "binary");
        let name = xml_attr(tag, "filename").ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS binary list response: <binary> without filename",
            )
        })?;
        files.push(BinaryFile {
            name,
            size: xml_attr_number(tag, "size")?,
            mtime: xml_attr_number(tag, "mtime")?,
        });
    }
    Ok(files)
}

/// Parse a `<binaryversionlist>...</binaryversionlist>` document into
/// the binary versions. A document that is not a binaryversionlist —
/// or a `<binary>` without a name, or with a non-numeric sizek — is a
/// server error, never an empty list.
pub(crate) fn parse_binary_version_list(body: &str) -> Result<Vec<BinaryVersion>, GeectlError> {
    let body = body.trim();
    // A malformed binary version list response still fails closed,
    // but the error carries a bounded preview of what was received,
    // so the failure is diagnosable instead of a bare "not a
    // <binaryversionlist> document". The preview is the debug
    // channel's redacting preview: credential values are masked,
    // binary bodies are never dumped, and the text is capped at
    // DEBUG_BODY_CAP bytes.
    let malformed = || {
        let preview = crate::debug::format_body_preview(body);
        let received = if preview.is_empty() {
            "<empty response>".to_string()
        } else {
            preview
        };
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected OBS binary version list response: not a <binaryversionlist> document; received: {received}"
            ),
        )
    };
    // The server may emit an XML prolog (`<?xml version="1.0"?>`)
    // before the <binaryversionlist> element; skip it. The prolog
    // must terminate (`?>`) — an unterminated prolog is malformed,
    // not a missing <binaryversionlist>.
    let body = match skip_xml_prolog(body) {
        Ok(body) => body,
        Err(()) => return Err(malformed()),
    };
    let Some(after_open) = body.strip_prefix("<binaryversionlist") else {
        return Err(malformed());
    };
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<binaryversionlist/>`: no binaries.
        return Ok(Vec::new());
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</binaryversionlist>") else {
        return Err(malformed());
    };
    if !inner[close + "</binaryversionlist>".len()..]
        .trim()
        .is_empty()
    {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut versions = Vec::new();
    for element in xml_elements(inner, "binary")? {
        let (tag, _) = xml_split_element(element, "binary");
        let name = xml_attr(tag, "name")
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    "unexpected OBS binary version list response: <binary> without name",
                )
            })?;
        versions.push(BinaryVersion {
            name,
            hdrmd5: xml_attr(tag, "hdrmd5"),
            size_kb: xml_attr_number(tag, "sizek")?,
            error: xml_attr(tag, "error"),
        });
    }
    Ok(versions)
}

/// Parse one `<provided_by>` / `<requiredby>` resolution element
/// (both carry the same attributes) into the package it names.
fn parse_provided_by(tag: &str) -> Result<ProvidedBy, GeectlError> {
    let name = xml_attr(tag, "name").ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS fileinfo response: a dependency resolution without name",
        )
    })?;
    Ok(ProvidedBy {
        name,
        epoch: xml_attr_number(tag, "epoch")?,
        version: xml_attr(tag, "version"),
        release: xml_attr(tag, "release"),
        arch: xml_attr(tag, "arch"),
        project: xml_attr(tag, "project"),
        repository: xml_attr(tag, "repository"),
    })
}

/// Parse one `<*_ext>` dependency element into its dependency
/// and its resolution: the packages requiring a provided dependency
/// (`<requiredby>` children) and the packages providing a required,
/// recommended, or supplemented dependency (`<provided_by>` under
/// both spellings). An element without a `dep` is a server error.
fn parse_ext_dependency(element: &str, name: &str) -> Result<ExtDependency, GeectlError> {
    let (tag, content) = xml_split_element(element, name);
    let dep = xml_attr(tag, "dep").ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected OBS fileinfo response: <{name}> without dep"),
        )
    })?;
    let mut required_by = Vec::new();
    for child in xml_elements(content, "requiredby")? {
        let (child_tag, _) = xml_split_element(child, "requiredby");
        required_by.push(parse_provided_by(child_tag)?);
    }
    let mut provided_by = Vec::new();
    for spelling in ["provided_by", "providedby"] {
        for child in xml_elements(content, spelling)? {
            let (child_tag, _) = xml_split_element(child, spelling);
            provided_by.push(parse_provided_by(child_tag)?);
        }
    }
    Ok(ExtDependency {
        dep,
        provided_by,
        required_by,
    })
}

/// Parse a `<fileinfo ...>...</fileinfo>` document into the file's
/// metadata. The plain view carries the dependency lists
/// (`provides`, `requires`, `prerequires`, `conflicts`,
/// `obsoletes`, `recommends`, `supplements`, `suggests`,
/// `enhances`) as repeated text elements; the `fileinfo_ext` view
/// carries `provides_ext` / `requires_ext` / `recommends_ext` /
/// `supplements_ext` with one element per dependency. A document
/// that is not a fileinfo — or a fileinfo without a filename, an ext
/// dependency without a `dep`, or a non-numeric size/mtime — is a
/// server error, never an empty fileinfo. The `provided_by` element
/// is read under both spellings: the backend writes `<providedby>`,
/// the API docs name it `<provided_by>`. `filelist` is only present
/// with `withfilelist`, which geectl never requests, so it is not
/// read.
pub(crate) fn parse_fileinfo(body: &str) -> Result<FileInfo, GeectlError> {
    let body = body.trim();
    // A malformed fileinfo response still fails closed, but the error
    // carries a bounded preview of what was received, so the failure
    // is diagnosable instead of a bare "not a <fileinfo> document". The
    // preview is the debug channel's redacting preview: credential
    // values are masked, binary bodies are never dumped, and the text
    // is capped at DEBUG_BODY_CAP bytes.
    let malformed = || {
        let preview = crate::debug::format_body_preview(body);
        let received = if preview.is_empty() {
            "<empty response>".to_string()
        } else {
            preview
        };
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected OBS fileinfo response: not a <fileinfo> document; received: {received}"
            ),
        )
    };
    // The server may emit an XML prolog (`<?xml version="1.0"?>`)
    // before the <fileinfo> element; skip it. The prolog must terminate
    // (`?>`) — an unterminated prolog is malformed, not a missing
    // <fileinfo>.
    let body = match skip_xml_prolog(body) {
        Ok(body) => body,
        Err(()) => return Err(malformed()),
    };
    let Some(after_open) = body.strip_prefix("<fileinfo") else {
        return Err(malformed());
    };
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    let filename = xml_attr(open_tag, "filename").ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS fileinfo response: <fileinfo> without filename",
        )
    })?;
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<fileinfo .../>`: a file with no metadata.
        return Ok(FileInfo {
            filename,
            name: None,
            epoch: None,
            version: None,
            release: None,
            arch: None,
            source: None,
            summary: None,
            description: None,
            disturl: None,
            size: None,
            mtime: None,
            provides: Vec::new(),
            requires: Vec::new(),
            prerequires: Vec::new(),
            conflicts: Vec::new(),
            obsoletes: Vec::new(),
            recommends: Vec::new(),
            supplements: Vec::new(),
            suggests: Vec::new(),
            enhances: Vec::new(),
            provides_ext: Vec::new(),
            requires_ext: Vec::new(),
            recommends_ext: Vec::new(),
            supplements_ext: Vec::new(),
        });
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</fileinfo>") else {
        return Err(malformed());
    };
    if !inner[close + "</fileinfo>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let number_text = |name: &str| -> Result<Option<u64>, GeectlError> {
        match xml_element_text(inner, name) {
            None => Ok(None),
            Some(value) => value.parse::<u64>().map(Some).map_err(|_| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    format!("unexpected OBS fileinfo response: <{name}> is not a number"),
                )
            }),
        }
    };
    let list = |name: &str| -> Result<Vec<String>, GeectlError> {
        Ok(xml_elements(inner, name)?
            .iter()
            .map(|element| {
                let (_, text) = xml_split_element(element, name);
                xml_unescape(text.trim())
            })
            .filter(|text| !text.is_empty())
            .collect())
    };
    let mut provides_ext = Vec::new();
    for element in xml_elements(inner, "provides_ext")? {
        provides_ext.push(parse_ext_dependency(element, "provides_ext")?);
    }
    let mut requires_ext = Vec::new();
    for element in xml_elements(inner, "requires_ext")? {
        requires_ext.push(parse_ext_dependency(element, "requires_ext")?);
    }
    let mut recommends_ext = Vec::new();
    for element in xml_elements(inner, "recommends_ext")? {
        recommends_ext.push(parse_ext_dependency(element, "recommends_ext")?);
    }
    let mut supplements_ext = Vec::new();
    for element in xml_elements(inner, "supplements_ext")? {
        supplements_ext.push(parse_ext_dependency(element, "supplements_ext")?);
    }
    Ok(FileInfo {
        filename,
        name: xml_element_text(inner, "name"),
        epoch: number_text("epoch")?,
        version: xml_element_text(inner, "version"),
        release: xml_element_text(inner, "release"),
        arch: xml_element_text(inner, "arch"),
        source: xml_element_text(inner, "source"),
        summary: xml_element_text(inner, "summary"),
        description: xml_element_text(inner, "description"),
        disturl: xml_element_text(inner, "disturl"),
        size: number_text("size")?,
        mtime: number_text("mtime")?,
        provides: list("provides")?,
        requires: list("requires")?,
        prerequires: list("prerequires")?,
        conflicts: list("conflicts")?,
        obsoletes: list("obsoletes")?,
        recommends: list("recommends")?,
        supplements: list("supplements")?,
        suggests: list("suggests")?,
        enhances: list("enhances")?,
        provides_ext,
        requires_ext,
        recommends_ext,
        supplements_ext,
    })
}

/// One package's build dependencies, from `_builddepinfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageDeps {
    /// The package.
    pub name: String,
    /// The source package, if the server reported one.
    pub source: Option<String>,
    /// The packages it depends on for the build (whole-package
    /// dependencies, not individual binaries). In the reverse
    /// (`revpkgnames`) view returned by [`ObsClient::whatdependson`],
    /// these are instead the packages depending on the queried
    /// package.
    pub dependencies: Vec<String>,
    /// The subpackages it produces.
    pub subpackages: Vec<String>,
}

/// Parse a `<builddepinfo>...</builddepinfo>` document into the
/// per-package build dependencies. Only the direct `<package>`
/// children count: the document also carries `<cycle>` and `<scc>`
/// elements whose `<package>` children are cycle members, not
/// dependencies. A document that is not a builddepinfo — or a
/// `<package>` without a name — is a server error, never an empty
/// list.
pub(crate) fn parse_builddepinfo(body: &str) -> Result<Vec<PackageDeps>, GeectlError> {
    let body = body.trim();
    // A malformed builddepinfo response still fails closed, but the error
    // carries a bounded preview of what was received, so the failure
    // is diagnosable instead of a bare "not a <builddepinfo> document". The
    // preview is the debug channel's redacting preview: credential
    // values are masked, binary bodies are never dumped, and the text
    // is capped at DEBUG_BODY_CAP bytes.
    let malformed = || {
        let preview = crate::debug::format_body_preview(body);
        let received = if preview.is_empty() {
            "<empty response>".to_string()
        } else {
            preview
        };
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected OBS builddepinfo response: not a <builddepinfo> document; received: {received}"
            ),
        )
    };
    // The server may emit an XML prolog (`<?xml version="1.0"?>`)
    // before the <builddepinfo> element; skip it. The prolog must terminate
    // (`?>`) — an unterminated prolog is malformed, not a missing
    // <builddepinfo>.
    let body = match skip_xml_prolog(body) {
        Ok(body) => body,
        Err(()) => return Err(malformed()),
    };
    let Some(after_open) = body.strip_prefix("<builddepinfo") else {
        return Err(malformed());
    };
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<builddepinfo/>`: no packages, no deps.
        return Ok(Vec::new());
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</builddepinfo>") else {
        return Err(malformed());
    };
    if !inner[close + "</builddepinfo>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut packages = Vec::new();
    for element in xml_child_elements(inner, "package")? {
        let (tag, content) = xml_split_element(element, "package");
        let name = xml_attr(tag, "name").ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS builddepinfo response: <package> without name",
            )
        })?;
        let dependencies = xml_elements(content, "pkgdep")?
            .iter()
            .map(|dep| {
                let (_, text) = xml_split_element(dep, "pkgdep");
                xml_unescape(text.trim())
            })
            .filter(|text| !text.is_empty())
            .collect();
        let subpackages = xml_elements(content, "subpkg")?
            .iter()
            .map(|sub| {
                let (_, text) = xml_split_element(sub, "subpkg");
                xml_unescape(text.trim())
            })
            .filter(|text| !text.is_empty())
            .collect();
        packages.push(PackageDeps {
            name,
            source: xml_element_text(content, "source"),
            dependencies,
            subpackages,
        });
    }
    Ok(packages)
}

/// Parse a `<project ...>...</project>` document (`_meta`) into the
/// project's repositories, with each repository's rebuild, block,
/// and linkedbuild flags, downloads, release targets, and host
/// systems. A document that is not a project — or a `<repository>`
/// without a name, or a `<path>` without a project — is a server
/// error, never an empty list.
pub(crate) fn parse_repositories(body: &str) -> Result<Vec<Repository>, GeectlError> {
    let body = body.trim();
    // A malformed project meta response still fails closed, but the error
    // carries a bounded preview of what was received, so the failure
    // is diagnosable instead of a bare "not a <project> document". The
    // preview is the debug channel's redacting preview: credential
    // values are masked, binary bodies are never dumped, and the text
    // is capped at DEBUG_BODY_CAP bytes.
    let malformed = || {
        let preview = crate::debug::format_body_preview(body);
        let received = if preview.is_empty() {
            "<empty response>".to_string()
        } else {
            preview
        };
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected OBS project meta response: not a <project> document; received: {received}"
            ),
        )
    };
    // The server may emit an XML prolog (`<?xml version="1.0"?>`)
    // before the <project> element; skip it. The prolog must terminate
    // (`?>`) — an unterminated prolog is malformed, not a missing
    // <project>.
    let body = match skip_xml_prolog(body) {
        Ok(body) => body,
        Err(()) => return Err(malformed()),
    };
    let Some(after_open) = body.strip_prefix("<project") else {
        return Err(malformed());
    };
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<project .../>`: no repositories.
        return Ok(Vec::new());
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</project>") else {
        return Err(malformed());
    };
    if !inner[close + "</project>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut repositories = Vec::new();
    for element in xml_child_elements(inner, "repository")? {
        let (tag, content) = xml_split_element(element, "repository");
        let name = xml_attr_non_empty(tag, "name").ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS project meta response: <repository> without name",
            )
        })?;
        let arch = xml_elements(content, "arch")?
            .iter()
            .map(|arch| {
                let (_, text) = xml_split_element(arch, "arch");
                xml_unescape(text.trim())
            })
            .filter(|text| !text.is_empty())
            .collect();
        let mut paths = Vec::new();
        for path in xml_elements(content, "path")? {
            let (path_tag, _) = xml_split_element(path, "path");
            let project = xml_attr_non_empty(path_tag, "project").ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    "unexpected OBS project meta response: <path> without project",
                )
            })?;
            // A path without a repository attribute names no
            // (project, repository) pair, so the response is
            // malformed, not a path to keep: the server always
            // names the repository a path builds against.
            let repository = xml_attr_non_empty(path_tag, "repository").ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    "unexpected OBS project meta response: <path> without repository",
                )
            })?;
            paths.push((project, repository));
        }
        let mut downloads = Vec::new();
        for download in xml_elements(content, "download")? {
            let (download_tag, download_inner) = xml_split_element(download, "download");
            let url = match xml_attr(download_tag, "url") {
                Some(url) if !url.is_empty() => url,
                Some(_) => {
                    return Err(GeectlError::new(
                        ErrorKind::ServerError,
                        "unexpected OBS project meta response: <download> with empty url",
                    ));
                }
                None => {
                    return Err(GeectlError::new(
                        ErrorKind::ServerError,
                        "unexpected OBS project meta response: <download> without url",
                    ));
                }
            };
            let master_elements = xml_elements(download_inner, "master")?;
            // `<master>`, `<archfilter>` and `<pubkey>` are singular
            // in the typed model, which keeps the first of each: a
            // second one is server-accepted state the parse keeps
            // readable, and the rewrite refuses it instead of
            // dropping it (see `unmodeled_repository_content`).
            let master = master_elements
                .into_iter()
                .map(|master_element| {
                    let (master_tag, _) = xml_split_element(master_element, "master");
                    match xml_attr(master_tag, "url") {
                        Some(url) if !url.is_empty() => Ok(DownloadMaster {
                            url,
                            sslfingerprint: xml_attr_non_empty(master_tag, "sslfingerprint"),
                        }),
                        Some(_) => Err(GeectlError::new(
                            ErrorKind::ServerError,
                            "unexpected OBS project meta response: <master> with empty url",
                        )),
                        None => Err(GeectlError::new(
                            ErrorKind::ServerError,
                            "unexpected OBS project meta response: <master> without url",
                        )),
                    }
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .next();
            downloads.push(RepositoryDownload {
                arch: xml_attr_non_empty(download_tag, "arch"),
                url,
                repotype: xml_attr_non_empty(download_tag, "repotype"),
                archfilter: xml_element_text(download_inner, "archfilter")
                    .filter(|value| !value.is_empty()),
                pubkey: xml_element_text(download_inner, "pubkey")
                    .filter(|value| !value.is_empty()),
                master,
            });
        }
        let mut release_targets = Vec::new();
        for target in xml_elements(content, "releasetarget")? {
            let (target_tag, _) = xml_split_element(target, "releasetarget");
            let project = xml_attr_non_empty(target_tag, "project").ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    "unexpected OBS project meta response: <releasetarget> without project",
                )
            })?;
            release_targets.push(ReleaseTarget {
                project,
                repository: xml_attr_non_empty(target_tag, "repository"),
                trigger: xml_attr_non_empty(target_tag, "trigger"),
            });
        }
        let mut hostsystem = Vec::new();
        for host_element in xml_elements(content, "hostsystem")? {
            let (host_tag, _) = xml_split_element(host_element, "hostsystem");
            let project = xml_attr_non_empty(host_tag, "project").ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    "unexpected OBS project meta response: <hostsystem> without project",
                )
            })?;
            let repository = xml_attr_non_empty(host_tag, "repository").ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    "unexpected OBS project meta response: <hostsystem> without repository",
                )
            })?;
            hostsystem.push((project, repository));
        }
        repositories.push(Repository {
            name,
            arch,
            paths,
            rebuild: xml_attr_non_empty(tag, "rebuild"),
            block: xml_attr_non_empty(tag, "block"),
            linkedbuild: xml_attr_non_empty(tag, "linkedbuild"),
            downloads,
            release_targets,
            hostsystem,
        });
    }
    Ok(repositories)
}

/// Render a `<repository>` element for `repository`: its name,
/// rebuild, block, and linkedbuild flags, one `<arch>` per
/// architecture, one `<path>` per rebuild source, its downloads,
/// release targets, and host system. Values are escaped, so a value
/// carrying XML syntax cannot break out of the element.
pub(crate) fn repository_element(repository: &Repository) -> String {
    let mut out = String::new();
    out.push_str("<repository name=\"");
    out.push_str(&xml_escape(&repository.name));
    out.push('"');
    // An empty value renders as nothing: the parser reads an empty
    // attribute as absent, so rendering one would not round-trip.
    if let Some(rebuild) = repository
        .rebuild
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        out.push_str(" rebuild=\"");
        out.push_str(&xml_escape(rebuild));
        out.push('"');
    }
    if let Some(block) = repository
        .block
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        out.push_str(" block=\"");
        out.push_str(&xml_escape(block));
        out.push('"');
    }
    if let Some(linkedbuild) = repository
        .linkedbuild
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        out.push_str(" linkedbuild=\"");
        out.push_str(&xml_escape(linkedbuild));
        out.push('"');
    }
    out.push('>');
    for arch in &repository.arch {
        out.push_str("<arch>");
        out.push_str(&xml_escape(arch));
        out.push_str("</arch>");
    }
    for (project, repo) in &repository.paths {
        out.push_str("<path project=\"");
        out.push_str(&xml_escape(project));
        out.push_str("\" repository=\"");
        out.push_str(&xml_escape(repo));
        out.push_str("\"/>");
    }
    for download in &repository.downloads {
        out.push_str(&download_element(download));
    }
    for target in &repository.release_targets {
        out.push_str("<releasetarget project=\"");
        out.push_str(&xml_escape(&target.project));
        out.push('"');
        if let Some(repo) = target
            .repository
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            out.push_str(" repository=\"");
            out.push_str(&xml_escape(repo));
            out.push('"');
        }
        if let Some(trigger) = target.trigger.as_deref().filter(|value| !value.is_empty()) {
            out.push_str(" trigger=\"");
            out.push_str(&xml_escape(trigger));
            out.push('"');
        }
        out.push_str("/>");
    }
    for (project, repo) in &repository.hostsystem {
        out.push_str("<hostsystem project=\"");
        out.push_str(&xml_escape(project));
        out.push_str("\" repository=\"");
        out.push_str(&xml_escape(repo));
        out.push_str("\"/>");
    }
    out.push_str("</repository>");
    out
}

/// Render a `<download>` element for `download`: its arch, url, and
/// repotype attributes, with its archfilter, master, and pubkey
/// children. A download without children is self-closing. Values
/// are escaped, so a value carrying XML syntax cannot break out of
/// the element.
fn download_element(download: &RepositoryDownload) -> String {
    let mut out = String::new();
    out.push_str("<download");
    if let Some(arch) = download.arch.as_deref().filter(|value| !value.is_empty()) {
        out.push_str(" arch=\"");
        out.push_str(&xml_escape(arch));
        out.push('"');
    }
    out.push_str(" url=\"");
    out.push_str(&xml_escape(&download.url));
    out.push('"');
    if let Some(repotype) = download
        .repotype
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        out.push_str(" repotype=\"");
        out.push_str(&xml_escape(repotype));
        out.push('"');
    }
    // An empty text child renders as nothing: the parser reads an
    // empty `<archfilter>` or `<pubkey>` as absent, so rendering one
    // would not round-trip. Empty attribute values render as nothing
    // for the same reason.
    let archfilter = download
        .archfilter
        .as_deref()
        .filter(|value| !value.is_empty());
    let pubkey = download.pubkey.as_deref().filter(|value| !value.is_empty());
    let has_children = archfilter.is_some() || download.master.is_some() || pubkey.is_some();
    if !has_children {
        out.push_str("/>");
        return out;
    }
    out.push('>');
    if let Some(archfilter) = archfilter {
        out.push_str("<archfilter>");
        out.push_str(&xml_escape(archfilter));
        out.push_str("</archfilter>");
    }
    if let Some(master) = &download.master {
        out.push_str("<master url=\"");
        out.push_str(&xml_escape(&master.url));
        out.push('"');
        if let Some(sslfingerprint) = master
            .sslfingerprint
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            out.push_str(" sslfingerprint=\"");
            out.push_str(&xml_escape(sslfingerprint));
            out.push('"');
        }
        out.push_str("/>");
    }
    if let Some(pubkey) = pubkey {
        out.push_str("<pubkey>");
        out.push_str(&xml_escape(pubkey));
        out.push_str("</pubkey>");
    }
    out.push_str("</download>");
    out
}

/// Validate a repository for adding: the name and every architecture
/// and path component must be an OBS name, the repository's settings
/// must be valid, and the repository must carry at least one
/// architecture and one path — like `osc repo add`, which requires
/// `--arch` and `--path`. A repository with no path builds against
/// nothing, so it is not addable through this operation.
pub(crate) fn validate_repository(repository: &Repository) -> Result<(), GeectlError> {
    if !valid_obs_name(&repository.name) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid OBS repository name: {:?}", repository.name),
        ));
    }
    if repository.arch.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!(
                "invalid OBS repository {:?}: no architectures",
                repository.name
            ),
        ));
    }
    let mut seen_arches = Vec::new();
    for arch in &repository.arch {
        if !valid_obs_name(arch) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS architecture name: {arch:?}"),
            ));
        }
        let lower = arch.to_lowercase();
        if seen_arches.contains(&lower) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "duplicate architecture {arch:?} in repository {:?}",
                    repository.name
                ),
            ));
        }
        seen_arches.push(lower);
    }
    if repository.paths.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid OBS repository {:?}: no paths", repository.name),
        ));
    }
    for (project, repo) in &repository.paths {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        if !valid_obs_name(repo) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS repository name: {repo:?}"),
            ));
        }
    }
    validate_repository_settings(repository)?;
    Ok(())
}

/// Validate the settings of one repository beyond name,
/// architectures, and paths: the rebuild, block, and linkedbuild
/// flags carry the server's enumerations (blank accepted, like the
/// API model),
/// and the download, release-target, and hostsystem settings carry
/// the server's rules — a download's arch is required, one of the
/// repository's architectures, and unique in the repository; a
/// download's url carries a scheme and its repotype is one of the
/// server's types; a release target names its repository; a master
/// url carries a scheme when present. A missing optional value is
/// never an error — only a present value is checked.
pub(crate) fn validate_repository_settings(repository: &Repository) -> Result<(), GeectlError> {
    let invalid = |what: &str| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid repository {:?}: {what}", repository.name),
        )
    };
    if let Some(rebuild) = &repository.rebuild {
        if !matches!(rebuild.as_str(), "" | "transitive" | "direct" | "local") {
            return Err(invalid(&format!(
                "rebuild {rebuild:?} is not one of blank, transitive, direct, local"
            )));
        }
    }
    if let Some(linkedbuild) = &repository.linkedbuild {
        if !matches!(
            linkedbuild.as_str(),
            "" | "off" | "localdep" | "all" | "alldirect" | "alldirect_or_localdep"
        ) {
            return Err(invalid(&format!(
                "linkedbuild {linkedbuild:?} is not one of blank, off, localdep, all, alldirect, alldirect_or_localdep"
            )));
        }
    }
    if let Some(block) = &repository.block {
        if !matches!(block.as_str(), "" | "all" | "local" | "never") {
            return Err(invalid(&format!(
                "block {block:?} is not one of blank, all, local, never"
            )));
        }
    }
    let mut download_arches = Vec::new();
    for download in &repository.downloads {
        let arch = match &download.arch {
            Some(arch) => arch,
            None => return Err(invalid("download arch is required")),
        };
        if !valid_obs_name(arch) {
            return Err(invalid(&format!(
                "download arch {arch:?} is not a valid name"
            )));
        }
        if !repository.arch.iter().any(|a| a == arch) {
            return Err(invalid(&format!(
                "download arch {arch:?} is not one of the repository's architectures"
            )));
        }
        // Exact match: a download arch that differs only by case
        // from a repository architecture is rejected by the
        // membership check above, and a repository listing both
        // casings is rejected by the repository-arch check, so no
        // case-folding is needed here.
        if download_arches.contains(arch) {
            return Err(invalid(&format!(
                "download arch {arch:?} is used more than once"
            )));
        }
        download_arches.push(arch.clone());
        if !has_url_scheme(&download.url) {
            return Err(invalid(&format!(
                "download arch {arch:?} has a url with no scheme"
            )));
        }
        let repotype = match &download.repotype {
            Some(repotype) => repotype,
            None => return Err(invalid("download repotype is required")),
        };
        if !DOWNLOAD_REPOTYPES.contains(&repotype.as_str()) {
            return Err(invalid(&format!(
                "download repotype {repotype:?} is not one of {}",
                DOWNLOAD_REPOTYPES.join(", ")
            )));
        }
        if let Some(master) = &download.master {
            if !has_url_scheme(&master.url) {
                return Err(invalid(&format!(
                    "download arch {arch:?} has a master url with no scheme"
                )));
            }
        }
    }
    for target in &repository.release_targets {
        if !valid_obs_name(&target.project) {
            return Err(invalid(&format!(
                "releasetarget project {:?} is not a valid name",
                target.project
            )));
        }
        let repo = match &target.repository {
            Some(repo) => repo,
            None => return Err(invalid("releasetarget repository is required")),
        };
        if !valid_obs_name(repo) {
            return Err(invalid(&format!(
                "releasetarget repository {repo:?} is not a valid name"
            )));
        }
    }
    for (project, repo) in &repository.hostsystem {
        if !valid_obs_name(project) {
            return Err(invalid(&format!(
                "hostsystem project {project:?} is not a valid name"
            )));
        }
        if !valid_obs_name(repo) {
            return Err(invalid(&format!(
                "hostsystem repository {repo:?} is not a valid name"
            )));
        }
    }
    Ok(())
}

/// The download repository types the server accepts.
const DOWNLOAD_REPOTYPES: [&str; 7] =
    ["rpmmd", "susetags", "deb", "arch", "mdk", "registry", "apk"];

/// A url carries a scheme when it starts with a URI scheme
/// (RFC 3986: a letter followed by letters, digits, `+`, or `-`),
/// then a colon, then at least one character, and holds no control
/// characters. A `.` is excluded: no scheme uses one, and a dot in
/// that position is a host:port typo like `example.com:8080`.
/// Deliberately not the server's check: the server accepts a bare
/// `x:` and rejects a legal `svn+ssh:` scheme, while this check
/// follows the scheme syntax in both directions.
fn has_url_scheme(url: &str) -> bool {
    let Some(colon) = url.find(':') else {
        return false;
    };
    let scheme = &url[..colon];
    let rest = &url[colon + 1..];
    !scheme.is_empty()
        && scheme
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-'))
        && !rest.is_empty()
        && url.chars().all(|c| !c.is_control())
}

/// Add `repository` to the project meta document `doc`: a
/// `<repository>` element appended after the existing children, and a
/// publish-disable entry when `disable_publish` is set. A repository
/// with the same name already there is an error, not a replacement —
/// like `osc repo add`, which refuses a duplicate. Unknown elements
/// survive untouched: the edit only appends.
fn add_repository_to_meta(
    doc: &mut XmlDoc,
    repository: &Repository,
    disable_publish: bool,
) -> Result<(), GeectlError> {
    if doc.has_element_with_attr("repository", "name", &repository.name) {
        return Err(GeectlError::new(
            ErrorKind::AlreadyExists,
            format!(
                "repository {:?} already exists in project meta",
                repository.name
            ),
        ));
    }
    doc.append_element(&repository_element(repository))?;
    if disable_publish {
        set_publish_disabled(doc, &repository.name)?;
    }
    Ok(())
}

/// Remove the repository named `name` from the project meta document
/// `doc`, along with its publish-disable entry when one is set.
/// Returns true when anything was removed: a repository that is not
/// there is not an error, like `osc repo remove`, which tolerates a
/// nonexistent name.
fn remove_repository_from_meta(doc: &mut XmlDoc, name: &str) -> Result<bool, GeectlError> {
    let removed = doc.remove_elements_with_attr("repository", "name", name);
    let publish_removed = clear_publish_disabled(doc, name)?;
    Ok(removed || publish_removed)
}

/// Ensure the project meta document `doc` carries a publish-disable
/// entry for the repository named `name`: `<publish><disable
/// repository="name"/></publish>`, added to an existing `<publish>`
/// when there is one. An entry already there is left alone.
fn set_publish_disabled(doc: &mut XmlDoc, name: &str) -> Result<(), GeectlError> {
    let Some(publish) = doc.child("publish") else {
        return doc.replace_or_insert_element(
            "publish",
            &format!(
                "<publish><disable repository=\"{}\"/></publish>",
                xml_escape(name)
            ),
        );
    };
    let mut publish_doc = XmlDoc::parse(&publish)?;
    if publish_doc.has_element_with_attr("disable", "repository", name) {
        return Ok(());
    }
    publish_doc.append_element(&format!("<disable repository=\"{}\"/>", xml_escape(name)))?;
    doc.replace_or_insert_element("publish", &publish_doc.render())
}

/// Drop the publish-disable entries for the repository named `name`
/// from the project meta document `doc`. True when an entry was
/// dropped. A `<publish>` left with no content at all is dropped
/// altogether, so removing the last disabled repository leaves no
/// empty flag section behind. A comment is content, so a commented
/// section survives.
fn clear_publish_disabled(doc: &mut XmlDoc, name: &str) -> Result<bool, GeectlError> {
    let Some(publish) = doc.child("publish") else {
        return Ok(false);
    };
    let mut publish_doc = XmlDoc::parse(&publish)?;
    if !publish_doc.remove_elements_with_attr("disable", "repository", name) {
        return Ok(false);
    }
    if publish_doc.has_content() {
        doc.replace_or_insert_element("publish", &publish_doc.render())?;
    } else {
        doc.remove_elements("publish");
    }
    Ok(true)
}

/// Parse a `<directory>...</directory>` document into the `name`
/// attributes of its `<entry>` children. A document that is not a
/// directory is a server error, never an empty list: an empty
/// directory is the "nothing matches" answer.
pub(crate) fn parse_directory_entry_names(body: &str) -> Result<Vec<String>, GeectlError> {
    let body = body.trim();
    // A malformed response still fails closed, but the error carries a
    // bounded preview of what was received, so the failure is
    // diagnosable instead of a bare "not a directory document". The
    // preview is the debug channel's redacting preview: credential
    // values are masked, binary bodies are never dumped, and the text
    // is capped at DEBUG_BODY_CAP bytes.
    let malformed = || {
        let preview = crate::debug::format_body_preview(body);
        let received = if preview.is_empty() {
            "<empty response>".to_string()
        } else {
            preview
        };
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected OBS response: not a <directory> document; received: {received}"),
        )
    };
    // The server may emit an XML prolog (`<?xml version=\"1.0\"?>`)
    // before the directory element; skip it. The prolog must
    // terminate (`?>`) — an unterminated prolog is malformed, not a
    // missing directory.
    let body = if let Some(after) = body.strip_prefix("<?xml") {
        let first = after.chars().next();
        if !matches!(first, Some(c) if c.is_whitespace() || c == '?') {
            return Err(malformed());
        }
        let Some(end) = after.find("?>") else {
            return Err(malformed());
        };
        after[end + 2..].trim_start()
    } else {
        body
    };
    let Some(after_open) = body.strip_prefix("<directory") else {
        return Err(malformed());
    };
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = find_tag_end(after_open) else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<directory/>`: no entries.
        return Ok(Vec::new());
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</directory>") else {
        return Err(malformed());
    };
    if !inner[close + "</directory>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut names = Vec::new();
    for element in xml_elements(inner, "entry")? {
        let (tag, _) = xml_split_element(element, "entry");
        if let Some(name) = xml_attr(tag, "name") {
            names.push(name);
        }
    }
    Ok(names)
}

fn xml_attr_raw(element: &str, name: &str) -> Option<String> {
    // Quote-aware: a `name=` inside an earlier attribute's value
    // (e.g. `details="x dirty=y"`) must not match. The scan tracks
    // whether it is inside a quoted value and only matches outside
    // one. The match position is always an ASCII byte (the first byte
    // of the ASCII needle), so the slicing below stays on UTF-8
    // boundaries. Whitespace around `=` is allowed: `name = "value"`
    // is the same attribute. A malformed occurrence (an unquoted
    // value) is skipped, not fatal — the attribute may still follow.
    if name.is_empty() {
        return None;
    }
    let bytes = element.as_bytes();
    let first = name.as_bytes()[0];
    let mut in_quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = in_quote {
            if b == q {
                in_quote = None;
            }
            i += 1;
            continue;
        }
        if b == b'"' || b == b'\'' {
            in_quote = Some(b);
            i += 1;
            continue;
        }
        if b == first
            && element[i..].starts_with(name)
            && (i == 0 || bytes[i - 1].is_ascii_whitespace())
        {
            // The name must be followed by `=`, with optional
            // whitespace around it.
            let mut j = i + name.len();
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'=' {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                    let quote = bytes[j];
                    let after = &element[j + 1..];
                    let close = after.find(quote as char)?;
                    return Some(xml_unescape(&after[..close]));
                }
            }
        }
        i += 1;
    }
    None
}

/// The attribute value, or None when the attribute is absent. An
/// empty value reads as present: only the repository parse collapses
/// an empty value to absent (see [`xml_attr_non_empty`]).
pub(crate) fn xml_attr(element: &str, name: &str) -> Option<String> {
    xml_attr_raw(element, name)
}

/// The attribute value, or None when the attribute is absent or
/// empty. Repository parsing only: the modeled Repository treats a
/// blank flag or optional value as unset, and rendering drops empty
/// values, so the parse must read them as absent to round-trip.
/// Every other caller reads through [`xml_attr`], where an empty
/// value stays present. A required repository attribute whose empty
/// value fails closed reads through [`xml_attr`] too: the empty case
/// errors, so no empty value can round-trip.
fn xml_attr_non_empty(element: &str, name: &str) -> Option<String> {
    xml_attr_raw(element, name).filter(|value| !value.is_empty())
}

/// The text between `<name>` and `</name>` in `body` (first
/// occurrence), trimmed and XML-unescaped. The opening tag may carry
/// attributes (`<name lang="en">`); a `<namex>` prefix name is
/// skipped, not read as `<name>`. None when the element is absent or
/// unterminated. A self-closing `<name/>` carries no text, so it
/// reads as None.
fn xml_element_text(body: &str, name: &str) -> Option<String> {
    let open_marker = format!("<{name}");
    let close_marker = format!("</{name}>");
    let mut offset = 0;
    while offset < body.len() {
        let rest = &body[offset..];
        let pos = rest.find(&open_marker)?;
        let after_tag = &rest[pos + open_marker.len()..];
        let first = after_tag.chars().next()?;
        if !(first.is_whitespace() || first == '/' || first == '>') {
            offset += pos + open_marker.len();
            continue;
        }
        let end = find_tag_end(after_tag)?;
        let tag = &after_tag[..end];
        if tag.trim_end().ends_with('/') {
            // Self-closing: no text.
            return None;
        }
        let start = offset + pos + open_marker.len() + end + 1;
        let end = body[start..].find(&close_marker)?;
        let end = end + start;
        return Some(xml_unescape(body[start..end].trim()));
    }
    None
}

/// Split a full element text (`<name ...>...</name>` or `<name .../>`,
/// as returned by [`xml_elements`]) into its opening tag (without the
/// brackets) and its inner content. The inner content is empty for a
/// self-closing element.
pub(crate) fn xml_split_element<'a>(element: &'a str, name: &str) -> (&'a str, &'a str) {
    let after_lt = element.strip_prefix('<').unwrap_or(element);
    let Some(end) = find_tag_end(after_lt) else {
        return (after_lt, "");
    };
    let tag = &after_lt[..end];
    if tag.trim_end().ends_with('/') {
        return (tag, "");
    }
    let inner = &after_lt[end + 1..];
    let close_marker = format!("</{name}>");
    let inner = inner.strip_suffix(&close_marker).unwrap_or(inner);
    (tag, inner)
}

/// Parse an unsigned attribute value: None when the attribute is
/// absent, an error when it is present but not a number.
fn xml_attr_number<T>(tag: &str, name: &str) -> Result<Option<T>, GeectlError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match xml_attr_raw(tag, name) {
        None => Ok(None),
        Some(value) => value.parse::<T>().map(Some).map_err(|_| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS response: attribute {name} is not a number"),
            )
        }),
    }
}

/// Parse a required unsigned attribute value: an error when it is
/// absent or not a number.
fn xml_attr_number_required<T>(tag: &str, name: &str) -> Result<T, GeectlError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    xml_attr_number(tag, name)?.ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected OBS response: missing attribute {name}"),
        )
    })
}

impl<'a> ObsClient<'a> {
    /// Refuse a credentialed build read without OBS Basic
    /// credentials, before any request: every `/build` endpoint
    /// requires Basic authentication, so a call without credentials
    /// would 401 — a failed lookup must fail closed, never read as
    /// "no build state".
    fn require_build_credentials(&self) -> Result<(), GeectlError> {
        if self.has_auth() {
            return Ok(());
        }
        Err(GeectlError::new(
            ErrorKind::ConfigError,
            "reading build state requires OBS credentials (GEECTL_OBS_APIKEY[_FILE] or GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD[_FILE])",
        ))
    }

    /// The build-results URL for `project` and `query`: `GET
    /// /build/<project>/_result` with the query's filters, in a fixed
    /// order (package, repositories, architectures, code, multibuild,
    /// locallink, lastbuild, oldstate).
    fn results_url(&self, project: &str, query: &ResultsQuery) -> String {
        let mut pairs: Vec<(&str, &str)> = Vec::new();
        if let Some(package) = &query.package {
            pairs.push(("package", package));
        }
        for repository in &query.repositories {
            pairs.push(("repository", repository));
        }
        for arch in &query.arches {
            pairs.push(("arch", arch));
        }
        if let Some(code) = &query.code {
            pairs.push(("code", code));
        }
        if query.multibuild {
            pairs.push(("multibuild", "1"));
        }
        if query.locallink {
            pairs.push(("locallink", "1"));
        }
        if query.lastbuild {
            pairs.push(("lastbuild", "1"));
        }
        if let Some(oldstate) = &query.oldstate {
            pairs.push(("oldstate", oldstate));
        }
        build_url(&self.base, &["build", project, "_result"], &pairs)
    }

    /// The build results for `project`: one entry per
    /// repository/architecture cell, each with the per-package build
    /// codes. The query filters by package, repository, architecture,
    /// or package build code; `multibuild` + `locallink` expand
    /// multibuild flavors into their own statuses, which is the
    /// per-package form. A package-filtered query omits a filtered
    /// package with no status instead of reporting it `unknown` (a
    /// server-side typo in `getresult`), so a never-scheduled
    /// package silently drops out of a filtered read. A filtered
    /// [`ObsClient::wait_for_results`] does not treat that absence as
    /// settled: it keeps polling until the package appears or the
    /// deadline passes.
    /// Requires OBS credentials. That is a client policy, not
    /// something the endpoint forces: the endpoint allows anonymous
    /// reads when the instance permits them, so without credentials
    /// the call fails closed instead of reading anonymously.
    /// A failed lookup is an error, never an empty matrix.
    pub fn results(
        &self,
        project: &str,
        query: &ResultsQuery,
    ) -> Result<BuildResults, GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        if let Some(package) = &query.package {
            if !valid_obs_name(package) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS package name: {package:?}"),
                ));
            }
        }
        for (label, values) in [
            ("repository", &query.repositories),
            ("architecture", &query.arches),
        ] {
            for value in values {
                if !valid_obs_name(value) {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!("invalid OBS {label} name: {value:?}"),
                    ));
                }
            }
        }
        if let Some(oldstate) = &query.oldstate {
            if oldstate.is_empty() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "invalid OBS results state hash: empty oldstate".to_string(),
                ));
            }
        }
        self.require_build_credentials()?;
        let url = self.results_url(project, query);
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_build_results(&body)
    }

    /// Poll the build results for `project` until they settle — no
    /// cell dirty, no cell or package in a waiting state (`blocked`,
    /// `scheduled`, `dispatching`, `building`, `signing`, `finished`)
    /// — or `deadline` passes. With a package filter, the package
    /// must also appear in the matrix: a never-scheduled package drops
    /// out of a filtered read, and that absence is not settled, so the
    /// poll keeps going until the package appears or the deadline
    /// passes. The poll reads the matrix every two seconds (less when
    /// the deadline is nearer, so a sleep never overshoots it); it
    /// deliberately does not send `oldstate`, even though
    /// the server holds an unchanged `oldstate` reply for up to five
    /// minutes: past the transport's 30-second read timeout, so an
    /// `oldstate` poll would time out instead of settling. Plain
    /// polling keeps the deadline honest; use
    /// `ResultsQuery.oldstate` with [`ObsClient::results`] directly
    /// for a hand-rolled long poll. On a deadline the error is a
    /// timeout: call [`ObsClient::results`] again for the current
    /// state instead of assuming.
    pub fn wait_for_results(
        &self,
        project: &str,
        query: &ResultsQuery,
        deadline: std::time::Duration,
    ) -> Result<BuildResults, GeectlError> {
        // Plain polling only: a caller-set `oldstate` would make the
        // server hold an unchanged reply for up to five minutes, past
        // the transport's read timeout.
        let query = ResultsQuery {
            oldstate: None,
            ..query.clone()
        };
        let start = std::time::Instant::now();
        loop {
            let fetched = self.results(project, &query)?;
            if results_settled(&fetched, query.package.as_deref()) {
                return Ok(fetched);
            }
            if start.elapsed() >= deadline {
                return Err(GeectlError::new(
                    ErrorKind::Timeout,
                    format!("build results for {project} did not settle within {deadline:?}"),
                ));
            }
            // Sleep at most two seconds, and never past the deadline:
            // a fixed sleep would overshoot a near deadline.
            let remaining = deadline.saturating_sub(start.elapsed());
            std::thread::sleep(std::cmp::min(std::time::Duration::from_secs(2), remaining));
        }
    }

    /// Validate the coordinates of a build read (project, package,
    /// repository, architecture): each must be a valid OBS name, so a
    /// `/`, `?`, or quote can never rewrite the URL. The package is
    /// optional: the repository-wide reads take None.
    fn validate_build_coordinates(
        project: &str,
        package: Option<&str>,
        repository: &str,
        arch: &str,
    ) -> Result<(), GeectlError> {
        let mut names: Vec<(&str, &str)> = vec![
            ("project", project),
            ("repository", repository),
            ("architecture", arch),
        ];
        if let Some(package) = package {
            names.push(("package", package));
        }
        for (label, value) in names {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        Ok(())
    }

    /// The build log for `package` in `repository`/`arch`: the raw
    /// log text. `nostream=1` reads the whole log at once (no
    /// streaming); the query selects the slice. None when there is no
    /// log (404): the package was never built, or the package/project
    /// does not exist — the server answers 404 for both, so the caller
    /// checks the package exists when the distinction matters.
    /// With `query.strip_time` the leading `[timestamp] ` is stripped
    /// from each line client-side before the text is returned.
    /// Requires OBS credentials. The log is capped by the
    /// transport's body limit, like every read.
    pub fn build_log(
        &self,
        project: &str,
        package: &str,
        repository: &str,
        arch: &str,
        query: &BuildLogQuery,
    ) -> Result<Option<String>, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        if query.last && query.lastsucceeded {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "--last and --lastsucceeded cannot be combined: pass at most one",
            ));
        }
        self.require_build_credentials()?;
        let start = query.start.map(|offset| offset.to_string());
        let mut pairs: Vec<(&str, &str)> = vec![("nostream", "1")];
        if let Some(start) = &start {
            pairs.push(("start", start));
        }
        if query.last {
            pairs.push(("last", "1"));
        }
        if query.lastsucceeded {
            pairs.push(("lastsucceeded", "1"));
        }
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, package, "_log"],
            &pairs,
        );
        match self.http.get_with(&url, &self.credentials) {
            Ok(body) => Ok(Some(if query.strip_time {
                strip_log_timestamps(&body)
            } else {
                body
            })),
            // A 404 means no log: never built, or the package/project
            // does not exist. Any other failure stays an error.
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The build status code for `package` in `repository`/`arch`
    /// (`succeeded`, `failed`, `unresolvable`, ...), or None when the
    /// package has no status: the server answers `code="unknown"`
    /// with 200 when nothing is known, and 404 when the
    /// package/project does not exist. Requires OBS credentials.
    pub fn build_status(
        &self,
        project: &str,
        package: &str,
        repository: &str,
        arch: &str,
    ) -> Result<Option<String>, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        self.require_build_credentials()?;
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, package, "_status"],
            &[],
        );
        let body = match self.http.get_with(&url, &self.credentials) {
            Ok(body) => body,
            // A 404 means the package/project does not exist. Any
            // other failure stays an error.
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let code = parse_build_status(&body)?;
        // The server's "no status known" is `code="unknown"` with a
        // 200, not a 404: it reads as None, like the 404 above.
        if code == "unknown" {
            return Ok(None);
        }
        Ok(Some(code))
    }

    /// The running build job for `package` in `repository`/`arch`,
    /// or None when no job is running. Requires OBS credentials.
    /// A 404 is an error (unknown project/package): unlike the log and
    /// the status, a missing job reads as an empty jobstatus element,
    /// not a 404, so a 404 here names something that does not exist.
    pub fn job_status(
        &self,
        project: &str,
        package: &str,
        repository: &str,
        arch: &str,
    ) -> Result<Option<JobStatus>, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        self.require_build_credentials()?;
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, package, "_jobstatus"],
            &[],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_job_status(&body)
    }

    /// The buildinfo for `package` in `repository`/`arch`: the error
    /// blocking the build, if any, and the dependencies the build
    /// pulls in. An unresolvable build surfaces its missing
    /// dependencies as the error, as a comma-separated list. Requires
    /// OBS credentials. A 404 is an error (unknown
    /// project/package): a buildinfo with no error and no dependencies
    /// reads as an empty document, not a 404.
    pub fn buildinfo(
        &self,
        project: &str,
        package: &str,
        repository: &str,
        arch: &str,
    ) -> Result<BuildInfo, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        self.require_build_credentials()?;
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, package, "_buildinfo"],
            &[],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_buildinfo(&body)
    }

    /// Rebuild package `package` in `project`/`repository`/`arch` on OBS
    /// (`POST /build/<project>?cmd=rebuild&repository=<repository>&arch=<arch>&package=<package>`).
    /// Requires credentials; without them the call fails loudly before
    /// any request. In dry-run the rebuild is recorded in the plan and
    /// nothing is rebuilt — the guard runs before the credentials
    /// check, like `create_package`.
    ///
    /// An agent rebuilds a package to rerun a build that failed for a
    /// transient reason or to pick up a changed dependency, without
    /// touching the sources.
    ///
    /// Success is the `<status code="ok"/>` body; any other body is a
    /// server error, never success.
    pub fn rebuild_package(
        &self,
        project: &str,
        repository: &str,
        arch: &str,
        package: &str,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        self.build_control(
            project,
            repository,
            arch,
            package,
            "rebuild",
            "rebuild package",
            dry_run,
        )
    }

    /// Restart the build of package `package` in
    /// `project`/`repository`/`arch` on OBS (`POST
    /// /build/<project>?cmd=restartbuild&repository=<repository>&arch=<arch>&package=<package>`).
    /// Requires credentials; without them the call fails loudly before
    /// any request. In dry-run the restart is recorded in the plan and
    /// nothing is restarted — the guard runs before the credentials
    /// check, like `create_package`.
    ///
    /// An agent restarts a build that is stuck or running against
    /// stale inputs, without touching the sources.
    ///
    /// Success is the `<status code="ok"/>` body; any other body is a
    /// server error, never success.
    pub fn restart_build(
        &self,
        project: &str,
        repository: &str,
        arch: &str,
        package: &str,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        self.build_control(
            project,
            repository,
            arch,
            package,
            "restartbuild",
            "restart the build of",
            dry_run,
        )
    }

    /// Abort the build of package `package` in
    /// `project`/`repository`/`arch` on OBS (`POST
    /// /build/<project>?cmd=abortbuild&repository=<repository>&arch=<arch>&package=<package>`).
    /// Requires credentials; without them the call fails loudly before
    /// any request. In dry-run the abort is recorded in the plan and
    /// nothing is aborted — the guard runs before the credentials
    /// check, like `create_package`.
    ///
    /// An agent aborts a build that is no longer wanted, for example
    /// after superseding the request that triggered it.
    ///
    /// Success is the `<status code="ok"/>` body; any other body is a
    /// server error, never success.
    pub fn abort_build(
        &self,
        project: &str,
        repository: &str,
        arch: &str,
        package: &str,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        self.build_control(
            project,
            repository,
            arch,
            package,
            "abortbuild",
            "abort the build of",
            dry_run,
        )
    }

    /// Send a build-control command (`rebuild`, `restartbuild`,
    /// `abortbuild`) for one package: `POST
    /// /build/<project>?cmd=<cmd>&repository=<repository>&arch=<arch>&package=<package>`
    /// with an empty body. The package is a query parameter: omitting
    /// it would target every package in the repository/arch, but this
    /// op always passes one package.
    ///
    /// Success means the command was accepted. A rebuild queues a
    /// rebuild event for the package; a restart or abort is accepted
    /// even when nothing is building — the backend swallows the
    /// "not building" failure and still answers ok — so for those
    /// two Ok carries no build state.
    // Seven arguments: the four build coordinates, the command and
    // its wording, and the dry-run guard. A coordinates struct would
    // break the library's positional convention for exactly this
    // one op.
    #[allow(clippy::too_many_arguments)]
    fn build_control(
        &self,
        project: &str,
        repository: &str,
        arch: &str,
        package: &str,
        cmd: &str,
        action: &str,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        if !dry_run.allow(&format!(
            "{action} {project}/{package} ({repository}/{arch})"
        )) {
            return Ok(());
        }
        if !self.has_auth() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!(
                    "cannot {cmd} package {project}/{package} without OBS credentials (GEECTL_OBS_APIKEY[_FILE] or GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD[_FILE])"
                ),
            ));
        }
        let url = build_url(
            &self.base,
            &["build", project],
            &[
                ("cmd", cmd),
                ("repository", repository),
                ("arch", arch),
                ("package", package),
            ],
        );
        let body = self.http.post(&url, "", &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS {cmd} response for package {project}/{package}"),
            ))
        }
    }

    /// The build configuration for `project`/`repository`: the
    /// expanded `_buildconfig` the engine takes as `--dist`. The text
    /// passes through as-is — it is the build script's own format,
    /// not XML, so there is nothing to type here. Requires OBS Basic
    /// credentials. A 404 is an error (unknown project/repository).
    pub fn buildconfig(&self, project: &str, repository: &str) -> Result<String, GeectlError> {
        // No arch here: the buildconfig is per project/repository.
        // Validate the two names directly instead of borrowing the
        // coordinate validator with a dummy arch.
        for (label, value) in [("project", project), ("repository", repository)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        self.require_build_credentials()?;
        let url = build_url(
            &self.base,
            &["build", project, repository, "_buildconfig"],
            &[],
        );
        self.http.get_with(&url, &self.credentials)
    }

    /// The build history for `package` in `repository`/`arch`: the
    /// completed builds, newest first, up to `limit` entries. An empty
    /// history (never built) reads as an empty list. Requires OBS
    /// credentials. A 404 is an error (unknown project/package).
    pub fn build_history(
        &self,
        project: &str,
        package: &str,
        repository: &str,
        arch: &str,
        limit: Option<u32>,
    ) -> Result<Vec<BuildHistoryEntry>, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        self.require_build_credentials()?;
        let limit = limit.map(|n| n.to_string());
        let mut pairs: Vec<(&str, &str)> = Vec::new();
        if let Some(limit) = &limit {
            pairs.push(("limit", limit));
        }
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, package, "_history"],
            &pairs,
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_build_history(&body)
    }

    /// The job history for `repository`/`arch`: the build jobs that
    /// ran there, newest first, optionally limited to `package`,
    /// `limit` entries, and a result `code`. `lastfailures` is
    /// rejected: it is a special server mode returning per-package
    /// success-then-failure pairs, not a chronological history. Use
    /// it to check a green build is from the
    /// current source revision, not a stale rebuild: match the entry's
    /// `srcmd5` against the package's current revision. An empty
    /// history reads as an empty list. Requires OBS credentials.
    /// A 404 is an error (unknown project/repository/architecture).
    pub fn job_history(
        &self,
        project: &str,
        repository: &str,
        arch: &str,
        package: Option<&str>,
        limit: Option<u32>,
        code: Option<&str>,
    ) -> Result<Vec<JobHistoryEntry>, GeectlError> {
        Self::validate_build_coordinates(project, package, repository, arch)?;
        self.require_build_credentials()?;
        if let Some(code) = code {
            if code.is_empty() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "invalid OBS job history code: empty".to_string(),
                ));
            }
            if code == "lastfailures" {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "invalid OBS job history code: lastfailures is a special server mode, not a result filter".to_string(),
                ));
            }
        }
        let limit = limit.map(|n| n.to_string());
        let mut pairs: Vec<(&str, &str)> = Vec::new();
        if let Some(package) = package {
            pairs.push(("package", package));
        }
        if let Some(limit) = &limit {
            pairs.push(("limit", limit));
        }
        if let Some(code) = code {
            pairs.push(("code", code));
        }
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, "_jobhistory"],
            &pairs,
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_job_history(&body)
    }

    /// The built files for `package` in `repository`/`arch`: the
    /// binary RPMs plus the build artifacts (`_buildenv`,
    /// `_statistics`, `rpmlint.log`, the `.src.rpm`, `-debuginfo` and
    /// `-debugsource` when built). With `package` None, the whole
    /// repository's files (`_repository`). An empty list means no
    /// binaries: a never-built existing package answers 200 with an
    /// empty list. A 404 is an error (the package or project does not
    /// exist), never an empty list: a misspelled package must not read
    /// as "nothing built".
    /// Requires OBS credentials. A multibuild flavor travels as
    /// `pkg:flavor`.
    pub fn list_binaries(
        &self,
        project: &str,
        package: Option<&str>,
        repository: &str,
        arch: &str,
    ) -> Result<Vec<BinaryFile>, GeectlError> {
        Self::validate_build_coordinates(project, package, repository, arch)?;
        self.require_build_credentials()?;
        let package = package.unwrap_or("_repository");
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, package],
            &[],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_binary_list(&body)
    }

    /// Download one built file (`filename`) for `package` in
    /// `repository`/`arch`: the raw bytes. Use it for a binary RPM or
    /// a build artifact by name (`rpmlint.log`). The download is
    /// capped at 10 MiB by the transport's body limit, like every
    /// read: a file larger than 10 MiB is an error, not a truncated
    /// download. Requires OBS credentials. A 404 is an error
    /// (the file does not exist).
    pub fn download_binary(
        &self,
        project: &str,
        package: &str,
        repository: &str,
        arch: &str,
        filename: &str,
    ) -> Result<Vec<u8>, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        if !valid_obs_filename(filename) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS binary filename: {filename:?}"),
            ));
        }
        self.require_build_credentials()?;
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, package, filename],
            &[],
        );
        self.http.get_bytes(&url, &self.credentials)
    }

    /// Download one built file (`filename`) for `package` in
    /// `repository`/`arch`, streaming the body to `dest` instead of
    /// buffering it: the byte-download counterpart of
    /// [`ObsClient::download_binary`], without the transport's 10 MiB
    /// body cap. Same validation and credentials; the file is created
    /// exclusively, and a failure mid-stream leaves a partial file
    /// for the caller to delete. Returns the bytes written.
    pub fn download_binary_to_file(
        &self,
        project: &str,
        package: &str,
        repository: &str,
        arch: &str,
        filename: &str,
        dest: &Path,
    ) -> Result<u64, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        if !valid_obs_filename(filename) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS binary filename: {filename:?}"),
            ));
        }
        self.require_build_credentials()?;
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, package, filename],
            &[],
        );
        self.http.get_to_file(&url, &self.credentials, dest)
    }

    /// Download one file from a published-repo mirror URL, streaming
    /// the body to `dest` instead of buffering it: the mirror
    /// download is always streamed, so a dep RPM is never held in
    /// memory whole. No credentials are sent — mirrors are public —
    /// and the download follows the mirror's redirect chain (see
    /// [`crate::http::HttpClient::get_to_file_mirror`]), single-shot
    /// with no retries. The body is capped at `max_bytes`: a body
    /// over the cap fails, and the caller falls back to the API. The
    /// file is created exclusively, and a failure mid-stream leaves
    /// a partial file for the caller to delete. A failure is the
    /// caller's to fall back from, never a reason to fail the build.
    pub(crate) fn download_mirror_to_file(
        &self,
        url: &str,
        dest: &Path,
        max_bytes: u64,
    ) -> Result<u64, GeectlError> {
        self.http
            .get_to_file_mirror(url, &Credentials::None, dest, max_bytes)
    }

    /// The versions of one binary in `repository`/`arch`: the
    /// server-reported name (`<binary>.<suffix>`, e.g. `foo.rpm`),
    /// the RPM header md5, and the size in KiB the server reports
    /// for it. Asking for the versions of a binary on a
    /// download-on-demand repository makes the server fetch the
    /// binary into the repository (the request blocks until the
    /// fetch completes) and reports the name it stored it under;
    /// a direct file download of a binary nobody has asked for 404s.
    /// A binary the repository does not carry is reported with an
    /// `error`, not a 404. Requires OBS credentials.
    pub fn binary_versions(
        &self,
        project: &str,
        repository: &str,
        arch: &str,
        binary: &str,
    ) -> Result<Vec<BinaryVersion>, GeectlError> {
        Self::validate_build_coordinates(project, None, repository, arch)?;
        if !valid_obs_name(binary) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS binary name: {binary:?}"),
            ));
        }
        self.require_build_credentials()?;
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, "_repository"],
            &[("view", "binaryversions"), ("binary", binary)],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_binary_version_list(&body)
    }

    /// The metadata of a built file (`filename`) for `package` in
    /// `repository`/`arch`: the RPM header fields plus the dependency
    /// lists. With `ext`, the `fileinfo_ext` view: each required
    /// dependency names the packages providing it, and each provided
    /// dependency names the packages requiring it. Requires OBS Basic
    /// credentials. A 404 is an error (the file does not exist).
    pub fn fileinfo(
        &self,
        project: &str,
        package: &str,
        repository: &str,
        arch: &str,
        filename: &str,
        ext: bool,
    ) -> Result<FileInfo, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        if !valid_obs_filename(filename) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS binary filename: {filename:?}"),
            ));
        }
        self.require_build_credentials()?;
        let view = if ext { "fileinfo_ext" } else { "fileinfo" };
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, package, filename],
            &[("view", view)],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_fileinfo(&body)
    }

    /// The build dependencies of the packages in `repository`/`arch`:
    /// which packages each package depends on for its build
    /// (whole-package dependencies, not individual binaries). With
    /// `package`, only that package's dependencies. Requires OBS Basic
    /// credentials. A 404 is an error (unknown
    /// project/repository/architecture). An empty list means the
    /// repository has no packages with recorded dependencies.
    pub fn dependson(
        &self,
        project: &str,
        repository: &str,
        arch: &str,
        package: Option<&str>,
    ) -> Result<Vec<PackageDeps>, GeectlError> {
        Self::validate_build_coordinates(project, package, repository, arch)?;
        self.require_build_credentials()?;
        let mut pairs: Vec<(&str, &str)> = vec![("view", "pkgnames")];
        if let Some(package) = package {
            pairs.push(("package", package));
        }
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, "_builddepinfo"],
            &pairs,
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_builddepinfo(&body)
    }

    /// The packages in `repository`/`arch` that depend on `package`
    /// for their build: a rebuild of `package` triggers a rebuild of
    /// each of them. The reverse of [`ObsClient::dependson`]: each
    /// entry's `dependencies` are the packages depending on the
    /// queried package, not the queried package's own dependencies.
    /// Requires OBS credentials. A 404 is an error (unknown
    /// project/repository/architecture).
    pub fn whatdependson(
        &self,
        project: &str,
        repository: &str,
        arch: &str,
        package: &str,
    ) -> Result<Vec<PackageDeps>, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        self.require_build_credentials()?;
        let url = build_url(
            &self.base,
            &["build", project, repository, arch, "_builddepinfo"],
            &[("view", "revpkgnames"), ("package", package)],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_builddepinfo(&body)
    }

    /// The worker ids that satisfy `constraints` for `package` in
    /// `repository`/`arch`: a query, not a build — it creates or
    /// modifies nothing, so it takes no dry-run guard. `constraints`
    /// is the package's constraints XML (`_constraints`), passed
    /// through as authored: there is no typed constraints model yet,
    /// so this is the one place a raw XML document crosses the
    /// public API. None posts an empty body, which the server
    /// accepts. An empty list means no worker satisfies the
    /// constraints (or no worker is registered). Requires OBS Basic
    /// credentials.
    pub fn check_constraints(
        &self,
        project: &str,
        package: &str,
        repository: &str,
        arch: &str,
        constraints: Option<&str>,
    ) -> Result<Vec<String>, GeectlError> {
        Self::validate_build_coordinates(project, Some(package), repository, arch)?;
        if let Some(constraints) = constraints {
            if constraints.is_empty() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "invalid OBS constraints: empty".to_string(),
                ));
            }
        }
        self.require_build_credentials()?;
        let url = build_url(
            &self.base,
            &["worker"],
            &[
                ("cmd", "checkconstraints"),
                ("project", project),
                ("package", package),
                ("repository", repository),
                ("arch", arch),
            ],
        );
        let body = self
            .http
            .post(&url, constraints.unwrap_or(""), &self.credentials)?;
        // The response is a `<directory>` of the matching workers,
        // one `<entry name="workerid"/>` each.
        parse_directory_entry_names(&body)
    }

    /// The repositories of `project`: their names, architectures, and
    /// the repositories they build against. A public read: project
    /// meta is public, so no credentials are needed. A 404 is an
    /// error (unknown project). An empty list means the project has no
    /// repositories.
    pub fn list_repositories(&self, project: &str) -> Result<Vec<Repository>, GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        let url = build_url(&self.base, &["public", "source", project, "_meta"], &[]);
        let body = self.http.get_with(&url, &Credentials::None)?;
        parse_repositories(&body)
    }

    /// Add `repository` to `project` (`PUT /source/<project>/_meta`
    /// with the project meta edited in place): the repository's name,
    /// architectures, and the repositories it builds against. A
    /// repository with the same name already there is an error, not a
    /// no-op — like `osc repo add`, which refuses a duplicate. With
    /// `disable_publish`, the added repository is also publish
    /// disabled. Requires OBS credentials. In dry-run the
    /// addition is recorded in the plan and nothing is written — the
    /// guard runs before the credentials check, like
    /// [`ObsClient::create_project`]. Returns true when the
    /// repository was added.
    pub fn add_repository(
        &self,
        project: &str,
        repository: &Repository,
        disable_publish: bool,
        dry_run: &DryRun,
    ) -> Result<bool, GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        validate_repository(repository)?;
        let mut plan = format!("add repository {} to project {project}", repository.name);
        if disable_publish {
            plan.push_str(" with publishing disabled");
        }
        // The dry-run guard: in dry-run the addition is recorded in
        // the plan and nothing is added — not even the credentials
        // check runs.
        if !dry_run.allow(&plan) {
            return Ok(false);
        }
        if !self.has_auth() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot add a repository without OBS credentials (GEECTL_OBS_APIKEY[_FILE] or GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD[_FILE])",
            ));
        }
        let mut doc = self.read_project_meta_doc(project)?;
        add_repository_to_meta(&mut doc, repository, disable_publish)?;
        self.put_project_meta(project, &doc.render())?;
        Ok(true)
    }

    /// Remove the repository named `name` from `project` (`PUT
    /// /source/<project>/_meta` with the project meta edited in
    /// place), along with its publish-disable entry when one is set.
    /// A repository that is not there is not an error — like
    /// `osc repo remove`, which tolerates a nonexistent name — and
    /// nothing is written then, unless a stale publish-disable entry
    /// for it remains: that is removed too, and the write happens.
    /// Requires OBS credentials. In dry-run the removal is
    /// recorded in the plan and nothing is written — the guard runs
    /// before the credentials check. Returns true when anything was
    /// removed: the repository, or its publish-disable entry when the
    /// repository itself is already gone.
    pub fn remove_repository(
        &self,
        project: &str,
        name: &str,
        dry_run: &DryRun,
    ) -> Result<bool, GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        if !valid_obs_name(name) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS repository name: {name:?}"),
            ));
        }
        if !dry_run.allow(&format!("remove repository {name} from project {project}")) {
            return Ok(false);
        }
        if !self.has_auth() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot remove a repository without OBS credentials (GEECTL_OBS_APIKEY[_FILE] or GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD[_FILE])",
            ));
        }
        let mut doc = self.read_project_meta_doc(project)?;
        if !remove_repository_from_meta(&mut doc, name)? {
            return Ok(false);
        }
        self.put_project_meta(project, &doc.render())?;
        Ok(true)
    }

    /// Read the project meta of `project` as an editable document.
    /// The fetched document must be a `<project>` document: a 200
    /// with another root (a `<status>` error document, a package
    /// meta) is a server error, not a document to edit and PUT back
    /// — the same fail-closed rule `parse_repositories` applies to
    /// the same response.
    fn read_project_meta_doc(&self, project: &str) -> Result<XmlDoc, GeectlError> {
        let url = build_url(&self.base, &["source", project, "_meta"], &[]);
        let meta = self.http.get_with(&url, &self.credentials)?;
        let doc = XmlDoc::parse(&meta)?;
        if doc.root_name() != "project" {
            let preview = crate::debug::format_body_preview(&meta);
            let received = if preview.is_empty() {
                "<empty response>"
            } else {
                preview.as_str()
            };
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected OBS project meta response: not a <project> document; received: {received}"
                ),
            ));
        }
        Ok(doc)
    }

    /// PUT the project meta of `project` back to the server, and fail
    /// closed when the server does not answer `<status code="ok"/>`.
    fn put_project_meta(&self, project: &str, rendered: &str) -> Result<(), GeectlError> {
        let url = build_url(&self.base, &["source", project, "_meta"], &[]);
        let body = self.http.put(&url, rendered, &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            let preview = crate::debug::format_body_preview(&body);
            let received = if preview.is_empty() {
                "<empty response>"
            } else {
                preview.as_str()
            };
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected OBS project meta response for project {project}; received: {received}"
                ),
            ))
        }
    }
}

/// One configured instance's read of the build-results matrix for
/// the merged view: the results, or the failure that kept them from
/// being read. A failure is data, not an abort: the merged view
/// renders it next to the other instances' results, so one
/// unreachable instance never drops the rest.
#[derive(Debug)]
pub struct InstanceBuildResults {
    /// The instance's alias from the `[instances]` table, or
    /// `default` for the configured instance with no alias.
    pub alias: String,
    /// The instance's API base URL.
    pub api_url: String,
    /// The instance's results, or the failure that kept them from
    /// being read.
    pub outcome: Result<BuildResults, GeectlError>,
}

/// Read the build results for `project` from every endpoint in
/// `endpoints`, in order, through `read`: one read per endpoint, and
/// a failure is recorded for that endpoint instead of aborting the
/// run, so one unreachable instance never drops the others'
/// results. The caller supplies the per-endpoint read (client plus
/// that instance's credentials), so a credential failure is an
/// endpoint failure like any other. An empty endpoint list reads
/// nothing and returns no outcomes.
pub fn results_across_instances(
    endpoints: &[(&str, &str)],
    project: &str,
    query: &ResultsQuery,
    read: impl Fn(&str, &str, &ResultsQuery) -> Result<BuildResults, GeectlError>,
) -> Vec<InstanceBuildResults> {
    endpoints
        .iter()
        .map(|(alias, api_url)| InstanceBuildResults {
            alias: (*alias).to_string(),
            api_url: (*api_url).to_string(),
            outcome: read(api_url, project, query),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use crate::obs::parse_project_meta;
    use crate::obs::test_support::{basic_credentials, FakeHttp};

    #[test]
    fn parse_directory_entry_names_accepts_an_xml_prolog() {
        // The server may emit an XML prolog before the directory
        // element; the result is the same as without one.
        let body = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><directory name=\"test:factory\"><entry name=\"pkgA\"/><entry name=\"pkgB\"/></directory>";
        assert_eq!(
            parse_directory_entry_names(body).unwrap(),
            vec!["pkgA".to_string(), "pkgB".to_string()]
        );
        // Bare prolog, and whitespace/newline between prolog and element.
        assert_eq!(
            parse_directory_entry_names("<?xml?><directory name=\"test:factory\"/>").unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            parse_directory_entry_names(
                "<?xml version=\"1.0\"?>\n<directory name=\"test:factory\"><entry name=\"pkgA\"/></directory>"
            )
            .unwrap(),
            vec!["pkgA".to_string()]
        );
        // A prolog that never terminates is malformed, not an empty listing.
        assert!(
            parse_directory_entry_names("<?xml version=\"1.0\" <directory></directory>").is_err()
        );
        assert!(parse_directory_entry_names("<?xml version=\"1.0\"").is_err());
        // `<?xmlfoo` is not a prolog at all: the guard rejects it as
        // malformed rather than skipping a prefix that is not one.
        assert!(
            parse_directory_entry_names("<?xmlfoo version=\"1.0\"?><directory></directory>")
                .is_err()
        );
    }

    #[test]
    fn parse_directory_entry_names_malformed_error_carries_response_preview() {
        // The failure stays fail-closed, but the error names what was
        // received, so the caller can diagnose it.
        let err = parse_directory_entry_names("garbage").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("garbage"), "{}", err.context);
        // A long body is capped: the preview carries the head, not the tail.
        let long = format!("<collection>{}</collection>", "x".repeat(3000));
        let err = parse_directory_entry_names(&long).unwrap_err();
        assert!(err.context.contains("... [truncated,"), "{}", err.context);
        assert!(!err.context.contains(&"x".repeat(3000)), "{}", err.context);
        // An empty body still fails closed, and says so.
        let err = parse_directory_entry_names("").unwrap_err();
        assert!(err.context.contains("<empty response>"), "{}", err.context);
    }

    #[test]
    fn xml_attr_reads_either_quote_style_and_requires_a_standalone_name() {
        assert_eq!(
            xml_attr("project=\"foo\" repository='bar'", "project"),
            Some("foo".to_string())
        );
        assert_eq!(
            xml_attr("project=\"foo\" repository='bar'", "repository"),
            Some("bar".to_string())
        );
        // `valid="..."` must not match a lookup for `id`.
        assert_eq!(
            xml_attr("valid=\"1\" id=\"42\"", "id"),
            Some("42".to_string())
        );
        assert_eq!(xml_attr("valid=\"1\"", "id"), None);
        // Unquoted values are not read.
        assert_eq!(xml_attr("code=failed", "code"), None);
        // Missing attribute is None.
        assert_eq!(xml_attr("code=\"failed\"", "state"), None);
        // An empty value reads as present.
        assert_eq!(xml_attr("code=\"\"", "code"), Some(String::new()));
    }

    #[test]
    fn xml_attr_non_empty_reads_an_empty_value_as_absent() {
        assert_eq!(xml_attr_non_empty("code=\"\"", "code"), None);
        assert_eq!(
            xml_attr_non_empty("code=\"building\"", "code"),
            Some("building".to_string())
        );
        assert_eq!(xml_attr_non_empty("code=\"failed\"", "state"), None);
    }

    #[test]
    fn xml_attr_ignores_a_name_inside_an_earlier_quoted_value() {
        // `dirty=` inside the details value must not match: the real
        // `dirty` attribute follows it.
        assert_eq!(
            xml_attr("details=\"x dirty=y\" dirty=\"true\"", "dirty"),
            Some("true".to_string())
        );
        assert_eq!(
            xml_attr("details=\"code=failed\" code=\"building\"", "code"),
            Some("building".to_string())
        );
    }

    #[test]
    fn xml_attr_tolerates_whitespace_and_skips_a_malformed_first_occurrence() {
        // Whitespace around `=` is the same attribute.
        assert_eq!(
            xml_attr("code = \"building\"", "code"),
            Some("building".to_string())
        );
        assert_eq!(
            xml_attr("code= \"building\"", "code"),
            Some("building".to_string())
        );
        // A malformed first occurrence does not stop the scan: the
        // well-formed later one wins.
        assert_eq!(
            xml_attr("code=failed code=\"building\"", "code"),
            Some("building".to_string())
        );
        assert_eq!(
            xml_attr("code = 'building'", "code"),
            Some("building".to_string())
        );
    }

    #[test]
    fn xml_element_text_reads_an_attributed_open_tag() {
        assert_eq!(
            xml_element_text("<summary lang=\"en\">hello</summary>", "summary").as_deref(),
            Some("hello")
        );
        assert_eq!(xml_element_text("<name attr=\"x\"/>", "name"), None);
        assert_eq!(
            xml_element_text("<namex>no</namex><name>yes</name>", "name").as_deref(),
            Some("yes")
        );
    }

    const RESULTS_FIXTURE: &str = "<resultlist state=\"f1cfebfd13354bcb8e792cb85f6218d8\">\n  <result project=\"testproject\" repository=\"openSUSE_Leap_42.3\" arch=\"x86_64\" code=\"published\" state=\"published\">\n    <status package=\"python-MarkupSafe\" code=\"disabled\" />\n  </result>\n  <result project=\"testproject\" repository=\"openSUSE_Leap_15.0\" arch=\"x86_64\" code=\"published\" state=\"published\">\n    <status package=\"python-MarkupSafe\" code=\"disabled\" />\n  </result>\n  <result project=\"testproject\" repository=\"SLE_12_SP4\" arch=\"x86_64\" code=\"published\" state=\"published\">\n    <status package=\"python-MarkupSafe\" code=\"disabled\" />\n  </result>\n  <result project=\"testproject\" repository=\"SLE_12_SP3\" arch=\"x86_64\" code=\"published\" state=\"published\">\n    <status package=\"python-MarkupSafe\" code=\"disabled\" />\n  </result>\n</resultlist>";

    const RESULTS_DIRTY_FIXTURE: &str = "<resultlist state=\"c57e2ee592dbbf26ebf19cc4f1bc1e83\">\n  <result project=\"testproject\" repository=\"openSUSE_Leap_42.3\" arch=\"x86_64\" code=\"published\" state=\"published\">\n    <status package=\"python-MarkupSafe\" code=\"disabled\" />\n  </result>\n  <result project=\"testproject\" repository=\"openSUSE_Leap_15.0\" arch=\"x86_64\" code=\"broken\" state=\"broken\" details=\"repository 'inheritedproject/openSUSE_Leap_15.0' is unavailable\">\n    <status package=\"python-MarkupSafe\" code=\"disabled\" />\n  </result>\n  <result project=\"testproject\" repository=\"SLE_12_SP4\" arch=\"x86_64\" code=\"publishing\" state=\"publishing\">\n    <status package=\"python-MarkupSafe\" code=\"disabled\" />\n  </result>\n  <result project=\"testproject\" repository=\"SLE_12_SP3\" arch=\"x86_64\" code=\"blocked\" state=\"blocked\" details=\"suspendproject\" dirty=\"true\">\n    <status package=\"python-MarkupSafe\" code=\"disabled\" />\n  </result>\n</resultlist>";

    #[test]
    fn parse_build_results_reads_the_matrix() {
        let parsed = parse_build_results(RESULTS_FIXTURE).unwrap();
        assert_eq!(
            parsed.state.as_deref(),
            Some("f1cfebfd13354bcb8e792cb85f6218d8")
        );
        assert_eq!(parsed.results.len(), 4);
        let first = &parsed.results[0];
        assert_eq!(first.project, "testproject");
        assert_eq!(first.repository, "openSUSE_Leap_42.3");
        assert_eq!(first.arch, "x86_64");
        assert_eq!(first.code, "published");
        assert_eq!(first.state, "published");
        assert!(!first.dirty);
        assert_eq!(first.details, None);
        assert_eq!(first.statuses.len(), 1);
        assert_eq!(first.statuses[0].package, "python-MarkupSafe");
        assert_eq!(first.statuses[0].code, "disabled");
        assert_eq!(first.statuses[0].details, None);
        assert!(results_settled(&parsed, None));
    }

    #[test]
    fn parse_build_results_reads_dirty_and_broken_cells() {
        let parsed = parse_build_results(RESULTS_DIRTY_FIXTURE).unwrap();
        assert_eq!(parsed.results.len(), 4);
        let broken = &parsed.results[1];
        assert_eq!(broken.code, "broken");
        assert_eq!(
            broken.details.as_deref(),
            Some("repository 'inheritedproject/openSUSE_Leap_15.0' is unavailable")
        );
        assert!(!broken.dirty);
        let blocked = &parsed.results[3];
        assert_eq!(blocked.code, "blocked");
        assert_eq!(blocked.details.as_deref(), Some("suspendproject"));
        assert!(blocked.dirty);
        assert!(!results_settled(&parsed, None));
    }

    #[test]
    fn parse_build_results_reads_an_empty_project() {
        let parsed = parse_build_results("<resultlist state=\"abc\"/>").unwrap();
        assert_eq!(parsed.state.as_deref(), Some("abc"));
        assert!(parsed.results.is_empty());
        assert!(results_settled(&parsed, None));
    }

    #[test]
    fn parse_build_results_rejects_malformed_documents() {
        assert!(parse_build_results("").is_err());
        assert!(parse_build_results("<collection></collection>").is_err());
        assert!(parse_build_results("<resultlist state=\"abc\">").is_err());
        assert!(parse_build_results("<resultlist>junk</resultlist>junk").is_err());
        // An unterminated <result> is malformed, not a shorter matrix.
        assert!(parse_build_results(
            "<resultlist><result project=\"p\" repository=\"r\" arch=\"x86_64\" code=\"succeeded\"></resultlist>"
        )
        .is_err());
        // A result without a required attribute is malformed, not
        // skipped: a partial matrix would read as a settled one.
        assert!(parse_build_results(
            "<resultlist><result project=\"p\" repository=\"r\" arch=\"x86_64\"/></resultlist>"
        )
        .is_err());
    }

    #[test]
    fn parse_build_results_accepts_an_xml_prolog() {
        // The server may emit an XML prolog before the <resultlist>
        // element; the result is the same as without one.
        let plain = "<resultlist/>";
        let prolog = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{plain}");
        assert_eq!(
            parse_build_results(&prolog).unwrap(),
            parse_build_results(plain).unwrap()
        );
        // Bare prolog, and whitespace/newline between prolog and element.
        assert_eq!(
            parse_build_results(&format!("<?xml?>{plain}")).unwrap(),
            parse_build_results(plain).unwrap()
        );
        assert_eq!(
            parse_build_results(&format!("<?xml version=\"1.0\"?>\n{plain}")).unwrap(),
            parse_build_results(plain).unwrap()
        );
        // A prolog that never terminates is malformed, not a missing <resultlist>.
        assert!(parse_build_results("<?xml version=\"1.0\" <resultlist></resultlist>").is_err());
        assert!(parse_build_results("<?xml version=\"1.0\"").is_err());
        // `<?xmlfoo` is not a prolog at all: the guard rejects it as
        // malformed rather than skipping a prefix that is not one.
        assert!(
            parse_build_results("<?xmlfoo version=\"1.0\"?><resultlist></resultlist>").is_err()
        );
    }

    #[test]
    fn parse_build_results_malformed_error_carries_response_preview() {
        // The failure stays fail-closed, but the error names what was
        // received, so the caller can diagnose it.
        let err = parse_build_results("garbage").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("garbage"), "{}", err.context);
        // A long body is capped: the preview carries the head, not the tail.
        let long = format!("<collection>{}</collection>", "x".repeat(3000));
        let err = parse_build_results(&long).unwrap_err();
        assert!(err.context.contains("... [truncated,"), "{}", err.context);
        assert!(!err.context.contains(&"x".repeat(3000)), "{}", err.context);
        // An empty body still fails closed, and says so.
        let err = parse_build_results("").unwrap_err();
        assert!(err.context.contains("<empty response>"), "{}", err.context);
    }

    #[test]
    fn results_settled_requires_no_dirty_and_no_waiting_code() {
        let settled = BuildResults {
            state: None,
            results: vec![BuildResult {
                project: "p".to_string(),
                repository: "r".to_string(),
                arch: "x86_64".to_string(),
                code: "published".to_string(),
                state: "published".to_string(),
                dirty: false,
                details: None,
                statuses: vec![PackageBuildStatus {
                    package: "pkg".to_string(),
                    code: "succeeded".to_string(),
                    details: None,
                }],
            }],
        };
        assert!(results_settled(&settled, None));
        let mut dirty = settled.clone();
        dirty.results[0].dirty = true;
        assert!(!results_settled(&dirty, None));
        let mut building = settled.clone();
        building.results[0].statuses[0].code = "building".to_string();
        assert!(!results_settled(&building, None));
        let mut scheduled = settled.clone();
        scheduled.results[0].code = "scheduled".to_string();
        assert!(!results_settled(&scheduled, None));
    }

    #[test]
    fn results_settled_requires_the_filtered_package_to_appear() {
        let settled = BuildResults {
            state: None,
            results: vec![BuildResult {
                project: "p".to_string(),
                repository: "r".to_string(),
                arch: "x86_64".to_string(),
                code: "published".to_string(),
                state: "published".to_string(),
                dirty: false,
                details: None,
                statuses: vec![PackageBuildStatus {
                    package: "pkg".to_string(),
                    code: "succeeded".to_string(),
                    details: None,
                }],
            }],
        };
        // The package is present and settled.
        assert!(results_settled(&settled, Some("pkg")));
        // A never-scheduled package drops out of a filtered read:
        // absence is not settled, even when every present cell is.
        assert!(!results_settled(&settled, Some("other")));
        // Without a filter the same matrix is settled.
        assert!(results_settled(&settled, None));
        // An empty matrix is settled without a filter, not with one.
        let empty = BuildResults {
            state: None,
            results: Vec::new(),
        };
        assert!(results_settled(&empty, None));
        assert!(!results_settled(&empty, Some("pkg")));
    }

    #[test]
    fn results_builds_the_per_package_url() {
        let query = ResultsQuery {
            package: Some("python-MarkupSafe".to_string()),
            multibuild: true,
            locallink: true,
            ..ResultsQuery::default()
        };
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/_result?package=python-MarkupSafe&multibuild=1&locallink=1",
            RESULTS_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let parsed = client.results("testproject", &query).unwrap();
        assert_eq!(parsed.results.len(), 4);
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/build/testproject/_result?package=python-MarkupSafe&multibuild=1&locallink=1"
                    .to_string(),
                true
            )]
        );
    }

    #[test]
    fn results_builds_the_project_url_without_filters() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/_result",
            RESULTS_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let parsed = client
            .results("testproject", &ResultsQuery::default())
            .unwrap();
        assert_eq!(parsed.results.len(), 4);
    }

    #[test]
    fn results_builds_repeated_repository_and_arch_pairs() {
        let query = ResultsQuery {
            repositories: vec!["standard".to_string(), "ports".to_string()],
            arches: vec!["x86_64".to_string(), "aarch64".to_string()],
            code: Some("failed".to_string()),
            lastbuild: true,
            ..ResultsQuery::default()
        };
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/_result?repository=standard&repository=ports&arch=x86_64&arch=aarch64&code=failed&lastbuild=1",
            "<resultlist state=\"abc\"/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let parsed = client.results("testproject", &query).unwrap();
        assert!(parsed.results.is_empty());
    }

    #[test]
    fn results_fails_closed_without_credentials() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/_result",
            RESULTS_FIXTURE,
        );
        let client = ObsClient::new(&http);
        let err = client
            .results("testproject", &ResultsQuery::default())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn results_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .results("bad/project", &ResultsQuery::default())
            .is_err());
        let query = ResultsQuery {
            package: Some("bad/package".to_string()),
            ..ResultsQuery::default()
        };
        assert!(client.results("testproject", &query).is_err());
        let query = ResultsQuery {
            repositories: vec!["bad repo".to_string()],
            ..ResultsQuery::default()
        };
        assert!(client.results("testproject", &query).is_err());
        let query = ResultsQuery {
            oldstate: Some(String::new()),
            ..ResultsQuery::default()
        };
        assert!(client.results("testproject", &query).is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn results_404_is_an_error_not_an_empty_matrix() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/build/nosuchproject/_result",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .results("nosuchproject", &ResultsQuery::default())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn results_across_instances_reads_each_endpoint_in_order() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.one.example/build/testproject/_result",
                "<resultlist state=\"aaa\"><result project=\"testproject\" repository=\"standard\" arch=\"x86_64\" code=\"published\" state=\"published\"><status package=\"foo\" code=\"succeeded\" /></result></resultlist>",
            )
            .with_get(
                "https://api.two.example/build/testproject/_result",
                "<resultlist state=\"bbb\"><result project=\"testproject\" repository=\"standard\" arch=\"x86_64\" code=\"published\" state=\"published\"><status package=\"foo\" code=\"failed\" /></result></resultlist>",
            );
        let query = ResultsQuery::default();
        let outcomes = results_across_instances(
            &[
                ("one", "https://api.one.example"),
                ("two", "https://api.two.example"),
            ],
            "testproject",
            &query,
            |url, project, query| {
                let client =
                    ObsClient::with_base_and_credentials(&http, url, basic_credentials()).unwrap();
                client.results(project, query)
            },
        );
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].alias, "one");
        assert_eq!(outcomes[0].api_url, "https://api.one.example");
        let first = outcomes[0].outcome.as_ref().unwrap();
        assert_eq!(first.state.as_deref(), Some("aaa"));
        assert_eq!(first.results[0].statuses[0].code, "succeeded");
        assert_eq!(outcomes[1].alias, "two");
        assert_eq!(outcomes[1].api_url, "https://api.two.example");
        let second = outcomes[1].outcome.as_ref().unwrap();
        assert_eq!(second.state.as_deref(), Some("bbb"));
        assert_eq!(second.results[0].statuses[0].code, "failed");
        assert_eq!(
            http.get_calls(),
            vec![
                (
                    "https://api.one.example/build/testproject/_result".to_string(),
                    true
                ),
                (
                    "https://api.two.example/build/testproject/_result".to_string(),
                    true
                ),
            ]
        );
    }

    #[test]
    fn results_across_instances_records_a_failure_without_dropping_the_other() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.one.example/build/testproject/_result",
                RESULTS_FIXTURE,
            )
            .with_get_error(
                "https://api.two.example/build/testproject/_result",
                ErrorKind::NotFound,
            );
        let query = ResultsQuery::default();
        let outcomes = results_across_instances(
            &[
                ("one", "https://api.one.example"),
                ("two", "https://api.two.example"),
            ],
            "testproject",
            &query,
            |url, project, query| {
                let client =
                    ObsClient::with_base_and_credentials(&http, url, basic_credentials()).unwrap();
                client.results(project, query)
            },
        );
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes[0].outcome.is_ok());
        assert_eq!(
            outcomes[1].outcome.as_ref().unwrap_err().kind(),
            ErrorKind::NotFound
        );
        // Both endpoints were attempted: the failure did not abort
        // the run.
        assert_eq!(http.get_calls().len(), 2);
    }

    #[test]
    fn results_across_instances_with_no_endpoints_reads_nothing() {
        let http = FakeHttp::new();
        let outcomes =
            results_across_instances(&[], "testproject", &ResultsQuery::default(), |_, _, _| {
                Ok(BuildResults {
                    state: None,
                    results: Vec::new(),
                })
            });
        assert!(outcomes.is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn wait_for_results_polls_plain_until_settled() {
        let http = FakeHttp::new().with_get_sequence(
            "https://api.opensuse.org/build/testproject/_result",
            &[RESULTS_DIRTY_FIXTURE, RESULTS_FIXTURE],
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let parsed = client
            .wait_for_results(
                "testproject",
                &ResultsQuery::default(),
                std::time::Duration::from_secs(30),
            )
            .unwrap();
        assert_eq!(parsed.results.len(), 4);
        assert!(results_settled(&parsed, None));
        assert_eq!(
            http.get_calls(),
            vec![
                (
                    "https://api.opensuse.org/build/testproject/_result".to_string(),
                    true
                ),
                (
                    "https://api.opensuse.org/build/testproject/_result".to_string(),
                    true
                ),
            ]
        );
    }

    #[test]
    fn wait_for_results_ignores_a_caller_set_oldstate() {
        let query = ResultsQuery {
            oldstate: Some("c57e2ee592dbbf26ebf19cc4f1bc1e83".to_string()),
            ..ResultsQuery::default()
        };
        let http = FakeHttp::new().with_get_sequence(
            "https://api.opensuse.org/build/testproject/_result",
            &[RESULTS_DIRTY_FIXTURE, RESULTS_FIXTURE],
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let parsed = client
            .wait_for_results("testproject", &query, std::time::Duration::from_secs(30))
            .unwrap();
        assert!(results_settled(&parsed, None));
        assert_eq!(
            http.get_calls(),
            vec![
                (
                    "https://api.opensuse.org/build/testproject/_result".to_string(),
                    true
                ),
                (
                    "https://api.opensuse.org/build/testproject/_result".to_string(),
                    true
                ),
            ]
        );
    }

    #[test]
    fn wait_for_results_returns_immediately_when_settled() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/_result",
            RESULTS_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let parsed = client
            .wait_for_results(
                "testproject",
                &ResultsQuery::default(),
                std::time::Duration::from_secs(30),
            )
            .unwrap();
        assert!(results_settled(&parsed, None));
        assert_eq!(http.get_calls().len(), 1);
    }

    #[test]
    fn wait_for_results_times_out_on_a_never_settling_build() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/_result",
            RESULTS_DIRTY_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .wait_for_results(
                "testproject",
                &ResultsQuery::default(),
                std::time::Duration::ZERO,
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Timeout);
        assert_eq!(http.get_calls().len(), 1);
    }

    #[test]
    fn wait_for_results_sleep_never_overshoots_a_short_deadline() {
        // A short deadline with an unsettled build: the sleep between
        // polls is capped at the remaining time, so the loop returns
        // at the deadline instead of overshooting it by a full sleep
        // interval (a fixed 2s sleep would take ~2s here).
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/_result",
            RESULTS_DIRTY_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let start = std::time::Instant::now();
        let err = client
            .wait_for_results(
                "testproject",
                &ResultsQuery::default(),
                std::time::Duration::from_millis(150),
            )
            .unwrap_err();
        let elapsed = start.elapsed();
        assert_eq!(err.kind(), ErrorKind::Timeout);
        assert!(elapsed < std::time::Duration::from_secs(1), "{elapsed:?}");
        assert_eq!(http.get_calls().len(), 2);
    }

    #[test]
    fn wait_for_results_polls_a_package_filtered_wait_until_the_package_appears() {
        // First poll: settled cells, but the filtered package is
        // absent (never scheduled) — not settled. Second poll: the
        // package appears, succeeded — settled.
        let http = FakeHttp::new().with_get_sequence(
            "https://api.opensuse.org/build/testproject/_result?package=foo",
            &[
                "<resultlist state=\"abc\"><result project=\"testproject\" repository=\"standard\" arch=\"x86_64\" code=\"published\" state=\"published\"></result></resultlist>",
                "<resultlist state=\"abc\"><result project=\"testproject\" repository=\"standard\" arch=\"x86_64\" code=\"published\" state=\"published\"><status package=\"foo\" code=\"succeeded\"/></result></resultlist>",
            ],
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let query = ResultsQuery {
            package: Some("foo".to_string()),
            ..ResultsQuery::default()
        };
        let parsed = client
            .wait_for_results("testproject", &query, std::time::Duration::from_secs(30))
            .unwrap();
        assert_eq!(parsed.results.len(), 1);
        assert_eq!(parsed.results[0].statuses.len(), 1);
        assert_eq!(parsed.results[0].statuses[0].package, "foo");
        assert_eq!(http.get_calls().len(), 2);
    }

    #[test]
    fn wait_for_results_times_out_on_a_never_scheduled_filtered_package() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/_result?package=foo",
            "<resultlist state=\"abc\"><result project=\"testproject\" repository=\"standard\" arch=\"x86_64\" code=\"published\" state=\"published\"></result></resultlist>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let query = ResultsQuery {
            package: Some("foo".to_string()),
            ..ResultsQuery::default()
        };
        let err = client
            .wait_for_results("testproject", &query, std::time::Duration::ZERO)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Timeout);
        assert_eq!(http.get_calls().len(), 1);
    }

    #[test]
    fn parse_job_status_reads_a_running_job() {
        let parsed = parse_job_status(
            "<jobstatus code=\"building\" details=\"building on sheep87:16\"><starttime>1618222900</starttime><lastduration>43</lastduration><workerid>sheep87:16</workerid><hostarch>x86_64</hostarch><uri>http://172.18.0.5:38963</uri><arch>x86_64</arch><job>home:alice::foo::1</job><jobid>17c8d25b92da67f6ec428ea51a9b5342</jobid><attempt>1</attempt></jobstatus>",
        )
        .unwrap();
        assert_eq!(
            parsed,
            Some(JobStatus {
                code: "building".to_string(),
                result: None,
                details: Some("building on sheep87:16".to_string()),
                starttime: Some(1_618_222_900),
                endtime: None,
                lastduration: Some(43),
                workerid: Some("sheep87:16".to_string()),
                hostarch: Some("x86_64".to_string()),
                uri: Some("http://172.18.0.5:38963".to_string()),
                arch: Some("x86_64".to_string()),
                job: Some("home:alice::foo::1".to_string()),
                jobid: Some("17c8d25b92da67f6ec428ea51a9b5342".to_string()),
                attempt: Some(1),
            })
        );
    }

    #[test]
    fn parse_job_status_empty_means_no_job_running() {
        assert_eq!(parse_job_status("<jobstatus/>").unwrap(), None);
        assert_eq!(parse_job_status("<jobstatus></jobstatus>").unwrap(), None);
        assert_eq!(parse_job_status("<jobstatus code=\"\"/>").unwrap(), None);
    }

    #[test]
    fn parse_job_status_rejects_malformed_documents() {
        assert!(parse_job_status("").is_err());
        assert!(parse_job_status("<collection></collection>").is_err());
        assert!(parse_job_status("<jobstatus code=\"building\"").is_err());
        assert!(parse_job_status(
            "<jobstatus code=\"building\"><starttime>soon</starttime></jobstatus>"
        )
        .is_err());
        assert!(parse_job_status("<jobstatus code=\"building\"><starttime>1</starttime>").is_err());
        assert!(parse_job_status("junk<jobstatus/>").is_err());
        assert!(parse_job_status("<jobstatus/>junk").is_err());
        assert!(parse_job_status("<jobstatus code=\"building\"></jobstatus>junk").is_err());
        assert!(parse_job_status("<jobstatusx/>").is_err());
    }

    #[test]
    fn parse_job_status_accepts_an_xml_prolog() {
        // The server may emit an XML prolog before the <jobstatus>
        // element; the result is the same as without one.
        let plain = "<jobstatus/>";
        let prolog = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{plain}");
        assert_eq!(
            parse_job_status(&prolog).unwrap(),
            parse_job_status(plain).unwrap()
        );
        // Bare prolog, and whitespace/newline between prolog and element.
        assert_eq!(
            parse_job_status(&format!("<?xml?>{plain}")).unwrap(),
            parse_job_status(plain).unwrap()
        );
        assert_eq!(
            parse_job_status(&format!("<?xml version=\"1.0\"?>\n{plain}")).unwrap(),
            parse_job_status(plain).unwrap()
        );
        // A prolog that never terminates is malformed, not a missing <jobstatus>.
        assert!(parse_job_status("<?xml version=\"1.0\" <jobstatus></jobstatus>").is_err());
        assert!(parse_job_status("<?xml version=\"1.0\"").is_err());
        // `<?xmlfoo` is not a prolog at all: the guard rejects it as
        // malformed rather than skipping a prefix that is not one.
        assert!(parse_job_status("<?xmlfoo version=\"1.0\"?><jobstatus></jobstatus>").is_err());
    }

    #[test]
    fn parse_job_status_malformed_error_carries_response_preview() {
        // The failure stays fail-closed, but the error names what was
        // received, so the caller can diagnose it.
        let err = parse_job_status("garbage").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("garbage"), "{}", err.context);
        // A long body is capped: the preview carries the head, not the tail.
        let long = format!("<collection>{}</collection>", "x".repeat(3000));
        let err = parse_job_status(&long).unwrap_err();
        assert!(err.context.contains("... [truncated,"), "{}", err.context);
        assert!(!err.context.contains(&"x".repeat(3000)), "{}", err.context);
        // An empty body still fails closed, and says so.
        let err = parse_job_status("").unwrap_err();
        assert!(err.context.contains("<empty response>"), "{}", err.context);
    }

    #[test]
    fn parse_build_status_reads_the_code() {
        assert_eq!(
            parse_build_status("<status package=\"foo\" code=\"failed\"/>").unwrap(),
            "failed"
        );
        assert_eq!(
            parse_build_status("<status code=\"succeeded\" package=\"foo\"></status>").unwrap(),
            "succeeded"
        );
    }

    #[test]
    fn parse_build_status_rejects_a_code_less_document() {
        assert!(parse_build_status("").is_err());
        assert!(parse_build_status("<status package=\"foo\"/>").is_err());
        assert!(parse_build_status("<status package=\"foo\" code=\"\"/>").is_err());
        assert!(parse_build_status("<collection></collection>").is_err());
        assert!(parse_build_status("junk<status code=\"failed\"/>").is_err());
        assert!(parse_build_status("<status code=\"failed\"/>junk").is_err());
        assert!(parse_build_status("<statusx code=\"failed\"/>").is_err());
    }

    #[test]
    fn parse_build_status_accepts_an_xml_prolog() {
        // The server may emit an XML prolog before the <status>
        // element; the result is the same as without one.
        let plain = "<status package=\"foo\" code=\"failed\"/>";
        let prolog = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{plain}");
        assert_eq!(
            parse_build_status(&prolog).unwrap(),
            parse_build_status(plain).unwrap()
        );
        // Bare prolog, and whitespace/newline between prolog and element.
        assert_eq!(
            parse_build_status(&format!("<?xml?>{plain}")).unwrap(),
            parse_build_status(plain).unwrap()
        );
        assert_eq!(
            parse_build_status(&format!("<?xml version=\"1.0\"?>\n{plain}")).unwrap(),
            parse_build_status(plain).unwrap()
        );
        // A prolog that never terminates is malformed, not a missing <status>.
        assert!(parse_build_status("<?xml version=\"1.0\" <status></status>").is_err());
        assert!(parse_build_status("<?xml version=\"1.0\"").is_err());
        // `<?xmlfoo` is not a prolog at all: the guard rejects it as
        // malformed rather than skipping a prefix that is not one.
        assert!(parse_build_status("<?xmlfoo version=\"1.0\"?><status></status>").is_err());
    }

    #[test]
    fn parse_build_status_malformed_error_carries_response_preview() {
        // The failure stays fail-closed, but the error names what was
        // received, so the caller can diagnose it.
        let err = parse_build_status("garbage").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("garbage"), "{}", err.context);
        // A long body is capped: the preview carries the head, not the tail.
        let long = format!("<collection>{}</collection>", "x".repeat(3000));
        let err = parse_build_status(&long).unwrap_err();
        assert!(err.context.contains("... [truncated,"), "{}", err.context);
        assert!(!err.context.contains(&"x".repeat(3000)), "{}", err.context);
        // An empty body still fails closed, and says so.
        let err = parse_build_status("").unwrap_err();
        assert!(err.context.contains("<empty response>"), "{}", err.context);
    }

    #[test]
    fn build_log_builds_the_url_and_returns_the_text() {
        let query = BuildLogQuery {
            start: Some(100),
            last: true,
            ..BuildLogQuery::default()
        };
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_log?nostream=1&start=100&last=1",
            "build log text",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .build_log("testproject", "foo", "standard", "x86_64", &query)
                .unwrap(),
            Some("build log text".to_string())
        );
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_log?nostream=1&start=100&last=1"
                    .to_string(),
                true
            )]
        );
    }

    #[test]
    fn build_log_keeps_the_multibuild_flavor_in_the_path() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo:flavor1/_log?nostream=1&lastsucceeded=1",
            "flavor log",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let query = BuildLogQuery {
            lastsucceeded: true,
            ..BuildLogQuery::default()
        };
        assert_eq!(
            client
                .build_log("testproject", "foo:flavor1", "standard", "x86_64", &query)
                .unwrap(),
            Some("flavor log".to_string())
        );
    }

    #[test]
    fn build_log_refuses_both_last_and_lastsucceeded() {
        // The two selectors name different logs: both together have
        // no meaning, so the call fails before any HTTP.
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let query = BuildLogQuery {
            last: true,
            lastsucceeded: true,
            ..BuildLogQuery::default()
        };
        let err = client
            .build_log("testproject", "foo", "standard", "x86_64", &query)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.context.contains("pass at most one"));
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn build_log_404_is_none_not_an_error() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_log?nostream=1",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .build_log(
                    "testproject",
                    "foo",
                    "standard",
                    "x86_64",
                    &BuildLogQuery::default()
                )
                .unwrap(),
            None
        );
    }

    #[test]
    fn build_log_fails_closed_without_credentials() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let err = client
            .build_log(
                "testproject",
                "foo",
                "standard",
                "x86_64",
                &BuildLogQuery::default(),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn build_log_strip_time_strips_the_timestamp_prefix() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_log?nostream=1",
            "[12:00:01] first line\nplain line\n[12:00:02] second line\n",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let query = BuildLogQuery {
            strip_time: true,
            ..BuildLogQuery::default()
        };
        assert_eq!(
            client
                .build_log("testproject", "foo", "standard", "x86_64", &query)
                .unwrap(),
            Some("first line\nplain line\nsecond line\n".to_string())
        );
    }

    #[test]
    fn build_log_without_strip_time_keeps_the_timestamps() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_log?nostream=1",
            "[12:00:01] first line\n",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .build_log(
                    "testproject",
                    "foo",
                    "standard",
                    "x86_64",
                    &BuildLogQuery::default()
                )
                .unwrap(),
            Some("[12:00:01] first line\n".to_string())
        );
    }

    #[test]
    fn strip_log_timestamps_keeps_lines_without_the_prefix() {
        // The bracketed prefix strips only at the line start, only
        // when the first `]` is followed by a space: `[a]b] c` keeps
        // its brackets (the first `]` is followed by `b`, not a
        // space), an unterminated `[` keeps the line, and a bare `[]`
        // with no trailing space is not a prefix either.
        assert_eq!(
            strip_log_timestamps(
                "[12:00:01] stripped\nplain\n[a]b] c\n[unterminated\n[]\n[] empty\n"
            ),
            "stripped\nplain\n[a]b] c\n[unterminated\n[]\nempty\n"
        );
        assert_eq!(strip_log_timestamps(""), "");
        assert_eq!(strip_log_timestamps("[12:00:01] no newline"), "no newline");
    }

    #[test]
    fn parse_build_log_url_accepts_the_api_form() {
        assert_eq!(
            parse_build_log_url(
                "https://api.opensuse.org/build/openSUSE:Factory/standard/x86_64/vim/_log",
                "https://api.opensuse.org"
            )
            .unwrap(),
            BuildLogCoordinates {
                project: "openSUSE:Factory".to_string(),
                package: "vim".to_string(),
                repository: "standard".to_string(),
                arch: "x86_64".to_string(),
            }
        );
    }

    #[test]
    fn parse_build_log_url_keeps_the_flavor_in_the_api_form() {
        assert_eq!(
            parse_build_log_url(
                "https://api.opensuse.org/build/openSUSE:Factory/standard/x86_64/multibuild-pkg:flavor1/_log",
                "https://api.opensuse.org"
            )
            .unwrap()
            .package,
            "multibuild-pkg:flavor1"
        );
    }

    #[test]
    fn parse_build_log_url_accepts_the_web_ui_form() {
        assert_eq!(
            parse_build_log_url(
                "https://build.opensuse.org/package/live_build_log/openSUSE:Factory/vim/standard/x86_64",
                "https://api.opensuse.org"
            )
            .unwrap(),
            BuildLogCoordinates {
                project: "openSUSE:Factory".to_string(),
                package: "vim".to_string(),
                repository: "standard".to_string(),
                arch: "x86_64".to_string(),
            }
        );
    }

    #[test]
    fn parse_build_log_url_keeps_the_flavor_in_the_web_ui_form() {
        assert_eq!(
            parse_build_log_url(
                "https://build.opensuse.org/package/live_build_log/openSUSE:Factory/multibuild-pkg:flavor2/standard/x86_64",
                "https://api.opensuse.org"
            )
            .unwrap()
            .package,
            "multibuild-pkg:flavor2"
        );
    }

    #[test]
    fn parse_build_log_url_accepts_a_loopback_api_host() {
        // The live instance is loopback http, not api.opensuse.org:
        // the host check compares against the configured base, so a
        // pasted URL from the configured instance parses.
        assert_eq!(
            parse_build_log_url(
                "http://127.0.0.1:3000/build/testproject/standard/x86_64/foo/_log",
                "http://127.0.0.1:3000"
            )
            .unwrap()
            .project,
            "testproject"
        );
    }

    #[test]
    fn parse_build_log_url_fails_closed() {
        let api = "https://api.opensuse.org";
        for url in [
            // Not a build-log URL at all.
            "openSUSE:Factory",
            "https://api.opensuse.org/build/openSUSE:Factory/standard/x86_64/vim",
            // Wrong path shape: a segment missing, one extra.
            "https://api.opensuse.org/build/openSUSE:Factory/standard/x86_64/_log",
            "https://api.opensuse.org/build/openSUSE:Factory/standard/x86_64/vim/extra/_log",
            // Trailing slash, query string, fragment: refused rather
            // than silently ignored.
            "https://api.opensuse.org/build/openSUSE:Factory/standard/x86_64/vim/_log/",
            "https://api.opensuse.org/build/openSUSE:Factory/standard/x86_64/vim/_log?start=100",
            "https://api.opensuse.org/build/openSUSE:Factory/standard/x86_64/vim/_log#top",
            // A different API instance than the configured one.
            "https://api.example.org/build/openSUSE:Factory/standard/x86_64/vim/_log",
            // A web-UI URL from another host, and a web-UI URL
            // against a non-public configured API.
            "https://build.example.org/package/live_build_log/openSUSE:Factory/vim/standard/x86_64",
            "https://api.opensuse.org/package/live_build_log/openSUSE:Factory/vim/standard/x86_64",
            // An empty coordinate is not a coordinate.
            "https://api.opensuse.org/build//standard/x86_64/vim/_log",
        ] {
            let err = parse_build_log_url(url, api).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput, "{url}");
        }
        // The web-UI form against a non-public API base.
        let err = parse_build_log_url(
            "https://build.opensuse.org/package/live_build_log/openSUSE:Factory/vim/standard/x86_64",
            "http://127.0.0.1:3000",
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn build_status_returns_the_code_and_none_on_404() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_status",
                "<status package=\"foo\" code=\"failed\"/>",
            )
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/baz/_status",
                "<status package=\"baz\" code=\"unknown\"/>",
            )
            .with_get_error(
                "https://api.opensuse.org/build/testproject/standard/x86_64/bar/_status",
                ErrorKind::NotFound,
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .build_status("testproject", "foo", "standard", "x86_64")
                .unwrap(),
            Some("failed".to_string())
        );
        assert_eq!(
            client
                .build_status("testproject", "bar", "standard", "x86_64")
                .unwrap(),
            None
        );
        assert_eq!(
            client
                .build_status("testproject", "baz", "standard", "x86_64")
                .unwrap(),
            None
        );
    }

    #[test]
    fn build_status_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .build_status("testproject", "bad/package", "standard", "x86_64")
            .is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn job_status_returns_the_job_and_none_when_idle() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_jobstatus",
                "<jobstatus code=\"building\"><starttime>1618222900</starttime></jobstatus>",
            )
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/bar/_jobstatus",
                "<jobstatus/>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let job = client
            .job_status("testproject", "foo", "standard", "x86_64")
            .unwrap();
        assert_eq!(job.as_ref().unwrap().code, "building");
        assert_eq!(job.as_ref().unwrap().starttime, Some(1_618_222_900));
        assert_eq!(
            client
                .job_status("testproject", "bar", "standard", "x86_64")
                .unwrap(),
            None
        );
    }

    #[test]
    fn job_status_404_is_an_error_not_none() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/build/testproject/standard/x86_64/nosuch/_jobstatus",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .job_status("testproject", "nosuch", "standard", "x86_64")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn parse_buildinfo_reads_the_error_and_the_dependencies() {
        let parsed = parse_buildinfo(
            "<buildinfo project=\"p\" repository=\"r\" package=\"foo\" arch=\"x86_64\"><error>unresolvable: DOES-NOT-EXIST</error><bdep name=\"gcc\"/><bdep name=\"make\"/></buildinfo>",
        )
        .unwrap();
        assert_eq!(
            parsed.error.as_deref(),
            Some("unresolvable: DOES-NOT-EXIST")
        );
        assert_eq!(
            parsed
                .bdeps
                .iter()
                .map(|dep| dep.name.as_str())
                .collect::<Vec<_>>(),
            vec!["gcc", "make"]
        );
    }

    #[test]
    fn parse_buildinfo_reads_bdep_origin_and_hdrmd5() {
        // The trust check reads the project set and the cache
        // verification reads the hdrmd5: both come from the `<bdep>`
        // attributes, not from a second lookup.
        let parsed = parse_buildinfo(
            "<buildinfo project=\"p\" repository=\"r\" package=\"foo\" arch=\"x86_64\">\
             <bdep name=\"gcc\" project=\"openSUSE:Factory\" repository=\"standard\" binary=\"gcc-13-1.1.x86_64.rpm\" hdrmd5=\"0123456789abcdef0123456789abcdef\" preinstall=\"1\"/>\
             <bdep name=\"make\" project=\"home:alice\" repository=\"standard\" vminstall=\"1\" runscripts=\"true\"/>\
             <bdep name=\"bash\"/></buildinfo>",
        )
        .unwrap();
        assert_eq!(parsed.bdeps.len(), 3);
        let gcc = &parsed.bdeps[0];
        assert_eq!(gcc.name, "gcc");
        assert_eq!(gcc.project.as_deref(), Some("openSUSE:Factory"));
        assert_eq!(gcc.repository.as_deref(), Some("standard"));
        assert_eq!(gcc.binary.as_deref(), Some("gcc-13-1.1.x86_64.rpm"));
        assert_eq!(
            gcc.hdrmd5.as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert!(gcc.preinstall);
        assert!(!gcc.vminstall);
        assert!(!gcc.runscripts);
        let make = &parsed.bdeps[1];
        assert_eq!(make.project.as_deref(), Some("home:alice"));
        assert!(make.vminstall);
        assert!(make.runscripts);
        assert!(!make.preinstall);
        // No origin at all reads as unknown, never as a match.
        assert_eq!(parsed.bdeps[2].name, "bash");
        assert_eq!(parsed.bdeps[2].project, None);
        assert_eq!(parsed.bdeps[2].hdrmd5, None);
        assert!(!parsed.bdeps[2].noinstall);
        assert_eq!(parsed.bdeps[2].repoarch, None);
        assert!(!parsed.bdeps[2].installonly);
        assert!(!parsed.bdeps[2].sysroot);
    }

    #[test]
    fn parse_buildinfo_reads_bdep_source_package_and_version() {
        // Real scheduler buildinfo carries no `binary` attribute:
        // the source package and the version/release/arch/epoch
        // are what the local build resolves the file from.
        let parsed = parse_buildinfo(
            "<buildinfo project=\"p\" repository=\"r\" package=\"foo\" arch=\"x86_64\">\
             <bdep name=\"libz1\" project=\"openSUSE:Factory\" repository=\"standard\" package=\"zlib\" version=\"1.3.1\" release=\"1.1\" arch=\"x86_64\" epoch=\"0\" hdrmd5=\"0123456789abcdef0123456789abcdef\"/>\
             <bdep name=\"bash\"/></buildinfo>",
        )
        .unwrap();
        assert_eq!(parsed.bdeps.len(), 2);
        let libz1 = &parsed.bdeps[0];
        assert_eq!(libz1.name, "libz1");
        assert_eq!(libz1.package.as_deref(), Some("zlib"));
        assert_eq!(libz1.binary, None);
        assert_eq!(libz1.version.as_deref(), Some("1.3.1"));
        assert_eq!(libz1.release.as_deref(), Some("1.1"));
        assert_eq!(libz1.arch.as_deref(), Some("x86_64"));
        assert_eq!(libz1.epoch.as_deref(), Some("0"));
        assert_eq!(
            libz1.hdrmd5.as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        // Absent reads as unknown on every new field, like the old ones.
        let bash = &parsed.bdeps[1];
        assert_eq!(bash.package, None);
        assert_eq!(bash.version, None);
        assert_eq!(bash.release, None);
        assert_eq!(bash.arch, None);
        assert_eq!(bash.epoch, None);
    }

    #[test]
    fn parse_buildinfo_reads_bdep_repoarch_installonly_sysroot() {
        // The local build keys the cache and the download URL by the
        // dep's own repository/repoarch, and the rpmlist carries the
        // installonly/sysroot shapes: all three come from the
        // `<bdep>` attributes.
        let parsed = parse_buildinfo(
            "<buildinfo project=\"p\" repository=\"r\" package=\"foo\" arch=\"x86_64\">\
             <bdep name=\"cross\" project=\"openSUSE:Factory\" repository=\"standard\" repoarch=\"aarch64\" binary=\"cross-1-1.aarch64.rpm\" installonly=\"1\"/>\
             <bdep name=\"sys\" project=\"openSUSE:Factory\" repository=\"standard\" binary=\"sys-1-1.x86_64.rpm\" sysroot=\"1\"/></buildinfo>",
        )
        .unwrap();
        assert_eq!(parsed.bdeps.len(), 2);
        let cross = &parsed.bdeps[0];
        assert_eq!(cross.repository.as_deref(), Some("standard"));
        assert_eq!(cross.repoarch.as_deref(), Some("aarch64"));
        assert!(cross.installonly);
        assert!(!cross.sysroot);
        let sys = &parsed.bdeps[1];
        assert!(sys.sysroot);
        assert!(!sys.installonly);
        assert_eq!(sys.repoarch, None);
    }

    #[test]
    fn parse_buildinfo_reads_the_preinstall_image() {
        let parsed = parse_buildinfo(
            "<buildinfo project=\"p\"><preinstallimage project=\"openSUSE:Factory\" repository=\"standard\" package=\"preinstallimage\" filename=\"openSUSE-Tumbleweed-preinstallimage.x86_64.rpm\" hdrmd5=\"abcdef0123456789abcdef0123456789\"/><bdep name=\"gcc\"/></buildinfo>",
        )
        .unwrap();
        let image = parsed.preinstall_image.as_ref().unwrap();
        assert_eq!(image.project.as_deref(), Some("openSUSE:Factory"));
        assert_eq!(image.repository.as_deref(), Some("standard"));
        assert_eq!(image.package.as_deref(), Some("preinstallimage"));
        assert_eq!(
            image.filename.as_deref(),
            Some("openSUSE-Tumbleweed-preinstallimage.x86_64.rpm")
        );
        assert_eq!(
            image.hdrmd5.as_deref(),
            Some("abcdef0123456789abcdef0123456789")
        );
        // No image element reads as none, and a second element is
        // ignored: the server sends at most one.
        let parsed =
            parse_buildinfo("<buildinfo project=\"p\"><bdep name=\"gcc\"/></buildinfo>").unwrap();
        assert_eq!(parsed.preinstall_image, None);
    }

    #[test]
    fn parse_buildinfo_reads_path_mirror_urls() {
        // The live shape: a published repo carries one `<path>` per
        // (project, repository) with the verbatim mirror URL; a repo
        // that is not published carries no `url` and reads as no
        // entry; a buildinfo with no paths at all reads as empty; a
        // later `<path>` for the same pair wins.
        let parsed = parse_buildinfo(
            "<buildinfo project=\"openSUSE:Factory\" repository=\"standard\" package=\"hello\" arch=\"x86_64\">\
             <path project=\"openSUSE:Factory\" repository=\"standard\" url=\"https://download.opensuse.org/tumbleweed/repo/oss/\"/>\
             <path project=\"openSUSE:Factory\" repository=\"ports\" url=\"https://download.opensuse.org/repositories/openSUSE:/Factory/ports/\"/>\
             <path project=\"openSUSE:Factory\" repository=\"standard\" url=\"https://mirror.example.com/later/\"/>\
             <path project=\"home:alice\" repository=\"standard\"/>\
             <bdep name=\"bash\"/></buildinfo>",
        )
        .unwrap();
        assert_eq!(parsed.urls.len(), 2);
        assert_eq!(
            parsed
                .urls
                .get(&("openSUSE:Factory".to_string(), "standard".to_string())),
            Some(&"https://mirror.example.com/later/".to_string())
        );
        assert_eq!(
            parsed
                .urls
                .get(&("openSUSE:Factory".to_string(), "ports".to_string())),
            Some(
                &"https://download.opensuse.org/repositories/openSUSE:/Factory/ports/".to_string()
            )
        );
        assert_eq!(
            parsed
                .urls
                .get(&("home:alice".to_string(), "standard".to_string())),
            None
        );
        let parsed =
            parse_buildinfo("<buildinfo project=\"p\"><bdep name=\"gcc\"/></buildinfo>").unwrap();
        assert!(parsed.urls.is_empty());
    }

    #[test]
    fn parse_buildinfo_skips_path_without_project_or_repository() {
        // A malformed `<path>` is skipped and counted, never fatal:
        // mirror data is only an optimisation, so the buildinfo
        // still parses and the dep fetch falls back to the API for
        // that repo.
        let parsed = parse_buildinfo(
            "<buildinfo project=\"p\"><path repository=\"standard\" url=\"https://example.com/\"/><bdep name=\"gcc\"/></buildinfo>",
        )
        .unwrap();
        assert!(parsed.urls.is_empty());
        assert_eq!(parsed.skipped_paths, 1);
        assert_eq!(parsed.bdeps.len(), 1);
        let parsed = parse_buildinfo(
            "<buildinfo project=\"p\"><path project=\"openSUSE:Factory\" url=\"https://example.com/\"/></buildinfo>",
        )
        .unwrap();
        assert!(parsed.urls.is_empty());
        assert_eq!(parsed.skipped_paths, 1);
    }

    #[test]
    fn parse_buildinfo_skips_a_mirror_url_that_fails_validation() {
        // A `<path>` whose URL is not a public https URL names no
        // usable mirror: the entry is skipped and counted, and a
        // valid entry for another repo still lands.
        let parsed = parse_buildinfo(
            "<buildinfo project=\"p\">\
             <path project=\"openSUSE:Factory\" repository=\"standard\" url=\"http://mirror.example.com/\"/>\
             <path project=\"openSUSE:Factory\" repository=\"ports\" url=\"https://user:pass@mirror.example.com/\"/>\
             <path project=\"openSUSE:Factory\" repository=\"images\" url=\"https://127.0.0.1/\"/>\
             <path project=\"openSUSE:Factory\" repository=\"good\" url=\"https://download.opensuse.org/tumbleweed/repo/oss/\"/>\
             </buildinfo>",
        )
        .unwrap();
        assert_eq!(parsed.urls.len(), 1);
        assert!(parsed
            .urls
            .contains_key(&("openSUSE:Factory".to_string(), "good".to_string())));
        assert_eq!(parsed.skipped_paths, 3);
    }

    #[test]
    fn parse_buildinfo_reads_the_preinstall_image_binaries() {
        let parsed = parse_buildinfo(
            "<buildinfo project=\"p\"><preinstallimage project=\"openSUSE:Factory\" repository=\"standard\" package=\"preinstallimage\" filename=\"_preinstallimage.abc123\" hdrmd5=\"abc123\"><binary>glibc</binary><binary>bash</binary></preinstallimage><bdep name=\"gcc\"/></buildinfo>",
        )
        .unwrap();
        let image = parsed.preinstall_image.as_ref().unwrap();
        assert_eq!(
            image.binaries,
            vec!["glibc".to_string(), "bash".to_string()]
        );
        // A self-closing element carries no binaries, and a second
        // element is ignored: the first is the offer.
        let parsed = parse_buildinfo(
            "<buildinfo project=\"p\"><preinstallimage project=\"openSUSE:Factory\" repository=\"standard\" package=\"preinstallimage\" filename=\"_preinstallimage.abc123\"/><preinstallimage project=\"q\"><binary>other</binary></preinstallimage></buildinfo>",
        )
        .unwrap();
        let image = parsed.preinstall_image.as_ref().unwrap();
        assert!(image.binaries.is_empty());
        // Whitespace around a name and an empty child are not names.
        let parsed = parse_buildinfo(
            "<buildinfo project=\"p\"><preinstallimage project=\"openSUSE:Factory\"><binary> glibc </binary><binary></binary><binary>bash</binary></preinstallimage></buildinfo>",
        )
        .unwrap();
        let image = parsed.preinstall_image.as_ref().unwrap();
        assert_eq!(
            image.binaries,
            vec!["glibc".to_string(), "bash".to_string()]
        );
        // A malformed child list (an unclosed `<binary>`) reads as
        // no binaries: the offer stands, but it covers nothing.
        let parsed = parse_buildinfo(
            "<buildinfo project=\"p\"><preinstallimage project=\"openSUSE:Factory\"><binary>glibc</binary><binary>pre</preinstallimage></buildinfo>",
        )
        .unwrap();
        let image = parsed.preinstall_image.as_ref().unwrap();
        assert!(image.binaries.is_empty());
    }

    #[test]
    fn parse_buildinfo_without_an_error_is_empty() {
        let parsed = parse_buildinfo("<buildinfo project=\"p\"/>").unwrap();
        assert_eq!(parsed.error, None);
        assert!(parsed.bdeps.is_empty());
        let parsed = parse_buildinfo("<buildinfo/>").unwrap();
        assert_eq!(parsed.error, None);
        assert!(parsed.bdeps.is_empty());
    }

    #[test]
    fn parse_buildinfo_rejects_malformed_documents() {
        assert!(parse_buildinfo("").is_err());
        assert!(parse_buildinfo("<collection></collection>").is_err());
        assert!(parse_buildinfo("<buildinfo>").is_err());
        assert!(parse_buildinfo("<buildinfo><bdep version=\"1\"/></buildinfo>").is_err());
    }

    #[test]
    fn parse_buildinfo_accepts_an_xml_prolog() {
        // The server may emit an XML prolog before the <buildinfo>
        // element; the result is the same as without one.
        let plain = "<buildinfo project=\"p\"/>";
        let prolog = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{plain}");
        assert_eq!(
            parse_buildinfo(&prolog).unwrap(),
            parse_buildinfo(plain).unwrap()
        );
        // Bare prolog, and whitespace/newline between prolog and element.
        assert_eq!(
            parse_buildinfo(&format!("<?xml?>{plain}")).unwrap(),
            parse_buildinfo(plain).unwrap()
        );
        assert_eq!(
            parse_buildinfo(&format!("<?xml version=\"1.0\"?>\n{plain}")).unwrap(),
            parse_buildinfo(plain).unwrap()
        );
        // A prolog that never terminates is malformed, not a missing <buildinfo>.
        assert!(parse_buildinfo("<?xml version=\"1.0\" <buildinfo></buildinfo>").is_err());
        assert!(parse_buildinfo("<?xml version=\"1.0\"").is_err());
        // `<?xmlfoo` is not a prolog at all: the guard rejects it as
        // malformed rather than skipping a prefix that is not one.
        assert!(parse_buildinfo("<?xmlfoo version=\"1.0\"?><buildinfo></buildinfo>").is_err());
    }

    #[test]
    fn parse_buildinfo_malformed_error_carries_response_preview() {
        // The failure stays fail-closed, but the error names what was
        // received, so the caller can diagnose it.
        let err = parse_buildinfo("garbage").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("garbage"), "{}", err.context);
        // A long body is capped: the preview carries the head, not the tail.
        let long = format!("<collection>{}</collection>", "x".repeat(3000));
        let err = parse_buildinfo(&long).unwrap_err();
        assert!(err.context.contains("... [truncated,"), "{}", err.context);
        assert!(!err.context.contains(&"x".repeat(3000)), "{}", err.context);
        // An empty body still fails closed, and says so.
        let err = parse_buildinfo("").unwrap_err();
        assert!(err.context.contains("<empty response>"), "{}", err.context);
    }

    #[test]
    fn parse_build_history_reads_entries() {
        let parsed = parse_build_history(
            "<buildhistory><entry rev=\"2\" srcmd5=\"def\" versrel=\"1.0-1\" bcnt=\"1\" time=\"1699900000\"/><entry rev=\"3\" srcmd5=\"abc\" versrel=\"1.0-1\" bcnt=\"2\" time=\"1700000000\" duration=\"120\"/></buildhistory>",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].rev, "3");
        assert_eq!(parsed[0].srcmd5, "abc");
        assert_eq!(parsed[0].versrel, "1.0-1");
        assert_eq!(parsed[0].bcnt, 2);
        assert_eq!(parsed[0].time, 1_700_000_000);
        assert_eq!(parsed[0].duration, Some(120));
        assert_eq!(parsed[1].duration, None);
    }

    #[test]
    fn parse_build_history_empty_is_an_empty_list() {
        assert_eq!(
            parse_build_history("<buildhistory/>").unwrap(),
            Vec::<BuildHistoryEntry>::new()
        );
        assert_eq!(
            parse_build_history("<buildhistory></buildhistory>").unwrap(),
            Vec::<BuildHistoryEntry>::new()
        );
    }

    #[test]
    fn parse_build_history_rejects_malformed_documents() {
        assert!(parse_build_history("").is_err());
        assert!(parse_build_history("<collection></collection>").is_err());
        assert!(
            parse_build_history(
                "<buildhistory><entry rev=\"3\" srcmd5=\"abc\" versrel=\"1.0-1\" bcnt=\"x\" time=\"1\"/></buildhistory>"
            )
            .is_err()
        );
        assert!(
            parse_build_history(
                "<buildhistory><entry srcmd5=\"abc\" versrel=\"1.0-1\" bcnt=\"1\" time=\"1\"/></buildhistory>"
            )
            .is_err()
        );
    }

    #[test]
    fn parse_build_history_accepts_an_xml_prolog() {
        // The server may emit an XML prolog before the <buildhistory>
        // element; the result is the same as without one.
        let plain = "<buildhistory/>";
        let prolog = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{plain}");
        assert_eq!(
            parse_build_history(&prolog).unwrap(),
            parse_build_history(plain).unwrap()
        );
        // Bare prolog, and whitespace/newline between prolog and element.
        assert_eq!(
            parse_build_history(&format!("<?xml?>{plain}")).unwrap(),
            parse_build_history(plain).unwrap()
        );
        assert_eq!(
            parse_build_history(&format!("<?xml version=\"1.0\"?>\n{plain}")).unwrap(),
            parse_build_history(plain).unwrap()
        );
        // A prolog that never terminates is malformed, not a missing <buildhistory>.
        assert!(
            parse_build_history("<?xml version=\"1.0\" <buildhistory></buildhistory>").is_err()
        );
        assert!(parse_build_history("<?xml version=\"1.0\"").is_err());
        // `<?xmlfoo` is not a prolog at all: the guard rejects it as
        // malformed rather than skipping a prefix that is not one.
        assert!(
            parse_build_history("<?xmlfoo version=\"1.0\"?><buildhistory></buildhistory>").is_err()
        );
    }

    #[test]
    fn parse_build_history_malformed_error_carries_response_preview() {
        // The failure stays fail-closed, but the error names what was
        // received, so the caller can diagnose it.
        let err = parse_build_history("garbage").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("garbage"), "{}", err.context);
        // A long body is capped: the preview carries the head, not the tail.
        let long = format!("<collection>{}</collection>", "x".repeat(3000));
        let err = parse_build_history(&long).unwrap_err();
        assert!(err.context.contains("... [truncated,"), "{}", err.context);
        assert!(!err.context.contains(&"x".repeat(3000)), "{}", err.context);
        // An empty body still fails closed, and says so.
        let err = parse_build_history("").unwrap_err();
        assert!(err.context.contains("<empty response>"), "{}", err.context);
    }

    #[test]
    fn parse_job_history_reads_entries() {
        let parsed = parse_job_history(
            "<jobhistlist><jobhist repository=\"standard\" arch=\"x86_64\" package=\"foo\" rev=\"3\" srcmd5=\"abc\" versrel=\"1.0-1\" bcnt=\"2\" readytime=\"1700000000\" starttime=\"1700000060\" endtime=\"1700000120\" code=\"succeeded\" uri=\"http://worker:3000\" workerid=\"w1\" hostarch=\"x86_64\" reason=\"source change\" verifymd5=\"abc\"/></jobhistlist>",
        )
        .unwrap();
        assert_eq!(parsed.len(), 1);
        let entry = &parsed[0];
        assert_eq!(entry.repository.as_deref(), Some("standard"));
        assert_eq!(entry.package, "foo");
        assert_eq!(entry.code, "succeeded");
        assert_eq!(entry.bcnt, Some(2));
        assert_eq!(entry.readytime, Some(1_700_000_000));
        assert_eq!(entry.starttime, Some(1_700_000_060));
        assert_eq!(entry.endtime, Some(1_700_000_120));
        assert_eq!(entry.reason.as_deref(), Some("source change"));
    }

    #[test]
    fn parse_job_history_reads_newest_first() {
        let parsed = parse_job_history(
            "<jobhistlist><jobhist package=\"foo\" code=\"failed\" endtime=\"1700000000\"/><jobhist package=\"foo\" code=\"succeeded\" endtime=\"1700000120\"/></jobhistlist>",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].code, "succeeded");
        assert_eq!(parsed[1].code, "failed");
    }

    #[test]
    fn parse_job_history_minimal_entry_is_enough() {
        let parsed = parse_job_history(
            "<jobhistlist><jobhist package=\"foo\" code=\"failed\"/></jobhistlist>",
        )
        .unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].package, "foo");
        assert_eq!(parsed[0].code, "failed");
        assert_eq!(parsed[0].bcnt, None);
    }

    #[test]
    fn parse_job_history_rejects_malformed_documents() {
        assert!(parse_job_history("").is_err());
        assert!(parse_job_history("<collection></collection>").is_err());
        assert!(
            parse_job_history("<jobhistlist><jobhist code=\"failed\"/></jobhistlist>").is_err()
        );
        assert!(
            parse_job_history("<jobhistlist><jobhist package=\"foo\"/></jobhistlist>").is_err()
        );
        assert!(
            parse_job_history(
                "<jobhistlist><jobhist package=\"foo\" code=\"failed\" starttime=\"soon\"/></jobhistlist>"
            )
            .is_err()
        );
    }

    #[test]
    fn parse_job_history_accepts_an_xml_prolog() {
        // The server may emit an XML prolog before the <jobhistlist>
        // element; the result is the same as without one.
        let plain = "<jobhistlist/>";
        let prolog = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{plain}");
        assert_eq!(
            parse_job_history(&prolog).unwrap(),
            parse_job_history(plain).unwrap()
        );
        // Bare prolog, and whitespace/newline between prolog and element.
        assert_eq!(
            parse_job_history(&format!("<?xml?>{plain}")).unwrap(),
            parse_job_history(plain).unwrap()
        );
        assert_eq!(
            parse_job_history(&format!("<?xml version=\"1.0\"?>\n{plain}")).unwrap(),
            parse_job_history(plain).unwrap()
        );
        // A prolog that never terminates is malformed, not a missing <jobhistlist>.
        assert!(parse_job_history("<?xml version=\"1.0\" <jobhistlist></jobhistlist>").is_err());
        assert!(parse_job_history("<?xml version=\"1.0\"").is_err());
        // `<?xmlfoo` is not a prolog at all: the guard rejects it as
        // malformed rather than skipping a prefix that is not one.
        assert!(
            parse_job_history("<?xmlfoo version=\"1.0\"?><jobhistlist></jobhistlist>").is_err()
        );
    }

    #[test]
    fn parse_job_history_malformed_error_carries_response_preview() {
        // The failure stays fail-closed, but the error names what was
        // received, so the caller can diagnose it.
        let err = parse_job_history("garbage").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("garbage"), "{}", err.context);
        // A long body is capped: the preview carries the head, not the tail.
        let long = format!("<collection>{}</collection>", "x".repeat(3000));
        let err = parse_job_history(&long).unwrap_err();
        assert!(err.context.contains("... [truncated,"), "{}", err.context);
        assert!(!err.context.contains(&"x".repeat(3000)), "{}", err.context);
        // An empty body still fails closed, and says so.
        let err = parse_job_history("").unwrap_err();
        assert!(err.context.contains("<empty response>"), "{}", err.context);
    }

    #[test]
    fn buildinfo_returns_the_error_and_dependencies() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_buildinfo",
            "<buildinfo project=\"testproject\" repository=\"standard\" package=\"foo\" arch=\"x86_64\"><error>unresolvable: DOES-NOT-EXIST</error><bdep name=\"gcc\"/></buildinfo>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let info = client
            .buildinfo("testproject", "foo", "standard", "x86_64")
            .unwrap();
        assert_eq!(info.error.as_deref(), Some("unresolvable: DOES-NOT-EXIST"));
        assert_eq!(
            info.bdeps
                .iter()
                .map(|dep| dep.name.as_str())
                .collect::<Vec<_>>(),
            vec!["gcc"]
        );
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_buildinfo"
                    .to_string(),
                true
            )]
        );
    }

    #[test]
    fn buildinfo_fails_closed_without_credentials() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let err = client
            .buildinfo("testproject", "foo", "standard", "x86_64")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn buildconfig_returns_the_raw_text() {
        // The buildconfig passes through as-is: the engine's own
        // format, not XML, so the client types nothing here.
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/_buildconfig",
            "###\n# comment\nSubstitute: gcc gcc13\n",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let text = client.buildconfig("testproject", "standard").unwrap();
        assert_eq!(text, "###\n# comment\nSubstitute: gcc gcc13\n");
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/build/testproject/standard/_buildconfig".to_string(),
                true
            )]
        );
    }

    #[test]
    fn buildconfig_fails_closed_without_credentials() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let err = client.buildconfig("testproject", "standard").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn build_history_builds_the_url_with_and_without_a_limit() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_history?limit=1",
                "<buildhistory><entry rev=\"3\" srcmd5=\"abc\" versrel=\"1.0-1\" bcnt=\"2\" time=\"1700000000\"/></buildhistory>",
            )
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/_history",
                "<buildhistory/>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let entries = client
            .build_history("testproject", "foo", "standard", "x86_64", Some(1))
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].rev, "3");
        let entries = client
            .build_history("testproject", "foo", "standard", "x86_64", None)
            .unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn build_history_404_is_an_error_not_an_empty_list() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/build/testproject/standard/x86_64/nosuch/_history",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .build_history("testproject", "nosuch", "standard", "x86_64", None)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn job_history_builds_the_url_with_filters() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/x86_64/_jobhistory?package=foo&limit=10&code=succeeded",
            "<jobhistlist><jobhist package=\"foo\" code=\"succeeded\"/></jobhistlist>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let entries = client
            .job_history(
                "testproject",
                "standard",
                "x86_64",
                Some("foo"),
                Some(10),
                Some("succeeded"),
            )
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].code, "succeeded");
    }

    #[test]
    fn job_history_builds_the_url_without_filters() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/x86_64/_jobhistory",
            "<jobhistlist/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let entries = client
            .job_history("testproject", "standard", "x86_64", None, None, None)
            .unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn job_history_rejects_an_empty_code_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .job_history("testproject", "standard", "x86_64", None, None, Some(""))
            .is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn job_history_rejects_lastfailures_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .job_history(
                "testproject",
                "standard",
                "x86_64",
                None,
                None,
                Some("lastfailures")
            )
            .is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn parse_binary_list_reads_the_whole_build_output() {
        let parsed = parse_binary_list(
            "<binarylist><binary filename=\"foo-1.0-1.x86_64.rpm\" size=\"58352\" mtime=\"1617270174\"/><binary filename=\"foo-1.0-1.src.rpm\" size=\"1234\" mtime=\"1617270174\"/><binary filename=\"foo-debuginfo-1.0-1.x86_64.rpm\" size=\"999\" mtime=\"1617270174\"/><binary filename=\"_buildenv\" size=\"10\" mtime=\"1617270174\"/><binary filename=\"_statistics\" size=\"20\" mtime=\"1617270174\"/><binary filename=\"rpmlint.log\" size=\"30\" mtime=\"1617270174\"/></binarylist>",
        )
        .unwrap();
        assert_eq!(parsed.len(), 6);
        assert_eq!(parsed[0].name, "foo-1.0-1.x86_64.rpm");
        assert_eq!(parsed[0].size, Some(58_352));
        assert_eq!(parsed[0].mtime, Some(1_617_270_174));
        assert_eq!(parsed[1].name, "foo-1.0-1.src.rpm");
        assert_eq!(parsed[3].name, "_buildenv");
        assert_eq!(parsed[5].name, "rpmlint.log");
    }

    #[test]
    fn parse_binary_list_empty_is_an_empty_list() {
        assert_eq!(
            parse_binary_list("<binarylist/>").unwrap(),
            Vec::<BinaryFile>::new()
        );
        assert_eq!(
            parse_binary_list("<binarylist></binarylist>").unwrap(),
            Vec::<BinaryFile>::new()
        );
    }

    #[test]
    fn parse_binary_list_rejects_malformed_documents() {
        assert!(parse_binary_list("").is_err());
        assert!(parse_binary_list("<directory></directory>").is_err());
        assert!(parse_binary_list("<binarylist><binary size=\"1\"/></binarylist>").is_err());
        assert!(parse_binary_list(
            "<binarylist><binary filename=\"x\" size=\"big\"/></binarylist>"
        )
        .is_err());
    }

    #[test]
    fn parse_binary_list_accepts_an_xml_prolog() {
        // The server may emit an XML prolog before the <binarylist>
        // element; the result is the same as without one.
        let plain = "<binarylist/>";
        let prolog = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{plain}");
        assert_eq!(
            parse_binary_list(&prolog).unwrap(),
            parse_binary_list(plain).unwrap()
        );
        // Bare prolog, and whitespace/newline between prolog and element.
        assert_eq!(
            parse_binary_list(&format!("<?xml?>{plain}")).unwrap(),
            parse_binary_list(plain).unwrap()
        );
        assert_eq!(
            parse_binary_list(&format!("<?xml version=\"1.0\"?>\n{plain}")).unwrap(),
            parse_binary_list(plain).unwrap()
        );
        // A prolog that never terminates is malformed, not a missing <binarylist>.
        assert!(parse_binary_list("<?xml version=\"1.0\" <binarylist></binarylist>").is_err());
        assert!(parse_binary_list("<?xml version=\"1.0\"").is_err());
        // `<?xmlfoo` is not a prolog at all: the guard rejects it as
        // malformed rather than skipping a prefix that is not one.
        assert!(parse_binary_list("<?xmlfoo version=\"1.0\"?><binarylist></binarylist>").is_err());
    }

    #[test]
    fn parse_binary_list_malformed_error_carries_response_preview() {
        // The failure stays fail-closed, but the error names what was
        // received, so the caller can diagnose it.
        let err = parse_binary_list("garbage").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("garbage"), "{}", err.context);
        // A long body is capped: the preview carries the head, not the tail.
        let long = format!("<collection>{}</collection>", "x".repeat(3000));
        let err = parse_binary_list(&long).unwrap_err();
        assert!(err.context.contains("... [truncated,"), "{}", err.context);
        assert!(!err.context.contains(&"x".repeat(3000)), "{}", err.context);
        // An empty body still fails closed, and says so.
        let err = parse_binary_list("").unwrap_err();
        assert!(err.context.contains("<empty response>"), "{}", err.context);
    }

    #[test]
    fn parse_fileinfo_reads_the_plain_view() {
        let parsed = parse_fileinfo(
            "<fileinfo filename=\"foo-1.0-1.x86_64.rpm\"><name>foo</name><epoch>1</epoch><version>1.0</version><release>1</release><arch>x86_64</arch><summary>Foo tool</summary><description>Does foo.</description><disturl>obs://build.opensuse.org/openSUSE:Factory/standard/abc-foo-1.0-1.x86_64.rpm</disturl><size>58352</size><mtime>1617270174</mtime><provides>foo = 1.0-1</provides><provides>libc.so.6()(64bit)</provides><requires>libc.so.6()(64bit)</requires><prerequires>/bin/sh</prerequires><conflicts>oldfoo</conflicts><obsoletes>oldfoo</obsoletes><recommends>foo-doc</recommends><supplements>foo</supplements><suggests>foo-extra</suggests><enhances>foo-base</enhances></fileinfo>",
        )
        .unwrap();
        assert_eq!(parsed.filename, "foo-1.0-1.x86_64.rpm");
        assert_eq!(parsed.name.as_deref(), Some("foo"));
        assert_eq!(parsed.version.as_deref(), Some("1.0"));
        assert_eq!(parsed.release.as_deref(), Some("1"));
        assert_eq!(parsed.arch.as_deref(), Some("x86_64"));
        assert_eq!(parsed.summary.as_deref(), Some("Foo tool"));
        assert_eq!(parsed.size, Some(58_352));
        assert_eq!(parsed.mtime, Some(1_617_270_174));
        assert_eq!(
            parsed.provides,
            vec!["foo = 1.0-1".to_string(), "libc.so.6()(64bit)".to_string()]
        );
        assert_eq!(parsed.requires, vec!["libc.so.6()(64bit)".to_string()]);
        assert_eq!(parsed.epoch, Some(1));
        assert_eq!(
            parsed.disturl.as_deref(),
            Some("obs://build.opensuse.org/openSUSE:Factory/standard/abc-foo-1.0-1.x86_64.rpm")
        );
        assert_eq!(parsed.prerequires, vec!["/bin/sh".to_string()]);
        assert_eq!(parsed.conflicts, vec!["oldfoo".to_string()]);
        assert_eq!(parsed.obsoletes, vec!["oldfoo".to_string()]);
        assert_eq!(parsed.recommends, vec!["foo-doc".to_string()]);
        assert_eq!(parsed.supplements, vec!["foo".to_string()]);
        assert_eq!(parsed.suggests, vec!["foo-extra".to_string()]);
        assert_eq!(parsed.enhances, vec!["foo-base".to_string()]);
        assert!(parsed.provides_ext.is_empty());
        assert!(parsed.requires_ext.is_empty());
        assert!(parsed.recommends_ext.is_empty());
        assert!(parsed.supplements_ext.is_empty());
    }

    #[test]
    fn parse_fileinfo_reads_the_ext_view_under_both_spellings() {
        let parsed = parse_fileinfo(
            "<fileinfo filename=\"foo-1.0-1.x86_64.rpm\"><name>foo</name><provides_ext dep=\"libc.so.6()(64bit)\"><requiredby name=\"bar\" version=\"2.0\" release=\"1\" arch=\"x86_64\" project=\"p\" repository=\"r\"/></provides_ext><requires_ext dep=\"libc.so.6()(64bit)\"><provided_by name=\"glibc\" epoch=\"2\" version=\"2.38\" release=\"1\" arch=\"x86_64\" project=\"p\" repository=\"r\"/></requires_ext><requires_ext dep=\"libz.so.1()(64bit)\"><providedby name=\"zlib\" version=\"1.3\" release=\"1\" arch=\"x86_64\" project=\"p\" repository=\"r\"/><providedby name=\"zlib-32bit\" version=\"1.3\" release=\"1\" arch=\"i586\" project=\"p\" repository=\"r\"/></requires_ext><recommends_ext dep=\"foo-doc\"><provided_by name=\"foo-doc\" epoch=\"1\" version=\"1.0\" release=\"1\" arch=\"x86_64\" project=\"p\" repository=\"r\"/></recommends_ext><supplements_ext dep=\"foo-base\"><requiredby name=\"bar\" version=\"2.0\" release=\"1\" arch=\"x86_64\" project=\"p\" repository=\"r\"/></supplements_ext></fileinfo>",
        )
        .unwrap();
        assert_eq!(parsed.provides_ext.len(), 1);
        assert_eq!(parsed.provides_ext[0].dep, "libc.so.6()(64bit)");
        assert_eq!(parsed.provides_ext[0].required_by.len(), 1);
        assert_eq!(parsed.provides_ext[0].required_by[0].name, "bar");
        assert_eq!(parsed.requires_ext.len(), 2);
        assert_eq!(parsed.requires_ext[0].dep, "libc.so.6()(64bit)");
        assert_eq!(parsed.requires_ext[0].provided_by[0].name, "glibc");
        assert_eq!(parsed.requires_ext[1].provided_by[0].name, "zlib");
        assert_eq!(parsed.requires_ext[1].provided_by.len(), 2);
        assert_eq!(parsed.requires_ext[1].provided_by[1].name, "zlib-32bit");
        assert_eq!(parsed.requires_ext[0].provided_by[0].epoch, Some(2));
        assert_eq!(parsed.recommends_ext.len(), 1);
        assert_eq!(parsed.recommends_ext[0].dep, "foo-doc");
        assert_eq!(parsed.recommends_ext[0].provided_by[0].name, "foo-doc");
        assert_eq!(parsed.recommends_ext[0].provided_by[0].epoch, Some(1));
        assert_eq!(parsed.supplements_ext.len(), 1);
        assert_eq!(parsed.supplements_ext[0].dep, "foo-base");
        assert_eq!(parsed.supplements_ext[0].required_by.len(), 1);
        assert_eq!(parsed.supplements_ext[0].required_by[0].name, "bar");
    }

    #[test]
    fn parse_fileinfo_rejects_malformed_documents() {
        assert!(parse_fileinfo("").is_err());
        assert!(parse_fileinfo("<collection></collection>").is_err());
        assert!(parse_fileinfo("<fileinfo><name>foo</name></fileinfo>").is_err());
        assert!(
            parse_fileinfo("<fileinfo filename=\"x\"><provides_ext><requiredby name=\"a\"/></provides_ext></fileinfo>")
                .is_err()
        );
        assert!(parse_fileinfo("<fileinfo filename=\"x\"><size>big</size></fileinfo>").is_err());
    }

    #[test]
    fn parse_fileinfo_accepts_an_xml_prolog() {
        // The server may emit an XML prolog before the <fileinfo>
        // element; the result is the same as without one.
        let plain = "<fileinfo filename=\"foo-1.0-1.x86_64.rpm\"/>";
        let prolog = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{plain}");
        assert_eq!(
            parse_fileinfo(&prolog).unwrap(),
            parse_fileinfo(plain).unwrap()
        );
        // Bare prolog, and whitespace/newline between prolog and element.
        assert_eq!(
            parse_fileinfo(&format!("<?xml?>{plain}")).unwrap(),
            parse_fileinfo(plain).unwrap()
        );
        assert_eq!(
            parse_fileinfo(&format!("<?xml version=\"1.0\"?>\n{plain}")).unwrap(),
            parse_fileinfo(plain).unwrap()
        );
        // A prolog that never terminates is malformed, not a missing <fileinfo>.
        assert!(parse_fileinfo("<?xml version=\"1.0\" <fileinfo></fileinfo>").is_err());
        assert!(parse_fileinfo("<?xml version=\"1.0\"").is_err());
        // `<?xmlfoo` is not a prolog at all: the guard rejects it as
        // malformed rather than skipping a prefix that is not one.
        assert!(parse_fileinfo("<?xmlfoo version=\"1.0\"?><fileinfo></fileinfo>").is_err());
    }

    #[test]
    fn parse_fileinfo_malformed_error_carries_response_preview() {
        // The failure stays fail-closed, but the error names what was
        // received, so the caller can diagnose it.
        let err = parse_fileinfo("garbage").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("garbage"), "{}", err.context);
        // A long body is capped: the preview carries the head, not the tail.
        let long = format!("<collection>{}</collection>", "x".repeat(3000));
        let err = parse_fileinfo(&long).unwrap_err();
        assert!(err.context.contains("... [truncated,"), "{}", err.context);
        assert!(!err.context.contains(&"x".repeat(3000)), "{}", err.context);
        // An empty body still fails closed, and says so.
        let err = parse_fileinfo("").unwrap_err();
        assert!(err.context.contains("<empty response>"), "{}", err.context);
    }

    #[test]
    fn list_binaries_lists_the_package_and_the_whole_repository() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo",
                "<binarylist><binary filename=\"foo-1.0-1.x86_64.rpm\" size=\"58352\" mtime=\"1617270174\"/><binary filename=\"_buildenv\" size=\"10\" mtime=\"1617270174\"/></binarylist>",
            )
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/_repository",
                "<binarylist><binary filename=\"foo-1.0-1.x86_64.rpm\" size=\"58352\" mtime=\"1617270174\"/></binarylist>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let files = client
            .list_binaries("testproject", Some("foo"), "standard", "x86_64")
            .unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].name, "_buildenv");
        let files = client
            .list_binaries("testproject", None, "standard", "x86_64")
            .unwrap();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn list_binaries_404_is_an_error() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .list_binaries("testproject", Some("foo"), "standard", "x86_64")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn list_binaries_keeps_the_multibuild_flavor_in_the_path() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo:flavor1",
            "<binarylist/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .list_binaries("testproject", Some("foo:flavor1"), "standard", "x86_64")
                .unwrap(),
            Vec::<BinaryFile>::new()
        );
    }

    #[test]
    fn download_binary_returns_raw_bytes_not_text() {
        // Non-UTF-8 bytes: the download must not go through the
        // string path, which would refuse them.
        let bytes: &[u8] = &[0xED, 0xAB, 0xEE, 0xDB, 0xFF, 0xFE];
        let http = FakeHttp::new().with_get_bytes(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo/foo-1.0-1.x86_64.rpm",
            bytes,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .download_binary(
                    "testproject",
                    "foo",
                    "standard",
                    "x86_64",
                    "foo-1.0-1.x86_64.rpm"
                )
                .unwrap(),
            bytes.to_vec()
        );
        assert_eq!(
            http.get_bytes_calls(),
            vec![(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/foo-1.0-1.x86_64.rpm"
                    .to_string(),
                true
            )]
        );
    }

    #[test]
    fn download_binary_404_is_an_error_not_empty_bytes() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo/nosuch.rpm",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .download_binary("testproject", "foo", "standard", "x86_64", "nosuch.rpm")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn download_binary_rejects_an_unsafe_filename_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .download_binary("testproject", "foo", "standard", "x86_64", "../evil")
            .is_err());
        assert!(http.get_bytes_calls().is_empty());
    }

    #[test]
    fn download_binary_to_file_writes_the_body_to_disk() {
        // The streamed download writes the body to the given path
        // and returns its length; an unsafe file name is refused
        // before any HTTP call, like `download_binary`.
        let bytes: &[u8] = &[0xED, 0xAB, 0xEE, 0xDB, 0xFF, 0xFE];
        let http = FakeHttp::new().with_get_to_file(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo/foo-1.0-1.x86_64.rpm",
            bytes,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let path = std::env::temp_dir().join(format!(
            "geectl-download-binary-to-file-{}.rpm",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let err = client
            .download_binary_to_file("testproject", "foo", "standard", "x86_64", "../evil", &path)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_to_file_calls().is_empty());
        let written = client
            .download_binary_to_file(
                "testproject",
                "foo",
                "standard",
                "x86_64",
                "foo-1.0-1.x86_64.rpm",
                &path,
            )
            .unwrap();
        assert_eq!(written, bytes.len() as u64);
        assert_eq!(std::fs::read(&path).unwrap(), bytes.to_vec());
        assert_eq!(
            http.get_to_file_calls(),
            vec![(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/foo-1.0-1.x86_64.rpm"
                    .to_string(),
                true
            )]
        );
        let err = client
            .download_binary_to_file(
                "testproject",
                "foo",
                "standard",
                "x86_64",
                "foo-1.0-1.x86_64.rpm",
                &path,
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&path).unwrap(), bytes.to_vec());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parse_binary_version_list_reads_entries() {
        let parsed = parse_binary_version_list(
            "<binaryversionlist><binary name=\"foo.rpm\" hdrmd5=\"abc123\" sizek=\"58352\"/><binary name=\"nosuch\" error=\"not available\"/></binaryversionlist>",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "foo.rpm");
        assert_eq!(parsed[0].hdrmd5.as_deref(), Some("abc123"));
        assert_eq!(parsed[0].size_kb, Some(58352));
        assert_eq!(parsed[0].error, None);
        assert_eq!(parsed[1].name, "nosuch");
        assert_eq!(parsed[1].error.as_deref(), Some("not available"));
    }

    #[test]
    fn parse_binary_version_list_empty_is_an_empty_list() {
        assert_eq!(
            parse_binary_version_list("<binaryversionlist/>").unwrap(),
            Vec::<BinaryVersion>::new()
        );
        assert_eq!(
            parse_binary_version_list("<binaryversionlist></binaryversionlist>").unwrap(),
            Vec::<BinaryVersion>::new()
        );
    }

    #[test]
    fn parse_binary_version_list_rejects_malformed_documents() {
        assert!(parse_binary_version_list("").is_err());
        assert!(parse_binary_version_list("<directory></directory>").is_err());
        assert!(parse_binary_version_list("<binaryversionlist>").is_err());
        assert!(
            parse_binary_version_list("<binaryversionlist><binary/></binaryversionlist>").is_err()
        );
        assert!(parse_binary_version_list(
            "<binaryversionlist><binary name=\"\"/></binaryversionlist>"
        )
        .is_err());
        assert!(parse_binary_version_list(
            "<binaryversionlist><binary name=\"foo.rpm\" sizek=\"big\"/></binaryversionlist>"
        )
        .is_err());
        assert!(parse_binary_version_list(
            "<binaryversionlist><binary name=\"foo.rpm\"/></binaryversionlist> trailing"
        )
        .is_err());
    }

    #[test]
    fn parse_binary_version_list_accepts_an_xml_prolog() {
        // The server may emit an XML prolog before the
        // <binaryversionlist> element; the result is the same as
        // without one.
        let plain = "<binaryversionlist><binary name=\"foo.rpm\"/></binaryversionlist>";
        let prolog = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{plain}");
        assert_eq!(
            parse_binary_version_list(&prolog).unwrap(),
            parse_binary_version_list(plain).unwrap()
        );
        // A prolog that never terminates is malformed, not a missing
        // <binaryversionlist>.
        assert!(parse_binary_version_list(
            "<?xml version=\"1.0\" <binaryversionlist></binaryversionlist>"
        )
        .is_err());
        assert!(parse_binary_version_list("<?xml version=\"1.0\"").is_err());
    }

    #[test]
    fn parse_binary_version_list_malformed_error_carries_response_preview() {
        // The failure stays fail-closed, but the error names what was
        // received, so the caller can diagnose it.
        let err = parse_binary_version_list("garbage").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("garbage"), "{}", err.context);
        // A long body is capped: the preview carries the head, not the tail.
        let long = format!("<collection>{}</collection>", "x".repeat(3000));
        let err = parse_binary_version_list(&long).unwrap_err();
        assert!(err.context.contains("... [truncated,"), "{}", err.context);
        assert!(!err.context.contains(&"x".repeat(3000)), "{}", err.context);
        // An empty body still fails closed, and says so.
        let err = parse_binary_version_list("").unwrap_err();
        assert!(err.context.contains("<empty response>"), "{}", err.context);
    }

    #[test]
    fn binary_versions_reports_the_fetched_binary() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/x86_64/_repository?view=binaryversions&binary=foo",
            "<binaryversionlist><binary name=\"foo.rpm\" hdrmd5=\"abc123\" sizek=\"58352\"/></binaryversionlist>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let versions = client
            .binary_versions("testproject", "standard", "x86_64", "foo")
            .unwrap();
        assert_eq!(
            versions,
            vec![BinaryVersion {
                name: "foo.rpm".to_string(),
                hdrmd5: Some("abc123".to_string()),
                size_kb: Some(58352),
                error: None,
            }]
        );
    }

    #[test]
    fn binary_versions_reports_an_unavailable_binary_as_an_error_entry() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/x86_64/_repository?view=binaryversions&binary=nosuch",
            "<binaryversionlist><binary name=\"nosuch\" error=\"not available\"/></binaryversionlist>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let versions = client
            .binary_versions("testproject", "standard", "x86_64", "nosuch")
            .unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].error.as_deref(), Some("not available"));
    }

    #[test]
    fn binary_versions_404_is_an_error() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/build/testproject/standard/x86_64/_repository?view=binaryversions&binary=foo",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .binary_versions("testproject", "standard", "x86_64", "foo")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn binary_versions_rejects_an_empty_binary_name_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .binary_versions("testproject", "standard", "x86_64", "")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn fileinfo_reads_both_views() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/foo-1.0-1.x86_64.rpm?view=fileinfo",
                "<fileinfo filename=\"foo-1.0-1.x86_64.rpm\"><name>foo</name><version>1.0</version></fileinfo>",
            )
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/foo/foo-1.0-1.x86_64.rpm?view=fileinfo_ext",
                "<fileinfo filename=\"foo-1.0-1.x86_64.rpm\"><name>foo</name><requires_ext dep=\"libc.so.6()(64bit)\"><provided_by name=\"glibc\"/></requires_ext></fileinfo>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let info = client
            .fileinfo(
                "testproject",
                "foo",
                "standard",
                "x86_64",
                "foo-1.0-1.x86_64.rpm",
                false,
            )
            .unwrap();
        assert_eq!(info.name.as_deref(), Some("foo"));
        assert!(info.requires_ext.is_empty());
        let info = client
            .fileinfo(
                "testproject",
                "foo",
                "standard",
                "x86_64",
                "foo-1.0-1.x86_64.rpm",
                true,
            )
            .unwrap();
        assert_eq!(info.requires_ext.len(), 1);
        assert_eq!(info.requires_ext[0].provided_by.len(), 1);
        assert_eq!(info.requires_ext[0].provided_by[0].name, "glibc");
    }

    #[test]
    fn fileinfo_404_is_an_error_not_an_empty_fileinfo() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/build/testproject/standard/x86_64/foo/nosuch.rpm?view=fileinfo",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .fileinfo(
                "testproject",
                "foo",
                "standard",
                "x86_64",
                "nosuch.rpm",
                false,
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn parse_builddepinfo_reads_packages_and_ignores_cycles() {
        let parsed = parse_builddepinfo(
            "<builddepinfo><package name=\"memcached\"><source>memcached</source><pkgdep>gzip</pkgdep><pkgdep>obs-server</pkgdep><subpkg>memcached-devel</subpkg><subpkg>memcached-debugsource</subpkg></package><package name=\"gzip\"><source>gzip</source><pkgdep>zlib</pkgdep><subpkg>gzip</subpkg></package><cycle><package>memcached</package><package>gzip</package></cycle><scc><package>memcached</package></scc></builddepinfo>",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "memcached");
        assert_eq!(parsed[0].source.as_deref(), Some("memcached"));
        assert_eq!(
            parsed[0].dependencies,
            vec!["gzip".to_string(), "obs-server".to_string()]
        );
        assert_eq!(
            parsed[0].subpackages,
            vec![
                "memcached-devel".to_string(),
                "memcached-debugsource".to_string()
            ]
        );
        assert_eq!(parsed[1].name, "gzip");
        assert_eq!(parsed[1].source.as_deref(), Some("gzip"));
        assert_eq!(parsed[1].dependencies, vec!["zlib".to_string()]);
    }

    #[test]
    fn parse_builddepinfo_empty_is_an_empty_list() {
        assert_eq!(
            parse_builddepinfo("<builddepinfo/>").unwrap(),
            Vec::<PackageDeps>::new()
        );
        assert_eq!(
            parse_builddepinfo("<builddepinfo></builddepinfo>").unwrap(),
            Vec::<PackageDeps>::new()
        );
    }

    #[test]
    fn parse_builddepinfo_rejects_malformed_documents() {
        assert!(parse_builddepinfo("").is_err());
        assert!(parse_builddepinfo("<collection></collection>").is_err());
        assert!(parse_builddepinfo(
            "<builddepinfo><package><pkgdep>x</pkgdep></package></builddepinfo>"
        )
        .is_err());
    }

    #[test]
    fn parse_builddepinfo_rejects_an_inner_unclosed_package() {
        // The package close is missing but the outer close is present:
        // a missing close is an error, not an empty list.
        assert!(parse_builddepinfo(
            "<builddepinfo><package name=\"a\"><pkgdep>x</pkgdep></builddepinfo>"
        )
        .is_err());
    }

    #[test]
    fn parse_builddepinfo_accepts_an_xml_prolog() {
        // The server may emit an XML prolog before the <builddepinfo>
        // element; the result is the same as without one.
        let plain = "<builddepinfo/>";
        let prolog = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{plain}");
        assert_eq!(
            parse_builddepinfo(&prolog).unwrap(),
            parse_builddepinfo(plain).unwrap()
        );
        // Bare prolog, and whitespace/newline between prolog and element.
        assert_eq!(
            parse_builddepinfo(&format!("<?xml?>{plain}")).unwrap(),
            parse_builddepinfo(plain).unwrap()
        );
        assert_eq!(
            parse_builddepinfo(&format!("<?xml version=\"1.0\"?>\n{plain}")).unwrap(),
            parse_builddepinfo(plain).unwrap()
        );
        // A prolog that never terminates is malformed, not a missing <builddepinfo>.
        assert!(parse_builddepinfo("<?xml version=\"1.0\" <builddepinfo></builddepinfo>").is_err());
        assert!(parse_builddepinfo("<?xml version=\"1.0\"").is_err());
        // `<?xmlfoo` is not a prolog at all: the guard rejects it as
        // malformed rather than skipping a prefix that is not one.
        assert!(
            parse_builddepinfo("<?xmlfoo version=\"1.0\"?><builddepinfo></builddepinfo>").is_err()
        );
    }

    #[test]
    fn parse_builddepinfo_malformed_error_carries_response_preview() {
        // The failure stays fail-closed, but the error names what was
        // received, so the caller can diagnose it.
        let err = parse_builddepinfo("garbage").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("garbage"), "{}", err.context);
        // A long body is capped: the preview carries the head, not the tail.
        let long = format!("<collection>{}</collection>", "x".repeat(3000));
        let err = parse_builddepinfo(&long).unwrap_err();
        assert!(err.context.contains("... [truncated,"), "{}", err.context);
        assert!(!err.context.contains(&"x".repeat(3000)), "{}", err.context);
        // An empty body still fails closed, and says so.
        let err = parse_builddepinfo("").unwrap_err();
        assert!(err.context.contains("<empty response>"), "{}", err.context);
    }

    #[test]
    fn parse_repositories_reads_repos_arches_and_paths() {
        let parsed = parse_repositories(
            "<project name=\"testproject\"><title>t</title><repository name=\"standard\"><arch>x86_64</arch><arch>i586</arch><path project=\"openSUSE.org:openSUSE:Tumbleweed\" repository=\"standard\"/></repository><repository name=\"ports\"><arch>aarch64</arch><path project=\"openSUSE.org:openSUSE:Tumbleweed\" repository=\"ports\"/></repository></project>",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "standard");
        assert_eq!(
            parsed[0].arch,
            vec!["x86_64".to_string(), "i586".to_string()]
        );
        assert_eq!(parsed[0].paths.len(), 1);
        assert_eq!(parsed[0].paths[0].0, "openSUSE.org:openSUSE:Tumbleweed");
        assert_eq!(parsed[0].paths[0].1, "standard");
        assert_eq!(parsed[1].name, "ports");
        assert_eq!(parsed[1].arch, vec!["aarch64".to_string()]);
    }

    #[test]
    fn parse_repositories_reads_the_full_repository_shape() {
        let parsed = parse_repositories(
            "<project name=\"testproject\"><repository name=\"standard\" rebuild=\"local\" block=\"never\" linkedbuild=\"all\"><arch>x86_64</arch><path project=\"openSUSE:Factory\" repository=\"snapshot\"/><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><archfilter>x86_64</archfilter><master url=\"https://master.example.com\" sslfingerprint=\"AB:CD\"/><pubkey>key-material</pubkey></download><releasetarget project=\"openSUSE:Leap:15.6\" repository=\"standard\" trigger=\"manual\"/><hostsystem project=\"openSUSE:Factory\" repository=\"snapshot\"/></repository></project>",
        )
        .unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(
            parsed[0],
            Repository {
                name: "standard".to_string(),
                arch: vec!["x86_64".to_string()],
                paths: vec![("openSUSE:Factory".to_string(), "snapshot".to_string())],
                rebuild: Some("local".to_string()),
                block: Some("never".to_string()),
                linkedbuild: Some("all".to_string()),
                downloads: vec![RepositoryDownload {
                    arch: Some("x86_64".to_string()),
                    url: "https://example.com".to_string(),
                    repotype: Some("rpmmd".to_string()),
                    archfilter: Some("x86_64".to_string()),
                    pubkey: Some("key-material".to_string()),
                    master: Some(DownloadMaster {
                        url: "https://master.example.com".to_string(),
                        sslfingerprint: Some("AB:CD".to_string()),
                    }),
                }],
                release_targets: vec![ReleaseTarget {
                    project: "openSUSE:Leap:15.6".to_string(),
                    repository: Some("standard".to_string()),
                    trigger: Some("manual".to_string()),
                }],
                hostsystem: vec![("openSUSE:Factory".to_string(), "snapshot".to_string())],
            }
        );
        // Rendered back, the repository is byte-identical.
        assert_eq!(
            repository_element(&parsed[0]),
            "<repository name=\"standard\" rebuild=\"local\" block=\"never\" linkedbuild=\"all\"><arch>x86_64</arch><path project=\"openSUSE:Factory\" repository=\"snapshot\"/><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><archfilter>x86_64</archfilter><master url=\"https://master.example.com\" sslfingerprint=\"AB:CD\"/><pubkey>key-material</pubkey></download><releasetarget project=\"openSUSE:Leap:15.6\" repository=\"standard\" trigger=\"manual\"/><hostsystem project=\"openSUSE:Factory\" repository=\"snapshot\"/></repository>"
        );
    }

    #[test]
    fn repository_element_drops_empty_optional_values() {
        let repo = Repository {
            name: "standard".to_string(),
            arch: vec!["x86_64".to_string()],
            paths: vec![("openSUSE:Factory".to_string(), "snapshot".to_string())],
            rebuild: Some(String::new()),
            block: Some(String::new()),
            linkedbuild: Some(String::new()),
            downloads: vec![RepositoryDownload {
                arch: Some(String::new()),
                url: "https://example.com".to_string(),
                repotype: Some(String::new()),
                archfilter: None,
                pubkey: None,
                master: Some(DownloadMaster {
                    url: "https://master.example.com".to_string(),
                    sslfingerprint: Some(String::new()),
                }),
            }],
            release_targets: vec![ReleaseTarget {
                project: "openSUSE:Leap:15.6".to_string(),
                repository: Some(String::new()),
                trigger: Some(String::new()),
            }],
            hostsystem: vec![("openSUSE:Factory".to_string(), "snapshot".to_string())],
        };
        assert_eq!(
            repository_element(&repo),
            "<repository name=\"standard\"><arch>x86_64</arch><path project=\"openSUSE:Factory\" repository=\"snapshot\"/><download url=\"https://example.com\"><master url=\"https://master.example.com\"/></download><releasetarget project=\"openSUSE:Leap:15.6\"/><hostsystem project=\"openSUSE:Factory\" repository=\"snapshot\"/></repository>"
        );
    }

    #[test]
    fn parse_repositories_reads_multiple_hostsystems() {
        let parsed = parse_repositories(
            "<project name=\"testproject\"><repository name=\"standard\"><arch>x86_64</arch><hostsystem project=\"openSUSE:Factory\" repository=\"snapshot\"/><hostsystem project=\"openSUSE:Factory:ARM\" repository=\"standard\"/></repository></project>",
        )
        .unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(
            parsed[0].hostsystem,
            vec![
                ("openSUSE:Factory".to_string(), "snapshot".to_string()),
                ("openSUSE:Factory:ARM".to_string(), "standard".to_string()),
            ]
        );
        let rendered = repository_element(&parsed[0]);
        assert!(
            rendered.contains("<hostsystem project=\"openSUSE:Factory\" repository=\"snapshot\"/>")
        );
        assert!(rendered
            .contains("<hostsystem project=\"openSUSE:Factory:ARM\" repository=\"standard\"/>"));
        assert_eq!(
            parse_repositories(&format!(
                "<project name=\"testproject\">{rendered}</project>"
            ))
            .unwrap(),
            parsed
        );
    }

    #[test]
    fn parse_repositories_round_trips_a_template_ordered_repository() {
        // The server emits children in template order (download,
        // releasetarget, path, hostsystem, arch); the renderer emits
        // arch first. The typed value survives the reorder.
        let parsed = parse_repositories(
            "<project name=\"testproject\"><repository name=\"standard\" rebuild=\"local\"><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"/><releasetarget project=\"openSUSE:Leap:15.6\" repository=\"standard\" trigger=\"manual\"/><path project=\"openSUSE:Factory\" repository=\"snapshot\"/><hostsystem project=\"openSUSE:Factory\" repository=\"snapshot\"/><arch>x86_64</arch></repository></project>",
        )
        .unwrap();
        assert_eq!(parsed.len(), 1);
        let rendered = repository_element(&parsed[0]);
        let reparsed = parse_repositories(&format!(
            "<project name=\"testproject\">{rendered}</project>"
        ))
        .unwrap();
        assert_eq!(reparsed, parsed);
    }

    #[test]
    fn both_repository_parsers_reject_children_without_required_values() {
        // Both parsers fail closed on the same malformed children.
        for body in [
            "<project name=\"p\"><repository><arch>x86_64</arch></repository></project>",
            "<project name=\"p\"><repository name=\"\"><arch>x86_64</arch></repository></project>",
            "<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch><download arch=\"x86_64\" repotype=\"rpmmd\"/></repository></project>",
            "<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><master sslfingerprint=\"AB:CD\"/></download></repository></project>",
            "<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch><releasetarget repository=\"standard\"/></repository></project>",
            "<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch><hostsystem project=\"openSUSE:Factory\"/></repository></project>",
            "<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch><path project=\"openSUSE:Factory\"/></repository></project>",
        ] {
            assert!(parse_project_meta(body).is_err(), "{body}");
            assert!(parse_repositories(body).is_err(), "{body}");
        }
    }

    #[test]
    fn both_repository_parsers_agree_on_empty_attribute_values() {
        let body = "<project name=\"testproject\"><repository name=\"standard\" rebuild=\"\" block=\"\" linkedbuild=\"\"><arch>x86_64</arch><download arch=\"\" url=\"https://example.com\" repotype=\"\"><archfilter> </archfilter><master url=\"https://master.example.com\" sslfingerprint=\"\"/><pubkey> </pubkey></download><releasetarget project=\"openSUSE:Leap:15.6\" repository=\"\" trigger=\"\"/><hostsystem project=\"openSUSE:Factory\" repository=\"snapshot\"/></repository></project>";
        let via_project_meta = parse_project_meta(body).unwrap();
        let via_repositories = parse_repositories(body).unwrap();
        assert_eq!(via_project_meta.repositories, via_repositories);
        let repo = &via_project_meta.repositories[0];
        assert_eq!(repo.rebuild, None);
        assert_eq!(repo.block, None);
        assert_eq!(repo.linkedbuild, None);
        assert_eq!(repo.downloads.len(), 1);
        assert_eq!(repo.downloads[0].arch, None);
        assert_eq!(repo.downloads[0].repotype, None);
        assert_eq!(repo.downloads[0].archfilter, None);
        assert_eq!(repo.downloads[0].pubkey, None);
        assert_eq!(
            repo.downloads[0].master.as_ref().unwrap().sslfingerprint,
            None
        );
        assert_eq!(repo.release_targets[0].repository, None);
        assert_eq!(repo.release_targets[0].trigger, None);
    }

    #[test]
    fn repository_element_escapes_values_in_the_full_shape() {
        let repo = Repository {
            name: "standard".to_string(),
            arch: vec!["x86_64".to_string()],
            paths: vec![("openSUSE:Factory".to_string(), "snapshot".to_string())],
            rebuild: Some("local".to_string()),
            block: None,
            linkedbuild: None,
            downloads: vec![RepositoryDownload {
                arch: Some("x86_64".to_string()),
                url: "https://example.com/?a=1&b=2".to_string(),
                repotype: Some("rpmmd".to_string()),
                archfilter: None,
                pubkey: None,
                master: Some(DownloadMaster {
                    url: "https://master.example.com/?x=\"y\"".to_string(),
                    sslfingerprint: None,
                }),
            }],
            release_targets: vec![ReleaseTarget {
                project: "openSUSE:Leap:15.6".to_string(),
                repository: Some("standard".to_string()),
                trigger: Some("a<b".to_string()),
            }],
            hostsystem: vec![("openSUSE:Factory".to_string(), "snapshot".to_string())],
        };
        assert_eq!(
            repository_element(&repo),
            "<repository name=\"standard\" rebuild=\"local\"><arch>x86_64</arch><path project=\"openSUSE:Factory\" repository=\"snapshot\"/><download arch=\"x86_64\" url=\"https://example.com/?a=1&amp;b=2\" repotype=\"rpmmd\"><master url=\"https://master.example.com/?x=&quot;y&quot;\"/></download><releasetarget project=\"openSUSE:Leap:15.6\" repository=\"standard\" trigger=\"a&lt;b\"/><hostsystem project=\"openSUSE:Factory\" repository=\"snapshot\"/></repository>"
        );
    }

    #[test]
    fn parse_repositories_round_trips_entity_escaped_values() {
        // The escape direction is pinned by
        // `repository_element_escapes_values_in_the_full_shape`; this
        // pins the parse direction: entity-containing XML parses back
        // to the unescaped values and re-renders to the same escaped
        // output.
        let body = "<project name=\"testproject\"><repository name=\"standard\" rebuild=\"local\"><arch>x86_64</arch><path project=\"openSUSE:Factory\" repository=\"snapshot\"/><download arch=\"x86_64\" url=\"https://example.com/?a=1&amp;b=2\" repotype=\"rpmmd\"><archfilter>a&amp;b</archfilter><master url=\"https://master.example.com/?x=&quot;y&quot;\"/></download><releasetarget project=\"openSUSE:Leap:15.6\" repository=\"standard\" trigger=\"a&lt;b&gt;c&apos;d\"/><hostsystem project=\"openSUSE:Factory\" repository=\"snapshot\"/></repository></project>";
        let parsed = parse_repositories(body).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].downloads[0].url, "https://example.com/?a=1&b=2");
        assert_eq!(
            parsed[0].downloads[0].master.as_ref().unwrap().url,
            "https://master.example.com/?x=\"y\""
        );
        assert_eq!(parsed[0].downloads[0].archfilter.as_deref(), Some("a&b"));
        assert_eq!(
            parsed[0].release_targets[0].trigger.as_deref(),
            Some("a<b>c'd")
        );
        let rendered = repository_element(&parsed[0]);
        assert_eq!(
            rendered,
            "<repository name=\"standard\" rebuild=\"local\"><arch>x86_64</arch><path project=\"openSUSE:Factory\" repository=\"snapshot\"/><download arch=\"x86_64\" url=\"https://example.com/?a=1&amp;b=2\" repotype=\"rpmmd\"><archfilter>a&amp;b</archfilter><master url=\"https://master.example.com/?x=&quot;y&quot;\"/></download><releasetarget project=\"openSUSE:Leap:15.6\" repository=\"standard\" trigger=\"a&lt;b&gt;c&apos;d\"/><hostsystem project=\"openSUSE:Factory\" repository=\"snapshot\"/></repository>"
        );
        // Re-parsing the re-rendered output is stable, and the project
        // meta parser agrees.
        assert_eq!(
            parse_repositories(&format!(
                "<project name=\"testproject\">{rendered}</project>"
            ))
            .unwrap(),
            parsed
        );
        assert_eq!(parse_project_meta(body).unwrap().repositories, parsed);
    }

    #[test]
    fn parse_repositories_empty_is_an_empty_list() {
        assert_eq!(
            parse_repositories("<project name=\"p\"/>").unwrap(),
            Vec::<Repository>::new()
        );
        assert_eq!(
            parse_repositories("<project name=\"p\"><title>t</title></project>").unwrap(),
            Vec::<Repository>::new()
        );
    }

    #[test]
    fn parse_repositories_rejects_children_without_required_values() {
        // A child missing a required value is malformed, not a child
        // to drop: the server always names them.
        for body in [
            "<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch><download arch=\"x86_64\" repotype=\"rpmmd\"/></repository></project>",
            "<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><master sslfingerprint=\"AB:CD\"/></download></repository></project>",
            "<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch><releasetarget repository=\"standard\"/></repository></project>",
            "<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch><hostsystem project=\"openSUSE:Factory\"/></repository></project>",
            "<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch><path project=\"openSUSE:Factory\"/></repository></project>",
        ] {
            assert!(parse_repositories(body).is_err(), "{body}");
        }
    }

    #[test]
    fn parse_repositories_distinguishes_an_empty_url_from_a_missing_one() {
        let missing_download = "<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch><download arch=\"x86_64\" repotype=\"rpmmd\"/></repository></project>";
        let empty_download = "<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"\" repotype=\"rpmmd\"/></repository></project>";
        assert!(parse_repositories(missing_download)
            .unwrap_err()
            .context
            .contains("<download> without url"));
        assert!(parse_repositories(empty_download)
            .unwrap_err()
            .context
            .contains("<download> with empty url"));
        let missing_master = "<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><master sslfingerprint=\"AB:CD\"/></download></repository></project>";
        let empty_master = "<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><master url=\"\" sslfingerprint=\"AB:CD\"/></download></repository></project>";
        assert!(parse_repositories(missing_master)
            .unwrap_err()
            .context
            .contains("<master> without url"));
        assert!(parse_repositories(empty_master)
            .unwrap_err()
            .context
            .contains("<master> with empty url"));
    }
    #[test]
    fn parse_repositories_keep_the_first_duplicate_download_child() {
        // `<master>`, `<archfilter>` and `<pubkey>` are singular in
        // the typed model: the parse keeps the first of each, so a
        // read never fails on server-accepted state — only a rewrite
        // refuses the duplicate (see the meta write tests).
        for (body, child) in [
            ("<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><master url=\"https://a.example.com\"/><master url=\"https://b.example.com\"/></download></repository></project>", "master"),
            ("<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><archfilter>x86_64</archfilter><archfilter>aarch64</archfilter></download></repository></project>", "archfilter"),
            ("<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch><download arch=\"x86_64\" url=\"https://example.com\" repotype=\"rpmmd\"><pubkey>key-one</pubkey><pubkey>key-two</pubkey></download></repository></project>", "pubkey"),
        ] {
            for parsed in [
                parse_repositories(body).unwrap(),
                parse_project_meta(body).unwrap().repositories,
            ] {
                let download = &parsed[0].downloads[0];
                match child {
                    "master" => assert_eq!(
                        download.master.as_ref().unwrap().url,
                        "https://a.example.com"
                    ),
                    "archfilter" => {
                        assert_eq!(download.archfilter.as_deref(), Some("x86_64"))
                    }
                    "pubkey" => {
                        assert_eq!(download.pubkey.as_deref(), Some("key-one"))
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    #[test]
    fn parse_repositories_rejects_malformed_documents() {
        assert!(parse_repositories("").is_err());
        assert!(parse_repositories("<collection></collection>").is_err());
        assert!(parse_repositories(
            "<project><repository><arch>x86_64</arch></repository></project>"
        )
        .is_err());
        assert!(
            parse_repositories(
                "<project><repository name=\"r\"><path repository=\"standard\"/></repository></project>"
            )
            .is_err()
        );
    }

    #[test]
    fn parse_repositories_rejects_an_inner_unclosed_repository() {
        // The repository close is missing but the outer close is
        // present: a missing close is an error, not an empty list.
        assert!(parse_repositories(
            "<project name=\"p\"><repository name=\"r\"><path project=\"q\" repository=\"s\"/></project>"
        )
        .is_err());
    }

    #[test]
    fn parse_repositories_accepts_an_xml_prolog() {
        // The server may emit an XML prolog before the <project>
        // element; the result is the same as without one.
        let plain = "<project name=\"testproject\"/>";
        let prolog = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{plain}");
        assert_eq!(
            parse_repositories(&prolog).unwrap(),
            parse_repositories(plain).unwrap()
        );
        // Bare prolog, and whitespace/newline between prolog and element.
        assert_eq!(
            parse_repositories(&format!("<?xml?>{plain}")).unwrap(),
            parse_repositories(plain).unwrap()
        );
        assert_eq!(
            parse_repositories(&format!("<?xml version=\"1.0\"?>\n{plain}")).unwrap(),
            parse_repositories(plain).unwrap()
        );
        // A prolog that never terminates is malformed, not a missing <project>.
        assert!(parse_repositories("<?xml version=\"1.0\" <project></project>").is_err());
        assert!(parse_repositories("<?xml version=\"1.0\"").is_err());
        // `<?xmlfoo` is not a prolog at all: the guard rejects it as
        // malformed rather than skipping a prefix that is not one.
        assert!(parse_repositories("<?xmlfoo version=\"1.0\"?><project></project>").is_err());
    }

    #[test]
    fn parse_repositories_malformed_error_carries_response_preview() {
        // The failure stays fail-closed, but the error names what was
        // received, so the caller can diagnose it.
        let err = parse_repositories("garbage").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("garbage"), "{}", err.context);
        // A long body is capped: the preview carries the head, not the tail.
        let long = format!("<collection>{}</collection>", "x".repeat(3000));
        let err = parse_repositories(&long).unwrap_err();
        assert!(err.context.contains("... [truncated,"), "{}", err.context);
        assert!(!err.context.contains(&"x".repeat(3000)), "{}", err.context);
        // An empty body still fails closed, and says so.
        let err = parse_repositories("").unwrap_err();
        assert!(err.context.contains("<empty response>"), "{}", err.context);
    }

    #[test]
    fn dependson_builds_the_url_with_and_without_a_package() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/_builddepinfo?view=pkgnames&package=foo",
                "<builddepinfo><package name=\"foo\"><pkgdep>bar</pkgdep><subpkg>foo</subpkg></package></builddepinfo>",
            )
            .with_get(
                "https://api.opensuse.org/build/testproject/standard/x86_64/_builddepinfo?view=pkgnames",
                "<builddepinfo/>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let deps = client
            .dependson("testproject", "standard", "x86_64", Some("foo"))
            .unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].dependencies, vec!["bar".to_string()]);
        let deps = client
            .dependson("testproject", "standard", "x86_64", None)
            .unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn whatdependson_builds_the_reverse_url() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/build/testproject/standard/x86_64/_builddepinfo?view=revpkgnames&package=bar",
            "<builddepinfo><package name=\"foo\"><pkgdep>bar</pkgdep></package></builddepinfo>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let deps = client
            .whatdependson("testproject", "standard", "x86_64", "bar")
            .unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "foo");
    }

    #[test]
    fn dependson_404_is_an_error_not_an_empty_list() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/build/testproject/standard/x86_64/_builddepinfo?view=pkgnames",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .dependson("testproject", "standard", "x86_64", None)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn check_constraints_posts_the_constraints_and_reads_worker_ids() {
        let constraints = "<constraints><hardware><disk><size unit=\"G\">10</size></disk></hardware></constraints>";
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/worker?cmd=checkconstraints&project=testproject&package=foo&repository=standard&arch=x86_64",
            "<directory><entry name=\"worker1\"/><entry name=\"worker2\"/></directory>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let workers = client
            .check_constraints(
                "testproject",
                "foo",
                "standard",
                "x86_64",
                Some(constraints),
            )
            .unwrap();
        assert_eq!(workers, vec!["worker1".to_string(), "worker2".to_string()]);
        assert_eq!(
            http.post_calls(),
            vec![(
                "https://api.opensuse.org/worker?cmd=checkconstraints&project=testproject&package=foo&repository=standard&arch=x86_64"
                    .to_string(),
                constraints.to_string(),
                true
            )]
        );
    }

    #[test]
    fn check_constraints_non_directory_body_is_an_error() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/worker?cmd=checkconstraints&project=testproject&package=foo&repository=standard&arch=x86_64",
            "<collection></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .check_constraints("testproject", "foo", "standard", "x86_64", None)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn check_constraints_without_constraints_posts_an_empty_body() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/worker?cmd=checkconstraints&project=testproject&package=foo&repository=standard&arch=x86_64",
            "<directory/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let workers = client
            .check_constraints("testproject", "foo", "standard", "x86_64", None)
            .unwrap();
        assert!(workers.is_empty());
        assert_eq!(http.post_calls()[0].1, String::new());
    }

    #[test]
    fn check_constraints_rejects_empty_constraints_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .check_constraints("testproject", "foo", "standard", "x86_64", Some(""))
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn list_repositories_reads_the_public_meta() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/testproject/_meta",
            "<project name=\"testproject\"><repository name=\"standard\"><arch>x86_64</arch><arch>i586</arch><path project=\"openSUSE.org:openSUSE:Tumbleweed\" repository=\"standard\"/></repository></project>",
        );
        let client = ObsClient::new(&http);
        let repos = client.list_repositories("testproject").unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "standard");
        assert_eq!(
            repos[0].arch,
            vec!["x86_64".to_string(), "i586".to_string()]
        );
        // The public read carries no credentials.
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/public/source/testproject/_meta".to_string(),
                false
            )]
        );
    }

    #[test]
    fn list_repositories_404_is_an_error_not_an_empty_list() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/public/source/nosuchproject/_meta",
            ErrorKind::NotFound,
        );
        let client = ObsClient::new(&http);
        let err = client.list_repositories("nosuchproject").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn list_repositories_rejects_an_invalid_name_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client.list_repositories("bad/project").is_err());
        assert!(http.get_calls().is_empty());
    }

    const PROJECT_META: &str = "<project name=\"test:factory\"><title>Test factory</title><description>A test project</description><repository name=\"standard\"><arch>x86_64</arch><arch>i586</arch><path project=\"openSUSE.org:openSUSE:Tumbleweed\" repository=\"standard\"/></repository></project>";

    const PROJECT_META_WITH_PUBLISH: &str = "<project name=\"test:factory\"><title>Test factory</title><publish><disable repository=\"standard\"/></publish><repository name=\"standard\"><arch>x86_64</arch><path project=\"openSUSE.org:openSUSE:Tumbleweed\" repository=\"standard\"/></repository></project>";

    fn new_repo() -> Repository {
        Repository {
            name: "new-repo".to_string(),
            arch: vec!["x86_64".to_string(), "aarch64".to_string()],
            paths: vec![
                ("test:factory".to_string(), "standard".to_string()),
                ("test:devel".to_string(), "standard".to_string()),
            ],
            rebuild: None,
            block: None,
            linkedbuild: None,
            downloads: vec![],
            release_targets: vec![],
            hostsystem: vec![],
        }
    }

    #[test]
    fn add_repository_puts_the_edited_meta() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/_meta",
                PROJECT_META,
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/_meta",
                "<status code=\"ok\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .add_repository("test:factory", &new_repo(), false, &DryRun::new(false))
            .unwrap());
        // One PUT, to the meta URL, with the credentials, carrying the
        // old repository plus the new one — and nothing else changed.
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].0,
            "https://api.opensuse.org/source/test:factory/_meta"
        );
        assert!(puts[0].2);
        let repos = parse_repositories(&puts[0].1).unwrap();
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].name, "standard");
        assert_eq!(repos[1], new_repo());
        assert!(puts[0].1.contains("<title>Test factory</title>"));
        assert!(puts[0]
            .1
            .contains("<description>A test project</description>"));
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/source/test:factory/_meta".to_string(),
                true
            )]
        );
    }

    #[test]
    fn add_repository_duplicate_is_an_error_without_a_put() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/test:factory/_meta",
            PROJECT_META,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let repo = Repository {
            name: "standard".to_string(),
            arch: vec!["x86_64".to_string()],
            paths: vec![("test:factory".to_string(), "standard".to_string())],
            rebuild: None,
            block: None,
            linkedbuild: None,
            downloads: vec![],
            release_targets: vec![],
            hostsystem: vec![],
        };
        let err = client
            .add_repository("test:factory", &repo, false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
        assert!(err.context.contains("already exists"), "{}", err.context);
        // The meta was read, but nothing was written.
        assert_eq!(http.get_calls().len(), 1);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn add_repository_with_disable_publish_adds_the_flag() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/_meta",
                PROJECT_META_WITH_PUBLISH,
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/_meta",
                "<status code=\"ok\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .add_repository("test:factory", &new_repo(), true, &DryRun::new(false))
            .unwrap());
        let body = &http.put_calls()[0].1;
        // The existing disable entry survives, and the new one joins it
        // in the same <publish> element.
        assert!(body.contains("<disable repository=\"standard\"/>"));
        assert!(body.contains("<disable repository=\"new-repo\"/>"));
        assert_eq!(body.matches("<publish>").count(), 1);
    }

    #[test]
    fn add_repository_without_disable_publish_leaves_publish_alone() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/_meta",
                PROJECT_META_WITH_PUBLISH,
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/_meta",
                "<status code=\"ok\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .add_repository("test:factory", &new_repo(), false, &DryRun::new(false))
            .unwrap());
        let body = &http.put_calls()[0].1;
        // The existing disable entry survives, and no new one appears.
        assert!(body.contains("<disable repository=\"standard\"/>"));
        assert_eq!(body.matches("<disable").count(), 1);
    }

    #[test]
    fn add_repository_dry_run_records_the_plan_and_writes_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        assert!(!client
            .add_repository("test:factory", &new_repo(), false, &dry_run)
            .unwrap());
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: add repository new-repo to project test:factory".to_string()]
        );
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
        // With publishing disabled, the plan says so.
        let dry_run = DryRun::new(true);
        assert!(!client
            .add_repository("test:factory", &new_repo(), true, &dry_run)
            .unwrap());
        assert_eq!(
            dry_run.plan(),
            vec![
                "[dry-run] would: add repository new-repo to project test:factory with publishing disabled"
                    .to_string()
            ]
        );
    }

    #[test]
    fn add_repository_rejects_invalid_input_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(false);
        assert_eq!(
            client
                .add_repository("bad/project", &new_repo(), false, &dry_run)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
        let mut bad_name = new_repo();
        bad_name.name = "bad/name".to_string();
        let mut no_arch = new_repo();
        no_arch.arch = vec![];
        let mut bad_arch = new_repo();
        bad_arch.arch = vec!["bad/arch".to_string()];
        let mut no_paths = new_repo();
        no_paths.paths = vec![];
        let mut bad_path_project = new_repo();
        bad_path_project.paths = vec![("bad/project".to_string(), "standard".to_string())];
        let mut bad_path_repo = new_repo();
        bad_path_repo.paths = vec![("test:factory".to_string(), "bad/repo".to_string())];
        for repo in [
            bad_name,
            no_arch,
            bad_arch,
            no_paths,
            bad_path_project,
            bad_path_repo,
        ] {
            assert_eq!(
                client
                    .add_repository("test:factory", &repo, false, &dry_run)
                    .unwrap_err()
                    .kind(),
                ErrorKind::InvalidInput,
                "repository {repo:?} must be rejected"
            );
        }
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn add_repository_without_credentials_is_a_config_error() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let err = client
            .add_repository("test:factory", &new_repo(), false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn add_repository_non_ok_put_is_a_server_error() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/_meta",
                PROJECT_META,
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/_meta",
                "<status code=\"error\"><summary>nope</summary></status>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .add_repository("test:factory", &new_repo(), false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("nope"), "{}", err.context);
    }

    #[test]
    fn remove_repository_puts_the_edited_meta() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory/_meta",
                PROJECT_META_WITH_PUBLISH,
            )
            .with_put(
                "https://api.opensuse.org/source/test:factory/_meta",
                "<status code=\"ok\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .remove_repository("test:factory", "standard", &DryRun::new(false))
            .unwrap());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        // No repositories left, and the publish section went with the
        // last disabled repository — no empty flag section remains.
        assert!(parse_repositories(&puts[0].1).unwrap().is_empty());
        assert!(!puts[0].1.contains("<publish"));
        assert!(!puts[0].1.contains("<repository"));
        assert!(puts[0].1.contains("<title>Test factory</title>"));
    }

    #[test]
    fn remove_repository_keeps_other_repos_and_publish_entries() {
        let meta = "<project name=\"test:factory\"><title>Test factory</title><publish><disable repository=\"standard\"/><disable repository=\"ports\"/></publish><repository name=\"standard\"><arch>x86_64</arch></repository><repository name=\"ports\"><arch>aarch64</arch></repository></project>";
        let http = FakeHttp::new()
            .with_get("https://api.opensuse.org/source/test:factory/_meta", meta)
            .with_put(
                "https://api.opensuse.org/source/test:factory/_meta",
                "<status code=\"ok\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .remove_repository("test:factory", "ports", &DryRun::new(false))
            .unwrap());
        let body = &http.put_calls()[0].1;
        let repos = parse_repositories(body).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "standard");
        assert!(body.contains("<disable repository=\"standard\"/>"));
        assert!(!body.contains("<disable repository=\"ports\"/>"));
    }

    #[test]
    fn remove_repository_missing_name_is_ok_false_without_a_put() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/test:factory/_meta",
            PROJECT_META,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(!client
            .remove_repository("test:factory", "does-not-exist", &DryRun::new(false))
            .unwrap());
        assert_eq!(http.get_calls().len(), 1);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn remove_repository_dry_run_records_the_plan_and_writes_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        assert!(!client
            .remove_repository("test:factory", "standard", &dry_run)
            .unwrap());
        assert_eq!(
            dry_run.plan(),
            vec![
                "[dry-run] would: remove repository standard from project test:factory".to_string()
            ]
        );
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn remove_repository_rejects_invalid_input_and_missing_credentials_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(false);
        assert_eq!(
            client
                .remove_repository("bad/project", "standard", &dry_run)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            client
                .remove_repository("test:factory", "bad/name", &dry_run)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
        assert!(http.get_calls().is_empty());
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .remove_repository("test:factory", "standard", &dry_run)
                .unwrap_err()
                .kind(),
            ErrorKind::ConfigError
        );
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn repository_element_escapes_values() {
        let repo = Repository {
            name: "a&b".to_string(),
            arch: vec!["x86_64".to_string()],
            paths: vec![("p&q".to_string(), "r\"s".to_string())],
            rebuild: None,
            block: None,
            linkedbuild: None,
            downloads: vec![],
            release_targets: vec![],
            hostsystem: vec![],
        };
        assert_eq!(
            repository_element(&repo),
            "<repository name=\"a&amp;b\"><arch>x86_64</arch><path project=\"p&amp;q\" repository=\"r&quot;s\"/></repository>"
        );
    }

    #[test]
    fn repository_settings_validation_rejects_invalid_values() {
        let mut repo = new_repo();
        repo.rebuild = Some("bogus".to_string());
        assert!(validate_repository_settings(&repo).is_err());
        assert!(validate_repository(&repo).is_err());
        repo.rebuild = Some(String::new());
        assert!(validate_repository_settings(&repo).is_ok());
        repo.rebuild = Some("local".to_string());
        repo.linkedbuild = Some("bogus".to_string());
        assert!(validate_repository_settings(&repo).is_err());
        repo.linkedbuild = Some(String::new());
        assert!(validate_repository_settings(&repo).is_ok());
        repo.linkedbuild = Some("all".to_string());
        repo.block = Some("bogus".to_string());
        assert!(validate_repository_settings(&repo).is_err());
        repo.block = Some(String::new());
        assert!(validate_repository_settings(&repo).is_ok());
        for block in ["all", "local", "never"] {
            repo.block = Some(block.to_string());
            assert!(validate_repository_settings(&repo).is_ok());
        }
        repo.block = None;
        // A download needs its arch, in the repository's architectures.
        repo.downloads = vec![RepositoryDownload {
            arch: None,
            url: "https://example.com".to_string(),
            repotype: Some("rpmmd".to_string()),
            archfilter: None,
            pubkey: None,
            master: None,
        }];
        assert!(validate_repository_settings(&repo).is_err());
        repo.downloads[0].arch = Some("bad arch!".to_string());
        assert!(validate_repository_settings(&repo).is_err());
        repo.downloads[0].arch = Some("i586".to_string());
        assert!(validate_repository_settings(&repo).is_err());
        repo.downloads[0].arch = Some("x86_64".to_string());
        assert!(validate_repository_settings(&repo).is_ok());
        // A second download on the same arch is a duplicate: the
        // arch is in the repository's architectures, so only the
        // dedup check rejects it.
        repo.downloads.push(RepositoryDownload {
            arch: Some("x86_64".to_string()),
            url: "https://other.example.com".to_string(),
            repotype: Some("deb".to_string()),
            archfilter: None,
            pubkey: None,
            master: None,
        });
        assert!(validate_repository_settings(&repo).is_err());
        repo.downloads.pop();
        // The url needs a scheme; the repotype is required and one of
        // the server's types.
        repo.downloads[0].url = "example.com/no-scheme".to_string();
        assert!(validate_repository_settings(&repo).is_err());
        repo.downloads[0].url = "https://example.com/\u{1}".to_string();
        assert!(validate_repository_settings(&repo).is_err());
        // A validation error never carries the url itself: it may
        // hold credentials.
        repo.downloads[0].url = "//user:secret@example.com/path".to_string();
        let err = validate_repository_settings(&repo).unwrap_err();
        assert!(!err.context.contains("secret"), "{}", err.context);
        assert!(!err.context.contains("user:secret"), "{}", err.context);
        repo.downloads[0].url = "https://example.com".to_string();
        repo.downloads[0].repotype = None;
        assert!(validate_repository_settings(&repo).is_err());
        repo.downloads[0].repotype = Some("rpm-md".to_string());
        assert!(validate_repository_settings(&repo).is_err());
        // Every server type is accepted.
        for repotype in ["rpmmd", "susetags", "deb", "arch", "mdk", "registry", "apk"] {
            repo.downloads[0].repotype = Some(repotype.to_string());
            assert!(validate_repository_settings(&repo).is_ok());
        }
        // A master url needs a scheme when present.
        repo.downloads[0].master = Some(DownloadMaster {
            url: "master.example.com".to_string(),
            sslfingerprint: None,
        });
        assert!(validate_repository_settings(&repo).is_err());
        repo.downloads[0].master.as_mut().unwrap().url =
            "//user:secret@example.com/master".to_string();
        let err = validate_repository_settings(&repo).unwrap_err();
        assert!(!err.context.contains("secret"), "{}", err.context);
        repo.downloads[0].master.as_mut().unwrap().url = "https://master.example.com".to_string();
        assert!(validate_repository_settings(&repo).is_ok());
        repo.downloads = vec![];
        // A release target names its repository.
        repo.release_targets = vec![ReleaseTarget {
            project: "openSUSE:Leap:15.6".to_string(),
            repository: None,
            trigger: None,
        }];
        assert!(validate_repository_settings(&repo).is_err());
        repo.release_targets[0].repository = Some("standard".to_string());
        assert!(validate_repository_settings(&repo).is_ok());
        // Empty values the server does not check are accepted.
        repo.block = Some(String::new());
        repo.release_targets[0].trigger = Some(String::new());
        repo.downloads = vec![RepositoryDownload {
            arch: Some("x86_64".to_string()),
            url: "https://example.com".to_string(),
            repotype: Some("rpmmd".to_string()),
            archfilter: Some(String::new()),
            pubkey: Some(String::new()),
            master: Some(DownloadMaster {
                url: "https://master.example.com".to_string(),
                sslfingerprint: Some(String::new()),
            }),
        }];
        assert!(validate_repository_settings(&repo).is_ok());
        repo.downloads = vec![];
        repo.hostsystem = vec![("bad project!".to_string(), "standard".to_string())];
        assert!(validate_repository_settings(&repo).is_err());
        repo.hostsystem = vec![("openSUSE:Factory".to_string(), "snapshot".to_string())];
        assert!(validate_repository_settings(&repo).is_ok());
        assert!(validate_repository(&repo).is_ok());
    }

    #[test]
    fn has_url_scheme_follows_uri_scheme_syntax() {
        assert!(has_url_scheme("https://example.com"));
        assert!(has_url_scheme("svn+ssh://example.com/repo"));
        assert!(has_url_scheme("git+https://example.com/repo.git"));
        assert!(!has_url_scheme("example.com:8080/path"));
        assert!(!has_url_scheme("example.com:8080"));
        assert!(!has_url_scheme("x:"));
        assert!(!has_url_scheme("http:"));
        assert!(!has_url_scheme("1http://example.com"));
        assert!(!has_url_scheme("example.com/no-scheme"));
        assert!(!has_url_scheme("https://example.com/\u{1}"));
        assert!(!has_url_scheme(""));
    }

    #[test]
    fn validate_repository_rejects_duplicate_architectures() {
        let mut repo = new_repo();
        repo.arch.push("x86_64".to_string());
        assert!(validate_repository(&repo).is_err());
        repo.arch.pop();
        repo.arch.push("X86_64".to_string());
        assert!(validate_repository(&repo).is_err());
        repo.arch.pop();
        assert!(validate_repository(&repo).is_ok());
    }

    #[test]
    fn validate_repository_rejects_a_download_arch_that_differs_only_by_case() {
        let mut repo = new_repo();
        repo.downloads.push(RepositoryDownload {
            arch: Some("x86_64".to_string()),
            url: "https://example.com".to_string(),
            repotype: Some("rpmmd".to_string()),
            archfilter: None,
            pubkey: None,
            master: None,
        });
        repo.downloads.push(RepositoryDownload {
            arch: Some("X86_64".to_string()),
            url: "https://other.example.com".to_string(),
            repotype: Some("deb".to_string()),
            archfilter: None,
            pubkey: None,
            master: None,
        });
        let err = validate_repository(&repo).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(
            err.context
                .contains("is not one of the repository's architectures"),
            "{}",
            err.context
        );
    }

    #[test]
    fn add_repository_to_meta_keeps_unknown_elements() {
        let mut doc = XmlDoc::parse(
            "<project name=\"test:factory\"><title>T</title><!-- a note --><lock><disable/></lock><repository name=\"standard\"><arch>x86_64</arch></repository></project>",
        )
        .unwrap();
        add_repository_to_meta(&mut doc, &new_repo(), false).unwrap();
        let rendered = doc.render();
        assert!(rendered.contains("<!-- a note -->"));
        assert!(rendered.contains("<lock><disable/></lock>"));
        assert!(rendered.contains("<repository name=\"standard\">"));
        assert!(rendered.contains("<repository name=\"new-repo\">"));
        // A duplicate is an error and the document is unchanged.
        assert!(add_repository_to_meta(&mut doc, &new_repo(), false).is_err());
        assert_eq!(rendered, doc.render());
    }

    #[test]
    fn publish_disable_round_trip_through_the_meta_edit() {
        // Adding with the flag, then removing the repository, leaves
        // no publish section behind.
        let mut doc =
            XmlDoc::parse("<project name=\"test:factory\"><title>T</title></project>").unwrap();
        add_repository_to_meta(&mut doc, &new_repo(), true).unwrap();
        let rendered = doc.render();
        assert!(rendered.contains("<publish><disable repository=\"new-repo\"/></publish>"));
        assert!(remove_repository_from_meta(&mut doc, "new-repo").unwrap());
        assert_eq!(
            doc.render(),
            "<project name=\"test:factory\"><title>T</title></project>"
        );
        // Removing again reports no change.
        assert!(!remove_repository_from_meta(&mut doc, "new-repo").unwrap());
    }

    #[test]
    fn set_publish_disabled_is_idempotent() {
        let mut doc = XmlDoc::parse(
            "<project name=\"p\"><publish><disable repository=\"x\"/></publish></project>",
        )
        .unwrap();
        set_publish_disabled(&mut doc, "x").unwrap();
        assert_eq!(doc.render().matches("<disable").count(), 1);
        set_publish_disabled(&mut doc, "y").unwrap();
        assert_eq!(doc.render().matches("<disable").count(), 2);
    }

    #[test]
    fn add_repository_wrong_root_meta_is_a_server_error_without_a_put() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/test:factory/_meta",
            "<package name=\"pkg\" project=\"test:factory\"><title>P</title></package>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .add_repository("test:factory", &new_repo(), false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn add_repository_status_error_document_is_a_server_error() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/test:factory/_meta",
            "<status code=\"404\"><summary>unknown project</summary></status>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .add_repository("test:factory", &new_repo(), false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn remove_repository_wrong_root_meta_is_a_server_error_without_a_put() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/test:factory/_meta",
            "<package name=\"pkg\" project=\"test:factory\"><title>P</title></package>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .remove_repository("test:factory", "standard", &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn remove_repository_stale_publish_entry_only_reports_a_change() {
        // No repository named "ghost", but a publish-disable entry for
        // it: removing it still writes, and the emptied publish
        // section goes with it.
        let meta = "<project name=\"test:factory\"><title>Test factory</title><publish><disable repository=\"ghost\"/></publish><repository name=\"standard\"><arch>x86_64</arch></repository></project>";
        let http = FakeHttp::new()
            .with_get("https://api.opensuse.org/source/test:factory/_meta", meta)
            .with_put(
                "https://api.opensuse.org/source/test:factory/_meta",
                "<status code=\"ok\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .remove_repository("test:factory", "ghost", &DryRun::new(false))
            .unwrap());
        let body = &http.put_calls()[0].1;
        assert!(!body.contains("<publish"));
        assert!(body.contains("<repository name=\"standard\">"));
    }

    #[test]
    fn remove_repository_keeps_a_commented_publish_section() {
        let mut doc = XmlDoc::parse(
            "<project name=\"p\"><publish><!-- keep me --><disable repository=\"x\"/></publish><repository name=\"x\"><arch>x86_64</arch></repository></project>",
        )
        .unwrap();
        assert!(remove_repository_from_meta(&mut doc, "x").unwrap());
        let rendered = doc.render();
        assert!(rendered.contains("<!-- keep me -->"));
        assert!(rendered.contains("<publish>"));
        assert!(!rendered.contains("<disable"));
        assert!(!rendered.contains("<repository"));
    }

    #[test]
    fn rebuild_package_posts_the_rebuild_command_and_succeeds() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/build/test:factory?cmd=rebuild&repository=standard&arch=x86_64&package=test-pkgA",
            "<status code=\"ok\"/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .rebuild_package(
                "test:factory",
                "standard",
                "x86_64",
                "test-pkgA",
                &DryRun::new(false)
            )
            .is_ok());
        let posts = http.post_calls();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].1, "");
        assert!(posts[0].2);
    }

    #[test]
    fn restart_build_posts_the_restartbuild_command() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/build/test:factory?cmd=restartbuild&repository=standard&arch=x86_64&package=test-pkgA",
            "<status code=\"ok\"/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .restart_build(
                "test:factory",
                "standard",
                "x86_64",
                "test-pkgA",
                &DryRun::new(false)
            )
            .is_ok());
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn abort_build_posts_the_abortbuild_command() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/build/test:factory?cmd=abortbuild&repository=standard&arch=x86_64&package=test-pkgA",
            "<status code=\"ok\"/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .abort_build(
                "test:factory",
                "standard",
                "x86_64",
                "test-pkgA",
                &DryRun::new(false)
            )
            .is_ok());
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn build_control_non_ok_body_is_error_not_success() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/build/test:factory?cmd=rebuild&repository=standard&arch=x86_64&package=test-pkgA",
            "<status code=\"error\"/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .rebuild_package(
                "test:factory",
                "standard",
                "x86_64",
                "test-pkgA",
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn build_control_rejects_invalid_coordinates_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .rebuild_package(
                "bad/project",
                "standard",
                "x86_64",
                "test-pkgA",
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn build_control_dry_run_records_the_plan_and_posts_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        assert!(client
            .rebuild_package("test:factory", "standard", "x86_64", "test-pkgA", &dry_run)
            .is_ok());
        assert!(http.post_calls().is_empty());
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: rebuild package test:factory/test-pkgA (standard/x86_64)"]
        );
    }

    #[test]
    fn build_control_missing_credentials_is_an_error_without_http() {
        use crate::auth::Credentials;
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, Credentials::None).unwrap();
        let err = client
            .rebuild_package(
                "test:factory",
                "standard",
                "x86_64",
                "test-pkgA",
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.post_calls().is_empty());
    }
}
