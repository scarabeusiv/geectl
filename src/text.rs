//! Mixed-encoding text: valid UTF-8 with legacy-encoded runs, transcoded to the characters they are.

/// Guess the legacy encoding of bytes that are not valid UTF-8, or `None` when the input is valid UTF-8.
pub(crate) fn guess_legacy_encoding(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    if std::str::from_utf8(bytes).is_ok() {
        return None;
    }
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    Some(detector.guess(None, chardetng::Utf8Detection::Allow))
}

/// Decode `bytes` with one legacy encoding, reporting whether the decoder substituted U+FFFD.
pub(crate) fn decode_with(
    bytes: &[u8],
    encoding: &'static encoding_rs::Encoding,
) -> (String, bool) {
    let (decoded, _, had_errors) = encoding.decode(bytes);
    (decoded.into_owned(), had_errors)
}

/// Decode bytes mixing valid UTF-8 with legacy-encoded bytes, reporting U+FFFD substitutions. Valid UTF-8 passes through; a line the guess decodes cleanly transcodes whole (multi-byte characters span bytes slicing would destroy), unless a single-byte guess meets valid multi-byte UTF-8 — then valid spans stay verbatim and each invalid run decodes with the guess.
pub(crate) fn decode_mixed(
    bytes: &[u8],
    guessed: &'static encoding_rs::Encoding,
) -> (String, bool) {
    // Fully valid UTF-8 is never re-decoded: its bytes can decode cleanly as half-width katakana in Shift_JIS.
    if let Ok(text) = std::str::from_utf8(bytes) {
        return (text.to_string(), false);
    }
    let (decoded, _, had_errors) = guessed.decode(bytes);
    if !had_errors && (guessed_can_error(guessed) || !contains_valid_multibyte_utf8(bytes)) {
        return (decoded.into_owned(), false);
    }
    let mut out = String::new();
    let mut had_replacement = false;
    let mut rest = bytes;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                out.push_str(text);
                break;
            }
            Err(err) => {
                let valid_up_to = err.valid_up_to();
                // The prefix is valid UTF-8 by construction, so this is verbatim.
                out.push_str(&String::from_utf8_lossy(&rest[..valid_up_to]));
                let skip = match err.error_len() {
                    Some(len) => len,
                    None => rest.len() - valid_up_to,
                };
                let (decoded, had_errors) =
                    decode_with(&rest[valid_up_to..valid_up_to + skip], guessed);
                had_replacement |= had_errors;
                out.push_str(&decoded);
                rest = &rest[valid_up_to + skip..];
            }
        }
    }
    (out, had_replacement)
}

/// Whether the guessed encoding can report a decode error at all (a single-byte encoding maps every byte, so a clean decode vouches for nothing there).
pub(crate) fn guessed_can_error(encoding: &'static encoding_rs::Encoding) -> bool {
    encoding.decode(b"\x82").2
}

/// Whether `bytes` hold a valid multi-byte UTF-8 sequence anywhere (a byte >= 0x80 inside valid UTF-8 belongs to a multi-byte character; ASCII-only and invalid bytes do not count).
pub(crate) fn contains_valid_multibyte_utf8(bytes: &[u8]) -> bool {
    let mut rest = bytes;
    while let Err(err) = std::str::from_utf8(rest) {
        if rest[..err.valid_up_to()].iter().any(|&byte| byte >= 0x80) {
            return true;
        }
        match err.error_len() {
            Some(len) => rest = &rest[err.valid_up_to() + len..],
            // A truncated sequence at the end holds no complete multi-byte character.
            None => return false,
        }
    }
    rest.iter().any(|&byte| byte >= 0x80)
}

/// Detect UTF-16 without a BOM from the NUL pattern: ASCII-heavy UTF-16LE carries a NUL at every odd byte, UTF-16BE at every even byte, and a text file never holds NULs otherwise, so a file at least a quarter NULs, with the NULs almost all on one parity, is UTF-16.
fn detect_utf16_without_bom(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    if bytes.len() < 4 {
        return None;
    }
    let (mut even, mut odd) = (0, 0);
    for (i, &byte) in bytes.iter().enumerate() {
        if byte == 0 {
            if i % 2 == 0 {
                even += 1;
            } else {
                odd += 1;
            }
        }
    }
    let total = even + odd;
    if total * 4 < bytes.len() {
        return None;
    }
    if odd * 10 >= total * 9 {
        Some(encoding_rs::UTF_16LE)
    } else if even * 10 >= total * 9 {
        Some(encoding_rs::UTF_16BE)
    } else {
        None
    }
}

/// What the BOM/UTF-16 preamble of a text input says; the policy is shared with `.changes` normalization.
pub(crate) enum TextPreamble<'a> {
    /// A UTF-32 BOM: no UTF-32 decoder here, so the bytes are never misread as UTF-16.
    Utf32(&'a [u8]),
    /// A UTF-16 input (BOM, NUL pattern, or chardetng guess): decode whole, never split on `\n` bytes.
    Utf16(&'static encoding_rs::Encoding, &'a [u8]),
    /// Anything else, with a UTF-8 BOM stripped, plus the whole-input legacy guess.
    Plain(&'a [u8], Option<&'static encoding_rs::Encoding>),
}

/// Split the BOM/UTF-16 preamble off `bytes`: a UTF-8 BOM is stripped, a UTF-32 BOM is guarded, and UTF-16 is detected whole before any per-line decode.
pub(crate) fn split_text_preamble(bytes: &[u8]) -> TextPreamble<'_> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    if bytes.starts_with(b"\xFF\xFE\x00\x00") || bytes.starts_with(b"\x00\x00\xFE\xFF") {
        return TextPreamble::Utf32(bytes);
    }
    let guessed = guess_legacy_encoding(bytes);
    if bytes.starts_with(b"\xFF\xFE") {
        return TextPreamble::Utf16(encoding_rs::UTF_16LE, &bytes[2..]);
    }
    if bytes.starts_with(b"\xFE\xFF") {
        return TextPreamble::Utf16(encoding_rs::UTF_16BE, &bytes[2..]);
    }
    if let Some(encoding) = detect_utf16_without_bom(bytes) {
        return TextPreamble::Utf16(encoding, bytes);
    }
    if let Some(encoding) = guessed {
        if encoding == encoding_rs::UTF_16LE || encoding == encoding_rs::UTF_16BE {
            return TextPreamble::Utf16(encoding, bytes);
        }
    }
    TextPreamble::Plain(bytes, guessed)
}

/// Decode bytes that are valid UTF-8 or legacy-encoded text: the shared entry point for reading a server text file that may not be UTF-8 (a `.changes` file). UTF-16 decodes whole; anything else decodes per line, so a valid UTF-8 line is never rewritten by the legacy guess.
pub(crate) fn decode_legacy_text(bytes: &[u8]) -> String {
    match split_text_preamble(bytes) {
        TextPreamble::Utf32(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        TextPreamble::Utf16(encoding, input) => {
            let (decoded, _, _) = encoding.decode(input);
            decoded.into_owned()
        }
        TextPreamble::Plain(bytes, guessed) => bytes
            .split(|&byte| byte == b'\n')
            .map(|line| match guessed {
                Some(encoding) => decode_mixed(line, encoding).0,
                None => String::from_utf8_lossy(line).into_owned(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// Decode bytes mixing valid UTF-8 with Windows-1252 runs, as a build log does: valid UTF-8 passes through, each invalid run decodes as Windows-1252, and bytes Windows-1252 leaves undefined substitute U+FFFD (read-only display text, never written back).
pub(crate) fn decode_mixed_utf8(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }
    let mut out = String::with_capacity(bytes.len());
    let mut rest = bytes;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                out.push_str(text);
                break;
            }
            Err(err) => {
                let valid_up_to = err.valid_up_to();
                // The prefix is valid UTF-8 by construction, so this is verbatim.
                out.push_str(&String::from_utf8_lossy(&rest[..valid_up_to]));
                let len = match err.error_len() {
                    Some(len) => len,
                    // A truncated sequence at the end: the rest of the input is the run.
                    None => rest.len() - valid_up_to,
                };
                let (decoded, _, _) =
                    encoding_rs::WINDOWS_1252.decode(&rest[valid_up_to..valid_up_to + len]);
                // encoding_rs maps the five bytes Windows-1252 leaves undefined to C1 controls; the log keeps the U+FFFD reading instead.
                for c in decoded.chars() {
                    out.push(match c {
                        '\u{0081}' | '\u{008d}' | '\u{008f}' | '\u{0090}' | '\u{009d}' => {
                            '\u{fffd}'
                        }
                        c => c,
                    });
                }
                rest = &rest[valid_up_to + len..];
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_utf8_passes_valid_utf8_through_unchanged() {
        let text = "Tomáš Chvátal — café\n";
        assert_eq!(decode_mixed_utf8(text.as_bytes()), text);
    }

    #[test]
    fn mixed_utf8_decodes_a_latin1_line() {
        assert_eq!(
            decode_mixed_utf8(b"Tom\xe1s Chv\xe1tal\n"),
            "Tom\u{e1}s Chv\u{e1}tal\n"
        );
    }

    #[test]
    fn mixed_utf8_keeps_a_utf8_half_and_decodes_a_latin1_half() {
        let mut bytes = "café ".as_bytes().to_vec();
        bytes.extend_from_slice(b"Tom\xe1s\n");
        assert_eq!(decode_mixed_utf8(&bytes), "café Tom\u{e1}s\n");
    }

    #[test]
    fn mixed_utf8_decodes_windows_1252_punctuation() {
        assert_eq!(decode_mixed_utf8(b"\x93quote\x94"), "\u{201c}quote\u{201d}");
        assert_eq!(decode_mixed_utf8(b"\x80"), "\u{20ac}");
        assert_eq!(decode_mixed_utf8(b"\x9a"), "\u{0161}");
    }

    #[test]
    fn mixed_utf8_replaces_undefined_windows_1252_bytes() {
        assert_eq!(decode_mixed_utf8(b"\x81"), "\u{fffd}");
        assert_eq!(decode_mixed_utf8(b"a\x8db"), "a\u{fffd}b");
    }

    #[test]
    fn mixed_utf8_decodes_a_truncated_tail_as_windows_1252() {
        assert_eq!(decode_mixed_utf8(b"abc\xc3"), "abc\u{c3}");
    }

    #[test]
    fn mixed_utf8_decodes_an_empty_input() {
        assert_eq!(decode_mixed_utf8(b""), "");
    }

    #[test]
    fn mixed_utf8_decodes_a_large_input_with_an_invalid_run_in_the_middle() {
        let mut bytes = vec![b'a'; 512 * 1024];
        bytes.extend_from_slice(b"Tom\xe1s\n");
        bytes.extend_from_slice(&vec![b'b'; 512 * 1024]);
        let decoded = decode_mixed_utf8(&bytes);
        assert!(decoded.starts_with(&"a".repeat(512 * 1024)));
        assert!(decoded.contains("Tom\u{e1}s\n"));
        assert!(decoded.ends_with(&"b".repeat(512 * 1024)));
    }

    #[test]
    fn legacy_text_keeps_a_valid_utf8_line_in_a_shift_jis_file() {
        let mut bytes = "café\n".as_bytes().to_vec();
        let japanese = b"\x93\xfa\x96\x7b\x8c\xea\x82\xc5\x82\xb7\n";
        bytes.extend_from_slice(japanese);
        bytes.extend_from_slice(japanese);
        bytes.extend_from_slice(japanese);
        assert_eq!(
            decode_legacy_text(&bytes),
            "café\n日本語です\n日本語です\n日本語です\n"
        );
    }

    #[test]
    fn legacy_text_strips_a_utf8_bom() {
        assert_eq!(decode_legacy_text(b"\xEF\xBB\xBFhello\n"), "hello\n");
    }

    #[test]
    fn legacy_text_decodes_utf16_with_a_bom() {
        let mut le = b"\xFF\xFE".to_vec();
        le.extend("hello\n".encode_utf16().flat_map(|unit| unit.to_le_bytes()));
        assert_eq!(decode_legacy_text(&le), "hello\n");
        let mut be = b"\xFE\xFF".to_vec();
        be.extend("hello\n".encode_utf16().flat_map(|unit| unit.to_be_bytes()));
        assert_eq!(decode_legacy_text(&be), "hello\n");
    }

    #[test]
    fn legacy_text_decodes_bom_less_utf16() {
        let bytes: Vec<u8> = "hello\n"
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        assert_eq!(decode_legacy_text(&bytes), "hello\n");
    }

    #[test]
    fn legacy_text_does_not_misread_a_utf32_bom_as_utf16() {
        let mut bytes = b"\xFF\xFE\x00\x00".to_vec();
        bytes.extend_from_slice(&[0x68, 0, 0, 0, 0x69, 0, 0, 0]);
        let decoded = decode_legacy_text(&bytes);
        assert!(decoded.contains('\u{FFFD}'), "decoded: {decoded:?}");
    }
}
