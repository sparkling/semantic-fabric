use std::collections::BTreeMap;

use sf_core::ir::LogicalSource;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};

use super::*;
use crate::compiler_control::normalization_test_support as structural;
use crate::compiler_control::CompileContext;
use crate::iq::node::{IqCond, IqNode};
use crate::iq::{CmpOp, ColRef, Scan, SqlCond, TermDef};
use crate::plan_measure::clone_root::{
    measure_copy_collection, measure_copy_root, CompilerCloneCollectionV1, CompilerCloneRootV1,
};

struct InnerFixture {
    tree: IqNode,
    fixed_work: u64,
    condition_work: u64,
    condition_measurement: u64,
    owned_fixed_pointer: usize,
    owned_condition_pointer: usize,
}

struct LeftFixture {
    tree: IqNode,
    right_work: u64,
    fake_collection_work: u64,
    condition_work: u64,
    condition_measurement: u64,
    owned_right_pointer: usize,
    owned_condition_pointer: usize,
}

fn budget(max_compiler_work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(
        max_compiler_work,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    ))
}

/// Hand-counted NORMALIZE prerequisite and trailing union pass for A >= 2
/// nonconstant arms. Keep this separate from the exact owned-copy schedule:
/// entry + arm/push visits + paid geometric IQ slots/relocation + first decline
/// + bool slots/probes + adjacent-run scan. The input also visits/collects children.
pub(crate) fn nonconstant_union_work(arms: usize) -> (u64, u64) {
    structural::union(arms)
}

#[test]
fn nonconstant_union_prerequisite_is_hand_counted_independently_of_copy_work() {
    for arms in [2, 3] {
        let control = budget(u64::MAX);
        iq::normalize::normalize_with_work_mode(
            fanout_union(arms),
            CompilerWorkMode::Metered(CompileContext::new(&control)),
        )
        .unwrap();
        assert_eq!(
            control.consumed(QueryCharge::CompilerWork),
            nonconstant_union_work(arms).0
        );
    }
}

fn assert_control_error(error: Error, expected: QueryControlError) {
    match error {
        Error::QueryControl(actual) => assert_eq!(actual, expected),
        other => panic!("expected query-control error, got {other:?}"),
    }
}

fn scan_leaf(alias: usize, source: &str) -> IqNode {
    IqNode::Extensional {
        scan: Scan {
            alias,
            source: (LogicalSource::Query(source.to_owned())).into(),
        },
        bind: BTreeMap::new(),
    }
}

fn conditions_with_nested_exists(marker: &str) -> (Vec<IqCond>, usize) {
    let marker = marker.to_owned();
    let pointer = marker.as_ptr() as usize;
    (
        vec![
            IqCond::Sql(SqlCond::Cmp(ColRef::new(99, "marker"), CmpOp::Eq, marker)),
            IqCond::Exists(Box::new(IqNode::Values {
                vars: vec!["inside".into()],
                rows: vec![vec![Some(TermDef::Const(
                    spargebra::term::NamedNode::new("http://example.test/nested-payload")
                        .unwrap()
                        .into(),
                ))]],
            })),
        ],
        pointer,
    )
}

fn nested_scan(alias: usize, source: &str) -> IqNode {
    IqNode::Filter {
        child: Box::new(scan_leaf(alias, source)),
        cond: conditions_with_nested_exists("nested-node-condition").0,
    }
}

fn scan_parts(node: &IqNode) -> (usize, &str, usize) {
    match node {
        IqNode::Filter { child, .. } => scan_parts(child),
        IqNode::Extensional {
            scan:
                Scan {
                    alias,
                    source: crate::iq::ScanSource::Logical(LogicalSource::Query(source)),
                },
            ..
        } => (*alias, source, source.as_ptr() as usize),
        other => panic!("expected a nested or bare query-backed scan, got {other:?}"),
    }
}

fn marker_pointer(conditions: &[IqCond]) -> usize {
    let Some(IqCond::Sql(SqlCond::Cmp(column, CmpOp::Eq, marker))) = conditions.first() else {
        panic!("expected the marker condition first: {conditions:?}")
    };
    assert_eq!(column.alias, 99);
    assert_eq!(column.column.as_ref(), "marker");
    marker.as_ptr() as usize
}

fn fanout_union(arm_count: usize) -> IqNode {
    let mut children = vec![
        scan_leaf(3, "duplicate-arm"),
        scan_leaf(1, "middle-arm"),
        scan_leaf(3, "duplicate-arm"),
    ];
    children.truncate(arm_count);
    IqNode::Union {
        children,
        project: Vec::new(),
    }
}

fn inner_fixture(arm_count: usize) -> InnerFixture {
    let fixed_left = nested_scan(10, "owned-inner-left");
    let owned_fixed_pointer = scan_parts(&fixed_left).2;
    let fixed = [fixed_left, scan_leaf(20, "fixed-inner-right")];
    let fixed_work = measure_copy_collection(CompilerCloneCollectionV1::IqNodes(&fixed))
        .unwrap()
        .total_work;
    let [fixed_left, fixed_right] = fixed;
    let (condition, owned_condition_pointer) =
        conditions_with_nested_exists("owned-inner-condition");
    let condition_cost =
        measure_copy_collection(CompilerCloneCollectionV1::IqConditions(&condition)).unwrap();

    InnerFixture {
        tree: IqNode::InnerJoin {
            children: vec![fixed_left, fanout_union(arm_count), fixed_right],
            cond: condition,
        },
        fixed_work,
        condition_work: condition_cost.total_work,
        condition_measurement: condition_cost.measurement_work,
        owned_fixed_pointer,
        owned_condition_pointer,
    }
}

fn left_fixture(arm_count: usize) -> LeftFixture {
    let right = nested_scan(20, "owned-left-join-right");
    let owned_right_pointer = scan_parts(&right).2;
    let right_work = measure_copy_root(CompilerCloneRootV1::IqNode(&right))
        .unwrap()
        .total_work;
    let fake_collection_work = measure_copy_collection(CompilerCloneCollectionV1::IqNodes(
        std::slice::from_ref(&right),
    ))
    .unwrap()
    .total_work;
    let (condition, owned_condition_pointer) =
        conditions_with_nested_exists("owned-left-join-condition");
    let condition_cost =
        measure_copy_collection(CompilerCloneCollectionV1::IqConditions(&condition)).unwrap();

    LeftFixture {
        tree: IqNode::LeftJoin {
            left: Box::new(fanout_union(arm_count)),
            right: Box::new(right),
            cond: condition,
        },
        right_work,
        fake_collection_work,
        condition_work: condition_cost.total_work,
        condition_measurement: condition_cost.measurement_work,
        owned_right_pointer,
        owned_condition_pointer,
    }
}

#[test]
fn exact_iq_node_collection_clone_accepts_n_and_rejects_n_minus_one() {
    let source = vec![
        nested_scan(51, "collection-nested"),
        scan_leaf(52, "collection-tail"),
    ];
    let measure = measure_copy_collection(CompilerCloneCollectionV1::IqNodes(&source)).unwrap();
    let source_allocation = source.as_ptr();
    let exact = budget(measure.total_work);

    let cloned = CompileContext::new(&exact).clone_iq_nodes(&source).unwrap();

    assert_eq!(format!("{cloned:?}"), format!("{source:?}"));
    assert_ne!(cloned.as_ptr(), source_allocation);
    assert_eq!(
        exact.consumed(QueryCharge::CompilerWork),
        measure.total_work
    );

    let short = budget(measure.total_work - 1);
    assert_control_error(
        CompileContext::new(&short)
            .clone_iq_nodes(&source)
            .expect_err("N-1 must reject before the IQ-node collection clone"),
        QueryControlError::CompilerWorkExceeded,
    );
    assert_eq!(source.as_ptr(), source_allocation);
    assert_eq!(
        short.consumed(QueryCharge::CompilerWork),
        measure.measurement_work
    );
}

#[test]
fn exact_scalar_iq_node_clone_does_not_charge_a_fake_collection_slot() {
    let source = nested_scan(61, "scalar-nested");
    let source_payload = scan_parts(&source).2;
    let scalar = measure_copy_root(CompilerCloneRootV1::IqNode(&source)).unwrap();
    let collection = measure_copy_collection(CompilerCloneCollectionV1::IqNodes(
        std::slice::from_ref(&source),
    ))
    .unwrap();
    let exact = budget(scalar.total_work);

    let cloned = CompileContext::new(&exact).clone_iq_node(&source).unwrap();

    assert_eq!(collection.deep_clone_work, scalar.deep_clone_work + 1);
    assert_eq!(format!("{cloned:?}"), format!("{source:?}"));
    assert_ne!(scan_parts(&cloned).2, source_payload);
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), scalar.total_work);

    let short = budget(scalar.total_work - 1);
    assert_control_error(
        CompileContext::new(&short)
            .clone_iq_node(&source)
            .expect_err("N-1 must reject before the scalar IQ-node clone"),
        QueryControlError::CompilerWorkExceeded,
    );
    assert_eq!(
        short.consumed(QueryCharge::CompilerWork),
        scalar.measurement_work
    );
}

#[test]
fn inner_join_three_arm_fanout_charges_every_copy_and_matches_raw() {
    let fixture = inner_fixture(3);
    let per_arm = fixture.fixed_work + fixture.condition_work;
    let expected = structural::inner(3).0 + per_arm * 2;
    let budget = budget(expected);
    let raw = iq::normalize::normalize(fixture.tree.clone()).unwrap();

    let metered = iq::normalize::normalize_with_work_mode(
        fixture.tree,
        CompilerWorkMode::Metered(CompileContext::new(&budget)),
    )
    .unwrap();

    assert_eq!(format!("{metered:?}"), format!("{raw:?}"));
    assert_eq!(budget.consumed(QueryCharge::CompilerWork), expected);
    let IqNode::Union { children, .. } = metered else {
        panic!("inner join must distribute over the Union")
    };
    assert_eq!(children.len(), 3, "duplicate arms retain multiplicity");
    for (index, (arm, union_alias)) in children.iter().zip([3, 1, 3]).enumerate() {
        let IqNode::InnerJoin {
            children: operands,
            cond,
        } = arm
        else {
            panic!("expected a distributed InnerJoin arm: {arm:?}")
        };
        assert_eq!(
            operands
                .iter()
                .map(|operand| scan_parts(operand).0)
                .collect::<Vec<_>>(),
            [10, union_alias, 20]
        );
        assert_eq!(
            scan_parts(&operands[0]).2 == fixture.owned_fixed_pointer,
            index == 2
        );
        assert_eq!(
            marker_pointer(cond) == fixture.owned_condition_pointer,
            index == 2
        );
    }
}

#[test]
fn inner_join_condition_rejection_keeps_the_fixed_collection_charge() {
    let fixture = inner_fixture(3);
    let per_arm = fixture.fixed_work + fixture.condition_work;
    let (_, base, insertion, arm) = structural::inner(3);
    let prefix = base + (1 + per_arm + insertion + arm) + 1 + insertion;
    let budget = budget(prefix + per_arm - 1);

    assert_control_error(
        iq::normalize::normalize_with_work_mode(
            fixture.tree,
            CompilerWorkMode::Metered(CompileContext::new(&budget)),
        )
        .expect_err("condition N-1 must reject after the fixed collection clone"),
        QueryControlError::CompilerWorkExceeded,
    );

    assert_eq!(
        budget.consumed(QueryCharge::CompilerWork),
        prefix + fixture.fixed_work + fixture.condition_measurement,
        "the first arm and the second fixed collection clone are not refunded"
    );
}

#[test]
fn left_join_three_arm_fanout_uses_scalar_right_charges_and_matches_raw() {
    let fixture = left_fixture(3);
    let per_arm = fixture.right_work + fixture.condition_work;
    let expected = structural::left(3).0 + per_arm * 2;
    let budget = budget(expected);
    let raw = iq::normalize::normalize(fixture.tree.clone()).unwrap();

    let metered = iq::normalize::normalize_with_work_mode(
        fixture.tree,
        CompilerWorkMode::Metered(CompileContext::new(&budget)),
    )
    .unwrap();

    assert_eq!(
        fixture.fake_collection_work,
        fixture.right_work + 3,
        "one synthetic slot plus its collection record/iteration are excluded"
    );
    assert_eq!(format!("{metered:?}"), format!("{raw:?}"));
    assert_eq!(budget.consumed(QueryCharge::CompilerWork), expected);
    let IqNode::Union { children, .. } = metered else {
        panic!("left join must distribute over its left Union")
    };
    assert_eq!(children.len(), 3, "duplicate arms retain multiplicity");
    for (index, (arm, left_alias)) in children.iter().zip([3, 1, 3]).enumerate() {
        let IqNode::LeftJoin { left, right, cond } = arm else {
            panic!("expected a distributed LeftJoin arm: {arm:?}")
        };
        assert_eq!(scan_parts(left).0, left_alias);
        assert_eq!(scan_parts(right).0, 20);
        assert_eq!(
            scan_parts(right).2 == fixture.owned_right_pointer,
            index == 2
        );
        assert_eq!(
            marker_pointer(cond) == fixture.owned_condition_pointer,
            index == 2
        );
    }
}

#[test]
fn left_join_condition_rejection_keeps_the_scalar_right_charge() {
    let fixture = left_fixture(3);
    let per_arm = fixture.right_work + fixture.condition_work;
    let (_, base, arm) = structural::left(3);
    let prefix = base + (1 + per_arm + arm) + 1;
    let budget = budget(prefix + per_arm - 1);

    assert_control_error(
        iq::normalize::normalize_with_work_mode(
            fixture.tree,
            CompilerWorkMode::Metered(CompileContext::new(&budget)),
        )
        .expect_err("condition N-1 must reject after the scalar right clone"),
        QueryControlError::CompilerWorkExceeded,
    );

    assert_eq!(
        budget.consumed(QueryCharge::CompilerWork),
        prefix + fixture.right_work + fixture.condition_measurement,
        "the first arm and the second scalar clone are not refunded"
    );
    assert_eq!(
        budget.checkpoint(),
        Err(QueryControlError::CompilerWorkExceeded)
    );
}
