use super::control::{RowVec, RowWork};
use crate::iq::node::{BindDef, Var};
use crate::iq::TermDef;
use crate::Result;
use spargebra::algebra::{Expression, Function};
use std::cmp::Ordering;
use std::collections::BTreeMap;

// BTreeMap order supplies a sorted index; fallible binary comparisons make each
// name scan visible without relying on private standard-library tree geometry.
fn binding_index<K: AsRef<str>, T>(
    entries: &[(K, T)],
    name: &str,
    work: RowWork<'_>,
) -> Result<Option<usize>> {
    let (mut low, mut high) = (0, entries.len());
    while low < high {
        let mid = low + (high - low) / 2;
        match work.name_order(entries[mid].0.as_ref(), name)? {
            Ordering::Less => low = mid + 1,
            Ordering::Greater => high = mid,
            Ordering::Equal => return Ok(Some(mid)),
        }
    }
    Ok(None)
}

pub(super) fn constant_subst_can_form_row(
    subst: &BTreeMap<Var, BindDef>,
    project: &[Var],
    work: RowWork<'_>,
) -> Result<bool> {
    work.charge(1)?;
    let mut entries = work.vector(subst.len())?;
    for entry in subst {
        work.charge(1)?;
        entries.push(entry);
    }
    for var in project {
        work.charge(1)?;
        let Some(i) = binding_index(&entries, var, work)? else {
            continue;
        };
        match entries[i].1 {
            BindDef::Resolved(TermDef::Const(_)) => {}
            BindDef::Expr(expr) if constant_expression_supported(expr, work)? => {}
            _ => return Ok(false),
        }
    }
    work.checkpoint()?;
    Ok(true)
}

pub(super) fn constant_row_from_subst(
    subst: BTreeMap<Var, BindDef>,
    project: &[Var],
    work: RowWork<'_>,
) -> Result<Option<Vec<Option<TermDef>>>> {
    work.charge(1)?;
    let mut entries = work.vector(subst.len())?;
    for (key, value) in subst {
        work.charge(1)?;
        entries.push((key, Some(value)));
    }
    let mut row = work.vector(project.len())?;
    for var in project {
        work.charge(1)?;
        let value = match binding_index(&entries, var, work)? {
            Some(i) => entries[i].1.take(),
            None => None,
        };
        let cell = match value {
            Some(BindDef::Resolved(TermDef::Const(term))) => Some(TermDef::Const(term)),
            Some(BindDef::Expr(expr)) => match constant_expression(&expr, work)? {
                Some(term) => Some(term),
                None => return Ok(None),
            },
            Some(_) => return Ok(None),
            None => None,
        };
        row.push(cell);
    }
    work.checkpoint()?;
    Ok(Some(row))
}

// Match bind_term_def's empty-environment domain, without constructing and
// immediately discarding a term (or an unobservable formatted error) to probe it.
fn constant_expression_supported(expr: &Expression, work: RowWork<'_>) -> Result<bool> {
    let mut pending = RowVec::new(Vec::new());
    work.push(&mut pending, expr)?;
    while !pending.values.is_empty() {
        work.charge(1)?;
        match pending.values.pop().expect("nonempty expression stack") {
            Expression::NamedNode(_) | Expression::Literal(_) => {}
            Expression::FunctionCall(Function::Concat, args) => {
                for arg in args.iter().rev() {
                    work.push(&mut pending, arg)?;
                }
            }
            _ => return Ok(false),
        }
    }
    work.checkpoint()?;
    Ok(true)
}

fn constant_expression(expr: &Expression, work: RowWork<'_>) -> Result<Option<TermDef>> {
    // Preserve the existing controlled AST depth envelope during materialization.
    let work = work.enter()?;
    let term = match expr {
        Expression::NamedNode(n) => TermDef::Const(work.copied(n)?.into()),
        Expression::Literal(l) => TermDef::Const(work.copied(l)?.into()),
        Expression::FunctionCall(Function::Concat, args) => {
            let mut parts = work.vector(args.len())?;
            for arg in args {
                let Some(part) = constant_expression(arg, work)? else {
                    return Ok(None);
                };
                parts.push(part);
            }
            TermDef::Concat(parts)
        }
        _ => return Ok(None),
    };
    work.checkpoint()?;
    Ok(Some(term))
}

/// Same duplicate-free variable set, independent of order and with no narrowing.
/// Validate the builder invariant for direct IQ callers too: one-way containment
/// on [x,x] and [x,y] would otherwise admit an impossible row permutation.
pub(super) fn same_var_set(a: &[Var], b: &[Var], work: RowWork<'_>) -> Result<bool> {
    work.charge(1)?;
    if a.len() != b.len() {
        return Ok(false);
    }
    for vars in [a, b] {
        for (i, var) in vars.iter().enumerate() {
            work.charge(1)?;
            if work.contains(&vars[..i], var)? {
                return Ok(false);
            }
        }
    }
    for var in a {
        if !work.contains(b, var)? {
            return Ok(false);
        }
    }
    work.checkpoint()?;
    Ok(true)
}
