//! The generator: the diff-derived skeleton entry (G1–G6) plus the
//! extracted upstream NEWS section (G7, extraction only —
//! distillation stays human).
//!
//! The output is a draft for a human to complete, never a
//! submission: what the diff cannot supply arrives as explicit
//! `TODO` slots (`reason:`, `upstream highlights:`), and the entry
//! is flagged incomplete (V9) when upstream news exists for the new
//! version and the draft has no substantive line beyond the version
//! line. `generate` never files anything; `--write` prepends the
//! draft into the `.changes` file, and refuses an incomplete draft.

use crate::changelog::changes::{
    civil_from_days, normalize_changes_content_checked, parse_changes, separator_line,
    weekday_name, Header,
};
use crate::changelog::news::NewsSection;
use crate::changelog::spec::parse_spec;
use crate::changelog::validate::{is_mechanical_bullet, PackageSnapshot};
use crate::error::{ErrorKind, GeectlError};
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

/// The generated draft.
#[derive(Debug, Clone)]
pub struct Generation {
    /// The full entry text: separator, header, blank line, body,
    /// trailing blank line — ready to prepend.
    pub entry: String,
    /// V9 applies: upstream news exists and the draft has no
    /// substantive line beyond the version line. `--write` refuses
    /// an incomplete draft.
    pub needs_detail: bool,
    /// A NEWS section was found for the new version.
    pub news_found: bool,
    /// The number of `TODO` slots in the draft.
    pub slots: usize,
}

/// Build the current UTC header for `author` (`Name <email>`).
pub fn now_header(author: &str) -> Header {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    let days = secs.div_euclid(86400);
    let (year, month, day) = civil_from_days(days);
    let rest = secs.rem_euclid(86400);
    Header {
        weekday: weekday_name(days).to_string(),
        month,
        day,
        hour: (rest / 3600) as u32,
        minute: ((rest % 3600) / 60) as u32,
        second: (rest % 60) as u32,
        year,
        author: author.to_string(),
        timestamp: secs,
    }
}

/// Wrap `text` at 67 columns for a bullet with the given first-line
/// prefix and continuation prefix. A word too long for the width
/// stays on a line of its own.
fn wrap_bullet(text: &str, first_prefix: &str, cont_prefix: &str) -> Vec<String> {
    let width = 67usize;
    let mut lines = Vec::new();
    let mut current = first_prefix.to_string();
    for word in text.split_whitespace() {
        let candidate = if current == first_prefix {
            format!("{current}{word}")
        } else {
            format!("{current} {word}")
        };
        if candidate.chars().count() > width {
            if current != first_prefix {
                lines.push(current.trim_end().to_string());
                current = format!("{cont_prefix}{word}");
            } else {
                // A single word too long for the width: it stays on
                // a line of its own (the V11 exception).
                lines.push(candidate);
                current = cont_prefix.to_string();
            }
        } else {
            current = candidate;
        }
    }
    if !current.trim().is_empty() || lines.is_empty() {
        lines.push(current.trim_end().to_string());
    }
    lines
}

/// Strip control characters from a `-m` message, keeping newline
/// and tab: the same hygiene the service text path applies via
/// `strip_terminal_controls`, except a tab survives — inside a
/// message it is whitespace the wrapper treats as a space, never
/// an escape. Applied in `message_bullets` so both the CLI and the
/// public `generate` entry point render the same bullet.
fn strip_message_controls(text: &str) -> String {
    text.chars()
        .filter(|c| {
            *c == '\n'
                || *c == '\t'
                || (!c.is_control()
                    && !matches!(
                        *c,
                        '\u{061c}'
                            | '\u{200e}'..='\u{200f}'
                            | '\u{202a}'..='\u{202e}'
                            | '\u{2066}'..='\u{2069}'
                            | '\u{feff}'
                    ))
        })
        .collect()
}

/// Render one `-m` message into entry lines: one bullet, with
/// embedded newlines as indented continuation lines of that bullet
/// and blank lines as paragraph breaks inside it. A leading bullet
/// marker on any line (`- `, `* `, `+ `, or the bare marker alone)
/// is stripped first — the entry renders with `- ` throughout —
/// following the NEWS-line precedent below, and because V11
/// forbids `* ` at the top level and `+ ` anywhere.
/// Wrapped at 67 columns like every other bullet. A line left
/// empty by the strip is skipped, never wrapped into a bare
/// marker.
pub fn message_bullets(text: &str) -> Vec<String> {
    let text = strip_message_controls(text);
    let mut lines = Vec::new();
    let mut first = true;
    for physical in text.lines() {
        let trimmed = physical.trim();
        if trimmed.is_empty() {
            lines.push(String::new());
            continue;
        }
        // A leading bullet marker (`- `, `* `, `+ `, or the bare
        // marker alone — `trim()` above already took the space)
        // is stripped, never double-bulleted.
        let clean = if trimmed == "-" || trimmed == "*" || trimmed == "+" {
            ""
        } else {
            trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
                .or_else(|| trimmed.strip_prefix("+ "))
                .unwrap_or(trimmed)
        };
        if clean.is_empty() {
            continue;
        }
        if first {
            lines.extend(wrap_bullet(clean, "- ", "  "));
            first = false;
        } else {
            lines.extend(wrap_bullet(clean, "  ", "  "));
        }
    }
    // Leading/trailing blank lines carry nothing — the CLI trims
    // each `-m`, but `generate` is public API, so the trim lives
    // here and both entry points render the same bullet. Interior
    // blank lines stay: paragraph breaks inside the bullet.
    while lines.first().is_some_and(|line| line.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines
}

/// The version an `Update to` bullet names, if the bullet is one:
/// `- Update to 2.0` / `- Updated to 2.0`, the marker matched
/// case-insensitively, the version kept verbatim — the bullet is
/// human text, so `2.0-RC1` stays `2.0-RC1` and compares equal to
/// the spec's version. (Lowercasing the whole bullet first returned
/// `2.0-rc1`, which read as a new version and demanded NEWS for a
/// hotfix.) Mirrors the update-bullet shape V9 recognises.
fn update_bullet_version(text: &str) -> Option<String> {
    let rest = ["- update to ", "- updated to "]
        .iter()
        .find_map(|marker| {
            text.get(..marker.len())
                .filter(|head| head.eq_ignore_ascii_case(marker))
                .map(|_| &text[marker.len()..])
        })?;
    let mut words = rest.split_whitespace();
    let mut version = words.next()?;
    if version.eq_ignore_ascii_case("version") {
        version = words.next()?;
    }
    Some(
        version
            .trim_end_matches(['.', ',', ')', ';', ':'])
            .to_string(),
    )
}

/// Whether the local version is new relative to the baseline: the
/// target spec when a target is given, else the version the local
/// `.changes` file already documents — the most recent `Update to`
/// bullet across its entries. A version with no baseline at all
/// (no target, no `.changes` entry documenting any version) is
/// new: the first entry for a new package. A spec with no version
/// is never new.
pub fn version_is_new(local: &PackageSnapshot, target: Option<&PackageSnapshot>) -> bool {
    let spec_names = local.spec_names();
    let local_spec = spec_names
        .first()
        .map(|name| parse_spec(&local.files[*name]));
    let Some(new_version) = local_spec.as_ref().and_then(|spec| spec.version.clone()) else {
        return false;
    };
    match target {
        Some(target) => {
            let target_spec = target
                .spec_names()
                .first()
                .map(|name| parse_spec(&target.files[*name]));
            let old_version = target_spec.as_ref().and_then(|spec| spec.version.clone());
            match old_version {
                Some(old) => old != new_version,
                None => true,
            }
        }
        None => {
            // No target: the baseline is what the local `.changes`
            // file already documents. The most recent `Update to`
            // bullet names the version the entries already cover —
            // a hotfix on that version, or a later entry for it,
            // still names it through an older bullet. Prefer the
            // changes file whose stem matches the chosen spec: in
            // a multibuild tree the first file in name order can
            // belong to another subpackage. Fall back to name
            // order when no file matches the chosen spec.
            let mut names = local.changes_names();
            if let Some(stem) = spec_names
                .first()
                .and_then(|name| name.strip_suffix(".spec"))
            {
                let preferred = format!("{stem}.changes");
                if let Some(pos) = names.iter().position(|name| *name == preferred.as_str()) {
                    let name = names.remove(pos);
                    names.insert(0, name);
                }
            }
            for name in names {
                let file = parse_changes(&local.files[name]);
                for entry in &file.entries {
                    for (_, text, _) in entry.bullets() {
                        if let Some(documented) = update_bullet_version(&text) {
                            return documented != new_version;
                        }
                    }
                }
            }
            true
        }
    }
}

/// Generate the skeleton entry for a local snapshot, optionally
/// diffed against the target snapshot. `messages` are the `-m`
/// custom-message bullets, one per occurrence, in order; each is
/// the human detail and replaces the TODO summary slot below.
pub fn generate(
    local: &PackageSnapshot,
    target: Option<&PackageSnapshot>,
    author: &str,
    news: Option<&NewsSection>,
    messages: &[String],
) -> Result<Generation, GeectlError> {
    let mut bullets: Vec<String> = Vec::new();
    let mut slots = 0;

    let spec_names = local.spec_names();
    let local_spec = spec_names
        .first()
        .map(|name| parse_spec(&local.files[*name]));
    let target_spec = target.and_then(|target| {
        target
            .spec_names()
            .first()
            .map(|name| parse_spec(&target.files[*name]))
    });
    let new_version = local_spec.as_ref().and_then(|spec| spec.version.clone());
    let version_changed = version_is_new(local, target);
    // NEWS is only for a new version: when the version is
    // unchanged, the upstream section for it was already distilled
    // when the version landed — a hotfix must not re-present it as
    // highlights, demand a summary, or flag needs-detail for it.
    let news = if version_changed { news } else { None };

    // G1: the version line.
    if version_changed {
        if let Some(version) = &new_version {
            bullets.push(format!("- Update to {version}"));
        }
    }

    if let Some(target) = target {
        // G2: patch add/remove lines, names and direction only.
        let old_patches: BTreeSet<&str> = target
            .files
            .keys()
            .filter(|name| is_patch_file(name))
            .map(String::as_str)
            .collect();
        let new_patches: BTreeSet<&str> = local
            .files
            .keys()
            .filter(|name| is_patch_file(name))
            .map(String::as_str)
            .collect();
        for patch in new_patches.difference(&old_patches) {
            bullets.push(format!("- Add {patch}"));
            bullets.push("  * reason: TODO".to_string());
            slots += 1;
        }
        for patch in old_patches.difference(&new_patches) {
            bullets.push(format!("- Drop {patch}"));
            bullets.push("  * reason: TODO".to_string());
            slots += 1;
        }
        // G4: source file add/remove lines.
        let old_sources: BTreeSet<&str> = target
            .files
            .keys()
            .filter(|name| !name.ends_with(".spec") && !name.ends_with(".changes"))
            .map(String::as_str)
            .collect();
        let new_sources: BTreeSet<&str> = local
            .files
            .keys()
            .filter(|name| !name.ends_with(".spec") && !name.ends_with(".changes"))
            .map(String::as_str)
            .collect();
        for file in new_sources.difference(&old_sources) {
            if is_patch_file(file) {
                continue; // already covered by G2
            }
            bullets.push(format!("- Add {file}"));
        }
        for file in old_sources.difference(&new_sources) {
            if is_patch_file(file) {
                continue;
            }
            bullets.push(format!("- Drop {file}"));
        }
        // G3: Requires/BuildRequires add/remove lines, names only.
        if let (Some(local_spec), Some(target_spec)) = (&local_spec, &target_spec) {
            for (label, old, new) in [
                ("Requires", &target_spec.requires, &local_spec.requires),
                (
                    "BuildRequires",
                    &target_spec.build_requires,
                    &local_spec.build_requires,
                ),
            ] {
                let old_set: BTreeSet<&str> = old.iter().map(String::as_str).collect();
                let new_set: BTreeSet<&str> = new.iter().map(String::as_str).collect();
                for value in new_set.difference(&old_set) {
                    bullets.push(format!("- Add {label}: {value}"));
                    bullets.push("  * reason: TODO".to_string());
                    slots += 1;
                }
                for value in old_set.difference(&new_set) {
                    bullets.push(format!("- Drop {label}: {value}"));
                    bullets.push("  * reason: TODO".to_string());
                    slots += 1;
                }
            }
            // G5: license change lines, plus the V10 cross-check is
            // validation's job.
            if local_spec.license != target_spec.license {
                let old_license = target_spec.license.as_deref().unwrap_or("(none)");
                let new_license = local_spec.license.as_deref().unwrap_or("(none)");
                bullets.push(format!("- Change License: {old_license} -> {new_license}"));
            }
        }
    }

    // G7: the extracted upstream section, as a TODO slot — the text
    // is extracted mechanically, never distilled here. The slot is
    // not substantive: the human still has to distill it. Mechanical
    // file/dependency/license lines are not substantive either: a
    // version line plus a tarball rename is still a bare entry.
    let news_found = news.is_some();
    let has_substantive_beyond_version = bullets.iter().any(|bullet| {
        !bullet.starts_with("- Update to")
            && !bullet.contains("TODO")
            && !is_mechanical_bullet(bullet)
    });
    if !messages.is_empty() {
        // A custom message is the human detail: it replaces the
        // TODO summary slot (the highlights slot below, or the
        // no-news summarize slot), so the extracted NEWS lines go
        // with it — the message is the distilled summary.
        for message in messages {
            bullets.extend(message_bullets(message));
        }
    } else if let Some(news) = news {
        bullets.push(
            "- Upstream highlights (TODO: distill to the user-facing items; drop CI, test-only and non-Linux noise):"
                .to_string(),
        );
        slots += 1;
        let mut taken = 0;
        for line in news.text.lines() {
            if taken >= 15 {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let clean = trimmed.trim_start_matches(['-', '*', ' ']).trim();
            if clean.is_empty() {
                continue;
            }
            bullets.extend(wrap_bullet(clean, "  * ", "    "));
            taken += 1;
        }
    } else if version_changed {
        // No NEWS section was found: the human still has to look.
        // This is a slot, not a V9 flag — upstream may maintain no
        // changelog at all.
        bullets.push(
            "  * TODO: summarize the upstream changes, or state '* No user-visible changes' if the release truly has none"
                .to_string(),
        );
        slots += 1;
    }

    // Nothing to generate: the version is unchanged, no target
    // diff produced a line, and no message was supplied. An empty
    // entry would be worse than none — refuse loudly and name the
    // remedy instead of printing a bare header.
    if bullets.iter().all(|bullet| bullet.trim().is_empty()) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!(
                "nothing to generate: version {} is unchanged, no target diff and no -m message supplied — pass -m with the change description, or diff against a target",
                new_version.as_deref().unwrap_or("(no version)")
            ),
        ));
    }

    // V9: upstream news exists and the draft has no substantive line
    // beyond the version line. A custom message is substantive
    // human detail by definition: it satisfies the message part,
    // so the flag clears whenever a message was supplied.
    let needs_detail =
        news_found && version_changed && !has_substantive_beyond_version && messages.is_empty();

    let header = now_header(author);
    let Some(rendered) = header.render() else {
        // Unreachable in practice: `now_header` builds from the
        // civil calendar, so the month is always in range. The
        // fallible render still needs a total handler here.
        return Err(GeectlError::new(
            ErrorKind::InternalError,
            "the entry header has an out-of-range month",
        ));
    };
    let mut entry = String::new();
    entry.push_str(&separator_line());
    entry.push('\n');
    entry.push_str(&rendered);
    entry.push('\n');
    entry.push('\n');
    for bullet in &bullets {
        entry.push_str(bullet);
        entry.push('\n');
    }
    entry.push('\n');
    Ok(Generation {
        entry,
        needs_detail,
        news_found,
        slots,
    })
}

fn is_patch_file(name: &str) -> bool {
    name.ends_with(".patch") || name.ends_with(".diff") || name.ends_with(".dif")
}

/// The result of prepending an entry: the new file content, the
/// encoding the old content was decoded from, when one was guessed
/// or detected (`None` for valid UTF-8), and whether that guess was
/// a single-byte encoding (unverifiable — the write path warns
/// about it instead of refusing).
pub struct PrependedEntry {
    pub content: String,
    pub source_encoding: Option<&'static str>,
    pub guessed_single_byte: bool,
}

/// Prepend an entry to a `.changes` file's content, verifying the
/// insertion: the old content survives as its normalized form and
/// the separator count rises by exactly one. Normalization is the
/// sanctioned rewrite (V11): trailing whitespace stripped, CRLF→LF,
/// legacy encodings transcoded to UTF-8 — so the suffix check runs
/// on the normalized text, not the raw bytes, and bytes no known
/// encoding decodes refuse the write instead of being kept as
/// U+FFFD.
pub fn prepend_entry(old_bytes: &[u8], entry: &str) -> Result<PrependedEntry, String> {
    let report = normalize_changes_content_checked(old_bytes);
    if report.had_undecodable_bytes {
        return Err(
            "refusing to write: the existing .changes file has bytes no known encoding decodes — fix the encoding by hand first"
                .to_string(),
        );
    }
    let old_content = report.text;
    let source_encoding = report.source_encoding;
    let guessed_single_byte = report.guessed_single_byte;
    let new_content = format!("{entry}{old_content}");
    let old_seps = old_content
        .lines()
        .filter(|line| {
            let trimmed = line.trim_end();
            trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-')
        })
        .count();
    let new_seps = new_content
        .lines()
        .filter(|line| {
            let trimmed = line.trim_end();
            trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-')
        })
        .count();
    if !new_content.ends_with(&old_content) || new_seps != old_seps + 1 {
        return Err(
            "prepend verification failed: the old entries did not survive as a normalized suffix"
                .to_string(),
        );
    }
    Ok(PrependedEntry {
        content: new_content,
        source_encoding,
        guessed_single_byte,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changelog::changes::parse_changes;
    use crate::changelog::news::NewsSection;
    use crate::changelog::validate::{validate, ValidateOptions};

    fn snapshot(files: &[(&str, &str)]) -> PackageSnapshot {
        PackageSnapshot {
            files: files
                .iter()
                .map(|(name, content)| ((*name).to_string(), (*content).to_string()))
                .collect(),
        }
    }

    const SPEC_V2: &str = "Name:           foo\nVersion:        2.0\nLicense:        GPL-3.0-only\nSource0:        foo-2.0.tar.gz\nPatch0:         new.patch\nBuildRequires:  pkgconfig\n%changelog\n";
    const SPEC_V1: &str = "Name:           foo\nVersion:        1.0\nLicense:        MIT\nSource0:        foo-1.0.tar.gz\nPatch0:         old.patch\n%changelog\n";

    #[test]
    fn generate_skeleton_from_the_diff() {
        let target = snapshot(&[
            ("foo.spec", SPEC_V1),
            ("foo-1.0.tar.gz", "binary"),
            ("old.patch", "diff"),
        ]);
        let local = snapshot(&[
            ("foo.spec", SPEC_V2),
            ("foo-2.0.tar.gz", "binary"),
            ("new.patch", "diff"),
        ]);
        let news = NewsSection {
            source_file: "NEWS".to_string(),
            text: "## 2.0\n\n- Fix a crash\n".to_string(),
        };
        let gen = generate(
            &local,
            Some(&target),
            "Jane Packager <jane@example.com>",
            Some(&news),
            &[],
        )
        .expect("generation runs");
        assert!(gen.entry.contains("- Update to 2.0"));
        assert!(gen.entry.contains("- Add new.patch"));
        assert!(gen.entry.contains("- Drop old.patch"));
        assert!(gen.entry.contains("- Add BuildRequires: pkgconfig"));
        assert!(gen.entry.contains("- Change License: MIT -> GPL-3.0-only"));
        assert!(gen.entry.contains("Upstream highlights (TODO: distill"));
        assert!(gen.entry.contains("Fix a crash"));
        assert!(gen.news_found);
        // Mechanical lines only (patch, dependency, license, source
        // add/drop): none of them is substantive, so the draft still
        // needs detail even though it is not version-line-only.
        assert!(gen.needs_detail);
        assert!(gen.slots >= 3);
        // The entry ends with a blank line, and the header is canonical.
        assert!(gen.entry.ends_with("\n\n"));
        assert!(gen.entry.starts_with(&"-".repeat(67)));
    }

    #[test]
    fn generate_without_a_target_emits_the_version_line_and_a_slot() {
        let local = snapshot(&[("foo.spec", SPEC_V2)]);
        let gen = generate(&local, None, "Jane Packager <jane@example.com>", None, &[])
            .expect("generation runs");
        assert!(gen.entry.contains("- Update to 2.0"));
        assert!(gen.entry.contains("TODO: summarize the upstream changes"));
        assert!(!gen.news_found);
        assert!(!gen.needs_detail);
    }

    #[test]
    fn generate_flags_needs_detail_when_news_exists_and_nothing_else() {
        // A version-only diff (no patch/dep/license/source change)
        // with upstream news: the draft is version-line-only plus
        // the highlights slot, which is not substantive.
        let target = snapshot(&[("foo.spec", SPEC_V1)]);
        let local_spec = SPEC_V1.replace("Version:        1.0", "Version:        2.0");
        let local = snapshot(&[("foo.spec", &local_spec)]);
        let news = NewsSection {
            source_file: "NEWS".to_string(),
            text: "## 2.0\n\n- Fix a crash\n".to_string(),
        };
        let gen = generate(
            &local,
            Some(&target),
            "Jane Packager <jane@example.com>",
            Some(&news),
            &[],
        )
        .expect("generation runs");
        assert!(gen.needs_detail);
    }

    #[test]
    fn generate_license_only_change_has_no_slots() {
        // Same version, only the license moved, no news: the draft
        // is one mechanical line with nothing to fill in — the
        // `--write` success case.
        let target = snapshot(&[("foo.spec", SPEC_V1)]);
        let local_spec = SPEC_V1.replace("License:        MIT", "License:        GPL-3.0-only");
        let local = snapshot(&[("foo.spec", &local_spec)]);
        let gen = generate(
            &local,
            Some(&target),
            "Jane Packager <jane@example.com>",
            None,
            &[],
        )
        .expect("generation runs");
        assert!(gen.entry.contains("- Change License: MIT -> GPL-3.0-only"));
        assert_eq!(gen.slots, 0);
        assert!(!gen.needs_detail);
    }

    #[test]
    fn prepend_entry_verifies_the_insertion() {
        let sep = "-".repeat(67);
        let old =
            format!("{sep}\nWed May 27 16:31:48 UTC 2026 - Jane <jane@example.com>\n\n- Old\n\n");
        let entry =
            format!("{sep}\nThu May 28 10:00:00 UTC 2026 - Jane <jane@example.com>\n\n- New\n\n");
        let new = prepend_entry(old.as_bytes(), &entry).expect("prepend succeeds");
        assert!(new.content.starts_with(&entry));
        assert!(new.content.ends_with(&old));
        assert_eq!(new.source_encoding, None);
    }

    #[test]
    fn prepend_entry_keeps_a_meego_style_old_entry() {
        // A Meego-style old entry: separator plus a header with no
        // ` - ` separator and no time. Prepend never parses old
        // entries, so the foreign style survives as a suffix — the
        // commit gate is what judges it (V5), not the prepend.
        let sep = "-".repeat(67);
        let old = format!(
            "{sep}\nSat Apr 14 2007 Tobias Blomberg <t@example.com>\n\n- Old Meego change\n\n"
        );
        let entry =
            format!("{sep}\nThu May 28 10:00:00 UTC 2026 - Jane <jane@example.com>\n\n- New\n\n");
        let new = prepend_entry(old.as_bytes(), &entry).expect("prepend succeeds");
        assert!(new.content.starts_with(&entry));
        assert!(new.content.ends_with(&old));
        assert_eq!(new.source_encoding, None);
    }

    #[test]
    fn prepend_entry_keeps_a_separator_less_meego_file() {
        // The pure Meego shape: no separators at all, entries start
        // with a short `* Day Mon DD YYYY Name` line. The new entry
        // supplies the only separator, so the count rises from zero
        // to one and the whole old file survives as the suffix.
        let sep = "-".repeat(67);
        let old = "* Sat Apr 14 2007 Tobias Blomberg <t@example.com>\n- Old Meego change\n\n* Fri Apr 13 2007 Tobias Blomberg <t@example.com>\n- Older change\n\n";
        let entry =
            format!("{sep}\nThu May 28 10:00:00 UTC 2026 - Jane <jane@example.com>\n\n- New\n\n");
        let new = prepend_entry(old.as_bytes(), &entry).expect("prepend succeeds");
        assert!(new.content.starts_with(&entry));
        assert!(new.content.ends_with(old));
        assert_eq!(new.source_encoding, None);
    }

    #[test]
    fn wrap_bullet_wraps_at_67() {
        let lines = wrap_bullet(
            "Fix a very long crash in the parser that runs past sixty-seven characters for sure",
            "- ",
            "  ",
        );
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|line| line.chars().count() <= 67));
        assert!(lines[0].starts_with("- "));
    }

    fn messages(messages: &[&str]) -> Vec<String> {
        messages.iter().map(|message| message.to_string()).collect()
    }

    #[test]
    fn message_renders_as_a_bullet_and_clears_the_summary_slot() {
        // No target, no news: without a message the draft carries
        // the summarize TODO slot; with one, the slot is replaced
        // by the message bullet and the draft is complete.
        let local = snapshot(&[("foo.spec", SPEC_V2)]);
        let gen = generate(
            &local,
            None,
            "Jane Packager <jane@example.com>",
            None,
            &messages(&["Fix the parser crash"]),
        )
        .expect("generation runs");
        assert!(gen.entry.contains("- Fix the parser crash"));
        assert!(!gen.entry.contains("TODO"));
        assert_eq!(gen.slots, 0);
        assert!(!gen.needs_detail);
    }

    #[test]
    fn message_multiline_newlines_become_continuation_lines() {
        let local = snapshot(&[("foo.spec", SPEC_V2)]);
        let gen = generate(
            &local,
            None,
            "Jane Packager <jane@example.com>",
            None,
            &messages(&["First line\nsecond line here"]),
        )
        .expect("generation runs");
        assert!(gen.entry.contains("- First line\n  second line here\n"));
        // One bullet in the parser's model: the continuation joins
        // it, it does not start a second bullet.
        let file = parse_changes(&gen.entry);
        assert_eq!(file.entries.len(), 1);
        assert_eq!(file.entries[0].bullets().len(), 2);
        assert!(file.entries[0].bullets()[1]
            .1
            .contains("First line second line here"));
    }

    #[test]
    fn message_long_lines_wrap_at_67() {
        let local = snapshot(&[("foo.spec", SPEC_V2)]);
        let gen = generate(
            &local,
            None,
            "Jane Packager <jane@example.com>",
            None,
            &messages(&[
                "Fix a very long crash in the parser that runs past sixty-seven characters for sure",
            ]),
        )
        .expect("generation runs");
        assert!(gen.entry.lines().all(|line| line.chars().count() <= 67));
        assert!(gen
            .entry
            .contains("- Fix a very long crash in the parser that runs past"));
    }

    #[test]
    fn message_repeatable_gives_separate_bullets_in_order() {
        let local = snapshot(&[("foo.spec", SPEC_V2)]);
        let gen = generate(
            &local,
            None,
            "Jane Packager <jane@example.com>",
            None,
            &messages(&["First bullet", "Second bullet"]),
        )
        .expect("generation runs");
        assert!(gen.entry.contains("- First bullet\n- Second bullet\n"));
    }

    #[test]
    fn message_leading_marker_is_stripped_not_doubled() {
        let local = snapshot(&[("foo.spec", SPEC_V2)]);
        let gen = generate(
            &local,
            None,
            "Jane Packager <jane@example.com>",
            None,
            &messages(&["- Fix it", "* Also this", "+ Third"]),
        )
        .expect("generation runs");
        assert!(gen.entry.contains("- Fix it\n"));
        assert!(gen.entry.contains("- Also this\n"));
        assert!(gen.entry.contains("- Third\n"));
        assert!(!gen.entry.contains("- - "));
        assert!(!gen.entry.contains("\n* "));
        assert!(!gen.entry.contains("\n+ "));
        // A marker on a continuation line is stripped too: the
        // marker check in `bullets()` runs before the indent
        // check, so an indented `- ` line would split the bullet.
        let gen = generate(
            &local,
            None,
            "Jane Packager <jane@example.com>",
            None,
            &messages(&["Fix foo\n- Fix bar"]),
        )
        .expect("generation runs");
        assert!(gen.entry.contains("- Fix foo\n  Fix bar\n"));
    }

    #[test]
    fn message_bullets_trims_leading_and_trailing_blank_lines() {
        let lines = message_bullets("\n\nFix it\n\n");
        assert_eq!(lines, vec!["- Fix it".to_string()]);
        // Interior blank lines stay: paragraph breaks.
        let lines = message_bullets("First\n\nSecond");
        assert_eq!(
            lines,
            vec!["- First".to_string(), String::new(), "  Second".to_string()]
        );
        // All blank renders to nothing.
        assert!(message_bullets("\n\n").is_empty());
    }

    #[test]
    fn message_bullets_strips_control_characters() {
        let lines = message_bullets("Fix it\u{1b}[31m red");
        assert!(lines.iter().all(|line| !line.contains('\u{1b}')));
        assert!(lines[0].contains("Fix it"));
        // A bidi mark is stripped too.
        let lines = message_bullets("Fix\u{202e} it");
        assert!(lines.iter().all(|line| !line.contains('\u{202e}')));
        // A tab is whitespace the wrapper treats as a space, never
        // an escape.
        let lines = message_bullets("Fix\tit");
        assert_eq!(lines, vec!["- Fix it".to_string()]);
    }

    #[test]
    fn message_blank_line_is_a_paragraph_break_within_the_bullet() {
        let local = snapshot(&[("foo.spec", SPEC_V2)]);
        let gen = generate(
            &local,
            None,
            "Jane Packager <jane@example.com>",
            None,
            &messages(&["Para one.\n\nPara two."]),
        )
        .expect("generation runs");
        assert!(gen.entry.contains("- Para one.\n\n  Para two.\n"));
        // The parser joins over the blank line: still one bullet.
        let file = parse_changes(&gen.entry);
        assert_eq!(file.entries[0].bullets().len(), 2);
        assert!(file.entries[0].bullets()[1]
            .1
            .contains("Para one. Para two."));
    }

    #[test]
    fn message_replaces_the_highlights_slot_when_news_exists() {
        let target = snapshot(&[("foo.spec", SPEC_V1)]);
        let local_spec = SPEC_V1.replace("Version:        1.0", "Version:        2.0");
        let local = snapshot(&[("foo.spec", &local_spec)]);
        let news = NewsSection {
            source_file: "NEWS".to_string(),
            text: "## 2.0\n\n- Fix a crash\n".to_string(),
        };
        let gen = generate(
            &local,
            Some(&target),
            "Jane Packager <jane@example.com>",
            Some(&news),
            &messages(&["Rework the parser error path"]),
        )
        .expect("generation runs");
        assert!(gen.news_found);
        assert!(gen.entry.contains("- Rework the parser error path"));
        assert!(!gen.entry.contains("Upstream highlights"));
        assert!(!gen.entry.contains("Fix a crash"));
        assert!(!gen.entry.contains("TODO"));
        assert_eq!(gen.slots, 0);
        assert!(!gen.needs_detail);
    }

    #[test]
    fn message_does_not_clear_reason_slots() {
        // A patch add carries a `reason: TODO` slot, which is
        // item-specific: the message satisfies the message part
        // (no needs-detail), but `--write` still refuses on the
        // unfilled slot.
        let target = snapshot(&[
            ("foo.spec", SPEC_V1),
            ("foo-1.0.tar.gz", "binary"),
            ("old.patch", "diff"),
        ]);
        let local = snapshot(&[
            ("foo.spec", SPEC_V2),
            ("foo-2.0.tar.gz", "binary"),
            ("new.patch", "diff"),
        ]);
        let gen = generate(
            &local,
            Some(&target),
            "Jane Packager <jane@example.com>",
            None,
            &messages(&["Rework the parser error path"]),
        )
        .expect("generation runs");
        assert!(!gen.needs_detail);
        assert!(gen.slots > 0);
        assert!(gen.entry.contains("reason: TODO"));
    }

    #[test]
    fn message_entry_validates_clean() {
        // The generated entry with a message, as a `.changes`
        // file, passes the V-rules: the `--write` path writes
        // exactly this text.
        let local = snapshot(&[("foo.spec", SPEC_V2)]);
        let gen = generate(
            &local,
            None,
            "Jane Packager <jane@example.com>",
            None,
            &messages(&[
                "Fix the parser crash\nacross two lines",
                "Add the new subcommand",
            ]),
        )
        .expect("generation runs");
        let written = snapshot(&[("foo.spec", SPEC_V2), ("foo.changes", &gen.entry)]);
        let report = validate(
            &written,
            None,
            &ValidateOptions {
                entries: 1,
                news_section: None,
                tarball_present: false,
                news_lookup_failed: false,
            },
        );
        assert!(report.passed(), "findings: {:?}", report.findings);
    }

    #[test]
    fn message_bullets_of_empty_text_has_no_bullet_lines() {
        // The CLI refuses these; the renderer itself never
        // invents a bullet from blank lines or a bare marker.
        assert!(message_bullets("")
            .iter()
            .all(|line| line.trim().is_empty()));
        assert!(message_bullets("   ")
            .iter()
            .all(|line| line.trim().is_empty()));
        assert!(message_bullets("* ")
            .iter()
            .all(|line| line.trim().is_empty()));
        assert!(message_bullets("\n\n")
            .iter()
            .all(|line| line.trim().is_empty()));
    }

    /// One `.changes` entry documenting `version`: the newest
    /// entry of a package already at that version.
    fn changes_at(version: &str) -> String {
        format!(
            "{}\nWed May 27 16:31:48 UTC 2026 - Jane Packager <jane@example.com>\n\n- Update to {version}\n\n",
            "-".repeat(67)
        )
    }

    #[test]
    fn version_is_new_reads_the_local_changes_file() {
        let at_v2 = snapshot(&[("foo.spec", SPEC_V2), ("foo.changes", &changes_at("2.0"))]);
        assert!(!version_is_new(&at_v2, None));
        let at_v1 = snapshot(&[("foo.spec", SPEC_V2), ("foo.changes", &changes_at("1.0"))]);
        assert!(version_is_new(&at_v1, None));
        let no_changes = snapshot(&[("foo.spec", SPEC_V2)]);
        assert!(version_is_new(&no_changes, None));
        // A later hotfix entry for the same version still names
        // it through the older Update bullet.
        let hotfix = snapshot(&[
            ("foo.spec", SPEC_V2),
            (
                "foo.changes",
                &format!(
                    "{}\nWed May 28 10:00:00 UTC 2026 - Jane Packager <jane@example.com>\n\n- fix bug122\n\n{}",
                    "-".repeat(67),
                    changes_at("2.0")
                ),
            ),
        ]);
        assert!(!version_is_new(&hotfix, None));
    }

    #[test]
    fn update_bullet_version_keeps_the_version_verbatim() {
        assert_eq!(
            update_bullet_version("- Update to 2.0-RC1").as_deref(),
            Some("2.0-RC1")
        );
        // The marker matches case-insensitively; the version does
        // not change case.
        assert_eq!(
            update_bullet_version("- UPDATED TO 2.0-rc1").as_deref(),
            Some("2.0-rc1")
        );
        assert_eq!(
            update_bullet_version("- Updated to version 2.0").as_deref(),
            Some("2.0")
        );
        assert_eq!(update_bullet_version("- Fix a crash"), None);
    }

    #[test]
    fn uppercase_version_hotfix_is_not_new() {
        // The P2: a `.changes` entry documenting `2.0-RC1` used
        // to read as new (lowercased to `2.0-rc1`), generating a
        // spurious `- Update to` line and demanding NEWS for a
        // hotfix. The version stays verbatim, so the hotfix just
        // carries its message.
        let spec_rc = SPEC_V2.replace("Version:        2.0", "Version:        2.0-RC1");
        let local = snapshot(&[
            ("foo.spec", &spec_rc),
            ("foo.changes", &changes_at("2.0-RC1")),
        ]);
        assert!(!version_is_new(&local, None));
        let gen = generate(
            &local,
            None,
            "Jane Packager <jane@example.com>",
            None,
            &messages(&["fix bug124"]),
        )
        .expect("generation runs");
        assert!(gen.entry.contains("- fix bug124"));
        assert!(!gen.entry.contains("Update to"));
        assert!(!gen.entry.contains("TODO"));
    }

    #[test]
    fn version_baseline_prefers_the_chosen_specs_changes_file() {
        // Multibuild tree: the alphabetically-first changes file
        // belongs to another subpackage. The baseline must come
        // from the chosen spec's own changes file.
        let local = snapshot(&[
            ("foo.spec", SPEC_V2),
            ("zzz.spec", SPEC_V1),
            ("aaa.changes", &changes_at("1.0")),
            ("foo.changes", &changes_at("2.0")),
        ]);
        assert!(!version_is_new(&local, None));
    }

    #[test]
    fn version_baseline_falls_back_to_name_order_without_a_match() {
        // No changes file matches the chosen spec's stem: the
        // first file in name order still supplies the baseline.
        let local = snapshot(&[("foo.spec", SPEC_V2), ("bar.changes", &changes_at("1.0"))]);
        assert!(version_is_new(&local, None));
    }

    #[test]
    fn unchanged_version_with_message_carries_only_the_message() {
        // Hotfix shape: spec 2.0, `.changes` already at 2.0, NEWS
        // tarball for 2.0 present, no target. The version is not
        // new, so no version line, no NEWS extraction, no slot —
        // the entry just carries the message.
        let local = snapshot(&[("foo.spec", SPEC_V2), ("foo.changes", &changes_at("2.0"))]);
        let news = NewsSection {
            source_file: "NEWS".to_string(),
            text: "## 2.0\n\n- Fix a crash\n".to_string(),
        };
        let gen = generate(
            &local,
            None,
            "Jane Packager <jane@example.com>",
            Some(&news),
            &messages(&["fix bug123"]),
        )
        .expect("generation runs");
        assert!(!gen.news_found);
        assert!(gen.entry.contains("- fix bug123"));
        assert!(!gen.entry.contains("Update to"));
        assert!(!gen.entry.contains("Fix a crash"));
        assert!(!gen.entry.contains("TODO"));
        assert_eq!(gen.slots, 0);
        assert!(!gen.needs_detail);
    }

    #[test]
    fn unchanged_version_without_message_or_diff_is_nothing_to_generate() {
        let local = snapshot(&[("foo.spec", SPEC_V2), ("foo.changes", &changes_at("2.0"))]);
        let news = NewsSection {
            source_file: "NEWS".to_string(),
            text: "## 2.0\n\n- Fix a crash\n".to_string(),
        };
        let err = generate(
            &local,
            None,
            "Jane Packager <jane@example.com>",
            Some(&news),
            &[],
        )
        .expect_err("nothing to generate");
        assert_eq!(err.kind, ErrorKind::InvalidInput);
        assert!(
            err.context.contains("nothing to generate"),
            "{}",
            err.context
        );
    }

    #[test]
    fn unchanged_version_with_target_diff_keeps_patch_lines() {
        // Same version on both sides, a patch added: the patch
        // lines still describe the change, and their `reason: TODO`
        // slots still stand — the message satisfies the message
        // part only.
        const SPEC_V2_OLD_PATCH: &str = "Name:           foo\nVersion:        2.0\nLicense:        GPL-3.0-only\nSource0:        foo-2.0.tar.gz\nPatch0:         old.patch\nBuildRequires:  pkgconfig\n%changelog\n";
        let target = snapshot(&[
            ("foo.spec", SPEC_V2_OLD_PATCH),
            ("foo-2.0.tar.gz", "binary"),
            ("old.patch", "diff"),
        ]);
        let local = snapshot(&[
            ("foo.spec", SPEC_V2),
            ("foo-2.0.tar.gz", "binary"),
            ("new.patch", "diff"),
        ]);
        let gen = generate(
            &local,
            Some(&target),
            "Jane Packager <jane@example.com>",
            None,
            &messages(&["fix bug123"]),
        )
        .expect("generation runs");
        assert!(gen.entry.contains("- Add new.patch"));
        assert!(gen.entry.contains("- Drop old.patch"));
        assert!(gen.entry.contains("- fix bug123"));
        assert!(!gen.entry.contains("Update to"));
        assert!(!gen.needs_detail);
        assert_eq!(gen.slots, 2);
        assert!(gen.entry.contains("reason: TODO"));
    }

    #[test]
    fn version_bump_without_target_still_extracts_news() {
        // The bump shape for the no-target path: spec 2.0,
        // `.changes` still at 1.0. The version is new, so the
        // version line and the NEWS slot stand as before.
        let local = snapshot(&[("foo.spec", SPEC_V2), ("foo.changes", &changes_at("1.0"))]);
        let news = NewsSection {
            source_file: "NEWS".to_string(),
            text: "## 2.0\n\n- Fix a crash\n".to_string(),
        };
        let gen = generate(
            &local,
            None,
            "Jane Packager <jane@example.com>",
            Some(&news),
            &[],
        )
        .expect("generation runs");
        assert!(gen.news_found);
        assert!(gen.entry.contains("- Update to 2.0"));
        assert!(gen.entry.contains("Upstream highlights (TODO: distill"));
        assert!(gen.entry.contains("Fix a crash"));
        assert!(gen.needs_detail);
        assert!(gen.slots >= 1);
    }
}
