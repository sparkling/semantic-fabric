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

use crate::{Error, Result};

use super::env::{composed_info_for, StarEnv};
use super::expr::rewrite_expr;
use super::walk::rewrite_pattern;

/// `BIND(expr AS ?v)` (rule R5a's Extend case, plus ADR-0032 D3 item 3's
/// `TRIPLE(e1,e2,e3)` BIND target): rewrites `inner` once, then delegates to
/// [`rewrite_extend_inner`] for the (possibly recursive) target-expression
/// handling.
pub(super) fn rewrite_extend(
    inner: &GraphPattern,
    variable: &Variable,
    expression: &Expression,
    n: &mut usize,
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
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<GraphPattern> {
    if let Expression::FunctionCall(Function::Triple, parts) = expression {
        if let [e1, e2, e3] = parts.as_slice() {
            let info = composed_info_for(variable, n, env);
            let e1 = rewrite_expr(e1, n, env)?;
            let e2 = rewrite_expr(e2, n, env)?;
            let with_s = GraphPattern::Extend {
                inner: Box::new(rewritten_inner),
                variable: info.s_var.clone(),
                expression: e1,
            };
            let with_p = GraphPattern::Extend {
                inner: Box::new(with_s),
                variable: info.p_var.clone(),
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
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<GraphPattern> {
    let n_rows = bindings.len();
    let mut out_columns: Vec<(Variable, Vec<Option<GroundTerm>>)> =
        Vec::with_capacity(variables.len());
    for (i, var) in variables.iter().enumerate() {
        let cells: Vec<Option<GroundTerm>> = bindings.iter().map(|row| row[i].clone()).collect();
        decompose_column(var.clone(), cells, n, &mut out_columns, env)?;
    }
    let out_variables: Vec<Variable> = out_columns.iter().map(|(v, _)| v.clone()).collect();
    let out_bindings: Vec<Vec<Option<GroundTerm>>> = (0..n_rows)
        .map(|r| {
            out_columns
                .iter()
                .map(|(_, cells)| cells[r].clone())
                .collect()
        })
        .collect();
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
    n: &mut usize,
    out: &mut Vec<(Variable, Vec<Option<GroundTerm>>)>,
    env: &mut StarEnv,
) -> Result<()> {
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
        return Err(Error::Unsupported(format!(
            "VALUES ?{} mixes a ground triple-term cell with a NamedNode/Literal cell for the \
             same variable → 501 (engine-total composed-ness must be uniform per var, ADR-0032 \
             D3)",
            var.as_str()
        )));
    }
    let info = composed_info_for(&var, n, env);
    let mut s_cells = Vec::with_capacity(cells.len());
    let mut p_cells = Vec::with_capacity(cells.len());
    let mut o_cells = Vec::with_capacity(cells.len());
    for cell in cells {
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
    out.push((info.s_var.clone(), s_cells));
    out.push((info.p_var.clone(), p_cells));
    decompose_column(info.o_var.clone(), o_cells, n, out, env)
}
