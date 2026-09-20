use super::super::owned::SqliteOwnedConnection;
use super::*;
use crate::backend::{BranchStream, SqlBackend};
use rusqlite::Connection;
use sf_core::query_control::{QueryBudget, QueryControlError, QueryLimits};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

struct Database(PathBuf);

impl Database {
    fn new(journal: &str) -> (Self, SqliteOwnedConnection, Connection) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sf-generation-{}-{}.sqlite",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(&format!(
            "PRAGMA journal_mode={journal}; CREATE TABLE items(value INTEGER); INSERT INTO items VALUES (7),(7);"
        )).unwrap();
        let writer = Connection::open(&path).unwrap();
        writer.busy_timeout(Duration::ZERO).unwrap();
        (Self(path), SqliteOwnedConnection::new(conn), writer)
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        for path in [
            self.0.clone(),
            self.0.with_extension("sqlite-wal"),
            self.0.with_extension("sqlite-shm"),
        ] {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn budget() -> Arc<QueryBudget> {
    Arc::new(QueryBudget::new(QueryLimits::new(
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    )))
}

async fn begin(member: &SqliteOwnedConnection) -> VerifiedSqliteGenerationLease {
    VerifiedSqliteGenerationLease::begin(member.acquire().await.unwrap(), budget(), None)
        .await
        .unwrap()
}

async fn reacquire(member: &SqliteOwnedConnection) -> SqliteOwnedLease {
    tokio::time::timeout(Duration::from_secs(5), member.acquire())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn wal_ddl_preserves_old_read_snapshot_and_rejects_next_generation() {
    let (_db, member, writer) = Database::new("WAL");
    let generation = begin(&member).await;
    let expected = generation.schema().clone();
    writer
        .execute_batch("ALTER TABLE items ADD COLUMN label TEXT; UPDATE items SET value=9;")
        .unwrap();
    let mut backend = generation.backend().unwrap();
    assert_eq!(
        backend.column_names("SELECT * FROM items").await.unwrap(),
        ["value"]
    );
    let mut stream = backend
        .open_branch("SELECT value FROM items", &[])
        .await
        .unwrap();
    let first = stream.next_row().await.unwrap().unwrap();
    let second = stream.next_row().await.unwrap().unwrap();
    assert_eq!(first.values, [Some("7".into())]);
    assert_eq!(first.values, second.values);
    assert_eq!(first.codes, second.codes);
    assert!(stream.next_row().await.unwrap().is_none());
    drop(stream);
    drop(backend);
    generation.finish().await.unwrap();
    assert!(VerifiedSqliteGenerationLease::begin(
        reacquire(&member).await,
        budget(),
        Some(expected)
    )
    .await
    .is_err());
    let current = begin(&member).await;
    assert_eq!(current.schema().tables()[0].columns.len(), 2);
    current.finish().await.unwrap();
}

#[tokio::test]
async fn rollback_journal_blocks_ddl_until_acknowledged_cleanup() {
    let (_db, member, writer) = Database::new("DELETE");
    let generation = begin(&member).await;
    assert!(writer
        .execute_batch("ALTER TABLE items ADD COLUMN label TEXT")
        .is_err());
    generation.finish().await.unwrap();
    writer
        .execute_batch("ALTER TABLE items ADD COLUMN label TEXT")
        .unwrap();
}

#[tokio::test]
async fn closing_waits_for_registered_queued_work_and_rejects_new_work() {
    let (_db, member, _writer) = Database::new("WAL");
    let generation = begin(&member).await;
    let state = generation.state.as_ref().unwrap().clone();
    let queued = state.register().unwrap();
    let mut backend = generation.backend().unwrap();
    let finish = tokio::spawn(generation.finish());
    while !state.operations.lock().unwrap().closing {
        tokio::task::yield_now().await;
    }
    assert!(state.register().is_err());
    assert!(backend
        .column_names("SELECT value FROM items")
        .await
        .is_err());
    assert!(!finish.is_finished());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), member.acquire())
            .await
            .is_err()
    );
    drop(queued);
    finish.await.unwrap().unwrap();
    drop(backend);
    drop(state);
    drop(reacquire(&member).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_queued_metadata_worker_runs_before_rollback() {
    let (_db, member, writer) = Database::new("WAL");
    let generation = begin(&member).await;
    let state = generation.state.as_ref().unwrap().clone();
    let mut rows_backend = generation.backend().unwrap();
    let mut metadata_backend = generation.backend().unwrap();
    let mut stream = rows_backend
        .open_branch("SELECT value FROM items", &[])
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while state.lease.conn.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // The first worker holds the mutex while blocked on the cap-one row channel.
    writer
        .execute_batch("ALTER TABLE items ADD COLUMN label TEXT")
        .unwrap();
    let metadata =
        tokio::spawn(async move { metadata_backend.column_names("SELECT * FROM items").await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while state.operations.lock().unwrap().active != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let finish = tokio::spawn(generation.finish());
    while !state.operations.lock().unwrap().closing {
        tokio::task::yield_now().await;
    }
    assert!(!finish.is_finished());
    for _ in 0..2 {
        assert_eq!(
            stream.next_row().await.unwrap().unwrap().values,
            [Some("7".into())]
        );
    }
    assert!(stream.next_row().await.unwrap().is_none());
    drop(stream);
    drop(rows_backend);
    assert_eq!(metadata.await.unwrap().unwrap(), ["value"]);
    finish.await.unwrap().unwrap();
    drop(state);
    drop(reacquire(&member).await);
}

#[tokio::test]
async fn cancelled_finish_and_dropped_owner_still_acknowledge_rollback_before_reuse() {
    let (_db, member, _writer) = Database::new("WAL");
    for abort_finish in [false, true] {
        let generation = begin(&member).await;
        let state = generation.state.as_ref().unwrap().clone();
        let queued = state.register().unwrap();
        if abort_finish {
            let finish = tokio::spawn(generation.finish());
            while !state.operations.lock().unwrap().closing {
                tokio::task::yield_now().await;
            }
            finish.abort();
            let _ = finish.await;
        } else {
            drop(generation);
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(20), member.acquire())
                .await
                .is_err()
        );
        drop(queued);
        drop(state);
        let lease = reacquire(&member).await;
        assert!(lease.state.conn.lock().unwrap().is_autocommit());
        drop(lease);
    }
}

#[tokio::test]
async fn deadline_rolls_back_but_never_reports_success() {
    let (_db, member, _writer) = Database::new("WAL");
    let control = budget();
    let generation =
        VerifiedSqliteGenerationLease::begin(reacquire(&member).await, control.clone(), None)
            .await
            .unwrap();
    control.terminate(QueryControlError::DeadlineExceeded);
    assert!(matches!(
        generation.finish().await,
        Err(Error::QueryControl(QueryControlError::DeadlineExceeded))
    ));
    drop(reacquire(&member).await);
}

#[tokio::test]
async fn cancellation_before_begin_recycles_only_autocommit_members() {
    for existing_transaction in [false, true] {
        let (_db, member, _writer) = Database::new("WAL");
        let lease = reacquire(&member).await;
        if existing_transaction {
            lease
                .state
                .conn
                .lock()
                .unwrap()
                .execute_batch("BEGIN DEFERRED")
                .unwrap();
        }
        let control = budget();
        // Admission was acquired, but the blocking BEGIN worker has not run yet.
        control.terminate(QueryControlError::DeadlineExceeded);
        let result = VerifiedSqliteGenerationLease::begin(lease, control, None).await;
        if existing_transaction {
            assert!(result.is_err());
            assert!(member.acquire().await.is_err());
        } else {
            assert!(matches!(
                result,
                Err(Error::QueryControl(QueryControlError::DeadlineExceeded))
            ));
            let lease = reacquire(&member).await;
            assert!(lease.state.conn.lock().unwrap().is_autocommit());
            drop(lease);
            begin(&member).await.finish().await.unwrap();
        }
    }
}

#[tokio::test]
async fn worker_panic_and_failed_rollback_poison_admission() {
    for panic_worker in [false, true] {
        let (_db, member, _writer) = Database::new("WAL");
        let generation = begin(&member).await;
        if panic_worker {
            let operation = generation.state.as_ref().unwrap().register().unwrap();
            assert!(tokio::task::spawn_blocking(move || {
                let _operation = operation;
                panic!("injected worker panic");
            })
            .await
            .is_err());
        } else {
            generation
                .state
                .as_ref()
                .unwrap()
                .lease
                .conn
                .lock()
                .unwrap()
                .execute_batch("ROLLBACK")
                .unwrap();
        }
        assert!(generation.finish().await.is_err());
        assert!(member.acquire().await.is_err());
    }
}

#[tokio::test]
async fn profile_rejects_memory_attached_and_temporary_objects() {
    let memory = SqliteOwnedConnection::new(Connection::open_in_memory().unwrap());
    assert!(
        VerifiedSqliteGenerationLease::begin(reacquire(&memory).await, budget(), None)
            .await
            .is_err()
    );
    for sql in [
        "ATTACH ':memory:' AS other",
        "CREATE TEMP TABLE shadow(x)",
        "PRAGMA read_uncommitted=1",
    ] {
        let (_db, member, _writer) = Database::new("WAL");
        member
            .raw_connection()
            .lock()
            .unwrap()
            .execute_batch(sql)
            .unwrap();
        assert!(
            VerifiedSqliteGenerationLease::begin(reacquire(&member).await, budget(), None)
                .await
                .is_err()
        );
        assert!(reacquire(&member)
            .await
            .state
            .conn
            .lock()
            .unwrap()
            .is_autocommit());
    }
}

#[tokio::test]
async fn unqualified_journal_mode_cannot_mint_a_generation() {
    let (_db, member, _writer) = Database::new("TRUNCATE");
    assert!(
        VerifiedSqliteGenerationLease::begin(reacquire(&member).await, budget(), None)
            .await
            .is_err()
    );
    assert!(reacquire(&member)
        .await
        .state
        .conn
        .lock()
        .unwrap()
        .is_autocommit());
}

#[tokio::test]
async fn zero_source_work_rejects_observation_and_restores_a_clean_member() {
    let (_db, member, _writer) = Database::new("WAL");
    let control = Arc::new(QueryBudget::new(QueryLimits::new(
        u64::MAX,
        0,
        u64::MAX,
        u64::MAX,
    )));
    let result =
        VerifiedSqliteGenerationLease::begin(reacquire(&member).await, control, None).await;
    assert!(matches!(
        result,
        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
    ));
    let lease = reacquire(&member).await;
    assert!(lease.state.conn.lock().unwrap().is_autocommit());
}

#[tokio::test]
async fn unchanged_compiler_columns_do_not_hide_index_drift() {
    let (_db, member, writer) = Database::new("WAL");
    let generation = begin(&member).await;
    let before = generation.schema().clone();
    generation.finish().await.unwrap();
    writer
        .execute_batch("CREATE INDEX idx_items ON items(value)")
        .unwrap();
    let generation = begin(&member).await;
    assert_eq!(before.tables(), generation.schema().tables());
    assert_ne!(before.as_ref(), generation.schema().as_ref());
    generation.finish().await.unwrap();
    assert!(
        VerifiedSqliteGenerationLease::begin(reacquire(&member).await, budget(), Some(before))
            .await
            .is_err()
    );
}

#[test]
fn missing_runtime_on_drop_poison_closes_the_sealed_member() {
    let (_db, member, _writer) = Database::new("WAL");
    let sealed = SqliteGenerationConnection(member);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let generation = runtime.block_on(sealed.begin(budget(), None)).unwrap();
    drop(runtime);
    drop(generation);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    assert!(runtime.block_on(sealed.begin(budget(), None)).is_err());
}

#[tokio::test]
async fn sealed_factory_rejects_lock_bypassing_uris_and_holds_real_delete_locks() {
    let (db, _member, writer) = Database::new("DELETE");
    for option in ["immutable=1", "nolock=1", "vfs=unix-none"] {
        assert!(
            SqliteGenerationConnection::open(format!("file:{}?{option}", db.0.display())).is_err()
        );
    }
    let member = SqliteGenerationConnection::open(&db.0).unwrap();
    let lease = member.begin(budget(), None).await.unwrap();
    assert!(writer
        .execute_batch("ALTER TABLE items ADD COLUMN label TEXT")
        .is_err());
    lease.finish().await.unwrap();
    writer
        .execute_batch("ALTER TABLE items ADD COLUMN label TEXT")
        .unwrap();
}
