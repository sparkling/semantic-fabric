use super::*;

/// Independent logical schedule: invocation, fragment bytes, geometric growth
/// target and old payload, then the one final full-content hash.
pub(super) fn key_work(source: &str) -> u64 {
    use std::fmt::{self, Write};
    struct Counter {
        length: usize,
        capacity: usize,
        work: u64,
    }
    impl Write for Counter {
        fn write_str(&mut self, fragment: &str) -> fmt::Result {
            self.work += fragment.len() as u64;
            let next = self.length + fragment.len();
            if next > self.capacity {
                self.capacity = next.max(64).max(2 * self.capacity);
                self.work += (self.capacity + self.length) as u64;
            }
            self.length = next;
            Ok(())
        }
    }
    let query = spargebra::SparqlParser::new().parse_query(source).unwrap();
    let mut counter = Counter {
        length: 0,
        capacity: 0,
        work: 1,
    };
    write!(&mut counter, "{query}").unwrap();
    counter.work + counter.length as u64
}

#[tokio::test]
async fn authenticated_cold_and_warm_cache_obey_compiler_allowance() {
    let mut cfg = Arc::new(protected(10_000));
    // The same immutable runtime/cache survives all requests and limit changes.
    for warm in [false, true] {
        if warm {
            Arc::get_mut(&mut cfg).unwrap().query_limits = QueryLimits::new(
                SELECT.len() as u64 + key_work(SELECT),
                u64::MAX,
                u64::MAX,
                u64::MAX,
            );
        }
        assert_values(
            router(cfg.clone())
                .oneshot(authenticated(SELECT))
                .await
                .unwrap(),
        )
        .await;
    }
    Arc::get_mut(&mut cfg)
        .expect("response released configuration")
        .query_limits = QueryLimits::new(0, u64::MAX, u64::MAX, u64::MAX);
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
            Arc::get_mut(&mut cfg).unwrap().query_limits =
                QueryLimits::new(exact - 1, u64::MAX, u64::MAX, u64::MAX);
            assert_budget_problem(
                router(cfg.clone())
                    .oneshot(authenticated(&query))
                    .await
                    .unwrap(),
            )
            .await;
            Arc::get_mut(&mut cfg).unwrap().query_limits =
                QueryLimits::new(exact, u64::MAX, u64::MAX, u64::MAX);
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
