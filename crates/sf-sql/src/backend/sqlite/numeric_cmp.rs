//! Query-owned numeric predicate; arguments are decoded lexical values and types.
use super::*;
use rusqlite::{functions::Context, types::ValueRef};
use sf_core::{
    numeric_compare::{compare, NumericOp},
    query_control::{QueryCharge, QueryControl},
};

pub(super) fn evaluate(
    args: &Context<'_>,
    control: Option<&dyn QueryControl>,
) -> Result<Option<String>> {
    let mut values = Vec::with_capacity(4);
    let mut bytes = 128u64; // bounded numeric promotion scratch
    for index in 0..4 {
        match args.get_raw(index) {
            ValueRef::Null => return Ok(None),
            ValueRef::Text(value) => {
                bytes = bytes
                    .checked_add(value.len() as u64)
                    .ok_or(Error::QueryControl(
                        sf_core::query_control::QueryControlError::SourceWorkExceeded,
                    ))?;
                values.push(value);
            }
            _ => return Err(Error::Marshal("invalid numeric comparison protocol".into())),
        }
    }
    if let Some(control) = control {
        control.consume(QueryCharge::SourceWork, bytes)?;
    }
    let op = match args.get_raw(4) {
        ValueRef::Integer(0) => NumericOp::Eq,
        ValueRef::Integer(1) => NumericOp::Ne,
        ValueRef::Integer(2) => NumericOp::Lt,
        ValueRef::Integer(3) => NumericOp::Le,
        ValueRef::Integer(4) => NumericOp::Gt,
        ValueRef::Integer(5) => NumericOp::Ge,
        _ => {
            return Err(Error::Marshal(
                "invalid numeric comparison operation".into(),
            ))
        }
    };
    let values = values
        .into_iter()
        .map(std::str::from_utf8)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::Marshal("numeric comparison is not UTF-8".into()))?;
    let answer = compare(values[0], values[1], values[2], values[3], op)
        .map_err(|e| Error::Marshal(e.to_string()))?;
    if let Some(control) = control {
        control.checkpoint()?;
    }
    Ok(answer.map(|yes| if yes { "1" } else { "0" }.into()))
}
