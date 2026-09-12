//! Public cache admission: paid lookup, unpaid publication and exact recovery.
use super::*;
use http_body_util::BodyExt;
use sf_core::query_control::QueryBudget;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) async fn allowance(cfg: &mut Arc<ServeConfig>, work: u64) {
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
    .expect("compiler ownership must recover");
}

#[tokio::test]
async fn cache_lookup_and_publication_refuse_before_source_then_recover_exact_results() {
    const QUERY: &str = "SELECT ?value WHERE { ?s <http://example.test/a> ?value }";
    const TOKEN: &str = "test-only-cache-operations-credential-123456";
    let maps = sf_mapping::parse_r2rml(r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        <urn:items> a rr:TriplesMap; rr:logicalTable [rr:tableName "items"];
          rr:subjectMap [rr:template "http://example.test/item/{id}"];
          rr:predicateObjectMap [rr:predicate <http://example.test/a>; rr:objectMap [rr:column "value"; rr:datatype <http://www.w3.org/2001/XMLSchema#string>]].
    "#).unwrap();
    let binding = sf_sparql::CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), maps.clone()),
        sf_sql::Dialect::Sqlite,
        Default::default(),
        vec![],
        Default::default(),
        64,
    );
    let budget = || QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    let cold = budget();
    let plan = binding
        .compile_shared_with_work_control(QUERY, &cold)
        .unwrap();
    let hit = budget();
    let reused = binding
        .compile_shared_with_work_control(QUERY, &hit)
        .unwrap();
    assert!(Arc::ptr_eq(&plan, &reused));
    let input = QUERY.len() as u64;
    let admission = compiler_key::plan_admission_work(&plan);
    let cold = input + cold.consumed(QueryCharge::CompilerWork);
    let hit = input + hit.consumed(QueryCharge::CompilerWork);
    let lookup = input + key_work(QUERY) + compiler_key::miss_work(QUERY);
    assert!(cold <= 1_000_000 && lookup < hit && hit < cold);
    for secured in [false, true] {
        let (mut cfg, pool) = config_with_mapping(0, maps.clone());
        pool.pick()
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TABLE items(id INTEGER, value TEXT); INSERT INTO items VALUES(1,'one');",
            )
            .unwrap();
        if secured {
            Arc::get_mut(&mut cfg)
                .unwrap()
                .set_query_admission(crate::QueryAdmission::Bearer(
                    crate::BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
                ));
        }
        let request = || {
            Request::post("/sparql")
                .header("content-type", "application/sparql-query")
                .header("accept", "application/sparql-results+json")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::from(QUERY))
                .unwrap()
        };
        // N-1 cold publication must not create a hit, even after repeated attempts.
        // Once admitted, N-1 warm equality must still refuse before source access.
        let mut observer = None;
        for (work, accepted) in [
            (lookup - 1, false),
            (cold - 1, false),
            (cold - 1, false),
            (hit, false),
            (cold + admission, true),
            (hit - 1, false),
            (hit + admission, true),
        ] {
            allowance(&mut cfg, work).await;
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
            let mut response = Box::pin(crate::router(cfg.clone()).oneshot(request()));
            if accepted {
                tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    tokio::select! {
                        _ = queued.notified() => (),
                        result = &mut response => panic!("funded cache path bypassed source: {result:?}"),
                    }
                }).await.expect("funded query reaches held source");
                assert_eq!(calls.load(Ordering::SeqCst), 1);
            }
            if accepted {
                drop(held);
            }
            let result = tokio::time::timeout(std::time::Duration::from_secs(2), response)
                .await
                .expect("refusal must not wait for source")
                .unwrap();
            assert_eq!(
                result.status(),
                if accepted {
                    StatusCode::OK
                } else {
                    StatusCode::TOO_MANY_REQUESTS
                }
            );
            let bytes = result.into_body().collect().await.unwrap().to_bytes();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            if accepted {
                assert_eq!(
                    body["results"]["bindings"],
                    serde_json::json!([{"value":{"type":"literal","value":"one"}}])
                );
                observer = None;
            } else {
                assert_eq!(body["code"], "query-budget-exceeded");
                assert_eq!(calls.load(Ordering::SeqCst), 0);
            }
        }
    }
}
