use super::*;
use sf_core::{ir::TermSpec, Literal};

#[test]
fn controlled_native_decoder_preserves_proof_and_exact_admission() {
    use crate::emit::pg_numeric::{decoder_key, decoder_key_controlled};
    use crate::iq::{scan::LexicalMode, LexicalKey};
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    use sf_sql::source_work::SourceWork;
    let budget = |n| QueryBudget::new(QueryLimits::new(u64::MAX, n, u64::MAX, u64::MAX));
    for (dialect, scalar) in [
        (Dialect::Postgres, NativeScalarKey::PostgresNumeric),
        (Dialect::Postgres, NativeScalarKey::PostgresFloat8),
        (Dialect::MySql, NativeScalarKey::MysqlFloat8),
    ] {
        for code in [Some(XsdTypeCode::Double), Some(XsdTypeCode::String), None] {
            let mut sources = actuals(XsdTypeCode::Double, scalar);
            sources
                .get_mut(&0)
                .unwrap()
                .datatype_columns
                .insert("src".into(), code);
            for name in ["src", "SRC", "missing"] {
                for alias in [0, 9] {
                    let column = ColRef::new(alias, name);
                    let keys = [
                        LexicalMode::Decoded,
                        LexicalMode::Natural,
                        LexicalMode::TypedLiteral {
                            datatype: XsdTypeCode::Double.iri().into(),
                        },
                    ]
                    .map(|mode| LexicalKey {
                        column: name.into(),
                        mode,
                    });
                    let expected = decoder_key(&column, dialect, &sources, &keys);
                    let run = |control: &QueryBudget| {
                        decoder_key_controlled(
                            &column,
                            dialect,
                            &sources,
                            &keys,
                            SourceWork::new(Some(control)),
                        )
                    };
                    let measured = budget(u64::MAX);
                    assert_eq!(run(&measured).unwrap(), expected);
                    let units = measured.consumed(QueryCharge::SourceWork);
                    assert_eq!(measured.consumed(QueryCharge::CompilerWork), 0);
                    assert_eq!(run(&budget(units)).unwrap(), expected);
                    assert!(matches!(
                        run(&budget(units - 1)),
                        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                    ));
                    if dialect == Dialect::MySql && code != Some(XsdTypeCode::Double) {
                        assert_eq!(expected, None, "native width alone is insufficient");
                    }
                }
            }
        }
    }
}

#[test]
fn literal_alias_lookup_cost_is_independent_of_hash_iteration_order() {
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits};
    use sf_sql::source_work::SourceWork;
    for order in [[0, 1, 2], [1, 2, 0], [2, 0, 1]] {
        let modes: Vec<_> = order.into_iter().map(|alias| (alias, Vec::new())).collect();
        for alias in [0, 9] {
            let control = QueryBudget::new(QueryLimits::new(u64::MAX, 3, u64::MAX, u64::MAX));
            let found = crate::emit::literal_roles::modes_for_alias(
                &modes,
                alias,
                SourceWork::new(Some(&control)),
            )
            .unwrap();
            assert_eq!(found.is_some(), alias == 0);
            assert_eq!(control.consumed(QueryCharge::SourceWork), 3);
        }
    }
}

#[test]
fn controlled_literal_roles_preserve_provenance_names_and_exact_admission() {
    use crate::emit::literal_roles::resolved_controlled;
    use crate::iq::{scan::LexicalMode, LexicalKey};
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    use sf_sql::source_work::SourceWork;
    let budget = |n| QueryBudget::new(QueryLimits::new(u64::MAX, n, u64::MAX, u64::MAX));
    for dialect in [Dialect::Postgres, Dialect::MySql, Dialect::Sqlite] {
        for code in [Some(XsdTypeCode::Double), Some(XsdTypeCode::String), None] {
            for name in ["src", "SRC", "missing"] {
                let mut aliases = actuals(XsdTypeCode::Double, NativeScalarKey::PostgresFloat8);
                let source = aliases.get_mut(&0).unwrap();
                source.natural_columns.insert("src".into(), code);
                source.sqlite_columns.insert(
                    "src".into(),
                    sf_sql::backend::SqliteDecode {
                        declared: code,
                        padding: None,
                    },
                );
                let typed = LexicalMode::TypedLiteral {
                    datatype: XsdTypeCode::Double.iri().into(),
                };
                let iri = LexicalMode::Iri {
                    base: Some("urn:base:α".into()),
                };
                let keys: Vec<_> = [
                    typed.clone(),
                    typed.clone(),
                    LexicalMode::Natural,
                    LexicalMode::Decoded,
                    LexicalMode::DecodedWithNatural,
                    iri.clone(),
                ]
                .into_iter()
                .map(|mode| LexicalKey {
                    column: name.into(),
                    mode,
                })
                .collect();
                let mut expected = vec![LexicalMode::DecodedWithNatural, iri];
                if code.is_none() || name == "missing" {
                    expected.push(typed);
                }
                expected.sort();
                let run = |control: &QueryBudget| {
                    resolved_controlled(&keys, dialect, source, SourceWork::new(Some(control))).map(
                        |keys| {
                            keys.into_iter()
                                .map(|key| (key.column, key.mode))
                                .collect::<Vec<_>>()
                        },
                    )
                };
                let expected: Vec<_> = expected
                    .into_iter()
                    .map(|mode| (Box::<str>::from(name), mode))
                    .collect();
                let measured = budget(u64::MAX);
                assert_eq!(run(&measured).unwrap(), expected);
                let units = measured.consumed(QueryCharge::SourceWork);
                assert_eq!(measured.consumed(QueryCharge::CompilerWork), 0);
                assert_eq!(run(&budget(units)).unwrap(), expected);
                assert!(matches!(
                    run(&budget(units - 1)),
                    Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                ));
            }
        }
    }
}

#[test]
fn numeric_inventory_cost_is_independent_of_descriptor_hash_order() {
    use crate::emit::{pg_numeric::distinct_keys_for_projection, BindingView};
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits};
    use sf_sql::source_work::SourceWork;
    let bindings = std::collections::BTreeMap::new();
    let mut expected = None;
    for reverse in [false, true].into_iter().cycle().take(20) {
        let mut sources = ActualColumns::new();
        for index in 0..3 {
            let alias = if reverse { 2 - index } else { index };
            let scalar = if alias == 0 {
                NativeScalarKey::PostgresFloat8
            } else {
                NativeScalarKey::Integer
            };
            sources.insert(
                alias,
                actuals(XsdTypeCode::Double, scalar).remove(&0).unwrap(),
            );
        }
        let control = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
        assert!(distinct_keys_for_projection(
            &BindingView::Direct(&bindings),
            Dialect::Postgres,
            &sources,
            &[],
            SourceWork::new(Some(&control))
        )
        .unwrap()
        .is_empty());
        let units = control.consumed(QueryCharge::SourceWork);
        assert_eq!(*expected.get_or_insert(units), units);
    }
}

#[test]
fn text_numeric_conversion_is_bounded_and_requires_exact_text_provenance() {
    for kind in [
        "double",
        "float",
        "decimal",
        "integer",
        "unsignedLong",
        "negativeInteger",
    ] {
        let datatype = format!("http://www.w3.org/2001/XMLSchema#{kind}");
        for key in [TextKey::Verbatim, TextKey::PostgresCharacter] {
            let sql = text::operand("t0.src", key, &datatype, Promotion::Double).unwrap();
            assert!(sql.contains("COLLATE \"C\""));
            assert!(sql.contains("left(d,1100)"));
            assert!(sql.contains("ord BETWEEN -500 AND 400"));
            assert!(sql.contains("CAST(CASE WHEN valid AND d<>''"));
            assert_eq!(
                sql.contains("bpcharsend"),
                key == TextKey::PostgresCharacter
            );
            Dialect::Postgres
                .emit_via_ast(&format!("SELECT {sql} FROM items t0"))
                .unwrap_or_else(|error| panic!("{error}: {sql}"));
        }
        assert!(text::operand(
            "t0.src",
            TextKey::SqliteCharacter(2),
            &datatype,
            Promotion::Double
        )
        .is_none());
    }
    assert!(text::operand(
        "t0.src",
        TextKey::Verbatim,
        XsdTypeCode::String.iri().as_str(),
        Promotion::Double
    )
    .is_none());
    assert!(text::operand(
        "t0.src",
        TextKey::Verbatim,
        XsdTypeCode::Double.iri().as_str(),
        Promotion::Float
    )
    .is_none());
}

fn actuals(code: XsdTypeCode, scalar: NativeScalarKey) -> ActualColumns {
    let source = LogicalSource::Table("items".into());
    let mut catalog = ColumnCatalog::default();
    catalog
        .insert_live_result(
            &source,
            vec![sf_sql::backend::ResultColumn {
                name: "src".into(),
                natural_datatype: Some(code),
                native_scalar: Some(scalar),
                text_key: None,
                sqlite_decode: None,
            }],
        )
        .unwrap();
    HashMap::from([(0, source_actuals(&source, &catalog))])
}

fn comparison_with(spec: TermSpec, op: crate::iq::CmpOp) -> LiteralComparison {
    LiteralComparison {
        left: LiteralOperand::Column {
            column: ColRef::new(0, "src"),
            spec,
        },
        right: LiteralOperand::Constant(Literal::new_typed_literal(
            "1.1",
            XsdTypeCode::Double.iri(),
        )),
        value_op: Some(op),
    }
}

#[test]
fn value_keys_preserve_width_and_ast_and_bind_only_promoted_constants() {
    for (code, scalar, marker) in [
        (
            XsdTypeCode::Double,
            NativeScalarKey::PostgresFloat4,
            "float4send",
        ),
        (XsdTypeCode::Double, NativeScalarKey::PostgresFloat8, "t0"),
        (
            XsdTypeCode::Integer,
            NativeScalarKey::Integer,
            "DOUBLE PRECISION",
        ),
        (
            XsdTypeCode::Decimal,
            NativeScalarKey::PostgresNumeric,
            "AS JSON",
        ),
    ] {
        let actuals = actuals(code, scalar);
        for op in [
            crate::iq::CmpOp::Eq,
            crate::iq::CmpOp::Ne,
            crate::iq::CmpOp::Lt,
            crate::iq::CmpOp::Le,
            crate::iq::CmpOp::Gt,
            crate::iq::CmpOp::Ge,
        ] {
            let cmp = comparison_with(TermSpec::plain_literal(), op);
            let mut params = vec![];
            let sql = comparison(&cmp, Dialect::Postgres, &actuals, &mut params, &mut 0)
                .unwrap()
                .unwrap();
            assert_eq!(params, ["1.1"]);
            assert!(sql.contains(marker), "{sql}");
            assert!(sql.contains("AS MATERIALIZED"));
            assert!(sql.contains("WHEN l IS NULL OR r IS NULL THEN NULL"));
            Dialect::Postgres
                .emit_via_ast(&format!("SELECT {sql} FROM items t0"))
                .unwrap();
            if scalar == NativeScalarKey::PostgresNumeric {
                assert!(pg_numeric::validates(
                    &SqlCond::LiteralCmp(Box::new(cmp)),
                    &actuals
                ));
            }
        }
    }
}

#[test]
fn matching_datatype_needs_natural_proof_but_double_override_uses_exact_numeric_wire() {
    for scalar in [
        NativeScalarKey::PostgresFloat4,
        NativeScalarKey::PostgresFloat8,
    ] {
        let mut actuals = actuals(XsdTypeCode::Double, scalar);
        actuals
            .get_mut(&0)
            .unwrap()
            .natural_columns
            .insert("src".into(), None);
        for spec in [
            TermSpec::plain_literal(),
            TermSpec::typed_literal(XsdTypeCode::Double.iri().into_owned()),
        ] {
            for op in [crate::iq::CmpOp::Eq, crate::iq::CmpOp::Lt] {
                let cmp = comparison_with(spec.clone(), op);
                assert!(
                    comparison(&cmp, Dialect::Postgres, &actuals, &mut vec![], &mut 0)
                        .unwrap_err()
                        .to_string()
                        .contains("exact native decoder")
                );
            }
        }
    }
    let cmp = comparison_with(
        TermSpec::typed_literal(XsdTypeCode::Double.iri().into_owned()),
        crate::iq::CmpOp::Gt,
    );
    for scalar in [NativeScalarKey::Integer, NativeScalarKey::PostgresNumeric] {
        let mut actuals = self::actuals(XsdTypeCode::Integer, scalar);
        actuals.get_mut(&0).unwrap().natural_columns.clear();
        let sql = comparison(&cmp, Dialect::Postgres, &actuals, &mut vec![], &mut 0)
            .unwrap()
            .unwrap();
        assert_eq!(
            sql.contains("AS JSON"),
            scalar == NativeScalarKey::PostgresNumeric
        );
        assert!(sql.contains("DOUBLE PRECISION"));
        actuals.get_mut(&0).unwrap().scalar_columns.clear();
        assert!(comparison(&cmp, Dialect::Postgres, &actuals, &mut vec![], &mut 0).is_err());
        actuals
            .get_mut(&0)
            .unwrap()
            .scalar_columns
            .insert("src".into(), NativeScalarKey::MysqlDecimal);
        assert!(comparison(&cmp, Dialect::Postgres, &actuals, &mut vec![], &mut 0).is_err());
    }
}

#[test]
fn nonnumeric_override_retains_native_numeric_decoder_obligation() {
    let actuals = actuals(XsdTypeCode::Decimal, NativeScalarKey::PostgresNumeric);
    let cmp = comparison_with(
        TermSpec::typed_literal(XsdTypeCode::String.iri().into_owned()),
        crate::iq::CmpOp::Lt,
    );
    let sql = comparison(&cmp, Dialect::Postgres, &actuals, &mut vec![], &mut 0)
        .unwrap()
        .unwrap();
    assert!(sql.contains("AS JSON"), "{sql}");
    assert!(pg_numeric::validates(
        &SqlCond::LiteralCmp(Box::new(cmp)),
        &actuals
    ));
}

#[test]
fn nonnumeric_unknown_or_incompatible_decoder_cannot_hide_source_errors() {
    let mut actuals = actuals(XsdTypeCode::Decimal, NativeScalarKey::PostgresNumeric);
    actuals.get_mut(&0).unwrap().scalar_columns.clear();
    let cmp = comparison_with(
        TermSpec::typed_literal(XsdTypeCode::String.iri().into_owned()),
        crate::iq::CmpOp::Lt,
    );
    assert!(comparison(&cmp, Dialect::Postgres, &actuals, &mut vec![], &mut 0).is_err());
    actuals
        .get_mut(&0)
        .unwrap()
        .natural_columns
        .insert("src".into(), None);
    assert!(comparison(&cmp, Dialect::Postgres, &actuals, &mut vec![], &mut 0).is_err());
    actuals
        .get_mut(&0)
        .unwrap()
        .text_columns
        .insert("src".into(), TextKey::SqliteCharacter(2));
    assert!(comparison(&cmp, Dialect::Postgres, &actuals, &mut vec![], &mut 0).is_err());
    actuals
        .get_mut(&0)
        .unwrap()
        .text_columns
        .insert("src".into(), TextKey::Verbatim);
    assert!(comparison(&cmp, Dialect::Postgres, &actuals, &mut vec![], &mut 0).is_ok());
}

#[test]
fn float_only_promotion_is_real_unless_natural_rdf_double_participates() {
    let float = sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#float");
    for (code, scalar) in [
        (XsdTypeCode::Integer, NativeScalarKey::Integer),
        (XsdTypeCode::Decimal, NativeScalarKey::PostgresNumeric),
        (XsdTypeCode::Double, NativeScalarKey::PostgresFloat4),
    ] {
        let mut actuals = actuals(code, scalar);
        let mut cmp = comparison_with(TermSpec::plain_literal(), crate::iq::CmpOp::Gt);
        cmp.right = LiteralOperand::Constant(Literal::new_typed_literal("1.1", float.clone()));
        let mut params = vec![];
        let sql = comparison(&cmp, Dialect::Postgres, &actuals, &mut params, &mut 0)
            .unwrap()
            .unwrap();
        if code == XsdTypeCode::Double {
            assert_eq!(params, [f64::from(1.1_f32).to_string()]);
            assert!(sql.contains("float4send"));
            actuals.get_mut(&0).unwrap().datatype_columns.clear();
            assert!(comparison(&cmp, Dialect::Postgres, &actuals, &mut vec![], &mut 0).is_err());
        } else {
            assert_eq!(params, ["1.1"]);
            assert!(sql.contains(" AS REAL)"));
            assert!(!sql.contains("DOUBLE PRECISION"));
        }
        Dialect::Postgres
            .emit_via_ast(&format!("SELECT {sql} FROM items t0"))
            .unwrap();
    }
}

#[test]
fn authored_float_preserves_wire_lexical_rounding_before_double_promotion() {
    let float = sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#float");
    for (code, scalar) in [
        (XsdTypeCode::Integer, NativeScalarKey::Integer),
        (XsdTypeCode::Decimal, NativeScalarKey::PostgresNumeric),
        (XsdTypeCode::Double, NativeScalarKey::PostgresFloat4),
        (XsdTypeCode::Double, NativeScalarKey::PostgresFloat8),
    ] {
        let mut actuals = actuals(code, scalar);
        let cmp = comparison_with(TermSpec::typed_literal(float.clone()), crate::iq::CmpOp::Eq);
        let sql = comparison(&cmp, Dialect::Postgres, &actuals, &mut vec![], &mut 0)
            .unwrap()
            .unwrap();
        assert!(sql.contains(" AS REAL)"));
        assert!(sql.contains("DOUBLE PRECISION"));
        assert_eq!(
            sql.contains("float8send"),
            scalar == NativeScalarKey::PostgresFloat8
        );
        Dialect::Postgres
            .emit_via_ast(&format!("SELECT {sql} FROM items t0"))
            .unwrap();
        actuals.get_mut(&0).unwrap().scalar_columns.clear();
        assert!(comparison(&cmp, Dialect::Postgres, &actuals, &mut vec![], &mut 0).is_err());
    }
}
