//! One paid post-order classification pass; no alias inventory or tree copies.
use super::{retained_order_window, SourceSizedState};
use crate::build::control::BuildWork;
use crate::compiler_control::CompileContext;
use crate::iq::{Branch, SqlCond};
use crate::{CompilerWorkMode, Plan, PlanForm, Result};
use sf_core::query_control::QueryControl;

const STATES: [SourceSizedState; 5] = [
    SourceSizedState::GlobalOrder,
    SourceSizedState::RustGroup,
    SourceSizedState::ProjectedDistinct,
    SourceSizedState::TermDedup,
    SourceSizedState::ConstructDedup,
];

impl Plan {
    /// Classify reachable source-sized state under the request's shared control.
    /// Pays actual visits and logical output storage, with bounded recursive
    /// depth. This does not execute a source or admit physical allocator bytes.
    pub fn source_sized_states_with_order_window_and_work_control(
        &self,
        maximum_order_rows: usize,
        control: &dyn QueryControl,
    ) -> Result<Vec<SourceSizedState>> {
        let work = BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control)));
        let mut flags = [false; 5];
        collect(self, maximum_order_rows, &mut flags, work)?;
        work.charge(STATES.len())?;
        let mut output = work.vector(flags.iter().filter(|v| **v).count())?;
        for (state, present) in STATES.into_iter().zip(flags) {
            work.charge(1)?;
            if present {
                output.push(state);
            }
        }
        work.checkpoint()?;
        Ok(output)
    }
}

fn collect(
    plan: &Plan,
    maximum: usize,
    flags: &mut [bool; 5],
    work: BuildWork<'_>,
) -> Result<bool> {
    let work = work.enter()?;
    let mut source = false;
    for branch in &plan.branches {
        let branch_work = work.enter()?;
        // alias_sources() only sees core/optional scans and nested WHERE EXISTS
        // scans, all covered here without allocating its alias/name vector.
        let mut branch_source =
            branch.path.is_some() || !branch.core.is_empty() || !branch.opts.is_empty();
        if !branch_source {
            branch_source = conditions(&branch.where_conds, branch_work)?;
        }
        // Every nested plan contributes states even when the enclosing branch
        // already reads a source. Reuse its source result instead of rescanning.
        for subplan in &branch.subplan_joins {
            branch_work.charge(1)?;
            let nested_source = collect(&subplan.plan, 0, flags, branch_work)?;
            branch_source |= nested_source;
            if !branch_source {
                branch_source = conditions(&subplan.on, branch_work)?;
            }
        }
        source |= branch_source;
        if branch_source && term_dedup(branch, branch_work)? {
            flags[3] = true;
        }
    }
    for scope in &plan.dedup_scopes {
        work.charge(1)?;
        if scope.is_some() {
            flags[3] = true;
            break;
        }
    }
    // All predicates below read fixed-size fields, never source or term payload.
    work.charge(5)?;
    if source {
        flags[0] |= !plan.order.is_empty()
            && !matches!(plan.form, PlanForm::Ask)
            && retained_order_window(plan.offset, plan.limit).is_none_or(|n| n > maximum);
        flags[1] |= plan.rust_group.is_some();
        flags[2] |= plan.distinct
            && plan.branches.len() > 1
            && matches!(plan.form, PlanForm::Select { .. });
        flags[4] |= crate::exec_core::construct_may_need_cross_branch_dedup(plan);
    }
    work.checkpoint()?;
    Ok(source)
}

fn conditions(conds: &[SqlCond], work: BuildWork<'_>) -> Result<bool> {
    for cond in conds {
        work.charge(1)?;
        if condition(cond, work)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn condition(cond: &SqlCond, work: BuildWork<'_>) -> Result<bool> {
    let work = work.enter()?;
    Ok(match cond {
        SqlCond::PathExists { .. } => true,
        SqlCond::Exists { scans, conds } | SqlCond::NotExists { scans, conds } => {
            !scans.is_empty() || conditions(conds, work)?
        }
        SqlCond::Not(inner) => condition(inner, work)?,
        SqlCond::And(conds) | SqlCond::Or(conds) => conditions(conds, work)?,
        SqlCond::ExpressionError
        | SqlCond::LiteralCmp(..)
        | SqlCond::IriCmp(..)
        | SqlCond::ColEq(..)
        | SqlCond::NativeColEq(..)
        | SqlCond::NullSafeEq(..)
        | SqlCond::Cmp(..)
        | SqlCond::NativeCmp(..)
        | SqlCond::StrMatch { .. }
        | SqlCond::IsNotNull(..)
        | SqlCond::DecodedIsNotNull(..)
        | SqlCond::IsNull(..)
        | SqlCond::TemplateEq(..) => false,
    })
}

fn term_dedup(branch: &Branch, work: BuildWork<'_>) -> Result<bool> {
    work.charge(1)?;
    if !branch.distinct
        || branch.path.is_some()
        || branch.agg.is_some()
        || branch
            .core
            .len()
            .saturating_add(branch.opts.len())
            .saturating_add(branch.subplan_joins.len())
            > 1
    {
        return Ok(false);
    }
    let mut noninjective = false;
    for def in branch.bindings.values() {
        work.charge(1)?;
        let injective = crate::cascade::binding_injective_with_work(def, work)?;
        noninjective |= !injective;
        if !crate::cascade::binding_is_term_dedup_safe_with_injectivity(def, injective) {
            return Ok(false);
        }
    }
    Ok(noninjective)
}
