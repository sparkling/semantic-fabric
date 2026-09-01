use std::future::pending;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::BodyExt;
use sf_core::query_control::{QueryControl, QueryControlError};
use sparesults::QueryResultsFormat;
use tokio::sync::{oneshot, Semaphore};
use tokio::time::Instant;
use tokio_stream::wrappers::ReceiverStream;
use tower::ServiceExt;

use crate::deadline::{join_task, run_compiler, CompilerRunError, RequestDeadline};
use crate::{router, Backend, ServeConfig};

#[tokio::test(start_paused = true)]
async fn request_clock_starts_before_body_extraction() {
    let conn = rusqlite::Connection::open_in_memory().expect("open fixture");
    let mut cfg = ServeConfig::new_unchecked(
        Backend::sqlite(conn),
        Vec::new(),
        sf_sparql::Tbox::default(),
        Vec::new(),
    );
    cfg.timeout = Duration::from_secs(15);

    let (_body_tx, body_rx) =
        tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(1);
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(axum::http::header::CONTENT_TYPE, "application/sparql-query")
        .body(axum::body::Body::from_stream(ReceiverStream::new(body_rx)))
        .expect("request");
    let response = tokio::spawn(router(Arc::new(cfg)).oneshot(request));

    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(15)).await;
    assert_eq!(
        response
            .await
            .expect("request task")
            .expect("router")
            .status(),
        axum::http::StatusCode::GATEWAY_TIMEOUT
    );
}

#[tokio::test(start_paused = true)]
async fn compiler_timeout_retains_its_permit_until_detached_work_ends() {
    let permits = Arc::new(Semaphore::new(1));
    let deadline = RequestDeadline::after(Duration::from_secs(60));
    let (started_tx, started_rx) = oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();

    let run = tokio::spawn(run_compiler(deadline, permits.clone(), move || {
        let _ = started_tx.send(());
        release_rx.recv().expect("release compiler barrier");
        7usize
    }));
    started_rx.await.expect("compiler reached barrier");

    tokio::time::advance(Duration::from_secs(60)).await;
    assert!(matches!(
        run.await.expect("compiler waiter task"),
        Err(CompilerRunError::Deadline(_))
    ));
    assert_eq!(permits.available_permits(), 0, "detached work owns permit");

    release_tx.send(()).expect("release compiler");
    let permit = permits.acquire().await.expect("permit returns after work");
    drop(permit);
}

#[tokio::test]
async fn cancelled_compiler_waiter_cannot_return_its_live_work_permit() {
    let permits = Arc::new(Semaphore::new(1));
    let deadline = RequestDeadline::after(Duration::from_secs(60));
    let (started_tx, started_rx) = oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();

    let waiter = tokio::spawn(run_compiler(deadline, permits.clone(), move || {
        let _ = started_tx.send(());
        release_rx.recv().expect("release compiler barrier");
    }));
    started_rx.await.expect("compiler reached barrier");
    waiter.abort();
    assert!(waiter.await.expect_err("waiter cancelled").is_cancelled());
    assert_eq!(
        permits.available_permits(),
        0,
        "blocking closure owns permit"
    );

    release_tx.send(()).expect("release compiler");
    let permit = permits.acquire().await.expect("permit returns after work");
    drop(permit);
}

#[tokio::test(start_paused = true)]
async fn pool_acquire_wait_uses_the_existing_absolute_deadline() {
    let pool = Arc::new(Semaphore::new(1));
    let held = pool.acquire().await.expect("hold only pool slot");
    let deadline = RequestDeadline::after(Duration::from_secs(30));
    let wait = tokio::spawn({
        let pool = pool.clone();
        async move { deadline.run(pool.acquire_owned()).await }
    });

    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(30)).await;
    assert!(wait.await.expect("pool waiter task").is_err());
    drop(held);
}

#[tokio::test(start_paused = true)]
async fn ask_timeout_aborts_the_joined_request_task() {
    struct Dropped(Option<oneshot::Sender<()>>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }

    let deadline = RequestDeadline::after(Duration::from_secs(20));
    let (started_tx, started_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let _guard = Dropped(Some(dropped_tx));
        let _ = started_tx.send(());
        pending::<bool>().await
    });
    started_rx.await.expect("ASK reached barrier");
    let wait = tokio::spawn(join_task(deadline, task));

    tokio::time::advance(Duration::from_secs(20)).await;
    assert!(wait.await.expect("ASK waiter task").is_err());
    dropped_rx.await.expect("ASK task aborted at deadline");
}

#[tokio::test(start_paused = true)]
async fn stream_driver_stall_ends_at_the_carried_deadline() {
    let deadline = RequestDeadline::after(Duration::from_secs(10));
    let (started_tx, started_rx) = oneshot::channel();
    let body = crate::stream::select_body_streaming(
        move |_sink| {
            Box::pin(async move {
                let _ = started_tx.send(());
                pending::<sf_sparql::Result<()>>().await
            })
        },
        QueryResultsFormat::Json,
        vec!["value".to_owned()],
        Some(deadline.into_std()),
    );
    started_rx.await.expect("stream driver reached barrier");

    tokio::time::advance(Duration::from_secs(10)).await;
    let error = body.collect().await.expect_err("stream must time out");
    assert!(error.to_string().contains("request timeout"), "{error}");
}

#[tokio::test(start_paused = true)]
async fn shared_control_port_uses_the_existing_tokio_clock() {
    let start = Instant::now();
    let deadline = RequestDeadline::after(Duration::from_secs(10));

    deadline
        .check_at(start + Duration::from_secs(6))
        .expect("checkpoint remains before the original deadline");
    assert_eq!(
        deadline.check_at(start + Duration::from_secs(10)),
        Err(QueryControlError::DeadlineExceeded)
    );
}

#[tokio::test(start_paused = true)]
async fn phases_consume_one_deadline_instead_of_refreshing_the_timeout() {
    let deadline = RequestDeadline::after(Duration::from_secs(10));
    deadline
        .run(tokio::time::sleep(Duration::from_secs(6)))
        .await
        .expect("first phase fits");
    assert!(deadline
        .run(tokio::time::sleep(Duration::from_secs(6)))
        .await
        .is_err());
}
