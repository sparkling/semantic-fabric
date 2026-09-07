use super::*;
use crate::budget::RequestBudget;
use crate::{
    BearerQueryAdmission, PortableRowPolicy, PortableRowRule, ProvisionedBearerAdmission,
    ProvisionedBearerSubject, QueryAdmission,
};

const TOKEN: &str = "test-only-federation-principal-0123456789";

#[tokio::test]
#[allow(
    clippy::await_holding_lock,
    reason = "deliberately retain the source lock to prove authentication rejects without source admission"
)]
async fn protected_union_rejects_before_pool_acquisition_and_then_serves_both_sources() {
    let (mut cfg, pools, _files) = config(
        ["http://example.test/left", "http://example.test/right"],
        [&["first"], &["second"]],
    );
    cfg.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    let app = router(Arc::new(cfg));
    let held = pools[0].pick();
    let guard = held.lock().unwrap();
    let response = tokio::time::timeout(
        Duration::from_secs(1),
        app.clone().oneshot(request(UNION_SAME_VAR)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    drop(guard);
    let mut authorized = request(UNION_SAME_VAR);
    authorized.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().unwrap(),
    );
    let response = app.oneshot(authorized).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("first") && text.contains("second"));
}

#[tokio::test]
async fn portable_subjects_isolate_both_public_union_fragments() {
    let (mut cfg, _pools, _files) = config(
        ["http://example.test/left", "http://example.test/right"],
        [&["a-left", "b-left"], &["a-right", "b-right"]],
    );
    let subject = |id: &str, token: &str, values: [&str; 2]| {
        ProvisionedBearerSubject::portable_rows(
            id,
            token,
            PortableRowPolicy::new(vec![
                PortableRowRule::new(0, "items", "value", values[0]).unwrap(),
                PortableRowRule::new(1, "items", "value", values[1]).unwrap(),
            ])
            .unwrap(),
        )
        .unwrap()
    };
    let second = "test-only-federation-principal-9876543210";
    cfg.set_query_admission(QueryAdmission::ProvisionedBearers(
        ProvisionedBearerAdmission::new(vec![
            subject("a", TOKEN, ["a-left", "a-right"]),
            subject("b", second, ["b-left", "b-right"]),
        ])
        .unwrap(),
    ));
    let app = router(Arc::new(cfg));
    for (token, present, absent) in [(TOKEN, "a-", "b-"), (second, "b-", "a-")] {
        let mut request = request(UNION_SAME_VAR);
        request.headers_mut().insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let text = String::from_utf8(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert_eq!(text.matches(present).count(), 2, "{text}");
        assert!(!text.contains(absent), "{text}");
    }
}

#[tokio::test]
async fn uncovered_portable_union_policy_rejects_before_source_io() {
    let (mut cfg, pools, _files) = config(
        ["http://example.test/left", "http://example.test/right"],
        [&["left"], &["right"]],
    );
    let policy = PortableRowPolicy::new(vec![
        PortableRowRule::new(0, "items", "value", "left").unwrap(),
        PortableRowRule::new(1, "other", "value", "right").unwrap(),
    ])
    .unwrap();
    cfg.set_query_admission(QueryAdmission::ProvisionedBearers(
        ProvisionedBearerAdmission::new(vec![ProvisionedBearerSubject::portable_rows(
            "subject", TOKEN, policy,
        )
        .unwrap()])
        .unwrap(),
    ));
    let io = Arc::new(AtomicUsize::new(0));
    for pool in &pools {
        let io = io.clone();
        pool.set_admission_pending_observer(move || {
            io.fetch_add(1, Ordering::SeqCst);
        });
    }
    let mut authorized = request(UNION_SAME_VAR);
    authorized.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().unwrap(),
    );
    let response = router(Arc::new(cfg)).oneshot(authorized).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(io.load(Ordering::SeqCst), 0, "rejection touched a pool");
}

#[test]
fn protected_fragments_keep_identical_context_without_reusing_raw_caches() {
    let (mut cfg, _pools, _files) = config(
        ["http://example.test/left", "http://example.test/right"],
        [&["first"], &["second"]],
    );
    cfg.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().unwrap(),
    );
    let context = cfg.query_admission.authenticate(&headers).unwrap().unwrap();
    let mut budget = RequestBudget::after(Duration::from_secs(10), crate::DEFAULT_QUERY_LIMITS);
    budget.retain_security(context).unwrap();
    let snapshot = cfg.runtime_lease().unwrap();
    let sources = [SourceId::new(0).unwrap(), SourceId::new(1).unwrap()];
    let policy = cfg.query_admission.policy().unwrap();
    let first = snapshot
        .compile_federated_secured(sources, UNION_SAME_VAR, &budget, policy)
        .unwrap();
    let second = snapshot
        .compile_federated_secured(sources, UNION_SAME_VAR, &budget, policy)
        .unwrap();
    for (a, b) in first
        .plan()
        .fragments()
        .iter()
        .zip(second.plan().fragments())
    {
        assert!(!Arc::ptr_eq(&a.shared_plan(), &b.shared_plan()));
    }
    let (_, fragments) = first.into_bound_plans();
    assert!(fragments
        .iter()
        .all(|fragment| fragment.security == Some(context)));
    let raw_budget = RequestBudget::after(Duration::from_secs(10), crate::DEFAULT_QUERY_LIMITS);
    assert!(snapshot
        .prepare_federated_request_execution(second, &raw_budget)
        .is_err());
}
