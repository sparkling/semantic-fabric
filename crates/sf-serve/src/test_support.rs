use std::fmt::Write as _;

use crate::SemanticOntology;

pub(crate) fn empty_ontology() -> SemanticOntology {
    SemanticOntology::from_turtle("").expect("empty test ontology is valid Turtle")
}

pub(crate) fn ontology(classes: &[&str], properties: &[&str]) -> SemanticOntology {
    let mut turtle = String::from(
        "@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n\
         @prefix owl: <http://www.w3.org/2002/07/owl#> .\n",
    );
    for class in classes {
        oxrdf::NamedNode::new(*class).expect("test class is an absolute IRI");
        writeln!(turtle, "<{class}> a owl:Class .").expect("write to String");
    }
    for property in properties {
        oxrdf::NamedNode::new(*property).expect("test property is an absolute IRI");
        writeln!(turtle, "<{property}> a rdf:Property .").expect("write to String");
    }
    SemanticOntology::from_turtle(&turtle).expect("generated test ontology is valid Turtle")
}
