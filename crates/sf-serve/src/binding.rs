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
        let (backend, schema, generation) = source.into_parts();
        let (mapping, ontology_digest, semantic_admission_digest, semantic_warnings) =
            mapping.into_parts();
        let profile = BackendProfile::from_kind(backend.kind());
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
        if !bound.binding_identity.ptr_eq(&self.binding_identity)
            || bound.scope != self.compiler.scope()
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

    pub(crate) fn binding_identity(&self) -> RuntimeBindingIdentity {
        self.binding_identity.clone()
    }

    #[cfg(test)]
    pub(crate) fn observed_schema(&self) -> &[TableSchema] {
        &self.observed_schema
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
    pub(crate) fn from_parts(
        binding_identity: RuntimeBindingIdentity,
        scope: CompileScope,
        source_id: SourceId,
        plan: Arc<Plan>,
    ) -> Self {
        Self {
            binding_identity,
            scope,
            source_id,
            plan,
        }
    }

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
    pub(crate) fn new(
        plan: FederatedPlan,
        binding_identities: [RuntimeBindingIdentity; 2],
        scopes: [CompileScope; 2],
    ) -> Self {
        Self {
            plan,
            binding_identities,
            scopes,
        }
    }

    pub(crate) fn plan(&self) -> &FederatedPlan {
        &self.plan
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        FederatedPlan,
        [RuntimeBindingIdentity; 2],
        [CompileScope; 2],
    ) {
        (self.plan, self.binding_identities, self.scopes)
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
mod tests {
    use super::*;

    const SECRET: &str = "sf_secret_binding_debug_must_not_expose";
    const MAPPING: &str = r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        <#items> a rr:TriplesMap ;
            rr:logicalTable [ rr:sqlQuery "SELECT sf_secret_binding_debug_must_not_expose FROM private_items" ] ;
            rr:subjectMap [ rr:template "http://example.test/item/{id}" ] .
    "#;

    fn binding(source_index: usize) -> RuntimeBinding {
        binding_at(source_index, Epoch::default())
    }

    fn binding_at(source_index: usize, epoch: Epoch) -> RuntimeBinding {
        let source_id = SourceId::new(source_index).unwrap();
        let mapping = sf_mapping::parse_r2rml_for_source(MAPPING, source_id).unwrap();
        let ontology = crate::test_support::empty_ontology();
        let source = IntrospectedSource::unchecked(
            Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
            vec![TableSchema::new(SECRET)],
        );
        let mapping = crate::semantic_admission::ValidatedMapping::validate(
            mapping,
            crate::semantic_admission::MappingOrigin::Authored,
            &ontology,
            &source,
        )
        .unwrap();
        RuntimeBinding::new(source, mapping, ontology.tbox().clone(), epoch)
    }

    #[test]
    fn backend_profiles_are_derived_and_never_admission_claims() {
        for kind in [
            BackendKind::Sqlite,
            BackendKind::Postgres,
            BackendKind::MySql,
        ] {
            let profile = BackendProfile::from_kind(kind);
            assert_eq!(profile.kind(), kind);
            assert_eq!(profile.dialect(), kind.dialect());
            assert_eq!(
                profile.supports_recursive_paths(),
                kind.dialect().supports_recursive_paths()
            );
            assert_eq!(
                profile.like_is_case_sensitive(),
                kind.dialect().like_is_case_sensitive()
            );
        }
    }

    #[test]
    fn a_plan_from_another_binding_is_rejected_before_execution() {
        let first = binding(0);
        let second = binding(0);
        let control = sf_core::query_control::UncontrolledQueryControl;
        let bound = first
            .compile("SELECT * WHERE { ?s ?p ?o }", &control)
            .unwrap();

        assert_eq!(first.scope(), second.scope(), "regression precondition");
        let Err(error) = second.prepare_execution(bound) else {
            panic!("content-equal bindings must not share plan authority");
        };
        assert_eq!(error, BindingMismatch);
        assert_eq!(
            error.to_string(),
            "compiled plan does not belong to this runtime binding"
        );
    }

    #[test]
    fn a_plan_remains_valid_for_its_original_binding() {
        let binding = binding(0);
        let control = sf_core::query_control::UncontrolledQueryControl;
        let bound = binding
            .compile("SELECT * WHERE { ?s ?p ?o }", &control)
            .unwrap();

        binding.prepare_execution(bound).unwrap();
    }

    #[test]
    fn binding_debug_output_is_structural_and_secret_free() {
        let binding = binding(7);
        let debug = format!("{binding:?}");

        assert!(debug.contains("digests"));
        assert!(debug.contains("triples_map_count"));
        assert!(debug.contains("Unverified"));
        assert!(!debug.contains(SECRET));
        assert!(!debug.contains("private_items"));
        assert!(!debug.contains("SELECT"));
    }

    #[test]
    fn runtime_binding_quarantines_catalogue_constraint_authority() {
        let binding = binding(0);
        assert_eq!(
            binding.compiler.constraint_authority(),
            sf_sparql::ConstraintAuthority::Unverified
        );
        assert_eq!(
            binding.scope().constraint_authority(),
            sf_sparql::ConstraintAuthority::Unverified
        );
    }

    #[test]
    fn runtime_binding_reuses_the_cached_plan_allocation() {
        let binding = binding(0);
        let control = sf_core::query_control::UncontrolledQueryControl;
        let first = binding
            .compile("SELECT * WHERE { ?s ?p ?o }", &control)
            .unwrap();
        let second = binding
            .compile("SELECT * WHERE { ?s ?p ?o }", &control)
            .unwrap();

        assert!(Arc::ptr_eq(&first.plan, &second.plan));
    }
}
