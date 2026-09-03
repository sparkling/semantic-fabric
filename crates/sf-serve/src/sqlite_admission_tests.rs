//! Exact HTTP receipts for SQLite connection admission.
//!
//! A synchronous SQLite UDF holds the sole physical connection after request A
//! has entered real source work. The test-only pool observer fires only after
//! request B polls that member's admission future to `Pending`; virtual time is
//! not advanced before that receipt. A missing, over-capacity, or prematurely
//! released gate therefore fails at the observer watchdog instead of producing
//! a false timeout pass.
//!
//! These tests do not claim fairness, cancellation of blocking UDF/VFS/I/O,
//! safety for legacy raw-connection users, or native cancellation for another
//! database driver.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, Response, StatusCode};
use http_body_util::BodyExt;
use rusqlite::functions::FunctionFlags;
use sf_sparql::Tbox;
use tower::{Service, ServiceExt};

use crate::{introspect_sqlite_all, router, Backend, ServeConfig, SqlitePool};

const WAITER_TIMEOUT: Duration = Duration::from_secs(1);
const HOLDER_TIMEOUT: Duration = Duration::from_secs(3_600);

const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#Holder> a rr:TriplesMap ;
  rr:logicalTable [ rr:sqlQuery "SELECT 1 AS id, test_hold() AS value" ] ;
  rr:subjectMap [ rr:template "http://example.test/holder/{id}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:heldValue ;
    rr:objectMap [ rr:column "value" ]
  ] .
<#Probe> a rr:TriplesMap ;
  rr:logicalTable [ rr:sqlQuery "SELECT 1 AS id, test_probe() AS value" ] ;
  rr:subjectMap [ rr:template "http://example.test/probe/{id}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:probeValue ;
    rr:objectMap [ rr:column "value" ]
  ] .
"#;

const HOLDER_QUERY: &str = "SELECT ?value WHERE { ?item <http://example.test/heldValue> ?value }";
const PROBE_QUERY: &str = "SELECT ?value WHERE { ?item <http://example.test/probeValue> ?value }";

#[derive(Default)]
struct StatementGate {
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}

impl StatementGate {
    fn enter_and_wait(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.0 = true;
        self.changed.notify_all();
        while !state.1 {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
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
        assert!(
            !timeout.timed_out(),
            "holder SQLite statement did not start"
        );
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

struct ReleaseOnDrop(Arc<StatementGate>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct Fixture {
    holder: Arc<ServeConfig>,
    waiter: Arc<ServeConfig>,
    recovery: Arc<ServeConfig>,
    pool: SqlitePool,
    gate: Arc<StatementGate>,
    probe_entries: Arc<AtomicUsize>,
}

fn fixture() -> Fixture {
    let conn = rusqlite::Connection::open_in_memory().expect("open fixture");
    let gate = Arc::new(StatementGate::default());
    conn.create_scalar_function("test_hold", 0, FunctionFlags::SQLITE_UTF8, {
        let gate = gate.clone();
        move |_| {
            gate.enter_and_wait();
            Ok(7_i64)
        }
    })
    .expect("register holder UDF");
    let probe_entries = Arc::new(AtomicUsize::new(0));
    conn.create_scalar_function("test_probe", 0, FunctionFlags::SQLITE_UTF8, {
        let entries = probe_entries.clone();
        move |_| {
            entries.fetch_add(1, Ordering::SeqCst);
            Ok(11_i64)
        }
    })
    .expect("register probe UDF");

    let schema = introspect_sqlite_all(&conn).expect("introspect fixture");
    let backend = Backend::sqlite(conn);
    let Backend::Sqlite(pool) = &backend else {
        unreachable!("fixture constructs SQLite")
    };
    let pool = pool.clone();
    let config = |timeout| {
        let mapping = sf_mapping::parse_r2rml(MAPPING).expect("parse mapping");
        let mut cfg =
            ServeConfig::new_unchecked(backend.clone(), mapping, Tbox::default(), schema.clone());
        cfg.timeout = timeout;
        Arc::new(cfg)
    };

    Fixture {
        holder: config(HOLDER_TIMEOUT),
        waiter: config(WAITER_TIMEOUT),
        recovery: config(HOLDER_TIMEOUT),
        pool,
        gate,
        probe_entries,
    }
}

fn request(query: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, "application/sparql-results+json")
        .body(Body::from(query.to_owned()))
        .expect("static request")
}

async fn collect(response: Response<Body>) {
    response
        .into_body()
        .collect()
        .await
        .expect("collect successful response");
}

async fn prewarm_probe(fixture: &Fixture) {
    let response = router(fixture.waiter.clone())
        .oneshot(request(PROBE_QUERY))
        .await
        .expect("route prewarm request");
    assert_eq!(response.status(), StatusCode::OK);
    collect(response).await;
    assert!(
        fixture.probe_entries.swap(0, Ordering::SeqCst) > 0,
        "prewarm must enter the probe UDF"
    );
}

async fn start_holder(fixture: &Fixture) -> Response<Body> {
    let response = router(fixture.holder.clone())
        .oneshot(request(HOLDER_QUERY))
        .await
        .expect("route holder request");
    assert_eq!(response.status(), StatusCode::OK);
    let gate = fixture.gate.clone();
    tokio::task::spawn_blocking(move || gate.wait_until_entered())
        .await
        .expect("holder-entry watchdog task");
    response
}

async fn run_waiter_to_deadline(fixture: &Fixture) -> Response<Body> {
    let (pending_tx, pending_rx) = std::sync::mpsc::channel();
    fixture.pool.set_admission_pending_observer(move || {
        pending_tx
            .send(())
            .expect("report pending SQLite admission")
    });

    let mut service = router(fixture.waiter.clone());
    std::future::poll_fn(|cx| Service::poll_ready(&mut service, cx))
        .await
        .expect("waiter service ready");
    // `call` mints the absolute deadline synchronously. The observer reports
    // only after `handle.acquire()` registers its semaphore waiter and returns
    // `Pending`, so advancing time after this receipt is a causal proof.
    let waiter = tokio::spawn(service.call(request(PROBE_QUERY)));
    tokio::task::spawn_blocking(move || {
        pending_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("waiter did not reach pending SQLite admission")
    })
    .await
    .expect("admission observer watchdog task");
    tokio::time::advance(WAITER_TIMEOUT).await;
    waiter
        .await
        .expect("waiter task")
        .expect("route waiter request")
}

#[tokio::test(start_paused = true)]
async fn queued_request_times_out_before_200_without_entering_sqlite() {
    let fixture = fixture();
    prewarm_probe(&fixture).await;
    let release = ReleaseOnDrop(fixture.gate.clone());
    let holder = start_holder(&fixture).await;

    let response = run_waiter_to_deadline(&fixture).await;
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(fixture.probe_entries.load(Ordering::SeqCst), 0);

    drop(release);
    collect(holder).await;
}

#[tokio::test(start_paused = true)]
async fn dropping_body_keeps_admission_until_blocking_producer_returns() {
    let fixture = fixture();
    prewarm_probe(&fixture).await;
    let release = ReleaseOnDrop(fixture.gate.clone());
    let holder = start_holder(&fixture).await;

    drop(holder);
    let response = run_waiter_to_deadline(&fixture).await;
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(fixture.probe_entries.load(Ordering::SeqCst), 0);

    drop(release);
    let recovered = router(fixture.recovery.clone())
        .oneshot(request(PROBE_QUERY))
        .await
        .expect("route recovery request");
    assert_eq!(recovered.status(), StatusCode::OK);
    collect(recovered).await;
    assert!(
        fixture.probe_entries.load(Ordering::SeqCst) > 0,
        "recovery request must enter the probe UDF"
    );
}
