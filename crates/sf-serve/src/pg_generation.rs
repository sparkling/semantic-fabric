//! PostgreSQL Direct-Mapping generation leases (ADR-0003 and ADR-0050).
//!
//! A lease owns one pool object from before `BEGIN` until a clean `ROLLBACK`.
//! Any cancellation, error, or drop before that rollback permanently detaches
//! the object and aborts its driver task; a possibly open transaction therefore
//! never reaches the pool recycler.

use std::collections::BTreeMap;
use std::sync::Arc;
#[cfg(test)]
use std::time::Duration;

use sf_core::schema_identity::ObservedSchemaIdentityV1;
use sf_core::{SourceId, SourceMapping, TableSchema};
use sf_sparql::MappingDigest;

use crate::binding_identity::RuntimeBindingIdentity;
use crate::budget::RequestBudget;
use crate::schema_observation::SourceSchemaObservationV1;
use crate::semantic_admission::{MappingOrigin, SemanticAdmissionError, ValidatedMapping};
use crate::telemetry::{self, Stage};

pub(crate) mod candidate_work;
mod context;
mod error;
mod execution;
mod lease;

pub(crate) use error::PgGenerationError;
pub(crate) use lease::VerifiedPostgresGenerationLease;

#[cfg(test)]
use context::validate_session_context;
use context::{capture_session_context, PgSessionContext, POSTGRES_SESSION_CONTEXT_QUERY_COUNT_V1};
#[cfg(test)]
use lease::{open_generation_before_lock_for_test, transaction_setup_sql, BEGIN_GENERATION_SQL};
use lease::{open_observed_generation, GENERATION_METADATA_PROBE_RESERVATION};

pub(crate) const PG_DIRECT_CONTROL_SOURCE_WORK_V1: u64 = GENERATION_METADATA_PROBE_RESERVATION;

#[derive(Clone, Default)]
pub(crate) enum SourceGeneration {
    #[default]
    Unverified,
    DirectPostgres(Arc<PostgresDirectGeneration>),
}

impl SourceGeneration {
    fn direct_postgres(generation: Arc<PostgresDirectGeneration>) -> Self {
        Self::DirectPostgres(generation)
    }

    pub(crate) fn requirement(
        &self,
        backend: &crate::Backend,
        binding_identity: &RuntimeBindingIdentity,
    ) -> Result<Option<PgGenerationRequirement>, PgGenerationError> {
        match self {
            Self::Unverified => Ok(None),
            Self::DirectPostgres(expected) => match backend {
                crate::Backend::Pg(pool) => Ok(Some(PgGenerationRequirement {
                    pool: pool.clone(),
                    expected: Arc::clone(expected),
                    binding_identity: binding_identity.clone(),
                })),
                crate::Backend::Sqlite(_) | crate::Backend::Mysql(_) => {
                    Err(PgGenerationError::Internal)
                }
            },
        }
    }

    pub(crate) const fn is_verified(&self) -> bool {
        matches!(self, Self::DirectPostgres(_))
    }

    pub(crate) fn ensure_mapping(
        &self,
        mapping: &ValidatedMapping,
    ) -> Result<(), SemanticAdmissionError> {
        match self {
            Self::Unverified => Ok(()),
            Self::DirectPostgres(expected)
                if mapping.origin() == MappingOrigin::Direct
                    && mapping.source_id() == expected.source_id
                    && mapping.mapping_digest() == expected.mapping_digest =>
            {
                Ok(())
            }
            Self::DirectPostgres(_) => Err(SemanticAdmissionError::ReceiptGenerationMismatch),
        }
    }
}

/// Immutable expectation produced by one successfully closed candidate lease.
pub(crate) struct PostgresDirectGeneration {
    source_id: SourceId,
    base_iri: Arc<str>,
    row_identity: sf_mapping::DirectMappingRowIdentity,
    mapping_digest: MappingDigest,
    identity: ObservedSchemaIdentityV1,
    session: PgSessionContext,
    tables: Arc<[TableSchema]>,
}

/// Opaque expectation retained by the one Direct-Mapping control coordinator.
#[derive(Clone)]
pub(crate) struct PostgresDirectExpectation(Arc<PostgresDirectGeneration>);

impl std::fmt::Debug for PostgresDirectGeneration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PostgresDirectGeneration")
            .field("source_id", &self.source_id)
            .field("base_iri_bytes", &self.base_iri.len())
            .field("row_identity", &self.row_identity)
            .field("mapping_digest", &self.mapping_digest)
            .field("identity", &self.identity)
            .field("table_count", &self.tables.len())
            .finish()
    }
}

pub(crate) struct PgGenerationRequirement {
    pool: crate::PostgresPool,
    expected: Arc<PostgresDirectGeneration>,
    binding_identity: RuntimeBindingIdentity,
}

impl PgGenerationRequirement {
    pub(crate) fn source_id(&self) -> SourceId {
        self.expected.source_id
    }
}

/// Request-owned verified transactions keyed by source and exact binding.
type RuntimeBoundGenerationLease = (RuntimeBindingIdentity, VerifiedPostgresGenerationLease);

#[derive(Default)]
pub(crate) struct VerifiedGenerationLeases {
    leases: BTreeMap<SourceId, RuntimeBoundGenerationLease>,
}

impl VerifiedGenerationLeases {
    pub(crate) async fn acquire(
        mut requirements: Vec<PgGenerationRequirement>,
        budget: &RequestBudget,
    ) -> Result<Self, PgGenerationError> {
        requirements.sort_by_key(PgGenerationRequirement::source_id);
        if requirements
            .windows(2)
            .any(|pair| pair[0].source_id() == pair[1].source_id())
        {
            return Err(PgGenerationError::Internal);
        }
        let mut leases = BTreeMap::new();
        for requirement in requirements {
            let source_id = requirement.source_id();
            let binding_identity = requirement.binding_identity.clone();
            match acquire_expected(requirement, budget).await {
                Ok(lease) => {
                    leases.insert(source_id, (binding_identity, lease));
                }
                Err(error) => {
                    close_leases(leases).await;
                    return Err(error);
                }
            }
        }
        Ok(Self { leases })
    }

    pub(crate) fn take(
        &mut self,
        source_id: SourceId,
        binding_identity: &RuntimeBindingIdentity,
    ) -> Option<VerifiedPostgresGenerationLease> {
        if !self.leases.get(&source_id)?.0.ptr_eq(binding_identity) {
            return None;
        }
        let (_, lease) = self.leases.remove(&source_id)?;
        debug_assert_eq!(lease.source_id(), source_id);
        Some(lease)
    }

    pub(crate) fn matches(
        &self,
        source_id: SourceId,
        binding_identity: &RuntimeBindingIdentity,
        verified_generation: bool,
    ) -> bool {
        match self.leases.get(&source_id) {
            Some((identity, _)) => verified_generation && identity.ptr_eq(binding_identity),
            None => !verified_generation,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.leases.is_empty()
    }

    /// Roll every still-owned transaction back, attempting all leases even if
    /// one close fails. Dirty failures detach on final drop.
    pub(crate) async fn finish(self) -> Result<(), PgGenerationError> {
        finish_leases(self.leases).await
    }
}

/// Source-side candidate whose schema, committed observation, and generation
/// expectation can only be assembled together by this module.
pub(crate) struct PostgresDirectSourceCandidate {
    tables: Vec<TableSchema>,
    observation: SourceSchemaObservationV1,
    generation: SourceGeneration,
}

impl PostgresDirectSourceCandidate {
    pub(crate) fn into_parts(
        self,
    ) -> (
        Vec<TableSchema>,
        SourceSchemaObservationV1,
        SourceGeneration,
    ) {
        (self.tables, self.observation, self.generation)
    }
}

/// Candidate output created while the exact rich observation transaction is
/// open, then returned only after its final exact recheck and clean rollback.
pub(crate) struct PostgresDirectCandidate {
    mapping: SourceMapping,
    source: PostgresDirectSourceCandidate,
}

impl PostgresDirectCandidate {
    pub(crate) fn into_parts(self) -> (SourceMapping, PostgresDirectSourceCandidate) {
        (self.mapping, self.source)
    }
}

/// Complete source-side result awaiting semantic admission and snapshot build.
pub(crate) struct BoundPostgresDirectCandidate {
    source: crate::IntrospectedSource,
    mapping: SourceMapping,
    expectation: PostgresDirectExpectation,
}

impl BoundPostgresDirectCandidate {
    pub(crate) fn into_parts(
        self,
    ) -> (
        crate::IntrospectedSource,
        SourceMapping,
        PostgresDirectExpectation,
    ) {
        (self.source, self.mapping, self.expectation)
    }
}

/// Consume one request-lane source, derive candidate inputs on the separately
/// supplied control pool, and return that source with inseparable facts from
/// the exact protected rich observation installed.
pub(crate) async fn build_and_bind_direct_candidate_on_control(
    source: crate::IntrospectedSource,
    control_pool: &crate::PostgresPool,
    base_iri: &str,
    source_id: SourceId,
    budget: &RequestBudget,
) -> Result<BoundPostgresDirectCandidate, PgGenerationError> {
    match source.backend() {
        crate::Backend::Pg(_) => {}
        crate::Backend::Sqlite(_) | crate::Backend::Mysql(_) => {
            return Err(PgGenerationError::Internal)
        }
    }
    let discovery = source.observed_schema().to_vec();
    let candidate =
        build_direct_candidate(control_pool, &discovery, base_iri, source_id, budget).await?;
    let (mapping, source_candidate) = candidate.into_parts();
    let SourceGeneration::DirectPostgres(expectation) = &source_candidate.generation else {
        return Err(PgGenerationError::Internal);
    };
    let expectation = PostgresDirectExpectation(Arc::clone(expectation));
    let source = source.bind_postgres_direct(source_candidate)?;
    Ok(BoundPostgresDirectCandidate {
        source,
        mapping,
        expectation,
    })
}

/// Compatibility helper for the low-level lease tests. The production
/// lifecycle always supplies its distinct control pool explicitly.
#[cfg(test)]
pub(crate) async fn build_and_bind_direct_candidate(
    source: crate::IntrospectedSource,
    base_iri: &str,
    source_id: SourceId,
    budget: &RequestBudget,
) -> Result<(crate::IntrospectedSource, SourceMapping), PgGenerationError> {
    let pool = match source.backend() {
        crate::Backend::Pg(pool) => pool.clone(),
        crate::Backend::Sqlite(_) | crate::Backend::Mysql(_) => {
            return Err(PgGenerationError::Internal)
        }
    };
    let candidate =
        build_and_bind_direct_candidate_on_control(source, &pool, base_iri, source_id, budget)
            .await?;
    let (source, mapping, _) = candidate.into_parts();
    Ok((source, mapping))
}

pub(crate) async fn probe_direct_expectation(
    control_pool: &crate::PostgresPool,
    expectation: &PostgresDirectExpectation,
    budget: &RequestBudget,
) -> Result<(), PgGenerationError> {
    let expected = Arc::clone(&expectation.0);
    let table_names = normalized_table_names(&expected.tables)?;
    let lease = open_observed_generation(control_pool, expected.source_id, &table_names, budget)
        .await?
        .promote_expected(expected)
        .await?;
    lease.finish_bounded(budget).await
}

async fn build_direct_candidate(
    pool: &crate::PostgresPool,
    discovery: &[TableSchema],
    base_iri: &str,
    source_id: SourceId,
    budget: &RequestBudget,
) -> Result<PostgresDirectCandidate, PgGenerationError> {
    let discovered_names = normalized_table_names(discovery)?;
    let observed = open_observed_generation(pool, source_id, &discovered_names, budget).await?;
    let tables = observed.tables().to_vec();
    if !postgres_direct_table_profile_is_unambiguous(&tables) {
        return observed.reject(PgGenerationError::CapabilityDrift).await;
    }
    let current_names = match normalized_table_names(&tables) {
        Ok(names) => names,
        Err(error) => return observed.reject(error).await,
    };
    if current_names != discovered_names {
        return observed.reject(PgGenerationError::SchemaDrift).await;
    }
    let identity = observed.identity();
    let session = observed.session().clone();
    let row_identity = sf_mapping::DirectMappingRowIdentity::RequirePrimaryKey;
    let base_iri_owned = base_iri.to_owned();
    let generation_span = telemetry::stage_span(Stage::GenerationBuild);
    let generated = candidate_work::run(budget, move || {
        generation_span.in_scope(|| {
            let mapping = sf_mapping::direct_mapping_for_source_with_row_identity(
                &tables,
                &base_iri_owned,
                source_id,
                row_identity,
            )?;
            let generation = Arc::new(PostgresDirectGeneration {
                source_id,
                base_iri: Arc::from(base_iri_owned),
                row_identity,
                mapping_digest: MappingDigest::from_mapping(&mapping),
                identity,
                session,
                tables: tables.clone().into(),
            });
            Ok::<_, sf_core::Error>((tables, mapping, generation))
        })
    })
    .await;
    let (tables, mapping, generation) = match generated {
        Err(error) => return observed.reject(error).await,
        Ok(Err(error)) => return observed.reject(PgGenerationError::Mapping(error)).await,
        Ok(Ok(generated)) => generated,
    };
    let (lease, observation) = observed.promote_candidate(Arc::clone(&generation)).await?;
    lease.finish_bounded(budget).await?;
    Ok(PostgresDirectCandidate {
        mapping,
        source: PostgresDirectSourceCandidate {
            tables,
            observation: SourceSchemaObservationV1::postgres16_public(observation),
            generation: SourceGeneration::direct_postgres(generation),
        },
    })
}

async fn acquire_expected(
    requirement: PgGenerationRequirement,
    budget: &RequestBudget,
) -> Result<VerifiedPostgresGenerationLease, PgGenerationError> {
    let expected = requirement.expected;
    let table_names = normalized_table_names(&expected.tables)?;
    open_observed_generation(&requirement.pool, expected.source_id, &table_names, budget)
        .await?
        .promote_expected(expected)
        .await
}

fn normalized_table_names(tables: &[TableSchema]) -> Result<Vec<String>, PgGenerationError> {
    let mut names = tables
        .iter()
        .map(|table| table.name.clone())
        .collect::<Vec<_>>();
    names.sort();
    if names.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(PgGenerationError::CapabilityDrift);
    }
    Ok(names)
}

/// The current SQL IR still represents the conformance-only physical row id as
/// the string `rowid`. Until that becomes a typed marker, exclude every spelling
/// that could be confused with it from the closed live PostgreSQL profile.
fn postgres_direct_table_profile_is_unambiguous(tables: &[TableSchema]) -> bool {
    tables.iter().all(|table| {
        table
            .columns
            .iter()
            .all(|column| !column.name.eq_ignore_ascii_case("rowid"))
    })
}

async fn close_leases(leases: BTreeMap<SourceId, RuntimeBoundGenerationLease>) {
    let _ = finish_leases(leases).await;
}

async fn finish_leases(
    leases: BTreeMap<SourceId, RuntimeBoundGenerationLease>,
) -> Result<(), PgGenerationError> {
    let mut first_error = None;
    for (_, (_, lease)) in leases {
        let result = lease.rollback_bounded().await;
        if first_error.is_none() {
            first_error = result.err();
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
#[path = "pg_generation/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "pg_generation/live_tests.rs"]
mod live_tests;
