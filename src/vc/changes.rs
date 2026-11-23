//! Parsing of `.changes` files: entries, headers, timestamps.
//!
//! The format is the one `vc` writes: a 67-dash separator, a
//! `Day Mon DD HH:MM:SS UTC YYYY - Name <email>` header, a blank line,
//! then `-` / `*` bullets with indented continuations. The parser is
//! lenient about what it accepts — validation decides what is wrong —
//! but strict about what it reports: a header that does not parse is
//! `None`, never a guessed date. No date library is involved: the
//! civil-date conversion is the standard days-from-civil algorithm.

/// The separator length `vc` writes (67 dashes).
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
            header.render(),
            "Wed May 27 16:31:48 UTC 2026 - Jane Packager <jane@example.com>"
        );
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
}
