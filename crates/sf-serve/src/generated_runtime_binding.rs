//! Opt-in generated-query compilation composed with the sealed admission receipt.

use std::sync::Arc;

use sf_core::query_control::{QueryControl, QueryControlError};
use sf_core::security_context::{PolicySnapshotId, SecurityContext};
use sf_sparql::cache::generated::{
    ConstantCoverageError, ConstantOccurrence, ConstantRole as QueryRole, GeneratedCompileError,
    GeneratedQueryRefusal,
};
use sf_sparql::cache::SecurityCompileError;
use sf_sparql::Plan;

use super::{BoundPlan, RuntimeBinding};
use crate::budget::RequestBudget;
use crate::generated_profile_identity::GeneratedProfileIdentity;
use crate::semantic_admission::generated_admission_receipt::ReceiptMismatch;
use crate::semantic_admission::generated_mapping_coverage::{
    ConstantRole, Coverage, MappingCoverage,
};

/// Redacted failure: carries no query text, IRI or mapping detail. Request
/// controls stay inside `Compiler(Error::QueryControl(_))`.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GeneratedRuntimeError {
    #[error(transparent)]
    Refused(GeneratedQueryRefusal),
    #[error("generated admission refused: security policy mismatch")]
    PolicyMismatch,
    #[error("generated admission refused: security context is missing")]
    MissingSecurityContext,
    #[error(transparent)]
    ReceiptMismatch(#[from] ReceiptMismatch),
    #[error(transparent)]
    Compiler(sf_sparql::Error),
}

impl From<GeneratedCompileError> for GeneratedRuntimeError {
    fn from(error: GeneratedCompileError) -> Self {
        match error {
            GeneratedCompileError::Refused(refusal) => Self::Refused(refusal),
            GeneratedCompileError::Compiler(error) => Self::Compiler(error),
        }
    }
}

impl From<SecurityCompileError> for GeneratedRuntimeError {
    fn from(error: SecurityCompileError) -> Self {
        match error {
            SecurityCompileError::GeneratedQueryRefused(refusal) => Self::Refused(refusal),
            SecurityCompileError::PolicyMismatch => Self::PolicyMismatch,
            SecurityCompileError::Compiler(error) => Self::Compiler(error),
            _ => Self::Compiler(sf_sparql::Error::Mapping(
                "security partition mismatch".into(),
            )),
        }
    }
}

/// Existing inseparable plan plus the stable response-profile identity, issued
/// only after a successful authoritative compilation.
#[derive(Debug)]
pub(crate) struct GeneratedCompiled {
    pub(crate) plan: BoundPlan,
    pub(crate) identity: GeneratedProfileIdentity,
}

fn stopped(cause: QueryControlError) -> GeneratedRuntimeError {
    GeneratedRuntimeError::Compiler(sf_sparql::Error::from(cause))
}

const fn map_role(role: QueryRole) -> ConstantRole {
    match role {
        QueryRole::Subject => ConstantRole::Subject,
        QueryRole::Predicate => ConstantRole::Predicate,
        QueryRole::Object => ConstantRole::Object,
        QueryRole::Class => ConstantRole::Class,
        QueryRole::NamedGraph => ConstantRole::NamedGraph,
        QueryRole::LiteralDatatype => ConstantRole::LiteralDatatype,
        QueryRole::Unresolved => ConstantRole::Unresolved,
    }
}

/// Covered proceeds; Uncovered and Indeterminate fail closed; controls stay controls.
fn admit_constant(
    coverage: &MappingCoverage<'_>,
    control: &dyn QueryControl,
    occurrence: ConstantOccurrence<'_>,
) -> Result<(), ConstantCoverageError> {
    match coverage.classify(map_role(occurrence.role()), occurrence.iri(), control) {
        Ok(Coverage::Covered) => Ok(()),
        Ok(Coverage::Uncovered | Coverage::Indeterminate) => Err(ConstantCoverageError::Uncovered),
        Err(cause) => Err(ConstantCoverageError::Control(cause)),
    }
}

impl RuntimeBinding {
    fn generated_pair(
        &self,
        plan: Arc<Plan>,
        security: Option<SecurityContext>,
    ) -> GeneratedCompiled {
        GeneratedCompiled {
            plan: BoundPlan {
                lineage: None,
                security,
                binding_identity: self.binding_identity.clone(),
                scope: self.compiler.scope(),
                source_id: self.compiler.source_id(),
                plan,
            },
            identity: self.generated.identity(),
        }
    }

    /// Cached authoritative compile behind sealed mapping coverage. Hits rerun
    /// structural and coverage checks.
    pub(crate) fn compile_generated(
        &self,
        sparql: &str,
        control: &dyn QueryControl,
    ) -> Result<GeneratedCompiled, GeneratedRuntimeError> {
        let coverage = self.generated.coverage(&self.compiler)?;
        control.checkpoint().map_err(stopped)?;
        let compiled =
            self.compiler
                .compile_shared_with_generated_admission(sparql, control, |occurrence| {
                    admit_constant(&coverage, control, occurrence)
                });
        control.checkpoint().map_err(stopped)?;
        let plan = compiled?;
        Ok(self.generated_pair(plan, None))
    }

    /// Uncached admission-only counterpart. Issues no identity and reads or
    /// writes no cache; the caller must discard the plan.
    pub(crate) fn preflight_generated(
        &self,
        sparql: &str,
        control: &dyn QueryControl,
    ) -> Result<Arc<Plan>, GeneratedRuntimeError> {
        let coverage = self.generated.coverage(&self.compiler)?;
        control.checkpoint().map_err(stopped)?;
        let compiled = self
            .compiler
            .compile_uncached_shared_with_generated_admission(sparql, control, |occurrence| {
                admit_constant(&coverage, control, occurrence)
            });
        control.checkpoint().map_err(stopped)?;
        compiled.map_err(Into::into)
    }

    /// Security-scoped path. The request context and policy are verified
    /// before checkpoint, receipt, parse, screen and coverage.
    pub(crate) fn compile_generated_secured(
        &self,
        sparql: &str,
        budget: &RequestBudget,
        policy: PolicySnapshotId,
    ) -> Result<GeneratedCompiled, GeneratedRuntimeError> {
        let context = budget
            .security_context()
            .ok_or(GeneratedRuntimeError::MissingSecurityContext)?;
        if !context.matches_policy_snapshot(policy) {
            return Err(GeneratedRuntimeError::PolicyMismatch);
        }
        budget.checkpoint().map_err(stopped)?;
        let coverage = self.generated.coverage(&self.compiler)?;
        let compiled = self
            .compiler
            .for_security_policy(policy, &self.security_cache)
            .compile_shared_with_generated_admission(&context, sparql, budget, |occurrence| {
                admit_constant(&coverage, budget, occurrence)
            });
        budget.checkpoint().map_err(stopped)?;
        let plan = compiled?;
        Ok(self.generated_pair(plan, Some(context)))
    }
}

#[cfg(test)]
#[path = "generated_runtime_binding_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "generated_runtime_security_tests.rs"]
mod security_tests;
