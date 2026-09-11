//! Paid counterpart of the existing FILTER decoder-source check. Preserve its
//! traversal, short-circuit order and error; bound recursion and every fanout
//! internally instead of guessing a linear allowance for nested derived plans.
use super::filter_projection::{projection, Column};
use super::{IriOperand, IriPart};
use crate::build::control::BuildWork;
use crate::iq::{Branch, ScanSource, SqlCond};
use crate::{CompilerWorkMode, Error, Result};
use sf_core::ir::{Segment, TermMap};
use sf_sql::Dialect;

pub(crate) fn validate_filter_source_with_work_mode(
    condition: &SqlCond,
    branch: &Branch,
    dialect: Dialect,
    mode: CompilerWorkMode<'_>,
) -> Result<()> {
    if matches!(mode, CompilerWorkMode::Uncontrolled) {
        return super::validate_filter_source(condition, branch, dialect)
            .map_err(Error::Unsupported);
    }
    validate(condition, branch, dialect, BuildWork::new(mode))
}

fn validate(cond: &SqlCond, branch: &Branch, dialect: Dialect, work: BuildWork<'_>) -> Result<()> {
    let work = work.enter()?;
    match cond {
        SqlCond::IriCmp(cmp) => {
            for operand in [&cmp.left, &cmp.right] {
                work.charge(1)?;
                let found = match operand {
                    IriOperand::Column { column, .. } => {
                        path_column(Column::from(column), branch, dialect, work)?
                    }
                    IriOperand::Template { parts, .. } => {
                        let mut found = false;
                        for part in parts {
                            work.charge(1)?;
                            if let IriPart::Column(column) = part {
                                if path_column(Column::from(column), branch, dialect, work)? {
                                    found = true;
                                    break;
                                }
                            }
                        }
                        found
                    }
                    IriOperand::Constant(_) => false,
                };
                if found {
                    return Err(Error::Unsupported(work.string(
                        "FILTER on a path endpoint requires an unimplemented decoder identity proof",
                    )?));
                }
            }
        }
        SqlCond::Not(inner) => validate(inner, branch, dialect, work)?,
        SqlCond::And(parts) | SqlCond::Or(parts) => {
            for part in parts {
                validate(part, branch, dialect, work)?;
            }
        }
        _ => {}
    }
    work.checkpoint()
}

fn path_column(
    column: Column<'_>,
    branch: &Branch,
    dialect: Dialect,
    work: BuildWork<'_>,
) -> Result<bool> {
    let work = work.enter()?;
    if let Some(path) = &branch.path {
        work.charge(1)?;
        if path.alias == column.alias {
            return Ok(true);
        }
    }
    for scan in branch
        .core
        .iter()
        .chain(branch.opts.iter().map(|opt| &opt.scan))
    {
        work.charge(1)?;
        if scan.alias == column.alias && path_scan_column(column.name, &scan.source, dialect, work)?
        {
            return Ok(true);
        }
    }
    for join in &branch.subplan_joins {
        work.charge(1)?;
        if join.alias != column.alias {
            continue;
        }
        for inner in &join.plan.branches {
            work.charge(1)?;
            let distinct = if join.plan.branches.len() == 1 {
                join.plan.distinct
            } else {
                inner.distinct
            };
            for (index, source) in projection(inner, distinct, dialect, work)?
                .iter()
                .enumerate()
            {
                if position(column.name, index, work)? {
                    if let Some(source) = source {
                        if path_column(*source, inner, dialect, work)? {
                            return Ok(true);
                        }
                    }
                }
            }
        }
    }
    Ok(false)
}

fn path_scan_column(
    name: &str,
    source: &ScanSource,
    dialect: Dialect,
    work: BuildWork<'_>,
) -> Result<bool> {
    let work = work.enter()?;
    match source {
        ScanSource::Logical(_) => Ok(false),
        ScanSource::Path { .. } => Ok(true),
        ScanSource::RefAtom { input, columns } => {
            for (index, column) in columns.iter().enumerate() {
                if position(name, index, work)?
                    && path_column(Column::from(column), input, dialect, work)?
                {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        ScanSource::Projection { input, columns, .. } => {
            for (output, map) in columns {
                work.charge(1)?;
                work.charge(output.len().min(name.len()))?;
                if output.as_ref() != name {
                    continue;
                }
                return match map {
                    TermMap::Column(column, _) => {
                        path_scan_column(column, &input.source, dialect, work)
                    }
                    TermMap::Template(template, _) => {
                        for part in template.segments() {
                            work.charge(1)?;
                            if let Segment::Column(column) = part {
                                if path_scan_column(column, &input.source, dialect, work)? {
                                    return Ok(true);
                                }
                            }
                        }
                        Ok(false)
                    }
                    TermMap::Constant(_) => Ok(false),
                };
            }
            Ok(false)
        }
    }
}

/// Exact `c{index}` identity without allocating a formatted name per candidate.
/// Reject leading zeros, signs and non-ASCII digits just as raw string equality.
pub(super) fn position(name: &str, index: usize, work: BuildWork<'_>) -> Result<bool> {
    work.charge(1)?;
    work.charge(name.len())?;
    let Some(digits) = name.strip_prefix('c') else {
        return Ok(false);
    };
    if digits.is_empty()
        || (digits.len() > 1 && digits.starts_with('0'))
        || !digits.bytes().all(|b| b.is_ascii_digit())
    {
        return Ok(false);
    }
    work.charge(digits.len())?;
    Ok(digits.parse::<usize>().is_ok_and(|n| n == index))
}
