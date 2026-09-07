use super::*;
use crate::budget::RequestBudget;
use crate::{BearerQueryAdmission, QueryAdmission};

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
