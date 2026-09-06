//! Physical cache partitions for compiler-governance profiles.

use super::{CompileProfileId, PlanCache};

/// Physically distinct capacity for raw and governed compiler profiles.
///
/// A profile key remains a second fail-closed check, but raw eviction and drop
/// work cannot enter the governed shard. The governed shard is dormant until a
/// separately admitted compile path exists.
pub(super) struct ProfiledPlanCaches<P> {
    uncontrolled: PlanCache<P>,
    governed_v1: PlanCache<P>,
}

impl<P: Clone> ProfiledPlanCaches<P> {
    pub(super) fn new(capacity_per_profile: usize) -> Self {
        Self {
            uncontrolled: PlanCache::new(capacity_per_profile),
            governed_v1: PlanCache::new(capacity_per_profile),
        }
    }

    pub(super) fn for_profile(&self, profile: CompileProfileId) -> &PlanCache<P> {
        match profile {
            CompileProfileId::Uncontrolled => &self.uncontrolled,
            CompileProfileId::GovernedV1 => &self.governed_v1,
        }
    }
}
