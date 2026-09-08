//! Decoder-equivalent lexical keys. No natural-literal canonicalization here.
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
    let code = match args.get_raw(1) {
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
    lexical(
        args.get_raw(0),
        SqliteDecode {
            declared: code,
            padding,
        },
        control,
    )
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
            _ => 32,
        };
        control.consume(
            QueryCharge::SourceWork,
            u64::try_from(input).map_err(|_| exceeded())?,
        )?;
    }
    let code = decode
        .declared
        .or_else(|| super::storage_class_code(&value));
    let result = super::lexical_typed(value, code)?;
    if let Some(control) = control {
        control.checkpoint()?;
    }
    Ok(result)
}

fn exceeded() -> Error {
    Error::QueryControl(sf_core::query_control::QueryControlError::SourceWorkExceeded)
}
