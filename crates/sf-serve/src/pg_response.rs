//! PostgreSQL response execution for observational and verified generations.

use std::sync::Arc;

use axum::body::Body;
use axum::response::Response;
use sf_sparql::{exec_pg, Plan};

use crate::backend::PgQueryClient;
use crate::budget::RequestBudget;
use crate::pg_generation::{PgGenerationError, VerifiedPostgresGenerationLease};
use crate::problem::{self, ProblemCode};
use crate::stream::{self, GraphFormat};

pub(crate) async fn select(
    pool: crate::PostgresPool,
    plan: Arc<Plan>,
    generation: Option<VerifiedPostgresGenerationLease>,
    rls_tables: Option<Arc<[String]>>,
    format: stream::SelectFormat,
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
    } else if budget.postgres_rls().is_some() {
        let lease = crate::pg_rls::PgRlsLease::acquire(&pool, rls_tables, &budget).await?;
        stream::select_body_streaming_controlled(
            move |sink| {
                Box::pin(async move {
                    let result = exec_pg::select_each_pg_controlled(
                        &plan,
                        lease.client(),
                        &drive_budget,
                        sink,
                    )
                    .await;
                    lease.finish().await?;
                    result
                })
            },
            format,
            variables,
            budget,
        )
    } else {
        let conn = Arc::new(crate::source_acquisition::acquire_pg(&pool, budget.clone()).await?);
        stream::select_body_streaming_controlled(
            move |sink| {
                Box::pin(async move {
                    let result = exec_pg::select_each_pg_controlled(
                        &plan,
                        PgQueryClient(conn.clone()),
                        &drive_budget,
                        sink,
                    )
                    .await;
                    conn.finish_result(result, &drive_budget).await
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
    pool: crate::PostgresPool,
    plan: Arc<Plan>,
    generation: Option<VerifiedPostgresGenerationLease>,
    rls_tables: Option<Arc<[String]>>,
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
    if budget.postgres_rls().is_some() {
        let lease = crate::pg_rls::PgRlsLease::acquire(&pool, rls_tables, &budget).await?;
        let result = budget
            .run(exec_pg::ask_pg_controlled(&plan, lease.client(), &budget))
            .await;
        lease
            .finish()
            .await
            .map_err(|_| problem::response_with_retry_after(ProblemCode::SourceUnavailable))?;
        return result.map_err(problem::response_for_control);
    }
    let conn = Arc::new(crate::source_acquisition::acquire_pg(&pool, budget.clone()).await?);
    budget
        .run(async {
            let result =
                exec_pg::ask_pg_controlled(&plan, PgQueryClient(conn.clone()), &budget).await;
            conn.finish_result(result, &budget).await
        })
        .await
        .map_err(problem::response_for_control)
}

pub(crate) async fn construct(
    pool: crate::PostgresPool,
    plan: Arc<Plan>,
    generation: Option<VerifiedPostgresGenerationLease>,
    rls_tables: Option<Arc<[String]>>,
    format: GraphFormat,
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
    } else if budget.postgres_rls().is_some() {
        let lease = crate::pg_rls::PgRlsLease::acquire(&pool, rls_tables, &budget).await?;
        stream::construct_body_streaming_controlled(
            move |sink| {
                Box::pin(async move {
                    let result = exec_pg::construct_each_pg_controlled(
                        &plan,
                        lease.client(),
                        &drive_budget,
                        sink,
                    )
                    .await;
                    lease.finish().await?;
                    result
                })
            },
            format,
            budget,
        )
    } else {
        let conn = Arc::new(crate::source_acquisition::acquire_pg(&pool, budget.clone()).await?);
        stream::construct_body_streaming_controlled(
            move |sink| {
                Box::pin(async move {
                    let result = exec_pg::construct_each_pg_controlled(
                        &plan,
                        PgQueryClient(conn.clone()),
                        &drive_budget,
                        sink,
                    )
                    .await;
                    conn.finish_result(result, &drive_budget).await
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
