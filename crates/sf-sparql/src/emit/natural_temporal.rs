//! Natural temporal identity is separate from raw IRI and SQL value authority.
use super::*;
use crate::iq::literal_cmp::{LiteralComparison, LiteralOperand};
use sf_core::datatype::XsdTypeCode;

pub(super) fn source_code(key: NativeScalarKey) -> Option<XsdTypeCode> {
    match key {
        NativeScalarKey::MysqlDate => Some(XsdTypeCode::Date),
        NativeScalarKey::MysqlDateTime => Some(XsdTypeCode::DateTime),
        _ => None,
    }
}

pub(super) fn column_code(column: &ColRef, actuals: &ActualColumns) -> Option<XsdTypeCode> {
    column_fact(column, actuals).flatten()
}

pub(super) fn column_fact(column: &ColRef, actuals: &ActualColumns) -> Option<Option<XsdTypeCode>> {
    let source = actuals.get(&column.alias)?;
    source
        .natural_temporals
        .get(resolve_col(&column.column, Some(&source.columns)))
        .copied()
}

fn natural(value: &LiteralOperand, actuals: &ActualColumns) -> Option<XsdTypeCode> {
    match value {
        LiteralOperand::Column { column, spec }
            if spec.datatype.is_none() && spec.language.is_none() =>
        {
            column_code(column, actuals)
        }
        _ => None,
    }
}

pub(super) fn authorized_conjunction(
    conds: &[&SqlCond],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<Option<String>> {
    fn validates(cond: &SqlCond, actuals: &ActualColumns) -> bool {
        match cond {
            SqlCond::LiteralCmp(cmp) => {
                cmp.value_op.is_none()
                    && [&cmp.left, &cmp.right]
                        .iter()
                        .any(|v| natural(v, actuals).is_some())
            }
            SqlCond::Not(c) => validates(c, actuals),
            SqlCond::And(cs) | SqlCond::Or(cs) => cs.iter().any(|c| validates(c, actuals)),
            _ => false,
        }
    }
    let (policies, rest): (Vec<_>, Vec<_>) = conds
        .iter()
        .copied()
        .partition(|c| matches!(c, SqlCond::NativeCmp(..)));
    if dialect != Dialect::MySql
        || policies.is_empty()
        || !rest.iter().any(|c| validates(c, actuals))
    {
        return Ok(None);
    }
    // Direct EXISTS bodies have no D1 barrier. These native conditions are
    // admission predicates: FALSE/NULL both deny, before fallible construction.
    let policy = policies
        .iter()
        .map(|c| render_cond(c, dialect, catalog, actuals, params, pidx))
        .collect::<Result<Vec<_>>>()?
        .join(" AND ");
    let body = rest
        .iter()
        .map(|c| render_cond(c, dialect, catalog, actuals, params, pidx))
        .collect::<Result<Vec<_>>>()?
        .join(" AND ");
    Ok(Some(format!(
        "CASE WHEN ({policy}) THEN ({body}) ELSE FALSE END"
    )))
}

fn key(raw: &str, code: XsdTypeCode) -> String {
    let text = format!("CONVERT(CAST({raw} AS CHAR) USING utf8mb4)");
    let year = format!("CAST(SUBSTRING({text}, 1, 4) AS DECIMAL(4, 0))");
    let month = format!("CAST(SUBSTRING({text}, 6, 2) AS DECIMAL(2, 0))");
    let day = format!("CAST(SUBSTRING({text}, 9, 2) AS DECIMAL(2, 0))");
    let leap = format!("(MOD({year}, 400) = 0 OR (MOD({year}, 4) = 0 AND MOD({year}, 100) <> 0))");
    let days = format!("CASE WHEN {month} = 2 THEN CASE WHEN {leap} THEN 29 ELSE 28 END WHEN {month} IN (4, 6, 9, 11) THEN 30 ELSE 31 END");
    let lexical = if code == XsdTypeCode::Date {
        text.clone()
    } else {
        let whole = format!("REPLACE(SUBSTRING_INDEX({text}, '.', 1), ' ', 'T')");
        let fraction = format!("TRIM(TRAILING '0' FROM SUBSTRING_INDEX({text}, '.', -1))");
        format!("CASE WHEN LOCATE('.', {text}) = 0 OR {fraction} = '' THEN {whole} ELSE CONCAT({whole}, '.', {fraction}) END")
    };
    // Native DATE/DATETIME supply bounded numeric fields. Year zero is valid
    // under the locked XSD parser; zero month/day and invalid calendars are not.
    format!("(CASE WHEN {raw} IS NULL THEN NULL WHEN {month} BETWEEN 1 AND 12 AND {day} BETWEEN 1 AND ({days}) THEN {lexical} ELSE JSON_EXTRACT('semantic-fabric-invalid-natural-date', '$') END)")
}

pub(super) fn comparison(
    cmp: &LiteralComparison,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<Option<String>> {
    if dialect == Dialect::MySql && cmp.value_op.is_none() && [&cmp.left, &cmp.right].iter().any(|value| {
        matches!(value, LiteralOperand::Column { column, spec } if spec.datatype.is_none() && spec.language.is_none() && column_fact(column, actuals) == Some(None))
    }) {
        return Err(Error::Unsupported("natural temporal identity requires a compatible decoder in every SubPlan arm".into()));
    }
    if dialect != Dialect::MySql
        || cmp.value_op.is_some()
        || ![&cmp.left, &cmp.right]
            .iter()
            .any(|value| natural(value, actuals).is_some())
    {
        return Ok(None);
    }
    let mut bind = |value: &str| {
        params.push(value.to_owned());
        *pidx += 1;
        dialect.placeholder(*pidx)
    };
    let mut component = |value: &LiteralOperand, part: usize| -> Result<String> {
        match value {
            LiteralOperand::Constant(literal) => Ok(bind(match part {
                0 => literal.value(),
                1 => literal.datatype().as_str(),
                _ => literal.language().unwrap_or(""),
            })),
            LiteralOperand::Column { column, spec } => {
                let raw = colref(column, dialect, actuals);
                if part == 2 {
                    return Ok(bind(spec.language.as_deref().unwrap_or("")));
                }
                if let Some(code) = natural(value, actuals) {
                    return Ok(if part == 0 {
                        key(&raw, code)
                    } else {
                        bind(code.iri().as_str())
                    });
                }
                if part == 1 {
                    return Ok(bind(
                        spec.datatype.as_ref().map(|dt| dt.as_str()).unwrap_or(
                            if spec.language.is_some() {
                                "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString"
                            } else {
                                "http://www.w3.org/2001/XMLSchema#string"
                            },
                        ),
                    ));
                }
                if let Some(code) = column_code(column, actuals) {
                    iri_cmp::scalar_lexical(
                        if code == XsdTypeCode::Date {
                            NativeScalarKey::MysqlDate
                        } else {
                            NativeScalarKey::MysqlDateTime
                        },
                        &raw,
                        dialect,
                    )
                } else if path_comparison::column_text(column, actuals).is_some() {
                    Ok(path_comparison::rdf_column(
                        column, dialect, catalog, actuals,
                    ))
                } else {
                    Err(Error::Unsupported(
                        "natural temporal identity requires each operand's decoder".into(),
                    ))
                }
            }
        }
    };
    // Anonymous MySQL binds must be appended in emitted SQL component order.
    let left = component(&cmp.left, 0)?;
    let right = component(&cmp.right, 0)?;
    let ldt = component(&cmp.left, 1)?;
    let rdt = component(&cmp.right, 1)?;
    let lang = component(&cmp.left, 2)?;
    let rang = component(&cmp.right, 2)?;
    // Bound query lexicals are deliberately NOT canonicalized for RDF identity.
    let exact = |value| path_comparison::exact_text(value, dialect);
    let equality = format!(
        "({} = {} AND {} = {} AND {} = {})",
        exact(left),
        exact(right),
        exact(ldt),
        exact(rdt),
        exact(lang),
        exact(rang)
    );
    let missing = cmp
        .columns()
        .map(|c| format!("{} IS NULL", colref(c, dialect, actuals)))
        .collect::<Vec<_>>();
    Ok(Some(format!(
        "CASE WHEN {} THEN NULL ELSE {equality} END",
        missing.join(" OR ")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iq::ScanSource;

    fn catalog(source: &LogicalSource, key: NativeScalarKey) -> ColumnCatalog {
        let mut catalog = ColumnCatalog::default();
        catalog
            .insert_live_result(
                source,
                vec![
                    sf_sql::backend::ResultColumn {
                        name: "src".into(),
                        native_scalar: Some(key),
                        text_key: None,
                        sqlite_decode: None,
                    },
                    sf_sql::backend::ResultColumn {
                        name: "tenant".into(),
                        native_scalar: None,
                        text_key: Some(TextKey::Verbatim),
                        sqlite_decode: None,
                    },
                ],
            )
            .unwrap();
        catalog
    }

    #[test]
    fn incompatible_temporal_union_keeps_rejection_proof() {
        let source = LogicalSource::Table("items".into());
        let other = LogicalSource::Table("other".into());
        let mut catalog = catalog(&source, NativeScalarKey::MysqlDate);
        catalog
            .insert_live_result(
                &other,
                vec![sf_sql::backend::ResultColumn {
                    name: "src".into(),
                    native_scalar: Some(NativeScalarKey::MysqlDateTime),
                    text_key: None,
                    sqlite_decode: None,
                }],
            )
            .unwrap();
        let maps = sf_mapping::parse_r2rml(
            r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
            <#m> rr:logicalTable [rr:tableName "items"]; rr:subject <http://ex/s>;
            rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:column "src"]]."#,
        )
        .unwrap();
        let mut plan = crate::parse_and_translate(
            "SELECT ?o WHERE { ?s <http://ex/p> ?o }",
            &maps,
            Dialect::MySql,
        )
        .unwrap();
        assert_eq!(
            subplan_actuals(&plan, Dialect::MySql, &catalog).natural_temporals["c0"],
            Some(XsdTypeCode::Date)
        );
        let mut branch = plan.branches[0].clone();
        branch.core[0].source = other.into();
        plan.branches.push(branch);
        let actual = subplan_actuals(&plan, Dialect::MySql, &catalog);
        assert_eq!(actual.natural_temporals["c0"], None);
        let cmp = LiteralComparison {
            left: LiteralOperand::Column {
                column: ColRef::new(7, "c0"),
                spec: sf_core::ir::TermSpec::plain_literal(),
            },
            right: LiteralOperand::Constant(sf_core::Literal::new_typed_literal(
                "2024-02-29",
                XsdTypeCode::Date.iri(),
            )),
            value_op: None,
        };
        assert!(comparison(
            &cmp,
            Dialect::MySql,
            &catalog,
            &HashMap::from([(7, actual)]),
            &mut vec![],
            &mut 0
        )
        .unwrap_err()
        .to_string()
        .contains("compatible decoder"));
    }

    #[test]
    fn natural_policy_validation_has_structural_dominance() {
        let maps = sf_mapping::parse_r2rml(
            r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
            <#m> rr:logicalTable [rr:tableName "items"]; rr:subject <http://ex/s>;
            rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:column "src"]]."#,
        )
        .unwrap();
        let plan = crate::parse_and_translate(
            "SELECT ?o WHERE { ?s <http://ex/p> ?o }",
            &maps,
            Dialect::MySql,
        )
        .unwrap();
        let catalog = catalog(&maps[0].source, NativeScalarKey::MysqlDateTime);
        let mut branch = plan.prepared_branches()[0].clone();
        let scan = &mut branch.core[0];
        let alias = scan.alias;
        let policy = SqlCond::NativeCmp(
            ColRef::new(alias, "tenant"),
            crate::iq::CmpOp::Eq,
            "allowed".into(),
        );
        let ScanSource::Projection { guards, .. } = &mut scan.source else {
            panic!("expected D1");
        };
        guards.push(policy.clone());
        let emitted = emit_branch_with(&branch, Dialect::MySql, &catalog).unwrap();
        assert!(
            emitted.sql.contains("LIMIT 18446744073709551615"),
            "{}",
            emitted.sql
        );
        assert!(emitted.sql.find("LIMIT 18446744073709551615") < emitted.sql.find("JSON_EXTRACT"));
        assert_eq!(emitted.params[0], "allowed");
        branch.core[0].source = maps[0].source.clone().into();
        branch.where_conds.push(policy);
        let emitted = emit_branch_with(&branch, Dialect::MySql, &catalog).unwrap();
        assert!(
            emitted.sql.contains("CASE WHEN (t0.`tenant` = ?) THEN"),
            "{}",
            emitted.sql
        );
        assert_eq!(emitted.params[0], "allowed");
    }

    #[test]
    fn natural_identity_binds_components_in_sql_order_and_preserves_null() {
        let source = LogicalSource::Table("items".into());
        let mut catalog = ColumnCatalog::default();
        catalog
            .insert_live_result(
                &source,
                vec![sf_sql::backend::ResultColumn {
                    name: "src".into(),
                    native_scalar: Some(NativeScalarKey::MysqlDateTime),
                    text_key: None,
                    sqlite_decode: None,
                }],
            )
            .unwrap();
        let actuals = HashMap::from([(0, source_actuals(&source, &catalog))]);
        let natural = LiteralOperand::Column {
            column: ColRef::new(0, "src"),
            spec: sf_core::ir::TermSpec::plain_literal(),
        };
        let constant = LiteralOperand::Constant(sf_core::Literal::new_typed_literal(
            "2024-02-29T12:34:56.100000",
            XsdTypeCode::DateTime.iri(),
        ));
        for (left, right) in [(natural.clone(), constant.clone()), (constant, natural)] {
            let cmp = LiteralComparison {
                left,
                right,
                value_op: None,
            };
            let mut params = Vec::new();
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
            assert_eq!(
                params,
                [
                    "2024-02-29T12:34:56.100000",
                    XsdTypeCode::DateTime.iri().as_str(),
                    XsdTypeCode::DateTime.iri().as_str(),
                    "",
                    ""
                ]
            );
            assert_eq!(sql.matches('?').count(), params.len());
            assert!(
                sql.starts_with("CASE WHEN t0.`src` IS NULL THEN NULL ELSE"),
                "{sql}"
            );
        }
    }
}
