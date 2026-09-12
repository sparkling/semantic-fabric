//! Operation-local admission for pass (3), preserving the raw fixpoint order.
use super::{fd::Fds, Branch, ColRef, LogicalSource, SqlCond, TableSchema};
use crate::build::control::{BuildVec, BuildWork};
use crate::Result;

pub(super) fn same(a: &ColRef, b: &ColRef, work: BuildWork<'_>) -> Result<bool> {
    work.charge(1)?;
    if a.alias != b.alias {
        return Ok(false);
    }
    work.charge(a.column.len().min(b.column.len()))?;
    let equal = a.column == b.column;
    work.checkpoint()?;
    Ok(equal)
}

fn column(alias: usize, name: &str, work: BuildWork<'_>) -> Result<ColRef> {
    Ok(ColRef::new(alias, work.string(name)?))
}

fn dependency<'a>(
    table: &'a TableSchema,
    name: &str,
    work: BuildWork<'_>,
) -> Result<Option<&'a sf_sql::FunctionalDep>> {
    for fd in &table.functional_dependencies {
        work.charge(1)?;
        if let [det] = fd.det.as_slice() {
            if super::control_join::equal(det, name, work)? {
                return Ok(Some(fd));
            }
        }
    }
    Ok(None)
}

pub(super) fn references_covered(
    branch: &Branch,
    drop: usize,
    fd: &sf_sql::FunctionalDep,
    work: BuildWork<'_>,
) -> Result<bool> {
    let mut allowed = BuildVec::new(Vec::new());
    for name in fd.dep.iter().chain(&fd.det) {
        work.charge(1)?;
        work.push(&mut allowed, name.as_str())?;
    }
    super::control_distinct::parent_columns(branch, drop, &allowed.values, work)
}

fn inner_candidate(
    branch: &Branch,
    schema: &super::SchemaMap<'_>,
    work: BuildWork<'_>,
) -> Result<Option<(usize, usize, usize, bool)>> {
    for (i, left) in branch.core.iter().enumerate() {
        work.charge(1)?;
        for right in &branch.core[i + 1..] {
            work.charge(1)?;
            let (Some(LogicalSource::Table(a)), Some(LogicalSource::Table(b))) =
                (left.source.logical(), right.source.logical())
            else {
                continue;
            };
            if !super::control_join::equal(a, b, work)? {
                continue;
            }
            // Preserve raw's missing-schema early return and first matching equality.
            let Some(table) = super::resolve_schema::lookup(schema, a, work)? else {
                return Ok(None);
            };
            let mut found = None;
            for (index, condition) in branch.where_conds.iter().enumerate() {
                work.charge(1)?;
                if let SqlCond::ColEq(a, b) = condition {
                    if ((a.alias == left.alias && b.alias == right.alias)
                        || (a.alias == right.alias && b.alias == left.alias))
                        && super::control_join::equal(&a.column, &b.column, work)?
                    {
                        found = Some((index, a));
                        break;
                    }
                }
            }
            let Some((index, col)) = found else {
                continue;
            };
            let Some(fd) = dependency(table, &col.column, work)? else {
                continue;
            };
            for (keep, drop) in [(left.alias, right.alias), (right.alias, left.alias)] {
                work.charge(1)?;
                if references_covered(branch, drop, fd, work)? {
                    let nullable = !super::control_distinct::non_null(table, &col.column, work)?;
                    return Ok(Some((keep, drop, index, nullable)));
                }
            }
        }
    }
    Ok(None)
}

fn left_candidate(
    branch: &Branch,
    index: usize,
    schema: &super::SchemaMap<'_>,
    work: BuildWork<'_>,
) -> Result<Option<usize>> {
    work.charge(1)?;
    let opt = &branch.opts[index];
    if !opt.extra.is_empty() {
        return Ok(None);
    }
    let (a, b) = match opt.on.as_slice() {
        [SqlCond::ColEq(a, b)] | [SqlCond::NullSafeEq(a, b)] => (a, b),
        _ => return Ok(None),
    };
    if !super::control_join::equal(&a.column, &b.column, work)? {
        return Ok(None);
    }
    let (det, keep) = if a.alias == opt.scan.alias && b.alias != opt.scan.alias {
        (a, b.alias)
    } else if b.alias == opt.scan.alias && a.alias != opt.scan.alias {
        (b, a.alias)
    } else {
        return Ok(None);
    };
    let Some(LogicalSource::Table(name)) = opt.scan.source.logical() else {
        return Ok(None);
    };
    let mut same_table = false;
    for scan in &branch.core {
        work.charge(1)?;
        if scan.alias == keep {
            if let Some(LogicalSource::Table(core_name)) = scan.source.logical() {
                same_table = super::control_join::equal(core_name, name, work)?;
            }
            break;
        }
    }
    if !same_table {
        return Ok(None);
    }
    let Some(table) = super::resolve_schema::lookup(schema, name, work)? else {
        return Ok(None);
    };
    if !super::control_distinct::non_null(table, &det.column, work)? {
        return Ok(None);
    }
    let Some(fd) = dependency(table, &det.column, work)? else {
        return Ok(None);
    };
    Ok(references_covered(branch, opt.scan.alias, fd, work)?.then_some(keep))
}

/// DISTINCT permits dependency-preserving merges, never nullable optional matches.
pub(super) fn eliminate(
    branch: &mut Branch,
    schema: &super::SchemaMap<'_>,
    context: &super::CascadeCtx<'_>,
    work: BuildWork<'_>,
) -> Result<()> {
    work.charge(1)?;
    if !context.distinct {
        return Ok(());
    }
    while let Some((keep, drop, index, nullable)) = inner_candidate(branch, schema, work)? {
        let SqlCond::ColEq(det, _) = &branch.where_conds[index] else {
            unreachable!();
        };
        let guard = if nullable {
            Some(column(keep, &det.column, work)?)
        } else {
            None
        };
        super::control_rewrite::remove(&mut branch.where_conds, index, work)?;
        super::control_rewrite::branch(branch, drop, keep, work)?;
        super::control_rewrite::retain_scan(&mut branch.core, drop, work)?;
        if let Some(guard) = guard {
            let mut found = false;
            for condition in &branch.where_conds {
                work.charge(1)?;
                if let SqlCond::IsNotNull(col) = condition {
                    if same(col, &guard, work)? {
                        found = true;
                        break;
                    }
                }
            }
            if !found {
                let mut conditions = BuildVec::new(std::mem::take(&mut branch.where_conds));
                work.push(&mut conditions, SqlCond::IsNotNull(guard))?;
                branch.where_conds = conditions.into_inner();
            }
        }
    }
    let mut index = 0;
    while index < branch.opts.len() {
        work.charge(1)?;
        if let Some(keep) = left_candidate(branch, index, schema, work)? {
            let opt = super::control_rewrite::remove(&mut branch.opts, index, work)?;
            super::control_rewrite::branch(branch, opt.scan.alias, keep, work)?;
            index = 0;
        } else {
            index += 1;
        }
    }
    work.checkpoint()
}

struct GrowingFds {
    deps: BuildVec<(ColRef, usize)>,
    cols: BuildVec<(ColRef, ColRef)>,
}

impl GrowingFds {
    fn new() -> Self {
        Self {
            deps: BuildVec::new(Vec::new()),
            cols: BuildVec::new(Vec::new()),
        }
    }

    fn add(&mut self, det: &ColRef, alias: usize, work: BuildWork<'_>) -> Result<bool> {
        for (known, target) in &self.deps.values {
            work.charge(1)?;
            if same(known, det, work)? && *target == alias {
                return Ok(false);
            }
        }
        let det = column(det.alias, &det.column, work)?;
        work.push(&mut self.deps, (det, alias))?;
        Ok(true)
    }

    fn add_col(&mut self, det: &ColRef, dep: &ColRef, work: BuildWork<'_>) -> Result<bool> {
        for (known, target) in &self.cols.values {
            work.charge(1)?;
            if same(known, det, work)? && same(target, dep, work)? {
                return Ok(false);
            }
        }
        let det = column(det.alias, &det.column, work)?;
        let dep = column(dep.alias, &dep.column, work)?;
        work.push(&mut self.cols, (det, dep))?;
        Ok(true)
    }

    fn propagate(&mut self, from: &ColRef, to: &ColRef, work: BuildWork<'_>) -> Result<bool> {
        // Snapshot targets before adding, exactly as the raw equality rule does.
        let mut targets = BuildVec::new(Vec::new());
        for (det, alias) in &self.deps.values {
            work.charge(1)?;
            if same(det, from, work)? {
                work.push(&mut targets, *alias)?;
            }
        }
        let mut changed = false;
        for alias in targets.values {
            work.charge(1)?;
            changed |= self.add(to, alias, work)?;
        }
        Ok(changed)
    }
}

/// Preserve first matching schema and key order; do not introduce new authority.
fn seed(branch: &Branch, schema: &[TableSchema], work: BuildWork<'_>) -> Result<GrowingFds> {
    let mut fds = GrowingFds::new();
    for scan in &branch.core {
        work.charge(1)?;
        let Some(LogicalSource::Table(name)) = scan.source.logical() else {
            continue;
        };
        for table in schema {
            work.charge(1)?;
            work.charge(table.name.len().min(name.len()))?;
            if table.name != *name {
                continue;
            }
            work.checkpoint()?;
            // Duplicate keys are naturally suppressed by add, as in raw inference.
            for key in std::iter::once(&table.primary_key).chain(&table.unique) {
                work.charge(1)?;
                if let [name] = key.as_slice() {
                    let det = column(scan.alias, name, work)?;
                    fds.add(&det, scan.alias, work)?;
                }
            }
            for dependency in &table.functional_dependencies {
                work.charge(1)?;
                if let [name] = dependency.det.as_slice() {
                    let det = column(scan.alias, name, work)?;
                    for name in &dependency.dep {
                        work.charge(1)?;
                        let dep = column(scan.alias, name, work)?;
                        fds.add_col(&det, &dep, work)?;
                    }
                }
            }
            break;
        }
    }
    Ok(fds)
}

pub(super) fn infer(branch: &Branch, schema: &[TableSchema], work: BuildWork<'_>) -> Result<Fds> {
    let mut fds = seed(branch, schema, work)?;
    loop {
        work.charge(1)?;
        let mut changed = false;
        let mut columns = work.vector(fds.cols.values.len())?;
        for (det, dep) in &fds.cols.values {
            work.charge(1)?;
            columns.push((
                column(det.alias, &det.column, work)?,
                column(dep.alias, &dep.column, work)?,
            ));
        }
        for condition in &branch.where_conds {
            work.charge(1)?;
            if let SqlCond::ColEq(a, b) = condition {
                changed |= fds.propagate(a, b, work)?;
                changed |= fds.propagate(b, a, work)?;
                for (det, dep) in &columns {
                    work.charge(1)?;
                    if same(det, a, work)? {
                        changed |= fds.add_col(b, dep, work)?;
                    }
                    if same(det, b, work)? {
                        changed |= fds.add_col(a, dep, work)?;
                    }
                }
            }
        }
        let mut rows = work.vector(fds.deps.values.len())?;
        for (det, alias) in &fds.deps.values {
            work.charge(1)?;
            rows.push((column(det.alias, &det.column, work)?, *alias));
        }
        for (x, m) in &rows {
            work.charge(1)?;
            for (y, n) in &rows {
                work.charge(1)?;
                if y.alias == *m {
                    changed |= fds.add(x, *n, work)?;
                }
            }
        }
        work.checkpoint()?;
        if !changed {
            return Ok(Fds {
                deps: fds.deps.into_inner(),
                col_deps: fds.cols.into_inner(),
            });
        }
    }
}

#[cfg(test)]
#[path = "control_fd_tests.rs"]
mod tests;
