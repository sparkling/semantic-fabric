//! Red tests for active SQLite VM cancellation only.
//!
//! Nonclaims: mutex-wait cancellation; blocking UDF, VFS, or I/O cancellation;
//! SQLite busy-timeout policy; other backends; compiler work; total M2; and
//! atomicity after an HTTP 200 response.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use rusqlite::{Connection, ErrorCode};
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};

use crate::backend::{BranchStream, SqlBackend};
use crate::error::Error;

use super::super::cancellation::{SqliteCancellationEvent, SqliteCancellationObserver};
use super::SqliteOwnedBackend;

const LONG_QUERY: &str = "WITH RECURSIVE counter(value) AS (\
    VALUES(0) UNION ALL SELECT value + 1 FROM counter WHERE value < 5000000\
) SELECT value FROM counter WHERE value = 5000000";

fn budget() -> Arc<QueryBudget> {
    Arc::new(QueryBudget::new(QueryLimits::new(
        u64::MAX,
        u64::MAX,
        u64::MAX,
    )))
}

fn assert_control_error(error: Error, expected: QueryControlError) {
    match error {
        Error::QueryControl(actual) => assert_eq!(actual, expected),
        other => panic!("expected {expected:?}, got {other:?}"),
    }
}

async fn first_error(mut backend: SqliteOwnedBackend, sql: &str) -> Error {
    let mut rows = backend.open_branch(sql, &[]).await.expect("open branch");
    match rows.next_row().await {
        Err(error) => error,
        Ok(_) => panic!("controlled worker must report an error"),
    }
}

#[derive(Default)]
struct EventRecorder {
    events: Mutex<Vec<SqliteCancellationEvent>>,
    changed: Condvar,
}

impl EventRecorder {
    fn observer(self: &Arc<Self>) -> SqliteCancellationObserver {
        let recorder = self.clone();
        SqliteCancellationObserver::new(Arc::new(move |event| {
            recorder
                .events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(event);
            recorder.changed.notify_all();
        }))
    }

    fn wait_for(&self, event: SqliteCancellationEvent) {
        let events = self
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (events, timeout) = self
            .changed
            .wait_timeout_while(events, Duration::from_secs(2), |events| {
                !events.contains(&event)
            })
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(!timeout.timed_out(), "event not observed: {event:?}");
        assert!(events.contains(&event));
    }

    fn last(&self) -> Option<SqliteCancellationEvent> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .last()
            .copied()
    }
}

#[derive(Debug)]
struct ArmedControl {
    armed: AtomicBool,
    reason: QueryControlError,
}

impl ArmedControl {
    fn new(reason: QueryControlError) -> Arc<Self> {
        Arc::new(Self {
            armed: AtomicBool::new(false),
            reason,
        })
    }
}

impl QueryControl for ArmedControl {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        if self.armed.load(Ordering::Acquire) {
            Err(self.reason)
        } else {
            Ok(())
        }
    }

    fn consume(&self, _charge: QueryCharge, _amount: u64) -> Result<(), QueryControlError> {
        self.checkpoint()
    }
}

fn stage_observer(
    control: Arc<ArmedControl>,
    recorder: &Arc<EventRecorder>,
    target: SqliteCancellationEvent,
    occurrence: usize,
) -> SqliteCancellationObserver {
    let recorder_observer = recorder.observer();
    let seen = Arc::new(AtomicUsize::new(0));
    SqliteCancellationObserver::new(Arc::new(move |event| {
        recorder_observer.observe(event);
        if event == target && seen.fetch_add(1, Ordering::AcqRel) + 1 == occurrence {
            control.armed.store(true, Ordering::Release);
        }
    }))
}

#[tokio::test]
async fn cancellation_interrupts_recursive_sqlite_vm_and_preserves_exact_cause() {
    let control = budget();
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let recorder = Arc::new(EventRecorder::default());
    let backend =
        SqliteOwnedBackend::new_controlled_observed(conn, control.clone(), recorder.observer());
    let terminator = {
        let control = control.clone();
        let recorder = recorder.clone();
        std::thread::spawn(move || {
            recorder.wait_for(SqliteCancellationEvent::ProgressCallbackEntered);
            control.terminate(QueryControlError::Cancelled);
        })
    };

    let error = first_error(backend, LONG_QUERY).await;
    terminator.join().unwrap();
    assert_control_error(error, QueryControlError::Cancelled);
}

#[tokio::test]
async fn deadline_interrupts_recursive_sqlite_vm_and_preserves_exact_cause() {
    let control = budget();
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let recorder = Arc::new(EventRecorder::default());
    let backend =
        SqliteOwnedBackend::new_controlled_observed(conn, control.clone(), recorder.observer());
    let terminator = {
        let control = control.clone();
        let recorder = recorder.clone();
        std::thread::spawn(move || {
            recorder.wait_for(SqliteCancellationEvent::ProgressCallbackEntered);
            control.terminate(QueryControlError::DeadlineExceeded);
        })
    };

    let error = first_error(backend, LONG_QUERY).await;
    terminator.join().unwrap();
    assert_control_error(error, QueryControlError::DeadlineExceeded);
}

#[tokio::test]
async fn progress_callbacks_do_not_charge_source_work() {
    let control = budget();
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let mut backend = SqliteOwnedBackend::new_controlled(conn, control.clone());
    let mut rows = backend
        .open_branch(
            "WITH RECURSIVE c(x) AS (VALUES(0) UNION ALL \
             SELECT x + 1 FROM c WHERE x < 10000) SELECT max(x) FROM c",
            &[],
        )
        .await
        .unwrap();
    assert!(rows.next_row().await.unwrap().is_some());
    assert!(rows.next_row().await.unwrap().is_none());

    assert_eq!(control.consumed(QueryCharge::SourceWork), 0);
}

#[tokio::test]
async fn unrelated_sqlite_interrupt_remains_a_driver_error() {
    let control = budget();
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let interrupt = conn
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get_interrupt_handle();
    let backend = SqliteOwnedBackend::new_controlled(conn, control);
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let interrupter = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                interrupt.interrupt();
                std::thread::yield_now();
            }
        })
    };

    let error = first_error(backend, LONG_QUERY).await;
    stop.store(true, Ordering::Release);
    interrupter.join().unwrap();
    match error {
        Error::Sqlite(rusqlite::Error::SqliteFailure(code, _)) => {
            assert_eq!(code.code, ErrorCode::OperationInterrupted);
        }
        other => panic!("unrelated interrupt was misclassified: {other:?}"),
    }
}

#[tokio::test]
async fn uncontrolled_backend_does_not_replace_connection_progress_handler() {
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let callback_calls = Arc::new(AtomicUsize::new(0));
    conn.lock()
        .unwrap_or_else(|p| p.into_inner())
        .progress_handler(1, {
            let callback_calls = callback_calls.clone();
            Some(move || {
                callback_calls.fetch_add(1, Ordering::SeqCst);
                true
            })
        })
        .unwrap();
    let backend = SqliteOwnedBackend::new(conn);

    let error = first_error(backend, LONG_QUERY).await;
    assert!(matches!(
        error,
        Error::Sqlite(rusqlite::Error::SqliteFailure(code, _))
            if code.code == ErrorCode::OperationInterrupted
    ));
    assert!(callback_calls.load(Ordering::SeqCst) > 0);
}

#[tokio::test]
async fn checkpoint_runs_after_mutex_acquisition() {
    let control = ArmedControl::new(QueryControlError::Cancelled);
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let recorder = Arc::new(EventRecorder::default());
    let observer = stage_observer(
        control.clone(),
        &recorder,
        SqliteCancellationEvent::MutexAcquired,
        1,
    );
    let mut backend = SqliteOwnedBackend::new_controlled_observed(conn, control, observer);

    let error = backend
        .column_names("SELECT 1")
        .await
        .expect_err("post-lock checkpoint must fail");
    assert_control_error(error, QueryControlError::Cancelled);
    assert_eq!(
        recorder.last(),
        Some(SqliteCancellationEvent::MutexAcquired)
    );
}

#[tokio::test]
async fn open_branch_checkpoint_runs_after_mutex_acquisition() {
    let control = ArmedControl::new(QueryControlError::Cancelled);
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let recorder = Arc::new(EventRecorder::default());
    let observer = stage_observer(
        control.clone(),
        &recorder,
        SqliteCancellationEvent::MutexAcquired,
        1,
    );
    let backend = SqliteOwnedBackend::new_controlled_observed(conn, control, observer);

    assert_control_error(
        first_error(backend, "SELECT 1").await,
        QueryControlError::Cancelled,
    );
    assert_eq!(
        recorder.last(),
        Some(SqliteCancellationEvent::MutexAcquired)
    );
}

#[tokio::test]
async fn checkpoint_runs_between_metadata_and_execution_preparation() {
    let control = ArmedControl::new(QueryControlError::Cancelled);
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let recorder = Arc::new(EventRecorder::default());
    let observer = stage_observer(
        control.clone(),
        &recorder,
        SqliteCancellationEvent::MetadataReady,
        1,
    );
    let backend = SqliteOwnedBackend::new_controlled_observed(conn, control, observer);

    assert_control_error(
        first_error(backend, "SELECT 1").await,
        QueryControlError::Cancelled,
    );
    assert_eq!(
        recorder.last(),
        Some(SqliteCancellationEvent::MetadataReady)
    );
}

#[tokio::test]
async fn checkpoint_runs_before_each_row_send() {
    let control = ArmedControl::new(QueryControlError::Cancelled);
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let recorder = Arc::new(EventRecorder::default());
    let observer = stage_observer(
        control.clone(),
        &recorder,
        SqliteCancellationEvent::BeforeRowSend,
        1,
    );
    let backend = SqliteOwnedBackend::new_controlled_observed(conn, control, observer);

    assert_control_error(
        first_error(backend, "SELECT 1").await,
        QueryControlError::Cancelled,
    );
    assert_eq!(
        recorder.last(),
        Some(SqliteCancellationEvent::BeforeRowSend)
    );
}

#[tokio::test]
async fn checkpoint_runs_before_second_row_send() {
    let control = ArmedControl::new(QueryControlError::Cancelled);
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let recorder = Arc::new(EventRecorder::default());
    let observer = stage_observer(
        control.clone(),
        &recorder,
        SqliteCancellationEvent::BeforeRowSend,
        2,
    );
    let mut backend = SqliteOwnedBackend::new_controlled_observed(conn, control, observer);
    let mut rows = backend
        .open_branch("SELECT 1 UNION ALL SELECT 2", &[])
        .await
        .unwrap();
    assert!(rows.next_row().await.unwrap().is_some());

    let error = match rows.next_row().await {
        Err(error) => error,
        Ok(_) => panic!("second-row checkpoint must fail"),
    };
    assert_control_error(error, QueryControlError::Cancelled);
    assert_eq!(
        recorder.last(),
        Some(SqliteCancellationEvent::BeforeRowSend)
    );
}

#[tokio::test]
async fn checkpoint_runs_before_clean_eof() {
    let control = ArmedControl::new(QueryControlError::Cancelled);
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let recorder = Arc::new(EventRecorder::default());
    let observer = stage_observer(
        control.clone(),
        &recorder,
        SqliteCancellationEvent::BeforeEof,
        1,
    );
    let mut backend = SqliteOwnedBackend::new_controlled_observed(conn, control, observer);
    let mut rows = backend.open_branch("SELECT 1", &[]).await.unwrap();
    assert!(rows.next_row().await.unwrap().is_some());

    let error = match rows.next_row().await {
        Err(error) => error,
        Ok(_) => panic!("EOF checkpoint must fail"),
    };
    assert_control_error(error, QueryControlError::Cancelled);
    assert_eq!(recorder.last(), Some(SqliteCancellationEvent::BeforeEof));
}
