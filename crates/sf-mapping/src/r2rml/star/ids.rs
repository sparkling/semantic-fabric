//! Deterministic id-template construction for RDF-star mapping emission
//! (ADR-0032 D1): the proposition-id family (`urn:sf-star:pf:…`, a pure
//! function of the quoted shape) and the reifier-id family
//! (`urn:sf-star:r:…`, keyed on the declaring triples map so distinct star
//! maps quoting the same shape mint distinct reifiers). Kept apart from the
//! `Graph`-walking orchestration in the parent module — these are pure
//! functions over already-parsed [`TermMap`]/[`Template`] values.
//!
//! `|` is outside R2RML's `iunreserved` set (R2RML §7.3), so it is always
//! percent-encoded when it appears inside an actual column value — a literal
//! `|` in a template can therefore never collide with an encoded column
//! value, the same soundness argument `Template::is_injective`'s doc comment
//! already relies on. No runtime hash function is used; determinism instead
//! falls straight out of the fact that the same underlying row always
//! expands to the same string, every time the mapping is compiled or the
//! query re-run.

use sf_core::ir::{Segment, Template, TermMap};
use sf_core::Result;

/// The deterministic proposition-id template (ADR-0032 D1/R2): a fixed
/// `urn:sf-star:pf:` prefix plus a compile-time slug of the quoted triple's
/// predicate IRI, followed by every `{column}` reference drawn from the
/// quoted triples map's own subject term, each bounded by `|` delimiters —
/// exactly `synthetic_template`'s v1 algorithm, renamed and re-prefixed.
///
/// The object component gets one of two treatments:
/// - ordinary (`object_is_nested == false`): the same column-flattening
///   treatment as the subject, matching v1 exactly.
/// - nested (`object_is_nested == true`, `object` is itself the inner
///   proposition id template of a recursively-expanded object-side
///   `rml:starMap`, ADR-0032 D1 item 5): the inner template's own segments
///   (literal marker text *and* columns) are spliced in verbatim rather than
///   flattened to bare column names, so the inner shape's own identity stays
///   embedded in the outer id — two different inner shapes sharing leaf
///   column names still cannot collide.
pub(super) fn proposition_template(
    predicate_iri: &str,
    subject: &TermMap,
    object: &TermMap,
    object_is_nested: bool,
) -> Result<Template> {
    let mut segments = vec![Segment::Literal(
        format!("urn:sf-star:pf:{}|", slug(predicate_iri)).into(),
    )];
    push_columns(&mut segments, subject);
    if object_is_nested {
        if let TermMap::Template(inner, _) = object {
            segments.extend(inner.segments().iter().cloned());
        }
    } else {
        push_columns(&mut segments, object);
    }
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

/// Append `|{column}|`-delimited segments for every `{column}` reference `term`
/// is built from — nothing for `rr:constant`, one for `rr:column`, every
/// placeholder (in template order) for `rr:template`.
fn push_columns(segments: &mut Vec<Segment>, term: &TermMap) {
    for column in columns_of(term) {
        segments.push(Segment::Column(column));
        segments.push(Segment::Literal("|".into()));
    }
}

/// Every `{column}` reference a term map is built from, in template order —
/// empty for `rr:constant`, a single name for `rr:column`, all placeholders
/// for `rr:template` (including a recursively-nested proposition-id template,
/// whose columns are exactly the leaf columns feeding it transitively).
fn columns_of(term: &TermMap) -> Vec<Box<str>> {
    match term {
        TermMap::Constant(_) => Vec::new(),
        TermMap::Column(column, _) => vec![column.clone()],
        TermMap::Template(template, _) => columns_of_template(template),
    }
}

/// [`columns_of`]'s `rr:template` arm, exposed directly for callers (the
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
