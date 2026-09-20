//! Sealed file-backed authored SQLite candidates and request execution.

use std::future::Future;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use sf_core::{SourceId, SourceMapping, Term, Triple};
use sf_sparql::{MappingDigest, Plan};
use sf_sql::backend::sqlite::{
    SqliteGenerationConnection, SqliteGenerationSchema, VerifiedSqliteGenerationLease,
};

use crate::budget::RequestBudget;
use crate::pg_generation::PgGenerationError;
use crate::semantic_admission::{MappingOrigin, SemanticAdmissionError, ValidatedMapping};
use crate::{IntrospectedSource, SemanticOntology, ServeError};

// Logical metadata/copy work for the finite 4 MiB / 8192-object source profile.
pub(crate) const CONTROL_SOURCE_WORK: u64 = 256 * 1024 * 1024;

pub(crate) struct SqliteGeneration {
    source_id: SourceId,
    mapping_digest: MappingDigest,
    schema: Arc<SqliteGenerationSchema>,
    members: Vec<SqliteGenerationConnection>,
    next: AtomicUsize,
}

impl SqliteGeneration {
    pub(crate) fn source_id(&self) -> SourceId {
        self.source_id
    }
    pub(crate) fn schema(&self) -> &Arc<SqliteGenerationSchema> {
        &self.schema
    }

    pub(crate) fn ensure_mapping(
        &self,
        mapping: &ValidatedMapping,
    ) -> Result<(), SemanticAdmissionError> {
        if mapping.origin() == MappingOrigin::Authored
            && mapping.source_id() == self.source_id
            && mapping.mapping_digest() == self.mapping_digest
        {
            Ok(())
        } else {
            Err(SemanticAdmissionError::ReceiptGenerationMismatch)
        }
    }

    pub(crate) async fn acquire(
        &self,
        budget: &RequestBudget,
    ) -> Result<SqliteRequestLease, PgGenerationError> {
        let index = self.next.fetch_add(1, Ordering::Relaxed) % self.members.len();
        let lease = budget
            .run(self.members[index].begin(Arc::new(budget.clone()), Some(self.schema.clone())))
            .await?
            .map_err(generation_error)?;
        Ok(SqliteRequestLease {
            source_id: self.source_id,
            lease,
        })
    }
}

/// Build protected members separately from the legacy backend handle. Neither
/// `sqlite_pool()` nor `pick()` can reveal a verified member's connection.
pub(crate) async fn build(
    path: String,
    pool_size: usize,
    mapping: SourceMapping,
    ontology: &SemanticOntology,
    budget: &RequestBudget,
    observe: &mut impl FnMut(SourceId, &IntrospectedSource) -> Result<(), ServeError>,
) -> Result<(IntrospectedSource, ValidatedMapping), ServeError> {
    let mapped = crate::pg_generation::authored::mapped_tables(&mapping)?;
    let opened = crate::pg_generation::candidate_work::run(budget, move || {
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
            | rusqlite::OpenFlags::SQLITE_OPEN_URI;
        let open = || rusqlite::Connection::open_with_flags(&path, flags);
        let mut members = Vec::with_capacity(pool_size.max(1));
        for _ in 0..pool_size.max(1) {
            members.push(SqliteGenerationConnection::open(&path)?);
        }
        Ok::<_, sf_sql::Error>((
            members,
            SqliteGenerationConnection::open(&path)?,
            crate::Backend::sqlite(open()?),
        ))
    })
    .await
    .map_err(startup_error)?
    .map_err(|_| startup_error(PgGenerationError::SourceUnavailable))?;
    let (members, control, backend) = opened;
    let lease = budget
        .run(control.begin(Arc::new(budget.clone()), None))
        .await
        .map_err(PgGenerationError::from)
        .map_err(startup_error)?
        .map_err(generation_error)
        .map_err(startup_error)?;
    let schema = lease.schema().clone();
    if mapped
        .iter()
        .any(|name| !schema.tables().iter().any(|table| &table.name == name))
    {
        let _ = lease.finish().await;
        return Err(startup_error(PgGenerationError::CapabilityDrift));
    }
    let expected = Arc::new(SqliteGeneration {
        source_id: mapping.source_id(),
        mapping_digest: MappingDigest::from_mapping(&mapping),
        schema,
        members,
        next: AtomicUsize::new(0),
    });
    let source = IntrospectedSource::observed_sqlite_generation(backend, expected);
    if let Err(error) = observe(mapping.source_id(), &source) {
        let _ = lease.finish().await;
        return Err(error);
    }
    let ontology = ontology.clone();
    let built = crate::pg_generation::candidate_work::run(budget, move || {
        ValidatedMapping::validate(mapping, MappingOrigin::Authored, &ontology, &source)
            .map(|mapping| (source, mapping))
    })
    .await;
    let closed = lease
        .finish()
        .await
        .map_err(generation_error)
        .map_err(startup_error);
    let admitted = built
        .map_err(startup_error)?
        .map_err(|_| startup_error(PgGenerationError::CapabilityDrift))?;
    closed?;
    Ok(admitted)
}

pub(crate) fn generation_error(error: sf_sql::Error) -> PgGenerationError {
    match error {
        sf_sql::Error::QueryControl(error) => PgGenerationError::Control(error),
        _ => PgGenerationError::SourceUnavailable,
    }
}

fn startup_error(error: PgGenerationError) -> ServeError {
    crate::pg_generation::authored::generation_error(error)
}

pub(crate) struct SqliteRequestLease {
    source_id: SourceId,
    lease: VerifiedSqliteGenerationLease,
}

impl SqliteRequestLease {
    pub(crate) fn source_id(&self) -> SourceId {
        self.source_id
    }
    pub(crate) async fn finish(self) -> Result<(), PgGenerationError> {
        self.lease.finish().await.map_err(generation_error)
    }

    async fn finish_result<T>(self, result: sf_sparql::Result<T>) -> sf_sparql::Result<T> {
        let closed = self.lease.finish().await.map_err(execution_error);
        let value = result?;
        closed?;
        Ok(value)
    }

    pub(crate) async fn select_each<F, Fut>(
        self,
        plan: &Plan,
        control: &dyn sf_core::query_control::QueryControl,
        sink: F,
    ) -> sf_sparql::Result<()>
    where
        F: FnMut(Vec<Option<Term>>) -> Fut + Send,
        Fut: Future<Output = sf_sparql::Result<()>> + Send,
    {
        let result = async {
            let mut backend = self.lease.backend().map_err(execution_error)?;
            sf_sparql::exec_core::select_each_async_controlled(plan, &mut backend, control, sink)
                .await
        }
        .await;
        self.finish_result(result).await
    }

    pub(crate) async fn construct_each<F, Fut>(
        self,
        plan: &Plan,
        budget: &RequestBudget,
        sink: F,
    ) -> sf_sparql::Result<()>
    where
        F: FnMut(Vec<Triple>) -> Fut + Send,
        Fut: Future<Output = sf_sparql::Result<()>> + Send,
    {
        let result = async {
            let mut backend = self.lease.backend().map_err(execution_error)?;
            sf_sparql::exec_core::construct_each_async_controlled(plan, &mut backend, budget, sink)
                .await
        }
        .await;
        self.finish_result(result).await
    }

    pub(crate) async fn ask(self, plan: &Plan, budget: &RequestBudget) -> sf_sparql::Result<bool> {
        let result = async {
            let mut backend = self.lease.backend().map_err(execution_error)?;
            sf_sparql::exec_core::ask_controlled(plan, &mut backend, budget).await
        }
        .await;
        self.finish_result(result).await
    }

    pub(crate) async fn lineage_each(
        self,
        plan: &Plan,
        spec: &sf_sparql::lineage::LineageSpec,
        budget: &RequestBudget,
        sink: crate::stream::OriginSink,
    ) -> sf_sparql::Result<()> {
        let result = async {
            let mut backend = self.lease.backend().map_err(execution_error)?;
            sf_sparql::exec_core::lineage_each_async_controlled(
                plan,
                spec,
                &mut backend,
                budget,
                sink,
            )
            .await
        }
        .await;
        self.finish_result(result).await
    }
}

fn execution_error(error: sf_sql::Error) -> sf_sparql::Error {
    match error {
        sf_sql::Error::QueryControl(error) => sf_sparql::Error::QueryControl(error),
        _ => sf_sparql::Error::Sql("verified SQLite generation failed".into()),
    }
}

#[cfg(test)]
#[path = "sqlite_generation_tests.rs"]
mod tests;
