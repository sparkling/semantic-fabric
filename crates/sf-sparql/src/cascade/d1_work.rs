//! Fallible D1 inventories and prepare-before-publication source rewrites.
use super::*;
use crate::build::control::{BuildVec, BuildWork};
use crate::iq::ScanSource;
use crate::Result;

fn rendered(source: &ScanSource, work: BuildWork<'_>) -> Result<bool> {
    work.charge(1)?;
    let ScanSource::Projection {
        input,
        columns,
        guards,
        distinct: true,
        native_keys,
        lexical_keys,
    } = source
    else {
        return Ok(false);
    };
    if !matches!(input.source, ScanSource::Logical(_))
        || !native_keys.is_empty()
        || !lexical_keys.is_empty()
    {
        return Ok(false);
    }
    for (_, term) in columns {
        work.charge(1)?;
        if !matches!(term, TermMap::Template(_, spec) if spec.term_type == TermType::Iri) {
            return Ok(false);
        }
    }
    for guard in guards {
        if !rendered_distinct::guard_with_work(guard, input.alias, work)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn apply(
    branch: &mut Branch,
    schema: &SchemaMap<'_>,
    dialect: sf_sql::Dialect,
    work: BuildWork<'_>,
) -> Result<()> {
    work.checkpoint()?;
    let mut uncovered = BuildVec::new(Vec::new());
    for scan in branch
        .core
        .iter()
        .chain(branch.opts.iter().map(|opt| &opt.scan))
    {
        work.charge(1)?;
        if rendered(&scan.source, work)? {
            continue;
        }
        let mut covered = false;
        if let Some(LogicalSource::Table(name)) = scan.source.logical() {
            if let Some(table) = resolve_schema::lookup(schema, name, work)? {
                covered = key_covered(table, scan.alias, &branch.bindings, work)?;
            }
        }
        if !covered {
            work.push(&mut uncovered, scan.alias)?;
        }
    }
    if !uncovered.values.is_empty() && rendered_distinct::wrap_with_work(branch, dialect, work)? {
        return Ok(());
    }
    let mut need_flag = false;
    for alias in uncovered.into_inner() {
        work.charge(1)?;
        let columns = used_columns(branch, alias, work)?;
        if columns.is_empty() || !bindings_injective(branch, alias, work)? {
            need_flag = true;
            continue;
        }
        wrap_scan(branch, alias, &columns, work)?;
    }
    if need_flag {
        work.charge(1)?;
        branch.distinct = true;
    }
    work.checkpoint()
}

fn equal(left: &str, right: &str, work: BuildWork<'_>) -> Result<bool> {
    work.charge(1)?;
    work.charge(left.len().min(right.len()))?;
    Ok(left == right)
}

pub(super) fn key_covered(
    table: &TableSchema,
    alias: usize,
    bindings: &std::collections::BTreeMap<String, TermDef>,
    work: BuildWork<'_>,
) -> Result<bool> {
    work.checkpoint()?;
    for key in std::iter::once(&table.primary_key).chain(&table.unique) {
        work.charge(1)?;
        if key.is_empty() {
            continue;
        }
        let mut nonnull = true;
        for name in key {
            work.charge(1)?;
            let mut proven = false;
            for primary in &table.primary_key {
                if equal(primary, name, work)? {
                    proven = true;
                    break;
                }
            }
            if !proven {
                for column in &table.columns {
                    if equal(&column.name, name, work)? {
                        proven = column.not_null;
                        break;
                    }
                }
            }
            if !proven {
                nonnull = false;
                break;
            }
        }
        if !nonnull {
            continue;
        }
        let mut covered = true;
        for name in key {
            work.charge(1)?;
            let mut found = false;
            for def in bindings.values() {
                work.charge(1)?;
                if !resolve_work::injective(def, work)? {
                    continue;
                }
                let (TermDef::Derived {
                    term_map,
                    alias: owner,
                }
                | TermDef::R2rmlBlank {
                    term_map,
                    alias: owner,
                    ..
                }) = def
                else {
                    continue;
                };
                if *owner != alias {
                    continue;
                }
                match term_map {
                    TermMap::Column(column, _) => found = equal(column, name, work)?,
                    TermMap::Template(template, _) => {
                        for segment in template.segments() {
                            work.charge(1)?;
                            if let Segment::Column(column) = segment {
                                if equal(column, name, work)? {
                                    found = true;
                                    break;
                                }
                            }
                        }
                    }
                    TermMap::Constant(_) => (),
                }
                if found {
                    break;
                }
            }
            if !found {
                covered = false;
                break;
            }
        }
        if covered {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn used_columns(
    branch: &Branch,
    alias: usize,
    work: BuildWork<'_>,
) -> Result<Vec<Box<str>>> {
    work.checkpoint()?;
    let mut columns = BuildVec::new(Vec::new());
    let mut push = |owner: usize, name: &str| -> Result<()> {
        work.charge(1)?;
        if owner == alias && !work.contains(&columns.values, name)? {
            work.push(&mut columns, work.variable(name)?)?;
        }
        Ok(())
    };
    for def in branch.bindings.values() {
        work.charge(1)?;
        for (owner, name) in resolve_work::columns(def, work)? {
            push(owner, name)?;
        }
    }
    let mut condition = |cond: &SqlCond| distinct_scan::condition_columns(cond, work, &mut push);
    for cond in &branch.where_conds {
        condition(cond)?;
    }
    for opt in &branch.opts {
        work.charge(1)?;
        for cond in opt.on.iter().chain(&opt.extra) {
            condition(cond)?;
        }
    }
    for join in &branch.subplan_joins {
        work.charge(1)?;
        for cond in &join.on {
            condition(cond)?;
        }
    }
    Ok(columns.into_inner())
}

fn logical_projection(source: &ScanSource, work: BuildWork<'_>) -> Result<bool> {
    let work = work.enter()?;
    match source {
        ScanSource::Logical(_) => Ok(true),
        ScanSource::Projection { input, .. } => logical_projection(&input.source, work),
        ScanSource::Path { .. } | ScanSource::RefAtom { .. } => Ok(false),
    }
}

pub(super) fn bindings_injective(
    branch: &Branch,
    alias: usize,
    work: BuildWork<'_>,
) -> Result<bool> {
    let keys = distinct_scan::keys_with_work(branch, alias, true, work)?;
    let term_safe = |map: &TermMap, owner: usize| -> Result<bool> {
        work.charge(1)?;
        if owner != alias {
            return Ok(true);
        }
        if let TermMap::Template(template, _) = map {
            for segment in template.segments() {
                work.charge(1)?;
                if let Segment::Literal(text) = segment {
                    work.charge(text.len())?;
                }
            }
        }
        if term_map_is_injective(map) {
            work.checkpoint()?;
            return Ok(true);
        }
        if let TermMap::Column(column, spec) = map {
            if spec.term_type == TermType::Iri {
                for key in &keys {
                    work.charge(1)?;
                    work.charge(key.column.len().min(column.len()))?;
                    if key.column == *column {
                        if let crate::iq::scan::LexicalMode::Iri { base } = &key.mode {
                            if let (Some(left), Some(right)) = (base, &spec.base) {
                                work.charge(left.len().min(right.len()))?;
                            }
                            if base == &spec.base {
                                return Ok(true);
                            }
                        }
                    }
                }
            }
        }
        Ok(false)
    };
    for def in branch.bindings.values() {
        work.charge(1)?;
        let safe = match def {
            TermDef::Derived {
                term_map,
                alias: owner,
            } => term_safe(term_map, *owner)?,
            TermDef::R2rmlBlank {
                term_map,
                alias: owner,
                graph,
            } => {
                term_safe(term_map, *owner)?
                    && match graph {
                        R2rmlGraphScope::Default => true,
                        R2rmlGraphScope::Mapped {
                            term_map,
                            alias: owner,
                        } => term_safe(term_map, *owner)?,
                    }
            }
            _ => {
                let mut reads_alias = false;
                for (owner, _) in resolve_work::columns(def, work)? {
                    work.charge(1)?;
                    if owner == alias {
                        reads_alias = true;
                        break;
                    }
                }
                !reads_alias || resolve_work::injective(def, work)?
            }
        };
        if !safe {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn wrap_scan(
    branch: &mut Branch,
    alias: usize,
    cols: &[Box<str>],
    work: BuildWork<'_>,
) -> Result<()> {
    let native_keys = distinct_scan::native_keys_with_work(branch, alias, work)?;
    let lexical_keys = distinct_scan::keys_with_work(branch, alias, true, work)?;
    let mut found = None;
    for (index, scan) in branch
        .core
        .iter()
        .chain(branch.opts.iter().map(|opt| &opt.scan))
        .enumerate()
    {
        work.charge(1)?;
        if scan.alias == alias {
            found = Some((index, scan));
            break;
        }
    }
    let (index, scan) = found.expect("alias belongs to this branch");
    if !logical_projection(&scan.source, work)? {
        return Ok(());
    }
    let mut columns = work.vector(cols.len())?;
    for column in cols {
        work.charge(1)?;
        columns.push((
            work.variable(column)?,
            TermMap::Column(
                work.variable(column)?,
                sf_core::ir::TermSpec::plain_literal(),
            ),
        ));
    }
    work.charge(std::mem::size_of::<Scan>())?;
    work.checkpoint()?;
    // No fallible operation after publication starts. Index preserves the
    // original first-match core-then-optional alias lookup, including duplicates.
    let scan = if index < branch.core.len() {
        &mut branch.core[index]
    } else {
        &mut branch.opts[index - branch.core.len()].scan
    };
    let source = std::mem::replace(&mut scan.source, LogicalSource::Table(String::new()).into());
    scan.source = ScanSource::Projection {
        input: Box::new(Scan { alias, source }),
        columns,
        guards: Vec::new(),
        distinct: true,
        native_keys,
        lexical_keys,
    };
    Ok(())
}

#[cfg(test)]
#[path = "d1_work_tests.rs"]
mod tests;
