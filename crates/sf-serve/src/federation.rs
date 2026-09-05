//! Execution of the bounded two-source `UnionAll` vertical.

use std::sync::Arc;

use axum::body::Body;
use axum::response::Response;
use sf_core::query_control::QueryControl;
use sf_sparql::{exec, exec_mysql, exec_pg, Plan};
use sparesults::QueryResultsFormat;

use crate::backend::PgConn;
use crate::binding::ExecutableFederatedPlan;
use crate::budget::RequestBudget;
use crate::{sqlite_admission, stream, Backend};

enum AcquiredFragment {
    Sqlite {
        lease: sf_sql::backend::sqlite::SqliteOwnedLease,
        plan: Arc<Plan>,
    },
    Postgres {
        connection: Box<PgConn>,
        plan: Arc<Plan>,
    },
    MySql {
        connection: mysql_async::Conn,
        plan: Arc<Plan>,
    },
}

/// Acquire both participating sources before committing a success response,
/// then concatenate their source-local solution streams through one serializer.
pub(crate) async fn select_union_body(
    execution: ExecutableFederatedPlan,
    format: QueryResultsFormat,
    budget: RequestBudget,
) -> Result<Body, Response> {
    let (variables, fragments) = execution.into_parts();
    let mut acquired = Vec::with_capacity(2);
    for fragment in fragments {
        let (backend, plan) = fragment.into_parts();
        let source = match backend {
            Backend::Sqlite(pool) => AcquiredFragment::Sqlite {
                lease: sqlite_admission::acquire(&pool, &budget).await?,
                plan,
            },
            Backend::Pg(pool) => AcquiredFragment::Postgres {
                connection: Box::new(crate::http::acquire_pg(&pool, budget.clone()).await?),
                plan,
            },
            Backend::Mysql(pool) => AcquiredFragment::MySql {
                connection: crate::http::acquire_mysql(&pool, &budget).await?,
                plan,
            },
        };
        acquired.push(source);
    }

    let drive_budget = budget.clone();
    Ok(stream::select_body_streaming_controlled(
        move |mut sink| {
            Box::pin(async move {
                for fragment in acquired {
                    match fragment {
                        AcquiredFragment::Sqlite { lease, plan } => {
                            let control: Arc<dyn QueryControl> = Arc::new(drive_budget.clone());
                            exec::select_each_sqlite_owned_interruptible_leased(
                                &plan, lease, control, &mut sink,
                            )
                            .await?;
                        }
                        AcquiredFragment::Postgres { connection, plan } => {
                            exec_pg::select_each_pg_controlled(
                                &plan,
                                *connection,
                                &drive_budget,
                                &mut sink,
                            )
                            .await?;
                        }
                        AcquiredFragment::MySql { connection, plan } => {
                            exec_mysql::select_each_mysql_controlled(
                                &plan,
                                connection,
                                &drive_budget,
                                &mut sink,
                            )
                            .await?;
                        }
                    }
                }
                Ok(())
            })
        },
        format,
        variables,
        budget.clone(),
    ))
}
