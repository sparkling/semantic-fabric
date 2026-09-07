use super::*;
use crate::{PostgresRlsClaims, ProvisionedBearerAdmission, ProvisionedBearerSubject};
use std::collections::BTreeMap;
use std::time::Duration;

const ALICE: &str = "test-only-alice-credential-0123456789";
const BOB: &str = "test-only-bob-credential-9876543210";

fn subject(id: &str, token: &str, tenant: &str) -> ProvisionedBearerSubject {
    ProvisionedBearerSubject::postgres_rls(
        id,
        token,
        PostgresRlsClaims::new(BTreeMap::from([("app.tenant_id".into(), tenant.into())])).unwrap(),
    )
    .unwrap()
}

fn profile(reverse: bool) -> QueryAdmission {
    let mut subjects = vec![subject("a", ALICE, "a"), subject("b", BOB, "b")];
    if reverse {
        subjects.reverse();
    }
    QueryAdmission::ProvisionedBearers(ProvisionedBearerAdmission::new(subjects).unwrap())
}

fn headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        format!("Bearer {token}").parse().unwrap(),
    );
    // These are untrusted and may neither select a subject nor overwrite claims.
    headers.insert("x-subject-id", "b".parse().unwrap());
    headers.insert("x-tenant-id", "b".parse().unwrap());
    headers
}

fn admitted(profile: &QueryAdmission, token: &str) -> RequestBudget {
    let mut budget = RequestBudget::after(Duration::from_secs(10), crate::DEFAULT_QUERY_LIMITS);
    budget
        .retain_authenticated(profile.admit(&headers(token)).unwrap())
        .unwrap();
    budget
}

#[test]
fn same_policy_selects_distinct_atomic_subject_and_claim_bundles() {
    let policy = profile(false);
    let mut a = admitted(&policy, ALICE);
    let b = admitted(&policy, BOB);
    let ca = a.security_context().unwrap();
    let cb = b.security_context().unwrap();
    assert_eq!(ca.policy_snapshot(), cb.policy_snapshot());
    assert_ne!(ca.subject(), cb.subject());
    assert_ne!(ca.request_attributes(), cb.request_attributes());
    assert_eq!(a.postgres_rls().unwrap().0["app.tenant_id"], "a");
    assert_eq!(b.postgres_rls().unwrap().0["app.tenant_id"], "b");
    assert!(policy.validate(&a).is_ok());
    assert!(policy.validate(&b).is_ok());
    let clone = a.clone();
    assert!(a
        .retain_authenticated(policy.admit(&headers(BOB)).unwrap())
        .is_err());
    assert_eq!(clone.security_context(), Some(ca));
    assert_eq!(clone.postgres_rls(), a.postgres_rls());

    let mut mismatched = RequestBudget::after(Duration::from_secs(10), crate::DEFAULT_QUERY_LIMITS);
    mismatched.retain_security(ca).unwrap();
    mismatched
        .retain_postgres_rls(b.postgres_rls().cloned())
        .unwrap();
    assert_eq!(policy.validate(&mismatched), Err(ProblemCode::AccessDenied));
}

#[test]
fn registry_order_is_canonical_and_rotating_credentials_preserves_subject_identity() {
    let policy = profile(false);
    let reordered = profile(true);
    assert_eq!(policy.policy(), reordered.policy());
    assert_eq!(
        admitted(&policy, ALICE).security_context(),
        admitted(&reordered, ALICE).security_context()
    );
    let rotated = QueryAdmission::ProvisionedBearers(
        ProvisionedBearerAdmission::new(vec![subject("a", BOB, "a")]).unwrap(),
    );
    let old = admitted(&policy, ALICE).security_context().unwrap();
    let new = admitted(&rotated, BOB).security_context().unwrap();
    assert_eq!(old.subject(), new.subject());
    assert_eq!(old.request_attributes(), new.request_attributes());
    assert_ne!(old.policy_snapshot(), new.policy_snapshot());
    assert!(rotated.admit(&headers(ALICE)).is_err());
    assert_eq!(
        rotated.validate(&admitted(&policy, ALICE)),
        Err(ProblemCode::AccessDenied)
    );
}

#[test]
fn duplicate_subjects_credentials_empty_and_oversized_registries_reject_redacted() {
    for subjects in [
        vec![],
        vec![subject("a", ALICE, "a"), subject("a", BOB, "b")],
        vec![subject("a", ALICE, "a"), subject("b", ALICE, "b")],
        (0..257)
            .map(|i| {
                subject(
                    &i.to_string(),
                    &format!("test-only-unique-credential-{i:032}"),
                    "a",
                )
            })
            .collect(),
    ] {
        let error = ProvisionedBearerAdmission::new(subjects).unwrap_err();
        let output = format!("{error:?}");
        assert!(!output.contains(ALICE) && !output.contains(BOB));
    }
    assert!(!format!("{:?}", profile(false)).contains(ALICE));
    let policy = profile(false);
    assert!(policy.admit(&HeaderMap::new()).is_err());
    let mut duplicate = headers(ALICE);
    duplicate.append(
        header::AUTHORIZATION,
        format!("Bearer {BOB}").parse().unwrap(),
    );
    assert!(policy.admit(&duplicate).is_err());
    assert!(policy
        .admit(&headers("unknown-credential-01234567890123456789"))
        .is_err());
}

#[test]
fn one_runtime_cache_and_execution_handoff_separate_registered_subjects() {
    let mut cfg = super::tests::config();
    cfg.set_query_admission(profile(false));
    let snapshot = cfg.runtime_lease().unwrap();
    let a = admitted(&cfg.query_admission, ALICE);
    let b = admitted(&cfg.query_admission, BOB);
    let source = sf_core::SourceId::new(0).unwrap();
    let policy = cfg.query_admission.policy().unwrap();
    let query = "SELECT ?name WHERE { ?s <http://ex/name> ?name }";
    let first = snapshot.compile_secured(source, query, &a, policy).unwrap();
    let hit = snapshot.compile_secured(source, query, &a, policy).unwrap();
    let other = snapshot.compile_secured(source, query, &b, policy).unwrap();
    assert!(std::ptr::eq(first.plan(), hit.plan()));
    assert!(!std::ptr::eq(first.plan(), other.plan()));
    assert!(snapshot.prepare_request_execution(first, &b).is_err());
    assert!(snapshot.prepare_request_execution(other, &a).is_err());
    assert!(snapshot.prepare_request_execution(hit, &a).is_ok());
}
