//! DuckDB execution path (ADR-0024 M8): thin public delegators from a live
//! embedded DuckDB connection to the driver-agnostic execution core.
//!
//! The owned [`DuckDbBackend`] moves every synchronous cursor onto a bounded
//! `spawn_blocking` bridge. One row is in flight, bound parameters are never
//! interpolated, and dropping an unfinished stream interrupts and joins its
//! worker. This module is available with the `duckdb-backend` feature.

use std::future::Future;
use std::sync::{Arc, Mutex};

use duckdb::Connection;
use sf_core::{Quad, Term, Triple};
use sf_sql::backend::duckdb::DuckDbBackend;
use sf_sql::Dialect;

use crate::exec::Solutions;
use crate::{Plan, Result};

/// Default cap for one decoded DuckDB value on the public execution path.
pub const MAX_VALUE_BYTES: usize = 16 * 1024 * 1024;

fn backend(conn: Arc<Mutex<Connection>>) -> DuckDbBackend {
    DuckDbBackend::new(conn)
        .with_max_value_bytes(MAX_VALUE_BYTES)
        .with_max_row_bytes(MAX_VALUE_BYTES)
}

/// Execute a SELECT over DuckDB and collect its projected solutions.
pub async fn select_duckdb(plan: &Plan, conn: Arc<Mutex<Connection>>) -> Result<Solutions> {
    let mut backend = backend(conn);
    crate::exec_core::select(plan, &mut backend).await
}

/// Execute an ASK over DuckDB.
pub async fn ask_duckdb(plan: &Plan, conn: Arc<Mutex<Connection>>) -> Result<bool> {
    let mut backend = backend(conn);
    crate::exec_core::ask(plan, &mut backend).await
}

/// Execute a CONSTRUCT over DuckDB and collect its triples.
pub async fn construct_triples_duckdb(
    plan: &Plan,
    conn: Arc<Mutex<Connection>>,
) -> Result<Vec<Triple>> {
    let mut backend = backend(conn);
    crate::exec_core::construct_triples(plan, &mut backend).await
}

/// Collect the mapping-IR quad dump over DuckDB.
pub async fn dump_quads_duckdb(
    maps: &[sf_core::ir::TriplesMap],
    conn: Arc<Mutex<Connection>>,
    dialect: Dialect,
) -> Result<Vec<Quad>> {
    let mut backend = backend(conn);
    crate::exec_core::dump_quads(maps, &mut backend, dialect).await
}

/// Stream a SELECT through the owned DuckDB bridge into an async sink.
pub async fn select_each_duckdb<F, Fut>(
    plan: &Plan,
    conn: Arc<Mutex<Connection>>,
    sink: F,
) -> Result<()>
where
    F: FnMut(Vec<Option<Term>>) -> Fut + Send,
    Fut: Future<Output = Result<()>> + Send,
{
    let mut backend = backend(conn);
    crate::exec_core::select_each_async(plan, &mut backend, sink).await
}

/// Stream a CONSTRUCT through the owned DuckDB bridge into an async sink.
pub async fn construct_each_duckdb<F, Fut>(
    plan: &Plan,
    conn: Arc<Mutex<Connection>>,
    sink: F,
) -> Result<()>
where
    F: FnMut(Vec<Triple>) -> Fut + Send,
    Fut: Future<Output = Result<()>> + Send,
{
    let mut backend = backend(conn);
    crate::exec_core::construct_each_async(plan, &mut backend, sink).await
}
