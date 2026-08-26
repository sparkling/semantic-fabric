//! Always-on DuckDB/SQLite differential plus DuckDB/live-PostgreSQL parity.
//!
//! The PostgreSQL arm skips only when no server is reachable; the repository CI
//! supplies a live service through `SF_PG_URL`, so that arm is active there.

use std::sync::{Arc, Mutex};

use sf_conformance::oracle::{engine_bag, solutions_bag_eq};
use sf_conformance::sqlite;
use sf_sparql::{exec, exec_duckdb, exec_pg, parse_and_translate_with, Tbox};
use sf_sql::introspect::introspect_postgres;
use sf_sql::{Dialect, TableSchema};
use tokio_postgres::{Client, NoTls};

const CREATE_SQL: &str = r#"
CREATE TABLE dept (id INTEGER PRIMARY KEY, label VARCHAR NOT NULL);
CREATE TABLE person (
    id INTEGER PRIMARY KEY,
    name VARCHAR NOT NULL,
    dept_id INTEGER NOT NULL,
    email VARCHAR,
    FOREIGN KEY (dept_id) REFERENCES dept(id)
);
INSERT INTO dept VALUES (10, 'Sales'), (20, 'Engineering');
INSERT INTO person VALUES (1, 'Ann', 10, 'ann@x');
INSERT INTO person VALUES (2, 'Bob', 10, NULL);
INSERT INTO person VALUES (3, 'Zed', 20, 'zed@x');
"#;

const R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .

<#Person>
    rr:logicalTable [ rr:tableName "person" ] ;
    rr:subjectMap [ rr:template "http://ex/person/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:email ; rr:objectMap [ rr:column "email" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:deptId ; rr:objectMap [ rr:column "dept_id" ] ] ;
    rr:predicateObjectMap [
        rr:predicate ex:dept ;
        rr:objectMap [
            rr:parentTriplesMap <#Dept> ;
            rr:joinCondition [ rr:child "dept_id" ; rr:parent "id" ]
        ]
    ] .

<#Dept>
    rr:logicalTable [ rr:tableName "dept" ] ;
    rr:subjectMap [ rr:template "http://ex/dept/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:label ; rr:objectMap [ rr:column "label" ] ] .
"#;

const SELECT_QUERIES: &[(&str, &str)] = &[
    (
        "join-filter-optional",
        r#"PREFIX ex: <http://ex/>
           SELECT ?name ?label ?email WHERE {
             ?p ex:name ?name ; ex:dept ?d . ?d ex:label ?label .
             OPTIONAL { ?p ex:email ?email }
             FILTER(?name != "Zed")
           }"#,
    ),
    (
        "typed-filter",
        r#"PREFIX ex: <http://ex/>
           SELECT ?name WHERE { ?p ex:name ?name ; ex:deptId ?id . FILTER(?id = 10) }"#,
    ),
    (
        "aggregate-union-distinct",
        r#"PREFIX ex: <http://ex/>
           SELECT ?label (COUNT(DISTINCT ?v) AS ?count) WHERE {
             ?p ex:dept ?d . ?d ex:label ?label .
             { ?p ex:name ?v } UNION { ?p ex:email ?v }
           } GROUP BY ?label"#,
    ),
    (
        "minus",
        r#"PREFIX ex: <http://ex/>
           SELECT ?name WHERE { ?p ex:name ?name MINUS { ?p ex:email ?email } }"#,
    ),
    (
        "sequence-path",
        r#"PREFIX ex: <http://ex/>
           SELECT ?name ?label WHERE { ?p ex:name ?name ; ex:dept/ex:label ?label }"#,
    ),
    (
        "right-nested-optional",
        r#"PREFIX ex: <http://ex/>
           SELECT DISTINCT ?name ?label WHERE {
             ?p ex:name ?name
             OPTIONAL { ?p ex:dept ?d . ?d ex:label ?label OPTIONAL { ?p ex:email ?email } }
           }"#,
    ),
];

fn duckdb_schema(conn: &duckdb::Connection) -> Vec<TableSchema> {
    ["dept", "person"]
        .into_iter()
        .map(|table| sf_sql::introspect::introspect_duckdb(conn, table).unwrap())
        .collect()
}

fn base_pg_conn() -> String {
    std::env::var("SF_PG_URL").unwrap_or_else(|_| {
        let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_owned());
        format!("host=localhost port=5432 user={user}")
    })
}

async fn connect_pg(conn_str: &str) -> Result<Client, String> {
    let (client, connection) = tokio_postgres::connect(conn_str, NoTls)
        .await
        .map_err(|error| error.to_string())?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok(client)
}

async fn postgres_schema(client: &Client) -> Result<Vec<TableSchema>, String> {
    let rows = client
        .query(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_type = 'BASE TABLE' \
             ORDER BY table_name",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?;
    let mut schemas = Vec::with_capacity(rows.len());
    for row in rows {
        let name: String = row.get(0);
        schemas.push(
            introspect_postgres(client, &name)
                .await
                .map_err(|error| error.to_string())?,
        );
    }
    Ok(schemas)
}

#[tokio::test]
async fn duckdb_and_sqlite_match_across_representative_obda_shapes() {
    let sqlite_conn = sqlite::load(CREATE_SQL).unwrap();
    let sqlite_schema = sqlite::introspect_all(&sqlite_conn).unwrap();
    let duckdb_conn = duckdb::Connection::open_in_memory().unwrap();
    duckdb_conn.execute_batch(CREATE_SQL).unwrap();
    let duck_schema = duckdb_schema(&duckdb_conn);
    let duckdb_conn = Arc::new(Mutex::new(duckdb_conn));
    let maps = sf_mapping::parse_r2rml(R2RML).unwrap();

    for (name, query) in SELECT_QUERIES {
        let sqlite_plan = parse_and_translate_with(
            query,
            &maps,
            Dialect::Sqlite,
            &Tbox::default(),
            &sqlite_schema,
        )
        .unwrap();
        let duckdb_plan = parse_and_translate_with(
            query,
            &maps,
            Dialect::DuckDb,
            &Tbox::default(),
            &duck_schema,
        )
        .unwrap();
        let sqlite_bag = engine_bag(&exec::select(&sqlite_plan, &sqlite_conn).unwrap());
        let duckdb_bag = engine_bag(
            &exec_duckdb::select_duckdb(&duckdb_plan, Arc::clone(&duckdb_conn))
                .await
                .unwrap(),
        );
        assert!(
            solutions_bag_eq(&sqlite_bag, &duckdb_bag),
            "{name} differs: sqlite={sqlite_bag:#?}, duckdb={duckdb_bag:#?}"
        );
    }

    for (query, expected) in [
        ("PREFIX ex: <http://ex/> ASK { ?p ex:name \"Ann\" }", true),
        (
            "PREFIX ex: <http://ex/> ASK { ?p ex:name \"Nobody\" }",
            false,
        ),
    ] {
        let sqlite_plan = parse_and_translate_with(
            query,
            &maps,
            Dialect::Sqlite,
            &Tbox::default(),
            &sqlite_schema,
        )
        .unwrap();
        let duckdb_plan = parse_and_translate_with(
            query,
            &maps,
            Dialect::DuckDb,
            &Tbox::default(),
            &duck_schema,
        )
        .unwrap();
        assert_eq!(exec::ask(&sqlite_plan, &sqlite_conn).unwrap(), expected);
        assert_eq!(
            exec_duckdb::ask_duckdb(&duckdb_plan, Arc::clone(&duckdb_conn))
                .await
                .unwrap(),
            expected
        );
    }
}

#[tokio::test]
async fn duckdb_and_live_postgres_match_when_postgres_is_available() {
    let base = base_pg_conn();
    let admin = match connect_pg(&format!("{base} dbname=postgres")).await {
        Ok(client) => client,
        Err(_) => {
            eprintln!("no PostgreSQL server reachable — skipping DuckDB/PostgreSQL differential");
            return;
        }
    };
    let dbname = format!("sf_duck_diff_{}", std::process::id());
    admin
        .batch_execute(&format!("DROP DATABASE IF EXISTS {dbname} WITH (FORCE)"))
        .await
        .expect("drop pre-existing throwaway PostgreSQL database");
    admin
        .batch_execute(&format!("CREATE DATABASE {dbname}"))
        .await
        .expect("create throwaway PostgreSQL database");

    let pg_client = Arc::new(
        connect_pg(&format!("{base} dbname={dbname}"))
            .await
            .expect("connect throwaway PostgreSQL database"),
    );
    pg_client
        .batch_execute(CREATE_SQL)
        .await
        .expect("seed PostgreSQL fixture");
    let pg_schema = postgres_schema(&pg_client)
        .await
        .expect("introspect PostgreSQL fixture");

    let duckdb_conn = duckdb::Connection::open_in_memory().unwrap();
    duckdb_conn.execute_batch(CREATE_SQL).unwrap();
    let duck_schema = duckdb_schema(&duckdb_conn);
    let duckdb_conn = Arc::new(Mutex::new(duckdb_conn));
    let maps = sf_mapping::parse_r2rml(R2RML).unwrap();

    for (name, query) in SELECT_QUERIES {
        let duckdb_plan = parse_and_translate_with(
            query,
            &maps,
            Dialect::DuckDb,
            &Tbox::default(),
            &duck_schema,
        )
        .unwrap();
        let pg_plan = parse_and_translate_with(
            query,
            &maps,
            Dialect::Postgres,
            &Tbox::default(),
            &pg_schema,
        )
        .unwrap();
        let duckdb_bag = engine_bag(
            &exec_duckdb::select_duckdb(&duckdb_plan, Arc::clone(&duckdb_conn))
                .await
                .unwrap(),
        );
        let pg_bag = engine_bag(&exec_pg::select_pg(&pg_plan, &pg_client).await.unwrap());
        assert!(
            solutions_bag_eq(&duckdb_bag, &pg_bag),
            "{name} differs: duckdb={duckdb_bag:#?}, postgres={pg_bag:#?}"
        );
    }

    for (query, expected) in [
        ("PREFIX ex: <http://ex/> ASK { ?p ex:name \"Ann\" }", true),
        (
            "PREFIX ex: <http://ex/> ASK { ?p ex:name \"Nobody\" }",
            false,
        ),
    ] {
        let duckdb_plan = parse_and_translate_with(
            query,
            &maps,
            Dialect::DuckDb,
            &Tbox::default(),
            &duck_schema,
        )
        .unwrap();
        let pg_plan = parse_and_translate_with(
            query,
            &maps,
            Dialect::Postgres,
            &Tbox::default(),
            &pg_schema,
        )
        .unwrap();
        assert_eq!(
            exec_duckdb::ask_duckdb(&duckdb_plan, Arc::clone(&duckdb_conn))
                .await
                .unwrap(),
            expected
        );
        assert_eq!(
            exec_pg::ask_pg(&pg_plan, Arc::clone(&pg_client))
                .await
                .unwrap(),
            expected
        );
    }

    drop(pg_client);
    admin
        .batch_execute(&format!("DROP DATABASE IF EXISTS {dbname} WITH (FORCE)"))
        .await
        .expect("drop throwaway PostgreSQL database");
}
