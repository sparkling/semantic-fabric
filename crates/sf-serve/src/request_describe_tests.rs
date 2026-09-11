//! Paid DESCRIBE form rewriting through ordinary and authenticated HTTP paths.
use super::*;
use http_body_util::BodyExt;
use sf_core::query_control::QueryBudget;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use tracing::{span::Attributes, Id, Subscriber};
use tracing_subscriber::{layer::Context, prelude::*, registry::LookupSpan, Layer};

const QUERIES: [&str; 3] = [
    "DESCRIBE <http://example.test/item/1>",
    "DESCRIBE ?s WHERE { VALUES ?s { <http://example.test/item/1> <http://example.test/item/1> } }",
    "DESCRIBE ?s WHERE { ?s ?__sf_describe_predicate_0 ?__sf_describe_object_0 FILTER(?s = <http://example.test/item/1>) }",
];

fn mapping() -> Vec<sf_core::ir::TriplesMap> {
    sf_mapping::parse_r2rml(
        r#"
      @prefix rr: <http://www.w3.org/ns/r2rml#> .
      <http://example.test/describe-work> a rr:TriplesMap;
        rr:logicalTable [rr:tableName "items"];
        rr:subjectMap [rr:template "http://example.test/item/{id}"];
        rr:predicateObjectMap [rr:predicate <http://example.test/a>;
          rr:objectMap [rr:column "value"; rr:datatype <http://www.w3.org/2001/XMLSchema#string>]].
    "#,
    )
    .unwrap()
}

#[derive(Default)]
struct RewriteSpan {
    relevant: bool,
    index: usize,
}
impl tracing::field::Visit for RewriteSpan {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "stage" {
            self.relevant = value == "rewrite";
        }
    }
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
}
struct Observe {
    control: Arc<QueryBudget>,
    bounds: Arc<Mutex<Vec<(u64, u64)>>>,
}
impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Observe {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if attrs.metadata().name() == "sf.compiler.stage" {
            let mut marker = RewriteSpan::default();
            attrs.record(&mut marker);
            if marker.relevant {
                ctx.span(id).unwrap().extensions_mut().insert(marker);
            }
        }
    }
    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).unwrap();
        let mut extensions = span.extensions_mut();
        if let Some(marker) = extensions.get_mut::<RewriteSpan>() {
            let mut bounds = self.bounds.lock().unwrap();
            marker.index = bounds.len();
            let consumed = self.control.consumed(QueryCharge::CompilerWork);
            bounds.push((consumed, consumed));
        }
    }
    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).unwrap();
        let extensions = span.extensions();
        if let Some(marker) = extensions.get::<RewriteSpan>() {
            self.bounds.lock().unwrap()[marker.index].1 =
                self.control.consumed(QueryCharge::CompilerWork);
        }
    }
}

fn work(query: &str) -> (u64, u64, u64, u64) {
    let binding = sf_sparql::CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), mapping()),
        sf_sql::Dialect::Sqlite,
        Default::default(),
        vec![],
        Default::default(),
        1,
    );
    let control = Arc::new(QueryBudget::new(QueryLimits::new(
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    )));
    let bounds = Arc::new(Mutex::new(Vec::new()));
    let raw = tracing::subscriber::with_default(
        tracing_subscriber::registry().with(Observe {
            control: control.clone(),
            bounds: bounds.clone(),
        }),
        || {
            binding
                .compile_shared_with_work_control(query, control.as_ref())
                .unwrap()
        },
    );
    let input = query.len() as u64;
    let total = input + control.consumed(QueryCharge::CompilerWork);
    let (start, end) = {
        let spans = bounds.lock().unwrap();
        assert_eq!(
            spans.len(),
            2,
            "initial star and DESCRIBE form are distinct rewrite spans"
        );
        let (start, end) = spans[1];
        assert!(end > start + 1, "actual DESCRIBE form work is paid");
        (input + start, input + end)
    };
    bounds.lock().unwrap().clear();
    let warm = Arc::new(QueryBudget::new(QueryLimits::new(
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    )));
    let hit = tracing::subscriber::with_default(
        tracing_subscriber::registry().with(Observe {
            control: warm.clone(),
            bounds: bounds.clone(),
        }),
        || {
            binding
                .compile_shared_with_work_control(query, warm.as_ref())
                .unwrap()
        },
    );
    assert!(
        Arc::ptr_eq(&raw, &hit),
        "constant-target parser binders retain shared cache identity"
    );
    assert!(
        bounds.lock().unwrap().is_empty(),
        "a warm hit performs neither rewrite"
    );
    (
        start,
        end,
        total,
        input + warm.consumed(QueryCharge::CompilerWork),
    )
}

async fn set_work(cfg: &mut Arc<ServeConfig>, work: u64) {
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
    .expect("compiler owners release configuration");
}

#[test]
fn describe_form_refuses_before_source_and_recovers_exact_cold_and_warm_graphs() {
    const CHILD: &str = "SF_DESCRIBE_WORK_TEST_PROCESS";
    const COMPLETED: i32 = 67;
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(cases());
        std::process::exit(COMPLETED);
    }
    // Isolate tracing's global callsite interest from parallel subscriber tests.
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "request_compile::tests::describe_work::describe_form_refuses_before_source_and_recovers_exact_cold_and_warm_graphs", "--nocapture"])
        .env_clear().env(CHILD, "1").spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(
                status.code(),
                Some(COMPLETED),
                "zero matching tests must not pass"
            );
            return;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("DESCRIBE acceptance child exceeded its bound");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

async fn cases() {
    for query in QUERIES {
        let (start, end, total, warm) = work(query);
        for _ in 0..8 {
            assert_eq!(
                work(query),
                (start, end, total, warm),
                "fresh parser binders cannot change exact HTTP admission budgets"
            );
        }
        assert!(warm <= start && start < end && end < total);
        assert!(
            total <= 1_000_000,
            "unchanged default admits the declared DESCRIBE corpus"
        );
        for secured in [false, true] {
            let (mut cfg, pool) = config_with_mapping(start, mapping());
            pool.pick().lock().unwrap().execute_batch("CREATE TABLE items (id INTEGER, value TEXT); INSERT INTO items VALUES (1, 'one'), (1, 'one'), (2, 'two');").unwrap();
            if secured {
                Arc::get_mut(&mut cfg)
                    .unwrap()
                    .set_query_admission(crate::QueryAdmission::Bearer(
                        crate::BearerQueryAdmission::for_service_principal(
                            "test-only-describe-credential-123456",
                        )
                        .unwrap(),
                    ));
            }
            let request = || {
                Request::post("/sparql")
                    .header("content-type", "application/sparql-query")
                    .header("accept", "text/turtle")
                    .header(
                        "authorization",
                        "Bearer test-only-describe-credential-123456",
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
            for limit in [start, end - 1, end - 1] {
                set_work(&mut cfg, limit).await;
                let response = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    crate::router(cfg.clone()).oneshot(request()),
                )
                .await
                .expect("unpaid form fails before held source")
                .unwrap();
                assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
                let bytes = response.into_body().collect().await.unwrap().to_bytes();
                let problem: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(problem["code"], "query-budget-exceeded");
                assert_eq!(calls.load(Ordering::SeqCst), 0);
            }
            drop(held);
            for (index, limit) in [total, warm].into_iter().enumerate() {
                set_work(&mut cfg, limit).await;
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
                    tokio::select! { _ = queued.notified() => (), result = &mut response => panic!("funded form bypassed source admission: {result:?}") }
                }).await.expect("funded cold and key-only warm requests reach held source");
                assert_eq!(calls.load(Ordering::SeqCst), index + 1);
                drop(held);
                let response = tokio::time::timeout(std::time::Duration::from_secs(2), response)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let bytes = response.into_body().collect().await.unwrap().to_bytes();
                let triples = oxttl::TurtleParser::new()
                    .for_slice(&bytes)
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                assert_eq!(
                    triples,
                    vec![oxrdf::Triple::new(
                        oxrdf::NamedNode::new("http://example.test/item/1").unwrap(),
                        oxrdf::NamedNode::new("http://example.test/a").unwrap(),
                        oxrdf::Literal::new_simple_literal("one"),
                    )],
                    "duplicate source rows/targets still produce an exact graph set"
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
