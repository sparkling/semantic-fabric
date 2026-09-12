use super::*;
use crate::{compiler_control::CompileContext, CompilerWorkMode};
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn empty_cascade_retains_stop_checks_without_branch_visit() {
    let control = budget(0);
    let run = || super::super::control::run(vec![], &[], &Default::default(), work(&control));
    assert!(run().unwrap().is_empty());
    assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
    control.terminate(QueryControlError::Cancelled);
    assert!(matches!(
        run(),
        Err(crate::Error::QueryControl(QueryControlError::Cancelled))
    ));
}

#[test]
fn fk_condition_remap_preserves_swaps_and_interrupts_recursion() {
    use super::super::control_conditions::ColumnRemap;
    let pairs = [("a".into(), "b".into()), ("b".into(), "a".into())];
    let remap = ColumnRemap {
        parent: 1,
        child: 0,
        pairs: &pairs,
    };
    let source = SqlCond::Not(Box::new(SqlCond::And(vec![
        SqlCond::ColEq(ColRef::new(1, "a"), ColRef::new(1, "b")),
        SqlCond::IsNull(ColRef::new(9, "a")),
    ])));
    let run = |control: &dyn QueryControl| {
        let mut output = source.clone();
        remap.condition(&mut output, work(control))?;
        Ok::<_, crate::Error>(output)
    };
    let measured = Stop {
        budget: budget(u64::MAX),
        calls: AtomicUsize::new(0),
        at: usize::MAX,
        cause: QueryControlError::Cancelled,
    };
    let expected = SqlCond::Not(Box::new(SqlCond::And(vec![
        SqlCond::ColEq(ColRef::new(0, "b"), ColRef::new(0, "a")),
        SqlCond::IsNull(ColRef::new(9, "a")),
    ])));
    assert_eq!(
        format!("{:?}", run(&measured).unwrap()),
        format!("{expected:?}")
    );
    let units = measured.budget.consumed(QueryCharge::CompilerWork);
    run(&budget(units)).unwrap();
    assert!(matches!(
        run(&budget(units - 1)),
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
            assert!(
                matches!(run(&stopped), Err(crate::Error::QueryControl(actual)) if actual == cause)
            );
        }
    }
}

#[test]
fn fk_term_rename_pays_for_swaps_payloads_and_every_stop() {
    use sf_core::ir::{Template, TermMap, TermSpec};
    let pairs = [("a".into(), "b".into()), ("b".into(), "a".into())];
    for source in [
        TermMap::Template(
            Template::parse("urn:é:{a}/{b}/{untouched}").unwrap(),
            TermSpec::iri(),
        ),
        TermMap::Column("a".into(), TermSpec::plain_literal()),
        TermMap::Constant(oxrdf::NamedNode::new("urn:constant").unwrap().into()),
    ] {
        let run = |control: &dyn QueryControl| {
            super::super::joinelim::rename_columns_with_work(&source, &pairs, work(control))
        };
        let measured = Stop {
            budget: budget(u64::MAX),
            calls: AtomicUsize::new(0),
            at: usize::MAX,
            cause: QueryControlError::Cancelled,
        };
        let actual = run(&measured).unwrap();
        if let TermMap::Template(template, _) = &actual {
            assert_eq!(
                template.segments(),
                Template::parse("urn:é:{b}/{a}/{untouched}")
                    .unwrap()
                    .segments()
            );
        }
        let units = measured.budget.consumed(QueryCharge::CompilerWork);
        assert_eq!(run(&budget(units)).unwrap(), actual);
        assert!(matches!(
            run(&budget(units - 1)),
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
                assert!(
                    matches!(run(&stopped), Err(crate::Error::QueryControl(actual)) if actual == cause)
                );
                assert_eq!(stopped.checkpoint(), Err(cause));
            }
        }
    }
}

#[test]
fn composite_fk_column_swaps_are_simultaneous() {
    use super::super::{joinelim, Scan, TermDef};
    use sf_core::ir::{Template, TermMap, TermSpec};
    let mut child = TableSchema::new("child");
    child.columns = vec![
        sf_sql::Column::new("a", "text", true),
        sf_sql::Column::new("b", "text", true),
    ];
    child.foreign_keys.push(sf_sql::ForeignKey {
        columns: vec!["b".into(), "a".into()],
        parent_table: "parent".into(),
        parent_columns: vec!["a".into(), "b".into()],
    });
    let mut parent = TableSchema::new("parent");
    parent.primary_key = vec!["a".into(), "b".into()];
    let mut branch = Branch::single(Scan {
        alias: 0,
        source: LogicalSource::Table("child".into()).into(),
    });
    branch.core.push(Scan {
        alias: 1,
        source: LogicalSource::Table("parent".into()).into(),
    });
    branch.where_conds = vec![
        SqlCond::ColEq(ColRef::new(0, "b"), ColRef::new(1, "a")),
        SqlCond::ColEq(ColRef::new(0, "a"), ColRef::new(1, "b")),
    ];
    branch.bindings.insert(
        "value".into(),
        TermDef::Derived {
            alias: 1,
            term_map: TermMap::Template(
                Template::parse("urn:item:{a}/{b}").unwrap(),
                TermSpec::iri(),
            ),
        },
    );
    let schema = [child, parent];
    let fds = super::super::fd::infer_functional_dependencies(&branch, &schema);
    joinelim::fk_pk_join_elimination(&mut branch, &schema, &fds);
    assert_eq!(branch.core.len(), 1);
    let TermDef::Derived {
        alias,
        term_map: TermMap::Template(template, _),
    } = &branch.bindings["value"]
    else {
        panic!("template binding");
    };
    assert_eq!(*alias, 0);
    assert_eq!(
        template.segments(),
        Template::parse("urn:item:{b}/{a}").unwrap().segments()
    );
}

#[test]
fn fd_elimination_preserves_nullable_distinct_and_coverage_guards() {
    use super::super::{CascadeCtx, Scan, TermDef};
    use crate::iq::OptJoin;
    use sf_core::ir::{TermMap, TermSpec};
    for variant in 0..8 {
        let mut table = TableSchema::new("items");
        table.columns = vec![sf_sql::Column::new(
            "det",
            "integer",
            variant != 1 && variant != 5,
        )];
        table.functional_dependencies.push(sf_sql::FunctionalDep {
            det: vec!["det".into()],
            dep: vec!["value".into()],
        });
        let scan = |alias| Scan {
            alias,
            source: LogicalSource::Table("items".into()).into(),
        };
        let mut branch = Branch::single(scan(0));
        for alias in 1..3 {
            let eq = SqlCond::ColEq(ColRef::new(alias - 1, "det"), ColRef::new(alias, "det"));
            if variant >= 4 {
                branch.opts.push(OptJoin {
                    scan: scan(alias),
                    on: vec![eq],
                    extra: if variant == 6 {
                        vec![SqlCond::IsNull(ColRef::new(alias, "value"))]
                    } else {
                        vec![]
                    },
                });
            } else {
                branch.core.push(scan(alias));
                branch.where_conds.push(eq);
            }
        }
        for alias in 0..3 {
            branch.bindings.insert(
                format!("v{alias}"),
                TermDef::Derived {
                    alias,
                    term_map: TermMap::Column(
                        if variant == 2 || variant == 7 {
                            "outside"
                        } else {
                            "value"
                        }
                        .into(),
                        TermSpec::iri(),
                    ),
                },
            );
        }
        let context = CascadeCtx {
            distinct: variant != 3,
            project: None,
        };
        let schema = [table];
        let map = super::super::build_schema_map(&schema);
        let run = |branch: &mut Branch, control: &dyn QueryControl| {
            eliminate(branch, &map, &context, work(control))
        };
        let mut expected = branch.clone();
        super::super::fd_self_join_elimination(&mut expected, &map, &context);
        assert_eq!(
            expected.core.len() + expected.opts.len(),
            if matches!(variant, 0 | 1 | 4) { 1 } else { 3 }
        );
        if variant == 1 {
            assert!(expected
                .where_conds
                .iter()
                .any(|c| matches!(c, SqlCond::IsNotNull(_))));
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
                assert!(
                    matches!(run(&mut branch.clone(), &stopped), Err(crate::Error::QueryControl(actual)) if actual == cause)
                );
                assert_eq!(stopped.checkpoint(), Err(cause));
            }
        }
    }
}

#[test]
fn foreign_key_promotion_requires_complete_match_proof_and_paid_work() {
    use super::super::{control_join, joinelim, Scan};
    use crate::iq::OptJoin;
    for variant in 0..6 {
        let mut table = TableSchema::new("items");
        table.primary_key = vec!["id".into()];
        table.columns = vec![sf_sql::Column::new("parent", "integer", variant != 4)];
        if variant != 5 {
            table.foreign_keys.push(sf_sql::ForeignKey {
                columns: vec!["parent".into()],
                parent_table: "items".into(),
                parent_columns: vec!["id".into()],
            });
        }
        let scan = |alias| Scan {
            alias,
            source: LogicalSource::Table("items".into()).into(),
        };
        let mut branch = Branch::single(scan(0));
        for alias in 1..3 {
            branch.opts.push(OptJoin {
                scan: scan(alias),
                on: vec![SqlCond::NullSafeEq(
                    ColRef::new(alias - 1, "parent"),
                    ColRef::new(alias, "id"),
                )],
                extra: match variant {
                    1 => vec![SqlCond::IsNotNull(ColRef::new(alias, "id"))],
                    2 => vec![SqlCond::IsNotNull(ColRef::new(alias, "nullable"))],
                    3 => vec![SqlCond::DecodedIsNotNull(ColRef::new(alias, "id"))],
                    _ => vec![],
                },
            });
        }
        let schema = [table];
        let run = |branch: &mut Branch, control: &dyn QueryControl| {
            control_join::downgrade(branch, &schema, work(control))
        };
        let mut expected = branch.clone();
        joinelim::lj_to_ij_fk_downgrade(&mut expected, &schema);
        assert_eq!(expected.opts.len(), if variant < 2 { 0 } else { 2 });
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

fn fixture() -> (Branch, Vec<TableSchema>) {
    let mut branch = Branch::single(super::super::Scan {
        alias: 0,
        source: LogicalSource::Table("a".into()).into(),
    });
    branch.core.push(super::super::Scan {
        alias: 1,
        source: LogicalSource::Table("b".into()).into(),
    });
    branch.where_conds = vec![
        // Reverse order forces later rounds to propagate the column dependency.
        SqlCond::ColEq(ColRef::new(1, "y"), ColRef::new(2, "z")),
        SqlCond::ColEq(ColRef::new(0, "x"), ColRef::new(1, "y")),
    ];
    let mut a = TableSchema::new("a");
    a.primary_key = vec!["x".into()];
    a.unique = vec![vec!["x".into()], vec!["x".into(), "other".into()]];
    a.functional_dependencies = vec![sf_sql::FunctionalDep {
        det: vec!["x".into()],
        dep: vec!["payload-é".into()],
    }];
    let mut b = TableSchema::new("b");
    b.primary_key = vec!["y".into()];
    (branch, vec![a, b])
}

fn budget(units: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX))
}

fn work(control: &dyn QueryControl) -> BuildWork<'_> {
    BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control)))
}

#[test]
fn fd_closure_preserves_raw_order_and_exact_boundary() {
    let (branch, schema) = fixture();
    let expected = super::super::fd::infer_functional_dependencies(&branch, &schema);
    let measured = budget(u64::MAX);
    let actual = infer(&branch, &schema, work(&measured)).unwrap();
    assert_eq!(actual.deps, expected.deps);
    assert_eq!(actual.col_deps, expected.col_deps);
    assert!(actual.determines_col(&ColRef::new(2, "z"), &ColRef::new(0, "payload-é")));
    let units = measured.consumed(QueryCharge::CompilerWork);
    assert!(infer(&branch, &schema, work(&budget(units))).is_ok());
    assert!(matches!(
        infer(&branch, &schema, work(&budget(units - 1))),
        Err(crate::Error::QueryControl(
            QueryControlError::CompilerWorkExceeded
        ))
    ));
    let empty = infer(&branch, &[], work(&budget(u64::MAX))).unwrap();
    assert!(empty.deps.is_empty() && empty.col_deps.is_empty());
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
            // Return success to prove the worker checks sticky termination itself.
            self.budget.terminate(self.cause);
        }
        Ok(())
    }
    fn terminate(&self, cause: QueryControlError) -> QueryControlError {
        self.budget.terminate(cause)
    }
}

#[test]
fn fd_closure_observes_cancellation_and_deadline_at_every_charge() {
    let (branch, schema) = fixture();
    let control = |at, cause| Stop {
        budget: budget(u64::MAX),
        calls: AtomicUsize::new(0),
        at,
        cause,
    };
    let measured = control(usize::MAX, QueryControlError::Cancelled);
    infer(&branch, &schema, work(&measured)).unwrap();
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for at in 1..=measured.calls.load(Ordering::Relaxed) {
            let stopped = control(at, cause);
            assert!(
                matches!(infer(&branch, &schema, work(&stopped)),
                Err(crate::Error::QueryControl(actual)) if actual == cause),
                "charge {at}"
            );
            assert_eq!(stopped.checkpoint(), Err(cause));
        }
    }
}
