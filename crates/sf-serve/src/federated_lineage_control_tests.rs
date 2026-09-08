use super::*;
use crate::{
    PortableRowPolicy, PortableRowRule, ProvisionedBearerAdmission, ProvisionedBearerSubject,
    QueryAdmission,
};

#[tokio::test]
async fn lineage_portable_callers_isolate_both_sources_and_policy_identity() {
    let (mut cfg, _pools, _files) =
        protected_config([&["a-left", "b-left"], &["a-right", "b-right"]]);
    let second = "test-only-other-lineage-credential-123456";
    let subject = |id: &str, token: &str| {
        ProvisionedBearerSubject::portable_rows(
            id,
            token,
            PortableRowPolicy::new(vec![
                PortableRowRule::new(0, "items", "value", format!("{id}-left")).unwrap(),
                PortableRowRule::new(1, "items", "value", format!("{id}-right")).unwrap(),
            ])
            .unwrap(),
        )
        .unwrap()
    };
    cfg.set_query_admission(QueryAdmission::ProvisionedBearers(
        ProvisionedBearerAdmission::new(vec![subject("a", TOKEN), subject("b", second)]).unwrap(),
    ));
    let cfg = Arc::new(cfg);
    let mut policies = Vec::new();
    for (token, present, absent) in [
        (TOKEN, "a-", "b-"),
        (second, "b-", "a-"),
        (TOKEN, "a-", "b-"),
    ] {
        let mut request = lineage_request(UNION_REVERSED);
        request.headers_mut().insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        let response = router(cfg.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(
            !text.contains(&format!("{absent}left"))
                && !text.contains(&format!("{absent}right"))
                && !text.contains(token)
        );
        let rows = parse(&bytes);
        assert_eq!(
            rows[1]["result"]["results"]["bindings"][0]["right"]["value"],
            format!("{present}right")
        );
        assert_eq!(
            rows[2]["result"]["results"]["bindings"][0]["left"]["value"],
            format!("{present}left")
        );
        assert_eq!([source(&rows[1]), source(&rows[2])], [1, 0]);
        policies.push(rows[0]["policy"].clone());
    }
    assert_eq!(policies[0], policies[2]);
    // Policy lineage names the registry snapshot, never a caller/cache partition.
    assert_eq!(policies[0], policies[1]);
}

#[tokio::test]
async fn lineage_auth_and_uncovered_policy_reject_without_source_admission() {
    let (mut cfg, pools, _files) = protected_config([&["one"], &["two"]]);
    let policy = PortableRowPolicy::new(vec![
        PortableRowRule::new(0, "items", "value", "one").unwrap(),
        PortableRowRule::new(1, "other", "value", "two").unwrap(),
    ])
    .unwrap();
    cfg.set_query_admission(QueryAdmission::ProvisionedBearers(
        ProvisionedBearerAdmission::new(vec![ProvisionedBearerSubject::portable_rows(
            "a", TOKEN, policy,
        )
        .unwrap()])
        .unwrap(),
    ));
    let _left = pools[0].pick_owned().acquire().await.unwrap();
    let _right = pools[1].pick_owned().acquire().await.unwrap();
    let io = Arc::new(AtomicUsize::new(0));
    for pool in pools {
        let io = io.clone();
        pool.set_admission_pending_observer(move || {
            io.fetch_add(1, Ordering::SeqCst);
        });
    }
    let cfg = Arc::new(cfg);
    for auth in [false, true] {
        let mut request = lineage_request(UNION_SAME_VAR);
        if !auth {
            request.headers_mut().remove(header::AUTHORIZATION);
        }
        let response =
            tokio::time::timeout(Duration::from_secs(1), router(cfg.clone()).oneshot(request))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            response.status(),
            if auth {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::UNAUTHORIZED
            }
        );
        assert_eq!(io.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn lineage_union_shares_result_and_exact_serialized_byte_limits() {
    let (cfg, _pools, _files) = protected_config([&["same"], &["same"]]);
    let response = router(Arc::new(cfg))
        .oneshot(lineage_request(UNION_SAME_VAR))
        .await
        .unwrap();
    let size = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .len() as u64;
    for (results, bytes, succeeds) in [
        (0, u64::MAX, false),
        (1, u64::MAX, false),
        (2, size, true),
        (2, size - 1, false),
    ] {
        let (mut cfg, _pools, _files) = protected_config([&["same"], &["same"]]);
        cfg.query_limits = QueryLimits::new(u64::MAX, u64::MAX, results, bytes);
        let response = router(Arc::new(cfg))
            .oneshot(lineage_request(UNION_SAME_VAR))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await;
        assert_eq!(body.is_ok(), succeeds, "results={results}, bytes={bytes}");
        if let Err(error) = body {
            assert_eq!(error.to_string(), "result stream failed");
        }
    }
}

#[tokio::test]
async fn lineage_union_acquisition_and_second_source_failures_release_cap_one() {
    let (mut cfg, pools, files) = protected_config([&["one"], &["two"]]);
    cfg.set_max_concurrent_requests(1).unwrap();
    cfg.timeout = Duration::from_millis(250);
    let cfg = Arc::new(cfg);
    let held = pools[1].pick_owned().acquire().await.unwrap();
    let response = router(cfg.clone())
        .oneshot(lineage_request(UNION_SAME_VAR))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    drop(held);
    assert_eq!(
        records(cfg.clone(), UNION_SAME_VAR).await.last().unwrap()["solutions"],
        2
    );
    for index in [0, 1] {
        files[index].execute("DROP TABLE items");
        let response = router(cfg.clone())
            .oneshot(lineage_request(UNION_SAME_VAR))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .into_body()
                .collect()
                .await
                .unwrap_err()
                .to_string(),
            "result stream failed"
        );
        files[index].execute(
            "CREATE TABLE items(value TEXT NOT NULL); INSERT INTO items VALUES ('recovered')",
        );
        let recovered =
            tokio::time::timeout(Duration::from_secs(2), records(cfg.clone(), UNION_SAME_VAR))
                .await
                .unwrap();
        assert_eq!(recovered.last().unwrap()["solutions"], 2);
    }
}

#[tokio::test]
async fn lineage_union_pins_both_generation_origins_through_activation() {
    let (cfg, _pools, _files) = protected_config([&["old-left"], &["old-right"]]);
    let cfg = Arc::new(cfg);
    let baseline = records(cfg.clone(), UNION_SAME_VAR).await;
    let response = router(cfg.clone())
        .oneshot(lineage_request(UNION_REVERSED))
        .await
        .unwrap();
    let (left, _pool, _left_file) = runtime_source(0, "http://example.test/left", &["new-left"]);
    let (right, _pool, _right_file) =
        runtime_source(1, "http://example.test/right", &["new-right"]);
    cfg.activate_snapshot(
        cfg.runtime_readiness().unwrap(),
        RuntimeSnapshot::new(
            Epoch(1),
            crate::test_support::ontology(
                &[],
                &["http://example.test/left", "http://example.test/right"],
            ),
            vec![left, right],
        )
        .unwrap(),
    )
    .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let old = parse(&bytes);
    assert_eq!(old[0]["snapshot"], baseline[0]["snapshot"]);
    assert_eq!(old[0]["sources"], baseline[0]["sources"]);
    assert_ne!(old[0]["logicalPlan"], baseline[0]["logicalPlan"]);
    assert_eq!(
        old[1]["result"]["results"]["bindings"][0]["right"]["value"],
        "old-right"
    );
    let new = records(cfg, UNION_SAME_VAR).await;
    assert_ne!(old[0]["snapshot"], new[0]["snapshot"]);
    assert_ne!(old[0]["sources"], new[0]["sources"]);
    assert_eq!(
        new[1]["result"]["results"]["bindings"][0]["value"]["value"],
        "new-left"
    );
}

#[tokio::test]
async fn second_arm_witness_overflow_cannot_complete_a_partial_union() {
    let (cfg, _pools, files) = protected_config([&["one"], &["two"]]);
    files[1].execute("DELETE FROM items; WITH RECURSIVE n(v) AS (SELECT 1 UNION ALL SELECT v+1 FROM n WHERE v<1025) INSERT INTO items SELECT CAST(v AS TEXT) FROM n");
    let cfg = Arc::new(cfg);
    let response = router(cfg.clone())
        .oneshot(lineage_request(UNION_SAME_VAR))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .into_body()
            .collect()
            .await
            .unwrap_err()
            .to_string(),
        "result stream failed"
    );
    files[1].execute("DELETE FROM items; INSERT INTO items VALUES ('recovered')");
    assert_eq!(
        records(cfg, UNION_SAME_VAR).await.last().unwrap()["solutions"],
        2
    );
}
