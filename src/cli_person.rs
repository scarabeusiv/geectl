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
//! Documented omissions (not gaps): the user table carries no
//! avatar URL (a URL, not an identifier); the project table
//! carries no description (long free text — titles stay). The
//! JSON payload remains the complete record in every case.

use crate::auth::{EnvGiteaAuth, EnvObsAuth};
use crate::config::{self, DEFAULT_GITEA_URL};
use crate::debug::{strip_terminal_controls_marked, Debug};
use crate::error::{ErrorKind, GeectlError};
use crate::gitea::{GiteaClient, GiteaUser};
use crate::http::{HttpClient, Timeout, UreqClient};
use crate::obs::{ObsClient, ProjectMatch, OBS_API_BASE};
use crate::person::{PersonBackend, PersonClient, PersonSearch, PersonSearchResult};
use clap::{Args, Subcommand};
use serde_json::{json, Value};

/// The `person` command group: the unified person search over
/// both backends.
#[derive(Debug, Subcommand)]
pub enum PersonCommand {
    /// Search for a person: the OBS projects naming the login,
    /// and the Gitea users matching the keyword. With no
    /// `--backend` both arms run, OBS first then Gitea.
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
                Some(PersonBackend::Obs) => with_obs(debug, |obs| {
                    search_with(&PersonClient::new(Some(obs), None), &query)
                }),
                Some(PersonBackend::Gitea) => with_gitea(debug, |gitea| {
                    search_with(&PersonClient::new(None, Some(gitea)), &query)
                }),
                None => with_obs(debug, |obs| {
                    with_gitea(debug, |gitea| {
                        search_with(&PersonClient::new(Some(obs), Some(gitea)), &query)
                    })
                }),
            }
        }
    }
}

/// Search for a person through the uniform client: the JSON
/// payload carries the query, the backend (`null` when both arms
/// run), and both arms' lists (the arm that did not run leaves
/// its list empty); the text is the header plus one section per
/// arm that ran.
fn search_with(
    client: &PersonClient,
    query: &PersonSearch,
) -> Result<(Value, String), GeectlError> {
    let result = client.search(query)?;
    let payload = json!({
        "query": query.query,
        "backend": query.backend.map(|backend| backend.as_str()),
        "projects": result.projects.iter().map(project_match_json).collect::<Vec<_>>(),
        "users": result.users.iter().map(gitea_user_json).collect::<Vec<_>>(),
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
/// env / default) with the environment Bearer <redacted>, resolved for
/// the configured URL from its instance alias like the OBS arm
/// (`EnvGiteaAuth::credentials_for_url`, the same per-URL shape
/// as the OBS arm), reporting on the debug channel. Construction
/// fails closed without a token.
/// The same construction as the `gitea` group's, except the
/// alias-aware resolution.
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
    let credentials = EnvGiteaAuth::credentials_for_url(base, base, &file_config.instances)?;
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

fn project_match_json(match_: &ProjectMatch) -> Value {
    json!({
        "name": match_.name,
        "title": match_.title,
        "description": match_.description,
    })
}

fn gitea_user_json(user: &GiteaUser) -> Value {
    json!({
        "login": user.login,
        "full_name": user.full_name,
        "email": user.email,
    })
}

fn with_header(header: &str, text: String) -> String {
    format!("{header}\n{text}")
}

/// The search text: the coordinate header, then one section per
/// arm that ran — the OBS projects, then the Gitea users. An arm
/// that did not run has no section; an arm that ran and matched
/// nothing names the empty case.
fn render_search_text(
    query: &PersonSearch,
    result: &PersonSearchResult,
) -> Result<String, GeectlError> {
    let mut header = format!("query: {}", query.query);
    if let Some(backend) = query.backend {
        header.push_str(&format!(" backend: {}", backend.as_str()));
    }
    let mut sections = Vec::new();
    if query.backend.is_none() || query.backend == Some(PersonBackend::Obs) {
        sections.push(format!(
            "OBS projects:\n{}",
            render_project_matches(&query.query, &result.projects)?
        ));
    }
    if query.backend.is_none() || query.backend == Some(PersonBackend::Gitea) {
        sections.push(format!(
            "Gitea users:\n{}",
            render_users(&query.query, &result.users)?
        ));
    }
    Ok(with_header(&header, sections.join("\n")))
}

fn render_project_matches(term: &str, matches: &[ProjectMatch]) -> Result<String, GeectlError> {
    if matches.is_empty() {
        return Ok(format!("no projects matching '{term}'"));
    }
    let rows: Vec<Vec<String>> = matches
        .iter()
        .map(|match_| {
            vec![
                match_.name.clone(),
                match_
                    .title
                    .as_deref()
                    .map(strip_terminal_controls_marked)
                    .unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    render_table(&["NAME", "TITLE"], &rows)
}

fn render_users(term: &str, users: &[GiteaUser]) -> Result<String, GeectlError> {
    if users.is_empty() {
        return Ok(format!("no users matching '{term}'"));
    }
    // Empty full names and emails render as `-`, like a missing
    // title in the project table: a blank cell would read as a
    // missing column.
    let rows: Vec<Vec<String>> = users
        .iter()
        .map(|user| {
            vec![
                user.login.clone(),
                {
                    let full_name = strip_terminal_controls_marked(&user.full_name);
                    if full_name.is_empty() {
                        "-".to_string()
                    } else {
                        full_name
                    }
                },
                {
                    let email = strip_terminal_controls_marked(&user.email);
                    if email.is_empty() {
                        "-".to_string()
                    } else {
                        email
                    }
                },
            ]
        })
        .collect();
    render_table(&["LOGIN", "FULL NAME", "EMAIL"], &rows)
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
    fn search_text_renders_both_sections_and_names_the_empty_case() {
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
        assert_eq!(lines.next().expect("an OBS section line"), "OBS projects:");
        let columns = lines.next().expect("a column header line");
        assert!(columns.contains("NAME"));
        assert!(columns.contains("TITLE"));
        let row = lines.next().expect("a row line");
        assert!(row.contains("devel:languages:python"));
        assert!(row.contains("Python"));
        assert_eq!(lines.next().expect("a Gitea section line"), "Gitea users:");
        let columns = lines.next().expect("a column header line");
        assert!(columns.contains("LOGIN"));
        assert!(columns.contains("FULL NAME"));
        assert!(columns.contains("EMAIL"));
        let row = lines.next().expect("a row line");
        assert!(row.contains("alice"));
        assert!(row.contains("Alice"));
        assert!(row.contains("alice@example.com"));
        assert_eq!(lines.next(), None);
        let empty = PersonSearchResult::default();
        assert_eq!(
            render_search_text(&query, &empty)
                .expect("the locally built rows match the headers"),
            "query: alice\nOBS projects:\nno projects matching 'alice'\nGitea users:\nno users matching 'alice'"
        );
    }

    #[test]
    fn search_text_scoped_to_one_arm_shows_only_that_section() {
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
            "query: alice backend: gitea\nGitea users:\nno users matching 'alice'"
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
        let row = text.lines().nth(3).expect("a row line");
        assert_eq!(
            row.split_whitespace().collect::<Vec<_>>(),
            ["alice2", "-", "-"],
            "empty full name and email render as dashes, not blanks:\n{text}"
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
        assert_eq!(project_json["name"], "devel:languages:python");
        assert_eq!(project_json["title"], "Python");
        let user_json = gitea_user_json(&user("alice", "Alice", "alice@example.com"));
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
        let projects = vec![project("devel:languages:python", Some("Py\u{1b}[2Jthon"))];
        let text = render_project_matches("alice", &projects).unwrap();
        assert!(!text.contains('\u{1b}'), "text: {text}");
        assert!(text.contains("Py…[2Jthon"), "text: {text}");
        let users = vec![
            user("alice", "Al\u{7}ice\u{202e}", "alice@example.com"),
            user("bob", "\u{7}", ""),
        ];
        let text = render_users("alice", &users).unwrap();
        assert!(!text.contains('\u{1b}'), "text: {text}");
        assert!(!text.contains('\u{7}'), "text: {text}");
        assert!(!text.contains('\u{202e}'), "text: {text}");
        assert!(text.contains("Al…ice…"), "text: {text}");
        assert!(text.contains("bob"), "text: {text}");
        let text = render_users("bob", &[user("bob", "\u{7}", "")]).unwrap();
        assert!(text.contains("…"), "text: {text}");
    }
}
