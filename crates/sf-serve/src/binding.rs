//! Immutable single-source runtime binding.
//!
//! The serve lane compiles and executes only through this owner, so a plan
//! cannot be detached from the backend, source identity, dialect, mapping,
//! T-Box, observed schema, or cache that produced it. This is the enforcing
//! single-source precursor to the proposed multi-source `RuntimeSnapshot`.
//! PostgreSQL supplies a coherent startup catalogue snapshot; the abstraction
//! does not claim that for every backend, nor live drift detection, federation,
//! or production capability admission.

use std::fmt;

use sf_core::{SourceId, SourceMapping};
use sf_sparql::{CompileScope, CompilerBinding, Plan, Tbox};
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
pub(crate) struct RuntimeBinding {
    backend: Backend,
    profile: BackendProfile,
    compiler: CompilerBinding,
}

impl RuntimeBinding {
    pub(crate) fn new(source: IntrospectedSource, mapping: SourceMapping, tbox: Tbox) -> Self {
        let (backend, schema) = source.into_parts();
        let profile = BackendProfile::from_kind(backend.kind());
        let compiler =
            CompilerBinding::new(mapping, profile.dialect(), tbox, schema, PLAN_CACHE_CAP);
        Self {
            backend,
            profile,
            compiler,
        }
    }

    pub(crate) fn compile(&self, sparql: &str) -> sf_sparql::Result<BoundPlan> {
        self.compiler.compile(sparql).map(|plan| BoundPlan {
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
}

impl fmt::Debug for RuntimeBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeBinding")
            .field("binding_id", &self.scope().binding_id())
            .field("source_id", &self.source_id())
            .field("profile", &self.profile)
            .field("compiler", &self.compiler)
            .finish()
    }
}

/// A compiled plan that remains attached to its source and compile scope.
pub(crate) struct BoundPlan {
    scope: CompileScope,
    source_id: SourceId,
    plan: Plan,
}

impl BoundPlan {
    pub(crate) const fn plan(&self) -> &Plan {
        &self.plan
    }
}

impl fmt::Debug for BoundPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundPlan")
            .field("binding_id", &self.scope.binding_id())
            .field("source_id", &self.source_id)
            .field("dialect", &self.plan.dialect)
            .finish()
    }
}

/// A verified backend/plan pair ready for form dispatch.
pub(crate) struct ExecutablePlan {
    backend: Backend,
    plan: Plan,
}

impl ExecutablePlan {
    pub(crate) fn into_parts(self) -> (Backend, Plan) {
        (self.backend, self.plan)
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
        let source_id = SourceId::new(source_index).unwrap();
        let mapping = sf_mapping::parse_r2rml_for_source(MAPPING, source_id).unwrap();
        let source = IntrospectedSource::unchecked(
            Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
            vec![TableSchema::new(SECRET)],
        );
        RuntimeBinding::new(source, mapping, Tbox::default())
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
        let bound = first.compile("SELECT * WHERE { ?s ?p ?o }").unwrap();

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

        assert!(debug.contains("binding_id"));
        assert!(debug.contains("triples_map_count"));
        assert!(!debug.contains(SECRET));
        assert!(!debug.contains("private_items"));
        assert!(!debug.contains("SELECT"));
    }
}
