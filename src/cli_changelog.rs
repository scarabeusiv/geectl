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
use crate::config::{Config, Identity, VC_MAILADDR_VAR, VC_REALNAME_VAR};
use crate::debug::Debug;
use crate::error::{ErrorKind, GeectlError};
use crate::http::{Timeout, UreqClient};
use crate::obs::{ObsClient, Person};
use clap::Subcommand;
use serde_json::{json, Value};
use std::collections::HashMap;
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
    /// Print the resolved `.changes` entry author in `Name <email>`
    /// form — the same value a bare `changelog generate` would
    /// write. The author resolves through the identity chain
    /// (#405): config `[identity]`, then `VC_REALNAME` /
    /// `VC_MAILADDR`, then the OBS account, then git identity; the
    /// command refuses when none yields an identity.
    Identity {
        /// The package directory (default: the current directory).
        /// The git level reads identity from here.
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
}

pub fn run_changelog(
    command: ChangelogCommand,
    debug: &Debug,
    live: bool,
    instance: Option<&str>,
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
        } => run_validate(
            &dir,
            target_project,
            target_package,
            entries,
            debug,
            instance,
        ),
        ChangelogCommand::Generate {
            dir,
            target_project,
            target_package,
            author,
            message,
            write,
        } => {
            // `--author` wins over the whole chain; without it the
            // author resolves through the identity chain (#405).
            let author = match author {
                Some(author) => Some(author),
                None => Some(resolve_changelog_identity(
                    &crate::config::load_config()?,
                    &crate::auth::snapshot_env(&[VC_REALNAME_VAR, VC_MAILADDR_VAR])?,
                    &dir,
                    || obs_identity(debug, instance),
                )?),
            };
            run_generate(
                &dir,
                target_project,
                target_package,
                author,
                message,
                write,
                debug,
                instance,
            )
        }
        ChangelogCommand::Identity { dir } => run_identity(&dir, debug, instance),
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
    instance: Option<&str>,
) -> Result<Option<PackageSnapshot>, GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let (base, _instances) = crate::cli_remote::obs_endpoint(instance)?;
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
/// no section) — the generate path reports it in its payload. V9 no
/// longer distinguishes the no-news cases: the finding message only
/// says whether news was found.
fn lookup_news(dir: &Path, local: &PackageSnapshot) -> (Option<NewsSection>, Option<String>) {
    let spec_names = local.spec_names();
    let Some(spec_name) = spec_names.first() else {
        return (None, None);
    };
    let spec = parse_spec(&local.files[*spec_name]);
    let Some(version) = spec.version else {
        return (None, None);
    };
    let source0 = spec.sources.first().map(String::as_str);
    let Some(tarball) = find_tarball(dir, &version, source0) else {
        return (None, None);
    };
    match extract_news_section(&tarball, &version) {
        Ok(section) => (section, None),
        Err(err) => (None, Some(err.to_string())),
    }
}

fn run_validate(
    dir: &Path,
    target_project: Option<String>,
    target_package: Option<String>,
    entries: usize,
    debug: &Debug,
    instance: Option<&str>,
) -> Result<(Value, String), GeectlError> {
    let target = resolve_target(target_project, target_package)?;
    let local = load_local(dir)?;
    let target_snapshot = match &target {
        Some((project, package)) => fetch_target(debug, project, package, instance)?,
        None => None,
    };
    let (news, news_error) = lookup_news(dir, &local);
    // The lookup failure is not a V9 skip: V9 runs whether or not
    // news was found. Name it on the debug channel so a corrupt or
    // undecodable tarball is visible instead of silent.
    if let Some(reason) = news_error {
        debug.emit(&format!(
            "changelog: upstream NEWS lookup failed ({reason})"
        ));
    }
    let options = ValidateOptions {
        entries,
        news_section: news.as_ref().map(|section| section.text.as_str()),
        ..Default::default()
    };
    let report = validate(&local, target_snapshot.as_ref(), &options);
    let skipped = report.skipped.clone();
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

/// Resolve the author for a generated `.changes` entry, in the
/// order Tom decided (#405): the config `[identity]` section,
/// then the `VC_REALNAME` / `VC_MAILADDR` environment, then the
/// OBS account's real name and email, then git identity
/// (`user.name` / `user.email`) — and refuse when none of those
/// yields an identity, rather than writing an entry with no
/// author. The OBS lookup runs only when the config and env
/// levels yield nothing, so a half-configured OBS credential
/// never breaks a generate the config or env already identifies;
/// a failed OBS lookup propagates instead of reading as no
/// identity. `env` is a snapshot (see [`snapshot_env`]).
pub(crate) fn resolve_changelog_identity(
    config: &Config,
    env: &HashMap<String, String>,
    dir: &Path,
    obs_lookup: impl FnOnce() -> Result<Option<Identity>, GeectlError>,
) -> Result<String, GeectlError> {
    if let Some(identity) = &config.identity {
        return Ok(identity.format());
    }
    if let Some(identity) = crate::config::identity_from_env_map(env)? {
        return Ok(identity.format());
    }
    if let Some(identity) = obs_lookup()? {
        return Ok(identity.format());
    }
    if let Some(identity) = crate::working_copy::git_identity(dir)? {
        return Ok(identity.format());
    }
    Err(GeectlError::new(
        ErrorKind::InvalidInput,
        "no changelog identity found: set [identity] name and email in the config file, VC_REALNAME and VC_MAILADDR in the environment, or pass --author 'Name <email>'",
    ))
}

/// The OBS-account level of the identity chain: the authenticated
/// user's real name and email from `whois`. No username in the
/// credentials (API-key auth carries none) yields nothing — the
/// chain falls through to git.
fn obs_identity(debug: &Debug, instance: Option<&str>) -> Result<Option<Identity>, GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let (base, instances) = crate::cli_remote::obs_endpoint(instance)?;
    let credentials = crate::auth::EnvObsAuth::credentials_for_url(&base, &base, &instances)?;
    let client =
        ObsClient::with_base_and_credentials(&http, &base, credentials)?.with_debug(debug.clone());
    identity_from_obs_client(&client)
}

fn identity_from_obs_client(client: &ObsClient) -> Result<Option<Identity>, GeectlError> {
    let Some(login) = client.credentials().username() else {
        return Ok(None);
    };
    let person = client.whois(login)?;
    Ok(identity_from_person(&person))
}

/// The OBS `whois` person as a changelog identity: real name and
/// email, both present and well-shaped, or nothing. A person
/// carrying only one of the two does not identify an author.
fn identity_from_person(person: &Person) -> Option<Identity> {
    let name = person
        .realname
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let email = person
        .email
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    if !crate::config::identity_shape_ok(name, email) {
        return None;
    }
    Some(Identity {
        name: name.to_string(),
        email: email.to_string(),
    })
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

/// Print the resolved `.changes` entry author: the same value a
/// bare `changelog generate` writes, from the same identity chain
/// (#405). The text is the author line itself, so a caller can
/// capture it directly; the payload carries the author and the
/// directory the git level read from.
fn run_identity(
    dir: &Path,
    debug: &Debug,
    instance: Option<&str>,
) -> Result<(Value, String), GeectlError> {
    identity_output(
        &crate::config::load_config()?,
        &crate::auth::snapshot_env(&[VC_REALNAME_VAR, VC_MAILADDR_VAR])?,
        dir,
        || obs_identity(debug, instance),
    )
}

/// The identity leaf with its inputs injected: the resolution order
/// is `resolve_changelog_identity`'s, the leaf only shapes the
/// output.
fn identity_output(
    config: &Config,
    env: &HashMap<String, String>,
    dir: &Path,
    obs_lookup: impl FnOnce() -> Result<Option<Identity>, GeectlError>,
) -> Result<(Value, String), GeectlError> {
    let author = resolve_changelog_identity(config, env, dir, obs_lookup)?;
    let payload = json!({
        "dir": dir.display().to_string(),
        "author": author,
    });
    Ok((payload, author))
}

#[allow(clippy::too_many_arguments)]
fn run_generate(
    dir: &Path,
    target_project: Option<String>,
    target_package: Option<String>,
    author: Option<String>,
    message: Vec<String>,
    write: bool,
    debug: &Debug,
    instance: Option<&str>,
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
        Some((project, package)) => fetch_target(debug, project, package, instance)?,
        None => None,
    };
    // NEWS is only looked up for a new version: when the version
    // is unchanged, the upstream section for it was already
    // distilled when the version landed, so there is nothing to
    // extract.
    let (news, news_error) = if version_is_new(&local, target_snapshot.as_ref()) {
        lookup_news(dir, &local)
    } else {
        (None, None)
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

    fn identity_config() -> Config {
        Config {
            identity: Some(Identity {
                name: "Config Author".to_string(),
                email: "config@example.com".to_string(),
            }),
            ..Default::default()
        }
    }

    fn env_identity() -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert(VC_REALNAME_VAR.to_string(), "Env Author".to_string());
        env.insert(VC_MAILADDR_VAR.to_string(), "env@example.com".to_string());
        env
    }

    fn obs_identity() -> Identity {
        Identity {
            name: "Obs Author".to_string(),
            email: "obs@example.com".to_string(),
        }
    }

    #[test]
    fn resolve_identity_config_wins_and_obs_never_runs() {
        let dir = tempfile::tempdir().unwrap();
        let obs_called = std::cell::Cell::new(false);
        let author =
            resolve_changelog_identity(&identity_config(), &env_identity(), dir.path(), || {
                obs_called.set(true);
                Ok(Some(obs_identity()))
            })
            .unwrap();
        assert_eq!(author, "Config Author <config@example.com>");
        assert!(
            !obs_called.get(),
            "the OBS lookup must not run when the config identifies"
        );
    }

    #[test]
    fn resolve_identity_env_wins_over_obs() {
        let dir = tempfile::tempdir().unwrap();
        let obs_called = std::cell::Cell::new(false);
        let author =
            resolve_changelog_identity(&Config::default(), &env_identity(), dir.path(), || {
                obs_called.set(true);
                Ok(Some(obs_identity()))
            })
            .unwrap();
        assert_eq!(author, "Env Author <env@example.com>");
        assert!(
            !obs_called.get(),
            "the OBS lookup must not run when the env identifies"
        );
    }

    #[test]
    fn resolve_identity_env_partial_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = HashMap::new();
        env.insert(VC_REALNAME_VAR.to_string(), "Env Author".to_string());
        let err = resolve_changelog_identity(&Config::default(), &env, dir.path(), || {
            Ok(Some(obs_identity()))
        })
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::ConfigError);
        assert!(err.context.contains(VC_MAILADDR_VAR), "{}", err.context);
    }

    #[test]
    fn resolve_identity_obs_used_when_config_and_env_empty() {
        let dir = tempfile::tempdir().unwrap();
        let author =
            resolve_changelog_identity(&Config::default(), &HashMap::new(), dir.path(), || {
                Ok(Some(obs_identity()))
            })
            .unwrap();
        assert_eq!(author, "Obs Author <obs@example.com>");
    }

    #[test]
    fn resolve_identity_obs_error_propagates() {
        let dir = tempfile::tempdir().unwrap();
        let err =
            resolve_changelog_identity(&Config::default(), &HashMap::new(), dir.path(), || {
                Err(GeectlError::new(ErrorKind::InvalidInput, "whois failed"))
            })
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidInput);
        assert!(err.context.contains("whois failed"), "{}", err.context);
    }

    #[test]
    fn resolve_identity_refuses_when_nothing_found() {
        // A missing dir makes the git level read as no identity
        // without touching the developer's global git config, so
        // the refusal is deterministic in any checkout.
        let err = resolve_changelog_identity(
            &Config::default(),
            &HashMap::new(),
            Path::new("/definitely/not/a/real/dir"),
            || Ok(None),
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidInput);
        assert!(
            err.context.contains("no changelog identity"),
            "{}",
            err.context
        );
        assert!(err.context.contains("[identity]"), "{}", err.context);
        assert!(err.context.contains("VC_REALNAME"), "{}", err.context);
        assert!(err.context.contains("--author"), "{}", err.context);
    }

    fn test_person(realname: Option<&str>, email: Option<&str>) -> Person {
        Person {
            login: "jane".to_string(),
            email: email.map(str::to_string),
            realname: realname.map(str::to_string),
            state: "confirmed".to_string(),
        }
    }

    #[test]
    fn identity_from_person_needs_both() {
        assert_eq!(
            identity_from_person(&test_person(
                Some("Jane Packager"),
                Some("jane@example.com")
            )),
            Some(Identity {
                name: "Jane Packager".to_string(),
                email: "jane@example.com".to_string(),
            })
        );
        assert_eq!(
            identity_from_person(&test_person(Some("Jane Packager"), None)),
            None
        );
        assert_eq!(
            identity_from_person(&test_person(None, Some("jane@example.com"))),
            None
        );
        assert_eq!(identity_from_person(&test_person(None, None)), None);
    }

    #[test]
    fn identity_from_person_rejects_misshapen() {
        assert_eq!(
            identity_from_person(&test_person(Some("Jane Packager"), Some("not-an-email"))),
            None
        );
        assert_eq!(
            identity_from_person(&test_person(Some("  "), Some("jane@example.com"))),
            None
        );
    }

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

    #[test]
    fn run_validate_logs_a_failed_news_lookup_on_the_debug_channel() {
        // A corrupt tarball reads as a plain V9 finding; the lookup
        // failure itself is named on the debug channel, not silent.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("foo.spec"),
            "Name:           foo\nVersion:        2.0\nLicense:        GPL-3.0-only\nSource0:        foo-2.0.tar.gz\n%changelog\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("foo.changes"),
            "-------------------------------------------------------------------\nMon Nov 23 00:00:00 UTC 2026 - Jane Packager <jane@example.com>\n\n- Update to 2.0\n\n-------------------------------------------------------------------\nMon Nov 16 00:00:00 UTC 2026 - Jane Packager <jane@example.com>\n\n- Initial package\n\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("foo-2.0.tar.gz"), b"not a tarball at all").unwrap();
        let (debug, lines) = Debug::captured(true, false);
        let err = run_validate(dir.path(), None, None, 1, &debug, None)
            .expect_err("a bare update is a V9 finding");
        assert!(err.context.contains("finding(s)"), "{}", err.context);
        let lines = lines.lock().unwrap();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("upstream NEWS lookup failed")),
            "debug lines: {lines:?}"
        );
    }

    #[test]
    fn identity_prints_the_config_author() {
        let dir = tempfile::tempdir().unwrap();
        let (payload, text) =
            identity_output(&identity_config(), &env_identity(), dir.path(), || {
                Ok(Some(obs_identity()))
            })
            .unwrap();
        assert_eq!(text, "Config Author <config@example.com>");
        assert_eq!(payload["author"], "Config Author <config@example.com>");
        assert_eq!(payload["dir"], dir.path().display().to_string());
    }

    #[test]
    fn identity_env_wins_when_config_empty() {
        let dir = tempfile::tempdir().unwrap();
        let (payload, text) =
            identity_output(&Config::default(), &env_identity(), dir.path(), || {
                Ok(Some(obs_identity()))
            })
            .unwrap();
        assert_eq!(text, "Env Author <env@example.com>");
        assert_eq!(payload["author"], "Env Author <env@example.com>");
    }

    #[test]
    fn identity_obs_used_when_config_and_env_empty() {
        let dir = tempfile::tempdir().unwrap();
        let (_, text) = identity_output(&Config::default(), &HashMap::new(), dir.path(), || {
            Ok(Some(obs_identity()))
        })
        .unwrap();
        assert_eq!(text, "Obs Author <obs@example.com>");
    }

    #[test]
    fn identity_refuses_when_nothing_found() {
        // A missing dir makes the git level read as no identity
        // without touching the developer's global git config, so
        // the refusal is deterministic in any checkout.
        let err = identity_output(
            &Config::default(),
            &HashMap::new(),
            Path::new("/definitely/not/a/real/dir"),
            || Ok(None),
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidInput);
        assert!(
            err.context.contains("no changelog identity"),
            "{}",
            err.context
        );
    }
}
