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

use crate::changelog::changes::{civil_from_days, separator_line, weekday_name, Header};
use crate::changelog::news::NewsSection;
use crate::changelog::spec::parse_spec;
use crate::changelog::validate::{is_mechanical_bullet, PackageSnapshot};
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

/// Generate the skeleton entry for a local snapshot, optionally
/// diffed against the target snapshot.
pub fn generate(
    local: &PackageSnapshot,
    target: Option<&PackageSnapshot>,
    author: &str,
    news: Option<&NewsSection>,
) -> Result<Generation, String> {
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
    let old_version = target_spec.as_ref().and_then(|spec| spec.version.clone());
    let version_changed = match (&new_version, &old_version) {
        (Some(new), Some(old)) => new != old,
        (Some(_), None) => true,
        _ => false,
    };

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
    if let Some(news) = news {
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

    // V9: upstream news exists and the draft has no substantive line
    // beyond the version line.
    let needs_detail = news_found && version_changed && !has_substantive_beyond_version;

    let header = now_header(author);
    let Some(rendered) = header.render() else {
        // Unreachable in practice: `now_header` builds from the
        // civil calendar, so the month is always in range. The
        // fallible render still needs a total handler here.
        return Err("the entry header has an out-of-range month".to_string());
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

/// Prepend an entry to a `.changes` file's content, verifying the
/// insertion: the old content survives as a byte-suffix and the
/// separator count rises by exactly one.
pub fn prepend_entry(old_content: &str, entry: &str) -> Result<String, String> {
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
    if !new_content.ends_with(old_content) || new_seps != old_seps + 1 {
        return Err(
            "prepend verification failed: the old entries did not survive as a byte-suffix"
                .to_string(),
        );
    }
    Ok(new_content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changelog::news::NewsSection;

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
        let gen = generate(&local, None, "Jane Packager <jane@example.com>", None)
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
        let new = prepend_entry(&old, &entry).expect("prepend succeeds");
        assert!(new.starts_with(&entry));
        assert!(new.ends_with(&old));
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
}
