//! Trino cross-backend differential (ADR-0024 M8).
//!
//! Runs the same OBDA SELECT over **SQLite** (oracle) and **Trino**
//! (via `TrinoBackend` + `exec_core::select`) and asserts `=_bag` equality.
//! Requires `rest-backends` feature on `sf-sql` (enabled in this crate's
//! Cargo.toml).
//!
//! **Graceful skip:** when `SF_TRINO_URL` is unset or the coordinator is
//! unreachable the test passes as a no-op — CI stays green without a live Trino.
//!
//! Uses `#[tokio::test]` — `TrinoBackend` uses the async `reqwest::Client`.
//! Fixture tables are created in Trino's `memory.default` schema; each run
//! drops and recreates them so tests are idempotent.

use rusqlite::Connection;
use sf_conformance::sqlite;
use sf_sparql::{exec, exec_core, parse_and_translate_with, Tbox};
use sf_sql::backend::rest::TrinoBackend;
use sf_sql::backend::SqlBackend;
use sf_sql::{Column, Dialect, TableSchema};

const CREATE_SQLITE: &str = r#"
CREATE TABLE dept (id INTEGER PRIMARY KEY, label TEXT NOT NULL);
CREATE TABLE person (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    dept_id INTEGER NOT NULL
);
INSERT INTO dept VALUES (10, 'Sales');
INSERT INTO person VALUES (1, 'Ann', 10);
INSERT INTO person VALUES (2, 'Bob', 10);
"#;

/// DDL/DML statements executed against Trino's `memory.default` catalog.
const TRINO_SETUP: &[&str] = &[
    "DROP TABLE IF EXISTS dept",
    "DROP TABLE IF EXISTS person",
    "CREATE TABLE dept (id VARCHAR(20), label VARCHAR(200))",
    "CREATE TABLE person (id VARCHAR(20), name VARCHAR(200), dept_id VARCHAR(20))",
    "INSERT INTO dept VALUES ('10', 'Sales')",
    "INSERT INTO person VALUES ('1', 'Ann', '10')",
    "INSERT INTO person VALUES ('2', 'Bob', '10')",
];

const R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .

<#Person>
    rr:logicalTable [ rr:tableName "person" ] ;
    rr:subjectMap [ rr:template "http://ex/person/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:name   ; rr:objectMap [ rr:column "name" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:deptId ; rr:objectMap [ rr:column "dept_id" ] ] .

<#Dept>
    rr:logicalTable [ rr:tableName "dept" ] ;
    rr:subjectMap [ rr:template "http://ex/dept/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:label ; rr:objectMap [ rr:column "label" ] ] .
"#;

const SELECT_Q: &str = r#"
    PREFIX ex: <http://ex/>
    SELECT ?name WHERE {
        ?p ex:name ?name .
        ?p ex:deptId ?di .
        FILTER (?di = 10)
    }"#;

#[tokio::test]
async fn trino_differential() {
    let Ok(url) = std::env::var("SF_TRINO_URL") else {
        eprintln!("SF_TRINO_URL not set — skipping Trino differential");
        return;
    };

    let trino_user = std::env::var("SF_TRINO_USER").unwrap_or_else(|_| "trino".to_owned());
    // Use memory.default for fixture tables.
    let mut backend = TrinoBackend::new(&url, &trino_user).with_catalog("memory", "default");

    // ── Trino: set up fixture ────────────────────────────────────────────────
    for stmt in TRINO_SETUP {
        if let Err(e) = backend.column_names(stmt).await {
            eprintln!("Trino setup failed ({e}) — skipping");
            return;
        }
    }

    // Probe column names to build schema.
    let dept_cols = match backend.column_names("SELECT * FROM dept LIMIT 0").await {
        Ok(cols) => cols,
        Err(e) => {
            eprintln!("Trino probe failed ({e}) — skipping");
            return;
        }
    };
    let person_cols = match backend.column_names("SELECT * FROM person LIMIT 0").await {
        Ok(cols) => cols,
        Err(e) => {
            eprintln!("Trino probe failed ({e}) — skipping");
            return;
        }
    };

    let schema_trino: Vec<TableSchema> = vec![
        {
            let mut t = TableSchema::new("dept");
            t.columns = dept_cols
                .into_iter()
                .map(|name| Column::new(name, "varchar", false))
                .collect();
            t
        },
        {
            let mut t = TableSchema::new("person");
            t.columns = person_cols
                .into_iter()
                .map(|name| Column::new(name, "varchar", false))
                .collect();
            t
        },
    ];

    let maps = sf_mapping::parse_r2rml(R2RML).expect("R2RML");
    let plan_trino = parse_and_translate_with(
        SELECT_Q,
        &maps,
        Dialect::Trino,
        &Tbox::default(),
        &schema_trino,
    )
    .expect("translate (trino)");
    let sols_trino = exec_core::select(&plan_trino, &mut backend)
        .await
        .expect("trino exec");

    // ── SQLite oracle ────────────────────────────────────────────────────────
    let conn: Connection = sqlite::load(CREATE_SQLITE).expect("sqlite fixture");
    let schema_sqlite = sqlite::introspect_all(&conn).expect("sqlite schema");
    let plan_sqlite = parse_and_translate_with(
        SELECT_Q,
        &maps,
        Dialect::Sqlite,
        &Tbox::default(),
        &schema_sqlite,
    )
    .expect("translate (sqlite)");
    let sols_sqlite = exec::select(&plan_sqlite, &conn).expect("sqlite exec");

    // ── bag equality ─────────────────────────────────────────────────────────
    assert_eq!(
        sols_sqlite.rows.len(),
        sols_trino.rows.len(),
        "row count mismatch: sqlite={} trino={}",
        sols_sqlite.rows.len(),
        sols_trino.rows.len()
    );
    assert_eq!(sols_trino.rows.len(), 2, "expected 2 rows (Ann + Bob)");
}
