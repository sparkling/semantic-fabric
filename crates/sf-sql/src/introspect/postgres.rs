//! Namespace-safe PostgreSQL catalogue introspection.

use std::collections::{BTreeMap, HashMap};

use tokio_postgres::types::Type;

use crate::error::{Error, Result};
use crate::schema::{Column, ForeignKey, TableSchema};

mod legacy_bounds;
mod legacy_query;
mod legacy_row;
mod legacy_sql;
mod observation;
use legacy_bounds::{validate_legacy_table_names, PRODUCTION_LEGACY_INPUT_LIMITS_V1};
use legacy_query::{
    query_bounded, TypedQueryParameter, LEGACY_RELATION_QUERY_LIMIT_PG16_V1,
    LEGACY_SET_QUERY_LIMIT_PG16_V1, MAX_LEGACY_RELATIONS_PG16_V1, MAX_LEGACY_ROWS_PER_SET_PG16_V1,
};
use legacy_row::{LegacyRow, LEGACY_TEXT_QUERY_LIMIT_PG16_V1};
use legacy_sql::{
    COLUMNS_SQL, EARLIER_RELATION_COLLISIONS_SQL, FOREIGN_KEYS_SQL, KEYS_SQL, NDISTINCT_SQL,
    RELTUPLES_SQL, TABLES_SQL,
};
pub use observation::{
    Postgres16PublicObservedSchemaV1, Postgres16PublicObservedSnapshotV1,
    PostgresSchemaIdentityAvailabilityV1, PostgresSchemaIdentityGuardCodeV1,
    PostgresSchemaIdentityLimitCodeV1, PostgresSchemaIdentityUnavailableV1,
    POSTGRES16_PUBLIC_CONSTRAINT_PROFILE_ID_V1, POSTGRES16_PUBLIC_STRUCTURAL_PROFILE_ID_V1,
    POSTGRES16_PUBLIC_TYPE_PROFILE_ID_V1,
};

const RUNTIME_SCHEMA: &str = "public";
const SNAPSHOT_TIMEOUTS_SQL: &str =
    "SET LOCAL statement_timeout = '5s'; SET LOCAL lock_timeout = '1s';";

/// Introspect one table from the runtime-supported PostgreSQL `public` schema.
pub async fn introspect_postgres(
    client: &tokio_postgres::Client,
    table: &str,
) -> Result<TableSchema> {
    validate_legacy_table_names(std::iter::once(table), PRODUCTION_LEGACY_INPUT_LIMITS_V1)?;
    let tables = vec![table.to_owned()];
    introspect_in_schema(client, RUNTIME_SCHEMA, &tables)
        .await?
        .pop()
        .ok_or_else(|| Error::Introspection("PostgreSQL introspection returned no table".into()))
}

/// Introspect named tables from the runtime-supported PostgreSQL `public`
/// schema in six set-based catalogue round trips.
pub async fn introspect_postgres_all(
    client: &tokio_postgres::Client,
    tables: &[String],
) -> Result<Vec<TableSchema>> {
    introspect_in_schema(client, RUNTIME_SCHEMA, tables).await
}

/// Capture every PostgreSQL `public` base table from one coherent catalogue
/// snapshot.
///
/// Enumeration plus six set-based metadata queries execute in a single
/// `REPEATABLE READ READ ONLY` transaction. This prevents concurrent DDL from
/// mixing catalogue states. Arbitrary schema-qualified identities remain
/// unsupported by the runtime data model.
pub async fn introspect_postgres_public_snapshot(
    client: &mut tokio_postgres::Client,
) -> Result<Vec<TableSchema>> {
    let transaction = client
        .build_transaction()
        .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await?;
    transaction.batch_execute(SNAPSHOT_TIMEOUTS_SQL).await?;
    let rows = query_bounded(
        &transaction,
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
    let schemas = introspect_in_schema(&transaction, RUNTIME_SCHEMA, &tables).await?;
    transaction.commit().await?;
    Ok(schemas)
}

/// Capture the legacy public snapshot only after the frozen PostgreSQL-16
/// profile guards pass. Rich identity availability is not yet returned by
/// this compatibility-shaped API; callers needing it must use the future
/// observed-snapshot entry point.
pub async fn introspect_postgres_public_snapshot_guarded(
    client: &mut tokio_postgres::Client,
) -> Result<Vec<TableSchema>> {
    let transaction = client
        .build_transaction()
        .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await?;
    transaction.batch_execute(SNAPSHOT_TIMEOUTS_SQL).await?;
    observation::qualify_profile_guard(&transaction)
        .await
        .map_err(|error| Error::Introspection(error.to_string()))?;
    let rows = query_bounded(
        &transaction,
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
    let schemas = introspect_in_schema(&transaction, RUNTIME_SCHEMA, &tables).await?;
    transaction.commit().await?;
    Ok(schemas)
}

/// Capture a guarded snapshot with explicit identity availability. Until the
/// rich catalogue normalizer is wired, the legacy projection is preserved and
/// availability is reported as `ProfileNotImplemented` rather than inferred.
pub async fn introspect_postgres_public_observed_snapshot(
    client: &mut tokio_postgres::Client,
) -> Result<Postgres16PublicObservedSnapshotV1> {
    let legacy_tables = introspect_postgres_public_snapshot_guarded(client).await?;
    Ok(
        observation::Postgres16PublicObservedSnapshotV1::unavailable(
            legacy_tables,
            PostgresSchemaIdentityUnavailableV1::ProfileNotImplemented,
        ),
    )
}

async fn introspect_in_schema<C>(
    client: &C,
    schema_name: &str,
    tables: &[String],
) -> Result<Vec<TableSchema>>
where
    C: tokio_postgres::GenericClient + Sync,
{
    validate_legacy_table_names(
        tables.iter().map(String::as_str),
        PRODUCTION_LEGACY_INPUT_LIMITS_V1,
    )?;
    if tables.is_empty() {
        return Ok(Vec::new());
    }
    let mut schemas: BTreeMap<String, TableSchema> = tables
        .iter()
        .map(|table| (table.clone(), TableSchema::new(table)))
        .collect();
    if schemas.len() != tables.len() {
        return Err(Error::Introspection(
            "PostgreSQL introspection table list contains duplicates".into(),
        ));
    }

    reject_earlier_relation_collisions(client, tables).await?;
    load_columns(client, schema_name, tables, &mut schemas).await?;
    load_keys(client, schema_name, tables, &mut schemas).await?;
    load_foreign_keys(client, schema_name, tables, &mut schemas).await?;
    load_statistics(client, schema_name, tables, &mut schemas).await?;

    tables
        .iter()
        .map(|table| {
            schemas.remove(table).ok_or_else(|| {
                Error::Introspection("PostgreSQL introspection lost a requested table".into())
            })
        })
        .collect()
}

async fn reject_earlier_relation_collisions<C>(client: &C, tables: &[String]) -> Result<()>
where
    C: tokio_postgres::GenericClient + Sync,
{
    let earlier_schema = "pg_catalog";
    let collisions: Vec<String> = query_bounded(
        client,
        EARLIER_RELATION_COLLISIONS_SQL,
        &[
            TypedQueryParameter::new(&tables, Type::TEXT_ARRAY),
            TypedQueryParameter::new(&earlier_schema, Type::TEXT),
            TypedQueryParameter::new(&LEGACY_TEXT_QUERY_LIMIT_PG16_V1, Type::INT4),
            TypedQueryParameter::new(&LEGACY_RELATION_QUERY_LIMIT_PG16_V1, Type::INT8),
        ],
        MAX_LEGACY_RELATIONS_PG16_V1,
        "catalogue-collision rows",
    )
    .await?
    .into_iter()
    .map(|row| LegacyRow::try_new(&row)?.text("bounded_text_0", "collision name"))
    .collect::<Result<_>>()?;
    if collisions.is_empty() {
        return Ok(());
    }
    Err(Error::Introspection(format!(
        "PostgreSQL public relation name(s) collide with the earlier pg_catalog \
         execution scope: {}; qualified relation identity is not yet supported",
        collisions.join(", ")
    )))
}

async fn load_columns<C>(
    client: &C,
    schema_name: &str,
    tables: &[String],
    schemas: &mut BTreeMap<String, TableSchema>,
) -> Result<()>
where
    C: tokio_postgres::GenericClient + Sync,
{
    for row in query_bounded(
        client,
        COLUMNS_SQL,
        &[
            TypedQueryParameter::new(&tables, Type::TEXT_ARRAY),
            TypedQueryParameter::new(&schema_name, Type::TEXT),
            TypedQueryParameter::new(&LEGACY_TEXT_QUERY_LIMIT_PG16_V1, Type::INT4),
            TypedQueryParameter::new(&LEGACY_SET_QUERY_LIMIT_PG16_V1, Type::INT8),
        ],
        MAX_LEGACY_ROWS_PER_SET_PG16_V1,
        "column rows",
    )
    .await?
    {
        let row = LegacyRow::try_new(&row)?;
        let table = row.text("bounded_text_0", "column table name")?;
        let name = row.text("bounded_text_1", "column name")?;
        let data_type = row.text("bounded_text_2", "column data type")?;
        let is_nullable = row.text("bounded_text_3", "column nullability")?;
        if let Some(schema) = schemas.get_mut(&table) {
            schema.columns.push(Column::new(
                name,
                data_type,
                is_nullable.eq_ignore_ascii_case("NO"),
            ));
        }
    }
    for (table, schema) in schemas {
        if schema.columns.is_empty() {
            return Err(Error::Introspection(format!(
                "PostgreSQL table {table:?} not found in schema {schema_name:?}"
            )));
        }
    }
    Ok(())
}

async fn load_keys<C>(
    client: &C,
    schema_name: &str,
    tables: &[String],
    schemas: &mut BTreeMap<String, TableSchema>,
) -> Result<()>
where
    C: tokio_postgres::GenericClient + Sync,
{
    let mut primary: HashMap<String, Vec<String>> = HashMap::new();
    let mut unique: HashMap<String, BTreeMap<String, Vec<String>>> = HashMap::new();
    for row in query_bounded(
        client,
        KEYS_SQL,
        &[
            TypedQueryParameter::new(&tables, Type::TEXT_ARRAY),
            TypedQueryParameter::new(&schema_name, Type::TEXT),
            TypedQueryParameter::new(&LEGACY_TEXT_QUERY_LIMIT_PG16_V1, Type::INT4),
            TypedQueryParameter::new(&LEGACY_SET_QUERY_LIMIT_PG16_V1, Type::INT8),
        ],
        MAX_LEGACY_ROWS_PER_SET_PG16_V1,
        "key rows",
    )
    .await?
    {
        let row = LegacyRow::try_new(&row)?;
        let table = row.text("bounded_text_0", "key table name")?;
        let constraint_type = row.text("bounded_text_1", "key constraint type")?;
        let constraint = row.text("bounded_text_2", "key constraint name")?;
        let column = row.text("bounded_text_3", "key column name")?;
        if constraint_type == "PRIMARY KEY" {
            primary.entry(table).or_default().push(column);
        } else {
            unique
                .entry(table)
                .or_default()
                .entry(constraint)
                .or_default()
                .push(column);
        }
    }
    for (table, schema) in schemas {
        schema.primary_key = primary.remove(table).unwrap_or_default();
        schema.unique = unique
            .remove(table)
            .map(|constraints| constraints.into_values().collect())
            .unwrap_or_default();
    }
    Ok(())
}

async fn load_foreign_keys<C>(
    client: &C,
    schema_name: &str,
    tables: &[String],
    schemas: &mut BTreeMap<String, TableSchema>,
) -> Result<()>
where
    C: tokio_postgres::GenericClient + Sync,
{
    let mut foreign: HashMap<String, BTreeMap<String, ForeignKey>> = HashMap::new();
    for row in query_bounded(
        client,
        FOREIGN_KEYS_SQL,
        &[
            TypedQueryParameter::new(&tables, Type::TEXT_ARRAY),
            TypedQueryParameter::new(&schema_name, Type::TEXT),
            TypedQueryParameter::new(&LEGACY_TEXT_QUERY_LIMIT_PG16_V1, Type::INT4),
            TypedQueryParameter::new(&LEGACY_SET_QUERY_LIMIT_PG16_V1, Type::INT8),
        ],
        MAX_LEGACY_ROWS_PER_SET_PG16_V1,
        "foreign-key rows",
    )
    .await?
    {
        let row = LegacyRow::try_new(&row)?;
        let table = row.text("bounded_text_0", "foreign-key child table")?;
        let constraint = row.text("bounded_text_1", "foreign-key constraint")?;
        let column = row.text("bounded_text_2", "foreign-key child column")?;
        let parent_table = row.text("bounded_text_3", "foreign-key parent table")?;
        let parent_schema = row.text("bounded_text_4", "foreign-key parent schema")?;
        let parent_column = row.text("bounded_text_5", "foreign-key parent column")?;
        require_same_schema(schema_name, &parent_schema, &table, &constraint)?;
        let key = foreign
            .entry(table)
            .or_default()
            .entry(constraint)
            .or_insert_with(|| ForeignKey {
                columns: Vec::new(),
                parent_table,
                parent_columns: Vec::new(),
            });
        key.columns.push(column);
        key.parent_columns.push(parent_column);
    }
    for (table, schema) in schemas {
        schema.foreign_keys = foreign
            .remove(table)
            .map(|constraints| constraints.into_values().collect())
            .unwrap_or_default();
    }
    Ok(())
}

async fn load_statistics<C>(
    client: &C,
    schema_name: &str,
    tables: &[String],
    schemas: &mut BTreeMap<String, TableSchema>,
) -> Result<()>
where
    C: tokio_postgres::GenericClient + Sync,
{
    for row in query_bounded(
        client,
        RELTUPLES_SQL,
        &[
            TypedQueryParameter::new(&tables, Type::TEXT_ARRAY),
            TypedQueryParameter::new(&schema_name, Type::TEXT),
            TypedQueryParameter::new(&LEGACY_TEXT_QUERY_LIMIT_PG16_V1, Type::INT4),
            TypedQueryParameter::new(&LEGACY_SET_QUERY_LIMIT_PG16_V1, Type::INT8),
        ],
        MAX_LEGACY_ROWS_PER_SET_PG16_V1,
        "relation-statistic rows",
    )
    .await?
    {
        let row = LegacyRow::try_new(&row)?;
        let table = row.text("bounded_text_0", "relation-statistic table")?;
        let estimate: i64 = row.scalar("row_estimate", "relation estimate")?;
        if estimate >= 0 {
            if let Some(schema) = schemas.get_mut(&table) {
                schema.row_estimate = Some(estimate as u64);
            }
        }
    }
    for row in query_bounded(
        client,
        NDISTINCT_SQL,
        &[
            TypedQueryParameter::new(&tables, Type::TEXT_ARRAY),
            TypedQueryParameter::new(&schema_name, Type::TEXT),
            TypedQueryParameter::new(&LEGACY_TEXT_QUERY_LIMIT_PG16_V1, Type::INT4),
            TypedQueryParameter::new(&LEGACY_SET_QUERY_LIMIT_PG16_V1, Type::INT8),
        ],
        MAX_LEGACY_ROWS_PER_SET_PG16_V1,
        "column-statistic rows",
    )
    .await?
    {
        let row = LegacyRow::try_new(&row)?;
        let table = row.text("bounded_text_0", "column-statistic table")?;
        let column = row.text("bounded_text_1", "column-statistic column")?;
        let estimate: f32 = row.scalar("distinct_estimate", "distinct estimate")?;
        if let Some(schema) = schemas.get_mut(&table) {
            let distinct = if estimate >= 0.0 {
                estimate as u64
            } else {
                schema
                    .row_estimate
                    .map(|rows| ((-estimate as f64) * rows as f64).round() as u64)
                    .unwrap_or(0)
            };
            if distinct > 0 {
                if let Some(target) = schema.columns.iter_mut().find(|c| c.name == column) {
                    target.distinct_estimate = Some(distinct);
                }
            }
        }
    }
    Ok(())
}

fn require_same_schema(
    expected: &str,
    parent: &str,
    child_table: &str,
    constraint: &str,
) -> Result<()> {
    if parent == expected {
        return Ok(());
    }
    Err(Error::Introspection(format!(
        "PostgreSQL foreign key {constraint:?} on {child_table:?} crosses from schema \
         {expected:?} to {parent:?}; schema-qualified foreign keys are not supported"
    )))
}

#[cfg(test)]
mod tests;
