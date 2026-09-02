//! Request-budget boundary proofs over the backend-generic executor.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
use sf_core::NamedNode;
use sf_sql::{BranchStream, Dialect, RawTuple, SqlBackend};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

use crate::iq::{Branch, Scan, TermDef};
use crate::{Error, Plan, PlanForm};

use super::{ask_controlled, construct_each_async_controlled, select_each_async_controlled};

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
        source: LogicalSource::Table("items".to_owned()),
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

#[test]
fn zero_source_budget_rejects_before_metadata_or_branch_io() {
    let plan = plan(
        PlanForm::Select {
            vars: vec!["v".to_owned()],
        },
        vec![column_branch(0)],
    );
    let (mut backend, calls) = backend(vec![vec![row("one")]]);
    let budget = QueryBudget::new(QueryLimits::new(0, u64::MAX, u64::MAX));
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
    let budget = QueryBudget::new(QueryLimits::new(7, 0, u64::MAX));
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
    assert_eq!(budget.consumed(QueryCharge::SourceWork), 7);
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
    let budget = QueryBudget::new(QueryLimits::new(10, 2, u64::MAX));
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
    let budget = QueryBudget::new(QueryLimits::new(10, 1, u64::MAX));
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
        let budget = QueryBudget::new(QueryLimits::new(10, 1, u64::MAX));

        assert_eq!(
            super::block_on(ask_controlled(&plan, &mut backend, &budget)).unwrap(),
            expected
        );
        assert_eq!(budget.consumed(QueryCharge::ResultItems), 1);
    }
}
