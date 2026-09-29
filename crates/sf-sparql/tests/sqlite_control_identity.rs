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

const ITEMS_MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#Items> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "items" ] ;
  rr:subjectMap [ rr:template "http://example.test/item/{id}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:value ;
    rr:objectMap [ rr:column "value" ]
  ] .
"#;

/// Source work one leased ASK spends over two rows whose second value differs.
async fn ask_source_work(second_value: &str) -> u64 {
    use sf_core::query_control::{QueryBudget, QueryLimits};

    let maps = sf_mapping::parse_r2rml(ITEMS_MAPPING).expect("parse mapping");
    let plan = parse_and_translate(
        "ASK { ?item <http://example.test/value> ?value }",
        &maps,
        Dialect::Sqlite,
    )
    .expect("translate query");
    let conn = Connection::open_in_memory().unwrap();
    conn.execute(
        "CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT NOT NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO items VALUES (1, 'one'), (2, ?1)",
        [second_value],
    )
    .unwrap();
    let member = SqliteOwnedConnection::new(conn);
    let budget = Arc::new(QueryBudget::new(QueryLimits::new(
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    )));
    let shared: Arc<dyn QueryControl> = budget.clone();
    let lease = member.acquire().await.expect("acquire serving lease");
    let answer = exec::ask_sqlite_owned_interruptible_leased(&plan, lease, shared)
        .await
        .expect("ASK executes under a real budget");
    assert!(answer);
    // Reacquiring admission proves the worker finished before the total is read.
    drop(member.acquire().await.expect("worker released admission"));
    budget.consumed(QueryCharge::SourceWork)
}

/// ASK opens its branch with the early-stop demand hint: the row after its
/// first solution is never decoded, so its size cannot reach the bill. Under
/// the old prefetch bridge the second row raced ASK's return and was billed.
#[tokio::test]
async fn ask_never_decodes_a_second_row_it_does_not_read() {
    let short = ask_source_work("two").await;
    let long = ask_source_work(&"x".repeat(4096)).await;
    assert_eq!(short, long, "ASK billed a speculatively decoded second row");
}

#[test]
fn rust_group_limit_zero_answers_empty_before_any_source_probe() {
    // A base table that does not exist on the connection: any probe fails.
    let maps = sf_mapping::parse_r2rml(
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#Missing> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "missing_items" ] ;
  rr:subjectMap [ rr:template "http://example.test/item/{id}" ] ;
  rr:predicateObjectMap [ rr:predicate <http://example.test/value> ; rr:objectMap [ rr:column "value" ] ] ."#,
    )
    .expect("parse mapping");
    let conn = Connection::open_in_memory().unwrap();
    let query =
        "SELECT (COUNT(DISTINCT *) AS ?n) WHERE { ?item <http://example.test/value> ?value }";
    let probing = parse_and_translate(query, &maps, Dialect::Sqlite).expect("translate");
    assert!(
        probing.rust_group.is_some(),
        "COUNT(DISTINCT *) routes to the Rust group path"
    );
    assert!(
        exec::select(&probing, &conn).is_err(),
        "rows are needed, so the absent table errors"
    );
    let zero = parse_and_translate(&format!("{query} LIMIT 0"), &maps, Dialect::Sqlite)
        .expect("translate");
    assert!(zero.rust_group.is_some());
    let solutions = exec::select(&zero, &conn).expect("LIMIT 0 answers before any source probe");
    assert!(solutions.rows.is_empty());
}

const REF_ATOM_MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#parent> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "ref_parent" ] ;
  rr:subjectMap [ rr:template "http://example.test/n/{label}" ] .
<#child> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "ref_child" ] ;
  rr:subjectMap [ rr:template "http://example.test/n/{s}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:edge ;
    rr:objectMap [ rr:parentTriplesMap <#parent>; rr:joinCondition [ rr:child "fk"; rr:parent "k" ] ]
  ] .
"#;

async fn ref_atom_source_work(query: &str) -> u64 {
    use sf_core::query_control::{QueryBudget, QueryLimits};

    let maps = sf_mapping::parse_r2rml(REF_ATOM_MAPPING).expect("parse mapping");
    let plan = parse_and_translate(query, &maps, Dialect::Sqlite).expect("translate query");
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE ref_parent(k TEXT, label TEXT);
         CREATE TABLE ref_child(s TEXT, fk TEXT);
         INSERT INTO ref_parent VALUES ('a','target');
         INSERT INTO ref_child VALUES ('one','a'), ('two','a');",
    )
    .unwrap();
    let conn = Arc::new(Mutex::new(conn));
    let budget = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    exec::select_each_sqlite_owned_controlled(&plan, conn, &budget, |_row| async { Ok(()) })
        .await
        .expect("ref-atom query executes under a real budget");
    budget.consumed(QueryCharge::SourceWork)
}

/// Pins `ref_atom.rs`'s own single call site that passes `work` into
/// `literal_roles::resolved_controlled`, distinct from the callee's own
/// exact/scaling tests in `literal_roles.rs`. The exact total below was
/// measured with the real call in place, then re-measured (48037, a 1332-unit
/// drop) after temporarily changing that one call's `work` argument to
/// `SourceWork::new(None)` and restoring it — proving this assertion would
/// fail if that specific call site regressed, not just that some SourceWork
/// is charged somewhere in a reference-atom join.
///
/// Re-pinned 2026-09-23 from 47471 (drop to 46139): `0f23b44f` moved
/// condition rendering into `emit/condition_control.rs` with prepaid per-node
/// charges, adding 1898 legitimate units to this query. The call-site delta is
/// unchanged at 1332, so this still isolates the same call.
#[tokio::test]
async fn resolved_controlled_call_site_in_ref_atom_sql_is_pinned() {
    let total = ref_atom_source_work("SELECT ?o WHERE { ?s <http://example.test/edge> ?o }").await;
    assert_eq!(total, 49369);
}
