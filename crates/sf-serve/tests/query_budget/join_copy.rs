//! Public accounting and recovery for right-heavy inner-join payloads.

use super::*;

#[tokio::test]
async fn right_heavy_join_rejects_then_recovers_with_exact_bag_and_cache_hits() {
    let payload = "λ".repeat(1024);
    let query = format!(
        "SELECT ?a ?b WHERE {{ VALUES ?a {{ 0 1 }} VALUES ?b {{ \"{payload}\" \"{payload}!\" }} }}"
    );
    // More than input admission alone, but insufficient for the repeated right payload.
    let mut cfg = Arc::new(protected(
        query.len() as u64 + key_work(&query) + super::cache_key::build_work(&query) + 512,
    ));
    assert_budget_problem(
        router(cfg.clone())
            .oneshot(authenticated(&query))
            .await
            .unwrap(),
    )
    .await;
    assert_values(
        router(cfg.clone())
            .oneshot(authenticated(SELECT))
            .await
            .unwrap(),
    )
    .await;
    Arc::get_mut(&mut cfg).unwrap().query_limits =
        QueryLimits::new(100_000, u64::MAX, u64::MAX, u64::MAX);
    for warm in [false, true] {
        if warm {
            // A completed shared-plan hit need not repeat either branch copy.
            Arc::get_mut(&mut cfg).unwrap().query_limits = QueryLimits::new(
                query.len() as u64 + key_work(&query),
                u64::MAX,
                u64::MAX,
                u64::MAX,
            );
        }
        let response = router(cfg.clone())
            .oneshot(authenticated(&query))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let mut tuples: Vec<_> = json["results"]["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                assert_eq!(row["b"]["type"], "literal");
                (
                    row["a"]["value"].as_str().unwrap().to_owned(),
                    row["b"]["value"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        tuples.sort();
        let mut expected: Vec<_> = ["0", "1"]
            .into_iter()
            .flat_map(|a| [payload.clone(), format!("{payload}!")].map(|b| (a.to_owned(), b)))
            .collect();
        expected.sort();
        assert_eq!(tuples, expected);
    }
}
