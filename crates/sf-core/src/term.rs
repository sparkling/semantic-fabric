//! RDF term generation from a source row + a term map (ADR-0003 R3), under the
//! ADR-0006 allocation discipline.
//!
//! [`generate_into`] is the write-through path used by the streaming CONSTRUCT /
//! SELECT pipeline: it returns a [`GenTerm`] that **borrows** its data from the
//! mapping IR (constants), the source row (column values), or a caller-owned
//! scratch buffer (template expansions) — so no owned `Term` is allocated per
//! row. [`generate`] is the owned convenience wrapper for callers (or result
//! serialisers) that need an `oxrdf::Term`.
//!
//! Datatype **canonicalisation** (R2RML §10) is deliberately *not* applied here:
//! it needs the column's catalog-resolved SQL type, which the [`Row`] boundary
//! intentionally does not carry. The executor canonicalises raw driver values
//! through the [`crate::datatype::canonical_lexical`] chokepoint (which lives
//! here in `sf-core`, so the mapping exists exactly once — ADR-0003 R3 /
//! ADR-0015). This function emits an explicit `rr:datatype` / `rr:language`
//! verbatim, and a column/template's value as its lexical form.

use oxrdf::{BlankNodeRef, LiteralRef, NamedNodeRef, Term};

use crate::ir::{TermMap, TermSpec, TermType};
use crate::{Error, Result, Row};

/// A generated RDF term, borrowing from the mapping IR, the source row, or the
/// caller's scratch buffer. Zero per-row allocation (ADR-0006).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenTerm<'a> {
    NamedNode(NamedNodeRef<'a>),
    BlankNode(BlankNodeRef<'a>),
    Literal(LiteralRef<'a>),
}

impl GenTerm<'_> {
    /// Copy into an owned `oxrdf::Term` (the SELECT / serialiser convenience).
    pub fn into_owned(self) -> Term {
        match self {
            GenTerm::NamedNode(n) => n.into(),
            GenTerm::BlankNode(b) => b.into(),
            GenTerm::Literal(l) => l.into(),
        }
    }
}

/// Generate the term for `term_map` against `row`, writing any derived lexical
/// form through `buf` and returning a borrowed [`GenTerm`].
///
/// Returns `Ok(None)` when a referenced column is SQL `NULL`/absent — no value,
/// so no term, so no triple (R2RML §11). `buf` is the caller's reusable scratch
/// buffer; templates and relative column IRIs write through it.
pub fn generate_into<'a, R: Row + ?Sized>(
    term_map: &'a TermMap,
    row: &'a R,
    buf: &'a mut String,
) -> Result<Option<GenTerm<'a>>> {
    match term_map {
        TermMap::Constant(term) => constant(term).map(Some),
        TermMap::Column(column, spec) => match row.value(column) {
            None => Ok(None),
            // R2RML §11.2 prefixes relative IRI values with the processor base
            // and validates per row; other term types pass through.
            Some(value) if spec.term_type == TermType::Iri => {
                column_iri(value, spec.base.as_deref(), buf).map(Some)
            }
            Some(value) => Ok(Some(from_value(value, spec))),
        },
        TermMap::Template(template, spec) => {
            if template.expand(row, spec.term_type == TermType::Iri, buf) {
                if spec.term_type == TermType::Iri {
                    if let Some(base) = spec.base.as_deref() {
                        finalize_template_iri(buf, base)?;
                    }
                }
                Ok(Some(from_value(buf.as_str(), spec)))
            } else {
                Ok(None)
            }
        }
    }
}

/// Finish an expanded template in place: absolute first, otherwise a verbatim
/// processor prefix. Substitutions have already been encoded exactly once.
fn finalize_template_iri(buf: &mut String, base: &str) -> Result<()> {
    if oxiri::Iri::parse(buf.as_str()).is_ok() {
        return Ok(());
    }
    if oxiri::Iri::parse(base).is_ok() {
        buf.insert_str(0, base);
        if oxiri::Iri::parse(buf.as_str()).is_ok() {
            return Ok(());
        }
    }
    Err(Error::Term(
        "rr:template expansion is invalid both directly and with the processor base prefix".into(),
    ))
}

/// Generate an IRI term from an `rr:column` value (R2RML §11.2): a valid absolute
/// IRI is used as is; otherwise prepend the processor base verbatim and validate.
/// This is not RFC3986 URL resolution: dot segments and leading slashes remain.
/// If neither yields a valid IRI it is a **data error**. The prefixed form is
/// written through `buf` so the absolute-IRI fast path stays allocation-free.
pub fn column_iri<'a>(
    value: &'a str,
    base: Option<&str>,
    buf: &'a mut String,
) -> Result<GenTerm<'a>> {
    if oxiri::Iri::parse(value).is_ok() {
        return Ok(GenTerm::NamedNode(NamedNodeRef::new_unchecked(value)));
    }
    if let Some(base) = base {
        if oxiri::Iri::parse(base).is_ok() {
            buf.clear();
            buf.push_str(base);
            buf.push_str(value);
            if oxiri::Iri::parse(buf.as_str()).is_ok() {
                return Ok(GenTerm::NamedNode(NamedNodeRef::new_unchecked(
                    buf.as_str(),
                )));
            }
        }
    }
    Err(Error::Term(format!(
        "rr:column IRI value {value:?} is invalid both directly and with the processor base prefix"
    )))
}

/// Owned convenience over [`generate_into`] (allocates; for SELECT / serialisers
/// that require an `oxrdf::Term`).
pub fn generate<R: Row + ?Sized>(term_map: &TermMap, row: &R) -> Result<Option<Term>> {
    let mut buf = String::new();
    Ok(generate_into(term_map, row, &mut buf)?.map(GenTerm::into_owned))
}

/// Borrow a pre-built constant term out of the IR (by reference, zero-copy).
fn constant(term: &Term) -> Result<GenTerm<'_>> {
    match term {
        Term::NamedNode(n) => Ok(GenTerm::NamedNode(n.as_ref())),
        Term::BlankNode(b) => Ok(GenTerm::BlankNode(b.as_ref())),
        Term::Literal(l) => Ok(GenTerm::Literal(l.as_ref())),
        Term::Triple(_) => Err(Error::Term(
            "triple-term constant is not an R2RML term map value".to_owned(),
        )),
    }
}

/// Build a term from a derived string value (`value` borrows the row or `buf`),
/// applying the term type and — for literals — the explicit datatype/language.
///
/// Column and late-template IRIs have already been validated; static template
/// recipes carry the parser's grammar proof (ADR-0006).
fn from_value<'a>(value: &'a str, spec: &'a TermSpec) -> GenTerm<'a> {
    match spec.term_type {
        TermType::Iri => GenTerm::NamedNode(NamedNodeRef::new_unchecked(value)),
        TermType::BlankNode => GenTerm::BlankNode(BlankNodeRef::new_unchecked(value)),
        TermType::Literal => {
            let literal = if let Some(language) = &spec.language {
                LiteralRef::new_language_tagged_literal_unchecked(value, language)
            } else if let Some(datatype) = &spec.datatype {
                LiteralRef::new_typed_literal(value, datatype.as_ref())
            } else {
                LiteralRef::new_simple_literal(value)
            };
            GenTerm::Literal(literal)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Template, TermSpec};
    use oxrdf::vocab::xsd;
    use oxrdf::{BlankNode, Literal, NamedNode};

    #[test]
    fn late_template_iri_finalizes_once_after_expansion() {
        let tm = TermMap::Template(
            Template::parse("{v}://host/{id}").unwrap(),
            TermSpec::iri().with_base("http://base/"),
        );
        for (scheme, expected) in [
            ("http", "http://host/a%2Fb"),
            ("1", "http://base/1://host/a%2Fb"),
        ] {
            let row = [("v", Some(scheme)), ("id", Some("a/b"))];
            assert_eq!(
                generate(&tm, &row[..]).unwrap().unwrap().to_string(),
                format!("<{expected}>")
            );
        }
        let tm = TermMap::Template(
            Template::parse("bad value/{v}").unwrap(),
            TermSpec::iri().with_base("http://base/"),
        );
        assert!(generate(&tm, &[("v", Some("x"))][..]).is_err());
    }

    fn owned(term_map: &TermMap, row: &[(&str, Option<&str>)]) -> Option<Term> {
        generate(term_map, row).unwrap()
    }

    #[test]
    fn constant_iri_is_emitted_by_reference() {
        let node = NamedNode::new_unchecked("http://ex.org/p");
        let tm = TermMap::Constant(Term::NamedNode(node.clone()));
        let row: &[(&str, Option<&str>)] = &[];
        let mut buf = String::new();

        let g = generate_into(&tm, row, &mut buf).unwrap().unwrap();
        // The returned ref borrows the IR's node (zero-copy).
        assert_eq!(g, GenTerm::NamedNode(node.as_ref()));
        let owned = g.into_owned();
        // Consuming `g` releases the buffer; the constant path never wrote it.
        assert!(
            buf.is_empty(),
            "constant path must not touch the scratch buffer"
        );
        assert_eq!(owned, Term::NamedNode(node));
    }

    #[test]
    fn constant_literal_is_supported() {
        let lit = Literal::new_typed_literal("42", xsd::INTEGER);
        let tm = TermMap::Constant(Term::Literal(lit.clone()));
        let row: &[(&str, Option<&str>)] = &[];
        assert_eq!(owned(&tm, row), Some(Term::Literal(lit)));
    }

    #[test]
    fn column_iri_uses_value_as_iri() {
        let tm = TermMap::Column("u".into(), TermSpec::iri());
        let row: &[(&str, Option<&str>)] = &[("u", Some("http://ex.org/x"))];
        assert_eq!(
            owned(&tm, row),
            Some(Term::NamedNode(NamedNode::new_unchecked("http://ex.org/x")))
        );
    }

    #[test]
    fn column_iri_relative_value_resolves_against_base() {
        // R2RML §11.2: prefix a relative column IRI with the processor base.
        let tm = TermMap::Column(
            "u".into(),
            TermSpec::iri().with_base("http://example.com/base/"),
        );
        let row: &[(&str, Option<&str>)] = &[("u", Some("Carlos"))];
        assert_eq!(
            owned(&tm, row),
            Some(Term::NamedNode(NamedNode::new_unchecked(
                "http://example.com/base/Carlos"
            )))
        );
    }

    #[test]
    fn column_iri_prefixes_processor_base_without_url_normalization() {
        for (base, value, expected) in [
            ("http://ex/base/", "../x", "http://ex/base/../x"),
            ("http://ex/base/", "/x", "http://ex/base//x"),
            ("http://ex/base/", "//other/x", "http://ex/base///other/x"),
            ("http://ex/base/", "?q", "http://ex/base/?q"),
            ("http://ex/base/?old", "?new", "http://ex/base/?old?new"),
            ("http://ex/base/#old", "x", "http://ex/base/#oldx"),
            ("http://ex/base/", "", "http://ex/base/"),
            ("http://ex/base", "x", "http://ex/basex"),
            ("http://ex/base/", "é/%ab", "http://ex/base/é/%ab"),
            (
                "http://ex/base/",
                "http://other/a/../x",
                "http://other/a/../x",
            ),
        ] {
            let mut scratch = String::from("old-buffer");
            assert_eq!(
                column_iri(value, Some(base), &mut scratch).unwrap(),
                GenTerm::NamedNode(NamedNodeRef::new(expected).unwrap())
            );
        }
        let mut scratch = String::new();
        assert!(column_iri("#new", Some("http://ex/#old"), &mut scratch).is_err());
        assert!(column_iri("bad value", Some("http://ex/"), &mut scratch).is_err());
    }

    #[test]
    fn column_iri_absolute_value_passes_through_base() {
        // An absolute IRI is used verbatim, base or no base.
        let tm = TermMap::Column(
            "u".into(),
            TermSpec::iri().with_base("http://example.com/base/"),
        );
        let row: &[(&str, Option<&str>)] = &[("u", Some("http://ex.org/ns#Jhon"))];
        assert_eq!(
            owned(&tm, row),
            Some(Term::NamedNode(NamedNode::new_unchecked(
                "http://ex.org/ns#Jhon"
            )))
        );
    }

    #[test]
    fn column_iri_invalid_value_is_a_data_error() {
        // Invalid both directly and after prefixing (a space is illegal in an
        // IRI) means a data error (R2RML §11.2).
        let tm = TermMap::Column(
            "u".into(),
            TermSpec::iri().with_base("http://example.com/base/"),
        );
        let row: &[(&str, Option<&str>)] = &[("u", Some("Juan Daniel"))];
        assert!(generate(&tm, row).is_err());
    }

    #[test]
    fn column_blank_node() {
        let tm = TermMap::Column("b".into(), TermSpec::blank_node());
        let row: &[(&str, Option<&str>)] = &[("b", Some("n1"))];
        assert_eq!(
            owned(&tm, row),
            Some(Term::BlankNode(BlankNode::new_unchecked("n1")))
        );
    }

    #[test]
    fn column_plain_literal() {
        let tm = TermMap::Column("name".into(), TermSpec::plain_literal());
        let row: &[(&str, Option<&str>)] = &[("name", Some("Ada"))];
        assert_eq!(
            owned(&tm, row),
            Some(Term::Literal(Literal::new_simple_literal("Ada")))
        );
    }

    #[test]
    fn column_typed_literal_emits_datatype_verbatim() {
        let tm = TermMap::Column(
            "age".into(),
            TermSpec::typed_literal(NamedNode::from(xsd::INTEGER)),
        );
        // No canonicalisation here: the explicit datatype + value pass through.
        let row: &[(&str, Option<&str>)] = &[("age", Some("007"))];
        assert_eq!(
            owned(&tm, row),
            Some(Term::Literal(Literal::new_typed_literal(
                "007",
                xsd::INTEGER
            )))
        );
    }

    #[test]
    fn column_language_literal() {
        let tm = TermMap::Column("label".into(), TermSpec::lang_literal("en"));
        let row: &[(&str, Option<&str>)] = &[("label", Some("colour"))];
        let expected = Literal::new_language_tagged_literal("colour", "en").unwrap();
        assert_eq!(owned(&tm, row), Some(Term::Literal(expected)));
    }

    #[test]
    fn template_iri_expands_and_encodes() {
        let tm = TermMap::Template(
            Template::parse("http://ex.org/emp/{id}").unwrap(),
            TermSpec::iri(),
        );
        let row: &[(&str, Option<&str>)] = &[("id", Some("a b"))];
        assert_eq!(
            owned(&tm, row),
            Some(Term::NamedNode(NamedNode::new_unchecked(
                "http://ex.org/emp/a%20b"
            )))
        );
    }

    #[test]
    fn template_literal_is_not_percent_encoded() {
        let tm = TermMap::Template(
            Template::parse("{a}/{b}").unwrap(),
            TermSpec::plain_literal(),
        );
        let row: &[(&str, Option<&str>)] = &[("a", Some("x y")), ("b", Some("z"))];
        assert_eq!(
            owned(&tm, row),
            Some(Term::Literal(Literal::new_simple_literal("x y/z")))
        );
    }

    #[test]
    fn null_column_yields_no_term() {
        let tm = TermMap::Column("x".into(), TermSpec::plain_literal());
        let row: &[(&str, Option<&str>)] = &[("x", None)];
        assert_eq!(owned(&tm, row), None);
    }

    #[test]
    fn null_in_template_yields_no_term() {
        let tm = TermMap::Template(
            Template::parse("http://ex.org/{a}/{b}").unwrap(),
            TermSpec::iri(),
        );
        let row: &[(&str, Option<&str>)] = &[("a", Some("1")), ("b", None)];
        assert_eq!(owned(&tm, row), None);
    }

    #[test]
    fn triple_term_constant_is_rejected() {
        use oxrdf::Triple;
        let t = Triple::new(
            NamedNode::new_unchecked("http://ex.org/s"),
            NamedNode::new_unchecked("http://ex.org/p"),
            NamedNode::new_unchecked("http://ex.org/o"),
        );
        let tm = TermMap::Constant(Term::Triple(Box::new(t)));
        let row: &[(&str, Option<&str>)] = &[];
        assert!(generate(&tm, row).is_err());
    }
}
