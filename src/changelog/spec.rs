//! Parsing of spec files: the tags and lists the validation rules
//! and the generator read — `Name:`, `Version:`, `License:`,
//! `%changelog`, subpackages (`%package`), `Source:`/`Patch:` values,
//! and `Requires:`/`BuildRequires:` lines. First occurrence wins for
//! the tags; a value is the rest of the line, trimmed, macros left
//! as-is.

/// The parsed facts of one spec file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpecInfo {
    pub name: Option<String>,
    pub version: Option<String>,
    pub license: Option<String>,
    pub has_changelog: bool,
    /// Full subpackage names (the main package name included only
    /// when a bare `%package` names it; see [`parse_spec`]).
    pub subpackages: Vec<String>,
    /// `Source:` values, in order.
    pub sources: Vec<String>,
    /// `Patch:` values, in order.
    pub patches: Vec<String>,
    /// `Requires:` values (any qualifier), in order.
    pub requires: Vec<String>,
    /// `BuildRequires:` values, in order.
    pub build_requires: Vec<String>,
}

fn tag_value<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(tag)?;
    let rest = rest.strip_prefix(':')?;
    Some(rest.trim())
}

/// Parse a spec file's text.
pub fn parse_spec(text: &str) -> SpecInfo {
    let mut info = SpecInfo::default();
    for line in text.lines() {
        if info.name.is_none() {
            if let Some(value) = tag_value(line, "Name") {
                if !value.is_empty() {
                    info.name = Some(value.to_string());
                }
            }
        }
        if info.version.is_none() {
            if let Some(value) = tag_value(line, "Version") {
                if !value.is_empty() {
                    info.version = Some(value.to_string());
                }
            }
        }
        if info.license.is_none() {
            if let Some(value) = tag_value(line, "License") {
                if !value.is_empty() {
                    info.license = Some(value.to_string());
                }
            }
        }
        let trimmed = line.trim_start();
        if trimmed == "%changelog"
            || trimmed.starts_with("%changelog ")
            || trimmed.starts_with("%changelog\t")
        {
            info.has_changelog = true;
        }
        if trimmed == "%package"
            || trimmed.starts_with("%package ")
            || trimmed.starts_with("%package\t")
        {
            let args: Vec<&str> = trimmed.split_whitespace().skip(1).collect();
            let main = info.name.clone().unwrap_or_default();
            let sub = subpackage_name(&main, &args);
            if let Some(sub) = sub {
                if !info.subpackages.contains(&sub) {
                    info.subpackages.push(sub);
                }
            }
        }
        if let Some(value) = tag_value(line, "Source") {
            // `Source:` with no number, or `Source0:` etc. — the tag
            // prefix match above takes the longest tag only when the
            // line starts with it; numbered forms need the digit
            // stripped first.
            info.sources.push(value.to_string());
        } else if let Some(value) = numbered_tag_value(line, "Source") {
            info.sources.push(value.to_string());
        }
        if let Some(value) = tag_value(line, "Patch") {
            info.patches.push(value.to_string());
        } else if let Some(value) = numbered_tag_value(line, "Patch") {
            info.patches.push(value.to_string());
        }
        if let Some(value) = requires_value(line, "Requires") {
            info.requires.push(value.to_string());
        }
        if let Some(value) = requires_value(line, "BuildRequires") {
            info.build_requires.push(value.to_string());
        }
    }
    info
}

/// `Source0:` / `Patch12:` forms: the tag plus digits, then `:`.
fn numbered_tag_value<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(tag)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let rest = rest[digits.len()..].strip_prefix(':')?;
    let value = rest.trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// `Requires:` / `BuildRequires:`, with an optional `(qualifier)`.
fn requires_value<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(tag)?;
    let rest = if let Some(rest) = rest.strip_prefix('(') {
        let (_, after) = rest.split_once(')')?;
        after
    } else {
        rest
    };
    let rest = rest.strip_prefix(':')?;
    let value = rest.trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// The subpackage a `%package` line names: `-n <name>` is the full
/// name, a bare argument is `<main>-<arg>`, no argument is the main
/// package itself. `None` when the line carries only flags and no
/// name (a bare `%package` before `Name:` is known).
fn subpackage_name(main: &str, args: &[&str]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        let arg = args[i];
        if arg == "-n" {
            return args.get(i + 1).map(|name| (*name).to_string());
        }
        if arg.starts_with('-') {
            i += 1;
            continue;
        }
        if main.is_empty() {
            return None;
        }
        return Some(format!("{main}-{arg}"));
    }
    if main.is_empty() {
        None
    } else {
        Some(main.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = "Name:           foo\nVersion:        2.0\nLicense:        GPL-3.0-only\nSource0:        %{name}-%{version}.tar.gz\nSource1:        foo.service\nPatch0:         fix-crash.patch\nBuildRequires:  pkgconfig\nRequires:       bar >= 1.0\n%package -n foo-doc\nSummary: docs\n%package devel\nSummary: devel\n%changelog\n";

    #[test]
    fn parse_spec_reads_tags_sources_and_subpackages() {
        let info = parse_spec(SPEC);
        assert_eq!(info.name.as_deref(), Some("foo"));
        assert_eq!(info.version.as_deref(), Some("2.0"));
        assert_eq!(info.license.as_deref(), Some("GPL-3.0-only"));
        assert!(info.has_changelog);
        assert_eq!(info.sources, ["%{name}-%{version}.tar.gz", "foo.service"]);
        assert_eq!(info.patches, ["fix-crash.patch"]);
        assert_eq!(info.build_requires, ["pkgconfig"]);
        assert_eq!(info.requires, ["bar >= 1.0"]);
        assert_eq!(info.subpackages, ["foo-doc", "foo-devel"]);
    }

    #[test]
    fn parse_spec_without_a_changelog_reports_it() {
        let info = parse_spec("Name: foo\nVersion: 1.0\n");
        assert!(!info.has_changelog);
        assert_eq!(info.subpackages, Vec::<String>::new());
    }

    #[test]
    fn numbered_tags_parse() {
        assert_eq!(
            numbered_tag_value("Source12: foo.tar.gz", "Source"),
            Some("foo.tar.gz")
        );
        assert_eq!(numbered_tag_value("Source: foo.tar.gz", "Source"), None);
        assert_eq!(
            numbered_tag_value("Patch0: a.patch", "Patch"),
            Some("a.patch")
        );
    }

    #[test]
    fn qualified_requires_parse() {
        assert_eq!(
            requires_value("Requires(post): /bin/sh", "Requires"),
            Some("/bin/sh")
        );
        assert_eq!(requires_value("Requires: bar", "Requires"), Some("bar"));
        assert_eq!(requires_value("BuildRequires: pkgconfig", "Requires"), None);
    }
}
