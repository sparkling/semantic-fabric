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

use crate::iq::{Branch, CmpOp, OptJoin, SqlCond, TermDef};
use crate::unify::{filter_cond, unify, Unify};
use crate::{Error, Result};

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
    if right.len() == 1 && right[0].core.len() == 1 {
        let r = &right[0];
        let mut out = Vec::new();
        for mut l in left {
            if let Some(b) = build_left_join(&mut l, r, expr, dialect)? {
                out.push(b);
            } else {
                out.push(l); // optional never matches → left unchanged (right vars unbound)
            }
        }
        return Ok(out);
    }

    // Multi-branch or multi-scan right: P OPT R = (P ⋈ R) ∪ (P - R)
    // Handles both: OPTIONAL with multiple triples-map branches (UNION) and
    // OPTIONAL with multiple table scans (JOIN) within one branch.  Each Ri is
    // inner-joined with P; P rows with no Ri match go in the no-match branch.
    // = (P ⋈_NL R1) ∪ (P ⋈_NL R2) ∪ … ∪ P_no_match
    let mut out = Vec::new();
    for l in &left {
        // One inner-join branch per right branch.
        for r in &right {
            if let Some(b) = inner_join_one(l, r, expr, dialect)? {
                out.push(b);
            }
        }
        // No-match branch: L with NOT EXISTS for each Ri that can possibly match.
        let mut no_match = l.clone();
        for r in &right {
            if let Some(cond) = not_exists_cond_for(l, r, expr, dialect)? {
                no_match.where_conds.push(cond);
            }
        }
        out.push(no_match);
    }
    Ok(out)
}

/// Inner join of one left branch with one single-scan, opt-free right branch.
/// Returns `None` when unification proves the join empty (L ∩ R = ∅).
///
/// Uses NULL-safe WHERE conditions (`null_safe`) so a left variable that is
/// unbound (NULL from a prior OPTIONAL) matches any right value — the same
/// compatibility rule as the LEFT JOIN path.  R2 COALESCE bindings are applied
/// for nullable left shared variables so their value comes from the right side
/// when the left was unbound.
pub(crate) fn inner_join_one(
    left: &Branch,
    right: &Branch,
    expr: Option<&spargebra::algebra::Expression>,
    dialect: sf_sql::Dialect,
) -> Result<Option<Branch>> {
    // A property-path branch on EITHER side has no sound representation in the
    // merged `Branch` this function builds below: it carries only ONE `path`
    // field (`path: left.path.clone()`, unconditionally — confirmed live: a
    // `right.path` was silently dropped here, producing bindings that still
    // reference the path's own CTE-only `sf_s`/`sf_o` columns with `path: None`
    // on the merged branch, "no such column" at SQL-execution time), and even
    // adopting `right.path` instead would silently drop `left`'s own scans/
    // opts/where_conds the moment `emit_branch_with` dispatches on `b.path` to
    // `emit_path_branch` (which renders ONLY the path's own CTE + projection,
    // nothing else). Neither side can be merged into the other's shape without
    // losing data — sound 501 instead of a crash (ADR-0007); see
    // `not_exists_cond_for`'s matching guard for the `(P − R)` half of this
    // same `(P ⋈ R) ∪ (P − R)` decomposition.
    if left.path.is_some() || right.path.is_some() {
        return Err(Error::Unsupported(
            "OPTIONAL decomposition where either side is a property-path pattern \
             is not yet supported → 501"
                .to_owned(),
        ));
    }
    let opt_aliases: HashSet<usize> = left.opts.iter().map(|o| o.scan.alias).collect();
    let mut where_conds = left.where_conds.clone();
    let mut bindings = left.bindings.clone();

    // Shared-variable compatibility → NULL-safe WHERE conditions (R1 analogue).
    for (var, rdef) in &right.bindings {
        if let Some(ldef) = left.bindings.get(var) {
            let left_nullable = def_is_nullable(ldef, &opt_aliases);
            match unify(ldef, rdef) {
                Unify::Sat(conds) => {
                    for c in conds {
                        where_conds.push(null_safe(c, left_nullable));
                    }
                }
                Unify::Empty => return Ok(None),
                Unify::Unsupported(why) => return Err(Error::Unsupported(why)),
            }
        }
    }

    // Right-side own conditions.
    where_conds.extend(right.where_conds.iter().cloned());

    // FILTER inside the OPTIONAL goes in the inner-join WHERE (R5 analogue).
    if let Some(e) = expr {
        let mut combined = left.bindings.clone();
        for (v, d) in &right.bindings {
            combined.entry(v.clone()).or_insert_with(|| d.clone());
        }
        where_conds.push(filter_cond(e, &combined, dialect).map_err(Error::Unsupported)?);
    }

    // R2 binding merge: COALESCE for nullable left shared vars; plain right def
    // for right-only vars.
    for (var, rdef) in &right.bindings {
        match left.bindings.get(var) {
            Some(ldef) if def_is_nullable(ldef, &opt_aliases) => {
                bindings.insert(
                    var.clone(),
                    TermDef::Coalesce(Box::new(ldef.clone()), Box::new(rdef.clone())),
                );
            }
            Some(_) => {} // non-nullable left — value equals right by join condition
            None => {
                bindings.insert(var.clone(), rdef.clone());
            }
        }
    }

    // Merge scans: left core + all right scans.
    let mut core = left.core.clone();
    core.extend(right.core.iter().cloned());

    // SubPlan joins from both sides survive the merge (mirrors `unfold::merge`'s
    // InnerJoin idiom) — an OPTIONAL whose LEFT operand is a derived-table subquery
    // (e.g. `{SELECT … LIMIT n}`) must keep that join alive here; previously this
    // was unconditionally zeroed, dropping `left`'s subplan join and producing SQL
    // that references a FROM alias never introduced (ADR-0007).
    let mut subplan_joins = left.subplan_joins.clone();
    subplan_joins.extend(right.subplan_joins.iter().cloned());

    Ok(Some(Branch {
        core,
        opts: left.opts.clone(),
        bindings,
        where_conds,
        distinct: left.distinct,
        limit: left.limit,
        offset: left.offset,
        order: left.order.clone(),
        path: left.path.clone(),
        agg: left.agg.clone(),
        subplan_joins,
    }))
}

/// Build the `NOT EXISTS` condition for one right branch in the no-match branch
/// of a multi-branch OPTIONAL.  Returns `None` when unification proves the join
/// always empty (NOT EXISTS is trivially true — omit the condition).
///
/// The FILTER inside the OPTIONAL (`expr`) must gate the anti-join too — a
/// right row that EXISTS but FAILS the filter is not a match, so EXISTS must
/// be false there (⇒ NOT EXISTS true ⇒ left NULL-padded). Mirrors
/// `inner_join_one`'s combined-bindings filter application (R5 analogue)
/// exactly. Omitting it (as this function once did) made a left row whose
/// only right candidate is filtered out vanish from BOTH the match branch
/// (excluded by the filter) and this no-match branch (NOT EXISTS wrongly
/// false, since the unfiltered join still exists) — a silent wrong answer
/// (ADR-0007).
pub(crate) fn not_exists_cond_for(
    left: &Branch,
    right: &Branch,
    expr: Option<&spargebra::algebra::Expression>,
    dialect: sf_sql::Dialect,
) -> Result<Option<SqlCond>> {
    // A right branch carrying its own SubPlan (e.g. a nested OPTIONAL whose right
    // side is itself `(SubselectLimit) OPTIONAL (...)`, forcing the inner
    // decomposition to hand back a SubPlan-carrying branch here) has no
    // representation in `SqlCond::NotExists::scans` (a plain `Vec<Scan>` — the
    // subplan's derived-table alias would be referenced in `conds` below but never
    // introduced anywhere, producing a crash at SQL-execution time rather than a
    // wrong answer). `left.subplan_joins` is fine (it rides along via the caller's
    // `no_match = left.clone()`, same as any other outer-scope column); only a
    // subplan on the `right` side is unrepresentable here. Sound 501 instead of a
    // crash (ADR-0007) — an ADR-0023 M5 boundary, not yet a supported shape.
    if !right.subplan_joins.is_empty() {
        return Err(Error::Unsupported(
            "OPTIONAL anti-join whose right side carries its own SubPlan derived \
             table is not yet supported → 501 (ADR-0023 M5 boundary)"
                .to_owned(),
        ));
    }
    // A property-path branch (`path: Some(_)`) has NO representation in
    // `SqlCond::NotExists::scans` either, for the SAME reason as the SubPlan
    // case just above: its own rows come from a recursive-CTE derived table
    // (`sf_s`/`sf_o` columns), never `right.core`'s plain scans (which are
    // empty for a path branch — confirmed live: `right.core.clone()` renders
    // as `scans: []`, yet `conds` still references the path's own CTE-only
    // columns, producing "no such column" at SQL-execution time rather than a
    // wrong answer). A `left` path is equally unrepresentable — the merged
    // `no_match` branch this condition attaches to is `left.clone()` at the
    // call site, so a `left`-side path CTE would need to be preserved onto it
    // too, which nothing here does. Sound 501 instead of a crash (ADR-0007) —
    // an architectural gap (this is the P/R decomposition model, not a lowering
    // omission — see `inner_join_one`'s matching guard for the `(P ⋈ R)` half).
    if left.path.is_some() || right.path.is_some() {
        return Err(Error::Unsupported(
            "OPTIONAL anti-join where either side is a property-path pattern is \
             not yet supported → 501"
                .to_owned(),
        ));
    }
    let opt_aliases: HashSet<usize> = left.opts.iter().map(|o| o.scan.alias).collect();
    let mut conds: Vec<SqlCond> = right.where_conds.clone();

    for (var, rdef) in &right.bindings {
        if let Some(ldef) = left.bindings.get(var) {
            let left_nullable = def_is_nullable(ldef, &opt_aliases);
            match unify(ldef, rdef) {
                Unify::Sat(cond_list) => {
                    for c in cond_list {
                        conds.push(null_safe(c, left_nullable));
                    }
                }
                // Unification is impossible → this Ri can never match left →
                // NOT EXISTS is trivially true; skip.
                Unify::Empty => return Ok(None),
                Unify::Unsupported(why) => return Err(Error::Unsupported(why)),
            }
        }
    }

    // FILTER inside the OPTIONAL goes inside the NOT EXISTS too (R5 analogue):
    // same combined bindings `inner_join_one` uses for the match branch, so
    // both branches agree on what counts as "a match" — the tautological
    // identity `(L⋈R) ∪ (L¬∃R)` this decomposition relies on requires it.
    if let Some(e) = expr {
        let mut combined = left.bindings.clone();
        for (v, d) in &right.bindings {
            combined.entry(v.clone()).or_insert_with(|| d.clone());
        }
        conds.push(filter_cond(e, &combined, dialect).map_err(Error::Unsupported)?);
    }

    Ok(Some(SqlCond::NotExists {
        scans: right.core.clone(),
        conds,
    }))
}

/// Returns `Some(branch-with-OptJoin)`, or `None` when the shared variables prove
/// the optional can never match (so the caller keeps the left side as-is).
fn build_left_join(
    left: &mut Branch,
    right: &Branch,
    expr: Option<&spargebra::algebra::Expression>,
    dialect: sf_sql::Dialect,
) -> Result<Option<Branch>> {
    // A property-path `left` (the OPTIONAL's OWN preceding pattern) has no sound
    // representation once this function pushes `right`'s scan into `left.opts`
    // below: `left.path` is never touched here (this function only ever ADDS an
    // `OptJoin` onto whatever `left` already is), so the merged branch ends up
    // with BOTH `path: Some(_)` AND a non-empty `opts` — a combination
    // `emit_branch_with`'s dispatch on `b.path` routes to `emit_path_branch`,
    // which renders ONLY the path's own recursive CTE + projection and has no
    // concept of `opts` at all, silently dropping the OPTIONAL's own JOIN
    // clause (confirmed live: `no such column: t1.child` — the OPT's alias is
    // referenced in the SELECT list but its LEFT JOIN clause is never rendered).
    // `right` can never be path-shaped here (the caller only reaches this
    // function when `right.core.len() == 1`, and a path branch always has
    // `core.len() == 0`). Sound 501 instead of a crash (ADR-0007) — the same
    // architectural gap as `inner_join_one`'s/`not_exists_cond_for`'s matching
    // guards, found via a third, independent entry point (the single-scan fast
    // path, not the multi-branch decomposition).
    if left.path.is_some() {
        return Err(Error::Unsupported(
            "OPTIONAL whose preceding pattern is a property-path is not yet \
             supported → 501"
                .to_owned(),
        ));
    }
    let mut on = Vec::new();
    let mut extra = right.where_conds.clone(); // constant-position constraints stay in the ON (R5)
                                               // Prior-OPTIONAL aliases on the preserved (left) side: a shared var whose left def
                                               // reads one of these can be UNBOUND, so its ON equality needs the NULL-safe guard.
    let opt_aliases: HashSet<usize> = left.opts.iter().map(|o| o.scan.alias).collect();
    for (var, rdef) in &right.bindings {
        if let Some(ldef) = left.bindings.get(var) {
            let left_nullable = def_is_nullable(ldef, &opt_aliases);
            match unify(ldef, rdef) {
                Unify::Sat(conds) => {
                    for c in conds {
                        on.push(null_safe(c, left_nullable)); // R1: shared-var compat, never plain a = b
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
fn null_safe(c: SqlCond, left_nullable: bool) -> SqlCond {
    if !left_nullable {
        return c;
    }
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
        // An aggregate result is produced post-grouping, never under an OPTIONAL.
        TermDef::Agg { .. } => false,
    }
}
