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
    query_bounded_mapped(
        client,
        sql,
        params,
        maximum,
        Error::Postgres,
        || {
            Error::Introspection(format!(
                "PostgreSQL introspection {resource} exceeds the configured limit"
            ))
        },
        Ok,
    )
    .await
}

pub(super) async fn query_bounded_mapped<C, T, E, F, DriverError, LimitError>(
    client: &C,
    sql: &str,
    params: &[TypedQueryParameter<'_>],
    maximum: usize,
    driver_error: DriverError,
    limit_error: LimitError,
    map: F,
) -> std::result::Result<Vec<T>, E>
where
    C: GenericClient + Sync,
    F: FnMut(Row) -> std::result::Result<T, E>,
    DriverError: Fn(tokio_postgres::Error) -> E + Copy,
    LimitError: Fn() -> E,
{
    let rows = client
        .query_typed_raw(
            sql,
            params
                .iter()
                .map(|parameter| (parameter.value, parameter.parameter_type.clone())),
        )
        .await
        .map_err(driver_error)?;
    collect_bounded_mapped_rows(rows, maximum, driver_error, limit_error, map).await
}

async fn collect_bounded_mapped_rows<T, U, S, E, F, DriverError, LimitError>(
    rows: S,
    maximum: usize,
    driver_error: DriverError,
    limit_error: LimitError,
    mut map: F,
) -> std::result::Result<Vec<T>, E>
where
    S: TryStream<Ok = U, Error = tokio_postgres::Error>,
    F: FnMut(U) -> std::result::Result<T, E>,
    DriverError: Fn(tokio_postgres::Error) -> E,
    LimitError: Fn() -> E,
{
    futures_util::pin_mut!(rows);
    let mut retained = Vec::new();
    while let Some(row) = std::future::poll_fn(|context| rows.as_mut().try_poll_next(context)).await
    {
        let row = row.map_err(&driver_error)?;
        if retained.len() == maximum {
            return Err(limit_error());
        }
        retained.push(map(row)?);
    }
    Ok(retained)
}

#[cfg(test)]
mod tests;
