//! Paid discovery and mutation for inner single/composite/nullable self joins.
use super::{Branch, ColRef, LogicalSource, Scan, SchemaMap, SqlCond};
use crate::build::control::{BuildVec, BuildWork};
use crate::Result;

pub(super) fn equal(a: &str, b: &str, work: BuildWork<'_>) -> Result<bool> {
    work.charge(1)?;
    work.charge(a.len().min(b.len()))?;
    Ok(a == b)
}

pub(super) fn table<'a>(
    scans: &'a [Scan],
    alias: usize,
    work: BuildWork<'_>,
) -> Result<Option<&'a str>> {
    for scan in scans {
        work.charge(1)?;
        if scan.alias == alias {
            return Ok(match scan.source.logical() {
                Some(LogicalSource::Table(name)) => Some(name.as_str()),
                _ => None,
            });
        }
    }
    Ok(None)
}

pub(super) fn unique(table: &super::TableSchema, name: &str, work: BuildWork<'_>) -> Result<bool> {
    for key in std::iter::once(&table.primary_key).chain(&table.unique) {
        work.charge(1)?;
        if let [column] = key.as_slice() {
            if equal(column, name, work)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub(super) fn schema_table<'a>(
    schema: &'a [super::TableSchema],
    name: &str,
    work: BuildWork<'_>,
) -> Result<Option<&'a super::TableSchema>> {
    for table in schema {
        work.charge(1)?;
        if equal(&table.name, name, work)? {
            return Ok(Some(table));
        }
    }
    Ok(None)
}

/// An FK proves the key match only. Extra predicates need their own proof.
fn guaranteed(
    branch: &Branch,
    index: usize,
    schema: &[super::TableSchema],
    work: BuildWork<'_>,
) -> Result<bool> {
    work.charge(1)?;
    let opt = &branch.opts[index];
    let Some(LogicalSource::Table(name)) = opt.scan.source.logical() else {
        return Ok(false);
    };
    let (child, parent) = match opt.on.as_slice() {
        [SqlCond::ColEq(a, b)] | [SqlCond::NullSafeEq(a, b)] => {
            if a.alias != opt.scan.alias && b.alias == opt.scan.alias {
                (a, b)
            } else if b.alias != opt.scan.alias && a.alias == opt.scan.alias {
                (b, a)
            } else {
                return Ok(false);
            }
        }
        _ => return Ok(false),
    };
    let Some(parent_schema) = schema_table(schema, name, work)? else {
        return Ok(false);
    };
    if !unique(parent_schema, &parent.column, work)? {
        return Ok(false);
    }
    for condition in &opt.extra {
        work.charge(1)?;
        match condition {
            SqlCond::IsNotNull(col) if col.alias == opt.scan.alias => {
                if !super::control_distinct::non_null(parent_schema, &col.column, work)? {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        }
    }
    let Some(child_name) = table(&branch.core, child.alias, work)? else {
        return Ok(false);
    };
    let Some(child_schema) = schema_table(schema, child_name, work)? else {
        return Ok(false);
    };
    for fk in &child_schema.foreign_keys {
        work.charge(1)?;
        if let ([column], [parent_column]) = (fk.columns.as_slice(), fk.parent_columns.as_slice()) {
            if equal(&fk.parent_table, name, work)?
                && equal(column, &child.column, work)?
                && equal(parent_column, &parent.column, work)?
            {
                return super::control_distinct::non_null(child_schema, &child.column, work);
            }
        }
    }
    Ok(false)
}

pub(super) fn downgrade(
    branch: &mut Branch,
    schema: &[super::TableSchema],
    work: BuildWork<'_>,
) -> Result<()> {
    let mut index = 0;
    while index < branch.opts.len() {
        work.charge(1)?;
        if !guaranteed(branch, index, schema, work)? {
            index += 1;
            continue;
        }
        let opt = super::control_rewrite::remove(&mut branch.opts, index, work)?;
        let mut conditions = BuildVec::new(std::mem::take(&mut branch.where_conds));
        for condition in opt.on.into_iter().chain(opt.extra) {
            work.charge(1)?;
            let condition = match condition {
                SqlCond::NullSafeEq(a, b) => SqlCond::ColEq(a, b),
                other => other,
            };
            work.push(&mut conditions, condition)?;
        }
        branch.where_conds = conditions.into_inner();
        let mut core = BuildVec::new(std::mem::take(&mut branch.core));
        work.push(&mut core, opt.scan)?;
        branch.core = core.into_inner();
    }
    work.checkpoint()
}

fn single(
    scans: &[Scan],
    conds: &[SqlCond],
    schema: &SchemaMap<'_>,
    nullable: bool,
    work: BuildWork<'_>,
) -> Result<Option<(usize, usize, usize)>> {
    for (index, condition) in conds.iter().enumerate() {
        work.charge(1)?;
        let SqlCond::ColEq(a, b) = condition else {
            continue;
        };
        if a.alias == b.alias || !equal(&a.column, &b.column, work)? {
            continue;
        }
        let Some(left) = table(scans, a.alias, work)? else {
            continue;
        };
        let Some(right) = table(scans, b.alias, work)? else {
            continue;
        };
        if !equal(left, right, work)? {
            continue;
        }
        if let Some(table) = super::resolve_schema::lookup(schema, left, work)? {
            if unique(table, &a.column, work)?
                && super::control_distinct::non_null(table, &a.column, work)? != nullable
            {
                return Ok(Some((a.alias.min(b.alias), a.alias.max(b.alias), index)));
            }
        }
    }
    Ok(None)
}

fn composite(
    scans: &[Scan],
    conditions: &[SqlCond],
    schema: &SchemaMap<'_>,
    work: BuildWork<'_>,
) -> Result<Option<(usize, usize, Vec<usize>)>> {
    for (i, left) in scans.iter().enumerate() {
        work.charge(1)?;
        let Some(LogicalSource::Table(name)) = left.source.logical() else {
            continue;
        };
        let Some(table) = super::resolve_schema::lookup(schema, name, work)? else {
            continue;
        };
        if table.primary_key.len() < 2 {
            continue;
        }
        for right in &scans[i + 1..] {
            work.charge(1)?;
            let Some(LogicalSource::Table(other)) = right.source.logical() else {
                continue;
            };
            if !equal(name, other, work)? {
                continue;
            }
            let mut covered = BuildVec::new(Vec::<&str>::new());
            let mut indices = BuildVec::new(Vec::new());
            for (index, condition) in conditions.iter().enumerate() {
                work.charge(1)?;
                let SqlCond::ColEq(a, b) = condition else {
                    continue;
                };
                if !equal(&a.column, &b.column, work)? {
                    continue;
                }
                if !((a.alias == left.alias && b.alias == right.alias)
                    || (a.alias == right.alias && b.alias == left.alias))
                {
                    continue;
                }
                let mut key = false;
                for column in &table.primary_key {
                    if equal(column, &a.column, work)? {
                        key = true;
                        break;
                    }
                }
                if !key {
                    continue;
                }
                let mut duplicate = false;
                for column in &covered.values {
                    if equal(column, &a.column, work)? {
                        duplicate = true;
                        break;
                    }
                }
                if !duplicate {
                    work.push(&mut covered, &a.column)?;
                    work.push(&mut indices, index)?;
                }
            }
            let mut all = true;
            for key in &table.primary_key {
                let mut found = false;
                for column in &covered.values {
                    if equal(column, key, work)? {
                        found = true;
                        break;
                    }
                }
                if !found {
                    all = false;
                    break;
                }
            }
            if all {
                return Ok(Some((
                    left.alias.min(right.alias),
                    left.alias.max(right.alias),
                    indices.into_inner(),
                )));
            }
        }
    }
    Ok(None)
}

pub(super) fn inner(
    branch: &mut Branch,
    schema: &SchemaMap<'_>,
    work: BuildWork<'_>,
) -> Result<()> {
    loop {
        work.charge(1)?;
        let Some((keep, drop, index)) =
            single(&branch.core, &branch.where_conds, schema, false, work)?
        else {
            break;
        };
        super::control_rewrite::remove(&mut branch.where_conds, index, work)?;
        super::control_rewrite::branch(branch, drop, keep, work)?;
        super::control_rewrite::retain_scan(&mut branch.core, drop, work)?;
    }
    loop {
        work.charge(1)?;
        let Some((keep, drop, indices)) =
            composite(&branch.core, &branch.where_conds, schema, work)?
        else {
            break;
        };
        // Discovery appends distinct condition indices in ascending order.
        for index in indices.into_iter().rev() {
            super::control_rewrite::remove(&mut branch.where_conds, index, work)?;
        }
        super::control_rewrite::branch(branch, drop, keep, work)?;
        super::control_rewrite::retain_scan(&mut branch.core, drop, work)?;
    }
    work.checkpoint()
}

pub(super) fn nullable(
    branch: &mut Branch,
    schema: &SchemaMap<'_>,
    work: BuildWork<'_>,
) -> Result<()> {
    loop {
        work.charge(1)?;
        let Some((keep, drop, index)) =
            single(&branch.core, &branch.where_conds, schema, true, work)?
        else {
            break;
        };
        let SqlCond::ColEq(column, _) = &branch.where_conds[index] else {
            unreachable!()
        };
        let guard = ColRef::new(keep, work.string(&column.column)?);
        super::control_rewrite::remove(&mut branch.where_conds, index, work)?;
        super::control_rewrite::branch(branch, drop, keep, work)?;
        super::control_rewrite::retain_scan(&mut branch.core, drop, work)?;
        let mut present = false;
        for condition in &branch.where_conds {
            work.charge(1)?;
            if let SqlCond::IsNotNull(column) = condition {
                if super::control_fd::same(column, &guard, work)? {
                    present = true;
                    break;
                }
            }
        }
        if !present {
            let mut conditions = BuildVec::new(std::mem::take(&mut branch.where_conds));
            work.push(&mut conditions, SqlCond::IsNotNull(guard))?;
            branch.where_conds = conditions.into_inner();
        }
    }
    work.checkpoint()
}

pub(super) fn subqueries(
    conditions: &mut [SqlCond],
    schema: &SchemaMap<'_>,
    work: BuildWork<'_>,
) -> Result<()> {
    let work = work.enter()?;
    for condition in conditions {
        work.charge(1)?;
        match condition {
            SqlCond::Exists { scans, conds } | SqlCond::NotExists { scans, conds } => {
                loop {
                    work.charge(1)?;
                    let Some((keep, drop, index)) = single(scans, conds, schema, false, work)?
                    else {
                        break;
                    };
                    super::control_rewrite::remove(conds, index, work)?;
                    for condition in conds.iter_mut() {
                        work.charge(1)?;
                        super::control_rewrite::condition(condition, drop, keep, work)?;
                    }
                    super::control_rewrite::retain_scan(scans, drop, work)?;
                }
                loop {
                    work.charge(1)?;
                    let Some((keep, drop, indices)) = composite(scans, conds, schema, work)? else {
                        break;
                    };
                    for index in indices.into_iter().rev() {
                        super::control_rewrite::remove(conds, index, work)?;
                    }
                    for condition in conds.iter_mut() {
                        work.charge(1)?;
                        super::control_rewrite::condition(condition, drop, keep, work)?;
                    }
                    super::control_rewrite::retain_scan(scans, drop, work)?;
                }
                subqueries(conds, schema, work)?;
            }
            SqlCond::Not(inner) => subqueries(std::slice::from_mut(inner.as_mut()), schema, work)?,
            SqlCond::And(children) | SqlCond::Or(children) => subqueries(children, schema, work)?,
            _ => (),
        }
    }
    work.checkpoint()
}

pub(super) fn left_key<'s>(
    branch: &Branch,
    index: usize,
    schema: &SchemaMap<'s>,
    work: BuildWork<'_>,
) -> Result<Option<(usize, &'s super::TableSchema)>> {
    work.charge(1)?;
    let optional = &branch.opts[index];
    let Some(LogicalSource::Table(name)) = optional.scan.source.logical() else {
        return Ok(None);
    };
    let [condition] = optional.on.as_slice() else {
        return Ok(None);
    };
    let (SqlCond::NullSafeEq(a, b) | SqlCond::ColEq(a, b)) = condition else {
        return Ok(None);
    };
    if !equal(&a.column, &b.column, work)? {
        return Ok(None);
    }
    let keep = if a.alias == optional.scan.alias && b.alias != optional.scan.alias {
        b
    } else if b.alias == optional.scan.alias && a.alias != optional.scan.alias {
        a
    } else {
        return Ok(None);
    };
    let Some(core) = table(&branch.core, keep.alias, work)? else {
        return Ok(None);
    };
    if !equal(core, name, work)? {
        return Ok(None);
    }
    let Some(table) = super::resolve_schema::lookup(schema, name, work)? else {
        return Ok(None);
    };
    if !unique(table, &keep.column, work)?
        || !super::control_distinct::non_null(table, &keep.column, work)?
    {
        return Ok(None);
    }
    Ok(Some((keep.alias, table)))
}

pub(super) fn left(branch: &mut Branch, schema: &SchemaMap<'_>, work: BuildWork<'_>) -> Result<()> {
    loop {
        work.charge(1)?;
        let mut found = None;
        for (index, optional) in branch.opts.iter().enumerate() {
            work.charge(1)?;
            if let Some((keep, table)) = left_key(branch, index, schema, work)? {
                if super::optional_prune::self_left_extra_with_work(optional, table, work)? {
                    found = Some((index, optional.scan.alias, keep));
                    break;
                }
            }
        }
        let Some((index, drop, keep)) = found else {
            break;
        };
        let optional = super::control_rewrite::remove(&mut branch.opts, index, work)?;
        let mut conditions = BuildVec::new(std::mem::take(&mut branch.where_conds));
        for condition in optional.extra {
            work.charge(1)?;
            if let SqlCond::DecodedIsNotNull(column) = condition {
                let copied = ColRef::new(column.alias, work.string(&column.column)?);
                let mut arms = work.vector(2)?;
                arms.push(SqlCond::IsNull(copied));
                arms.push(SqlCond::DecodedIsNotNull(column));
                work.push(&mut conditions, SqlCond::Or(arms))?;
            }
        }
        branch.where_conds = conditions.into_inner();
        super::control_rewrite::branch(branch, drop, keep, work)?;
    }
    super::control_conditions::left_contradictions(branch, schema, work)
}
