//! Actual VALUES cell copies and row/column products share the rewrite meter.
use spargebra::term::GroundTerm;

use super::control::StarWork;
use crate::Result;

pub(super) fn copy_cell(
    cell: &Option<GroundTerm>,
    work: StarWork<'_>,
) -> Result<Option<GroundTerm>> {
    work.charge(1)?;
    cell.as_ref().map(|term| work.0.copied(term)).transpose()
}

pub(super) fn copy_row(
    row: &[Option<GroundTerm>],
    work: StarWork<'_>,
) -> Result<Vec<Option<GroundTerm>>> {
    let mut out = work.0.vector(row.len())?;
    for cell in row {
        out.push(copy_cell(cell, work)?);
    }
    Ok(out)
}
