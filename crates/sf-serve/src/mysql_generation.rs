//! Protected authored MySQL candidates and request leases on owned sessions.
use mysql_async::{Opts, Pool};
use sf_core::{SourceId, SourceMapping};
use sf_sparql::MappingDigest;
use sf_sql::source_work::SourceWork;
use std::sync::Arc;

use crate::budget::RequestBudget;
use crate::mysql_query::MysqlQuery;
use crate::pg_generation::PgGenerationError;
use crate::semantic_admission::{MappingOrigin, SemanticAdmissionError, ValidatedMapping};
use crate::{IntrospectedSource, SemanticOntology, ServeError};

pub(crate) mod schema;
pub(crate) mod session;
pub(crate) const CONTROL_SOURCE_WORK: u64 = 256 * 1024 * 1024;

pub(crate) struct MysqlGeneration {
    source_id: SourceId,
    mapping_digest: MappingDigest,
    names: Arc<[String]>,
    schema: Arc<schema::Schema>,
    pool: Pool,
}

impl MysqlGeneration {
    pub(crate) fn source_id(&self) -> SourceId {
        self.source_id
    }
    pub(crate) fn schema(&self) -> &Arc<schema::Schema> {
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
    ) -> Result<MysqlRequestLease, PgGenerationError> {
        SourceWork::new(Some(budget))
            .charge(1)
            .map_err(schema::sql_error)?;
        let conn = budget
            .run(self.pool.get_conn())
            .await?
            .map_err(|_| PgGenerationError::SourceUnavailable)?;
        let (query, _) = MysqlQuery::acquire_generation(
            conn,
            budget.clone(),
            self.names.clone(),
            Some(&self.schema),
        )
        .await?;
        Ok(MysqlRequestLease {
            source_id: self.source_id,
            query,
        })
    }
}

pub(crate) struct MysqlRequestLease {
    source_id: SourceId,
    query: MysqlQuery,
}
impl MysqlRequestLease {
    pub(crate) fn source_id(&self) -> SourceId {
        self.source_id
    }
    pub(crate) fn into_query(self) -> MysqlQuery {
        self.query
    }
    pub(crate) async fn finish(self) -> Result<(), PgGenerationError> {
        self.query.finish(Ok(())).await.map_err(execution_error)
    }
}

pub(crate) async fn build(
    options: Opts,
    mapping: SourceMapping,
    ontology: &SemanticOntology,
    budget: &RequestBudget,
    observe: &mut impl FnMut(SourceId, &IntrospectedSource) -> Result<(), ServeError>,
) -> Result<(IntrospectedSource, ValidatedMapping), ServeError> {
    let names = crate::pg_generation::authored::mapped_tables(&mapping)?;
    let options = crate::mysql_query::target_options(options)
        .map_err(|_| startup_error(PgGenerationError::CapabilityDrift))?;
    let request = Pool::new(options.clone());
    // An unpooled control session cannot consume the request lane or leave an
    // unowned pool teardown future behind on a rejected/reloaded candidate.
    let conn = budget
        .run(mysql_async::Conn::new(options))
        .await
        .map_err(PgGenerationError::from)
        .map_err(startup_error)?
        .map_err(|_| startup_error(PgGenerationError::SourceUnavailable))?;
    let (query, schema) = MysqlQuery::acquire_generation(conn, budget.clone(), names.clone(), None)
        .await
        .map_err(startup_error)?;
    let expected = Arc::new(MysqlGeneration {
        source_id: mapping.source_id(),
        mapping_digest: MappingDigest::from_mapping(&mapping),
        names,
        schema,
        pool: request.clone(),
    });
    let source = IntrospectedSource::observed_mysql_generation(
        request,
        expected,
        SourceWork::new(Some(budget)),
    )
    .map_err(startup_error)?;
    if let Err(error) = observe(mapping.source_id(), &source) {
        let _ = query.finish(Ok(())).await;
        return Err(error);
    }
    let ontology = ontology.clone();
    let built = crate::pg_generation::candidate_work::run(budget, move || {
        ValidatedMapping::validate(mapping, MappingOrigin::Authored, &ontology, &source)
            .map(|mapping| (source, mapping))
    })
    .await;
    let closed = query
        .finish(Ok(()))
        .await
        .map_err(execution_error)
        .map_err(startup_error);
    let admitted = built
        .map_err(startup_error)?
        .map_err(|_| startup_error(PgGenerationError::CapabilityDrift))?;
    closed?;
    Ok(admitted)
}

fn execution_error(error: sf_sparql::Error) -> PgGenerationError {
    match error {
        sf_sparql::Error::QueryControl(error) => PgGenerationError::Control(error),
        _ => PgGenerationError::SourceUnavailable,
    }
}
fn startup_error(error: PgGenerationError) -> ServeError {
    crate::pg_generation::authored::generation_error(error)
}
