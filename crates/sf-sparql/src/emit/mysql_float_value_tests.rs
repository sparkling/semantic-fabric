use super::*;
use sf_core::{datatype::XsdTypeCode, ir::TermSpec, Literal};

fn setup(kind: &str, key: Option<TextKey>) -> (LiteralComparison, ColumnCatalog, ActualColumns) {
    let source = LogicalSource::Table("items".into());
    let mut catalog = ColumnCatalog::default();
    catalog
        .insert_live_result(
            &source,
            vec![sf_sql::backend::ResultColumn {
                name: "v".into(),
                natural_datatype: Some(XsdTypeCode::String),
                text_key: key,
                native_scalar: None,
                sqlite_decode: None,
            }],
        )
        .unwrap();
    let actuals = HashMap::from([(0, source_actuals(&source, &catalog))]);
    let cmp = LiteralComparison {
        left: LiteralOperand::Column {
            column: ColRef::new(0, "v"),
            spec: TermSpec::typed_literal(sf_core::NamedNode::new_unchecked(format!(
                "http://www.w3.org/2001/XMLSchema#{kind}"
            ))),
        },
        right: LiteralOperand::Constant(Literal::new_typed_literal("1", XsdTypeCode::Double.iri())),
        value_op: Some(crate::iq::CmpOp::Eq),
    };
    (cmp, catalog, actuals)
}

#[test]
fn mysql_floating_ast_and_anonymous_bind_order() {
    for kind in ["float", "double", "decimal", "integer", "byte"] {
        let (mut cmp, catalog, actuals) = setup(kind, Some(TextKey::Verbatim));
        for reverse in [false, true] {
            if reverse {
                std::mem::swap(&mut cmp.left, &mut cmp.right);
            }
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
            assert_eq!(params, ["1"]);
            let ast = Dialect::MySql
                .emit_via_ast(&format!("SELECT {sql} FROM items t0"))
                .unwrap_or_else(|e| panic!("{e}: {sql}"));
            assert!(ast.contains("JSON_EXTRACT"));
            assert!(ast.contains("LIMIT 18446744073709551615"));
            assert!(!ast.contains("LOG2"));
            assert_eq!(ast.matches('?').count(), sql.matches('?').count());
        }
    }
}

#[test]
fn mysql_floating_rejects_missing_foreign_and_revoked_authority() {
    for key in [None, Some(TextKey::PostgresCharacter)] {
        let (cmp, catalog, actuals) = setup("float", key);
        assert!(comparison(
            &cmp,
            Dialect::MySql,
            &catalog,
            &actuals,
            &mut vec![],
            &mut 0
        )
        .is_err());
    }
    let (cmp, catalog, mut actuals) = setup("float", Some(TextKey::Verbatim));
    actuals
        .get_mut(&0)
        .unwrap()
        .datatype_columns
        .insert("v".into(), None);
    assert!(comparison(
        &cmp,
        Dialect::MySql,
        &catalog,
        &actuals,
        &mut vec![],
        &mut 0
    )
    .is_err());
    assert!(comparison(
        &cmp,
        Dialect::Postgres,
        &catalog,
        &actuals,
        &mut vec![],
        &mut 0
    )
    .unwrap()
    .is_none());
}

#[test]
fn mysql_floating_boundaries_are_exact_decimal_integers() {
    assert_eq!(
        round::overflow(false),
        "340282356779733661637539395458142568448"
    );
    let double = round::overflow(true);
    assert_eq!(double.len(), 309);
    assert_eq!(double.parse::<f64>().unwrap(), f64::INFINITY);
    assert_eq!(round::power_digits(5, 150).len(), 105);
    assert_eq!(round::power_digits(2, 24), "16777216");
}

#[test]
fn mysql_floating_nonnumeric_boolean_keeps_validation_and_policy() {
    let (mut cmp, _, _) = setup("boolean", None);
    let source = LogicalSource::Table("items".into());
    let mut catalog = ColumnCatalog::default();
    catalog
        .insert_live_result(
            &source,
            vec![sf_sql::backend::ResultColumn {
                name: "v".into(),
                natural_datatype: Some(XsdTypeCode::Boolean),
                native_scalar: Some(NativeScalarKey::Integer),
                text_key: None,
                sqlite_decode: None,
            }],
        )
        .unwrap();
    let actuals = HashMap::from([(0, source_actuals(&source, &catalog))]);
    for invalid in ["NaN", "inf"] {
        cmp.right = LiteralOperand::Constant(Literal::new_typed_literal(
            invalid,
            XsdTypeCode::Double.iri(),
        ));
        for reverse in [false, true] {
            let mut cmp = cmp.clone();
            if reverse {
                std::mem::swap(&mut cmp.left, &mut cmp.right)
            }
            let condition = SqlCond::LiteralCmp(Box::new(cmp));
            let policy = SqlCond::NativeCmp(
                ColRef::new(0, "tenant"),
                crate::iq::CmpOp::Eq,
                "allowed".into(),
            );
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
            assert!(sql.starts_with("CASE WHEN (t0."));
            assert_eq!(params, ["allowed"]);
            assert!(sql.contains("__sf_ieee_checked"));
            assert!(sql.contains("NULLIF(LENGTH(v),LENGTH(v))"));
            assert!(sql.contains("JSON_EXTRACT"));
        }
    }
}
