//! W3C SQL-standard delimited identifiers adapted to MySQL's native quoting.
//!
//! The sealed suite uses `"identifier"`. The live session enables ANSI_QUOTES,
//! so MySQL itself accepts those queries, but the MySQL SQL-AST parser treats
//! double quotes as strings. This bounded lexical adapter changes identifier
//! delimiters only; strings and comments remain byte-for-byte unchanged.

use sf_core::ir::{LogicalSource, TriplesMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Plain,
    SingleQuote,
    DoubleIdentifier,
    Backtick,
    LineComment,
    BlockComment,
}

pub(super) fn normalize_sources(maps: &mut [TriplesMap]) -> Result<(), String> {
    for map in maps {
        if let LogicalSource::Query(query) = &mut map.source {
            *query = normalize_standard_identifiers(query)?;
        }
    }
    Ok(())
}

fn normalize_standard_identifiers(input: &str) -> Result<String, String> {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut state = State::Plain;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        let next = bytes.get(index + 1).copied();
        match state {
            State::Plain => match (byte, next) {
                (b'\'', _) => {
                    output.push(byte);
                    state = State::SingleQuote;
                }
                (b'"', _) => {
                    output.push(b'`');
                    state = State::DoubleIdentifier;
                }
                (b'`', _) => {
                    output.push(byte);
                    state = State::Backtick;
                }
                (b'-', Some(b'-')) | (b'#', _) => {
                    output.push(byte);
                    state = State::LineComment;
                }
                (b'/', Some(b'*')) => {
                    output.extend_from_slice(b"/*");
                    state = State::BlockComment;
                    index += 1;
                }
                _ => output.push(byte),
            },
            State::SingleQuote => {
                output.push(byte);
                if byte == b'\\' {
                    if let Some(next) = next {
                        output.push(next);
                        index += 1;
                    }
                } else if byte == b'\'' {
                    if next == Some(b'\'') {
                        output.push(b'\'');
                        index += 1;
                    } else {
                        state = State::Plain;
                    }
                }
            }
            State::DoubleIdentifier => {
                if byte == b'"' {
                    if next == Some(b'"') {
                        output.push(b'"');
                        index += 1;
                    } else {
                        output.push(b'`');
                        state = State::Plain;
                    }
                } else if byte == b'`' {
                    output.extend_from_slice(b"``");
                } else {
                    output.push(byte);
                }
            }
            State::Backtick => {
                output.push(byte);
                if byte == b'`' {
                    if next == Some(b'`') {
                        output.push(b'`');
                        index += 1;
                    } else {
                        state = State::Plain;
                    }
                }
            }
            State::LineComment => {
                output.push(byte);
                if byte == b'\n' || byte == b'\r' {
                    state = State::Plain;
                }
            }
            State::BlockComment => {
                output.push(byte);
                if byte == b'*' && next == Some(b'/') {
                    output.push(b'/');
                    index += 1;
                    state = State::Plain;
                }
            }
        }
        index += 1;
    }
    if !matches!(state, State::Plain | State::LineComment) {
        return Err("rr:sqlQuery contains an unterminated quoted value or comment".to_owned());
    }
    String::from_utf8(output).map_err(|_| "rr:sqlQuery is not valid UTF-8".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_only_standard_identifier_quotes() {
        let input = "SELECT \"EMP\".*, 'a\"b', \"a\"\"b`c\" FROM \"EMP\" -- \"kept\"\n";
        let normalized = normalize_standard_identifiers(input).expect("valid query");
        assert_eq!(
            normalized,
            "SELECT `EMP`.*, 'a\"b', `a\"b``c` FROM `EMP` -- \"kept\"\n"
        );
    }

    #[test]
    fn preserves_native_backticks_and_rejects_unterminated_input() {
        assert_eq!(
            normalize_standard_identifiers("SELECT `a``b` FROM `t`").unwrap(),
            "SELECT `a``b` FROM `t`"
        );
        for input in ["SELECT 'x", "SELECT \"x", "SELECT `x", "SELECT /* x"] {
            assert!(normalize_standard_identifiers(input).is_err(), "{input}");
        }
    }
}
