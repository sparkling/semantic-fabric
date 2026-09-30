//! Security-ordering tests for the generated-query profile: subject admission is
//! first, bearer subjects stay isolated, and no request value is authority.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::http::{header, HeaderMap, HeaderValue, StatusCode};

use crate::budget::RequestBudget;
use crate::generated_http_test_support::*;
use crate::{
    BearerQueryAdmission, PortableRowPolicy, PortableRowRule, PostgresRlsClaims,
    ProvisionedBearerAdmission, ProvisionedBearerSubject, QueryAdmission, QueryShapeProfile,
    RuntimeSource, ServeConfig,
};

const GENERATED: QueryShapeProfile = QueryShapeProfile::GeneratedSelectAsk;
const TOKEN_A: &str = "test-only-generated-profile-token-A-0123456789";
const TOKEN_B: &str = "test-only-generated-profile-token-B-9876543210";
const TOKEN_C: &str = "test-only-generated-profile-token-C-5555555555";
const WRONG: &str = "wrong-token-wrong-token-wrong-token-1234";
const STALE: &str = "sfgp1:0000000000000000000000000000000000000000000000000000000000000000";
const LINEAGE: &str = "application/vnd.semantic-fabric.lineage+json-seq";
const MIXED_LINEAGE: &str = "Application/Vnd.Semantic-Fabric.Lineage+json-seq;q=0.5";

fn subject(id: &str, token: &str, table: &str, tenant: &str) -> ProvisionedBearerSubject {
    let rule = PortableRowRule::new(0, table, "tenant", tenant).unwrap();
    let policy = PortableRowPolicy::new(vec![rule]).unwrap();
    ProvisionedBearerSubject::portable_rows(id, token, policy).unwrap()
}

fn admission(subjects: Vec<ProvisionedBearerSubject>) -> QueryAdmission {
    QueryAdmission::ProvisionedBearers(ProvisionedBearerAdmission::new(subjects).unwrap())
}

fn two_tenants() -> QueryAdmission {
    admission(vec![
        subject("alice", TOKEN_A, "people", "a"),
        subject("bob", TOKEN_B, "people", "b"),
    ])
}

fn with_eve() -> QueryAdmission {
    admission(vec![
        subject("alice", TOKEN_A, "people", "a"),
        subject("eve", TOKEN_C, "other", "a"),
    ])
}

/// A valid service principal restricted to PostgreSQL RLS claims. The SQLite
/// fixtures cannot enforce source RLS, so every request must be denied.
fn postgres_rls() -> QueryAdmission {
    let settings = BTreeMap::from([("app.tenant_id".to_owned(), "a".to_owned())]);
    let claims = PostgresRlsClaims::new(settings).unwrap();
    let principal = BearerQueryAdmission::for_service_principal(TOKEN_A).unwrap();
    QueryAdmission::Bearer(principal.with_postgres_rls(claims).unwrap())
}

fn federated_rls() -> Arc<ServeConfig> {
    let sources = [0_usize, 1].map(|index| {
        let (observed, _) = source();
        RuntimeSource::new(observed, mapping(index))
    });
    let mut cfg = ServeConfig::new_federated(sources, ontology(&[])).unwrap();
    cfg.set_query_admission(postgres_rls());
    cfg.set_query_shape_profile(GENERATED);
    Arc::new(cfg)
}

fn assert_policy_denied(reply: &Reply, label: &str) {
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{label}");
    assert!(reply.identity().is_none(), "{label}");
    let content_type = reply.headers.get(header::CONTENT_TYPE).unwrap();
    assert_eq!(content_type, "application/problem+json", "{label}");
    let json = reply.json();
    assert_eq!(json["code"], "access-denied", "{label}");
    assert!(json.get("rule").is_none(), "{label}");
    let text = reply.text();
    for leak in ["http://ex", "people", "lineage", "federation"] {
        assert!(!text.contains(leak), "{label}: {text}");
    }
}

/// A request budget carrying exactly the subject bundle the credential selects.
fn authenticated(cfg: &ServeConfig, token: &str) -> RequestBudget {
    let mut headers = HeaderMap::new();
    let value = HeaderValue::from_str(&format!("Bearer {token}")).unwrap();
    headers.insert(header::AUTHORIZATION, value);
    let admitted = cfg.query_admission.admit(&headers).unwrap();
    let mut budget = cfg.request_budget();
    budget.retain_authenticated(admitted).unwrap();
    budget
}

#[tokio::test]
async fn bearer_subjects_stay_isolated_cold_and_warm_with_one_identity() {
    let (cfg, _) = config(GENERATED, two_tenants());
    let mut identities = Vec::new();
    for (token, present, absent) in [(TOKEN_A, "Alice", "Bob"), (TOKEN_B, "Bob", "Alice")] {
        for _ in 0..2 {
            let reply = send(&cfg, post(SELECT, Some(token), None, &[])).await;
            assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
            let text = reply.text();
            assert!(text.contains(present), "{text}");
            assert!(!text.contains(absent), "{text}");
            identities.push(reply.identity().expect("identity"));
        }
    }
    assert_wire(&identities[0]);
    assert!(identities.iter().all(|wire| wire == &identities[0]));
}

#[tokio::test]
async fn authentication_failure_precedes_everything_and_a_supplied_identity_grants_nothing() {
    let (cfg, _) = config(GENERATED, two_tenants());
    let issued = send(&cfg, post(SELECT, Some(TOKEN_A), None, &[])).await;
    let issued = issued.identity().unwrap();
    let extra = [(HEADER, issued.as_str())];
    for token in [None, Some(WRONG)] {
        for query in [SELECT, CONSTRUCT, "not sparql"] {
            let reply = send(&cfg, post(query, token, None, &extra)).await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{query}");
            assert!(reply.identity().is_none());
            let text = reply.text();
            assert!(!text.contains("rule"), "{text}");
            assert!(!text.contains(&issued), "{text}");
        }
    }
}

#[tokio::test]
async fn a_request_supplied_identity_is_never_echoed_or_trusted() {
    let (cfg, _) = config(GENERATED, two_tenants());
    let issued = send(&cfg, post(SELECT, Some(TOKEN_A), None, &[])).await;
    let issued = issued.identity().unwrap();
    let stale = [(HEADER, STALE)];
    let reply = send(&cfg, post(SELECT, Some(TOKEN_A), None, &stale)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.identity().unwrap(), issued);

    let echoed = [(HEADER, issued.as_str())];
    let reply = send(&cfg, post(CONSTRUCT, Some(TOKEN_A), None, &echoed)).await;
    assert_refusal(&reply, "construct-form");

    let (ordinary, _) = config(QueryShapeProfile::Ordinary, open());
    let reply = send(&ordinary, post(SELECT, None, None, &echoed)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.identity().is_none());
}

#[tokio::test]
async fn row_policy_denial_precedes_generated_refusal_detail() {
    let (cfg, _) = config(GENERATED, with_eve());
    for query in [SELECT, CONSTRUCT] {
        let reply = send(&cfg, post(query, Some(TOKEN_C), None, &[])).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{query}");
        assert!(reply.identity().is_none());
        let json = reply.json();
        assert_eq!(json["code"], "access-denied");
        assert!(json.get("rule").is_none());
    }
    let reply = send(&cfg, post(CONSTRUCT, Some(TOKEN_A), None, &[])).await;
    assert_refusal(&reply, "construct-form");
}

#[tokio::test]
async fn row_policy_denial_precedes_lineage_refusal_without_source_work() {
    let (cfg, pool) = config(GENERATED, with_eve());
    let held = pool.pick_owned().acquire().await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    pool.set_admission_pending_observer(move || {
        observed.fetch_add(1, Ordering::SeqCst);
    });
    for accept in [LINEAGE, MIXED_LINEAGE] {
        for query in [SELECT, ASK] {
            let request = send(&cfg, post(query, Some(TOKEN_C), Some(accept), &[]));
            let reply = tokio::time::timeout(Duration::from_secs(2), request)
                .await
                .expect("denial must not wait for the source");
            assert_policy_denied(&reply, &format!("{query} {accept}"));
        }
        let request = send(&cfg, post(SELECT, Some(TOKEN_A), Some(accept), &[]));
        let reply = tokio::time::timeout(Duration::from_secs(2), request)
            .await
            .expect("refusal must not wait for the source");
        assert_refusal(&reply, "lineage-unsupported");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop(held);
}

#[tokio::test]
async fn source_rls_denial_precedes_generated_lineage_and_form_refusals() {
    let (cfg, _) = config(GENERATED, postgres_rls());
    for accept in [None, Some(LINEAGE), Some(MIXED_LINEAGE)] {
        for query in [SELECT, ASK, CONSTRUCT, "not sparql"] {
            let reply = send(&cfg, post(query, Some(TOKEN_A), accept, &[])).await;
            assert_policy_denied(&reply, &format!("{query} {accept:?}"));
        }
    }
    // Authentication still precedes the source-RLS policy decision.
    let reply = send(&cfg, post(SELECT, Some(WRONG), Some(LINEAGE), &[])).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(reply.identity().is_none());
}

#[tokio::test]
async fn source_rls_denial_precedes_generated_federation_refusal() {
    let cfg = federated_rls();
    for accept in [None, Some(LINEAGE)] {
        for query in [SELECT, "not sparql"] {
            let reply = send(&cfg, post(query, Some(TOKEN_A), accept, &[])).await;
            assert_policy_denied(&reply, &format!("federated {query} {accept:?}"));
        }
    }
}

#[tokio::test]
async fn deny_admission_never_reaches_the_generated_profile() {
    let (cfg, _) = config(GENERATED, QueryAdmission::Deny);
    for query in [SELECT, "not sparql"] {
        let reply = send(&cfg, post(query, Some(TOKEN_A), None, &[])).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{query}");
        assert!(reply.identity().is_none());
        assert!(!reply.text().contains("rule"));
    }
}

#[test]
fn denied_and_secured_requests_parse_at_most_once_per_compiler_pass() {
    isolated_async(|| async {
        let (cfg, _) = config(GENERATED, two_tenants());
        for token in [None, Some(WRONG)] {
            for query in [SELECT, CONSTRUCT, "not sparql"] {
                let (reply, parsed) = counted(&cfg, post(query, token, None, &[])).await;
                assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{query}");
                assert_eq!(parsed, 0, "{query}");
            }
        }
        let secured = [
            (SELECT, StatusCode::OK),
            (SELECT, StatusCode::OK),
            (ASK, StatusCode::OK),
            (CONSTRUCT, StatusCode::NOT_IMPLEMENTED),
            ("not sparql", StatusCode::NOT_IMPLEMENTED),
        ];
        for (query, status) in secured {
            let (reply, parsed) = counted(&cfg, post(query, Some(TOKEN_A), None, &[])).await;
            assert_eq!(reply.status, status, "{query}");
            assert_eq!(parsed, 1, "{query}");
        }

        // A budget without the admitted subject is denied on both passes unparsed.
        let lease = cfg.runtime_lease().unwrap();
        reset_parse_count();
        let preflight = crate::request_compile::preflight(
            cfg.clone(),
            lease.clone(),
            SELECT.into(),
            cfg.request_budget(),
        )
        .await;
        let denied = preflight.err().expect("denied preflight");
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let compiled = crate::generated_request::compile(
            cfg.clone(),
            lease.clone(),
            SELECT.into(),
            cfg.request_budget(),
            None,
        )
        .await;
        let denied = compiled.err().expect("denied compile");
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        assert_eq!(parse_count(), 0);

        // An admitted subject parses once in each compiler pass.
        let budget = authenticated(&cfg, TOKEN_A);
        reset_parse_count();
        let reservation = crate::request_compile::preflight(
            cfg.clone(),
            lease.clone(),
            SELECT.into(),
            budget.clone(),
        )
        .await
        .expect("admitted preflight");
        assert_eq!(parse_count(), 1);
        reset_parse_count();
        let compiled = crate::generated_request::compile(
            cfg.clone(),
            lease,
            SELECT.into(),
            budget,
            Some(reservation),
        )
        .await;
        assert!(compiled.is_ok());
        assert_eq!(parse_count(), 1);

        // Row-policy denial reuses the one parse and never reparses to deny.
        let (cfg, _) = config(GENERATED, with_eve());
        for query in [SELECT, CONSTRUCT] {
            let (reply, parsed) = counted(&cfg, post(query, Some(TOKEN_C), None, &[])).await;
            assert_eq!(reply.status, StatusCode::FORBIDDEN, "{query}");
            assert_eq!(parsed, 1, "{query}");
            let lease = cfg.runtime_lease().unwrap();
            reset_parse_count();
            let preflight = crate::request_compile::preflight(
                cfg.clone(),
                lease.clone(),
                query.to_owned(),
                authenticated(&cfg, TOKEN_C),
            )
            .await;
            let denied = preflight.err().expect("row-denied preflight");
            assert_eq!(denied.status(), StatusCode::FORBIDDEN, "{query}");
            assert_eq!(parse_count(), 1, "{query} preflight");
            reset_parse_count();
            let compiled = crate::generated_request::compile(
                cfg.clone(),
                lease,
                query.to_owned(),
                authenticated(&cfg, TOKEN_C),
                None,
            )
            .await;
            let denied = compiled.err().expect("row-denied compile");
            assert_eq!(denied.status(), StatusCode::FORBIDDEN, "{query}");
            assert_eq!(parse_count(), 1, "{query} authoritative");
        }

        // A lineage request authorizes rows once, then ends: denial or refusal.
        let lineage = [
            (SELECT, TOKEN_C, StatusCode::FORBIDDEN),
            (ASK, TOKEN_C, StatusCode::FORBIDDEN),
            (SELECT, TOKEN_A, StatusCode::NOT_IMPLEMENTED),
        ];
        for accept in [LINEAGE, MIXED_LINEAGE] {
            for (query, token, status) in lineage {
                let request = post(query, Some(token), Some(accept), &[]);
                let (reply, parsed) = counted(&cfg, request).await;
                assert_eq!(reply.status, status, "{query} {accept}");
                assert!(reply.identity().is_none(), "{query} {accept}");
                assert_eq!(parsed, 1, "{query} {accept}");
            }
        }

        // Source-RLS denial precedes parse, lineage and form refusal detail.
        let (cfg, _) = config(GENERATED, postgres_rls());
        for accept in [None, Some(LINEAGE)] {
            for query in [SELECT, "not sparql"] {
                let request = post(query, Some(TOKEN_A), accept, &[]);
                let (reply, parsed) = counted(&cfg, request).await;
                assert_eq!(reply.status, StatusCode::FORBIDDEN, "{query} {accept:?}");
                assert_eq!(parsed, 0, "{query} {accept:?}");
            }
        }

        let (cfg, _) = config(GENERATED, QueryAdmission::Deny);
        for query in [SELECT, "not sparql"] {
            let (reply, parsed) = counted(&cfg, post(query, Some(TOKEN_A), None, &[])).await;
            assert_eq!(reply.status, StatusCode::FORBIDDEN, "{query}");
            assert_eq!(parsed, 0, "{query}");
        }
    });
}
