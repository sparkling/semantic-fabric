use std::cell::Cell;
use std::sync::Arc;

pub(crate) use super::test_support::{isolated, parse_spans};
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
    UncontrolledQueryControl,
};
use sf_core::{SourceId, SourceMapping};
use sf_sql::Dialect;

use super::{
    admit_parsed, ConstantCoverageError, ConstantOccurrence, ConstantRole, GeneratedCompileError,
    GeneratedQueryRefusal, ShapeRule,
};
use crate::{CompilerBinding, Epoch, Error, Plan, PlanForm, Tbox};

const VALUES_Q: &str =
    "SELECT ?x WHERE { VALUES ?x { 1 2 3 } FILTER EXISTS { VALUES ?inside { 7 } } }";
const ASK_Q: &str = "ASK { ?s ?p ?o }";
const ROLES_Q: &str =
    "ASK { GRAPH <urn:g> { ?x a <urn:C> . ?x <urn:p> <urn:C> } FILTER(?x = <urn:z>) }";
const LIT_Q: &str = "ASK { ?x <urn:p> '5'^^<urn:dt> }";
const DUP_Q: &str = "ASK { ?a <urn:p> ?b . ?c <urn:p> ?d }";
const UPDATE_Q: &str = "INSERT DATA { <urn:s> <urn:p> <urn:o> }";
const SECRET_Q: &str = "SELECT * WHERE { <urn:secret-iri> <urn:p> 'hidden-literal' }";
const SECRET_SERVICE: &str = "ASK { SERVICE <urn:secret-service> { ?s ?p ?o } }";
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

const REFUSED: [&str; 5] = [
    "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
    "DESCRIBE <urn:x>",
    "ASK { SERVICE <urn:s> { ?s ?p ?o } }",
    "SELECT * FROM <urn:g> WHERE { ?s ?p ?o }",
    "SELECT * FROM NAMED <urn:g> WHERE { ?s ?p ?o }",
];
const RULES: [ShapeRule; 5] = [
    ShapeRule::ConstructForm,
    ShapeRule::DescribeForm,
    ShapeRule::ServiceInPattern,
    ShapeRule::DatasetClause,
    ShapeRule::DatasetClause,
];

const EXCEEDED: QueryControlError = QueryControlError::CompilerWorkExceeded;
const CANCELLED: QueryControlError = QueryControlError::Cancelled;
const DEADLINE: QueryControlError = QueryControlError::DeadlineExceeded;
const FREE: &UncontrolledQueryControl = &UncontrolledQueryControl;
const OK: Result<(), ConstantCoverageError> = Ok(());
const DENY: Result<(), ConstantCoverageError> = Err(ConstantCoverageError::Uncovered);

#[derive(Clone, Copy)]
enum Mode {
    Cold,
    Warm,
    Uncached,
}

const MODES: [Mode; 3] = [Mode::Cold, Mode::Warm, Mode::Uncached];

fn binding() -> CompilerBinding {
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
    let binding = binding();
    if matches!(mode, Mode::Warm) {
        binding.compile_shared(VALUES_Q).unwrap();
    }
    binding
}

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

fn work(control: &QueryBudget) -> u64 {
    control.consumed(QueryCharge::CompilerWork)
}

fn measure<T>(run: impl FnOnce(&QueryBudget) -> T) -> (T, u64) {
    let control = budget(u64::MAX);
    let value = run(&control);
    (value, work(&control))
}

fn allow(_: ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError> {
    Ok(())
}

fn deny(_: ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError> {
    Err(ConstantCoverageError::Uncovered)
}

fn scripted<'a>(
    calls: &'a Cell<u32>,
    stop: Option<(&'a QueryBudget, QueryControlError)>,
    verdict: Result<(), ConstantCoverageError>,
) -> impl FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError> + 'a {
    move |_| {
        calls.set(calls.get() + 1);
        if let Some((control, cause)) = stop {
            control.terminate(cause);
        }
        verdict
    }
}

fn admitted<F>(
    mode: Mode,
    binding: &CompilerBinding,
    query: &str,
    control: &dyn QueryControl,
    check: F,
) -> Result<Arc<Plan>, GeneratedCompileError>
where
    F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
{
    if matches!(mode, Mode::Uncached) {
        binding.compile_uncached_shared_with_generated_admission(query, control, check)
    } else {
        binding.compile_shared_with_generated_admission(query, control, check)
    }
}

fn admitted_on<F>(
    mode: Mode,
    binding: &CompilerBinding,
    query: &str,
    check: F,
) -> Result<Arc<Plan>, GeneratedCompileError>
where
    F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
{
    admitted(mode, binding, query, FREE, check)
}

fn admitted_run<F>(
    mode: Mode,
    binding: &CompilerBinding,
    control: &dyn QueryControl,
    check: F,
) -> Result<Arc<Plan>, GeneratedCompileError>
where
    F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
{
    admitted(mode, binding, VALUES_Q, control, check)
}

fn default_run(
    mode: Mode,
    binding: &CompilerBinding,
    control: &dyn QueryControl,
) -> crate::Result<Arc<Plan>> {
    if matches!(mode, Mode::Uncached) {
        binding.compile_uncached_shared_with_work_control(VALUES_Q, control)
    } else {
        binding.compile_shared_with_work_control(VALUES_Q, control)
    }
}

fn control_cause(error: &GeneratedCompileError) -> Option<QueryControlError> {
    match error {
        GeneratedCompileError::Compiler(Error::QueryControl(cause)) => Some(*cause),
        _ => None,
    }
}

fn refused_rule(error: &GeneratedCompileError) -> Option<ShapeRule> {
    match error {
        GeneratedCompileError::Refused(GeneratedQueryRefusal::Rule(rule)) => Some(*rule),
        _ => None,
    }
}

fn is_coverage_refusal(error: &GeneratedCompileError) -> bool {
    let GeneratedCompileError::Refused(refusal) = error else {
        return false;
    };
    *refusal == GeneratedQueryRefusal::CoverageRefused
}

fn collect(sparql: &str) -> Vec<(String, ConstantRole)> {
    let parsed = crate::parse_query(sparql).unwrap();
    let mut seen = Vec::new();
    admit_parsed(&parsed, FREE, |found| {
        seen.push((found.iri().to_owned(), found.role()));
        Ok(())
    })
    .unwrap();
    seen.sort();
    seen
}

fn owned(items: &[(&str, ConstantRole)]) -> Vec<(String, ConstantRole)> {
    let mut owned: Vec<_> = items
        .iter()
        .map(|(iri, role)| ((*iri).to_owned(), *role))
        .collect();
    owned.sort();
    owned
}

#[test]
fn select_and_ask_are_admitted_cached_and_uncached() {
    for query in [VALUES_Q, ASK_Q] {
        let binding = binding();
        let uncached = admitted_on(Mode::Uncached, &binding, query, allow);
        assert!(uncached.is_ok());
        assert_eq!(binding.cache_len(), 0);
        let cold = admitted_on(Mode::Cold, &binding, query, allow).unwrap();
        assert_eq!(binding.cache_len(), 1);
        let warm = admitted_on(Mode::Warm, &binding, query, allow).unwrap();
        assert!(Arc::ptr_eq(&cold, &warm));
        let raw = binding.compile_shared(query).unwrap();
        assert!(Arc::ptr_eq(&cold, &raw));
        let is_ask = matches!(cold.form, PlanForm::Ask);
        assert_eq!(is_ask, query == ASK_Q);
    }
}

#[test]
fn entry_points_report_the_seam_occurrences() {
    let expected = collect(VALUES_Q);
    assert_eq!(
        expected[0],
        (XSD_INTEGER.to_owned(), ConstantRole::LiteralDatatype)
    );
    for mode in MODES {
        let mut seen = Vec::new();
        admitted_on(mode, &binding(), VALUES_Q, |found| {
            seen.push((found.iri().to_owned(), found.role()));
            Ok(())
        })
        .unwrap();
        seen.sort();
        assert_eq!(seen, expected);
    }
}

#[test]
fn roles_and_duplicates_stay_observable() {
    let roles = [
        ("urn:g", ConstantRole::NamedGraph),
        (RDF_TYPE, ConstantRole::Predicate),
        ("urn:C", ConstantRole::Class),
        ("urn:p", ConstantRole::Predicate),
        ("urn:C", ConstantRole::Object),
        ("urn:z", ConstantRole::Unresolved),
    ];
    assert_eq!(collect(ROLES_Q), owned(&roles));
    let literal = [
        ("urn:p", ConstantRole::Predicate),
        ("urn:dt", ConstantRole::LiteralDatatype),
    ];
    assert_eq!(collect(LIT_Q), owned(&literal));
    let duplicate = [
        ("urn:p", ConstantRole::Predicate),
        ("urn:p", ConstantRole::Predicate),
    ];
    assert_eq!(collect(DUP_Q), owned(&duplicate));
}

#[test]
fn refused_forms_name_their_rule_before_any_callback() {
    for (query, rule) in REFUSED.into_iter().zip(RULES) {
        for mode in [Mode::Cold, Mode::Uncached] {
            let binding = binding();
            let calls = Cell::new(0);
            let result = admitted_on(mode, &binding, query, scripted(&calls, None, OK));
            let error = result.unwrap_err();
            assert_eq!(refused_rule(&error), Some(rule), "{query}");
            assert_eq!(calls.get(), 0, "{query}");
            assert_eq!(binding.cache_len(), 0);
        }
    }
}

#[test]
fn update_is_refused_as_form_not_admitted_without_callback() {
    for mode in [Mode::Cold, Mode::Uncached] {
        let binding = binding();
        let calls = Cell::new(0);
        let result = admitted_on(mode, &binding, UPDATE_Q, scripted(&calls, None, OK));
        let error = result.unwrap_err();
        assert_eq!(refused_rule(&error), Some(ShapeRule::FormNotAdmitted));
        assert_eq!(calls.get(), 0);
        assert_eq!(binding.cache_len(), 0);
    }
}

#[test]
fn callback_refusal_is_typed_and_leaves_cache_unchanged() {
    for mode in [Mode::Cold, Mode::Uncached] {
        let binding = binding();
        let calls = Cell::new(0);
        let result = admitted_on(mode, &binding, VALUES_Q, scripted(&calls, None, DENY));
        assert!(is_coverage_refusal(&result.unwrap_err()));
        assert_eq!(calls.get(), 1);
        assert_eq!(binding.cache_len(), 0);
    }
}

#[test]
fn warm_cache_never_caches_callback_authority() {
    let binding = binding();
    let first = admitted_on(Mode::Cold, &binding, VALUES_Q, allow).unwrap();
    assert_eq!(binding.cache_len(), 1);
    let calls = Cell::new(0);
    let result = admitted_on(Mode::Warm, &binding, VALUES_Q, scripted(&calls, None, DENY));
    assert!(is_coverage_refusal(&result.unwrap_err()));
    assert_eq!(calls.get(), 1);
    assert_eq!(binding.cache_len(), 1);
    let again = admitted_on(Mode::Warm, &binding, VALUES_Q, allow).unwrap();
    assert!(Arc::ptr_eq(&first, &again));
}

#[test]
fn admission_work_is_default_work_plus_screen_work_at_exact_budget() {
    let parsed = crate::parse_query(VALUES_Q).unwrap();
    let (screened, screen) = measure(|control| admit_parsed(&parsed, control, allow));
    screened.unwrap();
    assert!(screen > 0);
    for mode in MODES {
        let (plan, base) = measure(|control| default_run(mode, &prepared(mode), control));
        plan.unwrap();
        let total = base + screen;
        let exact = budget(total);
        let result = admitted_run(mode, &prepared(mode), &exact, allow);
        result.unwrap();
        assert_eq!(work(&exact), total);
        assert_eq!(exact.terminal(), None);
        let short = budget(total - 1);
        let result = admitted_run(mode, &prepared(mode), &short, allow);
        assert_eq!(control_cause(&result.unwrap_err()), Some(EXCEEDED));
        assert_eq!(short.terminal(), Some(EXCEEDED));
    }
}

#[test]
fn cancelled_control_stops_before_callback() {
    for mode in MODES {
        let binding = prepared(mode);
        let entries = binding.cache_len();
        let control = budget(u64::MAX);
        control.terminate(CANCELLED);
        let calls = Cell::new(0);
        let result = admitted_run(mode, &binding, &control, scripted(&calls, None, OK));
        assert_eq!(control_cause(&result.unwrap_err()), Some(CANCELLED));
        assert_eq!(calls.get(), 0);
        assert_eq!(work(&control), 0);
        assert_eq!(binding.cache_len(), entries);
    }
}

#[test]
fn termination_raised_during_the_callback_is_sticky() {
    for cause in [CANCELLED, DEADLINE] {
        let binding = binding();
        let control = budget(u64::MAX);
        let calls = Cell::new(0);
        let stop = Some((&control, cause));
        let result = admitted_run(Mode::Cold, &binding, &control, scripted(&calls, stop, OK));
        assert_eq!(control_cause(&result.unwrap_err()), Some(cause));
        assert_eq!(calls.get(), 1);
        assert_eq!(binding.cache_len(), 0);
        assert_eq!(control.terminal(), Some(cause));
        let later = default_run(Mode::Cold, &binding, &control);
        assert_eq!(later.unwrap_err().to_string(), cause.to_string());
        assert_eq!(control.checkpoint(), Err(cause));
    }
}

#[test]
fn callback_control_failure_is_not_flattened() {
    for mode in [Mode::Cold, Mode::Uncached] {
        let binding = binding();
        let control = budget(u64::MAX);
        let calls = Cell::new(0);
        let failure = Err(ConstantCoverageError::Control(DEADLINE));
        let result = admitted_run(mode, &binding, &control, scripted(&calls, None, failure));
        let error = result.unwrap_err();
        assert!(!is_coverage_refusal(&error));
        assert_eq!(control_cause(&error), Some(DEADLINE));
        assert_eq!(control.terminal(), Some(DEADLINE));
        assert_eq!(binding.cache_len(), 0);
    }
}

#[test]
fn default_path_is_unchanged_after_admission() {
    let admitted = binding();
    let raw = binding();
    let first = admitted_run(Mode::Cold, &admitted, FREE, allow).unwrap();
    raw.compile_shared(VALUES_Q).unwrap();
    let (hit, admitted_work) = measure(|control| default_run(Mode::Warm, &admitted, control));
    let (baseline, raw_work) = measure(|control| default_run(Mode::Warm, &raw, control));
    assert!(raw_work > 0);
    assert_eq!(admitted_work, raw_work);
    baseline.unwrap();
    let hit = hit.unwrap();
    assert!(Arc::ptr_eq(&first, &hit));
}

#[test]
fn failures_render_without_query_input() {
    for query in [SECRET_Q, SECRET_SERVICE] {
        let result = admitted_on(Mode::Cold, &binding(), query, deny);
        let error = result.unwrap_err();
        let rendered = format!("{error} {error:?}");
        assert!(!rendered.contains("secret"), "{rendered}");
        assert!(!rendered.contains("hidden"), "{rendered}");
    }
}

#[test]
fn each_admission_call_parses_exactly_once() {
    isolated(|| {
        let binding = binding();
        let mut counts = Vec::new();
        for mode in MODES {
            let (result, parses) = parse_spans(|| admitted_run(mode, &binding, FREE, allow));
            result.unwrap();
            counts.push(parses);
        }
        let (result, parses) = parse_spans(|| admitted_run(Mode::Cold, &binding, FREE, deny));
        assert!(result.is_err());
        counts.push(parses);
        assert_eq!(counts, [1, 1, 1, 1]);
    });
}
