//! `local-build` — the container service-run command group: run a
//! package's `_service` services, the precommit gate, and
//! `spec-cleaner`, all in the podman build vertical's container.
//!
//! Thin like `service` and `build`: the flags mirror the library
//! parameters, the JSON payload carries the typed record, and the
//! text renderer prints the same values in a human-readable form.
//! The commands make no server calls, so no credentials resolve:
//! a half-set credential env never breaks a local run.
//!
//! Mutations are gated on the global `--live`: without it, each
//! command prints the plan only (the library dry run) and touches
//! nothing. With it, the run executes in the container. A failing
//! service or a failing `pre_checkin.sh` fails the command with a
//! non-zero exit.
//!
//! Coordinate header (design rule 1): every text rendering opens
//! with one header line echoing the query coordinates (`directory:
//! …`, plus `mode: …` and `service: …` when the query carries
//! them).

use clap::Subcommand;
use geectl::auth::Credentials;
use geectl::config;
use geectl::debug::Debug;
use geectl::error::GeectlError;
use geectl::http::{Timeout, UreqClient};
use geectl::local_build::{
    LocalBuild, PrecommitRecord, ServiceRunMode, ServiceRunRecord, ServiceRunRequest,
    SpecCleanerResult,
};
use geectl::obs::{ObsClient, OBS_API_BASE};
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
}

/// Run a `local-build` command, returning the JSON payload and its
/// text rendering.
pub fn run_local_build(
    debug: &Debug,
    command: LocalBuildCommand,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    // No server calls: the service runs never touch OBS, so no
    // credentials resolve — a half-set credential env must not
    // break a local run.
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let effective = config::effective_config(&file_config)?;
    // Unreachable by construction — effective_config always yields
    // Some(obs_url) — but kept as a defensive default rather than an
    // unwrap: a config change must not turn into a panic here.
    let obs_url = effective.config.obs_url.as_deref().unwrap_or(OBS_API_BASE);
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = ObsClient::with_base_and_credentials(&http, obs_url, Credentials::None)?
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
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cli, Command};
    use clap::Parser;
    use geectl::local_build::{
        PlannedService, PrecheckinOutcome, ServiceRunOutcome, ServiceRunPlan,
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
}
