use sf_core::ir::LogicalSource;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use sf_sql::Dialect;

use super::*;
use crate::compiler_control::CompileContext;
use crate::iq::{Branch, Scan, SubPlanJoin};
use crate::plan_measure::clone_root::{
    measure_compiler_clone_collection_v1, CompilerCloneCollectionV1,
};

fn budget(max_compiler_work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(
        max_compiler_work,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    ))
}

fn branch(alias: usize) -> Branch {
    Branch::single(Scan {
        alias,
        source: LogicalSource::Table(format!("source_{alias}")),
    })
}

fn select_plan(branches: Vec<Branch>) -> Plan {
    Plan {
        dedup_scopes: vec![None; branches.len()],
        branches,
        form: PlanForm::Select { vars: Vec::new() },
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        rust_group: None,
        dialect: Dialect::Sqlite,
        construct_drops_some_branch_var: false,
    }
}

fn outer_with_nested_plan() -> Branch {
    let nested_branches = vec![branch(1), branch(2)];
    let mut outer = branch(3);
    outer.subplan_joins.push(SubPlanJoin {
        alias: 4,
        plan: Box::new(select_plan(nested_branches)),
        on: Vec::new(),
        left: false,
    });
    outer
}

fn outer_with_two_nested_levels() -> Branch {
    let mut nested_parent = branch(1);
    nested_parent.subplan_joins.push(SubPlanJoin {
        alias: 5,
        plan: Box::new(select_plan(vec![branch(6), branch(7)])),
        on: Vec::new(),
        left: false,
    });
    let mut outer = branch(3);
    outer.subplan_joins.push(SubPlanJoin {
        alias: 4,
        plan: Box::new(select_plan(vec![nested_parent, branch(2)])),
        on: Vec::new(),
        left: false,
    });
    outer
}

fn nested_branches(branch: &Branch) -> &[Branch] {
    &branch.subplan_joins[0].plan.branches
}

fn second_level_branches(branch: &Branch) -> &[Branch] {
    &nested_branches(branch)[0].subplan_joins[0].plan.branches
}

fn assert_control_error(error: Error, expected: QueryControlError) {
    match error {
        Error::QueryControl(actual) => assert_eq!(actual, expected),
        other => panic!("expected query-control error, got {other:?}"),
    }
}

#[test]
fn metered_nested_subplan_clone_accepts_the_exact_measure() {
    let source = outer_with_nested_plan();
    let measure = measure_compiler_clone_collection_v1(CompilerCloneCollectionV1::Branches(
        nested_branches(&source),
    ))
    .unwrap();
    let budget = budget(measure.deep_clone_work);
    let mode = CompilerWorkMode::Metered(CompileContext::new(&budget));
    let mut metered = source.clone();
    let mut raw = source;

    cascade_subplans(&mut metered, &[], mode).unwrap();
    cascade_subplans(&mut raw, &[], CompilerWorkMode::Uncontrolled).unwrap();

    assert_eq!(format!("{metered:?}"), format!("{raw:?}"));
    assert_eq!(
        budget.consumed(QueryCharge::CompilerWork),
        measure.deep_clone_work
    );
}

#[test]
fn metered_nested_subplan_clone_rejects_n_minus_one_before_mutation() {
    let mut source = outer_with_nested_plan();
    let measure = measure_compiler_clone_collection_v1(CompilerCloneCollectionV1::Branches(
        nested_branches(&source),
    ))
    .unwrap();
    let budget = budget(measure.deep_clone_work - 1);
    let before = format!("{source:?}");
    let allocation = nested_branches(&source).as_ptr();

    assert_control_error(
        cascade_subplans(
            &mut source,
            &[],
            CompilerWorkMode::Metered(CompileContext::new(&budget)),
        )
        .expect_err("N-1 must reject before the rollback candidate clone"),
        QueryControlError::CompilerWorkExceeded,
    );

    assert_eq!(format!("{source:?}"), before);
    assert_eq!(nested_branches(&source).as_ptr(), allocation);
    assert_eq!(budget.consumed(QueryCharge::CompilerWork), 0);
    assert_eq!(
        budget.checkpoint(),
        Err(QueryControlError::CompilerWorkExceeded)
    );
}

#[test]
fn nested_rejection_keeps_prior_operation_charge_without_whole_call_rollback() {
    let mut source = outer_with_two_nested_levels();
    let outer_measure = measure_compiler_clone_collection_v1(CompilerCloneCollectionV1::Branches(
        nested_branches(&source),
    ))
    .unwrap();
    let inner_measure = measure_compiler_clone_collection_v1(CompilerCloneCollectionV1::Branches(
        second_level_branches(&source),
    ))
    .unwrap();
    let budget = budget(outer_measure.deep_clone_work + inner_measure.deep_clone_work - 1);
    let before = format!("{source:?}");
    let original_outer_allocation = nested_branches(&source).as_ptr();

    assert_control_error(
        cascade_subplans(
            &mut source,
            &[],
            CompilerWorkMode::Metered(CompileContext::new(&budget)),
        )
        .expect_err("the inner operation must reject after the outer one completes"),
        QueryControlError::CompilerWorkExceeded,
    );

    assert_eq!(format!("{source:?}"), before, "semantics remain unchanged");
    assert_ne!(
        nested_branches(&source).as_ptr(),
        original_outer_allocation,
        "the completed outer replacement is not transactionally rolled back"
    );
    assert_eq!(
        budget.consumed(QueryCharge::CompilerWork),
        outer_measure.deep_clone_work,
        "completed operation charges are never refunded"
    );
    assert_eq!(
        budget.checkpoint(),
        Err(QueryControlError::CompilerWorkExceeded)
    );
}
