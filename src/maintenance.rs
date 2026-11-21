//! `maintenance` — the maintenance command group: the maintained-instances
//! listing and maintenance branching, thin over `ObsClient`.
//!
//! The group is the CLI surface for `src/obs/maintenance.rs` (batch 6).
//! Every leaf returns `(payload, text)` — the JSON payload and a human
//! rendering of the same data — per the #51/#52 design: text stays
//! opt-in via `--format text`, stdout carries only the result payload,
//! and nothing here touches raw XML (typed values only).

use clap::{Args, Subcommand};
use geectl::auth::{AuthProvider, EnvObsAuth};
use geectl::config;
use geectl::debug::Debug;
use geectl::error::GeectlError;
use geectl::http::{HttpClient, Timeout, UreqClient};
use geectl::obs::{
    DryRun, MaintainedInstance, MaintenanceBranchOptions, MaintenanceBranchResult, ObsClient,
    OBS_API_BASE,
};
use serde_json::{json, Value};

/// The `maintenance` subcommands. Names are the library operation names
/// in kebab-case, grouped under the top-level noun.
#[derive(Debug, Subcommand)]
pub enum MaintenanceCommand {
    /// List the maintained instances of a package: the projects carrying
    /// the OBS:Maintained attribute that the package exists in, with
    /// where a branch would land. Read-only; requires OBS credentials.
    MaintainedInstances(InstancesArgs),
    /// Branch a maintained package into its maintenance branch project
    /// and report where it landed. A mutation: dry-run is the default
    /// (only the plan is printed); the global `--live` performs the
    /// branch.
    MaintenanceBranch(BranchArgs),
}

/// Arguments for `maintenance maintained-instances`.
#[derive(Debug, Args)]
pub struct InstancesArgs {
    /// The package whose maintained instances to list.
    #[arg(long)]
    package: String,
    /// The attribute marking a project as maintaining the package
    /// (`NAMESPACE:NAME`, default `OBS:Maintained`).
    #[arg(long)]
    attribute: Option<String>,
    /// The attribute pointing a project at its update project
    /// (`NAMESPACE:NAME`, default `OBS:UpdateProject`).
    #[arg(long)]
    update_project_attribute: Option<String>,
}

/// Arguments for `maintenance maintenance-branch`.
#[derive(Debug, Args)]
pub struct BranchArgs {
    /// The package to branch into its maintenance branch project.
    #[arg(long)]
    package: String,
    /// The attribute marking a project as maintaining the package
    /// (`NAMESPACE:NAME`, default `OBS:Maintained`).
    #[arg(long)]
    attribute: Option<String>,
    /// The attribute pointing a project at its update project
    /// (`NAMESPACE:NAME`, default `OBS:UpdateProject`).
    #[arg(long)]
    update_project_attribute: Option<String>,
    /// Override the server-derived branch project
    /// (`home:<user>:branches:OBS_Maintained:<package>`).
    #[arg(long)]
    target_project: Option<String>,
}

/// Run a `maintenance` subcommand: returns `(payload, text)` — the JSON
/// payload and its human rendering. The caller selects by `--format`;
/// both arms carry the same data. `live` is the global `--live` flag:
/// without it every mutation is a dry run.
pub fn run(
    command: MaintenanceCommand,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    match command {
        MaintenanceCommand::MaintainedInstances(args) => maintained_instances(args, debug),
        MaintenanceCommand::MaintenanceBranch(args) => maintenance_branch(args, debug, live),
    }
}

/// The branch options for a command: the library defaults, overridden
/// by whichever flags were passed.
fn branch_options(
    attribute: Option<&str>,
    update_project_attribute: Option<&str>,
    target_project: Option<&str>,
) -> Result<MaintenanceBranchOptions, GeectlError> {
    let mut options = MaintenanceBranchOptions::new();
    if let Some(attribute) = attribute {
        options = options.with_attribute(attribute)?;
    }
    if let Some(attribute) = update_project_attribute {
        options = options.with_update_project_attribute(attribute)?;
    }
    if let Some(target) = target_project {
        options = options.with_target_project(target)?;
    }
    Ok(options)
}

/// The dry-run mode for a branch command: live only when the global
/// `--live` was passed. Dry-run is the default for mutations; live
/// mode must be asked for explicitly. The debug sink is attached so
/// the DryRun-guard decision line reaches the `--debug` channel.
fn dry_run_for(live: bool, debug: &Debug) -> DryRun {
    DryRun::new(!live).with_debug(debug.clone())
}

/// An `ObsClient` for the effective OBS base (config file / env /
/// default) with the environment OBS credentials, reporting on the
/// debug channel. Credentials resolve to `None` when unconfigured;
/// the operations fail closed themselves when they need them.
fn obs_client<'a>(http: &'a dyn HttpClient, debug: &Debug) -> Result<ObsClient<'a>, GeectlError> {
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let effective = config::effective_config(&file_config)?;
    // Unreachable by construction — effective_config always yields
    // Some(obs_url) — but kept as a defensive default rather than an
    // unwrap: a config change must not turn into a panic here.
    let obs_url = effective.config.obs_url.as_deref().unwrap_or(OBS_API_BASE);
    let credentials = EnvObsAuth.credentials()?;
    ObsClient::with_base_and_credentials(http, obs_url, credentials)
        .map(|client| client.with_debug(debug.clone()))
}

fn maintained_instances(
    args: InstancesArgs,
    debug: &Debug,
) -> Result<(Value, String), GeectlError> {
    let options = branch_options(
        args.attribute.as_deref(),
        args.update_project_attribute.as_deref(),
        None,
    )?;
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = obs_client(&http, debug)?;
    let instances = client.maintained_instances(&args.package, &options)?;
    let payload = json!({
        "package": args.package,
        "instances": instances.iter().map(instance_json).collect::<Vec<_>>(),
    });
    let text = render_instances_text(&args.package, &instances);
    Ok((payload, text))
}

/// One maintained instance as JSON. A missing devel project/package
/// is `null`, never omitted: the text table renders it as `-`.
fn instance_json(instance: &MaintainedInstance) -> Value {
    json!({
        "source_project": instance.source_project,
        "source_package": instance.source_package,
        "devel_project": instance.devel_project,
        "devel_package": instance.devel_package,
        "target_project": instance.target_project,
        "target_package": instance.target_package,
    })
}

/// Render the instances as a table: one header row, space-aligned
/// columns, no borders. An empty listing prints an explicit line,
/// never empty output.
fn render_instances_text(package: &str, instances: &[MaintainedInstance]) -> String {
    if instances.is_empty() {
        return format!("no maintained instances for {package}");
    }
    let rows: Vec<[String; 6]> = instances
        .iter()
        .map(|instance| {
            [
                instance.source_project.clone(),
                instance.source_package.clone(),
                instance
                    .devel_project
                    .clone()
                    .unwrap_or_else(|| "-".to_string()),
                instance
                    .devel_package
                    .clone()
                    .unwrap_or_else(|| "-".to_string()),
                instance.target_project.clone(),
                instance.target_package.clone(),
            ]
        })
        .collect();
    render_table(
        [
            "SOURCE_PROJECT",
            "SOURCE_PACKAGE",
            "DEVEL_PROJECT",
            "DEVEL_PACKAGE",
            "TARGET_PROJECT",
            "TARGET_PACKAGE",
        ],
        &rows,
    )
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
        .map(|(i, cell)| format!("{cell:<width$}", width = widths[i]))
        .collect::<Vec<_>>()
        .join(" ");
    joined.trim_end().to_string()
}

fn maintenance_branch(
    args: BranchArgs,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let options = branch_options(
        args.attribute.as_deref(),
        args.update_project_attribute.as_deref(),
        args.target_project.as_deref(),
    )?;
    let dry_run = dry_run_for(live, debug);
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = obs_client(&http, debug)?;
    match client.maintenance_branch(&args.package, &options, &dry_run)? {
        // Dry-run: the guard returned None and recorded the plan —
        // the plan is the payload.
        None => {
            let plan = dry_run.plan();
            let payload = json!({
                "package": args.package,
                "dry_run": true,
                "plan": plan,
            });
            Ok((payload, plan.join("\n")))
        }
        Some(result) => {
            let payload = branch_result_json(&args.package, &result);
            let text = render_branch_result_text(&args.package, &result);
            Ok((payload, text))
        }
    }
}

/// A live branch result as JSON. The absent fields of a
/// multi-package branch are `null`, never omitted.
fn branch_result_json(package: &str, result: &MaintenanceBranchResult) -> Value {
    json!({
        "package": package,
        "dry_run": false,
        "target_project": result.target_project,
        "target_package": result.target_package,
        "source_project": result.source_project,
        "source_package": result.source_package,
    })
}

/// Render a live branch result as `key: value` lines, one fact per
/// line. The package and the live mode come first, so a pasted
/// transcript names what was branched and that it was live. A field
/// the server did not report has no line: there is no value to render.
fn render_branch_result_text(package: &str, result: &MaintenanceBranchResult) -> String {
    let mut lines = vec![
        format!("package: {package}"),
        "dry_run: false".to_string(),
        format!("target_project: {}", result.target_project),
    ];
    if let Some(package) = &result.target_package {
        lines.push(format!("target_package: {package}"));
    }
    if let Some(project) = &result.source_project {
        lines.push(format!("source_project: {project}"));
    }
    if let Some(package) = &result.source_package {
        lines.push(format!("source_package: {package}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// The binary's CLI shape for parsing tests only: the
    /// `maintenance` group as it is registered in `main.rs`,
    /// including the global `--live` flag. The group is nested one
    /// level under the top level, exactly as in the real CLI —
    /// `geectl maintenance maintenance-branch …`, never the flat
    /// `geectl maintenance-branch …`.
    #[derive(Debug, Parser)]
    #[command(name = "geectl")]
    struct TestCli {
        /// Actually perform mutations (mirrors the global `--live`
        /// in `main.rs`).
        #[arg(long, global = true)]
        live: bool,
        #[command(subcommand)]
        command: TestCommand,
    }

    /// The top-level commands, reduced to the `maintenance` group.
    #[derive(Debug, Subcommand)]
    enum TestCommand {
        Maintenance {
            #[command(subcommand)]
            command: MaintenanceCommand,
        },
    }

    fn parse(argv: &[&str]) -> TestCli {
        TestCli::parse_from(argv)
    }

    fn maintenance_command(argv: &[&str]) -> MaintenanceCommand {
        match parse(argv).command {
            TestCommand::Maintenance { command } => command,
        }
    }

    fn fixture_instances() -> Vec<MaintainedInstance> {
        vec![
            MaintainedInstance {
                source_project: "openSUSE:Leap:15.6:Update".to_string(),
                source_package: "foo".to_string(),
                devel_project: Some("devel:languages:python".to_string()),
                devel_package: Some("foo".to_string()),
                target_project: "home:alice:branches:OBS_Maintained:foo".to_string(),
                target_package: "foo.openSUSE_Leap_15.6_Update".to_string(),
            },
            MaintainedInstance {
                source_project: "openSUSE:Leap:15.5:Update".to_string(),
                source_package: "foo".to_string(),
                devel_project: None,
                devel_package: None,
                target_project: "home:alice:branches:OBS_Maintained:foo".to_string(),
                target_package: "foo.openSUSE_Leap_15.5_Update".to_string(),
            },
        ]
    }

    #[test]
    fn maintained_instances_parses_package_and_flags() {
        match maintenance_command(&[
            "geectl",
            "maintenance",
            "maintained-instances",
            "--package",
            "foo",
            "--attribute",
            "OBS:Maintained",
            "--update-project-attribute",
            "OBS:UpdateProject",
        ]) {
            MaintenanceCommand::MaintainedInstances(args) => {
                assert_eq!(args.package, "foo");
                assert_eq!(args.attribute.as_deref(), Some("OBS:Maintained"));
                assert_eq!(
                    args.update_project_attribute.as_deref(),
                    Some("OBS:UpdateProject")
                );
            }
            MaintenanceCommand::MaintenanceBranch(_) => panic!("wrong subcommand"),
        }
    }

    #[test]
    fn maintenance_branch_parses_package_and_target() {
        match maintenance_command(&[
            "geectl",
            "maintenance",
            "maintenance-branch",
            "--package",
            "foo",
            "--target-project",
            "home:alice:branches:custom",
        ]) {
            MaintenanceCommand::MaintenanceBranch(args) => {
                assert_eq!(args.package, "foo");
                assert_eq!(
                    args.target_project.as_deref(),
                    Some("home:alice:branches:custom")
                );
            }
            MaintenanceCommand::MaintainedInstances(_) => panic!("wrong subcommand"),
        }
    }

    #[test]
    fn dry_run_mode_follows_live() {
        match maintenance_command(&[
            "geectl",
            "maintenance",
            "maintenance-branch",
            "--package",
            "foo",
        ]) {
            MaintenanceCommand::MaintenanceBranch(args) => {
                assert_eq!(args.package, "foo");
            }
            MaintenanceCommand::MaintainedInstances(_) => panic!("wrong subcommand"),
        }
        assert!(dry_run_for(false, &Debug::disabled()).enabled());
        assert!(!dry_run_for(true, &Debug::disabled()).enabled());
    }

    #[test]
    fn global_live_flag_parses_before_and_after_the_subcommand() {
        let cli = parse(&[
            "geectl",
            "--live",
            "maintenance",
            "maintenance-branch",
            "--package",
            "foo",
        ]);
        assert!(cli.live);
        let cli = parse(&[
            "geectl",
            "maintenance",
            "maintenance-branch",
            "--package",
            "foo",
            "--live",
        ]);
        assert!(cli.live);
        let cli = parse(&[
            "geectl",
            "maintenance",
            "maintenance-branch",
            "--package",
            "foo",
        ]);
        assert!(!cli.live);
    }

    #[test]
    fn parse_rejects_the_old_positional_form_and_a_missing_package() {
        // The pre-rewrite positional form is gone: the package must
        // be named with --package.
        assert!(
            TestCli::try_parse_from(["geectl", "maintenance", "maintenance-branch", "foo"])
                .is_err()
        );
        assert!(
            TestCli::try_parse_from(["geectl", "maintenance", "maintained-instances", "foo"])
                .is_err()
        );
        assert!(TestCli::try_parse_from(["geectl", "maintenance", "maintenance-branch"]).is_err());
        assert!(
            TestCli::try_parse_from(["geectl", "maintenance", "maintained-instances"]).is_err()
        );
    }

    #[test]
    fn branch_options_rejects_invalid_attribute_and_target() {
        assert!(branch_options(Some("bogus"), None, None).is_err());
        assert!(branch_options(None, Some("bogus"), None).is_err());
        assert!(branch_options(None, None, Some("foo/bar")).is_err());
        assert!(branch_options(None, None, None).is_ok());
    }

    #[test]
    fn instance_json_renders_missing_devel_as_null() {
        let value = instance_json(&fixture_instances()[1]);
        assert_eq!(value["source_project"], "openSUSE:Leap:15.5:Update");
        assert_eq!(value["devel_project"], Value::Null);
        assert_eq!(value["devel_package"], Value::Null);
        assert_eq!(value["target_package"], "foo.openSUSE_Leap_15.5_Update");
    }

    #[test]
    fn instances_text_is_a_table_with_every_identifying_field() {
        let instances = fixture_instances();
        let payload: Vec<Value> = instances.iter().map(instance_json).collect();
        let text = render_instances_text("foo", &instances);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "text:\n{text}");
        assert_eq!(
            lines[0].split_whitespace().collect::<Vec<_>>(),
            [
                "SOURCE_PROJECT",
                "SOURCE_PACKAGE",
                "DEVEL_PROJECT",
                "DEVEL_PACKAGE",
                "TARGET_PROJECT",
                "TARGET_PACKAGE"
            ]
        );
        assert!(lines[1].contains("openSUSE:Leap:15.6:Update"));
        assert!(lines[1].contains("devel:languages:python"));
        assert!(lines[2].contains("openSUSE:Leap:15.5:Update"));
        assert!(lines[2].contains(" - "), "text:\n{text}");
        assert!(
            !lines.iter().any(|line| line.ends_with(' ')),
            "no trailing spaces:\n{text}"
        );
        // Agreement: every identifying field in the JSON appears in
        // the text.
        for value in &payload {
            for key in [
                "source_project",
                "source_package",
                "target_project",
                "target_package",
            ] {
                let field = value[key].as_str().unwrap();
                assert!(text.contains(field), "{field} missing from text");
            }
            if let Some(devel) = value["devel_project"].as_str() {
                assert!(text.contains(devel), "{devel} missing from text");
            }
        }
    }

    #[test]
    fn instances_text_empty_is_explicit() {
        assert_eq!(
            render_instances_text("foo", &[]),
            "no maintained instances for foo"
        );
    }

    #[test]
    fn branch_result_text_renders_present_fields_only() {
        let full = MaintenanceBranchResult {
            target_project: "home:alice:branches:OBS_Maintained:foo".to_string(),
            target_package: Some("foo".to_string()),
            source_project: Some("openSUSE:Leap:15.6:Update".to_string()),
            source_package: Some("foo".to_string()),
        };
        let text = render_branch_result_text("foo", &full);
        assert_eq!(
            text,
            "package: foo\ndry_run: false\ntarget_project: home:alice:branches:OBS_Maintained:foo\ntarget_package: foo\nsource_project: openSUSE:Leap:15.6:Update\nsource_package: foo"
        );
        let payload = branch_result_json("foo", &full);
        for key in [
            "package",
            "target_project",
            "target_package",
            "source_project",
            "source_package",
        ] {
            let field = payload[key].as_str().unwrap();
            assert!(text.contains(field), "{field} missing from text");
        }
        assert!(text.contains("dry_run: false"), "dry_run missing from text");
        let partial = MaintenanceBranchResult {
            target_project: "home:alice:branches:OBS_Maintained:foo".to_string(),
            target_package: None,
            source_project: None,
            source_package: None,
        };
        assert_eq!(
            render_branch_result_text("foo", &partial),
            "package: foo\ndry_run: false\ntarget_project: home:alice:branches:OBS_Maintained:foo"
        );
        assert_eq!(
            branch_result_json("foo", &partial)["target_package"],
            Value::Null
        );
    }

    #[test]
    fn dry_run_plan_renders_verbatim() {
        let dry_run = DryRun::default();
        assert!(!dry_run.allow("branch maintained package foo"));
        let plan = dry_run.plan();
        assert_eq!(
            plan,
            vec!["[dry-run] would: branch maintained package foo".to_string()]
        );
        assert_eq!(
            plan.join("\n"),
            "[dry-run] would: branch maintained package foo"
        );
    }
}
