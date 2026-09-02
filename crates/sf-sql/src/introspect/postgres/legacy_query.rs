use futures_util::TryStream;
use tokio_postgres::types::{ToSql, Type};
use tokio_postgres::{GenericClient, Row};

use crate::error::{Error, Result};

pub(super) use super::legacy_bounds::{
    MAX_LEGACY_RELATIONS_PG16_V1, MAX_LEGACY_ROWS_PER_SET_PG16_V1,
};

pub(super) const LEGACY_RELATION_QUERY_LIMIT_PG16_V1: i64 = MAX_LEGACY_RELATIONS_PG16_V1 as i64 + 1;
pub(super) const LEGACY_SET_QUERY_LIMIT_PG16_V1: i64 = MAX_LEGACY_ROWS_PER_SET_PG16_V1 as i64 + 1;

pub(super) struct TypedQueryParameter<'a> {
    value: &'a (dyn ToSql + Sync),
    parameter_type: Type,
}

impl<'a> TypedQueryParameter<'a> {
    pub(super) fn new<T>(value: &'a T, parameter_type: Type) -> Self
    where
        T: ToSql + Sync,
    {
        Self {
            value,
            parameter_type,
        }
    }
}

pub(super) async fn query_bounded<C>(
    client: &C,
    sql: &str,
    params: &[TypedQueryParameter<'_>],
    maximum: usize,
    resource: &'static str,
) -> Result<Vec<Row>>
where
    C: GenericClient + Sync,
{
    let rows = client
        .query_typed_raw(
            sql,
            params
                .iter()
                .map(|parameter| (parameter.value, parameter.parameter_type.clone())),
        )
        .await?;
    collect_bounded_rows(rows, maximum, resource).await
}

async fn collect_bounded_rows<T, S>(
    rows: S,
    maximum: usize,
    resource: &'static str,
) -> Result<Vec<T>>
where
    S: TryStream<Ok = T, Error = tokio_postgres::Error>,
{
    futures_util::pin_mut!(rows);
    let mut retained = Vec::new();
    while let Some(row) = std::future::poll_fn(|context| rows.as_mut().try_poll_next(context)).await
    {
        let row = row?;
        if retained.len() == maximum {
            return Err(Error::Introspection(format!(
                "PostgreSQL introspection {resource} exceeds the configured limit"
            )));
        }
        retained.push(row);
    }
    Ok(retained)
}

#[cfg(test)]
mod tests;
