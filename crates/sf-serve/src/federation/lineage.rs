//! Typed origin execution reuses the already-acquired federation source owners.
use super::*;
use sf_core::SourceId;
use sf_sparql::exec_core::{lineage_each_async_controlled, LineageOutput};
use sf_sparql::lineage::LineageSpec;
use sf_sql::backend::{pg::PgBackend, sqlite::SqliteOwnedBackend};

pub(super) fn body(
    acquired: Vec<AcquiredFragment>,
    sources: [SourceId; 2],
    specs: [Arc<LineageSpec>; 2],
    proof: Arc<crate::lineage::Lineage>,
    variables: Vec<String>,
    budget: RequestBudget,
) -> Body {
    let control = budget.clone();
    stream::tagged_lineage_body(
        move |sink: stream::TaggedOriginSink| {
            Box::pin(async move {
                let sink = Arc::new(std::sync::Mutex::new(sink));
                let mut remaining = acquired.into_iter().zip(sources).zip(specs);
                while let Some(((fragment, source), spec)) = remaining.next() {
                    let sink = sink.clone();
                    let scope_budget = control.clone();
                    let mut peak = 0;
                    let sink: stream::OriginSink = Box::new(move |mut solution| {
                        let LineageOutput::Row(row) = &mut solution.output else {
                            return Box::pin(std::future::ready(Err(sf_sparql::Error::Mapping(
                                "lineage UNION expected row".into(),
                            ))));
                        };
                        if let Err(error) = scope_row(row, source, &scope_budget, &mut peak) {
                            return Box::pin(std::future::ready(Err(error)));
                        }
                        sink.lock().unwrap_or_else(|p| p.into_inner())(Some(source), solution)
                    });
                    if let Err(error) = drive_origin(fragment, &spec, &control, sink).await {
                        close_acquired(remaining.map(|((fragment, _), _)| fragment).collect())
                            .await;
                        return Err(error);
                    }
                }
                Ok(())
            })
        },
        proof,
        sf_sparql::PlanForm::Select { vars: variables },
        budget,
    )
}

async fn drive_origin(
    fragment: AcquiredFragment,
    spec: &LineageSpec,
    budget: &RequestBudget,
    sink: stream::OriginSink,
) -> sf_sparql::Result<()> {
    match fragment {
        AcquiredFragment::VerifiedSqlite { lease, plan } => {
            lease.lineage_each(&plan, spec, budget, sink).await
        }
        AcquiredFragment::Sqlite { lease, plan } => {
            let mut backend =
                SqliteOwnedBackend::new_controlled_leased(lease, Arc::new(budget.clone()));
            lineage_each_async_controlled(&plan, spec, &mut backend, budget, sink).await
        }
        AcquiredFragment::MySql { connection, plan } => {
            crate::mysql_query::lineage(&plan, spec, connection, budget, sink).await
        }
        AcquiredFragment::RlsPostgres { lease, plan } => {
            let result = {
                let mut backend = PgBackend::new(lease.client());
                lineage_each_async_controlled(&plan, spec, &mut backend, budget, sink).await
            };
            lease.finish().await?;
            result
        }
        AcquiredFragment::VerifiedPostgres { lease, plan } => lease
            .lineage_each(&plan, spec, budget, sink)
            .await
            .map_err(|_| sf_sparql::Error::Sql("verified generation close failed".into()))?,
        AcquiredFragment::Postgres { connection, plan } => {
            let conn = Arc::new(*connection);
            let result = {
                let mut backend = PgBackend::new(crate::backend::PgQueryClient(conn.clone()));
                lineage_each_async_controlled(&plan, spec, &mut backend, budget, sink).await
            };
            conn.finish_result(result, budget).await
        }
    }
}
