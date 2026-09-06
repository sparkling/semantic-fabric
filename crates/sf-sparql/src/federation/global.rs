//! Private bounded-global-operator comparison primitives.
//!
//! This module is not reachable from `sf-serve` and does not widen the accepted
//! two-arm [`super::FederatedPlan`] surface while ADR-0040 remains proposed.

mod mapping;
mod semantic_key;

#[cfg(test)]
mod tests;
