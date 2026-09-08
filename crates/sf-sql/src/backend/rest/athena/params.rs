//! `?` placeholder analysis and `ExecutionParameters` rendering.

use super::wire::WireResult;

/// Render `params` as Athena `ExecutionParameters`, after validating that the
/// `?` placeholders in `sql` line up with them.
///
/// LEXICAL LIMITATION — every parameter is rendered as a single-quoted Athena
/// **string** literal. `SqlBackend::open_branch` hands this backend untyped
/// lexical strings with no type codes, so there is nothing to derive a typed
/// binding from; this is NOT typed server-side binding. A comparison against a
/// non-string column therefore needs an explicit cast in the emitted SQL.
/// Embedded single quotes are doubled, which is the only escape Athena's
/// (Trino) string-literal grammar defines.
pub(crate) fn render_execution_parameters(sql: &str, params: &[String]) -> WireResult<Vec<String>> {
    let placeholders = count_placeholders(sql)?;
    if placeholders != params.len() {
        return Err(format!(
            "query has {placeholders} '?' placeholder(s) but {} parameter(s) were supplied",
            params.len()
        ));
    }
    Ok(params.iter().map(|p| quote_literal(p)).collect())
}

/// Single-quoted Athena string literal with `''` quote escaping.
pub(crate) fn quote_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            out.push('\'');
        }
        out.push(ch);
    }
    out.push('\'');
    out
}

/// Count `?` placeholders outside string literals, quoted identifiers, and
/// comments. Question marks in quoted text are ordinary characters and do not
/// consume an Athena execution parameter.
fn count_placeholders(sql: &str) -> WireResult<usize> {
    #[derive(PartialEq)]
    enum Lex {
        Plain,
        Single,
        Double,
        Backtick,
        LineComment,
        BlockComment,
    }

    let mut state = Lex::Plain;
    let mut count = 0usize;
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match state {
            Lex::Plain => match c {
                '\'' => state = Lex::Single,
                '"' => state = Lex::Double,
                '`' => state = Lex::Backtick,
                '-' if next == Some('-') => {
                    state = Lex::LineComment;
                    i += 1;
                }
                '/' if next == Some('*') => {
                    state = Lex::BlockComment;
                    i += 1;
                }
                '?' => count += 1,
                _ => {}
            },
            Lex::Single | Lex::Double | Lex::Backtick => {
                let quote = match state {
                    Lex::Single => '\'',
                    Lex::Double => '"',
                    _ => '`',
                };
                if c == quote {
                    if next == Some(quote) {
                        i += 1; // doubled quote escape — stay inside
                    } else {
                        state = Lex::Plain;
                    }
                }
            }
            Lex::LineComment => {
                if c == '\n' {
                    state = Lex::Plain;
                }
            }
            Lex::BlockComment => {
                if c == '*' && next == Some('/') {
                    state = Lex::Plain;
                    i += 1;
                }
            }
        }
        i += 1;
    }
    match state {
        Lex::Single | Lex::Double | Lex::Backtick => {
            Err("query ends inside an unterminated quoted literal or identifier".to_owned())
        }
        _ => Ok(count),
    }
}

#[cfg(test)]
mod tests {
    use super::{quote_literal, render_execution_parameters};

    #[test]
    fn parameters_render_as_quoted_string_literals() {
        let out = render_execution_parameters(
            "SELECT * FROM t WHERE id = ? AND name = ?",
            &["42".to_owned(), "O'Brien".to_owned()],
        )
        .unwrap();
        assert_eq!(out, vec!["'42'".to_owned(), "'O''Brien'".to_owned()]);
    }

    #[test]
    fn quote_literal_doubles_every_embedded_quote() {
        assert_eq!(quote_literal("a'b'c"), "'a''b''c'");
        assert_eq!(quote_literal("'; DROP TABLE t --"), "'''; DROP TABLE t --'");
        assert_eq!(quote_literal(""), "''");
    }

    #[test]
    fn placeholder_count_mismatch_is_rejected() {
        assert!(render_execution_parameters("SELECT ?", &[]).is_err());
        assert!(render_execution_parameters("SELECT 1", &["x".to_owned()]).is_err());
        assert!(render_execution_parameters("SELECT ?, ?", &["x".to_owned()]).is_err());
    }

    #[test]
    fn question_mark_inside_quotes_is_not_a_placeholder() {
        assert!(render_execution_parameters("SELECT 'what?'", &[]).is_ok());
        assert!(render_execution_parameters("SELECT \"col?\" FROM t", &[]).is_ok());
        assert!(render_execution_parameters("SELECT `col?` FROM t", &[]).is_ok());
        assert!(render_execution_parameters("SELECT 'unterminated", &[]).is_err());
    }

    #[test]
    fn placeholders_in_comments_are_not_counted() {
        assert!(render_execution_parameters("SELECT 1 -- why?\n", &[]).is_ok());
        assert!(render_execution_parameters("SELECT /* huh? */ 1", &[]).is_ok());
        let out = render_execution_parameters("SELECT ? -- and?\n", &["v".to_owned()]).unwrap();
        assert_eq!(out, vec!["'v'".to_owned()]);
    }

    #[test]
    fn doubled_quotes_do_not_end_the_literal() {
        assert!(render_execution_parameters("SELECT 'a''?b'", &[]).is_ok());
    }
}
