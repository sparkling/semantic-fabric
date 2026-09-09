//! Exact numeric values over qualified MySQL decoder lexicals, without narrowing.
use super::*;
use crate::iq::literal_cmp::{LiteralComparison, LiteralOperand};
use pg_decimal_value::{datatype, exact};

pub(super) fn comparison(
    cmp: &LiteralComparison,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<Option<String>> {
    let Some(op) = cmp.value_op else {
        return Ok(None);
    };
    let types = [datatype(&cmp.left, actuals), datatype(&cmp.right, actuals)];
    if dialect != Dialect::MySql
        || !types.iter().any(|dt| dt.is_some_and(exact))
        || types.iter().any(|dt| {
            matches!(
                *dt,
                Some(
                    "http://www.w3.org/2001/XMLSchema#float"
                        | "http://www.w3.org/2001/XMLSchema#double"
                )
            )
        })
    {
        return Ok(None);
    }
    let left = operand(&cmp.left, catalog, actuals, params, pidx)?;
    let right = operand(&cmp.right, catalog, actuals, params, pidx)?;
    // Each stage is one row with nullable cells, never a WHERE-filtered row.
    // LIMIT blocks merging/pushdown; both cells retain source validations.
    // Digit strings are binary only AFTER UTF8 regex normalization (MySQL's
    // regex engine rejects binary input). No collation-dependent ordering.
    Ok(Some(format!(
        r#"(WITH
      __sf_exact_values AS (SELECT {left} AS l, {right} AS r LIMIT 18446744073709551615),
      __sf_exact_parts AS (SELECT l,r,LEFT(l,1)='-' AS ln,LEFT(r,1)='-' AS rn,
        CAST(SUBSTRING_INDEX(TRIM(LEADING '-' FROM l),'.',1) AS BINARY) AS li,
        CAST(SUBSTRING_INDEX(l,'.',-1) AS BINARY) AS lf,
        CAST(SUBSTRING_INDEX(TRIM(LEADING '-' FROM r),'.',1) AS BINARY) AS ri,
        CAST(SUBSTRING_INDEX(r,'.',-1) AS BINARY) AS rf FROM __sf_exact_values LIMIT 18446744073709551615),
      __sf_exact_order AS (SELECT l,r,ln,rn,
        CASE WHEN (LENGTH(li),li,lf) = (LENGTH(ri),ri,rf) THEN 0
          WHEN (LENGTH(li),li,lf) < (LENGTH(ri),ri,rf) THEN -1 ELSE 1 END AS magnitude FROM __sf_exact_parts LIMIT 18446744073709551615)
      SELECT CASE WHEN l IS NULL OR r IS NULL THEN NULL ELSE
        (CASE WHEN ln AND NOT rn THEN -1 WHEN rn AND NOT ln THEN 1 WHEN ln THEN -magnitude ELSE magnitude END) {} 0 END FROM __sf_exact_order)"#,
        op.as_sql()
    )))
}

fn operand(
    value: &LiteralOperand,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    let unsupported = || {
        Error::Unsupported("exact MySQL numeric comparison requires compatible decoder and retained datatype provenance".into())
    };
    let datatype = datatype(value, actuals).ok_or_else(unsupported)?;
    let raw = match value {
        LiteralOperand::Constant(literal) => {
            if !exact(datatype) {
                return Ok("CAST(NULL AS CHAR)".into());
            }
            params.push(literal.value().to_owned());
            *pidx += 1;
            "CONVERT(? USING utf8mb4)".into()
        }
        LiteralOperand::Column { column, spec } => {
            // A UNION's incompatible marker revokes source authority even if
            // an authored type and a generic Integer scalar key remain.
            if literal_datatype::fact(column, actuals) == Some(None) {
                return Err(unsupported());
            }
            let raw = colref(column, Dialect::MySql, actuals);
            match path_comparison::column_text(column, actuals) {
                Some(TextKey::Verbatim) => format!("CONVERT({raw} USING utf8mb4)"),
                Some(_) => return Err(unsupported()),
                None => match iri_cmp::scalar_column(column, actuals) {
                    Some(key @ (NativeScalarKey::Integer | NativeScalarKey::MysqlDecimal)) => {
                        let natural = literal_datatype::fact(column, actuals).flatten();
                        if key == NativeScalarKey::Integer
                            && (!matches!(
                                natural,
                                Some(
                                    sf_core::datatype::XsdTypeCode::Integer
                                        | sf_core::datatype::XsdTypeCode::Boolean
                                )
                            ) || natural_literal::column_code(column, actuals) != natural)
                        {
                            return Err(unsupported());
                        }
                        if spec.uses_natural_type(natural)
                            && matches!(
                                natural,
                                Some(
                                    sf_core::datatype::XsdTypeCode::Integer
                                        | sf_core::datatype::XsdTypeCode::Boolean
                                )
                            )
                        {
                            native_literal_key::key(
                                column,
                                natural.unwrap(),
                                Dialect::MySql,
                                catalog,
                                actuals,
                            )?
                        } else {
                            iri_cmp::scalar_lexical(key, &raw, Dialect::MySql)?
                        }
                    }
                    _ => return Err(unsupported()),
                },
            }
        }
    };
    if !exact(datatype) {
        return Ok(format!("(WITH __sf_exact_checked AS (SELECT {raw} AS v LIMIT 18446744073709551615) SELECT CAST(NULLIF(LENGTH(v),LENGTH(v)) AS CHAR) FROM __sf_exact_checked)"));
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
    // Extra alphabet rejection makes '$' safe: ICU also matches just before a
    // final line terminator. Only XSD's four ASCII whitespace bytes are trimmed.
    Ok(format!(
        r#"(WITH
      __sf_exact_input AS (SELECT (REGEXP_REPLACE(CONVERT(({raw}) USING utf8mb4),CONCAT('^[',CONVERT(CHAR(32,9,13,10) USING utf8mb4),']+|[',CONVERT(CHAR(32,9,13,10) USING utf8mb4),']+$'),'',1,0,'c')) COLLATE utf8mb4_bin AS v LIMIT 18446744073709551615),
      __sf_exact_unsigned AS (SELECT v,LEFT(v,1)='-' AS neg,CASE WHEN LEFT(v,1) IN ('+','-') THEN SUBSTRING(v,2) ELSE v END AS u FROM __sf_exact_input LIMIT 18446744073709551615),
      __sf_exact_digits AS (SELECT v,neg,TRIM(LEADING '0' FROM SUBSTRING_INDEX(u,'.',1)) AS d,
        CASE WHEN LOCATE('.',u)>0 THEN TRIM(TRAILING '0' FROM SUBSTRING_INDEX(u,'.',-1)) ELSE '' END AS f FROM __sf_exact_unsigned LIMIT 18446744073709551615)
      SELECT CASE WHEN REGEXP_LIKE(v,'{grammar}','c') AND NOT REGEXP_LIKE(v,'[^0-9+.-]','c') AND ({facets}) THEN
        CONCAT(CASE WHEN neg AND (d<>'' OR f<>'') THEN '-' ELSE '' END,CASE WHEN d='' THEN '0' ELSE d END,'.',f)
        ELSE CAST(NULL AS CHAR) END FROM __sf_exact_digits)"#
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
    format!("({bound}<>'' AND (LENGTH(d)<LENGTH({bound}) OR (LENGTH(d)=LENGTH({bound}) AND CAST(d AS BINARY)<=CAST({bound} AS BINARY))))")
}

#[cfg(test)]
#[path = "mysql_decimal_value_tests.rs"]
mod tests;
