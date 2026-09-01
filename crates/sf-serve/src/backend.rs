//! Relational backend handles, SQLite pooling, and backend introspection.

use std::sync::{Arc, Mutex};

use sf_sql::introspect::introspect_sqlite;
use sf_sql::{Dialect, TableSchema};

use crate::source::POSTGRES_RELATION_SCOPE_SETTING;

/// A pooled PostgreSQL connection, re-derefed to `tokio_postgres::Client` in one
/// hop. `deadpool_postgres::Object` derefs to its own `ClientWrapper` (adds
/// statement caching), not directly to `Client` — `sf_sparql::exec_pg`'s generic
/// client-handle bound (`Deref<Target = Client>`, shared with the conformance
/// harness's plain `Arc<Client>`) needs the single hop this newtype provides.
pub(crate) struct PgConn(deadpool_postgres::Object);

impl PgConn {
    pub(crate) async fn checked(conn: deadpool_postgres::Object) -> Result<Self, String> {
        verify_pg_relation_scope(&conn).await?;
        Ok(Self(conn))
    }
}

impl std::ops::Deref for PgConn {
    type Target = tokio_postgres::Client;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Verify the relation-resolution invariant before catalogue reads or source
/// I/O. The error is deliberately structural and does not expose connection
/// configuration.
pub(crate) async fn verify_pg_relation_scope(
    client: &tokio_postgres::Client,
) -> Result<(), String> {
    let row = client
        .query_one(
            "SELECT pg_catalog.current_setting('search_path') = $1",
            &[&POSTGRES_RELATION_SCOPE_SETTING],
        )
        .await
        .map_err(|_| "PostgreSQL relation scope could not be verified".to_owned())?;
    if row.get::<_, bool>(0) {
        Ok(())
    } else {
        Err("PostgreSQL relation scope invariant mismatch".to_owned())
    }
}

/// A small fixed pool of SQLite connections, dispatched round-robin. Serve is a
/// READ-ONLY endpoint (query operation only), and SQLite allows many concurrent
/// READERS against a file-backed database regardless of journal mode — so a
/// file-backed source gets `pool_size` independent read-only connections instead
/// of forcing every concurrent request through one shared connection (ADR-0010
/// status-correction part 2: "SQLite remains a single `Mutex<Connection>` by
/// choice ... an open refinement" — this closes that refinement). Each request
/// takes exactly one member's mutex for its query's duration, so up to
/// `pool_size` requests proceed concurrently instead of fully serialising.
///
/// `:memory:` sources stay a pool of one, read-write (see [`Backend::sqlite`] /
/// [`crate::run`]): each `rusqlite::Connection::open(":memory:")` call creates an
/// independent, private, empty database, so pooling `:memory:` the normal way
/// would silently serve queries against the wrong (empty) database.
#[derive(Clone)]
pub struct SqlitePool {
    conns: Arc<Vec<Arc<Mutex<rusqlite::Connection>>>>,
    next: Arc<std::sync::atomic::AtomicUsize>,
}

impl SqlitePool {
    /// A pool of one connection — the original single-`Mutex` shape, used for
    /// `:memory:` sources and any caller that already owns one open connection.
    fn one(conn: rusqlite::Connection) -> Self {
        Self::new(vec![conn])
    }

    fn new(conns: Vec<rusqlite::Connection>) -> Self {
        assert!(
            !conns.is_empty(),
            "SqlitePool needs at least one connection"
        );
        Self {
            conns: Arc::new(conns.into_iter().map(|c| Arc::new(Mutex::new(c))).collect()),
            next: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Round-robin the next pool member (wraps around). A pool of one (every
    /// `:memory:` source, and any single-connection test fixture built via
    /// [`Backend::sqlite`]) always returns that same connection, so callers see
    /// identical behaviour to the pre-pool single-`Mutex` design.
    pub fn pick(&self) -> Arc<Mutex<rusqlite::Connection>> {
        let n = self.conns.len();
        let i = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % n;
        self.conns[i].clone()
    }
}

/// A live relational backend the endpoint queries (ADR-0006). SQLite is sync
/// (`rusqlite::Connection`, not `Send` across awaits — held behind a `Mutex`); its
/// blocking now lives entirely in the adapter's cap-1 `spawn_blocking` bridge
/// (ADR-0024 §4.1 `SqliteOwnedBackend`), so the serve lane drives all three
/// backends through the same async streamer. A file-backed SQLite source is a
/// small [`SqlitePool`] of read-only connections (SQLite read-concurrency,
/// ADR-0010 status-correction part 2), round-robin dispatched per request.
/// PostgreSQL is a bounded `deadpool_postgres::Pool` (ADR-0010 §C stream-lane
/// pool, ADR-0027; M4 wave-2 finding 2) — was a single shared `Client`
/// serialising every PG HTTP request; each request now draws a pooled connection
/// for its lifetime, mirroring MySQL's existing `mysql_async::Pool`.
#[derive(Clone)]
pub enum Backend {
    Sqlite(SqlitePool),
    Pg(deadpool_postgres::Pool),
    /// MySQL: a cloneable `mysql_async::Pool`; each streaming request draws a
    /// DEDICATED connection for the stream's lifetime, discarded/reset on early drop
    /// (ADR-0024 §4.2 — mirrors PG cancel-on-drop).
    Mysql(mysql_async::Pool),
}

/// Concrete backend implementation wired into the serve adapter.
///
/// This is an implementation identity, not a release-capability admission. The
/// current capability catalogue admits zero production backends.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BackendKind {
    Sqlite,
    Postgres,
    MySql,
}

impl BackendKind {
    pub const fn dialect(self) -> Dialect {
        match self {
            Self::Sqlite => Dialect::Sqlite,
            Self::Postgres => Dialect::Postgres,
            Self::MySql => Dialect::MySql,
        }
    }
}

impl Backend {
    /// Wrap a single open SQLite connection as a backend (a pool of one — the
    /// shape every `:memory:` source and most test fixtures want).
    pub fn sqlite(conn: rusqlite::Connection) -> Self {
        Backend::Sqlite(SqlitePool::one(conn))
    }

    /// Open `pool_size` independent READ-ONLY connections to the SQLite file at
    /// `path`, dispatched round-robin per request ([`SqlitePool`]). Never
    /// touches journal mode or any other persistent setting on the file.
    /// `:memory:` is special-cased to a single read-write connection regardless
    /// of `pool_size` ([`SqlitePool`] doc). Returns the introspected schema
    /// alongside the backend (introspected from one pool member — read-only
    /// queries against `sqlite_master`/`PRAGMA table_info`, safe to share). Public
    /// so callers (including tests) can build a multi-connection SQLite backend
    /// the same way `run::open_backend` does, without going through the blocking
    /// `serve` entry point — mirrors PG pools being fully caller-constructible via
    /// public `deadpool_postgres` APIs.
    pub fn sqlite_pool_from_path(
        path: &str,
        pool_size: usize,
    ) -> Result<(Self, Vec<TableSchema>), String> {
        if path == ":memory:" {
            let conn =
                rusqlite::Connection::open(path).map_err(|e| format!("open SQLite {path}: {e}"))?;
            let schema = introspect_sqlite_all(&conn)?;
            return Ok((Backend::sqlite(conn), schema));
        }
        let n = pool_size.max(1);
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
            | rusqlite::OpenFlags::SQLITE_OPEN_URI;
        let mut conns = Vec::with_capacity(n);
        for _ in 0..n {
            conns.push(
                rusqlite::Connection::open_with_flags(path, flags)
                    .map_err(|e| format!("open SQLite (read-only) {path}: {e}"))?,
            );
        }
        let schema = introspect_sqlite_all(&conns[0])?;
        Ok((Backend::Sqlite(SqlitePool::new(conns)), schema))
    }

    /// The concrete serve-adapter implementation kind.
    pub const fn kind(&self) -> BackendKind {
        match self {
            Backend::Sqlite(_) => BackendKind::Sqlite,
            Backend::Pg(_) => BackendKind::Postgres,
            Backend::Mysql(_) => BackendKind::MySql,
        }
    }

    /// The SQL dialect this backend speaks (drives emission/introspection).
    pub const fn dialect(&self) -> Dialect {
        self.kind().dialect()
    }
}

/// Introspect every SQLite base table (schema order from `sqlite_master`), filling
/// the source schema that makes the ADR-0007 cascade passes fire.
pub fn introspect_sqlite_all(conn: &rusqlite::Connection) -> Result<Vec<TableSchema>, String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .map_err(|e| e.to_string())?;
    let names: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let mut schemas = Vec::with_capacity(names.len());
    for name in names {
        schemas.push(introspect_sqlite(conn, &name).map_err(|e| e.to_string())?);
    }
    Ok(schemas)
}

/// Introspect every PostgreSQL public base table in one coherent read-only
/// snapshot: one enumeration plus 6 set-based catalogue round trips, rather
/// than 6 **per table** (M4 wave-2 finding 4 — the N+1 this function used to
/// drive via a per-table [`sf_sql::introspect::introspect_postgres`] loop).
pub async fn introspect_pg_all(
    client: &mut tokio_postgres::Client,
) -> Result<Vec<TableSchema>, String> {
    sf_sql::introspect::introspect_postgres_public_snapshot(client)
        .await
        .map_err(|e| e.to_string())
}
