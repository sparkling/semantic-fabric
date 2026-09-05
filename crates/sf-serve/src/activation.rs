//! Atomic publication and request leases for immutable runtime generations.

use std::fmt;
use std::sync::{Arc, RwLock};

use sf_core::query_control::QueryControl;
use sf_core::SourceId;

use crate::binding::{
    BindingMismatch, BoundFederatedPlan, BoundPlan, ExecutableFederatedPlan, ExecutablePlan,
};
use crate::pg_generation::{PgGenerationError, PgGenerationRequirement};
use crate::snapshot::RuntimeSnapshot;

/// Monotonic process-local identity for one published runtime generation.
///
/// This is deliberately distinct from repeatable content digests: publishing
/// equivalent content again still receives a fresh identity and cannot create
/// an A-to-B-to-A ambiguity.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ActivationId(u64);

impl ActivationId {
    const INITIAL: Self = Self(1);

    pub const fn get(self) -> u64 {
        self.0
    }

    #[allow(
        dead_code,
        reason = "activation stays sealed until the validated candidate builder lands"
    )]
    fn successor(self) -> Result<Self, ActivationError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(ActivationError::ActivationIdExhausted { current: self })
    }
}

/// Closed, non-sensitive reason why new requests may not acquire a snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadinessCause {
    SchemaDrift,
    SourceUnavailable,
    CapabilityDrift,
    Administrative,
}

/// Public readiness view; it never carries source names, SQL, paths, or secrets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeReadiness {
    Ready {
        activation_id: ActivationId,
    },
    NotReady {
        activation_id: ActivationId,
        cause: ReadinessCause,
    },
}

impl RuntimeReadiness {
    pub const fn activation_id(self) -> ActivationId {
        match self {
            Self::Ready { activation_id } | Self::NotReady { activation_id, .. } => activation_id,
        }
    }
}

/// A request-owned pin of exactly one immutable application snapshot.
#[derive(Clone)]
pub(crate) struct RuntimeSnapshotLease {
    activation_id: ActivationId,
    snapshot: Arc<RuntimeSnapshot>,
}

impl RuntimeSnapshotLease {
    pub const fn activation_id(&self) -> ActivationId {
        self.activation_id
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> &RuntimeSnapshot {
        &self.snapshot
    }

    pub(crate) fn compile(
        &self,
        source_id: SourceId,
        query: &str,
        control: &dyn QueryControl,
    ) -> sf_sparql::Result<BoundPlan> {
        self.snapshot.compile(source_id, query, control)
    }

    pub(crate) fn preflight_compile(
        &self,
        source_id: SourceId,
        query: &str,
        control: &dyn QueryControl,
    ) -> sf_sparql::Result<std::sync::Arc<sf_sparql::Plan>> {
        self.snapshot.preflight_compile(source_id, query, control)
    }

    pub(crate) fn prepare_execution(
        &self,
        plan: BoundPlan,
    ) -> Result<ExecutablePlan, BindingMismatch> {
        self.snapshot.prepare_execution(plan)
    }

    pub(crate) fn compile_federated_union(
        &self,
        source_ids: [SourceId; 2],
        query: &str,
        control: &dyn QueryControl,
    ) -> sf_sparql::Result<BoundFederatedPlan> {
        self.snapshot
            .compile_federated_union(source_ids, query, control)
    }

    pub(crate) fn preflight_federated_union(
        &self,
        source_ids: [SourceId; 2],
        query: &str,
        control: &dyn QueryControl,
    ) -> sf_sparql::Result<sf_sparql::federation::FederatedPlan> {
        self.snapshot
            .preflight_federated_union(source_ids, query, control)
    }

    pub(crate) fn generation_requirements(
        &self,
        source_ids: impl IntoIterator<Item = SourceId>,
    ) -> Result<Vec<PgGenerationRequirement>, PgGenerationError> {
        self.snapshot.generation_requirements(source_ids)
    }

    pub(crate) fn prepare_federated_execution(
        &self,
        plan: BoundFederatedPlan,
    ) -> Result<ExecutableFederatedPlan, BindingMismatch> {
        self.snapshot.prepare_federated_execution(plan)
    }

    #[cfg(test)]
    pub(crate) fn weak_snapshot(&self) -> std::sync::Weak<RuntimeSnapshot> {
        Arc::downgrade(&self.snapshot)
    }
}

impl fmt::Debug for RuntimeSnapshotLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeSnapshotLease")
            .field("activation_id", &self.activation_id)
            .field("epoch", &self.snapshot.epoch())
            .field("ontology_digest", &self.snapshot.ontology_digest())
            .finish()
    }
}

enum RuntimeState {
    Ready {
        activation_id: ActivationId,
        snapshot: Arc<RuntimeSnapshot>,
    },
    NotReady {
        activation_id: ActivationId,
        cause: ReadinessCause,
    },
}

impl RuntimeState {
    const fn activation_id(&self) -> ActivationId {
        match self {
            Self::Ready { activation_id, .. } | Self::NotReady { activation_id, .. } => {
                *activation_id
            }
        }
    }

    const fn readiness(&self) -> RuntimeReadiness {
        match self {
            Self::Ready { activation_id, .. } => RuntimeReadiness::Ready {
                activation_id: *activation_id,
            },
            Self::NotReady {
                activation_id,
                cause,
            } => RuntimeReadiness::NotReady {
                activation_id: *activation_id,
                cause: *cause,
            },
        }
    }
}

/// Single owner of runtime readiness and whole-generation publication.
///
/// Candidate construction is complete before [`Self::activate`] is called. The
/// final write-lock section only compares the expected identity and replaces the
/// complete state, so readers observe either the old or new generation.
pub(crate) struct RuntimeManager {
    state: RwLock<RuntimeState>,
}

impl RuntimeManager {
    pub(crate) fn new(initial: RuntimeSnapshot) -> Self {
        Self {
            state: RwLock::new(RuntimeState::Ready {
                activation_id: ActivationId::INITIAL,
                snapshot: Arc::new(initial),
            }),
        }
    }

    /// Load readiness exactly once and pin the corresponding snapshot.
    pub(crate) fn lease(&self) -> Result<RuntimeSnapshotLease, SnapshotUnavailable> {
        let state = self
            .state
            .read()
            .map_err(|_| SnapshotUnavailable::StatePoisoned)?;
        match &*state {
            RuntimeState::Ready {
                activation_id,
                snapshot,
            } => Ok(RuntimeSnapshotLease {
                activation_id: *activation_id,
                snapshot: snapshot.clone(),
            }),
            RuntimeState::NotReady {
                activation_id,
                cause,
            } => Err(SnapshotUnavailable::NotReady {
                activation_id: *activation_id,
                cause: *cause,
            }),
        }
    }

    pub(crate) fn readiness(&self) -> Result<RuntimeReadiness, ActivationError> {
        self.state
            .read()
            .map(|state| state.readiness())
            .map_err(|_| ActivationError::StatePoisoned)
    }

    /// Publish a prebuilt candidate if the complete expected readiness state is
    /// still current. This private primitive does not validate or authorize the
    /// candidate; the future candidate builder must do so before calling it.
    #[allow(
        dead_code,
        reason = "activation stays sealed until the validated candidate builder lands"
    )]
    pub(crate) fn activate(
        &self,
        expected: RuntimeReadiness,
        candidate: RuntimeSnapshot,
    ) -> Result<ActivationId, ActivationError> {
        let next = expected.activation_id().successor()?;
        let candidate = Arc::new(candidate);
        let mut state = self
            .state
            .write()
            .map_err(|_| ActivationError::StatePoisoned)?;
        let actual = state.readiness();
        if actual != expected {
            return Err(ActivationError::StaleState { expected, actual });
        }
        let previous = std::mem::replace(
            &mut *state,
            RuntimeState::Ready {
                activation_id: next,
                snapshot: candidate,
            },
        );
        drop(state);
        drop(previous);
        Ok(next)
    }

    /// Stop new request leases after generation-bound drift or an explicit
    /// administrative transition. Existing leases remain valid.
    pub(crate) fn mark_not_ready(
        &self,
        expected: ActivationId,
        cause: ReadinessCause,
    ) -> Result<(), ActivationError> {
        let mut state = self
            .state
            .write()
            .map_err(|_| ActivationError::StatePoisoned)?;
        let actual = state.activation_id();
        if actual != expected {
            return Err(ActivationError::StaleGeneration { expected, actual });
        }
        if matches!(&*state, RuntimeState::NotReady { .. }) {
            return Err(ActivationError::AlreadyNotReady {
                activation_id: actual,
            });
        }
        let previous = std::mem::replace(
            &mut *state,
            RuntimeState::NotReady {
                activation_id: actual,
                cause,
            },
        );
        drop(state);
        drop(previous);
        Ok(())
    }
}

impl fmt::Debug for RuntimeManager {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeManager")
            .field("readiness", &self.readiness())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum SnapshotUnavailable {
    #[error("runtime generation is not ready")]
    NotReady {
        activation_id: ActivationId,
        cause: ReadinessCause,
    },
    #[error("runtime generation state is unavailable")]
    StatePoisoned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ActivationError {
    #[error("runtime activation expected state {expected:?} but found {actual:?}")]
    StaleState {
        expected: RuntimeReadiness,
        actual: RuntimeReadiness,
    },
    #[error("runtime activation expected {expected:?} but found {actual:?}")]
    StaleGeneration {
        expected: ActivationId,
        actual: ActivationId,
    },
    #[error("runtime activation identity is exhausted at {current:?}")]
    ActivationIdExhausted { current: ActivationId },
    #[error("runtime generation {activation_id:?} is already not ready")]
    AlreadyNotReady { activation_id: ActivationId },
    #[error("runtime candidate does not contain selected {source_id}")]
    CandidateMissingSource { source_id: SourceId },
    #[error("runtime activation state is unavailable")]
    StatePoisoned,
}
