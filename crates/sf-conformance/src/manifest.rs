//! Parse a W3C RDB2RDF `manifest.ttl` into the per-case metadata the harness
//! drives (ADR-0005). A manifest declares, per database scenario, a set of
//! `rdb2rdftest:R2RML` and `rdb2rdftest:DirectMapping` tests, each pointing at a
//! mapping document and an expected-output file, and flagging error cases via
//! `rdb2rdftest:hasExpectedOutput false`.

use std::collections::HashMap;

use oxttl::TurtleParser;
use oxrdf::{NamedOrBlankNode, Term};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const NS: &str = "http://purl.org/NET/rdb2rdf-test#";
const DC_IDENTIFIER: &str = "http://purl.org/dc/elements/1.1/identifier";

/// The kind of mapping a test exercises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `rdb2rdftest:R2RML` — an R2RML mapping document → N-Quads output.
    R2rml,
    /// `rdb2rdftest:DirectMapping` — the auto-generated-R2RML path → Turtle output.
    DirectMapping,
}

/// One adjudicable test case from a manifest.
#[derive(Debug, Clone)]
pub struct Case {
    /// `dcterms:identifier`, e.g. `R2RMLTC0001a` / `DirectGraphTC0001`.
    pub identifier: String,
    pub kind: Kind,
    /// `rdb2rdftest:mappingDocument` filename (R2RML only; Direct Mapping is
    /// auto-generated from the schema, so it has none).
    pub mapping_document: Option<String>,
    /// `rdb2rdftest:output` filename (`None` for an error case).
    pub output: Option<String>,
    /// `rdb2rdftest:hasExpectedOutput` — `false` marks an error case (the
    /// processor must signal an error rather than produce output).
    pub has_expected_output: bool,
}

/// Parse a manifest's test cases, in identifier order (deterministic).
pub fn parse(turtle: &str) -> Result<Vec<Case>, String> {
    let g = Graph::load(turtle)?;
    let mut cases: Vec<Case> = g
        .subjects()
        .filter_map(|s| case_of(&g, s))
        .collect();
    cases.sort_by(|a, b| a.identifier.cmp(&b.identifier));
    Ok(cases)
}

fn case_of(g: &Graph, s: &NamedOrBlankNode) -> Option<Case> {
    let kind = match g.iri_object(s, RDF_TYPE)?.as_str() {
        t if t == format!("{NS}R2RML") => Kind::R2rml,
        t if t == format!("{NS}DirectMapping") => Kind::DirectMapping,
        _ => return None,
    };
    let identifier = g.string(s, DC_IDENTIFIER)?;
    let has_expected_output = g
        .string(s, &format!("{NS}hasExpectedOutput"))
        .map(|v| v == "true")
        .unwrap_or(true);
    let output = g.string(s, &format!("{NS}output"));
    let mapping_document = g.string(s, &format!("{NS}mappingDocument"));
    Some(Case {
        identifier,
        kind,
        mapping_document,
        output,
        has_expected_output,
    })
}

/// A minimal subject-indexed view of the manifest graph.
struct Graph {
    spo: HashMap<NamedOrBlankNode, Vec<(String, Term)>>,
}

impl Graph {
    fn load(turtle: &str) -> Result<Self, String> {
        // The manifest's own `@base` applies; a fallback keeps relative subjects
        // resolvable even if a file omits it.
        let parser = TurtleParser::new()
            .with_base_iri("http://www.w3.org/2001/sw/rdb2rdf/test-cases/")
            .map_err(|e| format!("invalid manifest base: {e}"))?;
        let mut spo: HashMap<NamedOrBlankNode, Vec<(String, Term)>> = HashMap::new();
        for t in parser.for_slice(turtle) {
            let t = t.map_err(|e| format!("manifest parse error: {e}"))?;
            spo.entry(t.subject)
                .or_default()
                .push((t.predicate.as_str().to_owned(), t.object));
        }
        Ok(Self { spo })
    }

    fn subjects(&self) -> impl Iterator<Item = &NamedOrBlankNode> {
        self.spo.keys()
    }

    fn object<'a>(&'a self, s: &NamedOrBlankNode, p: &str) -> Option<&'a Term> {
        self.spo
            .get(s)?
            .iter()
            .find(|(pred, _)| pred == p)
            .map(|(_, o)| o)
    }

    fn iri_object<'a>(&'a self, s: &NamedOrBlankNode, p: &str) -> Option<&'a oxrdf::NamedNode> {
        match self.object(s, p)? {
            Term::NamedNode(n) => Some(n),
            _ => None,
        }
    }

    fn string(&self, s: &NamedOrBlankNode, p: &str) -> Option<String> {
        match self.object(s, p)? {
            Term::Literal(l) => Some(l.value().to_owned()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: &str = r#"
@prefix test: <http://www.w3.org/2006/03/test-description#> .
@prefix dcterms: <http://purl.org/dc/elements/1.1/> .
@prefix rdb2rdftest: <http://purl.org/NET/rdb2rdf-test#> .
@base <http://www.w3.org/2001/sw/rdb2rdf/test-cases/#> .
<tc1> a rdb2rdftest:R2RML ; dcterms:identifier "R2RMLTC0001a" ;
   rdb2rdftest:output "mappeda.nq" ; rdb2rdftest:hasExpectedOutput true ;
   rdb2rdftest:mappingDocument "r2rmla.ttl" .
<tcErr> a rdb2rdftest:R2RML ; dcterms:identifier "R2RMLTC0001z" ;
   rdb2rdftest:hasExpectedOutput false ; rdb2rdftest:mappingDocument "r2rmlz.ttl" .
<dg1> a rdb2rdftest:DirectMapping ; dcterms:identifier "DirectGraphTC0001" ;
   rdb2rdftest:output "directGraph.ttl" ; rdb2rdftest:hasExpectedOutput true .
"#;

    #[test]
    fn parses_r2rml_directmapping_and_error_cases() {
        let cases = parse(M).unwrap();
        assert_eq!(cases.len(), 3);
        let tc = cases.iter().find(|c| c.identifier == "R2RMLTC0001a").unwrap();
        assert_eq!(tc.kind, Kind::R2rml);
        assert_eq!(tc.mapping_document.as_deref(), Some("r2rmla.ttl"));
        assert_eq!(tc.output.as_deref(), Some("mappeda.nq"));
        assert!(tc.has_expected_output);

        let err = cases.iter().find(|c| c.identifier == "R2RMLTC0001z").unwrap();
        assert!(!err.has_expected_output);
        assert!(err.output.is_none());

        let dg = cases.iter().find(|c| c.kind == Kind::DirectMapping).unwrap();
        assert_eq!(dg.output.as_deref(), Some("directGraph.ttl"));
    }
}
