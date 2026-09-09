use super::*;
use sf_core::{datatype::XsdTypeCode, ir::TermSpec, Literal};

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
fn catalog(
    text: Option<TextKey>,
    scalar: Option<NativeScalarKey>,
    code: Option<XsdTypeCode>,
) -> (ColumnCatalog, ActualColumns) {
    let source = LogicalSource::Table("items".into());
    let mut catalog = ColumnCatalog::default();
    catalog
        .insert_live_result(
            &source,
            vec![sf_sql::backend::ResultColumn {
                name: "src".into(),
                natural_datatype: code,
                text_key: text,
                native_scalar: scalar,
                sqlite_decode: None,
            }],
        )
        .unwrap();
    let actuals = HashMap::from([(0, source_actuals(&source, &catalog))]);
    (catalog, actuals)
}

#[test]
fn full_digit_mysql_roundtrip_and_decoder_authority() {
    for kind in [
        "decimal",
        "integer",
        "byte",
        "unsignedLong",
        "positiveInteger",
    ] {
        for (text, scalar, code) in [
            (Some(TextKey::Verbatim), None, Some(XsdTypeCode::String)),
            (
                None,
                Some(NativeScalarKey::Integer),
                Some(XsdTypeCode::Integer),
            ),
            (
                None,
                Some(NativeScalarKey::MysqlDecimal),
                Some(XsdTypeCode::Decimal),
            ),
        ] {
            let (catalog, actuals) = catalog(text, scalar, code);
            let mut params = vec![];
            let sql = comparison(
                &cmp(kind),
                Dialect::MySql,
                &catalog,
                &actuals,
                &mut params,
                &mut 0,
            )
            .unwrap()
            .unwrap();
            assert_eq!(params, ["1.00"]);
            let rendered = Dialect::MySql
                .emit_via_ast(&format!("SELECT {sql} FROM items t0"))
                .unwrap_or_else(|e| panic!("{e}: {sql}"));
            assert!(rendered.contains("LIMIT 18446744073709551615"));
            assert!(
                rendered.contains("CAST(d AS BINARY)")
                    || kind == "decimal"
                    || kind == "integer"
                    || kind == "positiveInteger"
            );
            assert!(!rendered.contains("AS DOUBLE"));
            assert!(!rendered.contains("AS DECIMAL(65"));
        }
    }
    for (text, scalar) in [
        (None, None),
        (Some(TextKey::PostgresCharacter), None),
        (None, Some(NativeScalarKey::PostgresNumeric)),
    ] {
        let (catalog, actuals) = catalog(text, scalar, Some(XsdTypeCode::Decimal));
        assert!(comparison(
            &cmp("decimal"),
            Dialect::MySql,
            &catalog,
            &actuals,
            &mut vec![],
            &mut 0
        )
        .is_err());
    }
    let (missing_catalog, missing_actuals) = catalog(None, Some(NativeScalarKey::Integer), None);
    assert!(comparison(
        &cmp("integer"),
        Dialect::MySql,
        &missing_catalog,
        &missing_actuals,
        &mut vec![],
        &mut 0
    )
    .is_err());
    let (catalog, mut actuals) = catalog(
        None,
        Some(NativeScalarKey::Integer),
        Some(XsdTypeCode::Integer),
    );
    actuals
        .get_mut(&0)
        .unwrap()
        .datatype_columns
        .insert("src".into(), None);
    assert!(comparison(
        &cmp("unsignedLong"),
        Dialect::MySql,
        &catalog,
        &actuals,
        &mut vec![],
        &mut 0
    )
    .is_err());
    actuals
        .get_mut(&0)
        .unwrap()
        .datatype_columns
        .insert("src".into(), Some(XsdTypeCode::Integer));
    actuals
        .get_mut(&0)
        .unwrap()
        .natural_columns
        .insert("src".into(), None);
    assert!(comparison(
        &cmp("integer"),
        Dialect::MySql,
        &catalog,
        &actuals,
        &mut vec![],
        &mut 0
    )
    .is_err());
}

#[test]
fn mysql_w3c_boolean_validation_survives_numeric_expression_error() {
    let (catalog, actuals) = catalog(
        None,
        Some(NativeScalarKey::Integer),
        Some(XsdTypeCode::Boolean),
    );
    let comparison = cmp("boolean");
    let sql = operand(&comparison.left, &catalog, &actuals, &mut vec![], &mut 0).unwrap();
    assert!(sql.contains("JSON_EXTRACT"));
    assert!(sql.contains("NULLIF(LENGTH(v),LENGTH(v))"));
    let condition = SqlCond::LiteralCmp(Box::new(comparison));
    let policy = SqlCond::NativeCmp(
        ColRef::new(0, "tenant"),
        crate::iq::CmpOp::Eq,
        "allowed".into(),
    );
    let sql = natural_literal::authorized_conjunction(
        &[&condition, &policy],
        Dialect::MySql,
        &catalog,
        &actuals,
        &mut vec![],
        &mut 0,
    )
    .unwrap()
    .unwrap();
    assert!(sql.starts_with("CASE WHEN (t0.`tenant` = ?) THEN"));
}

#[test]
fn mysql_exact_preserves_natural_obligations_and_constant_binding_order() {
    let (catalog, actuals) = catalog(
        None,
        Some(NativeScalarKey::Integer),
        Some(XsdTypeCode::Integer),
    );
    for (kind, invalid) in [
        ("integer", true),
        ("unsignedLong", false),
        ("decimal", false),
    ] {
        let sql = operand(&cmp(kind).left, &catalog, &actuals, &mut vec![], &mut 0).unwrap();
        assert_eq!(sql.contains("JSON_EXTRACT"), invalid, "{sql}");
        assert!(sql.contains("DECIMAL(20, 0)"));
    }
    let mut cmp = cmp("decimal");
    cmp.left = LiteralOperand::Constant(Literal::new_typed_literal(
        "9".repeat(1500),
        XsdTypeCode::Integer.iri(),
    ));
    let mut params = vec![];
    let sql = comparison(
        &cmp,
        Dialect::MySql,
        &catalog,
        &actuals,
        &mut params,
        &mut 0,
    )
    .unwrap()
    .unwrap();
    assert_eq!(params, ["9".repeat(1500), "1.00".into()]);
    // '?' inside regex literals is not an anonymous parameter.
    assert_eq!(sql.matches("CONVERT(? USING utf8mb4)").count(), 2);
    for kind in ["float", "double"] {
        cmp.left = LiteralOperand::Constant(Literal::new_typed_literal(
            "1.00",
            sf_core::NamedNode::new_unchecked(format!("http://www.w3.org/2001/XMLSchema#{kind}")),
        ));
        assert!(comparison(
            &cmp,
            Dialect::MySql,
            &catalog,
            &actuals,
            &mut vec![],
            &mut 0
        )
        .unwrap()
        .is_none());
    }
}

#[test]
fn mysql_natural_value_policy_precedes_validation_and_offline_is_unchanged() {
    let (catalog, actuals) = catalog(
        None,
        Some(NativeScalarKey::Integer),
        Some(XsdTypeCode::Integer),
    );
    let policy = SqlCond::NativeCmp(
        ColRef::new(0, "tenant"),
        crate::iq::CmpOp::Eq,
        "allowed".into(),
    );
    let condition = SqlCond::LiteralCmp(Box::new(cmp("integer")));
    let mut params = vec![];
    let sql = natural_literal::authorized_conjunction(
        &[&condition, &policy],
        Dialect::MySql,
        &catalog,
        &actuals,
        &mut params,
        &mut 0,
    )
    .unwrap()
    .unwrap();
    assert!(sql.starts_with("CASE WHEN (t0.`tenant` = ?) THEN"), "{sql}");
    assert_eq!(params, ["allowed", "1.00"]);
    assert!(sql.contains("JSON_EXTRACT"));
    let sql = literal_cmp::render(
        &cmp("decimal"),
        Dialect::MySql,
        &ColumnCatalog::default(),
        &ActualColumns::new(),
        &mut vec![],
        &mut 0,
    )
    .unwrap();
    assert!(!sql.contains("__sf_exact_"));
    let (catalog, actuals) = self::catalog(None, None, None);
    assert!(literal_cmp::render(
        &cmp("decimal"),
        Dialect::MySql,
        &catalog,
        &actuals,
        &mut vec![],
        &mut 0
    )
    .is_err());
}
