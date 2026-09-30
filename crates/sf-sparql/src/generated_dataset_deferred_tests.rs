use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use sf_core::query_control::{QueryBudget, QueryCharge, QueryControl, QueryControlError};

use crate::cache::generated::tests::{isolated, parse_spans};
use crate::cache::generated::{
    ConstantCoverageError, ConstantOccurrence, DatasetGraphAllowlist, GeneratedDatasetError,
};
use crate::{CompilerBinding, Error, Plan};

use super::test_support::*;

#[test]
fn dataset_deferred_refusal_authorizes_requested_named_graph_tables() {
    for cached in [false, true] {
        for (graph, other, table) in [(A, B, TABLE_A), (B, A, TABLE_B)] {
            let query = SELECT.replace(A, graph);
            let expected = scoped(&fixture(), &query);
            assert_eq!(tables(&expected), [table], "{query}");
            let excluded = DatasetGraphAllowlist::new([other]).unwrap();
            let cases = [
                (allow(), DENY, OK, R::GraphNotAdmitted),
                (allow(), OK, DENY, R::ConstantCoverage),
                (excluded, OK, OK, R::GraphNotAdmitted),
            ];
            for (list, graph_verdict, constant_verdict, rule) in cases {
                let binding = fixture();
                let check = move |_: &str| graph_verdict;
                let constant = move |_: ConstantOccurrence<'_>| constant_verdict;
                let outcome = deferred(cached, &binding, &query, &list, FREE, check, constant);
                let (refusal, plan) = refused(outcome.unwrap());
                assert_eq!(refusal, rule, "{query}");
                let plan = plan.expect("requested graph requires row authorization");
                assert_eq!(tables(&plan), [table], "unrelated graph authorized");
                assert_eq!(format!("{plan:?}"), format!("{expected:?}"), "{query}");
                assert_eq!(binding.cache_len(), 0, "{query}");
            }
        }
    }
}

#[test]
fn dataset_deferred_admitted_matches_existing_cold_warm_and_uncached() {
    for query in [SELECT, ASK, VALUES] {
        let (existing, mine) = (fixture(), fixture());
        let mut plans = Vec::new();
        for cached in [false, true, true] {
            let (paid, spent) = (budget(u64::MAX), budget(u64::MAX));
            let expected = ordinary(
                cached,
                &existing,
                query,
                &allow(),
                &paid,
                pass_graph,
                pass_constant,
            );
            let expected = expected.unwrap();
            let plan = admitted(open(cached, &mine, query, &spent).unwrap());
            assert_eq!(format!("{plan:?}"), format!("{expected:?}"), "{query}");
            assert!(work(&paid) > 0, "{query}");
            assert_eq!(work(&spent), work(&paid), "{query}");
            plans.push(plan);
        }
        assert!(!Arc::ptr_eq(&plans[0], &plans[1]));
        assert!(Arc::ptr_eq(&plans[1], &plans[2]));
        assert_eq!((mine.cache_len(), existing.cache_len()), (1, 1));
        let shared = ordinary(
            true,
            &mine,
            query,
            &allow(),
            FREE,
            pass_graph,
            pass_constant,
        );
        assert!(Arc::ptr_eq(&plans[1], &shared.unwrap()));
        assert_eq!(mine.cache_len(), 1);
    }
}

#[test]
fn dataset_deferred_warm_entry_is_returned_only_after_fresh_checks() {
    let binding = fixture();
    let cold = admitted(open(true, &binding, SELECT, FREE).unwrap());
    let (graph_calls, constant_calls) = (Cell::new(0), Cell::new(0));
    let cases = [
        (DENY, OK, R::GraphNotAdmitted, (1, 0)),
        (OK, DENY, R::ConstantCoverage, (2, 1)),
    ];
    for (graph, constant, rule, calls) in cases {
        let graph = graphs(&graph_calls, graph);
        let constant = constants(&constant_calls, constant);
        let outcome = deferred(true, &binding, SELECT, &allow(), FREE, graph, constant);
        let (refusal, plan) = refused(outcome.unwrap());
        assert_eq!(refusal, rule);
        assert!(!Arc::ptr_eq(&plan.expect("authorization plan"), &cold));
        assert_eq!((graph_calls.get(), constant_calls.get()), calls);
        assert_eq!(binding.cache_len(), 1);
    }
    let graph = graphs(&graph_calls, OK);
    let constant = constants(&constant_calls, OK);
    let warm = deferred(true, &binding, SELECT, &allow(), FREE, graph, constant);
    assert!(Arc::ptr_eq(&cold, &admitted(warm.unwrap())));
    assert_eq!((graph_calls.get(), constant_calls.get()), (3, 2));
}

#[test]
fn dataset_deferred_denials_return_uncached_plans_and_never_warm_a_cache() {
    let fresh = fixture();
    let cold_cost = budget(u64::MAX);
    admitted(open(true, &fresh, SELECT, &cold_cost).unwrap());
    let expected = format!("{:?}", scoped(&fresh, SELECT));
    let (full, only_b) = (allow(), DatasetGraphAllowlist::new([B]).unwrap());
    let cases = [
        (&only_b, OK, OK, R::GraphNotAdmitted),
        (&full, DENY, OK, R::GraphNotAdmitted),
        (&full, OK, DENY, R::ConstantCoverage),
    ];
    for (list, graph, constant, rule) in cases {
        for cached in [true, false] {
            let binding = fixture();
            let mut plans = Vec::new();
            for _ in 0..3 {
                let graph = move |_: &str| graph;
                let constant = move |_: ConstantOccurrence<'_>| constant;
                let outcome = deferred(cached, &binding, SELECT, list, FREE, graph, constant);
                let (refusal, plan) = refused(outcome.unwrap());
                assert_eq!(refusal, rule);
                let plan = plan.expect("authorization plan");
                assert_eq!(format!("{plan:?}"), expected);
                assert_eq!(binding.cache_len(), 0);
                plans.push(plan);
            }
            assert!(!Arc::ptr_eq(&plans[0], &plans[1]));
            let control = budget(u64::MAX);
            admitted(open(true, &binding, SELECT, &control).unwrap());
            assert_eq!(work(&control), work(&cold_cost));
            assert_eq!(binding.cache_len(), 1);
        }
    }
}

#[test]
fn dataset_deferred_refusals_keep_existing_rules_before_callbacks() {
    let lowering = fixture();
    for (template, rule) in REFUSED {
        let query = at(template);
        let lowered = original(&lowering, &query);
        assert_eq!(lowered.is_some(), *rule != R::FormNotAdmitted, "{query}");
        for cached in [true, false] {
            let binding = fixture();
            let existing = ordinary(
                cached,
                &binding,
                &query,
                &allow(),
                FREE,
                pass_graph,
                pass_constant,
            );
            assert_eq!(rule_of(&existing.unwrap_err()), Some(*rule), "{query}");
            let (graph_calls, constant_calls) = (Cell::new(0), Cell::new(0));
            let graph = graphs(&graph_calls, OK);
            let constant = constants(&constant_calls, OK);
            let outcome = deferred(cached, &binding, &query, &allow(), FREE, graph, constant);
            assert_eq!((graph_calls.get(), constant_calls.get()), (0, 0), "{query}");
            assert_eq!(binding.cache_len(), 0, "{query}");
            match &lowered {
                // Syntax refusals and Unsupported lowering: no plan.
                None | Some(Err(Error::Unsupported(_))) => {
                    let (refusal, plan) = refused(outcome.unwrap());
                    assert_eq!(refusal, *rule, "{query}");
                    assert!(plan.is_none(), "{query}");
                }
                Some(Ok(expected)) => {
                    let (refusal, plan) = refused(outcome.unwrap());
                    assert_eq!(refusal, *rule, "{query}");
                    let plan = plan.expect("authorization plan");
                    assert_eq!(format!("{plan:?}"), format!("{expected:?}"), "{query}");
                }
                Some(Err(error)) => {
                    assert_eq!(
                        outcome.unwrap_err().to_string(),
                        error.to_string(),
                        "{query}"
                    )
                }
            }
        }
    }
}

/// Structural-refusal oracle: the outcome is exactly the ordinary uncached
/// lowering of the original as parsed. A supported lowering keeps its plan,
/// only `Unsupported` yields none, and any other failure keeps its type.
fn lowered_as_parsed(
    cached: bool,
    binding: &CompilerBinding,
    query: &str,
    rule: R,
) -> Option<Arc<Plan>> {
    let lowered = original(binding, query).expect("parsed original");
    let outcome = open(cached, binding, query, FREE);
    assert_eq!(binding.cache_len(), 0, "{query}");
    match lowered {
        Ok(expected) => {
            let (refusal, plan) = refused(outcome.unwrap());
            assert_eq!(refusal, rule, "{query}");
            let plan = plan.expect("supported lowering keeps its plan");
            assert_eq!(format!("{plan:?}"), format!("{expected:?}"), "{query}");
            Some(plan)
        }
        Err(Error::Unsupported(_)) => {
            let (refusal, plan) = refused(outcome.unwrap());
            assert_eq!(refusal, rule, "{query}");
            assert!(plan.is_none(), "{query}");
            None
        }
        Err(error) => {
            let kept = outcome.unwrap_err();
            let same = std::mem::discriminant(&kept) == std::mem::discriminant(&error);
            assert!(same, "{query}: {kept}");
            assert_eq!(kept.to_string(), error.to_string(), "{query}");
            None
        }
    }
}

#[test]
fn dataset_deferred_structural_refusals_lower_the_original_query() {
    let multiple = SELECT.replace(" WHERE", " FROM <http://ex/B> WHERE");
    let head = "CONSTRUCT { ?s <http://ex/p> ?o }";
    let form = SELECT.replace("SELECT ?o", head);
    let reference = multiple.replace("<http://ex/p>", "<http://ex/ref>");
    let stripped = format!("{:?}", scoped(&fixture(), SELECT));
    for cached in [true, false] {
        let plan = lowered_as_parsed(cached, &fixture(), &multiple, R::MultipleDefaultGraphs);
        // Never stripped or rewritten into the admissible single-graph query.
        if let Some(plan) = plan {
            assert_ne!(format!("{plan:?}"), stripped);
        }
        lowered_as_parsed(cached, &fixture(), &form, R::QueryForm);
        // Whatever ordinary lowering reports, a mapping failure included, is kept.
        lowered_as_parsed(cached, &broken(), &reference, R::MultipleDefaultGraphs);
    }
}

/// Parsing observes no control, so a stop raised while it runs is first seen
/// after the entry checkpoint has already passed.
struct StopAfterEntry {
    budget: QueryBudget,
    cause: QueryControlError,
    entered: AtomicBool,
}

impl QueryControl for StopAfterEntry {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        let entry = self.budget.checkpoint();
        if !self.entered.swap(true, Ordering::SeqCst) {
            self.budget.terminate(self.cause);
        }
        entry
    }

    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), QueryControlError> {
        self.budget.consume(charge, amount)
    }

    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.budget.terminate(reason)
    }
}

#[test]
fn dataset_deferred_control_outranks_malformed_input_and_denials() {
    for cached in [true, false] {
        for query in [SELECT, UPDATE, "not sparql", MISSING] {
            let binding = fixture();
            let calls = Cell::new(0);
            let control = budget(u64::MAX);
            control.terminate(CANCELLED);
            let (graph, constant) = (graphs(&calls, OK), constants(&calls, OK));
            let outcome = deferred(cached, &binding, query, &allow(), &control, graph, constant);
            assert_eq!(cause(&outcome.unwrap_err()), Some(CANCELLED), "{query}");
            assert_eq!((calls.get(), work(&control)), (0, 0), "{query}");
            for stop in [CANCELLED, DEADLINE] {
                let control = StopAfterEntry {
                    budget: budget(u64::MAX),
                    cause: stop,
                    entered: AtomicBool::new(false),
                };
                let (graph, constant) = (graphs(&calls, OK), constants(&calls, OK));
                let outcome =
                    deferred(cached, &binding, query, &allow(), &control, graph, constant);
                assert_eq!(cause(&outcome.unwrap_err()), Some(stop), "{query}");
                assert_eq!(control.budget.terminal(), Some(stop), "{query}");
                assert_eq!((calls.get(), work(&control.budget)), (0, 0), "{query}");
            }
            assert_eq!(binding.cache_len(), 0, "{query}");
        }
        for in_graph in [true, false] {
            for stop in [Some(CANCELLED), None] {
                let binding = fixture();
                let control = budget(u64::MAX);
                let verdict = || match stop {
                    Some(cause) => {
                        control.terminate(cause);
                        DENY
                    }
                    None => Err(ConstantCoverageError::Control(DEADLINE)),
                };
                let graph = |_: &str| if in_graph { verdict() } else { OK };
                let constant = |_: ConstantOccurrence<'_>| if in_graph { OK } else { verdict() };
                let outcome = deferred(
                    cached,
                    &binding,
                    SELECT,
                    &allow(),
                    &control,
                    graph,
                    constant,
                );
                let expected = stop.unwrap_or(DEADLINE);
                assert_eq!(cause(&outcome.unwrap_err()), Some(expected));
                assert_eq!(control.terminal(), Some(expected));
                assert_eq!(binding.cache_len(), 0);
            }
        }
    }
}

#[test]
fn dataset_deferred_lowering_failures_stay_typed_and_are_not_refusals() {
    for (graph, constant) in [(DENY, OK), (OK, DENY)] {
        let graph = move |_: &str| graph;
        let constant = move |_: ConstantOccurrence<'_>| constant;
        for cached in [true, false] {
            let screening = budget(u64::MAX);
            let existing = ordinary(
                cached,
                &fixture(),
                SELECT,
                &allow(),
                &screening,
                graph,
                constant,
            );
            assert!(rule_of(&existing.unwrap_err()).is_some());
            assert!(work(&screening) > 0);
            let binding = fixture();
            let control = budget(work(&screening));
            let outcome = deferred(
                cached,
                &binding,
                SELECT,
                &allow(),
                &control,
                graph,
                constant,
            );
            assert_eq!(cause(&outcome.unwrap_err()), Some(EXCEEDED));
            assert_eq!(control.terminal(), Some(EXCEEDED));
            assert_eq!(binding.cache_len(), 0);

            let binding = broken();
            let existing = ordinary(cached, &binding, REF, &allow(), FREE, graph, constant);
            assert!(rule_of(&existing.unwrap_err()).is_some());
            let outcome = deferred(cached, &binding, REF, &allow(), FREE, graph, constant);
            let error = outcome.unwrap_err();
            assert!(matches!(error, Error::Mapping(_)), "{error}");
            assert_eq!(binding.cache_len(), 0);
        }
    }
    for cached in [true, false] {
        let binding = broken();
        let admitted = ordinary(
            cached,
            &binding,
            REF,
            &allow(),
            FREE,
            pass_graph,
            pass_constant,
        );
        let error = admitted.unwrap_err();
        let mapping = matches!(error, GeneratedDatasetError::Compiler(Error::Mapping(_)));
        assert!(mapping, "{error}");
        assert_eq!(binding.cache_len(), 0);
    }
}

#[test]
fn dataset_deferred_invalid_or_mismatched_graph_never_becomes_admitted() {
    let secret = "SELECT ?o FROM <http://secret.example/g> WHERE { ?s <http://ex/p> ?o }";
    let multiple = "SELECT ?o FROM <http://ex/A> FROM <http://ex/B> WHERE { ?s <http://ex/p> ?o }";
    let repeated = "SELECT ?o FROM <http://ex/A> FROM <http://ex/A> WHERE { ?s <http://ex/p> ?o }";
    let (full, only_b) = (allow(), DatasetGraphAllowlist::new([B]).unwrap());
    let cases = [
        (secret, &full, R::GraphNotAdmitted),
        (SELECT, &only_b, R::GraphNotAdmitted),
        (multiple, &full, R::MultipleDefaultGraphs),
    ];
    for cached in [true, false] {
        let binding = fixture();
        for (query, list, rule) in cases {
            let outcome = deferred(
                cached,
                &binding,
                query,
                list,
                FREE,
                pass_graph,
                pass_constant,
            );
            let outcome = outcome.unwrap();
            let rendered = format!("{outcome:?}");
            assert!(!rendered.contains("secret"), "{rendered}");
            assert_eq!(refused(outcome).0, rule, "{query}");
        }
        let calls = Cell::new(0);
        let second = |_: &str| {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                DENY
            } else {
                OK
            }
        };
        let outcome = deferred(
            cached,
            &binding,
            repeated,
            &full,
            FREE,
            second,
            pass_constant,
        );
        assert_eq!(refused(outcome.unwrap()).0, R::GraphNotAdmitted);
        assert_eq!(calls.get(), 2);
        assert_eq!(binding.cache_len(), 0);
        let outcome = open(cached, &binding, SELECT, FREE).unwrap();
        assert_eq!(format!("{outcome:?}"), "Admitted(<plan>)");
    }
}

#[test]
fn dataset_deferred_parses_exactly_once_and_never_after_cancellation() {
    isolated(|| {
        let binding = fixture();
        let mut counts = Vec::new();
        for cached in [true, true, false] {
            let (outcome, parses) = parse_spans(|| open(cached, &binding, SELECT, FREE));
            admitted(outcome.unwrap());
            counts.push(parses);
        }
        let cases = [
            (SELECT, DENY, OK),
            (SELECT, OK, DENY),
            (MISSING, OK, OK),
            (UPDATE, OK, OK),
            ("not sparql", OK, OK),
        ];
        for (query, graph, constant) in cases {
            let graph = move |_: &str| graph;
            let constant = move |_: ConstantOccurrence<'_>| constant;
            for cached in [true, false] {
                let (outcome, parses) = parse_spans(|| {
                    deferred(cached, &binding, query, &allow(), FREE, graph, constant)
                });
                refused(outcome.unwrap());
                counts.push(parses);
            }
        }
        assert!(counts.iter().all(|parses| *parses == 1), "{counts:?}");
        let control = budget(u64::MAX);
        control.terminate(CANCELLED);
        let (outcome, parses) = parse_spans(|| open(true, &binding, SELECT, &control));
        assert_eq!(cause(&outcome.unwrap_err()), Some(CANCELLED));
        assert_eq!(parses, 0);
    });
}
