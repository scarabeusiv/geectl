//! `pull-request` — the unified pull-request command group: read OBS
//! submit requests and Gitea pull requests as one surface.
//!
//! Thin over `PullRequestClient` like the OBS groups are over
//! `ObsClient`: the flags mirror the library parameters, the JSON
//! payload carries the typed values, and the text renderer prints
//! the same values in a human-readable form.
//!
//! Reads are always live; the act leaves (`approve`,
//! `request-changes`, `comment`, `build-comment`, `cancel-review-request`,
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

use crate::auth::{Credentials, EnvGiteaAuth, EnvObsAuth};
use crate::config::{self, DEFAULT_GITEA_URL};
use crate::debug::{strip_terminal_controls, Debug};
use crate::error::{ErrorKind, GeectlError};
use crate::gitea::{
    parse_gitea_repo, parse_scmsync_url, validate_gitea_base, GiteaClient, GiteaMergeMethod,
    PrCreateOptions, ScmsyncUrl,
};
use crate::http::{HttpClient, Timeout, UreqClient};
use crate::obs::{
    package_show_url, AcceptCleanupOutcome, CveId, DryRun, ForwardActionResult, ObsClient,
    ResultsQuery, ReviewTarget, SrPayload, OBS_API_BASE,
};
use crate::pull_request::{
    build_comment_body, PullRequest, PullRequestBackend, PullRequestClient, PullRequestEndpoint,
    PullRequestRef, PullRequestSearch, PullRequestStatus, PullRequestSummary, ReviewComment,
    ReviewOpinion, SubmitTarget, Submitted,
};
use crate::working_copy::{self, Backend};
use clap::{Args, Subcommand};
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
    /// Show the diff of a pull request: the unified diff, like
    /// `request diff`. With `--xml` the OBS arm renders the
    /// request XML with the diff injected; `--xml` is refused on
    /// a Gitea pull request, which has no request XML. A read,
    /// always live.
    Diff {
        /// The pull-request handle: `obs:<id>` or
        /// `gitea:<owner>/<repo>#<number>`.
        reference: String,
        /// Render the OBS request XML with the diff injected,
        /// instead of the bare diff. Refused on a Gitea ref.
        #[arg(long)]
        xml: bool,
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
    /// Post the build-link comment on a Gitea pull request: the
    /// OBS build results for the test project and package, as a
    /// link plus a one-line per-instance summary. The comment is
    /// posted once, on PR opening; there is no polling, no live
    /// update, and no edit. Dry-run by default; mutates only
    /// with `--live`.
    BuildComment(BuildCommentArgs),
    /// List the comments on a pull request, oldest first. A read,
    /// always live.
    Comments {
        /// The pull-request handle: `obs:<id>` or
        /// `gitea:<owner>/<repo>#<number>`.
        reference: String,
    },
    /// Cancel pending review requests on a pull request. Dry-run
    /// by default; mutates only with `--live`. On OBS the cancel
    /// fails closed.
    CancelReviewRequest(CancelArgs),
    /// Merge a pull request. On OBS this accepts the submit
    /// request and forwards it to its link parent; a refused
    /// forward is reported and the accepted request stands. On
    /// Gitea the forward is resolved and validated before the
    /// merge lands, and the merged pull request is forwarded to
    /// its link parent too, unless `--no-forward` is given. The
    /// forwarded request's `.changes` entry is the original
    /// description on the OBS arm and the pull request's text on
    /// the Gitea arm, overridable with `-m` / `--forward-message`.
    /// Dry-run by default; mutates only with `--live`.
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
    /// The project the request targets: the devel project, or
    /// openSUSE:Factory when the source is the recorded devel
    /// project and package. OBS only; required there.
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
    /// Gitea only; the parent of the checkout's origin remote
    /// (the fork's parent, never the fork) when unset.
    #[arg(long)]
    repo: Option<String>,
    /// The branch the change comes from. Gitea only; the
    /// checkout's current branch when unset.
    #[arg(long)]
    head: Option<String>,
    /// The branch the pull request targets. Gitea only; the
    /// branch the checkout tracks (its scmsync branch) when unset,
    /// else the target repo's default branch. The `factory`
    /// branch does not take pull requests.
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
    /// Pin the review to this head commit (Gitea only): the
    /// review is recorded against it server-side, and is refused
    /// when the head has moved. Best-effort: the head is read
    /// just before the review posts, so a push landing between
    /// the read and the post is not caught. Refused on an OBS ref.
    #[arg(long)]
    commit: Option<String>,
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

/// Arguments for `pull-request build-comment`.
#[derive(Debug, Args)]
pub struct BuildCommentArgs {
    /// The pull-request handle: `gitea:<owner>/<repo>#<number>`.
    /// An OBS handle is refused: the build link belongs on the
    /// Gitea pull request.
    reference: String,
    /// The test project the build results are read from.
    #[arg(long)]
    project: String,
    /// The package in the test project.
    #[arg(long)]
    package: String,
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
    /// The changes entry for the forwarded request, overriding
    /// the original description on the OBS arm and the pull
    /// request's text on the Gitea arm. Values starting with
    /// `-` are taken literally: an entry may open with a `- `
    /// bullet.
    #[arg(short = 'm', long = "forward-message", allow_hyphen_values = true)]
    forward_message: Option<String>,
    /// The Gitea merge style (Gitea only; refused on an OBS ref).
    #[arg(long, value_parser = ["merge", "rebase", "rebase-merge", "squash", "fast-forward-only"])]
    method: Option<String>,
    /// Accept past open reviews (OBS only; refused on a Gitea ref).
    #[arg(long)]
    force: bool,
    /// Do not forward the pull request after merging it: by
    /// default a merged Gitea pull request is forwarded to its
    /// link parent. (Gitea only; refused on an OBS ref.)
    #[arg(long)]
    no_forward: bool,
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
        PullRequestCommand::Diff { reference, xml } => {
            // The handle is parsed before any credential
            // resolution, so an invalid handle fails as
            // InvalidInput whether or not credentials are
            // configured; the --xml refusal on a Gitea ref is
            // decided the same way.
            let reference: PullRequestRef = reference.parse()?;
            if xml {
                if let PullRequestRef::Gitea { .. } = &reference {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        "--xml does not apply to a Gitea pull request: a Gitea pull request has no request XML",
                    ));
                }
            }
            match &reference {
                PullRequestRef::Obs(_) => with_obs(debug, true, |obs| {
                    diff_with(&PullRequestClient::new(Some(obs), None), &reference, xml)
                }),
                PullRequestRef::Gitea { .. } => with_gitea(debug, |gitea| {
                    diff_with(&PullRequestClient::new(None, Some(gitea)), &reference, xml)
                }),
            }
        }
        PullRequestCommand::Approve(args) => review_act_with(
            args,
            debug,
            live,
            "approve",
            "approved",
            |client, reference, target, message, commit, dry_run| {
                client.approve(reference, target, message, commit, dry_run)
            },
        ),
        PullRequestCommand::RequestChanges(args) => review_act_with(
            args,
            debug,
            live,
            "request changes on",
            "requested changes on",
            |client, reference, target, message, commit, dry_run| {
                client.request_changes(reference, target, message, commit, dry_run)
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
        PullRequestCommand::BuildComment(args) => {
            // The OBS ref is refused before any credential
            // resolution, so the refusal is reachable without
            // credentials — the library refuses again, as
            // enforcement.
            let reference: PullRequestRef = args.reference.parse()?;
            if let PullRequestRef::Obs(id) = &reference {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "build-comment posts the build link on a Gitea pull request; {id} is an OBS submit request"
                    ),
                ));
            }
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
            let (base, instances) = crate::cli_build::obs_endpoint()?;
            // The link forms before the results read: a
            // non-production host fails before any network.
            let link = package_show_url(&args.project, &args.package, &base)?;
            let query = ResultsQuery {
                package: Some(args.package.clone()),
                ..Default::default()
            };
            let outcomes = crate::cli_build::read_results_across_instances(
                &http,
                debug,
                &base,
                &instances,
                &args.project,
                &query,
            )?;
            let body = build_comment_body(&args.project, &args.package, &link, &outcomes);
            with_gitea(debug, |gitea| {
                let client = PullRequestClient::new(None, Some(gitea));
                let comment = client.build_comment(
                    &reference,
                    &args.project,
                    &args.package,
                    &link,
                    &outcomes,
                    &dry_run,
                )?;
                Ok((
                    build_comment_payload(
                        &reference,
                        &args.project,
                        &args.package,
                        &link,
                        &comment,
                        &dry_run,
                    )?,
                    build_comment_text(
                        &reference,
                        &args.project,
                        &args.package,
                        &comment,
                        &body,
                        &dry_run,
                    ),
                ))
            })
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
            let forward_message = args.forward_message.as_deref();
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
                    if args.no_forward {
                        return Err(GeectlError::new(
                            ErrorKind::InvalidInput,
                            "--no-forward does not apply to an OBS submit request: the merge always forwards after accepting",
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
            // The OBS arm merges with the accept-coupled forward:
            // the merge accepts and forwards after accepting, and
            // a refused forward is reported with the accepted
            // request standing. The Gitea arm validates the
            // forward before merging, then forwards the merged
            // pull request to its link parent, unless
            // `--no-forward` is given.
            match &reference {
                PullRequestRef::Obs(id) => merge_obs(
                    debug,
                    &id.to_string(),
                    message,
                    args.force,
                    forward_message,
                    live,
                ),
                PullRequestRef::Gitea { .. } => merge_gitea(
                    debug,
                    &args.reference,
                    message,
                    method,
                    args.no_forward,
                    forward_message,
                    live,
                ),
            }
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
            // The freshness check's Gitea client resolves lazily,
            // live-only, after the source meta says `<scmsync>`: a
            // dry-run plan reads nothing, so no client is wired
            // there; live, the filing path resolves the client
            // itself, so a malformed `gitea_url` / instance config
            // never fails an OBS-native filing that never reaches
            // Gitea.
            let gitea_http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
            let gitea_http = if live {
                Some(&gitea_http as &dyn HttpClient)
            } else {
                None
            };
            with_obs(debug, live, |obs| {
                working_copy::refuse_not_submittable_obs(&wc, obs)?;
                let client = PullRequestClient::new(Some(obs), None)
                    .with_debug(debug.clone())
                    .with_gitea_http(gitea_http);
                let submitted = client.submit(&target, &dry_run)?;
                Ok((
                    submit_payload(&submitted, &dry_run)?,
                    submit_text(&target, &submitted, &dry_run),
                ))
            })
        }
        Backend::Git => {
            // The instance URL comes first (config only, no
            // credentials): the target build's first phase validates
            // the flags and the origin host against it before any
            // credential resolution, like the other arms. The token
            // is then resolved once, alias-aware, and shared by the
            // API client and the pre-filing fetch, so the two can
            // never disagree (a plain-vars client against an
            // alias-vars fetch, or the reverse). The second phase
            // resolves the defaults the first left open — the
            // parent repo, the target's default branch — through
            // the client, so a dry-run plan reads those (like the
            // OBS arm's freshness read) but files nothing. The
            // fetch runs only live; in
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
            let parts = git_submit_parts(&args, &wc, &base)?;
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
            // `resolve_git_submit_target` consumes the parts, so
            // the explicitness is read first: verification rewords
            // its "pass --repo" remedy when the user already
            // passed it.
            let repo_explicit = parts.repo.is_some();
            let target = resolve_git_submit_target(parts, &gitea)?;
            let verification = match &target {
                SubmitTarget::Gitea {
                    owner,
                    repo,
                    options,
                } => Some(working_copy::GitTargetVerification {
                    client: &gitea,
                    instance_url: &base,
                    owner,
                    repo,
                    base: &options.base,
                    repo_explicit,
                }),
                SubmitTarget::Obs(_) => None,
            };
            working_copy::refuse_not_submittable_git(
                &wc,
                Some(&auth),
                verification.as_ref(),
                &dry_run,
            )?;
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

/// The parts of a Gitea filing target the checkout already
/// knows, before any API read: the target repo when the flags name
/// it (else the origin, for the parent resolution), the head, and
/// the base when the flags or the checkout name it (else the
/// target's default branch). Built before any credential
/// resolution, so an invalid flag fails as InvalidInput whether or
/// not credentials are configured.
#[derive(Debug)]
struct GitSubmitParts {
    /// The target repo when `--repo` names it; None when it must
    /// default to the origin's parent.
    repo: Option<(String, String)>,
    /// The origin repo, when it was read: always, for the head
    /// qualification in phase two — a bare head names a branch on
    /// the fork, and only the origin says which fork that is. None
    /// when `--repo` was explicit and the checkout carries no
    /// readable origin; the target verification re-reads the origin
    /// before anything files, so an unreadable origin is refused
    /// there, not silently filed bare.
    origin: Option<(String, String)>,
    /// The branch the change comes from.
    head: String,
    /// The target branch when the flags or the checkout's scmsync
    /// branch name it; None when it must default to the target
    /// repo's default branch.
    base: Option<String>,
    /// The pull request title.
    title: String,
    /// The pull request body.
    body: Option<String>,
    /// Open the pull request as a draft.
    draft: bool,
}

/// The Gitea filing target from the submit flags, phase one: the
/// foreign (OBS) flags are refused, the required flags are named,
/// and what the checkout already knows is resolved — the repo from
/// `--repo` or the origin remote, the head from `--head` or the
/// current branch, the base from `--base` or the checkout's
/// scmsync branch. The origin is read whether or not `--repo`
/// names the target: the default path resolves the parent from
/// it, and the explicit path qualifies the head with it (phase
/// two). The `factory` base is refused here when it is already
/// known; a defaulted base is refused in phase two.
fn git_submit_parts(
    args: &SubmitArgs,
    wc: &working_copy::WorkingCopy,
    instance_url: &str,
) -> Result<GitSubmitParts, GeectlError> {
    refuse_obs_flags(args)?;
    // The origin is read in both paths: the default path needs it
    // to resolve the parent, and the explicit path needs it to
    // qualify the head (phase two) — a bare head lives on the
    // fork, and only the origin names the fork. With an explicit
    // `--repo` the origin is best-effort: a checkout without a
    // readable origin still reaches the named target, and the
    // target verification re-reads the origin before anything
    // files, so an unreadable origin is refused there.
    let (repo, origin) = match &args.repo {
        Some(repo) => (
            Some(parse_gitea_repo(repo)?),
            working_copy::origin_repo(wc, instance_url).ok(),
        ),
        None => {
            let origin = working_copy::origin_repo(wc, instance_url)?;
            (None, Some(origin))
        }
    };
    let head = match &args.head {
        Some(head) => head.clone(),
        None => working_copy::current_branch(wc)?,
    };
    let base = match &args.base {
        Some(base) => Some(base.clone()),
        None => wc.scmsync_branch.clone(),
    };
    if let Some(base) = &base {
        refuse_factory_base(base)?;
    }
    let title = args.title.as_deref().ok_or_else(|| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            "--title is required to open a Gitea pull request",
        )
    })?;
    Ok(GitSubmitParts {
        repo,
        origin,
        head,
        base,
        title: title.to_string(),
        body: args.body.clone(),
        draft: args.draft,
    })
}

/// Refuse a pull request onto the `factory` branch: it does not
/// take pull requests. The Factory leg is an OBS submit request,
/// never a Gitea PR — the devel branch takes the PR instead. The
/// match is case-insensitive: the rule names the Factory leg, not
/// a case-sensitive branch spelling, so `Factory` is refused like
/// `factory`.
fn refuse_factory_base(base: &str) -> Result<(), GeectlError> {
    if base.eq_ignore_ascii_case("factory") {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "the factory branch does not take pull requests: target the devel branch with --base instead, and file the Factory leg as an OBS submit request (`osc sr`)",
        ));
    }
    Ok(())
}

/// Qualify a bare head with the fork owner: Gitea resolves a bare
/// head against the base repo, so a bare branch that also exists
/// on the target would file from the target's branch, and one
/// that does not fails head-not-found. A head the caller already
/// qualified (`owner:branch`) passes through untouched.
fn qualify_head(head: &str, owner: &str) -> String {
    if head.contains(':') {
        head.to_string()
    } else {
        format!("{owner}:{head}")
    }
}

/// The Gitea filing target from the submit flags, phase two: the
/// repo defaults to the origin's parent (resolved via `repo_get`
/// on the origin) and the base to the target repo's default branch
/// (via `repo_get`, only when the checkout carries no scmsync
/// branch). The head is qualified with the origin owner
/// (`owner:branch`) whenever the target is not the origin repo:
/// Gitea resolves a bare head against the base repo, so a bare
/// branch that also exists on the target would file from the
/// target's branch — or fail head-not-found when it does not. A
/// parentless origin is not a fork and names no target; a default
/// branch of `factory` is refused like an explicit one.
fn resolve_git_submit_target(
    parts: GitSubmitParts,
    gitea: &GiteaClient,
) -> Result<SubmitTarget, GeectlError> {
    // Destructured up front: the target resolution moves `repo`,
    // `origin` and `base` out of the parts in turn, and the head
    // qualification below still needs the origin and the head.
    let GitSubmitParts {
        repo,
        origin,
        head,
        base,
        title,
        body,
        draft,
    } = parts;
    let (owner, repo) = match repo {
        Some(repo) => repo,
        None => {
            // Unreachable by construction — `git_submit_parts`
            // sets exactly one of `repo`/`origin` — but kept as a
            // defensive error rather than an unwrap: a parts value
            // built any other way must not panic here.
            let (origin_owner, origin_repo) = origin.as_ref().ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::InternalError,
                    "the submit target has no repo and no origin to resolve it from",
                )
            })?;
            let origin = gitea.repo_get(origin_owner, origin_repo)?;
            let parent = origin.parent.as_ref().ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "the checkout's origin {origin_owner}/{origin_repo} is not a fork on this instance: pass --repo to name the repo the pull request goes to"
                    ),
                )
            })?;
            (parent.owner.login.clone(), parent.name.clone())
        }
    };
    let base = match base {
        Some(base) => base,
        None => {
            let target = gitea.repo_get(&owner, &repo)?;
            refuse_factory_base(&target.default_branch)?;
            target.default_branch
        }
    };
    // The head lives on the fork: qualify it with the origin
    // owner whenever the target is not the origin repo itself. A
    // head into the origin repo stays bare — it already names a
    // branch on the target. Without a readable origin there is no
    // fork owner to qualify with and the head stays bare; the
    // target verification re-reads the origin before anything
    // files, so that case never files unqualified.
    let head = match &origin {
        Some((origin_owner, origin_repo)) if origin_owner != &owner || origin_repo != &repo => {
            qualify_head(&head, origin_owner)
        }
        _ => head,
    };
    let mut options = PrCreateOptions::new(&head, &base, &title)?;
    options.body = body;
    options.draft = draft;
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
                args.commit.as_deref(),
                &dry_run,
            )?;
            Ok((
                review_act_payload(&reference, &opinion, args.commit.as_deref(), &dry_run)?,
                review_act_text(
                    present,
                    past,
                    &reference,
                    &opinion,
                    args.message.as_deref(),
                    args.commit.as_deref(),
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
                args.commit.as_deref(),
                &dry_run,
            )?;
            Ok((
                review_act_payload(&reference, &opinion, args.commit.as_deref(), &dry_run)?,
                review_act_text(
                    present,
                    past,
                    &reference,
                    &opinion,
                    args.message.as_deref(),
                    args.commit.as_deref(),
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
    commit: Option<&str>,
    dry_run: &DryRun,
) -> Result<Value, GeectlError> {
    let mut payload = json!({
        "reference": reference.to_string(),
        "review": match opinion {
            Some(opinion) => payload_json(opinion)?,
            None => Value::Null,
        },
        "commit": commit,
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
    commit: Option<&str>,
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
    if let Some(commit) = commit {
        text.push_str(&format!(" at {commit}"));
    }
    text
}

/// Merge an OBS submit request: accept it, then forward it to
/// its link parent. The merge stands on its own: once it lands,
/// the request is accepted, and a forward that is refused — or
/// fails any other way — is reported with the work left queued
/// in the devel project. Nothing is declined on a refusal. When
/// no submit action has a link parent, no forward happens. The
/// forwarded `.changes` entry is the original request's
/// description, as with `request accept`, overridable with
/// `-m` / `--forward-message`.
fn merge_obs(
    debug: &Debug,
    id: &str,
    message: Option<&str>,
    force: bool,
    forward_message: Option<&str>,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let (mut payload, mut text) = level_act_with(
        format!("obs:{id}"),
        debug,
        live,
        "merge",
        "merged",
        message,
        |client, reference, dry_run| client.merge(reference, message, None, force, dry_run),
    )?;
    // The merge landed (or, in dry-run, is planned): clean up the
    // branch home packages it merged, through the shared helper.
    // A cleanup that cannot run is reported, never an error here.
    let (cleanup_payload, cleanup_text) = crate::cli_request::accept_cleanup(debug, id, live);
    if let Value::Object(map) = &mut payload {
        map.insert("cleanup".to_string(), cleanup_payload);
    }
    if !cleanup_text.is_empty() {
        text.push_str(&format!("; {cleanup_text}"));
    }
    if !live {
        let forward_note = match forward_message {
            Some(entry) => format!(" with changes entry: {entry}"),
            None => String::new(),
        };
        if let Value::Object(map) = &mut payload {
            if let Some(Value::Array(plan)) = map.get_mut("plan") {
                plan.push(Value::String(format!(
                    "[dry-run] would: forward request {id} after merge{forward_note}"
                )));
            }
        }
        text.push_str(&format!("; would forward after merge{forward_note}"));
        return Ok((payload, text));
    }
    // Live: the merge landed; forward the now-merged request.
    let (forward_payload, forward_text) =
        crate::cli_request::forward(debug, id, forward_message, live).map_err(|err| {
            GeectlError::new(
                err.kind(),
                format!("merged request {id}, but the forward failed: {err}"),
            )
        })?;
    if let Value::Object(map) = &mut payload {
        map.insert("forward".to_string(), forward_payload);
    }
    text.push_str(&format!("; {forward_text}"));
    Ok((payload, text))
}

/// Merge a Gitea pull request and forward it to its link parent,
/// unless `--no-forward` is given. After the merge lands, the branch
/// home the pull request came from is cleaned up like the OBS arm's:
/// the OBS package synced from the merged head is resolved, and its
/// marker gates the delete. A cleanup that cannot run is reported,
/// never an error. Both arms are resolved and
/// validated before the merge lands (see
/// [`preflight_gitea_forward`]): a missing OBS credential, an
/// ambiguous or unforwardable package, or a refused hop fails
/// with the pull request still open. The forward runs after the
/// merge lands: the merged pull request's target package is
/// resolved from its scmsync record (the base repo and branch),
/// and the hop files like one submit action of the OBS forward —
/// the devel check, the broken-link refusal, the no-link skip,
/// the maintenance incident. A forward refusal after the merge
/// reports the refusal and changes nothing: the merge already
/// landed, and there is no request to decline on this path. A
/// read failure (a 5xx, a timeout, a rate-limit reply) aborts
/// with no filing, and the forward stays retryable by a direct
/// submit from the devel project. In dry-run nothing merges, so
/// nothing is forwarded either — the plan notes the forward that
/// would follow the merge, like the OBS arm; the preflight runs
/// in dry-run too, when OBS credentials are available.
fn merge_gitea(
    debug: &Debug,
    reference: &str,
    message: Option<&str>,
    method: Option<GiteaMergeMethod>,
    no_forward: bool,
    forward_message: Option<&str>,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    // Both arms are resolved and validated before the merge
    // lands: a missing OBS credential, an ambiguous or
    // unforwardable package, or a refused hop fails here, with
    // the pull request still open — never after the merge.
    // Dry-run performs the same preflight without merging, when
    // OBS credentials are available to verify against.
    let preflight = if no_forward {
        None
    } else {
        Some(preflight_gitea_forward(
            debug,
            reference,
            forward_message,
            live,
        )?)
    };
    let (mut payload, mut text) = level_act_with(
        reference.to_string(),
        debug,
        live,
        "merge",
        "merged",
        message,
        |client, reference, dry_run| client.merge(reference, message, method, false, dry_run),
    )?;
    // The merge landed (or, in dry-run, is planned): clean up the
    // branch home it merged, through the shared helper. A cleanup
    // that cannot run is reported, never an error here.
    let (cleanup_payload, cleanup_text) = merge_gitea_cleanup(debug, reference, live);
    if let Value::Object(map) = &mut payload {
        map.insert("cleanup".to_string(), cleanup_payload);
    }
    if !cleanup_text.is_empty() {
        text.push_str(&format!("; {cleanup_text}"));
    }
    if no_forward {
        return Ok((payload, text));
    }
    if !live {
        let forward_note = match forward_message {
            Some(entry) => format!(" with changes entry: {entry}"),
            None => String::new(),
        };
        if let Value::Object(map) = &mut payload {
            if let Some(Value::Array(plan)) = map.get_mut("plan") {
                plan.push(Value::String(format!(
                    "[dry-run] would: forward {reference} after merge{forward_note}"
                )));
                if matches!(preflight, Some(ForwardPreflight::SkippedNoObsCredentials)) {
                    plan.push(Value::String(
                        "[dry-run] note: forward preflight skipped: no OBS credentials, so the forward was not verified".to_string(),
                    ));
                }
            }
        }
        text.push_str(&format!("; would forward after merge{forward_note}"));
        return Ok((payload, text));
    }
    // Live: the merge landed; forward the merged pull request.
    let (forward_payload, forward_text) = forward_gitea(debug, reference, forward_message)
        .map_err(|err| {
            GeectlError::new(
                err.kind(),
                format!("merged {reference}, but the forward failed: {err}"),
            )
        })?;
    if let Value::Object(map) = &mut payload {
        map.insert("forward".to_string(), forward_payload);
    }
    text.push_str(&format!("; {forward_text}"));
    Ok((payload, text))
}

/// Run the merge-time branch cleanup for the Gitea arm: resolve the
/// OBS package synced from the merged pull request's head (the
/// fork), then clean up that branch home through the shared
/// per-source helper. The merge stands regardless: a cleanup that
/// cannot run at all (no OBS credentials, the fork deleted, the
/// head repo unreadable, an ambiguous sync) is reported as a
/// payload with an `error` and the one-line text, never an error
/// from this call — like the OBS arm's `accept_cleanup`. A pull
/// request whose head syncs no OBS package (not a geectl branch)
/// has nothing to clean up: empty outcomes, silent in text, the
/// payload records the empty result.
fn merge_gitea_cleanup(debug: &Debug, reference: &str, live: bool) -> (Value, String) {
    let dry_run = DryRun::new(!live).with_debug(debug.clone());
    let not_run = |err: GeectlError| -> (Value, String) {
        (
            json!({
                "id": reference,
                "dry_run": dry_run.enabled(),
                "outcomes": [],
                "error": err.to_string(),
            }),
            "branch cleanup not run".to_string(),
        )
    };
    // OBS credentials first: without them there is nothing to
    // resolve against, and the Gitea reads must not run either — a
    // credential-less dry-run prints the plan without a server.
    // Live, the merge already landed, so missing credentials report
    // instead of failing: the merge stands either way.
    let obs_ready = match with_obs(debug, live, |obs| {
        Ok(matches!(
            obs.credentials(),
            Credentials::Basic { .. } | Credentials::Bearer { .. }
        ))
    }) {
        Ok(ready) => ready,
        Err(err) => return not_run(err),
    };
    if !obs_ready {
        debug.emit("decision: skipping the merge-time branch cleanup: no OBS credentials in dry-run, so the home package cannot be resolved");
        return not_run(GeectlError::new(
            ErrorKind::ConfigError,
            "cannot clean up the branch home without OBS credentials (GEECTL_OBS_APIKEY[_FILE] or GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD[_FILE])",
        ));
    }
    let head = match gitea_merge_head_inputs(debug, reference) {
        Ok(Some(head)) => head,
        Ok(None) => {
            debug.emit("decision: skipping the merge-time branch cleanup: the merged pull request's head repo is gone, so no branch home can be resolved");
            return (
                merge_gitea_cleanup_json(reference, &[], &dry_run),
                String::new(),
            );
        }
        Err(err) => return not_run(err),
    };
    let outcome = match with_obs(debug, live, |obs| {
        let package = resolve_forward_package(
            obs,
            &head.repo_url,
            &head.owner,
            &head.repo,
            &head.branch,
            &head.default_branch,
        )?;
        match package {
            Some((project, package)) => Ok(Some(
                obs.cleanup_branch_source(&project, &package, &dry_run),
            )),
            None => {
                debug.emit(&format!(
                    "decision: skipping the merge-time branch cleanup: no OBS package is synced from {}#{}",
                    head.repo_url, head.branch
                ));
                Ok(None)
            }
        }
    }) {
        Ok(outcome) => outcome,
        Err(err) => return not_run(err),
    };
    let outcomes = outcome.into_iter().collect::<Vec<_>>();
    (
        merge_gitea_cleanup_json(reference, &outcomes, &dry_run),
        merge_gitea_cleanup_text(&outcomes, &dry_run),
    )
}

/// The head side of a merged Gitea pull request: the fork repo and
/// branch, and the repo's clone URL and default branch — the
/// coordinates the merge-time cleanup resolves the branch home
/// from, through the same scmsync lookup the forward uses for the
/// base side.
struct GiteaMergeHeadInputs {
    owner: String,
    repo: String,
    branch: String,
    default_branch: String,
    repo_url: String,
}

/// Read the merged pull request's head: the fork repo and branch,
/// plus the head repo's clone URL and default branch. `None` when
/// the fork the branch lived in is deleted — the pull request
/// keeps its head label, but there is no repo to resolve a branch
/// home from, so the cleanup has nothing to clean up.
fn gitea_merge_head_inputs(
    debug: &Debug,
    reference: &str,
) -> Result<Option<GiteaMergeHeadInputs>, GeectlError> {
    let reference: PullRequestRef = reference.parse()?;
    let (owner, repo, number) = match &reference {
        PullRequestRef::Gitea {
            owner,
            repo,
            number,
        } => (owner.clone(), repo.clone(), *number),
        PullRequestRef::Obs(_) => {
            return Err(GeectlError::new(
                ErrorKind::InternalError,
                "gitea_merge_head_inputs called on an OBS reference",
            ));
        }
    };
    let (pr, head) = with_gitea(debug, |gitea| {
        let pr = gitea.pr_get(&owner, &repo, number)?;
        let head = match pr.head.repo.as_ref() {
            Some(head) => {
                let (head_owner, head_name) =
                    parse_gitea_full_name(&head.full_name, &reference.to_string())?;
                let repo = gitea.repo_get(&head_owner, &head_name)?;
                Some((head_owner, head_name, repo))
            }
            None => None,
        };
        Ok((pr, head))
    })?;
    let Some((head_owner, head_name, head_repo)) = head else {
        return Ok(None);
    };
    let repo_url = head_repo
        .clone_url
        .strip_suffix(".git")
        .unwrap_or(&head_repo.clone_url);
    Ok(Some(GiteaMergeHeadInputs {
        owner: head_owner,
        repo: head_name,
        branch: pr.head.branch.clone(),
        default_branch: head_repo.default_branch,
        repo_url: repo_url.to_string(),
    }))
}

/// The Gitea-arm merge-time cleanup as JSON: the pull-request
/// reference, one outcome for the resolved branch home, and the
/// dry-run plan when dry-running. Mirrors the OBS arm's
/// `accept_cleanup` payload, with the reference in place of the
/// request id.
fn merge_gitea_cleanup_json(
    reference: &str,
    outcomes: &[AcceptCleanupOutcome],
    dry_run: &DryRun,
) -> Value {
    let mut payload = json!({
        "id": reference,
        "dry_run": dry_run.enabled(),
        "outcomes": outcomes
            .iter()
            .map(|outcome| json!({
                "source": format!("{}/{}", outcome.source_project, outcome.source_package),
                "package": crate::cli_request::cleanup_step_json(&outcome.package),
                "project": crate::cli_request::cleanup_step_json(&outcome.project),
            }))
            .collect::<Vec<_>>(),
    });
    add_plan(&mut payload, dry_run);
    payload
}

/// The Gitea-arm merge-time cleanup as text: silent when the
/// outcome was a plain no-marker skip — most merges are not branch
/// merges — and when nothing was resolved at all. The payload
/// records every outcome either way.
fn merge_gitea_cleanup_text(outcomes: &[AcceptCleanupOutcome], dry_run: &DryRun) -> String {
    let reportable: Vec<_> = outcomes
        .iter()
        .filter(|outcome| outcome.package.report_in_text())
        .collect();
    if reportable.is_empty() {
        return String::new();
    }
    let mut text = if dry_run.enabled() {
        "dry-run: branch cleanup:".to_string()
    } else {
        "branch cleanup:".to_string()
    };
    for outcome in reportable {
        text.push('\n');
        text.push_str(&crate::cli_request::branch_cleanup_line(
            &outcome.source_project,
            &outcome.source_package,
            &outcome.package,
            &outcome.project,
            dry_run,
        ));
    }
    text
}

/// The Gitea side of the forward, resolved once and shared by the
/// preflight and the forward itself, so the two cannot disagree on
/// what the merge targeted: the merged pull request's base repo and
/// branch, the repo's clone URL and default branch, and the forward
/// description and `.changes` entry carried from the pull request's
/// text — or the `-m` / `--forward-message` override for the
/// `.changes` entry.
struct GiteaForwardInputs {
    base_owner: String,
    base_repo: String,
    base_branch: String,
    default_branch: String,
    repo_url: String,
    description: String,
    changes_entry: String,
}

/// Read the merged pull request and its repo: the base repo and
/// branch name the scmsync record, the repo's clone URL spells
/// the repo URL, and its default branch is the branch a scmsync
/// URL without one tracks.
fn gitea_forward_inputs(
    debug: &Debug,
    reference: &str,
    forward_message: Option<&str>,
) -> Result<GiteaForwardInputs, GeectlError> {
    let reference: PullRequestRef = reference.parse()?;
    let (owner, repo, number) = match &reference {
        PullRequestRef::Gitea {
            owner,
            repo,
            number,
        } => (owner.clone(), repo.clone(), *number),
        PullRequestRef::Obs(_) => {
            return Err(GeectlError::new(
                ErrorKind::InternalError,
                "forward_gitea called on an OBS reference",
            ));
        }
    };
    let (pr, default_branch, clone_url) = with_gitea(debug, |gitea| {
        let pr = gitea.pr_get(&owner, &repo, number)?;
        let repo = gitea.repo_get(&owner, &repo)?;
        Ok((pr, repo.default_branch, repo.clone_url))
    })?;
    let base_repo = pr.base.repo.as_ref().ok_or_else(|| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("the merged pull request {reference} names no base repository"),
        )
    })?;
    let (base_owner, base_repo_name) =
        parse_gitea_full_name(&base_repo.full_name, &reference.to_string())?;
    let base_branch = pr.base.branch.clone();
    let repo_url = clone_url.strip_suffix(".git").unwrap_or(&clone_url);
    // The forward description and `.changes` entry carry the pull
    // request's text: the body when it has one, else the title,
    // else the forward line alone. An explicit `-m` override
    // replaces the `.changes` entry only: the description still
    // carries the pull request's text.
    let forward_line = format!(
        "Forwarded Gitea pull request {base_owner}/{base_repo_name}#{number} from {}",
        pr.user.login
    );
    let body = pr.body.as_deref().unwrap_or("").trim();
    let title = pr.title.trim();
    let description = if !body.is_empty() {
        format!("{forward_line}\n\n{body}")
    } else if !title.is_empty() {
        format!("{forward_line}\n\n{title}")
    } else {
        forward_line.clone()
    };
    let changes_entry = match forward_message {
        Some(entry) => entry.to_string(),
        None if !body.is_empty() => body.to_string(),
        None if !title.is_empty() => title.to_string(),
        None => forward_line,
    };
    Ok(GiteaForwardInputs {
        base_owner,
        base_repo: base_repo_name,
        base_branch,
        default_branch,
        repo_url: repo_url.to_string(),
        description,
        changes_entry,
    })
}

/// Split a Gitea `full_name` into its owner and repo: exactly one
/// non-empty owner/repo pair. Anything else is malformed server
/// data — a merged pull request always names its base repo this
/// way — so it maps to `ServerError`, never `InvalidInput`: the
/// caller did not supply the name.
fn parse_gitea_full_name(
    full_name: &str,
    reference: &str,
) -> Result<(String, String), GeectlError> {
    let mut parts = full_name.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(repo), None) if !owner.is_empty() && !repo.is_empty() => {
            Ok((owner.to_string(), repo.to_string()))
        }
        _ => Err(GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "the merged pull request {reference} names a malformed base repository {full_name:?}: expected one owner/repo pair"
            ),
        )),
    }
}

/// Whether the Gitea forward preflight verified the forward, or
/// had nothing to verify against.
enum ForwardPreflight {
    /// Both arms resolved and the hop verified: the forward
    /// would file, be skipped, or refuse for a reason the
    /// preflight already reported.
    Verified,
    /// Dry-run only, and only when no OBS credentials are
    /// available: there is nothing to verify the forward
    /// against, so the dry-run plan notes the forward
    /// unverified. A live merge never skips: without
    /// credentials it fails here instead.
    SkippedNoObsCredentials,
}

/// Resolve and validate both arms of the Gitea forward before the
/// merge lands: the pull request and its repo on Gitea, then the
/// OBS package synced from the base repo and branch and its hop
/// to the link parent. The hop is verified by running the forward
/// itself in dry-run: it resolves and validates everything the
/// live forward would, and files nothing. A failure here — a
/// missing credential, an ambiguous package, a refused hop —
/// aborts before the merge, with the pull request still open.
/// Dry-run performs the same preflight without merging, when OBS
/// credentials are available; without them the preflight is
/// skipped (there is nothing to verify against) and the caller
/// notes the forward unverified in the plan.
fn preflight_gitea_forward(
    debug: &Debug,
    reference: &str,
    forward_message: Option<&str>,
    live: bool,
) -> Result<ForwardPreflight, GeectlError> {
    // OBS credentials first: without them there is nothing to
    // verify against, and the Gitea reads must not run either — a
    // credential-less dry-run prints the plan without a server.
    // Live, the resolution fails fast on missing credentials, so
    // a live merge never reaches the skip below.
    let obs_ready = with_obs(debug, live, |obs| {
        Ok(matches!(
            obs.credentials(),
            Credentials::Basic { .. } | Credentials::Bearer { .. }
        ))
    })?;
    if !obs_ready {
        if live {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot merge a Gitea pull request without OBS credentials (GEECTL_OBS_APIKEY[_FILE] or GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD[_FILE]): the forward cannot be validated before the merge",
            ));
        }
        debug.emit("decision: skipping the forward preflight: no OBS credentials in dry-run, so the forward is planned unverified");
        return Ok(ForwardPreflight::SkippedNoObsCredentials);
    }
    let inputs = gitea_forward_inputs(debug, reference, forward_message)?;
    let dry_run = DryRun::new(true).with_debug(debug.clone());
    with_obs(debug, live, |obs| {
        let package = resolve_forward_package(
            obs,
            &inputs.repo_url,
            &inputs.base_owner,
            &inputs.base_repo,
            &inputs.base_branch,
            &inputs.default_branch,
        )?;
        if let Some((project, package)) = package {
            obs.forward_package(
                &project,
                &package,
                &inputs.description,
                &inputs.changes_entry,
                &dry_run,
            )?;
        }
        Ok(())
    })?;
    Ok(ForwardPreflight::Verified)
}

/// Forward a merged Gitea pull request: resolve the OBS package
/// synced from the base repo and branch, and forward that package
/// to its link parent. The payload carries the single forward
/// action, like one entry of the accept-coupled forward's result. The
/// pull request and repo come from [`gitea_forward_inputs`],
/// shared with the preflight, so the forward cannot disagree
/// with what was validated before the merge.
fn forward_gitea(
    debug: &Debug,
    reference: &str,
    forward_message: Option<&str>,
) -> Result<(Value, String), GeectlError> {
    let inputs = gitea_forward_inputs(debug, reference, forward_message)?;
    let dry_run = DryRun::new(false).with_debug(debug.clone());
    let action = with_obs(debug, true, |obs| {
        let package = resolve_forward_package(
            obs,
            &inputs.repo_url,
            &inputs.base_owner,
            &inputs.base_repo,
            &inputs.base_branch,
            &inputs.default_branch,
        )?;
        match package {
            Some((project, package)) => obs.forward_package(
                &project,
                &package,
                &inputs.description,
                &inputs.changes_entry,
                &dry_run,
            ),
            None => Ok(ForwardActionResult::Skipped {
                source: format!(
                    "{}/{}#{}",
                    inputs.base_owner, inputs.base_repo, inputs.base_branch
                ),
                reason: format!(
                    "no OBS package is synced from {}#{}",
                    inputs.repo_url, inputs.base_branch
                ),
            }),
        }
    })?;
    let payload = json!({
        "actions": [crate::cli_request::forward_action_json(&action)],
        "dry_run": false,
    });
    let text = crate::cli_request::forward_action_text(&action, &dry_run);
    Ok((payload, text))
}

/// Resolve the OBS package synced from a Gitea repo and branch:
/// search OBS packages by the scmsync repo URL, then verify each
/// hit against its own `_meta` scmsync — the search is a prefix
/// match, so a repo name that prefixes another matches both, and
/// only a hit whose parsed scmsync names this repo and branch is
/// the package. A hit with no branch named tracks the repo's
/// default branch; a hit whose fragment pins a commit still
/// tracks the branch its `trackingbranch` names. No hit is `None`
/// (nothing to forward); more than one distinct package is an
/// error — the forward must not guess which one the merge
/// targeted. A hit whose scmsync does not parse is skipped, not
/// an error: it cannot be verified as this repo, and the search
/// prefix alone does not prove it is.
fn resolve_forward_package(
    obs: &ObsClient,
    repo_url: &str,
    owner: &str,
    repo: &str,
    branch: &str,
    default_branch: &str,
) -> Result<Option<(String, String)>, GeectlError> {
    let candidates = obs.search_packages_by_scmsync(repo_url)?;
    let mut matched: Vec<(String, String)> = Vec::new();
    for candidate in &candidates {
        let meta = obs.package_meta_authenticated(&candidate.project, &candidate.name)?;
        let Some(scmsync) = &meta.scmsync else {
            continue;
        };
        let Ok(parsed) = parse_scmsync_url(scmsync) else {
            continue;
        };
        if parsed.owner != owner || parsed.repo != repo {
            continue;
        }
        if !scmsync_branch_matches(&parsed, branch, default_branch) {
            continue;
        }
        let found = (candidate.project.clone(), candidate.name.clone());
        if !matched.contains(&found) {
            matched.push(found);
        }
    }
    match matched.len() {
        0 => Ok(None),
        1 => Ok(matched.into_iter().next()),
        _ => Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!(
                "more than one OBS package is synced from {repo_url}#{branch}: {}; refusing to guess which one the merge targeted",
                matched
                    .iter()
                    .map(|(project, package)| format!("{project}/{package}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

/// Whether `value` pins a commit rather than naming a branch: a
/// full SHA-1 (40 hex) or SHA-256 (64 hex) — the shapes the server
/// writes into a scmsync fragment when it pins a revision. A branch
/// literally named 40 or 64 hex digits would read as a pin; no
/// such branch exists in practice, and the server's own fixtures
/// pin full SHAs.
fn is_commit_pin(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether a parsed scmsync URL tracks `branch`: the fragment
/// wins — when it names a branch, only that branch matches, and
/// the tracking branch does not. When the fragment pins a commit
/// instead, it names no branch at all: only the tracking branch
/// matches. With neither a fragment nor a tracking branch, the
/// package tracks the repo's default branch.
fn scmsync_branch_matches(parsed: &ScmsyncUrl, branch: &str, default_branch: &str) -> bool {
    match &parsed.branch {
        Some(fragment) if is_commit_pin(fragment) => {
            parsed.tracking_branch.as_deref() == Some(branch)
        }
        Some(fragment) => fragment.as_str() == branch,
        None => branch == default_branch,
    }
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

/// The payload for a build-link comment: the reference, the
/// test project and package, the link, the created comment
/// (`null` in dry-run), and `dry_run`, plus the plan in dry-run.
fn build_comment_payload(
    reference: &PullRequestRef,
    project: &str,
    package: &str,
    link: &str,
    comment: &Option<ReviewComment>,
    dry_run: &DryRun,
) -> Result<Value, GeectlError> {
    let mut payload = json!({
        "reference": reference.to_string(),
        "project": project,
        "package": package,
        "link": link,
        "comment": match comment {
            Some(comment) => payload_json(comment)?,
            None => Value::Null,
        },
        "dry_run": dry_run.enabled(),
    });
    add_plan(&mut payload, dry_run);
    Ok(payload)
}

/// The text for a build-link comment: the header first — the
/// reference and the test project/package — then the posted
/// comment body in full, so a dry-run shows the comment that
/// would be posted.
fn build_comment_text(
    reference: &PullRequestRef,
    project: &str,
    package: &str,
    comment: &Option<ReviewComment>,
    body: &str,
    dry_run: &DryRun,
) -> String {
    if dry_run.enabled() {
        format!(
            "dry-run: would post the build-link comment on {reference} for {project}/{package}:\n{body}"
        )
    } else {
        match comment {
            Some(comment) => format!(
                "posted the build-link comment on {reference} (comment {} by {}) for {project}/{package}:\n{body}",
                comment.id, comment.author
            ),
            None => format!(
                "posted the build-link comment on {reference} for {project}/{package}:\n{body}"
            ),
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
        line.push_str(&format!(": {}", strip_terminal_controls(&comment.body)));
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
/// Read the diff of a pull request: a content command like
/// `request diff` — the payload carries the reference, the diff
/// and the xml flag, and the text rendering is the diff verbatim.
/// The one exception is an empty diff, which renders an explicit
/// line naming the pull request instead of empty stdout.
fn diff_with(
    client: &PullRequestClient,
    reference: &PullRequestRef,
    xml: bool,
) -> Result<(Value, String), GeectlError> {
    let diff = client.diff(reference, xml)?;
    let payload = json!({
        "reference": reference.to_string(),
        "diff": diff,
        "xml": xml,
    });
    Ok((payload, render_diff_text(reference, &diff)))
}

fn render_diff_text(reference: &PullRequestRef, diff: &str) -> String {
    if diff.is_empty() {
        format!("no diff for pull request {reference}")
    } else {
        diff.to_string()
    }
}

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
        lines.push(format!("title: {}", strip_terminal_controls(&pr.title)));
    }
    lines.push(format!("author: {}", pr.author));
    lines.push(format!("state: {}", pr.status.as_str()));
    if !pr.description.is_empty() {
        lines.push(format!(
            "description: {}",
            strip_terminal_controls(&pr.description)
        ));
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
            lines.push(format!(
                "review {} comment: {}",
                index + 1,
                strip_terminal_controls(&review.comment)
            ));
        }
        if let Some(when) = &review.when {
            lines.push(format!("review {} at: {}", index + 1, when));
        }
    }
    if let Some(error) = &pr.reviews_error {
        lines.push(format!("reviews unavailable: {error}"));
    }
    if !pr.labels.is_empty() {
        lines.push(format!(
            "labels: {}",
            pr.labels
                .iter()
                .map(|label| strip_terminal_controls(label))
                .collect::<Vec<_>>()
                .join(", ")
        ));
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
    use crate::gitea::{GiteaPullRequest, GiteaReview};
    use crate::http::HttpClient;
    use crate::obs::parse_request;

    const REQUEST_XML: &str = r#"<request creator="creator" id="123">
  <action type="submit">
    <source package="abc" project="xyz" />
    <options>
      <sourceupdate>cleanup</sourceupdate>
      <updatelink>1</updatelink>
    </options>
    <acceptinfo rev="5" srcmd5="deadbeefdeadbeefdeadbeefdeadbeef" />
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
    fn gitea_mapping_strips_terminal_controls_from_free_text_fields() {
        // The Gitea arm maps server text into the unified PR: title,
        // body, review comments, and label names are stripped at the
        // mapping, so no downstream surface can print them.
        let mut pr: GiteaPullRequest = serde_json::from_str(PR_JSON).expect("fixture PR parses");
        pr.title = "ti\u{1b}[2Jtle".to_string();
        pr.body = Some("bo\u{7}dy\nline".to_string());
        pr.labels[0].name = "la\u{202e}bel".to_string();
        let mut reviews: Vec<GiteaReview> =
            serde_json::from_str(REVIEWS_JSON).expect("fixture reviews parse");
        reviews[0].body = Some("re\u{1b}]8;;http://x\u{7}view".to_string());
        let pr =
            PullRequest::from_gitea("openSUSE", "geectl", &pr, &reviews).expect("fixture PR maps");
        assert_eq!(pr.title, "ti[2Jtle");
        assert_eq!(pr.description, "body\nline");
        assert_eq!(pr.labels, vec!["label".to_string()]);
        assert_eq!(pr.reviews[0].comment, "re]8;;http://xview");
    }

    #[test]
    fn pull_request_text_strips_terminal_controls_from_free_text_fields() {
        // Free text from the server reaches the terminal through the
        // PR render: escape sequences, carriage returns, bells, and
        // bidirectional marks are stripped, while newlines survive.
        let mut pr = fixture_gitea_pull_request();
        pr.title = "ti\u{1b}[2Jtle".to_string();
        pr.description = "de\u{7}sc\nription".to_string();
        pr.reviews[0].comment = "re\u{202e}view".to_string();
        pr.labels = vec!["la\rbel".to_string()];
        let text = render_pull_request_text(&pr);
        assert!(!text.contains('\u{1b}'), "text: {text}");
        assert!(!text.contains('\u{7}'), "text: {text}");
        assert!(!text.contains('\r'), "text: {text}");
        assert!(!text.contains('\u{202e}'), "text: {text}");
        assert!(text.contains("title: ti[2Jtle"), "text: {text}");
        assert!(text.contains("description: desc\nription"), "text: {text}");
        assert!(text.contains("review 1 comment: review"), "text: {text}");
        assert!(text.contains("labels: label"), "text: {text}");
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
            state: crate::pull_request::OpinionState::Approved,
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
        let payload = review_act_payload(&reference, &opinion, None, &dry_run).unwrap();
        assert_eq!(payload["reference"], "gitea:openSUSE/geectl#122");
        assert_eq!(payload["review"]["reviewer"], "plusky");
        assert_eq!(payload["review"]["state"], "approved");
        assert_eq!(payload["review"]["comment"], "Looks good.");
        assert_eq!(payload["review"]["when"], "2026-11-22T19:52:58Z");
        assert_eq!(payload["dry_run"], false);
        assert_eq!(payload["commit"], serde_json::Value::Null);
        assert!(payload.get("plan").is_none());

        let text = review_act_text(
            "approve",
            "approved",
            &reference,
            &opinion,
            Some("Looks good."),
            None,
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
        let payload = review_act_payload(&reference, &None, None, &dry_run).unwrap();
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
            None,
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
    fn build_comment_json_and_text_agree() {
        let reference = gitea_reference();
        let comment = Some(fixture_comment());
        let link = "https://build.opensuse.org/package/show/home:test/foo";
        let body = "Build results for home:test/foo: https://build.opensuse.org/package/show/home:test/foo";
        let dry_run = DryRun::new(false);
        let payload =
            build_comment_payload(&reference, "home:test", "foo", link, &comment, &dry_run)
                .unwrap();
        assert_eq!(payload["reference"], "gitea:openSUSE/geectl#122");
        assert_eq!(payload["project"], "home:test");
        assert_eq!(payload["package"], "foo");
        assert_eq!(payload["link"], link);
        assert_eq!(payload["comment"]["id"], 7);
        assert_eq!(payload["comment"]["author"], "plusky");
        assert_eq!(payload["comment"]["body"], "One nit.");
        assert_eq!(payload["dry_run"], false);

        let text = build_comment_text(&reference, "home:test", "foo", &comment, body, &dry_run);
        for expected in [
            "gitea:openSUSE/geectl#122",
            "comment 7",
            "plusky",
            "Build results for home:test/foo",
        ] {
            assert!(
                text.contains(expected),
                "text missing {expected:?}:\n{text}"
            );
        }

        let dry_run = DryRun::new(true);
        dry_run.allow("comment on pull request openSUSE/geectl#122");
        let payload =
            build_comment_payload(&reference, "home:test", "foo", link, &None, &dry_run).unwrap();
        assert_eq!(payload["comment"], Value::Null);
        assert_eq!(payload["dry_run"], true);
        assert_eq!(
            payload["plan"][0],
            "[dry-run] would: comment on pull request openSUSE/geectl#122"
        );

        let text = build_comment_text(&reference, "home:test", "foo", &None, body, &dry_run);
        assert!(
            text.starts_with(
                "dry-run: would post the build-link comment on gitea:openSUSE/geectl#122 for home:test/foo:"
            ),
            "text: {text}"
        );
        assert!(text.contains(body), "text: {text}");
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
    fn comments_text_strips_terminal_controls_from_bodies() {
        // Comment bodies are free text from another user: escape
        // sequences, bells, and bidirectional marks are stripped at
        // the render.
        let reference = gitea_reference();
        let mut comment = fixture_comment();
        comment.body = "fi\u{1b}[2Jx\u{7}ed\nline\u{202e}two".to_string();
        let text = comments_text(&reference, &[comment]);
        assert!(!text.contains('\u{1b}'), "text: {text}");
        assert!(!text.contains('\u{7}'), "text: {text}");
        assert!(!text.contains('\u{202e}'), "text: {text}");
        assert!(
            text.contains("comment 7 by plusky at 2026-11-22T19:52:58Z: fi[2Jxed\nlinetwo"),
            "text: {text}"
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

    /// Gitea submit flags with the required title set and nothing
    /// else: the target and base must default.
    fn gitea_submit_args() -> SubmitArgs {
        SubmitArgs {
            target_project: None,
            target_package: None,
            description: None,
            changes_entry: None,
            cve: Vec::new(),
            without_cleanup: false,
            repo: None,
            head: None,
            base: None,
            title: Some("Update foo".to_string()),
            body: None,
            draft: false,
        }
    }

    /// A git-backed checkout for the submit-parts tests: a repo
    /// with an origin on the default instance and the record
    /// written by hand, like the `origin_repo` tests in the
    /// working-copy layer. The origin need not be reachable —
    /// phase one reads only local state.
    fn git_parts_checkout(
        scmsync_branch: Option<&str>,
    ) -> (tempfile::TempDir, working_copy::WorkingCopy) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .current_dir(root)
                .args(args)
                .output()
                .unwrap()
        };
        assert!(git(&["init", "--quiet", "-b", "main"]).status.success());
        assert!(git(&["config", "--local", "user.name", "geectl"])
            .status
            .success());
        assert!(
            git(&["config", "--local", "user.email", "geectl@localhost"])
                .status
                .success()
        );
        std::fs::write(root.join("foo.spec"), "Name: foo\n").unwrap();
        assert!(git(&["add", "-A"]).status.success());
        assert!(git(&["commit", "--quiet", "-m", "initial"])
            .status
            .success());
        let head = String::from_utf8(git(&["rev-parse", "HEAD"]).stdout).unwrap();
        let head = head.trim().to_string();
        for (key, value) in [
            (
                "remote.origin.url",
                "https://src.opensuse.org/scarabeus_iv/foo",
            ),
            ("geectl.apiurl", "https://api.opensuse.org"),
            ("geectl.project", "home:alice"),
            ("geectl.package", "foo"),
            ("geectl.backend", "git"),
            ("geectl.base-rev", head.as_str()),
            ("geectl.base-srcmd5", ""),
        ] {
            assert!(git(&["config", "--local", key, value]).status.success());
        }
        if let Some(branch) = scmsync_branch {
            assert!(git(&["config", "--local", "geectl.scmsync-branch", branch])
                .status
                .success());
        }
        let wc = working_copy::open(root).unwrap();
        (dir, wc)
    }

    #[test]
    fn git_submit_parts_defaults_base_from_the_scmsync_branch() {
        let (_dir, wc) = git_parts_checkout(Some("devel"));
        let args = gitea_submit_args();
        let parts = git_submit_parts(&args, &wc, "https://src.opensuse.org").unwrap();
        assert_eq!(parts.repo, None);
        assert_eq!(
            parts.origin,
            Some(("scarabeus_iv".to_string(), "foo".to_string()))
        );
        assert_eq!(parts.head, "main");
        assert_eq!(parts.base.as_deref(), Some("devel"));
        assert_eq!(parts.title, "Update foo");
    }

    #[test]
    fn git_submit_parts_explicit_repo_and_base_pass_through() {
        let (_dir, wc) = git_parts_checkout(Some("devel"));
        let mut args = gitea_submit_args();
        args.repo = Some("pool/foo".to_string());
        args.base = Some("main".to_string());
        let parts = git_submit_parts(&args, &wc, "https://src.opensuse.org").unwrap();
        assert_eq!(parts.repo, Some(("pool".to_string(), "foo".to_string())));
        // Phase one reads the origin even with an explicit `--repo`:
        // phase two qualifies the head with it.
        assert_eq!(
            parts.origin,
            Some(("scarabeus_iv".to_string(), "foo".to_string()))
        );
        assert_eq!(parts.head, "main");
        assert_eq!(parts.base.as_deref(), Some("main"));
    }

    #[test]
    fn git_submit_parts_refuses_an_explicit_factory_base() {
        let (_dir, wc) = git_parts_checkout(None);
        let mut args = gitea_submit_args();
        args.base = Some("factory".to_string());
        let err = git_submit_parts(&args, &wc, "https://src.opensuse.org").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(
            err.to_string()
                .contains("the factory branch does not take pull requests"),
            "err: {err}"
        );
        assert!(err.to_string().contains("osc sr"), "err: {err}");
    }

    #[test]
    fn git_submit_parts_refuses_a_factory_base_case_insensitively() {
        let (_dir, wc) = git_parts_checkout(None);
        let mut args = gitea_submit_args();
        args.base = Some("Factory".to_string());
        let err = git_submit_parts(&args, &wc, "https://src.opensuse.org").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(
            err.to_string()
                .contains("the factory branch does not take pull requests"),
            "err: {err}"
        );
    }

    #[test]
    fn git_submit_parts_refuses_a_factory_scmsync_branch() {
        let (_dir, wc) = git_parts_checkout(Some("factory"));
        let args = gitea_submit_args();
        let err = git_submit_parts(&args, &wc, "https://src.opensuse.org").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(
            err.to_string()
                .contains("the factory branch does not take pull requests"),
            "err: {err}"
        );
    }

    #[test]
    fn git_submit_parts_requires_a_title() {
        let (_dir, wc) = git_parts_checkout(None);
        let mut args = gitea_submit_args();
        args.title = None;
        let err = git_submit_parts(&args, &wc, "https://src.opensuse.org").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(
            err.to_string().contains("--title is required"),
            "err: {err}"
        );
    }

    #[test]
    fn git_submit_parts_without_a_base_leaves_it_for_phase_two() {
        let (_dir, wc) = git_parts_checkout(None);
        let args = gitea_submit_args();
        let parts = git_submit_parts(&args, &wc, "https://src.opensuse.org").unwrap();
        assert_eq!(parts.base, None);
    }

    /// A read-only HTTP fake for the phase-two resolve tests:
    /// canned GET bodies by URL, every mutation refused — resolve
    /// only reads. The lib's `FakeHttp` cannot serve here: it is
    /// `pub(crate)` to the library, and the CLI is the binary.
    struct ResolveHttp {
        gets: std::collections::HashMap<String, String>,
    }

    impl ResolveHttp {
        fn with_get(url: &str, body: &str) -> Self {
            ResolveHttp {
                gets: std::collections::HashMap::from([(url.to_string(), body.to_string())]),
            }
        }

        fn and_get(mut self, url: &str, body: &str) -> Self {
            self.gets.insert(url.to_string(), body.to_string());
            self
        }
    }

    /// Refuse a mutation the read-only resolve fake never serves.
    fn refused(op: &str) -> GeectlError {
        GeectlError::new(
            ErrorKind::InternalError,
            format!("test fake does not implement {op}"),
        )
    }

    impl HttpClient for ResolveHttp {
        fn get(&self, url: &str) -> Result<String, GeectlError> {
            self.get_with(url, &Credentials::None)
        }

        fn get_with(&self, url: &str, _credentials: &Credentials) -> Result<String, GeectlError> {
            self.gets.get(url).cloned().ok_or_else(|| {
                GeectlError::new(ErrorKind::InternalError, format!("no canned GET for {url}"))
            })
        }

        fn post(
            &self,
            _url: &str,
            _body: &str,
            _credentials: &Credentials,
        ) -> Result<String, GeectlError> {
            Err(refused("post"))
        }

        fn post_json(&self, _url: &str, _body: &str) -> Result<String, GeectlError> {
            Err(refused("post_json"))
        }

        fn post_json_with(
            &self,
            _url: &str,
            _body: &str,
            _credentials: &Credentials,
        ) -> Result<String, GeectlError> {
            Err(refused("post_json_with"))
        }

        fn put_json_with(
            &self,
            _url: &str,
            _body: &str,
            _credentials: &Credentials,
        ) -> Result<String, GeectlError> {
            Err(refused("put_json_with"))
        }

        fn patch_json_with(
            &self,
            _url: &str,
            _body: &str,
            _credentials: &Credentials,
        ) -> Result<String, GeectlError> {
            Err(refused("patch_json_with"))
        }

        fn post_text(
            &self,
            _url: &str,
            _body: &str,
            _credentials: &Credentials,
        ) -> Result<String, GeectlError> {
            Err(refused("post_text"))
        }

        fn put(
            &self,
            _url: &str,
            _body: &str,
            _credentials: &Credentials,
        ) -> Result<String, GeectlError> {
            Err(refused("put"))
        }

        fn put_file(
            &self,
            _url: &str,
            _body: &str,
            _credentials: &Credentials,
        ) -> Result<String, GeectlError> {
            Err(refused("put_file"))
        }

        fn delete(&self, _url: &str, _credentials: &Credentials) -> Result<String, GeectlError> {
            Err(refused("delete"))
        }

        fn delete_json_with(
            &self,
            _url: &str,
            _body: &str,
            _credentials: &Credentials,
        ) -> Result<String, GeectlError> {
            Err(refused("delete_json_with"))
        }
    }

    /// A repo JSON body, shaped like Gitea answers, with the given
    /// default branch and parent: the `submit_repo_json` shape from
    /// the working-copy tests, which the binary crate cannot import.
    fn resolve_repo_json(
        owner: &str,
        name: &str,
        default_branch: &str,
        parent: Option<(&str, &str)>,
    ) -> String {
        let parent_json = match parent {
            Some((parent_owner, parent_name)) => format!(
                r#"{{"owner": {{"login": "{parent_owner}"}}, "name": "{parent_name}", "full_name": "{parent_owner}/{parent_name}"}}"#
            ),
            None => "null".to_string(),
        };
        format!(
            r#"{{"owner": {{"login": "{owner}"}}, "name": "{name}", "full_name": "{owner}/{name}", "parent": {parent_json}, "default_branch": "{default_branch}", "private": false, "clone_url": "https://src.opensuse.org/{owner}/{name}.git", "ssh_url": "gitea@src.opensuse.org:{owner}/{name}.git"}}"#
        )
    }

    fn resolve_gitea_client(http: &ResolveHttp) -> GiteaClient<'_> {
        GiteaClient::with_base(
            http,
            "https://src.opensuse.org",
            Credentials::Bearer {
                token: "token".to_string(),
            },
        )
        .unwrap()
    }

    #[test]
    fn resolve_git_submit_target_explicit_repo_defaults_base_from_the_target() {
        let (_dir, wc) = git_parts_checkout(None);
        let mut args = gitea_submit_args();
        args.repo = Some("pool/repo".to_string());
        let parts = git_submit_parts(&args, &wc, "https://src.opensuse.org").unwrap();
        let http = ResolveHttp::with_get(
            "https://src.opensuse.org/api/v1/repos/pool/repo",
            &resolve_repo_json("pool", "repo", "devel", None),
        );
        let gitea = resolve_gitea_client(&http);
        let target = resolve_git_submit_target(parts, &gitea).unwrap();
        match target {
            SubmitTarget::Gitea {
                owner,
                repo,
                options,
            } => {
                assert_eq!(owner, "pool");
                assert_eq!(repo, "repo");
                // The target is not the origin repo, so the head is
                // qualified with the fork owner.
                assert_eq!(options.head, "scarabeus_iv:main");
                assert_eq!(options.base, "devel");
            }
            SubmitTarget::Obs(_) => panic!("expected a Gitea target"),
        }
    }

    #[test]
    fn resolve_git_submit_target_refuses_an_explicit_target_whose_default_is_factory() {
        let (_dir, wc) = git_parts_checkout(None);
        let mut args = gitea_submit_args();
        args.repo = Some("pool/repo".to_string());
        let parts = git_submit_parts(&args, &wc, "https://src.opensuse.org").unwrap();
        let http = ResolveHttp::with_get(
            "https://src.opensuse.org/api/v1/repos/pool/repo",
            &resolve_repo_json("pool", "repo", "Factory", None),
        );
        let gitea = resolve_gitea_client(&http);
        let err = resolve_git_submit_target(parts, &gitea).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(
            err.to_string()
                .contains("the factory branch does not take pull requests"),
            "err: {err}"
        );
    }

    #[test]
    fn resolve_git_submit_target_refuses_a_defaulted_target_whose_default_is_factory() {
        let (_dir, wc) = git_parts_checkout(None);
        let args = gitea_submit_args();
        let parts = git_submit_parts(&args, &wc, "https://src.opensuse.org").unwrap();
        let http = ResolveHttp::with_get(
            "https://src.opensuse.org/api/v1/repos/scarabeus_iv/foo",
            &resolve_repo_json("scarabeus_iv", "foo", "main", Some(("pool", "foo"))),
        )
        .and_get(
            "https://src.opensuse.org/api/v1/repos/pool/foo",
            &resolve_repo_json("pool", "foo", "factory", None),
        );
        let gitea = resolve_gitea_client(&http);
        let err = resolve_git_submit_target(parts, &gitea).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(
            err.to_string()
                .contains("the factory branch does not take pull requests"),
            "err: {err}"
        );
    }

    #[test]
    fn resolve_git_submit_target_defaults_to_the_parent_and_qualifies_the_head() {
        let (_dir, wc) = git_parts_checkout(None);
        let args = gitea_submit_args();
        let parts = git_submit_parts(&args, &wc, "https://src.opensuse.org").unwrap();
        let http = ResolveHttp::with_get(
            "https://src.opensuse.org/api/v1/repos/scarabeus_iv/foo",
            &resolve_repo_json("scarabeus_iv", "foo", "main", Some(("pool", "foo"))),
        )
        .and_get(
            "https://src.opensuse.org/api/v1/repos/pool/foo",
            &resolve_repo_json("pool", "foo", "devel", None),
        );
        let gitea = resolve_gitea_client(&http);
        let target = resolve_git_submit_target(parts, &gitea).unwrap();
        match target {
            SubmitTarget::Gitea {
                owner,
                repo,
                options,
            } => {
                assert_eq!(owner, "pool");
                assert_eq!(repo, "foo");
                assert_eq!(options.head, "scarabeus_iv:main");
                assert_eq!(options.base, "devel");
            }
            SubmitTarget::Obs(_) => panic!("expected a Gitea target"),
        }
    }

    #[test]
    fn resolve_git_submit_target_into_the_origin_keeps_a_bare_head() {
        let (_dir, wc) = git_parts_checkout(None);
        let mut args = gitea_submit_args();
        args.repo = Some("scarabeus_iv/foo".to_string());
        let parts = git_submit_parts(&args, &wc, "https://src.opensuse.org").unwrap();
        let http = ResolveHttp::with_get(
            "https://src.opensuse.org/api/v1/repos/scarabeus_iv/foo",
            &resolve_repo_json("scarabeus_iv", "foo", "devel", Some(("pool", "foo"))),
        );
        let gitea = resolve_gitea_client(&http);
        let target = resolve_git_submit_target(parts, &gitea).unwrap();
        match target {
            SubmitTarget::Gitea {
                owner,
                repo,
                options,
            } => {
                assert_eq!(owner, "scarabeus_iv");
                assert_eq!(repo, "foo");
                assert_eq!(options.head, "main");
                assert_eq!(options.base, "devel");
            }
            SubmitTarget::Obs(_) => panic!("expected a Gitea target"),
        }
    }

    #[test]
    fn resolve_git_submit_target_keeps_an_already_qualified_head() {
        let (_dir, wc) = git_parts_checkout(None);
        let mut args = gitea_submit_args();
        args.repo = Some("pool/repo".to_string());
        args.head = Some("other:feature".to_string());
        let parts = git_submit_parts(&args, &wc, "https://src.opensuse.org").unwrap();
        let http = ResolveHttp::with_get(
            "https://src.opensuse.org/api/v1/repos/pool/repo",
            &resolve_repo_json("pool", "repo", "devel", None),
        );
        let gitea = resolve_gitea_client(&http);
        let target = resolve_git_submit_target(parts, &gitea).unwrap();
        match target {
            SubmitTarget::Gitea { options, .. } => {
                assert_eq!(options.head, "other:feature");
            }
            SubmitTarget::Obs(_) => panic!("expected a Gitea target"),
        }
    }

    #[test]
    fn scmsync_branch_matches_fragment_pin_and_tracking_branch() {
        // A fragment that pins a commit still tracks the branch the
        // `trackingbranch` names; a fragment naming a branch
        // matches it — and only it; no branch at all tracks the
        // default branch only.
        let pinned = parse_scmsync_url(
            "http://127.0.0.1:9/pool/systemd?trackingbranch=factory#deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        )
        .expect("fixture scmsync parses");
        assert!(scmsync_branch_matches(&pinned, "factory", "factory"));
        assert!(!scmsync_branch_matches(&pinned, "main", "factory"));
        let fragment_names_a_branch =
            parse_scmsync_url("http://127.0.0.1:9/pool/systemd?trackingbranch=factory#main")
                .expect("fixture scmsync parses");
        assert!(scmsync_branch_matches(
            &fragment_names_a_branch,
            "main",
            "factory"
        ));
        // The fragment wins: the tracking branch does not match
        // when the fragment names another branch.
        assert!(!scmsync_branch_matches(
            &fragment_names_a_branch,
            "factory",
            "factory"
        ));
        let branched = parse_scmsync_url("http://127.0.0.1:9/pool/systemd#factory")
            .expect("fixture scmsync parses");
        assert!(scmsync_branch_matches(&branched, "factory", "factory"));
        assert!(!scmsync_branch_matches(&branched, "main", "factory"));
        let bare =
            parse_scmsync_url("http://127.0.0.1:9/pool/systemd").expect("fixture scmsync parses");
        assert!(scmsync_branch_matches(&bare, "factory", "factory"));
        assert!(!scmsync_branch_matches(&bare, "main", "factory"));
    }

    #[test]
    fn parse_gitea_full_name_requires_one_non_empty_pair() {
        assert_eq!(
            parse_gitea_full_name("pool/systemd", "gitea:pool/systemd#7").unwrap(),
            ("pool".to_string(), "systemd".to_string())
        );
        for malformed in [
            "pool",
            "pool/systemd/extra",
            "pool/",
            "/systemd",
            "",
            "pool//systemd",
        ] {
            let err = parse_gitea_full_name(malformed, "gitea:pool/systemd#7").unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ServerError, "{malformed}: {err}");
        }
    }
}
