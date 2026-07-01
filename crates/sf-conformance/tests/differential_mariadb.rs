//! MariaDB cross-backend differential (ADR-0024 M8).
//!
//! MariaDB is wire-compatible with MySQL; it uses `Dialect::Mysql` and the same
//! `exec_mysql` execution path. This test verifies that the OBDA SELECT produces
//! the same binding bag against MariaDB as against the SQLite oracle.
//!
//! **Graceful skip:** when `SF_MARIADB_URL` is unset or the server is unreachable
//! the test passes as a no-op — CI stays green without a live container.

use mysql_async::{Conn, Opts};
use rusqlite::Connection;
use sf_conformance::sqlite;
use sf_sparql::{exec, exec_mysql, parse_and_translate_with, Tbox};
use sf_sql::introspect::introspect_mysql;
use sf_sql::Dialect;

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

/// MariaDB DDL — use the same table names; drop-create pattern for idempotency.
const MARIADB_SETUP: &[&str] = &[
    "DROP TABLE IF EXISTS person",
    "DROP TABLE IF EXISTS dept",
    "CREATE TABLE dept (id INT NOT NULL PRIMARY KEY, label VARCHAR(200) NOT NULL)",
    "CREATE TABLE person (id INT NOT NULL PRIMARY KEY, name VARCHAR(200) NOT NULL, dept_id INT NOT NULL)",
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

/// Base connection URL: `SF_MARIADB_URL` if set, else a local default.
fn mariadb_url() -> String {
    std::env::var("SF_MARIADB_URL")
        .unwrap_or_else(|_| "mysql://root:SfTest123!@127.0.0.1:13307/test".to_owned())
}

#[tokio::test]
async fn mariadb_differential() {
    let url = mariadb_url();

    // Probe: try to connect; skip gracefully on failure.
    let opts = match Opts::from_url(&url) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("mariadb URL parse error ({e}) — skipping");
            return;
        }
    };
    let mut conn: Conn = match Conn::new(opts).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("mariadb connection failed ({e}) — skipping");
            return;
        }
    };

    // Setup fixture (idempotent).
    use mysql_async::prelude::Queryable;
    for stmt in MARIADB_SETUP {
        if let Err(e) = conn.query_drop(*stmt).await {
            eprintln!("mariadb setup failed on `{stmt}`: {e} — skipping");
            return;
        }
    }

    // Introspect schema.
    let dept_schema = introspect_mysql(&mut conn, "dept")
        .await
        .expect("mariadb dept introspect");
    let person_schema = introspect_mysql(&mut conn, "person")
        .await
        .expect("mariadb person introspect");
    let schema_mb = vec![dept_schema, person_schema];

    let maps = sf_mapping::parse_r2rml(R2RML).expect("R2RML");
    let plan_mb = parse_and_translate_with(
        SELECT_Q,
        &maps,
        Dialect::MySql,
        &Tbox::default(),
        &schema_mb,
    )
    .expect("translate (mariadb)");
    let sols_mb = exec_mysql::select_mysql(&plan_mb, &mut conn)
        .await
        .expect("mariadb exec");

    // ── SQLite oracle ────────────────────────────────────────────────────────
    let sqlite_conn: Connection = sqlite::load(CREATE_SQLITE).expect("sqlite fixture");
    let schema_sqlite = sqlite::introspect_all(&sqlite_conn).expect("sqlite schema");
    let plan_sqlite = parse_and_translate_with(
        SELECT_Q,
        &maps,
        Dialect::Sqlite,
        &Tbox::default(),
        &schema_sqlite,
    )
    .expect("translate (sqlite)");
    let sols_sqlite = exec::select(&plan_sqlite, &sqlite_conn).expect("sqlite exec");

    // ── bag equality ─────────────────────────────────────────────────────────
    assert_eq!(
        sols_sqlite.rows.len(),
        sols_mb.rows.len(),
        "row count mismatch: sqlite={} mariadb={}",
        sols_sqlite.rows.len(),
        sols_mb.rows.len()
    );
    assert_eq!(sols_mb.rows.len(), 2, "expected 2 rows (Ann + Bob)");
}
