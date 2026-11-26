//! `meta` — the unified meta-write command group: set the meta of
//! an OBS project, an OBS package, or a Gitea repository as one
//! operation.
//!
//! Thin over `MetaClient` like the OBS groups are over `ObsClient`:
//! the flags mirror the library parameters, the JSON payload carries
//! the typed values, and the text renderer prints the same values in
//! a human-readable form.
//!
//! One operation for both backends: the target selects the backend
//! — `--project` (with `--package` for a package) writes OBS meta,
//! `--repo` writes Gitea repo settings — and the fields are the
//! same on either arm: `--description` and `--url` (the repository
//! website on the Gitea arm). Only the given fields change; the
//! rest of the meta is kept as it is. There is no flag to clear a
//! field, and no flag names a field the other backend cannot set:
//! the OBS title, scmsync, devel project, and persons stay on
//! `remote set-project-meta` / `remote set-package-meta`.
//!
//! The write is dry-run by default and mutates only with the
//! global `--live`. A write on an arm needs that arm's credentials
//! — OBS credentials for `--project`, the Gitea Bearer <redacted>
//! for `--repo` — and fails closed naming them when they are
//! missing. In dry-run the OBS client builds without credentials
//! (the plan prints without them), like the request group; the
//! Gitea client still needs its credential to be built, so a
//! dry-run Gitea write without one fails closed naming it, like
//! the gitea group. The Gitea token resolves per URL — the
//! matching alias's variables from the config file's `[instances]`
//! table, else the plain variables — so the leaf works with the
//! credential resolution on main; it assumes no login scheme
//! beyond that.
//!
//! Coordinate header (design rule 1): every text rendering opens
//! with one header line echoing the query coordinates (`project:
//! …`, `project: … package: …`, `owner: … repo: …`).
//!
//! Documented omissions (not gaps): the text carries the fields
//! as `name: value` entries, like the `remote` set-meta leaves;
//! the JSON payload remains the complete record in every case.

use crate::auth::{Credentials, EnvObsAuth};
use crate::config;
use crate::debug::Debug;
use crate::error::{ErrorKind, GeectlError};
use crate::gitea::{parse_gitea_repo, GiteaClient};
use crate::http::{Timeout, UreqClient};
use crate::meta::{MetaClient, MetaFields, MetaTarget, SetMeta};
use crate::obs::{DryRun, ObsClient};
use clap::{Args, Subcommand};
use serde_json::{json, Value};

/// The `meta` subcommands. One operation for both backends: the
/// target selects the backend.
#[derive(Debug, Subcommand)]
pub enum MetaCommand {
    /// Set the meta of an OBS project, an OBS package, or a Gitea
    /// repository: the description and/or URL. The target selects
    /// the backend: `--project` (with `--package` for a package)
    /// writes OBS meta, `--repo` writes Gitea repo settings. A
    /// mutation: dry-run is the default (only the plan is
    /// printed); `--live` performs the write. Only the given
    /// fields change; the rest of the meta is kept as it is. There
    /// is no flag to clear a field.
    Set(SetMetaArgs),
}

/// Arguments for `meta set`.
#[derive(Debug, Args)]
pub struct SetMetaArgs {
    /// The OBS project whose meta to set.
    #[arg(long, conflicts_with = "repo")]
    project: Option<String>,
    /// The OBS package in `--project` whose meta to set; the
    /// project meta when omitted.
    #[arg(long, requires = "project")]
    package: Option<String>,
    /// The Gitea repository, as `owner/repo`, whose settings to set.
    #[arg(long, conflicts_with = "project")]
    repo: Option<String>,
    /// A short description.
    #[arg(long)]
    description: Option<String>,
    /// A URL with more information; the repository website on the
    /// Gitea arm.
    #[arg(long)]
    url: Option<String>,
}

/// Run a `meta` command, returning the JSON payload and its text
/// rendering.
pub fn run_meta(
    command: MetaCommand,
    debug: &Debug,
    live: bool,
    instance: Option<&str>,
) -> Result<(Value, String), GeectlError> {
    match command {
        MetaCommand::Set(args) => {
            // The target and the fields are validated before any
            // credential resolution, so an invalid target or an
            // empty write fails as InvalidInput whether or not
            // credentials are configured (the pull-request group
            // validates its handles this way).
            let target = set_meta_target(&args)?;
            let fields = MetaFields {
                description: args.description.clone(),
                url: args.url.clone(),
            };
            fields.validate()?;
            let field_list = fields.field_list();
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            let set = match &target {
                MetaTarget::ObsProject { .. } | MetaTarget::ObsPackage { .. } => {
                    with_obs(debug, live, instance, |obs| {
                        let client = MetaClient::new(Some(obs), None);
                        client.set_meta(&target, &fields, &dry_run)
                    })?
                }
                MetaTarget::GiteaRepo { .. } => with_gitea(debug, instance, |gitea| {
                    let client = MetaClient::new(None, Some(gitea));
                    client.set_meta(&target, &fields, &dry_run)
                })?,
            };
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(meta_set_result(&set, &field_list, dry_run.enabled(), plan))
        }
    }
}

/// The target of a `meta set` call: `--project` (with `--package`
/// for a package) or `--repo`. Neither target flag is an input
/// error naming both; both together is refused by clap.
fn set_meta_target(args: &SetMetaArgs) -> Result<MetaTarget, GeectlError> {
    if let Some(project) = &args.project {
        return Ok(match &args.package {
            Some(package) => MetaTarget::ObsPackage {
                project: project.clone(),
                package: package.clone(),
            },
            None => MetaTarget::ObsProject {
                project: project.clone(),
            },
        });
    }
    if let Some(repo) = &args.repo {
        let (owner, repo) = parse_gitea_repo(repo)?;
        return Ok(MetaTarget::GiteaRepo { owner, repo });
    }
    Err(GeectlError::new(
        ErrorKind::InvalidInput,
        "meta set needs --project or --repo",
    ))
}

/// Build the OBS client from the effective config and the
/// environment credentials for the configured base URL (resolved
/// for the URL from its instance alias — see
/// `auth::resolve_obs_for_url`), wiring the debug sink
/// through. The client borrows the transport, so both live in this
/// scope and the caller runs inside it. Reads pass `live` as true
/// and fail fast on a credential resolution error; an act in
/// dry-run takes the request group's leniency instead — the plan
/// prints without credentials, with the failure on the debug
/// channel and as a stderr warning.
fn with_obs<T>(
    debug: &Debug,
    live: bool,
    instance: Option<&str>,
    f: impl FnOnce(&ObsClient) -> Result<T, GeectlError>,
) -> Result<T, GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let base = config::obs_base_url(instance, &file_config)?;
    // With `--instance` the error propagates even in dry-run: the
    // flagged instance's credentials are strict, and a dry run
    // must not silently plan against an instance it cannot use.
    let resolve = match instance {
        Some(alias) => EnvObsAuth::credentials_for_instance(alias, &file_config.instances),
        None => EnvObsAuth::credentials_for_url(&base, &base, &file_config.instances),
    };
    let credentials = match resolve {
        Ok(credentials) => credentials,
        Err(err) if !live && instance.is_none() => {
            // The dry run prints the plan without credentials, but
            // the failure must not be silent: the same credentials
            // would fail a `--live` run, so the error goes on the
            // debug channel and as a stderr warning.
            debug.emit(&format!(
                "ignoring credential resolution error in dry-run mode: {err}"
            ));
            eprintln!("warning: ignoring credential resolution error in dry-run mode: {err}");
            Credentials::None
        }
        Err(err) => return Err(err),
    };
    let client =
        ObsClient::with_base_and_credentials(&http, &base, credentials)?.with_debug(debug.clone());
    f(&client)
}

/// Run `f` with a `GiteaClient` for the configured instance. The
/// client borrows the transport, so both live in this scope and
/// the caller runs inside it.
fn with_gitea<T>(
    debug: &Debug,
    instance: Option<&str>,
    f: impl FnOnce(&GiteaClient) -> Result<T, GeectlError>,
) -> Result<T, GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = crate::cli_gitea::gitea_client(&http, debug, instance)?;
    f(&client)
}

/// The `meta set` result: the payload carries the backend, the
/// target, and the resulting description and URL; the text states
/// the write, or the dry-run plan.
fn meta_set_result(
    set: &SetMeta,
    fields: &[String],
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let result_line = if dry_run {
        format!(
            "dry-run: would set meta of {} ({})",
            set.target.describe(),
            fields.join("; ")
        )
    } else {
        format!(
            "set meta of {} ({})",
            set.target.describe(),
            fields.join("; ")
        )
    };
    let payload = match &set.target {
        MetaTarget::ObsProject { project } => json!({
            "backend": set.backend.as_str(),
            "project": project,
            "description": set.description,
            "url": set.url,
            "fields": fields,
        }),
        MetaTarget::ObsPackage { project, package } => json!({
            "backend": set.backend.as_str(),
            "project": project,
            "package": package,
            "description": set.description,
            "url": set.url,
            "fields": fields,
        }),
        MetaTarget::GiteaRepo { owner, repo } => json!({
            "backend": set.backend.as_str(),
            "owner": owner,
            "repo": repo,
            "description": set.description,
            "url": set.url,
            "fields": fields,
        }),
    };
    mutation_result(
        payload,
        meta_header(&set.target),
        result_line,
        dry_run,
        plan,
    )
}

/// The coordinate header for a target (design rule 1).
fn meta_header(target: &MetaTarget) -> String {
    match target {
        MetaTarget::ObsProject { project } => format!("project: {project}"),
        MetaTarget::ObsPackage { project, package } => {
            format!("project: {project} package: {package}")
        }
        MetaTarget::GiteaRepo { owner, repo } => format!("owner: {owner} repo: {repo}"),
    }
}

/// Combine a mutation payload and its text lines: `dry_run` joins
/// the payload and goes right after the header line; the plan,
/// when present, joins the payload and follows the header and
/// result lines verbatim — the same shape as the `gitea` group's
/// mutations.
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// A minimal CLI parser holding the `meta` command, for parse
    /// tests: the group parses exactly as the binary parses it.
    #[derive(Debug, Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: MetaCommand,
    }

    fn parse(args: &[&str]) -> MetaCommand {
        TestCli::try_parse_from(std::iter::once("geectl").chain(args.iter().copied()))
            .unwrap()
            .command
    }

    fn parse_err(args: &[&str]) -> clap::Error {
        TestCli::try_parse_from(std::iter::once("geectl").chain(args.iter().copied())).unwrap_err()
    }

    #[test]
    fn meta_set_parses_project_and_package_targets() {
        let MetaCommand::Set(args) = parse(&[
            "set",
            "--project",
            "home:scarabeus_iv",
            "--description",
            "New description",
        ]);
        assert_eq!(args.project.as_deref(), Some("home:scarabeus_iv"));
        assert_eq!(args.package, None);
        let MetaCommand::Set(args) = parse(&[
            "set",
            "--project",
            "home:scarabeus_iv",
            "--package",
            "foo",
            "--url",
            "https://example.com/foo",
        ]);
        assert_eq!(args.package.as_deref(), Some("foo"));
        assert_eq!(args.url.as_deref(), Some("https://example.com/foo"));
    }

    #[test]
    fn meta_set_parses_repo_target() {
        let MetaCommand::Set(args) = parse(&[
            "set",
            "--repo",
            "pool/systemd",
            "--description",
            "New description",
            "--url",
            "https://example.com/systemd",
        ]);
        assert_eq!(args.repo.as_deref(), Some("pool/systemd"));
        assert_eq!(args.project, None);
    }

    #[test]
    fn meta_set_refuses_project_with_repo() {
        let err = parse_err(&[
            "set",
            "--project",
            "home:scarabeus_iv",
            "--repo",
            "pool/systemd",
            "--description",
            "New description",
        ]);
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn meta_set_refuses_package_without_project() {
        let err = parse_err(&[
            "set",
            "--package",
            "foo",
            "--description",
            "New description",
        ]);
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn set_meta_target_needs_a_target_flag() {
        let args = SetMetaArgs {
            project: None,
            package: None,
            repo: None,
            description: Some("New description".to_string()),
            url: None,
        };
        let err = set_meta_target(&args).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err
            .to_string()
            .contains("meta set needs --project or --repo"));
    }

    #[test]
    fn set_meta_target_rejects_a_malformed_repo() {
        let args = SetMetaArgs {
            project: None,
            package: None,
            repo: Some("pool".to_string()),
            description: Some("New description".to_string()),
            url: None,
        };
        let err = set_meta_target(&args).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    fn sample_set(target: MetaTarget, description: Option<&str>, url: Option<&str>) -> SetMeta {
        SetMeta {
            backend: target.backend(),
            target,
            description: description.map(str::to_string),
            url: url.map(str::to_string),
        }
    }

    #[test]
    fn meta_set_result_carries_target_and_fields_in_both_formats() {
        let set = sample_set(
            MetaTarget::ObsProject {
                project: "home:scarabeus_iv".to_string(),
            },
            Some("New description"),
            None,
        );
        let fields = vec!["description: New description".to_string()];
        let (payload, text) = meta_set_result(&set, &fields, true, None);
        assert_eq!(payload["backend"], "obs");
        assert_eq!(payload["project"], "home:scarabeus_iv");
        assert_eq!(payload["description"], "New description");
        assert_eq!(payload["dry_run"], true);
        assert!(text.contains("project: home:scarabeus_iv"));
        assert!(text.contains("dry_run: true"));
        assert!(text.contains(
            "dry-run: would set meta of project home:scarabeus_iv (description: New description)"
        ));
        let (payload, live_text) = meta_set_result(&set, &fields, false, None);
        assert_eq!(payload["dry_run"], false);
        assert!(live_text
            .contains("set meta of project home:scarabeus_iv (description: New description)"));
    }

    #[test]
    fn meta_set_result_carries_gitea_target_and_plan() {
        let set = sample_set(
            MetaTarget::GiteaRepo {
                owner: "pool".to_string(),
                repo: "systemd".to_string(),
            },
            Some("New description"),
            Some("https://example.com/systemd"),
        );
        let fields = vec![
            "description: New description".to_string(),
            "url: https://example.com/systemd".to_string(),
        ];
        let (payload, text) = meta_set_result(
            &set,
            &fields,
            true,
            Some(vec![
                "[dry-run] would: edit repository pool/systemd".to_string()
            ]),
        );
        assert_eq!(payload["backend"], "gitea");
        assert_eq!(payload["owner"], "pool");
        assert_eq!(payload["repo"], "systemd");
        assert_eq!(payload["url"], "https://example.com/systemd");
        assert_eq!(
            payload["plan"],
            json!(["[dry-run] would: edit repository pool/systemd"])
        );
        assert!(text.contains("owner: pool repo: systemd"));
        assert!(text.contains(
            "dry-run: would set meta of repo pool/systemd (description: New description; url: https://example.com/systemd)"
        ));
        assert!(text.contains("[dry-run] would: edit repository pool/systemd"));
    }

    #[test]
    fn meta_set_result_carries_package_target() {
        let set = sample_set(
            MetaTarget::ObsPackage {
                project: "home:scarabeus_iv".to_string(),
                package: "foo".to_string(),
            },
            None,
            Some("https://example.com/foo"),
        );
        let fields = vec!["url: https://example.com/foo".to_string()];
        let (payload, text) = meta_set_result(&set, &fields, false, None);
        assert_eq!(payload["project"], "home:scarabeus_iv");
        assert_eq!(payload["package"], "foo");
        assert!(text.contains("project: home:scarabeus_iv package: foo"));
        assert!(text
            .contains("set meta of package home:scarabeus_iv/foo (url: https://example.com/foo)"));
    }
}
