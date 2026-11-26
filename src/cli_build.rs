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
//! the payload names the written file; with `--all` it writes every
//! listed file into the `--output` directory and the payload names
//! each one. `check-constraints` takes the
//! constraints document inline (`--constraints`): the one place raw
//! XML crosses the public API, because there is no typed constraints
//! model. `buildconfig` returns raw text: the payload carries it as
//! one string and the text renders it verbatim, like `build-log`.
//!
//! Coordinate header (design rule 1):
//! every command's text starts with one header line echoing the query
//! coordinates — the required positionals, in the order project,
//! package, repository, arch (`project: p package: k repository: r
//! arch: a`), plus `package:` when the query is package-scoped. Tables stay as the design's
//! worked examples pin them; the header is the only addition. The
//! exceptions are `build-log` and `buildconfig`: their text *is* the
//! content, verbatim (design rule 4), so no header is prepended to
//! the content itself — the coordinates live in the JSON payload, and
//! the synthetic `no log` line (build-log only) does carry the header.
//!
//! Documented omissions (not gaps — see the renderers):
//! job-history text carries 11 of 16 fields (worker and verification
//! detail is payload-only); `fileinfo` resolved sections drop
//! epoch/arch/project/repository per package; a results cell with
//! statuses shows no row for the cell-level code/state (the
//! per-package rows carry the codes, the cell detail rides a note
//! line).
//!
//! Mutations (`rebuild-package`, `restart-build`, `abort-build`,
//! `wipe-binaries`)
//! follow the CLI-wide rule: dry-run by default, the global `--live`
//! performs them, and the text opens with the coordinate header, then
//! `dry_run:`, then the result line and the plan.

use crate::auth::{alias_for_url, same_api_url, Credentials, EnvObsAuth};
use crate::config;
use crate::debug::{strip_terminal_controls, Debug};
use crate::error::{ErrorKind, GeectlError};
use crate::http::{Timeout, UreqClient};
use crate::obs::{
    parse_build_log_url, results_across_instances, valid_obs_filename, valid_obs_name, BinaryFile,
    BuildHistoryEntry, BuildInfo, BuildLogQuery, BuildResults, DryRun, ExtDependency, FileInfo,
    InstanceBuildResults, JobHistoryEntry, JobStatus, ObsClient, PackageDeps, ProvidedBy,
    Repository, ResultsQuery, WipeBinariesOptions, WipeCode, OBS_API_BASE,
};
use clap::{Args, Subcommand};
use fs2::FileExt;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
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
        /// plainly.
        #[arg(long)]
        oldstate: Option<String>,
        /// Merge the results from every configured instance into
        /// one view, instead of only the instance currently pointed
        /// at. Cannot combine with `--oldstate`: a state hash is
        /// per instance. A partially-failing read still exits 0:
        /// per-instance errors render in the view and payload, and
        /// only an every-instance failure is an error.
        #[arg(long, conflicts_with = "oldstate")]
        all_instances: bool,
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
        /// The project the package is in — or a build-log URL, in
        /// which case the other coordinates are omitted. The URL
        /// form takes an API log URL
        /// (`http(s)://<api-host>/build/<project>/<repository>/<arch>/<package>/_log`)
        /// or a web-UI log URL
        /// (`http(s)://build.opensuse.org/package/live_build_log/<project>/<package>/<repository>/<arch>`).
        project: String,
        /// The package to read the log for. Omit with a URL.
        package: Option<String>,
        /// The repository the package builds in. Omit with a URL.
        repository: Option<String>,
        /// The architecture the package builds for. Omit with a URL.
        arch: Option<String>,
        /// Start reading at this byte offset; a negative offset reads
        /// from the end of the log.
        #[arg(long, allow_hyphen_values = true)]
        start: Option<i64>,
        /// The last build's log instead of the current one. Cannot
        /// combine with `--lastsucceeded`.
        #[arg(long, conflicts_with = "lastsucceeded")]
        last: bool,
        /// The last succeeded build's log. Cannot combine with `--last`.
        #[arg(long)]
        lastsucceeded: bool,
        /// Only this multibuild flavor's log: appends `:FLAVOR` to
        /// the package. Refused when the package already carries a
        /// flavor.
        #[arg(long, value_name = "FLAVOR")]
        multibuild: Option<String>,
        /// Strip the leading `[timestamp] ` from each log line.
        #[arg(long)]
        strip_time: bool,
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
    /// Rebuild a package: rerun a build without touching the
    /// sources. A mutation: dry-run is the default (only the plan
    /// is printed); `--live` performs the rebuild.
    RebuildPackage {
        /// The project the package is in.
        project: String,
        /// The package to rebuild.
        package: String,
        /// The repository the package builds in.
        repository: String,
        /// The architecture the package builds for.
        arch: String,
    },
    /// Restart the running build of a package, without touching
    /// the sources. A mutation: dry-run is the default (only the
    /// plan is printed); `--live` performs the restart.
    RestartBuild {
        /// The project the package is in.
        project: String,
        /// The package to restart the build for.
        package: String,
        /// The repository the package builds in.
        repository: String,
        /// The architecture the package builds for.
        arch: String,
    },
    /// Abort the running build of a package. A mutation: dry-run
    /// is the default (only the plan is printed); `--live`
    /// performs the abort.
    AbortBuild {
        /// The project the package is in.
        project: String,
        /// The package to abort the build for.
        package: String,
        /// The repository the package builds in.
        repository: String,
        /// The architecture the package builds for.
        arch: String,
    },
    /// Wipe the binary packages of a project or a package. A
    /// mutation: dry-run is the default (only the plan is printed);
    /// `--live` performs the wipe. At least one scope selector is
    /// required: `--all`, `--arch`, `--repository`, or a status
    /// filter.
    WipeBinaries {
        /// The project to wipe binaries in.
        project: String,
        /// The package to wipe binaries for. Omit to wipe the whole
        /// project.
        package: Option<String>,
        /// Wipe only binaries for this architecture.
        #[arg(long)]
        arch: Option<String>,
        /// Wipe only binaries for this repository.
        #[arg(long)]
        repository: Option<String>,
        /// Wipe all binaries regardless of package status. Combines
        /// with no status filter.
        #[arg(long)]
        all: bool,
        /// Wipe only binaries of packages whose build is disabled.
        #[arg(long)]
        build_disabled: bool,
        /// Wipe only binaries of packages whose build failed.
        #[arg(long)]
        build_failed: bool,
        /// Wipe only binaries of packages whose sources are bad.
        #[arg(long)]
        broken: bool,
        /// Wipe only binaries of packages with dependency errors.
        #[arg(long)]
        unresolvable: bool,
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
        /// Read the buildinfo for another project instead of
        /// `project`: the dep set the package would build against
        /// there.
        #[arg(long)]
        alternative_project: Option<String>,
    },
    /// Read the build configuration for a project/repository: the
    /// expanded `_buildconfig` text the engine takes as `--dist`.
    Buildconfig {
        /// The project to read the build configuration for.
        project: String,
        /// The repository to read the build configuration for.
        repository: String,
        /// Read the build configuration for another project instead
        /// of `project`: the expanded `_buildconfig` text the engine
        /// takes as `--dist` there.
        #[arg(long)]
        alternative_project: Option<String>,
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
    /// Download one built file by name. With `--all`, every built
    /// file.
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
        /// Required unless `--all` downloads every built file.
        #[arg(required_unless_present = "all")]
        filename: Option<String>,
        /// Where to write the downloaded bytes: the file path (it
        /// must not already exist), or the directory every built
        /// file goes into with `--all`.
        #[arg(long)]
        output: PathBuf,
        /// Download every built file into the `--output` directory;
        /// a file already there is skipped and reported, never
        /// re-downloaded and never deleted. The downloads are
        /// sequential, not atomic: if one fails, the files already
        /// downloaded stay.
        #[arg(long, conflicts_with = "filename")]
        all: bool,
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

/// Resolve the `build-log` coordinates: either all four
/// positionals, or a single build-log URL in the first positional.
/// A URL takes no other coordinates, and `--multibuild` appends
/// `:FLAVOR` to the resolved package (refused when the package
/// already carries a flavor, or when the flavor is not a valid
/// OBS name fragment).
fn resolve_build_log_coordinates(
    project: &str,
    package: Option<&str>,
    repository: Option<&str>,
    arch: Option<&str>,
    multibuild: Option<&str>,
    api_base: &str,
) -> Result<(String, String, String, String), GeectlError> {
    let (project, mut package, repository, arch) = match (package, repository, arch) {
        (Some(package), Some(repository), Some(arch)) => {
            if is_build_log_url(project) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "a build-log URL takes no other coordinates: pass the URL alone",
                ));
            }
            (
                project.to_string(),
                package.to_string(),
                repository.to_string(),
                arch.to_string(),
            )
        }
        (None, None, None) => {
            let coords = parse_build_log_url(project, api_base)?;
            (
                coords.project,
                coords.package,
                coords.repository,
                coords.arch,
            )
        }
        _ => {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "pass all four coordinates (PROJECT PACKAGE REPOSITORY ARCH) or a single build-log URL",
            ));
        }
    };
    if let Some(flavor) = multibuild {
        if flavor.contains(':') || !valid_obs_name(flavor) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("--multibuild needs a flavor name, not {flavor:?}"),
            ));
        }
        if package.contains(':') {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("package {package:?} already carries a flavor; --multibuild adds another"),
            ));
        }
        package = format!("{package}:{flavor}");
    }
    Ok((project, package, repository, arch))
}

/// True when the first positional is a pasted build-log URL rather
/// than a project name: project names never carry `://`.
fn is_build_log_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

/// The coordinate header for a resolved `build-log`: the four
/// coordinates, in order. It echoes resolved coordinates, never a
/// pasted URL (the helper's arm below can only echo what was given,
/// so `run_build` does not use it for `build-log`).
fn build_log_header(project: &str, package: &str, repository: &str, arch: &str) -> String {
    format!("project: {project} package: {package} repository: {repository} arch: {arch}")
}

/// Run one `build` subcommand: build the OBS client from the
/// effective config and credentials, call the library operation, and
/// return the `(payload, text)` pair. Every build command except
/// `list-repositories` requires OBS credentials and fails
/// loudly without them (the library's `require_build_credentials`),
/// except that the three control mutations resolve no credentials
/// in dry-run, like the `remote` mutations.
/// Credentials resolve per command: `list-repositories` is a public
/// read and never touches them (a half-set credential env must not
/// break it); every other command fails fast on a half-set pair up
/// front, unless it is a dry-run control preview. How the other
/// groups handle a half-configured pair is stated in `docs/cli.md`.
pub fn run_build(
    debug: &Debug,
    command: BuildCommand,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    // Credentials resolve lazily for `list-repositories`: the read is
    // public and never touches them, so a half-set credential env
    // (username without password) must not break it.
    let (base, instances) = obs_endpoint()?;
    // The build-log coordinates resolve before credentials: a pasted
    // URL or a half-given coordinate set fails with its own typed
    // error instead of a credentials error, and the header below
    // echoes resolved coordinates, never a pasted URL.
    let resolved_log = match &command {
        BuildCommand::BuildLog {
            project,
            package,
            repository,
            arch,
            multibuild,
            ..
        } => Some(resolve_build_log_coordinates(
            project,
            package.as_deref(),
            repository.as_deref(),
            arch.as_deref(),
            multibuild.as_deref(),
            &base,
        )?),
        _ => None,
    };
    let credentials = match &command {
        BuildCommand::ListRepositories { .. } => Credentials::None,
        // `results --all-instances` resolves credentials per
        // instance inside the merged read: a half-set pair for one
        // instance surfaces as that instance's error instead of
        // failing the run before the other instances are read.
        BuildCommand::Results {
            all_instances: true,
            ..
        } => Credentials::None,
        BuildCommand::RebuildPackage { .. }
        | BuildCommand::RestartBuild { .. }
        | BuildCommand::AbortBuild { .. }
        | BuildCommand::WipeBinaries { .. }
            if !live =>
        {
            Credentials::None
        }
        _ => EnvObsAuth::credentials_for_url(&base, &base, &instances)?,
    };
    let client =
        ObsClient::with_base_and_credentials(&http, &base, credentials)?.with_debug(debug.clone());
    // The coordinate header (design rule 1) is computed from the
    // command before the match moves it — except `build-log`, whose
    // header comes from the resolved coordinates above, so a pasted
    // URL never echoes as a project. Every arm prepends it — except
    // the `BuildLog` and `Buildconfig` content itself, whose text is
    // the content verbatim (rule 4); the `BuildLog` synthetic
    // `no log` line still carries the header.
    let header = match &resolved_log {
        Some((project, package, repository, arch)) => {
            build_log_header(project, package, repository, arch)
        }
        None => coordinates_header(&command),
    };
    match command {
        BuildCommand::Results {
            project,
            query,
            oldstate,
            all_instances,
        } => {
            let query = query.into_query(oldstate);
            if all_instances {
                results_all_instances(&http, debug, &base, &instances, &project, &query, &header)
            } else {
                let results = client.results(&project, &query)?;
                Ok((
                    results_payload(&results),
                    with_header(&header, render_results_text(&results)),
                ))
            }
        }
        BuildCommand::WaitForResults {
            project,
            query,
            deadline_secs,
        } => {
            let query = query.into_query(None);
            let results = client.wait_for_results(
                &project,
                &query,
                Duration::from_secs(deadline_secs),
                None,
                &[],
            )?;
            Ok((
                results_payload(&results),
                with_header(&header, render_results_text(&results)),
            ))
        }
        BuildCommand::BuildLog {
            start,
            last,
            lastsucceeded,
            strip_time,
            ..
        } => {
            let (project, package, repository, arch) =
                resolved_log.expect("build-log coordinates resolve before dispatch");
            let query = BuildLogQuery {
                start,
                last,
                lastsucceeded,
                strip_time,
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
        BuildCommand::RebuildPackage {
            project,
            package,
            repository,
            arch,
        } => {
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            client.rebuild_package(&project, &repository, &arch, &package, &dry_run)?;
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(build_control_result(
                BuildControlAction::Rebuild,
                &project,
                &package,
                &repository,
                &arch,
                dry_run.enabled(),
                plan,
            ))
        }
        BuildCommand::RestartBuild {
            project,
            package,
            repository,
            arch,
        } => {
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            client.restart_build(&project, &repository, &arch, &package, &dry_run)?;
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(build_control_result(
                BuildControlAction::Restart,
                &project,
                &package,
                &repository,
                &arch,
                dry_run.enabled(),
                plan,
            ))
        }
        BuildCommand::AbortBuild {
            project,
            package,
            repository,
            arch,
        } => {
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            client.abort_build(&project, &repository, &arch, &package, &dry_run)?;
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(build_control_result(
                BuildControlAction::Abort,
                &project,
                &package,
                &repository,
                &arch,
                dry_run.enabled(),
                plan,
            ))
        }
        BuildCommand::WipeBinaries {
            project,
            package,
            arch,
            repository,
            all,
            build_disabled,
            build_failed,
            broken,
            unresolvable,
        } => {
            let mut codes = Vec::new();
            if build_disabled {
                codes.push(WipeCode::Disabled);
            }
            if build_failed {
                codes.push(WipeCode::Failed);
            }
            if broken {
                codes.push(WipeCode::Broken);
            }
            if unresolvable {
                codes.push(WipeCode::Unresolvable);
            }
            let options = WipeBinariesOptions {
                package,
                arch,
                repository,
                all,
                codes,
            };
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            client.wipe_binaries(&project, &options, &dry_run)?;
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(wipe_binaries_result(
                &project,
                &options,
                dry_run.enabled(),
                plan,
            ))
        }
        BuildCommand::Buildinfo {
            project,
            package,
            repository,
            arch,
            alternative_project,
        } => {
            let fetch = alternative_project.as_deref().unwrap_or(&project);
            let info = client.buildinfo(fetch, &package, &repository, &arch)?;
            Ok((
                buildinfo_payload(
                    &project,
                    &package,
                    &repository,
                    &arch,
                    alternative_project.as_deref(),
                    &info,
                ),
                with_header(&header, render_buildinfo_text(&info)),
            ))
        }
        BuildCommand::Buildconfig {
            project,
            repository,
            alternative_project,
        } => {
            let fetch = alternative_project.as_deref().unwrap_or(&project);
            let config = client.buildconfig(fetch, &repository)?;
            // No coordinate header on the config itself: the text *is*
            // the build configuration, verbatim (design rule 4) — the
            // coordinates live in the JSON payload.
            Ok(buildconfig_result(
                &project,
                &repository,
                alternative_project.as_deref(),
                config,
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
            all,
        } => {
            if all {
                // One list call plus one streaming download per
                // file: the streamed download has no 10 MiB cap.
                // A file already in the output directory is
                // skipped and reported, never re-downloaded and
                // never deleted — a second run into the same
                // directory downloads only what is missing, so a
                // retry is not refused by the files the earlier
                // run left. A skipped file deleted after the
                // partition is downloaded in a second pass, not
                // reported as present (see `recheck_skipped`). The names
                // are validated up front,
                // before anything downloads, so a bad server name
                // fails the run with nothing downloaded; the size
                // of what this run downloads is checked against
                // the output filesystem before the first
                // download, so a disk that cannot hold the run
                // fails fast instead of mid-run. A failure aborts
                // the run; the files already downloaded stay, and
                // the failed file's partial download is removed
                // only when this run created it — a file that
                // pre-existed or was never created (a failure
                // before the first byte) is never deleted.
                let files = client.list_binaries(&project, Some(&package), &repository, &arch)?;
                validate_download_names(&files)?;
                std::fs::create_dir_all(&output).map_err(|e| {
                    GeectlError::new(
                        ErrorKind::InternalError,
                        format!("cannot create output directory {}: {e}", output.display()),
                    )
                })?;
                let output_canonical = std::fs::canonicalize(&output).map_err(|e| {
                    GeectlError::new(
                        ErrorKind::InternalError,
                        format!("cannot resolve output directory {}: {e}", output.display()),
                    )
                })?;
                let (to_download, mut skipped) =
                    partition_downloads(&files, &output, &output_canonical)?;
                let pending: Vec<BinaryFile> =
                    to_download.iter().map(|(file, _)| file.clone()).collect();
                check_disk_space(&output, total_download_size(&pending)?)?;
                let mut downloaded: Vec<(String, u64, PathBuf)> = Vec::new();
                // At most two passes: what was missing at partition,
                // then anything a skipped entry lost since (see
                // `recheck_skipped`) — a file deleted mid-run is
                // downloaded, not reported as present.
                let mut pending = to_download;
                let mut rechecked = false;
                loop {
                    for (file, dest) in pending {
                        match download_one_file(
                            |dest| {
                                client.download_binary_to_file(
                                    &project,
                                    &package,
                                    &repository,
                                    &arch,
                                    &file.name,
                                    dest,
                                )
                            },
                            &dest,
                        ) {
                            Ok(size) => downloaded.push((file.name.clone(), size, dest)),
                            // A file that appeared between the
                            // partition and the download is a skip, not
                            // a failure: the run still proceeds for the
                            // remaining files.
                            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                                skipped.push((file.name.clone(), dest));
                            }
                            Err(e) => return Err(e),
                        }
                    }
                    if rechecked {
                        break;
                    }
                    rechecked = true;
                    pending = recheck_skipped(&files, &mut skipped);
                    if pending.is_empty() {
                        break;
                    }
                }
                let payload = json!({
                    "project": project,
                    "package": package,
                    "repository": repository,
                    "arch": arch,
                    "output": output.to_string_lossy(),
                    "files": downloaded
                        .iter()
                        .map(|(filename, size, dest)| {
                            json!({
                                "filename": filename,
                                "path": dest.to_string_lossy(),
                                "size_bytes": size,
                            })
                        })
                        .collect::<Vec<_>>(),
                    "skipped": skipped
                        .iter()
                        .map(|(filename, dest)| {
                            json!({
                                "filename": filename,
                                "path": dest.to_string_lossy(),
                            })
                        })
                        .collect::<Vec<_>>(),
                });
                let text = download_all_text(&downloaded, &skipped);
                Ok((payload, with_header(&header, text)))
            } else {
                let filename = match filename {
                    Some(filename) => filename,
                    // Unreachable: clap requires the filename
                    // unless --all is set — kept as a defensive
                    // error rather than an unwrap.
                    None => {
                        return Err(GeectlError::new(
                            ErrorKind::InvalidInput,
                            "download-binary needs a file name, or --all",
                        ));
                    }
                };
                // Refuse an existing destination before downloading: the
                // bytes would be refused by write_downloaded anyway, and
                // the download is wasted work. The exclusive create in
                // write_downloaded is still the enforcement.
                if output.exists() {
                    return Err(GeectlError::new(
                        ErrorKind::AlreadyExists,
                        format!(
                            "cannot create download file {}: already exists",
                            output.display()
                        ),
                    ));
                }
                let bytes =
                    client.download_binary(&project, &package, &repository, &arch, &filename)?;
                write_downloaded(&output, &bytes)?;
                let size = bytes.len() as u64;
                let payload = json!({
                    "project": project,
                    "package": package,
                    "repository": repository,
                    "arch": arch,
                    "filename": filename,
                    "path": output.to_string_lossy(),
                    "size_bytes": size,
                });
                let text = download_text(&filename, size, &output);
                Ok((payload, with_header(&header, text)))
            }
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

/// One header line echoing the query coordinates (design rule 1):
/// the required positionals, in the order
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
        BuildCommand::Buildinfo {
            project,
            package,
            repository,
            arch,
            alternative_project,
        } => {
            coords.push(format!("project: {project}"));
            coords.push(format!("package: {package}"));
            coords.push(format!("repository: {repository}"));
            coords.push(format!("arch: {arch}"));
            if let Some(alt) = alternative_project {
                coords.push(format!("alternative-project: {alt}"));
            }
        }
        BuildCommand::BuildLog {
            project,
            package,
            repository,
            arch,
            ..
        } => {
            // Only the unresolved form reaches here: run_build
            // resolves build-log coordinates before dispatch and
            // headers the resolved form, so this echoes what was
            // given (a URL form echoes the URL as the project).
            coords.push(format!("project: {project}"));
            if let Some(package) = package {
                coords.push(format!("package: {package}"));
            }
            if let Some(repository) = repository {
                coords.push(format!("repository: {repository}"));
            }
            if let Some(arch) = arch {
                coords.push(format!("arch: {arch}"));
            }
        }
        BuildCommand::BuildStatus {
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
        | BuildCommand::RebuildPackage {
            project,
            package,
            repository,
            arch,
        }
        | BuildCommand::RestartBuild {
            project,
            package,
            repository,
            arch,
        }
        | BuildCommand::AbortBuild {
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
        BuildCommand::Buildconfig {
            project,
            repository,
            alternative_project,
        } => {
            coords.push(format!("project: {project}"));
            coords.push(format!("repository: {repository}"));
            if let Some(alt) = alternative_project {
                coords.push(format!("alternative-project: {alt}"));
            }
        }
        BuildCommand::WipeBinaries {
            project,
            package,
            repository,
            arch,
            ..
        } => {
            coords.push(format!("project: {project}"));
            if let Some(package) = package {
                coords.push(format!("package: {package}"));
            }
            if let Some(repository) = repository {
                coords.push(format!("repository: {repository}"));
            }
            if let Some(arch) = arch {
                coords.push(format!("arch: {arch}"));
            }
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

/// The build-control action a `rebuild-package`, `restart-build` or
/// `abort-build` call performs.
#[derive(Debug, Clone, Copy)]
enum BuildControlAction {
    Rebuild,
    Restart,
    Abort,
}

/// The `rebuild-package` / `restart-build` / `abort-build` result:
/// the payload carries the coordinates; the text states the performed
/// action, or the dry-run plan.
fn build_control_result(
    action: BuildControlAction,
    project: &str,
    package: &str,
    repository: &str,
    arch: &str,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let header =
        format!("project: {project} package: {package} repository: {repository} arch: {arch}");
    let result_line = match action {
        BuildControlAction::Rebuild => {
            if dry_run {
                format!("dry-run: would rebuild package {project}/{package} ({repository}/{arch})")
            } else {
                format!("rebuilt package {project}/{package} ({repository}/{arch})")
            }
        }
        BuildControlAction::Restart => {
            if dry_run {
                format!(
                    "dry-run: would restart the build of {project}/{package} ({repository}/{arch})"
                )
            } else {
                format!("restarted the build of {project}/{package} ({repository}/{arch})")
            }
        }
        BuildControlAction::Abort => {
            if dry_run {
                format!(
                    "dry-run: would abort the build of {project}/{package} ({repository}/{arch})"
                )
            } else {
                format!("aborted the build of {project}/{package} ({repository}/{arch})")
            }
        }
    };
    mutation_result(
        json!({
            "project": project,
            "package": package,
            "repository": repository,
            "arch": arch,
        }),
        header,
        result_line,
        dry_run,
        plan,
    )
}

/// The `wipe-binaries` result: the payload carries the scope;
/// the text states the wipe, or the dry-run plan.
fn wipe_binaries_result(
    project: &str,
    options: &WipeBinariesOptions,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let mut header_parts = vec![format!("project: {project}")];
    if let Some(package) = &options.package {
        header_parts.push(format!("package: {package}"));
    }
    if let Some(repository) = &options.repository {
        header_parts.push(format!("repository: {repository}"));
    }
    if let Some(arch) = &options.arch {
        header_parts.push(format!("arch: {arch}"));
    }
    let header = header_parts.join(" ");
    let scope = match &options.package {
        Some(package) => format!("package {package} in project {project}"),
        None => format!("project {project}"),
    };
    let suffix = wipe_binaries_options_suffix(options);
    let result_line = if dry_run {
        format!("dry-run: would wipe binaries in {scope}{suffix}")
    } else {
        format!("wiped binaries in {scope}{suffix}")
    };
    mutation_result(
        json!({
            "project": project,
            "package": options.package,
            "arch": options.arch,
            "repository": options.repository,
            "all": options.all,
            "codes": options.codes.iter().map(|code| code.as_str()).collect::<Vec<_>>(),
        }),
        header,
        result_line,
        dry_run,
        plan,
    )
}

/// The option suffix for a wipe-binaries plan/result line: the set
/// scope selectors, so a dry-run plan reads what the wipe would do.
fn wipe_binaries_options_suffix(options: &WipeBinariesOptions) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(arch) = &options.arch {
        parts.push(format!("arch {arch}"));
    }
    if let Some(repository) = &options.repository {
        parts.push(format!("repository {repository}"));
    }
    if options.all {
        parts.push("all".to_string());
    }
    if !options.codes.is_empty() {
        let codes: Vec<&str> = options.codes.iter().map(|code| code.as_str()).collect();
        parts.push(format!("codes {}", codes.join(", ")));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

/// The mutation result: the payload gains `dry_run` (and the plan
/// when dry-running), and the text opens with the coordinate header,
/// then `dry_run:`, then the result line and the plan.
fn mutation_result(
    mut payload: Value,
    header: String,
    result_line: String,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    payload["dry_run"] = json!(dry_run);
    let mut lines = vec![header, format!("dry_run: {dry_run}"), result_line];
    if let Some(plan) = plan {
        payload["plan"] = json!(plan);
        lines.extend(plan);
    }
    (payload, lines.join("\n"))
}

/// A server-reported binary name is safe to join onto the output
/// directory when it is a valid OBS filename with no backslash, NUL,
/// or colon: `valid_obs_filename` rejects `/` and a leading `.`, the
/// backslash is the Windows separator, and NUL and `:` (a
/// drive-relative name on Windows) can never be a file name — so a
/// safe name is a single path component on every platform.
fn is_safe_download_name(name: &str) -> bool {
    valid_obs_filename(name) && !name.contains('\\') && !name.contains('\0') && !name.contains(':')
}

/// Fail fast on the names a `--all` run would download: every name
/// must be safe to join onto the output directory (see
/// [`is_safe_download_name`]). A bad server name fails the run
/// before anything downloads, instead of hard-failing mid-run after
/// earlier files landed.
fn validate_download_names(files: &[BinaryFile]) -> Result<(), GeectlError> {
    for file in files {
        let name = &file.name;
        if !is_safe_download_name(name) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS binary filename: {name:?}"),
            ));
        }
    }
    Ok(())
}

/// The total bytes a `--all` run expects, summed from the sizes the
/// server reported. A file without a reported size contributes zero,
/// so the total is a lower bound when any size is unknown.
fn total_download_size(files: &[BinaryFile]) -> Result<u64, GeectlError> {
    let mut total: u64 = 0;
    for file in files {
        if let Some(size) = file.size {
            total = total.checked_add(size).ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::ServerError,
                    "binary list sizes overflow: the server reported sizes that do not fit",
                )
            })?;
        }
    }
    Ok(total)
}

/// What a `--all` run downloads, and what it skips as already
/// present: the files to download (with their destinations), and
/// the files already in the output directory (with their paths).
type DownloadPlan = (Vec<(BinaryFile, PathBuf)>, Vec<(String, PathBuf)>);

/// Split a `--all` file list into the files to download and the
/// files already in the output directory: a destination that
/// exists (file, directory, or symlink) is skipped, never
/// re-downloaded and never deleted. The split happens before the
/// disk-space check, so the probe covers only what this run
/// downloads.
fn partition_downloads(
    files: &[BinaryFile],
    output: &Path,
    output_canonical: &Path,
) -> Result<DownloadPlan, GeectlError> {
    let mut to_download = Vec::new();
    let mut skipped = Vec::new();
    for file in files {
        let dest = contained_dest(output, output_canonical, &file.name)?;
        if dest.symlink_metadata().is_ok() {
            skipped.push((file.name.clone(), dest));
        } else {
            to_download.push((file.clone(), dest));
        }
    }
    Ok((to_download, skipped))
}

/// Re-check the skipped entries after the download loop: a file
/// present at partition may have been deleted since, and reporting
/// it as skipped would claim a file that no longer exists. Entries
/// that still exist stay skipped; the vanished ones come back, with
/// their server entries, for one download pass instead of a false
/// skip line.
fn recheck_skipped(
    files: &[BinaryFile],
    skipped: &mut Vec<(String, PathBuf)>,
) -> Vec<(BinaryFile, PathBuf)> {
    let mut missing = Vec::new();
    skipped.retain(|(name, dest)| {
        if dest.symlink_metadata().is_ok() {
            true
        } else {
            if let Some(file) = files.iter().find(|f| &f.name == name) {
                missing.push((file.clone(), dest.clone()));
            }
            false
        }
    });
    missing
}

/// Join a validated binary name onto the output directory, confined
/// to it: the name is one path component (see
/// [`is_safe_download_name`]), so joining it onto the canonical
/// directory cannot escape it; the `starts_with` check is the runtime
/// proof of that, against the resolved directory — a symlinked
/// `--output` cannot fool it. The returned path keeps the caller's
/// spelling of the directory.
fn contained_dest(
    output: &Path,
    output_canonical: &Path,
    name: &str,
) -> Result<PathBuf, GeectlError> {
    if !is_safe_download_name(name) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid OBS binary filename: {name:?}"),
        ));
    }
    let canonical_dest = output_canonical.join(name);
    if !canonical_dest.starts_with(output_canonical) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("download name {name:?} escapes the output directory"),
        ));
    }
    Ok(output.join(name))
}

/// Download one file, cleaning up only what this run created: the
/// destination is recorded before the download starts, and a failure
/// removes it only when it did not exist before — a file that
/// pre-existed (refused by exclusive create) or was never created (a
/// failure before the first byte) is never removed. An existence
/// check that itself errors assumes the file existed: fail closed,
/// never delete. The check does not follow links, so a dangling
/// symlink counts as pre-existing and is never removed. The
/// existence check races a concurrent run: a file that appeared
/// after the check is refused by the exclusive create with
/// `AlreadyExists`, and that failure never removes either — this
/// run created nothing, and the file is the other run's. A connect
/// failure reports `NetworkError` before anything is created, and a
/// mid-stream write failure reports `InternalError` after this run
/// created the file.
fn download_one_file(
    download: impl FnOnce(&Path) -> Result<u64, GeectlError>,
    dest: &Path,
) -> Result<u64, GeectlError> {
    let existed_before = match dest.symlink_metadata() {
        Ok(_) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => true,
    };
    match download(dest) {
        Ok(size) => Ok(size),
        Err(e) => {
            if !existed_before && e.kind() != ErrorKind::AlreadyExists {
                let _ = std::fs::remove_file(dest);
            }
            Err(e)
        }
    }
}

/// Create the disk-space probe, removing a leftover probe from a
/// crashed run (same pid, recycled) once before giving up.
/// Best-effort: any failure returns `None` and the run proceeds.
fn create_probe_file(probe: &Path) -> Option<std::fs::File> {
    match std::fs::File::options()
        .write(true)
        .create_new(true)
        .open(probe)
    {
        Ok(file) => Some(file),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(probe);
            std::fs::File::options()
                .write(true)
                .create_new(true)
                .open(probe)
                .ok()
        }
        Err(_) => None,
    }
}

/// Fail fast when the output filesystem cannot hold a `--all` run:
/// allocate the run's total size in a probe file, then remove it. The
/// probe is best-effort — only running out of space fails the run;
/// any other allocation failure is ignored and the per-file writes
/// stay the backstop. A zero total needs no probe.
fn check_disk_space(dir: &Path, total_bytes: u64) -> Result<(), GeectlError> {
    if total_bytes == 0 {
        return Ok(());
    }
    let probe = dir.join(format!(".geectl-disk-probe-{}", std::process::id()));
    let Some(file) = create_probe_file(&probe) else {
        return Ok(());
    };
    let result = file.allocate(total_bytes);
    let _ = std::fs::remove_file(&probe);
    match result {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::StorageFull => Err(GeectlError::new(
            ErrorKind::InternalError,
            format!(
                "not enough disk space in {}: the download needs {total_bytes} bytes",
                dir.display()
            ),
        )),
        Err(_) => Ok(()),
    }
}

/// The `download-binary` confirmation line: the file, its size, and
/// where the bytes were written. The bytes themselves never ride the
/// text rendering. The name is server-provided, so it is stripped of
/// terminal controls before printing.
fn download_text(filename: &str, size: u64, output: &Path) -> String {
    format!(
        "downloaded {} ({size} bytes) to {}",
        strip_terminal_controls(filename),
        output.display()
    )
}

/// The `--all` download as text: one confirmation line per
/// downloaded file, then one line per file skipped as already
/// present, or that there is nothing to download.
fn download_all_text(
    downloaded: &[(String, u64, PathBuf)],
    skipped: &[(String, PathBuf)],
) -> String {
    let mut lines: Vec<String> = downloaded
        .iter()
        .map(|(filename, size, dest)| download_text(filename, *size, dest))
        .collect();
    lines.extend(
        skipped
            .iter()
            .map(|(filename, dest)| skipped_text(filename, dest)),
    );
    if lines.is_empty() {
        "no binaries".to_string()
    } else {
        lines.join("\n")
    }
}

/// The `--all` skip line: the file, and where the existing copy
/// sits. The existing file is never read or written. The name is
/// server-provided, so it is stripped of terminal controls before
/// printing.
fn skipped_text(filename: &str, output: &Path) -> String {
    format!(
        "skipped {} (already exists) to {}",
        strip_terminal_controls(filename),
        output.display()
    )
}

/// Write downloaded bytes to the `--output` path. The file is created
/// exclusively: an existing destination is refused instead of silently
/// overwritten, like `remote download`. A write
/// failure is an internal error naming the path (the download itself
/// already succeeded, so the failure is on our side, not the
/// server's). A failed write removes the partial file, so a retry
/// never hits AlreadyExists on the tool's own corrupt output.
fn write_downloaded(output: &PathBuf, bytes: &[u8]) -> Result<(), GeectlError> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|e| {
            let kind = if e.kind() == std::io::ErrorKind::AlreadyExists {
                ErrorKind::AlreadyExists
            } else {
                ErrorKind::InternalError
            };
            GeectlError::new(
                kind,
                format!("cannot create download file {}: {e}", output.display()),
            )
        })?;
    file.write_all(bytes).map_err(|e| {
        // The file was just created exclusively by this call, so
        // the partial is this run's: remove it, so a retry is not
        // refused by it.
        let _ = std::fs::remove_file(output);
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
/// [`auth::resolve_obs_for_url`]).
pub(crate) fn obs_endpoint() -> Result<(String, Vec<(String, String)>), GeectlError> {
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
pub(crate) fn results_payload(results: &BuildResults) -> Value {
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
/// Documented omission: a cell with statuses shows no
/// row for the cell-level code/state — the per-package rows carry the
/// codes, and the cell detail rides a note line. The cell-level
/// code/state stay in the JSON payload.
pub(crate) fn render_results_text(results: &BuildResults) -> String {
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
                cell_notes.push(format!(
                    "{} {}: {}",
                    cell.repository,
                    cell.arch,
                    strip_terminal_controls(details)
                ));
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
        Some(details) => format!("{code} ({})", strip_terminal_controls(details)),
        None => code.to_string(),
    }
}

/// The configured instances for a merged results read, in view
/// order: the configured (default) instance first, then the
/// `[instances]` table in alias order. An `[instances]` entry whose
/// URL matches the default instance's URL names the same instance,
/// so it is skipped, not read twice. The default instance is
/// labelled with its alias when the `[instances]` table names it,
/// else `default`. An entry whose URL fails to parse matches
/// nothing, so it is listed as its own endpoint and its read fails
/// for that instance alone instead of aborting the whole read.
fn merged_endpoints(
    base: &str,
    instances: &[(String, String)],
) -> Result<Vec<(String, String)>, GeectlError> {
    let label = alias_for_url(base, instances)?.unwrap_or("default");
    let mut endpoints = vec![(label.to_string(), base.to_string())];
    for (alias, url) in instances {
        let mut seen = false;
        for (_, seen_url) in &endpoints {
            if same_api_url(seen_url, url).unwrap_or(false) {
                seen = true;
                break;
            }
        }
        if seen {
            continue;
        }
        endpoints.push((alias.clone(), url.clone()));
    }
    Ok(endpoints)
}

/// Read the build results for `project` from every configured
/// instance, in view order: the configured (default) instance
/// first, then the `[instances]` table. One read per endpoint,
/// each with that instance's own credentials; a per-instance
/// failure is recorded for that instance and returned with the
/// others, never dropped. Every instance failing is an error:
/// there is no view to show — a single configured instance
/// reports its own failure unchanged. Shared by `results
/// --all-instances` and the build-link comment, so the two reads
/// never drift apart.
pub(crate) fn read_results_across_instances(
    http: &UreqClient,
    debug: &Debug,
    base: &str,
    instances: &[(String, String)],
    project: &str,
    query: &ResultsQuery,
) -> Result<Vec<InstanceBuildResults>, GeectlError> {
    let endpoints = merged_endpoints(base, instances)?;
    let refs: Vec<(&str, &str)> = endpoints
        .iter()
        .map(|(alias, url)| (alias.as_str(), url.as_str()))
        .collect();
    let outcomes = results_across_instances(&refs, project, query, |url, project, query| {
        let credentials = EnvObsAuth::credentials_for_url(url, base, instances)?;
        let client =
            ObsClient::with_base_and_credentials(http, url, credentials)?.with_debug(debug.clone());
        client.results(project, query)
    });
    if outcomes.iter().all(|outcome| outcome.outcome.is_err()) {
        if let [only] = outcomes.as_slice() {
            return Err(only.outcome.as_ref().unwrap_err().clone());
        }
        let kind = all_failed_kind(&outcomes);
        let detail = outcomes
            .iter()
            .map(|outcome| {
                format!(
                    "{}: {}",
                    outcome.alias,
                    outcome.outcome.as_ref().unwrap_err()
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        return Err(GeectlError::new(
            kind,
            format!("build results failed on every configured instance: {detail}"),
        ));
    }
    Ok(outcomes)
}

/// The merged build-results view for `results --all-instances`:
/// one read per configured instance, each with that instance's own
/// credentials, merged into one view. A per-instance failure is
/// recorded for that instance and rendered next to the other
/// instances' results, never dropped silently. Every instance
/// failing is an error: there is no view to show — a single
/// configured instance reports its own failure unchanged. A single
/// configured instance that succeeds still renders the merged
/// shape (one `instance:` section in text, one entry in the
/// payload's `instances` array), so the flag changes the view even
/// then; only the failure is identical to the plain read.
fn results_all_instances(
    http: &UreqClient,
    debug: &Debug,
    base: &str,
    instances: &[(String, String)],
    project: &str,
    query: &ResultsQuery,
    header: &str,
) -> Result<(Value, String), GeectlError> {
    let outcomes = read_results_across_instances(http, debug, base, instances, project, query)?;
    Ok((
        merged_results_payload(project, &outcomes),
        with_header(header, render_merged_results_text(&outcomes)),
    ))
}

/// The error kind for an every-instance failure: the kind most
/// instances failed with, ties broken by the smallest kind code —
/// deterministic regardless of instance order, so the same
/// configuration reports the same kind however the `[instances]`
/// table is ordered.
fn all_failed_kind(outcomes: &[InstanceBuildResults]) -> ErrorKind {
    let mut counts: BTreeMap<&str, (usize, ErrorKind)> = BTreeMap::new();
    for outcome in outcomes {
        let err = outcome.outcome.as_ref().unwrap_err();
        let entry = counts.entry(err.kind().code()).or_insert((0, err.kind()));
        entry.0 += 1;
    }
    let mut kinds: Vec<(usize, &str, ErrorKind)> = counts
        .into_iter()
        .map(|(code, (count, kind))| (count, code, kind))
        .collect();
    // Most failures first; ties broken by the smallest kind code.
    kinds.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    kinds
        .first()
        .map(|(_, _, kind)| *kind)
        // An every-instance failure always carries at least one
        // outcome: `merged_endpoints` pushes the default endpoint
        // first, so the outcomes list is never empty. The fallback
        // is unreachable; it keeps a violated invariant an error
        // instead of a panic.
        .unwrap_or(ErrorKind::InternalError)
}

/// The merged results as JSON: the project plus one object per
/// configured instance, in view order. A read instance carries its
/// state hash and matrix (the `results_payload` shape, plus its
/// alias and API URL); a failed instance carries its error instead
/// of a matrix, so the failure stays in the payload.
fn merged_results_payload(project: &str, outcomes: &[InstanceBuildResults]) -> Value {
    json!({
        "project": project,
        "instances": outcomes
            .iter()
            .map(|outcome| {
                let mut entry = match &outcome.outcome {
                    Ok(results) => results_payload(results),
                    Err(err) => json!({ "error": err.to_json()["error"].clone() }),
                };
                entry["alias"] = json!(outcome.alias);
                entry["api_url"] = json!(outcome.api_url);
                entry
            })
            .collect::<Vec<_>>(),
    })
}

/// The merged results as text: one section per configured instance,
/// in view order, each headed `instance: <alias> (<url>)` and
/// carrying that instance's own table, summary, and state hash —
/// or, for a failed instance, an `error:` line in place of its
/// table, so the failure is visible, never dropped.
fn render_merged_results_text(outcomes: &[InstanceBuildResults]) -> String {
    outcomes
        .iter()
        .map(|outcome| match &outcome.outcome {
            Ok(results) => format!(
                "instance: {} ({})\n{}",
                outcome.alias,
                outcome.api_url,
                render_results_text(results)
            ),
            Err(err) => format!(
                "instance: {} ({}): error: {err}",
                outcome.alias, outcome.api_url
            ),
        })
        .collect::<Vec<_>>()
        .join("\n")
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
    lines.extend(opt_line(
        "details",
        &job.details.as_deref().map(strip_terminal_controls),
    ));
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

/// The `buildconfig` result: the payload carries the coordinates and
/// the config text; the text is the config verbatim.
fn buildconfig_result(
    project: &str,
    repository: &str,
    alternative_project: Option<&str>,
    config: String,
) -> (Value, String) {
    let payload = json!({
        "project": project,
        "repository": repository,
        "alternative_project": alternative_project,
        "buildconfig": config,
    });
    (payload, config)
}

/// Buildinfo as JSON: the blocking error plus the dependency names
/// (`bdeps`, strings, so existing consumers keep reading names)
/// and the project each resolves from in the parallel
/// `bdep_projects` array, aligned by index (`null` when the server
/// did not say — a virtual provide or a not-installed flag).
fn buildinfo_payload(
    project: &str,
    package: &str,
    repository: &str,
    arch: &str,
    alternative_project: Option<&str>,
    info: &BuildInfo,
) -> Value {
    json!({
        "project": project,
        "package": package,
        "repository": repository,
        "arch": arch,
        "alternative_project": alternative_project,
        "error": info.error,
        "bdeps": info
            .bdeps
            .iter()
            .map(|dep| dep.name.clone())
            .collect::<Vec<_>>(),
        "bdep_projects": info
            .bdeps
            .iter()
            .map(|dep| dep.project.clone())
            .collect::<Vec<_>>(),
    })
}

/// Buildinfo as text: the blocking error, then the dependencies, each
/// with the project it resolves from when the server said one.
fn render_buildinfo_text(info: &BuildInfo) -> String {
    let mut lines = Vec::new();
    match &info.error {
        Some(error) => lines.push(format!("error: {}", strip_terminal_controls(error))),
        None => lines.push("no blocking error".to_string()),
    }
    if info.bdeps.is_empty() {
        lines.push("no dependencies".to_string());
    } else {
        lines.push("dependencies:".to_string());
        lines.extend(info.bdeps.iter().map(|dep| match &dep.project {
            Some(project) => format!("{} ({project})", dep.name),
            None => dep.name.clone(),
        }));
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
/// omission, not a gap.
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
    lines.extend(opt_line(
        "summary",
        &info.summary.as_deref().map(strip_terminal_controls),
    ));
    lines.extend(opt_line(
        "description",
        &info.description.as_deref().map(strip_terminal_controls),
    ));
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
/// documented omission, not a gap.
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
    use crate::obs::{BuildDep, BuildResult, InstanceBuildResults, PackageBuildStatus};
    use clap::FromArgMatches;

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
    fn mutation_result_carries_dry_run_and_plan_in_order() {
        let payload = json!({ "project": "P", "package": "k" });
        let (payload, text) = mutation_result(
            payload,
            "project: P package: k repository: standard arch: x86_64".to_string(),
            "dry-run: would rebuild package P/k (standard/x86_64)".to_string(),
            true,
            Some(vec![
                "[dry-run] would: rebuild package P/k (standard/x86_64)".to_string(),
            ]),
        );
        assert_eq!(payload["dry_run"], true);
        assert_eq!(
            payload["plan"][0],
            "[dry-run] would: rebuild package P/k (standard/x86_64)"
        );
        assert_eq!(
            text,
            "project: P package: k repository: standard arch: x86_64\ndry_run: true\ndry-run: would rebuild package P/k (standard/x86_64)\n[dry-run] would: rebuild package P/k (standard/x86_64)"
        );
    }

    #[test]
    fn build_control_result_states_each_action_live_and_dry() {
        let header = "project: P package: k repository: standard arch: x86_64";
        for (action, live, planned) in [
            (
                BuildControlAction::Rebuild,
                "rebuilt package P/k (standard/x86_64)",
                "dry-run: would rebuild package P/k (standard/x86_64)",
            ),
            (
                BuildControlAction::Restart,
                "restarted the build of P/k (standard/x86_64)",
                "dry-run: would restart the build of P/k (standard/x86_64)",
            ),
            (
                BuildControlAction::Abort,
                "aborted the build of P/k (standard/x86_64)",
                "dry-run: would abort the build of P/k (standard/x86_64)",
            ),
        ] {
            let (payload, text) =
                build_control_result(action, "P", "k", "standard", "x86_64", false, None);
            assert_eq!(payload["dry_run"], false);
            assert_eq!(text, format!("{header}\ndry_run: false\n{live}"));
            let (dry_payload, text) =
                build_control_result(action, "P", "k", "standard", "x86_64", true, None);
            assert_eq!(dry_payload["dry_run"], true);
            assert_eq!(text, format!("{header}\ndry_run: true\n{planned}"));
        }
    }

    #[test]
    fn clap_wipe_binaries_with_filters() {
        match parse_build(&[
            "wipe-binaries",
            "openSUSE:Factory",
            "foo",
            "--arch",
            "x86_64",
            "--repository",
            "standard",
            "--build-failed",
            "--broken",
        ]) {
            BuildCommand::WipeBinaries {
                project,
                package,
                arch,
                repository,
                all,
                build_disabled,
                build_failed,
                broken,
                unresolvable,
            } => {
                assert_eq!(project, "openSUSE:Factory");
                assert_eq!(package.as_deref(), Some("foo"));
                assert_eq!(arch.as_deref(), Some("x86_64"));
                assert_eq!(repository.as_deref(), Some("standard"));
                assert!(!all);
                assert!(!build_disabled);
                assert!(build_failed);
                assert!(broken);
                assert!(!unresolvable);
            }
            other => panic!("unexpected: {other:?}"),
        }
        // The project alone parses: the scope check is the
        // library's, so the leaf refuses an unscoped wipe at the
        // call, not at parse time.
        match parse_build(&["wipe-binaries", "openSUSE:Factory"]) {
            BuildCommand::WipeBinaries {
                package: None,
                all: false,
                ..
            } => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn wipe_binaries_result_carries_scope_in_both_formats() {
        let options = WipeBinariesOptions {
            package: Some("k".to_string()),
            arch: Some("x86_64".to_string()),
            repository: Some("standard".to_string()),
            codes: vec![WipeCode::Failed, WipeCode::Broken],
            ..WipeBinariesOptions::default()
        };
        let (payload, text) = wipe_binaries_result("P", &options, false, None);
        assert_eq!(payload["project"], "P");
        assert_eq!(payload["package"], "k");
        assert_eq!(payload["arch"], "x86_64");
        assert_eq!(payload["repository"], "standard");
        assert_eq!(payload["all"], false);
        assert_eq!(payload["codes"][0], "failed");
        assert_eq!(payload["codes"][1], "broken");
        assert_eq!(payload["dry_run"], false);
        assert_eq!(
            text,
            "project: P package: k repository: standard arch: x86_64\ndry_run: false\nwiped binaries in package k in project P (arch x86_64, repository standard, codes failed, broken)"
        );
        let (payload, text) = wipe_binaries_result(
            "P",
            &WipeBinariesOptions {
                all: true,
                ..WipeBinariesOptions::default()
            },
            true,
            Some(vec![
                "[dry-run] would: wipe binaries in project P (all)".to_string()
            ]),
        );
        assert_eq!(payload["package"], Value::Null);
        assert_eq!(payload["all"], true);
        assert_eq!(payload["dry_run"], true);
        assert_eq!(
            payload["plan"][0],
            "[dry-run] would: wipe binaries in project P (all)"
        );
        assert_eq!(
            text,
            "project: P\ndry_run: true\ndry-run: would wipe binaries in project P (all)\n[dry-run] would: wipe binaries in project P (all)"
        );
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
        // Design rule 1: one header line,
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
        // `build-log` has an arm in the helper for the unresolved
        // form, but `run_build` headers the resolved coordinates
        // (`build_log_header`), so a pasted URL never echoes as a
        // project. The run_build arm deliberately does not prepend
        // it to the log content (rule 4: the log is verbatim
        // content).
        assert_eq!(
            coordinates_header(&parse_build(&["build-log", "p", "k", "r", "a"])),
            "project: p package: k repository: r arch: a"
        );
        assert_eq!(
            build_log_header("p", "k", "r", "a"),
            "project: p package: k repository: r arch: a"
        );
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
            coordinates_header(&parse_build(&[
                "buildinfo",
                "p",
                "k",
                "r",
                "a",
                "--alternative-project",
                "openSUSE:Factory"
            ])),
            "project: p package: k repository: r arch: a alternative-project: openSUSE:Factory"
        );
        assert_eq!(
            coordinates_header(&parse_build(&["buildconfig", "p", "r"])),
            "project: p repository: r"
        );
        assert_eq!(
            coordinates_header(&parse_build(&[
                "buildconfig",
                "p",
                "r",
                "--alternative-project",
                "openSUSE:Factory"
            ])),
            "project: p repository: r alternative-project: openSUSE:Factory"
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
        assert_eq!(
            coordinates_header(&parse_build(&["wipe-binaries", "p"])),
            "project: p"
        );
        assert_eq!(
            coordinates_header(&parse_build(&[
                "wipe-binaries",
                "p",
                "k",
                "--repository",
                "r",
                "--arch",
                "a",
            ])),
            "project: p package: k repository: r arch: a"
        );
    }

    #[test]
    fn results_cell_level_code_is_payload_only_with_statuses() {
        // Documented omission: a cell with statuses
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

    /// The design doc's worked example for `build results`, pinned
    /// literally: the `project:` header, the `REPOSITORY ARCH
    /// PACKAGE CODE` table, and the alphabetical summary line
    /// (`1 failed, 3 succeeded`).
    #[test]
    fn results_text_matches_the_design_example() {
        let results = BuildResults {
            state: None,
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
                        code: "succeeded".to_string(),
                        details: None,
                    },
                    PackageBuildStatus {
                        package: "baz".to_string(),
                        code: "succeeded".to_string(),
                        details: None,
                    },
                    PackageBuildStatus {
                        package: "qux".to_string(),
                        code: "failed".to_string(),
                        details: None,
                    },
                ],
            }],
        };
        let text = with_header("project: openSUSE:Factory", render_results_text(&results));
        assert_eq!(
            text,
            "project: openSUSE:Factory\nREPOSITORY  ARCH    PACKAGE  CODE\nstandard    x86_64  foo      succeeded\nstandard    x86_64  bar      succeeded\nstandard    x86_64  baz      succeeded\nstandard    x86_64  qux      failed\n1 failed, 3 succeeded"
        );
    }

    /// Two faked instances for the merged view: the configured
    /// instance (`factory`) and a named one (`local`), each with
    /// its own state hash and matrix.
    fn merged_outcomes() -> Vec<InstanceBuildResults> {
        vec![
            InstanceBuildResults {
                alias: "factory".to_string(),
                api_url: "https://api.example.org".to_string(),
                outcome: Ok(results_fixture()),
            },
            InstanceBuildResults {
                alias: "local".to_string(),
                api_url: "http://127.0.0.1:3001".to_string(),
                outcome: Ok(BuildResults {
                    state: Some("def456".to_string()),
                    results: vec![BuildResult {
                        project: "openSUSE:Factory".to_string(),
                        repository: "standard".to_string(),
                        arch: "x86_64".to_string(),
                        code: "published".to_string(),
                        state: "published".to_string(),
                        dirty: false,
                        details: None,
                        statuses: vec![PackageBuildStatus {
                            package: "foo".to_string(),
                            code: "failed".to_string(),
                            details: Some("missing foo".to_string()),
                        }],
                    }],
                }),
            },
        ]
    }

    /// The merged view, pinned literally: one `instance:` section
    /// per configured instance, each with its own table, summary,
    /// and state hash — the shape the design doc's `build results`
    /// example cites.
    #[test]
    fn merged_results_text_sections_per_instance() {
        let text = with_header(
            "project: openSUSE:Factory package: foo",
            render_merged_results_text(&merged_outcomes()),
        );
        assert_eq!(
            text,
            "project: openSUSE:Factory package: foo\ninstance: factory (https://api.example.org)\nREPOSITORY  ARCH    PACKAGE  CODE\nstandard    x86_64  foo      succeeded\nstandard    x86_64  bar      failed (missing foo)\n1 failed, 1 succeeded\nstate: abc123\ninstance: local (http://127.0.0.1:3001)\nREPOSITORY  ARCH    PACKAGE  CODE\nstandard    x86_64  foo      failed (missing foo)\n1 failed\nstate: def456"
        );
    }

    #[test]
    fn merged_results_text_surfaces_a_failed_instance() {
        let mut outcomes = merged_outcomes();
        outcomes[1].outcome = Err(GeectlError::new(
            ErrorKind::NetworkError,
            "connection refused",
        ));
        let text = render_merged_results_text(&outcomes);
        assert!(text.contains("instance: factory (https://api.example.org)"));
        assert!(text.contains("1 failed, 1 succeeded"));
        assert!(text.contains(
            "instance: local (http://127.0.0.1:3001): error: network_error: a network error occurred (connection refused)"
        ));
    }

    #[test]
    fn merged_results_payload_shape() {
        let mut outcomes = merged_outcomes();
        outcomes[1].outcome = Err(GeectlError::new(ErrorKind::NotFound, "no such project"));
        let payload = merged_results_payload("openSUSE:Factory", &outcomes);
        assert_eq!(payload["project"], "openSUSE:Factory");
        assert_eq!(payload["instances"][0]["alias"], "factory");
        assert_eq!(
            payload["instances"][0]["api_url"],
            "https://api.example.org"
        );
        assert_eq!(payload["instances"][0]["state"], "abc123");
        assert_eq!(
            payload["instances"][0]["results"][0]["statuses"][1]["code"],
            "failed"
        );
        assert_eq!(payload["instances"][1]["alias"], "local");
        assert_eq!(payload["instances"][1]["error"]["code"], "not_found");
        assert_eq!(
            payload["instances"][1]["error"]["context"],
            "no such project"
        );
    }

    #[test]
    fn merged_endpoints_puts_the_default_first_and_dedupes() {
        let instances = vec![
            ("factory".to_string(), "https://api.example.org".to_string()),
            ("local".to_string(), "http://127.0.0.1:3001".to_string()),
        ];
        // The default URL matches the `factory` entry: it labels
        // the default instance and the entry itself is skipped, not
        // read twice.
        assert_eq!(
            merged_endpoints("https://api.example.org", &instances).unwrap(),
            vec![
                ("factory".to_string(), "https://api.example.org".to_string()),
                ("local".to_string(), "http://127.0.0.1:3001".to_string()),
            ]
        );
        // A default URL the table does not name is labelled
        // `default` and reads first, then the table in alias order.
        assert_eq!(
            merged_endpoints("https://api.other.example", &instances).unwrap(),
            vec![
                (
                    "default".to_string(),
                    "https://api.other.example".to_string()
                ),
                ("factory".to_string(), "https://api.example.org".to_string()),
                ("local".to_string(), "http://127.0.0.1:3001".to_string()),
            ]
        );
    }

    #[test]
    fn merged_endpoints_keeps_a_malformed_entry_as_its_own_endpoint() {
        // One malformed `[instances]` URL never aborts the merged
        // read: it matches nothing, so it is listed as its own
        // endpoint, and its read fails for that instance alone.
        let instances = vec![
            ("broken".to_string(), "notaurl".to_string()),
            ("local".to_string(), "http://127.0.0.1:3001".to_string()),
        ];
        assert_eq!(
            merged_endpoints("https://api.example.org", &instances).unwrap(),
            vec![
                ("default".to_string(), "https://api.example.org".to_string()),
                ("broken".to_string(), "notaurl".to_string()),
                ("local".to_string(), "http://127.0.0.1:3001".to_string()),
            ]
        );
    }

    #[test]
    fn all_failed_kind_is_deterministic_regardless_of_instance_order() {
        // The same failures in the opposite order report the same
        // kind: the kind is a property of the failure set, not of
        // the `[instances]` order.
        let failed = |kind: ErrorKind| InstanceBuildResults {
            alias: "x".to_string(),
            api_url: "https://api.example.org".to_string(),
            outcome: Err(GeectlError::new(kind, "failed")),
        };
        let forward = vec![failed(ErrorKind::NotFound), failed(ErrorKind::NetworkError)];
        let backward = vec![failed(ErrorKind::NetworkError), failed(ErrorKind::NotFound)];
        assert_eq!(all_failed_kind(&forward), all_failed_kind(&backward));
        // A tie breaks by the smallest kind code.
        assert_eq!(all_failed_kind(&forward), ErrorKind::NetworkError);
        // A majority wins regardless of order.
        let majority_first = vec![
            failed(ErrorKind::NotFound),
            failed(ErrorKind::NotFound),
            failed(ErrorKind::NetworkError),
        ];
        let majority_last = vec![
            failed(ErrorKind::NetworkError),
            failed(ErrorKind::NotFound),
            failed(ErrorKind::NotFound),
        ];
        assert_eq!(all_failed_kind(&majority_first), ErrorKind::NotFound);
        assert_eq!(all_failed_kind(&majority_last), ErrorKind::NotFound);
    }

    #[test]
    fn download_text_names_file_size_and_path() {
        assert_eq!(
            download_text("foo-1.0-1.x86_64.rpm", 1234, &PathBuf::from("/tmp/foo.rpm")),
            "downloaded foo-1.0-1.x86_64.rpm (1234 bytes) to /tmp/foo.rpm"
        );
    }

    #[test]
    fn download_binary_parses_all_and_single_forms() {
        match parse_build(&[
            "download-binary",
            "p",
            "k",
            "r",
            "a",
            "--all",
            "--output",
            "/tmp/dir",
        ]) {
            BuildCommand::DownloadBinary {
                filename,
                output,
                all,
                ..
            } => {
                assert_eq!(filename, None);
                assert_eq!(output, PathBuf::from("/tmp/dir"));
                assert!(all);
            }
            _ => panic!("wrong variant"),
        }
        match parse_build(&[
            "download-binary",
            "p",
            "k",
            "r",
            "a",
            "foo.rpm",
            "--output",
            "/tmp/foo.rpm",
        ]) {
            BuildCommand::DownloadBinary {
                filename,
                output,
                all,
                ..
            } => {
                assert_eq!(filename.as_deref(), Some("foo.rpm"));
                assert_eq!(output, PathBuf::from("/tmp/foo.rpm"));
                assert!(!all);
            }
            _ => panic!("wrong variant"),
        }
        // --all conflicts with the filename.
        assert!(build_command()
            .try_get_matches_from([
                "build",
                "download-binary",
                "p",
                "k",
                "r",
                "a",
                "foo.rpm",
                "--all",
                "--output",
                "/tmp/dir",
            ])
            .is_err());
        // Without --all the filename is required.
        assert!(build_command()
            .try_get_matches_from([
                "build",
                "download-binary",
                "p",
                "k",
                "r",
                "a",
                "--output",
                "/tmp/dir",
            ])
            .is_err());
    }

    #[test]
    fn download_all_text_names_each_file_or_nothing() {
        assert_eq!(download_all_text(&[], &[]), "no binaries");
        assert_eq!(
            download_all_text(
                &[
                    ("a.rpm".to_string(), 10, PathBuf::from("/tmp/d/a.rpm")),
                    ("b.rpm".to_string(), 20, PathBuf::from("/tmp/d/b.rpm")),
                ],
                &[]
            ),
            "downloaded a.rpm (10 bytes) to /tmp/d/a.rpm\n".to_string()
                + "downloaded b.rpm (20 bytes) to /tmp/d/b.rpm"
        );
        assert_eq!(
            download_all_text(
                &[],
                &[
                    ("a.rpm".to_string(), PathBuf::from("/tmp/d/a.rpm")),
                    ("b.rpm".to_string(), PathBuf::from("/tmp/d/b.rpm")),
                ]
            ),
            "skipped a.rpm (already exists) to /tmp/d/a.rpm\n".to_string()
                + "skipped b.rpm (already exists) to /tmp/d/b.rpm"
        );
        assert_eq!(
            download_all_text(
                &[("a.rpm".to_string(), 10, PathBuf::from("/tmp/d/a.rpm"))],
                &[("b.rpm".to_string(), PathBuf::from("/tmp/d/b.rpm"))]
            ),
            "downloaded a.rpm (10 bytes) to /tmp/d/a.rpm\n".to_string()
                + "skipped b.rpm (already exists) to /tmp/d/b.rpm"
        );
    }

    #[test]
    fn download_texts_strip_terminal_controls_from_server_names() {
        let text = download_all_text(
            &[(
                "a\u{202e}rpm".to_string(),
                10,
                PathBuf::from("/tmp/d/a.rpm"),
            )],
            &[("b\x1b[31mrpm".to_string(), PathBuf::from("/tmp/d/b.rpm"))],
        );
        assert!(!text.contains('\u{202e}'));
        assert!(!text.contains('\x1b'));
        assert!(text.contains("downloaded arpm (10 bytes)"));
        assert!(text.contains("skipped b[31mrpm (already exists)"));
    }

    #[test]
    fn download_name_safety_rejects_separators_and_reserved_names() {
        assert!(is_safe_download_name("foo-1.0-1.x86_64.rpm"));
        assert!(is_safe_download_name("_buildenv"));
        assert!(!is_safe_download_name("../../victim"));
        assert!(!is_safe_download_name("a/b.rpm"));
        assert!(!is_safe_download_name("a\\b.rpm"));
        assert!(!is_safe_download_name(".hidden"));
        assert!(!is_safe_download_name("_log"));
        assert!(!is_safe_download_name("x"));
        assert!(!is_safe_download_name("C:"));
        assert!(!is_safe_download_name("a:b.rpm"));
        assert!(!is_safe_download_name("a\0b.rpm"));
    }

    #[test]
    fn validate_download_names_fails_fast_naming_the_bad_name() {
        let files = vec![
            BinaryFile {
                name: "good-1.0-1.x86_64.rpm".to_string(),
                size: Some(10),
                mtime: None,
            },
            BinaryFile {
                name: "../bad.rpm".to_string(),
                size: Some(10),
                mtime: None,
            },
        ];
        let err = validate_download_names(&files).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.context.contains("../bad.rpm"));
        assert!(validate_download_names(&files[..1]).is_ok());
    }

    #[test]
    fn total_download_size_sums_known_sizes_and_ignores_unknown() {
        let files = vec![
            BinaryFile {
                name: "a.rpm".to_string(),
                size: Some(10),
                mtime: None,
            },
            BinaryFile {
                name: "b.rpm".to_string(),
                size: None,
                mtime: None,
            },
            BinaryFile {
                name: "c.rpm".to_string(),
                size: Some(20),
                mtime: None,
            },
        ];
        assert_eq!(total_download_size(&files).unwrap(), 30);
        let overflowing = vec![
            BinaryFile {
                name: "a.rpm".to_string(),
                size: Some(u64::MAX),
                mtime: None,
            },
            BinaryFile {
                name: "b.rpm".to_string(),
                size: Some(1),
                mtime: None,
            },
        ];
        assert!(total_download_size(&overflowing).is_err());
    }

    #[test]
    fn contained_dest_keeps_the_caller_spelling_and_confines() {
        let dir = std::env::temp_dir().join(format!(
            "geectl-cli-build-test-{}-contained",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let canonical = std::fs::canonicalize(&dir).unwrap();
        assert_eq!(
            contained_dest(&dir, &canonical, "foo.rpm").unwrap(),
            dir.join("foo.rpm")
        );
        assert!(contained_dest(&dir, &canonical, "../foo.rpm").is_err());
        assert!(contained_dest(&dir, &canonical, "a\\b.rpm").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn partition_downloads_skips_what_exists_and_keeps_the_rest() {
        let dir = std::env::temp_dir().join(format!(
            "geectl-cli-build-test-{}-partition",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let canonical = std::fs::canonicalize(&dir).unwrap();
        std::fs::write(dir.join("_buildenv"), b"planted").unwrap();
        let files = vec![
            BinaryFile {
                name: "_buildenv".to_string(),
                size: Some(9),
                mtime: None,
            },
            BinaryFile {
                name: "a-1.0-1.x86_64.rpm".to_string(),
                size: Some(10),
                mtime: None,
            },
        ];
        let (to_download, skipped) = partition_downloads(&files, &dir, &canonical).unwrap();
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].0, "_buildenv");
        assert_eq!(to_download.len(), 1);
        assert_eq!(to_download[0].0.name, "a-1.0-1.x86_64.rpm");
        // A dangling symlink counts as existing: it is skipped,
        // never followed and never deleted.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/nonexistent-target-xyz", dir.join("link.rpm")).unwrap();
            let files = vec![BinaryFile {
                name: "link.rpm".to_string(),
                size: Some(10),
                mtime: None,
            }];
            let (to_download, skipped) = partition_downloads(&files, &dir, &canonical).unwrap();
            assert!(to_download.is_empty());
            assert_eq!(skipped.len(), 1);
            assert_eq!(skipped[0].0, "link.rpm");
        }
        // An unsafe name fails the partition with nothing split.
        let bad = vec![BinaryFile {
            name: "../bad.rpm".to_string(),
            size: Some(10),
            mtime: None,
        }];
        assert!(partition_downloads(&bad, &dir, &canonical).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recheck_skipped_returns_what_vanished_since_partition() {
        let dir = std::env::temp_dir().join(format!(
            "geectl-cli-build-test-{}-recheck",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let canonical = std::fs::canonicalize(&dir).unwrap();
        std::fs::write(dir.join("kept.rpm"), b"kept").unwrap();
        std::fs::write(dir.join("gone.rpm"), b"gone").unwrap();
        let files = vec![
            BinaryFile {
                name: "kept.rpm".to_string(),
                size: Some(4),
                mtime: None,
            },
            BinaryFile {
                name: "gone.rpm".to_string(),
                size: Some(4),
                mtime: None,
            },
        ];
        let (_, mut skipped) = partition_downloads(&files, &dir, &canonical).unwrap();
        assert_eq!(skipped.len(), 2);
        std::fs::remove_file(dir.join("gone.rpm")).unwrap();
        let missing = recheck_skipped(&files, &mut skipped);
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].0, "kept.rpm");
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].0.name, "gone.rpm");
        assert_eq!(missing[0].1, dir.join("gone.rpm"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn download_one_file_removes_only_what_this_run_created() {
        let dir = std::env::temp_dir().join(format!(
            "geectl-cli-build-test-{}-one-file",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // A successful download keeps its file.
        let dest = dir.join("ok.rpm");
        let size = download_one_file(
            |d| {
                std::fs::write(d, b"bytes").unwrap();
                Ok(5)
            },
            &dest,
        )
        .unwrap();
        assert_eq!(size, 5);
        assert!(dest.exists());
        // A failure after this run created the file removes it.
        let dest = dir.join("partial.rpm");
        let err = download_one_file(
            |d| {
                std::fs::write(d, b"part").unwrap();
                Err(GeectlError::new(ErrorKind::NetworkError, "down"))
            },
            &dest,
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NetworkError);
        assert!(!dest.exists());
        // A failure with a pre-existing file keeps it.
        let dest = dir.join("kept.rpm");
        std::fs::write(&dest, b"good").unwrap();
        let err = download_one_file(
            |_| Err(GeectlError::new(ErrorKind::NetworkError, "down")),
            &dest,
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NetworkError);
        assert_eq!(std::fs::read(&dest).unwrap(), b"good");
        // A failure with no file at all is an error, not a panic.
        let dest = dir.join("absent.rpm");
        assert!(
            download_one_file(|_| Err(GeectlError::new(ErrorKind::Timeout, "slow")), &dest)
                .is_err()
        );
        assert!(!dest.exists());
        // A failure with a dangling pre-planted symlink keeps the
        // link: it pre-existed, even though its target never did.
        #[cfg(unix)]
        {
            let dest = dir.join("dangling.rpm");
            std::os::unix::fs::symlink("/nonexistent-target-xyz", &dest).unwrap();
            let err = download_one_file(
                |_| Err(GeectlError::new(ErrorKind::NetworkError, "down")),
                &dest,
            )
            .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NetworkError);
            assert!(dest.symlink_metadata().is_ok());
        }
        // An AlreadyExists failure never removes: the exclusive
        // create refused, so this run created nothing — a file a
        // concurrent run created after the existence check is the
        // other run's, and deleting it would turn its win into a
        // false skip.
        let dest = dir.join("winner.rpm");
        let err = download_one_file(
            |d| {
                std::fs::write(d, b"winner").unwrap();
                Err(GeectlError::new(ErrorKind::AlreadyExists, "exists"))
            },
            &dest,
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&dest).unwrap(), b"winner");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn download_all_text_keeps_u64_sizes() {
        assert_eq!(
            download_all_text(
                &[(
                    "big.rpm".to_string(),
                    5_000_000_000,
                    PathBuf::from("/tmp/d/big.rpm")
                )],
                &[]
            ),
            "downloaded big.rpm (5000000000 bytes) to /tmp/d/big.rpm"
        );
    }

    /// The agreement test for the single-object renderers: every id,
    /// name, and revision the JSON payload carries appears in the
    /// text rendering.
    #[test]
    fn single_object_texts_carry_every_field() {
        let job = Some(JobStatus {
            code: "building".to_string(),
            result: Some("succeeded".to_string()),
            details: Some("compiling foo".to_string()),
            starttime: Some(1_700_000_000),
            endtime: Some(1_700_000_120),
            lastduration: Some(42),
            workerid: Some("worker:1".to_string()),
            hostarch: Some("aarch64".to_string()),
            uri: Some("https://api.example/build/jobs/1".to_string()),
            arch: Some("x86_64".to_string()),
            job: Some("job-1".to_string()),
            jobid: Some("jobid-1".to_string()),
            attempt: Some(1),
        });
        let text = render_job_status_text(&job);
        // The expected lines come from the payload itself, not from
        // literals repeated from the rendering code, so a payload
        // field the text drops fails here. Host arch and arch differ,
        // so a label swap fails too.
        let job_payload = job_status_payload("p", "k", "r", "a", &job)["job"].clone();
        for field in [
            "code", "result", "details", "workerid", "hostarch", "uri", "arch", "job", "jobid",
        ] {
            let value = job_payload[field].as_str().expect("field is a string");
            assert!(
                text.contains(&format!("{field}: {value}")),
                "job status text misses {field}: {value}:\n{text}"
            );
        }
        for field in ["starttime", "endtime", "lastduration", "attempt"] {
            let value = job_payload[field].as_u64().expect("field is a number");
            assert!(
                text.contains(&format!("{field}: {value}")),
                "job status text misses {field}: {value}:\n{text}"
            );
        }

        let info = BuildInfo {
            error: Some("unresolvable: DOES-NOT-EXIST".to_string()),
            bdeps: vec![
                BuildDep {
                    name: "gcc".to_string(),
                    project: Some("openSUSE:Factory".to_string()),
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
            urls: std::collections::HashMap::new(),
            skipped_paths: 0,
        };
        let text = render_buildinfo_text(&info);
        let info_payload = buildinfo_payload("p", "k", "r", "a", None, &info);
        assert!(
            text.contains(&format!(
                "error: {}",
                info_payload["error"].as_str().expect("error is a string")
            )),
            "buildinfo text misses the error:\n{text}"
        );
        let bdeps = info_payload["bdeps"].as_array().expect("bdeps is an array");
        let projects = info_payload["bdep_projects"]
            .as_array()
            .expect("bdep_projects is an array");
        assert_eq!(
            bdeps.len(),
            projects.len(),
            "bdeps and bdep_projects stay aligned by index"
        );
        for (dep, project) in bdeps.iter().zip(projects) {
            let name = dep.as_str().expect("dep name is a string");
            assert!(
                text.contains(name),
                "buildinfo text misses dep {name}:\n{text}"
            );
            if let Some(project) = project.as_str() {
                assert!(
                    text.contains(project),
                    "buildinfo text misses dep project {project}:\n{text}"
                );
            }
        }

        let entries = vec![BuildHistoryEntry {
            rev: "1".to_string(),
            srcmd5: "deadbeef".to_string(),
            versrel: "1.0-1".to_string(),
            bcnt: 2,
            time: 1_700_000_000,
            duration: Some(120),
        }];
        assert_eq!(
            render_build_history_text(&entries),
            "REV  SRCMD5    VERSREL  BCNT  TIME        DURATION\n1    deadbeef  1.0-1    2     1700000000  120"
        );
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
    fn results_text_strips_terminal_controls_from_details() {
        // Cell-level detail text (in the code cell and in the note
        // line) and package-level detail text reach the terminal
        // through the results render: escape sequences, bells, and
        // bidirectional marks are stripped.
        let mut results = results_fixture();
        results.results[0].details = Some("repo\u{1b}[2J broken".to_string());
        results.results[0].statuses[0].details = Some("miss\u{7}ing foo\u{202e}".to_string());
        let text = render_results_text(&results);
        assert!(!text.contains('\u{1b}'), "text: {text}");
        assert!(!text.contains('\u{7}'), "text: {text}");
        assert!(!text.contains('\u{202e}'), "text: {text}");
        assert!(
            text.contains("standard x86_64: repo[2J broken"),
            "text: {text}"
        );
        assert!(text.contains("failed (missing foo)"), "text: {text}");
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
    fn job_status_text_strips_terminal_controls_from_details() {
        // Job detail text is server-authored free text and reaches
        // the terminal through the job status render.
        let job = Some(JobStatus {
            code: "building".to_string(),
            result: None,
            details: Some("build\u{1b}[2Jing\u{7}".to_string()),
            starttime: None,
            endtime: None,
            lastduration: None,
            workerid: None,
            hostarch: None,
            uri: None,
            arch: None,
            job: None,
            jobid: None,
            attempt: None,
        });
        let text = render_job_status_text(&job);
        assert!(!text.contains('\u{1b}'), "text: {text}");
        assert!(!text.contains('\u{7}'), "text: {text}");
        assert!(text.contains("details: build[2Jing"), "text: {text}");
    }

    #[test]
    fn buildinfo_error_and_clean() {
        let blocked = BuildInfo {
            error: Some("unresolvable: DOES-NOT-EXIST".to_string()),
            bdeps: vec![],
            preinstall_image: None,
            urls: std::collections::HashMap::new(),
            skipped_paths: 0,
        };
        assert!(render_buildinfo_text(&blocked).contains("error: unresolvable: DOES-NOT-EXIST"));
        assert_eq!(
            buildinfo_payload("p", "k", "r", "a", None, &blocked)["error"],
            "unresolvable: DOES-NOT-EXIST"
        );
        let clean = BuildInfo {
            error: None,
            bdeps: vec![
                BuildDep {
                    name: "gcc".to_string(),
                    project: Some("openSUSE:Factory".to_string()),
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
            urls: std::collections::HashMap::new(),
            skipped_paths: 0,
        };
        let text = render_buildinfo_text(&clean);
        assert!(text.contains("no blocking error"));
        assert!(text.contains("gcc (openSUSE:Factory)"));
        assert!(text.contains("make"));
        // A dep without a project renders bare, and its payload
        // project is null: the server did not resolve it to a binary.
        // `bdeps` stays strings; the projects ride the parallel
        // `bdep_projects` array, aligned by index.
        let payload = buildinfo_payload("p", "k", "r", "a", None, &clean);
        assert_eq!(payload["bdeps"][0], "gcc");
        assert_eq!(payload["bdeps"][1], "make");
        assert_eq!(payload["bdep_projects"][0], "openSUSE:Factory");
        assert!(payload["bdep_projects"][1].is_null());
    }

    #[test]
    fn buildinfo_renders_error_without_terminal_controls() {
        // The error is server text, and the render is the terminal
        // boundary: escape sequences, carriage returns, bells, and
        // bidirectional marks are stripped, while the error's own
        // newlines survive.
        let blocked = BuildInfo {
            error: Some("bad\u{1b}[2Jline\rwith\u{7}bell\nand\u{202e}bidi".to_string()),
            bdeps: vec![],
            preinstall_image: None,
            urls: std::collections::HashMap::new(),
            skipped_paths: 0,
        };
        assert_eq!(
            render_buildinfo_text(&blocked),
            "error: bad[2Jlinewithbell\nandbidi\nno dependencies"
        );
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
        // Documented omission: the worker and
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
        assert!(err.context.contains("cannot create download file"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_downloaded_refuses_an_existing_destination() {
        // The destination is created exclusively: a second write to
        // the same path is refused and the existing file is left
        // intact, never silently overwritten.
        let dir = std::env::temp_dir().join(format!(
            "geectl-cli-build-test-refuse-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("foo.rpm");
        write_downloaded(&path, b"bytes").unwrap();
        let err = write_downloaded(&path, b"bytes").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
        assert!(err.context.contains("cannot create download file"));
        assert_eq!(std::fs::read(&path).unwrap(), b"bytes");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn check_disk_space_probes_and_leaves_nothing_behind() {
        let dir =
            std::env::temp_dir().join(format!("geectl-cli-build-test-{}-disk", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(check_disk_space(&dir, 0).is_ok());
        assert!(check_disk_space(&dir, 1024).is_ok());
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            0,
            "the probe file is removed after the check"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn check_disk_space_removes_a_stale_probe_and_still_checks() {
        let dir = std::env::temp_dir().join(format!(
            "geectl-cli-build-test-{}-disk-stale",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let probe = dir.join(format!(".geectl-disk-probe-{}", std::process::id()));
        std::fs::write(&probe, b"leftover").unwrap();
        assert!(check_disk_space(&dir, 1024).is_ok());
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            0,
            "the stale probe is removed and the new probe after the check"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn check_disk_space_ignores_an_unusable_directory() {
        let dir = std::env::temp_dir().join(format!(
            "geectl-cli-build-test-{}-disk-ro",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        }
        assert!(check_disk_space(&dir, 1024).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
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
        // Documented omission: a resolved package's
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
    fn fileinfo_text_strips_terminal_controls_from_summary_and_description() {
        // The summary and description are RPM author text and reach
        // the terminal through the fileinfo render: escape sequences,
        // carriage returns, bells, and bidirectional marks are
        // stripped, while newlines survive.
        let mut info = fileinfo_fixture();
        info.summary = Some("A\u{1b}[2J foo".to_string());
        info.description = Some("Does\rfoo\u{7}.\nMore\u{202e} text".to_string());
        let text = render_fileinfo_text(&info);
        assert!(!text.contains('\u{1b}'), "text: {text}");
        assert!(!text.contains('\r'), "text: {text}");
        assert!(!text.contains('\u{7}'), "text: {text}");
        assert!(!text.contains('\u{202e}'), "text: {text}");
        assert!(text.contains("summary: A[2J foo"), "text: {text}");
        assert!(
            text.contains("description: Doesfoo.\nMore text"),
            "text: {text}"
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
            rebuild: None,
            block: None,
            linkedbuild: None,
            downloads: vec![],
            release_targets: vec![],
            hostsystem: vec![],
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
                all_instances,
            } => {
                assert_eq!(project, "openSUSE:Factory");
                assert_eq!(query.package.as_deref(), Some("foo"));
                assert_eq!(query.code.as_deref(), Some("failed"));
                assert_eq!(oldstate.as_deref(), Some("abc123"));
                assert!(!all_instances);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn clap_results_all_instances() {
        match parse_build(&["results", "openSUSE:Factory", "--all-instances"]) {
            BuildCommand::Results {
                project,
                all_instances,
                ..
            } => {
                assert_eq!(project, "openSUSE:Factory");
                assert!(all_instances);
            }
            other => panic!("unexpected: {other:?}"),
        }
        // `--oldstate` is per instance, so the merged read refuses
        // it as a parse error, not a silent ignore.
        let matches = build_command().try_get_matches_from([
            "build",
            "results",
            "openSUSE:Factory",
            "--all-instances",
            "--oldstate",
            "abc123",
        ]);
        assert!(matches.is_err());
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
    fn clap_build_log_multibuild_and_strip_time() {
        match parse_build(&[
            "build-log",
            "p",
            "k",
            "r",
            "a",
            "--multibuild",
            "flavor1",
            "--strip-time",
        ]) {
            BuildCommand::BuildLog {
                project,
                package,
                repository,
                arch,
                multibuild,
                strip_time,
                ..
            } => {
                assert_eq!(project, "p");
                assert_eq!(package.as_deref(), Some("k"));
                assert_eq!(repository.as_deref(), Some("r"));
                assert_eq!(arch.as_deref(), Some("a"));
                assert_eq!(multibuild.as_deref(), Some("flavor1"));
                assert!(strip_time);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn clap_build_log_url_form_parses_as_one_positional() {
        match parse_build(&["build-log", "https://api.opensuse.org/build/p/r/a/k/_log"]) {
            BuildCommand::BuildLog {
                project,
                package,
                repository,
                arch,
                ..
            } => {
                assert_eq!(project, "https://api.opensuse.org/build/p/r/a/k/_log");
                assert_eq!(package, None);
                assert_eq!(repository, None);
                assert_eq!(arch, None);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn resolve_build_log_coordinates_four_positionals() {
        assert_eq!(
            resolve_build_log_coordinates(
                "p",
                Some("k"),
                Some("r"),
                Some("a"),
                None,
                "https://api.opensuse.org"
            )
            .unwrap(),
            (
                "p".to_string(),
                "k".to_string(),
                "r".to_string(),
                "a".to_string()
            )
        );
    }

    #[test]
    fn resolve_build_log_coordinates_url_forms() {
        assert_eq!(
            resolve_build_log_coordinates(
                "https://api.opensuse.org/build/p/r/a/k/_log",
                None,
                None,
                None,
                None,
                "https://api.opensuse.org"
            )
            .unwrap(),
            (
                "p".to_string(),
                "k".to_string(),
                "r".to_string(),
                "a".to_string()
            )
        );
        assert_eq!(
            resolve_build_log_coordinates(
                "https://build.opensuse.org/package/live_build_log/p/k/r/a",
                None,
                None,
                None,
                None,
                "https://api.opensuse.org"
            )
            .unwrap(),
            (
                "p".to_string(),
                "k".to_string(),
                "r".to_string(),
                "a".to_string()
            )
        );
    }

    #[test]
    fn resolve_build_log_coordinates_multibuild_appends_the_flavor() {
        assert_eq!(
            resolve_build_log_coordinates(
                "p",
                Some("k"),
                Some("r"),
                Some("a"),
                Some("flavor1"),
                "https://api.opensuse.org"
            )
            .unwrap()
            .1,
            "k:flavor1"
        );
        // A URL form with a flavored package plus --multibuild is
        // the same append, on the resolved package.
        assert_eq!(
            resolve_build_log_coordinates(
                "https://api.opensuse.org/build/p/r/a/k/_log",
                None,
                None,
                None,
                Some("flavor1"),
                "https://api.opensuse.org"
            )
            .unwrap()
            .1,
            "k:flavor1"
        );
    }

    #[test]
    fn resolve_build_log_coordinates_fails_closed() {
        let api = "https://api.opensuse.org";
        // A half-given coordinate set names neither form.
        for (project, package, repository, arch) in [
            ("p", Some("k"), None, None),
            ("p", Some("k"), Some("r"), None),
            ("p", None, Some("r"), Some("a")),
        ] {
            let err = resolve_build_log_coordinates(project, package, repository, arch, None, api)
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
            assert!(
                err.context.contains("all four coordinates"),
                "{}",
                err.context
            );
        }
        // A URL with other coordinates is not a coordinate set.
        let err = resolve_build_log_coordinates(
            "https://api.opensuse.org/build/p/r/a/k/_log",
            Some("k"),
            Some("r"),
            Some("a"),
            None,
            api,
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(
            err.context.contains("no other coordinates"),
            "{}",
            err.context
        );
        // A single positional that is not a URL.
        let err = resolve_build_log_coordinates("p", None, None, None, None, api).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        // --multibuild on a package that already carries a flavor.
        let err = resolve_build_log_coordinates(
            "p",
            Some("k:flavor1"),
            Some("r"),
            Some("a"),
            Some("flavor2"),
            api,
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(
            err.context.contains("already carries a flavor"),
            "{}",
            err.context
        );
        // --multibuild with no flavor, or a flavor that is not a
        // name fragment.
        for flavor in ["", "a/b", "a:b"] {
            let err = resolve_build_log_coordinates(
                "p",
                Some("k"),
                Some("r"),
                Some("a"),
                Some(flavor),
                api,
            )
            .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput, "{flavor:?}");
        }
    }

    #[test]
    fn clap_buildconfig_parses_positionals() {
        match parse_build(&["buildconfig", "p", "r"]) {
            BuildCommand::Buildconfig {
                project,
                repository,
                alternative_project,
            } => {
                assert_eq!(project, "p");
                assert_eq!(repository, "r");
                assert_eq!(alternative_project, None);
            }
            other => panic!("unexpected: {other:?}"),
        }
        match parse_build(&[
            "buildconfig",
            "p",
            "r",
            "--alternative-project",
            "openSUSE:Factory",
        ]) {
            BuildCommand::Buildconfig {
                alternative_project,
                ..
            } => {
                assert_eq!(alternative_project.as_deref(), Some("openSUSE:Factory"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn buildconfig_result_is_verbatim() {
        let (payload, text) = buildconfig_result("p", "r", None, "Macro: foo\n".to_string());
        assert_eq!(payload["project"], "p");
        assert_eq!(payload["repository"], "r");
        assert_eq!(payload["alternative_project"], serde_json::Value::Null);
        assert_eq!(payload["buildconfig"], "Macro: foo\n");
        assert_eq!(text, "Macro: foo\n");
        // With an alternative project the payload names it; the text
        // stays the config verbatim (design rule 4).
        let (payload, text) = buildconfig_result(
            "p",
            "r",
            Some("openSUSE:Factory"),
            "Macro: foo\n".to_string(),
        );
        assert_eq!(payload["alternative_project"], "openSUSE:Factory");
        assert_eq!(payload["buildconfig"], "Macro: foo\n");
        assert_eq!(text, "Macro: foo\n");
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
