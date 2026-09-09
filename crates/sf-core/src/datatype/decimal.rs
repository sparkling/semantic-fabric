//! Full-range decimal lexical normalization, independent of arithmetic storage.

/// XSD 1.1 decimal: integral values have no decimal point or fractional part.
/// Validate the entire ASCII lexical space before emitting anything. All work
/// is linear in input length, with no intermediate allocation or numeric cast;
/// output needs at most input length + 1 bytes (the zero before `.fraction`).
pub(super) fn write_canonical(value: &str, out: &mut String) -> crate::Result<()> {
    let unsigned = value
        .strip_prefix('+')
        .or_else(|| value.strip_prefix('-'))
        .unwrap_or(value);
    let (integral, fractional) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if (integral.is_empty() && fractional.is_empty())
        || !integral.bytes().all(|b| b.is_ascii_digit())
        || !fractional.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(crate::Error::Datatype(
            "invalid xsd:decimal lexical form".into(),
        ));
    }
    let integral = integral.trim_start_matches('0');
    let fractional = fractional.trim_end_matches('0');
    if integral.is_empty() && fractional.is_empty() {
        out.push('0');
        return Ok(());
    }
    if value.starts_with('-') {
        out.push('-');
    }
    out.push_str(if integral.is_empty() { "0" } else { integral });
    if !fractional.is_empty() {
        out.push('.');
        out.push_str(fractional);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::datatype::{canonical_lexical, XsdTypeCode};

    fn canonical(value: &str) -> String {
        let mut out = String::from("reused scratch");
        canonical_lexical(value, XsdTypeCode::Decimal, &mut out).unwrap();
        out
    }

    #[test]
    fn natural_decimal_preserves_full_native_range() {
        let integral = "1".repeat(131_072);
        let fraction = format!("0.{}1", "0".repeat(16_382));
        for (raw, expected) in [
            (format!("+000{integral}.00"), integral.clone()),
            (format!("-{integral}.00"), format!("-{integral}")),
            (fraction.clone(), fraction),
            (
                "170141183460469231731.687303715884105728".into(),
                "170141183460469231731.687303715884105728".into(),
            ),
            (
                "0.0000000000000000001".into(),
                "0.0000000000000000001".into(),
            ),
        ] {
            let result = canonical(&raw);
            assert_eq!(result, expected);
            assert_eq!(canonical(&result), result);
            assert!(result.len() <= raw.len() + 1);
        }
    }

    #[test]
    fn decimal_normalization_matches_existing_representable_values() {
        for sign in ["", "+", "-"] {
            for integer in ["", "0", "000", "1", "001", "1234567890"] {
                for fraction in ["", ".", ".0", ".0000", ".00100", ".123456789012345678"] {
                    let raw = format!("{sign}{integer}{fraction}");
                    if let Ok(old) = raw.parse::<oxsdatatypes::Decimal>() {
                        let result = canonical(&raw);
                        assert_eq!(result, old.to_string(), "{raw}");
                        assert_eq!(canonical(&result), result);
                    }
                }
            }
        }
        assert_eq!(canonical("-.1200"), "-0.12");
        assert_eq!(canonical("-000.000"), "0");
        assert_eq!(canonical("+0001."), "1");
    }

    #[test]
    fn decimal_normalization_rejects_invalid_grammar_before_emission() {
        for raw in [
            "",
            "+",
            "-",
            ".",
            "+.",
            "-.",
            " 1",
            "1 ",
            "1\n",
            "1e0",
            "NaN",
            "INF",
            "-Infinity",
            "１",
            "١",
            "1.2.3",
            "--1",
            "+-1",
            "1\0",
            "00x.000",
        ] {
            let mut out = String::from("previous value");
            assert!(
                canonical_lexical(raw, XsdTypeCode::Decimal, &mut out).is_err(),
                "{raw:?}"
            );
            assert!(out.is_empty(), "no partial output for {raw:?}");
        }
    }
}
