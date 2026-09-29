use std::sync::atomic::{AtomicU64, Ordering};

use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits, UncontrolledQueryControl};
use spargebra::SparqlParser;

use super::*;

type Seen = Vec<(String, ConstantRole)>;

const O: &str = "http://e/o";
const P: &str = "http://e/p";

const BGP: &str = "SELECT * WHERE { <http://e/s> <http://e/p> <http://e/o> }";
const TYPED: &str = "ASK { ?x a <http://e/C> . ?x <http://e/p> <http://e/C> }";
const MIXED: &str = "ASK { GRAPH <http://e/g> { ?s a <http://e/C> } FILTER(?s != <http://e/x>) }";
const VALUES: &str = "ASK { VALUES ?x { <http://e/a> 1 UNDEF 'v'^^<http://e/dt> } }";
const PATH_ALT: &str = "ASK { <http://e/a> (<http://e/p>|<http://e/q>/<http://e/r>)* ?o }";
const SEC: &str = "ASK { <http://secret.example/s> <http://secret.example/p> 'hidden-literal' }";
const PARITY: [&str; 5] = [BGP, TYPED, MIXED, VALUES, PATH_ALT];

const OVER: QueryControlError = QueryControlError::CompilerWorkExceeded;
const EXCEEDED: ShapeRefusal = ShapeRefusal::Control(OVER);
const CANCELLED: ShapeRefusal = ShapeRefusal::Control(QueryControlError::Cancelled);
const REJECTED: ShapeRefusal = ShapeRefusal::ConstantRejected(ConstantRejection);
const DEPTH: usize = 50_000;

struct CancelOnConsume {
    budget: QueryBudget,
    remaining: AtomicU64,
}

impl QueryControl for CancelOnConsume {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        self.budget.checkpoint()
    }

    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), QueryControlError> {
        if self.remaining.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.budget.terminate(QueryControlError::Cancelled);
        }
        self.budget.consume(charge, amount)
    }

    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.budget.terminate(reason)
    }
}

fn accept(_: ConstantOccurrence<'_>) -> Result<(), ConstantRejection> {
    Ok(())
}

fn reject(_: ConstantOccurrence<'_>) -> Result<(), ConstantRejection> {
    Err(ConstantRejection)
}

fn parse(sparql: &str) -> Query {
    SparqlParser::new().parse_query(sparql).unwrap()
}

fn budget(limit: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(limit, u64::MAX, u64::MAX, u64::MAX))
}

fn compiler_work(control: &QueryBudget) -> u64 {
    control.consumed(QueryCharge::CompilerWork)
}

fn collect(
    query: &Query,
    control: &dyn QueryControl,
    seen: &mut Seen,
) -> Result<StructuralOnlyScreen, ShapeRefusal> {
    visit_parsed_query_constants(query, control, |found| {
        seen.push((found.iri().to_owned(), found.role()));
        Ok(())
    })
}

fn charged(query: &Query) -> u64 {
    let control = budget(u64::MAX);
    let mut seen = Seen::new();
    let screened = collect(query, &control, &mut seen).unwrap();
    screened.charged_compiler_work()
}

fn assert_redacted(refusal: ShapeRefusal) {
    let rendered = format!("{refusal} {refusal:?}");
    for secret in ["secret.example", "hidden-literal"] {
        assert!(!rendered.contains(secret), "leaked {secret}: {rendered}");
    }
}

fn wrap_distinct(mut pattern: GraphPattern, depth: usize) -> GraphPattern {
    for _ in 0..depth {
        pattern = GraphPattern::Distinct {
            inner: Box::new(pattern),
        };
    }
    pattern
}

fn dismantle(mut pattern: GraphPattern) {
    while let GraphPattern::Distinct { inner } = &mut pattern {
        let next = std::mem::take(&mut **inner);
        pattern = next;
    }
}

fn walk_deep_nesting() {
    let Query::Select { pattern, .. } = parse(BGP) else {
        panic!("select expected");
    };
    let query = Query::Select {
        dataset: None,
        pattern: wrap_distinct(pattern, DEPTH),
        base_iri: None,
    };
    let mut seen = Seen::new();
    collect(&query, &UncontrolledQueryControl, &mut seen).unwrap();
    assert_eq!(seen.len(), 3);
    if let Query::Select { pattern, .. } = query {
        dismantle(pattern);
    }
}

#[test]
fn visitor_charges_exactly_like_the_screen() {
    for sparql in PARITY {
        let query = parse(sparql);
        let plain = budget(u64::MAX);
        let expected = screen_parsed_query_structure(&query, &plain).unwrap();
        let walked = budget(u64::MAX);
        let found = visit_parsed_query_constants(&query, &walked, accept).unwrap();
        assert_eq!(found, expected, "{sparql}");
        assert_eq!(compiler_work(&walked), compiler_work(&plain), "{sparql}");
    }
}

#[test]
fn callback_rejection_stops_traversal_without_later_calls() {
    let query = parse(BGP);
    let control = budget(u64::MAX);
    let mut calls = 0;
    let result = visit_parsed_query_constants(&query, &control, |_| {
        calls += 1;
        if calls == 2 {
            return Err(ConstantRejection);
        }
        Ok(())
    });
    assert_eq!(result.unwrap_err(), REJECTED);
    assert_eq!(calls, 2);
    assert_eq!(control.terminal(), None);
}

#[test]
fn exact_budget_reports_all_and_short_budget_withholds_last_charge() {
    let query = parse(BGP);
    let full = charged(&query);
    let exact = budget(full);
    let mut seen = Seen::new();
    collect(&query, &exact, &mut seen).unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(compiler_work(&exact), full);
    assert_eq!(exact.terminal(), None);

    let short = budget(full - 1);
    let mut partial = Seen::new();
    let result = collect(&query, &short, &mut partial);
    assert_eq!(result.unwrap_err(), EXCEEDED);
    assert_eq!(short.terminal(), Some(OVER));
    assert_eq!(partial.len(), 2);
    assert_eq!(partial[0], (O.to_owned(), ConstantRole::Object));
    assert_eq!(partial[1], (P.to_owned(), ConstantRole::Predicate));

    let again = collect(&query, &short, &mut partial);
    assert_eq!(again.unwrap_err(), EXCEEDED);
    assert_eq!(partial.len(), 2);
}

#[test]
fn every_smaller_budget_fails_before_reporting_everything() {
    let query = parse(BGP);
    let full = charged(&query);
    for limit in 0..full {
        let control = budget(limit);
        let mut seen = Seen::new();
        let result = collect(&query, &control, &mut seen);
        assert_eq!(result.unwrap_err(), EXCEEDED, "limit {limit}");
        assert!(seen.len() < 3, "limit {limit}");
    }
}

#[test]
fn cancellation_before_any_callback_is_typed_and_sticky() {
    let query = parse(BGP);
    let control = budget(u64::MAX);
    control.terminate(QueryControlError::Cancelled);
    let mut seen = Seen::new();
    let result = collect(&query, &control, &mut seen);
    assert_eq!(result.unwrap_err(), CANCELLED);
    assert!(seen.is_empty());
    assert_eq!(compiler_work(&control), 0);
}

#[test]
fn cancellation_during_callback_stops_further_calls() {
    let query = parse(BGP);
    let control = budget(u64::MAX);
    let mut calls = 0;
    let result = visit_parsed_query_constants(&query, &control, |_| {
        calls += 1;
        control.terminate(QueryControlError::Cancelled);
        Ok(())
    });
    assert_eq!(result.unwrap_err(), CANCELLED);
    assert_eq!(calls, 1);
}

#[test]
fn cancellation_sweep_never_calls_back_after_termination() {
    let query = parse(MIXED);
    let full = charged(&query);
    let mut cancelled = 0;
    for at in 1..=full {
        let control = CancelOnConsume {
            budget: budget(u64::MAX),
            remaining: AtomicU64::new(at),
        };
        let mut late = 0;
        let result = visit_parsed_query_constants(&query, &control, |_| {
            if control.budget.terminal().is_some() {
                late += 1;
            }
            Ok(())
        });
        assert_eq!(late, 0, "at {at}");
        if let Err(refusal) = result {
            assert_eq!(refusal, CANCELLED, "at {at}");
            cancelled += 1;
        }
    }
    assert!(cancelled > 0);
}

#[test]
fn failures_render_without_query_input() {
    let query = parse(SEC);
    let rejected = visit_parsed_query_constants(&query, &UncontrolledQueryControl, reject);
    assert_redacted(rejected.unwrap_err());
    let short = budget(1);
    let exceeded = visit_parsed_query_constants(&query, &short, accept);
    assert_redacted(exceeded.unwrap_err());
}

#[test]
fn input_query_is_unchanged() {
    for sparql in PARITY {
        let query = parse(sparql);
        let before = query.clone();
        let mut seen = Seen::new();
        collect(&query, &UncontrolledQueryControl, &mut seen).unwrap();
        assert_eq!(query, before, "{sparql}");
    }
}

#[test]
fn deep_nesting_is_visited_iteratively() {
    let worker = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(walk_deep_nesting)
        .unwrap();
    worker.join().unwrap();
}
