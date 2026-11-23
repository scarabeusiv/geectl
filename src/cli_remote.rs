//! Remote CLI commands (`geectl remote ...`).
//!
//! The inspection surface for what the distribution holds: file
//! contents and downloads, listings, history, meta, search, ownership,
//! and diffs — plus the group's mutations, which edit a project's repository
//! list in its meta (`repo-add`, `repo-remove`) or create, upload
//! to, delete, restore, aggregate, and release projects and
//! packages. Each command is a thin wrapper over one `ObsClient`
//! operation: the flags mirror the library parameters, the JSON
//! payload carries the typed values, and the text renderer prints
//! the same values in a human-readable form. No XML reaches the user
//! anywhere.
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
//! `list-staging-projects` echoes the workflow project (`project:`);
//! `staging-project` echoes the workflow project and the staging
//! name (`project:` + `staging:`). `search-projects-by-person`
//! echoes the login (`login:`); `count-scmsync-projects` echoes the
//! URL prefix (`url-prefix:`).
//!
//! The group's mutations follow the
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
//! itself, not re-rendered. `create-project` and `create-package`
//! text carries no description (long free text); `delete-project`
//! text carries no comment (a message, not an identifier);
//! `create-package` text carries no `force` (a boolean; the dry-run
//! plan names it when set).
//! The JSON payload remains the complete record in every case.

use clap::Subcommand;
use geectl::auth::{Credentials, EnvObsAuth};
use geectl::config;
use geectl::debug::Debug;
use geectl::error::{ErrorKind, GeectlError};
use geectl::http::{Timeout, UreqClient};
use geectl::obs::{
    AggregateOptions, CreatePackageOptions, DeleteProjectOptions, DryRun, FilePatch,
    FilePatchStatus, HistoryEntry, NamedRole, ObsClient, Owner, PackageDiff, PackageDiffStatus,
    PackageMatch, PackageMeta, Person, ProjectMatch, ProjectMeta, ReleaseOptions, Repository,
    RevisionPatch, RevisionRange, RevisionSpec, SourceFile, StagedRequest, StagingProject,
    StagingReadOptions, FACTORY_PROJECT, OBS_API_BASE,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The `remote` command group: inspection of the OBS source tree,
/// and the mutations that change it.
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
        /// Print the file as of this revision (default: latest):
        /// a revision number or a srcmd5.
        #[arg(long)]
        rev: Option<String>,
    },
    /// Download a source file to a local path. The binary counterpart
    /// of `cat`: the library streams the file to disk instead of
    /// reading it as a UTF-8 string, so a binary file (a source
    /// tarball) downloads instead of failing the UTF-8 check, and a
    /// file larger than the transport's 10 MiB body cap downloads
    /// instead of failing. A read: `--live` is ignored, like `cat`.
    Download {
        /// The project the package lives in.
        project: String,
        /// The package the file lives in.
        package: String,
        /// The file to download.
        file: String,
        /// Download the file as of this revision: a revision number
        /// or a srcmd5 (default: latest).
        #[arg(long)]
        rev: Option<String>,
        /// Where to write the downloaded bytes.
        #[arg(long)]
        output: PathBuf,
    },
    /// List the files of a package.
    ListFiles {
        /// The project the package lives in.
        project: String,
        /// The package to list.
        package: String,
        /// List the files as of this revision (default: latest):
        /// a revision number or a srcmd5.
        #[arg(long)]
        rev: Option<String>,
    },
    /// List the packages of a project.
    ListPackages {
        /// The project to list.
        project: String,
    },
    /// List the top-level projects on the instance.
    ListProjects,
    /// List the staging projects of a workflow project.
    ListStagingProjects {
        /// The workflow project the staging projects belong to.
        project: String,
        /// Leave out the staged, untracked, and obsolete request
        /// lists.
        #[arg(long)]
        no_requests: bool,
        /// Leave out the overall state of each staging project.
        #[arg(long)]
        no_status: bool,
    },
    /// Show one staging project of a workflow project.
    StagingProject {
        /// The workflow project the staging project belongs to.
        project: String,
        /// The staging project name.
        name: String,
        /// Leave out the staged, untracked, and obsolete request
        /// lists.
        #[arg(long)]
        no_requests: bool,
        /// Leave out the overall state of the staging project.
        #[arg(long)]
        no_status: bool,
    },
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
        /// At most this many entries.
        #[arg(long)]
        limit: Option<u32>,
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
    /// Set fields of a project's meta. A mutation: dry-run is the
    /// default (only the plan is printed); `--live` performs the
    /// write. Only the given fields change; the rest of the meta is
    /// kept as it is. There is no flag to clear a field.
    SetProjectMeta {
        /// The project whose meta is written.
        project: String,
        /// The new title.
        #[arg(long)]
        title: Option<String>,
        /// The new description.
        #[arg(long)]
        description: Option<String>,
        /// The new URL.
        #[arg(long)]
        url: Option<String>,
        /// The new scmsync URL.
        #[arg(long)]
        scmsync: Option<String>,
        /// The new devel project.
        #[arg(long)]
        devel_project: Option<String>,
    },
    /// Show a package's meta (title, description, persons, devel project).
    PackageMeta {
        /// The project the package lives in.
        project: String,
        /// The package to show.
        package: String,
    },
    /// Set fields of a package's meta. A mutation: dry-run is the
    /// default (only the plan is printed); `--live` performs the
    /// write. Only the given fields change; the rest of the meta is
    /// kept as it is. There is no flag to clear a field.
    SetPackageMeta {
        /// The project the package lives in.
        project: String,
        /// The package whose meta is written.
        package: String,
        /// The new title.
        #[arg(long)]
        title: Option<String>,
        /// The new description.
        #[arg(long)]
        description: Option<String>,
        /// The new URL.
        #[arg(long)]
        url: Option<String>,
        /// The new scmsync URL.
        #[arg(long)]
        scmsync: Option<String>,
        /// The new release name.
        #[arg(long)]
        releasename: Option<String>,
        /// The new devel project.
        #[arg(long)]
        devel_project: Option<String>,
        /// The new devel package. Needs `--devel-project` unless the
        /// package already has a devel project.
        #[arg(long)]
        devel_package: Option<String>,
    },
    /// Update a package's meta from its spec file. A mutation:
    /// dry-run is the default (only the plan is printed); `--live`
    /// performs the update. The spec file is found by listing the
    /// package files when exactly one `*.spec` exists; otherwise
    /// name it with `--spec`.
    UpdatePackageMetaFromSpec {
        /// The project the package lives in.
        project: String,
        /// The package whose meta is updated.
        package: String,
        /// The spec file to read the meta from.
        #[arg(long)]
        spec: Option<String>,
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
    /// Search projects by a person's login: every project whose meta
    /// names the login in a role.
    SearchProjectsByPerson {
        /// The login to search projects for.
        userid: String,
    },
    /// Count the projects syncing from a git URL prefix.
    CountScmsyncProjects {
        /// The git URL prefix to count projects for.
        url_prefix: String,
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
    /// Without a login, the authenticated user.
    Whois {
        /// The login name to look up; without one, the
        /// authenticated user.
        login: Option<String>,
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
    /// Upload a file into a package. A mutation: dry-run is the
    /// default (only the plan is printed); `--live` performs the
    /// upload. The local file is read from `--from`; `--binary`
    /// uploads bytes instead of text.
    UploadFile {
        /// The project the package lives in.
        project: String,
        /// The package to upload into.
        package: String,
        /// The file name on the server.
        file: String,
        /// The local file to read the content from.
        #[arg(long)]
        from: PathBuf,
        /// Upload bytes instead of text: a binary file uploads
        /// instead of failing the UTF-8 check a text upload would hit.
        #[arg(long)]
        binary: bool,
    },
    /// Create a project. A mutation: dry-run is the default (only
    /// the plan is printed); `--live` performs the creation.
    CreateProject {
        /// The project to create.
        project: String,
        /// The project title.
        #[arg(long)]
        title: String,
        /// The project description.
        #[arg(long)]
        description: String,
    },
    /// Create a package. A mutation: dry-run is the default (only
    /// the plan is printed); `--live` performs the creation.
    /// Without `--scmsync` and `--force` the package is created
    /// directly; with either, the creation goes through the options
    /// form, which refuses an existing package unless `--force` is
    /// set and refuses a project managed in git.
    CreatePackage {
        /// The project to create the package in.
        project: String,
        /// The package to create.
        package: String,
        /// The package title.
        #[arg(long)]
        title: String,
        /// The package description.
        #[arg(long)]
        description: String,
        /// The git repository and branch the package syncs its
        /// sources from, as `url#branch`.
        #[arg(long)]
        scmsync: Option<String>,
        /// Overwrite the package when it already exists.
        #[arg(long)]
        force: bool,
    },
    /// Delete a package. A mutation: dry-run is the default (only
    /// the plan is printed); `--live` performs the deletion.
    /// Without `--force` a package with open submit requests is
    /// refused.
    DeletePackage {
        /// The project the package lives in.
        project: String,
        /// The package to delete.
        package: String,
        /// Delete even when open submit requests involve the package.
        #[arg(long)]
        force: bool,
    },
    /// Delete a `_link` file from a package. A mutation: dry-run is
    /// the default (only the plan is printed); `--live` performs the
    /// deletion. The package keeps its files and meta, but stops
    /// tracking the source.
    DeleteLink {
        /// The project the package lives in.
        project: String,
        /// The package whose `_link` is deleted.
        package: String,
    },
    /// Delete a project. A mutation: dry-run is the default (only
    /// the plan is printed); `--live` performs the deletion.
    /// Without `--recursive` a project holding packages is refused.
    DeleteProject {
        /// The project to delete.
        project: String,
        /// Delete the project with all its packages.
        #[arg(long)]
        recursive: bool,
        /// Ignore other projects depending on this one.
        #[arg(long)]
        force: bool,
        /// A comment recorded on the delete.
        #[arg(long)]
        comment: Option<String>,
    },
    /// Restore a deleted package. A mutation: dry-run is the
    /// default (only the plan is printed); `--live` performs the
    /// restore.
    UndeletePackage {
        /// The project the package lives in.
        project: String,
        /// The package to restore.
        package: String,
    },
    /// Restore a deleted project. A mutation: dry-run is the
    /// default (only the plan is printed); `--live` performs the
    /// restore.
    UndeleteProject {
        /// The project to restore.
        project: String,
    },
    /// Aggregate a package's binaries into another package. A
    /// mutation: dry-run is the default (only the plan is printed);
    /// `--live` performs the aggregation. The target package is
    /// created from the source package's meta when it does not exist
    /// yet; an `_aggregate` file that already exists is refused.
    AggregatePackage {
        /// The project the source package lives in.
        source_project: String,
        /// The source package.
        source_package: String,
        /// The project the target package lives in.
        target_project: String,
        /// The target package.
        target_package: String,
        /// Map a source repository to a target repository, as
        /// `SOURCE=TARGET` (repeatable).
        #[arg(long)]
        repository_map: Vec<String>,
        /// Aggregate binaries only, not the sources.
        #[arg(long)]
        no_sources: bool,
        /// Disable publishing of the aggregate package.
        #[arg(long)]
        disable_publish: bool,
    },
    /// Release a package's sources and binaries. A mutation:
    /// dry-run is the default (only the plan is printed); `--live`
    /// performs the release. A release to a target project needs a
    /// repository and a target repository.
    ReleasePackage {
        /// The project the package lives in.
        project: String,
        /// The package to release.
        package: String,
        /// Release only binaries from this repository.
        #[arg(long)]
        repository: Option<String>,
        /// Release only binaries for this architecture.
        #[arg(long)]
        arch: Option<String>,
        /// Release only to this project.
        #[arg(long)]
        target_project: Option<String>,
        /// Release only to this repository in the target project.
        #[arg(long)]
        target_repository: Option<String>,
        /// Rename binaries during the release using this release tag.
        #[arg(long)]
        set_release: Option<String>,
        /// Run the release immediately instead of queueing it as a job.
        #[arg(long)]
        no_delay: bool,
    },
    /// Release a whole project's sources and binaries. A mutation:
    /// dry-run is the default (only the plan is printed); `--live`
    /// performs the release. A release to a target project needs a
    /// repository and a target repository.
    ReleaseProject {
        /// The project to release.
        project: String,
        /// Release only binaries from this repository.
        #[arg(long)]
        repository: Option<String>,
        /// Release only binaries for this architecture.
        #[arg(long)]
        arch: Option<String>,
        /// Release only to this project.
        #[arg(long)]
        target_project: Option<String>,
        /// Release only to this repository in the target project.
        #[arg(long)]
        target_repository: Option<String>,
        /// Rename binaries during the release using this release tag.
        #[arg(long)]
        set_release: Option<String>,
        /// Run the release immediately instead of queueing it as a job.
        #[arg(long)]
        no_delay: bool,
    },
}

/// Run a `remote` command, returning the JSON payload and its text
/// rendering. `live` performs the group's mutations; the reads
/// ignore it.
///
/// Credentials resolve per command (see `docs/cli.md`).
pub fn run_remote(
    command: RemoteCommand,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let (base, instances) = obs_endpoint()?;
    // Credentials resolve per command (see `docs/cli.md`).
    let credentials = match &command {
        RemoteCommand::Cat { .. }
        | RemoteCommand::Download { .. }
        | RemoteCommand::ListFiles { .. }
        | RemoteCommand::ListPackages { .. }
        | RemoteCommand::ProjectMeta { .. }
        | RemoteCommand::PackageMeta { .. }
        | RemoteCommand::LinkTarget { .. }
        | RemoteCommand::DevelProject { .. }
        | RemoteCommand::RevisionPatch { .. }
        | RemoteCommand::DiffPackages { .. } => Credentials::None,
        RemoteCommand::ProjectDiff {
            annotate_requests, ..
        } if !*annotate_requests => Credentials::None,
        RemoteCommand::RepoAdd { .. } | RemoteCommand::RepoRemove { .. } if !live => {
            Credentials::None
        }
        RemoteCommand::UploadFile { .. } if !live => Credentials::None,
        RemoteCommand::CreateProject { .. } if !live => Credentials::None,
        RemoteCommand::CreatePackage { .. } if !live => Credentials::None,
        RemoteCommand::SetProjectMeta { .. } if !live => Credentials::None,
        RemoteCommand::SetPackageMeta { .. } if !live => Credentials::None,
        RemoteCommand::UpdatePackageMetaFromSpec { .. } if !live => Credentials::None,
        RemoteCommand::DeletePackage { .. } if !live => Credentials::None,
        RemoteCommand::DeleteLink { .. } if !live => Credentials::None,
        RemoteCommand::DeleteProject { .. } if !live => Credentials::None,
        RemoteCommand::UndeletePackage { .. } if !live => Credentials::None,
        RemoteCommand::UndeleteProject { .. } if !live => Credentials::None,
        RemoteCommand::AggregatePackage { .. } if !live => Credentials::None,
        RemoteCommand::ReleasePackage { .. } if !live => Credentials::None,
        RemoteCommand::ReleaseProject { .. } if !live => Credentials::None,
        _ => EnvObsAuth::credentials_for_url(&base, &base, &instances)?,
    };
    let client =
        ObsClient::with_base_and_credentials(&http, &base, credentials)?.with_debug(debug.clone());
    match command {
        RemoteCommand::Cat {
            project,
            package,
            file,
            rev,
        } => {
            let rev = rev.as_deref().map(RevisionSpec::parse).transpose()?;
            let content = client.cat(&project, &package, &file, rev.clone())?;
            // Content commands render verbatim: the text IS the file.
            // The writer terminates the payload with a trailing newline
            // (an already-terminated payload is untouched — see
            // `terminated` in main.rs), so a file ending in a newline
            // gains no extra blank line.
            Ok(cat_result(&project, &package, &file, rev, content))
        }
        RemoteCommand::Download {
            project,
            package,
            file,
            rev,
            output,
        } => {
            let rev = rev.as_deref().map(RevisionSpec::parse).transpose()?;
            let bytes =
                client.download_source_to_file(&project, &package, &file, rev.as_ref(), &output)?;
            Ok(download_result(
                &project,
                &package,
                &file,
                rev.as_ref(),
                &output,
                bytes,
            ))
        }
        RemoteCommand::ListFiles {
            project,
            package,
            rev,
        } => {
            let rev = rev.as_deref().map(RevisionSpec::parse).transpose()?;
            let files = client.list_files(&project, &package, rev.as_ref())?;
            let payload = json!({
                "project": project,
                "package": package,
                "rev": match &rev {
                    None => Value::Null,
                    Some(RevisionSpec::Rev(n)) => json!(n),
                    Some(RevisionSpec::Md5(md5)) => json!(md5),
                },
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
        RemoteCommand::ListStagingProjects {
            project,
            no_requests,
            no_status,
        } => {
            let options = StagingReadOptions {
                requests: !no_requests,
                status: !no_status,
            };
            let projects = client.list_staging_projects(&project, options)?;
            let text = with_header(
                &format!("project: {project}"),
                render_staging_projects(&projects),
            );
            let payload = json!({
                "project": project,
                "projects": projects.iter().map(staging_project_json).collect::<Vec<_>>(),
            });
            Ok((payload, text))
        }
        RemoteCommand::StagingProject {
            project,
            name,
            no_requests,
            no_status,
        } => {
            let options = StagingReadOptions {
                requests: !no_requests,
                status: !no_status,
            };
            let staging = client.staging_project(&project, &name, options)?;
            let text = with_header(
                &format!("project: {project} staging: {name}"),
                render_staging_projects(std::slice::from_ref(&staging)),
            );
            let mut payload = staging_project_json(&staging);
            payload["project"] = json!(project);
            Ok((payload, text))
        }
        RemoteCommand::PackageLog {
            project,
            package,
            range,
            limit,
        } => {
            let parsed = range.as_deref().map(RevisionRange::parse).transpose()?;
            let entries = client.package_log(&project, &package, parsed.as_ref(), limit)?;
            let payload = json!({
                "project": project,
                "package": package,
                "range": range,
                "limit": limit,
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
        RemoteCommand::SetProjectMeta {
            project,
            title,
            description,
            url,
            scmsync,
            devel_project,
        } => {
            let flags = ProjectMetaFlags {
                title,
                description,
                url,
                scmsync,
                devel_project,
            };
            let fields = project_meta_fields(&flags)?;
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            let current = if live {
                client.project_meta(&project)?
            } else {
                empty_project_meta(&project)
            };
            let merged = merge_project_meta(&current, &flags);
            client.set_project_meta(&project, &merged, &dry_run)?;
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(set_project_meta_result(
                &merged,
                &fields,
                dry_run.enabled(),
                plan,
            ))
        }
        RemoteCommand::SetPackageMeta {
            project,
            package,
            title,
            description,
            url,
            scmsync,
            releasename,
            devel_project,
            devel_package,
        } => {
            let flags = PackageMetaFlags {
                title,
                description,
                url,
                scmsync,
                releasename,
                devel_project,
                devel_package,
            };
            let fields = package_meta_fields(&flags)?;
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            let current = if live {
                client.package_meta(&project, &package)?
            } else {
                empty_package_meta(&project, &package)
            };
            let merged = merge_package_meta(&current, &flags)?;
            client.set_package_meta(&project, &package, &merged, &dry_run)?;
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(set_package_meta_result(
                &merged,
                &fields,
                dry_run.enabled(),
                plan,
            ))
        }
        RemoteCommand::UpdatePackageMetaFromSpec {
            project,
            package,
            spec,
        } => {
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            client.update_package_meta_from_spec(&project, &package, spec.as_deref(), &dry_run)?;
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(update_package_meta_from_spec_result(
                &project,
                &package,
                &spec,
                dry_run.enabled(),
                plan,
            ))
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
        RemoteCommand::SearchProjectsByPerson { userid } => {
            let matches = client.search_projects_by_person(&userid)?;
            let text = with_header(
                &format!("login: {userid}"),
                render_project_matches(&userid, &matches),
            );
            let payload = json!({
                "login": userid,
                "projects": matches.iter().map(project_match_json).collect::<Vec<_>>(),
            });
            Ok((payload, text))
        }
        RemoteCommand::CountScmsyncProjects { url_prefix } => {
            let count = client.count_scmsync_projects(&url_prefix)?;
            let text = with_header(
                &format!("url-prefix: {url_prefix}"),
                format!("count: {count}"),
            );
            let payload = json!({
                "url_prefix": url_prefix,
                "count": count,
            });
            Ok((payload, text))
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
            let login = resolve_whois_login(&login, client.credentials())?;
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
                rebuild: None,
                block: None,
                linkedbuild: None,
                downloads: vec![],
                release_targets: vec![],
                hostsystem: vec![],
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
            let result_line = repo_add_result_line(
                &name,
                &project,
                &arch,
                &repository.paths,
                disable_publish,
                dry_run.enabled(),
            );
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
        RemoteCommand::UploadFile {
            project,
            package,
            file,
            from,
            binary,
        } => {
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            let rev = if binary {
                let bytes = read_local_file_bytes(&from)?;
                client.upload_file_bytes(&project, &package, &file, &bytes, &dry_run)?
            } else {
                let content = read_local_file_text(&from)?;
                client.upload_file(&project, &package, &file, &content, &dry_run)?
            };
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(upload_file_result(
                &project,
                &package,
                &file,
                rev,
                dry_run.enabled(),
                plan,
            ))
        }
        RemoteCommand::CreateProject {
            project,
            title,
            description,
        } => {
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            client.create_project(&project, &title, &description, &dry_run)?;
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(create_project_result(
                &project,
                &title,
                &description,
                dry_run.enabled(),
                plan,
            ))
        }
        RemoteCommand::CreatePackage {
            project,
            package,
            title,
            description,
            scmsync,
            force,
        } => {
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            let options = CreatePackageOptions {
                scmsync: scmsync.clone(),
                force,
            };
            if scmsync.is_some() || force {
                client.create_package_with_options(
                    &project,
                    &package,
                    &title,
                    &description,
                    &options,
                    &dry_run,
                )?;
            } else {
                client.create_package(&project, &package, &title, &description, &dry_run)?;
            }
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(create_package_result(
                &project,
                &package,
                &title,
                &description,
                &options,
                dry_run.enabled(),
                plan,
            ))
        }
        RemoteCommand::DeletePackage {
            project,
            package,
            force,
        } => {
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            client.delete_package(&project, &package, force, &dry_run)?;
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(delete_package_result(
                &project,
                &package,
                force,
                dry_run.enabled(),
                plan,
            ))
        }
        RemoteCommand::DeleteLink { project, package } => {
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            client.delete_link(&project, &package, &dry_run)?;
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(delete_link_result(
                &project,
                &package,
                dry_run.enabled(),
                plan,
            ))
        }
        RemoteCommand::DeleteProject {
            project,
            recursive,
            force,
            comment,
        } => {
            let options = DeleteProjectOptions {
                recursive,
                force,
                comment: comment.clone(),
            };
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            client.delete_project(&project, &options, &dry_run)?;
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(delete_project_result(
                &project,
                &options,
                dry_run.enabled(),
                plan,
            ))
        }
        RemoteCommand::UndeletePackage { project, package } => {
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            client.undelete_package(&project, &package, &dry_run)?;
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(undelete_package_result(
                &project,
                &package,
                dry_run.enabled(),
                plan,
            ))
        }
        RemoteCommand::UndeleteProject { project } => {
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            client.undelete_project(&project, &dry_run)?;
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(undelete_project_result(&project, dry_run.enabled(), plan))
        }
        RemoteCommand::AggregatePackage {
            source_project,
            source_package,
            target_project,
            target_package,
            repository_map,
            no_sources,
            disable_publish,
        } => {
            let mappings = parse_repository_map(&repository_map)?;
            let options = AggregateOptions {
                repository_map: mappings,
                no_sources,
                disable_publish,
            };
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            client.aggregate_package(
                &source_project,
                &source_package,
                &target_project,
                &target_package,
                &options,
                &dry_run,
            )?;
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(aggregate_package_result(
                &source_project,
                &source_package,
                &target_project,
                &target_package,
                &options,
                dry_run.enabled(),
                plan,
            ))
        }
        RemoteCommand::ReleasePackage {
            project,
            package,
            repository,
            arch,
            target_project,
            target_repository,
            set_release,
            no_delay,
        } => {
            let options = ReleaseOptions {
                repository,
                architecture: arch,
                target_project,
                target_repository,
                set_release,
                no_delay,
            };
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            client.release_package(&project, &package, &options, &dry_run)?;
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(release_package_result(
                &project,
                &package,
                &options,
                dry_run.enabled(),
                plan,
            ))
        }
        RemoteCommand::ReleaseProject {
            project,
            repository,
            arch,
            target_project,
            target_repository,
            set_release,
            no_delay,
        } => {
            let options = ReleaseOptions {
                repository,
                architecture: arch,
                target_project,
                target_repository,
                set_release,
                no_delay,
            };
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            client.release_project(&project, &options, &dry_run)?;
            let plan = if dry_run.enabled() {
                Some(dry_run.plan())
            } else {
                None
            };
            Ok(release_project_result(
                &project,
                &options,
                dry_run.enabled(),
                plan,
            ))
        }
    }
}

/// The OBS base URL and the config file's `[instances]` table, for
/// credential resolution (see [`auth::resolve_obs_basic_for_url`]).
/// Credentials are NOT resolved here: they resolve per command in
/// `run_remote` (see `docs/cli.md` for which commands those are).
/// Here the dry-run mutations need none either, but the library proves
/// it (its dry-run guard runs before the credentials check), so nothing
/// is swallowed.
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

// ---------------------------------------------------------------------
// Per-command result builders: pure functions of the typed values, so
// every command's payload and text are unit-testable without a server.
// ---------------------------------------------------------------------

/// The `repo-add` result line: the repository and project, plus the
/// arch list and the rebuild-source paths — the repository's
/// definition, so the text carries the definition from the payload.
/// An empty arch or paths segment is omitted; with both empty the
/// definition parenthetical is omitted. The CLI requires a non-empty
/// arch and paths, so the omission is defensive rendering.
/// `disable_publish` is a
/// boolean, not an identifier: it reads as `(publish disabled)` on
/// the live line when set, the dry-run plan names it when set, and
/// the JSON payload carries it. `changed` needs no line: a live add
/// always changes the meta, and dry-run always reads "would add".
fn repo_add_result_line(
    name: &str,
    project: &str,
    arch: &[String],
    paths: &[(String, String)],
    disable_publish: bool,
    dry_run: bool,
) -> String {
    let mut segments = Vec::new();
    let arch = arch.join(", ");
    if !arch.is_empty() {
        segments.push(format!("arch: {arch}"));
    }
    let paths = paths
        .iter()
        .map(|(project, repository)| format!("{project}/{repository}"))
        .collect::<Vec<_>>()
        .join(", ");
    if !paths.is_empty() {
        segments.push(format!("paths: {paths}"));
    }
    let definition = if segments.is_empty() {
        String::new()
    } else {
        format!(" ({})", segments.join("; "))
    };
    let suffix = if disable_publish && !dry_run {
        " (publish disabled)"
    } else {
        ""
    };
    if dry_run {
        format!("dry-run: would add repository {name} to project {project}{definition}")
    } else {
        format!("added repository {name} to project {project}{definition}{suffix}")
    }
}

/// Prepend the one-line coordinate header to a text rendering
/// (design rule 1): `header` first, then the body unchanged.
fn with_header(header: &str, text: String) -> String {
    format!("{header}\n{text}")
}

fn cat_result(
    project: &str,
    package: &str,
    file: &str,
    rev: Option<RevisionSpec>,
    content: String,
) -> (Value, String) {
    let payload = json!({
        "project": project,
        "package": package,
        "file": file,
        "rev": match &rev {
            None => Value::Null,
            Some(RevisionSpec::Rev(n)) => json!(n),
            Some(RevisionSpec::Md5(md5)) => json!(md5),
        },
        "content": content.clone(),
    });
    // Content commands render verbatim: the text IS the file.
    (payload, content)
}

/// The `download` result: the payload carries the coordinates, the
/// revision (the normalized spec string — a number or an md5 — or
/// null for latest), the written path, and the byte count; the text
/// carries the coordinate header and the confirmation line. The
/// bytes themselves ride neither rendering: the file is the output.
fn download_result(
    project: &str,
    package: &str,
    file: &str,
    rev: Option<&RevisionSpec>,
    output: &Path,
    bytes: u64,
) -> (Value, String) {
    let payload = json!({
        "project": project,
        "package": package,
        "file": file,
        "rev": rev.map(|spec| spec.to_string()),
        "path": output.to_string_lossy(),
        "size_bytes": bytes,
    });
    (
        payload,
        with_header(
            &format!("project: {project} package: {package} file: {file}"),
            format!("downloaded {file} ({bytes} bytes) to {}", output.display()),
        ),
    )
}

/// Read a local file as text for upload. A missing file, or a file
/// that is not valid UTF-8, is an input error naming the path: the
/// text upload takes text, and the bytes upload takes `--binary`.
fn read_local_file_text(path: &PathBuf) -> Result<String, GeectlError> {
    std::fs::read_to_string(path).map_err(|e| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            format!("cannot read {}: {e}", path.display()),
        )
    })
}

/// Read a local file as bytes for a binary upload. A missing file is
/// an input error naming the path.
fn read_local_file_bytes(path: &PathBuf) -> Result<Vec<u8>, GeectlError> {
    std::fs::read(path).map_err(|e| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            format!("cannot read {}: {e}", path.display()),
        )
    })
}

/// The `upload-file` result: the payload carries the new revision
/// (`null` in dry-run); the text carries it too, in the result line.
fn upload_file_result(
    project: &str,
    package: &str,
    file: &str,
    rev: Option<u64>,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let payload = json!({
        "project": project,
        "package": package,
        "file": file,
        "rev": rev,
    });
    let result_line = if dry_run {
        format!("dry-run: would upload file {file} to project {project} package {package}")
    } else {
        match rev {
            Some(rev) => format!(
                "uploaded file {file} to project {project} package {package} (revision {rev})"
            ),
            None => format!("uploaded file {file} to project {project} package {package}"),
        }
    };
    mutation_result(
        payload,
        format!("project: {project} package: {package} file: {file}"),
        result_line,
        dry_run,
        plan,
    )
}

/// The `create-project` result: the payload carries the title and
/// the description; the text carries the title and leaves the
/// description to the payload (long free text).
fn create_project_result(
    project: &str,
    title: &str,
    description: &str,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let payload = json!({
        "project": project,
        "title": title,
        "description": description,
    });
    let result_line = if dry_run {
        format!("dry-run: would create project {project} (title: {title})")
    } else {
        format!("created project {project} (title: {title})")
    };
    mutation_result(
        payload,
        format!("project: {project}"),
        result_line,
        dry_run,
        plan,
    )
}

/// The `create-package` result: the payload carries the title,
/// the description, the scmsync, and `force`; the text carries the
/// title and the scmsync and leaves the description to the payload
/// (long free text) and `force` to the dry-run plan (a boolean).
fn create_package_result(
    project: &str,
    package: &str,
    title: &str,
    description: &str,
    options: &CreatePackageOptions,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let payload = json!({
        "project": project,
        "package": package,
        "title": title,
        "description": description,
        "scmsync": options.scmsync,
        "force": options.force,
    });
    let definition = match &options.scmsync {
        Some(scmsync) => format!(" (title: {title}; scmsync: {scmsync})"),
        None => format!(" (title: {title})"),
    };
    let result_line = if dry_run {
        format!("dry-run: would create package {package} in project {project}{definition}")
    } else {
        format!("created package {package} in project {project}{definition}")
    };
    mutation_result(
        payload,
        format!("project: {project} package: {package}"),
        result_line,
        dry_run,
        plan,
    )
}

/// The `delete-package` result: the payload carries `force`; the
/// text carries it too, in the result line — the dry-run plan does
/// not name it.
fn delete_package_result(
    project: &str,
    package: &str,
    force: bool,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let payload = json!({
        "project": project,
        "package": package,
        "force": force,
    });
    let suffix = if force { " (force)" } else { "" };
    let result_line = if dry_run {
        format!("dry-run: would delete package {package} from project {project}{suffix}")
    } else {
        format!("deleted package {package} from project {project}{suffix}")
    };
    mutation_result(
        payload,
        format!("project: {project} package: {package}"),
        result_line,
        dry_run,
        plan,
    )
}

/// The `delete-link` result: the payload carries the coordinates;
/// the text states the deletion, or the dry-run plan.
fn delete_link_result(
    project: &str,
    package: &str,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let payload = json!({
        "project": project,
        "package": package,
    });
    let result_line = if dry_run {
        format!("dry-run: would delete link of package {package} from project {project}")
    } else {
        format!("deleted link of package {package} from project {project}")
    };
    mutation_result(
        payload,
        format!("project: {project} package: {package}"),
        result_line,
        dry_run,
        plan,
    )
}

/// A project meta with only the name set: the base the flags
/// merge onto in dry-run, where no server is read.
fn empty_project_meta(project: &str) -> ProjectMeta {
    ProjectMeta {
        name: project.to_string(),
        title: None,
        description: None,
        url: None,
        scmsync: None,
        devel: None,
        persons: vec![],
        groups: vec![],
        repositories: vec![],
    }
}

/// A package meta with only the name and project set: the base the
/// flags merge onto in dry-run, where no server is read.
fn empty_package_meta(project: &str, package: &str) -> PackageMeta {
    PackageMeta {
        name: package.to_string(),
        project: project.to_string(),
        title: None,
        description: None,
        url: None,
        scmsync: None,
        releasename: None,
        devel: None,
        persons: vec![],
        groups: vec![],
    }
}

/// The flags of a `set-project-meta` call: one optional value per
/// writable field. Absent means keep the current value.
struct ProjectMetaFlags {
    title: Option<String>,
    description: Option<String>,
    url: Option<String>,
    scmsync: Option<String>,
    devel_project: Option<String>,
}

/// The flags of a `set-package-meta` call: one optional value per
/// writable field. Absent means keep the current value.
struct PackageMetaFlags {
    title: Option<String>,
    description: Option<String>,
    url: Option<String>,
    scmsync: Option<String>,
    releasename: Option<String>,
    devel_project: Option<String>,
    devel_package: Option<String>,
}

/// Merge the given project-meta flags onto the current meta: a flag
/// replaces its field, an absent flag keeps the current value.
fn merge_project_meta(current: &ProjectMeta, flags: &ProjectMetaFlags) -> ProjectMeta {
    let mut merged = current.clone();
    if let Some(title) = &flags.title {
        merged.title = Some(title.clone());
    }
    if let Some(description) = &flags.description {
        merged.description = Some(description.clone());
    }
    if let Some(url) = &flags.url {
        merged.url = Some(url.clone());
    }
    if let Some(scmsync) = &flags.scmsync {
        merged.scmsync = Some(scmsync.clone());
    }
    if let Some(devel) = &flags.devel_project {
        merged.devel = Some(devel.clone());
    }
    merged
}

/// Merge the given package-meta flags onto the current meta: a flag
/// replaces its field, an absent flag keeps the current value. A
/// `--devel-project` without `--devel-package` keeps the current
/// devel package, or the package's own name when it has none. A
/// `--devel-package` without `--devel-project` keeps the current
/// devel project, and errors when the package has none — in dry-run
/// the current meta is unread, so the pair is an error there too.
fn merge_package_meta(
    current: &PackageMeta,
    flags: &PackageMetaFlags,
) -> Result<PackageMeta, GeectlError> {
    let mut merged = current.clone();
    if let Some(title) = &flags.title {
        merged.title = Some(title.clone());
    }
    if let Some(description) = &flags.description {
        merged.description = Some(description.clone());
    }
    if let Some(url) = &flags.url {
        merged.url = Some(url.clone());
    }
    if let Some(scmsync) = &flags.scmsync {
        merged.scmsync = Some(scmsync.clone());
    }
    if let Some(releasename) = &flags.releasename {
        merged.releasename = Some(releasename.clone());
    }
    match (&flags.devel_project, &flags.devel_package) {
        (Some(project), package) => {
            let package = match package {
                Some(package) => package.clone(),
                None => match &current.devel {
                    Some((_, package)) => package.clone(),
                    None => merged.name.clone(),
                },
            };
            merged.devel = Some((project.clone(), package));
        }
        (None, Some(package)) => {
            if let Some((project, _)) = &current.devel {
                merged.devel = Some((project.clone(), package.clone()));
            } else {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "--devel-package needs --devel-project or a devel project already set on the package",
                ));
            }
        }
        (None, None) => {}
    }
    Ok(merged)
}

/// The project-meta fields a `set-project-meta` call sets, as
/// `name: value` entries for the result line. An empty flag set is
/// an input error: a write with nothing to write is a no-op.
fn project_meta_fields(flags: &ProjectMetaFlags) -> Result<Vec<String>, GeectlError> {
    let mut fields = Vec::new();
    if let Some(title) = &flags.title {
        fields.push(format!("title: {title}"));
    }
    if let Some(description) = &flags.description {
        fields.push(format!("description: {description}"));
    }
    if let Some(url) = &flags.url {
        fields.push(format!("url: {url}"));
    }
    if let Some(scmsync) = &flags.scmsync {
        fields.push(format!("scmsync: {scmsync}"));
    }
    if let Some(devel) = &flags.devel_project {
        fields.push(format!("devel-project: {devel}"));
    }
    if fields.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "set-project-meta needs at least one of --title, --description, --url, --scmsync, --devel-project",
        ));
    }
    Ok(fields)
}

/// The package-meta fields a `set-package-meta` call sets, as
/// `name: value` entries for the result line. An empty flag set is
/// an input error: a write with nothing to write is a no-op.
fn package_meta_fields(flags: &PackageMetaFlags) -> Result<Vec<String>, GeectlError> {
    let mut fields = Vec::new();
    if let Some(title) = &flags.title {
        fields.push(format!("title: {title}"));
    }
    if let Some(description) = &flags.description {
        fields.push(format!("description: {description}"));
    }
    if let Some(url) = &flags.url {
        fields.push(format!("url: {url}"));
    }
    if let Some(scmsync) = &flags.scmsync {
        fields.push(format!("scmsync: {scmsync}"));
    }
    if let Some(releasename) = &flags.releasename {
        fields.push(format!("releasename: {releasename}"));
    }
    if let Some(devel) = &flags.devel_project {
        fields.push(format!("devel-project: {devel}"));
    }
    if let Some(devel) = &flags.devel_package {
        fields.push(format!("devel-package: {devel}"));
    }
    if fields.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "set-package-meta needs at least one of --title, --description, --url, --scmsync, --releasename, --devel-project, --devel-package",
        ));
    }
    Ok(fields)
}

/// Drop the null and empty-array keys from a meta payload: in dry-run
/// the meta is flags-only, so a null or empty value means "not set by
/// the flags", not "cleared" — the live meta keeps those fields.
/// The dry-run payload carries only the fields the flags set.
fn dry_run_meta_payload(mut payload: Value) -> Value {
    if let Value::Object(map) = &mut payload {
        map.retain(|_, value| {
            !value.is_null() && !value.as_array().is_some_and(|items| items.is_empty())
        });
    }
    payload
}

/// The `set-project-meta` result: the payload carries the meta as
/// written; the text states the write, or the dry-run plan. In
/// dry-run the payload drops null and empty-array keys; the
/// flags-only shape holds because the dry-run merge starts from
/// the empty meta.
fn set_project_meta_result(
    meta: &ProjectMeta,
    fields: &[String],
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let result_line = if dry_run {
        format!(
            "dry-run: would set project meta of {} ({})",
            meta.name,
            fields.join("; ")
        )
    } else {
        format!("set project meta of {} ({})", meta.name, fields.join("; "))
    };
    let payload = if dry_run {
        dry_run_meta_payload(project_meta_json(meta))
    } else {
        project_meta_json(meta)
    };
    mutation_result(
        payload,
        format!("project: {}", meta.name),
        result_line,
        dry_run,
        plan,
    )
}

/// The `set-package-meta` result: the payload carries the meta as
/// written; the text states the write, or the dry-run plan. In
/// dry-run the payload drops null and empty-array keys; the
/// flags-only shape holds because the dry-run merge starts from
/// the empty meta.
fn set_package_meta_result(
    meta: &PackageMeta,
    fields: &[String],
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let result_line = if dry_run {
        format!(
            "dry-run: would set package meta of {}/{} ({})",
            meta.project,
            meta.name,
            fields.join("; ")
        )
    } else {
        format!(
            "set package meta of {}/{} ({})",
            meta.project,
            meta.name,
            fields.join("; ")
        )
    };
    let payload = if dry_run {
        dry_run_meta_payload(package_meta_json(meta))
    } else {
        package_meta_json(meta)
    };
    mutation_result(
        payload,
        format!("project: {} package: {}", meta.project, meta.name),
        result_line,
        dry_run,
        plan,
    )
}

/// The `update-package-meta-from-spec` result: the payload carries
/// the coordinates and the spec file, when named; the text states
/// the update, or the dry-run plan.
fn update_package_meta_from_spec_result(
    project: &str,
    package: &str,
    spec: &Option<String>,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let payload = json!({
        "project": project,
        "package": package,
        "spec": spec,
    });
    let source = match spec {
        Some(spec) => format!("spec {spec}"),
        None => "the spec file".to_string(),
    };
    let result_line = if dry_run {
        format!("dry-run: would update package meta of {project}/{package} from {source}")
    } else {
        format!("updated package meta of {project}/{package} from {source}")
    };
    mutation_result(
        payload,
        format!("project: {project} package: {package}"),
        result_line,
        dry_run,
        plan,
    )
}

/// The `delete-project` result: the payload carries the options
/// and the comment; the text carries the options and leaves the
/// comment to the payload (a message, not an identifier).
fn delete_project_result(
    project: &str,
    options: &DeleteProjectOptions,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let payload = json!({
        "project": project,
        "recursive": options.recursive,
        "force": options.force,
        "comment": options.comment,
    });
    let mut parts: Vec<&str> = Vec::new();
    if options.recursive {
        parts.push("recursive");
    }
    if options.force {
        parts.push("force");
    }
    let suffix = if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    };
    let result_line = if dry_run {
        format!("dry-run: would delete project {project}{suffix}")
    } else {
        format!("deleted project {project}{suffix}")
    };
    mutation_result(
        payload,
        format!("project: {project}"),
        result_line,
        dry_run,
        plan,
    )
}

/// The `undelete-package` result: the payload carries the
/// coordinates; the text carries them too, in the header and the
/// result line.
fn undelete_package_result(
    project: &str,
    package: &str,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let payload = json!({
        "project": project,
        "package": package,
    });
    let result_line = if dry_run {
        format!("dry-run: would restore deleted package {package} in project {project}")
    } else {
        format!("restored deleted package {package} in project {project}")
    };
    mutation_result(
        payload,
        format!("project: {project} package: {package}"),
        result_line,
        dry_run,
        plan,
    )
}

/// The `undelete-project` result: the payload carries the project;
/// the text carries it too, in the header and the result line.
fn undelete_project_result(
    project: &str,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let payload = json!({
        "project": project,
    });
    let result_line = if dry_run {
        format!("dry-run: would restore deleted project {project}")
    } else {
        format!("restored deleted project {project}")
    };
    mutation_result(
        payload,
        format!("project: {project}"),
        result_line,
        dry_run,
        plan,
    )
}

/// Parse `--repository-map SOURCE=TARGET` values into
/// `(source, target)` pairs: the two sides split on the first `=`.
/// A value with no `=`, or an empty side, is an input error naming
/// the expected form.
fn parse_repository_map(mappings: &[String]) -> Result<Vec<(String, String)>, GeectlError> {
    mappings
        .iter()
        .map(|mapping| match mapping.split_once('=') {
            Some((source, target)) if !source.is_empty() && !target.is_empty() => {
                Ok((source.to_string(), target.to_string()))
            }
            _ => Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid repository mapping {mapping:?}: expected SOURCE=TARGET"),
            )),
        })
        .collect()
}

/// The option suffix for an aggregate result line: the set options
/// (the dry-run plan carries the same suffix).
fn aggregate_options_suffix(options: &AggregateOptions) -> String {
    let mut flags: Vec<String> = Vec::new();
    if options.no_sources {
        flags.push("no sources".to_string());
    }
    if options.disable_publish {
        flags.push("publish disabled".to_string());
    }
    if !options.repository_map.is_empty() {
        flags.push(format!(
            "{} repository mapping{}",
            options.repository_map.len(),
            if options.repository_map.len() == 1 {
                ""
            } else {
                "s"
            }
        ));
    }
    if flags.is_empty() {
        String::new()
    } else {
        format!(" ({})", flags.join(", "))
    }
}

/// The `aggregate-package` result: the payload carries the source,
/// the target, and the options; the text carries them too, in the
/// header and the result line.
fn aggregate_package_result(
    source_project: &str,
    source_package: &str,
    target_project: &str,
    target_package: &str,
    options: &AggregateOptions,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let payload = json!({
        "source_project": source_project,
        "source_package": source_package,
        "target_project": target_project,
        "target_package": target_package,
        "repository_map": options
            .repository_map
            .iter()
            .map(|(source, target)| json!({
                "source": source,
                "target": target,
            }))
            .collect::<Vec<_>>(),
        "no_sources": options.no_sources,
        "disable_publish": options.disable_publish,
    });
    let suffix = aggregate_options_suffix(options);
    let result_line = if dry_run {
        format!(
            "dry-run: would aggregate package {source_project}/{source_package} into {target_project}/{target_package}{suffix}"
        )
    } else {
        format!(
            "aggregated package {source_project}/{source_package} into {target_project}/{target_package}{suffix}"
        )
    };
    mutation_result(
        payload,
        format!(
            "source: {source_project}/{source_package} target: {target_project}/{target_package}"
        ),
        result_line,
        dry_run,
        plan,
    )
}

/// The option suffix for a release result line: the set options
/// (the dry-run plan carries the same suffix).
fn release_options_suffix(options: &ReleaseOptions) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(repository) = &options.repository {
        parts.push(format!("repository {repository}"));
    }
    if let Some(architecture) = &options.architecture {
        parts.push(format!("architecture {architecture}"));
    }
    if let Some(target_project) = &options.target_project {
        match &options.target_repository {
            Some(target_repository) => {
                parts.push(format!("to {target_project}/{target_repository}"));
            }
            None => parts.push(format!("to {target_project}")),
        }
    }
    if let Some(set_release) = &options.set_release {
        parts.push(format!("release tag {set_release}"));
    }
    if options.no_delay {
        parts.push("no delay".to_string());
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

/// The `release-package` result: the payload carries the options;
/// the text carries them too, in the result line.
fn release_package_result(
    project: &str,
    package: &str,
    options: &ReleaseOptions,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let payload = json!({
        "project": project,
        "package": package,
        "repository": options.repository,
        "architecture": options.architecture,
        "target_project": options.target_project,
        "target_repository": options.target_repository,
        "set_release": options.set_release,
        "no_delay": options.no_delay,
    });
    let suffix = release_options_suffix(options);
    let result_line = if dry_run {
        format!("dry-run: would release package {package} in project {project}{suffix}")
    } else {
        format!("released package {package} in project {project}{suffix}")
    };
    mutation_result(
        payload,
        format!("project: {project} package: {package}"),
        result_line,
        dry_run,
        plan,
    )
}

/// The `release-project` result: the payload carries the options;
/// the text carries them too, in the result line.
fn release_project_result(
    project: &str,
    options: &ReleaseOptions,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let payload = json!({
        "project": project,
        "repository": options.repository,
        "architecture": options.architecture,
        "target_project": options.target_project,
        "target_repository": options.target_repository,
        "set_release": options.set_release,
        "no_delay": options.no_delay,
    });
    let suffix = release_options_suffix(options);
    let result_line = if dry_run {
        format!("dry-run: would release project {project}{suffix}")
    } else {
        format!("released project {project}{suffix}")
    };
    mutation_result(
        payload,
        format!("project: {project}"),
        result_line,
        dry_run,
        plan,
    )
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

fn staged_request_json(request: &StagedRequest) -> Value {
    json!({
        "id": request.id,
        "type": request.request_type,
        "creator": request.creator,
        "state": request.state,
        "package": request.package,
        "superseded_by": request.superseded_by,
    })
}

fn staging_project_json(project: &StagingProject) -> Value {
    json!({
        "name": project.name,
        "state": project.state,
        "staged_requests": project
            .staged_requests
            .as_ref()
            .map(|requests| requests.iter().map(staged_request_json).collect::<Vec<_>>()),
        "untracked_requests": project
            .untracked_requests
            .as_ref()
            .map(|requests| requests.iter().map(staged_request_json).collect::<Vec<_>>()),
        "obsolete_requests": project
            .obsolete_requests
            .as_ref()
            .map(|requests| requests.iter().map(staged_request_json).collect::<Vec<_>>()),
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

/// The login a `whois` read looks up: the given login, or the
/// authenticated user's login when none is given. Fails closed
/// when the configured credentials carry no username.
fn resolve_whois_login(
    login: &Option<String>,
    credentials: &Credentials,
) -> Result<String, GeectlError> {
    match login {
        Some(login) => Ok(login.clone()),
        None => credentials.username().map(str::to_string).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ConfigError,
                "remote whois with no login needs OBS Basic credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            )
        }),
    }
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

/// Render staging projects: one section per project, with the state
/// on the name line and the staged, untracked, and obsolete request
/// lists as tables. A list the read did not ask for is absent, never
/// an empty table; an asked-for list with no requests reads `none`.
fn render_staging_projects(projects: &[StagingProject]) -> String {
    if projects.is_empty() {
        return "no staging projects".to_string();
    }
    let mut sections: Vec<String> = Vec::new();
    for project in projects {
        let mut lines = vec![match &project.state {
            Some(state) => format!("name: {} state: {}", project.name, state),
            None => format!("name: {}", project.name),
        }];
        render_staged_requests(&mut lines, "staged", &project.staged_requests);
        render_staged_requests(&mut lines, "untracked", &project.untracked_requests);
        render_staged_requests(&mut lines, "obsolete", &project.obsolete_requests);
        sections.push(lines.join("\n"));
    }
    sections.join("\n")
}

fn render_staged_requests(
    lines: &mut Vec<String>,
    label: &str,
    requests: &Option<Vec<StagedRequest>>,
) {
    match requests {
        None => {}
        Some(requests) if requests.is_empty() => {
            lines.push(format!("{label}: none"));
        }
        Some(requests) => {
            lines.push(format!("{label}:"));
            let rows: Vec<Vec<String>> = requests
                .iter()
                .map(|request| {
                    vec![
                        request.id.to_string(),
                        request.request_type.clone(),
                        request.creator.clone(),
                        request.state.clone(),
                        request.package.clone().unwrap_or_else(|| "-".to_string()),
                    ]
                })
                .collect();
            lines.push(render_table(
                &["ID", "TYPE", "CREATOR", "STATE", "PACKAGE"],
                &rows,
            ));
        }
    }
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
        assert!(matches!(parsed, RemoteCommand::Cat { rev: Some(ref r), .. } if r == "12"));
        let parsed = parse(&[
            "geectl",
            "remote",
            "cat",
            "openSUSE:Factory",
            "vim",
            "vim.spec",
            "--rev",
            "0123456789abcdef0123456789abcdef",
        ]);
        assert!(
            matches!(parsed, RemoteCommand::Cat { rev: Some(ref r), .. } if r == "0123456789abcdef0123456789abcdef")
        );
    }

    #[test]
    fn list_files_parses_optional_rev() {
        let parsed = parse(&["geectl", "remote", "list-files", "P", "K"]);
        assert!(matches!(
            parsed,
            RemoteCommand::ListFiles {
                project,
                package,
                rev: None,
            } if project == "P" && package == "K"
        ));
        let parsed = parse(&["geectl", "remote", "list-files", "P", "K", "--rev", "12"]);
        assert!(matches!(parsed, RemoteCommand::ListFiles { rev: Some(ref r), .. } if r == "12"));
        let parsed = parse(&[
            "geectl",
            "remote",
            "list-files",
            "P",
            "K",
            "--rev",
            "0123456789abcdef0123456789abcdef",
        ]);
        assert!(
            matches!(parsed, RemoteCommand::ListFiles { rev: Some(ref r), .. } if r == "0123456789abcdef0123456789abcdef")
        );
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
        let parsed = parse(&[
            "geectl",
            "remote",
            "package-log",
            "P",
            "K",
            "--range",
            "1:3",
            "--limit",
            "2",
        ]);
        assert!(matches!(
            parsed,
            RemoteCommand::PackageLog {
                range: Some(range),
                limit: Some(2),
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
        match parse(&["geectl", "remote", "whois", "alice"]) {
            RemoteCommand::Whois { login } => {
                assert_eq!(login.as_deref(), Some("alice"));
            }
            _ => panic!("wrong variant"),
        }
        match parse(&["geectl", "remote", "whois"]) {
            RemoteCommand::Whois { login } => assert_eq!(login, None),
            _ => panic!("wrong variant"),
        }
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
    fn resolve_whois_login_prefers_the_given_login() {
        let resolved = resolve_whois_login(
            &Some("alice".to_string()),
            &Credentials::Basic {
                username: "bob".to_string(),
                password: "secret".to_string(),
            },
        )
        .unwrap();
        assert_eq!(resolved, "alice");
    }

    #[test]
    fn resolve_whois_login_defaults_to_the_authenticated_user() {
        let resolved = resolve_whois_login(
            &None,
            &Credentials::Basic {
                username: "bob".to_string(),
                password: "secret".to_string(),
            },
        )
        .unwrap();
        assert_eq!(resolved, "bob");
    }

    #[test]
    fn resolve_whois_login_fails_closed_without_a_username() {
        for credentials in [
            Credentials::None,
            Credentials::Bearer {
                token: "token".to_string(),
            },
            Credentials::SshKey {
                path: std::path::PathBuf::from("/x"),
            },
        ] {
            let err = resolve_whois_login(&None, &credentials).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ConfigError);
        }
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
    fn upload_file_parses_from_and_binary() {
        let parsed = parse(&[
            "geectl",
            "remote",
            "upload-file",
            "P",
            "K",
            "vim.spec",
            "--from",
            "vim.spec",
        ]);
        assert!(matches!(
            parsed,
            RemoteCommand::UploadFile {
                from,
                binary: false,
                ..
            } if from.as_os_str() == std::ffi::OsStr::new("vim.spec")
        ));
        let parsed = parse(&[
            "geectl",
            "remote",
            "upload-file",
            "P",
            "K",
            "vim.spec",
            "--from",
            "vim.spec",
            "--binary",
        ]);
        assert!(matches!(
            parsed,
            RemoteCommand::UploadFile { binary: true, .. }
        ));
        assert!(
            Cli::try_parse_from(["geectl", "remote", "upload-file", "P", "K", "vim.spec"]).is_err()
        );
    }

    #[test]
    fn upload_file_result_carries_revision_in_both_formats() {
        let (payload, text) = upload_file_result("P", "K", "vim.spec", Some(7), false, None);
        assert_eq!(payload["project"], "P");
        assert_eq!(payload["package"], "K");
        assert_eq!(payload["file"], "vim.spec");
        assert_eq!(payload["rev"], 7);
        assert_eq!(payload["dry_run"], false);
        assert_eq!(
            text,
            "project: P package: K file: vim.spec\ndry_run: false\nuploaded file vim.spec to project P package K (revision 7)"
        );
        let (payload, text) = upload_file_result(
            "P",
            "K",
            "vim.spec",
            None,
            true,
            Some(vec!["[dry-run] would: upload file P/K/vim.spec".to_string()]),
        );
        assert_eq!(payload["rev"], Value::Null);
        assert_eq!(payload["dry_run"], true);
        assert_eq!(
            payload["plan"][0],
            "[dry-run] would: upload file P/K/vim.spec"
        );
        assert_eq!(
            text,
            "project: P package: K file: vim.spec\ndry_run: true\ndry-run: would upload file vim.spec to project P package K\n[dry-run] would: upload file P/K/vim.spec"
        );
    }

    #[test]
    fn create_project_parses_title_and_description() {
        let parsed = parse(&[
            "geectl",
            "remote",
            "create-project",
            "home:alice",
            "--title",
            "Alice",
            "--description",
            "Alice's project",
        ]);
        assert!(matches!(
            parsed,
            RemoteCommand::CreateProject {
                title,
                description,
                ..
            } if title == "Alice" && description == "Alice's project"
        ));
        assert!(Cli::try_parse_from(["geectl", "remote", "create-project", "home:alice"]).is_err());
        assert!(Cli::try_parse_from([
            "geectl",
            "remote",
            "create-project",
            "home:alice",
            "--title",
            "Alice"
        ])
        .is_err());
    }

    #[test]
    fn create_project_result_carries_title_in_both_formats() {
        let (payload, text) = create_project_result(
            "home:alice",
            "Alice",
            "A very long description",
            false,
            None,
        );
        assert_eq!(payload["project"], "home:alice");
        assert_eq!(payload["title"], "Alice");
        assert_eq!(payload["description"], "A very long description");
        assert_eq!(payload["dry_run"], false);
        assert!(!text.contains("A very long description"));
        assert_eq!(
            text,
            "project: home:alice\ndry_run: false\ncreated project home:alice (title: Alice)"
        );
        let (payload, text) = create_project_result(
            "home:alice",
            "Alice",
            "A very long description",
            true,
            Some(vec![
                "[dry-run] would: create project home:alice".to_string()
            ]),
        );
        assert_eq!(payload["dry_run"], true);
        assert_eq!(
            payload["plan"][0],
            "[dry-run] would: create project home:alice"
        );
        assert_eq!(
            text,
            "project: home:alice\ndry_run: true\ndry-run: would create project home:alice (title: Alice)\n[dry-run] would: create project home:alice"
        );
    }

    #[test]
    fn create_package_parses_title_description_and_options() {
        let parsed = parse(&[
            "geectl",
            "remote",
            "create-package",
            "P",
            "K",
            "--title",
            "Vi",
            "--description",
            "Vi IMproved",
        ]);
        assert!(matches!(
            parsed,
            RemoteCommand::CreatePackage {
                scmsync: None,
                force: false,
                ..
            }
        ));
        let parsed = parse(&[
            "geectl",
            "remote",
            "create-package",
            "P",
            "K",
            "--title",
            "Vi",
            "--description",
            "Vi IMproved",
            "--scmsync",
            "https://example.com/vim#main",
            "--force",
        ]);
        assert!(matches!(
            parsed,
            RemoteCommand::CreatePackage {
                scmsync: Some(scmsync),
                force: true,
                ..
            } if scmsync == "https://example.com/vim#main"
        ));
        assert!(Cli::try_parse_from(["geectl", "remote", "create-package", "P", "K"]).is_err());
    }

    #[test]
    fn create_package_result_carries_scmsync_in_both_formats() {
        let (payload, text) = create_package_result(
            "P",
            "K",
            "Vi",
            "A very long description",
            &CreatePackageOptions {
                scmsync: Some("https://example.com/vim#main".to_string()),
                force: true,
            },
            false,
            None,
        );
        assert_eq!(payload["project"], "P");
        assert_eq!(payload["package"], "K");
        assert_eq!(payload["title"], "Vi");
        assert_eq!(payload["description"], "A very long description");
        assert_eq!(payload["scmsync"], "https://example.com/vim#main");
        assert_eq!(payload["force"], true);
        assert_eq!(payload["dry_run"], false);
        assert!(!text.contains("A very long description"));
        assert!(!text.contains("force"));
        assert_eq!(
            text,
            "project: P package: K\ndry_run: false\ncreated package K in project P (title: Vi; scmsync: https://example.com/vim#main)"
        );
        let (payload, text) = create_package_result(
            "P",
            "K",
            "Vi",
            "A very long description",
            &CreatePackageOptions::default(),
            true,
            Some(vec!["[dry-run] would: create package P/K".to_string()]),
        );
        assert_eq!(payload["scmsync"], Value::Null);
        assert_eq!(payload["force"], false);
        assert_eq!(payload["dry_run"], true);
        assert_eq!(payload["plan"][0], "[dry-run] would: create package P/K");
        assert_eq!(
            text,
            "project: P package: K\ndry_run: true\ndry-run: would create package K in project P (title: Vi)\n[dry-run] would: create package P/K"
        );
    }

    #[test]
    fn delete_package_parses_force() {
        let parsed = parse(&["geectl", "remote", "delete-package", "P", "K"]);
        assert!(matches!(
            parsed,
            RemoteCommand::DeletePackage { force: false, .. }
        ));
        let parsed = parse(&["geectl", "remote", "delete-package", "P", "K", "--force"]);
        assert!(matches!(
            parsed,
            RemoteCommand::DeletePackage { force: true, .. }
        ));
    }

    #[test]
    fn delete_link_parses_project_and_package() {
        let parsed = parse(&["geectl", "remote", "delete-link", "P", "K"]);
        assert!(matches!(
            parsed,
            RemoteCommand::DeleteLink { project, package }
                if project == "P" && package == "K"
        ));
    }

    #[test]
    fn delete_link_result_carries_coordinates_in_both_formats() {
        let (payload, text) = delete_link_result(
            "P",
            "K",
            true,
            Some(vec!["[dry-run] would: delete link of P/K".to_string()]),
        );
        assert_eq!(payload["project"], "P");
        assert_eq!(payload["package"], "K");
        assert_eq!(payload["dry_run"], true);
        assert_eq!(payload["plan"][0], "[dry-run] would: delete link of P/K");
        assert_eq!(
            text,
            "project: P package: K\ndry_run: true\ndry-run: would delete link of package K from project P\n[dry-run] would: delete link of P/K"
        );
        let (payload, text) = delete_link_result("P", "K", false, None);
        assert_eq!(payload["dry_run"], false);
        assert_eq!(
            text,
            "project: P package: K\ndry_run: false\ndeleted link of package K from project P"
        );
    }

    #[test]
    fn set_project_meta_parses_field_flags() {
        let parsed = parse(&[
            "geectl",
            "remote",
            "set-project-meta",
            "home:alice",
            "--title",
            "Alice",
            "--url",
            "https://example.org",
        ]);
        assert!(matches!(
            parsed,
            RemoteCommand::SetProjectMeta {
                project,
                title: Some(title),
                description: None,
                url: Some(url),
                scmsync: None,
                devel_project: None,
            } if project == "home:alice" && title == "Alice" && url == "https://example.org"
        ));
    }

    #[test]
    fn set_package_meta_parses_field_flags() {
        let parsed = parse(&[
            "geectl",
            "remote",
            "set-package-meta",
            "P",
            "K",
            "--releasename",
            "vim",
            "--devel-project",
            "devel:languages:vim",
        ]);
        assert!(matches!(
            parsed,
            RemoteCommand::SetPackageMeta {
                project,
                package,
                releasename: Some(releasename),
                devel_project: Some(devel),
                ..
            } if project == "P" && package == "K" && releasename == "vim" && devel == "devel:languages:vim"
        ));
    }

    #[test]
    fn update_package_meta_from_spec_parses_optional_spec() {
        let parsed = parse(&[
            "geectl",
            "remote",
            "update-package-meta-from-spec",
            "P",
            "K",
            "--spec",
            "vim.spec",
        ]);
        assert!(matches!(
            parsed,
            RemoteCommand::UpdatePackageMetaFromSpec {
                project,
                package,
                spec: Some(spec),
            } if project == "P" && package == "K" && spec == "vim.spec"
        ));
        let parsed = parse(&[
            "geectl",
            "remote",
            "update-package-meta-from-spec",
            "P",
            "K",
        ]);
        assert!(matches!(
            parsed,
            RemoteCommand::UpdatePackageMetaFromSpec { spec: None, .. }
        ));
    }

    #[test]
    fn merge_project_meta_keeps_unmentioned_fields() {
        let current = sample_project_meta();
        let flags = ProjectMetaFlags {
            title: Some("New title".to_string()),
            description: None,
            url: None,
            scmsync: Some("https://github.com/openSUSE/factory.git".to_string()),
            devel_project: None,
        };
        let merged = merge_project_meta(&current, &flags);
        assert_eq!(merged.title, Some("New title".to_string()));
        assert_eq!(
            merged.description,
            Some("The rolling distribution".to_string())
        );
        assert_eq!(merged.url, Some("https://www.opensuse.org".to_string()));
        assert_eq!(
            merged.scmsync,
            Some("https://github.com/openSUSE/factory.git".to_string())
        );
        assert_eq!(merged.devel, None);
        assert_eq!(merged.persons, current.persons);
        assert_eq!(merged.repositories, current.repositories);
    }

    #[test]
    fn merge_package_meta_defaults_devel_package_to_the_package_name() {
        let current = PackageMeta {
            name: "vim".to_string(),
            project: "P".to_string(),
            title: None,
            description: None,
            url: None,
            scmsync: None,
            releasename: None,
            devel: None,
            persons: vec![],
            groups: vec![],
        };
        let flags = PackageMetaFlags {
            title: None,
            description: None,
            url: None,
            scmsync: None,
            releasename: None,
            devel_project: Some("devel:languages:vim".to_string()),
            devel_package: None,
        };
        let merged = merge_package_meta(&current, &flags).unwrap();
        assert_eq!(
            merged.devel,
            Some(("devel:languages:vim".to_string(), "vim".to_string()))
        );
    }

    #[test]
    fn merge_package_meta_devel_package_alone_needs_a_devel_project() {
        let current = PackageMeta {
            name: "vim".to_string(),
            project: "P".to_string(),
            title: None,
            description: None,
            url: None,
            scmsync: None,
            releasename: None,
            devel: None,
            persons: vec![],
            groups: vec![],
        };
        let flags = PackageMetaFlags {
            title: None,
            description: None,
            url: None,
            scmsync: None,
            releasename: None,
            devel_project: None,
            devel_package: Some("other".to_string()),
        };
        // Both modes reject the pair: live reads the current meta and
        // finds no devel project; dry-run never reads the current meta,
        // so the pair is an error there too.
        let err = merge_package_meta(&current, &flags).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        // With a devel project already set on the package, the pair
        // resolves against it.
        let current = PackageMeta {
            devel: Some(("devel:languages:vim".to_string(), "vim".to_string())),
            ..current
        };
        let merged = merge_package_meta(&current, &flags).unwrap();
        assert_eq!(
            merged.devel,
            Some(("devel:languages:vim".to_string(), "other".to_string()))
        );
    }

    #[test]
    fn meta_fields_rejects_an_empty_flag_set() {
        let project_flags = ProjectMetaFlags {
            title: None,
            description: None,
            url: None,
            scmsync: None,
            devel_project: None,
        };
        assert!(project_meta_fields(&project_flags).is_err());
        let package_flags = PackageMetaFlags {
            title: None,
            description: None,
            url: None,
            scmsync: None,
            releasename: None,
            devel_project: None,
            devel_package: None,
        };
        assert!(package_meta_fields(&package_flags).is_err());
    }

    #[test]
    fn set_project_meta_result_carries_meta_and_fields_in_both_formats() {
        let meta = sample_project_meta();
        let fields = vec!["title: Factory".to_string()];
        let (payload, text) = set_project_meta_result(&meta, &fields, true, None);
        assert_eq!(payload["name"], "openSUSE:Factory");
        assert_eq!(payload["title"], "Factory");
        assert_eq!(payload["dry_run"], true);
        // The dry-run payload drops null and empty-array keys.
        assert!(payload.get("scmsync").is_none());
        assert!(payload.get("devel").is_none());
        assert!(payload.get("groups").is_none());
        let (payload, live_text) = set_project_meta_result(&meta, &fields, false, None);
        assert_eq!(payload["scmsync"], serde_json::Value::Null);
        assert_eq!(payload["groups"], json!([]));
        assert_eq!(
            live_text,
            "project: openSUSE:Factory\ndry_run: false\nset project meta of openSUSE:Factory (title: Factory)"
        );
        assert_eq!(
            text,
            "project: openSUSE:Factory\ndry_run: true\ndry-run: would set project meta of openSUSE:Factory (title: Factory)"
        );
    }

    #[test]
    fn set_package_meta_result_carries_meta_and_fields_in_both_formats() {
        let meta = PackageMeta {
            name: "vim".to_string(),
            project: "openSUSE:Factory".to_string(),
            title: Some("Vi Improved".to_string()),
            description: None,
            url: None,
            scmsync: None,
            releasename: None,
            devel: None,
            persons: vec![],
            groups: vec![],
        };
        let fields = vec!["title: Vi Improved".to_string()];
        let (payload, text) = set_package_meta_result(&meta, &fields, true, None);
        assert_eq!(payload["project"], "openSUSE:Factory");
        assert_eq!(payload["name"], "vim");
        assert_eq!(payload["title"], "Vi Improved");
        assert_eq!(payload["dry_run"], true);
        // The dry-run payload drops null and empty-array keys.
        assert!(payload.get("scmsync").is_none());
        assert!(payload.get("devel").is_none());
        assert!(payload.get("groups").is_none());
        assert_eq!(
            text,
            "project: openSUSE:Factory package: vim\ndry_run: true\ndry-run: would set package meta of openSUSE:Factory/vim (title: Vi Improved)"
        );
        let (payload, live_text) = set_package_meta_result(&meta, &fields, false, None);
        assert_eq!(payload["dry_run"], false);
        assert_eq!(payload["scmsync"], serde_json::Value::Null);
        assert_eq!(payload["groups"], json!([]));
        assert_eq!(
            live_text,
            "project: openSUSE:Factory package: vim\ndry_run: false\nset package meta of openSUSE:Factory/vim (title: Vi Improved)"
        );
    }

    #[test]
    fn update_package_meta_from_spec_result_states_the_update_in_both_formats() {
        let (payload, text) = update_package_meta_from_spec_result(
            "P",
            "vim",
            &Some("vim.spec".to_string()),
            true,
            None,
        );
        assert_eq!(payload["project"], "P");
        assert_eq!(payload["package"], "vim");
        assert_eq!(payload["spec"], "vim.spec");
        assert_eq!(payload["dry_run"], true);
        assert_eq!(
            text,
            "project: P package: vim\ndry_run: true\ndry-run: would update package meta of P/vim from spec vim.spec"
        );
        let (payload, live_text) =
            update_package_meta_from_spec_result("P", "vim", &None, false, None);
        assert_eq!(payload["dry_run"], false);
        assert_eq!(payload["spec"], serde_json::Value::Null);
        assert_eq!(
            live_text,
            "project: P package: vim\ndry_run: false\nupdated package meta of P/vim from the spec file"
        );
    }

    #[test]
    fn delete_package_result_carries_force_in_both_formats() {
        let (payload, text) = delete_package_result("P", "K", true, false, None);
        assert_eq!(payload["project"], "P");
        assert_eq!(payload["package"], "K");
        assert_eq!(payload["force"], true);
        assert_eq!(payload["dry_run"], false);
        assert_eq!(
            text,
            "project: P package: K\ndry_run: false\ndeleted package K from project P (force)"
        );
        let (payload, text) = delete_package_result(
            "P",
            "K",
            false,
            true,
            Some(vec!["[dry-run] would: delete package P/K".to_string()]),
        );
        assert_eq!(payload["force"], false);
        assert_eq!(payload["dry_run"], true);
        assert_eq!(payload["plan"][0], "[dry-run] would: delete package P/K");
        assert_eq!(
            text,
            "project: P package: K\ndry_run: true\ndry-run: would delete package K from project P\n[dry-run] would: delete package P/K"
        );
    }

    #[test]
    fn delete_project_parses_options() {
        let parsed = parse(&["geectl", "remote", "delete-project", "P"]);
        assert!(matches!(
            parsed,
            RemoteCommand::DeleteProject {
                recursive: false,
                force: false,
                comment: None,
                ..
            }
        ));
        let parsed = parse(&[
            "geectl",
            "remote",
            "delete-project",
            "P",
            "--recursive",
            "--force",
            "--comment",
            "cleanup",
        ]);
        assert!(matches!(
            parsed,
            RemoteCommand::DeleteProject {
                recursive: true,
                force: true,
                comment: Some(comment),
                ..
            } if comment == "cleanup"
        ));
    }

    #[test]
    fn delete_project_result_carries_options_in_both_formats() {
        let (payload, text) = delete_project_result(
            "P",
            &DeleteProjectOptions {
                recursive: true,
                force: true,
                comment: Some("cleanup".to_string()),
            },
            false,
            None,
        );
        assert_eq!(payload["project"], "P");
        assert_eq!(payload["recursive"], true);
        assert_eq!(payload["force"], true);
        assert_eq!(payload["comment"], "cleanup");
        assert_eq!(payload["dry_run"], false);
        assert!(!text.contains("cleanup"));
        assert_eq!(
            text,
            "project: P\ndry_run: false\ndeleted project P (recursive, force)"
        );
        let (payload, text) = delete_project_result(
            "P",
            &DeleteProjectOptions::default(),
            true,
            Some(vec!["[dry-run] would: delete project P".to_string()]),
        );
        assert_eq!(payload["recursive"], false);
        assert_eq!(payload["force"], false);
        assert_eq!(payload["comment"], Value::Null);
        assert_eq!(payload["dry_run"], true);
        assert_eq!(payload["plan"][0], "[dry-run] would: delete project P");
        assert_eq!(
            text,
            "project: P\ndry_run: true\ndry-run: would delete project P\n[dry-run] would: delete project P"
        );
    }

    #[test]
    fn undelete_package_parses_positionals() {
        assert!(matches!(
            parse(&["geectl", "remote", "undelete-package", "P", "K"]),
            RemoteCommand::UndeletePackage { .. }
        ));
    }

    #[test]
    fn undelete_package_result_carries_coordinates_in_both_formats() {
        let (payload, text) = undelete_package_result("P", "K", false, None);
        assert_eq!(payload["project"], "P");
        assert_eq!(payload["package"], "K");
        assert_eq!(payload["dry_run"], false);
        assert_eq!(
            text,
            "project: P package: K\ndry_run: false\nrestored deleted package K in project P"
        );
        let (payload, text) = undelete_package_result(
            "P",
            "K",
            true,
            Some(vec![
                "[dry-run] would: restore deleted package P/K".to_string()
            ]),
        );
        assert_eq!(payload["dry_run"], true);
        assert_eq!(
            payload["plan"][0],
            "[dry-run] would: restore deleted package P/K"
        );
        assert_eq!(
            text,
            "project: P package: K\ndry_run: true\ndry-run: would restore deleted package K in project P\n[dry-run] would: restore deleted package P/K"
        );
    }

    #[test]
    fn undelete_project_parses_positional() {
        assert!(matches!(
            parse(&["geectl", "remote", "undelete-project", "P"]),
            RemoteCommand::UndeleteProject { .. }
        ));
    }

    #[test]
    fn undelete_project_result_carries_project_in_both_formats() {
        let (payload, text) = undelete_project_result("P", false, None);
        assert_eq!(payload["project"], "P");
        assert_eq!(payload["dry_run"], false);
        assert_eq!(
            text,
            "project: P\ndry_run: false\nrestored deleted project P"
        );
        let (payload, text) = undelete_project_result(
            "P",
            true,
            Some(vec![
                "[dry-run] would: restore deleted project P".to_string()
            ]),
        );
        assert_eq!(payload["dry_run"], true);
        assert_eq!(
            payload["plan"][0],
            "[dry-run] would: restore deleted project P"
        );
        assert_eq!(
            text,
            "project: P\ndry_run: true\ndry-run: would restore deleted project P\n[dry-run] would: restore deleted project P"
        );
    }

    #[test]
    fn aggregate_package_parses_source_target_and_options() {
        let parsed = parse(&[
            "geectl",
            "remote",
            "aggregate-package",
            "SP",
            "SK",
            "TP",
            "TK",
            "--repository-map",
            "standard=standard",
            "--repository-map",
            "ports=ports",
            "--no-sources",
            "--disable-publish",
        ]);
        assert!(matches!(
            parsed,
            RemoteCommand::AggregatePackage {
                repository_map,
                no_sources: true,
                disable_publish: true,
                ..
            } if repository_map == vec!["standard=standard", "ports=ports"]
        ));
        assert!(matches!(
            parse(&["geectl", "remote", "aggregate-package", "SP", "SK", "TP", "TK"]),
            RemoteCommand::AggregatePackage {
                repository_map,
                no_sources: false,
                disable_publish: false,
                ..
            } if repository_map.is_empty()
        ));
    }

    #[test]
    fn repository_map_splits_on_the_first_equals() {
        assert_eq!(
            parse_repository_map(&["standard=standard".to_string()]).unwrap(),
            vec![("standard".to_string(), "standard".to_string())]
        );
        assert!(parse_repository_map(&["no-equals".to_string()]).is_err());
        assert!(parse_repository_map(&["=standard".to_string()]).is_err());
        assert!(parse_repository_map(&["standard=".to_string()]).is_err());
    }

    #[test]
    fn aggregate_package_result_carries_options_in_both_formats() {
        let options = AggregateOptions {
            repository_map: vec![
                ("standard".to_string(), "standard".to_string()),
                ("ports".to_string(), "ports".to_string()),
            ],
            no_sources: true,
            disable_publish: true,
        };
        let (payload, text) =
            aggregate_package_result("SP", "SK", "TP", "TK", &options, false, None);
        assert_eq!(payload["source_project"], "SP");
        assert_eq!(payload["source_package"], "SK");
        assert_eq!(payload["target_project"], "TP");
        assert_eq!(payload["target_package"], "TK");
        assert_eq!(payload["repository_map"][0]["source"], "standard");
        assert_eq!(payload["repository_map"][1]["target"], "ports");
        assert_eq!(payload["no_sources"], true);
        assert_eq!(payload["disable_publish"], true);
        assert_eq!(payload["dry_run"], false);
        assert_eq!(
            text,
            "source: SP/SK target: TP/TK\ndry_run: false\naggregated package SP/SK into TP/TK (no sources, publish disabled, 2 repository mappings)"
        );
        let (payload, text) = aggregate_package_result(
            "SP",
            "SK",
            "TP",
            "TK",
            &AggregateOptions::default(),
            true,
            Some(vec![
                "[dry-run] would: aggregate package SP/SK into TP/TK".to_string()
            ]),
        );
        assert_eq!(payload["repository_map"], json!([]));
        assert_eq!(payload["no_sources"], false);
        assert_eq!(payload["disable_publish"], false);
        assert_eq!(payload["dry_run"], true);
        assert_eq!(
            payload["plan"][0],
            "[dry-run] would: aggregate package SP/SK into TP/TK"
        );
        assert_eq!(
            text,
            "source: SP/SK target: TP/TK\ndry_run: true\ndry-run: would aggregate package SP/SK into TP/TK\n[dry-run] would: aggregate package SP/SK into TP/TK"
        );
    }

    #[test]
    fn release_package_parses_all_options() {
        let parsed = parse(&["geectl", "remote", "release-package", "P", "K"]);
        assert!(matches!(
            parsed,
            RemoteCommand::ReleasePackage {
                repository: None,
                arch: None,
                target_project: None,
                target_repository: None,
                set_release: None,
                no_delay: false,
                ..
            }
        ));
        let parsed = parse(&[
            "geectl",
            "remote",
            "release-package",
            "P",
            "K",
            "--repository",
            "standard",
            "--arch",
            "x86_64",
            "--target-project",
            "TP",
            "--target-repository",
            "TR",
            "--set-release",
            "1.0",
            "--no-delay",
        ]);
        assert!(matches!(
            parsed,
            RemoteCommand::ReleasePackage {
                repository: Some(repository),
                arch: Some(arch),
                target_project: Some(target_project),
                target_repository: Some(target_repository),
                set_release: Some(set_release),
                no_delay: true,
                ..
            } if repository == "standard"
                && arch == "x86_64"
                && target_project == "TP"
                && target_repository == "TR"
                && set_release == "1.0"
        ));
    }

    #[test]
    fn release_package_result_carries_options_in_both_formats() {
        let options = ReleaseOptions {
            repository: Some("standard".to_string()),
            architecture: Some("x86_64".to_string()),
            target_project: Some("TP".to_string()),
            target_repository: Some("TR".to_string()),
            set_release: Some("1.0".to_string()),
            no_delay: true,
        };
        let (payload, text) = release_package_result("P", "K", &options, false, None);
        assert_eq!(payload["project"], "P");
        assert_eq!(payload["package"], "K");
        assert_eq!(payload["repository"], "standard");
        assert_eq!(payload["architecture"], "x86_64");
        assert_eq!(payload["target_project"], "TP");
        assert_eq!(payload["target_repository"], "TR");
        assert_eq!(payload["set_release"], "1.0");
        assert_eq!(payload["no_delay"], true);
        assert_eq!(payload["dry_run"], false);
        assert_eq!(
            text,
            "project: P package: K\ndry_run: false\nreleased package K in project P (repository standard, architecture x86_64, to TP/TR, release tag 1.0, no delay)"
        );
        let (payload, text) = release_package_result(
            "P",
            "K",
            &ReleaseOptions::default(),
            true,
            Some(vec!["[dry-run] would: release package P/K".to_string()]),
        );
        assert_eq!(payload["repository"], Value::Null);
        assert_eq!(payload["architecture"], Value::Null);
        assert_eq!(payload["target_project"], Value::Null);
        assert_eq!(payload["target_repository"], Value::Null);
        assert_eq!(payload["set_release"], Value::Null);
        assert_eq!(payload["no_delay"], false);
        assert_eq!(payload["dry_run"], true);
        assert_eq!(payload["plan"][0], "[dry-run] would: release package P/K");
        assert_eq!(
            text,
            "project: P package: K\ndry_run: true\ndry-run: would release package K in project P\n[dry-run] would: release package P/K"
        );
    }

    #[test]
    fn release_project_parses_all_options() {
        let parsed = parse(&["geectl", "remote", "release-project", "P"]);
        assert!(matches!(
            parsed,
            RemoteCommand::ReleaseProject {
                repository: None,
                arch: None,
                target_project: None,
                target_repository: None,
                set_release: None,
                no_delay: false,
                ..
            }
        ));
        let parsed = parse(&[
            "geectl",
            "remote",
            "release-project",
            "P",
            "--repository",
            "standard",
            "--arch",
            "x86_64",
            "--target-project",
            "TP",
            "--target-repository",
            "TR",
            "--set-release",
            "1.0",
            "--no-delay",
        ]);
        assert!(matches!(
            parsed,
            RemoteCommand::ReleaseProject {
                repository: Some(repository),
                arch: Some(arch),
                target_project: Some(target_project),
                target_repository: Some(target_repository),
                set_release: Some(set_release),
                no_delay: true,
                ..
            } if repository == "standard"
                && arch == "x86_64"
                && target_project == "TP"
                && target_repository == "TR"
                && set_release == "1.0"
        ));
    }

    #[test]
    fn release_project_result_carries_options_in_both_formats() {
        let options = ReleaseOptions {
            repository: Some("standard".to_string()),
            architecture: Some("x86_64".to_string()),
            target_project: Some("TP".to_string()),
            target_repository: Some("TR".to_string()),
            set_release: Some("1.0".to_string()),
            no_delay: true,
        };
        let (payload, text) = release_project_result("P", &options, false, None);
        assert_eq!(payload["project"], "P");
        assert_eq!(payload["repository"], "standard");
        assert_eq!(payload["architecture"], "x86_64");
        assert_eq!(payload["target_project"], "TP");
        assert_eq!(payload["target_repository"], "TR");
        assert_eq!(payload["set_release"], "1.0");
        assert_eq!(payload["no_delay"], true);
        assert_eq!(payload["dry_run"], false);
        assert_eq!(
            text,
            "project: P\ndry_run: false\nreleased project P (repository standard, architecture x86_64, to TP/TR, release tag 1.0, no delay)"
        );
        let (payload, text) = release_project_result(
            "P",
            &ReleaseOptions::default(),
            true,
            Some(vec!["[dry-run] would: release project P".to_string()]),
        );
        assert_eq!(payload["repository"], Value::Null);
        assert_eq!(payload["no_delay"], false);
        assert_eq!(payload["dry_run"], true);
        assert_eq!(payload["plan"][0], "[dry-run] would: release project P");
        assert_eq!(
            text,
            "project: P\ndry_run: true\ndry-run: would release project P\n[dry-run] would: release project P"
        );
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
    fn list_staging_projects_parses_project_and_option_flags() {
        let command = parse(&[
            "geectl",
            "remote",
            "list-staging-projects",
            "openSUSE:Factory",
            "--no-requests",
        ]);
        assert!(matches!(
            command,
            RemoteCommand::ListStagingProjects {
                project,
                no_requests: true,
                no_status: false,
            } if project == "openSUSE:Factory"
        ));
    }

    #[test]
    fn staging_project_parses_project_name_and_option_flags() {
        let command = parse(&[
            "geectl",
            "remote",
            "staging-project",
            "openSUSE:Factory",
            "A",
            "--no-status",
        ]);
        assert!(matches!(
            command,
            RemoteCommand::StagingProject {
                project,
                name,
                no_requests: false,
                no_status: true,
            } if project == "openSUSE:Factory" && name == "A"
        ));
    }

    #[test]
    fn search_projects_by_person_parses_login_positional() {
        let command = parse(&["geectl", "remote", "search-projects-by-person", "alice"]);
        assert!(matches!(
            command,
            RemoteCommand::SearchProjectsByPerson { userid } if userid == "alice"
        ));
    }

    #[test]
    fn count_scmsync_projects_parses_url_prefix_positional() {
        let command = parse(&[
            "geectl",
            "remote",
            "count-scmsync-projects",
            "https://github.com/openSUSE/",
        ]);
        assert!(matches!(
            command,
            RemoteCommand::CountScmsyncProjects { url_prefix }
                if url_prefix == "https://github.com/openSUSE/"
        ));
    }

    #[test]
    fn staging_project_json_and_text_carry_state_and_requests() {
        let project = StagingProject {
            name: "openSUSE:Factory:Staging:A".to_string(),
            state: Some("acceptable".to_string()),
            staged_requests: Some(vec![StagedRequest {
                id: 123456,
                request_type: "submit".to_string(),
                creator: "alice".to_string(),
                state: "new".to_string(),
                package: Some("vim".to_string()),
                superseded_by: None,
            }]),
            untracked_requests: Some(vec![]),
            obsolete_requests: None,
        };
        let payload = staging_project_json(&project);
        assert_eq!(payload["name"], "openSUSE:Factory:Staging:A");
        assert_eq!(payload["state"], "acceptable");
        assert_eq!(payload["staged_requests"][0]["id"], 123456);
        assert_eq!(payload["untracked_requests"], json!([]));
        assert_eq!(payload["obsolete_requests"], serde_json::Value::Null);
        let text = render_staging_projects(std::slice::from_ref(&project));
        for field in [
            "openSUSE:Factory:Staging:A",
            "acceptable",
            "123456",
            "submit",
            "alice",
            "vim",
        ] {
            assert!(text.contains(field), "staging text misses {field}:\n{text}");
        }
        assert!(text.contains("untracked: none"));
        assert!(!text.contains("obsolete"));
    }

    #[test]
    fn staging_projects_renders_empty_list() {
        assert_eq!(render_staging_projects(&[]), "no staging projects");
    }

    #[test]
    fn unknown_remote_command_fails_to_parse() {
        assert!(Cli::try_parse_from(["geectl", "remote", "frobnicate"]).is_err());
        assert!(Cli::try_parse_from(["geectl", "remote", "cat", "only-one"]).is_err());
    }

    #[test]
    fn batch4_leaves_missing_positionals_fail_with_exit_code_2() {
        // Usage errors exit 2. A semantic rejection (a flag pair the
        // local merge refuses) is a runtime InvalidInput, not a
        // usage error, so it is not pinned here.
        for args in [
            vec!["geectl", "remote", "delete-link", "P"],
            vec!["geectl", "remote", "search-projects-by-person"],
            vec!["geectl", "remote", "count-scmsync-projects"],
            vec!["geectl", "build", "rebuild-package", "P"],
            vec!["geectl", "build", "restart-build", "P"],
            vec!["geectl", "build", "abort-build", "P"],
            vec!["geectl", "remote", "list-staging-projects"],
            vec!["geectl", "remote", "staging-project", "P"],
            vec!["geectl", "remote", "set-project-meta"],
            vec!["geectl", "remote", "set-package-meta", "P"],
            vec!["geectl", "remote", "update-package-meta-from-spec", "P"],
        ] {
            let err = Cli::try_parse_from(args.clone()).unwrap_err();
            assert_eq!(
                err.exit_code(),
                2,
                "args {args:?} must fail as a usage error"
            );
        }
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
                rebuild: None,
                block: None,
                linkedbuild: None,
                downloads: vec![],
                release_targets: vec![],
                hostsystem: vec![],
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
            Some(RevisionSpec::Rev(12)),
            "Name: vim\n".to_string(),
        );
        assert_eq!(payload["rev"], 12);
        assert_eq!(text, "Name: vim\n");
        let (payload, text) = cat_result(
            "openSUSE:Factory",
            "vim",
            "vim.spec",
            Some(RevisionSpec::Md5(
                "0123456789abcdef0123456789abcdef".to_string(),
            )),
            "Name: vim\n".to_string(),
        );
        assert_eq!(payload["rev"], "0123456789abcdef0123456789abcdef");
        assert_eq!(text, "Name: vim\n");
    }

    #[test]
    fn download_result_carries_path_size_and_rev() {
        // The expected path strings come from the path's display:
        // a `/tmp/...` literal renders with backslashes on Windows.
        let path = Path::new("/tmp/vim-9.1.tar.gz");
        let (payload, text) =
            download_result("openSUSE:Factory", "vim", "vim-9.1.tar.gz", None, path, 6);
        assert_eq!(payload["project"], "openSUSE:Factory");
        assert_eq!(payload["package"], "vim");
        assert_eq!(payload["file"], "vim-9.1.tar.gz");
        assert_eq!(payload["rev"], Value::Null);
        assert_eq!(
            payload["path"].as_str().unwrap(),
            path.to_string_lossy().as_ref()
        );
        assert_eq!(payload["size_bytes"], 6);
        assert_eq!(
            text,
            format!(
                "project: openSUSE:Factory package: vim file: vim-9.1.tar.gz\ndownloaded vim-9.1.tar.gz (6 bytes) to {}",
                path.display()
            )
        );
        let (payload, text) = download_result(
            "openSUSE:Factory",
            "vim",
            "vim-9.1.tar.gz",
            Some(&RevisionSpec::Rev(2)),
            path,
            6,
        );
        assert_eq!(payload["rev"], "2");
        assert!(text.contains("downloaded vim-9.1.tar.gz (6 bytes)"));
        let md5 = RevisionSpec::parse("AABBCCDDAABBCCDDAABBCCDDAABBCCDD").unwrap();
        let (payload, _) = download_result(
            "openSUSE:Factory",
            "vim",
            "vim-9.1.tar.gz",
            Some(&md5),
            path,
            6,
        );
        assert_eq!(payload["rev"], "aabbccddaabbccddaabbccddaabbccdd");
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

    #[test]
    fn repo_add_result_line_carries_arch_and_paths() {
        let arch = vec!["x86_64".to_string(), "aarch64".to_string()];
        let paths = vec![("openSUSE:Factory".to_string(), "standard".to_string())];
        assert_eq!(
            repo_add_result_line("standard", "P", &arch, &paths, false, true),
            "dry-run: would add repository standard to project P (arch: x86_64, aarch64; paths: openSUSE:Factory/standard)"
        );
        assert_eq!(
            repo_add_result_line("standard", "P", &arch, &paths, false, false),
            "added repository standard to project P (arch: x86_64, aarch64; paths: openSUSE:Factory/standard)"
        );
    }

    #[test]
    fn repo_add_result_line_marks_publish_disabled_on_live_line_only() {
        let arch = vec!["x86_64".to_string()];
        let paths = vec![("openSUSE:Factory".to_string(), "standard".to_string())];
        assert_eq!(
            repo_add_result_line("standard", "P", &arch, &paths, true, false),
            "added repository standard to project P (arch: x86_64; paths: openSUSE:Factory/standard) (publish disabled)"
        );
        // The dry-run result line carries no suffix: the plan names it.
        assert_eq!(
            repo_add_result_line("standard", "P", &arch, &paths, true, true),
            "dry-run: would add repository standard to project P (arch: x86_64; paths: openSUSE:Factory/standard)"
        );
    }

    #[test]
    fn repo_add_result_line_omits_empty_segments() {
        let no_arch: Vec<String> = Vec::new();
        let paths = vec![("openSUSE:Factory".to_string(), "standard".to_string())];
        assert_eq!(
            repo_add_result_line("standard", "P", &no_arch, &paths, false, false),
            "added repository standard to project P (paths: openSUSE:Factory/standard)"
        );
        let arch = vec!["x86_64".to_string()];
        let no_paths: Vec<(String, String)> = Vec::new();
        assert_eq!(
            repo_add_result_line("standard", "P", &arch, &no_paths, false, false),
            "added repository standard to project P (arch: x86_64)"
        );
        assert_eq!(
            repo_add_result_line("standard", "P", &no_arch, &no_paths, false, false),
            "added repository standard to project P"
        );
        assert_eq!(
            repo_add_result_line("standard", "P", &no_arch, &no_paths, true, false),
            "added repository standard to project P (publish disabled)"
        );
    }

    /// The design doc's worked example for `list-files`, pinned
    /// literally: the `NAME SIZE MTIME` table under the coordinate
    /// header, `-` for an unknown size or mtime.
    #[test]
    fn list_files_text_is_the_design_table() {
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
        let text = with_header(
            "project: openSUSE:Factory package: vim",
            render_source_files(&files),
        );
        assert_eq!(
            text,
            "project: openSUSE:Factory package: vim\nNAME     SIZE MTIME\nvim.spec 1234 1700000000\nREADME   -    -"
        );
    }

    #[test]
    fn package_log_text_is_a_table() {
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
        let text = with_header(
            "project: openSUSE:Factory package: vim",
            render_history(&entries),
        );
        let mut lines = text.lines();
        assert_eq!(
            lines.next().unwrap(),
            "project: openSUSE:Factory package: vim"
        );
        assert_eq!(
            lines.next().unwrap(),
            "REV VREV VERSION TIME       USER  REQUEST"
        );
        assert_eq!(
            lines.next().unwrap(),
            "3   3    9.1     1700000000 alice 123456"
        );
        assert!(lines.next().is_none());
    }

    /// The agreement test: every id, name, and revision the JSON
    /// payload carries appears in the text rendering, except the
    /// documented omissions (md5, srcmd5, comments; descriptions
    /// only in search tables).
    #[test]
    fn text_carries_every_identifying_field() {
        let meta = sample_project_meta();
        let payload = project_meta_json(&meta);
        let text = render_project_meta(&meta);
        for field in [
            "openSUSE:Factory",
            "Factory",
            "The rolling distribution",
            "url: https://www.opensuse.org",
            "alice",
            "maintainer",
            "standard",
            "x86_64",
            "aarch64",
        ] {
            assert!(
                text.contains(field),
                "project meta text misses {field}:\n{text}"
            );
        }
        assert_eq!(payload["repositories"][0]["name"], "standard");

        let package_meta = PackageMeta {
            name: "vim".to_string(),
            project: "openSUSE:Factory".to_string(),
            title: Some("Vi IMproved".to_string()),
            description: Some("A very long description".to_string()),
            url: Some("https://www.vim.org".to_string()),
            scmsync: None,
            releasename: Some("vim".to_string()),
            devel: Some(("editors".to_string(), "vim".to_string())),
            persons: vec![NamedRole {
                name: "alice".to_string(),
                role: "maintainer".to_string(),
            }],
            groups: vec![],
        };
        let text = render_package_meta(&package_meta);
        for field in [
            "vim",
            "openSUSE:Factory",
            "Vi IMproved",
            "A very long description",
            "https://www.vim.org",
            "editors/vim",
            "alice (maintainer)",
        ] {
            assert!(
                text.contains(field),
                "package meta text misses {field}:\n{text}"
            );
        }

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
        let text = render_owners(&owners);
        for field in [
            "openSUSE:Factory",
            "vim",
            "alice (maintainer)",
            "editors (bugowner)",
        ] {
            assert!(text.contains(field), "owners text misses {field}:\n{text}");
        }
        assert_eq!(owner_json(&owners[0])["package"], "vim");

        let matches = vec![PackageMatch {
            name: "vim".to_string(),
            project: "openSUSE:Factory".to_string(),
            title: Some("Vi IMproved".to_string()),
            description: Some("A very long description".to_string()),
        }];
        let text = render_package_matches("vim", &matches);
        for field in ["vim", "openSUSE:Factory", "Vi IMproved"] {
            assert!(text.contains(field), "search text misses {field}:\n{text}");
        }
        assert!(!text.contains("A very long description"));
    }
}
