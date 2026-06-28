//! The R2RML §10 natural SQL→XSD datatype mapping and the XSD-canonical literal
//! formatting chokepoint (ADR-0003 R3, ADR-0015). Lives in `sf-core` exactly
//! once so datatype semantics cannot drift across the engine.
//!
//! [`natural_xsd`] is the §10 lookup (SQL type name → [`XsdTypeCode`]).
//! [`canonical_lexical`] is the single Rust chokepoint that turns a raw value
//! into its XSD-canonical lexical form, written through a reusable buffer. It
//! formats via `oxsdatatypes` (parse → canonical `Display`), **never** `ryu` /
//! shortest-round-trip — which is not XSD-canonical and would be a conformance
//! bug (ADR-0015).
//!
//! Caveat for `xsd:double`: `oxsdatatypes::Double`'s `Display` is documented as
//! *not* canonical (it delegates to `f64`, e.g. `1` not `1.0E0`). R2RML §10
//! requires `E`-notation everywhere, so this module parses/validates via
//! `oxsdatatypes::Double` and then emits the canonical scientific form itself
//! (still not `ryu`). Every other type's `oxsdatatypes` `Display` *is* canonical.

use std::borrow::Cow;
use std::fmt::Write;

use oxrdf::{vocab::xsd, NamedNodeRef};
use oxsdatatypes::{Boolean, Date, DateTime, Decimal, Double, Integer, Time};

use crate::{Error, Result};

/// The XSD datatype a SQL value maps to under the R2RML §10 natural mapping.
/// `String` is the plain (`xsd:string`) literal case (character SQL types).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XsdTypeCode {
    String,
    HexBinary,
    Decimal,
    Integer,
    Double,
    Boolean,
    Date,
    Time,
    DateTime,
}

impl XsdTypeCode {
    /// The XSD datatype IRI, by reference from the `oxrdf` vocab (zero-copy).
    pub const fn iri(self) -> NamedNodeRef<'static> {
        match self {
            XsdTypeCode::String => xsd::STRING,
            XsdTypeCode::HexBinary => xsd::HEX_BINARY,
            XsdTypeCode::Decimal => xsd::DECIMAL,
            XsdTypeCode::Integer => xsd::INTEGER,
            XsdTypeCode::Double => xsd::DOUBLE,
            XsdTypeCode::Boolean => xsd::BOOLEAN,
            XsdTypeCode::Date => xsd::DATE,
            XsdTypeCode::Time => xsd::TIME,
            XsdTypeCode::DateTime => xsd::DATE_TIME,
        }
    }
}

/// The R2RML §10 *natural mapping* of an SQL column type to its RDF datatype.
///
/// Covers the SQL-standard names of §10 plus their ubiquitous aliases. `INTERVAL`
/// is left *undefined* by §10 and unknown types are unmapped — both return
/// `None`, and the caller falls back to a plain literal. Dialect-specific names
/// (`int4`, `float8`, `tinyint(1)`, `timestamptz`, …) are resolved by `sf-sql`'s
/// per-dialect `DbTypeMap` from catalog metadata (ADR-0015), not here.
pub fn natural_xsd(sql_type: &str) -> Option<XsdTypeCode> {
    use XsdTypeCode::*;
    match normalize_sql_type(sql_type).as_str() {
        "CHARACTER" | "CHARACTER VARYING" | "CHAR" | "VARCHAR" | "CLOB" | "NCHAR"
        | "NCHAR VARYING" | "NVARCHAR" | "NCLOB" | "TEXT" => Some(String),
        "BINARY" | "BINARY VARYING" | "VARBINARY" | "BINARY LARGE OBJECT" | "BLOB" => {
            Some(HexBinary)
        }
        "NUMERIC" | "DECIMAL" | "DEC" => Some(Decimal),
        "SMALLINT" | "INTEGER" | "INT" | "BIGINT" => Some(Integer),
        "FLOAT" | "REAL" | "DOUBLE PRECISION" | "DOUBLE" => Some(Double),
        "BOOLEAN" | "BOOL" => Some(Boolean),
        "DATE" => Some(Date),
        "TIME" => Some(Time),
        "TIMESTAMP" => Some(DateTime),
        _ => None, // INTERVAL (§10 undefined) and anything unrecognised
    }
}

/// Uppercase, drop any `(size[,scale])` modifier, and collapse internal
/// whitespace so e.g. `"character varying(255)"` → `"CHARACTER VARYING"`.
fn normalize_sql_type(sql_type: &str) -> String {
    let base = sql_type.split('(').next().unwrap_or(sql_type);
    let mut out = String::with_capacity(base.len());
    for word in base.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        for c in word.chars() {
            out.extend(c.to_uppercase());
        }
    }
    out
}

/// Emit the **XSD-canonical** lexical form of `value` for the target XSD type,
/// written through `out` (which is cleared first). The ADR-0015 chokepoint.
pub fn canonical_lexical(value: &str, code: XsdTypeCode, out: &mut String) -> Result<()> {
    out.clear();
    match code {
        XsdTypeCode::String => out.push_str(value),
        XsdTypeCode::Boolean => cast_display::<Boolean>(value, "xsd:boolean", out)?,
        XsdTypeCode::Integer => cast_display::<Integer>(value, "xsd:integer", out)?,
        XsdTypeCode::Decimal => cast_display::<Decimal>(value, "xsd:decimal", out)?,
        XsdTypeCode::Date => cast_display::<Date>(value, "xsd:date", out)?,
        XsdTypeCode::Time => cast_display::<Time>(value, "xsd:time", out)?,
        XsdTypeCode::DateTime => {
            // R2RML §10: a TIMESTAMP's space separator becomes 'T'.
            let normalized = normalize_timestamp(value);
            cast_display::<DateTime>(normalized.as_ref(), "xsd:dateTime", out)?;
        }
        XsdTypeCode::Double => write_canonical_double(value, out)?,
        XsdTypeCode::HexBinary => hex_binary_upper(value.as_bytes(), out),
    }
    Ok(())
}

/// Parse `value` via `oxsdatatypes` and append its canonical `Display`.
fn cast_display<T>(value: &str, datatype: &str, out: &mut String) -> Result<()>
where
    T: std::str::FromStr + std::fmt::Display,
    T::Err: std::fmt::Display,
{
    let parsed: T = value
        .parse()
        .map_err(|e| Error::Datatype(format!("{value:?} is not a valid {datatype}: {e}")))?;
    write!(out, "{parsed}").map_err(|e| Error::Datatype(e.to_string()))
}

/// Append the canonical `xsd:double` lexical form (mantissa with ≥1 fractional
/// digit, uppercase `E`, exponent without leading zeros) — see the module note
/// on why `oxsdatatypes::Double`'s `Display` cannot be used directly.
fn write_canonical_double(value: &str, out: &mut String) -> Result<()> {
    let parsed: Double = value
        .parse()
        .map_err(|e| Error::Datatype(format!("{value:?} is not a valid xsd:double: {e}")))?;
    let v = f64::from(parsed);
    if v.is_nan() {
        out.push_str("NaN");
        return Ok(());
    }
    if v.is_infinite() {
        out.push_str(if v < 0.0 { "-INF" } else { "INF" });
        return Ok(());
    }
    let start = out.len();
    write!(out, "{v:E}").map_err(|e| Error::Datatype(e.to_string()))?;
    // Ensure a fractional digit: "1E0" → "1.0E0", "-1E2" → "-1.0E2".
    if let Some(rel) = out[start..].find('E') {
        let e_idx = start + rel;
        if !out[start..e_idx].contains('.') {
            out.insert_str(e_idx, ".0");
        }
    }
    Ok(())
}

/// Replace the first space in a TIMESTAMP text with `T` (R2RML §10). Borrows
/// when already `T`-separated.
fn normalize_timestamp(value: &str) -> Cow<'_, str> {
    if value.contains(' ') {
        Cow::Owned(value.replacen(' ', "T", 1))
    } else {
        Cow::Borrowed(value)
    }
}

/// Append `bytes` as uppercase hex (`xsd:hexBinary`). `oxsdatatypes` does not
/// cover `hexBinary`, so this small encoder handles it (ADR-0015). Does not
/// clear `out` — the binary value's source is bytes, not text, so callers
/// (e.g. `sf-sql` with a raw `bytea`) drive it directly.
pub fn hex_binary_upper(bytes: &[u8], out: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.reserve(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_xsd_covers_section_10_table() {
        // Character types → plain (xsd:string) literal.
        for t in [
            "CHARACTER",
            "CHARACTER VARYING",
            "CLOB",
            "NCHAR",
            "NCHAR VARYING",
            "NCLOB",
        ] {
            assert_eq!(natural_xsd(t), Some(XsdTypeCode::String), "{t}");
        }
        // Binary types → xsd:hexBinary.
        for t in ["BINARY", "BINARY VARYING", "BINARY LARGE OBJECT"] {
            assert_eq!(natural_xsd(t), Some(XsdTypeCode::HexBinary), "{t}");
        }
        assert_eq!(natural_xsd("NUMERIC"), Some(XsdTypeCode::Decimal));
        assert_eq!(natural_xsd("DECIMAL"), Some(XsdTypeCode::Decimal));
        for t in ["SMALLINT", "INTEGER", "BIGINT"] {
            assert_eq!(natural_xsd(t), Some(XsdTypeCode::Integer), "{t}");
        }
        for t in ["FLOAT", "REAL", "DOUBLE PRECISION"] {
            assert_eq!(natural_xsd(t), Some(XsdTypeCode::Double), "{t}");
        }
        assert_eq!(natural_xsd("BOOLEAN"), Some(XsdTypeCode::Boolean));
        assert_eq!(natural_xsd("DATE"), Some(XsdTypeCode::Date));
        assert_eq!(natural_xsd("TIME"), Some(XsdTypeCode::Time));
        assert_eq!(natural_xsd("TIMESTAMP"), Some(XsdTypeCode::DateTime));
        // INTERVAL is undefined in §10; unknown types are unmapped.
        assert_eq!(natural_xsd("INTERVAL"), None);
        assert_eq!(natural_xsd("JSONB"), None);
    }

    #[test]
    fn natural_xsd_is_case_and_size_insensitive() {
        assert_eq!(natural_xsd("varchar(255)"), Some(XsdTypeCode::String));
        assert_eq!(natural_xsd("  Numeric(10, 2) "), Some(XsdTypeCode::Decimal));
        assert_eq!(natural_xsd("double precision"), Some(XsdTypeCode::Double));
        assert_eq!(natural_xsd("Int"), Some(XsdTypeCode::Integer));
    }

    #[test]
    fn type_code_maps_to_xsd_iri() {
        assert_eq!(XsdTypeCode::Integer.iri(), xsd::INTEGER);
        assert_eq!(XsdTypeCode::Double.iri(), xsd::DOUBLE);
        assert_eq!(XsdTypeCode::DateTime.iri(), xsd::DATE_TIME);
        assert_eq!(XsdTypeCode::HexBinary.iri(), xsd::HEX_BINARY);
    }

    fn canon(value: &str, code: XsdTypeCode) -> String {
        let mut out = String::new();
        canonical_lexical(value, code, &mut out).unwrap();
        out
    }

    #[test]
    fn canonical_string_is_verbatim() {
        assert_eq!(
            canon("any \"raw\" text", XsdTypeCode::String),
            "any \"raw\" text"
        );
    }

    #[test]
    fn canonical_boolean_is_lowercase_true_false() {
        assert_eq!(canon("1", XsdTypeCode::Boolean), "true");
        assert_eq!(canon("true", XsdTypeCode::Boolean), "true");
        assert_eq!(canon("0", XsdTypeCode::Boolean), "false");
        assert_eq!(canon("false", XsdTypeCode::Boolean), "false");
    }

    #[test]
    fn canonical_integer_strips_zeros_and_sign() {
        assert_eq!(canon("007", XsdTypeCode::Integer), "7");
        assert_eq!(canon("+42", XsdTypeCode::Integer), "42");
        assert_eq!(canon("-0", XsdTypeCode::Integer), "0");
    }

    #[test]
    fn canonical_decimal_trims_trailing_zeros() {
        assert_eq!(canon("1.50", XsdTypeCode::Decimal), "1.5");
        assert_eq!(canon("1.0", XsdTypeCode::Decimal), "1");
        assert_eq!(canon("+100000.00", XsdTypeCode::Decimal), "100000");
        assert_eq!(canon(".12200", XsdTypeCode::Decimal), "0.122");
        assert_eq!(canon("-1.23", XsdTypeCode::Decimal), "-1.23");
    }

    #[test]
    fn canonical_double_uses_e_notation() {
        assert_eq!(canon("1.0", XsdTypeCode::Double), "1.0E0");
        assert_eq!(canon("0", XsdTypeCode::Double), "0.0E0");
        assert_eq!(canon("3.14", XsdTypeCode::Double), "3.14E0");
        assert_eq!(canon("100", XsdTypeCode::Double), "1.0E2");
        assert_eq!(canon("-2.5", XsdTypeCode::Double), "-2.5E0");
        assert_eq!(canon("0.001", XsdTypeCode::Double), "1.0E-3");
        assert_eq!(canon("1E10", XsdTypeCode::Double), "1.0E10");
    }

    #[test]
    fn canonical_double_special_values() {
        assert_eq!(canon("INF", XsdTypeCode::Double), "INF");
        assert_eq!(canon("-INF", XsdTypeCode::Double), "-INF");
        assert_eq!(canon("NaN", XsdTypeCode::Double), "NaN");
    }

    #[test]
    fn canonical_datetime_replaces_space_with_t() {
        assert_eq!(
            canon("2020-01-02 03:04:05", XsdTypeCode::DateTime),
            "2020-01-02T03:04:05"
        );
        // Already T-separated, with timezone, passes through canonically.
        assert_eq!(
            canon("2020-01-02T03:04:05Z", XsdTypeCode::DateTime),
            "2020-01-02T03:04:05Z"
        );
    }

    #[test]
    fn canonical_time_and_date() {
        assert_eq!(canon("00:00:00+00:00", XsdTypeCode::Time), "00:00:00Z");
        assert_eq!(canon("0001-01-01", XsdTypeCode::Date), "0001-01-01");
    }

    #[test]
    fn hex_binary_is_uppercase() {
        let mut out = String::new();
        hex_binary_upper(&[0x00, 0x0f, 0xab, 0xFF], &mut out);
        assert_eq!(out, "000FABFF");
        // The text chokepoint hex-encodes the value's UTF-8 bytes.
        assert_eq!(canon("AB", XsdTypeCode::HexBinary), "4142");
    }

    #[test]
    fn invalid_values_are_datatype_errors() {
        let mut out = String::new();
        assert!(canonical_lexical("not-a-number", XsdTypeCode::Integer, &mut out).is_err());
        assert!(canonical_lexical("not-a-double", XsdTypeCode::Double, &mut out).is_err());
        assert!(canonical_lexical("not-a-date", XsdTypeCode::Date, &mut out).is_err());
    }
}
