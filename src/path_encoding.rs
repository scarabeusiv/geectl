//! One percent-encoding for names that become path segments.
//!
//! Both the working copy's checkout directories and the local
//! build's build roots name OBS projects and packages on disk, so
//! both use this encoding: `:` becomes `%3A`, `%` becomes `%25`.
//! It is injective — distinct names always map to distinct
//! directories — and reversible. This differs from
//! `crate::url::encode_path_segment`, which keeps `:` legal in a
//! URL segment and must not be used for a filesystem path.

/// A path segment for a project, package, repository, arch, or
/// flavor name: `:` is legal in an OBS name but not in a Windows
/// path, so it is percent-encoded (`:` becomes `%3A`, `%`
/// becomes `%25`). The encoding is injective — distinct names
/// always map to distinct directories — and reversible (see
/// `decode_path_segment`); a name without `%` or `:` is
/// unchanged. Only `:` needs encoding: it is the one
/// Windows-illegal character a validated OBS name
/// (`[A-Za-z0-9_.+-:]`) can carry, and `%` is escaped so the
/// encoding stays injective even for unvalidated input. The
/// marker in the build root — and the record in the checkout's
/// git config — keeps the real name; the directory name never
/// names the project or package back.
pub fn encode_path_segment(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            '%' => out.push_str("%25"),
            ':' => out.push_str("%3A"),
            c => out.push(c),
        }
    }
    out
}

/// The inverse of `encode_path_segment`: `%25` becomes `%`,
/// `%3A` becomes `:`. A `%` not followed by `25`, `3A`, or `3a`
/// passes through unchanged, so decoding a directory name the
/// encoder never wrote still yields a usable name, never a
/// failure.
pub fn decode_path_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    let mut rest = segment;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if let Some(tail) = after.strip_prefix("25") {
            out.push('%');
            rest = tail;
        } else if let Some(tail) = after
            .strip_prefix("3A")
            .or_else(|| after.strip_prefix("3a"))
        {
            out.push(':');
            rest = tail;
        } else {
            out.push('%');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_percent_encodes_colon_and_percent() {
        // Names without `%` or `:` are unchanged.
        assert_eq!(encode_path_segment("home_foo"), "home_foo");
        assert_eq!(encode_path_segment("home:foo"), "home%3Afoo");
        // `%` itself is escaped, so a literal `%3A` in a name
        // cannot collide with an encoded `:`.
        assert_eq!(encode_path_segment("home%3Afoo"), "home%253Afoo");
    }

    #[test]
    fn decode_round_trips_the_encoding() {
        for name in ["home:foo", "home%3Afoo", "home_foo", "plain"] {
            assert_eq!(decode_path_segment(&encode_path_segment(name)), name);
        }
    }

    #[test]
    fn distinct_names_stay_distinct() {
        // `home:foo` and `home_foo` are different names with
        // different directories — a second checkout or build
        // never reuses the first one's directory.
        assert_ne!(
            encode_path_segment("home:foo"),
            encode_path_segment("home_foo")
        );
    }
}
