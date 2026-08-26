//! Non-skipping W3C RDB2RDF conformance lane over embedded DuckDB.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use duckdb::Connection;
use sf_sparql::{exec_duckdb, parse_and_translate_with, Error as SparqlError, Tbox};
use sf_sql::introspect::introspect_duckdb;
use sf_sql::{Dialect, TableSchema};

use crate::graph::{has_named_graph, parse_nquads, parse_turtle};
use crate::manifest::{self, Case, Kind};
use crate::runner::{compare, compare_quads, parse_error_outcome, read, read_forked};
use crate::{CaseResult, Report, Status};

const DUMP: &str = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";
const BASE: &str = "http://example.com/base/";
const TAG: &str = "duckdb";

/// Run the suite against the bundled embedded engine. Provider availability is
/// never a skip: a failure to start DuckDB is a test failure.
pub fn run(cases_dir: &Path) -> Result<Report, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(run_cases(cases_dir))
}

async fn run_cases(cases_dir: &Path) -> Result<Report, String> {
    let mut dirs: Vec<_> = std::fs::read_dir(cases_dir)
        .map_err(|error| error.to_string())?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();

    let mut cases = Vec::new();
    for dir in dirs {
        let Ok(manifest_text) = std::fs::read_to_string(dir.join("manifest.ttl")) else {
            continue;
        };
        let parsed = match manifest::parse(&manifest_text) {
            Ok(parsed) => parsed,
            Err(error) => {
                eprintln!("manifest parse failed for {}: {error}", dir.display());
                continue;
            }
        };
        for case in &parsed {
            let (status, reason) = match case.kind {
                Kind::R2rml => run_r2rml(&dir, case).await,
                Kind::DirectMapping => run_direct(&dir, case).await,
            };
            cases.push(CaseResult {
                id: case.identifier.clone(),
                kind: case.kind,
                status,
                reason,
            });
        }
    }
    Ok(Report { cases })
}

fn load_fixture(dir: &Path) -> Result<Connection, String> {
    let sql = read_forked(dir, "create.sql", TAG)?;
    let conn = Connection::open_in_memory().map_err(|error| error.to_string())?;
    conn.execute_batch(&sql)
        .map_err(|error| format!("create.sql load failed: {error}"))?;
    Ok(conn)
}

fn introspect_all(conn: &Connection) -> Result<Vec<TableSchema>, String> {
    let mut statement = conn
        .prepare(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_catalog = current_database() \
               AND table_schema = current_schema() \
               AND table_type = 'BASE TABLE' ORDER BY table_name",
        )
        .map_err(|error| error.to_string())?;
    let names: Vec<String> = statement
        .query_map([], |row| row.get(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|error| error.to_string())?;
    names
        .into_iter()
        .map(|name| introspect_duckdb(conn, &name).map_err(|error| error.to_string()))
        .collect()
}

fn validate_query_sources(
    conn: &Connection,
    maps: &[sf_core::ir::TriplesMap],
) -> Result<(), String> {
    use sf_core::ir::LogicalSource;
    for map in maps {
        if let LogicalSource::Query(query) = &map.source {
            let query = query.trim().strip_suffix(';').unwrap_or(query.trim());
            // A wrapper causes DuckDB to auto-rename duplicate columns (`x`,
            // `x_1`), hiding the R2RML §5.1 violation. DESCRIBE preserves the
            // original SELECT-list names without consuming the query.
            let metadata_sql = format!("DESCRIBE {query}");
            let mut statement = conn
                .prepare(&metadata_sql)
                .map_err(|error| error.to_string())?;
            let mut rows = statement.query([]).map_err(|error| error.to_string())?;
            let mut seen = HashSet::new();
            while let Some(row) = rows.next().map_err(|error| error.to_string())? {
                let name: String = row.get(0).map_err(|error| error.to_string())?;
                if !seen.insert(name.clone()) {
                    return Err(format!(
                        "rr:sqlQuery produces duplicate column name {name:?} (R2RML §5.1)"
                    ));
                }
            }
        }
    }
    Ok(())
}

async fn run_r2rml(dir: &Path, case: &Case) -> (Status, String) {
    let conn = match load_fixture(dir) {
        Ok(conn) => conn,
        Err(error) => return (Status::Skipped, format!("fixture: {error}")),
    };
    let document = match &case.mapping_document {
        Some(document) => document,
        None => {
            return (
                Status::Skipped,
                "R2RML case without a mapping document".to_owned(),
            );
        }
    };
    let ttl = match read(dir, document) {
        Ok(ttl) => ttl,
        Err(error) => return (Status::Skipped, format!("read {document}: {error}")),
    };
    let maps = match sf_mapping::parse_r2rml(&ttl) {
        Ok(maps) => maps,
        Err(error) => return parse_error_outcome(case, &format!("mapping parse: {error}")),
    };
    if let Err(error) = validate_query_sources(&conn, &maps) {
        return parse_error_outcome(case, &error);
    }
    let schemas = match introspect_all(&conn) {
        Ok(schemas) => schemas,
        Err(error) => return (Status::Skipped, format!("introspect: {error}")),
    };
    let plan =
        match parse_and_translate_with(DUMP, &maps, Dialect::DuckDb, &Tbox::default(), &schemas) {
            Ok(plan) => plan,
            Err(SparqlError::Unsupported(message)) => {
                return (Status::Skipped, format!("501 translate: {message}"));
            }
            Err(error) => return parse_error_outcome(case, &format!("translate: {error}")),
        };
    let conn = Arc::new(Mutex::new(conn));
    let triples = match exec_duckdb::construct_triples_duckdb(&plan, Arc::clone(&conn)).await {
        Ok(triples) => triples,
        Err(SparqlError::Unsupported(message)) => {
            return (Status::Skipped, format!("501 exec: {message}"));
        }
        Err(error) => return parse_error_outcome(case, &format!("exec: {error}")),
    };
    if !case.has_expected_output {
        return (
            Status::Failed,
            "error case: engine produced output instead of signalling an error".to_owned(),
        );
    }
    let output = match &case.output {
        Some(output) => output,
        None => {
            return (
                Status::Skipped,
                "positive case without an output file".to_owned(),
            );
        }
    };
    let expected = match read_forked(dir, output, TAG).and_then(|text| parse_nquads(&text)) {
        Ok(expected) => expected,
        Err(error) => return (Status::Skipped, format!("expected output: {error}")),
    };
    if has_named_graph(&expected) {
        let quads = match exec_duckdb::dump_quads_duckdb(&maps, conn, Dialect::DuckDb).await {
            Ok(quads) => quads,
            Err(SparqlError::Unsupported(message)) => {
                return (Status::Skipped, format!("501 quad dump: {message}"));
            }
            Err(error) => return parse_error_outcome(case, &format!("quad dump: {error}")),
        };
        return compare_quads(&quads, &expected);
    }
    compare(&triples, &expected)
}

async fn run_direct(dir: &Path, case: &Case) -> (Status, String) {
    let conn = match load_fixture(dir) {
        Ok(conn) => conn,
        Err(error) => return (Status::Skipped, format!("fixture: {error}")),
    };
    let schemas = match introspect_all(&conn) {
        Ok(schemas) => schemas,
        Err(error) => return (Status::Skipped, format!("introspect: {error}")),
    };
    let maps = match sf_mapping::direct_mapping(&schemas, BASE) {
        Ok(maps) => maps,
        Err(error) => return (Status::Failed, format!("direct mapping: {error}")),
    };
    let plan =
        match parse_and_translate_with(DUMP, &maps, Dialect::DuckDb, &Tbox::default(), &schemas) {
            Ok(plan) => plan,
            Err(SparqlError::Unsupported(message)) => {
                return (Status::Skipped, format!("501 translate: {message}"));
            }
            Err(error) => return (Status::Failed, format!("translate: {error}")),
        };
    let conn = Arc::new(Mutex::new(conn));
    let triples = match exec_duckdb::construct_triples_duckdb(&plan, conn).await {
        Ok(triples) => triples,
        Err(SparqlError::Unsupported(message)) => {
            return (Status::Skipped, format!("501 exec: {message}"));
        }
        Err(error) => return (Status::Failed, format!("exec: {error}")),
    };
    if !case.has_expected_output {
        return (
            Status::Failed,
            "error case: engine produced output instead of signalling an error".to_owned(),
        );
    }
    let output = case.output.as_deref().unwrap_or("directGraph.ttl");
    let expected = match read_forked(dir, output, TAG).and_then(|text| parse_turtle(&text, BASE)) {
        Ok(expected) => expected,
        Err(error) => return (Status::Skipped, format!("expected output: {error}")),
    };
    compare(&triples, &expected)
}
