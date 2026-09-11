//! Controlled post-cascade traversal and raw-column projection inventories.
//! The optimizer's own run remains a separately open compiler boundary.
use sf_sql::TableSchema;

use crate::build::control::{BuildVec, BuildWork};
use crate::iq::{Branch, ColRef};
use crate::{cascade, Result};

#[path = "exec_core/dedup_scope_work.rs"]
pub(crate) mod scope;

#[cfg(test)]
#[path = "finalization_tests.rs"]
mod tests;

fn column(out: &mut BuildVec<ColRef>, alias: usize, name: &str, work: BuildWork<'_>) -> Result<()> {
    for existing in &out.values {
        work.charge(1)?;
        if existing.alias == alias {
            work.charge(existing.column.len().min(name.len()))?;
            if existing.column.as_ref() == name {
                return Ok(());
            }
        }
    }
    let column = work.string(name)?.into_boxed_str();
    work.push(out, ColRef { alias, column })
}

/// Same first-occurrence ordering and DISTINCT condition suppression as
/// Branch::projection, with borrowed recursive column recipes.
pub(crate) fn projection(branch: &Branch, work: BuildWork<'_>) -> Result<Vec<ColRef>> {
    work.charge(1)?;
    let mut out = BuildVec::new(Vec::new());
    for definition in branch.bindings.values() {
        work.charge(1)?;
        for (alias, name) in cascade::term_columns_with_work(definition, work)? {
            column(&mut out, alias, name, work)?;
        }
    }
    if !branch.distinct {
        for condition in &branch.where_conds {
            cascade::condition_columns_with_work(condition, work, &mut |alias, name| {
                column(&mut out, alias, name, work)
            })?;
        }
        for opt in &branch.opts {
            work.charge(1)?;
            for condition in opt.on.iter().chain(&opt.extra) {
                cascade::condition_columns_with_work(condition, work, &mut |alias, name| {
                    column(&mut out, alias, name, work)
                })?;
            }
        }
        for nested in &branch.subplan_joins {
            work.charge(1)?;
            for condition in &nested.on {
                cascade::condition_columns_with_work(condition, work, &mut |alias, name| {
                    column(&mut out, alias, name, work)
                })?;
            }
        }
    }
    work.checkpoint()?;
    Ok(out.into_inner())
}

pub(crate) fn subplans(
    branch: &mut Branch,
    schema: &[TableSchema],
    work: BuildWork<'_>,
) -> Result<()> {
    let work = work.enter()?;
    for nested in &mut branch.subplan_joins {
        work.charge(1)?;
        let context = cascade::CascadeCtx {
            distinct: false,
            project: None,
        };
        let original_count = nested.plan.branches.len();
        let candidate = work.mode.clone_branch_forest(&nested.plan.branches)?;
        // Preserve all existing optimizer passes, including their ordering.
        // This call is not claimed as internally metered by finalization.
        let post = cascade::run(candidate, schema, &context);
        work.checkpoint()?;
        let mut widths = work.vector(post.len())?;
        for branch in &post {
            widths.push(projection(branch, work)?.len());
        }
        work.charge(1)?;
        let safe = if original_count == 1 {
            true
        } else if post.len() != original_count {
            false
        } else {
            let mut equal = true;
            for pair in widths.windows(2) {
                work.charge(1)?;
                if pair[0] != pair[1] {
                    equal = false;
                    break;
                }
            }
            equal
        };
        if safe {
            nested.plan.branches = post;
        }
        for inner in &mut nested.plan.branches {
            subplans(inner, schema, work)?;
        }
    }
    work.checkpoint()
}
