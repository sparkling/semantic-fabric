//! Request-context checks at the immutable runtime lease boundary.
use super::*;
use crate::budget::RequestBudget;
use sf_core::security_context::PolicySnapshotId;

impl RuntimeSnapshotLease {
    pub(crate) fn permits_rls(&self, source: SourceId) -> bool {
        self.snapshot.permits_rls(source)
    }
    pub(crate) fn compile_secured(
        &self,
        source: SourceId,
        query: &str,
        budget: &RequestBudget,
        policy: PolicySnapshotId,
    ) -> sf_sparql::Result<BoundPlan> {
        self.snapshot.compile_secured(source, query, budget, policy)
    }

    pub(crate) fn compile_federated_secured(
        &self,
        sources: [SourceId; 2],
        query: &str,
        budget: &RequestBudget,
        policy: PolicySnapshotId,
    ) -> sf_sparql::Result<BoundFederatedPlan> {
        self.snapshot
            .compile_federated_secured(sources, query, budget, policy)
    }

    pub(crate) fn prepare_request_execution(
        &self,
        plan: BoundPlan,
        budget: &RequestBudget,
    ) -> Result<ExecutablePlan, BindingMismatch> {
        if plan.security != budget.security_context() {
            return Err(BindingMismatch);
        }
        self.prepare_execution(plan)
    }

    pub(crate) fn prepare_federated_request_execution(
        &self,
        plan: BoundFederatedPlan,
        budget: &RequestBudget,
    ) -> Result<ExecutableFederatedPlan, BindingMismatch> {
        if plan.security != budget.security_context() {
            return Err(BindingMismatch);
        }
        self.prepare_federated_execution(plan)
    }
}
