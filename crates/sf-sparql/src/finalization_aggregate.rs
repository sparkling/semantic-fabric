//! Build the aggregate DISTINCT wrapper before publishing any branch mutation.
use sf_core::ir::{TermMap, TermSpec};
use sf_core::query_control::QueryControlError;
use sf_sql::Dialect;

use crate::build::control::{BuildVec, BuildWork};
use crate::finalization_remap as remap;
use crate::iq::{Branch, ColRef, SubPlanJoin, TermDef};
use crate::plan_measure::clone_root::CompilerCloneRootV1 as Root;
use crate::{CompilerWorkMode, Error, Plan, PlanForm, Result};

#[cfg(test)]
#[path = "finalization_aggregate_tests.rs"]
mod tests;

fn overflow(work: BuildWork<'_>) -> Error {
    match work.mode {
        CompilerWorkMode::Metered(cx) => {
            cx.reject_build_resource(QueryControlError::AccountingOverflow)
        }
        CompilerWorkMode::Uncontrolled => {
            Error::Unsupported("aggregate wrapper alias overflow".into())
        }
    }
}

fn unique(cols: &mut BuildVec<ColRef>, col: &ColRef, work: BuildWork<'_>) -> Result<()> {
    for existing in &cols.values {
        if remap::equal(existing, col, work)? {
            return Ok(());
        }
    }
    let column = work.string(&col.column)?.into_boxed_str();
    work.push(
        cols,
        ColRef {
            alias: col.alias,
            column,
        },
    )
}

pub(crate) fn apply(branches: &mut [Branch], dialect: Dialect, work: BuildWork<'_>) -> Result<()> {
    work.checkpoint()?;
    for branch in branches {
        wrap(branch, dialect, work)?;
    }
    work.checkpoint()
}

fn wrap(branch: &mut Branch, dialect: Dialect, work: BuildWork<'_>) -> Result<()> {
    work.charge(1)?;
    let Some(agg) = branch.agg.as_ref().filter(|_| branch.distinct) else {
        return Ok(());
    };
    let mut cols = BuildVec::new(Vec::new());
    for key in &agg.keys {
        work.charge(1)?;
        for col in &key.cols {
            unique(&mut cols, col, work)?;
        }
    }
    for item in &agg.aggs {
        work.charge(1)?;
        if let Some(col) = &item.arg {
            unique(&mut cols, col, work)?;
        }
    }
    let cols = cols.into_inner();
    if cols.is_empty() {
        return Ok(());
    }

    let mut maximum = 0;
    for alias in branch
        .core
        .iter()
        .map(|s| s.alias)
        .chain(branch.opts.iter().map(|o| o.scan.alias))
        .chain(branch.subplan_joins.iter().map(|s| s.alias))
    {
        work.charge(1)?;
        maximum = maximum.max(alias);
    }
    let alias = maximum.checked_add(1).ok_or_else(|| overflow(work))?;

    if let CompilerWorkMode::Metered(cx) = work.mode {
        cx.reserve_ast_copy(Root::Aggregation(agg))?;
    }
    let mut agg = agg.clone();
    for key in &mut agg.keys {
        work.charge(1)?;
        for col in &mut key.cols {
            *col = remap::colref(col, &cols, alias, work)?;
        }
    }
    for item in &mut agg.aggs {
        work.charge(1)?;
        if let Some(col) = &mut item.arg {
            *col = remap::colref(col, &cols, alias, work)?;
        }
    }

    let mut replacements = BuildVec::new(Vec::new());
    for (key, def) in &branch.bindings {
        work.charge(1)?;
        if matches!(def, TermDef::Derived { .. } | TermDef::R2rmlBlank { .. }) {
            match remap::term(def, &cols, alias, work) {
                Ok(value) => {
                    let key = work.string(key)?;
                    work.push(&mut replacements, (key, value))?;
                }
                // Preserve the raw wrapper's unsupported-remap no-op. Never
                // swallow cancellation, exhaustion or arithmetic failure.
                Err(Error::Unsupported(_)) => {}
                Err(error) => return Err(error),
            }
        }
    }

    let mut inner = Branch::empty();
    inner.distinct = true;
    let mut vars = work.vector(cols.len())?;
    for (index, col) in cols.iter().enumerate() {
        let key = remap::name("k", index, 4, work)?;
        vars.push(work.string(&key)?);
        let column = work.string(&col.column)?.into_boxed_str();
        for existing in inner.bindings.keys() {
            work.charge(1)?;
            work.charge(existing.len().min(key.len()))?;
        }
        work.charge(std::mem::size_of::<(String, TermDef)>())?;
        inner.bindings.insert(
            key,
            TermDef::Derived {
                term_map: TermMap::Column(column, TermSpec::plain_literal()),
                alias: col.alias,
            },
        );
    }
    let mut nested = work.vector(1)?;
    nested.push(inner);
    let mut plan = work.boxed(Plan {
        branches: nested,
        form: PlanForm::Select { vars },
        distinct: true,
        limit: None,
        offset: 0,
        order: Vec::new(),
        rust_group: None,
        dialect,
        dedup_scopes: Vec::new(),
        construct_drops_some_branch_var: false,
    })?;
    let count = branch
        .subplan_joins
        .len()
        .checked_add(1)
        .ok_or_else(|| overflow(work))?;
    let mut joins = work.vector::<SubPlanJoin>(count)?;
    // Existing joins will move once into the replacement vector.
    work.charge(branch.subplan_joins.len())?;
    for (key, _) in &replacements.values {
        for existing in branch.bindings.keys() {
            work.charge(1)?;
            work.charge(existing.len().min(key.len()))?;
        }
    }
    work.charge(replacements.values.len())?;
    work.checkpoint()?;
    // All fallible preparation completed. Publish one coherent wrapper.
    plan.branches[0].core = std::mem::take(&mut branch.core);
    plan.branches[0].opts = std::mem::take(&mut branch.opts);
    plan.branches[0].where_conds = std::mem::take(&mut branch.where_conds);
    joins.append(&mut branch.subplan_joins);
    joins.push(SubPlanJoin {
        alias,
        plan,
        on: Vec::new(),
        left: false,
    });
    branch.subplan_joins = joins;
    branch.agg = Some(agg);
    for (key, value) in replacements.into_inner() {
        *branch
            .bindings
            .get_mut(&key)
            .expect("prepared existing key") = value;
    }
    branch.distinct = false;
    Ok(())
}
