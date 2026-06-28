//! Parse an R2RML mapping document (Turtle, RDF 1.2 via `oxttl`) into the
//! `sf-core` mapping IR — done exactly once (ADR-0003 R1; the `sf-mapping` row of
//! ADR-0006; `oxttl` `rdf-12` per ADR-0019).
//!
//! The mapping graph `M` is small and intensional (ADR-0003 / ADR-0004), so this
//! loads the whole document into a subject-indexed graph and walks the R2RML
//! vocabulary onto the IR. RDF terms stay `oxrdf` types end to end (ADR-0003 R2);
//! the IR they populate is the single rewrite target for the virtualiser.

use std::collections::HashMap;

use oxttl::TurtleParser;

use sf_core::ir::{
    Join, LogicalSource, ObjectMap, PredicateObjectMap, RefObjectMap, SubjectMap, Template, TermMap,
    TermSpec, TermType, TriplesMap,
};
use sf_core::{Error, NamedNode, NamedOrBlankNode, Result, Term, Triple};

mod sql;
use sql::{normalize_template_idents, resolve_iri_template, sql_identifier, strip_trailing_semicolon};

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
    for (_, subject) in subjects {
        maps.push(parse_triples_map(&graph, subject)?);
    }
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

fn parse_triples_map(g: &Graph, tm: &NamedOrBlankNode) -> Result<TriplesMap> {
    let source = parse_logical_source(g, tm)?;
    let subject = parse_subject_map(g, tm)?;
    let pom_nodes: Vec<NamedOrBlankNode> = g
        .objects(tm, RR_PREDICATE_OBJECT_MAP)
        .map(as_resource)
        .collect::<Result<_>>()?;
    let mut predicate_object_maps = Vec::with_capacity(pom_nodes.len());
    for pom in &pom_nodes {
        predicate_object_maps.push(parse_predicate_object_map(g, pom)?);
    }
    Ok(TriplesMap {
        id: node_id(tm),
        source,
        subject,
        predicate_object_maps,
    })
}

/// `rr:logicalTable` → `rr:tableName` (base table/view) or `rr:sqlQuery` (R2RML
/// view). R2RML-only: no reference formulation (ADR-0002).
fn parse_logical_source(g: &Graph, tm: &NamedOrBlankNode) -> Result<LogicalSource> {
    let lt = g.object(tm, RR_LOGICAL_TABLE).ok_or_else(|| {
        Error::Mapping(format!("triples map {} has no rr:logicalTable", node_id(tm)))
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
        Ok(LogicalSource::Query(strip_trailing_semicolon(lexical(query)?)))
    } else {
        Err(Error::Mapping(format!(
            "logical table {} has neither rr:tableName nor rr:sqlQuery",
            node_id(&node)
        )))
    }
}

/// The subject map (`rr:subjectMap`, or the `rr:subject` constant shortcut) plus
/// its `rr:class` types and graph maps (R2RML §6.1).
fn parse_subject_map(g: &Graph, tm: &NamedOrBlankNode) -> Result<SubjectMap> {
    // R2RML §6: a triples map has *exactly one* subject map (two is an error).
    let subject_map_count = g.objects(tm, RR_SUBJECT_MAP).count();
    let subject_count = g.objects(tm, RR_SUBJECT).count();
    if subject_map_count + subject_count > 1 {
        return Err(Error::Mapping(format!(
            "triples map {} has more than one subject map",
            node_id(tm)
        )));
    }
    let (term, carrier) = if let Some(sm) = g.object(tm, RR_SUBJECT_MAP) {
        let node = as_resource(sm)?;
        let term = parse_term_map(g, &node, Position::Subject)?;
        (term, node)
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
    Ok(SubjectMap {
        term,
        classes,
        graphs,
    })
}

/// A predicate-object map: `rr:predicateMap`/`rr:predicate` paired with
/// `rr:objectMap`/`rr:object`, plus graph maps (R2RML §6.3).
fn parse_predicate_object_map(g: &Graph, node: &NamedOrBlankNode) -> Result<PredicateObjectMap> {
    let pm_nodes: Vec<NamedOrBlankNode> = g
        .objects(node, RR_PREDICATE_MAP)
        .map(as_resource)
        .collect::<Result<_>>()?;
    let mut predicates = Vec::with_capacity(pm_nodes.len());
    for pm in &pm_nodes {
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
    for om in &om_nodes {
        objects.push(parse_object_map(g, om)?);
    }
    for constant in g.objects(node, RR_OBJECT) {
        objects.push(ObjectMap::Term(TermMap::Constant(constant.clone())));
    }

    let graphs = parse_graph_maps(g, node)?;
    Ok(PredicateObjectMap {
        predicates,
        objects,
        graphs,
    })
}

/// An object map is a referencing object map when it has `rr:parentTriplesMap`
/// (R2RML §8); otherwise it is a plain term map.
fn parse_object_map(g: &Graph, node: &NamedOrBlankNode) -> Result<ObjectMap> {
    let Some(parent) = g.object(node, RR_PARENT_TRIPLES_MAP) else {
        return Ok(ObjectMap::Term(parse_term_map(g, node, Position::Object)?));
    };
    let parent_triples_map = ref_id(parent)?;
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
    Ok(ObjectMap::Ref(RefObjectMap {
        parent_triples_map,
        joins,
    }))
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
