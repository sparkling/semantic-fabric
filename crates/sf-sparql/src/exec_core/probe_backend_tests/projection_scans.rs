use super::*;
use crate::iq::{OptJoin, ScanSource};
use sf_core::ir::Template;

#[test]
fn dedup_wrapper_walk_borrows_deep_plans_and_preserves_prepared_modifiers() {
    use crate::exec_core::dedup_scope_runtime::validate_runtime_scopes;
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let mut branch = column_branch(0, LogicalSource::Table("t".into()), "c0");
            let keys = branch.bindings.clone();
            for _ in 0..4096 {
                // An unordered single-branch plan replaces these stored modifiers.
                branch.limit = Some(7);
                branch.offset = 3;
                branch.distinct = true;
                let nested = select_plan(vec![branch]);
                branch = Branch::empty();
                branch.bindings = keys.clone();
                branch.subplan_joins.push(crate::iq::SubPlanJoin {
                    alias: 0,
                    plan: Box::new(nested),
                    on: vec![],
                    left: false,
                });
            }
            let branches = vec![
                branch,
                column_branch(0, LogicalSource::Table("t".into()), "c0"),
            ];
            let scopes = vec![
                Some(DedupScope {
                    group_id: 1,
                    key_bindings: keys.clone(),
                }),
                Some(DedupScope {
                    group_id: 1,
                    key_bindings: keys,
                }),
            ];
            let budget =
                |units| QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX));
            let measured = budget(u64::MAX);
            assert!(validate_runtime_scopes(&branches, &scopes, &measured).unwrap());
            let units = measured.consumed(QueryCharge::SourceWork);
            assert!(validate_runtime_scopes(&branches, &scopes, &budget(units)).unwrap());
            assert!(matches!(
                validate_runtime_scopes(&branches, &scopes, &budget(units - 1)),
                Err(crate::Error::QueryControl(
                    QueryControlError::SourceWorkExceeded
                ))
            ));
            let overlay = budget(u64::MAX);
            assert!(!super::super::driver::source_prepare::requires_key_overlay(
                &branches,
                &scopes,
                sf_sql::source_work::SourceWork::new(Some(&overlay)),
            )
            .unwrap());
            let mut plan = select_plan(branches);
            plan.dedup_scopes = scopes;
            let control = budget(units + overlay.consumed(QueryCharge::SourceWork));
            let mut backend = backend_with(vec![]);
            assert!(matches!(
                super::super::block_on(super::super::driver::for_each_solution_controlled(
                    &plan,
                    &mut backend,
                    &control,
                    |_, _| Ok(std::future::ready(Ok(()))),
                )),
                Err(crate::Error::QueryControl(
                    QueryControlError::SourceWorkExceeded
                ))
            ));
            assert!(backend.probes.is_empty());
            assert_eq!(backend.opens, 0);
            let previous = plan.branches[0]
                .bindings
                .insert(
                    "v".into(),
                    TermDef::Const(sf_core::Literal::new_simple_literal("collision").into()),
                )
                .unwrap();
            assert!(
                matches!(super::super::block_on(super::super::driver::for_each_solution_controlled(
                &plan, &mut backend, &budget(u64::MAX), |_, _| Ok(std::future::ready(Ok(()))),
            )), Err(crate::Error::Unsupported(message)) if message.contains("key collides"))
            );
            assert!(backend.probes.is_empty());
            assert_eq!(backend.opens, 0);
            plan.branches[0].bindings.insert("v".into(), previous);
            let mut branches = plan.branches;
            let scopes = plan.dedup_scopes;
            // Root modifiers have no enclosing plan override and must still refuse.
            branches[0].limit = Some(1);
            assert!(validate_runtime_scopes(&branches, &scopes, &budget(u64::MAX)).is_err());
            branches[0].limit = None;
            // Ordered plans do not clear stored branch slices during preparation.
            branches[0].subplan_joins[0]
                .plan
                .order
                .push(crate::iq::OrderKey {
                    var: "v".into(),
                    descending: false,
                    expr: None,
                });
            assert!(validate_runtime_scopes(&branches, &scopes, &budget(u64::MAX)).is_err());
            // Recursive IQ Drop is separate; test zero/nonzero modifier preparation.
            let mut zero = select_plan(vec![branches.remove(0)]);
            zero.limit = Some(0);
            zero.branches[0].distinct = true;
            let mut backend = backend_with(vec![]);
            assert!(run_select(&zero, &mut backend).unwrap().rows.is_empty());
            zero.rust_group = Some(crate::iq::RustGroup {
                keys: vec![],
                aggs: vec![],
                post_exprs: vec![],
            });
            assert!(run_select(&zero, &mut backend).unwrap().rows.is_empty());
            assert!(backend.probes.is_empty());
            assert_eq!(backend.opens, 0);
            zero.limit = Some(1);
            for grouped in [true, false] {
                if !grouped {
                    zero.rust_group = None;
                }
                let control = budget(0);
                assert!(matches!(
                    super::super::block_on(super::super::driver::for_each_solution_controlled(
                        &zero,
                        &mut backend,
                        &control,
                        |_, _| Ok(std::future::ready(Ok(()))),
                    )),
                    Err(crate::Error::QueryControl(
                        QueryControlError::SourceWorkExceeded
                    ))
                ));
                assert!(backend.probes.is_empty());
                assert_eq!(backend.opens, 0);
            }
            branches.push(zero.branches.pop().unwrap());
            for mut branch in branches {
                while let Some(mut join) = branch.subplan_joins.pop() {
                    branch = join.plan.branches.pop().unwrap();
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

fn projected(source: LogicalSource) -> Scan {
    Scan {
        alias: 7,
        source: ScanSource::Projection {
            input: Box::new(Scan {
                alias: 6,
                source: ScanSource::Projection {
                    input: Box::new(Scan {
                        alias: 5,
                        source: source.into(),
                    }),
                    columns: vec![(
                        "KEY".into(),
                        TermMap::Column("KEY".into(), TermSpec::plain_literal()),
                    )],
                    guards: vec![],
                    distinct: true,
                    native_keys: vec![],
                    lexical_keys: vec![],
                },
            }),
            columns: vec![(
                "value".into(),
                TermMap::Template(Template::parse("prefix/{KEY}").unwrap(), TermSpec::iri()),
            )],
            guards: vec![SqlCond::IsNotNull(ColRef::new(6, "KEY"))],
            distinct: false,
            native_keys: vec![],
            lexical_keys: vec![],
        },
    }
}

fn plan(scan: Scan, position: usize) -> Plan {
    let mut branch = Branch::empty();
    let condition = SqlCond::IsNotNull(ColRef::new(7, "value"));
    match position {
        0 => branch.core.push(scan),
        1 => branch.opts.push(OptJoin {
            scan,
            on: vec![condition],
            extra: vec![],
        }),
        2 => branch.where_conds.push(SqlCond::Exists {
            scans: vec![scan],
            conds: vec![condition],
        }),
        3 => branch.where_conds.push(SqlCond::NotExists {
            scans: vec![scan],
            conds: vec![condition],
        }),
        _ => unreachable!(),
    }
    let mut plan = select_plan(vec![branch]);
    plan.dialect = Dialect::Postgres;
    plan
}

#[test]
fn nested_projection_positions_probe_only_original_sources_and_fold_live_columns() {
    for source in [
        LogicalSource::Table("items".into()),
        LogicalSource::Query("SELECT actual AS key FROM items /* AS KEY is not metadata */".into()),
    ] {
        for position in 0..4 {
            let mut backend = backend_with(vec![Ok(vec!["key".into()])]);
            run_select(&plan(projected(source.clone()), position), &mut backend).unwrap();
            assert_eq!(backend.probes, vec![Dialect::Postgres.probe_sql(&source)]);
            assert_eq!(backend.opens, 1);
            assert!(
                backend.sql[0].contains("t5.\"key\" AS \"KEY\""),
                "{}",
                backend.sql[0]
            );
            assert!(
                backend.sql[0].contains("t6.\"KEY\" IS NOT NULL"),
                "{}",
                backend.sql[0]
            );
        }
    }
}

#[test]
fn missing_or_ambiguous_leaf_names_reject_before_any_cursor() {
    for names in [vec!["wrong".into()], vec!["key".into(), "Key".into()]] {
        for position in 0..4 {
            let mut backend = backend_with(vec![Ok(names.clone())]);
            assert!(run_select(
                &plan(projected(LogicalSource::Table("items".into())), position),
                &mut backend
            )
            .is_err());
            assert_eq!(backend.opens, 0);
        }
    }
}

#[test]
fn wrapper_recipes_are_charged_by_plan_measurement() {
    use crate::plan_measure::clone_root::{measure_compiler_clone_root_v1, CompilerCloneRootV1};
    let small = projected(LogicalSource::Table("items".into()));
    let mut large = small.clone();
    let ScanSource::Projection { columns, .. } = &mut large.source else {
        unreachable!()
    };
    columns[0].0 = "x".repeat(4096).into();
    let a = measure_compiler_clone_root_v1(CompilerCloneRootV1::Scan(&small)).unwrap();
    let b = measure_compiler_clone_root_v1(CompilerCloneRootV1::Scan(&large)).unwrap();
    assert!(b.deep_clone_work > a.deep_clone_work);
}

#[test]
fn parameter_values_stay_outside_projection_recipes() {
    let mut plan = plan(projected(LogicalSource::Table("items".into())), 0);
    plan.branches[0].where_conds.push(SqlCond::Cmp(
        ColRef::new(7, "value"),
        crate::iq::CmpOp::Eq,
        "user-supplied-'value".into(),
    ));
    let emitted = crate::emit::emit_branch(&plan.branches[0], Dialect::Postgres).unwrap();
    assert_eq!(emitted.params, vec!["user-supplied-'value"]);
    assert!(!emitted.sql.contains("user-supplied"));
    assert!(emitted.sql.contains("$1"));
}

fn reference_projection() -> Scan {
    let mut atom = Branch::single(Scan {
        alias: 4,
        source: LogicalSource::Table("child".into()).into(),
    });
    atom.core.push(Scan {
        alias: 5,
        source: LogicalSource::Table("parent".into()).into(),
    });
    atom.bindings.insert(
        "value".into(),
        TermDef::Derived {
            alias: 5,
            term_map: TermMap::Column("LABEL".into(), TermSpec::plain_literal()),
        },
    );
    atom.where_conds = vec![
        SqlCond::NativeColEq(ColRef::new(4, "FK"), ColRef::new(5, "K")),
        SqlCond::NativeCmp(
            ColRef::new(5, "TENANT"),
            crate::iq::CmpOp::Eq,
            "policy-'value".into(),
        ),
    ];
    let atom = crate::iq::scan::ref_atom::seal(atom)
        .unwrap()
        .core
        .remove(0);
    Scan {
        alias: 7,
        source: ScanSource::Projection {
            input: Box::new(atom),
            columns: vec![(
                "value".into(),
                TermMap::Column("c0".into(), TermSpec::plain_literal()),
            )],
            guards: vec![],
            distinct: false,
            native_keys: vec![],
            lexical_keys: vec![],
        },
    }
}

fn policy_projection() -> Scan {
    let mut scan = projected(LogicalSource::Table("items".into()));
    let ScanSource::Projection { input, .. } = &mut scan.source else {
        unreachable!()
    };
    let ScanSource::Projection { input, guards, .. } = &mut input.source else {
        unreachable!()
    };
    guards.push(SqlCond::NativeCmp(
        ColRef::new(input.alias, "TENANT"),
        crate::iq::CmpOp::Eq,
        "policy-'value".into(),
    ));
    scan
}

#[test]
fn policy_guards_validate_original_columns_before_open_and_do_not_widen_outputs() {
    for position in 0..4 {
        for valid in [false, true] {
            let names = if valid {
                vec!["key".into(), "tenant".into()]
            } else {
                vec!["key".into()]
            };
            let mut backend = backend_with(vec![Ok(names)]);
            let result = run_select(&plan(policy_projection(), position), &mut backend);
            assert_eq!(result.is_ok(), valid);
            assert_eq!(backend.opens, usize::from(valid));
            assert_eq!(
                backend.probes,
                vec![Dialect::Postgres.probe_sql(&LogicalSource::Table("items".into()))]
            );
            if valid {
                assert!(backend.sql[0].contains("t5.\"tenant\" = $1"));
                assert!(!backend.sql[0].contains("AS \"TENANT\""));
                assert!(!backend.sql[0].contains("policy-'value"));
            }
        }
    }
}

#[test]
fn policy_guards_revoke_table_restore_and_reject_other_operators_or_aliases() {
    for guard in [
        SqlCond::NativeCmp(
            ColRef::new(99, "TENANT"),
            crate::iq::CmpOp::Eq,
            "private".into(),
        ),
        SqlCond::NativeCmp(
            ColRef::new(5, "TENANT"),
            crate::iq::CmpOp::Ne,
            "private".into(),
        ),
        SqlCond::Cmp(
            ColRef::new(5, "TENANT"),
            crate::iq::CmpOp::Eq,
            "private".into(),
        ),
    ] {
        let mut scan = policy_projection();
        let ScanSource::Projection { input, .. } = &mut scan.source else {
            unreachable!()
        };
        assert!(input.source.distinct_table().is_none());
        let ScanSource::Projection { guards, .. } = &mut input.source else {
            unreachable!()
        };
        guards[0] = guard;
        let mut backend = backend_with(vec![Ok(vec!["key".into(), "tenant".into()])]);
        assert!(run_select(&plan(scan, 0), &mut backend).is_err());
        assert_eq!(backend.opens, 0);
    }
}

#[test]
fn ref_atom_probes_original_leaves_in_every_scan_position_before_open() {
    for position in 0..4 {
        for parent_label in ["label", "wrong"] {
            let mut backend = backend_with(vec![
                Ok(vec!["fk".into()]),
                Ok(vec![parent_label.into(), "k".into(), "tenant".into()]),
            ]);
            let result = run_select(&plan(reference_projection(), position), &mut backend);
            assert_eq!(result.is_ok(), parent_label == "label");
            assert_eq!(backend.opens, usize::from(parent_label == "label"));
            assert_eq!(
                backend.probes,
                vec![
                    Dialect::Postgres.probe_sql(&LogicalSource::Table("child".into())),
                    Dialect::Postgres.probe_sql(&LogicalSource::Table("parent".into())),
                ]
            );
            if parent_label == "label" {
                assert!(
                    backend.sql[0].contains("t4.\"FK\" = t5.\"K\"")
                        && backend.sql[0].contains("t4.\"fk\" AS \"FK\"")
                        && backend.sql[0].contains("t5.\"k\" AS \"K\""),
                    "{}",
                    backend.sql[0]
                );
            }
        }
    }
}

#[test]
fn ref_atom_parameters_follow_core_optional_and_existential_sql_order() {
    assert_parameter_order(reference_projection);
    assert_parameter_order(policy_projection);
}

fn assert_parameter_order(project: fn() -> Scan) {
    for position in 0..4 {
        for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
            let mut p = plan(project(), position);
            let query_column = if position < 2 {
                ColRef::new(7, "value")
            } else {
                p.branches[0].core.push(Scan {
                    alias: 8,
                    source: LogicalSource::Table("outer".into()).into(),
                });
                ColRef::new(8, "prefix")
            };
            // SQL WHERE precedes a nested EXISTS body but follows FROM/LEFT JOIN.
            p.branches[0].where_conds.insert(
                0,
                SqlCond::Cmp(query_column, crate::iq::CmpOp::Eq, "query-'value".into()),
            );
            let e = crate::emit::emit_branch(&p.branches[0], dialect).unwrap();
            let expected = if position < 2 {
                vec!["policy-'value", "query-'value"]
            } else {
                vec!["query-'value", "policy-'value"]
            };
            assert_eq!(e.params, expected, "{position}/{dialect:?}: {}", e.sql);
            assert!(!e.sql.contains("policy-'value") && !e.sql.contains("query-'value"));
        }
    }
}

#[test]
fn ref_atom_inner_source_and_policy_are_in_clone_work_measurement() {
    use crate::plan_measure::clone_root::{measure_compiler_clone_root_v1, CompilerCloneRootV1};
    let small = reference_projection();
    let mut large = small.clone();
    let ScanSource::Projection { input, .. } = &mut large.source else {
        unreachable!()
    };
    let ScanSource::RefAtom { input, .. } = &mut input.source else {
        unreachable!()
    };
    input.where_conds.push(SqlCond::NativeCmp(
        ColRef::new(4, "tenant"),
        crate::iq::CmpOp::Eq,
        "x".repeat(4096),
    ));
    let a = measure_compiler_clone_root_v1(CompilerCloneRootV1::Scan(&small)).unwrap();
    let b = measure_compiler_clone_root_v1(CompilerCloneRootV1::Scan(&large)).unwrap();
    assert!(b.deep_clone_work > a.deep_clone_work);
}
