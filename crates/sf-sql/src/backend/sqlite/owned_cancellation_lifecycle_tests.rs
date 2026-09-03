//! Lifecycle, cleanup, and connection-reuse proofs for controlled SQLite work.

use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::Duration;

use rusqlite::Connection;
use sf_core::query_control::{QueryBudget, QueryControlError, QueryLimits};

use crate::backend::{BranchStream, SqlBackend};
use crate::error::Error;

use super::super::cancellation::{SqliteCancellationEvent, SqliteCancellationObserver};
use super::SqliteOwnedBackend;

const LONG_QUERY: &str = "WITH RECURSIVE counter(value) AS (\
    VALUES(0) UNION ALL SELECT value + 1 FROM counter WHERE value < 5000000\
) SELECT value FROM counter WHERE value = 5000000";
const ISOLATION_QUERY: &str = "WITH RECURSIVE counter(value) AS (\
    VALUES(0) UNION ALL SELECT value + 1 FROM counter WHERE value < 10000\
) SELECT value FROM counter WHERE value = 10000";

fn budget() -> Arc<QueryBudget> {
    Arc::new(QueryBudget::new(QueryLimits::new(
        u64::MAX,
        u64::MAX,
        u64::MAX,
    )))
}

#[derive(Default)]
struct CallbackLatch {
    entered: Mutex<bool>,
    changed: Condvar,
}

#[derive(Default)]
struct CallbackBarrier {
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}

impl CallbackBarrier {
    fn observer(self: &Arc<Self>) -> SqliteCancellationObserver {
        let barrier = self.clone();
        SqliteCancellationObserver::new(Arc::new(move |event| {
            if event != SqliteCancellationEvent::ProgressCallbackEntered {
                return;
            }
            let mut state = barrier
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.0 = true;
            barrier.changed.notify_all();
            while !state.1 {
                state = barrier
                    .changed
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        }))
    }

    fn wait_until_entered_then_release_after(&self, action: impl FnOnce()) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut state, timeout) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(2), |state| !state.0)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            !timeout.timed_out(),
            "SQLite progress callback was not entered"
        );
        action();
        state.1 = true;
        self.changed.notify_all();
    }
}

impl CallbackLatch {
    fn observer(self: &Arc<Self>) -> SqliteCancellationObserver {
        let latch = self.clone();
        SqliteCancellationObserver::new(Arc::new(move |event| {
            if event == SqliteCancellationEvent::ProgressCallbackEntered {
                *latch
                    .entered
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
                latch.changed.notify_all();
            }
        }))
    }

    fn wait(&self) {
        let entered = self
            .entered
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (entered, timeout) = self
            .changed
            .wait_timeout_while(entered, Duration::from_secs(2), |entered| !*entered)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            !timeout.timed_out(),
            "SQLite progress callback was not entered"
        );
        assert!(*entered);
    }
}

async fn first_error(mut backend: SqliteOwnedBackend, sql: &str) -> Error {
    let mut rows = backend.open_branch(sql, &[]).await.expect("open branch");
    match rows.next_row().await {
        Err(error) => error,
        Ok(_) => panic!("controlled worker must report an error"),
    }
}

fn plain_scalar_bounded(conn: Arc<Mutex<Connection>>, sql: &'static str) -> i64 {
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let guard = conn.lock().unwrap_or_else(|poison| poison.into_inner());
        let result = guard.query_row(sql, [], |row| row.get::<_, i64>(0));
        let _ = tx.send(result);
    });
    rx.recv_timeout(Duration::from_secs(2))
        .expect("connection reuse timed out")
        .expect("connection reuse query failed")
}

fn assert_plain_reuse_is_not_governed_by_old_control(
    conn: &Arc<Mutex<Connection>>,
    old_control: &QueryBudget,
) {
    assert_eq!(plain_scalar_bounded(conn.clone(), "SELECT 1"), 1);
    old_control.terminate(QueryControlError::Cancelled);
    assert_eq!(
        plain_scalar_bounded(
            conn.clone(),
            "WITH RECURSIVE c(x) AS (VALUES(0) UNION ALL \
             SELECT x + 1 FROM c WHERE x < 10000) SELECT max(x) FROM c",
        ),
        10000
    );
}

#[tokio::test]
async fn progress_handler_is_removed_after_success() {
    let control = budget();
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let mut backend = SqliteOwnedBackend::new_controlled(conn.clone(), control.clone());
    let mut rows = backend.open_branch("SELECT 1", &[]).await.unwrap();
    assert!(rows.next_row().await.unwrap().is_some());
    assert!(rows.next_row().await.unwrap().is_none());

    assert_plain_reuse_is_not_governed_by_old_control(&conn, &control);
}

#[tokio::test]
async fn progress_handler_is_removed_after_setup_error() {
    let control = budget();
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let backend = SqliteOwnedBackend::new_controlled(conn.clone(), control.clone());
    assert!(matches!(
        first_error(backend, "SELECT * FROM").await,
        Error::Sqlite(_)
    ));

    assert_plain_reuse_is_not_governed_by_old_control(&conn, &control);
}

#[tokio::test]
async fn progress_handler_is_removed_after_query_error() {
    let control = budget();
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let backend = SqliteOwnedBackend::new_controlled(conn.clone(), control.clone());
    assert!(matches!(
        first_error(backend, "SELECT ?1").await,
        Error::Sqlite(_)
    ));

    assert_plain_reuse_is_not_governed_by_old_control(&conn, &control);
}

#[tokio::test]
async fn progress_handler_is_removed_after_receiver_drop() {
    let control = budget();
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let mut backend = SqliteOwnedBackend::new_controlled(conn.clone(), control.clone());
    let mut rows = backend
        .open_branch("SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3", &[])
        .await
        .unwrap();
    assert!(rows.next_row().await.unwrap().is_some());
    drop(rows);

    assert_plain_reuse_is_not_governed_by_old_control(&conn, &control);
}

#[tokio::test]
async fn progress_handler_is_removed_after_controlled_interruption() {
    let control = budget();
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let latch = Arc::new(CallbackLatch::default());
    let backend = SqliteOwnedBackend::new_controlled_observed(
        conn.clone(),
        control.clone(),
        latch.observer(),
    );
    let terminator = {
        let control = control.clone();
        let latch = latch.clone();
        std::thread::spawn(move || {
            latch.wait();
            control.terminate(QueryControlError::Cancelled);
        })
    };

    assert!(matches!(
        first_error(backend, LONG_QUERY).await,
        Error::QueryControl(QueryControlError::Cancelled)
    ));
    terminator.join().unwrap();
    assert_plain_reuse_is_not_governed_by_old_control(&conn, &control);
}

#[tokio::test]
async fn column_names_success_and_error_both_release_handler_for_reuse() {
    for probe in ["SELECT 1", "SELECT * FROM"] {
        let control = budget();
        let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let mut backend = SqliteOwnedBackend::new_controlled(conn.clone(), control.clone());
        let result = backend.column_names(probe).await;
        if probe == "SELECT 1" {
            assert_eq!(result.unwrap(), vec!["1"]);
        } else {
            assert!(matches!(result, Err(Error::Sqlite(_))));
        }

        assert_plain_reuse_is_not_governed_by_old_control(&conn, &control);
    }
}

#[tokio::test]
async fn worker_unwind_removes_handler_and_releases_mutex() {
    let control = budget();
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let observer = SqliteCancellationObserver::new(Arc::new(|event| {
        if event == SqliteCancellationEvent::BeforeRowSend {
            panic!("deterministic worker unwind");
        }
    }));
    let mut backend =
        SqliteOwnedBackend::new_controlled_observed(conn.clone(), control.clone(), observer);
    let mut rows = backend.open_branch("SELECT 1", &[]).await.unwrap();
    assert!(rows.next_row().await.unwrap().is_none());

    assert_plain_reuse_is_not_governed_by_old_control(&conn, &control);
}

#[tokio::test]
async fn delayed_request_a_cancellation_cannot_interrupt_or_classify_active_request_b() {
    let request_a = budget();
    let request_b = budget();
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));

    let mut backend_a = SqliteOwnedBackend::new_controlled(conn.clone(), request_a.clone());
    let mut rows_a = backend_a.open_branch("SELECT 1", &[]).await.unwrap();
    assert!(rows_a.next_row().await.unwrap().is_some());
    assert!(rows_a.next_row().await.unwrap().is_none());

    let barrier = Arc::new(CallbackBarrier::default());
    let mut backend_b =
        SqliteOwnedBackend::new_controlled_observed(conn, request_b, barrier.observer());
    let delayed_a = {
        let request_a = request_a.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait_until_entered_then_release_after(|| {
                request_a.terminate(QueryControlError::Cancelled);
            });
        })
    };
    // The barrier itself proves B is inside its VM callback when A terminates;
    // millions of additional VM steps add no evidence and make the suite load-
    // sensitive. Ten thousand steps still cross the 1,000-op callback interval.
    let mut rows_b = backend_b.open_branch(ISOLATION_QUERY, &[]).await.unwrap();
    let row = tokio::time::timeout(Duration::from_secs(5), rows_b.next_row())
        .await
        .expect("request B timed out")
        .expect("request B was misclassified")
        .expect("request B produced no row");
    assert_eq!(row.values[0].as_deref(), Some("10000"));
    assert!(rows_b.next_row().await.unwrap().is_none());
    delayed_a.join().unwrap();
}
