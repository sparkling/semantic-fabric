//! Borrowed condition traversal with paid continuations and one SQL byte buffer.
use super::*;
use sf_sql::source_work::{SourceVec, SourceWork};
use source_control::validation_error as error;
use std::borrow::Cow;

#[cfg(test)]
#[path = "condition_control_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "condition_scope_tests.rs"]
mod scope_tests;

#[derive(Clone, Copy)]
enum List<'a> {
    Refs(&'a [&'a SqlCond]),
    Slice(&'a [SqlCond]),
}
impl<'a> List<'a> {
    fn get(self, index: usize) -> Option<&'a SqlCond> {
        match self {
            Self::Refs(values) => values.get(index).copied(),
            Self::Slice(values) => values.get(index),
        }
    }
}

enum Step<'a> {
    Condition(&'a SqlCond),
    Conjunction(List<'a>, bool),
    List(List<'a>, usize, Option<bool>, &'static str, bool),
    Text(&'static str),
    Restore(Cow<'a, ActualColumns>),
}

/// Only boolean descendants contribute to the local validation barrier. EXISTS
/// has its own scope and policy barrier; looking through it would change SQL.
pub(super) fn validates(
    first: &SqlCond,
    dialect: Dialect,
    actuals: &ActualColumns,
    work: SourceWork<'_>,
) -> Result<bool> {
    let mut pending = SourceVec::default();
    pending
        .push(std::slice::from_ref(first), work)
        .map_err(error)?;
    while let Some(conditions) = pending.pop() {
        work.charge(1).map_err(error)?;
        let Some((condition, rest)) = conditions.split_first() else {
            continue;
        };
        if !rest.is_empty() {
            pending.push(rest, work).map_err(error)?;
        }
        match condition {
            SqlCond::Not(inner) => pending
                .push(std::slice::from_ref(inner.as_ref()), work)
                .map_err(error)?,
            SqlCond::And(inner) | SqlCond::Or(inner) => {
                pending.push(inner.as_slice(), work).map_err(error)?
            }
            _ => {
                condition_metadata::validation(condition, actuals, work)?;
                let found = match dialect {
                    Dialect::MySql => natural_literal::validates_leaf(condition, actuals),
                    Dialect::Postgres => pg_numeric::validates_leaf(condition, actuals),
                    _ => false,
                };
                work.checkpoint().map_err(error)?;
                if found {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

fn protected(
    list: List<'_>,
    dialect: Dialect,
    actuals: &ActualColumns,
    work: SourceWork<'_>,
) -> Result<bool> {
    if !matches!(dialect, Dialect::MySql | Dialect::Postgres) {
        return Ok(false);
    }
    let mut index = 0;
    let mut policy = false;
    while let Some(cond) = list.get(index) {
        work.charge(1).map_err(error)?;
        policy |= matches!(cond, SqlCond::NativeCmp(..));
        index += 1;
    }
    if !policy {
        return Ok(false);
    }
    index = 0;
    while let Some(cond) = list.get(index) {
        work.charge(1).map_err(error)?;
        if !matches!(cond, SqlCond::NativeCmp(..)) && validates(cond, dialect, actuals, work)? {
            return Ok(true);
        }
        index += 1;
    }
    Ok(false)
}

pub(super) fn authorized(
    conds: &[&SqlCond],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: SourceWork<'_>,
) -> Result<Option<String>> {
    if !protected(List::Refs(conds), dialect, actuals, work)? {
        return Ok(None);
    }
    run(
        Step::Conjunction(List::Refs(conds), true),
        dialect,
        catalog,
        actuals,
        params,
        pidx,
        work,
    )
    .map(Some)
}

pub(super) fn conjunction(
    conds: &[&SqlCond],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: SourceWork<'_>,
) -> Result<String> {
    run(
        Step::Conjunction(List::Refs(conds), false),
        dialect,
        catalog,
        actuals,
        params,
        pidx,
        work,
    )
}

pub(super) fn condition(
    cond: &SqlCond,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: SourceWork<'_>,
) -> Result<String> {
    run(
        Step::Condition(cond),
        dialect,
        catalog,
        actuals,
        params,
        pidx,
        work,
    )
}

fn run<'a>(
    first: Step<'a>,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &'a ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: SourceWork<'_>,
) -> Result<String> {
    let mut pending = SourceVec::default();
    pending.push(first, work).map_err(error)?;
    let mut output = SourceVec::default();
    let mut actuals = Cow::Borrowed(actuals);
    while let Some(step) = pending.pop() {
        work.charge(1).map_err(error)?;
        let mut emit = |text: &str| output.extend_bytes(text.as_bytes(), work).map_err(error);
        match step {
            Step::Text(text) => emit(text)?,
            Step::Restore(previous) => actuals = previous,
            Step::Conjunction(list, admitted) => {
                if list.get(0).is_none() {
                    emit("1 = 1")?;
                } else if admitted || protected(list, dialect, &actuals, work)? {
                    emit("CASE WHEN (")?;
                    pending
                        .push(Step::Text(") ELSE FALSE END"), work)
                        .map_err(error)?;
                    pending
                        .push(Step::List(list, 0, Some(false), " AND ", false), work)
                        .map_err(error)?;
                    pending.push(Step::Text(") THEN ("), work).map_err(error)?;
                    pending
                        .push(Step::List(list, 0, Some(true), " AND ", false), work)
                        .map_err(error)?;
                } else {
                    pending
                        .push(Step::List(list, 0, None, " AND ", false), work)
                        .map_err(error)?;
                }
            }
            Step::List(list, mut index, policy, separator, emitted) => {
                while let Some(cond) = list.get(index) {
                    work.charge(1).map_err(error)?;
                    index += 1;
                    if policy.is_some_and(|yes| yes != matches!(cond, SqlCond::NativeCmp(..))) {
                        continue;
                    }
                    if emitted {
                        emit(separator)?;
                    }
                    pending
                        .push(Step::List(list, index, policy, separator, true), work)
                        .map_err(error)?;
                    pending.push(Step::Condition(cond), work).map_err(error)?;
                    break;
                }
            }
            Step::Condition(cond) => match cond {
                SqlCond::Not(inner) => {
                    emit("(NOT ")?;
                    pending.push(Step::Text(")"), work).map_err(error)?;
                    pending.push(Step::Condition(inner), work).map_err(error)?;
                }
                SqlCond::And(conditions) => {
                    emit("(")?;
                    pending.push(Step::Text(")"), work).map_err(error)?;
                    pending
                        .push(Step::Conjunction(List::Slice(conditions), false), work)
                        .map_err(error)?;
                }
                SqlCond::Or(conditions) => {
                    if conditions.is_empty() {
                        emit("1 = 0")?;
                    } else {
                        emit("(")?;
                        pending.push(Step::Text(")"), work).map_err(error)?;
                        pending
                            .push(
                                Step::List(List::Slice(conditions), 0, None, " OR ", false),
                                work,
                            )
                            .map_err(error)?;
                    }
                }
                SqlCond::Exists { scans, conds } | SqlCond::NotExists { scans, conds } => {
                    emit(if matches!(cond, SqlCond::NotExists { .. }) {
                        "NOT EXISTS (SELECT 1"
                    } else {
                        "EXISTS (SELECT 1"
                    })?;
                    for (index, scan) in scans.iter().enumerate() {
                        work.charge(1).map_err(error)?;
                        emit(if index == 0 { " FROM " } else { " CROSS JOIN " })?;
                        emit(&scan_ref_controlled(
                            scan, dialect, catalog, params, pidx, work,
                        )?)?;
                    }
                    let mut nested = condition_metadata::copy(&actuals, work)?;
                    for scan in scans {
                        let alias =
                            metadata::scan_actuals_controlled(scan, dialect, catalog, work)?;
                        condition_metadata::install(&mut nested, scan.alias, alias, work)?;
                    }
                    emit(" WHERE ")?;
                    let previous = std::mem::replace(&mut actuals, Cow::Owned(nested));
                    pending.push(Step::Restore(previous), work).map_err(error)?;
                    pending.push(Step::Text(")"), work).map_err(error)?;
                    pending
                        .push(Step::Conjunction(List::Slice(conds), false), work)
                        .map_err(error)?;
                }
                SqlCond::PathExists { pc, conds, negated } => {
                    emit(if *negated { "NOT EXISTS (" } else { "EXISTS (" })?;
                    emit(&path_sql::prelude(pc, pc.alias, dialect, catalog, work)?)?;
                    emit(&path_sql::format::render(
                        work,
                        format_args!(" SELECT 1 FROM t{} WHERE ", pc.alias),
                    )?)?;
                    let mut nested = condition_metadata::copy(&actuals, work)?;
                    condition_metadata::install(
                        &mut nested,
                        pc.alias,
                        path_actuals_controlled(pc, catalog, work)?,
                        work,
                    )?;
                    let previous = std::mem::replace(&mut actuals, Cow::Owned(nested));
                    pending.push(Step::Restore(previous), work).map_err(error)?;
                    pending.push(Step::Text(")"), work).map_err(error)?;
                    pending
                        .push(Step::Conjunction(List::Slice(conds), false), work)
                        .map_err(error)?;
                }
                _ => emit(&condition_leaf::render(
                    cond, dialect, catalog, &actuals, params, pidx, work,
                )?)?,
            },
        }
        work.checkpoint().map_err(error)?;
    }
    // All appended pieces are UTF-8. Validation is itself paid; conversion reuses
    // the existing byte allocation instead of copying the completed SQL again.
    work.charge(output.as_slice().len()).map_err(error)?;
    let result = String::from_utf8(output.into_vec())
        .map_err(|_| Error::Sql("condition SQL encoding".into()))?;
    work.checkpoint().map_err(error)?;
    Ok(result)
}
