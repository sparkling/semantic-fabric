//! Immutable per-source runtime binding.
//!
//! The serve lane compiles and executes only through this owner, so a plan
//! cannot be detached from the backend, source identity, dialect, mapping,
//! T-Box, constraint-quarantined compiler schema, or cache that produced it.
//! A [`crate::RuntimeSnapshot`] owns these bindings through its source registry;
//! the CLI selects either one entry or the exact bounded two-source UNION path.
//! PostgreSQL supplies a coherent startup catalogue snapshot; the abstraction
//! does not claim that for every backend, nor live drift detection, general
//! federation, or production capability admission.

use std::fmt;
use std::sync::Arc;

use sf_core::query_control::QueryControl;
use sf_core::SourceId;
use sf_sparql::federation::FederatedPlan;
use sf_sparql::{
    CompileDigests, CompileScope, CompilerBinding, Epoch, Plan, SemanticIdentity, Tbox,
};
use sf_sql::Dialect;
#[cfg(test)]
use sf_sql::TableSchema;

use crate::backend::{Backend, BackendKind};
use crate::binding_identity::RuntimeBindingIdentity;
use crate::pg_generation::{PgGenerationError, PgGenerationRequirement, SourceGeneration};
use crate::schema_observation::BoundSourceSchemaObservationV1;
use crate::semantic_admission::ValidatedMapping;
use crate::IntrospectedSource;

/// Plan-cache capacity for one immutable compiler binding (ADR-0007).
const PLAN_CACHE_CAP: usize = 64;

#[cfg(test)]
std::thread_local! {
    static TEST_BINDING_CONSTRUCTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Exact compiler facts derived from a concrete serve adapter.
///
/// These booleans describe implemented SQL semantics only. They are not a
/// release-capability profile and cannot represent production admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackendProfile {
    kind: BackendKind,
    dialect: Dialect,
    recursive_paths: bool,
    case_sensitive_like: bool,
}

impl BackendProfile {
    const fn from_kind(kind: BackendKind) -> Self {
        let dialect = kind.dialect();
        Self {
            kind,
            dialect,
            recursive_paths: dialect.supports_recursive_paths(),
            case_sensitive_like: dialect.like_is_case_sensitive(),
        }
    }

    pub const fn kind(self) -> BackendKind {
        self.kind
    }

    pub const fn dialect(self) -> Dialect {
        self.dialect
    }

    pub const fn supports_recursive_paths(self) -> bool {
        self.recursive_paths
    }

    pub const fn like_is_case_sensitive(self) -> bool {
        self.case_sensitive_like
    }
}

/// One inseparable source/compiler/backend binding for the current serve lane.
/// Raw catalogue observations become a constraint-quarantined compiler view
/// before the binding or cache exists.
pub(crate) struct RuntimeBinding {
    binding_identity: RuntimeBindingIdentity,
    backend: Backend,
    profile: BackendProfile,
    #[cfg(test)]
    observed_schema: Arc<[TableSchema]>,
    schema_observation: BoundSourceSchemaObservationV1,
    generation: SourceGeneration,
    semantic_warnings: usize,
    compiler: CompilerBinding,
}

impl RuntimeBinding {
    pub(crate) fn new(
        source: IntrospectedSource,
        mapping: ValidatedMapping,
        tbox: Tbox,
        epoch: Epoch,
    ) -> Self {
        #[cfg(test)]
        TEST_BINDING_CONSTRUCTIONS.with(|count| count.set(count.get() + 1));
        let source_id = mapping.source_id();
        let (backend, schema, observation, generation) = source.into_parts();
        let profile = BackendProfile::from_kind(backend.kind());
        let schema_observation = observation.bind(profile.kind(), source_id);
        let (mapping, ontology_digest, semantic_admission_digest, semantic_warnings) =
            mapping.into_parts();
        #[cfg(test)]
        let observed_schema: Arc<[TableSchema]> = schema.clone().into();
        let compiler = CompilerBinding::from_observation_with_semantic_identity(
            mapping,
            profile.dialect(),
            tbox,
            schema,
            epoch,
            SemanticIdentity::new(ontology_digest, semantic_admission_digest),
            PLAN_CACHE_CAP,
        );
        Self {
            binding_identity: RuntimeBindingIdentity::fresh(),
            backend,
            profile,
            #[cfg(test)]
            observed_schema,
            schema_observation,
            generation,
            semantic_warnings,
            compiler,
        }
    }

    #[cfg(test)]
    pub(crate) fn reset_test_construction_count() {
        TEST_BINDING_CONSTRUCTIONS.with(|count| count.set(0));
    }

    #[cfg(test)]
    pub(crate) fn test_construction_count() -> usize {
        TEST_BINDING_CONSTRUCTIONS.with(std::cell::Cell::get)
    }

    /// Compile through the raw cache path while carrying the request control
    /// identity into the blocking worker. The checkpoints bound handoff only;
    /// compiler-work governance remains dormant until the controlled pipeline
    /// is complete.
    pub(crate) fn compile(
        &self,
        sparql: &str,
        control: &dyn QueryControl,
    ) -> sf_sparql::Result<BoundPlan> {
        control.checkpoint()?;
        let compiled = self.compiler.compile_shared(sparql);
        control.checkpoint()?;
        compiled.map(|plan| BoundPlan {
            binding_identity: self.binding_identity.clone(),
            scope: self.compiler.scope(),
            source_id: self.compiler.source_id(),
            plan,
        })
    }

    /// Compile an uncached plan for structural admission only. The caller must
    /// discard it and may not prepare it for execution.
    pub(crate) fn preflight_compile(
        &self,
        sparql: &str,
        control: &dyn QueryControl,
    ) -> sf_sparql::Result<Arc<Plan>> {
        control.checkpoint()?;
        let compiled = self.compiler.compile_uncached_shared(sparql);
        control.checkpoint()?;
        compiled
    }

    /// Verify plan ownership before returning the inseparable execution pair.
    /// No connection, pool slot, or source I/O is acquired before this check.
    pub(crate) fn prepare_execution(
        &self,
        bound: BoundPlan,
    ) -> Result<ExecutablePlan, BindingMismatch> {
        if bound.scope != self.compiler.scope()
            || !bound.binding_identity.ptr_eq(&self.binding_identity)
            || bound.source_id != self.compiler.source_id()
            || bound.plan.dialect != self.profile.dialect()
        {
            return Err(BindingMismatch);
        }
        Ok(ExecutablePlan {
            binding_identity: self.binding_identity.clone(),
            backend: self.backend.clone(),
            source_id: self.source_id(),
            verified_generation: self.generation.is_verified(),
            plan: bound.plan,
        })
    }

    pub(crate) const fn source_id(&self) -> SourceId {
        self.compiler.source_id()
    }

    pub(crate) const fn scope(&self) -> CompileScope {
        self.compiler.scope()
    }

    #[cfg(test)]
    pub(crate) fn observed_schema(&self) -> &[TableSchema] {
        &self.observed_schema
    }

    #[cfg(test)]
    pub(crate) const fn schema_observation(&self) -> &BoundSourceSchemaObservationV1 {
        &self.schema_observation
    }

    pub(crate) const fn digests(&self) -> CompileDigests {
        self.compiler.digests()
    }

    pub(crate) const fn semantic_warning_count(&self) -> usize {
        self.semantic_warnings
    }

    pub(crate) const fn compiler(&self) -> &CompilerBinding {
        &self.compiler
    }

    pub(crate) fn generation_requirement(
        &self,
    ) -> Result<Option<PgGenerationRequirement>, PgGenerationError> {
        self.generation
            .requirement(&self.backend, &self.binding_identity)
    }
}

impl fmt::Debug for RuntimeBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeBinding")
            .field("source_id", &self.source_id())
            .field("epoch", &self.scope().epoch())
            .field("digests", &self.digests())
            .field("profile", &self.profile)
            .field("schema_observation", &self.schema_observation)
            .field("semantic_warnings", &self.semantic_warnings)
            .field("compiler", &self.compiler)
            .finish()
    }
}

/// A compiled plan attached to its exact runtime binding, source, and scope.
pub(crate) struct BoundPlan {
    binding_identity: RuntimeBindingIdentity,
    scope: CompileScope,
    source_id: SourceId,
    plan: Arc<Plan>,
}

impl BoundPlan {
    pub(crate) fn plan(&self) -> &Plan {
        &self.plan
    }

    pub(crate) const fn source_id(&self) -> SourceId {
        self.source_id
    }
}

/// A federated plan plus the private binding identities and compile scopes that
/// prove each fragment still belongs to the activated binding before any I/O.
pub(crate) struct BoundFederatedPlan {
    plan: FederatedPlan,
    binding_identities: [RuntimeBindingIdentity; 2],
    scopes: [CompileScope; 2],
}

impl BoundFederatedPlan {
    pub(crate) fn new(plan: FederatedPlan, bindings: [&RuntimeBinding; 2]) -> Self {
        let binding_for = |index: usize| {
            let source_id = plan.fragments()[index].source_id();
            bindings
                .iter()
                .copied()
                .find(|binding| binding.source_id() == source_id)
                .expect("compiled fragment source remains bound")
        };
        let binding_identities = [
            binding_for(0).binding_identity.clone(),
            binding_for(1).binding_identity.clone(),
        ];
        let scopes = [binding_for(0).scope(), binding_for(1).scope()];
        Self {
            plan,
            binding_identities,
            scopes,
        }
    }

    pub(crate) fn plan(&self) -> &FederatedPlan {
        &self.plan
    }

    pub(crate) fn into_bound_plans(self) -> (Vec<String>, [BoundPlan; 2]) {
        let variables = self.plan.variables().to_vec();
        let fragments = self.plan.fragments();
        let [first_identity, second_identity] = self.binding_identities;
        let [first_scope, second_scope] = self.scopes;
        let plans = [
            BoundPlan {
                binding_identity: first_identity,
                scope: first_scope,
                source_id: fragments[0].source_id(),
                plan: fragments[0].shared_plan(),
            },
            BoundPlan {
                binding_identity: second_identity,
                scope: second_scope,
                source_id: fragments[1].source_id(),
                plan: fragments[1].shared_plan(),
            },
        ];
        (variables, plans)
    }
}

impl fmt::Debug for BoundPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundPlan")
            .field("source_id", &self.source_id)
            .field("epoch", &self.scope.epoch())
            .field("digests", &self.scope.digests())
            .field("dialect", &self.plan.dialect)
            .finish()
    }
}

/// An ownership-checked backend/plan pair ready for form dispatch.
pub(crate) struct ExecutablePlan {
    binding_identity: RuntimeBindingIdentity,
    backend: Backend,
    source_id: SourceId,
    verified_generation: bool,
    plan: Arc<Plan>,
}

impl ExecutablePlan {
    pub(crate) fn into_parts(self) -> (SourceId, RuntimeBindingIdentity, Backend, bool, Arc<Plan>) {
        (
            self.source_id,
            self.binding_identity,
            self.backend,
            self.verified_generation,
            self.plan,
        )
    }
}

pub(crate) struct ExecutableFederatedPlan {
    variables: Vec<String>,
    fragments: [ExecutablePlan; 2],
}

impl ExecutableFederatedPlan {
    pub(crate) fn new(variables: Vec<String>, fragments: [ExecutablePlan; 2]) -> Self {
        Self {
            variables,
            fragments,
        }
    }

    pub(crate) fn into_parts(self) -> (Vec<String>, [ExecutablePlan; 2]) {
        (self.variables, self.fragments)
    }
}

/// A plan was presented to a runtime binding other than the one that compiled it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("compiled plan does not belong to this runtime binding")]
pub(crate) struct BindingMismatch;

#[cfg(test)]
#[path = "binding/tests.rs"]
mod tests;
