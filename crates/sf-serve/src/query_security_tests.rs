use super::*;
use crate::{Backend, IntrospectedSource, ServeConfig};
use sf_core::SourceId;
use std::sync::Arc;
use std::time::Duration;

const TOKEN: &str = "test-only-service-principal-0123456789";
const OTHER: &str = "test-only-service-principal-9876543210";
const QUERY: &str = "SELECT ?name WHERE { ?s <http://ex/name> ?name }";

fn profile(token: &str) -> QueryAdmission {
    QueryAdmission::Bearer(BearerQueryAdmission::for_service_principal(token).unwrap())
}

fn context(profile: &QueryAdmission, token: &str) -> SecurityContext {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        format!("Bearer {token}").parse().unwrap(),
    );
    profile.authenticate(&headers).unwrap().unwrap()
}

fn budget(context: Option<SecurityContext>) -> RequestBudget {
    let mut budget = RequestBudget::after(Duration::from_secs(10), crate::DEFAULT_QUERY_LIMITS);
    if let Some(context) = context {
        budget.retain_security(context).unwrap();
    }
    budget
}

pub(super) fn config() -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE people(id INTEGER, name TEXT); INSERT INTO people VALUES (1,'Alice');",
    )
    .unwrap();
    let source = IntrospectedSource::observe_sqlite(Backend::sqlite(conn)).unwrap();
    let ontology = crate::test_support::ontology(&[], &["http://ex/name"]);
    let mapping = r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
      <#p> a rr:TriplesMap; rr:logicalTable [rr:tableName "people"];
      rr:subjectMap [rr:template "http://ex/{id}"];
      rr:predicateObjectMap [rr:predicate <http://ex/name>; rr:objectMap [rr:column "name"]]."#;
    let mut cfg = ServeConfig::from_authored_r2rml(source, mapping, ontology).unwrap();
    cfg.use_in_process_test_parser();
    cfg.set_query_admission(profile(TOKEN));
    cfg
}

#[test]
fn context_is_immutable_after_handoff_and_survives_every_budget_clone() {
    let policy = profile(TOKEN);
    let trusted = context(&policy, TOKEN);
    let mut budget = budget(Some(trusted));
    assert!(budget.retain_security(trusted).is_err());
    let clone = budget.clone();
    assert_eq!(clone.security_context(), Some(trusted));
    assert!(budget.retain_security(trusted).is_err());
    assert_eq!(policy.validate(&clone), Ok(()));
    assert_eq!(
        QueryAdmission::UnrestrictedDevelopment.validate(&clone),
        Err(ProblemCode::AccessDenied)
    );
}

#[tokio::test]
async fn missing_or_rotated_context_rejects_before_preflight_and_generation_work() {
    let cfg = Arc::new(config());
    let snapshot = cfg.runtime_lease().unwrap();
    for request in [budget(None), budget(Some(context(&profile(OTHER), OTHER)))] {
        let error =
            crate::request_generation::acquire(cfg.clone(), &snapshot, "malformed", &request)
                .await
                .err()
                .unwrap();
        assert_eq!(error.status(), axum::http::StatusCode::FORBIDDEN);
        let error = crate::request_compile::preflight(
            cfg.clone(),
            snapshot.clone(),
            "malformed".into(),
            request,
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error.status(), axum::http::StatusCode::FORBIDDEN);
    }
}

#[test]
fn secure_plans_cannot_execute_with_another_request_identity_or_raw_context() {
    let cfg = config();
    let snapshot = cfg.runtime_lease().unwrap();
    let policy = cfg.query_admission.policy().unwrap();
    let trusted = context(&cfg.query_admission, TOKEN);
    let source = SourceId::new(0).unwrap();
    let admitted = budget(Some(trusted));
    for other in [budget(None), budget(Some(context(&profile(OTHER), OTHER)))] {
        let plan = snapshot
            .compile_secured(source, QUERY, &admitted, policy)
            .unwrap();
        assert!(snapshot.prepare_request_execution(plan, &other).is_err());
    }
    let raw = snapshot.compile(source, QUERY, &admitted).unwrap();
    assert!(snapshot.prepare_request_execution(raw, &admitted).is_err());
    let secure = snapshot
        .compile_secured(source, QUERY, &admitted, policy)
        .unwrap();
    assert!(snapshot
        .prepare_request_execution(secure, &admitted)
        .is_ok());
    // Even a syntactically invalid query cannot get past the policy check.
    let other_policy = profile(OTHER).policy().unwrap();
    let error = snapshot
        .compile_secured(source, "malformed", &admitted, other_policy)
        .unwrap_err();
    assert!(matches!(error, sf_sparql::Error::Mapping(_)));
}

#[test]
fn served_plan_cache_reuses_only_an_exact_security_partition() {
    let cfg = config();
    let snapshot = cfg.runtime_lease().unwrap();
    let source = SourceId::new(0).unwrap();
    let policy = cfg.query_admission.policy().unwrap();
    let trusted = context(&cfg.query_admission, TOKEN);
    let first = snapshot
        .compile_secured(source, QUERY, &budget(Some(trusted)), policy)
        .unwrap();
    let hit = snapshot
        .compile_secured(source, QUERY, &budget(Some(trusted)), policy)
        .unwrap();
    assert!(std::ptr::eq(first.plan(), hit.plan()));
    let changed_subject = SecurityContext::new(
        policy,
        SubjectIdentity::from_digest([37; 32]).unwrap(),
        trusted.request_attributes(),
    );
    let changed_attributes = SecurityContext::new(
        policy,
        trusted.subject(),
        RequestAttributesIdentity::from_digest([38; 32]).unwrap(),
    );
    for identity in [changed_subject, changed_attributes] {
        let miss = snapshot
            .compile_secured(source, QUERY, &budget(Some(identity)), policy)
            .unwrap();
        assert!(!std::ptr::eq(first.plan(), miss.plan()));
    }
}

#[test]
fn source_claims_partition_cache_identity_and_cannot_change_after_handoff() {
    let claims = |tenant: &str| {
        crate::PostgresRlsClaims::new(std::collections::BTreeMap::from([(
            "app.tenant_id".into(),
            tenant.into(),
        )]))
        .unwrap()
    };
    let profile = |tenant: &str| {
        QueryAdmission::Bearer(
            BearerQueryAdmission::for_service_principal(TOKEN)
                .unwrap()
                .with_postgres_rls(claims(tenant))
                .unwrap(),
        )
    };
    let a = profile("a");
    let b = profile("b");
    let ca = context(&a, TOKEN);
    let cb = context(&b, TOKEN);
    assert_eq!(ca.subject(), cb.subject());
    assert_ne!(ca.request_attributes(), cb.request_attributes());
    assert_ne!(ca.policy_snapshot(), cb.policy_snapshot());
    let mut request = budget(Some(ca));
    assert!(
        a.validate(&request).is_err(),
        "missing claims must reject even matching digest"
    );
    request.retain_postgres_rls(a.postgres_rls()).unwrap();
    assert!(a.validate(&request).is_ok());
    assert!(b.validate(&request).is_err());
    let clone = request.clone();
    assert!(request.retain_postgres_rls(b.postgres_rls()).is_err());
    assert!(a.validate(&clone).is_ok());
}

#[test]
fn postgres_and_portable_row_policy_families_are_mutually_exclusive() {
    let claims = crate::PostgresRlsClaims::new(std::collections::BTreeMap::from([(
        "app.tenant_id".into(),
        "a".into(),
    )]))
    .unwrap();
    let rows = || {
        crate::PortableRowPolicy::new(vec![crate::PortableRowRule::new(
            0, "people", "tenant", "a",
        )
        .unwrap()])
        .unwrap()
    };
    assert!(BearerQueryAdmission::for_service_principal(TOKEN)
        .unwrap()
        .with_postgres_rls(claims.clone())
        .unwrap()
        .with_portable_rows(rows())
        .is_err());
    assert!(BearerQueryAdmission::for_service_principal(TOKEN)
        .unwrap()
        .with_portable_rows(rows())
        .unwrap()
        .with_postgres_rls(claims)
        .is_err());
}
