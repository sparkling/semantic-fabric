use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError as Stop, QueryLimits,
    UncontrolledQueryControl,
};
use sf_core::{SourceId, Term};
use sf_sparql::cache::generated::{GeneratedQueryRefusal, ShapeRule};
use sf_sparql::{exec, Epoch, Error, Plan, PlanForm};

use super::GeneratedRuntimeError as E;
use crate::binding::{BindingMismatch, BoundPlan, RuntimeBinding};
use crate::semantic_admission::{MappingOrigin, ValidatedMapping};
use crate::{Backend, IntrospectedSource};

pub(super) const SELECT: &str = "SELECT ?n WHERE { ?p a <http://ex/Person> ; <http://ex/name> ?n }";
pub(super) const ASK: &str = "ASK { ?p <http://ex/name> ?n }";
pub(super) const UNMAPPED: &str = "ASK { ?p <http://ex/unknown> ?n }";
pub(super) const EXTRA: &str = "http://ex/extra";
pub(super) const FREE: &UncontrolledQueryControl = &UncontrolledQueryControl;

const PRESENT: &str = "ASK { <http://ex/person/1> <http://ex/name> ?n }";
const ABSENT: &str = "ASK { <http://ex/person/42> <http://ex/name> ?n }";
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

const PREFIXES: &str = "@prefix rr: <http://www.w3.org/ns/r2rml#> .\n\
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n";
const HEAD: &str = "<#p> a rr:TriplesMap; rr:logicalTable [ rr:tableName 'people' ];\n \
    rr:subjectMap [ rr:template 'http://ex/person/{id}'; rr:class <http://ex/Person>";
const BODY: &str = " ];\n rr:predicateObjectMap [ rr:predicate <http://ex/name>; \
    rr:objectMap [ rr:column 'name' ] ]";
const AGE_POM: &str = ";\n rr:predicateObjectMap [ rr:predicate <http://ex/age>; \
    rr:objectMap [ rr:column 'age'; rr:datatype xsd:integer ] ]";
const MULTI: &str = ".\n<#multi> a rr:TriplesMap; rr:logicalTable [ rr:tableName 'people' ];\n \
    rr:subjectMap [ rr:template 'http://ex/multi/{id}/{name}' ].\n";

pub(super) fn mapping_text(age: bool, graph: bool) -> String {
    let graph = if graph {
        "; rr:graphMap [ rr:constant <http://ex/g> ]"
    } else {
        ""
    };
    let age = if age { AGE_POM } else { "" };
    format!("{PREFIXES}{HEAD}{graph}{BODY}{age}{MULTI}")
}

pub(super) fn bind_at(
    index: usize,
    mapping: &str,
    origin: MappingOrigin,
    extra: &[&str],
) -> RuntimeBinding {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE people(id INTEGER, name TEXT, age INTEGER); \
             INSERT INTO people VALUES (1, 'Alice', 30);",
        )
        .unwrap();
    let source = IntrospectedSource::observe_sqlite(Backend::sqlite(connection)).unwrap();
    let mut properties = vec!["http://ex/name", "http://ex/age"];
    properties.extend_from_slice(extra);
    let ontology = crate::test_support::ontology(&["http://ex/Person"], &properties);
    let mapping =
        sf_mapping::parse_r2rml_for_source(mapping, SourceId::new(index).unwrap()).unwrap();
    let validated = ValidatedMapping::validate(mapping, origin, &ontology, &source).unwrap();
    RuntimeBinding::new(source, validated, ontology.tbox().clone(), Epoch::default())
}

pub(super) fn bind(mapping: &str, origin: MappingOrigin, extra: &[&str]) -> RuntimeBinding {
    bind_at(0, mapping, origin, extra)
}

pub(super) fn bind_default() -> RuntimeBinding {
    bind(&mapping_text(true, false), MappingOrigin::Authored, &[])
}

/// Consume a bound plan through its owning binding's execution pair and return
/// the fixture's SQLite connection with the ownership-checked plan.
fn executable(
    binding: &RuntimeBinding,
    bound: BoundPlan,
) -> (Arc<Mutex<rusqlite::Connection>>, Arc<Plan>) {
    let prepared = binding.prepare_execution(bound).unwrap();
    let (source, identity, backend, verified, plan) = prepared.into_parts();
    assert_eq!(source, binding.source_id());
    assert!(identity.ptr_eq(&binding.binding_identity));
    assert!(!verified);
    let Backend::Sqlite(pool) = backend else {
        panic!("the fixture binding owns a SQLite backend");
    };
    (pool.pick(), plan)
}

fn is_alice(term: &Option<Term>) -> bool {
    let Some(Term::Literal(literal)) = term else {
        return false;
    };
    literal.value() == "Alice"
        && literal.language().is_none()
        && literal.datatype().as_str() == XSD_STRING
}

/// Execute through the existing SQLite executor and assert the fixture answer:
/// ASK is true and SELECT binds exactly Alice.
pub(super) fn assert_answers(binding: &RuntimeBinding, query: &str, bound: BoundPlan) {
    let (connection, plan) = executable(binding, bound);
    let connection = connection.lock().unwrap();
    match &plan.form {
        PlanForm::Ask => {
            assert_eq!(query, ASK);
            assert!(exec::ask(&plan, &connection).unwrap());
        }
        PlanForm::Select { vars } => {
            assert_eq!(query, SELECT);
            assert_eq!(vars.len(), 1);
            let mut rows = Vec::new();
            exec::select_each(&plan, &connection, |row| {
                rows.push(row.to_vec());
                Ok(())
            })
            .unwrap();
            assert_eq!(rows.len(), 1, "{rows:?}");
            assert_eq!(rows[0].len(), 1, "{rows:?}");
            assert!(is_alice(&rows[0][0]), "{rows:?}");
        }
        _ => panic!("generated plans are SELECT or ASK"),
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Cold,
    Warm,
    Uncached,
}

const MODES: [Mode; 3] = [Mode::Cold, Mode::Warm, Mode::Uncached];

fn prepared(mode: Mode, query: &str) -> RuntimeBinding {
    let binding = bind_default();
    if matches!(mode, Mode::Warm) {
        binding.compile(query, FREE).unwrap();
    }
    binding
}

fn run(
    mode: Mode,
    binding: &RuntimeBinding,
    query: &str,
    control: &dyn QueryControl,
) -> Result<Arc<Plan>, E> {
    match mode {
        Mode::Uncached => binding.preflight_generated(query, control),
        _ => binding
            .compile_generated(query, control)
            .map(|compiled| compiled.plan.plan),
    }
}

fn err<T>(result: Result<T, E>) -> E {
    match result {
        Err(error) => error,
        Ok(_) => panic!("expected a generated-admission failure"),
    }
}

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

fn work(control: &QueryBudget) -> u64 {
    control.consumed(QueryCharge::CompilerWork)
}

fn refused<T>(result: &Result<T, E>) -> bool {
    matches!(result, Err(E::Refused(_)))
}

fn coverage_refusal<T>(result: &Result<T, E>) -> bool {
    matches!(
        result,
        Err(E::Refused(GeneratedQueryRefusal::CoverageRefused))
    )
}

const ROLES: &[(&str, bool)] = &[
    ("ASK { <http://ex/person/42> <http://ex/name> ?n }", true),
    ("ASK { <http://ex/other/42> <http://ex/name> ?n }", false),
    ("ASK { <http://ex/multi/1/2> <http://ex/name> ?n }", false),
    ("ASK { <http://ex/name> <http://ex/name> ?n }", false),
    ("ASK { ?s <http://ex/name> <http://ex/person/9> }", false),
    ("ASK { ?s <http://ex/unknown> ?o }", false),
    ("ASK { ?s <http://ex/name> ?o }", true),
    ("ASK { ?s a <http://ex/Person> }", true),
    ("ASK { ?s a <http://ex/Other> }", false),
    ("ASK { ?s <http://ex/age> 5 }", true),
    ("ASK { ?s <http://ex/age> '5'^^<http://ex/dt> }", false),
    (
        "ASK { ?s <http://ex/name> ?n FILTER(?s = <http://ex/nowhere>) }",
        false,
    ),
];

struct Trip {
    budget: QueryBudget,
    calls: AtomicU64,
    at: u64,
    reason: Stop,
}

impl Trip {
    fn new(at: u64, reason: Stop) -> Self {
        Self {
            budget: budget(u64::MAX),
            calls: AtomicU64::new(0),
            at,
            reason,
        }
    }
}

impl QueryControl for Trip {
    fn checkpoint(&self) -> Result<(), Stop> {
        self.budget.checkpoint()
    }

    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), Stop> {
        if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.at {
            self.budget.terminate(self.reason);
        }
        self.budget.consume(charge, amount)
    }

    fn terminate(&self, reason: Stop) -> Stop {
        self.budget.terminate(reason)
    }
}

#[test]
fn select_and_ask_are_admitted_on_every_path_with_usable_bound_plans() {
    for query in [SELECT, ASK] {
        let binding = bind_default();
        let preflight = binding.preflight_generated(query, FREE).unwrap();
        let cold = binding.compile_generated(query, FREE).unwrap();
        assert!(!Arc::ptr_eq(&preflight, &cold.plan.plan));
        let warm = binding.compile_generated(query, FREE).unwrap();
        assert!(Arc::ptr_eq(&cold.plan.plan, &warm.plan.plan));
        let raw = binding.compile(query, FREE).unwrap();
        assert!(Arc::ptr_eq(&cold.plan.plan, &raw.plan));
        assert_eq!(cold.identity, warm.identity);
        assert!(cold.identity.wire().starts_with("sfgp1:"));
        assert_eq!(matches!(cold.plan.plan().form, PlanForm::Ask), query == ASK);
        assert!(cold.plan.security.is_none());
        assert!(binding.prepare_execution(cold.plan).is_ok());
    }
}

#[test]
fn cold_and_warm_generated_plans_execute_against_the_sqlite_fixture() {
    for query in [SELECT, ASK] {
        let binding = bind_default();
        let cold = binding.compile_generated(query, FREE).unwrap();
        let warm = binding.compile_generated(query, FREE).unwrap();
        assert!(Arc::ptr_eq(&cold.plan.plan, &warm.plan.plan));
        assert_eq!(cold.identity, warm.identity);
        assert!(cold.plan.security.is_none());
        assert_answers(&binding, query, cold.plan);
        assert_answers(&binding, query, warm.plan);
    }
}

#[test]
fn generated_asks_report_actual_rows_not_coverage() {
    let binding = bind_default();
    for (query, expected) in [(PRESENT, true), (ABSENT, false)] {
        let compiled = binding.compile_generated(query, FREE).unwrap();
        let (connection, plan) = executable(&binding, compiled.plan);
        let answer = exec::ask(&plan, &connection.lock().unwrap()).unwrap();
        assert_eq!(answer, expected, "{query}");
    }
}

#[test]
fn structural_forms_are_refused_by_name() {
    let cases = [
        (
            "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
            ShapeRule::ConstructForm,
        ),
        ("DESCRIBE <http://ex/person/1>", ShapeRule::DescribeForm),
        (
            "SELECT * FROM <http://ex/g> WHERE { ?s ?p ?o }",
            ShapeRule::DatasetClause,
        ),
    ];
    for (query, rule) in cases {
        for mode in MODES {
            let binding = prepared(mode, SELECT);
            let error = err(run(mode, &binding, query, FREE));
            let named = matches!(
                error,
                E::Refused(GeneratedQueryRefusal::Rule(found)) if found == rule
            );
            assert!(named, "{query}");
        }
    }
}

#[test]
fn every_constant_role_maps_to_the_sealed_coverage() {
    let binding = bind_default();
    for (query, covered) in ROLES {
        let result = binding.preflight_generated(query, FREE);
        assert_eq!(refused(&result), !covered, "{query}");
        if !covered {
            assert!(coverage_refusal(&result), "{query}");
        }
    }
}

#[test]
fn named_graph_role_is_covered_only_by_a_mapped_graph() {
    let binding = bind(&mapping_text(true, true), MappingOrigin::Authored, &[]);
    let mapped = "ASK { GRAPH <http://ex/g> { ?s <http://ex/name> ?n } }";
    let other = "ASK { GRAPH <http://ex/h> { ?s <http://ex/name> ?n } }";
    assert!(!refused(&binding.preflight_generated(mapped, FREE)));
    assert!(coverage_refusal(&binding.preflight_generated(other, FREE)));
}

#[test]
fn refusals_render_without_query_or_mapping_text() {
    let queries = [
        UNMAPPED,
        "ASK { <http://ex/multi/1/2> <http://ex/name> ?n }",
        "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
        "ASK { ?s ?p ?o FILTER(?s = <http://ex/nowhere>) }",
    ];
    for query in queries {
        for mode in MODES {
            let binding = prepared(mode, SELECT);
            let error = err(run(mode, &binding, query, FREE));
            let text = format!("{error} {error:?}");
            assert!(!text.contains("http://ex"), "{text}");
            assert!(
                !text.contains("unknown") && !text.contains("nowhere"),
                "{text}"
            );
        }
    }
}

#[test]
fn default_narrowing_is_unchanged_and_a_warm_default_cache_never_bypasses_refusal() {
    let binding = bind_default();
    assert!(binding.compile(UNMAPPED, FREE).is_ok());
    for mode in [Mode::Cold, Mode::Uncached] {
        assert!(coverage_refusal(&run(mode, &binding, UNMAPPED, FREE)));
    }
    assert!(binding.compile(UNMAPPED, FREE).is_ok());
    let generated = binding.compile_generated(SELECT, FREE).unwrap();
    let raw = binding.compile(SELECT, FREE).unwrap();
    assert!(Arc::ptr_eq(&generated.plan.plan, &raw.plan));
}

#[test]
fn admission_work_is_exact_and_fails_one_short() {
    for mode in MODES {
        let probe = budget(u64::MAX);
        run(mode, &prepared(mode, SELECT), SELECT, &probe).unwrap();
        let total = work(&probe);
        assert!(total > 0);
        let exact = budget(total);
        run(mode, &prepared(mode, SELECT), SELECT, &exact).unwrap();
        assert_eq!(work(&exact), total);
        assert_eq!(exact.terminal(), None);
        let short = budget(total - 1);
        let result = run(mode, &prepared(mode, SELECT), SELECT, &short);
        let exceeded = matches!(
            result,
            Err(E::Compiler(Error::QueryControl(Stop::CompilerWorkExceeded)))
        );
        assert!(exceeded);
        assert_eq!(short.terminal(), Some(Stop::CompilerWorkExceeded));
    }
}

#[test]
fn sticky_controls_stay_controls_and_never_become_coverage() {
    for reason in [Stop::Cancelled, Stop::DeadlineExceeded] {
        for mode in MODES {
            let binding = prepared(mode, SELECT);
            let control = budget(u64::MAX);
            control.terminate(reason);
            let result = run(mode, &binding, SELECT, &control);
            let stopped =
                matches!(result, Err(E::Compiler(Error::QueryControl(found))) if found == reason);
            assert!(stopped);
            assert_eq!(work(&control), 0);
        }
    }
    let binding = bind_default();
    let probe = Trip::new(u64::MAX, Stop::Cancelled);
    binding.preflight_generated(SELECT, &probe).unwrap();
    let total = probe.calls.load(Ordering::SeqCst);
    assert!(total > 1);
    for reason in [Stop::Cancelled, Stop::DeadlineExceeded] {
        for at in 1..=total {
            let trip = Trip::new(at, reason);
            let result = binding.preflight_generated(SELECT, &trip);
            let stopped =
                matches!(result, Err(E::Compiler(Error::QueryControl(found))) if found == reason);
            assert!(stopped, "{reason:?} at {at}");
            assert_eq!(trip.checkpoint(), Err(reason));
        }
    }
}

#[test]
fn foreign_receipts_are_rejected_for_every_identity_dimension() {
    let base = mapping_text(true, false);
    let others = [
        bind(&mapping_text(false, false), MappingOrigin::Authored, &[]),
        bind(&base, MappingOrigin::Authored, &[EXTRA]),
        bind(&base, MappingOrigin::Direct, &[]),
        bind_at(1, &base, MappingOrigin::Authored, &[]),
    ];
    let intact = bind_default();
    assert!(intact.generated.coverage(intact.compiler()).is_ok());
    for other in others {
        let mut binding = bind_default();
        binding.generated = other.generated;
        assert!(binding.generated.coverage(binding.compiler()).is_err());
        let compiled = binding.compile_generated(ASK, FREE);
        assert!(matches!(compiled, Err(E::ReceiptMismatch(_))));
        let preflight = binding.preflight_generated(ASK, FREE);
        assert!(matches!(preflight, Err(E::ReceiptMismatch(_))));
    }
}

#[test]
fn plans_from_equal_content_bindings_are_not_interchangeable() {
    let first = bind_default();
    let second = bind_default();
    assert_eq!(first.scope(), second.scope());
    let plan = first.compile_generated(SELECT, FREE).unwrap().plan;
    assert!(matches!(
        second.prepare_execution(plan),
        Err(BindingMismatch)
    ));
    let again = first.compile_generated(SELECT, FREE).unwrap().plan;
    assert!(first.prepare_execution(again).is_ok());
}
