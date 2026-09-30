use super::deferred_test_controls::{context, open, policy, request, stop, who, Trip};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError as Stop};
use sf_core::security_context::SecurityContext;
use sf_sparql::cache::generated::{GeneratedQueryRefusal, ShapeRule};
use sf_sparql::{Error, Plan, PlanForm};

use super::tests::{
    assert_answers, bind, bind_at, bind_default, mapping_text, ASK, EXTRA, FREE, SELECT, UNMAPPED,
};
use super::GeneratedCompiled;
use super::GeneratedDeferredOutcome as Out;
use super::GeneratedRuntimeError as E;
use crate::binding::{BindingMismatch, RuntimeBinding};
use crate::budget::RequestBudget;
use crate::semantic_admission::MappingOrigin;

const CONSTRUCT: &str = "CONSTRUCT { ?s <http://ex/name> ?n } WHERE { ?s <http://ex/name> ?n }";
const DATASET: &str = "SELECT ?n FROM <http://ex/g> WHERE { ?p <http://ex/name> ?n }";
const DESCRIBE_ALL: &str = "DESCRIBE * WHERE { ?s ?p ?o }";
const UPDATE: &str = "INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }";

#[derive(Clone, Copy)]
enum Route {
    Cold,
    Warm,
    Pre,
    Sec,
    SecWarm,
}

const ROUTES: [Route; 5] = [
    Route::Cold,
    Route::Warm,
    Route::Pre,
    Route::Sec,
    Route::SecWarm,
];

fn admitted<T>(outcome: Out<T>) -> T {
    match outcome {
        Out::Admitted(value) => value,
        Out::Refused { .. } => panic!("expected admission"),
    }
}

fn refused<T>(outcome: Out<T>) -> (GeneratedQueryRefusal, Option<Arc<Plan>>) {
    match outcome {
        Out::Refused {
            refusal,
            authorization_plan,
        } => (refusal, authorization_plan),
        Out::Admitted(_) => panic!("expected refusal"),
    }
}

fn stopped<T>(result: &Result<T, E>, cause: Stop) -> bool {
    matches!(result, Err(E::Compiler(Error::QueryControl(found))) if *found == cause)
}

fn secured_plan(binding: &RuntimeBinding, query: &str) -> Arc<Plan> {
    let bound = binding.compile_secured(query, &open(), policy(1));
    bound.unwrap().plan
}

fn secured_deferred(
    binding: &RuntimeBinding,
    query: &str,
    who: SecurityContext,
) -> Result<Out<GeneratedCompiled>, E> {
    let control = request(Some(who), u64::MAX);
    binding.compile_generated_secured_deferred(query, &control, policy(1))
}

fn prepared(route: Route, query: &str) -> RuntimeBinding {
    let binding = bind_default();
    match route {
        Route::Warm => drop(binding.compile(query, FREE)),
        Route::SecWarm => drop(binding.compile_secured(query, &open(), policy(1))),
        _ => {}
    }
    binding
}

fn ordinary(route: Route, binding: &RuntimeBinding, query: &str) -> Arc<Plan> {
    match route {
        Route::Sec | Route::SecWarm => secured_plan(binding, query),
        _ => binding.compile(query, FREE).unwrap().plan,
    }
}

fn run(
    route: Route,
    binding: &RuntimeBinding,
    query: &str,
    control: &RequestBudget,
) -> Result<Out<Arc<Plan>>, E> {
    match route {
        Route::Pre => binding.preflight_generated_deferred(query, control),
        Route::Sec | Route::SecWarm => {
            let compiled = binding.compile_generated_secured_deferred(query, control, policy(1));
            compiled.map(|outcome| outcome.map(|bound| bound.plan.plan))
        }
        _ => {
            let compiled = binding.compile_generated_deferred(query, control);
            compiled.map(|outcome| outcome.map(|bound| bound.plan.plan))
        }
    }
}

fn existing(
    route: Route,
    binding: &RuntimeBinding,
    query: &str,
    control: &RequestBudget,
) -> Result<(), E> {
    match route {
        Route::Pre => binding.preflight_generated(query, control).map(drop),
        Route::Sec | Route::SecWarm => {
            let compiled = binding.compile_generated_secured(query, control, policy(1));
            compiled.map(drop)
        }
        _ => binding.compile_generated(query, control).map(drop),
    }
}

fn attempt(route: Route, limit: u64, deferred: bool) -> (Result<(), E>, RequestBudget) {
    let binding = prepared(route, SELECT);
    let control = request(Some(who()), limit);
    let result = if deferred {
        run(route, &binding, SELECT, &control).map(|outcome| drop(admitted(outcome)))
    } else {
        existing(route, &binding, SELECT, &control)
    };
    (result, control)
}

#[test]
fn generated_deferred_admitted_matches_existing_identity_and_ownership() {
    for query in [SELECT, ASK] {
        let binding = bind_default();
        let pre = admitted(binding.preflight_generated_deferred(query, FREE).unwrap());
        let cold = admitted(binding.compile_generated_deferred(query, FREE).unwrap());
        let warm = admitted(binding.compile_generated_deferred(query, FREE).unwrap());
        let old = binding.compile_generated(query, FREE).unwrap();
        let raw = binding.compile(query, FREE).unwrap();
        assert!(!Arc::ptr_eq(&pre, &cold.plan.plan));
        assert!(Arc::ptr_eq(&cold.plan.plan, &warm.plan.plan));
        assert!(Arc::ptr_eq(&cold.plan.plan, &old.plan.plan));
        assert!(Arc::ptr_eq(&cold.plan.plan, &raw.plan));
        assert_eq!(cold.identity, old.identity);
        assert_eq!(warm.identity, old.identity);
        assert!(cold.plan.security.is_none());
        assert_eq!(matches!(cold.plan.plan().form, PlanForm::Ask), query == ASK);
        assert_answers(&binding, query, cold.plan);
        assert_answers(&binding, query, warm.plan);
    }
}

#[test]
fn generated_deferred_preflight_never_populates_the_cache() {
    let binding = bind_default();
    let first = admitted(binding.preflight_generated_deferred(SELECT, FREE).unwrap());
    let second = admitted(binding.preflight_generated_deferred(SELECT, FREE).unwrap());
    assert!(!Arc::ptr_eq(&first, &second));
    let cached = binding.compile(SELECT, FREE).unwrap();
    assert!(!Arc::ptr_eq(&first, &cached.plan));
    assert!(!Arc::ptr_eq(&second, &cached.plan));
    let again = binding.compile(SELECT, FREE).unwrap();
    assert!(Arc::ptr_eq(&cached.plan, &again.plan));
    let outcome = binding
        .preflight_generated_deferred(UNMAPPED, FREE)
        .unwrap();
    let (_, plan) = refused(outcome);
    let later = binding.compile(UNMAPPED, FREE).unwrap();
    assert!(!Arc::ptr_eq(&plan.unwrap(), &later.plan));
}

#[test]
fn generated_deferred_plans_stay_bound_to_their_own_binding() {
    let first = bind_default();
    let second = bind_default();
    assert_eq!(first.scope(), second.scope());
    let cached = admitted(first.compile_generated_deferred(SELECT, FREE).unwrap());
    let foreign = second.prepare_execution(cached.plan);
    assert!(matches!(foreign, Err(BindingMismatch)));
    let secured = admitted(secured_deferred(&first, ASK, who()).unwrap());
    let foreign = second.prepare_execution(secured.plan);
    assert!(matches!(foreign, Err(BindingMismatch)));
    let again = admitted(first.compile_generated_deferred(SELECT, FREE).unwrap());
    assert!(first.prepare_execution(again.plan).is_ok());
}

#[test]
fn generated_deferred_secured_admission_preserves_context_and_binding() {
    for query in [SELECT, ASK] {
        let binding = bind_default();
        let alice = who();
        let cold = admitted(secured_deferred(&binding, query, alice).unwrap());
        let warm = admitted(secured_deferred(&binding, query, alice).unwrap());
        assert!(Arc::ptr_eq(&cold.plan.plan, &warm.plan.plan));
        assert!(cold.plan.security == Some(alice));
        assert!(warm.plan.security == Some(alice));
        assert_eq!(cold.plan.source_id(), binding.source_id());
        assert_eq!(cold.plan.scope, binding.scope());
        let old = binding.compile_generated_secured(query, &open(), policy(1));
        let old = old.unwrap();
        assert!(Arc::ptr_eq(&cold.plan.plan, &old.plan.plan));
        assert_eq!(cold.identity, old.identity);
        assert_answers(&binding, query, cold.plan);
        assert_answers(&binding, query, warm.plan);
    }
}

#[test]
fn generated_deferred_secured_subjects_and_attributes_stay_isolated() {
    let binding = bind_default();
    let alice = who();
    let a1 = admitted(secured_deferred(&binding, SELECT, alice).unwrap());
    let a2 = admitted(secured_deferred(&binding, SELECT, alice).unwrap());
    assert!(Arc::ptr_eq(&a1.plan.plan, &a2.plan.plan));
    for different in [context(4, 3), context(2, 5)] {
        let miss = admitted(secured_deferred(&binding, SELECT, different).unwrap());
        assert!(!Arc::ptr_eq(&a1.plan.plan, &miss.plan.plan));
        assert!(miss.plan.security == Some(different));
    }
    let unscoped = binding.compile(SELECT, FREE).unwrap();
    assert!(!Arc::ptr_eq(&a1.plan.plan, &unscoped.plan));
}

#[test]
fn generated_deferred_secured_refusal_reruns_coverage_and_never_enters_the_cache() {
    let binding = bind_default();
    let warmed = secured_plan(&binding, UNMAPPED);
    for _ in 0..2 {
        let outcome = secured_deferred(&binding, UNMAPPED, who()).unwrap();
        let (refusal, plan) = refused(outcome);
        assert_eq!(refusal, GeneratedQueryRefusal::CoverageRefused);
        assert!(!Arc::ptr_eq(&plan.unwrap(), &warmed));
    }
    assert!(Arc::ptr_eq(&secured_plan(&binding, UNMAPPED), &warmed));
    let fresh = bind_default();
    let (_, plan) = refused(secured_deferred(&fresh, UNMAPPED, who()).unwrap());
    assert!(!Arc::ptr_eq(
        &plan.unwrap(),
        &secured_plan(&fresh, UNMAPPED)
    ));
}

#[test]
fn generated_deferred_structural_refusals_carry_a_diagnostic_plan_never_admission() {
    let cases = [
        (CONSTRUCT, ShapeRule::ConstructForm, true),
        (DATASET, ShapeRule::DatasetClause, false),
    ];
    for (query, rule, construct) in cases {
        for route in ROUTES {
            let binding = prepared(route, query);
            let outcome = run(route, &binding, query, &open()).unwrap();
            let (refusal, plan) = refused(outcome);
            assert_eq!(refusal, GeneratedQueryRefusal::Rule(rule), "{query}");
            let plan = plan.expect("lowerable refusal keeps a diagnostic plan");
            let is_construct = matches!(plan.form, PlanForm::Construct { .. });
            assert_eq!(is_construct, construct, "{query}");
            assert!(!Arc::ptr_eq(&plan, &ordinary(route, &binding, query)));
        }
    }
}

#[test]
fn generated_deferred_coverage_refusal_returns_a_diagnostic_plan_on_every_route() {
    for route in ROUTES {
        let binding = prepared(route, UNMAPPED);
        let outcome = run(route, &binding, UNMAPPED, &open()).unwrap();
        let (refusal, plan) = refused(outcome);
        assert_eq!(refusal, GeneratedQueryRefusal::CoverageRefused);
        let plan = plan.expect("lowerable refusal keeps a diagnostic plan");
        assert!(matches!(plan.form, PlanForm::Ask));
        assert!(!Arc::ptr_eq(&plan, &ordinary(route, &binding, UNMAPPED)));
        assert!(binding.compile(UNMAPPED, FREE).is_ok());
    }
}

#[test]
fn generated_deferred_syntax_refusals_have_no_plan_on_every_route() {
    let form = GeneratedQueryRefusal::Rule(ShapeRule::FormNotAdmitted);
    for query in [UPDATE, "CLEAR ALL", "not sparql", ""] {
        for route in ROUTES {
            let binding = prepared(route, SELECT);
            let outcome = run(route, &binding, query, &open()).unwrap();
            let (refusal, plan) = refused(outcome);
            assert_eq!(refusal, form, "{query:?}");
            assert!(plan.is_none(), "{query:?}");
        }
    }
}

#[test]
fn generated_deferred_unsupported_lowering_refuses_without_a_plan() {
    let describe = GeneratedQueryRefusal::Rule(ShapeRule::DescribeForm);
    for route in ROUTES {
        let binding = prepared(route, SELECT);
        let ordinary = binding.compile(DESCRIBE_ALL, FREE);
        assert!(matches!(ordinary, Err(Error::Unsupported(_))));
        let outcome = run(route, &binding, DESCRIBE_ALL, &open()).unwrap();
        let (refusal, plan) = refused(outcome);
        assert_eq!(refusal, describe);
        assert!(plan.is_none());
    }
}

#[test]
fn generated_deferred_denied_controls_are_exact_terminal_errors() {
    for cause in [Stop::Cancelled, Stop::DeadlineExceeded] {
        for query in [SELECT, UNMAPPED, CONSTRUCT, UPDATE] {
            for route in ROUTES {
                let binding = prepared(route, query);
                let control = open();
                stop(&control, cause);
                let result = run(route, &binding, query, &control);
                assert!(stopped(&result, cause), "{query}");
                assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
            }
        }
    }
}

#[test]
fn generated_deferred_admission_work_matches_existing_and_fails_one_short() {
    for route in ROUTES {
        let (probe, control) = attempt(route, u64::MAX, false);
        probe.unwrap();
        let total = control.consumed(QueryCharge::CompilerWork);
        assert!(total > 0);
        let (probe, control) = attempt(route, u64::MAX, true);
        probe.unwrap();
        assert_eq!(control.consumed(QueryCharge::CompilerWork), total);
        let (exact, control) = attempt(route, total, true);
        exact.unwrap();
        assert_eq!(control.consumed(QueryCharge::CompilerWork), total);
        let (short, control) = attempt(route, total - 1, true);
        assert!(stopped(&short, Stop::CompilerWorkExceeded));
        assert_eq!(control.checkpoint(), Err(Stop::CompilerWorkExceeded));
    }
}

#[test]
fn generated_deferred_lowering_control_failure_is_an_error_not_a_refusal() {
    for route in ROUTES {
        let probe = open();
        let old = existing(route, &prepared(route, UNMAPPED), UNMAPPED, &probe);
        assert!(matches!(
            old,
            Err(E::Refused(GeneratedQueryRefusal::CoverageRefused))
        ));
        let screen = probe.consumed(QueryCharge::CompilerWork);
        assert!(screen > 0);
        let control = request(Some(who()), screen);
        let binding = prepared(route, UNMAPPED);
        let result = run(route, &binding, UNMAPPED, &control);
        assert!(stopped(&result, Stop::CompilerWorkExceeded));
    }
}

#[test]
fn generated_deferred_sticky_controls_at_every_charge_never_become_refusals() {
    let binding = bind_default();
    for query in [SELECT, UNMAPPED] {
        let probe = Trip::new(u64::MAX, Stop::Cancelled);
        binding.preflight_generated_deferred(query, &probe).unwrap();
        let total = probe.calls.load(Ordering::SeqCst);
        assert!(total > 1);
        for reason in [Stop::Cancelled, Stop::DeadlineExceeded] {
            for at in 1..=total {
                let trip = Trip::new(at, reason);
                let result = binding.preflight_generated_deferred(query, &trip);
                assert!(stopped(&result, reason), "{query} {reason:?} at {at}");
                assert_eq!(trip.checkpoint(), Err(reason));
            }
        }
    }
}

#[test]
fn generated_deferred_foreign_receipts_are_rejected_on_every_route() {
    let base = mapping_text(true, false);
    let others = [
        bind(&mapping_text(false, false), MappingOrigin::Authored, &[]),
        bind(&base, MappingOrigin::Authored, &[EXTRA]),
        bind(&base, MappingOrigin::Direct, &[]),
        bind_at(1, &base, MappingOrigin::Authored, &[]),
    ];
    for other in others {
        let mut binding = bind_default();
        binding.generated = other.generated;
        let control = open();
        let compiled = binding.compile_generated_deferred(ASK, &control);
        assert!(matches!(compiled, Err(E::ReceiptMismatch(_))));
        let preflight = binding.preflight_generated_deferred(ASK, &control);
        assert!(matches!(preflight, Err(E::ReceiptMismatch(_))));
        let secured = binding.compile_generated_secured_deferred(ASK, &control, policy(1));
        assert!(matches!(secured, Err(E::ReceiptMismatch(_))));
        assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
    }
}

#[test]
fn generated_deferred_secured_context_and_policy_precede_all_other_work() {
    let mut binding = bind_default();
    let foreign = bind(&mapping_text(false, false), MappingOrigin::Authored, &[]);
    binding.generated = foreign.generated;
    let cancelled = open();
    stop(&cancelled, Stop::Cancelled);
    let none = request(None, u64::MAX);
    stop(&none, Stop::Cancelled);
    for query in ["not sparql", UNMAPPED, SELECT] {
        let missing = binding.compile_generated_secured_deferred(query, &none, policy(1));
        assert!(matches!(missing, Err(E::MissingSecurityContext)));
        let wrong = binding.compile_generated_secured_deferred(query, &cancelled, policy(4));
        assert!(matches!(wrong, Err(E::PolicyMismatch)));
        let held = binding.compile_generated_secured_deferred(query, &cancelled, policy(1));
        assert!(stopped(&held, Stop::Cancelled));
        let receipt = binding.compile_generated_secured_deferred(query, &open(), policy(1));
        assert!(matches!(receipt, Err(E::ReceiptMismatch(_))));
    }
    assert_eq!(cancelled.consumed(QueryCharge::CompilerWork), 0);
    let sane = bind_default();
    for query in ["not sparql", UNMAPPED] {
        let control = open();
        let wrong = sane.compile_generated_secured_deferred(query, &control, policy(4));
        assert!(matches!(wrong, Err(E::PolicyMismatch)));
        assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
    }
}

#[test]
fn generated_deferred_debug_output_is_redacted() {
    for route in ROUTES {
        for query in [UNMAPPED, CONSTRUCT, UPDATE, SELECT] {
            let binding = prepared(route, query);
            let outcome = run(route, &binding, query, &open()).unwrap();
            let text = format!("{outcome:?}");
            for leaked in ["http://ex", "unknown", "branches"] {
                assert!(!text.contains(leaked), "{text}");
            }
        }
    }
}

#[test]
fn generated_deferred_leaves_existing_generated_api_unchanged() {
    let binding = bind_default();
    let before = binding.compile_generated(SELECT, FREE).unwrap();
    for query in [UNMAPPED, CONSTRUCT, UPDATE] {
        refused(binding.compile_generated_deferred(query, FREE).unwrap());
        refused(binding.preflight_generated_deferred(query, FREE).unwrap());
    }
    let unmapped = binding.compile_generated(UNMAPPED, FREE);
    assert!(matches!(
        unmapped,
        Err(E::Refused(GeneratedQueryRefusal::CoverageRefused))
    ));
    let construct = binding.preflight_generated(CONSTRUCT, FREE);
    assert!(matches!(
        construct,
        Err(E::Refused(GeneratedQueryRefusal::Rule(
            ShapeRule::ConstructForm
        )))
    ));
    let after = binding.compile_generated(SELECT, FREE).unwrap();
    assert!(Arc::ptr_eq(&before.plan.plan, &after.plan.plan));
    assert_eq!(before.identity, after.identity);
    assert!(binding.compile(UNMAPPED, FREE).is_ok());
}
