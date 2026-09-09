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

/// Promote a numeric RDF lexical to double without first narrowing unbounded
/// integer/decimal values to this module's fixed-size exact representations.
/// This is only authority for a comparison whose promoted type is double.
/// Authored xsd:float values round to float *before* widening to double.
pub fn promote_to_double(value: &str, datatype: &str) -> Option<f64> {
    let kind = datatype.strip_prefix("http://www.w3.org/2001/XMLSchema#")?;
    let value = value.trim_matches([' ', '\t', '\r', '\n']);
    let integer = is_integer_datatype(datatype);
    if (!integer && !matches!(kind, "decimal" | "float" | "double"))
        || !valid_lexical(value, if integer { "integer" } else { kind })
        || (integer && !integer_facets(value, kind))
    {
        return None;
    }
    match value {
        "INF" => Some(f64::INFINITY),
        "-INF" => Some(f64::NEG_INFINITY),
        "NaN" => Some(f64::NAN),
        _ if kind == "float" => value.parse::<f32>().ok().map(f64::from),
        _ => value.parse().ok(),
    }
}

/// Standard XSD integer derivations participate in numeric promotion. Authored
/// columns still need their own lexical/decoder and facet-validation proof.
pub fn is_integer_datatype(datatype: &str) -> bool {
    matches!(
        datatype.strip_prefix("http://www.w3.org/2001/XMLSchema#"),
        Some(
            "integer"
                | "nonPositiveInteger"
                | "negativeInteger"
                | "long"
                | "int"
                | "short"
                | "byte"
                | "nonNegativeInteger"
                | "unsignedLong"
                | "unsignedInt"
                | "unsignedShort"
                | "unsignedByte"
                | "positiveInteger"
        )
    )
}

fn integer_facets(value: &str, kind: &str) -> bool {
    let digits = value.trim_start_matches(['+', '-']).trim_start_matches('0');
    let negative = value.starts_with('-') && !digits.is_empty();
    let bounded = |positive: &str, negative_bound: &str| {
        let bound = if negative { negative_bound } else { positive };
        !bound.is_empty()
            && (digits.len() < bound.len() || (digits.len() == bound.len() && digits <= bound))
    };
    match kind {
        "nonPositiveInteger" => negative || digits.is_empty(),
        "negativeInteger" => negative,
        "nonNegativeInteger" => !negative,
        "positiveInteger" => !negative && !digits.is_empty(),
        "long" => bounded("9223372036854775807", "9223372036854775808"),
        "int" => bounded("2147483647", "2147483648"),
        "short" => bounded("32767", "32768"),
        "byte" => bounded("127", "128"),
        "unsignedLong" => bounded("18446744073709551615", ""),
        "unsignedInt" => bounded("4294967295", ""),
        "unsignedShort" => bounded("65535", ""),
        "unsignedByte" => bounded("255", ""),
        "integer" => true,
        _ => false,
    }
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

    #[test]
    fn double_promotion_validates_before_rounding_full_range_lexicals() {
        let promote = |v: &str, kind: &str| {
            promote_to_double(v, &format!("http://www.w3.org/2001/XMLSchema#{kind}"))
        };
        assert_eq!(promote("1.1", "double"), Some(1.1));
        assert_eq!(promote("1.1", "float"), Some(f64::from(1.1_f32)));
        assert_ne!(promote("1.1", "float"), promote("1.1", "double"));
        assert_eq!(
            promote("9007199254740993", "integer"),
            Some(9007199254740992.0)
        );
        assert_eq!(promote(&"9".repeat(400), "integer"), Some(f64::INFINITY));
        assert_eq!(
            promote(&format!("0.{}1", "0".repeat(400)), "decimal"),
            Some(0.0)
        );
        assert_eq!(
            promote("  -1e-9999\t", "double").unwrap().to_bits(),
            (-0.0_f64).to_bits()
        );
        assert_eq!(promote("5e-324", "double"), Some(f64::from_bits(1)));
        assert_eq!(promote("INF", "float"), Some(f64::INFINITY));
        assert!(promote("NaN", "double").unwrap().is_nan());
        for value in [
            "inf", "nan", "Infinity", "+INF", "1e", ".", "1_0", "\u{a0}1",
        ] {
            assert_eq!(promote(value, "double"), None, "{value}");
        }
        assert_eq!(promote("1e2", "integer"), None);
        assert_eq!(promote("1e2", "decimal"), None);
        assert_eq!(promote("1", "string"), None);
        for (kind, lower, upper) in [
            ("long", "-9223372036854775808", "9223372036854775807"),
            ("int", "-2147483648", "2147483647"),
            ("short", "-32768", "32767"),
            ("byte", "-128", "127"),
            ("unsignedLong", "0", "18446744073709551615"),
            ("unsignedInt", "0", "4294967295"),
            ("unsignedShort", "0", "65535"),
            ("unsignedByte", "0", "255"),
        ] {
            assert_eq!(promote(lower, kind), lower.parse().ok());
            assert_eq!(promote(upper, kind), upper.parse().ok());
            assert_eq!(
                promote(&(lower.parse::<i128>().unwrap() - 1).to_string(), kind),
                None
            );
            assert_eq!(
                promote(&(upper.parse::<i128>().unwrap() + 1).to_string(), kind),
                None
            );
        }
        assert_eq!(promote(" +0001 ", "int"), Some(1.0));
        assert_eq!(promote("-000", "unsignedByte"), Some(-0.0));
        for kind in ["negativeInteger", "positiveInteger"] {
            assert_eq!(promote("-0", kind), None);
        }
        assert_eq!(promote("-1", "positiveInteger"), None);
        assert_eq!(promote("1", "nonPositiveInteger"), None);
        assert_eq!(promote("1", "negativeInteger"), None);
        assert_eq!(promote("-1", "nonNegativeInteger"), None);
        assert_eq!(
            promote(&"9".repeat(400), "positiveInteger"),
            Some(f64::INFINITY)
        );
        assert_eq!(
            promote(&format!("-{}", "9".repeat(400)), "negativeInteger"),
            Some(f64::NEG_INFINITY)
        );
    }
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
