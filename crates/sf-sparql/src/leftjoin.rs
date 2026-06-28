//! OPTIONAL → NULL-safe LEFT JOIN — the ISWC-2018 base translation's left-join
//! half (ADR-0007 R1–R5): the shared-variable compatibility `ON` (R1), the
//! `COALESCE(left, right)` projection of a shared variable (R2), and the
//! inner-FILTER-into-`ON` placement (R5). Split from [`crate::unfold`] so the
//! conjunctive core and the left-join semantics stay independently legible.

use std::collections::HashSet;

use crate::iq::{Branch, CmpOp, OptJoin, SqlCond, TermDef};
use crate::unify::{filter_cond, unify, Unify};
use crate::{Error, Result};

/// OPTIONAL → NULL-safe LEFT JOIN (ADR-0007 R1–R5). v1: a single-scan,
/// opt-free right side (the common case); richer right sides are deferred.
pub fn left_join_branches(
    left: Vec<Branch>,
    right: Vec<Branch>,
    expr: Option<&spargebra::algebra::Expression>,
    dialect: sf_sql::Dialect,
) -> Result<Vec<Branch>> {
    if right.len() != 1 {
        return Err(Error::Unsupported(
            "OPTIONAL with a UNION/empty right side is deferred → 501 (ADR-0007)".to_owned(),
        ));
    }
    let r = &right[0];
    if r.core.len() != 1 || !r.opts.is_empty() {
        return Err(Error::Unsupported(
            "multi-scan OPTIONAL right side is deferred → 501 (ADR-0007)".to_owned(),
        ));
    }
    let mut out = Vec::new();
    for mut l in left {
        if let Some(b) = build_left_join(&mut l, r, expr, dialect)? {
            out.push(b);
        } else {
            out.push(l); // optional never matches → left unchanged (right vars unbound)
        }
    }
    Ok(out)
}

/// Returns `Some(branch-with-OptJoin)`, or `None` when the shared variables prove
/// the optional can never match (so the caller keeps the left side as-is).
fn build_left_join(
    left: &mut Branch,
    right: &Branch,
    expr: Option<&spargebra::algebra::Expression>,
    dialect: sf_sql::Dialect,
) -> Result<Option<Branch>> {
    let mut on = Vec::new();
    let mut extra = right.where_conds.clone(); // constant-position constraints stay in the ON (R5)
    for (var, rdef) in &right.bindings {
        if let Some(ldef) = left.bindings.get(var) {
            match unify(ldef, rdef) {
                Unify::Sat(conds) => {
                    for c in conds {
                        on.push(null_safe(c)); // R1: shared-var compat, never plain a = b
                    }
                }
                Unify::Empty => return Ok(None),
                Unify::Unsupported(why) => return Err(Error::Unsupported(why)),
            }
        }
    }
    // Combined bindings for the inner FILTER (R5: it goes in the ON, not WHERE).
    if let Some(e) = expr {
        let mut combined = left.bindings.clone();
        for (v, d) in &right.bindings {
            combined.entry(v.clone()).or_insert_with(|| d.clone());
        }
        extra.push(filter_cond(e, &combined, dialect).map_err(Error::Unsupported)?);
    }
    // R2 projection (ADR-0007). Prior-OPTIONAL aliases are nullable. A shared
    // variable whose preserved (left) side can be NULL (a nested OPTIONAL) becomes
    // COALESCE(left, right) so the right value survives when left is unbound; a
    // mandatory-left shared var is never NULL (COALESCE(left,right)=left) so we keep
    // the simpler left def; a right-only var is the (possibly NULL) right output.
    let opt_aliases: HashSet<usize> = left.opts.iter().map(|o| o.scan.alias).collect();
    for (var, rdef) in &right.bindings {
        match left.bindings.get(var) {
            Some(ldef) if def_is_nullable(ldef, &opt_aliases) => {
                let c = TermDef::Coalesce(Box::new(ldef.clone()), Box::new(rdef.clone()));
                left.bindings.insert(var.clone(), c);
            }
            Some(_) => {}
            None => {
                left.bindings.insert(var.clone(), rdef.clone());
            }
        }
    }
    left.opts.push(OptJoin {
        scan: right.core[0].clone(),
        on,
        extra,
    });
    Ok(Some(left.clone()))
}

/// Turn an inner-join equality into the OPTIONAL NULL-safe form (R1): an unbound
/// shared variable is compatible with any value, so a nullable side must be
/// admitted.
fn null_safe(c: SqlCond) -> SqlCond {
    match c {
        // column = column: `(a = b OR a IS NULL OR b IS NULL)`.
        SqlCond::ColEq(a, b) => SqlCond::NullSafeEq(a, b),
        // constant vs (possibly nullable, e.g. nested-OPTIONAL) column: the constant
        // can never be NULL, so guard only the column: `(col = ? OR col IS NULL)`.
        SqlCond::Cmp(col, CmpOp::Eq, val) => SqlCond::Or(vec![
            SqlCond::Cmp(col.clone(), CmpOp::Eq, val),
            SqlCond::IsNull(col),
        ]),
        other => other,
    }
}

/// Whether a binding's value can be NULL because it reads a nullable
/// (prior-OPTIONAL) scan alias — the trigger for the R2 COALESCE projection.
fn def_is_nullable(def: &TermDef, opt_aliases: &HashSet<usize>) -> bool {
    match def {
        TermDef::Const(_) => false,
        TermDef::Derived { alias, .. } => opt_aliases.contains(alias),
        TermDef::Coalesce(l, r) => {
            def_is_nullable(l, opt_aliases) || def_is_nullable(r, opt_aliases)
        }
        TermDef::Concat(parts) => parts.iter().any(|p| def_is_nullable(p, opt_aliases)),
    }
}
