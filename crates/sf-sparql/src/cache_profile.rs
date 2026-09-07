//! Physical cache partitions for compiler-governance profiles.

use super::{CompileProfileId, PlanCache};

/// Physically distinct capacity for raw and governed compiler profiles.
///
/// A profile key remains a second fail-closed check, but raw eviction and drop
/// work cannot enter the governed shard. The governed shard is dormant until a
/// separately admitted compile path exists. Capacities are explicit and their
/// aggregate entry bound is checked before either cache is constructed.
pub(super) struct ProfiledPlanCaches<P> {
    uncontrolled: PlanCache<P>,
    governed_v1: PlanCache<P>,
}

impl<P: Clone> ProfiledPlanCaches<P> {
    /// Preserve the existing binding capacity as raw-only until activation has
    /// its own calibrated governed capacity and admission authority.
    pub(super) fn uncontrolled_only(capacity: usize) -> Self {
        Self::with_capacities(capacity, 0)
    }

    pub(super) fn with_capacities(uncontrolled_capacity: usize, governed_capacity: usize) -> Self {
        let aggregate = Self::aggregate_entry_bound(uncontrolled_capacity, governed_capacity);
        assert!(aggregate > 0, "plan cache capacity must be non-zero");
        Self {
            uncontrolled: PlanCache::new(uncontrolled_capacity),
            governed_v1: PlanCache::new(governed_capacity),
        }
    }

    pub(super) fn aggregate_entry_bound(
        uncontrolled_capacity: usize,
        governed_capacity: usize,
    ) -> usize {
        uncontrolled_capacity
            .checked_add(governed_capacity)
            .expect("aggregate plan cache capacity overflow")
    }

    pub(super) fn for_profile(&self, profile: CompileProfileId) -> &PlanCache<P> {
        match profile {
            CompileProfileId::Uncontrolled => &self.uncontrolled,
            CompileProfileId::GovernedV1 => &self.governed_v1,
        }
    }
}
