//! Person search (`geectl person search`).
//!
//! The unified person search: one operation over the OBS
//! person-match and the Gitea user search. A read, always live:
//! `--live` is ignored, like the other reads. Thin over
//! `PersonClient` like the OBS groups are over `ObsClient`: the
//! flags mirror the library parameters, the JSON payload carries
//! the typed values, and the text renderer prints the same values
//! in a human-readable form.
//!
//! Credentials resolve per command, like the pull-request group:
//! the OBS arm resolves its credentials for the configured URL
//! from its instance alias, and the Gitea arm resolves its token
//! for the configured URL the same way
//! (`EnvGiteaAuth::credentials_for_url`, the same per-URL shape
//! as the OBS arm) — a single-instance plain-token setup
//! behaves exactly as before. Both clients build before the
//! search runs, so a both-arms search fails closed on the first
//! missing credential — the same construction order as
//! `pull-request search` — while a scoped search never touches
//! the other arm.
//!
//! Coordinate header (design rule 1):
//! the text opens with one header line echoing the query
//! coordinates (`query: …`, plus `backend: …` when scoped).
//! Tables stay as designed; the header is the only addition.
//!
//! One result list, not two: the JSON payload carries a single
//! `results` array, each entry naming its `source` (`obs` or
//! `gitea`) and its `kind` (`project` or `user`), and the text
//! renders one table with SOURCE/KIND/NAME/DETAIL columns. A
//! scoped search returns only its arm's entries; with no
//! `--backend` both arms run, OBS first then Gitea.
//!
//! Documented omissions (not gaps): the unified table carries no
//! avatar URL (a URL, not an identifier) and no project
//! description (long free text — titles stay). The JSON payload
//! remains the complete record in every case.

use crate::auth::{EnvGiteaAuth, EnvObsAuth};
use crate::config::{self, DEFAULT_GITEA_URL};
use crate::debug::{strip_terminal_controls_marked, Debug};
use crate::error::{ErrorKind, GeectlError};
use crate::gitea::{GiteaClient, GiteaUser};
use crate::http::{HttpClient, Timeout, UreqClient};
use crate::obs::{ObsClient, ProjectMatch};
use crate::person::{PersonBackend, PersonClient, PersonSearch, PersonSearchResult};
use clap::{Args, Subcommand};
use serde_json::{json, Value};

/// The `person` command group: the unified person search over
/// both backends.
#[derive(Debug, Subcommand)]
pub enum PersonCommand {
    /// Search for a person: the OBS projects naming the login,
    /// and the Gitea users matching the keyword — one list of
    /// results, each naming its source. With no `--backend` both
    /// arms run, OBS first then Gitea.
    Search(SearchArgs),
}

/// Arguments for `person search`.
#[derive(Debug, Args)]
pub struct SearchArgs {
    /// The query: an OBS login, a Gitea keyword.
    query: String,
    /// Only search this backend (`obs` or `gitea`). With no
    /// backend both arms run, OBS first then Gitea.
    #[arg(long, value_parser = ["obs", "gitea"])]
    backend: Option<String>,
}

/// Run a `person` command, returning the JSON payload and its text
/// rendering.
pub fn run_person(
    command: PersonCommand,
    debug: &Debug,
    live: bool,
    instance: Option<&str>,
) -> Result<(Value, String), GeectlError> {
    // A read, always live: `--live` is ignored.
    let _ = live;
    match command {
        PersonCommand::Search(args) => {
            // The backend is parsed and the query validated
            // before any credential resolution, like the
            // pull-request group's search.
            let backend = match &args.backend {
                Some(backend) => Some(backend.parse::<PersonBackend>()?),
                None => None,
            };
            let query = PersonSearch {
                query: args.query.clone(),
                backend,
            };
            query.validate()?;
            match query.backend {
                Some(PersonBackend::Obs) => with_obs(debug, instance, |obs| {
                    search_with(&PersonClient::new(Some(obs), None), &query)
                }),
                Some(PersonBackend::Gitea) => with_gitea(debug, instance, |gitea| {
                    search_with(&PersonClient::new(None, Some(gitea)), &query)
                }),
                None => with_obs(debug, instance, |obs| {
                    with_gitea(debug, instance, |gitea| {
                        search_with(&PersonClient::new(Some(obs), Some(gitea)), &query)
                    })
                }),
            }
        }
    }
}

/// Search for a person through the uniform client: the JSON
/// payload carries the query, the backend (`null` when both arms
/// run), and one `results` list — the OBS projects then the
/// Gitea users, each entry naming its source and kind (an arm
/// that did not run contributes nothing); the text is the header
/// plus the one unified table.
fn search_with(
    client: &PersonClient,
    query: &PersonSearch,
) -> Result<(Value, String), GeectlError> {
    let result = client.search(query)?;
    let mut results: Vec<Value> = result.projects.iter().map(project_match_json).collect();
    results.extend(result.users.iter().map(gitea_user_json));
    let payload = json!({
        "query": query.query,
        "backend": query.backend.map(|backend| backend.as_str()),
        "results": results,
    });
    Ok((payload, render_search_text(query, &result)?))
}

/// Build the OBS client from the effective config and the
/// environment credentials for the configured base URL (resolved
/// for the URL from its instance alias — see
/// `auth::resolve_obs_for_url`), wiring the debug sink through.
/// The client borrows the transport, so both live in this scope
/// and the caller runs inside it. Reads fail fast on a credential
/// resolution error.
fn with_obs<T>(
    debug: &Debug,
    instance: Option<&str>,
    f: impl FnOnce(&ObsClient) -> Result<T, GeectlError>,
) -> Result<T, GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let base = config::obs_base_url(instance, &file_config)?;
    let credentials = match instance {
        Some(alias) => EnvObsAuth::credentials_for_instance(alias, &file_config.instances)?,
        None => EnvObsAuth::credentials_for_url(&base, &base, &file_config.instances)?,
    };
    let client =
        ObsClient::with_base_and_credentials(&http, &base, credentials)?.with_debug(debug.clone());
    f(&client)
}

/// A `GiteaClient` for the configured Gitea instance (config file /
/// env / default) with the environment Bearer token, resolved for
/// the configured URL from its instance alias like the OBS arm
/// (`EnvGiteaAuth::credentials_for_url`, the same per-URL shape
/// as the OBS arm), reporting on the debug channel. With
/// `--instance`, strictly the flagged alias's variables; the host
/// stays the configured URL. Construction fails closed without a
/// token.
/// The same construction as the `gitea` group's, except the
/// alias-aware resolution.
fn gitea_client<'a>(
    http: &'a dyn HttpClient,
    debug: &Debug,
    instance: Option<&str>,
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
    let credentials = match instance {
        Some(alias) => EnvGiteaAuth::credentials_for_instance(alias, &file_config.instances)?,
        None => EnvGiteaAuth::credentials_for_url(base, base, &file_config.instances)?,
    };
    GiteaClient::with_base(http, base, credentials).map(|client| client.with_debug(debug.clone()))
}

/// Run `f` with a `GiteaClient` for the configured instance. The
/// client borrows the transport, so both live in this scope and
/// the caller runs inside it.
fn with_gitea<T>(
    debug: &Debug,
    instance: Option<&str>,
    f: impl FnOnce(&GiteaClient) -> Result<T, GeectlError>,
) -> Result<T, GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = gitea_client(&http, debug, instance)?;
    f(&client)
}

fn project_match_json(match_: &ProjectMatch) -> Value {
    json!({
        "source": "obs",
        "kind": "project",
        "name": match_.name,
        "title": match_.title,
        "description": match_.description,
    })
}

fn gitea_user_json(user: &GiteaUser) -> Value {
    json!({
        "source": "gitea",
        "kind": "user",
        "login": user.login,
        "full_name": user.full_name,
        "email": user.email,
    })
}

fn with_header(header: &str, text: String) -> String {
    format!("{header}\n{text}")
}

/// The search text: the coordinate header, then one table over
/// both arms' results — SOURCE/KIND/NAME/DETAIL, OBS projects
/// first then Gitea users. A scoped search shows only its arm's
/// entries; a search that matched nothing names the empty case
/// once, not once per arm.
fn render_search_text(
    query: &PersonSearch,
    result: &PersonSearchResult,
) -> Result<String, GeectlError> {
    let mut header = format!("query: {}", query.query);
    if let Some(backend) = query.backend {
        header.push_str(&format!(" backend: {}", backend.as_str()));
    }
    Ok(with_header(&header, render_results(query, result)?))
}

fn render_results(
    query: &PersonSearch,
    result: &PersonSearchResult,
) -> Result<String, GeectlError> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    if query.backend.is_none() || query.backend == Some(PersonBackend::Obs) {
        for match_ in &result.projects {
            rows.push(vec![
                "obs".to_string(),
                "project".to_string(),
                match_.name.clone(),
                match_
                    .title
                    .as_deref()
                    .map(strip_terminal_controls_marked)
                    .unwrap_or_else(|| "-".to_string()),
            ]);
        }
    }
    if query.backend.is_none() || query.backend == Some(PersonBackend::Gitea) {
        for user in &result.users {
            rows.push(vec![
                "gitea".to_string(),
                "user".to_string(),
                user.login.clone(),
                user_detail(user),
            ]);
        }
    }
    if rows.is_empty() {
        return Ok(format!("no results matching '{}'", query.query));
    }
    render_table(&["SOURCE", "KIND", "NAME", "DETAIL"], &rows)
}

/// A user's detail: the full name, with the email in angle
/// brackets when present. Either part empty on its own renders
/// as the other; both empty render as `-`, like a missing title:
/// a blank cell would read as a missing column.
fn user_detail(user: &GiteaUser) -> String {
    let full_name = strip_terminal_controls_marked(&user.full_name);
    let email = strip_terminal_controls_marked(&user.email);
    match (full_name.is_empty(), email.is_empty()) {
        (true, true) => "-".to_string(),
        (false, true) => full_name,
        (true, false) => email,
        (false, false) => format!("{full_name} <{email}>"),
    }
}

/// A table with a header and space-aligned columns. A row with the
/// wrong cell count is an internal error: the table is built from
/// library shapes, so a mismatch is a caller bug, and the CLI
/// reports it instead of panicking — the same shape as the
/// pull-request group's `render_table`.
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
                    "person search table row has {} cells, headers have {}",
                    row.len(),
                    headers.len()
                ),
            ));
        }
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }
    let mut lines = vec![join_row(headers, &widths)];
    for row in rows {
        let cells: Vec<&str> = row.iter().map(String::as_str).collect();
        lines.push(join_row(&cells, &widths));
    }
    Ok(lines.join("\n"))
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
    use crate::obs::ProjectMatch;
    use clap::Parser;

    /// A minimal CLI for parsing `person` subcommands in tests.
    #[derive(Debug, Parser)]
    #[command(name = "geectl")]
    struct TestCli {
        #[command(subcommand)]
        command: PersonCommand,
    }

    fn parse(args: &[&str]) -> PersonCommand {
        TestCli::parse_from(args).command
    }

    fn project(name: &str, title: Option<&str>) -> ProjectMatch {
        ProjectMatch {
            name: name.to_string(),
            title: title.map(str::to_string),
            description: None,
        }
    }

    fn user(login: &str, full_name: &str, email: &str) -> GiteaUser {
        GiteaUser {
            login: login.to_string(),
            full_name: full_name.to_string(),
            email: email.to_string(),
        }
    }

    #[test]
    fn search_parses_query_positional() {
        let command = parse(&["geectl", "search", "alice"]);
        assert!(matches!(
            command,
            PersonCommand::Search(SearchArgs {
                query,
                backend: None
            }) if query == "alice"
        ));
    }

    #[test]
    fn search_parses_backend_flag() {
        let command = parse(&["geectl", "search", "alice", "--backend", "gitea"]);
        assert!(matches!(
            command,
            PersonCommand::Search(SearchArgs {
                query,
                backend: Some(backend)
            }) if query == "alice" && backend == "gitea"
        ));
    }

    #[test]
    fn search_rejects_an_unknown_backend() {
        let err = TestCli::try_parse_from(["geectl", "search", "alice", "--backend", "github"])
            .expect_err("an unknown backend must not parse");
        assert!(err.to_string().contains("invalid value 'github'"));
    }

    #[test]
    fn search_text_renders_one_table_and_names_the_empty_case_once() {
        let query = PersonSearch {
            query: "alice".to_string(),
            backend: None,
        };
        let result = PersonSearchResult {
            projects: vec![project("devel:languages:python", Some("Python"))],
            users: vec![user("alice", "Alice", "alice@example.com")],
        };
        let text =
            render_search_text(&query, &result).expect("the locally built rows match the headers");
        let mut lines = text.lines();
        assert_eq!(lines.next().expect("a header line"), "query: alice");
        let columns = lines.next().expect("a column header line");
        assert!(columns.contains("SOURCE"));
        assert!(columns.contains("KIND"));
        assert!(columns.contains("NAME"));
        assert!(columns.contains("DETAIL"));
        let row = lines.next().expect("a project row line");
        assert!(row.contains("obs"));
        assert!(row.contains("project"));
        assert!(row.contains("devel:languages:python"));
        assert!(row.contains("Python"));
        let row = lines.next().expect("a user row line");
        assert!(row.contains("gitea"));
        assert!(row.contains("user"));
        assert!(row.contains("alice"));
        assert!(row.contains("Alice <alice@example.com>"));
        assert_eq!(lines.next(), None);
        let empty = PersonSearchResult::default();
        assert_eq!(
            render_search_text(&query, &empty).expect("the locally built rows match the headers"),
            "query: alice\nno results matching 'alice'"
        );
    }

    #[test]
    fn search_text_scoped_to_one_arm_shows_only_that_arms_entries() {
        let query = PersonSearch {
            query: "alice".to_string(),
            backend: Some(PersonBackend::Gitea),
        };
        let result = PersonSearchResult {
            projects: vec![project("devel:languages:python", Some("Python"))],
            users: vec![],
        };
        assert_eq!(
            render_search_text(&query, &result).expect("the locally built rows match the headers"),
            "query: alice backend: gitea\nno results matching 'alice'"
        );
    }

    #[test]
    fn search_text_renders_a_dash_for_empty_user_fields() {
        let query = PersonSearch {
            query: "alice".to_string(),
            backend: Some(PersonBackend::Gitea),
        };
        let result = PersonSearchResult {
            projects: vec![],
            users: vec![user("alice2", "", "")],
        };
        let text =
            render_search_text(&query, &result).expect("the locally built rows match the headers");
        let row = text.lines().nth(2).expect("a row line");
        assert_eq!(
            row.split_whitespace().collect::<Vec<_>>(),
            ["gitea", "user", "alice2", "-"],
            "empty full name and email render as one dash, not blanks:\n{text}"
        );
    }

    #[test]
    fn render_table_rejects_a_row_with_the_wrong_cell_count() {
        let err = render_table(&["NAME"], &[vec!["a".to_string(), "b".to_string()]])
            .expect_err("a row with two cells for one header must fail");
        assert!(matches!(err.kind(), ErrorKind::InternalError));
        assert!(err.to_string().contains("2 cells"));
    }

    #[test]
    fn match_json_carries_the_typed_values() {
        // The payload builders, not the client: the client search
        // is covered by the library tests and the integration
        // tests, which run the whole leaf.
        let project_json = project_match_json(&project("devel:languages:python", Some("Python")));
        assert_eq!(project_json["source"], "obs");
        assert_eq!(project_json["kind"], "project");
        assert_eq!(project_json["name"], "devel:languages:python");
        assert_eq!(project_json["title"], "Python");
        let user_json = gitea_user_json(&user("alice", "Alice", "alice@example.com"));
        assert_eq!(user_json["source"], "gitea");
        assert_eq!(user_json["kind"], "user");
        assert_eq!(user_json["login"], "alice");
        assert_eq!(user_json["full_name"], "Alice");
        assert_eq!(user_json["email"], "alice@example.com");
    }

    #[test]
    fn search_text_marks_stripped_terminal_controls_in_titles_and_users() {
        // Project titles and Gitea full names and emails reach the
        // terminal through the person search tables: escape
        // sequences, bells, and bidirectional marks are stripped,
        // leaving a marker where characters were removed, while a
        // name that is empty still renders as `-`. A name of only
        // removed characters renders as the marker itself.
        let query = PersonSearch {
            query: "alice".to_string(),
            backend: None,
        };
        let result = PersonSearchResult {
            projects: vec![project("devel:languages:python", Some("Py\u{1b}[2Jthon"))],
            users: vec![
                user("alice", "Al\u{7}ice\u{202e}", "alice@example.com"),
                user("bob", "\u{7}", ""),
            ],
        };
        let text = render_results(&query, &result).unwrap();
        assert!(!text.contains('\u{1b}'), "text: {text}");
        assert!(!text.contains('\u{7}'), "text: {text}");
        assert!(!text.contains('\u{202e}'), "text: {text}");
        assert!(text.contains("Py…[2Jthon"), "text: {text}");
        assert!(text.contains("Al…ice…"), "text: {text}");
        assert!(text.contains("bob"), "text: {text}");
        let query = PersonSearch {
            query: "bob".to_string(),
            backend: Some(PersonBackend::Gitea),
        };
        let result = PersonSearchResult {
            projects: vec![],
            users: vec![user("bob", "\u{7}", "")],
        };
        let text = render_results(&query, &result).unwrap();
        assert!(text.contains("…"), "text: {text}");
    }
}
