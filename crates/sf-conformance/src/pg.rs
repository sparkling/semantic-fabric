//! Live-PostgreSQL conformance run (ADR-0005 / ADR-0010 / ADR-0015).
//!
//! The same W3C RDB2RDF cases the SQLite harness ([`crate::runner`]) drives, but
//! executed against a **real PostgreSQL server**: a throwaway database is created
//! for the run, each case's `create.sql` is loaded into a freshly recreated
//! `public` schema (so cases never see each other), the `CONSTRUCT { ?s ?p ?o }`
//! dump is translated with [`Dialect::Postgres`] and executed through the
//! bounded-memory server-side cursor ([`sf_sparql::exec_pg`], `query_raw`), and
//! the produced graph is adjudicated against the (optionally per-DBMS forked,
//! ADR-0015) gold by the same blank-node isomorphism.
//!
//! The canonical inventory is validated before any provider probe. Local runs
//! may explicitly return typed untested evidence when PostgreSQL is absent;
//! CI-required mode turns the same absence into an error.

use std::collections::HashSet;
use std::path::Path;

use sf_sparql::{exec_pg, parse_and_translate_with, Error as SparqlError, Tbox};
use sf_sql::introspect::introspect_postgres;
use sf_sql::{Dialect, TableSchema};
use tokio_postgres::{Client, NoTls};

use crate::graph::{has_named_graph, parse_nquads, parse_turtle};
use crate::manifest::{Case, Kind};
use crate::runner::{compare, compare_quads, input_error, parse_error_outcome, read, read_forked};
use crate::sealed_suite::{Backend, SealedSuite};
use crate::{CaseResult, Report, Status};

/// The W3C conformance query (the whole virtual graph as a triple dump).
const DUMP: &str = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";
/// Base IRI fixed by ADR-0005 for mapping parsing and Direct Mapping IRIs.
const BASE: &str = "http://example.com/base/";
/// The forked-fixture dialect tag (`create.postgres.sql`, `mappeda.postgres.nq`).
const TAG: &str = "postgres";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveMode {
    LocalOptional,
    CiRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UntestedReason {
    ProviderUnavailable { detail: String },
}

#[derive(Debug)]
pub enum SuiteRun {
    Tested(Report),
    Untested(UntestedReason),
}

/// Base connection params (host/port/user, **no** dbname): `SF_PG_URL` if set,
/// else a local trust-auth default keyed on `$USER`.
fn base_conn() -> String {
    std::env::var("SF_PG_URL").unwrap_or_else(|_| {
        let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_owned());
        format!("host=localhost port=5432 user={user}")
    })
}

/// Connect and spawn the driver task, returning the live client.
async fn connect(conn_str: &str) -> Result<Client, String> {
    let (client, connection) = tokio_postgres::connect(conn_str, NoTls)
        .await
        .map_err(|e| e.to_string())?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok(client)
}

/// Run the sealed suite against PostgreSQL. Inputs are validated before the
/// runtime or connection probe is created.
pub fn run(suite_root: &Path, mode: LiveMode) -> Result<SuiteRun, String> {
    let sealed = SealedSuite::load(suite_root)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async move {
        let base = base_conn();
        let admin = match connect(&format!("{base} dbname=postgres")).await {
            Ok(c) => c,
            Err(error) => return unavailable(mode, error),
        };
        let dbname = format!("sf_conformance_{}", std::process::id());
        admin
            .batch_execute(&format!("DROP DATABASE IF EXISTS {dbname} WITH (FORCE)"))
            .await
            .map_err(|e| e.to_string())?;
        admin
            .batch_execute(&format!("CREATE DATABASE {dbname}"))
            .await
            .map_err(|e| e.to_string())?;

        let work = connect(&format!("{base} dbname={dbname}")).await?;
        let report = run_cases(&sealed, &work).await;
        drop(work);
        // Best-effort teardown (FORCE terminates any lingering session).
        let _ = admin
            .batch_execute(&format!("DROP DATABASE IF EXISTS {dbname} WITH (FORCE)"))
            .await;
        report.map(SuiteRun::Tested)
    })
}

fn unavailable(mode: LiveMode, detail: String) -> Result<SuiteRun, String> {
    match mode {
        LiveMode::LocalOptional => Ok(SuiteRun::Untested(UntestedReason::ProviderUnavailable {
            detail,
        })),
        LiveMode::CiRequired => Err(format!(
            "required PostgreSQL provider is unavailable: {detail}"
        )),
    }
}

/// Adjudicate each case in canonical inventory order over `client`.
async fn run_cases(sealed: &SealedSuite, client: &Client) -> Result<Report, String> {
    let mut cases = Vec::new();
    for entry in sealed.cases() {
        let case = &entry.case;
        let (status, reason) = match case.kind {
            Kind::R2rml => run_r2rml_pg(&entry.directory, case, client).await,
            Kind::DirectMapping => run_direct_pg(&entry.directory, case, client).await,
        }?;
        cases.push(CaseResult {
            id: case.identifier.clone(),
            kind: case.kind,
            status,
            reason,
        });
    }
    let report = Report { cases };
    sealed.validate_report(Backend::Postgres, &report)?;
    Ok(report)
}

/// Recreate an empty `public` schema and load the (forked) `create.sql` into it.
/// A DDL PostgreSQL cannot accept (e.g. `VARBINARY`, `X'…'`) surfaces as an error
/// the caller turns into a documented skip.
async fn load_fixture(client: &Client, sql: &str) -> Result<(), String> {
    client
        .batch_execute(
            "DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public; SET search_path TO public;",
        )
        .await
        .map_err(|e| e.to_string())?;
    client
        .batch_execute(sql)
        .await
        .map_err(|e| format!("create.sql load failed: {e}"))
}

/// Introspect every base table in `public` (name order), for Direct Mapping and
/// for the ADR-0007 cascade over real catalog metadata.
async fn introspect_all(client: &Client) -> Result<Vec<TableSchema>, String> {
    let rows = client
        .query(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_type = 'BASE TABLE' ORDER BY table_name",
            &[],
        )
        .await
        .map_err(|e| e.to_string())?;
    let mut schemas = Vec::with_capacity(rows.len());
    for r in rows {
        let name: String = r.get(0);
        schemas.push(
            introspect_postgres(client, &name)
                .await
                .map_err(|e| e.to_string())?,
        );
    }
    Ok(schemas)
}

/// R2RML §5.1: an `rr:sqlQuery` view must not yield two identically-named result
/// columns — validate each against the live server via prepare-time metadata.
async fn validate_query_sources(
    client: &Client,
    maps: &[sf_core::ir::TriplesMap],
) -> Result<(), String> {
    use sf_core::ir::LogicalSource;
    for map in maps {
        if let LogicalSource::Query(q) = &map.source {
            if let Ok(stmt) = client.prepare(q).await {
                let mut seen = HashSet::new();
                for col in stmt.columns() {
                    if !seen.insert(col.name().to_owned()) {
                        return Err(format!(
                            "rr:sqlQuery produces duplicate column name {:?} (R2RML §5.1)",
                            col.name()
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

async fn run_r2rml_pg(
    dir: &Path,
    case: &Case,
    client: &Client,
) -> Result<(Status, String), String> {
    let sql =
        read_forked(dir, "create.sql", TAG).map_err(|e| input_error(case, "create.sql", &e))?;
    if let Err(e) = load_fixture(client, &sql).await {
        return Ok((Status::Skipped, format!("fixture: {e}")));
    }
    let doc = case
        .mapping_document
        .as_deref()
        .ok_or_else(|| input_error(case, "mappingDocument", "missing from sealed case"))?;
    let ttl = read(dir, doc).map_err(|e| input_error(case, doc, &e))?;
    let maps = match sf_mapping::parse_r2rml(&ttl) {
        Ok(m) => m,
        Err(e) => return Ok(parse_error_outcome(case, &format!("mapping parse: {e}"))),
    };
    if let Err(e) = validate_query_sources(client, &maps).await {
        return Ok(parse_error_outcome(case, &e));
    }
    let schemas = match introspect_all(client).await {
        Ok(s) => s,
        Err(e) => return Ok((Status::Skipped, format!("introspect: {e}"))),
    };
    let plan = match parse_and_translate_with(
        DUMP,
        &maps,
        Dialect::Postgres,
        &Tbox::default(),
        &schemas,
    ) {
        Ok(p) => p,
        Err(SparqlError::Unsupported(m)) => {
            return Ok((Status::Skipped, format!("501 translate: {m}")))
        }
        Err(e) => return Ok(parse_error_outcome(case, &format!("translate: {e}"))),
    };
    let triples = match exec_pg::construct_triples_pg(&plan, client).await {
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
    let expected_text = read_forked(dir, out, TAG).map_err(|e| input_error(case, out, &e))?;
    let expected = parse_nquads(&expected_text)
        .map_err(|e| input_error(case, out, &format!("invalid N-Quads: {e}")))?;
    if has_named_graph(&expected) {
        let quads = match exec_pg::dump_quads_pg(&maps, client, Dialect::Postgres).await {
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

async fn run_direct_pg(
    dir: &Path,
    case: &Case,
    client: &Client,
) -> Result<(Status, String), String> {
    let sql =
        read_forked(dir, "create.sql", TAG).map_err(|e| input_error(case, "create.sql", &e))?;
    if let Err(e) = load_fixture(client, &sql).await {
        return Ok((Status::Skipped, format!("fixture: {e}")));
    }
    let schemas = match introspect_all(client).await {
        Ok(s) => s,
        Err(e) => return Ok((Status::Skipped, format!("introspect: {e}"))),
    };
    let maps = match sf_mapping::direct_mapping(&schemas, BASE) {
        Ok(m) => m,
        Err(e) => return Ok((Status::Failed, format!("direct mapping: {e}"))),
    };
    let plan = match parse_and_translate_with(
        DUMP,
        &maps,
        Dialect::Postgres,
        &Tbox::default(),
        &schemas,
    ) {
        Ok(p) => p,
        Err(SparqlError::Unsupported(m)) => {
            return Ok((Status::Skipped, format!("501 translate: {m}")))
        }
        Err(e) => return Ok((Status::Failed, format!("translate: {e}"))),
    };
    let triples = match exec_pg::construct_triples_pg(&plan, client).await {
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
    let expected_text = read_forked(dir, out, TAG).map_err(|e| input_error(case, out, &e))?;
    let expected = parse_turtle(&expected_text, BASE)
        .map_err(|e| input_error(case, out, &format!("invalid Turtle: {e}")))?;
    Ok(compare(&triples, &expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_provider_absence_is_typed_untested() {
        let outcome = unavailable(LiveMode::LocalOptional, "connection refused".to_owned())
            .expect("local absence is allowed");
        assert!(matches!(
            outcome,
            SuiteRun::Untested(UntestedReason::ProviderUnavailable { detail })
                if detail == "connection refused"
        ));
    }

    #[test]
    fn ci_required_provider_absence_is_an_error() {
        let error = unavailable(LiveMode::CiRequired, "connection refused".to_owned())
            .expect_err("required provider absence must fail");
        assert!(error.contains("required PostgreSQL provider"), "{error}");
        assert!(error.contains("connection refused"), "{error}");
    }
}
