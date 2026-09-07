//! PostgreSQL response execution for observational and verified generations.

use std::sync::Arc;

use axum::body::Body;
use axum::response::Response;
use sf_sparql::{exec_pg, Plan};
use sparesults::QueryResultsFormat;

use crate::budget::RequestBudget;
use crate::pg_generation::{PgGenerationError, VerifiedPostgresGenerationLease};
use crate::problem::{self, ProblemCode};
use crate::stream::{self, RdfFormat};

pub(crate) async fn select(
    pool: deadpool_postgres::Pool,
    plan: Arc<Plan>,
    generation: Option<VerifiedPostgresGenerationLease>,
    format: QueryResultsFormat,
    variables: Vec<String>,
    budget: RequestBudget,
) -> Result<Body, Response> {
    let drive_budget = budget.clone();
    let body = if let Some(lease) = generation {
        stream::select_body_streaming_controlled(
            move |sink| {
                Box::pin(async move {
                    flatten_stream_result(lease.select_each(&plan, &drive_budget, sink).await)
                })
            },
            format,
            variables,
            budget,
        )
    } else {
        let conn = crate::source_acquisition::acquire_pg(&pool, budget.clone()).await?;
        stream::select_body_streaming_controlled(
            move |sink| {
                Box::pin(async move {
                    exec_pg::select_each_pg_controlled(&plan, conn, &drive_budget, sink).await
                })
            },
            format,
            variables,
            budget,
        )
    };
    Ok(body)
}

pub(crate) async fn ask(
    pool: deadpool_postgres::Pool,
    plan: Arc<Plan>,
    generation: Option<VerifiedPostgresGenerationLease>,
    budget: RequestBudget,
) -> Result<sf_sparql::Result<bool>, Response> {
    if let Some(lease) = generation {
        return match budget.run(lease.ask(&plan, &budget)).await {
            Err(error) => Err(problem::response_for_control(error)),
            Ok(Err(_)) => Err(problem::response_with_retry_after(
                ProblemCode::SourceUnavailable,
            )),
            Ok(Ok(result)) => Ok(result),
        };
    }
    let conn = crate::source_acquisition::acquire_pg(&pool, budget.clone()).await?;
    budget
        .run(exec_pg::ask_pg_controlled(&plan, conn, &budget))
        .await
        .map_err(problem::response_for_control)
}

pub(crate) async fn construct(
    pool: deadpool_postgres::Pool,
    plan: Arc<Plan>,
    generation: Option<VerifiedPostgresGenerationLease>,
    format: RdfFormat,
    budget: RequestBudget,
) -> Result<Body, Response> {
    let drive_budget = budget.clone();
    let body = if let Some(lease) = generation {
        stream::construct_body_streaming_controlled(
            move |sink| {
                Box::pin(async move {
                    flatten_stream_result(lease.construct_each(&plan, &drive_budget, sink).await)
                })
            },
            format,
            budget,
        )
    } else {
        let conn = crate::source_acquisition::acquire_pg(&pool, budget.clone()).await?;
        stream::construct_body_streaming_controlled(
            move |sink| {
                Box::pin(async move {
                    exec_pg::construct_each_pg_controlled(&plan, conn, &drive_budget, sink).await
                })
            },
            format,
            budget,
        )
    };
    Ok(body)
}

fn flatten_stream_result<T>(
    result: Result<sf_sparql::Result<T>, PgGenerationError>,
) -> sf_sparql::Result<T> {
    result.unwrap_or_else(|_| {
        Err(sf_sparql::Error::Sql(
            "verified generation close failed".to_owned(),
        ))
    })
}
