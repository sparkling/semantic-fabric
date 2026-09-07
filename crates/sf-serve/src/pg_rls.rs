//! Same-connection transaction-local identity with fail-closed pool ownership.
use crate::{
    backend::PgConn,
    budget::RequestBudget,
    problem::{self, ProblemCode},
};
use axum::response::Response;
use sf_core::query_control::{QueryCharge, QueryControl};
use std::{collections::BTreeSet, sync::Arc, time::Duration};

/// The bounded profile excludes authored SQL, views, qualified names and Direct
/// Mapping generations. Guard every mapping table, including parent references.
pub(crate) fn mapped_tables(mapping: &sf_core::SourceMapping) -> Option<Arc<[String]>> {
    let mut tables = BTreeSet::new();
    for map in mapping.triples_maps() {
        let sf_core::ir::LogicalSource::Table(name) = &map.source else {
            return None;
        };
        if name.is_empty()
            || name.len() > 63
            || name.bytes().enumerate().any(|(i, c)| {
                !(c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
            })
        {
            return None;
        }
        tables.insert(name.clone());
    }
    if tables.is_empty() || tables.len() > 256 {
        return None;
    }
    Some(tables.into_iter().collect::<Vec<_>>().into())
}

pub(crate) struct PgRlsLease(Arc<PgConn>);
pub(crate) struct PgRlsClient(Arc<PgConn>);
impl std::ops::Deref for PgRlsClient {
    type Target = tokio_postgres::Client;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PgRlsLease {
    pub(crate) async fn acquire(
        pool: &crate::PostgresPool,
        tables: Option<Arc<[String]>>,
        budget: &RequestBudget,
    ) -> Result<Self, Response> {
        let claims = budget.postgres_rls().ok_or_else(denied)?;
        let tables = tables.ok_or_else(denied)?;
        budget
            .consume(
                QueryCharge::SourceWork,
                (2 * tables.len() + claims.0.len() + 2) as u64,
            )
            .map_err(problem::response_for_control)?;
        let conn = crate::source_acquisition::acquire_pg(pool, budget.clone()).await?;
        conn.mark_generation_dirty(); // BEFORE sending BEGIN, including cancellation.
        let lease = Self(Arc::new(conn));
        let millis = budget
            .remaining_duration()
            .map_err(problem::response_for_control)?
            .unwrap_or(Duration::from_secs(30))
            .as_millis()
            .clamp(1, 2_147_483_647);
        let begin = format!(
            "BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY; \
            SET LOCAL statement_timeout = {millis}; SET LOCAL lock_timeout = {}; \
            SET LOCAL idle_in_transaction_session_timeout = {millis}; \
            SET LOCAL search_path = pg_catalog, public, pg_temp; SET LOCAL row_security = on;",
            millis.min(1000)
        );
        let setup = budget.run(async {
            lease.0.batch_execute(&begin).await.map_err(|_| unavailable())?;
            // Hold ACCESS SHARE until rollback. ALTER TABLE / policy DDL cannot
            // invalidate the active-table guard while source execution runs.
            for name in tables.iter() {
                lease.0.batch_execute(&format!("LOCK TABLE public.\"{name}\" IN ACCESS SHARE MODE"))
                    .await.map_err(|error| match error.code() {
                        Some(&tokio_postgres::error::SqlState::LOCK_NOT_AVAILABLE)
                        | Some(&tokio_postgres::error::SqlState::QUERY_CANCELED) => unavailable(),
                        _ => denied(),
                    })?;
            }
            let role = lease.0.query_one("SELECT NOT rolsuper AND NOT rolbypassrls \
                AND current_user = session_user AS safe FROM pg_catalog.pg_roles WHERE rolname = current_user", &[])
                .await.map_err(|_| unavailable())?;
            if !role.get::<_,bool>("safe") { return Err(denied()); }
            for name in tables.iter() {
                let row = lease.0.query_opt("SELECT c.relkind = 'r' AND NOT c.relispartition \
                    AND pg_catalog.to_regclass(pg_catalog.quote_ident($1)) = c.oid \
                    AND NOT pg_catalog.pg_has_role(c.relowner, 'USAGE') \
                    AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_inherits WHERE inhrelid=c.oid OR inhparent=c.oid) \
                    AND pg_catalog.row_security_active(c.oid) AS safe \
                    FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                    WHERE n.nspname='public' AND c.relname=$1", &[name]).await.map_err(|_| unavailable())?;
                if !row.is_some_and(|row| row.get::<_,bool>("safe")) { return Err(denied()); }
            }
            for (key,value) in &claims.0 {
                lease.0.query_one("SELECT pg_catalog.set_config($1, $2, true)", &[key,value])
                    .await.map_err(|_| denied())?;
            }
            Ok(())
        }).await;
        match setup {
            Ok(Ok(())) => Ok(lease),
            Ok(Err(response)) => {
                let _ = lease.finish().await;
                Err(response)
            }
            Err(error) => {
                let _ = lease.finish().await;
                Err(problem::response_for_control(error))
            }
        }
    }

    pub(crate) fn client(&self) -> PgRlsClient {
        PgRlsClient(self.0.clone())
    }

    /// Acknowledged rollback and unique ownership are necessary for pool reuse.
    /// Dropped/aborted futures keep PgConn dirty; its Drop detaches the session.
    pub(crate) async fn finish(self) -> sf_sparql::Result<()> {
        if Arc::strong_count(&self.0) == 1
            && matches!(
                tokio::time::timeout(Duration::from_secs(2), self.0.batch_execute("ROLLBACK"))
                    .await,
                Ok(Ok(()))
            )
        {
            self.0.mark_recyclable();
            Ok(())
        } else {
            Err(sf_sparql::Error::Sql(
                "row security transaction close failed".into(),
            ))
        }
    }
}

pub(crate) fn denied() -> Response {
    crate::access_telemetry::record(crate::access_telemetry::AccessDecision::Deny);
    problem::response(ProblemCode::AccessDenied)
}
fn unavailable() -> Response {
    problem::response_with_retry_after(ProblemCode::SourceUnavailable)
}
