use super::*;

/// Inner join of one left branch with one single-scan, opt-free right branch.
/// Returns `None` when unification proves the join empty (L ∩ R = ∅).
///
/// Uses NULL-safe WHERE conditions (`null_safe`) so a left variable that is
/// unbound (NULL from a prior OPTIONAL) matches any right value — the same
/// compatibility rule as the LEFT JOIN path.  R2 COALESCE bindings are applied
/// for nullable left shared variables so their value comes from the right side
/// when the left was unbound.
#[cfg(test)]
pub(crate) fn inner_join_one(
    left: &Branch,
    right: &Branch,
    expr: Option<&spargebra::algebra::Expression>,
    dialect: sf_sql::Dialect,
) -> Result<Option<Branch>> {
    inner_join_one_with_work_mode(left, right, expr, dialect, CompilerWorkMode::Uncontrolled)
}

pub(crate) fn inner_join_one_with_work_mode(
    left: &Branch,
    right: &Branch,
    expr: Option<&spargebra::algebra::Expression>,
    dialect: sf_sql::Dialect,
    mode: CompilerWorkMode<'_>,
) -> Result<Option<Branch>> {
    // A property-path branch on EITHER side has no sound representation in the
    // merged `Branch` this function builds below: it carries only ONE `path`
    // field. A `right.path` was previously silently dropped here, producing
    // bindings that still reference the path's own CTE-only `sf_s`/`sf_o`
    // columns with `path: None`
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
    let prep = preparation::Preparation::new(left, mode)?;
    if prep.shared_reads_left_subplan(left, right)? {
        return Err(Error::Unsupported(SHARED_LEFT_SUBPLAN_501.to_owned()));
    }
    work::copies(left, right, true, mode, |left, right| {
        let mut where_conds = left.where_conds.clone();
        let mut bindings = left.bindings.clone();

        // Shared-variable compatibility → NULL-safe WHERE conditions (R1 analogue).
        for (var, rdef) in &right.bindings {
            if let Some(ldef) = prep.lookup(&left.bindings, var)? {
                let left_nullable = prep.nullable(ldef)?;
                match work::unify_terms(mode, ldef, rdef)? {
                    Unify::Sat(conds) => {
                        for c in conds {
                            work::push_owned(
                                BuildWork::new(mode),
                                &mut where_conds,
                                conditions::null_safe(c, left_nullable, mode)?,
                            )?;
                        }
                    }
                    Unify::Empty => return Ok(None),
                    Unify::Unsupported(why) => return Err(Error::Unsupported(why)),
                }
            }
        }

        // Right-side own conditions.
        where_conds.extend(right.where_conds.iter().cloned());

        // Prepare the combined FILTER view in the output map itself. Shared vars
        // retain their original left definition until after FILTER lowering; the
        // deferred replacements below then install R2 COALESCE definitions.
        let mut nullable_shared = BuildVec::new(Vec::new());
        for (var, rdef) in &right.bindings {
            match prep.lookup(&left.bindings, var)? {
                Some(ldef) if prep.nullable(ldef)? => {
                    BuildWork::new(mode).push(&mut nullable_shared, (var.as_str(), rdef))?;
                }
                Some(_) => {} // non-nullable left — value equals right by join condition
                None => {
                    bindings.insert(var.clone(), rdef.clone());
                }
            }
        }

        // FILTER inside the OPTIONAL goes in the inner-join WHERE (R5 analogue).
        if let Some(e) = expr {
            where_conds.push(
                filter_scopes(e, &bindings, dialect, &[left, right]).map_err(Error::Unsupported)?,
            );
        }

        for (var, rdef) in nullable_shared.into_inner() {
            let (var, ldef) = bindings
                .remove_entry(var)
                .expect("nullable shared binding came from the left branch");
            bindings.insert(
                var,
                TermDef::Coalesce(Box::new(ldef), Box::new(rdef.clone())),
            );
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
            path: None,
            agg: left.agg.clone(),
            subplan_joins,
            nps: left.nps,
        }))
    })
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
#[cfg(test)]
pub(crate) fn not_exists_cond_for(
    left: &Branch,
    right: &Branch,
    expr: Option<&spargebra::algebra::Expression>,
    dialect: sf_sql::Dialect,
) -> Result<Option<SqlCond>> {
    not_exists_cond_for_with_work_mode(left, right, expr, dialect, CompilerWorkMode::Uncontrolled)
}

pub(crate) fn not_exists_cond_for_with_work_mode(
    left: &Branch,
    right: &Branch,
    expr: Option<&spargebra::algebra::Expression>,
    dialect: sf_sql::Dialect,
    mode: CompilerWorkMode<'_>,
) -> Result<Option<SqlCond>> {
    // A right branch carrying its own SubPlan (e.g. a nested OPTIONAL whose right
    // side is itself `(SubselectLimit) OPTIONAL (...)`, forcing the inner
    // decomposition to hand back a SubPlan-carrying branch here) has no
    // representation in `SqlCond::NotExists::scans` (a plain `Vec<Scan>` — the
    // subplan's derived-table alias would be referenced in `conds` below but never
    // introduced anywhere, producing a crash at SQL-execution time rather than a
    // wrong answer). `left.subplan_joins` is fine (it rides along on the caller's
    // owned no-match branch, like any other outer-scope column); only a
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
    // owned `left` branch this condition attaches to is the no-match result at
    // the call site, so a `left`-side path CTE would need to be preserved on it
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
    let prep = preparation::Preparation::new(left, mode)?;
    if prep.shared_reads_left_subplan(left, right)? {
        return Err(Error::Unsupported(SHARED_LEFT_SUBPLAN_501.to_owned()));
    }
    work::copies(left, right, expr.is_some(), mode, |left, right| {
        let mut conds: Vec<SqlCond> = right.where_conds.clone();

        for (var, rdef) in &right.bindings {
            if let Some(ldef) = prep.lookup(&left.bindings, var)? {
                let left_nullable = prep.nullable(ldef)?;
                match work::unify_terms(mode, ldef, rdef)? {
                    Unify::Sat(cond_list) => {
                        for c in cond_list {
                            work::push_owned(
                                BuildWork::new(mode),
                                &mut conds,
                                conditions::null_safe(c, left_nullable, mode)?,
                            )?;
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
            conds.push(
                filter_scopes(e, &combined, dialect, &[left, right]).map_err(Error::Unsupported)?,
            );
        }

        Ok(Some(SqlCond::NotExists {
            scans: right.core.clone(),
            conds,
        }))
    })
}
