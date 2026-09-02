//! Focused admission tests for SPARQL Protocol POST request bodies.

use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use sf_serve::{
    introspect_sqlite_all, router, serve_blocking, Backend, ServeConfig, ServeOptions, SourceRef,
};
use sf_sparql::Tbox;
use tower::ServiceExt;

const MAPPING_TTL: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#Items> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "items" ] ;
  rr:subjectMap [ rr:template "http://example.test/item/{id}" ] .
"#;

fn config(max_query_len: usize) -> ServeConfig {
    let connection = rusqlite::Connection::open_in_memory().expect("open fixture");
    connection
        .execute_batch("CREATE TABLE items (id INTEGER PRIMARY KEY);")
        .expect("create fixture table");
    let schema = introspect_sqlite_all(&connection).expect("introspect fixture");
    let mapping = sf_mapping::parse_r2rml(MAPPING_TTL).expect("parse mapping");
    let mut config = ServeConfig::new_unchecked(
        Backend::sqlite(connection),
        mapping,
        Tbox::default(),
        schema,
    );
    config
        .set_max_query_len(max_query_len)
        .expect("representable query limit");
    config
}

fn post(content_type: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, content_type)
        .body(body)
        .expect("static request")
}

async fn send(config: ServeConfig, request: Request<Body>) -> (StatusCode, String) {
    let response = router(Arc::new(config))
        .oneshot(request)
        .await
        .expect("route request");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect response")
        .to_bytes();
    (
        status,
        String::from_utf8(bytes.to_vec()).expect("UTF-8 response"),
    )
}

struct PollCountingStream {
    polls: Arc<AtomicUsize>,
}

impl tokio_stream::Stream for PollCountingStream {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Ready(Some(Err(std::io::Error::other(
            "body must remain unconsumed",
        ))))
    }
}

fn counted_body() -> (Body, Arc<AtomicUsize>) {
    let polls = Arc::new(AtomicUsize::new(0));
    let stream = PollCountingStream {
        polls: polls.clone(),
    };
    (Body::from_stream(stream), polls)
}

fn assert_problem(status: StatusCode, body: &str, expected: StatusCode, code: &str) {
    assert_eq!(status, expected, "body={body}");
    let json: serde_json::Value = serde_json::from_str(body).expect("problem JSON");
    assert_eq!(json["code"], code);
}

#[tokio::test]
async fn should_not_poll_body_when_post_media_type_is_unsupported() {
    let (request_body, polls) = counted_body();
    let (status, body) = send(config(64), post("text/plain", request_body)).await;

    assert_problem(
        status,
        &body,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unsupported-media-type",
    );
    assert_eq!(polls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn should_not_poll_body_when_content_type_is_missing() {
    let (request_body, polls) = counted_body();
    let request = Request::builder()
        .method("POST")
        .uri("/sparql")
        .body(request_body)
        .expect("static request");

    let (status, body) = send(config(64), request).await;

    assert_problem(
        status,
        &body,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unsupported-media-type",
    );
    assert_eq!(polls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn should_redact_an_admitted_body_stream_failure() {
    let (request_body, polls) = counted_body();
    let (status, body) = send(config(64), post("application/sparql-query", request_body)).await;

    assert_problem(status, &body, StatusCode::BAD_REQUEST, "invalid-request");
    assert!(!body.contains("body must remain unconsumed"));
    assert_eq!(polls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn should_reject_media_type_parameters_without_polling_the_body() {
    let (request_body, polls) = counted_body();
    let (status, body) = send(
        config(64),
        post("application/sparql-query; version=1.2", request_body),
    )
    .await;

    assert_problem(
        status,
        &body,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unsupported-media-type",
    );
    assert_eq!(polls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn should_reject_duplicate_content_type_headers_without_polling_the_body() {
    let (request_body, polls) = counted_body();
    let mut request = post("application/sparql-query", request_body);
    request.headers_mut().append(
        header::CONTENT_TYPE,
        "application/sparql-query".parse().expect("static header"),
    );

    let (status, body) = send(config(64), request).await;

    assert_problem(status, &body, StatusCode::BAD_REQUEST, "invalid-request");
    assert_eq!(polls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn should_reject_post_uri_parameters_without_polling_the_body() {
    let (request_body, polls) = counted_body();
    let request = Request::builder()
        .method("POST")
        .uri("/sparql?default-graph-uri=https%3A%2F%2Fexample.test%2Fgraph")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .body(request_body)
        .expect("static request");

    let (status, body) = send(config(64), request).await;

    assert_problem(status, &body, StatusCode::BAD_REQUEST, "invalid-request");
    assert_eq!(polls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn should_reject_get_dataset_and_version_parameters() {
    const QUERY: &str = "ASK { ?s ?p ?o }";
    for field in ["default-graph-uri", "named-graph-uri", "version"] {
        let encoded = form_urlencoded::Serializer::new(String::new())
            .append_pair("query", QUERY)
            .append_pair(field, "1.2")
            .finish();
        let request = Request::builder()
            .uri(format!("/sparql?{encoded}"))
            .body(Body::empty())
            .expect("static request");

        let (status, body) = send(config(QUERY.len()), request).await;

        assert_problem(status, &body, StatusCode::BAD_REQUEST, "invalid-request");
    }
}

#[tokio::test]
async fn should_accept_raw_query_at_exact_limit_and_reject_one_byte_more() {
    const QUERY: &str = "ASK { ?s ?p ?o }";

    let (status, body) = send(
        config(QUERY.len()),
        post("application/sparql-query", Body::from(QUERY)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let oversized = format!("{QUERY} ");
    let (status, body) = send(
        config(QUERY.len()),
        post("application/sparql-query", Body::from(oversized)),
    )
    .await;
    assert_problem(
        status,
        &body,
        StatusCode::PAYLOAD_TOO_LARGE,
        "payload-too-large",
    );
}

#[tokio::test]
async fn should_accept_percent_encoded_query_key_at_exact_worst_case_form_limit() {
    const QUERY: &str = "ASK { ?s ?p ?o }";
    let encoded_value = QUERY
        .bytes()
        .map(|byte| format!("%{byte:02X}"))
        .collect::<String>();
    let encoded = format!("%71%75%65%72%79={encoded_value}");
    assert_eq!(encoded.len(), 3 * QUERY.len() + 16);

    let (status, body) = send(
        config(QUERY.len()),
        post(
            "application/x-www-form-urlencoded",
            Body::from(encoded.clone()),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body}");

    let oversized = format!("{encoded}&");
    let (status, body) = send(
        config(QUERY.len()),
        post("application/x-www-form-urlencoded", Body::from(oversized)),
    )
    .await;
    assert_problem(
        status,
        &body,
        StatusCode::PAYLOAD_TOO_LARGE,
        "payload-too-large",
    );
}

#[tokio::test]
async fn should_reject_decoded_form_query_over_the_limit() {
    const QUERY: &str = "ASK { ?s ?p ?o }";
    let encoded = form_urlencoded::Serializer::new(String::new())
        .append_pair("query", QUERY)
        .finish();

    let (status, body) = send(
        config(QUERY.len() - 1),
        post("application/x-www-form-urlencoded", Body::from(encoded)),
    )
    .await;

    assert_problem(
        status,
        &body,
        StatusCode::PAYLOAD_TOO_LARGE,
        "payload-too-large",
    );
}

#[tokio::test]
async fn should_reject_duplicate_query_form_fields() {
    const QUERY: &str = "ASK { ?s ?p ?o }";
    let encoded = form_urlencoded::Serializer::new(String::new())
        .append_pair("query", QUERY)
        .append_pair("query", QUERY)
        .finish();

    let (status, body) = send(
        config(128),
        post("application/x-www-form-urlencoded", Body::from(encoded)),
    )
    .await;

    assert_problem(status, &body, StatusCode::BAD_REQUEST, "invalid-request");
}

#[tokio::test]
async fn should_reject_every_non_query_form_field() {
    const QUERY: &str = "ASK { ?s ?p ?o }";
    for field in ["default-graph-uri", "named-graph-uri", "version", "other"] {
        let encoded = form_urlencoded::Serializer::new(String::new())
            .append_pair("query", QUERY)
            .append_pair(field, "https://example.test/graph")
            .finish();
        let (status, body) = send(
            config(128),
            post("application/x-www-form-urlencoded", Body::from(encoded)),
        )
        .await;

        assert_problem(status, &body, StatusCode::BAD_REQUEST, "invalid-request");
    }
}

#[test]
fn should_reject_unrepresentable_limit_through_public_config_api() {
    let mut config = config(64);
    let max_representable = (usize::MAX - 16) / 3;
    config
        .set_max_query_len(max_representable)
        .expect("largest representable form-body cap must be accepted");

    let error = config
        .set_max_query_len(max_representable + 1)
        .expect_err("unrepresentable form-body cap must be rejected");

    assert_eq!(error.code(), "startup-configuration");
    assert_eq!(config.max_query_len(), max_representable);
    let _service = router(Arc::new(config));
}

#[test]
fn should_reject_unrepresentable_limit_before_source_or_file_io() {
    let options = ServeOptions {
        source: SourceRef::environment("SF_POST_BODY_ADMISSION_MUST_NOT_BE_READ"),
        mapping_path: "/path/that/must/not/be/read.ttl".to_owned(),
        ontology_path: Some("/ontology/that/must/not/be/read.ttl".to_owned()),
        bind: "203.0.113.1:1".to_owned(),
        timeout: Duration::from_secs(1),
        max_query_len: usize::MAX,
        max_source_work: 1,
        max_result_items: 1,
        max_serialized_bytes: 1,
        pg_pool_size: 1,
        pg_pool_wait: Duration::from_secs(1),
        sqlite_pool_size: 1,
    };

    let error = serve_blocking(options).expect_err("body-cap configuration must fail first");

    assert_eq!(error.code(), "startup-configuration");
}
