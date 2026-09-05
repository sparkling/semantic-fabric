//! HTTP contract tests for source-independent health routes.

use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use sf_core::{SourceId, SourceMapping};
use sf_sparql::Epoch;
use tokio_stream::StreamExt;
use tower::ServiceExt;

use crate::{
    router, Backend, IntrospectedSource, ReadinessCause, RuntimeSnapshot, RuntimeSource,
    ServeConfig,
};

fn config() -> Arc<ServeConfig> {
    let mut config = ServeConfig::new_with_unverified_source(
        Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
        Vec::new(),
        crate::test_support::empty_ontology(),
        Vec::new(),
    )
    .unwrap();
    config.set_max_concurrent_requests(1).unwrap();
    Arc::new(config)
}

fn candidate() -> RuntimeSnapshot {
    let source_id = SourceId::new(0).unwrap();
    RuntimeSnapshot::single(
        Epoch(1),
        crate::test_support::empty_ontology(),
        RuntimeSource::new(
            IntrospectedSource::unchecked(
                Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
                Vec::new(),
            ),
            SourceMapping::new(source_id, Vec::new()),
        ),
    )
    .unwrap()
}

fn request(path: &str, body: Body) -> Request<Body> {
    Request::builder().uri(path).body(body).unwrap()
}

async fn body(response: axum::response::Response) -> Bytes {
    response.into_body().collect().await.unwrap().to_bytes()
}

#[tokio::test]
async fn livez_reports_only_event_loop_liveness() {
    let config = config();
    let activation = config.runtime_readiness().unwrap().activation_id();
    config
        .mark_runtime_not_ready(activation, ReadinessCause::SourceUnavailable)
        .unwrap();

    let response = router(config)
        .oneshot(request("/livez", Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(body(response).await, r#"{"status":"live"}"#);
}

#[tokio::test]
async fn readyz_tracks_ready_not_ready_and_reactivated_snapshots() {
    let config = config();
    let ready = router(config.clone())
        .oneshot(request("/readyz", Body::empty()))
        .await
        .unwrap();
    assert_eq!(ready.status(), StatusCode::OK);
    assert_eq!(ready.headers()[header::CONTENT_TYPE], "application/json");
    assert_eq!(ready.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(ready.headers()["x-content-type-options"], "nosniff");
    assert_eq!(body(ready).await, r#"{"status":"ready"}"#);

    let activation = config.runtime_readiness().unwrap().activation_id();
    config
        .mark_runtime_not_ready(activation, ReadinessCause::SchemaDrift)
        .unwrap();
    let not_ready = router(config.clone())
        .oneshot(request("/readyz", Body::empty()))
        .await
        .unwrap();
    assert_eq!(not_ready.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        not_ready.headers()[header::CONTENT_TYPE],
        "application/json"
    );
    assert_eq!(not_ready.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(not_ready.headers()["x-content-type-options"], "nosniff");
    assert_eq!(not_ready.headers()[header::RETRY_AFTER], "1");
    assert_eq!(body(not_ready).await, r#"{"status":"not-ready"}"#);

    let expected = config.runtime_readiness().unwrap();
    config.activate_snapshot(expected, candidate()).unwrap();
    let ready_again = router(config)
        .oneshot(request("/readyz", Body::empty()))
        .await
        .unwrap();
    assert_eq!(ready_again.status(), StatusCode::OK);
    assert_eq!(body(ready_again).await, r#"{"status":"ready"}"#);
}

#[tokio::test]
async fn health_paths_poll_neither_request_body_nor_source_admission() {
    let backend = Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap());
    let Backend::Sqlite(pool) = backend.clone() else {
        unreachable!()
    };
    let mut unshared = ServeConfig::new_with_unverified_source(
        backend,
        Vec::new(),
        crate::test_support::empty_ontology(),
        Vec::new(),
    )
    .unwrap();
    unshared.set_max_concurrent_requests(1).unwrap();
    let config = Arc::new(unshared);
    let sole_permit = config
        .request_admission_permits()
        .try_acquire_owned()
        .expect("hold all application capacity");
    let polls = Arc::new(AtomicUsize::new(0));
    let source = pool.pick();
    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let blocker = std::thread::spawn(move || {
        let source_guard = source.lock().unwrap();
        held_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        drop(source_guard);
    });
    held_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("source lock held");

    for path in ["/livez", "/readyz"] {
        let observed = polls.clone();
        let stream = tokio_stream::iter([Ok::<_, Infallible>(Bytes::from_static(b"ignored"))]).map(
            move |chunk| {
                observed.fetch_add(1, Ordering::SeqCst);
                chunk
            },
        );
        let response = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            router(config.clone()).oneshot(request(path, Body::from_stream(stream))),
        )
        .await
        .expect("health route must not wait on the locked source")
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(polls.load(Ordering::SeqCst), 0);
    }

    assert_eq!(config.available_request_permits(), 0);
    release_tx.send(()).unwrap();
    blocker.join().unwrap();
    drop(sole_permit);
    assert_eq!(config.available_request_permits(), 1);
}
