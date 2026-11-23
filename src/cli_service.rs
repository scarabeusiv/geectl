//! `service` — the service command group: a package's source
//! services (`_service`), their last run state, and the server-side
//! trigger.
//!
//! Two reads and one mutation over `ObsClient`, thin like `remote`:
//! the flags mirror the library parameters, the JSON payload carries
//! the typed values, and the text renderer prints the same values in
//! a human-readable form. No XML reaches the user anywhere.
//!
//! The trigger is the one mutation: it runs the package's services
//! on the server. Wait and merge stay out by the Factory filter (see
//! `docs/cli-swaps.md`); local runs belong to the podman build
//! vertical (issue #43). There is no osc command for either read;
//! osc reaches the same data only via `osc api`.
//!
//! Coordinate header (design rule 1):
//! each text rendering opens with one header line echoing the query
//! coordinates (`project: … package: …`); tables follow as designed.
//!
//! Documented omissions (not gaps): `list` text carries
//! no parameter values beyond `name=value` pairs (the full values
//! are in the JSON payload); `state` text carries the error verbatim
//! (multi-line server text) but no md5s beyond the two the state
//! itself names. The JSON payload remains the complete record in
//! every case.
//!
//! Credentials in dry-run: the reads need none, but credentials
//! resolve up front (see `docs/cli.md`) — a half-configured pair
//! fails fast here, even for the reads — and the trigger is a
//! mutation like any other: dry-run is the default, `--live`
//! performs it.

use clap::Subcommand;
use geectl::auth::EnvObsAuth;
use geectl::config;
use geectl::debug::Debug;
use geectl::error::GeectlError;
use geectl::http::{HttpClient, Timeout, UreqClient};
use geectl::obs::{DryRun, ObsClient, ServiceDef, ServiceState, ServiceTrigger, OBS_API_BASE};
use serde_json::{json, Value};

/// The `service` subcommands. Names are the library operation names
/// in kebab-case, grouped under the top-level noun.
#[derive(Debug, Subcommand)]
pub enum ServiceCommand {
    /// List the services defined in a package's `_service` file.
    List {
        /// The project the package lives in.
        project: String,
        /// The package to list the services of.
        package: String,
    },
    /// Show the last service run state of a package.
    State {
        /// The project the package lives in.
        project: String,
        /// The package to show the service state of.
        package: String,
    },
    /// Trigger a server-side run of a package's services. A mutation:
    /// dry-run is the default (only the plan is printed); `--live`
    /// performs the trigger. The trigger only queues the run: the
    /// server runs the services asynchronously, so a successful
    /// trigger says the run was queued, not that it finished. Check
    /// the outcome with `service state`.
    Trigger {
        /// The project the package lives in.
        project: String,
        /// The package to trigger the services of.
        package: String,
        /// The service the trigger is for: it must be defined in the
        /// package's `_service`, and the trigger fails closed when it
        /// is not. The server runs the whole service set either way;
        /// without this flag, every service runs.
        #[arg(long)]
        service: Option<String>,
    },
}

/// Run a `service` command, returning the JSON payload and its text
/// rendering.
pub fn run_service(
    command: ServiceCommand,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = obs_client(&http, debug)?;
    let dry_run = DryRun::new(!live).with_debug(debug.clone());
    match command {
        ServiceCommand::List { project, package } => {
            let services = client.list_services(&project, &package)?;
            let payload = json!({
                "project": project,
                "package": package,
                "services": services.as_ref().map(|services| {
                    services.iter().map(service_json).collect::<Vec<_>>()
                }),
            });
            Ok((
                payload,
                with_header(
                    &format!("project: {project} package: {package}"),
                    render_services(&services),
                ),
            ))
        }
        ServiceCommand::State { project, package } => {
            let state = client.service_state(&project, &package)?;
            let payload = json!({
                "project": project,
                "package": package,
                "state": state.as_ref().map(service_state_json),
            });
            Ok((
                payload,
                with_header(
                    &format!("project: {project} package: {package}"),
                    render_service_state(&state),
                ),
            ))
        }
        ServiceCommand::Trigger {
            project,
            package,
            service,
        } => {
            let triggered =
                client.trigger_service(&project, &package, service.as_deref(), &dry_run)?;
            let mut payload = json!({
                "project": project,
                "package": package,
                "service": service,
                "triggered": triggered.is_some(),
                "queued": triggered.is_some(),
                "code": triggered.as_ref().map(|trigger| trigger.code.as_str()),
                "dry_run": dry_run.enabled(),
            });
            if dry_run.enabled() {
                if let Value::Object(map) = &mut payload {
                    map.insert(
                        "plan".to_string(),
                        Value::Array(dry_run.plan().into_iter().map(Value::String).collect()),
                    );
                }
            }
            Ok((
                payload,
                with_header(
                    &format!("project: {project} package: {package}"),
                    render_trigger(&triggered, &service, &dry_run, &project, &package),
                ),
            ))
        }
    }
}

/// An `ObsClient` for the effective OBS base (config file / env /
/// default) with the environment OBS credentials for that URL,
/// reporting on the debug channel. Credentials resolve to `None`
/// when unconfigured; the operations fail closed themselves when
/// they need them. A half-configured pair fails fast here, even for
/// the reads (see the module docs).
fn obs_client<'a>(http: &'a dyn HttpClient, debug: &Debug) -> Result<ObsClient<'a>, GeectlError> {
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let effective = config::effective_config(&file_config)?;
    // Unreachable by construction — effective_config always yields
    // Some(obs_url) — but kept as a defensive default rather than an
    // unwrap: a config change must not turn into a panic here.
    let obs_url = effective.config.obs_url.as_deref().unwrap_or(OBS_API_BASE);
    let credentials = EnvObsAuth::credentials_for_url(obs_url, obs_url, &file_config.instances)?;
    ObsClient::with_base_and_credentials(http, obs_url, credentials)
        .map(|client| client.with_debug(debug.clone()))
}

/// Prepend the one-line coordinate header to a text rendering
/// (design rule 1): `header` first, then the body unchanged.
fn with_header(header: &str, text: String) -> String {
    format!("{header}\n{text}")
}

fn service_json(service: &ServiceDef) -> Value {
    json!({
        "name": service.name,
        "mode": service.mode,
        "params": service
            .params
            .iter()
            .map(|param| json!({
                "name": param.name,
                "value": param.value,
            }))
            .collect::<Vec<_>>(),
        "comments": service.comments,
    })
}

fn service_state_json(state: &ServiceState) -> Value {
    json!({
        "code": state.code.to_string(),
        "xsrcmd5": state.xsrcmd5,
        "lsrcmd5": state.lsrcmd5,
        "error": state.error,
    })
}

/// The service list as a table: name, mode, and `name=value`
/// parameters. Comments inside a service follow the table as
/// `comment (<name>):` lines, one per comment. `None` means the
/// package has no `_service` file at all; an empty list means the
/// file defines no services.
fn render_services(services: &Option<Vec<ServiceDef>>) -> String {
    match services {
        None => "no _service file".to_string(),
        Some(services) if services.is_empty() => "no services".to_string(),
        Some(services) => {
            let rows: Vec<Vec<String>> = services
                .iter()
                .map(|service| {
                    vec![
                        service.name.clone(),
                        if service.mode.is_empty() {
                            "-".to_string()
                        } else {
                            service.mode.clone()
                        },
                        if service.params.is_empty() {
                            "-".to_string()
                        } else {
                            service
                                .params
                                .iter()
                                .map(|param| format!("{}={}", param.name, param.value))
                                .collect::<Vec<_>>()
                                .join(", ")
                        },
                    ]
                })
                .collect();
            let mut text = render_table(&["NAME", "MODE", "PARAMS"], &rows);
            for service in services {
                for comment in &service.comments {
                    // The parser never lists an empty comment, but
                    // the render skips an empty or whitespace-only
                    // one anyway: a bare newline with no text would
                    // be a stray blank line.
                    if comment.trim().is_empty() {
                        continue;
                    }
                    let rendered = render_comment(&service.name, comment);
                    text.push('\n');
                    text.push_str(&rendered);
                }
            }
            text
        }
    }
}

/// One comment as a line under the table: `comment (<name>): <text>`.
/// A multi-line comment keeps its lines, indented under the first.
fn render_comment(name: &str, comment: &str) -> String {
    let mut lines = comment.lines();
    let Some(first) = lines.next() else {
        return String::new();
    };
    let mut out = format!("comment ({name}): {first}");
    for line in lines {
        out.push('\n');
        out.push_str("  ");
        out.push_str(line);
    }
    out
}

/// The service state as `key: value` lines: the run code, the two
/// md5s when the listing carries them, and the error verbatim when
/// the run failed. `None` means the listing carries no service
/// state at all.
fn render_service_state(state: &Option<ServiceState>) -> String {
    match state {
        None => "no service state".to_string(),
        Some(state) => {
            let mut lines = vec![format!("code: {}", state.code)];
            if let Some(xsrcmd5) = &state.xsrcmd5 {
                lines.push(format!("xsrcmd5: {xsrcmd5}"));
            }
            if let Some(lsrcmd5) = &state.lsrcmd5 {
                lines.push(format!("lsrcmd5: {lsrcmd5}"));
            }
            if let Some(error) = &state.error {
                lines.push(format!("error: {error}"));
            }
            lines.join("\n")
        }
    }
}

/// The trigger outcome as `key: value` lines: whether the services
/// were triggered, which service the trigger was for (`all` when no
/// service was named), and the server's status code when the trigger
/// went out. A live trigger only queues the run, so `triggered`
/// reads `yes (queued)` and points at `service state` for the
/// outcome; `--service` only validates the name — the whole service
/// set runs — so a named trigger carries a warning line. In dry-run
/// the plan lines follow the `dry_run` line.
fn render_trigger(
    triggered: &Option<ServiceTrigger>,
    service: &Option<String>,
    dry_run: &DryRun,
    project: &str,
    package: &str,
) -> String {
    let mut lines = vec![format!(
        "triggered: {}",
        if triggered.is_some() {
            "yes (queued)"
        } else {
            "no"
        }
    )];
    lines.push(format!("service: {}", service.as_deref().unwrap_or("all")));
    if service.is_some() {
        lines.push(
            "warning: --service only validates the name; the server runs the whole service set"
                .to_string(),
        );
    }
    if let Some(trigger) = triggered {
        lines.push(format!("code: {}", trigger.code));
    }
    lines.push(format!("dry_run: {}", dry_run.enabled()));
    if triggered.is_some() {
        lines.push(format!(
            "state: check the run with `geectl service state {project} {package}`"
        ));
    }
    if dry_run.enabled() {
        lines.extend(dry_run.plan());
    }
    lines.join("\n")
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
    use crate::{Cli, Command};
    use clap::Parser;
    use geectl::obs::{ServiceCode, ServiceParam};

    fn parse(args: &[&str]) -> ServiceCommand {
        match Cli::parse_from(args).command {
            Some(Command::Service { command }) => command,
            _ => panic!("expected a service command, got {:?}", args),
        }
    }

    #[test]
    fn list_and_state_parse_positionals() {
        assert!(matches!(
            parse(&["geectl", "service", "list", "P", "K"]),
            ServiceCommand::List { .. }
        ));
        assert!(matches!(
            parse(&["geectl", "service", "state", "P", "K"]),
            ServiceCommand::State { .. }
        ));
    }

    #[test]
    fn trigger_parses_positionals_and_service_flag() {
        assert!(matches!(
            parse(&["geectl", "service", "trigger", "P", "K"]),
            ServiceCommand::Trigger { service: None, .. }
        ));
        match parse(&[
            "geectl",
            "service",
            "trigger",
            "P",
            "K",
            "--service",
            "download_url",
        ]) {
            ServiceCommand::Trigger { service, .. } => {
                assert_eq!(service.as_deref(), Some("download_url"));
            }
            other => panic!("expected a trigger command, got {other:?}"),
        }
    }

    #[test]
    fn unknown_service_command_fails_to_parse() {
        assert!(Cli::try_parse_from(["geectl", "service", "frobnicate"]).is_err());
        assert!(Cli::try_parse_from(["geectl", "service", "list", "only-one"]).is_err());
        assert!(Cli::try_parse_from(["geectl", "service", "trigger", "only-one"]).is_err());
        assert!(
            Cli::try_parse_from(["geectl", "service", "trigger", "P", "K", "--service"]).is_err()
        );
    }

    fn sample_services() -> Vec<ServiceDef> {
        vec![
            ServiceDef {
                name: "download_url".to_string(),
                mode: String::new(),
                params: vec![ServiceParam {
                    name: "url".to_string(),
                    value: "https://example.com/foo-1.0.tar.gz".to_string(),
                }],
                comments: vec!["fetched upstream\nfrom the mirror".to_string()],
            },
            ServiceDef {
                name: "set_version".to_string(),
                mode: "manual".to_string(),
                params: vec![],
                comments: vec![],
            },
        ]
    }

    #[test]
    fn list_renders_name_mode_and_params() {
        let text = render_services(&Some(sample_services()));
        assert!(text.contains("download_url"));
        assert!(text.contains("url=https://example.com/foo-1.0.tar.gz"));
        assert!(text.contains("set_version"));
        assert!(text.contains("manual"));
        let payload = json!({
            "services": sample_services().iter().map(service_json).collect::<Vec<_>>(),
        });
        assert_eq!(payload["services"][0]["name"], "download_url");
        assert_eq!(
            payload["services"][0]["params"][0]["value"],
            "https://example.com/foo-1.0.tar.gz"
        );
        assert_eq!(payload["services"][1]["mode"], "manual");
        assert_eq!(
            payload["services"][0]["comments"],
            json!(["fetched upstream\nfrom the mirror"])
        );
        assert_eq!(payload["services"][1]["comments"], json!([]));
        assert!(text.contains("comment (download_url): fetched upstream"));
        assert_eq!(render_services(&Some(vec![])), "no services");
        assert_eq!(render_services(&None), "no _service file");
    }

    #[test]
    fn list_renders_multiline_comment_with_indented_continuation() {
        let services = vec![ServiceDef {
            name: "download_url".to_string(),
            mode: String::new(),
            params: vec![],
            comments: vec!["first line\nsecond line".to_string()],
        }];
        let text = render_services(&Some(services));
        assert!(text.contains("comment (download_url): first line\n  second line"));
    }

    #[test]
    fn list_skips_whitespace_only_comments() {
        let services = vec![ServiceDef {
            name: "download_url".to_string(),
            mode: String::new(),
            params: vec![],
            comments: vec!["   ".to_string()],
        }];
        let text = render_services(&Some(services));
        assert!(!text.contains("comment ("));
    }

    #[test]
    fn state_renders_code_md5s_and_error() {
        let state = ServiceState {
            code: ServiceCode::Failed,
            xsrcmd5: Some("abcd1234".to_string()),
            lsrcmd5: None,
            error: Some("download failed\nsecond line".to_string()),
        };
        let text = render_service_state(&Some(state.clone()));
        assert!(text.contains("code: failed"));
        assert!(text.contains("xsrcmd5: abcd1234"));
        assert!(!text.contains("lsrcmd5"));
        assert!(text.contains("error: download failed\nsecond line"));
        let payload = service_state_json(&state);
        assert_eq!(payload["code"], "failed");
        assert_eq!(payload["xsrcmd5"], "abcd1234");
        assert_eq!(payload["lsrcmd5"], Value::Null);
        assert_eq!(render_service_state(&None), "no service state");
    }

    #[test]
    fn trigger_renders_outcome_service_and_code() {
        let dry_run = DryRun::new(false);
        let triggered = Some(ServiceTrigger {
            service: Some("download_url".to_string()),
            code: "ok".to_string(),
        });
        let text = render_trigger(
            &triggered,
            &Some("download_url".to_string()),
            &dry_run,
            "P",
            "K",
        );
        assert!(text.contains("triggered: yes (queued)"));
        assert!(text.contains("service: download_url"));
        assert!(text.contains(
            "warning: --service only validates the name; the server runs the whole service set"
        ));
        assert!(text.contains("code: ok"));
        assert!(text.contains("dry_run: false"));
        assert!(text.contains("state: check the run with `geectl service state P K`"));
        // No service named: the whole set runs, rendered as `all`,
        // with no warning line.
        let text = render_trigger(
            &Some(ServiceTrigger {
                service: None,
                code: "ok".to_string(),
            }),
            &None,
            &dry_run,
            "P",
            "K",
        );
        assert!(text.contains("triggered: yes (queued)"));
        assert!(text.contains("service: all"));
        assert!(!text.contains("warning:"));
        assert!(text.contains("code: ok"));
        assert!(text.contains("state: check the run with `geectl service state P K`"));
    }

    #[test]
    fn trigger_dry_run_renders_no_trigger_and_the_plan() {
        let dry_run = DryRun::new(true);
        dry_run.allow("trigger services for P/K (service download_url)");
        let text = render_trigger(&None, &Some("download_url".to_string()), &dry_run, "P", "K");
        assert!(text.contains("triggered: no"));
        assert!(text.contains("service: download_url"));
        assert!(text.contains(
            "warning: --service only validates the name; the server runs the whole service set"
        ));
        assert!(!text.contains("code:"));
        assert!(text.contains("dry_run: true"));
        assert!(!text.contains("state: check the run"));
        assert!(text.contains("[dry-run] would: trigger services for P/K (service download_url)"));
    }

    /// The agreement test: every identifying field the JSON payload
    /// carries — service name, mode, `name=value` parameters, and
    /// comments; state code, md5s, and error — appears in the text
    /// rendering.
    /// The expected fields come from the payload itself, not from
    /// literals repeated from the rendering tests, so a payload field
    /// the text drops fails here.
    #[test]
    fn text_carries_every_identifying_field() {
        let services = sample_services();
        let payload: Vec<Value> = services.iter().map(service_json).collect();
        let text = render_services(&Some(services));
        for service in &payload {
            let name = service["name"].as_str().expect("name is a string");
            assert!(text.contains(name), "list text misses name {name}:\n{text}");
            // An empty mode renders as `-`, not as an empty field:
            // only a non-empty mode is an identifying field the text
            // must carry, so the contains-check skips it; the `-`
            // itself is pinned by the exact-row asserts below.
            let mode = service["mode"].as_str().expect("mode is a string");
            if !mode.is_empty() {
                assert!(text.contains(mode), "list text misses mode {mode}:\n{text}");
            }
            for param in service["params"].as_array().expect("params is an array") {
                let pair = format!(
                    "{}={}",
                    param["name"].as_str().expect("param name is a string"),
                    param["value"].as_str().expect("param value is a string")
                );
                assert!(
                    text.contains(&pair),
                    "list text misses param {pair}:\n{text}"
                );
            }
            for comment in service["comments"]
                .as_array()
                .expect("comments is an array")
            {
                let comment = comment.as_str().expect("comment is a string");
                // A multi-line comment renders with its
                // continuation lines indented, so each line is
                // checked on its own, not the raw string.
                for line in comment.lines() {
                    assert!(
                        text.contains(line),
                        "list text misses comment line {line}:\n{text}"
                    );
                }
            }
        }
        // Empty mode and empty params render as `-`: the rows pin it
        // exactly, so a renderer dropping the placeholder fails here.
        let rows: Vec<&str> = text.lines().collect();
        assert_eq!(
            rows[1],
            "download_url -      url=https://example.com/foo-1.0.tar.gz"
        );
        assert_eq!(rows[2], "set_version  manual -");

        let state = ServiceState {
            code: ServiceCode::Failed,
            xsrcmd5: Some("abcd1234".to_string()),
            lsrcmd5: Some("efgh5678".to_string()),
            error: Some("download failed".to_string()),
        };
        let payload = service_state_json(&state);
        let text = render_service_state(&Some(state));
        assert!(
            text.contains(&format!(
                "code: {}",
                payload["code"].as_str().expect("code is a string")
            )),
            "state text misses code:\n{text}"
        );
        for field in ["xsrcmd5", "lsrcmd5", "error"] {
            let value = payload[field].as_str().expect("field is a string");
            assert!(
                text.contains(&format!("{field}: {value}")),
                "state text misses {field}: {value}:\n{text}"
            );
        }
    }

    /// The trigger agreement test: the service name, the triggered
    /// flag, and the server's status code the payload carries all
    /// appear in the text rendering.
    #[test]
    fn trigger_text_carries_every_identifying_field() {
        let dry_run = DryRun::new(false);
        let triggered = Some(ServiceTrigger {
            service: Some("download_url".to_string()),
            code: "ok".to_string(),
        });
        let payload = json!({
            "service": "download_url",
            "triggered": true,
            "queued": true,
            "code": "ok",
        });
        let text = render_trigger(
            &triggered,
            &Some("download_url".to_string()),
            &dry_run,
            "P",
            "K",
        );
        assert!(
            text.contains(&format!(
                "service: {}",
                payload["service"].as_str().expect("service is a string")
            )),
            "trigger text misses service:\n{text}"
        );
        assert!(
            text.contains("triggered: yes (queued)"),
            "trigger text misses triggered:\n{text}"
        );
        assert!(
            payload["queued"].as_bool().expect("queued is a bool") && text.contains("(queued)"),
            "trigger text misses queued:\n{text}"
        );
        assert!(
            text.contains(&format!(
                "code: {}",
                payload["code"].as_str().expect("code is a string")
            )),
            "trigger text misses code:\n{text}"
        );
    }
}
