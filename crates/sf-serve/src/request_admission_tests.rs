//! Deterministic service-boundary tests for aggregate request admission.

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use rusqlite::functions::FunctionFlags;
use sf_core::query_control::{QueryControl, QueryLimits};
use sf_sql::backend::sqlite::{SqliteOwnedBackend, SqliteOwnedConnection};
use sf_sql::backend::SqlBackend;
use tokio::sync::oneshot;
use tokio_stream::Stream;
use tower::{Service, ServiceExt};

use crate::{router, Backend, ServeConfig};

struct PendingBody {
    started: Option<oneshot::Sender<()>>,
    dropped: Option<oneshot::Sender<()>>,
}

impl Stream for PendingBody {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(started) = self.started.take() {
            let _ = started.send(());
        }
        Poll::Pending
    }
}

impl Drop for PendingBody {
    fn drop(&mut self) {
        if let Some(dropped) = self.dropped.take() {
            let _ = dropped.send(());
        }
    }
}

struct CountingBody(Arc<AtomicUsize>);

impl Stream for CountingBody {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}

fn config(capacity: usize, timeout: Duration) -> Arc<ServeConfig> {
    let conn = rusqlite::Connection::open_in_memory().expect("open fixture");
    let mut cfg = ServeConfig::new_with_unverified_source(
        Backend::sqlite(conn),
        Vec::new(),
        crate::test_support::empty_ontology(),
        Vec::new(),
    )
    .unwrap();
    cfg.timeout = timeout;
    cfg.set_max_concurrent_requests(capacity)
        .expect("valid request capacity");
    Arc::new(cfg)
}

fn stream_request<S>(stream: S) -> Request<Body>
where
    S: Stream<Item = Result<Bytes, io::Error>> + Send + 'static,
{
    Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .body(Body::from_stream(stream))
        .expect("static request")
}

fn missing_request() -> Request<Body> {
    Request::builder()
        .uri("/not-a-route")
        .body(Body::empty())
        .expect("static request")
}

#[tokio::test(start_paused = true)]
async fn saturation_sheds_without_queueing_or_polling_the_request_body() {
    let cfg = config(1, Duration::from_secs(60));
    let (started_tx, started_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let first = tokio::spawn(router(cfg.clone()).oneshot(stream_request(PendingBody {
        started: Some(started_tx),
        dropped: Some(dropped_tx),
    })));
    tokio::time::timeout(Duration::from_secs(1), started_rx)
        .await
        .expect("body-entry watchdog")
        .expect("first body reached inner router");
    assert_eq!(cfg.available_request_permits(), 0);

    let polls = Arc::new(AtomicUsize::new(0));
    let mut second = router(cfg.clone());
    std::future::poll_fn(|cx| Service::poll_ready(&mut second, cx))
        .await
        .expect("outer service remains ready while saturated");
    let response = second
        .call(stream_request(CountingBody(polls.clone())))
        .await
        .expect("infallible service");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()[header::RETRY_AFTER], "1");
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert!(response.headers().contains_key("x-correlation-id"));
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect overload problem")
        .to_bytes();
    let problem: serde_json::Value = serde_json::from_slice(&body).expect("problem JSON");
    assert_eq!(problem["code"], "service-overloaded");
    assert_eq!(polls.load(Ordering::SeqCst), 0);

    first.abort();
    assert!(first
        .await
        .expect_err("first request is cancelled")
        .is_cancelled());
    tokio::time::timeout(Duration::from_secs(1), dropped_rx)
        .await
        .expect("body-drop watchdog")
        .expect("pending request body dropped");
    assert_eq!(cfg.available_request_permits(), 1);
    assert_eq!(
        polls.load(Ordering::SeqCst),
        0,
        "shed request never retries"
    );

    let recovered = router(cfg)
        .oneshot(missing_request())
        .await
        .expect("infallible service");
    assert_eq!(recovered.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(start_paused = true)]
async fn expired_budget_precedes_overload_without_consuming_capacity() {
    let cfg = config(1, Duration::ZERO);
    let permits = cfg.request_admission_permits();
    let held = permits
        .clone()
        .try_acquire_owned()
        .expect("hold sole permit");
    let polls = Arc::new(AtomicUsize::new(0));

    let response = router(cfg)
        .oneshot(stream_request(CountingBody(polls.clone())))
        .await
        .expect("infallible service");

    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    assert_eq!(
        permits.available_permits(),
        0,
        "no extra permit was consumed"
    );
    drop(held);
    assert_eq!(permits.available_permits(), 1);
}

#[tokio::test(start_paused = true)]
async fn delayed_overload_handoff_is_reclassified_as_request_timeout() {
    let timeout = Duration::from_secs(5);
    let cfg = config(1, timeout);
    let permits = cfg.request_admission_permits();
    let held = permits
        .clone()
        .try_acquire_owned()
        .expect("hold sole permit");
    let mut service = router(cfg);
    std::future::poll_fn(|cx| Service::poll_ready(&mut service, cx))
        .await
        .expect("outer service ready");

    let response = service.call(missing_request());
    tokio::time::advance(timeout).await;
    let response = response.await.expect("infallible service");

    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(permits.available_permits(), 0);
    drop(held);
}

#[tokio::test(start_paused = true)]
async fn unrepresentable_deadline_fails_before_request_admission() {
    let cfg = config(1, Duration::MAX);
    let permits = cfg.request_admission_permits();

    let response = router(cfg)
        .oneshot(missing_request())
        .await
        .expect("infallible service");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(permits.available_permits(), 1);
}

#[tokio::test]
async fn closed_admission_gate_is_an_internal_failure_not_overload() {
    let cfg = config(1, Duration::from_secs(60));
    let permits = cfg.request_admission_permits();
    permits.close();

    let response = router(cfg)
        .oneshot(missing_request())
        .await
        .expect("infallible service");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect internal problem")
        .to_bytes();
    let problem: serde_json::Value = serde_json::from_slice(&body).expect("problem JSON");
    assert_eq!(problem["code"], "internal-error");
}

#[tokio::test]
async fn completed_inner_response_does_not_hold_capacity_while_body_drains() {
    let cfg = config(1, Duration::from_secs(60));
    let response = router(cfg.clone())
        .oneshot(missing_request())
        .await
        .expect("infallible service");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(cfg.available_request_permits(), 1);
    drop(response);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn submitted_sqlite_worker_retains_capacity_until_worker_exit() {
    let conn = rusqlite::Connection::open_in_memory().expect("open fixture");
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    conn.create_scalar_function("hold_request", 0, FunctionFlags::SQLITE_UTF8, move |_| {
        started_tx.send(()).expect("report SQLite worker entry");
        release_rx.recv().expect("release SQLite worker");
        Ok(7_i64)
    })
    .expect("register holder function");
    let owned = SqliteOwnedConnection::new(conn);
    let lease = owned.acquire().await.expect("take SQLite serving lease");

    let request_permits = Arc::new(tokio::sync::Semaphore::new(1));
    let mut budget = crate::budget::RequestBudget::after(
        Duration::from_secs(60),
        QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
    );
    budget
        .retain_admission(
            request_permits
                .clone()
                .try_acquire_owned()
                .expect("take aggregate request capacity"),
        )
        .expect("attach request admission before cloning");
    let control: Arc<dyn QueryControl> = Arc::new(budget.clone());
    let mut backend = SqliteOwnedBackend::new_controlled_leased(lease, control);
    let stream = backend
        .open_branch("SELECT hold_request() AS value", &[])
        .await
        .expect("submit SQLite worker");
    drop(backend);
    drop(budget);

    tokio::task::spawn_blocking(move || {
        started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("SQLite worker-entry watchdog");
    })
    .await
    .expect("worker-entry observer");
    assert_eq!(request_permits.available_permits(), 0);

    release_tx.send(()).expect("release SQLite worker");
    let permit = tokio::time::timeout(
        Duration::from_secs(2),
        request_permits.clone().acquire_owned(),
    )
    .await
    .expect("request-capacity watchdog")
    .expect("capacity returns only after worker exit");
    drop(permit);
    assert_eq!(request_permits.available_permits(), 1);
    drop(stream);
}
