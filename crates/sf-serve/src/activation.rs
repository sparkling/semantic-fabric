//! Atomic publication and request leases for immutable runtime generations.

use std::fmt;
use std::sync::{Arc, RwLock};

use sf_core::query_control::QueryControl;
use sf_core::SourceId;

use crate::binding::{
    BindingMismatch, BoundFederatedPlan, BoundPlan, ExecutableFederatedPlan, ExecutablePlan,
};
use crate::generation::GenerationRequirement;
use crate::pg_direct_lifecycle::{RuntimeTransitionAuthority, ValidatedRuntimeCandidate};
use crate::pg_generation::PgGenerationError;
use crate::snapshot::RuntimeSnapshot;
#[path = "activation_reload.rs"]
mod reload;
#[path = "activation_security.rs"]
mod security;

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

    fn successor(self) -> Result<Self, ActivationError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(ActivationError::ActivationIdExhausted { current: self })
    }
}

/// Opaque process-local ordering token for every runtime-state transition.
///
/// Unlike [`ActivationId`], this advances when readiness changes without publishing a new snapshot.
/// Callers can retain and compare the token but cannot construct or advance it.
///
/// ```compile_fail
/// let _forged = sf_serve::RuntimeStateRevision(7);
/// ```
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RuntimeStateRevision(u64);

impl RuntimeStateRevision {
    const INITIAL: Self = Self(1);

    fn successor(self) -> Result<Self, ActivationError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(ActivationError::StateRevisionExhausted { current: self })
    }
}

/// Closed, non-sensitive reason why new requests may not acquire a snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadinessCause {
    SchemaDrift,
    SourceUnavailable,
    CapabilityDrift,
    Administrative,
    StateRevisionExhausted,
}

/// Public readiness view; it never carries source names, SQL, paths, or secrets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeReadiness {
    Ready {
        activation_id: ActivationId,
        revision: RuntimeStateRevision,
    },
    NotReady {
        activation_id: ActivationId,
        revision: RuntimeStateRevision,
        cause: ReadinessCause,
    },
}

impl RuntimeReadiness {
    pub const fn activation_id(self) -> ActivationId {
        match self {
            Self::Ready { activation_id, .. } | Self::NotReady { activation_id, .. } => {
                activation_id
            }
        }
    }

    pub const fn state_revision(self) -> RuntimeStateRevision {
        match self {
            Self::Ready { revision, .. } | Self::NotReady { revision, .. } => revision,
        }
    }
}

/// A request-owned pin of exactly one immutable application snapshot.
#[derive(Clone)]
pub(crate) struct RuntimeSnapshotLease {
    activation_id: ActivationId,
    state_revision: RuntimeStateRevision,
    snapshot: Arc<RuntimeSnapshot>,
}

impl RuntimeSnapshotLease {
    pub const fn activation_id(&self) -> ActivationId {
        self.activation_id
    }

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
    ) -> Result<Vec<GenerationRequirement>, PgGenerationError> {
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
            .field("state_revision", &self.state_revision)
            .field("epoch", &self.snapshot.epoch())
            .field("ontology_digest", &self.snapshot.ontology_digest())
            .finish()
    }
}

enum RuntimeState {
    Ready {
        activation_id: ActivationId,
        revision: RuntimeStateRevision,
        snapshot: Arc<RuntimeSnapshot>,
    },
    NotReady {
        activation_id: ActivationId,
        revision: RuntimeStateRevision,
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

    const fn state_revision(&self) -> RuntimeStateRevision {
        match self {
            Self::Ready { revision, .. } | Self::NotReady { revision, .. } => *revision,
        }
    }

    const fn readiness(&self) -> RuntimeReadiness {
        match self {
            Self::Ready {
                activation_id,
                revision,
                ..
            } => RuntimeReadiness::Ready {
                activation_id: *activation_id,
                revision: *revision,
            },
            Self::NotReady {
                activation_id,
                revision,
                cause,
            } => RuntimeReadiness::NotReady {
                activation_id: *activation_id,
                revision: *revision,
                cause: *cause,
            },
        }
    }

    fn not_ready_transition(&self, cause: ReadinessCause) -> (Self, Option<ActivationError>) {
        match self.state_revision().successor() {
            Ok(revision) => (
                Self::NotReady {
                    activation_id: self.activation_id(),
                    revision,
                    cause,
                },
                None,
            ),
            Err(error) => (
                Self::NotReady {
                    activation_id: self.activation_id(),
                    revision: self.state_revision(),
                    cause: ReadinessCause::StateRevisionExhausted,
                },
                Some(error),
            ),
        }
    }
}

/// Single owner of runtime readiness and whole-generation publication.
///
/// Candidate construction is complete before [`Self::activate_candidate`] is called. The
/// final write-lock section compares the complete expected state and replaces
/// it, so readers observe either the old or new generation.
pub(crate) struct RuntimeManager {
    state: RwLock<RuntimeState>,
}

impl RuntimeManager {
    pub(crate) fn new(initial: RuntimeSnapshot) -> Self {
        Self {
            state: RwLock::new(RuntimeState::Ready {
                activation_id: ActivationId::INITIAL,
                revision: RuntimeStateRevision::INITIAL,
                snapshot: Arc::new(initial),
            }),
        }
    }

    /// Read the admitted warning count from the currently published snapshot.
    /// `None` means that no ready snapshot currently exposes this source.
    pub(crate) fn semantic_warning_count(&self, source_id: SourceId) -> Option<usize> {
        let state = self.state.read().ok()?;
        match &*state {
            RuntimeState::Ready { snapshot, .. } => {
                snapshot.registry().semantic_warning_count(source_id)
            }
            RuntimeState::NotReady { .. } => None,
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
                revision,
                snapshot,
            } => Ok(RuntimeSnapshotLease {
                activation_id: *activation_id,
                state_revision: *revision,
                snapshot: snapshot.clone(),
            }),
            RuntimeState::NotReady {
                activation_id,
                cause,
                ..
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

    /// Publish a sealed candidate only against its complete expected readiness.
    /// Both arguments are lifecycle-only capabilities request paths cannot mint.
    pub(crate) fn activate_candidate(
        &self,
        _authority: &RuntimeTransitionAuthority,
        candidate: ValidatedRuntimeCandidate,
    ) -> Result<ActivationId, ActivationError> {
        let expected = candidate.expected();
        self.publish(expected, candidate.into_snapshot())
    }

    fn publish(
        &self,
        expected: RuntimeReadiness,
        candidate: RuntimeSnapshot,
    ) -> Result<ActivationId, ActivationError> {
        if matches!(
            expected,
            RuntimeReadiness::NotReady {
                cause: ReadinessCause::Administrative | ReadinessCause::StateRevisionExhausted,
                ..
            }
        ) {
            return Err(ActivationError::ShuttingDown);
        }
        let candidate = Arc::new(candidate);
        let mut state = self
            .state
            .write()
            .map_err(|_| ActivationError::StatePoisoned)?;
        let actual = state.readiness();
        if actual != expected {
            return Err(ActivationError::StaleState { expected, actual });
        }
        let next = actual.activation_id().successor()?;
        let revision = actual.state_revision().successor()?;
        let previous = std::mem::replace(
            &mut *state,
            RuntimeState::Ready {
                activation_id: next,
                revision,
                snapshot: candidate,
            },
        );
        drop(state);
        drop(previous);
        Ok(next)
    }

    /// Test-only raw bridge; production requires the sealed lifecycle candidate.
    #[cfg(test)]
    pub(crate) fn activate_test_snapshot(
        &self,
        expected: RuntimeReadiness,
        snapshot: RuntimeSnapshot,
    ) -> Result<ActivationId, ActivationError> {
        self.activate_candidate(
            &RuntimeTransitionAuthority::for_test(),
            crate::pg_direct_lifecycle::sealed_candidate_for_test(expected, snapshot),
        )
    }

    /// Fence new leases after observed drift; existing leases remain valid.
    pub(crate) fn transition_not_ready(
        &self,
        _authority: &RuntimeTransitionAuthority,
        expected: RuntimeReadiness,
        cause: ReadinessCause,
    ) -> Result<RuntimeReadiness, ActivationError> {
        self.fence(expected, cause)
    }

    fn fence(
        &self,
        expected: RuntimeReadiness,
        cause: ReadinessCause,
    ) -> Result<RuntimeReadiness, ActivationError> {
        let mut state = self
            .state
            .write()
            .map_err(|_| ActivationError::StatePoisoned)?;
        let actual = state.readiness();
        if actual != expected {
            return Err(ActivationError::StaleState { expected, actual });
        }
        let (next, error) = state.not_ready_transition(cause);
        let readiness = next.readiness();
        let previous = std::mem::replace(&mut *state, next);
        drop(state);
        drop(previous);
        error.map_or(Ok(readiness), Err)
    }

    #[cfg(test)]
    pub(crate) fn mark_not_ready(
        &self,
        expected: RuntimeReadiness,
        cause: ReadinessCause,
    ) -> Result<RuntimeReadiness, ActivationError> {
        self.transition_not_ready(&RuntimeTransitionAuthority::for_test(), expected, cause)
    }

    /// Establish an administrative fence against whichever generation and
    /// readiness event is current at the write-lock linearization point.
    pub(crate) fn mark_current_administratively_not_ready(
        &self,
    ) -> Result<RuntimeReadiness, ActivationError> {
        let mut state = self
            .state
            .write()
            .map_err(|_| ActivationError::StatePoisoned)?;
        let (next, error) = state.not_ready_transition(ReadinessCause::Administrative);
        let readiness = next.readiness();
        let previous = std::mem::replace(&mut *state, next);
        drop(state);
        drop(previous);
        error.map_or(Ok(readiness), Err)
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
    #[error("runtime transition expected state {expected:?} but found {actual:?}")]
    StaleState {
        expected: RuntimeReadiness,
        actual: RuntimeReadiness,
    },
    #[error("runtime activation identity is exhausted at {current:?}")]
    ActivationIdExhausted { current: ActivationId },
    #[error("runtime state revision is exhausted at {current:?}")]
    StateRevisionExhausted { current: RuntimeStateRevision },
    #[error("runtime candidate does not contain selected {source_id}")]
    CandidateMissingSource { source_id: SourceId },
    #[error("runtime transition is unavailable during shutdown")]
    ShuttingDown,
    #[error("runtime activation state is unavailable")]
    StatePoisoned,
}

#[cfg(test)]
#[path = "activation/tests.rs"]
mod tests;
