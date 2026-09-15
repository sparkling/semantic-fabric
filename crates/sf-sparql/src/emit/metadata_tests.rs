use super::*;
use crate::iq::Scan;

#[test]
fn nested_sql_keeps_parent_source_control_and_exact_output() {
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    use sf_sql::source_work::SourceWork;
    let budget = |n| QueryBudget::new(QueryLimits::new(u64::MAX, n, u64::MAX, u64::MAX));
    for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
        for query in [
            "SELECT * WHERE { { SELECT * WHERE {} } }",
            "SELECT (COUNT(*) AS ?n) WHERE { { SELECT * WHERE {} } }",
        ] {
            let mut plan = crate::parse_and_translate(query, &[], dialect).unwrap();
            let condition = SqlCond::Or(vec![SqlCond::ExpressionError]);
            plan.branches[0]
                .where_conds
                .push(SqlCond::Not(Box::new(condition)));
            plan.branches[0].subplan_joins.push(crate::iq::SubPlanJoin {
                alias: 100,
                plan: Box::new(
                    crate::parse_and_translate("SELECT * WHERE {}", &[], dialect).unwrap(),
                ),
                on: vec![],
                left: false,
            });
            let catalog = ColumnCatalog::default();
            let expected = emit_subplan_sql(&plan, dialect, &catalog).unwrap();
            let run = |control: &QueryBudget| {
                let work = SourceWork::new(Some(control));
                emit_subplan_sql_controlled(&plan, dialect, &catalog, work)
            };
            let measured = budget(u64::MAX);
            assert_eq!(run(&measured).unwrap(), expected);
            let n = measured.consumed(QueryCharge::SourceWork);
            assert!(n > 1);
            assert_eq!(measured.consumed(QueryCharge::CompilerWork), 0);
            assert_eq!(run(&budget(n)).unwrap(), expected);
            assert!(matches!(
                run(&budget(n - 1)),
                Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
            ));
            let stopped = budget(u64::MAX);
            sf_core::query_control::QueryControl::terminate(&stopped, QueryControlError::Cancelled);
            assert!(matches!(
                run(&stopped),
                Err(Error::QueryControl(QueryControlError::Cancelled))
            ));
        }
    }
}

#[test]
fn controlled_subplan_source_layout_preserves_raw_metadata_not_validation() {
    use sf_core::ir::{Template, TermSpec};
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    use sf_sql::source_work::SourceWork;
    let budget = |n| QueryBudget::new(QueryLimits::new(u64::MAX, n, u64::MAX, u64::MAX));
    for aggregate in [false, true] {
        for path in [false, true] {
            for distinct in [false, true] {
                let mut branch = Branch::empty();
                branch.core = (0..2)
                    .map(|alias| Scan {
                        alias,
                        source: LogicalSource::Table("items".into()).into(),
                    })
                    .collect();
                branch.bindings.insert(
                    "x".into(),
                    TermDef::Derived {
                        alias: 0,
                        term_map: TermMap::Template(
                            Template::parse("{a}{b}").unwrap(),
                            TermSpec::iri(),
                        ),
                    },
                );
                branch.where_conds = vec![SqlCond::IsNotNull(ColRef::new(1, "filter"))];
                if aggregate {
                    branch.agg = Some(Aggregation {
                        keys: vec![crate::iq::GroupKey {
                            var: "x".into(),
                            cols: vec![ColRef::new(0, "a"), ColRef::new(0, "a")],
                        }],
                        aggs: vec![AggCol {
                            var: "avg".into(),
                            kind: AggKind::Avg,
                            arg: Some(ColRef::new(0, "b")),
                            distinct: false,
                            out: ColRef::new(2, "result"),
                            fixed_type: None,
                        }],
                    });
                }
                if path {
                    branch.path = Some(PathClosure {
                        alias: 3,
                        kind: PathKind::One,
                        hop: HopExpr::Alt(vec![]),
                    });
                }
                for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
                    let expected = source_projection(&branch, distinct, dialect);
                    if aggregate && !path {
                        assert_eq!(expected[2], None);
                        assert_eq!(
                            expected.len(),
                            if dialect == Dialect::Sqlite { 4 } else { 3 }
                        );
                    } else if distinct && !path {
                        assert!(projection_layout::validate_distinct_with(&branch, true).is_err());
                        assert!(!expected.is_empty()); // Metadata does not add validation rejection.
                    }
                    let run = |control: &QueryBudget| {
                        metadata_subplan::source_layout(
                            &branch,
                            distinct,
                            dialect,
                            SourceWork::new(Some(control)),
                        )
                    };
                    let measured = budget(u64::MAX);
                    assert_eq!(run(&measured).unwrap(), expected);
                    let n = measured.consumed(QueryCharge::SourceWork);
                    assert_eq!(measured.consumed(QueryCharge::CompilerWork), 0);
                    assert_eq!(run(&budget(n)).unwrap(), expected);
                    assert!(matches!(
                        run(&budget(n - 1)),
                        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                    ));
                }
            }
        }
    }
}

#[test]
fn propagated_subplan_path_flag_matches_recursive_oracle_despite_alias_overwrite() {
    use crate::iq::SubPlanJoin;
    use sf_sql::source_work::SourceWork;
    let path = || PathClosure {
        alias: 0,
        kind: PathKind::One,
        hop: HopExpr::Alt(vec![]),
    };
    let plan = |branch| {
        let mut plan =
            crate::parse_and_translate("SELECT * WHERE {}", &[], Dialect::Sqlite).unwrap();
        plan.branches = vec![branch];
        plan
    };
    for location in 0..6 {
        let mut branch = Branch::empty();
        let condition = || {
            SqlCond::Not(Box::new(SqlCond::Or(vec![SqlCond::PathExists {
                pc: path(),
                conds: vec![],
                negated: false,
            }])))
        };
        match location {
            1 => branch.path = Some(path()),
            2 => branch.where_conds.push(condition()),
            3 => branch.subplan_joins.push(SubPlanJoin {
                alias: 0,
                plan: Box::new(plan(Branch::empty())),
                on: vec![condition()],
                left: true,
            }),
            4 | 5 => {
                let mut child = Branch::empty();
                child.path = Some(path());
                branch.subplan_joins.push(SubPlanJoin {
                    alias: 0,
                    plan: Box::new(plan(child)),
                    on: vec![],
                    left: false,
                });
                if location == 5 {
                    branch.subplan_joins.push(SubPlanJoin {
                        alias: 0,
                        plan: Box::new(plan(Branch::empty())),
                        on: vec![],
                        left: false,
                    });
                }
            }
            _ => {}
        }
        let expected = path_comparison::branch_has_path(&branch);
        assert_eq!(expected, location != 0);
        let mut plan = plan(branch);
        plan.branches.push(Branch::empty());
        let result = metadata::subplan_actuals_controlled(
            &plan,
            Dialect::Sqlite,
            &ColumnCatalog::default(),
            SourceWork::new(None),
        )
        .unwrap();
        assert_eq!(result.path, expected);
    }
}

#[test]
fn subplan_metadata_descent_uses_shared_stack_and_releases_each_arm() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            use crate::iq::SubPlanJoin;
            use sf_core::ir::TermSpec;
            use sf_core::query_control::{
                QueryBudget, QueryCharge, QueryControlError, QueryLimits,
            };
            use sf_sql::source_work::SourceWork;
            fn plan(branch: Branch) -> crate::Plan {
                crate::Plan {
                    branches: vec![branch],
                    form: crate::PlanForm::Select {
                        vars: vec!["x".into()],
                    },
                    distinct: false,
                    limit: None,
                    offset: 0,
                    order: vec![],
                    rust_group: None,
                    dialect: Dialect::Sqlite,
                    dedup_scopes: vec![],
                    construct_drops_some_branch_var: false,
                }
            }
            let source = LogicalSource::Table("items".into());
            let mut catalog = ColumnCatalog::default();
            catalog.insert(&source, vec!["id".into()]);
            let mut leaf = Branch::single(Scan {
                alias: 0,
                source: source.into(),
            });
            leaf.bindings.insert(
                "x".into(),
                TermDef::Derived {
                    alias: 0,
                    term_map: TermMap::Column("id".into(), TermSpec::plain_literal()),
                },
            );
            let mut nested = plan(leaf);
            for _ in 0..1024 {
                let mut branch = Branch::empty();
                branch.bindings.insert(
                    "x".into(),
                    TermDef::Derived {
                        alias: 0,
                        term_map: TermMap::Column("c0".into(), TermSpec::plain_literal()),
                    },
                );
                branch.subplan_joins.push(SubPlanJoin {
                    alias: 0,
                    plan: Box::new(nested),
                    on: vec![],
                    left: false,
                });
                nested = plan(branch);
            }
            let budget = |n| QueryBudget::new(QueryLimits::new(u64::MAX, n, u64::MAX, u64::MAX));
            let run = |control: &QueryBudget| {
                metadata::subplan_actuals_controlled(
                    &nested,
                    Dialect::Sqlite,
                    &catalog,
                    SourceWork::new(Some(control)),
                )
            };
            let measured = budget(u64::MAX);
            assert_eq!(run(&measured).unwrap().columns, ["c0"]);
            let units = measured.consumed(QueryCharge::SourceWork);
            assert_eq!(measured.consumed(QueryCharge::CompilerWork), 0);
            assert_eq!(run(&budget(units)).unwrap().columns, ["c0"]);
            assert!(matches!(
                run(&budget(units - 1)),
                Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
            ));
            // Avoid recursively dropping the input fixture on this intentionally small stack.
            loop {
                let mut branch = nested.branches.pop().unwrap();
                let Some(join) = branch.subplan_joins.pop() else {
                    break;
                };
                nested = *join.plan;
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn projection_reference_metadata_descent_uses_one_explicit_stack() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            use crate::iq::ScanSource;
            use sf_core::ir::TermSpec;
            use sf_core::query_control::{
                QueryBudget, QueryCharge, QueryControlError, QueryLimits,
            };
            use sf_sql::source_work::SourceWork;
            let source = LogicalSource::Table("items".into());
            let mut catalog = ColumnCatalog::default();
            catalog.insert(&source, vec!["id".into()]);
            let mut scan = Scan {
                alias: 0,
                source: source.into(),
            };
            for depth in 0..1024 {
                scan = Scan {
                    alias: 0,
                    source: ScanSource::Projection {
                        input: Box::new(scan),
                        columns: vec![(
                            "id".into(),
                            TermMap::Column(
                                if depth == 0 { "id" } else { "c0" }.into(),
                                TermSpec::plain_literal(),
                            ),
                        )],
                        guards: vec![],
                        distinct: false,
                        native_keys: vec![],
                        lexical_keys: vec![],
                    },
                };
                scan = Scan {
                    alias: 0,
                    source: ScanSource::RefAtom {
                        input: Box::new(Branch::single(scan)),
                        columns: vec![ColRef::new(0, "id")],
                    },
                };
            }
            let budget = |n| QueryBudget::new(QueryLimits::new(u64::MAX, n, u64::MAX, u64::MAX));
            let run = |control: &QueryBudget| {
                metadata::scan_actuals_controlled(
                    &scan,
                    Dialect::Sqlite,
                    &catalog,
                    SourceWork::new(Some(control)),
                )
            };
            let measured = budget(u64::MAX);
            assert_eq!(run(&measured).unwrap().columns, ["c0"]);
            let units = measured.consumed(QueryCharge::SourceWork);
            assert_eq!(measured.consumed(QueryCharge::CompilerWork), 0);
            assert_eq!(run(&budget(units)).unwrap().columns, ["c0"]);
            assert!(matches!(
                run(&budget(units - 1)),
                Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
            ));
            // Input ownership destruction is a separate boundary; release this test
            // fixture iteratively rather than confusing Drop with traversal depth.
            loop {
                match scan.source {
                    ScanSource::RefAtom { mut input, .. } => scan = input.core.pop().unwrap(),
                    ScanSource::Projection { input, .. } => scan = *input,
                    ScanSource::Logical(_) => break,
                    _ => unreachable!(),
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn controlled_source_metadata_keeps_facts_and_stable_exact_admission() {
    use sf_core::datatype::XsdTypeCode;
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    use sf_sql::source_work::SourceWork;
    let budget = |n| QueryBudget::new(QueryLimits::new(u64::MAX, n, u64::MAX, u64::MAX));
    for source in [
        LogicalSource::Table("table α".into()),
        LogicalSource::Query("SELECT α".into()),
    ] {
        let mut measured_cost = None;
        for reverse in [false, true].into_iter().cycle().take(12) {
            let mut columns: Vec<_> = ["α", "long column", "s"]
                .into_iter()
                .map(|name| sf_sql::backend::ResultColumn {
                    name: name.into(),
                    natural_datatype: Some(XsdTypeCode::String),
                    native_scalar: None,
                    text_key: Some(TextKey::Verbatim),
                    sqlite_decode: Some(SqliteDecode {
                        declared: Some(XsdTypeCode::String),
                        padding: None,
                    }),
                })
                .collect();
            if reverse {
                columns.reverse();
            }
            let mut catalog = ColumnCatalog::default();
            catalog
                .insert_live_result(&source, columns.clone())
                .unwrap();
            let run = |control: &QueryBudget| {
                source_actuals_controlled(&source, &catalog, SourceWork::new(Some(control)))
            };
            let control = budget(u64::MAX);
            let actuals = run(&control).unwrap();
            assert_eq!(
                actuals.columns,
                columns.iter().map(|c| c.name.clone()).collect::<Vec<_>>()
            );
            for column in &columns {
                assert_eq!(
                    actuals.datatype_columns[&column.name],
                    Some(XsdTypeCode::String)
                );
                assert_eq!(
                    actuals.natural_columns[&column.name],
                    Some(XsdTypeCode::String)
                );
                assert_eq!(actuals.text_columns[&column.name], TextKey::Verbatim);
                assert_eq!(
                    actuals.sqlite_columns[&column.name],
                    column.sqlite_decode.unwrap()
                );
            }
            assert!(
                actuals.scalar_columns.is_empty()
                    && actuals.lexical_columns.is_empty()
                    && actuals.lexical_comparison_columns.is_empty()
                    && actuals.static_iri_columns.is_empty()
                    && actuals.iri_unreserved_columns.is_empty()
                    && !actuals.path
            );
            assert_eq!(
                actuals.source_kind,
                if matches!(source, LogicalSource::Table(_)) {
                    AliasSourceKind::Table
                } else {
                    AliasSourceKind::Query
                }
            );
            let units = control.consumed(QueryCharge::SourceWork);
            assert_eq!(*measured_cost.get_or_insert(units), units);
            assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
            assert_eq!(run(&budget(units)).unwrap().columns, actuals.columns);
            assert!(matches!(
                run(&budget(units - 1)),
                Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
            ));
        }
    }
}

#[test]
fn controlled_logical_aliases_keep_nested_order_and_root_overwrite() {
    use sf_core::query_control::{QueryBudget, QueryLimits};
    use sf_sql::source_work::SourceWork;
    let a = LogicalSource::Table("a".into());
    let b = LogicalSource::Table("b".into());
    let mut branch = Branch::single(Scan {
        alias: 0,
        source: a.clone().into(),
    });
    branch.where_conds = vec![SqlCond::Not(Box::new(SqlCond::And(vec![
        SqlCond::Exists {
            scans: vec![Scan {
                alias: 0,
                source: b.clone().into(),
            }],
            conds: vec![SqlCond::NotExists {
                scans: vec![Scan {
                    alias: 7,
                    source: b.clone().into(),
                }],
                conds: vec![],
            }],
        },
    ])))];
    let control = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    let work = SourceWork::new(Some(&control));
    let aliases = scan::logical_aliases(&branch, work).unwrap();
    let expected = branch.alias_sources();
    assert_eq!(aliases.len(), expected.len());
    for ((alias, source), (expected_alias, expected_source)) in aliases.iter().zip(&expected) {
        assert_eq!(alias, expected_alias);
        assert!(std::ptr::eq(*source, *expected_source));
    }
    let mut catalog = ColumnCatalog::default();
    catalog.insert(&a, vec!["a_field".into()]);
    catalog.insert(&b, vec!["b_field".into()]);
    let actuals = branch_actuals_controlled(&branch, Dialect::Sqlite, &catalog, work).unwrap();
    assert_eq!(actuals[&0].columns, ["a_field"]);
    assert_eq!(actuals[&7].columns, ["b_field"]);
}
