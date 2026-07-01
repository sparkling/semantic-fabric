//! MySQL execution path (WS-D dialect breadth; ADR-0006 *Streaming & bounded memory*;
//! ADR-0015): run the emitted MySQL SQL over a **live** server through the
//! driver-agnostic execution core ([`crate::exec_core`], generic over
//! [`sf_sql::SqlBackend`]) with the MySQL adapter ([`MysqlBackend`]). The adapter
//! opens a packet-streamed `exec_iter` cursor ([`sf_sql::backend::mysql`] — never
//! the buffer-all `exec()`), marshals each row via `mysql_value_to_string`, and
//! reconstructs `oxrdf` terms through the **single** sf-core term-gen path shared
//! with the SQLite and PostgreSQL executors ([`crate::exec`], ADR-0003 R3).
//!
//! Since ADR-0024 M4 this file is a thin set of delegators: the branches loop,
//! `rust_group` dispatch (q9), DISTINCT dedup (q15), ORDER/OFFSET/LIMIT, and the §10
//! catalog probe all live once in [`crate::exec_core`]; the per-cell
//! `mysql_value_to_string` decode and positional bind live in
//! [`sf_sql::backend::mysql`]. The public collecting entry-point signatures are
//! unchanged, so every caller (the `mysql_e2e` integration suite) is untouched; the
//! two `*_each_mysql` streaming forms are new (the §4.2 dedicated-connection serve
//! lane, MySQL's first).
//!
//! MySQL's text protocol does not expose a server-side cursor equivalent to
//! PostgreSQL's `query_raw`; `mysql_async` fetches the result in wire packets and
//! delivers rows asynchronously. For virtualisation workloads (read-only, bounded by
//! mapping size × result pages) this satisfies ADR-0010 §C's bounded-memory
//! requirement at the packet level.
//!
//! Values are **bound parameters only** (ADR-0010 R1): the emitted `?` placeholders
//! are filled from `EmittedBranch::params`, never interpolated into SQL.
//!
//! Requires a live server; exercised by the integration suite (ADR-0012).

use std::future::Future;

use mysql_async::Conn;
use sf_core::{Quad, Term, Triple};
use sf_sql::backend::mysql::MysqlBackend;
use sf_sql::Dialect;

use crate::exec::Solutions;
use crate::{Plan, Result};

/// Execute a SELECT over a live MySQL connection, collecting solutions —
/// the async mirror of the sync SQLite [`crate::exec::select`] (ADR-0003 R3: the
/// SAME reconstruction). Bounded-memory streaming is the [`crate::exec_core`] core
/// (packet-streamed `exec_iter`, one row in flight); this collects the projected
/// rows for callers/tests.
pub async fn select_mysql(plan: &Plan, conn: &mut Conn) -> Result<Solutions> {
    let mut b = MysqlBackend::new(conn);
    crate::exec_core::select(plan, &mut b).await
}

/// Execute an ASK over a live MySQL connection — true iff at least one solution
/// exists. Same streaming core, same reconstruction.
pub async fn ask_mysql(plan: &Plan, conn: &mut Conn) -> Result<bool> {
    let mut b = MysqlBackend::new(conn);
    crate::exec_core::ask(plan, &mut b).await
}

/// Collect a CONSTRUCT's triples over a live MySQL connection. Streaming is the
/// bounded-memory core ([`crate::exec_core::construct_triples`]); this collects for
/// the integration harness.
pub async fn construct_triples_mysql(plan: &Plan, conn: &mut Conn) -> Result<Vec<Triple>> {
    let mut b = MysqlBackend::new(conn);
    crate::exec_core::construct_triples(plan, &mut b).await
}

/// Collect the mapping-IR **quad** dump over a live MySQL connection (ADR-0005
/// named-graph conformance) — each triple carries the graph term from the applicable
/// `rr:graphMap`(s), built through the same sf-core term-gen path.
pub async fn dump_quads_mysql(
    maps: &[sf_core::ir::TriplesMap],
    conn: &mut Conn,
    dialect: Dialect,
) -> Result<Vec<Quad>> {
    let mut b = MysqlBackend::new(conn);
    crate::exec_core::dump_quads(maps, &mut b, dialect).await
}

/// Stream a SELECT over a **dedicated** MySQL connection (design §4.2): the owned
/// `Conn` is held for the stream's full lifetime and discarded/reset on early drop
/// (LIMIT/deadline/client-gone). `sink(..).await` runs per projected row.
///
/// Takes an owned `Conn` (not `&mut Conn`) because this future is `tokio::spawn`ed by
/// the serve lane: an owned, `'static` backend ([`MysqlBackend<Conn>`]) is what lets
/// the generic core's `Send` bound (`for<'s> B::Stream<'s>: Send`, ADR-0024 §1.103)
/// hold across the spawn.
pub async fn select_each_mysql<F, Fut>(plan: &Plan, conn: Conn, sink: F) -> Result<()>
where
    F: FnMut(Vec<Option<Term>>) -> Fut + Send,
    Fut: Future<Output = Result<()>> + Send,
{
    let mut b = MysqlBackend::new(conn);
    crate::exec_core::select_each_async(plan, &mut b, sink).await
}

/// Stream a CONSTRUCT's per-solution triples over a dedicated MySQL connection
/// (design §4.2), bounded by the template size — never the whole graph.
pub async fn construct_each_mysql<F, Fut>(plan: &Plan, conn: Conn, sink: F) -> Result<()>
where
    F: FnMut(Vec<Triple>) -> Fut + Send,
    Fut: Future<Output = Result<()>> + Send,
{
    let mut b = MysqlBackend::new(conn);
    crate::exec_core::construct_each_async(plan, &mut b, sink).await
}

/// Execute an ASK over a **dedicated** owned MySQL connection (serve lane) — true
/// iff at least one solution exists.
///
/// Takes an owned `Conn` (not `&mut Conn` like [`ask_mysql`]) because this future is
/// `tokio::spawn`ed by the serve lane. MySQL's branch cursor BORROWS the connection
/// (`MysqlBranch<'s>` over `&'s mut Conn`), so an `&mut Conn` holder
/// ([`MysqlBackend<&mut Conn>`]) leaves the borrowing stream's higher-ranked `Send`
/// obligation (`for<'s> Stream<'s>: Send`) undischargeable once the future is
/// spawned — the same reason [`select_each_mysql`] takes an owned `Conn`. An owned
/// backend ([`MysqlBackend<Conn>`]) makes the ASK future `Send` (design §4.2).
pub async fn ask_each_mysql(plan: &Plan, conn: Conn) -> Result<bool> {
    let mut b = MysqlBackend::new(conn);
    crate::exec_core::ask(plan, &mut b).await
}
