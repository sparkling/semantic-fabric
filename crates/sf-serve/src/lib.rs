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
//!
//! Governance (ADR-0010): one request budget spans body extraction, admitted
//! compilation, pool wait, controlled execution, and serialisation. It combines
//! an absolute deadline with finite observable source-work, semantic-result, and
//! serialized-byte ceilings, plus producer cancellation on client drop. It does
//! not count compiler CPU or recursive SQL work and is not source-native statement
//! cancellation, SQLite interruptibility, or atomic streamed failure. Pre-response
//! policy limits map to 429; every post-200 failure stays a redacted body error.

pub mod ontology;
pub mod run;
pub mod source;
pub mod stream;

mod admission;
mod backend;
mod binding;
mod budget;
mod config;
mod deadline;
mod http;
mod problem;

#[cfg(test)]
mod deadline_tests;
#[cfg(test)]
mod query_budget_tests;

pub use backend::{introspect_pg_all, introspect_sqlite_all, Backend, BackendKind, SqlitePool};
pub use binding::{BackendProfile, IntrospectedSource};
pub use config::{ServeConfig, DEFAULT_QUERY_LIMITS};
pub use http::router;
pub use ontology::tbox_from_turtle;
pub use problem::ServeError;
pub use run::{serve_blocking, ServeOptions};
pub use source::{SourceInput, SourceRef, MAX_SOURCE_ENV_NAME_BYTES, MAX_SOURCE_INPUT_BYTES};
pub use stream::RdfFormat;
