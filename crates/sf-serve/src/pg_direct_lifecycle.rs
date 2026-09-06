//! Closed PostgreSQL Direct-Mapping lifecycle authority (ADR-0050).
//!
//! This module remains private until the complete profile passes independent
//! admission review. Its types separate validated candidate authority from raw
//! runtime snapshot construction.

#![allow(
    dead_code,
    reason = "profile remains dormant until its independent admission review"
)]

mod builder;
mod candidate;
mod coordinator;
mod pools;

#[allow(
    unused_imports,
    reason = "dormant profile entry points await admission review"
)]
pub(crate) use builder::{
    InitialPgDirectGeneration, PgDirectLifecycleBuild, PgDirectLifecycleSpec,
};
pub(crate) use candidate::ValidatedRuntimeCandidate;
#[allow(
    unused_imports,
    reason = "dormant supervisor entry points await admission review"
)]
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
