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
use crate::pg_generation::{VerifiedGenerationLeases, VerifiedPostgresGenerationLease};
use crate::problem::{self, ProblemCode};
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
    VerifiedPostgres {
        lease: VerifiedPostgresGenerationLease,
        plan: Arc<Plan>,
    },
    MySql {
        connection: mysql_async::Conn,
        plan: Arc<Plan>,
    },
}

struct GenerationMismatch;

fn take_postgres_generation(
    generations: &mut VerifiedGenerationLeases,
    source_id: sf_core::SourceId,
    binding_identity: &crate::binding_identity::RuntimeBindingIdentity,
    verified_generation: bool,
) -> Result<Option<VerifiedPostgresGenerationLease>, GenerationMismatch> {
    if !generations.matches(source_id, binding_identity, verified_generation) {
        return Err(GenerationMismatch);
    }
    if !verified_generation {
        return Ok(None);
    }
    generations
        .take(source_id, binding_identity)
        .map(Some)
        .ok_or(GenerationMismatch)
}

/// Acquire both participating sources before committing a success response,
/// then concatenate their source-local solution streams through one serializer.
pub(crate) async fn select_union_body(
    execution: ExecutableFederatedPlan,
    mut generations: VerifiedGenerationLeases,
    format: QueryResultsFormat,
    budget: RequestBudget,
) -> Result<Body, Response> {
    let (variables, fragments) = execution.into_parts();
    let fragments = fragments.map(|fragment| fragment.into_parts());
    let valid = fragments[0].0 != fragments[1].0
        && fragments.iter().all(
            |(source_id, binding_identity, backend, verified_generation, _)| {
                generations.matches(*source_id, binding_identity, *verified_generation)
                    && (!*verified_generation || matches!(backend, Backend::Pg(_)))
            },
        );
    if !valid {
        let _ = generations.finish().await;
        return Err(problem::response(ProblemCode::Internal));
    }

    let mut acquired = Vec::with_capacity(2);
    for (source_id, binding_identity, backend, verified_generation, plan) in fragments {
        let source = match backend {
            Backend::Sqlite(pool) => sqlite_admission::acquire(&pool, &budget)
                .await
                .map(|lease| AcquiredFragment::Sqlite { lease, plan }),
            Backend::Pg(pool) => match take_postgres_generation(
                &mut generations,
                source_id,
                &binding_identity,
                verified_generation,
            ) {
                Ok(Some(lease)) => Ok(AcquiredFragment::VerifiedPostgres { lease, plan }),
                Ok(None) => crate::source_acquisition::acquire_pg(&pool, budget.clone())
                    .await
                    .map(|connection| AcquiredFragment::Postgres {
                        connection: Box::new(connection),
                        plan,
                    }),
                Err(GenerationMismatch) => Err(problem::response(ProblemCode::Internal)),
            },
            Backend::Mysql(pool) => crate::source_acquisition::acquire_mysql(&pool, &budget)
                .await
                .map(|connection| AcquiredFragment::MySql { connection, plan }),
        };
        match source {
            Ok(source) => acquired.push(source),
            Err(response) => {
                close_acquired(acquired).await;
                let _ = generations.finish().await;
                return Err(response);
            }
        }
    }
    if !generations.is_empty() {
        let _ = generations.finish().await;
        return Err(problem::response(ProblemCode::Internal));
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
                        AcquiredFragment::VerifiedPostgres { lease, plan } => {
                            match lease.select_each(&plan, &drive_budget, &mut sink).await {
                                Ok(result) => result?,
                                Err(_) => {
                                    return Err(sf_sparql::Error::Sql(
                                        "verified generation close failed".to_owned(),
                                    ))
                                }
                            }
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

async fn close_acquired(acquired: Vec<AcquiredFragment>) {
    for fragment in acquired {
        if let AcquiredFragment::VerifiedPostgres { lease, .. } = fragment {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), lease.finish()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binding_identity::RuntimeBindingIdentity;

    #[test]
    fn verified_fragment_without_its_identity_bound_lease_is_internal() {
        let mut generations = VerifiedGenerationLeases::default();
        let source_id = sf_core::SourceId::new(0).unwrap();
        let identity = RuntimeBindingIdentity::fresh();

        let Err(GenerationMismatch) =
            take_postgres_generation(&mut generations, source_id, &identity, true)
        else {
            panic!("verified fragment must not fall through to ordinary acquisition");
        };

        assert!(generations.is_empty());
    }
}
