//! Serve-lane admission tests for ADR-0038 M1 blocking-operator containment.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use sf_core::query_control::QueryLimits;
use sf_serve::{router, Backend, ServeConfig, SqlitePool};
use tower::ServiceExt;

mod support;

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
    let backend = Backend::sqlite(conn);
    let Backend::Sqlite(pool) = &backend else {
        unreachable!("fixture is SQLite")
    };
    let pool = pool.clone();
    (support::serve_config(backend, MAPPING_TTL), pool)
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
async fn ordered_ask_is_admitted_and_respects_the_offset_boundary() {
    let cfg = Arc::new(config());
    for (offset, expected) in [(1, true), (2, false)] {
        let query = format!(
            "ASK WHERE {{ ?item <http://example.test/label> ?label }} \
             ORDER BY DESC(?label) OFFSET {offset} LIMIT 1"
        );
        let response = router(cfg.clone())
            .oneshot(query_request(&query))
            .await
            .expect("route request");

        assert_eq!(response.status(), StatusCode::OK, "offset={offset}");
        let body = response
            .into_body()
            .collect()
            .await
            .expect("collect successful response")
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).expect("result JSON");
        assert_eq!(json["boolean"], expected, "offset={offset}");
    }
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

#[tokio::test]
async fn finite_order_window_is_admitted_and_exact() {
    let response = router(Arc::new(config()))
        .oneshot(query_request(
            "SELECT ?label WHERE { ?item <http://example.test/label> ?label } \
             ORDER BY ?label LIMIT 2",
        ))
        .await
        .expect("route request");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let bindings = json["results"]["bindings"].as_array().unwrap();
    assert_eq!(bindings[0]["label"]["value"], "alpha");
    assert_eq!(bindings[1]["label"]["value"], "zeta");
}

#[tokio::test]
async fn zero_order_window_returns_empty_before_source_or_payload_budget_io() {
    let (mut cfg, pool) = config_and_pool();
    cfg.set_max_order_rows(0);
    cfg.query_limits = QueryLimits::new(u64::MAX, 0, 0, u64::MAX).with_max_retained_bytes(0);
    pool.pick()
        .lock()
        .unwrap()
        .execute_batch("DROP TABLE \"items\"")
        .unwrap();

    let response = router(Arc::new(cfg))
        .oneshot(query_request(
            "SELECT ?label WHERE { ?item <http://example.test/label> ?label } \
             ORDER BY ?label OFFSET 18446744073709551615 LIMIT 0",
        ))
        .await
        .expect("route request");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["results"]["bindings"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn order_window_over_result_ceiling_is_501_before_source_io() {
    let (mut cfg, pool) = config_and_pool();
    cfg.set_max_order_rows(1);
    pool.pick()
        .lock()
        .unwrap()
        .execute_batch("DROP TABLE \"items\"")
        .unwrap();

    let response = router(Arc::new(cfg))
        .oneshot(query_request(
            "SELECT ?label WHERE { ?item <http://example.test/label> ?label } \
             ORDER BY ?label LIMIT 2",
        ))
        .await
        .expect("route request");
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn order_expression_is_501_before_source_io() {
    let (cfg, pool) = config_and_pool();
    pool.pick()
        .lock()
        .unwrap()
        .execute_batch("DROP TABLE \"items\"")
        .unwrap();

    let response = router(Arc::new(cfg))
        .oneshot(query_request(
            "SELECT ?label WHERE { ?item <http://example.test/label> ?label } \
             ORDER BY STRLEN(?label) LIMIT 1",
        ))
        .await
        .expect("route request");

    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&body).contains("no such table"));
}
