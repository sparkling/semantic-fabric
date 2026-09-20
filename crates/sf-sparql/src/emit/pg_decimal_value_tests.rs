use super::*;
use sf_core::{datatype::XsdTypeCode, ir::TermSpec, Literal};

#[test]
fn source_lexical_inventory_matches_compiler_oracle_and_exact_budget() {
    use crate::cascade::distinct_scan::{binding_lexical_keys_controlled, lexical_keys};
    use crate::iq::{Branch, R2rmlGraphScope, TermDef};
    use sf_core::ir::{Template, TermMap};
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    use sf_sql::source_work::SourceWork;
    let mut branch = Branch::empty();
    for (i, spec) in [
        TermSpec::iri(),
        TermSpec::plain_literal(),
        TermSpec::blank_node(),
        TermSpec::typed_literal(XsdTypeCode::String.iri().into()),
    ]
    .into_iter()
    .enumerate()
    {
        branch.bindings.insert(
            format!("column{i}"),
            TermDef::Derived {
                alias: i % 2,
                term_map: TermMap::Column("shared".into(), spec.clone()),
            },
        );
        branch.bindings.insert(
            format!("template{i}"),
            TermDef::Derived {
                alias: 0,
                term_map: TermMap::Template(Template::parse("urn:{shared}/{other}").unwrap(), spec),
            },
        );
    }
    branch.bindings.insert(
        "graph".into(),
        TermDef::R2rmlBlank {
            alias: 0,
            term_map: TermMap::Column("node".into(), TermSpec::blank_node()),
            graph: R2rmlGraphScope::Mapped {
                alias: 0,
                term_map: TermMap::Column("graph".into(), TermSpec::iri()),
            },
        },
    );
    let budget = |n| QueryBudget::new(QueryLimits::new(u64::MAX, n, u64::MAX, u64::MAX));
    for alias in 0..=2 {
        let run = |control: &QueryBudget| {
            binding_lexical_keys_controlled(
                branch.bindings.values(),
                alias,
                SourceWork::new(Some(control)),
            )
        };
        let measured = budget(u64::MAX);
        let pairs = |keys: Vec<crate::iq::LexicalKey>| {
            keys.into_iter()
                .map(|k| (k.column, k.mode))
                .collect::<Vec<_>>()
        };
        let expected = pairs(lexical_keys(&branch, alias));
        assert_eq!(pairs(run(&measured).unwrap()), expected);
        assert_eq!(measured.consumed(QueryCharge::CompilerWork), 0);
        let units = measured.consumed(QueryCharge::SourceWork);
        assert_eq!(pairs(run(&budget(units)).unwrap()), expected);
        assert!(matches!(
            run(&budget(units - 1)),
            Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
        ));
    }
}

#[test]
fn source_lexical_inventory_handles_deep_unknown_recipes_iteratively() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            use crate::iq::TermDef;
            use sf_core::ir::TermMap;
            use sf_sql::source_work::SourceWork;
            let mut definition = TermDef::Derived {
                alias: 0,
                term_map: TermMap::Column("key".into(), TermSpec::iri()),
            };
            for _ in 0..4096 {
                definition = TermDef::Concat(vec![definition]);
            }
            let keys = crate::cascade::distinct_scan::binding_lexical_keys_controlled(
                std::iter::once(&definition),
                0,
                SourceWork::new(None),
            )
            .unwrap();
            assert!(
                keys.is_empty(),
                "unknown compound consumer must not gain lexical proof"
            );
            while let TermDef::Concat(mut parts) = definition {
                definition = parts.pop().unwrap();
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn source_lexical_inventory_preserves_terminal_reason_at_each_charge() {
    use crate::iq::TermDef;
    use sf_core::ir::TermMap;
    use sf_core::query_control::{
        QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Stop {
        budget: QueryBudget,
        calls: AtomicUsize,
        at: usize,
        reason: QueryControlError,
    }
    impl QueryControl for Stop {
        fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
            self.budget.checkpoint()
        }
        fn consume(
            &self,
            kind: QueryCharge,
            units: u64,
        ) -> std::result::Result<(), QueryControlError> {
            self.budget.consume(kind, units)?;
            if self.calls.fetch_add(1, Ordering::Relaxed) + 1 == self.at {
                self.budget.terminate(self.reason);
            }
            Ok(())
        }
        fn terminate(&self, reason: QueryControlError) -> QueryControlError {
            self.budget.terminate(reason)
        }
    }
    let make = |at, reason| Stop {
        budget: QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX)),
        calls: AtomicUsize::new(0),
        at,
        reason,
    };
    let definition = TermDef::Derived {
        alias: 0,
        term_map: TermMap::Column(
            "key".into(),
            TermSpec::typed_literal(XsdTypeCode::String.iri().into()),
        ),
    };
    let run = |control: &Stop| {
        crate::cascade::distinct_scan::binding_lexical_keys_controlled(
            std::iter::once(&definition),
            0,
            sf_sql::source_work::SourceWork::new(Some(control)),
        )
    };
    let measured = make(usize::MAX, QueryControlError::Cancelled);
    run(&measured).unwrap();
    for reason in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for at in 1..=measured.calls.load(Ordering::Relaxed) {
            let stopped = make(at, reason);
            assert!(
                matches!(run(&stopped), Err(Error::QueryControl(actual)) if actual == reason),
                "charge {at}"
            );
            assert_eq!(stopped.checkpoint(), Err(reason));
            assert_eq!(stopped.budget.consumed(QueryCharge::CompilerWork), 0);
        }
    }
}

#[test]
fn hidden_native_identity_and_order_match_explicit_binding_overlay() {
    use crate::emit::{emit_branch_binding_view, emit_branch_with, BindingView, BranchModifiers};
    use crate::iq::{Branch, OrderKey, Scan, TermDef};
    use sf_core::ir::TermMap;
    use sf_sql::source_work::SourceWork;
    for (dialect, scalar) in [
        (Dialect::Postgres, NativeScalarKey::PostgresNumeric),
        (Dialect::Postgres, NativeScalarKey::PostgresFloat8),
        (Dialect::MySql, NativeScalarKey::MysqlFloat8),
        (Dialect::Sqlite, NativeScalarKey::Integer),
    ] {
        for distinct in [false, true] {
            for order in [false, true] {
                let source = LogicalSource::Table("items".into());
                let mut b = Branch::single(Scan {
                    alias: 0,
                    source: source.clone().into(),
                });
                b.distinct = distinct;
                b.bindings.insert(
                    "visible".into(),
                    TermDef::Const(Literal::new_simple_literal("constant").into()),
                );
                if order {
                    b.order.push(OrderKey {
                        var: "hidden".into(),
                        descending: true,
                        expr: None,
                    });
                }
                let overlay = [(
                    "hidden".into(),
                    TermDef::Derived {
                        alias: 0,
                        term_map: TermMap::Column(
                            "src".into(),
                            TermSpec::typed_literal(if dialect == Dialect::Sqlite {
                                XsdTypeCode::Double.iri().into()
                            } else {
                                XsdTypeCode::String.iri().into()
                            }),
                        ),
                    },
                )]
                .into();
                let view = BindingView::merged(&b.bindings, Some(&overlay), SourceWork::new(None))
                    .unwrap();
                let mut owned = b.clone();
                owned.bindings.extend(overlay.clone());
                let mut catalog = ColumnCatalog::default();
                catalog
                    .insert_live_result(
                        &source,
                        vec![sf_sql::backend::ResultColumn {
                            name: "src".into(),
                            natural_datatype: Some(if scalar == NativeScalarKey::PostgresNumeric {
                                XsdTypeCode::Decimal
                            } else {
                                XsdTypeCode::Double
                            }),
                            native_scalar: Some(scalar),
                            text_key: None,
                            sqlite_decode: (dialect == Dialect::Sqlite).then_some(
                                sf_sql::backend::SqliteDecode {
                                    declared: Some(XsdTypeCode::Double),
                                    padding: None,
                                },
                            ),
                        }],
                    )
                    .unwrap();
                let expected = emit_branch_with(&owned, dialect, &catalog).unwrap();
                let actual = crate::exec_core::block_on(emit_branch_binding_view(
                    &b,
                    &view,
                    dialect,
                    &catalog,
                    BranchModifiers::stored(&b),
                    SourceWork::new(None),
                ))
                .unwrap();
                assert_eq!(actual.sql, expected.sql);
                assert_eq!(actual.metadata_sql, expected.metadata_sql);
                assert_eq!(actual.projection, expected.projection);
                assert_eq!(actual.params, expected.params);
                assert_eq!(actual.sqlite_lexical_keys, expected.sqlite_lexical_keys);
                assert_eq!(actual.sqlite_character_keys, expected.sqlite_character_keys);
                assert_eq!(actual.projection, [ColRef::new(0, "src")]);
                if distinct {
                    assert!(
                        actual.sql.contains("PARTITION BY"),
                        "{dialect:?}/{scalar:?}: {}",
                        actual.sql
                    );
                }
                assert!(!b.bindings.contains_key("hidden"));
            }
        }
    }
}

fn cmp(kind: &str) -> LiteralComparison {
    LiteralComparison {
        left: LiteralOperand::Column {
            column: ColRef::new(0, "src"),
            spec: TermSpec::typed_literal(sf_core::NamedNode::new_unchecked(format!(
                "http://www.w3.org/2001/XMLSchema#{kind}"
            ))),
        },
        right: LiteralOperand::Constant(Literal::new_typed_literal(
            "1.00",
            XsdTypeCode::Decimal.iri(),
        )),
        value_op: Some(crate::iq::CmpOp::Lt),
    }
}

fn actuals(text: Option<TextKey>, scalar: Option<NativeScalarKey>) -> ActualColumns {
    let source = LogicalSource::Table("items".into());
    let mut catalog = ColumnCatalog::default();
    catalog
        .insert_live_result(
            &source,
            vec![sf_sql::backend::ResultColumn {
                name: "src".into(),
                natural_datatype: Some(XsdTypeCode::Decimal),
                text_key: text,
                native_scalar: scalar,
                sqlite_decode: None,
            }],
        )
        .unwrap();
    HashMap::from([(0, source_actuals(&source, &catalog))])
}

#[test]
fn offline_branch_rendering_is_not_missing_live_decoder_authority() {
    let maps = sf_mapping::parse_r2rml(r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
      <#m> rr:logicalTable [rr:tableName "items"]; rr:subject <http://ex/s>;
      rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:column "src"; rr:datatype <http://www.w3.org/2001/XMLSchema#decimal>]]."#).unwrap();
    let plan = crate::parse_and_translate(
        "SELECT ?o WHERE { ?s <http://ex/p> ?o FILTER(?o < 1) }",
        &maps,
        Dialect::Postgres,
    )
    .unwrap();
    let branches = plan.prepared_branches();
    let branch = &branches[0];
    let sql = emit_branch(branch, Dialect::Postgres).unwrap().sql;
    assert!(!sql.contains("__sf_exact_values"));
    let mut catalog = ColumnCatalog::default();
    catalog.insert(&maps[0].source, vec!["src".into()]);
    assert!(emit_branch_with(branch, Dialect::Postgres, &catalog).is_ok());
    catalog
        .insert_live_result(
            &maps[0].source,
            vec![sf_sql::backend::ResultColumn {
                name: "src".into(),
                natural_datatype: None,
                text_key: None,
                native_scalar: None,
                sqlite_decode: None,
            }],
        )
        .unwrap();
    assert!(
        emit_branch_with(branch, Dialect::Postgres, &catalog).is_err(),
        "live names without decoder metadata cannot inherit offline authority"
    );
}

#[test]
fn full_digit_comparison_roundtrips_and_preserves_decoder_authority() {
    for kind in [
        "decimal",
        "integer",
        "positiveInteger",
        "unsignedLong",
        "byte",
    ] {
        for (text, scalar, marker) in [
            (Some(TextKey::Verbatim), None, "COLLATE \"C\""),
            (Some(TextKey::PostgresCharacter), None, "bpcharsend"),
            (None, Some(NativeScalarKey::Integer), "AS TEXT"),
            (None, Some(NativeScalarKey::PostgresNumeric), "AS JSON"),
        ] {
            let mut params = vec![];
            let value =
                operand(&cmp(kind).left, &actuals(text, scalar), &mut vec![], &mut 0).unwrap();
            Dialect::Postgres
                .emit_via_ast(&format!("SELECT {value} FROM items t0"))
                .unwrap_or_else(|e| panic!("operand: {e}: {value}"));
            let sql = comparison(
                &cmp(kind),
                Dialect::Postgres,
                &actuals(text, scalar),
                &mut params,
                &mut 0,
            )
            .unwrap()
            .unwrap();
            assert_eq!(params, ["1.00"]);
            assert!(sql.contains(marker));
            assert!(sql.contains("__sf_exact_values AS MATERIALIZED"));
            assert!(!sql.contains("AS NUMERIC"));
            assert!(!sql.contains("trim_scale"));
            assert!(!sql.contains("1100"));
            Dialect::Postgres
                .emit_via_ast(&format!("SELECT {sql} FROM items t0"))
                .unwrap_or_else(|e| panic!("{e}: {sql}"));
        }
    }
    for (text, scalar) in [
        (None, None),
        (Some(TextKey::SqliteCharacter(2)), None),
        (None, Some(NativeScalarKey::PostgresFloat8)),
        (None, Some(NativeScalarKey::MysqlDecimal)),
    ] {
        assert!(comparison(
            &cmp("integer"),
            Dialect::Postgres,
            &actuals(text, scalar),
            &mut vec![],
            &mut 0
        )
        .is_err());
    }
}

#[test]
fn exact_constants_and_nonnumeric_operand_retain_full_lexicals_and_obligations() {
    let mut cmp = cmp("decimal");
    cmp.left = LiteralOperand::Constant(Literal::new_typed_literal(
        "9".repeat(1500),
        XsdTypeCode::Integer.iri(),
    ));
    let mut params = vec![];
    let sql = comparison(
        &cmp,
        Dialect::Postgres,
        &ActualColumns::new(),
        &mut params,
        &mut 0,
    )
    .unwrap()
    .unwrap();
    assert_eq!(params[0].len(), 1500);
    assert!(sql.contains("ROW(pg_catalog.length(li),li,lf)"));
    let nul =
        LiteralOperand::Constant(Literal::new_typed_literal("\0", XsdTypeCode::Decimal.iri()));
    let mut params = vec![];
    assert_eq!(
        operand(&nul, &ActualColumns::new(), &mut params, &mut 0).unwrap(),
        "CAST(NULL AS TEXT)"
    );
    assert!(params.is_empty());
    cmp.left = LiteralOperand::Column {
        column: ColRef::new(0, "src"),
        spec: TermSpec::typed_literal(XsdTypeCode::String.iri().into()),
    };
    let sql = comparison(
        &cmp,
        Dialect::Postgres,
        &actuals(None, Some(NativeScalarKey::PostgresNumeric)),
        &mut vec![],
        &mut 0,
    )
    .unwrap()
    .unwrap();
    assert!(sql.contains("AS JSON"));
    assert!(sql.contains("NULLIF(pg_catalog.length(v),pg_catalog.length(v))"));
}
