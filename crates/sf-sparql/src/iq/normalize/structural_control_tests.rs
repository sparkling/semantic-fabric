use std::collections::BTreeMap;

use sf_core::query_control::{QueryBudget, QueryControlError, QueryLimits};

use super::normalize_with_work_mode;
use super::structural_test_support::*;
use crate::compiler_control::CompileContext;
use crate::iq::node::IqNode;
use crate::{CompilerWorkMode, Error};

fn rejects_unpaid(node: IqNode) {
    let control = QueryBudget::new(QueryLimits::new(0, u64::MAX, u64::MAX, u64::MAX));
    assert!(matches!(
        normalize_with_work_mode(
            node,
            CompilerWorkMode::Metered(CompileContext::new(&control))
        ),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
}

#[test]
fn root_visit_rejects_unpaid_work_even_for_identity() {
    rejects_unpaid(IqNode::True);
}

#[test]
fn empty_absorbing_join_rejects_unpaid_scope_walk() {
    rejects_unpaid(IqNode::InnerJoin {
        children: vec![
            IqNode::True,
            IqNode::Empty {
                vars: vec!["x".into()],
            },
        ],
        cond: vec![],
    });
}

#[test]
fn construction_composition_rejects_unpaid_work() {
    let projection = |child| IqNode::Construction {
        child: Box::new(child),
        subst: BTreeMap::new(),
        project: vec!["x".into()],
    };
    rejects_unpaid(projection(projection(IqNode::True)));
}

fn fixtures() -> Vec<IqNode> {
    let v = IqNode::Values {
        vars: vec!["東京".into(), "x".into(), "東京".into()],
        rows: vec![],
    };
    vec![
        IqNode::True, v.clone(), data(1, &["é", "x"]), union(),
        IqNode::Empty { vars: vec!["x".into(), "x".into()] },
        IqNode::InnerJoin { children: vec![v.clone(), data(1, &["y"]), IqNode::Empty { vars: vec!["last".into()] }], cond: vec![] },
        IqNode::InnerJoin { children: vec![IqNode::True, IqNode::True], cond: vec![truth()] },
        IqNode::InnerJoin { children: vec![union(), data(9, &["fixed"]), union()], cond: vec![truth()] },
        IqNode::LeftJoin { left: Box::new(v.clone()), right: Box::new(union()), cond: vec![truth()] },
        IqNode::LeftJoin { left: Box::new(union()), right: Box::new(v), cond: vec![] },
        IqNode::OrderBy { child: Box::new(union()), keys: vec![] },
        IqNode::Aggregation { child: Box::new(union()), grouping: vec!["x".into()], aggs: vec![] },
        resolved("SELECT ?s WHERE { ?s <http://ex/p>+ ?o }"),
        resolved("SELECT (COUNT(?s) AS ?n) WHERE { ?s <http://ex/p> ?o } GROUP BY ?o ORDER BY ?n LIMIT 3"),
    ]
}

#[test]
fn structural_shapes_preserve_raw_output_with_exact_and_short_work() {
    for tree in fixtures() {
        exact_and_short(tree);
    }
}

#[test]
fn every_structural_charge_observes_sticky_cancellation_and_deadline() {
    for tree in fixtures() {
        every_stop(tree);
    }
}

#[test]
fn scope_keeps_all_children_before_empty_absorption_and_optional_right_union() {
    let tree = IqNode::InnerJoin {
        children: vec![
            IqNode::Empty {
                vars: vec!["first".into()],
            },
            data(0, &["z", "a"]),
            IqNode::Values {
                vars: vec!["last".into(), "a".into()],
                rows: vec![],
            },
        ],
        cond: vec![],
    };
    let result = run(tree, &budget(u64::MAX)).unwrap();
    assert!(
        matches!(result, IqNode::Empty { vars } if vars == ["first", "a", "z", "last"].map(Into::into))
    );
    let optional = IqNode::LeftJoin {
        left: Box::new(data(0, &["left"])),
        right: Box::new(union()),
        cond: vec![],
    };
    assert!(
        matches!(run(optional, &budget(u64::MAX)).unwrap(), IqNode::LeftJoin { right, .. } if matches!(*right, IqNode::Union { .. }))
    );
}

#[test]
fn normalized_identity_charges_exactly_one_visit() {
    use sf_core::query_control::QueryCharge;
    let c = budget(1);
    assert!(matches!(run(IqNode::True, &c).unwrap(), IqNode::True));
    assert_eq!(c.consumed(QueryCharge::CompilerWork), 1);
}

#[test]
fn structural_depth_envelope_holds_on_the_default_test_stack() {
    let max = crate::compile_envelope::algebra::MAX_ALGEBRA_DEPTH_V1;
    for shape in 0..5 {
        for depth in [max, max + 1] {
            let mut tree = IqNode::True;
            for _ in 1..depth {
                tree = match shape {
                    0 => empty_projection(tree),
                    1 => IqNode::OrderBy {
                        child: Box::new(tree),
                        keys: vec![],
                    },
                    2 => IqNode::LeftJoin {
                        left: Box::new(tree),
                        right: Box::new(IqNode::True),
                        cond: vec![],
                    },
                    3 => filter(tree, vec![]),
                    _ => IqNode::InnerJoin {
                        children: vec![tree, IqNode::True],
                        cond: vec![],
                    },
                };
            }
            let result = run(tree, &budget(u64::MAX));
            if depth == max {
                assert!(result.is_ok(), "shape {shape}: {result:?}");
            } else {
                assert!(
                    matches!(
                        result,
                        Err(Error::QueryControl(
                            QueryControlError::CompilerEnvelopeExceeded
                        ))
                    ),
                    "shape {shape}: {result:?}"
                );
            }
        }
    }
}
