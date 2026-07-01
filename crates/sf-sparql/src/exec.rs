//! Execute — run the emitted SQL on a live SQLite source, reconstruct `oxrdf`
//! bindings/triples, and stream results (ADR-0006 *Streaming & bounded memory*;
//! ADR-0007 step 7).
//!
//! The reconstruction is the **single term-gen path** (ADR-0003 R3): the SQL
//! projects raw key columns and `sf-core`'s `generate_into` materialises the RDF
//! term per output position — terms are built here, in the outermost projection,
//! never inside a join/filter (ADR-0007 lifting). Streaming uses `sf-sql`'s
//! bounded SQLite cursor ([`sf_sql::sqlite_for_each`]) — one row in flight, so
//! memory is independent of result size. CPU-bound term-gen belongs on the
//! dedicated rayon pool ([`crate::pool`]); the sync SQLite path here generates
//! inline (no async runtime to protect — ADR-0006).

use std::future::Future;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use sf_core::{Term, Triple};

use crate::{Plan, Result};

/// A SELECT solution set — the projected-variable rows. Physically single-homed in
/// the driver-agnostic core ([`crate::exec_core`]) as of ADR-0024 M5; re-exported
/// here so `exec::Solutions` stays the stable path for the `=_bag` differential
/// harness and the PostgreSQL / MySQL executors that import it.
pub use crate::exec_core::Solutions;

/// Stream the triples of a `CONSTRUCT` (or the `?s ?p ?o` dump), invoking `sink`
/// per well-formed triple. Ill-formed instantiations (e.g. a literal subject) are
/// skipped, per SPARQL CONSTRUCT semantics. Runs the driver-agnostic
/// [`crate::exec_core`] core over a live SQLite connection (ADR-0024).
pub fn construct(plan: &Plan, conn: &Connection, sink: impl FnMut(Triple)) -> Result<u64> {
    let mut backend = sf_sql::backend::sqlite::SqliteBackend::new(conn);
    crate::exec_core::block_on(crate::exec_core::construct(plan, &mut backend, sink))
}

/// Collect a CONSTRUCT's triples (test/diagnostic convenience; the streaming
/// [`construct`] is the bounded-memory API).
pub fn construct_triples(plan: &Plan, conn: &Connection) -> Result<Vec<Triple>> {
    let mut backend = sf_sql::backend::sqlite::SqliteBackend::new(conn);
    crate::exec_core::block_on(crate::exec_core::construct_triples(plan, &mut backend))
}

/// Stream the whole mapping as **quads** (ADR-0005 named-graph conformance),
/// invoking `sink` per well-formed quad. Distinct from the `?s ?p ?o` CONSTRUCT
/// dump: it walks the mapping IR ([`crate::dump`]) so each triple carries the
/// graph term from the applicable `rr:graphMap`(s), built through the *same*
/// `sf-core` term-gen path. Bounded-memory: one row in flight via
/// [`crate::exec_core`]. A triple whose subject/predicate/object column is NULL is
/// dropped (R2RML §11); a named-graph branch whose graph map produces no value
/// drops that quad (no silent default-graph fallback).
pub fn dump_quads_stream(
    maps: &[sf_core::ir::TriplesMap],
    conn: &Connection,
    dialect: sf_sql::Dialect,
    sink: impl FnMut(sf_core::Quad),
) -> Result<()> {
    let mut backend = sf_sql::backend::sqlite::SqliteBackend::new(conn);
    crate::exec_core::block_on(crate::exec_core::dump_quads_stream(
        maps,
        &mut backend,
        dialect,
        sink,
    ))
}

/// Collect the mapping-IR quad dump (conformance convenience; the streaming
/// [`dump_quads_stream`] is the bounded-memory API).
pub fn dump_quads(
    maps: &[sf_core::ir::TriplesMap],
    conn: &Connection,
    dialect: sf_sql::Dialect,
) -> Result<Vec<sf_core::Quad>> {
    let mut backend = sf_sql::backend::sqlite::SqliteBackend::new(conn);
    crate::exec_core::block_on(crate::exec_core::dump_quads(maps, &mut backend, dialect))
}

/// Stream a SELECT's solutions, invoking `sink` per projected row (in projection
/// order, `None` = unbound) — one row in flight (bounded memory). The HTTP layer
/// drives this to serialise + flush each row without collecting (ADR-0010 §C).
pub fn select_each(
    plan: &Plan,
    conn: &Connection,
    sink: impl FnMut(&[Option<Term>]) -> Result<()>,
) -> Result<()> {
    let mut backend = sf_sql::backend::sqlite::SqliteBackend::new(conn);
    crate::exec_core::block_on(crate::exec_core::select_each(plan, &mut backend, sink))
}

/// Execute a SELECT, collecting solutions (bounded-memory streaming is the
/// [`crate::exec_core`] core; this collects for callers/tests).
pub fn select(plan: &Plan, conn: &Connection) -> Result<Solutions> {
    let mut backend = sf_sql::backend::sqlite::SqliteBackend::new(conn);
    crate::exec_core::block_on(crate::exec_core::select(plan, &mut backend))
}

/// Execute an ASK — true iff at least one solution exists.
pub fn ask(plan: &Plan, conn: &Connection) -> Result<bool> {
    let mut backend = sf_sql::backend::sqlite::SqliteBackend::new(conn);
    crate::exec_core::block_on(crate::exec_core::ask(plan, &mut backend))
}

// --- SQLite serve-lane wrappers (ADR-0024 M5, §4.1) ---------------------------
// The async, `Send`-spawnable SQLite mirrors of the PG/MySQL serve wrappers
// (`exec_pg::select_each_pg` / `exec_mysql::select_each_mysql`): each drives the
// generic core over the OWNED cap-1-bridge [`SqliteOwnedBackend`], so the
// monomorphized future is concretely `Send` and `tokio::spawn`-able by the serve
// lane (the abstract-`B` generic future is NOT provably `Send` — AFIT). Takes an
// owned `Arc<Mutex<Connection>>` (the serve backend's shape); the sync in-process
// entry points above keep the borrowing `SqliteBackend` + `block_on`.

/// Stream a SELECT's solution rows over an owned SQLite handle into an async `sink`
/// (serve lane): the cap-1 `spawn_blocking` bridge streams one row in flight and
/// `sink(..).await` backpressures it (ADR-0006 / ADR-0010 §C).
pub async fn select_each_sqlite_owned<F, Fut>(
    plan: &Plan,
    conn: Arc<Mutex<Connection>>,
    sink: F,
) -> Result<()>
where
    F: FnMut(Vec<Option<Term>>) -> Fut + Send,
    Fut: Future<Output = Result<()>> + Send,
{
    let mut b = sf_sql::backend::sqlite::SqliteOwnedBackend::new(conn);
    crate::exec_core::select_each_async(plan, &mut b, sink).await
}

/// Stream a CONSTRUCT's per-solution triples over an owned SQLite handle into an
/// async `sink` (serve lane), bounded by the template size — never the whole graph.
pub async fn construct_each_sqlite_owned<F, Fut>(
    plan: &Plan,
    conn: Arc<Mutex<Connection>>,
    sink: F,
) -> Result<()>
where
    F: FnMut(Vec<Triple>) -> Fut + Send,
    Fut: Future<Output = Result<()>> + Send,
{
    let mut b = sf_sql::backend::sqlite::SqliteOwnedBackend::new(conn);
    crate::exec_core::construct_each_async(plan, &mut b, sink).await
}

/// Execute an ASK over an owned SQLite handle (serve lane) — true iff at least one
/// solution exists. Spawnable: the concrete owned-backend future is `Send`.
pub async fn ask_sqlite_owned(plan: &Plan, conn: Arc<Mutex<Connection>>) -> Result<bool> {
    let mut b = sf_sql::backend::sqlite::SqliteOwnedBackend::new(conn);
    crate::exec_core::ask(plan, &mut b).await
}

/// Serialise triples as N-Triples 1.2 (ADR-0019 G1: triple-term graphs serialise
/// as N-Triples/Turtle, not JSON-LD). One triple per line; streamed.
pub fn write_ntriples(triples: &[Triple]) -> String {
    let mut out = String::new();
    for t in triples {
        out.push_str(&t.to_string());
        out.push_str(" .\n");
    }
    out
}
