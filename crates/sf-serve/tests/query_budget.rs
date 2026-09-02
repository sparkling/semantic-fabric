//! End-to-end receipts for request-wide query-budget policy.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use sf_core::query_control::QueryLimits;
use sf_serve::{introspect_sqlite_all, router, Backend, ServeConfig};
use sf_sparql::Tbox;
use tower::ServiceExt;

const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#Items> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "items" ] ;
  rr:subjectMap [ rr:template "http://example.test/item/{id}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:value ;
    rr:objectMap [ rr:column "value" ]
  ] .
"#;

fn config(limits: QueryLimits) -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().expect("open fixture");
    conn.execute_batch(
        "CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT NOT NULL); \
         INSERT INTO items VALUES (1, 'one'), (2, 'two');",
    )
    .expect("seed fixture");
    let schema = introspect_sqlite_all(&conn).expect("introspect fixture");
    let mapping = sf_mapping::parse_r2rml(MAPPING).expect("parse mapping");
    let mut config =
        ServeConfig::new_unchecked(Backend::sqlite(conn), mapping, Tbox::default(), schema);
    config.query_limits = limits;
    config
}

fn request(query: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, "application/sparql-results+json")
        .body(Body::from(query.to_owned()))
        .expect("static request")
}

async fn route(limits: QueryLimits, query: &str) -> axum::response::Response {
    router(Arc::new(config(limits)))
        .oneshot(request(query))
        .await
        .expect("route request")
}

async fn assert_budget_problem(response: axum::response::Response) {
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/problem+json"
    );
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect problem")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).expect("problem JSON");
    assert_eq!(json["code"], "query-budget-exceeded");
    assert_eq!(json["status"], 429);
}

#[tokio::test]
async fn ask_result_limit_is_a_pre_response_429() {
    let response = route(
        QueryLimits::new(100, 0, u64::MAX),
        "ASK { ?item <http://example.test/value> ?value }",
    )
    .await;
    assert_budget_problem(response).await;
}

#[tokio::test]
async fn ask_serialized_byte_limit_is_a_pre_response_429() {
    let response = route(
        QueryLimits::new(100, 1, 0),
        "ASK { ?item <http://example.test/value> ?value }",
    )
    .await;
    assert_budget_problem(response).await;
}

async fn assert_post_handoff_failure(limits: QueryLimits) {
    let response = route(
        limits,
        "SELECT ?value WHERE { ?item <http://example.test/value> ?value }",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let error = response
        .into_body()
        .collect()
        .await
        .expect_err("stream must terminate at its budget");
    assert_eq!(error.to_string(), "result stream failed");
}

#[tokio::test]
async fn select_source_limit_is_deterministically_post_handoff() {
    for _ in 0..8 {
        assert_post_handoff_failure(QueryLimits::new(0, u64::MAX, u64::MAX)).await;
    }
}

#[tokio::test]
async fn select_result_limit_is_a_redacted_post_handoff_failure() {
    assert_post_handoff_failure(QueryLimits::new(100, 1, u64::MAX)).await;
}

#[tokio::test]
async fn serializer_header_counts_toward_the_post_handoff_byte_limit() {
    assert_post_handoff_failure(QueryLimits::new(100, 2, 0)).await;
}
