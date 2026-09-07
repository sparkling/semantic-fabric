//! Namespace-safe PostgreSQL catalogue introspection.

use std::collections::{BTreeMap, HashMap};

use tokio_postgres::types::Type;

use crate::error::{Error, Result};
use crate::schema::{Column, ForeignKey, TableSchema};

mod evidence;
mod generation;
mod legacy_bounds;
mod legacy_inventory;
mod legacy_query;
mod legacy_row;
mod legacy_sql;
mod observation;
mod observed_snapshot;
#[allow(unused_imports)] // Re-exported by introspect.rs when the serving caller is wired.
pub use generation::{
    introspect_postgres_public_observed_snapshot_in_transaction,
    introspect_postgres_public_observed_snapshot_in_transaction_classified,
    lock_postgres_public_base_tables, lock_postgres_public_base_tables_classified,
    PostgresGenerationObservationFailure, PostgresPublicTableLockFailure,
    POSTGRES_GENERATION_TRANSACTION_PROBE_QUERY_COUNT_V1,
    POSTGRES_PROFILE_PREQUALIFICATION_QUERY_COUNT_V1,
};
use legacy_bounds::{
    require_same_schema, validate_legacy_table_names, PRODUCTION_LEGACY_INPUT_LIMITS_V1,
};
use legacy_inventory::LegacyCatalogueQueryV1;
use legacy_query::{
    query_bounded, query_bounded_observed, TypedQueryParameter,
    LEGACY_RELATION_QUERY_LIMIT_PG16_V1, LEGACY_SET_QUERY_LIMIT_PG16_V1,
    MAX_LEGACY_RELATIONS_PG16_V1, MAX_LEGACY_ROWS_PER_SET_PG16_V1,
};
use legacy_row::{LegacyRow, LEGACY_TEXT_QUERY_LIMIT_PG16_V1};
#[cfg(test)]
use legacy_sql::{
    COLUMNS_SQL, EARLIER_RELATION_COLLISIONS_SQL, FOREIGN_KEYS_SQL, KEYS_SQL, NDISTINCT_SQL,
    RELTUPLES_SQL, TABLES_SQL,
};

/// Query count for one complete version-1 legacy public-catalogue projection.
pub const POSTGRES_LEGACY_CATALOGUE_QUERY_COUNT_V1: u64 =
    legacy_inventory::LEGACY_CATALOGUE_QUERY_INVENTORY_V1.len() as u64;
/// Query count for one version-1 rich-profile capture, including its guard.
pub const POSTGRES_RICH_CAPTURE_CATALOGUE_QUERY_COUNT_V1: u64 =
    observation::RICH_CAPTURE_QUERY_INVENTORY_V1.len() as u64;
pub use observation::{
    Postgres16PublicObservedSchemaV1, Postgres16PublicObservedSnapshotV1,
    PostgresSchemaIdentityAvailabilityV1, PostgresSchemaIdentityGuardCodeV1,
    PostgresSchemaIdentityLimitCodeV1, PostgresSchemaIdentityUnavailableV1,
    POSTGRES16_PUBLIC_CONSTRAINT_PROFILE_ID_V1, POSTGRES16_PUBLIC_STRUCTURAL_PROFILE_ID_V1,
    POSTGRES16_PUBLIC_TYPE_PROFILE_ID_V1,
};
pub use observed_snapshot::{
    introspect_postgres_public_observed_snapshot, introspect_postgres_public_snapshot_guarded,
};

#[cfg(feature = "postgres-observation-evidence")]
pub use evidence::{
    PostgresObservationCommitV1, PostgresObservationEvidenceV1, PostgresObservationPhaseV1,
    PostgresObservationSavepointV1, PostgresObservationStreamEvidenceV1,
    PostgresObservationStreamTerminalV1, PostgresObservationStreamV1,
};
#[cfg(feature = "postgres-observation-evidence")]
pub use observed_snapshot::introspect_postgres_public_observed_snapshot_with_evidence;

const RUNTIME_SCHEMA: &str = "public";
const SNAPSHOT_TIMEOUTS_SQL: &str =
    "SET LOCAL statement_timeout = '5s'; SET LOCAL lock_timeout = '1s'; SELECT set_config('search_path','pg_catalog,public,pg_temp',true);";

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
        LegacyCatalogueQueryV1::Tables.sql(),
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

async fn introspect_in_schema<C>(
    client: &C,
    schema_name: &str,
    tables: &[String],
) -> Result<Vec<TableSchema>>
where
    C: tokio_postgres::GenericClient + Sync,
{
    let mut observer = evidence::NoopPostgresObservationObserverV1;
    introspect_in_schema_observed(client, schema_name, tables, &mut observer).await
}

async fn introspect_in_schema_observed<C, O>(
    client: &C,
    schema_name: &str,
    tables: &[String],
    observer: &mut O,
) -> Result<Vec<TableSchema>>
where
    C: tokio_postgres::GenericClient + Sync,
    O: evidence::PostgresObservationObserverV1,
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

    reject_earlier_relation_collisions(client, tables, observer).await?;
    load_columns(client, schema_name, tables, &mut schemas, observer).await?;
    load_keys(client, schema_name, tables, &mut schemas, observer).await?;
    load_foreign_keys(client, schema_name, tables, &mut schemas, observer).await?;
    load_statistics(client, schema_name, tables, &mut schemas, observer).await?;

    tables
        .iter()
        .map(|table| {
            schemas.remove(table).ok_or_else(|| {
                Error::Introspection("PostgreSQL introspection lost a requested table".into())
            })
        })
        .collect()
}

async fn reject_earlier_relation_collisions<C, O>(
    client: &C,
    tables: &[String],
    observer: &mut O,
) -> Result<()>
where
    C: tokio_postgres::GenericClient + Sync,
    O: evidence::PostgresObservationObserverV1,
{
    let earlier_schema = "pg_catalog";
    let collisions: Vec<String> = query_bounded_observed(
        client,
        LegacyCatalogueQueryV1::EarlierRelationCollisions.sql(),
        &[
            TypedQueryParameter::new(&tables, Type::TEXT_ARRAY),
            TypedQueryParameter::new(&earlier_schema, Type::TEXT),
            TypedQueryParameter::new(&LEGACY_TEXT_QUERY_LIMIT_PG16_V1, Type::INT4),
            TypedQueryParameter::new(&LEGACY_RELATION_QUERY_LIMIT_PG16_V1, Type::INT8),
        ],
        MAX_LEGACY_RELATIONS_PG16_V1,
        "catalogue-collision rows",
        evidence::PostgresObservationStreamV1::LegacyEarlierCollisions,
        observer,
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

async fn load_columns<C, O>(
    client: &C,
    schema_name: &str,
    tables: &[String],
    schemas: &mut BTreeMap<String, TableSchema>,
    observer: &mut O,
) -> Result<()>
where
    C: tokio_postgres::GenericClient + Sync,
    O: evidence::PostgresObservationObserverV1,
{
    for row in query_bounded_observed(
        client,
        LegacyCatalogueQueryV1::Columns.sql(),
        &[
            TypedQueryParameter::new(&tables, Type::TEXT_ARRAY),
            TypedQueryParameter::new(&schema_name, Type::TEXT),
            TypedQueryParameter::new(&LEGACY_TEXT_QUERY_LIMIT_PG16_V1, Type::INT4),
            TypedQueryParameter::new(&LEGACY_SET_QUERY_LIMIT_PG16_V1, Type::INT8),
        ],
        MAX_LEGACY_ROWS_PER_SET_PG16_V1,
        "column rows",
        evidence::PostgresObservationStreamV1::LegacyColumns,
        observer,
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

async fn load_keys<C, O>(
    client: &C,
    schema_name: &str,
    tables: &[String],
    schemas: &mut BTreeMap<String, TableSchema>,
    observer: &mut O,
) -> Result<()>
where
    C: tokio_postgres::GenericClient + Sync,
    O: evidence::PostgresObservationObserverV1,
{
    let mut primary: HashMap<String, Vec<String>> = HashMap::new();
    let mut unique: HashMap<String, BTreeMap<String, Vec<String>>> = HashMap::new();
    for row in query_bounded_observed(
        client,
        LegacyCatalogueQueryV1::Keys.sql(),
        &[
            TypedQueryParameter::new(&tables, Type::TEXT_ARRAY),
            TypedQueryParameter::new(&schema_name, Type::TEXT),
            TypedQueryParameter::new(&LEGACY_TEXT_QUERY_LIMIT_PG16_V1, Type::INT4),
            TypedQueryParameter::new(&LEGACY_SET_QUERY_LIMIT_PG16_V1, Type::INT8),
        ],
        MAX_LEGACY_ROWS_PER_SET_PG16_V1,
        "key rows",
        evidence::PostgresObservationStreamV1::LegacyKeys,
        observer,
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

async fn load_foreign_keys<C, O>(
    client: &C,
    schema_name: &str,
    tables: &[String],
    schemas: &mut BTreeMap<String, TableSchema>,
    observer: &mut O,
) -> Result<()>
where
    C: tokio_postgres::GenericClient + Sync,
    O: evidence::PostgresObservationObserverV1,
{
    let mut foreign: HashMap<String, BTreeMap<String, ForeignKey>> = HashMap::new();
    for row in query_bounded_observed(
        client,
        LegacyCatalogueQueryV1::ForeignKeys.sql(),
        &[
            TypedQueryParameter::new(&tables, Type::TEXT_ARRAY),
            TypedQueryParameter::new(&schema_name, Type::TEXT),
            TypedQueryParameter::new(&LEGACY_TEXT_QUERY_LIMIT_PG16_V1, Type::INT4),
            TypedQueryParameter::new(&LEGACY_SET_QUERY_LIMIT_PG16_V1, Type::INT8),
        ],
        MAX_LEGACY_ROWS_PER_SET_PG16_V1,
        "foreign-key rows",
        evidence::PostgresObservationStreamV1::LegacyForeignKeys,
        observer,
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

async fn load_statistics<C, O>(
    client: &C,
    schema_name: &str,
    tables: &[String],
    schemas: &mut BTreeMap<String, TableSchema>,
    observer: &mut O,
) -> Result<()>
where
    C: tokio_postgres::GenericClient + Sync,
    O: evidence::PostgresObservationObserverV1,
{
    for row in query_bounded_observed(
        client,
        LegacyCatalogueQueryV1::RelationStatistics.sql(),
        &[
            TypedQueryParameter::new(&tables, Type::TEXT_ARRAY),
            TypedQueryParameter::new(&schema_name, Type::TEXT),
            TypedQueryParameter::new(&LEGACY_TEXT_QUERY_LIMIT_PG16_V1, Type::INT4),
            TypedQueryParameter::new(&LEGACY_SET_QUERY_LIMIT_PG16_V1, Type::INT8),
        ],
        MAX_LEGACY_ROWS_PER_SET_PG16_V1,
        "relation-statistic rows",
        evidence::PostgresObservationStreamV1::LegacyRelationStatistics,
        observer,
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
    for row in query_bounded_observed(
        client,
        LegacyCatalogueQueryV1::ColumnStatistics.sql(),
        &[
            TypedQueryParameter::new(&tables, Type::TEXT_ARRAY),
            TypedQueryParameter::new(&schema_name, Type::TEXT),
            TypedQueryParameter::new(&LEGACY_TEXT_QUERY_LIMIT_PG16_V1, Type::INT4),
            TypedQueryParameter::new(&LEGACY_SET_QUERY_LIMIT_PG16_V1, Type::INT8),
        ],
        MAX_LEGACY_ROWS_PER_SET_PG16_V1,
        "column-statistic rows",
        evidence::PostgresObservationStreamV1::LegacyColumnStatistics,
        observer,
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

#[cfg(test)]
mod tests;
