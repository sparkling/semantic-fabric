use super::control::{RowVec, RowWork};
use super::values::{constant_row_from_subst, constant_subst_can_form_row};
use crate::iq::node::{IqNode, Var};
use crate::iq::TermDef;
use crate::{CompilerWorkMode, Result};

// ---- (d) Slice-over-Values truncation (ADR-0023 optimizer-residue Wave C; Ontop
// ValuesNodeOptimization::test1/test2normalizationSlice) --------------------------

/// Truncate a literal `Values` table directly when a `Slice` sits over it (possibly
/// through the identity/pure-projection `Construction` the builder wraps a top-level
/// `SELECT`'s VALUES body in — the common case, confirmed empirically: `SELECT ?x
/// WHERE { VALUES ?x {…} } LIMIT n` normalizes to `Slice(Construction(Values))`, not
/// a bare `Slice(Values)`), instead of carrying the `Slice` down to LOWER (which would
/// lower every row to its own branch and apply offset/limit as a `Plan`-level SQL
/// clause over the full arm count). A `Values` block's rows have a fixed as-written
/// order and nothing else can reorder them upstream of this `Slice`, so reading the
/// same `[offset, offset+limit)` window in Rust here is `=_bag`-identical to lowering
/// the full table and slicing at emission — just without ever materializing the
/// dropped rows/branches (cosmetic SQL-shape parity with Ontop, not a correctness fix;
/// §4.15-adjacent, not a currently-numbered §4 rule). The `Construction`'s `subst`/
/// `project` apply per-row and don't reorder rows, so they commute with slicing
/// unconditionally. Any other child shape keeps the `Slice` node as-is.
pub(super) fn normalize_slice(
    offset: usize,
    limit: Option<usize>,
    child: IqNode,
    mode: CompilerWorkMode<'_>,
) -> Result<IqNode> {
    let work = RowWork::new(mode);
    work.charge(1)?;
    let result = match child {
        IqNode::Values { vars, rows } => IqNode::Values {
            vars,
            rows: slice_rows(rows, offset, limit, work)?,
        },
        IqNode::Union { children, project } => {
            normalize_slice_over_union(offset, limit, children, project, work)?
        }
        IqNode::Construction {
            child: inner,
            subst,
            project,
        } if matches!(*inner, IqNode::Values { .. } | IqNode::Union { .. }) => {
            IqNode::Construction {
                child: work.boxed(normalize_slice(offset, limit, *inner, mode)?)?,
                subst,
                project,
            }
        }
        child => IqNode::Slice {
            child: work.boxed(child)?,
            offset,
            limit,
        },
    };
    work.checkpoint()?;
    Ok(result)
}

/// The `[offset, offset+limit)` window of `rows` (`limit == None` ⇒ to the end).
fn slice_rows(
    rows: Vec<Vec<Option<TermDef>>>,
    offset: usize,
    limit: Option<usize>,
    work: RowWork<'_>,
) -> Result<Vec<Vec<Option<TermDef>>>> {
    work.charge(1)?;
    let start = offset.min(rows.len());
    let keep = (rows.len() - start).min(limit.unwrap_or(usize::MAX));
    let mut out = work.vector(keep)?;
    for (index, row) in rows.into_iter().enumerate() {
        // Logical visits and retained slots, not recursive payload destruction.
        work.charge(1)?;
        if index >= start && index - start < keep {
            out.push(row);
        }
    }
    work.checkpoint()?;
    Ok(out)
}

// ---- (d2) Slice-over-Union arm-drop / residual-limit (ADR-0023 optimizer-residue
// Wave C; Ontop ValuesNodeOptimization::test5-7SliceUnionValuesNonValues) ---------

/// An arm's statically-known row set matching `project` in EXACT column order: a
/// bare `Values` leaf, or a single-row all-constant `Construction{child: True, ...}`
/// (the same two shapes `fold_constant_union` recognizes for a WHOLE `Union`,
/// generalized here to one arm at a time — a mixed `Union` with a genuine DATA arm
/// declines the full constant fold entirely, so an all-constant arm can still
/// reach this function unfolded). `None` ⇒ unknown cardinality (a real pattern, or
/// a column-order mismatch this rule doesn't reconcile — cf. test26).
fn static_row_count(arm: &IqNode, project: &[Var], work: RowWork<'_>) -> Result<Option<usize>> {
    work.charge(1)?;
    if let IqNode::Values { vars, rows } = arm {
        return Ok(work.vars_equal(vars, project)?.then_some(rows.len()));
    }
    // A lone `VALUES` block as a Union arm is ALSO wrapped in the builder's identity-
    // projection `Construction` (confirmed empirically — the same pattern `normalize_
    // slice`/`normalize_distinct` already handle for the top-level-body case, but here
    // for one arm among several). `child.as_ref()` decides which of the two shapes
    // this is; no ambiguity between them (`Values`/`True` are distinct variants).
    let IqNode::Construction {
        child,
        subst,
        project: arm_project,
    } = arm
    else {
        return Ok(None);
    };
    Ok(match child.as_ref() {
        IqNode::Values { vars, rows }
            if subst.is_empty()
                && work.vars_equal(arm_project, project)?
                && work.vars_equal(vars, project)? =>
        {
            Some(rows.len())
        }
        IqNode::True if constant_subst_can_form_row(subst, project, work)? => Some(1),
        _ => None,
    })
}

fn into_static_rows(
    arm: IqNode,
    project: &[Var],
    work: RowWork<'_>,
) -> Result<Option<Vec<Vec<Option<TermDef>>>>> {
    work.charge(1)?;
    Ok(match arm {
        IqNode::Values { vars, rows } if work.vars_equal(&vars, project)? => Some(rows),
        IqNode::Construction {
            child,
            subst,
            project: arm_project,
        } => match *child {
            IqNode::Values { vars, rows }
                if subst.is_empty()
                    && work.vars_equal(&arm_project, project)?
                    && work.vars_equal(&vars, project)? =>
            {
                Some(rows)
            }
            IqNode::True => {
                let Some(row) = constant_row_from_subst(subst, project, work)? else {
                    return Ok(None);
                };
                let mut rows = work.vector(1)?;
                rows.push(row);
                Some(rows)
            }
            _ => None,
        },
        _ => None,
    })
}

#[derive(Clone, Copy)]
struct SliceUnionPlan {
    known_arms: usize,
    drop_remaining: bool,
}

fn plan_slice_over_union(
    offset: usize,
    limit: Option<usize>,
    arms: &[IqNode],
    project: &[Var],
    work: RowWork<'_>,
) -> Result<Option<SliceUnionPlan>> {
    work.charge(1)?;
    let limit_n = limit.unwrap_or(usize::MAX);
    let window_end = offset.saturating_add(limit_n);
    let mut cursor = 0usize;
    let mut survivor_count = 0usize;
    let mut i = 0;
    let mut changed = false;

    while i < arms.len() {
        work.charge(1)?;
        if survivor_count >= limit_n {
            return Ok(Some(SliceUnionPlan {
                known_arms: i,
                drop_remaining: true,
            }));
        }
        let Some(row_count) = static_row_count(&arms[i], project, work)? else {
            break;
        };
        let local_start = offset.saturating_sub(cursor).min(row_count);
        let local_end = window_end.saturating_sub(cursor).min(row_count);
        if local_start > 0 || local_end < row_count {
            changed = true;
        }
        survivor_count += local_end.saturating_sub(local_start);
        cursor += row_count;
        i += 1;
    }

    Ok(changed.then_some(SliceUnionPlan {
        known_arms: i,
        drop_remaining: false,
    }))
}

/// Resolve a `Slice(offset, limit)` over a `Union` whose LEADING arms have a
/// statically-known row count ([`static_row_count`]) by walking them in as-written
/// order, tracking `cursor` (the true row count consumed so far) and `survivors`
/// (the rows the `[offset, offset+limit)` window actually keeps from them):
///
/// * An arm entirely BEFORE the window (`cursor + n <= offset`) contributes nothing
///   — dropped outright.
/// * An arm straddling the window contributes its own local `[offset-cursor,
///   limit_end-cursor)` slice — dropped down to just the surviving rows.
/// * Once `survivors.len()` already reaches `limit`, every remaining arm is
///   unreachable regardless of its own size or kind — dropped, INCLUDING an
///   unknown-cardinality one.
/// * An unknown-cardinality arm reached BEFORE the window is satisfied means we
///   genuinely cannot tell how many (if any) of its rows are needed — processing
///   STOPS there: that arm and everything after survives untouched, under a fresh
///   `Slice(0, limit)` over `[the truncated survivor prefix (if non-empty), the
///   unknown arm, ...the rest]` — offset 0 because the survivors already account
///   for everything before them in as-written order, so a plain Slice-over-Union
///   (whose own semantics already read arms in order from position 0) does the
///   right thing for whatever follows, at any depth, with no further bookkeeping.
///
/// Keeps the original owned `Slice{Union}` when nothing at all could be dropped or
/// truncated — e.g. the first arm is already unknown and the window is not yet
/// satisfied — matching every other rule's sound-decline convention.
fn normalize_slice_over_union(
    offset: usize,
    limit: Option<usize>,
    arms: Vec<IqNode>,
    project: Vec<Var>,
    work: RowWork<'_>,
) -> Result<IqNode> {
    let Some(plan) = plan_slice_over_union(offset, limit, &arms, &project, work)? else {
        return Ok(IqNode::Slice {
            child: work.boxed(IqNode::Union {
                children: arms,
                project,
            })?,
            offset,
            limit,
        });
    };

    let limit_n = limit.unwrap_or(usize::MAX);
    let window_end = offset.saturating_add(limit_n);
    let mut cursor = 0usize;
    let mut survivors = RowVec::new(Vec::new());
    let mut source = arms.into_iter();

    for _ in 0..plan.known_arms {
        work.charge(1)?;
        let rows = into_static_rows(
            source.next().expect("slice plan mirrors source arms"),
            &project,
            work,
        )?
        .expect("static_row_count admitted every planned arm");
        let row_count = rows.len();
        let local_start = offset.saturating_sub(cursor).min(row_count);
        let local_end = window_end.saturating_sub(cursor).min(row_count);
        work.append(
            &mut survivors,
            slice_rows(
                rows,
                local_start,
                Some(local_end.saturating_sub(local_start)),
                work,
            )?,
        )?;
        cursor += row_count;
    }

    let remaining = if plan.drop_remaining {
        work.charge(source.len())?;
        Vec::new()
    } else {
        let mut remaining = work.vector(source.len())?;
        for arm in source {
            work.charge(1)?;
            remaining.push(arm);
        }
        remaining
    };

    Ok(if remaining.is_empty() {
        IqNode::Values {
            vars: project,
            rows: survivors.into_inner(),
        }
    } else {
        let mut new_arms = RowVec::new(Vec::new());
        if !survivors.values.is_empty() {
            work.push(
                &mut new_arms,
                IqNode::Values {
                    vars: work.mode.clone_variables(&project)?,
                    rows: survivors.into_inner(),
                },
            )?;
        }
        work.append(&mut new_arms, remaining)?;
        let mut new_arms = new_arms.into_inner();
        let child = if new_arms.len() == 1 {
            new_arms.pop().expect("len checked == 1")
        } else {
            IqNode::Union {
                children: new_arms,
                project,
            }
        };
        IqNode::Slice {
            child: work.boxed(child)?,
            // NOT unconditionally 0: an adversarial review caught that the known
            // arms' cumulative row count (`cursor`) may still fall SHORT of the
            // original `offset` when it isn't enough to cover the whole skip on its
            // own (e.g. `offset=4` over a single known 3-row arm then a data arm --
            // all 3 rows are dropped, but 1 MORE row of skip still needs to land on
            // the data arm itself). `survivors` only ever holds what's already
            // correctly positioned at `offset` or later, so the residual is exactly
            // however much of `offset` the known prefix DIDN'T already consume.
            offset: offset.saturating_sub(cursor),
            limit,
        }
    })
}
