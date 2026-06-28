//! Expected-output parsing and **blank-node-aware graph isomorphism** (ADR-0005).
//!
//! Comparison is *not* byte equality: R2RML / Direct Mapping outputs contain
//! blank nodes whose labels are arbitrary, so two graphs match iff they are
//! isomorphic. We reuse `oxrdf`'s RDF Dataset Canonicalization (`rdfc-10`,
//! ADR-0019): canonicalize both datasets and compare for equality — the same
//! algorithm Oxigraph itself uses, so no second RDF stack enters the harness.

use oxrdf::dataset::CanonicalizationAlgorithm;
use oxrdf::{Dataset, GraphName, GraphNameRef, Quad, Triple};
use oxttl::{NQuadsParser, TurtleParser};

/// Parse N-Quads (the R2RML expected output, named-graph capable) into a dataset.
pub fn parse_nquads(text: &str) -> Result<Dataset, String> {
    let mut quads = Vec::new();
    for q in NQuadsParser::new().for_slice(text) {
        quads.push(q.map_err(|e| format!("N-Quads parse error: {e}"))?);
    }
    Ok(Dataset::from_iter(quads))
}

/// Parse Turtle (the Direct Mapping expected output) into a dataset, placing
/// every triple in the default graph. `base` resolves the relative IRIs the
/// Direct Mapping `directGraph.ttl` files use (`<Student>`); a `@base` directive
/// in the document overrides it.
pub fn parse_turtle(text: &str, base: &str) -> Result<Dataset, String> {
    let parser = TurtleParser::new()
        .with_base_iri(base)
        .map_err(|e| format!("invalid base IRI {base:?}: {e}"))?;
    let mut quads = Vec::new();
    for t in parser.for_slice(text) {
        let Triple {
            subject,
            predicate,
            object,
        } = t.map_err(|e| format!("Turtle parse error: {e}"))?;
        quads.push(Quad::new(
            subject,
            predicate,
            object,
            GraphName::DefaultGraph,
        ));
    }
    Ok(Dataset::from_iter(quads))
}

/// Build a dataset (default graph) from engine-produced triples.
pub fn triples_to_dataset(triples: &[Triple]) -> Dataset {
    Dataset::from_iter(triples.iter().map(|t| {
        Quad::new(
            t.subject.clone(),
            t.predicate.clone(),
            t.object.clone(),
            GraphName::DefaultGraph,
        )
    }))
}

/// Build a dataset (named graphs included) from engine-produced quads — the
/// mapping-IR quad dump's adjudication form (ADR-0005 `rr:graphMap` output).
pub fn quads_to_dataset(quads: &[Quad]) -> Dataset {
    Dataset::from_iter(quads.iter().cloned())
}

/// Does the dataset assign any triple to a **named** graph? The `?s ?p ?o`
/// CONSTRUCT dump emits only the default graph, so a case whose expected output
/// uses `rr:graphMap` (named quads) is outside the dump's reach (ADR-0005).
pub fn has_named_graph(ds: &Dataset) -> bool {
    ds.iter()
        .any(|q| q.graph_name != GraphNameRef::DefaultGraph)
}

/// True iff `a` and `b` are isomorphic (blank-node-aware), via RDFC-1.0
/// canonicalization (ADR-0005 / ADR-0019).
pub fn isomorphic(a: &Dataset, b: &Dataset) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut a = a.clone();
    let mut b = b.clone();
    a.canonicalize(CanonicalizationAlgorithm::Unstable);
    b.canonicalize(CanonicalizationAlgorithm::Unstable);
    a == b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isomorphism_is_blank_node_label_independent() {
        let g1 = parse_turtle(
            "@prefix : <http://e/> . _:a :p :o . :s :q _:a .",
            "http://e/",
        )
        .unwrap();
        // Same shape, different blank-node label ⇒ isomorphic.
        let g2 = parse_turtle(
            "@prefix : <http://e/> . _:zzz :p :o . :s :q _:zzz .",
            "http://e/",
        )
        .unwrap();
        assert!(isomorphic(&g1, &g2));
    }

    #[test]
    fn distinct_graphs_are_not_isomorphic() {
        let g1 = parse_turtle("@prefix : <http://e/> . :s :p :o .", "http://e/").unwrap();
        let g2 = parse_turtle("@prefix : <http://e/> . :s :p :other .", "http://e/").unwrap();
        assert!(!isomorphic(&g1, &g2));
    }

    #[test]
    fn detects_named_graph_quads() {
        let dg = parse_nquads("<http://e/s> <http://e/p> <http://e/o> .").unwrap();
        assert!(!has_named_graph(&dg));
        let ng = parse_nquads("<http://e/s> <http://e/p> <http://e/o> <http://e/g> .").unwrap();
        assert!(has_named_graph(&ng));
    }
}
