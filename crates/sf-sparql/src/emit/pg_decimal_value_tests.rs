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
