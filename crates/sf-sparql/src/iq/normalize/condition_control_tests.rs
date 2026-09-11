use super::structural_test_support::*;
use crate::iq::node::{IqCond, IqNode};

#[test]
fn condition_depth_shares_the_node_envelope_on_the_default_stack() {
    use crate::Error;
    use sf_core::query_control::QueryControlError;
    let max = crate::compile_envelope::algebra::MAX_ALGEBRA_DEPTH_V1;
    for shape in 0..3 {
        for depth in [max, max + 1] {
            let mut cond = truth();
            // The Filter root is depth one; its first condition is depth two.
            for _ in 2..depth {
                cond = match shape {
                    0 => IqCond::Not(Box::new(cond)),
                    1 => IqCond::And(vec![cond]),
                    _ => IqCond::Or(vec![cond]),
                };
            }
            let result = run(filter(data(0, &[]), vec![cond]), &budget(u64::MAX));
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

#[test]
fn nested_conditions_and_exists_keep_order_and_control_at_every_step() {
    let conds = vec![
        truth(),
        IqCond::And(vec![
            IqCond::Exists(Box::new(union())),
            IqCond::Not(Box::new(truth())),
        ]),
        IqCond::Or(vec![
            IqCond::NotExists {
                inner: Box::new(data(4, &["x"])),
                is_minus: false,
            },
            IqCond::NotExists {
                inner: Box::new(union()),
                is_minus: true,
            },
        ]),
        IqCond::Expr(Box::new(spargebra::algebra::Expression::Literal(
            sf_core::Literal::from(true),
        ))),
    ];
    for tree in [
        filter(filter(data(1, &["x"]), vec![truth()]), conds.clone()),
        filter(
            construction(
                filter(data(1, &["x"]), vec![truth()]),
                &[("x", value("first"))],
            ),
            conds.clone(),
        ),
        filter(union(), conds.clone()),
        IqNode::InnerJoin {
            children: vec![IqNode::True, IqNode::True],
            cond: conds,
        },
    ] {
        exact_and_short(tree.clone());
        every_stop(tree);
    }
}
