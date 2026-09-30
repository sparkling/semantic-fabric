//! Source-only single-default dataset composition. No HTTP activation or grant.
use std::sync::Arc;

use sf_core::query_control::{QueryControl, QueryControlError};
use sf_core::security_context::PolicySnapshotId;
use sf_sparql::cache::generated::{
    ConstantCoverageError, DatasetAllowlistError, DatasetGraphAllowlist, GeneratedDatasetError,
};
use sf_sparql::cache::{SecurityCompileError, SecurityDatasetCompileError};
use sf_sparql::Plan;

use super::{admit_constant, GeneratedCompiled, RuntimeBinding};
use crate::budget::RequestBudget;
use crate::generated_profile_identity::PinnedGraphAllowlist;
use crate::semantic_admission::generated_admission_receipt::ReceiptMismatch;
use crate::semantic_admission::generated_mapping_coverage::{
    ConstantRole, Coverage, MappingCoverage,
};

/// Typed causes remain inspectable; Display/Debug never expose compiler details.
#[derive(thiserror::Error)]
pub(crate) enum GeneratedDatasetRuntimeError {
    #[error("generated dataset compilation failed")]
    Dataset(#[from] GeneratedDatasetError),
    #[error("generated dataset security compilation failed")]
    Security(#[from] SecurityCompileError),
    #[error(transparent)]
    Receipt(#[from] ReceiptMismatch),
    #[error(transparent)]
    Allowlist(#[from] DatasetAllowlistError),
    #[error("generated dataset admission requires a security context")]
    MissingSecurityContext,
    #[error("generated dataset admission security policy mismatch")]
    PolicyMismatch,
}

impl std::fmt::Debug for GeneratedDatasetRuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GeneratedDatasetRuntimeError({self})")
    }
}

impl From<SecurityDatasetCompileError> for GeneratedDatasetRuntimeError {
    fn from(error: SecurityDatasetCompileError) -> Self {
        match error {
            SecurityDatasetCompileError::Dataset(error) => Self::Dataset(error),
            SecurityDatasetCompileError::Security(error) => Self::Security(error),
        }
    }
}

fn stopped(cause: QueryControlError) -> GeneratedDatasetRuntimeError {
    GeneratedDatasetError::Compiler(sf_sparql::Error::from(cause)).into()
}

fn admit_graph(
    coverage: &MappingCoverage<'_>,
    control: &dyn QueryControl,
    graph: &str,
) -> Result<(), ConstantCoverageError> {
    match coverage.classify(ConstantRole::NamedGraph, graph, control) {
        Ok(Coverage::Covered) => Ok(()),
        Ok(Coverage::Uncovered | Coverage::Indeterminate) => Err(ConstantCoverageError::Uncovered),
        Err(cause) => Err(ConstantCoverageError::Control(cause)),
    }
}

impl RuntimeBinding {
    /// One immutable allowlist supplies both compile membership and identity.
    /// Sealed source/digest validation precedes parse, coverage and cache access.
    pub(crate) fn compile_generated_single_default(
        &self,
        sparql: &str,
        pinned: &PinnedGraphAllowlist,
        control: &dyn QueryControl,
    ) -> Result<GeneratedCompiled, GeneratedDatasetRuntimeError> {
        let coverage = self.generated.coverage(&self.compiler)?;
        control.checkpoint().map_err(stopped)?;
        let allowlist = DatasetGraphAllowlist::new(pinned.iter())?;
        let plan = self.compiler.compile_shared_with_single_default_dataset(
            sparql,
            &allowlist,
            control,
            |graph| admit_graph(&coverage, control, graph),
            |constant| admit_constant(&coverage, control, constant),
        )?;
        control.checkpoint().map_err(stopped)?;
        let receipt = self
            .generated
            .for_single_default_dataset(&self.compiler, pinned)?;
        let mut compiled = self.generated_pair(plan, None);
        compiled.identity = receipt.identity();
        Ok(compiled)
    }

    /// Uncached preflight returns only a plan, never a response identity or
    /// an executable BoundPlan. No dataset identity is minted along this path.
    pub(crate) fn preflight_generated_single_default(
        &self,
        sparql: &str,
        pinned: &PinnedGraphAllowlist,
        control: &dyn QueryControl,
    ) -> Result<Arc<Plan>, GeneratedDatasetRuntimeError> {
        let coverage = self.generated.coverage(&self.compiler)?;
        control.checkpoint().map_err(stopped)?;
        let allowlist = DatasetGraphAllowlist::new(pinned.iter())?;
        let plan = self
            .compiler
            .compile_uncached_shared_with_single_default_dataset(
                sparql,
                &allowlist,
                control,
                |graph| admit_graph(&coverage, control, graph),
                |constant| admit_constant(&coverage, control, constant),
            )?;
        control.checkpoint().map_err(stopped)?;
        Ok(plan)
    }

    /// Policy and context are checked before any control, parser or coverage
    /// work. The protected compiler rechecks callbacks even on cache hits.
    pub(crate) fn compile_generated_single_default_secured(
        &self,
        sparql: &str,
        pinned: &PinnedGraphAllowlist,
        budget: &RequestBudget,
        policy: PolicySnapshotId,
    ) -> Result<GeneratedCompiled, GeneratedDatasetRuntimeError> {
        let context = budget
            .security_context()
            .ok_or(GeneratedDatasetRuntimeError::MissingSecurityContext)?;
        if !context.matches_policy_snapshot(policy) {
            return Err(GeneratedDatasetRuntimeError::PolicyMismatch);
        }
        let coverage = self.generated.coverage(&self.compiler)?;
        budget.checkpoint().map_err(stopped)?;
        let allowlist = DatasetGraphAllowlist::new(pinned.iter())?;
        let plan = self
            .compiler
            .for_security_policy(policy, &self.security_cache)
            .compile_shared_with_single_default_dataset(
                &context,
                sparql,
                &allowlist,
                budget,
                |graph| admit_graph(&coverage, budget, graph),
                |constant| admit_constant(&coverage, budget, constant),
            )?;
        budget.checkpoint().map_err(stopped)?;
        let receipt = self
            .generated
            .for_single_default_dataset(&self.compiler, pinned)?;
        let mut compiled = self.generated_pair(plan, Some(context));
        compiled.identity = receipt.identity();
        Ok(compiled)
    }
}

#[cfg(test)]
#[path = "generated_dataset_runtime_security_tests.rs"]
mod security_tests;
#[cfg(test)]
#[path = "generated_dataset_runtime_tests.rs"]
mod tests;
