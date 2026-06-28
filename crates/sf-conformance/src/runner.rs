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
pub fn run_case(dir: &Path, case: &Case) -> CaseResult {
    let (status, reason) = match case.kind {
        Kind::R2rml => run_r2rml(dir, case),
        Kind::DirectMapping => run_direct(dir, case),
    };
    CaseResult {
        id: case.identifier.clone(),
        kind: case.kind,
        status,
        reason,
    }
}

fn run_r2rml(dir: &Path, case: &Case) -> (Status, String) {
    let conn = match read(dir, "create.sql").and_then(|s| sqlite::load(&s)) {
        Ok(c) => c,
        Err(e) => return (Status::Skipped, format!("fixture: {e}")),
    };
    let doc = match &case.mapping_document {
        Some(d) => d,
        None => {
            return (
                Status::Skipped,
                "R2RML case without a mapping document".to_owned(),
            )
        }
    };
    let ttl = match read(dir, doc) {
        Ok(t) => t,
        Err(e) => return (Status::Skipped, format!("read {doc}: {e}")),
    };

    let maps = match sf_mapping::parse_r2rml(&ttl) {
        Ok(m) => m,
        Err(e) => return parse_error_outcome(case, &format!("mapping parse: {e}")),
    };
    // R2RML §5.1: an R2RML view's SQL query must not produce two columns with the
    // same name — validate each `rr:sqlQuery` source against the live database
    // (the source RDBMS is the SQL authority in a virtualiser).
    if let Err(e) = validate_query_sources(&conn, &maps) {
        return parse_error_outcome(case, &e);
    }
    // Introspect the live base tables so the ADR-0007 cascade passes (self-join /
    // FD / FK-PK join elimination / redundant-DISTINCT) actually fire over the
    // W3C data — this is the real correctness exercise of those rewrites.
    let schemas = match sqlite::introspect_all(&conn) {
        Ok(s) => s,
        Err(e) => return (Status::Skipped, format!("introspect: {e}")),
    };
    let plan =
        match parse_and_translate_with(DUMP, &maps, Dialect::Sqlite, &Tbox::default(), &schemas) {
            Ok(p) => p,
            Err(SparqlError::Unsupported(m)) => {
                return (Status::Skipped, format!("501 translate: {m}"))
            }
            Err(e) => return parse_error_outcome(case, &format!("translate: {e}")),
        };
    let triples = match exec::construct_triples(&plan, &conn) {
        Ok(t) => t,
        Err(SparqlError::Unsupported(m)) => return (Status::Skipped, format!("501 exec: {m}")),
        Err(e) => return parse_error_outcome(case, &format!("exec: {e}")),
    };

    if !case.has_expected_output {
        return (
            Status::Failed,
            "error case: engine produced output instead of signalling an error".to_owned(),
        );
    }

    let out = match &case.output {
        Some(o) => o,
        None => {
            return (
                Status::Skipped,
                "positive case without an output file".to_owned(),
            )
        }
    };
    let expected = match read(dir, out)
        .map_err(|e| e.to_string())
        .and_then(|t| parse_nquads(&t))
    {
        Ok(d) => d,
        Err(e) => return (Status::Skipped, format!("expected output: {e}")),
    };
    if has_named_graph(&expected) {
        // `rr:graphMap` named-graph output: the `?s ?p ?o` CONSTRUCT triple dump
        // cannot carry the graph term, so re-run as a mapping-IR **quad** dump
        // (ADR-0005) — the graph term comes from the applicable graph maps via the
        // single `sf-core` term-gen path — and adjudicate the full Dataset (named
        // graphs included) by blank-node isomorphism against the gold N-Quads.
        let quads = match exec::dump_quads(&maps, &conn, Dialect::Sqlite) {
            Ok(q) => q,
            Err(SparqlError::Unsupported(m)) => {
                return (Status::Skipped, format!("501 quad dump: {m}"))
            }
            Err(e) => return parse_error_outcome(case, &format!("quad dump: {e}")),
        };
        return compare_quads(&quads, &expected);
    }
    compare(&triples, &expected)
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

fn run_direct(dir: &Path, case: &Case) -> (Status, String) {
    let conn = match read(dir, "create.sql").and_then(|s| sqlite::load(&s)) {
        Ok(c) => c,
        Err(e) => return (Status::Skipped, format!("fixture: {e}")),
    };
    let schemas = match sqlite::introspect_all(&conn) {
        Ok(s) => s,
        Err(e) => return (Status::Skipped, format!("introspect: {e}")),
    };
    let maps = match sf_mapping::direct_mapping(&schemas, BASE) {
        Ok(m) => m,
        Err(e) => return (Status::Failed, format!("direct mapping: {e}")),
    };
    let plan =
        match parse_and_translate_with(DUMP, &maps, Dialect::Sqlite, &Tbox::default(), &schemas) {
            Ok(p) => p,
            Err(SparqlError::Unsupported(m)) => {
                return (Status::Skipped, format!("501 translate: {m}"))
            }
            Err(e) => return (Status::Failed, format!("translate: {e}")),
        };
    let triples = match exec::construct_triples(&plan, &conn) {
        Ok(t) => t,
        Err(SparqlError::Unsupported(m)) => return (Status::Skipped, format!("501 exec: {m}")),
        Err(e) => return (Status::Failed, format!("exec: {e}")),
    };

    if !case.has_expected_output {
        return (
            Status::Failed,
            "error case: engine produced output instead of signalling an error".to_owned(),
        );
    }
    let out = case.output.as_deref().unwrap_or("directGraph.ttl");
    let expected = match read(dir, out)
        .map_err(|e| e.to_string())
        .and_then(|t| parse_turtle(&t, BASE))
    {
        Ok(d) => d,
        Err(e) => return (Status::Skipped, format!("expected output: {e}")),
    };
    compare(&triples, &expected)
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
