//! Prepare the rendered projection before changing the admitted source.
use super::*;
use crate::build::control::BuildWork;
use crate::{CompilerWorkMode, Result};

fn late(condition: &SqlCond, work: BuildWork<'_>) -> Result<bool> {
    let work = work.enter()?;
    Ok(match condition {
        SqlCond::IriCmp(cmp) => {
            let mut found = false;
            for operand in [&cmp.left, &cmp.right] {
                work.charge(1)?;
                if matches!(
                    operand,
                    crate::iq::iri_cmp::IriOperand::Template { base: Some(_), .. }
                ) {
                    found = true;
                    break;
                }
            }
            found
        }
        SqlCond::Not(inner) => late(inner, work)?,
        SqlCond::And(parts) | SqlCond::Or(parts) => {
            let mut found = false;
            for part in parts {
                if late(part, work)? {
                    found = true;
                    break;
                }
            }
            found
        }
        _ => false,
    })
}

pub(crate) fn guard(condition: &SqlCond, alias: usize, work: BuildWork<'_>) -> Result<bool> {
    let work = work.enter()?;
    Ok(match condition {
        SqlCond::IsNull(c) | SqlCond::IsNotNull(c) | SqlCond::DecodedIsNotNull(c) => {
            c.alias == alias
        }
        SqlCond::IriCmp(cmp) => {
            for operand in [&cmp.left, &cmp.right] {
                work.charge(1)?;
                if let crate::iq::iri_cmp::IriOperand::Template { parts, .. } = operand {
                    work.charge(parts.len())?;
                }
                for column in operand.columns() {
                    work.charge(1)?;
                    if column.alias != alias {
                        return Ok(false);
                    }
                }
            }
            true
        }
        SqlCond::Not(inner) => guard(inner, alias, work)?,
        SqlCond::And(parts) | SqlCond::Or(parts) => {
            for part in parts {
                if !guard(part, alias, work)? {
                    return Ok(false);
                }
            }
            true
        }
        _ => false,
    })
}

pub(crate) fn wrap_with_work(
    branch: &mut Branch,
    dialect: sf_sql::Dialect,
    work: BuildWork<'_>,
) -> Result<bool> {
    work.charge(1)?;
    if dialect != sf_sql::Dialect::Sqlite
        || branch.core.len() != 1
        || !branch.opts.is_empty()
        || !branch.subplan_joins.is_empty()
        || branch.path.is_some()
        || branch.agg.is_some()
        || !branch.order.is_empty()
        || !matches!(branch.core[0].source, ScanSource::Logical(_))
    {
        return Ok(false);
    }
    let mut needed = false;
    for def in branch.bindings.values() {
        if !super::super::resolve_work::injective(def, work)? {
            needed = true;
            break;
        }
    }
    if !needed {
        for condition in &branch.where_conds {
            if late(condition, work)? {
                needed = true;
                break;
            }
        }
    }
    if !needed {
        return Ok(false);
    }
    let alias = branch.core[0].alias;
    for condition in &branch.where_conds {
        if !guard(condition, alias, work)? {
            return Ok(false);
        }
    }
    let mut count = 0;
    for def in branch.bindings.values() {
        work.charge(1)?;
        match def {
            TermDef::Const(_) => (),
            TermDef::Derived {
                term_map: TermMap::Template(_, spec),
                alias: owner,
            } if *owner == alias && spec.term_type == TermType::Iri => count += 1,
            _ => return Ok(false),
        }
    }
    let mut prepared = work.vector(count)?;
    let mut columns = work.vector(count)?;
    for def in branch.bindings.values() {
        work.charge(1)?;
        if let TermDef::Derived {
            term_map: TermMap::Template(_, spec),
            ..
        } = def
        {
            let index = prepared.len();
            let digits = if index == 0 {
                1
            } else {
                index.ilog10() as usize + 1
            };
            work.charge(2 + digits)?;
            let name: Box<str> = format!("rv{index}").into();
            if let CompilerWorkMode::Metered(cx) = work.mode {
                cx.reserve_ast_copy(
                    crate::plan_measure::clone_root::CompilerCloneRootV1::TermSpec(spec),
                )?;
            }
            let mut resolved = spec.clone();
            resolved.base = None;
            let output = TermMap::Column(work.variable(&name)?, resolved);
            prepared.push((name, output));
        }
    }
    // Publication has no fallible work: reserve its traversal and Box first.
    work.charge(branch.bindings.len())?;
    work.charge(std::mem::size_of::<Scan>())?;
    work.checkpoint()?;
    let mut prepared = prepared.into_iter();
    for def in branch.bindings.values_mut() {
        if let TermDef::Derived { term_map, .. } = def {
            let (name, output) = prepared.next().expect("validated template inventory");
            columns.push((name, std::mem::replace(term_map, output)));
        }
    }
    let input = branch.core.pop().expect("validated logical scan");
    branch.core.push(Scan {
        alias,
        source: ScanSource::Projection {
            input: Box::new(input),
            columns,
            guards: std::mem::take(&mut branch.where_conds),
            distinct: true,
            native_keys: Vec::new(),
            lexical_keys: Vec::new(),
        },
    });
    branch.distinct = false;
    Ok(true)
}

#[cfg(test)]
#[path = "rendered_distinct_work_tests.rs"]
mod tests;
