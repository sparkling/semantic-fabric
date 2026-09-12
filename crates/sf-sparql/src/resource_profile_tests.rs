use super::SourceSizedState;
use crate::iq::{
    AggCol, AggKind, Aggregation, Branch, ColRef, OrderKey, RustGroup, Scan, SubPlanJoin, TermDef,
};
use crate::{DedupScope, Plan, PlanForm};
use sf_core::ir::{LogicalSource, Template, TermMap, TermSpec};
use sf_sql::Dialect;

fn branch(alias: usize) -> Branch {
    Branch::single(Scan {
        alias,
        source: (LogicalSource::Table("items".to_owned())).into(),
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
fn finite_order_window_has_exact_zero_cap_and_overflow_boundaries() {
    let mut candidate = select_plan(vec![branch(1)]);
    candidate.order.push(OrderKey {
        var: "value".to_owned(),
        descending: false,
        expr: None,
    });
    candidate.offset = 3;
    candidate.limit = Some(2);
    assert!(candidate
        .source_sized_states_with_order_window(5)
        .is_empty());
    assert_eq!(
        candidate.source_sized_states_with_order_window(4),
        vec![SourceSizedState::GlobalOrder]
    );

    candidate.offset = usize::MAX;
    candidate.limit = Some(0);
    assert!(candidate
        .source_sized_states_with_order_window(0)
        .is_empty());
    candidate.limit = Some(1);
    assert_eq!(
        candidate.source_sized_states_with_order_window(usize::MAX),
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
fn construct_distinct_is_not_a_graph_set_fallback_marker() {
    let mut candidate = plan(
        vec![branch(1), branch(2)],
        PlanForm::Construct {
            template: Vec::new(),
        },
    );
    candidate.distinct = true;

    assert!(candidate.source_sized_states().is_empty());
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
