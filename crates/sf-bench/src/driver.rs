//! Run a GTFS query through the live virtualizer over a relational source
//! (ADR-0005/0006). Parse the mapping once (`sf-mapping`), translate SPARQL→SQL
//! (`sf-sparql`), execute over SQLite, reconstruct `oxrdf` terms, stream.
//!
//! Two execution shapes:
//! * **SELECT** ([`run_select`]) — collects solutions; used for per-query
//!   wall-clock latency.
//! * **CONSTRUCT** ([`stream_construct_count`], [`stream_construct_timed`]) — the
//!   bounded-memory streaming path (`exec::construct` drives a per-triple sink and
//!   never builds the triple set), used for first-result latency and the
//!   constant-memory demonstration.
//!
//! ## Backend dispatch (sync SQLite vs async PostgreSQL)
//!
//! The same OBDA query runs over either of two relational backends, and the
//! dispatch is **at the call site by signature**, not behind one runtime-erased
//! entry point — because the two halves have fundamentally different execution
//! models (ADR-0006):
//!
//! * **SQLite** — [`run_select`] is **synchronous** (`rusqlite`'s blocking
//!   cursor). No `tokio` runtime is involved or required.
//! * **PostgreSQL** — [`run_select_pg`] is **async** (`tokio-postgres` + the
//!   `exec_pg::select_pg` server-side `query_raw` cursor) and so takes a live
//!   `&tokio_postgres::Client` and must be `.await`ed on a `tokio` runtime.
//!
//! Both reuse the *same* translation ([`parse_and_translate_with`], differing
//! only in [`Dialect`]) and the *same* single sf-core term-gen reconstruction
//! (ADR-0003 R3) — only the source executor differs. Keeping them as two
//! functions means the SQLite path never pays for, nor links against, an async
//! runtime: the caller (a `criterion` bench, a server handler) picks the arm that
//! matches its source and its sync/async context.

use std::time::{Duration, Instant};

use rusqlite::Connection;
use sf_core::ir::TriplesMap;
use sf_sparql::{
    exec, exec_pg, parse_and_translate_flat_with, parse_and_translate_tree_with,
    parse_and_translate_with, Tbox,
};
use sf_sql::{Dialect, TableSchema};
use tokio_postgres::Client;

use crate::workload::MAPPING_TTL;

/// Boxed error so the driver can thread `sf-mapping` / `sf-sparql` / `rusqlite`
/// failures uniformly without pulling in `anyhow` (kept off the dependency set).
pub type DResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Parse the workload's R2RML mapping into the `sf-core` IR (once per run).
pub fn mapping() -> DResult<Vec<TriplesMap>> {
    Ok(sf_mapping::parse_r2rml(MAPPING_TTL)?)
}

/// Introspect every base table of the source **once** per run. Threading the
/// resulting `&[TableSchema]` into translation is what makes the ADR-0007 cascade
/// passes (self-join / FD / FK-PK join elimination / redundant-DISTINCT) fire on
/// the live OBDA path; with an empty schema they are sound no-ops.
pub fn introspect(conn: &Connection) -> DResult<Vec<TableSchema>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let names: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);
    let mut schemas = Vec::with_capacity(names.len());
    for name in &names {
        schemas.push(sf_sql::introspect::introspect_sqlite(conn, name)?);
    }
    Ok(schemas)
}

/// Execute a SELECT and return its solution-row count. Full-result wall-clock is
/// the caller's `Instant` around this call (the `criterion` latency track).
pub fn run_select(
    maps: &[TriplesMap],
    conn: &Connection,
    schema: &[TableSchema],
    sparql: &str,
) -> DResult<usize> {
    let plan = parse_and_translate_with(sparql, maps, Dialect::Sqlite, &Tbox::default(), schema)?;
    let sol = exec::select(&plan, conn)?;
    Ok(sol.rows.len())
}

/// Execute a SELECT through the **flat unfold path** (the ADR-0023 oracle,
/// permanent fallback) and return its solution-row count. Used by the shootout
/// bench group `obda_select_flat_1x` to give flat a baseline independent of the
/// M8 default switch (which routes the public API through the tree path).
pub fn run_select_flat(
    maps: &[TriplesMap],
    conn: &Connection,
    schema: &[TableSchema],
    sparql: &str,
) -> DResult<usize> {
    let plan =
        parse_and_translate_flat_with(sparql, maps, Dialect::Sqlite, &Tbox::default(), schema)?;
    let sol = exec::select(&plan, conn)?;
    Ok(sol.rows.len())
}

/// Execute a SELECT through the **operator-tree (IQ) path** (ADR-0023 M7) and
/// return its solution-row count — the tree-path sibling of [`run_select`].
/// Translation uses [`parse_and_translate_tree_with`]; execution is the same
/// `exec::select` the flat path uses, so timing isolates translation + plan
/// structure rather than the executor. The benchmark target: tree ≥ flat speed.
pub fn run_select_tree(
    maps: &[TriplesMap],
    conn: &Connection,
    schema: &[TableSchema],
    sparql: &str,
) -> DResult<usize> {
    let plan =
        parse_and_translate_tree_with(sparql, maps, Dialect::Sqlite, &Tbox::default(), schema)?;
    let sol = exec::select(&plan, conn)?;
    Ok(sol.rows.len())
}

/// Execute a SELECT over a **live PostgreSQL** source and return its solution-row
/// count — the async sibling of [`run_select`]. Translation is identical bar the
/// [`Dialect::Postgres`] target; execution goes through the bounded-memory
/// server-side cursor ([`exec_pg::select_pg`], `query_raw`), reusing the same
/// sf-core term-gen reconstruction (ADR-0003 R3). This is the path the fair
/// Ontop-vs-sf latency benchmark and serve-over-PG build on. Full-result
/// wall-clock is the caller's `Instant` around the `.await`.
pub async fn run_select_pg(
    maps: &[TriplesMap],
    client: &Client,
    schema: &[TableSchema],
    sparql: &str,
) -> DResult<usize> {
    let plan = parse_and_translate_with(sparql, maps, Dialect::Postgres, &Tbox::default(), schema)?;
    let sol = exec_pg::select_pg(&plan, client).await?;
    Ok(sol.rows.len())
}

/// Stream a CONSTRUCT through a discarding sink and return the produced-triple
/// count. Bounded memory by construction: `exec::construct` holds one row /
/// triple in flight, never the result set (ADR-0006).
pub fn stream_construct_count(
    maps: &[TriplesMap],
    conn: &Connection,
    schema: &[TableSchema],
    sparql: &str,
) -> DResult<u64> {
    let plan = parse_and_translate_with(sparql, maps, Dialect::Sqlite, &Tbox::default(), schema)?;
    let count = exec::construct(&plan, conn, |_triple| {})?;
    Ok(count)
}

/// Stream a CONSTRUCT, returning `(triples, first_result_latency, total_latency)`.
/// `first_result_latency` is the wall-time from query start to the first produced
/// triple (the bounded-first-result claim, ADR-0006); `total_latency` covers the
/// full stream. The sink still discards, so memory stays bounded throughout.
pub fn stream_construct_timed(
    maps: &[TriplesMap],
    conn: &Connection,
    schema: &[TableSchema],
    sparql: &str,
) -> DResult<(u64, Duration, Duration)> {
    let plan = parse_and_translate_with(sparql, maps, Dialect::Sqlite, &Tbox::default(), schema)?;
    let start = Instant::now();
    let mut first: Option<Duration> = None;
    let count = exec::construct(&plan, conn, |_triple| {
        if first.is_none() {
            first = Some(start.elapsed());
        }
    })?;
    let total = start.elapsed();
    Ok((count, first.unwrap_or(total), total))
}
