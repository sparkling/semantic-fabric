//! Real mapped COUNT requests: exact results and cache reuse before source I/O.
//! Independent parsing randomizes private name lengths, hence work can differ.
//! Witness warm key-only work with actual stages, not a sampled numeric budget.
use super::*;
use std::sync::Mutex;
use tracing::{span::Attributes, Id, Subscriber};
use tracing_subscriber::{layer::Context, prelude::*, Layer};

const COUNT: [&str; 2] = [
    "SELECT ?value (COUNT(*) AS ?n) WHERE { ?item <http://example.test/a> ?value } GROUP BY ?value",
    "SELECT ?value (COUNT(*) AS ?n) WHERE { { ?item <http://example.test/a> ?value } UNION { ?item <http://example.test/a> ?value } } GROUP BY ?value",
];

#[derive(Clone, Default)]
struct Stages(Arc<Mutex<Vec<String>>>);

impl Stages {
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

impl tracing::field::Visit for Stages {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "stage" {
            self.0.lock().unwrap().push(value.to_owned());
        }
    }
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
}

impl<S: Subscriber> Layer<S> for Stages {
    fn on_new_span(&self, attrs: &Attributes<'_>, _: &Id, _: Context<'_, S>) {
        if attrs.metadata().name() == "sf.compiler.stage" {
            attrs.record(&mut self.clone());
        }
    }
}

#[test]
fn mapped_count_cache_identity_reuses_plan_before_source_admission() {
    mapped_process(
        "request_compile::tests::optional_work::cache_identity::mapped_count_cache_identity_reuses_plan_before_source_admission",
        MappedProfile::Identity,
    );
}

pub(super) async fn cases() {
    let stages = Stages::default();
    // The existing process-isolated helper also verifies that this exact child
    // executed all cases; a zero-test libtest exit cannot pass as evidence.
    tracing::subscriber::set_global_default(tracing_subscriber::registry().with(stages.clone()))
        .unwrap();
    for (variant, query) in COUNT.into_iter().enumerate() {
        for secured in [false, true] {
            let (mut cfg, pool) = config_with_mapping(1_000_000, mapped_fixture());
            pool.pick()
                .lock()
                .unwrap()
                .execute_batch(
                    "CREATE TABLE items (id INTEGER, value TEXT, extra TEXT); \
                 INSERT INTO items VALUES (1, 'one', 'x'), (2, 'one', NULL), (3, 'two', 'y');",
                )
                .unwrap();
            if secured {
                Arc::get_mut(&mut cfg)
                    .unwrap()
                    .set_query_admission(crate::QueryAdmission::Bearer(
                        crate::BearerQueryAdmission::for_service_principal(
                            "fixture-only-count-credential-123456",
                        )
                        .unwrap(),
                    ));
            }
            let request = || {
                Request::post("/sparql")
                    .header("content-type", "application/sparql-query")
                    .header("accept", "application/sparql-results+json")
                    .header(
                        "authorization",
                        "Bearer fixture-only-count-credential-123456",
                    )
                    .body(Body::from(query))
                    .unwrap()
            };
            let calls = Arc::new(AtomicUsize::new(0));
            for warm in [false, true] {
                let held = pool.pick_owned().acquire().await.unwrap();
                let queued = Arc::new(tokio::sync::Notify::new());
                let notify = queued.clone();
                let observed = calls.clone();
                pool.set_admission_pending_observer(move || {
                    observed.fetch_add(1, Ordering::SeqCst);
                    notify.notify_one();
                });
                let before = calls.load(Ordering::SeqCst);
                // Even a warm cached plan must pay the key after input. This
                // refusal is deterministic and cannot reach the held source.
                change_work(&mut cfg, query.len() as u64).await;
                stages.take();
                let response = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    crate::router(cfg.clone()).oneshot(request()),
                )
                .await
                .expect("key refusal precedes source admission")
                .unwrap();
                assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
                let bytes = response.into_body().collect().await.unwrap().to_bytes();
                let problem: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(problem["code"], "query-budget-exceeded");
                assert_eq!(calls.load(Ordering::SeqCst), before);
                assert_eq!(stages.take(), ["parse"]);

                change_work(&mut cfg, 1_000_000).await;
                let mut response = Box::pin(crate::router(cfg.clone()).oneshot(request()));
                tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    tokio::select! {
                        _ = queued.notified() => (),
                        result = &mut response => panic!("COUNT request bypassed held source: {result:?}"),
                    }
                }).await.expect("cold and warm COUNT reach source admission");
                let observed = stages.take();
                if warm {
                    assert_eq!(
                        observed,
                        ["parse"],
                        "warm hit must skip all compilation stages after parsing"
                    );
                } else {
                    assert!(observed.iter().any(|v| v == "build"));
                    assert!(observed.iter().any(|v| v == "lower"));
                }
                assert_eq!(calls.load(Ordering::SeqCst), before + 1);
                drop(held);
                let response = tokio::time::timeout(std::time::Duration::from_secs(2), response)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let bytes = response.into_body().collect().await.unwrap().to_bytes();
                let result: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(result["head"]["vars"], serde_json::json!(["value", "n"]));
                let mut rows: Vec<_> = result["results"]["bindings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|row| {
                        assert_eq!(row["value"]["type"], "literal");
                        assert_eq!(row["n"]["type"], "literal");
                        assert_eq!(
                            row["n"]["datatype"],
                            "http://www.w3.org/2001/XMLSchema#integer"
                        );
                        (
                            row["value"]["value"].as_str().unwrap(),
                            row["n"]["value"].as_str().unwrap(),
                        )
                    })
                    .collect();
                rows.sort();
                assert_eq!(
                    rows,
                    if variant == 0 {
                        vec![("one", "2"), ("two", "1")]
                    } else {
                        vec![("one", "4"), ("two", "2")]
                    }
                );
                let compiler = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    cfg.compiler_permits().acquire_many_owned(4),
                )
                .await
                .expect("all compiler permits recover")
                .unwrap();
                drop(compiler);
                let source = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    pool.pick_owned().acquire(),
                )
                .await
                .expect("source permit recovers")
                .unwrap();
                drop(source);
            }
        }
    }
}
