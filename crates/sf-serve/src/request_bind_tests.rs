//! BIND and projection/output work refusal, with shared positive source controls.
use super::*;
#[path = "request_bind_observer_tests.rs"]
mod observer;

const QUERIES: [&str; 4] = [
    "SELECT ?label WHERE { ?item <http://example.test/a> ?value BIND(?value AS ?z) BIND(CONCAT(?z, \"!\") AS ?label) FILTER(?item = <http://example.test/item/1>) }",
    "SELECT ?label WHERE { { ?item <http://example.test/a> ?value } UNION { ?item <http://example.test/a> ?value } BIND(?value AS ?z) BIND(CONCAT(?z, \"!\") AS ?label) FILTER(?item = <http://example.test/item/1>) }",
    "SELECT ?label WHERE { ?item <http://example.test/a> ?value OPTIONAL { { ?item <http://example.test/b> ?optional } UNION { ?item <http://example.test/b> ?optional } } BIND(?value AS ?z) BIND(CONCAT(?z, \"!\") AS ?label) FILTER(?item = <http://example.test/item/1>) }",
    "SELECT DISTINCT ?label WHERE { ?item <http://example.test/a> ?value BIND(?value AS ?z) BIND(CONCAT(?z, \"!\") AS ?label) FILTER(?item = <http://example.test/item/1>) }",
];

const BASE_QUERIES: [&str; 3] = [
    QUERIES[0],
    "SELECT ?label ?tag WHERE { VALUES ?tag { \"a\" UNDEF \"a\" } ?item <http://example.test/a> ?value BIND(CONCAT(?value, \"!\") AS ?label) FILTER(?item = <http://example.test/item/1>) }",
    "SELECT ?label ?tag WHERE { ?item <http://example.test/a> ?value VALUES ?tag { \"a\" UNDEF \"a\" } BIND(CONCAT(?value, \"!\") AS ?label) FILTER(?item = <http://example.test/item/1>) }",
];

#[test]
fn mapped_base_phases_refuse_before_source_and_recover_exact_bags() {
    mapped_process("request_compile::tests::optional_work::bind::mapped_base_phases_refuse_before_source_and_recover_exact_bags", MappedProfile::Base);
}

#[test]
fn mapped_bind_phases_refuse_before_source_and_recover_exact_bags() {
    mapped_process("request_compile::tests::optional_work::bind::mapped_bind_phases_refuse_before_source_and_recover_exact_bags", MappedProfile::Bind);
}

#[test]
fn mapped_projection_phases_refuse_before_source_and_recover_exact_bags() {
    mapped_process("request_compile::tests::optional_work::bind::mapped_projection_phases_refuse_before_source_and_recover_exact_bags", MappedProfile::Projection);
}

pub(super) async fn cases(profile: MappedProfile) {
    let projection = profile == MappedProfile::Projection;
    let base = profile == MappedProfile::Base;
    let queries = if base {
        &BASE_QUERIES[..]
    } else if projection {
        &QUERIES[..]
    } else {
        &QUERIES[..3]
    };
    for (variant, query) in queries.iter().copied().enumerate() {
        let (cuts, complete) = if base {
            observer::base_work(query)
        } else if projection {
            observer::projection_work(query)
        } else {
            observer::work(query)
        };
        assert!(
            complete <= 1_000_000,
            "existing default admits supported BIND"
        );
        let cached = query.len() as u64 + filter::key_work(query, &mapped_fixture());
        assert!(cuts.iter().all(|work| cached < *work && *work < complete));
        for secured in [false, true] {
            let (mut cfg, pool) = config_with_mapping(cuts[0], mapped_fixture());
            pool.pick().lock().unwrap().execute_batch(
                "CREATE TABLE items (id INTEGER, value TEXT, extra TEXT); INSERT INTO items VALUES (1, 'one', 'x'), (2, 'one', NULL), (3, 'two', 'y');"
            ).unwrap();
            if secured {
                Arc::get_mut(&mut cfg)
                    .unwrap()
                    .set_query_admission(crate::QueryAdmission::Bearer(
                        crate::BearerQueryAdmission::for_service_principal(
                            "test-only-bind-credential-123456",
                        )
                        .unwrap(),
                    ));
            }
            let request = || {
                Request::post("/sparql")
                    .header("content-type", "application/sparql-query")
                    .header("accept", "application/sparql-results+json")
                    .header("authorization", "Bearer test-only-bind-credential-123456")
                    .body(Body::from(query))
                    .unwrap()
            };
            let calls = Arc::new(AtomicUsize::new(0));
            let queued = Arc::new(tokio::sync::Notify::new());
            let notify = queued.clone();
            let observed = calls.clone();
            pool.set_admission_pending_observer(move || {
                observed.fetch_add(1, Ordering::SeqCst);
                notify.notify_one();
            });
            let held = pool.pick_owned().acquire().await.unwrap();
            // Repeat the final failure: sticky refusal must not leak a permit
            // or consume the still-armed, one-shot source admission observer.
            for work in cuts.iter().copied().chain(cuts.last().copied()) {
                change_work(&mut cfg, work).await;
                let response = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    crate::router(cfg.clone()).oneshot(request()),
                )
                .await
                .expect("bind work fails before held source")
                .unwrap();
                assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
                let bytes = response.into_body().collect().await.unwrap().to_bytes();
                let problem: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(problem["code"], "query-budget-exceeded");
                assert_eq!(calls.load(Ordering::SeqCst), 0);
            }
            drop(held);
            for (index, work) in [complete, cached].into_iter().enumerate() {
                change_work(&mut cfg, work).await;
                let held = pool.pick_owned().acquire().await.unwrap();
                if index > 0 {
                    let notify = queued.clone();
                    let observed = calls.clone();
                    pool.set_admission_pending_observer(move || {
                        observed.fetch_add(1, Ordering::SeqCst);
                        notify.notify_one();
                    });
                }
                let mut response = Box::pin(crate::router(cfg.clone()).oneshot(request()));
                tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    tokio::select! { _ = queued.notified() => (), result = &mut response => panic!("paid bind bypassed source: {result:?}") }
                }).await.expect("funded cold and key-only warm queries reach held source");
                assert_eq!(calls.load(Ordering::SeqCst), index + 1);
                drop(held);
                let response = tokio::time::timeout(std::time::Duration::from_secs(2), response)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let bytes = response.into_body().collect().await.unwrap().to_bytes();
                let result: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let values = base && variant > 0;
                assert_eq!(
                    result["head"]["vars"],
                    if values {
                        serde_json::json!(["label", "tag"])
                    } else {
                        serde_json::json!(["label"])
                    }
                );
                let rows = result["results"]["bindings"].as_array().unwrap();
                assert_eq!(
                    rows.len(),
                    if values {
                        3
                    } else if matches!(variant, 0 | 3) {
                        1
                    } else {
                        2
                    }
                );
                let mut unbound = 0;
                for row in rows {
                    let mut expected =
                        serde_json::json!({"label":{"type":"literal","value":"one!"}});
                    if values && row.get("tag").is_some() {
                        expected["tag"] = serde_json::json!({"type":"literal","value":"a"});
                    } else if values {
                        unbound += 1;
                    }
                    assert_eq!(row, &expected);
                }
                assert_eq!(unbound, usize::from(values));
                drop(
                    tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        cfg.compiler_permits().acquire_many_owned(4),
                    )
                    .await
                    .unwrap()
                    .unwrap(),
                );
                drop(
                    tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        pool.pick_owned().acquire(),
                    )
                    .await
                    .unwrap()
                    .unwrap(),
                );
            }
        }
    }
}
