//! Backend/schema observations admitted to runtime construction.

use std::fmt;

use sf_sql::TableSchema;

use crate::backend::{Backend, BackendKind};
use crate::pg_generation::{PgGenerationError, PostgresDirectSourceCandidate, SourceGeneration};
use crate::schema_observation::SourceSchemaObservationV1;

/// A backend paired with the schema observation made through that backend.
///
/// Pairing prevents later constructors from independently mixing a handle and
/// unrelated schema vector. PostgreSQL observes one coherent read-only,
/// repeatable-read `public` catalogue snapshot; SQLite and MySQL do not yet
/// observe a whole catalogue in one explicit transaction. No path detects later
/// drift, so this type deliberately says `Introspected`, not `VerifiedSnapshot`.
pub struct IntrospectedSource {
    backend: Backend,
    schema: Vec<TableSchema>,
    observation: SourceSchemaObservationV1,
    generation: SourceGeneration,
}

impl IntrospectedSource {
    /// Build an explicitly unchecked pair inside unit tests only.
    #[cfg(test)]
    pub(crate) fn unchecked(backend: Backend, schema: Vec<TableSchema>) -> Self {
        Self {
            backend,
            schema,
            observation: SourceSchemaObservationV1::unavailable(),
            generation: SourceGeneration::Unverified,
        }
    }

    pub(crate) fn observed(backend: Backend, schema: Vec<TableSchema>) -> Self {
        Self {
            backend,
            schema,
            observation: SourceSchemaObservationV1::unavailable(),
            generation: SourceGeneration::Unverified,
        }
    }

    pub(crate) fn observed_postgres(
        pool: deadpool_postgres::Pool,
        snapshot: sf_sql::introspect::Postgres16PublicObservedSnapshotV1,
    ) -> Self {
        let (schema, observation) = SourceSchemaObservationV1::from_postgres_snapshot(snapshot);
        Self {
            backend: Backend::Pg(pool),
            schema,
            observation,
            generation: SourceGeneration::Unverified,
        }
    }

    #[cfg(test)]
    pub(crate) fn postgres_unavailable(
        pool: deadpool_postgres::Pool,
        schema: Vec<TableSchema>,
        reason: sf_sql::introspect::PostgresSchemaIdentityUnavailableV1,
    ) -> Self {
        Self {
            backend: Backend::Pg(pool),
            schema,
            observation: SourceSchemaObservationV1::postgres_unavailable(reason),
            generation: SourceGeneration::Unverified,
        }
    }

    /// Observe every table through the exact SQLite backend that will serve
    /// requests. Callers cannot inject a detached schema vector.
    pub fn observe_sqlite(backend: Backend) -> Result<Self, String> {
        let Backend::Sqlite(pool) = &backend else {
            return Err("SQLite source observation requires a SQLite backend".to_owned());
        };
        let connection = pool.pick();
        let guard = connection
            .lock()
            .map_err(|_| "SQLite source observation is unavailable".to_owned())?;
        let schema = crate::introspect_sqlite_all(&guard)?;
        drop(guard);
        Ok(Self::observed(backend, schema))
    }

    /// Observe PostgreSQL through the exact configured pool and relation scope
    /// that the serving lane will use.
    pub async fn observe_postgres(pool: deadpool_postgres::Pool) -> Result<Self, String> {
        let mut connection = pool
            .get()
            .await
            .map_err(|_| "PostgreSQL source observation is unavailable".to_owned())?;
        crate::backend::verify_pg_relation_scope(&connection).await?;
        let snapshot =
            sf_sql::introspect::introspect_postgres_public_observed_snapshot(&mut connection)
                .await
                .map_err(|_| "PostgreSQL source observation is unavailable".to_owned())?;
        drop(connection);
        Ok(Self::observed_postgres(pool, snapshot))
    }

    /// Observe MySQL through the exact configured pool and database that the
    /// serving lane will use.
    pub async fn observe_mysql(pool: mysql_async::Pool) -> Result<Self, String> {
        let mut connection = pool
            .get_conn()
            .await
            .map_err(|_| "MySQL source observation is unavailable".to_owned())?;
        let schema = crate::run::introspect_mysql_all(&mut connection).await?;
        drop(connection);
        Ok(Self::observed(Backend::Mysql(pool), schema))
    }

    /// Clone the observed SQLite pool for cancellation/lifecycle integration
    /// tests or embedding diagnostics without exposing schema construction.
    pub fn sqlite_pool(&self) -> Option<crate::SqlitePool> {
        match &self.backend {
            Backend::Sqlite(pool) => Some(pool.clone()),
            Backend::Pg(_) | Backend::Mysql(_) => None,
        }
    }

    pub(crate) fn bind_postgres_direct(
        mut self,
        candidate: PostgresDirectSourceCandidate,
    ) -> Result<Self, PgGenerationError> {
        if self.kind() != BackendKind::Postgres {
            return Err(PgGenerationError::Internal);
        }
        let (schema, observation, generation) = candidate.into_parts();
        self.schema = schema;
        self.observation = observation;
        self.generation = generation;
        Ok(self)
    }

    pub const fn kind(&self) -> BackendKind {
        self.backend.kind()
    }

    pub(crate) fn observed_schema(&self) -> &[TableSchema] {
        &self.schema
    }

    pub(crate) fn backend(&self) -> &Backend {
        &self.backend
    }

    pub(crate) fn ensure_generation_mapping(
        &self,
        mapping: &crate::semantic_admission::ValidatedMapping,
    ) -> Result<(), crate::SemanticAdmissionError> {
        self.generation.ensure_mapping(mapping)
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Backend,
        Vec<TableSchema>,
        SourceSchemaObservationV1,
        SourceGeneration,
    ) {
        (self.backend, self.schema, self.observation, self.generation)
    }
}

impl fmt::Debug for IntrospectedSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IntrospectedSource")
            .field("backend_kind", &self.kind())
            .field("schema_table_count", &self.schema.len())
            .field("verified_generation", &self.generation.is_verified())
            .finish()
    }
}
