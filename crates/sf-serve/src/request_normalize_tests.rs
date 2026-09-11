use super::*;
use http_body_util::BodyExt;
use std::sync::atomic::{AtomicUsize, Ordering};

const MAPPED_QUERY: &str =
    "SELECT ?value WHERE { { SELECT ?value WHERE { ?item <http://example.test/a> ?value } } }";

fn mapped_fixture() -> Vec<sf_core::ir::TriplesMap> {
    sf_mapping::parse_r2rml(
        r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        <http://example.test/normalization> a rr:TriplesMap;
          rr:logicalTable [rr:tableName "items"];
          rr:subjectMap [rr:template "http://example.test/item/{id}"];
          rr:predicateObjectMap [rr:predicate <http://example.test/a>;
            rr:objectMap [rr:column "value"; rr:datatype <http://www.w3.org/2001/XMLSchema#string>]].
    "#,
    )
    .unwrap()
}

fn mapped_work(maps: &[sf_core::ir::TriplesMap], phase: &'static str) -> (u64, u64, u64) {
    use sf_core::query_control::QueryBudget;
    use std::sync::Mutex;
    use tracing::{
        span::{Attributes, Id},
        Subscriber,
    };
    use tracing_subscriber::{layer::Context, prelude::*, registry::LookupSpan, Layer};

    struct NormalizationSpan(bool, &'static str);
    impl tracing::field::Visit for NormalizationSpan {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            if field.name() == "stage" {
                self.0 = value == self.1;
            }
        }
        fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
    }
    struct Observe {
        phase: &'static str,
        control: Arc<QueryBudget>,
        bounds: Arc<Mutex<(u64, u64, usize)>>,
    }
    impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Observe {
        fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
            if attrs.metadata().name() == "sf.compiler.stage" {
                let mut marker = NormalizationSpan(false, self.phase);
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
                .get::<NormalizationSpan>()
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
                .get::<NormalizationSpan>()
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
        phase,
        control: control.clone(),
        bounds: bounds.clone(),
    });
    tracing::subscriber::with_default(observer, || {
        binding
            .compile_shared_with_work_control(MAPPED_QUERY, control.as_ref())
            .unwrap();
    });
    let complete = MAPPED_QUERY.len() as u64 + control.consumed(QueryCharge::CompilerWork);
    // Observe the existing payload-free stage span, not an assumed leaf shape or
    // cold-total subtraction that could absorb later LOWER/cascade work.
    let (start, end, visits) = *bounds.lock().unwrap();
    assert_eq!(visits, 1);
    if phase == "normalize" {
        assert_eq!(end - start, normalization_work(MAPPED_QUERY, maps));
    }
    assert!(end > start, "selected phase must perform paid work");
    let prefix = MAPPED_QUERY.len() as u64 + start;
    assert!(prefix > MAPPED_QUERY.len() as u64 + key_work(MAPPED_QUERY) + build_work(MAPPED_QUERY));
    (prefix, MAPPED_QUERY.len() as u64 + end, complete)
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
fn mapped_normalization_refusal_precedes_a_proven_source_admission_boundary() {
    mapped_process("request_compile::tests::structural_normalization::mapped_normalization_refusal_precedes_a_proven_source_admission_boundary", "normalize");
}

pub(super) fn mapped_process(selector: &str, phase: &'static str) {
    const CHILD: &str = "SF_NORMALIZATION_TEST_PROCESS";
    const COMPLETED: i32 = 61;
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(mapped_admission_cases(phase));
        // Only successful execution of every case emits this witness. An exact
        // libtest selector matching zero tests exits 0 and must not pass here.
        std::process::exit(COMPLETED);
    }
    // Compiler spans have process-wide callsite interest. Keep the calibration
    // independent of parallel tests registering/using that same callsite.
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", selector, "--nocapture"])
        .env_clear()
        .env(CHILD, "1")
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

async fn mapped_admission_cases(phase: &'static str) {
    let maps = mapped_fixture();
    let (prefix, normalized, exact) = mapped_work(&maps, phase);
    let cached = MAPPED_QUERY.len() as u64 + key_work(MAPPED_QUERY);
    assert!(cached < prefix);
    for secured in [false, true] {
        let (mut cfg, pool) = config_with_mapping(prefix, maps.clone());
        pool.pick()
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TABLE items (id INTEGER, value TEXT); \
             INSERT INTO items VALUES (1, 'one'), (2, 'one'), (3, 'two');",
            )
            .unwrap();
        if secured {
            Arc::get_mut(&mut cfg)
                .unwrap()
                .set_query_admission(crate::QueryAdmission::Bearer(
                    crate::BearerQueryAdmission::for_service_principal(
                        "test-only-mapped-normalization-credential-123456",
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
                    "Bearer test-only-mapped-normalization-credential-123456",
                )
                .body(Body::from(MAPPED_QUERY))
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
            .expect("NORMALIZE rejects before held source")
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
            let mut values: Vec<_> = result["results"]["bindings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["value"]["value"].as_str().unwrap())
                .collect();
            values.sort();
            assert_eq!(values, ["one", "one", "two"]);
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

#[tokio::test]
async fn structural_normalization_rejects_before_held_source_and_recovers_capacity() {
    for query in compiler_key::STRUCTURAL_QUERIES {
        let prefix = query.len() as u64 + key_work(query) + rewrite_work(query) + build_work(query);
        let normalization = normalization_work(query, &[]);
        let complete = query.len() as u64 + compiler_key::structural_compile_work(query);
        assert!(complete >= prefix + normalization);
        for secured in [false, true] {
            for work in [prefix, prefix + normalization - 1] {
                let (mut cfg, pool) = config(work);
                if secured {
                    Arc::get_mut(&mut cfg).unwrap().set_query_admission(
                        crate::QueryAdmission::Bearer(
                            crate::BearerQueryAdmission::for_service_principal(
                                "test-only-structural-work-credential-123456",
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
                        "Bearer test-only-structural-work-credential-123456",
                    )
                    .body(Body::from(query))
                    .unwrap();
                let response = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    crate::router(cfg.clone()).oneshot(request),
                )
                .await
                .expect("NORMALIZE must reject before source admission")
                .unwrap();
                assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
                let bytes = response.into_body().collect().await.unwrap().to_bytes();
                let problem: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(problem["code"], "query-budget-exceeded");
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
