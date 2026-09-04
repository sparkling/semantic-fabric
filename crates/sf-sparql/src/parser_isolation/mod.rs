//! Dormant parser-worker isolation protocol foundations (ADR-0053).
//!
//! This module deliberately contains no process launcher, parser invocation,
//! query wire format, admission witness, or serving integration.

pub(crate) mod protocol;

#[cfg(test)]
mod tests;
