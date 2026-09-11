use super::*;
use crate::compiler_control::CompileContext;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

fn run(node: IqNode, control: &dyn QueryControl) -> Result<IqNode> {
    let tbox = Tbox::default();
    let mut cx = ResolveCx::new(&[], &tbox, sf_sql::Dialect::Sqlite, &[]).with_work_mode(
        crate::CompilerWorkMode::Metered(CompileContext::new(control)),
    );
    resolve(node, &mut cx)
}

#[test]
fn resolve_recursive_stops_are_sticky_and_restore_existential_scope() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Stop {
        budget: QueryBudget,
        calls: AtomicUsize,
        at: usize,
        cause: QueryControlError,
    }
    impl QueryControl for Stop {
        fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
            self.budget.checkpoint()
        }
        fn consume(
            &self,
            charge: QueryCharge,
            amount: u64,
        ) -> std::result::Result<(), QueryControlError> {
            self.budget.consume(charge, amount)?;
            if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.at {
                return Err(self.budget.terminate(self.cause));
            }
            Ok(())
        }
        fn terminate(&self, cause: QueryControlError) -> QueryControlError {
            self.budget.terminate(cause)
        }
    }
    let node = IqNode::Filter {
        child: Box::new(IqNode::True),
        cond: vec![IqCond::Exists(Box::new(IqNode::Filter {
            child: Box::new(IqNode::True),
            cond: vec![IqCond::NotExists {
                inner: Box::new(IqNode::True),
                is_minus: true,
            }],
        }))],
    };
    let observer = Stop {
        budget: budget(u64::MAX),
        calls: AtomicUsize::new(0),
        at: usize::MAX,
        cause: QueryControlError::Cancelled,
    };
    run(node.clone(), &observer).unwrap();
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for at in 1..=observer.calls.load(Ordering::SeqCst) {
            let stop = Stop {
                budget: budget(u64::MAX),
                calls: AtomicUsize::new(0),
                at,
                cause,
            };
            let tbox = Tbox::default();
            let mut cx = ResolveCx::new(&[], &tbox, sf_sql::Dialect::Sqlite, &[])
                .with_work_mode(crate::CompilerWorkMode::Metered(CompileContext::new(&stop)));
            assert!(
                matches!(resolve(node.clone(), &mut cx), Err(Error::QueryControl(actual)) if actual == cause)
            );
            assert!(!cx.unfolder.in_existential, "scope restored at stop {at}");
            assert_eq!(stop.checkpoint(), Err(cause));
        }
    }
}

#[test]
fn resolve_pass_through_depth_is_bounded() {
    let mut admitted = IqNode::True;
    for _ in 1..crate::compile_envelope::algebra::MAX_ALGEBRA_DEPTH_V1 {
        admitted = IqNode::Distinct {
            child: Box::new(admitted),
        };
    }
    run(admitted, &budget(u64::MAX)).unwrap();
    let mut node = IqNode::True;
    for _ in 0..crate::compile_envelope::algebra::MAX_ALGEBRA_DEPTH_V1 {
        node = IqNode::Distinct {
            child: Box::new(node),
        };
    }
    assert!(matches!(
        run(node, &budget(u64::MAX)),
        Err(Error::QueryControl(
            QueryControlError::CompilerEnvelopeExceeded
        ))
    ));
}

#[test]
fn resolve_pass_through_visits_are_paid_and_exact() {
    let node = IqNode::Filter {
        child: Box::new(IqNode::True),
        cond: vec![IqCond::Exists(Box::new(IqNode::True))],
    };
    let paid = budget(u64::MAX);
    let expected = run(node.clone(), &paid).unwrap();
    assert_eq!(format!("{expected:?}"), format!("{node:?}"));
    let total = paid.consumed(QueryCharge::CompilerWork);
    assert!(
        total > 0,
        "RESOLVE must pay even when no mapping leaf expands"
    );
    assert_eq!(
        format!("{:?}", run(node.clone(), &budget(total)).unwrap()),
        format!("{expected:?}")
    );
    assert!(matches!(
        run(node, &budget(total - 1)),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
}

#[test]
fn resolve_empty_leaf_preserves_ordered_unique_graph_variables_at_exact_budget() {
    use spargebra::term::{NamedNodePattern, TriplePattern, Variable};
    for graph_name in ["s", "g"] {
        let node = IqNode::Intensional {
            pattern: TriplePattern {
                subject: Variable::new_unchecked("s").into(),
                predicate: Variable::new_unchecked("p").into(),
                object: Variable::new_unchecked("s").into(),
            },
            graph: Some(NamedNodePattern::Variable(Variable::new_unchecked(
                graph_name,
            ))),
        };
        let measured = budget(u64::MAX);
        let result = run(node.clone(), &measured).unwrap();
        let IqNode::Empty { vars } = &result else {
            panic!("no mapping arms");
        };
        let expected = if graph_name == "s" {
            vec!["s", "p"]
        } else {
            vec!["s", "p", "g"]
        };
        assert_eq!(
            vars.iter().map(|v| v.as_ref()).collect::<Vec<_>>(),
            expected
        );
        let units = measured.consumed(QueryCharge::CompilerWork);
        assert_eq!(
            format!("{:?}", run(node.clone(), &budget(units)).unwrap()),
            format!("{result:?}")
        );
        assert!(matches!(
            run(node, &budget(units - 1)),
            Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
        ));
    }
}
