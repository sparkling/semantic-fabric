//! Execution of sealed two-source UNION and bounded inner-join profiles.

use std::sync::Arc;

use axum::body::Body;
use axum::response::Response;
use sf_core::query_control::QueryControl;
use sf_sparql::{exec, exec_pg, Plan};
use sparesults::QueryResultsFormat;

use crate::backend::PgConn;
use crate::binding::ExecutableFederatedPlan;
use crate::budget::RequestBudget;
use crate::pg_generation::{VerifiedGenerationLeases, VerifiedPostgresGenerationLease};
use crate::problem::{self, ProblemCode};
use crate::{sqlite_admission, stream, Backend};

mod join;
mod lineage;
type RowSink = Box<
    dyn FnMut(
            Vec<Option<sf_core::Term>>,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = sf_sparql::Result<()>> + Send>>
        + Send,
>;

enum AcquiredFragment {
    RlsPostgres {
        lease: crate::pg_rls::PgRlsLease,
        plan: Arc<Plan>,
    },
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
        connection: crate::mysql_query::MysqlQuery,
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
        .ok_or(GenerationMismatch)?
        .into_postgres()
        .map(Some)
        .map_err(|_| GenerationMismatch)
}

/// Acquire both participating sources before committing a success response,
/// then dispatch sequential UNION streaming or the capped pre-200 join.
pub(crate) async fn select_union_body(
    mut execution: ExecutableFederatedPlan,
    mut generations: VerifiedGenerationLeases,
    format: QueryResultsFormat,
    proof: Option<Arc<crate::lineage::Lineage>>,
    budget: RequestBudget,
) -> Result<Body, Response> {
    let join = execution.bounded_join();
    let (variables, fragments) = execution.into_parts();
    let specs = fragments.each_ref().map(|fragment| fragment.lineage());
    let rls_tables = fragments.each_ref().map(|fragment| fragment.rls_tables());
    let fragments = fragments.map(|fragment| fragment.into_parts());
    let source_ids = fragments.each_ref().map(|fragment| fragment.0);
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
    if proof.as_ref().is_some_and(|proof| {
        if let Some(join) = &join {
            !proof.matches_join(join, source_ids) || specs.iter().any(Option::is_some)
        } else {
            source_ids.iter().zip(&specs).any(|(id, spec)| {
                spec.as_ref()
                    .is_none_or(|spec| !proof.matches_source(*id, spec))
            })
        }
    }) || (proof.is_none()
        && (specs.iter().any(Option::is_some)
            || join.as_ref().is_some_and(|j| j.mapping_origins().is_some())))
    {
        let _ = generations.finish().await;
        return Err(problem::response(ProblemCode::Internal));
    }

    let mut acquired = Vec::with_capacity(2);
    for ((source_id, binding_identity, backend, verified_generation, plan), tables) in
        fragments.into_iter().zip(rls_tables)
    {
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
                Ok(None) if budget.postgres_rls().is_some() => {
                    crate::pg_rls::PgRlsLease::acquire(&pool, tables, &budget)
                        .await
                        .map(|lease| AcquiredFragment::RlsPostgres { lease, plan })
                }
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
        close_acquired(acquired).await;
        let _ = generations.finish().await;
        return Err(problem::response(ProblemCode::Internal));
    }

    if let Some(join) = join {
        let format = proof.map_or(
            stream::SelectFormat::Standard(format),
            stream::SelectFormat::Lineage,
        );
        return join::body(acquired, source_ids, join, variables, format, budget).await;
    }
    if let Some(proof) = proof {
        return Ok(lineage::body(
            acquired,
            source_ids,
            specs.map(Option::unwrap),
            proof,
            variables,
            budget,
        ));
    }

    let drive_budget = budget.clone();
    Ok(stream::select_body_streaming_controlled(
        move |sink| {
            Box::pin(async move {
                let sink = Arc::new(std::sync::Mutex::new(sink));
                for (fragment, source) in acquired.into_iter().zip(source_ids) {
                    let sink = sink.clone();
                    let scope_budget = drive_budget.clone();
                    let mut peak = 0;
                    let mut scoped: RowSink = Box::new(move |mut row| {
                        if let Err(error) = scope_row(&mut row, source, &scope_budget, &mut peak) {
                            return Box::pin(std::future::ready(Err(error)));
                        }
                        sink.lock().unwrap_or_else(|p| p.into_inner())(row)
                    });
                    drive(
                        fragment,
                        &drive_budget,
                        Arc::new(drive_budget.clone()),
                        &mut scoped,
                    )
                    .await?;
                }
                Ok(())
            })
        },
        format,
        variables,
        budget.clone(),
    ))
}

fn scope_row(
    row: &mut [Option<sf_core::Term>],
    source: sf_core::SourceId,
    budget: &RequestBudget,
    peak: &mut u64,
) -> sf_sparql::Result<()> {
    let bytes = join::row_bytes(row)?
        .checked_mul(8)
        .ok_or(sf_core::query_control::QueryControlError::AccountingOverflow)?;
    if bytes > *peak {
        budget.consume(
            sf_core::query_control::QueryCharge::RetainedBytes,
            bytes - *peak,
        )?;
        *peak = bytes;
    }
    join::scope(row, source);
    Ok(())
}

async fn drive(
    fragment: AcquiredFragment,
    budget: &RequestBudget,
    control: Arc<dyn QueryControl>,
    sink: &mut RowSink,
) -> sf_sparql::Result<()> {
    match fragment {
        AcquiredFragment::RlsPostgres { lease, plan } => {
            let result =
                exec_pg::select_each_pg_controlled(&plan, lease.client(), control.as_ref(), sink)
                    .await;
            lease.finish().await?;
            result
        }
        AcquiredFragment::Sqlite { lease, plan } => {
            exec::select_each_sqlite_owned_interruptible_leased(&plan, lease, control, sink).await
        }
        AcquiredFragment::Postgres { connection, plan } => {
            let conn = Arc::new(*connection);
            let result = exec_pg::select_each_pg_controlled(
                &plan,
                crate::backend::PgQueryClient(conn.clone()),
                control.as_ref(),
                sink,
            )
            .await;
            conn.finish_result(result, budget).await
        }
        AcquiredFragment::VerifiedPostgres { lease, plan } => lease
            .select_each_with_control(&plan, budget, control.as_ref(), sink)
            .await
            .map_err(|_| sf_sparql::Error::Sql("verified generation close failed".into()))?,
        AcquiredFragment::MySql { connection, plan } => {
            crate::mysql_query::select(&plan, connection, control.as_ref(), sink).await
        }
    }
}

impl AcquiredFragment {
    fn plan_mut(&mut self) -> &mut Arc<Plan> {
        match self {
            Self::Sqlite { plan, .. }
            | Self::Postgres { plan, .. }
            | Self::RlsPostgres { plan, .. }
            | Self::VerifiedPostgres { plan, .. }
            | Self::MySql { plan, .. } => plan,
        }
    }
}

async fn close_acquired(acquired: Vec<AcquiredFragment>) {
    for fragment in acquired {
        match fragment {
            AcquiredFragment::VerifiedPostgres { lease, .. } => {
                let _ = lease.rollback_bounded().await;
            }
            AcquiredFragment::RlsPostgres { lease, .. } => {
                let _ = lease.finish().await;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binding_identity::RuntimeBindingIdentity;

    #[test]
    fn source_scope_preserves_nested_blank_identity_and_charges_before_mutation() {
        use oxrdf::{BlankNode, NamedNode, Term, Triple};
        let blank = BlankNode::new("same").unwrap();
        let predicate = NamedNode::new("urn:p").unwrap();
        let triple = Triple::new(
            blank.clone(),
            predicate.clone(),
            Term::Triple(Box::new(Triple::new(
                blank.clone(),
                predicate,
                blank.clone(),
            ))),
        );
        let original = vec![
            Some(Term::BlankNode(blank)),
            Some(Term::Triple(Box::new(triple))),
        ];
        let budget = RequestBudget::after(
            std::time::Duration::from_secs(2),
            crate::DEFAULT_QUERY_LIMITS,
        );
        let mut left = original.clone();
        let mut right = original.clone();
        scope_row(
            &mut left,
            sf_core::SourceId::new(0).unwrap(),
            &budget,
            &mut 0,
        )
        .unwrap();
        scope_row(
            &mut right,
            sf_core::SourceId::new(1).unwrap(),
            &budget,
            &mut 0,
        )
        .unwrap();
        assert_ne!(left, right);
        let Some(Term::Triple(outer)) = &left[1] else {
            panic!("triple")
        };
        assert_eq!(Some(Term::from(outer.subject.clone())), left[0]);
        let Term::Triple(inner) = &outer.object else {
            panic!("nested triple")
        };
        assert_eq!(Some(inner.object.clone()), left[0]);
        let zero = RequestBudget::after(
            std::time::Duration::from_secs(2),
            sf_core::query_control::QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
                .with_max_retained_bytes(0),
        );
        let mut denied = original.clone();
        assert!(scope_row(
            &mut denied,
            sf_core::SourceId::new(0).unwrap(),
            &zero,
            &mut 0
        )
        .is_err());
        assert_eq!(denied, original);
    }

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
