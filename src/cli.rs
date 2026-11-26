//! The clap CLI definition: the argument types for every command group.
//!
//! The shipped binary parses these types; the `geectl-gen` generator builds
//! the same `Command` from them, so the generated man pages and shell
//! completions can never drift from the CLI.

use crate::{
    cli_branch, cli_build, cli_changelog, cli_gitea, cli_local_build, cli_meta, cli_person,
    cli_prjconf, cli_pull_request, cli_remote, cli_request, cli_service, maintenance,
};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// The output format. JSON is the default; it is the only format an agent
/// should parse. `text` is a human-readable rendering of the same data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Json,
    Text,
}

impl Format {
    /// The config-file / environment spelling of this format
    /// (`"json"` / `"text"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Json => "json",
            Format::Text => "text",
        }
    }
}

// The binary name is pinned so --help reads the same on every
// platform: without it clap prints argv[0], `geectl.exe` on Windows.
#[derive(Debug, Parser)]
#[command(
    name = "geectl",
    bin_name = "geectl",
    version,
    about = "Geeko Controller — a CLI for openSUSE packaging workflows",
    long_about = None
)]
pub struct Cli {
    /// Output format for the result payload. Applies to every command.
    /// Default: the `GEECTL_FORMAT` environment variable, else the
    /// `format` key in the config file, else `json`.
    #[arg(long, global = true, value_enum)]
    pub format: Option<Format>,

    /// Enable the debug channel: human-readable diagnostic lines on
    /// stderr (HTTP attempts, retries, library decisions). Off by
    /// default; `GEECTL_DEBUG=1` enables it without the flag.
    #[arg(long, global = true)]
    pub debug: bool,

    /// Also print request and response bodies on the debug channel
    /// (capped at 2048 bytes, redacted, never headers). Implies
    /// `--debug`.
    #[arg(long, global = true)]
    pub debug_body: bool,

    /// Actually perform mutations. Without it, only the plan is
    /// printed: dry-run is the default for mutations.
    #[arg(long, global = true)]
    pub live: bool,

    /// Select the instance for this call, by its alias in the
    /// config file's `[instances]` table. The flag is authoritative:
    /// it beats the `GEECTL_OBS_URL` environment variable, the
    /// configured URL, and any working-copy state. For OBS, the
    /// flagged alias's URL replaces the configured URL and
    /// credentials resolve only from the flagged instance's own
    /// variables (`GEECTL_OBS_USERNAME_<ALIAS>` /
    /// `GEECTL_OBS_PASSWORD[_FILE]_<ALIAS>` /
    /// `GEECTL_OBS_APIKEY[_FILE]_<ALIAS>`); another instance's
    /// variables are never borrowed, and a missing credential for
    /// the flagged instance fails the call. For Gitea, the host is
    /// unaffected, but every Gitea call authenticates with the
    /// flagged alias's `GEECTL_GITEA_TOKEN[_FILE]_<ALIAS>` only.
    #[arg(long, global = true, value_name = "ALIAS")]
    pub instance: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print version information as the result payload.
    Version,
    /// Inspect or initialise the config file.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Remote reads and mutations: inspect what the
    /// distribution holds, and change it.
    Remote {
        #[command(subcommand)]
        command: cli_remote::RemoteCommand,
    },
    /// Read and act on OBS submit requests, and file new ones.
    Request {
        #[command(subcommand)]
        command: Box<cli_request::RequestCommand>,
    },
    /// Branch, copy, and link packages.
    Branch {
        #[command(subcommand)]
        // Boxed: the eleven branch subcommands make BranchCommand far
        // larger than the other variants (clippy::large_enum_variant).
        command: Box<cli_branch::BranchCommand>,
    },
    /// Build-status commands: results, logs, histories, binaries, buildconfig.
    Build {
        #[command(subcommand)]
        command: cli_build::BuildCommand,
    },
    /// Local builds in the podman build vertical: build a package,
    /// run its `_service` services, the precommit gate,
    /// `spec-cleaner`, wipe a build root, prune the cache, manage
    /// the daily periodic cleanup, enter the preserved build root,
    /// and read the log of the last local build.
    LocalBuild {
        #[command(subcommand)]
        command: cli_local_build::LocalBuildCommand,
    },
    /// Maintenance operations: maintained-instances listing and
    /// maintenance branching.
    Maintenance {
        #[command(subcommand)]
        command: maintenance::MaintenanceCommand,
    },
    /// Service reads and trigger: list a package's services, their
    /// run state, and trigger a server-side run.
    Service {
        #[command(subcommand)]
        command: cli_service::ServiceCommand,
    },
    /// Gitea operations: repository and branch reads and mutations
    /// on the configured instance.
    Gitea {
        #[command(subcommand)]
        command: cli_gitea::GiteaCommand,
    },
    /// Read and act on pull requests: OBS submit requests and
    /// Gitea pull requests as one surface.
    PullRequest {
        #[command(subcommand)]
        command: cli_pull_request::PullRequestCommand,
    },
    /// Search for a person: the OBS projects naming the login
    /// and the Gitea users matching the keyword, as one operation.
    Person {
        #[command(subcommand)]
        command: cli_person::PersonCommand,
    },
    /// Set the meta of an OBS project, an OBS package, or a Gitea
    /// repository as one operation: the target selects the backend.
    Meta {
        #[command(subcommand)]
        command: cli_meta::MetaCommand,
    },
    /// Validate and generate `.changes` entries in a package
    /// directory, pre-file.
    Changelog {
        #[command(subcommand)]
        command: cli_changelog::ChangelogCommand,
    },
    /// Check out a package into a git working copy, under the
    /// current directory.
    Checkout {
        /// The OBS project.
        project: String,
        /// The OBS package.
        package: String,
        /// The revision to check out (a number or a source md5).
        /// The latest when omitted.
        #[arg(long)]
        rev: Option<String>,
    },
    /// Update the working copy in the current directory to the
    /// latest base.
    Update {
        /// The revision to update to (a number or a source md5).
        /// The latest when omitted.
        #[arg(long)]
        rev: Option<String>,
    },
    /// Show the working copy's state against the base.
    Status,
    /// Show the diff of the worktree and the index against the base,
    /// plus the link diff on a linked checkout.
    Diff,
    /// Stage files for commit.
    Add {
        /// The files to stage, relative to the checkout root.
        #[arg(required = true)]
        files: Vec<String>,
    },
    /// Remove files from the working copy.
    Rm {
        /// The files to remove, relative to the checkout root.
        #[arg(required = true)]
        files: Vec<String>,
    },
    /// Stage new files and worktree deletions.
    Addremove,
    /// Commit the staged changes.
    Commit {
        /// The commit message.
        #[arg(long, short)]
        message: String,
        /// Skip the service gate (source services and
        /// `pre_checkin.sh`).
        #[arg(long)]
        noservice: bool,
    },
    /// Pull a linked package to the link target's latest revision.
    Pull,
    /// Stage the resolved files and continue a stopped rebase.
    Resolve,
    /// Read, validate and clean a project's prjconf.
    Prjconf {
        #[command(subcommand)]
        command: cli_prjconf::PrjconfCommand,
    },
}

/// The `config` subcommands.
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Show the effective config: endpoint URLs with their source,
    /// and credential status (never a secret value).
    Show,
    /// Write the config template (endpoints only). Refuses if the
    /// file already exists.
    Init,
    /// Import one instance mapping from an oscrc file (one-time
    /// migration). Reads the oscrc file once, on explicit request —
    /// geectl never reads it at runtime — and writes only the alias
    /// to apiurl mapping into the config `[instances]` table. The
    /// password is never stored and never printed: set it with the
    /// printed `GEECTL_OBS_PASSWORD_<ALIAS>` variable yourself. The
    /// username is not written into the config either: it stays in
    /// the environment as `GEECTL_OBS_USERNAME_<ALIAS>`.
    /// Dry-run is the default (only the plan is printed); `--live`
    /// writes the config.
    ImportOscrc {
        /// The oscrc file to read (default
        /// `$XDG_CONFIG_HOME/osc/oscrc`, else `~/.config/osc/oscrc`).
        #[arg(long)]
        oscrc: Option<PathBuf>,
        /// The apiurl section to import. Required when the file holds
        /// more than one section with a username; the only such
        /// section otherwise.
        #[arg(long)]
        apiurl: Option<String>,
        /// The instance alias to write. Defaults to the apiurl host
        /// with non-alphanumerics as `_`.
        #[arg(long)]
        alias: Option<String>,
        /// Replace the alias when it already names another URL.
        #[arg(long)]
        force: bool,
    },
}

/// The clap `Command` for the shipped binary, shared with the `geectl-gen`
/// generator.
pub fn cli_command() -> clap::Command {
    Cli::command()
}
