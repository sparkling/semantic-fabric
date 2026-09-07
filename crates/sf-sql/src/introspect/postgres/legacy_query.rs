use futures_util::TryStream;
use tokio_postgres::types::{ToSql, Type};
use tokio_postgres::{GenericClient, Row};

use crate::error::{Error, Result};

use super::evidence::{
    NoopPostgresObservationObserverV1, PostgresObservationObserverV1,
    PostgresObservationStreamTerminalV1, PostgresObservationStreamV1,
};

pub(super) use super::legacy_bounds::{
    MAX_LEGACY_RELATIONS_PG16_V1, MAX_LEGACY_ROWS_PER_SET_PG16_V1,
};

pub(super) const LEGACY_RELATION_QUERY_LIMIT_PG16_V1: i64 = MAX_LEGACY_RELATIONS_PG16_V1 as i64 + 1;
pub(super) const LEGACY_SET_QUERY_LIMIT_PG16_V1: i64 = MAX_LEGACY_ROWS_PER_SET_PG16_V1 as i64 + 1;

pub(super) struct TypedQueryParameter<'a> {
    value: &'a (dyn ToSql + Sync),
    parameter_type: Type,
}

pub(super) struct BoundedQueryObservationV1<'a, O> {
    maximum: usize,
    stream: PostgresObservationStreamV1,
    observer: &'a mut O,
}

impl<'a, O> BoundedQueryObservationV1<'a, O> {
    pub(super) fn new(
        maximum: usize,
        stream: PostgresObservationStreamV1,
        observer: &'a mut O,
    ) -> Self {
        Self {
            maximum,
            stream,
            observer,
        }
    }
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
    let mut observer = NoopPostgresObservationObserverV1;
    query_bounded_observed(
        client,
        sql,
        params,
        maximum,
        resource,
        PostgresObservationStreamV1::LegacyTables,
        &mut observer,
    )
    .await
}

pub(super) async fn query_bounded_observed<C, O>(
    client: &C,
    sql: &str,
    params: &[TypedQueryParameter<'_>],
    maximum: usize,
    resource: &'static str,
    stream: PostgresObservationStreamV1,
    observer: &mut O,
) -> Result<Vec<Row>>
where
    C: GenericClient + Sync,
    O: PostgresObservationObserverV1,
{
    query_bounded_mapped_observed(
        client,
        sql,
        params,
        Error::Postgres,
        || {
            Error::Introspection(format!(
                "PostgreSQL introspection {resource} exceeds the configured limit"
            ))
        },
        Ok,
        BoundedQueryObservationV1::new(maximum, stream, observer),
    )
    .await
}

pub(super) async fn query_bounded_mapped_observed<C, T, E, F, DriverError, LimitError, O>(
    client: &C,
    sql: &str,
    params: &[TypedQueryParameter<'_>],
    driver_error: DriverError,
    limit_error: LimitError,
    map: F,
    observation: BoundedQueryObservationV1<'_, O>,
) -> std::result::Result<Vec<T>, E>
where
    C: GenericClient + Sync,
    F: FnMut(Row) -> std::result::Result<T, E>,
    DriverError: Fn(tokio_postgres::Error) -> E + Copy,
    LimitError: Fn() -> E,
    O: PostgresObservationObserverV1,
{
    let BoundedQueryObservationV1 {
        maximum,
        stream,
        observer,
    } = observation;
    observer.stream_started(stream, maximum);
    let rows = match client
        .query_typed_raw(
            sql,
            params
                .iter()
                .map(|parameter| (parameter.value, parameter.parameter_type.clone())),
        )
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            observer.stream_terminal(stream, PostgresObservationStreamTerminalV1::QueryFailure);
            return Err(driver_error(error));
        }
    };
    collect_bounded_mapped_rows_observed(
        rows,
        maximum,
        driver_error,
        limit_error,
        map,
        stream,
        observer,
    )
    .await
}

#[cfg(test)]
async fn collect_bounded_mapped_rows<T, U, S, E, F, DriverError, LimitError>(
    rows: S,
    maximum: usize,
    driver_error: DriverError,
    limit_error: LimitError,
    map: F,
) -> std::result::Result<Vec<T>, E>
where
    S: TryStream<Ok = U, Error = tokio_postgres::Error>,
    F: FnMut(U) -> std::result::Result<T, E>,
    DriverError: Fn(tokio_postgres::Error) -> E,
    LimitError: Fn() -> E,
{
    let mut observer = NoopPostgresObservationObserverV1;
    collect_bounded_mapped_rows_observed(
        rows,
        maximum,
        driver_error,
        limit_error,
        map,
        PostgresObservationStreamV1::LegacyTables,
        &mut observer,
    )
    .await
}

pub(super) async fn collect_bounded_mapped_rows_observed<
    T,
    U,
    S,
    E,
    F,
    DriverError,
    LimitError,
    O,
>(
    rows: S,
    maximum: usize,
    driver_error: DriverError,
    limit_error: LimitError,
    mut map: F,
    stream: PostgresObservationStreamV1,
    observer: &mut O,
) -> std::result::Result<Vec<T>, E>
where
    S: TryStream<Ok = U, Error = tokio_postgres::Error>,
    F: FnMut(U) -> std::result::Result<T, E>,
    DriverError: Fn(tokio_postgres::Error) -> E,
    LimitError: Fn() -> E,
    O: PostgresObservationObserverV1,
{
    futures_util::pin_mut!(rows);
    let mut retained = Vec::new();
    while let Some(row) = std::future::poll_fn(|context| rows.as_mut().try_poll_next(context)).await
    {
        let row = match row {
            Ok(row) => row,
            Err(error) => {
                observer.stream_terminal(stream, PostgresObservationStreamTerminalV1::QueryFailure);
                return Err(driver_error(error));
            }
        };
        observer.row_polled(stream);
        if retained.len() == maximum {
            observer.stream_terminal(stream, PostgresObservationStreamTerminalV1::Overflow);
            return Err(limit_error());
        }
        let decoded = match map(row) {
            Ok(decoded) => decoded,
            Err(error) => {
                observer.stream_terminal(stream, PostgresObservationStreamTerminalV1::RowFailure);
                return Err(error);
            }
        };
        retained.push(decoded);
        observer.row_decoded(stream, retained.len());
    }
    observer.stream_terminal(stream, PostgresObservationStreamTerminalV1::Complete);
    Ok(retained)
}

#[cfg(test)]
mod tests;
