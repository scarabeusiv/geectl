//! Minimal XML document editing for the small documents geectl
//! rewrites in place: a package `_meta` and a `_link`.
//!
//! The editor is `pub(crate)`: XML never leaves the library. Callers
//! work with typed values; the edited document is only ever PUT back
//! to the server, which validates it. The parser keeps the root open
//! tag plus the child elements as raw text, so unknown elements
//! survive an edit untouched — an edit never drops a part of the
//! document it does not understand.
//!
//! The parser is hand-rolled, like the `parse_*` functions in `obs`:
//! no XML crate is a dependency. It handles what these documents
//! carry — elements (nested, self-closing), attributes (single or
//! double quoted), comments, processing instructions, and text — and
//! refuses anything else: a malformed document is an error, never a
//! guess.

use crate::error::{ErrorKind, GeectlError};

/// A parsed XML document: the root open tag plus its child elements
/// as raw text, in document order.
///
/// Text between elements (usually just whitespace) and comments are
/// kept verbatim as opaque entries, so rendering an untouched
/// document reproduces it — except the `<?xml...?>` prolog, which
/// `parse` drops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XmlDoc {
    /// The root open tag without its closing `>` and without a
    /// trailing `/` for a self-closing root, e.g.
    /// `<package name="foo" project="bar"`.
    root_open: String,
    /// The root element name, e.g. `package`.
    root_name: String,
    /// The child entries, each a complete element (`<title>..</title>`,
    /// `<lock/>`) or an opaque chunk (a comment, a processing
    /// instruction, text), in document order.
    children: Vec<String>,
    /// True when the root was self-closing (`<link .../>`): there are
    /// no children and no close tag.
    self_closing: bool,
}

/// True when `c` can appear in an XML element or attribute name here:
/// ASCII alphanumerics plus `_`, `-`, `.`, `:`. The documents geectl
/// edits use plain names (`package`, `devel`, `build`); anything else
/// is not a name.
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')
}

/// The element name starting at `start` (just past the `<`, or past
/// the `</` for a close tag): the name and the index just past it.
/// None when there is no name there.
fn parse_name(xml: &str, start: usize) -> Option<(String, usize)> {
    let bytes = xml.as_bytes();
    let mut end = start;
    while end < bytes.len() && is_name_char(bytes[end] as char) {
        end += 1;
    }
    if end == start {
        return None;
    }
    Some((xml[start..end].to_string(), end))
}

/// Scan a tag starting at `start` (the `<`) to just past its closing
/// `>`, respecting quoted attribute values (a `>` inside quotes does
/// not end the tag). Returns the end index and whether the tag is
/// self-closing (`/>`). None when the tag never closes.
pub(crate) fn scan_tag(xml: &str, start: usize) -> Option<(usize, bool)> {
    let bytes = xml.as_bytes();
    let mut i = start + 1;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) => {
                if b == q {
                    quote = None;
                }
            }
            None => match b {
                b'"' | b'\'' => quote = Some(b),
                b'>' => {
                    let self_closing = i > start + 1 && bytes[i - 1] == b'/';
                    return Some((i + 1, self_closing));
                }
                _ => {}
            },
        }
        i += 1;
    }
    None
}

/// The tag name of a child entry, or None when the entry is not an
/// element (a comment, a processing instruction, or text).
fn element_name(entry: &str) -> Option<&str> {
    let after_open = entry.strip_prefix('<')?;
    if after_open.starts_with(['/', '!', '?']) {
        return None;
    }
    let end = after_open
        .find(|c: char| !is_name_char(c))
        .unwrap_or(after_open.len());
    if end == 0 {
        return None;
    }
    Some(&after_open[..end])
}

/// Find attribute `name` in a tag's text (an open tag without its
/// closing `>`, or a whole element's open tag): the attribute value
/// with its quotes stripped, unescaped. The name must match exactly
/// and be preceded by whitespace, so `project=` does not match
/// `target_project=`. None when the attribute is absent or malformed.
pub(crate) fn find_attr(tag: &str, name: &str) -> Option<String> {
    let mut rest = tag;
    let needle = format!("{name}=");
    while let Some(pos) = rest.find(&needle) {
        if pos > 0 && !rest.as_bytes()[pos - 1].is_ascii_whitespace() {
            rest = &rest[pos + needle.len()..];
            continue;
        }
        let after_eq = &rest[pos + needle.len()..];
        let quote = after_eq.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let close = after_eq[1..].find(quote)?;
        return Some(xml_unescape(&after_eq[1..1 + close]));
    }
    None
}

/// Unescape the five standard XML entities in an attribute value.
/// Anything else (including numeric references) is left as-is: the
/// values geectl reads are OBS names and revisions, which never
/// carry them. `&amp;` is unescaped last, so `&amp;lt;` reads as
/// `&lt;`, not `<`.
fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Escape text for an XML attribute value.
fn xml_escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Byte length of the character at byte position `pos` in `s`
/// (1 when `pos` is at the end of the string).
fn char_len_at(s: &str, pos: usize) -> usize {
    s[pos..].chars().next().map_or(1, |c| c.len_utf8())
}

impl XmlDoc {
    /// Parse `xml` into a document. An optional `<?xml ...?>` prolog
    /// is skipped; anything else malformed (an unclosed tag, a
    /// mismatched close, junk after the root) is an error.
    pub(crate) fn parse(xml: &str) -> Result<XmlDoc, GeectlError> {
        let malformed = |what: &str| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS XML document: {what}"),
            )
        };
        let mut text = xml.trim();
        // An optional prolog: `<?xml ...?>`, then the root element.
        if let Some(after_prolog) = text.strip_prefix("<?") {
            let Some(end) = after_prolog.find("?>") else {
                return Err(malformed("unterminated prolog"));
            };
            text = after_prolog[end + 2..].trim();
        }
        let Some(after_open) = text.strip_prefix('<') else {
            return Err(malformed("no root element"));
        };
        if after_open.starts_with(['/', '!', '?']) {
            return Err(malformed("no root element"));
        }
        let (root_name, _) = parse_name(text, 1).ok_or_else(|| malformed("bad root name"))?;
        let (tag_end, self_closing) =
            scan_tag(text, 0).ok_or_else(|| malformed("unterminated root tag"))?;
        // The root open tag without its closing `>` (and without a
        // trailing `/` for a self-closing root). A self-closing root
        // written `<tag ... />` also leaves a trailing space in the
        // open tag; drop it so attribute edits render cleanly
        // (`<tag ... rev="42"/>`, not `<tag ...  rev="42"/>`).
        let mut root_open = text[..tag_end - 1].to_string();
        if self_closing {
            root_open.pop();
            root_open.truncate(root_open.trim_end().len());
        }
        if self_closing {
            if !text[tag_end..].trim().is_empty() {
                return Err(malformed("content after the root element"));
            }
            return Ok(XmlDoc {
                root_open,
                root_name,
                children: Vec::new(),
                self_closing: true,
            });
        }
        let mut children: Vec<String> = Vec::new();
        let mut pos = tag_end;
        let close_tag = format!("</{root_name}>");
        loop {
            if pos >= text.len() {
                return Err(malformed("unclosed root element"));
            }
            if text[pos..].starts_with(&close_tag) {
                let after = &text[pos + close_tag.len()..];
                if !after.trim().is_empty() {
                    return Err(malformed("content after the root element"));
                }
                return Ok(XmlDoc {
                    root_open,
                    root_name,
                    children,
                    self_closing: false,
                });
            }
            if text[pos..].starts_with("</") {
                return Err(malformed("mismatched close tag"));
            }
            if text[pos..].starts_with("<!--") {
                let Some(end) = text[pos + 4..].find("-->") else {
                    return Err(malformed("unterminated comment"));
                };
                children.push(text[pos..pos + 4 + end + 3].to_string());
                pos += 4 + end + 3;
                continue;
            }
            if text[pos..].starts_with("<?") {
                let Some(end) = text[pos + 2..].find("?>") else {
                    return Err(malformed("unterminated processing instruction"));
                };
                children.push(text[pos..pos + 2 + end + 2].to_string());
                pos += 2 + end + 2;
                continue;
            }
            if text[pos..].starts_with('<') {
                let element = Self::scan_element(text, pos)?;
                pos += element.len();
                children.push(element);
                continue;
            }
            // Text up to the next tag, kept verbatim.
            let next = text[pos..].find('<').unwrap_or(text.len() - pos);
            children.push(text[pos..pos + next].to_string());
            pos += next;
        }
    }

    /// Scan one complete element starting at `start` (the `<`):
    /// a self-closing element, or an open tag with its matching close
    /// tag, counting nested elements of the same name. Comments,
    /// processing instructions, and CDATA sections inside are skipped
    /// as opaque text, so a `<` inside them cannot end the element.
    fn scan_element(xml: &str, start: usize) -> Result<String, GeectlError> {
        let malformed = || {
            GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS XML document: unterminated element".to_string(),
            )
        };
        let (name, _) = parse_name(xml, start + 1).ok_or_else(malformed)?;
        let (tag_end, self_closing) = scan_tag(xml, start).ok_or_else(malformed)?;
        if self_closing {
            return Ok(xml[start..tag_end].to_string());
        }
        let mut depth = 1;
        let mut pos = tag_end;
        let open_prefix = format!("<{name}");
        let close_tag = format!("</{name}>");
        while pos < xml.len() {
            if xml[pos..].starts_with("<!--") {
                let Some(end) = xml[pos + 4..].find("-->") else {
                    return Err(malformed());
                };
                pos += 4 + end + 3;
                continue;
            }
            if xml[pos..].starts_with("<?") {
                let Some(end) = xml[pos + 2..].find("?>") else {
                    return Err(malformed());
                };
                pos += 2 + end + 2;
                continue;
            }
            if xml[pos..].starts_with("<![CDATA[") {
                let Some(end) = xml[pos + 9..].find("]]>") else {
                    return Err(malformed());
                };
                pos += 9 + end + 3;
                continue;
            }
            if xml[pos..].starts_with(&close_tag) {
                depth -= 1;
                pos += close_tag.len();
                if depth == 0 {
                    return Ok(xml[start..pos].to_string());
                }
                continue;
            }
            if xml[pos..].starts_with(&open_prefix) {
                // A nested element of the same name: it opens a new
                // level unless it is self-closing. The name must end
                // here, so `<linkage>` does not count as `<link>`.
                let after = &xml[pos + open_prefix.len()..];
                let Some(first) = after.chars().next() else {
                    return Err(malformed());
                };
                if !(first.is_whitespace() || first == '/' || first == '>') {
                    pos += char_len_at(xml, pos);
                    continue;
                }
                let (end, nested_self_closing) = scan_tag(xml, pos).ok_or_else(malformed)?;
                if !nested_self_closing {
                    depth += 1;
                }
                pos = end;
                continue;
            }
            // Step one character, not one byte: the text here can
            // hold multi-byte characters (non-ASCII titles or
            // descriptions), and slicing at a byte inside one panics.
            pos += char_len_at(xml, pos);
        }
        Err(malformed())
    }

    /// The value of attribute `name` on the root element, or None
    /// when it is absent.
    pub(crate) fn root_attr(&self, name: &str) -> Option<String> {
        find_attr(&self.root_open, name)
    }

    /// Set attribute `name` on the root element to `value`, replacing
    /// it when it is already there.
    pub(crate) fn set_root_attr(&mut self, name: &str, value: &str) {
        let escaped = xml_escape_attr(value);
        let needle = format!("{name}=");
        let mut rest = self.root_open.clone();
        let mut search_from = 0;
        while let Some(pos) = rest[search_from..].find(&needle) {
            let pos = search_from + pos;
            if pos > 0 && !rest.as_bytes()[pos - 1].is_ascii_whitespace() {
                search_from = pos + needle.len();
                continue;
            }
            let after_eq = &rest[pos + needle.len()..];
            let Some(quote) = after_eq.chars().next() else {
                return;
            };
            if quote != '"' && quote != '\'' {
                return;
            }
            let Some(close) = after_eq[1..].find(quote) else {
                return;
            };
            let value_start = pos + needle.len() + 1;
            let value_end = value_start + close;
            rest.replace_range(value_start..value_end, &escaped);
            self.root_open = rest;
            return;
        }
        // Absent: append it to the root open tag.
        self.root_open.push(' ');
        self.root_open.push_str(name);
        self.root_open.push_str("=\"");
        self.root_open.push_str(&escaped);
        self.root_open.push('"');
    }

    /// The raw text of the first child element named `name`, or None
    /// when there is no such child. Text, comments, and processing
    /// instructions are never elements, so they are never returned.
    pub(crate) fn child(&self, name: &str) -> Option<String> {
        self.children
            .iter()
            .find(|child| element_name(child) == Some(name))
            .cloned()
    }

    /// Drop the first child element named one of `names` and
    /// everything after it. True when anything was dropped; false
    /// when no child carries one of the names. Entries before the
    /// first match survive untouched, including the text between them.
    pub(crate) fn truncate_at_element(&mut self, names: &[&str]) -> bool {
        let Some(at) = self
            .children
            .iter()
            .position(|child| element_name(child).is_some_and(|name| names.contains(&name)))
        else {
            return false;
        };
        self.children.truncate(at);
        true
    }

    /// True when any child is an element (as opposed to text, a
    /// comment, or a processing instruction).
    pub(crate) fn has_elements(&self) -> bool {
        self.children
            .iter()
            .any(|child| element_name(child).is_some())
    }

    /// Remove attribute `name` from the root element, when it is
    /// there. The whitespace before the attribute goes with it, so
    /// the tag stays well-formed.
    pub(crate) fn remove_root_attr(&mut self, name: &str) {
        let needle = format!("{name}=");
        let mut search_from = 0;
        while let Some(found) = self.root_open[search_from..].find(&needle) {
            let pos = search_from + found;
            if pos > 0 && !self.root_open.as_bytes()[pos - 1].is_ascii_whitespace() {
                search_from = pos + needle.len();
                continue;
            }
            let after_eq = &self.root_open[pos + needle.len()..];
            let Some(quote) = after_eq.chars().next() else {
                return;
            };
            if quote != '"' && quote != '\'' {
                return;
            }
            let Some(close) = after_eq[1..].find(quote) else {
                return;
            };
            let value_end = pos + needle.len() + 1 + close + 1;
            let mut start = pos;
            while start > 0 && self.root_open.as_bytes()[start - 1].is_ascii_whitespace() {
                start -= 1;
            }
            self.root_open.replace_range(start..value_end, "");
            return;
        }
    }

    /// Remove every child element named `name`. Opaque entries
    /// (comments, text) are never elements, so they survive.
    pub(crate) fn remove_elements(&mut self, name: &str) {
        self.children
            .retain(|child| element_name(child) != Some(name));
    }

    /// Replace the child element named `name` with `xml`, or append
    /// `xml` when there is no such element. `xml` must be a single
    /// element with that name; anything else is an error, so a bad
    /// replacement can never be rendered.
    pub(crate) fn replace_or_insert_element(
        &mut self,
        name: &str,
        xml: &str,
    ) -> Result<(), GeectlError> {
        let parsed = XmlDoc::parse(xml).map_err(|_| {
            GeectlError::new(
                ErrorKind::InvalidInput,
                format!("not a single <{name}> element: {xml:?}"),
            )
        })?;
        if parsed.root_name != name {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("not a single <{name}> element: {xml:?}"),
            ));
        }
        self.remove_elements(name);
        self.children.push(xml.trim().to_string());
        self.self_closing = false;
        Ok(())
    }

    /// Render the document back to XML.
    pub(crate) fn render(&self) -> String {
        if self.self_closing && self.children.is_empty() {
            return format!("{}/>", self.root_open);
        }
        let mut out = String::new();
        out.push_str(&self.root_open);
        out.push('>');
        for child in &self.children {
            out.push_str(child);
        }
        out.push_str("</");
        out.push_str(&self.root_name);
        out.push('>');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const META: &str = "<package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description><devel project=\"test:devel\"/><person userid=\"alice\" role=\"maintainer\"/><lock><enable/></lock></package>";

    #[test]
    fn parse_and_render_round_trips_a_meta_document() {
        let doc = XmlDoc::parse(META).unwrap();
        assert_eq!(doc.render(), META);
    }

    #[test]
    fn parse_round_trips_non_ascii_text() {
        // Non-ASCII titles and descriptions: the element scanner
        // steps over the text one character at a time, and stepping
        // a byte at a time would land inside a multi-byte character
        // and panic on the next slice.
        let meta = "<package name=\"test-pkgA\" project=\"test:factory\"><title>Příliš žluťoučký kůň</title><description>Ünïcödé — — —</description></package>";
        let doc = XmlDoc::parse(meta).unwrap();
        assert_eq!(doc.render(), meta);
    }

    #[test]
    fn scan_element_skips_a_longer_name_with_the_same_prefix() {
        // `<packageX>` is not a nested `<package>`: the scanner
        // steps past the longer name, then over the multi-byte
        // title text, and still finds the matching close tag.
        let doc =
            XmlDoc::parse("<package><packageX/><title>Žluťoučký kůň</title></package>").unwrap();
        assert_eq!(
            doc.render(),
            "<package><packageX/><title>Žluťoučký kůň</title></package>"
        );
    }

    #[test]
    fn child_returns_the_first_matching_element() {
        let doc = XmlDoc::parse(META).unwrap();
        assert_eq!(
            doc.child("devel").as_deref(),
            Some("<devel project=\"test:devel\"/>")
        );
        assert_eq!(
            doc.child("person").as_deref(),
            Some("<person userid=\"alice\" role=\"maintainer\"/>")
        );
        assert!(doc.child("missing").is_none());
    }

    #[test]
    fn truncate_at_element_drops_the_match_and_after() {
        let mut inner =
            XmlDoc::parse("<patches><topadd name=\"top.patch\"/><apply name=\"bad.patch\"/><delete name=\"gone\"/></patches>")
                .unwrap();
        assert!(inner.truncate_at_element(&["apply", "delete"]));
        assert_eq!(
            inner.render(),
            "<patches><topadd name=\"top.patch\"/></patches>"
        );
        assert!(inner.has_elements());
        // A second truncation finds nothing more to drop.
        assert!(!inner.truncate_at_element(&["apply", "delete"]));
    }

    #[test]
    fn truncate_at_element_false_without_a_match() {
        let mut inner = XmlDoc::parse("<patches><topadd name=\"top.patch\"/></patches>").unwrap();
        assert!(!inner.truncate_at_element(&["apply", "delete"]));
        assert_eq!(
            inner.render(),
            "<patches><topadd name=\"top.patch\"/></patches>"
        );
    }

    #[test]
    fn has_elements_ignores_text_and_comments() {
        let doc = XmlDoc::parse("<package>text<!-- comment --></package>").unwrap();
        assert!(!doc.has_elements());
        assert!(doc.child("title").is_none());
        let doc = XmlDoc::parse("<package><title>Test</title></package>").unwrap();
        assert!(doc.has_elements());
    }

    #[test]
    fn remove_root_attr_drops_the_attribute_and_its_space() {
        let mut doc = XmlDoc::parse(META).unwrap();
        doc.remove_root_attr("project");
        assert_eq!(
            doc.render(),
            "<package name=\"test-pkgA\"><title>Test</title><description>A test package</description><devel project=\"test:devel\"/><person userid=\"alice\" role=\"maintainer\"/><lock><enable/></lock></package>"
        );
        // Removing an absent attribute changes nothing.
        doc.remove_root_attr("project");
        assert_eq!(doc.root_attr("project"), None);
        // Removing the first attribute leaves a well-formed tag.
        let mut doc = XmlDoc::parse("<link rev=\"42\" package=\"srcpkg\"/>").unwrap();
        doc.remove_root_attr("rev");
        assert_eq!(doc.render(), "<link package=\"srcpkg\"/>");
    }

    #[test]
    fn remove_elements_drops_only_the_named_ones() {
        let mut doc = XmlDoc::parse(META).unwrap();
        doc.remove_elements("devel");
        doc.remove_elements("person");
        doc.remove_elements("lock");
        assert_eq!(
            doc.render(),
            "<package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description></package>"
        );
        // Removing an absent element is a no-op, not an error.
        doc.remove_elements("devel");
        assert!(!doc.render().contains("<devel"));
    }

    #[test]
    fn set_root_attr_replaces_and_appends() {
        let mut doc = XmlDoc::parse(META).unwrap();
        doc.set_root_attr("project", "test:target");
        assert!(doc.render().contains("project=\"test:target\""));
        doc.set_root_attr("name", "renamed");
        assert!(doc.render().contains("name=\"renamed\""));
        // An escaped value is written escaped.
        doc.set_root_attr("project", "a&b");
        assert!(doc.render().contains("project=\"a&amp;b\""));
    }

    #[test]
    fn replace_or_insert_element_replaces_and_appends() {
        let mut doc = XmlDoc::parse(META).unwrap();
        doc.replace_or_insert_element("build", "<build><disable/></build>")
            .unwrap();
        assert!(doc.render().contains("<build><disable/></build>"));
        // Replacing again swaps the element, it does not duplicate it.
        doc.replace_or_insert_element("build", "<build><enable/></build>")
            .unwrap();
        assert_eq!(doc.render().matches("<build>").count(), 1);
        assert!(doc.render().contains("<build><enable/></build>"));
    }

    #[test]
    fn replace_or_insert_element_rejects_a_wrong_element() {
        let mut doc = XmlDoc::parse(META).unwrap();
        assert!(doc
            .replace_or_insert_element("build", "<publish><disable/></publish>")
            .is_err());
        assert!(doc.replace_or_insert_element("build", "not xml").is_err());
        assert!(doc.replace_or_insert_element("build", "<build>").is_err());
    }

    #[test]
    fn parse_handles_a_self_closing_root_and_inserting_a_child() {
        let mut doc = XmlDoc::parse("<link project=\"a\" package=\"b\"/>").unwrap();
        assert!(doc.self_closing);
        assert_eq!(doc.render(), "<link project=\"a\" package=\"b\"/>");
        doc.replace_or_insert_element("apply", "<apply name=\"fix.diff\"/>")
            .unwrap();
        assert!(!doc.self_closing);
        assert_eq!(
            doc.render(),
            "<link project=\"a\" package=\"b\"><apply name=\"fix.diff\"/></link>"
        );
    }

    #[test]
    fn parse_keeps_comments_and_text_verbatim() {
        let xml = "<package name=\"p\"><!-- a note --><title>T</title>\n  <description>D</description></package>";
        let doc = XmlDoc::parse(xml).unwrap();
        assert_eq!(doc.render(), xml);
        let mut doc = doc;
        doc.remove_elements("title");
        assert!(doc.render().contains("<!-- a note -->"));
    }

    #[test]
    fn parse_rejects_malformed_documents() {
        // Unclosed root.
        assert!(XmlDoc::parse("<package name=\"p\"><title>T</title>").is_err());
        // Mismatched close.
        assert!(XmlDoc::parse("<package name=\"p\"></project>").is_err());
        // Junk after the root.
        assert!(XmlDoc::parse("<package name=\"p\"/> extra").is_err());
        // Unclosed child element.
        assert!(XmlDoc::parse("<package name=\"p\"><title>T</package>").is_err());
        // No root element at all.
        assert!(XmlDoc::parse("").is_err());
        assert!(XmlDoc::parse("just text").is_err());
        // Unterminated root tag.
        assert!(XmlDoc::parse("<package name=\"p\"").is_err());
        // A longer name is not the root close.
        assert!(XmlDoc::parse("<package name=\"p\"></packages>").is_err());
    }

    #[test]
    fn parse_skips_a_prolog() {
        let doc = XmlDoc::parse("<?xml version=\"1.0\"?><package name=\"p\"/>").unwrap();
        assert_eq!(doc.render(), "<package name=\"p\"/>");
    }

    #[test]
    fn xml_unescape_replaces_ampersand_last() {
        // `&amp;lt;` is an escaped `&lt;`, not a `<`: unescaping
        // `&amp;` first would read it as a literal `<`.
        assert_eq!(xml_unescape("&amp;lt;"), "&lt;");
        assert_eq!(xml_unescape("&lt;"), "<");
        assert_eq!(xml_unescape("&amp;"), "&");
    }

    #[test]
    fn nested_same_name_elements_parse_whole() {
        // A <build> containing a nested <build> is pathological, but
        // the depth count must keep the outer element whole.
        let xml = "<package name=\"p\"><build><build><disable/></build></build></package>";
        let doc = XmlDoc::parse(xml).unwrap();
        assert_eq!(doc.render(), xml);
    }
}
