use std::collections::BTreeMap;

use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_core::{Literal, Term};
use sf_sparql::iq::node::{BindDef, IqCond, IqNode};
use sf_sparql::iq::normalize::normalize;
use sf_sparql::iq::{CmpOp, ColRef, Scan, SqlCond, TermDef};

fn literal_cell(value: &str) -> (Option<TermDef>, usize) {
    let literal = Literal::new_simple_literal(value.to_owned());
    let pointer = literal.value().as_ptr() as usize;
    (Some(TermDef::Const(Term::Literal(literal))), pointer)
}

fn literal_parts(cell: &Option<TermDef>) -> (&str, usize) {
    let Some(TermDef::Const(Term::Literal(literal))) = cell else {
        panic!("expected a constant literal cell: {cell:?}")
    };
    (literal.value(), literal.value().as_ptr() as usize)
}

fn constant_arm(value: &str) -> (IqNode, usize) {
    let (cell, pointer) = literal_cell(value);
    let mut subst = BTreeMap::new();
    subst.insert(
        "x".into(),
        BindDef::Resolved(cell.expect("helper always returns a bound cell")),
    );
    (
        IqNode::Construction {
            child: Box::new(IqNode::True),
            subst,
            project: vec!["x".into()],
        },
        pointer,
    )
}

fn data_arm(source: String) -> (IqNode, usize) {
    let pointer = source.as_ptr() as usize;
    (
        IqNode::Extensional {
            scan: Scan {
                alias: 17,
                source: LogicalSource::Query(source),
            },
            bind: BTreeMap::new(),
        },
        pointer,
    )
}

fn query_source_parts(node: &IqNode) -> (&str, usize) {
    let IqNode::Extensional {
        scan: Scan {
            source: LogicalSource::Query(source),
            ..
        },
        ..
    } = node
    else {
        panic!("expected an extensional SQL-query arm: {node:?}")
    };
    (source, source.as_ptr() as usize)
}

fn scan_leaf(alias: usize, source: &str) -> IqNode {
    IqNode::Extensional {
        scan: Scan {
            alias,
            source: LogicalSource::Table(source.to_owned()),
        },
        bind: BTreeMap::new(),
    }
}

fn scan_parts(node: &IqNode) -> (usize, &str, usize) {
    let IqNode::Extensional {
        scan: Scan {
            alias,
            source: LogicalSource::Table(source),
        },
        ..
    } = node
    else {
        panic!("expected a table-backed extensional scan: {node:?}")
    };
    (*alias, source, source.as_ptr() as usize)
}

fn fanout_union() -> IqNode {
    IqNode::Union {
        children: vec![
            scan_leaf(3, "first"),
            scan_leaf(1, "middle"),
            scan_leaf(3, "duplicate-alias"),
        ],
        project: Vec::new(),
    }
}

fn marker_condition(value: &str) -> (Vec<IqCond>, usize) {
    let value = value.to_owned();
    let pointer = value.as_ptr() as usize;
    (
        vec![IqCond::Sql(SqlCond::Cmp(
            ColRef::new(99, "marker"),
            CmpOp::Eq,
            value,
        ))],
        pointer,
    )
}

fn marker_pointer(conditions: &[IqCond]) -> usize {
    match conditions {
        [IqCond::Sql(SqlCond::Cmp(column, CmpOp::Eq, value))]
            if column.alias == 99 && column.column.as_ref() == "marker" =>
        {
            value.as_ptr() as usize
        }
        other => panic!("expected the marker condition: {other:?}"),
    }
}

#[test]
fn full_constant_fold_moves_rows_and_reordered_cells() {
    let (x1, x1_pointer) = literal_cell("first-x-owned-allocation");
    let (y1, y1_pointer) = literal_cell("first-y-owned-allocation");
    let (y2, y2_pointer) = literal_cell("second-y-owned-allocation");
    let (x2, x2_pointer) = literal_cell("second-x-owned-allocation");
    let tree = IqNode::Union {
        children: vec![
            IqNode::Values {
                vars: vec!["x".into(), "y".into()],
                rows: vec![vec![x1, y1]],
            },
            IqNode::Values {
                vars: vec!["y".into(), "x".into()],
                rows: vec![vec![y2, x2]],
            },
        ],
        project: vec!["x".into(), "y".into()],
    };

    let IqNode::Values { vars, rows } = normalize(tree).unwrap() else {
        panic!("constant arms must fold to one VALUES node")
    };
    assert_eq!(vars, vec!["x".into(), "y".into()]);
    assert_eq!(rows.len(), 2);
    assert_eq!(
        literal_parts(&rows[0][0]),
        ("first-x-owned-allocation", x1_pointer)
    );
    assert_eq!(
        literal_parts(&rows[0][1]),
        ("first-y-owned-allocation", y1_pointer)
    );
    assert_eq!(
        literal_parts(&rows[1][0]),
        ("second-x-owned-allocation", x2_pointer)
    );
    assert_eq!(
        literal_parts(&rows[1][1]),
        ("second-y-owned-allocation", y2_pointer)
    );
}

#[test]
fn partial_fold_moves_the_lone_data_arm_and_constant_cells() {
    let source = "SELECT a_very_long_column_name FROM a_very_long_table_name";
    let (data, source_pointer) = data_arm(source.to_owned());
    let (constant_a, a_pointer) = constant_arm("constant-a-owned-allocation");
    let (constant_b, b_pointer) = constant_arm("constant-b-owned-allocation");
    let tree = IqNode::Union {
        children: vec![data, constant_a, constant_b],
        project: vec!["x".into()],
    };

    let IqNode::Union { children, .. } = normalize(tree).unwrap() else {
        panic!("the data arm and folded VALUES arm must remain a union")
    };
    assert_eq!(children.len(), 2);
    assert_eq!(query_source_parts(&children[0]), (source, source_pointer));
    let IqNode::Values { rows, .. } = &children[1] else {
        panic!("the two constant arms must fold in their original position")
    };
    assert_eq!(literal_parts(&rows[0][0]).1, a_pointer);
    assert_eq!(literal_parts(&rows[1][0]).1, b_pointer);
}

#[test]
fn slice_moves_wrapped_values_survivors_and_the_untouched_tail() {
    let (dropped, _) = literal_cell("dropped-owned-allocation");
    let (kept_b, b_pointer) = literal_cell("kept-b-owned-allocation");
    let (kept_c, c_pointer) = literal_cell("kept-c-owned-allocation");
    let source = "SELECT another_long_column FROM another_long_table";
    let (data, source_pointer) = data_arm(source.to_owned());
    let values = IqNode::Construction {
        child: Box::new(IqNode::Values {
            vars: vec!["x".into()],
            rows: vec![vec![dropped], vec![kept_b], vec![kept_c]],
        }),
        subst: BTreeMap::new(),
        project: vec!["x".into()],
    };
    let tree = IqNode::Slice {
        child: Box::new(IqNode::Union {
            children: vec![values, data],
            project: vec!["x".into()],
        }),
        offset: 1,
        limit: Some(5),
    };

    let IqNode::Slice { child, offset, .. } = normalize(tree).unwrap() else {
        panic!("the unknown data tail requires a residual slice")
    };
    assert_eq!(offset, 0);
    let IqNode::Union { children, .. } = *child else {
        panic!("surviving VALUES rows must precede the data arm")
    };
    let IqNode::Values { rows, .. } = &children[0] else {
        panic!("the first arm must contain the surviving VALUES rows")
    };
    assert_eq!(literal_parts(&rows[0][0]).1, b_pointer);
    assert_eq!(literal_parts(&rows[1][0]).1, c_pointer);
    assert_eq!(query_source_parts(&children[1]), (source, source_pointer));
}

#[test]
fn distinct_keeps_first_occurrences_without_cloning_survivor_rows() {
    let (first_a, first_a_pointer) = literal_cell("dedup-a-owned-allocation");
    let (duplicate_a, _) = literal_cell("dedup-a-owned-allocation");
    let (first_b, first_b_pointer) = literal_cell("dedup-b-owned-allocation");
    let tree = IqNode::Distinct {
        child: Box::new(IqNode::Values {
            vars: vec!["x".into()],
            rows: vec![vec![first_a], vec![duplicate_a], vec![first_b]],
        }),
    };

    let IqNode::Values { rows, .. } = normalize(tree).unwrap() else {
        panic!("plain constant rows should deduplicate in place")
    };
    assert_eq!(rows.len(), 2);
    assert_eq!(
        literal_parts(&rows[0][0]),
        ("dedup-a-owned-allocation", first_a_pointer)
    );
    assert_eq!(
        literal_parts(&rows[1][0]),
        ("dedup-b-owned-allocation", first_b_pointer)
    );
}

#[test]
fn construction_fanout_moves_payload_to_final_arm_in_exact_bag_order() {
    let column: Box<str> = "owned-construction-column".into();
    let owned_column = column.as_ptr() as usize;
    let mut subst = BTreeMap::new();
    subst.insert(
        "payload".into(),
        BindDef::Resolved(TermDef::Derived {
            term_map: TermMap::Column(column, TermSpec::plain_literal()),
            alias: 99,
        }),
    );
    let tree = IqNode::Construction {
        child: Box::new(fanout_union()),
        subst,
        project: vec!["payload".into()],
    };

    let IqNode::Union { children, project } = normalize(tree).unwrap() else {
        panic!("construction fan-out must retain the union")
    };
    assert_eq!(project, vec!["payload".into()]);
    assert_eq!(children.len(), 3, "duplicate arms retain bag multiplicity");
    for (index, (arm, expected_alias)) in children.iter().zip([3, 1, 3]).enumerate() {
        let IqNode::Construction {
            child,
            subst,
            project,
        } = arm
        else {
            panic!("every arm must carry the construction: {arm:?}")
        };
        assert_eq!(scan_parts(child).0, expected_alias);
        assert_eq!(project, &vec!["payload".into()]);
        let BindDef::Resolved(TermDef::Derived {
            term_map: TermMap::Column(column, _),
            alias: 99,
        }) = subst.get("payload").expect("distributed payload")
        else {
            panic!("distributed payload changed shape: {subst:?}")
        };
        assert_eq!(
            column.as_ptr() as usize == owned_column,
            index == 2,
            "only the final arm receives the original substitution"
        );
    }
}

#[test]
fn inner_join_fanout_moves_fixed_inputs_to_final_arm_in_exact_order() {
    let fixed_left = scan_leaf(10, "owned-inner-left");
    let owned_left_source = scan_parts(&fixed_left).2;
    let (condition, owned_marker) = marker_condition("owned-inner-condition");
    let tree = IqNode::InnerJoin {
        children: vec![fixed_left, fanout_union(), scan_leaf(20, "fixed-right")],
        cond: condition,
    };

    let IqNode::Union { children, .. } = normalize(tree).unwrap() else {
        panic!("inner join over union must distribute")
    };
    assert_eq!(children.len(), 3, "duplicate arms retain bag multiplicity");
    for (index, (arm, distributed_alias)) in children.iter().zip([3, 1, 3]).enumerate() {
        let IqNode::InnerJoin {
            children: operands,
            cond,
        } = arm
        else {
            panic!("every distributed arm must remain an inner join: {arm:?}")
        };
        assert_eq!(
            operands
                .iter()
                .map(|node| scan_parts(node).0)
                .collect::<Vec<_>>(),
            [10, distributed_alias, 20],
            "fixed operand and union-arm order is exact"
        );
        assert_eq!(scan_parts(&operands[0]).2 == owned_left_source, index == 2);
        assert_eq!(marker_pointer(cond) == owned_marker, index == 2);
    }
}

#[test]
fn filter_fanout_moves_condition_to_final_arm_without_losing_duplicates() {
    let (condition, owned_marker) = marker_condition("owned-filter-condition");
    let tree = IqNode::Filter {
        child: Box::new(fanout_union()),
        cond: condition,
    };

    let IqNode::Union { children, .. } = normalize(tree).unwrap() else {
        panic!("filter over union must distribute")
    };
    assert_eq!(children.len(), 3, "duplicate arms retain bag multiplicity");
    for (index, (arm, expected_alias)) in children.iter().zip([3, 1, 3]).enumerate() {
        let IqNode::Filter { child, cond } = arm else {
            panic!("every distributed arm must retain the filter: {arm:?}")
        };
        assert_eq!(scan_parts(child).0, expected_alias);
        assert_eq!(marker_pointer(cond) == owned_marker, index == 2);
    }
}

#[test]
fn left_join_fanout_moves_shared_right_and_condition_to_final_left_arm() {
    let right = scan_leaf(20, "owned-left-join-right");
    let owned_right_source = scan_parts(&right).2;
    let (condition, owned_marker) = marker_condition("owned-left-join-condition");
    let tree = IqNode::LeftJoin {
        left: Box::new(fanout_union()),
        right: Box::new(right),
        cond: condition,
    };

    let IqNode::Union { children, .. } = normalize(tree).unwrap() else {
        panic!("left join over its preserved union side must distribute")
    };
    assert_eq!(children.len(), 3, "duplicate arms retain bag multiplicity");
    for (index, (arm, expected_alias)) in children.iter().zip([3, 1, 3]).enumerate() {
        let IqNode::LeftJoin { left, right, cond } = arm else {
            panic!("every distributed arm must retain the left join: {arm:?}")
        };
        assert_eq!(scan_parts(left).0, expected_alias);
        assert_eq!(scan_parts(right).0, 20);
        assert_eq!(scan_parts(right).2 == owned_right_source, index == 2);
        assert_eq!(marker_pointer(cond) == owned_marker, index == 2);
    }
}
