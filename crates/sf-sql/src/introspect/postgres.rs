//! Namespace-safe PostgreSQL catalogue introspection.

use std::collections::{BTreeMap, HashMap};

use crate::error::{Error, Result};
use crate::schema::{Column, ForeignKey, TableSchema};

const RUNTIME_SCHEMA: &str = "public";

const TABLES_SQL: &str = "SELECT table_name FROM information_schema.tables \
     WHERE table_schema = $1 AND table_type = 'BASE TABLE' ORDER BY table_name";

// Generated base-table SQL is intentionally unqualified under the runtime's
// exact `pg_catalog,public,pg_temp` search path. A `public` base table that has
// the same name as any `pg_catalog` relation would therefore be introspected
// from `public` but executed from the earlier catalogue namespace. Until the IR
// carries qualified relation identity, reject that database at introspection.
const EARLIER_RELATION_COLLISIONS_SQL: &str = "SELECT c.relname FROM pg_catalog.pg_class c \
     JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
     WHERE c.relname = ANY($1) AND n.nspname = $2 ORDER BY c.relname";

const COLUMNS_SQL: &str = "SELECT table_name, column_name, data_type, is_nullable \
     FROM information_schema.columns \
     WHERE table_name = ANY($1) AND table_schema = $2 \
     ORDER BY table_name, ordinal_position";

const KEYS_SQL: &str =
    "SELECT tc.table_name, tc.constraint_type, tc.constraint_name, kcu.column_name \
     FROM information_schema.table_constraints tc \
     JOIN information_schema.key_column_usage kcu \
       ON tc.constraint_catalog = kcu.constraint_catalog \
      AND tc.constraint_schema = kcu.constraint_schema \
      AND tc.constraint_name = kcu.constraint_name \
      AND tc.table_catalog = kcu.table_catalog \
      AND tc.table_schema = kcu.table_schema \
      AND tc.table_name = kcu.table_name \
     WHERE tc.table_name = ANY($1) AND tc.table_schema = $2 \
       AND tc.constraint_type IN ('PRIMARY KEY', 'UNIQUE') \
     ORDER BY tc.table_name, tc.constraint_name, kcu.ordinal_position";

// The paired attnum arrays preserve composite-FK column alignment. Parent
// namespace is selected explicitly because the current DTO cannot represent a
// schema-qualified parent and must reject that case rather than misbind it.
const FOREIGN_KEYS_SQL: &str = "SELECT child.relname, con.conname, ca.attname, parent.relname, \
            parent_ns.nspname, pa.attname \
     FROM pg_catalog.pg_constraint con \
     JOIN pg_catalog.pg_class child ON child.oid = con.conrelid \
     JOIN pg_catalog.pg_namespace child_ns ON child_ns.oid = child.relnamespace \
     JOIN pg_catalog.pg_class parent ON parent.oid = con.confrelid \
     JOIN pg_catalog.pg_namespace parent_ns ON parent_ns.oid = parent.relnamespace \
     JOIN LATERAL ROWS FROM ( \
          pg_catalog.unnest(con.conkey), pg_catalog.unnest(con.confkey) \
     ) WITH ORDINALITY AS k(child_attnum, parent_attnum, ord) ON true \
     JOIN pg_catalog.pg_attribute ca \
       ON ca.attrelid = con.conrelid AND ca.attnum = k.child_attnum \
     JOIN pg_catalog.pg_attribute pa \
       ON pa.attrelid = con.confrelid AND pa.attnum = k.parent_attnum \
     WHERE con.contype = 'f' AND child.relname = ANY($1) \
       AND child_ns.nspname = $2 \
     ORDER BY child.relname, con.conname, k.ord";

const RELTUPLES_SQL: &str = "SELECT c.relname, GREATEST(c.reltuples, 0)::bigint \
     FROM pg_catalog.pg_class c \
     JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
     WHERE c.relname = ANY($1) AND n.nspname = $2 \
       AND c.relkind IN ('r', 'p', 'm', 'v')";

const NDISTINCT_SQL: &str = "SELECT tablename, attname, n_distinct FROM pg_catalog.pg_stats \
     WHERE tablename = ANY($1) AND schemaname = $2";

/// Introspect one table from the runtime-supported PostgreSQL `public` schema.
pub async fn introspect_postgres(
    client: &tokio_postgres::Client,
    table: &str,
) -> Result<TableSchema> {
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
    let rows = transaction.query(TABLES_SQL, &[&RUNTIME_SCHEMA]).await?;
    let tables: Vec<String> = rows.into_iter().map(|row| row.get(0)).collect();
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
    let collisions: Vec<String> = client
        .query(EARLIER_RELATION_COLLISIONS_SQL, &[&tables, &earlier_schema])
        .await?
        .into_iter()
        .map(|row| row.get(0))
        .collect();
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
    for row in client.query(COLUMNS_SQL, &[&tables, &schema_name]).await? {
        let table: String = row.get(0);
        let name: String = row.get(1);
        let data_type: String = row.get(2);
        let is_nullable: String = row.get(3);
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
    for row in client.query(KEYS_SQL, &[&tables, &schema_name]).await? {
        let table: String = row.get(0);
        let constraint_type: String = row.get(1);
        let constraint: String = row.get(2);
        let column: String = row.get(3);
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
    for row in client
        .query(FOREIGN_KEYS_SQL, &[&tables, &schema_name])
        .await?
    {
        let table: String = row.get(0);
        let constraint: String = row.get(1);
        let column: String = row.get(2);
        let parent_table: String = row.get(3);
        let parent_schema: String = row.get(4);
        let parent_column: String = row.get(5);
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
    for row in client
        .query(RELTUPLES_SQL, &[&tables, &schema_name])
        .await?
    {
        let table: String = row.get(0);
        let estimate: i64 = row.get(1);
        if estimate >= 0 {
            if let Some(schema) = schemas.get_mut(&table) {
                schema.row_estimate = Some(estimate as u64);
            }
        }
    }
    for row in client
        .query(NDISTINCT_SQL, &[&tables, &schema_name])
        .await?
    {
        let table: String = row.get(0);
        let column: String = row.get(1);
        let estimate: f32 = row.get(2);
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
mod tests {
    use super::*;

    #[test]
    fn catalogue_queries_bind_complete_relation_identity() {
        for (sql, schema_identity) in [
            (COLUMNS_SQL, "table_schema"),
            (KEYS_SQL, "table_schema"),
            (FOREIGN_KEYS_SQL, "nspname"),
            (RELTUPLES_SQL, "nspname"),
            (NDISTINCT_SQL, "schemaname"),
        ] {
            assert!(sql.contains("ANY($1)"));
            assert!(sql.contains("$2"));
            assert!(sql.contains(schema_identity));
        }
        assert!(TABLES_SQL.contains("table_schema = $1"));
        assert!(EARLIER_RELATION_COLLISIONS_SQL.contains("c.relname = ANY($1)"));
        assert!(EARLIER_RELATION_COLLISIONS_SQL.contains("n.nspname = $2"));
        assert!(KEYS_SQL.contains("tc.table_name = kcu.table_name"));
        assert!(KEYS_SQL.contains("tc.table_catalog = kcu.table_catalog"));
    }

    #[test]
    fn cross_schema_foreign_keys_fail_closed() {
        assert!(require_same_schema("public", "public", "child", "fk").is_ok());
        let error = require_same_schema("public", "private", "child", "fk")
            .expect_err("unrepresentable qualified parent must fail closed");
        assert!(error.to_string().contains("schema-qualified"));
    }

    #[test]
    fn earlier_relation_collision_message_is_non_ambiguous() {
        let error = Error::Introspection(
            "PostgreSQL public relation name(s) collide with the earlier pg_catalog execution scope: pg_class; qualified relation identity is not yet supported".into(),
        );
        assert!(error.to_string().contains("pg_catalog"));
        assert!(error.to_string().contains("qualified relation identity"));
    }
}
