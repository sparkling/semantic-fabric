//! Dormant parser-worker isolation protocol foundations (ADR-0053).
//!
//! The Linux supervisor foundation is deliberately dormant: no production
//! worker entry point calls it. Parser invocation, query wire format, admission
//! witness, permits, and serving integration remain absent.

pub(crate) mod protocol;
mod supervisor;

#[cfg(test)]
mod tests;
