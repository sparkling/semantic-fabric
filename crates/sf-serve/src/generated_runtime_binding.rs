//! Opt-in generated-query compilation composed with the sealed admission receipt.

use std::fmt;
use std::sync::Arc;

use sf_core::query_control::{QueryControl, QueryControlError};
use sf_core::security_context::{PolicySnapshotId, SecurityContext};
use sf_sparql::cache::generated::{
    ConstantCoverageError, ConstantOccurrence, ConstantRole as QueryRole, GeneratedCompileError,
    GeneratedDeferred, GeneratedQueryRefusal,
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

/// Outcome of deferred-refusal generated admission. `Refused` holds no identity
/// and no `BoundPlan`. Its optional plan is row-authorization input only: it
/// must never be executed, cached or treated as admitted.
pub(crate) enum GeneratedDeferredOutcome<T> {
    Admitted(T),
    Refused {
        refusal: GeneratedQueryRefusal,
        authorization_plan: Option<Arc<Plan>>,
    },
}

impl<T> GeneratedDeferredOutcome<T> {
    pub(crate) fn map<U>(self, admit: impl FnOnce(T) -> U) -> GeneratedDeferredOutcome<U> {
        match self {
            Self::Admitted(value) => GeneratedDeferredOutcome::Admitted(admit(value)),
            Self::Refused {
                refusal,
                authorization_plan,
            } => GeneratedDeferredOutcome::Refused {
                refusal,
                authorization_plan,
            },
        }
    }
}

impl From<GeneratedDeferred> for GeneratedDeferredOutcome<Arc<Plan>> {
    fn from(deferred: GeneratedDeferred) -> Self {
        match deferred {
            GeneratedDeferred::Admitted(plan) => Self::Admitted(plan),
            GeneratedDeferred::Refused {
                refusal,
                authorization_plan,
            } => Self::Refused {
                refusal,
                authorization_plan,
            },
        }
    }
}

impl<T> fmt::Debug for GeneratedDeferredOutcome<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::Refused { refusal, .. } = self else {
            return f.write_str("Admitted(<redacted>)");
        };
        f.debug_struct("Refused")
            .field("refusal", refusal)
            .finish_non_exhaustive()
    }
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

    /// Deferred-refusal counterpart of [`Self::compile_generated`]. A refusal is
    /// data: no identity, no `BoundPlan`, nothing cached.
    pub(crate) fn compile_generated_deferred(
        &self,
        sparql: &str,
        control: &dyn QueryControl,
    ) -> Result<GeneratedDeferredOutcome<GeneratedCompiled>, GeneratedRuntimeError> {
        let coverage = self.generated.coverage(&self.compiler)?;
        control.checkpoint().map_err(stopped)?;
        let check = |found: ConstantOccurrence<'_>| admit_constant(&coverage, control, found);
        let compiled = self
            .compiler
            .compile_shared_with_generated_admission_deferred(sparql, control, check);
        control.checkpoint().map_err(stopped)?;
        let deferred = compiled.map_err(GeneratedRuntimeError::Compiler)?;
        let outcome = GeneratedDeferredOutcome::from(deferred);
        Ok(outcome.map(|plan| self.generated_pair(plan, None)))
    }

    /// Uncached deferred-refusal counterpart of [`Self::preflight_generated`].
    /// Issues no identity and reads or writes no cache.
    pub(crate) fn preflight_generated_deferred(
        &self,
        sparql: &str,
        control: &dyn QueryControl,
    ) -> Result<GeneratedDeferredOutcome<Arc<Plan>>, GeneratedRuntimeError> {
        let coverage = self.generated.coverage(&self.compiler)?;
        control.checkpoint().map_err(stopped)?;
        let check = |found: ConstantOccurrence<'_>| admit_constant(&coverage, control, found);
        let compiled = self
            .compiler
            .compile_uncached_shared_with_generated_admission_deferred(sparql, control, check);
        control.checkpoint().map_err(stopped)?;
        let deferred = compiled.map_err(GeneratedRuntimeError::Compiler)?;
        Ok(GeneratedDeferredOutcome::from(deferred))
    }

    /// Deferred-refusal counterpart of [`Self::compile_generated_secured`]. The
    /// request context and policy are verified before checkpoint, receipt, parse
    /// and coverage.
    pub(crate) fn compile_generated_secured_deferred(
        &self,
        sparql: &str,
        budget: &RequestBudget,
        policy: PolicySnapshotId,
    ) -> Result<GeneratedDeferredOutcome<GeneratedCompiled>, GeneratedRuntimeError> {
        let context = budget
            .security_context()
            .ok_or(GeneratedRuntimeError::MissingSecurityContext)?;
        if !context.matches_policy_snapshot(policy) {
            return Err(GeneratedRuntimeError::PolicyMismatch);
        }
        budget.checkpoint().map_err(stopped)?;
        let coverage = self.generated.coverage(&self.compiler)?;
        let check = |found: ConstantOccurrence<'_>| admit_constant(&coverage, budget, found);
        let compiled = self
            .compiler
            .for_security_policy(policy, &self.security_cache)
            .compile_shared_with_generated_admission_deferred(&context, sparql, budget, check);
        budget.checkpoint().map_err(stopped)?;
        let outcome = GeneratedDeferredOutcome::from(compiled?);
        Ok(outcome.map(|plan| self.generated_pair(plan, Some(context))))
    }
}

#[cfg(test)]
#[path = "generated_runtime_binding_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "generated_runtime_security_tests.rs"]
mod security_tests;

#[cfg(test)]
#[path = "generated_runtime_deferred_tests.rs"]
mod deferred_tests;

#[cfg(test)]
mod deferred_test_controls {
    use crate::budget::RequestBudget;
    use sf_core::query_control::{
        QueryBudget, QueryCharge, QueryControl, QueryControlError as Stop, QueryLimits,
    };
    use sf_core::security_context::{
        PolicySnapshotId, RequestAttributesIdentity, SecurityContext, SubjectIdentity,
    };
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;
    pub(super) fn policy(value: u8) -> PolicySnapshotId {
        PolicySnapshotId::from_digest([value; 32]).unwrap()
    }

    pub(super) fn context(subject: u8, attributes: u8) -> SecurityContext {
        SecurityContext::new(
            policy(1),
            SubjectIdentity::from_digest([subject; 32]).unwrap(),
            RequestAttributesIdentity::from_digest([attributes; 32]).unwrap(),
        )
    }

    pub(super) fn who() -> SecurityContext {
        context(2, 3)
    }

    pub(super) fn request(context: Option<SecurityContext>, work: u64) -> RequestBudget {
        let limits = QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX);
        let mut budget = RequestBudget::after(Duration::from_secs(30), limits);
        if let Some(context) = context {
            budget.retain_security(context).unwrap();
        }
        budget
    }

    pub(super) fn open() -> RequestBudget {
        request(Some(who()), u64::MAX)
    }

    pub(super) fn stop(budget: &RequestBudget, cause: Stop) {
        QueryControl::terminate(budget, cause);
    }

    pub(super) struct Trip {
        budget: QueryBudget,
        pub(super) calls: AtomicU64,
        at: u64,
        reason: Stop,
    }

    impl Trip {
        pub(super) fn new(at: u64, reason: Stop) -> Self {
            let limits = QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX);
            Self {
                budget: QueryBudget::new(limits),
                calls: AtomicU64::new(0),
                at,
                reason,
            }
        }
    }

    impl QueryControl for Trip {
        fn checkpoint(&self) -> Result<(), Stop> {
            self.budget.checkpoint()
        }

        fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), Stop> {
            if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.at {
                self.budget.terminate(self.reason);
            }
            self.budget.consume(charge, amount)
        }

        fn terminate(&self, reason: Stop) -> Stop {
            self.budget.terminate(reason)
        }
    }
}
