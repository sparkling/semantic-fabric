//! EARL report emitter (ADR-0005): write `earl-semantic-fabric-{r2rml,direct}.ttl`,
//! the W3C Evaluation And Report Language assertions — semantic-fabric's entry in
//! the RDB2RDF implementation report (the first Rust implementation).

use std::fmt::Write as _;
use std::path::Path;

use crate::manifest::Kind;
use crate::{CaseResult, Status};

const TEST_NS: &str = "http://www.w3.org/2001/sw/rdb2rdf/test-cases/#";
const SUBJECT: &str = "https://example.org/tools/semantic-fabric";

/// Serialise EARL assertions for every case of `kind` into Turtle.
pub fn to_turtle(cases: &[CaseResult], kind: Kind) -> String {
    let mut out = String::new();
    out.push_str(HEADER);
    let _ = write!(
        out,
        "\n<{SUBJECT}> a earl:TestSubject, doap:Project ;\n  doap:name \"semantic-fabric\" ;\n  doap:programming-language \"Rust\" .\n"
    );
    for c in cases.iter().filter(|c| c.kind == kind) {
        let outcome = match c.status {
            Status::Passed => "earl:passed",
            Status::Failed => "earl:failed",
            Status::Skipped => "earl:untested",
        };
        let _ = write!(
            out,
            "\n[] a earl:Assertion ;\n  earl:assertedBy <{SUBJECT}> ;\n  earl:subject <{SUBJECT}> ;\n  earl:test <{TEST_NS}{id}> ;\n  earl:result [ a earl:TestResult ; earl:outcome {outcome}",
            id = c.id
        );
        if !c.reason.is_empty() {
            let _ = write!(out, " ;\n    dct:description {}", turtle_string(&c.reason));
        }
        out.push_str(" ] .\n");
    }
    out
}

/// Write the EARL report for `kind` to `path`.
pub fn write(cases: &[CaseResult], kind: Kind, path: &Path) -> std::io::Result<()> {
    std::fs::write(path, to_turtle(cases, kind))
}

const HEADER: &str = "@prefix earl: <http://www.w3.org/ns/earl#> .\n\
@prefix doap: <http://usefulinc.com/ns/doap#> .\n\
@prefix dct:  <http://purl.org/dc/terms/> .\n\
@prefix foaf: <http://xmlns.com/foaf/0.1/> .\n";

/// A Turtle-escaped string literal.
fn turtle_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(id: &str, status: Status) -> CaseResult {
        CaseResult {
            id: id.to_owned(),
            kind: Kind::R2rml,
            status,
            reason: "x \"y\"".to_owned(),
        }
    }

    #[test]
    fn emits_well_formed_earl_with_outcomes() {
        let cases = vec![
            case("R2RMLTC0001a", Status::Passed),
            case("R2RMLTC0002c", Status::Failed),
        ];
        let ttl = to_turtle(&cases, Kind::R2rml);
        assert!(ttl.contains("earl:passed"));
        assert!(ttl.contains("earl:failed"));
        assert!(ttl.contains("R2RMLTC0001a"));
        // The EARL Turtle must itself parse (well-formedness).
        assert!(crate::graph::parse_turtle(&ttl, "http://e/").is_ok());
    }
}
