use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use sf_core::{SourceId, SourceMapping};
use sf_sparql::Epoch;
use tokio_stream::StreamExt;
use tower::ServiceExt;

use crate::{
    introspect_sqlite_all, router, Backend, IntrospectedSource, ReadinessCause, RuntimeSnapshot,
    RuntimeSource, ServeConfig,
};

const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#people> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "People" ] ;
  rr:subjectMap [ rr:template "http://example.test/person/{id}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:name ;
    rr:objectMap [ rr:column "name" ]
  ] .
"#;

fn source_parts(name: &str) -> (IntrospectedSource, SourceMapping) {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE \"People\" (\"id\" INTEGER PRIMARY KEY, \"name\" TEXT NOT NULL);\
         INSERT INTO \"People\" VALUES (1, '{}');",
        name.replace('\'', "''")
    ))
    .unwrap();
    let schema = introspect_sqlite_all(&conn).unwrap();
    let source_id = SourceId::new(0).unwrap();
    let mapping = SourceMapping::new(source_id, sf_mapping::parse_r2rml(MAPPING).unwrap());
    (
        IntrospectedSource::unchecked(Backend::sqlite(conn), schema),
        mapping,
    )
}

fn runtime_source(name: &str) -> RuntimeSource {
    let (source, mapping) = source_parts(name);
    RuntimeSource::new(source, mapping)
}

fn config(name: &str) -> ServeConfig {
    let (source, mapping) = source_parts(name);
    ServeConfig::new(
        source,
        mapping,
        crate::test_support::ontology(&[], &["http://example.test/name"]),
    )
    .unwrap()
}

fn request(body: Body) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, "application/sparql-results+json")
        .body(body)
        .unwrap()
}

async fn query_name(config: Arc<ServeConfig>) -> String {
    let response = router(config)
        .oneshot(request(Body::from(
            "SELECT ?name WHERE { ?s <http://example.test/name> ?name }",
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    json["results"]["bindings"][0]["name"]["value"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn new_http_requests_switch_whole_snapshots_after_activation() {
    let config = Arc::new(config("Alice"));
    assert_eq!(query_name(config.clone()).await, "Alice");

    let expected = config.runtime_readiness().unwrap();
    let activated = config
        .activate_snapshot(
            expected,
            RuntimeSnapshot::single(
                Epoch(1),
                crate::test_support::ontology(&[], &["http://example.test/name"]),
                runtime_source("Bob"),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(activated > expected.activation_id());
    assert_eq!(query_name(config).await, "Bob");
}

#[tokio::test]
async fn not_ready_rejects_before_polling_the_request_body() {
    let config = Arc::new(config("Alice"));
    let activation_id = config.runtime_readiness().unwrap().activation_id();
    config
        .mark_runtime_not_ready(activation_id, ReadinessCause::SchemaDrift)
        .unwrap();

    let polled = Arc::new(AtomicBool::new(false));
    let observed = polled.clone();
    let stream =
        tokio_stream::iter([Ok::<_, Infallible>(Bytes::from_static(b"query"))]).map(move |chunk| {
            observed.store(true, Ordering::SeqCst);
            chunk
        });
    let response = router(config)
        .oneshot(request(Body::from_stream(stream)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()[header::RETRY_AFTER], "1");
    assert!(!polled.load(Ordering::SeqCst));
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], "source-unavailable");
}
