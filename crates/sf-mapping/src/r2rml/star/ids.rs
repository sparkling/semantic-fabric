//! Deterministic id-template construction for RDF-star mapping emission
//! (ADR-0032 D1): the proposition-id family (`urn:sf-star:pf:…`, a pure
//! function of the quoted shape) and the reifier-id family
//! (`urn:sf-star:r:…`, keyed on the declaring triples map so distinct star
//! maps quoting the same shape mint distinct reifiers). Kept apart from the
//! `Graph`-walking orchestration in the parent module — these are pure
//! functions over already-parsed [`TermMap`]/[`Template`] values.
//!
//! The proposition id must be an INJECTIVE function of the quoted (s,p,o)
//! shape (RDF 1.2 Semantics §5's requirement on `IT`: id-equality must imply
//! decoded-triple-equality), so [`proposition_template`] folds in each of
//! the subject's and object's FULL rendered lexical form via [`push_term`] —
//! not just its referenced columns:
//! * `rr:column`: the column's value alone (there is no template
//!   prefix/suffix to lose), tagged with its term kind/datatype/language.
//! * `rr:template`: the template's OWN segment list — literal prefixes and
//!   suffixes *and* columns — spliced in verbatim (also tagged), so two
//!   templates over the same column(s) with different fixed text can never
//!   collide; this is the exact same segment list [`Template::expand`] uses
//!   to materialize the REAL term elsewhere in the same description map, so
//!   the id inherits that rendering's own injectivity for free. It is also
//!   how a nested object's own inner proposition id (itself a
//!   `TermMap::Template`, ADR-0032 D1 item 5) ends up embedded in the outer
//!   id, keeping the inner shape's identity intact.
//! * `rr:constant`: never expanded at all elsewhere, so this module renders
//!   the term's own canonical form and percent-encodes it in directly.
//!
//! `|` is outside R2RML's `iunreserved` set (R2RML §7.3), so it is always
//! percent-encoded when it appears inside an actual column value — a literal
//! `|` in a template's own fixed text can therefore never collide with an
//! encoded column value, the same soundness argument `Template::is_injective`'s
//! doc comment already relies on. Neither can a `|` inside an `rr:constant`'s
//! value or a kind/datatype/language tag: both are run through this module's
//! own [`percent_encode`] before being wrapped in a `|` delimiter, so every
//! `|` surviving into the finished id is one of this module's own
//! delimiters, never smuggled in from mapping-author-controlled content. No
//! runtime hash function is used; determinism instead falls straight out of
//! the fact that the same underlying row always expands to the same string,
//! every time the mapping is compiled or the query re-run.

use sf_core::ir::{Segment, Template, TermMap, TermSpec, TermType};
use sf_core::Result;

/// The deterministic proposition-id template (ADR-0032 D1/R2): a fixed
/// `urn:sf-star:pf:` prefix carrying a compile-time slug of the quoted
/// triple's predicate IRI, followed by the subject's and then the object's
/// full rendered lexical form (see the module doc and [`push_term`]) — an
/// injective function of the quoted (s,p,o) shape, so every reference to the
/// same shape shares the same id, and two DIFFERENT shapes (even ones
/// sharing a predicate and column arity) never collide. A nested object
/// (`object` is itself the inner proposition id template of a recursively-
/// expanded object-side `rml:starMap`, ADR-0032 D1 item 5) needs no special
/// case here: it is already a `TermMap::Template`, so [`push_term`]'s normal
/// `rr:template` treatment splices its segments in verbatim.
pub(super) fn proposition_template(
    predicate_iri: &str,
    subject: &TermMap,
    object: &TermMap,
) -> Result<Template> {
    let mut segments = vec![Segment::Literal(
        format!("urn:sf-star:pf:{}|", slug(predicate_iri)).into(),
    )];
    push_term(&mut segments, subject);
    push_term(&mut segments, object);
    Template::from_segments(segments)
}

/// The deterministic reifier-id template (ADR-0032 D1): a fixed
/// `urn:sf-star:r:` prefix plus a compile-time slug of the *declaring*
/// triples map's own id (so distinct star-map declarations quoting the same
/// shape mint distinct reifiers), followed by the same quoted-shape columns
/// as `pfid` — row-deterministic, in the same `|`-delimited form.
pub(super) fn reifier_template(outer_tm_id: &str, pfid: &Template) -> Result<Template> {
    let mut segments = vec![Segment::Literal(
        format!("urn:sf-star:r:{}|", slug(outer_tm_id)).into(),
    )];
    for column in columns_of_template(pfid) {
        segments.push(Segment::Column(column));
        segments.push(Segment::Literal("|".into()));
    }
    Template::from_segments(segments)
}

/// Append the segments capturing `term`'s FULL rendered lexical form to
/// `segments` (see the module doc for why each variant is handled the way it
/// is). Every component ends in a `Segment::Literal("|")`, so it is always
/// separated from whatever segment follows it by a non-empty literal
/// ([`Template::is_injective`]'s own adjacency requirement).
fn push_term(segments: &mut Vec<Segment>, term: &TermMap) {
    match term {
        TermMap::Constant(value) => segments.push(Segment::Literal(
            format!("{}|", percent_encode(&value.to_string())).into(),
        )),
        TermMap::Column(column, spec) => {
            segments.push(Segment::Literal(format!("{}|", kind_tag(spec)).into()));
            segments.push(Segment::Column(column.clone()));
            segments.push(Segment::Literal("|".into()));
        }
        TermMap::Template(template, spec) => {
            segments.push(Segment::Literal(format!("{}|", kind_tag(spec)).into()));
            segments.extend(template.segments().iter().cloned());
            segments.push(Segment::Literal("|".into()));
        }
    }
}

/// A compile-time-fixed marker for `spec`'s term kind (plus datatype/language
/// for a literal) — a [`TermSpec`] never varies per row, so this folds
/// straight into the id as fixed text. Without it, two `rr:column`/
/// `rr:template` components that render the same per-row text but produce
/// DIFFERENT kinds of RDF term (e.g. an IRI-valued column vs. a same-text
/// literal, or two literals with different datatypes) would mint the same id
/// despite denoting different terms.
fn kind_tag(spec: &TermSpec) -> String {
    match spec.term_type {
        TermType::Iri => "I".to_owned(),
        TermType::BlankNode => "B".to_owned(),
        TermType::Literal => match (&spec.datatype, &spec.language) {
            (Some(dt), _) => format!("L^^{}", percent_encode(dt.as_str())),
            (None, Some(lang)) => format!("L@{}", percent_encode(lang)),
            (None, None) => "L".to_owned(),
        },
    }
}

/// A local percent-encoding helper for the compile-time-fixed text this
/// module bakes into an id (`rr:constant` renderings, kind/datatype/language
/// tags) — every *row*-derived column value is already percent-encoded by
/// [`Template::expand`] itself (`encode_iri=true`, since the id template it
/// fills in is always IRI-typed). Mirrors `sf-core`'s own private
/// `percent_encode_iri` (`sf-core/src/ir.rs`) byte-for-byte; duplicated
/// rather than exposed across the crate boundary, since this fix's scope is
/// `sf-mapping` only.
fn percent_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii() {
            let byte = ch as u8;
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                out.push(ch);
            } else {
                out.push('%');
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 0x0f) as usize] as char);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// [`push_term`]'s `rr:template` arm, exposed directly for callers (the
/// reifier template) that already hold a bare [`Template`] rather than a
/// [`TermMap`].
pub(super) fn columns_of_template(template: &Template) -> Vec<Box<str>> {
    template
        .segments()
        .iter()
        .filter_map(|s| match s {
            Segment::Column(c) => Some(c.clone()),
            Segment::Literal(_) => None,
        })
        .collect()
}

/// A deterministic, compile-time slug for a template's fixed prefix
/// (predicate IRI for the proposition id, declaring-map id for the reifier
/// id): every maximal run of non-alphanumeric characters collapses to a
/// single `_`; leading/trailing `_` is trimmed.
fn slug(iri: &str) -> String {
    let mut out = String::with_capacity(iri.len());
    let mut last_was_sep = false;
    for c in iri.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_was_sep = false;
        } else if !last_was_sep {
            out.push('_');
            last_was_sep = true;
        }
    }
    out.trim_matches('_').to_owned()
}
