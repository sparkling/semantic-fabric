//! OPTIONAL → NULL-safe LEFT JOIN — the ISWC-2018 base translation's left-join
//! half (ADR-0007 R1–R5): the shared-variable compatibility `ON` (R1), the
//! `COALESCE(left, right)` projection of a shared variable (R2), and the
//! inner-FILTER-into-`ON` placement (R5). Split from [`crate::unfold`] so the
//! conjunctive core and the left-join semantics stay independently legible.
//!
//! For `P OPT R` with a multi-branch or multi-scan right the ISWC-2018
//! decomposition is used: one inner-join branch per Ri (all Ri scans merged
//! into the FROM clause) plus a no-match branch (`NOT EXISTS Ri` for each Ri).

use std::collections::HashSet;

use sf_core::ir::Segment;

use crate::build::control::{BuildVec, BuildWork};
use crate::iq::{Branch, CmpOp, ColRef, OptJoin, SqlCond, TermDef};
use crate::unify::Unify;
use crate::{CompilerWorkMode, Error, Result};
pub(crate) mod conditions;
mod decomposition;
mod fast;
pub(crate) mod preparation;
pub(crate) mod work;
#[cfg(test)]
pub(crate) use decomposition::{inner_join_one, not_exists_cond_for};
pub(crate) use decomposition::{inner_join_one_with_work_mode, not_exists_cond_for_with_work_mode};
use fast::build_left_join;
#[cfg(test)]
#[path = "leftjoin/condition_work_tests.rs"]
mod optional_condition_work_tests;
#[cfg(test)]
#[path = "leftjoin/filter_work_tests.rs"]
mod optional_filter_work_tests;
#[cfg(test)]
#[path = "leftjoin/preparation_tests.rs"]
mod optional_preparation_tests;
#[cfg(test)]
#[path = "leftjoin/unification_work_tests.rs"]
mod optional_unification_work_tests;
#[cfg(test)]
#[path = "leftjoin/work_test_support.rs"]
pub(crate) mod optional_work_test_support;
#[cfg(test)]
#[path = "leftjoin/work_tests.rs"]
mod optional_work_tests;

/// OPTIONAL → NULL-safe branches (ADR-0007 R1–R5).
///
/// - Empty `right`: identity (`left` unchanged — `OPTIONAL {}` = noop).
/// - Single-branch, single-scan `right`, opt-free: SQL LEFT JOIN via
///   [`build_left_join`].
/// - Any other `right` (multi-branch or multi-scan per branch, opt-free):
///   ISWC-2018 decomposition `P OPT R = (P ⋈ R) ∪ (P - R)`.
///   One inner-join branch per Ri (all scans merged into a FROM clause)
///   plus a no-match branch with `NOT EXISTS Ri` for every Ri.
///   Nested OPTIONAL inside the right (opts non-empty) remains → 501.
pub fn left_join_branches(
    left: Vec<Branch>,
    right: Vec<Branch>,
    expr: Option<&spargebra::algebra::Expression>,
    dialect: sf_sql::Dialect,
) -> Result<Vec<Branch>> {
    left_join_branches_with_work_mode(left, right, expr, dialect, CompilerWorkMode::Uncontrolled)
}

/// Request-owned OPTIONAL candidate and direct-copy admission.
pub(crate) fn left_join_branches_with_work_mode(
    left: Vec<Branch>,
    right: Vec<Branch>,
    expr: Option<&spargebra::algebra::Expression>,
    dialect: sf_sql::Dialect,
    mode: CompilerWorkMode<'_>,
) -> Result<Vec<Branch>> {
    let work = BuildWork::new(mode);
    work.checkpoint()?;
    // OPTIONAL {} = identity.
    if right.is_empty() {
        return Ok(left);
    }

    // All right branches must be opt-free (nested OPTIONAL inside OPTIONAL
    // right is not yet supported).  Multi-scan right (core.len() > 1) is sound
    // via the decomposition below: (P ⋈ R) ∪ (P - R).
    for r in &right {
        if !r.opts.is_empty() {
            return Err(Error::Unsupported(
                "nested OPTIONAL inside an OPTIONAL right side is deferred → 501 (ADR-0007)"
                    .to_owned(),
            ));
        }
    }

    // Single-branch, single-scan right: SQL LEFT JOIN (the common case).
    // A sealed Ref still represents the former two-scan native relation. Keep
    // its decomposition path: making it an OptJoin here can strand nested
    // OPTIONALs in `opts` and diverge from the tree lowerer's decomposition.
    if right.len() == 1
        && right[0].core.len() == 1
        // A newly introduced mapping constant still becomes UNBOUND when the
        // optional row is absent. Const has no nullable source witness: use the
        // existing match/no-match decomposition rather than invent a value.
        && !right[0].bindings.iter().any(|(var, def)| {
            matches!(def, TermDef::Const(_))
                && left.iter().any(|branch| !matches!(branch.bindings.get(var), Some(TermDef::Const(_))))
        })
        && !matches!(
            right[0].core[0].source,
            crate::iq::ScanSource::RefAtom { .. }
        )
    {
        let r = &right[0];
        work::candidates(mode, left.len(), 1, false)?;
        let mut out = BuildVec::new(Vec::new());
        for l in left {
            work.push(&mut out, build_left_join(l, r, expr, dialect, mode)?)?;
        }
        return Ok(out.into_inner());
    }

    // Multi-branch or multi-scan right: P OPT R = (P ⋈ R) ∪ (P - R)
    // Handles both: OPTIONAL with multiple triples-map branches (UNION) and
    // OPTIONAL with multiple table scans (JOIN) within one branch.  Each Ri is
    // inner-joined with P; P rows with no Ri match go in the no-match branch.
    // = (P ⋈_NL R1) ∪ (P ⋈_NL R2) ∪ … ∪ P_no_match
    work::candidates(mode, left.len(), right.len(), true)?;
    let mut out = BuildVec::new(Vec::new());
    for mut l in left {
        // One inner-join branch per right branch.
        for r in &right {
            if let Some(b) = inner_join_one_with_work_mode(&l, r, expr, dialect, mode)? {
                work.push(&mut out, b)?;
            }
        }
        // No-match branch: L with NOT EXISTS for each Ri that can possibly match.
        for r in &right {
            if let Some(cond) = not_exists_cond_for_with_work_mode(&l, r, expr, dialect, mode)? {
                work::push_owned(work, &mut l.where_conds, cond)?;
            }
        }
        work.push(&mut out, l)?;
    }
    Ok(out.into_inner())
}

/// Turn an inner-join equality into the OPTIONAL NULL-safe form (R1): an unbound
/// shared variable is compatible with any value, so a nullable side must be
/// admitted.
///
/// The disjunctive `OR … IS NULL` guard is ONLY emitted when the LEFT (preserved /
/// outer) binding of the shared variable can actually be NULL — i.e. it reads a
/// prior-OPTIONAL scan alias (`left_nullable`). When the left binding is mandatory
/// (e.g. a subject bound by a non-OPTIONAL `?t a gtfs:Trip`) the shared variable is
/// never unbound, `a IS NULL` is dead, and the RIGHT shared-var column is itself
/// non-NULL (a subject/FK key by PK, or an object column already carrying an
/// `IS NOT NULL` where-cond in this branch), so `(a = b OR a IS NULL OR b IS NULL)`
/// is result-equivalent to the plain `a = b`. Emitting the plain equality lets
/// PostgreSQL use a hash/merge join instead of a disjunction-forced nested loop —
/// the O(n²) blow-up on nested/multi-scan OPTIONAL (q14) collapses to a linear join.
pub(crate) fn null_safe(c: SqlCond, left_nullable: bool) -> SqlCond {
    if !left_nullable {
        return c;
    }
    match c {
        SqlCond::IriCmp(cmp) => {
            let mut parts: Vec<_> = cmp.columns().cloned().map(SqlCond::IsNull).collect();
            parts.insert(0, SqlCond::IriCmp(cmp));
            SqlCond::Or(parts)
        }
        SqlCond::LiteralCmp(cmp) if cmp.value_op.is_none() => {
            let mut parts: Vec<_> = cmp.columns().cloned().map(SqlCond::IsNull).collect();
            parts.insert(0, SqlCond::LiteralCmp(cmp));
            SqlCond::Or(parts)
        }
        // column = column: `(a = b OR a IS NULL OR b IS NULL)`.
        SqlCond::ColEq(a, b) => SqlCond::NullSafeEq(a, b),
        // constant vs (possibly nullable, e.g. nested-OPTIONAL) column: the constant
        // can never be NULL, so guard only the column: `(col = ? OR col IS NULL)`.
        SqlCond::Cmp(col, CmpOp::Eq, val) => SqlCond::Or(vec![
            SqlCond::Cmp(col.clone(), CmpOp::Eq, val),
            SqlCond::IsNull(col),
        ]),
        // template = template (Run 4 Wave B3's `SqlCond::TemplateEq` fallback,
        // Run 4 B-repair FIX 1): R1's null-compatibility disjunct, generalized
        // to a template's WHOLE column set. R2RML §11 (`sf_core::ir::Template::
        // expand`): a template's constructed term is unbound the moment ANY ONE
        // of its referenced columns is NULL — so EITHER side being unbound (any
        // single column of that side NULL) must admit the row, the same
        // "unbound is compatible with anything" rule the `ColEq`/`Cmp` arms
        // above apply to their one column. Before this fix, `TemplateEq` fell
        // through to the `other => other` catch-all below, silently dropping
        // R1 whenever `left_nullable` — a left row whose ?v was legitimately
        // UNBOUND (a prior OPTIONAL never matched) then failed to join a
        // compatible right row instead of admitting it.
        SqlCond::TemplateEq(sx, a1, sy, a2, encode) => {
            let mut disjuncts = Vec::with_capacity(1 + sx.len() + sy.len());
            for seg in &sx {
                if let Segment::Column(c) = seg {
                    disjuncts.push(SqlCond::IsNull(ColRef::new(a1, c.clone())));
                }
            }
            for seg in &sy {
                if let Segment::Column(c) = seg {
                    disjuncts.push(SqlCond::IsNull(ColRef::new(a2, c.clone())));
                }
            }
            disjuncts.insert(0, SqlCond::TemplateEq(sx, a1, sy, a2, encode));
            SqlCond::Or(disjuncts)
        }
        other => other,
    }
}

/// Whether a binding's value can be NULL because it reads a nullable
/// (prior-OPTIONAL) scan alias — the trigger for the R2 COALESCE projection.
pub(crate) fn def_is_nullable(def: &TermDef, opt_aliases: &HashSet<usize>) -> bool {
    match def {
        TermDef::Const(_) => false,
        TermDef::Derived { alias, .. } => opt_aliases.contains(alias),
        TermDef::R2rmlBlank { .. } => def
            .columns()
            .iter()
            .any(|column| opt_aliases.contains(&column.alias)),
        TermDef::Coalesce(l, r) => {
            def_is_nullable(l, opt_aliases) || def_is_nullable(r, opt_aliases)
        }
        TermDef::Concat(parts) => parts.iter().any(|p| def_is_nullable(p, opt_aliases)),
        // An aggregate result is produced post-grouping, never under an OPTIONAL.
        TermDef::Agg { .. } => false,
        // ADR-0032 D2: forced arm (new `TermDef` variant) — not actually reachable
        // here in practice (a `ComposedTriple` binding is only installed by `lib.rs`'s
        // env-composed projection override, AFTER every OPTIONAL is already built),
        // but recurses through its three components for the same reason `Coalesce`/
        // `Concat` do, should that ever change.
        TermDef::ComposedTriple {
            subject,
            predicate,
            object,
        } => {
            def_is_nullable(subject, opt_aliases)
                || def_is_nullable(predicate, opt_aliases)
                || def_is_nullable(object, opt_aliases)
        }
    }
}

/// The sound-501 message for a shared-variable correlation that reads one of `left`'s
/// SubPlan derived-table aliases, LEFT- or INNER-joined alike (see
/// [`preparation::Preparation::shared_reads_left_subplan`]).
pub(crate) const SHARED_LEFT_SUBPLAN_501: &str =
    "OPTIONAL/MINUS decomposition correlating on a variable bound by a SubPlan derived \
     table on the preserved side (its ON/anti-join would reference a table emitted to \
     its right) is not yet supported → 501 (ADR-0023 Item 1d boundary)";

/// Whether any shared variable's LEFT (preserved) binding reads ANY of `left`'s SubPlan
/// derived-table aliases — INNER-joined (ADR-0034 D1's own per-pattern wrap; `left ==
/// false`) as well as LEFT-joined (`left == true`, the case this function/constant's own
/// naming originally targeted).
///
/// Such a correlation cannot be lowered soundly by the flat LEFT-JOIN / anti-join
/// builders in this module: EITHER kind of SubPlan is emitted as a derived table AFTER
/// `opts` in the FROM clause ([`crate::emit`] `render_from` — the ordering does not
/// distinguish `on`/`left`), so an `OptJoin.on` referencing it is
/// an "ON clause references a table to its right" SQL error — a CRASH at execution, not
/// a wrong answer, reproduced by an adversarial review of `e7cb7e6` (ADR-0023 Item 1d) —
/// and the `(P − R)` anti-join half of the decomposition cannot faithfully carry it
/// either. Detect it and return a sound 501 (ADR-0007: a 501 beats a crash), narrowing
/// Item 1d's capability to the case where a SubPlan-OPTIONAL's bound variable is NOT
/// re-correlated by a following plain OPTIONAL or decomposed MINUS.
///
/// A correlating `FILTER EXISTS` / `NOT EXISTS` / `MINUS` over such a variable does NOT
/// reach here: `lower_iq_exists` handles those correctly (its null-safe
/// EXISTS-substitution guard references the derived table from the WHERE clause, where it
/// is legally in scope), and a SubPlan RIGHT operand of a following OPTIONAL is handled
/// by `left_join_over_subplan` (subplans emit in order, so a later one may reference an
/// earlier one). Only a plain-scan / multi-scan right side routed through these builders
/// hits the FROM-ordering wall.
#[cfg(test)]
fn shared_reads_left_subplan(left: &Branch, right: &Branch) -> bool {
    let sp_aliases: HashSet<usize> = left.subplan_joins.iter().map(|s| s.alias).collect();
    if sp_aliases.is_empty() {
        return false;
    }
    right.bindings.keys().any(|v| {
        left.bindings
            .get(v)
            .is_some_and(|ldef| ldef.columns().iter().any(|c| sp_aliases.contains(&c.alias)))
    })
}

#[cfg(test)]
#[path = "leftjoin/tests.rs"]
mod tests;
