//! Protocol request-shape tests: rejected queries, methods, and bodies.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};

use crate::fixtures::{post_query, send, sqlite_config};

#[tokio::test]
async fn deferred_feature_returns_501() {
    let cfg = Arc::new(sqlite_config());
    let req = post_query(
        "SELECT * WHERE { SERVICE <http://example.org/sparql> { ?s ?p ?o } }",
        "application/sparql-results+json",
    );
    let (status, _ctype, _body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn malformed_query_returns_400() {
    let cfg = Arc::new(sqlite_config());
    let req = post_query(
        "SELECT ?x WHERE { this is not sparql",
        "application/sparql-results+json",
    );
    let (status, _ctype, _body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn missing_query_param_returns_400() {
    let cfg = Arc::new(sqlite_config());
    let req = Request::builder()
        .method("GET")
        // A truly query-less GET is W3C Service Description discovery. A query
        // string which omits the required `query` field remains invalid.
        .uri("/sparql?default-graph-uri=https%3A%2F%2Fexample.test%2Fgraph")
        .body(Body::empty())
        .unwrap();
    let (status, _ctype, _body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn post_form_urlencoded_body_is_accepted() {
    let cfg = Arc::new(sqlite_config());
    let query = "PREFIX ex: <http://ex/> SELECT ?n WHERE { ?p ex:name ?n }";
    let encoded = form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>();
    let req = Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::ACCEPT, "application/sparql-results+json")
        .body(Body::from(format!("query={encoded}")))
        .unwrap();
    let (status, _, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Alice") || body.contains("Bob"));
}

#[tokio::test]
async fn post_unsupported_content_type_returns_415() {
    let cfg = Arc::new(sqlite_config());
    let req = Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from("SELECT * WHERE { ?s ?p ?o }"))
        .unwrap();
    let (status, ctype, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(ctype, "application/problem+json");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["code"],
        "unsupported-media-type"
    );
}

#[tokio::test]
async fn post_sparql_query_body_non_utf8_returns_400() {
    let cfg = Arc::new(sqlite_config());
    let req = Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .body(Body::from(vec![0xff, 0xfe, 0x00, 0x01])) // invalid UTF-8
        .unwrap();
    let (status, ctype, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(ctype, "application/problem+json");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["code"],
        "invalid-request"
    );
}
