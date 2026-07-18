//! PostgreSQL execution path (ADR-0006 *Streaming & bounded memory*; ADR-0010 §C;
//! ADR-0015): run the emitted PostgreSQL SQL over a **live** server through the
//! driver-agnostic execution core ([`crate::exec_core`], generic over
//! [`sf_sql::SqlBackend`]) with the PostgreSQL adapter ([`PgBackend`]). The adapter
//! opens a bounded-memory server-side cursor ([`sf_sql::PgRowStream`], `query_raw`
//! — never the buffer-all `query()`), marshals each row, and reconstructs `oxrdf`
//! terms through the **single** sf-core term-gen path shared with the SQLite
//! executor (ADR-0003 R3).
//!
//! Since ADR-0024 M3 this file is a thin set of delegators: the branches loop,
//! `rust_group` dispatch (q9), DISTINCT dedup (q15), ORDER/OFFSET/LIMIT, and the §10
//! catalog probe all live once in [`crate::exec_core`]; the q12 typed-column bind,
//! `pg_value`, and `pg_xsd_code` live in [`sf_sql::backend::pg`]. Every existing
//! caller (conformance, serve, bench) is untouched: `construct_each_pg` /
//! `select_each_pg` / `ask_pg` took a hardcoded `Arc<Client>` and are now generic
//! over any owned, `Send + 'static` handle that derefs to a `Client` (ADR-0027 PG
//! connection pooling, M4 wave-2 finding 2) — conformance's `Arc<Client>` and the
//! serve lane's pooled `deadpool_postgres::Object` both satisfy it, so no caller's
//! argument type needs to change.
//!
//! Requires a live server, so this path is exercised by the conformance
//! integration suite (ADR-0012), never the in-crate unit tests.

use std::future::Future;
use std::ops::Deref;

use sf_core::{Quad, Term, Triple};
use sf_sql::backend::pg::PgBackend;
use sf_sql::Dialect;
use tokio_postgres::Client;

use crate::exec::Solutions;
use crate::{Plan, Result};

/// Collect a CONSTRUCT's triples over a live PostgreSQL connection. Streaming is
/// the bounded-memory core ([`crate::exec_core::construct_triples`]); this collects
/// for the conformance harness.
pub async fn construct_triples_pg(plan: &Plan, client: &Client) -> Result<Vec<Triple>> {
    let mut b = PgBackend::new(client);
    crate::exec_core::construct_triples(plan, &mut b).await
}

/// Stream a CONSTRUCT over a live PostgreSQL connection: `sink` is awaited once
/// per solution with that solution's instantiated triples (bounded by the
/// template size — never the whole graph). `sink(..).await` backpressures the
/// server-side `query_raw` cursor (ADR-0010 §C).
///
/// Takes an owned client handle `C` (not `&Client`) because this future is
/// `tokio::spawn`ed by the serve lane: a `'static` backend (`PgBackend<C>`) is
/// what lets the generic core's `Send` bound hold across the spawn (ADR-0024
/// §1.103 — the AFIT `Send`-future requirement). `C: Deref<Target = Client>` so
/// both `Arc<Client>` (conformance) and a pooled `deadpool_postgres::Object`
/// (the serve lane, ADR-0027) work unchanged.
pub async fn construct_each_pg<C, F, Fut>(plan: &Plan, client: C, sink: F) -> Result<()>
where
    C: Deref<Target = Client> + Send + 'static,
    F: FnMut(Vec<Triple>) -> Fut + Send,
    Fut: Future<Output = Result<()>> + Send,
{
    let mut b = PgBackend::new(client);
    crate::exec_core::construct_each_async(plan, &mut b, sink).await
}

/// Execute a SELECT over a live PostgreSQL connection, collecting solutions —
/// the async mirror of the sync SQLite [`crate::exec::select`] (ADR-0003 R3: the
/// SAME reconstruction). Bounded-memory streaming is the [`crate::exec_core`]
/// core (server-side `query_raw` cursor, one row in flight); this collects the
/// projected rows for callers/tests.
pub async fn select_pg(plan: &Plan, client: &Client) -> Result<Solutions> {
    let mut b = PgBackend::new(client);
    crate::exec_core::select(plan, &mut b).await
}

/// Stream a SELECT's solution rows over a live PostgreSQL connection into an
/// async `sink`, awaited once per projected row (in `plan` projection order,
/// `None` = unbound). The HTTP layer serialises + flushes each row into the
/// response body without ever collecting the result set, and `sink(..).await`
/// applies per-row backpressure to the `query_raw` cursor (ADR-0006 / ADR-0010 §C).
///
/// Generic over the owned client handle `C` — see [`construct_each_pg`].
pub async fn select_each_pg<C, F, Fut>(plan: &Plan, client: C, sink: F) -> Result<()>
where
    C: Deref<Target = Client> + Send + 'static,
    F: FnMut(Vec<Option<Term>>) -> Fut + Send,
    Fut: Future<Output = Result<()>> + Send,
{
    let mut b = PgBackend::new(client);
    crate::exec_core::select_each_async(plan, &mut b, sink).await
}

/// Execute an ASK over a live PostgreSQL connection — true iff at least one
/// solution exists. The async mirror of the sync [`crate::exec::ask`] (same
/// streaming core, same reconstruction).
///
/// Takes an owned client handle `C` (not `&Client`) because the serve lane
/// awaits this inside an axum handler future that must be `Send`; `C: Deref<Target
/// = Client> + Send + 'static` — see [`construct_each_pg`] — covers both
/// conformance's `Arc<Client>` and the serve lane's pooled
/// `deadpool_postgres::Object`.
pub async fn ask_pg<C>(plan: &Plan, client: C) -> Result<bool>
where
    C: Deref<Target = Client> + Send + 'static,
{
    let mut b = PgBackend::new(client);
    crate::exec_core::ask(plan, &mut b).await
}

/// Collect the mapping-IR **quad** dump over a live PostgreSQL connection
/// (ADR-0005 named-graph conformance) — each triple carries the graph term from
/// the applicable `rr:graphMap`(s), built through the same sf-core term-gen path.
pub async fn dump_quads_pg(
    maps: &[sf_core::ir::TriplesMap],
    client: &Client,
    dialect: Dialect,
) -> Result<Vec<Quad>> {
    let mut b = PgBackend::new(client);
    crate::exec_core::dump_quads(maps, &mut b, dialect).await
}
