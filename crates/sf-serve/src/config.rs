//! Immutable per-server configuration and governance defaults.

use std::sync::Arc;
use std::time::Duration;

use sf_core::query_control::QueryLimits;
use sf_core::{ir::TriplesMap, SourceId, SourceMapping};
use sf_sparql::{Epoch, Tbox};
use sf_sql::TableSchema;
use tokio::sync::Semaphore;

use crate::activation::{
    ActivationError, ActivationId, ReadinessCause, RuntimeManager, RuntimeReadiness,
    RuntimeSnapshotLease, SnapshotUnavailable,
};
use crate::binding::IntrospectedSource;
use crate::problem::StartupCause;
use crate::snapshot::{RuntimeSnapshot, RuntimeSource, SnapshotError};
use crate::{Backend, ServeError};

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
    fn source_ids(self) -> [Option<SourceId>; 2] {
        match self {
            Self::Single(source_id) => [Some(source_id), None],
            Self::SourceAffineUnion(source_ids) => source_ids.map(Some),
        }
    }
}

/// The immutable server configuration shared (in an `Arc`) across all requests.
/// Semantic/compiler/backend state is private and inseparable inside one
/// immutable [`RuntimeSnapshot`]. The current serving API selects its sole
/// registered source; only request-governance knobs remain configurable.
pub struct ServeConfig {
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
}

impl ServeConfig {
    /// Build a source-bound config with the default governance knobs.
    pub fn new(source: IntrospectedSource, mapping: SourceMapping, tbox: Tbox) -> Self {
        let source_id = mapping.source_id();
        let snapshot =
            RuntimeSnapshot::single(Epoch::default(), tbox, RuntimeSource::new(source, mapping));
        Self::from_snapshot(QueryMode::Single(source_id), snapshot)
    }

    /// Build the bounded two-source serving profile. This mode accepts only the
    /// source-affine top-level SELECT UNION vertical; it is not broad federation.
    pub fn new_federated(sources: [RuntimeSource; 2], tbox: Tbox) -> Result<Self, SnapshotError> {
        let source_ids = [sources[0].source_id(), sources[1].source_id()];
        if source_ids[0] == source_ids[1] {
            return Err(SnapshotError::DuplicateSource {
                source_id: source_ids[0],
            });
        }
        let snapshot = RuntimeSnapshot::new(Epoch::default(), tbox, Vec::from(sources))?;
        Ok(Self::from_snapshot(
            QueryMode::SourceAffineUnion(source_ids),
            snapshot,
        ))
    }

    fn from_snapshot(query_mode: QueryMode, snapshot: RuntimeSnapshot) -> Self {
        let max_form_body_len = checked_form_body_len(DEFAULT_MAX_QUERY_LEN)
            .expect("default query length has a representable form-body limit");
        Self {
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
        }
    }

    /// Compatibility/test constructor for a caller that cannot yet provide an
    /// observed backend/schema pair and source-aware mapping explicitly.
    ///
    /// The name keeps the missing provenance visible. Product startup does not
    /// use this path.
    pub fn new_unchecked(
        backend: Backend,
        mapping: Vec<TriplesMap>,
        tbox: Tbox,
        schema: Vec<TableSchema>,
    ) -> Self {
        let source_id = SourceId::new(0).expect("single-source slot zero is representable");
        Self::new(
            IntrospectedSource::unchecked(backend, schema),
            SourceMapping::new(source_id, mapping),
            tbox,
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

    /// Current redacted readiness and activation identity.
    pub fn runtime_readiness(&self) -> Result<RuntimeReadiness, ActivationError> {
        self.runtime.readiness()
    }

    /// Atomically publish a prebuilt candidate that still contains the source
    /// selected by this serving configuration. This private primitive does not
    /// validate or authorize the candidate and stays sealed until the complete
    /// candidate builder exists.
    #[allow(
        dead_code,
        reason = "activation stays sealed until the validated candidate builder lands"
    )]
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
        self.runtime.activate(expected, candidate)
    }

    /// Reject new requests for the current generation while preserving every
    /// lease already in flight.
    pub fn mark_runtime_not_ready(
        &self,
        expected: ActivationId,
        cause: ReadinessCause,
    ) -> Result<(), ActivationError> {
        self.runtime.mark_not_ready(expected, cause)
    }

    pub(crate) fn runtime_lease(&self) -> Result<RuntimeSnapshotLease, SnapshotUnavailable> {
        self.runtime.lease()
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
mod tests {
    use super::*;

    fn config() -> ServeConfig {
        ServeConfig::new_unchecked(
            Backend::sqlite(rusqlite::Connection::open_in_memory().expect("open fixture")),
            Vec::new(),
            Tbox::default(),
            Vec::new(),
        )
    }

    #[test]
    fn request_admission_defaults_to_one_shared_finite_gate() {
        let config = config();
        assert_eq!(
            config.max_concurrent_requests(),
            DEFAULT_MAX_CONCURRENT_REQUESTS
        );
        assert_eq!(
            config.available_request_permits(),
            DEFAULT_MAX_CONCURRENT_REQUESTS
        );
        assert_eq!(config.max_order_rows(), DEFAULT_MAX_ORDER_ROWS);
        assert_eq!(
            config.query_limits.max_retained_bytes(),
            DEFAULT_MAX_ORDER_BYTES
        );
    }

    #[test]
    fn request_admission_setter_is_checked_and_preserves_state_on_error() {
        let mut config = config();
        config
            .set_max_concurrent_requests(3)
            .expect("finite request ceiling");
        let configured_gate = config.request_admission_permits();
        assert_eq!(config.available_request_permits(), 3);

        for invalid in [0, Semaphore::MAX_PERMITS + 1] {
            let error = config
                .set_max_concurrent_requests(invalid)
                .expect_err("invalid request ceiling");
            assert_eq!(error.code(), "startup-configuration");
            assert!(matches!(
                error.internal_cause(),
                StartupCause::Configuration { .. }
            ));
            assert_eq!(config.max_concurrent_requests(), 3);
            assert!(Arc::ptr_eq(
                &configured_gate,
                &config.request_admission_permits()
            ));
        }
    }

    #[test]
    fn activation_rejects_a_candidate_without_the_selected_source() {
        let config = config();
        let current = config.runtime_readiness().unwrap();
        let selected_source = SourceId::new(0).unwrap();
        let other_source = SourceId::new(1).unwrap();
        let candidate = RuntimeSnapshot::single(
            Epoch(1),
            Tbox::default(),
            RuntimeSource::new(
                IntrospectedSource::unchecked(
                    Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
                    Vec::new(),
                ),
                SourceMapping::new(other_source, Vec::new()),
            ),
        );

        assert!(matches!(
            config.activate_snapshot(current, candidate),
            Err(ActivationError::CandidateMissingSource { source_id })
                if source_id == selected_source
        ));
        assert_eq!(config.runtime_readiness().unwrap(), current);
    }
}
