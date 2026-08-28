//! Live-PostgreSQL conformance run (ADR-0005 / ADR-0010 / ADR-0015).
//!
//! The same W3C RDB2RDF cases the SQLite harness ([`crate::runner`]) drives, but
//! executed against a **real PostgreSQL server**: a throwaway database is created
//! for the run, each case's `create.sql` is loaded into a freshly recreated
//! `public` schema (so cases never see each other), the `CONSTRUCT { ?s ?p ?o }`
//! dump is translated with [`Dialect::Postgres`] and executed through the
//! bounded-memory server-side cursor ([`sf_sparql::exec_pg`], `query_raw`), and
//! the produced graph is adjudicated against the inventory-captured gold by the
//! same blank-node isomorphism.
//!
//! The canonical inventory is validated before any provider probe. Local runs
//! may explicitly return typed untested evidence when PostgreSQL is absent;
//! CI-required mode turns the same absence into an error.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sf_sparql::{exec_pg, parse_and_translate_with, Error as SparqlError, Tbox};
use sf_sql::introspect::introspect_postgres;
use sf_sql::{Dialect, TableSchema};
use tokio_postgres::{Client, Config, NoTls};

use crate::graph::{has_named_graph, parse_nquads, parse_turtle};
use crate::manifest::Kind;
use crate::runner::{compare, compare_quads, input_error};
use crate::sealed_suite::{
    Backend, ClassifiedCaseResult, ClassifiedReport, OutcomeCode, SealedCase, SealedSuite,
};
use crate::{Report, Status};

mod outcome;
use outcome::{classified_error, classify_comparison, outcome, plain_report, CaseOutcome};
#[cfg(test)]
mod tests;

/// The W3C conformance query (the whole virtual graph as a triple dump).
const DUMP: &str = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";
/// Base IRI fixed by ADR-0005 for mapping parsing and Direct Mapping IRIs.
const BASE: &str = "http://example.com/base/";
static NEXT_DATABASE: AtomicU64 = AtomicU64::new(0);

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
pub enum LiveRun<T> {
    Tested(T),
    Untested(UntestedReason),
}

pub type SuiteRun = LiveRun<Report>;
pub type ClassifiedSuiteRun = LiveRun<ClassifiedReport>;

/// Base connection params (host/port/user, **no** dbname): `SF_PG_URL` if set,
/// else a local trust-auth default keyed on `$USER`.
fn base_config() -> Result<Config, String> {
    let value = std::env::var("SF_PG_URL").unwrap_or_else(|_| {
        let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_owned());
        format!("host=localhost port=5432 user={user}")
    });
    parse_base_config(&value)
}

fn parse_base_config(value: &str) -> Result<Config, String> {
    value
        .parse()
        .map_err(|_| "invalid PostgreSQL connection configuration".to_owned())
}

fn connection_error<T>(_error: T) -> String {
    "PostgreSQL connection failed".to_owned()
}

/// Connect and spawn the driver task, returning the live client.
async fn connect(base: &Config, database: &str) -> Result<Client, String> {
    let mut config = base.clone();
    config.dbname(database);
    let (client, connection) = config.connect(NoTls).await.map_err(connection_error)?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok(client)
}

/// Run the sealed suite against PostgreSQL. Inputs are validated before the
/// runtime or connection probe is created.
pub fn run(suite_root: &Path, mode: LiveMode) -> Result<SuiteRun, String> {
    let sealed = SealedSuite::load(suite_root)?;
    match run_sealed_suite(&sealed, mode)? {
        LiveRun::Tested(report) => Ok(LiveRun::Tested(plain_report(report))),
        LiveRun::Untested(reason) => Ok(LiveRun::Untested(reason)),
    }
}

/// Execute an already captured suite. Receipt callers use `CiRequired`, making
/// provider absence fatal without consulting ambient process state.
pub fn run_sealed_suite(
    sealed: &SealedSuite,
    mode: LiveMode,
) -> Result<ClassifiedSuiteRun, String> {
    let base = base_config()?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let admin = match connect(&base, "postgres").await {
            Ok(c) => c,
            Err(error) => return unavailable(mode, error),
        };
        let dbname = scratch_database_name()?;
        admin
            .batch_execute(&format!("CREATE DATABASE {dbname}"))
            .await
            .map_err(|e| e.to_string())?;

        let report = match connect(&base, &dbname).await {
            Ok(work) => {
                let report = run_cases(sealed, &work).await;
                drop(work);
                report
            }
            Err(error) => Err(error),
        };
        let cleanup = admin
            .batch_execute(&format!("DROP DATABASE IF EXISTS {dbname} WITH (FORCE)"))
            .await
            .map_err(|error| format!("drop PostgreSQL scratch database: {error}"));
        match (report, cleanup) {
            (Ok(report), Ok(())) => Ok(LiveRun::Tested(report)),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    })
}

/// Required-live receipt entry point. Untested evidence can never cross this
/// boundary, irrespective of the ambient `CI` environment variable.
pub fn run_sealed_suite_required(sealed: &SealedSuite) -> Result<ClassifiedReport, String> {
    match run_sealed_suite(sealed, LiveMode::CiRequired)? {
        LiveRun::Tested(report) => Ok(report),
        LiveRun::Untested(_) => {
            Err("required PostgreSQL run returned untested evidence".to_owned())
        }
    }
}

fn scratch_database_name() -> Result<String, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before the Unix epoch".to_owned())?
        .as_nanos();
    let serial = NEXT_DATABASE.fetch_add(1, Ordering::Relaxed);
    Ok(format!(
        "sf_conformance_{:x}_{nanos:x}_{serial:x}",
        std::process::id(),
    ))
}

fn unavailable<T>(mode: LiveMode, detail: String) -> Result<LiveRun<T>, String> {
    match mode {
        LiveMode::LocalOptional => Ok(LiveRun::Untested(UntestedReason::ProviderUnavailable {
            detail,
        })),
        LiveMode::CiRequired => Err(format!(
            "required PostgreSQL provider is unavailable: {detail}"
        )),
    }
}

/// Adjudicate each case in canonical inventory order over `client`.
async fn run_cases(sealed: &SealedSuite, client: &Client) -> Result<ClassifiedReport, String> {
    let mut cases = Vec::new();
    for entry in sealed.cases() {
        let case = &entry.case;
        let result = match case.kind {
            Kind::R2rml => run_r2rml_pg(entry, client).await,
            Kind::DirectMapping => run_direct_pg(entry, client).await,
        }?;
        cases.push(ClassifiedCaseResult {
            id: case.identifier.clone(),
            kind: case.kind,
            status: result.status,
            outcome_code: result.code,
            reason: result.reason,
        });
    }
    let report = ClassifiedReport { cases };
    sealed.validate_classified_report(Backend::Postgres, &report)?;
    Ok(report)
}

/// Recreate an empty `public` schema and load the sealed `create.sql` into it.
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

async fn run_r2rml_pg(entry: &SealedCase, client: &Client) -> Result<CaseOutcome, String> {
    let case = &entry.case;
    if let Err(error) = load_fixture(client, entry.create_sql()).await {
        return Ok(outcome(
            Status::Skipped,
            OutcomeCode::FixtureLoadError,
            format!("fixture: {error}"),
        ));
    }
    let doc = case
        .mapping_document
        .as_deref()
        .ok_or_else(|| input_error(case, "mappingDocument", "missing from sealed case"))?;
    let ttl = entry
        .mapping_text()
        .ok_or_else(|| input_error(case, doc, "missing from sealed snapshot"))?;
    let maps = match sf_mapping::parse_r2rml(ttl) {
        Ok(m) => m,
        Err(error) => {
            return Ok(classified_error(
                case,
                OutcomeCode::MappingError,
                format!("mapping parse: {error}"),
            ))
        }
    };
    if let Err(error) = validate_query_sources(client, &maps).await {
        return Ok(classified_error(
            case,
            OutcomeCode::SourceValidationError,
            error,
        ));
    }
    let schemas = match introspect_all(client).await {
        Ok(s) => s,
        Err(error) => {
            return Ok(outcome(
                Status::Skipped,
                OutcomeCode::IntrospectionError,
                format!("introspect: {error}"),
            ))
        }
    };
    let plan = match parse_and_translate_with(
        DUMP,
        &maps,
        Dialect::Postgres,
        &Tbox::default(),
        &schemas,
    ) {
        Ok(p) => p,
        Err(SparqlError::Unsupported(message)) => {
            return Ok(outcome(
                Status::Skipped,
                OutcomeCode::TranslationUnsupported,
                format!("501 translate: {message}"),
            ))
        }
        Err(error) => {
            return Ok(classified_error(
                case,
                OutcomeCode::TranslationError,
                format!("translate: {error}"),
            ))
        }
    };
    let triples = match exec_pg::construct_triples_pg(&plan, client).await {
        Ok(t) => t,
        Err(SparqlError::Unsupported(message)) => {
            return Ok(outcome(
                Status::Skipped,
                OutcomeCode::ExecutionUnsupported,
                format!("501 exec: {message}"),
            ))
        }
        Err(error) => {
            return Ok(classified_error(
                case,
                OutcomeCode::ExecutionError,
                format!("exec: {error}"),
            ))
        }
    };
    if !case.has_expected_output {
        return Ok(outcome(
            Status::Failed,
            OutcomeCode::UnexpectedOutput,
            "error case: engine produced output instead of signalling an error".to_owned(),
        ));
    }
    let out = case
        .output
        .as_deref()
        .ok_or_else(|| input_error(case, "output", "missing from sealed positive case"))?;
    let expected_text = entry
        .output_text()
        .ok_or_else(|| input_error(case, out, "missing from sealed snapshot"))?;
    let expected = parse_nquads(expected_text)
        .map_err(|error| input_error(case, out, &format!("invalid N-Quads: {error}")))?;
    if has_named_graph(&expected) {
        let quads = match exec_pg::dump_quads_pg(&maps, client, Dialect::Postgres).await {
            Ok(q) => q,
            Err(SparqlError::Unsupported(message)) => {
                return Ok(outcome(
                    Status::Skipped,
                    OutcomeCode::QuadDumpUnsupported,
                    format!("501 quad dump: {message}"),
                ))
            }
            Err(error) => {
                return Ok(classified_error(
                    case,
                    OutcomeCode::QuadDumpError,
                    format!("quad dump: {error}"),
                ))
            }
        };
        return Ok(classify_comparison(
            compare_quads(&quads, &expected),
            OutcomeCode::DatasetMatched,
            OutcomeCode::DatasetMismatch,
        ));
    }
    Ok(classify_comparison(
        compare(&triples, &expected),
        OutcomeCode::GraphMatched,
        OutcomeCode::GraphMismatch,
    ))
}

async fn run_direct_pg(entry: &SealedCase, client: &Client) -> Result<CaseOutcome, String> {
    let case = &entry.case;
    if let Err(error) = load_fixture(client, entry.create_sql()).await {
        return Ok(outcome(
            Status::Skipped,
            OutcomeCode::FixtureLoadError,
            format!("fixture: {error}"),
        ));
    }
    let schemas = match introspect_all(client).await {
        Ok(s) => s,
        Err(error) => {
            return Ok(outcome(
                Status::Skipped,
                OutcomeCode::IntrospectionError,
                format!("introspect: {error}"),
            ))
        }
    };
    let maps = match sf_mapping::direct_mapping(&schemas, BASE) {
        Ok(m) => m,
        Err(error) => {
            return Ok(outcome(
                Status::Failed,
                OutcomeCode::DirectMappingError,
                format!("direct mapping: {error}"),
            ))
        }
    };
    let plan = match parse_and_translate_with(
        DUMP,
        &maps,
        Dialect::Postgres,
        &Tbox::default(),
        &schemas,
    ) {
        Ok(p) => p,
        Err(SparqlError::Unsupported(message)) => {
            return Ok(outcome(
                Status::Skipped,
                OutcomeCode::TranslationUnsupported,
                format!("501 translate: {message}"),
            ))
        }
        Err(error) => {
            return Ok(outcome(
                Status::Failed,
                OutcomeCode::TranslationError,
                format!("translate: {error}"),
            ))
        }
    };
    let triples = match exec_pg::construct_triples_pg(&plan, client).await {
        Ok(t) => t,
        Err(SparqlError::Unsupported(message)) => {
            return Ok(outcome(
                Status::Skipped,
                OutcomeCode::ExecutionUnsupported,
                format!("501 exec: {message}"),
            ))
        }
        Err(error) => {
            return Ok(outcome(
                Status::Failed,
                OutcomeCode::ExecutionError,
                format!("exec: {error}"),
            ))
        }
    };
    if !case.has_expected_output {
        return Ok(outcome(
            Status::Failed,
            OutcomeCode::UnexpectedOutput,
            "error case: engine produced output instead of signalling an error".to_owned(),
        ));
    }
    let out = case
        .output
        .as_deref()
        .ok_or_else(|| input_error(case, "output", "missing from sealed positive case"))?;
    let expected_text = entry
        .output_text()
        .ok_or_else(|| input_error(case, out, "missing from sealed snapshot"))?;
    let expected = parse_turtle(expected_text, BASE)
        .map_err(|error| input_error(case, out, &format!("invalid Turtle: {error}")))?;
    Ok(classify_comparison(
        compare(&triples, &expected),
        OutcomeCode::GraphMatched,
        OutcomeCode::GraphMismatch,
    ))
}
