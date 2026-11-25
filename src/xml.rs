//! Minimal XML document editing for the small documents geectl
//! rewrites in place: a package `_meta`, a project `_meta`, and a `_link`.
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
//! double quoted), comments, processing instructions, CDATA sections,
//! and text — and refuses anything else: a malformed document is an
//! error, never a guess.

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

/// True when `c` can start an XML element or attribute name here:
/// ASCII alphabetic plus `_` and `:`. Digits, `-` and `.` can
/// appear in a name but never start one, so a `<` followed by
/// one of those is bare text, not a tag.
fn is_name_start_char(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c, '_' | ':')
}

/// True when `text` holds a `<` outside a quoted section (either
/// quote kind). A nested tag's own span cannot hold one: a `<`
/// there means the `>` that ended the span belongs to a later
/// construct, not to this tag.
fn has_bare_lt(text: &str) -> bool {
    let mut quote: Option<char> = None;
    for c in text.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' => quote = Some(c),
                '<' => return true,
                _ => {}
            },
        }
    }
    false
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

/// The value of attribute `attr` on the open tag of the child element
/// `entry`, or None when the entry is not an element (a comment, a
/// processing instruction, or text) or the attribute is absent. The
/// open tag ends at the first `>` outside a quoted value, so a `>`
/// inside a value cannot end it early.
fn child_open_tag_attr(entry: &str, attr: &str) -> Option<String> {
    let (end, _) = scan_tag(entry, 0)?;
    find_attr(&entry[..end], attr)
}

/// Find attribute `name` in a tag's text (an open tag without its
/// closing `>`, or a whole element's open tag): the attribute value
/// with its quotes stripped, unescaped. The name must match exactly
/// and be preceded by whitespace, so `project=` does not match
/// `target_project=`. None when the attribute is absent or malformed.
/// The scan is quote-aware: a `name=` inside an earlier attribute's
/// value (e.g. `title="see name=x"`) must not match, and a malformed
/// occurrence (an unquoted value) is skipped, not fatal — the
/// attribute may still follow. Whitespace around `=` is allowed:
/// `name = "value"` is the same attribute.
pub(crate) fn find_attr(tag: &str, name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    let bytes = tag.as_bytes();
    let first = name.as_bytes()[0];
    let mut in_quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = in_quote {
            if b == q {
                in_quote = None;
            }
            i += 1;
            continue;
        }
        if b == b'"' || b == b'\'' {
            in_quote = Some(b);
            i += 1;
            continue;
        }
        if b == first
            && tag[i..].starts_with(name)
            && (i == 0 || bytes[i - 1].is_ascii_whitespace())
        {
            // The name must be followed by `=`, with optional
            // whitespace around it.
            let mut j = i + name.len();
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'=' {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                    let quote = bytes[j];
                    let after = &tag[j + 1..];
                    let close = after.find(quote as char)?;
                    return Some(xml_unescape(&after[..close]));
                }
            }
        }
        i += 1;
    }
    None
}

/// XML whitespace, one definition per domain. Attribute values use
/// Unicode whitespace (`str::trim`): a whitespace-only value is
/// blank, and NBSP counts as whitespace. Element content uses only
/// the four XML whitespace characters (space, tab, CR, LF), so NBSP
/// between children is character data, not formatting. The attribute
/// side is the rule for `tag_attr`, `xml_attr_non_blank`, and the
/// remote-parser optional-attribute reads.
pub(crate) fn xml_attr_blank(value: &str) -> bool {
    value.trim().is_empty()
}

/// Unescape XML text or an attribute value: the five entities
/// [`xml_escape`] produces, plus numeric character references
/// (`&#65;`, `&#x41;`). Anything else is left verbatim: an unknown
/// entity is a server response the library does not understand, and
/// passing it through beats inventing a character.
pub(crate) fn xml_unescape(s: &str) -> String {
    // Fast path: nothing to unescape.
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        match after.find(';') {
            Some(semi) => {
                let entity = &after[..semi];
                match entity {
                    "amp" => out.push('&'),
                    "lt" => out.push('<'),
                    "gt" => out.push('>'),
                    "quot" => out.push('"'),
                    "apos" => out.push('\''),
                    _ => {
                        let numeric = entity
                            .strip_prefix("#x")
                            .or_else(|| entity.strip_prefix("#X"))
                            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                            .or_else(|| {
                                entity
                                    .strip_prefix('#')
                                    .and_then(|dec| dec.parse::<u32>().ok())
                            })
                            .and_then(char::from_u32);
                        match numeric {
                            Some(c) => out.push(c),
                            // Unknown entity: keep it verbatim.
                            None => {
                                out.push('&');
                                out.push_str(entity);
                                out.push(';');
                            }
                        }
                    }
                }
                rest = &after[semi + 1..];
            }
            None => {
                // A bare `&` with no `;` after it: keep it verbatim.
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Escape text for an XML element or attribute value.
pub(crate) fn xml_escape(s: &str) -> String {
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
            if text[pos..].starts_with("<![CDATA[") {
                // A CDATA section is character data, kept verbatim as
                // an opaque entry like text: it carries no element
                // name, so the name-based edits never touch it.
                let Some(end) = text[pos + 9..].find("]]>") else {
                    return Err(malformed("unterminated CDATA section"));
                };
                children.push(text[pos..pos + 9 + end + 3].to_string());
                pos += 9 + end + 3;
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

    /// The root element name, e.g. `project`.
    pub(crate) fn root_name(&self) -> &str {
        &self.root_name
    }

    /// Set attribute `name` on the root element to `value`, replacing
    /// it when it is already there. The value range is computed first
    /// and replaced in place; the root open tag is never cloned.
    pub(crate) fn set_root_attr(&mut self, name: &str, value: &str) {
        let escaped = xml_escape(value);
        let needle = format!("{name}=");
        let mut search_from = 0;
        while let Some(pos) = self.root_open[search_from..].find(&needle) {
            let pos = search_from + pos;
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
            let value_start = pos + needle.len() + 1;
            let value_end = value_start + close;
            self.root_open
                .replace_range(value_start..value_end, &escaped);
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

    /// The names of the direct child elements, in document order.
    /// Opaque entries (comments, processing instructions, text)
    /// carry no name and are skipped.
    pub(crate) fn child_names(&self) -> Vec<&str> {
        self.children
            .iter()
            .filter_map(|entry| element_name(entry))
            .collect()
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

    /// True when any child is not whitespace-only text: an element, a
    /// comment, a processing instruction, or text with content.
    /// Callers use this to decide whether an emptied section is
    /// really empty: whitespace-only children carry no content to
    /// preserve.
    pub(crate) fn has_content(&self) -> bool {
        self.children
            .iter()
            .any(|child| element_name(child).is_some() || !child.trim().is_empty())
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

    /// True when a child element named `name` carries attribute `attr`
    /// with value `value`. Opaque entries are never elements, so they
    /// never match.
    pub(crate) fn has_element_with_attr(&self, name: &str, attr: &str, value: &str) -> bool {
        self.children.iter().any(|child| {
            element_name(child) == Some(name)
                && child_open_tag_attr(child, attr).as_deref() == Some(value)
        })
    }

    /// Remove the child elements named `name` whose attribute `attr`
    /// equals `value`. True when anything was removed. Other elements
    /// with the same name survive, as do opaque entries.
    pub(crate) fn remove_elements_with_attr(
        &mut self,
        name: &str,
        attr: &str,
        value: &str,
    ) -> bool {
        let before = self.children.len();
        self.children.retain(|child| {
            !(element_name(child) == Some(name)
                && child_open_tag_attr(child, attr).as_deref() == Some(value))
        });
        self.children.len() != before
    }

    /// Append `xml` as a new child element, after the existing
    /// children. `xml` must be a single element; anything else is an
    /// error, so a bad fragment can never be rendered. A self-closing
    /// root gains children, so it stops being self-closing.
    pub(crate) fn append_element(&mut self, xml: &str) -> Result<(), GeectlError> {
        // A prolog-prefixed fragment would embed the prolog
        // mid-document: `parse` skips it, but the raw string is what
        // gets appended. Reject it here instead.
        if xml.trim_start().starts_with("<?") {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("not a single element: {xml:?}"),
            ));
        }
        // Only the validation matters: the raw fragment is appended,
        // so the document keeps its own formatting.
        let _parsed = XmlDoc::parse(xml).map_err(|_| {
            GeectlError::new(
                ErrorKind::InvalidInput,
                format!("not a single element: {xml:?}"),
            )
        })?;
        self.children.push(xml.trim().to_string());
        self.self_closing = false;
        Ok(())
    }

    /// Replace the child element named `name` with `xml`, or append
    /// `xml` when there is no such element. `xml` must be a single
    /// element with that name; anything else is an error, so a bad
    /// replacement can never be rendered. The replacement takes the
    /// place of the first matching child, so the document order is
    /// preserved; any further duplicates are dropped. The old
    /// remove-then-push moved the element to the end, which reordered
    /// the document and caused a spurious extra PUT on the next
    /// no-change comparison.
    pub(crate) fn replace_or_insert_element(
        &mut self,
        name: &str,
        xml: &str,
    ) -> Result<(), GeectlError> {
        // A prolog-prefixed fragment would embed the prolog
        // mid-document: `parse` skips it, but the raw string is what
        // gets stored. Reject it here instead — the same guard as
        // `append_element`.
        if xml.trim_start().starts_with("<?") {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("not a single <{name}> element: {xml:?}"),
            ));
        }
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
        let replacement = xml.trim().to_string();
        match self
            .children
            .iter()
            .position(|child| element_name(child) == Some(name))
        {
            Some(idx) => {
                self.children[idx] = replacement;
                // Drop any further duplicates, keeping the replaced
                // element at its original position.
                let mut i = idx + 1;
                while i < self.children.len() {
                    if element_name(&self.children[i]) == Some(name) {
                        self.children.remove(i);
                    } else {
                        i += 1;
                    }
                }
            }
            None => self.children.push(replacement),
        }
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

/// Read the value of attribute `name` in an opening tag's text (the
/// text between `<tag` and `>`). Either quote style works, and the
/// attribute name must stand on its own — `valid="..."` must not match
/// a lookup for `id` — so the character before the name has to be
/// whitespace. None when the attribute is absent or unquoted. The
/// value is XML-unescaped: an attribute the server sent as
/// `details="a &amp; b"` reads as `a & b`.
/// The index of the first `>` in `s` that is not inside a quoted
/// attribute value, if any. A `>` inside a value (e.g. in a `details`
/// attribute) does not end the tag; scanning for it naively would
/// truncate the tag and drop the attributes after it.
pub(crate) fn find_tag_end(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut in_quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = in_quote {
            if b == q {
                in_quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            in_quote = Some(b);
        } else if b == b'>' {
            return Some(i);
        }
        i += 1;
    }
    None
}
/// Skip a leading XML prolog (`<?xml ...?>`) in `body`, if present.
/// The input is expected to be already trimmed: a prolog is only
/// recognised at the very start, and anything after it is trimmed of
/// leading whitespace. Without a prolog the input is returned
/// unchanged. The prolog must terminate (`?>`) — an unterminated
/// prolog is an error, not a missing document — and `<?xmlfoo` is not
/// a prolog at all, so it is an error too.
pub(crate) fn skip_xml_prolog(body: &str) -> Result<&str, ()> {
    let Some(after) = body.strip_prefix("<?xml") else {
        return Ok(body);
    };
    let first = after.chars().next();
    if !matches!(first, Some(c) if c.is_whitespace() || c == '?') {
        return Err(());
    }
    let Some(end) = after.find("?>") else {
        return Err(());
    };
    Ok(after[end + 2..].trim_start())
}

/// Each occurrence of element `name` in `body`, as its full text
/// including the tags. The tag name must end at whitespace, `/`, or
/// `>` (so `<resultlist>` never matches a search for `<result`).
/// An unterminated element is a server error: the scan fails instead
/// of returning the partial list, so a truncated document never reads
/// as a smaller matrix.
pub(crate) fn xml_elements<'a>(body: &'a str, name: &str) -> Result<Vec<&'a str>, GeectlError> {
    let unterminated = || {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected OBS response: unterminated <{name}> element"),
        )
    };
    let mut out = Vec::new();
    let mut offset = 0;
    let open_marker = format!("<{name}");
    let close_marker = format!("</{name}>");
    while offset < body.len() {
        let rest = &body[offset..];
        let Some(pos) = rest.find(&open_marker) else {
            break;
        };
        let after_tag = &rest[pos + open_marker.len()..];
        let Some(first) = after_tag.chars().next() else {
            return Err(unterminated());
        };
        if !(first.is_whitespace() || first == '/' || first == '>') {
            offset += pos + open_marker.len();
            continue;
        }
        let Some(end) = find_tag_end(after_tag) else {
            return Err(unterminated());
        };
        let tag = &after_tag[..end];
        let elem_start = offset + pos;
        if tag.trim_end().ends_with('/') {
            // Self-closing: the element is just the tag.
            out.push(&body[elem_start..elem_start + open_marker.len() + end + 1]);
            offset = elem_start + open_marker.len() + end + 1;
            continue;
        }
        let Some(close) = after_tag[end + 1..].find(&close_marker) else {
            return Err(unterminated());
        };
        let full_end = end + 1 + close + close_marker.len();
        out.push(&body[elem_start..elem_start + open_marker.len() + full_end]);
        offset = elem_start + open_marker.len() + full_end;
    }
    Ok(out)
}

/// The index just past the element starting at `start` (the `<` of
/// its opening tag), honouring nesting of the same element name.
/// None when the element is unterminated. The opening tag must not be
/// self-closing: the caller checks that first.
fn xml_element_end(inner: &str, start: usize, name: &str) -> Option<usize> {
    let after_lt = &inner[start + 1..];
    let gt = find_tag_end(after_lt)?;
    let mut depth = 1;
    let mut scan = start + 1 + gt + 1;
    while scan < inner.len() {
        let lt = inner[scan..].find('<')?;
        let pos = scan + lt;
        let after = &inner[pos + 1..];
        if let Some(after_slash) = after.strip_prefix('/') {
            // Closing tag: which element does it close?
            let cname_end = after_slash
                .find(|c: char| c.is_whitespace() || c == '>')
                .unwrap_or(after_slash.len());
            if &after_slash[..cname_end] == name {
                depth -= 1;
                if depth == 0 {
                    let cgt = after.find('>')?;
                    return Some(pos + 1 + cgt + 1);
                }
            }
            scan = pos + 1;
            continue;
        }
        if let Some(content) = after.strip_prefix("!--") {
            // Comment (`after` is past the `<`, so the marker is
            // `!--`): skip to `-->`, not to the first `>`: a `>`
            // inside a comment does not end it. The search starts
            // past the opener: a `-->` overlapping it (`<!-->`,
            // `<!--->`) is not a terminator, it is an unterminated
            // comment.
            let rel = content.find("-->")?;
            scan = pos + 1 + 3 + rel + 3;
            continue;
        }
        if after.starts_with('?') {
            // Processing instruction: skip to `?>`.
            let end = after.find("?>")?;
            scan = pos + 1 + end + 2;
            continue;
        }
        if let Some(cdata) = after.strip_prefix("![CDATA[") {
            // CDATA section: opaque character data, skipped to
            // `]]>` — a `>` or `<!--` inside it is content, not
            // markup.
            let rel = cdata.find("]]>")?;
            scan = pos + 1 + "![CDATA[".len() + rel + 3;
            continue;
        }
        if after.starts_with('!') {
            // Other markup (e.g. a doctype): skip to `>`.
            let end = after.find('>')?;
            scan = pos + 1 + end + 1;
            continue;
        }
        // Opening tag: the same name, not self-closing, nests.
        let oname_end = after
            .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .unwrap_or(after.len());
        if &after[..oname_end] == name {
            let ogt = find_tag_end(after)?;
            if has_bare_lt(&after[..ogt]) {
                // An unterminated nested tag of the same name is
                // bare text, like any other `<` whose span holds
                // a `<` outside quotes: step past it instead of
                // swallowing the element's own close tag.
                scan = pos + 1;
            } else {
                if !after[..ogt].trim_end().ends_with('/') {
                    depth += 1;
                }
                scan = pos + 1 + ogt + 1;
            }
        } else if after.chars().next().is_some_and(is_name_start_char) {
            // A nested opening tag of another name: skip the whole
            // tag, not just past its `<`. Its attributes may hold
            // `<` or `<!--` (e.g. `<b title="<!--">`), which
            // re-scanning would mistake for a comment opener. The
            // tag's content is still scanned for the matching close.
            // But only when the span holds no `<` outside quotes:
            // a `<` there means the `>` that ended the span belongs
            // to a later construct (e.g. `<b</a>`), and skipping
            // whole would swallow the element's own close tag.
            let ogt = find_tag_end(after)?;
            if has_bare_lt(&after[..ogt]) {
                scan = pos + 1;
            } else {
                scan = pos + 1 + ogt + 1;
            }
        } else {
            // Bare `<` in element text, not a tag: step past it,
            // as before — jumping past the next `>` would swallow
            // the element's own close tag.
            scan = pos + 1;
        }
    }
    None
}

/// The direct child elements named `name` in `inner` (the content
/// between an element's open and close tags), as their full text
/// including the tags. Unlike [`xml_elements`], this does not descend:
/// a `<package>` inside a `<cycle>` is a grandchild, not a child, and
/// is skipped. A malformed remainder — an unterminated child,
/// comment, processing instruction, or markup, a missing tag end, an
/// empty tag name, a missing element end, or a stray closing tag at
/// this level — is a server error, never a partial list: like
/// [`xml_elements`], the caller treats the missing content as malformed.
pub(crate) fn xml_child_elements<'a>(
    inner: &'a str,
    name: &str,
) -> Result<Vec<&'a str>, GeectlError> {
    let unterminated = || {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected OBS response: unterminated <{name}> element"),
        )
    };
    let stray_close = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS response: stray closing tag in element content",
        )
    };
    let mut out = Vec::new();
    let mut offset = 0;
    while offset < inner.len() {
        let rest = &inner[offset..];
        let Some(lt) = rest.find('<') else {
            break;
        };
        let tag_start = offset + lt;
        let after_lt = &rest[lt + 1..];
        if after_lt.starts_with('/') {
            // A closing tag at this level is stray: the parent's close
            // was stripped by the caller, so nothing may close here.
            return Err(stray_close());
        }
        if let Some(content) = after_lt.strip_prefix("!--") {
            // Comment (`after_lt` is past the `<`, so the marker
            // is `!--`): skip to `-->`, not to the first `>`. The
            // search starts past the opener: a `-->` overlapping
            // it (`<!-->`, `<!--->`) is not a terminator, it is an
            // unterminated comment.
            let Some(rel) = content.find("-->") else {
                return Err(unterminated());
            };
            let end = 3 + rel;
            offset = tag_start + 1 + end + 3;
            continue;
        }
        if after_lt.starts_with('?') {
            // Processing instruction: skip to `?>`.
            let Some(end) = after_lt.find("?>") else {
                return Err(unterminated());
            };
            offset = tag_start + 1 + end + 2;
            continue;
        }
        if let Some(cdata) = after_lt.strip_prefix("![CDATA[") {
            // CDATA section: opaque character data, skipped to
            // `]]>` — a `>` or `<!--` inside it is content, not
            // markup.
            let Some(rel) = cdata.find("]]>") else {
                return Err(unterminated());
            };
            offset = tag_start + 1 + "![CDATA[".len() + rel + 3;
            continue;
        }
        if after_lt.starts_with('!') {
            // Other markup (e.g. a doctype): skip to `>`.
            let Some(end) = after_lt.find('>') else {
                return Err(unterminated());
            };
            offset = tag_start + 1 + end + 1;
            continue;
        }
        let name_end = after_lt
            .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .unwrap_or(after_lt.len());
        let tag_name = &after_lt[..name_end];
        let Some(gt) = find_tag_end(after_lt) else {
            return Err(unterminated());
        };
        if tag_name.is_empty() {
            // An empty tag (`<></>`) has no name to match or to close
            // against: it is malformed content, not a child to skip.
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS response: empty tag name in element content",
            ));
        }
        let tag_end = tag_start + 1 + gt + 1;
        if after_lt[..gt].trim_end().ends_with('/') {
            // Self-closing: a direct child when the name matches.
            if tag_name == name {
                out.push(&inner[tag_start..tag_end]);
            }
            offset = tag_end;
            continue;
        }
        let Some(elem_end) = xml_element_end(inner, tag_start, tag_name) else {
            return Err(unterminated());
        };
        if tag_name == name {
            out.push(&inner[tag_start..elem_end]);
        }
        offset = elem_end;
    }
    Ok(out)
}

/// The character data directly inside `inner` (the content between
/// an element's open and close tags): the text between child
/// elements, plus the content of CDATA sections there. Comments and
/// processing instructions are skipped, not text. Text inside a
/// child element belongs to that element, not to this level. None
/// when a child element, comment, processing instruction, or CDATA
/// section never closes, or a closing tag appears at this level:
/// the callers that need a well-formed document fail it themselves
/// before collecting.
pub(crate) fn xml_top_level_text(inner: &str) -> Option<String> {
    let mut out = String::new();
    let mut offset = 0;
    while offset < inner.len() {
        let rest = &inner[offset..];
        let Some(lt) = rest.find('<') else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..lt]);
        let tag_start = offset + lt;
        let after_lt = &rest[lt + 1..];
        if let Some(content) = after_lt.strip_prefix("!--") {
            // Comment: skip to `-->`, not to the first `>`. The
            // search starts past the opener: a `-->` overlapping
            // it (`<!-->`, `<!--->`) is not a terminator.
            let rel = content.find("-->")?;
            let end = 3 + rel;
            offset = tag_start + 1 + end + 3;
            continue;
        }
        if after_lt.starts_with('?') {
            // Processing instruction: skip to `?>`.
            let end = after_lt.find("?>")?;
            offset = tag_start + 1 + end + 2;
            continue;
        }
        if after_lt.starts_with("![CDATA[") {
            // A CDATA section is character data at this level: its
            // content counts as text, unlike a comment.
            let end = after_lt.find("]]>")?;
            out.push_str(&after_lt["![CDATA[".len()..end]);
            offset = tag_start + 1 + end + 3;
            continue;
        }
        if after_lt.starts_with('/') {
            // A closing tag at this level is stray: the parent's
            // close was stripped by the caller, so nothing may close
            // here.
            return None;
        }
        if after_lt.starts_with('!') {
            // Other markup (e.g. a doctype): skip to `>`.
            let end = after_lt.find('>')?;
            offset = tag_start + 1 + end + 1;
            continue;
        }
        let name_end = after_lt
            .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .unwrap_or(after_lt.len());
        let tag_name = &after_lt[..name_end];
        if tag_name.is_empty() {
            return None;
        }
        let gt = find_tag_end(after_lt)?;
        let tag_end = tag_start + 1 + gt + 1;
        if after_lt[..gt].trim_end().ends_with('/') {
            // Self-closing: no content to skip.
            offset = tag_end;
            continue;
        }
        let elem_end = xml_element_end(inner, tag_start, tag_name)?;
        offset = elem_end;
    }
    Some(out)
}

/// The bodies of the `<!-- ... -->` comments that are direct
/// children of the scanned text, in file order: the text between
/// `<!--` and `-->`, verbatim. A comment inside a child element
/// belongs to that element, not to the enclosing one, so the walk
/// skips whole child elements (open tag to matching close,
/// self-closing tags included) and collects only the comments
/// between them. A comment that never terminates, or an element
/// that never closes, ends the scan: the callers that need a
/// well-formed document fail it themselves before collecting.
pub(crate) fn xml_direct_comments(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut offset = 0;
    while offset < text.len() {
        let rest = &text[offset..];
        let Some(lt) = rest.find('<') else {
            break;
        };
        let tag_start = offset + lt;
        let after_lt = &rest[lt + 1..];
        if let Some(content) = after_lt.strip_prefix("!--") {
            // Direct-child comment: collect it, skip to `-->`.
            // The search starts past the opener: a `-->` overlapping
            // it (`<!-->`, `<!--->`) is not a terminator, it is an
            // unterminated comment, and the scan ends here.
            let Some(rel) = content.find("-->") else {
                break;
            };
            let end = 3 + rel;
            out.push(&after_lt["!--".len()..end]);
            offset = tag_start + 1 + end + 3;
            continue;
        }
        if after_lt.starts_with('/') {
            // A closing tag at this level is stray: the caller
            // fails it before collecting, so the scan ends here.
            break;
        }
        if after_lt.starts_with('?') {
            let Some(end) = after_lt.find("?>") else {
                break;
            };
            offset = tag_start + 1 + end + 2;
            continue;
        }
        if let Some(cdata) = after_lt.strip_prefix("![CDATA[") {
            // CDATA section: opaque character data, skipped to
            // `]]>` — a `>` or `<!--` inside it is content, not
            // markup, and never a collected comment.
            let Some(rel) = cdata.find("]]>") else {
                break;
            };
            offset = tag_start + 1 + "![CDATA[".len() + rel + 3;
            continue;
        }
        if after_lt.starts_with('!') {
            let Some(end) = after_lt.find('>') else {
                break;
            };
            offset = tag_start + 1 + end + 1;
            continue;
        }
        let name_end = after_lt
            .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .unwrap_or(after_lt.len());
        let tag_name = &after_lt[..name_end];
        if tag_name.is_empty() {
            break;
        }
        let Some(gt) = find_tag_end(after_lt) else {
            break;
        };
        let tag_end = tag_start + 1 + gt + 1;
        if after_lt[..gt].trim_end().ends_with('/') {
            // Self-closing: no content to skip.
            offset = tag_end;
            continue;
        }
        let Some(elem_end) = xml_element_end(text, tag_start, tag_name) else {
            break;
        };
        offset = elem_end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obs::build::xml_attr;
    use crate::obs::build::xml_split_element;

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
    fn parse_keeps_a_cdata_section_as_an_opaque_child() {
        let doc = XmlDoc::parse("<project><![CDATA[junk]]><title>T</title></project>").unwrap();
        assert_eq!(
            doc.render(),
            "<project><![CDATA[junk]]><title>T</title></project>"
        );
        assert_eq!(doc.child_names(), vec!["title"]);
        assert!(doc.has_content());
        assert!(XmlDoc::parse("<project><![CDATA[never closed</project>").is_err());
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
    fn child_names_lists_direct_child_elements() {
        let doc = XmlDoc::parse(META).unwrap();
        assert_eq!(
            doc.child_names(),
            vec!["title", "description", "devel", "person", "lock"]
        );
    }

    #[test]
    fn child_names_skips_comments_processing_instructions_and_text() {
        let doc = XmlDoc::parse(
            "<project>text<!-- a comment --><?pi data?><title>T</title><!--x--><description>D</description></project>",
        )
        .unwrap();
        assert_eq!(doc.child_names(), vec!["title", "description"]);
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
    fn has_element_with_attr_matches_name_and_value() {
        let doc = XmlDoc::parse(META).unwrap();
        assert!(doc.has_element_with_attr("person", "userid", "alice"));
        assert!(doc.has_element_with_attr("person", "role", "maintainer"));
        assert!(!doc.has_element_with_attr("person", "userid", "bob"));
        assert!(!doc.has_element_with_attr("person", "missing", "alice"));
        assert!(!doc.has_element_with_attr("devel", "userid", "alice"));
        // A `name=` inside an earlier attribute's value must not match.
        let doc = XmlDoc::parse(
            "<project><repository description=\"name=x\" name=\"standard\"/></project>",
        )
        .unwrap();
        assert!(doc.has_element_with_attr("repository", "name", "standard"));
        assert!(!doc.has_element_with_attr("repository", "name", "x"));
    }

    #[test]
    fn remove_elements_with_attr_drops_only_the_matching_ones() {
        let mut doc = XmlDoc::parse(
            "<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch></repository><repository name=\"ports\"><arch>aarch64</arch></repository></project>",
        )
        .unwrap();
        assert!(doc.remove_elements_with_attr("repository", "name", "ports"));
        assert_eq!(
            doc.render(),
            "<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch></repository></project>"
        );
        // Removing an absent name reports false and changes nothing.
        assert!(!doc.remove_elements_with_attr("repository", "name", "ports"));
        assert!(!doc.remove_elements_with_attr("repository", "name", "missing"));
        assert!(doc.remove_elements_with_attr("repository", "name", "standard"));
        assert_eq!(doc.render(), "<project name=\"p\"></project>");
    }

    #[test]
    fn append_element_adds_a_child_and_uncloses_a_self_closing_root() {
        let mut doc = XmlDoc::parse("<project name=\"p\"/>").unwrap();
        assert!(doc.self_closing);
        doc.append_element("<repository name=\"standard\"><arch>x86_64</arch></repository>")
            .unwrap();
        assert!(!doc.self_closing);
        assert_eq!(
            doc.render(),
            "<project name=\"p\"><repository name=\"standard\"><arch>x86_64</arch></repository></project>"
        );
        // Appending again adds a second element, it does not replace.
        doc.append_element("<repository name=\"ports\"><arch>aarch64</arch></repository>")
            .unwrap();
        assert_eq!(doc.render().matches("<repository").count(), 2);
        // A fragment that is not a single element is rejected.
        assert!(doc.append_element("not xml").is_err());
        assert!(doc.append_element("<a/><b/>").is_err());
        assert!(doc.append_element("<repository>").is_err());
        // A prolog-prefixed fragment is rejected too: it would embed
        // the prolog mid-document.
        assert!(doc
            .append_element("<?xml version=\"1.0\"?><repository name=\"x\"/>")
            .is_err());
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
    fn replace_or_insert_element_preserves_position() {
        // Replacing a middle child keeps it in place: the old
        // remove-then-push moved it to the end, reordering the
        // document and causing a spurious extra PUT on the next
        // no-change comparison.
        let mut doc = XmlDoc::parse(META).unwrap();
        doc.replace_or_insert_element("devel", "<devel project=\"test:other\"/>")
            .unwrap();
        assert_eq!(
            doc.render(),
            "<package name=\"test-pkgA\" project=\"test:factory\"><title>Test</title><description>A test package</description><devel project=\"test:other\"/><person userid=\"alice\" role=\"maintainer\"/><lock><enable/></lock></package>"
        );
    }

    #[test]
    fn replace_or_insert_element_rejects_a_wrong_element() {
        let mut doc = XmlDoc::parse(META).unwrap();
        assert!(doc
            .replace_or_insert_element("build", "<publish><disable/></publish>")
            .is_err());
        assert!(doc.replace_or_insert_element("build", "not xml").is_err());
        assert!(doc.replace_or_insert_element("build", "<build>").is_err());
        // A prolog-prefixed fragment is rejected too: it would embed
        // the prolog mid-document.
        assert!(doc
            .replace_or_insert_element("build", "<?xml version=\"1.0\"?><build><disable/></build>")
            .is_err());
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
    fn xml_attr_blank_reads_unicode_whitespace_only_values_as_blank() {
        // Space, tab, and NBSP are all attribute whitespace: a
        // value of only those carries nothing and reads as absent.
        assert!(xml_attr_blank(" "));
        assert!(xml_attr_blank("  \t "));
        assert!(xml_attr_blank("\u{a0}"));
        // A value with real content keeps its surrounding spaces:
        // only an all-whitespace value is blank, nothing is trimmed.
        assert!(!xml_attr_blank(" foo "));
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
    fn xml_unescape_decodes_numeric_references() {
        // The single shared implementation decodes numeric character
        // references too: the old xml.rs copy left them verbatim.
        assert_eq!(xml_unescape("&#65;"), "A");
        assert_eq!(xml_unescape("&#x41;"), "A");
        assert_eq!(xml_unescape("&#X41;"), "A");
        assert_eq!(xml_unescape("&#65;&#x42;"), "AB");
        // Unknown entities and a bare `&` still pass through verbatim.
        assert_eq!(xml_unescape("&nbsp;"), "&nbsp;");
        assert_eq!(xml_unescape("a & b"), "a & b");
    }

    #[test]
    fn find_attr_decodes_numeric_references() {
        // An attribute read through XmlDoc decodes the same way as the
        // parse_* functions: a numeric reference is a character, not
        // literal text.
        assert_eq!(
            find_attr("name=\"&#65;pple\"", "name").as_deref(),
            Some("Apple")
        );
    }

    #[test]
    fn find_attr_ignores_a_name_inside_an_earlier_quoted_value() {
        // `project=` inside the details value must not match: the
        // real `project` attribute follows it.
        assert_eq!(
            find_attr("details=\"see project=x\" project=\"foo\"", "project").as_deref(),
            Some("foo")
        );
        // Whitespace around `=` is the same attribute.
        assert_eq!(
            find_attr("project = \"foo\"", "project").as_deref(),
            Some("foo")
        );
        assert_eq!(
            find_attr("project= \"foo\"", "project").as_deref(),
            Some("foo")
        );
        // A malformed first occurrence does not stop the scan: the
        // well-formed later one wins.
        assert_eq!(
            find_attr("project=bad project=\"foo\"", "project").as_deref(),
            Some("foo")
        );
        // Either quote style works.
        assert_eq!(
            find_attr("project='foo'", "project").as_deref(),
            Some("foo")
        );
    }

    #[test]
    fn nested_same_name_elements_parse_whole() {
        // A <build> containing a nested <build> is pathological, but
        // the depth count must keep the outer element whole.
        let xml = "<package name=\"p\"><build><build><disable/></build></build></package>";
        let doc = XmlDoc::parse(xml).unwrap();
        assert_eq!(doc.render(), xml);
    }

    #[test]
    fn xml_unescape_reverses_escape() {
        assert_eq!(xml_unescape("plain"), "plain");
        assert_eq!(xml_unescape("a &amp; b"), "a & b");
        assert_eq!(xml_unescape("&lt;tag&gt;"), "<tag>");
        assert_eq!(xml_unescape("&quot;q&quot;"), "\"q\"");
        assert_eq!(xml_unescape("&apos;a&apos;"), "'a'");
        assert_eq!(xml_unescape("&#65;"), "A");
        assert_eq!(xml_unescape("&#x41;"), "A");
        // Unknown entities and a bare `&` pass through verbatim.
        assert_eq!(xml_unescape("&nbsp;"), "&nbsp;");
        assert_eq!(xml_unescape("a & b"), "a & b");
        assert_eq!(xml_unescape("a &"), "a &");
    }

    #[test]
    fn xml_escape_escapes_the_five_xml_specials() {
        // `tests/test__private_api.py::TestXmlEscape` pins the same
        // five escapes (`<`, `>`, `'`, `"`, `&`).
        assert_eq!(xml_escape("<"), "&lt;");
        assert_eq!(xml_escape(">"), "&gt;");
        assert_eq!(xml_escape("'"), "&apos;");
        assert_eq!(xml_escape("\""), "&quot;");
        assert_eq!(xml_escape("&"), "&amp;");
        assert_eq!(xml_escape("a<b>&\"c\""), "a&lt;b&gt;&amp;&quot;c&quot;");
    }

    #[test]
    fn xml_unescape_decodes_named_and_numeric_entities() {
        assert_eq!(
            xml_unescape("a &amp; b &lt; c &gt; d &quot;e&quot; &apos;f&apos;"),
            "a & b < c > d \"e\" 'f'"
        );
        assert_eq!(xml_unescape("&#65;&#x42;"), "AB");
        assert_eq!(xml_unescape("&amp;lt;"), "&lt;");
        assert_eq!(xml_unescape("plain text"), "plain text");
        assert_eq!(xml_unescape("&bogus; stays"), "&bogus; stays");
    }

    #[test]
    fn xml_unescape_decodes_entities_and_leaves_unknown_ones() {
        assert_eq!(xml_unescape("a &amp; b"), "a & b");
        assert_eq!(xml_unescape("&lt;tag&gt; &quot;x&quot;"), "<tag> \"x\"");
        assert_eq!(xml_unescape("&#65;&#x42;"), "AB");
        assert_eq!(xml_unescape("a &nosuch; b"), "a &nosuch; b");
        assert_eq!(xml_unescape("a &amp;amp; b"), "a &amp; b");
        assert_eq!(xml_unescape("plain"), "plain");
        assert_eq!(xml_unescape("a & b"), "a & b");
    }

    #[test]
    fn xml_elements_ignores_a_gt_inside_a_quoted_value() {
        let body = "<resultlist><result project=\"a\" code=\"broken\" details=\"a > b\" dirty=\"true\"/><result project=\"b\" code=\"published\"/></resultlist>";
        let found = xml_elements(body, "result").unwrap();
        assert_eq!(found.len(), 2);
        let (tag, _) = xml_split_element(found[0], "result");
        assert_eq!(xml_attr(tag, "dirty").as_deref(), Some("true"));
    }

    #[test]
    fn xml_unescape_decodes_uppercase_hex_references() {
        assert_eq!(xml_unescape("&#X41;&#x42;"), "AB");
    }

    #[test]
    fn xml_elements_finds_full_elements_and_ignores_prefix_names() {
        let body = "<resultlist><result project=\"a\"/><resultlistx/><result project=\"b\">x</result></resultlist>";
        let found = xml_elements(body, "result").unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0], "<result project=\"a\"/>");
        assert_eq!(found[1], "<result project=\"b\">x</result>");
        assert!(xml_elements(body, "status").unwrap().is_empty());
    }

    #[test]
    fn xml_elements_rejects_an_unterminated_element() {
        // An element with no closing tag is a server error, not a
        // shorter list: a truncated document must never read as a
        // smaller matrix.
        // A complete element followed by an unterminated one is an
        // error, not a one-element list.
        assert!(xml_elements("<result project=\"a\"/><result project=\"b\">x", "result").is_err());
        assert!(xml_elements("<resultlist><result project=\"a\"", "result").is_err());
        assert!(xml_elements("<resultlist><result project=\"a\">x", "result").is_err());
    }

    #[test]
    fn xml_child_elements_reads_direct_children_only() {
        let inner = "<package name=\"a\"><pkgdep>x</pkgdep></package><cycle><package name=\"b\"/></cycle><package name=\"c\"/>";
        let found = xml_child_elements(inner, "package").unwrap();
        assert_eq!(found.len(), 2);
        assert!(found[0].contains("name=\"a\""));
        assert!(found[1].contains("name=\"c\""));
    }

    #[test]
    fn xml_child_elements_skips_nested_same_name_elements() {
        let inner = "<package name=\"a\"><package name=\"inner\"/></package><package name=\"b\"/>";
        let found = xml_child_elements(inner, "package").unwrap();
        assert_eq!(found.len(), 2);
        assert!(found[0].contains("name=\"a\""));
        assert!(found[0].contains("name=\"inner\""));
    }

    #[test]
    fn xml_child_elements_rejects_malformed_remainders() {
        // Every former fail-open break site is an error now: an
        // unterminated child, comment, processing instruction, or
        // markup, a missing tag end, an empty tag name, and a stray
        // closing tag at this level.
        assert!(xml_child_elements("<package name=\"a\"><pkgdep>x</pkgdep>", "package").is_err());
        assert!(xml_child_elements("<!-- never closed", "package").is_err());
        assert!(xml_child_elements("<?pi never closed", "package").is_err());
        assert!(xml_child_elements("<!DOCTYPE never closed", "package").is_err());
        assert!(xml_child_elements("<package name=\"a\"", "package").is_err());
        assert!(xml_child_elements("</package>", "package").is_err());
        assert!(xml_child_elements("<></>", "package").is_err());
        assert!(xml_child_elements("<package name=\"a\"/><></>", "package").is_err());
    }

    #[test]
    fn xml_child_elements_skips_cdata_sections() {
        // A CDATA section is opaque character data: a `>` or a
        // `<!--` inside it neither ends a tag nor opens a comment,
        // and its content is never a collected comment.
        let inner = "<![CDATA[ a > <!-- ]]><package name=\"p\"/>";
        let found = xml_child_elements(inner, "package").unwrap();
        assert_eq!(found.len(), 1);
        let inner = "<a><![CDATA[ a > <!-- ]]></a><package name=\"p\"/>";
        let found = xml_child_elements(inner, "package").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            xml_direct_comments("<![CDATA[ a > <!-- ]]><!-- real -->"),
            vec![" real "]
        );
        // An unterminated CDATA section fails the file, like an
        // unterminated comment.
        assert!(xml_child_elements("<a><![CDATA[ never closed</a>", "package").is_err());
    }

    #[test]
    fn xml_element_end_skips_a_nested_tag_whole() {
        // A `<!--` inside a nested tag's quoted attribute value
        // is not a comment opener: the tag is skipped whole and
        // its close still ends the element.
        let inner = "<a><b title=\"<!--\">x</b></a>";
        assert_eq!(xml_element_end(inner, 0, "a"), Some(inner.len()));
        let inner = "<a><b title=\"<!--\">x</b></a><package name=\"p\"/>";
        let found = xml_child_elements(inner, "package").unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("name=\"p\""));
    }

    #[test]
    fn xml_element_end_bare_lt_in_text_still_parses() {
        // A bare `<` in element text is not a tag: the element
        // still ends at its own close tag. That holds for `<`
        // followed by a digit, `-`, or `.` too: those can appear
        // in a name but never start one.
        let inner = "<title>1 < 2</title><package name=\"p\"/>";
        let found = xml_child_elements(inner, "package").unwrap();
        assert_eq!(found.len(), 1);
        let inner = "<a>text <</a><package name=\"p\"/>";
        let found = xml_child_elements(inner, "package").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(xml_element_end("<a>1 <2</a>", 0, "a"), Some(11));
        let inner = "<title>needs <2GB RAM</title><package name=\"p\"/>";
        let found = xml_child_elements(inner, "package").unwrap();
        assert_eq!(found.len(), 1);
        let inner = "<a>a <- b</a><package name=\"p\"/>";
        let found = xml_child_elements(inner, "package").unwrap();
        assert_eq!(found.len(), 1);
        let inner = "<a>a <. b</a><package name=\"p\"/>";
        let found = xml_child_elements(inner, "package").unwrap();
        assert_eq!(found.len(), 1);
        // An unterminated nested tag is bare text too: the `<b`
        // in `<b</a>` has no `>` of its own, so the element still
        // ends at its own close tag.
        assert_eq!(xml_element_end("<a>text <b</a>", 0, "a"), Some(14));
        assert_eq!(xml_element_end("<a>a <b c</a>", 0, "a"), Some(13));
        // The same-name branch has the same guard.
        assert_eq!(xml_element_end("<a>a <a b</a>", 0, "a"), Some(13));
        assert_eq!(xml_element_end("<a>text <a</a>", 0, "a"), Some(14));
    }

    #[test]
    fn xml_child_elements_skips_comments_carrying_markup() {
        // A comment is skipped to `-->`, not to the first `>`: a
        // comment carrying an element — including a closing tag for
        // the element being scanned — is opaque text, never a child
        // and never a close.
        let inner = "<package name=\"a\"><!-- <package name=\"x\">y</package> --><pkgdep>z</pkgdep></package><package name=\"b\"/>";
        let found = xml_child_elements(inner, "package").unwrap();
        assert_eq!(found.len(), 2);
        assert!(found[0].contains("name=\"a\""));
        assert!(found[0].contains("pkgdep"));
        assert!(found[1].contains("name=\"b\""));
        // A comment carrying only a `>` is skipped whole as well.
        let inner = "<!-- a > b --><package name=\"a\"/>";
        let found = xml_child_elements(inner, "package").unwrap();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn xml_top_level_text_degenerate_openers_are_unterminated() {
        // A `-->` overlapping the comment opener is not a
        // terminator: the text is unterminated, like in the other
        // scanners.
        assert!(xml_top_level_text("<!-->").is_none());
        assert!(xml_top_level_text("<!---><arch>x</arch>").is_none());
    }

    #[test]
    fn xml_top_level_text_reads_text_between_elements_and_cdata() {
        assert_eq!(
            xml_top_level_text("junk<arch>x86_64</arch>more"),
            Some("junkmore".to_string())
        );
        assert_eq!(
            xml_top_level_text("<arch>x86_64</arch>"),
            Some(String::new())
        );
        assert_eq!(
            xml_top_level_text("<![CDATA[junk]]><arch>x86_64</arch>"),
            Some("junk".to_string())
        );
        assert_eq!(
            xml_top_level_text("  \n  <arch>x86_64</arch>\n  "),
            Some("  \n  \n  ".to_string())
        );
        // Comments and processing instructions are skipped, not
        // text; text inside a child belongs to the child.
        assert_eq!(
            xml_top_level_text("<!-- a note --><arch>x86_64</arch><?pi data?>"),
            Some(String::new())
        );
        // A nested element of the same name is skipped whole.
        assert_eq!(
            xml_top_level_text("<a><a>inner</a>tail</a>after"),
            Some("after".to_string())
        );
        // Malformed content is None, never a partial read.
        assert_eq!(xml_top_level_text("</arch>"), None);
        assert_eq!(xml_top_level_text("<!-- never closed"), None);
        assert_eq!(xml_top_level_text("<![CDATA[never closed"), None);
        assert_eq!(xml_top_level_text("<arch>x86_64"), None);
        assert_eq!(xml_top_level_text("<>"), None);
    }

    #[test]
    fn xml_direct_comments_collects_only_direct_children() {
        // Direct comments in file order, with `>` inside one.
        assert_eq!(
            xml_direct_comments("a<!-- one --><b/><!-- two -->c"),
            vec![" one ", " two "]
        );
        assert!(xml_direct_comments("no comments here").is_empty());
        // A comment inside a child element belongs to that
        // element, not to the enclosing text.
        assert_eq!(
            xml_direct_comments("<param>x<!-- inner --></param><!-- outer -->"),
            vec![" outer "]
        );
        assert!(xml_direct_comments("<extra><!-- deep --></extra>").is_empty());
        // A `<!--` inside a quoted attribute value is not a
        // comment opener: the element is skipped whole, and the
        // real comment after it is still collected.
        assert_eq!(
            xml_direct_comments("<param name=\"<!--\">x</param><!-- real -->"),
            vec![" real "]
        );
        // An unterminated comment ends the scan: the tail is not a
        // body, and the complete comment before it still is.
        assert_eq!(
            xml_direct_comments("<!-- done --><!-- never closed"),
            vec![" done "]
        );
    }

    #[test]
    fn xml_direct_comments_degenerate_openers_end_the_scan() {
        // `<!-->` and `<!--->` are not terminated comments: the
        // `-->` in them overlaps the opener, so the scan ends and
        // nothing after them is collected. An empty comment
        // (`<!---->`) still collects as an empty body.
        assert!(xml_direct_comments("<!-->").is_empty());
        assert!(xml_direct_comments("<!--->").is_empty());
        assert_eq!(xml_direct_comments("<!-- done --><!-->"), vec![" done "]);
        assert_eq!(xml_direct_comments("<!-- done --><!--->"), vec![" done "]);
        assert_eq!(xml_direct_comments("<!---->"), vec![""]);
    }

    #[test]
    fn xml_child_elements_degenerate_comments_are_unterminated() {
        // A degenerate comment opener fails the file, like any
        // unterminated comment: it must not swallow the child
        // that follows it.
        assert!(xml_child_elements("<!--><package name=\"a\"/>", "package").is_err());
        assert!(xml_child_elements("<!---><package name=\"a\"/>", "package").is_err());
    }

    #[test]
    fn skip_xml_prolog_skips_a_prolog_and_nothing_else() {
        assert_eq!(skip_xml_prolog("<a/>").unwrap(), "<a/>");
        assert_eq!(
            skip_xml_prolog("<?xml version=\"1.0\" encoding=\"UTF-8\"?><a/>").unwrap(),
            "<a/>"
        );
        // Bare prolog, and whitespace/newline between prolog and element.
        assert_eq!(skip_xml_prolog("<?xml?><a/>").unwrap(), "<a/>");
        assert_eq!(
            skip_xml_prolog("<?xml version=\"1.0\"?>\n<a/>").unwrap(),
            "<a/>"
        );
        // A prolog that never terminates is an error, not a missing document.
        assert!(skip_xml_prolog("<?xml version=\"1.0\" <a/>").is_err());
        assert!(skip_xml_prolog("<?xml version=\"1.0\"").is_err());
        // `<?xmlfoo` is not a prolog at all.
        assert!(skip_xml_prolog("<?xmlfoo version=\"1.0\"?><a/>").is_err());
        // Leading whitespace is the caller's job: the input is expected trimmed.
        assert_eq!(skip_xml_prolog("  <a/>").unwrap(), "  <a/>");
    }
}
