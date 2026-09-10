use super::*;

#[path = "../support/compiler_key.rs"]
mod compiler_key;
pub(super) use compiler_key::key_work;

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
    let mut cfg = Arc::new(protected(10_000));
    // The same immutable runtime/cache survives all requests and limit changes.
    for warm in [false, true] {
        if warm {
            set_work_after_cleanup(&mut cfg, SELECT.len() as u64 + key_work(SELECT)).await;
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
    // A single VALUES leaf has no branch product: input plus exact key work.
    let query = "SELECT ?value WHERE { VALUES ?value { \"one\" \"two\" } } # café";
    let wire = form_urlencoded::Serializer::new(String::new())
        .append_pair("query", query)
        .finish();
    for method in ["GET", "POST"] {
        for (work, accepted) in [
            (query.len() as u64 - 1, false),
            (query.len() as u64 + key_work(query) - 1, false),
            (query.len() as u64 + key_work(query), true),
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
            let exact = query.len() as u64 + key_work(&query);
            set_work_after_cleanup(&mut cfg, exact - 1).await;
            assert_budget_problem(
                router(cfg.clone())
                    .oneshot(authenticated(&query))
                    .await
                    .unwrap(),
            )
            .await;
            set_work_after_cleanup(&mut cfg, exact).await;
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
