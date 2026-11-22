//! Remote-read CLI commands (`geectl remote ...`).
//!
//! The read surface for inspecting what the distribution holds: file
//! contents, listings, history, meta, search, ownership, and diffs —
//! plus the group's two mutations, `repo-add` and `repo-remove`,
//! which edit a project's repository list in its meta.
//! Each command is a thin wrapper over one `ObsClient` operation: the
//! flags mirror the library parameters, the JSON payload carries the
//! typed values, and the text renderer prints the same values in a
//! human-readable form. No XML reaches the user anywhere.
//!
//! The command surface follows osc's read commands where they exist
//! (`cat`, `ls`, `log`, `meta`, `search`, `maintainer`, `whois`,
//! `develproject`), with the deliberate deviations listed in the CLI
//! swaps table (see `docs/cli-swaps.md`): one
//! command per library operation instead of osc's overloaded commands,
//! explicit project/package arguments everywhere (no working-copy
//! context) — except `devel-project`, which is Factory-only
//! and takes only the package — and one flag name per concept.
//!
//! Two diffs differ from osc by mechanism, not just by name. osc
//! `rdiff` is a server-side diff; geectl's `revision-patch` and
//! `project-diff` are computed client-side from per-file reads (the
//! library lists and cats the files itself), so a large `project-diff`
//! issues many requests and a differing binary file fails the call.
//! And osc `rdiff`'s two-package form has no counterpart here at all:
//! the library has no public two-package diff op (see issue #74).
//!
//! Coordinate header (design rule 1):
//! every text rendering opens with one header line echoing the query
//! coordinates — the required positionals, in the order project,
//! package (`project: p package: k`), or the command's own coordinate
//! where the query has no project (`term:`, `login:`, `binary:`).
//! Tables stay as designed; the header is the only addition. Two
//! commands are content commands and stay verbatim (design rule 4):
//! `cat` (the file itself) and the diffs (`revision-patch`,
//! `project-diff`, `diff-packages`) — a synthetic empty line (`no changes …`,
//! `no packages`) still carries the header. Three single-object
//! renders need no separate header because their first line already
//! is the coordinate: `project-meta` (`name:`), `package-meta`
//! (`name:` + `project:`) and `whois` (`login:`). `list-projects`
//! carries no header either: the query names no project, package,
//! or term to echo, so the table is the whole rendering.
//!
//! The group's two mutations (`repo-add`, `repo-remove`) follow the
//! CLI-wide rule: dry-run by default, the global `--live` performs
//! them, and the text opens with the coordinate header, then
//! `dry_run:`, then the result line and the plan.
//!
//! Documented omissions (not gaps): `list-files` text
//! carries no md5 (a verification value, not an identifier);
//! `package-log` text carries no comment or srcmd5 (long free text
//! and a verification value); search tables carry no description
//! (long free text — titles stay); `revision-patch`,
//! `project-diff`, and `diff-packages` diffs are the server text
//! itself, not re-rendered.
//! The JSON payload remains the complete record in every case.

use clap::Subcommand;
use geectl::auth::{Credentials, EnvObsAuth};
use geectl::config;
use geectl::debug::Debug;
use geectl::error::{ErrorKind, GeectlError};
use geectl::http::{Timeout, UreqClient};
use geectl::obs::{
    DryRun, FilePatch, FilePatchStatus, HistoryEntry, NamedRole, ObsClient, Owner, PackageDiff,
    PackageDiffStatus, PackageMatch, PackageMeta, Person, ProjectMatch, ProjectMeta, Repository,
    RevisionPatch, RevisionRange, SourceFile, FACTORY_PROJECT, OBS_API_BASE,
};
use serde_json::{json, Value};

/// The `remote` command group: read-only inspection of the OBS
/// source tree.
#[derive(Debug, Subcommand)]
pub enum RemoteCommand {
    /// Print a source file to standard output. Text only: the
    /// library reads the file as a UTF-8 string, so a binary file
    /// fails the call with a server error ("response body is not
    /// valid UTF-8") instead of printing bytes.
    Cat {
        /// The project the package lives in.
        project: String,
        /// The package the file lives in.
        package: String,
        /// The file to print.
        file: String,
        /// Print the file as of this revision (default: latest).
        #[arg(long)]
        rev: Option<u64>,
    },
    /// List the files of a package.
    ListFiles {
        /// The project the package lives in.
        project: String,
        /// The package to list.
        package: String,
    },
    /// List the packages of a project.
    ListPackages {
        /// The project to list.
        project: String,
    },
    /// List the top-level projects on the instance.
    ListProjects,
    /// Show the commit history of a package.
    PackageLog {
        /// The project the package lives in.
        project: String,
        /// The package to show the history of.
        package: String,
        /// Only entries in this revision range (`FROM:TO`, either side
        /// may be empty; a side is a revision number or a srcmd5).
        #[arg(long)]
        range: Option<String>,
    },
    /// Show the per-file diff of one revision against the previous one.
    /// Computed client-side from per-file reads (the library lists both
    /// revisions and cats each changed file), not a server-side diff;
    /// a differing binary file fails the call.
    RevisionPatch {
        /// The project the package lives in.
        project: String,
        /// The package to diff.
        package: String,
        /// The revision whose changes to show.
        rev: u64,
    },
    /// Show a project's meta (title, description, persons, repositories).
    ProjectMeta {
        /// The project to show.
        project: String,
    },
    /// Show a package's meta (title, description, persons, devel project).
    PackageMeta {
        /// The project the package lives in.
        project: String,
        /// The package to show.
        package: String,
    },
    /// Search packages by name. Substring matches on name, title, and
    /// description by default; `--exact` keeps only exact-name matches
    /// (exact matching is not provided server-side, so the filter runs
    /// client-side).
    SearchPackages {
        /// The search term.
        term: String,
        /// Only show packages whose name is exactly the term.
        #[arg(long)]
        exact: bool,
    },
    /// Search projects by name. Substring matches on name, title, and
    /// description by default; `--exact` keeps only exact-name matches
    /// (exact matching is not provided server-side, so the filter runs
    /// client-side).
    SearchProjects {
        /// The search term.
        term: String,
        /// Only show projects whose name is exactly the term.
        #[arg(long)]
        exact: bool,
    },
    /// Show the owners of a package (project/package scope).
    Owners {
        /// The project the package lives in.
        project: String,
        /// The package to show the owners of.
        package: String,
    },
    /// Show the owners of a binary package.
    OwnersForBinary {
        /// The binary package name.
        binary: String,
    },
    /// Show the maintainers of a package.
    Maintainers {
        /// The project the package lives in.
        project: String,
        /// The package to show the maintainers of.
        package: String,
    },
    /// Show a user's full name, email, and account state.
    Whois {
        /// The login name to look up.
        login: String,
    },
    /// Show the link target of a package, when it is a link.
    LinkTarget {
        /// The project the package lives in.
        project: String,
        /// The package to show the link target of.
        package: String,
    },
    /// Diff two projects, package by package. Computed client-side:
    /// the library lists both projects and reads every file of every
    /// differing package, so a large diff issues many requests and a
    /// differing binary file fails the call.
    ProjectDiff {
        /// The old project.
        old: String,
        /// The new project.
        new: String,
        /// Only include packages with these exact names (repeatable).
        #[arg(long)]
        include: Vec<String>,
        /// Exclude packages with these exact names (repeatable).
        #[arg(long)]
        exclude: Vec<String>,
        /// Annotate differing packages with their open submit requests.
        #[arg(long)]
        annotate_requests: bool,
    },
    /// Diff two packages, file by file. Computed client-side: the
    /// library diffs the two file lists and cats each changed file,
    /// so a differing binary file fails the call.
    DiffPackages {
        /// The old project.
        old_project: String,
        /// The old package.
        old_package: String,
        /// The new project.
        new_project: String,
        /// The new package.
        new_package: String,
    },
    /// Show the devel project and devel package of a package on
    /// Factory, when it has one.
    DevelProject {
        /// The package to look up.
        package: String,
    },
    /// Add a repository to a project. A mutation: dry-run is the
    /// default (only the plan is printed); `--live` performs the
    /// addition.
    RepoAdd {
        /// The project to add the repository to.
        project: String,
        /// The repository name.
        name: String,
        /// An architecture the repository builds for (repeatable;
        /// at least one is required).
        #[arg(long, required = true)]
        arch: Vec<String>,
        /// A rebuild source as `PROJECT/REPOSITORY` (repeatable; at
        /// least one is required).
        #[arg(long, required = true)]
        path: Vec<String>,
        /// Disable publishing for the added repository.
        #[arg(long)]
        disable_publish: bool,
    },
    /// Remove a repository from a project. A mutation: dry-run is
    /// the default (only the plan is printed); `--live` performs
    /// the removal. A repository that is not there is not an error.
    RepoRemove {
        /// The project to remove the repository from.
        project: String,
        /// The repository name.
        name: String,
    },
}

/// Run a `remote` command, returning the JSON payload and its text
/// rendering. `live` performs the group's two mutations
/// (`repo-add`, `repo-remove`); the reads ignore it.
pub fn run_remote(
    command: RemoteCommand,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let (base, credentials) = obs_endpoint()?;
    let client =
        ObsClient::with_base_and_credentials(&http, &base, credentials)?.with_debug(debug.clone());
    match command {
        RemoteCommand::Cat {
            project,
            package,
            file,
            rev,
        } => {
            let content = client.cat(&project, &package, &file, rev)?;
            // Content commands render verbatim: the text IS the file.
            // The writer terminates the payload with a trailing newline
            // (an already-terminated payload is untouched — see
            // `terminated` in main.rs), so a file ending in a newline
            // gains no extra blank line.
            Ok(cat_result(&project, &package, &file, rev, content))
        }
        RemoteCommand::ListFiles { project, package } => {
            let files = client.list_files(&project, &package)?;
            let payload = json!({
                "project": project,
                "package": package,
                "files": files.iter().map(source_file_json).collect::<Vec<_>>(),
            });
            Ok((
                payload,
                with_header(
                    &format!("project: {project} package: {package}"),
                    render_source_files(&files),
                ),
            ))
        }
        RemoteCommand::ListPackages { project } => {
            let packages = client.list_packages(&project)?;
            Ok(list_packages_result(&project, packages))
        }
        RemoteCommand::ListProjects => {
            let projects = client.list_projects()?;
            let payload = json!({
                "projects": projects,
            });
            Ok((payload, render_project_names(&projects)))
        }
        RemoteCommand::PackageLog {
            project,
            package,
            range,
        } => {
            let parsed = range.as_deref().map(RevisionRange::parse).transpose()?;
            let entries = client.package_log(&project, &package, parsed.as_ref())?;
            let payload = json!({
                "project": project,
                "package": package,
                "range": range,
                "entries": entries.iter().map(history_entry_json).collect::<Vec<_>>(),
            });
            Ok((
                payload,
                with_header(
                    &format!("project: {project} package: {package}"),
                    render_history(&entries),
                ),
            ))
        }
        RemoteCommand::RevisionPatch {
            project,
            package,
            rev,
        } => {
            let patch = client.revision_patch(&project, &package, rev)?;
            let payload = json!({
                "project": project,
                "package": package,
                "rev": rev,
                "files": patch.files.iter().map(file_patch_json).collect::<Vec<_>>(),
            });
            let text = render_revision_patch(&patch);
            // Content stays verbatim; only the synthetic empty line
            // carries the header (design rule 1).
            let text = if patch.files.is_empty() {
                with_header(&format!("project: {project} package: {package}"), text)
            } else {
                text
            };
            Ok((payload, text))
        }
        RemoteCommand::ProjectMeta { project } => {
            let meta = client.project_meta(&project)?;
            let payload = project_meta_json(&meta);
            Ok((payload, render_project_meta(&meta)))
        }
        RemoteCommand::PackageMeta { project, package } => {
            let meta = client.package_meta(&project, &package)?;
            let payload = package_meta_json(&meta);
            Ok((payload, render_package_meta(&meta)))
        }
        RemoteCommand::SearchPackages { term, exact } => {
            let matches = apply_exact(client.search_packages(&term)?, &term, exact, |match_| {
                match_.name.as_str()
            });
            let payload = json!({
                "term": term,
                "exact": exact,
                "packages": matches.iter().map(package_match_json).collect::<Vec<_>>(),
            });
            Ok((
                payload,
                with_header(
                    &format!("term: {term}"),
                    render_package_matches(&term, &matches),
                ),
            ))
        }
        RemoteCommand::SearchProjects { term, exact } => {
            let matches = apply_exact(client.search_projects(&term)?, &term, exact, |match_| {
                match_.name.as_str()
            });
            let payload = json!({
                "term": term,
                "exact": exact,
                "projects": matches.iter().map(project_match_json).collect::<Vec<_>>(),
            });
            Ok((
                payload,
                with_header(
                    &format!("term: {term}"),
                    render_project_matches(&term, &matches),
                ),
            ))
        }
        RemoteCommand::Owners { project, package } => {
            let owners = client.owners(&project, &package)?;
            let payload = json!({
                "project": project,
                "package": package,
                "owners": owners.iter().map(owner_json).collect::<Vec<_>>(),
            });
            Ok((
                payload,
                with_header(
                    &format!("project: {project} package: {package}"),
                    render_owners(&owners),
                ),
            ))
        }
        RemoteCommand::OwnersForBinary { binary } => {
            let owners = client.owners_for_binary(&binary)?;
            let payload = json!({
                "binary": binary,
                "owners": owners.iter().map(owner_json).collect::<Vec<_>>(),
            });
            Ok((
                payload,
                with_header(&format!("binary: {binary}"), render_owners(&owners)),
            ))
        }
        RemoteCommand::Maintainers { project, package } => {
            let maintainers = client.maintainers(&project, &package)?;
            Ok(maintainers_result(&project, &package, maintainers))
        }
        RemoteCommand::Whois { login } => {
            let person = client.whois(&login)?;
            let payload = person_json(&person);
            Ok((payload, render_person(&person)))
        }
        RemoteCommand::LinkTarget { project, package } => {
            let target = client.link_target(&project, &package)?;
            Ok(link_target_result(&project, &package, &target))
        }
        RemoteCommand::ProjectDiff {
            old,
            new,
            include,
            exclude,
            annotate_requests,
        } => {
            let include_refs: Vec<&str> = include.iter().map(String::as_str).collect();
            let exclude_refs: Vec<&str> = exclude.iter().map(String::as_str).collect();
            let packages =
                client.project_diff(&old, &new, &include_refs, &exclude_refs, annotate_requests)?;
            let payload = json!({
                "old": old,
                "new": new,
                "include": include,
                "exclude": exclude,
                "annotate_requests": annotate_requests,
                "packages": packages.iter().map(package_diff_json).collect::<Vec<_>>(),
            });
            let text = render_project_diff(&packages);
            // Content stays verbatim; only the synthetic empty line
            // carries the header (design rule 1).
            let text = if packages.is_empty() {
                with_header(&format!("old: {old} new: {new}"), text)
            } else {
                text
            };
            Ok((payload, text))
        }
        RemoteCommand::DevelProject { package } => {
            // Factory-only by design (see the swaps table): the
            // library op takes the project explicitly since #77.
            let devel = client.devel_project(FACTORY_PROJECT, &package)?;
            Ok(devel_project_result(&package, devel))
        }
        RemoteCommand::DiffPackages {
            old_project,
            old_package,
            new_project,
            new_package,
        } => {
            let files =
                client.diff_packages(&old_project, &old_package, &new_project, &new_package)?;
            let payload = json!({
                "old_project": old_project,
                "old_package": old_package,
                "new_project": new_project,
                "new_package": new_package,
                "files": files.iter().map(file_patch_json).collect::<Vec<_>>(),
            });
            let text = render_diff_packages(&files);
            // Content stays verbatim; only the synthetic empty line
            // carries the header (design rule 1).
            let text = if files.is_empty() {
                with_header(
                    &format!("old: {old_project}/{old_package} new: {new_project}/{new_package}"),
                    text,
                )
            } else {
                text
            };
            Ok((payload, text))
        }
        RemoteCommand::RepoAdd {
            project,
            name,
            arch,
            path,
            disable_publish,
        } => {
            let paths = parse_repo_paths(&path)?;
            let repository = Repository {
                name: name.clone(),
                arch: arch.clone(),
                paths,
            };
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            let changed =
                client.add_repository(&project, &repository, disable_publish, &dry_run)?;
            let payload = json!({
                "project": project,
                "repository": repository_json(&repository),
                "disable_publish": disable_publish,
                "changed": changed,
            });
            let result_line = if dry_run.enabled() {
                format!("dry-run: would add repository {name} to project {project}")
            } else {
                format!("added repository {name} to project {project}")
            };
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(mutation_result(
                payload,
                format!("project: {project} repository: {name}"),
                result_line,
                dry_run.enabled(),
                plan,
            ))
        }
        RemoteCommand::RepoRemove { project, name } => {
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            let changed = client.remove_repository(&project, &name, &dry_run)?;
            let payload = json!({
                "project": project,
                "repository": name,
                "changed": changed,
            });
            let result_line = if dry_run.enabled() {
                format!("dry-run: would remove repository {name} from project {project}")
            } else if changed {
                format!("removed repository {name} from project {project}")
            } else {
                format!("repository {name} not present in project {project}")
            };
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(mutation_result(
                payload,
                format!("project: {project} repository: {name}"),
                result_line,
                dry_run.enabled(),
                plan,
            ))
        }
    }
}

/// The OBS base URL and credentials for a remote call, from the
/// resolved config (file, environment, then the default) and the
/// environment auth provider.
///
/// Credentials resolve up front for every remote command, even
/// though several remote reads are public and need none in the
/// library (`cat`, `list-files`, `list-packages`, `project-meta`,
/// `package-meta`, `link-target`, `devel-project`, `revision-patch`,
/// and `project-diff` — the last only without `--annotate-requests` —
/// all read via `Credentials::None`):
/// failing fast on a half-set credential env is a deliberate
/// choice — a genuine misconfiguration fails early, at startup,
/// rather than halfway through a run. Per-command lazy resolution
/// is follow-up work. This group differs from `request` deliberately:
/// remote commands are reads, so a half-set pair is a misconfiguration
/// to surface at startup; `request` swallows it in dry-run because a
/// request preview never needs credentials at all.
fn obs_endpoint() -> Result<(String, Credentials), GeectlError> {
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let effective = config::effective_config(&file_config)?;
    let base = effective
        .config
        .obs_url
        .unwrap_or_else(|| OBS_API_BASE.to_string());
    let credentials = EnvObsAuth::credentials_for_url(&base, &base, &file_config.instances)?;
    Ok((base, credentials))
}

// ---------------------------------------------------------------------
// Per-command result builders: pure functions of the typed values, so
// every command's payload and text are unit-testable without a server.
// ---------------------------------------------------------------------

/// Prepend the one-line coordinate header to a text rendering
/// (design rule 1): `header` first, then the body unchanged.
fn with_header(header: &str, text: String) -> String {
    format!("{header}\n{text}")
}

fn cat_result(
    project: &str,
    package: &str,
    file: &str,
    rev: Option<u64>,
    content: String,
) -> (Value, String) {
    let payload = json!({
        "project": project,
        "package": package,
        "file": file,
        "rev": rev,
        "content": content.clone(),
    });
    // Content commands render verbatim: the text IS the file.
    (payload, content)
}

fn list_packages_result(project: &str, packages: Vec<String>) -> (Value, String) {
    let payload = json!({
        "project": project,
        "packages": packages,
    });
    (
        payload,
        with_header(
            &format!("project: {project}"),
            render_package_names(&packages),
        ),
    )
}

fn render_package_names(packages: &[String]) -> String {
    if packages.is_empty() {
        return "no packages".to_string();
    }
    let rows: Vec<Vec<String>> = packages.iter().map(|name| vec![name.clone()]).collect();
    render_table(&["NAME"], &rows)
}

/// The top-level project names as a table. No coordinate header:
/// the query names no project, package, or term to echo, so the
/// table is the whole rendering.
fn render_project_names(projects: &[String]) -> String {
    if projects.is_empty() {
        return "no projects".to_string();
    }
    let rows: Vec<Vec<String>> = projects.iter().map(|name| vec![name.clone()]).collect();
    render_table(&["NAME"], &rows)
}

/// Combine a mutation payload and its text lines: `dry_run` joins
/// the payload and goes right after the header line (the branch
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

/// Parse `--path PROJECT/REPOSITORY` values into
/// `(project, repository)` pairs: the two sides split on the first
/// `/`. A value with no `/`, or an empty side, is an input error
/// naming the expected form.
fn parse_repo_paths(paths: &[String]) -> Result<Vec<(String, String)>, GeectlError> {
    paths
        .iter()
        .map(|path| match path.split_once('/') {
            Some((project, repository)) if !project.is_empty() && !repository.is_empty() => {
                Ok((project.to_string(), repository.to_string()))
            }
            _ => Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid repository path {path:?}: expected PROJECT/REPOSITORY"),
            )),
        })
        .collect()
}

fn maintainers_result(
    project: &str,
    package: &str,
    maintainers: Vec<NamedRole>,
) -> (Value, String) {
    let payload = json!({
        "project": project,
        "package": package,
        "maintainers": maintainers.iter().map(named_role_json).collect::<Vec<_>>(),
    });
    (
        payload,
        with_header(
            &format!("project: {project} package: {package}"),
            render_maintainers(&maintainers),
        ),
    )
}

fn render_maintainers(maintainers: &[NamedRole]) -> String {
    if maintainers.is_empty() {
        return "no maintainers".to_string();
    }
    let rows: Vec<Vec<String>> = maintainers
        .iter()
        .map(|role| vec![role.name.clone(), role.role.clone()])
        .collect();
    render_table(&["NAME", "ROLE"], &rows)
}

fn link_target_result(
    project: &str,
    package: &str,
    target: &Option<(String, String)>,
) -> (Value, String) {
    let payload = json!({
        "project": project,
        "package": package,
        "target": target.as_ref().map(|(project, package)| json!({
            "project": project,
            "package": package,
        })),
    });
    let text = match target {
        Some((project, package)) => format!("link: {project}/{package}"),
        None => "no link".to_string(),
    };
    (
        payload,
        with_header(&format!("project: {project} package: {package}"), text),
    )
}

fn devel_project_result(package: &str, devel: Option<(String, String)>) -> (Value, String) {
    let payload = json!({
        "package": package,
        "devel_project": devel.as_ref().map(|(project, _)| project),
        "devel_package": devel.as_ref().map(|(_, devel_package)| devel_package),
    });
    let text = match &devel {
        Some((project, devel_package)) => {
            format!("devel project: {project} package: {devel_package}")
        }
        None => "no devel project".to_string(),
    };
    (payload, with_header(&format!("package: {package}"), text))
}

/// Keep only exact-name matches when `exact` is set; otherwise the
/// matches as returned (substring matches on name, title, and
/// description). The library has no exact search, so the filter runs
/// here — this is what makes osc's default reachable from the CLI.
fn apply_exact<T>(
    matches: Vec<T>,
    term: &str,
    exact: bool,
    name_of: impl Fn(&T) -> &str,
) -> Vec<T> {
    if !exact {
        return matches;
    }
    matches
        .into_iter()
        .filter(|match_| name_of(match_) == term)
        .collect()
}

// ---------------------------------------------------------------------
// JSON payload builders: typed values only, no XML anywhere.
// ---------------------------------------------------------------------

fn named_role_json(role: &NamedRole) -> Value {
    json!({
        "name": role.name,
        "role": role.role,
    })
}

fn repository_json(repository: &Repository) -> Value {
    json!({
        "name": repository.name,
        "arch": repository.arch,
        "paths": repository
            .paths
            .iter()
            .map(|(project, repository)| json!({
                "project": project,
                "repository": repository,
            }))
            .collect::<Vec<_>>(),
    })
}

fn source_file_json(file: &SourceFile) -> Value {
    json!({
        "name": file.name,
        "size": file.size,
        "mtime": file.mtime,
        "md5": file.md5,
    })
}

fn history_entry_json(entry: &HistoryEntry) -> Value {
    json!({
        "rev": entry.rev,
        "vrev": entry.vrev,
        "srcmd5": entry.srcmd5,
        "version": entry.version,
        "time": entry.time,
        "user": entry.user,
        "comment": entry.comment,
        "request_id": entry.request_id,
    })
}

fn file_patch_json(patch: &FilePatch) -> Value {
    json!({
        "filename": patch.filename,
        "status": file_patch_status(patch.status),
        "diff": patch.diff,
    })
}

fn package_diff_json(diff: &PackageDiff) -> Value {
    json!({
        "name": diff.name,
        "status": package_diff_status(diff.status),
        "files": diff.files.iter().map(file_patch_json).collect::<Vec<_>>(),
        "requests": diff.requests,
    })
}

fn owner_json(owner: &Owner) -> Value {
    json!({
        "root_project": owner.root_project,
        "project": owner.project,
        "package": owner.package,
        "persons": owner.persons.iter().map(named_role_json).collect::<Vec<_>>(),
        "groups": owner.groups.iter().map(named_role_json).collect::<Vec<_>>(),
    })
}

fn package_match_json(match_: &PackageMatch) -> Value {
    json!({
        "name": match_.name,
        "project": match_.project,
        "title": match_.title,
        "description": match_.description,
    })
}

fn project_match_json(match_: &ProjectMatch) -> Value {
    json!({
        "name": match_.name,
        "title": match_.title,
        "description": match_.description,
    })
}

fn project_meta_json(meta: &ProjectMeta) -> Value {
    json!({
        "name": meta.name,
        "title": meta.title,
        "description": meta.description,
        "url": meta.url,
        "scmsync": meta.scmsync,
        "devel": meta.devel,
        "persons": meta.persons.iter().map(named_role_json).collect::<Vec<_>>(),
        "groups": meta.groups.iter().map(named_role_json).collect::<Vec<_>>(),
        "repositories": meta
            .repositories
            .iter()
            .map(repository_json)
            .collect::<Vec<_>>(),
    })
}

fn package_meta_json(meta: &PackageMeta) -> Value {
    json!({
        "name": meta.name,
        "project": meta.project,
        "title": meta.title,
        "description": meta.description,
        "url": meta.url,
        "scmsync": meta.scmsync,
        "releasename": meta.releasename,
        "devel": meta.devel.as_ref().map(|(project, package)| json!({
            "project": project,
            "package": package,
        })),
        "persons": meta.persons.iter().map(named_role_json).collect::<Vec<_>>(),
        "groups": meta.groups.iter().map(named_role_json).collect::<Vec<_>>(),
    })
}

fn person_json(person: &Person) -> Value {
    json!({
        "login": person.login,
        "email": person.email,
        "realname": person.realname,
        "state": person.state,
    })
}

fn file_patch_status(status: FilePatchStatus) -> &'static str {
    match status {
        FilePatchStatus::Added => "added",
        FilePatchStatus::Modified => "modified",
        FilePatchStatus::Deleted => "deleted",
    }
}

fn package_diff_status(status: PackageDiffStatus) -> &'static str {
    match status {
        PackageDiffStatus::Identical => "identical",
        PackageDiffStatus::Differs => "differs",
        PackageDiffStatus::OnlyInOld => "only-in-old",
        PackageDiffStatus::OnlyInNew => "only-in-new",
    }
}

// ---------------------------------------------------------------------
// Text renderers. Tables have a header row and space-aligned columns,
// no borders, no truncation. Long free text (descriptions, comments)
// and verification values (md5, srcmd5) live in the JSON payload only.
// ---------------------------------------------------------------------

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

fn render_source_files(files: &[SourceFile]) -> String {
    if files.is_empty() {
        return "no files".to_string();
    }
    // md5 stays JSON-only: it is a verification value, not a
    // human-readable identifier.
    let rows: Vec<Vec<String>> = files
        .iter()
        .map(|file| {
            vec![
                file.name.clone(),
                file.size
                    .map(|size| size.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                file.mtime
                    .map(|mtime| mtime.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    render_table(&["NAME", "SIZE", "MTIME"], &rows)
}

fn render_history(entries: &[HistoryEntry]) -> String {
    if entries.is_empty() {
        return "no revisions".to_string();
    }
    // The comment is long free text and the srcmd5 a verification
    // value: both stay JSON-only.
    let rows: Vec<Vec<String>> = entries
        .iter()
        .map(|entry| {
            vec![
                entry.rev.to_string(),
                entry.vrev.to_string(),
                entry.version.clone().unwrap_or_else(|| "-".to_string()),
                entry
                    .time
                    .map(|time| time.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                entry.user.clone().unwrap_or_else(|| "-".to_string()),
                entry.request_id.clone().unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    render_table(
        &["REV", "VREV", "VERSION", "TIME", "USER", "REQUEST"],
        &rows,
    )
}

fn render_revision_patch(patch: &RevisionPatch) -> String {
    if patch.files.is_empty() {
        if patch.rev == 1 {
            // Revision 1 created the package: there is no previous
            // revision to diff against, not "no changes".
            return "revision 1 has no previous revision to diff against".to_string();
        }
        return format!("no changes in revision {}", patch.rev);
    }
    render_file_patch_sections(&patch.files)
}

/// The per-file diff sections of a two-package diff, verbatim: one
/// `file: <name> (<status>)` header per file, then the unified diff
/// itself. A content rendering — the diffs are the server text, not
/// re-rendered.
fn render_diff_packages(files: &[FilePatch]) -> String {
    if files.is_empty() {
        return "no changes".to_string();
    }
    render_file_patch_sections(files)
}

/// One `file: <name> (<status>)` section per file patch, the diff
/// text verbatim underneath.
fn render_file_patch_sections(files: &[FilePatch]) -> String {
    let mut sections = Vec::new();
    for file in files {
        sections.push(format!(
            "file: {} ({})\n{}",
            file.filename,
            file_patch_status(file.status),
            file.diff
        ));
    }
    sections.join("\n")
}

fn render_named_roles(label: &str, roles: &[NamedRole]) -> Vec<String> {
    roles
        .iter()
        .map(|role| format!("{label} {} ({})", role.name, role.role))
        .collect()
}

fn render_project_meta(meta: &ProjectMeta) -> String {
    let mut lines = vec![format!("name: {}", meta.name)];
    if let Some(title) = &meta.title {
        lines.push(format!("title: {title}"));
    }
    if let Some(description) = &meta.description {
        lines.push(format!("description: {description}"));
    }
    if let Some(url) = &meta.url {
        lines.push(format!("url: {url}"));
    }
    if let Some(scmsync) = &meta.scmsync {
        lines.push(format!("scmsync: {scmsync}"));
    }
    if let Some(devel) = &meta.devel {
        lines.push(format!("devel: {devel}"));
    }
    lines.extend(render_named_roles("person", &meta.persons));
    lines.extend(render_named_roles("group", &meta.groups));
    for repository in &meta.repositories {
        let arch = if repository.arch.is_empty() {
            "-".to_string()
        } else {
            repository.arch.join(", ")
        };
        let paths = if repository.paths.is_empty() {
            "-".to_string()
        } else {
            repository
                .paths
                .iter()
                .map(|(project, repository)| format!("{project}/{repository}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        lines.push(format!(
            "repository {}: arch {}; paths {}",
            repository.name, arch, paths
        ));
    }
    lines.join("\n")
}

fn render_package_meta(meta: &PackageMeta) -> String {
    let mut lines = vec![
        format!("name: {}", meta.name),
        format!("project: {}", meta.project),
    ];
    if let Some(title) = &meta.title {
        lines.push(format!("title: {title}"));
    }
    if let Some(description) = &meta.description {
        lines.push(format!("description: {description}"));
    }
    if let Some(url) = &meta.url {
        lines.push(format!("url: {url}"));
    }
    if let Some(scmsync) = &meta.scmsync {
        lines.push(format!("scmsync: {scmsync}"));
    }
    if let Some(releasename) = &meta.releasename {
        lines.push(format!("releasename: {releasename}"));
    }
    if let Some((project, package)) = &meta.devel {
        lines.push(format!("devel: {project}/{package}"));
    }
    lines.extend(render_named_roles("person", &meta.persons));
    lines.extend(render_named_roles("group", &meta.groups));
    lines.join("\n")
}

fn render_package_matches(term: &str, matches: &[PackageMatch]) -> String {
    if matches.is_empty() {
        return format!("no packages matching '{term}'");
    }
    // Descriptions are long free text and stay JSON-only; titles are
    // short identifiers and stay in the table.
    let rows: Vec<Vec<String>> = matches
        .iter()
        .map(|match_| {
            vec![
                match_.name.clone(),
                match_.project.clone(),
                match_.title.clone().unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    render_table(&["NAME", "PROJECT", "TITLE"], &rows)
}

fn render_project_matches(term: &str, matches: &[ProjectMatch]) -> String {
    if matches.is_empty() {
        return format!("no projects matching '{term}'");
    }
    let rows: Vec<Vec<String>> = matches
        .iter()
        .map(|match_| {
            vec![
                match_.name.clone(),
                match_.title.clone().unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    render_table(&["NAME", "TITLE"], &rows)
}

fn render_owners(owners: &[Owner]) -> String {
    if owners.is_empty() {
        return "no owners".to_string();
    }
    let lines: Vec<String> = owners
        .iter()
        .map(|owner| {
            let persons = if owner.persons.is_empty() {
                "-".to_string()
            } else {
                owner
                    .persons
                    .iter()
                    .map(|role| format!("{} ({})", role.name, role.role))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let groups = if owner.groups.is_empty() {
                "-".to_string()
            } else {
                owner
                    .groups
                    .iter()
                    .map(|role| format!("{} ({})", role.name, role.role))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            format!(
                "{} {} {}: persons: {}; groups: {}",
                owner.root_project,
                owner.project,
                owner.package.as_deref().unwrap_or("-"),
                persons,
                groups
            )
        })
        .collect();
    lines.join("\n")
}

fn render_person(person: &Person) -> String {
    let mut lines = vec![
        format!("login: {}", person.login),
        format!("email: {}", person.email),
    ];
    if let Some(realname) = &person.realname {
        lines.push(format!("realname: {realname}"));
    }
    lines.push(format!("state: {}", person.state));
    lines.join("\n")
}

fn render_project_diff(packages: &[PackageDiff]) -> String {
    if packages.is_empty() {
        return "no packages".to_string();
    }
    let rows: Vec<Vec<String>> = packages
        .iter()
        .map(|package| {
            vec![
                package.name.clone(),
                package_diff_status(package.status).to_string(),
            ]
        })
        .collect();
    let mut text = render_table(&["NAME", "STATUS"], &rows);
    for package in packages {
        if package.status != PackageDiffStatus::Differs {
            continue;
        }
        text.push_str(&format!("\npackage {}:", package.name));
        for file in &package.files {
            text.push_str(&format!(
                "\nfile: {} ({})\n{}",
                file.filename,
                file_patch_status(file.status),
                file.diff
            ));
        }
        if !package.requests.is_empty() {
            text.push_str(&format!("\nrequests: {}", package.requests.join(", ")));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cli, Command};
    use clap::Parser;

    fn parse(args: &[&str]) -> RemoteCommand {
        match Cli::parse_from(args).command {
            Some(Command::Remote { command }) => command,
            _ => panic!("expected a remote command, got {:?}", args),
        }
    }

    #[test]
    fn cat_parses_positionals_and_optional_rev() {
        let parsed = parse(&[
            "geectl",
            "remote",
            "cat",
            "openSUSE:Factory",
            "vim",
            "vim.spec",
        ]);
        assert!(matches!(
            parsed,
            RemoteCommand::Cat {
                project,
                package,
                file,
                rev: None,
            } if project == "openSUSE:Factory" && package == "vim" && file == "vim.spec"
        ));
        let parsed = parse(&[
            "geectl",
            "remote",
            "cat",
            "openSUSE:Factory",
            "vim",
            "vim.spec",
            "--rev",
            "12",
        ]);
        assert!(matches!(parsed, RemoteCommand::Cat { rev: Some(12), .. }));
    }

    #[test]
    fn list_commands_parse_positionals() {
        assert!(matches!(
            parse(&["geectl", "remote", "list-files", "P", "K"]),
            RemoteCommand::ListFiles { .. }
        ));
        assert!(matches!(
            parse(&["geectl", "remote", "list-packages", "P"]),
            RemoteCommand::ListPackages { .. }
        ));
        assert!(matches!(
            parse(&["geectl", "remote", "package-log", "P", "K"]),
            RemoteCommand::PackageLog { range: None, .. }
        ));
        let parsed = parse(&[
            "geectl",
            "remote",
            "package-log",
            "P",
            "K",
            "--range",
            "1:3",
        ]);
        assert!(matches!(
            parsed,
            RemoteCommand::PackageLog {
                range: Some(range),
                ..
            } if range == "1:3"
        ));
        assert!(matches!(
            parse(&["geectl", "remote", "revision-patch", "P", "K", "7"]),
            RemoteCommand::RevisionPatch { rev: 7, .. }
        ));
        assert!(matches!(
            parse(&["geectl", "remote", "project-meta", "P"]),
            RemoteCommand::ProjectMeta { .. }
        ));
        assert!(matches!(
            parse(&["geectl", "remote", "package-meta", "P", "K"]),
            RemoteCommand::PackageMeta { .. }
        ));
        assert!(matches!(
            parse(&["geectl", "remote", "search-packages", "vim"]),
            RemoteCommand::SearchPackages { exact: false, .. }
        ));
        assert!(matches!(
            parse(&["geectl", "remote", "search-packages", "vim", "--exact"]),
            RemoteCommand::SearchPackages { exact: true, .. }
        ));
        assert!(matches!(
            parse(&["geectl", "remote", "search-projects", "openSUSE"]),
            RemoteCommand::SearchProjects { exact: false, .. }
        ));
        assert!(matches!(
            parse(&["geectl", "remote", "search-projects", "openSUSE", "--exact"]),
            RemoteCommand::SearchProjects { exact: true, .. }
        ));
        assert!(matches!(
            parse(&["geectl", "remote", "owners", "P", "K"]),
            RemoteCommand::Owners { .. }
        ));
        assert!(matches!(
            parse(&["geectl", "remote", "owners-for-binary", "vim"]),
            RemoteCommand::OwnersForBinary { .. }
        ));
        assert!(matches!(
            parse(&["geectl", "remote", "maintainers", "P", "K"]),
            RemoteCommand::Maintainers { .. }
        ));
        assert!(matches!(
            parse(&["geectl", "remote", "whois", "alice"]),
            RemoteCommand::Whois { .. }
        ));
        assert!(matches!(
            parse(&["geectl", "remote", "link-target", "P", "K"]),
            RemoteCommand::LinkTarget { .. }
        ));
        assert!(matches!(
            parse(&["geectl", "remote", "devel-project", "vim"]),
            RemoteCommand::DevelProject { .. }
        ));
    }

    #[test]
    fn project_diff_parses_repeatable_filters_and_flag() {
        let parsed = parse(&[
            "geectl",
            "remote",
            "project-diff",
            "OLD",
            "NEW",
            "--include",
            "vim",
            "--include",
            "curl",
            "--exclude",
            "kernel",
            "--annotate-requests",
        ]);
        assert!(matches!(
            parsed,
            RemoteCommand::ProjectDiff {
                include,
                exclude,
                annotate_requests: true,
                ..
            } if include == vec!["vim", "curl"] && exclude == vec!["kernel"]
        ));
    }

    #[test]
    fn new_remote_commands_parse() {
        assert!(matches!(
            parse(&["geectl", "remote", "list-projects"]),
            RemoteCommand::ListProjects
        ));
        assert!(matches!(
            parse(&[
                "geectl",
                "remote",
                "diff-packages",
                "OLD",
                "vim",
                "NEW",
                "vim"
            ]),
            RemoteCommand::DiffPackages { .. }
        ));
        let parsed = parse(&[
            "geectl",
            "remote",
            "repo-add",
            "P",
            "standard",
            "--arch",
            "x86_64",
            "--arch",
            "aarch64",
            "--path",
            "openSUSE:Factory/standard",
            "--disable-publish",
        ]);
        assert!(matches!(
            parsed,
            RemoteCommand::RepoAdd {
                arch,
                path,
                disable_publish: true,
                ..
            } if arch == vec!["x86_64", "aarch64"]
                && path == vec!["openSUSE:Factory/standard"]
        ));
        assert!(matches!(
            parse(&["geectl", "remote", "repo-remove", "P", "standard"]),
            RemoteCommand::RepoRemove { .. }
        ));
    }

    #[test]
    fn repo_add_requires_arch_and_path() {
        // clap enforces at least one `--arch` and one `--path`:
        // the library refuses an empty arch or path list, so the
        // flags must fail at parse time, not at the call.
        assert!(Cli::try_parse_from([
            "geectl",
            "remote",
            "repo-add",
            "P",
            "standard",
            "--path",
            "openSUSE:Factory/standard",
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "geectl", "remote", "repo-add", "P", "standard", "--arch", "x86_64",
        ])
        .is_err());
    }

    #[test]
    fn repo_paths_split_on_the_first_slash() {
        assert_eq!(
            parse_repo_paths(&["openSUSE:Factory/standard".to_string()]).unwrap(),
            vec![("openSUSE:Factory".to_string(), "standard".to_string())]
        );
        assert!(parse_repo_paths(&["no-slash".to_string()]).is_err());
        assert!(parse_repo_paths(&["/standard".to_string()]).is_err());
        assert!(parse_repo_paths(&["openSUSE:Factory/".to_string()]).is_err());
    }

    #[test]
    fn list_projects_renders_a_name_table() {
        let text =
            render_project_names(&["openSUSE:Factory".to_string(), "home:alice".to_string()]);
        assert!(text.contains("openSUSE:Factory"));
        assert!(text.contains("home:alice"));
        assert_eq!(render_project_names(&[]), "no projects");
    }

    #[test]
    fn diff_packages_renders_per_file_sections_verbatim() {
        let files = vec![FilePatch {
            filename: "vim.spec".to_string(),
            status: FilePatchStatus::Modified,
            diff: "--- a/vim.spec\n+++ b/vim.spec\n@@ -1 +1 @@\n-old\n+new\n".to_string(),
        }];
        let text = render_diff_packages(&files);
        assert!(text.contains("file: vim.spec (modified)"));
        assert!(text.contains("--- a/vim.spec"));
        assert_eq!(render_diff_packages(&[]), "no changes");
    }

    #[test]
    fn mutation_result_carries_dry_run_and_plan_in_order() {
        let payload = json!({ "project": "P", "repository": "standard" });
        let (payload, text) = mutation_result(
            payload,
            "project: P repository: standard".to_string(),
            "dry-run: would remove repository standard from project P".to_string(),
            true,
            Some(vec!["remove repository standard from project P".to_string()]),
        );
        assert_eq!(payload["dry_run"], true);
        assert_eq!(
            payload["plan"][0],
            "remove repository standard from project P"
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "project: P repository: standard");
        assert_eq!(lines[1], "dry_run: true");
        assert!(lines[2].starts_with("dry-run: would remove"));
        assert_eq!(lines[3], "remove repository standard from project P");
    }

    #[test]
    fn unknown_remote_command_fails_to_parse() {
        assert!(Cli::try_parse_from(["geectl", "remote", "frobnicate"]).is_err());
        assert!(Cli::try_parse_from(["geectl", "remote", "cat", "only-one"]).is_err());
    }

    #[test]
    fn table_aligns_columns_and_trims_trailing_space() {
        let text = render_table(
            &["NAME", "SIZE"],
            &[
                vec!["a".to_string(), "1".to_string()],
                vec!["long-name".to_string(), "22".to_string()],
            ],
        );
        assert_eq!(text, "NAME      SIZE\na         1\nlong-name 22");
    }

    #[test]
    fn list_files_payload_carries_md5_but_text_does_not() {
        let files = vec![
            SourceFile {
                name: "vim.spec".to_string(),
                size: Some(1234),
                mtime: Some(1_700_000_000),
                md5: Some("deadbeef".to_string()),
            },
            SourceFile {
                name: "README".to_string(),
                size: None,
                mtime: None,
                md5: None,
            },
        ];
        let payload = json!({
            "files": files.iter().map(source_file_json).collect::<Vec<_>>(),
        });
        assert_eq!(payload["files"][0]["md5"], "deadbeef");
        assert_eq!(payload["files"][1]["md5"], Value::Null);
        let text = render_source_files(&files);
        assert!(text.contains("vim.spec"));
        assert!(text.contains("1234"));
        assert!(text.contains("1700000000"));
        assert!(text.contains("README"));
        assert!(!text.contains("deadbeef"));
        assert_eq!(render_source_files(&[]), "no files");
    }

    #[test]
    fn package_log_payload_carries_comment_and_srcmd5_but_text_does_not() {
        let entries = vec![HistoryEntry {
            rev: 3,
            vrev: 3,
            srcmd5: Some("abcd1234".to_string()),
            version: Some("9.1".to_string()),
            time: Some(1_700_000_000),
            user: Some("alice".to_string()),
            comment: Some("Fix a long-standing bug in the parser".to_string()),
            request_id: Some("123456".to_string()),
        }];
        let payload = json!({
            "entries": entries.iter().map(history_entry_json).collect::<Vec<_>>(),
        });
        assert_eq!(
            payload["entries"][0]["comment"],
            "Fix a long-standing bug in the parser"
        );
        assert_eq!(payload["entries"][0]["srcmd5"], "abcd1234");
        let text = render_history(&entries);
        assert!(text.contains("3"));
        assert!(text.contains("9.1"));
        assert!(text.contains("alice"));
        assert!(text.contains("123456"));
        assert!(!text.contains("abcd1234"));
        assert!(!text.contains("Fix a long-standing bug"));
        assert_eq!(render_history(&[]), "no revisions");
    }

    #[test]
    fn revision_patch_renders_per_file_sections_verbatim() {
        let patch = RevisionPatch {
            rev: 5,
            files: vec![
                FilePatch {
                    filename: "a.spec".to_string(),
                    status: FilePatchStatus::Modified,
                    diff: "--- a/a.spec\n+++ b/a.spec\n@@ -1 +1 @@\n-old\n+new\n".to_string(),
                },
                FilePatch {
                    filename: "b.txt".to_string(),
                    status: FilePatchStatus::Added,
                    diff: "--- /dev/null\n+++ b/b.txt\n@@ -0,0 +1 @@\n+hello\n".to_string(),
                },
            ],
        };
        let text = render_revision_patch(&patch);
        assert!(text.contains("file: a.spec (modified)"));
        assert!(text.contains("file: b.txt (added)"));
        assert!(text.contains("--- a/a.spec"));
        assert!(text.contains("+hello"));
        let payload = json!({
            "files": patch.files.iter().map(file_patch_json).collect::<Vec<_>>(),
        });
        assert_eq!(payload["files"][0]["status"], "modified");
        assert_eq!(payload["files"][1]["status"], "added");
        assert_eq!(
            render_revision_patch(&RevisionPatch {
                rev: 5,
                files: vec![]
            }),
            "no changes in revision 5"
        );
    }

    fn sample_project_meta() -> ProjectMeta {
        ProjectMeta {
            name: "openSUSE:Factory".to_string(),
            title: Some("Factory".to_string()),
            description: Some("The rolling distribution".to_string()),
            url: Some("https://www.opensuse.org".to_string()),
            scmsync: None,
            devel: None,
            persons: vec![NamedRole {
                name: "alice".to_string(),
                role: "maintainer".to_string(),
            }],
            groups: vec![],
            repositories: vec![Repository {
                name: "standard".to_string(),
                arch: vec!["x86_64".to_string(), "aarch64".to_string()],
                paths: vec![("openSUSE:Factory".to_string(), "standard".to_string())],
            }],
        }
    }

    #[test]
    fn project_meta_renders_every_field() {
        let meta = sample_project_meta();
        let payload = project_meta_json(&meta);
        assert_eq!(payload["name"], "openSUSE:Factory");
        assert_eq!(payload["repositories"][0]["arch"][1], "aarch64");
        let text = render_project_meta(&meta);
        assert!(text.contains("name: openSUSE:Factory"));
        assert!(text.contains("title: Factory"));
        assert!(text.contains("person alice (maintainer)"));
        assert!(text.contains("repository standard: arch x86_64, aarch64"));
        assert!(text.contains("paths openSUSE:Factory/standard"));
    }

    #[test]
    fn package_meta_renders_devel_pair() {
        let meta = PackageMeta {
            name: "vim".to_string(),
            project: "openSUSE:Factory".to_string(),
            title: None,
            description: None,
            url: None,
            scmsync: None,
            releasename: Some("vim".to_string()),
            devel: Some(("editors".to_string(), "vim".to_string())),
            persons: vec![],
            groups: vec![],
        };
        let payload = package_meta_json(&meta);
        assert_eq!(payload["devel"]["project"], "editors");
        assert_eq!(payload["devel"]["package"], "vim");
        let text = render_package_meta(&meta);
        assert!(text.contains("name: vim"));
        assert!(text.contains("project: openSUSE:Factory"));
        assert!(text.contains("releasename: vim"));
        assert!(text.contains("devel: editors/vim"));
    }

    #[test]
    fn search_renders_titles_but_not_descriptions() {
        let packages = vec![PackageMatch {
            name: "vim".to_string(),
            project: "openSUSE:Factory".to_string(),
            title: Some("Vi IMproved".to_string()),
            description: Some("A very long description of the editor".to_string()),
        }];
        let payload = json!({
            "packages": packages.iter().map(package_match_json).collect::<Vec<_>>(),
        });
        assert_eq!(
            payload["packages"][0]["description"],
            "A very long description of the editor"
        );
        let text = render_package_matches("vim", &packages);
        assert!(text.contains("vim"));
        assert!(text.contains("openSUSE:Factory"));
        assert!(text.contains("Vi IMproved"));
        assert!(!text.contains("A very long description"));
        assert_eq!(
            render_package_matches("nothing", &[]),
            "no packages matching 'nothing'"
        );
        assert_eq!(
            render_project_matches("nothing", &[]),
            "no projects matching 'nothing'"
        );
    }

    #[test]
    fn owners_render_persons_and_groups() {
        let owners = vec![Owner {
            root_project: "openSUSE:Factory".to_string(),
            project: "openSUSE:Factory".to_string(),
            package: Some("vim".to_string()),
            persons: vec![NamedRole {
                name: "alice".to_string(),
                role: "maintainer".to_string(),
            }],
            groups: vec![NamedRole {
                name: "editors".to_string(),
                role: "bugowner".to_string(),
            }],
        }];
        let payload = json!({
            "owners": owners.iter().map(owner_json).collect::<Vec<_>>(),
        });
        assert_eq!(payload["owners"][0]["package"], "vim");
        let text = render_owners(&owners);
        assert!(text.contains("openSUSE:Factory openSUSE:Factory vim:"));
        assert!(text.contains("persons: alice (maintainer)"));
        assert!(text.contains("groups: editors (bugowner)"));
        assert_eq!(render_owners(&[]), "no owners");
    }

    #[test]
    fn whois_renders_realname_when_present() {
        let person = Person {
            login: "alice".to_string(),
            email: "alice@example.com".to_string(),
            realname: Some("Alice Example".to_string()),
            state: "confirmed".to_string(),
        };
        let text = render_person(&person);
        assert!(text.contains("login: alice"));
        assert!(text.contains("email: alice@example.com"));
        assert!(text.contains("realname: Alice Example"));
        assert!(text.contains("state: confirmed"));
        let payload = person_json(&person);
        assert_eq!(payload["login"], "alice");
        assert_eq!(payload["realname"], "Alice Example");
    }

    #[test]
    fn project_diff_renders_table_then_per_package_diffs() {
        let packages = vec![
            PackageDiff {
                name: "same".to_string(),
                status: PackageDiffStatus::Identical,
                files: vec![],
                requests: vec![],
            },
            PackageDiff {
                name: "vim".to_string(),
                status: PackageDiffStatus::Differs,
                files: vec![FilePatch {
                    filename: "vim.spec".to_string(),
                    status: FilePatchStatus::Modified,
                    diff: "--- a/vim.spec\n+++ b/vim.spec\n@@ -1 +1 @@\n-old\n+new\n".to_string(),
                }],
                requests: vec!["123".to_string()],
            },
            PackageDiff {
                name: "gone".to_string(),
                status: PackageDiffStatus::OnlyInOld,
                files: vec![],
                requests: vec![],
            },
        ];
        let text = render_project_diff(&packages);
        assert!(text.contains("same"));
        assert!(text.contains("identical"));
        assert!(text.contains("vim"));
        assert!(text.contains("differs"));
        assert!(text.contains("gone"));
        assert!(text.contains("only-in-old"));
        assert!(text.contains("package vim:"));
        assert!(text.contains("file: vim.spec (modified)"));
        assert!(text.contains("--- a/vim.spec"));
        assert!(text.contains("requests: 123"));
        let payload = json!({
            "packages": packages.iter().map(package_diff_json).collect::<Vec<_>>(),
        });
        assert_eq!(payload["packages"][1]["status"], "differs");
        assert_eq!(payload["packages"][1]["requests"][0], "123");
        assert_eq!(render_project_diff(&[]), "no packages");
    }

    #[test]
    fn link_target_result_carries_target_in_both_formats() {
        let (payload, text) = link_target_result(
            "openSUSE:Factory",
            "vim",
            &Some(("editors".to_string(), "vim".to_string())),
        );
        assert_eq!(payload["target"]["project"], "editors");
        assert_eq!(payload["target"]["package"], "vim");
        assert_eq!(
            text,
            "project: openSUSE:Factory package: vim\nlink: editors/vim"
        );
        let (payload, text) = link_target_result("openSUSE:Factory", "vim", &None);
        assert_eq!(payload["target"], Value::Null);
        assert_eq!(text, "project: openSUSE:Factory package: vim\nno link");
    }

    #[test]
    fn cat_result_is_verbatim_with_rev_null_or_some() {
        let (payload, text) = cat_result(
            "openSUSE:Factory",
            "vim",
            "vim.spec",
            None,
            "Name: vim\n".to_string(),
        );
        assert_eq!(payload["rev"], Value::Null);
        assert_eq!(payload["content"], "Name: vim\n");
        assert_eq!(text, "Name: vim\n");
        let (payload, text) = cat_result(
            "openSUSE:Factory",
            "vim",
            "vim.spec",
            Some(12),
            "Name: vim\n".to_string(),
        );
        assert_eq!(payload["rev"], 12);
        assert_eq!(text, "Name: vim\n");
    }

    #[test]
    fn list_packages_result_renders_table_or_empty_line() {
        let (payload, text) = list_packages_result(
            "openSUSE:Factory",
            vec!["curl".to_string(), "vim".to_string()],
        );
        assert_eq!(payload["project"], "openSUSE:Factory");
        assert_eq!(payload["packages"][0], "curl");
        assert_eq!(payload["packages"][1], "vim");
        assert_eq!(text, "project: openSUSE:Factory\nNAME\ncurl\nvim");
        let (payload, text) = list_packages_result("openSUSE:Factory", vec![]);
        assert_eq!(payload["packages"], json!([]));
        assert_eq!(text, "project: openSUSE:Factory\nno packages");
    }

    #[test]
    fn maintainers_result_renders_table_or_empty_line() {
        let (payload, text) = maintainers_result(
            "openSUSE:Factory",
            "vim",
            vec![NamedRole {
                name: "alice".to_string(),
                role: "maintainer".to_string(),
            }],
        );
        assert_eq!(payload["maintainers"][0]["name"], "alice");
        assert_eq!(payload["maintainers"][0]["role"], "maintainer");
        assert!(text.contains("NAME"));
        assert!(text.contains("alice"));
        assert!(text.contains("maintainer"));
        let (payload, text) = maintainers_result("openSUSE:Factory", "vim", vec![]);
        assert_eq!(payload["maintainers"], json!([]));
        assert_eq!(
            text,
            "project: openSUSE:Factory package: vim\nno maintainers"
        );
    }

    #[test]
    fn devel_project_result_carries_pair_or_none_line() {
        let (payload, text) =
            devel_project_result("vim", Some(("editors".to_string(), "vim".to_string())));
        assert_eq!(payload["package"], "vim");
        assert_eq!(payload["devel_project"], "editors");
        assert_eq!(payload["devel_package"], "vim");
        assert_eq!(text, "package: vim\ndevel project: editors package: vim");
        let (payload, text) = devel_project_result("vim", None);
        assert_eq!(payload["devel_project"], Value::Null);
        assert_eq!(payload["devel_package"], Value::Null);
        assert_eq!(text, "package: vim\nno devel project");
    }

    #[test]
    fn apply_exact_keeps_only_exact_names() {
        let packages = vec![
            PackageMatch {
                name: "vim".to_string(),
                project: "openSUSE:Factory".to_string(),
                title: None,
                description: None,
            },
            PackageMatch {
                name: "vim-data".to_string(),
                project: "openSUSE:Factory".to_string(),
                title: None,
                description: None,
            },
        ];
        let kept = apply_exact(packages.clone(), "vim", false, |m| m.name.as_str());
        assert_eq!(kept.len(), 2);
        let kept = apply_exact(packages, "vim", true, |m| m.name.as_str());
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].name, "vim");
        let projects = vec![ProjectMatch {
            name: "openSUSE:Factory".to_string(),
            title: None,
            description: None,
        }];
        let kept = apply_exact(projects, "openSUSE", true, |m| m.name.as_str());
        assert!(kept.is_empty());
    }

    #[test]
    fn search_projects_renders_non_empty_table() {
        let matches = vec![ProjectMatch {
            name: "openSUSE:Factory".to_string(),
            title: Some("Factory".to_string()),
            description: Some("A very long description".to_string()),
        }];
        let text = render_project_matches("openSUSE", &matches);
        assert!(text.contains("openSUSE:Factory"));
        assert!(text.contains("Factory"));
        assert!(!text.contains("A very long description"));
    }

    #[test]
    fn whois_renders_without_realname() {
        let person = Person {
            login: "alice".to_string(),
            email: "alice@example.com".to_string(),
            realname: None,
            state: "confirmed".to_string(),
        };
        let text = render_person(&person);
        assert!(text.contains("login: alice"));
        assert!(text.contains("email: alice@example.com"));
        assert!(!text.contains("realname:"));
        assert!(text.contains("state: confirmed"));
        assert_eq!(person_json(&person)["realname"], Value::Null);
    }

    #[test]
    fn owners_render_package_none_as_dash() {
        let owners = vec![Owner {
            root_project: "openSUSE:Factory".to_string(),
            project: "openSUSE:Factory".to_string(),
            package: None,
            persons: vec![],
            groups: vec![],
        }];
        let payload = json!({
            "owners": owners.iter().map(owner_json).collect::<Vec<_>>(),
        });
        assert_eq!(payload["owners"][0]["package"], Value::Null);
        let text = render_owners(&owners);
        assert!(text.contains("openSUSE:Factory openSUSE:Factory -:"));
        assert!(text.contains("persons: -"));
        assert!(text.contains("groups: -"));
    }

    #[test]
    fn revision_patch_rev_one_has_no_previous_revision() {
        assert_eq!(
            render_revision_patch(&RevisionPatch {
                rev: 1,
                files: vec![]
            }),
            "revision 1 has no previous revision to diff against"
        );
        assert_eq!(
            render_revision_patch(&RevisionPatch {
                rev: 5,
                files: vec![]
            }),
            "no changes in revision 5"
        );
    }
}
