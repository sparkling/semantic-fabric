use spargebra::algebra::Expression;
use spargebra::term::NamedNodePattern;

use crate::iq::node::IqCond;
use crate::Result;

use super::build;
use super::control::{BuildVec, BuildWork};

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
    work: BuildWork<'_>,
) -> Result<Vec<IqCond>> {
    let mut out = BuildVec::new(Vec::new());
    collect_conjuncts(expr, current_graph, &mut out, work)?;
    Ok(out.into_inner())
}

/// Flatten a top-level `&&` chain into independent conjuncts, lowering each.
fn collect_conjuncts(
    expr: &Expression,
    current_graph: Option<&NamedNodePattern>,
    out: &mut BuildVec<IqCond>,
    work: BuildWork<'_>,
) -> Result<()> {
    match expr {
        Expression::And(a, b) => {
            let work = work.enter()?;
            collect_conjuncts(a, current_graph, out, work)?;
            collect_conjuncts(b, current_graph, out, work)
        }
        other => work.push(out, lower_iqcond(other, current_graph, work)?),
    }
}

/// Lower a single (non-top-level-`&&`) FILTER expression to one [`IqCond`].
fn lower_iqcond(
    expr: &Expression,
    current_graph: Option<&NamedNodePattern>,
    work: BuildWork<'_>,
) -> Result<IqCond> {
    let work = work.enter()?;
    match expr {
        Expression::Exists(p) => Ok(IqCond::Exists(work.boxed(build(
            p,
            current_graph,
            work,
        )?)?)),
        Expression::Not(inner) => match inner.as_ref() {
            Expression::Exists(p) => Ok(IqCond::NotExists {
                inner: work.boxed(build(p, current_graph, work.enter()?)?)?,
                is_minus: false,
            }),
            other => Ok(IqCond::Not(work.boxed(lower_iqcond(
                other,
                current_graph,
                work,
            )?)?)),
        },
        Expression::And(a, b) | Expression::Or(a, b) => {
            let mut children = work.vector(2)?;
            children.push(lower_iqcond(a, current_graph, work)?);
            children.push(lower_iqcond(b, current_graph, work)?);
            Ok(if matches!(expr, Expression::And(..)) {
                IqCond::And(children)
            } else {
                IqCond::Or(children)
            })
        }
        // A pushable leaf is carried SYMBOLIC (IqCond::Expr) and resolved to a SqlCond
        // per leaf-CQ at LOWER via the flat lower_filter_expr (M3 design §2.1): a FILTER
        // above a Union has no single column for a variable until the union is split.
        other => Ok(IqCond::Expr(work.boxed(work.copied(other)?)?)),
    }
}
