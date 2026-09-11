//! OPTIONAL-specific lowering; shared helpers retain the raw flat oracle.
use super::*;
use crate::build::control::{BuildVec, BuildWork};
use crate::leftjoin::{
    conditions, inner_join_one_with_work_mode as inner_join_one, materialization,
    not_exists_cond_for_with_work_mode as not_exists_cond_for, preparation, shape, work,
};

/// The OPTS-FREE form of `left OPT right` — the ISWC-2018 `(P⋈R)∪(P−R)` decomposition,
/// used when this `LeftJoin` is itself the RIGHT operand of an enclosing `LeftJoin` and so
/// must yield re-feedable opts-free branches (§5.3 nested-right closure). It mirrors the
/// multi-branch arm of [`left_join_branches`] but NEVER takes the single-scan `OptJoin`
/// shortcut (which would leave `opts` set), reusing the proven [`inner_join_one`] /
/// [`not_exists_cond_for`] helpers verbatim. `right` is always opts-free here (it was
/// lowered with the `decompose` flag, and only `build_left_join` — gated on `decompose ==
/// false` — ever sets `opts`), so the `!r.opts.is_empty()` guard below is a dead defensive
/// boundary (see [`left_join_as_subplan`]).
pub(super) fn left_join_decomposed(
    left: Vec<Branch>,
    right: Vec<Branch>,
    expr: Option<&Expression>,
    dialect: sf_sql::Dialect,
    mode: CompilerWorkMode<'_>,
) -> Result<Vec<Branch>> {
    let work = BuildWork::new(mode);
    work.checkpoint()?;
    if right.is_empty() {
        return Ok(left); // OPTIONAL {} = identity
    }
    // Defensive: a right branch still carrying `opts` would mean the decomposition is
    // unavailable. This is UNREACHABLE in practice — the `decompose` invariant guarantees
    // every right branch fed here is opts-free (only `build_left_join`, gated on
    // `decompose == false`, ever sets `opts`). `left_join_as_subplan` is a dead-code
    // boundary kept only so this match stays total (see its doc comment).
    if shape::right_has_options(&right, mode)? {
        return left_join_as_subplan(left, right, expr, dialect, mode);
    }
    // (P ⋈ Ri) for each right branch, plus one no-match branch (P − R): NOT EXISTS Ri
    // for every Ri that can possibly match. Identical to `left_join_branches`' multi
    // arm, so the opts-free output is `=_bag` to it.
    work::candidates(mode, left.len(), right.len(), true)?;
    let mut out = BuildVec::new(Vec::new());
    for mut l in left {
        for r in &right {
            if let Some(b) = inner_join_one(&l, r, expr, dialect, mode)? {
                work.push(&mut out, b)?;
            }
        }
        // Derive every anti-join from the same unmodified left branch, matching
        // the previous rollback-copy semantics before transferring ownership.
        let mut no_match_conditions = BuildVec::new(Vec::new());
        for r in &right {
            if let Some(cond) = not_exists_cond_for(&l, r, expr, dialect, mode)? {
                work.push(&mut no_match_conditions, cond)?;
            }
        }
        // Match branches have already copied the left state they need. Move the
        // original into the one no-match tail instead of cloning the whole Branch.
        for cond in no_match_conditions.into_inner() {
            work::push_owned(work, &mut l.where_conds, cond)?;
        }
        work.push(&mut out, l)?;
    }
    Ok(out.into_inner())
}

/// Whether `r` is a single pure-SubPlan branch — a modifier subquery
/// (Aggregation/Distinct/Slice/OrderBy) lowered by [`lower_as_subplan`]: exactly
/// one `SubPlanJoin`, and NOTHING else (no base scans, OPTIONALs, path CTE, own
/// aggregate, or residual WHERE conds). This is the shape [`left_join_over_subplan`]
/// attaches as a derived-table LEFT JOIN; any richer right shape falls through to the
/// ordinary decomposition (which stays a sound 501 for the not-yet-supported cases).
pub(super) fn is_single_subplan_branch(r: &[Branch]) -> bool {
    r.len() == 1
        && r[0].core.is_empty()
        && r[0].opts.is_empty()
        && r[0].path.is_none()
        && r[0].agg.is_none()
        && r[0].where_conds.is_empty()
        && r[0].subplan_joins.len() == 1
        && subplan_emits_soundly_as_derived_table(&r[0].subplan_joins[0].plan)
}

/// Whether a nested `Plan` is emitted FAITHFULLY when inlined as a `(SELECT …) t`
/// derived table (`emit::emit_subplan_sql`). A derived table is pure SQL with NO
/// exec stage, but [`Plan::prepared_branches`] deliberately keeps `ORDER BY` OUT of
/// SQL (it is applied in `exec` via the type-aware, collation-independent
/// comparator) — so an `ORDER BY` paired with a `LIMIT`/`OFFSET` inside a SubPlan
/// would silently drop BOTH, letting the WRONG rows survive the slice (a wrong
/// answer, not a 501). Reject that exact combination so it falls through to the
/// ordinary decomposition and stays a SOUND 501 (ADR-0007). `ORDER BY` alone is
/// `=_bag`-harmless (order does not change the multiset); `LIMIT`/`OFFSET` without
/// `ORDER BY` is pushed into the branch SQL verbatim and is faithful.
fn subplan_emits_soundly_as_derived_table(plan: &Plan) -> bool {
    !(!plan.order.is_empty() && (plan.limit.is_some() || plan.offset > 0))
}

/// `left OPT right` where `right` is a single pure-SubPlan branch (a modifier
/// subquery as the OPTIONAL's right operand — ADR-0023 parity backlog Item 1d) and
/// the OPTIONAL carries NO inner FILTER (the caller gates on `expr.is_none()` — the
/// FILTER-in-ON case did not evaluate soundly over a SubPlan and stays a 501).
/// Attaches the nested SubPlan to each left branch as a derived-table LEFT JOIN
/// (`SubPlanJoin { left: true, on: <correlation> }`), which the emit LEFT JOIN path
/// (`emit::render_from`) already renders. Mirrors [`crate::leftjoin`]'s
/// `build_left_join` R1/R2 — the shared-var compatibility ON (R1, [`null_safe`]) and
/// the `COALESCE(left, right)` projection of a nullable-left shared var (R2,
/// [`def_is_nullable`]) — but the right side is a derived table, not a single scan.
/// The result is OPTS-FREE (only `subplan_joins` grow), so it is re-feedable under
/// the `decompose` nested-right closure (§5.3) with no special case.
///
/// The SubPlan is a genuine SPARQL sub-SELECT: it is evaluated INDEPENDENTLY
/// (bottom-up), producing its own solution multiset, which is then LEFT-JOINed onto
/// `left` on the shared variables — exactly the LEFT JOIN of a derived table on the
/// correlation `ON`.
pub(super) fn left_join_over_subplan(
    left: Vec<Branch>,
    right: &Branch,
    dialect: sf_sql::Dialect,
    mode: CompilerWorkMode<'_>,
) -> Result<Vec<Branch>> {
    let work = BuildWork::new(mode);
    work::candidates(mode, left.len(), 1, false)?;
    let _ = dialect; // reserved for parity with the scan-based build_left_join signature
    let mut out = BuildVec::new(Vec::new());
    for mut l in left {
        // A property-path LEFT branch (`path: Some(_)`, empty `core`) has NO sound
        // representation once a SubPlan is pushed onto it: `emit_branch_with` dispatches
        // unconditionally on `b.path` to `emit_path_branch`, which renders ONLY the path's
        // own recursive CTE + projection and IGNORES `subplan_joins` entirely — the
        // subplan's derived table is never emitted, so a binding reading `t{sp}` has no
        // FROM entry ("no such column" at SQL-execution time). Sound 501 instead of a
        // crash (ADR-0007), mirroring `build_left_join`'s matching path-left guard for the
        // plain-scan-right case (`path_as_optional_left_via_single_scan_fast_path_...`).
        if l.path.is_some() {
            return Err(Error::Unsupported(
                "OPTIONAL whose preceding pattern is a property-path, with a SubPlan \
                 (modifier sub-SELECT) right side, is not yet supported → 501"
                    .to_owned(),
            ));
        }
        let l = work::right_copy(right, mode, |right| {
            let sp = &right.subplan_joins[0]; // is_single_subplan_branch checked this
                                              // Prior-OPTIONAL scan aliases AND prior LEFT-JOINed SubPlan aliases on this left
                                              // branch are nullable — a shared var reading one needs the R1 null-safe ON / R2
                                              // COALESCE. (Chained SubPlan-OPTIONALs: the SECOND correlates on the FIRST's
                                              // subplan var, which `nullable_aliases` now flags, and emit renders subplans in
                                              // order so the ON reference is valid SQL.)
            let prep = preparation::Preparation::new(&l, mode)?;
            // R1: shared-variable compatibility ON (NullSafeEq when the left side is a
            // prior-OPTIONAL nullable determinant, else the plain equality).
            let mut on = BuildVec::new(Vec::new());
            let mut disjoint = false;
            for (var, rdef) in &right.bindings {
                if let Some(ldef) = prep.lookup(&l.bindings, var)? {
                    let left_nullable = prep.nullable(ldef)?;
                    match work::unify_terms(mode, ldef, rdef)? {
                        Unify::Sat(conds) => {
                            for c in conds {
                                work.push(&mut on, conditions::null_safe(c, left_nullable, mode)?)?;
                            }
                        }
                        // Provably disjoint on a shared var ⇒ the OPTIONAL can never match ⇒
                        // right vars stay UNBOUND (absent); the left row survives unchanged.
                        Unify::Empty => {
                            disjoint = true;
                            break;
                        }
                        Unify::Unsupported(why) => return Err(Error::Unsupported(why)),
                    }
                }
            }
            if disjoint {
                return Ok(l);
            }
            // R2: COALESCE for a nullable-left shared var (its value comes from the right when
            // the left was unbound); plain right def for a right-only var (possibly NULL output).
            for (var, rdef) in &right.bindings {
                match prep.lookup(&l.bindings, var)? {
                    Some(ldef) if prep.nullable(ldef)? => {
                        materialization::coalesce_copied(&mut l.bindings, var, rdef, mode)?;
                    }
                    Some(_) => {} // mandatory-left shared var — value equals right by the ON
                    None => {
                        materialization::insert_copied(&mut l.bindings, var, rdef, mode)?;
                    }
                }
            }
            let mut sp2 = sp.clone();
            sp2.left = true;
            sp2.on = on.into_inner();
            work::push_owned(work, &mut l.subplan_joins, sp2)?;
            Ok(l)
        })?;
        work.push(&mut out, l)?;
    }
    Ok(out.into_inner())
}

/// Fallback for a `left_join_decomposed` right branch that still carries `opts`.
///
/// **This is currently UNREACHABLE dead code, retained only as a defensive boundary.**
/// It is called from exactly one site — [`left_join_decomposed`], guarded by
/// `right.iter().any(|r| !r.opts.is_empty())` — but NO branch fed there can carry
/// `opts`: the only producer of `opts` is `build_left_join` (via the single-scan fast
/// path of [`left_join_branches`]), which the tree invokes ONLY under `decompose ==
/// false`; under `decompose == true` a `LeftJoin` routes to [`left_join_decomposed`]
/// (never `build_left_join`), so its right operand — itself lowered with `decompose ==
/// true` — is always opts-free. Hence the guard at the call site never fires and this
/// function is never entered. Its `right.len() != 1` and `!r.opts.is_empty()` arms below
/// therefore both 501 defensively, and the inner-join/NOT-EXISTS "happy path" at the
/// bottom is DOUBLY dead (the `!r.opts.is_empty()` arm always precedes it given the
/// caller's own precondition). It is NOT the mechanism that closes any real
/// subplan-OPTIONAL shape: a SubPlan as the OPTIONAL's right operand is handled up front
/// by [`left_join_over_subplan`] (Item 1d), and a nested-OPTIONAL-inside-a-subselect that
/// this could conceivably see still 501s via `not_exists_cond_for`'s SubPlan boundary.
/// Left in place (rather than deleted) so the `left_join_decomposed` guard stays a total,
/// panic-free match; a future engineer should not mistake the happy path below for a live
/// code path.
pub(super) fn left_join_as_subplan(
    left: Vec<Branch>,
    right: Vec<Branch>,
    expr: Option<&spargebra::algebra::Expression>,
    dialect: sf_sql::Dialect,
    mode: CompilerWorkMode<'_>,
) -> Result<Vec<Branch>> {
    // Unreachable in practice (see the doc comment): a right branch here would need
    // non-empty `opts`, which the `decompose` invariant forbids. Both 501 arms below are
    // defensive; the inner-join/NOT-EXISTS tail is dead (the `!r.opts.is_empty()` arm
    // always fires first given the caller's precondition).
    if right.len() != 1 {
        return Err(Error::Unsupported(
            "LeftJoinJoinLimit: multi-branch right-side SubPlan not yet supported → 501 (M5 Wave 2 scope)"
                .to_owned(),
        ));
    }
    let r = right.into_iter().next().expect("len checked == 1");
    if !r.opts.is_empty() {
        return Err(Error::Unsupported(
            "LeftJoinJoinLimit: opts-carrying right branch → SubPlan LEFT JOIN not yet implemented → 501"
                .to_owned(),
        ));
    }
    let work = BuildWork::new(mode);
    work::candidates(mode, left.len(), 1, true)?;
    // Dead tail (see the doc comment): reached only if a caller ever passes a single
    // opts-free right branch, which the `left_join_decomposed` guard never does.
    let mut out = BuildVec::new(Vec::new());
    for mut l in left {
        if let Some(b) = inner_join_one(&l, &r, expr, dialect, mode)? {
            work.push(&mut out, b)?;
        }
        if let Some(cond) = not_exists_cond_for(&l, &r, expr, dialect, mode)? {
            work::push_owned(work, &mut l.where_conds, cond)?;
        }
        work.push(&mut out, l)?;
    }
    Ok(out.into_inner())
}
