//! Parsing of `.changes` files: entries, headers, timestamps.
//!
//! The format is the one `changelog` writes: a 67-dash separator, a
//! `Day Mon DD HH:MM:SS UTC YYYY - Name <email>` header, a blank line,
//! then `-` / `*` bullets with indented continuations. The parser is
//! lenient about what it accepts — validation decides what is wrong —
//! but strict about what it reports: a header that does not parse is
//! `None`, never a guessed date. No date library is involved: the
//! civil-date conversion is the standard days-from-civil algorithm.

/// The separator length `changelog` writes (67 dashes).
pub const SEPARATOR_LEN: usize = 67;

/// A parsed entry header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub weekday: String,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub year: i32,
    /// The author part after ` - `, verbatim (`Name <email>` or
    /// whatever the file carries).
    pub author: String,
    /// Seconds since the Unix epoch (UTC).
    pub timestamp: i64,
}

impl Header {
    /// The author is the full `Name <email>` form: a non-empty name
    /// and an `<email>` with an `@` inside. A bare email fails this.
    pub fn author_is_full_form(&self) -> bool {
        let Some((name, rest)) = self.author.split_once('<') else {
            return false;
        };
        if name.trim().is_empty() || !rest.ends_with('>') {
            return false;
        }
        let email = &rest[..rest.len() - 1];
        !email.is_empty() && email.contains('@') && !email.contains(char::is_whitespace)
    }

    /// Render the header line in canonical form (English names,
    /// space-padded day, exactly as `LC_ALL=C date -u` writes it).
    /// `None` when the month is out of range: a hand-built header
    /// with `month: 0` or `month: 13` fails here, never panics.
    pub fn render(&self) -> Option<String> {
        let month = month_name(self.month)?;
        Some(format!(
            "{} {} {:2} {:02}:{:02}:{:02} UTC {} - {}",
            self.weekday,
            month,
            self.day,
            self.hour,
            self.minute,
            self.second,
            self.year,
            self.author
        ))
    }
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The month name, `None` for an out-of-range month: a `Header`
/// built by hand with `month: 0` or `month: 13` must not panic its
/// renderer.
fn month_name(month: u32) -> Option<&'static str> {
    month
        .checked_sub(1)
        .and_then(|index| MONTHS.get(index as usize).copied())
}

fn month_number(name: &str) -> Option<u32> {
    MONTHS.iter().position(|m| *m == name).map(|i| i as u32 + 1)
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Days from the civil date to the Unix epoch (Howard Hinnant's
/// algorithm). The inverse is [`civil_from_days`].
pub fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp as i64 + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// The civil date of `days` since the Unix epoch.
pub fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { y + 1 } else { y } as i32, month, day)
}

/// Weekday of `days` since the epoch, 0 = Monday. 1970-01-01 was a
/// Thursday.
pub fn weekday_from_days(days: i64) -> usize {
    (((days + 3) % 7 + 7) % 7) as usize
}

/// The weekday name of `days` since the epoch.
pub fn weekday_name(days: i64) -> &'static str {
    WEEKDAYS[weekday_from_days(days)]
}

/// Parse a header line. `None` when the line does not carry a
/// `Day Mon DD HH:MM:SS UTC YYYY - author` shape with a valid calendar
/// date — including when the ` - ` separator before the author is
/// missing (the source validator reads the date by accumulating tokens
/// until `-` or `@`, so a missing separator swallows the name into the
/// date and fails there too).
pub fn parse_header(line: &str) -> Option<Header> {
    let (date_part, author) = line.split_once(" - ")?;
    let author = author.trim().to_string();
    if author.is_empty() {
        return None;
    }
    let mut parts = date_part.split_whitespace();
    let weekday = parts.next()?;
    let month_name = parts.next()?;
    let day: u32 = parts.next()?.parse().ok()?;
    let time = parts.next()?;
    let zone = parts.next()?;
    let year: i32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || zone != "UTC" || !WEEKDAYS.contains(&weekday) {
        return None;
    }
    let month = month_number(month_name)?;
    let mut time_parts = time.split(':');
    let hour: u32 = time_parts.next()?.parse().ok()?;
    let minute: u32 = time_parts.next()?.parse().ok()?;
    let second: u32 = time_parts.next()?.parse().ok()?;
    if time_parts.next().is_some() {
        return None;
    }
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
        || !(1000..=9999).contains(&year)
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(Header {
        weekday: weekday.to_string(),
        month,
        day,
        hour,
        minute,
        second,
        year,
        author,
        timestamp: days * 86400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64,
    })
}

/// A line of three or more dashes (trailing whitespace allowed)
/// starts a new entry.
pub fn is_separator(line: &str) -> bool {
    let trimmed = line.trim_end();
    trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-')
}

/// One entry: its separator, its header, and its body lines.
#[derive(Debug, Clone)]
pub struct Entry {
    /// 1-based line number of the separator.
    pub separator_line: usize,
    /// 1-based line number of the header line.
    pub header_line: usize,
    /// The parsed header, or `None` when the header line does not
    /// parse (validation reports it; the entry still exists).
    pub header: Option<Header>,
    /// The header line verbatim.
    pub header_raw: String,
    /// Body lines with 1-based line numbers, blank lines included,
    /// from the line after the header to the line before the next
    /// separator (or EOF).
    pub body: Vec<(usize, String)>,
    /// 1-based line number of the last body line.
    pub end_line: usize,
}

impl Entry {
    /// Non-blank body lines.
    pub fn body_nonblank(&self) -> Vec<(usize, &str)> {
        self.body
            .iter()
            .filter(|(_, line)| !line.trim().is_empty())
            .map(|(n, line)| (*n, line.as_str()))
            .collect()
    }

    /// Bullets joined over their continuation lines: (first line
    /// number, joined text, indent). A line starting with `- `, `* `
    /// or `+ ` starts a bullet; an indented line continues the
    /// previous one; any other non-blank line starts a bullet of its
    /// own so validation can flag it.
    pub fn bullets(&self) -> Vec<(usize, String, usize)> {
        let mut bullets: Vec<(usize, String, usize)> = Vec::new();
        for (n, line) in self.body_nonblank() {
            let indent = line.len() - line.trim_start().len();
            let trimmed = line.trim_start();
            if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
                bullets.push((n, trimmed.to_string(), indent));
            } else if indent > 0 || line.starts_with('\t') {
                match bullets.last_mut() {
                    Some(last) => {
                        last.1.push(' ');
                        last.1.push_str(line.trim());
                    }
                    // No bullet yet: a continuation on its own is a
                    // bullet of its own, as in the else arm below.
                    None => bullets.push((n, trimmed.to_string(), indent)),
                }
            } else {
                bullets.push((n, trimmed.to_string(), indent));
            }
        }
        bullets
    }

    /// Top-level `-` bullets with their sub-bullets joined, for the
    /// checks that read a claim with its ids (V12): (first line
    /// number, joined text).
    pub fn groups(&self) -> Vec<(usize, String)> {
        let mut groups: Vec<(usize, String)> = Vec::new();
        for (n, line) in self.body_nonblank() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("- ") || groups.is_empty() {
                groups.push((n, trimmed.to_string()));
            } else {
                match groups.last_mut() {
                    Some(last) => {
                        last.1.push(' ');
                        last.1.push_str(line.trim());
                    }
                    // Unreachable: the empty case pushes above. Kept
                    // total so the group is never dropped silently.
                    None => groups.push((n, trimmed.to_string())),
                }
            }
        }
        groups
    }

    /// The entry's raw text (separator through the last body line),
    /// for the byte-compare of older entries against the target.
    pub fn raw(&self, lines: &[String]) -> String {
        let start = self.separator_line - 1;
        let end = self.end_line;
        lines[start..end].join("\n")
    }
}

/// A parsed `.changes` file.
#[derive(Debug, Clone, Default)]
pub struct ChangesFile {
    pub lines: Vec<String>,
    pub entries: Vec<Entry>,
    /// True when line 1 is a separator.
    pub starts_with_separator: bool,
}

/// Parse a `.changes` file. A file with no separators has no entries;
/// the whole content is then a preamble validation flags.
pub fn parse_changes(text: &str) -> ChangesFile {
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    let seps: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| is_separator(line))
        .map(|(i, _)| i)
        .collect();
    let starts_with_separator = seps.first() == Some(&0);
    let mut entries = Vec::new();
    for (k, &sep) in seps.iter().enumerate() {
        let header_idx = sep + 1;
        let header_raw = lines.get(header_idx).cloned().unwrap_or_default();
        let header = parse_header(&header_raw);
        let body_start = header_idx + 1;
        let body_end = seps.get(k + 1).copied().unwrap_or(lines.len());
        let body: Vec<(usize, String)> = lines[body_start..body_end]
            .iter()
            .enumerate()
            .map(|(off, line)| (body_start + off + 1, line.clone()))
            .collect();
        let end_line = body_end;
        entries.push(Entry {
            separator_line: sep + 1,
            header_line: header_idx + 1,
            header,
            header_raw,
            body,
            end_line,
        });
    }
    ChangesFile {
        lines,
        entries,
        starts_with_separator,
    }
}

/// The separator line: exactly 67 dashes.
pub fn separator_line() -> String {
    "-".repeat(SEPARATOR_LEN)
}

/// Guess the legacy encoding of `.changes` bytes, or `None` when
/// the input is valid UTF-8. The guess is computed on the whole
/// file; each line is then decoded on its own, so a wrong guess
/// cannot corrupt a line that is valid UTF-8.
fn guess_legacy_encoding(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    if std::str::from_utf8(bytes).is_ok() {
        return None;
    }
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    Some(detector.guess(None, chardetng::Utf8Detection::Allow))
}

/// Decode `bytes` with one legacy encoding. Bytes the encoding
/// cannot represent arrive as U+FFFD in the decoded text itself;
/// they are kept, not re-decoded as UTF-8.
fn decode_with(bytes: &[u8], encoding: &'static encoding_rs::Encoding) -> String {
    let (decoded, _, _) = encoding.decode(bytes);
    decoded.into_owned()
}

/// Decode bytes that mix valid UTF-8 with legacy-encoded bytes.
/// Fully valid UTF-8 is never re-decoded. Otherwise a line the
/// guessed encoding decodes cleanly is transcoded whole: that is
/// the only correct reading when the encoding is multi-byte
/// (Shift_JIS, EUC-JP, GBK, Big5), since its characters span bytes
/// that byte-at-a-time slicing would destroy — and a clean decode
/// is evidence the line is wholly in that encoding. The exception
/// is a line that itself mixes encodings and happens to decode
/// cleanly: it is transcoded whole and loses its UTF-8 half.
/// A single-byte encoding never reports an error, so a clean decode
/// vouches for nothing there: the whole-line reading is used only
/// when the line holds no valid multi-byte UTF-8. Any other line keeps its valid
/// spans verbatim, and each invalid run is transcoded with the
/// guessed encoding. A wrong-but-decodable guess keeps that
/// encoding's interpretation of the invalid run only; the valid
/// spans are never sacrificed to the guess.
fn decode_mixed(bytes: &[u8], guessed: &'static encoding_rs::Encoding) -> String {
    // Fully valid UTF-8 is never re-decoded: its bytes can decode
    // cleanly as half-width katakana in Shift_JIS.
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }
    let (decoded, _, had_errors) = guessed.decode(bytes);
    if !had_errors && (guessed_can_error(guessed) || !contains_valid_multibyte_utf8(bytes)) {
        return decoded.into_owned();
    }
    let mut out = String::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                out.push_str(text);
                break;
            }
            Err(err) => {
                let valid_up_to = err.valid_up_to();
                // The prefix is valid UTF-8 by construction, so
                // this is verbatim.
                out.push_str(&String::from_utf8_lossy(&rest[..valid_up_to]));
                let skip = match err.error_len() {
                    Some(len) => len,
                    None => rest.len() - valid_up_to,
                };
                out.push_str(&decode_with(
                    &rest[valid_up_to..valid_up_to + skip],
                    guessed,
                ));
                rest = &rest[valid_up_to + skip..];
            }
        }
    }
    out
}

/// Whether the guessed encoding can report a decode error at
/// all. A single-byte encoding maps every byte, so a clean decode
/// vouches for nothing; a multi-byte one cannot represent a lone
/// lead byte. 0x82 is such a byte in every multi-byte encoding
/// chardetng guesses (UTF-8, Shift_JIS, EUC-JP, GBK, Big5,
/// UTF-16) and a defined character in every single-byte one.
/// (0x80 does not work as the probe: Shift_JIS and GBK decode it
/// as U+0080.)
fn guessed_can_error(encoding: &'static encoding_rs::Encoding) -> bool {
    encoding.decode(b"\x82").2
}

/// Whether `bytes` hold a valid multi-byte UTF-8 sequence
/// anywhere. A byte >= 0x80 inside valid UTF-8 belongs to a
/// multi-byte character; ASCII-only and invalid bytes do not count.
fn contains_valid_multibyte_utf8(bytes: &[u8]) -> bool {
    let mut rest = bytes;
    loop {
        match std::str::from_utf8(rest) {
            Ok(_) => return rest.iter().any(|&byte| byte >= 0x80),
            Err(err) => {
                if rest[..err.valid_up_to()].iter().any(|&byte| byte >= 0x80) {
                    return true;
                }
                match err.error_len() {
                    Some(len) => rest = &rest[err.valid_up_to() + len..],
                    // A truncated sequence at the end: no complete
                    // multi-byte character here.
                    None => return false,
                }
            }
        }
    }
}

/// Normalize `.changes` content for writing: transcode to UTF-8 and
/// strip trailing whitespace on every line, preserved entries
/// included. Decoding is per line, so a valid UTF-8 line is never
/// rewritten by the legacy-encoding guess. A trailing line that
/// trims to empty is dropped when the input has no trailing
/// newline, and the trailing-newline state of the input is kept.
pub fn normalize_changes_content(bytes: &[u8]) -> String {
    let guessed = guess_legacy_encoding(bytes);
    let trailing_newline = bytes.ends_with(b"\n");
    let mut segments: Vec<&[u8]> = bytes.split(|&byte| byte == b'\n').collect();
    if trailing_newline {
        segments.pop();
    }
    let mut lines: Vec<String> = segments
        .iter()
        .map(|segment| {
            let text = match guessed {
                Some(encoding) => decode_mixed(segment, encoding),
                None => String::from_utf8_lossy(segment).into_owned(),
            };
            text.trim_end().to_string()
        })
        .collect();
    if !trailing_newline {
        while lines.last().is_some_and(|line| line.is_empty()) {
            lines.pop();
        }
    }
    let mut out = lines.join("\n");
    if trailing_newline {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_header_accepts_the_canonical_form() {
        let header =
            parse_header("Wed May 27 16:31:48 UTC 2026 - Jane Packager <jane@example.com>")
                .expect("canonical header parses");
        assert_eq!(header.weekday, "Wed");
        assert_eq!((header.year, header.month, header.day), (2026, 5, 27));
        assert_eq!((header.hour, header.minute, header.second), (16, 31, 48));
        assert_eq!(header.author, "Jane Packager <jane@example.com>");
        assert!(header.author_is_full_form());
        assert_eq!(
            header.render().as_deref(),
            Some("Wed May 27 16:31:48 UTC 2026 - Jane Packager <jane@example.com>")
        );
    }

    #[test]
    fn render_returns_none_for_an_out_of_range_month() {
        let header =
            parse_header("Wed May 27 16:31:48 UTC 2026 - Jane Packager <jane@example.com>")
                .expect("canonical header parses");
        let mut bad = header.clone();
        bad.month = 0;
        assert_eq!(bad.render(), None);
        bad.month = 13;
        assert_eq!(bad.render(), None);
    }

    #[test]
    fn parse_header_rejects_a_missing_author_separator() {
        // The validator swallows the name into the date here.
        assert!(parse_header("Sat Apr 14 2007 Tobias Blomberg <t@example.com>").is_none());
    }

    #[test]
    fn parse_header_rejects_impossible_dates_and_zones() {
        assert!(parse_header("Mon Feb 30 10:00:00 UTC 2026 - A <a@b.c>").is_none());
        assert!(parse_header("Mon Feb 29 10:00:00 UTC 2026 - A <a@b.c>").is_none());
        assert!(parse_header("Sat Feb 29 10:00:00 UTC 2020 - A <a@b.c>").is_some());
        assert!(parse_header("Wed May 27 16:31:48 CEST 2026 - A <a@b.c>").is_none());
        assert!(parse_header("Wed May 27 16:31:48 UTC 2026 - A <a@b.c>").is_some());
    }

    #[test]
    fn parse_header_keeps_a_bare_email_for_the_author_form_check() {
        let header =
            parse_header("Wed May 27 16:31:48 UTC 2026 - jane@example.com").expect("date parses");
        assert!(!header.author_is_full_form());
    }

    #[test]
    fn civil_round_trips() {
        for (y, m, d) in [(1970, 1, 1), (2026, 5, 27), (2020, 2, 29), (2000, 2, 29)] {
            let days = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(days), (y, m, d));
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(weekday_from_days(0), 3); // Thursday
        assert_eq!(
            WEEKDAYS[weekday_from_days(days_from_civil(2026, 5, 27))],
            "Wed"
        );
    }

    #[test]
    fn parse_changes_splits_entries() {
        let text = [
            "-------------------------------------------------------------------",
            "Wed May 27 16:31:48 UTC 2026 - Jane <jane@example.com>",
            "",
            "- Update to 2.0",
            "",
            "-------------------------------------------------------------------",
            "Tue May 26 10:00:00 UTC 2026 - Jane <jane@example.com>",
            "",
            "- Initial package",
            "",
        ]
        .join("\n");
        let file = parse_changes(&text);
        assert!(file.starts_with_separator);
        assert_eq!(file.entries.len(), 2);
        assert_eq!(file.entries[0].separator_line, 1);
        assert_eq!(file.entries[0].header_line, 2);
        assert!(file.entries[0].header.is_some());
        assert_eq!(file.entries[1].separator_line, 6);
        assert_eq!(file.entries[0].bullets().len(), 1);
        assert_eq!(file.entries[0].groups().len(), 1);
    }

    #[test]
    fn parse_changes_without_a_separator_has_no_entries() {
        let file = parse_changes("- Update to 2.0\n");
        assert!(!file.starts_with_separator);
        assert!(file.entries.is_empty());
    }

    #[test]
    fn normalize_trims_trailing_whitespace_on_preserved_lines() {
        let bytes = b"-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Jane <jane@example.com>\n\n- Update to 2.0   \n  continuation with a tab\t\n";
        let out = normalize_changes_content(bytes);
        assert_eq!(
            out,
            "-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Jane <jane@example.com>\n\n- Update to 2.0\n  continuation with a tab\n"
        );
    }

    #[test]
    fn normalize_keeps_valid_utf8_verbatim() {
        let text = "-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Tomáš Chvátal <tomas.chvatal@gmail.com>\n\n- Update to 2.0\n";
        assert_eq!(normalize_changes_content(text.as_bytes()), text);
    }

    #[test]
    fn normalize_transcodes_a_latin1_author_name() {
        let mut bytes = b"-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Tom".to_vec();
        bytes.extend_from_slice(b"\xe1s Chv\xe1tal <tomas.chvatal@gmail.com>\n\n- Update to 2.0\n");
        let out = normalize_changes_content(&bytes);
        assert_eq!(
            out,
            "-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Tomás Chvátal <tomas.chvatal@gmail.com>\n\n- Update to 2.0\n"
        );
    }

    #[test]
    fn normalize_preserves_the_trailing_newline_state() {
        assert_eq!(
            normalize_changes_content(b"- Update to 2.0"),
            "- Update to 2.0"
        );
        assert_eq!(
            normalize_changes_content(b"- Update to 2.0\n"),
            "- Update to 2.0\n"
        );
        assert_eq!(normalize_changes_content(b""), "");
        // A final line that trims to empty is dropped when the
        // input has no trailing newline; the newline state is kept.
        assert_eq!(normalize_changes_content(b"a\n   "), "a");
        assert_eq!(normalize_changes_content(b"a\n\n   "), "a");
        // CRLF line endings normalize to LF.
        assert_eq!(normalize_changes_content(b"a\r\nb\r\n"), "a\nb\n");
        assert_eq!(normalize_changes_content(b"a\r\nb"), "a\nb");
    }

    #[test]
    fn decode_with_replaces_undecodable_bytes() {
        // A lone Shift_JIS lead byte decodes with errors in Shift_JIS;
        // the decoder marks it U+FFFD.
        let out = decode_with(b"\x82", encoding_rs::SHIFT_JIS);
        assert_eq!(out, "\u{FFFD}");
    }

    #[test]
    fn normalize_transcodes_and_trims_preserved_entries() {
        // Two preserved entries: the older one carries trailing
        // spaces and a Latin-1 author; both come out clean.
        let mut bytes = b"-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Jane <jane@example.com>\n\n- Update to 2.0\n\n-------------------------------------------------------------------\nTue May 26 10:00:00 UTC 2026 - Tom".to_vec();
        bytes.extend_from_slice(
            b"\xe1s Chv\xe1tal <tomas.chvatal@gmail.com>  \n\n- Initial package   \n",
        );
        let out = normalize_changes_content(&bytes);
        assert_eq!(
            out,
            "-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Jane <jane@example.com>\n\n- Update to 2.0\n\n-------------------------------------------------------------------\nTue May 26 10:00:00 UTC 2026 - Tomás Chvátal <tomas.chvatal@gmail.com>\n\n- Initial package\n"
        );
    }

    #[test]
    fn normalize_mixed_utf8_and_latin1_preserves_the_utf8_half() {
        // A long-lived file: a newer UTF-8 entry above an older
        // Latin-1 entry. The UTF-8 half must come out
        // byte-identical; the Latin-1 byte is transcoded to the
        // right character.
        let mut input = "-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Tomás Chvátal <tomas.chvatal@gmail.com>\n\n- Update to 2.0\n\n-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Andr".as_bytes().to_vec();
        input.extend_from_slice(b"\xe9 Martin <andre@example.com>\n\n- Fix the th\xe9\n");
        let normalized = normalize_changes_content(&input);
        assert_eq!(
            normalized,
            "-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Tomás Chvátal <tomas.chvatal@gmail.com>\n\n- Update to 2.0\n\n-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - André Martin <andre@example.com>\n\n- Fix the thé\n"
        );
    }

    #[test]
    fn normalize_mixed_line_keeps_the_valid_utf8_span() {
        // Valid UTF-8 and a legacy byte on the same line: the valid
        // span survives, the bad byte is transcoded.
        let mut input = b"-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Jane Packager <jane@example.com>\n\n- Th".to_vec();
        input.extend_from_slice("é and Th".as_bytes());
        input.extend_from_slice(b"\xe9\n");
        let normalized = normalize_changes_content(&input);
        assert_eq!(
            normalized,
            "-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Jane Packager <jane@example.com>\n\n- Thé and Thé\n"
        );
    }

    #[test]
    fn normalize_transcodes_a_windows1250_czech_entry() {
        // 0xF8 is ř in windows-1250 and ø in windows-1252, 0xB9
        // is š in windows-1250 and ¹ in windows-1252: this pins
        // which encoding the detector guessed.
        let input = b"-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Jan Novak <jan@example.com>\n\n- Oprava zpracov\xe1n\xed z\xe1znam\xf9 a p\xf8ehled\xf9\n- Zm\xecna nastaven\xed pro v\xb9echny u\xbeivatele\n";
        let normalized = normalize_changes_content(input);
        assert_eq!(
            normalized,
            "-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - Jan Novak <jan@example.com>\n\n- Oprava zpracování záznamů a přehledů\n- Změna nastavení pro všechny uživatele\n"
        );
    }

    #[test]
    fn normalize_transcodes_a_shift_jis_entry() {
        // A multi-byte legacy encoding: each character is two
        // bytes in Shift_JIS. The whole line is transcoded, not
        // sliced apart byte by byte.
        let mut input = b"-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - ".to_vec();
        input.extend_from_slice(b"\x8eR\x93c\x91\xbe\x98Y <yamada@example.com>\n\n- \x83e\x83X\x83g\x82\xcc\x8fC\x90\xb3\n");
        let normalized = normalize_changes_content(&input);
        assert_eq!(
            normalized,
            "-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - 山田太郎 <yamada@example.com>\n\n- テストの修正\n"
        );
    }

    #[test]
    fn normalize_mixed_shift_jis_and_utf8_preserves_the_utf8_line() {
        // A file with Shift_JIS entries and one newer UTF-8 line:
        // the UTF-8 line stays verbatim even though its bytes
        // decode cleanly as half-width katakana in Shift_JIS.
        let mut input = b"-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - ".to_vec();
        input.extend_from_slice(b"\x8eR\x93c\x91\xbe\x98Y <yamada@example.com>\n\n- \x83e\x83X\x83g\x82\xcc\x8fC\x90\xb3\n- Update the th\xc3\xa9\n");
        let normalized = normalize_changes_content(&input);
        assert_eq!(
            normalized,
            "-------------------------------------------------------------------\nWed May 27 16:31:48 UTC 2026 - 山田太郎 <yamada@example.com>\n\n- テストの修正\n- Update the thé\n"
        );
    }

    #[test]
    fn normalize_replaces_a_lone_undecodable_byte() {
        // A single 0x82 is not valid UTF-8 and not a decodable
        // legacy sequence the detector can place: the detector
        // reads it as windows-1252 ‚.
        let normalized = normalize_changes_content(b"\x82");
        assert_eq!(normalized, "‚");
    }
}
