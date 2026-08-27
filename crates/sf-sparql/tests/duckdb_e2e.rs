#![cfg(feature = "duckdb-backend")]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use sf_sparql::exec_duckdb::{ask_duckdb, construct_triples_duckdb, select_duckdb};
use sf_sparql::{parse_and_translate_with, Tbox};
use sf_sql::Dialect;

const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

<#People> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "People" ] ;
  rr:subjectMap [ rr:template "http://ex/person/{slug}" ; rr:class ex:Person ] ;
  rr:predicateObjectMap [
    rr:predicate ex:name ;
    rr:objectMap [ rr:column "name" ]
  ] ;
  rr:predicateObjectMap [
    rr:predicate ex:age ;
    rr:objectMap [ rr:column "age" ; rr:datatype xsd:integer ]
  ] .

"#;

const EDGES_MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .

<#Edges> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "Edges" ] ;
  rr:subjectMap [ rr:template "http://ex/node/{src}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:next ;
    rr:objectMap [ rr:template "http://ex/node/{dst}" ]
  ] .
"#;

#[tokio::test]
async fn public_duckdb_executor_supports_bound_select_ask_and_construct() {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE People(slug VARCHAR PRIMARY KEY, name VARCHAR, age INTEGER); \
         INSERT INTO People VALUES ('a b/%', 'Alice', 30), ('bob', 'Bob', 25);",
    )
    .unwrap();
    let schema = vec![sf_sql::introspect::introspect_duckdb(&conn, "People").unwrap()];
    let conn = Arc::new(Mutex::new(conn));
    let maps = sf_mapping::parse_r2rml(MAPPING).unwrap();

    let select = parse_and_translate_with(
        "SELECT ?s ?name WHERE { \
           ?s <http://ex/name> ?name ; <http://ex/age> ?age . \
           FILTER(?name = \"Alice\" && ?age = 30) \
         }",
        &maps,
        Dialect::DuckDb,
        &Tbox::default(),
        &schema,
    )
    .unwrap();
    let emitted = select.emitted().unwrap();
    assert_eq!(emitted.len(), 1);
    assert_eq!(emitted[0].sql.matches('?').count(), 2);
    assert!(!emitted[0].sql.contains("Alice"));
    assert_eq!(emitted[0].params.len(), 2);

    let solutions = select_duckdb(&select, Arc::clone(&conn)).await.unwrap();
    assert_eq!(solutions.rows.len(), 1);
    let row: Vec<String> = solutions.rows[0]
        .iter()
        .map(|term| term.as_ref().unwrap().to_string())
        .collect();
    assert_eq!(row, ["<http://ex/person/a%20b%2F%25>", "\"Alice\""]);

    let ask_true = parse_and_translate_with(
        "ASK { ?s <http://ex/name> \"Bob\" }",
        &maps,
        Dialect::DuckDb,
        &Tbox::default(),
        &schema,
    )
    .unwrap();
    assert!(ask_duckdb(&ask_true, Arc::clone(&conn)).await.unwrap());

    let ask_false = parse_and_translate_with(
        "ASK { ?s <http://ex/name> \"Carol\" }",
        &maps,
        Dialect::DuckDb,
        &Tbox::default(),
        &schema,
    )
    .unwrap();
    assert!(!ask_duckdb(&ask_false, Arc::clone(&conn)).await.unwrap());

    let construct = parse_and_translate_with(
        "CONSTRUCT { ?s <http://ex/name> ?name } \
         WHERE { ?s <http://ex/name> ?name }",
        &maps,
        Dialect::DuckDb,
        &Tbox::default(),
        &schema,
    )
    .unwrap();
    let triples: BTreeSet<String> = construct_triples_duckdb(&construct, conn)
        .await
        .unwrap()
        .into_iter()
        .map(|triple| triple.to_string())
        .collect();
    assert_eq!(
        triples,
        BTreeSet::from([
            "<http://ex/person/a%20b%2F%25> <http://ex/name> \"Alice\"".to_owned(),
            "<http://ex/person/bob> <http://ex/name> \"Bob\"".to_owned(),
        ])
    );
}

#[tokio::test]
async fn duckdb_cycle_detecting_recursion_terminates_and_deduplicates_pairs() {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE Edges(src INTEGER, dst INTEGER); \
         INSERT INTO Edges VALUES (1, 2), (1, 2), (2, 3), (3, 1), (3, 4);",
    )
    .unwrap();
    let schema = vec![sf_sql::introspect::introspect_duckdb(&conn, "Edges").unwrap()];
    let conn = Arc::new(Mutex::new(conn));
    let maps = sf_mapping::parse_r2rml(EDGES_MAPPING).unwrap();

    let plus = parse_and_translate_with(
        "SELECT ?start ?end WHERE { ?start <http://ex/next>+ ?end }",
        &maps,
        Dialect::DuckDb,
        &Tbox::default(),
        &schema,
    )
    .unwrap();
    let sql = plus.emitted().unwrap()[0].sql.clone();
    assert!(sql.contains("list_contains"), "{sql}");
    assert!(sql.contains("list_append"), "{sql}");
    assert!(sql.contains("sf_d") && sql.contains("< 256"), "{sql}");
    let plus_rows = select_duckdb(&plus, Arc::clone(&conn)).await.unwrap();
    let plus_pairs: BTreeSet<(String, String)> = plus_rows
        .rows
        .into_iter()
        .map(|row| {
            (
                row[0].as_ref().unwrap().to_string(),
                row[1].as_ref().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(plus_pairs.len(), 12, "{plus_pairs:#?}");

    let star = parse_and_translate_with(
        "SELECT ?start ?end WHERE { ?start <http://ex/next>* ?end }",
        &maps,
        Dialect::DuckDb,
        &Tbox::default(),
        &schema,
    )
    .unwrap();
    let star_rows = select_duckdb(&star, conn).await.unwrap();
    let star_pairs: BTreeSet<(String, String)> = star_rows
        .rows
        .into_iter()
        .map(|row| {
            (
                row[0].as_ref().unwrap().to_string(),
                row[1].as_ref().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(star_pairs.len(), 13, "{star_pairs:#?}");
    assert!(star_pairs.contains(&(
        "<http://ex/node/4>".to_owned(),
        "<http://ex/node/4>".to_owned()
    )));
}
