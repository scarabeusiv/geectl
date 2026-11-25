//! `changelog` CLI commands (`geectl changelog ...`).
//!
//! Pre-file validation/generation for `.changes` entries. The
//! commands work on a package directory on disk, optionally compared
//! against a target project/package on the instance (public reads,
//! no credentials). `validate` runs the V1–V12 rules; `generate`
//! emits the diff-derived skeleton entry as a draft for a human to
//! complete. Neither command files anything. `commit` runs the
//! changelog gate on the staged `.changes` entries, before anything
//! is filed.
//!
//! Coordinate header (design rule 1): `validate` text opens with
//! `dir: <dir>` (plus `target: <project>/<package>` when given).
//! `generate` is a content command (design rule 4): the text IS the
//! entry, verbatim. On findings, `validate` exits 24
//! (`validation_failed`) and the error context lists one
//! `file:line: V#: message` line per finding, plus the skipped
//! rules — the same shape the skill's `changes-lint.sh` prints.

use crate::changelog::changes::normalize_changes_content;
use crate::changelog::generate::{generate, message_bullets, prepend_entry, version_is_new};
use crate::changelog::news::{extract_news_section, find_tarball, NewsSection};
use crate::changelog::spec::parse_spec;
use crate::changelog::validate::{validate, PackageSnapshot, ValidateOptions};
use crate::debug::Debug;
use crate::error::{ErrorKind, GeectlError};
use crate::http::{Timeout, UreqClient};
use crate::obs::ObsClient;
use clap::Subcommand;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The `changelog` command group: `.changes` validation and generation.
#[derive(Debug, Subcommand)]
pub enum ChangelogCommand {
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
        /// form — exactly what a bare `changelog` invocation writes.
        #[arg(long)]
        author: Option<String>,
        /// Custom message bullets for the entry, one bullet per
        /// `-m`. Repeatable: `-m "Fix the crash" -m "Add the
        /// subcommand"`. A message may span lines — embedded
        /// newlines become indented continuation lines of that
        /// bullet, and a blank line inside a message is a paragraph
        /// break within it. A leading `- `, `* ` or `+ ` marker is
        /// stripped, never double-bulleted. The message is the
        /// human detail: it replaces the TODO summary slot, so
        /// `--write` no longer refuses the draft for the missing
        /// message; other TODO slots still refuse.
        #[arg(short, long)]
        message: Vec<String>,
        /// Prepend the draft into the package's `.changes` file
        /// instead of printing it. The whole file is normalized
        /// on write: trailing ASCII whitespace stripped, CRLF→LF,
        /// legacy encodings transcoded to UTF-8, a BOM stripped.
        /// The write is atomic and refuses a symlink destination,
        /// an unreadable file, or bytes no known encoding decodes.
        /// A file decoded from a guessed single-byte encoding is
        /// transcoded on the guess instead — a wrong guess cannot
        /// be detected — and the write warns about it, naming the
        /// encoding.
        #[arg(long)]
        write: bool,
    },
}

pub fn run_changelog(
    command: ChangelogCommand,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    // `changelog` performs no OBS mutation: `--live` is ignored, like
    // `cat`. `--write` is the explicit local-write flag.
    let _ = live;
    match command {
        ChangelogCommand::Validate {
            dir,
            target_project,
            target_package,
            entries,
        } => run_validate(&dir, target_project, target_package, entries, debug),
        ChangelogCommand::Generate {
            dir,
            target_project,
            target_package,
            author,
            message,
            write,
        } => run_generate(
            &dir,
            target_project,
            target_package,
            author,
            message,
            write,
            debug,
        ),
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
/// every `.spec` file and the normalized text of every `.changes`
/// file (fetched as bytes: a legacy-encoded target is not valid
/// UTF-8, and `cat` would error on it instead of reaching V11). A
/// target that does not exist is `None` (a new package), never an
/// error; any other failure propagates — a failed lookup must never
/// read as good news.
fn fetch_target(
    debug: &Debug,
    project: &str,
    package: &str,
) -> Result<Option<PackageSnapshot>, GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let (base, _instances) = crate::cli_remote::obs_endpoint()?;
    let client =
        ObsClient::with_base_and_credentials(&http, &base, crate::auth::Credentials::None)?
            .with_debug(debug.clone());
    let files = match client.list_files(project, package, None, false) {
        Ok(files) => files,
        Err(err) if err.kind == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let mut snapshot = PackageSnapshot::default();
    for file in files {
        if file.name.ends_with(".changes") {
            let bytes = client.cat_bytes(project, package, &file.name, None)?;
            snapshot
                .files
                .insert(file.name, normalize_changes_content(&bytes));
        } else if file.name.ends_with(".spec") {
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
        skipped.push(crate::changelog::Skipped {
            rule: crate::changelog::Rule::V9,
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

/// Write a generated entry into a `.changes` file: the preserved
/// entries are normalized on write, then the entry is prepended.
/// The write is atomic (a temp file in the same directory, then a
/// rename), so a crash or a full disk never leaves a truncated
/// `.changes` behind.
/// Returns the single-byte-guess warning, when the old content
/// was transcoded from a guessed single-byte encoding: the caller
/// prints it visibly (stderr), so a wrong guess is noticed instead
/// of silently rewriting the file.
fn write_generated_entry(
    changes_path: &Path,
    entry: &str,
    debug: &Debug,
) -> Result<Option<String>, GeectlError> {
    refuse_symlink_destination(changes_path)?;
    // Only a missing file reads as empty: any other read error
    // (permission denied, EISDIR, I/O) propagates instead of
    // silently dropping the preserved entries.
    let old_bytes = match std::fs::read(changes_path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(err) => {
            return Err(GeectlError::new(
                ErrorKind::InternalError,
                format!("cannot read {}: {err}", changes_path.display()),
            ));
        }
    };
    let prepended = prepend_entry(&old_bytes, entry).map_err(|err| {
        GeectlError::new(
            ErrorKind::InternalError,
            format!("cannot write {}: {err}", changes_path.display()),
        )
    })?;
    // Name the guessed encoding on the debug channel, so a wrong
    // guess is visible instead of silent: the normalized text is
    // only as good as the guess.
    if let Some(encoding) = prepended.source_encoding {
        debug.emit(&format!(
            "changelog: normalized {} from {encoding} to UTF-8",
            changes_path.display()
        ));
    }
    // A single-byte guess can never be wrong loudly: the write
    // keeps the transcode, but the guess is warned about visibly
    // (not only on the debug channel), naming the encoding.
    let warning = if prepended.guessed_single_byte {
        prepended.source_encoding.map(|encoding| {
            format!(
                "warning: {} was transcoded from {encoding} to UTF-8; a single-byte guess cannot be verified, so check the preserved entries",
                changes_path.display()
            )
        })
    } else {
        None
    };
    write_file_atomically(changes_path, prepended.content.as_bytes())?;
    Ok(warning)
}

/// Write `content` to `path` atomically: a temp file in the same
/// directory, then a rename over the destination. The rename
/// replaces the destination itself — even if the path became a
/// symlink after the refusal check, the link is replaced, never
/// followed. The temp file is hidden (dot-prefixed) so a leftover
/// is never picked up as a package file.
fn write_file_atomically(path: &Path, content: &[u8]) -> Result<(), GeectlError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    // Pid + counter, so two writes in one process never share a name.
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temp_path = parent.join(format!(
        ".{file_name}.tmp.{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let write = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temp_path)?;
        // Keep the destination's permissions: a fresh temp file is
        // 0666 & ~umask, which would silently widen a 0600
        // `.changes` to 0644 on rename.
        if let Ok(metadata) = std::fs::metadata(path) {
            std::fs::set_permissions(&temp_path, metadata.permissions())?;
        }
        file.write_all(content)?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(err) = write {
        let _ = std::fs::remove_file(&temp_path);
        return Err(GeectlError::new(
            ErrorKind::InternalError,
            format!("cannot write {}: {err}", path.display()),
        ));
    }
    if let Err(err) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(GeectlError::new(
            ErrorKind::InternalError,
            format!("cannot write {}: {err}", path.display()),
        ));
    }
    // Sync the directory so the rename itself is durable, not just
    // the file's contents.
    sync_dir_after_rename(parent);
    Ok(())
}

/// Sync `dir` after a rename so the rename itself is durable.
/// Unix-only: opening a directory is not portable. Best-effort:
/// the rename already happened, so a failure here must not report
/// the write as failed.
#[cfg(unix)]
fn sync_dir_after_rename(dir: &Path) {
    if let Ok(dir) = std::fs::File::open(dir) {
        let _ = dir.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir_after_rename(_dir: &Path) {}

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
    message: Vec<String>,
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
    // A message must carry text: blank lines or a bare bullet
    // marker alone render to no bullet at all. Leading and
    // trailing blank lines are trimmed first, so a message that
    // is only whitespace around its text still renders as one
    // bullet.
    let messages: Vec<String> = message
        .iter()
        .map(|message| message.trim().to_string())
        .collect();
    for message in &messages {
        if message_bullets(message)
            .iter()
            .all(|line| line.trim().is_empty())
        {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "--message must carry text, not just blank lines or a bare bullet marker",
            ));
        }
    }
    let target = resolve_target(target_project, target_package)?;
    let local = load_local(dir)?;
    let target_snapshot = match &target {
        Some((project, package)) => fetch_target(debug, project, package)?,
        None => None,
    };
    // NEWS is only looked up for a new version: when the version
    // is unchanged, the upstream section for it was already
    // distilled when the version landed, so there is nothing to
    // extract.
    let (news, news_error, _) = if version_is_new(&local, target_snapshot.as_ref()) {
        lookup_news(dir, &local)
    } else {
        (None, None, false)
    };
    let generation = generate(
        &local,
        target_snapshot.as_ref(),
        &author,
        news.as_ref(),
        &messages,
    )?;
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
    if let Some(warning) = write_generated_entry(&changes_path, &generation.entry, debug)? {
        eprintln!("{warning}");
    }
    payload["written"] = json!(changes_path.display().to_string());
    Ok((payload, generation.entry))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    /// One generated entry, shared by the write tests.
    const ENTRY: &str = "-------------------------------------------------------------------\nMon Nov 23 00:00:00 UTC 2026 - Jane Packager <jane@example.com>\n\n- Update to 2.1\n\n";

    #[test]
    #[cfg(unix)]
    fn write_destination_refuses_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.changes");
        std::fs::write(&target, "original\n").unwrap();
        let link = dir.path().join("foo.changes");
        symlink(&target, &link).unwrap();
        let err = refuse_symlink_destination(&link).expect_err("symlink is refused");
        assert_eq!(err.kind, ErrorKind::ValidationFailed);
        assert!(err.context.contains("is a symlink"));
        // A regular file and a missing file are both fine.
        assert!(refuse_symlink_destination(&target).is_ok());
        assert!(refuse_symlink_destination(&dir.path().join("missing.changes")).is_ok());
    }

    #[test]
    fn write_generated_entry_normalizes_preserved_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("foo.changes");
        // A long-lived file: a UTF-8 entry above a Latin-1 entry
        // with trailing spaces.
        let mut old = "-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Tomás Chvátal <tomas.chvatal@gmail.com>\n\n- Update to 2.0   \n\n-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Andr".as_bytes().to_vec();
        old.extend_from_slice(b"\xe9 Martin <andre@example.com>\n\n- Fix the th\xe9\n");
        std::fs::write(&path, &old).unwrap();
        let entry = "-------------------------------------------------------------------\nMon Nov 23 00:00:00 UTC 2026 - Jane Packager <jane@example.com>\n\n- Update to 2.1\n\n";
        let warning =
            write_generated_entry(&path, entry, &Debug::disabled()).expect("write succeeds");
        assert!(warning.is_some_and(|warning| warning.contains("windows-1252")));
        let written = std::fs::read(&path).unwrap();
        let mut expected = entry.as_bytes().to_vec();
        expected.extend_from_slice("-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Tomás Chvátal <tomas.chvatal@gmail.com>\n\n- Update to 2.0\n\n-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - André Martin <andre@example.com>\n\n- Fix the thé\n".as_bytes());
        assert_eq!(written, expected);
    }

    #[test]
    #[cfg(unix)]
    fn write_generated_entry_refuses_a_symlink_and_leaves_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.changes");
        std::fs::write(&target, "original\n").unwrap();
        let link = dir.path().join("foo.changes");
        symlink(&target, &link).unwrap();
        let err = write_generated_entry(&link, ENTRY, &Debug::disabled())
            .expect_err("symlink is refused");
        assert_eq!(err.kind, ErrorKind::ValidationFailed);
        assert!(err.context.contains("is a symlink"));
        assert_eq!(std::fs::read(&target).unwrap(), b"original\n");
    }

    #[test]
    #[cfg(unix)]
    fn write_generated_entry_propagates_an_unreadable_file() {
        // Runs as the invoking user; root bypasses permissions.
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("foo.changes");
        std::fs::write(&path, "original\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let err = write_generated_entry(&path, ENTRY, &Debug::disabled())
            .expect_err("unreadable file errors");
        assert!(err.context.contains("cannot read"));
        // Restore permissions so cleanup can remove the file.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"original\n");
    }

    #[test]
    #[cfg(unix)]
    fn write_generated_entry_keeps_the_destination_permissions() {
        // A 0600 `.changes` must stay 0600: the temp file is
        // created 0666 & ~umask and the rename must not widen it.
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("foo.changes");
        std::fs::write(&path, "original\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let warning =
            write_generated_entry(&path, ENTRY, &Debug::disabled()).expect("write succeeds");
        assert!(warning.is_none());
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn write_generated_entry_warns_on_a_single_byte_guess() {
        // PROBE1 shape end to end: a mixed file with one lone
        // corrupt byte still writes — the guess cannot be refused —
        // but the write warns, naming windows-1252, instead of
        // staying silent. The corrupt byte was transcoded (as
        // U+201A), not refused.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("foo.changes");
        let mut old = "-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Tom\u{e1}s Chv\u{e1}tal <tomas.chvatal@gmail.com>\n\n- Update to 2.0\n\n-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Andr".as_bytes().to_vec();
        old.extend_from_slice(b"\xe9 Martin <andre@example.com>\n\n- Fix the th\xe9\n");
        old.extend_from_slice(b"\x82\n");
        std::fs::write(&path, &old).unwrap();
        let warning =
            write_generated_entry(&path, ENTRY, &Debug::disabled()).expect("write succeeds");
        let warning = warning.expect("a single-byte guess warns");
        assert!(warning.contains("windows-1252"), "{warning}");
        let written = std::fs::read(&path).unwrap();
        assert!(String::from_utf8_lossy(&written).contains('\u{201A}'));
    }

    #[test]
    fn write_generated_entry_refuses_undecodable_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("foo.changes");
        // Truncated UTF-16: the whole-input decode hits the lone byte.
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend("ab\n".encode_utf16().flat_map(|unit| unit.to_le_bytes()));
        bytes.push(0x61);
        std::fs::write(&path, &bytes).unwrap();
        let err = write_generated_entry(&path, ENTRY, &Debug::disabled())
            .expect_err("undecodable bytes are refused");
        assert!(err.context.contains("no known encoding decodes"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}
