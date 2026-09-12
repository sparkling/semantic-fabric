//! Request-owned cascade execution with operation-local work and stop checks.
//! Uncontrolled execution retains the legacy entry. Shared pass implementations
//! still require independent semantic and public-path acceptance evidence.
use super::*;
use crate::build::control::{BuildVec, BuildWork};
use crate::{CompilerWorkMode, Result};

pub(crate) fn run(
    branches: Vec<Branch>,
    schema: &[TableSchema],
    context: &CascadeCtx<'_>,
    work: BuildWork<'_>,
) -> Result<Vec<Branch>> {
    if matches!(work.mode, CompilerWorkMode::Uncontrolled) {
        return Ok(super::run(branches, schema, context));
    }
    // Empty output has no branch-depth visit; schema admission and the final
    // checkpoint still run, including when the caller is already cancelled.
    let work = if branches.is_empty() {
        work
    } else {
        work.enter()?
    };
    let schema_map = resolve_schema::build(schema, work)?;
    let mut output = BuildVec::new(Vec::new());
    for mut branch in branches {
        work.charge(1)?;
        if branch.path.is_some() {
            work.push(&mut output, branch)?;
            continue;
        }
        if branch.agg.is_some() {
            control_join::inner(&mut branch, &schema_map, work)?;
            control_join::nullable(&mut branch, &schema_map, work)?;
            work.push(&mut output, branch)?;
            continue;
        }
        if !branch.subplan_joins.is_empty() {
            work.push(&mut output, branch)?;
            continue;
        }
        if control_conditions::has_subquery(&branch, work)? {
            control_join::subqueries(&mut branch.where_conds, &schema_map, work)?;
            work.push(&mut output, branch)?;
            continue;
        }
        tier0_eliminate(&mut branch, &schema_map);
        if !control_conditions::consistent(&branch, work)? {
            continue;
        }
        control_join::inner(&mut branch, &schema_map, work)?;
        control_join::nullable(&mut branch, &schema_map, work)?;
        if !control_conditions::consistent(&branch, work)? {
            continue;
        }
        control_join::downgrade(&mut branch, schema, work)?;
        control_join::left(&mut branch, &schema_map, work)?;
        sameterm::with_work(&mut branch, context, work)?;
        control_distinct::prune_optional(&mut branch, context, work)?;
        control_fd::eliminate(&mut branch, &schema_map, context, work)?;
        let fds = control_fd::infer(&branch, schema, work)?;
        joinelim::with_work(&mut branch, schema, &fds, work)?;
        control_conditions::selection(&mut branch, work)?;
        if !control_conditions::intersect(&mut branch, work)? {
            continue;
        }
        work.push(&mut output, branch)?;
    }
    let mut output = output.into_inner();
    if context.distinct && output.len() == 1 && output[0].path.is_none() && output[0].agg.is_none()
    {
        work.charge(1)?;
        output[0].distinct = true;
        if output[0].subplan_joins.is_empty() {
            control_distinct::remove(&mut output[0], &schema_map, context.project, work)?;
        }
    }
    if let Some(project) = context.project {
        for branch in &mut output {
            work.charge(1)?;
            let mut keep = work.vector(branch.bindings.len())?;
            for variable in branch.bindings.keys() {
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
                keep.push(projected);
            }
            work.charge(branch.bindings.len())?;
            work.checkpoint()?;
            let mut keep = keep.into_iter();
            branch
                .bindings
                .retain(|_, _| keep.next().expect("one paid decision per binding"));
        }
    }
    work.checkpoint()?;
    Ok(output)
}

/// Independent pricing for VALUES-only LOWER-boundary fixtures. Do not measure
/// the whole translation: earlier clone-refusal expectations remain independent.
#[cfg(test)]
pub(crate) fn values_work(query: &spargebra::Query) -> u64 {
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits};
    let spargebra::Query::Select { pattern, .. } = query else {
        panic!("SELECT fixture required");
    };
    let built = crate::build::build_tree(pattern, None).unwrap();
    let normalized = crate::iq::normalize::normalize(built).unwrap();
    let plan = crate::iq::lower::lower(
        normalized,
        sf_sql::Dialect::Sqlite,
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    let crate::PlanForm::Select { vars } = &plan.form else {
        panic!("SELECT plan required");
    };
    let context = CascadeCtx {
        distinct: plan.distinct,
        project: Some(vars),
    };
    let budget = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    let work = BuildWork::new(CompilerWorkMode::Metered(
        crate::compiler_control::CompileContext::new(&budget),
    ));
    run(plan.branches, &[], &context, work).unwrap();
    budget.consumed(QueryCharge::CompilerWork)
}
