use super::build::xml_attr;
use super::ActionShape;
use super::HistoryEntry;
use super::NamedRole;
use super::RevisionRange;
use super::RevisionSpec;
use crate::error::ErrorKind;
use crate::error::GeectlError;
use crate::http::is_loopback_url;
use crate::url::encode_path_segment;
use crate::url::encode_query_value;
use crate::url::UrlBuilder;
use crate::xml::find_tag_end;
use crate::xml::xml_unescape;

/// The OBS API base (production).
pub const OBS_API_BASE: &str = "https://api.opensuse.org";

/// The Factory project: the devel lookup starts here.
pub const FACTORY_PROJECT: &str = "openSUSE:Factory";

/// True when `name` is a valid OBS package or project name
/// (`[A-Za-z0-9_.+-:]`). Anything else is rejected rather than quoted
/// into a URL or an XPath literal, where a `/`, `?`, or quote would
/// rewrite the request.
pub fn valid_obs_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+' | ':'))
}

/// True when `name` is an OBS attribute name in `NAMESPACE:NAME`
/// shape: exactly one colon, both sides non-empty. The server's
/// shape check is only "exactly two parts when split on the colon"
/// (the branch command's update project attribute, refused with an
/// ArgumentError otherwise), so an empty namespace passes there and
/// geectl rejects it deliberately; the maintained attribute is only
/// looked up by name (`AttribType.find_by_name!`, `RecordNotFound`
/// when absent), so the same shape check there is client-side
/// strictness, not a server rule.
pub fn valid_attribute_name(name: &str) -> bool {
    let Some((namespace, attr)) = name.split_once(':') else {
        return false;
    };
    !namespace.is_empty() && !attr.is_empty() && !attr.contains(':')
}

/// True when `name` is a valid OBS source filename for upload.
/// Mirrors the server's rule (`PackageService::FileVerifier#file_name_invalid?`:
/// `^[^./][^/]+$` — first character not `.` or `/`, no `/` anywhere;
/// the `+` needs at least one more character, so a one-character name
/// is invalid and the minimum length is two), plus the names the
/// routes reserve: `_meta` (the package-meta route, not a file) and
/// `_attribute` (rejected by `Package.verify_file!`), and the build
/// routes' leaves (`_log`, `_status`, `_buildinfo`, `_history`,
/// `_jobstatus`, `_jobhistory`) — under `/build/.../<package>/` those
/// address the log, status, buildinfo, history, job-status, and
/// job-history routes, not a
/// file, so a download by one of those names would return route
/// output as if it were a file. Anything else is rejected rather
/// than quoted into a URL, where a `/` would rewrite the request.
pub fn valid_obs_filename(name: &str) -> bool {
    name.chars().count() >= 2
        && !name.starts_with('.')
        && !name.contains('/')
        && name != "_meta"
        && name != "_attribute"
        && name != "_log"
        && name != "_status"
        && name != "_buildinfo"
        && name != "_history"
        && name != "_jobstatus"
        && name != "_jobhistory"
}

/// Split a `project/package` path on `/`, dropping empty segments:
/// a leading, trailing, or doubled slash contributes no segment, so
/// `"//"` and `"/////////"` split to nothing and `"a//b"` to
/// `["a", "b"]`. The segments are returned verbatim (no trimming);
/// callers validate them as OBS names.
pub fn split_slash_segments(s: &str) -> Vec<String> {
    s.split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_string())
        .collect()
}

/// Split a `project/package` path into its two parts. Exactly two
/// non-empty segments, or an error: a bare project, a bare package,
/// or more than one slash is not a project/package pair.
pub fn parse_project_package(s: &str) -> Result<(String, String), GeectlError> {
    let segments = split_slash_segments(s);
    if segments.len() != 2 {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("expected a project/package path, got {s:?}"),
        ));
    }
    Ok((segments[0].clone(), segments[1].clone()))
}

/// Build a request URL from a base, path segments, and query pairs,
/// using the encoders from `url.rs`. Segments are encoded as exactly
/// one element each; query pairs as `name=value`.
pub(crate) fn build_url(base: &str, segments: &[&str], query: &[(&str, &str)]) -> String {
    let mut url = base.trim_end_matches('/').to_string();
    for segment in segments {
        url.push('/');
        url.push_str(&encode_path_segment(segment));
    }
    if !query.is_empty() {
        url.push('?');
        for (i, (name, value)) in query.iter().enumerate() {
            if i > 0 {
                url.push('&');
            }
            url.push_str(&encode_query_value(name));
            url.push('=');
            url.push_str(&encode_query_value(value));
        }
    }
    url
}

/// Validate an OBS API base URL. It must be a bare `https://` URL
/// (scheme plus authority, no path, query, fragment, or userinfo)
/// with a host (the production API), or a bare `http://` URL on a
/// loopback host (a test instance only). Anything else is refused at
/// construction, not at the first request. A trailing slash is
/// stripped, not a path.
pub(crate) fn validate_base(base: &str) -> Result<(), GeectlError> {
    // A base is scheme plus authority only: anything after the
    // authority would be string-concatenated into every OBS URL (see
    // `build_url`), so a base such as `https://api.opensuse.org/evil`
    // would rewrite them all. The live harness enforces the same
    // bare shape for its base URL.
    let base = base.trim_end_matches('/');
    if let Some(authority) = base.strip_prefix("https://") {
        if !is_bare_authority(authority) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "OBS base URL must be a bare https:// URL (scheme + host, no path, query, fragment, or userinfo)",
            ));
        }
        // Reuse the production builder's validation (https + host).
        UrlBuilder::new(base).map(|_| ())
    } else if let Some(authority) = base.strip_prefix("http://") {
        if !is_bare_authority(authority) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "OBS base URL must be a bare http:// URL (scheme + host, no path, query, fragment, or userinfo)",
            ));
        }
        if is_loopback_url(base) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "an http:// OBS base must be a loopback URL (a test instance only)",
            ))
        }
    } else {
        Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "OBS base URL must be https:// (or http:// loopback for a test instance)",
        ))
    }
}

/// True when the authority's `:port`, if present, is a decimal port
/// number (0-65535). A port is never anything else: `api.opensuse.org:abc`
/// names no server, and today it sails through validation only to fail
/// at the first request inside the transport, with a transport error
/// instead of the bare-base message. Neither downstream check looks at
/// the port — `UrlBuilder::new` checks scheme and host only, and
/// `is_loopback_url` classifies the host and ignores the port — so this
/// is the one place that can reject it early. An empty port
/// (`api.opensuse.org:`) stays accepted: it means the default port.
fn port_is_valid(authority: &str) -> bool {
    // The port separator is the `:` after the closing bracket of a
    // bracketed host, or the first `:` of a bare host (a bare host
    // carries no `:` of its own).
    let after_host = if authority.starts_with('[') {
        match authority.find(']') {
            Some(end) => &authority[end + 1..],
            // Unterminated bracket: the bracket check rejects the
            // authority; this check has nothing to say about it.
            None => return true,
        }
    } else {
        match authority.find(':') {
            Some(pos) => &authority[pos..],
            None => return true,
        }
    };
    let Some(port) = after_host.strip_prefix(':') else {
        return true;
    };
    port.is_empty()
        || (port.chars().all(|c| c.is_ascii_digit())
            && port.parse::<u32>().is_ok_and(|p| p <= 65535))
}

/// True when `authority` is a bare authority: non-empty, with no path,
/// query, fragment, userinfo, or whitespace, and with a non-empty host.
/// The authority ends at the first `/`, `?`, or `#`; an `@` inside it
/// is userinfo, which a base must never carry (credentials belong in
/// `Credentials`, not in the URL). An authority starting with `:` or
/// `[]` carries no host at all. A bracketed host is only a bracketed
/// host if the closing bracket is followed by the end of the authority
/// or a `:port`: without that, `[::1]evil.com` is not a host at all.
/// A `:port`, when present, must be a decimal port number (see
/// `port_is_valid`).
pub(crate) fn is_bare_authority(authority: &str) -> bool {
    !authority.is_empty()
        && !authority.contains(['/', '?', '#', '@'])
        && !authority.chars().any(|c| c.is_whitespace())
        && !authority.starts_with(':')
        && !authority.starts_with("[]")
        && (!authority.starts_with('[')
            || match authority.find(']') {
                Some(end) => {
                    let rest = &authority[end + 1..];
                    rest.is_empty() || rest.starts_with(':')
                }
                None => false,
            })
        && port_is_valid(authority)
}

/// Extract the `matches` count from an OBS search response
/// (`<collection matches="N">...</collection>`). None when the
/// response does not carry a count.
pub(crate) fn parse_search_matches(body: &str) -> Option<u32> {
    quoted_attr_value(body, "matches")?.parse::<u32>().ok()
}

/// The value of attribute `name` in `element` (the text between a
/// tag name and its `>`), in either quote style: `name="v"` or
/// `name='v'`. None when the attribute is absent or unterminated.
/// The attribute name must stand on its own — `subproject="..."` must
/// not match `project=` — so the character before `name` has to be
/// whitespace. The scan is quote-aware: a `name=` inside an earlier
/// attribute's value must not match. Whitespace around `=` is
/// allowed: `name = "v"` is the same attribute. A malformed
/// occurrence (an unquoted value, or a value whose closing quote
/// never comes) does not stop the scan: the server never emits one,
/// but a later well-formed occurrence still carries the value.
pub(crate) fn quoted_attr_value(element: &str, name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    let bytes = element.as_bytes();
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
            && element[i..].starts_with(name)
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
                    let after = &element[j + 1..];
                    match after.find(quote as char) {
                        Some(close) => return Some(after[..close].to_string()),
                        // Unterminated: resume just past the opening
                        // quote, so a later well-formed occurrence
                        // still carries the value.
                        None => {
                            i = j + 1;
                            continue;
                        }
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// The numeric `id` attribute of a `<request ...>` opening tag's
/// text (the text between `<request` and `>`), read order-tolerantly:
/// the attribute may sit anywhere in the tag, in either quote style.
/// None when the tag carries no numeric id. The attribute name must
/// stand on its own — `valid="..."` or `grid="..."` must not match —
/// so the character before `id=` has to be whitespace.
fn request_tag_id(element: &str) -> Option<String> {
    let mut rest = element;
    while let Some(pos) = rest.find("id=") {
        if pos > 0 && !rest.as_bytes()[pos - 1].is_ascii_whitespace() {
            rest = &rest[pos + "id=".len()..];
            continue;
        }
        let after_eq = &rest[pos + "id=".len()..];
        // A malformed occurrence (an unquoted value, a value whose
        // closing quote never comes, or a non-numeric value) does not
        // stop the scan: a later well-formed occurrence still carries
        // the id. Same treatment as `quoted_attr_value`.
        let Some(quote) = after_eq.chars().next() else {
            rest = after_eq;
            continue;
        };
        if quote != '"' && quote != '\'' {
            rest = after_eq;
            continue;
        }
        let Some(close) = after_eq[1..].find(quote) else {
            rest = &after_eq[1..];
            continue;
        };
        let id = &after_eq[1..1 + close];
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
            rest = &after_eq[1 + close + 1..];
            continue;
        }
        return Some(id.to_string());
    }
    None
}

/// Extract the request id from an OBS create-request response
/// (`<request id="1234567" ...>`). The id is read from inside the
/// request tag, so the attribute order does not matter and either
/// quote style works. None when the response carries no numeric id.
pub(crate) fn parse_request_id(body: &str) -> Option<String> {
    let mut rest = body;
    while let Some(pos) = rest.find("<request") {
        let after_tag = &rest[pos + "<request".len()..];
        // The tag name must end here: `<requests>` is not a request
        // element. The next character is whitespace, `/`, or `>`.
        let first = after_tag.chars().next()?;
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let end = after_tag.find('>')?;
        return request_tag_id(&after_tag[..end]);
    }
    None
}

/// Extract every request id from an OBS search response
/// (`<collection matches="N"><request id="N"/>...</collection>`).
/// None when the response is not a collection at all (no
/// `<collection` tag), when the collection carries no `matches`
/// count, when any request tag carries no numeric id, or when the
/// count says there are requests but none parsed: an unexpected
/// response is an error, never an empty list. An empty collection
/// (with its count) reads as an empty list. A non-numeric id anywhere
/// in the collection is also unexpected: dropping it silently could
/// hide a request that blocks a delete. The ids are the returned
/// page: the count is required so a malformed collection cannot read
/// as empty, but it is not compared against the number of ids — a
/// search that paginates would return fewer elements than the total,
/// and refusing on that would be a false refusal. The id is read
/// from inside each request tag, so the attribute order does not
/// matter and either quote style works.
pub(crate) fn parse_request_ids(body: &str) -> Option<Vec<String>> {
    if !body.contains("<collection") {
        return None;
    }
    let matches = parse_search_matches(body)?;
    let mut ids = Vec::new();
    let mut rest = body;
    while let Some(pos) = rest.find("<request") {
        let after_tag = &rest[pos + "<request".len()..];
        // The tag name must end here: `<requests>` is not a request
        // element. The next character is whitespace, `/`, or `>`.
        let first = after_tag.chars().next()?;
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        // The tag ends at the first `>`: an unterminated request tag
        // is a malformed collection, not a skippable element.
        let end = after_tag.find('>')?;
        let id = request_tag_id(&after_tag[..end])?;
        ids.push(id);
        rest = &after_tag[end..];
    }
    // The count says there are requests but none parsed: the server
    // changed shape underneath us, and an empty list here would let
    // the filing dedup pass and the delete pre-check proceed.
    if matches > 0 && ids.is_empty() {
        return None;
    }
    Some(ids)
}

/// The ids of the open SUBMIT requests in an OBS search response
/// (`<collection matches="N">` of full `<request>` elements): a
/// request counts only when one of its `<action>` elements has
/// `type="submit"`. The search engine does whitelist `action/@type`,
/// so the type filter could live in the match — it lives here
/// instead, on the response the server actually returns, as
/// defence-in-depth. The search match still
/// narrows to the package and the open states; this only drops the
/// non-submit requests it also returns. A request element is kept
/// only when its own block contains a submit action; a self-closing
/// request tag carries no action and never counts. None when the
/// response is not a collection at all, has no matches count, when
/// any request tag carries no numeric id, or when the count says
/// there are requests but none parsed — same fail-closed discipline
/// as [`parse_request_ids`]. A collection whose requests are all
/// non-submit reads as an empty list, not an error: that is the
/// answer "no open submit request".
pub(crate) fn parse_submit_request_ids(body: &str) -> Option<Vec<String>> {
    parse_request_ids_for_action(body, "submit", ActionShape::Any)
}

/// The ids of the requests in a search-response collection whose
/// action is of type `action_type` (state is whatever the search
/// matched) and matches `shape`. The search already filters on the
/// type server-side; the parser re-checks it here, so a misshapen
/// response cannot let a wrong-type id through the dedup — and it
/// applies the shape where the search cannot. None when the body is
/// not a collection at all, or when the count says there are
/// requests but none parsed at all — the server changed shape
/// underneath us, and an empty list there would let a filing dedup
/// pass. A collection of only other action types reads as an empty
/// list, not None.
pub(crate) fn parse_request_ids_for_action(
    body: &str,
    action_type: &str,
    shape: ActionShape<'_>,
) -> Option<Vec<String>> {
    if !body.contains("<collection") {
        return None;
    }
    let matches = parse_search_matches(body)?;
    let mut ids = Vec::new();
    let mut parsed = 0usize;
    let mut rest = body;
    while let Some(pos) = rest.find("<request") {
        let after_tag = &rest[pos + "<request".len()..];
        // The tag name must end here: `<requests>` is not a request
        // element. The next character is whitespace, `/`, or `>`.
        let first = after_tag.chars().next()?;
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        // The tag ends at the first `>`: an unterminated request tag
        // is a malformed collection, not a skippable element.
        let end = after_tag.find('>')?;
        let tag = &after_tag[..end];
        let id = request_tag_id(tag)?;
        parsed += 1;
        if tag.trim_end().ends_with('/') {
            // Self-closing: no children, so no action.
            rest = &after_tag[end..];
            continue;
        }
        let inner = &after_tag[end + 1..];
        let close = inner.find("</request>")?;
        // A truncated action block is a malformed response, not a
        // match: the parse fails so the caller reports a server
        // error instead of treating the request as blocking.
        if request_block_matches_action(&inner[..close], action_type, shape)? {
            ids.push(id);
        }
        rest = &inner[close + "</request>".len()..];
    }
    if matches > 0 && parsed == 0 {
        return None;
    }
    Some(ids)
}

/// True when a `<request>` element's inner XML contains an `<action>`
/// element whose `type` attribute is `action_type` and which matches
/// `shape`. The `type` is read from inside the action tag, so its
/// position does not matter and either quote style works; a mention of
/// the type elsewhere in the block (a description, say) does not count.
/// The shape is read from the action's `<target>` element the same way.
/// None when the block is truncated (an action tag with no closing
/// `>` or a type-matching action with no close tag): the caller fails
/// closed on a malformed response — a truncated block is a server
/// error, not a block that happens to match.
pub(crate) fn request_block_matches_action(
    block: &str,
    action_type: &str,
    shape: ActionShape<'_>,
) -> Option<bool> {
    let mut rest = block;
    while let Some(pos) = rest.find("<action") {
        let after_tag = &rest[pos + "<action".len()..];
        let Some(first) = after_tag.chars().next() else {
            // The block ends mid-tag: truncated.
            return None;
        };
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let Some(end) = after_tag.find('>') else {
            // An action tag with no closing `>` is truncated.
            return None;
        };
        let tag = &after_tag[..end];
        if action_tag_type(tag).as_deref() == Some(action_type) {
            if tag.trim_end().ends_with('/') {
                // Self-closing: no children, so no target — the shape
                // reads it as carrying nothing.
                if shape.matches_action(None) {
                    return Some(true);
                }
            } else {
                let inner = &after_tag[end + 1..];
                let Some(close) = inner.find("</action>") else {
                    // A type-matching action without a close tag is
                    // truncated.
                    return None;
                };
                if shape.matches_action(Some(&inner[..close])) {
                    return Some(true);
                }
                rest = &inner[close + "</action>".len()..];
                continue;
            }
        }
        rest = &after_tag[end..];
    }
    Some(false)
}

/// The `type` attribute of an `<action ...>` opening tag's text,
/// read order-tolerantly in either quote style. None when the tag
/// carries no `type` attribute. The attribute name must stand on its
/// own — `subtype="..."` must not match — so the character before
/// `type=` has to be whitespace.
pub(crate) fn action_tag_type(element: &str) -> Option<String> {
    tag_attr(element, "type")
}

/// True when an OBS response body carries `<status code="ok" ...>`.
/// The write endpoints (request `changestate`, source delete) answer
/// `render_ok` on success; any other status code in the body — or no
/// status element at all — reads as failure, never success.
pub(crate) fn parse_status_ok(body: &str) -> bool {
    let Some(pos) = body.find("<status") else {
        return false;
    };
    let after_tag = &body[pos + "<status".len()..];
    // The tag name must end here: `<statuscode>` is not a status
    // element. The next character is whitespace, `/`, or `>`.
    let first = match after_tag.chars().next() {
        Some(c) => c,
        None => return false,
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return false;
    }
    // The code must be an attribute of this element, not text after it
    // — and it must be the `code` attribute, not a longer name that
    // ends in it (`mycode="ok"` is not `code="ok"`). The tag end is
    // found quote-aware, so a `>` inside an earlier attribute's value
    // does not cut the element short; the value is read with
    // `xml_attr`, which is quote-aware and tolerates whitespace
    // around `=`.
    let Some(end) = find_tag_end(after_tag) else {
        return false;
    };
    let element = &after_tag[..end];
    xml_attr(element, "code").is_some_and(|code| code == "ok")
}

/// The opening tag of the first `<name ...>` element in `body`
/// (without the closing `>`), or None when there is none. The name
/// must end at the tag boundary, so `<package>` does not match
/// `<packages>`.
pub(crate) fn find_element_tag<'a>(body: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}");
    let mut rest = body;
    while let Some(pos) = rest.find(&open) {
        let after_tag = &rest[pos + open.len()..];
        let first = after_tag.chars().next()?;
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let end = after_tag.find('>')?;
        return Some(&after_tag[..end]);
    }
    None
}

/// The text of the first `<data name="NAME">VALUE</data>` element in
/// a `<status>` document. None when there is no such element, or it
/// is self-closing (an empty value is not a result). The name is
/// matched exactly, quoted: `<data name="target_project">` does not
/// match a search for `project`.
pub(crate) fn status_data(body: &str, name: &str) -> Option<String> {
    let open = format!("<data name=\"{name}\">");
    let pos = body.find(&open)?;
    let after = &body[pos + open.len()..];
    let end = after.find("</data>")?;
    let value = &after[..end];
    if value.is_empty() {
        return None;
    }
    Some(value.to_string())
}

/// True when `body` is a `<status code="invoked"/>` document: the
/// server queued the operation as a job instead of running it (a
/// project copy or a project release without `nodelay`). The same
/// tag-boundary discipline as [`parse_status_ok`]: the code must be
/// the `code` attribute of the `<status>` element itself, not text
/// after it and not a longer attribute name ending in it.
pub(crate) fn parse_status_invoked(body: &str) -> bool {
    let Some(pos) = body.find("<status") else {
        return false;
    };
    let after_tag = &body[pos + "<status".len()..];
    // The tag name must end here: `<statuscode>` is not a status
    // element. The next character is whitespace, `/`, or `>`.
    let first = match after_tag.chars().next() {
        Some(c) => c,
        None => return false,
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return false;
    }
    // The code must be an attribute of this element, not text after it
    // — and it must be the `code` attribute, not a longer name that
    // ends in it (`mycode="invoked"` is not `code="invoked"`). The tag
    // end is found quote-aware, so a `>` inside an earlier attribute's
    // value does not cut the element short; the value is read with
    // `xml_attr`, which is quote-aware and tolerates whitespace
    // around `=`.
    let Some(end) = find_tag_end(after_tag) else {
        return false;
    };
    let element = &after_tag[..end];
    xml_attr(element, "code").is_some_and(|code| code == "invoked")
}

/// The value of attribute `name` in an opening tag's text (the text
/// between `<tag` and `>`), read order-tolerantly in either quote
/// style. None when the tag carries no such attribute. The attribute
/// name must stand on its own — `myname="..."` must not match — so
/// the character before the name has to be whitespace. The scan is
/// quote-aware: a `name=` inside an earlier attribute's value must
/// not match, and a malformed occurrence (an unquoted value) is
/// skipped, not fatal. Whitespace around `=` is allowed: `name =
/// "value"` is the same attribute. An empty value reads as absent.
pub(crate) fn tag_attr(element: &str, name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    let bytes = element.as_bytes();
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
            && element[i..].starts_with(name)
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
                    let after = &element[j + 1..];
                    let close = after.find(quote as char)?;
                    let value = &after[..close];
                    if value.is_empty() {
                        return None;
                    }
                    return Some(value.to_string());
                }
            }
        }
        i += 1;
    }
    None
}

/// Restrict `entries` (server order, oldest first) to `range`.
/// A numeric bound compares revision numbers; an md5 bound resolves
/// to the revision carrying that md5 (an unknown md5 is an error,
/// never an empty list). A one-sided range from a single specifier
/// matches exactly that revision; `"a:b"` matches `a..=b`, and a
/// reversed range is an error.
pub(crate) fn filter_history(
    entries: Vec<HistoryEntry>,
    range: &RevisionRange,
) -> Result<Vec<HistoryEntry>, GeectlError> {
    let resolve = |spec: &RevisionSpec| -> Result<u64, GeectlError> {
        match spec {
            RevisionSpec::Rev(number) => Ok(*number),
            RevisionSpec::Md5(md5) => entries
                .iter()
                .find(|entry| entry.srcmd5.as_deref() == Some(md5.as_str()))
                .map(|entry| entry.rev)
                .ok_or_else(|| {
                    GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!("revision md5 {md5:?} not found in the package history"),
                    )
                }),
        }
    };
    let (low, high) = match (&range.from, &range.to) {
        (None, None) => return Ok(entries),
        (Some(from), None) => {
            let rev = resolve(from)?;
            (rev, rev)
        }
        (None, Some(to)) => {
            let rev = resolve(to)?;
            (0, rev)
        }
        (Some(from), Some(to)) => {
            let from = resolve(from)?;
            let to = resolve(to)?;
            if from > to {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("reversed revision range: {from}..{to}"),
                ));
            }
            (from, to)
        }
    };
    Ok(entries
        .into_iter()
        .filter(|entry| entry.rev >= low && entry.rev <= high)
        .collect())
}

/// The text of the first `<tag>...</tag>` element in `block`, or
/// None when there is none. The text is XML-unescaped. A
/// self-closing `<tag/>` carries no text and reads as None.
pub(crate) fn element_text(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let mut rest = block;
    while let Some(pos) = rest.find(&open) {
        let after_tag = &rest[pos + open.len()..];
        let first = after_tag.chars().next()?;
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let end = after_tag.find('>')?;
        let open_text = &after_tag[..end];
        if open_text.trim_end().ends_with('/') {
            return None;
        }
        let inner = &after_tag[end + 1..];
        let close = format!("</{tag}>");
        let close_pos = inner.find(&close)?;
        return Some(xml_unescape(&inner[..close_pos]));
    }
    None
}

/// Each `<tag ...>...</tag>` (or self-closing `<tag .../>`) element
/// in `block`, as `(opening-tag text, inner text)`. The tag name must
/// stand on its own. An unterminated element is an error, never a
/// skipped element.
pub(crate) fn elements(block: &str, tag: &str) -> Result<Vec<(String, String)>, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected OBS response: malformed <{tag}> element"),
        )
    };
    let open = format!("<{tag}");
    let mut out = Vec::new();
    let mut rest = block;
    while let Some(pos) = rest.find(&open) {
        let after_tag = &rest[pos + open.len()..];
        let Some(first) = after_tag.chars().next() else {
            return Err(malformed());
        };
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let Some(end) = after_tag.find('>') else {
            return Err(malformed());
        };
        let element = &after_tag[..end];
        if element.trim_end().ends_with('/') {
            out.push((element.to_string(), String::new()));
            rest = &after_tag[end + 1..];
        } else {
            let inner = &after_tag[end + 1..];
            let close = format!("</{tag}>");
            let Some(close_pos) = inner.find(&close) else {
                return Err(malformed());
            };
            out.push((element.to_string(), inner[..close_pos].to_string()));
            rest = &inner[close_pos + close.len()..];
        }
    }
    Ok(out)
}

/// The `<person userid role/>` and `<group groupid role/>` children
/// of a meta document. An element without its name or role attribute
/// is skipped, not an error: only the meta envelope itself is
/// validated here.
pub(crate) fn meta_roles(block: &str) -> Result<(Vec<NamedRole>, Vec<NamedRole>), GeectlError> {
    let mut persons = Vec::new();
    for (element, _) in elements(block, "person")? {
        if let (Some(name), Some(role)) = (tag_attr(&element, "userid"), tag_attr(&element, "role"))
        {
            persons.push(NamedRole {
                name: xml_unescape(&name),
                role: xml_unescape(&role),
            });
        }
    }
    let mut groups = Vec::new();
    for (element, _) in elements(block, "group")? {
        if let (Some(name), Some(role)) =
            (tag_attr(&element, "groupid"), tag_attr(&element, "role"))
        {
            groups.push(NamedRole {
                name: xml_unescape(&name),
                role: xml_unescape(&role),
            });
        }
    }
    Ok((persons, groups))
}

/// Split a meta document into its root opening-tag text and inner
/// text. The body must be a `<root>` document: a body that is not
/// one (empty, truncated, or some other document) is an error, never
/// a meta with no name. A self-closing root reads as an empty inner
/// text.
pub(crate) fn meta_envelope<'a>(
    body: &'a str,
    root: &str,
) -> Result<(String, &'a str), GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected OBS meta response: not a <{root}> document"),
        )
    };
    let body = body.trim();
    let open = format!("<{root}");
    let Some(after_open) = body.strip_prefix(&open) else {
        return Err(malformed());
    };
    // The tag name must end here: `<packages>` is not a package
    // element. The next character is whitespace, `/`, or `>`.
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = after_open.find('>') else {
        return Err(malformed());
    };
    let open_tag = after_open[..end].to_string();
    if open_tag.trim_end().ends_with('/') {
        return Ok((open_tag, ""));
    }
    let inner = &after_open[end + 1..];
    let close = format!("</{root}>");
    let Some(close_pos) = inner.find(&close) else {
        return Err(malformed());
    };
    // Nothing but whitespace may follow the closing tag.
    if !inner[close_pos + close.len()..].trim().is_empty() {
        return Err(malformed());
    }
    Ok((open_tag, &inner[..close_pos]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_obs_name_accepts_and_rejects() {
        assert!(valid_obs_name("python3-requests"));
        assert!(valid_obs_name("foo:bar"));
        assert!(valid_obs_name("gcc-c++"));
        assert!(!valid_obs_name(""));
        assert!(!valid_obs_name("foo/bar"));
        assert!(!valid_obs_name("foo?x=1"));
        assert!(!valid_obs_name("foo' or '1'='1"));
    }

    #[test]
    fn parse_search_matches_counts() {
        assert_eq!(
            parse_search_matches("<collection matches=\"3\"></collection>"),
            Some(3)
        );
        assert_eq!(
            parse_search_matches("<collection matches=\"0\"></collection>"),
            Some(0)
        );
        assert_eq!(parse_search_matches("<collection></collection>"), None);
        // Either quote style carries the count.
        assert_eq!(
            parse_search_matches("<collection matches='7'></collection>"),
            Some(7)
        );
    }

    #[test]
    fn quoted_attr_value_skips_a_malformed_first_occurrence() {
        // An unquoted first occurrence does not stop the scan: the
        // well-formed later one wins.
        assert_eq!(
            quoted_attr_value(" project=devel:bad project=\"devel:foo\"", "project"),
            Some("devel:foo".to_string())
        );
        // An unterminated first occurrence likewise does not stop it:
        // its opening quote never closes (the later value quotes with
        // the other style, so it cannot close the first one).
        assert_eq!(
            quoted_attr_value(" project=\"devel:bad project='devel:foo'", "project"),
            Some("devel:foo".to_string())
        );
        // Every occurrence malformed: None, not a partial value.
        assert_eq!(quoted_attr_value(" project=devel:bad", "project"), None);
        assert_eq!(quoted_attr_value(" project=\"devel:bad", "project"), None);
    }

    #[test]
    fn quoted_attr_value_matches_the_attribute_name_only() {
        // `subproject="..."` must not match `project=`.
        assert_eq!(quoted_attr_value(" subproject=\"foo\"", "project"), None);
        // With both present, the real `project` value wins.
        assert_eq!(
            quoted_attr_value(" subproject=\"foo\" project=\"bar\"", "project"),
            Some("bar".to_string())
        );
    }

    #[test]
    fn quoted_attr_value_tolerates_whitespace_and_a_name_inside_a_value() {
        // Whitespace around `=` is the same attribute.
        assert_eq!(
            quoted_attr_value(" project = \"devel:foo\"", "project"),
            Some("devel:foo".to_string())
        );
        assert_eq!(
            quoted_attr_value(" project= \"devel:foo\"", "project"),
            Some("devel:foo".to_string())
        );
        // `project=` inside an earlier attribute's value must not
        // match: the real `project` attribute follows it.
        assert_eq!(
            quoted_attr_value(" title=\"see project=x\" project=\"devel:foo\"", "project"),
            Some("devel:foo".to_string())
        );
    }

    #[test]
    fn parse_request_id_extracts_numeric_id() {
        assert_eq!(
            parse_request_id("<request id=\"1400001\" creator=\"scarabeus_iv\"></request>"),
            Some("1400001".to_string())
        );
        assert_eq!(parse_request_id("<request></request>"), None);
        assert_eq!(parse_request_id("<request id=\"abc\"></request>"), None);
        assert_eq!(parse_request_id(""), None);
        // The attribute order does not matter, and either quote style
        // works: the id is read from inside the request tag.
        assert_eq!(
            parse_request_id("<request creator=\"scarabeus_iv\" id=\"1400001\"></request>"),
            Some("1400001".to_string())
        );
        assert_eq!(
            parse_request_id("<request id='1400001'></request>"),
            Some("1400001".to_string())
        );
    }

    #[test]
    fn request_tag_id_skips_a_malformed_first_occurrence() {
        // An unquoted first occurrence does not stop the scan: the
        // well-formed later one wins.
        assert_eq!(
            request_tag_id(" id=abc id=\"1400001\""),
            Some("1400001".to_string())
        );
        // An unterminated first occurrence likewise does not stop it:
        // its opening quote never closes (the later value quotes with
        // the other style, so it cannot close the first one).
        assert_eq!(
            request_tag_id(" id=\"1400001 id='1400002'"),
            Some("1400002".to_string())
        );
        // A non-numeric first occurrence likewise does not stop it.
        assert_eq!(
            request_tag_id(" id=\"abc\" id=\"1400001\""),
            Some("1400001".to_string())
        );
        // Every occurrence malformed: None, not a partial id.
        assert_eq!(request_tag_id(" id=abc"), None);
        assert_eq!(request_tag_id(" id=\"abc\""), None);
        assert_eq!(request_tag_id(" id=\"1400001"), None);
        assert_eq!(request_tag_id(" id="), None);
    }

    #[test]
    fn parse_request_ids_requires_a_matches_count() {
        // A collection without its matches count is unexpected, not
        // empty: without the count a malformed response would read as
        // "no requests", which is exactly the answer that lets a
        // delete or a refile proceed.
        assert_eq!(parse_request_ids("<collection></collection>"), None);
        assert_eq!(parse_request_ids(""), None);
        assert_eq!(
            parse_request_ids("<collection matches=\"0\"></collection>"),
            Some(vec![])
        );
        // The count says there are requests but none parsed: the
        // server changed shape, and an empty list would let the
        // filing dedup pass and the delete pre-check proceed.
        assert_eq!(
            parse_request_ids("<collection matches=\"1\"></collection>"),
            None
        );
        assert_eq!(
            parse_request_ids(
                "<collection matches=\"1\"><request creator=\"scarabeus_iv\" /></collection>"
            ),
            None
        );
    }

    #[test]
    fn parse_status_ok_reads_the_status_code() {
        assert!(parse_status_ok("<status code=\"ok\" />"));
        assert!(parse_status_ok("<status code=\"ok\"/>"));
        assert!(!parse_status_ok(
            "<status code=\"not_found\"><summary>no such request</summary></status>"
        ));
        assert!(!parse_status_ok("<status code=\"deleted\" />"));
        assert!(!parse_status_ok(""));
        assert!(!parse_status_ok("<request id=\"1398492\" />"));
        // A `code="ok"` outside the status element does not count.
        assert!(!parse_status_ok("<other code=\"ok\" />"));
        // A `code="ok"` inside a longer attribute name is not the
        // status code.
        assert!(!parse_status_ok("<status mycode=\"ok\" />"));
        assert!(parse_status_ok("<status code='ok' />"));
    }

    #[test]
    fn parse_submit_request_ids_keeps_only_submit_requests() {
        // The search response carries full request elements: a
        // submit action counts, a delete action does not, and a
        // self-closing request tag carries no action at all.
        assert_eq!(
            parse_submit_request_ids(
                "<collection matches=\"3\"><request id=\"1\"><action type=\"submit\"><source project=\"a\" package=\"foo\" /><target project=\"b\" package=\"foo\" /></action><state name=\"new\" /></request><request id=\"2\"><action type=\"delete\"><target project=\"b\" package=\"foo\" /></action><state name=\"new\" /></request><request id=\"3\" /></collection>"
            ),
            Some(vec!["1".to_string()])
        );
        // All non-submit still parses: the answer is an empty list,
        // not an error — that is "no open submit request".
        assert_eq!(
            parse_submit_request_ids(
                "<collection matches=\"1\"><request id=\"2\"><action type=\"delete\"><target project=\"b\" package=\"foo\" /></action><state name=\"new\" /></request></collection>"
            ),
            Some(vec![])
        );
        // Not a collection, or no count, or a count with nothing
        // parsed, is unexpected, never empty.
        assert_eq!(parse_submit_request_ids(""), None);
        assert_eq!(parse_submit_request_ids("<collection></collection>"), None);
        assert_eq!(
            parse_submit_request_ids("<collection matches=\"1\"></collection>"),
            None
        );
        // The action type is read from the tag, not the raw text: a
        // description mentioning submit does not count.
        assert_eq!(
            parse_submit_request_ids(
                "<collection matches=\"1\"><request id=\"2\"><action type=\"delete\" /><description>submit this please</description></request></collection>"
            ),
            Some(vec![])
        );
        // Either quote style works, and the type may sit anywhere in
        // the action tag.
        assert_eq!(
            parse_submit_request_ids(
                "<collection matches=\"1\"><request id=\"2\"><action foo=\"x\" type='submit' /></request></collection>"
            ),
            Some(vec!["2".to_string()])
        );
    }

    #[test]
    fn parse_submit_request_ids_truncated_action_is_malformed() {
        // A type-matching action with no close tag is a malformed
        // response, not a block: the parse fails so filing reports a
        // server error instead of "already targets".
        assert_eq!(
            parse_submit_request_ids(
                "<collection matches=\"1\"><request id=\"1\"><action type=\"submit\"><target project=\"b\" package=\"foo\" /></request></collection>"
            ),
            None
        );
        // An action tag with no closing `>` is truncated the same way.
        assert_eq!(
            parse_submit_request_ids(
                "<collection matches=\"1\"><request id=\"1\"><action type=\"submit\"</request></collection>"
            ),
            None
        );
    }

    #[test]
    fn parse_request_ids_collects_all_ids() {
        assert_eq!(
            parse_request_ids(
                "<collection matches=\"2\"><request id=\"1398492\" /><request id=\"1398493\" /></collection>"
            ),
            Some(vec!["1398492".to_string(), "1398493".to_string()])
        );
        assert_eq!(
            parse_request_ids("<collection matches=\"0\"></collection>"),
            Some(vec![])
        );
        // Not a collection at all: unexpected, not empty.
        assert_eq!(parse_request_ids("<request id=\"1398492\" />"), None);
        assert_eq!(parse_request_ids(""), None);
        // A malformed id hides a request: unexpected, not partial.
        assert_eq!(
            parse_request_ids("<collection matches=\"1\"><request id=\"abc\" /></collection>"),
            None
        );
        // The id is read from inside the request tag: the attribute
        // order does not matter, and either quote style works.
        assert_eq!(
            parse_request_ids(
                "<collection matches=\"2\"><request creator=\"scarabeus_iv\" id=\"1398492\" /><request id='1398493' /></collection>"
            ),
            Some(vec!["1398492".to_string(), "1398493".to_string()])
        );
    }

    #[test]
    fn parse_status_invoked_reads_the_status_code() {
        assert!(parse_status_invoked(
            "<status code=\"invoked\"><summary>Job invoked</summary></status>"
        ));
        assert!(parse_status_invoked("<status code=\"invoked\" />"));
        assert!(!parse_status_invoked("<status code=\"ok\" />"));
        assert!(!parse_status_invoked("<status code=\"error\" />"));
        assert!(!parse_status_invoked(""));
        assert!(!parse_status_invoked("<other code=\"invoked\" />"));
        assert!(!parse_status_invoked("<status mycode=\"invoked\" />"));
        assert!(parse_status_invoked("<status code='invoked' />"));
    }

    #[test]
    fn parse_status_readers_tolerate_whitespace_and_gt_in_values() {
        // Whitespace around `=` is the same attribute.
        assert!(parse_status_ok("<status code = \"ok\" />"));
        assert!(parse_status_ok("<status code= \"ok\" />"));
        assert!(parse_status_invoked("<status code = \"invoked\" />"));
        // A `>` inside an earlier attribute's value does not cut the
        // element short: the code after it still reads.
        assert!(parse_status_ok("<status summary=\"a > b\" code=\"ok\" />"));
        assert!(parse_status_ok("<status code=\"ok\" summary=\"x>y\"/>"));
        assert!(parse_status_invoked(
            "<status summary=\"a > b\" code=\"invoked\" />"
        ));
        // Still fail-closed on the wrong code.
        assert!(!parse_status_ok("<status code = \"failed\" />"));
        assert!(!parse_status_invoked("<status code = \"ok\" />"));
    }

    #[test]
    fn slash_split_drops_empty_segments() {
        // No slash, leading/trailing slash, middle slash, `//` and
        // `/////////` collapse (empty segments dropped).
        assert_eq!(split_slash_segments("foo"), vec!["foo"]);
        assert_eq!(split_slash_segments("a/b"), vec!["a", "b"]);
        assert_eq!(split_slash_segments("/a/b"), vec!["a", "b"]);
        assert_eq!(split_slash_segments("a/b/"), vec!["a", "b"]);
        assert_eq!(split_slash_segments("a//b"), vec!["a", "b"]);
        assert!(split_slash_segments("//").is_empty());
        assert!(split_slash_segments("/////////").is_empty());
        assert!(split_slash_segments("").is_empty());
    }

    #[test]
    fn project_package_split_needs_exactly_two_segments() {
        assert_eq!(
            parse_project_package("devel:languages:python/foo").unwrap(),
            ("devel:languages:python".to_string(), "foo".to_string())
        );
        assert_eq!(
            parse_project_package("/a/b/").unwrap(),
            ("a".to_string(), "b".to_string())
        );
        assert!(parse_project_package("foo").is_err());
        assert!(parse_project_package("").is_err());
        assert!(parse_project_package("a/b/c").is_err());
        assert!(parse_project_package("a//b/c").is_err());
    }

    #[test]
    fn tag_attr_reads_either_quote_style_in_any_order() {
        assert_eq!(
            tag_attr("entry name=\"foo\" md5=\"abc\"", "name").as_deref(),
            Some("foo")
        );
        assert_eq!(
            tag_attr("entry md5='abc' name='foo'", "name").as_deref(),
            Some("foo")
        );
        // The name must stand on its own: `myname` does not match.
        assert_eq!(tag_attr("entry myname=\"foo\"", "name"), None);
        assert_eq!(tag_attr("entry name=\"\"", "name"), None);
        assert_eq!(tag_attr("entry", "name"), None);
    }

    #[test]
    fn tag_attr_tolerates_whitespace_and_skips_a_malformed_first_occurrence() {
        // Whitespace around `=` is the same attribute.
        assert_eq!(
            tag_attr("entry name = \"foo\"", "name").as_deref(),
            Some("foo")
        );
        assert_eq!(
            tag_attr("entry name= \"foo\"", "name").as_deref(),
            Some("foo")
        );
        // `name=` inside an earlier attribute's value must not match:
        // the real `name` attribute follows it.
        assert_eq!(
            tag_attr("entry details=\"see name=x\" name=\"foo\"", "name").as_deref(),
            Some("foo")
        );
        // A malformed first occurrence does not stop the scan: the
        // well-formed later one wins.
        assert_eq!(
            tag_attr("entry name=bad name=\"foo\"", "name").as_deref(),
            Some("foo")
        );
        // Either quote style still works with whitespace.
        assert_eq!(
            tag_attr("entry name = 'foo'", "name").as_deref(),
            Some("foo")
        );
    }

    fn mixed_action_collection() -> &'static str {
        "<collection matches=\"4\">\
        <request id=\"1\" creator=\"alice\"><action type=\"submit\"><source project=\"home:alice\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request>\
        <request id=\"2\" creator=\"alice\"><action type=\"maintenance_incident\"><source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" /><target project=\"openSUSE:Maintenance\" /></action><state name=\"review\" /></request>\
        <request id=\"3\" creator=\"alice\"><action type=\"change_devel\"><source project=\"devel:languages:python\" package=\"foo\" /><target project=\"openSUSE:Factory\" package=\"foo\" /></action><state name=\"new\" /></request>\
        <request id=\"4\" creator=\"alice\"><action type=\"delete\"><target project=\"openSUSE:Factory\" package=\"foo\" /></action><state name=\"declined\" /></request>\
        </collection>"
    }

    #[test]
    fn parse_request_ids_for_action_filters_by_type() {
        let body = mixed_action_collection();
        assert_eq!(
            parse_request_ids_for_action(body, "maintenance_incident", ActionShape::Any).unwrap(),
            vec!["2".to_string()]
        );
        assert_eq!(
            parse_request_ids_for_action(body, "change_devel", ActionShape::Any).unwrap(),
            vec!["3".to_string()]
        );
        assert_eq!(
            parse_request_ids_for_action(body, "delete", ActionShape::Any).unwrap(),
            vec!["4".to_string()]
        );
        // The submit path delegates to the same parser.
        assert_eq!(
            parse_submit_request_ids(body).unwrap(),
            vec!["1".to_string()]
        );
    }

    #[test]
    fn parse_request_ids_for_action_filters_delete_shape() {
        let body = "<collection matches=\"5\"><request id=\"1\" creator=\"alice\"><action type=\"delete\"><target project=\"openSUSE:Factory\" package=\"foo\" /></action><state name=\"new\" /></request><request id=\"2\" creator=\"alice\"><action type=\"delete\"><target project=\"openSUSE:Factory\" package=\"bar\" /></action><state name=\"new\" /></request><request id=\"3\" creator=\"alice\"><action type=\"delete\"><target project=\"openSUSE:Factory\" repository=\"standard\" /></action><state name=\"new\" /></request><request id=\"4\" creator=\"alice\"><action type=\"delete\"><target project=\"openSUSE:Factory\" /></action><state name=\"new\" /></request><request id=\"5\" creator=\"alice\"><action type=\"submit\"><source project=\"home:alice\" package=\"foo\" /><target project=\"openSUSE:Factory\" package=\"foo\" /></action><state name=\"new\" /></request></collection>";
        // A package delete is blocked by the same package and by a
        // whole-project delete, not by another package or a repository.
        assert_eq!(
            parse_request_ids_for_action(
                body,
                "delete",
                ActionShape::DeleteTarget {
                    package: Some("foo"),
                    repository: None,
                }
            )
            .unwrap(),
            vec!["1".to_string(), "4".to_string()]
        );
        // A repository delete: the same repository, or whole-project.
        assert_eq!(
            parse_request_ids_for_action(
                body,
                "delete",
                ActionShape::DeleteTarget {
                    package: None,
                    repository: Some("standard"),
                }
            )
            .unwrap(),
            vec!["3".to_string(), "4".to_string()]
        );
        // A whole-project delete: a whole-project delete blocks it,
        // and so does a repository delete — the two share the server's
        // delete identity, which carries no repository.
        assert_eq!(
            parse_request_ids_for_action(
                body,
                "delete",
                ActionShape::DeleteTarget {
                    package: None,
                    repository: None,
                }
            )
            .unwrap(),
            vec!["3".to_string(), "4".to_string()]
        );
    }

    #[test]
    fn parse_request_ids_for_action_filters_release_project() {
        let body = "<collection matches=\"2\"><request id=\"1\" creator=\"alice\"><action type=\"maintenance_incident\"><source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" /><target project=\"openSUSE:Maintenance\" releaseproject=\"openSUSE:Leap:15.6:Update\" /></action><state name=\"new\" /></request><request id=\"2\" creator=\"alice\"><action type=\"maintenance_incident\"><source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" /><target project=\"openSUSE:Maintenance\" /></action><state name=\"new\" /></request></collection>";
        // The server's incident identity includes the release project:
        // only the same one blocks a filing that names one. A filing
        // without a release project matches any stored value: the
        // server stores the filing-time-resolved value even when the
        // filed action named none.
        assert_eq!(
            parse_request_ids_for_action(
                body,
                "maintenance_incident",
                ActionShape::ReleaseProject(Some("openSUSE:Leap:15.6:Update"))
            )
            .unwrap(),
            vec!["1".to_string()]
        );
        assert_eq!(
            parse_request_ids_for_action(
                body,
                "maintenance_incident",
                ActionShape::ReleaseProject(None)
            )
            .unwrap(),
            vec!["1".to_string(), "2".to_string()]
        );
    }

    #[test]
    fn parse_request_ids_for_action_blocks_targetless_action_for_release_project() {
        // A self-closing incident action has no target to read: it
        // blocks, fail-closed, even when a release project is expected.
        let body = "<collection matches=\"1\"><request id=\"7\" creator=\"alice\"><action type=\"maintenance_incident\" /></request></collection>";
        assert_eq!(
            parse_request_ids_for_action(
                body,
                "maintenance_incident",
                ActionShape::ReleaseProject(Some("openSUSE:Leap:15.6:Update"))
            )
            .unwrap(),
            vec!["7".to_string()]
        );
    }

    #[test]
    fn parse_request_ids_for_action_rejects_truncated_matching_action() {
        // A type-matching action without a close tag is malformed:
        // the parse fails instead of the id slipping past the dedup
        // as a duplicate — the caller reports an unexpected response,
        // not "already targets".
        let body = "<collection matches=\"1\"><request id=\"7\" creator=\"alice\"><action type=\"delete\"><target project=\"openSUSE:Factory\" package=\"foo\" /></request></collection>";
        assert!(parse_request_ids_for_action(body, "delete", ActionShape::Any).is_none());
    }

    #[test]
    fn parse_request_ids_for_action_rejects_unterminated_action_tag() {
        // An action tag with no closing `>` is truncated: malformed,
        // not a match.
        let body = "<collection matches=\"1\"><request id=\"7\" creator=\"alice\"><action type=\"delete\"</request></collection>";
        assert!(parse_request_ids_for_action(body, "delete", ActionShape::Any).is_none());
    }

    #[test]
    fn parse_request_ids_for_action_rejects_bare_action_tag_at_end() {
        // The request ends with a bare `<action`: truncated —
        // malformed, not a match.
        let body = "<collection matches=\"1\"><request id=\"7\" creator=\"alice\"><action</request></collection>";
        assert!(parse_request_ids_for_action(body, "delete", ActionShape::Any).is_none());
    }

    #[test]
    fn parse_request_ids_for_action_empty_when_no_match() {
        let body = "<collection matches=\"1\"><request id=\"1\" creator=\"alice\"><action type=\"submit\"><source project=\"home:alice\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request></collection>";
        assert_eq!(
            parse_request_ids_for_action(body, "maintenance_incident", ActionShape::Any).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn parse_request_ids_for_action_rejects_malformed() {
        assert!(parse_request_ids_for_action("not xml", "submit", ActionShape::Any).is_none());
        // The count says there are requests but none parsed: the
        // server changed shape, and an empty list would let a filing
        // dedup pass.
        assert!(parse_request_ids_for_action(
            "<collection matches=\"1\"><request /></collection>",
            "submit",
            ActionShape::Any
        )
        .is_none());
    }

    #[test]
    fn tag_attr_reads_quoted_values_and_respects_boundaries() {
        assert_eq!(
            tag_attr("<package project=\"foo\" package='bar' />", "project").unwrap(),
            "foo".to_string()
        );
        assert_eq!(
            tag_attr("<package project=\"foo\" package='bar' />", "package").unwrap(),
            "bar".to_string()
        );
        // `myproject` does not read as `project`.
        assert!(tag_attr("<package myproject=\"foo\" />", "project").is_none());
        // Unquoted or empty: no value.
        assert!(tag_attr("<package project=foo />", "project").is_none());
        assert!(tag_attr("<package project=\"\" />", "project").is_none());
        assert!(tag_attr("<package />", "project").is_none());
    }

    #[test]
    fn valid_obs_filename_rejects_build_route_leaves() {
        // Under `/build/.../<package>/` these address routes, not
        // files: a download by one of them would return route output
        // as if it were a file.
        for name in [
            "_log",
            "_status",
            "_buildinfo",
            "_history",
            "_jobstatus",
            "_jobhistory",
        ] {
            assert!(!valid_obs_filename(name), "{name} must be rejected");
        }
        assert!(valid_obs_filename("foo-1.0-1.x86_64.rpm"));
        assert!(valid_obs_filename("rpmlint.log"));
    }
}
