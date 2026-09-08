//! Multi-origin recipes remain inseparable from the ordinary binding/security owner.
use super::*;
use crate::budget::RequestBudget;
use sf_core::security_context::PolicySnapshotId;

impl RuntimeBinding {
    pub(crate) fn compile_lineage(
        &self,
        query: &str,
        budget: &RequestBudget,
        policy: Option<PolicySnapshotId>,
    ) -> sf_sparql::Result<BoundPlan> {
        let security = budget.security_context();
        if policy.is_some_and(|policy| {
            security.is_none_or(|context| !context.matches_policy_snapshot(policy))
        }) {
            return Err(sf_sparql::Error::Mapping(
                "security partition mismatch".into(),
            ));
        }
        budget.checkpoint()?;
        let (plan, spec) = self.compiler.compile_lineage(query, budget)?;
        Ok(BoundPlan {
            security,
            binding_identity: self.binding_identity.clone(),
            scope: self.scope(),
            source_id: self.source_id(),
            plan: Arc::new(plan),
            lineage: Some(Arc::new(spec)),
        })
    }
}
impl ExecutablePlan {
    pub(crate) fn lineage(&self) -> Option<Arc<sf_sparql::lineage::LineageSpec>> {
        self.lineage.clone()
    }
}

impl crate::snapshot::RuntimeSnapshot {
    pub(crate) fn compile_federated_lineage(
        &self,
        sources: [SourceId; 2],
        query: &str,
        budget: &RequestBudget,
        policy: Option<PolicySnapshotId>,
    ) -> sf_sparql::Result<BoundFederatedPlan> {
        let security = budget.security_context();
        if policy.is_some_and(|p| security.is_none_or(|c| !c.matches_policy_snapshot(p))) {
            return Err(sf_sparql::Error::Mapping(
                "security partition mismatch".into(),
            ));
        }
        let left = self
            .registry()
            .binding(sources[0])
            .ok_or_else(|| sf_sparql::Error::Mapping("lineage source missing".into()))?;
        let right = self
            .registry()
            .binding(sources[1])
            .ok_or_else(|| sf_sparql::Error::Mapping("lineage source missing".into()))?;
        let plan = sf_sparql::federation::compile_source_affine_union_lineage(
            query,
            [left.compiler(), right.compiler()],
            budget,
        )?;
        let mut bound = BoundFederatedPlan::new(plan, [left, right]);
        bound.security = security;
        Ok(bound)
    }
}
