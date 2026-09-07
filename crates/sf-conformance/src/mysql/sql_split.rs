//! Minimal statement boundary scanner for sealed SQL fixture text.
//!
//! It does not rewrite SQL. Delimiters inside strings, identifiers, and comments
//! remain part of the captured statement sent to MySQL.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Plain,
    SingleQuote,
    DoubleQuote,
    Backtick,
    LineComment,
    BlockComment,
}

pub(super) fn split(input: &str) -> Result<Vec<&str>, String> {
    let bytes = input.as_bytes();
    let mut statements = Vec::new();
    let mut state = State::Plain;
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        let next = bytes.get(index + 1).copied();
        match state {
            State::Plain => match (byte, next) {
                (b'\'', _) => state = State::SingleQuote,
                (b'"', _) => state = State::DoubleQuote,
                (b'`', _) => state = State::Backtick,
                (b'-', Some(b'-')) | (b'#', _) => state = State::LineComment,
                (b'/', Some(b'*')) => {
                    state = State::BlockComment;
                    index += 1;
                }
                (b';', _) => {
                    push_nonempty(&mut statements, &input[start..index]);
                    start = index + 1;
                }
                _ => {}
            },
            State::SingleQuote => {
                if byte == b'\\' {
                    index += usize::from(next.is_some());
                } else if byte == b'\'' {
                    if next == Some(b'\'') {
                        index += 1;
                    } else {
                        state = State::Plain;
                    }
                }
            }
            State::DoubleQuote => {
                if byte == b'\\' {
                    index += usize::from(next.is_some());
                } else if byte == b'"' {
                    if next == Some(b'"') {
                        index += 1;
                    } else {
                        state = State::Plain;
                    }
                }
            }
            State::Backtick => {
                if byte == b'\\' {
                    index += usize::from(next.is_some());
                } else if byte == b'`' {
                    if next == Some(b'`') {
                        index += 1;
                    } else {
                        state = State::Plain;
                    }
                }
            }
            State::LineComment => {
                if byte == b'\n' || byte == b'\r' {
                    state = State::Plain;
                }
            }
            State::BlockComment => {
                if byte == b'*' && next == Some(b'/') {
                    state = State::Plain;
                    index += 1;
                }
            }
        }
        index += 1;
    }
    if !matches!(state, State::Plain | State::LineComment) {
        return Err("create.sql contains an unterminated quoted value or comment".to_owned());
    }
    push_nonempty(&mut statements, &input[start..]);
    if statements.is_empty() {
        return Err("create.sql contains no executable statements".to_owned());
    }
    Ok(statements)
}

fn push_nonempty<'a>(statements: &mut Vec<&'a str>, candidate: &'a str) {
    let trimmed = candidate.trim();
    if !trimmed.is_empty() {
        statements.push(trimmed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_delimiters_inside_values_identifiers_and_comments() {
        let sql = "CREATE TABLE \"a;b\" (`c;d` TEXT);\n\
                   INSERT INTO \"a;b\" VALUES ('x;''y'); -- ignored ;\n\
                   /* ignored ; */ INSERT INTO \"a;b\" VALUES ('z');";
        let statements = split(sql).expect("split fixture SQL");
        assert_eq!(statements.len(), 3);
        assert!(statements[0].contains("\"a;b\""));
        assert!(statements[1].contains("'x;''y'"));
        assert!(statements[2].contains("/* ignored ; */"));
    }

    #[test]
    fn rejects_empty_and_unterminated_input() {
        assert!(split(" \n").unwrap_err().contains("no executable"));
        for sql in ["SELECT 'x", "SELECT \"x", "SELECT `x", "SELECT /* x"] {
            assert!(split(sql).unwrap_err().contains("unterminated"), "{sql}");
        }
    }
}
