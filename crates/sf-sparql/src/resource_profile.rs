//! Resource-state profile for compiled plans (ADR-0038 M1).
//!
//! Reports Rust fallbacks whose retained state can scale with source data, so
//! production serving can reject them before execution. A finite root ORDER BY
//! window is discharged only against an explicit retained-row ceiling; its
//! independent retained-payload budget is enforced during execution.

use std::collections::BTreeSet;
use std::fmt;

use crate::{Plan, PlanForm};

/// An implemented fallback whose retained state can grow with source cardinality.
/// This execution-property report is not itself a capability claim.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SourceSizedState {
    /// An ordered result without an admitted finite root window.
    GlobalOrder,
    /// `exec_core::rust_group_execute` collects every inner solution before grouping.
    RustGroup,
    /// Multi-branch SELECT DISTINCT keeps projected solution tuples in a set.
    ProjectedDistinct,
    /// Per-branch or shared-group reconstructed-term dedup keeps term tuples in a set.
    TermDedup,
    /// Cross-branch CONSTRUCT dedup keeps produced triples in a set.
    ConstructDedup,
}

impl SourceSizedState {
    /// Stable, low-cardinality code suitable for a typed rejection response.
    pub const fn code(self) -> &'static str {
        match self {
            Self::GlobalOrder => "global-order",
            Self::RustGroup => "rust-group",
            Self::ProjectedDistinct => "projected-distinct",
            Self::TermDedup => "term-dedup",
            Self::ConstructDedup => "construct-dedup",
        }
    }
}

impl fmt::Display for SourceSizedState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl Plan {
    /// Return every source-sized fallback reachable from this plan. Conditions
    /// reuse executor eligibility helpers rather than duplicating them.
    pub fn source_sized_states(&self) -> Vec<SourceSizedState> {
        self.source_sized_states_with_order_window(0)
    }

    /// Return source-sized fallbacks after admitting an exact ordered result
    /// window of at most `maximum_order_rows` retained solutions.
    pub fn source_sized_states_with_order_window(
        &self,
        maximum_order_rows: usize,
    ) -> Vec<SourceSizedState> {
        let mut states = BTreeSet::new();
        collect_states(self, maximum_order_rows, &mut states);
        states.into_iter().collect()
    }
}

/// Number of sorted solutions that can influence `OFFSET` followed by `LIMIT`.
/// `LIMIT 0` retains no row regardless of the offset; overflow is unbounded.
pub(crate) fn retained_order_window(offset: usize, limit: Option<usize>) -> Option<usize> {
    match limit {
        Some(0) => Some(0),
        Some(limit) => offset.checked_add(limit),
        None => None,
    }
}

fn collect_states(plan: &Plan, maximum_order_rows: usize, states: &mut BTreeSet<SourceSizedState>) {
    let source_backed = plan_reads_source(plan);
    let order_is_bounded = retained_order_window(plan.offset, plan.limit)
        .is_some_and(|window| window <= maximum_order_rows);
    if source_backed
        && !plan.order.is_empty()
        && !matches!(plan.form, PlanForm::Ask)
        && !order_is_bounded
    {
        states.insert(SourceSizedState::GlobalOrder);
    }
    if source_backed && plan.rust_group.is_some() {
        states.insert(SourceSizedState::RustGroup);
    }
    if source_backed
        && plan.distinct
        && plan.branches.len() > 1
        && matches!(plan.form, PlanForm::Select { .. })
    {
        states.insert(SourceSizedState::ProjectedDistinct);
    }
    if plan.dedup_scopes.iter().any(Option::is_some)
        || plan.branches.iter().any(|branch| {
            if !branch_reads_source(branch) {
                return false;
            }
            crate::cascade::eligible_for_term_dedup(branch)
        })
    {
        states.insert(SourceSizedState::TermDedup);
    }
    if source_backed && crate::exec_core::construct_may_need_cross_branch_dedup(plan) {
        states.insert(SourceSizedState::ConstructDedup);
    }

    for branch in &plan.branches {
        for subplan in &branch.subplan_joins {
            // Nested ordered plans remain independently closed.
            collect_states(&subplan.plan, 0, states);
        }
    }
}

fn plan_reads_source(plan: &Plan) -> bool {
    plan.branches.iter().any(branch_reads_source)
}

fn branch_reads_source(branch: &crate::iq::Branch) -> bool {
    branch.path.is_some()
        || !branch.core.is_empty()
        || !branch.opts.is_empty()
        || !branch.alias_sources().is_empty()
        || branch.where_conds.iter().any(condition_reads_source)
        || branch
            .opts
            .iter()
            .any(|opt| opt.on.iter().chain(&opt.extra).any(condition_reads_source))
        || branch.subplan_joins.iter().any(|subplan| {
            plan_reads_source(&subplan.plan) || subplan.on.iter().any(condition_reads_source)
        })
}

fn condition_reads_source(condition: &crate::iq::SqlCond) -> bool {
    use crate::iq::SqlCond;

    match condition {
        SqlCond::PathExists { .. } => true,
        SqlCond::NotExists { scans, conds } | SqlCond::Exists { scans, conds } => {
            !scans.is_empty() || conds.iter().any(condition_reads_source)
        }
        SqlCond::Not(inner) => condition_reads_source(inner),
        SqlCond::And(conditions) | SqlCond::Or(conditions) => {
            conditions.iter().any(condition_reads_source)
        }
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
    }
}

#[cfg(test)]
#[path = "resource_profile_control_tests.rs"]
mod control_tests;
#[path = "resource_profile_control.rs"]
mod controlled;
#[cfg(test)]
#[path = "resource_profile_tests.rs"]
mod tests;
