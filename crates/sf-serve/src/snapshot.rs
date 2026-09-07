//! Immutable semantic/runtime state and its source-keyed registry.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use sf_core::query_control::QueryControl;
use sf_core::SourceId;
#[cfg(test)]
use sf_core::SourceMapping;
use sf_sparql::federation::{
    compile_source_affine_union, compile_source_affine_union_uncached, FederatedPlan,
};
#[cfg(test)]
use sf_sparql::{CompileDigests, CompileScope};
use sf_sparql::{Epoch, OntologyDigest, Plan};
#[cfg(test)]
use sf_sql::TableSchema;

use crate::binding::{
    BindingMismatch, BoundFederatedPlan, BoundPlan, ExecutableFederatedPlan, ExecutablePlan,
    RuntimeBinding,
};
use crate::pg_generation::{PgGenerationError, PgGenerationRequirement};
#[cfg(test)]
use crate::semantic_admission::MappingOrigin;
use crate::semantic_admission::{SemanticAdmissionError, ValidatedMapping};
use crate::{IntrospectedSource, SemanticOntology};

enum RuntimeMapping {
    #[cfg(test)]
    Pending {
        mapping: SourceMapping,
        origin: MappingOrigin,
    },
    Validated(ValidatedMapping),
}

impl RuntimeMapping {
    const fn source_id(&self) -> SourceId {
        match self {
            #[cfg(test)]
            Self::Pending { mapping, .. } => mapping.source_id(),
            Self::Validated(mapping) => mapping.source_id(),
        }
    }

    fn validate(
        self,
        ontology: &SemanticOntology,
        source: &IntrospectedSource,
    ) -> Result<ValidatedMapping, SemanticAdmissionError> {
        match self {
            #[cfg(test)]
            Self::Pending { mapping, origin } => {
                ValidatedMapping::validate(mapping, origin, ontology, source)
            }
            Self::Validated(mapping) => mapping.ensure_context(ontology, source),
        }
    }
}

/// One source/backend/schema/mapping input awaiting snapshot validation.
pub(crate) struct RuntimeSource {
    source: IntrospectedSource,
    mapping: RuntimeMapping,
}

impl RuntimeSource {
    /// Construct an authored mapping candidate. Semantic admission runs before
    /// any runtime binding, compiler, or plan cache is allocated.
    #[cfg(test)]
    pub(crate) fn new(source: IntrospectedSource, mapping: SourceMapping) -> Self {
        Self {
            source,
            mapping: RuntimeMapping::Pending {
                mapping,
                origin: MappingOrigin::Authored,
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn direct(source: IntrospectedSource, mapping: SourceMapping) -> Self {
        Self {
            source,
            mapping: RuntimeMapping::Pending {
                mapping,
                origin: MappingOrigin::Direct,
            },
        }
    }

    pub(crate) fn admitted(
        source: IntrospectedSource,
        mapping: ValidatedMapping,
    ) -> Result<Self, SemanticAdmissionError> {
        source.ensure_generation_mapping(&mapping)?;
        Ok(Self {
            source,
            mapping: RuntimeMapping::Validated(mapping),
        })
    }

    pub(crate) const fn source_id(&self) -> SourceId {
        self.mapping.source_id()
    }

    fn validate(
        self,
        ontology: &SemanticOntology,
    ) -> Result<(SourceId, IntrospectedSource, ValidatedMapping), SnapshotError> {
        let source_id = self.source_id();
        let mapping = self
            .mapping
            .validate(ontology, &self.source)
            .map_err(|cause| SnapshotError::SemanticAdmission { source_id, cause })?;
        self.source
            .ensure_generation_mapping(&mapping)
            .map_err(|cause| SnapshotError::SemanticAdmission { source_id, cause })?;
        Ok((source_id, self.source, mapping))
    }
}

impl fmt::Debug for RuntimeSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeSource")
            .field("source_id", &self.source_id())
            .field("backend_kind", &self.source.kind())
            .finish()
    }
}

/// Immutable lookup of runtime source bindings by snapshot-local [`SourceId`].
pub(crate) struct SourceRegistry {
    entries: BTreeMap<SourceId, RuntimeBinding>,
}

impl SourceRegistry {
    fn build(
        epoch: Epoch,
        ontology: &SemanticOntology,
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

        // Admission completes for every source before the first compiler or
        // cache is constructed, so federated publication is all-or-nothing.
        let validated = sources
            .into_iter()
            .map(|source| source.validate(ontology))
            .collect::<Result<Vec<_>, _>>()?;

        let entries = validated
            .into_iter()
            .map(|(source_id, source, mapping)| {
                let binding = RuntimeBinding::new(source, mapping, ontology.tbox().clone(), epoch);
                (source_id, binding)
            })
            .collect();
        Ok(Self { entries })
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn contains_source(&self, source_id: SourceId) -> bool {
        self.entries.contains_key(&source_id)
    }

    #[cfg(test)]
    pub(crate) fn source_ids(&self) -> impl Iterator<Item = SourceId> + '_ {
        self.entries.keys().copied()
    }

    #[cfg(test)]
    pub(crate) fn schema(&self, source_id: SourceId) -> Option<&[TableSchema]> {
        self.entries
            .get(&source_id)
            .map(RuntimeBinding::observed_schema)
    }

    #[cfg(test)]
    pub(crate) fn digests(&self, source_id: SourceId) -> Option<CompileDigests> {
        self.entries.get(&source_id).map(RuntimeBinding::digests)
    }

    #[cfg(test)]
    pub(crate) fn scope(&self, source_id: SourceId) -> Option<CompileScope> {
        self.entries.get(&source_id).map(RuntimeBinding::scope)
    }

    pub(crate) fn semantic_warning_count(&self, source_id: SourceId) -> Option<usize> {
        self.entries
            .get(&source_id)
            .map(RuntimeBinding::semantic_warning_count)
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
pub(crate) struct RuntimeSnapshot {
    epoch: Epoch,
    _ontology: SemanticOntology,
    ontology_digest: OntologyDigest,
    registry: SourceRegistry,
}

impl RuntimeSnapshot {
    pub(crate) fn permits_rls(&self, source: SourceId) -> bool {
        self.registry
            .binding(source)
            .is_some_and(RuntimeBinding::permits_rls)
    }
    /// Validate source identity uniqueness before constructing any bindings.
    pub(crate) fn new(
        epoch: Epoch,
        ontology: SemanticOntology,
        sources: Vec<RuntimeSource>,
    ) -> Result<Self, SnapshotError> {
        let registry = SourceRegistry::build(epoch, &ontology, sources)?;
        let ontology_digest = OntologyDigest::from_sha256(ontology.document_digest());
        #[cfg(test)]
        debug_assert!(registry
            .entries
            .values()
            .all(|binding| binding.digests().ontology() == ontology_digest));
        Ok(Self {
            epoch,
            _ontology: ontology,
            ontology_digest,
            registry,
        })
    }

    pub(crate) fn single(
        epoch: Epoch,
        ontology: SemanticOntology,
        source: RuntimeSource,
    ) -> Result<Self, SnapshotError> {
        Self::new(epoch, ontology, vec![source])
    }

    pub const fn epoch(&self) -> Epoch {
        self.epoch
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

    pub(crate) fn preflight_compile(
        &self,
        source_id: SourceId,
        sparql: &str,
        control: &dyn QueryControl,
    ) -> sf_sparql::Result<std::sync::Arc<Plan>> {
        self.registry
            .binding(source_id)
            .ok_or_else(|| {
                sf_sparql::Error::Mapping("runtime source is not registered".to_owned())
            })?
            .preflight_compile(sparql, control)
    }

    pub(crate) fn compile_secured(
        &self,
        source_id: SourceId,
        query: &str,
        budget: &crate::budget::RequestBudget,
        policy: sf_core::security_context::PolicySnapshotId,
    ) -> sf_sparql::Result<BoundPlan> {
        self.registry
            .binding(source_id)
            .ok_or_else(|| sf_sparql::Error::Mapping("source is not registered".into()))?
            .compile_secured(query, budget, policy)
    }

    pub(crate) fn compile_federated_secured(
        &self,
        source_ids: [SourceId; 2],
        query: &str,
        budget: &crate::budget::RequestBudget,
        policy: sf_core::security_context::PolicySnapshotId,
    ) -> sf_sparql::Result<BoundFederatedPlan> {
        let context = budget
            .security_context()
            .filter(|context| context.matches_policy_snapshot(policy))
            .ok_or_else(|| sf_sparql::Error::Mapping("security partition mismatch".into()))?;
        // Authoritative compilation under the generation lease, with NO raw cache
        // access. Both fragments execute under this same admitted budget/context.
        let plan = self.preflight_federated_union(source_ids, query, budget)?;
        let bindings =
            source_ids.map(|id| self.registry.binding(id).expect("compiled source exists"));
        let mut bound = BoundFederatedPlan::new(plan, bindings);
        bound.security = Some(context);
        Ok(bound)
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
        Ok(BoundFederatedPlan::new(plan, [left, right]))
    }

    pub(crate) fn preflight_federated_union(
        &self,
        source_ids: [SourceId; 2],
        sparql: &str,
        control: &dyn QueryControl,
    ) -> sf_sparql::Result<FederatedPlan> {
        if source_ids[0] == source_ids[1] {
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
        compile_source_affine_union_uncached(sparql, [left.compiler(), right.compiler()], control)
    }

    pub(crate) fn generation_requirements(
        &self,
        source_ids: impl IntoIterator<Item = SourceId>,
    ) -> Result<Vec<PgGenerationRequirement>, PgGenerationError> {
        let mut requirements = Vec::new();
        for source_id in source_ids {
            let binding = self
                .registry
                .binding(source_id)
                .ok_or(PgGenerationError::Internal)?;
            if let Some(requirement) = binding.generation_requirement()? {
                requirements.push(requirement);
            }
        }
        Ok(requirements)
    }

    pub(crate) fn prepare_federated_execution(
        &self,
        bound: BoundFederatedPlan,
    ) -> Result<ExecutableFederatedPlan, BindingMismatch> {
        let (variables, plans) = bound.into_bound_plans();
        let mut executable = Vec::with_capacity(2);
        for plan in plans {
            let binding = self
                .registry
                .binding(plan.source_id())
                .ok_or(BindingMismatch)?;
            executable.push(binding.prepare_execution(plan)?);
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
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SnapshotError {
    #[error("runtime snapshot requires at least one source")]
    EmptyRegistry,
    #[error("runtime snapshot contains duplicate {source_id}")]
    DuplicateSource { source_id: SourceId },
    #[error("runtime snapshot rejected {source_id}: {cause}")]
    SemanticAdmission {
        source_id: SourceId,
        #[source]
        cause: SemanticAdmissionError,
    },
}
