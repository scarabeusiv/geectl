use super::build_url;
use super::valid_obs_filename;
use super::valid_obs_name;
use super::ObsClient;
use super::Repository;
use crate::auth::Credentials;
use crate::error::ErrorKind;
use crate::error::GeectlError;
use crate::xml::find_tag_end;
use crate::xml::skip_xml_prolog;
use crate::xml::xml_child_elements;
use crate::xml::xml_elements;
use crate::xml::xml_unescape;

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
    /// as no log (None) rather than the live log.
    pub last: bool,
    /// The last succeeded build's log.
    pub lastsucceeded: bool,
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

/// The buildinfo for a package in a repository/architecture: the
/// error that blocks the build, if any, and the dependencies the
/// build pulls in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInfo {
    /// The error blocking the build (e.g. `unresolvable:
    /// DOES-NOT-EXIST`), if the build cannot start.
    pub error: Option<String>,
    /// The names of the build dependencies, in build order.
    pub bdeps: Vec<String>,
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
        bdeps.push(name);
    }
    Ok(BuildInfo { error, bdeps })
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
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS builddepinfo response: not a <builddepinfo> document",
        )
    };
    let body = body.trim();
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
/// project's repositories. A document that is not a project — or a
/// `<repository>` without a name, or a `<path>` without a project —
/// is a server error, never an empty list.
pub(crate) fn parse_repositories(body: &str) -> Result<Vec<Repository>, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS project meta response: not a <project> document",
        )
    };
    let body = body.trim();
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
        let name = xml_attr(tag, "name").ok_or_else(|| {
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
            let project = xml_attr(path_tag, "project").ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    "unexpected OBS project meta response: <path> without project",
                )
            })?;
            // A path without a repository attribute names no
            // (project, repository) pair, so it is dropped - the
            // same rule project_meta applies to its paths.
            if let Some(repository) = xml_attr(path_tag, "repository") {
                paths.push((project, repository));
            }
        }
        repositories.push(Repository { name, arch, paths });
    }
    Ok(repositories)
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

pub(crate) fn xml_attr(element: &str, name: &str) -> Option<String> {
    // Quote-aware: a `name=` inside an earlier attribute's value
    // (e.g. `details="x dirty=y"`) must not match. The scan tracks
    // whether it is inside a quoted value and only matches outside
    // one. The match position is always an ASCII byte (the first byte
    // of the ASCII needle), so the slicing below stays on UTF-8
    // boundaries.
    let needle = format!("{name}=");
    let bytes = element.as_bytes();
    let first = needle.as_bytes()[0];
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
            && element[i..].starts_with(&needle)
            && (i == 0 || bytes[i - 1].is_ascii_whitespace())
        {
            let after_eq = &element[i + needle.len()..];
            let quote = after_eq.chars().next()?;
            if quote != '"' && quote != '\'' {
                return None;
            }
            let close = after_eq[1..].find(quote)?;
            return Some(xml_unescape(&after_eq[1..1 + close]));
        }
        i += 1;
    }
    None
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
    match xml_attr(tag, name) {
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
        if self.has_basic() {
            return Ok(());
        }
        Err(GeectlError::new(
            ErrorKind::ConfigError,
            "reading build state requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
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
    /// Requires OBS Basic credentials. That is a client policy, not
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
    /// Requires OBS Basic credentials. The log is capped by the
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
            Ok(body) => Ok(Some(body)),
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
    /// package/project does not exist. Requires OBS Basic credentials.
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
    /// or None when no job is running. Requires OBS Basic credentials.
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
    /// OBS Basic credentials. A 404 is an error (unknown
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

    /// The build history for `package` in `repository`/`arch`: the
    /// completed builds, newest first, up to `limit` entries. An empty
    /// history (never built) reads as an empty list. Requires OBS
    /// Basic credentials. A 404 is an error (unknown project/package).
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
    /// history reads as an empty list. Requires OBS Basic credentials.
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
    /// Requires OBS Basic credentials. A multibuild flavor travels as
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
    /// download. Requires OBS Basic credentials. A 404 is an error
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
    /// Requires OBS Basic credentials. A 404 is an error (unknown
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
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
        assert_eq!(parsed.bdeps, vec!["gcc".to_string(), "make".to_string()]);
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
        assert_eq!(info.bdeps, vec!["gcc".to_string()]);
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
        // this used to read as an empty list instead of an error.
        assert!(parse_builddepinfo(
            "<builddepinfo><package name=\"a\"><pkgdep>x</pkgdep></builddepinfo>"
        )
        .is_err());
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
        // present: this used to read as an empty list instead of an
        // error.
        assert!(parse_repositories(
            "<project name=\"p\"><repository name=\"r\"><path project=\"q\" repository=\"s\"/></project>"
        )
        .is_err());
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
}
