//! Native owned execution for the typed bounded multi-origin sink.
use crate::{
    budget::RequestBudget, lineage::Lineage, pg_generation::VerifiedPostgresGenerationLease,
    problem, stream, Backend,
};
use axum::response::Response;
use sf_core::query_control::QueryControl;
use sf_sparql::{exec_core::lineage_each_async_controlled, lineage::LineageSpec, Plan};
use sf_sql::backend::{pg::PgBackend, sqlite::SqliteOwnedBackend};
use std::sync::Arc;

pub(crate) async fn respond(
    backend: Backend,
    plan: Arc<Plan>,
    spec: Arc<LineageSpec>,
    generation: Option<VerifiedPostgresGenerationLease>,
    rls_tables: Option<Arc<[String]>>,
    proof: Arc<Lineage>,
    budget: RequestBudget,
) -> Response {
    let form = plan.form.clone();
    let body = match backend {
        Backend::Sqlite(pool) => {
            if generation.is_some() {
                return problem::response(problem::ProblemCode::Internal);
            }
            let lease = match crate::sqlite_admission::acquire(&pool, &budget).await {
                Ok(lease) => lease,
                Err(response) => return response,
            };
            let control: Arc<dyn QueryControl> = Arc::new(budget.clone());
            stream::multiple_lineage_body(
                move |sink| {
                    Box::pin(async move {
                        let mut backend =
                            SqliteOwnedBackend::new_controlled_leased(lease, control.clone());
                        lineage_each_async_controlled(
                            &plan,
                            &spec,
                            &mut backend,
                            control.as_ref(),
                            sink,
                        )
                        .await
                    })
                },
                proof,
                form,
                budget,
            )
        }
        Backend::Mysql(pool) => {
            if generation.is_some() {
                return problem::response(problem::ProblemCode::Internal);
            }
            let query = match crate::source_acquisition::acquire_mysql(&pool, &budget).await {
                Ok(query) => query,
                Err(response) => return response,
            };
            let control = budget.clone();
            stream::multiple_lineage_body(
                move |sink| {
                    Box::pin(async move {
                        crate::mysql_query::lineage(&plan, &spec, query, &control, sink).await
                    })
                },
                proof,
                form,
                budget,
            )
        }
        Backend::Pg(pool) => {
            let control = budget.clone();
            if let Some(lease) = generation {
                stream::multiple_lineage_body(
                    move |sink| {
                        Box::pin(async move {
                            lease
                                .lineage_each(&plan, &spec, &control, sink)
                                .await
                                .unwrap_or_else(|_| {
                                    Err(sf_sparql::Error::Sql(
                                        "verified generation close failed".into(),
                                    ))
                                })
                        })
                    },
                    proof,
                    form,
                    budget,
                )
            } else if budget.postgres_rls().is_some() {
                let lease =
                    match crate::pg_rls::PgRlsLease::acquire(&pool, rls_tables, &budget).await {
                        Ok(lease) => lease,
                        Err(response) => return response,
                    };
                stream::multiple_lineage_body(
                    move |sink| {
                        Box::pin(async move {
                            let result = {
                                let mut backend = PgBackend::new(lease.client());
                                lineage_each_async_controlled(
                                    &plan,
                                    &spec,
                                    &mut backend,
                                    &control,
                                    sink,
                                )
                                .await
                            };
                            lease.finish().await?;
                            result
                        })
                    },
                    proof,
                    form,
                    budget,
                )
            } else {
                let conn = match crate::source_acquisition::acquire_pg(&pool, budget.clone()).await
                {
                    Ok(conn) => Arc::new(conn),
                    Err(response) => return response,
                };
                stream::multiple_lineage_body(
                    move |sink| {
                        Box::pin(async move {
                            let result = {
                                let mut backend =
                                    PgBackend::new(crate::backend::PgQueryClient(conn.clone()));
                                lineage_each_async_controlled(
                                    &plan,
                                    &spec,
                                    &mut backend,
                                    &control,
                                    sink,
                                )
                                .await
                            };
                            conn.finish_result(result, &control).await
                        })
                    },
                    proof,
                    form,
                    budget,
                )
            }
        }
    };
    Response::builder()
        .status(200)
        .header(axum::http::header::CONTENT_TYPE, crate::lineage::MEDIA_TYPE)
        .body(body)
        .expect("fixed lineage response")
}
