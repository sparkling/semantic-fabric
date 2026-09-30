use std::cell::Cell;
use std::sync::Arc;

use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
    UncontrolledQueryControl,
};
use sf_core::{SourceId, SourceMapping};
use sf_sql::Dialect;

use super::test_support::{isolated, parse_spans};
use super::{
    admit_parsed, authorization_only, ConstantCoverageError, ConstantOccurrence, GeneratedDeferred,
    GeneratedQueryRefusal, ShapeRule,
};
use crate::{CompilerBinding, Epoch, Error, Plan, PlanForm, Tbox};

const VALUES_Q: &str =
    "SELECT ?x WHERE { VALUES ?x { 1 2 3 } FILTER EXISTS { VALUES ?inside { 7 } } }";
const ASK_Q: &str = "ASK { ?s ?p ?o }";
const CONSTRUCT_Q: &str = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";
const DATASET_Q: &str = "SELECT * FROM <urn:g> WHERE { ?s ?p ?o }";
const UPDATE_Q: &str = "INSERT DATA { <urn:s> <urn:p> <urn:o> }";
const UNSUPPORTED_Q: &str = "SELECT * WHERE { ?s !<http://ex/p> ?o }";
const SECRET_Q: &str = "SELECT * WHERE { <urn:secret-iri> <urn:p> 'hidden-literal' }";

const EXCEEDED: QueryControlError = QueryControlError::CompilerWorkExceeded;
const CANCELLED: QueryControlError = QueryControlError::Cancelled;
const DEADLINE: QueryControlError = QueryControlError::DeadlineExceeded;
const FREE: &UncontrolledQueryControl = &UncontrolledQueryControl;
const OK: Result<(), ConstantCoverageError> = Ok(());
const DENY: Result<(), ConstantCoverageError> = Err(ConstantCoverageError::Uncovered);
const FORM: GeneratedQueryRefusal = GeneratedQueryRefusal::Rule(ShapeRule::FormNotAdmitted);

#[derive(Clone, Copy)]
enum Mode {
    Cold,
    Warm,
    Uncached,
}

const MODES: [Mode; 3] = [Mode::Cold, Mode::Warm, Mode::Uncached];

fn fresh() -> CompilerBinding {
    CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), vec![]),
        Dialect::Sqlite,
        Tbox::default(),
        vec![],
        Epoch::default(),
        8,
    )
}

fn prepared(mode: Mode) -> CompilerBinding {
    let binding = fresh();
    if matches!(mode, Mode::Warm) {
        binding.compile_shared(ASK_Q).unwrap();
    }
    binding
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

fn run<F>(
    mode: Mode,
    binding: &CompilerBinding,
    query: &str,
    control: &dyn QueryControl,
    check: F,
) -> Result<GeneratedDeferred, Error>
where
    F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
{
    if matches!(mode, Mode::Uncached) {
        binding.compile_uncached_shared_with_generated_admission_deferred(query, control, check)
    } else {
        binding.compile_shared_with_generated_admission_deferred(query, control, check)
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

fn control_cause(error: &Error) -> Option<QueryControlError> {
    match error {
        Error::QueryControl(cause) => Some(*cause),
        _ => None,
    }
}

#[test]
fn generated_deferred_admitted_keeps_existing_cache_behavior() {
    let binding = fresh();
    let uncached = admitted(run(Mode::Uncached, &binding, VALUES_Q, FREE, allow).unwrap());
    assert_eq!(binding.cache_len(), 0);
    let cold = admitted(run(Mode::Cold, &binding, VALUES_Q, FREE, allow).unwrap());
    assert_eq!(binding.cache_len(), 1);
    let warm = admitted(run(Mode::Warm, &binding, VALUES_Q, FREE, allow).unwrap());
    assert!(Arc::ptr_eq(&cold, &warm));
    assert!(!Arc::ptr_eq(&uncached, &cold));
    let raw = binding.compile_shared(VALUES_Q).unwrap();
    assert!(Arc::ptr_eq(&cold, &raw));
    assert_eq!(binding.cache_len(), 1);

    let existing = binding
        .compile_uncached_shared_with_generated_admission(ASK_Q, FREE, allow)
        .unwrap();
    let deferred = admitted(run(Mode::Uncached, &binding, ASK_Q, FREE, allow).unwrap());
    assert_eq!(format!("{existing:?}"), format!("{deferred:?}"));
}

#[test]
fn generated_deferred_structural_refusal_returns_uncached_diagnostic_plan() {
    let cases = [
        (CONSTRUCT_Q, ShapeRule::ConstructForm, true),
        (DATASET_Q, ShapeRule::DatasetClause, false),
    ];
    for (query, rule, construct) in cases {
        for mode in MODES {
            let binding = prepared(mode);
            let entries = binding.cache_len();
            let calls = Cell::new(0);
            let deferred = run(mode, &binding, query, FREE, counting(&calls, OK)).unwrap();
            let (refusal, plan) = refused(deferred);
            assert_eq!(refusal, GeneratedQueryRefusal::Rule(rule), "{query}");
            let plan = plan.expect("diagnostic authorization plan");
            let is_construct = matches!(plan.form, PlanForm::Construct { .. });
            assert_eq!(is_construct, construct, "{query}");
            assert_eq!(calls.get(), 0, "{query}");
            assert_eq!(binding.cache_len(), entries, "{query}");
            let ordinary = binding.compile_shared(query).unwrap();
            assert!(!Arc::ptr_eq(&plan, &ordinary), "{query}");
            assert_eq!(binding.cache_len(), entries + 1, "{query}");
        }
    }
}

#[test]
fn generated_deferred_coverage_refusal_never_returns_or_stores_a_cached_plan() {
    let binding = fresh();
    let cached = binding.compile_shared(VALUES_Q).unwrap();
    for mode in MODES {
        let calls = Cell::new(0);
        let deferred = run(mode, &binding, VALUES_Q, FREE, counting(&calls, DENY)).unwrap();
        let (refusal, plan) = refused(deferred);
        assert_eq!(refusal, GeneratedQueryRefusal::CoverageRefused);
        let plan = plan.expect("diagnostic authorization plan");
        assert!(!Arc::ptr_eq(&plan, &cached));
        assert_eq!(calls.get(), 1);
        assert_eq!(binding.cache_len(), 1);
    }
    for mode in [Mode::Cold, Mode::Uncached] {
        let empty = fresh();
        let (refusal, plan) = refused(run(mode, &empty, VALUES_Q, FREE, deny).unwrap());
        assert_eq!(refusal, GeneratedQueryRefusal::CoverageRefused);
        assert!(plan.is_some());
        assert_eq!(empty.cache_len(), 0);
    }
}

#[test]
fn generated_deferred_syntax_refusal_has_no_plan_and_no_callback() {
    for query in [UPDATE_Q, "CLEAR ALL", "not sparql", ""] {
        for mode in MODES {
            let binding = prepared(mode);
            let entries = binding.cache_len();
            let calls = Cell::new(0);
            let deferred = run(mode, &binding, query, FREE, counting(&calls, OK)).unwrap();
            let (refusal, plan) = refused(deferred);
            assert_eq!(refusal, FORM, "{query:?}");
            assert!(plan.is_none(), "{query:?}");
            assert_eq!(calls.get(), 0, "{query:?}");
            assert_eq!(binding.cache_len(), entries, "{query:?}");
        }
    }
}

#[test]
fn generated_deferred_unsupported_lowering_yields_no_plan() {
    for mode in [Mode::Cold, Mode::Uncached] {
        let binding = fresh();
        let (refusal, plan) = refused(run(mode, &binding, UNSUPPORTED_Q, FREE, deny).unwrap());
        assert_eq!(refusal, GeneratedQueryRefusal::CoverageRefused);
        assert!(plan.is_none());
        assert_eq!(binding.cache_len(), 0);
        let error = run(mode, &binding, UNSUPPORTED_Q, FREE, allow).unwrap_err();
        assert!(matches!(error, Error::Unsupported(_)), "{error}");
    }
}

#[test]
fn generated_deferred_lowering_control_failure_propagates_instead_of_refusal() {
    let parsed = crate::parse_query(VALUES_Q).unwrap();
    let screening = budget(u64::MAX);
    assert!(admit_parsed(&parsed, &screening, deny).is_err());
    let screen = work(&screening);
    assert!(screen > 0);
    for mode in [Mode::Cold, Mode::Uncached] {
        let binding = fresh();
        let control = budget(screen);
        let error = run(mode, &binding, VALUES_Q, &control, deny).unwrap_err();
        assert_eq!(control_cause(&error), Some(EXCEEDED));
        assert_eq!(control.terminal(), Some(EXCEEDED));
        assert_eq!(binding.cache_len(), 0);
    }
}

#[test]
fn generated_deferred_only_unsupported_lowering_is_absorbed() {
    let plan = fresh().compile_uncached_shared(ASK_Q).unwrap();
    let kept = authorization_only(Ok(Arc::clone(&plan))).unwrap().unwrap();
    assert!(Arc::ptr_eq(&kept, &plan));
    let unsupported = Error::Unsupported("x".into());
    assert!(authorization_only(Err(unsupported)).unwrap().is_none());
    let typed = [
        Error::Mapping("m".into()),
        Error::Sql("s".into()),
        Error::Core("c".into()),
        Error::Parse("p".into()),
        Error::QueryControl(CANCELLED),
    ];
    for error in typed {
        let text = error.to_string();
        let kept = authorization_only(Err(error)).unwrap_err();
        assert_eq!(kept.to_string(), text);
    }
}

#[test]
fn generated_deferred_terminal_control_precedes_refusal() {
    for query in [VALUES_Q, UPDATE_Q, "not sparql", CONSTRUCT_Q] {
        for mode in MODES {
            let binding = prepared(mode);
            let entries = binding.cache_len();
            let control = budget(u64::MAX);
            control.terminate(CANCELLED);
            let calls = Cell::new(0);
            let check = counting(&calls, DENY);
            let error = run(mode, &binding, query, &control, check).unwrap_err();
            assert_eq!(control_cause(&error), Some(CANCELLED), "{query:?}");
            assert_eq!(calls.get(), 0, "{query:?}");
            assert_eq!(work(&control), 0, "{query:?}");
            assert_eq!(binding.cache_len(), entries, "{query:?}");
        }
    }
    for cause in [CANCELLED, DEADLINE] {
        for mode in [Mode::Cold, Mode::Uncached] {
            let binding = fresh();
            let control = budget(u64::MAX);
            let calls = Cell::new(0);
            let check = |_: ConstantOccurrence<'_>| {
                calls.set(calls.get() + 1);
                control.terminate(cause);
                DENY
            };
            let error = run(mode, &binding, VALUES_Q, &control, check).unwrap_err();
            assert_eq!(control_cause(&error), Some(cause));
            assert_eq!(calls.get(), 1);
            assert_eq!(control.terminal(), Some(cause));
            assert_eq!(binding.cache_len(), 0);
        }
    }
}

#[test]
fn generated_deferred_admitted_work_matches_existing_generated_admission() {
    for mode in MODES {
        let existing = budget(u64::MAX);
        let binding = prepared(mode);
        let plan = if matches!(mode, Mode::Uncached) {
            binding.compile_uncached_shared_with_generated_admission(VALUES_Q, &existing, allow)
        } else {
            binding.compile_shared_with_generated_admission(VALUES_Q, &existing, allow)
        };
        plan.unwrap();
        let deferred = budget(u64::MAX);
        let binding = prepared(mode);
        admitted(run(mode, &binding, VALUES_Q, &deferred, allow).unwrap());
        assert!(work(&existing) > 0);
        assert_eq!(work(&deferred), work(&existing));
    }
}

#[test]
fn generated_deferred_leaves_existing_refusal_entry_points_unchanged() {
    for mode in MODES {
        let binding = prepared(mode);
        let result = if matches!(mode, Mode::Uncached) {
            binding.compile_uncached_shared_with_generated_admission(CONSTRUCT_Q, FREE, allow)
        } else {
            binding.compile_shared_with_generated_admission(CONSTRUCT_Q, FREE, allow)
        };
        let refusal = match result.unwrap_err() {
            super::GeneratedCompileError::Refused(refusal) => refusal,
            other => panic!("expected refusal, got {other:?}"),
        };
        assert_eq!(
            refusal,
            GeneratedQueryRefusal::Rule(ShapeRule::ConstructForm)
        );
    }
}

#[test]
fn generated_deferred_debug_does_not_render_query_or_plan() {
    let binding = fresh();
    let deferred = run(Mode::Uncached, &binding, SECRET_Q, FREE, deny).unwrap();
    let rendered = format!("{deferred:?}");
    for secret in ["secret", "hidden", "urn:"] {
        assert!(!rendered.contains(secret), "{rendered}");
    }
    let plain = run(Mode::Uncached, &binding, SECRET_Q, FREE, allow).unwrap();
    assert_eq!(format!("{plain:?}"), "Admitted(<plan>)");
}

#[test]
fn generated_deferred_calls_parse_exactly_once() {
    isolated(|| {
        let cases = [
            (VALUES_Q, OK),
            (VALUES_Q, DENY),
            (CONSTRUCT_Q, OK),
            (DATASET_Q, OK),
            (UPDATE_Q, OK),
            ("not sparql", OK),
        ];
        let binding = fresh();
        let calls = Cell::new(0);
        let mut counts = Vec::new();
        for mode in MODES {
            for (query, verdict) in cases {
                let (result, parses) =
                    parse_spans(|| run(mode, &binding, query, FREE, counting(&calls, verdict)));
                assert!(result.is_ok(), "{query:?}");
                counts.push(parses);
            }
        }
        assert!(counts.iter().all(|parses| *parses == 1), "{counts:?}");
        let control = budget(u64::MAX);
        control.terminate(CANCELLED);
        let (result, parses) = parse_spans(|| {
            run(
                Mode::Cold,
                &binding,
                UPDATE_Q,
                &control,
                counting(&calls, OK),
            )
        });
        assert_eq!(control_cause(&result.unwrap_err()), Some(CANCELLED));
        assert_eq!(parses, 0);
    });
}
