//! Red integration test for one shared exec-core/SQLite-worker control identity.
//!
//! This covers the leased serving path's active-VM contract. Admission waiting
//! happens before this entry point; the test does not claim cancellation of the
//! raw mutex, a submitted blocking task, blocking UDF/VFS/I/O, busy-timeout,
//! compiler work, another backend, total M2, or post-200 atomic delivery.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};
use sf_sparql::{exec, parse_and_translate, Error};
use sf_sql::backend::sqlite::SqliteOwnedConnection;
use sf_sql::Dialect;

const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#Slow> a rr:TriplesMap ;
  rr:logicalTable [ rr:sqlQuery """
    WITH RECURSIVE counter(value) AS (
      VALUES(0)
      UNION ALL
      SELECT value + 1 FROM counter WHERE value < 5000000
    )
    SELECT value AS id, value FROM counter WHERE value = 5000000
  """ ] ;
  rr:subjectMap [ rr:template "http://example.test/item/{id}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:value ;
    rr:objectMap [ rr:column "value" ]
  ] .
"#;

#[derive(Debug)]
struct IdentityControl {
    source_charges: AtomicUsize,
    executor_thread: Mutex<Option<std::thread::ThreadId>>,
    worker_observed_armed_identity: AtomicBool,
}

impl IdentityControl {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            source_charges: AtomicUsize::new(0),
            executor_thread: Mutex::new(None),
            worker_observed_armed_identity: AtomicBool::new(false),
        })
    }
}

impl QueryControl for IdentityControl {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        if self.source_charges.load(Ordering::Acquire) < 3 {
            return Ok(());
        }
        let executor_thread = *self
            .executor_thread
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if executor_thread.is_some_and(|id| id != std::thread::current().id()) {
            self.worker_observed_armed_identity
                .store(true, Ordering::Release);
            Err(QueryControlError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), QueryControlError> {
        if charge == QueryCharge::SourceWork {
            *self
                .executor_thread
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(std::thread::current().id());
            self.source_charges.fetch_add(
                usize::try_from(amount).expect("test charge fits usize"),
                Ordering::AcqRel,
            );
        }
        self.checkpoint()
    }

    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        reason
    }
}

#[tokio::test]
async fn exec_core_and_sqlite_worker_use_the_exact_same_control_identity() {
    let maps = sf_mapping::parse_r2rml(MAPPING).expect("parse mapping");
    let plan = parse_and_translate(
        "ASK { ?item <http://example.test/value> ?value }",
        &maps,
        Dialect::Sqlite,
    )
    .expect("translate query");
    let conn = SqliteOwnedConnection::new(Connection::open_in_memory().unwrap());
    let lease = conn.acquire().await.expect("acquire serving lease");
    let control = IdentityControl::new();
    let shared: Arc<dyn QueryControl> = control.clone();

    let error = exec::ask_sqlite_owned_interruptible_leased(&plan, lease, shared)
        .await
        .expect_err("worker callback must observe executor's armed identity");

    assert!(matches!(
        error,
        Error::QueryControl(QueryControlError::Cancelled)
    ));
    assert!(
        control
            .worker_observed_armed_identity
            .load(Ordering::Acquire),
        "a fresh control/deadline in the worker cannot observe exec-core state"
    );
}
