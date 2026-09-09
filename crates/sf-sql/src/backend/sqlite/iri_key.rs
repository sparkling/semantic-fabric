//! Query-owned IRI resolution uses exactly the row reconstruction semantics.
use crate::error::{Error, Result};
use rusqlite::{functions::Context, types::ValueRef};
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};

pub(super) fn evaluate(
    args: &Context<'_>,
    control: Option<&dyn QueryControl>,
) -> Result<Option<String>> {
    let bytes = |value| match value {
        ValueRef::Text(bytes) => Ok(bytes),
        _ => Err(Error::Marshal("IRI key requires decoded text".into())),
    };
    if matches!(args.get_raw(0), ValueRef::Null) {
        return Ok(None);
    }
    let value = bytes(args.get_raw(0))?;
    let base = match args.get_raw(1) {
        ValueRef::Null => None,
        raw => Some(bytes(raw)?),
    };
    if let Some(control) = control {
        let work = value
            .len()
            .checked_add(base.map_or(0, <[u8]>::len))
            .and_then(|n| n.checked_add(128))
            .and_then(|n| u64::try_from(n).ok())
            .ok_or(Error::QueryControl(QueryControlError::SourceWorkExceeded))?;
        control.consume(QueryCharge::SourceWork, work)?;
    }
    // Byte lengths are charged before UTF-8 validation scans either operand.
    let text = |bytes| {
        std::str::from_utf8(bytes).map_err(|_| Error::Marshal("invalid UTF-8 in IRI key".into()))
    };
    let value = text(value)?;
    let base = base.map(text).transpose()?;
    let mut scratch = String::new();
    let sf_core::term::GenTerm::NamedNode(iri) =
        sf_core::term::column_iri(value, base, &mut scratch)
            .map_err(|error| Error::Marshal(error.to_string()))?
    else {
        unreachable!("column IRI resolver emits only named nodes")
    };
    let result = iri.as_str().to_owned();
    if let Some(control) = control {
        control.checkpoint()?;
    }
    Ok(Some(result))
}
