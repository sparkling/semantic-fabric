//! PostgreSQL generation-snapshot primitives for ADR-0050.
//!
//! These functions deliberately return no authority token. They only let a
//! caller keep relation locks and catalogue observation on one connection and
//! one caller-owned transaction.

use std::fmt::Write as _;

use tokio_postgres::types::Type;
use tokio_postgres::{error::SqlState, GenericClient};

use crate::dialect::Dialect;
use crate::error::{Error, Result};

use super::legacy_bounds::{validate_legacy_table_names, PRODUCTION_LEGACY_INPUT_LIMITS_V1};
use super::legacy_query::{
    query_bounded, TypedQueryParameter, LEGACY_RELATION_QUERY_LIMIT_PG16_V1,
    MAX_LEGACY_RELATIONS_PG16_V1,
};
use super::legacy_row::{LegacyRow, LEGACY_TEXT_QUERY_LIMIT_PG16_V1};
use super::{
    introspect_in_schema, observation, Postgres16PublicObservedSnapshotV1, RUNTIME_SCHEMA,
    TABLES_SQL,
};

const EXPLICIT_TRANSACTION_PROBE_SQL: &str =
    "SAVEPOINT sf_generation_transaction_probe; RELEASE SAVEPOINT sf_generation_transaction_probe;";
const TRANSACTION_MODE_SQL: &str =
    "SELECT pg_catalog.current_setting('transaction_isolation')::pg_catalog.text = 'repeatable read'::pg_catalog.text AS repeatable_read, pg_catalog.current_setting('transaction_read_only')::pg_catalog.text = 'on'::pg_catalog.text AS read_only";
const MAX_POSTGRES_IDENTIFIER_BYTES_V1: usize = 63;

/// Capture the guarded PostgreSQL-16 public observation in a caller-owned
/// `REPEATABLE READ READ ONLY` transaction.
///
/// This function starts and commits no transaction. It accepts either a
/// [`tokio_postgres::Transaction`] or a [`tokio_postgres::Client`] after the
/// caller has issued an explicit manual `BEGIN`. The transaction-mode guard
/// rejects autocommit, other isolation levels, and read-write transactions.
///
/// A verified-generation caller must acquire its complete, closed relation set
/// with [`lock_postgres_public_base_tables`] first. The lock call freezes the
/// repeatable-read snapshot only after the locks are held. The caller must set
/// transaction-local statement, lock, idle, search-path, and replication-role
/// policy before that lock; this borrowed collector never widens or replaces
/// those limits. It collects the legacy and rich projections and checks their
/// exact relation/column coordinates before returning the non-authorizing
/// snapshot.
pub async fn introspect_postgres_public_observed_snapshot_in_transaction<C>(
    client: &C,
) -> Result<Postgres16PublicObservedSnapshotV1>
where
    C: GenericClient + Sync,
{
    introspect_postgres_public_observed_snapshot_in_transaction_classified(client)
        .await
        .map_err(|error| match error {
            PostgresGenerationObservationFailure::SourceUnavailable => {
                redacted("PostgreSQL observed snapshot collection failed")
            }
            PostgresGenerationObservationFailure::ProfileUnavailable(reason) => {
                closed_observation_error(reason)
            }
        })
}

/// Identifier-free split between transient source failure and a coherently
/// observed profile rejection. Verified runtimes use this distinction to avoid
/// quarantining an activation merely because a query timed out or disconnected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PostgresGenerationObservationFailure {
    SourceUnavailable,
    ProfileUnavailable(super::PostgresSchemaIdentityUnavailableV1),
}

pub async fn introspect_postgres_public_observed_snapshot_in_transaction_classified<C>(
    client: &C,
) -> std::result::Result<Postgres16PublicObservedSnapshotV1, PostgresGenerationObservationFailure>
where
    C: GenericClient + Sync,
{
    require_generation_transaction(client)
        .await
        .map_err(|_| PostgresGenerationObservationFailure::SourceUnavailable)?;
    collect_observed_snapshot_classified(client).await
}

/// Acquire `ACCESS SHARE` locks for an exact bounded set of quoted `public`
/// relation names and retain them until the caller ends its transaction.
///
/// Names are validated before SQL execution, sorted to impose one global lock
/// order, and emitted as separately double-quoted schema and relation
/// identifiers. `ONLY` prevents implicit descendant expansion; `NOWAIT`
/// prevents an unbounded lock wait. This function neither starts nor ends the
/// transaction and returns no verified-generation authority. The subsequent
/// observed-snapshot capture must still prove that every name is an admitted
/// public base table and that its complete expected relation set is unchanged.
#[allow(dead_code)] // Exposed at the crate boundary by the serving integration slice.
pub async fn lock_postgres_public_base_tables<C>(client: &C, tables: &[String]) -> Result<()>
where
    C: GenericClient + Sync,
{
    lock_postgres_public_base_tables_classified(client, tables)
        .await
        .map_err(|_| redacted("PostgreSQL public relation lock failed"))
}

/// Redacted structured outcome for a generation-lock attempt. This preserves
/// only the distinction needed by runtime readiness; it never exposes source
/// names, SQL text, connection details, or backend messages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PostgresPublicTableLockFailure {
    InvalidRelationSet,
    RelationSetChanged,
    Unavailable,
}

/// Structured sibling used by verified-generation adapters to distinguish a
/// stale relation set from transient lock/transport unavailability.
pub async fn lock_postgres_public_base_tables_classified<C>(
    client: &C,
    tables: &[String],
) -> std::result::Result<(), PostgresPublicTableLockFailure>
where
    C: GenericClient + Sync,
{
    let sql = build_public_base_table_lock_sql(tables)
        .map_err(|_| PostgresPublicTableLockFailure::InvalidRelationSet)?;
    if let Some(sql) = sql {
        client
            .batch_execute(&sql)
            .await
            .map_err(classify_lock_error)?;
    }
    require_generation_transaction(client)
        .await
        .map_err(|_| PostgresPublicTableLockFailure::Unavailable)
}

fn classify_lock_error(error: tokio_postgres::Error) -> PostgresPublicTableLockFailure {
    classify_lock_sqlstate(error.code())
}

fn classify_lock_sqlstate(code: Option<&SqlState>) -> PostgresPublicTableLockFailure {
    match code {
        Some(code) if *code == SqlState::UNDEFINED_TABLE || *code == SqlState::UNDEFINED_SCHEMA => {
            PostgresPublicTableLockFailure::RelationSetChanged
        }
        _ => PostgresPublicTableLockFailure::Unavailable,
    }
}

async fn collect_observed_snapshot_classified<C>(
    client: &C,
) -> std::result::Result<Postgres16PublicObservedSnapshotV1, PostgresGenerationObservationFailure>
where
    C: GenericClient + Sync,
{
    observation::qualify_profile_guard(client)
        .await
        .map_err(classify_observation_failure)?;
    let legacy_tables = collect_legacy_public_tables(client)
        .await
        .map_err(|_| PostgresGenerationObservationFailure::SourceUnavailable)?;
    let rich = observation::capture_registered_observation(client, RUNTIME_SCHEMA, &legacy_tables)
        .await
        .map_err(classify_observation_failure)?;
    Ok(Postgres16PublicObservedSnapshotV1::available(
        legacy_tables,
        rich,
    ))
}

fn classify_observation_failure(
    reason: super::PostgresSchemaIdentityUnavailableV1,
) -> PostgresGenerationObservationFailure {
    if reason == super::PostgresSchemaIdentityUnavailableV1::CatalogQuery {
        PostgresGenerationObservationFailure::SourceUnavailable
    } else {
        PostgresGenerationObservationFailure::ProfileUnavailable(reason)
    }
}

async fn collect_legacy_public_tables<C>(client: &C) -> Result<Vec<crate::schema::TableSchema>>
where
    C: GenericClient + Sync,
{
    let rows = query_bounded(
        client,
        TABLES_SQL,
        &[
            TypedQueryParameter::new(&RUNTIME_SCHEMA, Type::TEXT),
            TypedQueryParameter::new(&LEGACY_TEXT_QUERY_LIMIT_PG16_V1, Type::INT4),
            TypedQueryParameter::new(&LEGACY_RELATION_QUERY_LIMIT_PG16_V1, Type::INT8),
        ],
        MAX_LEGACY_RELATIONS_PG16_V1,
        "table rows",
    )
    .await?;
    let tables: Vec<String> = rows
        .into_iter()
        .map(|row| LegacyRow::try_new(&row)?.text("bounded_text_0", "table name"))
        .collect::<Result<_>>()?;
    introspect_in_schema(client, RUNTIME_SCHEMA, &tables).await
}

async fn require_generation_transaction<C>(client: &C) -> Result<()>
where
    C: GenericClient + Sync,
{
    client
        .batch_execute(EXPLICIT_TRANSACTION_PROBE_SQL)
        .await
        .map_err(|_| redacted("PostgreSQL generation transaction required"))?;
    let row = client
        .query_one(TRANSACTION_MODE_SQL, &[])
        .await
        .map_err(|_| redacted("PostgreSQL generation transaction check failed"))?;
    let repeatable_read = row
        .try_get::<_, bool>("repeatable_read")
        .map_err(|_| redacted("PostgreSQL generation transaction check failed"))?;
    let read_only = row
        .try_get::<_, bool>("read_only")
        .map_err(|_| redacted("PostgreSQL generation transaction check failed"))?;
    if repeatable_read && read_only {
        Ok(())
    } else {
        Err(redacted(
            "PostgreSQL generation transaction must be repeatable-read and read-only",
        ))
    }
}

fn build_public_base_table_lock_sql(tables: &[String]) -> Result<Option<String>> {
    validate_legacy_table_names(
        tables.iter().map(String::as_str),
        PRODUCTION_LEGACY_INPUT_LIMITS_V1,
    )?;
    let mut names: Vec<&str> = tables.iter().map(String::as_str).collect();
    for name in &names {
        if name.is_empty()
            || name.len() > MAX_POSTGRES_IDENTIFIER_BYTES_V1
            || name.as_bytes().contains(&0)
        {
            return Err(redacted("PostgreSQL public relation name is invalid"));
        }
    }
    names.sort_unstable();
    if names.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(redacted(
            "PostgreSQL public relation name set contains duplicates",
        ));
    }
    if names.is_empty() {
        return Ok(None);
    }

    let schema = Dialect::Postgres.quote_ident(RUNTIME_SCHEMA);
    let mut sql = String::from("LOCK TABLE ");
    for (index, name) in names.into_iter().enumerate() {
        if index != 0 {
            sql.push_str(", ");
        }
        write!(sql, "ONLY {schema}.{}", Dialect::Postgres.quote_ident(name))
            .map_err(|_| redacted("PostgreSQL public relation lock construction failed"))?;
    }
    sql.push_str(" IN ACCESS SHARE MODE NOWAIT");
    Ok(Some(sql))
}

fn closed_observation_error(error: super::PostgresSchemaIdentityUnavailableV1) -> Error {
    Error::Introspection(error.to_string())
}

fn redacted(message: &'static str) -> Error {
    Error::Introspection(message.into())
}

#[cfg(test)]
mod tests;
