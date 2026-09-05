//! Bounded, immutable ontology document plus its tier-1 reasoning projection.
//!
//! Runtime admission retains the complete RDF graph for M⋈T validation. The
//! smaller [`Tbox`] is derived from that graph for query rewriting; it is never
//! treated as the semantic document or its identity.

use oxrdf::{Graph, NamedOrBlankNodeRef, TermRef};
use sf_sparql::Tbox;
use sha2::{Digest, Sha256};

const DOCUMENT_IDENTITY_DOMAIN: &[u8] = b"semantic-fabric/ontology-document/v1";
const RDFS_SUBCLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const RDFS_SUBPROPERTY_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subPropertyOf";
const OWL_INVERSE_OF: &str = "http://www.w3.org/2002/07/owl#inverseOf";
const OWL_SYMMETRIC_PROPERTY: &str = "http://www.w3.org/2002/07/owl#SymmetricProperty";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// Full, parsed ontology graph retained beside the derived reasoning T-box.
/// Construction is bounded and private-fielded; runtime snapshots accept this
/// type rather than a caller-supplied digest or reduced T-box.
#[derive(Clone)]
pub struct SemanticOntology {
    graph: Graph,
    tbox: Tbox,
    document_digest: [u8; 32],
}

impl SemanticOntology {
    /// Parse a bounded Turtle ontology. The digest is deliberately the exact
    /// input-document identity, not a claim of RDF graph canonicalization.
    pub fn from_turtle(turtle: &str) -> Result<Self, String> {
        let graph = sf_validation::parse_turtle_graph(turtle, sf_validation::DEFAULT_GRAPH_LIMITS)
            .map_err(|error| error.to_string())?;
        let tbox = tbox_from_graph(&graph);
        let mut hasher = Sha256::new();
        hasher.update(DOCUMENT_IDENTITY_DOMAIN);
        hasher.update(
            u64::try_from(turtle.len())
                .unwrap_or(u64::MAX)
                .to_be_bytes(),
        );
        hasher.update(turtle.as_bytes());
        Ok(Self {
            graph,
            tbox,
            document_digest: hasher.finalize().into(),
        })
    }

    pub(crate) const fn graph(&self) -> &Graph {
        &self.graph
    }

    pub(crate) const fn tbox(&self) -> &Tbox {
        &self.tbox
    }

    pub(crate) const fn document_digest(&self) -> [u8; 32] {
        self.document_digest
    }
}

impl std::fmt::Debug for SemanticOntology {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SemanticOntology")
            .field("triple_count", &self.graph.len())
            .finish_non_exhaustive()
    }
}

/// Compatibility parser for compiler-only callers. Serving constructors accept
/// [`SemanticOntology`] so this lossy projection cannot bypass admission.
pub fn tbox_from_turtle(turtle: &str) -> Result<Tbox, String> {
    SemanticOntology::from_turtle(turtle).map(|ontology| ontology.tbox)
}

fn tbox_from_graph(graph: &Graph) -> Tbox {
    let mut tbox = Tbox::new();
    for triple in graph.iter() {
        let NamedOrBlankNodeRef::NamedNode(subject) = triple.subject else {
            continue;
        };
        let TermRef::NamedNode(object) = triple.object else {
            continue;
        };
        match triple.predicate.as_str() {
            RDFS_SUBCLASS_OF => tbox.add_subclass(subject.as_str(), object.as_str()),
            RDFS_SUBPROPERTY_OF => tbox.add_subproperty(subject.as_str(), object.as_str()),
            OWL_INVERSE_OF => tbox.add_inverse(subject.as_str(), object.as_str()),
            RDF_TYPE if object.as_str() == OWL_SYMMETRIC_PROPERTY => {
                tbox.add_symmetric(subject.as_str())
            }
            _ => {}
        }
    }
    tbox
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_declarations_that_the_reasoning_projection_does_not_use() {
        let ttl = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex: <http://ex/> .
ex:Student a owl:Class; rdfs:subClassOf ex:Person .
"#;
        let ontology = SemanticOntology::from_turtle(ttl).unwrap();
        assert_eq!(ontology.graph().len(), 2);
        assert!(ontology
            .tbox()
            .saturate_class("http://ex/Person")
            .contains(&"http://ex/Student".to_owned()));
    }

    #[test]
    fn exact_document_changes_partition_identity_conservatively() {
        let first = SemanticOntology::from_turtle("<urn:C> <urn:p> <urn:O> .").unwrap();
        let second = SemanticOntology::from_turtle("<urn:C>  <urn:p> <urn:O> .").unwrap();
        assert_ne!(first.document_digest(), second.document_digest());
        assert_eq!(first.graph(), second.graph());
    }

    #[test]
    fn malformed_turtle_surfaces_a_redacted_error() {
        let error = SemanticOntology::from_turtle("<postgres://secret@host/x> [").unwrap_err();
        assert!(!error.contains("secret"));
    }

    #[test]
    fn duplicate_hierarchy_statements_are_set_valued() {
        let ttl = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://ex/> .
ex:A rdfs:subClassOf ex:B .
ex:A rdfs:subClassOf ex:B .
"#;
        let tbox = tbox_from_turtle(ttl).unwrap();
        assert_eq!(
            tbox.saturate_class("http://ex/B"),
            vec!["http://ex/B", "http://ex/A"]
        );
    }
}
