//! BIND substitution work refusal at actual phases, with positive source controls.
use super::*;
#[path = "request_bind_observer_tests.rs"]
mod observer;

const QUERIES: [&str; 3] = [
    "SELECT ?label WHERE { ?item <http://example.test/a> ?value BIND(?value AS ?z) BIND(CONCAT(?z, \"!\") AS ?label) FILTER(?item = <http://example.test/item/1>) }",
    "SELECT ?label WHERE { { ?item <http://example.test/a> ?value } UNION { ?item <http://example.test/a> ?value } BIND(?value AS ?z) BIND(CONCAT(?z, \"!\") AS ?label) FILTER(?item = <http://example.test/item/1>) }",
    "SELECT ?label WHERE { ?item <http://example.test/a> ?value OPTIONAL { { ?item <http://example.test/b> ?optional } UNION { ?item <http://example.test/b> ?optional } } BIND(?value AS ?z) BIND(CONCAT(?z, \"!\") AS ?label) FILTER(?item = <http://example.test/item/1>) }",
];

#[test]
fn mapped_bind_phases_refuse_before_source_and_recover_exact_bags() {
    mapped_process("request_compile::tests::optional_work::bind::mapped_bind_phases_refuse_before_source_and_recover_exact_bags", MappedProfile::Bind);
}

pub(super) async fn cases() {
    for (variant, query) in QUERIES.iter().copied().enumerate() {
        let (cuts, complete) = observer::work(query);
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
                assert_eq!(result["head"]["vars"], serde_json::json!(["label"]));
                let rows = result["results"]["bindings"].as_array().unwrap();
                assert_eq!(rows.len(), if variant == 0 { 1 } else { 2 });
                for row in rows {
                    assert_eq!(
                        row,
                        &serde_json::json!({"label":{"type":"literal","value":"one!"}})
                    );
                }
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
