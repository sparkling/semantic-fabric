//! Deterministic proofs for consumer-driven early-stop demand: no row is
//! decoded or charged before its consumer asks, retried polls mint no extra
//! demand, terminal causes stay sticky, and an idle worker releases admission
//! on consumer drop, cancellation, or deadline.

use std::future::Future;
use std::sync::{mpsc, Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

use rusqlite::Connection;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::Error;

use super::super::super::cancellation::{SqliteCancellationEvent, SqliteCancellationObserver};
use super::super::{SqliteOwnedBackend, SqliteOwnedConnection};
use super::{bridge, SqliteReceiverStream};

const TWO_ROWS: &str = "SELECT v FROM t";

fn budget() -> Arc<QueryBudget> {
    Arc::new(QueryBudget::new(QueryLimits::new(
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    )))
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime")
}

fn tuple(value: &str) -> RawTuple {
    RawTuple {
        values: vec![Some(value.to_owned())],
        codes: vec![None],
    }
}

/// Poll one `next_row` call exactly once, then drop its future.
fn poll_once(rt: &tokio::runtime::Runtime, rows: &mut SqliteReceiverStream) -> bool {
    rt.block_on(async {
        let mut next = std::pin::pin!(rows.next_row());
        std::future::poll_fn(|cx| Poll::Ready(next.as_mut().poll(cx).is_ready())).await
    })
}

fn two_row_connection(second: &str) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    let sql = format!("CREATE TABLE t(v); INSERT INTO t VALUES (1), ({second});");
    conn.execute_batch(&sql).unwrap();
    conn
}

#[test]
fn retried_polls_mint_one_demand_and_terminal_state_mints_none() {
    let rt = runtime();
    let (mut sink, mut rows) = bridge(true);
    for _ in 0..3 {
        assert!(!poll_once(&rt, &mut rows), "no row exists before demand");
    }
    assert_eq!(rows.issued_demands(), Some(1));
    assert!(sink.ready(None));
    assert!(sink.row(Ok(tuple("1"))));
    let row = rt.block_on(rows.next_row()).unwrap();
    let row = row.expect("demanded row");
    assert_eq!(row.values[0].as_deref(), Some("1"));
    assert_eq!(rows.issued_demands(), Some(1));

    assert!(!poll_once(&rt, &mut rows));
    assert_eq!(rows.issued_demands(), Some(2));
    assert!(sink.ready(None));
    drop(sink); // clean EOF
    for _ in 0..3 {
        assert!(rt.block_on(rows.next_row()).unwrap().is_none());
    }
    assert_eq!(rows.issued_demands(), Some(2));
}

#[test]
fn the_first_terminal_error_is_sticky_and_latches_demand() {
    let rt = runtime();
    let (mut sink, mut rows) = bridge(true);
    assert!(!poll_once(&rt, &mut rows));
    assert!(sink.ready(None));
    assert!(!sink.row(Err(Error::Emit("first".into()))));
    sink.fail(None, Error::Emit("second".into()));
    drop(sink);
    assert!(matches!(
        rt.block_on(rows.next_row()),
        Err(Error::Emit(message)) if message == "first"
    ));
    for _ in 0..3 {
        assert!(matches!(rt.block_on(rows.next_row()), Ok(None)));
    }
    assert_eq!(rows.issued_demands(), Some(1));
}

#[test]
fn a_cause_found_behind_an_unread_row_replaces_clean_eof() {
    let rt = runtime();
    let request = budget();
    let control: &dyn QueryControl = &*request;
    let (mut sink, mut rows) = bridge(true);
    assert!(!poll_once(&rt, &mut rows)); // demand outstanding, future dropped
    assert!(sink.ready(Some(control)));
    assert!(sink.row(Ok(tuple("1"))));
    request.terminate(QueryControlError::Cancelled);
    assert!(!sink.ready(Some(control)));
    drop(sink);

    let row = rt.block_on(rows.next_row()).unwrap();
    let row = row.expect("buffered row");
    assert_eq!(row.values[0].as_deref(), Some("1"));
    assert!(matches!(
        rt.block_on(rows.next_row()),
        Err(Error::QueryControl(QueryControlError::Cancelled))
    ));
    assert!(matches!(rt.block_on(rows.next_row()), Ok(None)));
    assert_eq!(rows.issued_demands(), Some(2));
}

#[test]
fn an_idle_worker_observes_termination_while_the_receiver_is_retained() {
    for reason in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        let rt = runtime();
        let request = budget();
        let (mut sink, mut rows) = bridge(true);
        let (done_tx, done_rx) = mpsc::channel();
        let worker = {
            let request = request.clone();
            std::thread::spawn(move || {
                let control: &dyn QueryControl = &*request;
                let _ = done_tx.send(sink.ready(Some(control)));
            })
        };
        request.terminate(reason);
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(2)), Ok(false));
        worker.join().unwrap();
        assert!(matches!(
            rt.block_on(rows.next_row()),
            Err(Error::QueryControl(actual)) if actual == reason
        ));
        assert!(matches!(rt.block_on(rows.next_row()), Ok(None)));
    }
}

#[test]
fn dropping_the_consumer_releases_an_idle_worker() {
    let (mut sink, rows) = bridge(true);
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = done_tx.send(sink.ready(None));
    });
    drop(rows);
    assert_eq!(done_rx.recv_timeout(Duration::from_secs(2)), Ok(false));
    worker.join().unwrap();
}

#[test]
fn full_scan_bridge_mints_no_demand() {
    let (mut sink, rows) = bridge(false);
    assert!(sink.ready(None));
    assert!(sink.row(Ok(tuple("1")))); // buffered with no consumer call
    assert_eq!(rows.issued_demands(), None);
}

/// Source work spent by one early-stop open that reads `read` rows, is dropped,
/// and whose worker has then released admission.
async fn early_stop_work(second: &str, read: usize) -> u64 {
    let member = SqliteOwnedConnection::new(two_row_connection(second));
    let control = budget();
    let lease = member.acquire().await.unwrap();
    let mut backend = SqliteOwnedBackend::new_controlled_leased(lease, control.clone());
    let mut rows = backend
        .open_branch_with_demand(TWO_ROWS, &[], None, false, false, true)
        .await
        .unwrap();
    for _ in 0..read {
        assert!(rows.next_row().await.unwrap().is_some());
    }
    drop(rows);
    drop(backend);
    let _next = tokio::time::timeout(Duration::from_secs(2), member.acquire())
        .await
        .expect("worker did not release admission")
        .expect("admission unexpectedly closed");
    control.consumed(QueryCharge::SourceWork)
}

#[tokio::test]
async fn early_stop_decodes_and_charges_only_demanded_rows() {
    // REAL's lexical bound (327) exceeds INTEGER's (20): a decoded second row
    // shows exactly that difference, and an undecoded one shows none.
    for read in [0, 1] {
        assert_eq!(
            early_stop_work("0.5", read).await,
            early_stop_work("7", read).await,
            "an undemanded second row was decoded after reading {read} rows"
        );
    }
    let real = early_stop_work("0.5", 2).await;
    assert_eq!(real - early_stop_work("7", 2).await, 327 - 20);
}

#[tokio::test]
async fn idle_early_stop_worker_releases_admission_on_termination() {
    for reason in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        let member = SqliteOwnedConnection::new(two_row_connection("2"));
        let control = budget();
        let lease = member.acquire().await.unwrap();
        let mut backend = SqliteOwnedBackend::new_controlled_leased(lease, control.clone());
        let mut rows = backend
            .open_branch_with_demand(TWO_ROWS, &[], None, false, false, true)
            .await
            .unwrap();
        assert!(rows.next_row().await.unwrap().is_some());
        drop(backend);
        control.terminate(reason);
        // The stream stays retained and unpolled; only the worker's own
        // control check can end its wait and release admission.
        let _next = tokio::time::timeout(Duration::from_secs(2), member.acquire())
            .await
            .expect("idle worker ignored termination")
            .expect("admission unexpectedly closed");
        assert!(matches!(
            rows.next_row().await,
            Err(Error::QueryControl(actual)) if actual == reason
        ));
        assert!(matches!(rows.next_row().await, Ok(None)));
        assert_eq!(rows.issued_demands(), Some(2));
    }
}

#[tokio::test]
async fn setup_error_needs_no_demand_and_frees_admission_with_receiver_retained() {
    // Both bridge modes: full-scan prefetch and early-stop consumer demand.
    for early_stop in [false, true] {
        let member = SqliteOwnedConnection::new(two_row_connection("2"));
        let lease = member.acquire().await.unwrap();
        let mut backend = SqliteOwnedBackend::new_controlled_leased(lease, budget());
        let mut rows = backend
            .open_branch_with_demand("SELECT * FROM", &[], None, false, false, early_stop)
            .await
            .unwrap();
        drop(backend);
        // The receiver stays retained and unpolled: the worker must deliver
        // its setup error without a demand, exit, and release admission.
        let lease = tokio::time::timeout(Duration::from_secs(2), member.acquire())
            .await
            .expect("setup-error worker did not release admission")
            .expect("admission unexpectedly closed");
        assert_eq!(rows.issued_demands(), early_stop.then_some(0));

        // Admission is usable again while the stale receiver is still held.
        let mut next = SqliteOwnedBackend::new_controlled_leased(lease, budget());
        let mut fresh = next
            .open_branch_with_demand(TWO_ROWS, &[], None, false, false, early_stop)
            .await
            .unwrap();
        assert!(fresh.next_row().await.unwrap().is_some());
        drop(fresh);
        drop(next);

        assert!(matches!(rows.next_row().await, Err(Error::Sqlite(_))));
        assert!(matches!(rows.next_row().await, Ok(None)));
        assert_eq!(rows.issued_demands(), early_stop.then_some(1));
    }
}

#[tokio::test]
async fn full_scan_open_still_prefetches_before_any_pull() {
    let (decoded, first_decode) = mpsc::sync_channel(2);
    let observer = SqliteCancellationObserver::new(Arc::new(move |event| {
        if event == SqliteCancellationEvent::BeforeRowSend {
            let _ = decoded.try_send(());
        }
    }));
    let conn = Arc::new(Mutex::new(two_row_connection("2")));
    let mut backend = SqliteOwnedBackend::new_controlled_observed(conn, budget(), observer);
    let rows = backend
        .open_branch_with_demand(TWO_ROWS, &[], None, false, false, false)
        .await
        .unwrap();
    assert_eq!(rows.issued_demands(), None);
    first_decode
        .recv_timeout(Duration::from_secs(2))
        .expect("a full scan decodes its first row before any pull");
    drop(rows);
}
