use tokio_postgres::types::FromSqlOwned;
use tokio_postgres::Row;

use crate::error::{Error, Result};

pub(super) const MAX_LEGACY_TEXT_BYTES_PG16_V1: usize = 256;
pub(super) const LEGACY_TEXT_QUERY_LIMIT_PG16_V1: i32 = MAX_LEGACY_TEXT_BYTES_PG16_V1 as i32;

pub(super) struct LegacyRow<'a> {
    row: &'a Row,
}

impl<'a> LegacyRow<'a> {
    pub(super) fn try_new(row: &'a Row) -> Result<Self> {
        let rejected = row
            .try_get::<_, bool>("sf_text_rejected")
            .map_err(|_| decode_error("text guard"))?;
        validate_text_guard(rejected)?;
        Ok(Self { row })
    }

    pub(super) fn text(&self, column: &'static str, resource: &'static str) -> Result<String> {
        let value = self
            .row
            .try_get::<_, Option<String>>(column)
            .map_err(|_| decode_error(resource))?;
        validate_text(value, resource)
    }

    pub(super) fn scalar<T>(&self, column: &'static str, resource: &'static str) -> Result<T>
    where
        T: FromSqlOwned,
    {
        self.row.try_get(column).map_err(|_| decode_error(resource))
    }
}

fn validate_text_guard(rejected: bool) -> Result<()> {
    if rejected {
        return Err(Error::Introspection(
            "PostgreSQL introspection catalogue text is unavailable or exceeds the configured limit"
                .into(),
        ));
    }
    Ok(())
}

fn validate_text(value: Option<String>, resource: &'static str) -> Result<String> {
    match value {
        Some(value) if value.len() <= MAX_LEGACY_TEXT_BYTES_PG16_V1 => Ok(value),
        Some(_) => Err(Error::Introspection(
            "PostgreSQL introspection catalogue text exceeds the configured limit".into(),
        )),
        None => Err(decode_error(resource)),
    }
}

fn decode_error(resource: &'static str) -> Error {
    Error::Introspection(format!(
        "PostgreSQL introspection could not decode catalogue {resource}"
    ))
}

#[cfg(test)]
mod tests;
