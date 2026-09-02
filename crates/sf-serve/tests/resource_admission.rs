//! Serve-lane admission tests for ADR-0038 M1 source-sized Rust fallbacks.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use sf_serve::{introspect_sqlite_all, router, Backend, ServeConfig, SqlitePool};
use sf_sparql::Tbox;
use tower::ServiceExt;

const CREATE_SQL: &str = r#"
CREATE TABLE "items" ("id" INTEGER PRIMARY KEY, "label" TEXT NOT NULL);
INSERT INTO "items" VALUES (1, 'zeta'), (2, 'alpha');
"#;

const MAPPING_TTL: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#Items> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "items" ] ;
  rr:subjectMap [ rr:template "http://example.test/item/{id}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:label ;
    rr:objectMap [ rr:column "label" ]
  ] .
"#;

fn config() -> ServeConfig {
    config_and_pool().0
}

fn config_and_pool() -> (ServeConfig, SqlitePool) {
    let conn = rusqlite::Connection::open_in_memory().expect("open fixture");
    conn.execute_batch(CREATE_SQL).expect("seed fixture");
    let schema = introspect_sqlite_all(&conn).expect("introspect fixture");
    let maps = sf_mapping::parse_r2rml(MAPPING_TTL).expect("parse fixture mapping");
    let backend = Backend::sqlite(conn);
    let Backend::Sqlite(pool) = &backend else {
        unreachable!("fixture is SQLite")
    };
    let pool = pool.clone();
    (
        ServeConfig::new_unchecked(backend, maps, Tbox::default(), schema),
        pool,
    )
}

fn query_request(query: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, "application/sparql-results+json")
        .body(Body::from(query.to_owned()))
        .expect("static request")
}

#[tokio::test]
async fn plain_streaming_plan_remains_admitted() {
    let response = router(Arc::new(config()))
        .oneshot(query_request(
            "SELECT ?label WHERE { ?item <http://example.test/label> ?label }",
        ))
        .await
        .expect("route request");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect successful response")
        .to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("alpha"));
}

#[tokio::test]
async fn ordered_ask_is_admitted_without_a_global_sort_buffer() {
    let response = router(Arc::new(config()))
        .oneshot(query_request(
            "ASK WHERE { ?item <http://example.test/label> ?label } \
             ORDER BY DESC(?label) OFFSET 1 LIMIT 1",
        ))
        .await
        .expect("route request");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect successful response")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).expect("result JSON");
    assert_eq!(json["boolean"], true);
}

#[tokio::test]
async fn global_order_is_typed_501_before_a_missing_live_table_is_touched() {
    let (cfg, pool) = config_and_pool();
    pool.pick()
        .lock()
        .expect("lock fixture")
        .execute_batch("DROP TABLE \"items\"")
        .expect("remove live table after schema snapshot");

    let response = router(Arc::new(cfg))
        .oneshot(query_request(
            "SELECT ?label WHERE { ?item <http://example.test/label> ?label } ORDER BY ?label",
        ))
        .await
        .expect("route request");

    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect rejection")
        .to_bytes();
    let body = String::from_utf8(body.to_vec()).expect("UTF-8 rejection");
    let json: serde_json::Value = serde_json::from_str(&body).expect("problem JSON");
    assert_eq!(json["code"], "unsupported-query", "body={body}");
    assert!(!body.contains("global-order"), "body={body}");
    assert!(!body.contains("no such table"), "body={body}");
}
