//! One-time import of the osc instance mapping from an oscrc file.
//!
//! geectl never reads an oscrc file at runtime: it is another tool's
//! file, with plaintext passwords and possibly a different or stale
//! identity, so trusting it at runtime would let geectl silently act
//! as the wrong identity. The import command (`geectl config
//! import-oscrc`) reads it once, on explicit request, and migrates
//! only the non-secret mapping (alias to apiurl, username) into the
//! config `[instances]` table. The password is never retained: the
//! parser drops `pass` values on sight, and no function here returns
//! one — a section carries only its apiurl and its username.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::auth::{open_file_nonblocking, snapshot_env};
use crate::config::home_dir;
use crate::error::{ErrorKind, GeectlError};

/// The largest oscrc accepted (64 KiB, the same cap as the config and
/// secret files).
pub const MAX_OSCRC_BYTES: usize = 64 * 1024;

/// One apiurl section of an oscrc file: the instance URL and the
/// username for it, when the section carries one. There is no password
/// field by design: the parser drops `pass` values on sight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OscrcSection {
    /// The instance API URL: the section name, or the `apiurl` key of
    /// the `[general]` section.
    pub apiurl: String,
    /// The `user` key of the section, when present and non-empty.
    pub username: Option<String>,
}

/// The default oscrc path: `$XDG_CONFIG_HOME/osc/oscrc` when set,
/// else `~/.config/osc/oscrc`.
pub fn default_oscrc_path() -> Result<PathBuf, GeectlError> {
    let env = snapshot_env(&[
        "XDG_CONFIG_HOME",
        "HOME",
        #[cfg(windows)]
        "USERPROFILE",
    ])?;
    resolve_oscrc_path(
        env.get("XDG_CONFIG_HOME").map(String::as_str),
        home_dir(
            env.get("HOME").map(String::as_str),
            env.get("USERPROFILE").map(String::as_str),
        ),
    )
}

/// Pure path resolution, for tests: the XDG path wins when set and
/// absolute, else the home default.
pub fn resolve_oscrc_path(
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> Result<PathBuf, GeectlError> {
    if let Some(xdg) = xdg_config_home.filter(|p| Path::new(p).is_absolute()) {
        return Ok(PathBuf::from(xdg).join("osc").join("oscrc"));
    }
    if let Some(home) = home {
        return Ok(PathBuf::from(home)
            .join(".config")
            .join("osc")
            .join("oscrc"));
    }
    Err(GeectlError::new(
        ErrorKind::ConfigError,
        "cannot locate an oscrc file: set XDG_CONFIG_HOME or HOME, or pass --oscrc",
    ))
}

/// Read an oscrc file: a missing file is an error (unlike the geectl
/// config — the user asked to import from this file), a FIFO or device
/// is refused before any byte is read, the read is capped, and the
/// content must be UTF-8. The content is returned as text; the caller
/// parses it with [`parse_oscrc`], which drops password values.
pub fn read_oscrc(path: &Path) -> Result<String, GeectlError> {
    let file = match open_file_nonblocking(path) {
        Ok(file) => file,
        Err(e) => {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!("{}: cannot read oscrc: {e}", path.display()),
            ));
        }
    };
    match file.metadata() {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!("{}: not a regular file", path.display()),
            ));
        }
        Err(e) => {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!("{}: cannot read oscrc: {e}", path.display()),
            ));
        }
    }
    let mut bytes = Vec::new();
    if let Err(e) = file
        .take((MAX_OSCRC_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
    {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("{}: cannot read oscrc: {e}", path.display()),
        ));
    }
    if bytes.len() > MAX_OSCRC_BYTES {
        return Err(GeectlError::new(
            ErrorKind::ConfigError,
            format!("{}: oscrc exceeds {MAX_OSCRC_BYTES} bytes", path.display()),
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        GeectlError::new(
            ErrorKind::ConfigError,
            format!("{}: oscrc is not valid UTF-8", path.display()),
        )
    })
}

/// Parse oscrc text (INI: `[section]` headers, `key = value` lines,
/// `#`/`;` comments, indented continuation lines). Returns the
/// instance sections in first-seen order, merged by apiurl: the
/// `[general]` section contributes its `apiurl` key, any other
/// section named by an `http(s)` URL contributes itself, and any
/// other section is ignored (it names no instance). A section's
/// username is its `user` key, when present and non-empty; the
/// `pass` value is dropped on sight and never retained.
///
/// A malformed line, a key before the first section, or a duplicate
/// section fails closed with the line number only — never the line
/// content, which could hold a password.
pub fn parse_oscrc(text: &str, path: &Path) -> Result<Vec<OscrcSection>, GeectlError> {
    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut current: Option<usize> = None;
    // The key a continuation line extends, as an index into the
    // current section's keys; cleared by a blank line or a new
    // section, so a continuation can never land in the wrong section.
    let mut pending_key: Option<usize> = None;
    for (index, raw_line) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = raw_line.trim_end();
        if line.starts_with([' ', '\t']) {
            match (current, pending_key) {
                (Some(section), Some(key)) if key < sections[section].1.len() => {
                    sections[section].1[key].1.push('\n');
                    sections[section].1[key].1.push_str(line.trim());
                    continue;
                }
                _ => return Err(malformed(path, line_no)),
            }
        }
        let line = strip_inline_comment(line).trim_start();
        if line.is_empty() {
            pending_key = None;
            continue;
        }
        if line.starts_with(['#', ';']) {
            continue;
        }
        if line.starts_with('[') {
            let Some(rest) = line.strip_prefix('[') else {
                return Err(malformed(path, line_no));
            };
            let Some((name, after)) = rest.split_once(']') else {
                return Err(malformed(path, line_no));
            };
            let after = strip_inline_comment(after).trim();
            if !after.is_empty() {
                return Err(malformed(path, line_no));
            }
            let name = name.trim().to_string();
            if name.is_empty() {
                return Err(malformed(path, line_no));
            }
            if sections.iter().any(|(n, _)| n == &name) {
                return Err(GeectlError::new(
                    ErrorKind::ConfigError,
                    format!(
                        "{}: duplicate oscrc section at line {line_no}",
                        path.display()
                    ),
                ));
            }
            sections.push((name, Vec::new()));
            current = Some(sections.len() - 1);
            pending_key = None;
            continue;
        }
        let Some(section) = current else {
            return Err(malformed(path, line_no));
        };
        // The delimiter is the first `=` or `:` in the line, as in
        // configparser. The value keeps its own `:` (the URL scheme)
        // because the split happens at the first delimiter only.
        let Some(split) = line
            .find(['=', ':'])
            .map(|i| (line[..i].trim().to_ascii_lowercase(), line[i + 1..].trim()))
        else {
            return Err(malformed(path, line_no));
        };
        let (key, value) = split;
        if key.is_empty() {
            return Err(malformed(path, line_no));
        }
        sections[section].1.push((key, value.to_string()));
        pending_key = Some(sections[section].1.len() - 1);
    }
    let mut merged: Vec<OscrcSection> = Vec::new();
    for (name, keys) in sections {
        let username = keys
            .iter()
            .find(|(key, _)| key == "user")
            .map(|(_, value)| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if name.eq_ignore_ascii_case("general") {
            if let Some(apiurl) = keys
                .iter()
                .find(|(key, _)| key == "apiurl")
                .map(|(_, value)| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                merge_section(&mut merged, apiurl, username);
            }
            continue;
        }
        if name.starts_with("http://") || name.starts_with("https://") {
            merge_section(&mut merged, name, username);
        }
        // Any other section names no instance; ignored.
    }
    Ok(merged)
}

/// Strip an inline comment: a `#` or `;` at the start, or preceded by
/// whitespace, starts one — a `#` glued to a value (as in a password)
/// does not. The value is never inspected by the caller beyond this
/// cut, so a password containing `#` survives the cut untouched and
/// is then dropped with the rest of the `pass` line.
fn strip_inline_comment(line: &str) -> &str {
    let mut previous = ' ';
    for (index, ch) in line.char_indices() {
        if (ch == '#' || ch == ';') && (index == 0 || previous.is_whitespace()) {
            return &line[..index];
        }
        previous = ch;
    }
    line
}

/// Merge one parsed section into the result by apiurl: a second
/// section for the same URL fills in a missing username only — the
/// first username wins, so the order is deterministic.
fn merge_section(merged: &mut Vec<OscrcSection>, apiurl: String, username: Option<String>) {
    if let Some(existing) = merged.iter_mut().find(|s| s.apiurl == apiurl) {
        if existing.username.is_none() {
            existing.username = username;
        }
        return;
    }
    merged.push(OscrcSection { apiurl, username });
}

/// A malformed oscrc line: the path and line number only, never the
/// line content — it could hold a password.
fn malformed(path: &Path, line: usize) -> GeectlError {
    GeectlError::new(
        ErrorKind::ConfigError,
        format!("{}: malformed oscrc at line {line}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oscrc_path() -> PathBuf {
        PathBuf::from("/x/osc/oscrc")
    }

    const TWO_SECTIONS: &str = "\
[general]
apiurl = https://api.opensuse.org

[https://api.opensuse.org]
user = fakeuser@example.com
pass = fake-password-7XQ2

[https://obs.example.com]
user = otheruser
pass = fake-password-9ZQ4
";

    #[test]
    fn parse_two_sections_returns_both_with_usernames() {
        let sections = parse_oscrc(TWO_SECTIONS, &oscrc_path()).unwrap();
        assert_eq!(
            sections,
            vec![
                OscrcSection {
                    apiurl: "https://api.opensuse.org".to_string(),
                    username: Some("fakeuser@example.com".to_string()),
                },
                OscrcSection {
                    apiurl: "https://obs.example.com".to_string(),
                    username: Some("otheruser".to_string()),
                },
            ]
        );
    }

    #[test]
    fn parse_never_retains_a_password() {
        let sections = parse_oscrc(TWO_SECTIONS, &oscrc_path()).unwrap();
        let rendered = format!("{sections:?}");
        assert!(!rendered.contains("fake-password-7XQ2"), "{rendered}");
        assert!(!rendered.contains("fake-password-9ZQ4"), "{rendered}");
    }

    #[test]
    fn parse_section_without_user_has_no_username() {
        let sections = parse_oscrc(
            "[general]\napiurl = https://api.opensuse.org\n\n[https://api.opensuse.org]\npass = fake-password-7XQ2\n",
            &oscrc_path(),
        )
        .unwrap();
        assert_eq!(
            sections,
            vec![OscrcSection {
                apiurl: "https://api.opensuse.org".to_string(),
                username: None,
            }]
        );
    }

    #[test]
    fn parse_general_only_uses_its_apiurl() {
        let sections = parse_oscrc(
            "[general]\napiurl = https://api.opensuse.org\nuser = fakeuser@example.com\n",
            &oscrc_path(),
        )
        .unwrap();
        assert_eq!(
            sections,
            vec![OscrcSection {
                apiurl: "https://api.opensuse.org".to_string(),
                username: Some("fakeuser@example.com".to_string()),
            }]
        );
    }

    #[test]
    fn parse_general_and_section_merge_by_apiurl() {
        // The general section names the URL; the URL-named section
        // carries the username. One merged entry, username kept.
        let sections = parse_oscrc(
            "[general]\napiurl = https://api.opensuse.org\n\n[https://api.opensuse.org]\nuser = fakeuser@example.com\npass = fake-password-7XQ2\n",
            &oscrc_path(),
        )
        .unwrap();
        assert_eq!(
            sections,
            vec![OscrcSection {
                apiurl: "https://api.opensuse.org".to_string(),
                username: Some("fakeuser@example.com".to_string()),
            }]
        );
    }

    #[test]
    fn parse_ignores_non_instance_sections() {
        let sections = parse_oscrc(
            "[general]\napiurl = https://api.opensuse.org\nuser = fakeuser@example.com\n\n[unrelated]\nuser = nobody\n",
            &oscrc_path(),
        )
        .unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].apiurl, "https://api.opensuse.org");
    }

    #[test]
    fn parse_malformed_line_fails_closed_without_content() {
        let marker = "MARKER-NOT-A-SECRET-123";
        let text = format!("[general]\napiurl = https://api.opensuse.org\nbogus line {marker}\n");
        let err = parse_oscrc(&text, &oscrc_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("line 3"), "{}", err.context);
        assert!(
            !err.context.contains(marker),
            "error must not echo the line: {}",
            err.context
        );
    }

    #[test]
    fn parse_key_before_section_fails_closed() {
        let err = parse_oscrc("user = fakeuser@example.com\n", &oscrc_path()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("line 1"), "{}", err.context);
    }

    #[test]
    fn parse_duplicate_section_fails_closed() {
        let err = parse_oscrc(
            "[general]\napiurl = https://api.opensuse.org\n[general]\napiurl = https://api.opensuse.org\n",
            &oscrc_path(),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("duplicate"), "{}", err.context);
    }

    #[test]
    fn parse_unclosed_section_fails_closed() {
        let err = parse_oscrc(
            "[general\napiurl = https://api.opensuse.org\n",
            &oscrc_path(),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(err.context.contains("line 1"), "{}", err.context);
    }

    #[test]
    fn parse_empty_file_returns_no_sections() {
        assert_eq!(parse_oscrc("", &oscrc_path()).unwrap(), Vec::new());
        assert_eq!(
            parse_oscrc("# only a comment\n", &oscrc_path()).unwrap(),
            Vec::new()
        );
    }

    /// Absolute fake homes for the path-resolution test: unix
    /// paths on unix, drive-letter paths on Windows — a bare `/xdg`
    /// is not absolute on Windows, so the unix spelling would fall
    /// through to the home default there.
    #[cfg(unix)]
    const TEST_HOME: &str = "/home/u";
    #[cfg(windows)]
    const TEST_HOME: &str = r"C:\Users\u";
    #[cfg(unix)]
    const TEST_XDG: &str = "/xdg";
    #[cfg(windows)]
    const TEST_XDG: &str = r"C:\xdg";

    #[test]
    fn resolve_oscrc_path_prefers_xdg() {
        assert_eq!(
            resolve_oscrc_path(Some(TEST_XDG), Some(TEST_HOME)).unwrap(),
            PathBuf::from(TEST_XDG).join("osc").join("oscrc")
        );
        assert_eq!(
            resolve_oscrc_path(None, Some(TEST_HOME)).unwrap(),
            PathBuf::from(TEST_HOME)
                .join(".config")
                .join("osc")
                .join("oscrc")
        );
        assert_eq!(
            resolve_oscrc_path(Some("relative/xdg"), Some(TEST_HOME)).unwrap(),
            PathBuf::from(TEST_HOME)
                .join(".config")
                .join("osc")
                .join("oscrc")
        );
        assert!(resolve_oscrc_path(None, None).is_err());
    }
}
