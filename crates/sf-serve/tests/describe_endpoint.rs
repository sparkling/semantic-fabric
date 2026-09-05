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

fn config_and_pool() -> (ServeConfig, SqlitePool) {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(CREATE_SQL).unwrap();
    let schema = introspect_sqlite_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(MAPPING_TTL).unwrap();
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

#[tokio::test]
async fn one_iri_returns_its_exact_one_hop_outgoing_description() {
    let (config, _) = config_and_pool();
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
    let (config, _) = config_and_pool();
    let query = r#"DESCRIBE ?__sf_describe_object_0 WHERE {
        VALUES ?__sf_describe_object_0 { <http://ex/person/1> }
    }"#;
    let (status, triples) = send(config, query).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        triples.len(),
        3,
        "the user binding must not filter outgoing triples"
    );
    assert_eq!(predicate_iris(&triples).len(), 3);
}

#[tokio::test]
async fn multiple_targets_reject_before_source_io_until_bounded_union_exists() {
    let (config, pool) = config_and_pool();
    pool.pick()
        .lock()
        .unwrap()
        .execute_batch("DROP TABLE \"People\"")
        .unwrap();
    let (status, triples) =
        send(config, "DESCRIBE <http://ex/person/1> <http://ex/person/2>").await;

    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert!(triples.is_empty());
}
