//! Private bounded-global-operator comparison primitives.
//!
//! This module is not reachable from `sf-serve` and does not widen the accepted
//! two-arm [`super::FederatedPlan`] surface while ADR-0040 remains proposed.

mod mapping;
mod semantic_key;

// This is a private Linux comparison substrate only. It is deliberately not
// wired into planning or serving while ADR-0040 remains proposed.
#[cfg(target_os = "linux")]
mod spill;

#[cfg(test)]
mod tests;
