use super::*;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use sf_core::{query_control::QueryLimits, SourceId, SourceMapping};
use tower::ServiceExt;

const QUERY: &str = "SELECT ?s WHERE { ?s ?p ?o }";

fn key_work(source: &str) -> u64 {
    use std::fmt::{self, Write};
    struct Counter {
        length: usize,
        capacity: usize,
        work: u64,
    }
    impl Write for Counter {
        fn write_str(&mut self, fragment: &str) -> fmt::Result {
            self.work += fragment.len() as u64;
            let next = self.length + fragment.len();
            if next > self.capacity {
                self.capacity = next.max(64).max(2 * self.capacity);
                self.work += (self.capacity + self.length) as u64;
            }
            self.length = next;
            Ok(())
        }
    }
    let query = spargebra::SparqlParser::new().parse_query(source).unwrap();
    let mut counter = Counter {
        length: 0,
        capacity: 0,
        work: 1,
    };
    write!(&mut counter, "{query}").unwrap();
    counter.work + counter.length as u64
}

fn config(work: u64) -> (Arc<ServeConfig>, crate::SqlitePool) {
    config_with_mapping(work, vec![])
}

fn config_with_mapping(
    work: u64,
    maps: Vec<sf_core::ir::TriplesMap>,
) -> (Arc<ServeConfig>, crate::SqlitePool) {
    let backend = crate::Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap());
    let crate::Backend::Sqlite(pool) = &backend else {
        unreachable!()
    };
    let pool = pool.clone();
    let source = crate::IntrospectedSource::unchecked(backend, vec![]);
    let ontology =
        crate::test_support::ontology(&[], &["http://example.test/a", "http://example.test/b"]);
    let mut cfg = ServeConfig::new(
        source,
        SourceMapping::new(SourceId::new(0).unwrap(), maps),
        ontology,
    )
    .unwrap();
    cfg.query_limits = QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX);
    (Arc::new(cfg), pool)
}

#[tokio::test]
async fn preflight_and_authoritative_compile_share_cumulative_input_charge() {
    let exact = 2 * QUERY.len() as u64 + key_work(QUERY);
    let canonical = spargebra::SparqlParser::new()
        .parse_query(QUERY)
        .unwrap()
        .to_string()
        .len() as u64;
    for (work, succeeds, consumed) in [
        (2 * QUERY.len() as u64 - 1, false, QUERY.len() as u64),
        (exact - 1, false, exact - canonical),
        (exact, true, exact),
    ] {
        let (cfg, _) = config(work);
        let budget = cfg.request_budget();
        let snapshot = cfg.runtime_lease().unwrap();
        let reservation = preflight(cfg.clone(), snapshot.clone(), QUERY.into(), budget.clone())
            .await
            .unwrap();
        assert_eq!(
            budget.consumed(QueryCharge::CompilerWork),
            QUERY.len() as u64
        );
        let result = compile(
            cfg.clone(),
            snapshot,
            QUERY.into(),
            budget.clone(),
            Some(reservation),
            false,
        )
        .await;
        if succeeds {
            assert!(result.is_ok());
            assert_eq!(budget.consumed(QueryCharge::CompilerWork), work);
        } else {
            assert_eq!(
                result.err().unwrap().status(),
                StatusCode::TOO_MANY_REQUESTS
            );
            assert_eq!(budget.consumed(QueryCharge::CompilerWork), consumed);
            assert!(budget.checkpoint().is_err());
        }
        assert_eq!(cfg.compiler_permits().available_permits(), 4);
    }
}

#[tokio::test]
async fn zero_allowance_rejects_preflight_before_compiler_queue() {
    let (cfg, _) = config(0);
    let held = cfg.compiler_permits().acquire_many_owned(4).await.unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        preflight(
            cfg.clone(),
            cfg.runtime_lease().unwrap(),
            QUERY.into(),
            cfg.request_budget(),
        ),
    )
    .await
    .expect("must not queue behind held compiler permits");
    assert_eq!(
        result.err().unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    drop(held);
}

#[tokio::test]
async fn public_zero_allowance_never_enters_source_admission() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (cfg, pool) = config(0);
    let held = pool.pick_owned().acquire().await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    pool.set_admission_pending_observer(move || {
        observed.fetch_add(1, Ordering::SeqCst);
    });
    let request = Request::post("/sparql")
        .header("content-type", "application/sparql-query")
        .body(Body::from(QUERY))
        .unwrap();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        crate::router(cfg).oneshot(request),
    )
    .await
    .expect("must not wait for source")
    .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop(held);
}

#[tokio::test]
async fn compiler_expansion_work_rejects_before_source_admission() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let cloning = "SELECT ?x WHERE { VALUES ?x { 1 2 3 } FILTER EXISTS { VALUES ?inside { 7 } } }";
    let products = concat!(
        "SELECT ?a ?b ?c WHERE { VALUES ?a { 0 1 2 3 } ",
        "VALUES ?b { 0 1 2 3 } VALUES ?c { 0 1 2 3 } }",
    );
    let right_heavy = format!(
        "SELECT ?a ?b WHERE {{ VALUES ?a {{ 0 1 }} VALUES ?b {{ \"{}\" }} }}",
        "λ".repeat(1024),
    );
    let absent = "SELECT ?s ?o WHERE { ?s <http://example.test/absent> ?o }";
    let mapping = sf_mapping::parse_r2rml(
        r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        <http://example.test/map> a rr:TriplesMap ;
          rr:logicalTable [ rr:tableName "items" ] ;
          rr:subject <http://example.test/s> ;
          rr:predicateObjectMap [
            rr:predicate <http://example.test/a>, <http://example.test/b> ;
            rr:object "x", "y", "z" ] .
    "#,
    )
    .unwrap();
    let path_maps = sf_mapping::parse_r2rml(
        r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        <http://example.test/path> a rr:TriplesMap ;
          rr:logicalTable [ rr:tableName "items" ] ;
          rr:subjectMap [ rr:template "http://example.test/node/{s}" ] ;
          rr:predicateObjectMap [
            rr:predicate <http://example.test/a>, <http://example.test/b> ;
            rr:objectMap [ rr:template "http://example.test/node/{o}" ] ] .
        "#,
    )
    .unwrap();
    for (query, maps, extra) in [
        (cloning, vec![], 0),
        (products, vec![], 0),
        (right_heavy.as_str(), vec![], 512),
        (absent, mapping, 5),
        (
            "SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s <http://example.test/a>+ ?o } }",
            path_maps.clone(),
            0,
        ),
        ("SELECT ?s ?o WHERE { ?s !<urn:absent> ?o }", path_maps, 0),
    ] {
        let preflight_work = query.len() as u64 + extra;
        let (mut cfg, pool) = config_with_mapping(preflight_work + key_work(query), maps);
        Arc::get_mut(&mut cfg)
            .unwrap()
            .set_query_admission(crate::QueryAdmission::Bearer(
                crate::BearerQueryAdmission::for_service_principal(
                    "test-only-product-work-credential",
                )
                .unwrap(),
            ));
        let held = pool.pick_owned().acquire().await.unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        pool.set_admission_pending_observer(move || {
            observed.fetch_add(1, Ordering::SeqCst);
        });
        let request = Request::post("/sparql")
            .header("content-type", "application/sparql-query")
            .header("authorization", "Bearer test-only-product-work-credential")
            .body(Body::from(query.to_owned()))
            .unwrap();
        // Preflight does not render a cache key; keep its original later-phase
        // limit independently so the new earlier key charge cannot mask it.
        let mut preflight_budget = RequestBudget::after(
            std::time::Duration::from_secs(5),
            QueryLimits::new(preflight_work, u64::MAX, u64::MAX, u64::MAX),
        );
        preflight_budget
            .retain_authenticated(cfg.query_admission.admit(request.headers()).unwrap())
            .unwrap();
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            crate::router(cfg.clone()).oneshot(request),
        )
        .await
        .expect("must reject before source wait")
        .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let preflight = preflight(
            cfg.clone(),
            cfg.runtime_lease().unwrap(),
            query.into(),
            preflight_budget,
        )
        .await;
        assert_eq!(
            preflight.err().unwrap().status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        // The sticky terminal can wake the waiter before the blocking
        // closure finishes dropping its owned state. Require bounded full
        // recovery, not a scheduler-dependent immediate permit count.
        let recovered = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            cfg.compiler_permits().acquire_many_owned(4),
        )
        .await
        .expect("all compiler workers must finish")
        .unwrap();
        drop(recovered);
        assert_eq!(cfg.compiler_permits().available_permits(), 4);
        drop(held);
    }
}

#[tokio::test]
async fn canonical_key_failure_precedes_held_source_and_recovers_compiler_capacity() {
    use http_body_util::BodyExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    for secured in [false, true] {
        for warm in [false, true] {
            let (mut cfg, pool) = config(100_000);
            if secured {
                Arc::get_mut(&mut cfg)
                    .unwrap()
                    .set_query_admission(crate::QueryAdmission::Bearer(
                        crate::BearerQueryAdmission::for_service_principal(
                            "test-only-key-work-credential-123456",
                        )
                        .unwrap(),
                    ));
            }
            let request = || {
                Request::post("/sparql")
                    .header("content-type", "application/sparql-query")
                    .header(
                        "authorization",
                        "Bearer test-only-key-work-credential-123456",
                    )
                    .body(Body::from(QUERY))
                    .unwrap()
            };
            if warm {
                let response = crate::router(cfg.clone()).oneshot(request()).await.unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                response.into_body().collect().await.unwrap();
            }
            Arc::get_mut(&mut cfg).unwrap().query_limits = QueryLimits::new(
                QUERY.len() as u64 + key_work(QUERY) - 1,
                u64::MAX,
                u64::MAX,
                u64::MAX,
            );
            let held = pool.pick_owned().acquire().await.unwrap();
            let calls = Arc::new(AtomicUsize::new(0));
            let observed = calls.clone();
            pool.set_admission_pending_observer(move || {
                observed.fetch_add(1, Ordering::SeqCst);
            });
            let response = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                crate::router(cfg.clone()).oneshot(request()),
            )
            .await
            .expect("key fails before source wait")
            .unwrap();
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            assert_eq!(calls.load(Ordering::SeqCst), 0);
            drop(response);
            let recovered = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                cfg.compiler_permits().acquire_many_owned(4),
            )
            .await
            .unwrap()
            .unwrap();
            drop(recovered);
            drop(held);
        }
    }
}
