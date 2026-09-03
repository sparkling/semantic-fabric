use std::future::pending;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::BodyExt;
use sf_core::query_control::{QueryControlError, QueryLimits};
use sparesults::QueryResultsFormat;
use tokio::sync::{oneshot, Semaphore};
use tokio::time::Instant;
use tokio_stream::wrappers::ReceiverStream;
use tower::{Service, ServiceExt};

use crate::budget::RequestBudget;
use crate::deadline::{join_task, run_compiler, run_compiler_observed, CompilerRunError};
use crate::{router, Backend, ServeConfig};

fn request_budget(timeout: Duration) -> RequestBudget {
    RequestBudget::after(timeout, QueryLimits::new(u64::MAX, u64::MAX, u64::MAX))
}

#[tokio::test(start_paused = true)]
async fn time_before_inner_router_dispatch_counts_toward_the_request_deadline() {
    let conn = rusqlite::Connection::open_in_memory().expect("open fixture");
    let mut cfg = ServeConfig::new_unchecked(
        Backend::sqlite(conn),
        Vec::new(),
        sf_sparql::Tbox::default(),
        Vec::new(),
    );
    cfg.timeout = Duration::from_secs(15);

    let request = axum::http::Request::builder()
        .uri("/sparql?query=ASK%20%7B%7D")
        .body(axum::body::Body::empty())
        .expect("request");
    let mut service = router(Arc::new(cfg));
    std::future::poll_fn(|cx| {
        <crate::RequestDeadlineService as Service<
            axum::http::Request<axum::body::Body>,
        >>::poll_ready(&mut service, cx)
    })
    .await
    .expect("outer service ready");

    // `call` mints the deadline synchronously. Delaying the first poll therefore
    // models time before the inner Axum Router is dispatched.
    let response = service.call(request);
    tokio::time::advance(Duration::from_secs(15)).await;

    assert_eq!(
        response.await.expect("outer service").status(),
        axum::http::StatusCode::GATEWAY_TIMEOUT
    );
}

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
    let request_permits = Arc::new(Semaphore::new(1));
    let mut deadline = request_budget(Duration::from_secs(60));
    let request_permit = request_permits
        .clone()
        .try_acquire_owned()
        .expect("take request admission permit");
    assert!(deadline.retain_admission(request_permit).is_ok());
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
        Err(CompilerRunError::Control(
            QueryControlError::DeadlineExceeded
        ))
    ));
    assert_eq!(permits.available_permits(), 0, "detached work owns permit");
    assert_eq!(
        request_permits.available_permits(),
        0,
        "detached work owns aggregate request capacity"
    );

    release_tx.send(()).expect("release compiler");
    let permit = permits.acquire().await.expect("permit returns after work");
    drop(permit);
    let request_permit = request_permits
        .acquire()
        .await
        .expect("request capacity returns after work");
    drop(request_permit);
}

#[tokio::test]
async fn cancelled_compiler_waiter_cannot_return_its_live_work_permit() {
    let permits = Arc::new(Semaphore::new(1));
    let request_permits = Arc::new(Semaphore::new(1));
    let mut deadline = request_budget(Duration::from_secs(60));
    deadline
        .retain_admission(
            request_permits
                .clone()
                .try_acquire_owned()
                .expect("take aggregate request capacity"),
        )
        .expect("attach request admission before cloning");
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
    assert_eq!(
        request_permits.available_permits(),
        0,
        "blocking closure owns aggregate request capacity"
    );

    release_tx.send(()).expect("release compiler");
    let permit = permits.acquire().await.expect("permit returns after work");
    drop(permit);
    let request_permit = request_permits
        .acquire()
        .await
        .expect("request capacity returns after work");
    drop(request_permit);
}

#[test]
fn cancelled_queued_compiler_retains_aggregate_capacity_until_work_exits() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .expect("build bounded test runtime");

    runtime.block_on(async {
        let (blocker_started_tx, blocker_started_rx) = oneshot::channel();
        let (release_blocker_tx, release_blocker_rx) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            let _ = blocker_started_tx.send(());
            release_blocker_rx.recv().expect("release blocking lane");
        });
        tokio::time::timeout(Duration::from_secs(2), blocker_started_rx)
            .await
            .expect("blocking-lane watchdog")
            .expect("blocking lane occupied");

        let compiler_permits = Arc::new(Semaphore::new(1));
        let request_permits = Arc::new(Semaphore::new(1));
        let mut budget = request_budget(Duration::from_secs(60));
        budget
            .retain_admission(
                request_permits
                    .clone()
                    .try_acquire_owned()
                    .expect("take aggregate request capacity"),
            )
            .expect("attach request admission before cloning");
        let (submitted_tx, submitted_rx) = oneshot::channel();
        let (finished_tx, finished_rx) = oneshot::channel();
        let waiter = tokio::spawn(run_compiler_observed(
            budget,
            compiler_permits.clone(),
            move || {
                let _ = finished_tx.send(());
            },
            move || {
                let _ = submitted_tx.send(());
            },
        ));
        tokio::time::timeout(Duration::from_secs(2), submitted_rx)
            .await
            .expect("compiler-submission watchdog")
            .expect("compiler submitted behind occupied lane");

        waiter.abort();
        assert!(waiter
            .await
            .expect_err("compiler waiter is cancelled")
            .is_cancelled());
        assert_eq!(compiler_permits.available_permits(), 0);
        assert_eq!(request_permits.available_permits(), 0);

        release_blocker_tx.send(()).expect("release blocking lane");
        blocker.await.expect("blocking-lane holder task");
        tokio::time::timeout(Duration::from_secs(2), finished_rx)
            .await
            .expect("queued-compiler watchdog")
            .expect("queued compiler eventually ran");
        let request_permit = tokio::time::timeout(
            Duration::from_secs(2),
            request_permits.clone().acquire_owned(),
        )
        .await
        .expect("aggregate-capacity watchdog")
        .expect("aggregate capacity returns after compiler exit");
        drop(request_permit);
        assert_eq!(compiler_permits.available_permits(), 1);
    });
}

#[tokio::test(start_paused = true)]
async fn pool_acquire_wait_uses_the_existing_absolute_deadline() {
    let pool = Arc::new(Semaphore::new(1));
    let held = pool.acquire().await.expect("hold only pool slot");
    let deadline = request_budget(Duration::from_secs(30));
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

    let deadline = request_budget(Duration::from_secs(20));
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
    let deadline = request_budget(Duration::from_secs(10));
    let (started_tx, started_rx) = oneshot::channel();
    let body = crate::stream::select_body_streaming_controlled(
        move |_sink| {
            Box::pin(async move {
                let _ = started_tx.send(());
                pending::<sf_sparql::Result<()>>().await
            })
        },
        QueryResultsFormat::Json,
        vec!["value".to_owned()],
        deadline,
    );
    started_rx.await.expect("stream driver reached barrier");

    tokio::time::advance(Duration::from_secs(10)).await;
    let error = body.collect().await.expect_err("stream must time out");
    assert_eq!(error.to_string(), "result stream failed");
}

#[tokio::test(start_paused = true)]
async fn shared_control_port_uses_the_existing_tokio_clock() {
    let start = Instant::now();
    let deadline = request_budget(Duration::from_secs(10));

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
    let deadline = request_budget(Duration::from_secs(10));
    deadline
        .run(tokio::time::sleep(Duration::from_secs(6)))
        .await
        .expect("first phase fits");
    assert!(deadline
        .run(tokio::time::sleep(Duration::from_secs(6)))
        .await
        .is_err());
}
