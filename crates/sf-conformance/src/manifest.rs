//! Parse a W3C RDB2RDF `manifest.ttl` into the per-case metadata the harness
//! drives (ADR-0005). A manifest declares, per database scenario, a set of
//! `rdb2rdftest:R2RML` and `rdb2rdftest:DirectMapping` tests, each pointing at a
//! mapping document and an expected-output file, and flagging error cases via
//! `rdb2rdftest:hasExpectedOutput false`.

use std::collections::{HashMap, HashSet};

use oxrdf::{NamedOrBlankNode, Term};
use oxttl::TurtleParser;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
const RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
const RDF_NIL: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil";
const NS: &str = "http://purl.org/NET/rdb2rdf-test#";
const DC_IDENTIFIER: &str = "http://purl.org/dc/elements/1.1/identifier";
const MF_INCLUDE: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#include";
const MF_MANIFEST: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#Manifest";
const DEFAULT_BASE: &str = "http://www.w3.org/2001/sw/rdb2rdf/test-cases/";

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
    let mut cases: Vec<Case> = g.subjects().filter_map(|s| case_of(&g, s)).collect();
    cases.sort_by(|a, b| a.identifier.cmp(&b.identifier));
    Ok(cases)
}

/// Parse every declared RDB2RDF case without silently dropping incomplete or
/// contradictory entries. This is the fail-closed path used when sealing the
/// vendored standards corpus; the execution harness retains [`parse`] until it
/// is wired to the canonical inventory in a separate change.
pub fn parse_strict(turtle: &str) -> Result<Vec<Case>, String> {
    let g = Graph::load(turtle)?;
    let mut cases = Vec::new();
    for subject in g.subjects() {
        if let Some(case) = strict_case_of(&g, subject)? {
            cases.push(case);
        }
    }
    if cases.is_empty() {
        return Err("manifest declares no RDB2RDF cases".to_owned());
    }
    cases.sort_by(|a, b| a.identifier.cmp(&b.identifier));
    for pair in cases.windows(2) {
        if pair[0].identifier == pair[1].identifier {
            return Err(format!("duplicate case identifier {}", pair[0].identifier));
        }
    }
    Ok(cases)
}

/// Parse the ordered scenario-manifest paths from the suite-level RDF list.
pub fn parse_suite_manifest(turtle: &str) -> Result<Vec<String>, String> {
    let g = Graph::load(turtle)?;
    let includes: Vec<_> = g
        .spo
        .iter()
        .flat_map(|(subject, values)| {
            values
                .iter()
                .filter(|(predicate, _)| predicate == MF_INCLUDE)
                .map(move |(_, object)| (subject, object))
        })
        .collect();
    let [(manifest_subject, head)] = includes.as_slice() else {
        return Err(format!(
            "suite manifest must declare exactly one mf:include list; found {}",
            includes.len()
        ));
    };
    let is_manifest = g
        .objects(manifest_subject, RDF_TYPE)
        .iter()
        .any(|term| matches!(term, Term::NamedNode(node) if node.as_str() == MF_MANIFEST));
    if !is_manifest {
        return Err("mf:include subject is not typed mf:Manifest".to_owned());
    }

    let mut entries = Vec::new();
    let mut cursor = (*head).clone();
    let mut seen = HashSet::new();
    loop {
        if matches!(&cursor, Term::NamedNode(node) if node.as_str() == RDF_NIL) {
            break;
        }
        let subject = match cursor {
            Term::NamedNode(node) => NamedOrBlankNode::NamedNode(node),
            Term::BlankNode(node) => NamedOrBlankNode::BlankNode(node),
            _ => return Err("mf:include list node is not an RDF resource".to_owned()),
        };
        if !seen.insert(subject.to_string()) {
            return Err("mf:include list contains a cycle".to_owned());
        }
        let first = single_object(&g, &subject, RDF_FIRST, "rdf:first")?;
        let Term::NamedNode(entry) = first else {
            return Err("mf:include entry is not an IRI".to_owned());
        };
        let path = entry
            .as_str()
            .strip_prefix(DEFAULT_BASE)
            .ok_or_else(|| format!("mf:include entry {} is outside the suite base", entry))?;
        if path.is_empty() {
            return Err("mf:include entry has an empty relative path".to_owned());
        }
        entries.push(path.to_owned());
        cursor = single_object(&g, &subject, RDF_REST, "rdf:rest")?.clone();
    }
    if entries.is_empty() {
        return Err("mf:include list is empty".to_owned());
    }
    Ok(entries)
}

fn single_object<'a>(
    g: &'a Graph,
    subject: &NamedOrBlankNode,
    predicate: &str,
    label: &str,
) -> Result<&'a Term, String> {
    let values = g.objects(subject, predicate);
    match values.as_slice() {
        [value] => Ok(*value),
        _ => Err(format!(
            "suite manifest list node {subject} must have exactly one {label}; found {}",
            values.len()
        )),
    }
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

fn strict_case_of(g: &Graph, s: &NamedOrBlankNode) -> Result<Option<Case>, String> {
    let mut kinds = Vec::new();
    for object in g.objects(s, RDF_TYPE) {
        let Term::NamedNode(node) = object else {
            continue;
        };
        match node.as_str() {
            t if t == format!("{NS}R2RML") => kinds.push(Kind::R2rml),
            t if t == format!("{NS}DirectMapping") => kinds.push(Kind::DirectMapping),
            _ => {}
        }
    }
    let Some(kind) = kinds.first().copied() else {
        return Ok(None);
    };
    if kinds.len() != 1 {
        return Err(format!("case {s} has multiple RDB2RDF types"));
    }

    let identifier = g.required_string(s, DC_IDENTIFIER, "identifier")?;
    if identifier.is_empty() {
        return Err(format!("case {s} has an empty identifier"));
    }
    let expected = g.required_string(s, &format!("{NS}hasExpectedOutput"), "hasExpectedOutput")?;
    let has_expected_output = match expected.as_str() {
        "true" => true,
        "false" => false,
        value => {
            return Err(format!(
                "case {identifier} has invalid hasExpectedOutput value {value:?}"
            ))
        }
    };
    let output = g.optional_string(s, &format!("{NS}output"))?;
    let mapping_document = g.optional_string(s, &format!("{NS}mappingDocument"))?;

    match (kind, mapping_document.as_ref()) {
        (Kind::R2rml, None) => return Err(format!("case {identifier} has no mappingDocument")),
        (Kind::DirectMapping, Some(_)) => {
            return Err(format!(
                "direct-mapping case {identifier} unexpectedly has a mappingDocument"
            ))
        }
        _ => {}
    }
    match (has_expected_output, output.as_ref()) {
        (true, None) => return Err(format!("positive case {identifier} has no output")),
        (false, Some(_)) => {
            return Err(format!(
                "error case {identifier} unexpectedly has an output"
            ))
        }
        _ => {}
    }

    Ok(Some(Case {
        identifier,
        kind,
        mapping_document,
        output,
        has_expected_output,
    }))
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
            .with_base_iri(DEFAULT_BASE)
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

    fn objects<'a>(&'a self, s: &NamedOrBlankNode, p: &str) -> Vec<&'a Term> {
        self.spo
            .get(s)
            .into_iter()
            .flatten()
            .filter(|(pred, _)| pred == p)
            .map(|(_, object)| object)
            .collect()
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

    fn optional_string(&self, s: &NamedOrBlankNode, p: &str) -> Result<Option<String>, String> {
        let values = self.objects(s, p);
        match values.as_slice() {
            [] => Ok(None),
            [Term::Literal(value)] => Ok(Some(value.value().to_owned())),
            [_] => Err(format!("case {s} property {p} must be a literal")),
            _ => Err(format!("case {s} property {p} occurs more than once")),
        }
    }

    fn required_string(
        &self,
        s: &NamedOrBlankNode,
        p: &str,
        label: &str,
    ) -> Result<String, String> {
        self.optional_string(s, p)?
            .ok_or_else(|| format!("case {s} has no {label}"))
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
        let tc = cases
            .iter()
            .find(|c| c.identifier == "R2RMLTC0001a")
            .unwrap();
        assert_eq!(tc.kind, Kind::R2rml);
        assert_eq!(tc.mapping_document.as_deref(), Some("r2rmla.ttl"));
        assert_eq!(tc.output.as_deref(), Some("mappeda.nq"));
        assert!(tc.has_expected_output);

        let err = cases
            .iter()
            .find(|c| c.identifier == "R2RMLTC0001z")
            .unwrap();
        assert!(!err.has_expected_output);
        assert!(err.output.is_none());

        let dg = cases
            .iter()
            .find(|c| c.kind == Kind::DirectMapping)
            .unwrap();
        assert_eq!(dg.output.as_deref(), Some("directGraph.ttl"));
    }

    #[test]
    fn strict_parse_rejects_incomplete_declared_case() {
        let malformed = M.replace(
            "dcterms:identifier \"R2RMLTC0001a\" ;",
            "dcterms:title \"missing identifier\" ;",
        );
        let error = parse_strict(&malformed).unwrap_err();
        assert!(error.contains("has no identifier"), "{error}");
    }

    #[test]
    fn strict_parse_rejects_duplicate_identifier() {
        let duplicate = M.replace("R2RMLTC0001z", "R2RMLTC0001a");
        let error = parse_strict(&duplicate).unwrap_err();
        assert!(error.contains("duplicate case identifier"), "{error}");
    }
}
