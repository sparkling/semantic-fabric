//! Exhaustive qualified native natural keys; datatype knowledge alone is insufficient.
use super::*;
use crate::iq::literal_cmp::LiteralOperand;
use sf_core::datatype::XsdTypeCode;

pub(super) fn qualified_source(
    code: XsdTypeCode,
    scalar: Option<NativeScalarKey>,
    text: Option<TextKey>,
) -> bool {
    use NativeScalarKey::*;
    use XsdTypeCode as X;
    match code {
        X::String => text.is_some(),
        X::Integer => scalar == Some(Integer),
        X::Boolean => matches!(scalar, Some(Integer | PostgresBoolean)),
        X::HexBinary => matches!(scalar, Some(PostgresBytea | MysqlBinaryBytes | MysqlBit)),
        X::Decimal => matches!(scalar, Some(PostgresNumeric | MysqlDecimal)),
        X::Date => scalar == Some(MysqlDate),
        X::DateTime => scalar == Some(MysqlDateTime),
        X::Double => matches!(scalar, Some(PostgresFloat4 | PostgresFloat8)),
        X::Time => false,
    }
}

pub(super) fn renderable(value: &LiteralOperand, actuals: &ActualColumns) -> bool {
    match value {
        LiteralOperand::Constant(_) => true,
        LiteralOperand::Column { column, spec } => {
            if spec.uses_natural_type(literal_datatype::fact(column, actuals).flatten()) {
                return natural_literal::natural(value, actuals).is_some();
            }
            natural_literal::natural(value, actuals).is_some()
                || iri_cmp::scalar_column(column, actuals).is_some_and(|key| {
                    !matches!(
                        key,
                        NativeScalarKey::MysqlFloat4 | NativeScalarKey::MysqlFloat8
                    )
                })
                || path_comparison::column_text(column, actuals).is_some()
        }
    }
}

pub(super) fn key(
    column: &ColRef,
    code: XsdTypeCode,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
) -> Result<String> {
    let raw = colref(column, dialect, actuals);
    let invalid = "JSON_EXTRACT('semantic-fabric-invalid-natural-literal', '$')";
    match (dialect, code) {
        (_, XsdTypeCode::String) => Ok(path_comparison::rdf_column(column, dialect, catalog, actuals)),
        (Dialect::MySql, XsdTypeCode::Integer) => {
            let lexical = iri_cmp::scalar_lexical(NativeScalarKey::Integer, &raw, dialect)?;
            Ok(format!("CASE WHEN {raw} IS NULL THEN NULL WHEN {raw} BETWEEN -9223372036854775808 AND 9223372036854775807 THEN {lexical} ELSE {invalid} END"))
        }
        (Dialect::MySql, XsdTypeCode::Boolean) => Ok(format!("CASE WHEN {raw} IS NULL THEN NULL WHEN {raw} = 0 THEN 'false' WHEN {raw} = 1 THEN 'true' ELSE {invalid} END")),
        (Dialect::MySql | Dialect::Postgres, XsdTypeCode::Decimal) => Ok(natural_decimal::key(&raw, dialect)),
        (Dialect::Postgres, XsdTypeCode::Double) => {
            let scalar = iri_cmp::scalar_column(column, actuals).filter(|key| pg_float::is_float(*key)).ok_or_else(|| Error::Unsupported("natural double lost its exact native float decoder".into()))?;
            Ok(pg_float::lexical(&raw, scalar, true))
        }
        (Dialect::MySql, XsdTypeCode::Date | XsdTypeCode::DateTime) => Ok(natural_literal::key(&raw, code)),
        (Dialect::Postgres, XsdTypeCode::Integer | XsdTypeCode::Boolean)
        | (Dialect::Postgres | Dialect::MySql, XsdTypeCode::HexBinary) => {
            let scalar = iri_cmp::scalar_column(column, actuals).ok_or_else(|| Error::Unsupported("natural literal key lost its native decoder proof".into()))?;
            iri_cmp::scalar_lexical(scalar, &raw, dialect)
        }
        _ => Err(Error::Unsupported("natural literal identity has no qualified native key for this decoder".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iq::literal_cmp::LiteralComparison;
    use sf_core::{ir::TermSpec, Literal};

    fn catalog(code: XsdTypeCode, scalar: NativeScalarKey) -> ColumnCatalog {
        let mut catalog = ColumnCatalog::default();
        catalog
            .insert_live_result(
                &LogicalSource::Table("items".into()),
                vec![sf_sql::backend::ResultColumn {
                    name: "src".into(),
                    natural_datatype: Some(code),
                    native_scalar: Some(scalar),
                    text_key: None,
                    sqlite_decode: None,
                }],
            )
            .unwrap();
        catalog
    }

    #[test]
    fn integer_and_boolean_datatypes_do_not_prove_mysql_union_decoders() {
        let maps = sf_mapping::parse_r2rml(
            r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
            <#m> rr:logicalTable [rr:tableName "items"]; rr:subject <http://ex/s>;
            rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:column "src"]]."#,
        )
        .unwrap();
        for code in [XsdTypeCode::Integer, XsdTypeCode::Boolean] {
            let catalog = catalog(code, NativeScalarKey::Integer);
            let mut plan = crate::parse_and_translate(
                "SELECT ?o WHERE { ?s <http://ex/p> ?o }",
                &maps,
                Dialect::MySql,
            )
            .unwrap();
            let single = subplan_actuals(&plan, Dialect::MySql, &catalog);
            assert_eq!(single.datatype_columns["c0"], Some(code));
            assert_eq!(single.natural_columns["c0"], Some(code));
            let prepared = &plan.prepared_branches()[0];
            let raw = ref_atom::actuals(prepared, &prepared.projection(), Dialect::MySql, &catalog);
            assert_eq!(raw.datatype_columns["c0"], Some(code));
            plan.branches.push(plan.branches[0].clone());
            let union = subplan_actuals(&plan, Dialect::MySql, &catalog);
            // Equal XSD types do not retain native width or signedness. MySQL
            // may promote the integer to decimal, or Boolean TINYINT to integer.
            assert_eq!(union.datatype_columns["c0"], None);
            assert_eq!(union.natural_columns["c0"], None);
            let cmp = LiteralComparison {
                left: LiteralOperand::Column {
                    column: ColRef::new(0, "c0"),
                    spec: TermSpec::plain_literal(),
                },
                right: LiteralOperand::Constant(Literal::new_typed_literal("1", code.iri())),
                value_op: None,
            };
            assert!(literal_cmp::render(
                &cmp,
                Dialect::MySql,
                &catalog,
                &HashMap::from([(0, union)]),
                &mut vec![],
                &mut 0
            )
            .unwrap_err()
            .to_string()
            .contains("compatible decoder"));
        }
    }

    #[test]
    fn matching_explicit_datatype_cannot_borrow_an_unqualified_raw_recipe() {
        let catalog = catalog(XsdTypeCode::DateTime, NativeScalarKey::MysqlTimestamp);
        let actuals = HashMap::from([(
            0,
            source_actuals(&LogicalSource::Table("items".into()), &catalog),
        )]);
        let mut spec = TermSpec::plain_literal();
        spec.datatype = Some(XsdTypeCode::DateTime.iri().into_owned());
        let column = ColRef::new(0, "src");
        assert_eq!(
            literal_datatype::fact(&column, &actuals),
            Some(Some(XsdTypeCode::DateTime))
        );
        assert!(!renderable(
            &LiteralOperand::Column {
                column: column.clone(),
                spec: spec.clone()
            },
            &actuals
        ));
        // An actual datatype override consumes the original lexical instead.
        spec.datatype = Some(XsdTypeCode::String.iri().into_owned());
        assert!(renderable(
            &LiteralOperand::Column { column, spec },
            &actuals
        ));
    }

    #[test]
    fn datatype_disjointness_is_not_a_numeric_value_comparison() {
        let mut catalog = ColumnCatalog::default();
        catalog
            .insert_live_result(
                &LogicalSource::Table("items".into()),
                vec![sf_sql::backend::ResultColumn {
                    name: "src".into(),
                    natural_datatype: Some(XsdTypeCode::Double),
                    native_scalar: None,
                    text_key: None,
                    sqlite_decode: None,
                }],
            )
            .unwrap();
        let actuals = HashMap::from([(
            0,
            source_actuals(&LogicalSource::Table("items".into()), &catalog),
        )]);
        let mut cmp = LiteralComparison {
            left: LiteralOperand::Column {
                column: ColRef::new(0, "src"),
                spec: TermSpec::plain_literal(),
            },
            right: LiteralOperand::Constant(Literal::new_typed_literal(
                "1",
                XsdTypeCode::Integer.iri(),
            )),
            value_op: None,
        };
        assert_eq!(
            literal_datatype::mismatch(&cmp, Dialect::Postgres, &actuals).unwrap(),
            Some("CASE WHEN t0.\"src\" IS NULL THEN NULL ELSE FALSE END".into())
        );
        cmp.value_op = Some(crate::iq::CmpOp::Eq);
        assert!(
            literal_datatype::mismatch(&cmp, Dialect::Postgres, &actuals)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn mysql_natural_integer_and_boolean_keys_keep_decoder_validation() {
        for code in [XsdTypeCode::Integer, XsdTypeCode::Boolean] {
            let catalog = catalog(code, NativeScalarKey::Integer);
            let actuals = HashMap::from([(
                0,
                source_actuals(&LogicalSource::Table("items".into()), &catalog),
            )]);
            let sql = key(
                &ColRef::new(0, "src"),
                code,
                Dialect::MySql,
                &catalog,
                &actuals,
            )
            .unwrap();
            assert!(sql.contains("IS NULL THEN NULL"));
            assert!(sql.contains("ELSE JSON_EXTRACT("));
            assert!(sql.contains(if code == XsdTypeCode::Integer {
                "BETWEEN -9223372036854775808 AND 9223372036854775807"
            } else {
                "THEN 'false'"
            }));
            Dialect::MySql
                .emit_via_ast(&format!("SELECT {sql} FROM items t0"))
                .unwrap();
        }
    }

    #[test]
    fn metadata_without_datatypes_cannot_authorize_new_mixed_literal_joins() {
        let source = LogicalSource::Table("items".into());
        let mut catalog = ColumnCatalog::default();
        catalog
            .insert_live_result(
                &source,
                vec![sf_sql::backend::ResultColumn {
                    name: "src".into(),
                    natural_datatype: None,
                    native_scalar: None,
                    text_key: None,
                    sqlite_decode: None,
                }],
            )
            .unwrap();
        let mut explicit = TermSpec::plain_literal();
        explicit.datatype = Some(XsdTypeCode::Decimal.iri().into_owned());
        let cmp = LiteralComparison {
            left: LiteralOperand::Column {
                column: ColRef::new(0, "src"),
                spec: TermSpec::plain_literal(),
            },
            right: LiteralOperand::Column {
                column: ColRef::new(1, "src"),
                spec: explicit,
            },
            value_op: None,
        };
        for dialect in [
            Dialect::Postgres,
            Dialect::MySql,
            Dialect::SqlServer,
            Dialect::Oracle,
            Dialect::Trino,
        ] {
            let actuals = HashMap::from([
                (0, source_actuals(&source, &catalog)),
                (1, source_actuals(&source, &catalog)),
            ]);
            assert!(
                literal_cmp::render(&cmp, dialect, &catalog, &actuals, &mut vec![], &mut 0)
                    .is_err(),
                "{dialect:?}: column names alone are not natural type authority"
            );
            let mut legacy = cmp.clone();
            if let LiteralOperand::Column { spec, .. } = &mut legacy.right {
                spec.datatype = Some(XsdTypeCode::String.iri().into_owned());
            }
            assert!(
                literal_cmp::render(&legacy, dialect, &catalog, &actuals, &mut vec![], &mut 0)
                    .is_ok(),
                "preserve the previously admitted, unqualified legacy string lane"
            );
            let offline = ColumnCatalog::default();
            let actuals = HashMap::from([
                (0, source_actuals(&source, &offline)),
                (1, source_actuals(&source, &offline)),
            ]);
            assert!(
                literal_cmp::render(&cmp, dialect, &offline, &actuals, &mut vec![], &mut 0).is_ok(),
                "offline SQL rendering is not live correctness evidence"
            );
        }
    }
}
