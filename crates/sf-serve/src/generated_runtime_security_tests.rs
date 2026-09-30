use std::sync::Arc;
use std::time::Duration;

use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError as Stop, QueryLimits};
use sf_core::security_context::{
    PolicySnapshotId, RequestAttributesIdentity, SecurityContext, SubjectIdentity,
};
use sf_sparql::cache::generated::{GeneratedQueryRefusal, ShapeRule};
use sf_sparql::Error;

use super::tests::{
    assert_answers, bind, bind_at, bind_default, mapping_text, ASK, EXTRA, FREE, SELECT, UNMAPPED,
};
use super::GeneratedRuntimeError as E;
use crate::budget::RequestBudget;
use crate::generated_profile_identity::GeneratedProfileClaim;
use crate::semantic_admission::MappingOrigin;

fn policy(value: u8) -> PolicySnapshotId {
    PolicySnapshotId::from_digest([value; 32]).unwrap()
}

fn context(subject: u8, attributes: u8) -> SecurityContext {
    SecurityContext::new(
        policy(1),
        SubjectIdentity::from_digest([subject; 32]).unwrap(),
        RequestAttributesIdentity::from_digest([attributes; 32]).unwrap(),
    )
}

fn request(context: Option<SecurityContext>, work: u64) -> RequestBudget {
    let limits = QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX);
    let mut budget = RequestBudget::after(Duration::from_secs(30), limits);
    if let Some(context) = context {
        budget.retain_security(context).unwrap();
    }
    budget
}

fn secured(
    binding: &crate::binding::RuntimeBinding,
    query: &str,
    who: SecurityContext,
) -> Result<super::GeneratedCompiled, E> {
    binding.compile_generated_secured(query, &request(Some(who), u64::MAX), policy(1))
}

fn form_refused<T>(result: &Result<T, E>) -> bool {
    matches!(
        result,
        Err(E::Refused(GeneratedQueryRefusal::Rule(
            ShapeRule::FormNotAdmitted
        )))
    )
}

#[test]
fn policy_is_verified_before_control_parse_screen_and_coverage() {
    let binding = bind_default();
    let who = context(2, 3);
    let cancelled = request(Some(who), u64::MAX);
    QueryControl::terminate(&cancelled, Stop::Cancelled);
    let result = binding.compile_generated_secured("not sparql", &cancelled, policy(4));
    assert!(matches!(result, Err(E::PolicyMismatch)));
    let fresh = request(Some(who), u64::MAX);
    let result = binding.compile_generated_secured(UNMAPPED, &fresh, policy(4));
    assert!(matches!(result, Err(E::PolicyMismatch)));
    assert_eq!(fresh.consumed(QueryCharge::CompilerWork), 0);

    let result = binding.compile_generated_secured("not sparql", &cancelled, policy(1));
    assert!(matches!(
        result,
        Err(E::Compiler(Error::QueryControl(Stop::Cancelled)))
    ));
    let fresh = request(Some(who), u64::MAX);
    let result = binding.compile_generated_secured("not sparql", &fresh, policy(1));
    assert!(form_refused(&result));
    let result =
        binding.compile_generated_secured(UNMAPPED, &request(Some(who), u64::MAX), policy(1));
    assert!(matches!(
        result,
        Err(E::Refused(GeneratedQueryRefusal::CoverageRefused))
    ));
}

#[test]
fn update_and_malformed_forms_are_refused_on_every_route_without_identity() {
    let forms = [
        "INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }",
        "CLEAR ALL",
        "SELECT WHERE {",
        "",
    ];
    let binding = bind_default();
    let who = context(2, 3);
    for warm in [false, true] {
        if warm {
            binding.compile_generated(SELECT, FREE).unwrap();
            secured(&binding, SELECT, who).unwrap();
        }
        for query in forms {
            let compiled = binding.compile_generated(query, FREE);
            assert!(form_refused(&compiled), "{query:?}");
            let preflight = binding.preflight_generated(query, FREE);
            assert!(form_refused(&preflight), "{query:?}");
            let result = secured(&binding, query, who);
            assert!(form_refused(&result), "{query:?}");
            let error = compiled.err().unwrap();
            let text = format!("{error} {error:?}");
            assert!(!text.contains("http://ex"), "{text}");
        }
    }
    let cancelled = request(Some(who), u64::MAX);
    QueryControl::terminate(&cancelled, Stop::Cancelled);
    let stopped = |result: Result<super::GeneratedCompiled, E>| {
        matches!(
            result,
            Err(E::Compiler(Error::QueryControl(Stop::Cancelled)))
        )
    };
    assert!(stopped(binding.compile_generated(forms[0], &cancelled)));
    assert!(stopped(binding.compile_generated_secured(
        forms[0],
        &cancelled,
        policy(1)
    )));
}

#[test]
fn a_missing_security_context_is_refused() {
    let binding = bind_default();
    let result = binding.compile_generated_secured(SELECT, &request(None, u64::MAX), policy(1));
    assert!(matches!(result, Err(E::MissingSecurityContext)));
}

#[test]
fn secured_plan_carries_the_request_context_and_runtime_binding_fields() {
    let binding = bind_default();
    let who = context(2, 3);
    let compiled = secured(&binding, SELECT, who).unwrap();
    assert!(compiled.plan.security == Some(who));
    assert_eq!(compiled.plan.source_id(), binding.source_id());
    assert_eq!(compiled.plan.scope, binding.scope());
    let ordinary = binding.compile(SELECT, FREE).unwrap();
    assert!(ordinary.security.is_none());
    assert!(binding.prepare_execution(compiled.plan).is_ok());
}

#[test]
fn secured_generated_plans_execute_cold_and_warm_on_their_own_binding() {
    let who = context(2, 3);
    for query in [SELECT, ASK] {
        let binding = bind_default();
        let cold = secured(&binding, query, who).unwrap();
        let warm = secured(&binding, query, who).unwrap();
        assert!(Arc::ptr_eq(&cold.plan.plan, &warm.plan.plan));
        assert_eq!(cold.identity, warm.identity);
        assert!(cold.plan.security == Some(who));
        assert!(warm.plan.security == Some(who));
        assert_eq!(cold.plan.scope, binding.scope());
        assert_answers(&binding, query, cold.plan);
        assert_answers(&binding, query, warm.plan);
        let other = bind_default();
        assert_eq!(other.scope(), binding.scope());
        let foreign = secured(&binding, query, who).unwrap().plan;
        assert!(other.prepare_execution(foreign).is_err());
    }
}

#[test]
fn subjects_and_attributes_stay_isolated_and_hits_rerun_coverage() {
    let binding = bind_default();
    let (alice, bob, other) = (context(2, 3), context(4, 3), context(2, 5));
    let a1 = secured(&binding, SELECT, alice).unwrap();
    let a2 = secured(&binding, SELECT, alice).unwrap();
    assert!(Arc::ptr_eq(&a1.plan.plan, &a2.plan.plan));
    for different in [bob, other] {
        let miss = secured(&binding, SELECT, different).unwrap();
        assert!(!Arc::ptr_eq(&a1.plan.plan, &miss.plan.plan));
    }
    let raw = binding
        .compile_secured(SELECT, &request(Some(alice), u64::MAX), policy(1))
        .unwrap();
    assert!(Arc::ptr_eq(&a1.plan.plan, &raw.plan));
    let unscoped = binding.compile(SELECT, FREE).unwrap();
    assert!(!Arc::ptr_eq(&a1.plan.plan, &unscoped.plan));

    for subject in [alice, bob] {
        let warmed =
            binding.compile_secured(UNMAPPED, &request(Some(subject), u64::MAX), policy(1));
        assert!(warmed.is_ok());
        for _ in 0..2 {
            let result = secured(&binding, UNMAPPED, subject);
            assert!(matches!(
                result,
                Err(E::Refused(GeneratedQueryRefusal::CoverageRefused))
            ));
        }
    }
}

#[test]
fn secured_admission_work_is_exact_and_fails_one_short() {
    let who = context(2, 3);
    for warm in [false, true] {
        let attempt = |limit: u64| {
            let binding = bind_default();
            if warm {
                let plan =
                    binding.compile_secured(SELECT, &request(Some(who), u64::MAX), policy(1));
                plan.unwrap();
            }
            let control = request(Some(who), limit);
            let result = binding.compile_generated_secured(SELECT, &control, policy(1));
            (result, control)
        };
        let (probe, control) = attempt(u64::MAX);
        probe.unwrap();
        let total = control.consumed(QueryCharge::CompilerWork);
        assert!(total > 0);
        let (exact, control) = attempt(total);
        exact.unwrap();
        assert_eq!(control.consumed(QueryCharge::CompilerWork), total);
        let (short, control) = attempt(total - 1);
        assert!(matches!(
            short,
            Err(E::Compiler(Error::QueryControl(Stop::CompilerWorkExceeded)))
        ));
        assert_eq!(control.checkpoint(), Err(Stop::CompilerWorkExceeded));
    }
}

fn identity_of(
    binding: &crate::binding::RuntimeBinding,
) -> crate::generated_profile_identity::GeneratedProfileIdentity {
    binding.compile_generated(ASK, FREE).unwrap().identity
}

#[test]
fn identity_is_stable_across_equivalent_bindings_and_paths() {
    let first = bind_default();
    let second = bind_default();
    let identity = identity_of(&first);
    assert_eq!(identity, identity_of(&second));
    assert_eq!(identity, identity_of(&first));
    assert_eq!(identity.wire(), identity_of(&second).wire());
    let secure = secured(&first, ASK, context(2, 3)).unwrap();
    assert_eq!(secure.identity, identity);
    let claim = GeneratedProfileClaim::parse(&identity.wire()).unwrap();
    assert!(claim.matches(&identity));
}

#[test]
fn identity_changes_with_ontology_mapping_origin_and_source() {
    let base = identity_of(&bind_default());
    let text = mapping_text(true, false);
    let variants = [
        identity_of(&bind(&text, MappingOrigin::Authored, &[EXTRA])),
        identity_of(&bind(
            &mapping_text(false, false),
            MappingOrigin::Authored,
            &[],
        )),
        identity_of(&bind(&text, MappingOrigin::Direct, &[])),
        identity_of(&bind_at(1, &text, MappingOrigin::Authored, &[])),
    ];
    for (index, variant) in variants.iter().enumerate() {
        assert_ne!(*variant, base, "variant {index}");
    }
}

#[test]
fn no_identity_is_issued_on_refusal_wrong_policy_or_control_failure() {
    let binding = bind_default();
    assert!(binding.compile_generated(UNMAPPED, FREE).is_err());
    let wrong = binding.compile_generated_secured(
        SELECT,
        &request(Some(context(2, 3)), u64::MAX),
        policy(4),
    );
    assert!(wrong.is_err());
    let control = request(Some(context(2, 3)), u64::MAX);
    QueryControl::terminate(&control, Stop::DeadlineExceeded);
    assert!(binding
        .compile_generated_secured(SELECT, &control, policy(1))
        .is_err());
    assert!(binding.compile(SELECT, FREE).is_ok());
}

#[test]
fn dataset_receipt_derivation_preserves_legacy_binding_and_admission() {
    use crate::generated_profile_identity::PinnedGraphAllowlist;
    let binding = bind_default();
    let before = identity_of(&binding);
    let allowlist = PinnedGraphAllowlist::new(["http://ex/g"]).unwrap();
    let receipt = binding
        .generated
        .for_single_default_dataset(binding.compiler(), &allowlist)
        .unwrap();
    assert!(receipt.coverage(binding.compiler()).is_ok());
    assert_ne!(receipt.identity(), before);
    assert_eq!(binding.generated.identity(), before);
    assert_eq!(identity_of(&binding), before);
    assert_eq!(
        secured(&binding, ASK, context(2, 3)).unwrap().identity,
        before
    );
    assert_answers(
        &binding,
        SELECT,
        binding.compile_generated(SELECT, FREE).unwrap().plan,
    );
    let equivalent = bind_default();
    assert_eq!(
        receipt.identity(),
        equivalent
            .generated
            .for_single_default_dataset(equivalent.compiler(), &allowlist)
            .unwrap()
            .identity()
    );
    let empty = PinnedGraphAllowlist::new(std::iter::empty::<&str>()).unwrap();
    let empty_receipt = binding
        .generated
        .for_single_default_dataset(binding.compiler(), &empty)
        .unwrap();
    assert_ne!(empty_receipt.identity(), receipt.identity());
    assert_ne!(empty_receipt.identity(), before);
    assert!(binding
        .compile_generated("ASK FROM <http://ex/g> { ?s ?p ?o }", FREE)
        .is_err());
    let debug = format!("{receipt:?} {allowlist:?}");
    assert!(!debug.contains("http://ex/g"));
    assert!(!debug.contains(&receipt.identity().wire()[6..]));
}

#[test]
fn dataset_receipt_derivation_rejects_each_foreign_sealed_dimension() {
    use crate::generated_profile_identity::PinnedGraphAllowlist;
    let base = mapping_text(true, false);
    let binding = bind_default();
    let allowlist = PinnedGraphAllowlist::new(["http://ex/g"]).unwrap();
    let others = [
        bind(&mapping_text(false, false), MappingOrigin::Authored, &[]),
        bind(&base, MappingOrigin::Authored, &[EXTRA]),
        bind(&base, MappingOrigin::Direct, &[]),
        bind_at(1, &base, MappingOrigin::Authored, &[]),
    ];
    for other in others {
        assert!(binding
            .generated
            .for_single_default_dataset(other.compiler(), &allowlist)
            .is_err());
        assert!(other
            .generated
            .for_single_default_dataset(binding.compiler(), &allowlist)
            .is_err());
    }
}
