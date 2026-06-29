//! Pass (2c) of the optimizer cascade — "same terms" self-join elimination
//! under `DISTINCT` (ADR-0022, WAVE 1: Ontop `SelfJoinSameTermsTest`).
//! Kept in its own file to hold pass 2 within the size budget.

use sf_core::ir::LogicalSource;

use crate::iq::{collect_cond_cols, Branch, CmpOp, ColRef, SqlCond};

use super::{rewrite_def_alias, CascadeCtx};

/// Eliminate redundant same-table scans under `DISTINCT` (ADR-0022, WAVE 1).
///
/// When multiple scans of the same table are joined and the join columns form
/// the entire projected output, `DISTINCT` makes the extra scans redundant —
/// only the most-constrained scan need survive.
///
/// Sound proof: the join result with `DISTINCT` projects only the shared
/// columns; a single, more-constrained scan with `IS NOT NULL` guards on those
/// columns produces the same distinct tuples (a bag of projections under
/// `DISTINCT` equals the set). Scan b is dropped in favour of scan a when
/// (i) every projected variable that references b is reachable from a via an
/// existing `ColEq`, and (ii) b has no **uncovered** local conditions — i.e.
/// every local `Cmp`/`IsNotNull`/`IsNull` of b has an equivalent on a. A sound
/// no-op when the precondition fails.
pub(super) fn same_terms_elimination(b: &mut Branch, ctx: &CascadeCtx) {
    if !ctx.distinct {
        return;
    }
    while let Some((keep, drop, cond_idxs, guards)) = find_same_terms_eliminable(b, ctx.project) {
        // Remove cross-scan ColEq conditions (highest index first).
        for idx in cond_idxs.iter().copied().rev() {
            b.where_conds.remove(idx);
        }
        // Remove any remaining conditions that reference ONLY the drop alias
        // (verified "covered" by equivalent conditions on keep).
        b.where_conds.retain(|cond| {
            let mut refs_drop = false;
            let mut refs_other = false;
            collect_cond_cols(cond, &mut |c| {
                if c.alias == drop {
                    refs_drop = true;
                } else {
                    refs_other = true;
                }
            });
            !refs_drop || refs_other
        });
        // Add IS NOT NULL guards for variable shared columns.
        for guard in guards {
            if !b
                .where_conds
                .iter()
                .any(|c| matches!(c, SqlCond::IsNotNull(x) if x == &guard))
            {
                b.where_conds.push(SqlCond::IsNotNull(guard));
            }
        }
        // Rewrite drop→keep in bindings only (conditions already cleaned).
        for def in b.bindings.values_mut() {
            rewrite_def_alias(def, drop, keep);
        }
        b.core.retain(|s| s.alias != drop);
    }
}

/// Find `(keep, drop, cond_indices, guard_cols)` for `same_terms_elimination`,
/// or `None` if no eliminable pair exists.
fn find_same_terms_eliminable(
    b: &Branch,
    project: Option<&[String]>,
) -> Option<(usize, usize, Vec<usize>, Vec<ColRef>)> {
    let is_projected = |var: &str| project.is_none_or(|p| p.iter().any(|v| v == var));

    for i in 0..b.core.len() {
        let LogicalSource::Table(ti) = &b.core[i].source else {
            continue;
        };
        let keep = b.core[i].alias;

        'drop_loop: for j in 0..b.core.len() {
            if i == j {
                continue;
            }
            let LogicalSource::Table(tj) = &b.core[j].source else {
                continue;
            };
            if ti != tj {
                continue;
            }
            let drop = b.core[j].alias;

            // Drop scan must not appear in any OptJoin.
            if b.opts.iter().any(|opt| {
                opt.scan.alias == drop
                    || opt.on.iter().any(|c| cond_refs_alias(c, drop))
                    || opt.extra.iter().any(|c| cond_refs_alias(c, drop))
            }) {
                continue 'drop_loop;
            }

            // Collect cross-scan ColEq indices and keep-side column refs.
            let mut cross_idxs: Vec<usize> = Vec::new();
            let mut shared_keep_cols: Vec<ColRef> = Vec::new();
            for (idx, cond) in b.where_conds.iter().enumerate() {
                let SqlCond::ColEq(a, c) = cond else {
                    continue;
                };
                if a.alias == keep && c.alias == drop {
                    cross_idxs.push(idx);
                    shared_keep_cols.push(a.clone());
                } else if a.alias == drop && c.alias == keep {
                    cross_idxs.push(idx);
                    shared_keep_cols.push(c.clone());
                }
            }

            // Every projected binding referencing drop must be covered by a ColEq.
            for (var, def) in &b.bindings {
                if !is_projected(var) {
                    continue;
                }
                for dc in def.columns() {
                    if dc.alias != drop {
                        continue;
                    }
                    let covered = b.where_conds.iter().any(|cond| {
                        let SqlCond::ColEq(a, c) = cond else {
                            return false;
                        };
                        (a.alias == keep && c.alias == drop && c.column == dc.column)
                            || (c.alias == keep && a.alias == drop && a.column == dc.column)
                    });
                    if !covered {
                        continue 'drop_loop;
                    }
                }
            }

            // Drop must have no local conditions not covered by an equivalent on keep.
            for (idx, cond) in b.where_conds.iter().enumerate() {
                if cross_idxs.contains(&idx) {
                    continue;
                }
                let mut refs_drop = false;
                let mut refs_other = false;
                collect_cond_cols(cond, &mut |c| {
                    if c.alias == drop {
                        refs_drop = true;
                    } else {
                        refs_other = true;
                    }
                });
                if !refs_drop || refs_other {
                    continue;
                }
                if !same_cond_on_keep(&b.where_conds, cond, drop, keep) {
                    continue 'drop_loop;
                }
            }

            // IS NOT NULL guards: shared keep-side columns with no constant equality.
            let guard_cols: Vec<ColRef> = shared_keep_cols
                .iter()
                .filter(|kc| {
                    !b.where_conds
                        .iter()
                        .any(|cond| matches!(cond, SqlCond::Cmp(c, CmpOp::Eq, _) if c == *kc))
                })
                .cloned()
                .collect();

            return Some((keep, drop, cross_idxs, guard_cols));
        }
    }
    None
}

/// Does a drop-only condition have an equivalent on keep?
/// Handles `Cmp`, `IsNotNull`, `IsNull` (the only simple local conditions on base scans).
fn same_cond_on_keep(conds: &[SqlCond], drop_cond: &SqlCond, drop: usize, keep: usize) -> bool {
    conds.iter().any(|c| match (c, drop_cond) {
        (SqlCond::Cmp(ec, eop, eval), SqlCond::Cmp(dc, dop, dval)) => {
            eop == dop
                && eval == dval
                && ec.column == dc.column
                && ec.alias == keep
                && dc.alias == drop
        }
        (SqlCond::IsNotNull(ec), SqlCond::IsNotNull(dc)) => {
            ec.column == dc.column && ec.alias == keep && dc.alias == drop
        }
        (SqlCond::IsNull(ec), SqlCond::IsNull(dc)) => {
            ec.column == dc.column && ec.alias == keep && dc.alias == drop
        }
        _ => false,
    })
}

/// Does `cond` reference `alias` anywhere?
pub(super) fn cond_refs_alias(cond: &SqlCond, alias: usize) -> bool {
    let mut found = false;
    collect_cond_cols(cond, &mut |c| {
        if c.alias == alias {
            found = true;
        }
    });
    found
}
