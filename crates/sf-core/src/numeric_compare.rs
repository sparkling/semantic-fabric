//! SPARQL numeric promotion, separate from RDF term identity and total ordering.
use crate::{Error, Result};
use oxsdatatypes::{Decimal, Double, Float, Integer};

#[derive(Clone, Copy, Debug)]
pub enum NumericOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy)]
enum Number {
    Integer(Integer),
    Decimal(Decimal),
    Float(Float),
    Double(Double),
}

/// None is an expression type/lexical error. Representational overflow fails
/// closed instead of falsely declaring a valid XSD value unequal.
pub fn compare(
    left: &str,
    left_type: &str,
    right: &str,
    right_type: &str,
    op: NumericOp,
) -> Result<Option<bool>> {
    let (Some(a), Some(b)) = (parse(left, left_type)?, parse(right, right_type)?) else {
        return Ok(None);
    };
    use Number::*;
    Ok(Some(match (a, b) {
        (Double(_), _) | (_, Double(_)) => apply(a.double()?, b.double()?, op),
        (Float(_), _) | (_, Float(_)) => apply(a.float()?, b.float()?, op),
        (Decimal(_), _) | (_, Decimal(_)) => apply(a.decimal(), b.decimal(), op),
        (Integer(a), Integer(b)) => apply(a, b, op),
    }))
}

fn apply<T: PartialOrd>(a: T, b: T, op: NumericOp) -> bool {
    match op {
        NumericOp::Eq => a == b,
        NumericOp::Ne => a != b,
        NumericOp::Lt => a < b,
        NumericOp::Le => a <= b,
        NumericOp::Gt => a > b,
        NumericOp::Ge => a >= b,
    }
}

fn range_error() -> Error {
    Error::Datatype("numeric comparison exceeds supported representation".into())
}

impl Number {
    fn double(self) -> Result<Double> {
        Ok(match self {
            Self::Integer(v) => Double::from(v),
            Self::Decimal(v) => v.to_string().parse().map_err(|_| range_error())?,
            Self::Float(v) => Double::from(v),
            Self::Double(v) => v,
        })
    }
    fn float(self) -> Result<Float> {
        Ok(match self {
            Self::Integer(v) => Float::from(v),
            Self::Decimal(v) => v.to_string().parse().map_err(|_| range_error())?,
            Self::Float(v) => v,
            Self::Double(_) => return Err(range_error()),
        })
    }
    fn decimal(self) -> Decimal {
        match self {
            Self::Integer(v) => Decimal::from(v),
            Self::Decimal(v) => v,
            _ => unreachable!("numeric promotion"),
        }
    }
}

fn parse(value: &str, datatype: &str) -> Result<Option<Number>> {
    let Some(kind) = datatype.strip_prefix("http://www.w3.org/2001/XMLSchema#") else {
        return Ok(None);
    };
    let value = value.trim_matches([' ', '\t', '\r', '\n']);
    if !matches!(kind, "integer" | "decimal" | "float" | "double") || !valid_lexical(value, kind) {
        return Ok(None);
    }
    Ok(Some(match kind {
        "integer" => Number::Integer(value.parse().map_err(|_| range_error())?),
        "decimal" => Number::Decimal(value.parse().map_err(|_| range_error())?),
        "float" => Number::Float(value.parse().map_err(|_| range_error())?),
        "double" => Number::Double(value.parse().map_err(|_| range_error())?),
        _ => unreachable!(),
    }))
}

fn valid_lexical(value: &str, kind: &str) -> bool {
    let floating = matches!(kind, "float" | "double");
    if floating && matches!(value, "INF" | "-INF" | "NaN") {
        return true;
    }
    let bytes = value.as_bytes();
    let mut at = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        at += 1;
    }
    let start = at;
    while bytes.get(at).is_some_and(u8::is_ascii_digit) {
        at += 1;
    }
    let mut digits = at - start;
    if kind != "integer" && bytes.get(at) == Some(&b'.') {
        at += 1;
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        digits += at - start;
    }
    if digits == 0 {
        return false;
    }
    if floating && matches!(bytes.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if at == start {
            return false;
        }
    }
    at == bytes.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(a: &str, at: &str, b: &str, bt: &str, op: NumericOp) -> Option<bool> {
        compare(
            a,
            &format!("http://www.w3.org/2001/XMLSchema#{at}"),
            b,
            &format!("http://www.w3.org/2001/XMLSchema#{bt}"),
            op,
        )
        .unwrap()
    }
    #[test]
    fn exact_numeric_promotion_and_nan_are_not_lexical_or_total_order() {
        for (a, at, b, bt) in [
            ("9007199254740993", "integer", "9007199254740992", "integer"),
            (
                "1.000000000000000002",
                "decimal",
                "1.000000000000000001",
                "decimal",
            ),
            ("1.000000059604644776", "decimal", "1", "float"),
            ("10", "integer", "9", "decimal"),
        ] {
            assert_eq!(check(a, at, b, bt, NumericOp::Gt), Some(true));
            assert_eq!(check(b, bt, a, at, NumericOp::Lt), Some(true));
        }
        assert_eq!(
            check("16777217", "integer", "16777216", "float", NumericOp::Eq),
            Some(true)
        );
        assert_eq!(
            check("-0", "double", "0", "integer", NumericOp::Eq),
            Some(true)
        );
        for op in [
            NumericOp::Eq,
            NumericOp::Lt,
            NumericOp::Le,
            NumericOp::Gt,
            NumericOp::Ge,
        ] {
            assert_eq!(check("NaN", "double", "NaN", "double", op), Some(false));
        }
        assert_eq!(
            check("NaN", "double", "0", "double", NumericOp::Ne),
            Some(true)
        );
        assert_eq!(
            check("INF", "float", "1", "double", NumericOp::Gt),
            Some(true)
        );
    }
    #[test]
    fn lexical_errors_and_representation_limits_are_distinct() {
        for value in ["inf", "nan", "Infinity", "+INF", "1e", ".", "--1", "1_0"] {
            assert_eq!(
                check(value, "double", "0", "double", NumericOp::Eq),
                None,
                "{value}"
            );
        }
        assert_eq!(
            check("1e2", "integer", "100", "integer", NumericOp::Eq),
            None
        );
        assert_eq!(
            check("  +1.0\t", "decimal", "1", "integer", NumericOp::Eq),
            Some(true)
        );
        assert!(compare(
            "9223372036854775808",
            "http://www.w3.org/2001/XMLSchema#integer",
            "1",
            "http://www.w3.org/2001/XMLSchema#integer",
            NumericOp::Eq
        )
        .is_err());
    }
}
