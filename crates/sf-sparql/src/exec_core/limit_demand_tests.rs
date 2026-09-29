//! Counter-based proofs that unordered multi-branch LIMIT stops pulling and opening.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_core::{Literal, Term};
use sf_sql::{BranchStream, Dialect, RawTuple, SqlBackend};

use super::{ask, block_on, select};
use crate::iq::{Branch, OrderKey, Scan, TermDef};
use crate::{Plan, PlanForm};

#[derive(Default)]
struct Counters {
    probes: AtomicUsize,
    opens: AtomicUsize,
    pulls: AtomicUsize,
    hints: Mutex<Vec<bool>>,
}

// Opening any branch past the provisioned streams fails the run.
struct MockBackend {
    streams: VecDeque<Vec<RawTuple>>,
    provisioned: usize,
    counters: Arc<Counters>,
}

struct CountingStream {
    rows: std::vec::IntoIter<RawTuple>,
    counters: Arc<Counters>,
}

impl BranchStream for CountingStream {
    async fn next_row(&mut self) -> sf_sql::Result<Option<RawTuple>> {
        self.counters.pulls.fetch_add(1, Ordering::Relaxed);
        Ok(self.rows.next())
    }
}

impl SqlBackend for MockBackend {
    type Stream<'s>
        = CountingStream
    where
        Self: 's;

    async fn column_names(&mut self, _probe: &str) -> sf_sql::Result<Vec<String>> {
        self.counters.probes.fetch_add(1, Ordering::Relaxed);
        Ok(vec!["value".to_owned()])
    }

    async fn open_branch(
        &mut self,
        _sql: &str,
        _params: &[String],
    ) -> sf_sql::Result<CountingStream> {
        let index = self.counters.opens.fetch_add(1, Ordering::Relaxed);
        if index >= self.provisioned {
            return Err(sf_sql::Error::Introspection(
                "branch opened past its provisioned streams".to_owned(),
            ));
        }
        Ok(CountingStream {
            rows: self.streams.pop_front().unwrap_or_default().into_iter(),
            counters: Arc::clone(&self.counters),
        })
    }

    async fn open_branch_with_demand<'s>(
        &'s mut self,
        sql: &str,
        params: &[String],
        _metadata_sql: Option<&str>,
        _sqlite_character_keys: bool,
        _sqlite_lexical_keys: bool,
        early_stop: bool,
    ) -> sf_sql::Result<CountingStream> {
        self.counters.hints.lock().unwrap().push(early_stop);
        self.open_branch(sql, params).await
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

fn plan(branches: usize, limit: Option<usize>, offset: usize) -> Plan {
    Plan {
        branches: (0..branches).map(column_branch).collect(),
        form: PlanForm::Select {
            vars: vec!["v".to_owned()],
        },
        distinct: false,
        limit,
        offset,
        order: Vec::new(),
        rust_group: None,
        dialect: Dialect::Sqlite,
        dedup_scopes: Vec::new(),
        construct_drops_some_branch_var: false,
    }
}

fn mock(streams: Vec<Vec<&str>>) -> (MockBackend, Arc<Counters>) {
    let counters = Arc::new(Counters::default());
    let provisioned = streams.len();
    let streams: VecDeque<Vec<RawTuple>> = streams
        .into_iter()
        .map(|rows| rows.into_iter().map(row).collect::<Vec<RawTuple>>())
        .collect();
    let backend = MockBackend {
        streams,
        provisioned,
        counters: Arc::clone(&counters),
    };
    (backend, counters)
}

struct Observed {
    terms: Vec<Term>,
    probes: usize,
    opens: usize,
    pulls: usize,
    hints: Vec<bool>,
}

fn observe(plan: &Plan, streams: Vec<Vec<&str>>) -> Observed {
    let (mut backend, counters) = mock(streams);
    let solutions = block_on(select(plan, &mut backend)).unwrap();
    let terms: Vec<Term> = solutions.rows.into_iter().flatten().flatten().collect();
    let hints: Vec<bool> = counters.hints.lock().unwrap().clone();
    Observed {
        terms,
        probes: counters.probes.load(Ordering::Relaxed),
        opens: counters.opens.load(Ordering::Relaxed),
        pulls: counters.pulls.load(Ordering::Relaxed),
        hints,
    }
}

fn lits(values: &[&str]) -> Vec<Term> {
    values
        .iter()
        .map(|value| Term::Literal(Literal::new_simple_literal(*value)))
        .collect()
}

fn assert_seen(seen: &Observed, terms: &[&str], opens: usize, pulls: usize, early: bool) {
    assert_eq!(seen.terms, lits(terms));
    assert_eq!(seen.opens, opens);
    assert_eq!(seen.pulls, pulls);
    assert_eq!(seen.hints, vec![early; opens]);
}

fn order_by_v() -> Vec<OrderKey> {
    vec![OrderKey {
        var: "v".to_owned(),
        descending: false,
        expr: None,
    }]
}

#[test]
fn limit_one_stops_at_the_first_solution_without_opening_later_branches() {
    let plan = plan(3, Some(1), 0);
    let seen = observe(&plan, vec![vec!["a", "b", "c"]]);
    assert_seen(&seen, &["a"], 1, 1, true);
}

#[test]
fn limit_spanning_branches_stops_once_fulfilled() {
    let plan = plan(3, Some(3), 0);
    let seen = observe(&plan, vec![vec!["a", "b"], vec!["c", "d", "e"]]);
    // Branch 0: a, b, EOF. Branch 1: only c is demanded.
    assert_seen(&seen, &["a", "b", "c"], 2, 4, true);
}

#[test]
fn offset_spanning_branches_pulls_only_what_offset_and_limit_need() {
    let plan = plan(3, Some(2), 3);
    let seen = observe(&plan, vec![vec!["a", "b"], vec!["c", "d", "e", "f"]]);
    // Branch 0: a, b, EOF. Branch 1: c, d, e; f is never pulled.
    assert_seen(&seen, &["d", "e"], 2, 6, true);
}

#[test]
fn empty_first_branch_still_reaches_the_second() {
    let plan = plan(3, Some(1), 0);
    let seen = observe(&plan, vec![vec![], vec!["a", "b"]]);
    assert_seen(&seen, &["a"], 2, 2, true);
}

#[test]
fn bag_duplicates_count_toward_limit_without_distinct() {
    let plan = plan(2, Some(2), 0);
    let seen = observe(&plan, vec![vec!["a", "a", "b"]]);
    assert_seen(&seen, &["a", "a"], 1, 2, true);
}

#[test]
fn distinct_duplicates_do_not_count_and_keep_pulling() {
    let mut plan = plan(3, Some(3), 0);
    plan.distinct = true;
    let seen = observe(&plan, vec![vec!["a", "a"], vec!["a", "b", "b", "c", "d"]]);
    // Branch 0: a, a, EOF. Branch 1: a (dup), b, b (dup), c; d is never pulled.
    assert_seen(&seen, &["a", "b", "c"], 2, 7, true);
}

#[test]
fn limit_zero_touches_no_source() {
    let plan = plan(2, Some(0), 0);
    let seen = observe(&plan, Vec::new());
    assert_seen(&seen, &[], 0, 0, true);
    assert_eq!(seen.probes, 0);
}

#[test]
fn ask_stops_at_the_first_solution_across_branches() {
    let mut plan = plan(2, None, 0);
    plan.form = PlanForm::Ask;
    let (mut backend, counters) = mock(vec![vec!["a", "b"]]);
    assert!(block_on(ask(&plan, &mut backend)).unwrap());
    assert_eq!(counters.opens.load(Ordering::Relaxed), 1);
    assert_eq!(counters.pulls.load(Ordering::Relaxed), 1);
    assert_eq!(*counters.hints.lock().unwrap(), vec![true]);
}

#[test]
fn ordered_limit_still_scans_every_branch_without_demand_hint() {
    let mut plan = plan(2, Some(2), 0);
    plan.order = order_by_v();
    let seen = observe(&plan, vec![vec!["b", "a"], vec!["c"]]);
    // Branch 0: b, a, EOF. Branch 1: c, EOF.
    assert_seen(&seen, &["a", "b"], 2, 5, false);
}

#[test]
fn full_scan_reads_every_branch_to_eof_without_demand_hint() {
    let plan = plan(2, None, 0);
    let seen = observe(&plan, vec![vec!["a", "b"], vec!["c"]]);
    assert_seen(&seen, &["a", "b", "c"], 2, 5, false);
}

#[test]
fn single_branch_limit_stays_sql_sliced_without_demand_hint() {
    let plan = plan(1, Some(2), 0);
    let seen = observe(&plan, vec![vec!["a", "b"]]);
    assert_seen(&seen, &["a", "b"], 1, 3, false);
}
