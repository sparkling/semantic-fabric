//! Resource-state profile for compiled plans (ADR-0038 M1).
//!
//! The in-process executor remains a correctness oracle and still implements the
//! Rust fallbacks classified here. Production serving, however, cannot advertise
//! those paths as bounded: each one can retain state proportional to source rows
//! or distinct output. This module reports the exact predicates that currently
//! activate those fallbacks so `sf-serve` can reject them before execution.
//!
//! The profile is recursive because a derived-table [`crate::iq::SubPlanJoin`]
//! carries a complete nested [`Plan`]. The report itself is bounded by plan shape,
//! never source data, and is deterministically ordered and de-duplicated.

use std::collections::BTreeSet;
use std::fmt;

use crate::{Plan, PlanForm};

/// A currently implemented Rust fallback whose retained state can grow with source
/// cardinality. This is an execution-property report, not a capability claim: a
/// future bounded physical implementation should remove its corresponding state
/// only when the executor no longer takes that fallback.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SourceSizedState {
    /// `exec_core::run_branches` buffers every ordered solution before sorting.
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
    /// Return every currently source-sized Rust fallback reachable from this plan,
    /// including nested derived-table plans.
    ///
    /// The conditions deliberately share the executor's existing eligibility
    /// helpers for term and CONSTRUCT dedup. That keeps this admission boundary from
    /// growing a second, subtly different copy of those soundness predicates.
    pub fn source_sized_states(&self) -> Vec<SourceSizedState> {
        let mut states = BTreeSet::new();
        collect_states(self, &mut states);
        states.into_iter().collect()
    }
}

fn collect_states(plan: &Plan, states: &mut BTreeSet<SourceSizedState>) {
    let source_backed = plan_reads_source(plan);
    if source_backed && !plan.order.is_empty() && !matches!(plan.form, PlanForm::Ask) {
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
            collect_states(&subplan.plan, states);
        }
    }
}

fn plan_reads_source(plan: &Plan) -> bool {
    plan.branches.iter().any(branch_reads_source)
}

fn branch_reads_source(branch: &crate::iq::Branch) -> bool {
    branch.path.is_some()
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
        SqlCond::ColEq(..)
        | SqlCond::NullSafeEq(..)
        | SqlCond::Cmp(..)
        | SqlCond::StrMatch { .. }
        | SqlCond::IsNotNull(..)
        | SqlCond::IsNull(..)
        | SqlCond::TemplateEq(..) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::SourceSizedState;
    use crate::iq::{
        AggCol, AggKind, Aggregation, Branch, ColRef, OrderKey, RustGroup, Scan, SubPlanJoin,
        TermDef,
    };
    use crate::{DedupScope, Plan, PlanForm};
    use sf_core::ir::{LogicalSource, Template, TermMap, TermSpec};
    use sf_sql::Dialect;

    fn branch(alias: usize) -> Branch {
        Branch::single(Scan {
            alias,
            source: LogicalSource::Table("items".to_owned()),
        })
    }

    fn plan(branches: Vec<Branch>, form: PlanForm) -> Plan {
        Plan {
            branches,
            form,
            distinct: false,
            limit: None,
            offset: 0,
            order: Vec::new(),
            rust_group: None,
            dialect: Dialect::Sqlite,
            dedup_scopes: Vec::new(),
            construct_drops_some_branch_var: false,
        }
    }

    fn select_plan(branches: Vec<Branch>) -> Plan {
        plan(
            branches,
            PlanForm::Select {
                vars: vec!["value".to_owned()],
            },
        )
    }

    #[test]
    fn classifies_global_order() {
        let mut candidate = select_plan(vec![branch(1)]);
        candidate.order.push(OrderKey {
            var: "value".to_owned(),
            descending: false,
            expr: None,
        });

        assert_eq!(
            candidate.source_sized_states(),
            vec![SourceSizedState::GlobalOrder]
        );
    }

    #[test]
    fn ordered_ask_does_not_claim_the_unused_global_order_buffer() {
        let mut candidate = plan(vec![branch(1)], PlanForm::Ask);
        candidate.order.push(OrderKey {
            var: "value".to_owned(),
            descending: false,
            expr: None,
        });

        assert!(candidate.source_sized_states().is_empty());
    }

    #[test]
    fn ordered_ask_does_not_exempt_an_ordered_nested_select() {
        let mut nested = select_plan(vec![branch(2)]);
        nested.order.push(OrderKey {
            var: "value".to_owned(),
            descending: false,
            expr: None,
        });
        let mut outer_branch = Branch::empty();
        outer_branch.subplan_joins.push(SubPlanJoin {
            alias: 3,
            plan: Box::new(nested),
            on: Vec::new(),
            left: false,
        });
        let mut candidate = plan(vec![outer_branch], PlanForm::Ask);
        candidate.order.push(OrderKey {
            var: "value".to_owned(),
            descending: true,
            expr: None,
        });

        assert_eq!(
            candidate.source_sized_states(),
            vec![SourceSizedState::GlobalOrder]
        );
    }

    #[test]
    fn classifies_rust_group() {
        let mut candidate = select_plan(vec![branch(1)]);
        candidate.rust_group = Some(RustGroup {
            keys: vec!["value".to_owned()],
            aggs: Vec::new(),
            post_exprs: Vec::new(),
        });

        assert_eq!(
            candidate.source_sized_states(),
            vec![SourceSizedState::RustGroup]
        );
    }

    #[test]
    fn classifies_projected_distinct_only_for_multi_branch_select() {
        let mut candidate = select_plan(vec![branch(1), branch(2)]);
        candidate.distinct = true;
        assert_eq!(
            candidate.source_sized_states(),
            vec![SourceSizedState::ProjectedDistinct]
        );

        candidate.form = PlanForm::Ask;
        assert!(candidate.source_sized_states().is_empty());
    }

    #[test]
    fn classifies_eligible_per_branch_term_dedup() {
        let mut candidate_branch = branch(1);
        candidate_branch.distinct = true;
        candidate_branch.bindings.insert(
            "value".to_owned(),
            TermDef::Derived {
                term_map: TermMap::Template(
                    Template::parse("{left}{right}").expect("valid template"),
                    TermSpec::plain_literal(),
                ),
                alias: 1,
            },
        );
        let candidate = select_plan(vec![candidate_branch]);

        assert_eq!(
            candidate.source_sized_states(),
            vec![SourceSizedState::TermDedup]
        );
    }

    #[test]
    fn classifies_shared_term_dedup_group() {
        let mut candidate = select_plan(vec![branch(7)]);
        candidate.dedup_scopes = vec![Some(DedupScope {
            group_id: 99,
            key_bindings: candidate.branches[0].bindings.clone(),
        })];

        assert_eq!(
            candidate.source_sized_states(),
            vec![SourceSizedState::TermDedup]
        );
    }

    #[test]
    fn classifies_construct_dedup() {
        let mut candidate = plan(
            vec![branch(1), branch(2)],
            PlanForm::Construct {
                template: Vec::new(),
            },
        );
        candidate.construct_drops_some_branch_var = true;

        assert_eq!(
            candidate.source_sized_states(),
            vec![SourceSizedState::ConstructDedup]
        );
    }

    #[test]
    fn recurses_through_nested_subplans_and_deduplicates_state_kinds() {
        let mut nested = select_plan(vec![branch(2)]);
        nested.order.push(OrderKey {
            var: "value".to_owned(),
            descending: false,
            expr: None,
        });
        let mut outer_branch = branch(1);
        outer_branch.subplan_joins.push(SubPlanJoin {
            alias: 2,
            plan: Box::new(nested.clone()),
            on: Vec::new(),
            left: false,
        });
        outer_branch.subplan_joins.push(SubPlanJoin {
            alias: 3,
            plan: Box::new(nested),
            on: Vec::new(),
            left: false,
        });

        assert_eq!(
            select_plan(vec![outer_branch]).source_sized_states(),
            vec![SourceSizedState::GlobalOrder]
        );
    }

    #[test]
    fn source_pushed_branch_modifiers_are_safe_controls() {
        let mut safe_branch = branch(1);
        safe_branch.distinct = true;
        safe_branch.order.push(OrderKey {
            var: "value".to_owned(),
            descending: false,
            expr: None,
        });
        safe_branch.agg = Some(Aggregation {
            keys: Vec::new(),
            aggs: vec![AggCol {
                var: "count".to_owned(),
                kind: AggKind::Count,
                arg: None,
                distinct: false,
                out: ColRef::new(1, "count"),
                fixed_type: None,
            }],
        });

        assert!(select_plan(vec![safe_branch])
            .source_sized_states()
            .is_empty());
    }

    #[test]
    fn plain_streaming_plan_is_a_safe_control() {
        assert!(select_plan(vec![branch(1)])
            .source_sized_states()
            .is_empty());
    }

    #[test]
    fn source_free_blocking_states_are_plan_bounded_controls() {
        let mut candidate = select_plan(vec![Branch::empty(), Branch::empty()]);
        candidate.distinct = true;
        candidate.order.push(OrderKey {
            var: "value".to_owned(),
            descending: false,
            expr: None,
        });
        candidate.rust_group = Some(RustGroup {
            keys: Vec::new(),
            aggs: Vec::new(),
            post_exprs: Vec::new(),
        });

        assert!(candidate.source_sized_states().is_empty());
    }

    #[test]
    fn returns_all_reachable_state_kinds_in_stable_order() {
        let mut nested_construct = plan(
            vec![branch(20), branch(21)],
            PlanForm::Construct {
                template: Vec::new(),
            },
        );
        nested_construct.construct_drops_some_branch_var = true;

        let mut wrapper = branch(1);
        wrapper.subplan_joins.push(SubPlanJoin {
            alias: 20,
            plan: Box::new(nested_construct),
            on: Vec::new(),
            left: false,
        });

        let mut term_branch = branch(2);
        term_branch.distinct = true;
        term_branch.bindings.insert(
            "value".to_owned(),
            TermDef::Derived {
                term_map: TermMap::Template(
                    Template::parse("{left}{right}").expect("valid template"),
                    TermSpec::blank_node(),
                ),
                alias: 2,
            },
        );

        let mut candidate = select_plan(vec![wrapper, term_branch]);
        candidate.distinct = true;
        candidate.order.push(OrderKey {
            var: "value".to_owned(),
            descending: false,
            expr: None,
        });
        candidate.rust_group = Some(RustGroup {
            keys: Vec::new(),
            aggs: Vec::new(),
            post_exprs: Vec::new(),
        });

        assert_eq!(
            candidate.source_sized_states(),
            vec![
                SourceSizedState::GlobalOrder,
                SourceSizedState::RustGroup,
                SourceSizedState::ProjectedDistinct,
                SourceSizedState::TermDedup,
                SourceSizedState::ConstructDedup,
            ]
        );
    }
}
