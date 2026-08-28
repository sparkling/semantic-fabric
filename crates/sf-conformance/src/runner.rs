//! The per-case CONSTRUCT runner (ADR-0005): build the SQLite fixture, load the
//! mapping (R2RML parsed, or Direct Mapping auto-generated), run
//! `CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }` through the virtualiser, and
//! adjudicate the produced triples against the expected graph by isomorphism,
//! cross-checked through the in-memory oracle.
//!
//! Honesty contract (ADR-0005 *Confirmation*): a feature the engine defers to
//! `501` becomes a documented **skip** (with reason), never a silent pass; an
//! error case that the engine fails to reject, and a positive case that produces
//! the wrong graph, are honest **failures**. Expected outputs are never altered.

use std::path::Path;

use sf_sparql::{exec, parse_and_translate_with, Error as SparqlError, Tbox};
use sf_sql::Dialect;

use crate::graph::{
    has_named_graph, isomorphic, parse_nquads, parse_turtle, quads_to_dataset, triples_to_dataset,
};
use crate::manifest::{Case, Kind};
use crate::oracle;
use crate::{sqlite, CaseResult, Status};

/// The W3C conformance query: the whole virtual graph as a triple dump (ADR-0005).
const DUMP: &str = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";

/// Base IRI fixed by ADR-0005 for both mapping-document parsing and Direct
/// Mapping IRI generation.
const BASE: &str = "http://example.com/base/";

/// Adjudicate one case, given its scenario directory.
pub fn run_case(dir: &Path, case: &Case) -> Result<CaseResult, String> {
    let (status, reason) = match case.kind {
        Kind::R2rml => run_r2rml(dir, case),
        Kind::DirectMapping => run_direct(dir, case),
    }?;
    Ok(CaseResult {
        id: case.identifier.clone(),
        kind: case.kind,
        status,
        reason,
    })
}

fn run_r2rml(dir: &Path, case: &Case) -> Result<(Status, String), String> {
    let sql = read(dir, "create.sql").map_err(|e| input_error(case, "create.sql", &e))?;
    let conn = match sqlite::load(&sql) {
        Ok(c) => c,
        Err(e) => return Ok((Status::Skipped, format!("fixture: {e}"))),
    };
    let doc = case
        .mapping_document
        .as_deref()
        .ok_or_else(|| input_error(case, "mappingDocument", "missing from sealed case"))?;
    let ttl = read(dir, doc).map_err(|e| input_error(case, doc, &e))?;

    let maps = match sf_mapping::parse_r2rml(&ttl) {
        Ok(m) => m,
        Err(e) => return Ok(parse_error_outcome(case, &format!("mapping parse: {e}"))),
    };
    // R2RML §5.1: an R2RML view's SQL query must not produce two columns with the
    // same name — validate each `rr:sqlQuery` source against the live database
    // (the source RDBMS is the SQL authority in a virtualiser).
    if let Err(e) = validate_query_sources(&conn, &maps) {
        return Ok(parse_error_outcome(case, &e));
    }
    // Introspect the live base tables so the ADR-0007 cascade passes (self-join /
    // FD / FK-PK join elimination / redundant-DISTINCT) actually fire over the
    // W3C data — this is the real correctness exercise of those rewrites.
    let schemas = match sqlite::introspect_all(&conn) {
        Ok(s) => s,
        Err(e) => return Ok((Status::Skipped, format!("introspect: {e}"))),
    };
    let plan =
        match parse_and_translate_with(DUMP, &maps, Dialect::Sqlite, &Tbox::default(), &schemas) {
            Ok(p) => p,
            Err(SparqlError::Unsupported(m)) => {
                return Ok((Status::Skipped, format!("501 translate: {m}")))
            }
            Err(e) => return Ok(parse_error_outcome(case, &format!("translate: {e}"))),
        };
    let triples = match exec::construct_triples(&plan, &conn) {
        Ok(t) => t,
        Err(SparqlError::Unsupported(m)) => return Ok((Status::Skipped, format!("501 exec: {m}"))),
        Err(e) => return Ok(parse_error_outcome(case, &format!("exec: {e}"))),
    };

    if !case.has_expected_output {
        return Ok((
            Status::Failed,
            "error case: engine produced output instead of signalling an error".to_owned(),
        ));
    }

    let out = case
        .output
        .as_deref()
        .ok_or_else(|| input_error(case, "output", "missing from sealed positive case"))?;
    let expected_text = read(dir, out).map_err(|e| input_error(case, out, &e))?;
    let expected = parse_nquads(&expected_text)
        .map_err(|e| input_error(case, out, &format!("invalid N-Quads: {e}")))?;
    if has_named_graph(&expected) {
        // `rr:graphMap` named-graph output: the `?s ?p ?o` CONSTRUCT triple dump
        // cannot carry the graph term, so re-run as a mapping-IR **quad** dump
        // (ADR-0005) — the graph term comes from the applicable graph maps via the
        // single `sf-core` term-gen path — and adjudicate the full Dataset (named
        // graphs included) by blank-node isomorphism against the gold N-Quads.
        let quads = match exec::dump_quads(&maps, &conn, Dialect::Sqlite) {
            Ok(q) => q,
            Err(SparqlError::Unsupported(m)) => {
                return Ok((Status::Skipped, format!("501 quad dump: {m}")))
            }
            Err(e) => return Ok(parse_error_outcome(case, &format!("quad dump: {e}"))),
        };
        return Ok(compare_quads(&quads, &expected));
    }
    Ok(compare(&triples, &expected))
}

/// Compare the engine's mapping-IR quad dump to the expected N-Quads by full
/// blank-node-aware Dataset isomorphism (ADR-0005). The expected file is the W3C
/// gold output, so it is the ground truth directly (the default-graph dump oracle
/// does not model named graphs and so does not apply here).
pub(crate) fn compare_quads(quads: &[oxrdf::Quad], expected: &oxrdf::Dataset) -> (Status, String) {
    let engine = quads_to_dataset(quads);
    if isomorphic(&engine, expected) {
        (Status::Passed, String::new())
    } else {
        (
            Status::Failed,
            format!(
                "named-graph mismatch: engine produced {} quads, expected {}",
                engine.len(),
                expected.len()
            ),
        )
    }
}

fn run_direct(dir: &Path, case: &Case) -> Result<(Status, String), String> {
    let sql = read(dir, "create.sql").map_err(|e| input_error(case, "create.sql", &e))?;
    let conn = match sqlite::load(&sql) {
        Ok(c) => c,
        Err(e) => return Ok((Status::Skipped, format!("fixture: {e}"))),
    };
    let schemas = match sqlite::introspect_all(&conn) {
        Ok(s) => s,
        Err(e) => return Ok((Status::Skipped, format!("introspect: {e}"))),
    };
    let maps = match sf_mapping::direct_mapping(&schemas, BASE) {
        Ok(m) => m,
        Err(e) => return Ok((Status::Failed, format!("direct mapping: {e}"))),
    };
    let plan =
        match parse_and_translate_with(DUMP, &maps, Dialect::Sqlite, &Tbox::default(), &schemas) {
            Ok(p) => p,
            Err(SparqlError::Unsupported(m)) => {
                return Ok((Status::Skipped, format!("501 translate: {m}")))
            }
            Err(e) => return Ok((Status::Failed, format!("translate: {e}"))),
        };
    let triples = match exec::construct_triples(&plan, &conn) {
        Ok(t) => t,
        Err(SparqlError::Unsupported(m)) => return Ok((Status::Skipped, format!("501 exec: {m}"))),
        Err(e) => return Ok((Status::Failed, format!("exec: {e}"))),
    };

    if !case.has_expected_output {
        return Ok((
            Status::Failed,
            "error case: engine produced output instead of signalling an error".to_owned(),
        ));
    }
    let out = case
        .output
        .as_deref()
        .ok_or_else(|| input_error(case, "output", "missing from sealed positive case"))?;
    let expected_text = read(dir, out).map_err(|e| input_error(case, out, &e))?;
    let expected = parse_turtle(&expected_text, BASE)
        .map_err(|e| input_error(case, out, &format!("invalid Turtle: {e}")))?;
    Ok(compare(&triples, &expected))
}

pub(crate) fn input_error(case: &Case, input: &str, detail: &str) -> String {
    format!(
        "sealed input error for {} ({input}): {detail}",
        case.identifier
    )
}

/// Compare engine triples to the expected graph **through the oracle** (ADR-0005):
/// the oracle evaluates the dump over the expected store; the engine's live-SQL
/// answer must be isomorphic to it.
pub(crate) fn compare(triples: &[sf_core::Triple], expected: &oxrdf::Dataset) -> (Status, String) {
    let engine = triples_to_dataset(triples);
    let oracle = oracle::evaluate_dump(expected);
    if isomorphic(&engine, &oracle) {
        (Status::Passed, String::new())
    } else {
        (
            Status::Failed,
            format!(
                "graph mismatch: engine produced {} triples, expected {}",
                engine.len(),
                oracle.len()
            ),
        )
    }
}

/// An error during mapping/translate/exec is the *expected* outcome for an error
/// case (PASS) and a genuine failure for a positive case.
pub(crate) fn parse_error_outcome(case: &Case, detail: &str) -> (Status, String) {
    if case.has_expected_output {
        (Status::Failed, detail.to_owned())
    } else {
        (
            Status::Passed,
            format!("error correctly surfaced — {detail}"),
        )
    }
}

/// Validate every `rr:sqlQuery` (R2RML view) source against the live database:
/// preparing the query exposes its result-column names, and R2RML §5.1 makes two
/// identically-named columns a non-conforming mapping (an error). A query that
/// fails to prepare is left to the normal exec path to surface.
fn validate_query_sources(
    conn: &rusqlite::Connection,
    maps: &[sf_core::ir::TriplesMap],
) -> Result<(), String> {
    use sf_core::ir::LogicalSource;
    for map in maps {
        if let LogicalSource::Query(q) = &map.source {
            if let Ok(names) = sf_sql::sqlite_column_names(conn, q) {
                let mut seen = std::collections::HashSet::new();
                for name in &names {
                    if !seen.insert(name.as_str()) {
                        return Err(format!(
                            "rr:sqlQuery produces duplicate column name {name:?} (R2RML §5.1)"
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn read(dir: &Path, file: &str) -> Result<String, String> {
    std::fs::read_to_string(dir.join(file)).map_err(|e| e.to_string())
}

/// Read a per-DBMS **forked** fixture/gold file (ADR-0015): prefer the
/// dialect-suffixed variant (`<stem>.<dialect>.<ext>`, e.g. `create.postgres.sql`)
/// when present, else fall back to the shared `file`. The forked-fixtures layout
/// lets a case whose gold differs by source dialect (CHAR(n) padding, binary
/// rendering) carry a PostgreSQL-specific expectation without disturbing SQLite.
pub(crate) fn read_forked(dir: &Path, file: &str, dialect_tag: &str) -> Result<String, String> {
    if let Some((stem, ext)) = file.rsplit_once('.') {
        let forked = format!("{stem}.{dialect_tag}.{ext}");
        if dir.join(&forked).exists() {
            return read(dir, &forked);
        }
    }
    read(dir, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_sealed_input_is_an_error_not_a_skip() {
        let case = Case {
            identifier: "R2RMLTC0000".to_owned(),
            kind: Kind::R2rml,
            mapping_document: Some("r2rml.ttl".to_owned()),
            output: Some("mapped.nq".to_owned()),
            has_expected_output: true,
        };
        let error = run_case(Path::new("/definitely/not/a/w3c/scenario"), &case)
            .expect_err("missing input must fail closed");
        assert!(error.contains("sealed input error"), "{error}");
        assert!(error.contains("create.sql"), "{error}");
    }
}
