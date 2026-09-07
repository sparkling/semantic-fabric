//! Ground RDF projection of executable mapping IR for semantic admission.
//!
//! Only facts consumed by the sealed M⋈T shapes are projected. Generated node
//! IRIs use source-local IR coordinates, so authored and Direct mappings take
//! one path and mapping blank nodes can never alias ontology blank nodes.

use oxrdf::{vocab::rdf, vocab::xsd, Graph, NamedNode, Term, Triple};
use sf_core::ir::{LogicalSource, ObjectMap, TermMap, TermSpec, TermType};
use sf_core::SourceMapping;

const RR_SUBJECT_MAP: &str = "http://www.w3.org/ns/r2rml#subjectMap";
const RR_CLASS: &str = "http://www.w3.org/ns/r2rml#class";
const RR_PREDICATE_OBJECT_MAP: &str = "http://www.w3.org/ns/r2rml#predicateObjectMap";
const RR_PREDICATE: &str = "http://www.w3.org/ns/r2rml#predicate";
const RR_OBJECT_MAP: &str = "http://www.w3.org/ns/r2rml#objectMap";
const RR_DATATYPE: &str = "http://www.w3.org/ns/r2rml#datatype";

/// Namespace reserved exclusively for structural nodes minted by this
/// projection. Ontology facts in this namespace are rejected at the merge gate.
pub const MAPPING_PROJECTION_NODE_PREFIX: &str = "urn:semantic-fabric:mjoin-t:v1:";
/// Upper bound on the ground validation projection, independent of source rows.
pub const MAX_MAPPING_PROJECTION_TRIPLES: usize = 250_000;
/// Every attempted projection statement counts, including graph duplicates.
pub const MAX_MAPPING_PROJECTION_OCCURRENCES: usize = 250_000;
/// Bound lexical material walked or cloned while building the projection.
pub const MAX_MAPPING_PROJECTION_BYTES: usize = 32 * 1024 * 1024;

/// Why executable mapping IR could not be proven against a finite ontology.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProjectionError {
    #[error("mapping has a non-constant predicate target")]
    DynamicPredicate,
    #[error("mapping predicate target is not an IRI")]
    InvalidPredicate,
    #[error("mapping validation projection exceeds its triple limit")]
    TripleLimit,
    #[error("mapping validation projection exceeds its byte limit")]
    ByteLimit,
    #[error("mapping implicit literal datatype cannot be proven from the source")]
    ImplicitDatatypeUnresolved,
}

/// Project the complete effective semantics of one source mapping. The resolver
/// must return the datatype that row reconstruction will emit for an implicit
/// literal column, or `None` when that fact cannot be proven. Dynamic predicates
/// and unresolved implicit datatypes fail closed.
pub fn project_to_rdf(
    mapping: &SourceMapping,
    mut resolve_implicit: impl FnMut(&LogicalSource, &str) -> Option<NamedNode>,
) -> Result<Graph, ProjectionError> {
    project(
        mapping,
        ProjectionMode::Complete(&mut resolve_implicit),
        limits(),
    )
}

/// Schema-independent semantic preflight. An implicit column's object edge is
/// deliberately withheld until a coherent source observation can resolve its
/// effective datatype; this function never creates a runtime admission receipt.
pub fn project_static_to_rdf(mapping: &SourceMapping) -> Result<Graph, ProjectionError> {
    project(mapping, ProjectionMode::Static, limits())
}

fn project(
    mapping: &SourceMapping,
    mut mode: ProjectionMode<'_>,
    limits: ProjectionLimits,
) -> Result<Graph, ProjectionError> {
    let mut graph = Graph::new();
    let mut budget = ProjectionBudget::new(limits);
    for (map_index, triples_map) in mapping.triples_maps().iter().enumerate() {
        let map = node(mapping, map_index, "map", 0, 0);
        let subject = node(mapping, map_index, "subject", 0, 0);
        insert(
            &mut graph,
            &mut budget,
            &map,
            RR_SUBJECT_MAP,
            subject.clone(),
        )?;
        for class in &triples_map.subject.classes {
            insert(&mut graph, &mut budget, &subject, RR_CLASS, class.clone())?;
        }

        for (pom_index, pom) in triples_map.predicate_object_maps.iter().enumerate() {
            let pom_node = node(mapping, map_index, "pom", pom_index, 0);
            insert(
                &mut graph,
                &mut budget,
                &map,
                RR_PREDICATE_OBJECT_MAP,
                pom_node.clone(),
            )?;
            for predicate in &pom.predicates {
                let predicate = match predicate {
                    TermMap::Constant(Term::NamedNode(predicate)) => predicate.clone(),
                    TermMap::Constant(_) => return Err(ProjectionError::InvalidPredicate),
                    TermMap::Column(_, _) | TermMap::Template(_, _) => {
                        return Err(ProjectionError::DynamicPredicate)
                    }
                };
                insert(&mut graph, &mut budget, &pom_node, RR_PREDICATE, predicate)?;
            }
            for (object_index, object) in pom.objects.iter().enumerate() {
                let object_node = node(mapping, map_index, "object", pom_index, object_index);
                let datatype = effective_datatype(object, &triples_map.source, &mut mode)?;
                if datatype == EffectiveDatatype::Deferred {
                    continue;
                }
                insert(
                    &mut graph,
                    &mut budget,
                    &pom_node,
                    RR_OBJECT_MAP,
                    object_node.clone(),
                )?;
                if let EffectiveDatatype::Known(datatype) = datatype {
                    insert(&mut graph, &mut budget, &object_node, RR_DATATYPE, datatype)?;
                }
            }
        }
    }
    Ok(graph)
}

enum ProjectionMode<'a> {
    Static,
    Complete(&'a mut dyn FnMut(&LogicalSource, &str) -> Option<NamedNode>),
}

#[derive(Eq, PartialEq)]
enum EffectiveDatatype {
    Known(NamedNode),
    NonLiteral,
    Deferred,
}

fn effective_datatype(
    object: &ObjectMap,
    source: &LogicalSource,
    mode: &mut ProjectionMode<'_>,
) -> Result<EffectiveDatatype, ProjectionError> {
    match object {
        ObjectMap::Term(TermMap::Column(column, spec)) => {
            column_datatype(source, column, spec, mode)
        }
        ObjectMap::Term(TermMap::Template(_, spec)) => {
            Ok(template_datatype(spec)
                .map_or(EffectiveDatatype::NonLiteral, EffectiveDatatype::Known))
        }
        ObjectMap::Term(TermMap::Constant(Term::Literal(literal))) => {
            Ok(EffectiveDatatype::Known(literal.datatype().into_owned()))
        }
        ObjectMap::Term(TermMap::Constant(_)) | ObjectMap::Ref(_) => {
            Ok(EffectiveDatatype::NonLiteral)
        }
    }
}

fn column_datatype(
    source: &LogicalSource,
    column: &str,
    spec: &TermSpec,
    mode: &mut ProjectionMode<'_>,
) -> Result<EffectiveDatatype, ProjectionError> {
    if let Some(datatype) = declared_datatype(spec) {
        return Ok(EffectiveDatatype::Known(datatype));
    }
    if spec.term_type != TermType::Literal {
        return Ok(EffectiveDatatype::NonLiteral);
    }
    match mode {
        ProjectionMode::Static => Ok(EffectiveDatatype::Deferred),
        ProjectionMode::Complete(resolve) => resolve(source, column)
            .map(EffectiveDatatype::Known)
            .ok_or(ProjectionError::ImplicitDatatypeUnresolved),
    }
}

fn declared_datatype(spec: &TermSpec) -> Option<NamedNode> {
    if spec.term_type != TermType::Literal {
        return None;
    }
    spec.datatype.clone().or_else(|| {
        spec.language
            .is_some()
            .then(|| rdf::LANG_STRING.into_owned())
    })
}

fn template_datatype(spec: &TermSpec) -> Option<NamedNode> {
    if spec.term_type != TermType::Literal {
        return None;
    }
    declared_datatype(spec).or_else(|| Some(xsd::STRING.into_owned()))
}

fn node(mapping: &SourceMapping, map: usize, role: &str, pom: usize, object: usize) -> NamedNode {
    NamedNode::new_unchecked(format!(
        "{MAPPING_PROJECTION_NODE_PREFIX}s{}:m{map}:{role}:p{pom}:o{object}",
        mapping.source_id().index()
    ))
}

fn insert(
    graph: &mut Graph,
    budget: &mut ProjectionBudget,
    subject: &NamedNode,
    predicate: &str,
    object: impl Into<Term>,
) -> Result<(), ProjectionError> {
    let triple = Triple::new(subject.clone(), NamedNode::new_unchecked(predicate), object);
    budget.record(&triple)?;
    graph.insert(&triple);
    Ok(())
}

#[derive(Clone, Copy)]
struct ProjectionLimits {
    max_occurrences: usize,
    max_bytes: usize,
}

const fn limits() -> ProjectionLimits {
    ProjectionLimits {
        max_occurrences: MAX_MAPPING_PROJECTION_OCCURRENCES,
        max_bytes: MAX_MAPPING_PROJECTION_BYTES,
    }
}

struct ProjectionBudget {
    limits: ProjectionLimits,
    occurrences: usize,
    bytes: usize,
}

impl ProjectionBudget {
    const fn new(limits: ProjectionLimits) -> Self {
        Self {
            limits,
            occurrences: 0,
            bytes: 0,
        }
    }

    fn record(&mut self, triple: &Triple) -> Result<(), ProjectionError> {
        self.occurrences = self
            .occurrences
            .checked_add(1)
            .ok_or(ProjectionError::TripleLimit)?;
        if self.occurrences > self.limits.max_occurrences {
            return Err(ProjectionError::TripleLimit);
        }
        self.bytes = self
            .bytes
            .checked_add(triple.to_string().len())
            .ok_or(ProjectionError::ByteLimit)?;
        if self.bytes > self.limits.max_bytes {
            return Err(ProjectionError::ByteLimit);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use oxrdf::{NamedOrBlankNodeRef, TermRef};
    use sf_core::ir::{LogicalSource, PredicateObjectMap, SubjectMap, TermSpec, TriplesMap};
    use sf_core::{SourceId, Term};

    use super::*;

    const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<#items> a rr:TriplesMap;
  rr:logicalTable [ rr:tableName "items" ];
  rr:subjectMap [ rr:template "http://example.com/item/{id}"; rr:class ex:Item ];
  rr:predicateObjectMap [
    rr:predicate ex:quantity;
    rr:objectMap [ rr:column "quantity"; rr:datatype xsd:integer ]
  ].
"#;

    #[test]
    fn projects_class_predicate_and_datatype_without_blank_nodes() {
        let mapping = crate::parse_r2rml_for_source(MAPPING, SourceId::new(4).unwrap()).unwrap();
        let graph = project_to_rdf(&mapping, |_, _| None).unwrap();
        assert!(graph
            .iter()
            .any(|triple| triple.predicate.as_str() == RR_CLASS));
        assert!(graph
            .iter()
            .any(|triple| triple.predicate.as_str() == RR_PREDICATE));
        assert!(graph
            .iter()
            .any(|triple| triple.predicate.as_str() == RR_DATATYPE));
        assert!(graph.iter().all(|triple| {
            matches!(triple.subject, NamedOrBlankNodeRef::NamedNode(_))
                && !matches!(triple.object, TermRef::BlankNode(_))
        }));
    }

    #[test]
    fn rejects_a_row_dependent_predicate_target() {
        let mapping = SourceMapping::new(
            SourceId::new(0).unwrap(),
            vec![TriplesMap {
                id: "urn:map".to_owned(),
                source: LogicalSource::Table("items".to_owned()),
                subject: SubjectMap {
                    term: TermMap::Constant(Term::NamedNode(NamedNode::new_unchecked("urn:s"))),
                    classes: vec![],
                    graphs: vec![],
                },
                predicate_object_maps: vec![PredicateObjectMap {
                    predicates: vec![TermMap::Column("predicate".into(), TermSpec::iri())],
                    objects: vec![],
                    graphs: vec![],
                }],
            }],
        );
        assert!(matches!(
            project_to_rdf(&mapping, |_, _| None),
            Err(ProjectionError::DynamicPredicate)
        ));
    }

    #[test]
    fn implicit_column_uses_the_runtime_type_resolver() {
        let mapping = crate::parse_r2rml_for_source(
            &MAPPING.replace("; rr:datatype xsd:integer", ""),
            SourceId::new(4).unwrap(),
        )
        .unwrap();
        let graph = project_to_rdf(&mapping, |source, column| {
            assert!(matches!(source, LogicalSource::Table(table) if table == "items"));
            assert_eq!(column, "quantity");
            Some(xsd::INTEGER.into_owned())
        })
        .unwrap();
        assert!(graph.iter().any(|triple| {
            triple.predicate.as_str() == RR_DATATYPE
                && triple.object == TermRef::NamedNode(xsd::INTEGER)
        }));
    }

    #[test]
    fn unresolved_implicit_column_fails_closed_but_static_preflight_withholds_it() {
        let mapping = crate::parse_r2rml_for_source(
            &MAPPING.replace("; rr:datatype xsd:integer", ""),
            SourceId::new(4).unwrap(),
        )
        .unwrap();
        assert_eq!(
            project_to_rdf(&mapping, |_, _| None),
            Err(ProjectionError::ImplicitDatatypeUnresolved)
        );
        let static_graph = project_static_to_rdf(&mapping).unwrap();
        assert!(!static_graph
            .iter()
            .any(|triple| triple.predicate.as_str() == RR_OBJECT_MAP));
    }

    #[test]
    fn duplicate_projection_occurrences_are_bounded_before_graph_deduplication() {
        let class = NamedNode::new_unchecked("urn:Class");
        let mapping_with = |count| {
            SourceMapping::new(
                SourceId::new(0).unwrap(),
                vec![TriplesMap {
                    id: "urn:map".to_owned(),
                    source: LogicalSource::Table("items".to_owned()),
                    subject: SubjectMap {
                        term: TermMap::Constant(Term::NamedNode(NamedNode::new_unchecked("urn:s"))),
                        classes: vec![class.clone(); count],
                        graphs: vec![],
                    },
                    predicate_object_maps: vec![],
                }],
            )
        };
        let limits = ProjectionLimits {
            max_occurrences: 3,
            max_bytes: usize::MAX,
        };
        assert!(project(&mapping_with(2), ProjectionMode::Static, limits).is_ok());
        assert_eq!(
            project(&mapping_with(3), ProjectionMode::Static, limits),
            Err(ProjectionError::TripleLimit)
        );
    }

    #[test]
    fn projection_lexical_bytes_are_bounded() {
        let mapping = crate::parse_r2rml_for_source(MAPPING, SourceId::new(4).unwrap()).unwrap();
        let limits = ProjectionLimits {
            max_occurrences: usize::MAX,
            max_bytes: 1,
        };
        assert_eq!(
            project(&mapping, ProjectionMode::Static, limits),
            Err(ProjectionError::ByteLimit)
        );
    }
}
