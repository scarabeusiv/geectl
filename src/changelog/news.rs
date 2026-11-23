//! Upstream NEWS extraction from a source tarball.
//!
//! The tarball is on disk in the local checkout (the `changelog generate`
//! / V9 source), so the extraction reads it with the `tar` command
//! instead of a new native dependency: `tar -tf` lists members,
//! `tar -xOf` extracts one to stdout. Both run under a bounded
//! timeout; a missing `tar`, a timeout, or an unreadable archive is
//! an explicit error, never an empty section — a failed lookup must
//! never read as good news (the caller reports V9 as skipped with
//! the reason, and `generate` notes the missing news).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// How long a `tar` invocation may run.
const TAR_TIMEOUT: Duration = Duration::from_secs(30);

/// Cap on a `tar` invocation's stdout/stderr: a hostile multi-GB
/// member must not OOM the CLI while the section cap waits downstream.
const MAX_TAR_OUTPUT: usize = 1 << 20;

/// Why the news lookup failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewsError {
    /// No `tar` on PATH.
    NoTar,
    /// The `tar` invocation timed out.
    Timeout,
    /// The archive could not be listed or read.
    Unreadable(String),
    /// A `tar` invocation produced more than the output cap.
    OutputTooLarge,
}

impl std::fmt::Display for NewsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NewsError::NoTar => write!(f, "no tar command on PATH"),
            NewsError::Timeout => write!(f, "tar timed out"),
            NewsError::Unreadable(detail) => write!(f, "tarball unreadable: {detail}"),
            NewsError::OutputTooLarge => write!(f, "tar output exceeded the 1 MiB cap"),
        }
    }
}

/// An extracted NEWS section for one version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsSection {
    /// The archive member the section came from.
    pub source_file: String,
    /// The section text (headings kept), trimmed.
    pub text: String,
}

/// Find the source tarball in a package directory: the `Source0`
/// value when it names a tarball on disk, else a tarball whose name
/// carries the version, else the only tarball in the directory.
pub fn find_tarball(dir: &Path, version: &str, source0: Option<&str>) -> Option<PathBuf> {
    let is_tarball = |name: &str| {
        name.ends_with(".tar.gz")
            || name.ends_with(".tgz")
            || name.ends_with(".tar.xz")
            || name.ends_with(".tar.bz2")
            || name.ends_with(".tar.zst")
            || name.ends_with(".tar")
    };
    let entries = std::fs::read_dir(dir).ok()?;
    let mut tarballs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_tarball(&name) && entry.path().is_file() {
            tarballs.push(entry.path());
        }
    }
    if let Some(source0) = source0 {
        let base = source0.rsplit('/').next().unwrap_or(source0);
        if let Some(found) = tarballs
            .iter()
            .find(|path| path.file_name().map(|name| name == base).unwrap_or(false))
        {
            return Some(found.clone());
        }
    }
    if let Some(found) = tarballs.iter().find(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().contains(version))
            .unwrap_or(false)
    }) {
        return Some(found.clone());
    }
    if tarballs.len() == 1 {
        return tarballs.into_iter().next();
    }
    None
}

/// The archive path as a `tar` argument: a path starting with `-`
/// would be parsed as an option, so it is prefixed `./` (the same
/// file). The path is the `-f` argument either way; the prefix only
/// removes the option reading.
fn tar_archive_arg(tarball: &Path) -> String {
    let path = tarball.to_string_lossy().into_owned();
    if path.starts_with('-') {
        format!("./{path}")
    } else {
        path
    }
}

/// List the members of a tarball.
fn tar_list(tarball: &Path) -> Result<Vec<String>, NewsError> {
    let output = run_tar(&["-tf", &tar_archive_arg(tarball)])?;
    if !output.status.success() {
        return Err(NewsError::Unreadable(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect())
}

/// Extract one member of a tarball to a string.
fn tar_extract(tarball: &Path, member: &str) -> Result<String, NewsError> {
    // The member comes from `tar -tf` output, so it is
    // attacker-controlled: `--` ends option parsing before it, or a
    // member starting with `-` would be parsed as a flag.
    let output = run_tar(&["-xOf", &tar_archive_arg(tarball), "--", member])?;
    if !output.status.success() {
        return Err(NewsError::Unreadable(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Drain a pipe, keeping at most `MAX_TAR_OUTPUT` bytes: the kept
/// bytes plus whether more arrived (the caller errors on that).
/// Bytes past the cap are discarded but still read, so the child is
/// never blocked on a full pipe.
fn drain_capped<R: Read>(pipe: Option<R>) -> (Vec<u8>, bool) {
    let mut buf = Vec::new();
    let mut total: usize = 0;
    let Some(mut pipe) = pipe else {
        return (buf, false);
    };
    let mut chunk = [0u8; 8192];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                total += n;
                if buf.len() < MAX_TAR_OUTPUT {
                    let take = (MAX_TAR_OUTPUT - buf.len()).min(n);
                    buf.extend_from_slice(&chunk[..take]);
                }
            }
            Err(_) => break,
        }
    }
    (buf, total > MAX_TAR_OUTPUT)
}

/// Run `tar` with the given arguments under the timeout. The output
/// is drained on threads, capped at `MAX_TAR_OUTPUT`, so a large
/// member cannot deadlock the caller on a full pipe or OOM it.
fn run_tar(args: &[&str]) -> Result<std::process::Output, NewsError> {
    let mut child: Child = match Command::new("tar")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Err(NewsError::NoTar),
        Err(err) => return Err(NewsError::Unreadable(err.to_string())),
    };
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let stdout_drain = std::thread::spawn(move || drain_capped(stdout.take()));
    let stderr_drain = std::thread::spawn(move || drain_capped(stderr.take()));
    let deadline = std::time::Instant::now() + TAR_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // A panicked stdout drain fails closed: the
                // truncation signal is kept, never silently cleared.
                let (stdout, stdout_truncated) = stdout_drain.join().unwrap_or((vec![], true));
                let (stderr, _) = stderr_drain.join().unwrap_or_default();
                // Only stdout is retained and parsed: a stderr flood
                // must not fail an otherwise good extraction.
                if stdout_truncated {
                    return Err(NewsError::OutputTooLarge);
                }
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(NewsError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) => return Err(NewsError::Unreadable(err.to_string())),
        }
    }
}

/// The NEWS-ish member names, in preference order. A shallow member
/// (fewer `/` components) wins over a deeper one with the same name.
fn news_candidates(members: &[String]) -> Vec<&String> {
    const NAMES: [&str; 12] = [
        "NEWS",
        "NEWS.md",
        "NEWS.txt",
        "NEWS.rst",
        "ChangeLog",
        "CHANGELOG",
        "CHANGELOG.md",
        "CHANGELOG.txt",
        "CHANGES",
        "CHANGES.md",
        "HISTORY",
        "HISTORY.md",
    ];
    let mut candidates: Vec<&String> = members
        .iter()
        .filter(|member| {
            let base = member.rsplit('/').next().unwrap_or(member);
            NAMES.contains(&base)
        })
        .collect();
    candidates.sort_by_key(|member| {
        let base = member.rsplit('/').next().unwrap_or(member);
        (
            NAMES
                .iter()
                .position(|name| *name == base)
                .unwrap_or(NAMES.len()),
            member.matches('/').count(),
        )
    });
    candidates
}

/// Extract the section for `version` from a tarball's NEWS file:
/// the heading naming the version, up to the next version heading.
/// `None` when the tarball has no NEWS file, or no section for this
/// version — the two cases are indistinguishable by design, and the
/// caller treats both as "no news found".
pub fn extract_news_section(
    tarball: &Path,
    version: &str,
) -> Result<Option<NewsSection>, NewsError> {
    let members = tar_list(tarball)?;
    let Some(member) = news_candidates(&members).into_iter().next() else {
        return Ok(None);
    };
    let member = member.clone();
    let text = tar_extract(tarball, &member)?;
    Ok(
        extract_version_section(&text, version).map(|section| NewsSection {
            source_file: member,
            text: section,
        }),
    )
}

/// The section of a NEWS text for `version`: from the heading that
/// names the version to the next version heading (or EOF). A heading
/// is a line that, after stripping leading `#`s and space, starts
/// with the version (optional `v` prefix), `Version <version>`, or
/// `<version>` followed by a date or bracket. A file with no version
/// headings has no attributable section.
fn extract_version_section(text: &str, version: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let is_heading = |line: &str| -> bool {
        let stripped = line.trim_start_matches(['#', ' ']);
        is_version_heading(stripped, version)
    };
    let start = lines.iter().position(|line| is_heading(line))?;
    // The section heading's level: a section ends at a heading of
    // equal or higher level (fewer or equal `#`s). Deeper subheadings
    // (`### Added` under `## 2.0`) stay inside the section. An
    // underline-style heading counts as level 1.
    let section_level = lines[start]
        .chars()
        .take_while(|c| *c == '#')
        .count()
        .max(1);
    let mut end = lines.len();
    for (i, line) in lines.iter().enumerate().skip(start + 1) {
        // A new section starts at a version heading for another
        // version (at any level), or at a markdown heading of equal
        // or higher level than the section heading.
        let stripped = line.trim_start_matches(['#', ' ']);
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if is_version_heading(stripped, "") {
            end = i;
            break;
        }
        if hashes > 0 && hashes <= section_level && !stripped.is_empty() {
            end = i;
            break;
        }
        // Underline-style headings (`===` / `---`) end a section
        // when the line above them is not the section heading.
        if (line.trim().chars().all(|c| c == '=') || line.trim().chars().all(|c| c == '-'))
            && !line.trim().is_empty()
            && i > start + 1
        {
            end = i - 1;
            break;
        }
    }
    let section: Vec<&str> = lines[start..end]
        .iter()
        .copied()
        .skip_while(|line| line.trim().is_empty())
        .collect();
    let section: Vec<&str> = section
        .iter()
        .rev()
        .skip_while(|line| line.trim().is_empty())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .copied()
        .collect();
    if section.iter().all(|line| line.trim().is_empty()) {
        return None;
    }
    // Cap the section: the generator takes the first lines; a
    // runaway NEWS file must not flood the entry.
    let capped: Vec<&str> = section.into_iter().take(100).collect();
    Some(capped.join("\n"))
}

/// True when `stripped` (a line with leading `#`s and space removed)
/// is a heading for `version`. With an empty `version`, any
/// version-like heading matches.
fn is_version_heading(stripped: &str, version: &str) -> bool {
    if stripped.is_empty() {
        return false;
    }
    // `Version 2.0`, `version 2.0`, `Release 2.0`.
    for prefix in ["Version ", "version ", "Release ", "release "] {
        if let Some(rest) = stripped.strip_prefix(prefix) {
            if version.is_empty() {
                return rest.chars().next().is_some_and(|c| c.is_ascii_digit());
            }
            return rest == version || rest.starts_with(&format!("{version} "));
        }
    }
    // `2.0`, `v2.0`, `2.0 (2026-01-01)`, `[2.0]`.
    let candidate = stripped
        .strip_prefix('v')
        .or_else(|| stripped.strip_prefix('['))
        .unwrap_or(stripped);
    if version.is_empty() {
        // Any dotted version, not just any line starting with a
        // digit (`2026-01-01: ...` is a date, not a heading).
        let mut chars = candidate.chars();
        if !chars.next().is_some_and(|c| c.is_ascii_digit()) {
            return false;
        }
        let mut seen_dot = false;
        let mut seen_digit_after_dot = false;
        for c in chars {
            if c == '.' {
                seen_dot = true;
            } else if c.is_ascii_digit() {
                if seen_dot {
                    seen_digit_after_dot = true;
                }
            } else {
                break;
            }
        }
        return seen_dot && seen_digit_after_dot;
    }
    candidate == version
        || candidate.starts_with(&format!("{version} "))
        || candidate.starts_with(&format!("{version}("))
        || candidate.starts_with(&format!("{version}]"))
        || candidate.starts_with(&format!("{version}:"))
        || candidate.starts_with(&format!("{version}-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_section_is_extracted_up_to_the_next_heading() {
        let news = [
            "## 1.0",
            "",
            "- Old news",
            "",
            "## 2.0",
            "",
            "- Fix a crash",
            "- Add a feature",
            "",
            "## 3.0",
            "",
            "- Future news",
            "",
        ]
        .join("\n");
        let section = extract_version_section(&news, "2.0").expect("section found");
        assert!(section.contains("Fix a crash"));
        assert!(section.contains("Add a feature"));
        assert!(!section.contains("Old news"));
        assert!(!section.contains("Future news"));
    }

    #[test]
    fn version_heading_forms_match() {
        assert!(is_version_heading("2.0 (2026-01-01)", "2.0"));
        assert!(is_version_heading("v2.0", "2.0"));
        assert!(is_version_heading("Version 2.0", "2.0"));
        assert!(is_version_heading("[2.0]", "2.0"));
        assert!(!is_version_heading("2.1", "2.0"));
        assert!(!is_version_heading("Fix a crash", "2.0"));
        assert!(is_version_heading("3.0", ""));
    }

    #[test]
    fn version_section_keeps_deeper_subheadings() {
        // Keep-a-Changelog shape: `### Added` / `### Fixed` under
        // `## 2.0` stay inside the section; only the next `##`
        // version heading (or a heading of equal or higher level)
        // ends it.
        let news = [
            "## 1.0",
            "",
            "- Old news",
            "",
            "## 2.0",
            "",
            "### Added",
            "",
            "- Add a new subcommand",
            "",
            "### Fixed",
            "",
            "- Fix a crash in the parser",
            "",
            "## 1.0",
            "",
            "- Old news again",
            "",
        ]
        .join("\n");
        let section = extract_version_section(&news, "2.0").expect("section found");
        assert!(section.contains("Add a new subcommand"));
        assert!(section.contains("Fix a crash in the parser"));
        assert!(!section.contains("Old news"));
    }

    #[test]
    fn no_version_headings_means_no_section() {
        let news = "- Fix a crash\n- Add a feature\n";
        assert!(extract_version_section(news, "2.0").is_none());
    }

    #[test]
    fn news_candidates_prefer_news_over_changelog_and_shallow_over_deep() {
        let members = vec![
            "foo-2.0/doc/NEWS".to_string(),
            "foo-2.0/ChangeLog".to_string(),
            "foo-2.0/NEWS".to_string(),
        ];
        let candidates = news_candidates(&members);
        assert_eq!(candidates[0], "foo-2.0/NEWS");
    }

    #[test]
    fn tar_round_trip_extracts_a_section() {
        // Build a real tarball with the `tar` command when it is
        // available; skip the test where it is not.
        if Command::new("tar").arg("--version").output().is_err() {
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("geectl-changelog-news-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("foo-2.0")).unwrap();
        std::fs::write(
            dir.join("foo-2.0/NEWS"),
            "## 1.0\n\n- Old\n\n## 2.0\n\n- Fix a crash\n",
        )
        .unwrap();
        let tarball = dir.join("foo-2.0.tar.gz");
        let status = Command::new("tar")
            .args(["-czf"])
            .arg(&tarball)
            .args(["-C"])
            .arg(&dir)
            .arg("foo-2.0")
            .status()
            .unwrap();
        assert!(status.success());
        let section = extract_news_section(&tarball, "2.0")
            .expect("extraction runs")
            .expect("section found");
        assert_eq!(section.source_file, "foo-2.0/NEWS");
        assert!(section.text.contains("Fix a crash"));
        assert!(!section.text.contains("Old"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_tarball_prefers_source0_then_version_then_sole() {
        // A distinct temp dir from the tar round-trip test: the two
        // run in parallel threads of one process.
        let dir = std::env::temp_dir().join(format!(
            "geectl-changelog-news-find-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["foo-1.0.tar.gz", "foo-2.0.tar.gz", "other.tar.gz"] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        let base =
            |p: Option<PathBuf>| p.map(|p| p.file_name().unwrap().to_string_lossy().into_owned());
        // Source0 names one outright.
        assert_eq!(
            base(find_tarball(&dir, "2.0", Some("other.tar.gz"))),
            Some("other.tar.gz".to_string())
        );
        // Otherwise the tarball carrying the version wins.
        assert_eq!(
            base(find_tarball(&dir, "2.0", None)),
            Some("foo-2.0.tar.gz".to_string())
        );
        // With no version match and several tarballs, no pick.
        assert_eq!(find_tarball(&dir, "9.9", None), None);
        // A sole tarball is the pick whatever its name.
        std::fs::remove_file(dir.join("foo-1.0.tar.gz")).unwrap();
        std::fs::remove_file(dir.join("foo-2.0.tar.gz")).unwrap();
        assert_eq!(
            base(find_tarball(&dir, "9.9", None)),
            Some("other.tar.gz".to_string())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tar_archive_arg_prefixes_dash_leading_paths() {
        assert_eq!(
            tar_archive_arg(std::path::Path::new("-foo-2.0.tar.gz")),
            "./-foo-2.0.tar.gz"
        );
        assert_eq!(
            tar_archive_arg(std::path::Path::new("foo-2.0.tar.gz")),
            "foo-2.0.tar.gz"
        );
        assert_eq!(
            tar_archive_arg(std::path::Path::new("/tmp/foo-2.0.tar.gz")),
            "/tmp/foo-2.0.tar.gz"
        );
    }

    #[test]
    fn tar_extracts_a_dash_leading_member_name() {
        // A member whose name starts with `-` is an operand, never
        // an option: without the `--` separator `tar` parses it as a
        // flag.
        if Command::new("tar").arg("--version").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "geectl-changelog-news-dash-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("--weird"), "## 2.0\n\n- Fix a crash\n").unwrap();
        let tarball = dir.join("foo-2.0.tar.gz");
        let status = Command::new("tar")
            .args(["-czf"])
            .arg(&tarball)
            .args(["-C"])
            .arg(&dir)
            .args(["--", "--weird"])
            .status()
            .unwrap();
        assert!(status.success());
        let members = tar_list(&tarball).expect("listing runs");
        assert!(members.iter().any(|m| m == "--weird"));
        let text = tar_extract(&tarball, "--weird").expect("extraction runs");
        assert!(text.contains("Fix a crash"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tar_extract_refuses_an_oversized_member() {
        if Command::new("tar").arg("--version").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "geectl-changelog-news-cap-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("foo-2.0")).unwrap();
        let big = "- filler\n".repeat(200_000);
        std::fs::write(dir.join("foo-2.0/NEWS"), format!("## 2.0\n\n{big}")).unwrap();
        let tarball = dir.join("foo-2.0.tar.gz");
        let status = Command::new("tar")
            .args(["-czf"])
            .arg(&tarball)
            .args(["-C"])
            .arg(&dir)
            .arg("foo-2.0")
            .status()
            .unwrap();
        assert!(status.success());
        let err = extract_news_section(&tarball, "2.0").expect_err("oversized member is refused");
        assert_eq!(err, NewsError::OutputTooLarge);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
