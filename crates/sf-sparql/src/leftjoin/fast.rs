use super::*;

/// Consumes and returns `left`, adding an [`OptJoin`] when the shared variables
/// can match and otherwise leaving the branch unchanged.
pub(super) fn build_left_join(
    mut left: Branch,
    right: &Branch,
    expr: Option<&spargebra::algebra::Expression>,
    dialect: sf_sql::Dialect,
    mode: CompilerWorkMode<'_>,
) -> Result<Branch> {
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
    // A `right` carrying its OWN SubPlan derived table (e.g. it is itself the product
    // of a nested subplan-OPTIONAL — `{ ?a p ?nm OPTIONAL { <subSELECT> } }` — which
    // `left_join_over_subplan` lowered to a core-scan-PLUS-`subplan_joins` branch) has
    // no representation on this single-scan fast path: it pushes `right.core[0]` as an
    // `OptJoin` but has NOWHERE to carry `right.subplan_joins` — the derived-table alias
    // `t{sp}` would stay referenced by the merged bindings (a right-only var maps to
    // `t{sp}.c{i}`) while no FROM clause ever introduces it ("no such column t{sp}.c{i}"
    // at SQL-execution time). Merely copying `right.subplan_joins` onto the merged branch
    // is NOT sound either: the SubPlan's `on` correlation references `right.core[0]` (now
    // a LEFT-JOINed opt), and a shared variable whose RIGHT def reads the SubPlan alias
    // would place a `t{sp}` reference into the OptJoin's own ON — a table emitted to its
    // right. Sound 501 instead of a crash (ADR-0007), mirroring `not_exists_cond_for`'s
    // matching `!right.subplan_joins.is_empty()` boundary for the `(P − R)` anti-join half.
    if !right.subplan_joins.is_empty() {
        return Err(Error::Unsupported(
            "OPTIONAL whose right side is a nested subplan-OPTIONAL (a branch carrying \
             its own SubPlan derived table) is not yet supported → 501 \
             (ADR-0023 Item 1d boundary)"
                .to_owned(),
        ));
    }
    let prep = preparation::Preparation::new(&left, mode)?;
    if prep.shared_reads_left_subplan(&left, right)? {
        return Err(Error::Unsupported(SHARED_LEFT_SUBPLAN_501.to_owned()));
    }
    work::right_copy(right, mode, |right| {
        let mut on = BuildVec::new(Vec::new());
        let mut extra = right.where_conds.clone(); // constant-position constraints stay in the ON (R5)
                                                   // Prior-OPTIONAL aliases on the preserved (left) side: a shared var whose left def
                                                   // reads one of these can be UNBOUND, so its ON equality needs the NULL-safe guard.
        for (var, rdef) in &right.bindings {
            if let Some(ldef) = prep.lookup(&left.bindings, var)? {
                let left_nullable = prep.nullable(ldef)?;
                match unify(ldef, rdef) {
                    Unify::Sat(conds) => {
                        for c in conds {
                            BuildWork::new(mode)
                                .push(&mut on, conditions::null_safe(c, left_nullable, mode)?)?;
                        }
                    }
                    Unify::Empty => return Ok(left),
                    Unify::Unsupported(why) => return Err(Error::Unsupported(why)),
                }
            }
        }
        // Build the FILTER view directly in the owned output map. Right-only
        // definitions can move into their final location immediately; nullable
        // shared replacements stay deferred so FILTER lowering sees the same
        // left-preferred combined bindings as before.
        let mut nullable_shared = BuildVec::new(Vec::new());
        // R2 projection (ADR-0007). Prior-OPTIONAL aliases are nullable. A shared
        // variable whose preserved (left) side can be NULL (a nested OPTIONAL) becomes
        // COALESCE(left, right) so the right value survives when left is unbound; a
        // mandatory-left shared var is never NULL (COALESCE(left,right)=left) so we keep
        // the simpler left def; a right-only var is the (possibly NULL) right output.
        for (var, rdef) in &right.bindings {
            match prep.lookup(&left.bindings, var)? {
                Some(ldef) if prep.nullable(ldef)? => {
                    BuildWork::new(mode).push(&mut nullable_shared, (var.as_str(), rdef))?;
                }
                Some(_) => {}
                None => {
                    left.bindings.insert(var.clone(), rdef.clone());
                }
            }
        }
        // Combined bindings for the inner FILTER (R5: it goes in the ON, not WHERE).
        if let Some(e) = expr {
            extra.push(
                filter_scopes(e, &left.bindings, dialect, &[&left, right])
                    .map_err(Error::Unsupported)?,
            );
        }
        for (var, rdef) in nullable_shared.into_inner() {
            let (var, ldef) = left
                .bindings
                .remove_entry(var)
                .expect("nullable shared binding came from the left branch");
            left.bindings.insert(
                var,
                TermDef::Coalesce(Box::new(ldef), Box::new(rdef.clone())),
            );
        }
        work::push_owned(
            BuildWork::new(mode),
            &mut left.opts,
            OptJoin {
                scan: right.core[0].clone(),
                on: on.into_inner(),
                extra,
            },
        )?;
        Ok(left)
    })
}
