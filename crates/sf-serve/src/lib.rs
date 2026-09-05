//! `sf-serve` — the SPARQL 1.2 **Protocol** HTTP endpoint over the OBDA
//! virtualiser (ADR-0019 G8: own the 1.2 query endpoint — Oxigraph ships only a
//! 1.1 server binary). Read-only (query operation only; no update).
//!
//! Per request: extract the query (GET `?query=`, POST form `query=`, or a raw
//! `application/sparql-query` body) → compile through the source-bound cached
//! [`sf_sparql::CompilerBinding`] against mapping `M`, T-Box `T`, dialect, and a
//! constraint-quarantined compiler schema (off the async runtime via
//! `spawn_blocking`, ADR-0006/0007) → ownership-check and execute over the bound
//! backend → serialise the negotiated form, **streaming** the bytes into the
//! response body (ADR-0010 §C; [`stream`]). Values stay bound parameters end to
//! end—the rewriter/executors never interpolate (ADR-0010 R1).
//! An exact query-less `GET`/`HEAD /sparql` returns a fixed, redacted W3C Service
//! Description without acquiring query capacity or a runtime/source lease.
//!
//! Governance (ADR-0010): one request budget spans body extraction, admitted
//! compilation, pool wait, controlled execution, and serialisation. It combines
//! an absolute deadline with finite observable source-work, semantic-result, and
//! serialized-byte ceilings, plus producer cancellation on client drop. A
//! server-wide fail-fast gate bounds requests admitted into application work;
//! its permit follows that budget through active producers and detached blocking
//! workers, but not through already-produced body bytes. Stream terminal errors
//! use an out-of-band outcome so a full client channel cannot park cleanup. This
//! does not count compiler CPU or recursive SQL work and does not provide a
//! common source-native cancellation or atomic streamed-failure contract. The owned
//! SQLite serving path cancels its per-connection admission wait and interrupts
//! active VM work after mutex acquisition, but not raw mutex or submitted Tokio
//! blocking-task waits, busy timeouts, blocking UDF/VFS/I/O, compiler work,
//! raw/conformance paths, or committed response prefixes. Pre-response policy
//! limits map to 429; every post-200 failure stays a redacted body error.

pub mod ontology;
pub mod run;
pub mod source;
pub mod stream;

mod activation;
mod admission;
mod backend;
mod binding;
mod budget;
mod config;
mod deadline;
mod federation;
mod health;
mod http;
mod lifecycle;
mod observed_source;
mod pg_generation;
mod pg_response;
mod post_body;
mod problem;
mod request_compile;
mod request_deadline;
mod request_generation;
mod semantic_admission;
mod service_description;
mod snapshot;
mod source_acquisition;
mod sqlite_admission;
mod startup;
mod terminal_body;

#[cfg(test)]
mod sqlite_admission_tests;

#[cfg(test)]
mod budget_tests;
#[cfg(test)]
mod deadline_tests;
#[cfg(test)]
mod federated_union_tests;
#[cfg(test)]
mod health_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod query_budget_tests;
#[cfg(test)]
mod request_admission_tests;
#[cfg(test)]
mod runtime_activation_http_tests;
#[cfg(test)]
mod runtime_snapshot_tests;
#[cfg(test)]
mod semantic_admission_tests;
#[cfg(test)]
mod test_support;

pub use activation::{ActivationError, ActivationId, ReadinessCause, RuntimeReadiness};
pub use backend::{introspect_pg_all, introspect_sqlite_all, Backend, BackendKind, SqlitePool};
pub use binding::BackendProfile;
pub use config::{
    ServeConfig, DEFAULT_MAX_CONCURRENT_REQUESTS, DEFAULT_MAX_ORDER_BYTES, DEFAULT_MAX_ORDER_ROWS,
    DEFAULT_QUERY_LIMITS,
};
pub use http::router;
pub use lifecycle::DEFAULT_SHUTDOWN_TIMEOUT;
pub use observed_source::IntrospectedSource;
pub use ontology::{tbox_from_turtle, SemanticOntology};
pub use problem::ServeError;
pub use request_deadline::{RequestDeadlineMakeService, RequestDeadlineService};
pub use run::{serve_blocking, AdditionalSourceOptions, MappingRef, ServeOptions};
pub use semantic_admission::SemanticAdmissionError;
#[cfg(test)]
pub(crate) use snapshot::RuntimeSnapshot;
pub(crate) use snapshot::RuntimeSource;
pub use snapshot::SnapshotError;
pub use source::{SourceInput, SourceRef, MAX_SOURCE_ENV_NAME_BYTES, MAX_SOURCE_INPUT_BYTES};
pub use stream::RdfFormat;
