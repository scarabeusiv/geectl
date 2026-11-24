//! The debug channel: one coherent `--debug` stream on stderr.
//!
//! The channel carries human-readable lines, one per event, in time
//! order: HTTP attempts (method, path, status or error kind,
//! duration, attempt against the retry budget), read-after-write
//! retry probes, and library decision points (dedup checks, the
//! DryRun guard, endpoint selection, the accept-state check before a
//! retried `change_state`). The machine-readable verdict already
//! exists (the JSON error payload plus the exit code), so debug lines
//! are for a human diagnosing a run, never for parsing.
//!
//! The sink ([`Debug`]) is the public handle: an enabled flag plus
//! pre-formed lines. Call sites check `enabled()` before formatting,
//! so there is zero cost when the channel is off. The sink takes
//! pre-formed lines only — never a header map, never a credential
//! value — so a secret cannot reach it by accident.
//!
//! The emit path goes through the [`log`] facade, to a minimal
//! bespoke [`log::Log`] backend installed by [`install`] that writes
//! the plain human line to stderr. There is no `env_logger`, no
//! `RUST_LOG` semantics, and no level beyond the single channel:
//! the backend accepts only records from this module (the target of
//! the `log::debug!` in [`Debug::emit`]) — another crate's `debug!`
//! records (ureq's, on every request) are dropped, so the channel
//! never prints foreign logs. Debug never fails a run: a stderr
//! write failure is ignored, like `write_error` in the binary.
//!
//! Request and response bodies are never printed by default. Behind
//! `--debug-body` only, a body is printed size-capped
//! ([`DEBUG_BODY_CAP`] bytes, with a truncation marker) and
//! credential-redacted ([`redact_body`], credential values only —
//! everything else passes through); headers are never printed, and a body that
//! looks binary is shown as `<binary body, N bytes>` and never
//! dumped.

use std::io::Write;
use std::sync::{Arc, Mutex, Once};

/// The maximum number of body bytes printed behind `--debug-body`.
/// A longer body is truncated at a character boundary with a marker
/// naming the total length.
pub const DEBUG_BODY_CAP: usize = 2048;

/// The debug sink: the public handle the library's call sites use.
///
/// `enabled` turns the channel on; `bodies` additionally prints
/// request/response bodies (the `--debug-body` flag, which implies
/// `--debug`). `capture` is the test seam: a sink built with
/// [`Debug::captured`] records its lines in the returned buffer
/// instead of sending them to the `log` backend, so tests assert on
/// exact lines without a live server or a stderr capture.
#[derive(Debug, Clone)]
pub struct Debug {
    enabled: bool,
    bodies: bool,
    capture: Option<Arc<Mutex<Vec<String>>>>,
}

impl Debug {
    /// A sink with the channel off (the default: every constructor
    /// builds one, and the caller opts in with `with_debug`).
    pub fn disabled() -> Self {
        Debug {
            enabled: false,
            bodies: false,
            capture: None,
        }
    }

    /// A sink with the given channel settings, writing to stderr
    /// through the `log` backend.
    pub fn new(enabled: bool, bodies: bool) -> Self {
        Debug {
            enabled,
            bodies,
            capture: None,
        }
    }

    /// A sink for tests: like [`Debug::new`], but lines are recorded
    /// in the returned buffer instead of written to stderr. The
    /// buffer is shared with the sink, so a test reads it after the
    /// run under test.
    pub fn captured(enabled: bool, bodies: bool) -> (Self, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        (
            Debug {
                enabled,
                bodies,
                capture: Some(Arc::clone(&lines)),
            },
            lines,
        )
    }

    /// True when the channel is on.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// True when bodies print behind the channel (`--debug-body`).
    pub fn bodies_enabled(&self) -> bool {
        self.enabled && self.bodies
    }

    /// Emit one pre-formed line. A no-op when the channel is off.
    /// The line goes to the capture buffer in tests, and through
    /// `log::debug!` to the stderr backend otherwise.
    pub fn emit(&self, line: &str) {
        if !self.enabled {
            return;
        }
        if let Some(capture) = &self.capture {
            if let Ok(mut lines) = capture.lock() {
                lines.push(line.to_string());
            }
            return;
        }
        install();
        log::debug!("{line}");
    }
}

/// The `log` target the backend accepts: this module, the target of
/// the `log::debug!` in [`Debug::emit`]. Anything else — ureq's own
/// `debug!` records included — is foreign to the channel and is
/// dropped, so enabling `--debug` never prints another crate's logs.
const DEBUG_TARGET: &str = module_path!();

/// The bespoke `log` backend: writes each record's plain message to
/// stderr, one line, ignoring write failures. No timestamps, no
/// levels beyond the single channel; the only filter is the target
/// ([`DEBUG_TARGET`]): the sink takes pre-formed lines, and the
/// target check keeps other crates' records out.
struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Debug && metadata.target() == DEBUG_TARGET
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let stderr = std::io::stderr();
            let mut handle = stderr.lock();
            let _ = writeln!(handle, "{}", record.args());
        }
    }

    fn flush(&self) {}
}

static LOGGER: StderrLogger = StderrLogger;

/// Install the stderr backend once, and cap the `log` max level at
/// `Debug` so the channel's records reach it. Safe to call
/// repeatedly; a logger installed by the host first is left alone.
/// The max level is raised only when `set_logger` actually
/// succeeded: a failed install must not raise the level for a
/// foreign logger, which would let other crates' records through.
/// Callers install only when the channel is enabled (see
/// [`Debug::emit`] and the binary), so a run without `--debug`
/// never touches the global logger at all.
pub fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if log::set_logger(&LOGGER).is_ok() {
            log::set_max_level(log::LevelFilter::Debug);
        }
    });
}

/// The path + query of `url`, with any userinfo stripped (a URL with
/// embedded credentials must never reach a debug line). A URL with
/// no path yields `/`. The scheme and authority are dropped: the
/// host already appears in the startup config line, and the path is
/// the diagnostic.
pub fn url_path(url: &str) -> String {
    let safe = crate::http::sanitize_url_for_error(url);
    let after_scheme = match safe.find("://") {
        Some(pos) => &safe[pos + 3..],
        None => return safe,
    };
    match after_scheme.find('/') {
        Some(pos) => after_scheme[pos..].to_string(),
        None => "/".to_string(),
    }
}

/// The host of `url`: the authority with any userinfo and any port
/// stripped, brackets stripped from an IPv6 literal. Empty when the
/// URL has no usable host. Only a host is ever printed — never a
/// username, never a value.
pub fn url_host(url: &str) -> &str {
    let Some(after_scheme) = url.split("://").nth(1) else {
        return "";
    };
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    // Strip any userinfo first: an `@` in it must not be mistaken
    // for the host separator.
    let without_userinfo = authority.rsplit('@').next().unwrap_or(authority);
    if let Some(bracketed) = without_userinfo.strip_prefix('[') {
        match bracketed.split_once(']') {
            Some((inside, _)) => inside,
            None => "",
        }
    } else {
        without_userinfo.split(':').next().unwrap_or("")
    }
}

/// True when `name` (an attribute, JSON key, element, or bare
/// `name: value` name, case-insensitive) looks credential-bearing:
/// it contains `user`, `password`, `passwd`, `pwd`, `token`,
/// `secret`, `sessionid`, `apikey`, `authorization`, `accesskey`,
/// or `privatekey` (with `-`, `_`, `.`, and spaces ignored, so
/// `api_key`, `api.key`, `api key`, and `auth-token` all match). The match is
/// `contains`, not equality: `secret_key`, `password_hash`, and
/// `token_value` are credentials too. Over-matching only redacts
/// more, which is the safe direction. Bare `auth` is deliberately
/// not a stem: it would redact every `<author>` name. The match is
/// on the name only; the value is never inspected.
/// The name with case, separators, and quoting dropped, so
/// `api_key`, `api.key`, `api key`, and `auth-token` all compare the same.
fn normalize_name(name: &str) -> String {
    name.to_ascii_lowercase()
        .chars()
        .filter(|c| *c != '-' && *c != '_' && *c != ' ' && *c != '"' && *c != '\\' && *c != '.')
        .collect()
}

/// True when `name` is the `Authorization` header (any case or
/// separator spelling): its value is `scheme credentials`, so the
/// whole rest of the line is the credential, not just the first
/// whitespace-separated token.
fn is_authorization_name(name: &str) -> bool {
    normalize_name(name).contains("authorization")
}

fn is_credential_name(name: &str) -> bool {
    let normalized = normalize_name(name);
    [
        "user",
        "password",
        "passwd",
        "pwd",
        "token",
        "secret",
        "sessionid",
        "apikey",
        "authorization",
        "accesskey",
        "privatekey",
    ]
    .iter()
    .any(|candidate| normalized.contains(candidate))
}

/// The marker a credential value is replaced with. The name, the
/// quote style, and the surrounding syntax are kept; only the value
/// changes.
const REDACTED: &str = "[redacted]";

/// Read a run of name characters (`[A-Za-z0-9_-]`) starting at
/// `start`. Returns the index just past the run and the name, or
/// `None` when there is no name character at `start`.
fn read_name(body: &str, start: usize) -> Option<(usize, &str)> {
    let bytes = body.as_bytes();
    let mut end = start;
    while end < bytes.len()
        && (bytes[end].is_ascii_alphanumeric()
            || bytes[end] == b'_'
            || bytes[end] == b'-'
            || bytes[end] == b'.')
    {
        end += 1;
    }
    if end == start {
        return None;
    }
    Some((end, &body[start..end]))
}

/// The index of the closing `quote` for the quoted string opening
/// at `start`, skipping `\`-escaped characters. `None` when the
/// quote never closes.
fn scan_quoted(body: &str, start: usize, quote: u8) -> Option<usize> {
    let bytes = body.as_bytes();
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == quote {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// The value following a credential name at `pos` (just past the
/// name, or just past its closing quote): skip whitespace, expect
/// `=` or `:`, skip whitespace again, then take the value. A
/// quoted value runs to its closing quote (escapes skipped); an
/// unquoted value runs to the next delimiter — or, when `line_value`
/// is true (the `Authorization` header, whose value is `scheme
/// credentials`), to the end of the line. Returns the value's
/// byte range, with the end at the closing quote for a quoted
/// value. `None` when there is no assignment here, when the value
/// is a structured `[`/`{` value, or when it is empty — those copy
/// through unchanged.
fn assignment_value(body: &str, pos: usize, line_value: bool) -> Option<(usize, usize)> {
    let bytes = body.as_bytes();
    let mut i = pos;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() || (bytes[i] != b'=' && bytes[i] != b':') {
        return None;
    }
    i += 1;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    if bytes[i] == b'"' || bytes[i] == b'\'' {
        let close = scan_quoted(body, i, bytes[i])?;
        if close == i + 1 {
            return None;
        }
        return Some((i + 1, close));
    }
    if bytes[i] == b'[' || bytes[i] == b'{' {
        return None;
    }
    let start = i;
    // An Authorization value is `scheme credentials`: it runs to
    // the end of the line. Any other unquoted value ends at the
    // first whitespace.
    while i < bytes.len()
        && !(if line_value {
            bytes[i] == b'\n' || bytes[i] == b'\r'
        } else {
            bytes[i].is_ascii_whitespace()
        })
        && !matches!(
            bytes[i],
            b',' | b'<' | b'>' | b'}' | b']' | b'"' | b'\'' | b';' | b'&'
        )
    {
        i += 1;
    }
    if i == start {
        return None;
    }
    Some((start, i))
}

/// The text content of the credential element opening at `body[i]`
/// (`<name>text</name>`, with an optional namespace prefix):
/// returns the text's byte range, ending at the next `<` (or the
/// end of the body). `None` when the tag at `i` is not a credential
/// element — a non-credential name, a closing tag, or a tag
/// carrying attributes.
fn credential_element_text(body: &str, i: usize) -> Option<(usize, usize)> {
    let bytes = body.as_bytes();
    let (mut pos, mut name) = read_name(body, i + 1)?;
    if pos < bytes.len() && bytes[pos] == b':' {
        let (local_end, local) = read_name(body, pos + 1)?;
        name = local;
        pos = local_end;
    }
    if !is_credential_name(name) {
        return None;
    }
    // Whitespace may sit between the name and `>`; attributes may
    // not — a tag carrying attributes is left alone by design.
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    if pos >= bytes.len() || bytes[pos] != b'>' {
        return None;
    }
    let text_start = pos + 1;
    let mut text_end = text_start;
    while text_end < bytes.len() && bytes[text_end] != b'<' {
        text_end += 1;
    }
    Some((text_start, text_end))
}

/// Try to redact a quoted credential name starting at `body[i]`
/// (the opening quote): when the quoted text is a credential name
/// followed by an assignment, return the value's byte range, with
/// the end at the closing quote for a quoted value. `None` when this
/// is not a credential assignment.
fn try_redact_quoted_name(body: &str, i: usize, quote: u8) -> Option<(usize, usize)> {
    let close = scan_quoted(body, i, quote)?;
    let name = &body[i + 1..close];
    if !is_credential_name(name) {
        return None;
    }
    assignment_value(body, close + 1, is_authorization_name(name))
}

/// Redact credential-looking values in `body`, replacing each value
/// with `[redacted]` and keeping the name and quote style. Anything
/// else passes through unchanged.
///
/// This is deliberately a simple name-based scan, not a parser and
/// not a redaction of arbitrary content: OBS exposes the rest of a
/// body by default, and safeguarding non-credential content is the
/// server's job, not geectl's. The scan covers the shapes a
/// credential arrives in with a name next to it — an XML attribute
/// (`name="value"`), a JSON pair (`"name": "value"`), a bare
/// assignment (`name: value`, `name=value`, quoted or unquoted),
/// and element text (`<name>value</name>`, including a namespace
/// prefix). A credential value with no name next to it — a bare
/// array item, a structured object value, a CDATA section, a
/// nested element, an element tag carrying attributes — is left
/// alone: that is the design, not a gap to chase. Malformed input
/// is left alone too: an unterminated quote or an empty value
/// copies through verbatim, and the scan never fails.
pub fn redact_body(body: &str) -> String {
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if let Some((text_start, text_end)) = credential_element_text(body, i) {
                out.push_str(&body[i..text_start]);
                if text_end > text_start {
                    out.push_str(REDACTED);
                }
                i = text_end;
                continue;
            }
            out.push('<');
            i += 1;
            continue;
        }
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            if let Some((value_start, value_end)) = try_redact_quoted_name(body, i, bytes[i]) {
                out.push_str(&body[i..value_start]);
                out.push_str(REDACTED);
                i = value_end;
                continue;
            }
            let len = body[i..].chars().next().unwrap().len_utf8();
            out.push_str(&body[i..i + len]);
            i += len;
            continue;
        }
        if bytes[i].is_ascii_alphabetic() {
            if let Some((after_name, name)) = read_name(body, i) {
                if is_credential_name(name) {
                    if let Some((value_start, value_end)) =
                        assignment_value(body, after_name, is_authorization_name(name))
                    {
                        out.push_str(&body[i..value_start]);
                        out.push_str(REDACTED);
                        i = value_end;
                        continue;
                    }
                }
            }
        }
        let len = body[i..].chars().next().unwrap().len_utf8();
        out.push_str(&body[i..i + len]);
        i += len;
    }
    out
}

fn is_binary_body(body: &str) -> bool {
    body.bytes()
        .any(|b| b == 0 || b == 0x7f || (b < 0x20 && b != b'\n' && b != b'\r' && b != b'\t'))
}

/// The body preview printed behind `--debug-body`: a binary body is
/// never dumped (`<binary body, N bytes>`); a text body has its
/// credential values redacted ([`redact_body`]) and is truncated
/// at [`DEBUG_BODY_CAP`] bytes, at
/// a character boundary, with a marker naming the original body's
/// total length (the redacted form can be shorter, so the marker
/// never names the redacted length).
pub fn format_body_preview(body: &str) -> String {
    if is_binary_body(body) {
        return format!("<binary body, {} bytes>", body.len());
    }
    let redacted = redact_body(body);
    // Terminal control and bidirectional / format characters are
    // stripped: the preview is printed raw to the terminal, and a
    // text body may carry a raw C1 control character (the binary
    // check only catches C0 bytes). Newlines survive: they are
    // the preview's own line structure.
    let redacted = strip_terminal_controls(&redacted);
    if redacted.len() <= DEBUG_BODY_CAP {
        return redacted;
    }
    let mut end = DEBUG_BODY_CAP;
    while !redacted.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}... [truncated, {} bytes total]",
        &redacted[..end],
        body.len()
    )
}

/// Strip the characters that must not reach the terminal from a
/// string: C0/C1 control characters (except newline) and the
/// bidirectional / format characters that reorder displayed text
/// (U+061C, U+200E-U+200F, U+202A-U+202E, U+2066-U+2069, U+FEFF). Stored
/// `_service` files and error bodies are untrusted input, and both
/// are printed raw to the terminal.
pub(crate) fn strip_terminal_controls(text: &str) -> String {
    text.chars()
        .filter(|c| {
            *c == '\n'
                || (!c.is_control()
                    && !matches!(
                        *c,
                        '\u{061c}'
                            | '\u{200e}'..='\u{200f}'
                            | '\u{202a}'..='\u{202e}'
                            | '\u{2066}'..='\u{2069}'
                            | '\u{feff}'
                    ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn captured_lines(lines: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        lines.lock().unwrap().clone()
    }

    #[test]
    fn disabled_sink_emits_nothing() {
        let (debug, lines) = Debug::captured(false, false);
        debug.emit("http: GET /test -> 200 in 1ms (attempt 1/4)");
        assert!(captured_lines(&lines).is_empty());
    }

    #[test]
    fn enabled_sink_records_preformed_lines() {
        let (debug, lines) = Debug::captured(true, false);
        debug.emit("http: GET /test -> 200 in 1ms (attempt 1/4)");
        assert_eq!(
            captured_lines(&lines),
            vec!["http: GET /test -> 200 in 1ms (attempt 1/4)".to_string()]
        );
    }

    #[test]
    fn bodies_flag_is_off_without_the_channel() {
        // `--debug-body` implies `--debug` at the CLI layer; the sink
        // itself never prints bodies when the channel is off, even
        // with the bodies flag set.
        let debug = Debug::new(false, true);
        assert!(!debug.bodies_enabled());
        let debug = Debug::new(true, false);
        assert!(!debug.bodies_enabled());
        let debug = Debug::new(true, true);
        assert!(debug.bodies_enabled());
    }

    #[test]
    fn install_is_idempotent() {
        install();
        install();
    }

    #[test]
    fn stderr_logger_rejects_foreign_targets() {
        use log::Log;
        // ureq emits `debug!` records on every request; the backend
        // must drop them, or the first real HTTP command would print
        // ureq's raw lines on every run. A record the logger reports
        // as disabled produces no output, so the assertion is on
        // `enabled()` — and `log()` with the same record is exercised
        // too, to prove the guard holds on that path as well.
        let foreign = log::MetadataBuilder::new()
            .target("ureq")
            .level(log::Level::Debug)
            .build();
        assert!(!StderrLogger.enabled(&foreign));
        let record = log::Record::builder()
            .metadata(foreign)
            .args(format_args!("ureq line"))
            .build();
        LOGGER.log(&record);
        let own = log::MetadataBuilder::new()
            .target(DEBUG_TARGET)
            .level(log::Level::Debug)
            .build();
        assert!(StderrLogger.enabled(&own));
        // Above Debug is out of the channel too.
        let trace = log::MetadataBuilder::new()
            .target(DEBUG_TARGET)
            .level(log::Level::Trace)
            .build();
        assert!(!StderrLogger.enabled(&trace));
    }

    #[test]
    fn redact_body_redacts_credential_attributes() {
        let body =
            r#"<request><auth password="hunter2-fake" token='tok-fake' other="keep"/></request>"#;
        let redacted = redact_body(body);
        assert!(!redacted.contains("hunter2-fake"));
        assert!(!redacted.contains("tok-fake"));
        assert!(redacted.contains(r#"password="[redacted]""#));
        assert!(redacted.contains(r#"token='[redacted]'"#));
        assert!(redacted.contains(r#"other="keep""#));
    }

    #[test]
    fn redact_body_matches_credential_names_case_insensitively() {
        let body = r#"{"Password": "hunter2-fake", "TOKEN": "tok-fake", "Secret": "sec-fake"}"#;
        let redacted = redact_body(body);
        assert!(!redacted.contains("hunter2-fake"));
        assert!(!redacted.contains("tok-fake"));
        assert!(!redacted.contains("sec-fake"));
        assert!(redacted.contains(r#""Password": "[redacted]""#));
    }

    #[test]
    fn redact_body_leaves_non_credential_attributes_alone() {
        let body = r#"<package name="geectl" project="openSUSE:Factory" />"#;
        assert_eq!(redact_body(body), body);
    }

    #[test]
    fn redact_body_redacts_json_pairs() {
        let body = r#"{"password": "hunter2-fake", "name": "keep"}"#;
        let redacted = redact_body(body);
        assert!(!redacted.contains("hunter2-fake"));
        assert!(redacted.contains(r#""name": "keep""#));
    }

    #[test]
    fn redact_body_redacts_credential_names_with_suffixes() {
        let body = r#"{"secret_key": "sec-fake", "password_hash": "hash-fake", "token_value": "tok-fake"}"#;
        let redacted = redact_body(body);
        assert!(!redacted.contains("sec-fake"));
        assert!(!redacted.contains("hash-fake"));
        assert!(!redacted.contains("tok-fake"));
    }

    #[test]
    fn redact_body_redacts_element_text() {
        let body = "<auth><password>hunter2-fake</password><user>tom</user></auth>";
        let redacted = redact_body(body);
        assert!(!redacted.contains("hunter2-fake"));
        assert!(!redacted.contains(">tom<"));
        assert!(redacted.contains("<password>[redacted]</password>"));
        assert!(redacted.contains("<user>[redacted]</user>"));
    }

    #[test]
    fn redact_body_redacts_namespaced_element_text() {
        let body = "<ns:password>hunter2-fake</ns:password>";
        assert_eq!(redact_body(body), "<ns:password>[redacted]</ns:password>");
    }

    #[test]
    fn redact_body_redacts_yaml_and_unquoted_forms() {
        let body = "password: hunter2-fake
token=hunter2-fake2
name: keep";
        let redacted = redact_body(body);
        assert!(!redacted.contains("hunter2-fake"));
        assert!(redacted.contains("name: keep"));
    }

    #[test]
    fn redact_body_redacts_authorization_values() {
        let body = "Authorization: Basic dXNlcjpwYXNzZmFrZQ==
Host: api.opensuse.org";
        let redacted = redact_body(body);
        assert!(!redacted.contains("dXNlcjpwYXNzZmFrZQ=="));
        assert!(redacted.contains("Authorization: [redacted]"));
        assert!(redacted.contains("Host: api.opensuse.org"));
    }

    #[test]
    fn redact_body_redacts_username_values() {
        let body = r#"{"username": "tom-fake", "user_name": "tom-fake2"}"#;
        let redacted = redact_body(body);
        assert!(!redacted.contains("tom-fake"));
        assert!(redacted.contains(r#""username": "[redacted]""#));
    }

    #[test]
    fn redact_body_redacts_api_key_and_session_id_forms() {
        let body = r#"{"api key": "key-fake", "api_key": "key-fake2", "session id": "sess-fake"}"#;
        let redacted = redact_body(body);
        assert!(!redacted.contains("key-fake"));
        assert!(!redacted.contains("sess-fake"));
    }

    #[test]
    fn redact_body_redacts_pwd_and_sessionid_stems_not_pass() {
        let body = r#"{"pwd": "pw-fake", "sessionid": "sess-fake", "pass": "keep"}"#;
        let redacted = redact_body(body);
        assert!(!redacted.contains("pw-fake"));
        assert!(!redacted.contains("sess-fake"));
        assert!(redacted.contains(r#""pass": "keep""#));
    }

    #[test]
    fn redact_body_redacts_access_key_and_private_key_values() {
        let body = r#"{"access-key": "ak-fake", "private-key": "pk-fake"}"#;
        let redacted = redact_body(body);
        assert!(!redacted.contains("ak-fake"));
        assert!(!redacted.contains("pk-fake"));
    }

    #[test]
    fn redact_body_leaves_author_names_alone() {
        let body = "<author>Tom Chvatal</author>";
        assert_eq!(redact_body(body), body);
    }

    #[test]
    fn redact_body_leaves_bare_prose_alone() {
        let body = "the password policy is strict and the token system is new";
        assert_eq!(redact_body(body), body);
    }

    #[test]
    fn redact_body_redacts_credential_values_across_newlines() {
        let body = "{\"password\": \"hunter2-\nfake\"}";
        let redacted = redact_body(body);
        assert!(!redacted.contains("hunter2-"));
        assert!(redacted.contains(r#""password": "[redacted]""#));
    }

    #[test]
    fn redact_body_redacts_bare_value_split_at_angle_bracket() {
        let body = "password=hunter2-fake<br/>next: keep";
        let redacted = redact_body(body);
        assert!(!redacted.contains("hunter2-fake"));
        assert!(redacted.contains("next: keep"));
    }

    #[test]
    fn redact_body_redacts_unquoted_json_scalars() {
        let body = r#"{"token": tok-fake, "name": "keep"}"#;
        let redacted = redact_body(body);
        assert!(!redacted.contains("tok-fake"));
        assert!(redacted.contains(r#""name": "keep""#));
    }

    #[test]
    fn redact_body_redacts_dot_separated_credential_names() {
        let body = r#"{"api.key": "key-fake", "session.id": "sess-fake"}"#;
        let redacted = redact_body(body);
        assert!(!redacted.contains("key-fake"));
        assert!(!redacted.contains("sess-fake"));
        assert_eq!(redact_body("api.key=key-fake"), "api.key=[redacted]");
        assert_eq!(
            redact_body("<session.id>sess-fake</session.id>"),
            "<session.id>[redacted]</session.id>"
        );
    }

    #[test]
    fn redact_body_redacts_single_quoted_keys() {
        let body = "{'password': 'hunter2-fake', 'name': 'keep'}";
        let redacted = redact_body(body);
        assert!(!redacted.contains("hunter2-fake"));
        assert!(redacted.contains("'name': 'keep'"));
    }

    #[test]
    fn redact_body_redacts_element_text_with_space_before_close() {
        assert_eq!(
            redact_body("<password >hunter2-fake</password>"),
            "<password >[redacted]</password>"
        );
    }

    #[test]
    fn redact_body_leaves_bare_array_items_alone() {
        // Deliberate: a bare value with no name next to it is not a
        // credential the scan can see, so it passes through.
        let body = r#"{"token": ["tok-fake"], "name": ["keep"]}"#;
        assert_eq!(redact_body(body), body);
    }

    #[test]
    fn redact_body_leaves_cdata_sections_alone() {
        // Deliberate: a CDATA section has no name next to its
        // content, so it passes through.
        let body = "<password><![CDATA[hunter2-fake]]></password>";
        assert_eq!(redact_body(body), body);
    }

    #[test]
    fn redact_body_leaves_unterminated_quotes_alone() {
        let body = r#"password="hunter2-fake"#;
        assert_eq!(redact_body(body), body);
    }

    #[test]
    fn redact_body_leaves_empty_credential_values_alone() {
        let body = r#"{"password": "", "token": "tok-fake"}"#;
        let redacted = redact_body(body);
        assert!(redacted.contains(r#""password": """#));
        assert!(!redacted.contains("tok-fake"));
    }

    #[test]
    fn body_preview_passes_short_bodies_through() {
        assert_eq!(format_body_preview("hello"), "hello");
    }

    #[test]
    fn body_preview_truncates_long_bodies_with_a_marker() {
        let body = "x".repeat(DEBUG_BODY_CAP + 100);
        let preview = format_body_preview(&body);
        assert!(preview.starts_with(&"x".repeat(DEBUG_BODY_CAP)));
        assert!(preview.contains(&format!(
            "[truncated, {} bytes total]",
            DEBUG_BODY_CAP + 100
        )));
    }

    #[test]
    fn body_preview_truncates_at_a_character_boundary() {
        // A multi-byte character straddling the cap must not split:
        // the preview stays valid UTF-8 and the marker still names
        // the total. The leading `a` puts the cap inside a
        // two-byte character (a boundary at every odd byte).
        let body = format!("a{}", "é".repeat(DEBUG_BODY_CAP));
        let preview = format_body_preview(&body);
        assert!(preview.contains("[truncated, 4097 bytes total]"));
        // The cap (2048) falls inside a two-byte character, so the
        // preview backs off to 2047 bytes: `a` plus 1023 whole `é`s.
        assert_eq!(preview.matches('é').count(), 1023);
    }

    #[test]
    fn strip_terminal_controls_removes_escapes_and_bidi_marks() {
        // The filter is a pure function: control characters
        // (except newline) and bidirectional / format characters
        // are removed, everything else survives.
        assert_eq!(strip_terminal_controls("a\u{1b}[2Jb"), "a[2Jb");
        assert_eq!(strip_terminal_controls("a\u{7}b"), "ab");
        assert_eq!(strip_terminal_controls("a\rb"), "ab");
        assert_eq!(strip_terminal_controls("a\nb"), "a\nb");
        assert_eq!(strip_terminal_controls("a\u{061c}b"), "ab");
        assert_eq!(strip_terminal_controls("a\u{200e}b"), "ab");
        assert_eq!(strip_terminal_controls("a\u{200f}b"), "ab");
        assert_eq!(strip_terminal_controls("a\u{202a}b"), "ab");
        assert_eq!(strip_terminal_controls("a\u{202e}b"), "ab");
        assert_eq!(strip_terminal_controls("a\u{2066}b"), "ab");
        assert_eq!(strip_terminal_controls("a\u{2069}b"), "ab");
        assert_eq!(strip_terminal_controls("a\u{feff}b"), "ab");
        assert_eq!(strip_terminal_controls("a\tb"), "ab");
    }

    #[test]
    fn body_preview_never_dumps_binary_bodies() {
        let body = "ok\0binary";
        assert_eq!(format_body_preview(body), "<binary body, 9 bytes>");
    }

    #[test]
    fn body_preview_strips_terminal_controls() {
        // A raw C1 control character is not caught by the binary
        // check (it only looks at C0 bytes), so the preview must
        // strip it; a bidirectional mark is stripped too. The
        // preview is printed raw to the terminal.
        let preview = format_body_preview("ok\u{9b}te\u{202e}xt");
        assert!(!preview.chars().any(|c| c.is_control() && c != '\n'));
        assert!(!preview.contains('\u{202e}'));
        assert!(preview.contains("text"));
    }

    #[test]
    fn body_preview_truncation_marker_names_the_original_length() {
        // The credential redacts shorter than the original, so the
        // marker must name the original body, not the redacted one.
        let body = format!(
            r#"<auth password="hunter2-fake" />{}"#,
            "x".repeat(DEBUG_BODY_CAP)
        );
        let preview = format_body_preview(&body);
        assert!(preview.contains(&format!("[truncated, {} bytes total]", body.len())));
    }

    #[test]
    fn url_path_strips_scheme_authority_and_userinfo() {
        assert_eq!(
            url_path("https://api.opensuse.org/source/foo/bar?rev=1"),
            "/source/foo/bar?rev=1"
        );
        assert_eq!(
            url_path("https://user:secret@example.com/source/foo"),
            "/source/foo"
        );
        assert_eq!(url_path("https://api.opensuse.org"), "/");
    }

    #[test]
    fn url_host_strips_userinfo_port_and_brackets() {
        assert_eq!(
            url_host("https://api.opensuse.org/source/foo"),
            "api.opensuse.org"
        );
        assert_eq!(
            url_host("https://user:secret@example.com:8443/source/foo"),
            "example.com"
        );
        assert_eq!(url_host("http://[::1]:8080/test"), "::1");
        assert_eq!(url_host("not-a-url"), "");
    }
}
