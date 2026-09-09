use super::*;
use sf_core::{ir::TermSpec, Literal};

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
