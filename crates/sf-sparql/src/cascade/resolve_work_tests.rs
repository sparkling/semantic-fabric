use super::*;
use crate::compiler_control::CompileContext;
use sf_core::ir::{Template, TermMap, TermSpec};
use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};

#[test]
fn relation_lookup_preserves_optional_condition_order_and_every_stop() {
    use sf_core::query_control::QueryControl;
    let scan = |alias, name: &str| Scan {
        alias,
        source: sf_core::ir::LogicalSource::Table(name.into()).into(),
    };
    let exists = |alias, name| SqlCond::Exists {
        scans: vec![scan(alias, name)],
        conds: vec![],
    };
    let mut branch = Branch::single(scan(0, "core"));
    branch.where_conds = vec![exists(2, "where-first")];
    branch.opts = vec![crate::iq::OptJoin {
        scan: scan(1, "optional"),
        on: vec![exists(2, "on-second")],
        extra: vec![exists(3, "extra")],
    }];
    struct Stop {
        budget: QueryBudget,
        at: u64,
        cause: QueryControlError,
    }
    impl QueryControl for Stop {
        fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
            self.budget.checkpoint()
        }
        fn consume(
            &self,
            charge: QueryCharge,
            amount: u64,
        ) -> std::result::Result<(), QueryControlError> {
            self.budget.consume(charge, amount)?;
            if self.budget.consumed(QueryCharge::CompilerWork) >= self.at {
                return Err(self.budget.terminate(self.cause));
            }
            Ok(())
        }
        fn terminate(&self, cause: QueryControlError) -> QueryControlError {
            self.budget.terminate(cause)
        }
    }
    let budget = || QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    for alias in [0, 1, 2, 3, 99] {
        let expected = branch
            .relation_scans()
            .into_iter()
            .find(|scan| scan.alias == alias);
        let run = |control: &dyn QueryControl| {
            relation_scan(
                &branch,
                alias,
                BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
            )
        };
        let measured = budget();
        assert_eq!(
            format!("{:?}", run(&measured).unwrap()),
            format!("{expected:?}")
        );
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            for at in 1..=measured.consumed(QueryCharge::CompilerWork) {
                let stopped = Stop {
                    budget: budget(),
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
fn relation_lookup_checks_depth_before_descending() {
    let limit = crate::compile_envelope::algebra::MAX_ALGEBRA_DEPTH_V1;
    for depth in [limit, limit + 1] {
        let mut cond = SqlCond::ExpressionError;
        for _ in 1..depth {
            cond = SqlCond::Not(Box::new(cond));
        }
        let mut branch = Branch::empty();
        branch.where_conds = vec![cond];
        let control = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
        let result = relation_scan(
            &branch,
            1,
            BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(&control))),
        );
        if depth == limit {
            assert!(result.unwrap().is_none());
        } else {
            assert!(matches!(
                result,
                Err(crate::Error::QueryControl(
                    QueryControlError::CompilerEnvelopeExceeded
                ))
            ));
        }
    }
}

#[test]
fn unverified_pooling_is_independent_of_unused_catalogue_observations() {
    use crate::compiler_schema::ColumnTypeUse;
    use sf_sql::{Dialect, TableSchema};
    let schema: Vec<_> = (0..2048)
        .rev()
        .map(|index| TableSchema::new(format!("unused_{index}")))
        .collect();
    for same_source in [false, true] {
        let mut branches = [Branch::empty(), Branch::empty()];
        for (index, branch) in branches.iter_mut().enumerate() {
            let source = if index == 0 || same_source {
                "left"
            } else {
                "right"
            };
            branch.core.push(Scan {
                alias: index,
                source: sf_core::ir::LogicalSource::Table(source.into()).into(),
            });
            branch.bindings.insert(
                "s".into(),
                crate::iq::TermDef::Derived {
                    alias: index,
                    term_map: TermMap::Column("value".into(), TermSpec::plain_literal()),
                },
            );
        }
        let members = branches.iter().collect::<Vec<_>>();
        let run = |schema: &[TableSchema]| {
            let control =
                QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
            let result = super::super::group_pool_type_safety_with_work(
                &members,
                None,
                schema,
                Dialect::Postgres,
                ColumnTypeUse::Unverified,
                BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(&control))),
            )
            .unwrap();
            (result, control.consumed(QueryCharge::CompilerWork))
        };
        let expected = if same_source {
            super::super::PoolTypeSafety::ProvenSafe
        } else {
            super::super::PoolTypeSafety::Unproven
        };
        let empty = run(&[]);
        assert_eq!(empty.0, expected);
        assert_eq!(run(&schema), empty);
    }
}

#[test]
fn controlled_pool_type_lookup_matches_raw_authority_and_exact_budget() {
    use crate::compiler_schema::ColumnTypeUse;
    use sf_sql::{Column, Dialect, TableSchema};
    let branch = |name: &str, alias| {
        let mut branch = Branch::single(Scan {
            alias,
            source: sf_core::ir::LogicalSource::Table(name.into()).into(),
        });
        branch.bindings.insert(
            "s".into(),
            crate::iq::TermDef::Derived {
                alias,
                term_map: TermMap::Column("value".into(), TermSpec::plain_literal()),
            },
        );
        branch
    };
    let budget = |units| QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX));
    for right_type in ["text", "REAL", "float8"] {
        let branches = [branch("left", 1), branch("right", 2)];
        let members = branches.iter().collect::<Vec<_>>();
        let authorities = branches
            .iter()
            .map(PoolSourceAuthority::capture)
            .collect::<Vec<_>>();
        let authorities = authorities.iter().collect::<Vec<_>>();
        let schema = [
            TableSchema {
                columns: vec![Column::new("value", "real", false)],
                ..TableSchema::new("left")
            },
            TableSchema {
                columns: vec![Column::new("value", right_type, false)],
                ..TableSchema::new("right")
            },
        ];
        for use_types in [
            ColumnTypeUse::Unverified,
            ColumnTypeUse::CallerAuthorizedFrozen,
        ] {
            let expected = super::super::group_pool_type_safety_with_source_authority(
                &members,
                &authorities,
                &schema,
                Dialect::Postgres,
                use_types,
            );
            let run = |control: &QueryBudget| {
                super::super::group_pool_type_safety_with_work(
                    &members,
                    Some(&authorities),
                    &schema,
                    Dialect::Postgres,
                    use_types,
                    BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
                )
            };
            let measured = budget(u64::MAX);
            assert_eq!(run(&measured).unwrap(), expected);
            let units = measured.consumed(QueryCharge::CompilerWork);
            assert_eq!(run(&budget(units)).unwrap(), expected);
            assert!(matches!(
                run(&budget(units - 1)),
                Err(crate::Error::QueryControl(
                    QueryControlError::CompilerWorkExceeded
                ))
            ));
        }
    }
}

#[test]
fn sql_type_comparison_preserves_ascii_rules_and_exact_budget_without_folding_copy() {
    let types = [
        "REAL",
        "real",
        "realistic",
        "FLOAT8",
        "double precision",
        "text",
        "INTEGER",
        "",
        "éFLOAT界",
    ];
    let raw_float = |text: &str| {
        let text = text.to_ascii_lowercase();
        text == "real" || text.contains("float") || text.contains("double")
    };
    let budget = |units| QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX));
    for left in types {
        for right in types {
            let expected =
                !left.eq_ignore_ascii_case(right) && (raw_float(left) || raw_float(right));
            let run = |control: &QueryBudget| {
                incompatible_sql_types(
                    left,
                    right,
                    BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
                )
            };
            let measured = budget(u64::MAX);
            assert_eq!(run(&measured).unwrap(), expected, "{left:?} / {right:?}");
            let units = measured.consumed(QueryCharge::CompilerWork);
            assert_eq!(run(&budget(units)).unwrap(), expected);
            assert!(matches!(
                run(&budget(units - 1)),
                Err(crate::Error::QueryControl(
                    QueryControlError::CompilerWorkExceeded
                ))
            ));
        }
    }
}

#[test]
fn borrowed_columns_preserve_scoped_deduplication_and_exact_budget() {
    use crate::iq::{R2rmlGraphScope, TermDef};
    let identifier = TermMap::Template(Template::parse("{id}/{id}").unwrap(), TermSpec::iri());
    let scoped = TermDef::R2rmlBlank {
        term_map: identifier.clone(),
        alias: 1,
        graph: R2rmlGraphScope::Mapped {
            term_map: TermMap::Template(
                Template::parse("{id}/{graph}/{graph}").unwrap(),
                TermSpec::iri(),
            ),
            alias: 1,
        },
    };
    let definition = TermDef::Concat(vec![
        TermDef::Derived {
            term_map: identifier,
            alias: 1,
        },
        scoped.clone(),
        TermDef::Coalesce(Box::new(scoped.clone()), Box::new(scoped)),
    ]);
    let expected = definition.columns();
    let expected: Vec<_> = expected
        .iter()
        .map(|col| (col.alias, col.column.as_ref()))
        .collect();
    let budget = |units| QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX));
    let run = |control: &QueryBudget| {
        columns(
            &definition,
            BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
        )
    };
    let measured = budget(u64::MAX);
    assert_eq!(run(&measured).unwrap(), expected);
    let units = measured.consumed(QueryCharge::CompilerWork);
    assert_eq!(run(&budget(units)).unwrap(), expected);
    assert!(matches!(
        run(&budget(units - 1)),
        Err(crate::Error::QueryControl(
            QueryControlError::CompilerWorkExceeded
        ))
    ));
    let stopped = budget(u64::MAX);
    stopped.terminate(QueryControlError::Cancelled);
    assert!(run(&stopped).is_err());
}

#[test]
fn narrowing_preserves_guards_and_projection_rules_with_exact_budget() {
    use crate::iq::scan::{LexicalKey, LexicalMode};
    let budget = |units| QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX));
    let keep = ["s".to_owned()].into_iter().collect();
    for native in [false, true] {
        for raw in [false, true] {
            let mut branch = Branch::empty();
            for name in ["s", "internal"] {
                branch.bindings.insert(
                    name.into(),
                    crate::iq::TermDef::Derived {
                        alias: 1,
                        term_map: TermMap::Column("id".into(), TermSpec::iri()),
                    },
                );
            }
            branch.core.push(Scan {
                alias: 1,
                source: crate::iq::ScanSource::Projection {
                    input: Box::new(Scan {
                        alias: 2,
                        source: sf_core::ir::LogicalSource::Table("items".into()).into(),
                    }),
                    columns: vec![(
                        "id".into(),
                        TermMap::Column(
                            if raw { "id" } else { "other" }.into(),
                            TermSpec::plain_literal(),
                        ),
                    )],
                    guards: vec![SqlCond::ExpressionError],
                    distinct: true,
                    native_keys: if native {
                        vec![("id".into(), true)]
                    } else {
                        vec![]
                    },
                    lexical_keys: vec![LexicalKey {
                        column: "id".into(),
                        mode: LexicalMode::Iri { base: None },
                    }],
                },
            });
            let mut expected = vec![branch.clone()];
            super::super::narrow_group_for_shared_term_dedup(&mut expected, &keep);
            let run = |control: &QueryBudget| {
                let mut actual = vec![branch.clone()];
                narrow(
                    &mut actual,
                    &keep,
                    BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
                )?;
                Ok::<_, crate::Error>(actual)
            };
            let measured = budget(u64::MAX);
            assert_eq!(
                format!("{:?}", run(&measured).unwrap()),
                format!("{expected:?}")
            );
            let units = measured.consumed(QueryCharge::CompilerWork);
            assert_eq!(
                format!("{:?}", run(&budget(units)).unwrap()),
                format!("{expected:?}")
            );
            assert!(matches!(
                run(&budget(units - 1)),
                Err(crate::Error::QueryControl(
                    QueryControlError::CompilerWorkExceeded
                ))
            ));
            let stopped = budget(u64::MAX);
            stopped.terminate(QueryControlError::Cancelled);
            assert!(run(&stopped).is_err());
        }
    }
}

#[test]
fn fallback_rules_match_raw_with_exact_budgets_and_sticky_stops() {
    let budget = |units| QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX));
    let keep = ["s".to_owned()].into_iter().collect();
    for map in [
        TermMap::Column("id".into(), TermSpec::iri()),
        TermMap::Column(
            "id".into(),
            TermSpec::iri().with_base("http://example.test/"),
        ),
        TermMap::Template(Template::parse("{a}{b}").unwrap(), TermSpec::iri()),
        TermMap::Template(
            Template::parse("{a}{b}").unwrap(),
            TermSpec::plain_literal(),
        ),
    ] {
        let mut branch = Branch::empty();
        branch.bindings.insert(
            "s".into(),
            crate::iq::TermDef::Derived {
                alias: 1,
                term_map: map,
            },
        );
        for branches in [
            vec![branch.clone()],
            vec![branch.clone(), branch.clone()],
            vec![branch.clone(), Branch::empty()],
        ] {
            for needs in [false, true] {
                let expected = if needs {
                    super::super::group_needs_resolved_iri_dedup(&branches, &keep)
                } else {
                    super::super::group_can_fallback_to_shared_term_dedup(&branches, &keep)
                };
                let run = |control: &QueryBudget| {
                    let work =
                        BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control)));
                    if needs {
                        needs_resolved_iri(&branches, &keep, work)
                    } else {
                        can_fallback(&branches, &keep, work)
                    }
                };
                let measured = budget(u64::MAX);
                assert_eq!(run(&measured).unwrap(), expected);
                let units = measured.consumed(QueryCharge::CompilerWork);
                assert_eq!(run(&budget(units)).unwrap(), expected);
                if units > 0 {
                    assert!(matches!(
                        run(&budget(units - 1)),
                        Err(crate::Error::QueryControl(
                            QueryControlError::CompilerWorkExceeded
                        ))
                    ));
                }
                for cause in [
                    QueryControlError::Cancelled,
                    QueryControlError::DeadlineExceeded,
                ] {
                    let stopped = budget(u64::MAX);
                    stopped.terminate(cause);
                    assert!(
                        matches!(run(&stopped), Err(crate::Error::QueryControl(actual)) if actual == cause)
                    );
                }
            }
        }
    }
}
