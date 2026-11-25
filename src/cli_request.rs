//! `request` command group: read and act on OBS submit requests, and
//! file new ones. Thin over the library (`ObsClient` request
//! operations, `SrFiling`); every leaf returns the (payload, text)
//! pair, so JSON and `--format text` agree on every id, name and
//! state. No XML in any user-facing surface — typed values only.
//!
//! The command surface covers the full request inventory, but not
//! its inconsistencies: every request operation lives under the
//! `request` noun, one flag name per concept everywhere (`--comment`
//! for state-change and review messages, `--description` for a
//! filing's description), and subcommand names are the library
//! operation names in kebab-case. Deliberate deviations from the
//! inventory are tracked as swap rows in the rollout's swaps table.
//!
//! Mutations are dry-run by default, like the library: the global
//! `--live` performs them. A dry-run mutation prints its plan on stderr (the
//! library's `[dry-run]` narration) and returns a payload with
//! `dry_run: true`, the recorded `plan`, and no id. In a dry run
//! `try-change-state` reports `changed: true`: that is the library's
//! would-change answer, not a change that happened — read it together
//! with `dry_run`. The one exception is `request supersede`: the
//! library's raw supersede has no dry-run, so without `--live` that
//! command refuses — `request refile --supersede` is the
//! dry-runnable supersede flow.
//!
//! Credentials in dry-run: this group swallows a half-configured
//! credential pair in dry-run (the plan prints without credentials;
//! the dropped error goes to the debug channel and to stderr as a
//! warning), so a request preview
//! never needs credentials at all. A live run — and every read —
//! still resolves credentials up front and fails fast. How the other
//! groups handle a half-configured pair is stated in `docs/cli.md`.
//!
//! Text output echoes the query coordinates in one header line per
//! command: the request id for the single-request reads and the
//! one-line actions (`show` opens with `id:`, `log` and
//! `list-comments` with `id:` in the non-empty case — the id is
//! echoed in the `no …` line when empty), and the filters actually
//! given plus the result count for `search`. `diff` is a content
//! command and is exempt from the header (design rule 1, rule 4):
//! its text is the diff verbatim, and the coordinates live in the
//! JSON payload. An empty diff — a request with no submit action,
//! for which the server returns an empty body — renders
//! `no diff for request <id>` instead of empty output (design
//! rule 5). Tables stay narrow by design — the search table shows
//! id, state, creator and actions only, and the actions column is a
//! compact summary (`submit foo`): target names and `source_rev` are
//! omitted there, and stay in the JSON payload and in
//! `request show <id>`. In `show`, the accepted revision renders but
//! the acceptinfo hashes and origin project/package do not — they stay
//! in the JSON payload. `--with-history` enriches the JSON payload
//! only; the table never renders history. Action texts are one-line
//! summaries naming the id and the outcome; the full result is the
//! JSON payload. The `file` / `refile` filing texts carry the CVE
//! ids (identifiers) but not the description or the `.changes` entry
//! (long free text); the other filing texts carry no CVE ids.
//! These omissions are decided, not gaps: do not expand the tables
//! to echo the full payload.

use crate::auth::{Credentials, EnvObsAuth};
use crate::config;
use crate::debug::{strip_terminal_controls, Debug};
use crate::error::{ErrorKind, GeectlError};
use crate::http::{HttpClient, Timeout, UreqClient};
use crate::obs::{
    AcceptInfo, AddRolePayload, ChangeDevelPayload, CveId, DeletePayload, DeleteTarget, DryRun,
    FiledRequest, ForwardActionResult, ForwardResult, MaintenanceIncidentPayload, ObsClient,
    Request, RequestAction, RequestComment, RequestHistory, RequestReview, RequestSearch,
    RequestState, RequestStateInfo, ReviewState, ReviewTarget, SrFiling, SrPayload, OBS_API_BASE,
};
use clap::{Args, Subcommand};
use serde_json::{json, Value};

/// The `request` subcommands: one leaf per library operation, named
/// after the operation in kebab-case.
#[derive(Debug, Subcommand)]
pub enum RequestCommand {
    /// Show one submit request: source/target/message, reviews, history.
    Show {
        /// The request id.
        id: String,
    },
    /// Show one submit request's history: creation, then each state change.
    Log {
        /// The request id.
        id: String,
    },
    /// List the submit requests matching the filters: a typed search.
    Search {
        /// Only requests with these ids.
        #[arg(long = "request-id")]
        request_ids: Vec<u64>,
        /// Only requests with an action from or to these projects.
        #[arg(long)]
        project: Vec<String>,
        /// Only requests with an action from or to these packages.
        #[arg(long)]
        package: Vec<String>,
        /// Only requests with an action from these source projects.
        #[arg(long)]
        source_project: Vec<String>,
        /// Only requests with an action from these source packages.
        #[arg(long)]
        source_package: Vec<String>,
        /// Only requests with an action to these target projects.
        #[arg(long)]
        target_project: Vec<String>,
        /// Only requests with an action to these target packages.
        #[arg(long)]
        target_package: Vec<String>,
        /// Only requests filed by these users.
        #[arg(long)]
        creator: Vec<String>,
        /// Only requests involving these users.
        #[arg(long)]
        who: Vec<String>,
        /// Only requests with an open review addressed to these users.
        /// With no `--state`, only new and review requests are listed.
        #[arg(long)]
        reviewer: Vec<String>,
        /// Only requests in these states. When none are given, only
        /// new, review and declined requests are listed; `--reviewer`
        /// narrows this to new and review.
        #[arg(long, value_parser = [
            "new", "review", "accepted", "declined", "revoked", "superseded", "deleted",
        ])]
        state: Vec<String>,
        /// Only requests with an action of these types.
        #[arg(long = "type")]
        types: Vec<String>,
        /// Exclude requests with an action targeting these projects.
        #[arg(long)]
        exclude_target_project: Vec<String>,
        /// Include the history of each request.
        #[arg(long)]
        with_history: bool,
        /// Include the full history of each request.
        #[arg(long)]
        with_full_history: bool,
        /// Only requests with a history entry at or after this
        /// date, or a last update at or after it, as `YYYY-MM-DD`
        /// or `YYYY-MM-DDTHH:MM:SS`.
        #[arg(long)]
        since: Option<String>,
        /// Ask the server for at most this many matches.
        #[arg(long)]
        limit: Option<usize>,
    },
    /// List submit requests: the same search, with the states still
    /// awaiting action (new, review) as the default. A thin wrapper
    /// over the search operation: the flags select the same
    /// server-side search.
    List {
        /// Requests touching this project.
        #[arg(long)]
        project: Vec<String>,
        /// Requests touching this package; requires `--project`.
        #[arg(long)]
        package: Vec<String>,
        /// Requests in this state; when empty, new and review.
        #[arg(long)]
        state: Vec<String>,
        /// Only requests with a history entry at or after this
        /// date, or a last update at or after it, as `YYYY-MM-DD`
        /// or `YYYY-MM-DDTHH:MM:SS`.
        #[arg(long)]
        since: Option<String>,
        /// Ask the server for at most this many matches.
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Show the diff of a submit request: the unified diff, or the
    /// request XML with the diff injected with `--xml`.
    Diff {
        /// The request id.
        id: String,
        /// Render the request XML with the diff injected, instead of
        /// the unified diff.
        #[arg(long)]
        xml: bool,
    },
    /// List the comments on a submit request, in id order.
    ListComments {
        /// The request id.
        id: String,
    },
    /// Accept a submit request, with a comment as the message left
    /// with the accept. An accepted request is forwarded to its
    /// link parent unless `--no-forward` is given; a refused
    /// forward is reported and the accepted request stands, with
    /// the work staying queued in the devel project. Dry-run by
    /// default; mutates only with `--live`.
    Accept {
        /// The request id.
        id: String,
        /// The message left with the accept. Values starting with `-` are taken literally: a comment may open with a `- ` bullet.
        #[arg(long, default_value = "", allow_hyphen_values = true)]
        comment: String,
        /// Accept a request still in `review` state, overriding its
        /// open reviews.
        #[arg(long)]
        force: bool,
        /// Do not forward the request after accepting it: by
        /// default an accepted request is forwarded to its link
        /// parent.
        #[arg(long)]
        no_forward: bool,
    },
    /// Decline a submit request, with a comment as the reason.
    Decline {
        /// The request id.
        id: String,
        /// The reason for the decline. Values starting with `-` are taken literally: a comment may open with a `- ` bullet.
        #[arg(long, default_value = "", allow_hyphen_values = true)]
        comment: String,
    },
    /// Revoke (withdraw) a submit request, with a comment as the reason.
    Revoke {
        /// The request id.
        id: String,
        /// The reason for the revocation. Values starting with `-` are taken literally: a comment may open with a `- ` bullet.
        #[arg(long, default_value = "", allow_hyphen_values = true)]
        comment: String,
    },
    /// Change a submit request's state to one of
    /// new/review/accepted/declined/revoked.
    ChangeState {
        /// The request id.
        id: String,
        /// The state to change to.
        #[arg(value_parser = ["new", "review", "accepted", "declined", "revoked"])]
        state: String,
        /// The message left with the change. Values starting with `-` are taken literally: a comment may open with a `- ` bullet.
        #[arg(long, default_value = "", allow_hyphen_values = true)]
        comment: String,
        /// Force the change, overriding open reviews. Only honoured when changing to accepted.
        #[arg(long)]
        force: bool,
    },
    /// Best-effort state change: try it, and report whether it happened.
    TryChangeState {
        /// The request id.
        id: String,
        /// The state to change to.
        #[arg(value_parser = ["new", "review", "accepted", "declined", "revoked"])]
        state: String,
        /// The message left with the change. Values starting with `-` are taken literally: a comment may open with a `- ` bullet.
        #[arg(long, default_value = "", allow_hyphen_values = true)]
        comment: String,
        /// Force the change, overriding open reviews. Only honoured when changing to accepted.
        #[arg(long)]
        force: bool,
    },
    /// Ask a user, group, project, or package to review a submit request.
    AddReview {
        /// The request id.
        id: String,
        /// The user the review is asked of.
        #[arg(long)]
        user: Option<String>,
        /// The group the review is asked of.
        #[arg(long)]
        group: Option<String>,
        /// The project the review is asked of.
        #[arg(long)]
        project: Option<String>,
        /// The package the review is asked of (needs its project).
        #[arg(long)]
        package: Option<String>,
        /// The reason the review is asked for. Values starting with `-` are taken literally: a comment may open with a `- ` bullet.
        #[arg(long, allow_hyphen_values = true)]
        comment: String,
    },
    /// Change the state of one review on a submit request: accept it, decline it, or reopen it to new.
    ChangeReviewState {
        /// The request id.
        id: String,
        /// The state to change the review to.
        #[arg(value_parser = ["new", "accepted", "declined"])]
        state: String,
        /// The user the review was asked of.
        #[arg(long)]
        user: Option<String>,
        /// The group the review was asked of.
        #[arg(long)]
        group: Option<String>,
        /// The project the review was asked of.
        #[arg(long)]
        project: Option<String>,
        /// The package the review was asked of (needs its project).
        #[arg(long)]
        package: Option<String>,
        /// The message left with the change. Values starting with `-` are taken literally: a comment may open with a `- ` bullet.
        #[arg(long, default_value = "", allow_hyphen_values = true)]
        comment: String,
    },
    /// Comment on a submit request, optionally as a reply to another comment.
    AddComment {
        /// The request id.
        id: String,
        /// The comment text. Values starting with `-` are taken literally: the body may open with a `- ` bullet.
        #[arg(long, allow_hyphen_values = true)]
        body: String,
        /// The comment being replied to, when this is a reply.
        #[arg(long)]
        parent: Option<String>,
    },
    /// File a submit request: source package to a devel project, superseding every open submit request for the package from the same user.
    File {
        #[command(flatten)]
        file: FileArgs,
    },
    /// File a submit request and supersede an old one with it, plus
    /// every other open submit request for the package from the same
    /// user: the new request is filed first, and only then is each
    /// open one superseded pointing at it.
    Refile {
        #[command(flatten)]
        file: FileArgs,
        /// The open request this filing supersedes.
        #[arg(long)]
        supersede: String,
    },
    /// Supersede one request with another existing one. A mutation
    /// that needs `--live`; the dry-runnable supersede flow is
    /// `request refile --supersede`.
    Supersede {
        /// The open request being superseded.
        old_id: String,
        /// The existing request superseding it.
        new_id: String,
    },
    /// File an add-role request: grant a person or a group a role on
    /// a project or a package.
    FileAddRole {
        /// The project the role is granted on.
        #[arg(long)]
        target_project: String,
        /// The package the role is granted on; omit for a
        /// project-level grant.
        #[arg(long)]
        target_package: Option<String>,
        /// The person granted the role.
        #[arg(long, requires = "person_role")]
        person: Option<String>,
        /// The role granted to the person.
        #[arg(long, requires = "person")]
        person_role: Option<String>,
        /// The group granted the role.
        #[arg(long, requires = "group_role")]
        group: Option<String>,
        /// The role granted to the group.
        #[arg(long, requires = "group")]
        group_role: Option<String>,
        /// The request description. Values starting with `-` are
        /// taken literally: a description may open with a `- ` bullet.
        #[arg(long, allow_hyphen_values = true)]
        description: String,
    },
    /// File a change-devel request: change a package's devel project.
    FileChangeDevel {
        /// The project the package lives in.
        #[arg(long)]
        source_project: String,
        /// The package whose devel project changes.
        #[arg(long)]
        source_package: String,
        /// The project that becomes the devel project.
        #[arg(long)]
        target_project: String,
        /// The package in the devel project.
        #[arg(long)]
        target_package: String,
        /// The request description. Values starting with `-` are
        /// taken literally: a description may open with a `- ` bullet.
        #[arg(long, allow_hyphen_values = true)]
        description: String,
    },
    /// File a delete request: delete a package, a repository, or a
    /// whole project.
    FileDelete {
        /// The project the deletion targets.
        #[arg(long)]
        target_project: String,
        /// The package to delete. Omit both this and `--repository`
        /// to delete the whole project (with `--whole-project`).
        #[arg(long, conflicts_with = "repository")]
        package: Option<String>,
        /// The repository to delete. Omit both this and `--package`
        /// to delete the whole project (with `--whole-project`).
        #[arg(long)]
        repository: Option<String>,
        /// Delete the whole project: required when neither
        /// `--package` nor `--repository` is given. A whole-project
        /// deletion is destructive, so omitting the target is not
        /// enough — the flag is the explicit confirmation.
        #[arg(long, conflicts_with_all = ["package", "repository"])]
        whole_project: bool,
        /// The request description. Values starting with `-` are
        /// taken literally: a description may open with a `- ` bullet.
        #[arg(long, allow_hyphen_values = true)]
        description: String,
        /// Auto-accept the request this many hours after filing
        /// (the server accepts it on its own once the time passes),
        /// as `osc deletereq --accept-in-hours` does. The timestamp
        /// is computed at filing time; 0 means accept immediately.
        #[arg(long)]
        accept_in_hours: Option<u64>,
    },
    /// File a maintenance incident request: ask the maintenance team
    /// to start a maintenance incident from a maintenance update.
    FileMaintenanceIncident {
        #[command(flatten)]
        incident: MaintenanceIncidentArgs,
    },
    /// File a maintenance incident request and supersede an old one
    /// with it: the new request is filed first, and only then is the
    /// old one superseded pointing at it.
    RefileMaintenanceIncident {
        #[command(flatten)]
        incident: MaintenanceIncidentArgs,
        /// The open request this filing supersedes.
        #[arg(long)]
        supersede: String,
    },
}

/// The flags shared by `request file` and `request refile`: the
/// submit-request payload, mirroring `SrPayload::new`.
#[derive(Debug, Args)]
pub struct FileArgs {
    /// The project the change comes from (the branch).
    #[arg(long)]
    source_project: String,
    /// The package in the source project.
    #[arg(long)]
    source_package: String,
    /// The project the request targets: the devel project, or
    /// openSUSE:Factory when the source is the recorded devel
    /// project and package.
    #[arg(long)]
    target_project: String,
    /// The package in the target project.
    #[arg(long)]
    target_package: String,
    /// The request description: one headline, then `- ` lines for
    /// staging, needs, and rebuilds. Values starting with `-` are
    /// taken literally: a description may open with a `- ` bullet.
    #[arg(long, allow_hyphen_values = true)]
    description: String,
    /// The one `.changes` entry for this submission. It must name
    /// every CVE passed with `--cve`. Values starting with `-` are
    /// taken literally: a `.changes` entry starts with `- `.
    #[arg(long, allow_hyphen_values = true)]
    changes_entry: String,
    /// A CVE this submission claims to fix. Repeatable; each one
    /// must be named in `--changes-entry`.
    #[arg(long)]
    cve: Vec<String>,
    /// Keep the source branch on accept instead of deleting it
    /// (cleanup is the default).
    #[arg(long)]
    without_cleanup: bool,
}

/// The flags shared by `request file-maintenance-incident` and
/// `request refile-maintenance-incident`: the maintenance incident
/// payload, mirroring `MaintenanceIncidentPayload::new`. Unlike a
/// submit request, the branch survives the incident by default;
/// `--with-cleanup` opts into deleting it on accept.
#[derive(Debug, Args)]
pub struct MaintenanceIncidentArgs {
    /// The project the maintenance update comes from.
    #[arg(long)]
    source_project: String,
    /// A package in the source project. Repeatable: one action per
    /// package; at least one is required.
    #[arg(long)]
    source_package: Vec<String>,
    /// The maintenance project the incident goes to.
    #[arg(long)]
    target_project: String,
    /// The release project the update releases into. When omitted,
    /// the server resolves it from the maintenance project.
    #[arg(long)]
    release_project: Option<String>,
    /// The request description. Values starting with `-` are
    /// taken literally: a description may open with a `- ` bullet.
    #[arg(long, allow_hyphen_values = true)]
    description: String,
    /// Delete the source branch when the incident is accepted. By
    /// default the branch survives the incident.
    #[arg(long)]
    with_cleanup: bool,
}

/// Run one `request` subcommand, returning the (payload, text) pair:
/// the JSON payload and a human rendering of the same data.
pub fn run(
    command: RequestCommand,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    match command {
        RequestCommand::Show { id } => with_client(debug, true, |client| {
            let request = client.show_request(&id)?;
            Ok((request_json(&request), render_request_text(&request)))
        }),
        RequestCommand::Log { id } => with_client(debug, true, |client| {
            let history = client.request_log(&id)?;
            let payload = json!({
                "id": request_id_json(&id),
                "history": history.iter().map(history_json).collect::<Vec<_>>(),
            });
            Ok((payload, render_history_text(&id, &history)))
        }),
        RequestCommand::Search {
            request_ids,
            project,
            package,
            source_project,
            source_package,
            target_project,
            target_package,
            creator,
            who,
            reviewer,
            state,
            types,
            exclude_target_project,
            with_history,
            with_full_history,
            since,
            limit,
        } => {
            // The coordinates are read before `build_search` moves
            // the filter values, and the search is built and
            // validated before any credential resolution, so an
            // invalid filter fails as InvalidInput whether or not
            // credentials are configured (the file arms already
            // validate their payloads this way).
            let ids: Vec<String> = request_ids.iter().map(u64::to_string).collect();
            let since_values: Vec<String> = since.iter().cloned().collect();
            let limit_values: Vec<String> = limit.iter().map(usize::to_string).collect();
            let coordinates = search_coordinates(&[
                ("request-id", &ids),
                ("project", &project),
                ("package", &package),
                ("source-project", &source_project),
                ("source-package", &source_package),
                ("target-project", &target_project),
                ("target-package", &target_package),
                ("creator", &creator),
                ("who", &who),
                ("reviewer", &reviewer),
                ("state", &state),
                ("type", &types),
                ("exclude-target-project", &exclude_target_project),
                ("since", &since_values),
                ("limit", &limit_values),
            ]);
            let search = build_search(
                request_ids,
                project,
                package,
                source_project,
                source_package,
                target_project,
                target_package,
                creator,
                who,
                reviewer,
                state,
                types,
                exclude_target_project,
                with_history,
                with_full_history,
                since,
                limit,
            )?;
            search.match_expr()?;
            with_client(debug, true, |client| {
                let requests = client.search_requests(&search)?;
                let payload = json!({
                    "requests": requests.iter().map(request_json).collect::<Vec<_>>(),
                    "count": requests.len(),
                });
                Ok((payload, render_search_text(&coordinates, &requests)))
            })
        }
        RequestCommand::List {
            project,
            package,
            state,
            since,
            limit,
        } => {
            // `--package` names a package inside a project, so it
            // is refused without `--project` before anything else,
            // and the search is built and validated before any
            // credential resolution, like the search arm.
            validate_list_scope(&project, &package)?;
            let since_values: Vec<String> = since.iter().cloned().collect();
            let limit_values: Vec<String> = limit.iter().map(usize::to_string).collect();
            let coordinates = search_coordinates(&[
                ("project", &project),
                ("package", &package),
                ("state", &state),
                ("since", &since_values),
                ("limit", &limit_values),
            ]);
            let search = build_search(
                vec![],
                project,
                package,
                vec![],
                vec![],
                vec![],
                vec![],
                vec![],
                vec![],
                vec![],
                list_default_states(state),
                vec![],
                vec![],
                false,
                false,
                since,
                limit,
            )?;
            search.match_expr()?;
            with_client(debug, true, |client| {
                let requests = client.search_requests(&search)?;
                let payload = json!({
                    "requests": requests.iter().map(request_json).collect::<Vec<_>>(),
                    "count": requests.len(),
                });
                Ok((payload, render_list_text(&coordinates, &requests)))
            })
        }
        RequestCommand::Diff { id, xml } => with_client(debug, true, |client| {
            let diff = client.request_diff(&id, xml)?;
            // A content command: the payload is the diff, and the
            // text rendering is the diff verbatim. Content commands
            // are exempt from the coordinate header (design rule 1):
            // the coordinates live in the JSON payload. The one
            // exception is an empty diff, which renders an explicit
            // line naming the request (design rule 5) instead of
            // empty stdout.
            let payload = json!({ "id": request_id_json(&id), "diff": diff, "xml": xml });
            Ok((payload, render_diff_text(&id, &diff)))
        }),
        RequestCommand::ListComments { id } => with_client(debug, true, |client| {
            let comments = client.list_comments(&id)?;
            let payload = json!({
                "id": request_id_json(&id),
                "comments": comments.iter().map(comment_json).collect::<Vec<_>>(),
            });
            Ok((payload, render_comments_text(&id, &comments)))
        }),
        RequestCommand::Accept {
            id,
            comment,
            force,
            no_forward,
        } => accept(debug, &id, &comment, force, no_forward, live),
        RequestCommand::Decline { id, comment } => act(
            debug,
            &id,
            RequestState::Declined,
            &comment,
            false,
            live,
            "declined",
        ),
        RequestCommand::Revoke { id, comment } => act(
            debug,
            &id,
            RequestState::Revoked,
            &comment,
            false,
            live,
            "revoked",
        ),
        RequestCommand::ChangeState {
            id,
            state,
            comment,
            force,
        } => {
            let state = parse_state(&state)?;
            let verb = state.as_str();
            act(debug, &id, state, &comment, force, live, verb)
        }
        RequestCommand::TryChangeState {
            id,
            state,
            comment,
            force,
        } => {
            let state = parse_state(&state)?;
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            with_client(debug, live, |client| {
                let changed = client.try_change_state(&id, state, &comment, force, &dry_run)?;
                // `changed` is the library's would-change answer: in
                // a dry run it is true by design (the change is only
                // planned, not performed), so read it together with
                // `dry_run`, never as "the state changed".
                let mut payload = json!({
                    "id": request_id_json(&id),
                    "target_state": state.as_str(),
                    "comment": comment,
                    "changed": changed,
                    "dry_run": dry_run.enabled(),
                });
                add_plan(&mut payload, &dry_run);
                let mut text = if dry_run.enabled() {
                    format!(
                        "dry-run: would try to change request {id} to {}",
                        state.as_str()
                    )
                } else if changed {
                    format!("changed request {id} to {}", state.as_str())
                } else {
                    format!("request {id} not changed to {}", state.as_str())
                };
                if !comment.is_empty() {
                    text.push_str(&format!(" — {comment}"));
                }
                Ok((payload, text))
            })
        }
        RequestCommand::AddReview {
            id,
            user,
            group,
            project,
            package,
            comment,
        } => {
            let target = ReviewTarget::new(
                user.as_deref(),
                group.as_deref(),
                project.as_deref(),
                package.as_deref(),
            )?;
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            with_client(debug, live, |client| {
                client.add_review(&id, &target, &comment, &dry_run)?;
                let mut payload = json!({
                    "id": request_id_json(&id),
                    "review": {
                        "by_user": target.by_user,
                        "by_group": target.by_group,
                        "by_project": target.by_project,
                        "by_package": target.by_package,
                    },
                    "comment": comment,
                    "dry_run": dry_run.enabled(),
                });
                add_plan(&mut payload, &dry_run);
                let mut text = if dry_run.enabled() {
                    format!(
                        "dry-run: would ask {} to review request {id}",
                        target.describe()
                    )
                } else {
                    format!("asked {} to review request {id}", target.describe())
                };
                if !comment.is_empty() {
                    text.push_str(&format!(" — {comment}"));
                }
                Ok((payload, text))
            })
        }
        RequestCommand::ChangeReviewState {
            id,
            state,
            user,
            group,
            project,
            package,
            comment,
        } => {
            // Clap's value_parser admits only new, accepted, declined,
            // and all three parse, so this cannot fail from the CLI.
            let parsed = ReviewState::parse(&state);
            debug_assert!(
                parsed.is_some(),
                "clap value_parser admits only new, accepted, declined"
            );
            let state = parsed.ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid review state: {state:?}"),
                )
            })?;
            let target = ReviewTarget::new(
                user.as_deref(),
                group.as_deref(),
                project.as_deref(),
                package.as_deref(),
            )?;
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            with_client(debug, live, |client| {
                client.change_review_state(&id, state, &target, &comment, &dry_run)?;
                let mut payload = json!({
                    "id": request_id_json(&id),
                    "target_state": state.as_str(),
                    "review": {
                        "by_user": target.by_user,
                        "by_group": target.by_group,
                        "by_project": target.by_project,
                        "by_package": target.by_package,
                    },
                    "comment": comment,
                    "dry_run": dry_run.enabled(),
                });
                add_plan(&mut payload, &dry_run);
                let mut text = if dry_run.enabled() {
                    format!(
                        "dry-run: would change the review by {} on request {id} to {}",
                        target.describe(),
                        state.as_str()
                    )
                } else {
                    format!(
                        "changed the review by {} on request {id} to {}",
                        target.describe(),
                        state.as_str()
                    )
                };
                if !comment.is_empty() {
                    text.push_str(&format!(" — {comment}"));
                }
                Ok((payload, text))
            })
        }
        RequestCommand::AddComment { id, body, parent } => {
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            with_client(debug, live, |client| {
                client.add_comment(&id, &body, parent.as_deref(), &dry_run)?;
                let mut payload = json!({
                    "id": request_id_json(&id),
                    "body": body,
                    "dry_run": dry_run.enabled(),
                });
                add_plan(&mut payload, &dry_run);
                let text = if dry_run.enabled() {
                    format!("dry-run: would comment on request {id}: {body}")
                } else {
                    format!("commented on request {id}: {body}")
                };
                Ok((payload, text))
            })
        }
        RequestCommand::File { file } => file_request(debug, live, &file, None),
        RequestCommand::Refile { file, supersede } => {
            file_request(debug, live, &file, Some(&supersede))
        }
        RequestCommand::FileMaintenanceIncident { incident } => {
            file_maintenance_incident(debug, live, &incident, None)
        }
        RequestCommand::RefileMaintenanceIncident {
            incident,
            supersede,
        } => file_maintenance_incident(debug, live, &incident, Some(&supersede)),
        RequestCommand::Supersede { old_id, new_id } => {
            if !live {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "request supersede has no dry-run: pass --live to perform it, or use `request refile --supersede` for the dry-runnable supersede flow",
                ));
            }
            with_client(debug, true, |client| {
                client.supersede_request(&old_id, &new_id)?;
                let payload = json!({
                    "old_id": request_id_json(&old_id),
                    "new_id": request_id_json(&new_id),
                    "dry_run": false,
                });
                Ok((
                    payload,
                    format!("superseded request {old_id} with {new_id}"),
                ))
            })
        }
        RequestCommand::FileChangeDevel {
            source_project,
            source_package,
            target_project,
            target_package,
            description,
        } => {
            let payload = ChangeDevelPayload::new(
                &source_project,
                &source_package,
                &target_project,
                &target_package,
                &description,
            )?;
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            with_client(debug, live, |client| {
                let filing = SrFiling::new(client, &dry_run).with_debug(debug.clone());
                let id = filing.file_change_devel(&payload)?;
                let filed = filed_payload(&id, &dry_run, |id| {
                    json!({
                        "id": id.map(request_id_json).unwrap_or(Value::Null),
                        "source_project": source_project,
                        "source_package": source_package,
                        "target_project": target_project,
                        "target_package": target_package,
                        "description": description,
                    })
                });
                let text = file_change_devel_text(
                    id.as_deref(),
                    &source_project,
                    &source_package,
                    &target_project,
                    &target_package,
                );
                Ok((filed, text))
            })
        }
        RequestCommand::FileAddRole {
            target_project,
            target_package,
            person,
            person_role,
            group,
            group_role,
            description,
        } => {
            let payload = AddRolePayload::new(
                &target_project,
                target_package.as_deref(),
                person.as_deref().zip(person_role.as_deref()),
                group.as_deref().zip(group_role.as_deref()),
                &description,
            )?;
            let target = match &target_package {
                Some(package) => format!("{target_project}/{package}"),
                None => target_project.clone(),
            };
            let mut grantees = Vec::new();
            if let (Some(name), Some(role)) = (&person, &person_role) {
                grantees.push(format!("person {name} as {role}"));
            }
            if let (Some(name), Some(role)) = (&group, &group_role) {
                grantees.push(format!("group {name} as {role}"));
            }
            let grantees = grantees.join(" and ");
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            with_client(debug, live, |client| {
                let filing = SrFiling::new(client, &dry_run).with_debug(debug.clone());
                let id = filing.file_add_role(&payload)?;
                let filed = filed_payload(&id, &dry_run, |id| {
                    json!({
                        "id": id.map(request_id_json).unwrap_or(Value::Null),
                        "target_project": target_project,
                        "target_package": target_package,
                        "person": person
                            .as_ref()
                            .zip(person_role.as_ref())
                            .map(|(name, role)| json!({"name": name, "role": role}))
                            .unwrap_or(Value::Null),
                        "group": group
                            .as_ref()
                            .zip(group_role.as_ref())
                            .map(|(name, role)| json!({"name": name, "role": role}))
                            .unwrap_or(Value::Null),
                        "description": description,
                    })
                });
                let text = file_add_role_text(id.as_deref(), &target, &grantees);
                Ok((filed, text))
            })
        }
        RequestCommand::FileDelete {
            target_project,
            package,
            repository,
            whole_project,
            description,
            accept_in_hours,
        } => {
            let target = match (package, repository) {
                (Some(package), None) => DeleteTarget::Package(package),
                (None, Some(repository)) => DeleteTarget::Repository(repository),
                (None, None) if whole_project => DeleteTarget::WholeProject,
                (None, None) => {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        "deleting a whole project needs --whole-project: pass it to confirm, or --package / --repository to delete part of the project",
                    ));
                }
                // clap's `conflicts_with` refuses both flags together,
                // so this arm is unreachable; it stays so a future
                // flag change fails loudly instead of silently
                // deleting the whole project.
                (Some(_), Some(_)) => {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        "pass exactly one of --package and --repository, or --whole-project to delete the whole project",
                    ));
                }
            };
            let target_text = match &target {
                DeleteTarget::Package(package) => format!("{target_project}/{package}"),
                DeleteTarget::Repository(repository) => {
                    format!("{target_project} (repository {repository})")
                }
                DeleteTarget::WholeProject => target_project.clone(),
            };
            let mut payload = DeletePayload::new(&target_project, target, &description)?;
            if let Some(hours) = accept_in_hours {
                payload = payload.with_accept_at_in_hours(hours)?;
            }
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            with_client(debug, live, |client| {
                let filing = SrFiling::new(client, &dry_run).with_debug(debug.clone());
                let id = filing.file_delete_request(&payload)?;
                let filed = filed_payload(&id, &dry_run, |id| {
                    json!({
                        "id": id.map(request_id_json).unwrap_or(Value::Null),
                        "target": target_text,
                        "description": description,
                        "accept_in_hours": accept_in_hours,
                    })
                });
                let text = file_delete_text(id.as_deref(), &target_text, accept_in_hours);
                Ok((filed, text))
            })
        }
    }
}

/// The filing text for `file-change-devel`: the filed (or
/// would-file) request id and the source and target coordinates.
/// The description is long free text and stays JSON-only.
fn file_change_devel_text(
    id: Option<&str>,
    source_project: &str,
    source_package: &str,
    target_project: &str,
    target_package: &str,
) -> String {
    match id {
        Some(id) => format!(
            "filed change-devel request {id} for {source_project}/{source_package} to {target_project}/{target_package}"
        ),
        None => format!(
            "dry-run: would file change-devel request for {source_project}/{source_package} to {target_project}/{target_package}"
        ),
    }
}

/// The filing text for `file-add-role`: the filed (or would-file)
/// request id, the grantees, and the target. The description is long
/// free text and stays JSON-only.
fn file_add_role_text(id: Option<&str>, target: &str, grantees: &str) -> String {
    match id {
        Some(id) => format!("filed add-role request {id} for {grantees} on {target}"),
        None => format!("dry-run: would file add-role request for {grantees} on {target}"),
    }
}

/// The filing text for `file-delete`: the filed (or would-file)
/// request id, the deletion target, and the auto-accept delay when
/// one was given. The description is long free text and stays
/// JSON-only.
fn file_delete_text(id: Option<&str>, target: &str, accept_in_hours: Option<u64>) -> String {
    let accept_text = match accept_in_hours {
        Some(hours) => format!(" (auto-accept in {hours}h)"),
        None => String::new(),
    };
    match id {
        Some(id) => format!("filed delete request {id} for {target}{accept_text}"),
        None => format!("dry-run: would file delete request for {target}{accept_text}"),
    }
}

/// Parse a state name. The domain is restricted by clap's
/// `value_parser` at each call site — five changeable states for
/// `change-state` / `try-change-state`, all seven for `search` —
/// so a failure here is a caller bug, not user input.
fn parse_state(value: &str) -> Result<RequestState, GeectlError> {
    RequestState::parse(value).ok_or_else(|| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid request state: {value:?}"),
        )
    })
}

/// The states `request list` searches when none were named: the
/// states still awaiting action, new and review.
fn list_default_states(state: Vec<String>) -> Vec<String> {
    if state.is_empty() {
        vec!["new".to_string(), "review".to_string()]
    } else {
        state
    }
}

/// `--package` names a package inside a project, so `request list`
/// refuses it without `--project`.
fn validate_list_scope(project: &[String], package: &[String]) -> Result<(), GeectlError> {
    if !package.is_empty() && project.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "request list --package requires --project",
        ));
    }
    Ok(())
}

/// Build the `RequestSearch` from the `search` flags. The flags
/// mirror the builder methods one to one; an unknown `--state`
/// value is refused here rather than silently matching nothing.
/// When no `--state` is given, the builder's own default applies
/// (new/review/declined): passing
/// an empty state list would instead mean "no state filter at
/// all", so the states are only set when the caller named some.
#[allow(clippy::too_many_arguments)]
fn build_search(
    request_ids: Vec<u64>,
    project: Vec<String>,
    package: Vec<String>,
    source_project: Vec<String>,
    source_package: Vec<String>,
    target_project: Vec<String>,
    target_package: Vec<String>,
    creator: Vec<String>,
    who: Vec<String>,
    reviewer: Vec<String>,
    state: Vec<String>,
    types: Vec<String>,
    exclude_target_project: Vec<String>,
    with_history: bool,
    with_full_history: bool,
    since: Option<String>,
    limit: Option<usize>,
) -> Result<RequestSearch, GeectlError> {
    if let Some(limit) = limit {
        if limit == 0 {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "a request search limit must be at least 1",
            ));
        }
    }
    let mut states = Vec::new();
    for value in &state {
        states.push(parse_state(value)?);
    }
    let mut search = RequestSearch::new()
        .request_ids(request_ids)
        .projects(project)
        .packages(package)
        .source_projects(source_project)
        .source_packages(source_package)
        .target_projects(target_project)
        .target_packages(target_package)
        .creators(creator)
        .who(who)
        .reviewers(reviewer)
        .types(types)
        .exclude_target_projects(exclude_target_project)
        .with_history(with_history)
        .with_full_history(with_full_history);
    if !states.is_empty() {
        search = search.states(states);
    }
    if let Some(since) = since {
        search = search.since(since);
    }
    if let Some(limit) = limit {
        search = search.limit(limit);
    }
    Ok(search)
}

/// File a submit request, or refile one when `supersede` names the
/// old request: build the payload from the flags, then file it.
/// Cleanup is the default on the payload; `--without-cleanup`
/// selects update instead.
fn file_request(
    debug: &Debug,
    live: bool,
    file: &FileArgs,
    supersede: Option<&str>,
) -> Result<(Value, String), GeectlError> {
    let mut cves = Vec::new();
    for value in &file.cve {
        cves.push(CveId::parse(value)?);
    }
    let mut payload = SrPayload::new(
        &file.source_project,
        &file.source_package,
        &file.target_project,
        &file.target_package,
        &file.description,
        &file.changes_entry,
        &cves,
    )?;
    if file.without_cleanup {
        payload = payload.without_cleanup();
    }
    let dry_run = DryRun::new(!live).with_debug(debug.clone());
    // The freshness check's Gitea client resolves lazily, live-only,
    // after the source meta says `<scmsync>`: a dry-run plan reads
    // nothing, so no client is wired there; live, the filing path
    // resolves the client itself, so a malformed `gitea_url` /
    // instance config never fails an OBS-native filing that never
    // reaches Gitea.
    let gitea_http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let gitea_http = if live {
        Some(&gitea_http as &dyn HttpClient)
    } else {
        None
    };
    with_client(debug, live, |client| {
        let filing = SrFiling::new(client, &dry_run)
            .with_debug(debug.clone())
            .with_gitea_http(gitea_http);
        let filed_request = match supersede {
            Some(old_id) => filing.refile(&payload, old_id)?,
            None => filing.file(&payload)?,
        };
        // The filed JSON and the text both carry the parsed CVE ids:
        // the raw `--cve` strings stay only in the CLI args. Both
        // also carry the superseded ids: a filing that supersedes
        // other requests says so, instead of reporting only the new
        // id.
        let id = filed_request.as_ref().map(|filed| filed.id.clone());
        let filed = filed_payload(&id, &dry_run, |id| {
            file_request_json(id, filed_request.as_ref(), file, &cves)
        });
        let text = file_request_text(
            id.as_deref(),
            supersede,
            filed_request
                .as_ref()
                .map(|filed| filed.superseded.as_slice()),
            file,
            &cves,
        );
        Ok((filed, text))
    })
}

/// The filed JSON for `file` / `refile`: the filed (or would-file)
/// request id, the ids it superseded, the source and target
/// coordinates, the parsed CVE ids, and the cleanup choice. The CVEs
/// are the parsed ids, never the raw `--cve` strings. The superseded
/// ids are null in dry-run — the open requests are unknown there —
/// and an array in live mode, empty when nothing was open.
fn file_request_json(
    id: Option<&str>,
    filed: Option<&FiledRequest>,
    file: &FileArgs,
    cves: &[CveId],
) -> Value {
    let cve_strings: Vec<&str> = cves.iter().map(CveId::as_str).collect();
    let superseded = filed.map_or(Value::Null, |filed| {
        Value::Array(
            filed
                .superseded
                .iter()
                .map(|id| request_id_json(id))
                .collect(),
        )
    });
    json!({
        "id": id.map(request_id_json).unwrap_or(Value::Null),
        "superseded": superseded,
        "source_project": file.source_project,
        "source_package": file.source_package,
        "target_project": file.target_project,
        "target_package": file.target_package,
        "description": file.description,
        "changes_entry": file.changes_entry,
        "cve": cve_strings,
        "without_cleanup": file.without_cleanup,
    })
}

/// The filing text for `file` / `refile`: the filed (or
/// would-file) request id, the source and target coordinates, the
/// CVE ids when any were claimed, and the superseded ids when any
/// were superseded. The CVEs belong to the new request, so they ride
/// the coordinates, before the superseded ids. CVE ids are
/// identifiers (design rule 1), so they ride the text; the
/// description and the `.changes` entry are long free text and stay
/// JSON-only.
fn file_request_text(
    id: Option<&str>,
    supersede: Option<&str>,
    superseded: Option<&[String]>,
    file: &FileArgs,
    cves: &[CveId],
) -> String {
    let cves = if cves.is_empty() {
        String::new()
    } else {
        format!(
            " ({})",
            cves.iter()
                .map(CveId::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let coords = format!(
        "{}/{} to {}/{}",
        file.source_project, file.source_package, file.target_project, file.target_package
    );
    let superseded = match superseded {
        Some(ids) if !ids.is_empty() => format!(", superseded {}", ids.join(", ")),
        _ => String::new(),
    };
    match (id, supersede) {
        (Some(id), Some(_old_id)) => {
            format!("filed request {id} for {coords}{cves}{superseded}")
        }
        (Some(id), None) => format!("filed request {id} for {coords}{cves}{superseded}"),
        (None, Some(old_id)) => {
            format!("dry-run: would file request for {coords}{cves} and supersede {old_id}")
        }
        (None, None) => format!("dry-run: would file request for {coords}{cves}"),
    }
}

/// File a maintenance incident request, or refile one when
/// `supersede` names the old request: build the payload from the
/// flags, then file it. The branch survives the incident by
/// default; `--with-cleanup` opts into deleting it on accept.
fn file_maintenance_incident(
    debug: &Debug,
    live: bool,
    incident: &MaintenanceIncidentArgs,
    supersede: Option<&str>,
) -> Result<(Value, String), GeectlError> {
    let source_packages: Vec<&str> = incident.source_package.iter().map(String::as_str).collect();
    let mut payload = MaintenanceIncidentPayload::new(
        &incident.source_project,
        &source_packages,
        &incident.target_project,
        &incident.description,
    )?;
    if let Some(release_project) = &incident.release_project {
        payload = payload.with_release_project(release_project)?;
    }
    if incident.with_cleanup {
        payload = payload.with_cleanup()?;
    }
    let dry_run = DryRun::new(!live).with_debug(debug.clone());
    with_client(debug, live, |client| {
        let filing = SrFiling::new(client, &dry_run).with_debug(debug.clone());
        let id = match supersede {
            Some(old_id) => filing.refile_maintenance_incident(&payload, old_id)?,
            None => filing.file_maintenance_incident(&payload)?,
        };
        let filed = filed_payload(&id, &dry_run, |id| {
            json!({
                "id": id.map(request_id_json).unwrap_or(Value::Null),
                "source_project": incident.source_project,
                "source_packages": incident.source_package,
                "target_project": incident.target_project,
                "release_project": incident.release_project,
                "description": incident.description,
                "with_cleanup": incident.with_cleanup,
            })
        });
        let text = file_maintenance_incident_text(id.as_deref(), supersede, incident);
        Ok((filed, text))
    })
}

/// The filing text for `file-maintenance-incident` /
/// `refile-maintenance-incident`: the filed (or would-file) request
/// id, every source package, the source and target projects, the
/// release project when given, and the superseded id when refiling.
/// The description is long free text and stays JSON-only.
fn file_maintenance_incident_text(
    id: Option<&str>,
    supersede: Option<&str>,
    incident: &MaintenanceIncidentArgs,
) -> String {
    let packages = incident.source_package.join(", ");
    let release = match &incident.release_project {
        Some(release) => format!(", releasing into {release}"),
        None => String::new(),
    };
    let coords = format!(
        "{packages} from {} to {}{release}",
        incident.source_project, incident.target_project
    );
    match (id, supersede) {
        (Some(id), Some(old_id)) => {
            format!("filed maintenance incident request {id} for {coords}, superseded {old_id}")
        }
        (Some(id), None) => {
            format!("filed maintenance incident request {id} for {coords}")
        }
        (None, Some(old_id)) => format!(
            "dry-run: would file maintenance incident request for {coords} and supersede {old_id}"
        ),
        (None, None) => {
            format!("dry-run: would file maintenance incident request for {coords}")
        }
    }
}

/// Add the dry-run `plan` to a mutation payload, like the other
/// groups' `finish()`: the recorded plan lines, only when this run
/// is a dry run. A live run's plan is empty by construction (the
/// library records plan lines only when dry-run is enabled), so the
/// field is omitted there.
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

/// The payload for a filing result: the new id in live mode, `null`
/// in dry-run, plus `dry_run` and the fields the caller passed, so
/// the payload is complete in both modes.
fn filed_payload(
    id: &Option<String>,
    dry_run: &DryRun,
    with_id: impl FnOnce(Option<&str>) -> Value,
) -> Value {
    let mut payload = with_id(id.as_deref());
    if let Value::Object(map) = &mut payload {
        map.insert("dry_run".to_string(), Value::Bool(dry_run.enabled()));
    }
    add_plan(&mut payload, dry_run);
    payload
}

/// One state-change act (`accept`, `decline`, `revoke`,
/// `change-state`): change the state, or record the plan in dry-run.
fn act(
    debug: &Debug,
    id: &str,
    state: RequestState,
    comment: &str,
    force: bool,
    live: bool,
    verb: &str,
) -> Result<(Value, String), GeectlError> {
    let dry_run = DryRun::new(!live).with_debug(debug.clone());
    with_client(debug, live, |client| {
        client.change_state(id, state, comment, force, &dry_run)?;
        let mut payload = json!({
            "id": request_id_json(id),
            "state": state.as_str(),
            "comment": comment,
            "dry_run": dry_run.enabled(),
        });
        add_plan(&mut payload, &dry_run);
        let mut text = if dry_run.enabled() {
            format!("dry-run: would change request {id} to {verb}")
        } else {
            format!("changed request {id} to {verb}")
        };
        if !comment.is_empty() {
            text.push_str(&format!(" — {comment}"));
        }
        Ok((payload, text))
    })
}

/// Accept a submit request, and forward it to its link parent
/// afterwards unless `--no-forward` is given. The accept stands
/// on its own: once it lands, the request merges, and a forward
/// that is refused — or fails any other way — is reported with
/// the work left queued in the devel project. Nothing is declined
/// on a refusal: declining the new forward request cannot change
/// the accepted original. In dry-run nothing is accepted, so
/// nothing is forwarded either — the plan notes the forward that
/// would follow the accept.
fn accept(
    debug: &Debug,
    id: &str,
    comment: &str,
    force: bool,
    no_forward: bool,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let (mut payload, mut text) = act(
        debug,
        id,
        RequestState::Accepted,
        comment,
        force,
        live,
        "accepted",
    )?;
    if no_forward {
        return Ok((payload, text));
    }
    if !live {
        if let Value::Object(map) = &mut payload {
            if let Some(Value::Array(plan)) = map.get_mut("plan") {
                plan.push(Value::String(format!(
                    "[dry-run] would: forward request {id} after accept"
                )));
            }
        }
        text.push_str("; would forward after accept");
        return Ok((payload, text));
    }
    // Live: the accept landed; forward the now-accepted request.
    let (forward_payload, forward_text) = forward(debug, id, None, live).map_err(|err| {
        GeectlError::new(
            err.kind(),
            format!("accepted request {id}, but the forward failed: {err}"),
        )
    })?;
    if let Value::Object(map) = &mut payload {
        map.insert("forward".to_string(), forward_payload);
    }
    text.push_str(&format!("; {forward_text}"));
    Ok((payload, text))
}

/// Forward an accepted submit request: resolve each submit action's
/// hop and file the forwards, or record the plan in dry-run. The
/// accept-coupled forward, shared by `request accept` and
/// `pull-request merge` (OBS arm): there is no standalone forward
/// leaf — the retry path after an accept with `--no-forward` is a
/// direct submit from the devel project to Factory.
pub(crate) fn forward(
    debug: &Debug,
    id: &str,
    message: Option<&str>,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let dry_run = DryRun::new(!live).with_debug(debug.clone());
    with_client(debug, live, |client| {
        let result = client.forward_request(id, message, &dry_run)?;
        Ok((
            forward_json(&result, &dry_run),
            forward_text(&result, &dry_run),
        ))
    })
}

/// The forward result as JSON: the original id, one outcome per
/// submit action, and the dry-run plan when dry-running.
fn forward_json(result: &ForwardResult, dry_run: &DryRun) -> Value {
    let mut payload = json!({
        "id": result.id,
        "actions": result.actions.iter().map(forward_action_json).collect::<Vec<_>>(),
        "dry_run": dry_run.enabled(),
    });
    add_plan(&mut payload, dry_run);
    payload
}

/// One forward outcome as JSON: the forwarded package, the hop
/// target — the maintenance project for an incident, with the
/// maintained hop target as `release_project` — the outcome, the
/// new request id when one was filed, and the skip reason when the
/// action was not forwarded.
pub(crate) fn forward_action_json(action: &ForwardActionResult) -> Value {
    match action {
        ForwardActionResult::Forwarded {
            source,
            target,
            new_id,
        } => json!({
            "source": source,
            "target": target,
            "outcome": "forwarded",
            "new_id": new_id.as_ref().map(|id| request_id_json(id)).unwrap_or(Value::Null),
            "reason": Value::Null,
        }),
        ForwardActionResult::Incident {
            source,
            target,
            release_project,
            new_id,
        } => json!({
            "source": source,
            "target": target,
            "release_project": release_project,
            "outcome": "incident",
            "new_id": new_id.as_ref().map(|id| request_id_json(id)).unwrap_or(Value::Null),
            "reason": Value::Null,
        }),
        ForwardActionResult::Skipped { source, reason } => json!({
            "source": source,
            "target": Value::Null,
            "outcome": "skipped",
            "new_id": Value::Null,
            "reason": reason,
        }),
    }
}

/// The forward result as text: one header line naming the
/// request and whether anything was forwarded, then one line per
/// action. The full result is the JSON payload.
fn forward_text(result: &ForwardResult, dry_run: &DryRun) -> String {
    let filed = result.actions.iter().any(|action| {
        matches!(
            action,
            ForwardActionResult::Forwarded { .. } | ForwardActionResult::Incident { .. }
        )
    });
    let mut text = if result.actions.is_empty() {
        if dry_run.enabled() {
            format!(
                "dry-run: request {} has no submit actions to forward",
                result.id
            )
        } else {
            format!("request {} has no submit actions to forward", result.id)
        }
    } else if filed {
        if dry_run.enabled() {
            format!("dry-run: would forward request {}", result.id)
        } else {
            format!("forwarded request {}", result.id)
        }
    } else if dry_run.enabled() {
        format!("dry-run: request {} has no forwardable actions", result.id)
    } else {
        format!("request {}: nothing forwarded", result.id)
    };
    if result.actions.is_empty() {
        return text;
    }
    for action in &result.actions {
        text.push('\n');
        text.push_str(&forward_action_text(action, dry_run));
    }
    text
}

/// One forward outcome as a text line: the hop and the new request
/// id, or the skip with its reason.
pub(crate) fn forward_action_text(action: &ForwardActionResult, dry_run: &DryRun) -> String {
    match action {
        ForwardActionResult::Forwarded {
            source,
            target,
            new_id,
        } => {
            if dry_run.enabled() {
                format!("would forward {source} to {target}")
            } else {
                format!(
                    "forwarded {source} to {target} as request {}",
                    new_id.as_deref().unwrap_or("?")
                )
            }
        }
        ForwardActionResult::Incident {
            source,
            target,
            release_project,
            new_id,
        } => {
            if dry_run.enabled() {
                format!(
                    "would file a maintenance incident for {source} to {target} (release project {release_project})"
                )
            } else {
                format!(
                    "filed maintenance incident {} for {source} to {target} (release project {release_project})",
                    new_id.as_deref().unwrap_or("?")
                )
            }
        }
        ForwardActionResult::Skipped { source, reason } => {
            format!("skipped {source}: {reason}")
        }
    }
}

/// Build the OBS client from the effective config and the
/// environment credentials for the configured base URL (resolved
/// for the URL from its instance alias — see
/// `auth::resolve_obs_for_url`), wiring the debug sink
/// through. The client borrows the transport, so both live in this
/// scope and the caller runs inside it.
///
/// `live` says whether this run may perform live work. A dry-run
/// mutation never touches the server, so a half-configured
/// credential set (a username without a password source, or the
/// reverse) must not fail it: the resolution error is replaced with
/// no credentials and the plan prints. The dropped error is reported
/// as a stderr warning (and on the debug channel), so a dry run still
/// shows that the same credentials would fail a `--live` run — the
/// debug channel alone would leave it silent without `--debug`.
/// A live run — and every
/// read, which is always live — still resolves credentials up front,
/// so a misconfiguration fails fast with the precise error.
fn with_client<T>(
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

// ---------- JSON converters ----------
// The payloads name the same fields as the library types, with
// `null` for absent optionals, so a consumer can read them without
// knowing the server's XML shape.

/// The request id as a JSON number: the server's ids are numeric,
/// and `show`/`search` already emit numbers, so every arm agrees.
/// Falls back to the string if the id is not numeric — unreachable
/// for a server-validated id, but the payload stays total without
/// an unwrap.
fn request_id_json(id: &str) -> Value {
    match id.parse::<u64>() {
        Ok(number) => json!(number),
        Err(_) => json!(id),
    }
}

fn request_json(request: &Request) -> Value {
    json!({
        "id": request.id,
        "creator": request.creator,
        "title": request.title,
        "description": request.description,
        "actions": request.actions.iter().map(action_json).collect::<Vec<_>>(),
        "priority": request.priority.map(|priority| priority.as_str()),
        "state": state_json(&request.state),
        "reviews": request.reviews.iter().map(review_json).collect::<Vec<_>>(),
        "history": request.history.iter().map(history_json).collect::<Vec<_>>(),
        "accept_at": request.accept_at,
    })
}

fn action_json(action: &RequestAction) -> Value {
    match action {
        RequestAction::Submit {
            source_project,
            source_package,
            source_rev,
            target_project,
            target_package,
            sourceupdate,
            updatelink,
            makeoriginolder,
            acceptinfo,
        } => json!({
            "type": "submit",
            "source_project": source_project,
            "source_package": source_package,
            "source_rev": source_rev,
            "target_project": target_project,
            "target_package": target_package,
            "sourceupdate": sourceupdate.map(|update| update.as_str()),
            "updatelink": updatelink,
            "makeoriginolder": makeoriginolder,
            "acceptinfo": acceptinfo.as_ref().map(accept_info_json),
        }),
        RequestAction::Delete {
            target_project,
            target_package,
            target_repository,
        } => json!({
            "type": "delete",
            "target_project": target_project,
            "target_package": target_package,
            "target_repository": target_repository,
        }),
        RequestAction::AddRole {
            target_project,
            target_package,
            person,
            group,
        } => json!({
            "type": "add_role",
            "target_project": target_project,
            "target_package": target_package,
            "person": person.as_ref().map(|(name, role)| json!({ "name": name, "role": role })),
            "group": group.as_ref().map(|(name, role)| json!({ "name": name, "role": role })),
        }),
        RequestAction::SetBugowner {
            target_project,
            target_package,
            person_name,
            group_name,
        } => json!({
            "type": "set_bugowner",
            "target_project": target_project,
            "target_package": target_package,
            "person_name": person_name,
            "group_name": group_name,
        }),
        RequestAction::ChangeDevel {
            source_project,
            source_package,
            target_project,
            target_package,
        } => json!({
            "type": "change_devel",
            "source_project": source_project,
            "source_package": source_package,
            "target_project": target_project,
            "target_package": target_package,
        }),
        RequestAction::MaintenanceIncident {
            source_project,
            source_package,
            target_project,
            target_package,
            sourceupdate,
            makeoriginolder,
            acceptinfo,
        } => json!({
            "type": "maintenance_incident",
            "source_project": source_project,
            "source_package": source_package,
            "target_project": target_project,
            "target_package": target_package,
            "sourceupdate": sourceupdate.map(|update| update.as_str()),
            "makeoriginolder": makeoriginolder,
            "acceptinfo": acceptinfo.as_ref().map(accept_info_json),
        }),
        RequestAction::MaintenanceRelease {
            source_project,
            source_package,
            target_project,
            target_package,
            sourceupdate,
            makeoriginolder,
            acceptinfo,
        } => json!({
            "type": "maintenance_release",
            "source_project": source_project,
            "source_package": source_package,
            "target_project": target_project,
            "target_package": target_package,
            "sourceupdate": sourceupdate.map(|update| update.as_str()),
            "makeoriginolder": makeoriginolder,
            "acceptinfo": acceptinfo.as_ref().map(accept_info_json),
        }),
        RequestAction::Release {
            source_project,
            source_package,
            target_project,
            target_package,
            sourceupdate,
            makeoriginolder,
            acceptinfo,
        } => json!({
            "type": "release",
            "source_project": source_project,
            "source_package": source_package,
            "target_project": target_project,
            "target_package": target_package,
            "sourceupdate": sourceupdate.map(|update| update.as_str()),
            "makeoriginolder": makeoriginolder,
            "acceptinfo": acceptinfo.as_ref().map(accept_info_json),
        }),
    }
}

fn accept_info_json(info: &AcceptInfo) -> Value {
    json!({
        "rev": info.rev,
        "srcmd5": info.srcmd5,
        "oproject": info.oproject,
        "opackage": info.opackage,
        "osrcmd5": info.osrcmd5,
        "xsrcmd5": info.xsrcmd5,
        "oxsrcmd5": info.oxsrcmd5,
    })
}

fn state_json(state: &RequestStateInfo) -> Value {
    json!({
        "name": state.name.as_str(),
        "who": state.who,
        "when": state.when,
        "created": state.created,
        "superseded_by": state.superseded_by,
        "approver": state.approver,
        "comment": state.comment,
    })
}

fn review_json(review: &RequestReview) -> Value {
    json!({
        "state": review.state.as_str(),
        "when": review.when,
        "who": review.who,
        "by_user": review.by_user,
        "by_group": review.by_group,
        "by_project": review.by_project,
        "by_package": review.by_package,
        "comment": review.comment,
        "history": review.history.iter().map(history_json).collect::<Vec<_>>(),
    })
}

fn history_json(entry: &RequestHistory) -> Value {
    json!({
        "who": entry.who,
        "when": entry.when,
        "description": entry.description,
        "comment": entry.comment,
    })
}

fn comment_json(comment: &RequestComment) -> Value {
    json!({
        "id": comment.id,
        "who": comment.who,
        "when": comment.when,
        "parent_id": comment.parent_id,
        "body": comment.body,
    })
}

// ---------- Text renderers ----------
// One line per value, `key: value`; lists are tables with a header
// and space-aligned columns, never truncated; empty results print an
// explicit line. Every id, name and revision in the JSON payload
// appears in the text rendering of `show`, `log` and `list-comments` —
// except the acceptinfo hashes (`srcmd5`, `osrcmd5`, `xsrcmd5`,
// `oxsrcmd5`) and origin (`oproject`, `opackage`) in `show`, which
// stay in the JSON payload (see the module docs);
// `diff` is a content command (verbatim text, coordinates in JSON);
// an empty diff prints `no diff for request <id>` instead of empty
// stdout (design rule 5) — a request with no submit action has no
// diff, and the server returns an empty body for it (verified live
// against api.opensuse.org, request 43359: `add_role` +
// `set_bugowner` only). The search table and the one-line action
// texts carry the documented omissions (see the module docs and
// design rule 1).

/// The diff text: the diff verbatim, except an empty diff, which
/// renders an explicit line naming the request.
fn render_diff_text(id: &str, diff: &str) -> String {
    if diff.is_empty() {
        format!("no diff for request {id}")
    } else {
        diff.to_string()
    }
}

fn render_request_text(request: &Request) -> String {
    let mut lines = vec![
        format!("id: {}", request.id),
        format!("state: {}", request.state.name.as_str()),
        format!("state by: {}", request.state.who),
        format!("state at: {}", request.state.when),
    ];
    if let Some(created) = &request.state.created {
        lines.push(format!("created: {created}"));
    }
    lines.push(format!("creator: {}", request.creator));
    if let Some(title) = &request.title {
        lines.push(format!("title: {}", strip_terminal_controls(title)));
    }
    if !request.description.is_empty() {
        lines.push(format!(
            "description: {}",
            strip_terminal_controls(&request.description)
        ));
    }
    if let Some(priority) = request.priority {
        lines.push(format!("priority: {}", priority.as_str()));
    }
    if let Some(accept_at) = &request.accept_at {
        lines.push(format!("accept at: {accept_at}"));
    }
    if let Some(superseded_by) = request.state.superseded_by {
        lines.push(format!("superseded by: {superseded_by}"));
    }
    if let Some(approver) = &request.state.approver {
        lines.push(format!("approver: {approver}"));
    }
    if !request.state.comment.is_empty() {
        lines.push(format!(
            "state comment: {}",
            strip_terminal_controls(&request.state.comment)
        ));
    }
    for (index, action) in request.actions.iter().enumerate() {
        lines.push(format!("action {}: {}", index + 1, action_text(action)));
    }
    for (index, review) in request.reviews.iter().enumerate() {
        lines.push(review_text(index + 1, review));
        for (history_index, entry) in review.history.iter().enumerate() {
            lines.push(format!(
                "review {} history {}: {}",
                index + 1,
                history_index + 1,
                history_text(entry)
            ));
        }
    }
    for (index, entry) in request.history.iter().enumerate() {
        lines.push(format!("history {}: {}", index + 1, history_text(entry)));
    }
    lines.join("\n")
}

/// The full action line for `request show`: type, source, target,
/// and the options that change what accept does.
fn action_text(action: &RequestAction) -> String {
    match action {
        RequestAction::Submit {
            source_project,
            source_package,
            source_rev,
            target_project,
            target_package,
            sourceupdate,
            updatelink,
            makeoriginolder,
            acceptinfo,
        } => {
            let mut line = format!("submit {source_project}/{source_package}");
            // The revision qualifies the source: `submit SRC rev N ->
            // TARGET`, as the design doc's worked example pins it.
            if let Some(rev) = source_rev {
                line.push_str(&format!(" rev {rev}"));
            }
            line.push_str(&format!(
                " -> {}",
                target_text(target_project.as_deref(), target_package.as_deref())
            ));
            if let Some(update) = sourceupdate {
                line.push_str(&format!(" ({})", update.as_str()));
            }
            if *updatelink {
                line.push_str(" (updatelink)");
            }
            if *makeoriginolder {
                line.push_str(" (makeoriginolder)");
            }
            if let Some(info) = acceptinfo {
                line.push_str(&format!(" (accepted rev {})", info.rev));
            }
            line
        }
        RequestAction::Delete {
            target_project,
            target_package,
            target_repository,
        } => {
            let mut line = format!(
                "delete {}",
                target_text(Some(target_project), target_package.as_deref())
            );
            if let Some(repository) = target_repository {
                line.push_str(&format!(" (repository {repository})"));
            }
            line
        }
        RequestAction::AddRole {
            target_project,
            target_package,
            person,
            group,
        } => {
            let mut who = Vec::new();
            if let Some((name, role)) = person {
                who.push(format!("user {name} as {role}"));
            }
            if let Some((name, role)) = group {
                who.push(format!("group {name} as {role}"));
            }
            format!(
                "add_role {} on {}",
                who.join(", "),
                target_text(Some(target_project), target_package.as_deref())
            )
        }
        RequestAction::SetBugowner {
            target_project,
            target_package,
            person_name,
            group_name,
        } => {
            let mut who = Vec::new();
            if let Some(name) = person_name {
                who.push(format!("user {name}"));
            }
            if let Some(name) = group_name {
                who.push(format!("group {name}"));
            }
            format!(
                "set_bugowner {} on {}",
                who.join(", "),
                target_text(Some(target_project), target_package.as_deref())
            )
        }
        RequestAction::ChangeDevel {
            source_project,
            source_package,
            target_project,
            target_package,
        } => format!(
            "change_devel {source_project}/{source_package} -> {target_project}/{target_package}"
        ),
        RequestAction::MaintenanceIncident {
            source_project,
            source_package,
            target_project,
            target_package,
            sourceupdate,
            makeoriginolder,
            acceptinfo,
        } => {
            let mut line = format!(
                "maintenance_incident {source_project}/{source_package} -> {}",
                target_text(Some(target_project), target_package.as_deref())
            );
            if let Some(update) = sourceupdate {
                line.push_str(&format!(" ({})", update.as_str()));
            }
            if *makeoriginolder {
                line.push_str(" (makeoriginolder)");
            }
            if let Some(info) = acceptinfo {
                line.push_str(&format!(" (accepted rev {})", info.rev));
            }
            line
        }
        RequestAction::MaintenanceRelease {
            source_project,
            source_package,
            target_project,
            target_package,
            sourceupdate,
            makeoriginolder,
            acceptinfo,
        } => {
            let mut line = format!(
                "maintenance_release {source_project}/{source_package} -> {}",
                target_text(Some(target_project), target_package.as_deref())
            );
            if let Some(update) = sourceupdate {
                line.push_str(&format!(" ({})", update.as_str()));
            }
            if *makeoriginolder {
                line.push_str(" (makeoriginolder)");
            }
            if let Some(info) = acceptinfo {
                line.push_str(&format!(" (accepted rev {})", info.rev));
            }
            line
        }
        RequestAction::Release {
            source_project,
            source_package,
            target_project,
            target_package,
            sourceupdate,
            makeoriginolder,
            acceptinfo,
        } => {
            let mut line = format!(
                "release {source_project}/{source_package} -> {}",
                target_text(Some(target_project), target_package.as_deref())
            );
            if let Some(update) = sourceupdate {
                line.push_str(&format!(" ({})", update.as_str()));
            }
            if *makeoriginolder {
                line.push_str(" (makeoriginolder)");
            }
            if let Some(info) = acceptinfo {
                line.push_str(&format!(" (accepted rev {})", info.rev));
            }
            line
        }
    }
}

/// A `project/package` target, or the project alone, or `(no target)`
/// when a submit names neither.
fn target_text(project: Option<&str>, package: Option<&str>) -> String {
    match (project, package) {
        (Some(project), Some(package)) => format!("{project}/{package}"),
        (Some(project), None) => project.to_string(),
        (None, Some(package)) => package.to_string(),
        (None, None) => "(no target)".to_string(),
    }
}

/// The compact action summary for the `search` table: type plus the
/// package it acts on. Deliberately narrow: target names and
/// `source_rev` are omitted here (see the module docs),
/// and stay in the JSON payload and in `request show`. A party-less
/// `add_role` / `set_bugowner` (degenerate server data) renders
/// `(unknown party)`, never a bare sentinel.
fn action_summary(action: &RequestAction) -> String {
    match action {
        RequestAction::Submit { source_package, .. } => format!("submit {source_package}"),
        RequestAction::Delete {
            target_package,
            target_project,
            ..
        } => format!(
            "delete {}",
            target_package.as_deref().unwrap_or(target_project)
        ),
        RequestAction::AddRole { person, group, .. } => format!(
            "add_role {}",
            person
                .as_ref()
                .map(|(name, _)| name.as_str())
                .or_else(|| group.as_ref().map(|(name, _)| name.as_str()))
                .unwrap_or("(unknown party)")
        ),
        RequestAction::SetBugowner {
            person_name,
            group_name,
            ..
        } => format!(
            "set_bugowner {}",
            person_name
                .as_deref()
                .or(group_name.as_deref())
                .unwrap_or("(unknown party)")
        ),
        RequestAction::ChangeDevel { source_package, .. } => {
            format!("change_devel {source_package}")
        }
        RequestAction::MaintenanceIncident { source_package, .. } => {
            format!("maintenance_incident {source_package}")
        }
        RequestAction::MaintenanceRelease { source_package, .. } => {
            format!("maintenance_release {source_package}")
        }
        RequestAction::Release { source_package, .. } => format!("release {source_package}"),
    }
}

fn review_text(index: usize, review: &RequestReview) -> String {
    let target = review_target_text(review);
    let mut line = if target.is_empty() {
        format!("review {index}: state {}", review.state.as_str())
    } else {
        format!(
            "review {index}: by {target} state {}",
            review.state.as_str()
        )
    };
    match (&review.who, &review.when) {
        (Some(who), Some(when)) => line.push_str(&format!(", by {who} at {when}")),
        (Some(who), None) => line.push_str(&format!(", by {who}")),
        (None, Some(when)) => line.push_str(&format!(", at {when}")),
        (None, None) => {}
    }
    if !review.comment.is_empty() {
        line.push_str(&format!(" — {}", strip_terminal_controls(&review.comment)));
    }
    line
}

fn review_target_text(review: &RequestReview) -> String {
    let mut parts = Vec::new();
    if let Some(by_user) = &review.by_user {
        parts.push(format!("user:{by_user}"));
    }
    if let Some(by_group) = &review.by_group {
        parts.push(format!("group:{by_group}"));
    }
    if let Some(by_project) = &review.by_project {
        parts.push(format!("project:{by_project}"));
    }
    if let Some(by_package) = &review.by_package {
        parts.push(format!("package:{by_package}"));
    }
    parts.join(", ")
}

fn history_text(entry: &RequestHistory) -> String {
    let mut line = format!("{} {}", entry.who, entry.when);
    if !entry.description.is_empty() {
        line.push_str(&format!(" {}", strip_terminal_controls(&entry.description)));
    }
    if !entry.comment.is_empty() {
        line.push_str(&format!(" — {}", strip_terminal_controls(&entry.comment)));
    }
    line
}

fn render_history_text(id: &str, history: &[RequestHistory]) -> String {
    if history.is_empty() {
        return format!("no history for request {id}");
    }
    let mut lines = vec![format!("id: {id}")];
    lines.extend(
        history
            .iter()
            .enumerate()
            .map(|(index, entry)| format!("history {}: {}", index + 1, history_text(entry))),
    );
    lines.join("\n")
}

/// Render stored Markdown as plaintext for text output.
///
/// The typed model keeps the raw Markdown and JSON output carries it
/// verbatim; only the human-readable text rendering converts. Inline
/// formatting is stripped, link text is kept and the URL dropped,
/// headings and list items keep their text. A literal `<foo>` in the
/// body passes through unchanged: it is user-typed text, not markup.
///
/// Block spacing: a paragraph ends with one newline, so
/// multi-paragraph bodies render as separated lines with no blank
/// line between them; a loose list renders its items
/// blank-separated. Lists are flattened: nested items lose their
/// indentation and a nested ordered list restarts its numbering;
/// the blank line between a tight nested list and a following
/// paragraph in the same item is not preserved.
fn markdown_to_plaintext(markdown: &str) -> String {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let mut out = String::new();
    let mut lists: Vec<Option<u64>> = Vec::new();
    // Looseness per list level: true when the list's parent item
    // ended a paragraph before the list started. The enclosing
    // item's end keeps its newline in that case, preserving the
    // outer list's blank line.
    let mut list_loose: Vec<bool> = Vec::new();
    // True when the previous event closed a (nested) list. Both
    // flags are cleared by any event other than an item or list end,
    // so they cannot leak into a later list.
    let mut just_closed_list = false;
    let mut closed_loose = false;
    // True while an item's first block is still to come: a block tag
    // there shares the marker line ("- head") instead of starting a
    // line of its own.
    let mut at_item_start = false;
    for event in Parser::new(markdown) {
        match &event {
            Event::End(TagEnd::Item) | Event::End(TagEnd::List(_)) => {}
            _ => {
                just_closed_list = false;
                closed_loose = false;
            }
        }
        match &event {
            Event::Start(Tag::Item) => at_item_start = true,
            Event::Start(Tag::Heading { .. })
            | Event::Start(Tag::CodeBlock(_))
            | Event::Start(Tag::BlockQuote(_))
            | Event::Start(Tag::HtmlBlock)
            | Event::Rule => {}
            _ => at_item_start = false,
        }
        match event {
            Event::Text(text) | Event::Code(text) => out.push_str(&text),
            Event::Html(html) | Event::InlineHtml(html) => out.push_str(&html),
            Event::Start(Tag::Paragraph)
            | Event::Start(Tag::Link { .. })
            | Event::Start(Tag::Image { .. })
            | Event::Start(Tag::Emphasis)
            | Event::Start(Tag::Strong) => {}
            Event::Start(Tag::Heading { .. })
            | Event::Start(Tag::CodeBlock(_))
            | Event::Start(Tag::BlockQuote(_))
            | Event::Start(Tag::HtmlBlock) => {
                // Block tags start on their own line; at an item's
                // start the block shares the marker line ("- head").
                if !at_item_start && !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            Event::Start(
                Tag::FootnoteDefinition(_)
                | Tag::DefinitionList
                | Tag::DefinitionListTitle
                | Tag::DefinitionListDefinition
                | Tag::Table(_)
                | Tag::TableHead
                | Tag::TableRow
                | Tag::TableCell
                | Tag::Strikethrough
                | Tag::Superscript
                | Tag::Subscript
                | Tag::MetadataBlock(_),
            ) => {}
            Event::End(TagEnd::Paragraph) => {
                out.push('\n');
            }
            Event::End(TagEnd::Heading(_)) => {
                // A heading with no text emits no Text event; the
                // line is already ended, so pushing another newline
                // here would double-count it into a blank line.
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                // Code block text already ends with a newline;
                // pushing another would double-count it into a
                // blank line.
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            Event::End(TagEnd::Link)
            | Event::End(TagEnd::Image)
            | Event::End(TagEnd::Emphasis)
            | Event::End(TagEnd::Strong) => {}
            Event::End(
                TagEnd::BlockQuote(_)
                | TagEnd::HtmlBlock
                | TagEnd::FootnoteDefinition
                | TagEnd::DefinitionList
                | TagEnd::DefinitionListTitle
                | TagEnd::DefinitionListDefinition
                | TagEnd::Table
                | TagEnd::TableHead
                | TagEnd::TableRow
                | TagEnd::TableCell
                | TagEnd::Strikethrough
                | TagEnd::Superscript
                | TagEnd::Subscript
                | TagEnd::MetadataBlock(_),
            ) => {}
            Event::Start(Tag::List(start)) => {
                // A list nested inside an item starts on its own
                // line: the parent item's text ends with no newline
                // of its own in a tight list, and with one in a
                // loose list — a blank line needs one more either way.
                let loose = !lists.is_empty() && out.ends_with('\n');
                if !lists.is_empty() {
                    out.push('\n');
                }
                lists.push(start);
                list_loose.push(loose);
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                closed_loose = list_loose.pop().unwrap_or(false);
                just_closed_list = true;
            }
            Event::Start(Tag::Item) => {
                let marker = match lists.last_mut() {
                    Some(Some(number)) => {
                        let marker = format!("{number}. ");
                        *number += 1;
                        marker
                    }
                    _ => "- ".to_string(),
                };
                out.push_str(&marker);
            }
            Event::End(TagEnd::Item) => {
                // After a nested list, the enclosing item adds a
                // newline only when the parent was loose and no
                // blank line is already there: in a tight list the
                // nested item's end already separated the lines.
                if !just_closed_list || (closed_loose && !out.ends_with("\n\n")) {
                    out.push('\n');
                }
                just_closed_list = false;
                closed_loose = false;
            }
            Event::SoftBreak => out.push(' '),
            Event::HardBreak => out.push('\n'),
            Event::Rule => {
                if !at_item_start && !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("---\n");
            }
            // The parser runs with default options, so these
            // extension events never fire; they are matched
            // explicitly so a future option change is a deliberate
            // decision here, not a silent catch-all fallthrough —
            // revisit these arms if any option is enabled.
            Event::TaskListMarker(checked) => {
                out.push_str(if checked { "[x] " } else { "[ ] " });
            }
            Event::FootnoteReference(label) => {
                out.push_str(&format!("[^{label}]"));
            }
            Event::InlineMath(text) | Event::DisplayMath(text) => out.push_str(&text),
        }
    }
    // One line per line, no trailing whitespace, no leading or
    // trailing blank lines: a single-paragraph body renders as one
    // line, multi-paragraph bodies as separated lines.
    let text = out
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    // Terminal control characters are stripped from the converted
    // text: comment bodies are free text from another user, and the
    // Markdown parser passes escape sequences through as text.
    strip_terminal_controls(&text)
}

fn render_comments_text(id: &str, comments: &[RequestComment]) -> String {
    if comments.is_empty() {
        return format!("no comments on request {id}");
    }
    let mut lines = vec![format!("id: {id}")];
    lines.extend(comments.iter().map(|comment| {
        let mut line = format!(
            "comment {} by {} at {}: {}",
            comment.id,
            comment.who,
            comment.when,
            markdown_to_plaintext(&comment.body)
        );
        if let Some(parent) = comment.parent_id {
            line.push_str(&format!(" (reply to {parent})"));
        }
        line
    }));
    lines.join("\n")
}

/// The search coordinates as one fragment: the filters actually
/// given, as `key=value` pairs joined by spaces, with a leading
/// space — or empty when no filter was given. One argument (a slice
/// of key/value pairs) so the twelve filters do not become twelve
/// parameters. `with_history` and `with_full_history` are output
/// shape, not coordinates, so they are not echoed.
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
/// count: `search project=home:foo state=new (2 requests)`, or
/// `search (0 requests)` when no filter was given.
fn search_header(coordinates: &str, count: usize) -> String {
    format!(
        "search{coordinates} ({} request{})",
        count,
        if count == 1 { "" } else { "s" }
    )
}

/// The search text: one header line echoing the coordinates, then
/// the narrow table (id, state, creator, actions) — or `no requests`
/// under the header when nothing matched. The table never renders
/// history, even with `--with-history`: that flag enriches the JSON
/// payload only.
fn render_search_text(coordinates: &str, requests: &[Request]) -> String {
    render_request_list(&search_header(coordinates, requests.len()), requests)
}

/// The list text: the same narrow table as the search text, under a
/// `list` header echoing the coordinates. The list leaf is a thin
/// wrapper over the search operation, so the table is shared, not
/// re-implemented.
fn render_list_text(coordinates: &str, requests: &[Request]) -> String {
    render_request_list(
        &format!(
            "list{coordinates} ({} request{})",
            requests.len(),
            if requests.len() == 1 { "" } else { "s" }
        ),
        requests,
    )
}

/// A header line plus the narrow request table (id, state, creator,
/// actions) — or `no requests` under the header when nothing matched.
fn render_request_list(header: &str, requests: &[Request]) -> String {
    if requests.is_empty() {
        return format!("{header}\nno requests");
    }
    let rows: Vec<Vec<String>> = requests
        .iter()
        .map(|request| {
            vec![
                request.id.to_string(),
                request.state.name.as_str().to_string(),
                request.creator.clone(),
                request
                    .actions
                    .iter()
                    .map(action_summary)
                    .collect::<Vec<_>>()
                    .join(", "),
            ]
        })
        .collect();
    format!(
        "{header}\n{}",
        render_table(&["ID", "STATE", "CREATOR", "ACTIONS"], &rows)
    )
}

/// A table with a header and space-aligned columns, never truncated:
/// the last column is unpadded, the rest are padded to the widest
/// cell, and columns are joined with two spaces.
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
    lines.join("\n")
}

// ---------- Tests ----------
// Fixture requests come from the library's public `parse_request`
// on server-shaped XML, so the converters and renderers are tested
// against the same documents the client reads.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obs::{parse_comments, parse_request};
    use clap::Parser;

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

    const COMMENTS_XML: &str = r#"<comments request="42">
  <comment who="user1" when="2010-12-29T10:00:00" id="1">first comment</comment>
  <comment who="user2" when="2010-12-29T11:00:00" id="2" parent="1">a reply</comment>
</comments>"#;

    #[derive(Debug, Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: RequestCommand,
    }

    fn fixture_request() -> Request {
        parse_request(REQUEST_XML).expect("fixture request parses")
    }

    #[test]
    fn request_json_and_text_agree() {
        let request = fixture_request();
        let payload = request_json(&request);
        assert_eq!(payload["id"], 123);
        assert_eq!(payload["creator"], "creator");
        assert_eq!(payload["state"]["name"], "review");
        assert_eq!(payload["state"]["approver"], "someone");
        assert_eq!(payload["actions"][0]["type"], "submit");
        assert_eq!(payload["actions"][0]["sourceupdate"], "cleanup");
        assert_eq!(payload["actions"][0]["updatelink"], true);
        assert_eq!(payload["actions"][1]["person"]["name"], "bar");
        assert_eq!(payload["actions"][1]["person"]["role"], "maintainer");
        assert_eq!(payload["reviews"][0]["by_group"], "group1");
        assert_eq!(payload["reviews"][0]["comment"], "review start");

        // Every id, name and revision in the JSON appears in the text.
        let text = render_request_text(&request);
        for expected in [
            "123",
            "creator",
            "review",
            "someone",
            "xyz/abc",
            "cleanup",
            "updatelink",
            "bar",
            "maintainer",
            "groupxyz",
            "reader",
            "home:foo",
            "group:group1",
            "review start",
            "2010-12-27T01:36:29",
            "2010-12-28T00:11:22",
            "Created request",
            "accepted rev 5",
        ] {
            assert!(
                text.contains(expected),
                "text missing {expected:?}:\n{text}"
            );
        }
    }

    #[test]
    fn request_text_strips_terminal_controls_from_free_text_fields() {
        // Free text from the server reaches the terminal through the
        // request render: escape sequences, carriage returns, bells,
        // and bidirectional marks are stripped, while newlines
        // survive. The description is stripped here at the render —
        // it round-trips to the server on forward, so the parse keeps
        // it verbatim.
        let mut request = fixture_request();
        request.title = Some("ti\u{1b}[2Jtle".to_string());
        request.description = "de\u{7}sc\nription".to_string();
        request.state.comment = "st\u{202e}ate".to_string();
        request.reviews[0].comment = "re\u{1b}]8;;http://x\u{7}view".to_string();
        request.history[0].description = "hi\u{200e}story".to_string();
        request.history[0].comment = "co\rmment".to_string();
        let text = render_request_text(&request);
        assert!(!text.contains('\u{1b}'), "text: {text}");
        assert!(!text.contains('\u{7}'), "text: {text}");
        assert!(!text.contains('\r'), "text: {text}");
        assert!(!text.contains('\u{202e}'), "text: {text}");
        assert!(text.contains("title: ti[2Jtle"), "text: {text}");
        assert!(text.contains("description: desc\nription"), "text: {text}");
        assert!(text.contains("state comment: state"), "text: {text}");
        assert!(text.contains(" — re]8;;http://xview"), "text: {text}");
        assert!(
            text.contains("history 1: creator 2010-12-11T00:00:00 history — comment"),
            "text: {text}"
        );
    }

    #[test]
    fn search_text_renders_a_table_and_names_the_empty_case() {
        let request = fixture_request();
        let text = render_search_text("", &[request]);
        let mut lines = text.lines();
        let header = lines.next().expect("a header line");
        assert_eq!(header, "search (1 request)");
        let columns = lines.next().expect("a column header line");
        assert!(columns.contains("ID"));
        assert!(columns.contains("STATE"));
        assert!(columns.contains("CREATOR"));
        assert!(columns.contains("ACTIONS"));
        let row = lines.next().expect("a row line");
        assert!(row.contains("123"));
        assert!(row.contains("review"));
        assert!(row.contains("creator"));
        assert!(row.contains("submit abc"));
        assert!(row.contains("add_role bar"));
        assert_eq!(
            render_search_text(" project=home:foo", &[]),
            "search project=home:foo (0 requests)\nno requests"
        );
    }

    #[test]
    fn render_table_measures_widths_in_characters() {
        // Widths are characters, not bytes: a two-byte `é` is one
        // column wide, so the second column still aligns.
        let text = render_table(
            &["ID", "STATE"],
            &[
                vec!["é".to_string(), "x".to_string()],
                vec!["yy".to_string(), "z".to_string()],
            ],
        );
        assert_eq!(text, "ID  STATE\né   x\nyy  z");
    }

    #[test]
    fn party_less_role_actions_name_the_unknown_party() {
        assert_eq!(
            action_summary(&RequestAction::AddRole {
                target_project: "home:foo".to_string(),
                target_package: None,
                person: None,
                group: None,
            }),
            "add_role (unknown party)"
        );
        assert_eq!(
            action_summary(&RequestAction::SetBugowner {
                target_project: "home:foo".to_string(),
                target_package: None,
                person_name: None,
                group_name: None,
            }),
            "set_bugowner (unknown party)"
        );
    }

    #[test]
    fn search_coordinates_echo_only_the_given_filters() {
        assert_eq!(search_coordinates(&[]), "");
        assert_eq!(
            search_coordinates(&[
                ("request-id", &["42".to_string()]),
                ("project", &["home:foo".to_string(), "home:bar".to_string()],),
                ("state", &["new".to_string()]),
            ]),
            " request-id=42 project=home:foo,home:bar state=new"
        );
        assert_eq!(search_header("", 0), "search (0 requests)");
        assert_eq!(
            search_header(" project=home:foo state=new", 2),
            "search project=home:foo state=new (2 requests)"
        );
    }

    #[test]
    fn comments_text_threads_replies_and_names_the_empty_case() {
        let comments = parse_comments(COMMENTS_XML).expect("comments parse");
        let text = render_comments_text("42", &comments);
        assert!(text.contains("id: 42"));
        assert!(text.contains("first comment"));
        assert!(text.contains("a reply"));
        assert!(text.contains("(reply to 1)"));
        assert_eq!(render_comments_text("42", &[]), "no comments on request 42");
    }

    const COMMENTS_MARKDOWN_XML: &str = r#"<comments request="42">
  <comment who="user1" when="2010-12-29T10:00:00" id="1">**bold** and [link](https://example.com)</comment>
  <comment who="user2" when="2010-12-29T11:00:00" id="2">see req#123 and @user</comment>
  <comment who="user3" when="2010-12-29T12:00:00" id="3">Please fix &lt;foo&gt; and see **note**</comment>
</comments>"#;

    #[test]
    fn comments_text_renders_markdown_as_plaintext() {
        let comments = parse_comments(COMMENTS_MARKDOWN_XML).expect("comments parse");
        let text = render_comments_text("42", &comments);
        assert!(text.contains("comment 1 by user1 at 2010-12-29T10:00:00: bold and link"));
        assert!(text.contains("comment 2 by user2 at 2010-12-29T11:00:00: see req#123 and @user"));
        assert!(text
            .contains("comment 3 by user3 at 2010-12-29T12:00:00: Please fix <foo> and see note"));
        assert!(!text.contains("**"));
        assert!(!text.contains("https://example.com"));
    }

    #[test]
    fn comments_text_strips_terminal_controls_from_bodies() {
        // Comment bodies are free text from another user, and the
        // Markdown parser passes escape sequences through as text:
        // the converted plaintext is stripped at the render, while
        // the JSON payload keeps the raw Markdown.
        let xml = "<comments request=\"42\">\n  <comment who=\"user1\" when=\"2010-12-29T10:00:00\" id=\"1\">fix\u{1b}[2Jit\u{7}now\nline\u{202e}two</comment>\n</comments>";
        let comments = parse_comments(xml).expect("comments parse");
        let text = render_comments_text("42", &comments);
        assert!(!text.contains('\u{1b}'), "text: {text}");
        assert!(!text.contains('\u{7}'), "text: {text}");
        assert!(!text.contains('\u{202e}'), "text: {text}");
        assert!(
            text.contains("comment 1 by user1 at 2010-12-29T10:00:00: fix[2Jitnow linetwo"),
            "text: {text}"
        );
    }

    #[test]
    fn comment_json_keeps_the_raw_markdown() {
        let comments = parse_comments(COMMENTS_MARKDOWN_XML).expect("comments parse");
        let payload = comment_json(&comments[0]);
        assert_eq!(payload["body"], "**bold** and [link](https://example.com)");
    }

    #[test]
    fn markdown_to_plaintext_keeps_plain_text_and_lists() {
        assert_eq!(markdown_to_plaintext("plain text"), "plain text");
        assert_eq!(
            markdown_to_plaintext("# Title\n\n- one\n- two"),
            "Title\n- one\n- two"
        );
        assert_eq!(
            markdown_to_plaintext("1. first\n2. second"),
            "1. first\n2. second"
        );
        assert_eq!(
            markdown_to_plaintext("- a\n  - b\n  - c\n- d"),
            "- a\n- b\n- c\n- d"
        );
        assert_eq!(
            markdown_to_plaintext("1. a\n   1. b\n   2. c\n2. d"),
            "1. a\n1. b\n2. c\n2. d"
        );
        assert_eq!(
            markdown_to_plaintext("- a\n\n  - b\n\n- d"),
            "- a\n\n- b\n\n- d"
        );
        assert_eq!(
            markdown_to_plaintext("1. a\n\n   1. b\n   2. c\n\n2. d"),
            "1. a\n\n1. b\n2. c\n\n2. d"
        );
        assert_eq!(
            markdown_to_plaintext("- a\n\n  - b\n\n    - c\n\n  - d\n\n- e"),
            "- a\n\n- b\n\n- c\n\n- d\n\n- e"
        );
        assert_eq!(
            markdown_to_plaintext("- a\n  - b\n    - c\n  - d\n- e"),
            "- a\n- b\n- c\n- d\n- e"
        );
        assert_eq!(
            markdown_to_plaintext("`code` and *italic*"),
            "code and italic"
        );
        assert_eq!(markdown_to_plaintext(""), "");
    }

    #[test]
    fn markdown_to_plaintext_keeps_block_tags_on_their_own_line() {
        assert_eq!(
            markdown_to_plaintext("- a\n  ```\n  code\n  ```\n- b"),
            "- a\ncode\n\n- b"
        );
        assert_eq!(markdown_to_plaintext("```\ncode\n```\npara"), "code\npara");
        assert_eq!(
            markdown_to_plaintext("- a\n  # head\n- b"),
            "- a\nhead\n\n- b"
        );
        assert_eq!(markdown_to_plaintext("- a\n  ***\n- b"), "- a\n---\n\n- b");
        assert_eq!(
            markdown_to_plaintext("- a\n  > quote\n- b"),
            "- a\nquote\n\n- b"
        );
        assert_eq!(
            markdown_to_plaintext("- a\n  <div>\n  foo\n  </div>\n- b"),
            "- a\n<div>\nfoo\n</div>\n\n- b"
        );
        // A block as an item's first content shares the marker line.
        assert_eq!(
            markdown_to_plaintext("- # head\n  - b\n- d"),
            "- head\n\n- b\n\n- d"
        );
        // An empty heading adds no line of its own: the guarded
        // Start already ended the line.
        assert_eq!(markdown_to_plaintext("- a\n  #\n- b"), "- a\n\n- b");
        assert_eq!(
            markdown_to_plaintext("- a\n  #\n  - b\n- d"),
            "- a\n\n- b\n\n- d"
        );
        assert_eq!(
            markdown_to_plaintext("- a\n  # ![](x.png)\n- b"),
            "- a\n\n- b"
        );
        assert_eq!(markdown_to_plaintext("- a\n\n  #\n\n- b"), "- a\n\n- b");
    }

    #[test]
    fn markdown_to_plaintext_pins_spacing_and_literal_passthrough() {
        // Multi-paragraph bodies render as separated lines with no
        // blank line between them; a loose list keeps its blank line.
        assert_eq!(markdown_to_plaintext("para1\n\npara2"), "para1\npara2");
        assert_eq!(markdown_to_plaintext("- one\n\n- two"), "- one\n\n- two");
        // A fenced block renders its code; the info string is dropped.
        assert_eq!(
            markdown_to_plaintext("```rust\nfn main() {}\n```"),
            "fn main() {}"
        );
        // Default parser options: strikethrough, tables and math are
        // not extensions here, so they pass through literally.
        assert_eq!(markdown_to_plaintext("~~struck~~"), "~~struck~~");
        assert_eq!(
            markdown_to_plaintext("| a | b |\n|---|---|\n| c | d |"),
            "| a | b | |---|---| | c | d |"
        );
        assert_eq!(markdown_to_plaintext("$x^2$"), "$x^2$");
        // An autolink keeps its address as the link text; an image
        // keeps its alt text (an empty alt leaves the two spaces
        // around it, which is the current behaviour).
        assert_eq!(
            markdown_to_plaintext("see <https://example.com>"),
            "see https://example.com"
        );
        assert_eq!(
            markdown_to_plaintext("see ![alt](https://example.com/x.png) now"),
            "see alt now"
        );
        assert_eq!(
            markdown_to_plaintext("see ![](https://example.com/x.png) now"),
            "see  now"
        );
        // A blockquote renders as its text; raw HTML passes through
        // unchanged, like `<foo>`.
        assert_eq!(markdown_to_plaintext("> quote\n> more"), "quote more");
        assert_eq!(markdown_to_plaintext("<!-- secret -->"), "<!-- secret -->");
        // A footnote definition is consumed by the parser without
        // the footnotes extension; the reference keeps its number.
        assert_eq!(markdown_to_plaintext("text[^1]\n\n[^1]: note"), "text^1");
        // Known flattening loss, pre-existing: the blank line
        // between a tight nested list and a following paragraph in
        // the same item is not preserved.
        assert_eq!(
            markdown_to_plaintext("- a\n  - b\n\n  after\n- d"),
            "- a\n\n- b\nafter\n\n- d"
        );
    }

    #[test]
    fn history_text_names_the_empty_case() {
        let request = fixture_request();
        let text = render_history_text(&request.id.to_string(), &request.history);
        assert!(text.contains("id: 123"));
        assert!(text.contains("Created request"));
        assert_eq!(render_history_text("42", &[]), "no history for request 42");
    }

    #[test]
    fn diff_text_is_verbatim_but_names_the_empty_case() {
        assert_eq!(
            render_diff_text("42", "--- a/foo\n+++ b/foo\n"),
            "--- a/foo\n+++ b/foo\n"
        );
        assert_eq!(render_diff_text("42", ""), "no diff for request 42");
    }

    #[test]
    fn clap_parses_the_search_flags() {
        let cli = TestCli::try_parse_from([
            "geectl",
            "search",
            "--state",
            "new",
            "--state",
            "review",
            "--creator",
            "alice",
            "--with-history",
        ])
        .expect("search parses");
        match cli.command {
            RequestCommand::Search {
                state,
                creator,
                with_history,
                ..
            } => {
                assert_eq!(state, vec!["new", "review"]);
                assert_eq!(creator, vec!["alice"]);
                assert!(with_history);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn clap_parses_the_filing_flags() {
        let cli = TestCli::try_parse_from([
            "geectl",
            "file",
            "--source-project",
            "home:scarabeus_iv:branches:devel:languages:python",
            "--source-package",
            "foo",
            "--target-project",
            "devel:languages:python",
            "--target-package",
            "foo",
            "--description",
            "Submit foo",
            "--changes-entry",
            "- Fix CVE-2026-12345.",
            "--cve",
            "CVE-2026-12345",
            "--cve",
            "CVE-2026-12346",
        ])
        .expect("file parses");
        match cli.command {
            RequestCommand::File { file } => {
                assert_eq!(file.cve, vec!["CVE-2026-12345", "CVE-2026-12346"]);
                assert!(!file.without_cleanup);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn clap_parses_the_maintenance_incident_flags() {
        let cli = TestCli::try_parse_from([
            "geectl",
            "file-maintenance-incident",
            "--source-project",
            "home:scarabeus_iv:branches:openSUSE:Maintenance",
            "--source-package",
            "foo",
            "--source-package",
            "bar",
            "--target-project",
            "openSUSE:Maintenance",
            "--release-project",
            "openSUSE:Leap:15.6:Update",
            "--description",
            "Fix foo and bar",
            "--with-cleanup",
        ])
        .expect("file-maintenance-incident parses");
        match cli.command {
            RequestCommand::FileMaintenanceIncident { incident } => {
                assert_eq!(incident.source_package, vec!["foo", "bar"]);
                assert_eq!(
                    incident.release_project.as_deref(),
                    Some("openSUSE:Leap:15.6:Update")
                );
                assert!(incident.with_cleanup);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn clap_refuses_the_old_per_command_live_flag() {
        // The leaf defines no `--live` (it is a global on the
        // top-level `Cli`).
        let err = TestCli::try_parse_from(["geectl", "accept", "42", "--live"])
            .expect_err("per-command --live must not parse");
        assert!(err.to_string().contains("unexpected argument"));
    }

    #[test]
    fn clap_accepts_a_changes_entry_starting_with_a_bullet() {
        let cli = TestCli::try_parse_from([
            "geectl",
            "file",
            "--source-project",
            "a",
            "--source-package",
            "b",
            "--target-project",
            "c",
            "--target-package",
            "d",
            "--description",
            "Submit b",
            "--changes-entry",
            "- Fix CVE-2026-12345.",
            "--cve",
            "CVE-2026-12345",
        ])
        .expect("a bullet changes entry parses");
        match cli.command {
            RequestCommand::File { file } => {
                assert_eq!(file.changes_entry, "- Fix CVE-2026-12345.");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn clap_refuses_conflicting_delete_targets() {
        let err = TestCli::try_parse_from([
            "geectl",
            "file-delete",
            "--target-project",
            "foo",
            "--package",
            "bar",
            "--repository",
            "baz",
        ])
        .expect_err("package and repository conflict");
        assert!(err.to_string().contains("cannot be used with"));
    }

    #[test]
    fn clap_parses_the_add_role_flags() {
        let cli = TestCli::try_parse_from([
            "geectl",
            "file-add-role",
            "--target-project",
            "foo",
            "--target-package",
            "bar",
            "--person",
            "alice",
            "--person-role",
            "maintainer",
            "--group",
            "factory-maintainers",
            "--group-role",
            "maintainer",
            "--description",
            "Grant roles on bar",
        ])
        .expect("file-add-role parses");
        match cli.command {
            RequestCommand::FileAddRole {
                target_project,
                target_package,
                person,
                person_role,
                group,
                group_role,
                description,
            } => {
                assert_eq!(target_project, "foo");
                assert_eq!(target_package.as_deref(), Some("bar"));
                assert_eq!(person.as_deref(), Some("alice"));
                assert_eq!(person_role.as_deref(), Some("maintainer"));
                assert_eq!(group.as_deref(), Some("factory-maintainers"));
                assert_eq!(group_role.as_deref(), Some("maintainer"));
                assert_eq!(description, "Grant roles on bar");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn clap_refuses_a_person_without_a_role() {
        let err = TestCli::try_parse_from([
            "geectl",
            "file-add-role",
            "--target-project",
            "foo",
            "--person",
            "alice",
            "--description",
            "Grant a role",
        ])
        .expect_err("a person without a role must not parse");
        assert!(err.to_string().contains("required"));
    }

    #[test]
    fn clap_refuses_a_role_without_a_group() {
        let err = TestCli::try_parse_from([
            "geectl",
            "file-add-role",
            "--target-project",
            "foo",
            "--group-role",
            "maintainer",
            "--description",
            "Grant a role",
        ])
        .expect_err("a role without a group must not parse");
        assert!(err.to_string().contains("required"));
    }

    #[test]
    fn clap_refuses_a_final_state_for_change_state() {
        let err = TestCli::try_parse_from(["geectl", "change-state", "42", "superseded"])
            .expect_err("superseded is not a changeable state");
        assert!(err.to_string().contains("invalid value"));
    }

    #[test]
    fn clap_refuses_final_states_for_change_review_state() {
        for state in ["superseded", "obsoleted"] {
            let err = TestCli::try_parse_from([
                "geectl",
                "change-review-state",
                "42",
                state,
                "--user",
                "Admin",
            ])
            .expect_err("a final state is not a review state change");
            assert!(err.to_string().contains("invalid value"));
        }
    }

    #[test]
    fn state_parsing_accepts_valid_states_and_rejects_garbage() {
        for value in [
            "new",
            "review",
            "accepted",
            "declined",
            "revoked",
            "superseded",
            "deleted",
        ] {
            assert!(parse_state(value).is_ok(), "{value} parses");
        }
        assert!(parse_state("bogus").is_err());
        assert!(parse_state("").is_err());
    }

    #[test]
    fn build_search_keeps_the_default_states_and_honours_named_ones() {
        let search = build_search(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            false,
            false,
            None,
            None,
        )
        .expect("an empty search builds");
        let expr = search.match_expr().expect("a match builds");
        for state in ["new", "review", "declined"] {
            assert!(expr.contains(state), "default match names {state}: {expr}");
        }
        let search = build_search(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec!["accepted".to_string()],
            Vec::new(),
            Vec::new(),
            false,
            false,
            None,
            None,
        )
        .expect("a state search builds");
        let expr = search.match_expr().expect("a match builds");
        assert!(
            expr.contains("accepted"),
            "named match names accepted: {expr}"
        );
        assert!(
            !expr.contains("declined"),
            "named match drops the default: {expr}"
        );
    }

    #[test]
    fn build_search_passes_reviewer_through() {
        let search = build_search(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec!["alice".to_string()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            false,
            false,
            None,
            None,
        )
        .expect("a reviewer search builds");
        assert_eq!(
            search.match_expr().expect("a match builds"),
            "review[@by_user='alice' and @state='new'] and state[@name='new' or @name='review']"
        );
    }

    #[test]
    fn clap_accepts_a_comment_starting_with_a_bullet() {
        let cli = TestCli::try_parse_from(["geectl", "accept", "42", "--comment", "- looks good"])
            .expect("a bullet comment parses");
        match cli.command {
            RequestCommand::Accept { comment, .. } => {
                assert_eq!(comment, "- looks good");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn review_text_keeps_a_lone_who_or_when_and_names_no_reviewer() {
        use crate::obs::ReviewState;
        let who_only = RequestReview {
            state: ReviewState::New,
            when: None,
            who: Some("alice".to_string()),
            by_user: None,
            by_group: None,
            by_project: None,
            by_package: None,
            comment: String::new(),
            history: Vec::new(),
        };
        let text = review_text(1, &who_only);
        assert!(text.contains("state new"), "{text}");
        assert!(text.contains("by alice"), "{text}");
        assert!(!text.contains("by  state"), "{text}");
        let when_only = RequestReview {
            state: ReviewState::New,
            when: Some("2010-12-28T00:11:22".to_string()),
            who: None,
            by_user: None,
            by_group: None,
            by_project: None,
            by_package: None,
            comment: String::new(),
            history: Vec::new(),
        };
        let text = review_text(2, &when_only);
        assert!(text.contains("at 2010-12-28T00:11:22"), "{text}");
        assert!(!text.contains("by  state"), "{text}");
    }

    #[test]
    fn show_text_renders_the_accepted_revision_but_not_the_hashes() {
        let request = fixture_request();
        let text = render_request_text(&request);
        assert!(text.contains("accepted rev 5"));
        // The acceptinfo hashes stay in the JSON payload
        // (see the module docs); only the revision renders.
        assert!(!text.contains("deadbeef"));
    }

    #[test]
    fn add_plan_inserts_recorded_lines_in_dry_run_only() {
        let dry_run = DryRun::new(true);
        dry_run.allow("file delete request for test:delete-me");
        let mut payload = json!({ "dry_run": true });
        add_plan(&mut payload, &dry_run);
        let plan = payload["plan"].as_array().expect("plan is present");
        assert_eq!(plan.len(), 1);
        assert!(plan[0].as_str().unwrap().contains("file delete request"));

        // A live run's plan is empty by construction, so the field is
        // omitted there.
        let live = DryRun::new(false);
        let mut payload = json!({ "dry_run": false });
        add_plan(&mut payload, &live);
        assert!(payload.get("plan").is_none());
    }

    #[test]
    fn filed_payload_carries_dry_run_and_plan() {
        let dry_run = DryRun::new(true);
        dry_run.allow("change request 123 state to accepted");
        let payload = filed_payload(
            &None,
            &dry_run,
            |id| json!({ "id": id.map(request_id_json).unwrap_or(Value::Null) }),
        );
        assert_eq!(payload["dry_run"], true);
        assert_eq!(payload["id"], Value::Null);
        let plan = payload["plan"].as_array().expect("plan is present");
        assert_eq!(plan.len(), 1);
    }

    fn sample_file_args() -> FileArgs {
        FileArgs {
            source_project: "home:alice:branches:test".to_string(),
            source_package: "foo".to_string(),
            target_project: "devel:languages:python".to_string(),
            target_package: "foo".to_string(),
            description: "Fix a bug".to_string(),
            changes_entry: "- Fix a bug (CVE-2026-1234)".to_string(),
            cve: vec!["CVE-2026-1234".to_string()],
            without_cleanup: false,
        }
    }

    #[test]
    fn file_request_text_carries_cves_in_all_four_arms() {
        let file = sample_file_args();
        let cves = parse_cves(&file);
        assert_eq!(
            file_request_text(Some("123"), None, None, &file, &cves),
            "filed request 123 for home:alice:branches:test/foo to devel:languages:python/foo (CVE-2026-1234)"
        );
        assert_eq!(
            file_request_text(
                Some("123"),
                Some("122"),
                Some(&["122".to_string()]),
                &file,
                &cves
            ),
            "filed request 123 for home:alice:branches:test/foo to devel:languages:python/foo (CVE-2026-1234), superseded 122"
        );
        // A filing that supersedes several requests names them all,
        // in supersede order — including a plain file, not only a
        // refile.
        assert_eq!(
            file_request_text(
                Some("123"),
                None,
                Some(&["122".to_string(), "121".to_string()]),
                &file,
                &cves
            ),
            "filed request 123 for home:alice:branches:test/foo to devel:languages:python/foo (CVE-2026-1234), superseded 122, 121"
        );
        assert_eq!(
            file_request_text(None, None, None, &file, &cves),
            "dry-run: would file request for home:alice:branches:test/foo to devel:languages:python/foo (CVE-2026-1234)"
        );
        assert_eq!(
            file_request_text(None, Some("122"), None, &file, &cves),
            "dry-run: would file request for home:alice:branches:test/foo to devel:languages:python/foo (CVE-2026-1234) and supersede 122"
        );
        // The CVE ids belong to the new request: the parenthetical
        // rides the coordinates, before the superseded id — it must
        // never read as qualifying the superseded id.
        let refiled = file_request_text(
            Some("123"),
            Some("122"),
            Some(&["122".to_string()]),
            &file,
            &cves,
        );
        assert!(
            refiled.find("(CVE-2026-1234)").expect("CVE suffix present")
                < refiled
                    .find("superseded 122")
                    .expect("superseded id present"),
            "{refiled}"
        );
        // Lowercase input normalizes to uppercase in the text,
        // matching the filed request.
        let lower = FileArgs {
            cve: vec!["cve-2026-1234".to_string()],
            ..sample_file_args()
        };
        let lower_cves = parse_cves(&lower);
        assert!(
            file_request_text(Some("123"), None, None, &lower, &lower_cves)
                .contains("(CVE-2026-1234)"),
            "lowercase CVE must render normalized"
        );
        // No CVE claimed: no suffix — the description and the
        // `.changes` entry stay JSON-only (long free text).
        let plain = FileArgs {
            cve: vec![],
            ..sample_file_args()
        };
        assert_eq!(
            file_request_text(Some("123"), None, None, &plain, &[]),
            "filed request 123 for home:alice:branches:test/foo to devel:languages:python/foo"
        );
        let two = FileArgs {
            cve: vec!["CVE-2026-1234".to_string(), "CVE-2026-5678".to_string()],
            ..sample_file_args()
        };
        let two_cves = parse_cves(&two);
        assert!(
            file_request_text(Some("123"), None, None, &two, &two_cves)
                .ends_with("(CVE-2026-1234, CVE-2026-5678)"),
            "{}",
            file_request_text(Some("123"), None, None, &two, &two_cves)
        );
    }

    #[test]
    fn file_request_json_carries_normalized_cves() {
        let file = sample_file_args();
        let cves = parse_cves(&file);
        let filed = FiledRequest {
            id: "123".to_string(),
            superseded: vec!["122".to_string()],
        };
        let value = file_request_json(Some("123"), Some(&filed), &file, &cves);
        assert_eq!(value["id"], json!(123));
        assert_eq!(value["superseded"], json!([122]));
        // No superseded requests: an empty array, not null — the
        // open requests were read and there were none.
        let empty = FiledRequest {
            id: "123".to_string(),
            superseded: vec![],
        };
        assert_eq!(
            file_request_json(Some("123"), Some(&empty), &file, &cves)["superseded"],
            json!([])
        );
        // A dry-run id of None reads as null, and so do the
        // superseded ids — the open requests are unknown there.
        assert_eq!(
            file_request_json(None, None, &file, &cves)["superseded"],
            serde_json::Value::Null
        );
        assert_eq!(value["source_project"], "home:alice:branches:test");
        assert_eq!(value["target_project"], "devel:languages:python");
        assert_eq!(value["cve"], json!(["CVE-2026-1234"]));
        assert_eq!(value["without_cleanup"], false);
        // Lowercase input normalizes to uppercase in the JSON,
        // matching the text.
        let lower = FileArgs {
            cve: vec!["cve-2026-1234".to_string()],
            ..sample_file_args()
        };
        let lower_cves = parse_cves(&lower);
        assert_eq!(
            file_request_json(Some("123"), Some(&filed), &lower, &lower_cves)["cve"],
            json!(["CVE-2026-1234"])
        );
        // No CVE claimed: an empty list.
        let plain = FileArgs {
            cve: vec![],
            ..sample_file_args()
        };
        assert_eq!(
            file_request_json(Some("123"), Some(&filed), &plain, &[])["cve"],
            json!([])
        );
        // A dry-run id of None reads as null.
        assert_eq!(
            file_request_json(None, None, &file, &cves)["id"],
            serde_json::Value::Null
        );
    }

    fn parse_cves(file: &FileArgs) -> Vec<CveId> {
        file.cve
            .iter()
            .map(|cve| CveId::parse(cve).expect("sample CVE parses"))
            .collect()
    }

    #[test]
    fn filing_texts_carry_every_coordinate() {
        assert_eq!(
            file_change_devel_text(
                Some("123"),
                "home:alice:branches:test",
                "foo",
                "devel:languages:python",
                "foo"
            ),
            "filed change-devel request 123 for home:alice:branches:test/foo to devel:languages:python/foo"
        );
        assert_eq!(
            file_change_devel_text(
                None,
                "home:alice:branches:test",
                "foo",
                "devel:languages:python",
                "foo"
            ),
            "dry-run: would file change-devel request for home:alice:branches:test/foo to devel:languages:python/foo"
        );
        assert_eq!(
            file_delete_text(Some("123"), "openSUSE:Factory/foo", None),
            "filed delete request 123 for openSUSE:Factory/foo"
        );
        assert_eq!(
            file_delete_text(
                Some("123"),
                "openSUSE:Factory (repository standard)",
                Some(48)
            ),
            "filed delete request 123 for openSUSE:Factory (repository standard) (auto-accept in 48h)"
        );
        assert_eq!(
            file_delete_text(None, "openSUSE:Factory", Some(0)),
            "dry-run: would file delete request for openSUSE:Factory (auto-accept in 0h)"
        );
        let incident = MaintenanceIncidentArgs {
            source_project: "home:alice:branches:test".to_string(),
            source_package: vec!["foo".to_string(), "bar".to_string()],
            target_project: "openSUSE:Maintenance:1234".to_string(),
            release_project: Some("openSUSE:Leap:15.6:Update".to_string()),
            description: "Fix things".to_string(),
            with_cleanup: false,
        };
        assert_eq!(
            file_maintenance_incident_text(Some("123"), None, &incident),
            "filed maintenance incident request 123 for foo, bar from home:alice:branches:test to openSUSE:Maintenance:1234, releasing into openSUSE:Leap:15.6:Update"
        );
        assert_eq!(
            file_maintenance_incident_text(None, Some("122"), &incident),
            "dry-run: would file maintenance incident request for foo, bar from home:alice:branches:test to openSUSE:Maintenance:1234, releasing into openSUSE:Leap:15.6:Update and supersede 122"
        );
    }

    /// The design doc's worked example for `request show`, pinned
    /// literally: id 1398977, state new, the submit action with rev
    /// 42, the factory-staging review.
    const DESIGN_SHOW_XML: &str = r#"<request creator="alice" id="1398977">
  <action type="submit">
    <source package="foo" project="openSUSE:Factory" rev="42" />
    <target project="openSUSE:Factory" />
  </action>
  <state name="new" who="alice" when="2026-11-21T10:00:00" created="2026-11-21T09:00:00" />
  <review by_group="factory-staging" state="accepted" />
</request>"#;

    #[test]
    fn show_text_matches_the_design_example() {
        let request = parse_request(DESIGN_SHOW_XML).expect("design example parses");
        let text = render_request_text(&request);
        for expected in [
            "id: 1398977",
            "state: new",
            "state by: alice",
            "state at: 2026-11-21T10:00:00",
            "created: 2026-11-21T09:00:00",
            "creator: alice",
            "action 1: submit openSUSE:Factory/foo rev 42 -> openSUSE:Factory",
            "review 1: by group:factory-staging state accepted",
        ] {
            assert!(
                text.contains(expected),
                "text missing {expected:?}:\n{text}"
            );
        }
    }

    #[test]
    fn build_search_passes_since_and_limit_through() {
        let search = build_search(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            false,
            false,
            Some("2026-10-01".to_string()),
            Some(3),
        )
        .expect("a valid since and limit build");
        assert_eq!(search.limit_value(), Some(3));
        assert!(
            search
                .match_expr()
                .expect("the built search builds")
                .contains(
                "(history[@when>='2026-10-01T00:00:00'] or state[@when>='2026-10-01T00:00:00'])"
            ),
            "since clause present"
        );
    }

    #[test]
    fn build_search_refuses_a_zero_limit() {
        let err = build_search(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            false,
            false,
            None,
            Some(0),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn search_coordinates_echo_since_and_limit() {
        assert_eq!(
            search_coordinates(&[
                ("since", &["2026-10-01".to_string()]),
                ("limit", &["3".to_string()]),
            ]),
            " since=2026-10-01 limit=3"
        );
    }

    #[test]
    fn list_default_states_defaults_to_new_and_review() {
        assert_eq!(
            list_default_states(vec![]),
            vec!["new".to_string(), "review".to_string()]
        );
        assert_eq!(
            list_default_states(vec!["declined".to_string()]),
            vec!["declined".to_string()]
        );
    }

    #[test]
    fn validate_list_scope_refuses_a_package_without_a_project() {
        let err = validate_list_scope(&[], &["foo".to_string()]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(validate_list_scope(&["home:foo".to_string()], &["foo".to_string()]).is_ok());
        assert!(validate_list_scope(&[], &[]).is_ok());
    }

    #[test]
    fn list_text_matches_the_search_table() {
        let request = parse_request(REQUEST_XML).expect("fixture parses");
        let text = render_list_text("", &[request]);
        let mut lines = text.lines();
        assert_eq!(lines.next().unwrap(), "list (1 request)");
        assert_eq!(lines.next().unwrap(), "ID   STATE   CREATOR  ACTIONS");
        assert_eq!(
            lines.next().unwrap(),
            "123  review  creator  submit abc, add_role bar"
        );
        assert!(lines.next().is_none());
        assert_eq!(
            render_list_text(" project=foo", &[]),
            "list project=foo (0 requests)\nno requests"
        );
    }

    #[test]
    fn search_text_matches_the_design_example() {
        let request = parse_request(DESIGN_SHOW_XML).expect("design example parses");
        let text = render_search_text("", &[request]);
        let mut lines = text.lines();
        assert_eq!(lines.next().unwrap(), "search (1 request)");
        assert_eq!(lines.next().unwrap(), "ID       STATE  CREATOR  ACTIONS");
        assert_eq!(lines.next().unwrap(), "1398977  new    alice    submit foo");
        assert!(lines.next().is_none());
    }
}
