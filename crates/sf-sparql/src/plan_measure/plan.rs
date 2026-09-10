use super::{PlanMeasureError, Walker, Work};
use crate::{DedupScope, Plan, PlanForm};

pub(super) fn visit_plan<'a>(
    walker: &mut Walker<'a>,
    plan: &'a Plan,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let Plan {
        branches,
        form,
        distinct: _,
        limit: _,
        offset: _,
        order,
        rust_group,
        dialect: _,
        dedup_scopes,
        construct_drops_some_branch_var: _,
    } = plan;
    super::model::push_branches(walker, branches, depth)?;
    walker.push(depth, Work::PlanForm(form))?;
    super::model::push_order_keys(walker, order, depth)?;
    if let Some(group) = rust_group {
        walker.push(depth, Work::RustGroup(group))?;
    }
    walker.collection(dedup_scopes.len())?;
    for scope in dedup_scopes {
        walker.checkpoint()?;
        if let Some(scope) = scope {
            walker.push(depth, Work::DedupScope(scope))?;
        }
    }
    Ok(())
}

pub(super) fn visit_plan_form<'a>(
    walker: &mut Walker<'a>,
    form: &'a PlanForm,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match form {
        PlanForm::Select { vars } => {
            walker.collection(vars.len())?;
            for var in vars {
                walker.payload(var.len())?;
            }
        }
        PlanForm::Construct { template } => {
            walker.collection(template.len())?;
            for triple in template {
                walker.push(depth, Work::TriplePattern(triple))?;
            }
        }
        PlanForm::Ask => {}
    }
    Ok(())
}

pub(super) fn visit_dedup_scope<'a>(
    walker: &mut Walker<'a>,
    scope: &'a DedupScope,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let DedupScope {
        group_id: _,
        key_bindings,
    } = scope;
    walker.collection(key_bindings.len())?;
    for (name, term) in key_bindings {
        walker.payload(name.len())?;
        walker.push(depth, Work::TermDef(term))?;
    }
    Ok(())
}
