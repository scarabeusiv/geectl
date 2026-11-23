//! The validation rules V1–V12 over a package snapshot.
//!
//! V1–V4 are the `factory-auto` bot (`check_source.py`,
//! `detect_mentioned_patches`); V5–V8 the source validator service;
//! V9–V12 the review-team rules. V9 is a heuristic, the rest are
//! exact. Where a rule is already owned by the packaging skill, the
//! rule text here stays minimal and the skill file owns the detail:
//! format and content in `changelog-rules.md`, mechanics in
//! `changelog-entry.md`, the mechanical gates in `changes-lint.sh` /
//! `changes-guard.sh` / `changes-patches.sh` — the V4, V11 and V12
//! checks below are ports of those scripts, simplified where the
//! full script carries attribution logic no deterministic layer
//! needs (the simplifications are named at each check).
//!
//! A check that needs the target and has none is skipped with a
//! reason, never passed silently. A failed lookup (the target cannot
//! be read at all) is the caller's error, not a skip — see the CLI.

use crate::vc::changes::{parse_changes, ChangesFile, Entry};
use crate::vc::spec::{parse_spec, SpecInfo};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// One validation rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rule {
    V1,
    V2,
    V3,
    V4,
    V5,
    V6,
    V7,
    V8,
    V9,
    V10,
    V11,
    V12,
}

impl Rule {
    /// The stable id printed in findings (`V1` … `V12`).
    pub fn id(self) -> &'static str {
        match self {
            Rule::V1 => "V1",
            Rule::V2 => "V2",
            Rule::V3 => "V3",
            Rule::V4 => "V4",
            Rule::V5 => "V5",
            Rule::V6 => "V6",
            Rule::V7 => "V7",
            Rule::V8 => "V8",
            Rule::V9 => "V9",
            Rule::V10 => "V10",
            Rule::V11 => "V11",
            Rule::V12 => "V12",
        }
    }
}

/// One finding: the rule, the file, the 1-based line (0 when the
/// finding is about the file as a whole), and the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule: Rule,
    pub file: String,
    pub line: usize,
    pub message: String,
}

/// A rule that did not run, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub rule: Rule,
    pub reason: String,
}

/// The result of [`validate`].
#[derive(Debug, Clone, Default)]
pub struct ValidationReport {
    pub findings: Vec<Finding>,
    pub skipped: Vec<Skipped>,
}

impl ValidationReport {
    /// True when no rule failed.
    pub fn passed(&self) -> bool {
        self.findings.is_empty()
    }
}

/// A package directory: file name to text content. Content is
/// carried only for `.spec` and `.changes` files (UTF-8 lossy);
/// every other file maps to `""` — it is present by name only, which
/// is all V7 needs, and a source tarball is never read into memory
/// as text. Only top-level regular files; hidden files (`.`-prefixed)
/// are skipped by the directory reader.
#[derive(Debug, Clone, Default)]
pub struct PackageSnapshot {
    pub files: BTreeMap<String, String>,
}

impl PackageSnapshot {
    /// Read a directory into a snapshot.
    pub fn from_dir(dir: &std::path::Path) -> std::io::Result<Self> {
        let mut files = BTreeMap::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            // Only .spec and .changes files are ever parsed; every
            // other file is needed by name alone (V7), so its content
            // stays empty — a source tarball must not be read into
            // memory as text.
            let content = if name.ends_with(".spec") || name.ends_with(".changes") {
                let bytes = std::fs::read(&path)?;
                String::from_utf8_lossy(&bytes).into_owned()
            } else {
                String::new()
            };
            files.insert(name, content);
        }
        Ok(PackageSnapshot { files })
    }

    pub fn spec_names(&self) -> Vec<&str> {
        self.files
            .keys()
            .filter(|name| name.ends_with(".spec"))
            .map(String::as_str)
            .collect()
    }

    pub fn changes_names(&self) -> Vec<&str> {
        self.files
            .keys()
            .filter(|name| name.ends_with(".changes"))
            .map(String::as_str)
            .collect()
    }
}

/// Options for [`validate`].
pub struct ValidateOptions<'a> {
    /// Newest N entries to lint for format (V11), and the number of
    /// new entries the submission may carry vs the target. Default 1:
    /// one entry per SR.
    pub entries: usize,
    /// The extracted upstream NEWS section for the new version, when
    /// the tarball has one. `None` means no news was found — V9 does
    /// not flag that case (upstream may maintain no changelog at
    /// all); the report records the skip instead.
    pub news_section: Option<&'a str>,
    /// Whether a source tarball was found on disk at all. V9
    /// distinguishes the two no-news cases: a tarball with no NEWS
    /// section is a plain skip, while no tarball means the bare
    /// update line was never checked against upstream NEWS.
    /// Default false: unknown, treated as absent.
    pub tarball_present: bool,
    /// Whether the upstream NEWS lookup failed. The caller reports
    /// that failure itself, so V9 adds no second skip for it.
    /// Default false.
    pub news_lookup_failed: bool,
}

impl Default for ValidateOptions<'_> {
    fn default() -> Self {
        ValidateOptions {
            entries: 1,
            news_section: None,
            tarball_present: false,
            news_lookup_failed: false,
        }
    }
}

struct Parsed {
    specs: BTreeMap<String, SpecInfo>,
    changes: BTreeMap<String, ChangesFile>,
}

fn parse_snapshot(snapshot: &PackageSnapshot) -> Parsed {
    let mut specs = BTreeMap::new();
    let mut changes = BTreeMap::new();
    for name in snapshot.spec_names() {
        specs.insert(name.to_string(), parse_spec(&snapshot.files[name]));
    }
    for name in snapshot.changes_names() {
        changes.insert(name.to_string(), parse_changes(&snapshot.files[name]));
    }
    Parsed { specs, changes }
}

fn stem<'a>(file: &'a str, suffix: &str) -> &'a str {
    file.strip_suffix(suffix).unwrap_or(file)
}

fn is_patch_file(name: &str) -> bool {
    name.ends_with(".patch") || name.ends_with(".diff") || name.ends_with(".dif")
}

/// A `%{macro}` in a `Source:`/`Patch:` value matches any run of
/// characters; the rest matches literally. A small glob: `*` matches
/// any run, `?` one character.
fn glob_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_here(&pat, 0, &txt, 0)
}

/// Iterative `*`/`?` glob: the recursive version backtracked over
/// every split per `*` (exponential worst case). The star remembers
/// where it started and the match restarts from there, consuming one
/// more character per retry — linear in the text per star.
fn glob_here(pat: &[char], mut pi: usize, txt: &[char], mut ti: usize) -> bool {
    let mut star_pi: Option<usize> = None;
    let mut star_ti = 0;
    while ti < txt.len() {
        if pi < pat.len() && (pat[pi] == '?' || pat[pi] == txt[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pat.len() && pat[pi] == '*' {
            while pi < pat.len() && pat[pi] == '*' {
                pi += 1;
            }
            star_pi = Some(pi);
            star_ti = ti;
        } else if let Some(restart) = star_pi {
            pi = restart;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == '*' {
        pi += 1;
    }
    pi == pat.len()
}

/// A `Source:`/`Patch:` value as a glob: `%{...}` macros become `*`,
/// and only the basename is matched (values are usually bare
/// filenames, sometimes a URL).
fn source_value_matches(value: &str, file: &str) -> bool {
    let base = value.rsplit('/').next().unwrap_or(value);
    let mut pattern = String::new();
    let mut rest = base;
    while let Some(idx) = rest.find("%{") {
        pattern.push_str(&rest[..idx]);
        pattern.push('*');
        match rest[idx + 2..].find('}') {
            Some(end) => rest = &rest[idx + 2 + end + 1..],
            None => {
                rest = "";
                break;
            }
        }
    }
    pattern.push_str(rest);
    glob_match(&pattern, file)
}

/// The LCS table is capped: past this many cells the diff falls back
/// to a count-aware set diff (V4 only needs membership with
/// multiplicity, not hunk structure). A `.changes` file pair past
/// the cap is already absurd; the table would be O(n*m) time and
/// memory with no bound.
const MAX_LCS_CELLS: usize = 1_000_000;

/// Lines in `from` beyond their count in `to`, in `from` order:
/// each line appears `count(from) - count(to)` times (zero when `to`
/// has at least as many). The multiplicity-sensitive half of the
/// past-the-cap fallback for [`diff_added_removed`].
fn count_diff(from: &[String], to: &[String]) -> Vec<String> {
    let mut from_counts: HashMap<&str, usize> = HashMap::new();
    for line in from {
        *from_counts.entry(line.as_str()).or_default() += 1;
    }
    let mut to_counts: HashMap<&str, usize> = HashMap::new();
    for line in to {
        *to_counts.entry(line.as_str()).or_default() += 1;
    }
    let mut remaining: HashMap<&str, usize> = HashMap::new();
    for (line, count) in &from_counts {
        let in_to = to_counts.get(*line).copied().unwrap_or(0);
        if *count > in_to {
            remaining.insert(*line, *count - in_to);
        }
    }
    let mut out = Vec::new();
    for line in from {
        if let Some(left) = remaining.get_mut(line.as_str()) {
            if *left > 0 {
                *left -= 1;
                out.push(line.clone());
            }
        }
    }
    out
}

/// Line-based diff: the added lines (in `new` only) and the removed
/// lines (in `old` only), via a longest-common-subsequence table.
/// Only membership with multiplicity matters for V4, not hunk
/// structure: a line twice in `new` and once in `old` is one
/// addition, and the fallback must match the LCS path there, or V4
/// misfires on long-lived `.changes` files that reach the cap.
/// (The two paths still split whenever line order prevents matching
/// all shared copies — a pure reorder is the simplest case, but any
/// reorder mixed with additions or removals splits them too. The
/// fallback's set is always a subset of the LCS set, so past the cap
/// V4 can only gain findings, never lose them.)
fn diff_added_removed(old: &[String], new: &[String]) -> (Vec<String>, Vec<String>) {
    let n = old.len();
    let m = new.len();
    if n.saturating_mul(m) > MAX_LCS_CELLS {
        return (count_diff(new, old), count_diff(old, new));
    }
    let mut table = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = if old[i] == new[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old[i] == new[j] {
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            removed.push(old[i].clone());
            i += 1;
        } else {
            added.push(new[j].clone());
            j += 1;
        }
    }
    removed.extend_from_slice(&old[i..]);
    added.extend_from_slice(&new[j..]);
    (added, removed)
}

/// Case-insensitive substring search.
fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// The newest entry of a `.changes` file, if it has one.
fn newest_entry(file: &ChangesFile) -> Option<&Entry> {
    file.entries.first()
}

fn newest_timestamp(file: &ChangesFile) -> Option<i64> {
    newest_entry(file).and_then(|e| e.header.as_ref().map(|h| h.timestamp))
}

/// Validate a local package snapshot, optionally against the target
/// snapshot (the target project's current files). `None` as the
/// target means local-only mode: the target-relative rules are
/// skipped with a reason.
pub fn validate(
    local: &PackageSnapshot,
    target: Option<&PackageSnapshot>,
    options: &ValidateOptions,
) -> ValidationReport {
    let mut report = ValidationReport::default();
    let parsed = parse_snapshot(local);
    let target_parsed = target.map(parse_snapshot);
    let entries = options.entries.max(1);

    // The package names: spec `Name:` tags plus subpackages.
    let mut package_names: BTreeSet<String> = BTreeSet::new();
    for spec in parsed.specs.values() {
        if let Some(name) = &spec.name {
            package_names.insert(name.clone());
        }
        for sub in &spec.subpackages {
            package_names.insert(sub.clone());
        }
    }
    let source_values: BTreeSet<String> = parsed
        .specs
        .values()
        .flat_map(|spec| spec.sources.iter().cloned())
        .collect();

    // V1: every spec has a matching `<name>.changes` file.
    for spec_name in local.spec_names() {
        let wanted = format!("{}.changes", stem(spec_name, ".spec"));
        if !local.files.contains_key(&wanted) {
            report.findings.push(Finding {
                rule: Rule::V1,
                file: spec_name.to_string(),
                line: 0,
                message: format!("no matching {wanted} file for this spec"),
            });
        }
    }

    // V3: every spec contains a `%changelog` line. V8: `Version:` is
    // present and a single token at line start.
    for (spec_name, spec) in &parsed.specs {
        if !spec.has_changelog {
            report.findings.push(Finding {
                rule: Rule::V3,
                file: spec_name.clone(),
                line: 0,
                message: "no %changelog line in this spec".to_string(),
            });
        }
        match &spec.version {
            None => report.findings.push(Finding {
                rule: Rule::V8,
                file: spec_name.clone(),
                line: 0,
                message: "no Version: tag at line start in this spec".to_string(),
            }),
            Some(version) if version.split_whitespace().count() != 1 => {
                report.findings.push(Finding {
                    rule: Rule::V8,
                    file: spec_name.clone(),
                    line: 0,
                    message: format!("Version: {version:?} is not a single token"),
                });
            }
            _ => {}
        }
    }

    // V6: every `.changes` file matches a subpackage or is referenced
    // as a `Source:` in the spec. The Source: comparison is the V7
    // glob (`%{...}` is a wildcard), over the value's basename, so a
    // macro or URL source exempts its `.changes` file too.
    for changes_name in local.changes_names() {
        let name = stem(changes_name, ".changes");
        let named_as_source = source_values.iter().any(|value| {
            let base = value.rsplit('/').next().unwrap_or(value);
            source_value_matches(base, &format!("{name}.changes"))
                || source_value_matches(base, name)
        });
        if !package_names.contains(name) && !named_as_source {
            report.findings.push(Finding {
                rule: Rule::V6,
                file: changes_name.to_string(),
                line: 0,
                message: format!(
                    "{changes_name} matches no subpackage of the spec and is not a Source: in it"
                ),
            });
        }
    }

    // V7: every file in the directory is referenced in the spec as a
    // source or a patch. The spec and `.changes` files are the
    // package itself; `_`-prefixed files are service/link state.
    let referenced: BTreeSet<&str> = parsed
        .specs
        .values()
        .flat_map(|spec| {
            spec.sources
                .iter()
                .chain(spec.patches.iter())
                .map(String::as_str)
        })
        .collect();
    for file in local.files.keys() {
        if file.ends_with(".spec")
            || file.ends_with(".changes")
            || file.starts_with('_')
            || file.starts_with('.')
        {
            continue;
        }
        let named = referenced
            .iter()
            .any(|value| source_value_matches(value, file));
        if !named {
            report.findings.push(Finding {
                rule: Rule::V7,
                file: file.clone(),
                line: 0,
                message: "file is in the package directory but referenced by no Source: or Patch: in the spec"
                    .to_string(),
            });
        }
    }

    // V5 (file-global) and V11 (newest N entries), per `.changes` file.
    for (changes_name, file) in &parsed.changes {
        check_sequence(changes_name, file, &mut report);
        check_entry_format(changes_name, file, entries, &mut report);
    }

    // Target-relative rules. `target` and `target_parsed` are built
    // together, so the snapshot is present exactly when the parse is;
    // the match on the pair keeps that total instead of an expect in
    // a library path.
    match (target, target_parsed.as_ref()) {
        (None, _) | (_, None) => {
            for (rule, reason) in [
                (
                    Rule::V2,
                    "no target given — pass --target-project/--target-package to compare",
                ),
                (
                    Rule::V4,
                    "no target given — pass --target-project/--target-package to compare",
                ),
                (
                    Rule::V5,
                    "no target given — the newest-entry-vs-target check did not run",
                ),
                (
                    Rule::V11,
                    "no target given — the older-entries-unchanged check did not run",
                ),
            ] {
                report.skipped.push(Skipped {
                    rule,
                    reason: reason.to_string(),
                });
            }
        }
        (Some(target_snapshot), Some(target_parsed)) => {
            check_target_rules(
                local,
                &parsed,
                target_snapshot,
                target_parsed,
                entries,
                &mut report,
            );
        }
    }

    // V9, V10, V12: the newest entry of each `.changes` file — the
    // entry this submission adds.
    for (changes_name, file) in &parsed.changes {
        let Some(entry) = newest_entry(file) else {
            continue;
        };
        let spec_name = format!("{}.spec", stem(changes_name, ".changes"));
        let spec = parsed.specs.get(&spec_name);
        let target_spec = target_parsed
            .as_ref()
            .and_then(|parsed| parsed.specs.get(&spec_name));
        check_bare_update(changes_name, entry, spec, target_spec, options, &mut report);
        if let Some(spec) = spec {
            check_license(changes_name, entry, spec, target_spec, &mut report);
        }
        check_bug_refs(changes_name, entry, &mut report);
    }

    report
}

/// V5: headers parse as `Day Mon DD HH:MM:SS UTC YYYY` and are
/// strictly descending; the duplicated-header pair (a full-timestamp
/// header followed by a short `* Day Mon DD YYYY Name` header for the
/// same entry, same date and author) is an invalid date the source
/// validator rejects. A body line that merely quotes a date is not
/// the pair: the date and the author must match the entry's header.
fn check_sequence(changes_name: &str, file: &ChangesFile, report: &mut ValidationReport) {
    let mut last_timestamp: Option<(i64, usize)> = None;
    for entry in &file.entries {
        match &entry.header {
            None => report.findings.push(Finding {
                rule: Rule::V5,
                file: changes_name.to_string(),
                line: entry.header_line,
                message: format!(
                    "header does not parse as 'Day Mon DD HH:MM:SS UTC YYYY - Name <email>': {}",
                    entry.header_raw.trim()
                ),
            }),
            Some(header) => {
                if let Some((prev, prev_line)) = last_timestamp {
                    if header.timestamp >= prev {
                        report.findings.push(Finding {
                            rule: Rule::V5,
                            file: changes_name.to_string(),
                            line: entry.header_line,
                            message: format!(
                                "entry headers are not strictly descending (this entry is not older than the one at line {prev_line})"
                            ),
                        });
                    }
                }
                last_timestamp = Some((header.timestamp, entry.header_line));
            }
        }
        // The duplicated-header pair: a short `* Day Mon DD YYYY
        // Name` header line inside the body, matching this entry's
        // full header (same date and author).
        let header = entry.header.as_ref();
        for (line_no, line) in entry.body_nonblank() {
            let trimmed = line.trim_start();
            if let Some(after) = trimmed.strip_prefix("* ") {
                let mut tokens = after.split_whitespace();
                let weekday = tokens.next().unwrap_or("");
                let month = tokens.next().unwrap_or("");
                let day: Result<u32, _> = tokens.next().unwrap_or("").parse();
                let year: Result<i32, _> = tokens.next().unwrap_or("").parse();
                let author = after
                    .split_whitespace()
                    .skip(4)
                    .collect::<Vec<_>>()
                    .join(" ");
                let is_pair = match (header, day, year) {
                    (Some(header), Ok(day), Ok(year)) => {
                        let months = [
                            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct",
                            "Nov", "Dec",
                        ];
                        header.weekday == weekday
                            && header
                                .month
                                .checked_sub(1)
                                .and_then(|index| months.get(index as usize))
                                .is_some_and(|name| *name == month)
                            && header.day == day
                            && header.year == year
                            && !author.is_empty()
                            && header.author.starts_with(&author)
                    }
                    _ => false,
                };
                if is_pair {
                    report.findings.push(Finding {
                        rule: Rule::V5,
                        file: changes_name.to_string(),
                        line: line_no,
                        message: "short '* Day Mon DD YYYY Name' header after a full-timestamp header — the duplicated-header pair the source validator rejects as an invalid date"
                            .to_string(),
                    });
                }
            }
        }
    }
}

/// V11, newest N entries: the format port of `changes-lint.sh`
/// (separators, blank lines, bullets, line length, author form, no
/// request/staging/status mentions). Historical entries are out of
/// scope, as in the script — except the blank line closing the newest
/// entry, which sits on the boundary and is checked.
fn check_entry_format(
    changes_name: &str,
    file: &ChangesFile,
    entries: usize,
    report: &mut ValidationReport,
) {
    if file.entries.is_empty() {
        if file.lines.is_empty() {
            report.findings.push(Finding {
                rule: Rule::V11,
                file: changes_name.to_string(),
                line: 0,
                message: "empty .changes file — a submission carries an entry".to_string(),
            });
        } else {
            report.findings.push(Finding {
                rule: Rule::V11,
                file: changes_name.to_string(),
                line: 1,
                message: "first line is not a separator".to_string(),
            });
        }
        return;
    }
    for (k, entry) in file.entries.iter().take(entries).enumerate() {
        // Separator dash count.
        let sep = &file.lines[entry.separator_line - 1];
        if sep.len() != 67 || !sep.chars().all(|c| c == '-') {
            report.findings.push(Finding {
                rule: Rule::V11,
                file: changes_name.to_string(),
                line: entry.separator_line,
                message: format!(
                    "separator is {} dashes, must be exactly 67",
                    sep.trim_end().len()
                ),
            });
        }
        // Blank line after the header.
        if let Some((_, first)) = entry.body.first() {
            if !first.trim().is_empty() {
                report.findings.push(Finding {
                    rule: Rule::V11,
                    file: changes_name.to_string(),
                    line: entry.header_line + 1,
                    message: "missing blank line after the entry header".to_string(),
                });
            }
        } else {
            report.findings.push(Finding {
                rule: Rule::V11,
                file: changes_name.to_string(),
                line: entry.header_line,
                message: "entry has no body — describe the change".to_string(),
            });
        }
        // Blank line before the next separator (the boundary after
        // the newest entry is in scope even when the next entry is
        // not linted).
        let next_sep = file.entries.get(k + 1).map(|e| e.separator_line);
        if let Some(next_sep) = next_sep {
            if next_sep >= 2 {
                let before = &file.lines[next_sep - 2];
                if !before.trim().is_empty() {
                    report.findings.push(Finding {
                        rule: Rule::V11,
                        file: changes_name.to_string(),
                        line: next_sep - 1,
                        message: "missing blank line before the separator".to_string(),
                    });
                }
            }
        }
        // Author form: the date part is V5's; the `Name <email>`
        // form is V11's.
        if let Some(header) = &entry.header {
            if !header.author_is_full_form() {
                report.findings.push(Finding {
                    rule: Rule::V11,
                    file: changes_name.to_string(),
                    line: entry.header_line,
                    message: format!(
                        "bare email in author field — use 'Name <email>', not '{}'",
                        header.author
                    ),
                });
            }
        }
        // Body lines: bullets or continuations, no trailing
        // whitespace, wrapped at 67. Bullet levels are the `.changes`
        // convention: `- ` at indent 0, `* ` nested at indent 2
        // under it. `last_bullet` tracks the nearest valid bullet
        // (level, indent); an invalid bullet never becomes a parent,
        // so one bad bullet does not cascade onto the next.
        let mut last_bullet: Option<(u8, usize)> = None;
        for (line_no, line) in &entry.body {
            if line != &line.trim_end().to_string() {
                report.findings.push(Finding {
                    rule: Rule::V11,
                    file: changes_name.to_string(),
                    line: *line_no,
                    message: "trailing whitespace".to_string(),
                });
            }
            if line.trim().is_empty() {
                continue;
            }
            let trimmed = line.trim_start();
            let indent = line.len() - trimmed.len();
            if trimmed.starts_with("+ ") {
                report.findings.push(Finding {
                    rule: Rule::V11,
                    file: changes_name.to_string(),
                    line: *line_no,
                    message: "third-level '+' bullet — use '-' for the first level and '*' for the second only"
                        .to_string(),
                });
            } else if trimmed.starts_with("- ") {
                if indent == 0 {
                    last_bullet = Some((0, 0));
                } else {
                    report.findings.push(Finding {
                        rule: Rule::V11,
                        file: changes_name.to_string(),
                        line: *line_no,
                        message: format!(
                            "first-level '-' bullet is indented (indent {indent}) — top-level bullets start at column 0"
                        ),
                    });
                }
            } else if trimmed.starts_with("* ") {
                match last_bullet {
                    Some((0, 0)) | Some((1, 2)) if indent == 2 => {
                        last_bullet = Some((1, 2));
                    }
                    _ => {
                        let message = if indent == 0 {
                            "'*' bullet at the top level — use '-' for the first level and '*' only nested under it"
                                .to_string()
                        } else if last_bullet.is_none() {
                            "second-level '*' bullet has no '-' bullet above it".to_string()
                        } else {
                            "second-level '*' bullet must sit at indent 2 under a '-' bullet"
                                .to_string()
                        };
                        report.findings.push(Finding {
                            rule: Rule::V11,
                            file: changes_name.to_string(),
                            line: *line_no,
                            message,
                        });
                    }
                }
            } else if indent == 0 && !line.starts_with('\t') {
                report.findings.push(Finding {
                    rule: Rule::V11,
                    file: changes_name.to_string(),
                    line: *line_no,
                    message: format!(
                        "top-level line is neither bullet nor continuation: {:?}",
                        line.chars().take(50).collect::<String>()
                    ),
                });
            }
            // Line length: a word too long for 67 excuses the line
            // only when it is all of it.
            if line.chars().count() > 67 {
                let words: Vec<&str> = line.split_whitespace().collect();
                let lead = indent
                    + if words.first().is_some_and(|w| ["-", "*", "+"].contains(w)) {
                        2
                    } else {
                        0
                    };
                let text = if words.first().is_some_and(|w| ["-", "*", "+"].contains(w)) {
                    &words[1..]
                } else {
                    &words[..]
                };
                let single_long_word = text.len() == 1 && text[0].chars().count() + lead > 67;
                if !single_long_word {
                    report.findings.push(Finding {
                        rule: Rule::V11,
                        file: changes_name.to_string(),
                        line: *line_no,
                        message: format!(
                            "line is {} characters: wrap it at 67, the .changes convention",
                            line.chars().count()
                        ),
                    });
                }
            }
        }
        // No request numbers, staging orders, or review status in the
        // entry: those go in the SR message, not in the shipped file.
        for (line_no, text) in entry.body_nonblank() {
            if names_request(text) {
                report.findings.push(Finding {
                    rule: Rule::V11,
                    file: changes_name.to_string(),
                    line: line_no,
                    message: "bullet names a submit request or a staging order — put it in the SR message, not in .changes"
                        .to_string(),
                });
            } else if mentions_review_status(text) {
                report.findings.push(Finding {
                    rule: Rule::V11,
                    file: changes_name.to_string(),
                    line: line_no,
                    message: "bullet reports submission or review status — that goes in the SR message, not in .changes, which ships in the RPM"
                        .to_string(),
                });
            }
        }
    }
}

/// A bullet names an OBS request (`sr#N`, `SR N`, `rq N`,
/// `request #N` with 6+ digits — upstream "pull request #N" excepted)
/// or a staging order (`lands after`, `stage together`).
fn names_request(text: &str) -> bool {
    let lower = text.to_lowercase();
    if contains_ci(&lower, "lands after")
        || contains_ci(&lower, "lands before")
        || contains_ci(&lower, "landing after")
        || contains_ci(&lower, "stage together")
        || contains_ci(&lower, "stages together")
        || contains_ci(&lower, "staged together")
        || contains_ci(&lower, "goes in after")
        || contains_ci(&lower, "goes in before")
    {
        return true;
    }
    // `sr#123456` / `SR 123456` / `rq 123456` / `request #123456`.
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &lower[i..];
        let (prefix, advance) = if rest.starts_with("sr#")
            || rest.starts_with("sr ")
            || rest.starts_with("rq#")
            || rest.starts_with("rq ")
        {
            (true, 3)
        } else if rest.starts_with("request #") {
            (true, 9)
        } else if rest.starts_with("request ") {
            (true, 8)
        } else {
            (false, 1)
        };
        if prefix {
            let digits: String = rest[advance..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            // "pull request #N" is upstream's, not OBS's.
            let before = lower[..i].trim_end();
            let upstream = before.ends_with("pull") || before.ends_with("merge");
            if digits.len() >= 6 && !upstream {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn mentions_review_status(text: &str) -> bool {
    contains_ci(text, "factory-auto")
        || contains_ci(text, "source validator")
        || contains_ci(text, "source-validator")
        || contains_ci(text, "source_url validation")
        || contains_ci(text, "factory forward")
        || contains_ci(text, "forwarded to factory")
        || contains_ci(text, "forwarding to factory")
}

/// V2, V4, V5-target, V11-old-entries, and the entry count — the
/// rules that compare the local snapshot against the target.
#[allow(clippy::too_many_arguments)]
fn check_target_rules(
    local: &PackageSnapshot,
    parsed: &Parsed,
    target: &PackageSnapshot,
    target_parsed: &Parsed,
    entries: usize,
    report: &mut ValidationReport,
) {
    // V2: at least one `.changes` file differs from the target
    // (byte-compare; a new or deleted file counts as changed).
    let mut any_changes_differs = false;
    let mut all_changes: BTreeSet<&str> = BTreeSet::new();
    all_changes.extend(local.changes_names());
    all_changes.extend(target.changes_names());
    for name in &all_changes {
        let local_text = local.files.get(*name);
        let target_text = target.files.get(*name);
        if local_text != target_text {
            any_changes_differs = true;
        }
    }
    if !any_changes_differs {
        report.findings.push(Finding {
            rule: Rule::V2,
            file: local
                .changes_names()
                .first()
                .copied()
                .map(str::to_string)
                .or_else(|| {
                    local
                        .spec_names()
                        .first()
                        .map(|spec| format!("{}.changes", stem(spec, ".spec")))
                })
                .unwrap_or_default(),
            line: 0,
            message:
                "no .changes file differs from the target — every submit request carries an entry"
                    .to_string(),
        });
    }

    // The target package does not exist at all: a new package, and
    // the diff-based rules have nothing to compare against.
    let target_exists = !target.files.is_empty();
    if !target_exists {
        for (rule, reason) in [
            (
                Rule::V4,
                "target package does not exist (new package) — factory-auto skips this check",
            ),
            (
                Rule::V5,
                "target package does not exist (new package) — no target newest entry",
            ),
            (
                Rule::V11,
                "target package does not exist (new package) — no older entries to compare",
            ),
        ] {
            report.skipped.push(Skipped {
                rule,
                reason: reason.to_string(),
            });
        }
        return;
    }

    // V4: every patch added or removed appears as a substring in an
    // added or removed line of the `.changes` diff. Exempt when the
    // filename appears as a `Source:` value in either spec. This is
    // the `changes-patches.sh` algorithm against the target
    // snapshot (not the checkout's last commit).
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
    let mut to_mention: BTreeMap<&str, &str> = BTreeMap::new();
    for patch in old_patches.difference(&new_patches) {
        to_mention.insert(patch, "deleted");
    }
    for patch in new_patches.difference(&old_patches) {
        to_mention.insert(patch, "added");
    }
    if !to_mention.is_empty() {
        // The +/- lines of the `.changes` diffs.
        let mut plus_minus: Vec<String> = Vec::new();
        let mut all_changes: BTreeSet<&str> = BTreeSet::new();
        all_changes.extend(local.changes_names());
        all_changes.extend(target.changes_names());
        for name in &all_changes {
            let old_lines: Vec<String> = target
                .files
                .get(*name)
                .map(|text| text.lines().map(str::to_string).collect())
                .unwrap_or_default();
            let new_lines: Vec<String> = local
                .files
                .get(*name)
                .map(|text| text.lines().map(str::to_string).collect())
                .unwrap_or_default();
            let (added, removed) = diff_added_removed(&old_lines, &new_lines);
            plus_minus.extend(added.into_iter().map(|line| line.trim().to_string()));
            plus_minus.extend(removed.into_iter().map(|line| line.trim().to_string()));
        }
        for patch in to_mention.keys() {
            if !plus_minus.iter().any(|line| line.contains(patch)) {
                let verb = if to_mention[patch] == "added" {
                    "added"
                } else {
                    "deleted"
                };
                let noun = if to_mention[patch] == "added" {
                    "addition"
                } else {
                    "removal"
                };
                // The Source: exemption (either spec). The
                // comparison is the V7/V6 glob (`%{...}` is a
                // wildcard, over the value's basename), not exact
                // equality: a patch carried as a macro or URL source
                // is exempt too.
                let exempt = parsed
                    .specs
                    .values()
                    .chain(target_parsed.specs.values())
                    .flat_map(|spec| spec.sources.iter())
                    .any(|value| source_value_matches(value.trim(), patch));
                if !exempt {
                    let glued: String = plus_minus.concat();
                    let hint = if glued.contains(patch) {
                        " (it is there, but wrapped across lines — keep the filename on one line)"
                    } else {
                        ""
                    };
                    report.findings.push(Finding {
                        rule: Rule::V4,
                        file: local
                            .changes_names()
                            .first()
                            .copied()
                            .unwrap_or("")
                            .to_string(),
                        line: 0,
                        message: format!(
                            "A patch ({patch}) is being {verb} without this {noun} being mentioned in the changelog.{hint}"
                        ),
                    });
                }
            }
        }
    }

    // V5 target-relative: the newest local entry must be newer than
    // the target's newest entry; a stale-base filing surfaces as a
    // sequence failure, and re-filing is the fix.
    for (name, file) in &parsed.changes {
        let Some(local_newest) = newest_timestamp(file) else {
            continue;
        };
        if let Some(target_file) = target_parsed.changes.get(name) {
            if let Some(target_newest) = newest_timestamp(target_file) {
                if local_newest <= target_newest {
                    report.findings.push(Finding {
                        rule: Rule::V5,
                        file: name.clone(),
                        line: file.entries.first().map(|e| e.header_line).unwrap_or(1),
                        message: "newest local entry is not newer than the target's newest entry — re-file against the current target"
                            .to_string(),
                    });
                }
            }
        }
    }

    // V11: the number of new entries, and the older entries
    // byte-identical to the target's. Sanctioned exceptions (a typo
    // fix, an added bug reference, a header the validator itself
    // rejects) are for the human to judge — the finding names them.
    for (name, file) in &parsed.changes {
        let Some(target_file) = target_parsed.changes.get(name) else {
            continue;
        };
        let local_count = file.entries.len();
        let target_count = target_file.entries.len();
        if local_count < target_count {
            report.findings.push(Finding {
                rule: Rule::V11,
                file: name.clone(),
                line: 0,
                message: format!(
                    "entries removed vs the target ({local_count} local, {target_count} target) — never alter or delete a previous entry"
                ),
            });
            continue;
        }
        let new_entries = local_count - target_count;
        if new_entries > entries {
            report.findings.push(Finding {
                rule: Rule::V11,
                file: name.clone(),
                line: 0,
                message: format!(
                    "{new_entries} new entries vs the target, expected at most {entries} — fold a fix-up into the top entry (one entry per SR); pass --entries {new_entries} for deliberate per-version entries"
                ),
            });
        }
        for (i, old_entry) in file.entries.iter().skip(new_entries).enumerate() {
            let target_entry = &target_file.entries[i];
            if old_entry.raw(&file.lines) != target_entry.raw(&target_file.lines) {
                report.findings.push(Finding {
                    rule: Rule::V11,
                    file: name.clone(),
                    line: old_entry.separator_line,
                    message: "an older entry differs from the target — never alter a previous entry (sanctioned exceptions only: a typo fix, an added bug reference, or a header the source validator itself rejects)"
                        .to_string(),
                });
                break;
            }
        }
    }
}

/// A bullet that only names a mechanical file, dependency, or
/// license operation — it says nothing about what the new version
/// brings upstream, so it never counts as substantive detail for V9.
/// Mirrors the lines `generate` emits: patch add/drop (G2), source
/// add/drop (G4), dependency add/drop (G3), license change (G5).
/// A file add/drop is a single filename token (it carries an
/// extension); a multi-word addition is a real statement.
pub(crate) fn is_mechanical_bullet(text: &str) -> bool {
    let trimmed = text.trim_start();
    if let Some(rest) = trimmed
        .strip_prefix("- Add ")
        .or_else(|| trimmed.strip_prefix("- Drop "))
    {
        let rest = rest.trim();
        if !rest.contains(char::is_whitespace) && rest.contains('.') {
            return true;
        }
    }
    trimmed.starts_with("- Add Requires:")
        || trimmed.starts_with("- Drop Requires:")
        || trimmed.starts_with("- Add BuildRequires:")
        || trimmed.starts_with("- Drop BuildRequires:")
        || trimmed.starts_with("- Change License:")
}

/// A line carrying an unfilled generation slot, in one of the shapes
/// `generate` emits: a `reason: TODO` sub-bullet, or a `TODO: ...`
/// header. A bare word `TODO` quoted from upstream text is not a
/// slot — the shapes are what the generator writes.
fn is_unfilled_slot(line: &str) -> bool {
    line.contains("reason: TODO") || line.contains("TODO:")
}

/// V9: a version update carries more than a bare version line when
/// upstream news exists for the new version. The upstream-news
/// condition is load-bearing: when no NEWS section was found, the
/// rule does not flag (upstream may maintain no changelog at all).
/// When no tarball was on disk at all, the skip says so instead:
/// the line was never checked, not merely unflaggable. A failed
/// lookup is reported once by the caller; V9 stays silent rather
/// than adding a second skip.
/// A draft marker left by `generate` (`TODO` slots) is also a
/// needs-detail finding: the human step has not run yet.
fn check_bare_update(
    changes_name: &str,
    entry: &Entry,
    spec: Option<&SpecInfo>,
    target_spec: Option<&SpecInfo>,
    options: &ValidateOptions<'_>,
    report: &mut ValidationReport,
) {
    let bullets = entry.bullets();
    let update = |text: &str| {
        let lower = text.to_lowercase();
        lower.starts_with("- update to") || lower.starts_with("- updated to")
    };
    let has_update = bullets.iter().any(|(_, text, _)| update(text));
    if !has_update {
        return;
    }
    // The version moved (target) or the entry is an update line at
    // all (local-only: the newest entry is presumed this SR's).
    let version_moved = match (spec, target_spec) {
        (Some(spec), Some(target)) => spec.version != target.version,
        _ => true,
    };
    if !version_moved {
        return;
    }
    let other: Vec<&(usize, String, usize)> = bullets
        .iter()
        .filter(|(_, text, _)| !update(text) && !is_mechanical_bullet(text))
        .collect();
    let states_no_change = entry.body_nonblank().iter().any(|(_, line)| {
        contains_ci(line, "no user-visible change")
            || contains_ci(line, "no user visible change")
            || contains_ci(line, "no visible change")
    });
    // A leftover generation slot is never a finished entry.
    let has_todo_slot = entry
        .body_nonblank()
        .iter()
        .any(|(_, line)| is_unfilled_slot(line));
    if has_todo_slot {
        report.findings.push(Finding {
            rule: Rule::V9,
            file: changes_name.to_string(),
            line: entry.bullets().first().map(|(n, _, _)| *n).unwrap_or(entry.header_line),
            message: "entry still carries unfilled generation slots (TODO) — fill in the reasons and the upstream highlights before filing"
                .to_string(),
        });
        return;
    }
    if other.is_empty() && !states_no_change {
        match options.news_section {
            Some(section) if !section.trim().is_empty() => {
                report.findings.push(Finding {
                    rule: Rule::V9,
                    file: changes_name.to_string(),
                    line: entry.bullets().first().map(|(n, _, _)| *n).unwrap_or(entry.header_line),
                    message: "bare 'Update to <version>' with no summary of the upstream changes — upstream news exists for the new version; add substantive bullets, or state '* No user-visible changes'"
                        .to_string(),
                });
            }
            _ => {
                // A failed lookup is reported once by the caller;
                // V9 adds no second, contradictory skip for it.
                if !options.news_lookup_failed {
                    let version = spec.and_then(|s| s.version.clone()).unwrap_or_default();
                    // No NEWS section: the advisory differs by
                    // whether a tarball was there to check at all.
                    // A spec with no version never had a tarball to
                    // look for, whatever another spec's lookup found.
                    let reason = if version.is_empty() {
                        "the spec carries no version, so the bare update line was not checked against upstream NEWS"
                            .to_string()
                    } else if options.tarball_present {
                        format!(
                            "no upstream NEWS section found for version {version} — nothing to distill, so the bare update line is not flagged"
                        )
                    } else {
                        format!(
                            "no source tarball on disk for version {version} — the bare update line was not checked against upstream NEWS"
                        )
                    };
                    report.skipped.push(Skipped {
                        rule: Rule::V9,
                        reason,
                    });
                }
            }
        }
    }
}

/// V10: a license named in the entry matches the spec `License:`
/// tag; a license change in the entry without a matching spec change
/// fails. License tokens are recognised two ways: a token in a line
/// that names a license (`Change LICENSE: GPLv3 -> SEGV-1.1`), or a
/// token that is itself a known license id. Normalisation maps the
/// common spellings to one form (`GPLv3` → `GPL-3.0`, `-only` /
/// `-or-later` stripped), so the comparison is on the license, not
/// the spelling.
fn check_license(
    changes_name: &str,
    entry: &Entry,
    spec: &SpecInfo,
    target_spec: Option<&SpecInfo>,
    report: &mut ValidationReport,
) {
    let Some(spec_license) = &spec.license else {
        return;
    };
    let spec_tokens = license_tokens_in_tag(spec_license);
    let mut flagged: BTreeSet<String> = BTreeSet::new();
    for (line_no, text) in entry.body_nonblank() {
        let mentions_license = contains_ci(text, "license");
        for token in entry_tokens(text) {
            let Some(normalized) = normalize_license(&token) else {
                continue;
            };
            if !mentions_license && !is_known_license(&normalized) {
                continue;
            }
            if !spec_tokens.contains(&normalized) && !flagged.contains(&normalized) {
                flagged.insert(normalized.clone());
                report.findings.push(Finding {
                    rule: Rule::V10,
                    file: changes_name.to_string(),
                    line: line_no,
                    message: format!(
                        "entry names license {normalized}, which the spec License: tag ({spec_license}) does not carry — a license change in the entry needs the matching spec change"
                    ),
                });
            }
        }
    }
    // The change direction, only when no token finding fired above:
    // the entry claims a license change while the spec `License:`
    // is unchanged vs the target.
    if flagged.is_empty() {
        if let Some(target_spec) = target_spec {
            if spec.license == target_spec.license {
                for (line_no, text) in entry.body_nonblank() {
                    let lower = text.to_lowercase();
                    // "change" as a word: "changes" claims a change,
                    // "exchange" and "unchanged" do not. A line that
                    // states there is no change is not a claim.
                    let claims_change = lower.contains("->")
                        || contains_word(&lower, "change")
                        || contains_word(&lower, "changes");
                    let negated = contains_word(&lower, "unchanged") || lower.contains("no change");
                    if lower.contains("license") && claims_change && !negated {
                        report.findings.push(Finding {
                            rule: Rule::V10,
                            file: changes_name.to_string(),
                            line: line_no,
                            message: "entry claims a license change but the spec License: tag is unchanged vs the target"
                                .to_string(),
                        });
                        break;
                    }
                }
            }
        }
    }
}

/// Tokens of a text: runs of letters, digits, `+`, `-`, `.`.
fn entry_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '+' || ch == '-' || ch == '.' {
            current.push(ch);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Normalise a license spelling to one form, or `None` when the
/// token does not look like a license id at all.
fn normalize_license(token: &str) -> Option<String> {
    let upper = token.to_uppercase();
    let base = upper
        .strip_suffix("-ONLY")
        .or_else(|| upper.strip_suffix("-OR-LATER"))
        .unwrap_or(&upper);
    let mapped = match base {
        "GPLV2" => "GPL-2.0",
        "GPLV3" => "GPL-3.0",
        "LGPLV2" => "LGPL-2.0",
        "LGPLV2.1" => "LGPL-2.1",
        "LGPLV3" => "LGPL-3.0",
        "AGPLV3" => "AGPL-3.0",
        "GPL-2" => "GPL-2.0",
        "GPL-3" => "GPL-3.0",
        "LGPL-2" => "LGPL-2.0",
        "LGPL-3" => "LGPL-3.0",
        "AGPL-1" => "AGPL-1.0",
        "AGPL-3" => "AGPL-3.0",
        other => other,
    };
    // A license id carries a letter and a digit, or is a known short
    // name. A bare version number ("2.0") is neither.
    let known_short = ["MIT", "ISC", "ZLIB", "BSD", "WTFPL", "UNLICENSE", "APACHE"];
    if (mapped.chars().any(|c| c.is_ascii_alphabetic())
        && mapped.chars().any(|c| c.is_ascii_digit()))
        || known_short.contains(&mapped)
    {
        Some(mapped.to_string())
    } else {
        None
    }
}

fn is_known_license(normalized: &str) -> bool {
    let known = [
        "GPL-2.0",
        "GPL-3.0",
        "LGPL-2.0",
        "LGPL-2.1",
        "LGPL-3.0",
        "AGPL-1.0",
        "AGPL-3.0",
        "MIT",
        "ISC",
        "ZLIB",
        "BSD",
        "BSD-2-CLAUSE",
        "BSD-3-CLAUSE",
        "APACHE-2.0",
        "MPL-2.0",
        "MPL-1.1",
        "ARTISTIC-2.0",
        "CC-BY-4.0",
        "CC0-1.0",
        "EPL-2.0",
        "EUPL-1.2",
        "OPENSSL",
        "PYTHON-2.0",
        "PHP-3.0",
        "PHP-3.01",
        "WTFPL",
        "UNLICENSE",
        "BSL-1.0",
        "CDDL-1.0",
    ];
    known.contains(&normalized)
}

/// The license ids a `License:` tag carries, normalised.
fn license_tokens_in_tag(tag: &str) -> BTreeSet<String> {
    tag.split(|c: char| c == '&' || c == '|' || c == '(' || c == ')' || c.is_whitespace())
        .filter(|token| {
            let upper = token.to_uppercase();
            upper != "AND" && upper != "OR" && upper != "WITH" && !token.is_empty()
        })
        .filter_map(normalize_license)
        .collect()
}

/// V12: bug references use the tracker abbreviations (`boo#`,
/// `bsc#`, `CVE-...`). Simplified port of `changes-lint.sh`: the
/// full script's not-affected attribution (which id a phrase names,
/// which sub-bullets a header lists) stays in the skill — this check
/// flags the shapes that need no attribution to decide.
fn check_bug_refs(changes_name: &str, entry: &Entry, report: &mut ValidationReport) {
    for (line_no, text) in entry.groups() {
        // `bnc#`: the old SUSE bugzilla abbreviation — use
        // `boo#`/`bsc#` instead.
        for id in bug_refs(&text, "bnc") {
            report.findings.push(Finding {
                rule: Rule::V12,
                file: changes_name.to_string(),
                line: line_no,
                message: format!(
                    "bug reference {id} uses the old bnc# abbreviation — use boo# (openSUSE) or bsc# (SUSE)"
                ),
            });
        }
        // Incomplete CVE ids.
        for partial in partial_cve_ids(&text) {
            report.findings.push(Finding {
                rule: Rule::V12,
                file: changes_name.to_string(),
                line: line_no,
                message: format!(
                    "incomplete CVE id ({partial}) — cite the full CVE-YYYY-NNNN, or leave it out"
                ),
            });
        }
        // A bare `CVE`/`CVEs` or a security claim with no full CVE id
        // in the group. A bug ref or GHSA stands in for a security
        // claim, never for bare `CVEs`; `CVE pending` claims nothing,
        // and neither does a negated claim (`no security fixes`).
        let has_full_cve = !full_cve_ids(&text).is_empty();
        let has_ref = !bug_refs(&text, "boo").is_empty()
            || !bug_refs(&text, "bsc").is_empty()
            || !bug_refs(&text, "bnc").is_empty()
            || text.to_uppercase().contains("GHSA-");
        let lower = text.to_lowercase();
        let bare_cve = contains_word(&lower, "cve") || contains_word(&lower, "cves");
        let security_claim = [
            "security fix",
            "security fixes",
            "security issue",
            "security issues",
            "security bug",
            "security bugs",
            "security problem",
            "security flaw",
            "security update",
            "security release",
            "vulnerability",
            "vulnerabilities",
        ]
        .iter()
        .any(|phrase| {
            let mut start = 0;
            while let Some(idx) = lower[start..].find(phrase) {
                let pos = start + idx;
                let before = &lower[..pos];
                let negated = before.ends_with("no ")
                    || before.ends_with("non-")
                    || before.ends_with("non ")
                    || before.ends_with("not a ")
                    || before.ends_with("without ");
                if !negated {
                    return true;
                }
                start = pos + phrase.len();
            }
            false
        });
        if bare_cve && !has_full_cve && !lower.contains("cve pending") {
            report.findings.push(Finding {
                rule: Rule::V12,
                file: changes_name.to_string(),
                line: line_no,
                message: "bullet says 'CVE'/'CVEs' without a full CVE id — cite each CVE-YYYY-NNNN, or describe the fix itself"
                    .to_string(),
            });
        } else if security_claim && !has_full_cve && !has_ref {
            report.findings.push(Finding {
                rule: Rule::V12,
                file: changes_name.to_string(),
                line: line_no,
                message: "bullet claims security fixes without a full CVE id — cite each CVE-YYYY-NNNN (with its boo#/bsc#), or describe the fix itself without the security claim"
                    .to_string(),
            });
        }
        // A CVE id named as not fixed, or cited for an issue the
        // entry says is not affected: every id in `.changes` reads
        // as fixed here, so the assessment belongs in the bug.
        let ids: Vec<String> = full_cve_ids(&text)
            .into_iter()
            .chain(bug_refs(&text, "boo"))
            .chain(bug_refs(&text, "bsc"))
            .chain(bug_refs(&text, "bnc"))
            .collect();
        if !ids.is_empty() {
            let not_fixed = NOT_FIXED.iter().any(|phrase| lower.contains(phrase));
            let not_affected = NOT_AFFECTED.iter().any(|phrase| lower.contains(phrase));
            // A fix statement is a fix verb that survives the
            // not-fixed / not-affected phrases being stripped: "is
            // not fixed yet" contains "fixed" as a substring, so a
            // plain substring search would read it as a fix. The
            // `- CVE-` carve-out (a fix laid out as a bare id line)
            // applies only when the group states no not-fixed /
            // not-affected phrase — `- CVE-… is not affected` is the
            // not-affected statement, not a fix.
            let plain = strip_phrases(&lower, &NOT_FIXED);
            let plain = strip_phrases(&plain, &NOT_AFFECTED);
            let states_a_fix = [
                "fix",
                "fixes",
                "fixed",
                "fixing",
                "address",
                "addresses",
                "addressed",
                "resolves",
                "resolved",
                "closes",
                "closed",
            ]
            .iter()
            .any(|verb| contains_word(&plain, verb))
                || ((text.starts_with("- CVE-") || text.starts_with("* CVE-"))
                    && !not_fixed
                    && !not_affected);
            if (not_fixed || not_affected) && !states_a_fix {
                report.findings.push(Finding {
                    rule: Rule::V12,
                    file: changes_name.to_string(),
                    line: line_no,
                    message: format!(
                        "bullet cites {} for an issue it says is not fixed / not affected — the trackers read any CVE/boo#/bsc#/bnc# in .changes as fixed here; put the assessment in the bug comment",
                        ids.join(", ")
                    ),
                });
            }
        }
    }
}

/// Full `CVE-YYYY-NNNN` ids in a text (4+ digits in the second group).
fn full_cve_ids(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let upper = text.to_uppercase();
    let bytes = upper.as_bytes();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if upper[i..].starts_with("CVE-") {
            let after = &upper[i + 4..];
            let year: String = after.chars().take_while(char::is_ascii_digit).collect();
            if year.len() == 4 {
                let rest = &after[4..];
                if let Some(number) = rest.strip_prefix('-') {
                    let digits: String = number.chars().take_while(char::is_ascii_digit).collect();
                    if digits.len() >= 4 {
                        ids.push(format!("CVE-{year}-{digits}"));
                        i += 4 + 4 + 1 + digits.len();
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    ids
}

/// `CVE-` occurrences that are not full ids.
fn partial_cve_ids(text: &str) -> Vec<String> {
    let full = full_cve_ids(text);
    let mut partials = Vec::new();
    let upper = text.to_uppercase();
    let mut i = 0;
    while i + 4 <= upper.len() {
        if upper[i..].starts_with("CVE-") {
            let token: String = upper[i + 4..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == 'X' || *c == 'x')
                .collect();
            let id = format!("CVE-{token}");
            if !full.iter().any(|full| full == &id) && !partials.contains(&id) {
                partials.push(id);
            }
            i += 4 + token.len();
        } else {
            i += 1;
        }
    }
    partials
}

/// `boo#123456` style references for one tracker prefix.
fn bug_refs(text: &str, prefix: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut ids = Vec::new();
    let needle = format!("{prefix}#");
    let mut start = 0;
    while let Some(idx) = lower[start..].find(&needle) {
        let pos = start + idx;
        let after = &lower[pos + needle.len()..];
        let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() {
            ids.push(format!("{prefix}#{digits}"));
        }
        start = pos + needle.len();
    }
    ids
}

/// Phrases that state an issue is not fixed here.
const NOT_FIXED: [&str; 17] = [
    "not fixed",
    "not yet fixed",
    "not addressed",
    "not yet addressed",
    "unfixed",
    "unpatched",
    "not patched",
    "still vulnerable",
    "still affected",
    "tracked separately",
    "does not fix",
    "did not fix",
    "do not fix",
    "doesn't fix",
    "remains open",
    "remains unfixed",
    "remains vulnerable",
];

/// Phrases that state an issue does not affect this package.
const NOT_AFFECTED: [&str; 9] = [
    "not affected",
    "is not affected by",
    "unaffected",
    "not vulnerable",
    "not impacted",
    "above the fixed version",
    "above the patched version",
    "already fixed upstream",
    "already fixed in",
];

/// A text with every occurrence of each phrase replaced by a space.
fn strip_phrases(text: &str, phrases: &[&str]) -> String {
    let mut out = text.to_string();
    for phrase in phrases {
        out = out.replace(phrase, " ");
    }
    out
}

/// A whole word in a lowercased text.
fn contains_word(lower: &str, word: &str) -> bool {
    let mut start = 0;
    while let Some(idx) = lower[start..].find(word) {
        let pos = start + idx;
        let before_ok = pos == 0 || !lower.as_bytes()[pos - 1].is_ascii_alphanumeric();
        let end = pos + word.len();
        let after_ok = end >= lower.len() || !lower.as_bytes()[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        start = pos + word.len();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(files: &[(&str, &str)]) -> PackageSnapshot {
        PackageSnapshot {
            files: files
                .iter()
                .map(|(name, content)| ((*name).to_string(), (*content).to_string()))
                .collect(),
        }
    }

    fn header(day: &str) -> String {
        format!("{day} - Jane Packager <jane@example.com>")
    }

    fn changes_file(newest_body: &str, older: &str) -> String {
        let sep = "-".repeat(67);
        format!(
            "{sep}\n{}\n\n{}\n\n{sep}\n{}\n\n{}\n\n",
            header("Wed May 27 16:31:48 UTC 2026"),
            newest_body,
            header("Tue May 26 10:00:00 UTC 2026"),
            older
        )
    }

    const SPEC: &str = "Name:           foo\nVersion:        2.0\nLicense:        GPL-3.0-only\nSource0:        foo-2.0.tar.gz\nPatch0:         fix-crash.patch\n%changelog\n";

    #[test]
    fn clean_package_passes() {
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file(
                    "- Update to 2.0\n  * Fix a crash (boo#123456)",
                    "- Initial package",
                ),
            ),
            ("foo-2.0.tar.gz", "binary"),
            ("fix-crash.patch", "diff"),
        ]);
        let report = validate(&local, None, &ValidateOptions::default());
        assert!(report.passed(), "findings: {:?}", report.findings);
        // Local-only mode skips the target-relative rules.
        assert_eq!(report.skipped.len(), 4);
    }

    #[test]
    fn v1_flags_a_spec_without_a_changes_file() {
        let local = snapshot(&[("foo.spec", SPEC)]);
        let report = validate(&local, None, &ValidateOptions::default());
        assert!(report.findings.iter().any(|f| f.rule == Rule::V1));
    }

    #[test]
    fn v3_and_v8_flag_a_bare_spec() {
        let local = snapshot(&[("foo.spec", "Name: foo\n"), ("foo.changes", "")]);
        let report = validate(&local, None, &ValidateOptions::default());
        assert!(report.findings.iter().any(|f| f.rule == Rule::V3));
        assert!(report.findings.iter().any(|f| f.rule == Rule::V8));
    }

    #[test]
    fn v5_flags_out_of_sequence_and_bad_dates() {
        let sep = "-".repeat(67);
        let text = format!(
            "{sep}\n{}\n\n- Update to 2.0\n\n{sep}\n{}\n\n- Initial package\n\n",
            header("Tue May 26 10:00:00 UTC 2026"),
            header("Wed May 27 16:31:48 UTC 2026"),
        );
        let local = snapshot(&[("foo.spec", SPEC), ("foo.changes", &text)]);
        let report = validate(&local, None, &ValidateOptions::default());
        assert!(report.findings.iter().any(|f| f.rule == Rule::V5));
    }

    #[test]
    fn v5_flags_the_duplicated_header_pair() {
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file(
                    "- Update to 2.0\n  * Wed May 27 2026 Jane Packager",
                    "- Initial package",
                ),
            ),
        ]);
        let report = validate(&local, None, &ValidateOptions::default());
        assert!(report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V5 && f.message.contains("duplicated-header")));
    }

    #[test]
    fn v6_flags_a_changes_file_for_no_subpackage() {
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file("- Update to 2.0", "- Initial package"),
            ),
            (
                "foo-doc.changes",
                &changes_file("- Update to 2.0", "- Initial package"),
            ),
        ]);
        let report = validate(&local, None, &ValidateOptions::default());
        assert!(report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V6 && f.file == "foo-doc.changes"));
    }

    #[test]
    fn v7_flags_an_unreferenced_file() {
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file("- Update to 2.0", "- Initial package"),
            ),
            ("foo-2.0.tar.gz", "binary"),
            ("fix-crash.patch", "diff"),
            ("stray.txt", "nothing references me"),
        ]);
        let report = validate(&local, None, &ValidateOptions::default());
        assert!(report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V7 && f.file == "stray.txt"));
    }

    #[test]
    fn v11_flags_format_defects_in_the_newest_entry() {
        let sep = "-".repeat(67);
        // 66 dashes, no blank line after the header, a 70-char line,
        // a `+` bullet, and a bare prose line.
        let bad_sep = "-".repeat(66);
        let text = format!(
            "{bad_sep}\n{}\n- Update to 2.0, a very long line that runs past sixty-seven characters for sure\n  + third level\nbare prose\n\n{sep}\n{}\n\n- Initial package\n\n",
            header("Wed May 27 16:31:48 UTC 2026"),
            header("Tue May 26 10:00:00 UTC 2026"),
        );
        let local = snapshot(&[("foo.spec", SPEC), ("foo.changes", &text)]);
        let report = validate(&local, None, &ValidateOptions::default());
        let v11: Vec<&Finding> = report
            .findings
            .iter()
            .filter(|f| f.rule == Rule::V11)
            .collect();
        assert!(v11.iter().any(|f| f.message.contains("66 dashes")));
        assert!(v11.iter().any(|f| f.message.contains("blank line after")));
        assert!(v11.iter().any(|f| f.message.contains("67")));
        assert!(v11.iter().any(|f| f.message.contains("third-level")));
        assert!(v11.iter().any(|f| f.message.contains("neither bullet")));
    }

    #[test]
    fn v11_flags_a_request_number_in_the_entry() {
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file(
                    "- Update to 2.0 (lands after sr#1385697)",
                    "- Initial package",
                ),
            ),
        ]);
        let report = validate(&local, None, &ValidateOptions::default());
        assert!(report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V11 && f.message.contains("submit request")));
    }

    #[test]
    fn v9_flags_a_bare_update_when_news_exists() {
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file("- Update to 2.0", "- Initial package"),
            ),
        ]);
        let options = ValidateOptions {
            entries: 1,
            news_section: Some("## 2.0\n\n- Fix a crash\n"),
        };
        let report = validate(&local, None, &options);
        assert!(report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V9 && f.message.contains("bare 'Update")));
    }

    #[test]
    fn v9_does_not_flag_when_no_news_exists() {
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file("- Update to 2.0", "- Initial package"),
            ),
        ]);
        let report = validate(&local, None, &ValidateOptions::default());
        assert!(!report.findings.iter().any(|f| f.rule == Rule::V9));
        assert!(report.skipped.iter().any(|s| s.rule == Rule::V9));
    }

    #[test]
    fn v9_passes_when_the_entry_states_no_visible_changes() {
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file(
                    "- Update to 2.0\n  * No user-visible changes",
                    "- Initial package",
                ),
            ),
        ]);
        let options = ValidateOptions {
            entries: 1,
            news_section: Some("## 2.0\n\n- Fix a crash\n"),
        };
        let report = validate(&local, None, &options);
        assert!(!report.findings.iter().any(|f| f.rule == Rule::V9));
    }

    #[test]
    fn v10_flags_a_license_the_spec_does_not_carry() {
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file(
                    "- Update to 2.0\n  * Change LICENSE: GPLv3 -> SEGV-1.1",
                    "- Initial package",
                ),
            ),
        ]);
        let report = validate(&local, None, &ValidateOptions::default());
        assert!(report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V10 && f.message.contains("SEGV-1.1")));
        // GPLv3 normalises to GPL-3.0, which the spec carries.
        assert!(!report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V10 && f.message.contains("GPL-3.0,")));
    }

    #[test]
    fn v12_flags_bnc_incomplete_cve_and_bare_cve_claims() {
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file(
                    "- Update to 2.0\n  * Fix a crash (bnc#123456)\n  * Security fixes, see CVE-2026\n  * Fixes CVEs",
                    "- Initial package",
                ),
            ),
        ]);
        let report = validate(&local, None, &ValidateOptions::default());
        let v12: Vec<&Finding> = report
            .findings
            .iter()
            .filter(|f| f.rule == Rule::V12)
            .collect();
        assert!(v12.iter().any(|f| f.message.contains("bnc#")));
        assert!(v12.iter().any(|f| f.message.contains("incomplete CVE")));
        assert!(v12
            .iter()
            .any(|f| f.message.contains("without a full CVE id")));
    }

    #[test]
    fn v12_flags_a_cve_named_as_not_fixed() {
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file(
                    "- Update to 2.0\n  * CVE-2026-12345 is not fixed yet, tracked separately",
                    "- Initial package",
                ),
            ),
        ]);
        let report = validate(&local, None, &ValidateOptions::default());
        assert!(report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V12 && f.message.contains("not fixed / not affected")));
    }

    #[test]
    fn target_rules_v2_v4_v5() {
        let target_spec = SPEC.replace("Version:        2.0", "Version:        1.0");
        let target = snapshot(&[
            ("foo.spec", &target_spec),
            (
                "foo.changes",
                &changes_file("- Update to 1.0", "- Initial package"),
            ),
            ("foo-1.0.tar.gz", "binary"),
            ("old.patch", "diff"),
        ]);
        // Local: version bump, old patch dropped, new patch added but
        // not mentioned; .changes identical to the target.
        let local_spec = SPEC.replace(
            "Patch0:         fix-crash.patch",
            "Patch0:         new.patch",
        );
        let local = snapshot(&[
            ("foo.spec", &local_spec),
            (
                "foo.changes",
                &changes_file("- Update to 1.0", "- Initial package"),
            ),
            ("foo-2.0.tar.gz", "binary"),
            ("new.patch", "diff"),
        ]);
        let report = validate(&local, Some(&target), &ValidateOptions::default());
        // V2: no .changes differs. V4: old.patch dropped and
        // new.patch added, neither named.
        assert!(report.findings.iter().any(|f| f.rule == Rule::V2));
        let v4: Vec<&Finding> = report
            .findings
            .iter()
            .filter(|f| f.rule == Rule::V4)
            .collect();
        assert_eq!(v4.len(), 2);
        // V5 target-relative: the newest local entry is not newer
        // than the target's (same timestamp).
        assert!(report.findings.iter().any(|f| f.rule == Rule::V5));
    }

    #[test]
    fn target_rules_v4_passes_when_patches_are_named() {
        let target_spec = SPEC.replace("Version:        2.0", "Version:        1.0");
        let target = snapshot(&[
            ("foo.spec", &target_spec),
            (
                "foo.changes",
                &changes_file("- Update to 1.0", "- Initial package"),
            ),
            ("foo-1.0.tar.gz", "binary"),
            ("old.patch", "diff"),
        ]);
        let local_spec = SPEC.replace(
            "Patch0:         fix-crash.patch",
            "Patch0:         new.patch",
        );
        let local = snapshot(&[
            ("foo.spec", &local_spec),
            (
                "foo.changes",
                &changes_file(
                    "- Update to 2.0\n  * Drop old.patch, add new.patch",
                    "- Initial package",
                ),
            ),
            ("foo-2.0.tar.gz", "binary"),
            ("new.patch", "diff"),
        ]);
        let report = validate(&local, Some(&target), &ValidateOptions::default());
        assert!(!report.findings.iter().any(|f| f.rule == Rule::V4));
        assert!(!report.findings.iter().any(|f| f.rule == Rule::V2));
    }

    #[test]
    fn target_rules_old_entries_must_stay_byte_identical() {
        let target = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file("- Update to 1.0", "- Initial package"),
            ),
        ]);
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file("- Update to 2.0", "- Initial package (edited)"),
            ),
        ]);
        let report = validate(&local, Some(&target), &ValidateOptions::default());
        assert!(report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V11 && f.message.contains("older entry differs")));
    }

    #[test]
    fn new_package_skips_the_diff_rules() {
        let target = PackageSnapshot::default();
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file("- Update to 2.0", "- Initial package"),
            ),
            ("foo-2.0.tar.gz", "binary"),
        ]);
        let report = validate(&local, Some(&target), &ValidateOptions::default());
        assert!(report.skipped.iter().any(|s| s.rule == Rule::V4));
        assert!(!report.findings.iter().any(|f| f.rule == Rule::V4));
    }

    #[test]
    fn diff_added_removed_finds_both_sides() {
        let old: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let new: Vec<String> = ["a", "x", "c", "d"].iter().map(|s| s.to_string()).collect();
        let (added, removed) = diff_added_removed(&old, &new);
        assert_eq!(added, ["x", "d"]);
        assert_eq!(removed, ["b"]);
    }

    #[test]
    fn glob_matches_macros() {
        assert!(source_value_matches(
            "foo-%{version}.tar.gz",
            "foo-2.0.tar.gz"
        ));
        assert!(glob_match("*.patch", "fix-crash.patch"));
        assert!(!glob_match("foo-*.tar.gz", "bar-2.0.tar.gz"));
        assert!(source_value_matches(
            "%{name}-%{version}.tar.gz",
            "foo-2.0.tar.gz"
        ));
    }

    #[test]
    fn license_normalisation_maps_spellings() {
        assert_eq!(normalize_license("GPLv3").as_deref(), Some("GPL-3.0"));
        assert_eq!(
            normalize_license("GPL-3.0-only").as_deref(),
            Some("GPL-3.0")
        );
        assert_eq!(normalize_license("MIT").as_deref(), Some("MIT"));
        assert_eq!(normalize_license("SEGV-1.1").as_deref(), Some("SEGV-1.1"));
        assert_eq!(normalize_license("change"), None);
        let tags = license_tokens_in_tag("GPL-3.0-only AND LGPL-2.1-or-later");
        assert!(tags.contains("GPL-3.0"));
        assert!(tags.contains("LGPL-2.1"));
    }

    #[test]
    fn v9_flags_a_bare_update_with_only_a_tarball_line() {
        // A version line plus a tarball rename is still a bare
        // entry: the file-list line says nothing about what the new
        // version brings.
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file("- Update to 2.0\n- Add foo-2.0.tar.gz", "- Initial package"),
            ),
        ]);
        let options = ValidateOptions {
            entries: 1,
            news_section: Some("## 2.0\n\n- Fix a crash\n"),
        };
        let report = validate(&local, None, &options);
        assert!(report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V9 && f.message.contains("bare 'Update")));
    }

    #[test]
    fn v9_todo_slot_is_a_finding_but_quoted_todo_text_is_not() {
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file("- Update to 2.0\n  * reason: TODO", "- Initial package"),
            ),
        ]);
        let options = ValidateOptions {
            entries: 1,
            news_section: Some("## 2.0\n\n- Fix a crash\n"),
        };
        let report = validate(&local, None, &options);
        assert!(report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V9 && f.message.contains("unfilled generation slots")));
        // A bullet quoting upstream TODO text is not a slot: the
        // slot shapes are `reason: TODO` and `TODO: ...`.
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file(
                    "- Update to 2.0\n- Fix the TODO parser",
                    "- Initial package",
                ),
            ),
        ]);
        let report = validate(&local, None, &options);
        assert!(!report.findings.iter().any(|f| f.rule == Rule::V9));
    }

    #[test]
    fn v10_direction_ignores_a_line_saying_no_change() {
        let target = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file("- Update to 1.0", "- Initial package"),
            ),
        ]);
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file(
                    "- Update to 2.0\n- License unchanged, still GPL-3.0-only",
                    "- Initial package",
                ),
            ),
        ]);
        let report = validate(&local, Some(&target), &ValidateOptions::default());
        assert!(!report.findings.iter().any(|f| f.rule == Rule::V10));
        // The same shape with a real change claim fires.
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file(
                    "- Update to 2.0\n- License change: the license changed",
                    "- Initial package",
                ),
            ),
        ]);
        let report = validate(&local, Some(&target), &ValidateOptions::default());
        assert!(report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V10 && f.message.contains("claims a license change")));
    }

    #[test]
    fn v11_flags_a_star_at_the_top_level_and_an_indented_dash() {
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file(
                    "- Update to 2.0\n* Fix a crash\n  * Sub item\n    - deep detail",
                    "- Initial package",
                ),
            ),
        ]);
        let report = validate(&local, None, &ValidateOptions::default());
        let bullet_findings: Vec<&Finding> = report
            .findings
            .iter()
            .filter(|f| {
                f.rule == Rule::V11
                    && (f.message.contains("top level") || f.message.contains("indented"))
            })
            .collect();
        assert_eq!(bullet_findings.len(), 2);
    }

    #[test]
    fn v12_flags_a_top_level_cve_named_as_not_affected() {
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file(
                    "- Update to 2.0\n- CVE-2026-12345 is not affected by this update",
                    "- Initial package",
                ),
            ),
        ]);
        let report = validate(&local, None, &ValidateOptions::default());
        assert!(report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V12 && f.message.contains("not fixed / not affected")));
    }

    #[test]
    fn v12_flags_a_pure_security_claim() {
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file("- Update to 2.0\n- Security fixes", "- Initial package"),
            ),
        ]);
        let report = validate(&local, None, &ValidateOptions::default());
        assert!(report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V12 && f.message.contains("claims security fixes")));
    }

    #[test]
    fn v4_source_exemption_covers_a_patch_named_as_source() {
        let target_spec = SPEC.replace("Version:        2.0", "Version:        1.0");
        let target = snapshot(&[
            ("foo.spec", &target_spec),
            (
                "foo.changes",
                &changes_file("- Update to 1.0", "- Initial package"),
            ),
            ("foo-1.0.tar.gz", "binary"),
            ("old.patch", "diff"),
        ]);
        // Local: old.patch dropped and new.patch added, both named;
        // data.patch added, named only as a Source: — exempt.
        let local_spec = SPEC
            .replace(
                "Patch0:         fix-crash.patch",
                "Patch0:         new.patch",
            )
            .replace(
                "Source0:        foo-2.0.tar.gz",
                "Source0:        foo-2.0.tar.gz\nSource1:        data.patch",
            );
        let local = snapshot(&[
            ("foo.spec", &local_spec),
            (
                "foo.changes",
                &changes_file(
                    "- Update to 2.0\n  * Drop old.patch, add new.patch",
                    "- Initial package",
                ),
            ),
            ("foo-2.0.tar.gz", "binary"),
            ("new.patch", "diff"),
            ("data.patch", "diff"),
        ]);
        let report = validate(&local, Some(&target), &ValidateOptions::default());
        assert!(!report.findings.iter().any(|f| f.rule == Rule::V4));
        // Without the Source: declaration the same file is a V4
        // finding.
        let local_spec = SPEC.replace(
            "Patch0:         fix-crash.patch",
            "Patch0:         new.patch",
        );
        let local = snapshot(&[
            ("foo.spec", &local_spec),
            (
                "foo.changes",
                &changes_file(
                    "- Update to 2.0\n  * Drop old.patch, add new.patch",
                    "- Initial package",
                ),
            ),
            ("foo-2.0.tar.gz", "binary"),
            ("new.patch", "diff"),
            ("data.patch", "diff"),
        ]);
        let report = validate(&local, Some(&target), &ValidateOptions::default());
        assert!(report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V4 && f.message.contains("data.patch")));
    }

    #[test]
    fn v11_entry_count_and_entries_removed() {
        let sep = "-".repeat(67);
        let target = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &format!(
                    "{sep}\nMon May 25 09:00:00 UTC 2026 - Jane Packager <jane@example.com>\n\n- Update to 1.0\n\n{sep}\nSun May 24 09:00:00 UTC 2026 - Jane Packager <jane@example.com>\n\n- Initial package\n\n"
                ),
            ),
        ]);
        // Local: two new entries on top of the target's two, the
        // older entries byte-identical — only the count fires.
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &format!(
                    "{sep}\nWed May 27 16:31:48 UTC 2026 - Jane Packager <jane@example.com>\n\n- Update to 2.0\n\n{sep}\nTue May 26 10:00:00 UTC 2026 - Jane Packager <jane@example.com>\n\n- Update to 1.9\n\n{sep}\nMon May 25 09:00:00 UTC 2026 - Jane Packager <jane@example.com>\n\n- Update to 1.0\n\n{sep}\nSun May 24 09:00:00 UTC 2026 - Jane Packager <jane@example.com>\n\n- Initial package\n\n"
                ),
            ),
        ]);
        let report = validate(&local, Some(&target), &ValidateOptions::default());
        assert!(report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V11 && f.message.contains("new entries vs the target")));
        assert!(!report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V11 && f.message.contains("older entry differs")));
        // Local with fewer entries than the target: entries removed.
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &format!(
                    "{sep}\nWed May 27 16:31:48 UTC 2026 - Jane Packager <jane@example.com>\n\n- Update to 2.0\n\n"
                ),
            ),
        ]);
        let report = validate(&local, Some(&target), &ValidateOptions::default());
        assert!(report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V11 && f.message.contains("entries removed")));
    }

    #[test]
    fn v5_pair_needs_a_matching_date_and_author() {
        // The date matches the entry header but the author does not:
        // a quoted date, not the duplicated-header pair.
        let local = snapshot(&[
            ("foo.spec", SPEC),
            (
                "foo.changes",
                &changes_file(
                    "- Update to 2.0\n  * Wed May 27 2026 release",
                    "- Initial package",
                ),
            ),
        ]);
        let report = validate(&local, None, &ValidateOptions::default());
        assert!(!report
            .findings
            .iter()
            .any(|f| f.rule == Rule::V5 && f.message.contains("duplicated-header")));
    }

    #[test]
    fn v6_macro_source_exempts_its_changes_file() {
        // `foo-2.0.changes` matches no subpackage (the spec names
        // only `foo`), but it is the macro Source: value
        // `%{name}-%{version}.changes` — the V7 glob exempts it.
        let spec = SPEC.replace(
            "Source0:        foo-2.0.tar.gz",
            "Source0:        foo-2.0.tar.gz\nSource1:        %{name}-%{version}.changes",
        );
        let local = snapshot(&[
            ("foo.spec", &spec),
            (
                "foo-2.0.changes",
                &changes_file("- Update to 2.0", "- Initial package"),
            ),
        ]);
        let report = validate(&local, None, &ValidateOptions::default());
        assert!(!report.findings.iter().any(|f| f.rule == Rule::V6));
    }
}
