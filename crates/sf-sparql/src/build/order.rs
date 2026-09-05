use spargebra::algebra::{Expression, OrderExpression};

use crate::iq::OrderKey;

/// Reuse the flat ORDER BY lowering (design §2 / iq.rs [`OrderKey`]): a variable key
/// stores `expr: None`; a complex expression key stores the cloned [`Expression`]
/// under an internal name that SPARQL's `VARNAME` grammar cannot express. The
/// impossible NUL prefix prevents a user binding from being overwritten or used
/// as the key when expression evaluation fails.
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
                let syn = format!("\0sf-order-{}", keys.len());
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
