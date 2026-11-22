//! `build` commands: the CLI surface for the build-status library
//! batch (`ObsClient` methods in `src/obs/build.rs`).
//!
//! One thin subcommand per library operation, under the `build`
//! noun. Required coordinates are positionals, in the order project,
//! package, repository, arch; optional query fields are flags
//! mirroring the library parameters. Every command returns
//! the `(payload, text)` pair the #52 design requires: the JSON
//! payload and a human rendering of the same data.
//!
//! Two shape notes. `download-binary` returns raw bytes, which cannot
//! ride the JSON payload: it writes them to the `--output` path and
//! the payload names the written file. `check-constraints` takes the
//! constraints document inline (`--constraints`): the one place raw
//! XML crosses the public API, because there is no typed constraints
//! model yet.
//!
//! Coordinate header (design rule 1, decided 2026-11-21, Tom):
//! every command's text starts with one header line echoing the query
//! coordinates — the required positionals, in the order project,
//! package, repository, arch (`project: p package: k repository: r
//! arch: a`), plus `package:` when the query is package-scoped. Tables stay as the design's
//! worked examples pin them; the header is the only addition. The one
//! exception is `build-log`: its text *is* the log content, verbatim
//! (design rule 4), so no header is prepended to the log itself —
//! the coordinates live in the JSON payload, and the synthetic
//! `no log` line does carry the header.
//!
//! Documented omissions (decided, not gaps — see the renderers):
//! job-history text carries 11 of 16 fields (worker and verification
//! detail is payload-only); `fileinfo` resolved sections drop
//! epoch/arch/project/repository per package; a results cell with
//! statuses shows no row for the cell-level code/state (the
//! per-package rows carry the codes, the cell detail rides a note
//! line).

use clap::{Args, Subcommand};
use geectl::auth::{Credentials, EnvObsAuth};
use geectl::config;
use geectl::debug::Debug;
use geectl::error::{ErrorKind, GeectlError};
use geectl::http::{Timeout, UreqClient};
use geectl::obs::{
    BinaryFile, BuildHistoryEntry, BuildInfo, BuildLogQuery, BuildResults, ExtDependency, FileInfo,
    JobHistoryEntry, JobStatus, ObsClient, PackageDeps, ProvidedBy, Repository, ResultsQuery,
    OBS_API_BASE,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

/// The `build` subcommands: one per build-status library operation.
#[derive(Debug, Subcommand)]
pub enum BuildCommand {
    /// Read the build-results matrix for a project.
    Results {
        /// The project to read.
        project: String,
        /// The build-results query filters.
        #[command(flatten)]
        query: ResultsQueryArgs,
        /// Poll for changes since this state hash (from a previous
        /// run). Only `results` takes it: `wait-for-results` polls
        /// plainly, and the library strips a caller-set `oldstate`
        /// there (a held reply would outlast the read timeout), so
        /// the flag is not offered on the wait.
        #[arg(long)]
        oldstate: Option<String>,
    },
    /// Poll the build results until they settle, or the deadline passes.
    WaitForResults {
        /// The project to poll.
        project: String,
        /// The build-results query filters.
        #[command(flatten)]
        query: ResultsQueryArgs,
        /// Deadline in seconds: the wait fails with a timeout past it.
        #[arg(long)]
        deadline_secs: u64,
    },
    /// Read the build log for a package.
    BuildLog {
        /// The project the package is in.
        project: String,
        /// The package to read the log for.
        package: String,
        /// The repository the package builds in.
        repository: String,
        /// The architecture the package builds for.
        arch: String,
        /// Start reading at this byte offset; a negative offset reads
        /// from the end of the log.
        #[arg(long, allow_hyphen_values = true)]
        start: Option<i64>,
        /// The last build's log instead of the current one.
        #[arg(long)]
        last: bool,
        /// The last succeeded build's log.
        #[arg(long)]
        lastsucceeded: bool,
    },
    /// Read the build status code for a package.
    BuildStatus {
        /// The project the package is in.
        project: String,
        /// The package to read the status for.
        package: String,
        /// The repository the package builds in.
        repository: String,
        /// The architecture the package builds for.
        arch: String,
    },
    /// Read the running build job for a package, if any.
    JobStatus {
        /// The project the package is in.
        project: String,
        /// The package to read the job for.
        package: String,
        /// The repository the package builds in.
        repository: String,
        /// The architecture the package builds for.
        arch: String,
    },
    /// Read the buildinfo for a package: the blocking error, if any,
    /// and the dependencies the build pulls in.
    Buildinfo {
        /// The project the package is in.
        project: String,
        /// The package to read the buildinfo for.
        package: String,
        /// The repository the package builds in.
        repository: String,
        /// The architecture the package builds for.
        arch: String,
    },
    /// List the completed builds for a package, newest first.
    BuildHistory {
        /// The project the package is in.
        project: String,
        /// The package to list the history for.
        package: String,
        /// The repository the package builds in.
        repository: String,
        /// The architecture the package builds for.
        arch: String,
        /// At most this many entries.
        #[arg(long)]
        limit: Option<u32>,
    },
    /// List the build jobs that ran in a repository, newest first.
    JobHistory {
        /// The project the repository is in.
        project: String,
        /// The repository the jobs ran in.
        repository: String,
        /// The architecture the jobs ran on.
        arch: String,
        /// Only jobs for this package.
        #[arg(long)]
        package: Option<String>,
        /// At most this many entries.
        #[arg(long)]
        limit: Option<u32>,
        /// Only jobs with this result code (e.g. `failed`).
        #[arg(long)]
        code: Option<String>,
    },
    /// List the built files for a package (or the whole repository).
    ListBinaries {
        /// The project the package is in.
        project: String,
        /// The repository the files were built in.
        repository: String,
        /// The architecture the files were built for.
        arch: String,
        /// Only this package's files; without it, the whole
        /// repository's files.
        #[arg(long)]
        package: Option<String>,
    },
    /// Download one built file by name.
    DownloadBinary {
        /// The project the package is in.
        project: String,
        /// The package the file belongs to.
        package: String,
        /// The repository the file was built in.
        repository: String,
        /// The architecture the file was built for.
        arch: String,
        /// The file to download (e.g. `foo-1.0-1.x86_64.rpm`).
        filename: String,
        /// Where to write the downloaded bytes.
        #[arg(long)]
        output: PathBuf,
    },
    /// Read the metadata of a built file: RPM header fields plus
    /// dependency lists.
    Fileinfo {
        /// The project the package is in.
        project: String,
        /// The package the file belongs to.
        package: String,
        /// The repository the file was built in.
        repository: String,
        /// The architecture the file was built for.
        arch: String,
        /// The file to read the metadata for.
        filename: String,
        /// The `fileinfo_ext` view: each dependency names the
        /// packages providing or requiring it.
        #[arg(long)]
        ext: bool,
    },
    /// List which packages each package depends on for its build.
    Dependson {
        /// The project the repository is in.
        project: String,
        /// The repository to read.
        repository: String,
        /// The architecture to read.
        arch: String,
        /// Only this package's dependencies.
        #[arg(long)]
        package: Option<String>,
    },
    /// List the packages that depend on a package for their build.
    Whatdependson {
        /// The project the repository is in.
        project: String,
        /// The package to find the dependents of.
        package: String,
        /// The repository to read.
        repository: String,
        /// The architecture to read.
        arch: String,
    },
    /// List the worker ids satisfying a `_constraints` document.
    CheckConstraints {
        /// The project the package is in.
        project: String,
        /// The package to check the constraints for.
        package: String,
        /// The repository the package builds in.
        repository: String,
        /// The architecture the package builds for.
        arch: String,
        /// The constraints document, as authored. Without it, an
        /// empty body is posted, which the server accepts.
        #[arg(long)]
        constraints: Option<String>,
    },
    /// List the repositories of a project.
    ListRepositories {
        /// The project to read.
        project: String,
    },
}

/// The build-results query filters, shared by `results` and
/// `wait-for-results`. Every field is an optional filter, mirroring
/// [`ResultsQuery`].
#[derive(Debug, Clone, Args)]
pub struct ResultsQueryArgs {
    /// Only this package's statuses (the repository/architecture
    /// cells still come back whole).
    #[arg(long)]
    package: Option<String>,
    /// Only these repositories (repeatable).
    #[arg(long)]
    repository: Vec<String>,
    /// Only these architectures (repeatable).
    #[arg(long)]
    arch: Vec<String>,
    /// Only packages with this build code (e.g. `failed`).
    #[arg(long)]
    code: Option<String>,
    /// Expand multibuild flavors into their own statuses.
    #[arg(long)]
    multibuild: bool,
    /// Follow local links when expanding multibuild flavors.
    #[arg(long)]
    locallink: bool,
    /// The last build's results instead of the current state.
    #[arg(long)]
    lastbuild: bool,
}

impl ResultsQueryArgs {
    /// Build the library query. `oldstate` travels separately: only
    /// `results` accepts it (see the `Results` variant).
    fn into_query(self, oldstate: Option<String>) -> ResultsQuery {
        ResultsQuery {
            package: self.package,
            repositories: self.repository,
            arches: self.arch,
            code: self.code,
            multibuild: self.multibuild,
            locallink: self.locallink,
            lastbuild: self.lastbuild,
            oldstate,
        }
    }
}

/// Run one `build` subcommand: build the OBS client from the
/// effective config and credentials, call the library operation, and
/// return the `(payload, text)` pair. Every build read except
/// `list-repositories` requires OBS Basic credentials and fails
/// loudly without them (the library's `require_build_credentials`).
/// Credentials resolve per command: `list-repositories` is a public
/// read and never touches them (a half-set credential env must not
/// break it); every other read fails fast on a half-set pair up
/// front. That differs from `request` deliberately, which swallows
/// a half-configured pair in dry-run so a preview never needs
/// credentials at all.
pub fn run_build(debug: &Debug, command: BuildCommand) -> Result<(Value, String), GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    // Credentials resolve lazily for `list-repositories`: the read is
    // public and never touches them, so a half-set credential env
    // (username without password) must not break it.
    let (base, instances) = obs_endpoint()?;
    let credentials = match &command {
        BuildCommand::ListRepositories { .. } => Credentials::None,
        _ => EnvObsAuth::credentials_for_url(&base, &base, &instances)?,
    };
    let client =
        ObsClient::with_base_and_credentials(&http, &base, credentials)?.with_debug(debug.clone());
    // The coordinate header (design rule 1, decided 2026-11-21) is
    // computed from the command before the match moves it. Every arm
    // prepends it — except the `BuildLog` content itself, whose text
    // is the log verbatim (rule 4); its synthetic `no log` line
    // still carries the header.
    let header = coordinates_header(&command);
    match command {
        BuildCommand::Results {
            project,
            query,
            oldstate,
        } => {
            let query = query.into_query(oldstate);
            let results = client.results(&project, &query)?;
            Ok((
                results_payload(&results),
                with_header(&header, render_results_text(&results)),
            ))
        }
        BuildCommand::WaitForResults {
            project,
            query,
            deadline_secs,
        } => {
            let query = query.into_query(None);
            let results =
                client.wait_for_results(&project, &query, Duration::from_secs(deadline_secs))?;
            Ok((
                results_payload(&results),
                with_header(&header, render_results_text(&results)),
            ))
        }
        BuildCommand::BuildLog {
            project,
            package,
            repository,
            arch,
            start,
            last,
            lastsucceeded,
        } => {
            let query = BuildLogQuery {
                start,
                last,
                lastsucceeded,
            };
            let log = client.build_log(&project, &package, &repository, &arch, &query)?;
            // No coordinate header on the log itself: the text *is*
            // the log content, verbatim (design rule 4) — prepending
            // a header would corrupt piping the log to a file. The
            // synthetic `no log` line is not content, so it carries
            // the header; the coordinates live in the JSON payload.
            let text = match &log {
                Some(log) => log.clone(),
                None => with_header(&header, "no log".to_string()),
            };
            let payload = json!({
                "project": project,
                "package": package,
                "repository": repository,
                "arch": arch,
                "log": log,
            });
            Ok((payload, text))
        }
        BuildCommand::BuildStatus {
            project,
            package,
            repository,
            arch,
        } => {
            let code = client.build_status(&project, &package, &repository, &arch)?;
            let text = match &code {
                Some(code) => format!("code: {code}"),
                None => "no status".to_string(),
            };
            let payload = json!({
                "project": project,
                "package": package,
                "repository": repository,
                "arch": arch,
                "code": code,
            });
            Ok((payload, with_header(&header, text)))
        }
        BuildCommand::JobStatus {
            project,
            package,
            repository,
            arch,
        } => {
            let job = client.job_status(&project, &package, &repository, &arch)?;
            Ok((
                job_status_payload(&project, &package, &repository, &arch, &job),
                with_header(&header, render_job_status_text(&job)),
            ))
        }
        BuildCommand::Buildinfo {
            project,
            package,
            repository,
            arch,
        } => {
            let info = client.buildinfo(&project, &package, &repository, &arch)?;
            Ok((
                buildinfo_payload(&project, &package, &repository, &arch, &info),
                with_header(&header, render_buildinfo_text(&info)),
            ))
        }
        BuildCommand::BuildHistory {
            project,
            package,
            repository,
            arch,
            limit,
        } => {
            let entries = client.build_history(&project, &package, &repository, &arch, limit)?;
            Ok((
                history_payload(&project, &package, &repository, &arch, &entries),
                with_header(&header, render_build_history_text(&entries)),
            ))
        }
        BuildCommand::JobHistory {
            project,
            repository,
            arch,
            package,
            limit,
            code,
        } => {
            let entries = client.job_history(
                &project,
                &repository,
                &arch,
                package.as_deref(),
                limit,
                code.as_deref(),
            )?;
            Ok((
                job_history_payload(&project, &repository, &arch, &entries),
                with_header(&header, render_job_history_text(&entries)),
            ))
        }
        BuildCommand::ListBinaries {
            project,
            repository,
            arch,
            package,
        } => {
            let files = client.list_binaries(&project, package.as_deref(), &repository, &arch)?;
            Ok((
                binaries_payload(&project, &repository, &arch, &files),
                with_header(&header, render_binaries_text(&files)),
            ))
        }
        BuildCommand::DownloadBinary {
            project,
            package,
            repository,
            arch,
            filename,
            output,
        } => {
            let bytes =
                client.download_binary(&project, &package, &repository, &arch, &filename)?;
            write_downloaded(&output, &bytes)?;
            let size = bytes.len();
            let payload = json!({
                "project": project,
                "package": package,
                "repository": repository,
                "arch": arch,
                "filename": filename,
                "path": output.to_string_lossy(),
                "size_bytes": size,
            });
            let text = format!(
                "downloaded {filename} ({size} bytes) to {}",
                output.display()
            );
            Ok((payload, with_header(&header, text)))
        }
        BuildCommand::Fileinfo {
            project,
            package,
            repository,
            arch,
            filename,
            ext,
        } => {
            let info = client.fileinfo(&project, &package, &repository, &arch, &filename, ext)?;
            Ok((
                fileinfo_payload(&project, &package, &repository, &arch, &info),
                with_header(&header, render_fileinfo_text(&info)),
            ))
        }
        BuildCommand::Dependson {
            project,
            repository,
            arch,
            package,
        } => {
            let packages = client.dependson(&project, &repository, &arch, package.as_deref())?;
            Ok((
                deps_payload(&project, &repository, &arch, &packages),
                with_header(&header, render_deps_text(&packages, false)),
            ))
        }
        BuildCommand::Whatdependson {
            project,
            repository,
            arch,
            package,
        } => {
            let packages = client.whatdependson(&project, &repository, &arch, &package)?;
            Ok((
                deps_payload(&project, &repository, &arch, &packages),
                with_header(&header, render_deps_text(&packages, true)),
            ))
        }
        BuildCommand::CheckConstraints {
            project,
            package,
            repository,
            arch,
            constraints,
        } => {
            let workers = client.check_constraints(
                &project,
                &package,
                &repository,
                &arch,
                constraints.as_deref(),
            )?;
            let payload = json!({
                "project": project,
                "package": package,
                "repository": repository,
                "arch": arch,
                "workers": workers,
            });
            let text = render_workers_text(&workers);
            Ok((payload, with_header(&header, text)))
        }
        BuildCommand::ListRepositories { project } => {
            let repositories = client.list_repositories(&project)?;
            Ok((
                repositories_payload(&project, &repositories),
                with_header(&header, render_repositories_text(&repositories)),
            ))
        }
    }
}

/// One header line echoing the query coordinates (design rule 1 as
/// decided 2026-11-21, Tom): the required positionals, in the order
/// project, package, repository, arch — `project: p package: k
/// repository: r arch: a` — plus `package:` when the query is
/// package-scoped (results, wait-for-results, job-history,
/// list-binaries, dependson). The filename is never part of the
/// header: it is already the first body line of the `fileinfo` text
/// and of the `download-binary` confirmation (the header line sits
/// above it).
fn coordinates_header(command: &BuildCommand) -> String {
    let mut coords: Vec<String> = Vec::new();
    match command {
        BuildCommand::Results { project, query, .. }
        | BuildCommand::WaitForResults { project, query, .. } => {
            coords.push(format!("project: {project}"));
            if let Some(package) = &query.package {
                coords.push(format!("package: {package}"));
            }
        }
        BuildCommand::BuildLog {
            project,
            package,
            repository,
            arch,
            ..
        }
        | BuildCommand::BuildStatus {
            project,
            package,
            repository,
            arch,
        }
        | BuildCommand::JobStatus {
            project,
            package,
            repository,
            arch,
        }
        | BuildCommand::Buildinfo {
            project,
            package,
            repository,
            arch,
        }
        | BuildCommand::BuildHistory {
            project,
            package,
            repository,
            arch,
            ..
        }
        | BuildCommand::Fileinfo {
            project,
            package,
            repository,
            arch,
            ..
        }
        | BuildCommand::DownloadBinary {
            project,
            package,
            repository,
            arch,
            ..
        }
        | BuildCommand::CheckConstraints {
            project,
            package,
            repository,
            arch,
            ..
        } => {
            coords.push(format!("project: {project}"));
            coords.push(format!("package: {package}"));
            coords.push(format!("repository: {repository}"));
            coords.push(format!("arch: {arch}"));
        }
        BuildCommand::JobHistory {
            project,
            repository,
            arch,
            package,
            ..
        }
        | BuildCommand::ListBinaries {
            project,
            repository,
            arch,
            package,
        }
        | BuildCommand::Dependson {
            project,
            repository,
            arch,
            package,
        } => {
            coords.push(format!("project: {project}"));
            if let Some(package) = package {
                coords.push(format!("package: {package}"));
            }
            coords.push(format!("repository: {repository}"));
            coords.push(format!("arch: {arch}"));
        }
        BuildCommand::Whatdependson {
            project,
            package,
            repository,
            arch,
        } => {
            coords.push(format!("project: {project}"));
            coords.push(format!("package: {package}"));
            coords.push(format!("repository: {repository}"));
            coords.push(format!("arch: {arch}"));
        }
        BuildCommand::ListRepositories { project } => {
            coords.push(format!("project: {project}"));
        }
    }
    coords.join(" ")
}

/// Prepend the coordinate header line to a renderer's text.
fn with_header(header: &str, text: String) -> String {
    format!("{header}\n{text}")
}

/// Write downloaded bytes to the `--output` path. A write failure is
/// an internal error naming the path (the download itself already
/// succeeded, so the failure is on our side, not the server's).
fn write_downloaded(output: &PathBuf, bytes: &[u8]) -> Result<(), GeectlError> {
    std::fs::write(output, bytes).map_err(|e| {
        GeectlError::new(
            ErrorKind::InternalError,
            format!("cannot write {}: {e}", output.display()),
        )
    })
}

/// The effective OBS base URL: env > config file > default, resolved
/// once here — callers must not re-derive it by re-reading the
/// environment.
/// The configured OBS base URL and the config file's `[instances]`
/// table, for credential resolution (see
/// [`auth::resolve_obs_basic_for_url`]).
fn obs_endpoint() -> Result<(String, Vec<(String, String)>), GeectlError> {
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let effective = config::effective_config(&file_config)?;
    Ok((
        effective
            .config
            .obs_url
            .unwrap_or_else(|| OBS_API_BASE.to_string()),
        file_config.instances,
    ))
}

/// A left-aligned table: columns padded to their widest cell, joined
/// with two spaces. The first row is the header; no separator line.
fn table(rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut widths = vec![0; cols];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let mut out = String::new();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i > 0 {
                out.push_str("  ");
            }
            if i + 1 == row.len() {
                out.push_str(cell);
            } else {
                out.push_str(cell);
                out.push_str(&" ".repeat(widths[i] - cell.chars().count()));
            }
        }
        out.push('\n');
    }
    out.pop();
    out
}

/// One `label: value` line for a present option, nothing when absent.
fn opt_line(label: &str, value: &Option<String>) -> Option<String> {
    value.as_ref().map(|v| format!("{label}: {v}"))
}

/// One `label: value` line for a present numeric option.
fn opt_num_line(label: &str, value: &Option<u64>) -> Option<String> {
    value.map(|v| format!("{label}: {v}"))
}

/// The results matrix as JSON: the state hash plus one object per
/// repository/architecture cell, each with its per-package codes.
fn results_payload(results: &BuildResults) -> Value {
    json!({
        "state": results.state,
        "results": results.results.iter().map(|cell| json!({
            "project": cell.project,
            "repository": cell.repository,
            "arch": cell.arch,
            "code": cell.code,
            "state": cell.state,
            "dirty": cell.dirty,
            "details": cell.details,
            "statuses": cell.statuses.iter().map(|status| json!({
                "package": status.package,
                "code": status.code,
                "details": status.details,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

/// The results matrix as text: one row per package code (cells with
/// no packages get one row for the cell itself), then a summary line
/// counting package codes, then one note line per cell carrying a
/// server detail text, then the state hash for polling.
///
/// Decided omission (2026-11-21, Tom): a cell with statuses shows no
/// row for the cell-level code/state — the per-package rows carry the
/// codes, and the cell detail rides a note line. The cell-level
/// code/state stay in the JSON payload.
fn render_results_text(results: &BuildResults) -> String {
    if results.results.is_empty() {
        return "no results".to_string();
    }
    let mut rows = vec![vec![
        "REPOSITORY".to_string(),
        "ARCH".to_string(),
        "PACKAGE".to_string(),
        "CODE".to_string(),
    ]];
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    // Cell-level detail text (e.g. why a repository is `broken`) has
    // no column in the design's four-column table. Cells without
    // statuses render it in the code cell; cells with statuses carry
    // it as a note line after the summary, so it never vanishes.
    let mut cell_notes: Vec<String> = Vec::new();
    for cell in &results.results {
        // `dirty` is a state fact (rule 1): a dirty cell is marked for
        // re-scheduling, so its rows may already be stale. It rides on
        // the code cell — the table keeps the design's four columns.
        let dirty = if cell.dirty { " (dirty)" } else { "" };
        if cell.statuses.is_empty() {
            rows.push(vec![
                cell.repository.clone(),
                cell.arch.clone(),
                "-".to_string(),
                format!("{}{dirty}", code_with_details(&cell.code, &cell.details)),
            ]);
            *counts.entry(cell.code.clone()).or_insert(0) += 1;
        } else {
            if let Some(details) = &cell.details {
                cell_notes.push(format!("{} {}: {details}", cell.repository, cell.arch));
            }
            for status in &cell.statuses {
                rows.push(vec![
                    cell.repository.clone(),
                    cell.arch.clone(),
                    status.package.clone(),
                    format!(
                        "{}{dirty}",
                        code_with_details(&status.code, &status.details)
                    ),
                ]);
                *counts.entry(status.code.clone()).or_insert(0) += 1;
            }
        }
    }
    let mut text = table(&rows);
    let summary = counts
        .iter()
        .map(|(code, n)| format!("{n} {code}"))
        .collect::<Vec<_>>()
        .join(", ");
    text.push_str(&format!("\n{summary}"));
    for note in cell_notes {
        text.push_str(&format!("\n{note}"));
    }
    if let Some(state) = &results.state {
        text.push_str(&format!("\nstate: {state}"));
    }
    text
}

/// A code with its server detail text in parentheses, when present.
fn code_with_details(code: &str, details: &Option<String>) -> String {
    match details {
        Some(details) => format!("{code} ({details})"),
        None => code.to_string(),
    }
}

/// A running job (or the absence of one) as JSON.
fn job_status_payload(
    project: &str,
    package: &str,
    repository: &str,
    arch: &str,
    job: &Option<JobStatus>,
) -> Value {
    json!({
        "project": project,
        "package": package,
        "repository": repository,
        "arch": arch,
        "job": job.as_ref().map(|job| json!({
            "code": job.code,
            "result": job.result,
            "details": job.details,
            "starttime": job.starttime,
            "endtime": job.endtime,
            "lastduration": job.lastduration,
            "workerid": job.workerid,
            "hostarch": job.hostarch,
            "uri": job.uri,
            "arch": job.arch,
            "job": job.job,
            "jobid": job.jobid,
            "attempt": job.attempt,
        })),
    })
}

/// A running job as text: one `key: value` line per known field.
fn render_job_status_text(job: &Option<JobStatus>) -> String {
    let Some(job) = job else {
        return "no job running".to_string();
    };
    let mut lines = vec![format!("code: {}", job.code)];
    lines.extend(opt_line("result", &job.result));
    lines.extend(opt_line("details", &job.details));
    lines.extend(opt_num_line("starttime", &job.starttime));
    lines.extend(opt_num_line("endtime", &job.endtime));
    lines.extend(opt_num_line("lastduration", &job.lastduration));
    lines.extend(opt_line("workerid", &job.workerid));
    lines.extend(opt_line("hostarch", &job.hostarch));
    lines.extend(opt_line("uri", &job.uri));
    lines.extend(opt_line("arch", &job.arch));
    lines.extend(opt_line("job", &job.job));
    lines.extend(opt_line("jobid", &job.jobid));
    lines.extend(opt_num_line("attempt", &job.attempt));
    lines.join("\n")
}

/// Buildinfo as JSON: the blocking error plus the dependency names.
fn buildinfo_payload(
    project: &str,
    package: &str,
    repository: &str,
    arch: &str,
    info: &BuildInfo,
) -> Value {
    json!({
        "project": project,
        "package": package,
        "repository": repository,
        "arch": arch,
        "error": info.error,
        "bdeps": info.bdeps.iter().map(|dep| dep.name.as_str()).collect::<Vec<_>>(),
    })
}

/// Buildinfo as text: the blocking error, then the dependencies.
fn render_buildinfo_text(info: &BuildInfo) -> String {
    let mut lines = Vec::new();
    match &info.error {
        Some(error) => lines.push(format!("error: {error}")),
        None => lines.push("no blocking error".to_string()),
    }
    if info.bdeps.is_empty() {
        lines.push("no dependencies".to_string());
    } else {
        lines.push("dependencies:".to_string());
        lines.extend(info.bdeps.iter().map(|dep| dep.name.clone()));
    }
    lines.join("\n")
}

/// A build history as JSON: one object per completed build.
fn history_payload(
    project: &str,
    package: &str,
    repository: &str,
    arch: &str,
    entries: &[BuildHistoryEntry],
) -> Value {
    json!({
        "project": project,
        "package": package,
        "repository": repository,
        "arch": arch,
        "entries": entries.iter().map(|entry| json!({
            "rev": entry.rev,
            "srcmd5": entry.srcmd5,
            "versrel": entry.versrel,
            "bcnt": entry.bcnt,
            "time": entry.time,
            "duration": entry.duration,
        })).collect::<Vec<_>>(),
    })
}

/// A build history as text: one row per completed build.
fn render_build_history_text(entries: &[BuildHistoryEntry]) -> String {
    if entries.is_empty() {
        return "no builds".to_string();
    }
    let mut rows = vec![vec![
        "REV".to_string(),
        "SRCMD5".to_string(),
        "VERSREL".to_string(),
        "BCNT".to_string(),
        "TIME".to_string(),
        "DURATION".to_string(),
    ]];
    for entry in entries {
        rows.push(vec![
            entry.rev.clone(),
            entry.srcmd5.clone(),
            entry.versrel.clone(),
            entry.bcnt.to_string(),
            entry.time.to_string(),
            entry
                .duration
                .map(|d| d.to_string())
                .unwrap_or_else(|| "-".to_string()),
        ]);
    }
    table(&rows)
}

/// A job history as JSON: one object per job that ran.
fn job_history_payload(
    project: &str,
    repository: &str,
    arch: &str,
    entries: &[JobHistoryEntry],
) -> Value {
    json!({
        "project": project,
        "repository": repository,
        "arch": arch,
        "entries": entries.iter().map(|entry| json!({
            "repository": entry.repository,
            "arch": entry.arch,
            "package": entry.package,
            "rev": entry.rev,
            "srcmd5": entry.srcmd5,
            "versrel": entry.versrel,
            "bcnt": entry.bcnt,
            "readytime": entry.readytime,
            "starttime": entry.starttime,
            "endtime": entry.endtime,
            "code": entry.code,
            "uri": entry.uri,
            "workerid": entry.workerid,
            "hostarch": entry.hostarch,
            "reason": entry.reason,
            "verifymd5": entry.verifymd5,
        })).collect::<Vec<_>>(),
    })
}

/// A job history as text: one row per job that ran. The table carries
/// the full revision identity (versrel, rev, srcmd5, bcnt) per design
/// rule 1; the worker and verification detail (starttime, uri,
/// workerid, hostarch, verifymd5) is payload-only — a documented
/// omission decided 2026-11-21 (Tom), not a gap.
fn render_job_history_text(entries: &[JobHistoryEntry]) -> String {
    if entries.is_empty() {
        return "no jobs".to_string();
    }
    let mut rows = vec![vec![
        "PACKAGE".to_string(),
        "CODE".to_string(),
        "REPOSITORY".to_string(),
        "ARCH".to_string(),
        "VERSREL".to_string(),
        "REV".to_string(),
        "SRCMD5".to_string(),
        "BCNT".to_string(),
        "READYTIME".to_string(),
        "ENDTIME".to_string(),
        "REASON".to_string(),
    ]];
    for entry in entries {
        rows.push(vec![
            entry.package.clone(),
            entry.code.clone(),
            entry.repository.clone().unwrap_or_else(|| "-".to_string()),
            entry.arch.clone().unwrap_or_else(|| "-".to_string()),
            entry.versrel.clone().unwrap_or_else(|| "-".to_string()),
            entry.rev.clone().unwrap_or_else(|| "-".to_string()),
            entry.srcmd5.clone().unwrap_or_else(|| "-".to_string()),
            entry
                .bcnt
                .map(|n| n.to_string())
                .unwrap_or_else(|| "-".to_string()),
            entry
                .readytime
                .map(|t| t.to_string())
                .unwrap_or_else(|| "-".to_string()),
            entry
                .endtime
                .map(|t| t.to_string())
                .unwrap_or_else(|| "-".to_string()),
            entry.reason.clone().unwrap_or_else(|| "-".to_string()),
        ]);
    }
    table(&rows)
}

/// Built files as JSON: one object per file.
fn binaries_payload(project: &str, repository: &str, arch: &str, files: &[BinaryFile]) -> Value {
    json!({
        "project": project,
        "repository": repository,
        "arch": arch,
        "files": files.iter().map(|file| json!({
            "name": file.name,
            "size": file.size,
            "mtime": file.mtime,
        })).collect::<Vec<_>>(),
    })
}

/// Worker ids satisfying a constraints document as text: a `WORKER`
/// table, one row per worker id (rule 3: lists render as tables).
fn render_workers_text(workers: &[String]) -> String {
    if workers.is_empty() {
        return "no workers".to_string();
    }
    let mut rows = vec![vec!["WORKER".to_string()]];
    rows.extend(workers.iter().map(|worker| vec![worker.clone()]));
    table(&rows)
}

/// Built files as text: the design's pinned `list_files` shape — a
/// `NAME SIZE MTIME` table, `-` for an unknown size or mtime.
fn render_binaries_text(files: &[BinaryFile]) -> String {
    if files.is_empty() {
        return "no binaries".to_string();
    }
    let mut rows = vec![vec![
        "NAME".to_string(),
        "SIZE".to_string(),
        "MTIME".to_string(),
    ]];
    for file in files {
        rows.push(vec![
            file.name.clone(),
            file.size
                .map(|size| size.to_string())
                .unwrap_or_else(|| "-".to_string()),
            file.mtime
                .map(|mtime| mtime.to_string())
                .unwrap_or_else(|| "-".to_string()),
        ]);
    }
    table(&rows)
}

/// One `fileinfo_ext` dependency with its resolution as JSON.
fn ext_dependency_json(dep: &ExtDependency) -> Value {
    json!({
        "dep": dep.dep,
        "provided_by": dep.provided_by.iter().map(provided_by_json).collect::<Vec<_>>(),
        "required_by": dep.required_by.iter().map(provided_by_json).collect::<Vec<_>>(),
    })
}

/// One package named by a `fileinfo_ext` resolution as JSON.
fn provided_by_json(pkg: &ProvidedBy) -> Value {
    json!({
        "name": pkg.name,
        "epoch": pkg.epoch,
        "version": pkg.version,
        "release": pkg.release,
        "arch": pkg.arch,
        "project": pkg.project,
        "repository": pkg.repository,
    })
}

/// File metadata as JSON: the RPM header fields plus the dependency
/// lists, plain and resolved.
fn fileinfo_payload(
    project: &str,
    package: &str,
    repository: &str,
    arch: &str,
    info: &FileInfo,
) -> Value {
    json!({
        "project": project,
        "package": package,
        "repository": repository,
        "arch": arch,
        "filename": info.filename,
        "name": info.name,
        "epoch": info.epoch,
        "version": info.version,
        "release": info.release,
        "arch": info.arch,
        "source": info.source,
        "summary": info.summary,
        "description": info.description,
        "disturl": info.disturl,
        "size": info.size,
        "mtime": info.mtime,
        "provides": info.provides,
        "requires": info.requires,
        "prerequires": info.prerequires,
        "conflicts": info.conflicts,
        "obsoletes": info.obsoletes,
        "recommends": info.recommends,
        "supplements": info.supplements,
        "suggests": info.suggests,
        "enhances": info.enhances,
        "provides_ext": info.provides_ext.iter().map(ext_dependency_json).collect::<Vec<_>>(),
        "requires_ext": info.requires_ext.iter().map(ext_dependency_json).collect::<Vec<_>>(),
        "recommends_ext": info.recommends_ext.iter().map(ext_dependency_json).collect::<Vec<_>>(),
        "supplements_ext": info.supplements_ext.iter().map(ext_dependency_json).collect::<Vec<_>>(),
    })
}

/// File metadata as text: header fields, then one section per
/// non-empty dependency list; resolved lists name the providing or
/// requiring packages.
fn render_fileinfo_text(info: &FileInfo) -> String {
    let mut lines = vec![format!("file: {}", info.filename)];
    lines.extend(opt_line("name", &info.name));
    lines.extend(opt_num_line("epoch", &info.epoch));
    lines.extend(opt_line("version", &info.version));
    lines.extend(opt_line("release", &info.release));
    lines.extend(opt_line("arch", &info.arch));
    lines.extend(opt_line("source", &info.source));
    lines.extend(opt_line("summary", &info.summary));
    lines.extend(opt_line("description", &info.description));
    lines.extend(opt_line("disturl", &info.disturl));
    lines.extend(opt_num_line("size", &info.size));
    lines.extend(opt_num_line("mtime", &info.mtime));
    render_dep_section(&mut lines, "provides", &info.provides);
    render_dep_section(&mut lines, "requires", &info.requires);
    render_dep_section(&mut lines, "prerequires", &info.prerequires);
    render_dep_section(&mut lines, "conflicts", &info.conflicts);
    render_dep_section(&mut lines, "obsoletes", &info.obsoletes);
    render_dep_section(&mut lines, "recommends", &info.recommends);
    render_dep_section(&mut lines, "supplements", &info.supplements);
    render_dep_section(&mut lines, "suggests", &info.suggests);
    render_dep_section(&mut lines, "enhances", &info.enhances);
    render_ext_section(&mut lines, "provides (resolved)", &info.provides_ext);
    render_ext_section(&mut lines, "requires (resolved)", &info.requires_ext);
    render_ext_section(&mut lines, "recommends (resolved)", &info.recommends_ext);
    render_ext_section(&mut lines, "supplements (resolved)", &info.supplements_ext);
    lines.join("\n")
}

/// One `label:` section plus one line per dependency, when the list
/// is non-empty.
fn render_dep_section(lines: &mut Vec<String>, label: &str, deps: &[String]) {
    if !deps.is_empty() {
        lines.push(format!("{label}:"));
        lines.extend(deps.iter().cloned());
    }
}

/// One resolved-dependency section: each dependency names the
/// packages on either side of it, with their version-release when
/// known (a revision fact per design rule 1). Both sides render when
/// present: the library's own parse fixture shows `supplements_ext`
/// resolved via `requiredby`, while its struct doc says `provided_by`
/// — the text does not depend on which side the server populates.
/// Epoch, arch, project, and repository are payload-only — a
/// documented omission decided 2026-11-21 (Tom), not a gap.
fn render_ext_section(lines: &mut Vec<String>, label: &str, deps: &[ExtDependency]) {
    if deps.is_empty() {
        return;
    }
    lines.push(format!("{label}:"));
    for dep in deps {
        let mut parts = Vec::new();
        if !dep.provided_by.is_empty() {
            parts.push(format!(
                "provided by: {}",
                dep.provided_by
                    .iter()
                    .map(provided_by_text)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !dep.required_by.is_empty() {
            parts.push(format!(
                "required by: {}",
                dep.required_by
                    .iter()
                    .map(provided_by_text)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if parts.is_empty() {
            lines.push(dep.dep.clone());
        } else {
            lines.push(format!("{} ({})", dep.dep, parts.join("; ")));
        }
    }
}

/// A resolved package as text: name plus version-release when known.
fn provided_by_text(pkg: &ProvidedBy) -> String {
    match (&pkg.version, &pkg.release) {
        (Some(version), Some(release)) => format!("{} {version}-{release}", pkg.name),
        (Some(version), None) => format!("{} {version}", pkg.name),
        (None, _) => pkg.name.clone(),
    }
}

/// Per-package build dependencies as JSON.
fn deps_payload(project: &str, repository: &str, arch: &str, packages: &[PackageDeps]) -> Value {
    json!({
        "project": project,
        "repository": repository,
        "arch": arch,
        "packages": packages.iter().map(|pkg| json!({
            "name": pkg.name,
            "source": pkg.source,
            "dependencies": pkg.dependencies,
            "subpackages": pkg.subpackages,
        })).collect::<Vec<_>>(),
    })
}

/// Per-package build dependencies as text. In the reverse view
/// (`whatdependson`) the dependency list names the packages depending
/// on the queried package, not its own dependencies.
fn render_deps_text(packages: &[PackageDeps], reverse: bool) -> String {
    if packages.is_empty() {
        return if reverse {
            "no dependents".to_string()
        } else {
            "no dependencies".to_string()
        };
    }
    let mut lines = Vec::new();
    for pkg in packages {
        lines.push(pkg.name.clone());
        if let Some(source) = &pkg.source {
            lines.push(format!("  source: {source}"));
        }
        if !pkg.dependencies.is_empty() {
            let label = if reverse {
                "depended on by"
            } else {
                "depends on"
            };
            lines.push(format!("  {label}: {}", pkg.dependencies.join(", ")));
        }
        if !pkg.subpackages.is_empty() {
            lines.push(format!("  subpackages: {}", pkg.subpackages.join(", ")));
        }
    }
    lines.join("\n")
}

/// Project repositories as JSON.
fn repositories_payload(project: &str, repositories: &[Repository]) -> Value {
    json!({
        "project": project,
        "repositories": repositories.iter().map(|repo| json!({
            "name": repo.name,
            "arch": repo.arch,
            "paths": repo.paths.iter().map(|(project, repository)| json!([project, repository])).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

/// Project repositories as text: one line per repository with its
/// architectures, plus its rebuild-source paths.
fn render_repositories_text(repositories: &[Repository]) -> String {
    if repositories.is_empty() {
        return "no repositories".to_string();
    }
    let mut lines = Vec::new();
    for repo in repositories {
        lines.push(format!("{} ({})", repo.name, repo.arch.join(", ")));
        for (project, repository) in &repo.paths {
            lines.push(format!("  path: {project}/{repository}"));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::FromArgMatches;
    use geectl::obs::{BuildDep, BuildResult, PackageBuildStatus};

    /// The clap `Command` for the `build` group, built from the
    /// `Subcommand` derive: there is no `CommandFactory` for a
    /// subcommand enum, `augment_subcommands` is the constructor.
    fn build_command() -> clap::Command {
        BuildCommand::augment_subcommands(clap::Command::new("build"))
    }

    fn results_fixture() -> BuildResults {
        BuildResults {
            state: Some("abc123".to_string()),
            results: vec![BuildResult {
                project: "openSUSE:Factory".to_string(),
                repository: "standard".to_string(),
                arch: "x86_64".to_string(),
                code: "published".to_string(),
                state: "published".to_string(),
                dirty: false,
                details: None,
                statuses: vec![
                    PackageBuildStatus {
                        package: "foo".to_string(),
                        code: "succeeded".to_string(),
                        details: None,
                    },
                    PackageBuildStatus {
                        package: "bar".to_string(),
                        code: "failed".to_string(),
                        details: Some("missing foo".to_string()),
                    },
                ],
            }],
        }
    }

    #[test]
    fn table_pads_columns() {
        let text = table(&[
            vec!["A".to_string(), "B".to_string()],
            vec!["xx".to_string(), "y".to_string()],
        ]);
        assert_eq!(text, "A   B\nxx  y");
    }

    #[test]
    fn table_measures_widths_in_characters() {
        // Widths are characters, not bytes: a two-byte `é` is one
        // column wide, so the second column still aligns.
        let text = table(&[
            vec!["é".to_string(), "B".to_string()],
            vec!["xx".to_string(), "y".to_string()],
        ]);
        assert_eq!(text, "é   B\nxx  y");
    }

    #[test]
    fn header_prepends_one_line() {
        assert_eq!(
            with_header("project: p", "no results".to_string()),
            "project: p\nno results"
        );
    }

    #[test]
    fn coordinate_header_echoes_positionals_in_order() {
        // Design rule 1, decided 2026-11-21 (Tom): one header line,
        // the required positionals in the order project, package,
        // repository, arch, plus `package:` when the query is
        // package-scoped.
        assert_eq!(
            coordinates_header(&parse_build(&["build-status", "p", "k", "r", "a"])),
            "project: p package: k repository: r arch: a"
        );
        assert_eq!(
            coordinates_header(&parse_build(&["list-repositories", "p"])),
            "project: p"
        );
        assert_eq!(
            coordinates_header(&parse_build(&["results", "p"])),
            "project: p"
        );
        assert_eq!(
            coordinates_header(&parse_build(&["results", "p", "--package", "foo"])),
            "project: p package: foo"
        );
        assert_eq!(
            coordinates_header(&parse_build(&["job-history", "p", "r", "a"])),
            "project: p repository: r arch: a"
        );
        assert_eq!(
            coordinates_header(&parse_build(&[
                "job-history",
                "p",
                "r",
                "a",
                "--package",
                "foo"
            ])),
            "project: p package: foo repository: r arch: a"
        );
        assert_eq!(
            coordinates_header(&parse_build(&["whatdependson", "p", "k", "r", "a"])),
            "project: p package: k repository: r arch: a"
        );
        assert_eq!(
            coordinates_header(&parse_build(&["list-binaries", "p", "r", "a"])),
            "project: p repository: r arch: a"
        );
        assert_eq!(
            coordinates_header(&parse_build(&["build-log", "p", "k", "r", "a"])),
            "project: p package: k repository: r arch: a"
        );
        // The helper still builds the header for `build-log`; the
        // run_build arm deliberately does not prepend it to the log
        // content (rule 4: the log is verbatim content).
        assert_eq!(
            coordinates_header(&parse_build(&[
                "wait-for-results",
                "p",
                "--deadline-secs",
                "30"
            ])),
            "project: p"
        );
        assert_eq!(
            coordinates_header(&parse_build(&["job-status", "p", "k", "r", "a"])),
            "project: p package: k repository: r arch: a"
        );
        assert_eq!(
            coordinates_header(&parse_build(&["buildinfo", "p", "k", "r", "a"])),
            "project: p package: k repository: r arch: a"
        );
        assert_eq!(
            coordinates_header(&parse_build(&["build-history", "p", "k", "r", "a"])),
            "project: p package: k repository: r arch: a"
        );
        let fileinfo_header =
            coordinates_header(&parse_build(&["fileinfo", "p", "k", "r", "a", "foo.rpm"]));
        assert_eq!(
            fileinfo_header,
            "project: p package: k repository: r arch: a"
        );
        assert!(
            !fileinfo_header.contains("foo.rpm"),
            "filename is not a coordinate"
        );
        let download_header = coordinates_header(&parse_build(&[
            "download-binary",
            "p",
            "k",
            "r",
            "a",
            "foo.rpm",
            "--output",
            "/tmp/x",
        ]));
        assert_eq!(
            download_header,
            "project: p package: k repository: r arch: a"
        );
        assert!(
            !download_header.contains("foo.rpm"),
            "filename is not a coordinate"
        );
        assert_eq!(
            coordinates_header(&parse_build(&["dependson", "p", "r", "a"])),
            "project: p repository: r arch: a"
        );
        assert_eq!(
            coordinates_header(&parse_build(&[
                "dependson",
                "p",
                "r",
                "a",
                "--package",
                "foo"
            ])),
            "project: p package: foo repository: r arch: a"
        );
        assert_eq!(
            coordinates_header(&parse_build(&["check-constraints", "p", "k", "r", "a"])),
            "project: p package: k repository: r arch: a"
        );
    }

    #[test]
    fn results_cell_level_code_is_payload_only_with_statuses() {
        // Decided omission (2026-11-21, Tom): a cell with statuses
        // shows no row for the cell-level code/state — the fixture
        // cell's code/state are both `published`, and neither may
        // appear in the text.
        let text = render_results_text(&results_fixture());
        assert!(!text.contains("published"));
    }

    #[test]
    fn results_payload_shape() {
        let payload = results_payload(&results_fixture());
        assert_eq!(payload["state"], "abc123");
        assert_eq!(payload["results"][0]["repository"], "standard");
        assert_eq!(payload["results"][0]["statuses"][0]["package"], "foo");
        assert_eq!(payload["results"][0]["statuses"][1]["code"], "failed");
        assert_eq!(
            payload["results"][0]["statuses"][1]["details"],
            "missing foo"
        );
    }

    #[test]
    fn results_text_has_table_and_summary() {
        let text = render_results_text(&results_fixture());
        assert!(text.contains("REPOSITORY  ARCH    PACKAGE  CODE"));
        assert!(text.contains("foo"));
        assert!(text.contains("bar"));
        assert!(text.contains("failed (missing foo)"));
        assert!(text.contains("1 failed, 1 succeeded"));
        assert!(text.contains("state: abc123"));
    }

    #[test]
    fn results_text_and_payload_agree() {
        let results = results_fixture();
        let payload = results_payload(&results);
        let text = render_results_text(&results);
        for status in payload["results"][0]["statuses"].as_array().unwrap() {
            let package = status["package"].as_str().unwrap();
            let code = status["code"].as_str().unwrap();
            assert!(text.contains(package), "text misses {package}");
            assert!(text.contains(code), "text misses {code}");
        }
    }

    #[test]
    fn results_empty() {
        let empty = BuildResults {
            state: None,
            results: vec![],
        };
        assert_eq!(render_results_text(&empty), "no results");
        assert_eq!(results_payload(&empty)["results"], json!([]));
    }

    #[test]
    fn results_cell_without_packages() {
        let results = BuildResults {
            state: None,
            results: vec![BuildResult {
                project: "openSUSE:Factory".to_string(),
                repository: "standard".to_string(),
                arch: "x86_64".to_string(),
                code: "broken".to_string(),
                state: "broken".to_string(),
                dirty: false,
                details: Some("no such repository".to_string()),
                statuses: vec![],
            }],
        };
        let text = render_results_text(&results);
        assert!(text.contains("broken (no such repository)"));
        assert!(text.contains("1 broken"));
    }

    #[test]
    fn results_dirty_cell_marks_its_rows() {
        let mut results = results_fixture();
        results.results[0].dirty = true;
        let text = render_results_text(&results);
        assert!(text.contains("succeeded (dirty)"));
        assert!(text.contains("failed (missing foo) (dirty)"));
    }

    #[test]
    fn results_cell_details_render_with_statuses() {
        let mut results = results_fixture();
        results.results[0].details = Some("repository is broken".to_string());
        let text = render_results_text(&results);
        assert!(text.contains("standard x86_64: repository is broken"));
    }

    #[test]
    fn workers_empty_and_list() {
        assert_eq!(render_workers_text(&[]), "no workers");
        let text = render_workers_text(&["worker:1".to_string(), "worker:2".to_string()]);
        assert!(text.contains("WORKER"));
        assert!(text.contains("worker:1"));
        assert!(text.contains("worker:2"));
    }

    #[test]
    fn job_status_none() {
        assert_eq!(render_job_status_text(&None), "no job running");
        let payload = job_status_payload("p", "k", "r", "a", &None);
        assert!(payload["job"].is_null());
    }

    #[test]
    fn job_status_some() {
        let job = Some(JobStatus {
            code: "building".to_string(),
            result: None,
            details: None,
            starttime: Some(1_700_000_000),
            endtime: None,
            lastduration: Some(42),
            workerid: Some("worker:1".to_string()),
            hostarch: Some("x86_64".to_string()),
            uri: None,
            arch: Some("x86_64".to_string()),
            job: Some("job-1".to_string()),
            jobid: None,
            attempt: Some(1),
        });
        let text = render_job_status_text(&job);
        assert!(text.contains("code: building"));
        assert!(text.contains("workerid: worker:1"));
        assert!(text.contains("starttime: 1700000000"));
        assert!(!text.contains("endtime"));
        assert_eq!(
            job_status_payload("p", "k", "r", "a", &job)["job"]["code"],
            "building"
        );
    }

    #[test]
    fn buildinfo_error_and_clean() {
        let blocked = BuildInfo {
            error: Some("unresolvable: DOES-NOT-EXIST".to_string()),
            bdeps: vec![],
            preinstall_image: None,
        };
        assert!(render_buildinfo_text(&blocked).contains("error: unresolvable: DOES-NOT-EXIST"));
        assert_eq!(
            buildinfo_payload("p", "k", "r", "a", &blocked)["error"],
            "unresolvable: DOES-NOT-EXIST"
        );
        let clean = BuildInfo {
            error: None,
            bdeps: vec![
                BuildDep {
                    name: "gcc".to_string(),
                    project: None,
                    repository: None,
                    package: None,
                    binary: None,
                    version: None,
                    release: None,
                    arch: None,
                    epoch: None,
                    hdrmd5: None,
                    preinstall: false,
                    vminstall: false,
                    runscripts: false,
                    noinstall: false,
                    repoarch: None,
                    installonly: false,
                    sysroot: false,
                },
                BuildDep {
                    name: "make".to_string(),
                    project: None,
                    repository: None,
                    package: None,
                    binary: None,
                    version: None,
                    release: None,
                    arch: None,
                    epoch: None,
                    hdrmd5: None,
                    preinstall: false,
                    vminstall: false,
                    runscripts: false,
                    noinstall: false,
                    repoarch: None,
                    installonly: false,
                    sysroot: false,
                },
            ],
            preinstall_image: None,
        };
        let text = render_buildinfo_text(&clean);
        assert!(text.contains("no blocking error"));
        assert!(text.contains("gcc"));
        assert!(text.contains("make"));
    }

    #[test]
    fn build_history_empty_and_entry() {
        assert_eq!(render_build_history_text(&[]), "no builds");
        let entries = vec![BuildHistoryEntry {
            rev: "1".to_string(),
            srcmd5: "deadbeef".to_string(),
            versrel: "1.0-1".to_string(),
            bcnt: 2,
            time: 1_700_000_000,
            duration: Some(120),
        }];
        let text = render_build_history_text(&entries);
        assert!(text.contains("1.0-1"));
        assert!(text.contains("deadbeef"));
        assert_eq!(
            history_payload("p", "k", "r", "a", &entries)["entries"][0]["bcnt"],
            2
        );
    }

    #[test]
    fn job_history_empty_and_entry() {
        assert_eq!(render_job_history_text(&[]), "no jobs");
        let entries = vec![JobHistoryEntry {
            repository: Some("standard".to_string()),
            arch: Some("x86_64".to_string()),
            package: "foo".to_string(),
            rev: Some("42".to_string()),
            srcmd5: Some("deadbeef".to_string()),
            versrel: Some("1.0-1".to_string()),
            bcnt: Some(1),
            readytime: Some(1_700_000_000),
            starttime: Some(1_700_000_050),
            endtime: Some(1_700_000_100),
            code: "succeeded".to_string(),
            uri: Some("https://worker.example/job-1".to_string()),
            workerid: Some("worker:1".to_string()),
            hostarch: Some("aarch64".to_string()),
            reason: Some("source change".to_string()),
            verifymd5: Some("abcdef".to_string()),
        }];
        let text = render_job_history_text(&entries);
        let header = text.lines().next().unwrap();
        assert!(header.contains("REV"));
        assert!(header.contains("SRCMD5"));
        assert!(header.contains("BCNT"));
        // Decided omission (2026-11-21, Tom): the worker and
        // verification detail is payload-only — the fixture entry
        // carries all five dropped fields.
        assert!(!text.contains("worker:1"));
        assert!(!text.contains("1700000050"));
        assert!(!text.contains("https://worker.example/job-1"));
        assert!(!text.contains("aarch64"));
        assert!(!text.contains("abcdef"));
        let row = text.lines().find(|line| line.contains("foo")).unwrap();
        assert!(row.contains("succeeded"));
        assert!(row.contains("source change"));
        assert!(row.contains("42"));
        assert!(row.contains("deadbeef"));
        assert!(row.contains("1.0-1"));
        assert_eq!(
            job_history_payload("p", "r", "a", &entries)["entries"][0]["package"],
            "foo"
        );
        assert_eq!(
            job_history_payload("p", "r", "a", &entries)["entries"][0]["reason"],
            "source change"
        );
        assert_eq!(
            job_history_payload("p", "r", "a", &entries)["entries"][0]["bcnt"],
            1
        );
    }

    #[test]
    fn binaries_empty_and_sized() {
        assert_eq!(render_binaries_text(&[]), "no binaries");
        let files = vec![
            BinaryFile {
                name: "foo-1.0-1.x86_64.rpm".to_string(),
                size: Some(1234),
                mtime: Some(1_700_000_000),
            },
            BinaryFile {
                name: "size-only.rpm".to_string(),
                size: Some(99),
                mtime: None,
            },
            BinaryFile {
                name: "mtime-only.rpm".to_string(),
                size: None,
                mtime: Some(1_700_000_001),
            },
            BinaryFile {
                name: "_buildenv".to_string(),
                size: None,
                mtime: None,
            },
        ];
        let text = render_binaries_text(&files);
        let header = text.lines().next().unwrap();
        assert!(header.contains("NAME"));
        assert!(header.contains("SIZE"));
        assert!(header.contains("MTIME"));
        let fields = |name: &str| -> Vec<&str> {
            text.lines()
                .find(|line| line.contains(name))
                .unwrap()
                .split_whitespace()
                .collect()
        };
        assert_eq!(
            fields("foo-1.0-1.x86_64.rpm"),
            ["foo-1.0-1.x86_64.rpm", "1234", "1700000000"]
        );
        assert_eq!(fields("size-only.rpm"), ["size-only.rpm", "99", "-"]);
        assert_eq!(
            fields("mtime-only.rpm"),
            ["mtime-only.rpm", "-", "1700000001"]
        );
        assert_eq!(fields("_buildenv"), ["_buildenv", "-", "-"]);
        assert_eq!(
            binaries_payload("p", "r", "a", &files)["files"][0]["size"],
            1234
        );
        assert_eq!(
            binaries_payload("p", "r", "a", &files)["files"][0]["mtime"],
            1_700_000_000
        );
    }

    #[test]
    fn write_downloaded_writes_bytes_and_fails_on_missing_parent() {
        let dir =
            std::env::temp_dir().join(format!("geectl-cli-build-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("foo.rpm");
        write_downloaded(&path, b"bytes").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"bytes");
        std::fs::remove_file(&path).unwrap();
        let missing_parent = dir.join("no-such-dir").join("foo.rpm");
        let err = write_downloaded(&missing_parent, b"bytes").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InternalError);
        assert!(err.context.contains("cannot write"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn fileinfo_fixture() -> FileInfo {
        FileInfo {
            filename: "foo-1.0-1.x86_64.rpm".to_string(),
            name: Some("foo".to_string()),
            epoch: None,
            version: Some("1.0".to_string()),
            release: Some("1".to_string()),
            arch: Some("x86_64".to_string()),
            source: None,
            summary: Some("A foo".to_string()),
            description: None,
            disturl: None,
            size: Some(1234),
            mtime: None,
            provides: vec!["foo = 1.0".to_string()],
            requires: vec!["libc.so.6()(64bit)".to_string()],
            prerequires: vec![],
            conflicts: vec![],
            obsoletes: vec![],
            recommends: vec![],
            supplements: vec![],
            suggests: vec![],
            enhances: vec![],
            provides_ext: vec![ExtDependency {
                dep: "foo = 1.0".to_string(),
                provided_by: vec![],
                required_by: vec![ProvidedBy {
                    name: "bar".to_string(),
                    epoch: None,
                    version: Some("2.0".to_string()),
                    release: None,
                    arch: None,
                    project: None,
                    repository: None,
                }],
            }],
            requires_ext: vec![ExtDependency {
                dep: "libc.so.6()(64bit)".to_string(),
                provided_by: vec![ProvidedBy {
                    name: "glibc".to_string(),
                    epoch: None,
                    version: None,
                    release: None,
                    arch: None,
                    project: None,
                    repository: None,
                }],
                required_by: vec![],
            }],
            recommends_ext: vec![],
            supplements_ext: vec![ExtDependency {
                dep: "foo-base".to_string(),
                provided_by: vec![],
                required_by: vec![ProvidedBy {
                    name: "bar".to_string(),
                    epoch: Some(1),
                    version: Some("2.0".to_string()),
                    release: Some("1".to_string()),
                    arch: Some("aarch64".to_string()),
                    project: Some("home:alice".to_string()),
                    repository: Some("myrepo".to_string()),
                }],
            }],
        }
    }

    #[test]
    fn fileinfo_text_and_payload() {
        let info = fileinfo_fixture();
        let text = render_fileinfo_text(&info);
        assert!(text.contains("file: foo-1.0-1.x86_64.rpm"));
        assert!(text.contains("summary: A foo"));
        assert!(text.contains("provides:"));
        assert!(text.contains("foo = 1.0 (required by: bar 2.0)"));
        assert!(text.contains("libc.so.6()(64bit) (provided by: glibc)"));
        assert!(text.contains("foo-base (required by: bar 2.0-1)"));
        // Decided omission (2026-11-21, Tom): a resolved package's
        // epoch, arch, project, and repository are payload-only —
        // the fixture's supplements_ext bar carries epoch 1, arch
        // aarch64, project home:alice, repository myrepo, and none
        // of those values may appear in the text.
        assert!(!text.contains("epoch"));
        assert!(!text.contains("aarch64"));
        assert!(!text.contains("home:alice"));
        assert!(!text.contains("myrepo"));
        let payload = fileinfo_payload("p", "k", "r", "a", &info);
        assert_eq!(payload["name"], "foo");
        assert_eq!(payload["provides_ext"][0]["required_by"][0]["name"], "bar");
        assert_eq!(
            payload["requires_ext"][0]["provided_by"][0]["name"],
            "glibc"
        );
        assert_eq!(
            payload["supplements_ext"][0]["required_by"][0]["release"],
            "1"
        );
    }

    #[test]
    fn deps_forward_reverse_and_empty() {
        let packages = vec![PackageDeps {
            name: "foo".to_string(),
            source: Some("foo".to_string()),
            dependencies: vec!["bar".to_string(), "baz".to_string()],
            subpackages: vec!["foo-devel".to_string()],
        }];
        let text = render_deps_text(&packages, false);
        assert!(text.contains("foo"));
        assert!(text.contains("depends on: bar, baz"));
        assert!(text.contains("subpackages: foo-devel"));
        let reverse = render_deps_text(&packages, true);
        assert!(reverse.contains("depended on by: bar, baz"));
        assert_eq!(render_deps_text(&[], false), "no dependencies");
        assert_eq!(render_deps_text(&[], true), "no dependents");
        assert_eq!(
            deps_payload("p", "r", "a", &packages)["packages"][0]["name"],
            "foo"
        );
    }

    #[test]
    fn repositories_empty_and_entry() {
        assert_eq!(render_repositories_text(&[]), "no repositories");
        let repos = vec![Repository {
            name: "standard".to_string(),
            arch: vec!["x86_64".to_string(), "aarch64".to_string()],
            paths: vec![("openSUSE:Factory".to_string(), "standard".to_string())],
        }];
        let text = render_repositories_text(&repos);
        assert!(text.contains("standard (x86_64, aarch64)"));
        assert!(text.contains("path: openSUSE:Factory/standard"));
        assert_eq!(
            repositories_payload("p", &repos)["repositories"][0]["name"],
            "standard"
        );
    }

    fn parse_build(args: &[&str]) -> BuildCommand {
        let mut full = vec!["build"];
        full.extend_from_slice(args);
        let matches = build_command().try_get_matches_from(full).unwrap();
        BuildCommand::from_arg_matches(&matches).unwrap()
    }

    #[test]
    fn clap_results_with_filters() {
        match parse_build(&[
            "results",
            "openSUSE:Factory",
            "--package",
            "foo",
            "--code",
            "failed",
            "--oldstate",
            "abc123",
        ]) {
            BuildCommand::Results {
                project,
                query,
                oldstate,
            } => {
                assert_eq!(project, "openSUSE:Factory");
                assert_eq!(query.package.as_deref(), Some("foo"));
                assert_eq!(query.code.as_deref(), Some("failed"));
                assert_eq!(oldstate.as_deref(), Some("abc123"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn clap_wait_for_results_rejects_oldstate() {
        // `wait-for-results` polls plainly; the library strips a
        // caller-set `oldstate` there, so the flag is not offered and
        // clap rejects it instead of silently ignoring it.
        let matches = build_command().try_get_matches_from([
            "build",
            "wait-for-results",
            "openSUSE:Factory",
            "--deadline-secs",
            "30",
            "--oldstate",
            "abc123",
        ]);
        assert!(matches.is_err());
    }

    #[test]
    fn clap_build_log_negative_start() {
        match parse_build(&[
            "build-log",
            "p",
            "k",
            "r",
            "a",
            "--start",
            "-5",
            "--lastsucceeded",
        ]) {
            BuildCommand::BuildLog {
                start,
                lastsucceeded,
                ..
            } => {
                assert_eq!(start, Some(-5));
                assert!(lastsucceeded);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn clap_kebab_names() {
        assert!(matches!(
            parse_build(&["wait-for-results", "p", "--deadline-secs", "30"]),
            BuildCommand::WaitForResults { .. }
        ));
        assert!(matches!(
            parse_build(&["build-status", "p", "k", "r", "a"]),
            BuildCommand::BuildStatus { .. }
        ));
        assert!(matches!(
            parse_build(&["job-status", "p", "k", "r", "a"]),
            BuildCommand::JobStatus { .. }
        ));
        assert!(matches!(
            parse_build(&["build-history", "p", "k", "r", "a"]),
            BuildCommand::BuildHistory { .. }
        ));
        assert!(matches!(
            parse_build(&["job-history", "p", "r", "a"]),
            BuildCommand::JobHistory { .. }
        ));
        assert!(matches!(
            parse_build(&["list-binaries", "p", "r", "a"]),
            BuildCommand::ListBinaries { .. }
        ));
        assert!(matches!(
            parse_build(&["list-repositories", "p"]),
            BuildCommand::ListRepositories { .. }
        ));
        assert!(matches!(
            parse_build(&["check-constraints", "p", "k", "r", "a"]),
            BuildCommand::CheckConstraints { .. }
        ));
    }

    #[test]
    fn clap_missing_project_fails() {
        let matches = build_command().try_get_matches_from(["build", "results"]);
        assert!(matches.is_err());
    }
}
