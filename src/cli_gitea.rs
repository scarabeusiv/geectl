//! `gitea` — the Gitea command group: repository and branch reads
//! plus the repo mutations.
//!
//! Thin over `GiteaClient` like the OBS groups are over `ObsClient`:
//! the flags mirror the library parameters, the JSON payload carries
//! the typed values, and the text renderer prints the same values in
//! a human-readable form.
//!
//! The token is required even in dry-run: `GiteaClient` construction
//! fails closed without a Bearer <redacted> (`GEECTL_GITEA_TOKEN` or
//! `GEECTL_GITEA_TOKEN_FILE`), so a dry-run preview cannot be built
//! without one. That is the library's rule, not a CLI shortcut.
//! Per-instance Gitea credentials (one token per configured Gitea
//! instance, on the alias model) are not part of this group: it
//! takes the single configured instance.
//!
//! The `branch` composite (Gitea fork + OBS `cmd=fork`, library
//! `GiteaClient::branch`) has no CLI command: it composes the
//! two backends behind the uniform flow, and its CLI shape is a
//! separate decision.
//!
//! Coordinate header (design rule 1):
//! every text rendering opens with one header line echoing the query
//! coordinates (`owner: … repo: …`, `repo: owner/name`), except the
//! content command `branch-head`, whose text IS the bare SHA.
//! Tables stay as designed.
//!
//! Documented omissions (not gaps): `repo-list` and `repo-search`
//! text carry no clone URLs (identifiers stay; the URLs are in the
//! JSON payload), no fork parent (the parent is in the JSON
//! payload), and no description (long free text, in the JSON
//! payload); `branch-list` text carries the full head SHA (the
//! sync-verification primitive) but no commit message (long free
//! text); `repo-get` text carries no description or website (long
//! free text) and `repo-create` text carries no description (long
//! free text). The JSON payload remains the complete record in every case.

use crate::auth::EnvGiteaAuth;
use crate::config::{self, DEFAULT_GITEA_URL};
use crate::debug::Debug;
use crate::error::GeectlError;
use crate::gitea::{
    require_credentials_for_url, ForkRepoOptions, GiteaBranch, GiteaClient, GiteaRepository,
    MaintainerInfo, Maintainership, RepoCreateOptions,
};
use crate::http::{HttpClient, Timeout, UreqClient};
use crate::obs::DryRun;
use clap::{Args, Subcommand};
use serde_json::{json, Value};

/// The `gitea` subcommands. Names are the library operation names
/// in kebab-case, grouped under the top-level noun.
#[derive(Debug, Subcommand)]
pub enum GiteaCommand {
    /// Show a repository: owner, default branch, privacy, clone URLs.
    RepoGet {
        /// The repository owner.
        owner: String,
        /// The repository name.
        repo: String,
    },
    /// List repositories: the token owner's own repos, one user's
    /// repos with `--owner`, one organization's with `--org`, or both
    /// (the user's first, then the organization's).
    RepoList(RepoListArgs),
    /// Search repositories across the instance by keyword: the
    /// keyword matches the name and the description. The Gitea
    /// counterpart of `remote search-packages` / `remote
    /// search-projects`.
    RepoSearch {
        /// The search keyword.
        query: String,
    },
    /// Create a repository under the token owner's account. A
    /// mutation: dry-run is the default (only the plan is printed);
    /// `--live` performs the creation.
    RepoCreate(RepoCreateArgs),
    /// Delete a repository. A mutation: dry-run is the default (only
    /// the plan is printed); `--live` performs the deletion. The
    /// library refuses without `--force`, even live.
    RepoDelete {
        /// The repository owner.
        owner: String,
        /// The repository name.
        repo: String,
        /// Confirm the deletion.
        #[arg(long)]
        force: bool,
    },
    /// Fork a repository into the token owner's account (or an
    /// organization with `--organization`). A mutation: dry-run is
    /// the default (only the plan is printed); `--live` performs
    /// the fork.
    RepoFork(RepoForkArgs),
    /// List the branches of a repository.
    BranchList {
        /// The repository owner.
        owner: String,
        /// The repository name.
        repo: String,
    },
    /// Show one branch: name, head commit, protection.
    BranchGet {
        /// The repository owner.
        owner: String,
        /// The repository name.
        repo: String,
        /// The branch name.
        branch: String,
    },
    /// Print the head commit SHA of a branch. Content command: the
    /// text IS the SHA, with no header.
    BranchHead {
        /// The repository owner.
        owner: String,
        /// The repository name.
        repo: String,
        /// The branch name.
        branch: String,
    },
    /// Show the maintainers recorded in a git project's
    /// `_maintainership.json`: the project maintainers and the
    /// per-package maintainers. The git-side counterpart of `remote
    /// maintainers`, read from the repo's default branch.
    Maintainers {
        /// The repository owner.
        owner: String,
        /// The repository name.
        repo: String,
    },
}

/// Arguments for `gitea repo-list`.
#[derive(Debug, Args)]
pub struct RepoListArgs {
    /// List this user's repositories instead of the token owner's.
    #[arg(long)]
    owner: Option<String>,
    /// List this organization's repositories instead of (or, with
    /// `--owner`, after) the user's.
    #[arg(long)]
    org: Option<String>,
}

/// Arguments for `gitea repo-create`.
#[derive(Debug, Args)]
pub struct RepoCreateArgs {
    /// The repository name.
    name: String,
    /// Create a private repository; public by default.
    #[arg(long)]
    private: bool,
    /// A short description.
    #[arg(long)]
    description: Option<String>,
    /// The default branch; the server default when unset.
    #[arg(long)]
    default_branch: Option<String>,
}

/// Arguments for `gitea repo-fork`.
#[derive(Debug, Args)]
pub struct RepoForkArgs {
    /// The repository owner.
    owner: String,
    /// The repository name.
    repo: String,
    /// Name for the fork; defaults to the source repo name.
    #[arg(long)]
    new_name: Option<String>,
    /// Organization to fork into; defaults to the token owner's
    /// account.
    #[arg(long)]
    organization: Option<String>,
}

/// Run a `gitea` command, returning the JSON payload and its text
/// rendering.
pub fn run_gitea(
    command: GiteaCommand,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = gitea_client(&http, debug)?;
    let dry_run = DryRun::new(!live).with_debug(debug.clone());
    match command {
        GiteaCommand::RepoGet { owner, repo } => {
            let repository = client.repo_get(&owner, &repo)?;
            let payload = gitea_repository_json(&repository);
            Ok((payload, render_repository(&repository)))
        }
        GiteaCommand::RepoList(args) => {
            let repos = client.repo_list(args.owner.as_deref(), args.org.as_deref())?;
            let payload = json!({
                "owner": args.owner,
                "org": args.org,
                "repositories": repos.iter().map(gitea_repository_json).collect::<Vec<_>>(),
            });
            Ok((
                payload,
                with_header(&repo_list_header(&args), render_repositories(&repos)),
            ))
        }
        GiteaCommand::RepoSearch { query } => {
            let repos = client.repo_search(&query)?;
            let payload = json!({
                "query": query,
                "repositories": repos.iter().map(gitea_repository_json).collect::<Vec<_>>(),
            });
            Ok((
                payload,
                with_header(&repo_search_header(&query), render_repositories(&repos)),
            ))
        }
        GiteaCommand::RepoCreate(args) => {
            let options = RepoCreateOptions {
                private: args.private,
                description: args.description.clone(),
                default_branch: args.default_branch.clone(),
            };
            let created = client.repo_create(&args.name, &options, &dry_run)?;
            let payload = json!({
                "name": args.name,
                "private": args.private,
                "description": args.description,
                "default_branch": args.default_branch,
                "repository": created.as_ref().map(gitea_repository_json),
            });
            let result_line = repo_create_result_line(
                created.as_ref(),
                &args.name,
                args.private,
                &args.default_branch,
            );
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(mutation_result(
                payload,
                format!("repo: {}", args.name),
                result_line,
                dry_run.enabled(),
                plan,
            ))
        }
        GiteaCommand::RepoDelete { owner, repo, force } => {
            let deleted = client.repo_delete(&owner, &repo, force, &dry_run)?;
            let payload = json!({
                "owner": owner,
                "repo": repo,
                "force": force,
                "deleted": deleted,
            });
            let result_line = match &deleted {
                Some(name) => format!("deleted repository {owner}/{name}"),
                None => format!("dry-run: would delete repository {owner}/{repo}"),
            };
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(mutation_result(
                payload,
                format!("owner: {owner} repo: {repo}"),
                result_line,
                dry_run.enabled(),
                plan,
            ))
        }
        GiteaCommand::RepoFork(args) => {
            let options = ForkRepoOptions {
                new_name: args.new_name.clone(),
                organization: args.organization.clone(),
            };
            let forked = client.fork_repo(&args.owner, &args.repo, &options, &dry_run)?;
            let payload = json!({
                "owner": args.owner,
                "repo": args.repo,
                "new_name": args.new_name,
                "organization": args.organization,
                "repository": forked.as_ref().map(gitea_repository_json),
            });
            let result_line = repo_fork_result_line(forked.as_ref(), &args.owner, &args.repo);
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(mutation_result(
                payload,
                format!("owner: {} repo: {}", args.owner, args.repo),
                result_line,
                dry_run.enabled(),
                plan,
            ))
        }
        GiteaCommand::BranchList { owner, repo } => {
            let branches = client.branch_list(&owner, &repo)?;
            let payload = json!({
                "owner": owner,
                "repo": repo,
                "branches": branches.iter().map(gitea_branch_json).collect::<Vec<_>>(),
            });
            Ok((
                payload,
                with_header(
                    &format!("owner: {owner} repo: {repo}"),
                    render_branches(&branches),
                ),
            ))
        }
        GiteaCommand::BranchGet {
            owner,
            repo,
            branch,
        } => {
            let branch = client.branch_get(&owner, &repo, &branch)?;
            let payload = gitea_branch_json(&branch);
            Ok((
                payload,
                with_header(
                    &format!("owner: {owner} repo: {repo}"),
                    render_branch(&branch),
                ),
            ))
        }
        GiteaCommand::BranchHead {
            owner,
            repo,
            branch,
        } => {
            let head = client.branch_head(&owner, &repo, &branch)?;
            let payload = json!({
                "owner": owner,
                "repo": repo,
                "branch": branch,
                "head": head,
            });
            // Content command: the text IS the SHA.
            Ok((payload, head))
        }
        GiteaCommand::Maintainers { owner, repo } => {
            let maintainership = client.maintainership(&owner, &repo)?;
            Ok(maintainers_result(&owner, &repo, &maintainership))
        }
    }
}

/// A `GiteaClient` for the configured Gitea instance (config file /
/// env / default) with the Bearer token resolved for that URL — the
/// matching alias's variables when the URL is named in `[instances]`,
/// the plain variables otherwise — reporting on the debug channel.
/// Construction fails closed without a token — even in dry-run, even
/// for reads (see the `gitea` group's module docs). The one
/// construction for the `gitea` group, the `pull-request` group's
/// Gitea backend, and the `meta` group's Gitea arm, so the credential
/// resolution cannot drift between them.
pub(crate) fn gitea_client<'a>(
    http: &'a dyn HttpClient,
    debug: &Debug,
) -> Result<GiteaClient<'a>, GeectlError> {
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let effective = config::effective_config(&file_config)?;
    // Unreachable by construction — effective_config always yields
    // Some(gitea_url) — but kept as a defensive default rather than
    // an unwrap: a config change must not turn into a panic here.
    let base = effective
        .config
        .gitea_url
        .as_deref()
        .unwrap_or(DEFAULT_GITEA_URL);
    // Both arguments are `base`: it is the effective URL
    // (environment, then config file, then default), so the plain
    // variables belong to it. A second URL must never be resolved
    // here as its own default — resolve it with this `base` as
    // `default_url` instead, or the default's token would be
    // forwarded to that host.
    let credentials = require_credentials_for_url(
        EnvGiteaAuth::credentials_for_url(base, base, &file_config.instances)?,
        base,
        base,
        &file_config.instances,
    )?;
    GiteaClient::with_base(http, base, credentials).map(|client| client.with_debug(debug.clone()))
}

/// The `repo-create` result line: the created repository's full
/// name when live, the requested name in dry-run — plus `(private)`
/// when the `--private` flag is passed and the default branch when
/// given. The description is long free text and stays JSON-only.
fn repo_create_result_line(
    created: Option<&GiteaRepository>,
    name: &str,
    private: bool,
    default_branch: &Option<String>,
) -> String {
    let mut options = Vec::new();
    if private {
        options.push("private".to_string());
    }
    if let Some(branch) = default_branch {
        options.push(format!("default branch {branch}"));
    }
    let suffix = if options.is_empty() {
        String::new()
    } else {
        format!(" ({})", options.join(", "))
    };
    match created {
        Some(repository) => format!("created repository {}{suffix}", repository.full_name),
        None => format!("dry-run: would create repository {name}{suffix}"),
    }
}

/// The `repo-fork` result line: the forked repository's full name
/// when live — which carries the new name and the organization —
/// the source coordinates in dry-run (the plan names the fork
/// target).
fn repo_fork_result_line(forked: Option<&GiteaRepository>, owner: &str, repo: &str) -> String {
    match forked {
        Some(repository) => format!(
            "forked repository {owner}/{repo} to {}",
            repository.full_name
        ),
        None => format!("dry-run: would fork repository {owner}/{repo}"),
    }
}

/// Prepend the one-line coordinate header to a text rendering
/// (design rule 1): `header` first, then the body unchanged.
fn with_header(header: &str, text: String) -> String {
    format!("{header}\n{text}")
}

/// Combine a mutation payload and its text lines: `dry_run` joins the
/// payload and goes right after the header line (the branch
/// renderer's order); the plan, when present, joins the payload and
/// follows the header and result lines verbatim.
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

/// The `repo-list` coordinate header: the filters given, or the
/// token owner's own repos when neither is given.
fn repo_list_header(args: &RepoListArgs) -> String {
    let mut parts = Vec::new();
    match &args.owner {
        Some(owner) => parts.push(format!("owner: {owner}")),
        None if args.org.is_none() => parts.push("owner: own".to_string()),
        None => {}
    }
    if let Some(org) = &args.org {
        parts.push(format!("org: {org}"));
    }
    parts.join(" ")
}

/// The `repo-search` coordinate header: the keyword searched for.
fn repo_search_header(query: &str) -> String {
    format!("query: {query}")
}

fn gitea_repository_json(repository: &GiteaRepository) -> Value {
    json!({
        "owner": { "login": repository.owner.login },
        "name": repository.name,
        "full_name": repository.full_name,
        "parent": repository.parent.as_ref().map(|parent| json!({
            "owner": { "login": parent.owner.login },
            "name": parent.name,
            "full_name": parent.full_name,
        })),
        "default_branch": repository.default_branch,
        "private": repository.private,
        "description": repository.description,
        "website": repository.website,
        "clone_url": repository.clone_url,
        "ssh_url": repository.ssh_url,
    })
}

fn gitea_branch_json(branch: &GiteaBranch) -> Value {
    json!({
        "name": branch.name,
        "commit": {
            "id": branch.commit.id,
            "message": branch.commit.message,
        },
        "protected": branch.protected,
    })
}

/// A repository as `key: value` lines. The first line already is
/// the coordinate (`repo: owner/name`), so no separate header.
fn render_repository(repository: &GiteaRepository) -> String {
    let mut lines = vec![format!("repo: {}", repository.full_name)];
    if let Some(parent) = &repository.parent {
        lines.push(format!("parent: {}", parent.full_name));
    }
    lines.push(format!("default branch: {}", repository.default_branch));
    lines.push(format!(
        "private: {}",
        if repository.private { "yes" } else { "no" }
    ));
    lines.push(format!("clone: {}", repository.clone_url));
    lines.push(format!("ssh: {}", repository.ssh_url));
    lines.join("\n")
}

fn render_repositories(repositories: &[GiteaRepository]) -> String {
    if repositories.is_empty() {
        return "no repositories".to_string();
    }
    let rows: Vec<Vec<String>> = repositories
        .iter()
        .map(|repository| {
            vec![
                repository.full_name.clone(),
                repository.default_branch.clone(),
                if repository.private {
                    "yes".to_string()
                } else {
                    "no".to_string()
                },
            ]
        })
        .collect();
    render_table(&["NAME", "DEFAULT BRANCH", "PRIVATE"], &rows)
}

fn render_branches(branches: &[GiteaBranch]) -> String {
    if branches.is_empty() {
        return "no branches".to_string();
    }
    let rows: Vec<Vec<String>> = branches
        .iter()
        .map(|branch| {
            vec![
                branch.name.clone(),
                branch.commit.id.clone(),
                if branch.protected {
                    "yes".to_string()
                } else {
                    "no".to_string()
                },
            ]
        })
        .collect();
    render_table(&["NAME", "HEAD", "PROTECTED"], &rows)
}

/// A branch as `key: value` lines. The first line already is the
/// coordinate (`branch: <name>`), so no separate header. The commit
/// message stays when the server sent one.
fn render_branch(branch: &GiteaBranch) -> String {
    let mut lines = vec![format!("branch: {}", branch.name)];
    lines.push(format!("commit: {}", branch.commit.id));
    if !branch.commit.message.is_empty() {
        lines.push(format!("message: {}", branch.commit.message));
    }
    lines.push(format!(
        "protected: {}",
        if branch.protected { "yes" } else { "no" }
    ));
    lines.join("\n")
}

/// The maintainership read as its JSON payload and its text
/// rendering: the whole document in both — the typed form is the
/// document, so the payload carries the project scope and every
/// package scope, and the text lists them one per line.
fn maintainers_result(owner: &str, repo: &str, maintainership: &Maintainership) -> (Value, String) {
    (
        maintainership_json(owner, repo, maintainership),
        with_header(
            &format!("owner: {owner} repo: {repo}"),
            render_maintainership(maintainership),
        ),
    )
}

fn maintainership_json(owner: &str, repo: &str, maintainership: &Maintainership) -> Value {
    let mut packages = serde_json::Map::new();
    for (name, info) in &maintainership.packages {
        packages.insert(name.clone(), maintainer_info_json(info));
    }
    json!({
        "owner": owner,
        "repo": repo,
        "version": maintainership.version,
        "project": maintainer_info_json(&maintainership.project),
        "packages": packages,
    })
}

fn maintainer_info_json(info: &MaintainerInfo) -> Value {
    json!({
        "users": info.users,
        "groups": info.groups,
    })
}

/// The maintainership as `key: value` lines: the project scope
/// first, then one line per package (sorted). An empty scope reads
/// `-`; a document with no maintainers at all reads `no
/// maintainers`.
fn render_maintainership(maintainership: &Maintainership) -> String {
    if maintainership.project.names().is_empty() && maintainership.packages.is_empty() {
        return "no maintainers".to_string();
    }
    let mut lines = vec![format!(
        "project: {}",
        maintainer_names_or_dash(&maintainership.project)
    )];
    for (package, info) in &maintainership.packages {
        lines.push(format!("{package}: {}", maintainer_names_or_dash(info)));
    }
    lines.join("\n")
}

/// One scope's maintainers as a comma-separated list (`@`-prefixed
/// groups), or `-` when the scope has none.
fn maintainer_names_or_dash(info: &MaintainerInfo) -> String {
    let names = info.names();
    if names.is_empty() {
        "-".to_string()
    } else {
        names.join(", ")
    }
}

/// Render rows as a space-aligned table with a header row. Widths are
/// the widest value in the printed set, in characters.
fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|header| header.chars().count())
        .collect();
    for row in rows {
        // A row longer than the headers would panic on the width
        // lookup below; fail with a clear message instead of an
        // index-out-of-bounds panic in the shipped binary.
        assert_eq!(
            row.len(),
            headers.len(),
            "row has {} cells, headers have {}",
            row.len(),
            headers.len()
        );
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }
    let mut lines = vec![join_row(headers, &widths)];
    for row in rows {
        let cells: Vec<&str> = row.iter().map(String::as_str).collect();
        lines.push(join_row(&cells, &widths));
    }
    lines.join("\n")
}

fn join_row(cells: &[&str], widths: &[usize]) -> String {
    cells
        .iter()
        .enumerate()
        .map(|(index, cell)| format!("{:<width$}", cell, width = widths[index]))
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Command};
    use crate::gitea::{GiteaBranchCommit, GiteaOwner, GiteaParent};
    use clap::Parser;

    fn parse(args: &[&str]) -> GiteaCommand {
        match Cli::parse_from(args).command {
            Some(Command::Gitea { command }) => command,
            _ => panic!("expected a gitea command, got {:?}", args),
        }
    }

    #[test]
    fn gitea_maintainers_command_parses() {
        assert!(matches!(
            parse(&["geectl", "gitea", "maintainers", "pool", "systemd"]),
            GiteaCommand::Maintainers { .. }
        ));
    }

    fn maintainership_fixture() -> Maintainership {
        Maintainership {
            version: "1.0".to_string(),
            project: MaintainerInfo {
                users: vec!["alice".to_string()],
                groups: vec!["team".to_string()],
            },
            packages: [
                (
                    "package1".to_string(),
                    MaintainerInfo {
                        users: vec!["bob".to_string()],
                        groups: vec!["pkg1-group".to_string()],
                    },
                ),
                (
                    "package2".to_string(),
                    MaintainerInfo {
                        users: vec!["charlie".to_string()],
                        groups: vec![],
                    },
                ),
            ]
            .into_iter()
            .collect(),
        }
    }

    #[test]
    fn maintainers_result_renders_project_and_package_lines() {
        let (payload, text) = maintainers_result("pool", "systemd", &maintainership_fixture());
        assert_eq!(payload["owner"], json!("pool"));
        assert_eq!(payload["repo"], json!("systemd"));
        assert_eq!(payload["version"], json!("1.0"));
        assert_eq!(payload["project"]["users"], json!(["alice"]));
        assert_eq!(payload["project"]["groups"], json!(["team"]));
        assert_eq!(
            payload["packages"]["package1"]["groups"],
            json!(["pkg1-group"])
        );
        assert_eq!(payload["packages"]["package2"]["users"], json!(["charlie"]));
        assert_eq!(
            text,
            "owner: pool repo: systemd\nproject: alice, @team\npackage1: bob, @pkg1-group\npackage2: charlie"
        );
    }

    #[test]
    fn maintainers_result_empty_is_no_maintainers() {
        let empty = Maintainership {
            version: "1.0".to_string(),
            project: MaintainerInfo::default(),
            packages: Default::default(),
        };
        let (payload, text) = maintainers_result("pool", "systemd", &empty);
        assert_eq!(payload["packages"], json!({}));
        assert_eq!(text, "owner: pool repo: systemd\nno maintainers");
    }

    #[test]
    fn gitea_commands_parse() {
        assert!(matches!(
            parse(&["geectl", "gitea", "repo-get", "pool", "systemd"]),
            GiteaCommand::RepoGet { .. }
        ));
        let parsed = parse(&[
            "geectl",
            "gitea",
            "repo-list",
            "--owner",
            "alice",
            "--org",
            "pool",
        ]);
        assert!(matches!(
            parsed,
            GiteaCommand::RepoList(RepoListArgs {
                owner: Some(owner),
                org: Some(org),
            }) if owner == "alice" && org == "pool"
        ));
        assert!(matches!(
            parse(&["geectl", "gitea", "repo-search", "geectl"]),
            GiteaCommand::RepoSearch { query } if query == "geectl"
        ));
        let parsed = parse(&[
            "geectl",
            "gitea",
            "repo-create",
            "vim",
            "--private",
            "--description",
            "Vim, the editor",
            "--default-branch",
            "factory",
        ]);
        assert!(matches!(
            parsed,
            GiteaCommand::RepoCreate(RepoCreateArgs {
                private: true,
                description: Some(description),
                default_branch: Some(branch),
                ..
            }) if description == "Vim, the editor" && branch == "factory"
        ));
        assert!(matches!(
            parse(&["geectl", "gitea", "repo-delete", "pool", "vim", "--force"]),
            GiteaCommand::RepoDelete { force: true, .. }
        ));
        let parsed = parse(&[
            "geectl",
            "gitea",
            "repo-fork",
            "pool",
            "systemd",
            "--new-name",
            "systemd-fork",
            "--organization",
            "pool",
        ]);
        assert!(matches!(
            parsed,
            GiteaCommand::RepoFork(RepoForkArgs {
                new_name: Some(name),
                organization: Some(org),
                ..
            }) if name == "systemd-fork" && org == "pool"
        ));
        assert!(matches!(
            parse(&["geectl", "gitea", "branch-list", "pool", "systemd"]),
            GiteaCommand::BranchList { .. }
        ));
        assert!(matches!(
            parse(&[
                "geectl",
                "gitea",
                "branch-get",
                "pool",
                "systemd",
                "factory"
            ]),
            GiteaCommand::BranchGet { .. }
        ));
        assert!(matches!(
            parse(&[
                "geectl",
                "gitea",
                "branch-head",
                "pool",
                "systemd",
                "factory"
            ]),
            GiteaCommand::BranchHead { .. }
        ));
    }

    #[test]
    fn unknown_gitea_command_fails_to_parse() {
        assert!(Cli::try_parse_from(["geectl", "gitea", "frobnicate"]).is_err());
        assert!(Cli::try_parse_from(["geectl", "gitea", "repo-get", "only-one"]).is_err());
    }

    fn sample_repository() -> GiteaRepository {
        GiteaRepository {
            owner: GiteaOwner {
                login: "pool".to_string(),
            },
            name: "systemd".to_string(),
            full_name: "pool/systemd".to_string(),
            parent: None,
            default_branch: "factory".to_string(),
            private: false,
            description: None,
            website: None,
            clone_url: "https://src.opensuse.org/pool/systemd.git".to_string(),
            ssh_url: "git@src.opensuse.org:pool/systemd.git".to_string(),
        }
    }

    fn sample_fork() -> GiteaRepository {
        let mut fork = sample_repository();
        fork.owner = GiteaOwner {
            login: "alice".to_string(),
        };
        fork.name = "systemd".to_string();
        fork.full_name = "alice/systemd".to_string();
        fork.parent = Some(GiteaParent {
            owner: GiteaOwner {
                login: "pool".to_string(),
            },
            name: "systemd".to_string(),
            full_name: "pool/systemd".to_string(),
        });
        fork
    }

    fn sample_branch() -> GiteaBranch {
        GiteaBranch {
            name: "factory".to_string(),
            commit: GiteaBranchCommit {
                id: "0123456789abcdef0123456789abcdef01234567".to_string(),
                message: "Update systemd".to_string(),
            },
            protected: true,
        }
    }

    #[test]
    fn repo_list_header_echoes_the_filters() {
        assert_eq!(
            repo_list_header(&RepoListArgs {
                owner: None,
                org: None,
            }),
            "owner: own"
        );
        assert_eq!(
            repo_list_header(&RepoListArgs {
                owner: Some("alice".to_string()),
                org: None,
            }),
            "owner: alice"
        );
        assert_eq!(
            repo_list_header(&RepoListArgs {
                owner: None,
                org: Some("pool".to_string()),
            }),
            "org: pool"
        );
        assert_eq!(
            repo_list_header(&RepoListArgs {
                owner: Some("alice".to_string()),
                org: Some("pool".to_string()),
            }),
            "owner: alice org: pool"
        );
    }

    #[test]
    fn repository_renders_parent_when_a_fork() {
        let text = render_repository(&sample_fork());
        assert!(text.starts_with("repo: alice/systemd"));
        assert!(text.contains("parent: pool/systemd"));
        assert!(text.contains("default branch: factory"));
        assert!(text.contains("private: no"));
        assert!(text.contains("clone: https://src.opensuse.org/pool/systemd.git"));
        let payload = gitea_repository_json(&sample_fork());
        assert_eq!(payload["parent"]["full_name"], "pool/systemd");
        assert_eq!(payload["parent"]["owner"]["login"], "pool");
        assert_eq!(payload["private"], false);
        let plain = render_repository(&sample_repository());
        assert!(!plain.contains("parent:"));
    }

    #[test]
    fn repositories_render_a_table_without_clone_urls() {
        let text = render_repositories(&[sample_repository(), sample_fork()]);
        assert!(text.contains("pool/systemd"));
        assert!(text.contains("alice/systemd"));
        assert!(!text.contains("src.opensuse.org"));
        assert_eq!(render_repositories(&[]), "no repositories");
        // The fork parent is JSON-only: a fork-only list carries the
        // fork's own name and the PRIVATE column, never the parent.
        let fork_only = render_repositories(&[sample_fork()]);
        assert!(fork_only.contains("alice/systemd"));
        assert!(!fork_only.contains("pool/systemd"));
        assert!(fork_only.contains("PRIVATE"));
        let row = fork_only
            .lines()
            .find(|line| line.contains("alice/systemd"))
            .expect("fork row present");
        assert!(row.ends_with("no"), "{row}");
    }

    #[test]
    fn repo_search_text_carries_the_query_header() {
        assert_eq!(repo_search_header("geectl"), "query: geectl");
        let text = with_header(
            &repo_search_header("geectl"),
            render_repositories(&[sample_repository(), sample_fork()]),
        );
        assert!(text.starts_with("query: geectl\n"));
        assert!(text.contains("pool/systemd"));
        assert!(text.contains("alice/systemd"));
        let empty = with_header(&repo_search_header("nobody"), render_repositories(&[]));
        assert_eq!(empty, "query: nobody\nno repositories");
    }

    #[test]
    fn branch_renders_head_and_protection() {
        let text = render_branch(&sample_branch());
        assert!(text.starts_with("branch: factory"));
        assert!(text.contains("commit: 0123456789abcdef0123456789abcdef01234567"));
        assert!(text.contains("message: Update systemd"));
        assert!(text.contains("protected: yes"));
        let payload = gitea_branch_json(&sample_branch());
        assert_eq!(
            payload["commit"]["id"],
            "0123456789abcdef0123456789abcdef01234567"
        );
        assert_eq!(payload["protected"], true);
        assert_eq!(render_branches(&[]), "no branches");
    }

    #[test]
    fn mutation_result_carries_dry_run_and_plan_in_order() {
        let payload = json!({ "owner": "pool", "repo": "systemd" });
        let (payload, text) = mutation_result(
            payload,
            "owner: pool repo: systemd".to_string(),
            "dry-run: would delete repository pool/systemd".to_string(),
            true,
            Some(vec!["delete repository pool/systemd".to_string()]),
        );
        assert_eq!(payload["dry_run"], true);
        assert_eq!(payload["plan"][0], "delete repository pool/systemd");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "owner: pool repo: systemd");
        assert_eq!(lines[1], "dry_run: true");
        assert!(lines[2].starts_with("dry-run: would delete"));
        assert_eq!(lines[3], "delete repository pool/systemd");
    }

    #[test]
    fn repo_create_result_line_carries_privacy_and_default_branch() {
        let fork = sample_fork();
        assert_eq!(
            repo_create_result_line(Some(&fork), "systemd", false, &None),
            "created repository alice/systemd"
        );
        assert_eq!(
            repo_create_result_line(Some(&fork), "systemd", true, &Some("factory".to_string())),
            "created repository alice/systemd (private, default branch factory)"
        );
        assert_eq!(
            repo_create_result_line(None, "systemd", false, &None),
            "dry-run: would create repository systemd"
        );
        assert_eq!(
            repo_create_result_line(None, "systemd", true, &Some("main".to_string())),
            "dry-run: would create repository systemd (private, default branch main)"
        );
    }

    #[test]
    fn repo_fork_result_line_carries_the_fork_target() {
        let fork = sample_fork();
        assert_eq!(
            repo_fork_result_line(Some(&fork), "pool", "systemd"),
            "forked repository pool/systemd to alice/systemd"
        );
        assert_eq!(
            repo_fork_result_line(None, "pool", "systemd"),
            "dry-run: would fork repository pool/systemd"
        );
    }

    /// The agreement test: every id, name, and revision the JSON
    /// payload carries appears in the text rendering, except the
    /// documented omissions (clone URLs in `repo-list`, commit
    /// messages in `branch-list`).
    #[test]
    fn text_carries_every_identifying_field() {
        let repo = sample_fork();
        let text = render_repository(&repo);
        for field in [
            "alice/systemd",
            "pool/systemd",
            "factory",
            "https://src.opensuse.org/pool/systemd.git",
            "git@src.opensuse.org:pool/systemd.git",
        ] {
            assert!(text.contains(field), "repo text misses {field}:\n{text}");
        }
        let text = render_repositories(&[sample_repository(), sample_fork()]);
        for field in ["pool/systemd", "alice/systemd", "factory"] {
            assert!(
                text.contains(field),
                "repo list text misses {field}:\n{text}"
            );
        }
        assert!(!text.contains("src.opensuse.org"));

        let branch = sample_branch();
        let text = render_branches(std::slice::from_ref(&branch));
        assert!(text.contains("factory"));
        assert!(text.contains("0123456789abcdef0123456789abcdef01234567"));
        assert!(text.contains("yes"));
        assert!(!text.contains("Update systemd"));
        let text = render_branch(&branch);
        for field in [
            "factory",
            "0123456789abcdef0123456789abcdef01234567",
            "Update systemd",
            "protected: yes",
        ] {
            assert!(text.contains(field), "branch text misses {field}:\n{text}");
        }
    }
}
