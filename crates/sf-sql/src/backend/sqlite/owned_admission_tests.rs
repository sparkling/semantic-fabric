//! Exact unit proofs for the per-physical-connection serving lease.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use rusqlite::Connection;
use sf_core::query_control::{QueryBudget, QueryLimits};

use crate::backend::SqlBackend;

use super::super::cancellation::{SqliteCancellationEvent, SqliteCancellationObserver};
use super::{SqliteOwnedBackend, SqliteOwnedConnection};

fn control() -> Arc<QueryBudget> {
    Arc::new(QueryBudget::new(QueryLimits::new(
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
