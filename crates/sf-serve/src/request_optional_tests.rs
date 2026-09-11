use super::*;
use http_body_util::BodyExt;
use std::sync::atomic::{AtomicUsize, Ordering};
#[path = "request_bind_tests.rs"]
mod bind;
#[path = "request_cache_identity_tests.rs"]
mod cache_identity;
#[path = "request_condition_tests.rs"]
mod condition;
#[path = "request_optional_filter_tests.rs"]
mod filter;
#[path = "request_optional_preparation_tests.rs"]
mod preparation;
#[path = "request_optional_shape_tests.rs"]
mod shape;
#[path = "request_optional_unification_tests.rs"]
mod unification;
const QUERIES: [&str; 4] = [
    "SELECT ?value ?optional WHERE { ?item <http://example.test/a> ?value OPTIONAL { ?item <http://example.test/b> ?optional } }",
    "SELECT ?value ?optional WHERE { ?item <http://example.test/a> ?value OPTIONAL { { ?item <http://example.test/b> ?optional } UNION { ?item <http://example.test/b> ?optional } } }",
    "SELECT ?value ?optional WHERE { ?item <http://example.test/a> ?value OPTIONAL { ?item <http://example.test/b> ?optional OPTIONAL { ?item <http://example.test/b> ?nested } } }",
    "SELECT ?value ?optional WHERE { ?item <http://example.test/a> ?value OPTIONAL { { SELECT DISTINCT ?item ?optional WHERE { ?item <http://example.test/b> ?optional } } } }",
];
const SCOPE_QUERIES: [&str; 3] = [
    "SELECT ?value WHERE { ?item <http://example.test/a> ?value }",
    "SELECT (?value AS ?renamed) WHERE { ?item <http://example.test/a> ?value }",
    "SELECT ?item (?value AS ?renamed) WHERE { ?item <http://example.test/a> ?value }",
];
const ALIAS_QUERIES: [&str; 2] = [
    // Keep alias-reservation accounting independent of parser-generated COUNT
    // names. Separate cache_identity tests prove the COUNT cold/warm public path.
    "SELECT ?value WHERE { ?item <http://example.test/a> ?value } GROUP BY ?value",
    "SELECT ?value WHERE { { ?item <http://example.test/a> ?value } UNION { ?item <http://example.test/a> ?value } } GROUP BY ?value",
];
#[derive(Clone, Copy, PartialEq, Eq)]
enum MappedProfile {
    Optional,
    Scope,
    Alias,
    Identity,
    Preparation,
    Unification,
    Filter,
    Shape,
    Materialization,
    Condition,
    Bind,
}

fn mapped_fixture() -> Vec<sf_core::ir::TriplesMap> {
    sf_mapping::parse_r2rml(
        r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        <http://example.test/optional_work> a rr:TriplesMap;
          rr:logicalTable [rr:tableName "items"];
          rr:subjectMap [rr:template "http://example.test/item/{id}"];
          rr:predicateObjectMap [rr:predicate <http://example.test/a>;
            rr:objectMap [rr:column "value"; rr:datatype <http://www.w3.org/2001/XMLSchema#string>]];
          rr:predicateObjectMap [rr:predicate <http://example.test/b>;
            rr:objectMap [rr:column "extra"; rr:datatype <http://www.w3.org/2001/XMLSchema#string>]].
    "#,
    )
    .unwrap()
}

fn mapped_work(
    query: &str,
    maps: &[sf_core::ir::TriplesMap],
    profile: MappedProfile,
) -> (u64, u64, u64) {
    if profile == MappedProfile::Shape {
        return shape::helper_work(query, maps);
    }
    if profile == MappedProfile::Filter {
        return filter::helper_work(query, maps);
    }
    if profile == MappedProfile::Unification {
        return unification::helper_work(query, maps);
    }
    use sf_core::query_control::QueryBudget;
    use std::sync::Mutex;
    use tracing::{
        span::{Attributes, Id},
        Subscriber,
    };
    use tracing_subscriber::{layer::Context, prelude::*, registry::LookupSpan, Layer};

    #[derive(Default)]
    struct LoweringSpan(bool);
    impl tracing::field::Visit for LoweringSpan {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            if field.name() == "stage" {
                self.0 = value == "lower";
            }
        }
        fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
    }
    struct Observe {
        control: Arc<QueryBudget>,
        bounds: Arc<Mutex<(u64, u64, usize)>>,
    }
    impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Observe {
        fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
            if attrs.metadata().name() == "sf.compiler.stage" {
                let mut marker = LoweringSpan::default();
                attrs.record(&mut marker);
                if marker.0 {
                    ctx.span(id).unwrap().extensions_mut().insert(marker);
                }
            }
        }
        fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
            if ctx
                .span(id)
                .unwrap()
                .extensions()
                .get::<LoweringSpan>()
                .is_some()
            {
                let mut bounds = self.bounds.lock().unwrap();
                bounds.0 = self.control.consumed(QueryCharge::CompilerWork);
                bounds.2 += 1;
            }
        }
        fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
            if ctx
                .span(id)
                .unwrap()
                .extensions()
                .get::<LoweringSpan>()
                .is_some()
            {
                self.bounds.lock().unwrap().1 = self.control.consumed(QueryCharge::CompilerWork);
            }
        }
    }
    let binding = sf_sparql::CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), maps.to_vec()),
        sf_sql::Dialect::Sqlite,
        sf_sparql::Tbox::default(),
        vec![],
        sf_sparql::cache::Epoch::default(),
        1,
    );
    let control = Arc::new(QueryBudget::new(QueryLimits::new(
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    )));
    let bounds = Arc::new(Mutex::new((0, 0, 0)));
    let observer = tracing_subscriber::registry().with(Observe {
        control: control.clone(),
        bounds: bounds.clone(),
    });
    tracing::subscriber::with_default(observer, || {
        binding
            .compile_shared_with_work_control(query, control.as_ref())
            .unwrap();
    });
    let complete = query.len() as u64 + control.consumed(QueryCharge::CompilerWork);
    // Observe the existing payload-free stage span, not an assumed leaf shape or
    // cold-total subtraction that could absorb later LOWER/cascade work.
    let (start, end, visits) = *bounds.lock().unwrap();
    assert_eq!(visits, 1);
    let independently_lowered = match profile {
        MappedProfile::Scope => compiler_key::lower_scope_work(query, maps),
        MappedProfile::Optional => compiler_key::lowering_work(query, maps),
        MappedProfile::Alias => compiler_key::lower_alias_work(query, maps),
        MappedProfile::Identity
        | MappedProfile::Preparation
        | MappedProfile::Unification
        | MappedProfile::Shape
        | MappedProfile::Materialization
        | MappedProfile::Condition
        | MappedProfile::Bind
        | MappedProfile::Filter => {
            unreachable!("identity uses stage observation, not LOWER calibration")
        }
    };
    assert_eq!(end - start, independently_lowered);
    let prefix = query.len() as u64 + start;
    assert!(prefix > query.len() as u64 + key_work(query) + build_work(query));
    (prefix, query.len() as u64 + end, complete)
}

async fn change_work(cfg: &mut Arc<ServeConfig>, work: u64) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Some(cfg) = Arc::get_mut(cfg) {
                cfg.query_limits = QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX);
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("existing compiler owners release configuration");
}

#[test]
fn mapped_optional_work_refusal_precedes_a_proven_source_admission_boundary() {
    mapped_process(
        "request_compile::tests::optional_work::mapped_optional_work_refusal_precedes_a_proven_source_admission_boundary",
        MappedProfile::Optional,
    );
}

#[test]
fn mapped_lower_scope_work_refusal_and_projection_recovery() {
    mapped_process(
        "request_compile::tests::optional_work::mapped_lower_scope_work_refusal_and_projection_recovery",
        MappedProfile::Scope,
    );
}

#[test]
fn mapped_alias_work_sql_group_and_pool_refusal_recovery() {
    mapped_process(
        "request_compile::tests::optional_work::mapped_alias_work_sql_group_and_pool_refusal_recovery",
        MappedProfile::Alias,
    );
}

fn mapped_process(selector: &str, profile: MappedProfile) {
    const CHILD: &str = "SF_OPTIONAL_TEST_PROCESS";
    const COMPLETED: i32 = 61;
    if let Some(actual) = std::env::var_os(CHILD) {
        assert_eq!(actual, selector, "the exact intended child must execute");
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                if matches!(
                    profile,
                    MappedProfile::Preparation | MappedProfile::Materialization
                ) {
                    preparation::cases(profile).await;
                } else if profile == MappedProfile::Identity {
                    cache_identity::cases().await;
                } else if profile == MappedProfile::Condition {
                    condition::cases().await;
                } else if profile == MappedProfile::Bind {
                    bind::cases().await;
                } else {
                    mapped_admission_cases(profile).await;
                }
            });
        // Only successful execution of every case emits this witness. An exact
        // libtest selector matching zero tests exits 0 and must not pass here.
        std::process::exit(COMPLETED);
    }
    // Compiler spans have process-wide callsite interest. Keep the calibration
    // independent of parallel tests registering/using that same callsite.
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", selector, "--nocapture"])
        .env_clear()
        .env(CHILD, selector)
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(
                status.code(),
                Some(COMPLETED),
                "mapped admission child failed"
            );
            return;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("mapped admission child exceeded test bound");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

async fn mapped_admission_cases(profile: MappedProfile) {
    let maps = mapped_fixture();
    let scope_only = profile == MappedProfile::Scope;
    let queries = match profile {
        MappedProfile::Scope => SCOPE_QUERIES.as_slice(),
        MappedProfile::Optional => QUERIES.as_slice(),
        MappedProfile::Alias => ALIAS_QUERIES.as_slice(),
        MappedProfile::Unification => unification::QUERIES.as_slice(),
        MappedProfile::Filter => filter::QUERIES.as_slice(),
        MappedProfile::Shape => &QUERIES[..2],
        MappedProfile::Identity
        | MappedProfile::Preparation
        | MappedProfile::Materialization
        | MappedProfile::Condition
        | MappedProfile::Bind => {
            unreachable!("separate helper acceptance")
        }
    };
    for (variant, query) in queries.iter().copied().enumerate() {
        let (prefix, normalized, exact) = mapped_work(query, &maps, profile);
        let cached = query.len() as u64
            + if profile == MappedProfile::Filter {
                filter::key_work(query, &maps)
            } else {
                key_work(query)
            };
        assert!(cached < prefix);
        for secured in [false, true] {
            let (mut cfg, pool) = config_with_mapping(prefix, maps.clone());
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
                            "test-only-mapped-optional_work-credential-123456",
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
                        "Bearer test-only-mapped-optional_work-credential-123456",
                    )
                    .body(Body::from(query))
                    .unwrap()
            };
            let held = pool.pick_owned().acquire().await.unwrap();
            let calls = Arc::new(AtomicUsize::new(0));
            let observed = calls.clone();
            let queued = Arc::new(tokio::sync::Notify::new());
            let notify = queued.clone();
            pool.set_admission_pending_observer(move || {
                observed.fetch_add(1, Ordering::SeqCst);
                notify.notify_one();
            });
            for work in [prefix, normalized - 1, normalized - 1] {
                change_work(&mut cfg, work).await;
                let response = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    crate::router(cfg.clone()).oneshot(request()),
                )
                .await
                .expect("LOWER rejects before held source")
                .unwrap();
                assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
                let bytes = response.into_body().collect().await.unwrap().to_bytes();
                let problem: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(problem["code"], "query-budget-exceeded");
                assert_eq!(calls.load(Ordering::SeqCst), 0);
            }
            drop(held);
            // The same runtime must also admit a warm query funded only for input
            // and its cache key: reuse skips compilation, but not source admission.
            for (index, work) in [exact, cached].into_iter().enumerate() {
                change_work(&mut cfg, work).await;
                let held = pool.pick_owned().acquire().await.unwrap();
                let queued = if index == 0 {
                    // Refusals must leave the original one-shot observer uncalled.
                    queued.clone()
                } else {
                    let queued = Arc::new(tokio::sync::Notify::new());
                    let notify = queued.clone();
                    let observed = calls.clone();
                    pool.set_admission_pending_observer(move || {
                        observed.fetch_add(1, Ordering::SeqCst);
                        notify.notify_one();
                    });
                    queued
                };
                let mut response = Box::pin(crate::router(cfg.clone()).oneshot(request()));
                tokio::time::timeout(std::time::Duration::from_secs(2), async {
                tokio::select! {
                    _ = queued.notified() => (),
                    result = &mut response => panic!("paid mapped request bypassed held source: {result:?}"),
                }
            }).await.expect("cold and warm requests reach the same held source admission");
                assert_eq!(calls.load(Ordering::SeqCst), index + 1);
                drop(held);
                let response = tokio::time::timeout(std::time::Duration::from_secs(2), response)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let bytes = response.into_body().collect().await.unwrap().to_bytes();
                let result: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                if profile == MappedProfile::Alias {
                    assert_eq!(result["head"]["vars"], serde_json::json!(["value"]));
                    let rows = result["results"]["bindings"].as_array().unwrap();
                    let mut values: Vec<_> = rows
                        .iter()
                        .map(|row| {
                            // SPARQL JSON omits xsd:string for a simple literal.
                            assert_eq!(row["value"]["type"], "literal");
                            assert!(row["value"].get("datatype").is_none());
                            assert!(row["value"].get("xml:lang").is_none());
                            row["value"]["value"].as_str().unwrap()
                        })
                        .collect();
                    values.sort();
                    assert_eq!(values, ["one", "two"]);
                } else {
                    let field = if scope_only && variant > 0 {
                        "renamed"
                    } else {
                        "value"
                    };
                    let head = if scope_only {
                        if variant == 2 {
                            vec!["item", "renamed"]
                        } else {
                            vec![field]
                        }
                    } else {
                        vec!["value", "optional"]
                    };
                    assert_eq!(result["head"]["vars"], serde_json::json!(head));
                    if scope_only && variant == 2 {
                        let mut items: Vec<_> = result["results"]["bindings"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|row| {
                                (
                                    row["item"]["value"].as_str().unwrap(),
                                    row[field]["value"].as_str().unwrap(),
                                )
                            })
                            .collect();
                        items.sort();
                        assert_eq!(
                            items,
                            vec![
                                ("http://example.test/item/1", "one"),
                                ("http://example.test/item/2", "one"),
                                ("http://example.test/item/3", "two")
                            ]
                        );
                    }
                    let mut values: Vec<_> = result["results"]["bindings"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|row| {
                            (
                                row[field]["value"].as_str().unwrap(),
                                row.get("optional").map(|v| v["value"].as_str().unwrap()),
                            )
                        })
                        .collect();
                    values.sort();
                    let mut expected = if profile == MappedProfile::Filter {
                        vec![("one", Some("x")), ("one", None), ("two", None)]
                    } else if scope_only {
                        vec![("one", None), ("one", None), ("two", None)]
                    } else {
                        vec![("one", Some("x")), ("one", None), ("two", Some("y"))]
                    };
                    if !scope_only && variant == 1 {
                        expected.push(("one", Some("x")));
                        if profile != MappedProfile::Filter {
                            expected.push(("two", Some("y")));
                        }
                    }
                    expected.sort();
                    assert_eq!(values, expected, "variant {variant}");
                }
                let recovered = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    cfg.compiler_permits().acquire_many_owned(4),
                )
                .await
                .expect("all compiler capacity recovers")
                .unwrap();
                drop(recovered);
                let source = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    pool.pick_owned().acquire(),
                )
                .await
                .expect("source admission recovers")
                .unwrap();
                drop(source);
            }
        }
    }
}
