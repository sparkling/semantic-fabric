//! Immutable semantic/runtime state and its source-keyed registry.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use sf_core::query_control::QueryControl;
use sf_core::{SourceId, SourceMapping};
use sf_sparql::federation::compile_source_affine_union;
use sf_sparql::{CompileDigests, Epoch, OntologyDigest, Tbox};
use sf_sql::TableSchema;

use crate::binding::{
    BindingMismatch, BoundFederatedPlan, BoundPlan, ExecutableFederatedPlan, ExecutablePlan,
    IntrospectedSource, RuntimeBinding,
};
use crate::BackendProfile;

/// One source/backend/schema/mapping input awaiting snapshot validation.
pub struct RuntimeSource {
    source: IntrospectedSource,
    mapping: SourceMapping,
}

impl RuntimeSource {
    pub fn new(source: IntrospectedSource, mapping: SourceMapping) -> Self {
        Self { source, mapping }
    }

    pub const fn source_id(&self) -> SourceId {
        self.mapping.source_id()
    }
}

impl fmt::Debug for RuntimeSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeSource")
            .field("source_id", &self.source_id())
            .field("backend_kind", &self.source.kind())
            .field("mapping", &self.mapping)
            .finish()
    }
}

/// Immutable lookup of runtime source bindings by snapshot-local [`SourceId`].
pub struct SourceRegistry {
    entries: BTreeMap<SourceId, RuntimeBinding>,
}

impl SourceRegistry {
    fn build(
        epoch: Epoch,
        ontology: &Tbox,
        sources: Vec<RuntimeSource>,
    ) -> Result<Self, SnapshotError> {
        if sources.is_empty() {
            return Err(SnapshotError::EmptyRegistry);
        }

        let mut ids = BTreeSet::new();
        for source in &sources {
            if !ids.insert(source.source_id()) {
                return Err(SnapshotError::DuplicateSource {
                    source_id: source.source_id(),
                });
            }
        }

        let entries = sources
            .into_iter()
            .map(|source| {
                let source_id = source.source_id();
                let binding =
                    RuntimeBinding::new(source.source, source.mapping, ontology.clone(), epoch);
                (source_id, binding)
            })
            .collect();
        Ok(Self { entries })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains_source(&self, source_id: SourceId) -> bool {
        self.entries.contains_key(&source_id)
    }

    pub fn source_ids(&self) -> impl Iterator<Item = SourceId> + '_ {
        self.entries.keys().copied()
    }

    pub fn profile(&self, source_id: SourceId) -> Option<BackendProfile> {
        self.entries.get(&source_id).map(RuntimeBinding::profile)
    }

    pub fn schema(&self, source_id: SourceId) -> Option<&[TableSchema]> {
        self.entries
            .get(&source_id)
            .map(RuntimeBinding::observed_schema)
    }

    pub fn digests(&self, source_id: SourceId) -> Option<CompileDigests> {
        self.entries.get(&source_id).map(RuntimeBinding::digests)
    }

    fn binding(&self, source_id: SourceId) -> Option<&RuntimeBinding> {
        self.entries.get(&source_id)
    }
}

impl fmt::Debug for SourceRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SourceRegistry")
            .field("source_ids", &self.entries.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Immutable ontology, source mappings, observed schemas, runtime bindings,
/// epochs, and deterministic digests for one runtime generation.
pub struct RuntimeSnapshot {
    epoch: Epoch,
    ontology: Tbox,
    ontology_digest: OntologyDigest,
    registry: SourceRegistry,
}

impl RuntimeSnapshot {
    /// Validate source identity uniqueness before constructing any bindings.
    pub fn new(
        epoch: Epoch,
        ontology: Tbox,
        sources: Vec<RuntimeSource>,
    ) -> Result<Self, SnapshotError> {
        let registry = SourceRegistry::build(epoch, &ontology, sources)?;
        let ontology_digest = registry
            .entries
            .values()
            .next()
            .expect("a validated registry is non-empty")
            .digests()
            .ontology();
        debug_assert!(registry
            .entries
            .values()
            .all(|binding| binding.digests().ontology() == ontology_digest));
        Ok(Self {
            epoch,
            ontology,
            ontology_digest,
            registry,
        })
    }

    /// Infallible compatibility path for the current one-source CLI/server.
    pub fn single(epoch: Epoch, ontology: Tbox, source: RuntimeSource) -> Self {
        Self::new(epoch, ontology, vec![source])
            .expect("one runtime source has a unique, non-empty registry identity")
    }

    pub const fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub const fn ontology(&self) -> &Tbox {
        &self.ontology
    }

    pub const fn ontology_digest(&self) -> OntologyDigest {
        self.ontology_digest
    }

    pub const fn registry(&self) -> &SourceRegistry {
        &self.registry
    }

    pub(crate) fn compile(
        &self,
        source_id: SourceId,
        sparql: &str,
        control: &dyn QueryControl,
    ) -> sf_sparql::Result<BoundPlan> {
        self.registry
            .binding(source_id)
            .ok_or_else(|| {
                sf_sparql::Error::Mapping("runtime source is not registered".to_owned())
            })?
            .compile(sparql, control)
    }

    pub(crate) fn prepare_execution(
        &self,
        bound: BoundPlan,
    ) -> Result<ExecutablePlan, BindingMismatch> {
        self.registry
            .binding(bound.source_id())
            .ok_or(BindingMismatch)?
            .prepare_execution(bound)
    }

    pub(crate) fn compile_federated_union(
        &self,
        source_ids: [SourceId; 2],
        sparql: &str,
        control: &dyn QueryControl,
    ) -> sf_sparql::Result<BoundFederatedPlan> {
        if source_ids[0] == source_ids[1]
            || source_ids
                .iter()
                .any(|source_id| !self.registry.contains_source(*source_id))
        {
            return Err(sf_sparql::Error::Mapping(
                "federated source registry does not match the serving configuration".to_owned(),
            ));
        }

        let left = self.registry.binding(source_ids[0]).ok_or_else(|| {
            sf_sparql::Error::Mapping("runtime source is not registered".to_owned())
        })?;
        let right = self.registry.binding(source_ids[1]).ok_or_else(|| {
            sf_sparql::Error::Mapping("runtime source is not registered".to_owned())
        })?;
        let plan =
            compile_source_affine_union(sparql, [left.compiler(), right.compiler()], control)?;
        let scope_for = |index: usize| {
            self.registry
                .binding(plan.fragments()[index].source_id())
                .expect("compiled fragment source remains registered")
                .scope()
        };
        let scopes = [scope_for(0), scope_for(1)];
        Ok(BoundFederatedPlan::new(plan, scopes))
    }

    pub(crate) fn prepare_federated_execution(
        &self,
        bound: BoundFederatedPlan,
    ) -> Result<ExecutableFederatedPlan, BindingMismatch> {
        let (plan, scopes) = bound.into_parts();
        let variables = plan.variables().to_vec();
        let mut executable = Vec::with_capacity(2);
        for (fragment, scope) in plan.fragments().iter().zip(scopes) {
            if fragment.source_id() != scope.source_id() {
                return Err(BindingMismatch);
            }
            let binding = self
                .registry
                .binding(fragment.source_id())
                .ok_or(BindingMismatch)?;
            executable.push(binding.prepare_execution(BoundPlan::from_parts(
                scope,
                fragment.source_id(),
                fragment.shared_plan(),
            ))?);
        }
        let executable = executable.try_into().map_err(|_| BindingMismatch)?;
        Ok(ExecutableFederatedPlan::new(variables, executable))
    }
}

impl fmt::Debug for RuntimeSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeSnapshot")
            .field("epoch", &self.epoch)
            .field("ontology_digest", &self.ontology_digest)
            .field("registry", &self.registry)
            .finish()
    }
}

/// Runtime snapshot construction rejected an invalid source registry.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SnapshotError {
    #[error("runtime snapshot requires at least one source")]
    EmptyRegistry,
    #[error("runtime snapshot contains duplicate {source_id}")]
    DuplicateSource { source_id: SourceId },
}
