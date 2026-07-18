//! `rml:StarMap` expansion (ADR-0029): parser-side desugaring of an RDF-star
//! quoted-triple subject into the existing R2RML IR — no new `sf-core` IR
//! variant, no downstream/executor/SQL change (ADR-0029 decision 1).
//!
//! A subject or object map carrying `rml:starMap [ rml:quotedTriplesMap <QTM> ;
//! rml:nonAssertedTriplesMap <QTM> ]` expands, via [`expand_star_map`] (the
//! shared core both positions call), to a compiler-derived synthetic-id
//! `TermMap` plus 4 `rdf:PropositionForm` basic-encoding predicate-object maps
//! (ADR-0029 §B, Rules R1/R2). In subject position (`r2rml.rs::parse_subject_map`)
//! those 4 POMs inject directly onto the enclosing `TriplesMap`, since its
//! subject IS the synthetic id. In object position
//! (`r2rml.rs::parse_object_map`) the synthetic id is only the object of one
//! triple, so the 4 POMs instead populate a standalone synthetic `TriplesMap`.
//! Single-level (non-nested) `StarMap`s only — nesting and predicate-position
//! `StarMap`s are rejected at load time (R3/R4).

use super::*;
use sf_core::ir::Segment;

/// Whether the quoted triples map referenced by an `rml:starMap` should also be
/// emitted as its own ordinary, independently-matchable `TriplesMap`
/// (ADR-0029 §B.3): asserted (`rml:quotedTriplesMap` only) or non-asserted
/// (`rml:nonAssertedTriplesMap` also present on the same StarMap node).
pub(super) struct StarAssertion {
    pub(super) quoted_id: String,
    pub(super) asserted: bool,
}

/// Expand an `rml:starMap` node into the synthetic-id subject [`TermMap`], the
/// 4 basic-encoding [`PredicateObjectMap`]s to inject, and the asserted/
/// non-asserted bookkeeping for `rml:quotedTriplesMap` (ADR-0029 §B).
pub(super) fn expand_star_map(
    g: &Graph,
    star_node: &NamedOrBlankNode,
    outer_source: &LogicalSource,
) -> Result<(TermMap, Vec<PredicateObjectMap>, StarAssertion)> {
    let quoted_ref = g
        .object(star_node, RML_QUOTED_TRIPLES_MAP)
        .ok_or_else(|| Error::Mapping("rml:starMap has no rml:quotedTriplesMap".to_owned()))?;
    let quoted_node = as_resource(quoted_ref)?;
    let quoted_id = node_id(&quoted_node);
    let asserted = g.object(star_node, RML_NON_ASSERTED_TRIPLES_MAP).is_none();

    // R3 (subject side): the quoted triples map's own subject must not itself
    // be a StarMap — only single-level nesting is supported (ADR-0029 §D).
    let quoted_subject_map_node = g
        .object(&quoted_node, RR_SUBJECT_MAP)
        .map(as_resource)
        .transpose()?;
    if let Some(n) = &quoted_subject_map_node {
        if g.object(n, RML_STAR_MAP).is_some() {
            return Err(Error::Mapping(format!(
                "nested rml:starMap: quoted triples map {quoted_id}'s own subject is itself a StarMap (only single-level StarMap nesting is supported)"
            )));
        }
    }

    // Cross-source join deferred to a later version (ADR-0029 §D): the quoted
    // triples map must share the outer map's logical source.
    let quoted_source = parse_logical_source(g, &quoted_node)?;
    if !same_source(&quoted_source, outer_source) {
        return Err(Error::Mapping(format!(
            "quoted triples map {quoted_id} has a different logical source than the outer triples map (cross-source StarMap joins are not supported)"
        )));
    }

    let subject_term = if let Some(n) = &quoted_subject_map_node {
        parse_term_map(g, n, Position::Subject)?
    } else if let Some(constant) = g.object(&quoted_node, RR_SUBJECT) {
        TermMap::Constant(constant.clone())
    } else {
        return Err(Error::Mapping(format!(
            "quoted triples map {quoted_id} has no rr:subjectMap / rr:subject"
        )));
    };

    // R2RML §6 / ADR-0029: exactly one predicate-object map, one predicate,
    // one object — a "non-single-spo" quoted triples map is rejected.
    let pom_nodes: Vec<NamedOrBlankNode> = g
        .objects(&quoted_node, RR_PREDICATE_OBJECT_MAP)
        .map(as_resource)
        .collect::<Result<_>>()?;
    if pom_nodes.len() != 1 {
        return Err(Error::Mapping(format!(
            "quoted triples map {quoted_id} must have exactly one predicate-object map (non-single-spo, found {})",
            pom_nodes.len()
        )));
    }
    let pom_node = &pom_nodes[0];

    let pm_nodes: Vec<NamedOrBlankNode> = g
        .objects(pom_node, RR_PREDICATE_MAP)
        .map(as_resource)
        .collect::<Result<_>>()?;
    let predicate_constants: Vec<&Term> = g.objects(pom_node, RR_PREDICATE).collect();
    if pm_nodes.len() + predicate_constants.len() != 1 {
        return Err(Error::Mapping(format!(
            "quoted triples map {quoted_id} must have exactly one predicate (non-single-spo)"
        )));
    }
    let predicate_term = if let Some(pm_node) = pm_nodes.first() {
        parse_term_map(g, pm_node, Position::Predicate)?
    } else {
        TermMap::Constant(predicate_constants[0].clone())
    };
    // The predicate must be compile-time known (rr:predicate / a constant
    // rr:predicateMap) — the synthetic-id template bakes it in as fixed text,
    // never as a per-row column (ADR-0029 §B.1).
    let predicate_iri = match &predicate_term {
        TermMap::Constant(Term::NamedNode(n)) => n.as_str().to_owned(),
        _ => {
            return Err(Error::Mapping(format!(
                "quoted triples map {quoted_id}'s predicate must be a constant IRI (rr:predicate) for RDF-star synthetic-id derivation"
            )))
        }
    };

    let om_nodes: Vec<NamedOrBlankNode> = g
        .objects(pom_node, RR_OBJECT_MAP)
        .map(as_resource)
        .collect::<Result<_>>()?;
    let object_constants: Vec<&Term> = g.objects(pom_node, RR_OBJECT).collect();
    if om_nodes.len() + object_constants.len() != 1 {
        return Err(Error::Mapping(format!(
            "quoted triples map {quoted_id} must have exactly one object (non-single-spo)"
        )));
    }
    let object_term = if let Some(om_node) = om_nodes.first() {
        // R3 (object side): the quoted triples map's own object must not
        // itself be a StarMap.
        if g.object(om_node, RML_STAR_MAP).is_some() {
            return Err(Error::Mapping(format!(
                "nested rml:starMap: quoted triples map {quoted_id}'s own object is itself a StarMap (only single-level StarMap nesting is supported)"
            )));
        }
        if g.object(om_node, RR_PARENT_TRIPLES_MAP).is_some() {
            return Err(Error::Mapping(format!(
                "quoted triples map {quoted_id}'s object is a referencing object map (rr:parentTriplesMap), not supported for RDF-star quoting"
            )));
        }
        parse_term_map(g, om_node, Position::Object)?
    } else {
        TermMap::Constant(object_constants[0].clone())
    };

    let synthetic_term = TermMap::Template(
        synthetic_template(&predicate_iri, &subject_term, &object_term)?,
        TermSpec::iri(),
    );

    let injected = vec![
        proposition_form_pom(
            RDF_TYPE,
            ObjectMap::Term(TermMap::Constant(Term::NamedNode(
                NamedNode::new(RDF_PROPOSITION_FORM).expect("valid constant IRI"),
            ))),
        ),
        proposition_form_pom(RDF_PROPOSITION_FORM_SUBJECT, ObjectMap::Term(subject_term)),
        proposition_form_pom(
            RDF_PROPOSITION_FORM_PREDICATE,
            ObjectMap::Term(predicate_term),
        ),
        proposition_form_pom(RDF_PROPOSITION_FORM_OBJECT, ObjectMap::Term(object_term)),
    ];

    Ok((
        synthetic_term,
        injected,
        StarAssertion {
            quoted_id,
            asserted,
        },
    ))
}

/// One basic-encoding predicate-object map: a constant predicate paired with
/// `object` (the subject comes from the enclosing triples map's synthetic-id
/// subject map, shared by every predicate-object map on that triples map).
fn proposition_form_pom(predicate: &str, object: ObjectMap) -> PredicateObjectMap {
    PredicateObjectMap {
        predicates: vec![TermMap::Constant(Term::NamedNode(
            NamedNode::new(predicate).expect("valid constant IRI"),
        ))],
        objects: vec![object],
        graphs: Vec::new(),
    }
}

/// Do two logical sources refer to the same table/query (ADR-0029 §D:
/// cross-source StarMap joins are out of scope for v1)?
fn same_source(a: &LogicalSource, b: &LogicalSource) -> bool {
    match (a, b) {
        (LogicalSource::Table(x), LogicalSource::Table(y)) => x == y,
        (LogicalSource::Query(x), LogicalSource::Query(y)) => x == y,
        _ => false,
    }
}

/// The deterministic synthetic-id template (ADR-0029 §B.1/R2): a fixed
/// `urn:sf-star:` prefix plus a compile-time slug of the quoted triple's
/// predicate IRI, followed by every `{column}` reference drawn from the quoted
/// triples map's own subject and object term maps, each bounded by `|`
/// delimiters on both sides.
///
/// `|` is outside R2RML's `iunreserved` set (R2RML §7.3), so it is always
/// percent-encoded when it appears inside an actual column value — a literal
/// `|` in the template can therefore never collide with an encoded column
/// value, the same soundness argument `Template::is_injective`'s doc comment
/// already relies on. No runtime hash function is used (ADR-0029 Consequences,
/// Neutral clause; backend portability) — determinism instead falls straight
/// out of the fact that the same underlying row always expands to the same
/// string, every time the mapping is compiled or the query re-run.
fn synthetic_template(
    predicate_iri: &str,
    subject: &TermMap,
    object: &TermMap,
) -> Result<Template> {
    let mut segments = vec![Segment::Literal(
        format!("urn:sf-star:{}|", slug(predicate_iri)).into(),
    )];
    for column in columns_of(subject).into_iter().chain(columns_of(object)) {
        segments.push(Segment::Column(column));
        segments.push(Segment::Literal("|".into()));
    }
    Template::from_segments(segments)
}

/// Every `{column}` reference a term map is built from, in template order —
/// empty for `rr:constant` (nothing varies per row), a single name for
/// `rr:column`, all placeholders for `rr:template`.
fn columns_of(term: &TermMap) -> Vec<Box<str>> {
    match term {
        TermMap::Constant(_) => Vec::new(),
        TermMap::Column(column, _) => vec![column.clone()],
        TermMap::Template(template, _) => template
            .segments()
            .iter()
            .filter_map(|s| match s {
                Segment::Column(c) => Some(c.clone()),
                Segment::Literal(_) => None,
            })
            .collect(),
    }
}

/// A deterministic, compile-time slug of a predicate IRI for the synthetic-id
/// template's fixed prefix (ADR-0029 Consequences/Neutral: the exact scheme is
/// an implementation detail, not architectural — any stable derivation
/// satisfies R2). Every maximal run of non-alphanumeric characters collapses to
/// a single `_`; leading/trailing `_` is trimmed.
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
