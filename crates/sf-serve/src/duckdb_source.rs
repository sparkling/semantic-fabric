//! Safe file-backed DuckDB source pooling for the HTTP serve lane.
//!
//! File-backed sources are opened read-only and must already exist. DuckDB's
//! extension autoload/install and external file/network access are disabled:
//! this backend is a relational SQL source, not the deferred RML file-reader
//! path (ADR-0006). A fixed semaphore rejects excess work before spawning a
//! blocking cursor, so concurrency cannot create an unbounded worker queue.

use std::path::Path;
use std::sync::{Arc, Mutex};

use duckdb::{AccessMode, Config, Connection};
use sf_sql::TableSchema;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const MAX_POOL_SIZE: usize = 8;

/// A fixed pool of DuckDB connections. Each admitted request owns one permit
/// and one currently idle connection for its full execution lifetime.
#[derive(Clone)]
pub struct DuckDbPool {
    conns: Arc<Vec<Arc<Mutex<Connection>>>>,
    free: Arc<Mutex<Vec<usize>>>,
    admission: Arc<Semaphore>,
    // Keeps DuckDB spill files in a private, pool-owned directory that is
    // removed when the source closes.
    _temp_dir: Arc<tempfile::TempDir>,
}

/// One admitted DuckDB request. Dropping it releases the pool permit.
pub(crate) struct DuckDbLease {
    conn: Arc<Mutex<Connection>>,
    index: usize,
    free: Arc<Mutex<Vec<usize>>>,
    permit: Option<OwnedSemaphorePermit>,
}

impl Drop for DuckDbLease {
    fn drop(&mut self) {
        match self.conn.try_lock() {
            Ok(guard) => {
                drop(guard);
                self.free
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(self.index);
                drop(self.permit.take());
                return;
            }
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                drop(poisoned.into_inner());
                self.free
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(self.index);
                drop(self.permit.take());
                return;
            }
            Err(std::sync::TryLockError::WouldBlock) => {}
        }

        let conn = Arc::clone(&self.conn);
        let free = Arc::clone(&self.free);
        let index = self.index;
        let permit = self.permit.take();
        let release = move || {
            // An interrupted DuckDB receiver schedules its blocking worker for
            // completion. Do not re-admit this slot until that worker has really
            // released the connection mutex.
            drop(conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner()));
            free.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(index);
            drop(permit);
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn_blocking(release);
        } else {
            std::thread::spawn(release);
        }
    }
}

impl DuckDbLease {
    pub(crate) fn connection(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.conn)
    }
}

impl DuckDbPool {
    fn new(conns: Vec<Connection>, temp_dir: tempfile::TempDir) -> Self {
        assert!(!conns.is_empty(), "DuckDbPool needs a connection");
        let size = conns.len();
        Self {
            conns: Arc::new(conns.into_iter().map(|c| Arc::new(Mutex::new(c))).collect()),
            free: Arc::new(Mutex::new((0..size).rev().collect())),
            admission: Arc::new(Semaphore::new(size)),
            _temp_dir: Arc::new(temp_dir),
        }
    }

    /// Open an existing DuckDB database and introspect all base tables in the
    /// active schema. Serving accepts only an existing, read-only file.
    pub(crate) fn open(path: &str, pool_size: usize) -> Result<(Self, Vec<TableSchema>), String> {
        if path.is_empty() {
            return Err("DuckDB source path must not be empty".to_owned());
        }
        if pool_size == 0 {
            return Err("DuckDB pool size must be at least 1".to_owned());
        }
        if pool_size > MAX_POOL_SIZE {
            return Err(format!(
                "DuckDB pool size {pool_size} exceeds the serve limit of {MAX_POOL_SIZE}"
            ));
        }

        if path == ":memory:" {
            return Err(
                "duckdb::memory: is not a serve source; provide an existing DuckDB file".to_owned(),
            );
        }

        // Canonicalization both rejects a typo before DuckDB can create a new
        // database and makes the exact file being opened explicit in errors.
        let canonical = std::fs::canonicalize(Path::new(path))
            .map_err(|e| format!("resolve DuckDB source {path:?}: {e}"))?;
        if !canonical.is_file() {
            return Err(format!(
                "DuckDB source {:?} is not a regular file",
                canonical.display()
            ));
        }

        let temp_dir = tempfile::Builder::new()
            .prefix("semantic-fabric-duckdb-")
            .tempdir()
            .map_err(|e| format!("create private DuckDB spill directory: {e}"))?;
        let config = restricted_config(AccessMode::ReadOnly)?;
        // Open the canonical path exactly once, then clone connections to that
        // database instance. A pathname replacement cannot redirect a running
        // pool to a different source between independent opens.
        let primary = Connection::open_with_flags(&canonical, config)
            .map_err(|e| format!("open DuckDB read-only {}: {e}", canonical.display()))?;
        let temp_dir_text = temp_dir
            .path()
            .to_str()
            .ok_or_else(|| "DuckDB spill directory path is not valid UTF-8".to_owned())?;
        primary
            .execute("SET temp_directory = ?", [temp_dir_text])
            .map_err(|e| format!("set private DuckDB spill directory: {e}"))?;
        primary
            .execute_batch(
                "SET enable_external_access = false; \
                 SET lock_configuration = true",
            )
            .map_err(|e| format!("lock restricted DuckDB configuration: {e}"))?;
        let schema = introspect_duckdb_all(&primary)?;
        let mut conns = Vec::with_capacity(pool_size);
        conns.push(primary);
        for _ in 1..pool_size {
            let cloned = conns[0]
                .try_clone()
                .map_err(|e| format!("clone DuckDB serve connection: {e}"))?;
            conns.push(cloned);
        }
        Ok((Self::new(conns, temp_dir), schema))
    }

    /// Admit one request without queueing. The caller maps exhaustion to HTTP
    /// 503 and keeps the lease alive until execution or streaming completes.
    pub(crate) fn try_acquire(&self) -> Result<DuckDbLease, ()> {
        let permit = Arc::clone(&self.admission)
            .try_acquire_owned()
            .map_err(|_| ())?;
        let index = self
            .free
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop()
            .expect("a semaphore permit always has one free DuckDB connection");
        Ok(DuckDbLease {
            conn: Arc::clone(&self.conns[index]),
            index,
            free: Arc::clone(&self.free),
            permit: Some(permit),
        })
    }
}

fn restricted_config(access_mode: AccessMode) -> Result<Config, String> {
    Config::default()
        .access_mode(access_mode)
        .and_then(|config| config.enable_autoload_extension(false))
        .and_then(|config| config.max_memory("512MB"))
        .and_then(|config| config.threads(2))
        .and_then(|config| config.with("max_temp_directory_size", "1GB"))
        .and_then(|config| config.with("allow_community_extensions", "false"))
        .and_then(|config| config.with("allow_unsigned_extensions", "false"))
        .and_then(|config| config.with("allow_persistent_secrets", "false"))
        .map_err(|e| format!("configure restricted DuckDB source: {e}"))
}

/// Introspect every base table in DuckDB's active database/schema.
pub(crate) fn introspect_duckdb_all(conn: &Connection) -> Result<Vec<TableSchema>, String> {
    const MAX_TABLES: usize = 4_096;
    const MAX_COLUMNS_PER_TABLE: usize = 4_096;
    {
        let mut statement = conn
            .prepare(
                "SELECT table_name, count(*) AS column_count \
                 FROM information_schema.columns \
                 WHERE table_catalog = current_database() \
                   AND table_schema = current_schema() \
                 GROUP BY table_name \
                 HAVING count(*) > 4096 \
                 LIMIT 1",
            )
            .map_err(|e| format!("preflight DuckDB columns: {e}"))?;
        let mut rows = statement
            .query([])
            .map_err(|e| format!("query DuckDB column preflight: {e}"))?;
        if let Some(row) = rows
            .next()
            .map_err(|e| format!("read DuckDB column preflight: {e}"))?
        {
            let table: String = row.get(0).map_err(|e| e.to_string())?;
            let columns: i64 = row.get(1).map_err(|e| e.to_string())?;
            return Err(format!(
                "DuckDB table {table:?} has {columns} columns, exceeding the {MAX_COLUMNS_PER_TABLE}-column serve limit"
            ));
        }
    }
    let mut statement = conn
        .prepare(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_catalog = current_database() \
               AND table_schema = current_schema() \
               AND table_type = 'BASE TABLE' \
             ORDER BY table_name \
             LIMIT 4097",
        )
        .map_err(|e| format!("list DuckDB tables: {e}"))?;
    let names: Vec<String> = statement
        .query_map([], |row| row.get(0))
        .map_err(|e| format!("query DuckDB tables: {e}"))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("read DuckDB table name: {e}"))?;
    if names.len() > MAX_TABLES {
        return Err(format!(
            "DuckDB catalog exceeds the {MAX_TABLES}-table serve limit"
        ));
    }
    let mut schemas = Vec::with_capacity(names.len());
    for name in names {
        let schema =
            sf_sql::introspect::introspect_duckdb(conn, &name).map_err(|e| e.to_string())?;
        if schema.columns.len() > MAX_COLUMNS_PER_TABLE {
            return Err(format!(
                "DuckDB table {name:?} exceeds the {MAX_COLUMNS_PER_TABLE}-column serve limit"
            ));
        }
        schemas.push(schema);
    }
    Ok(schemas)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "sf_serve_duckdb_{tag}_{}_{unique}.duckdb",
            std::process::id()
        ))
    }

    #[test]
    fn file_sources_are_existing_read_only_databases_without_external_access() {
        let path = temp_path("restricted");
        {
            let conn = Connection::open(&path).expect("create fixture");
            conn.execute_batch(
                "CREATE TABLE widgets(id INTEGER PRIMARY KEY, name VARCHAR); \
                 INSERT INTO widgets VALUES (1, 'sprocket');",
            )
            .expect("seed fixture");
        }

        let (pool, schema) = DuckDbPool::open(path.to_str().unwrap(), 1).expect("open pool");
        assert_eq!(schema.len(), 1);
        assert_eq!(schema[0].name, "widgets");
        let lease = pool.try_acquire().expect("first request admitted");
        let conn = lease.conn.lock().unwrap();
        assert!(
            conn.execute("INSERT INTO widgets VALUES (2, 'forbidden')", [])
                .is_err(),
            "serve connection must be read-only"
        );
        let external: bool = conn
            .query_row(
                "SELECT current_setting('enable_external_access')::BOOLEAN",
                [],
                |row| row.get(0),
            )
            .expect("read DuckDB setting");
        assert!(!external, "external file/network access must default off");
        let spill_dir: String = conn
            .query_row("SELECT current_setting('temp_directory')", [], |row| {
                row.get(0)
            })
            .expect("read DuckDB spill directory");
        let spill_dir = Path::new(&spill_dir);
        assert!(spill_dir.is_dir(), "private spill directory must exist");
        assert!(
            spill_dir
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("semantic-fabric-duckdb-")),
            "spill directory must be pool-owned: {}",
            spill_dir.display()
        );
        assert!(
            conn.execute_batch("SET enable_external_access = true")
                .is_err(),
            "locked configuration must prevent re-enabling external access"
        );
        assert!(
            conn.execute_batch("SET threads = 8").is_err(),
            "locked configuration must prevent resource-limit changes"
        );
        for (operation, sql) in [
            ("DDL", "CREATE TABLE forbidden(id INTEGER)"),
            ("attach", "ATTACH '/tmp/forbidden.duckdb' AS forbidden"),
            (
                "external CSV",
                "SELECT * FROM read_csv_auto('/tmp/forbidden.csv')",
            ),
            (
                "external Parquet",
                "SELECT * FROM read_parquet('/tmp/forbidden.parquet')",
            ),
            ("extension install", "INSTALL httpfs"),
            ("extension load", "LOAD httpfs"),
            (
                "persistent secret",
                "CREATE PERSISTENT SECRET forbidden (TYPE S3, KEY_ID 'x', SECRET 'y')",
            ),
        ] {
            assert!(
                conn.execute_batch(sql).is_err(),
                "restricted serve connection must reject {operation}"
            );
        }
        drop(conn);
        drop(lease);
        drop(pool);
        std::fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn missing_file_is_rejected_without_creating_it() {
        let path = temp_path("missing");
        assert!(!path.exists());
        let error = DuckDbPool::open(path.to_str().unwrap(), 1)
            .err()
            .expect("missing source must fail");
        assert!(error.contains("resolve DuckDB source"), "{error}");
        assert!(!path.exists(), "a source typo must not create a database");
    }

    #[test]
    fn admission_is_bounded_by_pool_size() {
        let path = temp_path("pool_two");
        {
            let conn = Connection::open(&path).expect("create fixture");
            conn.execute_batch("CREATE TABLE widgets(id INTEGER)")
                .expect("seed fixture");
        }
        let (pool, _) = DuckDbPool::open(path.to_str().unwrap(), 2).expect("open pool");
        let first = pool.try_acquire().expect("first request admitted");
        let second = pool.try_acquire().expect("second request admitted");
        {
            let conn = second.conn.lock().unwrap();
            let external: bool = conn
                .query_row(
                    "SELECT current_setting('enable_external_access')::BOOLEAN",
                    [],
                    |row| row.get(0),
                )
                .expect("read cloned connection setting");
            assert!(!external, "cloned connections must remain restricted");
            assert!(
                conn.execute_batch("SET enable_external_access = true")
                    .is_err(),
                "cloned connections must inherit the configuration lock"
            );
        }
        assert!(pool.try_acquire().is_err(), "third request must be shed");
        drop(first);
        assert!(pool.try_acquire().is_ok(), "permit must return on drop");
        drop(second);
        drop(pool);
        std::fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn schema_snapshot_changes_only_after_source_restart() {
        let path = temp_path("schema_restart");
        {
            let conn = Connection::open(&path).expect("create fixture");
            conn.execute_batch("CREATE TABLE widgets(id INTEGER)")
                .expect("seed fixture");
        }
        let (pool, before) = DuckDbPool::open(path.to_str().unwrap(), 1).expect("open pool");
        assert_eq!(before[0].columns.len(), 1);
        let lease = pool.try_acquire().expect("request admitted");
        assert!(
            lease
                .conn
                .lock()
                .unwrap()
                .execute_batch("ALTER TABLE widgets ADD COLUMN name VARCHAR")
                .is_err(),
            "the live read-only source cannot drift through serving DDL"
        );
        drop(lease);
        drop(pool);

        {
            let conn = Connection::open(&path).expect("reopen writable fixture");
            conn.execute_batch("ALTER TABLE widgets ADD COLUMN name VARCHAR")
                .expect("change schema between server lifetimes");
        }
        let (reopened, after) = DuckDbPool::open(path.to_str().unwrap(), 1).expect("reopen pool");
        assert_eq!(after[0].columns.len(), 2);
        drop(reopened);
        std::fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn in_memory_source_is_not_exposed_by_the_file_opener() {
        let error = DuckDbPool::open(":memory:", 1)
            .err()
            .expect("CLI memory source must fail");
        assert!(error.contains("not a serve source"), "{error}");
    }

    #[test]
    fn excessive_pool_size_is_rejected_before_opening() {
        let error = DuckDbPool::open("missing.duckdb", MAX_POOL_SIZE + 1)
            .err()
            .expect("excessive pool must fail at validation");
        assert!(error.contains("serve limit of 8"), "{error}");
    }
}
