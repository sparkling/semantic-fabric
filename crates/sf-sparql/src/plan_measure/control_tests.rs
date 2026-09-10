use std::sync::atomic::{AtomicUsize, Ordering};

use ::spargebra::algebra::{Expression, GraphPattern};
use ::spargebra::term::{GroundTerm, Literal, Variable};
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};

use super::clone_root::*;
use super::*;

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

#[test]
fn leaf_measurement_costs_constant_work_not_its_payload_bytes_twice() {
    for text in ["x".to_owned(), "東京".repeat(1024)] {
        let var = Variable::new_unchecked(text);
        let root = CompilerCloneRootV1::Variable(&var);
        let work = 4 + std::mem::size_of::<Pending<'_>>() as u64;
        let exact = budget(work);
        let measured = measure_compiler_clone_root_with_control(root, &exact).unwrap();
        assert_eq!(measured, measure_compiler_clone_root_v1(root).unwrap());
        assert_eq!(measured.deep_clone_work, 1 + var.as_str().len() as u64);
        assert_eq!(exact.consumed(QueryCharge::CompilerWork), work);
        assert_eq!(exact.consumed(QueryCharge::SourceWork), 0);
        assert_eq!(
            measure_compiler_clone_root_with_control(root, &budget(work - 1)),
            Err(PlanMeasureError::Control(
                QueryControlError::CompilerWorkExceeded
            ))
        );
    }
}

#[test]
fn empty_and_variable_only_collections_pay_actual_iteration_without_stack() {
    for vars in [vec![], vec!["café".into(), "東京".into()]] {
        let root = CompilerCloneCollectionV1::Variables(&vars);
        let work = 2 + 2 * vars.len() as u64;
        let exact = budget(work);
        let measured = measure_compiler_clone_collection_with_control(root, &exact).unwrap();
        assert_eq!(
            measured,
            measure_compiler_clone_collection_v1(root).unwrap()
        );
        assert_eq!(measured.max_pending_items, 0);
        assert_eq!(exact.consumed(QueryCharge::CompilerWork), work);
        assert_eq!(
            measure_compiler_clone_collection_with_control(root, &budget(work - 1)),
            Err(PlanMeasureError::Control(
                QueryControlError::CompilerWorkExceeded
            ))
        );
    }
}

#[test]
fn logical_stack_growth_is_prepaid_even_when_the_allocator_has_slack() {
    let var = Variable::new_unchecked("x");
    for physical in [0, 64] {
        let paid = budget(u64::MAX);
        let mut walker = Walker::controlled(PlanMeasureLimits::V1, &paid).unwrap();
        walker.stack = Vec::with_capacity(physical);
        for _ in 0..4 {
            walker.push(0, Work::Variable(&var)).unwrap();
        }
        let measure = walker.finish().unwrap();
        // Entry + four pushes + four (node,payload) records; capacity1,2,4
        // growth with0,1,2 live slots relocated:1+3+6 slots.
        let work = 13 + 10 * std::mem::size_of::<Pending<'_>>() as u64;
        assert_eq!(paid.consumed(QueryCharge::CompilerWork), work);
        assert_eq!(measure.nodes, 4);
        assert_eq!(measure.max_pending_items, 4);
    }
    let paid = budget(1 + 1 + std::mem::size_of::<Pending<'_>>() as u64 - 1);
    let mut walker = Walker::controlled(PlanMeasureLimits::V1, &paid).unwrap();
    assert!(matches!(
        walker.push(0, Work::Variable(&var)),
        Err(PlanMeasureError::Control(
            QueryControlError::CompilerWorkExceeded
        ))
    ));
    assert_eq!(walker.stack.capacity(), 0);
    assert!(walker.stack.is_empty());
    assert_eq!(walker.logical_capacity, 0);
    assert_eq!(walker.measure.nodes, 0);
}

struct Interrupt {
    budget: QueryBudget,
    checks: AtomicUsize,
    charges: AtomicUsize,
    stop_check: usize,
    stop_charge: usize,
    cause: QueryControlError,
}

impl Interrupt {
    fn new(stop_check: usize, stop_charge: usize, cause: QueryControlError) -> Self {
        Self {
            budget: budget(u64::MAX),
            checks: AtomicUsize::new(0),
            charges: AtomicUsize::new(0),
            stop_check,
            stop_charge,
            cause,
        }
    }
}

impl QueryControl for Interrupt {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        if self.checks.fetch_add(1, Ordering::SeqCst) + 1 == self.stop_check {
            self.budget.terminate(self.cause);
        }
        self.budget.checkpoint()
    }
    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), QueryControlError> {
        self.budget.consume(charge, amount)?;
        if self.charges.fetch_add(1, Ordering::SeqCst) + 1 == self.stop_charge {
            self.budget.terminate(self.cause);
        }
        Ok(())
    }
    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.budget.terminate(reason)
    }
}

fn interrupts_every_step(
    run: impl Fn(&dyn QueryControl) -> Result<PlanMeasureV1, PlanMeasureError>,
) {
    let observed = Interrupt::new(usize::MAX, usize::MAX, QueryControlError::Cancelled);
    run(&observed).unwrap();
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for stop in 1..=observed.checks.load(Ordering::SeqCst) {
            let control = Interrupt::new(stop, usize::MAX, cause);
            assert_eq!(run(&control), Err(PlanMeasureError::Control(cause)));
            assert_eq!(control.checks.load(Ordering::SeqCst), stop);
            assert_eq!(
                control.terminate(QueryControlError::CompilerWorkExceeded),
                cause
            );
        }
        for stop in 1..=observed.charges.load(Ordering::SeqCst) {
            let control = Interrupt::new(usize::MAX, stop, cause);
            assert_eq!(run(&control), Err(PlanMeasureError::Control(cause)));
            assert_eq!(control.charges.load(Ordering::SeqCst), stop);
        }
    }
}

#[test]
fn ast_iq_and_absent_collections_stop_at_every_charge_and_checkpoint() {
    let graph = GraphPattern::Filter {
        expr: Expression::Exists(Box::new(GraphPattern::Values {
            variables: vec![Variable::new_unchecked("x")],
            bindings: vec![
                vec![],
                vec![None; 9],
                vec![Some(GroundTerm::Literal(Literal::from("東京"))), None],
            ],
        })),
        inner: Box::new(GraphPattern::Bgp { patterns: vec![] }),
    };
    interrupts_every_step(|c| {
        measure_compiler_clone_root_with_control(CompilerCloneRootV1::GraphPattern(&graph), c)
    });
    let rows = vec![vec![None; 12]];
    interrupts_every_step(|c| {
        measure_compiler_clone_collection_with_control(
            CompilerCloneCollectionV1::TermDefRows(&rows),
            c,
        )
    });
    interrupts_every_step(|c| {
        measure_compiler_clone_collection_with_control(
            CompilerCloneCollectionV1::GroundRows(
                &rows.iter().map(|r| vec![None; r.len()]).collect::<Vec<_>>(),
            ),
            c,
        )
    });
    let iq = IqNode::Values { vars: vec![], rows };
    interrupts_every_step(|c| {
        measure_compiler_clone_root_with_control(CompilerCloneRootV1::IqNode(&iq), c)
    });
    let plan = Plan {
        branches: vec![],
        form: PlanForm::Ask,
        distinct: false,
        limit: None,
        offset: 0,
        order: vec![],
        rust_group: None,
        dialect: sf_sql::Dialect::Sqlite,
        dedup_scopes: vec![None; 12],
        construct_drops_some_branch_var: false,
    };
    interrupts_every_step(|c| {
        measure_compiler_clone_root_with_control(CompilerCloneRootV1::Plan(&plan), c)
    });
}

#[test]
fn default_stack_depth_and_pending_envelopes_preserve_raw_metrics_and_errors() {
    let mut expr = Expression::Variable(Variable::new_unchecked("x"));
    for _ in 0..126 {
        expr = Expression::Not(Box::new(expr));
    }
    let root = CompilerCloneRootV1::Expression(&expr);
    assert_eq!(
        measure_compiler_clone_root_with_control(root, &budget(u64::MAX))
            .unwrap()
            .max_depth,
        128
    );
    expr = Expression::Not(Box::new(expr));
    let root = CompilerCloneRootV1::Expression(&expr);
    assert_eq!(
        measure_compiler_clone_root_with_control(root, &budget(u64::MAX)),
        measure_compiler_clone_root_v1(root)
    );
    let vars = [Variable::new_unchecked("a"), Variable::new_unchecked("b")];
    let paid = budget(u64::MAX);
    let limits = PlanMeasureLimits {
        max_pending_items: 1,
        ..PlanMeasureLimits::V1
    };
    let mut walker = Walker::controlled(limits, &paid).unwrap();
    walker.push(0, Work::Variable(&vars[0])).unwrap();
    let capacity = walker.logical_capacity;
    assert!(matches!(
        walker.push(0, Work::Variable(&vars[1])),
        Err(PlanMeasureError::LimitExceeded {
            dimension: PlanMeasureLimit::PendingItems,
            ..
        })
    ));
    assert_eq!(walker.stack.len(), 1);
    assert_eq!(walker.logical_capacity, capacity);
}
