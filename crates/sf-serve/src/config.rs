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
/// Finite serve defaults; CLI help and programmatic construction share this value.
pub const DEFAULT_QUERY_LIMITS: QueryLimits =
    QueryLimits::new(1_000_000, 100_000, 64 * 1024 * 1024);

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
    /// Inclusive request-wide source/result/serialization ceilings.
    pub query_limits: QueryLimits,
    /// Bounds active `spawn_blocking` compilers. An owned permit lives inside the
    /// blocking closure, including after its request waiter times out.
    compiler_permits: Arc<Semaphore>,
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
            compiler_permits: Arc::new(Semaphore::new(DEFAULT_COMPILER_PERMITS)),
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

    pub(crate) fn max_form_body_len(&self) -> usize {
        self.max_form_body_len
    }

    pub(crate) fn compile(&self, query: &str) -> sf_sparql::Result<BoundPlan> {
        self.binding.compile(query)
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
