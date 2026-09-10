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

fn native_double() -> (LiteralComparison, ColumnCatalog, ActualColumns) {
    native_with(NativeScalarKey::MysqlFloat8)
}

fn native_with(key: NativeScalarKey) -> (LiteralComparison, ColumnCatalog, ActualColumns) {
    let (cmp, _, _) = setup("double", None);
    let source = LogicalSource::Table("items".into());
    let mut catalog = ColumnCatalog::default();
    catalog
        .insert_live_result(
            &source,
            vec![sf_sql::backend::ResultColumn {
                name: "v".into(),
                natural_datatype: Some(XsdTypeCode::Double),
                native_scalar: Some(key),
                text_key: None,
                sqlite_decode: None,
            }],
        )
        .unwrap();
    let actuals = HashMap::from([(0, source_actuals(&source, &catalog))]);
    (cmp, catalog, actuals)
}

#[test]
fn mysql_native_double_value_and_lexical_authority_are_separate() {
    let (cmp, catalog, actuals) = native_double();
    for natural in [false, true] {
        let mut cmp = cmp.clone();
        if natural {
            let LiteralOperand::Column { spec, .. } = &mut cmp.left else {
                unreachable!()
            };
            *spec = TermSpec::plain_literal();
        }
        assert!(!native_literal_key::renderable(&cmp.left, &actuals));
        for reverse in [false, true] {
            let mut cmp = cmp.clone();
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
            assert!(sql.contains("IS NULL THEN 3 ELSE 0"));
            assert!(!sql.contains("AS CHAR"));
            Dialect::MySql
                .emit_via_ast(&format!("SELECT {sql} FROM items t0"))
                .unwrap();
        }
    }
    assert!(!native_literal_key::qualified_source(
        XsdTypeCode::Double,
        Some(NativeScalarKey::MysqlFloat8),
        None
    ));
    assert_eq!(
        natural_literal::source_code(NativeScalarKey::MysqlFloat8),
        None
    );
    assert!(iri_cmp::scalar_lexical(NativeScalarKey::MysqlFloat8, "v", Dialect::MySql).is_ok());
    for kind in ["string", "decimal"] {
        let mut cmp = cmp.clone();
        let LiteralOperand::Column { spec, .. } = &mut cmp.left else {
            unreachable!()
        };
        *spec = TermSpec::typed_literal(sf_core::NamedNode::new_unchecked(format!(
            "http://www.w3.org/2001/XMLSchema#{kind}"
        )));
        assert!(!native_literal_key::renderable(&cmp.left, &actuals));
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
}

#[test]
fn mysql_native_floating_widths_keep_value_and_lexical_authority_separate() {
    for key in [NativeScalarKey::MysqlFloat4, NativeScalarKey::MysqlFloat8] {
        let (mut cmp, catalog, actuals) = native_with(key);
        for kind in ["float", "double"] {
            let LiteralOperand::Column { spec, .. } = &mut cmp.left else {
                unreachable!()
            };
            *spec = TermSpec::typed_literal(sf_core::NamedNode::new_unchecked(format!(
                "http://www.w3.org/2001/XMLSchema#{kind}"
            )));
            assert!(!native_literal_key::renderable(&cmp.left, &actuals));
            let sql = comparison(
                &cmp,
                Dialect::MySql,
                &catalog,
                &actuals,
                &mut vec![],
                &mut 0,
            )
            .unwrap()
            .unwrap();
            Dialect::MySql
                .emit_via_ast(&format!("SELECT {sql} FROM items t0"))
                .unwrap();
            if key == NativeScalarKey::MysqlFloat4 && kind == "double" {
                assert!(sql.contains("WHEN -151 THEN"));
                assert!(sql.contains("SELECT 9 AS precision_digits"));
                assert!(!sql.contains("SELECT 10 AS precision_digits"));
            }
            if key == NativeScalarKey::MysqlFloat8 && kind == "float" {
                assert!(sql.contains("SELECT 17 AS precision_digits"));
                assert!(
                    sql.contains("WHERE CAST(lexical AS DOUBLE) = x")
                        || sql.contains("WHERE CAST(lexical AS DOUBLE)=x")
                );
            }
            let mut revoked = actuals.clone();
            revoked
                .get_mut(&0)
                .unwrap()
                .datatype_columns
                .insert("v".into(), None);
            assert!(comparison(
                &cmp,
                Dialect::MySql,
                &catalog,
                &revoked,
                &mut vec![],
                &mut 0
            )
            .is_err());
        }
        assert_eq!(natural_literal::source_code(key), None);
        assert!(!native_literal_key::qualified_source(
            XsdTypeCode::Double,
            Some(key),
            None
        ));
        assert!(iri_cmp::scalar_lexical(key, "v", Dialect::MySql).is_ok());
        let LiteralOperand::Column { spec, .. } = &mut cmp.left else {
            unreachable!()
        };
        spec.language = Some("en".into());
        assert!(comparison(
            &cmp,
            Dialect::MySql,
            &catalog,
            &actuals,
            &mut vec![],
            &mut 0
        )
        .is_err());
        let maps = sf_mapping::parse_r2rml(
            r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
          <#m> rr:logicalTable [rr:tableName "items"]; rr:subject <http://ex/s>;
          rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:column "v"]]."#,
        )
        .unwrap();
        let mut plan = crate::parse_and_translate(
            "SELECT ?o WHERE { ?s <http://ex/p> ?o }",
            &maps,
            Dialect::MySql,
        )
        .unwrap();
        assert_eq!(
            subplan_actuals(&plan, Dialect::MySql, &catalog).scalar_columns["c0"],
            key
        );
        let branch = &plan.prepared_branches()[0];
        assert_eq!(
            ref_atom::actuals(branch, &branch.projection(), Dialect::MySql, &catalog)
                .scalar_columns["c0"],
            key
        );
        plan.branches.push(plan.branches[0].clone());
        assert!(!subplan_actuals(&plan, Dialect::MySql, &catalog)
            .scalar_columns
            .contains_key("c0"));
    }
}

#[test]
fn mysql_native_double_requires_independent_retained_facts() {
    let (cmp, catalog, actuals) = native_double();
    for code in [None, Some(None), Some(Some(XsdTypeCode::Decimal))] {
        let mut actuals = actuals.clone();
        let facts = &mut actuals.get_mut(&0).unwrap().datatype_columns;
        facts.remove("v");
        if let Some(code) = code {
            facts.insert("v".into(), code);
        }
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
    for key in [
        None,
        Some(NativeScalarKey::PostgresFloat8),
        Some(NativeScalarKey::PostgresFloat4),
    ] {
        let mut actuals = actuals.clone();
        let facts = &mut actuals.get_mut(&0).unwrap().scalar_columns;
        facts.remove("v");
        if let Some(key) = key {
            facts.insert("v".into(), key);
        }
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
}

#[test]
fn mysql_native_double_projection_retains_but_union_revokes_value_authority() {
    let (_, catalog, _) = native_double();
    let maps = sf_mapping::parse_r2rml(
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
      <#m> rr:logicalTable [rr:tableName "items"]; rr:subject <http://ex/s>;
      rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:column "v"]]."#,
    )
    .unwrap();
    let mut plan = crate::parse_and_translate(
        "SELECT ?o WHERE { ?s <http://ex/p> ?o }",
        &maps,
        Dialect::MySql,
    )
    .unwrap();
    let single = subplan_actuals(&plan, Dialect::MySql, &catalog);
    assert_eq!(single.scalar_columns["c0"], NativeScalarKey::MysqlFloat8);
    assert_eq!(single.datatype_columns["c0"], Some(XsdTypeCode::Double));
    let branch = &plan.prepared_branches()[0];
    let raw = ref_atom::actuals(branch, &branch.projection(), Dialect::MySql, &catalog);
    assert_eq!(raw.scalar_columns["c0"], NativeScalarKey::MysqlFloat8);
    plan.branches.push(plan.branches[0].clone());
    let union = subplan_actuals(&plan, Dialect::MySql, &catalog);
    // Even same-width DOUBLE(M,D) arms can impose UNION display rounding. No
    // positional lexical/decoder normalization has been qualified here yet.
    assert!(!union.scalar_columns.contains_key("c0"));
    let cmp = LiteralComparison {
        left: LiteralOperand::Column {
            column: ColRef::new(0, "c0"),
            spec: TermSpec::plain_literal(),
        },
        right: LiteralOperand::Constant(Literal::new_typed_literal("1", XsdTypeCode::Double.iri())),
        value_op: Some(crate::iq::CmpOp::Eq),
    };
    assert!(comparison(
        &cmp,
        Dialect::MySql,
        &catalog,
        &HashMap::from([(0, union)]),
        &mut vec![],
        &mut 0
    )
    .is_err());
}
