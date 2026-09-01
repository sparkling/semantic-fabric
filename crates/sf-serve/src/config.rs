//! Immutable per-server configuration and governance defaults.

use std::sync::Arc;
use std::time::Duration;

use sf_core::ir::TriplesMap;
use sf_sparql::{Epoch, Plan, PlanCache, Tbox};
use sf_sql::TableSchema;
use tokio::sync::Semaphore;

use crate::Backend;

/// Plan-cache capacity (ADR-0007 *Plan cache, hot path*). 64 entries covers a
/// diverse serve-mode workload without over-committing memory; the cache is sized
/// by `⟨T, M⟩` (never by data), so it cannot go stale vs a live source.
const PLAN_CACHE_CAP: usize = 64;

/// Default request timeout and max query length when constructed via [`ServeConfig::new`].
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_MAX_QUERY_LEN: usize = 1 << 20; // 1 MiB

/// Blocking query compilers admitted per server instance. Queue time consumes the
/// same request deadline; this is a partial-M2 capacity bound, not a work budget.
const DEFAULT_COMPILER_PERMITS: usize = 4;

/// The immutable server configuration shared (in an `Arc`) across all requests:
/// the parsed mapping `M`, the tier-1 T-Box `T`, the introspected source schema,
/// the backend, the ADR-0010 governance knobs, and the plan-compile cache.
pub struct ServeConfig {
    pub mapping: Vec<TriplesMap>,
    pub tbox: Tbox,
    pub schema: Vec<TableSchema>,
    pub backend: Backend,
    pub timeout: Duration,
    pub max_query_len: usize,
    /// Compiled-plan cache (ADR-0007): repeated queries at the same `⟨T, M⟩` +
    /// schema epoch reuse their plan without recompilation.
    plan_cache: PlanCache<Plan>,
    /// Monotonic epoch invalidated by ontology/mapping/schema reloads.
    epoch: Epoch,
    /// Bounds active `spawn_blocking` compilers. An owned permit lives inside the
    /// blocking closure, including after its request waiter times out.
    compiler_permits: Arc<Semaphore>,
}

impl ServeConfig {
    /// Build a config with the default governance knobs (30 s timeout, 1 MiB cap).
    pub fn new(
        backend: Backend,
        mapping: Vec<TriplesMap>,
        tbox: Tbox,
        schema: Vec<TableSchema>,
    ) -> Self {
        Self {
            mapping,
            tbox,
            schema,
            backend,
            timeout: DEFAULT_TIMEOUT,
            max_query_len: DEFAULT_MAX_QUERY_LEN,
            plan_cache: PlanCache::new(PLAN_CACHE_CAP),
            epoch: Epoch::default(),
            compiler_permits: Arc::new(Semaphore::new(DEFAULT_COMPILER_PERMITS)),
        }
    }

    pub(crate) fn plan_cache(&self) -> &PlanCache<Plan> {
        &self.plan_cache
    }

    pub(crate) fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub(crate) fn compiler_permits(&self) -> Arc<Semaphore> {
        self.compiler_permits.clone()
    }
}
