//! Paid RDF-star rewrite/realization through ordinary and authenticated HTTP paths.
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
    "SELECT ?s WHERE { ?s <http://example.test/a> ?o FILTER(isTRIPLE(TRIPLE(<urn:a>, <urn:p>, ?o))) }",
    "SELECT ?s WHERE { ?s <http://example.test/a> ?o VALUES ?t { <<( <urn:a> <urn:p> <urn:b> )>> <<( <urn:a> <urn:p> <urn:b> )>> } }",
    "SELECT ?s WHERE { { ?s <http://example.test/a> ?o BIND(TRIPLE(<urn:a>, <urn:p>, ?o) AS ?t) } UNION { ?s <http://example.test/a> ?o BIND(?o AS ?t) } FILTER(isTRIPLE(?t)) }",
];

const REALIZATION: [&str; 3] = [
    "SELECT ?t WHERE { ?s <http://example.test/a> ?o BIND(TRIPLE(<urn:a>, <urn:p>, TRIPLE(<urn:b>, <urn:q>, ?o)) AS ?t) }",
    "CONSTRUCT { <urn:result> <urn:quoted> ?t } WHERE { ?s <http://example.test/a> ?o BIND(TRIPLE(<urn:a>, <urn:p>, TRIPLE(<urn:b>, <urn:q>, ?o)) AS ?t) }",
    "SELECT ?t WHERE { ?s <http://example.test/a> ?o . { SELECT DISTINCT ?t WHERE { ?x <http://example.test/a> ?y BIND(TRIPLE(<urn:a>, <urn:p>, TRIPLE(<urn:b>, <urn:q>, ?y)) AS ?t) } } }",
];

fn mapping() -> Vec<sf_core::ir::TriplesMap> {
    sf_mapping::parse_r2rml(
        r#"
      @prefix rr: <http://www.w3.org/ns/r2rml#> .
      <http://example.test/star-work> a rr:TriplesMap;
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
    realization: bool,
}
impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Observe {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if self.realization && attrs.metadata().name() == "sf.compiler.star_realization" {
            ctx.span(id).unwrap().extensions_mut().insert(RewriteSpan {
                relevant: true,
                index: 0,
            });
        } else if !self.realization && attrs.metadata().name() == "sf.compiler.stage" {
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

fn work(query: &str, realization: bool) -> (u64, u64, u64, u64) {
    let binding = sf_sparql::CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), mapping()),
        sf_sql::Dialect::Sqlite,
        Default::default(),
        vec![],
        Default::default(),
        64, // Match RuntimeBinding's cache geometry, not a capacity-one cache.
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
            realization,
        }),
        || {
            binding
                .compile_shared_with_work_control(query, control.as_ref())
                .inspect(|plan| crate::admission::admit(plan, 0, control.as_ref()).unwrap())
                .unwrap()
        },
    );
    let input = query.len() as u64;
    let total = input + control.consumed(QueryCharge::CompilerWork);
    let (start, end) = {
        let spans = bounds.lock().unwrap();
        assert_eq!(
            spans.len(),
            1,
            "one root realization or initial rewrite span"
        );
        let (start, end) = spans[0];
        assert!(end > start + 1, "actual RDF-star phase work is paid");
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
            realization,
        }),
        || {
            binding
                .compile_shared_with_work_control(query, warm.as_ref())
                .inspect(|plan| crate::admission::admit(plan, 0, warm.as_ref()).unwrap())
                .unwrap()
        },
    );
    assert!(
        Arc::ptr_eq(&raw, &hit),
        "warm requests retain shared cache identity"
    );
    assert!(
        bounds.lock().unwrap().is_empty(),
        "a warm hit skips initial rewrite"
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
fn star_rewrite_refuses_before_source_and_recovers_exact_cold_and_warm_bags() {
    isolated(false);
}

#[test]
fn star_realization_refuses_before_source_and_recovers_exact_graph_bags_and_subplans() {
    isolated(true);
}

fn isolated(realization: bool) {
    const CHILD: &str = "SF_STAR_WORK_TEST_PROCESS";
    const COMPLETED: i32 = 67;
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(cases(realization));
        std::process::exit(COMPLETED);
    }
    // Isolate tracing's global callsite interest from parallel subscriber tests.
    let thread = std::thread::current();
    let name = thread.name().expect("named test");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
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
                "zero matching tests must not pass"
            );
            return;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("RDF-star acceptance child exceeded its bound");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

async fn cases(realization: bool) {
    for (case, query) in (if realization { REALIZATION } else { QUERIES })
        .into_iter()
        .enumerate()
    {
        let (start, end, total, warm) = work(query, realization);
        // Bindings realization is the final paid translation phase for these
        // fixtures, so its end may equal the cold compilation total.
        // Warm equality pays canonical comparison; the cold rewrite prefix
        // has only an empty lookup, so warm need not be <= that prefix.
        assert!(warm < total && start < end && end <= total,
            "case={case} realization={realization}: warm={warm} start={start} end={end} total={total}");
        if !realization {
            assert!(end < total);
        }
        assert!(
            total <= 1_000_000,
            "unchanged default admits the declared RDF-star corpus"
        );
        for secured in [false, true] {
            let (mut cfg, pool) = config_with_mapping(start, mapping());
            // Two distinct RDF objects project to the same subject; identical
            // source triples correctly collapse under RDF graph set semantics.
            pool.pick().lock().unwrap().execute_batch("CREATE TABLE items (id INTEGER, value TEXT); INSERT INTO items VALUES (1, 'one'), (1, 'another'), (2, 'two');").unwrap();
            if secured {
                Arc::get_mut(&mut cfg)
                    .unwrap()
                    .set_query_admission(crate::QueryAdmission::Bearer(
                        crate::BearerQueryAdmission::for_service_principal(
                            "test-only-star-credential-123456",
                        )
                        .unwrap(),
                    ));
            }
            let request = || {
                Request::post("/sparql")
                    .header("content-type", "application/sparql-query")
                    .header(
                        "accept",
                        if realization && case == 1 {
                            "text/turtle"
                        } else {
                            "application/sparql-results+json"
                        },
                    )
                    .header("authorization", "Bearer test-only-star-credential-123456")
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
                if realization {
                    assert_realization(case, &bytes);
                } else {
                    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    let mut actual: Vec<_> = json["results"]["bindings"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|row| {
                            assert_eq!(row["s"]["type"], "uri");
                            row["s"]["value"].as_str().unwrap().to_owned()
                        })
                        .collect();
                    let repetitions = if case == 1 { 2 } else { 1 };
                    let mut expected: Vec<_> = (0..repetitions)
                        .flat_map(|_| [1, 1, 2])
                        .map(|id| format!("http://example.test/item/{id}"))
                        .collect();
                    actual.sort();
                    expected.sort();
                    assert_eq!(
                        actual, expected,
                        "source and VALUES duplicates retain exact bags"
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

fn assert_realization(case: usize, bytes: &[u8]) {
    if case == 1 {
        let mut actual: Vec<_> = oxttl::TurtleParser::new()
            .for_slice(bytes)
            .map(|t| t.unwrap().to_string())
            .collect();
        let expected = ["one", "another", "two"].map(|value| format!(
            "<urn:result> <urn:quoted> <<( <urn:a> <urn:p> <<( <urn:b> <urn:q> \"{value}\" )>> )>> ."
        )).join("\n");
        let mut expected: Vec<_> = oxttl::TurtleParser::new()
            .for_slice(expected.as_bytes())
            .map(|t| t.unwrap().to_string())
            .collect();
        actual.sort();
        expected.sort();
        assert_eq!(
            actual, expected,
            "CONSTRUCT realizes the exact nested triple graph"
        );
    } else {
        let json: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        let mut actual: Vec<_> = json["results"]["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let t = &row["t"];
                assert_eq!(t["type"], "triple");
                assert_eq!(t["value"]["subject"]["value"], "urn:a");
                assert_eq!(t["value"]["predicate"]["value"], "urn:p");
                let inner = &t["value"]["object"];
                assert_eq!(inner["type"], "triple");
                assert_eq!(inner["value"]["subject"]["value"], "urn:b");
                assert_eq!(inner["value"]["predicate"]["value"], "urn:q");
                assert_eq!(inner["value"]["object"]["type"], "literal");
                inner["value"]["object"]["value"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        let mut expected: Vec<_> = (0..if case == 2 { 3 } else { 1 })
            .flat_map(|_| ["one".to_owned(), "another".to_owned(), "two".to_owned()])
            .collect();
        actual.sort();
        expected.sort();
        assert_eq!(
            actual, expected,
            "nested DISTINCT remap preserves the exact outer bag"
        );
    }
}
