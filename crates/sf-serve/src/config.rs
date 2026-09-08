//! Immutable per-server configuration and governance defaults.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use sf_core::query_control::QueryLimits;
use sf_core::SourceId;
#[cfg(test)]
use sf_core::{ir::TriplesMap, SourceMapping};
use sf_sparql::Epoch;
#[cfg(test)]
use sf_sql::TableSchema;
use tokio::sync::{watch, Semaphore};

#[cfg(test)]
use crate::activation::ActivationId;
use crate::activation::{
    ActivationError, ReadinessCause, RuntimeManager, RuntimeReadiness, RuntimeSnapshotLease,
    SnapshotUnavailable,
};
use crate::budget::RequestBudget;
use crate::lifecycle::ShutdownPhase;
use crate::problem::StartupCause;
use crate::semantic_admission::{MappingOrigin, ValidatedMapping};
use crate::snapshot::{RuntimeSnapshot, RuntimeSource, SnapshotError};
use crate::telemetry::CorrelationId;
#[cfg(test)]
use crate::Backend;
use crate::{IntrospectedSource, SemanticOntology, ServeError};

mod parser;
use parser::ParserSetup;

/// Worst-case wire bytes for the percent-encoded `query` key plus `=`.
const FORM_QUERY_FIELD_OVERHEAD: usize = 16;

/// Default request timeout and max query length when constructed via [`ServeConfig::new`].
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_MAX_QUERY_LEN: usize = 1 << 20; // 1 MiB
/// Textual binding payload retained by an exact global ORDER operation.
pub const DEFAULT_MAX_ORDER_BYTES: u64 = 64 * 1024 * 1024;
/// Finite serve defaults; CLI help and programmatic construction share these values.
pub const DEFAULT_QUERY_LIMITS: QueryLimits =
    QueryLimits::new(1_000_000, 1_000_000, 100_000, 64 * 1024 * 1024)
        .with_max_retained_bytes(DEFAULT_MAX_ORDER_BYTES);
/// Maximum exact in-process ORDER BY window (`OFFSET + LIMIT`) admitted by default.
pub const DEFAULT_MAX_ORDER_ROWS: usize = 100_000;
/// Conservative finite governance default for admitted requests. This value is
/// not a throughput target or a load-test result.
pub const DEFAULT_MAX_CONCURRENT_REQUESTS: usize = 64;

/// Blocking query compilers admitted per server instance. Queue time consumes the
/// same request deadline; this is a partial-M2 capacity bound, not a work budget.
const DEFAULT_COMPILER_PERMITS: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QueryMode {
    Single(SourceId),
    SourceAffineUnion([SourceId; 2]),
}

impl QueryMode {
    pub(crate) fn source_ids(self) -> [Option<SourceId>; 2] {
        match self {
            Self::Single(source_id) => [Some(source_id), None],
            Self::SourceAffineUnion(source_ids) => source_ids.map(Some),
        }
    }
}

/// The immutable server configuration shared (in an `Arc`) across all requests.
/// Semantic/compiler/backend state is private and inseparable inside one
/// immutable [`RuntimeSnapshot`]. The serving API selects either one registered
/// source or the sealed bounded two-source UNION/join profiles; request-governance
/// knobs remain configurable.
pub struct ServeConfig {
    parser: ParserSetup,
    pub(crate) query_admission: crate::QueryAdmission,
    runtime: Arc<RuntimeManager>,
    query_mode: QueryMode,
    pub timeout: Duration,
    max_query_len: usize,
    max_form_body_len: usize,
    /// Inclusive request-wide compiler/source/result/serialization ceilings.
    pub query_limits: QueryLimits,
    max_order_rows: usize,
    /// Bounds active `spawn_blocking` compilers. An owned permit lives inside the
    /// blocking closure, including after its request waiter times out.
    compiler_permits: Arc<Semaphore>,
    max_concurrent_requests: usize,
    request_admission_permits: Arc<Semaphore>,
    shutdown: watch::Sender<ShutdownPhase>,
    pg_direct_lifecycle_claimed: AtomicBool,
}

impl ServeConfig {
    /// Build a source-bound config with the default governance knobs.
    #[cfg(test)]
    pub(crate) fn new(
        source: IntrospectedSource,
        mapping: SourceMapping,
        ontology: SemanticOntology,
    ) -> Result<Self, SnapshotError> {
        let source_id = mapping.source_id();
        let snapshot = RuntimeSnapshot::single(
            Epoch::default(),
            ontology,
            RuntimeSource::new(source, mapping),
        )?;
        let mut cfg = Self::from_snapshot(QueryMode::Single(source_id), snapshot);
        cfg.use_in_process_test_parser();
        cfg.set_query_admission(crate::QueryAdmission::UnrestrictedDevelopment);
        Ok(cfg)
    }

    /// Build the bounded two-source serving profile. This mode accepts only the
    /// source-affine SELECT UNION and bounded inner join; not broad federation.
    #[cfg(test)]
    pub(crate) fn new_federated(
        sources: [RuntimeSource; 2],
        ontology: SemanticOntology,
    ) -> Result<Self, SnapshotError> {
        let source_ids = [sources[0].source_id(), sources[1].source_id()];
        if source_ids[0] == source_ids[1] {
            return Err(SnapshotError::DuplicateSource {
                source_id: source_ids[0],
            });
        }
        let snapshot = RuntimeSnapshot::new(Epoch::default(), ontology, Vec::from(sources))?;
        let mut cfg = Self::from_snapshot(QueryMode::SourceAffineUnion(source_ids), snapshot);
        cfg.use_in_process_test_parser();
        Ok(cfg)
    }

    pub(crate) fn from_runtime_source(
        source: RuntimeSource,
        ontology: SemanticOntology,
    ) -> Result<Self, SnapshotError> {
        let source_id = source.source_id();
        let snapshot = RuntimeSnapshot::single(Epoch::default(), ontology, source)?;
        Ok(Self::from_snapshot(QueryMode::Single(source_id), snapshot))
    }

    /// Consume the sealed initial output of the dormant PostgreSQL Direct
    /// lifecycle. This is initial construction only; runtime publication still
    /// requires the validated-candidate and transition-authority pair.
    #[allow(
        dead_code,
        reason = "the sealed profile remains disconnected pending independent admission review"
    )]
    pub(crate) fn from_initial_pg_direct(
        initial: crate::pg_direct_lifecycle::InitialPgDirectGeneration,
    ) -> (Self, crate::pg_generation::PostgresDirectExpectation) {
        let (snapshot, expectation) = initial.into_parts();
        let source_id = SourceId::new(0).expect("the closed profile uses source slot zero");
        (
            Self::from_snapshot(QueryMode::Single(source_id), snapshot),
            expectation,
        )
    }

    pub(crate) fn from_snapshot(query_mode: QueryMode, snapshot: RuntimeSnapshot) -> Self {
        let max_form_body_len = checked_form_body_len(DEFAULT_MAX_QUERY_LEN)
            .expect("default query length has a representable form-body limit");
        let (shutdown, _) = watch::channel(ShutdownPhase::Running);
        Self {
            parser: ParserSetup::Missing,
            query_admission: crate::QueryAdmission::Deny,
            runtime: Arc::new(RuntimeManager::new(snapshot)),
            query_mode,
            timeout: DEFAULT_TIMEOUT,
            max_query_len: DEFAULT_MAX_QUERY_LEN,
            max_form_body_len,
            query_limits: DEFAULT_QUERY_LIMITS,
            max_order_rows: DEFAULT_MAX_ORDER_ROWS,
            compiler_permits: Arc::new(Semaphore::new(DEFAULT_COMPILER_PERMITS)),
            max_concurrent_requests: DEFAULT_MAX_CONCURRENT_REQUESTS,
            request_admission_permits: Arc::new(Semaphore::new(DEFAULT_MAX_CONCURRENT_REQUESTS)),
            shutdown,
            pg_direct_lifecycle_claimed: AtomicBool::new(false),
        }
    }

    /// Build a single-source embedding from authored R2RML and an opaque source
    /// observation. Generated Direct-Mapping IR cannot enter this API, and the
    /// source type has no public detached-schema constructor.
    pub fn from_authored_r2rml(
        source: IntrospectedSource,
        mapping_turtle: &str,
        ontology: SemanticOntology,
    ) -> Result<Self, ServeError> {
        if mapping_turtle.len() > sf_validation::DEFAULT_GRAPH_LIMITS.max_utf8_bytes {
            return Err(ServeError::new(StartupCause::MappingParse {
                error: "mapping document exceeds its byte limit".to_owned(),
            }));
        }
        let source_id = SourceId::new(0).expect("single-source slot zero is representable");
        let mapping =
            sf_mapping::parse_r2rml_for_source(mapping_turtle, source_id).map_err(|_| {
                ServeError::new(StartupCause::MappingParse {
                    error: "mapping document is invalid".to_owned(),
                })
            })?;
        ValidatedMapping::preflight(&mapping, &ontology).map_err(|error| {
            ServeError::new(StartupCause::Configuration {
                error: error.to_string(),
            })
        })?;
        let mapping =
            ValidatedMapping::validate(mapping, MappingOrigin::Authored, &ontology, &source)
                .map_err(|error| {
                    ServeError::new(StartupCause::Configuration {
                        error: error.to_string(),
                    })
                })?;
        let source = RuntimeSource::admitted(source, mapping).map_err(|error| {
            ServeError::new(StartupCause::Configuration {
                error: error.to_string(),
            })
        })?;
        Self::from_runtime_source(source, ontology).map_err(|error| {
            ServeError::new(StartupCause::Configuration {
                error: error.to_string(),
            })
        })
    }

    /// Select access policy before sharing this config. Changes require a new
    /// server; this service-lifetime policy stays pinned across runtime leases.
    pub fn set_query_admission(&mut self, admission: crate::QueryAdmission) {
        self.query_admission = admission;
    }

    /// Unit-test construction over an explicitly fabricated observation.
    #[cfg(test)]
    pub(crate) fn new_with_unverified_source(
        backend: Backend,
        mapping: Vec<TriplesMap>,
        ontology: SemanticOntology,
        schema: Vec<TableSchema>,
    ) -> Result<Self, SnapshotError> {
        let source_id = SourceId::new(0).expect("single-source slot zero is representable");
        Self::new(
            IntrospectedSource::unchecked(backend, schema),
            SourceMapping::new(source_id, mapping),
            ontology,
        )
    }

    /// Set the decoded query limit after proving that the corresponding
    /// worst-case urlencoded request-body limit is representable.
    pub fn set_max_query_len(&mut self, max_query_len: usize) -> Result<(), ServeError> {
        let max_form_body_len = validate_max_query_len(max_query_len)?;
        self.max_query_len = max_query_len;
        self.max_form_body_len = max_form_body_len;
        Ok(())
    }

    /// Maximum admitted decoded query length in bytes.
    pub fn max_query_len(&self) -> usize {
        self.max_query_len
    }

    /// Replace the unopened server-wide request-admission gate with `maximum`
    /// permits after validating Tokio's semaphore boundary.
    pub fn set_max_concurrent_requests(&mut self, maximum: usize) -> Result<(), ServeError> {
        validate_max_concurrent_requests(maximum)?;
        self.max_concurrent_requests = maximum;
        self.request_admission_permits = Arc::new(Semaphore::new(maximum));
        Ok(())
    }

    /// Configured server-wide request-admission ceiling.
    pub fn max_concurrent_requests(&self) -> usize {
        self.max_concurrent_requests
    }

    /// Set the independent exact ORDER BY retained-row ceiling. Zero disables
    /// every non-empty ordered window while still admitting `LIMIT 0`.
    pub fn set_max_order_rows(&mut self, maximum: usize) {
        self.max_order_rows = maximum;
    }

    /// Maximum exact ORDER BY window admitted for in-process retention.
    pub fn max_order_rows(&self) -> usize {
        self.max_order_rows
    }

    pub(crate) fn max_form_body_len(&self) -> usize {
        self.max_form_body_len
    }

    /// Current redacted readiness, generation identity, and opaque state revision.
    pub fn runtime_readiness(&self) -> Result<RuntimeReadiness, ActivationError> {
        let readiness = self.runtime.readiness()?;
        if matches!(self.parser, ParserSetup::Missing) {
            return Ok(RuntimeReadiness::NotReady {
                activation_id: readiness.activation_id(),
                revision: readiness.state_revision(),
                cause: ReadinessCause::Administrative,
            });
        }
        Ok(readiness)
    }

    /// Warning-level findings admitted for `source_id` in the active semantic
    /// generation. Returns `None` when the source or a ready generation is absent.
    pub fn semantic_warning_count(&self, source_id: SourceId) -> Option<usize> {
        self.runtime.semantic_warning_count(source_id)
    }

    /// Raw snapshots can exercise activation invariants only in crate tests.
    #[cfg(test)]
    pub(crate) fn activate_snapshot(
        &self,
        expected: RuntimeReadiness,
        candidate: RuntimeSnapshot,
    ) -> Result<ActivationId, ActivationError> {
        for source_id in self.query_mode.source_ids().into_iter().flatten() {
            if !candidate.registry().contains_source(source_id) {
                return Err(ActivationError::CandidateMissingSource { source_id });
            }
        }
        self.ensure_runtime_transitions_open()?;
        let result = self.runtime.activate_candidate(
            &crate::pg_direct_lifecycle::RuntimeTransitionAuthority::for_test(),
            crate::pg_direct_lifecycle::sealed_candidate_for_test(expected, candidate),
        );
        self.normalize_shutdown_race(result)
    }

    /// Reject new requests only if the complete expected state is current,
    /// while preserving every lease already in flight.
    #[cfg(test)]
    pub(crate) fn mark_runtime_not_ready(
        &self,
        expected: RuntimeReadiness,
        cause: ReadinessCause,
    ) -> Result<RuntimeReadiness, ActivationError> {
        self.ensure_runtime_transitions_open()?;
        let result = self.runtime.transition_not_ready(
            &crate::pg_direct_lifecycle::RuntimeTransitionAuthority::for_test(),
            expected,
            cause,
        );
        self.normalize_shutdown_race(result)
    }

    #[cfg(test)]
    fn ensure_runtime_transitions_open(&self) -> Result<(), ActivationError> {
        if *self.shutdown.borrow() == ShutdownPhase::Running {
            Ok(())
        } else {
            Err(ActivationError::ShuttingDown)
        }
    }

    #[cfg(test)]
    fn normalize_shutdown_race<T>(
        &self,
        result: Result<T, ActivationError>,
    ) -> Result<T, ActivationError> {
        match result {
            Err(ActivationError::StaleState { .. })
                if *self.shutdown.borrow() != ShutdownPhase::Running =>
            {
                Err(ActivationError::ShuttingDown)
            }
            result => result,
        }
    }

    pub(crate) fn runtime_lease(&self) -> Result<RuntimeSnapshotLease, SnapshotUnavailable> {
        self.runtime.lease()
    }

    pub(crate) fn lifecycle_runtime(&self) -> Arc<RuntimeManager> {
        Arc::clone(&self.runtime)
    }

    pub(crate) fn claim_pg_direct_lifecycle(&self) -> Result<(), ReadinessCause> {
        self.pg_direct_lifecycle_claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| ReadinessCause::CapabilityDrift)
    }

    pub(crate) const fn query_mode(&self) -> QueryMode {
        self.query_mode
    }

    pub(crate) fn compiler_permits(&self) -> Arc<Semaphore> {
        self.compiler_permits.clone()
    }

    pub(crate) fn request_admission_permits(&self) -> Arc<Semaphore> {
        self.request_admission_permits.clone()
    }

    pub(crate) fn request_budget_for(&self, correlation: CorrelationId) -> RequestBudget {
        let budget = RequestBudget::after_with_shutdown(
            self.timeout,
            self.query_limits,
            self.shutdown.subscribe(),
            correlation,
        );
        if *self.shutdown.borrow() != ShutdownPhase::Running {
            budget.cancel();
        }
        budget
    }

    #[cfg(test)]
    pub(crate) fn request_budget(&self) -> RequestBudget {
        self.request_budget_for(CorrelationId::generate())
    }

    pub(crate) fn begin_shutdown(&self) {
        let began = self.shutdown.send_if_modified(|phase| {
            if *phase == ShutdownPhase::Running {
                *phase = ShutdownPhase::Draining;
                true
            } else {
                false
            }
        });
        if began {
            let _ = self.runtime.mark_current_administratively_not_ready();
        }
    }

    /// Cancel identities that did not complete within the graceful drain bound.
    pub(crate) fn force_shutdown(&self) {
        self.shutdown.send_if_modified(|phase| {
            if *phase != ShutdownPhase::Forced {
                *phase = ShutdownPhase::Forced;
                true
            } else {
                false
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn available_request_permits(&self) -> usize {
        self.request_admission_permits.available_permits()
    }
}

pub(crate) fn validate_max_concurrent_requests(maximum: usize) -> Result<(), ServeError> {
    if maximum == 0 || maximum > Semaphore::MAX_PERMITS {
        return Err(ServeError::new(StartupCause::Configuration {
            error: format!(
                "max concurrent requests must be between 1 and {}",
                Semaphore::MAX_PERMITS
            ),
        }));
    }
    Ok(())
}

pub(crate) fn validate_max_query_len(max_query_len: usize) -> Result<usize, ServeError> {
    checked_form_body_len(max_query_len).ok_or_else(|| {
        ServeError::new(StartupCause::Configuration {
            error: "max query length cannot be represented as a form-body limit".to_owned(),
        })
    })
}

fn checked_form_body_len(max_query_len: usize) -> Option<usize> {
    max_query_len
        .checked_mul(3)
        .and_then(|encoded| encoded.checked_add(FORM_QUERY_FIELD_OVERHEAD))
}

#[cfg(test)]
#[path = "config/tests.rs"]
mod tests;
