use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use sf_core::query_control::{QueryBudget, QueryCharge, QueryControl, QueryControlError as Stop};

use crate::cache::generated::tests::{isolated, parse_spans};
use crate::cache::generated::{
    ConstantOccurrence, DatasetGraphAllowlist, DatasetRule, GeneratedDatasetError,
};
use crate::cache::{SecurityCompileError, SecurityDatasetCompileError as E};
use crate::Error;

use super::test_support::*;

struct ObservedControl {
    budget: QueryBudget,
    checkpoints: AtomicUsize,
    stop_after_entry: Option<Stop>,
}

impl ObservedControl {
    fn new(stop_after_entry: Option<Stop>) -> Self {
        Self {
            budget: budget(u64::MAX),
            checkpoints: AtomicUsize::new(0),
            stop_after_entry,
        }
    }
}

impl QueryControl for ObservedControl {
    fn checkpoint(&self) -> Result<(), Stop> {
        if self.checkpoints.fetch_add(1, Ordering::SeqCst) > 0 {
            if let Some(cause) = self.stop_after_entry {
                self.budget.terminate(cause);
            }
        }
        self.budget.checkpoint()
    }

    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), Stop> {
        self.budget.consume(charge, amount)
    }

    fn terminate(&self, reason: Stop) -> Stop {
        self.budget.terminate(reason)
    }
}

#[test]
fn dataset_deferred_security_wrong_policy_is_first_and_unpaid() {
    isolated(|| {
        let (binding, cache) = (fixture(), cache());
        let control = ObservedControl::new(None);
        control.terminate(Stop::Cancelled);
        let (result, parses) = parse_spans(|| {
            binding
                .for_security_policy(policy(1), &cache)
                .compile_shared_with_single_default_dataset_deferred(
                    &context(4, 2, 3),
                    "not sparql",
                    &allow(),
                    &control,
                    |_| panic!("graph visited"),
                    |_| panic!("constant visited"),
                )
        });
        assert!(matches!(
            result,
            Err(E::Security(SecurityCompileError::PolicyMismatch))
        ));
        assert_eq!(parses, 0);
        assert_eq!(control.checkpoints.load(Ordering::SeqCst), 0);
        assert_eq!(work(&control.budget), 0);
        assert_eq!(cache.access_counts(), (0, 0));
        assert_eq!((cache.len(), binding.cache_len()), (0, 0));
    });
}

#[test]
fn dataset_deferred_security_admitted_matches_existing_and_keeps_partitions() {
    let (binding, mine, theirs) = (fixture(), cache(), cache());
    let alice = context(1, 2, 3);
    for query in [SELECT, ASK] {
        let mut plans = Vec::new();
        for _ in 0..2 {
            let (paid, spent) = (budget(u64::MAX), budget(u64::MAX));
            let expected = existing(
                &binding,
                &theirs,
                &alice,
                query,
                &paid,
                pass_graph,
                pass_constant,
            );
            let expected = expected.unwrap();
            let outcome = run(
                &binding,
                &mine,
                &alice,
                query,
                &spent,
                pass_graph,
                pass_constant,
            );
            let plan = admitted(outcome.unwrap());
            assert_eq!(format!("{plan:?}"), format!("{expected:?}"), "{query}");
            assert!(work(&paid) > 0, "{query}");
            assert_eq!(work(&spent), work(&paid), "{query}");
            plans.push(plan);
        }
        assert!(Arc::ptr_eq(&plans[0], &plans[1]));
        let shared = existing(
            &binding,
            &mine,
            &alice,
            query,
            FREE,
            pass_graph,
            pass_constant,
        );
        assert!(Arc::ptr_eq(&plans[0], &shared.unwrap()));
    }
    let first = admitted(
        run(
            &binding,
            &mine,
            &alice,
            SELECT,
            FREE,
            pass_graph,
            pass_constant,
        )
        .unwrap(),
    );
    for other in [context(4, 2, 3), context(1, 4, 3), context(1, 2, 4)] {
        let next = run(
            &binding,
            &mine,
            &other,
            SELECT,
            FREE,
            pass_graph,
            pass_constant,
        );
        let next = admitted(next.unwrap());
        assert!(!Arc::ptr_eq(&first, &next));
        let again = run(
            &binding,
            &mine,
            &other,
            SELECT,
            FREE,
            pass_graph,
            pass_constant,
        );
        assert!(Arc::ptr_eq(&next, &admitted(again.unwrap())));
    }
    assert_eq!(mine.len(), 5);
    assert_eq!(binding.cache_len(), 0);
    let raw = binding.compile_shared_with_single_default_dataset(
        SELECT,
        &allow(),
        FREE,
        pass_graph,
        pass_constant,
    );
    assert!(!Arc::ptr_eq(&raw.unwrap(), &first));
    assert_eq!((binding.cache_len(), mine.len()), (1, 5));
}

#[test]
fn dataset_deferred_security_refusal_authorizes_requested_named_graph_tables() {
    let (binding, cache) = (fixture(), cache());
    let alice = context(1, 2, 3);
    for (iri, table) in [(A, TABLE_A), (B, TABLE_B)] {
        let query = SELECT.replace(A, iri);
        let expected = scoped(&binding, &query);
        assert_eq!(tables(&expected), [table], "{query}");
        for graph_denied in [true, false] {
            let verdicts = if graph_denied { (DENY, OK) } else { (OK, DENY) };
            let graph = move |_: &str| verdicts.0;
            let constant = move |_: ConstantOccurrence<'_>| verdicts.1;
            let outcome = run(&binding, &cache, &alice, &query, FREE, graph, constant);
            let (_, plan) = refused(outcome.unwrap());
            let plan = plan.expect("requested graph requires row authorization");
            assert_eq!(tables(&plan), [table], "unrelated graph authorized");
            assert_eq!(format!("{plan:?}"), format!("{expected:?}"), "{query}");
        }
    }
    assert_eq!(cache.access_counts(), (0, 0));
    assert_eq!((cache.len(), binding.cache_len()), (0, 0));
}

#[test]
fn dataset_deferred_security_refusals_bypass_both_caches_after_fresh_checks() {
    let (binding, cache) = (fixture(), cache());
    let alice = context(1, 2, 3);
    let cached = run(
        &binding,
        &cache,
        &alice,
        SELECT,
        FREE,
        pass_graph,
        pass_constant,
    );
    let cached = admitted(cached.unwrap());
    let expected = format!("{:?}", scoped(&binding, SELECT));
    let accesses = cache.access_counts();
    let only_b = DatasetGraphAllowlist::new([B]).unwrap();
    for _ in 0..2 {
        for graph_denied in [true, false] {
            let graph = |_: &str| {
                assert_eq!(cache.access_counts(), accesses);
                if graph_denied {
                    DENY
                } else {
                    OK
                }
            };
            let constant = |_: ConstantOccurrence<'_>| {
                assert!(!graph_denied);
                assert_eq!(cache.access_counts(), accesses);
                DENY
            };
            let outcome = run(&binding, &cache, &alice, SELECT, FREE, graph, constant);
            let (refusal, plan) = refused(outcome.unwrap());
            let rule = if graph_denied {
                DatasetRule::GraphNotAdmitted
            } else {
                DatasetRule::ConstantCoverage
            };
            assert_eq!(refusal, rule);
            let plan = plan.expect("authorization plan");
            assert!(!Arc::ptr_eq(&plan, &cached));
            assert_eq!(format!("{plan:?}"), expected);
        }
        let outcome = binding
            .for_security_policy(policy(1), &cache)
            .compile_shared_with_single_default_dataset_deferred(
                &alice,
                SELECT,
                &only_b,
                FREE,
                |_| panic!("not allowlisted"),
                |_| panic!("not allowlisted"),
            );
        assert_eq!(refused(outcome.unwrap()).0, DatasetRule::GraphNotAdmitted);
        assert_eq!(cache.access_counts(), accesses);
        assert_eq!((cache.len(), binding.cache_len()), (1, 0));
    }
    let again = run(
        &binding,
        &cache,
        &alice,
        SELECT,
        FREE,
        pass_graph,
        pass_constant,
    );
    assert!(Arc::ptr_eq(&cached, &admitted(again.unwrap())));
}

#[test]
fn dataset_deferred_security_structural_refusals_keep_rules_without_cache_access() {
    let (binding, cache) = (fixture(), cache());
    let alice = context(1, 2, 3);
    let cases = [
        (MISSING, DatasetRule::MissingDataset),
        (NAMED, DatasetRule::NamedGraphDataset),
        (MULTIPLE, DatasetRule::MultipleDefaultGraphs),
        (CONSTRUCT, DatasetRule::QueryForm),
        (UPDATE, DatasetRule::FormNotAdmitted),
        ("not sparql", DatasetRule::FormNotAdmitted),
    ];
    for (query, rule) in cases {
        let error = existing(
            &binding,
            &cache,
            &alice,
            query,
            FREE,
            pass_graph,
            pass_constant,
        );
        assert_eq!(rule_of(&error.unwrap_err()), Some(rule), "{query}");
        let lowered = original(&binding, query);
        assert_eq!(
            lowered.is_some(),
            rule != DatasetRule::FormNotAdmitted,
            "{query}"
        );
        let graph = |_: &str| -> Verdict { panic!("graph visited") };
        let constant = |_: ConstantOccurrence<'_>| -> Verdict { panic!("constant visited") };
        let outcome = run(&binding, &cache, &alice, query, FREE, graph, constant);
        match lowered {
            // Syntax refusals and Unsupported lowering: no plan.
            None | Some(Err(Error::Unsupported(_))) => {
                let (refusal, plan) = refused(outcome.unwrap());
                assert_eq!(refusal, rule, "{query}");
                assert!(plan.is_none(), "{query}");
            }
            Some(Ok(expected)) => {
                let (refusal, plan) = refused(outcome.unwrap());
                assert_eq!(refusal, rule, "{query}");
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
        assert_eq!(cache.access_counts(), (0, 0), "{query}");
        assert_eq!((cache.len(), binding.cache_len()), (0, 0), "{query}");
    }
    // The multiple-default refusal above was lowered as parsed, never stripped
    // into the admissible single-graph query.
    let outcome = run(
        &binding,
        &cache,
        &alice,
        MULTIPLE,
        FREE,
        pass_graph,
        pass_constant,
    );
    if let Some(plan) = refused(outcome.unwrap()).1 {
        let stripped = scoped(&binding, SELECT);
        assert_ne!(format!("{plan:?}"), format!("{stripped:?}"));
    }
}

#[test]
fn dataset_deferred_security_failures_keep_typed_partitions() {
    let (binding, cache) = (fixture(), cache());
    let alice = context(1, 2, 3);
    let stopped = budget(u64::MAX);
    stopped.terminate(Stop::Cancelled);
    let error = run(
        &binding,
        &cache,
        &alice,
        SELECT,
        &stopped,
        pass_graph,
        pass_constant,
    );
    assert!(matches!(
        error.unwrap_err(),
        E::Dataset(GeneratedDatasetError::Compiler(Error::QueryControl(
            Stop::Cancelled
        )))
    ));
    for stop in [Stop::Cancelled, Stop::DeadlineExceeded] {
        let control = ObservedControl::new(Some(stop));
        let error = run(
            &binding,
            &cache,
            &alice,
            "not sparql",
            &control,
            pass_graph,
            pass_constant,
        );
        assert_eq!(cause(&error.unwrap_err()), Some(stop));
        assert_eq!(control.checkpoints.load(Ordering::SeqCst), 2);
    }
    for in_graph in [true, false] {
        let control = budget(u64::MAX);
        let graph = |_: &str| {
            if in_graph {
                control.terminate(Stop::DeadlineExceeded);
                DENY
            } else {
                OK
            }
        };
        let constant = |_: ConstantOccurrence<'_>| {
            control.terminate(Stop::DeadlineExceeded);
            DENY
        };
        let error = run(&binding, &cache, &alice, SELECT, &control, graph, constant);
        assert_eq!(cause(&error.unwrap_err()), Some(Stop::DeadlineExceeded));
    }
    let deny = |_: &str| DENY;
    let screening = budget(u64::MAX);
    let refusal = existing(
        &binding,
        &cache,
        &alice,
        SELECT,
        &screening,
        deny,
        pass_constant,
    );
    assert_eq!(
        rule_of(&refusal.unwrap_err()),
        Some(DatasetRule::GraphNotAdmitted)
    );
    let control = budget(work(&screening));
    let error = run(
        &binding,
        &cache,
        &alice,
        SELECT,
        &control,
        deny,
        pass_constant,
    );
    assert!(matches!(
        error.unwrap_err(),
        E::Security(SecurityCompileError::Compiler(Error::QueryControl(
            Stop::CompilerWorkExceeded
        )))
    ));
    let broken = broken();
    // A screened denial reaches the broken reference in its requested graph.
    let result = run(&broken, &cache, &alice, REF, FREE, deny, pass_constant);
    let error = result.unwrap_err();
    let mapping = matches!(
        error,
        E::Security(SecurityCompileError::Compiler(Error::Mapping(_)))
    );
    assert!(mapping, "{error}");
    // A structural refusal keeps whatever ordinary lowering of the original
    // reports; a mapping failure there is never replaced by the refusal.
    let lowered = original(&broken, MULTIPLE_REF).expect("parsed original");
    let result = run(
        &broken,
        &cache,
        &alice,
        MULTIPLE_REF,
        FREE,
        deny,
        pass_constant,
    );
    match lowered {
        Ok(expected) => {
            let (refusal, plan) = refused(result.unwrap());
            assert_eq!(refusal, DatasetRule::MultipleDefaultGraphs);
            let plan = plan.expect("authorization plan");
            assert_eq!(format!("{plan:?}"), format!("{expected:?}"));
        }
        Err(Error::Unsupported(_)) => assert!(refused(result.unwrap()).1.is_none()),
        Err(error) => {
            let Err(E::Security(SecurityCompileError::Compiler(kept))) = result else {
                panic!("typed lowering failure must propagate");
            };
            let same = std::mem::discriminant(&kept) == std::mem::discriminant(&error);
            assert!(same, "{kept}");
            assert_eq!(kept.to_string(), error.to_string());
        }
    }
    assert_eq!(cache.access_counts(), (0, 0));
    assert_eq!(
        (cache.len(), binding.cache_len(), broken.cache_len()),
        (0, 0, 0)
    );
}

#[test]
fn dataset_deferred_security_parses_exactly_once() {
    isolated(|| {
        let (binding, cache) = (fixture(), cache());
        let alice = context(1, 2, 3);
        let cases = [
            (SELECT, OK, OK),
            (SELECT, OK, OK),
            (SELECT, DENY, OK),
            (SELECT, OK, DENY),
            (MISSING, OK, OK),
            ("not sparql", OK, OK),
        ];
        let mut counts = Vec::new();
        for (query, graph, constant) in cases {
            let graph = move |_: &str| graph;
            let constant = move |_: ConstantOccurrence<'_>| constant;
            let (result, parses) =
                parse_spans(|| run(&binding, &cache, &alice, query, FREE, graph, constant));
            assert!(result.is_ok(), "{query}");
            counts.push(parses);
        }
        assert_eq!(counts, [1; 6]);
        assert_eq!((cache.len(), binding.cache_len()), (1, 0));
    });
}
