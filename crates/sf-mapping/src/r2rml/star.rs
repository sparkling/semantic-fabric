//! `rml:StarMap` expansion (ADR-0032 D1, superseding ADR-0029 §B): parser-side
//! desugaring of an RDF-star quoted-triple subject/object into the existing
//! R2RML IR — no new `sf-core` IR variant, no downstream/executor/SQL change.
//!
//! D1's role split replaces v1's single synthetic id with two distinct
//! deterministic id families, minted by [`ids::proposition_template`] /
//! [`ids::reifier_template`]:
//! * a **proposition id** (`urn:sf-star:pf:…`) standing in for the triple
//!   term itself — a pure function of the quoted (s,p,o) shape, so every
//!   reference to the same shape (from anywhere) shares the same id
//!   (Semantics §5's injective `IT`, realized structurally);
//! * a **reifier id** (`urn:sf-star:r:…`) — one per star-map *declaration*
//!   (keyed on the declaring triples map's own id), so two different star
//!   maps quoting the same shape mint two distinct reifiers of the one
//!   proposition (Concepts §1.5's reifier multiplicity).
//!
//! In **subject position** (`r2rml.rs::parse_subject_map`), the outer
//! triples map's subject becomes the reifier id; one injected `rdf:reifies`
//! predicate-object map points it at the proposition id; the author's own
//! POMs stay on the outer map (annotations ride the reifier, the native
//! shape). In **object position** (`r2rml.rs::parse_object_map`), the object
//! is the proposition id directly — unchanged from v1's shape, no reifier
//! (there is nothing there to reify).
//!
//! Either way, the 4 `rdf:PropositionForm` basic-encoding predicate-object
//! maps (whose subject IS the proposition id, never the reifier) populate a
//! standalone [`TriplesMap`] keyed on the quoted map's own id
//! ([`description_map_id`]) — shared by every reference to the same quoted
//! shape, deduplicated by `r2rml.rs::parse_r2rml`'s existing
//! `standalone_ids_seen` bookkeeping exactly like v1's object-position
//! carrier.
//!
//! **Nesting** (D1 item 5) is now supported on the object side, recursively
//! and to arbitrary depth: [`quote_shape`] expands the innermost quote
//! first, and the outer proposition id splices the inner one's own segments
//! in (see [`ids::proposition_template`]'s doc comment). Subject-side
//! nesting stays a load-time error (D5) — spec-impossible, not a scope
//! choice: RDF 1.2 Concepts §3.1, triple terms are object-position-only.
//!
//! **Cross-source** quotes (D4) are supported via `rr:joinCondition` declared
//! directly on the `rml:starMap` node: [`crossing_reference`] compiles the
//! crossing reference as an ordinary `RefObjectMap` join onto the
//! description map (which itself always compiles on the *quoted* source) —
//! reusing the join engine's existing cross-source capability verbatim.
//! Cross-source without a declared join stays a load-time error.
//!
//! `rml:reifierMap` (subject position only, a documented extension) lets
//! authors override the default deterministic reifier term map — see
//! [`reifier_term`].

use super::*;

mod ids;

/// Whether the quoted triples map referenced by an `rml:starMap` should also
/// be emitted as its own ordinary, independently-matchable `TriplesMap`:
/// asserted (`rml:quotedTriplesMap` only) or non-asserted
/// (`rml:nonAssertedTriplesMap` also present on the same StarMap node).
pub(super) struct StarAssertion {
    pub(super) quoted_id: String,
    pub(super) asserted: bool,
}

/// The result of expanding a subject-position `rml:starMap`: the reifier
/// term that becomes the outer triples map's own subject, the one injected
/// `rdf:reifies` predicate-object map, and every standalone description map
/// / asserted-non-asserted bookkeeping entry touched while expanding the
/// quoted shape (including any nested quotes on its object side).
pub(super) struct SubjectExpansion {
    pub(super) reifier_term: TermMap,
    pub(super) reifies_pom: PredicateObjectMap,
    pub(super) description_maps: Vec<TriplesMap>,
    pub(super) assertions: Vec<StarAssertion>,
}

/// The result of expanding an object-position `rml:starMap`: the object map
/// itself (the proposition id, inline or cross-source-joined), plus the same
/// description-map / bookkeeping threading as [`SubjectExpansion`].
pub(super) struct ObjectExpansion {
    pub(super) object: ObjectMap,
    pub(super) description_maps: Vec<TriplesMap>,
    pub(super) assertions: Vec<StarAssertion>,
}

/// Expand a subject-position `rml:starMap` (ADR-0032 D1). `outer_tm_id` is
/// the enclosing triples map's own id (the reifier id's declaring-map key);
/// `outer_source` is its logical source (the D4 cross-source baseline).
pub(super) fn expand_star_map_subject(
    g: &Graph,
    star_node: &NamedOrBlankNode,
    outer_tm_id: &str,
    outer_source: &LogicalSource,
) -> Result<SubjectExpansion> {
    let quoted_node = quoted_triples_map_node(g, star_node)?;
    let shape = quote_shape(g, &quoted_node)?;
    let reifies_object = crossing_reference(g, star_node, &shape, outer_source)?;
    let reifier = reifier_term(g, star_node, outer_tm_id, &shape)?;

    let mut assertions = shape.nested_assertions;
    assertions.push(StarAssertion {
        quoted_id: shape.quoted_id,
        asserted: is_asserted(g, star_node),
    });

    let reifies_pom = PredicateObjectMap {
        predicates: vec![TermMap::Constant(Term::NamedNode(
            NamedNode::new(RDF_REIFIES).expect("valid constant IRI"),
        ))],
        objects: vec![reifies_object],
        graphs: Vec::new(),
    };

    Ok(SubjectExpansion {
        reifier_term: reifier,
        reifies_pom,
        description_maps: shape.description_maps,
        assertions,
    })
}

/// Expand an object-position `rml:starMap` (ADR-0029 shape, ADR-0032 D1
/// naming). No reifier is minted here — `rml:reifierMap` is rejected.
pub(super) fn expand_star_map_object(
    g: &Graph,
    star_node: &NamedOrBlankNode,
    outer_source: &LogicalSource,
) -> Result<ObjectExpansion> {
    if g.object(star_node, RML_REIFIER_MAP).is_some() {
        return Err(Error::Mapping(format!(
            "rml:reifierMap is not allowed on an object-position rml:starMap ({}): no reifier is minted for an object-position quote (RDF 1.2 Concepts §3.1 — there is nothing there to reify)",
            node_id(star_node)
        )));
    }
    let quoted_node = quoted_triples_map_node(g, star_node)?;
    let shape = quote_shape(g, &quoted_node)?;
    let object = crossing_reference(g, star_node, &shape, outer_source)?;

    let mut assertions = shape.nested_assertions;
    assertions.push(StarAssertion {
        quoted_id: shape.quoted_id,
        asserted: is_asserted(g, star_node),
    });

    Ok(ObjectExpansion {
        object,
        description_maps: shape.description_maps,
        assertions,
    })
}

/// A quoted triple's own (s,p,o) shape, fully expanded: its proposition id,
/// the standalone description [`TriplesMap`]s needed to carry the 4
/// basic-encoding POMs (this level's own is always `.last()`; any nested
/// object-side quote's come first — bottom-up), and every `StarAssertion`
/// contributed by a *nested* star map encountered along the way (this
/// shape's own assertion is the caller's responsibility — only the caller
/// holds the `rml:starMap` node the asserted/non-asserted marker lives on).
struct QuotedShape {
    pfid: Template,
    quoted_id: String,
    quoted_source: LogicalSource,
    description_maps: Vec<TriplesMap>,
    nested_assertions: Vec<StarAssertion>,
}

/// Expand `quoted_node`'s own (s,p,o) shape (ADR-0032 D1). Recurses into an
/// object-side `rml:starMap` (D1 item 5, arbitrary depth); rejects a
/// subject-side one (D5, RDF 1.2 Concepts §3.1) and a non-single-spo shape
/// (R2RML §6), exactly as v1 did — cross-source-ness of any nested quote is
/// resolved against *this* level's own `quoted_source`, not the top-level
/// caller's, matching D4's "outer map" being the immediately enclosing quote.
fn quote_shape(g: &Graph, quoted_node: &NamedOrBlankNode) -> Result<QuotedShape> {
    let quoted_id = node_id(quoted_node);
    let quoted_source = parse_logical_source(g, quoted_node)?;

    // D5 (subject side): the quoted triples map's own subject must not
    // itself be a StarMap — RDF 1.2 Concepts §3.1: triple terms are
    // object-position-only, so a triple-term subject is not RDF (impossible
    // in the data model itself, not a v1 scope choice).
    let quoted_subject_map_node = g
        .object(quoted_node, RR_SUBJECT_MAP)
        .map(as_resource)
        .transpose()?;
    if let Some(n) = &quoted_subject_map_node {
        if g.object(n, RML_STAR_MAP).is_some() {
            return Err(Error::Mapping(format!(
                "rml:starMap in subject position: quoted triples map {quoted_id}'s own subject is itself a StarMap — RDF 1.2 Concepts §3.1: triple terms are object-position-only, a triple-term subject is not RDF"
            )));
        }
    }

    let subject_term = if let Some(n) = &quoted_subject_map_node {
        parse_term_map(g, n, Position::Subject)?
    } else if let Some(constant) = g.object(quoted_node, RR_SUBJECT) {
        TermMap::Constant(constant.clone())
    } else {
        return Err(Error::Mapping(format!(
            "quoted triples map {quoted_id} has no rr:subjectMap / rr:subject"
        )));
    };

    // R2RML §6 / ADR-0029: exactly one predicate-object map, one predicate,
    // one object — a "non-single-spo" quoted triples map is rejected.
    let pom_nodes: Vec<NamedOrBlankNode> = g
        .objects(quoted_node, RR_PREDICATE_OBJECT_MAP)
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
    // rr:predicateMap) — the id templates bake it in as fixed text, never as
    // a per-row column (ADR-0029 §B.1, unchanged by ADR-0032).
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

    // The object is either an ordinary term, or itself a nested star map
    // (D1 item 5: recurse bottom-up) — `object_for_id` feeds
    // `ids::proposition_template` directly (nested or not, it is already a
    // `TermMap`, so no separate splice-vs-flatten choice is needed);
    // `object_pom_value` is what the propositionFormObject POM actually
    // carries (inline term, or a D4 cross-source Ref).
    let (object_for_id, _, object_pom_value, mut description_maps, nested_assertions) =
        if let Some(om_node) = om_nodes.first() {
            if g.object(om_node, RR_PARENT_TRIPLES_MAP).is_some() {
                return Err(Error::Mapping(format!(
                    "quoted triples map {quoted_id}'s object is a referencing object map (rr:parentTriplesMap), not supported for RDF-star quoting"
                )));
            }
            if let Some(inner_star) = g.object(om_node, RML_STAR_MAP) {
                let inner_star_node = as_resource(inner_star)?;
                let inner_quoted_node = quoted_triples_map_node(g, &inner_star_node)?;
                let inner_shape = quote_shape(g, &inner_quoted_node)?;
                let inner_pom_value =
                    crossing_reference(g, &inner_star_node, &inner_shape, &quoted_source)?;
                let mut assertions = inner_shape.nested_assertions;
                assertions.push(StarAssertion {
                    quoted_id: inner_shape.quoted_id.clone(),
                    asserted: is_asserted(g, &inner_star_node),
                });
                let inner_term = TermMap::Template(inner_shape.pfid.clone(), TermSpec::iri());
                (
                    inner_term,
                    true,
                    inner_pom_value,
                    inner_shape.description_maps,
                    assertions,
                )
            } else {
                let t = parse_term_map(g, om_node, Position::Object)?;
                (t.clone(), false, ObjectMap::Term(t), Vec::new(), Vec::new())
            }
        } else {
            let t = TermMap::Constant(object_constants[0].clone());
            (t.clone(), false, ObjectMap::Term(t), Vec::new(), Vec::new())
        };

    let pfid = ids::proposition_template(&predicate_iri, &subject_term, &object_for_id)?;

    let description_poms = vec![
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
        proposition_form_pom(RDF_PROPOSITION_FORM_OBJECT, object_pom_value),
    ];
    description_maps.push(TriplesMap {
        id: description_map_id(&quoted_id),
        source: quoted_source.clone(),
        subject: SubjectMap {
            term: TermMap::Template(pfid.clone(), TermSpec::iri()),
            classes: Vec::new(),
            graphs: Vec::new(),
        },
        predicate_object_maps: description_poms,
    });

    Ok(QuotedShape {
        pfid,
        quoted_id,
        quoted_source,
        description_maps,
        nested_assertions,
    })
}

/// The crossing reference to `shape`'s description map — used for a subject-
/// position `rdf:reifies` object, an object-position direct object, and each
/// object-side nesting level's own `propositionFormObject` (ADR-0032 D4):
/// same source as `outer_source` ⇒ the proposition id inline; different
/// source ⇒ an ordinary `RefObjectMap` join onto the description map, using
/// `rr:joinCondition`s declared directly on `star_node`. No join declared
/// under a source mismatch is a load-time error naming both sources.
fn crossing_reference(
    g: &Graph,
    star_node: &NamedOrBlankNode,
    shape: &QuotedShape,
    outer_source: &LogicalSource,
) -> Result<ObjectMap> {
    if same_source(&shape.quoted_source, outer_source) {
        return Ok(ObjectMap::Term(TermMap::Template(
            shape.pfid.clone(),
            TermSpec::iri(),
        )));
    }
    let joins = parse_join_conditions(g, star_node)?;
    if joins.is_empty() {
        return Err(Error::Mapping(format!(
            "quoted triples map {} has a different logical source ({:?}) than its outer triples map ({:?}), and no rr:joinCondition was declared on the rml:starMap (cross-source StarMap joins require rr:joinCondition, ADR-0032 D4)",
            shape.quoted_id, shape.quoted_source, outer_source
        )));
    }
    Ok(ObjectMap::Ref(RefObjectMap {
        parent_triples_map: description_map_id(&shape.quoted_id),
        joins,
    }))
}

/// The reifier term map for a subject-position star map: `rml:reifierMap` if
/// present (must be IRI- or blank-node-valued — R2RML §7.4's own subject-map
/// constraint, reused via `Position::Subject`, plus an explicit check for the
/// `rr:constant` literal case `build_term_spec` never sees), otherwise the
/// default deterministic template keyed on `outer_tm_id` (ADR-0032 D1).
fn reifier_term(
    g: &Graph,
    star_node: &NamedOrBlankNode,
    outer_tm_id: &str,
    shape: &QuotedShape,
) -> Result<TermMap> {
    if let Some(rm) = g.object(star_node, RML_REIFIER_MAP) {
        let node = as_resource(rm)?;
        let term = parse_term_map(g, &node, Position::Subject)?;
        if let TermMap::Constant(Term::Literal(_)) = &term {
            return Err(Error::Mapping(format!(
                "rml:reifierMap {} must be IRI- or blank-node-valued (found rr:constant of a literal)",
                node_id(&node)
            )));
        }
        return Ok(term);
    }
    Ok(TermMap::Template(
        ids::reifier_template(outer_tm_id, &shape.pfid)?,
        TermSpec::iri(),
    ))
}

/// `star_node`'s `rml:quotedTriplesMap` object, as a resource.
fn quoted_triples_map_node(g: &Graph, star_node: &NamedOrBlankNode) -> Result<NamedOrBlankNode> {
    let quoted_ref = g
        .object(star_node, RML_QUOTED_TRIPLES_MAP)
        .ok_or_else(|| Error::Mapping("rml:starMap has no rml:quotedTriplesMap".to_owned()))?;
    as_resource(quoted_ref)
}

/// Whether `star_node` marks its quoted map asserted (no
/// `rml:nonAssertedTriplesMap`) — the default.
fn is_asserted(g: &Graph, star_node: &NamedOrBlankNode) -> bool {
    g.object(star_node, RML_NON_ASSERTED_TRIPLES_MAP).is_none()
}

/// The standalone description map's id for a quoted map (ADR-0032 D1: keyed
/// on the quoted map's own id, never the referencing outer map's — so every
/// reference to the same quoted shape, subject or object position, nested or
/// not, resolves to the very same carrier, deduplicated by
/// `r2rml.rs::parse_r2rml`'s existing `standalone_ids_seen` bookkeeping).
fn description_map_id(quoted_id: &str) -> String {
    format!("urn:sf-star:desc:{quoted_id}")
}

/// One basic-encoding predicate-object map: a constant predicate paired with
/// `object` (the subject comes from the enclosing triples map's proposition-
/// id subject map, shared by every predicate-object map on that triples map).
fn proposition_form_pom(predicate: &str, object: ObjectMap) -> PredicateObjectMap {
    PredicateObjectMap {
        predicates: vec![TermMap::Constant(Term::NamedNode(
            NamedNode::new(predicate).expect("valid constant IRI"),
        ))],
        objects: vec![object],
        graphs: Vec::new(),
    }
}

/// Do two logical sources refer to the same table/query (ADR-0032 D4: same
/// source ⇒ inline; different ⇒ a declared-join `RefObjectMap`)?
fn same_source(a: &LogicalSource, b: &LogicalSource) -> bool {
    match (a, b) {
        (LogicalSource::Table(x), LogicalSource::Table(y)) => x == y,
        (LogicalSource::Query(x), LogicalSource::Query(y)) => x == y,
        _ => false,
    }
}
