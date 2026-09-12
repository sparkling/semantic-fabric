use super::*;

#[path = "../support/compiler_key.rs"]
mod compiler_key;
pub(super) use compiler_key::{
    admission_work, build_work, key_work, miss_work, normalization_work, rewrite_work,
    source_free_entry_work, source_free_join_seed_work, source_free_values_work, warm_work,
    CONSTANT_QUERIES, STRUCTURAL_QUERIES,
};

// Match the owned HTTP fixture's observed table. Success-only calibration;
// earlier BUILD/NORMALIZE/LOWER refusal cutpoints remain independently counted.
pub(super) fn fixture_compile_work(source: &str) -> u64 {
    let mut table = sf_sql::TableSchema::new("items");
    table.columns = vec![
        sf_sql::Column::new("id", "INTEGER", false),
        sf_sql::Column::new("value", "TEXT", true),
    ];
    table.primary_key = vec!["id".into()];
    compiler_key::source_free_compile_work_with_schema(source, vec![table])
}

pub(super) fn constant_compile_work(source: &str) -> u64 {
    assert!(CONSTANT_QUERIES.contains(&source));
    fixture_compile_work(source)
}

pub(super) fn structural_compile_work(source: &str) -> u64 {
    assert!(STRUCTURAL_QUERIES.contains(&source));
    fixture_compile_work(source)
}

async fn set_work_after_cleanup(cfg: &mut Arc<ServeConfig>, work: u64) {
    // Terminal failure wakes the response before the blocking compiler closure
    // necessarily drops its configuration. Preserve the same runtime/cache and
    // require bounded cleanup, not scheduler-dependent immediate Arc uniqueness.
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if let Some(cfg) = Arc::get_mut(cfg) {
                cfg.query_limits = QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX);
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("request workers must release configuration within the cleanup bound");
}

#[tokio::test]
async fn changing_budget_waits_for_the_existing_configuration_owner() {
    use std::{future::Future, task::Poll};
    let mut cfg = Arc::new(protected(10_000));
    let identity = Arc::as_ptr(&cfg);
    let worker = cfg.clone();
    let mut change = Box::pin(set_work_after_cleanup(&mut cfg, 123));
    std::future::poll_fn(|cx| {
        assert!(change.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(worker.query_limits.max_compiler_work(), 10_000);
    drop(worker);
    change.await;
    assert_eq!(Arc::as_ptr(&cfg), identity);
    assert_eq!(cfg.query_limits.max_compiler_work(), 123);
}

#[tokio::test]
async fn authenticated_cold_and_warm_cache_obey_compiler_allowance() {
    // Cold acceptance uses the existing production default; exact warm and
    // unpaid key/lookup boundaries below remain independently budgeted.
    let mut cfg = Arc::new(protected(1_000_000));
    // The same immutable runtime/cache survives all requests and limit changes.
    for warm in [false, true] {
        if warm {
            set_work_after_cleanup(
                &mut cfg,
                SELECT.len() as u64
                    + warm_work(SELECT)
                    + admission_work(SELECT, &sf_mapping::parse_r2rml(MAPPING).unwrap()),
            )
            .await;
        }
        assert_values(
            router(cfg.clone())
                .oneshot(authenticated(SELECT))
                .await
                .unwrap(),
        )
        .await;
    }
    set_work_after_cleanup(&mut cfg, 0).await;
    assert_budget_problem(router(cfg).oneshot(authenticated(SELECT)).await.unwrap()).await;
    assert_budget_problem(
        router(Arc::new(protected(0)))
            .oneshot(authenticated(SELECT))
            .await
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn compiler_input_allowance_counts_decoded_utf8_not_form_encoding() {
    // Earlier input/key/BUILD cutpoints remain independent of the success-only
    // complete schedule; exact/N-1 checks cover cold completion as well.
    let query = "SELECT ?value WHERE { VALUES ?value { \"one\" \"two\" } } # café";
    let wire = form_urlencoded::Serializer::new(String::new())
        .append_pair("query", query)
        .finish();
    let complete = query.len() as u64 + fixture_compile_work(query);
    for method in ["GET", "POST"] {
        for (work, accepted) in [
            (query.len() as u64 - 1, false),
            (query.len() as u64 + key_work(query) - 1, false),
            (
                query.len() as u64
                    + key_work(query)
                    + miss_work(query)
                    + rewrite_work(query)
                    + build_work(query)
                    - 1,
                false,
            ),
            (complete - 1, false),
            (complete, true),
        ] {
            let req = Request::builder()
                .method(method)
                .uri(if method == "GET" {
                    format!("/sparql?{wire}")
                } else {
                    "/sparql".into()
                })
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(if method == "GET" {
                    Body::empty()
                } else {
                    Body::from(wire.clone())
                })
                .unwrap();
            let response = router(Arc::new(protected(work)))
                .oneshot(req)
                .await
                .unwrap();
            if accepted {
                assert_values(response).await;
            } else {
                assert_budget_problem(response).await;
            }
        }
    }
}

#[tokio::test]
async fn prefix_expanded_utf8_key_is_paid_on_cold_and_warm_public_paths() {
    let iri = format!("http://example.test/{}", "λ".repeat(256));
    let query =
        format!("PREFIX ex: <{iri}> SELECT ?value WHERE {{ VALUES ?value {{ ex:a ex:b }} }}");
    for secured in [false, true] {
        let mut cfg = Arc::new(if secured {
            protected(100_000)
        } else {
            config(QueryLimits::new(100_000, u64::MAX, u64::MAX, u64::MAX))
        });
        for warm in [false, true] {
            let exact = query.len() as u64
                + if warm {
                    warm_work(&query)
                } else {
                    fixture_compile_work(&query)
                };
            let tail = admission_work(&query, &[]);
            let compiler_cut = if warm { exact - 1 } else { exact - tail - 1 };
            set_work_after_cleanup(&mut cfg, compiler_cut).await;
            assert_budget_problem(
                router(cfg.clone())
                    .oneshot(authenticated(&query))
                    .await
                    .unwrap(),
            )
            .await;
            set_work_after_cleanup(&mut cfg, exact + if warm { tail } else { 0 }).await;
            let response = router(cfg.clone())
                .oneshot(authenticated(&query))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "secured={secured} warm={warm}"
            );
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let mut values: Vec<_> = json["results"]["bindings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| {
                    assert_eq!(row["value"]["type"], "uri");
                    row["value"]["value"].as_str().unwrap().to_owned()
                })
                .collect();
            values.sort();
            assert_eq!(values, [format!("{iri}a"), format!("{iri}b")]);
        }
    }
}
