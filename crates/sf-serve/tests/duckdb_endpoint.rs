#![cfg(feature = "duckdb-backend")]

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use sf_serve::{router, Backend, ServeConfig};
use sf_sparql::Tbox;
use tower::ServiceExt;

const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .

<#People> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "People" ] ;
  rr:subjectMap [ rr:template "http://ex/person/{id}" ; rr:class ex:Person ] ;
  rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] .

<#Slow> a rr:TriplesMap ;
  rr:logicalTable [
    rr:sqlQuery "SELECT sum(sin(i::DOUBLE)) AS value FROM range(1000000000) values_(i)"
  ] ;
  rr:subjectMap [ rr:template "http://ex/slow/{value}" ] ;
  rr:predicateObjectMap [ rr:predicate ex:slowValue ; rr:objectMap [ rr:column "value" ] ] .

<#Large> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "Large" ] ;
  rr:subjectMap [ rr:template "http://ex/large/{id}" ] ;
  rr:predicateObjectMap [ rr:predicate ex:largeName ; rr:objectMap [ rr:column "name" ] ] .

<#Error> a rr:TriplesMap ;
  rr:logicalTable [
    rr:sqlQuery "SELECT error('SENSITIVE_DUCKDB_DETAIL') AS value"
  ] ;
  rr:subjectMap [ rr:template "http://ex/error/{value}" ] ;
  rr:predicateObjectMap [ rr:predicate ex:errorValue ; rr:objectMap [ rr:column "value" ] ] .
"#;

fn duckdb_config(timeout: Duration) -> (tempfile::TempDir, Arc<ServeConfig>) {
    let source = tempfile::tempdir().expect("create DuckDB fixture directory");
    let path = source.path().join("source.duckdb");
    {
        let conn = duckdb::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE People(id INTEGER PRIMARY KEY, name VARCHAR); \
             INSERT INTO People VALUES (1, 'Alice'), (2, 'Bob'); \
             CREATE TABLE Large(id BIGINT PRIMARY KEY, name VARCHAR); \
             INSERT INTO Large SELECT i, repeat('x', 20000) FROM range(100) values_(i);",
        )
        .unwrap();
    }
    let (backend, schema) = Backend::duckdb_pool_from_path(path.to_str().unwrap(), 1)
        .expect("open restricted file-backed DuckDB source");
    let maps = sf_mapping::parse_r2rml(MAPPING).unwrap();
    let mut config = ServeConfig::new(backend, maps, Tbox::default(), schema);
    config.timeout = timeout;
    (source, Arc::new(config))
}

fn post_query(query: &str, accept: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, accept)
        .body(Body::from(query.to_owned()))
        .unwrap()
}

async fn send(
    config: Arc<ServeConfig>,
    query: &str,
    accept: &str,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let response = router(config)
        .oneshot(post_query(query, accept))
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn wait_for_fast_query(config: Arc<ServeConfig>) -> String {
    tokio::time::timeout(Duration::from_secs(5), async move {
        loop {
            let (status, _headers, body) = send(
                Arc::clone(&config),
                "SELECT ?name WHERE { ?s <http://ex/name> ?name }",
                "application/sparql-results+json",
            )
            .await;
            if status == StatusCode::SERVICE_UNAVAILABLE {
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
            assert_eq!(status, StatusCode::OK, "{body}");
            return body;
        }
    })
    .await
    .expect("interrupted DuckDB request must release the size-one pool")
}

#[tokio::test]
async fn endpoint_supports_select_ask_and_construct_over_duckdb() {
    let (_source, config) = duckdb_config(Duration::from_secs(5));
    let (status, _headers, body) = send(
        Arc::clone(&config),
        "SELECT ?name WHERE { ?s <http://ex/name> ?name . FILTER(?name = \"Alice\") }",
        "application/sparql-results+json",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let bindings = json["results"]["bindings"].as_array().unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0]["name"]["value"], "Alice");

    let (status, _headers, body) = send(
        Arc::clone(&config),
        "ASK { ?s <http://ex/name> \"Bob\" }",
        "application/sparql-results+json",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["boolean"],
        true
    );

    let (status, headers, body) = send(
        config,
        "CONSTRUCT { ?s <http://ex/name> ?name } WHERE { ?s <http://ex/name> ?name }",
        "application/n-triples",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "application/n-triples; charset=utf-8"
    );
    assert!(body.contains("Alice") && body.contains("Bob"), "{body}");
}

#[tokio::test]
async fn exhausted_duckdb_pool_is_shed_with_retry_after() {
    let (_source, config) = duckdb_config(Duration::from_secs(5));
    let first = router(Arc::clone(&config))
        .oneshot(post_query(
            "SELECT ?name WHERE { ?s <http://ex/largeName> ?name }",
            "application/sparql-results+json",
        ))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let second = router(config)
        .oneshot(post_query(
            "SELECT ?name WHERE { ?s <http://ex/name> ?name }",
            "application/sparql-results+json",
        ))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(second.headers()[header::RETRY_AFTER], "1");
    drop(first);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pre_first_row_timeout_interrupts_and_releases_duckdb() {
    let (_source, config) = duckdb_config(Duration::from_millis(20));
    let response = router(Arc::clone(&config))
        .oneshot(post_query(
            "SELECT ?value WHERE { ?s <http://ex/slowValue> ?value }",
            "application/sparql-results+json",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.into_body().collect().await.is_err(),
        "the body must report the stream deadline"
    );

    let body = wait_for_fast_query(config).await;
    assert!(body.contains("Alice") && body.contains("Bob"), "{body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_response_body_interrupts_and_releases_duckdb() {
    // Debug builds spend noticeable time in IRI-template percent encoding before
    // the shared 16 KiB serializer emits its first frame, so keep this deadline
    // comfortably above that work; this test targets client-drop, not timeout.
    let (_source, config) = duckdb_config(Duration::from_secs(30));
    let response = router(Arc::clone(&config))
        .oneshot(post_query(
            "SELECT ?name WHERE { ?s <http://ex/largeName> ?name }",
            "application/sparql-results+json",
        ))
        .await
        .unwrap();
    let mut body = response.into_body();
    let frame = tokio::time::timeout(Duration::from_secs(10), body.frame())
        .await
        .expect("large DuckDB query should produce a response frame")
        .expect("stream should have a frame")
        .expect("first frame should be successful");
    assert!(frame.data_ref().is_some());
    drop(body);

    let follow_up = wait_for_fast_query(config).await;
    assert!(follow_up.contains("Alice") && follow_up.contains("Bob"));
}

#[tokio::test]
async fn duckdb_execution_errors_are_redacted() {
    let (_source, config) = duckdb_config(Duration::from_secs(5));
    let (status, _headers, body) = send(
        config,
        "ASK { ?s <http://ex/errorValue> ?value }",
        "application/sparql-results+json",
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(body, "query execution failed");
    assert!(!body.contains("SENSITIVE_DUCKDB_DETAIL"));
    assert!(!body.contains("SELECT"));
    assert!(!body.contains("source.duckdb"));
}
