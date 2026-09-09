//! The R2RML-only mapping intermediate representation (ADR-0002, ADR-0003 R1).
//!
//! The IR models exactly what R2RML needs — no RML-Core reference-formulation /
//! iterator / heterogeneous-source generality (ADR-0002 removed that). It is the
//! single source of truth for what triples each source row produces, and it must
//! be **walkable as a rewrite target** by the virtualiser (ADR-0003).
//!
//! Constants (predicate / `rr:class` type / `rr:datatype` IRIs) are pre-built
//! into `oxrdf` nodes once, so term generation can emit them by reference; every
//! `rr:template` is pre-compiled to a [`Segment`] list so there is no per-row
//! placeholder scan (ADR-0006).

use oxrdf::{NamedNode, Term};

use crate::{Error, Result, Row};
pub mod encoding;
use encoding::percent_encode_iri;

/// One R2RML `rr:TriplesMap` (§6): one logical table, one subject, N
/// predicate-object maps. Every row of the logical table is processed once.
#[derive(Debug, Clone)]
pub struct TriplesMap {
    /// The TriplesMap's identifier (its IRI in the mapping graph).
    pub id: String,
    pub source: LogicalSource,
    pub subject: SubjectMap,
    pub predicate_object_maps: Vec<PredicateObjectMap>,
}

/// The logical table feeding a TriplesMap (R2RML §5): a base table/view, or an
/// R2RML view defined by an SQL query. R2RML-only — no reference formulation.
#[derive(Debug, Clone)]
pub enum LogicalSource {
    /// `rr:tableName` — a base table or SQL view.
    Table(String),
    /// `rr:sqlQuery` — an R2RML view (evaluated by the source RDBMS).
    Query(String),
}

/// The subject map (R2RML §6.1) plus its `rr:class` shortcuts and graph maps.
#[derive(Debug, Clone)]
pub struct SubjectMap {
    /// Generates the subject term (must be an IRI or blank node).
    pub term: TermMap,
    /// `rr:class` values — pre-built type IRIs emitted as `rdf:type` triples
    /// (constant, emitted by reference at term-gen time).
    pub classes: Vec<NamedNode>,
    /// `rr:graphMap` — empty means the default graph.
    pub graphs: Vec<TermMap>,
}

/// A predicate-object map (R2RML §6.3): pairs predicate maps with object maps.
/// Multiple predicates × objects produce a Cartesian product of triples.
#[derive(Debug, Clone)]
pub struct PredicateObjectMap {
    pub predicates: Vec<TermMap>,
    pub objects: Vec<ObjectMap>,
    /// `rr:graphMap` — empty means the subject map's / default graph.
    pub graphs: Vec<TermMap>,
}

/// An object map either produces a term directly, or joins to a parent
/// TriplesMap (R2RML §8).
#[derive(Debug, Clone)]
pub enum ObjectMap {
    Term(TermMap),
    Ref(RefObjectMap),
}

/// A referencing object map (R2RML §8): the object is the *subject* of a parent
/// TriplesMap, reached by an equi-join. Empty `joins` ⇒ a self-join.
#[derive(Debug, Clone)]
pub struct RefObjectMap {
    /// `rr:parentTriplesMap` — referenced by its [`TriplesMap::id`].
    pub parent_triples_map: String,
    pub joins: Vec<Join>,
}

/// One `rr:joinCondition` (child column = parent column); multiple are ANDed.
#[derive(Debug, Clone)]
pub struct Join {
    pub child: String,
    pub parent: String,
}

/// A term map: how a source row becomes an RDF term (R2RML §6). The three
/// generation mechanisms are distinct variants so invalid states (e.g. a
/// constant carrying an `rr:datatype`) are unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermMap {
    /// `rr:constant` — a fixed RDF term, pre-built once and emitted by reference.
    Constant(Term),
    /// `rr:column` — the value of a named column.
    Column(Box<str>, TermSpec),
    /// `rr:template` — a pre-compiled string template with `{column}` slots.
    Template(Template, TermSpec),
}

/// The term-type plus literal modifiers shared by `rr:column` / `rr:template`
/// term maps (R2RML §6.2/§7). For `rr:Literal`, at most one of `datatype` /
/// `language` is set; `datatype` is pre-built for by-reference emission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermSpec {
    pub term_type: TermType,
    pub datatype: Option<NamedNode>,
    pub language: Option<Box<str>>,
    /// The processor's output base, distinct from Turtle's document base, kept
    /// for column and dynamically resolved template IRIs. After expansion, a
    /// non-absolute value receives this verbatim prefix (R2RML §11.2). Proven
    /// static templates bake the base in; non-IRI term maps leave this `None`.
    pub base: Option<Box<str>>,
}

impl TermSpec {
    /// R2RML §11.2: an explicit datatype equal to the resolved source type is
    /// not an override. This applies to column literals, not template values.
    pub fn uses_natural_type(&self, code: Option<crate::datatype::XsdTypeCode>) -> bool {
        self.term_type == TermType::Literal
            && self.language.is_none()
            && self
                .datatype
                .as_ref()
                .is_none_or(|datatype| code.is_some_and(|code| datatype.as_ref() == code.iri()))
    }

    /// An IRI-valued term map.
    pub fn iri() -> Self {
        Self {
            term_type: TermType::Iri,
            datatype: None,
            language: None,
            base: None,
        }
    }

    /// A blank-node-valued term map.
    pub fn blank_node() -> Self {
        Self {
            term_type: TermType::BlankNode,
            datatype: None,
            language: None,
            base: None,
        }
    }

    /// A plain (`xsd:string`) literal term map — no datatype / language.
    pub fn plain_literal() -> Self {
        Self {
            term_type: TermType::Literal,
            datatype: None,
            language: None,
            base: None,
        }
    }

    /// A typed literal term map with the given (pre-built) `rr:datatype` IRI.
    pub fn typed_literal(datatype: NamedNode) -> Self {
        Self {
            term_type: TermType::Literal,
            datatype: Some(datatype),
            language: None,
            base: None,
        }
    }

    /// A language-tagged literal term map (`rr:language`).
    pub fn lang_literal(language: impl Into<Box<str>>) -> Self {
        Self {
            term_type: TermType::Literal,
            datatype: None,
            language: Some(language.into()),
            base: None,
        }
    }

    /// Attach the processor output base for per-row IRI finalization.
    pub fn with_base(mut self, base: impl Into<Box<str>>) -> Self {
        self.base = Some(base.into());
        self
    }
}

/// The RDF term type a term map produces (R2RML §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermType {
    Iri,
    BlankNode,
    Literal,
}

/// A pre-compiled `rr:template` (R2RML §7.3): an alternating list of fixed text
/// and `{column}` references, so expansion is a straight walk with no per-row
/// placeholder scan (ADR-0006).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    segments: Vec<Segment>,
}

/// One piece of a compiled [`Template`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// Fixed text (emitted by reference, zero-copy).
    Literal(Box<str>),
    /// A `{column}` reference.
    Column(Box<str>),
}

impl Template {
    /// Pre-compile an `rr:template` string into a segment list.
    ///
    /// `{` / `}` delimit a column name; a literal brace or backslash is escaped
    /// with a backslash (`\{`, `\}`, `\\`), per R2RML §7.3.
    pub fn parse(template: &str) -> Result<Self> {
        let mut segments = Vec::new();
        let mut text = String::new();
        let mut chars = template.chars();

        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    // Outside braces, only `\{ \} \\` are valid escapes.
                    match chars.next() {
                        Some(e @ ('{' | '}' | '\\')) => text.push(e),
                        other => {
                            return Err(Error::Mapping(format!(
                                "invalid template escape '\\{}'",
                                other.map_or_else(|| "<eof>".to_owned(), |c| c.to_string())
                            )))
                        }
                    }
                }
                '{' => {
                    if !text.is_empty() {
                        segments.push(Segment::Literal(std::mem::take(&mut text).into()));
                    }
                    let column = read_column(&mut chars)?;
                    segments.push(Segment::Column(column.into()));
                }
                '}' => {
                    return Err(Error::Mapping(
                        "unescaped '}' outside a template column".to_owned(),
                    ))
                }
                _ => text.push(c),
            }
        }
        if !text.is_empty() {
            segments.push(Segment::Literal(text.into()));
        }
        if segments.is_empty() {
            return Err(Error::Mapping("empty rr:template".to_owned()));
        }
        Ok(Self { segments })
    }

    /// Build a template directly from a pre-built [`Segment`] list — the path used
    /// by the auto-generated R2RML of Direct Mapping (`sf-mapping`), which assembles
    /// segments programmatically rather than parsing an `rr:template` string.
    /// Fails only on an empty list (an `rr:template` must produce something).
    pub fn from_segments(segments: Vec<Segment>) -> Result<Self> {
        if segments.is_empty() {
            return Err(Error::Mapping("empty rr:template".to_owned()));
        }
        Ok(Self { segments })
    }

    /// The compiled segments (for inspection / by the rewriter).
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Prove that distinct column tuples produce distinct encoded expansions.
    ///
    /// Every adjacent column pair needs a fixed character absent from encoded
    /// values. ASCII unreserved characters, Unicode and `%` can all occur in
    /// those values, so separators such as `-`, `é` and `%2F` prove nothing.
    /// A reserved ASCII delimiter such as `/` does prove separation. This is a
    /// conservative proof: false means no proof, not necessarily a collision.
    /// For literal/blank-node templates (no encoding), callers must additionally
    /// require at most one column slot (`cascade::distinct_removal`).
    pub fn is_injective(&self) -> bool {
        let mut prev_col = false;
        for seg in &self.segments {
            match seg {
                Segment::Column(_) => {
                    if prev_col {
                        return false; // adjacent column slots — not injective
                    }
                    prev_col = true;
                }
                Segment::Literal(text)
                    if text.bytes().any(|byte| {
                        byte.is_ascii()
                            && !byte.is_ascii_alphanumeric()
                            && !matches!(byte, b'-' | b'.' | b'_' | b'~' | b'%')
                    }) =>
                {
                    prev_col = false; // delimiter cannot appear in encoded values
                }
                Segment::Literal(_) => {} // no proven delimiter
            }
        }
        true
    }

    /// Expand the template for `row` into `out` (which is **cleared** first),
    /// percent-encoding column values when `encode_iri` is set (R2RML §7.3 only
    /// percent-encodes for IRI term types).
    ///
    /// Returns `false` — emitting nothing usable — when any referenced column is
    /// SQL `NULL`/absent (R2RML §6.4/§11: no value ⇒ no term). No allocation:
    /// fixed segments are pushed by reference, column values are written through.
    pub fn expand<R: Row + ?Sized>(&self, row: &R, encode_iri: bool, out: &mut String) -> bool {
        out.clear();
        for segment in &self.segments {
            match segment {
                Segment::Literal(text) => out.push_str(text),
                Segment::Column(column) => match row.value(column) {
                    None => return false,
                    Some(value) if encode_iri => percent_encode_iri(value, out),
                    Some(value) => out.push_str(value),
                },
            }
        }
        true
    }
}

/// Read a column name after a `{`, consuming the closing `}`.
fn read_column(chars: &mut std::str::Chars<'_>) -> Result<String> {
    let mut column = String::new();
    for c in chars.by_ref() {
        match c {
            '}' => {
                if column.is_empty() {
                    return Err(Error::Mapping("empty {} column in rr:template".to_owned()));
                }
                return Ok(column);
            }
            '{' => return Err(Error::Mapping("nested '{' in rr:template".to_owned())),
            _ => column.push(c),
        }
    }
    Err(Error::Mapping("unterminated '{' in rr:template".to_owned()))
}

#[cfg(test)]
mod encoding_tests;
#[cfg(test)]
mod injectivity_tests;
#[cfg(test)]
mod tests {
    use super::*;

    fn lit(s: &str) -> Segment {
        Segment::Literal(s.into())
    }
    fn col(s: &str) -> Segment {
        Segment::Column(s.into())
    }

    #[test]
    fn template_precompiles_to_segments() {
        let t = Template::parse("http://ex.org/{dept}/{id}").unwrap();
        assert_eq!(
            t.segments(),
            &[lit("http://ex.org/"), col("dept"), lit("/"), col("id")]
        );
    }

    #[test]
    fn template_leading_and_adjacent_columns() {
        let t = Template::parse("{a}{b}tail").unwrap();
        assert_eq!(t.segments(), &[col("a"), col("b"), lit("tail")]);
    }

    #[test]
    fn template_escapes_braces_and_backslash() {
        let t = Template::parse("a\\{b\\}\\\\c").unwrap();
        assert_eq!(t.segments(), &[lit("a{b}\\c")]);
    }

    #[test]
    fn template_rejects_malformed() {
        assert!(Template::parse("{unterminated").is_err());
        assert!(Template::parse("nested {a{b}}").is_err());
        assert!(Template::parse("bare } brace").is_err());
        assert!(Template::parse("empty {} slot").is_err());
        assert!(Template::parse("bad \\escape").is_err());
        assert!(Template::parse("").is_err());
    }

    #[test]
    fn expand_writes_through_and_percent_encodes_iris() {
        let t = Template::parse("http://ex.org/{name}").unwrap();
        let row: &[(&str, Option<&str>)] = &[("name", Some("a b/c"))];
        let mut buf = String::new();
        assert!(t.expand(row, true, &mut buf));
        assert_eq!(buf, "http://ex.org/a%20b%2Fc");

        // unreserved characters pass through untouched
        let row2: &[(&str, Option<&str>)] = &[("name", Some("A-z.0_9~"))];
        assert!(t.expand(row2, true, &mut buf));
        assert_eq!(buf, "http://ex.org/A-z.0_9~");
    }

    #[test]
    fn expand_literal_template_does_not_encode() {
        let t = Template::parse("{a}/{b}").unwrap();
        let row: &[(&str, Option<&str>)] = &[("a", Some("x y")), ("b", Some("z/w"))];
        let mut buf = String::new();
        assert!(t.expand(row, false, &mut buf));
        assert_eq!(buf, "x y/z/w");
    }

    #[test]
    fn expand_returns_false_on_null_column() {
        let t = Template::parse("http://ex.org/{id}").unwrap();
        let row: &[(&str, Option<&str>)] = &[("id", None)];
        let mut buf = String::new();
        assert!(!t.expand(row, true, &mut buf));
    }

    // -- Template::is_injective — DISTINCT-elimination soundness gate --------

    #[test]
    fn is_injective_true_when_columns_separated_by_nonempty_literal() {
        // "http://ex/{a}/{b}" — the '/' separator makes collisions impossible:
        // (a="1", b="23") and (a="12", b="3") expand to different strings
        // because the separator can never appear verbatim inside a percent-
        // encoded column value.
        let t = Template::parse("http://ex/{a}/{b}").unwrap();
        assert!(t.is_injective());
    }

    #[test]
    fn is_injective_false_when_adjacent_columns_have_no_separator() {
        // "http://ex/{a}{b}" — no separator between `a` and `b`: (a="1",
        // b="23") and (a="12", b="3") both expand to "http://ex/123".
        let t = Template::parse("http://ex/{a}{b}").unwrap();
        assert!(!t.is_injective());

        // Collision witness, concretely, via `expand`.
        let mut buf1 = String::new();
        let row1: &[(&str, Option<&str>)] = &[("a", Some("1")), ("b", Some("23"))];
        assert!(t.expand(row1, false, &mut buf1));

        let mut buf2 = String::new();
        let row2: &[(&str, Option<&str>)] = &[("a", Some("12")), ("b", Some("3"))];
        assert!(t.expand(row2, false, &mut buf2));

        assert_eq!(
            buf1, buf2,
            "distinct column tuples collided to the same string"
        );
    }

    #[test]
    fn is_injective_true_for_single_column_template() {
        // A single column slot has no adjacent-column pair to collide with,
        // regardless of surrounding literals (or their absence).
        let t = Template::parse("http://ex/{id}").unwrap();
        assert!(t.is_injective());

        let bare = Template::parse("{id}").unwrap();
        assert!(bare.is_injective());
    }

    #[test]
    fn is_injective_true_for_constant_only_template() {
        // No column slots at all — vacuously injective (there is only ever
        // one output string, so distinct tuples of zero columns can't exist).
        let t = Template::parse("http://ex/constant").unwrap();
        assert!(t.is_injective());
    }

    #[test]
    fn is_injective_false_when_only_empty_literal_between_columns() {
        // An empty Literal segment between two Columns must not reset
        // adjacency — it is indistinguishable from no separator at all.
        let t = Template::from_segments(vec![col("a"), lit(""), col("b")]).unwrap();
        assert!(!t.is_injective());
    }

    #[test]
    fn is_injective_true_for_three_columns_each_separated() {
        let t = Template::parse("{a}/{b}/{c}").unwrap();
        assert!(t.is_injective());
    }
}
