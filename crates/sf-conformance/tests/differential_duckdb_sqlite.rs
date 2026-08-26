//! Always-on cross-backend differential for the admitted embedded engines.

use std::sync::{Arc, Mutex};

use sf_conformance::oracle::{engine_bag, solutions_bag_eq};
use sf_conformance::sqlite;
use sf_sparql::{exec, exec_duckdb, parse_and_translate_with, Tbox};
use sf_sql::{Dialect, TableSchema};

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
