use std::cell::Cell;
use std::num::NonZeroUsize;
use std::sync::Arc;

use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
use sf_core::security_context::{
    PolicySnapshotId, RequestAttributesIdentity, SecurityContext, SubjectIdentity,
};
use sf_core::{SourceId, SourceMapping};
use sf_sql::Dialect;

use super::{SecurityCompileError, SecurityPlanCache};
use crate::cache::generated::tests::{isolated, parse_spans};
use crate::cache::generated::{
    admit_parsed, ConstantCoverageError, ConstantOccurrence, GeneratedDeferred,
    GeneratedQueryRefusal, ShapeRule,
};
use crate::{CompilerBinding, CompilerSchema, Error, Plan, PlanForm, Tbox};

const VALUES_Q: &str =
    "SELECT ?x WHERE { VALUES ?x { 1 2 3 } FILTER EXISTS { VALUES ?inside { 7 } } }";
const CONSTRUCT_Q: &str = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";
const DATASET_Q: &str = "SELECT * FROM <urn:g> WHERE { ?s ?p ?o }";
const UPDATE_Q: &str = "INSERT DATA { <urn:s> <urn:p> <urn:o> }";
const UNSUPPORTED_Q: &str = "SELECT * WHERE { ?s !<http://ex/p> ?o }";
const EXCEEDED: QueryControlError = QueryControlError::CompilerWorkExceeded;
const CANCELLED: QueryControlError = QueryControlError::Cancelled;
const DEADLINE: QueryControlError = QueryControlError::DeadlineExceeded;
const OK: Result<(), ConstantCoverageError> = Ok(());
const DENY: Result<(), ConstantCoverageError> = Err(ConstantCoverageError::Uncovered);
const FORM: GeneratedQueryRefusal = GeneratedQueryRefusal::Rule(ShapeRule::FormNotAdmitted);

fn digest(value: u8) -> [u8; 32] {
    [value; 32]
}

fn policy(value: u8) -> PolicySnapshotId {
    PolicySnapshotId::from_digest(digest(value)).unwrap()
}

fn context(policy_value: u8, subject: u8, attributes: u8) -> SecurityContext {
    SecurityContext::new(
        policy(policy_value),
        SubjectIdentity::from_digest(digest(subject)).unwrap(),
        RequestAttributesIdentity::from_digest(digest(attributes)).unwrap(),
    )
}

fn binding() -> CompilerBinding {
    CompilerBinding::new(
        SourceMapping::new(SourceId::new(0).unwrap(), Vec::new()),
        Dialect::Sqlite,
        Tbox::default(),
        CompilerSchema::from_unverified_observation(Vec::new()),
        8,
    )
}

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

fn work(control: &QueryBudget) -> u64 {
    control.consumed(QueryCharge::CompilerWork)
}

fn allow(_: ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError> {
    Ok(())
}

fn deny(_: ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError> {
    Err(ConstantCoverageError::Uncovered)
}

fn counting<'a>(
    calls: &'a Cell<u32>,
    verdict: Result<(), ConstantCoverageError>,
) -> impl FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError> + 'a {
    move |_| {
        calls.set(calls.get() + 1);
        verdict
    }
}

fn admitted(deferred: GeneratedDeferred) -> Arc<Plan> {
    match deferred {
        GeneratedDeferred::Admitted(plan) => plan,
        GeneratedDeferred::Refused { .. } => panic!("expected admission"),
    }
}

fn refused(deferred: GeneratedDeferred) -> (GeneratedQueryRefusal, Option<Arc<Plan>>) {
    match deferred {
        GeneratedDeferred::Refused {
            refusal,
            authorization_plan,
        } => (refusal, authorization_plan),
        GeneratedDeferred::Admitted(_) => panic!("expected refusal"),
    }
}

fn control_cause(error: &SecurityCompileError) -> Option<QueryControlError> {
    match error {
        SecurityCompileError::Compiler(Error::QueryControl(cause)) => Some(*cause),
        _ => None,
    }
}

struct Fixture {
    binding: CompilerBinding,
    cache: SecurityPlanCache,
}

impl Fixture {
    fn new() -> Self {
        Self {
            binding: binding(),
            cache: SecurityPlanCache::new(NonZeroUsize::new(8).unwrap()),
        }
    }

    fn run<F>(
        &self,
        expected: u8,
        identity: &SecurityContext,
        query: &str,
        control: &QueryBudget,
        check: F,
    ) -> Result<GeneratedDeferred, SecurityCompileError>
    where
        F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
    {
        let compiler = self
            .binding
            .for_security_policy(policy(expected), &self.cache);
        compiler.compile_shared_with_generated_admission_deferred(identity, query, control, check)
    }

    fn go(
        &self,
        identity: &SecurityContext,
        query: &str,
        calls: &Cell<u32>,
        verdict: Result<(), ConstantCoverageError>,
    ) -> Result<GeneratedDeferred, SecurityCompileError> {
        let control = budget(u64::MAX);
        self.run(1, identity, query, &control, counting(calls, verdict))
    }
}

#[test]
fn generated_deferred_policy_mismatch_precedes_control_parse_and_callback() {
    let fixture = Fixture::new();
    let control = budget(u64::MAX);
    control.terminate(CANCELLED);
    let calls = Cell::new(0);
    let wrong = context(4, 2, 3);
    let check = counting(&calls, OK);
    let result = fixture.run(1, &wrong, "not valid SPARQL", &control, check);
    assert!(matches!(result, Err(SecurityCompileError::PolicyMismatch)));
    assert_eq!(calls.get(), 0);
    assert_eq!(fixture.cache.access_counts(), (0, 0));
    assert_eq!(work(&control), 0);

    let right = context(1, 2, 3);
    let check = counting(&calls, OK);
    let result = fixture.run(1, &right, "not valid SPARQL", &control, check);
    assert_eq!(control_cause(&result.unwrap_err()), Some(CANCELLED));
    assert_eq!(calls.get(), 0);

    let result = fixture.go(&right, "not valid SPARQL", &calls, OK);
    let (refusal, plan) = refused(result.unwrap());
    assert_eq!(refusal, FORM);
    assert!(plan.is_none());
    assert_eq!(calls.get(), 0);
    assert_eq!(fixture.cache.access_counts(), (0, 0));
}

#[test]
fn generated_deferred_security_admitted_keeps_partition_and_cache() {
    let fixture = Fixture::new();
    let alice = context(1, 2, 3);
    let bob = context(1, 4, 3);
    let calls = Cell::new(0);
    let cold = admitted(fixture.go(&alice, VALUES_Q, &calls, OK).unwrap());
    assert_eq!(fixture.cache.len(), 1);
    assert!(calls.get() > 0);
    let warm = admitted(fixture.go(&alice, VALUES_Q, &calls, OK).unwrap());
    assert!(Arc::ptr_eq(&cold, &warm));
    let other = admitted(fixture.go(&bob, VALUES_Q, &calls, OK).unwrap());
    assert!(!Arc::ptr_eq(&cold, &other));
    assert_eq!(fixture.cache.len(), 2);
    let compiler = fixture
        .binding
        .for_security_policy(policy(1), &fixture.cache);
    let existing = compiler.compile_shared(&alice, VALUES_Q).unwrap();
    assert!(Arc::ptr_eq(&cold, &existing));
    assert_eq!(fixture.binding.cache_len(), 0);
}

#[test]
fn generated_deferred_security_structural_refusal_bypasses_the_cache() {
    for (query, construct) in [(CONSTRUCT_Q, true), (DATASET_Q, false)] {
        let rule = if construct {
            ShapeRule::ConstructForm
        } else {
            ShapeRule::DatasetClause
        };
        let alice = context(1, 2, 3);
        let calls = Cell::new(0);

        let cold = Fixture::new();
        let (refusal, plan) = refused(cold.go(&alice, query, &calls, OK).unwrap());
        assert_eq!(refusal, GeneratedQueryRefusal::Rule(rule));
        assert!(plan.is_some());
        assert_eq!(cold.cache.access_counts(), (0, 0));
        assert_eq!(cold.cache.len(), 0);
        assert_eq!(cold.binding.cache_len(), 0);

        let fixture = Fixture::new();
        let compiler = fixture
            .binding
            .for_security_policy(policy(1), &fixture.cache);
        let cached = compiler.compile_shared(&alice, query).unwrap();
        assert_eq!(fixture.cache.len(), 1);
        fixture.cache.reset_access_counts();
        let (refusal, plan) = refused(fixture.go(&alice, query, &calls, OK).unwrap());
        assert_eq!(refusal, GeneratedQueryRefusal::Rule(rule));
        let plan = plan.expect("diagnostic authorization plan");
        assert!(!Arc::ptr_eq(&plan, &cached));
        let is_construct = matches!(plan.form, PlanForm::Construct { .. });
        assert_eq!(is_construct, construct);
        assert_eq!(calls.get(), 0);
        assert_eq!(fixture.cache.access_counts(), (0, 0));
        assert_eq!(fixture.cache.len(), 1);
        assert_eq!(fixture.binding.cache_len(), 0);
    }
}

#[test]
fn generated_deferred_security_coverage_refusal_never_returns_cached_plan() {
    let fixture = Fixture::new();
    let alice = context(1, 2, 3);
    let calls = Cell::new(0);
    let cached = admitted(fixture.go(&alice, VALUES_Q, &calls, OK).unwrap());
    let seen = calls.get();
    fixture.cache.reset_access_counts();
    let (refusal, plan) = refused(fixture.go(&alice, VALUES_Q, &calls, DENY).unwrap());
    assert_eq!(refusal, GeneratedQueryRefusal::CoverageRefused);
    let plan = plan.expect("diagnostic authorization plan");
    assert!(!Arc::ptr_eq(&plan, &cached));
    assert_eq!(calls.get(), seen + 1);
    assert_eq!(fixture.cache.access_counts(), (0, 0));
    assert_eq!(fixture.cache.len(), 1);
    let again = admitted(fixture.go(&alice, VALUES_Q, &calls, OK).unwrap());
    assert!(Arc::ptr_eq(&cached, &again));
    assert_eq!(fixture.binding.cache_len(), 0);
}

#[test]
fn generated_deferred_security_syntax_refusal_has_no_plan_or_cache_access() {
    let fixture = Fixture::new();
    let alice = context(1, 2, 3);
    let calls = Cell::new(0);
    admitted(fixture.go(&alice, VALUES_Q, &calls, OK).unwrap());
    let seen = calls.get();
    fixture.cache.reset_access_counts();
    for query in [UPDATE_Q, "CLEAR ALL", "not valid SPARQL", ""] {
        let (refusal, plan) = refused(fixture.go(&alice, query, &calls, OK).unwrap());
        assert_eq!(refusal, FORM, "{query:?}");
        assert!(plan.is_none(), "{query:?}");
        assert_eq!(calls.get(), seen, "{query:?}");
        assert_eq!(fixture.cache.access_counts(), (0, 0), "{query:?}");
        assert_eq!(fixture.cache.len(), 1, "{query:?}");
    }
}

#[test]
fn generated_deferred_security_unsupported_lowering_yields_no_plan() {
    let fixture = Fixture::new();
    let alice = context(1, 2, 3);
    let calls = Cell::new(0);
    let result = fixture.go(&alice, UNSUPPORTED_Q, &calls, DENY);
    let (refusal, plan) = refused(result.unwrap());
    assert_eq!(refusal, GeneratedQueryRefusal::CoverageRefused);
    assert!(plan.is_none());
    assert_eq!(fixture.cache.access_counts(), (0, 0));
    let error = fixture.go(&alice, UNSUPPORTED_Q, &calls, OK).unwrap_err();
    let typed = matches!(error, SecurityCompileError::Compiler(Error::Unsupported(_)));
    assert!(typed, "{error}");
    assert_eq!(fixture.cache.len(), 0);
}

#[test]
fn generated_deferred_security_lowering_control_failure_propagates() {
    let parsed = crate::parse_query(VALUES_Q).unwrap();
    let screening = budget(u64::MAX);
    assert!(admit_parsed(&parsed, &screening, deny).is_err());
    let screen = work(&screening);
    assert!(screen > 0);
    let fixture = Fixture::new();
    let alice = context(1, 2, 3);
    let control = budget(screen);
    let error = fixture
        .run(1, &alice, VALUES_Q, &control, deny)
        .unwrap_err();
    assert_eq!(control_cause(&error), Some(EXCEEDED));
    assert_eq!(control.terminal(), Some(EXCEEDED));
    assert_eq!(fixture.cache.access_counts(), (0, 0));
    assert_eq!(fixture.cache.len(), 0);
}

#[test]
fn generated_deferred_security_terminal_control_precedes_refusal() {
    let alice = context(1, 2, 3);
    for cause in [CANCELLED, DEADLINE] {
        let fixture = Fixture::new();
        let control = budget(u64::MAX);
        let calls = Cell::new(0);
        let check = |_: ConstantOccurrence<'_>| {
            calls.set(calls.get() + 1);
            control.terminate(cause);
            DENY
        };
        let error = fixture
            .run(1, &alice, VALUES_Q, &control, check)
            .unwrap_err();
        assert_eq!(control_cause(&error), Some(cause));
        assert_eq!(calls.get(), 1);
        assert_eq!(fixture.cache.access_counts(), (0, 0));
        assert_eq!(fixture.cache.len(), 0);
    }
    for query in [VALUES_Q, UPDATE_Q, CONSTRUCT_Q] {
        let fixture = Fixture::new();
        let control = budget(u64::MAX);
        control.terminate(CANCELLED);
        let calls = Cell::new(0);
        let check = counting(&calls, DENY);
        let error = fixture.run(1, &alice, query, &control, check).unwrap_err();
        assert_eq!(control_cause(&error), Some(CANCELLED), "{query:?}");
        assert_eq!(calls.get(), 0, "{query:?}");
        assert_eq!(fixture.cache.access_counts(), (0, 0), "{query:?}");
    }
}

#[test]
fn generated_deferred_security_admitted_work_matches_existing_admission() {
    let alice = context(1, 2, 3);
    for warm in [false, true] {
        let measure = |deferred: bool| {
            let fixture = Fixture::new();
            let compiler = fixture
                .binding
                .for_security_policy(policy(1), &fixture.cache);
            if warm {
                compiler.compile_shared(&alice, VALUES_Q).unwrap();
            }
            let control = budget(u64::MAX);
            if deferred {
                let result = compiler.compile_shared_with_generated_admission_deferred(
                    &alice, VALUES_Q, &control, allow,
                );
                admitted(result.unwrap());
            } else {
                let result = compiler
                    .compile_shared_with_generated_admission(&alice, VALUES_Q, &control, allow);
                result.unwrap();
            }
            work(&control)
        };
        let existing = measure(false);
        assert!(existing > 0);
        assert_eq!(measure(true), existing);
    }
}

#[test]
fn generated_deferred_security_calls_parse_exactly_once() {
    isolated(|| {
        let fixture = Fixture::new();
        let alice = context(1, 2, 3);
        let calls = Cell::new(0);
        let cases = [
            (VALUES_Q, OK),
            (VALUES_Q, OK),
            (VALUES_Q, DENY),
            (CONSTRUCT_Q, OK),
            (DATASET_Q, OK),
            (UPDATE_Q, OK),
            (UNSUPPORTED_Q, DENY),
        ];
        let mut counts = Vec::new();
        for (query, verdict) in cases {
            let (result, parses) = parse_spans(|| fixture.go(&alice, query, &calls, verdict));
            assert!(result.is_ok(), "{query:?}");
            counts.push(parses);
        }
        assert!(counts.iter().all(|parses| *parses == 1), "{counts:?}");
        let control = budget(u64::MAX);
        let check = counting(&calls, OK);
        let (mismatch, parses) = parse_spans(|| fixture.run(4, &alice, VALUES_Q, &control, check));
        assert!(matches!(
            mismatch,
            Err(SecurityCompileError::PolicyMismatch)
        ));
        assert_eq!(parses, 0);
    });
}
