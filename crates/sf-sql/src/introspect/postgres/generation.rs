//! PostgreSQL generation-snapshot primitives for ADR-0050.
//!
//! These functions deliberately return no authority token. They only let a
//! caller keep relation locks and catalogue observation on one connection and
//! one caller-owned transaction.

use std::fmt::Write as _;

use tokio_postgres::types::Type;
use tokio_postgres::GenericClient;

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
    require_generation_transaction(client).await?;
    collect_observed_snapshot(client).await
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
    if let Some(sql) = build_public_base_table_lock_sql(tables)? {
        client
            .batch_execute(&sql)
            .await
            .map_err(|_| redacted("PostgreSQL public relation lock failed"))?;
    }
    require_generation_transaction(client).await
}

async fn collect_observed_snapshot<C>(client: &C) -> Result<Postgres16PublicObservedSnapshotV1>
where
    C: GenericClient + Sync,
{
    observation::qualify_profile_guard(client)
        .await
        .map_err(closed_observation_error)?;
    let legacy_tables = collect_legacy_public_tables(client)
        .await
        .map_err(|_| redacted("PostgreSQL observed snapshot legacy collection failed"))?;
    let rich = observation::capture_registered_observation(client, RUNTIME_SCHEMA, &legacy_tables)
        .await
        .map_err(closed_observation_error)?;
    Ok(Postgres16PublicObservedSnapshotV1::available(
        legacy_tables,
        rich,
    ))
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
