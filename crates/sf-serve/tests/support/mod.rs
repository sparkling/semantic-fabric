//! Semantic fixtures for HTTP/transport tests whose subject is not admission.
//!
//! These declarations are generated only inside test binaries. Product code
//! never derives ontology authority from a mapping.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use sf_core::ir::{TermMap, TriplesMap};
use sf_core::Term;
use sf_serve::{Backend, IntrospectedSource, SemanticOntology, ServeConfig};

#[allow(
    dead_code,
    reason = "each integration-test binary compiles this shared fixture independently"
)]
pub fn serve_config(backend: Backend, mapping_turtle: &str) -> ServeConfig {
    let mapping = sf_mapping::parse_r2rml(mapping_turtle).expect("parse test mapping");
    let ontology = ontology_for_mapping(&mapping);
    let source = IntrospectedSource::observe_sqlite(backend).expect("observe SQLite test source");
    let mut cfg = ServeConfig::from_authored_r2rml(source, mapping_turtle, ontology)
        .expect("test mapping has matching test-only ontology declarations");
    cfg.set_query_admission(sf_serve::QueryAdmission::UnrestrictedDevelopment);
    cfg.set_parser_runtime(parser_runtime());
    cfg
}

#[allow(dead_code)]
pub fn parser_runtime() -> sf_sparql::ParserRuntime {
    static PARSER: std::sync::OnceLock<sf_sparql::ParserRuntime> = std::sync::OnceLock::new();
    PARSER
        .get_or_init(|| {
            sf_sparql::ParserRuntime::prepare(std::path::Path::new(env!(
                "CARGO_BIN_EXE_semantic-fabric-parser"
            )))
            .expect("the explicit Rust parser host must be available")
        })
        .clone()
}

pub fn ontology_for_mapping(mapping: &[TriplesMap]) -> SemanticOntology {
    let mut classes = BTreeSet::new();
    let mut properties = BTreeSet::new();
    for triples_map in mapping {
        classes.extend(
            triples_map
                .subject
                .classes
                .iter()
                .map(|class| class.as_str().to_owned()),
        );
        for predicate_object_map in &triples_map.predicate_object_maps {
            for predicate in &predicate_object_map.predicates {
                if let TermMap::Constant(Term::NamedNode(predicate)) = predicate {
                    properties.insert(predicate.as_str().to_owned());
                }
            }
        }
    }

    let mut turtle = String::from(
        "@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n\
         @prefix owl: <http://www.w3.org/2002/07/owl#> .\n",
    );
    for class in classes {
        writeln!(turtle, "<{class}> a owl:Class .").expect("write to String");
    }
    for property in properties {
        writeln!(turtle, "<{property}> a rdf:Property .").expect("write to String");
    }
    SemanticOntology::from_turtle(&turtle).expect("generated test ontology is valid Turtle")
}
