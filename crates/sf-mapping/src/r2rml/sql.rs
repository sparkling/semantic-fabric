//! SQL-identifier and `rr:template` / `rr:sqlQuery` normalisation helpers for the
//! R2RML parser (ADR-0002 R2RML-only; the SQL-identifier and base-resolution
//! rules of R2RML §7.3/§11/§C). Kept apart from the vocabulary walk in the parent
//! module.

use sf_core::ir::{Segment, Template};

/// Normalise an SQL identifier (`rr:tableName` / `rr:column` / `rr:child` /
/// `rr:parent`): a value written as a **delimited identifier** (`"deptId"`,
/// R2RML §C/SQL) is unwrapped to its bare name (with `""` un-escaped to `"`), so
/// the engine re-quotes it once per dialect rather than double-quoting. A plain
/// identifier passes through unchanged.
pub(super) fn sql_identifier(raw: &str) -> String {
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"' {
        raw[1..raw.len() - 1].replace("\"\"", "\"")
    } else {
        raw.to_owned()
    }
}

/// Strip a single trailing `;` (and surrounding whitespace) from an `rr:sqlQuery`
/// so it can be wrapped as a derived table `(<query>) t0` without a stray
/// statement terminator breaking the enclosing SQL.
pub(super) fn strip_trailing_semicolon(query: &str) -> String {
    let trimmed = query.trim_end();
    trimmed
        .strip_suffix(';')
        .unwrap_or(trimmed)
        .trim_end()
        .to_owned()
}

/// Normalise delimited-identifier column names inside a template's placeholders
/// (`{"job"}` → `{job}`), mirroring [`sql_identifier`] for `rr:column`.
pub(super) fn normalize_template_idents(template: Template) -> Template {
    let segments: Vec<Segment> = template
        .segments()
        .iter()
        .map(|s| match s {
            Segment::Column(c) => Segment::Column(sql_identifier(c).into()),
            Segment::Literal(l) => Segment::Literal(l.clone()),
        })
        .collect();
    Template::from_segments(segments).unwrap_or(template)
}

/// A pragmatic BCP47 [RFC 5646] well-formedness check for `rr:language` (R2RML
/// §7.4). The primary language subtag must be a 2–3-letter ISO 639 code (the 4-
/// and 5–8-letter `langtag` productions are reserved with no current assignments,
/// so a value like `"english"` / `"spanish"` is rejected); each remaining subtag
/// must be 1–8 alphanumerics. This accepts `en`, `es`, `de-DE`, `zh-Hant` and
/// rejects spelt-out language names.
pub(super) fn is_well_formed_language_tag(tag: &str) -> bool {
    let mut subtags = tag.split('-');
    let Some(primary) = subtags.next() else {
        return false;
    };
    let primary_ok =
        (2..=3).contains(&primary.len()) && primary.bytes().all(|b| b.is_ascii_alphabetic());
    if !primary_ok {
        return false;
    }
    subtags.all(|s| (1..=8).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_delimited_identifier_quotes() {
        assert_eq!(sql_identifier("\"deptId\""), "deptId");
        assert_eq!(sql_identifier("plain"), "plain");
        assert_eq!(sql_identifier("\"a\"\"b\""), "a\"b"); // "" → "
    }

    #[test]
    fn strips_trailing_query_semicolon() {
        assert_eq!(strip_trailing_semicolon("SELECT 1 ;\n"), "SELECT 1");
        assert_eq!(strip_trailing_semicolon("SELECT 1"), "SELECT 1");
    }

    #[test]
    fn relative_template_gets_base_absolute_does_not() {
        let base = "http://example.com/base/";
        let (rel, late) = super::super::iri_template::resolve_iri_template(
            Template::parse("{Name}").unwrap(),
            base,
        );
        assert!(!late);
        assert_eq!(rel.segments()[0], Segment::Literal(base.into()));
        let (abs, late) = super::super::iri_template::resolve_iri_template(
            Template::parse("http://e/{id}").unwrap(),
            base,
        );
        assert!(!late);
        assert_eq!(abs.segments()[0], Segment::Literal("http://e/".into()));
    }

    #[test]
    fn language_tag_well_formedness() {
        // ISO 639 primary subtags (2–3 ALPHA), with optional subtags, are valid;
        // spelt-out names (the reserved 5–8 ALPHA form) are rejected (R2RML §7.4).
        assert!(is_well_formed_language_tag("en"));
        assert!(is_well_formed_language_tag("es"));
        assert!(is_well_formed_language_tag("de-DE"));
        assert!(is_well_formed_language_tag("zh-Hant"));
        assert!(!is_well_formed_language_tag("english"));
        assert!(!is_well_formed_language_tag("spanish"));
        assert!(!is_well_formed_language_tag(""));
        assert!(!is_well_formed_language_tag("e"));
    }

    #[test]
    fn normalises_quoted_template_placeholder() {
        let t = normalize_template_idents(Template::parse("http://e/{\"job\"}").unwrap());
        assert!(t.segments().contains(&Segment::Column("job".into())));
    }
}
