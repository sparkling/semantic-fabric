//! Complete off-path builder for the closed PostgreSQL Direct profile.

use std::sync::Arc;
use std::time::Duration;

use sf_core::query_control::QueryLimits;
use sf_core::SourceId;
use sf_sparql::Epoch;

use super::{PgDirectPools, ValidatedRuntimeCandidate};
use crate::budget::RequestBudget;
use crate::pg_generation::{
    BoundPostgresDirectCandidate, PgGenerationError, PostgresDirectExpectation,
    PG_DIRECT_CONTROL_SOURCE_WORK_V1,
};
use crate::semantic_admission::{MappingOrigin, ValidatedMapping};
use crate::snapshot::{RuntimeSnapshot, RuntimeSource};
use crate::{ReadinessCause, RuntimeReadiness, SemanticOntology};

const SINGLE_SOURCE_ID: usize = 0;

/// Immutable configuration for the one admitted lifecycle profile.
#[derive(Clone)]
pub(crate) struct PgDirectLifecycleSpec {
    pools: PgDirectPools,
    ontology: Arc<SemanticOntology>,
    base_iri: Arc<str>,
    operation_timeout: Duration,
}

impl PgDirectLifecycleSpec {
    pub(crate) fn from_resolved_config(
        config: tokio_postgres::Config,
        request_pool_size: usize,
        pool_wait: Duration,
        ontology: SemanticOntology,
        base_iri: &str,
        operation_timeout: Duration,
    ) -> Result<Self, ReadinessCause> {
        if operation_timeout.is_zero()
            || tokio::time::Instant::now()
                .checked_add(operation_timeout)
                .is_none()
        {
            return Err(ReadinessCause::CapabilityDrift);
        }
        sf_mapping::validate_direct_mapping_base(base_iri)
            .map_err(|_| ReadinessCause::CapabilityDrift)?;
        let pools = PgDirectPools::from_resolved_config(config, request_pool_size, pool_wait)
            .map_err(|_| ReadinessCause::CapabilityDrift)?;
        Ok(Self {
            pools,
            ontology: Arc::new(ontology),
            base_iri: Arc::from(base_iri),
            operation_timeout,
        })
    }

    pub(crate) async fn build_initial(&self) -> Result<InitialPgDirectGeneration, ReadinessCause> {
        let (snapshot, expectation) = self.build_snapshot(Epoch::default()).await?;
        Ok(InitialPgDirectGeneration::new(snapshot, expectation))
    }

    pub(crate) async fn build_candidate(
        &self,
        expected: RuntimeReadiness,
    ) -> Result<PgDirectLifecycleBuild, ReadinessCause> {
        let epoch = Epoch(expected.activation_id().get());
        let (snapshot, expectation) = self.build_snapshot(epoch).await?;
        let candidate = ValidatedRuntimeCandidate::from_validated(ValidatedCandidateParts::new(
            expected, snapshot,
        ));
        Ok(PgDirectLifecycleBuild::new(candidate, expectation))
    }

    pub(crate) async fn probe(
        &self,
        expectation: &PostgresDirectExpectation,
    ) -> Result<(), ReadinessCause> {
        let budget = self.control_budget();
        crate::pg_generation::probe_direct_expectation(&self.pools.control(), expectation, &budget)
            .await
            .map_err(generation_cause)
    }

    async fn build_snapshot(
        &self,
        epoch: Epoch,
    ) -> Result<(RuntimeSnapshot, PostgresDirectExpectation), ReadinessCause> {
        let budget = self.control_budget();
        let observed = observe_on_control(&self.pools, &budget).await?;
        let built = crate::pg_generation::build_and_bind_direct_candidate_on_control(
            observed,
            &self.pools.control(),
            &self.base_iri,
            source_id(),
            &budget,
        )
        .await
        .map_err(generation_cause)?;
        let ontology = Arc::clone(&self.ontology);
        crate::pg_generation::candidate_work::run(&budget, move || {
            finish_snapshot(built, &ontology, epoch)
        })
        .await
        .map_err(generation_cause)?
    }

    fn control_budget(&self) -> RequestBudget {
        RequestBudget::for_control(
            self.operation_timeout,
            QueryLimits::new(0, PG_DIRECT_CONTROL_SOURCE_WORK_V1, 0, 0),
        )
    }

    pub(super) const fn operation_timeout(&self) -> Duration {
        self.operation_timeout
    }

    #[cfg(test)]
    pub(crate) fn pool_capacities(&self) -> (usize, usize) {
        self.pools.capacities()
    }

    #[cfg(test)]
    pub(crate) fn pools_for_test(&self) -> PgDirectPools {
        self.pools.clone()
    }
}

/// Private construction proof: only this complete builder can mint it.
pub(super) struct ValidatedCandidateParts {
    expected: RuntimeReadiness,
    snapshot: RuntimeSnapshot,
}

impl ValidatedCandidateParts {
    fn new(expected: RuntimeReadiness, snapshot: RuntimeSnapshot) -> Self {
        Self { expected, snapshot }
    }

    pub(super) fn into_parts(self) -> (RuntimeReadiness, RuntimeSnapshot) {
        (self.expected, self.snapshot)
    }
}

pub(crate) struct InitialPgDirectGeneration {
    snapshot: RuntimeSnapshot,
    expectation: PostgresDirectExpectation,
}

impl InitialPgDirectGeneration {
    fn new(snapshot: RuntimeSnapshot, expectation: PostgresDirectExpectation) -> Self {
        Self {
            snapshot,
            expectation,
        }
    }

    pub(crate) fn into_parts(self) -> (RuntimeSnapshot, PostgresDirectExpectation) {
        (self.snapshot, self.expectation)
    }

    #[cfg(test)]
    pub(crate) const fn snapshot(&self) -> &RuntimeSnapshot {
        &self.snapshot
    }

    #[cfg(test)]
    pub(crate) const fn expectation(&self) -> &PostgresDirectExpectation {
        &self.expectation
    }
}

pub(crate) struct PgDirectLifecycleBuild {
    candidate: ValidatedRuntimeCandidate,
    expectation: PostgresDirectExpectation,
}

impl PgDirectLifecycleBuild {
    fn new(candidate: ValidatedRuntimeCandidate, expectation: PostgresDirectExpectation) -> Self {
        Self {
            candidate,
            expectation,
        }
    }

    pub(super) fn into_parts(self) -> (ValidatedRuntimeCandidate, PostgresDirectExpectation) {
        (self.candidate, self.expectation)
    }
}

async fn observe_on_control(
    pools: &PgDirectPools,
    budget: &RequestBudget,
) -> Result<crate::IntrospectedSource, ReadinessCause> {
    let control_pool = pools.control();
    let mut connection = budget
        .run(control_pool.get())
        .await
        .map_err(|_| ReadinessCause::SourceUnavailable)?
        .map_err(|_| ReadinessCause::SourceUnavailable)?;
    let snapshot = budget
        .run(sf_sql::introspect::introspect_postgres_public_observed_snapshot(&mut connection))
        .await
        .map_err(|_| ReadinessCause::SourceUnavailable)?
        .map_err(|_| ReadinessCause::SourceUnavailable)?;
    drop(connection);
    crate::IntrospectedSource::observed_postgres_direct(pools.request(), snapshot)
        .map_err(generation_cause)
}

fn finish_snapshot(
    built: BoundPostgresDirectCandidate,
    ontology: &SemanticOntology,
    epoch: Epoch,
) -> Result<(RuntimeSnapshot, PostgresDirectExpectation), ReadinessCause> {
    let (source, mapping, expectation) = built.into_parts();
    let mapping = ValidatedMapping::validate(mapping, MappingOrigin::Direct, ontology, &source)
        .map_err(|_| ReadinessCause::CapabilityDrift)?;
    let source =
        RuntimeSource::admitted(source, mapping).map_err(|_| ReadinessCause::CapabilityDrift)?;
    let snapshot = RuntimeSnapshot::single(epoch, ontology.clone(), source)
        .map_err(|_| ReadinessCause::CapabilityDrift)?;
    Ok((snapshot, expectation))
}

fn source_id() -> SourceId {
    SourceId::new(SINGLE_SOURCE_ID).expect("the closed single-source slot is representable")
}

pub(crate) fn generation_cause(error: PgGenerationError) -> ReadinessCause {
    match error {
        PgGenerationError::Control(_) | PgGenerationError::SourceUnavailable => {
            ReadinessCause::SourceUnavailable
        }
        PgGenerationError::SchemaDrift => ReadinessCause::SchemaDrift,
        PgGenerationError::CapabilityDrift
        | PgGenerationError::Mapping(_)
        | PgGenerationError::Internal => ReadinessCause::CapabilityDrift,
    }
}

#[cfg(test)]
#[path = "builder/tests.rs"]
mod tests;
