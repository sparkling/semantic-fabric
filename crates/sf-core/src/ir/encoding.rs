//! R2RML §7.3 uses RFC3987 iunreserved, not every non-ASCII code point.

/// RFC3987 §2.2 `ucschar` ranges. SQL template emitters consume the same table.
/// Private-use, noncharacter and C1-control ranges are deliberately absent.
pub const UCSCHAR_RANGES: &[(u32, u32)] = &[
    (0xA0, 0xD7FF),
    (0xF900, 0xFDCF),
    (0xFDF0, 0xFFEF),
    (0x10000, 0x1FFFD),
    (0x20000, 0x2FFFD),
    (0x30000, 0x3FFFD),
    (0x40000, 0x4FFFD),
    (0x50000, 0x5FFFD),
    (0x60000, 0x6FFFD),
    (0x70000, 0x7FFFD),
    (0x80000, 0x8FFFD),
    (0x90000, 0x9FFFD),
    (0xA0000, 0xAFFFD),
    (0xB0000, 0xBFFFD),
    (0xC0000, 0xCFFFD),
    (0xD0000, 0xDFFFD),
    (0xE1000, 0xEFFFD),
];

/// Append the R2RML IRI-safe form, escaping UTF-8 bytes outside `iunreserved`.
/// The caller owns/clears `out`; fixed mapping slugs and row substitutions use
/// this same function. This encodes a component, not a complete IRI.
pub fn percent_encode_iri(value: &str, out: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut utf8 = [0; 4];
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric()
            || matches!(ch, '-' | '.' | '_' | '~')
            || UCSCHAR_RANGES
                .iter()
                .any(|&(lo, hi)| (lo..=hi).contains(&(ch as u32)))
        {
            out.push(ch);
        } else {
            for byte in ch.encode_utf8(&mut utf8).bytes() {
                out.push('%');
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 0x0f) as usize] as char);
            }
        }
    }
}
