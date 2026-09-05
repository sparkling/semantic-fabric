//! Immutable per-server configuration and governance defaults.

use std::sync::Arc;
use std::time::Duration;

use sf_core::query_control::QueryLimits;
use sf_core::{ir::TriplesMap, SourceId, SourceMapping};
use sf_sparql::Tbox;
use sf_sql::TableSchema;
use tokio::sync::Semaphore;

use crate::binding::{BoundPlan, ExecutablePlan, IntrospectedSource, RuntimeBinding};
use crate::problem::StartupCause;
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

/// The immutable server configuration shared (in an `Arc`) across all requests.
/// Semantic/compiler/backend state is private and inseparable inside one
/// [`RuntimeBinding`]; only request-governance knobs remain independently
/// configurable.
pub struct ServeConfig {
    binding: RuntimeBinding,
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
        let max_form_body_len = checked_form_body_len(DEFAULT_MAX_QUERY_LEN)
            .expect("default query length has a representable form-body limit");
        Self {
            binding: RuntimeBinding::new(source, mapping, tbox),
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

    pub(crate) fn compile(
        &self,
        query: &str,
        control: &dyn sf_core::query_control::QueryControl,
    ) -> sf_sparql::Result<BoundPlan> {
        self.binding.compile(query, control)
    }

    pub(crate) fn prepare_execution(
        &self,
        plan: BoundPlan,
    ) -> Result<ExecutablePlan, crate::binding::BindingMismatch> {
        self.binding.prepare_execution(plan)
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
}
