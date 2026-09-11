//! Exactness and terminal-state evidence for the finite DESCRIBE form block.
use std::sync::atomic::{AtomicUsize, Ordering};

use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use spargebra::{Query, SparqlParser};

use super::*;
use crate::compiler_control::CompileContext;
use crate::{CompilerWorkMode, Result};

fn pattern(source: &str) -> GraphPattern {
    let Query::Describe { pattern, .. } = SparqlParser::new().parse_query(source).unwrap() else {
        panic!("DESCRIBE fixture required")
    };
    pattern
}

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

fn controlled(
    p: &GraphPattern,
    control: &dyn QueryControl,
) -> Result<(GraphPattern, Vec<TriplePattern>)> {
    rewrite_with_work_mode(p, CompilerWorkMode::Metered(CompileContext::new(control)))
}

fn fixtures() -> Vec<GraphPattern> {
    [
        "DESCRIBE <http://example.test/item/1>",
        "DESCRIBE <http://example.test/東京> WHERE { VALUES ?x { 1 1 UNDEF } }",
        "DESCRIBE ?s WHERE { VALUES ?s { <http://ex/a> <http://ex/a> } }",
        "DESCRIBE ?s WHERE { ?s ?p ?o FILTER(EXISTS { ?s ?q ?x } && !BOUND(?__sf_describe_predicate_0)) }",
        "DESCRIBE ?s WHERE { ?s ?__sf_describe_predicate_0 ?__sf_describe_object_0 OPTIONAL { ?s ?__sf_describe_predicate_1 ?__sf_describe_object_1 } }",
        "DESCRIBE ?s WHERE { GRAPH ?g { ?s ?p ?o } BIND(CONCAT(STR(?o), 'café 東京') AS ?label) }",
    ].into_iter().map(pattern).collect()
}

#[test]
fn describe_rewrite_exact_and_one_short_preserve_raw_shape_and_authored_names() {
    for p in fixtures() {
        let before = p.clone();
        let raw = rewrite(&p).unwrap();
        let measured = budget(u64::MAX);
        assert_eq!(controlled(&p, &measured).unwrap(), raw);
        let work = measured.consumed(QueryCharge::CompilerWork);
        assert!(work > 0 && work < 1_000_000);
        let exact = budget(work);
        assert_eq!(controlled(&p, &exact).unwrap(), raw);
        assert_eq!(exact.consumed(QueryCharge::CompilerWork), work);
        let short = budget(work - 1);
        assert!(matches!(
            controlled(&p, &short),
            Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
        ));
        assert_eq!(
            short.checkpoint(),
            Err(QueryControlError::CompilerWorkExceeded)
        );
        assert_eq!(p, before, "refusal cannot change the borrowed input");
        assert_eq!(measured.consumed(QueryCharge::SourceWork), 0);
    }
}

#[test]
fn describe_shape_and_unbound_target_keep_the_existing_error_order() {
    for source in [
        "DESCRIBE * WHERE {}",
        "DESCRIBE <http://ex/a> <http://ex/b>",
    ] {
        let p = pattern(source);
        let control = budget(0);
        assert_eq!(
            controlled(&p, &control).unwrap_err().to_string(),
            rewrite(&p).unwrap_err().to_string()
        );
        assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
    }
    let p = pattern("DESCRIBE ?s WHERE { ?x ?p ?o }");
    assert!(matches!(
        controlled(&p, &budget(0)),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    assert_eq!(
        controlled(&p, &budget(u64::MAX)).unwrap_err().to_string(),
        rewrite(&p).unwrap_err().to_string()
    );
}

#[test]
fn describe_final_checkpoint_keeps_the_first_error_and_sticky_terminal_state() {
    struct StopAtCheckpoint {
        budget: QueryBudget,
        calls: AtomicUsize,
        stop: usize,
        cause: QueryControlError,
    }
    impl QueryControl for StopAtCheckpoint {
        fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.stop {
                self.budget.terminate(self.cause);
            }
            self.budget.checkpoint()
        }
        fn consume(
            &self,
            charge: QueryCharge,
            units: u64,
        ) -> std::result::Result<(), QueryControlError> {
            self.budget.consume(charge, units)
        }
        fn terminate(&self, cause: QueryControlError) -> QueryControlError {
            self.budget.terminate(cause)
        }
    }
    for (source, semantic_error) in [
        ("DESCRIBE ?s WHERE { ?x ?p ?o }", true),
        ("DESCRIBE <http://ex/a>", false),
    ] {
        let p = pattern(source);
        let observed = StopAtCheckpoint {
            budget: budget(u64::MAX),
            calls: AtomicUsize::new(0),
            stop: usize::MAX,
            cause: QueryControlError::Cancelled,
        };
        let original = controlled(&p, &observed);
        assert_eq!(original.is_err(), semantic_error);
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            let control = StopAtCheckpoint {
                budget: budget(u64::MAX),
                calls: AtomicUsize::new(0),
                stop: observed.calls.load(Ordering::SeqCst),
                cause,
            };
            let error = controlled(&p, &control).unwrap_err();
            if semantic_error {
                assert_eq!(
                    error.to_string(),
                    original.as_ref().unwrap_err().to_string()
                );
            } else {
                assert!(matches!(error, Error::QueryControl(actual) if actual == cause));
            }
            assert_eq!(control.budget.checkpoint(), Err(cause));
        }
    }
}

struct StopAtCharge {
    budget: QueryBudget,
    calls: AtomicUsize,
    stop: usize,
    cause: QueryControlError,
}

impl QueryControl for StopAtCharge {
    fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
        self.budget.checkpoint()
    }
    fn consume(
        &self,
        charge: QueryCharge,
        units: u64,
    ) -> std::result::Result<(), QueryControlError> {
        self.budget.consume(charge, units)?;
        if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.stop {
            self.budget.terminate(self.cause);
        }
        Ok(())
    }
    fn terminate(&self, cause: QueryControlError) -> QueryControlError {
        self.budget.terminate(cause)
    }
}

#[test]
fn describe_checks_cancellation_and_deadline_at_every_paid_boundary() {
    for p in fixtures() {
        let observed = StopAtCharge {
            budget: budget(u64::MAX),
            calls: AtomicUsize::new(0),
            stop: usize::MAX,
            cause: QueryControlError::Cancelled,
        };
        controlled(&p, &observed).unwrap();
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            for stop in 1..=observed.calls.load(Ordering::SeqCst) {
                let control = StopAtCharge {
                    budget: budget(u64::MAX),
                    calls: AtomicUsize::new(0),
                    stop,
                    cause,
                };
                assert!(
                    matches!(controlled(&p, &control), Err(Error::QueryControl(actual)) if actual == cause),
                    "stop={stop}"
                );
                assert_eq!(control.checkpoint(), Err(cause));
            }
        }
    }
}

#[test]
fn fresh_collision_attempts_are_paid_before_mutation_and_overflow_is_sticky() {
    let mut names: BTreeSet<_> = (0..12)
        .map(|n| Variable::new_unchecked(format!("__sf_describe_predicate_{n}")))
        .collect();
    let original = names.clone();
    let measured = budget(u64::MAX);
    let mode = CompilerWorkMode::Metered(CompileContext::new(&measured));
    let got = fresh(
        &mut names,
        "predicate",
        crate::build::control::BuildWork::new(mode),
    )
    .unwrap();
    assert_eq!(got.as_str(), "__sf_describe_predicate_12");
    let short = budget(measured.consumed(QueryCharge::CompilerWork) - 1);
    let work = crate::build::control::BuildWork::new(CompilerWorkMode::Metered(
        CompileContext::new(&short),
    ));
    let mut unchanged = original.clone();
    assert!(fresh(&mut unchanged, "predicate", work).is_err());
    assert_eq!(
        unchanged, original,
        "failed candidate must not enter the name set"
    );
    let overflow = budget(u64::MAX);
    let work = crate::build::control::BuildWork::new(CompilerWorkMode::Metered(
        CompileContext::new(&overflow),
    ));
    assert!(matches!(
        control::reserve_fresh(work, usize::MAX, 9, 0),
        Err(Error::QueryControl(QueryControlError::AccountingOverflow))
    ));
    assert_eq!(
        overflow.checkpoint(),
        Err(QueryControlError::AccountingOverflow)
    );
}

#[test]
fn describe_input_depth_is_checked_before_recursive_copy_and_scope_discovery() {
    let mut p = GraphPattern::Values {
        variables: vec![Variable::new_unchecked("s")],
        bindings: vec![],
    };
    for _ in 0..140 {
        p = GraphPattern::Distinct { inner: Box::new(p) };
    }
    let p = GraphPattern::Project {
        inner: Box::new(p),
        variables: vec![Variable::new_unchecked("s")],
    };
    assert!(matches!(
        controlled(&p, &budget(u64::MAX)),
        Err(Error::QueryControl(
            QueryControlError::CompilerEnvelopeExceeded
        ))
    ));
}
