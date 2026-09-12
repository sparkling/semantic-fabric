//! The admission tail is paid on both newly compiled and resident plans.
use super::*;
use http_body_util::BodyExt;
use sf_core::query_control::QueryBudget;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn portable_authorization_precedes_shape_rejection_and_profiles_accepted_plan() {
    const TOKEN: &str = "test-only-portable-shape-credential-123456";
    const BASE: &str = "SELECT ?value WHERE { ?s <http://example.test/a> ?value }";
    let maps = sf_mapping::parse_r2rml(r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        <urn:items> a rr:TriplesMap; rr:logicalTable [rr:tableName "items"];
          rr:subjectMap [rr:template "http://example.test/item/{id}"];
          rr:predicateObjectMap [rr:predicate <http://example.test/a>; rr:objectMap [rr:column "value"; rr:datatype <http://www.w3.org/2001/XMLSchema#string>]].
    "#).unwrap();
    for covered in [false, true] {
        let (mut cfg, pool) = config_with_mapping(1_000_000, maps.clone());
        pool.pick().lock().unwrap().execute_batch("CREATE TABLE items(id INTEGER, value TEXT); INSERT INTO items VALUES(1,'one'),(2,'hidden');").unwrap();
        let policy = crate::PortableRowPolicy::new(vec![crate::PortableRowRule::new(
            0,
            if covered { "items" } else { "other" },
            "value",
            "one",
        )
        .unwrap()])
        .unwrap();
        Arc::get_mut(&mut cfg).unwrap().set_query_admission(
            crate::QueryAdmission::ProvisionedBearers(
                crate::ProvisionedBearerAdmission::new(vec![
                    crate::ProvisionedBearerSubject::portable_rows("subject", TOKEN, policy)
                        .unwrap(),
                ])
                .unwrap(),
            ),
        );
        for ordered in [true, false] {
            let query = format!("{BASE}{}", if ordered { " ORDER BY ?value" } else { "" });
            let response = crate::router(cfg.clone())
                .oneshot(
                    Request::post("/sparql")
                        .header("content-type", "application/sparql-query")
                        .header("accept", "application/sparql-results+json")
                        .header("authorization", format!("Bearer {TOKEN}"))
                        .body(Body::from(query))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = if !covered {
                StatusCode::FORBIDDEN
            } else if ordered {
                StatusCode::NOT_IMPLEMENTED
            } else {
                StatusCode::OK
            };
            assert_eq!(response.status(), status);
            let body: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            if status == StatusCode::OK {
                assert_eq!(
                    body["results"]["bindings"],
                    serde_json::json!([{"value":{"type":"literal","value":"one"}}])
                );
            } else {
                assert_eq!(
                    body["code"],
                    if covered {
                        "unsupported-query"
                    } else {
                        "access-denied"
                    }
                );
            }
        }
    }
}

#[tokio::test]
async fn federation_admission_charges_both_fragments_before_source_work() {
    const QUERY: &str = "SELECT ?value WHERE { { ?s <http://example.test/a> ?value } UNION { ?s <http://example.test/b> ?value } }";
    let mut pools = Vec::new();
    let sources = ["a", "b"].into_iter().enumerate().map(|(index, predicate)| {
        let backend = crate::Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap());
        let crate::Backend::Sqlite(pool) = &backend else { unreachable!() };
        pool.pick().lock().unwrap().execute_batch("CREATE TABLE items(value TEXT); INSERT INTO items VALUES('one'),('one');").unwrap();
        pools.push(pool.clone());
        let maps = sf_mapping::parse_r2rml(&format!(r#"
            @prefix rr: <http://www.w3.org/ns/r2rml#> .
            <urn:items> a rr:TriplesMap; rr:logicalTable [rr:tableName "items"];
              rr:subject <http://example.test/item>;
              rr:predicateObjectMap [rr:predicate <http://example.test/{predicate}>; rr:objectMap [rr:column "value"; rr:datatype <http://www.w3.org/2001/XMLSchema#string>]].
        "#)).unwrap();
        crate::RuntimeSource::new(crate::IntrospectedSource::unchecked(backend, vec![]), SourceMapping::new(SourceId::new(index).unwrap(), maps))
    }).collect::<Vec<_>>();
    let mut cfg = Arc::new(
        ServeConfig::new_federated(
            sources.try_into().ok().unwrap(),
            crate::test_support::ontology(&[], &["http://example.test/a", "http://example.test/b"]),
        )
        .unwrap(),
    );
    Arc::get_mut(&mut cfg)
        .unwrap()
        .set_query_admission(crate::QueryAdmission::UnrestrictedDevelopment);
    let crate::config::QueryMode::SourceAffineUnion(ids) = cfg.query_mode() else {
        unreachable!()
    };
    // Deliberately populate both caches, then measure the exact warm compiler
    // independently of the post-compile fragment-admission tail.
    let unlimited = || QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    let snapshot = cfg.runtime_lease().unwrap();
    snapshot
        .compile_federated_union(ids, QUERY, &unlimited())
        .unwrap();
    let work = unlimited();
    let plan = snapshot.compile_federated_union(ids, QUERY, &work).unwrap();
    let compiler = QUERY.len() as u64 + work.consumed(QueryCharge::CompilerWork);
    let costs: Vec<_> = plan
        .plan()
        .fragments()
        .iter()
        .map(|f| compiler_key::plan_admission_work(f.plan()))
        .collect();
    assert_eq!(costs.len(), 2);
    assert!(costs.iter().all(|n| *n > 0));
    drop(plan);
    drop(snapshot);
    let first = compiler + costs[0];
    let exact = first + costs[1];
    for allowance in [first, exact - 1, exact] {
        super::cache_operations::allowance(&mut cfg, allowance).await;
        let held_left = pools[0].pick_owned().acquire().await.unwrap();
        let held_right = pools[1].pick_owned().acquire().await.unwrap();
        let request = Request::post("/sparql")
            .header("content-type", "application/sparql-query")
            .header("accept", "application/sparql-results+json")
            .body(Body::from(QUERY))
            .unwrap();
        let response = crate::router(cfg.clone()).oneshot(request);
        if allowance == exact {
            drop(held_left);
            drop(held_right);
        }
        let response = tokio::time::timeout(std::time::Duration::from_secs(2), response)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            response.status(),
            if allowance == exact {
                StatusCode::OK
            } else {
                StatusCode::TOO_MANY_REQUESTS
            }
        );
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        if allowance == exact {
            // Each source repeats the same RDF triple (one graph fact), while
            // UNION retains one identical solution from each source arm.
            assert_eq!(
                body["results"]["bindings"],
                serde_json::json!([
                    {"value":{"type":"literal","value":"one"}},
                    {"value":{"type":"literal","value":"one"}}
                ])
            );
        } else {
            assert_eq!(body["code"], "query-budget-exceeded");
        }
    }
}

#[tokio::test]
async fn paid_admission_tail_refuses_before_source_and_recovers_cold_warm_bags() {
    const QUERY: &str = "SELECT ?value ?extra WHERE { ?s <http://example.test/a> ?value OPTIONAL { ?s <http://example.test/b> ?extra } }";
    const TOKEN: &str = "test-only-admission-tail-credential-123456";
    let maps = sf_mapping::parse_r2rml(r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        <urn:items> a rr:TriplesMap; rr:logicalTable [rr:tableName "items"];
          rr:subjectMap [rr:template "http://example.test/item/{id}"];
          rr:predicateObjectMap [rr:predicate <http://example.test/a>; rr:objectMap [rr:column "value"; rr:datatype <http://www.w3.org/2001/XMLSchema#string>]],
            [rr:predicate <http://example.test/b>; rr:objectMap [rr:column "extra"; rr:datatype <http://www.w3.org/2001/XMLSchema#string>]].
    "#).unwrap();
    let binding = sf_sparql::CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), maps.clone()),
        sf_sql::Dialect::Sqlite,
        Default::default(),
        vec![],
        Default::default(),
        64,
    );
    let unlimited = || QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    let cold = unlimited();
    let plan = binding
        .compile_shared_with_work_control(QUERY, &cold)
        .unwrap();
    let warm = unlimited();
    assert!(Arc::ptr_eq(
        &plan,
        &binding
            .compile_shared_with_work_control(QUERY, &warm)
            .unwrap()
    ));
    let tail = unlimited();
    crate::admission::admit(&plan, 0, &tail).unwrap();
    let tail = tail.consumed(QueryCharge::CompilerWork);
    assert!(tail > 0);
    let cold = QUERY.len() as u64 + cold.consumed(QueryCharge::CompilerWork);
    let warm = QUERY.len() as u64 + warm.consumed(QueryCharge::CompilerWork);
    assert!(cold + tail <= 1_000_000);
    for secured in [false, true] {
        let (mut cfg, pool) = config_with_mapping(0, maps.clone());
        pool.pick().lock().unwrap().execute_batch(
            "CREATE TABLE items(id INTEGER, value TEXT, extra TEXT); INSERT INTO items VALUES(1,'one',NULL),(2,'one','two'),(3,'two',NULL);"
        ).unwrap();
        if secured {
            Arc::get_mut(&mut cfg)
                .unwrap()
                .set_query_admission(crate::QueryAdmission::Bearer(
                    crate::BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
                ));
        }
        // The failed cold tail may publish its completed compiler plan. It must
        // still pay admission on the next warm request, never inherit a verdict.
        let mut observer = None;
        for (work, accepted) in [
            (cold + tail - 1, false),
            (warm + tail - 1, false),
            (warm + tail, true),
            (warm + tail - 1, false),
            (warm + tail, true),
        ] {
            super::cache_operations::allowance(&mut cfg, work).await;
            let held = pool.pick_owned().acquire().await.unwrap();
            let (calls, queued) = observer
                .get_or_insert_with(|| {
                    let calls = Arc::new(AtomicUsize::new(0));
                    let count = calls.clone();
                    let queued = Arc::new(tokio::sync::Notify::new());
                    let notify = queued.clone();
                    pool.set_admission_pending_observer(move || {
                        count.fetch_add(1, Ordering::SeqCst);
                        notify.notify_one();
                    });
                    (calls, queued)
                })
                .clone();
            let request = Request::post("/sparql")
                .header("content-type", "application/sparql-query")
                .header("accept", "application/sparql-results+json")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::from(QUERY))
                .unwrap();
            let mut response = Box::pin(crate::router(cfg.clone()).oneshot(request));
            if accepted {
                tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    tokio::select! { _ = queued.notified() => (), r = &mut response => panic!("funded path bypassed source: {r:?}") }
                }).await.unwrap();
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                drop(held);
            }
            let response = tokio::time::timeout(std::time::Duration::from_secs(2), response)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                response.status(),
                if accepted {
                    StatusCode::OK
                } else {
                    StatusCode::TOO_MANY_REQUESTS
                }
            );
            let body: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            if accepted {
                let mut rows: Vec<_> = body["results"]["bindings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| {
                        (
                            r["value"]["value"].as_str().unwrap().to_owned(),
                            r.get("extra")
                                .map(|e| e["value"].as_str().unwrap().to_owned()),
                        )
                    })
                    .collect();
                rows.sort();
                assert_eq!(
                    rows,
                    vec![
                        ("one".into(), None),
                        ("one".into(), Some("two".into())),
                        ("two".into(), None)
                    ]
                );
                observer = None;
            } else {
                assert_eq!(body["code"], "query-budget-exceeded");
                assert_eq!(calls.load(Ordering::SeqCst), 0);
            }
        }
    }
}
