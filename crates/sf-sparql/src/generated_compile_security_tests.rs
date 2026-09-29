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
    admit_parsed, ConstantCoverageError, ConstantOccurrence, GeneratedQueryRefusal, ShapeRule,
};
use crate::{CompilerBinding, CompilerSchema, Error, Plan, Tbox};

const VALUES_Q: &str =
    "SELECT ?x WHERE { VALUES ?x { 1 2 3 } FILTER EXISTS { VALUES ?inside { 7 } } }";
const UPDATE_Q: &str = "INSERT DATA { <urn:s> <urn:p> <urn:o> }";
const EXCEEDED: QueryControlError = QueryControlError::CompilerWorkExceeded;
const CANCELLED: QueryControlError = QueryControlError::Cancelled;
const DEADLINE: QueryControlError = QueryControlError::DeadlineExceeded;
const OK: Result<(), ConstantCoverageError> = Ok(());
const DENY: Result<(), ConstantCoverageError> = Err(ConstantCoverageError::Uncovered);

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

fn security_cache() -> SecurityPlanCache {
    SecurityPlanCache::new(NonZeroUsize::new(8).unwrap())
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

fn counting<'a>(
    calls: &'a Cell<u32>,
    verdict: Result<(), ConstantCoverageError>,
) -> impl FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError> + 'a {
    move |_| {
        calls.set(calls.get() + 1);
        verdict
    }
}

fn control_cause(error: &SecurityCompileError) -> Option<QueryControlError> {
    match error {
        SecurityCompileError::Compiler(Error::QueryControl(cause)) => Some(*cause),
        _ => None,
    }
}

fn is_coverage(error: &SecurityCompileError) -> bool {
    let SecurityCompileError::GeneratedQueryRefused(refusal) = error else {
        return false;
    };
    *refusal == GeneratedQueryRefusal::CoverageRefused
}

fn is_form_refusal(error: &SecurityCompileError) -> bool {
    let SecurityCompileError::GeneratedQueryRefused(refusal) = error else {
        return false;
    };
    *refusal == GeneratedQueryRefusal::Rule(ShapeRule::FormNotAdmitted)
}

struct Fixture {
    binding: CompilerBinding,
    cache: SecurityPlanCache,
}

impl Fixture {
    fn new() -> Self {
        Self {
            binding: binding(),
            cache: security_cache(),
        }
    }

    fn admit(
        &self,
        identity: &SecurityContext,
        calls: &Cell<u32>,
        verdict: Result<(), ConstantCoverageError>,
    ) -> Result<Arc<Plan>, SecurityCompileError> {
        self.admit_under(1, identity, calls, verdict)
    }

    fn admit_under(
        &self,
        expected: u8,
        identity: &SecurityContext,
        calls: &Cell<u32>,
        verdict: Result<(), ConstantCoverageError>,
    ) -> Result<Arc<Plan>, SecurityCompileError> {
        let control = budget(u64::MAX);
        let binding = &self.binding;
        let compiler = binding.for_security_policy(policy(expected), &self.cache);
        let check = counting(calls, verdict);
        compiler.compile_shared_with_generated_admission(identity, VALUES_Q, &control, check)
    }
}

fn run(
    warm: bool,
    admitted: bool,
    control: &QueryBudget,
) -> Result<Arc<Plan>, SecurityCompileError> {
    let binding = binding();
    let cache = security_cache();
    let compiler = binding.for_security_policy(policy(1), &cache);
    let identity = context(1, 2, 3);
    if warm {
        compiler.compile_shared(&identity, VALUES_Q).unwrap();
    }
    if admitted {
        compiler.compile_shared_with_generated_admission(&identity, VALUES_Q, control, allow)
    } else {
        compiler.compile_shared_with_work_control(&identity, VALUES_Q, control)
    }
}

#[test]
fn policy_mismatch_precedes_parse_screen_callback_and_control() {
    let binding = binding();
    let cache = security_cache();
    let compiler = binding.for_security_policy(policy(1), &cache);
    let control = budget(u64::MAX);
    control.terminate(CANCELLED);
    let calls = Cell::new(0);
    let wrong = context(4, 2, 3);
    let result = compiler.compile_shared_with_generated_admission(
        &wrong,
        "not valid SPARQL",
        &control,
        counting(&calls, OK),
    );
    assert!(matches!(result, Err(SecurityCompileError::PolicyMismatch)));
    assert_eq!(calls.get(), 0);
    assert_eq!(cache.access_counts(), (0, 0));
    assert_eq!(work(&control), 0);

    let right = context(1, 2, 3);
    let result = compiler.compile_shared_with_generated_admission(
        &right,
        "not valid SPARQL",
        &control,
        counting(&calls, OK),
    );
    assert_eq!(control_cause(&result.unwrap_err()), Some(CANCELLED));
    assert_eq!(calls.get(), 0);

    let fresh = budget(u64::MAX);
    let result = compiler.compile_shared_with_generated_admission(
        &right,
        "not valid SPARQL",
        &fresh,
        counting(&calls, OK),
    );
    assert!(is_form_refusal(&result.unwrap_err()));
    assert_eq!(calls.get(), 0);
    assert_eq!(cache.access_counts(), (0, 0));
}

#[test]
fn form_refusal_precedes_cache_and_callback_on_a_warm_secured_route() {
    let fixture = Fixture::new();
    let identity = context(1, 2, 3);
    let calls = Cell::new(0);
    fixture.admit(&identity, &calls, OK).unwrap();
    let warm_calls = calls.get();
    assert_eq!(fixture.cache.len(), 1);
    fixture.cache.reset_access_counts();
    for query in [UPDATE_Q, "CLEAR ALL", "not valid SPARQL", ""] {
        let control = budget(u64::MAX);
        let compiler = fixture
            .binding
            .for_security_policy(policy(1), &fixture.cache);
        let check = counting(&calls, OK);
        let result =
            compiler.compile_shared_with_generated_admission(&identity, query, &control, check);
        assert!(is_form_refusal(&result.unwrap_err()), "{query:?}");
        assert_eq!(calls.get(), warm_calls, "{query:?}");
        assert_eq!(fixture.cache.access_counts(), (0, 0), "{query:?}");
        assert_eq!(fixture.cache.len(), 1, "{query:?}");
        assert_eq!(work(&control), 0, "{query:?}");
    }
    let control = budget(u64::MAX);
    control.terminate(CANCELLED);
    let compiler = fixture
        .binding
        .for_security_policy(policy(1), &fixture.cache);
    let check = counting(&calls, OK);
    let result =
        compiler.compile_shared_with_generated_admission(&identity, UPDATE_Q, &control, check);
    assert_eq!(control_cause(&result.unwrap_err()), Some(CANCELLED));
    assert_eq!(calls.get(), warm_calls);
    assert_eq!(fixture.cache.access_counts(), (0, 0));
}

#[test]
fn warm_cache_admission_reruns_and_denial_refuses() {
    let fixture = Fixture::new();
    let identity = context(1, 2, 3);
    let calls = Cell::new(0);
    let cold = fixture.admit(&identity, &calls, OK).unwrap();
    assert_eq!(fixture.cache.len(), 1);
    let cold_calls = calls.get();
    assert!(cold_calls > 0);
    fixture.cache.reset_access_counts();
    let denied = fixture.admit(&identity, &calls, DENY);
    assert!(is_coverage(&denied.unwrap_err()));
    assert_eq!(calls.get(), cold_calls + 1);
    assert_eq!(fixture.cache.access_counts(), (0, 0));
    assert_eq!(fixture.cache.len(), 1);
    let warm = fixture.admit(&identity, &calls, OK).unwrap();
    assert!(Arc::ptr_eq(&cold, &warm));
    assert_eq!(calls.get(), cold_calls + 1 + cold_calls);
    assert_eq!(fixture.binding.cache_len(), 0);
}

#[test]
fn each_security_context_reruns_the_callback_and_keeps_its_own_plan() {
    let fixture = Fixture::new();
    let calls = Cell::new(0);
    let alice = context(1, 2, 3);
    let bob = context(1, 4, 3);
    let attributes = context(1, 2, 4);
    let a = fixture.admit(&alice, &calls, OK).unwrap();
    let after_alice = calls.get();
    let b = fixture.admit(&bob, &calls, OK).unwrap();
    assert!(calls.get() > after_alice);
    assert!(!Arc::ptr_eq(&a, &b));
    let c = fixture.admit(&attributes, &calls, OK).unwrap();
    assert!(!Arc::ptr_eq(&a, &c));
    assert_eq!(fixture.cache.len(), 3);
    let denied = fixture.admit(&bob, &calls, DENY);
    assert!(is_coverage(&denied.unwrap_err()));
    let again = fixture.admit(&alice, &calls, OK).unwrap();
    assert!(Arc::ptr_eq(&a, &again));
    assert_eq!(fixture.cache.len(), 3);
    assert_eq!(fixture.binding.cache_len(), 0);
    let unscoped = fixture.binding.compile_shared(VALUES_Q).unwrap();
    assert!(!Arc::ptr_eq(&a, &unscoped));
    assert_eq!(fixture.binding.cache_len(), 1);
    assert_eq!(fixture.cache.len(), 3);
}

#[test]
fn admission_work_is_default_work_plus_screen_work_at_exact_budget() {
    let parsed = crate::parse_query(VALUES_Q).unwrap();
    let screening = budget(u64::MAX);
    admit_parsed(&parsed, &screening, allow).unwrap();
    let screen = work(&screening);
    assert!(screen > 0);
    for warm in [false, true] {
        let baseline = budget(u64::MAX);
        run(warm, false, &baseline).unwrap();
        let total = work(&baseline) + screen;
        let exact = budget(total);
        run(warm, true, &exact).unwrap();
        assert_eq!(work(&exact), total);
        let short = budget(total - 1);
        let error = run(warm, true, &short).unwrap_err();
        assert_eq!(control_cause(&error), Some(EXCEEDED));
        assert_eq!(short.terminal(), Some(EXCEEDED));
    }
}

#[test]
fn callback_control_failure_keeps_its_typed_cause() {
    let fixture = Fixture::new();
    let identity = context(1, 2, 3);
    let calls = Cell::new(0);
    let failure = Err(ConstantCoverageError::Control(DEADLINE));
    let result = fixture.admit(&identity, &calls, failure);
    let error = result.unwrap_err();
    assert!(!is_coverage(&error));
    assert_eq!(control_cause(&error), Some(DEADLINE));
    assert_eq!(fixture.cache.access_counts(), (0, 0));
    assert_eq!(fixture.cache.len(), 0);
}

#[test]
fn default_security_path_is_unchanged_after_admission() {
    let fixture = Fixture::new();
    let identity = context(1, 2, 3);
    let calls = Cell::new(0);
    let admitted = fixture.admit(&identity, &calls, OK).unwrap();
    let raw_control = budget(u64::MAX);
    run(true, false, &raw_control).unwrap();
    let binding = &fixture.binding;
    let compiler = binding.for_security_policy(policy(1), &fixture.cache);
    let admitted_control = budget(u64::MAX);
    let hit = compiler.compile_shared_with_work_control(&identity, VALUES_Q, &admitted_control);
    assert!(Arc::ptr_eq(&admitted, &hit.unwrap()));
    assert!(work(&raw_control) > 0);
    assert_eq!(work(&admitted_control), work(&raw_control));
    let raw = compiler.compile_shared(&identity, VALUES_Q).unwrap();
    assert!(Arc::ptr_eq(&admitted, &raw));
}

#[test]
fn security_admission_parses_exactly_once_and_policy_mismatch_never_parses() {
    isolated(|| {
        let fixture = Fixture::new();
        let identity = context(1, 2, 3);
        let calls = Cell::new(0);
        let (cold, first) = parse_spans(|| fixture.admit(&identity, &calls, OK));
        cold.unwrap();
        let (warm, second) = parse_spans(|| fixture.admit(&identity, &calls, OK));
        warm.unwrap();
        let (denied, third) = parse_spans(|| fixture.admit(&identity, &calls, DENY));
        assert!(denied.is_err());
        let (mismatch, fourth) = parse_spans(|| fixture.admit_under(4, &identity, &calls, OK));
        assert!(mismatch.is_err());
        assert_eq!([first, second, third, fourth], [1, 1, 1, 0]);
    });
}
