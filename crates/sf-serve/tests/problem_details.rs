//! Adversarial receipts for the public RFC 9457 error boundary.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use sf_serve::{introspect_sqlite_all, router, Backend, ServeConfig};
use sf_sparql::Tbox;
use tower::ServiceExt;

const SECRET: &str = "sf_secret_NEVER_EXPOSE_7f42";
const SECRET_COLUMN: &str = "sf_secret_NEVER_EXPOSE_7f42_column";
const SECRET_TABLE: &str = "sf_secret_NEVER_EXPOSE_7f42_table";
const MAPPING_TTL: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#Items> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "sf_secret_NEVER_EXPOSE_7f42_table" ] ;
  rr:subjectMap [ rr:template "http://example.test/item/{id}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:value ;
    rr:objectMap [ rr:column "sf_secret_NEVER_EXPOSE_7f42_column" ]
  ] .
"#;

fn config_with_stale_schema() -> ServeConfig {
    config_after_schema_change(&format!(
        "ALTER TABLE \"{SECRET_TABLE}\" RENAME COLUMN \"{SECRET_COLUMN}\" TO public_value"
    ))
}

fn config_with_dropped_table() -> ServeConfig {
    config_after_schema_change(&format!("DROP TABLE \"{SECRET_TABLE}\""))
}

fn config_after_schema_change(change: &str) -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().expect("open fixture");
    conn.execute_batch(&format!(
        "CREATE TABLE \"{SECRET_TABLE}\" (id INTEGER PRIMARY KEY, \"{SECRET_COLUMN}\" TEXT); \
         INSERT INTO \"{SECRET_TABLE}\" VALUES (1, 'value');"
    ))
    .expect("seed fixture");
    let schema = introspect_sqlite_all(&conn).expect("snapshot schema");
    let mapping = sf_mapping::parse_r2rml(MAPPING_TTL).expect("parse mapping");
    conn.execute_batch(change).expect("drift live schema");
    ServeConfig::new_unchecked(Backend::sqlite(conn), mapping, Tbox::default(), schema)
}

fn request(query: &str, content_type: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, content_type)
        .header(header::ACCEPT, "text/csv")
        .body(Body::from(query.to_owned()))
        .expect("static request")
}

async fn assert_problem(
    cfg: ServeConfig,
    req: Request<Body>,
    expected_status: StatusCode,
    expected_code: &str,
) -> Vec<u8> {
    let response = router(Arc::new(cfg))
        .oneshot(req)
        .await
        .expect("route request");
    assert_eq!(response.status(), expected_status);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/problem+json"
    );
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    assert_eq!(
        response.headers().get("x-content-type-options").unwrap(),
        "nosniff"
    );
    let correlation = response
        .headers()
        .get("x-correlation-id")
        .expect("correlation header")
        .to_str()
        .expect("ASCII correlation id")
        .to_owned();
    assert!(correlation.starts_with("sf-"));
    assert!(correlation.len() <= 32, "correlation={correlation:?}");
    assert!(
        correlation
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "correlation={correlation:?}"
    );

    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect problem")
        .to_bytes()
        .to_vec();
    let json: serde_json::Value = serde_json::from_slice(&body).expect("RFC 9457 JSON");
    assert_eq!(json["status"], expected_status.as_u16());
    assert_eq!(json["code"], expected_code);
    assert_eq!(json["correlationId"], correlation);
    assert_eq!(
        json["instance"],
        format!("urn:semantic-fabric:problem-instance:{correlation}")
    );
    assert_eq!(
        json["type"],
        format!("urn:semantic-fabric:problem:{expected_code}")
    );
    assert!(json["title"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(json["detail"].as_str().is_some_and(|s| !s.is_empty()));
    body
}

fn assert_secret_absent(body: &[u8]) {
    assert!(
        !body
            .windows(SECRET.len())
            .any(|window| window == SECRET.as_bytes()),
        "secret sentinel escaped byte-for-byte: {}",
        String::from_utf8_lossy(body)
    );
}

#[tokio::test]
async fn schema_drift_failure_is_a_redacted_internal_problem() {
    let body = assert_problem(
        config_with_stale_schema(),
        request(
            "ASK { ?item <http://example.test/value> ?value }",
            "application/sparql-query",
        ),
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal-error",
    )
    .await;

    assert_secret_absent(&body);
    let text = String::from_utf8(body).expect("problem UTF-8");
    assert!(!text.to_ascii_lowercase().contains("sqlite"), "body={text}");
    assert!(!text.contains("no such column"), "body={text}");
}

#[tokio::test]
async fn dropped_table_ask_failure_is_a_redacted_internal_problem() {
    let body = assert_problem(
        config_with_dropped_table(),
        request(
            "ASK { ?item <http://example.test/value> ?value }",
            "application/sparql-query",
        ),
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal-error",
    )
    .await;

    assert_secret_absent(&body);
    let text = String::from_utf8(body).expect("problem UTF-8");
    assert!(!text.to_ascii_lowercase().contains("sqlite"), "body={text}");
    assert!(!text.contains("no such table"), "body={text}");
}

#[tokio::test]
async fn malformed_query_and_content_type_never_echo_request_sentinels() {
    let malformed = format!("SELECT {SECRET} WHERE {{ ?s ?p ?o }}");
    let body = assert_problem(
        config_with_stale_schema(),
        request(&malformed, "application/sparql-query"),
        StatusCode::BAD_REQUEST,
        "invalid-request",
    )
    .await;
    assert_secret_absent(&body);

    let content_type = format!("application/{SECRET}");
    let body = assert_problem(
        config_with_stale_schema(),
        request("ASK {}", &content_type),
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unsupported-media-type",
    )
    .await;
    assert_secret_absent(&body);
}
