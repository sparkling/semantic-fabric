//! Qualified RDF floating value comparison, not native SQL identity or ordering.
use super::*;
use crate::iq::literal_cmp::{LiteralComparison, LiteralOperand};
use sf_core::datatype::XsdTypeCode;
#[path = "pg_float_text.rs"]
mod text;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Integer,
    Decimal,
    Float,
    Double,
    Nonnumeric,
    Unknown,
}

#[derive(Clone, Copy)]
enum Promotion {
    Float,
    Double,
}

impl Promotion {
    fn sql_type(self) -> &'static str {
        match self {
            Self::Float => "REAL",
            Self::Double => "DOUBLE PRECISION",
        }
    }

    fn constant(self, value: &str, datatype: &str) -> Option<String> {
        match self {
            Self::Float => {
                sf_core::numeric_compare::promote_to_float(value, datatype).map(|v| v.to_string())
            }
            Self::Double => {
                sf_core::numeric_compare::promote_to_double(value, datatype).map(|v| v.to_string())
            }
        }
    }
}

fn kind(value: &LiteralOperand, actuals: &ActualColumns) -> Kind {
    let datatype = match value {
        LiteralOperand::Constant(literal) => Some(literal.datatype().as_str()),
        LiteralOperand::Column { column, spec } => {
            if spec.language.is_some() {
                return Kind::Nonnumeric;
            }
            spec.datatype.as_ref().map(|dt| dt.as_str()).or_else(|| {
                literal_datatype::fact(column, actuals)
                    .flatten()
                    .map(|code| code.iri().as_str())
            })
        }
    };
    match datatype.and_then(|dt| dt.strip_prefix("http://www.w3.org/2001/XMLSchema#")) {
        _ if datatype.is_some_and(sf_core::numeric_compare::is_integer_datatype) => Kind::Integer,
        Some("decimal") => Kind::Decimal,
        Some("float") => Kind::Float,
        Some("double") => Kind::Double,
        _ if datatype.is_some() => Kind::Nonnumeric,
        _ => Kind::Unknown,
    }
}

pub(super) fn comparison(
    cmp: &LiteralComparison,
    dialect: Dialect,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<Option<String>> {
    let Some(op) = cmp.value_op else {
        return Ok(None);
    };
    if dialect != Dialect::Postgres {
        return Ok(None);
    }
    let kinds = [kind(&cmp.left, actuals), kind(&cmp.right, actuals)];
    let promotion = if kinds.contains(&Kind::Double) {
        Promotion::Double
    } else if kinds.contains(&Kind::Float) {
        Promotion::Float
    } else {
        return Ok(None);
    };
    let sql_type = promotion.sql_type();
    let left = operand(&cmp.left, promotion, actuals, params, pidx)?;
    let right = operand(&cmp.right, promotion, actuals, params, pidx)?;
    let nan_result = if op == crate::iq::CmpOp::Ne {
        "TRUE"
    } else {
        "FALSE"
    };
    // Evaluate both decoder obligations before NULL/NaN short-circuiting. A
    // source NUMERIC NaN is an invalid RDF decimal, not a floating NaN value.
    Ok(Some(format!("(WITH __sf_double_values AS MATERIALIZED (SELECT {left} AS l, {right} AS r) SELECT CASE WHEN l IS NULL OR r IS NULL THEN NULL WHEN l = CAST('NaN' AS {sql_type}) OR r = CAST('NaN' AS {sql_type}) THEN {nan_result} ELSE l {} r END FROM __sf_double_values)", op.as_sql())))
}

fn operand(
    value: &LiteralOperand,
    promotion: Promotion,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    let sql_type = promotion.sql_type();
    let null = format!("CAST(NULL AS {sql_type})");
    if let LiteralOperand::Constant(literal) = value {
        return Ok(
            match promotion.constant(literal.value(), literal.datatype().as_str()) {
                Some(value) => {
                    params.push(value);
                    *pidx += 1;
                    format!("CAST(${} AS {sql_type})", *pidx)
                }
                None => null,
            },
        );
    }
    let LiteralOperand::Column { column, spec } = value else {
        unreachable!()
    };
    let unsupported = || {
        Error::Unsupported(
            "floating value comparison requires each numeric operand's exact native decoder".into(),
        )
    };
    let raw = colref(column, Dialect::Postgres, actuals);
    let scalar = iri_cmp::scalar_column(column, actuals);
    if spec.language.is_none() {
        if let (Some(datatype), Some(key)) = (
            &spec.datatype,
            path_comparison::column_text(column, actuals),
        ) {
            if let Some(value) = text::operand(&raw, key, datatype.as_str(), promotion) {
                return Ok(value);
            }
        }
    }
    if spec.language.is_none()
        && spec
            .datatype
            .as_ref()
            .is_some_and(|dt| dt.as_str() == "http://www.w3.org/2001/XMLSchema#float")
    {
        return authored_float(&raw, scalar.ok_or_else(unsupported)?, promotion);
    }
    if spec.language.is_none()
        && spec
            .datatype
            .as_ref()
            .is_some_and(|dt| dt.as_str() == XsdTypeCode::Double.iri().as_str())
        && matches!(promotion, Promotion::Double)
    {
        // INTEGER/NUMERIC wire lexicals are parsed directly as Double. This
        // override does not borrow natural identity or revive float metadata.
        match scalar {
            Some(NativeScalarKey::Integer) => return Ok(format!("CAST({raw} AS {sql_type})")),
            Some(NativeScalarKey::PostgresNumeric) => return Ok(numeric(&raw, promotion)),
            _ => {}
        }
    }
    if kind(value, actuals) == Kind::Nonnumeric {
        return Ok(if scalar == Some(NativeScalarKey::PostgresNumeric) {
            // Even an actual datatype override consumes the NUMERIC decoder.
            // Refer to the checked field: a constant-NULL projection can be
            // pruned and would hide an invalid authorized source value.
            format!("(WITH __sf_checked_numeric AS MATERIALIZED (SELECT {} AS lexical) SELECT CAST(NULLIF(length(lexical), length(lexical)) AS {sql_type}) FROM __sf_checked_numeric)", pg_numeric::lexical(&raw))
        } else if matches!(
            path_comparison::column_text(column, actuals),
            Some(TextKey::Verbatim | TextKey::PostgresCharacter)
        ) || matches!(
            scalar,
            Some(
                NativeScalarKey::Integer
                    | NativeScalarKey::PostgresBoolean
                    | NativeScalarKey::PostgresBytea
                    | NativeScalarKey::PostgresFloat4
                    | NativeScalarKey::PostgresFloat8
            )
        ) {
            null
        } else {
            return Err(unsupported());
        });
    }
    match (natural_literal::natural(value, actuals), scalar) {
        (Some(XsdTypeCode::Double), Some(NativeScalarKey::PostgresFloat4))
            if matches!(promotion, Promotion::Double) =>
        {
            // The product constructs Double from Rust's f32 shortest decimal,
            // not the binary widening used by PostgreSQL's REAL comparison.
            Ok(format!(
                "CAST({} AS DOUBLE PRECISION)",
                pg_float::lexical(&raw, NativeScalarKey::PostgresFloat4, false)
            ))
        }
        (Some(XsdTypeCode::Double), Some(NativeScalarKey::PostgresFloat8))
            if matches!(promotion, Promotion::Double) =>
        {
            Ok(raw)
        }
        (Some(XsdTypeCode::Integer), Some(NativeScalarKey::Integer)) => {
            Ok(format!("CAST({raw} AS {sql_type})"))
        }
        (Some(XsdTypeCode::Decimal), Some(NativeScalarKey::PostgresNumeric)) => {
            Ok(numeric(&raw, promotion))
        }
        // An authored numeric override or a coercing SQL pool cannot borrow a
        // source's natural recipe. Keep unsupported paths explicit, not raw SQL.
        _ => Err(unsupported()),
    }
}

fn authored_float(raw: &str, scalar: NativeScalarKey, promotion: Promotion) -> Result<String> {
    // This is a proof about parsing the original wire lexical as xsd:float,
    // not permission to replace the RDF literal or reuse natural Double keys.
    let value = match scalar {
        NativeScalarKey::Integer => format!("CAST({raw} AS REAL)"),
        NativeScalarKey::PostgresNumeric => numeric(raw, Promotion::Float),
        NativeScalarKey::PostgresFloat4 | NativeScalarKey::PostgresFloat8 => {
            let finite = if scalar == NativeScalarKey::PostgresFloat4 {
                raw.to_owned()
            } else {
                // f64's shortest decimal can lie across an f32 midpoint from
                // the exact binary value. A direct FLOAT8->REAL cast is wrong.
                numeric(
                    &format!("CAST({} AS NUMERIC)", pg_float::lexical(raw, scalar, false)),
                    Promotion::Float,
                )
            };
            // Rust's raw infinity lexicals are inf/-inf, not valid XSD INF.
            format!("CASE WHEN {raw} IN (CAST('Infinity' AS DOUBLE PRECISION), CAST('-Infinity' AS DOUBLE PRECISION)) THEN CAST(NULL AS REAL) WHEN {raw} = CAST('NaN' AS DOUBLE PRECISION) THEN CAST('NaN' AS REAL) ELSE {finite} END")
        }
        _ => {
            return Err(Error::Unsupported(
                "authored float requires an exact native PostgreSQL numeric decoder".into(),
            ))
        }
    };
    Ok(match promotion {
        Promotion::Float => value,
        Promotion::Double => format!("CAST(({value}) AS DOUBLE PRECISION)"),
    })
}

fn numeric(raw: &str, promotion: Promotion) -> String {
    let validated = pg_numeric::lexical(raw);
    let sql_type = promotion.sql_type();
    let (overflow_power, half_ulp_power, underflow_power) = match promotion {
        Promotion::Float => (128, 103, 150),
        Promotion::Double => (1024, 970, 1075),
    };
    // Correct nearest-even boundaries, expressed as exact NUMERIC integers.
    // A PostgreSQL numeric->float cast errors on infinity/zero underflow, while
    // RDF double promotion returns the corresponding signed IEEE value.
    format!("(WITH __sf_decimal_value AS MATERIALIZED (SELECT CAST({validated} AS NUMERIC) AS n), __sf_double_bounds AS MATERIALIZED (SELECT pg_catalog.power(CAST(2 AS NUMERIC), {overflow_power}) - pg_catalog.power(CAST(2 AS NUMERIC), {half_ulp_power}) AS overflow, CAST(CAST(pg_catalog.trim_scale(pg_catalog.power(CAST(5 AS NUMERIC), {underflow_power})) AS TEXT) || 'e-{underflow_power}' AS NUMERIC) AS underflow) SELECT CASE WHEN n IS NULL THEN NULL WHEN pg_catalog.abs(n) >= overflow THEN CASE WHEN n < 0 THEN CAST('-Infinity' AS {sql_type}) ELSE CAST('Infinity' AS {sql_type}) END WHEN pg_catalog.abs(n) <= underflow THEN CASE WHEN n < 0 THEN CAST('-0' AS {sql_type}) ELSE CAST('0' AS {sql_type}) END ELSE CAST(n AS {sql_type}) END FROM __sf_decimal_value CROSS JOIN __sf_double_bounds)")
}

#[cfg(test)]
#[path = "pg_float_value_tests.rs"]
mod tests;
