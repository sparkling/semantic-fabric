//! Explicit child/continuation ownership for source metadata construction.
//! Child traversal and derived transforms share prospective source accounting.
//! SQL rendering and driver work remain separate admission boundaries.
use super::*;
use crate::iq::{Scan, ScanSource};
use sf_sql::source_work::{SourceVec, SourceWork};
use source_control::validation_error as error;

enum Step<'a> {
    Branch(&'a Branch),
    Next(&'a Branch, usize, ActualColumns, bool),
    Install(&'a Branch, usize, usize, ActualColumns, bool),
    Scan(&'a Scan),
    Projection(&'a ScanSource),
    Reference(&'a [ColRef]),
    Joins(&'a Branch, usize, ActualColumns, bool),
    JoinInstall(&'a Branch, usize, ActualColumns, bool),
    PlanNext(&'a crate::Plan, usize, metadata_subplan::Accumulator),
    PlanMerge(&'a crate::Plan, usize, metadata_subplan::Accumulator),
}

enum Value {
    Branch(ActualColumns, bool),
    Alias(AliasActuals),
}

fn install(
    out: &mut ActualColumns,
    alias: usize,
    actuals: AliasActuals,
    work: SourceWork<'_>,
) -> Result<()> {
    work.charge(1).map_err(error)?;
    work.product(out.len(), 2 + std::mem::size_of::<(usize, AliasActuals)>())
        .map_err(error)?;
    work.charge(std::mem::size_of::<(usize, AliasActuals)>())
        .map_err(error)?;
    out.try_reserve(1)
        .map_err(|_| Error::Sql("alias metadata allocation failed".into()))?;
    out.insert(alias, actuals);
    work.checkpoint().map_err(error)
}

fn run(
    first: Step<'_>,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: SourceWork<'_>,
) -> Result<Value> {
    let mut pending = SourceVec::default();
    pending.push(first, work).map_err(error)?;
    let mut value = None;
    while !pending.as_slice().is_empty() {
        work.charge(1).map_err(error)?;
        match pending.pop().expect("nonempty metadata continuation") {
            Step::Branch(branch) => {
                let mut out = HashMap::new();
                let path = metadata_path::local_flag(branch, work)?;
                for (alias, source) in scan::logical_aliases(branch, work)? {
                    install(
                        &mut out,
                        alias,
                        source_actuals_controlled(source, catalog, work)?,
                        work,
                    )?;
                }
                pending
                    .push(Step::Next(branch, 0, out, path), work)
                    .map_err(error)?;
            }
            Step::Next(branch, index, mut out, path) => {
                let scan = if index < branch.core.len() {
                    branch.core.get(index)
                } else {
                    branch
                        .opts
                        .get(index - branch.core.len())
                        .map(|join| &join.scan)
                };
                if let Some(scan) = scan {
                    pending
                        .push(Step::Install(branch, index, scan.alias, out, path), work)
                        .map_err(error)?;
                    pending.push(Step::Scan(scan), work).map_err(error)?;
                } else {
                    // Path metadata uses the same paid continuation/control boundary.
                    if let Some(path) = &branch.path {
                        install(
                            &mut out,
                            path.alias,
                            metadata_path::actuals(path, catalog, work)?,
                            work,
                        )?;
                    }
                    pending
                        .push(Step::Joins(branch, 0, out, path), work)
                        .map_err(error)?;
                }
            }
            Step::Install(branch, index, alias, mut out, path) => {
                let Some(Value::Alias(actuals)) = value.take() else {
                    return Err(invariant());
                };
                install(&mut out, alias, actuals, work)?;
                let next = index.checked_add(1).ok_or_else(invariant)?;
                pending
                    .push(Step::Next(branch, next, out, path), work)
                    .map_err(error)?;
            }
            Step::Scan(scan) => match &scan.source {
                ScanSource::Logical(source) => {
                    value = Some(Value::Alias(source_actuals_controlled(
                        source, catalog, work,
                    )?))
                }
                ScanSource::Projection { input, .. } => {
                    pending
                        .push(Step::Projection(&scan.source), work)
                        .map_err(error)?;
                    pending.push(Step::Scan(input), work).map_err(error)?;
                }
                ScanSource::RefAtom { input, columns } => {
                    pending
                        .push(Step::Reference(columns), work)
                        .map_err(error)?;
                    pending.push(Step::Branch(input), work).map_err(error)?;
                }
                ScanSource::Path { closure, .. } => {
                    value = Some(Value::Alias(metadata_path::actuals(
                        closure, catalog, work,
                    )?))
                }
            },
            Step::Projection(source) => {
                let Some(Value::Alias(inner)) = value.take() else {
                    return Err(invariant());
                };
                value = Some(Value::Alias(metadata_projection::actuals(
                    source, dialect, &inner, work,
                )?));
            }
            Step::Reference(columns) => {
                let Some(Value::Branch(inner, _)) = value.take() else {
                    return Err(invariant());
                };
                value = Some(Value::Alias(metadata_ref_atom::actuals(
                    columns, &inner, work,
                )?));
            }
            Step::Joins(branch, index, out, path) => {
                if let Some(join) = branch.subplan_joins.get(index) {
                    pending
                        .push(Step::JoinInstall(branch, index, out, path), work)
                        .map_err(error)?;
                    pending
                        .push(Step::PlanNext(&join.plan, 0, Default::default()), work)
                        .map_err(error)?;
                } else {
                    value = Some(Value::Branch(out, path));
                }
            }
            Step::JoinInstall(branch, index, mut out, mut path) => {
                let Some(Value::Alias(actuals)) = value.take() else {
                    return Err(invariant());
                };
                path |= actuals.path;
                install(&mut out, branch.subplan_joins[index].alias, actuals, work)?;
                pending
                    .push(
                        Step::Joins(
                            branch,
                            index.checked_add(1).ok_or_else(invariant)?,
                            out,
                            path,
                        ),
                        work,
                    )
                    .map_err(error)?;
            }
            Step::PlanNext(plan, index, accumulator) => {
                #[cfg(test)]
                if index == 0 {
                    path_comparison::metadata_visit();
                }
                if let Some(branch) = plan.branches.get(index) {
                    pending
                        .push(Step::PlanMerge(plan, index, accumulator), work)
                        .map_err(error)?;
                    pending.push(Step::Branch(branch), work).map_err(error)?;
                } else {
                    value = Some(Value::Alias(accumulator.finish(dialect, work)?));
                }
            }
            Step::PlanMerge(plan, index, mut accumulator) => {
                let Some(Value::Branch(actuals, path)) = value.take() else {
                    return Err(invariant());
                };
                // Merge one arm and release its metadata before starting another.
                accumulator.add(plan, &plan.branches[index], dialect, &actuals, path, work)?;
                pending
                    .push(
                        Step::PlanNext(
                            plan,
                            index.checked_add(1).ok_or_else(invariant)?,
                            accumulator,
                        ),
                        work,
                    )
                    .map_err(error)?;
            }
        }
        work.checkpoint().map_err(error)?;
    }
    value.ok_or_else(invariant)
}

pub(super) fn invariant() -> Error {
    Error::Sql("invalid metadata continuation state".into())
}

pub(super) fn branch_actuals_controlled(
    branch: &Branch,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: SourceWork<'_>,
) -> Result<ActualColumns> {
    match run(Step::Branch(branch), dialect, catalog, work)? {
        Value::Branch(actuals, _) => Ok(actuals),
        _ => Err(invariant()),
    }
}

pub(super) fn scan_actuals_controlled(
    scan: &Scan,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: SourceWork<'_>,
) -> Result<AliasActuals> {
    match run(Step::Scan(scan), dialect, catalog, work)? {
        Value::Alias(actuals) => Ok(actuals),
        _ => Err(invariant()),
    }
}

#[cfg(test)]
pub(super) fn subplan_actuals_controlled(
    plan: &crate::Plan,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: SourceWork<'_>,
) -> Result<AliasActuals> {
    match run(
        Step::PlanNext(plan, 0, Default::default()),
        dialect,
        catalog,
        work,
    )? {
        Value::Alias(actuals) => Ok(actuals),
        _ => Err(invariant()),
    }
}
