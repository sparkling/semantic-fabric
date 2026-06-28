//! Bounded-memory, server-side-cursor result streaming (ADR-0006 *Streaming &
//! bounded memory*; ADR-0010 §C).
//!
//! The invariant: results stream end to end and **no operator buffers instance
//! data unbounded**. Engine memory is bounded by `⟨T, M⟩` plus a fixed streaming
//! budget, independent of source size. The abstractions here enforce that *by
//! shape* — none of them can hand back the whole result set:
//!
//! * **SQLite** — [`sqlite_for_each`] drives a query through `rusqlite`'s lazy
//!   cursor one row at a time, and [`SqliteRowStream`] is the pull-style
//!   equivalent. Neither ever builds a `Vec<Row>`.
//! * **PostgreSQL** — [`PgRowStream`] wraps `tokio_postgres`'s `query_raw`
//!   server-side cursor (**never** the buffer-all `query()`, which collects a
//!   `Vec<Row>`; ADR-0010 §C). `query_raw` bounds client memory and propagates
//!   TCP backpressure to the backend.
//!
//! Values are always **bound parameters** (ADR-0010 R1): the constructors take a
//! parameter list, never interpolated SQL.

use std::pin::Pin;

use futures_util::TryStreamExt;

use crate::error::Result;

// --- SQLite (synchronous cursor) ------------------------------------------

/// Run `sql` (with bound `params`) and invoke `f` for **each row in turn**, over
/// `rusqlite`'s lazy cursor — never collecting a `Vec<Row>`. Returns the number
/// of rows seen. Bounded memory by contract: exactly one `&Row` is live at a
/// time, so memory is independent of the result size (ADR-0006).
pub fn sqlite_for_each<P, F>(
    conn: &rusqlite::Connection,
    sql: &str,
    params: P,
    mut f: F,
) -> Result<u64>
where
    P: rusqlite::Params,
    F: FnMut(&rusqlite::Row<'_>) -> Result<()>,
{
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query(params)?; // lazy cursor — NOT query_map().collect()
    let mut seen = 0u64;
    while let Some(row) = rows.next()? {
        f(row)?;
        seen += 1;
    }
    Ok(seen)
}

/// The declared SQL type of each result column of `sql`, in projection order
/// (`None` for a computed/expression column with no declared type). Read from the
/// prepared statement's column metadata, which SQLite traces back through
/// `rr:sqlQuery` views and derived tables to the originating base column — so the
/// R2RML §10 natural datatype mapping (ADR-0015) applies uniformly to base-table
/// and view columns. No rows are fetched (metadata is known at prepare time).
pub fn sqlite_column_decltypes(
    conn: &rusqlite::Connection,
    sql: &str,
) -> Result<Vec<Option<String>>> {
    let stmt = conn.prepare(sql)?;
    Ok(stmt
        .columns()
        .iter()
        .map(|c| c.decl_type().map(str::to_owned))
        .collect())
}

/// The result-set column **names** of `sql`, in projection order (R2RML §5.1: an
/// R2RML view's SQL query must not yield two columns with the same name). Read
/// from the prepared statement metadata; no rows are fetched.
pub fn sqlite_column_names(conn: &rusqlite::Connection, sql: &str) -> Result<Vec<String>> {
    let stmt = conn.prepare(sql)?;
    Ok(stmt.columns().iter().map(|c| c.name().to_owned()).collect())
}

/// A pull-style, bounded SQLite row cursor: each [`next_row`](Self::next_row)
/// borrows exactly one row, valid only until the following call, so the whole
/// result set is never materialised (ADR-0006). Construct it from a prepared
/// statement's `query()` cursor.
pub struct SqliteRowStream<'stmt> {
    rows: rusqlite::Rows<'stmt>,
    seen: u64,
}

impl<'stmt> SqliteRowStream<'stmt> {
    /// Wrap a `rusqlite` cursor (`stmt.query(params)?`).
    pub fn new(rows: rusqlite::Rows<'stmt>) -> Self {
        Self { rows, seen: 0 }
    }

    /// Advance to the next row, borrowed until the next call. `None` ends the
    /// stream. One row in flight — bounded memory.
    pub fn next_row(&mut self) -> Result<Option<&rusqlite::Row<'_>>> {
        match self.rows.next()? {
            Some(row) => {
                self.seen += 1;
                Ok(Some(row))
            }
            None => Ok(None),
        }
    }

    /// Rows yielded so far.
    pub fn rows_seen(&self) -> u64 {
        self.seen
    }
}

// --- PostgreSQL (async server-side cursor) --------------------------------

/// Bound parameter slice for the PostgreSQL streaming path (the same shape the
/// driver's own `query` family takes).
pub type PgParams<'a> = &'a [&'a (dyn tokio_postgres::types::ToSql + Sync)];

/// A streaming PostgreSQL result over a **server-side cursor** opened with
/// `query_raw` — never the buffer-all `query()` (ADR-0010 §C). `query_raw`
/// bounds client memory and propagates backpressure to the backend.
///
/// **Cancel-on-drop:** dropping this stream drops the underlying portal, so the
/// backend cancels the in-flight query; the connection must then be *discarded,
/// not recycled* (ADR-0010 §C — that pooling step belongs to the executor /
/// stream-lane pool and is tracked there, not in this leaf type).
///
/// Requires a live server, so it is exercised by the integration suite
/// (ADR-0012), not the in-crate unit tests.
pub struct PgRowStream {
    inner: Pin<Box<tokio_postgres::RowStream>>,
    seen: u64,
}

impl PgRowStream {
    /// Open a streaming cursor for `sql` with bound `params`. `prepare` first so
    /// a malformed query is a clean error *before* streaming begins (ADR-0010
    /// §C); values are bound parameters only (ADR-0010 R1) — they are never
    /// interpolated into `sql`.
    pub async fn open(
        client: &tokio_postgres::Client,
        sql: &str,
        params: PgParams<'_>,
    ) -> Result<Self> {
        let statement = client.prepare(sql).await?;
        // query_raw (server-side cursor / streaming), never query() (buffer-all).
        let stream = client.query_raw(&statement, params.iter().copied()).await?;
        Ok(Self {
            inner: Box::pin(stream),
            seen: 0,
        })
    }

    /// The next streamed row, or `None` at end. One row in flight.
    pub async fn try_next(&mut self) -> Result<Option<tokio_postgres::Row>> {
        match self.inner.try_next().await? {
            Some(row) => {
                self.seen += 1;
                Ok(Some(row))
            }
            None => Ok(None),
        }
    }

    /// Rows yielded so far.
    pub fn rows_seen(&self) -> u64 {
        self.seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thousand_rows() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE t(id INTEGER NOT NULL);
             WITH RECURSIVE c(x) AS (SELECT 0 UNION ALL SELECT x + 1 FROM c WHERE x < 999)
             INSERT INTO t(id) SELECT x FROM c;",
        )
        .unwrap();
        conn
    }

    #[test]
    fn for_each_streams_every_row_without_collecting() {
        let conn = thousand_rows();
        let mut sum = 0i64;
        let seen = sqlite_for_each(&conn, "SELECT id FROM t", [], |row| {
            sum += row.get::<_, i64>(0)?;
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, 1000);
        assert_eq!(sum, (0..1000).sum::<i64>());
    }

    #[test]
    fn pull_stream_yields_one_row_at_a_time() {
        let conn = thousand_rows();
        let mut stmt = conn.prepare("SELECT id FROM t ORDER BY id").unwrap();
        let mut stream = SqliteRowStream::new(stmt.query([]).unwrap());
        let mut n = 0i64;
        while let Some(row) = stream.next_row().unwrap() {
            assert_eq!(row.get::<_, i64>(0).unwrap(), n);
            n += 1;
        }
        assert_eq!(n, 1000);
        assert_eq!(stream.rows_seen(), 1000);
    }

    #[test]
    fn for_each_propagates_callback_errors() {
        let conn = thousand_rows();
        let result = sqlite_for_each(&conn, "SELECT id FROM t", [], |_row| {
            Err(crate::error::Error::Introspection("boom".to_owned()))
        });
        assert!(result.is_err());
    }
}
