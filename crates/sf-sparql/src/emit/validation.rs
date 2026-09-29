//! Live column preflight: one paid queue validates every reachable source column
//! before any branch cursor opens.
use super::*;

#[path = "validation_scope.rs"]
mod validation_scope;
use validation_scope::{
    validate_hop, validate_named_ref, validate_ref, AliasMap, AliasSource, Task,
};

/// Validate every raw base-source column that live emission can reference before
/// any branch cursor opens. Offline [`emit_branch`] remains permissive because it
/// never calls this preflight and continues to emit mapping-authored identifiers.
#[cfg(test)]
pub(crate) fn validate_live_columns(
    branches: &[Branch],
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> Result<()> {
    validate_live_columns_controlled(
        branches,
        dialect,
        catalog,
        sf_sql::source_work::SourceWork::new(None),
    )
}

#[cfg(test)]
pub(crate) fn validate_live_columns_controlled(
    branches: &[Branch],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<()> {
    validate_source_root(ValidationRoot::Branches(branches), dialect, catalog, work)
}

pub(crate) fn validate_execution_columns(
    branches: &[Branch],
    scopes: &[Option<crate::DedupScope>],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<()> {
    validate_source_root(
        ValidationRoot::Execution(branches, scopes),
        dialect,
        catalog,
        work,
    )
}

pub(crate) enum ValidationRoot<'a> {
    Execution(&'a [Branch], &'a [Option<crate::DedupScope>]),
    #[cfg(test)]
    Branches(&'a [Branch]),
    #[cfg(test)]
    Projection(
        &'a crate::iq::Scan,
        &'a [(Box<str>, TermMap)],
        &'a [SqlCond],
    ),
}

/// One queue owns every Branch/Scan/EXISTS/Projection continuation. Leaf checks
/// cannot recurse back into this driver; scopes are restored before resuming.
pub(crate) fn validate_source_root(
    root: ValidationRoot<'_>,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<()> {
    use crate::iq::ScanSource;
    use sf_sql::source_work::SourceVec;
    use source_control::validation_error as error;
    work.charge(1).map_err(error)?;
    let mut pending = SourceVec::default();
    let mut scopes: SourceVec<AliasMap<'_>> = Default::default();
    match root {
        ValidationRoot::Execution(branches, scopes) => pending
            .push(Task::Execution(branches, scopes), work)
            .map_err(error)?,
        #[cfg(test)]
        ValidationRoot::Branches(branches) => pending
            .push(Task::Branches(branches), work)
            .map_err(error)?,
        #[cfg(test)]
        ValidationRoot::Projection(input, columns, guards) => pending
            .push(Task::Projection(input, columns, guards), work)
            .map_err(error)?,
    }
    while let Some(task) = pending.pop() {
        work.charge(1).map_err(error)?;
        match task {
            Task::Execution(items, scopes) => {
                if let Some((first, rest)) = items.split_first() {
                    let (scope, remaining) = scopes
                        .split_first()
                        .map_or((None, scopes), |(scope, rest)| (scope.as_ref(), rest));
                    pending
                        .push(Task::Execution(rest, remaining), work)
                        .map_err(error)?;
                    pending
                        .push(Task::Enter(first, scope), work)
                        .map_err(error)?;
                }
            }
            Task::Branches(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending.push(Task::Branches(rest), work).map_err(error)?;
                    pending
                        .push(Task::Enter(first, None), work)
                        .map_err(error)?;
                }
            }
            Task::Scans(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending.push(Task::Scans(rest), work).map_err(error)?;
                    pending.push(Task::Scan(first), work).map_err(error)?;
                }
            }
            Task::OptScans(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending.push(Task::OptScans(rest), work).map_err(error)?;
                    pending.push(Task::Scan(&first.scan), work).map_err(error)?;
                }
            }
            Task::Conditions(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending.push(Task::Conditions(rest), work).map_err(error)?;
                    pending.push(Task::Condition(first), work).map_err(error)?;
                }
            }
            Task::OptConditions(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending
                        .push(Task::OptConditions(rest), work)
                        .map_err(error)?;
                    pending
                        .push(Task::Conditions(&first.extra), work)
                        .map_err(error)?;
                    pending
                        .push(Task::Conditions(&first.on), work)
                        .map_err(error)?;
                }
            }
            Task::Joins(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending.push(Task::Joins(rest), work).map_err(error)?;
                    pending.push(Task::Join(first), work).map_err(error)?;
                }
            }
            Task::Enter(branch, scope) => {
                scopes.push(AliasMap::default(), work).map_err(error)?;
                pending
                    .push(Task::Body(branch, scope), work)
                    .map_err(error)?;
                pending
                    .push(Task::OptScans(&branch.opts), work)
                    .map_err(error)?;
                pending
                    .push(Task::Scans(&branch.core), work)
                    .map_err(error)?;
            }
            Task::Scan(scan) => match &scan.source {
                ScanSource::Logical(source) => pending
                    .push(Task::Install(scan.alias, AliasSource::Base(source)), work)
                    .map_err(error)?,
                ScanSource::Path { closure, .. } => {
                    validate_hop(&closure.hop, dialect, catalog, work)?;
                    pending
                        .push(Task::Install(scan.alias, AliasSource::Path), work)
                        .map_err(error)?;
                }
                ScanSource::RefAtom { input, columns } => {
                    ref_atom::validate_shape_controlled(input, columns, work)?;
                    pending
                        .push(
                            Task::Install(scan.alias, AliasSource::RefAtom(columns.len())),
                            work,
                        )
                        .map_err(error)?;
                    pending
                        .push(Task::Enter(input, None), work)
                        .map_err(error)?;
                }
                ScanSource::Projection {
                    input,
                    columns,
                    guards,
                    ..
                } => {
                    pending
                        .push(
                            Task::Install(scan.alias, AliasSource::Projection(columns)),
                            work,
                        )
                        .map_err(error)?;
                    pending
                        .push(Task::Projection(input, columns, guards), work)
                        .map_err(error)?;
                }
            },
            Task::Install(alias, source) => {
                let mut aliases = scopes.pop().expect("scan installation has a caller scope");
                aliases.insert(alias, source, work)?;
                scopes.push(aliases, work).map_err(error)?;
            }
            Task::Body(branch, scope) => {
                let mut aliases = scopes.pop().expect("branch body has a scope");
                for subplan in &branch.subplan_joins {
                    aliases.insert(subplan.alias, AliasSource::Derived, work)?;
                }
                let bindings = BindingView::merged(
                    &branch.bindings,
                    scope.map(|scope| &scope.key_bindings),
                    work,
                )?;
                for (_, definition) in bindings.iter() {
                    source_control::validate_definition_columns(
                        definition,
                        work,
                        |alias, column| {
                            validate_named_ref(alias, column, &aliases, dialect, catalog, work)
                        },
                    )?;
                }
                scopes.push(aliases, work).map_err(error)?;
                pending.push(Task::Finish(branch), work).map_err(error)?;
                pending
                    .push(Task::Joins(&branch.subplan_joins), work)
                    .map_err(error)?;
                pending
                    .push(Task::OptConditions(&branch.opts), work)
                    .map_err(error)?;
                pending
                    .push(Task::Conditions(&branch.where_conds), work)
                    .map_err(error)?;
            }
            Task::Join(join) => {
                pending
                    .push(Task::Branches(&join.plan.branches), work)
                    .map_err(error)?;
                pending
                    .push(Task::Conditions(&join.on), work)
                    .map_err(error)?;
            }
            Task::Finish(branch) => {
                let aliases = scopes.pop().expect("branch finish has a scope");
                if let Some(path) = &branch.path {
                    validate_hop(&path.hop, dialect, catalog, work)?;
                }
                if let Some(aggregation) = &branch.agg {
                    for key in &aggregation.keys {
                        work.charge(1).map_err(error)?;
                        for column in &key.cols {
                            validate_ref(column, &aliases, dialect, catalog, work)?;
                        }
                    }
                    for aggregate in &aggregation.aggs {
                        work.charge(1).map_err(error)?;
                        if let Some(column) = &aggregate.arg {
                            validate_ref(column, &aliases, dialect, catalog, work)?;
                        }
                    }
                }
            }
            Task::Leave => {
                scopes.pop().expect("condition exit has a scope");
            }
            Task::Condition(condition) => {
                let aliases = scopes.as_slice().last().expect("condition has a scope");
                let result: Result<()> = match condition {
                    SqlCond::Not(inner) => {
                        pending.push(Task::Condition(inner), work).map_err(error)
                    }
                    SqlCond::And(parts) | SqlCond::Or(parts) => {
                        pending.push(Task::Conditions(parts), work).map_err(error)
                    }
                    SqlCond::Exists { scans, conds } | SqlCond::NotExists { scans, conds } => {
                        let nested = aliases.copy(work)?;
                        scopes.push(nested, work).map_err(error)?;
                        pending.push(Task::Leave, work).map_err(error)?;
                        pending.push(Task::Conditions(conds), work).map_err(error)?;
                        pending.push(Task::Scans(scans), work).map_err(error)?;
                        Ok(())
                    }
                    SqlCond::PathExists { pc, conds, .. } => {
                        validate_hop(&pc.hop, dialect, catalog, work)?;
                        pending.push(Task::Conditions(conds), work).map_err(error)?;
                        Ok(())
                    }
                    SqlCond::ExpressionError => Ok(()),
                    SqlCond::IriCmp(cmp) => {
                        use crate::iq::iri_cmp::{IriOperand, IriPart};
                        for operand in [&cmp.left, &cmp.right] {
                            work.charge(1).map_err(source_control::validation_error)?;
                            match operand {
                                IriOperand::Column { column, .. } => {
                                    validate_ref(column, aliases, dialect, catalog, work)?
                                }
                                IriOperand::Template { parts, .. } => {
                                    for part in parts {
                                        work.charge(1).map_err(source_control::validation_error)?;
                                        if let IriPart::Column(column) = part {
                                            validate_ref(column, aliases, dialect, catalog, work)?;
                                        }
                                    }
                                }
                                IriOperand::Constant(_) => {}
                            }
                        }
                        Ok(())
                    }
                    SqlCond::LiteralCmp(cmp) => {
                        for column in cmp.columns() {
                            validate_ref(column, aliases, dialect, catalog, work)?;
                        }
                        Ok(())
                    }
                    SqlCond::ColEq(left, right)
                    | SqlCond::NativeColEq(left, right)
                    | SqlCond::NullSafeEq(left, right) => {
                        validate_ref(left, aliases, dialect, catalog, work)?;
                        validate_ref(right, aliases, dialect, catalog, work)
                    }
                    SqlCond::Cmp(column, _, _)
                    | SqlCond::NativeCmp(column, _, _)
                    | SqlCond::IsNotNull(column)
                    | SqlCond::DecodedIsNotNull(column)
                    | SqlCond::IsNull(column)
                    | SqlCond::StrMatch { col: column, .. } => {
                        validate_ref(column, aliases, dialect, catalog, work)
                    }
                    SqlCond::TemplateEq(left, left_alias, right, right_alias, _) => {
                        for segment in left {
                            work.charge(1).map_err(source_control::validation_error)?;
                            if let Segment::Column(column) = segment {
                                validate_named_ref(
                                    *left_alias,
                                    column,
                                    aliases,
                                    dialect,
                                    catalog,
                                    work,
                                )?;
                            }
                        }
                        for segment in right {
                            work.charge(1).map_err(source_control::validation_error)?;
                            if let Segment::Column(column) = segment {
                                validate_named_ref(
                                    *right_alias,
                                    column,
                                    aliases,
                                    dialect,
                                    catalog,
                                    work,
                                )?;
                            }
                        }
                        Ok(())
                    }
                };
                result?;
            }
            Task::Projection(input, columns, guards) => {
                pending
                    .push(Task::ProjectionInput(input, columns, guards), work)
                    .map_err(error)?;
                if let ScanSource::Projection {
                    input,
                    columns,
                    guards,
                    ..
                } = &input.source
                {
                    pending
                        .push(Task::Projection(input, columns, guards), work)
                        .map_err(error)?;
                }
            }
            Task::ProjectionInput(input, columns, guards) => {
                pending
                    .push(Task::ProjectionOutput(input, columns, guards), work)
                    .map_err(error)?;
                if let ScanSource::RefAtom { input, columns } = &input.source {
                    ref_atom::validate_shape_controlled(input, columns, work)?;
                    pending
                        .push(Task::Enter(input, None), work)
                        .map_err(error)?;
                }
            }
            Task::ProjectionOutput(input, columns, guards) => {
                projection_layout::validate_projection_level(
                    input, columns, guards, dialect, catalog, work,
                )?
            }
        }
    }
    work.checkpoint().map_err(error)
}
