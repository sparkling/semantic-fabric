//! Ground RDF projection of executable mapping IR for semantic admission.
//!
//! Only facts consumed by the sealed M⋈T shapes are projected. Generated node
//! IRIs use source-local IR coordinates, so authored and Direct mappings take
//! one path and mapping blank nodes can never alias ontology blank nodes.

use oxrdf::{Graph, NamedNode, Term, Triple};
use sf_core::ir::{ObjectMap, TermMap};
use sf_core::SourceMapping;

const RR_SUBJECT_MAP: &str = "http://www.w3.org/ns/r2rml#subjectMap";
const RR_CLASS: &str = "http://www.w3.org/ns/r2rml#class";
const RR_PREDICATE_OBJECT_MAP: &str = "http://www.w3.org/ns/r2rml#predicateObjectMap";
const RR_PREDICATE: &str = "http://www.w3.org/ns/r2rml#predicate";
const RR_OBJECT_MAP: &str = "http://www.w3.org/ns/r2rml#objectMap";
const RR_DATATYPE: &str = "http://www.w3.org/ns/r2rml#datatype";

/// Upper bound on the ground validation projection, independent of source rows.
pub const MAX_MAPPING_PROJECTION_TRIPLES: usize = 250_000;

/// Why executable mapping IR could not be proven against a finite ontology.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProjectionError {
    #[error("mapping has a non-constant predicate target")]
    DynamicPredicate,
    #[error("mapping predicate target is not an IRI")]
    InvalidPredicate,
    #[error("mapping validation projection exceeds its triple limit")]
    TripleLimit,
}

/// Project the effective, gate-relevant semantics of one source mapping to a
/// ground graph. Dynamic predicate maps reject: finite membership in T cannot
/// be proven from a row-dependent value.
pub fn project_to_rdf(mapping: &SourceMapping) -> Result<Graph, ProjectionError> {
    let mut graph = Graph::new();
    for (map_index, triples_map) in mapping.triples_maps().iter().enumerate() {
        let map = node(mapping, map_index, "map", 0, 0);
        let subject = node(mapping, map_index, "subject", 0, 0);
        insert(&mut graph, &map, RR_SUBJECT_MAP, subject.clone())?;
        for class in &triples_map.subject.classes {
            insert(&mut graph, &subject, RR_CLASS, class.clone())?;
        }

        for (pom_index, pom) in triples_map.predicate_object_maps.iter().enumerate() {
            let pom_node = node(mapping, map_index, "pom", pom_index, 0);
            insert(&mut graph, &map, RR_PREDICATE_OBJECT_MAP, pom_node.clone())?;
            for predicate in &pom.predicates {
                let predicate = match predicate {
                    TermMap::Constant(Term::NamedNode(predicate)) => predicate.clone(),
                    TermMap::Constant(_) => return Err(ProjectionError::InvalidPredicate),
                    TermMap::Column(_, _) | TermMap::Template(_, _) => {
                        return Err(ProjectionError::DynamicPredicate)
                    }
                };
                insert(&mut graph, &pom_node, RR_PREDICATE, predicate)?;
            }
            for (object_index, object) in pom.objects.iter().enumerate() {
                let object_node = node(mapping, map_index, "object", pom_index, object_index);
                insert(&mut graph, &pom_node, RR_OBJECT_MAP, object_node.clone())?;
                if let Some(datatype) = effective_explicit_datatype(object) {
                    insert(&mut graph, &object_node, RR_DATATYPE, datatype)?;
                }
            }
        }
    }
    Ok(graph)
}

fn effective_explicit_datatype(object: &ObjectMap) -> Option<NamedNode> {
    match object {
        ObjectMap::Term(TermMap::Column(_, spec) | TermMap::Template(_, spec)) => {
            spec.datatype.clone()
        }
        ObjectMap::Term(TermMap::Constant(Term::Literal(literal))) => {
            Some(literal.datatype().into_owned())
        }
        ObjectMap::Term(TermMap::Constant(_)) | ObjectMap::Ref(_) => None,
    }
}

fn node(mapping: &SourceMapping, map: usize, role: &str, pom: usize, object: usize) -> NamedNode {
    NamedNode::new_unchecked(format!(
        "urn:semantic-fabric:mjoin-t:v1:s{}:m{map}:{role}:p{pom}:o{object}",
        mapping.source_id().index()
    ))
}

fn insert(
    graph: &mut Graph,
    subject: &NamedNode,
    predicate: &str,
    object: impl Into<Term>,
) -> Result<(), ProjectionError> {
    graph.insert(&Triple::new(
        subject.clone(),
        NamedNode::new_unchecked(predicate),
        object,
    ));
    if graph.len() > MAX_MAPPING_PROJECTION_TRIPLES {
        return Err(ProjectionError::TripleLimit);
    }
    Ok(())
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
        let graph = project_to_rdf(&mapping).unwrap();
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
            project_to_rdf(&mapping),
            Err(ProjectionError::DynamicPredicate)
        ));
    }
}
