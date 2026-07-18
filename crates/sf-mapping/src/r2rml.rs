//! Parse an R2RML mapping document (Turtle, RDF 1.2 via `oxttl`) into the
//! `sf-core` mapping IR — done exactly once (ADR-0003 R1; the `sf-mapping` row of
//! ADR-0006; `oxttl` `rdf-12` per ADR-0019).
//!
//! The mapping graph `M` is small and intensional (ADR-0003 / ADR-0004), so this
//! loads the whole document into a subject-indexed graph and walks the R2RML
//! vocabulary onto the IR. RDF terms stay `oxrdf` types end to end (ADR-0003 R2);
//! the IR they populate is the single rewrite target for the virtualiser.

use std::collections::{HashMap, HashSet};

use oxttl::TurtleParser;

use sf_core::ir::{
    Join, LogicalSource, ObjectMap, PredicateObjectMap, RefObjectMap, SubjectMap, Template,
    TermMap, TermSpec, TermType, TriplesMap,
};
use sf_core::{Error, NamedNode, NamedOrBlankNode, Result, Term, Triple};

mod sql;
use sql::{
    normalize_template_idents, resolve_iri_template, sql_identifier, strip_trailing_semicolon,
};

mod star;
use star::StarAssertion;

// --- R2RML vocabulary (namespace `http://www.w3.org/ns/r2rml#`, R2RML §11) ----

const RR_LOGICAL_TABLE: &str = "http://www.w3.org/ns/r2rml#logicalTable";
const RR_TABLE_NAME: &str = "http://www.w3.org/ns/r2rml#tableName";
const RR_SQL_QUERY: &str = "http://www.w3.org/ns/r2rml#sqlQuery";
const RR_SQL_VERSION: &str = "http://www.w3.org/ns/r2rml#sqlVersion";
const RR_SQL2008: &str = "http://www.w3.org/ns/r2rml#SQL2008";
const RR_SUBJECT_MAP: &str = "http://www.w3.org/ns/r2rml#subjectMap";
const RR_SUBJECT: &str = "http://www.w3.org/ns/r2rml#subject";
const RR_PREDICATE_OBJECT_MAP: &str = "http://www.w3.org/ns/r2rml#predicateObjectMap";
const RR_PREDICATE_MAP: &str = "http://www.w3.org/ns/r2rml#predicateMap";
const RR_PREDICATE: &str = "http://www.w3.org/ns/r2rml#predicate";
const RR_OBJECT_MAP: &str = "http://www.w3.org/ns/r2rml#objectMap";
const RR_OBJECT: &str = "http://www.w3.org/ns/r2rml#object";
const RR_TEMPLATE: &str = "http://www.w3.org/ns/r2rml#template";
const RR_COLUMN: &str = "http://www.w3.org/ns/r2rml#column";
const RR_CONSTANT: &str = "http://www.w3.org/ns/r2rml#constant";
const RR_TERM_TYPE: &str = "http://www.w3.org/ns/r2rml#termType";
const RR_DATATYPE: &str = "http://www.w3.org/ns/r2rml#datatype";
const RR_LANGUAGE: &str = "http://www.w3.org/ns/r2rml#language";
const RR_CLASS: &str = "http://www.w3.org/ns/r2rml#class";
const RR_GRAPH_MAP: &str = "http://www.w3.org/ns/r2rml#graphMap";
const RR_GRAPH: &str = "http://www.w3.org/ns/r2rml#graph";
const RR_PARENT_TRIPLES_MAP: &str = "http://www.w3.org/ns/r2rml#parentTriplesMap";
const RR_JOIN_CONDITION: &str = "http://www.w3.org/ns/r2rml#joinCondition";
const RR_CHILD: &str = "http://www.w3.org/ns/r2rml#child";
const RR_PARENT: &str = "http://www.w3.org/ns/r2rml#parent";

const RR_IRI: &str = "http://www.w3.org/ns/r2rml#IRI";
const RR_BLANK_NODE: &str = "http://www.w3.org/ns/r2rml#BlankNode";
const RR_LITERAL: &str = "http://www.w3.org/ns/r2rml#Literal";

// --- RML-STAR vocabulary (namespace `http://semweb.mmlab.be/ns/rml#`) — the
// first RML-namespace terms this processor parses (ADR-0029). ------------------

const RML_STAR_MAP: &str = "http://semweb.mmlab.be/ns/rml#starMap";
const RML_QUOTED_TRIPLES_MAP: &str = "http://semweb.mmlab.be/ns/rml#quotedTriplesMap";
const RML_NON_ASSERTED_TRIPLES_MAP: &str = "http://semweb.mmlab.be/ns/rml#nonAssertedTriplesMap";
/// ADR-0032 D1, documented extension: overrides the default deterministic
/// reifier term map on a subject-position `rml:starMap`.
const RML_REIFIER_MAP: &str = "http://semweb.mmlab.be/ns/rml#reifierMap";

// --- RDF 1.2 Interoperability "basic encoding" (ADR-0029 §B.2) — the compiled
// target for an `rml:StarMap`, namespace `http://www.w3.org/1999/02/22-rdf-syntax-ns#`.

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDF_PROPOSITION_FORM: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#PropositionForm";
const RDF_PROPOSITION_FORM_SUBJECT: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormSubject";
const RDF_PROPOSITION_FORM_PREDICATE: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormPredicate";
const RDF_PROPOSITION_FORM_OBJECT: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormObject";
/// RDF 1.2's native reification predicate (ADR-0032 D1); matches
/// `sf-sparql/src/star.rs`'s own copy (`oxrdf::vocab::rdf::REIFIES`) exactly.
const RDF_REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";

/// Base IRI applied to the mapping document so relative triples-map identifiers
/// (`<#TriplesMap1>`, ubiquitous in R2RML) resolve to absolute IRIs consistently;
/// a `@base` directive in the document overrides it. R2RML's own examples use
/// exactly this base.
const DEFAULT_BASE_IRI: &str = "http://example.com/base/";

/// Parse an R2RML mapping document into the shared IR (ADR-0003 R1).
///
/// Triples maps are returned sorted by identifier so the result is deterministic
/// regardless of hash-map iteration order. A triples map is any resource bearing
/// `rr:logicalTable` / `rr:subjectMap` / `rr:subject`; each must resolve to a
/// logical table and a subject map or parsing fails.
///
/// An `rml:starMap` subject (ADR-0032 D1) is expanded in place into a reifier-id
/// subject plus one injected `rdf:reifies` predicate-object map, with the 4
/// basic-encoding predicate-object maps moved onto a standalone description
/// map; the quoted triples map it references is suppressed from the result
/// when every `rml:starMap` referencing it marks it `rml:nonAssertedTriplesMap`.
pub fn parse_r2rml(turtle: &str) -> Result<Vec<TriplesMap>> {
    let graph = Graph::load(turtle)?;

    let mut subjects: Vec<(String, &NamedOrBlankNode)> = graph
        .spo
        .keys()
        .filter(|&s| is_triples_map(&graph, s))
        .map(|s| (node_id(s), s))
        .collect();
    subjects.sort_by(|a, b| a.0.cmp(&b.0));

    let mut maps = Vec::with_capacity(subjects.len());
    // ADR-0029 §B.3: a quoted triples map is suppressed only if every
    // `rml:starMap` referencing it marks it non-asserted — an explicit
    // assertion anywhere (a plain occurrence in the doc doesn't count; only
    // another StarMap's bare `rml:quotedTriplesMap`) wins.
    let mut asserted_ids: HashSet<String> = HashSet::new();
    let mut non_asserted_ids: HashSet<String> = HashSet::new();
    // Every `rml:starMap` (ADR-0032 D1; both positions, and every nesting
    // level — see `star::quote_shape`) carries a standalone description
    // TriplesMap for its 4 basic-encoding predicate-object maps — collected
    // here and appended after the outer maps, deduplicated by id (keyed on
    // the quoted map's own id, so two different star maps quoting the same
    // shape share one carrier).
    let mut standalone_maps: Vec<TriplesMap> = Vec::new();
    let mut standalone_ids_seen: HashSet<String> = HashSet::new();
    for (_, subject) in subjects {
        let (map, stars, tm_standalone) = parse_triples_map(&graph, subject)?;
        for assertion in stars {
            if assertion.asserted {
                asserted_ids.insert(assertion.quoted_id);
            } else {
                non_asserted_ids.insert(assertion.quoted_id);
            }
        }
        maps.push(map);
        for standalone in tm_standalone {
            if standalone_ids_seen.insert(standalone.id.clone()) {
                standalone_maps.push(standalone);
            }
        }
    }
    maps.extend(standalone_maps);
    maps.retain(|m| asserted_ids.contains(&m.id) || !non_asserted_ids.contains(&m.id));
    // Re-sort: the standalone carriers were appended after the (already
    // sorted) outer maps, so the doc-comment invariant ("sorted by
    // identifier") needs restoring here.
    maps.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(maps)
}

/// A resource is a triples map if it carries a logical table or a subject map
/// (the constant `rr:subject` shortcut counts as a subject map). Subject-map,
/// predicate-object-map, logical-table and join-condition nodes never carry
/// these predicates, so they are not mistaken for triples maps.
fn is_triples_map(g: &Graph, s: &NamedOrBlankNode) -> bool {
    g.object(s, RR_LOGICAL_TABLE).is_some()
        || g.object(s, RR_SUBJECT_MAP).is_some()
        || g.object(s, RR_SUBJECT).is_some()
}

fn parse_triples_map(
    g: &Graph,
    tm: &NamedOrBlankNode,
) -> Result<(TriplesMap, Vec<StarAssertion>, Vec<TriplesMap>)> {
    let source = parse_logical_source(g, tm)?;
    let outer_id = node_id(tm);
    let (subject, injected_poms, subject_standalone, subject_star) =
        parse_subject_map(g, tm, &source)?;
    let pom_nodes: Vec<NamedOrBlankNode> = g
        .objects(tm, RR_PREDICATE_OBJECT_MAP)
        .map(as_resource)
        .collect::<Result<_>>()?;
    let mut predicate_object_maps = Vec::with_capacity(pom_nodes.len() + injected_poms.len());
    let mut star_assertions = subject_star;
    let mut standalone_maps = subject_standalone;
    for pom in &pom_nodes {
        let (pom_ir, pom_standalone, pom_stars) = parse_predicate_object_map(g, pom, &source)?;
        predicate_object_maps.push(pom_ir);
        standalone_maps.extend(pom_standalone);
        star_assertions.extend(pom_stars);
    }
    // ADR-0029 §B: the injected star-derived POM(s) are appended after the
    // author's own — order-stable either way, appending keeps the author's
    // mapping-authored POMs first in the common (non-star) diff-review case.
    predicate_object_maps.extend(injected_poms);
    Ok((
        TriplesMap {
            id: outer_id,
            source,
            subject,
            predicate_object_maps,
        },
        star_assertions,
        standalone_maps,
    ))
}

/// `rr:logicalTable` → `rr:tableName` (base table/view) or `rr:sqlQuery` (R2RML
/// view). R2RML-only: no reference formulation (ADR-0002).
fn parse_logical_source(g: &Graph, tm: &NamedOrBlankNode) -> Result<LogicalSource> {
    let lt = g.object(tm, RR_LOGICAL_TABLE).ok_or_else(|| {
        Error::Mapping(format!(
            "triples map {} has no rr:logicalTable",
            node_id(tm)
        ))
    })?;
    let node = as_resource(lt)?;
    // R2RML §5.1: the only SQL version identifier this processor recognises is
    // `rr:SQL2008` (Core SQL 2008). Any other `rr:sqlVersion` is an undefined
    // identifier — a non-conforming mapping (e.g. `rr:SQL1979`).
    if let Some(v) = g.object(&node, RR_SQL_VERSION) {
        if !matches!(v, Term::NamedNode(n) if n.as_str() == RR_SQL2008) {
            return Err(Error::Mapping(format!(
                "unsupported rr:sqlVersion {v} (only rr:SQL2008 is recognised)"
            )));
        }
    }
    if let Some(table) = g.object(&node, RR_TABLE_NAME) {
        Ok(LogicalSource::Table(sql_identifier(lexical(table)?)))
    } else if let Some(query) = g.object(&node, RR_SQL_QUERY) {
        Ok(LogicalSource::Query(strip_trailing_semicolon(lexical(
            query,
        )?)))
    } else {
        Err(Error::Mapping(format!(
            "logical table {} has neither rr:tableName nor rr:sqlQuery",
            node_id(&node)
        )))
    }
}

/// [`parse_subject_map`]'s return: the subject map itself; any predicate-
/// object map(s) to inject onto the enclosing triples map (ADR-0032 D1: the
/// one `rdf:reifies` POM, for a star-map subject); any standalone
/// description triples maps the star expansion needed; and any star-map
/// assertion bookkeeping entries touched.
type SubjectMapResult = (
    SubjectMap,
    Vec<PredicateObjectMap>,
    Vec<TriplesMap>,
    Vec<StarAssertion>,
);

/// The subject map (`rr:subjectMap`, or the `rr:subject` constant shortcut) plus
/// its `rr:class` types and graph maps (R2RML §6.1).
///
/// When the subject-map node carries `rml:starMap` (ADR-0032 D1), the subject
/// is instead the reifier id [`TermMap`], and this also returns the one
/// injected `rdf:reifies` predicate-object map, the standalone description
/// map(s) needed for the quoted shape (and any nested quotes on its object
/// side), and every asserted/non-asserted bookkeeping entry touched.
fn parse_subject_map(
    g: &Graph,
    tm: &NamedOrBlankNode,
    outer_source: &LogicalSource,
) -> Result<SubjectMapResult> {
    // R2RML §6: a triples map has *exactly one* subject map (two is an error).
    let subject_map_count = g.objects(tm, RR_SUBJECT_MAP).count();
    let subject_count = g.objects(tm, RR_SUBJECT).count();
    if subject_map_count + subject_count > 1 {
        return Err(Error::Mapping(format!(
            "triples map {} has more than one subject map",
            node_id(tm)
        )));
    }
    let mut injected_poms = Vec::new();
    let mut standalone_maps = Vec::new();
    let mut star_assertions = Vec::new();
    let (term, carrier) = if let Some(sm) = g.object(tm, RR_SUBJECT_MAP) {
        let node = as_resource(sm)?;
        if let Some(star_map) = g.object(&node, RML_STAR_MAP) {
            let star_node = as_resource(star_map)?;
            let outer_tm_id = node_id(tm);
            let expansion =
                star::expand_star_map_subject(g, &star_node, &outer_tm_id, outer_source)?;
            injected_poms = vec![expansion.reifies_pom];
            standalone_maps = expansion.description_maps;
            star_assertions = expansion.assertions;
            (expansion.reifier_term, node)
        } else {
            let term = parse_term_map(g, &node, Position::Subject)?;
            (term, node)
        }
    } else if let Some(constant) = g.object(tm, RR_SUBJECT) {
        (TermMap::Constant(constant.clone()), tm.clone())
    } else {
        return Err(Error::Mapping(format!(
            "triples map {} has no rr:subjectMap / rr:subject",
            node_id(tm)
        )));
    };
    let classes: Vec<NamedNode> = g
        .objects(&carrier, RR_CLASS)
        .map(as_named_node)
        .collect::<Result<_>>()?;
    let graphs = parse_graph_maps(g, &carrier)?;
    Ok((
        SubjectMap {
            term,
            classes,
            graphs,
        },
        injected_poms,
        standalone_maps,
        star_assertions,
    ))
}

/// A predicate-object map: `rr:predicateMap`/`rr:predicate` paired with
/// `rr:objectMap`/`rr:object`, plus graph maps (R2RML §6.3).
///
/// `outer_source` is the enclosing triples map's own logical source —
/// needed for the D4 cross-source check an object-position `rml:starMap`
/// requires (see [`parse_object_map`]). Returns any standalone description
/// triples maps alongside the ordinary [`PredicateObjectMap`], plus every
/// `rml:starMap` assertion bookkeeping entry encountered among this
/// predicate-object map's objects.
fn parse_predicate_object_map(
    g: &Graph,
    node: &NamedOrBlankNode,
    outer_source: &LogicalSource,
) -> Result<(PredicateObjectMap, Vec<TriplesMap>, Vec<StarAssertion>)> {
    let pm_nodes: Vec<NamedOrBlankNode> = g
        .objects(node, RR_PREDICATE_MAP)
        .map(as_resource)
        .collect::<Result<_>>()?;
    let mut predicates = Vec::with_capacity(pm_nodes.len());
    for pm in &pm_nodes {
        // ADR-0032 D5 (R4): rml:starMap is rejected in predicate position —
        // RDF 1.2 Concepts §3.1, predicates are IRIs only.
        if g.object(pm, RML_STAR_MAP).is_some() {
            return Err(Error::Mapping(format!(
                "rml:starMap is not allowed in predicate position ({}): RDF 1.2 Concepts §3.1 — predicates are IRIs only",
                node_id(pm)
            )));
        }
        predicates.push(parse_term_map(g, pm, Position::Predicate)?);
    }
    for constant in g.objects(node, RR_PREDICATE) {
        predicates.push(TermMap::Constant(constant.clone()));
    }

    let om_nodes: Vec<NamedOrBlankNode> = g
        .objects(node, RR_OBJECT_MAP)
        .map(as_resource)
        .collect::<Result<_>>()?;
    let mut objects = Vec::with_capacity(om_nodes.len());
    let mut standalone_maps = Vec::new();
    let mut star_assertions = Vec::new();
    for om in &om_nodes {
        let (object, om_standalone, om_star) = parse_object_map(g, om, outer_source)?;
        objects.push(object);
        standalone_maps.extend(om_standalone);
        star_assertions.extend(om_star);
    }
    for constant in g.objects(node, RR_OBJECT) {
        objects.push(ObjectMap::Term(TermMap::Constant(constant.clone())));
    }

    let graphs = parse_graph_maps(g, node)?;
    Ok((
        PredicateObjectMap {
            predicates,
            objects,
            graphs,
        },
        standalone_maps,
        star_assertions,
    ))
}

/// An object map is: an `rml:starMap` (ADR-0029 shape, ADR-0032 D1 naming —
/// object position, see below), a referencing object map when it has
/// `rr:parentTriplesMap` (R2RML §8), or otherwise a plain term map.
///
/// When the object-map node carries `rml:starMap`, the proposition id
/// becomes the object here (shared core: [`star::expand_star_map_object`]) —
/// unlike subject position, there is no reifier: the object-map's enclosing
/// triples map cannot host a reifying POM (its own subject is unrelated).
/// The 4 basic-encoding POMs (whose subject IS the proposition id) live on a
/// standalone description [`TriplesMap`] — keyed on the quoted map's own id,
/// so it is shared with every other reference to the same shape, subject or
/// object position — returned alongside the ordinary [`ObjectMap`] and
/// threaded up through [`parse_predicate_object_map`] → [`parse_triples_map`]
/// → [`parse_r2rml`], which appends it to the output (subject to the same
/// asserted/non-asserted retain filter as any other map — though its id
/// never matches a bare `quoted_id`, so that filter is always a no-op for
/// it; the description carrier is unconditional, exactly like the
/// subject-position `rdf:reifies` POM).
fn parse_object_map(
    g: &Graph,
    node: &NamedOrBlankNode,
    outer_source: &LogicalSource,
) -> Result<(ObjectMap, Vec<TriplesMap>, Vec<StarAssertion>)> {
    if let Some(star_map) = g.object(node, RML_STAR_MAP) {
        let star_node = as_resource(star_map)?;
        let expansion = star::expand_star_map_object(g, &star_node, outer_source)?;
        return Ok((
            expansion.object,
            expansion.description_maps,
            expansion.assertions,
        ));
    }
    let Some(parent) = g.object(node, RR_PARENT_TRIPLES_MAP) else {
        return Ok((
            ObjectMap::Term(parse_term_map(g, node, Position::Object)?),
            Vec::new(),
            Vec::new(),
        ));
    };
    let parent_triples_map = ref_id(parent)?;
    let joins = parse_join_conditions(g, node)?;
    Ok((
        ObjectMap::Ref(RefObjectMap {
            parent_triples_map,
            joins,
        }),
        Vec::new(),
        Vec::new(),
    ))
}

/// `rr:joinCondition`s (child/parent column pairs) declared directly on
/// `node` — an object map for an ordinary `rr:parentTriplesMap` join, or an
/// `rml:starMap` node for a cross-source star-map join (ADR-0032 D4).
fn parse_join_conditions(g: &Graph, node: &NamedOrBlankNode) -> Result<Vec<Join>> {
    let jc_nodes: Vec<NamedOrBlankNode> = g
        .objects(node, RR_JOIN_CONDITION)
        .map(as_resource)
        .collect::<Result<_>>()?;
    let mut joins = Vec::with_capacity(jc_nodes.len());
    for jc in &jc_nodes {
        let child = g
            .object(jc, RR_CHILD)
            .ok_or_else(|| Error::Mapping("rr:joinCondition has no rr:child".to_owned()))?;
        let parent_col = g
            .object(jc, RR_PARENT)
            .ok_or_else(|| Error::Mapping("rr:joinCondition has no rr:parent".to_owned()))?;
        joins.push(Join {
            child: sql_identifier(lexical(child)?),
            parent: sql_identifier(lexical(parent_col)?),
        });
    }
    Ok(joins)
}

/// `rr:graphMap` (term-map form) + `rr:graph` (constant shortcut). Empty ⇒ the
/// default graph.
fn parse_graph_maps(g: &Graph, carrier: &NamedOrBlankNode) -> Result<Vec<TermMap>> {
    let gm_nodes: Vec<NamedOrBlankNode> = g
        .objects(carrier, RR_GRAPH_MAP)
        .map(as_resource)
        .collect::<Result<_>>()?;
    let mut graphs = Vec::with_capacity(gm_nodes.len());
    for gm in &gm_nodes {
        graphs.push(parse_term_map(g, gm, Position::Graph)?);
    }
    for constant in g.objects(carrier, RR_GRAPH) {
        graphs.push(TermMap::Constant(constant.clone()));
    }
    Ok(graphs)
}

/// A term map: `rr:constant` (fixed term) | `rr:column` | `rr:template` (R2RML
/// §6.2). `rr:constant` takes the RDF term verbatim; the others carry a
/// [`TermSpec`] (term type + literal datatype/language).
fn parse_term_map(g: &Graph, node: &NamedOrBlankNode, position: Position) -> Result<TermMap> {
    if let Some(constant) = g.object(node, RR_CONSTANT) {
        return Ok(TermMap::Constant(constant.clone()));
    }
    let column = g.object(node, RR_COLUMN);
    let template = g.object(node, RR_TEMPLATE);
    let spec = build_term_spec(g, node, position, column.is_some())?;
    match (column, template) {
        (Some(col), _) => Ok(TermMap::Column(sql_identifier(lexical(col)?).into(), spec)),
        (None, Some(tmpl)) => {
            // A template placeholder column may be a delimited identifier
            // (`{"job"}`, R2RML §7.3); normalise it the same way as `rr:column`.
            let template = normalize_template_idents(Template::parse(lexical(tmpl)?)?);
            // R2RML §11/§7.3: a relative-IRI template is resolved against the
            // mapping base IRI. Templates that already begin with a URI scheme are
            // absolute and left untouched (the common case, allocation-free).
            let template = if spec.term_type == TermType::Iri {
                resolve_iri_template(template, DEFAULT_BASE_IRI)
            } else {
                template
            };
            Ok(TermMap::Template(template, spec))
        }
        (None, None) => Err(Error::Mapping(format!(
            "term map {} has none of rr:constant / rr:column / rr:template",
            node_id(node)
        ))),
    }
}

/// `rr:termType` (+ `rr:datatype` / `rr:language` for literals) with the R2RML
/// §7.4 default term type. `datatype`/`language` are kept only for literal term
/// maps (the IR contract); both present at once is an error.
fn build_term_spec(
    g: &Graph,
    node: &NamedOrBlankNode,
    position: Position,
    is_column: bool,
) -> Result<TermSpec> {
    let datatype = match g.object(node, RR_DATATYPE) {
        Some(t) => Some(as_named_node(t)?),
        None => None,
    };
    let language = match g.object(node, RR_LANGUAGE) {
        Some(t) => {
            let tag = lexical(t)?;
            if !sql::is_well_formed_language_tag(tag) {
                return Err(Error::Mapping(format!(
                    "rr:language {tag:?} of {} is not a valid BCP47 language tag",
                    node_id(node)
                )));
            }
            Some(tag.to_owned().into_boxed_str())
        }
        None => None,
    };
    if datatype.is_some() && language.is_some() {
        return Err(Error::Mapping(format!(
            "term map {} has both rr:datatype and rr:language",
            node_id(node)
        )));
    }
    let term_type = match g.object(node, RR_TERM_TYPE) {
        Some(Term::NamedNode(n)) => parse_term_type(n.as_str())?,
        Some(_) => {
            return Err(Error::Mapping(format!(
                "rr:termType of {} must be an IRI",
                node_id(node)
            )))
        }
        None => default_term_type(position, is_column, datatype.is_some(), language.is_some()),
    };
    // R2RML §7.4 term-type constraints by position: a subject map is an IRI or
    // blank node (never a literal); predicate and graph maps are IRIs only.
    match position {
        Position::Subject if term_type == TermType::Literal => {
            return Err(Error::Mapping(format!(
                "subject map {} has rr:termType rr:Literal (must be IRI or blank node)",
                node_id(node)
            )))
        }
        Position::Predicate | Position::Graph if term_type != TermType::Iri => {
            return Err(Error::Mapping(format!(
                "predicate/graph term map {} must have rr:termType rr:IRI",
                node_id(node)
            )))
        }
        _ => {}
    }
    let (datatype, language) = if term_type == TermType::Literal {
        (datatype, language)
    } else {
        (None, None)
    };
    // An `rr:column` IRI term map resolves its per-row value against the mapping
    // base (R2RML §7.3); `rr:template` IRIs already bake the base in at parse time.
    let base = if term_type == TermType::Iri && is_column {
        Some(DEFAULT_BASE_IRI.into())
    } else {
        None
    };
    Ok(TermSpec {
        term_type,
        datatype,
        language,
        base,
    })
}

/// R2RML §7.4 default term type: a literal only for an object map that is
/// column-valued or carries a datatype/language; an IRI everywhere else
/// (subjects, predicates, graphs, and template/constant objects).
fn default_term_type(
    position: Position,
    is_column: bool,
    has_datatype: bool,
    has_language: bool,
) -> TermType {
    match position {
        Position::Object if is_column || has_datatype || has_language => TermType::Literal,
        _ => TermType::Iri,
    }
}

fn parse_term_type(iri: &str) -> Result<TermType> {
    match iri {
        RR_IRI => Ok(TermType::Iri),
        RR_BLANK_NODE => Ok(TermType::BlankNode),
        RR_LITERAL => Ok(TermType::Literal),
        other => Err(Error::Mapping(format!("unknown rr:termType <{other}>"))),
    }
}

/// Where a term map sits — fixes the §7.4 default term type.
#[derive(Clone, Copy)]
enum Position {
    Subject,
    Predicate,
    Object,
    Graph,
}

// --- helpers ------------------------------------------------------------------

/// The stable identifier of a map resource: the IRI for a named node, `_:id` for
/// a blank node. Used for [`TriplesMap::id`] and `rr:parentTriplesMap` matching.
fn node_id(node: &NamedOrBlankNode) -> String {
    match node {
        NamedOrBlankNode::NamedNode(n) => n.as_str().to_owned(),
        NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
    }
}

/// A term used as a map node must be a resource (IRI or blank node).
fn as_resource(term: &Term) -> Result<NamedOrBlankNode> {
    match term {
        Term::NamedNode(n) => Ok(NamedOrBlankNode::NamedNode(n.clone())),
        Term::BlankNode(b) => Ok(NamedOrBlankNode::BlankNode(b.clone())),
        _ => Err(Error::Mapping(
            "expected an IRI or blank node (a map node)".to_owned(),
        )),
    }
}

/// An `rr:class` / `rr:datatype` value must be an IRI.
fn as_named_node(term: &Term) -> Result<NamedNode> {
    match term {
        Term::NamedNode(n) => Ok(n.clone()),
        _ => Err(Error::Mapping("expected an IRI".to_owned())),
    }
}

/// `rr:parentTriplesMap` references a triples map by its [`node_id`].
fn ref_id(term: &Term) -> Result<String> {
    match term {
        Term::NamedNode(n) => Ok(n.as_str().to_owned()),
        Term::BlankNode(b) => Ok(format!("_:{}", b.as_str())),
        _ => Err(Error::Mapping(
            "rr:parentTriplesMap must reference a triples map".to_owned(),
        )),
    }
}

/// The lexical value of a string-literal property (`rr:column`, `rr:template`,
/// `rr:tableName`, `rr:sqlQuery`, `rr:language`, `rr:child`, `rr:parent`).
fn lexical(term: &Term) -> Result<&str> {
    match term {
        Term::Literal(l) => Ok(l.value()),
        _ => Err(Error::Mapping("expected a string literal".to_owned())),
    }
}

/// The mapping graph `M`, indexed by subject. Small/intensional (ADR-0003), so a
/// per-subject `Vec` scanned by predicate is more than enough.
struct Graph {
    spo: HashMap<NamedOrBlankNode, Vec<(NamedNode, Term)>>,
}

impl Graph {
    fn load(turtle: &str) -> Result<Self> {
        let parser = TurtleParser::new()
            .with_base_iri(DEFAULT_BASE_IRI)
            .map_err(|e| Error::Mapping(format!("invalid default base IRI: {e}")))?;
        let mut spo: HashMap<NamedOrBlankNode, Vec<(NamedNode, Term)>> = HashMap::new();
        for triple in parser.for_slice(turtle) {
            let Triple {
                subject,
                predicate,
                object,
            } = triple.map_err(|e| Error::Mapping(format!("R2RML Turtle parse error: {e}")))?;
            spo.entry(subject).or_default().push((predicate, object));
        }
        Ok(Self { spo })
    }

    /// Every object of `s p ?o`, in document order.
    fn objects<'a>(
        &'a self,
        s: &NamedOrBlankNode,
        p: &'static str,
    ) -> impl Iterator<Item = &'a Term> + 'a {
        self.spo
            .get(s)
            .into_iter()
            .flatten()
            .filter(move |(predicate, _)| predicate.as_str() == p)
            .map(|(_, object)| object)
    }

    /// The first object of `s p ?o` (R2RML map properties are single-valued).
    fn object<'a>(&'a self, s: &NamedOrBlankNode, p: &'static str) -> Option<&'a Term> {
        self.objects(s, p).next()
    }
}

#[cfg(test)]
mod tests;

/// Unit coverage for `build_term_spec`'s R2RML §7.4 rejection branches — the
/// validation gate a malformed `rr:termType` / `rr:datatype` / `rr:language`
/// combination must fail through before it ever reaches the IR. Each test
/// drives `build_term_spec` directly against a single-node Turtle fixture
/// (rather than a full `parse_r2rml` pipeline) so the assertion is scoped to
/// exactly the branch under test.
#[cfg(test)]
mod term_spec_tests {
    use super::*;

    /// Load `turtle` and run the term map at `<http://ex.org/tm>` through
    /// `build_term_spec` for `position`/`is_column`.
    fn term_spec(turtle: &str, position: Position, is_column: bool) -> Result<TermSpec> {
        let g = Graph::load(turtle).expect("fixture parses as turtle");
        let node = NamedOrBlankNode::NamedNode(NamedNode::new("http://ex.org/tm").unwrap());
        build_term_spec(&g, &node, position, is_column)
    }

    /// Unwrap an `Error::Mapping` message, panicking on any other outcome —
    /// keeps each test's assertion pinned to *why* it failed, not just *that*
    /// it failed.
    fn mapping_err_message(result: Result<TermSpec>) -> String {
        match result {
            Err(Error::Mapping(msg)) => msg,
            Err(other) => panic!("expected Error::Mapping, got {other:?}"),
            Ok(spec) => panic!("expected a rejection, got {spec:?}"),
        }
    }

    #[test]
    fn rejects_malformed_bcp47_language_tag() {
        let turtle = r#"
            @prefix rr: <http://www.w3.org/ns/r2rml#> .
            <http://ex.org/tm> rr:language "english" .
        "#;
        let msg = mapping_err_message(term_spec(turtle, Position::Object, true));
        assert!(
            msg.contains("not a valid BCP47 language tag"),
            "unexpected message: {msg}"
        );
    }

    #[test]
    fn rejects_datatype_and_language_both_set() {
        let turtle = r#"
            @prefix rr: <http://www.w3.org/ns/r2rml#> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            <http://ex.org/tm> rr:datatype xsd:string ; rr:language "en" .
        "#;
        let msg = mapping_err_message(term_spec(turtle, Position::Object, true));
        assert!(
            msg.contains("has both rr:datatype and rr:language"),
            "unexpected message: {msg}"
        );
    }

    #[test]
    fn rejects_term_type_that_is_not_an_iri() {
        // rr:termType given a plain literal instead of an IRI (rr:IRI / rr:Literal / …).
        let turtle = r#"
            @prefix rr: <http://www.w3.org/ns/r2rml#> .
            <http://ex.org/tm> rr:termType "not-an-iri" .
        "#;
        let msg = mapping_err_message(term_spec(turtle, Position::Object, true));
        assert!(
            msg.contains("rr:termType of") && msg.contains("must be an IRI"),
            "unexpected message: {msg}"
        );
    }

    #[test]
    fn rejects_unknown_term_type_iri() {
        let turtle = r#"
            @prefix rr: <http://www.w3.org/ns/r2rml#> .
            <http://ex.org/tm> rr:termType <http://example.com/bogus> .
        "#;
        let msg = mapping_err_message(term_spec(turtle, Position::Object, true));
        assert!(
            msg.contains("unknown rr:termType"),
            "unexpected message: {msg}"
        );
    }

    #[test]
    fn rejects_literal_term_type_on_a_subject_map() {
        let turtle = r#"
            @prefix rr: <http://www.w3.org/ns/r2rml#> .
            <http://ex.org/tm> rr:termType rr:Literal .
        "#;
        let msg = mapping_err_message(term_spec(turtle, Position::Subject, false));
        assert!(
            msg.contains("must be IRI or blank node"),
            "unexpected message: {msg}"
        );
    }

    #[test]
    fn rejects_non_iri_term_type_on_a_predicate_map() {
        let turtle = r#"
            @prefix rr: <http://www.w3.org/ns/r2rml#> .
            <http://ex.org/tm> rr:termType rr:BlankNode .
        "#;
        let msg = mapping_err_message(term_spec(turtle, Position::Predicate, false));
        assert!(
            msg.contains("must have rr:termType rr:IRI"),
            "unexpected message: {msg}"
        );
    }

    #[test]
    fn rejects_non_iri_term_type_on_a_graph_map() {
        let turtle = r#"
            @prefix rr: <http://www.w3.org/ns/r2rml#> .
            <http://ex.org/tm> rr:termType rr:BlankNode .
        "#;
        let msg = mapping_err_message(term_spec(turtle, Position::Graph, false));
        assert!(
            msg.contains("must have rr:termType rr:IRI"),
            "unexpected message: {msg}"
        );
    }

    // Not a rejection branch: R2RML §7.4 gives literal/blank-node term maps
    // (rr:IRI is not requested) no datatype/language-driven default beyond
    // "IRI unless object + column/datatype/language" — accepting the request
    // and letting default_term_type() decide is intended, not a validation
    // gap. Likewise `rr:column` *and* `rr:template` both present on one term
    // map is not rejected by `build_term_spec`: `parse_term_map`'s
    // `match (column, template)` silently prefers `rr:column` and ignores the
    // template (see `parse_term_map`, ~line 278) — there is no "both set" error
    // to trigger here.
}
