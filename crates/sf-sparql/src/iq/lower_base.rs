//! Owned LOWER leaves and the INNER identity use existing request work controls.
//! Logical carriers/comparisons are admitted; moved payload and physical drop
//! are not cloned or claimed to have an allocator/destruction bound here.
use super::*;
use crate::leftjoin::work::binding_edit;

pub(super) fn single(branch: Branch, work: BuildWork<'_>) -> Result<Vec<Branch>> {
    let _span = tracing::debug_span!("sf.compiler.leaf_output").entered();
    let mut out = work.vector(1)?;
    out.push(branch);
    work.checkpoint()?;
    Ok(out)
}

pub(super) fn scan(scan: Scan, work: BuildWork<'_>) -> Result<Vec<Branch>> {
    let mut core = work.vector(1)?;
    core.push(scan);
    single(
        Branch {
            core,
            ..Branch::empty()
        },
        work,
    )
}

pub(super) fn values(
    vars: Vec<Var>,
    rows: Vec<Vec<Option<TermDef>>>,
    work: BuildWork<'_>,
) -> Result<Vec<Branch>> {
    let _span = tracing::debug_span!("sf.compiler.values_materialization").entered();
    work.charge(1)?;
    let mut out = work.vector(rows.len())?;
    for row in rows {
        work.charge(1)?;
        let mut branch = Branch::empty();
        // Preserve raw zip semantics: surplus cells/variables are not visited.
        // A duplicate Some replaces; a duplicate None leaves the earlier value.
        for (var, cell) in vars.iter().zip(row) {
            work.charge(1)?;
            if let Some(td) = cell {
                binding_edit(work.mode, &branch.bindings, var)?;
                branch.bindings.insert(work.string(var)?, td);
                work.checkpoint()?;
            }
        }
        out.push(branch); // the exact row slot and its visit were prepaid
        work.checkpoint()?;
    }
    Ok(out)
}

pub(super) fn join_seed(work: BuildWork<'_>) -> Result<Vec<Branch>> {
    let _span = tracing::debug_span!("sf.compiler.join_seed").entered();
    work.charge(1)?;
    let mut out = work.vector(1)?;
    out.push(Branch::empty());
    work.checkpoint()?;
    Ok(out)
}
