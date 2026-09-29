use super::*;
use crate::emit::ColumnCatalog;
use crate::iq::{ColRef, Scan, SqlCond, TermDef};
use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_core::query_control::{QueryBudget, QueryLimits};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

struct Probe {
    budget: QueryBudget,
    calls: AtomicUsize,
    at: usize,
    cause: QueryControlError,
    log: Mutex<Vec<u64>>,
}

impl Probe {
    fn new(source_units: u64, at: usize, cause: QueryControlError) -> Self {
        Self {
            budget: QueryBudget::new(QueryLimits::new(u64::MAX, source_units, u64::MAX, u64::MAX)),
            calls: AtomicUsize::new(0),
            at,
            cause,
            log: Mutex::new(Vec::new()),
        }
    }
}

impl QueryControl for Probe {
    fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
        self.budget.checkpoint()
    }
    fn consume(&self, kind: QueryCharge, units: u64) -> std::result::Result<(), QueryControlError> {
        self.budget.consume(kind, units)?;
        if kind == QueryCharge::SourceWork {
            self.log.lock().unwrap().push(units);
        }
        if self.calls.fetch_add(1, Ordering::Relaxed) + 1 == self.at {
            self.budget.terminate(self.cause);
        }
        Ok(())
    }
    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.budget.terminate(reason)
    }
}

fn branch() -> Branch {
    let mut branch = Branch::empty();
    branch.core = ["a", "b"]
        .into_iter()
        .enumerate()
        .map(|(alias, name)| Scan {
            alias,
            source: LogicalSource::Table(name.into()).into(),
        })
        .collect();
    branch.where_conds.push(SqlCond::NativeColEq(
        ColRef::new(0, Box::<str>::from("key")),
        ColRef::new(1, Box::<str>::from("key")),
    ));
    branch.bindings.insert(
        "x".into(),
        TermDef::Derived {
            alias: 0,
            term_map: TermMap::Column("key".into(), TermSpec::plain_literal()),
        },
    );
    branch
}

fn render(control: Option<&dyn QueryControl>) -> Result<(String, Vec<String>)> {
    let (mut params, mut pidx) = (Vec::new(), 1);
    let sql = super::super::sql(
        &branch(),
        &[ColRef::new(0, Box::<str>::from("key"))],
        Dialect::Postgres,
        &ColumnCatalog::default(),
        &mut params,
        &mut pidx,
        SourceWork::new(control),
    )?;
    Ok((sql, params))
}

/// Exact/N-1, and cancel/deadline at every charge with the sticky cause.
/// Returns the SourceWork charge sequence of the uninterrupted run.
fn assert_boundaries(run: impl Fn(&dyn QueryControl) -> Result<()>) -> Vec<u64> {
    let never = usize::MAX;
    let counted = Probe::new(u64::MAX, never, QueryControlError::Cancelled);
    run(&counted).unwrap();
    let total = counted.budget.consumed(QueryCharge::SourceWork);
    assert!(total > 0);
    assert_eq!(counted.budget.consumed(QueryCharge::CompilerWork), 0);
    run(&Probe::new(total, never, QueryControlError::Cancelled)).unwrap();
    assert!(matches!(
        run(&Probe::new(total - 1, never, QueryControlError::Cancelled)),
        Err(crate::Error::QueryControl(
            QueryControlError::SourceWorkExceeded
        ))
    ));
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for at in 1..=counted.calls.load(Ordering::Relaxed) {
            assert!(
                matches!(run(&Probe::new(u64::MAX, at, cause)),
                    Err(crate::Error::QueryControl(actual)) if actual == cause),
                "charge {at}"
            );
        }
    }
    counted.log.into_inner().unwrap()
}

#[test]
fn legacy_sql_matches_raw_and_charges_no_compiler_work() {
    let (raw, raw_params) = render(None).unwrap();
    assert!(
        raw.contains("DISTINCT") && !raw.contains("ROW_NUMBER"),
        "{raw}"
    );
    let budget = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    assert_eq!(render(Some(&budget)).unwrap(), (raw, raw_params));
    assert_eq!(budget.consumed(QueryCharge::CompilerWork), 0);
    assert!(budget.consumed(QueryCharge::SourceWork) > 0);
}

#[test]
fn controlled_d1_has_exact_boundaries() {
    let charges = assert_boundaries(|control| {
        force_distinct(
            &mut branch(),
            Dialect::Postgres,
            SourceWork::new(Some(control)),
        )
    });
    assert!(charges.len() > 4);
}

#[test]
fn production_callsite_pays_d1_after_the_clone_copy() {
    let d1 = assert_boundaries(|control| {
        force_distinct(
            &mut branch(),
            Dialect::Postgres,
            SourceWork::new(Some(control)),
        )
    });
    let copy = Probe::new(u64::MAX, usize::MAX, QueryControlError::Cancelled);
    super::super::metadata_source::branch_copy_controlled(&branch(), SourceWork::new(Some(&copy)))
        .unwrap();
    let expected = [copy.log.into_inner().unwrap(), d1].concat();
    let sql = assert_boundaries(|control| render(Some(control)).map(|_| ()));
    // A reverted uncontrolled dispatch drops the D1 charges that follow the copy.
    assert!(
        sql.windows(expected.len())
            .any(|window| window == expected.as_slice()),
        "D1 charges missing from ref_atom::sql: {expected:?} not in {sql:?}"
    );
}
