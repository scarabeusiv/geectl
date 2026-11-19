//! The single URL builder.
//!
//! There is exactly one place that builds URLs, and one encoder per
//! context: path segments and query values are encoded differently, because
//! `+` means a space in a query value but is literal in a path segment,
//! and `/` in a path segment would change the path structure. Encoding is
//! a property of the builder, never of each caller remembering to encode.
//!
//! Rules (from the design doc):
//! - Path segments are encoded with an empty safe set except `:` (the OBS
//!   `project:package` separator) — `/` is always encoded, never passed
//!   through, and segments are never stripped or trimmed.
//! - Query values use the query encoder, where `+` is encoded (`%2B`)
//!   because a query string decodes `+` as a space.
//! - No raw-string query path: query pairs go through the encoder.

use crate::error::{ErrorKind, GeectlError};

/// Percent-encode a path segment. Alphanumerics and `-_.~:` pass through;
/// everything else (including `/`) is percent-encoded per byte.
pub fn encode_path_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Percent-encode a query value. Alphanumerics and `-_.~` pass through;
/// everything else (including `+`, which a query string decodes as a
/// space) is percent-encoded per byte.
pub fn encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// A URL built from a base, path segments, and query pairs. Segments are
/// appended in order, each encoded as one element; query pairs are encoded
/// as `name=value`. The base is used verbatim (it is a constant, not
/// caller input); only segments and query values are encoded. The base
/// must be `https://` (enforced in `new`).
#[derive(Debug, Clone)]
pub struct UrlBuilder {
    base: String,
    segments: Vec<String>,
    query: Vec<(String, String)>,
}

impl UrlBuilder {
    /// Create a builder for `base`. The base must be `https://` with a
    /// host: an `http://` base would send credentials in cleartext,
    /// and a hostless `https://` names no server, so both fail fast
    /// here instead of at the first request.
    pub fn new(base: impl Into<String>) -> Result<Self, GeectlError> {
        let base = base.into();
        if !base.starts_with("https://") {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "URL base must use https://",
            ));
        }
        // The base must carry a host: `https://` and `https:///path`
        // pass the scheme check but name no server.
        let after_scheme = &base["https://".len()..];
        let host_end = after_scheme
            .find(['/', '?', '#'])
            .unwrap_or(after_scheme.len());
        if after_scheme[..host_end].is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "URL base must have a host",
            ));
        }
        Ok(UrlBuilder {
            base,
            segments: Vec::new(),
            query: Vec::new(),
        })
    }

    /// Append one path segment. The segment is encoded as exactly one
    /// element: a `/` inside it becomes `%2F`, never a second element.
    pub fn segment(mut self, segment: &str) -> Self {
        self.segments.push(encode_path_segment(segment));
        self
    }

    /// Append one query pair.
    pub fn query(mut self, name: &str, value: &str) -> Self {
        self.query
            .push((encode_query_value(name), encode_query_value(value)));
        self
    }

    /// Build the URL string.
    pub fn build(self) -> String {
        let mut url = self.base;
        for segment in self.segments {
            if !url.ends_with('/') {
                url.push('/');
            }
            url.push_str(&segment);
        }
        if !self.query.is_empty() {
            url.push('?');
            for (i, (name, value)) in self.query.iter().enumerate() {
                if i > 0 {
                    url.push('&');
                }
                url.push_str(name);
                url.push('=');
                url.push_str(value);
            }
        }
        url
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_in_segment_produces_one_encoded_element() {
        // A segment containing `/` must become one element (%2F), never
        // two path elements — this is the injection the builder prevents.
        let url = UrlBuilder::new("https://api.opensuse.org")
            .unwrap()
            .segment("public")
            .segment("source")
            .segment("openSUSE:Factory")
            .segment("foo/bar")
            .build();
        assert_eq!(
            url,
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo%2Fbar"
        );
    }

    #[test]
    fn colon_is_preserved_in_path_segments() {
        // The OBS project:package separator is legal in a segment.
        assert_eq!(
            encode_path_segment("devel:languages:python"),
            "devel:languages:python"
        );
    }

    #[test]
    fn plus_is_encoded_in_query_values() {
        // `+` in a query value decodes as a space, so it must be encoded.
        assert_eq!(encode_query_value("gcc-c++"), "gcc-c%2B%2B");
        let url = UrlBuilder::new("https://example.com")
            .unwrap()
            .segment("search")
            .query("name", "gcc-c++")
            .build();
        assert_eq!(url, "https://example.com/search?name=gcc-c%2B%2B");
    }

    #[test]
    fn segments_are_not_stripped() {
        // No strip/lstrip on segments: a leading or trailing `/` in a
        // segment is encoded, not silently removed.
        assert_eq!(encode_path_segment("/foo/"), "%2Ffoo%2F");
    }

    #[test]
    fn http_base_is_rejected() {
        // An http base would send credentials in cleartext.
        assert!(UrlBuilder::new("http://api.opensuse.org").is_err());
        assert!(UrlBuilder::new("api.opensuse.org").is_err());
        assert!(UrlBuilder::new("https://api.opensuse.org").is_ok());
    }

    #[test]
    fn https_base_without_host_is_rejected() {
        // `https://` and `https:///path` pass the scheme check but name
        // no server; they must fail fast here, not at the first request.
        assert!(UrlBuilder::new("https://").is_err());
        assert!(UrlBuilder::new("https:///path").is_err());
        assert!(UrlBuilder::new("https://?q=1").is_err());
        assert!(UrlBuilder::new("https://api.opensuse.org").is_ok());
    }
}
