//! Admission, limit, timeout, and failure-mapping tests.

use std::sync::Arc;

use axum::http::{header, StatusCode};
use http_body_util::BodyExt;
use sf_serve::{router, IntrospectedSource, ServeConfig, DEFAULT_QUERY_LIMITS};
use tower::ServiceExt;

use crate::fixtures::{big_sqlite_config_with_limits, limits_with, post_query, send};
use crate::fixtures::{source_work_limits, sqlite_config, sqlite_config_with_pool};
use crate::fixtures::{
    streams_completely, FIXTURE_20000_ROW_SOURCE_WORK, ROWS_20000_MIN_SOURCE_WORK,
};
use crate::support;

/// The 20 000-row concurrency fixtures exceed the default finite source-work
/// limit. Raising every other named limit alone does not admit them; the
/// pinned minimal source-work value admits them and one unit less refuses.
#[tokio::test]
async fn source_work_limit_alone_refuses_20000_row_stream() {
    const ROWS: usize = 20_000;
    const RAISED: u64 = 1 << 40;
    let make = |limits| big_sqlite_config_with_limits(ROWS, limits);
    let defaults = DEFAULT_QUERY_LIMITS;

    assert!(
        !streams_completely(make(defaults), ROWS).await,
        "default limits must keep refusing the 20000-row stream"
    );

    let others_raised = limits_with(RAISED, defaults.max_source_work(), RAISED, RAISED);
    assert!(
        !streams_completely(make(others_raised), ROWS).await,
        "raising every other named limit must not admit the stream"
    );

    let below = source_work_limits(ROWS_20000_MIN_SOURCE_WORK - 1);
    assert!(
        !streams_completely(make(below), ROWS).await,
        "one source-work unit below the measured minimum must refuse the stream"
    );

    let minimum = source_work_limits(ROWS_20000_MIN_SOURCE_WORK);
    assert!(
        streams_completely(make(minimum), ROWS).await,
        "the measured minimal source work alone must admit the stream"
    );

    let fixture = source_work_limits(FIXTURE_20000_ROW_SOURCE_WORK);
    assert!(
        streams_completely(make(fixture), ROWS).await,
        "the concurrency fixtures' source-work allowance must admit the stream"
    );
}

#[tokio::test]
async fn oversized_query_returns_413() {
    let mut cfg = sqlite_config();
    cfg.set_max_query_len(16)
        .expect("representable query limit"); // shrink below any real query
    let cfg = Arc::new(cfg);
    let req = post_query(
        "PREFIX ex: <http://ex/> SELECT ?n WHERE { ?p ex:name ?n }",
        "application/sparql-results+json",
    );
    let (status, ctype, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(ctype, "application/problem+json");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["code"],
        "payload-too-large"
    );
}

#[tokio::test]
async fn ask_query_exceeding_timeout_returns_504() {
    // Deterministic (not machine-speed-dependent): `Backend::Sqlite` guards its
    // connection with a plain blocking `std::sync::Mutex` (confirmed in
    // `sf_serve::lib`). Hold that raw lock from a background OS thread and use
    // an explicit acquisition/release barrier — the leased worker genuinely
    // blocks trying to acquire it, so the request deadline reliably elapses
    // before any response is sent (ASK collects a single boolean,
    // unlike a streamed SELECT/CONSTRUCT whose 200 status line commits before
    // any deadline is ever checked mid-body — why ASK is the clean way to
    // reach 504 here). A workload-dependent "make the SQL itself slow" query
    // is inherently flaky across machines; this is not.
    let (mut cfg, pool) = sqlite_config_with_pool();
    cfg.timeout = std::time::Duration::from_millis(20);
    let conn = pool.pick();
    let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let hold = std::thread::spawn(move || {
        let _guard = conn.lock().unwrap();
        acquired_tx.send(()).expect("signal held raw mutex");
        release_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("release held raw mutex");
    });
    tokio::task::spawn_blocking(move || {
        acquired_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("raw mutex holder did not reach barrier")
    })
    .await
    .expect("raw mutex barrier task");
    let cfg = Arc::new(cfg);
    let req = post_query("ASK { ?s ?p ?o }", "application/sparql-results+json");
    let (status, ctype, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(ctype, "application/problem+json");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["code"],
        "request-timeout"
    );
    release_tx.send(()).expect("release raw mutex holder");
    hold.join().unwrap();
}

#[tokio::test]
async fn ask_query_backend_sql_error_returns_500() {
    // A genuine runtime SQL failure (not a compile-time / translate-time one):
    // the runtime binding's observed schema still declares the "age" column (so translate
    // succeeds), but the LIVE connection's table no longer has it — a schema
    // drift between compile-time metadata and the actual DB state, exactly the
    // shape `SparqlError::Sql` -> 500 exists to cover. Cleaner than any hack:
    // it's a real, reachable production scenario (concurrent DDL change).
    let (cfg, pool) = sqlite_config_with_pool();
    pool.pick()
        .lock()
        .unwrap()
        .execute_batch("ALTER TABLE \"People\" RENAME COLUMN \"age\" TO \"age_renamed\";")
        .unwrap();
    let cfg = Arc::new(cfg);
    let req = post_query(
        "PREFIX ex: <http://ex/> ASK { ?p ex:age ?a }",
        "application/sparql-results+json",
    );
    let (status, _, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!body.is_empty());
}

// ---- PostgreSQL variant: gate-skips when no server on localhost:5432 ----------

mod pg {
    use super::*;

    use crate::fixtures::pg::{base_conn, pg_pool_sized_with_wait};

    /// The ADR-0010 §C admission-control addition ([`sf_serve::acquire_pg`], not
    /// public — exercised only through the endpoint): a `max_size=1` pool with a
    /// short wait timeout sheds a request as `503` + `Retry-After` while the only
    /// connection is held, rather than queueing it indefinitely or failing with
    /// a generic `500`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pg_pool_exhaustion_sheds_503_with_retry_after() {
        let conn_str = base_conn();
        let Ok((client, connection)) =
            tokio_postgres::connect(&conn_str, tokio_postgres::NoTls).await
        else {
            eprintln!(
                "SKIP pg_pool_exhaustion_sheds_503_with_retry_after: \
                 no PostgreSQL on localhost:5432"
            );
            return;
        };
        tokio::spawn(async move {
            let _ = connection.await;
        });

        let table = format!("sf_serve_pool_503_{}", std::process::id());
        const ROWS: i64 = 32;
        client
            .batch_execute(&format!(
                "DROP TABLE IF EXISTS \"{table}\"; \
                 CREATE TABLE \"{table}\" (\"id\" INTEGER PRIMARY KEY, \"name\" TEXT); \
                 INSERT INTO \"{table}\" SELECT g, 'Person' || g \
                 FROM generate_series(1, {ROWS}) AS g;"
            ))
            .await
            .expect("seed table");

        let mapping_ttl = format!(
            r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#People> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "{table}" ] ;
  rr:subjectMap [ rr:template "http://ex/person/{{id}}" ; rr:class ex:Person ] ;
  rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] .
"#
        );
        let maps = sf_mapping::parse_r2rml(&mapping_ttl).unwrap();
        let ontology = support::ontology_for_mapping(&maps);

        // Hold the correctly configured pool's only connection explicitly.
        // This proves the exhaustion mapping without racing a first request's
        // compiler and stream startup against an arbitrary sleep.
        let pool =
            pg_pool_sized_with_wait(&conn_str, 1, Some(std::time::Duration::from_millis(50)));
        let source = IntrospectedSource::observe_postgres(pool.clone())
            .await
            .unwrap();
        let mut cfg = ServeConfig::from_authored_r2rml(source, &mapping_ttl, ontology).unwrap();
        cfg.set_parser_runtime(support::parser_runtime());
        cfg.set_query_admission(sf_serve::QueryAdmission::UnrestrictedDevelopment);
        let cfg = Arc::new(cfg);
        let held = pool.get().await.expect("hold the sole PG pool connection");

        let resp2 = router(cfg.clone())
            .oneshot(post_query(
                "SELECT ?name WHERE { ?s <http://ex/name> ?name }",
                "application/sparql-results+json",
            ))
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            resp2.headers().get(header::RETRY_AFTER).unwrap(),
            "1",
            "shed response must carry Retry-After (ADR-0010 §C)"
        );
        let body2 = String::from_utf8(
            resp2
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(body2.contains("\"code\":\"source-unavailable\""), "{body2}");
        assert!(!body2.contains("PoolError"), "{body2}");

        // Once capacity returns, the same governed path must recover and use a
        // relation-scope-verified pooled session.
        drop(held);
        let resp1 = router(cfg.clone())
            .oneshot(post_query(
                "SELECT ?name WHERE { ?s <http://ex/name> ?name }",
                "application/sparql-results+json",
            ))
            .await
            .unwrap();
        assert_eq!(
            resp1.status(),
            StatusCode::OK,
            "the request after capacity is released must succeed"
        );
        // Drop the body without draining it to exercise cancel-on-drop and free
        // the connection before fixture cleanup.
        drop(resp1);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let _ = client
            .batch_execute(&format!("DROP TABLE IF EXISTS \"{table}\""))
            .await;
    }
}
