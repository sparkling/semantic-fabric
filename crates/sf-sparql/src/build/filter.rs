use spargebra::algebra::Expression;
use spargebra::term::NamedNodePattern;

use crate::iq::node::IqCond;
use crate::Result;

use super::build_tree;

/// Lower a SPARQL FILTER / OPTIONAL ON-expression to a conjunction of [`IqCond`]s,
/// splitting a top-level `&&` into independent conjuncts (design §2 Filter arm; §9
/// `IqCond` amendment). It mirrors the *Expression coverage* of the flat
/// [`crate::unfold::Unfolder::lower_filter_expr`]: `EXISTS`/`NOT EXISTS` build a
/// first-class subtree (the case the flat `SqlCond` cannot carry before lowering,
/// §9); `||`/`!` compose via [`IqCond::Or`]/[`IqCond::Not`]. A pushable leaf
/// (comparison/`BOUND`/`REGEX`/string match) needs the bound-column + dialect
/// resolution that the context-free builder lacks → a tracked sound-501 (M3); any
/// expression the flat model would itself 501 propagates the same 501.
pub(super) fn lower_filter_to_iqconds(
    expr: &Expression,
    current_graph: Option<&NamedNodePattern>,
) -> Result<Vec<IqCond>> {
    let mut out = Vec::new();
    collect_conjuncts(expr, current_graph, &mut out)?;
    Ok(out)
}

/// Flatten a top-level `&&` chain into independent conjuncts, lowering each.
fn collect_conjuncts(
    expr: &Expression,
    current_graph: Option<&NamedNodePattern>,
    out: &mut Vec<IqCond>,
) -> Result<()> {
    match expr {
        Expression::And(a, b) => {
            collect_conjuncts(a, current_graph, out)?;
            collect_conjuncts(b, current_graph, out)
        }
        other => {
            out.push(lower_iqcond(other, current_graph)?);
            Ok(())
        }
    }
}

/// Lower a single (non-top-level-`&&`) FILTER expression to one [`IqCond`].
fn lower_iqcond(expr: &Expression, current_graph: Option<&NamedNodePattern>) -> Result<IqCond> {
    match expr {
        Expression::Exists(p) => Ok(IqCond::Exists(Box::new(build_tree(p, current_graph)?))),
        Expression::Not(inner) => match inner.as_ref() {
            Expression::Exists(p) => Ok(IqCond::NotExists {
                inner: Box::new(build_tree(p, current_graph)?),
                is_minus: false,
            }),
            other => Ok(IqCond::Not(Box::new(lower_iqcond(other, current_graph)?))),
        },
        Expression::And(a, b) => Ok(IqCond::And(vec![
            lower_iqcond(a, current_graph)?,
            lower_iqcond(b, current_graph)?,
        ])),
        Expression::Or(a, b) => Ok(IqCond::Or(vec![
            lower_iqcond(a, current_graph)?,
            lower_iqcond(b, current_graph)?,
        ])),
        // A pushable leaf is carried SYMBOLIC (IqCond::Expr) and resolved to a SqlCond
        // per leaf-CQ at LOWER via the flat lower_filter_expr (M3 design §2.1): a FILTER
        // above a Union has no single column for a variable until the union is split.
        other => Ok(IqCond::Expr(Box::new(other.clone()))),
    }
}
