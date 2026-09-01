use spargebra::algebra::{Expression, OrderExpression};

use crate::iq::OrderKey;

/// Reuse the flat ORDER BY lowering (design §2 / iq.rs [`OrderKey`]): a variable key
/// stores `expr: None`; a complex expression key stores the cloned [`Expression`]
/// under a synthetic `__sf_ord_{n}` variable that exec evaluates before sorting.
pub(super) fn order_keys(expression: &[OrderExpression]) -> Vec<OrderKey> {
    let mut keys = Vec::with_capacity(expression.len());
    for oe in expression {
        let (expr, descending) = match oe {
            OrderExpression::Asc(e) => (e, false),
            OrderExpression::Desc(e) => (e, true),
        };
        match expr {
            Expression::Variable(v) => keys.push(OrderKey {
                var: v.as_str().to_owned(),
                descending,
                expr: None,
            }),
            other => {
                let syn = format!("__sf_ord_{}", keys.len());
                keys.push(OrderKey {
                    var: syn,
                    descending,
                    expr: Some(Box::new(other.clone())),
                });
            }
        }
    }
    keys
}
