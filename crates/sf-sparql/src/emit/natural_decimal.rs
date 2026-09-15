//! Exact natural-decimal comparison keys; source payloads are never replaced.
use super::*;
use crate::iq::literal_cmp::{LiteralComparison, LiteralOperand};
use sf_core::datatype::XsdTypeCode;

#[cfg(test)]
pub(super) fn value_equality(
    cmp: &LiteralComparison,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<Option<String>> {
    value_equality_controlled(
        cmp,
        dialect,
        catalog,
        actuals,
        params,
        pidx,
        sf_sql::source_work::SourceWork::new(None),
    )
}

pub(super) fn value_equality_controlled(
    cmp: &LiteralComparison,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<Option<String>> {
    work.charge(1).map_err(source_control::validation_error)?;
    use crate::iq::CmpOp;
    let natural = |value: &LiteralOperand| {
        natural_literal::natural(value, actuals) == Some(XsdTypeCode::Decimal)
    };
    let qualified = |value: &LiteralOperand| {
        natural(value)
            || matches!(value,
        LiteralOperand::Constant(literal) if [XsdTypeCode::Integer.iri(), XsdTypeCode::Decimal.iri()].contains(&literal.datatype()))
    };
    if !matches!(dialect, Dialect::Postgres | Dialect::MySql)
        || !matches!(cmp.value_op, Some(CmpOp::Eq | CmpOp::Ne))
    {
        return Ok(None);
    }
    if [&cmp.left, &cmp.right].iter().any(|value| {
        matches!(value,
        LiteralOperand::Column { column, spec } if spec.language.is_none()
            && natural_literal::column_fact(column, actuals) == Some(None))
    }) {
        return Err(Error::Unsupported(
            "natural numeric equality requires a compatible decoder in every SubPlan arm".into(),
        ));
    }
    if ![&cmp.left, &cmp.right].iter().any(|v| natural(v)) {
        return Ok(None);
    }
    if [&cmp.left, &cmp.right].iter().any(|value| matches!(value,
        LiteralOperand::Column { spec, .. } if spec.datatype.is_none() && spec.language.is_none() && !natural(value))) {
        return Err(Error::Unsupported("natural numeric equality requires each column's decimal decoder".into()));
    }
    if ![&cmp.left, &cmp.right].iter().all(|v| qualified(v)) {
        return Ok(None);
    }
    // Numeric equality promotes integer/decimal values, not RDF term identity.
    // Full lexical normalization is injective on these finite values. Never
    // canonicalize the original query literal or feed this key to ordered ops.
    let normalize = |value: &LiteralOperand| -> Result<Option<LiteralOperand>> {
        let literal = match value {
            LiteralOperand::Constant(literal) => literal,
            LiteralOperand::Column { column, spec } => {
                // The identity comparison owns its operands: pay every owned
                // string the copy duplicates (column, datatype, language, base).
                for len in [
                    column.column.len(),
                    spec.datatype.as_ref().map_or(0, |dt| dt.as_str().len()),
                    spec.language.as_deref().map_or(0, str::len),
                    spec.base.as_deref().map_or(0, str::len),
                ] {
                    work.charge(len).map_err(source_control::validation_error)?;
                }
                work.charge(1).map_err(source_control::validation_error)?;
                return Ok(Some(value.clone()));
            }
        };
        let raw = literal.value();
        // Prepay the integer grammar pass, decimal split/validation/trimming,
        // canonical output and owned literal copy. Decimal output is <= n+1.
        work.product(raw.len(), 8)
            .map_err(source_control::validation_error)?;
        work.charge(2).map_err(source_control::validation_error)?;
        if literal.datatype() == XsdTypeCode::Integer.iri() {
            let digits = raw.strip_prefix(['+', '-']).unwrap_or(raw);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return Ok(None);
            }
        }
        let mut canonical = String::new();
        // A valid str cannot exceed isize::MAX bytes, so n+1 fits usize.
        canonical
            .try_reserve_exact(raw.len() + 1)
            .map_err(|_| Error::Sql("decimal normalization allocation failed".into()))?;
        if sf_core::datatype::canonical_lexical(raw, XsdTypeCode::Decimal, &mut canonical).is_err()
        {
            return Ok(None);
        }
        work.checkpoint()
            .map_err(source_control::validation_error)?;
        Ok(Some(LiteralOperand::Constant(
            sf_core::Literal::new_typed_literal(canonical, XsdTypeCode::Decimal.iri()),
        )))
    };
    let (Some(left), Some(right)) = (normalize(&cmp.left)?, normalize(&cmp.right)?) else {
        return Ok(Some("(NULL = 1)".into()));
    };
    let identity = LiteralComparison {
        left,
        right,
        value_op: None,
    };
    let sql = natural_literal::comparison_controlled(
        &identity, dialect, catalog, actuals, params, pidx, work,
    )?;
    Ok(sql.map(|sql| {
        if cmp.value_op == Some(CmpOp::Ne) {
            format!("NOT ({sql})")
        } else {
            sql
        }
    }))
}

pub(super) fn key(raw: &str, dialect: Dialect) -> String {
    if dialect == Dialect::Postgres {
        // trim_scale changes only insignificant scale; JSON still rejects the
        // non-finite NUMERIC values rejected by the Rust decoder. No fixed cast.
        return pg_numeric::lexical(&format!("pg_catalog.trim_scale({raw})"));
    }
    // NEWDECIMAL is finite, but CAST AS CHAR can retain ZEROFILL and scale.
    // Normalize lexically over its full range, never through a narrower number.
    let text = format!("CONVERT(CAST({raw} AS CHAR) USING utf8mb4)");
    let magnitude = format!("TRIM(LEADING '-' FROM {text})");
    let integer = format!("TRIM(LEADING '0' FROM SUBSTRING_INDEX({magnitude}, '.', 1))");
    let fraction = format!("CASE WHEN LOCATE('.', {magnitude}) = 0 THEN '' ELSE TRIM(TRAILING '0' FROM SUBSTRING_INDEX({magnitude}, '.', -1)) END");
    format!("CONCAT(CASE WHEN LEFT({text}, 1) = '-' AND ({integer} <> '' OR ({fraction}) <> '') THEN '-' ELSE '' END, CASE WHEN {integer} = '' THEN '0' ELSE {integer} END, CASE WHEN ({fraction}) = '' THEN '' ELSE CONCAT('.', ({fraction})) END)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iq::literal_cmp::{LiteralComparison, LiteralOperand};
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    use sf_core::{datatype::XsdTypeCode, ir::TermSpec};

    fn catalog(source: &LogicalSource, key: NativeScalarKey) -> ColumnCatalog {
        let mut catalog = ColumnCatalog::default();
        catalog
            .insert_live_result(
                source,
                vec![sf_sql::backend::ResultColumn {
                    natural_datatype: None,
                    name: "src".into(),
                    native_scalar: Some(key),
                    text_key: None,
                    sqlite_decode: None,
                }],
            )
            .unwrap();
        catalog
    }

    fn comparison(column: ColRef) -> LiteralComparison {
        LiteralComparison {
            left: LiteralOperand::Column {
                column,
                spec: TermSpec::plain_literal(),
            },
            right: LiteralOperand::Constant(sf_core::Literal::new_typed_literal(
                "1.00",
                XsdTypeCode::Decimal.iri(),
            )),
            value_op: None,
        }
    }

    #[test]
    fn natural_identity_preserves_constants_null_and_value_lane() {
        let source = LogicalSource::Table("items".into());
        for (dialect, scalar) in [
            (Dialect::Postgres, NativeScalarKey::PostgresNumeric),
            (Dialect::MySql, NativeScalarKey::MysqlDecimal),
        ] {
            let catalog = catalog(&source, scalar);
            let actuals = HashMap::from([(0, source_actuals(&source, &catalog))]);
            let mut cmp = comparison(ColRef::new(0, "src"));
            for reverse in [false, true] {
                if reverse {
                    std::mem::swap(&mut cmp.left, &mut cmp.right);
                }
                let mut params = Vec::new();
                let sql =
                    literal_cmp::render(&cmp, dialect, &catalog, &actuals, &mut params, &mut 0)
                        .unwrap();
                assert_eq!(
                    params,
                    [
                        "1.00",
                        XsdTypeCode::Decimal.iri().as_str(),
                        XsdTypeCode::Decimal.iri().as_str(),
                        "",
                        ""
                    ]
                );
                assert!(
                    sql.starts_with("CASE WHEN ") && sql.contains("IS NULL THEN NULL ELSE"),
                    "{sql}"
                );
                assert!(
                    sql.contains(if dialect == Dialect::Postgres {
                        "pg_catalog.trim_scale"
                    } else {
                        "TRIM(LEADING '0'"
                    }),
                    "{sql}"
                );
                dialect
                    .emit_via_ast(&format!("SELECT 1 FROM items t0 WHERE {sql}"))
                    .unwrap();
                if dialect == Dialect::MySql {
                    assert_eq!(sql.matches('?').count(), params.len());
                }
            }
            cmp.value_op = Some(crate::iq::CmpOp::Eq);
            assert!(natural_literal::comparison(
                &cmp,
                dialect,
                &catalog,
                &actuals,
                &mut vec![],
                &mut 0
            )
            .unwrap()
            .is_none());
            cmp.value_op = None;
            cmp.right = LiteralOperand::Column {
                column: ColRef::new(9, "unknown"),
                spec: TermSpec::plain_literal(),
            };
            // Restore a known natural left operand before testing missing proof.
            cmp.left = comparison(ColRef::new(0, "src")).left;
            assert!(natural_literal::comparison(
                &cmp,
                dialect,
                &catalog,
                &actuals,
                &mut vec![],
                &mut 0
            )
            .unwrap_err()
            .to_string()
            .contains("natural decoder"));
        }
    }

    #[test]
    fn decimal_proof_follows_raw_positions_but_not_coercing_unions() {
        let maps = sf_mapping::parse_r2rml(
            r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
            <#m> rr:logicalTable [rr:tableName "items"]; rr:subject <http://ex/s>;
            rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:column "src"]]."#,
        )
        .unwrap();
        for (dialect, scalar) in [
            (Dialect::Postgres, NativeScalarKey::PostgresNumeric),
            (Dialect::MySql, NativeScalarKey::MysqlDecimal),
        ] {
            let catalog = catalog(&maps[0].source, scalar);
            let mut plan = crate::parse_and_translate(
                "SELECT ?o WHERE { ?s <http://ex/p> ?o }",
                &maps,
                dialect,
            )
            .unwrap();
            let prepared = &plan.prepared_branches()[0];
            let raw = prepared.projection()[0].clone();
            let actuals = branch_actuals(prepared, dialect, &catalog);
            assert_eq!(
                natural_literal::column_code(&raw, &actuals),
                Some(XsdTypeCode::Decimal)
            );
            assert_eq!(
                subplan_actuals(&plan, dialect, &catalog).natural_columns["c0"],
                Some(XsdTypeCode::Decimal)
            );
            let ref_actuals = ref_atom::actuals(prepared, &[raw], dialect, &catalog);
            assert_eq!(
                ref_actuals.natural_columns["c0"],
                Some(XsdTypeCode::Decimal)
            );
            plan.branches.push(plan.branches[0].clone());
            assert_eq!(
                subplan_actuals(&plan, dialect, &catalog).natural_columns["c0"],
                if dialect == Dialect::Postgres {
                    Some(XsdTypeCode::Decimal)
                } else {
                    None
                }
            );
            plan.branches[1].core[0].source = LogicalSource::Table("unknown".into()).into();
            let actual = subplan_actuals(&plan, dialect, &catalog);
            assert_eq!(actual.natural_columns["c0"], None);
            let mut cmp = comparison(ColRef::new(7, "c0"));
            let actuals = HashMap::from([(7, actual)]);
            for op in [None, Some(crate::iq::CmpOp::Eq), Some(crate::iq::CmpOp::Ne)] {
                cmp.value_op = op;
                assert!(literal_cmp::render(
                    &cmp,
                    dialect,
                    &catalog,
                    &actuals,
                    &mut vec![],
                    &mut 0
                )
                .unwrap_err()
                .to_string()
                .contains("compatible decoder"));
            }
        }
    }

    #[test]
    fn numeric_equality_normalizes_only_qualified_values_without_range_loss() {
        let source = LogicalSource::Table("items".into());
        for (dialect, scalar) in [
            (Dialect::Postgres, NativeScalarKey::PostgresNumeric),
            (Dialect::MySql, NativeScalarKey::MysqlDecimal),
        ] {
            let catalog = catalog(&source, scalar);
            let actuals = HashMap::from([(0, source_actuals(&source, &catalog))]);
            let mut cmp = comparison(ColRef::new(0, "src"));
            cmp.value_op = Some(crate::iq::CmpOp::Eq);
            for (lexical, code, expected) in [
                ("+0001.000".into(), XsdTypeCode::Decimal, "1".into()),
                ("-000".into(), XsdTypeCode::Integer, "0".into()),
                (
                    "9".repeat(131_072),
                    XsdTypeCode::Integer,
                    "9".repeat(131_072),
                ),
                (
                    format!("0.{}1", "0".repeat(16_382)),
                    XsdTypeCode::Decimal,
                    format!("0.{}1", "0".repeat(16_382)),
                ),
            ] {
                cmp.right = LiteralOperand::Constant(sf_core::Literal::new_typed_literal(
                    lexical,
                    code.iri(),
                ));
                let mut params = Vec::new();
                assert!(
                    value_equality(&cmp, dialect, &catalog, &actuals, &mut params, &mut 0)
                        .unwrap()
                        .is_some()
                );
                assert_eq!(params[0], expected);
                let budget =
                    |n| QueryBudget::new(QueryLimits::new(u64::MAX, n, u64::MAX, u64::MAX));
                let run = |control: &QueryBudget| {
                    let mut values = vec![];
                    let mut index = 0;
                    let sql = value_equality_controlled(
                        &cmp,
                        dialect,
                        &catalog,
                        &actuals,
                        &mut values,
                        &mut index,
                        sf_sql::source_work::SourceWork::new(Some(control)),
                    )?;
                    Ok::<_, Error>((sql, values, index))
                };
                let measured = budget(u64::MAX);
                let result = run(&measured).unwrap();
                assert_eq!(result.1, params);
                let n = measured.consumed(QueryCharge::SourceWork);
                assert_eq!(run(&budget(n)).unwrap(), result);
                assert!(matches!(
                    run(&budget(n - 1)),
                    Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                ));
            }
            {
                // The column operand's owned strings are paid before its copy.
                let consumed = |cmp: &LiteralComparison| {
                    let control =
                        QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
                    value_equality_controlled(
                        cmp,
                        dialect,
                        &catalog,
                        &actuals,
                        &mut vec![],
                        &mut 0,
                        sf_sql::source_work::SourceWork::new(Some(&control)),
                    )
                    .unwrap()
                    .unwrap();
                    control.consumed(QueryCharge::SourceWork)
                };
                let mut typed = comparison(ColRef::new(0, "src"));
                typed.value_op = Some(crate::iq::CmpOp::Eq);
                let plain = consumed(&typed);
                let datatype = XsdTypeCode::Decimal.iri();
                typed.left = LiteralOperand::Column {
                    column: ColRef::new(0, "src"),
                    spec: TermSpec {
                        datatype: Some(datatype.into_owned()),
                        ..TermSpec::plain_literal()
                    },
                };
                assert_eq!(consumed(&typed), plain + datatype.as_str().len() as u64);
            }
            for (lexical, code) in [
                ("1.0", XsdTypeCode::Integer),
                ("+", XsdTypeCode::Integer),
                ("1e0", XsdTypeCode::Decimal),
                ("NaN", XsdTypeCode::Decimal),
            ] {
                cmp.right = LiteralOperand::Constant(sf_core::Literal::new_typed_literal(
                    lexical,
                    code.iri(),
                ));
                for op in [crate::iq::CmpOp::Eq, crate::iq::CmpOp::Ne] {
                    cmp.value_op = Some(op);
                    let mut params = Vec::new();
                    assert_eq!(
                        value_equality(&cmp, dialect, &catalog, &actuals, &mut params, &mut 0)
                            .unwrap()
                            .unwrap(),
                        "(NULL = 1)"
                    );
                    assert!(params.is_empty());
                }
            }
            cmp.right = LiteralOperand::Column {
                column: ColRef::new(9, "unknown"),
                spec: TermSpec::plain_literal(),
            };
            assert!(
                value_equality(&cmp, dialect, &catalog, &actuals, &mut vec![], &mut 0)
                    .unwrap_err()
                    .to_string()
                    .contains("decimal decoder")
            );
        }
    }

    #[test]
    fn postgres_literal_validation_itself_is_dominated_by_policy() {
        let source = LogicalSource::Table("items".into());
        let catalog = catalog(&source, NativeScalarKey::PostgresNumeric);
        let actuals = HashMap::from([(0, source_actuals(&source, &catalog))]);
        for op in [None, Some(crate::iq::CmpOp::Eq), Some(crate::iq::CmpOp::Ne)] {
            let mut cmp = comparison(ColRef::new(0, "src"));
            cmp.value_op = op;
            let identity = SqlCond::LiteralCmp(Box::new(cmp));
            let cond = SqlCond::Not(Box::new(identity));
            let policy = SqlCond::NativeCmp(
                ColRef::new(0, "tenant"),
                crate::iq::CmpOp::Eq,
                "allowed".into(),
            );
            let mut params = Vec::new();
            let sql = natural_literal::authorized_conjunction(
                &[&cond, &policy],
                Dialect::Postgres,
                &catalog,
                &actuals,
                &mut params,
                &mut 0,
            )
            .unwrap()
            .unwrap();
            assert!(
                sql.starts_with("CASE WHEN (t0.\"tenant\" = $1) THEN"),
                "{sql}"
            );
            assert!(sql.contains("AS JSON"));
            assert_eq!(params[0], "allowed");
        }
    }
}
