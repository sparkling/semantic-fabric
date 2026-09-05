//! Immutable per-source runtime binding.
//!
//! The serve lane compiles and executes only through this owner, so a plan
//! cannot be detached from the backend, source identity, dialect, mapping,
//! T-Box, constraint-quarantined compiler schema, or cache that produced it.
//! A [`crate::RuntimeSnapshot`] owns these bindings through its source registry;
//! the current CLI selects exactly one entry and exposes no federated query path.
//! PostgreSQL supplies a coherent startup catalogue snapshot; the abstraction
//! does not claim that for every backend, nor live drift detection, federation,
//! or production capability admission.

use std::fmt;
use std::sync::Arc;

use sf_core::query_control::QueryControl;
use sf_core::{SourceId, SourceMapping};
use sf_sparql::federation::FederatedPlan;
use sf_sparql::{CompileDigests, CompileScope, CompilerBinding, Epoch, Plan, Tbox};
use sf_sql::{Dialect, TableSchema};

use crate::backend::{Backend, BackendKind};

/// Plan-cache capacity for one immutable compiler binding (ADR-0007).
const PLAN_CACHE_CAP: usize = 64;

/// A backend paired with the schema observation made through that backend.
///
/// Pairing prevents later constructors from independently mixing a handle and
/// unrelated schema vector. PostgreSQL observes one coherent read-only,
/// repeatable-read `public` catalogue snapshot; SQLite and MySQL do not yet
/// observe a whole catalogue in one explicit transaction. No path detects later
/// drift, so this type deliberately says `Introspected`, not `VerifiedSnapshot`.
pub struct IntrospectedSource {
    backend: Backend,
    schema: Vec<TableSchema>,
}

impl IntrospectedSource {
    /// Build an explicitly unchecked pair for tests and embedding compatibility.
    /// Production startup uses the crate-private observed constructor returned by
    /// the backend opener.
    pub fn unchecked(backend: Backend, schema: Vec<TableSchema>) -> Self {
        Self { backend, schema }
    }

    pub(crate) fn observed(backend: Backend, schema: Vec<TableSchema>) -> Self {
        Self { backend, schema }
    }

    pub const fn kind(&self) -> BackendKind {
        self.backend.kind()
    }

    pub(crate) fn into_parts(self) -> (Backend, Vec<TableSchema>) {
        (self.backend, self.schema)
    }
}

impl fmt::Debug for IntrospectedSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IntrospectedSource")
            .field("backend_kind", &self.kind())
            .field("schema_table_count", &self.schema.len())
            .finish()
    }
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
    backend: Backend,
    profile: BackendProfile,
    observed_schema: Arc<[TableSchema]>,
    compiler: CompilerBinding,
}

impl RuntimeBinding {
    pub(crate) fn new(
        source: IntrospectedSource,
        mapping: SourceMapping,
        tbox: Tbox,
        epoch: Epoch,
    ) -> Self {
        let (backend, schema) = source.into_parts();
        let profile = BackendProfile::from_kind(backend.kind());
        let observed_schema: Arc<[TableSchema]> = schema.into();
        let compiler = CompilerBinding::from_unverified_observation(
            mapping,
            profile.dialect(),
            tbox,
            observed_schema.to_vec(),
            epoch,
            PLAN_CACHE_CAP,
        );
        Self {
            backend,
            profile,
            observed_schema,
            compiler,
        }
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
            scope: self.compiler.scope(),
            source_id: self.compiler.source_id(),
            plan,
        })
    }

    /// Verify plan ownership before returning the inseparable execution pair.
    /// No connection, pool slot, or source I/O is acquired before this check.
    pub(crate) fn prepare_execution(
        &self,
        bound: BoundPlan,
    ) -> Result<ExecutablePlan, BindingMismatch> {
        if bound.scope != self.compiler.scope()
            || bound.source_id != self.compiler.source_id()
            || bound.plan.dialect != self.profile.dialect()
        {
            return Err(BindingMismatch);
        }
        Ok(ExecutablePlan {
            backend: self.backend.clone(),
            plan: bound.plan,
        })
    }

    pub(crate) const fn source_id(&self) -> SourceId {
        self.compiler.source_id()
    }

    pub(crate) const fn scope(&self) -> CompileScope {
        self.compiler.scope()
    }

    pub(crate) const fn profile(&self) -> BackendProfile {
        self.profile
    }

    pub(crate) fn observed_schema(&self) -> &[TableSchema] {
        &self.observed_schema
    }

    pub(crate) const fn digests(&self) -> CompileDigests {
        self.compiler.digests()
    }

    pub(crate) const fn compiler(&self) -> &CompilerBinding {
        &self.compiler
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
            .field("compiler", &self.compiler)
            .finish()
    }
}

/// A compiled plan that remains attached to its source and compile scope.
pub(crate) struct BoundPlan {
    scope: CompileScope,
    source_id: SourceId,
    plan: Arc<Plan>,
}

impl BoundPlan {
    pub(crate) fn from_parts(scope: CompileScope, source_id: SourceId, plan: Arc<Plan>) -> Self {
        Self {
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

/// A federated plan plus the private compile scopes needed to prove that each
/// fragment still belongs to the activated source binding before any I/O.
pub(crate) struct BoundFederatedPlan {
    plan: FederatedPlan,
    scopes: [CompileScope; 2],
}

impl BoundFederatedPlan {
    pub(crate) fn new(plan: FederatedPlan, scopes: [CompileScope; 2]) -> Self {
        Self { plan, scopes }
    }

    pub(crate) fn plan(&self) -> &FederatedPlan {
        &self.plan
    }

    pub(crate) fn into_parts(self) -> (FederatedPlan, [CompileScope; 2]) {
        (self.plan, self.scopes)
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
    backend: Backend,
    plan: Arc<Plan>,
}

impl ExecutablePlan {
    pub(crate) fn into_parts(self) -> (Backend, Arc<Plan>) {
        (self.backend, self.plan)
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
        let source = IntrospectedSource::unchecked(
            Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
            vec![TableSchema::new(SECRET)],
        );
        RuntimeBinding::new(source, mapping, Tbox::default(), epoch)
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
        let second = binding_at(0, Epoch(1));
        let control = sf_core::query_control::UncontrolledQueryControl;
        let bound = first
            .compile("SELECT * WHERE { ?s ?p ?o }", &control)
            .unwrap();

        assert_ne!(first.scope(), second.scope());
        assert!(matches!(
            second.prepare_execution(bound),
            Err(BindingMismatch)
        ));
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
