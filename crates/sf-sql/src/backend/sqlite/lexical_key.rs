//! Decoder-equivalent keys with separately requested natural canonicalization.
#[cfg(test)]
mod tests;
use crate::{
    backend::SqliteDecode,
    error::{Error, Result},
};
use rusqlite::{functions::Context, types::ValueRef};
use sf_core::{
    datatype::XsdTypeCode,
    query_control::{QueryCharge, QueryControl},
};

const CODES: [XsdTypeCode; 9] = [
    XsdTypeCode::String,
    XsdTypeCode::HexBinary,
    XsdTypeCode::Decimal,
    XsdTypeCode::Integer,
    XsdTypeCode::Double,
    XsdTypeCode::Boolean,
    XsdTypeCode::Date,
    XsdTypeCode::Time,
    XsdTypeCode::DateTime,
];

impl SqliteDecode {
    /// Internal compiler/callback protocol; zero is storage-class fallback.
    pub fn declared_key_code(self) -> usize {
        self.declared
            .and_then(|code| CODES.iter().position(|c| *c == code))
            .map_or(0, |i| i + 1)
    }
}

pub(super) fn evaluate(
    args: &Context<'_>,
    control: Option<&dyn QueryControl>,
) -> Result<Option<String>> {
    let natural = matches!(args.get_raw(1), ValueRef::Integer(16..=25));
    let raw_code = match args.get_raw(1) {
        ValueRef::Integer(n) if natural => ValueRef::Integer(n - 16),
        other => other,
    };
    let code = match raw_code {
        ValueRef::Integer(0) => None,
        ValueRef::Integer(n) if (1..=9).contains(&n) => Some(CODES[n as usize - 1]),
        _ => return Err(Error::Marshal("invalid lexical decoder code".into())),
    };
    let padding = match args.get_raw(2) {
        ValueRef::Integer(-1) => None,
        ValueRef::Integer(n) if n >= 0 => Some(
            usize::try_from(n)
                .map_err(|_| Error::Marshal("invalid lexical decoder width".into()))?,
        ),
        _ => return Err(Error::Marshal("invalid lexical decoder width".into())),
    };
    let decode = SqliteDecode {
        declared: code,
        padding,
    };
    let result = lexical(args.get_raw(0), decode, control)?;
    if natural {
        let Some(value) = result else {
            return Ok(None);
        };
        if let Some(control) = control {
            control.consume(
                QueryCharge::SourceWork,
                (value.len() as u64).checked_add(128).ok_or_else(exceeded)?,
            )?;
        }
        let effective = code
            .or_else(|| super::decode::storage_class_code(&args.get_raw(0)))
            .unwrap_or(XsdTypeCode::String);
        let mut canonical = String::new();
        sf_core::datatype::natural_lexical(&value, effective, &mut canonical)
            .map_err(|e| Error::Marshal(e.to_string()))?;
        if let Some(control) = control {
            control.checkpoint()?;
        }
        Ok(Some(canonical))
    } else {
        Ok(result)
    }
}

pub(super) fn lexical(
    value: ValueRef<'_>,
    decode: SqliteDecode,
    control: Option<&dyn QueryControl>,
) -> Result<Option<String>> {
    if let Some(width) = decode.padding {
        return super::text_key::character(value, width, control);
    }
    if let Some(control) = control {
        let input = match value {
            ValueRef::Text(bytes) => bytes.len(),
            ValueRef::Blob(bytes) => bytes.len().checked_mul(2).ok_or_else(exceeded)?,
            // f64::to_string()'s exact worst case (signed subnormal near zero,
            // e.g. -5e-324), not the 32-byte catch-all below, which undercharges
            // by ~10x.
            ValueRef::Real(_) => 327,
            _ => 32,
        };
        control.consume(
            QueryCharge::SourceWork,
            u64::try_from(input).map_err(|_| exceeded())?,
        )?;
    }
    let code = decode
        .declared
        .or_else(|| super::decode::storage_class_code(&value));
    // Already charged above (this function's own bound), so pass an
    // uncontrolled SourceWork here to avoid double-charging.
    let result =
        super::decode::lexical_typed(value, code, crate::source_work::SourceWork::new(None))?;
    if let Some(control) = control {
        control.checkpoint()?;
    }
    Ok(result)
}

fn exceeded() -> Error {
    Error::QueryControl(sf_core::query_control::QueryControlError::SourceWorkExceeded)
}
