//! Dormant parser-worker isolation protocol foundations (ADR-0053).
//!
//! The private invocation discriminator fails closed before CLI parsing, but
//! every reserved invocation still terminates at an unavailable worker stub.
//! Parser invocation, final confinement, query wire format, admission witness,
//! permits, and serving integration remain absent.

pub(crate) mod protocol;
mod supervisor;
mod worker;

pub use worker::dispatch_private_parser_worker_v1;

#[cfg(test)]
mod tests;
