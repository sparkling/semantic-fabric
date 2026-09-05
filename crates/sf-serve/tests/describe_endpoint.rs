//! Exact endpoint evidence for the bounded one-target DESCRIBE profile.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use oxttl::TurtleParser;
use sf_serve::{introspect_sqlite_all, router, Backend, ServeConfig, SqlitePool};
use sf_sparql::Tbox;
use tower::ServiceExt;

const CREATE_SQL: &str = r#"
CREATE TABLE "People" ("id" INTEGER PRIMARY KEY, "name" TEXT, "age" INTEGER);
INSERT INTO "People" VALUES (1, 'Alice', 30), (2, 'Bob', 25);
"#;

const MAPPING_TTL: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#People> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "People" ] ;
  rr:subjectMap [ rr:template "http://ex/person/{id}" ; rr:class ex:Person ] ;
  rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] ;
  rr:predicateObjectMap [ rr:predicate ex:age ; rr:objectMap [ rr:column "age" ] ] .
"#;

const SINGLE_POM_MAPPING_TTL: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#People> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "People" ] ;
  rr:subjectMap [ rr:template "http://ex/person/{id}" ] ;
  rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] .
"#;

const BLANK_OBJECT_MAPPING_TTL: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#People> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "People" ] ;
  rr:subjectMap [ rr:template "http://ex/person/{id}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:address ;
    rr:objectMap [ rr:template "address-{id}" ; rr:termType rr:BlankNode ]
  ] .
<#Addresses> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "People" ] ;
  rr:subjectMap [ rr:template "address-{id}" ; rr:termType rr:BlankNode ] ;
  rr:predicateObjectMap [ rr:predicate ex:city ; rr:objectMap [ rr:constant "London" ] ] .
"#;

fn config_and_pool(mapping: &str) -> (ServeConfig, SqlitePool) {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(CREATE_SQL).unwrap();
    let schema = introspect_sqlite_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(mapping).unwrap();
    let backend = Backend::sqlite(conn);
    let Backend::Sqlite(pool) = &backend else {
        unreachable!()
    };
    let pool = pool.clone();
    (
        ServeConfig::new_unchecked(backend, maps, Tbox::default(), schema),
        pool,
    )
}

fn request(query: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, "text/turtle")
        .body(Body::from(query.to_owned()))
        .unwrap()
}

async fn send(config: ServeConfig, query: &str) -> (StatusCode, Vec<oxrdf::Triple>) {
    let response = router(Arc::new(config))
        .oneshot(request(query))
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let triples = if status == StatusCode::OK {
        TurtleParser::new()
            .for_slice(&body)
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    } else {
        Vec::new()
    };
    (status, triples)
}

fn predicate_iris(triples: &[oxrdf::Triple]) -> BTreeSet<&str> {
    triples
        .iter()
        .map(|triple| triple.predicate.as_str())
        .collect()
}

fn poison_source(pool: &SqlitePool) {
    pool.pick()
        .lock()
        .unwrap()
        .execute_batch("DROP TABLE \"People\"")
        .unwrap();
}

#[test]
fn flat_oracle_keeps_repeated_target_results_graph_exact() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(CREATE_SQL).unwrap();
    let schema = introspect_sqlite_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(SINGLE_POM_MAPPING_TTL).unwrap();
    let plan = sf_sparql::parse_and_translate_flat_with(
        "DESCRIBE ?s WHERE { VALUES ?s { <http://ex/person/1> <http://ex/person/1> } }",
        &maps,
        sf_sql::Dialect::Sqlite,
        &Tbox::default(),
        &schema,
    )
    .unwrap();

    let triples = sf_sparql::exec::construct_triples(&plan, &conn).unwrap();
    assert_eq!(triples.len(), 1);
    assert_eq!(predicate_iris(&triples), BTreeSet::from(["http://ex/name"]));
}

#[tokio::test]
async fn one_iri_returns_its_exact_one_hop_outgoing_description() {
    let (config, _) = config_and_pool(MAPPING_TTL);
    let (status, triples) = send(config, "DESCRIBE <http://ex/person/1>").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(triples.len(), 3);
    assert!(triples
        .iter()
        .all(|triple| triple.subject.to_string() == "<http://ex/person/1>"));
    assert_eq!(
        predicate_iris(&triples),
        BTreeSet::from([
            "http://ex/age",
            "http://ex/name",
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
        ])
    );
}

#[tokio::test]
async fn legal_user_variable_names_cannot_capture_description_internals() {
    let (config, _) = config_and_pool(SINGLE_POM_MAPPING_TTL);
    let query = r#"DESCRIBE ?s WHERE {
        VALUES (?s ?__sf_describe_predicate_0 ?__sf_describe_object_0) {
            (<http://ex/person/1> <urn:sentinel:predicate> "sentinel")
        }
    }"#;
    let (status, triples) = send(config, query).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        triples.len(),
        1,
        "legal user bindings must not capture the description internals"
    );
    assert_eq!(predicate_iris(&triples), BTreeSet::from(["http://ex/name"]));
    assert_eq!(triples[0].object.to_string(), "\"Alice\"");
}

#[tokio::test]
async fn mapped_where_binding_is_an_admitted_variable_target() {
    let (config, _) = config_and_pool(SINGLE_POM_MAPPING_TTL);
    let (status, triples) = send(
        config,
        "DESCRIBE ?s WHERE { ?s <http://ex/name> \"Alice\" }",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(triples.len(), 1);
    assert_eq!(triples[0].subject.to_string(), "<http://ex/person/1>");
    assert_eq!(predicate_iris(&triples), BTreeSet::from(["http://ex/name"]));
}

#[tokio::test]
async fn one_effective_wildcard_target_is_admitted_but_multiple_are_not() {
    let (config, _) = config_and_pool(SINGLE_POM_MAPPING_TTL);
    let (status, triples) =
        send(config, "DESCRIBE * WHERE { ?s <http://ex/name> \"Alice\" }").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(triples.len(), 1);

    let (config, pool) = config_and_pool(SINGLE_POM_MAPPING_TTL);
    poison_source(&pool);
    let (status, triples) = send(config, "DESCRIBE * WHERE { ?s ?p ?o }").await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert!(triples.is_empty());
}

#[tokio::test]
async fn repeated_target_solutions_still_emit_a_graph_set() {
    let (config, _) = config_and_pool(SINGLE_POM_MAPPING_TTL);
    let (status, triples) = send(
        config,
        "DESCRIBE ?s WHERE { VALUES ?s { <http://ex/person/1> <http://ex/person/1> } }",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(triples.len(), 1);
    assert_eq!(predicate_iris(&triples), BTreeSet::from(["http://ex/name"]));
}

#[tokio::test]
async fn target_repeated_across_union_arms_still_emits_one_graph() {
    let (config, _) = config_and_pool(SINGLE_POM_MAPPING_TTL);
    let query = r#"DESCRIBE ?s WHERE {
        { VALUES ?s { <http://ex/person/1> } }
        UNION
        { VALUES ?s { <http://ex/person/1> } }
    }"#;
    let (status, triples) = send(config, query).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(triples.len(), 1);
    assert_eq!(predicate_iris(&triples), BTreeSet::from(["http://ex/name"]));
}

#[tokio::test]
async fn constant_target_with_one_where_solution_keeps_disjoint_outgoing_branches() {
    let (config, _) = config_and_pool(MAPPING_TTL);
    let (status, triples) = send(
        config,
        "DESCRIBE <http://ex/person/1> WHERE { VALUES ?unused { 1 } }",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(triples.len(), 3);
    assert_eq!(
        predicate_iris(&triples),
        BTreeSet::from([
            "http://ex/age",
            "http://ex/name",
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
        ])
    );
}

#[tokio::test]
async fn repeated_constant_target_selection_rejects_before_source_io() {
    let (config, pool) = config_and_pool(SINGLE_POM_MAPPING_TTL);
    poison_source(&pool);
    let (status, triples) = send(
        config,
        "DESCRIBE <http://ex/person/1> WHERE { VALUES ?unused { 1 2 } }",
    )
    .await;

    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert!(triples.is_empty());
}

#[tokio::test]
async fn blank_node_object_is_emitted_without_recursive_description() {
    let (config, _) = config_and_pool(BLANK_OBJECT_MAPPING_TTL);
    let (status, triples) = send(config, "DESCRIBE <http://ex/person/1>").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(triples.len(), 1);
    assert_eq!(
        predicate_iris(&triples),
        BTreeSet::from(["http://ex/address"])
    );
    assert!(triples[0].object.is_blank_node());
}

#[tokio::test]
async fn unbound_target_rejects_before_source_io() {
    let (config, pool) = config_and_pool(SINGLE_POM_MAPPING_TTL);
    poison_source(&pool);
    let (status, triples) = send(
        config,
        "DESCRIBE ?s WHERE { ?x <http://ex/name> \"Alice\" }",
    )
    .await;

    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert!(triples.is_empty());
}

#[tokio::test]
async fn unqualified_modifier_shapes_reject_before_source_io() {
    for query in [
        "DESCRIBE ?s WHERE { ?s <http://ex/name> \"Alice\" } LIMIT 1",
        "DESCRIBE ?s WHERE { ?s <http://ex/name> \"Alice\" } OFFSET 1",
        "DESCRIBE ?s WHERE { ?s <http://ex/name> \"Alice\" } ORDER BY ?s",
        "DESCRIBE ?s WHERE { ?s <http://ex/name> \"Alice\" } ORDER BY ?s LIMIT 1",
    ] {
        let (config, pool) = config_and_pool(SINGLE_POM_MAPPING_TTL);
        poison_source(&pool);
        let (status, triples) = send(config, query).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "query: {query}");
        assert!(triples.is_empty(), "query: {query}");
    }
}

#[tokio::test]
async fn multiple_targets_reject_before_source_io_until_bounded_union_exists() {
    let (config, pool) = config_and_pool(MAPPING_TTL);
    poison_source(&pool);
    let (status, triples) =
        send(config, "DESCRIBE <http://ex/person/1> <http://ex/person/2>").await;

    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert!(triples.is_empty());
}
