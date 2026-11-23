//! `pull-request` — the unified pull-request command group: read OBS
//! submit requests and Gitea pull requests as one surface.
//!
//! Thin over `PullRequestClient` like the OBS groups are over
//! `ObsClient`: the flags mirror the library parameters, the JSON
//! payload carries the typed values, and the text renderer prints
//! the same values in a human-readable form.
//!
//! Reads are always live; the act leaves (`approve`,
//! `request-changes`, `comment`, `cancel-review-request`,
//! `merge`, `decline`, `withdraw`, `reopen`, `submit`) are
//! dry-run by default and mutate only with the global `--live`.
//! `submit` files from the working copy in the current directory:
//! an OBS submit request from an OBS-native checkout, a Gitea pull
//! request from a git-backed one — the checkout's backend decides,
//! no flag does.
//! A read or a live act on an arm needs that arm's
//! credentials — OBS credentials for `obs:` handles, the
//! Gitea Bearer token for `gitea:` handles — and fails closed
//! naming them when they are missing. `search` with no `--backend`
//! runs both arms, OBS first then Gitea, so it needs both sets of
//! credentials; scope with `--backend` when only one is configured. `--repo` needs
//! `--backend gitea`.
//!
//! In dry-run the OBS client builds without credentials (the plan
//! prints without them), like the request group; the Gitea client
//! still needs its credential to be built, so a dry-run Gitea act
//! without one fails closed naming it, like the gitea group. A
//! dry-run approve on OBS without OBS credentials also fails
//! closed: the plan names the acting user, and without credentials
//! there is no actor to name. A dry-run merge, decline, withdraw
//! or reopen on OBS prints without credentials: the plan names no
//! acting user.
//!
//! Coordinate header (design rule 1): every text rendering opens
//! with one header line echoing the query coordinates — the
//! reference for `show`, the filters for `search`. Tables stay as
//! designed.
//!
//! Documented omissions (not gaps): `show` text carries no title
//! or description when the PR has none (empty reads as absent),
//! and no source or target line when the endpoint is empty (a
//! delete request has no source); the JSON payload remains the
//! complete record in every case. The search table carries no
//! title (long free text) and no review detail — those are a
//! `show` call away.

use clap::{Args, Subcommand};
use geectl::auth::{Credentials, EnvGiteaAuth, EnvObsAuth};
use geectl::config::{self, DEFAULT_GITEA_URL};
use geectl::debug::Debug;
use geectl::error::{ErrorKind, GeectlError};
use geectl::gitea::{
    parse_gitea_repo, validate_gitea_base, GiteaClient, GiteaMergeMethod, PrCreateOptions,
};
use geectl::http::{Timeout, UreqClient};
use geectl::obs::{CveId, DryRun, ObsClient, ReviewTarget, SrPayload, OBS_API_BASE};
use geectl::pull_request::{
    PullRequest, PullRequestBackend, PullRequestClient, PullRequestEndpoint, PullRequestRef,
    PullRequestSearch, PullRequestStatus, PullRequestSummary, ReviewComment, ReviewOpinion,
    SubmitTarget, Submitted,
};
use geectl::working_copy::{self, Backend};
use serde::Serialize;
use serde_json::{json, Value};

/// The `pull-request` subcommands. Names are the library operation
/// names in kebab-case, grouped under the top-level noun.
#[derive(Debug, Subcommand)]
pub enum PullRequestCommand {
    /// Show one pull request: `obs:<id>` or
    /// `gitea:<owner>/<repo>#<number>`.
    Show {
        /// The pull-request handle: `obs:<id>` or
        /// `gitea:<owner>/<repo>#<number>`.
        reference: String,
    },
    /// Search pull requests: OBS submit requests and Gitea pull
    /// requests as one list, OBS first then Gitea. With no
    /// `--backend` both arms run; `--backend` scopes to one.
    Search(SearchArgs),
    /// Approve a pull request as the review asked of the target:
    /// with no target flag, as the review asked of the acting
    /// user (OBS) or the token owner (Gitea). Dry-run by default;
    /// mutates only with `--live`.
    Approve(ReviewActArgs),
    /// Ask for changes on a pull request, as `approve` but with
    /// the review declined instead of accepted. Dry-run by
    /// default; mutates only with `--live`.
    RequestChanges(ReviewActArgs),
    /// Comment on a pull request. Dry-run by default; mutates
    /// only with `--live`.
    Comment(CommentArgs),
    /// List the comments on a pull request, oldest first. A read,
    /// always live.
    Comments {
        /// The pull-request handle: `obs:<id>` or
        /// `gitea:<owner>/<repo>#<number>`.
        reference: String,
    },
    /// Cancel pending review requests on a pull request. Dry-run
    /// by default; mutates only with `--live`. On OBS the cancel
    /// fails closed: OBS has no request call that removes a
    /// reviewer.
    CancelReviewRequest(CancelArgs),
    /// Merge a pull request. On OBS this accepts the submit
    /// request; on Gitea it merges the pull request. Dry-run by
    /// default; mutates only with `--live`.
    Merge(MergeArgs),
    /// Decline a pull request. On OBS this changes the request
    /// state to declined; on Gitea it closes the pull request.
    /// Dry-run by default; mutates only with `--live`.
    Decline(LevelCommentArgs),
    /// Withdraw a pull request. On OBS this revokes the request;
    /// on Gitea it closes the pull request. Dry-run by default;
    /// mutates only with `--live`.
    Withdraw(LevelCommentArgs),
    /// Reopen a pull request. On OBS this changes a declined
    /// request back to `new`; on Gitea it reopens the closed pull
    /// request. Dry-run by default; mutates only with `--live`.
    Reopen {
        /// The pull-request handle: `obs:<id>` or
        /// `gitea:<owner>/<repo>#<number>`.
        reference: String,
    },
    /// File a submission from the working copy in the current
    /// directory: an OBS submit request from an OBS-native
    /// checkout, a Gitea pull request from a git-backed one. The
    /// checkout's backend decides; the other arm's flags are
    /// refused. Dry-run by default; mutates only with `--live`.
    Submit(SubmitArgs),
}

/// Arguments for `pull-request submit`: the OBS arm's flags and
/// the Gitea arm's flags in one set, each arm taking its own.
/// The required flags are enforced per arm, after the checkout's
/// backend is known: `--target-project`, `--description` and
/// `--changes-entry` on OBS; `--base` and `--title` on Gitea.
#[derive(Debug, Args)]
pub struct SubmitArgs {
    /// The project the request targets (the devel project, never
    /// openSUSE:Factory directly). OBS only; required there.
    #[arg(long)]
    target_project: Option<String>,
    /// The package in the target project. OBS only; the checkout
    /// package when unset.
    #[arg(long)]
    target_package: Option<String>,
    /// The request description: one headline, then `- ` lines for
    /// staging, needs, and rebuilds. OBS only; required there.
    /// Values starting with `-` are taken literally: a description
    /// may open with a `- ` bullet.
    #[arg(long, allow_hyphen_values = true)]
    description: Option<String>,
    /// The one `.changes` entry for this submission. It must name
    /// every CVE passed with `--cve`. OBS only; required there.
    /// Values starting with `-` are taken literally: a `.changes`
    /// entry starts with `- `.
    #[arg(long, allow_hyphen_values = true)]
    changes_entry: Option<String>,
    /// A CVE this submission claims to fix. Repeatable; each one
    /// must be named in `--changes-entry`. OBS only.
    #[arg(long)]
    cve: Vec<String>,
    /// Keep the source branch on accept instead of deleting it
    /// (cleanup is the default). OBS only.
    #[arg(long)]
    without_cleanup: bool,
    /// The repo to open the pull request in (`owner/repo`).
    /// Gitea only; the checkout's origin remote when unset.
    #[arg(long)]
    repo: Option<String>,
    /// The branch the change comes from. Gitea only; the
    /// checkout's current branch when unset.
    #[arg(long)]
    head: Option<String>,
    /// The branch the pull request targets. Gitea only; required
    /// there.
    #[arg(long)]
    base: Option<String>,
    /// The pull request title. Gitea only; required there. Values
    /// starting with `-` are taken literally.
    #[arg(long, allow_hyphen_values = true)]
    title: Option<String>,
    /// The pull request body. Gitea only. Values starting with `-`
    /// are taken literally: the body may open with a `- ` bullet.
    #[arg(long, allow_hyphen_values = true)]
    body: Option<String>,
    /// Open the pull request as a draft. Gitea only.
    #[arg(long)]
    draft: bool,
}

/// Arguments shared by `pull-request approve` and `pull-request
/// request-changes`.
#[derive(Debug, Args)]
pub struct ReviewActArgs {
    /// The pull-request handle: `obs:<id>` or
    /// `gitea:<owner>/<repo>#<number>`.
    reference: String,
    /// The message left with the review. Values starting with `-`
    /// are taken literally: a message may open with a `- ` bullet.
    #[arg(long, allow_hyphen_values = true)]
    message: Option<String>,
    /// Act on the review asked of this user (OBS only).
    #[arg(long)]
    user: Option<String>,
    /// Act on the review asked of this group (OBS only).
    #[arg(long)]
    group: Option<String>,
    /// Act on the review asked of this project (OBS only).
    #[arg(long)]
    project: Option<String>,
    /// Act on the review asked of this package (OBS only; needs
    /// its project).
    #[arg(long)]
    package: Option<String>,
}

/// Arguments for `pull-request comment`.
#[derive(Debug, Args)]
pub struct CommentArgs {
    /// The pull-request handle: `obs:<id>` or
    /// `gitea:<owner>/<repo>#<number>`.
    reference: String,
    /// The comment text. Values starting with `-` are taken
    /// literally: the body may open with a `- ` bullet.
    #[arg(long, allow_hyphen_values = true)]
    body: String,
    /// The comment being replied to, when this is a reply (OBS
    /// only; refused on a Gitea ref).
    #[arg(long)]
    parent: Option<String>,
}

/// Arguments for `pull-request cancel-review-request`.
#[derive(Debug, Args)]
pub struct CancelArgs {
    /// The pull-request handle: `obs:<id>` or
    /// `gitea:<owner>/<repo>#<number>`.
    reference: String,
    /// The reviewers to cancel, repeatable; `@team` names a team.
    #[arg(long)]
    reviewer: Vec<String>,
    /// Cancel every pending review request.
    #[arg(long)]
    all: bool,
    /// With `--all`, leave these reviewers out; `@team` names a
    /// team.
    #[arg(long)]
    exclude: Vec<String>,
}

/// Arguments for `pull-request merge`.
#[derive(Debug, Args)]
pub struct MergeArgs {
    /// The pull-request handle: `obs:<id>` or
    /// `gitea:<owner>/<repo>#<number>`.
    reference: String,
    /// The message left with the merge. Values starting with `-`
    /// are taken literally: a message may open with a `- ` bullet.
    #[arg(long, allow_hyphen_values = true)]
    message: Option<String>,
    /// The Gitea merge style (Gitea only; refused on an OBS ref).
    #[arg(long, value_parser = ["merge", "rebase", "rebase-merge", "squash", "fast-forward-only"])]
    method: Option<String>,
    /// Accept past open reviews (OBS only; refused on a Gitea ref).
    #[arg(long)]
    force: bool,
}

/// Arguments shared by `pull-request decline` and `pull-request
/// withdraw`.
#[derive(Debug, Args)]
pub struct LevelCommentArgs {
    /// The pull-request handle: `obs:<id>` or
    /// `gitea:<owner>/<repo>#<number>`.
    reference: String,
    /// The reason for the decline or withdrawal. Values starting
    /// with `-` are taken literally: a message may open with a
    /// `- ` bullet.
    #[arg(long, allow_hyphen_values = true)]
    message: Option<String>,
}

/// Arguments for `pull-request search`.
#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Only this backend (`obs` or `gitea`). With no backend both
    /// arms run, OBS first then Gitea.
    #[arg(long, value_parser = ["obs", "gitea"])]
    backend: Option<String>,
    /// Only pull requests in these unified states. With none given
    /// each arm applies its own default (new/review/declined on
    /// OBS, open on Gitea). OBS has no closed state: `--state
    /// closed` reads OBS declined requests, which report as
    /// `declined`.
    #[arg(long, value_parser = [
        "open", "merged", "declined", "withdrawn", "closed", "superseded", "deleted",
    ])]
    state: Vec<String>,
    /// Only pull requests by this author.
    #[arg(long)]
    creator: Option<String>,
    /// Only pull requests in this Gitea repo (`owner/repo`).
    /// Requires `--backend gitea`.
    #[arg(long)]
    repo: Option<String>,
}

pub fn run_pull_request(
    command: PullRequestCommand,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    match command {
        PullRequestCommand::Show { reference } => {
            // The handle is parsed before any credential
            // resolution, so an invalid handle fails as
            // InvalidInput whether or not credentials are
            // configured (the request group's search arm validates
            // its filters this way).
            let reference: PullRequestRef = reference.parse()?;
            match &reference {
                PullRequestRef::Obs(_) => with_obs(debug, true, |obs| {
                    show_with(&PullRequestClient::new(Some(obs), None), &reference)
                }),
                PullRequestRef::Gitea { .. } => with_gitea(debug, |gitea| {
                    show_with(&PullRequestClient::new(None, Some(gitea)), &reference)
                }),
            }
        }
        PullRequestCommand::Approve(args) => review_act_with(
            args,
            debug,
            live,
            "approve",
            "approved",
            |client, reference, target, message, dry_run| {
                client.approve(reference, target, message, dry_run)
            },
        ),
        PullRequestCommand::RequestChanges(args) => review_act_with(
            args,
            debug,
            live,
            "request changes on",
            "requested changes on",
            |client, reference, target, message, dry_run| {
                client.request_changes(reference, target, message, dry_run)
            },
        ),
        PullRequestCommand::Comment(args) => {
            let reference: PullRequestRef = args.reference.parse()?;
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            match &reference {
                PullRequestRef::Obs(_) => with_obs(debug, live, |obs| {
                    let client = PullRequestClient::new(Some(obs), None);
                    let comment =
                        client.comment(&reference, &args.body, args.parent.as_deref(), &dry_run)?;
                    Ok((
                        comment_payload(&reference, &comment, &dry_run)?,
                        comment_text(&reference, &comment, &args.body, &dry_run),
                    ))
                }),
                PullRequestRef::Gitea { .. } => with_gitea(debug, |gitea| {
                    let client = PullRequestClient::new(None, Some(gitea));
                    let comment =
                        client.comment(&reference, &args.body, args.parent.as_deref(), &dry_run)?;
                    Ok((
                        comment_payload(&reference, &comment, &dry_run)?,
                        comment_text(&reference, &comment, &args.body, &dry_run),
                    ))
                }),
            }
        }
        PullRequestCommand::Comments { reference } => {
            let reference: PullRequestRef = reference.parse()?;
            match &reference {
                PullRequestRef::Obs(_) => with_obs(debug, true, |obs| {
                    let client = PullRequestClient::new(Some(obs), None);
                    let comments = client.comments(&reference)?;
                    Ok((
                        comments_payload(&reference, &comments)?,
                        comments_text(&reference, &comments),
                    ))
                }),
                PullRequestRef::Gitea { .. } => with_gitea(debug, |gitea| {
                    let client = PullRequestClient::new(None, Some(gitea));
                    let comments = client.comments(&reference)?;
                    Ok((
                        comments_payload(&reference, &comments)?,
                        comments_text(&reference, &comments),
                    ))
                }),
            }
        }
        PullRequestCommand::CancelReviewRequest(args) => {
            let reference: PullRequestRef = args.reference.parse()?;
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            match &reference {
                PullRequestRef::Obs(_) => with_obs(debug, live, |obs| {
                    let client = PullRequestClient::new(Some(obs), None);
                    let cancelled = client.cancel_review_request(
                        &reference,
                        &args.reviewer,
                        args.all,
                        &args.exclude,
                        &dry_run,
                    )?;
                    Ok((
                        cancel_payload(&reference, &cancelled, &dry_run)?,
                        cancel_text(&reference, &args.reviewer, args.all, &cancelled, &dry_run),
                    ))
                }),
                PullRequestRef::Gitea { .. } => with_gitea(debug, |gitea| {
                    let client = PullRequestClient::new(None, Some(gitea));
                    let cancelled = client.cancel_review_request(
                        &reference,
                        &args.reviewer,
                        args.all,
                        &args.exclude,
                        &dry_run,
                    )?;
                    Ok((
                        cancel_payload(&reference, &cancelled, &dry_run)?,
                        cancel_text(&reference, &args.reviewer, args.all, &cancelled, &dry_run),
                    ))
                }),
            }
        }
        PullRequestCommand::Merge(args) => {
            let method = match &args.method {
                Some(method) => Some(method.parse::<GiteaMergeMethod>()?),
                None => None,
            };
            let message = args.message.as_deref();
            // Backend-only options are refused before any
            // credential resolution, so the refusal is reachable
            // without credentials — the library refuses again, as
            // enforcement. `review_target` validates the same way.
            let reference: PullRequestRef = args.reference.parse()?;
            match &reference {
                PullRequestRef::Obs(_) => {
                    if let Some(method) = method {
                        return Err(GeectlError::new(
                            ErrorKind::InvalidInput,
                            format!(
                                "--method {} does not apply to an OBS submit request: OBS has no merge style",
                                method.as_str()
                            ),
                        ));
                    }
                }
                PullRequestRef::Gitea { .. } => {
                    if args.force {
                        return Err(GeectlError::new(
                            ErrorKind::InvalidInput,
                            "--force does not apply to a Gitea pull request: the merge API takes no force flag",
                        ));
                    }
                }
            }
            level_act_with(
                args.reference,
                debug,
                live,
                "merge",
                "merged",
                message,
                |client, reference, dry_run| {
                    client.merge(reference, message, method, args.force, dry_run)
                },
            )
        }
        PullRequestCommand::Decline(args) => {
            let message = args.message.as_deref();
            level_act_with(
                args.reference,
                debug,
                live,
                "decline",
                "declined",
                message,
                |client, reference, dry_run| client.decline(reference, message, dry_run),
            )
        }
        PullRequestCommand::Withdraw(args) => {
            let message = args.message.as_deref();
            level_act_with(
                args.reference,
                debug,
                live,
                "withdraw",
                "withdrew",
                message,
                |client, reference, dry_run| client.withdraw(reference, message, dry_run),
            )
        }
        PullRequestCommand::Reopen { reference } => level_act_with(
            reference,
            debug,
            live,
            "reopen",
            "reopened",
            None,
            |client, reference, dry_run| client.reopen(reference, dry_run),
        ),
        PullRequestCommand::Submit(args) => submit(args, debug, live),
        PullRequestCommand::Search(args) => {
            // The filters are parsed and validated before any
            // credential resolution, like the handle above.
            let backend = match &args.backend {
                Some(backend) => Some(backend.parse::<PullRequestBackend>()?),
                None => None,
            };
            let mut states = Vec::new();
            for state in &args.state {
                states.push(state.parse::<PullRequestStatus>()?);
            }
            let repo = match &args.repo {
                Some(repo) => Some(parse_gitea_repo(repo)?),
                None => None,
            };
            let query = PullRequestSearch {
                backend,
                states,
                creator: args.creator.clone(),
                repo,
            };
            query.validate()?;
            let backend_values: Vec<String> = args.backend.clone().into_iter().collect();
            let creator_values: Vec<String> = args.creator.clone().into_iter().collect();
            let repo_values: Vec<String> = args.repo.clone().into_iter().collect();
            let coordinates = search_coordinates(&[
                ("backend", &backend_values),
                ("state", &args.state),
                ("creator", &creator_values),
                ("repo", &repo_values),
            ]);
            match query.backend {
                Some(PullRequestBackend::Obs) => with_obs(debug, true, |obs| {
                    search_with(
                        &PullRequestClient::new(Some(obs), None),
                        &query,
                        &coordinates,
                    )
                }),
                Some(PullRequestBackend::Gitea) => with_gitea(debug, |gitea| {
                    search_with(
                        &PullRequestClient::new(None, Some(gitea)),
                        &query,
                        &coordinates,
                    )
                }),
                None => with_obs(debug, true, |obs| {
                    with_gitea(debug, |gitea| {
                        search_with(
                            &PullRequestClient::new(Some(obs), Some(gitea)),
                            &query,
                            &coordinates,
                        )
                    })
                }),
            }
        }
    }
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
    f: impl FnOnce(&ObsClient) -> Result<T, GeectlError>,
) -> Result<T, GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let effective = config::effective_config(&file_config)?;
    let base = effective.config.obs_url.as_deref().unwrap_or(OBS_API_BASE);
    let credentials = match EnvObsAuth::credentials_for_url(base, base, &file_config.instances) {
        Ok(credentials) => credentials,
        Err(err) if !live => {
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
        ObsClient::with_base_and_credentials(&http, base, credentials)?.with_debug(debug.clone());
    f(&client)
}

/// Run `f` with a `GiteaClient` for the configured instance. The
/// client borrows the transport, so both live in this scope and
/// the caller runs inside it.
fn with_gitea<T>(
    debug: &Debug,
    f: impl FnOnce(&GiteaClient) -> Result<T, GeectlError>,
) -> Result<T, GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = crate::cli_gitea::gitea_client(&http, debug)?;
    f(&client)
}

/// File a submission from the working copy in the current
/// directory. The checkout opens first (a local read, no
/// credentials), its backend decides the arm, and the arm's flags
/// are validated before any credential resolution — like the other
/// arms validate their handles and targets first. Each arm then
/// refuses a checkout that is not submittable before filing.
fn submit(args: SubmitArgs, debug: &Debug, live: bool) -> Result<(Value, String), GeectlError> {
    let dry_run = DryRun::new(!live).with_debug(debug.clone());
    let wc = if live {
        working_copy::open(&current_dir()?)
    } else {
        working_copy::open_read(&current_dir()?)
    }?;
    match wc.backend {
        Backend::Obs => {
            let target = obs_submit_target(&args, &wc)?;
            with_obs(debug, live, |obs| {
                working_copy::refuse_not_submittable_obs(&wc, obs)?;
                let client = PullRequestClient::new(Some(obs), None).with_debug(debug.clone());
                let submitted = client.submit(&target, &dry_run)?;
                Ok((
                    submit_payload(&submitted, &dry_run)?,
                    submit_text(&target, &submitted, &dry_run),
                ))
            })
        }
        Backend::Git => {
            // The instance URL comes first (config only, no
            // credentials): the target build validates the flags
            // and the origin host against it before any credential
            // resolution, like the other arms. The token is then
            // resolved once, alias-aware, and shared by the API
            // client and the pre-filing fetch, so the two can never
            // disagree (a plain-vars client against an alias-vars
            // fetch, or the reverse). The fetch runs only live; in
            // dry-run the token serves the client alone — a dry-run
            // Gitea act still needs one to build, like every Gitea
            // act in this group. That is the asymmetry with the OBS
            // arm, which prints its dry-run plan with a warning
            // when credentials fail: the Gitea client fails closed
            // without a Bearer token, so there is no unauthenticated
            // client to plan with — the token is a build
            // requirement, not a live-only one. Lifting that is a
            // group-wide policy call, not this verb's.
            let (base, instances) = gitea_instance()?;
            // The base validates before any credential is resolved:
            // a configured base with a path is the base's own error,
            // not a misleading "no token configured".
            validate_gitea_base(&base)?;
            let target = git_submit_target(&args, &wc, &base)?;
            let token = gitea_token_for(&base, &instances)?;
            let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
            let gitea = GiteaClient::with_base(
                &http,
                &base,
                Credentials::Bearer {
                    token: token.clone(),
                },
            )
            .map(|client| client.with_debug(debug.clone()))?;
            let auth = working_copy::GiteaAuth {
                token: &token,
                url: &base,
            };
            working_copy::refuse_not_submittable_git(&wc, Some(&auth), &dry_run)?;
            let client = PullRequestClient::new(None, Some(&gitea)).with_debug(debug.clone());
            let submitted = client.submit(&target, &dry_run)?;
            Ok((
                submit_payload(&submitted, &dry_run)?,
                submit_text(&target, &submitted, &dry_run),
            ))
        }
    }
}

/// The current directory: the checkout root for `submit`, like
/// the working-copy leaves.
fn current_dir() -> Result<std::path::PathBuf, GeectlError> {
    std::env::current_dir().map_err(|e| {
        GeectlError::new(
            ErrorKind::InternalError,
            format!("failed to read the current directory: {e}"),
        )
    })
}

/// The configured Gitea instance URL and the config file's
/// `[instances]` table, for the alias-aware token resolution.
/// Config only — no credentials are read here, so the target build
/// (flag validation, origin-host check) can run before any
/// credential resolution.
fn gitea_instance() -> Result<(String, Vec<(String, String)>), GeectlError> {
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let effective = config::effective_config(&file_config)?;
    let base = effective
        .config
        .gitea_url
        .as_deref()
        .unwrap_or(DEFAULT_GITEA_URL)
        .to_string();
    Ok((base, file_config.instances))
}

/// The Gitea token for `base`, alias-aware: the matching alias's
/// variables from `[instances]` when one matches, else the plain
/// variables (see `resolve_gitea_bearer_for_url`). The one
/// resolution the submit arm shares between its API client and its
/// pre-filing fetch. A missing token fails closed with the plain
/// message; the alias-aware error names the alias variables when
/// an alias matches.
fn gitea_token_for(base: &str, instances: &[(String, String)]) -> Result<String, GeectlError> {
    match EnvGiteaAuth::credentials_for_url(base, base, instances)? {
        Credentials::Bearer { token } => Ok(token),
        _ => Err(GeectlError::new(
            ErrorKind::ConfigError,
            "no Gitea token configured (GEECTL_GITEA_TOKEN or GEECTL_GITEA_TOKEN_FILE)",
        )),
    }
}

/// The OBS filing target from the submit flags: the foreign
/// (Gitea) flags are refused, the required flags are named, the
/// CVEs are parsed. The source is the checkout itself.
fn obs_submit_target(
    args: &SubmitArgs,
    wc: &working_copy::WorkingCopy,
) -> Result<SubmitTarget, GeectlError> {
    refuse_gitea_flags(args)?;
    let target_project = args.target_project.as_deref().ok_or_else(|| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            "--target-project is required to file an OBS submit request",
        )
    })?;
    let description = args.description.as_deref().ok_or_else(|| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            "--description is required to file an OBS submit request",
        )
    })?;
    let changes_entry = args.changes_entry.as_deref().ok_or_else(|| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            "--changes-entry is required to file an OBS submit request",
        )
    })?;
    let mut cves = Vec::new();
    for value in &args.cve {
        cves.push(CveId::parse(value)?);
    }
    let mut payload = SrPayload::new(
        &wc.project,
        &wc.package,
        target_project,
        args.target_package.as_deref().unwrap_or(&wc.package),
        description,
        changes_entry,
        &cves,
    )?;
    if args.without_cleanup {
        payload = payload.without_cleanup();
    }
    Ok(SubmitTarget::Obs(payload))
}

/// Refuse the Gitea arm's flags on an OBS-native checkout: each
/// names what applies instead, so the agent sees the OBS spelling.
fn refuse_gitea_flags(args: &SubmitArgs) -> Result<(), GeectlError> {
    if args.repo.is_some() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "--repo does not apply to an OBS-native checkout: an OBS submit request targets a project, not a Gitea repo",
        ));
    }
    if args.head.is_some() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "--head does not apply to an OBS-native checkout: the request files the checkout as it stands",
        ));
    }
    if args.base.is_some() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "--base does not apply to an OBS-native checkout: the target is --target-project",
        ));
    }
    if args.title.is_some() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "--title does not apply to an OBS-native checkout: the request takes --description",
        ));
    }
    if args.body.is_some() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "--body does not apply to an OBS-native checkout: the request takes --description",
        ));
    }
    if args.draft {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "--draft does not apply to an OBS-native checkout: OBS has no draft requests",
        ));
    }
    Ok(())
}

/// The Gitea filing target from the submit flags: the foreign
/// (OBS) flags are refused, the required flags are named, the repo
/// and head default from the checkout (its origin remote, its
/// current branch).
fn git_submit_target(
    args: &SubmitArgs,
    wc: &working_copy::WorkingCopy,
    instance_url: &str,
) -> Result<SubmitTarget, GeectlError> {
    refuse_obs_flags(args)?;
    let (owner, repo) = match &args.repo {
        Some(repo) => parse_gitea_repo(repo)?,
        None => working_copy::origin_repo(wc, instance_url)?,
    };
    let head = match &args.head {
        Some(head) => head.clone(),
        None => working_copy::current_branch(wc)?,
    };
    let base = args.base.as_deref().ok_or_else(|| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            "--base is required to open a Gitea pull request",
        )
    })?;
    let title = args.title.as_deref().ok_or_else(|| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            "--title is required to open a Gitea pull request",
        )
    })?;
    let mut options = PrCreateOptions::new(&head, base, title)?;
    options.body = args.body.clone();
    options.draft = args.draft;
    Ok(SubmitTarget::Gitea {
        owner,
        repo,
        options,
    })
}

/// Refuse the OBS arm's flags on a git-backed checkout: each names
/// what applies instead, so the agent sees the Gitea spelling.
fn refuse_obs_flags(args: &SubmitArgs) -> Result<(), GeectlError> {
    if args.target_project.is_some() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "--target-project does not apply to a git-backed checkout: the pull request goes to --repo",
        ));
    }
    if args.target_package.is_some() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "--target-package does not apply to a git-backed checkout: the pull request files the whole branch",
        ));
    }
    if args.description.is_some() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "--description does not apply to a git-backed checkout: the pull request takes --title and --body",
        ));
    }
    if args.changes_entry.is_some() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "--changes-entry does not apply to a git-backed checkout: OBS-only",
        ));
    }
    if !args.cve.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "--cve does not apply to a git-backed checkout: OBS-only",
        ));
    }
    if args.without_cleanup {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "--without-cleanup does not apply to a git-backed checkout: OBS-only",
        ));
    }
    Ok(())
}

/// The review target for an act: `None` when no target flag
/// was given (the review asked of the acting user), else the
/// target the flags name. Built before any credential
/// resolution, so an invalid flag fails as InvalidInput whether
/// or not credentials are configured.
fn review_target(
    user: Option<&str>,
    group: Option<&str>,
    project: Option<&str>,
    package: Option<&str>,
) -> Result<Option<ReviewTarget>, GeectlError> {
    match (user, group, project, package) {
        (None, None, None, None) => Ok(None),
        _ => Ok(Some(ReviewTarget::new(user, group, project, package)?)),
    }
}

/// One review act (`approve` / `request-changes`) through the
/// uniform client: the handle and the target are validated before
/// any credential resolution, like `show`.
#[allow(clippy::too_many_arguments)]
fn review_act_with(
    args: ReviewActArgs,
    debug: &Debug,
    live: bool,
    present: &str,
    past: &str,
    act: impl FnOnce(
        &PullRequestClient,
        &PullRequestRef,
        Option<&ReviewTarget>,
        Option<&str>,
        &DryRun,
    ) -> Result<Option<ReviewOpinion>, GeectlError>,
) -> Result<(Value, String), GeectlError> {
    let reference: PullRequestRef = args.reference.parse()?;
    let target = review_target(
        args.user.as_deref(),
        args.group.as_deref(),
        args.project.as_deref(),
        args.package.as_deref(),
    )?;
    let dry_run = DryRun::new(!live).with_debug(debug.clone());
    match &reference {
        PullRequestRef::Obs(_) => with_obs(debug, live, |obs| {
            let client = PullRequestClient::new(Some(obs), None);
            let opinion = act(
                &client,
                &reference,
                target.as_ref(),
                args.message.as_deref(),
                &dry_run,
            )?;
            Ok((
                review_act_payload(&reference, &opinion, &dry_run)?,
                review_act_text(
                    present,
                    past,
                    &reference,
                    &opinion,
                    args.message.as_deref(),
                    &dry_run,
                ),
            ))
        }),
        PullRequestRef::Gitea { .. } => with_gitea(debug, |gitea| {
            let client = PullRequestClient::new(None, Some(gitea));
            let opinion = act(
                &client,
                &reference,
                target.as_ref(),
                args.message.as_deref(),
                &dry_run,
            )?;
            Ok((
                review_act_payload(&reference, &opinion, &dry_run)?,
                review_act_text(
                    present,
                    past,
                    &reference,
                    &opinion,
                    args.message.as_deref(),
                    &dry_run,
                ),
            ))
        }),
    }
}

/// The payload for a review act: the reference, the recorded
/// opinion (`null` in dry-run), and `dry_run`, plus the plan in
/// dry-run, like the request group's act payloads.
fn review_act_payload(
    reference: &PullRequestRef,
    opinion: &Option<ReviewOpinion>,
    dry_run: &DryRun,
) -> Result<Value, GeectlError> {
    let mut payload = json!({
        "reference": reference.to_string(),
        "review": match opinion {
            Some(opinion) => payload_json(opinion)?,
            None => Value::Null,
        },
        "dry_run": dry_run.enabled(),
    });
    add_plan(&mut payload, dry_run);
    Ok(payload)
}

/// The text for a review act: the reference first (the coordinate
/// echo), then the recorded reviewer in live mode, the message
/// when one was given.
fn review_act_text(
    present: &str,
    past: &str,
    reference: &PullRequestRef,
    opinion: &Option<ReviewOpinion>,
    message: Option<&str>,
    dry_run: &DryRun,
) -> String {
    let mut text = match (dry_run.enabled(), opinion) {
        (true, _) => format!("dry-run: would {present} {reference}"),
        (false, Some(opinion)) => format!("{past} {reference} as {}", opinion.reviewer),
        (false, None) => format!("{past} {reference}"),
    };
    if let Some(message) = message {
        if !message.is_empty() {
            text.push_str(&format!(" — {message}"));
        }
    }
    text
}

/// One pull-request-level act (`merge` / `decline` /
/// `withdraw` / `reopen`) through the uniform client: the handle
/// is validated before any credential resolution, like `show`.
fn level_act_with(
    reference: String,
    debug: &Debug,
    live: bool,
    present: &str,
    past: &str,
    message: Option<&str>,
    act: impl FnOnce(
        &PullRequestClient,
        &PullRequestRef,
        &DryRun,
    ) -> Result<Option<PullRequestStatus>, GeectlError>,
) -> Result<(Value, String), GeectlError> {
    let reference: PullRequestRef = reference.parse()?;
    let dry_run = DryRun::new(!live).with_debug(debug.clone());
    match &reference {
        PullRequestRef::Obs(_) => with_obs(debug, live, |obs| {
            let client = PullRequestClient::new(Some(obs), None);
            let status = act(&client, &reference, &dry_run)?;
            Ok((
                level_act_payload(&reference, &status, &dry_run)?,
                level_act_text(present, past, &reference, &status, message, &dry_run),
            ))
        }),
        PullRequestRef::Gitea { .. } => with_gitea(debug, |gitea| {
            let client = PullRequestClient::new(None, Some(gitea));
            let status = act(&client, &reference, &dry_run)?;
            Ok((
                level_act_payload(&reference, &status, &dry_run)?,
                level_act_text(present, past, &reference, &status, message, &dry_run),
            ))
        }),
    }
}

/// The payload for a pull-request-level act: the reference, the
/// unified status after the act (`null` in dry-run), and
/// `dry_run`, plus the plan in dry-run, like the request group's
/// act payloads.
fn level_act_payload(
    reference: &PullRequestRef,
    status: &Option<PullRequestStatus>,
    dry_run: &DryRun,
) -> Result<Value, GeectlError> {
    let mut payload = json!({
        "reference": reference.to_string(),
        "status": match status {
            Some(status) => Value::String(status.as_str().to_string()),
            None => Value::Null,
        },
        "dry_run": dry_run.enabled(),
    });
    add_plan(&mut payload, dry_run);
    Ok(payload)
}

/// The text for a pull-request-level act: the reference first
/// (the coordinate echo), then the status after the act in live
/// mode, the message when one was given.
fn level_act_text(
    present: &str,
    past: &str,
    reference: &PullRequestRef,
    status: &Option<PullRequestStatus>,
    message: Option<&str>,
    dry_run: &DryRun,
) -> String {
    let mut text = match (dry_run.enabled(), status) {
        (true, _) => format!("dry-run: would {present} {reference}"),
        (false, Some(status)) => format!("{past} {reference} (now {})", status.as_str()),
        (false, None) => format!("{past} {reference}"),
    };
    if let Some(message) = message {
        if !message.is_empty() {
            text.push_str(&format!(" — {message}"));
        }
    }
    text
}

/// The payload for a comment: the reference, the created comment
/// (`null` in dry-run), and `dry_run`, plus the plan in dry-run.
fn comment_payload(
    reference: &PullRequestRef,
    comment: &Option<ReviewComment>,
    dry_run: &DryRun,
) -> Result<Value, GeectlError> {
    let mut payload = json!({
        "reference": reference.to_string(),
        "comment": match comment {
            Some(comment) => payload_json(comment)?,
            None => Value::Null,
        },
        "dry_run": dry_run.enabled(),
    });
    add_plan(&mut payload, dry_run);
    Ok(payload)
}

/// The text for a comment: the reference first, then the created
/// comment's id and body in live mode.
fn comment_text(
    reference: &PullRequestRef,
    comment: &Option<ReviewComment>,
    body: &str,
    dry_run: &DryRun,
) -> String {
    if dry_run.enabled() {
        format!("dry-run: would comment on {reference}: {body}")
    } else {
        match comment {
            Some(comment) => format!(
                "commented on {reference} (comment {} by {}): {}",
                comment.id, comment.author, comment.body
            ),
            None => format!("commented on {reference}: {body}"),
        }
    }
}

/// The payload for a submit: the filed handle (`null` in
/// dry-run), the superseded handles (`null` in dry-run), and
/// `dry_run`, plus the plan in dry-run.
fn submit_payload(submitted: &Option<Submitted>, dry_run: &DryRun) -> Result<Value, GeectlError> {
    let mut payload = json!({
        "reference": match submitted {
            Some(submitted) => payload_json(&submitted.reference)?,
            None => Value::Null,
        },
        "superseded": match submitted {
            Some(submitted) => {
                let mut items = Vec::new();
                for reference in &submitted.superseded {
                    items.push(payload_json(reference)?);
                }
                Value::Array(items)
            }
            None => Value::Null,
        },
        "dry_run": dry_run.enabled(),
    });
    add_plan(&mut payload, dry_run);
    Ok(payload)
}

/// The text for a submit: in dry-run, what would be filed and
/// where — the checkout against its target; live, the filed
/// handle and what it superseded.
fn submit_text(target: &SubmitTarget, submitted: &Option<Submitted>, dry_run: &DryRun) -> String {
    if dry_run.enabled() {
        match target {
            SubmitTarget::Obs(payload) => format!(
                "dry-run: would file a submit request for {}/{} to {}",
                payload.source_project(),
                payload.source_package(),
                payload.target_project()
            ),
            SubmitTarget::Gitea {
                owner,
                repo,
                options,
            } => format!(
                "dry-run: would open a pull request for {owner}/{repo} from {} to {}",
                options.head, options.base
            ),
        }
    } else {
        match submitted {
            Some(submitted) => {
                let mut text = format!("filed {}", submitted.reference);
                if !submitted.superseded.is_empty() {
                    let names: Vec<String> = submitted
                        .superseded
                        .iter()
                        .map(ToString::to_string)
                        .collect();
                    text.push_str(&format!(" (supersedes {})", names.join(", ")));
                }
                text
            }
            // Unreachable: the library returns `Some` live, `None`
            // only in dry-run — but the type allows it, so name it
            // rather than panic.
            None => "filed the submission".to_string(),
        }
    }
}

/// The payload for a comment list: the reference, the comments,
/// and the count.
fn comments_payload(
    reference: &PullRequestRef,
    comments: &[ReviewComment],
) -> Result<Value, GeectlError> {
    let mut items = Vec::new();
    for comment in comments {
        items.push(payload_json(comment)?);
    }
    Ok(json!({
        "reference": reference.to_string(),
        "comments": items,
        "count": comments.len(),
    }))
}

/// The text for a comment list: one header line echoing the
/// reference and the count, then one line per comment — id,
/// author, time when recorded, body, and the parent when it is a
/// reply — or `no comments` under the header when empty.
fn comments_text(reference: &PullRequestRef, comments: &[ReviewComment]) -> String {
    let header = format!(
        "comments for {reference} ({} comment{})",
        comments.len(),
        if comments.len() == 1 { "" } else { "s" }
    );
    if comments.is_empty() {
        return format!("{header}\nno comments");
    }
    let mut lines = vec![header];
    for comment in comments {
        let mut line = format!("comment {} by {}", comment.id, comment.author);
        if !comment.created.is_empty() {
            line.push_str(&format!(" at {}", comment.created));
        }
        line.push_str(&format!(": {}", comment.body));
        if let Some(parent) = comment.parent_id {
            line.push_str(&format!(" (reply to {parent})"));
        }
        lines.push(line);
    }
    lines.join("\n")
}

/// The payload for a cancel: the reference, the cancelled names
/// (`null` in dry-run), and `dry_run`, plus the plan in dry-run.
fn cancel_payload(
    reference: &PullRequestRef,
    cancelled: &Option<Vec<String>>,
    dry_run: &DryRun,
) -> Result<Value, GeectlError> {
    let mut payload = json!({
        "reference": reference.to_string(),
        "cancelled": match cancelled {
            Some(cancelled) => Value::Array(
                cancelled.iter().map(|name| Value::String(name.clone())).collect(),
            ),
            None => Value::Null,
        },
        "dry_run": dry_run.enabled(),
    });
    add_plan(&mut payload, dry_run);
    Ok(payload)
}

/// The text for a cancel: the reference first, then the cancelled
/// names in live mode, or the requested names in dry-run.
fn cancel_text(
    reference: &PullRequestRef,
    reviewers: &[String],
    all: bool,
    cancelled: &Option<Vec<String>>,
    dry_run: &DryRun,
) -> String {
    if dry_run.enabled() {
        if matches!(cancelled, Some(cancelled) if cancelled.is_empty()) {
            format!("no pending review requests to cancel on {reference}")
        } else if all {
            format!("dry-run: would cancel all pending review requests on {reference}")
        } else {
            format!(
                "dry-run: would cancel the review request of {} on {reference}",
                reviewers.join(", ")
            )
        }
    } else {
        match cancelled {
            Some(cancelled) if cancelled.is_empty() => {
                format!("no pending review requests to cancel on {reference}")
            }
            Some(cancelled) => format!(
                "cancelled review requests on {reference}: {}",
                cancelled.join(", ")
            ),
            None => format!("cancelled review requests on {reference}"),
        }
    }
}

/// Insert the recorded dry-run plan into a payload, in dry-run
/// only: a live run's plan is empty by construction, so the field
/// is omitted there. Mirrors the request group's `add_plan`.
fn add_plan(payload: &mut Value, dry_run: &DryRun) {
    if dry_run.enabled() {
        if let Value::Object(map) = payload {
            map.insert(
                "plan".to_string(),
                Value::Array(dry_run.plan().into_iter().map(Value::String).collect()),
            );
        }
    }
}

/// Show one pull request through the uniform client: the JSON
/// payload is the unified shape, the text its human rendering.
fn show_with(
    client: &PullRequestClient,
    reference: &PullRequestRef,
) -> Result<(Value, String), GeectlError> {
    let pr = client.show(reference)?;
    Ok((payload_json(&pr)?, render_pull_request_text(&pr)))
}

/// Search pull requests through the uniform client: the JSON
/// payload is the list shape with a count, the text the header
/// plus the table.
fn search_with(
    client: &PullRequestClient,
    query: &PullRequestSearch,
    coordinates: &str,
) -> Result<(Value, String), GeectlError> {
    let summaries = client.search(query)?;
    let mut items = Vec::new();
    for summary in &summaries {
        items.push(payload_json(summary)?);
    }
    let payload = json!({
        "pull_requests": items,
        "count": summaries.len(),
    });
    Ok((payload, render_search_text(coordinates, &summaries)?))
}

/// Serialize a pull-request shape to its JSON payload value.
fn payload_json<T: Serialize>(value: &T) -> Result<Value, GeectlError> {
    serde_json::to_value(value).map_err(|err| {
        GeectlError::new(
            ErrorKind::InternalError,
            format!("cannot serialize the pull-request payload: {err}"),
        )
    })
}

// ---------- Text renderers ----------

/// The `show` text: the reference first (the coordinate echo),
/// then the fields in order — title and description only when
/// present, the merge flags only when set, the endpoints only
/// when non-empty, one line per review, the labels last.
fn render_pull_request_text(pr: &PullRequest) -> String {
    let mut lines = vec![format!("reference: {}", pr.reference)];
    if !pr.title.is_empty() {
        lines.push(format!("title: {}", pr.title));
    }
    lines.push(format!("author: {}", pr.author));
    lines.push(format!("state: {}", pr.status.as_str()));
    if !pr.description.is_empty() {
        lines.push(format!("description: {}", pr.description));
    }
    if pr.draft {
        lines.push("draft: true".to_string());
    }
    if pr.merged {
        lines.push("merged: true".to_string());
    }
    if let Some(mergeable) = pr.mergeable {
        lines.push(format!("mergeable: {mergeable}"));
    }
    if let Some(text) = endpoint_text(&pr.source) {
        lines.push(format!("source: {text}"));
    }
    if let Some(text) = endpoint_text(&pr.target) {
        lines.push(format!("target: {text}"));
    }
    for (index, review) in pr.reviews.iter().enumerate() {
        lines.push(format!(
            "review {}: {} {}",
            index + 1,
            review.reviewer,
            review.state.as_str()
        ));
        if !review.comment.is_empty() {
            lines.push(format!("review {} comment: {}", index + 1, review.comment));
        }
        if let Some(when) = &review.when {
            lines.push(format!("review {} at: {}", index + 1, when));
        }
    }
    if let Some(error) = &pr.reviews_error {
        lines.push(format!("reviews unavailable: {error}"));
    }
    if !pr.labels.is_empty() {
        lines.push(format!("labels: {}", pr.labels.join(", ")));
    }
    lines.join("\n")
}

/// One endpoint as text: the repository, the branch in
/// parentheses when present, the commit SHA when present. `None`
/// when the endpoint is entirely empty — a delete request has no
/// source, and the line is omitted rather than rendered blank.
fn endpoint_text(endpoint: &PullRequestEndpoint) -> Option<String> {
    if endpoint.repository.is_empty() && endpoint.branch.is_none() && endpoint.commit.is_none() {
        return None;
    }
    let mut parts = Vec::new();
    if !endpoint.repository.is_empty() {
        parts.push(endpoint.repository.clone());
    }
    if let Some(branch) = &endpoint.branch {
        parts.push(format!("({branch})"));
    }
    if let Some(commit) = &endpoint.commit {
        parts.push(commit.clone());
    }
    Some(parts.join(" "))
}

/// The coordinate filters as one echo string: ` backend=obs
/// state=open creator=scarabeusiv`, or empty when no filter was
/// given. Only the given filters are echoed.
fn search_coordinates(filters: &[(&str, &[String])]) -> String {
    let coordinates: Vec<String> = filters
        .iter()
        .filter(|(_, values)| !values.is_empty())
        .map(|(key, values)| format!("{key}={}", values.join(",")))
        .collect();
    if coordinates.is_empty() {
        String::new()
    } else {
        format!(" {}", coordinates.join(" "))
    }
}

/// One header line echoing the search coordinates and the result
/// count: `search backend=obs state=open (2 pull requests)`, or
/// `search (0 pull requests)` when no filter was given.
fn search_header(coordinates: &str, count: usize) -> String {
    format!(
        "search{coordinates} ({} pull request{})",
        count,
        if count == 1 { "" } else { "s" }
    )
}

/// The search text: one header line echoing the coordinates, then
/// the table (reference, state, author, source, target) — or `no
/// pull requests` under the header when nothing matched.
fn render_search_text(
    coordinates: &str,
    summaries: &[PullRequestSummary],
) -> Result<String, GeectlError> {
    let header = search_header(coordinates, summaries.len());
    if summaries.is_empty() {
        return Ok(format!("{header}\nno pull requests"));
    }
    let rows: Vec<Vec<String>> = summaries
        .iter()
        .map(|summary| {
            vec![
                summary.reference.to_string(),
                summary.status.as_str().to_string(),
                summary.author.clone(),
                endpoint_cell(&summary.source),
                endpoint_cell(&summary.target),
            ]
        })
        .collect();
    Ok(format!(
        "{header}\n{}",
        render_table(&["REFERENCE", "STATE", "AUTHOR", "SOURCE", "TARGET"], &rows)?
    ))
}

/// One endpoint as a table cell: the repository, the branch in
/// parentheses when present. The commit SHA stays JSON-only (and
/// in the `show` text): a 40-hex SHA in a table cell would widen
/// every row.
fn endpoint_cell(endpoint: &PullRequestEndpoint) -> String {
    if endpoint.repository.is_empty() {
        return match &endpoint.branch {
            // A deleted fork: the repo is gone, but the branch the
            // PR came from is still known — name it, so the cell
            // is not blank while the JSON carries the branch.
            Some(branch) => format!("({branch})"),
            None => String::new(),
        };
    }
    match &endpoint.branch {
        Some(branch) => format!("{} ({branch})", endpoint.repository),
        None => endpoint.repository.clone(),
    }
}

/// A table with a header and space-aligned columns, never truncated:
/// the last column is unpadded, the rest are padded to the widest
/// cell, and columns are joined with two spaces. A row with the
/// wrong cell count is an internal error: the table is built from
/// library shapes, so a mismatch is a caller bug, and the CLI
/// reports it instead of panicking.
fn render_table(headers: &[&str], rows: &[Vec<String>]) -> Result<String, GeectlError> {
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|header| header.chars().count())
        .collect();
    for row in rows {
        if row.len() != headers.len() {
            return Err(GeectlError::new(
                ErrorKind::InternalError,
                format!(
                    "search table row has {} cells, headers have {}",
                    row.len(),
                    headers.len()
                ),
            ));
        }
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }
    let render_row = |cells: &[String]| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(index, cell)| {
                if index + 1 == cells.len() {
                    cell.clone()
                } else {
                    format!("{:<width$}", cell, width = widths[index])
                }
            })
            .collect::<Vec<_>>()
            .join("  ")
    };
    let mut lines = vec![render_row(
        &headers.iter().map(ToString::to_string).collect::<Vec<_>>(),
    )];
    for row in rows {
        lines.push(render_row(row));
    }
    Ok(lines.join("\n"))
}

// ---------- Tests ----------
// Fixture pull requests come from the library's `from_obs` on
// server-shaped XML and `from_gitea` on server-shaped JSON, so the
// converters and renderers are tested against the same documents
// the client reads.

#[cfg(test)]
mod tests {
    use super::*;
    use geectl::gitea::{GiteaPullRequest, GiteaReview};
    use geectl::obs::parse_request;

    const REQUEST_XML: &str = r#"<request creator="creator" id="123">
  <action type="submit">
    <source package="abc" project="xyz" />
    <options>
      <sourceupdate>cleanup</sourceupdate>
      <updatelink>1</updatelink>
    </options>
    <acceptinfo rev="5" srcmd5="deadbeef" />
  </action>
  <action type="add_role">
    <target project="home:foo" />
    <person name="bar" role="maintainer" />
    <group name="groupxyz" role="reader" />
  </action>
  <state approver="someone" name="review" when="2010-12-27T01:36:29" who="abc" />
  <review by_group="group1" state="new" when="2010-12-28T00:11:22" who="abc">
    <comment>review start</comment>
  </review>
  <history when="2010-12-11T00:00:00" who="creator">
    <description>Created request</description>
  </history>
</request>"#;

    /// A pull-request JSON body, shaped like Gitea 1.24.7 answers.
    const PR_JSON: &str = r#"{"number": 122, "user": {"login": "scarabeusiv"}, "title": "change_review_state", "body": "A test PR body.", "labels": [{"name": "bug"}], "state": "open", "draft": false, "merged": false, "mergeable": true, "head": {"label": "scarabeusiv:change-review-state", "ref": "change-review-state", "sha": "e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa", "repo": {"full_name": "scarabeusiv/geectl"}}, "base": {"label": "main", "ref": "main", "sha": "e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa", "repo": {"full_name": "openSUSE/geectl"}}}"#;

    /// Two reviews, shaped like Gitea 1.24.7 answers.
    const REVIEWS_JSON: &str = r#"[{"user": {"login": "plusky"}, "state": "APPROVED", "body": "Looks good.", "dismissed": false, "submitted_at": "2026-11-22T19:52:58Z"}, {"user": {"login": "mimi1vx"}, "state": "COMMENT", "body": "One nit.", "dismissed": false, "submitted_at": "2026-11-22T19:53:10Z"}]"#;

    fn fixture_obs_pull_request() -> PullRequest {
        let request = parse_request(REQUEST_XML).expect("fixture request parses");
        PullRequest::from_obs(&request).expect("fixture request maps")
    }

    fn fixture_gitea_pull_request() -> PullRequest {
        let pr: GiteaPullRequest = serde_json::from_str(PR_JSON).expect("fixture PR parses");
        let reviews: Vec<GiteaReview> =
            serde_json::from_str(REVIEWS_JSON).expect("fixture reviews parse");
        PullRequest::from_gitea("openSUSE", "geectl", &pr, &reviews).expect("fixture PR maps")
    }

    #[test]
    fn pull_request_json_and_text_agree() {
        let pr = fixture_gitea_pull_request();
        let payload = payload_json(&pr).unwrap();
        assert_eq!(payload["reference"], "gitea:openSUSE/geectl#122");
        assert_eq!(payload["title"], "change_review_state");
        assert_eq!(payload["description"], "A test PR body.");
        assert_eq!(payload["author"], "scarabeusiv");
        assert_eq!(payload["status"], "open");
        assert_eq!(payload["draft"], false);
        assert_eq!(payload["merged"], false);
        assert_eq!(payload["mergeable"], true);
        assert!(
            payload.as_object().unwrap().contains_key("reviews_error"),
            "reviews_error must be emitted on every read"
        );
        assert_eq!(payload["reviews_error"], serde_json::Value::Null);
        assert_eq!(payload["source"]["repository"], "scarabeusiv/geectl");
        assert_eq!(payload["source"]["branch"], "change-review-state");
        assert_eq!(payload["target"]["repository"], "openSUSE/geectl");
        assert_eq!(payload["target"]["branch"], "main");
        assert_eq!(payload["reviews"][0]["reviewer"], "plusky");
        assert_eq!(payload["reviews"][0]["state"], "approved");
        assert_eq!(payload["reviews"][0]["comment"], "Looks good.");
        assert_eq!(payload["labels"][0], "bug");

        // Every id, name and revision in the JSON appears in the text.
        let text = render_pull_request_text(&pr);
        for expected in [
            "gitea:openSUSE/geectl#122",
            "change_review_state",
            "A test PR body.",
            "scarabeusiv",
            "open",
            "scarabeusiv/geectl",
            "change-review-state",
            "e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa",
            "openSUSE/geectl",
            "main",
            "plusky",
            "approved",
            "Looks good.",
            "mimi1vx",
            "commented",
            "One nit.",
            "bug",
        ] {
            assert!(
                text.contains(expected),
                "text missing {expected:?}:\n{text}"
            );
        }
    }

    #[test]
    fn obs_show_text_skips_the_absent_title_and_empty_target() {
        let pr = fixture_obs_pull_request();
        assert_eq!(pr.title, "");
        assert_eq!(pr.target.repository, "");
        let text = render_pull_request_text(&pr);
        assert!(
            !text.contains("title:"),
            "empty title must not render:\n{text}"
        );
        assert!(
            !text.contains("target:"),
            "empty target must not render:\n{text}"
        );
        for expected in [
            "obs:123",
            "creator",
            "open",
            "xyz/abc",
            "group group1",
            "review start",
        ] {
            assert!(
                text.contains(expected),
                "text missing {expected:?}:\n{text}"
            );
        }
    }

    #[test]
    fn gitea_show_text_names_a_failed_reviews_fetch() {
        let mut pr = fixture_gitea_pull_request();
        pr.reviews = Vec::new();
        pr.reviews_error = Some("NetworkError: canned GET error".to_string());
        let payload = payload_json(&pr).unwrap();
        assert_eq!(payload["reviews_error"], "NetworkError: canned GET error");
        let text = render_pull_request_text(&pr);
        assert!(
            text.contains("reviews unavailable: NetworkError: canned GET error"),
            "the text must name the failed reviews fetch:\n{text}"
        );
        assert!(
            !text.contains("review 1:"),
            "no reviews must render when the fetch failed:\n{text}"
        );
    }

    #[test]
    fn search_text_renders_a_table_and_names_the_empty_case() {
        let gitea_pr: GiteaPullRequest = serde_json::from_str(PR_JSON).expect("fixture PR parses");
        let summary = PullRequestSummary::from_gitea("openSUSE", "geectl", &gitea_pr)
            .expect("fixture PR maps");
        let text = render_search_text("", &[summary]).expect("table renders");
        let mut lines = text.lines();
        let header = lines.next().expect("a header line");
        assert_eq!(header, "search (1 pull request)");
        let columns = lines.next().expect("a column header line");
        assert!(columns.contains("REFERENCE"));
        assert!(columns.contains("STATE"));
        assert!(columns.contains("AUTHOR"));
        assert!(columns.contains("SOURCE"));
        assert!(columns.contains("TARGET"));
        let row = lines.next().expect("a row line");
        assert!(row.contains("gitea:openSUSE/geectl#122"));
        assert!(row.contains("open"));
        assert!(row.contains("scarabeusiv"));
        assert!(row.contains("scarabeusiv/geectl (change-review-state)"));
        assert!(row.contains("openSUSE/geectl (main)"));
        assert_eq!(
            render_search_text(" backend=gitea", &[]).expect("empty table renders"),
            "search backend=gitea (0 pull requests)\nno pull requests"
        );
    }

    #[test]
    fn deleted_fork_source_renders_its_branch_in_both_formats() {
        let mut pr = fixture_gitea_pull_request();
        pr.source.repository = String::new();
        let text = render_pull_request_text(&pr);
        assert!(
            text.contains("source: (change-review-state)"),
            "a deleted-fork source must name its branch without a blank repository:\n{text}"
        );
        let gitea_pr: GiteaPullRequest = serde_json::from_str(PR_JSON).expect("fixture PR parses");
        let mut summary = PullRequestSummary::from_gitea("openSUSE", "geectl", &gitea_pr)
            .expect("fixture PR maps");
        summary.source.repository = String::new();
        let table = render_search_text("", &[summary]).expect("table renders");
        assert!(
            table.contains("(change-review-state)"),
            "the search table must name the deleted-fork branch:\n{table}"
        );
    }

    #[test]
    fn search_coordinates_echo_only_the_given_filters() {
        assert_eq!(search_coordinates(&[]), "");
        assert_eq!(
            search_coordinates(&[
                ("backend", &["gitea".to_string()]),
                ("state", &["open".to_string(), "merged".to_string()]),
                ("creator", &[]),
            ]),
            " backend=gitea state=open,merged"
        );
        assert_eq!(search_header("", 0), "search (0 pull requests)");
        assert_eq!(
            search_header(" backend=gitea state=open", 2),
            "search backend=gitea state=open (2 pull requests)"
        );
    }

    fn fixture_opinion() -> ReviewOpinion {
        ReviewOpinion {
            reviewer: "plusky".to_string(),
            state: geectl::pull_request::OpinionState::Approved,
            comment: "Looks good.".to_string(),
            when: Some("2026-11-22T19:52:58Z".to_string()),
        }
    }

    fn fixture_comment() -> ReviewComment {
        ReviewComment {
            id: 7,
            author: "plusky".to_string(),
            created: "2026-11-22T19:52:58Z".to_string(),
            body: "One nit.".to_string(),
            parent_id: None,
        }
    }

    fn gitea_reference() -> PullRequestRef {
        PullRequestRef::Gitea {
            owner: "openSUSE".to_string(),
            repo: "geectl".to_string(),
            number: 122,
        }
    }

    #[test]
    fn approve_json_and_text_agree() {
        let reference = gitea_reference();
        let opinion = Some(fixture_opinion());
        let dry_run = DryRun::new(false);
        let payload = review_act_payload(&reference, &opinion, &dry_run).unwrap();
        assert_eq!(payload["reference"], "gitea:openSUSE/geectl#122");
        assert_eq!(payload["review"]["reviewer"], "plusky");
        assert_eq!(payload["review"]["state"], "approved");
        assert_eq!(payload["review"]["comment"], "Looks good.");
        assert_eq!(payload["review"]["when"], "2026-11-22T19:52:58Z");
        assert_eq!(payload["dry_run"], false);
        assert!(payload.get("plan").is_none());

        let text = review_act_text(
            "approve",
            "approved",
            &reference,
            &opinion,
            Some("Looks good."),
            &dry_run,
        );
        for expected in [
            "gitea:openSUSE/geectl#122",
            "approved",
            "plusky",
            "Looks good.",
        ] {
            assert!(
                text.contains(expected),
                "text missing {expected:?}:\n{text}"
            );
        }
    }

    #[test]
    fn review_act_dry_run_reports_no_opinion_and_carries_the_plan() {
        let reference = gitea_reference();
        let dry_run = DryRun::new(true);
        dry_run.allow("approve pull request openSUSE/geectl#122");
        let payload = review_act_payload(&reference, &None, &dry_run).unwrap();
        assert_eq!(payload["review"], serde_json::Value::Null);
        assert_eq!(payload["dry_run"], true);
        assert_eq!(
            payload["plan"][0],
            "[dry-run] would: approve pull request openSUSE/geectl#122"
        );

        let text = review_act_text(
            "approve",
            "approved",
            &reference,
            &None,
            Some("Looks good."),
            &dry_run,
        );
        assert_eq!(
            text,
            "dry-run: would approve gitea:openSUSE/geectl#122 — Looks good."
        );
    }

    #[test]
    fn comment_json_and_text_agree() {
        let reference = gitea_reference();
        let comment = Some(fixture_comment());
        let dry_run = DryRun::new(false);
        let payload = comment_payload(&reference, &comment, &dry_run).unwrap();
        assert_eq!(payload["reference"], "gitea:openSUSE/geectl#122");
        assert_eq!(payload["comment"]["id"], 7);
        assert_eq!(payload["comment"]["author"], "plusky");
        assert_eq!(payload["comment"]["body"], "One nit.");
        assert_eq!(payload["dry_run"], false);

        let text = comment_text(&reference, &comment, "One nit.", &dry_run);
        for expected in [
            "gitea:openSUSE/geectl#122",
            "comment 7",
            "plusky",
            "One nit.",
        ] {
            assert!(
                text.contains(expected),
                "text missing {expected:?}:\n{text}"
            );
        }

        let dry_run = DryRun::new(true);
        let text = comment_text(&reference, &None, "One nit.", &dry_run);
        assert_eq!(
            text,
            "dry-run: would comment on gitea:openSUSE/geectl#122: One nit."
        );
    }

    #[test]
    fn comments_json_and_text_agree() {
        let reference = gitea_reference();
        let mut reply = fixture_comment();
        reply.id = 8;
        reply.parent_id = Some(7);
        reply.body = "Fixed.".to_string();
        let comments = vec![fixture_comment(), reply];
        let payload = comments_payload(&reference, &comments).unwrap();
        assert_eq!(payload["reference"], "gitea:openSUSE/geectl#122");
        assert_eq!(payload["count"], 2);
        assert_eq!(payload["comments"][1]["parent_id"], 8 - 1);

        let text = comments_text(&reference, &comments);
        for expected in [
            "comments for gitea:openSUSE/geectl#122 (2 comments)",
            "comment 7 by plusky at 2026-11-22T19:52:58Z: One nit.",
            "comment 8 by plusky at 2026-11-22T19:52:58Z: Fixed. (reply to 7)",
        ] {
            assert!(
                text.contains(expected),
                "text missing {expected:?}:\n{text}"
            );
        }
        assert_eq!(
            comments_text(&reference, &[]),
            "comments for gitea:openSUSE/geectl#122 (0 comments)\nno comments"
        );
    }

    #[test]
    fn cancel_json_and_text_agree() {
        let reference = gitea_reference();
        let cancelled = Some(vec!["bob".to_string(), "@probe-team".to_string()]);
        let dry_run = DryRun::new(false);
        let payload = cancel_payload(&reference, &cancelled, &dry_run).unwrap();
        assert_eq!(payload["reference"], "gitea:openSUSE/geectl#122");
        assert_eq!(payload["cancelled"][0], "bob");
        assert_eq!(payload["cancelled"][1], "@probe-team");
        assert_eq!(payload["dry_run"], false);

        let text = cancel_text(&reference, &[], false, &cancelled, &dry_run);
        assert_eq!(
            text,
            "cancelled review requests on gitea:openSUSE/geectl#122: bob, @probe-team"
        );
        let text = cancel_text(&reference, &[], false, &Some(Vec::new()), &dry_run);
        assert_eq!(
            text,
            "no pending review requests to cancel on gitea:openSUSE/geectl#122"
        );

        let dry_run = DryRun::new(true);
        let text = cancel_text(&reference, &["bob".to_string()], false, &None, &dry_run);
        assert_eq!(
            text,
            "dry-run: would cancel the review request of bob on gitea:openSUSE/geectl#122"
        );
        let text = cancel_text(&reference, &[], true, &None, &dry_run);
        assert_eq!(
            text,
            "dry-run: would cancel all pending review requests on gitea:openSUSE/geectl#122"
        );
        let text = cancel_text(&reference, &[], true, &Some(Vec::new()), &dry_run);
        assert_eq!(
            text,
            "no pending review requests to cancel on gitea:openSUSE/geectl#122"
        );
    }

    #[test]
    fn level_act_json_and_text_agree() {
        let reference = gitea_reference();
        let status = Some(PullRequestStatus::Merged);
        let dry_run = DryRun::new(false);
        let payload = level_act_payload(&reference, &status, &dry_run).unwrap();
        assert_eq!(payload["reference"], "gitea:openSUSE/geectl#122");
        assert_eq!(payload["status"], "merged");
        assert_eq!(payload["dry_run"], false);
        assert!(payload.get("plan").is_none());

        let text = level_act_text(
            "merge",
            "merged",
            &reference,
            &status,
            Some("Merging."),
            &dry_run,
        );
        assert_eq!(
            text,
            "merged gitea:openSUSE/geectl#122 (now merged) — Merging."
        );
    }

    #[test]
    fn level_act_dry_run_reports_no_status_and_carries_the_plan() {
        let reference = gitea_reference();
        let dry_run = DryRun::new(true);
        dry_run.allow("merge pull request openSUSE/geectl#122");
        let payload = level_act_payload(&reference, &None, &dry_run).unwrap();
        assert_eq!(payload["status"], serde_json::Value::Null);
        assert_eq!(payload["dry_run"], true);
        assert_eq!(
            payload["plan"][0],
            "[dry-run] would: merge pull request openSUSE/geectl#122"
        );

        let text = level_act_text(
            "decline",
            "declined",
            &reference,
            &None,
            Some("Not now."),
            &dry_run,
        );
        assert_eq!(
            text,
            "dry-run: would decline gitea:openSUSE/geectl#122 — Not now."
        );
        let text = level_act_text("reopen", "reopened", &reference, &None, None, &dry_run);
        assert_eq!(text, "dry-run: would reopen gitea:openSUSE/geectl#122");
    }

    fn obs_submit_target_fixture() -> SubmitTarget {
        SubmitTarget::Obs(
            SrPayload::new(
                "home:alice",
                "foo",
                "home:devel",
                "foo",
                "Update foo",
                "- Update to 1.1.",
                &[],
            )
            .unwrap(),
        )
    }

    fn gitea_submit_target_fixture() -> SubmitTarget {
        SubmitTarget::Gitea {
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            options: PrCreateOptions::new("feature", "main", "Update foo").unwrap(),
        }
    }

    fn obs_submitted_fixture() -> Submitted {
        Submitted {
            reference: PullRequestRef::Obs(123),
            superseded: vec![PullRequestRef::Obs(122)],
        }
    }

    fn gitea_submitted_fixture() -> Submitted {
        Submitted {
            reference: PullRequestRef::Gitea {
                owner: "owner".to_string(),
                repo: "repo".to_string(),
                number: 7,
            },
            superseded: vec![],
        }
    }

    #[test]
    fn submit_json_and_text_agree() {
        let target = obs_submit_target_fixture();
        let submitted = Some(obs_submitted_fixture());
        let dry_run = DryRun::new(false);
        let payload = submit_payload(&submitted, &dry_run).unwrap();
        assert_eq!(payload["reference"], "obs:123");
        assert_eq!(payload["superseded"][0], "obs:122");
        assert_eq!(payload["dry_run"], false);
        assert!(payload.get("plan").is_none());
        let text = submit_text(&target, &submitted, &dry_run);
        assert_eq!(text, "filed obs:123 (supersedes obs:122)");

        let dry_run = DryRun::new(true);
        dry_run.allow("file a submit request for home:alice/foo to home:devel");
        let payload = submit_payload(&None, &dry_run).unwrap();
        assert_eq!(payload["reference"], serde_json::Value::Null);
        assert_eq!(payload["superseded"], serde_json::Value::Null);
        assert_eq!(payload["dry_run"], true);
        assert_eq!(
            payload["plan"][0],
            "[dry-run] would: file a submit request for home:alice/foo to home:devel"
        );
        let text = submit_text(&target, &None, &dry_run);
        assert_eq!(
            text,
            "dry-run: would file a submit request for home:alice/foo to home:devel"
        );
    }

    #[test]
    fn submit_gitea_json_and_text_agree() {
        let target = gitea_submit_target_fixture();
        let submitted = Some(gitea_submitted_fixture());
        let dry_run = DryRun::new(false);
        let payload = submit_payload(&submitted, &dry_run).unwrap();
        assert_eq!(payload["reference"], "gitea:owner/repo#7");
        assert_eq!(payload["superseded"], serde_json::json!([]));
        assert_eq!(payload["dry_run"], false);
        let text = submit_text(&target, &submitted, &dry_run);
        assert_eq!(text, "filed gitea:owner/repo#7");

        let dry_run = DryRun::new(true);
        let text = submit_text(&target, &None, &dry_run);
        assert_eq!(
            text,
            "dry-run: would open a pull request for owner/repo from feature to main"
        );
    }
}
