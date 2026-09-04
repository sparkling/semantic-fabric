use std::collections::BTreeMap;

use sf_core::ir::LogicalSource;
use sf_core::{Literal, Term};
use sf_sparql::iq::node::{BindDef, IqNode};
use sf_sparql::iq::normalize::normalize;
use sf_sparql::iq::{Scan, TermDef};

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
