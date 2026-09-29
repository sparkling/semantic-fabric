//! Exact unit proofs for the per-physical-connection serving lease.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use rusqlite::Connection;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};

use crate::backend::{BranchStream, SqlBackend};

use super::super::cancellation::{SqliteCancellationEvent, SqliteCancellationObserver};
use super::{SqliteOwnedBackend, SqliteOwnedConnection};

fn control() -> Arc<QueryBudget> {
    Arc::new(QueryBudget::new(QueryLimits::new(
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    )))
}

#[derive(Default)]
struct MutexBarrier {
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}

impl MutexBarrier {
    fn observer(self: &Arc<Self>) -> SqliteCancellationObserver {
        let barrier = Arc::clone(self);
        SqliteCancellationObserver::new(Arc::new(move |event| {
            if event != SqliteCancellationEvent::MutexAcquired {
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

    fn wait_until_entered(&self) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (state, timeout) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(2), |state| !state.0)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(!timeout.timed_out(), "SQLite worker did not acquire mutex");
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

struct ReleaseOnDrop(Arc<MutexBarrier>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[tokio::test]
async fn admission_identity_is_cap_one_and_per_physical_connection() {
    let first = SqliteOwnedConnection::new(Connection::open_in_memory().unwrap());
    let same = first.clone();
    let other = SqliteOwnedConnection::new(Connection::open_in_memory().unwrap());

    assert_eq!(first.available_permits(), 1);
    let lease = first.acquire().await.unwrap();
    assert_eq!(first.available_permits(), 0);
    assert_eq!(same.available_permits(), 0);
    assert_eq!(other.available_permits(), 1);

    drop(lease);
    assert_eq!(same.available_permits(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn column_names_worker_keeps_lease_after_async_caller_is_aborted() {
    let member = SqliteOwnedConnection::new(Connection::open_in_memory().unwrap());
    let lease = member.acquire().await.unwrap();
    let barrier = Arc::new(MutexBarrier::default());
    let release = ReleaseOnDrop(Arc::clone(&barrier));
    let mut backend =
        SqliteOwnedBackend::new_controlled_leased_observed(lease, control(), barrier.observer());
    let task = tokio::spawn(async move { backend.column_names("SELECT 1").await });
    barrier.wait_until_entered();

    task.abort();
    let _ = task.await;
    assert_eq!(member.available_permits(), 0);

    drop(release);
    let reacquired = tokio::time::timeout(Duration::from_secs(2), member.acquire())
        .await
        .expect("worker did not release admission")
        .expect("admission unexpectedly closed");
    drop(reacquired);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_branch_worker_keeps_lease_after_stream_and_backend_drop() {
    let member = SqliteOwnedConnection::new(Connection::open_in_memory().unwrap());
    let lease = member.acquire().await.unwrap();
    let barrier = Arc::new(MutexBarrier::default());
    let release = ReleaseOnDrop(Arc::clone(&barrier));
    let mut backend =
        SqliteOwnedBackend::new_controlled_leased_observed(lease, control(), barrier.observer());
    let stream = backend.open_branch("SELECT 1", &[]).await.unwrap();
    barrier.wait_until_entered();

    drop(stream);
    drop(backend);
    assert_eq!(member.available_permits(), 0);

    drop(release);
    let reacquired = tokio::time::timeout(Duration::from_secs(2), member.acquire())
        .await
        .expect("worker did not release admission")
        .expect("admission unexpectedly closed");
    drop(reacquired);
}

struct ControlDropProbe {
    member: SqliteOwnedConnection,
    observed: Option<tokio::sync::oneshot::Sender<usize>>,
    reject: bool,
}

impl QueryControl for ControlDropProbe {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        if self.reject {
            Err(QueryControlError::Cancelled)
        } else {
            Ok(())
        }
    }
    fn consume(&self, _: QueryCharge, _: u64) -> Result<(), QueryControlError> {
        self.checkpoint()
    }
    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        reason
    }
}

impl Drop for ControlDropProbe {
    fn drop(&mut self) {
        let _ = self
            .observed
            .take()
            .unwrap()
            .send(self.member.available_permits());
    }
}

#[tokio::test]
async fn final_backend_control_drops_before_admission_reopens() {
    let member = SqliteOwnedConnection::new(Connection::open_in_memory().unwrap());
    let (tx, rx) = tokio::sync::oneshot::channel();
    let backend = SqliteOwnedBackend::new_controlled_leased(
        member.acquire().await.unwrap(),
        Arc::new(ControlDropProbe {
            member: member.clone(),
            observed: Some(tx),
            reject: false,
        }),
    );
    drop(backend);
    assert_eq!(
        rx.await.unwrap(),
        0,
        "request state must drop while permit is still held"
    );
    assert_eq!(member.available_permits(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn final_worker_control_drops_before_admission_on_every_terminal_path() {
    // Both bridge modes: full-scan prefetch and early-stop consumer demand.
    for early_stop in [false, true] {
        for (sql, receiver_drop, reject) in [
            ("SELECT 1 UNION ALL SELECT 2", false, false),
            ("SELECT 1 UNION ALL SELECT 2", true, false),
            ("SELECT missing FROM absent", false, false),
            ("SELECT json_extract('invalid-json', '$')", false, false),
            ("SELECT 1", false, true),
        ] {
            let member = SqliteOwnedConnection::new(Connection::open_in_memory().unwrap());
            let (tx, rx) = tokio::sync::oneshot::channel();
            let probe = Arc::new(ControlDropProbe {
                member: member.clone(),
                observed: Some(tx),
                reject,
            });
            let barrier = Arc::new(MutexBarrier::default());
            let release = ReleaseOnDrop(Arc::clone(&barrier));
            let mut backend = SqliteOwnedBackend::new_controlled_leased_observed(
                member.acquire().await.unwrap(),
                probe,
                barrier.observer(),
            );
            let mut stream = backend
                .open_branch_with_demand(sql, &[], None, false, false, early_stop)
                .await
                .unwrap();
            barrier.wait_until_entered();
            drop(backend); // only the worker now owns request control and admission
            drop(release);
            if !receiver_drop {
                while matches!(stream.next_row().await, Ok(Some(_))) {}
            }
            drop(stream);
            let permits = tokio::time::timeout(Duration::from_secs(2), rx)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                permits, 0,
                "{sql} (early_stop {early_stop}): control outlived admission"
            );
            let _next = tokio::time::timeout(Duration::from_secs(2), member.acquire())
                .await
                .unwrap()
                .unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn final_metadata_control_drops_before_admission_after_caller_abort() {
    for result_columns in [false, true] {
        let member = SqliteOwnedConnection::new(Connection::open_in_memory().unwrap());
        let (tx, rx) = tokio::sync::oneshot::channel();
        let probe = Arc::new(ControlDropProbe {
            member: member.clone(),
            observed: Some(tx),
            reject: false,
        });
        let barrier = Arc::new(MutexBarrier::default());
        let release = ReleaseOnDrop(Arc::clone(&barrier));
        let mut backend = SqliteOwnedBackend::new_controlled_leased_observed(
            member.acquire().await.unwrap(),
            probe,
            barrier.observer(),
        );
        let task = tokio::spawn(async move {
            if result_columns {
                backend.result_columns("SELECT 1").await.map(|_| ())
            } else {
                backend.column_names("SELECT 1").await.map(|_| ())
            }
        });
        barrier.wait_until_entered();
        task.abort();
        let _ = task.await;
        drop(release);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), rx)
                .await
                .unwrap()
                .unwrap(),
            0
        );
        let _next = tokio::time::timeout(Duration::from_secs(2), member.acquire())
            .await
            .unwrap()
            .unwrap();
    }
}
