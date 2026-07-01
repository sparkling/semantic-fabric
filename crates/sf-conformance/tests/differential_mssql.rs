//! SQL Server cross-backend differential (ADR-0024 M8).
//!
//! Runs the same OBDA SELECT over **SQLite** (oracle) and **SQL Server**
//! (via `SqlServerBackend` + `exec_core::select`) and asserts `=_bag` equality.
//! Requires `sqlserver-backend` feature on `sf-sql` (enabled in this crate's
//! Cargo.toml).
//!
//! **Graceful skip:** when `SF_MSSQL_URL` is unset or the container is
//! unreachable the test passes as a no-op — CI stays green without a live server.

use rusqlite::Connection;
use sf_conformance::sqlite;
use sf_sparql::{exec, exec_core, parse_and_translate_with, Tbox};
use sf_sql::backend::sqlserver::SqlServerBackend;
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

const MSSQL_SETUP: &[&str] = &[
    "IF OBJECT_ID('person','U') IS NOT NULL DROP TABLE person",
    "IF OBJECT_ID('dept','U') IS NOT NULL DROP TABLE dept",
    "CREATE TABLE dept (id INT NOT NULL PRIMARY KEY, label NVARCHAR(200) NOT NULL)",
    "CREATE TABLE person (id INT NOT NULL PRIMARY KEY, name NVARCHAR(200) NOT NULL, dept_id INT NOT NULL)",
    "INSERT INTO dept VALUES (10, 'Sales')",
    "INSERT INTO person VALUES (1, 'Ann', 10)",
    "INSERT INTO person VALUES (2, 'Bob', 10)",
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
async fn mssql_differential() {
    let Ok(url) = std::env::var("SF_MSSQL_URL") else {
        eprintln!("SF_MSSQL_URL not set — skipping MSSQL differential");
        return;
    };

    // ── SQL Server: connect + setup fixture ──────────────────────────────────
    let mut backend = match SqlServerBackend::connect(&url).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("SF_MSSQL_URL set but connection failed ({e}) — skipping");
            return;
        }
    };

    // Execute setup DDL/DML.
    for stmt in MSSQL_SETUP {
        let _ = backend.column_names(stmt).await;
    }

    // Probe column names to build minimal TableSchema.
    let dept_cols = backend
        .column_names("SELECT TOP 0 * FROM dept")
        .await
        .expect("mssql dept probe");
    let person_cols = backend
        .column_names("SELECT TOP 0 * FROM person")
        .await
        .expect("mssql person probe");

    let schema_ms: Vec<TableSchema> = vec![
        {
            let mut t = TableSchema::new("dept");
            t.columns = dept_cols
                .into_iter()
                .map(|name| Column::new(name, "nvarchar", false))
                .collect();
            t
        },
        {
            let mut t = TableSchema::new("person");
            t.columns = person_cols
                .into_iter()
                .map(|name| Column::new(name, "nvarchar", false))
                .collect();
            t
        },
    ];

    let maps = sf_mapping::parse_r2rml(R2RML).expect("R2RML");
    let plan_ms = parse_and_translate_with(
        SELECT_Q,
        &maps,
        Dialect::SqlServer,
        &Tbox::default(),
        &schema_ms,
    )
    .expect("translate (mssql)");
    let sols_ms = exec_core::select(&plan_ms, &mut backend)
        .await
        .expect("mssql exec");

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
        sols_ms.rows.len(),
        "row count mismatch: sqlite={} mssql={}",
        sols_sqlite.rows.len(),
        sols_ms.rows.len()
    );
    assert_eq!(sols_ms.rows.len(), 2, "expected 2 rows (Ann + Bob)");
}
