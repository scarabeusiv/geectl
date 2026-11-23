//! `vc` CLI commands (`geectl vc ...`).
//!
//! Pre-file validation/generation for `.changes` entries. The
//! commands work on a package directory on disk, optionally compared
//! against a target project/package on the instance (public reads,
//! no credentials). `validate` runs the V1–V12 rules; `generate`
//! emits the diff-derived skeleton entry as a draft for a human to
//! complete. Neither command files anything, and neither wires into
//! `commit` — pre-commit placement waits on the working-copy layer.
//!
//! Coordinate header (design rule 1): `validate` text opens with
//! `dir: <dir>` (plus `target: <project>/<package>` when given).
//! `generate` is a content command (design rule 4): the text IS the
//! entry, verbatim. On findings, `validate` exits 24
//! (`validation_failed`) and the error context lists one
//! `file:line: V#: message` line per finding, plus the skipped
//! rules — the same shape the skill's `changes-lint.sh` prints.

use clap::Subcommand;
use geectl::debug::Debug;
use geectl::error::{ErrorKind, GeectlError};
use geectl::http::{Timeout, UreqClient};
use geectl::obs::ObsClient;
use geectl::vc::generate::{generate, prepend_entry};
use geectl::vc::news::{extract_news_section, find_tarball, NewsSection};
use geectl::vc::spec::parse_spec;
use geectl::vc::validate::{validate, PackageSnapshot, ValidateOptions};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The `vc` command group: `.changes` validation and generation.
#[derive(Debug, Subcommand)]
pub enum VcCommand {
    /// Validate the `.changes` entries in a package directory
    /// against the V1–V12 rules. Local-only by default; with
    /// `--target-project`/`--target-package` the target-relative
    /// rules (V2, V4, V5 newest-vs-target, V11 older entries) run
    /// against the target's current files. Exits non-zero naming
    /// the failing rule.
    Validate {
        /// The package directory (default: the current directory).
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// The target project to compare against.
        #[arg(long)]
        target_project: Option<String>,
        /// The target package to compare against.
        #[arg(long)]
        target_package: Option<String>,
        /// Newest N entries to lint for format, and the number of
        /// new entries the submission may carry vs the target
        /// (default 1: one entry per SR).
        #[arg(long, default_value_t = 1)]
        entries: usize,
    },
    /// Generate a skeleton `.changes` entry from the diff against
    /// the target (version line, patch and dependency lists,
    /// license lines) plus the upstream NEWS section for the new
    /// version, extracted from the source tarball. The output is a
    /// draft: what the diff cannot supply arrives as explicit
    /// `TODO` slots for a human to fill. `--write` prepends the
    /// draft into the package's `.changes` file, and refuses an
    /// incomplete draft (V9 needs-detail).
    Generate {
        /// The package directory (default: the current directory).
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// The target project to diff against.
        #[arg(long)]
        target_project: Option<String>,
        /// The target package to diff against.
        #[arg(long)]
        target_package: Option<String>,
        /// The author line for the entry header, in `Name <email>`
        /// form — exactly what a bare `vc` invocation writes.
        #[arg(long)]
        author: Option<String>,
        /// Prepend the draft into the package's `.changes` file
        /// instead of printing it.
        #[arg(long)]
        write: bool,
    },
}

pub fn run_vc(
    command: VcCommand,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    // `vc` performs no OBS mutation: `--live` is ignored, like
    // `cat`. `--write` is the explicit local-write flag.
    let _ = live;
    match command {
        VcCommand::Validate {
            dir,
            target_project,
            target_package,
            entries,
        } => run_validate(&dir, target_project, target_package, entries, debug),
        VcCommand::Generate {
            dir,
            target_project,
            target_package,
            author,
            write,
        } => run_generate(&dir, target_project, target_package, author, write, debug),
    }
}

fn resolve_target(
    target_project: Option<String>,
    target_package: Option<String>,
) -> Result<Option<(String, String)>, GeectlError> {
    match (target_project, target_package) {
        (Some(project), Some(package)) => Ok(Some((project, package))),
        (None, None) => Ok(None),
        _ => Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "--target-project and --target-package go together: pass both or neither",
        )),
    }
}

fn load_local(dir: &Path) -> Result<PackageSnapshot, GeectlError> {
    let snapshot = PackageSnapshot::from_dir(dir).map_err(|err| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            format!("cannot read package directory {}: {err}", dir.display()),
        )
    })?;
    if snapshot.spec_names().is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("no .spec file in {}", dir.display()),
        ));
    }
    Ok(snapshot)
}

/// Fetch the target's current files: the file list, plus the text of
/// every `.spec` and `.changes` file. A target that does not exist
/// is `None` (a new package), never an error; any other failure
/// propagates — a failed lookup must never read as good news.
fn fetch_target(
    debug: &Debug,
    project: &str,
    package: &str,
) -> Result<Option<PackageSnapshot>, GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let (base, _instances) = crate::cli_remote::obs_endpoint()?;
    let client =
        ObsClient::with_base_and_credentials(&http, &base, geectl::auth::Credentials::None)?
            .with_debug(debug.clone());
    let files = match client.list_files(project, package, None) {
        Ok(files) => files,
        Err(err) if err.kind == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let mut snapshot = PackageSnapshot::default();
    for file in files {
        if file.name.ends_with(".spec") || file.name.ends_with(".changes") {
            let content = client.cat(project, package, &file.name, None)?;
            snapshot.files.insert(file.name, content);
        } else {
            // The name alone is enough for the file-list rules (V4);
            // a binary file must never be read as text.
            snapshot.files.insert(file.name, String::new());
        }
    }
    Ok(Some(snapshot))
}

/// The upstream NEWS section for the local version, when the
/// tarball is on disk and carries one. The second return value is
/// the lookup failure, when the lookup failed (as opposed to finding
/// no section); the third is whether a tarball was found at all
/// (V9 tells the two no-news cases apart).
fn lookup_news(dir: &Path, local: &PackageSnapshot) -> (Option<NewsSection>, Option<String>, bool) {
    let spec_names = local.spec_names();
    let Some(spec_name) = spec_names.first() else {
        return (None, None, false);
    };
    let spec = parse_spec(&local.files[*spec_name]);
    let Some(version) = spec.version else {
        return (None, None, false);
    };
    let source0 = spec.sources.first().map(String::as_str);
    let Some(tarball) = find_tarball(dir, &version, source0) else {
        return (None, None, false);
    };
    match extract_news_section(&tarball, &version) {
        Ok(section) => (section, None, true),
        Err(err) => (None, Some(err.to_string()), true),
    }
}

fn run_validate(
    dir: &Path,
    target_project: Option<String>,
    target_package: Option<String>,
    entries: usize,
    debug: &Debug,
) -> Result<(Value, String), GeectlError> {
    let target = resolve_target(target_project, target_package)?;
    let local = load_local(dir)?;
    let target_snapshot = match &target {
        Some((project, package)) => fetch_target(debug, project, package)?,
        None => None,
    };
    let (news, news_error, tarball_present) = lookup_news(dir, &local);
    let options = ValidateOptions {
        entries,
        news_section: news.as_ref().map(|section| section.text.as_str()),
        tarball_present,
        news_lookup_failed: news_error.is_some(),
    };
    let report = validate(&local, target_snapshot.as_ref(), &options);
    let mut skipped = report.skipped.clone();
    if let Some(reason) = news_error {
        skipped.push(geectl::vc::Skipped {
            rule: geectl::vc::Rule::V9,
            reason: format!("upstream NEWS lookup failed ({reason}) — V9 did not run"),
        });
    }
    let target_text = target
        .as_ref()
        .map(|(project, package)| format!(" target: {project}/{package}"))
        .unwrap_or_default();
    if report.passed() {
        let payload = json!({
            "dir": dir.display().to_string(),
            "target_project": target.as_ref().map(|(project, _)| project),
            "target_package": target.as_ref().map(|(_, package)| package),
            "findings": [],
            "skipped": skipped.iter().map(|s| json!({
                "rule": s.rule.id(),
                "reason": s.reason,
            })).collect::<Vec<_>>(),
            "verdict": "pass",
        });
        let mut text = format!("dir: {}{target_text}", dir.display());
        for skip in &skipped {
            text.push_str(&format!("\nskipped {}: {}", skip.rule.id(), skip.reason));
        }
        text.push_str("\nOK: clean");
        return Ok((payload, text));
    }
    let mut lines: Vec<String> = report
        .findings
        .iter()
        .map(|finding| {
            format!(
                "{}:{}: {}: {}",
                finding.file,
                finding.line,
                finding.rule.id(),
                finding.message
            )
        })
        .collect();
    for skip in &skipped {
        lines.push(format!("skipped {}: {}", skip.rule.id(), skip.reason));
    }
    Err(GeectlError::new(
        ErrorKind::ValidationFailed,
        format!(
            "{} finding(s):\n{}",
            report.findings.len(),
            lines.join("\n")
        ),
    ))
}

fn parse_author(author: &str) -> Result<String, GeectlError> {
    let Some((name, rest)) = author.split_once('<') else {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "--author takes 'Name <email>', got a value with no <email>",
        ));
    };
    // The same form `validate` V11 demands of a header author: a
    // non-empty name, and an email with no whitespace.
    let email = rest.strip_suffix('>').unwrap_or(rest);
    if name.trim().is_empty()
        || !rest.ends_with('>')
        || email.is_empty()
        || !email.contains('@')
        || email.contains(char::is_whitespace)
    {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "--author takes 'Name <email>' with a non-empty name and an email address",
        ));
    }
    Ok(author.trim().to_string())
}

/// Refuse a `--write` destination that is a symlink: the write
/// would follow it out of the checkout (`foo.changes` linked at
/// `~/.bashrc` turns `--write` into an arbitrary overwrite).
fn refuse_symlink_destination(path: &Path) -> Result<(), GeectlError> {
    if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(GeectlError::new(
            ErrorKind::ValidationFailed,
            format!(
                "refusing to write {}: the destination is a symlink",
                path.display()
            ),
        ));
    }
    Ok(())
}

fn run_generate(
    dir: &Path,
    target_project: Option<String>,
    target_package: Option<String>,
    author: Option<String>,
    write: bool,
    debug: &Debug,
) -> Result<(Value, String), GeectlError> {
    let author = match author {
        Some(author) => parse_author(&author)?,
        None => {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "generate needs --author 'Name <email>' for the entry header",
            ));
        }
    };
    let target = resolve_target(target_project, target_package)?;
    let local = load_local(dir)?;
    let target_snapshot = match &target {
        Some((project, package)) => fetch_target(debug, project, package)?,
        None => None,
    };
    let (news, news_error, _) = lookup_news(dir, &local);
    let generation = generate(&local, target_snapshot.as_ref(), &author, news.as_ref())
        .map_err(|err| GeectlError::new(ErrorKind::InternalError, err))?;
    let spec_name = local
        .spec_names()
        .first()
        .copied()
        .unwrap_or("")
        .to_string();
    let changes_name = format!(
        "{}.changes",
        spec_name.strip_suffix(".spec").unwrap_or(&spec_name)
    );
    let mut payload = json!({
        "dir": dir.display().to_string(),
        "target_project": target.as_ref().map(|(project, _)| project),
        "target_package": target.as_ref().map(|(_, package)| package),
        "entry": generation.entry,
        "needs_detail": generation.needs_detail,
        "news_found": generation.news_found,
        "slots": generation.slots,
        "news_error": news_error,
    });
    if !write {
        return Ok((payload, generation.entry));
    }
    // An incomplete draft is never written: a needs-detail flag, or
    // any unfilled TODO slot. The draft goes to stdout only, with
    // the refusal naming what is missing.
    let mut reasons: Vec<String> = Vec::new();
    if generation.needs_detail {
        reasons.push(
            "upstream news exists for the new version and the draft has no substantive line beyond the version line"
                .to_string(),
        );
    }
    if generation.slots > 0 {
        reasons.push(format!(
            "the draft still carries {} unfilled TODO slot(s)",
            generation.slots
        ));
    }
    if !reasons.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::ValidationFailed,
            format!(
                "refusing to write an incomplete entry (V9 needs-detail): {} — fill in the TODO slots and distill the highlights first:\n{}",
                reasons.join("; "),
                generation.entry
            ),
        ));
    }
    let changes_path = dir.join(&changes_name);
    refuse_symlink_destination(&changes_path)?;
    let old_content = std::fs::read_to_string(&changes_path).unwrap_or_default();
    let new_content = prepend_entry(&old_content, &generation.entry)
        .map_err(|err| GeectlError::new(ErrorKind::InternalError, err))?;
    std::fs::write(&changes_path, &new_content).map_err(|err| {
        GeectlError::new(
            ErrorKind::InternalError,
            format!("cannot write {}: {err}", changes_path.display()),
        )
    })?;
    payload["written"] = json!(changes_path.display().to_string());
    Ok((payload, generation.entry))
}
