//! Build a tier-1 [`Tbox`] (ADR-0008) from an optional ontology Turtle document.
//!
//! The runtime reasoning lane is OWL-RL Safe-Group / RDFS tier-1 only (ADR-0008):
//! we extract exactly the axioms the saturator consumes — `rdfs:subClassOf`,
//! `rdfs:subPropertyOf`, `owl:inverseOf`, and `owl:SymmetricProperty` — and ignore
//! everything else (no DL reasoner is wired at runtime). A `--ontology` flag is
//! therefore honoured, not silently dropped.

use oxrdf::{NamedOrBlankNode, Term};
use oxttl::TurtleParser;
use sf_sparql::Tbox;

const RDFS_SUBCLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const RDFS_SUBPROPERTY_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subPropertyOf";
const OWL_INVERSE_OF: &str = "http://www.w3.org/2002/07/owl#inverseOf";
const OWL_SYMMETRIC_PROPERTY: &str = "http://www.w3.org/2002/07/owl#SymmetricProperty";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// Parse `turtle` and populate a [`Tbox`] with the tier-1 RDFS/OWL-RL axioms the
/// saturator understands. Subjects/objects that are not IRIs are skipped (the
/// saturator keys on IRI predicate/class names).
pub fn tbox_from_turtle(turtle: &str) -> Result<Tbox, String> {
    let mut tbox = Tbox::new();
    for triple in TurtleParser::new().for_slice(turtle) {
        let t = triple.map_err(|e| format!("ontology Turtle parse error: {e}"))?;
        let subject = match &t.subject {
            NamedOrBlankNode::NamedNode(n) => n.as_str().to_owned(),
            NamedOrBlankNode::BlankNode(_) => continue,
        };
        let object_iri = match &t.object {
            Term::NamedNode(n) => Some(n.as_str().to_owned()),
            _ => None,
        };
        match t.predicate.as_str() {
            RDFS_SUBCLASS_OF => {
                if let Some(o) = object_iri {
                    tbox.add_subclass(subject, o);
                }
            }
            RDFS_SUBPROPERTY_OF => {
                if let Some(o) = object_iri {
                    tbox.add_subproperty(subject, o);
                }
            }
            OWL_INVERSE_OF => {
                if let Some(o) = object_iri {
                    tbox.add_inverse(subject, o);
                }
            }
            RDF_TYPE if object_iri.as_deref() == Some(OWL_SYMMETRIC_PROPERTY) => {
                tbox.add_symmetric(subject);
            }
            _ => {}
        }
    }
    Ok(tbox)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_tier1_axioms() {
        let ttl = r#"
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            @prefix owl: <http://www.w3.org/2002/07/owl#> .
            @prefix ex: <http://ex/> .
            ex:Student rdfs:subClassOf ex:Person .
            ex:knows a owl:SymmetricProperty .
            ex:parentOf owl:inverseOf ex:childOf .
        "#;
        let tbox = tbox_from_turtle(ttl).unwrap();
        assert!(tbox
            .saturate_class("http://ex/Person")
            .contains(&"http://ex/Student".to_owned()));
        assert!(tbox
            .inverse_predicates("http://ex/parentOf")
            .contains(&"http://ex/childOf".to_owned()));
    }

    #[test]
    fn malformed_turtle_surfaces_as_an_err_not_a_panic() {
        let err = tbox_from_turtle("this is not valid turtle @@@ ][ .").unwrap_err();
        assert!(
            err.contains("ontology Turtle parse error"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn empty_document_yields_an_empty_tbox() {
        let tbox = tbox_from_turtle("").unwrap();
        assert!(tbox.is_empty());
    }

    #[test]
    fn blank_node_subject_is_skipped_not_an_error() {
        // A blank-node subject can never be a class/property IRI the saturator
        // keys on — `tbox_from_turtle` must skip it (continue) rather than error
        // or attempt to stringify the blank node id as an IRI.
        let ttl = r#"
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            @prefix ex: <http://ex/> .
            _:b1 rdfs:subClassOf ex:Person .
        "#;
        let tbox = tbox_from_turtle(ttl).unwrap();
        assert!(
            tbox.is_empty(),
            "a blank-node-subject triple must contribute no axiom"
        );
    }

    #[test]
    fn non_iri_object_is_skipped_not_an_error() {
        // The saturator's axioms (subClassOf/subPropertyOf/inverseOf) all key on
        // an IRI OBJECT; a literal object on one of these predicates has no
        // representation and must be skipped, not misinterpreted as an IRI or
        // cause an error.
        let ttl = r#"
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            @prefix ex: <http://ex/> .
            ex:Student rdfs:subClassOf "not an IRI" .
        "#;
        let tbox = tbox_from_turtle(ttl).unwrap();
        assert!(
            tbox.is_empty(),
            "a literal-object subClassOf triple must contribute no axiom"
        );
    }

    #[test]
    fn irrelevant_predicates_are_ignored() {
        // Any triple whose predicate isn't one of the 4 tier-1 axiom predicates
        // (or rdf:type owl:SymmetricProperty) contributes nothing — confirms the
        // `_ => {}` catch-all doesn't accidentally swallow-as-error or otherwise
        // misbehave on ordinary application data mixed into the same document.
        let ttl = r#"
            @prefix ex: <http://ex/> .
            ex:Student ex:enrolledIn ex:CS101 .
        "#;
        let tbox = tbox_from_turtle(ttl).unwrap();
        assert!(tbox.is_empty());
    }
}
