use super::*;
use crate::compiler_control::CompileContext;
use crate::iq::{Aggregation, GroupKey, R2rmlGraphScope, Scan};
use sf_core::ir::{LogicalSource, Template};
use sf_core::query_control::{QueryBudget, QueryCharge, QueryControl, QueryLimits};
use std::sync::atomic::{AtomicUsize, Ordering};

fn budget(units: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX))
}

fn branch(alias: usize) -> Branch {
    let mut branch = Branch::single(Scan {
        alias,
        source: LogicalSource::Table("items".into()).into(),
    });
    branch.distinct = true;
    branch.agg = Some(Aggregation {
        keys: vec![GroupKey {
            var: "x".into(),
            cols: vec![
                ColRef::new(alias, "value"),
                ColRef::new(alias, "value"),
                ColRef::new(alias, "graph"),
            ],
        }],
        aggs: Vec::new(),
    });
    branch.bindings.insert(
        "x".into(),
        TermDef::Derived {
            term_map: TermMap::Column("value".into(), TermSpec::plain_literal()),
            alias,
        },
    );
    branch.bindings.insert(
        "blank".into(),
        TermDef::R2rmlBlank {
            term_map: TermMap::Template(
                Template::parse("blank/{value}").unwrap(),
                TermSpec::blank_node(),
            ),
            alias,
            graph: R2rmlGraphScope::Mapped {
                term_map: TermMap::Template(
                    Template::parse("http://ex/{graph}/{value}").unwrap(),
                    TermSpec::iri(),
                ),
                alias,
            },
        },
    );
    branch.bindings.insert(
        "unmapped".into(),
        TermDef::Derived {
            term_map: TermMap::Column("not_projected".into(), TermSpec::plain_literal()),
            alias,
        },
    );
    branch
}

#[test]
fn aggregate_finalization_matches_raw_and_publishes_only_after_funding() {
    for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
        let input = vec![branch(1)];
        let mut raw = input.clone();
        crate::cascade::dedup_before_aggregate(&mut raw, dialect);
        assert_eq!(
            format!("{:?}", raw[0].bindings["unmapped"]),
            format!("{:?}", input[0].bindings["unmapped"])
        );
        for key in ["x", "blank"] {
            assert_ne!(
                format!("{:?}", raw[0].bindings[key]),
                format!("{:?}", input[0].bindings[key])
            );
        }
        let measured = budget(u64::MAX);
        let mut output = input.clone();
        apply(
            &mut output,
            dialect,
            BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(&measured))),
        )
        .unwrap();
        assert_eq!(format!("{output:?}"), format!("{raw:?}"));
        let units = measured.consumed(QueryCharge::CompilerWork);
        for limit in 0..units {
            let mut output = input.clone();
            let control = budget(limit);
            assert!(matches!(
                apply(
                    &mut output,
                    dialect,
                    BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(&control)))
                ),
                Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
            ));
            assert_eq!(
                format!("{output:?}"),
                format!("{input:?}"),
                "publication at {limit}"
            );
        }
        let exact = budget(units);
        let mut output = input;
        apply(
            &mut output,
            dialect,
            BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(&exact))),
        )
        .unwrap();
        assert_eq!(format!("{output:?}"), format!("{raw:?}"));
    }
}

struct Stop {
    budget: QueryBudget,
    calls: AtomicUsize,
    stop: usize,
    cause: QueryControlError,
}

impl QueryControl for Stop {
    fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
        self.budget.checkpoint()
    }

    fn consume(
        &self,
        charge: QueryCharge,
        units: u64,
    ) -> std::result::Result<(), QueryControlError> {
        self.budget.consume(charge, units)?;
        if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.stop {
            self.budget.terminate(self.cause);
        }
        Ok(())
    }

    fn terminate(&self, cause: QueryControlError) -> QueryControlError {
        self.budget.terminate(cause)
    }
}

#[test]
fn aggregate_finalization_propagates_every_stop_before_publication() {
    let input = vec![branch(1)];
    let observer = Stop {
        budget: budget(u64::MAX),
        calls: AtomicUsize::new(0),
        stop: usize::MAX,
        cause: QueryControlError::Cancelled,
    };
    let run = |output: &mut [Branch], control: &dyn QueryControl| {
        apply(
            output,
            Dialect::Sqlite,
            BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
        )
    };
    run(&mut input.clone(), &observer).unwrap();
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for stop in 1..=observer.calls.load(Ordering::SeqCst) {
            let control = Stop {
                budget: budget(u64::MAX),
                calls: AtomicUsize::new(0),
                stop,
                cause,
            };
            let mut output = input.clone();
            assert!(
                matches!(run(&mut output, &control), Err(Error::QueryControl(actual)) if actual == cause)
            );
            assert_eq!(
                format!("{output:?}"),
                format!("{input:?}"),
                "publication at {stop}"
            );
            assert_eq!(control.checkpoint(), Err(cause));
        }
    }
}

#[test]
fn aggregate_alias_overflow_is_typed_before_mutation() {
    let mut branches = vec![branch(usize::MAX)];
    let before = format!("{branches:?}");
    let control = budget(u64::MAX);
    assert!(matches!(
        apply(
            &mut branches,
            Dialect::Sqlite,
            BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(&control)))
        ),
        Err(Error::QueryControl(QueryControlError::AccountingOverflow))
    ));
    assert_eq!(format!("{branches:?}"), before);
}
