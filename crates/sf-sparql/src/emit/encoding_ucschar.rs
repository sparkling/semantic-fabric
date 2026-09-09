//! Fixed engine grammar for the shared RFC3987 alphabet; no source values inline.
use sf_core::ir::encoding::UCSCHAR_RANGES;
use sf_sql::Dialect;

/// Does byte `n` belong to a complete, valid UTF-8 encoding of a ucschar?
/// Byte iteration preserves embedded NUL; explicit windows avoid interpreting
/// isolated continuation bytes as Unicode replacement characters.
pub(super) fn byte_member(column: &str, dialect: Dialect) -> String {
    byte_in_ranges(column, dialect, UCSCHAR_RANGES)
}

pub(super) fn valid_non_ascii_byte(column: &str, dialect: Dialect) -> String {
    byte_in_ranges(column, dialect, &[(0x80, 0xD7FF), (0xE000, 0x10FFFF)])
}

fn byte_in_ranges(column: &str, dialect: Dialect, scalar_ranges: &[(u32, u32)]) -> String {
    let blob = match dialect {
        Dialect::Sqlite => format!("CAST({column} AS BLOB)"),
        Dialect::MySql => format!("CAST({column} AS BINARY)"),
        _ => unreachable!("byte-oriented encoders only"),
    };
    // MySQL FOR ORDINALITY is unsigned; predicate evaluation order must not
    // determine whether a window near byte one underflows.
    let index = if dialect == Dialect::MySql {
        "CAST(n AS SIGNED)"
    } else {
        "n"
    };
    let hex = |offset: usize, length: usize| {
        let expression = format!("HEX(SUBSTR({blob}, {index} - {offset}, {length}))");
        match dialect {
            Dialect::Sqlite => format!("({expression} COLLATE BINARY)"),
            _ => format!("CAST({expression} AS BINARY)"),
        }
    };
    let scalar_hex = |point| {
        char::from_u32(point)
            .expect("fixed scalar boundary")
            .to_string()
            .bytes()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>()
    };
    let mut alternatives = Vec::new();
    for (width, first, last) in [(2, 0x80, 0x7FF), (3, 0x800, 0xFFFF), (4, 0x10000, 0x10FFFF)] {
        let ranges: Vec<_> = scalar_ranges
            .iter()
            .filter_map(|&(lo, hi)| {
                let (lo, hi) = (lo.max(first), hi.min(last));
                (lo <= hi).then(|| (scalar_hex(lo), scalar_hex(hi)))
            })
            .collect();
        for offset in 0..width {
            let window = hex(offset, width);
            let ranges = ranges
                .iter()
                .map(|(lo, hi)| format!("{window} BETWEEN '{lo}' AND '{hi}'"))
                .collect::<Vec<_>>()
                .join(" OR ");
            // Every continuation byte must be 80..BF; a lexicographic range
            // alone would also admit malformed/truncated byte sequences.
            let mut guards = vec![
                format!("{index} > {offset}"),
                format!("LENGTH({blob}) >= {index} - {offset} + {width} - 1"),
            ];
            for continuation in 1..width {
                let position = format!("{index} - {offset} + {continuation}");
                let part = format!("HEX(SUBSTR({blob}, {position}, 1))");
                let part = match dialect {
                    Dialect::Sqlite => format!("({part} COLLATE BINARY)"),
                    _ => format!("CAST({part} AS BINARY)"),
                };
                guards.push(format!("{part} BETWEEN '80' AND 'BF'"));
            }
            guards.push(format!("({ranges})"));
            alternatives.push(format!("({})", guards.join(" AND ")));
        }
    }
    format!("({})", alternatives.join(" OR "))
}

pub(super) fn codepoint_member(expression: &str) -> String {
    format!(
        "({})",
        UCSCHAR_RANGES
            .iter()
            .map(|(lo, hi)| format!("{expression} BETWEEN {lo} AND {hi}"))
            .collect::<Vec<_>>()
            .join(" OR ")
    )
}
