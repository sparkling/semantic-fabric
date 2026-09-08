//! Closed PostgreSQL Direct-Mapping lifecycle authority (ADR-0050).
//!
//! Crate-private types separate validated candidate authority from raw runtime
//! snapshot construction. Only the closed public startup path enables them.

mod builder;
mod candidate;
mod coordinator;
mod pools;

pub(crate) use builder::{InitialPgDirectGeneration, PgDirectLifecycleSpec};
pub(crate) use candidate::ValidatedRuntimeCandidate;
pub(crate) use coordinator::{
    PgDirectCoordinatorPolicy, PgDirectLifecycleSupervisor, RuntimeTransitionAuthority,
};
pub(crate) use pools::PgDirectPools;

#[cfg(test)]
pub(crate) fn sealed_candidate_for_test(
    expected: crate::RuntimeReadiness,
    snapshot: crate::RuntimeSnapshot,
) -> ValidatedRuntimeCandidate {
    ValidatedRuntimeCandidate::from_test(expected, snapshot)
}
