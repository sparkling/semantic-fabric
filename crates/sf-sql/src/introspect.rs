//! Per-DBMS schema introspection (ADR-0006 crate table; ADR-0015 type
//! determination). Fills the dialect-neutral [`crate::schema::TableSchema`].
//!
//! * **SQLite** ([`introspect_sqlite`]) — fully implemented over the
//!   table-valued `pragma_*` functions and `sqlite_stat1`, all via **bound
//!   parameters** (no string-built catalog SQL) and the streaming cursor. Tested
//!   against an in-memory database.
//! * **PostgreSQL** ([`introspect_postgres`]) — implemented over
//!   `information_schema` + `pg_class.reltuples` + `pg_stats`; it needs a live
//!   server, so it is exercised by the integration suite (ADR-0012), not here.
//!
//! Catalog result sets are tiny and bounded, so introspection uses ordinary
//! buffered queries — the buffer-all ban (ADR-0010 §C) applies to *instance-data*
//! result streaming ([`crate::stream`]), not to a handful of catalog rows.

use std::collections::{BTreeMap, HashMap};

use crate::error::{Error, Result};
use crate::schema::{Column, ForeignKey, TableSchema};

// --- SQLite ---------------------------------------------------------------

/// Introspect `table` from a live SQLite connection: columns + types, primary
/// key, unique constraints, foreign keys, and — when `ANALYZE` has populated
/// `sqlite_stat1` — row and distinct-key estimates.
pub fn introspect_sqlite(conn: &rusqlite::Connection, table: &str) -> Result<TableSchema> {
    let mut schema = TableSchema::new(table);
    sqlite_columns_and_pk(conn, table, &mut schema)?;
    sqlite_foreign_keys(conn, table, &mut schema)?;
    let index_first_col = sqlite_unique_indexes(conn, table, &mut schema)?;
    sqlite_statistics(conn, table, &index_first_col, &mut schema)?;
    Ok(schema)
}

/// Columns (name, type, NOT NULL) and the ordered primary key, from
/// `pragma_table_info`.
fn sqlite_columns_and_pk(
    conn: &rusqlite::Connection,
    table: &str,
    schema: &mut TableSchema,
) -> Result<()> {
    let mut stmt =
        conn.prepare(r#"SELECT name, "type", "notnull", pk FROM pragma_table_info(?1)"#)?;
    let mut rows = stmt.query([table])?; // cursor, one row in flight
    let mut pk: Vec<(i64, String)> = Vec::new();
    while let Some(row) = rows.next()? {
        let name: String = row.get(0)?;
        let sql_type: String = row.get(1)?;
        let not_null: i64 = row.get(2)?;
        let pk_pos: i64 = row.get(3)?;
        if pk_pos > 0 {
            pk.push((pk_pos, name.clone()));
        }
        schema
            .columns
            .push(Column::new(name, sql_type, not_null != 0));
    }
    if schema.columns.is_empty() {
        return Err(Error::Introspection(format!(
            "SQLite table {table:?} not found or has no columns"
        )));
    }
    pk.sort_by_key(|(pos, _)| *pos);
    schema.primary_key = pk.into_iter().map(|(_, n)| n).collect();
    Ok(())
}

/// Foreign keys (grouped into composite keys by `id`), from
/// `pragma_foreign_key_list`.
fn sqlite_foreign_keys(
    conn: &rusqlite::Connection,
    table: &str,
    schema: &mut TableSchema,
) -> Result<()> {
    let mut stmt = conn.prepare(
        r#"SELECT id, "table", "from", "to" FROM pragma_foreign_key_list(?1) ORDER BY id, seq"#,
    )?;
    let mut rows = stmt.query([table])?;
    let mut current: Option<(i64, ForeignKey)> = None;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let parent_table: String = row.get(1)?;
        let from: String = row.get(2)?;
        let to: String = row.get(3)?;
        match &mut current {
            Some((cur_id, fk)) if *cur_id == id => {
                fk.columns.push(from);
                fk.parent_columns.push(to);
            }
            _ => {
                if let Some((_, fk)) = current.take() {
                    schema.foreign_keys.push(fk);
                }
                current = Some((
                    id,
                    ForeignKey {
                        columns: vec![from],
                        parent_table,
                        parent_columns: vec![to],
                    },
                ));
            }
        }
    }
    if let Some((_, fk)) = current.take() {
        schema.foreign_keys.push(fk);
    }
    Ok(())
}

/// Unique constraints (origin `u`/`c`, not the PK auto-index) from
/// `pragma_index_list`, and the first indexed column of *every* index (for the
/// `sqlite_stat1` distinct-key join).
fn sqlite_unique_indexes(
    conn: &rusqlite::Connection,
    table: &str,
    schema: &mut TableSchema,
) -> Result<HashMap<String, String>> {
    let mut index_first_col: HashMap<String, String> = HashMap::new();
    let mut stmt = conn.prepare(r#"SELECT name, "unique", origin FROM pragma_index_list(?1)"#)?;
    let mut rows = stmt.query([table])?;
    let mut indexes: Vec<(String, bool, String)> = Vec::new();
    while let Some(row) = rows.next()? {
        let name: String = row.get(0)?;
        let unique: i64 = row.get(1)?;
        let origin: String = row.get(2)?;
        indexes.push((name, unique != 0, origin));
    }
    for (idx_name, is_unique, origin) in indexes {
        let cols = sqlite_index_columns(conn, &idx_name)?;
        if let Some(first) = cols.first() {
            index_first_col.insert(idx_name.clone(), first.clone());
        }
        // PK columns are already in `primary_key`; record only real unique
        // constraints/indexes here.
        if is_unique && origin != "pk" && !cols.is_empty() {
            schema.unique.push(cols);
        }
    }
    Ok(index_first_col)
}

/// The columns of one index, in order, from `pragma_index_info`.
fn sqlite_index_columns(conn: &rusqlite::Connection, index: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")?;
    let mut rows = stmt.query([index])?;
    let mut cols = Vec::new();
    while let Some(row) = rows.next()? {
        let name: Option<String> = row.get(0)?; // NULL for expression columns
        if let Some(name) = name {
            cols.push(name);
        }
    }
    Ok(cols)
}

/// Row estimate + per-column distinct-key estimates from `sqlite_stat1` (present
/// only after `ANALYZE`). The `stat` string is `"<rows> <avg0> <avg1> …"`, where
/// `avg0` is the average number of rows sharing the index's first-column value,
/// so `distinct(first_col) ≈ ⌈rows / avg0⌉`.
fn sqlite_statistics(
    conn: &rusqlite::Connection,
    table: &str,
    index_first_col: &HashMap<String, String>,
    schema: &mut TableSchema,
) -> Result<()> {
    if !sqlite_has_stat1(conn)? {
        return Ok(());
    }
    let mut stmt = conn.prepare("SELECT idx, stat FROM sqlite_stat1 WHERE tbl = ?1")?;
    let mut rows = stmt.query([table])?;
    let mut distinct_by_col: HashMap<String, u64> = HashMap::new();
    while let Some(row) = rows.next()? {
        let idx: Option<String> = row.get(0)?;
        let stat: String = row.get(1)?;
        let mut toks = stat.split_whitespace();
        let Some(total) = toks.next().and_then(|t| t.parse::<u64>().ok()) else {
            continue;
        };
        schema.row_estimate = Some(schema.row_estimate.map_or(total, |r| r.max(total)));
        if let (Some(idx), Some(avg0)) = (
            idx.as_ref(),
            toks.next().and_then(|t| t.parse::<u64>().ok()),
        ) {
            if let Some(col) = index_first_col.get(idx) {
                let distinct = if avg0 == 0 {
                    total
                } else {
                    total.div_ceil(avg0)
                };
                distinct_by_col
                    .entry(col.clone())
                    .and_modify(|d| *d = (*d).max(distinct))
                    .or_insert(distinct);
            }
        }
    }
    for col in &mut schema.columns {
        if let Some(d) = distinct_by_col.get(&col.name) {
            col.distinct_estimate = Some(*d);
        }
    }
    Ok(())
}

fn sqlite_has_stat1(conn: &rusqlite::Connection) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='sqlite_stat1'",
        [],
        |r| r.get(0),
    )?;
    Ok(count > 0)
}

// --- PostgreSQL (integration-tested, ADR-0012) ----------------------------

const PG_COLUMNS_SQL: &str = "SELECT column_name, data_type, is_nullable \
     FROM information_schema.columns WHERE table_name = $1 ORDER BY ordinal_position";

const PG_KEYS_SQL: &str = "SELECT tc.constraint_type, tc.constraint_name, kcu.column_name \
     FROM information_schema.table_constraints tc \
     JOIN information_schema.key_column_usage kcu \
       ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema \
     WHERE tc.table_name = $1 AND tc.constraint_type IN ('PRIMARY KEY', 'UNIQUE') \
     ORDER BY tc.constraint_name, kcu.ordinal_position";

// Composite-FK column alignment via `pg_catalog`: the parallel `conkey`/`confkey`
// attnum arrays are unnested WITH ORDINALITY so child↔parent columns stay paired
// in order. (The `information_schema` `key_column_usage`×`constraint_column_usage`
// join cross-products a composite FK's columns and loses the pairing — the
// ADR-0014 weakness.) Columns: constraint_name, child_column, parent_table,
// parent_column — matching the row reader below.
const PG_FK_SQL: &str = "SELECT con.conname, ca.attname, parent.relname, pa.attname \
     FROM pg_constraint con \
     JOIN pg_class child ON child.oid = con.conrelid \
     JOIN pg_class parent ON parent.oid = con.confrelid \
     JOIN LATERAL unnest(con.conkey, con.confkey) WITH ORDINALITY AS k(child_attnum, parent_attnum, ord) ON true \
     JOIN pg_attribute ca ON ca.attrelid = con.conrelid AND ca.attnum = k.child_attnum \
     JOIN pg_attribute pa ON pa.attrelid = con.confrelid AND pa.attnum = k.parent_attnum \
     WHERE con.contype = 'f' AND child.relname = $1 \
     ORDER BY con.conname, k.ord";

const PG_RELTUPLES_SQL: &str = "SELECT GREATEST(reltuples, 0)::bigint FROM pg_class \
     WHERE relname = $1 AND relkind IN ('r', 'p', 'm', 'v')";

const PG_NDISTINCT_SQL: &str = "SELECT attname, n_distinct FROM pg_stats WHERE tablename = $1";

/// Introspect `table` from a live PostgreSQL connection (`information_schema` +
/// `pg_class.reltuples` + `pg_stats`). All catalog SQL binds the table name as a
/// parameter (ADR-0010 R1). Requires a live server, so it is covered by the
/// integration suite (ADR-0012), not the in-crate unit tests.
///
/// Caveat: composite-FK parent-column alignment via `constraint_column_usage` is
/// a known `information_schema` weakness; production hardening tracks it
/// (ADR-0014).
pub async fn introspect_postgres(
    client: &tokio_postgres::Client,
    table: &str,
) -> Result<TableSchema> {
    let mut schema = TableSchema::new(table);

    for row in client.query(PG_COLUMNS_SQL, &[&table]).await? {
        let name: String = row.get(0);
        let data_type: String = row.get(1);
        let is_nullable: String = row.get(2);
        schema.columns.push(Column::new(
            name,
            data_type,
            is_nullable.eq_ignore_ascii_case("NO"),
        ));
    }
    if schema.columns.is_empty() {
        return Err(Error::Introspection(format!(
            "PostgreSQL table {table:?} not found in information_schema"
        )));
    }

    let mut pk: Vec<String> = Vec::new();
    let mut uniques: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in client.query(PG_KEYS_SQL, &[&table]).await? {
        let ctype: String = row.get(0);
        let cname: String = row.get(1);
        let col: String = row.get(2);
        if ctype == "PRIMARY KEY" {
            pk.push(col);
        } else {
            uniques.entry(cname).or_default().push(col);
        }
    }
    schema.primary_key = pk;
    schema.unique = uniques.into_values().collect();

    let mut fks: BTreeMap<String, ForeignKey> = BTreeMap::new();
    for row in client.query(PG_FK_SQL, &[&table]).await? {
        let cname: String = row.get(0);
        let col: String = row.get(1);
        let parent_table: String = row.get(2);
        let parent_col: String = row.get(3);
        let fk = fks.entry(cname).or_insert_with(|| ForeignKey {
            columns: Vec::new(),
            parent_table,
            parent_columns: Vec::new(),
        });
        fk.columns.push(col);
        fk.parent_columns.push(parent_col);
    }
    schema.foreign_keys = fks.into_values().collect();

    if let Some(row) = client.query_opt(PG_RELTUPLES_SQL, &[&table]).await? {
        let rt: i64 = row.get(0);
        if rt >= 0 {
            schema.row_estimate = Some(rt as u64);
        }
    }

    for row in client.query(PG_NDISTINCT_SQL, &[&table]).await? {
        let attname: String = row.get(0);
        let n_distinct: f32 = row.get(1);
        let distinct = if n_distinct >= 0.0 {
            n_distinct as u64
        } else {
            // Negative n_distinct is a fraction of the row count (-1 = unique).
            schema
                .row_estimate
                .map(|r| ((-n_distinct as f64) * r as f64).round() as u64)
                .unwrap_or(0)
        };
        if distinct > 0 {
            if let Some(col) = schema.columns.iter_mut().find(|c| c.name == attname) {
                col.distinct_estimate = Some(distinct);
            }
        }
    }

    Ok(schema)
}

// --- MySQL (integration-tested, ADR-0012) -------------------------------------

/// Columns, NOT NULL, and data type — from `information_schema.COLUMNS`, bound
/// by table name with a `?` positional placeholder (Dialect::MySql, ADR-0010 R1).
const MYSQL_COLUMNS_SQL: &str = "SELECT COLUMN_NAME, DATA_TYPE, IS_NULLABLE \
     FROM information_schema.COLUMNS \
     WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ? \
     ORDER BY ORDINAL_POSITION";

/// PRIMARY KEY and UNIQUE constraints — from `information_schema.STATISTICS` which
/// is per-index-column and available without additional joins (ADR-0014 §mysql-pk).
const MYSQL_KEYS_SQL: &str = "SELECT INDEX_NAME, NON_UNIQUE, COLUMN_NAME, SEQ_IN_INDEX \
     FROM information_schema.STATISTICS \
     WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ? \
     ORDER BY INDEX_NAME, SEQ_IN_INDEX";

/// Foreign keys — from `information_schema.KEY_COLUMN_USAGE` cross-joined to
/// `REFERENTIAL_CONSTRAINTS` so we get both child and parent column names in
/// ordinal order (positional `?` bind for table name, ADR-0010 R1).
const MYSQL_FK_SQL: &str =
    "SELECT kcu.CONSTRAINT_NAME, kcu.COLUMN_NAME, kcu.REFERENCED_TABLE_NAME, \
            kcu.REFERENCED_COLUMN_NAME \
     FROM information_schema.KEY_COLUMN_USAGE kcu \
     JOIN information_schema.REFERENTIAL_CONSTRAINTS rc \
       ON rc.CONSTRAINT_NAME = kcu.CONSTRAINT_NAME \
      AND rc.CONSTRAINT_SCHEMA = kcu.CONSTRAINT_SCHEMA \
     WHERE kcu.TABLE_SCHEMA = DATABASE() AND kcu.TABLE_NAME = ? \
       AND kcu.REFERENCED_TABLE_NAME IS NOT NULL \
     ORDER BY kcu.CONSTRAINT_NAME, kcu.ORDINAL_POSITION";

/// Introspect `table` from a live MySQL connection via `information_schema`.
/// All catalog SQL uses `?` positional placeholders (ADR-0010 R1). Statistics
/// (row estimate, distinct counts) are not available without `ANALYZE` having
/// been run; they are omitted and treated as unknown by the planner.
///
/// Requires a live server; exercised by the integration suite (ADR-0012).
pub async fn introspect_mysql(conn: &mut mysql_async::Conn, table: &str) -> Result<TableSchema> {
    use mysql_async::prelude::Queryable;
    use mysql_async::Value;

    let mut schema = TableSchema::new(table);

    // Columns.
    let col_rows: Vec<mysql_async::Row> =
        conn.exec(MYSQL_COLUMNS_SQL, (Value::from(table),)).await?;
    for row in &col_rows {
        let name: String = row.get(0).ok_or_else(|| {
            Error::Introspection(format!("MySQL COLUMNS missing COLUMN_NAME for {table}"))
        })?;
        let data_type: String = row.get(1).ok_or_else(|| {
            Error::Introspection(format!("MySQL COLUMNS missing DATA_TYPE for {name}"))
        })?;
        let is_nullable: String = row.get(2).ok_or_else(|| {
            Error::Introspection(format!("MySQL COLUMNS missing IS_NULLABLE for {name}"))
        })?;
        schema.columns.push(Column::new(
            name,
            data_type,
            is_nullable.eq_ignore_ascii_case("NO"),
        ));
    }
    if schema.columns.is_empty() {
        return Err(Error::Introspection(format!(
            "MySQL table {table:?} not found in information_schema"
        )));
    }

    // Primary key + unique indexes.
    let key_rows: Vec<mysql_async::Row> = conn.exec(MYSQL_KEYS_SQL, (Value::from(table),)).await?;
    let mut indexes: BTreeMap<String, (bool, Vec<String>)> = BTreeMap::new(); // name → (unique, cols)
    for row in &key_rows {
        let index_name: String = row.get(0).unwrap_or_default();
        let non_unique: u8 = row.get::<u8, _>(1).unwrap_or(1);
        let col_name: String = row.get(2).unwrap_or_default();
        let e = indexes
            .entry(index_name)
            .or_insert((non_unique == 0, Vec::new()));
        e.1.push(col_name);
    }
    for (name, (unique, cols)) in &indexes {
        if name == "PRIMARY" {
            schema.primary_key = cols.clone();
        } else if *unique {
            schema.unique.push(cols.clone());
        }
    }

    // Foreign keys.
    let fk_rows: Vec<mysql_async::Row> = conn.exec(MYSQL_FK_SQL, (Value::from(table),)).await?;
    let mut fk_map: BTreeMap<String, (Vec<String>, String, Vec<String>)> = BTreeMap::new();
    for row in &fk_rows {
        let cname: String = row.get(0).unwrap_or_default();
        let col: String = row.get(1).unwrap_or_default();
        let ptable: String = row.get(2).unwrap_or_default();
        let pcol: String = row.get(3).unwrap_or_default();
        let e = fk_map
            .entry(cname)
            .or_insert((Vec::new(), ptable, Vec::new()));
        e.0.push(col);
        e.2.push(pcol);
    }
    for (_, (cols, parent_table, parent_cols)) in fk_map {
        schema.foreign_keys.push(ForeignKey {
            columns: cols,
            parent_table,
            parent_columns: parent_cols,
        });
    }

    Ok(schema)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE departments (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
             CREATE TABLE employees (
                 id      INTEGER PRIMARY KEY,
                 dept_id INTEGER NOT NULL REFERENCES departments(id),
                 email   TEXT UNIQUE,
                 name    TEXT NOT NULL
             );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn introspects_columns_types_and_not_null() {
        let conn = fixture();
        let t = introspect_sqlite(&conn, "employees").unwrap();
        assert_eq!(t.columns.len(), 4);
        let dept = t.column("dept_id").unwrap();
        assert!(dept.not_null);
        assert!(dept.sql_type.to_uppercase().contains("INT"));
        assert!(!t.column("email").unwrap().not_null);
    }

    #[test]
    fn introspects_primary_key_and_unique_and_fk() {
        let conn = fixture();
        let t = introspect_sqlite(&conn, "employees").unwrap();
        assert_eq!(t.primary_key, vec!["id".to_owned()]);
        // The UNIQUE on `email` is reported (PK auto-index is not duplicated here).
        assert!(t.unique.iter().any(|c| c == &vec!["email".to_owned()]));
        assert!(!t.unique.iter().any(|c| c == &vec!["id".to_owned()]));
        // Foreign key dept_id -> departments(id).
        assert_eq!(t.foreign_keys.len(), 1);
        let fk = &t.foreign_keys[0];
        assert_eq!(fk.columns, vec!["dept_id".to_owned()]);
        assert_eq!(fk.parent_table, "departments");
        assert_eq!(fk.parent_columns, vec!["id".to_owned()]);
    }

    #[test]
    fn single_column_pk_is_recognised_as_unique_key() {
        let conn = fixture();
        let t = introspect_sqlite(&conn, "employees").unwrap();
        assert!(t.is_unique_key("id"));
        assert!(t.is_unique_key("email"));
        assert!(!t.is_unique_key("dept_id"));
    }

    #[test]
    fn statistics_yield_row_and_distinct_estimates() {
        let conn = fixture();
        // 3 departments; employees split across them so dept_id has 3 distinct.
        conn.execute_batch(
            "INSERT INTO departments(id, name) VALUES (1,'a'),(2,'b'),(3,'c');
             INSERT INTO employees(id, dept_id, email, name) VALUES
                 (1,1,'e1','n1'),(2,1,'e2','n2'),(3,2,'e3','n3'),
                 (4,2,'e4','n4'),(5,3,'e5','n5'),(6,3,'e6','n6');
             ANALYZE;",
        )
        .unwrap();
        let t = introspect_sqlite(&conn, "employees").unwrap();
        assert_eq!(t.row_estimate, Some(6));
        // dept_id distinct ≈ 3 (indexed via the FK? not necessarily) — assert the
        // unique-key path instead, which is always available.
        assert_eq!(t.distinct_key_cardinality("id"), Some(6)); // PK: every value distinct
        assert_eq!(t.distinct_key_cardinality("email"), Some(6)); // UNIQUE
    }

    #[test]
    fn analyze_populates_distinct_for_indexed_nonunique_column() {
        let conn = fixture();
        // Add a non-unique index on dept_id so sqlite_stat1 records its distinctness.
        conn.execute_batch(
            "CREATE INDEX emp_dept ON employees(dept_id);
             INSERT INTO departments(id, name) VALUES (1,'a'),(2,'b'),(3,'c');
             INSERT INTO employees(id, dept_id, email, name) VALUES
                 (1,1,'e1','n1'),(2,1,'e2','n2'),(3,2,'e3','n3'),
                 (4,2,'e4','n4'),(5,3,'e5','n5'),(6,3,'e6','n6');
             ANALYZE;",
        )
        .unwrap();
        let t = introspect_sqlite(&conn, "employees").unwrap();
        // dept_id has 3 distinct values over 6 rows; stat1 should estimate ~3.
        let dept = t.column("dept_id").unwrap();
        assert_eq!(dept.distinct_estimate, Some(3), "schema = {t:?}");
        assert_eq!(t.distinct_key_cardinality("dept_id"), Some(3));
    }

    #[test]
    fn missing_table_is_an_introspection_error() {
        let conn = fixture();
        assert!(introspect_sqlite(&conn, "no_such_table").is_err());
    }

    #[test]
    fn pg_catalog_sql_binds_table_as_parameter() {
        // The PostgreSQL path is integration-tested, but its catalog SQL must
        // never inline the table name — assert the $1 bind is present.
        for sql in [
            PG_COLUMNS_SQL,
            PG_KEYS_SQL,
            PG_FK_SQL,
            PG_RELTUPLES_SQL,
            PG_NDISTINCT_SQL,
        ] {
            assert!(
                sql.contains("$1"),
                "catalog SQL must bind table name: {sql}"
            );
        }
    }

    #[test]
    fn mysql_catalog_sql_binds_table_as_parameter() {
        // MySQL catalog SQL must use ? placeholders (Dialect::MySql) — never inline.
        for sql in [MYSQL_COLUMNS_SQL, MYSQL_KEYS_SQL, MYSQL_FK_SQL] {
            assert!(
                sql.contains('?'),
                "MySQL catalog SQL must bind table name: {sql}"
            );
        }
    }
}
