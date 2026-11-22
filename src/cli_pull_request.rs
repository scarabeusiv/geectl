//! `pull-request` — the unified pull-request command group: read OBS
//! submit requests and Gitea pull requests as one surface.
//!
//! Thin over `PullRequestClient` like the OBS groups are over
//! `ObsClient`: the flags mirror the library parameters, the JSON
//! payload carries the typed values, and the text renderer prints
//! the same values in a human-readable form.
//!
//! Reads are always live: there are no mutations in this group, so
//! no `--live` applies. A read on an arm needs that arm's
//! credentials — OBS Basic credentials for `obs:` handles, the
//! Gitea Bearer token for `gitea:` handles — and fails closed
//! naming them when they are missing. `search` with no `--backend`
//! runs both arms, OBS first then Gitea, so it needs both sets of
//! credentials; scope with `--backend` when only one is configured. `--repo` needs
//! `--backend gitea`.
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
use geectl::auth::{AuthProvider, EnvGiteaAuth, EnvObsAuth};
use geectl::config::{self, DEFAULT_GITEA_URL};
use geectl::debug::Debug;
use geectl::error::{ErrorKind, GeectlError};
use geectl::gitea::{parse_gitea_repo, GiteaClient};
use geectl::http::{HttpClient, Timeout, UreqClient};
use geectl::obs::{ObsClient, OBS_API_BASE};
use geectl::pull_request::{
    PullRequest, PullRequestBackend, PullRequestClient, PullRequestEndpoint, PullRequestRef,
    PullRequestSearch, PullRequestStatus, PullRequestSummary,
};
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
                PullRequestRef::Obs(_) => with_obs(debug, |obs| {
                    show_with(&PullRequestClient::new(Some(obs), None), &reference)
                }),
                PullRequestRef::Gitea { .. } => with_gitea(debug, |gitea| {
                    show_with(&PullRequestClient::new(None, Some(gitea)), &reference)
                }),
            }
        }
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
                Some(PullRequestBackend::Obs) => with_obs(debug, |obs| {
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
                None => with_obs(debug, |obs| {
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
/// `auth::resolve_obs_basic_for_url`), wiring the debug sink
/// through. The client borrows the transport, so both live in this
/// scope and the caller runs inside it. Reads are always live, so
/// a credential resolution error fails fast with the precise error
/// (unlike the request group's dry-run leniency, which exists for
/// dry-run mutations only).
fn with_obs<T>(
    debug: &Debug,
    f: impl FnOnce(&ObsClient) -> Result<T, GeectlError>,
) -> Result<T, GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let effective = config::effective_config(&file_config)?;
    let base = effective.config.obs_url.as_deref().unwrap_or(OBS_API_BASE);
    let credentials = EnvObsAuth::credentials_for_url(base, base, &file_config.instances)?;
    let client =
        ObsClient::with_base_and_credentials(&http, base, credentials)?.with_debug(debug.clone());
    f(&client)
}

/// A `GiteaClient` for the configured Gitea instance (config file /
/// env / default) with the environment Bearer token, reporting on
/// the debug channel. Construction fails closed without a token —
/// even for reads (see the `gitea` group's module docs). The same
/// construction as the `gitea` group; each group owns its client
/// construction.
fn gitea_client<'a>(
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
    let credentials = EnvGiteaAuth.credentials()?;
    GiteaClient::with_base(http, base, credentials).map(|client| client.with_debug(debug.clone()))
}

/// Run `f` with a `GiteaClient` for the configured instance. The
/// client borrows the transport, so both live in this scope and
/// the caller runs inside it.
fn with_gitea<T>(
    debug: &Debug,
    f: impl FnOnce(&GiteaClient) -> Result<T, GeectlError>,
) -> Result<T, GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = gitea_client(&http, debug)?;
    f(&client)
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
}
