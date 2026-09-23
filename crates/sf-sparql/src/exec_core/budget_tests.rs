//! Request-budget boundary proofs over the backend-generic executor.
use super::row::Bindings;
use super::{ask_controlled, construct_each_async_controlled, select_each_async_controlled};
use crate::iq::{Branch, OrderKey, RustGroup, Scan, TermDef};
use crate::{Error, Plan, PlanForm};
use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
use sf_core::{Literal, NamedNode, Term};
use sf_sql::{BranchStream, Dialect, RawTuple, SqlBackend};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
#[derive(Default)]
struct Calls {
    probes: AtomicUsize,
    opens: AtomicUsize,
    pulls: AtomicUsize,
}

struct BudgetBackend {
    streams: VecDeque<Vec<RawTuple>>,
    calls: Arc<Calls>,
}

struct BudgetStream {
    rows: std::vec::IntoIter<RawTuple>,
    calls: Arc<Calls>,
}

impl BranchStream for BudgetStream {
    async fn next_row(&mut self) -> sf_sql::Result<Option<RawTuple>> {
        panic!("controlled executor must dispatch next_row_controlled")
    }

    async fn next_row_controlled(
        &mut self,
        control: &dyn sf_core::query_control::QueryControl,
    ) -> sf_sql::Result<Option<RawTuple>> {
        control.checkpoint()?;
        self.calls.pulls.fetch_add(1, Ordering::Relaxed);
        Ok(self.rows.next())
    }
}

impl SqlBackend for BudgetBackend {
    type Stream<'s>
        = BudgetStream
    where
        Self: 's;

    async fn column_names(&mut self, _probe: &str) -> sf_sql::Result<Vec<String>> {
        self.calls.probes.fetch_add(1, Ordering::Relaxed);
        Ok(vec!["value".to_owned()])
    }

    async fn open_branch(
        &mut self,
        _sql: &str,
        _params: &[String],
    ) -> sf_sql::Result<BudgetStream> {
        self.calls.opens.fetch_add(1, Ordering::Relaxed);
        Ok(BudgetStream {
            rows: self.streams.pop_front().unwrap_or_default().into_iter(),
            calls: Arc::clone(&self.calls),
        })
    }
}

fn row(value: &str) -> RawTuple {
    RawTuple {
        values: vec![Some(value.to_owned())],
        codes: vec![None],
    }
}

fn column_branch(alias: usize) -> Branch {
    let mut branch = Branch::single(Scan {
        alias,
        source: (LogicalSource::Table("items".to_owned())).into(),
    });
    branch.bindings.insert(
        "v".to_owned(),
        TermDef::Derived {
            term_map: TermMap::Column("value".into(), TermSpec::plain_literal()),
            alias,
        },
    );
    branch
}

fn plan(form: PlanForm, branches: Vec<Branch>) -> Plan {
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

fn backend(streams: Vec<Vec<RawTuple>>) -> (BudgetBackend, Arc<Calls>) {
    let calls = Arc::new(Calls::default());
    (
        BudgetBackend {
            streams: streams.into(),
            calls: Arc::clone(&calls),
        },
        calls,
    )
}

fn assert_calls(calls: &Calls, probes: usize, opens: usize, pulls: usize) {
    assert_eq!(calls.probes.load(Ordering::Relaxed), probes);
    assert_eq!(calls.opens.load(Ordering::Relaxed), opens);
    assert_eq!(calls.pulls.load(Ordering::Relaxed), pulls);
}

// Pay preparation independently to isolate cursor/result boundaries.
fn preparation_work(plan: &Plan) -> u64 {
    let control = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    let work = sf_sql::source_work::SourceWork::new(Some(&control));
    let branches = &plan.branches;
    let mut seen = crate::emit::SourceSet::default();
    let mut catalog = crate::emit::ColumnCatalog::default();
    for source in crate::emit::live_metadata_sources_controlled(branches, &control).unwrap() {
        if seen.insert(source, &control).unwrap() {
            crate::emit::source_probe_controlled(source, plan.dialect, &control).unwrap();
            catalog
                .insert_live_result_controlled(
                    source,
                    vec![sf_sql::backend::ResultColumn {
                        name: "value".into(),
                        natural_datatype: None,
                        native_scalar: None,
                        text_key: None,
                        sqlite_decode: None,
                    }],
                    &control,
                )
                .unwrap();
        }
    }
    crate::emit::validate_live_columns_controlled(branches, plan.dialect, &catalog, work).unwrap();
    let grouping = plan.rust_group.is_some();
    for branch in branches {
        let emitted = crate::emit::emit_branch_controlled(
            branch,
            plan.dialect,
            &catalog,
            crate::emit::BranchModifiers::prepared(
                branch,
                branches.len() == 1,
                !grouping && plan.distinct,
                grouping || plan.order.is_empty(),
                if grouping { None } else { plan.limit },
                if grouping { 0 } else { plan.offset },
            ),
            work,
        )
        .unwrap();
        super::row::build_col_index_controlled(&emitted.projection, work).unwrap();
        super::row::intern_bindings_controlled(branch, work).unwrap();
    }
    control.consumed(QueryCharge::SourceWork)
}

/// The SourceWork ONE reconstructed row of `column_branch`'s single `?v`
/// binding costs, measured against an unbounded control rather than restated as
/// a literal: `reconstruct_controlled` charges a per-row checkpoint, one unit
/// per candidate binding, and the generated term's own width. The tests below
/// pin `preparation_work + pulls + rows * reconstruction_work(..)`, so they
/// stay exact boundaries while remaining readable as "setup, pulls, rows".
fn reconstruction_work(plan: &Plan, alias: usize, value: &str) -> u64 {
    let control = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    let branch = &plan.branches[0];
    let work = sf_sql::source_work::SourceWork::new(Some(&control));
    let interned = super::row::intern_bindings_controlled(branch, work).unwrap();
    let before = control.consumed(QueryCharge::SourceWork);
    let schema = vec![crate::iq::ColRef::new(alias, "value")];
    let col_index = super::row::build_col_index(&schema);
    let tuple = row(value);
    let raw = super::row::RawRow {
        values: &tuple.values,
        codes: &tuple.codes,
        index: &col_index,
    };
    super::row::reconstruct_controlled(
        &interned,
        &raw,
        sf_core::term_work::TermWork::new(Some(&control)),
    )
    .unwrap();
    control.consumed(QueryCharge::SourceWork) - before
}

#[test]
fn zero_source_budget_rejects_before_metadata_or_branch_io() {
    let plan = plan(
        PlanForm::Select {
            vars: vec!["v".to_owned()],
        },
        vec![column_branch(0)],
    );
    let (mut backend, calls) = backend(vec![vec![row("one")]]);
    let budget = QueryBudget::new(QueryLimits::new(u64::MAX, 0, u64::MAX, u64::MAX));
    let sinks = AtomicUsize::new(0);

    let error = super::block_on(select_each_async_controlled(
        &plan,
        &mut backend,
        &budget,
        |_| {
            sinks.fetch_add(1, Ordering::Relaxed);
            std::future::ready(Ok::<(), Error>(()))
        },
    ))
    .unwrap_err();

    assert!(matches!(
        error,
        Error::QueryControl(QueryControlError::SourceWorkExceeded)
    ));
    assert_calls(&calls, 0, 0, 0);
    assert_eq!(sinks.load(Ordering::Relaxed), 0);
    assert_eq!(budget.consumed(QueryCharge::SourceWork), 0);
}

#[test]
fn offset_discarded_rows_charge_every_pull_attempt() {
    let mut plan = plan(
        PlanForm::Select {
            vars: vec!["v".to_owned()],
        },
        vec![column_branch(0), column_branch(1)],
    );
    plan.offset = usize::MAX;
    let (mut backend, calls) = backend(vec![vec![row("one"), row("two")], Vec::new()]);
    // Both rows are reconstructed before OFFSET discards them (reconstruction
    // precedes slicing), so their term-generation work is charged too.
    let source = preparation_work(&plan)
        + 7
        + reconstruction_work(&plan, 0, "one")
        + reconstruction_work(&plan, 0, "two");
    let budget = QueryBudget::new(QueryLimits::new(u64::MAX, source, 0, u64::MAX));
    let sinks = AtomicUsize::new(0);

    super::block_on(select_each_async_controlled(
        &plan,
        &mut backend,
        &budget,
        |_| {
            sinks.fetch_add(1, Ordering::Relaxed);
            std::future::ready(Ok::<(), Error>(()))
        },
    ))
    .unwrap();

    assert_calls(&calls, 1, 2, 4);
    assert_eq!(budget.consumed(QueryCharge::SourceWork), source);
    assert_eq!(budget.consumed(QueryCharge::ResultItems), 0);
    assert_eq!(sinks.load(Ordering::Relaxed), 0);
}

#[test]
fn select_result_budget_rejects_before_over_limit_sink() {
    let plan = plan(
        PlanForm::Select {
            vars: vec!["v".to_owned()],
        },
        vec![column_branch(0)],
    );
    let (mut backend, _) = backend(vec![vec![row("one"), row("two"), row("three")]]);
    // The whole first batch is reconstructed before the per-row sink runs, so
    // all three rows' term work is charged even though the result-item limit
    // cuts the sink at two.
    let budget = QueryBudget::new(QueryLimits::new(
        u64::MAX,
        preparation_work(&plan)
            + 10
            + reconstruction_work(&plan, 0, "one")
            + reconstruction_work(&plan, 0, "two")
            + reconstruction_work(&plan, 0, "three"),
        2,
        u64::MAX,
    ));
    let sinks = AtomicUsize::new(0);

    let error = super::block_on(select_each_async_controlled(
        &plan,
        &mut backend,
        &budget,
        |_| {
            sinks.fetch_add(1, Ordering::Relaxed);
            std::future::ready(Ok::<(), Error>(()))
        },
    ))
    .unwrap_err();

    assert!(matches!(
        error,
        Error::QueryControl(QueryControlError::ResultItemsExceeded)
    ));
    assert_eq!(sinks.load(Ordering::Relaxed), 2);
    assert_eq!(budget.consumed(QueryCharge::ResultItems), 2);
}

#[test]
fn ordered_retained_payload_has_an_exact_typed_high_water_limit() {
    let mut plan = plan(
        PlanForm::Select {
            vars: vec!["v".to_owned()],
        },
        vec![column_branch(0)],
    );
    plan.order.push(OrderKey {
        var: "v".to_owned(),
        descending: false,
        expr: None,
    });
    plan.limit = Some(1);
    let mut expected = Bindings::new();
    expected.insert(
        Arc::from("v"),
        Term::Literal(Literal::new_simple_literal("payload")),
    );
    let exact = expected.retained_payload_bytes().unwrap();

    let (mut exact_backend, _) = backend(vec![vec![row("payload")]]);
    let limits = QueryLimits::new(
        u64::MAX,
        preparation_work(&plan) + 10 + reconstruction_work(&plan, 0, "payload"),
        1,
        u64::MAX,
    )
    .with_max_retained_bytes(exact);
    let budget = QueryBudget::new(limits);
    super::block_on(select_each_async_controlled(
        &plan,
        &mut exact_backend,
        &budget,
        |_| std::future::ready(Ok::<(), Error>(())),
    ))
    .unwrap();
    assert_eq!(budget.consumed(QueryCharge::RetainedBytes), exact);

    let (mut backend, _) = backend(vec![vec![row("payload")]]);
    // Source work must still admit the row's reconstruction, so the retained
    // limit is what this half proves, not an incidental source refusal.
    let limits = QueryLimits::new(
        u64::MAX,
        preparation_work(&plan) + 10 + reconstruction_work(&plan, 0, "payload"),
        1,
        u64::MAX,
    )
    .with_max_retained_bytes(exact - 1);
    let budget = QueryBudget::new(limits);
    let error = super::block_on(select_each_async_controlled(
        &plan,
        &mut backend,
        &budget,
        |_| std::future::ready(Ok::<(), Error>(())),
    ))
    .unwrap_err();

    assert!(matches!(
        error,
        Error::QueryControl(QueryControlError::RetainedBytesExceeded)
    ));
    assert_eq!(budget.consumed(QueryCharge::RetainedBytes), 0);
}

fn constant_triple(predicate: &str) -> TriplePattern {
    TriplePattern {
        subject: TermPattern::NamedNode(NamedNode::new_unchecked("urn:subject")),
        predicate: NamedNodePattern::NamedNode(NamedNode::new_unchecked(predicate)),
        object: TermPattern::NamedNode(NamedNode::new_unchecked("urn:object")),
    }
}

#[test]
fn construct_budget_counts_triples_not_solutions() {
    let plan = plan(
        PlanForm::Construct {
            template: vec![constant_triple("urn:p1"), constant_triple("urn:p2")],
        },
        vec![column_branch(0)],
    );
    let (mut backend, _) = backend(vec![vec![row("one")]]);
    let budget = QueryBudget::new(QueryLimits::new(
        u64::MAX,
        preparation_work(&plan) + 10,
        1,
        u64::MAX,
    ));
    let sinks = AtomicUsize::new(0);

    let error = super::block_on(construct_each_async_controlled(
        &plan,
        &mut backend,
        &budget,
        |_| {
            sinks.fetch_add(1, Ordering::Relaxed);
            std::future::ready(Ok::<(), Error>(()))
        },
    ))
    .unwrap_err();

    assert!(matches!(
        error,
        Error::QueryControl(QueryControlError::ResultItemsExceeded)
    ));
    assert_eq!(sinks.load(Ordering::Relaxed), 0);
    assert_eq!(budget.consumed(QueryCharge::ResultItems), 0);
}

#[test]
fn ask_charges_exactly_one_boolean_for_true_and_false() {
    for (rows, expected) in [(vec![row("one"), row("two")], true), (Vec::new(), false)] {
        let plan = plan(PlanForm::Ask, vec![column_branch(0)]);
        let (mut backend, _) = backend(vec![rows]);
        let budget = QueryBudget::new(QueryLimits::new(
            u64::MAX,
            preparation_work(&plan) + 10,
            1,
            u64::MAX,
        ));

        assert_eq!(
            super::block_on(ask_controlled(&plan, &mut backend, &budget)).unwrap(),
            expected
        );
        assert_eq!(budget.consumed(QueryCharge::ResultItems), 1);
    }
}

#[test]
fn ask_stops_after_the_first_solution_and_its_pull() {
    let plan = plan(PlanForm::Ask, vec![column_branch(0)]);
    let (mut backend, calls) = backend(vec![vec![row("one"), row("two"), row("three")]]);
    // ASK pulls one row at a time and stops after the first solution, so
    // exactly one row is ever reconstructed.
    let source = preparation_work(&plan) + 3 + reconstruction_work(&plan, 0, "one");
    let budget = QueryBudget::new(QueryLimits::new(u64::MAX, source, 1, u64::MAX));

    assert!(super::block_on(ask_controlled(&plan, &mut backend, &budget)).unwrap());
    assert_calls(&calls, 1, 1, 1);
    assert_eq!(budget.consumed(QueryCharge::SourceWork), source);
    assert_eq!(budget.consumed(QueryCharge::ResultItems), 1);
}

#[test]
fn ask_does_not_truncate_the_inner_rows_of_rust_group() {
    let mut plan = plan(PlanForm::Ask, vec![column_branch(0)]);
    plan.offset = 1;
    plan.rust_group = Some(RustGroup {
        keys: vec!["v".to_owned()],
        aggs: Vec::new(),
        post_exprs: Vec::new(),
    });
    let (mut backend, calls) = backend(vec![vec![row("one"), row("two")]]);
    // The grouped inner collection reconstructs every inner row before the
    // outer ASK answers, so both rows' term work is charged.
    let expected = preparation_work(&plan)
        + 5
        + reconstruction_work(&plan, 0, "one")
        + reconstruction_work(&plan, 0, "two");
    let budget = QueryBudget::new(QueryLimits::new(u64::MAX, expected, 1, u64::MAX));

    assert!(super::block_on(ask_controlled(&plan, &mut backend, &budget)).unwrap());
    assert_calls(&calls, 1, 1, 3);
    assert_eq!(budget.consumed(QueryCharge::SourceWork), expected);
}

#[test]
fn ask_ignores_order_but_still_applies_a_single_branch_offset() {
    let mut plan = plan(PlanForm::Ask, vec![column_branch(0)]);
    plan.order = vec![OrderKey {
        var: "v".to_owned(),
        descending: false,
        expr: None,
    }];
    plan.offset = 3;
    let (mut backend, calls) = backend(vec![vec![row("one"), row("two")]]);
    // Both rows are reconstructed before the offset discards them and the
    // stream ends without a solution.
    let budget = QueryBudget::new(QueryLimits::new(
        u64::MAX,
        preparation_work(&plan)
            + 5
            + reconstruction_work(&plan, 0, "one")
            + reconstruction_work(&plan, 0, "two"),
        1,
        u64::MAX,
    ));

    assert!(!super::block_on(ask_controlled(&plan, &mut backend, &budget)).unwrap());
    assert_calls(&calls, 1, 1, 3);
}

#[test]
fn ordered_ask_stops_after_the_first_post_offset_solution() {
    let mut plan = plan(PlanForm::Ask, vec![column_branch(0)]);
    plan.order = vec![OrderKey {
        var: "v".to_owned(),
        descending: true,
        expr: None,
    }];
    plan.offset = 1;
    let (mut backend, calls) = backend(vec![vec![row("one"), row("two"), row("three")]]);
    // Two rows are reconstructed: the one the offset discards, and the first
    // post-offset solution the ASK stops on.
    let budget = QueryBudget::new(QueryLimits::new(
        u64::MAX,
        preparation_work(&plan)
            + 4
            + reconstruction_work(&plan, 0, "one")
            + reconstruction_work(&plan, 0, "two"),
        1,
        u64::MAX,
    ));

    assert!(super::block_on(ask_controlled(&plan, &mut backend, &budget)).unwrap());
    assert_calls(&calls, 1, 1, 2);
}

#[test]
fn ordered_ask_limit_zero_emits_no_solution() {
    let mut plan = plan(PlanForm::Ask, vec![column_branch(0)]);
    plan.order = vec![OrderKey {
        var: "v".to_owned(),
        descending: false,
        expr: None,
    }];
    plan.limit = Some(0);
    let (mut backend, calls) = backend(vec![vec![row("one")]]);
    let budget = QueryBudget::new(QueryLimits::new(u64::MAX, 3, 1, u64::MAX));

    assert!(!super::block_on(ask_controlled(&plan, &mut backend, &budget)).unwrap());
    assert_calls(&calls, 0, 0, 0);
}

#[test]
fn zero_ask_result_budget_rejects_before_source_io() {
    let plan = plan(PlanForm::Ask, vec![column_branch(0)]);
    let (mut backend, calls) = backend(vec![vec![row("one")]]);
    let budget = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, 0, u64::MAX));

    let error = super::block_on(ask_controlled(&plan, &mut backend, &budget)).unwrap_err();

    assert!(matches!(
        error,
        Error::QueryControl(QueryControlError::ResultItemsExceeded)
    ));
    assert_calls(&calls, 0, 0, 0);
    assert_eq!(budget.consumed(QueryCharge::ResultItems), 0);
}
