//! Owned PostgreSQL queries: dirty before I/O, reusable only after a barrier.
use crate::budget::RequestBudget;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

pub(crate) struct PgConn {
    object: Option<deadpool_postgres::Object>,
    recyclable: AtomicBool,
    tls: tokio_postgres_rustls::MakeRustlsConnect,
    budget: Option<RequestBudget>,
}
pub(crate) struct PgQueryClient(pub(crate) Arc<PgConn>);
impl std::ops::Deref for PgQueryClient {
    type Target = tokio_postgres::Client;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PgConn {
    #[cfg(test)]
    pub(crate) async fn checked(
        conn: deadpool_postgres::Object,
        tls: tokio_postgres_rustls::MakeRustlsConnect,
    ) -> Result<Self, String> {
        Self::checked_owned(conn, tls, None).await
    }
    pub(crate) async fn checked_for_request(
        conn: deadpool_postgres::Object,
        tls: tokio_postgres_rustls::MakeRustlsConnect,
        budget: RequestBudget,
    ) -> Result<Self, String> {
        Self::checked_owned(conn, tls, Some(budget)).await
    }
    async fn checked_owned(
        conn: deadpool_postgres::Object,
        tls: tokio_postgres_rustls::MakeRustlsConnect,
        budget: Option<RequestBudget>,
    ) -> Result<Self, String> {
        // The guard exists before even the scope probe can be cancelled.
        let owned = Self {
            object: Some(conn),
            recyclable: AtomicBool::new(false),
            tls,
            budget,
        };
        super::verify_pg_relation_scope(&owned).await?;
        Ok(owned)
    }
    pub(crate) fn mark_generation_dirty(&self) {
        self.recyclable.store(false, Ordering::Release);
    }
    pub(crate) fn mark_recyclable(&self) {
        self.recyclable.store(true, Ordering::Release);
    }

    pub(crate) async fn bound_statement(&self, budget: &RequestBudget) -> sf_sparql::Result<()> {
        let millis = budget
            .remaining_duration()?
            .unwrap_or(Duration::from_secs(30))
            .as_millis()
            .saturating_add(1)
            .clamp(1, i32::MAX as u128);
        budget.run(self.batch_execute(&format!("SET statement_timeout = {millis}; SET idle_in_transaction_session_timeout = {millis}")))
            .await?.map_err(|_| sf_sparql::Error::Sql("source deadline setup failed".into()))
    }
    pub(crate) async fn finish(self: Arc<Self>, budget: &RequestBudget) -> sf_sparql::Result<()> {
        // A successful executor can stop before cursor EOF (ASK/LIMIT). This
        // acknowledged protocol barrier must drain preceding work before reuse.
        if Arc::strong_count(&self) != 1 {
            return Err(sf_sparql::Error::Sql(
                "source query ownership retained".into(),
            ));
        }
        budget
            .run(self.batch_execute(
                "ROLLBACK; RESET statement_timeout; RESET idle_in_transaction_session_timeout",
            ))
            .await?
            .map_err(|_| sf_sparql::Error::Sql("source query cleanup failed".into()))?;
        self.mark_recyclable();
        Ok(())
    }

    pub(crate) async fn finish_result<T>(
        self: Arc<Self>,
        result: sf_sparql::Result<T>,
        budget: &RequestBudget,
    ) -> sf_sparql::Result<T> {
        let value = result?;
        self.finish(budget).await?;
        Ok(value)
    }
}
impl std::ops::Deref for PgConn {
    type Target = tokio_postgres::Client;
    fn deref(&self) -> &Self::Target {
        self.object
            .as_deref()
            .expect("active PostgreSQL connection")
    }
}

// Captured BEFORE spawn: aborting an unpolled cleanup task still discards,
// rather than returning the raw pooled object to a new request.
struct Discard {
    object: Option<deadpool_postgres::Object>,
    _budget: Option<RequestBudget>,
}
impl Drop for Discard {
    fn drop(&mut self) {
        if let Some(object) = self.object.take() {
            drop(deadpool_postgres::Object::take(object));
        }
    }
}
impl Drop for PgConn {
    fn drop(&mut self) {
        if self.recyclable.load(Ordering::Acquire) {
            return;
        }
        let Some(object) = self.object.take() else {
            return;
        };
        let cancellation = object.cancel_token();
        let tls = self.tls.clone();
        let owned = Discard {
            object: Some(object),
            _budget: self.budget.take(),
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                // Native CancelRequest has no termination acknowledgement.
                // Hold the member and request capacity until the bounded attempt
                // ends, then detach/close regardless of its result.
                let _ =
                    tokio::time::timeout(Duration::from_secs(1), cancellation.cancel_query(tls))
                        .await;
                drop(owned);
            });
        }
    }
}
