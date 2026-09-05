//! Direct-Mapping row subjects and RFC 3987 name encoding.

use sf_core::ir::{Segment, Template, TermMap, TermSpec};
use sf_core::{NamedNode, Result, TableSchema, Term};

use super::input::SYNTHETIC_ROW_ID_COLUMN;
use super::DirectMappingRowIdentity;

pub(super) fn blank_node_template(
    table: &TableSchema,
    row_identity: DirectMappingRowIdentity,
) -> Result<TermMap> {
    let identity_column = match row_identity {
        DirectMappingRowIdentity::SqliteRowId
        | DirectMappingRowIdentity::PostgresCtidConformance => SYNTHETIC_ROW_ID_COLUMN,
        DirectMappingRowIdentity::RequirePrimaryKey => {
            return Err(sf_core::Error::Mapping(format!(
                "Direct Mapping table {:?} has no primary key and this backend has no admitted row identity",
                table.name
            )))
        }
    };
    let segments = vec![
        Segment::Literal(format!("{}_", encode(&table.name)).into()),
        Segment::Column(identity_column.into()),
    ];
    Ok(TermMap::Template(
        Template::from_segments(segments).expect("non-empty segment list"),
        TermSpec::blank_node(),
    ))
}

/// Shared logical name resolved by the dialect emitter to SQLite `rowid` or,
/// only in the conformance profile, PostgreSQL `ctid`.
pub(super) fn constant_iri(iri: &str) -> Result<TermMap> {
    NamedNode::new(iri)
        .map(|node| TermMap::Constant(Term::NamedNode(node)))
        .map_err(|error| {
            sf_core::Error::Mapping(format!("invalid Direct Mapping IRI {iri:?}: {error}"))
        })
}

/// Encode an SQL identifier for the fixed portion of a Direct-Mapping IRI.
pub(super) fn encode(name: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::with_capacity(encoded_len(name).unwrap_or(name.len()));
    for character in name.chars() {
        if is_iunreserved(character) {
            output.push(character);
        } else {
            let mut utf8 = [0_u8; 4];
            for byte in character.encode_utf8(&mut utf8).as_bytes() {
                output.push('%');
                output.push(HEX[(byte >> 4) as usize] as char);
                output.push(HEX[(byte & 0x0f) as usize] as char);
            }
        }
    }
    output
}

/// Prospective encoded byte count without allocating the encoded identifier.
pub(super) fn encoded_len(name: &str) -> Option<usize> {
    name.chars().try_fold(0_usize, |total, character| {
        let bytes = if is_iunreserved(character) {
            character.len_utf8()
        } else {
            character.len_utf8().checked_mul(3)?
        };
        total.checked_add(bytes)
    })
}

/// RFC 3987 `iunreserved` for a path/fragment component. `iprivate` is not
/// included: RFC 3987 admits those code points only in `iquery`, while Direct
/// Mapping inserts database identifiers into path and fragment components.
fn is_iunreserved(character: char) -> bool {
    character.is_ascii_alphanumeric()
        || matches!(character, '-' | '.' | '_' | '~')
        || matches!(
            character as u32,
            0x00A0..=0xD7FF
                | 0xF900..=0xFDCF
                | 0xFDF0..=0xFFEF
                | 0x10000..=0x1FFFD
                | 0x20000..=0x2FFFD
                | 0x30000..=0x3FFFD
                | 0x40000..=0x4FFFD
                | 0x50000..=0x5FFFD
                | 0x60000..=0x6FFFD
                | 0x70000..=0x7FFFD
                | 0x80000..=0x8FFFD
                | 0x90000..=0x9FFFD
                | 0xA0000..=0xAFFFD
                | 0xB0000..=0xBFFFD
                | 0xC0000..=0xCFFFD
                | 0xD0000..=0xDFFFD
                | 0xE1000..=0xEFFFD
        )
}

#[cfg(test)]
mod tests {
    use super::{encode, encoded_len};

    #[test]
    fn unreserved_ascii_passes_through_untouched() {
        assert_eq!(encode("Table-Name_1.2~3"), "Table-Name_1.2~3");
    }

    #[test]
    fn ascii_specials_are_percent_encoded_with_uppercase_hex() {
        assert_eq!(encode("a b"), "a%20b");
        assert_eq!(encode("a#b"), "a%23b");
        assert_eq!(encode("a/b"), "a%2Fb");
    }

    #[test]
    fn rfc3987_ucschar_passes_through_verbatim() {
        // W3C Direct Mapping uses RFC 3987 IRI encoding: ucschar (for example,
        // CJK) is not percent-escaped like a strict RFC 3986 URI component.
        assert_eq!(encode("café"), "café");
        assert_eq!(encode("表"), "表");
    }

    #[test]
    fn controls_private_use_and_noncharacters_are_percent_encoded() {
        let cases = [
            ('\0', "%00"),
            ('\u{007F}', "%7F"),
            ('\u{0080}', "%C2%80"),
            ('\u{009F}', "%C2%9F"),
            ('\u{E000}', "%EE%80%80"),
            ('\u{F8FF}', "%EF%A3%BF"),
            ('\u{FDD0}', "%EF%B7%90"),
            ('\u{FDEF}', "%EF%B7%AF"),
            ('\u{FFFE}', "%EF%BF%BE"),
            ('\u{1FFFE}', "%F0%9F%BF%BE"),
            ('\u{F0000}', "%F3%B0%80%80"),
            ('\u{10FFFD}', "%F4%8F%BF%BD"),
        ];
        for (input, expected) in cases {
            let input = input.to_string();
            assert_eq!(
                encode(&input),
                expected,
                "U+{:04X}",
                input.chars().next().unwrap() as u32
            );
            assert_eq!(encoded_len(&input), Some(expected.len()));
        }
    }

    #[test]
    fn ucschar_boundary_code_points_pass_and_adjacent_holes_encode() {
        for character in [
            '\u{00A0}',
            '\u{D7FF}',
            '\u{F900}',
            '\u{FDCF}',
            '\u{FDF0}',
            '\u{FFEF}',
            '\u{10000}',
            '\u{1FFFD}',
            '\u{E1000}',
            '\u{EFFFD}',
        ] {
            assert_eq!(encode(&character.to_string()), character.to_string());
        }
        for character in ['\u{0080}', '\u{FDD0}', '\u{FDEF}', '\u{FFF0}', '\u{E0FFF}'] {
            assert!(encode(&character.to_string()).starts_with('%'));
        }
    }
}
