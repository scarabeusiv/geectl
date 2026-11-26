//! `prjconf` — the project-config command group: read, validate and
//! clean a project's prjconf (`_config`).
//!
//! A project's prjconf is the file the scheduler reads to decide what
//! a build installs, prefers, replaces and ignores. It is plain text
//! with RPM macro expansion and `%if` conditionals, inherited along
//! the repository path. This group reads the config, sanity-checks it
//! against the effective config the server builds per repository, and
//! cleans it up — removing only the lines proven dead in every
//! repository and architecture.
//!
//! The checks: a `Prefer` / `Substitute` / `Support` target that
//! resolves to no package in any repository on the path and no binary
//! built by one; a `!X` removal of a token that was never added; a
//! `!*` clear of an empty list; a `Substitute` line a later line
//! overrides; a duplicate token; an unknown keyword; a `Prefer: -X`
//! avoid losing to an inherited `Support` / `Preinstall` /
//! `Required`; a `%if` branch false in a repository and
//! architecture evaluation (cleanup removes it only when it is false
//! in every evaluation, with `--include-path-dependent`); and parse errors (`FromScratch:` not first,
//! unclosed `%if`, stray `%endif`).
//!
//! A `Prefer` split across lines takes effect: list directives
//! accumulate, so a multi-line `Prefer` is one list. What the parser
//! makes dead is flagged instead (see the module docs in
//! `src/obs/prjconf.rs` for the server-source pins).
//!
//! `validate` exits 24 when error findings exist, with the findings
//! in the error context (the changelog `validate` semantics).
//! Warnings alone do not fail the run. `clean` is a mutation:
//! dry-run by default, `--live` applies the write. A parse error in
//! the own config, or an inherited unclosed `%if`, refuses a
//! cleanup. A prjconf is inherited, so by default `clean` removes
//! only lines proven dead independent of the repository path and
//! architecture; `--include-path-dependent` also removes lines
//! proven dead in every evaluation of this project.
//!
//! Coordinate header (design rule 1): every text rendering opens
//! with one header line echoing the query coordinates (`project: …`);
//! findings follow as `repository/project:line: kind: message` lines.

use crate::config;
use crate::debug::Debug;
use crate::error::{ErrorKind, GeectlError};
use crate::http::{HttpClient, Timeout, UreqClient};
use crate::obs::{
    format_prjconf_report, prjconf_finding_line, prjconf_skipped_line, DryRun, ObsClient,
    PrjconfCleanResult, PrjconfFinding, PrjconfSeverity, PrjconfSkipped, PrjconfValidation,
};
use clap::Subcommand;
use serde_json::{json, Value};

/// The `prjconf` subcommands. Names are the library operation names
/// in kebab-case, grouped under the top-level noun.
#[derive(Debug, Subcommand)]
pub enum PrjconfCommand {
    /// Show a project's raw prjconf.
    Show {
        /// The project to read the prjconf of.
        project: String,
    },
    /// Validate a project's prjconf across its repositories.
    /// Exits 24 when error findings exist.
    ///
    /// The config is evaluated once per repository and
    /// architecture, with one buildconfig fetch per repository;
    /// package and owner lookups are cached across evaluations.
    /// At most 32 repositories are evaluated; a project with more
    /// is refused. Requires OBS credentials, checked before any
    /// request. A `%if` branch is
    /// reported dead only when it is dead in every evaluation.
    Validate {
        /// The project to validate the prjconf of.
        project: String,
    },
    /// Clean a project's prjconf. A mutation: dry-run is the
    /// default. Shares validate's credential gate and
    /// 32-repository bound.
    ///
    /// A prjconf is inherited: downstream projects concatenate
    /// this config with different repository paths, architectures
    /// and `_repository` / `_project` values. By default clean
    /// removes only lines proven dead independent of the
    /// repository path and architecture (same-line duplicate
    /// tokens and unconditionally overridden `Substitute` lines);
    /// `--include-path-dependent` also removes lines proven dead
    /// in every repository and architecture evaluation of this
    /// project.
    Clean {
        /// The project to clean the prjconf of.
        project: String,
        /// Also remove lines proven dead only in this project's
        /// own repository and architecture evaluations.
        #[arg(long)]
        include_path_dependent: bool,
    },
}

/// Run a `prjconf` command, returning the JSON payload and its text
/// rendering.
pub fn run_prjconf(
    command: PrjconfCommand,
    debug: &Debug,
    live: bool,
    instance: Option<&str>,
) -> Result<(Value, String), GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let client = obs_client(&http, debug, instance)?;
    match command {
        PrjconfCommand::Show { project } => {
            let config = client.project_config(&project)?;
            let payload = json!({
                "project": project,
                "config": config,
            });
            Ok((payload, with_header(&format!("project: {project}"), config)))
        }
        PrjconfCommand::Validate { project } => {
            let validation = client.validate_project_config(&project)?;
            run_validate(&project, &validation)
        }
        PrjconfCommand::Clean {
            project,
            include_path_dependent,
        } => {
            let dry_run = DryRun::new(!live).with_debug(debug.clone());
            let result = client.clean_project_config(&project, &dry_run, include_path_dependent)?;
            Ok((clean_json(&result), render_clean(&result, live)))
        }
    }
}

/// `validate`: a config with no error findings returns the payload;
/// a config with error findings returns the validation error (exit
/// 24) with the findings in the error context, per the changelog
/// `validate` semantics. Warnings alone do not fail the run: they
/// are listed in the text rendering with the payload.
fn run_validate(
    project: &str,
    validation: &PrjconfValidation,
) -> Result<(Value, String), GeectlError> {
    if validation
        .findings
        .iter()
        .any(|f| f.severity == PrjconfSeverity::Error)
    {
        return Err(GeectlError::new(
            ErrorKind::ValidationFailed,
            format_prjconf_report(&validation.findings, &validation.skipped),
        ));
    }
    let payload = validation_json(validation);
    let mut text = format!("project: {project}");
    for finding in &validation.findings {
        text.push_str(&format!("\n{}", prjconf_finding_line(finding)));
    }
    for skip in &validation.skipped {
        text.push_str(&format!("\n{}", prjconf_skipped_line(skip)));
    }
    if validation.findings.is_empty() {
        text.push_str("\nOK: clean");
    } else {
        text.push_str(&format!(
            "\nwarnings: {} finding(s), no errors",
            validation.findings.len()
        ));
    }
    Ok((payload, text))
}

/// An `ObsClient` for the effective OBS base (config file / env /
/// default) with the environment OBS credentials for that URL,
/// reporting on the debug channel. Credentials resolve to `None`
/// when unconfigured; the operations fail closed themselves when
/// they need them.
fn obs_client<'a>(
    http: &'a dyn HttpClient,
    debug: &Debug,
    instance: Option<&str>,
) -> Result<ObsClient<'a>, GeectlError> {
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let (obs_url, credentials) = config::obs_endpoint(instance, &file_config)?;
    ObsClient::with_base_and_credentials(http, &obs_url, credentials)
        .map(|client| client.with_debug(debug.clone()))
}

/// Prepend the one-line coordinate header to a text rendering
/// (design rule 1): `header` first, then the body unchanged.
fn with_header(header: &str, text: String) -> String {
    format!("{header}\n{text}")
}

fn validation_json(validation: &PrjconfValidation) -> Value {
    json!({
        "project": validation.project,
        "findings": validation.findings.iter().map(finding_json).collect::<Vec<_>>(),
        "skipped": validation.skipped.iter().map(skipped_json).collect::<Vec<_>>(),
        "verdict": if validation.findings.iter().any(|f| f.severity == PrjconfSeverity::Error) { "fail" } else { "pass" },
    })
}

fn clean_json(result: &PrjconfCleanResult) -> Value {
    json!({
        "project": result.project,
        "applied": result.applied,
        "removed_lines": result.removed_lines,
        "cleaned": result.cleaned,
        "findings": result.findings.iter().map(finding_json).collect::<Vec<_>>(),
        "skipped": result.skipped.iter().map(skipped_json).collect::<Vec<_>>(),
    })
}

fn finding_json(finding: &PrjconfFinding) -> Value {
    json!({
        "kind": finding.kind.as_str(),
        "severity": match finding.severity {
            PrjconfSeverity::Error => "error",
            PrjconfSeverity::Warning => "warning",
        },
        "repository": finding.repository,
        "project": finding.project,
        "line": finding.line,
        "message": finding.message,
    })
}

fn skipped_json(skipped: &PrjconfSkipped) -> Value {
    json!({
        "repository": skipped.repository,
        "project": skipped.project,
        "line": skipped.line,
        "reason": skipped.reason,
    })
}

/// One finding as a text line (library rendering, sanitized).
fn finding_line(finding: &PrjconfFinding) -> String {
    prjconf_finding_line(finding)
}

fn skipped_line(skipped: &PrjconfSkipped) -> String {
    prjconf_skipped_line(skipped)
}

fn render_clean(result: &PrjconfCleanResult, live: bool) -> String {
    let mut text = format!("project: {}", result.project);
    let removed = result.removed_lines.len();
    if removed == 0 {
        text.push_str("\nnothing to remove: no dead lines");
    } else if live {
        text.push_str(&format!(
            "\napplied: removed {} line{}",
            removed,
            if removed == 1 { "" } else { "s" }
        ));
    } else {
        text.push_str(&format!(
            "\ndry-run: would put cleaned config for {} ({} line{} removed)",
            result.project,
            removed,
            if removed == 1 { "" } else { "s" }
        ));
    }
    if !result.removed_lines.is_empty() {
        let lines: Vec<String> = result.removed_lines.iter().map(usize::to_string).collect();
        text.push_str(&format!("\nremoved lines: {}", lines.join(", ")));
    }
    for finding in &result.findings {
        text.push_str(&format!("\n{}", finding_line(finding)));
    }
    for skip in &result.skipped {
        text.push_str(&format!("\n{}", skipped_line(skip)));
    }
    if !result.cleaned.is_empty() {
        text.push_str(&format!("\n{}", truncate_cleaned(&result.cleaned)));
    }
    text
}

/// The text rendering of a cleaned config is capped: a large config
/// would drown the findings. The JSON payload keeps the full text.
const CLEAN_TEXT_MAX_LINES: usize = 100;

fn truncate_cleaned(cleaned: &str) -> String {
    let trimmed = cleaned.trim_end();
    let mut lines = trimmed.split_inclusive('\n');
    let mut kept: Vec<&str> = lines.by_ref().take(CLEAN_TEXT_MAX_LINES).collect();
    if let Some(last) = kept.last_mut() {
        *last = last
            .strip_suffix("\r\n")
            .or_else(|| last.strip_suffix('\n'))
            .unwrap_or(last);
    }
    let rest = lines.count();
    if rest == 0 {
        return trimmed.to_string();
    }
    format!(
        "{}\n… ({} more line{})",
        kept.concat(),
        rest,
        if rest == 1 { "" } else { "s" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_cleaned_caps_long_configs() {
        let cleaned: String = (1..=150)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let rendered = truncate_cleaned(&cleaned);
        assert!(rendered.starts_with("line 1\nline 2"));
        assert!(rendered.contains("line 100"));
        assert!(!rendered.contains("line 101"));
        assert!(rendered.ends_with("… (50 more lines)"));
    }

    #[test]
    fn truncate_cleaned_singular_more_line() {
        let cleaned: String = (1..=101)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        assert!(truncate_cleaned(&cleaned).ends_with("… (1 more line)"));
    }

    #[test]
    fn truncate_cleaned_preserves_crlf_line_endings() {
        let cleaned = "line 1\r\nline 2\r\n";
        assert_eq!(truncate_cleaned(cleaned), "line 1\r\nline 2");
    }

    #[test]
    fn truncate_cleaned_caps_crlf_configs() {
        let cleaned: String = (1..=150).map(|n| format!("line {n}\r\n")).collect();
        let out = truncate_cleaned(&cleaned);
        assert!(out.starts_with("line 1\r\nline 2\r\n"));
        assert!(out.ends_with("… (50 more lines)"));
        assert_eq!(out.matches("\r\n").count(), 99);
    }

    #[test]
    fn truncate_cleaned_keeps_short_configs_whole() {
        assert_eq!(truncate_cleaned("Prefer: foo\n"), "Prefer: foo");
        assert_eq!(truncate_cleaned(""), "");
    }
}
