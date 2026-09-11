//! RESOLVE schema authority: unique names only, with interruptible construction.
use super::*;
use crate::build::control::BuildWork;
use crate::Result;

pub(crate) fn build<'a>(schema: &'a [TableSchema], work: BuildWork<'_>) -> Result<SchemaMap<'a>> {
    let mut map: SchemaMap<'_> = work.vector(schema.len())?;
    for table in schema {
        work.charge(1)?;
        let mut lo = 0;
        let mut hi = map.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            work.charge(1)?;
            work.charge(map[mid].0.len())?;
            work.charge(table.name.len())?;
            match map[mid].0.cmp(table.name.as_str()) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => {
                    return Err(crate::Error::Unsupported(
                        work.string("ambiguous compiler schema: duplicate table name")?,
                    ))
                }
            }
        }
        let moved = map.len() - lo;
        work.charge(moved)?;
        if let crate::CompilerWorkMode::Metered(cx) = work.mode {
            cx.reserve_checked_product(&[moved, std::mem::size_of::<(&str, &TableSchema)>()])?;
        }
        work.checkpoint()?;
        map.insert(lo, (&table.name, table));
    }
    work.checkpoint()?;
    Ok(map)
}

#[cfg(test)]
fn force_distinct_with_work(
    branches: &mut [Branch],
    schema: &[TableSchema],
    dialect: sf_sql::Dialect,
    work: BuildWork<'_>,
) -> Result<()> {
    let schema = build(schema, work)?;
    force_distinct_with_schema(branches, &schema, dialect, work)
}

pub(crate) fn force_distinct_with_schema(
    branches: &mut [Branch],
    schema: &SchemaMap<'_>,
    dialect: sf_sql::Dialect,
    work: BuildWork<'_>,
) -> Result<()> {
    for branch in branches {
        work.charge(1)?;
        if branch.distinct || branch.path.is_some() || branch.agg.is_some() {
            continue;
        }
        d1_work::apply(branch, schema, dialect, work)?;
    }
    Ok(())
}

pub(super) fn lookup<'a>(
    map: &SchemaMap<'a>,
    name: &str,
    work: BuildWork<'_>,
) -> Result<Option<&'a TableSchema>> {
    let mut width = 0;
    for (candidate, _) in map {
        work.charge(1)?;
        width = width.max(candidate.len().min(name.len()));
    }
    let mut remaining = map.len();
    while remaining > 1 {
        work.charge(1)?;
        work.charge(width)?;
        remaining -= remaining / 2;
    }
    if remaining != 0 {
        work.charge(1)?;
        work.charge(width)?;
    }
    Ok(schema_map_get(map, name))
}

#[cfg(test)]
#[path = "resolve_schema_tests.rs"]
mod tests;
