//! Full-precision integer/decimal value comparison; never a floating prefix.
use super::*;
use crate::iq::literal_cmp::{LiteralComparison, LiteralOperand};

pub(super) fn datatype<'a>(value: &'a LiteralOperand, actuals: &ActualColumns) -> Option<&'a str> {
    match value {
        LiteralOperand::Constant(literal) => Some(literal.datatype().as_str()),
        LiteralOperand::Column { column, spec } => {
            if spec.language.is_some() {
                return Some("http://www.w3.org/1999/02/22-rdf-syntax-ns#langString");
            }
            spec.datatype.as_ref().map(|dt| dt.as_str()).or_else(|| {
                literal_datatype::fact(column, actuals)
                    .flatten()
                    .map(|code| code.iri().as_str())
            })
        }
    }
}

pub(super) fn exact(datatype: &str) -> bool {
    datatype == "http://www.w3.org/2001/XMLSchema#decimal"
        || sf_core::numeric_compare::is_integer_datatype(datatype)
}

#[cfg(test)]
pub(super) fn comparison(
    cmp: &LiteralComparison,
    dialect: Dialect,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<Option<String>> {
    comparison_controlled(
        cmp,
        dialect,
        actuals,
        params,
        pidx,
        sf_sql::source_work::SourceWork::new(None),
    )
}

pub(super) fn comparison_controlled(
    cmp: &LiteralComparison,
    dialect: Dialect,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<Option<String>> {
    work.charge(1).map_err(source_control::validation_error)?;
    let Some(op) = cmp.value_op else {
        return Ok(None);
    };
    if dialect != Dialect::Postgres
        || ![&cmp.left, &cmp.right]
            .iter()
            .any(|v| datatype(v, actuals).is_some_and(exact))
    {
        return Ok(None);
    }
    let left = operand_controlled(&cmp.left, actuals, params, pidx, work)?;
    let right = operand_controlled(&cmp.right, actuals, params, pidx, work)?;
    // Canonical nonzero negatives have exactly one leading '-'. After removing
    // it, compare integral length, integral digits and fractional digits under
    // C collation. Fractions retain leading zeros and discard only trailing
    // zeros: a prefix is smaller exactly when its omitted suffix is nonzero.
    // Both materialized operands must discharge source decoder obligations
    // even if the other operand is NULL or lexically invalid.
    Ok(Some(format!(
        r#"(WITH
      __sf_exact_values AS MATERIALIZED (SELECT {left} AS l, {right} AS r),
      __sf_exact_parts AS MATERIALIZED (SELECT l, r, pg_catalog.left(l,1)='-' AS ln, pg_catalog.left(r,1)='-' AS rn,
        pg_catalog.split_part(pg_catalog.ltrim(l,'-'),'.',1) AS li, pg_catalog.split_part(l,'.',2) AS lf,
        pg_catalog.split_part(pg_catalog.ltrim(r,'-'),'.',1) AS ri, pg_catalog.split_part(r,'.',2) AS rf FROM __sf_exact_values),
      __sf_exact_order AS MATERIALIZED (SELECT l,r,ln,rn,
        CASE WHEN ROW(pg_catalog.length(li),li,lf) = ROW(pg_catalog.length(ri),ri,rf) THEN 0
          WHEN ROW(pg_catalog.length(li),li,lf) < ROW(pg_catalog.length(ri),ri,rf) THEN -1 ELSE 1 END AS magnitude FROM __sf_exact_parts)
      SELECT CASE WHEN l IS NULL OR r IS NULL THEN NULL ELSE
        (CASE WHEN ln AND NOT rn THEN -1 WHEN rn AND NOT ln THEN 1 WHEN ln THEN -magnitude ELSE magnitude END) {} 0 END FROM __sf_exact_order)"#,
        op.as_sql()
    )))
}

#[cfg(test)]
fn operand(
    value: &LiteralOperand,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    operand_controlled(
        value,
        actuals,
        params,
        pidx,
        sf_sql::source_work::SourceWork::new(None),
    )
}

fn operand_controlled(
    value: &LiteralOperand,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    let unsupported = || {
        Error::Unsupported("exact decimal value comparison requires each operand's compatible decoder, retained native lexical proof and datatype".into())
    };
    let datatype = datatype(value, actuals).ok_or_else(unsupported)?;
    let raw = match value {
        LiteralOperand::Constant(literal) => {
            // PostgreSQL text cannot transport NUL. It is never a valid XSD
            // numeric character, so retain an expression error without a bind;
            // the parent still evaluates the other source decoder obligation.
            work.charge(literal.value().len())
                .map_err(source_control::validation_error)?;
            if !exact(datatype) || literal.value().contains('\0') {
                return Ok("CAST(NULL AS TEXT)".into());
            }
            work.parameter(params, pidx, literal.value())
                .map_err(source_control::validation_error)?;
            format!("CAST(${} AS TEXT)", *pidx)
        }
        LiteralOperand::Column { column, .. } => {
            let raw = colref(column, Dialect::Postgres, actuals);
            match path_comparison::column_text(column, actuals) {
                Some(TextKey::Verbatim) => raw,
                Some(TextKey::PostgresCharacter) => {
                    format!("pg_catalog.convert_from(pg_catalog.bpcharsend({raw}), 'UTF8')")
                }
                Some(_) => return Err(unsupported()),
                None => match iri_cmp::scalar_column(column, actuals) {
                    Some(NativeScalarKey::Integer) => format!("CAST({raw} AS TEXT)"),
                    // Preserve the decoder's scale: authored integer '1.0'
                    // is invalid, even though the native value is integral.
                    Some(NativeScalarKey::PostgresNumeric) => pg_numeric::lexical(&raw),
                    _ => return Err(unsupported()),
                },
            }
        }
    };
    if !exact(datatype) {
        return Ok(format!("(WITH __sf_exact_checked AS MATERIALIZED (SELECT {raw} AS v) SELECT CAST(NULLIF(pg_catalog.length(v),pg_catalog.length(v)) AS TEXT) FROM __sf_exact_checked)"));
    }
    let kind = datatype
        .strip_prefix("http://www.w3.org/2001/XMLSchema#")
        .unwrap();
    let grammar = if kind == "decimal" {
        "^[+-]?([0-9]+([.][0-9]*)?|[.][0-9]+)$"
    } else {
        "^[+-]?[0-9]+$"
    };
    let facets = facets(kind);
    Ok(format!(
        r#"(WITH
      __sf_exact_input AS MATERIALIZED (SELECT (pg_catalog.btrim(CAST({raw} AS TEXT),pg_catalog.chr(32)||pg_catalog.chr(9)||pg_catalog.chr(13)||pg_catalog.chr(10))) COLLATE "C" AS v),
      __sf_exact_digits AS MATERIALIZED (SELECT v, pg_catalog.left(v,1)='-' AS neg,
        pg_catalog.ltrim(pg_catalog.split_part(pg_catalog.ltrim(v,'+-'),'.',1),'0') AS d,
        pg_catalog.rtrim(pg_catalog.split_part(v,'.',2),'0') AS f FROM __sf_exact_input)
      SELECT CASE WHEN v ~ '{grammar}' AND ({facets}) THEN
        (CASE WHEN neg AND (d<>'' OR f<>'') THEN '-' ELSE '' END) || (CASE WHEN d='' THEN '0' ELSE d END) || '.' || f
        ELSE CAST(NULL AS TEXT) END FROM __sf_exact_digits)"#
    ))
}

fn facets(kind: &str) -> String {
    let negative = "(neg AND d<>'')";
    let bounds = match kind {
        "nonPositiveInteger" => return format!("({negative} OR d='')"),
        "negativeInteger" => return negative.into(),
        "nonNegativeInteger" => return format!("NOT {negative}"),
        "positiveInteger" => return format!("(NOT {negative} AND d<>'')"),
        "long" => ("9223372036854775807", "9223372036854775808"),
        "int" => ("2147483647", "2147483648"),
        "short" => ("32767", "32768"),
        "byte" => ("127", "128"),
        "unsignedLong" => ("18446744073709551615", ""),
        "unsignedInt" => ("4294967295", ""),
        "unsignedShort" => ("65535", ""),
        "unsignedByte" => ("255", ""),
        _ => return "TRUE".into(),
    };
    let bound = format!(
        "(CASE WHEN {negative} THEN '{}' ELSE '{}' END)",
        bounds.1, bounds.0
    );
    format!("({bound}<>'' AND (pg_catalog.length(d)<pg_catalog.length({bound}) OR (pg_catalog.length(d)=pg_catalog.length({bound}) AND d<={bound})))")
}

#[cfg(test)]
#[path = "pg_decimal_value_tests.rs"]
mod tests;

#[cfg(test)]
mod admission_tests {
    use super::*;
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    use sf_sql::source_work::SourceWork;

    #[test]
    fn decimal_bindings_preserve_values_and_exact_limits_in_both_backends() {
        for dialect in [Dialect::Postgres, Dialect::MySql] {
            for value in ["123456789012345678901234567890.000", "invalid", "\0"] {
                let term = |v| {
                    LiteralOperand::Constant(sf_core::Literal::new_typed_literal(
                        v,
                        sf_core::datatype::XsdTypeCode::Decimal.iri(),
                    ))
                };
                let cmp = LiteralComparison {
                    left: term(value),
                    right: term("1.0"),
                    value_op: Some(crate::iq::CmpOp::Eq),
                };
                let actuals = ActualColumns::new();
                let catalog = ColumnCatalog::default();
                let run = |work| {
                    let mut params = vec![];
                    let mut index = 0;
                    let sql = if dialect == Dialect::Postgres {
                        comparison_controlled(
                            &cmp,
                            dialect,
                            &actuals,
                            &mut params,
                            &mut index,
                            work,
                        )?
                    } else {
                        mysql_decimal_value::comparison_controlled(
                            &cmp,
                            dialect,
                            &catalog,
                            &actuals,
                            &mut params,
                            &mut index,
                            work,
                        )?
                    };
                    Ok::<_, Error>((sql, params, index))
                };
                let expected = run(SourceWork::new(None)).unwrap();
                assert_eq!(
                    expected.2,
                    if dialect == Dialect::Postgres && value == "\0" {
                        1
                    } else {
                        2
                    }
                );
                let budget =
                    |n| QueryBudget::new(QueryLimits::new(u64::MAX, n, u64::MAX, u64::MAX));
                let measured = budget(u64::MAX);
                assert_eq!(run(SourceWork::new(Some(&measured))).unwrap(), expected);
                let n = measured.consumed(QueryCharge::SourceWork);
                let exact = budget(n);
                let short = budget(n - 1);
                assert_eq!(run(SourceWork::new(Some(&exact))).unwrap(), expected);
                assert!(matches!(
                    run(SourceWork::new(Some(&short))),
                    Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                ));
            }
        }
    }
}
