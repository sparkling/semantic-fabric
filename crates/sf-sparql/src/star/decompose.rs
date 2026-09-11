//! The two composed-variable mint sites outside an ordinary BGP triple
//! pattern (`super::walk::rewrite_triple`'s own reifies-bare-variable case is
//! the third): a `VALUES` column carrying a ground triple term
//! ([`rewrite_values`]/[`decompose_column`], rule R6) and a
//! `BIND(TRIPLE(e1,e2,e3) AS ?v)` target ([`rewrite_extend`]/
//! [`rewrite_extend_inner`], rule R5a's `Extend` case plus ADR-0032 D3 item
//! 3). Both ultimately register their variable via
//! [`super::env::composed_info_for`], the same lookup-before-mint entry point
//! `rewrite_triple` uses, so a variable composed from two different
//! syntactic positions in one query still gets one shared set of component
//! vars.

use spargebra::algebra::{Expression, Function, GraphPattern};
use spargebra::term::{GroundTerm, GroundTriple, Variable};

use super::control_values::copy_cell;
use crate::plan_measure::clone_root::CompilerCloneRootV1;
use crate::{Error, Result};

use super::env::{composed_info_for, StarEnv};
use super::expr::rewrite_expr;
use super::util::FreshVars;
use super::walk::rewrite_pattern;

/// `BIND(expr AS ?v)` (rule R5a's Extend case, plus ADR-0032 D3 item 3's
/// `TRIPLE(e1,e2,e3)` BIND target): rewrites `inner` once, then delegates to
/// [`rewrite_extend_inner`] for the (possibly recursive) target-expression
/// handling.
pub(super) fn rewrite_extend(
    inner: &GraphPattern,
    variable: &Variable,
    expression: &Expression,
    n: &mut FreshVars,
    env: &mut StarEnv,
) -> Result<GraphPattern> {
    let rewritten_inner = rewrite_pattern(inner, n, env)?;
    rewrite_extend_inner(rewritten_inner, variable, expression, n, env)
}

/// The recursive core of [`rewrite_extend`], operating on an ALREADY-rewritten
/// `inner` so it can recurse onto itself for OBJECT-side `TRIPLE(...)`
/// nesting without re-rewriting `inner` at every level. A `TRIPLE(e1,e2,e3)`
/// target marks `variable` composed (`composed_info_for` reuses an
/// already-registered `variable`, e.g. one ALSO reified elsewhere) and
/// replaces the single BIND with THREE synthetic per-component `Extend`s —
/// `BIND(e1 AS s_var) BIND(e2 AS p_var) BIND(e3 AS o_var)`, innermost-first so
/// each is in scope for the next — reusing `unify::bind_term_def`'s existing
/// (narrow but adequate) machinery to lower e1/e2/e3 verbatim; `variable`
/// itself is never bound by any real pattern here — its projection is
/// realized wholly by `lib.rs`'s env-composed override, keyed off
/// `s_var`/`p_var`/`o_var` being bound (see [`StarEnv`]'s doc comment), not
/// off `variable`. `e3` (object position — the only position RDF 1.2 §3.1
/// allows to nest) recurses if it is ITSELF a `TRIPLE(...)` call, giving
/// arbitrary-depth nested composition for free. Anything else is an ordinary
/// BIND, `expression` rewritten in place (which also resolves a `TRIPLE(...)`
/// reached through equality/SUBJECT/PREDICATE/OBJECT/isTRIPLE — see
/// `super::expr::rewrite_and_check_composed` — or leaves an otherwise-unroutable
/// `TRIPLE(...)` Unsupported, see `super::expr::rewrite_function_call`).
pub(super) fn rewrite_extend_inner(
    rewritten_inner: GraphPattern,
    variable: &Variable,
    expression: &Expression,
    n: &mut FreshVars,
    env: &mut StarEnv,
) -> Result<GraphPattern> {
    n.work.local(CompilerCloneRootV1::Expression(expression))?;
    n.work.charge(6)?;
    n.work.charge(variable.as_str().len())?;
    if let Expression::FunctionCall(Function::Triple, parts) = expression {
        if let [e1, e2, e3] = parts.as_slice() {
            let info = composed_info_for(variable, n, env)?;
            let e1 = rewrite_expr(e1, n, env)?;
            let e2 = rewrite_expr(e2, n, env)?;
            let with_s = GraphPattern::Extend {
                inner: Box::new(rewritten_inner),
                variable: n.work.variable(&info.s_var)?,
                expression: e1,
            };
            let with_p = GraphPattern::Extend {
                inner: Box::new(with_s),
                variable: n.work.variable(&info.p_var)?,
                expression: e2,
            };
            return rewrite_extend_inner(with_p, &info.o_var, e3, n, env);
        }
    }
    Ok(GraphPattern::Extend {
        inner: Box::new(rewritten_inner),
        variable: variable.clone(),
        expression: rewrite_expr(expression, n, env)?,
    })
}

/// ADR-0032 D3 item 2, R6 (Wave 2b) — decompose any VALUES column carrying a
/// ground triple term. Column-major transpose, decompose each column
/// independently ([`decompose_column`]), transpose back — the row count and
/// row order are unaffected (`=_bag` preserving), only the column set/arity
/// changes for a composed variable.
pub(super) fn rewrite_values(
    variables: &[Variable],
    bindings: &[Vec<Option<GroundTerm>>],
    n: &mut FreshVars,
    env: &mut StarEnv,
) -> Result<GraphPattern> {
    let n_rows = bindings.len();
    let mut out_columns: Vec<(Variable, Vec<Option<GroundTerm>>)> = Vec::new();
    for (i, var) in variables.iter().enumerate() {
        n.work.charge(1)?;
        let mut cells = n.work.0.vector(n_rows)?;
        for row in bindings {
            cells.push(copy_cell(row.get(i).unwrap_or(&None), n.work)?);
        }
        decompose_column(n.work.variable(var)?, cells, n, &mut out_columns, env)?;
    }
    let mut out_variables = n.work.0.vector(out_columns.len())?;
    for (var, _) in &out_columns {
        out_variables.push(n.work.variable(var)?);
    }
    // The derived column count is only known now. Pay the actual R*C product
    // before building the output rows; each deep cell copy is separately paid.
    n.work.product(&[n_rows, out_columns.len()])?;
    let mut out_bindings = n.work.0.vector(n_rows)?;
    for r in 0..n_rows {
        let mut row = n.work.0.vector(out_columns.len())?;
        for (_, cells) in &out_columns {
            row.push(copy_cell(&cells[r], n.work)?);
        }
        out_bindings.push(row);
    }
    Ok(GraphPattern::Values {
        variables: out_variables,
        bindings: out_bindings,
    })
}

/// One VALUES column: passed through unchanged unless it carries ANY
/// `GroundTerm::Triple` cell, in which case EVERY bound (non-UNDEF) cell MUST
/// be one too (a column mixing a triple-term cell with a NamedNode/Literal
/// cell for the same variable is a genuine shape ambiguity this transform
/// cannot represent in one flat table → explicit Unsupported, never a silent
/// prune — the uniform-composed-ness law, `differential_star.rs`-locked). A
/// triple cell decomposes into 3 columns: subject/predicate are always
/// `NamedNode` (`GroundTriple`'s own field types — RDF 1.2 §3.1, no
/// recursion possible there), object recurses ([`decompose_column`] again)
/// since it may itself be another ground triple, arbitrary depth.
pub(super) fn decompose_column(
    var: Variable,
    cells: Vec<Option<GroundTerm>>,
    n: &mut FreshVars,
    out: &mut Vec<(Variable, Vec<Option<GroundTerm>>)>,
    env: &mut StarEnv,
) -> Result<()> {
    n.work.product(&[2, cells.len()])?;
    // A recursive column adds at most three retained column slots. Reallocation
    // may move all existing logical entries, which is included before append.
    n.work.product(&[
        3,
        out.len().checked_add(1).ok_or_else(|| n.work.overflow())?,
    ])?;
    let any_triple = cells
        .iter()
        .any(|c| matches!(c, Some(GroundTerm::Triple(_))));
    if !any_triple {
        out.push((var, cells));
        return Ok(());
    }
    if cells
        .iter()
        .any(|c| !matches!(c, None | Some(GroundTerm::Triple(_))))
    {
        n.work.charge(256 + var.as_str().len())?;
        return Err(Error::Unsupported(format!(
            "VALUES ?{} mixes a ground triple-term cell with a NamedNode/Literal cell for the \
             same variable → 501 (engine-total composed-ness must be uniform per var, ADR-0032 \
             D3)",
            var.as_str()
        )));
    }
    let info = composed_info_for(&var, n, env)?;
    let mut s_cells = n.work.0.vector(cells.len())?;
    let mut p_cells = n.work.0.vector(cells.len())?;
    let mut o_cells = n.work.0.vector(cells.len())?;
    for cell in cells {
        n.work.charge(4)?;
        match cell {
            Some(GroundTerm::Triple(t)) => {
                let GroundTriple {
                    subject,
                    predicate,
                    object,
                } = *t;
                s_cells.push(Some(GroundTerm::NamedNode(subject)));
                p_cells.push(Some(GroundTerm::NamedNode(predicate)));
                o_cells.push(Some(object));
            }
            None => {
                s_cells.push(None);
                p_cells.push(None);
                o_cells.push(None);
            }
            Some(_) => unreachable!("the mixed-shape check above already rejected this"),
        }
    }
    out.push((info.s_var, s_cells));
    out.push((info.p_var, p_cells));
    decompose_column(info.o_var, o_cells, n, out, env)
}
