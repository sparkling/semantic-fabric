//! Public refusal at the actual paid OPTIONAL helper, not an earlier stage.
use super::*;
use std::sync::Mutex;
use tracing::{span::Attributes, Id, Subscriber};
use tracing_subscriber::{layer::Context, prelude::*, registry::LookupSpan, Layer};

const CHAINED: [&str; 2] = [
    "SELECT ?value ?optional ?matched WHERE { ?item <http://example.test/a> ?value OPTIONAL { ?item <http://example.test/b> ?optional } OPTIONAL { ?matched <http://example.test/b> ?optional } }",
    "SELECT ?value ?optional ?matched WHERE { ?item <http://example.test/a> ?value OPTIONAL { ?item <http://example.test/b> ?optional } OPTIONAL { { SELECT DISTINCT ?matched ?optional WHERE { ?matched <http://example.test/b> ?optional } } } }",
];

fn helper_work(query: &str, profile: MappedProfile) -> (u64, u64, u64) {
    use sf_core::query_control::QueryBudget;
    struct Marker;
    struct Observe {
        span_name: &'static str,
        budget: Arc<QueryBudget>,
        bounds: Arc<Mutex<Vec<(u64, u64)>>>,
    }
    impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Observe {
        fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
            if attrs.metadata().name() == self.span_name {
                ctx.span(id).unwrap().extensions_mut().insert(Marker);
            }
        }
        fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
            if ctx.span(id).unwrap().extensions().get::<Marker>().is_some() {
                let work = self.budget.consumed(QueryCharge::CompilerWork);
                self.bounds.lock().unwrap().push((work, work));
            }
        }
        fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
            if ctx.span(id).unwrap().extensions().get::<Marker>().is_some() {
                self.bounds.lock().unwrap().last_mut().unwrap().1 =
                    self.budget.consumed(QueryCharge::CompilerWork);
            }
        }
    }
    let budget = Arc::new(QueryBudget::new(QueryLimits::new(
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    )));
    let bounds = Arc::new(Mutex::new(Vec::new()));
    let binding = sf_sparql::CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), mapped_fixture()),
        sf_sql::Dialect::Sqlite,
        Default::default(),
        vec![],
        Default::default(),
        64, // Match RuntimeBinding's cache geometry, not a capacity-one cache.
    );
    tracing::subscriber::with_default(
        tracing_subscriber::registry().with(Observe {
            span_name: if profile == MappedProfile::Preparation {
                "sf.compiler.optional"
            } else {
                "sf.compiler.optional_bindings"
            },
            budget: budget.clone(),
            bounds: bounds.clone(),
        }),
        || {
            binding
                .compile_shared_with_work_control(query, budget.as_ref())
                .inspect(|plan| crate::admission::admit(plan, 0, budget.as_ref()).unwrap())
                .unwrap();
        },
    );
    let bounds = bounds.lock().unwrap();
    let expected = if profile == MappedProfile::Preparation {
        2
    } else {
        3
    };
    assert_eq!(
        bounds.len(),
        expected,
        "actual OPTIONAL helper callers execute"
    );
    let (start, end) = *bounds.last().unwrap();
    assert!(
        end > start + 1,
        "second helper pays a prior nullable alias, not only entry"
    );
    let input = query.len() as u64;
    (
        input + start,
        input + end,
        input + budget.consumed(QueryCharge::CompilerWork),
    )
}

#[test]
fn mapped_nullable_optional_helper_refuses_before_source_and_recovers() {
    mapped_process("request_compile::tests::optional_work::preparation::mapped_nullable_optional_helper_refuses_before_source_and_recovers",
        MappedProfile::Preparation);
}

#[test]
fn mapped_optional_binding_construction_refuses_before_source_and_recovers() {
    mapped_process("request_compile::tests::optional_work::preparation::mapped_optional_binding_construction_refuses_before_source_and_recovers",
        MappedProfile::Materialization);
}

pub(super) async fn cases(profile: MappedProfile) {
    for query in CHAINED {
        let (prefix, helper_end, complete) = helper_work(query, profile);
        let cached = query.len() as u64
            + compiler_key::warm_work(query)
            + compiler_key::admission_work(query, &mapped_fixture());
        assert!(cached < prefix && prefix < helper_end && helper_end < complete);
        for secured in [false, true] {
            let (mut cfg, pool) = config_with_mapping(prefix, mapped_fixture());
            pool.pick().lock().unwrap().execute_batch(
                "CREATE TABLE items (id INTEGER, value TEXT, extra TEXT); INSERT INTO items VALUES (1, 'one', 'x'), (2, 'one', NULL), (3, 'two', 'y');"
            ).unwrap();
            if secured {
                Arc::get_mut(&mut cfg)
                    .unwrap()
                    .set_query_admission(crate::QueryAdmission::Bearer(
                        crate::BearerQueryAdmission::for_service_principal(
                            "test-only-optional-helper-credential-123456",
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
                        "Bearer test-only-optional-helper-credential-123456",
                    )
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
            for work in [prefix, helper_end - 1, helper_end - 1] {
                change_work(&mut cfg, work).await;
                let response = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    crate::router(cfg.clone()).oneshot(request()),
                )
                .await
                .expect("OPTIONAL helper fails before held source")
                .unwrap();
                assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
                let body = response.into_body().collect().await.unwrap().to_bytes();
                let problem: serde_json::Value = serde_json::from_slice(&body).unwrap();
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
                    tokio::select! { _ = queued.notified() => (), result = &mut response => panic!("paid OPTIONAL bypassed source: {result:?}") }
                }).await.expect("paid cold/warm query reaches held source");
                assert_eq!(calls.load(Ordering::SeqCst), index + 1);
                drop(held);
                let response = tokio::time::timeout(std::time::Duration::from_secs(2), response)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let body = response.into_body().collect().await.unwrap().to_bytes();
                let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(
                    result["head"]["vars"],
                    serde_json::json!(["value", "optional", "matched"])
                );
                let mut rows: Vec<_> = result["results"]["bindings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|row| {
                        assert_eq!(row["value"]["type"], "literal");
                        assert_eq!(row["optional"]["type"], "literal");
                        assert_eq!(row["matched"]["type"], "uri");
                        (
                            row["value"]["value"].as_str().unwrap(),
                            row["optional"]["value"].as_str().unwrap(),
                            row["matched"]["value"].as_str().unwrap(),
                        )
                    })
                    .collect();
                rows.sort();
                assert_eq!(
                    rows,
                    [
                        ("one", "x", "http://example.test/item/1"),
                        ("one", "x", "http://example.test/item/1"),
                        ("one", "y", "http://example.test/item/3"),
                        ("two", "y", "http://example.test/item/3")
                    ]
                );
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
