use spargebra::algebra::{Expression, OrderExpression};

use super::control::BuildWork;
use crate::iq::OrderKey;
use crate::Result;

/// Reuse the flat ORDER BY lowering (design §2 / iq.rs [`OrderKey`]): a variable key
/// stores `expr: None`; a complex expression key stores the cloned [`Expression`]
/// under an internal name that SPARQL's `VARNAME` grammar cannot express. The
/// impossible NUL prefix prevents a user binding from being overwritten or used
/// as the key when expression evaluation fails.
pub(super) fn order_keys(
    expression: &[OrderExpression],
    work: BuildWork<'_>,
) -> Result<Vec<OrderKey>> {
    let mut keys = work.vector(expression.len())?;
    for oe in expression {
        work.charge(1)?;
        let (expr, descending) = match oe {
            OrderExpression::Asc(e) => (e, false),
            OrderExpression::Desc(e) => (e, true),
        };
        match expr {
            Expression::Variable(v) => keys.push(OrderKey {
                var: work.string(v.as_str())?,
                descending,
                expr: None,
            }),
            other => {
                let digits = keys.len().checked_ilog10().map_or(1, |n| n as usize + 1);
                work.charge(1)?;
                work.charge("\0sf-order-".len() + digits)?;
                let syn = format!("\0sf-order-{}", keys.len());
                work.checkpoint()?;
                keys.push(OrderKey {
                    var: syn,
                    descending,
                    expr: Some(work.boxed(work.copied(other)?)?),
                });
            }
        }
    }
    Ok(keys)
}
