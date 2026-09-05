//! Deadline-bounded connection acquisition for observational source adapters.

use axum::response::Response;
use deadpool_postgres::PoolError;

use crate::backend::PgConn;
use crate::budget::RequestBudget;
use crate::problem::{self, ProblemCode};

pub(crate) async fn acquire_mysql(
    pool: &mysql_async::Pool,
    budget: &RequestBudget,
) -> Result<mysql_async::Conn, Response> {
    match budget.run(pool.get_conn()).await {
        Err(error) => Err(problem::response_for_control(error)),
        Ok(Err(_)) => Err(problem::response(ProblemCode::SourceUnavailable)),
        Ok(Ok(conn)) => Ok(conn),
    }
}

/// Pool exhaustion is shed as an honest `503` with a fixed retry hint instead
/// of queuing past the request's absolute deadline.
pub(crate) async fn acquire_pg(
    pool: &deadpool_postgres::Pool,
    budget: RequestBudget,
) -> Result<PgConn, Response> {
    let acquired = match budget.run(pool.get()).await {
        Ok(result) => result,
        Err(error) => return Err(problem::response_for_control(error)),
    };
    let conn = acquired.map_err(|error| match error {
        PoolError::Timeout(_) => problem::response_with_retry_after(ProblemCode::SourceUnavailable),
        _ => problem::response(ProblemCode::Internal),
    })?;
    match budget.run(PgConn::checked(conn)).await {
        Err(error) => Err(problem::response_for_control(error)),
        Ok(Err(_)) => Err(problem::response(ProblemCode::Internal)),
        Ok(Ok(conn)) => Ok(conn),
    }
}
