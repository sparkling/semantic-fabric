//! Native HTTP receipts for the narrow SQLite active-VM cancellation contract.
//!
//! These tests intentionally make no claim about cancellation while waiting for
//! the connection mutex, blocking UDF/VFS/I/O calls, SQLite's default busy
//! timeout, non-SQLite backends, compiler work, total M2 completion, or atomic
//! failure after an HTTP 200 response has been committed.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use rusqlite::functions::FunctionFlags;
use sf_serve::{introspect_sqlite_all, router, Backend, ServeConfig};
use sf_sparql::Tbox;
use tower::ServiceExt;

const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#Slow> a rr:TriplesMap ;
  rr:logicalTable [ rr:sqlQuery """
    WITH RECURSIVE counter(value) AS (
      VALUES(test_notify())
      UNION ALL
      SELECT value + 1 FROM counter WHERE value < 500000000
    )
    SELECT value AS id, value FROM counter WHERE value = 500000000
  """ ] ;
  rr:subjectMap [ rr:template "http://example.test/slow/{id}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:slowValue ;
    rr:objectMap [ rr:column "value" ]
  ] .
<#Fast> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "fast_items" ] ;
  rr:subjectMap [ rr:template "http://example.test/fast/{id}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:fastValue ;
    rr:objectMap [ rr:column "value" ]
  ] .
"#;

#[derive(Default)]
struct StatementBarrier {
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}

impl StatementBarrier {
    fn enter_and_wait(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.0 = true;
        self.changed.notify_all();
        while !state.1 {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    fn wait_until_entered(&self) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (state, timeout) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(2), |state| !state.0)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(!timeout.timed_out(), "slow SQLite statement did not start");
        assert!(state.0);
    }

    fn release(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.1 = true;
        self.changed.notify_all();
    }
}

fn shared_config(timeout: Duration) -> (Arc<ServeConfig>, Arc<StatementBarrier>) {
    let conn = rusqlite::Connection::open_in_memory().expect("open fixture");
    conn.execute_batch(
        "CREATE TABLE fast_items (id INTEGER PRIMARY KEY, value TEXT NOT NULL); \
         INSERT INTO fast_items VALUES (1, 'ready');",
    )
    .expect("seed fixture");
    let barrier = Arc::new(StatementBarrier::default());
    conn.create_scalar_function("test_notify", 0, FunctionFlags::SQLITE_UTF8, {
        let barrier = barrier.clone();
        move |_| {
            barrier.enter_and_wait();
            Ok(0_i64)
        }
    })
    .expect("register statement-entry latch");
    let schema = introspect_sqlite_all(&conn).expect("introspect fixture");
    let mapping = sf_mapping::parse_r2rml(MAPPING).expect("parse mapping");
    let mut config =
        ServeConfig::new_unchecked(Backend::sqlite(conn), mapping, Tbox::default(), schema);
    config.timeout = timeout;
    (Arc::new(config), barrier)
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

async fn wait_for_statement(barrier: Arc<StatementBarrier>) {
    tokio::task::spawn_blocking(move || barrier.wait_until_entered())
        .await
        .expect("statement latch task");
}

async fn assert_fast_request_reuses_connection(config: Arc<ServeConfig>) {
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        router(config).oneshot(request(
            "ASK { ?item <http://example.test/fastValue> ?value }",
        )),
    )
    .await
    .expect("connection reuse timed out")
    .expect("route fast request");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timed_out_ask_interrupts_started_sqlite_statement_before_connection_reuse() {
    let (config, barrier) = shared_config(Duration::from_secs(1));
    let slow = tokio::spawn(router(config.clone()).oneshot(request(
        "ASK { ?item <http://example.test/slowValue> ?value }",
    )));
    wait_for_statement(barrier.clone()).await;

    let response = tokio::time::timeout(Duration::from_secs(2), slow).await;
    barrier.release();
    let response = response
        .expect("slow request did not reach its deadline")
        .expect("slow request task")
        .expect("route slow request");
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_fast_request_reuses_connection(config).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_select_body_interrupts_started_sqlite_statement_before_connection_reuse() {
    let (config, barrier) = shared_config(Duration::from_secs(30));
    let response = router(config.clone())
        .oneshot(request(
            "SELECT ?value WHERE { ?item <http://example.test/slowValue> ?value }",
        ))
        .await
        .expect("route slow request");
    let status = response.status();
    wait_for_statement(barrier.clone()).await;

    drop(response);
    barrier.release();
    assert_eq!(status, StatusCode::OK);
    assert_fast_request_reuses_connection(config).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_200_sqlite_interruption_ends_with_stable_redacted_stream_error() {
    let (config, barrier) = shared_config(Duration::from_secs(1));
    let response = router(config.clone())
        .oneshot(request(
            "SELECT ?value WHERE { ?item <http://example.test/slowValue> ?value }",
        ))
        .await
        .expect("route slow request");
    let status = response.status();
    wait_for_statement(barrier.clone()).await;

    let collected =
        tokio::time::timeout(Duration::from_secs(2), response.into_body().collect()).await;
    barrier.release();
    let error = collected
        .expect("stream did not reach its deadline")
        .expect_err("active SQLite stream must be interrupted");
    assert_eq!(status, StatusCode::OK);
    assert_eq!(error.to_string(), "result stream failed");
    assert_fast_request_reuses_connection(config).await;
}
