//! Single-default dataset admission over the existing security cache.
//! Authentication, mapping coverage and source execution remain caller-owned.

use std::sync::Arc;

use sf_core::query_control::QueryControl;
use sf_core::security_context::SecurityContext;

use super::{SecurityCompileError, SecurityScopedCompiler};
use crate::cache::generated::{
    admit_single_default_dataset, ConstantCoverageError, ConstantOccurrence, DatasetGraphAllowlist,
    GeneratedDatasetError,
};
use crate::Plan;

#[path = "generated_dataset_security_deferred.rs"]
mod deferred;

/// Dataset refusals and security/compiler failures retain their typed causes.
#[derive(Debug, thiserror::Error)]
pub enum SecurityDatasetCompileError {
    #[error(transparent)]
    Dataset(#[from] GeneratedDatasetError),
    #[error(transparent)]
    Security(#[from] SecurityCompileError),
}

impl SecurityScopedCompiler<'_> {
    /// Check policy before any work; parse and admit exactly once before every
    /// lookup in the policy/subject/attributes-partitioned cache. No access to
    /// the unscoped plan cache and no caching of admission verdicts occurs.
    pub fn compile_shared_with_single_default_dataset<G, F>(
        &self,
        context: &SecurityContext,
        sparql: &str,
        allowlist: &DatasetGraphAllowlist,
        control: &dyn QueryControl,
        graph_check: G,
        constant_check: F,
    ) -> Result<Arc<Plan>, SecurityDatasetCompileError>
    where
        G: FnMut(&str) -> Result<(), ConstantCoverageError>,
        F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
    {
        if !context.matches_policy_snapshot(self.expected_policy) {
            return Err(SecurityCompileError::PolicyMismatch.into());
        }
        let query =
            admit_single_default_dataset(sparql, allowlist, control, graph_check, constant_check)?;
        self.compile_parsed_impl(context, &query, Some(control))
            .map_err(Into::into)
    }
}

#[cfg(test)]
#[path = "generated_dataset_security_tests.rs"]
mod tests;
