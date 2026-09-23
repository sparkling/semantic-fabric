//! Ordinary authored startup attempts the qualified protected generation for
//! each backend and falls back to the unverified path when a builder declines.
//! G3 decisions (2026-09-23): file-backed WAL/DELETE SQLite; PostgreSQL and
//! MySQL only where admission permits verified generations, so native RLS
//! bearers keep the unverified path where the binding applies RLS.

use sf_core::query_control::QueryControl;
use sf_core::SourceId;

use super::{semantic_admission_error, ServeOptions};
use crate::budget::RequestBudget;
use crate::semantic_admission::ValidatedMapping;
use crate::source::PreparedSource;
use crate::{IntrospectedSource, RuntimeSource, SemanticOntology, ServeError};

type Built = Result<(IntrospectedSource, ValidatedMapping), ServeError>;

/// `Ok(None)` means the protected profile does not apply or declined: the
/// caller keeps the unchanged unverified path.
pub(super) async fn try_verified(
    opts: &ServeOptions,
    source: &PreparedSource,
    authored: &sf_core::SourceMapping,
    ontology: &SemanticOntology,
    generation_budget: Option<&RequestBudget>,
    observe: &mut impl FnMut(SourceId, &IntrospectedSource) -> Result<(), ServeError>,
) -> Result<Option<RuntimeSource>, ServeError> {
    let eligible = match source {
        PreparedSource::Sqlite { path, .. } => sqlite_eligible(path, authored),
        PreparedSource::Postgres { .. } | PreparedSource::Mysql { .. } => {
            opts.query_admission.permits_verified_generation()
                && crate::pg_rls::mapped_tables(authored).is_some()
        }
    };
    if !eligible {
        return Ok(None);
    }
    let budget = match generation_budget {
        Some(budget) => budget.clone(),
        None => crate::startup_authored::control_budget(None, source)?,
    };
    let attempt: Built = match source {
        PreparedSource::Sqlite { path, .. } => {
            crate::sqlite_generation::build(
                path.clone(),
                opts.sqlite_pool_size,
                authored.clone(),
                ontology,
                &budget,
                observe,
            )
            .await
        }
        PreparedSource::Postgres { config, tls, .. } => {
            let Ok(pools) = crate::pg_direct_lifecycle::PgDirectPools::with_tls(
                (**config).clone(),
                opts.pg_pool_size,
                opts.pg_pool_wait,
                (**tls).clone(),
            ) else {
                return Ok(None);
            };
            crate::pg_generation::authored::build(
                &pools,
                authored.clone(),
                ontology,
                &budget,
                observe,
            )
            .await
        }
        PreparedSource::Mysql { options, .. } => {
            crate::mysql_generation::build(
                options.clone(),
                authored.clone(),
                ontology,
                &budget,
                observe,
            )
            .await
        }
    };
    match attempt {
        Ok((source, mapping)) => RuntimeSource::admitted(source, mapping)
            .map(Some)
            .map_err(semantic_admission_error),
        // The request control itself stopped: surface it, never mask it.
        Err(error) if QueryControl::checkpoint(&budget).is_err() => Err(error),
        // The protected profile declined this source: keep the old path.
        Err(_) => Ok(None),
    }
}

/// A cheap pre-filter for the sealed SQLite generation: file-backed databases
/// in WAL or DELETE journal mode with bounded unqualified base-table mappings.
/// In-memory, URI and other journal modes never attempt it. Passing this is
/// not admission: `open_ordinary` still falls back if the builder declines.
fn sqlite_eligible(path: &str, mapping: &sf_core::SourceMapping) -> bool {
    if path == ":memory:" || path.starts_with("file:") {
        return false;
    }
    if crate::pg_rls::mapped_tables(mapping).is_none() {
        return false;
    }
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let Ok(conn) = rusqlite::Connection::open_with_flags(path, flags) else {
        return false;
    };
    conn.query_row("PRAGMA main.journal_mode", [], |row| {
        row.get::<_, String>(0)
    })
    .is_ok_and(|mode| matches!(mode.to_ascii_lowercase().as_str(), "wal" | "delete"))
}
