use super::*;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use sf_core::{query_control::QueryLimits, SourceId, SourceMapping};
use tower::ServiceExt;

const QUERY: &str = "SELECT ?s WHERE { ?s ?p ?o }";

#[path = "../tests/support/compiler_key.rs"]
mod compiler_key;
use compiler_key::{
    build_work, key_work, normalization_work, rewrite_work, source_free_entry_work,
    source_free_join_seed_work, source_free_values_work,
};

#[path = "request_normalize_tests.rs"]
mod structural_normalization;

#[path = "request_describe_tests.rs"]
mod describe_work;

#[path = "request_star_tests.rs"]
mod star_work;

#[path = "request_optional_tests.rs"]
mod optional_work;

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
    let (prefix, tail) = source_free_entry_work(QUERY);
    // Resolution leaves Empty: its one leaf visit is not entry/scope work.
    let build = build_work(QUERY) + normalization_work(QUERY, &[]) + prefix + tail + 1;
    let input = QUERY.len() as u64;
    let key = key_work(QUERY);
    let rewrite = rewrite_work(QUERY);
    let pass = rewrite + build;
    let exact = 2 * input + key + 2 * pass;
    let canonical = spargebra::SparqlParser::new()
        .parse_query(QUERY)
        .unwrap()
        .to_string()
        .len() as u64;
    for (work, succeeds, consumed) in [
        (2 * input + pass - 1, false, input + pass),
        (
            2 * input + pass + key - 1,
            false,
            2 * input + pass + key - canonical,
        ),
        (exact, true, exact),
    ] {
        let (cfg, _) = config(work);
        let budget = cfg.request_budget();
        let snapshot = cfg.runtime_lease().unwrap();
        let reservation = preflight(cfg.clone(), snapshot.clone(), QUERY.into(), budget.clone())
            .await
            .unwrap();
        assert_eq!(budget.consumed(QueryCharge::CompilerWork), input + pass);
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
        // A sticky terminal wakes the waiter before the owned blocking worker
        // necessarily drops its permit. Require bounded actual cleanup.
        let recovered = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            cfg.compiler_permits().acquire_many_owned(4),
        )
        .await
        .expect("both compiler passes must finish cleanup")
        .unwrap();
        drop(recovered);
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
        (cloning, vec![], source_free_values_work(3, "x")),
        (
            products,
            vec![],
            source_free_join_seed_work() + 1 + source_free_values_work(4, "a") + 3,
        ), // One short of the first 1×4 product, after seed/first VALUES.
        (
            right_heavy.as_str(),
            vec![],
            source_free_join_seed_work()
                + 2
                + source_free_values_work(2, "a")
                + source_free_values_work(1, "b")
                + 512,
        ), // New seed/leaves only; preserve the old repeated-payload cut.
        (absent, mapping, 5),
        (
            "SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s <http://example.test/a>+ ?o } }",
            path_maps.clone(),
            0,
        ),
        ("SELECT ?s ?o WHERE { ?s !<urn:absent> ?o }", path_maps, 0),
    ] {
        // Source-free fixtures target LOWER; mapped fixtures still target earlier
        // RESOLVE. Do not move their existing rejection boundary to NORMALIZE.
        let normalization = if maps.is_empty() {
            normalization_work(query, &[]) + source_free_entry_work(query).0
        } else {
            0
        };
        let preflight_work =
            query.len() as u64 + rewrite_work(query) + build_work(query) + normalization + extra;
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

#[tokio::test]
async fn structural_build_failure_precedes_source_while_completed_hits_skip_build() {
    use http_body_util::BodyExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    for secured in [false, true] {
        for (warm, extra) in [
            (false, 0),
            (false, rewrite_work(QUERY) + build_work(QUERY) - 1),
            (true, 0),
        ] {
            let (mut cfg, pool) = config(100_000);
            if secured {
                Arc::get_mut(&mut cfg)
                    .unwrap()
                    .set_query_admission(crate::QueryAdmission::Bearer(
                        crate::BearerQueryAdmission::for_service_principal(
                            "test-only-build-work-credential-123456",
                        )
                        .unwrap(),
                    ));
            }
            let request = || {
                Request::post("/sparql")
                    .header("content-type", "application/sparql-query")
                    .header(
                        "authorization",
                        "Bearer test-only-build-work-credential-123456",
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
                QUERY.len() as u64 + key_work(QUERY) + extra,
                u64::MAX,
                u64::MAX,
                u64::MAX,
            );
            let held = pool.pick_owned().acquire().await.unwrap();
            let calls = Arc::new(AtomicUsize::new(0));
            let observed = calls.clone();
            let queued = Arc::new(tokio::sync::Notify::new());
            let notify = queued.clone();
            pool.set_admission_pending_observer(move || {
                observed.fetch_add(1, Ordering::SeqCst);
                notify.notify_one();
            });
            let mut response = Box::pin(crate::router(cfg.clone()).oneshot(request()));
            let mut held = Some(held);
            if warm {
                // Paid shared-plan hits skip BUILD, not normal source admission.
                tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    tokio::select! {
                        _ = queued.notified() => (),
                        result = &mut response => panic!("warm hit bypassed held source: {result:?}"),
                    }
                }).await.expect("paid cache hit reaches source admission");
                drop(held.take());
            }
            let response = tokio::time::timeout(std::time::Duration::from_secs(2), response)
                .await
                .expect("BUILD must not wait for source")
                .unwrap();
            assert_eq!(
                response.status(),
                if warm {
                    StatusCode::OK
                } else {
                    StatusCode::TOO_MANY_REQUESTS
                }
            );
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            if warm {
                let result: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(result["results"]["bindings"], serde_json::json!([]));
            }
            if warm {
                assert!(calls.load(Ordering::SeqCst) > 0);
            } else {
                assert_eq!(calls.load(Ordering::SeqCst), 0);
            }
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

#[tokio::test]
async fn constant_normalization_failure_never_enters_held_source_admission() {
    use http_body_util::BodyExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    for query in compiler_key::CONSTANT_QUERIES {
        let exact = query.len() as u64 + compiler_key::constant_compile_work(query);
        let prerequisite =
            query.len() as u64 + key_work(query) + rewrite_work(query) + build_work(query);
        assert!(exact > prerequisite);
        for secured in [false, true] {
            for work in [prerequisite, exact - 1] {
                let (mut cfg, pool) = config(work);
                if secured {
                    Arc::get_mut(&mut cfg).unwrap().set_query_admission(
                        crate::QueryAdmission::Bearer(
                            crate::BearerQueryAdmission::for_service_principal(
                                "test-only-row-work-credential-123456",
                            )
                            .unwrap(),
                        ),
                    );
                }
                let held = pool.pick_owned().acquire().await.unwrap();
                let calls = Arc::new(AtomicUsize::new(0));
                let observed = calls.clone();
                pool.set_admission_pending_observer(move || {
                    observed.fetch_add(1, Ordering::SeqCst);
                });
                let request = Request::post("/sparql")
                    .header("content-type", "application/sparql-query")
                    .header(
                        "authorization",
                        "Bearer test-only-row-work-credential-123456",
                    )
                    .body(Body::from(query))
                    .unwrap();
                let response = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    crate::router(cfg.clone()).oneshot(request),
                )
                .await
                .expect("row normalization rejects without source wait")
                .unwrap();
                assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
                response.into_body().collect().await.unwrap();
                assert_eq!(calls.load(Ordering::SeqCst), 0);
                let recovered = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    cfg.compiler_permits().acquire_many_owned(4),
                )
                .await
                .expect("all compiler capacity recovers")
                .unwrap();
                drop(recovered);
                drop(held);
            }
        }
    }
}
