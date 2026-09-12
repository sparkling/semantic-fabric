use std::collections::BTreeMap;

use sf_core::ir::LogicalSource;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use sf_sql::Dialect;

use super::*;
use crate::compiler_control::CompileContext;
use crate::iq::node::{IqCond, IqNode};
use crate::iq::{Branch, Scan, SubPlanJoin, TermDef};
use crate::plan_measure::clone_root::{measure_copy_collection, CompilerCloneCollectionV1};

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
        source: (LogicalSource::Table(format!("source_{alias}"))).into(),
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

// Independently price the work between candidate cloning and recursive entry.
// These fixtures have equal projection widths, so every adjacent pair is visited.
fn candidate_tail_work(source: &[Branch]) -> u64 {
    let control = budget(u64::MAX);
    let work = crate::build::control::BuildWork::new(CompilerWorkMode::Metered(
        CompileContext::new(&control),
    ));
    let post = cascade::run_with_work(source.to_vec(), &[], &cascade::CascadeCtx::default(), work)
        .unwrap();
    for branch in &post {
        crate::finalization::projection(branch, work).unwrap();
    }
    let vector = post.len() as u64 * (1 + std::mem::size_of::<usize>() as u64);
    let comparisons = if source.len() == 1 {
        0
    } else {
        post.len().saturating_sub(1) as u64
    };
    vector + control.consumed(QueryCharge::CompilerWork) + 1 + comparisons
}

fn condition_with_nested_exists() -> Vec<IqCond> {
    vec![IqCond::Exists(Box::new(IqNode::Values {
        vars: vec!["inside".into()],
        rows: vec![vec![Some(TermDef::Const(
            spargebra::term::NamedNode::new("http://example.test/nested-payload")
                .unwrap()
                .into(),
        ))]],
    }))]
}

fn extensional_arm(alias: usize) -> IqNode {
    IqNode::Extensional {
        scan: Scan {
            alias,
            source: (LogicalSource::Table(format!("source_{alias}"))).into(),
        },
        bind: BTreeMap::new(),
    }
}

fn filter_over_union(aliases: &[usize], cond: Vec<IqCond>) -> IqNode {
    IqNode::Filter {
        child: Box::new(IqNode::Union {
            children: aliases.iter().copied().map(extensional_arm).collect(),
            project: Vec::new(),
        }),
        cond,
    }
}

fn distributed_filter_aliases(node: &IqNode) -> Vec<usize> {
    let IqNode::Union { children, .. } = node else {
        panic!("expected distributed Union, got {node:?}")
    };
    children
        .iter()
        .map(|child| {
            let IqNode::Filter { child, .. } = child else {
                panic!("expected Filter arm, got {child:?}")
            };
            let IqNode::Extensional { scan, .. } = child.as_ref() else {
                panic!("expected Extensional Filter child, got {child:?}")
            };
            scan.alias
        })
        .collect()
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
    let measure = measure_copy_collection(CompilerCloneCollectionV1::Branches(nested_branches(
        &source,
    )))
    .unwrap();
    // Root entry + join visit, candidate preparation, then two leaf entries.
    let expected = 2 + measure.total_work + candidate_tail_work(nested_branches(&source)) + 2;
    let budget = budget(expected);
    let mode = CompilerWorkMode::Metered(CompileContext::new(&budget));
    let mut metered = source.clone();
    let mut raw = source;

    cascade_subplans(&mut metered, &[], mode).unwrap();
    cascade_subplans(&mut raw, &[], CompilerWorkMode::Uncontrolled).unwrap();

    assert_eq!(format!("{metered:?}"), format!("{raw:?}"));
    assert_eq!(budget.consumed(QueryCharge::CompilerWork), expected);
}

#[test]
fn metered_nested_subplan_clone_rejects_n_minus_one_before_mutation() {
    let mut source = outer_with_nested_plan();
    let measure = measure_copy_collection(CompilerCloneCollectionV1::Branches(nested_branches(
        &source,
    )))
    .unwrap();
    let budget = budget(2 + measure.total_work - 1);
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
    assert_eq!(
        budget.consumed(QueryCharge::CompilerWork),
        2 + measure.measurement_work
    );
    assert_eq!(
        budget.checkpoint(),
        Err(QueryControlError::CompilerWorkExceeded)
    );
}

#[test]
fn nested_rejection_keeps_prior_operation_charge_without_whole_call_rollback() {
    let mut source = outer_with_two_nested_levels();
    let outer_measure = measure_copy_collection(CompilerCloneCollectionV1::Branches(
        nested_branches(&source),
    ))
    .unwrap();
    let inner_measure = measure_copy_collection(CompilerCloneCollectionV1::Branches(
        second_level_branches(&source),
    ))
    .unwrap();
    let before_inner =
        2 + outer_measure.total_work + candidate_tail_work(nested_branches(&source)) + 2;
    let budget = budget(before_inner + inner_measure.total_work - 1);
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
        before_inner + inner_measure.measurement_work,
        "completed operation charges are never refunded"
    );
    assert_eq!(
        budget.checkpoint(),
        Err(QueryControlError::CompilerWorkExceeded)
    );
}

#[test]
fn metered_filter_over_three_arms_charges_two_exact_nested_condition_clones() {
    let cond = condition_with_nested_exists();
    let measure = measure_copy_collection(CompilerCloneCollectionV1::IqConditions(&cond)).unwrap();
    let expected = crate::compiler_control::normalization_test_support::filter(3).0
        + measure.total_work.checked_mul(2).unwrap();
    let budget = budget(expected);

    let normalized = iq::normalize::normalize_with_work_mode(
        filter_over_union(&[11, 11, 22], cond),
        CompilerWorkMode::Metered(CompileContext::new(&budget)),
    )
    .unwrap();

    assert_eq!(budget.consumed(QueryCharge::CompilerWork), expected);
    assert_eq!(distributed_filter_aliases(&normalized), vec![11, 11, 22]);
}

#[test]
fn later_filter_arm_rejection_keeps_the_completed_clone_charge() {
    let cond = condition_with_nested_exists();
    let measure = measure_copy_collection(CompilerCloneCollectionV1::IqConditions(&cond)).unwrap();
    let (_, prefix, between) = crate::compiler_control::normalization_test_support::filter(3);
    let prefix = prefix + measure.total_work + between;
    let budget = budget(prefix + measure.total_work - 1);

    assert_control_error(
        iq::normalize::normalize_with_work_mode(
            filter_over_union(&[21, 22, 23], cond),
            CompilerWorkMode::Metered(CompileContext::new(&budget)),
        )
        .expect_err("the second clone must reject after the first operation completes"),
        QueryControlError::CompilerWorkExceeded,
    );

    assert_eq!(
        budget.consumed(QueryCharge::CompilerWork),
        prefix + measure.measurement_work,
        "completed operation charges are never refunded"
    );
}

#[test]
fn metered_filter_union_exact_n_matches_the_public_raw_path() {
    let cond = condition_with_nested_exists();
    let measure = measure_copy_collection(CompilerCloneCollectionV1::IqConditions(&cond)).unwrap();
    let source = filter_over_union(&[31, 32], cond);
    let expected =
        crate::compiler_control::normalization_test_support::filter(2).0 + measure.total_work;
    let budget = budget(expected);

    let raw = iq::normalize::normalize(source.clone()).unwrap();
    let metered = iq::normalize::normalize_with_work_mode(
        source,
        CompilerWorkMode::Metered(CompileContext::new(&budget)),
    )
    .unwrap();

    assert_eq!(format!("{metered:?}"), format!("{raw:?}"));
    assert_eq!(budget.consumed(QueryCharge::CompilerWork), expected);
}

#[test]
fn metered_filter_union_rejects_n_minus_one_before_the_guarded_clone() {
    let cond = condition_with_nested_exists();
    let measure = measure_copy_collection(CompilerCloneCollectionV1::IqConditions(&cond)).unwrap();
    let (_, prefix, _) = crate::compiler_control::normalization_test_support::filter(2);
    let budget = budget(prefix + measure.total_work - 1);

    assert_control_error(
        iq::normalize::normalize_with_work_mode(
            filter_over_union(&[41, 42], cond),
            CompilerWorkMode::Metered(CompileContext::new(&budget)),
        )
        .expect_err("N-1 must reject before the preceding-arm condition clone"),
        QueryControlError::CompilerWorkExceeded,
    );

    assert_eq!(
        budget.consumed(QueryCharge::CompilerWork),
        prefix + measure.measurement_work
    );
    assert_eq!(
        budget.checkpoint(),
        Err(QueryControlError::CompilerWorkExceeded)
    );
}
