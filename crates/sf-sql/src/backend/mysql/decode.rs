//! Prospective application-side MySQL lexical decoding; native packets excluded.
use super::*;
use crate::source_work::SourceWork;

pub(super) fn row(
    columns: usize,
    codes: &[Option<XsdTypeCode>],
    mut take: impl FnMut(usize) -> Option<Value>,
    work: SourceWork<'_>,
) -> Result<RawTuple> {
    work.checkpoint()?;
    ensure_row_arity(columns, codes.len())?;
    let mut values = work.vector(columns)?;
    let mut owned_codes = work.vector(codes.len())?;
    work.product(codes.len(), std::mem::size_of::<Option<XsdTypeCode>>())?;
    owned_codes.extend_from_slice(codes);
    for (i, code) in codes.iter().copied().enumerate() {
        work.charge(1)?;
        let native = take(i)
            .ok_or_else(|| Error::Marshal("MySQL result row column is unavailable".into()))?;
        values.push(value(native, code, work)?);
    }
    work.checkpoint()?;
    Ok(RawTuple {
        values,
        codes: owned_codes,
    })
}

pub(super) fn value(
    v: Value,
    code: Option<XsdTypeCode>,
    work: SourceWork<'_>,
) -> Result<Option<String>> {
    use mysql_async::Value::*;
    work.charge(1)?;
    match &v {
        NULL => {}
        Bytes(bytes) if code == Some(XsdTypeCode::HexBinary) => work.product(bytes.len(), 3)?,
        Bytes(bytes) => work.charge(bytes.len())?,
        // f64 Display has at most 327 characters even for subnormal values;
        // other scalar/temporal variants fit below this bounded allowance.
        _ => work.charge(512)?,
    }
    let result = match v {
        NULL => None,
        Bytes(bytes) if code == Some(XsdTypeCode::HexBinary) => {
            let mut encoded = String::new();
            encoded
                .try_reserve_exact(
                    bytes
                        .len()
                        .checked_mul(2)
                        .ok_or_else(|| Error::Marshal("MySQL binary length overflow".into()))?,
                )
                .map_err(|_| Error::Marshal("MySQL decoding allocation failed".into()))?;
            datatype::hex_binary_upper(&bytes, &mut encoded);
            Some(encoded)
        }
        Bytes(bytes) => Some(
            String::from_utf8(bytes)
                .map_err(|error| Error::Marshal(format!("non-UTF8 MySQL text column: {error}")))?,
        ),
        Int(i) => Some(i.to_string()),
        UInt(u) => Some(u.to_string()),
        Float(f) => Some(f.to_string()),
        Double(d) => Some(d.to_string()),
        Date(y, mo, d, h, mi, s, us) => {
            if code == Some(XsdTypeCode::Date) {
                Some(format!("{y:04}-{mo:02}-{d:02}"))
            } else if us == 0 {
                Some(format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}"))
            } else {
                Some(format!(
                    "{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{us:06}"
                ))
            }
        }
        Time(neg, days, h, mi, s, us) => {
            let sign = if neg { "-" } else { "" };
            let total_h = u64::from(days) * 24 + u64::from(h);
            if us == 0 {
                Some(format!("{sign}{total_h:02}:{mi:02}:{s:02}"))
            } else {
                Some(format!("{sign}{total_h:02}:{mi:02}:{s:02}.{us:06}"))
            }
        }
    };
    work.checkpoint()?;
    Ok(result)
}

#[cfg(test)]
#[path = "decode_tests.rs"]
mod tests;
