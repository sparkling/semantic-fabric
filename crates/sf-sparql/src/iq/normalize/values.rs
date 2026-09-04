use std::collections::BTreeMap;

use crate::iq::node::{BindDef, Var};
use crate::iq::TermDef;
use crate::unify::bind_term_def;

pub(super) fn constant_subst_can_form_row(subst: &BTreeMap<Var, BindDef>, project: &[Var]) -> bool {
    let no_vars = BTreeMap::new();
    project.iter().all(|var| match subst.get(var) {
        Some(BindDef::Resolved(TermDef::Const(_))) | None => true,
        Some(BindDef::Expr(expression)) => bind_term_def(expression, &no_vars).is_ok(),
        Some(_) => false,
    })
}

pub(super) fn constant_row_from_subst(
    mut subst: BTreeMap<Var, BindDef>,
    project: &[Var],
) -> Option<Vec<Option<TermDef>>> {
    let no_vars = BTreeMap::new();
    let mut row = Vec::with_capacity(project.len());
    for var in project {
        let cell = match subst.remove(var) {
            Some(BindDef::Resolved(TermDef::Const(term))) => Some(TermDef::Const(term)),
            Some(BindDef::Expr(expression)) => Some(bind_term_def(&expression, &no_vars).ok()?),
            Some(_) => return None,
            None => None,
        };
        row.push(cell);
    }
    Some(row)
}

/// Whether `a` and `b` name exactly the same set of variables (order-independent,
/// no narrowing either way). Assumes both are already duplicate-free (true of every
/// `project`/`vars` this crate builds — SPARQL rejects a repeated variable name in a
/// projection or a `VALUES` header); a caller passing a list with a repeated name
/// would get a false positive here.
pub(super) fn same_var_set(a: &[Var], b: &[Var]) -> bool {
    a.len() == b.len() && a.iter().all(|v| b.contains(v))
}
