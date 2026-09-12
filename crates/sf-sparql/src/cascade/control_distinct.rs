//! DISTINCT optional pruning keeps all value, correlation and decoder consumers.
use super::{Branch, CascadeCtx, SqlCond};
use crate::build::control::{BuildVec, BuildWork};
use crate::Result;

fn equal(a: &str, b: &str, work: BuildWork<'_>) -> Result<bool> {
    work.charge(1)?;
    work.charge(a.len().min(b.len()))?;
    Ok(a == b)
}

fn projected(variable: &str, project: Option<&[String]>, work: BuildWork<'_>) -> Result<bool> {
    let Some(project) = project else {
        return Ok(true);
    };
    for name in project {
        if equal(variable, name, work)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn unique(definition: &super::TermDef, work: BuildWork<'_>) -> Result<bool> {
    work.charge(1)?;
    if matches!(definition, super::TermDef::Derived { term_map: super::TermMap::Column(_, spec), .. }
        if spec.term_type == super::TermType::Literal && spec.language.is_none())
    {
        return Ok(false);
    }
    super::binding_injective_with_work(definition, work)
}

pub(super) fn non_null(
    table: &super::TableSchema,
    name: &str,
    work: BuildWork<'_>,
) -> Result<bool> {
    for key in &table.primary_key {
        if equal(key, name, work)? {
            return Ok(true);
        }
    }
    for column in &table.columns {
        if equal(&column.name, name, work)? {
            return Ok(column.not_null);
        }
    }
    Ok(false)
}

fn reads(
    definition: &super::TermDef,
    alias: usize,
    name: &str,
    work: BuildWork<'_>,
) -> Result<bool> {
    for (owner, column) in super::term_columns_with_work(definition, work)? {
        work.charge(1)?;
        if owner == alias && equal(column, name, work)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn redundant(
    branch: &Branch,
    schema: &super::SchemaMap<'_>,
    project: Option<&[String]>,
    work: BuildWork<'_>,
) -> Result<bool> {
    if branch.core.len() == 1 {
        let scan = &branch.core[0];
        let Some(super::LogicalSource::Table(name)) = scan.source.logical() else {
            return Ok(false);
        };
        let Some(table) = super::resolve_schema::lookup(schema, name, work)? else {
            return Ok(false);
        };
        let mut keys = BuildVec::new(Vec::new());
        for key in std::iter::once(&table.primary_key).chain(&table.unique) {
            work.charge(1)?;
            if let [name] = key.as_slice() {
                if non_null(table, name, work)? {
                    work.push(&mut keys, name.as_str())?;
                }
            }
        }
        for (variable, definition) in &branch.bindings {
            work.charge(1)?;
            if projected(variable, project, work)? && unique(definition, work)? {
                for key in &keys.values {
                    work.charge(1)?;
                    if reads(definition, scan.alias, key, work)? {
                        return Ok(true);
                    }
                }
            }
        }
        if table.primary_key.len() > 1 {
            for (variable, definition) in &branch.bindings {
                work.charge(1)?;
                if !projected(variable, project, work)? || !unique(definition, work)? {
                    continue;
                }
                let super::TermDef::Derived {
                    term_map: super::TermMap::Template(template, _),
                    alias,
                } = definition
                else {
                    continue;
                };
                if *alias != scan.alias {
                    continue;
                }
                let mut covered = true;
                for key in &table.primary_key {
                    work.charge(1)?;
                    let mut found = false;
                    for segment in template.segments() {
                        work.charge(1)?;
                        if let super::Segment::Column(column) = segment {
                            if equal(column, key, work)? {
                                found = true;
                                break;
                            }
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
        }
        return Ok(false);
    }
    for scan in &branch.core {
        work.charge(1)?;
        let Some(super::LogicalSource::Table(name)) = scan.source.logical() else {
            return Ok(false);
        };
        let Some(table) = super::resolve_schema::lookup(schema, name, work)? else {
            return Ok(false);
        };
        if table.primary_key.is_empty() {
            return Ok(false);
        }
        for key in &table.primary_key {
            work.charge(1)?;
            let mut covered = false;
            for (variable, definition) in &branch.bindings {
                work.charge(1)?;
                if projected(variable, project, work)?
                    && unique(definition, work)?
                    && reads(definition, scan.alias, key, work)?
                {
                    covered = true;
                    break;
                }
            }
            if !covered {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

pub(super) fn remove(
    branch: &mut Branch,
    schema: &super::SchemaMap<'_>,
    project: Option<&[String]>,
    work: BuildWork<'_>,
) -> Result<()> {
    work.charge(1)?;
    if !branch.distinct || branch.core.is_empty() || !branch.opts.is_empty() {
        return Ok(());
    }
    let redundant = redundant(branch, schema, project, work)?;
    work.checkpoint()?;
    if redundant {
        branch.distinct = false;
    }
    Ok(())
}

fn contains(values: &[usize], alias: usize, work: BuildWork<'_>) -> Result<bool> {
    for known in values {
        work.charge(1)?;
        if *known == alias {
            return Ok(true);
        }
    }
    Ok(false)
}

fn require(values: &mut BuildVec<usize>, alias: usize, work: BuildWork<'_>) -> Result<()> {
    if !contains(&values.values, alias, work)? {
        work.push(values, alias)?;
    }
    Ok(())
}

fn aliases(
    condition: &SqlCond,
    work: BuildWork<'_>,
    visit: &mut impl FnMut(usize) -> Result<()>,
) -> Result<()> {
    let work = work.enter()?;
    match condition {
        SqlCond::Not(inner) => aliases(inner, work, visit),
        SqlCond::And(children)
        | SqlCond::Or(children)
        | SqlCond::Exists {
            conds: children, ..
        }
        | SqlCond::NotExists {
            conds: children, ..
        }
        | SqlCond::PathExists {
            conds: children, ..
        } => {
            for child in children {
                work.charge(1)?;
                aliases(child, work, visit)?;
            }
            Ok(())
        }
        _ => super::condition_columns_with_work(condition, work, &mut |alias, _| visit(alias)),
    }
}

fn decoder(condition: &SqlCond, alias: Option<usize>, work: BuildWork<'_>) -> Result<bool> {
    let work = work.enter()?;
    match condition {
        SqlCond::DecodedIsNotNull(col) => Ok(alias.is_none_or(|alias| alias == col.alias)),
        SqlCond::Not(inner) => decoder(inner, alias, work),
        SqlCond::And(children)
        | SqlCond::Or(children)
        | SqlCond::Exists {
            conds: children, ..
        }
        | SqlCond::NotExists {
            conds: children, ..
        }
        | SqlCond::PathExists {
            conds: children, ..
        } => {
            for child in children {
                work.charge(1)?;
                if decoder(child, alias, work)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

pub(super) fn prune_optional(
    branch: &mut Branch,
    context: &CascadeCtx<'_>,
    work: BuildWork<'_>,
) -> Result<()> {
    work.charge(1)?;
    if !context.distinct
        || !branch.order.is_empty()
        || branch.agg.is_some()
        || branch.path.is_some()
        || !branch.subplan_joins.is_empty()
    {
        return Ok(());
    }
    let Some(project) = context.project else {
        return Ok(());
    };
    let mut required = BuildVec::new(Vec::new());
    for (variable, definition) in &branch.bindings {
        work.charge(1)?;
        let mut projected = false;
        for name in project {
            work.charge(1)?;
            work.charge(name.len().min(variable.len()))?;
            if name == variable {
                projected = true;
                break;
            }
        }
        if projected {
            for (alias, _) in super::term_columns_with_work(definition, work)? {
                work.charge(1)?;
                require(&mut required, alias, work)?;
            }
        }
    }
    for condition in &branch.where_conds {
        work.charge(1)?;
        aliases(condition, work, &mut |alias| {
            require(&mut required, alias, work)
        })?;
    }
    for optional in &branch.opts {
        work.charge(1)?;
        for condition in optional.on.iter().chain(&optional.extra) {
            work.charge(1)?;
            if decoder(condition, None, work)? {
                require(&mut required, optional.scan.alias, work)?;
                break;
            }
        }
        for condition in optional.on.iter().chain(&optional.extra) {
            work.charge(1)?;
            aliases(condition, work, &mut |alias| {
                work.charge(1)?;
                if alias != optional.scan.alias {
                    require(&mut required, alias, work)?;
                }
                Ok(())
            })?;
        }
    }
    let mut keep = work.vector(branch.opts.len())?;
    for optional in &branch.opts {
        work.charge(1)?;
        keep.push(contains(&required.values, optional.scan.alias, work)?);
    }
    // All discovery and movement is paid before the sole mutation. Required
    // dependencies of removed optionals remain conservative, as in the raw pass.
    work.charge(branch.opts.len())?;
    if let crate::CompilerWorkMode::Metered(context) = work.mode {
        context.reserve_checked_product(&[
            branch.opts.len(),
            std::mem::size_of::<crate::iq::OptJoin>(),
        ])?;
    }
    work.checkpoint()?;
    let mut keep = keep.into_iter();
    branch
        .opts
        .retain(|_| keep.next().expect("paid optional decision"));
    work.checkpoint()
}

/// Parent elimination needs complete column coverage and retained decoder identity.
pub(super) fn parent_columns(
    branch: &Branch,
    parent: usize,
    allowed: &[&str],
    work: BuildWork<'_>,
) -> Result<bool> {
    let accepts = |alias: usize, name: &str| -> Result<bool> {
        work.charge(1)?;
        if alias != parent {
            return Ok(true);
        }
        for candidate in allowed {
            work.charge(1)?;
            if equal(candidate, name, work)? {
                return Ok(true);
            }
        }
        Ok(false)
    };
    for definition in branch.bindings.values() {
        work.charge(1)?;
        for (alias, name) in super::term_columns_with_work(definition, work)? {
            if !accepts(alias, name)? {
                return Ok(false);
            }
        }
    }
    for condition in &branch.where_conds {
        work.charge(1)?;
        if decoder(condition, Some(parent), work)? {
            return Ok(false);
        }
        let mut covered = true;
        super::condition_columns_with_work(condition, work, &mut |alias, name| {
            covered &= accepts(alias, name)?;
            Ok(())
        })?;
        if !covered {
            return Ok(false);
        }
    }
    for optional in &branch.opts {
        work.charge(1)?;
        for condition in optional.on.iter().chain(&optional.extra) {
            work.charge(1)?;
            if decoder(condition, Some(parent), work)? {
                return Ok(false);
            }
            let mut covered = true;
            super::condition_columns_with_work(condition, work, &mut |alias, name| {
                covered &= accepts(alias, name)?;
                Ok(())
            })?;
            if !covered {
                return Ok(false);
            }
        }
    }
    work.checkpoint()?;
    Ok(true)
}
