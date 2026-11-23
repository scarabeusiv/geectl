//! `local-build` — the podman build vertical's command group: build
//! a package in the container, run its `_service` services, the
//! precommit gate, `spec-cleaner`, wipe a build root, prune the
//! cache, manage the daily periodic cleanup, enter the preserved
//! build root to run repro commands, and read the log of the last
//! local build.
//!
//! Thin like `service` and `build`: the flags mirror the library
//! parameters, the JSON payload carries the typed record, and the
//! text renderer prints the same values in a human-readable form.
//! The service leaves, the wipe leaf, and the log leaf make no
//! server calls, so no credentials resolve there: a half-set
//! credential env never breaks a local run. The build leaf fetches buildinfo/buildconfig
//! on a live run, so it resolves the endpoint's credentials; its
//! dry run touches nothing and resolves none.
//!
//! Mutations are gated on the global `--live`: without it, each
//! command prints the plan only (the library dry run) and touches
//! nothing. With it, the run executes in the container. A failing
//! service or a failing `pre_checkin.sh` fails the command with a
//! non-zero exit. One leaf is excepted: `prune` deletes only with
//! its own `--apply` flag — the plan is the default, and the global
//! `--live` does not gate it — because the scheduled task runs
//! exactly `prune --apply` and the leaf's confirmation is the flag
//! itself.
//!
//! Two mutations no flag names still go through the dry-run guard
//! (`DryRun::allow`): the first live `build` records the
//! would-install of the periodic schedule, and `prune --apply`
//! records the would-delete — a dry run prints the plan entry
//! instead of mutating.
//!
//! Coordinate header (design rule 1): every text rendering opens
//! with one header line echoing the query coordinates (`directory:
//! …`, plus `mode: …` and `service: …` when the query carries
//! them). The log leaf is a content command (design rule 4): its
//! text is the log verbatim, so no header is prepended; the
//! coordinates, log path, verdict, exit code, and truncation flag
//! ride in the JSON payload only (a DOCUMENTED omission).

use clap::Subcommand;
use geectl::auth::{Credentials, EnvObsAuth};
use geectl::config;
use geectl::debug::Debug;
use geectl::error::{ErrorKind, GeectlError};
use geectl::http::{Timeout, UreqClient};
use geectl::local_build::{
    validate_build_request, BuildLog, BuildRecord, BuildRequest, ChrootRecord, EvictionReport,
    LocalBuild, PrecommitRecord, ServiceRunMode, ServiceRunRecord, ServiceRunRequest,
    SpecCleanerResult, WipeOutcome,
};
use geectl::obs::{DryRun, ObsClient, OBS_API_BASE};
use geectl::periodic::{self, PeriodicOs, PeriodicOutcome, PeriodicStatus};
use serde_json::{json, Value};
use std::path::PathBuf;

/// The `local-build` subcommands. Names are the library operation
/// names in kebab-case, grouped under the top-level noun.
#[derive(Debug, Subcommand)]
pub enum LocalBuildCommand {
    /// Run the `_service` file's services in the container,
    /// filtered by the run mode.
    ServiceRun {
        /// The source directory carrying the `_service` file.
        directory: PathBuf,
        /// The run mode: run, manualrun, precommit, localrun,
        /// disabledrun.
        #[arg(long, default_value = "run")]
        mode: String,
        /// Run only this service, with the run mode forced as its
        /// mode.
        #[arg(long)]
        service: Option<String>,
        /// Timeout in seconds for the whole run.
        #[arg(long, default_value_t = 3600)]
        timeout_secs: u64,
    },
    /// The precommit gate: the `precommit`-mode service run, then
    /// `pre_checkin.sh` when the source directory carries one.
    ///
    /// The script step is part of the gate, not of the service
    /// mode: the mode filter alone runs services only.
    Precommit {
        /// The source directory carrying the `_service` file and
        /// optionally `pre_checkin.sh`.
        directory: PathBuf,
        /// Timeout in seconds for the whole gate.
        #[arg(long, default_value_t = 3600)]
        timeout_secs: u64,
    },
    /// Run `spec-cleaner` on a spec in the container.
    SpecCleaner {
        /// The source directory carrying the spec.
        directory: PathBuf,
        /// The spec file, inside the source directory.
        spec: PathBuf,
        /// Extra arguments for `spec-cleaner`, after `--`.
        #[arg(last = true)]
        args: Vec<String>,
        /// Timeout in seconds for the run.
        #[arg(long, default_value_t = 3600)]
        timeout_secs: u64,
    },
    /// Build the package in the container: the run-mode service
    /// run, the precommit gate, then the engine build.
    ///
    /// A failing pre-step fails the command before the build runs.
    Build {
        /// The source directory (spec + sources).
        directory: PathBuf,
        /// The OBS project the package belongs to.
        #[arg(long)]
        project: String,
        /// The repository to build for (e.g. `standard`).
        #[arg(long)]
        repo: String,
        /// The architecture to build for; must equal the host arch.
        #[arg(long, default_value_t = std::env::consts::ARCH.to_string())]
        arch: String,
        /// The multibuild flavor; required when the source carries
        /// a `_multibuild` file. One invocation builds one flavor.
        #[arg(long)]
        flavor: Option<String>,
        /// Wipe the root before building.
        #[arg(long)]
        clean: bool,
        /// The release number, passed through to the engine.
        #[arg(long)]
        release: Option<String>,
        /// Allow build dependencies from other users' `home:`
        /// projects (their packages run code in the build root).
        /// Without this such builds are refused.
        #[arg(long)]
        trust_all_projects: bool,
        /// Force an image pull even when the local image is fresh.
        #[arg(long)]
        refresh_image: bool,
        /// Build against another project instead of `project`:
        /// buildinfo, buildconfig, the trust check, and dep
        /// resolution use it. The root stays keyed by `project`.
        #[arg(long)]
        alternative_project: Option<String>,
        /// Build in this directory instead of the derived
        /// cache-dir root.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Skip the root initialization: start the build
        /// immediately in the existing root. Needs an initialized
        /// root (a previous build wrote the rpmlist and
        /// buildconfig there).
        #[arg(long)]
        noinit: bool,
        /// Use the preinstall image when the buildinfo offers one.
        /// Off by default: the engine extracts the image with
        /// bsdtar `--chroot` plus device nodes, which fail as a
        /// non-root user, so a rootless build never selects the
        /// image. Only set it when the engine runs as root.
        #[arg(long)]
        preinstall_image: bool,
        /// Save the built RPMs into this directory (created when
        /// missing) on a green build.
        #[arg(long, short = 'k')]
        keep_rpms: Option<PathBuf>,
        /// Prefer RPMs from this directory when installing the
        /// build root: a dep whose file is there is used instead of
        /// the cached or downloaded copy.
        #[arg(long, short = 'p')]
        prefer_packages: Option<PathBuf>,
        /// Timeout in seconds for the whole run.
        #[arg(long, default_value_t = 3600)]
        timeout_secs: u64,
    },
    /// Wipe a build root: delete its contents (the lock file
    /// stays). A mutation: dry-run is the default (only the
    /// plan is printed); `--live` performs the wipe. There is
    /// no force flag: the marker check and the in-use lock are
    /// safety invariants, never bypassed.
    Wipe {
        /// The repository the root was built for. Required
        /// unless `--root` names the root directly.
        #[arg(required_unless_present = "root")]
        repo: Option<String>,
        /// The architecture the root was built for. Required
        /// unless `--root` names the root directly.
        #[arg(required_unless_present = "root")]
        arch: Option<String>,
        /// The source directory (spec + sources); the package
        /// comes from the spec file in it, like the build.
        /// Required unless `--root` names the root directly.
        #[arg(required_unless_present = "root")]
        spec: Option<PathBuf>,
        /// The OBS project the package belongs to. Required
        /// unless `--root` names the root directly.
        #[arg(long, required_unless_present = "root")]
        project: Option<String>,
        /// The multibuild flavor; required when the source
        /// carries a `_multibuild` file.
        #[arg(long)]
        flavor: Option<String>,
        /// The root to wipe, named directly. Conflicts with
        /// the coordinates: when set, the root is wiped as named.
        #[arg(long, conflicts_with_all = ["repo", "arch", "spec", "project", "flavor"])]
        root: Option<PathBuf>,
    },
    /// Print the eviction plan (what would be evicted, in order,
    /// with bytes); delete only with `--apply`. The plan is the
    /// default: the global `--live` does not gate this leaf, the
    /// leaf's own `--apply` is the confirmation, so the scheduled
    /// task runs exactly `prune --apply`.
    Prune {
        /// Delete what the plan lists.
        #[arg(long)]
        apply: bool,
    },
    /// Install the daily periodic cleanup for this platform: write
    /// the scheduler entry and enable it. A mutation: dry-run is
    /// the default (only the plan is printed); `--live` performs
    /// the install.
    PeriodicInstall,
    /// Remove the daily periodic cleanup for this platform: disable
    /// the scheduler entry, then remove its files. A mutation:
    /// dry-run is the default (only the plan is printed); `--live`
    /// performs the uninstall.
    PeriodicUninstall,
    /// Show whether the daily periodic cleanup is installed and
    /// enabled on this platform. Read-only.
    PeriodicStatus,
    /// Enter the preserved build root of the last build for the
    /// package and run repro commands there, read from stdin, one
    /// per line, joined with `&&` into one shell command. A
    /// mutation: dry-run is the default (only the plan is printed);
    /// `--live` runs the commands in the root. There is no
    /// interactive shell: a terminal on stdin is refused instead of
    /// blocking on a read that never arrives. A failing repro still
    /// exits 0: the exit code is data, in `exit:` and in
    /// `record.result.exit_code`.
    Chroot {
        /// The source directory (spec + sources); the package
        /// comes from the spec file in it, like the build.
        directory: PathBuf,
        /// The OBS project the package belongs to.
        #[arg(long)]
        project: String,
        /// The repository the root was built for (e.g. `standard`).
        #[arg(long)]
        repo: String,
        /// The architecture the root was built for; defaults to the host arch.
        #[arg(long, default_value_t = std::env::consts::ARCH.to_string())]
        arch: String,
        /// The multibuild flavor; required when the source
        /// carries a `_multibuild` file.
        #[arg(long)]
        flavor: Option<String>,
        /// Timeout in seconds for the whole run.
        #[arg(long, default_value_t = 3600)]
        timeout_secs: u64,
    },
    /// Read the log of the last local build: the `<root>/.build.log`
    /// the build's coordinates name. A read, not a mutation: no
    /// `--live` needed. The text is the log verbatim (design rule 4);
    /// the verdict, exit code, log path, and truncation flag ride in
    /// the JSON payload.
    Log {
        /// The repository the root was built for.
        repo: String,
        /// The architecture the root was built for.
        arch: String,
        /// The source directory (spec + sources); the package comes
        /// from the spec file in it, like the build.
        spec: PathBuf,
        /// The OBS project the package belongs to.
        #[arg(long)]
        project: String,
        /// The multibuild flavor; required when the source carries
        /// a `_multibuild` file.
        #[arg(long)]
        flavor: Option<String>,
        /// Print only the last N lines of the log.
        #[arg(long)]
        tail: Option<usize>,
        /// Read the whole log without the 4 MiB cap.
        #[arg(long)]
        full: bool,
    },
}

/// Run a `local-build` command, returning the JSON payload and its
/// text rendering.
pub fn run_local_build(
    debug: &Debug,
    command: LocalBuildCommand,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let effective = config::effective_config(&file_config)?;
    // Unreachable by construction — effective_config always yields
    // Some(obs_url) — but kept as a defensive default rather than an
    // unwrap: a config change must not turn into a panic here.
    let obs_url = effective.config.obs_url.as_deref().unwrap_or(OBS_API_BASE);
    // Credentials resolve per command: the service leaves make no
    // server calls, so none resolve there — a half-set credential
    // env must not break a local run. The build leaf fetches
    // buildinfo/buildconfig on a live run, so it resolves the
    // endpoint's credentials; its dry run touches nothing and
    // resolves none.
    let credentials = match &command {
        LocalBuildCommand::Build { .. } if live => {
            EnvObsAuth::credentials_for_url(obs_url, obs_url, &file_config.instances)?
        }
        _ => Credentials::None,
    };
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = ObsClient::with_base_and_credentials(&http, obs_url, credentials)?
        .with_debug(debug.clone());
    let vertical = LocalBuild::from_config(&client, &effective.config)?.with_debug(debug.clone());
    match command {
        LocalBuildCommand::ServiceRun {
            directory,
            mode,
            service,
            timeout_secs,
        } => {
            let mode = ServiceRunMode::from_command_name(&mode)?;
            let mut req = ServiceRunRequest::new(directory.clone(), mode);
            req.service = service.clone();
            req.timeout_secs = timeout_secs;
            req.dry_run = !live;
            let record = vertical.service_run(&req)?;
            let header = match &service {
                Some(service) => format!(
                    "directory: {} mode: {} service: {service}",
                    directory.display(),
                    mode.command_name()
                ),
                None => format!(
                    "directory: {} mode: {}",
                    directory.display(),
                    mode.command_name()
                ),
            };
            let text = with_header(&header, render_service_run(&record));
            let payload = json!({
                "directory": directory,
                "mode": mode.command_name(),
                "service": service,
                "dry_run": !live,
                "record": record,
            });
            Ok((payload, text))
        }
        LocalBuildCommand::Precommit {
            directory,
            timeout_secs,
        } => {
            let record = vertical.precommit(&directory, timeout_secs, !live)?;
            let text = with_header(
                &format!("directory: {}", directory.display()),
                render_precommit(&record),
            );
            let payload = json!({
                "directory": directory,
                "dry_run": !live,
                "record": record,
            });
            Ok((payload, text))
        }
        LocalBuildCommand::SpecCleaner {
            directory,
            spec,
            args,
            timeout_secs,
        } => {
            let result = vertical.spec_cleaner(&directory, &spec, &args, timeout_secs, !live)?;
            let text = with_header(
                &format!(
                    "directory: {} spec: {}",
                    directory.display(),
                    spec.display()
                ),
                render_spec_cleaner(&result),
            );
            let payload = json!({
                "directory": directory,
                "spec": spec,
                "dry_run": !live,
                "result": result,
            });
            Ok((payload, text))
        }
        LocalBuildCommand::Build {
            directory,
            project,
            repo,
            arch,
            flavor,
            clean,
            release,
            trust_all_projects,
            refresh_image,
            alternative_project,
            root,
            noinit,
            preinstall_image,
            keep_rpms,
            prefer_packages,
            timeout_secs,
        } => {
            // One invocation does the whole loop: the run-mode
            // service run fetches the sources, the precommit gate
            // runs the precommit-mode services and pre_checkin.sh,
            // then the engine builds. A failing pre-step fails the
            // command before the build runs. The run-mode step
            // already covers trylocal/localonly services, so the
            // precommit step runs them a second time; services
            // regenerate their outputs, so the repeat is a second
            // run of the same generation, not a second effect.
            // The request is built and validated first: a bad
            // coordinate (project, repo, arch, timeout) fails
            // before any service runs or script executes — the
            // pre-steps take no project or arch, so they cannot
            // reject one themselves.
            let mut build_req = BuildRequest::new(directory.clone(), &project, &repo, &arch);
            build_req.flavor = flavor.clone();
            build_req.clean = clean;
            build_req.release = release.clone();
            build_req.trust_all_projects = trust_all_projects;
            build_req.refresh_image = refresh_image;
            build_req.alternative_project = alternative_project.clone();
            build_req.root = root.clone();
            build_req.noinit = noinit;
            build_req.preinstall_image = preinstall_image;
            build_req.keep_rpms = keep_rpms.clone();
            build_req.prefer_packages = prefer_packages.clone();
            build_req.timeout_secs = timeout_secs;
            build_req.dry_run = !live;
            validate_build_request(&build_req)?;
            // First live build installs the daily cleanup schedule,
            // so nobody has to remember `periodic-install`. Best
            // effort only: a missing scheduler never fails the
            // build, and a dry run installs nothing. The notice is
            // printed when the install happens, so it is not lost
            // when the build itself fails. The install is a
            // mutation the build does not ask for by name, so it
            // goes through the dry-run guard like every other
            // mutation: a dry run records the would-install in
            // the plan instead of installing.
            let dry_run = DryRun::new(!live);
            let auto_install_notice =
                if dry_run.allow("install the daily periodic cache cleanup on first live build") {
                    auto_install_periodic_notice()
                } else {
                    None
                };
            if let Some(notice) = &auto_install_notice {
                eprintln!("{notice}");
            }
            let mut service_req = ServiceRunRequest::new(directory.clone(), ServiceRunMode::Run);
            service_req.timeout_secs = timeout_secs;
            service_req.dry_run = !live;
            // One budget for the whole run: the service run, the
            // precommit gate, and the engine build share a single
            // deadline minted here, so a slow pre-step leaves less
            // time for the build instead of each phase taking a
            // fresh full timeout.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
            let services = vertical.service_run_with_deadline(&service_req, deadline)?;
            let precommit =
                vertical.precommit_with_deadline(&directory, timeout_secs, deadline, !live)?;
            let record = vertical.build_with_deadline(&build_req, deadline)?;
            let mut header = format!(
                "directory: {} project: {project} repo: {repo} arch: {arch}",
                directory.display()
            );
            if let Some(alt) = &alternative_project {
                header.push_str(&format!(" alternative-project: {alt}"));
            }
            if let Some(flavor) = &flavor {
                header.push_str(&format!(" flavor: {flavor}"));
            }
            let text = with_header(
                &header,
                format!(
                    "services:\n{}\nprecommit:\n{}\nbuild:\n{}",
                    render_service_run(&services),
                    render_precommit(&precommit),
                    render_build(&record)
                ),
            );
            let payload = json!({
                "directory": directory,
                "project": project,
                "repo": repo,
                "arch": arch,
                "flavor": flavor,
                "alternative_project": alternative_project,
                "noinit": noinit,
                "keep_rpms": keep_rpms,
                "prefer_packages": prefer_packages,
                "dry_run": !live,
                "periodic_auto_install": auto_install_notice,
                "services": services,
                "precommit": precommit,
                "record": record,
            });
            Ok((payload, text))
        }
        LocalBuildCommand::Wipe {
            repo,
            arch,
            spec,
            project,
            flavor,
            root,
        } => {
            let outcome = match (&root, &repo, &arch, &spec, &project) {
                (Some(root), _, _, _, _) => vertical.wipe_named_root(root, !live)?,
                (None, Some(repo), Some(arch), Some(spec), Some(project)) => {
                    vertical.wipe_source_root(spec, project, repo, arch, &flavor, !live)?
                }
                // Unreachable: clap requires the coordinates
                // unless --root is set — kept as a defensive
                // error rather than an unwrap.
                (None, _, _, _, _) => {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        "wipe needs the repository, architecture, source directory and project, or --root",
                    ));
                }
            };
            let header = match &root {
                Some(root) => format!("root: {}", root.display()),
                None => match (&spec, &project, &repo, &arch, &flavor) {
                    (Some(spec), Some(project), Some(repo), Some(arch), Some(flavor)) => format!(
                        "spec: {} project: {project} repo: {repo} arch: {arch} flavor: {flavor}",
                        spec.display()
                    ),
                    (Some(spec), Some(project), Some(repo), Some(arch), None) => format!(
                        "spec: {} project: {project} repo: {repo} arch: {arch}",
                        spec.display()
                    ),
                    // Unreachable: same clap guarantee as above.
                    _ => String::new(),
                },
            };
            let text = with_header(&header, render_wipe(&outcome));
            let payload = json!({
                "spec": spec,
                "project": project,
                "repo": repo,
                "arch": arch,
                "flavor": flavor,
                "root": root,
                "dry_run": !live,
                "outcome": outcome,
            });
            Ok((payload, text))
        }
        LocalBuildCommand::Prune { apply } => {
            // The leaf's own --apply is the confirmation, not the
            // global --live: the scheduled task runs exactly
            // `prune --apply`, and the plan is the default. A
            // `--live` without `--apply` says so in text, so it
            // never looks like a deletion that did not happen.
            // `--apply` is the confirmation, not the global
            // `--live` — but the deletion still goes through the
            // dry-run guard, so a plan run records the
            // would-delete instead of deleting.
            let dry_run = DryRun::new(!apply);
            let report = if dry_run.allow("delete the local build cache") {
                vertical.prune_apply()?
            } else {
                vertical.prune_plan()?
            };
            let header = format!("cache: {}", vertical.cache_dir().display());
            let mut text = with_header(&header, render_prune(&report, apply));
            if let Some(note) = prune_live_note(live, apply) {
                text.push('\n');
                text.push_str(note);
            }
            let payload = json!({
                "cache_dir": vertical.cache_dir(),
                "dry_run": !apply,
                "report": report,
            });
            Ok((payload, text))
        }
        LocalBuildCommand::PeriodicInstall => {
            let os = periodic_os()?;
            let exe = std::env::current_exe().map_err(|e| {
                GeectlError::new(
                    ErrorKind::InternalError,
                    format!("cannot resolve the geectl binary path: {e}"),
                )
            })?;
            let home = periodic::resolve_home()?;
            let xdg = std::env::var("XDG_CONFIG_HOME").ok();
            let outcome = periodic::install(os, &exe, &home, xdg.as_deref(), !live)?;
            let header = format!("periodic: {}", os.name());
            let text = with_header(&header, render_periodic_outcome(&outcome, true));
            let payload = json!({
                "platform": os.name(),
                "dry_run": !live,
                "outcome": outcome,
            });
            Ok((payload, text))
        }
        LocalBuildCommand::PeriodicUninstall => {
            let os = periodic_os()?;
            let home = periodic::resolve_home()?;
            let xdg = std::env::var("XDG_CONFIG_HOME").ok();
            let outcome = periodic::uninstall(os, &home, xdg.as_deref(), !live)?;
            let header = format!("periodic: {}", os.name());
            let text = with_header(&header, render_periodic_outcome(&outcome, false));
            let payload = json!({
                "platform": os.name(),
                "dry_run": !live,
                "outcome": outcome,
            });
            Ok((payload, text))
        }
        LocalBuildCommand::PeriodicStatus => {
            let os = periodic_os()?;
            let home = periodic::resolve_home()?;
            let xdg = std::env::var("XDG_CONFIG_HOME").ok();
            let status = periodic::status(os, &home, xdg.as_deref())?;
            let header = format!("periodic: {}", os.name());
            let text = with_header(&header, render_periodic_status(&status));
            let payload = json!({
                "platform": os.name(),
                "status": status,
            });
            Ok((payload, text))
        }
        LocalBuildCommand::Chroot {
            directory,
            project,
            repo,
            arch,
            flavor,
            timeout_secs,
        } => {
            let commands = read_stdin_commands()?;
            let record = vertical.build_chroot(
                &directory,
                &project,
                &repo,
                &arch,
                &flavor,
                &commands,
                timeout_secs,
                !live,
            )?;
            let mut header = format!(
                "directory: {} project: {project} repo: {repo} arch: {arch}",
                directory.display()
            );
            if let Some(flavor) = &flavor {
                header.push_str(&format!(" flavor: {flavor}"));
            }
            let text = with_header(&header, render_chroot(&record));
            let payload = json!({
                "directory": directory,
                "project": project,
                "repo": repo,
                "arch": arch,
                "flavor": flavor,
                "dry_run": !live,
                "record": record,
            });
            Ok((payload, text))
        }
        LocalBuildCommand::Log {
            repo,
            arch,
            spec,
            project,
            flavor,
            tail,
            full,
        } => {
            let log = vertical
                .build_log_for_source(&spec, &project, &repo, &arch, &flavor, tail, full)?;
            Ok(log_result(
                &spec, &project, &repo, &arch, &flavor, tail, full, &log,
            ))
        }
    }
}

/// The first-run auto-install, resolved and swallowed: any missing
/// piece (unsupported platform, no home, no binary path) or any
/// install failure is a quiet skip, never a build failure.
fn auto_install_periodic_notice() -> Option<String> {
    let os = periodic_os().ok()?;
    let exe = std::env::current_exe().ok()?;
    let home = periodic::resolve_home().ok()?;
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    periodic::maybe_auto_install(os, &exe, &home, xdg.as_deref())
}

/// The current platform's schedule, or `UnsupportedOs` where no
/// schedule ships.
fn periodic_os() -> Result<PeriodicOs, GeectlError> {
    PeriodicOs::current().ok_or_else(|| {
        GeectlError::new(
            ErrorKind::UnsupportedOs,
            format!("no periodic cleanup ships on {}", std::env::consts::OS),
        )
    })
}

/// Prepend the one-line coordinate header to a text rendering
/// (design rule 1): `header` first, then the body unchanged.
fn with_header(header: &str, text: String) -> String {
    format!("{header}\n{text}")
}

/// A service run as text: the dry-run plan (image plus one argv
/// line per planned service), or the run's per-service exit codes
/// plus the image digest. An empty run reports that no services
/// were selected.
fn render_service_run(record: &ServiceRunRecord) -> String {
    if record.dry_run {
        match &record.plan {
            Some(plan) if !plan.services.is_empty() => {
                let mut lines = vec![format!("image: {}", plan.image)];
                for service in &plan.services {
                    lines.push(format!("{}: {}", service.name, service.argv.join(" ")));
                }
                lines.join("\n")
            }
            _ => "no services selected".to_string(),
        }
    } else if record.ran.is_empty() {
        "no services selected".to_string()
    } else {
        let mut lines: Vec<String> = record
            .ran
            .iter()
            .map(|run| format!("{}: exit {}", run.name, run.exit_code))
            .collect();
        lines.push(format!(
            "image: {}",
            record.image_digest.as_deref().unwrap_or("<unknown>")
        ));
        lines.join("\n")
    }
}

/// A precommit record as text: the service run, then the
/// `pre_checkin.sh` line — ran with its exit code, not present, or
/// not evaluated on a dry run.
fn render_precommit(record: &PrecommitRecord) -> String {
    let mut text = render_service_run(&record.services);
    let line = if record.dry_run {
        "pre_checkin.sh: not evaluated (dry run)".to_string()
    } else if record.pre_checkin.ran {
        format!(
            "pre_checkin.sh: ran, exit {}",
            record.pre_checkin.exit_code.unwrap_or(-1)
        )
    } else {
        "pre_checkin.sh: not present".to_string()
    };
    text.push('\n');
    text.push_str(&line);
    text
}

/// A `spec-cleaner` result as text: the dry-run plan argv, or the
/// exit code plus the tool's stdout and stderr when non-empty.
fn render_spec_cleaner(result: &SpecCleanerResult) -> String {
    if result.dry_run {
        match &result.plan {
            Some(argv) => format!("plan: {}", argv.join(" ")),
            None => "no plan".to_string(),
        }
    } else {
        let mut lines = vec![format!("exit: {}", result.exit_code)];
        if !result.stdout.is_empty() {
            lines.push(result.stdout.clone());
        }
        if !result.stderr.is_empty() {
            lines.push(result.stderr.clone());
        }
        lines.join("\n")
    }
}

/// A build record as text: the dry-run plan (image, root, repo,
/// arch, flavor, the new-flag fields when set, and the engine argv),
/// or the verdict with the exit code, log path, RPMs, rpmlint and
/// check summaries, image digest, engine version, root, cache
/// numbers, and eviction, plus the trusted foreign projects when
/// any were accepted.
fn render_build(record: &BuildRecord) -> String {
    if record.dry_run {
        match &record.plan {
            Some(plan) => {
                let mut lines = vec![
                    format!("plan: {}", plan.argv.join(" ")),
                    format!("image: {}", plan.image),
                    format!("root: {}", plan.root.display()),
                    format!("repo: {}", plan.repo),
                    format!("arch: {}", plan.arch),
                ];
                if let Some(flavor) = &plan.flavor {
                    lines.push(format!("flavor: {flavor}"));
                }
                if let Some(alt) = &plan.alternative_project {
                    lines.push(format!("alternative-project: {alt}"));
                }
                if plan.trust_all_projects {
                    lines.push("trust-all-projects: true".to_string());
                }
                if let Some(keep) = &plan.keep_rpms {
                    lines.push(format!("keep-rpms: {}", keep.display()));
                }
                if let Some(prefer) = &plan.prefer_packages {
                    lines.push(format!("prefer-packages: {}", prefer.display()));
                }
                lines.join("\n")
            }
            None => "no plan".to_string(),
        }
    } else {
        let mut lines = vec![
            format!("verdict: {}", record.verdict),
            format!("exit: {}", record.exit_code),
            format!("log: {}", record.log_path.display()),
        ];
        if record.rpms.is_empty() {
            lines.push("rpms: none".to_string());
        } else {
            for rpm in &record.rpms {
                lines.push(format!("rpm: {}", rpm.display()));
            }
        }
        for rpm in &record.kept_rpms {
            lines.push(format!("kept-rpm: {}", rpm.display()));
        }
        lines.push(format!(
            "rpmlint: {} errors, {} warnings (badness {})",
            record.rpmlint.errors, record.rpmlint.warnings, record.rpmlint.badness
        ));
        if let Some(checks) = &record.checks {
            lines.push(format!(
                "check: {} passed, {} failed of {}",
                checks.passed, checks.failed, checks.total
            ));
        }
        lines.push(format!(
            "image: {}",
            record.image_digest.as_deref().unwrap_or("<unknown>")
        ));
        lines.push(format!(
            "engine: {}",
            record.engine_version.as_deref().unwrap_or("<unknown>")
        ));
        if record.root_reused {
            lines.push(format!("root: {} (reused)", record.root_path.display()));
        } else {
            lines.push(format!("root: {}", record.root_path.display()));
        }
        let mut cache = format!(
            "cache: pkg {} hits, {} misses",
            record.cache.pkg_hits, record.cache.pkg_misses
        );
        if record.cache.pkg_preferred > 0 {
            cache.push_str(&format!(", {} preferred", record.cache.pkg_preferred));
        }
        if let Some(hits) = record.cache.sccache_hits {
            cache.push_str(&format!(", sccache {hits} hits"));
        }
        lines.push(cache);
        if let Some(image) = &record.preinstall_image {
            lines.push(format!("preinstall-image: {image}"));
        }
        if !record.trusted_foreign_projects.is_empty() {
            lines.push(format!(
                "trusted-foreign-projects: {}",
                record.trusted_foreign_projects.join(", ")
            ));
        }
        if record.eviction.ran {
            lines.push(format!(
                "eviction: ran, freed {} bytes ({} items)",
                record.eviction.freed_bytes,
                record.eviction.evicted.len()
            ));
            if record.eviction.shortfall_bytes > 0 {
                lines.push(format!(
                    "eviction shortfall: {} bytes",
                    record.eviction.shortfall_bytes
                ));
            }
        }
        if !record.eviction.failures.is_empty() {
            lines.push(format!(
                "eviction failures: {}",
                record.eviction.failures.join("; ")
            ));
        }
        lines.join("\n")
    }
}

/// A wipe as text: the dry-run plan, or the performed wipe. The
/// plan names the entry count and bytes the wipe would delete; a
/// root without the marker reports that there is nothing to wipe
/// instead of failing the plan — the live wipe refuses it, like
/// the build refuses an unknown root. A corrupt marker fails the
/// plan outright: only a missing marker reports the absence.
fn render_wipe(outcome: &WipeOutcome) -> String {
    if outcome.dry_run {
        if outcome.existed {
            format!(
                "would wipe {} entries ({} bytes) at {}",
                outcome.entries,
                outcome.bytes,
                outcome.root.display()
            )
        } else {
            format!("no geectl build root at {}", outcome.root.display())
        }
    } else {
        format!(
            "wiped {} entries ({} bytes) at {}",
            outcome.entries,
            outcome.bytes,
            outcome.root.display()
        )
    }
}

/// The note appended to prune text when `--live` is passed
/// without `--apply`: `--live` never deletes here, so a plan that
/// follows it must say so instead of looking like a deletion that
/// did not happen.
fn prune_live_note(live: bool, apply: bool) -> Option<&'static str> {
    if live && !apply {
        Some("note: --live does not delete here; use --apply to delete")
    } else {
        None
    }
}

/// A prune run as text: the plan, or the performed eviction. The
/// plan lists what would be evicted, in order, with bytes; the
/// applied run lists what was evicted. A run that evicts nothing
/// says so, and a run skipped on the eviction lock says so — the
/// lock failure is in the report either way.
fn render_prune(report: &EvictionReport, applied: bool) -> String {
    let mut lines = Vec::new();
    if !report.ran {
        if report.failures.is_empty() {
            lines.push("eviction: nothing to evict".to_string());
        } else {
            lines.push("eviction: skipped, another eviction holds the lock".to_string());
        }
    } else if applied {
        lines.push(format!(
            "eviction: ran, freed {} bytes ({} items)",
            report.freed_bytes,
            report.evicted.len()
        ));
    } else {
        lines.push(format!(
            "eviction: plan, would free {} bytes ({} items)",
            report.freed_bytes,
            report.evicted.len()
        ));
    }
    for item in &report.evicted {
        lines.push(format!(
            "eviction: {} {} ({} bytes, {})",
            item.kind, item.path, item.bytes, item.reason
        ));
    }
    if report.shortfall_bytes > 0 {
        lines.push(format!(
            "eviction shortfall: {} bytes",
            report.shortfall_bytes
        ));
    }
    if !report.failures.is_empty() {
        lines.push(format!("eviction failures: {}", report.failures.join("; ")));
    }
    lines.join("\n")
}

/// An install or uninstall run as text: the plan in a dry run, the
/// performed run with `--live`. Files first, then the commands.
fn render_periodic_outcome(outcome: &PeriodicOutcome, install: bool) -> String {
    let mut lines = vec![format!("task: {}", outcome.label)];
    for file in &outcome.files {
        let verb = if outcome.dry_run {
            if install {
                "would write"
            } else {
                "would remove"
            }
        } else if install {
            "wrote"
        } else {
            "removed"
        };
        lines.push(format!("{verb}: {}", file.display()));
    }
    for command in &outcome.commands {
        if outcome.dry_run {
            lines.push(format!("would run: {}", command.join(" ")));
        } else {
            lines.push(format!("ran: {}", command.join(" ")));
        }
    }
    lines.join("\n")
}
/// The largest stdin payload `chroot` reads: 64 KiB, matching the
/// config and secret file caps, and far below the exec `ARG_MAX` the
/// joined `--shell-cmd` would otherwise die at. An unbounded read
/// lets `yes | ...` grow memory without bound.
const MAX_STDIN_COMMANDS_BYTES: usize = 64 * 1024;

/// The most repro commands one `chroot` run takes: a repro is a
/// handful of fragments, and the byte cap above bounds the total
/// size either way — the count cap bounds the join and the argv
/// handling for many tiny lines.
const MAX_STDIN_COMMANDS: usize = 1024;

/// Read the repro commands from stdin, one per line: blank lines
/// are dropped, the rest are shell fragments for the root, in
/// order. A terminal on stdin is refused instead of blocking on a
/// read that never arrives: there is no interactive shell to fall
/// back to. The read is capped at [`MAX_STDIN_COMMANDS_BYTES`]
/// bytes and [`MAX_STDIN_COMMANDS`] commands: anything larger is
/// invalid input, not a read.
fn read_stdin_commands() -> Result<Vec<String>, GeectlError> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "chroot reads repro commands from stdin; pipe them in (no interactive shell)",
        ));
    }
    read_commands_from(std::io::stdin())
}

/// Read repro commands from `reader`, capped: one byte past the byte
/// cap, or one command past the count cap, is invalid input. The
/// byte cap is enforced during the read, so an endless pipe stops
/// at the cap instead of growing memory without bound.
fn read_commands_from<R: std::io::Read>(reader: R) -> Result<Vec<String>, GeectlError> {
    use std::io::Read;
    let mut text = String::new();
    reader
        .take(MAX_STDIN_COMMANDS_BYTES as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| {
            GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot read repro commands from stdin: {e}"),
            )
        })?;
    if text.len() > MAX_STDIN_COMMANDS_BYTES {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("repro commands exceed the {MAX_STDIN_COMMANDS_BYTES}-byte stdin cap"),
        ));
    }
    let commands = parse_stdin_commands(&text);
    if commands.len() > MAX_STDIN_COMMANDS {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("repro commands exceed the {MAX_STDIN_COMMANDS}-command stdin cap"),
        ));
    }
    Ok(commands)
}

/// Split piped stdin into repro commands: one non-blank line each,
/// trimmed, in order.
fn parse_stdin_commands(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// A chroot record as text: the root, one `run:` line per command,
/// and on a live run the exit code plus the command's stdout and
/// stderr when non-empty. The commands stay visible on a live run:
/// the payload carries them, so the text must too.
fn render_chroot(record: &ChrootRecord) -> String {
    let mut lines = vec![format!("root: {}", record.root.display())];
    for command in &record.commands {
        lines.push(format!("run: {command}"));
    }
    if !record.dry_run {
        if let Some(result) = &record.result {
            lines.push(format!("exit: {}", result.exit_code));
            if !result.stdout.is_empty() {
                lines.push(result.stdout.clone());
            }
            if !result.stderr.is_empty() {
                lines.push(result.stderr.clone());
            }
        }
    }
    lines.join("\n")
}

/// The periodic status as text: installed, enabled and active, with
/// `unknown` where the scheduler cannot be asked.
fn render_periodic_status(status: &PeriodicStatus) -> String {
    let state = |value: Option<bool>| match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    };
    format!(
        "task: {}\ninstalled: {}\nenabled: {}\nactive: {}",
        status.label,
        state(status.installed),
        state(status.enabled),
        state(status.active)
    )
}
/// The `log` result: the payload carries the query coordinates,
/// the read window (`tail`, `full`), and the log record (log path,
/// lines, truncation flag, verdict, exit code); the text is the log
/// lines verbatim (design rule 4 — a content command), so piping it
/// to a file keeps the log intact. The verdict, exit code, log path,
/// and truncation flag are a DOCUMENTED omission from the text: they
/// ride in the JSON payload only.
#[allow(clippy::too_many_arguments)]
fn log_result(
    spec: &PathBuf,
    project: &str,
    repo: &str,
    arch: &str,
    flavor: &Option<String>,
    tail: Option<usize>,
    full: bool,
    log: &BuildLog,
) -> (Value, String) {
    let payload = json!({
        "spec": spec,
        "project": project,
        "repo": repo,
        "arch": arch,
        "flavor": flavor,
        "tail": tail,
        "full": full,
        "log": log,
    });
    (payload, render_build_log(log))
}

/// A build log as text: the log lines verbatim.
fn render_build_log(log: &BuildLog) -> String {
    log.lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cli, Command};
    use clap::Parser;
    use geectl::local_build::{
        BuildLog, BuildRecord, CacheStats, CheckSummary, EnginePlan, EvictedItem, EvictionReport,
        PlannedService, PrecheckinOutcome, RpmlintSummary, ServiceRunOutcome, ServiceRunPlan,
        ShellResult,
    };

    fn parse(args: &[&str]) -> LocalBuildCommand {
        match Cli::parse_from(args).command {
            Some(Command::LocalBuild { command }) => command,
            _ => panic!("expected a local-build command, got {:?}", args),
        }
    }

    #[test]
    fn service_run_parses_defaults_and_flags() {
        assert!(matches!(
            parse(&["geectl", "local-build", "service-run", "/tmp/pkg"]),
            LocalBuildCommand::ServiceRun { .. }
        ));
        match parse(&[
            "geectl",
            "local-build",
            "service-run",
            "/tmp/pkg",
            "--mode",
            "manualrun",
            "--service",
            "tar_scm",
            "--timeout-secs",
            "60",
        ]) {
            LocalBuildCommand::ServiceRun {
                directory,
                mode,
                service,
                timeout_secs,
            } => {
                assert_eq!(directory, PathBuf::from("/tmp/pkg"));
                assert_eq!(mode, "manualrun");
                assert_eq!(service.as_deref(), Some("tar_scm"));
                assert_eq!(timeout_secs, 60);
            }
            _ => panic!("wrong variant"),
        }
        match parse(&["geectl", "local-build", "service-run", "/tmp/pkg"]) {
            LocalBuildCommand::ServiceRun {
                mode,
                service,
                timeout_secs,
                ..
            } => {
                assert_eq!(mode, "run");
                assert_eq!(service, None);
                assert_eq!(timeout_secs, 3600);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn precommit_and_spec_cleaner_parse() {
        assert!(matches!(
            parse(&["geectl", "local-build", "precommit", "/tmp/pkg"]),
            LocalBuildCommand::Precommit { .. }
        ));
        match parse(&[
            "geectl",
            "local-build",
            "spec-cleaner",
            "/tmp/pkg",
            "foo.spec",
            "--",
            "-i",
            "--remove-groups",
        ]) {
            LocalBuildCommand::SpecCleaner {
                directory,
                spec,
                args,
                timeout_secs,
            } => {
                assert_eq!(directory, PathBuf::from("/tmp/pkg"));
                assert_eq!(spec, PathBuf::from("foo.spec"));
                assert_eq!(args, vec!["-i", "--remove-groups"]);
                assert_eq!(timeout_secs, 3600);
            }
            _ => panic!("wrong variant"),
        }
        assert!(Cli::try_parse_from(["geectl", "local-build", "frobnicate"]).is_err());
        assert!(Cli::try_parse_from(["geectl", "local-build", "service-run"]).is_err());
    }

    #[test]
    fn build_parses_defaults_and_flags() {
        match parse(&[
            "geectl",
            "local-build",
            "build",
            "/tmp/pkg",
            "--project",
            "home:foo",
            "--repo",
            "standard",
        ]) {
            LocalBuildCommand::Build {
                directory,
                project,
                repo,
                arch,
                flavor,
                clean,
                release,
                trust_all_projects,
                refresh_image,
                timeout_secs,
                ..
            } => {
                assert_eq!(directory, PathBuf::from("/tmp/pkg"));
                assert_eq!(project, "home:foo");
                assert_eq!(repo, "standard");
                assert_eq!(arch, std::env::consts::ARCH);
                assert_eq!(flavor, None);
                assert!(!clean);
                assert_eq!(release, None);
                assert!(!trust_all_projects);
                assert!(!refresh_image);
                assert_eq!(timeout_secs, 3600);
            }
            _ => panic!("wrong variant"),
        }
        match parse(&[
            "geectl",
            "local-build",
            "build",
            "/tmp/pkg",
            "--project",
            "home:foo",
            "--repo",
            "standard",
            "--arch",
            "x86_64",
            "--flavor",
            "lib",
            "--clean",
            "--release",
            "2",
            "--trust-all-projects",
            "--refresh-image",
            "--timeout-secs",
            "60",
        ]) {
            LocalBuildCommand::Build {
                arch,
                flavor,
                clean,
                release,
                trust_all_projects,
                refresh_image,
                timeout_secs,
                ..
            } => {
                assert_eq!(arch, "x86_64");
                assert_eq!(flavor.as_deref(), Some("lib"));
                assert!(clean);
                assert_eq!(release.as_deref(), Some("2"));
                assert!(trust_all_projects);
                assert!(refresh_image);
                assert_eq!(timeout_secs, 60);
            }
            _ => panic!("wrong variant"),
        }
        match parse(&[
            "geectl",
            "local-build",
            "build",
            "/tmp/pkg",
            "--project",
            "home:foo",
            "--repo",
            "standard",
            "--alternative-project",
            "openSUSE:Factory",
            "--root",
            "/tmp/custom-root",
            "--noinit",
            "--preinstall-image",
            "--keep-rpms",
            "/tmp/keep",
            "--prefer-packages",
            "/tmp/prefer",
        ]) {
            LocalBuildCommand::Build {
                alternative_project,
                root,
                noinit,
                preinstall_image,
                keep_rpms,
                prefer_packages,
                ..
            } => {
                assert_eq!(alternative_project.as_deref(), Some("openSUSE:Factory"));
                assert_eq!(root, Some(PathBuf::from("/tmp/custom-root")));
                assert!(noinit);
                assert!(preinstall_image);
                assert_eq!(keep_rpms, Some(PathBuf::from("/tmp/keep")));
                assert_eq!(prefer_packages, Some(PathBuf::from("/tmp/prefer")));
            }
            _ => panic!("wrong variant"),
        }
        match parse(&[
            "geectl",
            "local-build",
            "build",
            "/tmp/pkg",
            "--project",
            "home:foo",
            "--repo",
            "standard",
            "-k",
            "/tmp/keep",
            "-p",
            "/tmp/prefer",
        ]) {
            LocalBuildCommand::Build {
                keep_rpms,
                prefer_packages,
                ..
            } => {
                assert_eq!(keep_rpms, Some(PathBuf::from("/tmp/keep")));
                assert_eq!(prefer_packages, Some(PathBuf::from("/tmp/prefer")));
            }
            _ => panic!("wrong variant"),
        }
        assert!(Cli::try_parse_from(["geectl", "local-build", "build", "/tmp/pkg"]).is_err());
        assert!(Cli::try_parse_from([
            "geectl",
            "local-build",
            "build",
            "/tmp/pkg",
            "--project",
            "home:foo",
        ])
        .is_err());
    }

    #[test]
    fn wipe_parses_coordinates_and_root() {
        match parse(&[
            "geectl",
            "local-build",
            "wipe",
            "standard",
            "x86_64",
            "/tmp/pkg",
            "--project",
            "home:foo",
            "--flavor",
            "lib",
        ]) {
            LocalBuildCommand::Wipe {
                repo,
                arch,
                spec,
                project,
                flavor,
                root,
            } => {
                assert_eq!(repo.as_deref(), Some("standard"));
                assert_eq!(arch.as_deref(), Some("x86_64"));
                assert_eq!(spec, Some(PathBuf::from("/tmp/pkg")));
                assert_eq!(project.as_deref(), Some("home:foo"));
                assert_eq!(flavor.as_deref(), Some("lib"));
                assert_eq!(root, None);
            }
            _ => panic!("wrong variant"),
        }
        match parse(&["geectl", "local-build", "wipe", "--root", "/tmp/root"]) {
            LocalBuildCommand::Wipe {
                root,
                repo,
                arch,
                spec,
                project,
                flavor,
            } => {
                assert_eq!(root, Some(PathBuf::from("/tmp/root")));
                assert_eq!(repo, None);
                assert_eq!(arch, None);
                assert_eq!(spec, None);
                assert_eq!(project, None);
                assert_eq!(flavor, None);
            }
            _ => panic!("wrong variant"),
        }
        // --root conflicts with the coordinates.
        assert!(Cli::try_parse_from([
            "geectl",
            "local-build",
            "wipe",
            "standard",
            "x86_64",
            "/tmp/pkg",
            "--project",
            "home:foo",
            "--root",
            "/tmp/root",
        ])
        .is_err());
        // Without --root the coordinates are required.
        assert!(Cli::try_parse_from(["geectl", "local-build", "wipe"]).is_err());
    }

    #[test]
    fn render_wipe_reports_plan_and_outcome() {
        let planned = WipeOutcome {
            root: PathBuf::from("/tmp/root"),
            existed: true,
            dry_run: true,
            entries: 3,
            bytes: 42,
        };
        assert_eq!(
            render_wipe(&planned),
            "would wipe 3 entries (42 bytes) at /tmp/root"
        );
        let missing = WipeOutcome {
            root: PathBuf::from("/tmp/root"),
            existed: false,
            dry_run: true,
            entries: 0,
            bytes: 0,
        };
        assert_eq!(render_wipe(&missing), "no geectl build root at /tmp/root");
        let wiped = WipeOutcome {
            root: PathBuf::from("/tmp/root"),
            existed: true,
            dry_run: false,
            entries: 3,
            bytes: 42,
        };
        assert_eq!(
            render_wipe(&wiped),
            "wiped 3 entries (42 bytes) at /tmp/root"
        );
    }

    #[test]
    fn chroot_parses_defaults_and_flags() {
        match parse(&[
            "geectl",
            "local-build",
            "chroot",
            "/tmp/pkg",
            "--project",
            "home:foo",
            "--repo",
            "standard",
        ]) {
            LocalBuildCommand::Chroot {
                directory,
                project,
                repo,
                arch,
                flavor,
                timeout_secs,
            } => {
                assert_eq!(directory, PathBuf::from("/tmp/pkg"));
                assert_eq!(project, "home:foo");
                assert_eq!(repo, "standard");
                assert_eq!(arch, std::env::consts::ARCH);
                assert_eq!(flavor, None);
                assert_eq!(timeout_secs, 3600);
            }
            _ => panic!("wrong variant"),
        }
        match parse(&[
            "geectl",
            "local-build",
            "chroot",
            "/tmp/pkg",
            "--project",
            "home:foo",
            "--repo",
            "standard",
            "--arch",
            "x86_64",
            "--flavor",
            "lib",
            "--timeout-secs",
            "60",
        ]) {
            LocalBuildCommand::Chroot {
                arch,
                flavor,
                timeout_secs,
                ..
            } => {
                assert_eq!(arch, "x86_64");
                assert_eq!(flavor.as_deref(), Some("lib"));
                assert_eq!(timeout_secs, 60);
            }
            _ => panic!("wrong variant"),
        }
        assert!(Cli::try_parse_from(["geectl", "local-build", "chroot", "/tmp/pkg"]).is_err());
        assert!(Cli::try_parse_from([
            "geectl",
            "local-build",
            "chroot",
            "/tmp/pkg",
            "--project",
            "home:foo",
        ])
        .is_err());
    }

    #[test]
    fn parse_stdin_commands_splits_lines_and_drops_blanks() {
        assert_eq!(
            parse_stdin_commands("echo hi\n\n  rpm -qa \n"),
            vec!["echo hi".to_string(), "rpm -qa".to_string()]
        );
        assert!(parse_stdin_commands("").is_empty());
        assert!(parse_stdin_commands("\n  \n").is_empty());
        assert_eq!(
            parse_stdin_commands("a\r\nb\r\n"),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn stdin_commands_cap_bytes() {
        // One byte past the cap is invalid input, not a read: the
        // cap is enforced during the read, so an endless pipe
        // stops at the cap.
        let over = "x".repeat(MAX_STDIN_COMMANDS_BYTES + 1);
        let err = read_commands_from(std::io::Cursor::new(over)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        // Exactly at the cap reads fine.
        let at = "x".repeat(MAX_STDIN_COMMANDS_BYTES);
        let commands = read_commands_from(std::io::Cursor::new(at)).unwrap();
        assert_eq!(commands.len(), 1);
    }

    #[test]
    fn stdin_commands_cap_count() {
        // One command past the count cap is invalid input.
        let over = "x\n".repeat(MAX_STDIN_COMMANDS + 1);
        let err = read_commands_from(std::io::Cursor::new(over)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        // Exactly at the cap reads fine.
        let at = "x\n".repeat(MAX_STDIN_COMMANDS);
        let commands = read_commands_from(std::io::Cursor::new(at)).unwrap();
        assert_eq!(commands.len(), MAX_STDIN_COMMANDS);
    }

    fn chroot_record() -> ChrootRecord {
        ChrootRecord {
            root: PathBuf::from("/tmp/root"),
            commands: vec!["echo hi".to_string(), "rpm -qa".to_string()],
            dry_run: false,
            result: Some(ShellResult {
                exit_code: 0,
                stdout: "shell output: echo hi\n".to_string(),
                stderr: String::new(),
            }),
        }
    }

    fn dry_run_chroot_record() -> ChrootRecord {
        ChrootRecord {
            root: PathBuf::from("/tmp/root"),
            commands: vec!["echo hi".to_string(), "rpm -qa".to_string()],
            dry_run: true,
            result: None,
        }
    }

    #[test]
    fn chroot_renders_plan_and_result() {
        let text = render_chroot(&dry_run_chroot_record());
        assert!(text.contains("root: /tmp/root"));
        assert!(text.contains("run: echo hi"));
        assert!(text.contains("run: rpm -qa"));
        assert!(!text.contains("exit:"));
        let text = render_chroot(&chroot_record());
        assert!(text.contains("root: /tmp/root"));
        assert!(text.contains("run: echo hi"));
        assert!(text.contains("run: rpm -qa"));
        assert!(text.contains("exit: 0"));
        assert!(text.contains("shell output: echo hi"));
        let mut failed = chroot_record();
        failed.result.as_mut().unwrap().exit_code = 1;
        failed.result.as_mut().unwrap().stdout = String::new();
        failed.result.as_mut().unwrap().stderr = "rpm: no such package\n".to_string();
        let text = render_chroot(&failed);
        assert!(text.contains("exit: 1"));
        assert!(text.contains("rpm: no such package"));
    }

    /// The agreement test: every identifying field the JSON payload
    /// carries for the chroot — the root, the commands, the exit
    /// code, and the captured stdout and stderr — appears in the
    /// text rendering. The expected fields come from the payload
    /// itself, not from literals repeated from the rendering
    /// tests, so a payload field the text drops fails here.
    #[test]
    fn chroot_text_carries_every_identifying_field() {
        let record = chroot_record();
        let payload = serde_json::to_value(&record).expect("record serializes");
        let text = render_chroot(&record);
        let root = payload["root"].as_str().expect("root is a string");
        assert!(
            text.contains(root),
            "chroot text misses root {root}:\n{text}"
        );
        for command in payload["commands"]
            .as_array()
            .expect("commands is an array")
        {
            let command = command.as_str().expect("command is a string");
            assert!(
                text.contains(command),
                "chroot text misses command {command}:\n{text}"
            );
        }
        let result = &payload["result"];
        assert!(text.contains(&format!("exit: {}", result["exit_code"])));
        assert!(text.contains(result["stdout"].as_str().expect("stdout is a string")));
        let mut failed = chroot_record();
        failed.result.as_mut().unwrap().stdout = String::new();
        failed.result.as_mut().unwrap().stderr = "rpm: no such package\n".to_string();
        let payload = serde_json::to_value(&failed).expect("record serializes");
        let text = render_chroot(&failed);
        assert!(text.contains(
            payload["result"]["stderr"]
                .as_str()
                .expect("stderr is a string")
        ));
    }

    #[test]
    fn prune_parses_defaults_and_apply() {
        match parse(&["geectl", "local-build", "prune"]) {
            LocalBuildCommand::Prune { apply } => assert!(!apply),
            _ => panic!("wrong variant"),
        }
        match parse(&["geectl", "local-build", "prune", "--apply"]) {
            LocalBuildCommand::Prune { apply } => assert!(apply),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn periodic_leaves_parse() {
        assert!(matches!(
            parse(&["geectl", "local-build", "periodic-install"]),
            LocalBuildCommand::PeriodicInstall
        ));
        assert!(matches!(
            parse(&["geectl", "local-build", "periodic-uninstall"]),
            LocalBuildCommand::PeriodicUninstall
        ));
        assert!(matches!(
            parse(&["geectl", "local-build", "periodic-status"]),
            LocalBuildCommand::PeriodicStatus
        ));
    }

    fn eviction_report(
        ran: bool,
        freed: u64,
        items: Vec<EvictedItem>,
        shortfall: u64,
        failures: Vec<String>,
    ) -> EvictionReport {
        EvictionReport {
            ran,
            cap_bytes: 20 * 1024 * 1024 * 1024,
            low_water_bytes: 16 * 1024 * 1024 * 1024,
            bytes_before: 21 * 1024 * 1024 * 1024,
            bytes_after: 21 * 1024 * 1024 * 1024 - freed,
            freed_bytes: freed,
            evicted: items,
            failures,
            protected_bytes: 0,
            shortfall_bytes: shortfall,
        }
    }

    #[test]
    fn render_prune_reports_plan_applied_and_nothing() {
        let empty = eviction_report(false, 0, vec![], 0, vec![]);
        assert_eq!(render_prune(&empty, false), "eviction: nothing to evict");
        let plan = eviction_report(
            true,
            4096,
            vec![
                EvictedItem {
                    kind: "pkg".to_string(),
                    path: "/cache/pkg/foo.rpm".to_string(),
                    bytes: 1024,
                    reason: "lru".to_string(),
                },
                EvictedItem {
                    kind: "sccache".to_string(),
                    path: "/cache/sccache/bar".to_string(),
                    bytes: 3072,
                    reason: "lru".to_string(),
                },
            ],
            512,
            vec![],
        );
        assert_eq!(
            render_prune(&plan, false),
            "eviction: plan, would free 4096 bytes (2 items)\n\
             eviction: pkg /cache/pkg/foo.rpm (1024 bytes, lru)\n\
             eviction: sccache /cache/sccache/bar (3072 bytes, lru)\n\
             eviction shortfall: 512 bytes"
        );
        assert_eq!(
            render_prune(&plan, true),
            "eviction: ran, freed 4096 bytes (2 items)\n\
             eviction: pkg /cache/pkg/foo.rpm (1024 bytes, lru)\n\
             eviction: sccache /cache/sccache/bar (3072 bytes, lru)\n\
             eviction shortfall: 512 bytes"
        );
    }

    #[test]
    fn prune_live_note_names_apply_only_without_it() {
        assert_eq!(
            prune_live_note(true, false),
            Some("note: --live does not delete here; use --apply to delete")
        );
        assert_eq!(prune_live_note(true, true), None);
        assert_eq!(prune_live_note(false, false), None);
        assert_eq!(prune_live_note(false, true), None);
    }

    #[test]
    fn render_prune_reports_a_lock_skip() {
        let skipped = eviction_report(
            false,
            0,
            vec![],
            0,
            vec!["eviction lock held by another build; eviction skipped".to_string()],
        );
        assert_eq!(
            render_prune(&skipped, true),
            "eviction: skipped, another eviction holds the lock\n\
             eviction failures: eviction lock held by another build; eviction skipped"
        );
    }

    #[test]
    fn render_periodic_outcome_reports_plan_and_run() {
        let planned = PeriodicOutcome {
            platform: PeriodicOs::Linux,
            label: "geectl-prune.timer".to_string(),
            dry_run: true,
            files: vec![PathBuf::from(
                "/home/tester/.config/systemd/user/geectl-prune.timer",
            )],
            commands: vec![vec![
                "systemctl".to_string(),
                "--user".to_string(),
                "enable".to_string(),
                "--now".to_string(),
                "geectl-prune.timer".to_string(),
            ]],
        };
        assert_eq!(
            render_periodic_outcome(&planned, true),
            "task: geectl-prune.timer\n\
             would write: /home/tester/.config/systemd/user/geectl-prune.timer\n\
             would run: systemctl --user enable --now geectl-prune.timer"
        );
        let removed = PeriodicOutcome {
            platform: PeriodicOs::Linux,
            label: "geectl-prune.timer".to_string(),
            dry_run: false,
            files: vec![PathBuf::from(
                "/home/tester/.config/systemd/user/geectl-prune.timer",
            )],
            commands: vec![vec![
                "systemctl".to_string(),
                "--user".to_string(),
                "disable".to_string(),
                "--now".to_string(),
                "geectl-prune.timer".to_string(),
            ]],
        };
        assert_eq!(
            render_periodic_outcome(&removed, false),
            "task: geectl-prune.timer\n\
             removed: /home/tester/.config/systemd/user/geectl-prune.timer\n\
             ran: systemctl --user disable --now geectl-prune.timer"
        );
    }

    #[test]
    fn render_periodic_status_reports_states() {
        let status = PeriodicStatus {
            platform: PeriodicOs::Linux,
            label: "geectl-prune.timer".to_string(),
            installed: Some(true),
            enabled: Some(true),
            active: None,
            files: vec![],
        };
        assert_eq!(
            render_periodic_status(&status),
            "task: geectl-prune.timer\ninstalled: yes\nenabled: yes\nactive: unknown"
        );
    }

    #[test]
    fn render_periodic_status_reports_unknown_installed() {
        let status = PeriodicStatus {
            platform: PeriodicOs::Linux,
            label: "geectl-prune.timer".to_string(),
            installed: None,
            enabled: None,
            active: None,
            files: vec![],
        };
        assert_eq!(
            render_periodic_status(&status),
            "task: geectl-prune.timer\ninstalled: unknown\nenabled: unknown\nactive: unknown"
        );
    }

    fn dry_run_record() -> ServiceRunRecord {
        ServiceRunRecord {
            mode: "run".to_string(),
            ran: vec![],
            image_digest: None,
            dry_run: true,
            plan: Some(ServiceRunPlan {
                image: "registry.opensuse.org/opensuse/tumbleweed".to_string(),
                services: vec![PlannedService {
                    name: "alpha".to_string(),
                    mode: String::new(),
                    argv: vec![
                        "/usr/lib/obs/service/alpha".to_string(),
                        "--outdir".to_string(),
                        "/work/.geectl-service-alpha-1-0".to_string(),
                    ],
                }],
            }),
        }
    }

    #[test]
    fn service_run_renders_plan_and_outcomes() {
        let text = render_service_run(&dry_run_record());
        assert!(text.contains("image: registry.opensuse.org/opensuse/tumbleweed"));
        assert!(text.contains("alpha: /usr/lib/obs/service/alpha --outdir"));
        let record = ServiceRunRecord {
            mode: "run".to_string(),
            ran: vec![
                ServiceRunOutcome {
                    name: "alpha".to_string(),
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                },
                ServiceRunOutcome {
                    name: "beta".to_string(),
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                },
            ],
            image_digest: Some("sha256:abc".to_string()),
            dry_run: false,
            plan: None,
        };
        let text = render_service_run(&record);
        assert!(text.contains("alpha: exit 0"));
        assert!(text.contains("beta: exit 0"));
        assert!(text.contains("image: sha256:abc"));
        let empty = ServiceRunRecord {
            mode: "run".to_string(),
            ran: vec![],
            image_digest: None,
            dry_run: false,
            plan: None,
        };
        assert_eq!(render_service_run(&empty), "no services selected");
    }

    #[test]
    fn precommit_renders_the_script_line() {
        let record = PrecommitRecord {
            services: dry_run_record(),
            pre_checkin: PrecheckinOutcome {
                ran: false,
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
            },
            dry_run: true,
        };
        let text = render_precommit(&record);
        assert!(text.contains("alpha: /usr/lib/obs/service/alpha"));
        assert!(text.contains("pre_checkin.sh: not evaluated (dry run)"));
        let record = PrecommitRecord {
            services: ServiceRunRecord {
                mode: "precommit".to_string(),
                ran: vec![],
                image_digest: None,
                dry_run: false,
                plan: None,
            },
            pre_checkin: PrecheckinOutcome {
                ran: true,
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            },
            dry_run: false,
        };
        let text = render_precommit(&record);
        assert!(text.contains("no services selected"));
        assert!(text.contains("pre_checkin.sh: ran, exit 0"));
        let record = PrecommitRecord {
            services: ServiceRunRecord {
                mode: "precommit".to_string(),
                ran: vec![],
                image_digest: None,
                dry_run: false,
                plan: None,
            },
            pre_checkin: PrecheckinOutcome {
                ran: false,
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
            },
            dry_run: false,
        };
        assert!(render_precommit(&record).contains("pre_checkin.sh: not present"));
    }

    #[test]
    fn spec_cleaner_renders_plan_and_exit() {
        let result = SpecCleanerResult {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
            image_digest: None,
            dry_run: true,
            plan: Some(vec!["spec-cleaner".to_string(), "foo.spec".to_string()]),
        };
        assert_eq!(render_spec_cleaner(&result), "plan: spec-cleaner foo.spec");
        let result = SpecCleanerResult {
            exit_code: 0,
            stdout: "cleaned foo.spec\n".to_string(),
            stderr: String::new(),
            image_digest: Some("sha256:abc".to_string()),
            dry_run: false,
            plan: None,
        };
        let text = render_spec_cleaner(&result);
        assert!(text.contains("exit: 0"));
        assert!(text.contains("cleaned foo.spec"));
    }

    fn build_record() -> BuildRecord {
        BuildRecord {
            verdict: "green".to_string(),
            exit_code: 0,
            log_path: PathBuf::from("/tmp/root/.build.log"),
            rpms: vec![PathBuf::from("/tmp/root/RPMS/foo-1.0-1.x86_64.rpm")],
            kept_rpms: vec![PathBuf::from("/tmp/keep/foo-1.0-1.x86_64.rpm")],
            rpmlint: RpmlintSummary {
                errors: 0,
                warnings: 1,
                badness: 10,
            },
            checks: Some(CheckSummary {
                passed: 12,
                failed: 0,
                total: 12,
            }),
            image_digest: Some("sha256:abc".to_string()),
            engine_version: Some("20261016".to_string()),
            root_path: PathBuf::from("/tmp/root"),
            root_reused: true,
            cache: CacheStats {
                pkg_hits: 3,
                pkg_misses: 0,
                pkg_preferred: 1,
                sccache_hits: Some(5),
            },
            eviction: EvictionReport::default(),
            preinstall_image: Some("preinstallimage.x86_64.rpm".to_string()),
            trusted_foreign_projects: Vec::new(),
            dry_run: false,
            plan: None,
        }
    }

    fn dry_run_build_record() -> BuildRecord {
        BuildRecord {
            verdict: "dry-run".to_string(),
            exit_code: 0,
            log_path: PathBuf::from("/tmp/root/.build.log"),
            rpms: vec![],
            kept_rpms: vec![],
            rpmlint: RpmlintSummary::default(),
            checks: None,
            image_digest: None,
            engine_version: Some("20261016".to_string()),
            root_path: PathBuf::from("/tmp/root"),
            root_reused: false,
            cache: CacheStats::default(),
            eviction: EvictionReport::default(),
            preinstall_image: None,
            trusted_foreign_projects: Vec::new(),
            dry_run: true,
            plan: Some(EnginePlan {
                image: "registry.opensuse.org/opensuse/tumbleweed".to_string(),
                root: PathBuf::from("/tmp/root"),
                repo: "standard".to_string(),
                arch: "x86_64".to_string(),
                flavor: None,
                alternative_project: None,
                trust_all_projects: false,
                keep_rpms: None,
                prefer_packages: None,
                argv: vec![
                    "build".to_string(),
                    "--root=/tmp/root".to_string(),
                    "foo.spec".to_string(),
                ],
            }),
        }
    }

    #[test]
    fn build_renders_plan_and_record() {
        let text = render_build(&dry_run_build_record());
        assert!(text.contains("plan: build --root=/tmp/root foo.spec"));
        assert!(text.contains("image: registry.opensuse.org/opensuse/tumbleweed"));
        assert!(text.contains("root: /tmp/root"));
        assert!(text.contains("repo: standard"));
        assert!(text.contains("arch: x86_64"));
        let mut flagged = dry_run_build_record();
        let plan = flagged.plan.as_mut().unwrap();
        plan.alternative_project = Some("openSUSE:Factory".to_string());
        plan.keep_rpms = Some(PathBuf::from("/tmp/keep"));
        plan.prefer_packages = Some(PathBuf::from("/tmp/prefer"));
        plan.trust_all_projects = true;
        let text = render_build(&flagged);
        assert!(text.contains("alternative-project: openSUSE:Factory"));
        assert!(text.contains("trust-all-projects: true"));
        assert!(text.contains("keep-rpms: /tmp/keep"));
        assert!(text.contains("prefer-packages: /tmp/prefer"));
        let mut record = dry_run_build_record();
        record.plan = None;
        assert_eq!(render_build(&record), "no plan");
        let text = render_build(&build_record());
        assert!(text.contains("verdict: green"));
        assert!(text.contains("kept-rpm: /tmp/keep/foo-1.0-1.x86_64.rpm"));
        assert!(text.contains("cache: pkg 3 hits, 0 misses, 1 preferred, sccache 5 hits"));
        assert!(text.contains("exit: 0"));
        assert!(text.contains("log: /tmp/root/.build.log"));
        assert!(text.contains("rpm: /tmp/root/RPMS/foo-1.0-1.x86_64.rpm"));
        assert!(text.contains("rpmlint: 0 errors, 1 warnings (badness 10)"));
        assert!(text.contains("check: 12 passed, 0 failed of 12"));
        assert!(text.contains("image: sha256:abc"));
        assert!(text.contains("engine: 20261016"));
        assert!(text.contains("root: /tmp/root (reused)"));
        assert!(text.contains("preinstall-image: preinstallimage.x86_64.rpm"));
        assert!(!text.contains("eviction:"));
        let mut record = build_record();
        record.verdict = "failed".to_string();
        record.exit_code = 1;
        record.rpms = vec![];
        record.checks = None;
        record.preinstall_image = None;
        let text = render_build(&record);
        assert!(text.contains("verdict: failed"));
        assert!(text.contains("exit: 1"));
        assert!(text.contains("rpms: none"));
        assert!(!text.contains("check:"));
        assert!(!text.contains("preinstall-image:"));
        let mut record = build_record();
        record.eviction = EvictionReport {
            ran: true,
            freed_bytes: 4096,
            evicted: vec![
                EvictedItem {
                    kind: "root".to_string(),
                    path: "/tmp/old-root".to_string(),
                    bytes: 3072,
                    reason: "lru".to_string(),
                },
                EvictedItem {
                    kind: "pkg".to_string(),
                    path: "/tmp/old.rpm".to_string(),
                    bytes: 1024,
                    reason: "lru".to_string(),
                },
            ],
            failures: vec!["cannot remove /tmp/stuck: permission denied".to_string()],
            shortfall_bytes: 512,
            ..EvictionReport::default()
        };
        let text = render_build(&record);
        assert!(text.contains("eviction: ran, freed 4096 bytes (2 items)"));
        assert!(text.contains("eviction shortfall: 512 bytes"));
        assert!(text.contains("eviction failures: cannot remove /tmp/stuck: permission denied"));
        let mut record = build_record();
        record.image_digest = None;
        record.engine_version = None;
        record.root_reused = false;
        record.cache.sccache_hits = None;
        let text = render_build(&record);
        assert!(text.contains("image: <unknown>"));
        assert!(text.contains("engine: <unknown>"));
        assert!(text.contains("root: /tmp/root"));
        assert!(!text.contains("(reused)"));
        assert!(text.contains("cache: pkg 3 hits, 0 misses"));
        assert!(!text.contains("sccache"));
    }

    /// The agreement test: every identifying field the JSON payload
    /// carries for the build — the verdict, exit code, log path,
    /// RPMs, rpmlint and check summaries, image digest, engine
    /// version, root, cache and eviction summaries, and the plan
    /// argv, repo, and arch — appears in the text rendering.
    /// The expected fields come from the payload itself, not from
    /// literals repeated from the rendering tests, so a payload
    /// field the text drops fails here.
    #[test]
    fn build_text_carries_every_identifying_field() {
        let mut record = build_record();
        record.eviction = EvictionReport {
            ran: true,
            freed_bytes: 4096,
            evicted: vec![EvictedItem {
                kind: "root".to_string(),
                path: "/tmp/old-root".to_string(),
                bytes: 4096,
                reason: "lru".to_string(),
            }],
            failures: vec!["cannot remove /tmp/stuck: permission denied".to_string()],
            ..EvictionReport::default()
        };
        let payload = serde_json::to_value(&record).expect("record serializes");
        let text = render_build(&record);
        for field in ["verdict", "log_path", "root_path"] {
            let value = payload[field].as_str().expect("field is a string");
            assert!(
                text.contains(value),
                "build text misses {field}: {value}:\n{text}"
            );
        }
        assert!(text.contains(&format!("exit: {}", payload["exit_code"])));
        for rpm in payload["rpms"].as_array().expect("rpms is an array") {
            let rpm = rpm.as_str().expect("rpm is a string");
            assert!(text.contains(rpm), "build text misses rpm {rpm}:\n{text}");
        }
        let rpmlint = &payload["rpmlint"];
        assert!(text.contains(&format!(
            "{} errors, {} warnings (badness {})",
            rpmlint["errors"], rpmlint["warnings"], rpmlint["badness"]
        )));
        let checks = &payload["checks"];
        assert!(text.contains(&format!(
            "{} passed, {} failed of {}",
            checks["passed"], checks["failed"], checks["total"]
        )));
        assert!(text.contains(
            payload["image_digest"]
                .as_str()
                .expect("digest is a string")
        ));
        assert!(text.contains(
            payload["engine_version"]
                .as_str()
                .expect("version is a string")
        ));
        assert!(text.contains(
            payload["preinstall_image"]
                .as_str()
                .expect("image is a string")
        ));
        let eviction = &payload["eviction"];
        assert!(text.contains(&format!("freed {} bytes", eviction["freed_bytes"])));
        assert!(text.contains(&format!(
            "({} items)",
            eviction["evicted"]
                .as_array()
                .expect("evicted is an array")
                .len()
        )));
        for failure in eviction["failures"]
            .as_array()
            .expect("failures is an array")
        {
            let failure = failure.as_str().expect("failure is a string");
            assert!(
                text.contains(failure),
                "build text misses eviction failure {failure}:\n{text}"
            );
        }
        let cache = &payload["cache"];
        assert!(text.contains(&format!(
            "pkg {} hits, {} misses",
            cache["pkg_hits"], cache["pkg_misses"]
        )));
        if let Some(hits) = cache["sccache_hits"].as_u64() {
            assert!(text.contains(&format!("sccache {hits} hits")));
        }
        // The dry-run plan: the argv carries the invocation.
        let plan_record = dry_run_build_record();
        let plan_payload = serde_json::to_value(&plan_record).expect("record serializes");
        let plan_text = render_build(&plan_record);
        for arg in plan_payload["plan"]["argv"]
            .as_array()
            .expect("argv is an array")
        {
            let arg = arg.as_str().expect("argv element is a string");
            assert!(
                plan_text.contains(arg),
                "plan text misses argv {arg}:\n{plan_text}"
            );
        }
        assert!(plan_text.contains(
            plan_payload["plan"]["repo"]
                .as_str()
                .expect("repo is a string")
        ));
        assert!(plan_text.contains(
            plan_payload["plan"]["arch"]
                .as_str()
                .expect("arch is a string")
        ));
    }

    #[test]
    fn log_parses_coordinates_and_flags() {
        match parse(&[
            "geectl",
            "local-build",
            "log",
            "standard",
            "x86_64",
            "/tmp/pkg",
            "--project",
            "home:foo",
        ]) {
            LocalBuildCommand::Log {
                repo,
                arch,
                spec,
                project,
                flavor,
                tail,
                full,
            } => {
                assert_eq!(repo, "standard");
                assert_eq!(arch, "x86_64");
                assert_eq!(spec, PathBuf::from("/tmp/pkg"));
                assert_eq!(project, "home:foo");
                assert_eq!(flavor, None);
                assert_eq!(tail, None);
                assert!(!full);
            }
            _ => panic!("wrong variant"),
        }
        match parse(&[
            "geectl",
            "local-build",
            "log",
            "standard",
            "x86_64",
            "/tmp/pkg",
            "--project",
            "home:foo",
            "--flavor",
            "lib",
            "--tail",
            "50",
            "--full",
        ]) {
            LocalBuildCommand::Log {
                flavor, tail, full, ..
            } => {
                assert_eq!(flavor.as_deref(), Some("lib"));
                assert_eq!(tail, Some(50));
                assert!(full);
            }
            _ => panic!("wrong variant"),
        }
        // Coordinates are required.
        assert!(Cli::try_parse_from(["geectl", "local-build", "log"]).is_err());
        assert!(Cli::try_parse_from([
            "geectl",
            "local-build",
            "log",
            "standard",
            "x86_64",
            "/tmp/pkg",
        ])
        .is_err());
        // A negative tail is not a line count: usize rejects it at parse.
        assert!(Cli::try_parse_from([
            "geectl",
            "local-build",
            "log",
            "standard",
            "x86_64",
            "/tmp/pkg",
            "--project",
            "home:foo",
            "--tail",
            "-1",
        ])
        .is_err());
    }

    fn build_log_fixture() -> BuildLog {
        BuildLog {
            log_path: PathBuf::from("/tmp/root/.build.log"),
            lines: vec![
                "first line".to_string(),
                "finished \"build foo.spec\"".to_string(),
            ],
            truncated: false,
            verdict: Some("green".to_string()),
            exit_code: Some(0),
        }
    }

    #[test]
    fn log_renders_the_lines_verbatim() {
        let (_, text) = log_result(
            &PathBuf::from("/tmp/pkg"),
            "home:foo",
            "standard",
            "x86_64",
            &None,
            None,
            false,
            &build_log_fixture(),
        );
        assert_eq!(text, "first line\nfinished \"build foo.spec\"");
        let empty = BuildLog {
            lines: vec![],
            ..build_log_fixture()
        };
        let (_, text) = log_result(
            &PathBuf::from("/tmp/pkg"),
            "home:foo",
            "standard",
            "x86_64",
            &None,
            None,
            false,
            &empty,
        );
        assert_eq!(text, "");
    }

    /// The agreement test: the text is the log lines verbatim, so
    /// every payload line appears in the text in order; the verdict,
    /// exit code, log path, and truncation flag are a DOCUMENTED
    /// rule-4 omission — they ride in the JSON payload only, never
    /// in the text.
    #[test]
    fn log_text_carries_every_line_and_the_payload_carries_the_rest() {
        let log = build_log_fixture();
        let (payload, text) = log_result(
            &PathBuf::from("/tmp/pkg"),
            "home:foo",
            "standard",
            "x86_64",
            &None,
            None,
            false,
            &log,
        );
        assert_eq!(payload["spec"], "/tmp/pkg");
        assert_eq!(payload["project"], "home:foo");
        assert_eq!(payload["repo"], "standard");
        assert_eq!(payload["arch"], "x86_64");
        assert_eq!(payload["flavor"], Value::Null);
        assert_eq!(payload["tail"], Value::Null);
        assert_eq!(payload["full"], false);
        assert_eq!(payload["log"]["log_path"], "/tmp/root/.build.log");
        assert_eq!(payload["log"]["verdict"], "green");
        assert_eq!(payload["log"]["exit_code"], 0);
        assert_eq!(payload["log"]["truncated"], false);
        let lines = payload["log"]["lines"]
            .as_array()
            .expect("lines is an array");
        assert_eq!(
            text,
            lines
                .iter()
                .map(|l| l.as_str().expect("line is a string"))
                .collect::<Vec<_>>()
                .join("\n"),
            "log text is not the payload lines verbatim"
        );
        // The truncated case: the flag is set in the payload, and
        // the text stays verbatim (no truncation notice is prepended).
        let mut truncated = build_log_fixture();
        truncated.truncated = true;
        let (payload, text) = log_result(
            &PathBuf::from("/tmp/pkg"),
            "home:foo",
            "standard",
            "x86_64",
            &Some("lib".to_string()),
            Some(50),
            true,
            &truncated,
        );
        assert_eq!(payload["log"]["truncated"], true);
        assert_eq!(payload["flavor"], "lib");
        assert_eq!(payload["tail"], 50);
        assert_eq!(payload["full"], true);
        assert!(!text.contains("truncated"));
    }
}
