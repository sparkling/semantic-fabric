//! IEEE numeric value comparison over retained MySQL decoder lexicals.
use super::*;
use crate::iq::literal_cmp::{LiteralComparison, LiteralOperand};
use pg_decimal_value::datatype;
#[path = "mysql_double_lexical.rs"]
mod double_lexical;
#[path = "mysql_float_identity.rs"]
pub(super) mod identity;
#[path = "mysql_float_round.rs"]
mod round;
#[path = "mysql_float_shortest.rs"]
mod shortest;
#[path = "mysql_float_text.rs"]
mod text;

pub(super) fn float_lexical(raw: &str) -> String {
    shortest::float_lexical(raw)
}

pub(super) fn double_lexical(raw: &str) -> String {
    double_lexical::lexical(raw)
}

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
    let double = types.contains(&Some("http://www.w3.org/2001/XMLSchema#double"));
    if dialect != Dialect::MySql
        || (!double && !types.contains(&Some("http://www.w3.org/2001/XMLSchema#float")))
    {
        return Ok(None);
    }
    let left = operand(&cmp.left, double, catalog, actuals, params, pidx)?;
    let right = operand(&cmp.right, double, catalog, actuals, params, pidx)?;
    let nan = if op == crate::iq::CmpOp::Ne {
        "TRUE"
    } else {
        "FALSE"
    };
    // Tags: -1/-Inf, 0/finite, 1/+Inf, 2/NaN, 3/expression error.
    // JSON's native J_DOUBLE carrier preserves all 64 bits. Never stringify it.
    // Both one-row operands materialize before error/NaN dispatch, preserving
    // validation of authorized source values even opposite an invalid literal.
    Ok(Some(format!(
        r#"(WITH
      __sf_ieee_values AS (SELECT {left} AS l,{right} AS r LIMIT 18446744073709551615),
      __sf_ieee_parts AS (SELECT CAST(JSON_EXTRACT(l,'$[0]') AS SIGNED) AS lt,
        CAST(JSON_EXTRACT(r,'$[0]') AS SIGNED) AS rt,
        CAST(JSON_EXTRACT(l,'$[1]') AS DOUBLE) AS lv,
        CAST(JSON_EXTRACT(r,'$[1]') AS DOUBLE) AS rv FROM __sf_ieee_values LIMIT 18446744073709551615)
      SELECT CASE WHEN lt=3 OR rt=3 THEN NULL WHEN lt=2 OR rt=2 THEN {nan}
        ELSE (CASE WHEN lt<rt THEN -1 WHEN lt>rt THEN 1 WHEN lt<>0 THEN 0
          WHEN lv<rv THEN -1 WHEN lv>rv THEN 1 ELSE 0 END) {} 0 END FROM __sf_ieee_parts)"#,
        op.as_sql()
    )))
}

fn operand(
    value: &LiteralOperand,
    double: bool,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    if let LiteralOperand::Constant(literal) = value {
        let number = if double {
            sf_core::numeric_compare::promote_to_double(
                literal.value(),
                literal.datatype().as_str(),
            )
        } else {
            sf_core::numeric_compare::promote_to_float(literal.value(), literal.datatype().as_str())
                .map(f64::from)
        };
        let tag = match number {
            None => 3,
            Some(v) if v.is_nan() => 2,
            Some(v) if v == f64::INFINITY => 1,
            Some(v) if v == f64::NEG_INFINITY => -1,
            _ => 0,
        };
        return Ok(if tag == 0 {
            params.push(number.unwrap().to_string());
            *pidx += 1;
            "JSON_ARRAY(0,CAST(? AS DOUBLE))".into()
        } else {
            format!("JSON_ARRAY({tag},CAST(0 AS DOUBLE))")
        });
    }
    let datatype = datatype(value, actuals).ok_or_else(|| Error::Unsupported("MySQL floating comparison requires compatible decoder and retained datatype provenance".into()))?;
    if let LiteralOperand::Column { column, spec } = value {
        use sf_core::datatype::XsdTypeCode::Double;
        if iri_cmp::scalar_column(column, actuals) == Some(NativeScalarKey::MysqlFloat4)
            && literal_datatype::fact(column, actuals) == Some(Some(Double))
        {
            let raw = colref(column, Dialect::MySql, actuals);
            if spec.uses_natural_type(Some(Double)) {
                return Ok(shortest::float_value(&raw));
            }
            if spec.language.is_none()
                && spec.datatype.as_ref().map(|value| value.as_str())
                    == Some("http://www.w3.org/2001/XMLSchema#float")
            {
                return Ok(format!("JSON_ARRAY(CASE WHEN {raw} IS NULL THEN 3 ELSE 0 END,CAST(COALESCE({raw},0) AS DOUBLE))"));
            }
        }
        if iri_cmp::scalar_column(column, actuals) == Some(NativeScalarKey::MysqlFloat8)
            && literal_datatype::fact(column, actuals) == Some(Some(Double))
        {
            let raw = colref(column, Dialect::MySql, actuals);
            if spec.uses_natural_type(Some(Double)) {
                // The wire f64's Rust shortest spelling parses back to this
                // exact finite binary64 value; not lexical identity authority.
                return Ok(format!("JSON_ARRAY(CASE WHEN {raw} IS NULL THEN 3 ELSE 0 END,CAST(COALESCE({raw},0) AS DOUBLE))"));
            }
            if spec.language.is_none()
                && spec.datatype.as_ref().map(|value| value.as_str())
                    == Some("http://www.w3.org/2001/XMLSchema#float")
            {
                return Ok(shortest::double_as_float(&raw));
            }
        }
    }
    let raw = mysql_decimal_value::raw_operand(value, catalog, actuals, params, pidx)?;
    let kind = datatype
        .strip_prefix("http://www.w3.org/2001/XMLSchema#")
        .unwrap_or("");
    if !pg_decimal_value::exact(datatype) && !matches!(kind, "float" | "double") {
        return Ok(format!("(WITH __sf_ieee_checked AS (SELECT {raw} AS v LIMIT 18446744073709551615) SELECT JSON_ARRAY(3,CAST(COALESCE(NULLIF(LENGTH(v),LENGTH(v)),0) AS DOUBLE)) FROM __sf_ieee_checked)"));
    }
    Ok(text::operand(&raw, kind, double && kind != "float"))
}

#[cfg(test)]
#[path = "mysql_float_value_tests.rs"]
mod tests;
