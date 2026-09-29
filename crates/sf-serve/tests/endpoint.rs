//! End-to-end tests for the SPARQL 1.2 Protocol endpoint (ADR-0019 G8), driven
//! in-process via `tower::ServiceExt::oneshot` (no real socket). The SQLite suite
//! runs on an in-memory fixture; the PostgreSQL variant gate-skips when no server
//! is reachable on localhost:5432.

#[path = "endpoint/admission.rs"]
mod admission;
#[path = "endpoint/concurrency.rs"]
mod concurrency;
#[path = "endpoint/fixtures.rs"]
mod fixtures;
#[path = "endpoint/formats.rs"]
mod formats;
#[path = "endpoint/protocol.rs"]
mod protocol;
mod support;
