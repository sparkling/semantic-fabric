use std::collections::BTreeMap;

use sf_core::ir::LogicalSource;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use sf_sql::Dialect;
use spargebra::{Query, SparqlParser};

use super::*;
use crate::compiler_control::CompileContext;
use crate::compiler_schema::ColumnTypeUse;
use crate::iq::node::{IqCond, IqNode};
use crate::iq::Scan;
use crate::plan_measure::clone_root::{measure_compiler_clone_root_v1, CompilerCloneRootV1};

fn budget(max_compiler_work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(
        max_compiler_work,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    ))
}

fn assert_control_error(error: Error, expected: QueryControlError) {
    match error {
        Error::QueryControl(actual) => assert_eq!(actual, expected),
        other => panic!("expected query-control error, got {other:?}"),
    }
}

fn exists_body(alias: usize, marker: &str) -> (IqNode, u64) {
    let node = IqNode::Extensional {
        scan: Scan {
            alias,
            source: LogicalSource::Query(format!("SELECT 1 /* {marker} */")),
        },
        bind: BTreeMap::new(),
    };
    let work = measure_compiler_clone_root_v1(CompilerCloneRootV1::IqNode(&node))
        .unwrap()
        .deep_clone_work;
    assert!(work > 0);
    (node, work)
}

fn values(branch_count: usize) -> IqNode {
    IqNode::Values {
        vars: Vec::new(),
        rows: (0..branch_count).map(|_| Vec::new()).collect(),
    }
}

fn lower(source: IqNode, work_mode: CompilerWorkMode<'_>) -> Result<Plan> {
    iq::lower::lower_with_work_mode(
        source,
        Dialect::Sqlite,
        &Default::default(),
        &Default::default(),
        work_mode,
    )
}

fn assert_direct_schedule(source: IqNode, branch_count: u64, clone_work: u64, prior_work: u64) {
    assert!(branch_count >= 3);
    let expected = prior_work + clone_work.checked_mul(branch_count - 1).unwrap();
    let retained_before_last_rejection =
        prior_work + clone_work.checked_mul(branch_count - 2).unwrap();
    let exact = budget(expected);
    let raw = iq::lower::lower(
        source.clone(),
        Dialect::Sqlite,
        &Default::default(),
        &Default::default(),
    )
    .unwrap();

    let metered = lower(
        source.clone(),
        CompilerWorkMode::Metered(CompileContext::new(&exact)),
    )
    .unwrap();

    assert_eq!(metered.branches.len() as u64, branch_count);
    assert_eq!(format!("{metered:?}"), format!("{raw:?}"));
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), expected);

    let short = budget(expected - 1);
    assert_control_error(
        lower(
            source,
            CompilerWorkMode::Metered(CompileContext::new(&short)),
        )
        .expect_err("N-1 must reject before the final borrowed-branch EXISTS clone"),
        QueryControlError::CompilerWorkExceeded,
    );
    assert_eq!(
        short.consumed(QueryCharge::CompilerWork),
        retained_before_last_rejection,
        "every earlier completed clone charge is retained"
    );
    assert_eq!(
        short.checkpoint(),
        Err(QueryControlError::CompilerWorkExceeded)
    );
}

fn nested_subplan_with_conditions(cond: Vec<IqCond>, branch_count: usize) -> IqNode {
    IqNode::Union {
        children: vec![
            IqNode::True,
            IqNode::OrderBy {
                child: Box::new(IqNode::Filter {
                    child: Box::new(values(branch_count)),
                    cond,
                }),
                keys: Vec::new(),
            },
        ],
        project: Vec::new(),
    }
}

fn nested_branch_count(plan: &Plan) -> usize {
    let subplan = plan
        .branches
        .iter()
        .flat_map(|branch| &branch.subplan_joins)
        .next()
        .expect("the modifier operand must lower as a SubPlan");
    subplan.plan.branches.len()
}

#[test]
fn metered_lower_retains_mode_inside_three_branch_subplan_and_charges_b_minus_one() {
    let (body, work) = exists_body(81, "owned-final-subplan-exists-body");
    let source = nested_subplan_with_conditions(vec![IqCond::Exists(Box::new(body))], 3);
    let expected = work * 2;
    let control = budget(expected);
    let raw = iq::lower::lower(
        source.clone(),
        Dialect::Sqlite,
        &Default::default(),
        &Default::default(),
    )
    .unwrap();

    let metered = lower(
        source,
        CompilerWorkMode::Metered(CompileContext::new(&control)),
    )
    .unwrap();

    assert_eq!(nested_branch_count(&metered), 3);
    assert_eq!(format!("{metered:?}"), format!("{raw:?}"));
    assert_eq!(control.consumed(QueryCharge::CompilerWork), expected);
}

#[test]
fn nested_subplan_exists_rejects_n_minus_one_before_the_first_clone() {
    let (body, work) = exists_body(82, "first-subplan-exists-boundary");
    let source = nested_subplan_with_conditions(vec![IqCond::Exists(Box::new(body))], 3);
    let control = budget(work - 1);

    assert_control_error(
        lower(
            source,
            CompilerWorkMode::Metered(CompileContext::new(&control)),
        )
        .expect_err("N-1 must reject inside the nested SubPlan before its EXISTS clone"),
        QueryControlError::CompilerWorkExceeded,
    );
    assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
    assert_eq!(
        control.checkpoint(),
        Err(QueryControlError::CompilerWorkExceeded)
    );
}

#[test]
fn later_nested_exists_failure_retains_the_completed_clone_charge() {
    let (first, first_work) = exists_body(83, "first-completed-subplan-exists-clone");
    let (second, second_work) = exists_body(84, "second-rejected-subplan-exists-clone");
    let source = nested_subplan_with_conditions(
        vec![
            IqCond::Exists(Box::new(first)),
            IqCond::Exists(Box::new(second)),
        ],
        3,
    );
    let control = budget(first_work + second_work - 1);

    assert_control_error(
        lower(
            source,
            CompilerWorkMode::Metered(CompileContext::new(&control)),
        )
        .expect_err("the second EXISTS clone must reject after the first completes"),
        QueryControlError::CompilerWorkExceeded,
    );
    assert_eq!(control.consumed(QueryCharge::CompilerWork), first_work);
    assert_eq!(
        control.checkpoint(),
        Err(QueryControlError::CompilerWorkExceeded)
    );
}

#[test]
fn wrapped_exists_over_four_filter_branches_charges_three_clones() {
    let (body, work) = exists_body(85, "wrapped-filter-exists-body");
    let nested_exists = IqCond::Exists(Box::new(body));
    let negated = IqCond::Not(Box::new(nested_exists));
    let wrapped = IqCond::And(vec![IqCond::Or(vec![negated])]);
    let source = IqNode::Filter {
        child: Box::new(values(4)),
        cond: vec![wrapped],
    };

    assert_direct_schedule(source, 4, work, 0);
}

#[test]
fn construction_filter_over_three_branches_uses_the_owned_final_schedule() {
    let (body, work) = exists_body(86, "construction-filter-exists-body");
    let source = IqNode::Construction {
        child: Box::new(IqNode::Filter {
            child: Box::new(values(3)),
            cond: vec![IqCond::Exists(Box::new(body))],
        }),
        subst: BTreeMap::new(),
        project: Vec::new(),
    };

    assert_direct_schedule(source, 3, work, 0);
}

#[test]
fn inner_join_condition_over_three_branches_uses_the_owned_final_schedule() {
    let (body, work) = exists_body(87, "inner-join-condition-exists-body");
    let source = IqNode::InnerJoin {
        children: vec![values(3), IqNode::True],
        cond: vec![IqCond::Exists(Box::new(body))],
    };

    // Two products (1×3 and 3×1), each copying only empty scalar branches,
    // precede the independent B-1 EXISTS-clone schedule.
    let empty_copy = measure_compiler_clone_root_v1(CompilerCloneRootV1::Branch(&Branch::empty()))
        .unwrap()
        .deep_clone_work;
    assert_direct_schedule(source, 3, work, 6 * (1 + empty_copy));
}

fn whole_pipeline_fixture() -> (Query, u64) {
    let query = SparqlParser::new()
        .parse_query(
            "SELECT ?x WHERE { VALUES ?x { 1 2 3 } \
             FILTER EXISTS { VALUES ?inside { 7 } } }",
        )
        .unwrap();
    let Query::Select { pattern, .. } = &query else {
        panic!("fixture must parse as SELECT")
    };
    let built = build::build_tree(pattern, None).unwrap();
    let IqNode::Construction { child, .. } = built else {
        panic!("SELECT projection must build a Construction")
    };
    let IqNode::Filter { child, cond } = *child else {
        panic!("fixture projection must contain its Filter")
    };
    let IqNode::Values { rows, .. } = *child else {
        panic!("fixture Filter must contain VALUES")
    };
    assert_eq!(rows.len(), 3);
    let [IqCond::Exists(inner)] = cond.as_slice() else {
        panic!("fixture must carry one EXISTS condition")
    };
    let work = measure_compiler_clone_root_v1(CompilerCloneRootV1::IqNode(inner))
        .unwrap()
        .deep_clone_work;
    (query, work)
}

fn translate_fixture(query: &Query, control: &QueryBudget) -> Result<Plan> {
    translate_tree_with_column_type_use(
        query,
        &[],
        &Tbox::default(),
        Dialect::Sqlite,
        &[],
        ColumnTypeUse::CallerAuthorizedFrozen,
        CompilerWorkMode::Metered(CompileContext::new(control)),
    )
}

#[test]
fn private_whole_pipeline_entry_preserves_exact_lowering_mode_and_failure_charge() {
    let (query, work) = whole_pipeline_fixture();
    let expected = work * 2;
    let exact = budget(expected);
    let raw = translate_tree(&query, &[], &Tbox::default(), Dialect::Sqlite, &[]).unwrap();

    let metered = translate_fixture(&query, &exact).unwrap();

    assert_eq!(metered.branches.len(), 3);
    assert_eq!(format!("{metered:?}"), format!("{raw:?}"));
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), expected);

    let short = budget(expected - 1);
    assert_control_error(
        translate_fixture(&query, &short)
            .expect_err("whole-pipeline N-1 must reject before the second EXISTS clone"),
        QueryControlError::CompilerWorkExceeded,
    );
    assert_eq!(short.consumed(QueryCharge::CompilerWork), work);
    assert_eq!(
        short.checkpoint(),
        Err(QueryControlError::CompilerWorkExceeded)
    );
}
