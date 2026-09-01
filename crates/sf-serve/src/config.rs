//! Immutable per-server configuration and governance defaults.

use std::sync::Arc;
use std::time::Duration;

use sf_core::{ir::TriplesMap, SourceId, SourceMapping};
use sf_sparql::Tbox;
use sf_sql::TableSchema;
use tokio::sync::Semaphore;

use crate::binding::{BoundPlan, ExecutablePlan, IntrospectedSource, RuntimeBinding};
use crate::Backend;

/// Default request timeout and max query length when constructed via [`ServeConfig::new`].
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_MAX_QUERY_LEN: usize = 1 << 20; // 1 MiB

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
    pub max_query_len: usize,
    /// Bounds active `spawn_blocking` compilers. An owned permit lives inside the
    /// blocking closure, including after its request waiter times out.
    compiler_permits: Arc<Semaphore>,
}

impl ServeConfig {
    /// Build a source-bound config with the default governance knobs.
    pub fn new(source: IntrospectedSource, mapping: SourceMapping, tbox: Tbox) -> Self {
        Self {
            binding: RuntimeBinding::new(source, mapping, tbox),
            timeout: DEFAULT_TIMEOUT,
            max_query_len: DEFAULT_MAX_QUERY_LEN,
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
