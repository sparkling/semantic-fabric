//! `sf-serve` — the SPARQL 1.2 **Protocol** HTTP endpoint over the OBDA
//! virtualiser (ADR-0019 G8: own the 1.2 query endpoint — Oxigraph ships only a
//! 1.1 server binary). Read-only (query operation only; no update).
//!
//! Per request: extract the query (GET `?query=`, POST form `query=`, or a raw
//! `application/sparql-query` body) → [`parse_and_translate_with`] against the
//! configured mapping `M` + T-Box `T` + dialect (the rewriter, off the async
//! runtime via `spawn_blocking`, ADR-0006) → execute over the configured backend →
//! serialise the negotiated form, **streaming** the bytes into the response body
//! (ADR-0010 §C; [`stream`]). Values stay bound parameters end to end — the
//! rewriter/executors never interpolate (ADR-0010 R1).
//!
//! Governance (ADR-0010): one configurable absolute request deadline spans body
//! extraction, admitted compilation, pool wait, async execution, and serialisation;
//! plus a max-query-length cap and producer cancel-on-client-drop. This partial-M2
//! clock is not source-statement cancellation, SQLite interruptibility, or a
//! row/byte/work budget. Error → status mapping: parse → 400, unsupported
//! feature → 501, execution → 500, success → 200.

pub mod ontology;
pub mod run;
pub mod stream;

mod admission;
mod backend;
mod config;
mod deadline;
mod http;
mod problem;

#[cfg(test)]
mod deadline_tests;

pub use backend::{introspect_pg_all, introspect_sqlite_all, Backend, SqlitePool};
pub use config::ServeConfig;
pub use http::router;
pub use ontology::tbox_from_turtle;
pub use problem::ServeError;
pub use run::{serve_blocking, ServeOptions};
pub use stream::RdfFormat;
