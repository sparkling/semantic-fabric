use sf_core::datatype::XsdTypeCode;
use sf_core::Term;
use spargebra::algebra::{AggregateExpression, AggregateFunction, Expression};
use spargebra::term::Variable;

use super::control::BuildWork;
use crate::iq::node::{AggArg, AggDef};
use crate::iq::{AggKind, TermDef};
use crate::Result;

/// Lower an aggregate-argument expression to a [`TermDef`] (design §2 Group arm;
/// `BIND` is carried symbolically as `BindDef::Expr`, not via this function).
/// Context-free, only a constant IRI/literal is lowerable; a variable / `CONCAT` /
/// arithmetic aggregate argument stays a tracked sound-501 (M3 design §2.3).
fn lower_expr_to_termdef(expr: &Expression, work: BuildWork<'_>) -> Result<TermDef> {
    match expr {
        Expression::NamedNode(n) => Ok(TermDef::Const(Term::NamedNode(work.copied(n)?))),
        Expression::Literal(l) => Ok(TermDef::Const(Term::Literal(work.copied(l)?))),
        other => Err(work.unsupported(
            "BIND/expression resolution needs bound columns (M3) → 501",
            || format!("BIND/expression resolution needs bound columns (M3) → 501: {other:?}"),
        )?),
    }
}

/// Map one `(output-variable, AggregateExpression)` to an [`AggDef`] (design §2
/// Group arm). `COUNT(*)` carries no argument (and rides `distinct` for
/// `COUNT(DISTINCT *)`, design §1); every other set function takes an argument
/// (a bare variable → [`AggArg::Var`], else a lowered constant expression →
/// [`AggArg::Expr`]). `GROUP_CONCAT`/`SAMPLE` need the M6 [`AggKind`] extension that
/// does not exist yet → a tracked sound-501 (do not invent the variant).
pub(super) fn lower_agg_def(
    out: &Variable,
    expr: &AggregateExpression,
    work: BuildWork<'_>,
) -> Result<AggDef> {
    let work = work.enter()?;
    let var = work.variable(out.as_str())?;
    match expr {
        // COUNT(*) / COUNT(DISTINCT *) — no argument column; result xsd:integer.
        AggregateExpression::CountSolutions { distinct } => Ok(AggDef {
            var,
            kind: AggKind::Count,
            arg: None,
            distinct: *distinct,
            fixed_type: Some(XsdTypeCode::Integer),
        }),
        AggregateExpression::FunctionCall {
            name,
            expr,
            distinct,
        } => {
            // COUNT pins xsd:integer; SUM/AVG/MIN/MAX take the value's resolved §10
            // type at reconstruction (None) — mirrors the flat `lower_aggregate`.
            let (kind, fixed_type) = match name {
                AggregateFunction::Count => (AggKind::Count, Some(XsdTypeCode::Integer)),
                AggregateFunction::Sum => (AggKind::Sum, None),
                AggregateFunction::Avg => (AggKind::Avg, None),
                AggregateFunction::Min => (AggKind::Min, None),
                AggregateFunction::Max => (AggKind::Max, None),
                AggregateFunction::GroupConcat { .. } => {
                    return Err(work.unsupported(
                        "GROUP_CONCAT needs AggKind::GroupConcat (M6) → 501",
                        || "GROUP_CONCAT needs AggKind::GroupConcat (M6) → 501".to_owned(),
                    )?)
                }
                AggregateFunction::Sample => {
                    return Err(work
                        .unsupported("SAMPLE needs AggKind::Sample (M6) → 501", || {
                            "SAMPLE needs AggKind::Sample (M6) → 501".to_owned()
                        })?)
                }
                AggregateFunction::Custom(_) => {
                    return Err(work.unsupported("custom aggregate function → 501", || {
                        "custom aggregate function → 501".to_owned()
                    })?)
                }
            };
            Ok(AggDef {
                var,
                kind,
                arg: Some(lower_agg_arg(expr, work)?),
                distinct: *distinct,
                fixed_type,
            })
        }
    }
}

/// An aggregate's argument: a bare variable → [`AggArg::Var`] (resolved
/// context-free — it is only a name); any other expression → [`AggArg::Expr`] over
/// a lowered [`TermDef`] (so a constant lowers; `SUM(?a + ?b)` defers).
fn lower_agg_arg(expr: &Expression, work: BuildWork<'_>) -> Result<AggArg> {
    let work = work.enter()?;
    match expr {
        Expression::Variable(v) => Ok(AggArg::Var(work.variable(v.as_str())?)),
        other => Ok(AggArg::Expr(lower_expr_to_termdef(other, work)?)),
    }
}
