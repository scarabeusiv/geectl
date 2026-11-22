//! `branch` — the branch/copy/link command group: branching,
//! copying, and linking packages, thin over `ObsClient`.
//!
//! Every leaf returns `(payload, text)` — the JSON payload and a human
//! rendering of the same data — per the #51/#52 design: text stays
//! opt-in via `--format text`, stdout carries only the result payload,
//! and nothing here touches raw XML (typed values only).
//!
//! All ten commands take explicit `--project`/`--package` (etc.)
//! flags, always required: no positionals, no working-copy defaults
//! (geectl has no working copy in v1). Mutations are dry-run by
//! default; the global `--live` flag performs them.
//!
//! Text rendering (Tom, 2026-11-21): every command's text starts
//! with one header line echoing its query coordinates
//! (`<command> <project>/<package> [-> <target>]`), so the text
//! always says what was asked. Tables stay narrow as designed
//! (never widened to echo the full payload); anything else the
//! JSON carries but the text does not is a documented omission at
//! the renderer, not a gap — see the per-handler notes.
//!
//! Credentials in dry-run: this group fails fast on a half-configured
//! credential pair (`GEECTL_OBS_USERNAME` with no password source, or
//! vice versa) before any work, in dry-run too — a dry run that cannot
//! be performed live is not a useful preview. An unconfigured pair
//! resolves to `None` and the operations fail closed themselves when
//! they need credentials. The request group differs deliberately: it
//! swallows a half-configured pair in dry-run (see `cli_request`), so
//! a request preview never needs credentials at all.

use clap::{Args, Subcommand};
use geectl::auth::{same_api_url, EnvObsAuth};
use geectl::config;
use geectl::debug::Debug;
use geectl::error::{ErrorKind, GeectlError};
use geectl::http::{HttpClient, Timeout, UreqClient};
use geectl::obs::{
    BranchOptions, BranchResult, CopyOptions, CopyProjectOptions, DryRun, LinkOptions,
    LinkRevision, ObsClient, RevisionSpec, OBS_API_BASE,
};
use serde_json::{json, Value};

/// The `branch` subcommands. Names are the library operation names
/// in kebab-case, grouped under the top-level noun.
#[derive(Debug, Subcommand)]
pub enum BranchCommand {
    /// Branch a package: create a linked copy under a branch project.
    /// A mutation: dry-run is the default (only the plan is printed);
    /// `--live` performs the branch.
    BranchPackage(BranchPackageArgs),
    /// Copy a package to another project (or another OBS instance
    /// with `--to-apiurl`). A mutation: dry-run is the default.
    CopyPackage(CopyPackageArgs),
    /// Copy a whole project. A mutation: dry-run is the default.
    CopyProject(CopyProjectArgs),
    /// Set (or, without `--devel-project`, remove) the devel project
    /// of a package. A mutation: dry-run is the default.
    SetDevelProject(SetDevelProjectArgs),
    /// Link a package to another package. A mutation: dry-run is the
    /// default.
    LinkPackage(LinkPackageArgs),
    /// Repair a broken link: strip the failing patches from `_link`
    /// server-side and pin it at a revision where it expands.
    /// A mutation: dry-run is the default.
    RepairLink(RepairLinkArgs),
    /// Set the revision a link expands at. A mutation: dry-run is
    /// the default.
    SetLinkRev(SetLinkRevArgs),
    /// Set the revision every linked package in a project expands
    /// at. A mutation: dry-run is the default.
    SetLinkRevProject(SetLinkRevProjectArgs),
    /// Turn a link into a branch: replace the link with a copy of
    /// the linked files. A mutation: dry-run is the default.
    LinkToBranch(LinkToBranchArgs),
    /// Detach a branch: replace the branch link with a copy of the
    /// branched files. A mutation: dry-run is the default.
    DetachBranch(DetachBranchArgs),
}

/// Arguments for `branch branch-package`.
#[derive(Debug, Args)]
pub struct BranchPackageArgs {
    /// The project to branch from.
    #[arg(long)]
    project: String,
    /// The package to branch.
    #[arg(long)]
    package: String,
    /// The branch project. Without it, the server picks the default
    /// (usually `home:<user>:branches:<source project>`).
    #[arg(long)]
    target_project: Option<String>,
    /// The branch package name. Without it, the source package name.
    #[arg(long)]
    target_package: Option<String>,
    /// Ignore the devel project when resolving the link source.
    #[arg(long)]
    no_devel_project: bool,
    /// Branch a package that does not exist yet.
    #[arg(long)]
    new_package: bool,
    /// Overwrite an existing target package.
    #[arg(long)]
    force: bool,
    /// Branch from this source revision instead of the latest.
    #[arg(long)]
    revision: Option<String>,
    /// The link revision the branch records.
    #[arg(long)]
    linkrev: Option<String>,
    /// A comment recorded on the branch.
    #[arg(long)]
    comment: Option<String>,
    /// Do not give the branch read access to the source package's project.
    #[arg(long)]
    noaccess: bool,
    /// Add the source project's repositories to the branch project.
    #[arg(long)]
    add_repositories: bool,
    /// Block the added repositories. Only sent with `--add-repositories`
    /// (clap refuses it without that flag: a block the live run would
    /// silently drop must fail at parse time, not vanish).
    #[arg(long, requires = "add_repositories")]
    add_repositories_block: Option<String>,
    /// Rebuild the added repositories. Only sent with `--add-repositories`
    /// (clap refuses it without that flag, as above).
    #[arg(long, requires = "add_repositories")]
    add_repositories_rebuild: Option<String>,
    /// Extend the branch package names with the source project name.
    #[arg(long)]
    extend_package_names: bool,
    /// Branch as a maintenance incident.
    #[arg(long)]
    maintenance: bool,
}

/// Arguments for `branch copy-package`.
#[derive(Debug, Args)]
pub struct CopyPackageArgs {
    /// The project to copy from.
    #[arg(long)]
    source_project: String,
    /// The package to copy.
    #[arg(long)]
    source_package: String,
    /// The project to copy to.
    #[arg(long)]
    target_project: String,
    /// The package to copy to.
    #[arg(long)]
    target_package: String,
    /// Copy this source revision instead of the latest.
    #[arg(long)]
    revision: Option<String>,
    /// Keep the source package's link when copying a link package.
    #[arg(long)]
    keep_link: bool,
    /// Copy the expanded source (the link resolved) instead of the
    /// link itself.
    #[arg(long)]
    expand: bool,
    /// A comment recorded on the copy.
    #[arg(long)]
    comment: Option<String>,
    /// Do not run source services on the copied files.
    #[arg(long)]
    noservice: bool,
    /// Do not update the source package when copying a link onto its
    /// target.
    #[arg(long)]
    dont_update_source: bool,
    /// Carry the source revision's vrev over to the copy.
    #[arg(long)]
    with_vrev: bool,
    /// Copy to another OBS instance at this API URL instead of the
    /// configured one. The target's credentials come from the
    /// instance alias matching that URL in the config file's
    /// `[instances]` table
    /// (`GEECTL_OBS_USERNAME_<ALIAS>` / `GEECTL_OBS_PASSWORD[_FILE]_<ALIAS>`;
    /// see `auth::resolve_obs_basic_for_url`) and the copy fails
    /// clearly when none are configured — the source instance's
    /// credentials are never forwarded to the target. The copy is
    /// client-side and always expanded; `--keep-link` and the
    /// server-side copy options are refused with it.
    #[arg(long)]
    to_apiurl: Option<String>,
}

/// Arguments for `branch copy-project`.
#[derive(Debug, Args)]
pub struct CopyProjectArgs {
    /// The project to copy from.
    #[arg(long)]
    source_project: String,
    /// The project to copy to.
    #[arg(long)]
    target_project: String,
    /// Copy the built binaries as well. Admin-only on the server.
    #[arg(long)]
    with_binaries: bool,
    /// Copy the project history as well.
    #[arg(long)]
    with_history: bool,
    /// Make the copied project older than the source.
    #[arg(long)]
    make_older: bool,
    /// Make the copied project older than the source's origin.
    #[arg(long)]
    make_origin_older: bool,
    /// Do not run source services on the copied files.
    #[arg(long)]
    noservice: bool,
    /// Re-sign the copied project.
    #[arg(long)]
    resign: bool,
    /// A comment recorded on the copy.
    #[arg(long)]
    comment: Option<String>,
    /// Run the copy synchronously instead of as a background job.
    #[arg(long)]
    no_delay: bool,
}

/// Arguments for `branch set-devel-project`.
#[derive(Debug, Args)]
pub struct SetDevelProjectArgs {
    /// The project holding the package.
    #[arg(long)]
    project: String,
    /// The package whose devel project to set.
    #[arg(long)]
    package: String,
    /// The devel project. Without it, the devel project is removed.
    #[arg(long)]
    devel_project: Option<String>,
    /// The devel package. Without it, the devel project alone is set.
    #[arg(long, requires = "devel_project")]
    devel_package: Option<String>,
}

/// Arguments for `branch link-package`.
#[derive(Debug, Args)]
pub struct LinkPackageArgs {
    /// The project to link from.
    #[arg(long)]
    source_project: String,
    /// The package to link.
    #[arg(long)]
    source_package: String,
    /// The project to link in.
    #[arg(long)]
    target_project: String,
    /// The package to link as.
    #[arg(long)]
    target_package: String,
    /// Replace an existing link in the target package.
    #[arg(long)]
    force: bool,
    /// Pin the link at this source revision (a number or an md5).
    #[arg(long)]
    revision: Option<String>,
    /// The link's vrev attribute.
    #[arg(long)]
    vrev: Option<String>,
    /// The link's cicount attribute: `add`, `copy`, or `local`.
    #[arg(long, value_parser = ["add", "copy", "local"])]
    cicount: Option<String>,
    /// Allow the link to point at a package that does not exist yet.
    #[arg(long)]
    missing_target: bool,
    /// Disable building the target package.
    #[arg(long)]
    disable_build: bool,
    /// Disable publishing the target package.
    #[arg(long)]
    disable_publish: bool,
}

/// Arguments for `branch repair-link`.
#[derive(Debug, Args)]
pub struct RepairLinkArgs {
    /// The project holding the package.
    #[arg(long)]
    project: String,
    /// The package whose link to repair.
    #[arg(long)]
    package: String,
}

/// Arguments for `branch set-link-rev`.
#[derive(Debug, Args)]
pub struct SetLinkRevArgs {
    /// The project holding the package.
    #[arg(long)]
    project: String,
    /// The package whose link revision to set.
    #[arg(long)]
    package: String,
    /// The revision: `latest`, `expanded`, `unset`, a bare revision
    /// (a number or an md5, expanded), or `pinned:<rev>` /
    /// `expanded:<rev>`.
    #[arg(long)]
    revision: String,
}

/// Arguments for `branch set-link-rev-project`.
#[derive(Debug, Args)]
pub struct SetLinkRevProjectArgs {
    /// The project whose linked packages to set.
    #[arg(long)]
    project: String,
    /// The revision: `latest`, `expanded`, or `unset`. A pinned
    /// revision needs a package: use `set-link-rev` for one package
    /// at a time.
    #[arg(long)]
    revision: String,
}

/// Arguments for `branch link-to-branch`.
#[derive(Debug, Args)]
pub struct LinkToBranchArgs {
    /// The project holding the package.
    #[arg(long)]
    project: String,
    /// The package whose link to turn into a branch.
    #[arg(long)]
    package: String,
}

/// Arguments for `branch detach-branch`.
#[derive(Debug, Args)]
pub struct DetachBranchArgs {
    /// The project holding the package.
    #[arg(long)]
    project: String,
    /// The branch package to detach.
    #[arg(long)]
    package: String,
}

/// Run a `branch` subcommand: returns `(payload, text)` — the JSON
/// payload and its human rendering. The caller selects by `--format`;
/// both arms carry the same data.
pub fn run_branch(
    command: BranchCommand,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    match command {
        BranchCommand::BranchPackage(args) => branch_package(args, debug, live),
        BranchCommand::CopyPackage(args) => copy_package(args, debug, live),
        BranchCommand::CopyProject(args) => copy_project(args, debug, live),
        BranchCommand::SetDevelProject(args) => set_devel_project(args, debug, live),
        BranchCommand::LinkPackage(args) => link_package(args, debug, live),
        BranchCommand::RepairLink(args) => repair_link(args, debug, live),
        BranchCommand::SetLinkRev(args) => set_link_rev(args, debug, live),
        BranchCommand::SetLinkRevProject(args) => set_link_rev_project(args, debug, live),
        BranchCommand::LinkToBranch(args) => link_to_branch(args, debug, live),
        BranchCommand::DetachBranch(args) => detach_branch(args, debug, live),
    }
}

/// The dry-run mode for a mutation: live only when `--live` was
/// passed. Dry-run is the default for mutations; live mode must be
/// asked for explicitly. The debug sink is attached so the DryRun-guard
/// decision line reaches the `--debug` channel.
fn dry_run_for(live: bool, debug: &Debug) -> DryRun {
    DryRun::new(!live).with_debug(debug.clone())
}

/// An `ObsClient` for the effective OBS base (config file / env /
/// default) with the environment OBS credentials for that URL,
/// reporting on the debug channel. Credentials resolve to `None`
/// when unconfigured; the operations fail closed themselves when
/// they need them. A half-configured pair fails fast here, even in
/// dry-run (see the module docs).
fn obs_client<'a>(http: &'a dyn HttpClient, debug: &Debug) -> Result<ObsClient<'a>, GeectlError> {
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let effective = config::effective_config(&file_config)?;
    // Unreachable by construction — effective_config always yields
    // Some(obs_url) — but kept as a defensive default rather than an
    // unwrap: a config change must not turn into a panic here.
    let obs_url = effective.config.obs_url.as_deref().unwrap_or(OBS_API_BASE);
    let credentials = EnvObsAuth::credentials_for_url(obs_url, obs_url, &file_config.instances)?;
    ObsClient::with_base_and_credentials(http, obs_url, credentials)
        .map(|client| client.with_debug(debug.clone()))
}

/// An `ObsClient` for another OBS instance (`--to-apiurl`) with
/// that instance's own credentials, resolved by URL from the
/// environment (the matching instance alias's variables, else a clear error naming them —
/// the source instance's credentials are never forwarded to the
/// target; see `auth::resolve_obs_basic_for_url`). The base URL is
/// validated here, so a malformed `--to-apiurl` fails fast — in
/// dry-run too, before any read.
fn target_client<'a>(
    http: &'a dyn HttpClient,
    debug: &Debug,
    api_url: &str,
) -> Result<ObsClient<'a>, GeectlError> {
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let effective = config::effective_config(&file_config)?;
    let default_url = effective.config.obs_url.as_deref().unwrap_or(OBS_API_BASE);
    // `--to-apiurl` means another instance: pointing it at the
    // configured instance would silently take the degraded
    // client-side copy (no server copy, `--keep-link`/`--comment`
    // refused) instead of the same-instance server copy. Refuse it
    // with the fix — in dry-run too, before any read.
    if same_api_url(api_url, default_url)? {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!(
                "--to-apiurl points at the configured instance ({default_url}); omit --to-apiurl for a same-instance copy"
            ),
        ));
    }
    let credentials =
        EnvObsAuth::credentials_for_url(api_url, default_url, &file_config.instances)?;
    ObsClient::with_base_and_credentials(http, api_url, credentials)
        .map(|client| client.with_debug(debug.clone()))
}

/// Parse a `--revision` value for `set-link-rev` /
/// `set-link-rev-project` into the library revision. Bare values are
/// validated by [`RevisionSpec::parse`] (a positive number or a 32-hex
/// md5, else `InvalidInput`) and normalized through it: an uppercase
/// md5 is stored lowercase, so the link records the canonical form.
/// A bare value expands, like osc's default: `pinned:<rev>` selects
/// the plain form.
pub fn parse_link_revision(value: &str) -> Result<LinkRevision, GeectlError> {
    match value {
        "latest" => Ok(LinkRevision::Latest),
        "expanded" => Ok(LinkRevision::Expanded),
        "unset" => Ok(LinkRevision::Unset),
        _ => {
            if let Some(rev) = value.strip_prefix("pinned:") {
                Ok(LinkRevision::Pinned(link_revision_value(rev)?))
            } else if let Some(rev) = value.strip_prefix("expanded:") {
                Ok(LinkRevision::PinnedExpanded(link_revision_value(rev)?))
            } else {
                Ok(LinkRevision::PinnedExpanded(link_revision_value(value)?))
            }
        }
    }
}

/// A bare revision value, validated and normalized through
/// [`RevisionSpec::parse`]: a number renders as itself, an md5
/// renders lowercase.
fn link_revision_value(value: &str) -> Result<String, GeectlError> {
    match RevisionSpec::parse(value)? {
        RevisionSpec::Rev(n) => Ok(n.to_string()),
        RevisionSpec::Md5(md5) => Ok(md5),
    }
}

/// A `project/package` coordinate pair.
fn coords(project: &str, package: &str) -> String {
    format!("{project}/{package}")
}

/// A coordinate pair with optional sides: the present parts joined
/// by `/`; `None` when neither side is present.
fn coords_opt(project: Option<&str>, package: Option<&str>) -> Option<String> {
    match (project, package) {
        (Some(project), Some(package)) => Some(format!("{project}/{package}")),
        (Some(project), None) => Some(project.to_string()),
        (None, Some(package)) => Some(package.to_string()),
        (None, None) => None,
    }
}

/// One header line echoing the query coordinates:
/// `<command> <source>[ -> <target>]`. The header is the text's
/// first line; `finish` puts `dry_run:` right after it.
fn header(command: &str, source: String, target: Option<String>) -> String {
    match target {
        Some(target) => format!("{command} {source} -> {target}"),
        None => format!("{command} {source}"),
    }
}

/// Combine a payload and its text lines: `dry_run` joins the payload
/// and goes right after the header line (the maintenance renderer's
/// order); the plan, when present, joins the payload and follows the
/// header and result lines verbatim.
fn finish(
    mut payload: Value,
    mut lines: Vec<String>,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    payload["dry_run"] = json!(dry_run);
    let at = if lines.is_empty() { 0 } else { 1 };
    lines.insert(at, format!("dry_run: {dry_run}"));
    if let Some(plan) = plan {
        payload["plan"] = json!(plan);
        lines.extend(plan);
    }
    (payload, lines.join("\n"))
}

/// Render a header row plus one row per item, columns space-aligned to
/// the widest value in the printed set. No truncation: a long value
/// makes a wide line, and the JSON payload remains the complete
/// record. No trailing spaces: the last column is never padded.
fn render_table<const N: usize>(header: [&str; N], rows: &[[String; N]]) -> String {
    let mut widths: [usize; N] = header.map(|h| h.chars().count());
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let mut lines = vec![render_row(&header.map(str::to_string), &widths)];
    for row in rows {
        lines.push(render_row(row, &widths));
    }
    lines.join("\n")
}

fn render_row<const N: usize>(cells: &[String; N], widths: &[usize; N]) -> String {
    let joined = cells
        .iter()
        .enumerate()
        .map(|(i, cell)| format!("{:<width$}", cell, width = widths[i]))
        .collect::<Vec<_>>()
        .join(" ");
    joined.trim_end().to_string()
}

fn branch_package(
    args: BranchPackageArgs,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let options = BranchOptions {
        target_project: args.target_project.clone(),
        target_package: args.target_package.clone(),
        no_devel_project: args.no_devel_project,
        new_package: args.new_package,
        force: args.force,
        revision: args.revision.clone(),
        linkrev: args.linkrev.clone(),
        comment: args.comment.clone(),
        noaccess: args.noaccess,
        add_repositories: args.add_repositories,
        add_repositories_block: args.add_repositories_block.clone(),
        add_repositories_rebuild: args.add_repositories_rebuild.clone(),
        extend_package_names: args.extend_package_names,
        maintenance: args.maintenance,
    };
    let dry_run = dry_run_for(live, debug);
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = obs_client(&http, debug)?;
    match client.branch_package(&args.project, &args.package, &options, &dry_run)? {
        // Dry-run: the guard returned None and recorded the plan.
        // The target is the requested one when the flags named it,
        // else null: the server resolves it only when live.
        None => {
            let plan = dry_run.plan();
            let payload = json!({
                "source_project": args.project,
                "source_package": args.package,
                "target_project": args.target_project,
                "target_package": args.target_package,
            });
            // Text omission (decided): the target when the flags
            // did not name it — null in the payload, the server
            // resolves it only when live. The header echoes only
            // the requested coordinates.
            let lines = vec![header(
                "branch-package",
                coords(&args.project, &args.package),
                coords_opt(
                    args.target_project.as_deref(),
                    args.target_package.as_deref(),
                ),
            )];
            Ok(finish(payload, lines, true, Some(plan)))
        }
        Some(result) => Ok(finish(
            branch_result_json(&result),
            render_branch_result_text(&result),
            false,
            None,
        )),
    }
}

/// A live branch result as JSON. The target is the server-resolved
/// one, never the requested flag value.
fn branch_result_json(result: &BranchResult) -> Value {
    json!({
        "source_project": result.source_project,
        "source_package": result.source_package,
        "target_project": result.target_project,
        "target_package": result.target_package,
    })
}

/// Render a live branch result as its header line: the result IS
/// the four coordinates (source and server-resolved target), so the
/// header carries the whole payload and the text has nothing else.
/// No omission: every payload field is in the header.
fn render_branch_result_text(result: &BranchResult) -> Vec<String> {
    vec![header(
        "branch-package",
        coords(&result.source_project, &result.source_package),
        Some(coords(&result.target_project, &result.target_package)),
    )]
}

fn copy_package(
    args: CopyPackageArgs,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let options = CopyOptions {
        revision: args.revision.clone(),
        keep_link: args.keep_link,
        expand: args.expand,
        comment: args.comment.clone(),
        noservice: args.noservice,
        dont_update_source: args.dont_update_source,
        with_vrev: args.with_vrev,
    };
    let dry_run = dry_run_for(live, debug);
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = obs_client(&http, debug)?;
    let target_package = &args.target_package;
    let payload = json!({
        "source_project": args.source_project,
        "source_package": args.source_package,
        "target_project": args.target_project,
        "target_package": target_package,
    });
    let mut lines = vec![header(
        "copy-package",
        coords(&args.source_project, &args.source_package),
        Some(coords(&args.target_project, target_package)),
    )];
    // The target instance is a query coordinate too: it rides the
    // header after `@`, not a second line.
    if let Some(api_url) = &args.to_apiurl {
        lines[0].push_str(&format!(" @ {api_url}"));
    }
    match &args.to_apiurl {
        None => match client.copy_package(
            &args.source_project,
            &args.source_package,
            &args.target_project,
            target_package,
            &options,
            &dry_run,
        )? {
            // Dry-run: the guard returned None and recorded the plan.
            // Text omission (decided): the revision is null in the
            // payload — no line; the copy hasn't happened.
            None => {
                let plan = dry_run.plan();
                let mut payload = payload;
                payload["revision"] = Value::Null;
                Ok(finish(payload, lines, true, Some(plan)))
            }
            Some(revision) => {
                let mut payload = payload;
                payload["revision"] = json!(revision);
                let mut lines = lines;
                lines.push(format!("revision: {revision}"));
                Ok(finish(payload, lines, false, None))
            }
        },
        Some(api_url) => {
            let target = target_client(&http, debug, api_url)?;
            let result = client.copy_package_to_instance(
                &target,
                &args.source_project,
                &args.source_package,
                &args.target_project,
                target_package,
                &options,
                &dry_run,
            )?;
            // The cross-instance op returns its counts in the result
            // itself (never None): in dry-run the revision is None
            // and the counts are 0 — the handler tells them apart by
            // `live`, so the dry payload never reports counts.
            let mut payload = payload;
            payload["target_api_url"] = json!(api_url);
            payload["revision"] = json!(result.revision);
            let mut lines = lines;
            // Text omission (decided): dry text never reports
            // counts — the op returns 0/0 in dry-run and the
            // handler omits them; live with all files identical
            // reports no revision line (revision null).
            if live {
                payload["copied_files"] = json!(result.copied_files);
                payload["skipped_files"] = json!(result.skipped_files);
                lines.push(format!("copied_files: {}", result.copied_files));
                lines.push(format!("skipped_files: {}", result.skipped_files));
                if let Some(revision) = result.revision {
                    lines.push(format!("revision: {revision}"));
                }
                Ok(finish(payload, lines, false, None))
            } else {
                let plan = dry_run.plan();
                Ok(finish(payload, lines, true, Some(plan)))
            }
        }
    }
}

fn copy_project(
    args: CopyProjectArgs,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let options = CopyProjectOptions {
        with_binaries: args.with_binaries,
        with_history: args.with_history,
        make_older: args.make_older,
        make_origin_older: args.make_origin_older,
        noservice: args.noservice,
        resign: args.resign,
        comment: args.comment.clone(),
        no_delay: args.no_delay,
    };
    let dry_run = dry_run_for(live, debug);
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = obs_client(&http, debug)?;
    client.copy_project(
        &args.source_project,
        &args.target_project,
        &options,
        &dry_run,
    )?;
    let payload = json!({
        "source_project": args.source_project,
        "target_project": args.target_project,
    });
    // No omission: the payload is the two projects, the dry flag,
    // and the plan — all in the text.
    let lines = vec![header(
        "copy-project",
        args.source_project.clone(),
        Some(args.target_project.clone()),
    )];
    if live {
        Ok(finish(payload, lines, false, None))
    } else {
        let plan = dry_run.plan();
        Ok(finish(payload, lines, true, Some(plan)))
    }
}

fn set_devel_project(
    args: SetDevelProjectArgs,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let dry_run = dry_run_for(live, debug);
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = obs_client(&http, debug)?;
    client.set_devel_project(
        &args.project,
        &args.package,
        args.devel_project.as_deref(),
        args.devel_package.as_deref(),
        &dry_run,
    )?;
    let payload = json!({
        "project": args.project,
        "package": args.package,
        "devel_project": args.devel_project,
        "devel_package": args.devel_package,
    });
    // Text omission (decided): a removal (no devel flags) has no
    // devel line — the bare header IS the removal; the payload's
    // nulls are not echoed.
    let lines = vec![header(
        "set-devel-project",
        coords(&args.project, &args.package),
        coords_opt(args.devel_project.as_deref(), args.devel_package.as_deref()),
    )];
    if live {
        Ok(finish(payload, lines, false, None))
    } else {
        let plan = dry_run.plan();
        Ok(finish(payload, lines, true, Some(plan)))
    }
}

fn link_package(
    args: LinkPackageArgs,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let options = LinkOptions {
        force: args.force,
        revision: args.revision.clone(),
        vrev: args.vrev.clone(),
        cicount: args.cicount.clone(),
        missing_target: args.missing_target,
        disable_build: args.disable_build,
        disable_publish: args.disable_publish,
    };
    let dry_run = dry_run_for(live, debug);
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = obs_client(&http, debug)?;
    let target_package = &args.target_package;
    client.link_package(
        &args.source_project,
        &args.source_package,
        &args.target_project,
        target_package,
        &options,
        &dry_run,
    )?;
    let payload = json!({
        "source_project": args.source_project,
        "source_package": args.source_package,
        "target_project": args.target_project,
        "target_package": target_package,
    });
    // No omission: the payload is the four coordinates, the dry
    // flag, and the plan — all in the text.
    let lines = vec![header(
        "link-package",
        coords(&args.source_project, &args.source_package),
        Some(coords(&args.target_project, target_package)),
    )];
    if live {
        Ok(finish(payload, lines, false, None))
    } else {
        let plan = dry_run.plan();
        Ok(finish(payload, lines, true, Some(plan)))
    }
}

fn repair_link(
    args: RepairLinkArgs,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let dry_run = dry_run_for(live, debug);
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = obs_client(&http, debug)?;
    client.repair_link(&args.project, &args.package, &dry_run)?;
    let payload = json!({
        "project": args.project,
        "package": args.package,
    });
    // No omission: the payload is the two coordinates, the dry
    // flag, and the plan — all in the text.
    let lines = vec![header(
        "repair-link",
        coords(&args.project, &args.package),
        None,
    )];
    if live {
        Ok(finish(payload, lines, false, None))
    } else {
        let plan = dry_run.plan();
        Ok(finish(payload, lines, true, Some(plan)))
    }
}

fn set_link_rev(
    args: SetLinkRevArgs,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let revision = parse_link_revision(&args.revision)?;
    let dry_run = dry_run_for(live, debug);
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = obs_client(&http, debug)?;
    let set = client.set_link_rev(&args.project, &args.package, &revision, &dry_run)?;
    // None is dry-run (the guard) or live-unset (the link follows
    // its target again): the handler tells them apart by `live`.
    let payload = json!({
        "project": args.project,
        "package": args.package,
        "revision": set,
    });
    // Text omission (decided): dry text does not echo the requested
    // revision (the payload's revision is null); live-unset likewise
    // has no revision line — the link follows its target again.
    let mut lines = vec![header(
        "set-link-rev",
        coords(&args.project, &args.package),
        None,
    )];
    if let Some(revision) = &set {
        lines.push(format!("revision: {revision}"));
    }
    if live {
        Ok(finish(payload, lines, false, None))
    } else {
        let plan = dry_run.plan();
        Ok(finish(payload, lines, true, Some(plan)))
    }
}

fn set_link_rev_project(
    args: SetLinkRevProjectArgs,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let revision = parse_link_revision(&args.revision)?;
    let dry_run = dry_run_for(live, debug);
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = obs_client(&http, debug)?;
    let set = client.set_link_rev_project(&args.project, &revision, &dry_run)?;
    if live {
        let payload = json!({
            "project": args.project,
            "set": set
                .iter()
                .map(|(package, revision)| json!({"package": package, "revision": revision}))
                .collect::<Vec<_>>(),
            "dry_run": false,
        });
        // Text omission (decided): the requested `--revision` value
        // is not repeated in the live text — the table shows the
        // resulting per-package revisions; the table stays narrow
        // (PACKAGE REVISION) and is not widened to echo the payload.
        let header_line = header("set-link-rev-project", args.project.clone(), None);
        let text = if set.is_empty() {
            format!("{header_line}\ndry_run: false\nno links set")
        } else {
            let rows: Vec<[String; 2]> = set
                .iter()
                .map(|(package, revision)| {
                    [
                        package.clone(),
                        revision.clone().unwrap_or_else(|| "-".to_string()),
                    ]
                })
                .collect();
            format!(
                "{header_line}\ndry_run: false\n{}",
                render_table(["PACKAGE", "REVISION"], &rows)
            )
        };
        Ok((payload, text))
    } else {
        let plan = dry_run.plan();
        let payload = json!({
            "project": args.project,
            "set": [],
            "dry_run": true,
            "plan": plan,
        });
        // Dry: the set is empty by construction, so there is no
        // `no links set` line.
        let header_line = header("set-link-rev-project", args.project.clone(), None);
        let text = format!("{header_line}\ndry_run: true\n{}", plan.join("\n"));
        Ok((payload, text))
    }
}

fn link_to_branch(
    args: LinkToBranchArgs,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let dry_run = dry_run_for(live, debug);
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = obs_client(&http, debug)?;
    client.link_to_branch(&args.project, &args.package, &dry_run)?;
    let payload = json!({
        "project": args.project,
        "package": args.package,
    });
    // No omission: the payload is the two coordinates, the dry
    // flag, and the plan — all in the text.
    let lines = vec![header(
        "link-to-branch",
        coords(&args.project, &args.package),
        None,
    )];
    if live {
        Ok(finish(payload, lines, false, None))
    } else {
        let plan = dry_run.plan();
        Ok(finish(payload, lines, true, Some(plan)))
    }
}

fn detach_branch(
    args: DetachBranchArgs,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let dry_run = dry_run_for(live, debug);
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = obs_client(&http, debug)?;
    let detached = client.detach_branch(&args.project, &args.package, &dry_run)?;
    // None is dry-run (the guard) or a live detach that dropped the
    // link instead of copying: the handler tells them apart by `live`.
    let payload = json!({
        "project": args.project,
        "package": args.package,
        "revision": detached,
    });
    // Text omission (decided): a null revision (dry, or a live
    // detach that dropped the link) has no line.
    let mut lines = vec![header(
        "detach-branch",
        coords(&args.project, &args.package),
        None,
    )];
    if let Some(revision) = detached {
        lines.push(format!("revision: {revision}"));
    }
    if live {
        Ok(finish(payload, lines, false, None))
    } else {
        let plan = dry_run.plan();
        Ok(finish(payload, lines, true, Some(plan)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// The binary's CLI shape for parsing tests only: the `branch`
    /// group as it is registered in `main.rs`.
    #[derive(Debug, Parser)]
    #[command(name = "geectl")]
    struct TestCli {
        #[command(subcommand)]
        command: TestCommand,
    }

    #[derive(Debug, Subcommand)]
    enum TestCommand {
        Branch {
            #[command(subcommand)]
            command: BranchCommand,
        },
    }

    fn parse(argv: &[&str]) -> BranchCommand {
        match TestCli::parse_from(argv).command {
            TestCommand::Branch { command } => command,
        }
    }

    fn parse_fails(argv: &[&str]) {
        assert!(TestCli::try_parse_from(argv).is_err(), "argv: {argv:?}");
    }

    #[test]
    fn branch_package_parses_all_flags() {
        let BranchCommand::BranchPackage(args) = parse(&[
            "geectl",
            "branch",
            "branch-package",
            "--project",
            "test:factory",
            "--package",
            "foo",
            "--target-project",
            "home:alice:branches:test:factory",
            "--target-package",
            "foo",
            "--no-devel-project",
            "--new-package",
            "--force",
            "--revision",
            "3",
            "--linkrev",
            "2",
            "--comment",
            "branch",
            "--noaccess",
            "--add-repositories",
            "--add-repositories-block",
            "local",
            "--add-repositories-rebuild",
            "local",
            "--extend-package-names",
            "--maintenance",
        ]) else {
            panic!("wrong variant");
        };
        assert_eq!(args.project, "test:factory");
        assert_eq!(args.package, "foo");
        assert_eq!(
            args.target_project.as_deref(),
            Some("home:alice:branches:test:factory")
        );
        assert_eq!(args.target_package.as_deref(), Some("foo"));
        assert!(args.no_devel_project);
        assert!(args.new_package);
        assert!(args.force);
        assert_eq!(args.revision.as_deref(), Some("3"));
        assert_eq!(args.linkrev.as_deref(), Some("2"));
        assert_eq!(args.comment.as_deref(), Some("branch"));
        assert!(args.noaccess);
        assert!(args.add_repositories);
        assert_eq!(args.add_repositories_block.as_deref(), Some("local"));
        assert_eq!(args.add_repositories_rebuild.as_deref(), Some("local"));
        assert!(args.extend_package_names);
        assert!(args.maintenance);
    }

    #[test]
    fn branch_package_requires_project_and_package() {
        parse_fails(&["geectl", "branch", "branch-package", "--package", "foo"]);
        parse_fails(&["geectl", "branch", "branch-package", "--project", "p"]);
    }

    #[test]
    fn branch_package_rejects_repositories_options_without_add_repositories() {
        // A block/rebuild the live run would silently drop must fail
        // at parse time: the options are only sent with
        // `--add-repositories`.
        parse_fails(&[
            "geectl",
            "branch",
            "branch-package",
            "--project",
            "p",
            "--package",
            "foo",
            "--add-repositories-block",
            "local",
        ]);
        parse_fails(&[
            "geectl",
            "branch",
            "branch-package",
            "--project",
            "p",
            "--package",
            "foo",
            "--add-repositories-rebuild",
            "local",
        ]);
    }

    #[test]
    fn copy_package_parses_all_flags_including_to_apiurl() {
        let BranchCommand::CopyPackage(args) = parse(&[
            "geectl",
            "branch",
            "copy-package",
            "--source-project",
            "test:factory",
            "--source-package",
            "foo",
            "--target-project",
            "test:target",
            "--target-package",
            "foo",
            "--revision",
            "3",
            "--keep-link",
            "--expand",
            "--comment",
            "copy",
            "--noservice",
            "--dont-update-source",
            "--with-vrev",
            "--to-apiurl",
            "https://api.example.org",
        ]) else {
            panic!("wrong variant");
        };
        assert_eq!(args.source_project, "test:factory");
        assert_eq!(args.target_package, "foo");
        assert!(args.keep_link);
        assert!(args.expand);
        assert_eq!(args.comment.as_deref(), Some("copy"));
        assert_eq!(args.to_apiurl.as_deref(), Some("https://api.example.org"));
    }

    #[test]
    fn copy_package_requires_source_target_and_package() {
        parse_fails(&[
            "geectl",
            "branch",
            "copy-package",
            "--source-package",
            "foo",
            "--target-project",
            "t",
            "--target-package",
            "foo",
        ]);
        parse_fails(&[
            "geectl",
            "branch",
            "copy-package",
            "--source-project",
            "s",
            "--source-package",
            "foo",
            "--target-project",
            "t",
        ]);
    }

    #[test]
    fn copy_project_parses_all_flags() {
        let BranchCommand::CopyProject(args) = parse(&[
            "geectl",
            "branch",
            "copy-project",
            "--source-project",
            "test:factory",
            "--target-project",
            "test:target",
            "--with-binaries",
            "--with-history",
            "--make-older",
            "--make-origin-older",
            "--noservice",
            "--resign",
            "--comment",
            "copy",
            "--no-delay",
        ]) else {
            panic!("wrong variant");
        };
        assert!(args.with_binaries);
        assert!(args.with_history);
        assert!(args.make_older);
        assert!(args.make_origin_older);
        assert!(args.noservice);
        assert!(args.resign);
        assert_eq!(args.comment.as_deref(), Some("copy"));
        assert!(args.no_delay);
    }

    #[test]
    fn set_devel_project_parses_and_requires_project_for_package() {
        let BranchCommand::SetDevelProject(args) = parse(&[
            "geectl",
            "branch",
            "set-devel-project",
            "--project",
            "test:factory",
            "--package",
            "foo",
            "--devel-project",
            "devel:languages:python",
            "--devel-package",
            "foo",
        ]) else {
            panic!("wrong variant");
        };
        assert_eq!(
            args.devel_project.as_deref(),
            Some("devel:languages:python")
        );
        assert_eq!(args.devel_package.as_deref(), Some("foo"));
        // A devel package without a devel project is meaningless:
        // clap refuses it, the handler never sees it.
        parse_fails(&[
            "geectl",
            "branch",
            "set-devel-project",
            "--project",
            "p",
            "--package",
            "foo",
            "--devel-package",
            "foo",
        ]);
    }

    #[test]
    fn link_package_parses_all_flags_and_rejects_bad_cicount() {
        let BranchCommand::LinkPackage(args) = parse(&[
            "geectl",
            "branch",
            "link-package",
            "--source-project",
            "test:factory",
            "--source-package",
            "foo",
            "--target-project",
            "test:target",
            "--target-package",
            "foo",
            "--force",
            "--revision",
            "3",
            "--vrev",
            "1",
            "--cicount",
            "copy",
            "--missing-target",
            "--disable-build",
            "--disable-publish",
        ]) else {
            panic!("wrong variant");
        };
        assert!(args.force);
        assert_eq!(args.cicount.as_deref(), Some("copy"));
        assert!(args.missing_target);
        assert!(args.disable_build);
        assert!(args.disable_publish);
        parse_fails(&[
            "geectl",
            "branch",
            "link-package",
            "--source-project",
            "s",
            "--source-package",
            "foo",
            "--target-project",
            "t",
            "--target-package",
            "foo",
            "--cicount",
            "sometimes",
        ]);
    }

    #[test]
    fn repair_link_link_to_branch_and_detach_branch_parse() {
        assert!(matches!(
            parse(&[
                "geectl",
                "branch",
                "repair-link",
                "--project",
                "p",
                "--package",
                "foo"
            ]),
            BranchCommand::RepairLink(_)
        ));
        assert!(matches!(
            parse(&[
                "geectl",
                "branch",
                "link-to-branch",
                "--project",
                "p",
                "--package",
                "foo"
            ]),
            BranchCommand::LinkToBranch(_)
        ));
        assert!(matches!(
            parse(&[
                "geectl",
                "branch",
                "detach-branch",
                "--project",
                "p",
                "--package",
                "foo"
            ]),
            BranchCommand::DetachBranch(_)
        ));
        parse_fails(&["geectl", "branch", "repair-link", "--project", "p"]);
    }

    #[test]
    fn set_link_rev_parses() {
        let BranchCommand::SetLinkRev(args) = parse(&[
            "geectl",
            "branch",
            "set-link-rev",
            "--project",
            "p",
            "--package",
            "foo",
            "--revision",
            "pinned:3",
        ]) else {
            panic!("wrong variant");
        };
        assert_eq!(args.revision, "pinned:3");
        parse_fails(&[
            "geectl",
            "branch",
            "set-link-rev",
            "--project",
            "p",
            "--package",
            "foo",
        ]);
    }

    #[test]
    fn set_link_rev_project_parses() {
        assert!(matches!(
            parse(&[
                "geectl",
                "branch",
                "set-link-rev-project",
                "--project",
                "p",
                "--revision",
                "latest"
            ]),
            BranchCommand::SetLinkRevProject(_)
        ));
    }

    #[test]
    fn parse_link_revision_accepts_all_six_forms() {
        assert_eq!(parse_link_revision("latest").unwrap(), LinkRevision::Latest);
        assert_eq!(
            parse_link_revision("expanded").unwrap(),
            LinkRevision::Expanded
        );
        assert_eq!(parse_link_revision("unset").unwrap(), LinkRevision::Unset);
        assert_eq!(
            parse_link_revision("3").unwrap(),
            LinkRevision::PinnedExpanded("3".to_string())
        );
        assert_eq!(
            parse_link_revision("pinned:3").unwrap(),
            LinkRevision::Pinned("3".to_string())
        );
        assert_eq!(
            parse_link_revision("expanded:3").unwrap(),
            LinkRevision::PinnedExpanded("3".to_string())
        );
        // An md5 is validated and normalized to lowercase.
        let md5 = "0123456789ABCDEF0123456789ABCDEF";
        assert_eq!(
            parse_link_revision(md5).unwrap(),
            LinkRevision::PinnedExpanded(md5.to_ascii_lowercase())
        );
        assert_eq!(
            parse_link_revision("pinned:0123456789ABCDEF0123456789ABCDEF").unwrap(),
            LinkRevision::Pinned(md5.to_ascii_lowercase())
        );
    }

    #[test]
    fn parse_link_revision_rejects_invalid_values() {
        for bad in [
            "",
            "0",
            "xyz",
            "pinned:",
            "pinned:0",
            "expanded:xyz",
            "1:2",
            "latest:3",
        ] {
            assert!(parse_link_revision(bad).is_err(), "value: {bad:?}");
        }
    }

    #[test]
    fn finish_puts_dry_run_after_the_header_and_appends_the_plan() {
        let payload = json!({"project": "p"});
        let (payload, text) = finish(
            payload,
            vec!["branch-package test:factory/foo".to_string()],
            true,
            Some(vec!["[dry-run] would: do it".to_string()]),
        );
        assert_eq!(payload["dry_run"], true);
        assert_eq!(payload["plan"][0], "[dry-run] would: do it");
        assert_eq!(
            text,
            "branch-package test:factory/foo\ndry_run: true\n[dry-run] would: do it"
        );
        let (payload, text) = finish(
            json!({"project": "p"}),
            vec!["branch-package test:factory/foo".to_string()],
            false,
            None,
        );
        assert_eq!(payload["dry_run"], false);
        assert!(payload.get("plan").is_none());
        assert_eq!(text, "branch-package test:factory/foo\ndry_run: false");
    }

    #[test]
    fn render_table_aligns_columns_without_trailing_spaces() {
        let text = render_table(
            ["PACKAGE", "REVISION"],
            &[
                ["foo".to_string(), "1".to_string()],
                ["longer-name".to_string(), "-".to_string()],
            ],
        );
        assert_eq!(text, "PACKAGE     REVISION\nfoo         1\nlonger-name -");
        for line in text.lines() {
            assert_eq!(line, line.trim_end());
        }
    }

    #[test]
    fn branch_result_header_carries_every_field_of_the_payload() {
        // The agreement test: every id, name, and revision in the
        // JSON payload appears in the text rendering — here, in the
        // single header line.
        let result = BranchResult {
            source_project: "test:factory".to_string(),
            source_package: "foo".to_string(),
            target_project: "home:alice:branches:test:factory".to_string(),
            target_package: "foo".to_string(),
        };
        let payload = branch_result_json(&result);
        let text = render_branch_result_text(&result).join("\n");
        for key in [
            "source_project",
            "source_package",
            "target_project",
            "target_package",
        ] {
            let value = payload[key].as_str().unwrap();
            assert!(text.contains(value), "text misses {key}={value}:\n{text}");
        }
    }

    #[test]
    fn header_echoes_coordinates_in_one_line() {
        assert_eq!(
            header("branch-package", coords("test:factory", "foo"), None),
            "branch-package test:factory/foo"
        );
        assert_eq!(
            header(
                "copy-package",
                coords("test:factory", "foo"),
                Some(coords("test:target", "foo")),
            ),
            "copy-package test:factory/foo -> test:target/foo"
        );
        assert_eq!(
            coords_opt(Some("p"), Some("foo")),
            Some("p/foo".to_string())
        );
        assert_eq!(coords_opt(Some("p"), None), Some("p".to_string()));
        assert_eq!(coords_opt(None, Some("foo")), Some("foo".to_string()));
        assert_eq!(coords_opt(None, None), None);
    }

    #[test]
    fn link_rev_set_table_carries_every_package_and_revision() {
        let set = [
            ("foo".to_string(), Some("1".to_string())),
            ("bar".to_string(), None),
        ];
        let rows: Vec<[String; 2]> = set
            .iter()
            .map(|(package, revision)| {
                [
                    package.clone(),
                    revision.clone().unwrap_or_else(|| "-".to_string()),
                ]
            })
            .collect();
        let text = render_table(["PACKAGE", "REVISION"], &rows);
        assert!(text.contains("foo"));
        assert!(text.contains("bar"));
        assert!(text.contains('1'));
    }
}
