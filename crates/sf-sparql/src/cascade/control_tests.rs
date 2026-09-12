use super::*;
use crate::{compiler_control::CompileContext, CompilerWorkMode};
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn same_terms_preserves_distinct_coverage_and_stops_at_every_charge() {
    use super::super::{sameterm, CascadeCtx, LogicalSource, Scan, TermDef};
    use sf_core::ir::{TermMap, TermSpec};
    for variant in 0..6 {
        let mut branch = fixture();
        branch.core = (0..3)
            .map(|alias| Scan {
                alias,
                source: LogicalSource::Table("t".into()).into(),
            })
            .collect();
        branch.where_conds = vec![
            SqlCond::ColEq(ColRef::new(0, "id"), ColRef::new(1, "id")),
            SqlCond::ColEq(ColRef::new(1, "id"), ColRef::new(2, "id")),
        ];
        for alias in 0..3 {
            branch.bindings.insert(
                format!("v{alias}"),
                TermDef::Derived {
                    alias,
                    term_map: TermMap::Column("id".into(), TermSpec::iri()),
                },
            );
        }
        if variant == 1 || variant == 2 {
            for alias in 0..3 {
                branch.where_conds.push(SqlCond::Cmp(
                    ColRef::new(alias, "id"),
                    CmpOp::Eq,
                    if variant == 2 && alias == 1 {
                        "different"
                    } else {
                        "é"
                    }
                    .into(),
                ));
            }
        }
        if variant == 3 {
            branch.bindings.insert(
                "extra".into(),
                TermDef::Derived {
                    alias: 1,
                    term_map: TermMap::Column("unshared".into(), TermSpec::iri()),
                },
            );
        }
        if variant == 4 {
            branch
                .where_conds
                .push(SqlCond::IsNull(ColRef::new(1, "local")));
        }
        let context = CascadeCtx {
            distinct: variant != 5,
            project: None,
        };
        let run = |b: &mut Branch, control: &dyn QueryControl| {
            sameterm::with_work(b, &context, work(control))
        };
        let mut expected = branch.clone();
        sameterm::same_terms_elimination(&mut expected, &context);
        if variant == 0 {
            assert_eq!(expected.core.len(), 1);
        }
        if variant == 5 {
            assert_eq!(expected.core.len(), 3);
        }
        let measured = Stop {
            budget: budget(u64::MAX),
            calls: AtomicUsize::new(0),
            at: usize::MAX,
            cause: QueryControlError::Cancelled,
        };
        let mut actual = branch.clone();
        run(&mut actual, &measured).unwrap();
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
        let units = measured.budget.consumed(QueryCharge::CompilerWork);
        run(&mut branch.clone(), &budget(units)).unwrap();
        assert!(matches!(
            run(&mut branch.clone(), &budget(units - 1)),
            Err(crate::Error::QueryControl(
                QueryControlError::CompilerWorkExceeded
            ))
        ));
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            for at in 1..=measured.calls.load(Ordering::Relaxed) {
                let stopped = Stop {
                    budget: budget(u64::MAX),
                    calls: AtomicUsize::new(0),
                    at,
                    cause,
                };
                assert!(matches!(run(&mut branch.clone(), &stopped),
                    Err(crate::Error::QueryControl(actual)) if actual == cause));
                assert_eq!(stopped.checkpoint(), Err(cause));
            }
        }
    }
}

fn fixture() -> Branch {
    let mut branch = Branch::single(super::super::Scan {
        alias: 0,
        source: super::super::LogicalSource::Table("t".into()).into(),
    });
    branch.where_conds = vec![
        SqlCond::ColEq(ColRef::new(1, "b"), ColRef::new(2, "c")),
        SqlCond::ColEq(ColRef::new(0, "a"), ColRef::new(1, "b")),
        SqlCond::Cmp(ColRef::new(0, "a"), CmpOp::Eq, "é".into()),
    ];
    branch
}
fn budget(units: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX))
}
fn work(control: &dyn QueryControl) -> BuildWork<'_> {
    BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control)))
}

#[test]
fn constant_fixpoint_matches_raw_for_success_and_contradiction() {
    for conflict in [false, true] {
        let mut branch = fixture();
        if conflict {
            branch
                .where_conds
                .push(SqlCond::Cmp(ColRef::new(2, "c"), CmpOp::Eq, "other".into()));
        }
        let measured = budget(u64::MAX);
        let result = consistent(&branch, work(&measured)).unwrap();
        assert_eq!(result, super::super::prune_iri_template_mismatch(&branch));
        assert_eq!(result, !conflict);
        let units = measured.consumed(QueryCharge::CompilerWork);
        assert_eq!(consistent(&branch, work(&budget(units))).unwrap(), result);
        assert!(matches!(
            consistent(&branch, work(&budget(units - 1))),
            Err(crate::Error::QueryControl(
                QueryControlError::CompilerWorkExceeded
            ))
        ));
    }
}

#[test]
fn selection_preserves_stable_raw_partition_and_nested_flattening() {
    let mut branch = fixture();
    branch.where_conds.push(SqlCond::And(vec![
        SqlCond::And(vec![SqlCond::IsNull(ColRef::new(7, "n"))]),
        SqlCond::ColEq(ColRef::new(8, "x"), ColRef::new(9, "y")),
    ]));
    let mut expected = branch.clone();
    super::super::selection_pushdown(&mut expected);
    let measured = budget(u64::MAX);
    let mut actual = branch.clone();
    selection(&mut actual, work(&measured)).unwrap();
    assert_eq!(
        format!("{:?}", actual.where_conds),
        format!("{:?}", expected.where_conds)
    );
    let units = measured.consumed(QueryCharge::CompilerWork);
    selection(&mut branch.clone(), work(&budget(units))).unwrap();
    assert!(matches!(
        selection(&mut branch, work(&budget(units - 1))),
        Err(crate::Error::QueryControl(
            QueryControlError::CompilerWorkExceeded
        ))
    ));
}

struct Stop {
    budget: QueryBudget,
    calls: AtomicUsize,
    at: usize,
    cause: QueryControlError,
}
impl QueryControl for Stop {
    fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
        self.budget.checkpoint()
    }
    fn consume(&self, kind: QueryCharge, units: u64) -> std::result::Result<(), QueryControlError> {
        self.budget.consume(kind, units)?;
        if self.calls.fetch_add(1, Ordering::Relaxed) + 1 == self.at {
            self.budget.terminate(self.cause);
        }
        Ok(())
    }
    fn terminate(&self, cause: QueryControlError) -> QueryControlError {
        self.budget.terminate(cause)
    }
}

#[test]
fn condition_operations_stop_at_every_charge() {
    for operation in 0..3 {
        let run = |control: &dyn QueryControl| -> Result<()> {
            let mut branch = fixture();
            match operation {
                0 => {
                    consistent(&branch, work(control))?;
                }
                1 => selection(&mut branch, work(control))?,
                _ => {
                    has_subquery(&branch, work(control))?;
                }
            }
            Ok(())
        };
        let control = |at, cause| Stop {
            budget: budget(u64::MAX),
            calls: AtomicUsize::new(0),
            at,
            cause,
        };
        let measured = control(usize::MAX, QueryControlError::Cancelled);
        run(&measured).unwrap();
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            for at in 1..=measured.calls.load(Ordering::Relaxed) {
                let stopped = control(at, cause);
                assert!(
                    matches!(run(&stopped),Err(crate::Error::QueryControl(actual)) if actual==cause),
                    "operation {operation}, charge {at}"
                );
                assert_eq!(stopped.checkpoint(), Err(cause));
            }
        }
    }
}

#[test]
fn controlled_entry_preserves_firing_self_join_and_projection() {
    use super::super::{CascadeCtx, LogicalSource, Scan, TableSchema};
    use crate::iq::TermDef;
    use sf_core::ir::{TermMap, TermSpec};
    let mut branch = Branch::single(Scan {
        alias: 0,
        source: LogicalSource::Table("items".into()).into(),
    });
    branch.core.push(Scan {
        alias: 1,
        source: LogicalSource::Table("items".into()).into(),
    });
    branch.where_conds = vec![
        SqlCond::ColEq(ColRef::new(0, "id"), ColRef::new(1, "id")),
        SqlCond::Cmp(ColRef::new(1, "value"), CmpOp::Eq, "é".into()),
    ];
    branch.bindings.insert(
        "value".into(),
        TermDef::Derived {
            alias: 1,
            term_map: TermMap::Column("value".into(), TermSpec::plain_literal()),
        },
    );
    let mut table = TableSchema::new("items");
    table.primary_key = vec!["id".into()];
    table.columns = vec![sf_sql::Column::new("id", "text", true)];
    let schema = [table];
    let project = ["value".into()];
    let ctx = CascadeCtx {
        distinct: false,
        project: Some(&project),
    };
    let expected = super::super::run(vec![branch.clone()], &schema, &ctx);
    assert_eq!(
        expected[0].core.len(),
        1,
        "fixture must actually fire self-join elimination"
    );
    let measured = budget(u64::MAX);
    let actual =
        super::super::run_with_work(vec![branch.clone()], &schema, &ctx, work(&measured)).unwrap();
    assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
    let units = measured.consumed(QueryCharge::CompilerWork);
    assert!(
        super::super::run_with_work(vec![branch.clone()], &schema, &ctx, work(&budget(units)))
            .is_ok()
    );
    assert!(matches!(
        super::super::run_with_work(vec![branch], &schema, &ctx, work(&budget(units - 1))),
        Err(crate::Error::QueryControl(
            QueryControlError::CompilerWorkExceeded
        ))
    ));
}

#[test]
fn disjunction_fixpoint_preserves_duplicates_order_and_exact_refusal() {
    let or = |values: &[&str]| {
        SqlCond::Or(
            values
                .iter()
                .map(|value| SqlCond::Cmp(ColRef::new(0, "a"), CmpOp::Eq, (*value).into()))
                .collect(),
        )
    };
    for lists in [
        vec![vec!["é", "b", "c"], vec!["c", "é"], vec!["é"]],
        vec![vec!["é", "é"], vec!["é"]],
        vec![vec!["a"], vec!["b"]],
        vec![vec![], vec!["a"]],
    ] {
        let mut branch = fixture();
        branch.where_conds = lists.iter().map(|values| or(values)).collect();
        let mut expected = branch.clone();
        let verdict = super::super::disjunction_intersection_simplify(&mut expected);
        let control = budget(u64::MAX);
        let mut actual = branch.clone();
        assert_eq!(intersect(&mut actual, work(&control)).unwrap(), verdict);
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
        let units = control.consumed(QueryCharge::CompilerWork);
        assert_eq!(
            intersect(&mut branch.clone(), work(&budget(units))).unwrap(),
            verdict
        );
        assert!(matches!(
            intersect(&mut branch, work(&budget(units - 1))),
            Err(crate::Error::QueryControl(
                QueryControlError::CompilerWorkExceeded
            ))
        ));
    }
}

#[test]
fn disjunction_repair_observes_every_cancellation_and_deadline() {
    let mut branch = fixture();
    branch.where_conds = [vec!["a", "b", "c"], vec!["a", "b"], vec!["a"]]
        .into_iter()
        .map(|values| {
            SqlCond::Or(
                values
                    .into_iter()
                    .map(|value| SqlCond::Cmp(ColRef::new(0, "a"), CmpOp::Eq, value.into()))
                    .collect(),
            )
        })
        .collect();
    let control = |at, cause| Stop {
        budget: budget(u64::MAX),
        calls: AtomicUsize::new(0),
        at,
        cause,
    };
    let measured = control(usize::MAX, QueryControlError::Cancelled);
    intersect(&mut branch.clone(), work(&measured)).unwrap();
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for at in 1..=measured.calls.load(Ordering::Relaxed) {
            let stopped = control(at, cause);
            assert!(
                matches!(intersect(&mut branch.clone(),work(&stopped)),Err(crate::Error::QueryControl(actual)) if actual==cause),
                "charge {at}"
            );
            assert_eq!(stopped.checkpoint(), Err(cause));
        }
    }
}

#[test]
fn distinct_proofs_preserve_raw_null_literal_composite_and_multiscan_guards() {
    use super::super::{LogicalSource, Scan, TableSchema, TermDef};
    use sf_core::ir::{Segment, Template, TermMap, TermSpec};
    for variant in 0..6 {
        let mut branch = fixture();
        branch.distinct = true;
        branch.core = vec![Scan {
            alias: 0,
            source: LogicalSource::Table("items".into()).into(),
        }];
        branch.where_conds.clear();
        let mut table = TableSchema::new("items");
        table.unique = vec![vec!["id".into()]];
        table.columns = vec![sf_sql::Column::new("id", "text", variant != 1)];
        let spec = if variant == 2 {
            TermSpec::plain_literal()
        } else {
            TermSpec::iri()
        };
        branch.bindings.insert(
            "value".into(),
            TermDef::Derived {
                alias: 0,
                term_map: TermMap::Column("id".into(), spec),
            },
        );
        if variant == 3 {
            table.primary_key = vec!["id".into(), "other".into()];
            table.unique.clear();
            branch.bindings.insert(
                "value".into(),
                TermDef::Derived {
                    alias: 0,
                    term_map: TermMap::Template(
                        Template::from_segments(vec![
                            Segment::Literal("https://example.test/".into()),
                            Segment::Column("id".into()),
                            Segment::Literal("/".into()),
                            Segment::Column("other".into()),
                        ])
                        .unwrap(),
                        TermSpec::iri(),
                    ),
                },
            );
        }
        if variant == 4 {
            table.primary_key = vec!["id".into()];
            branch.core.push(Scan {
                alias: 1,
                source: LogicalSource::Table("items".into()).into(),
            });
            branch.bindings.insert(
                "second".into(),
                TermDef::Derived {
                    alias: 1,
                    term_map: TermMap::Column("id".into(), TermSpec::iri()),
                },
            );
        }
        let schema = [table];
        let map = super::super::build_schema_map(&schema);
        let project = if variant == 5 {
            vec!["missing".into()]
        } else {
            vec!["value".into(), "second".into()]
        };
        let run = |branch: &mut Branch, control: &dyn QueryControl| {
            super::super::control_distinct::remove(branch, &map, Some(&project), work(control))
        };
        let mut expected = branch.clone();
        super::super::distinct_removal(&mut expected, &map, Some(&project));
        assert_eq!(expected.distinct, matches!(variant, 1 | 2 | 5));
        let measured = Stop {
            budget: budget(u64::MAX),
            calls: AtomicUsize::new(0),
            at: usize::MAX,
            cause: QueryControlError::Cancelled,
        };
        let mut actual = branch.clone();
        run(&mut actual, &measured).unwrap();
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
        let units = measured.budget.consumed(QueryCharge::CompilerWork);
        run(&mut branch.clone(), &budget(units)).unwrap();
        let mut failed = branch.clone();
        assert!(matches!(
            run(&mut failed, &budget(units - 1)),
            Err(crate::Error::QueryControl(
                QueryControlError::CompilerWorkExceeded
            ))
        ));
        assert_eq!(failed.distinct, branch.distinct);
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            for at in 1..=measured.calls.load(Ordering::Relaxed) {
                let stopped = Stop {
                    budget: budget(u64::MAX),
                    calls: AtomicUsize::new(0),
                    at,
                    cause,
                };
                let mut failed = branch.clone();
                assert!(
                    matches!(run(&mut failed,&stopped),Err(crate::Error::QueryControl(actual)) if actual==cause)
                );
                assert_eq!(format!("{failed:?}"), format!("{branch:?}"));
            }
        }
    }
}
