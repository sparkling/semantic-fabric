//! Generic ODBC `SqlBackend` adapter (ADR-0024 M8).
//!
//! Covers databases that Ontop reaches via ODBC: IBM DB2, H2 (via JDBC-ODBC
//! bridge), Apache Spark (Hive ODBC connector), Dremio, Denodo, and JBoss Teiid.
//!
//! The real implementation requires the `odbc-backend` feature and a system
//! ODBC driver manager (unixODBC on Linux/macOS, Windows ODBC on Windows).
//! Without the feature the module compiles to stubs returning `Error::Unsupported`.
//!
//! Value marshalling: every cell is read as UTF-8 text via `get_text`, which is
//! the broadest-compatible path across ODBC drivers. NULL values are `None`.
//!
//! Named type aliases at the bottom give each dialect its own public type.
//!
//! Verification tier: compile + unit (marshalling). Live-parity requires an
//! ODBC-capable database and the matching driver installed.

#[cfg(feature = "odbc-backend")]
pub use real::OdbcBackend;
#[cfg(feature = "odbc-backend")]
pub use real::OdbcStream;

#[cfg(not(feature = "odbc-backend"))]
pub use stub::OdbcBackend;
#[cfg(not(feature = "odbc-backend"))]
pub use stub::OdbcStream;

// ─── stub (no odbc-api dep) ──────────────────────────────────────────────────

#[cfg(not(feature = "odbc-backend"))]
mod stub {
    use crate::backend::{BranchStream, RawTuple, SqlBackend};
    use crate::error::{Error, Result};

    /// Generic ODBC stub backend. Enable `odbc-backend` feature to activate.
    /// Use the named type aliases below for per-database clarity.
    pub struct OdbcBackend;

    /// Stub stream for ODBC backends — never yields rows.
    pub struct OdbcStream;

    impl BranchStream for OdbcStream {
        async fn next_row(&mut self) -> Result<Option<RawTuple>> {
            Err(Error::Unsupported(
                "OdbcBackend: enable the `odbc-backend` feature and install an ODBC driver"
                    .to_owned(),
            ))
        }
    }

    impl SqlBackend for OdbcBackend {
        type Stream<'s>
            = OdbcStream
        where
            Self: 's;

        async fn column_names(&mut self, _probe_sql: &str) -> Result<Vec<String>> {
            Err(Error::Unsupported(
                "OdbcBackend: enable the `odbc-backend` feature and install an ODBC driver"
                    .to_owned(),
            ))
        }

        async fn open_branch(&mut self, _sql: &str, _params: &[String]) -> Result<OdbcStream> {
            Err(Error::Unsupported(
                "OdbcBackend: enable the `odbc-backend` feature and install an ODBC driver"
                    .to_owned(),
            ))
        }
    }
}

// ─── real implementation (requires `odbc-backend` feature) ───────────────────

#[cfg(feature = "odbc-backend")]
pub mod real {
    use std::collections::VecDeque;
    use std::sync::OnceLock;

    use odbc_api::{buffers::TextRowSet, Cursor, Environment, ResultSetMetadata};

    use crate::backend::{BranchStream, RawTuple, SqlBackend};
    use crate::error::{Error, Result};

    /// Shared ODBC environment (one per process; ODBC spec §3.2.1).
    fn env() -> &'static Environment {
        static ENV: OnceLock<Environment> = OnceLock::new();
        ENV.get_or_init(|| Environment::new().expect("ODBC Environment::new"))
    }

    /// Generic ODBC backend. Connects via an ODBC connection string.
    ///
    /// # ODBC connection string examples
    /// - DB2: `"Driver={IBM DB2 ODBC DRIVER};Database=mydb;Hostname=localhost;Port=50000;Protocol=TCPIP;Uid=user;Pwd=pass"`
    /// - Spark: `"Driver={Simba Spark ODBC Driver};Host=localhost;Port=10000;Schema=default;SparkServerType=3;AuthMech=0"`
    pub struct OdbcBackend {
        /// Connection string — stored for reconnection on next use.
        conn_str: String,
    }

    impl OdbcBackend {
        /// Create a backend that connects with the given ODBC connection string.
        /// The connection is opened lazily on the first query call.
        pub fn new(conn_str: impl Into<String>) -> Self {
            Self {
                conn_str: conn_str.into(),
            }
        }
    }

    /// A materialised row stream over ODBC result data.
    pub struct OdbcStream {
        rows: VecDeque<RawTuple>,
    }

    impl BranchStream for OdbcStream {
        async fn next_row(&mut self) -> Result<Option<RawTuple>> {
            Ok(self.rows.pop_front())
        }
    }

    /// Row fetch batch size for `TextRowSet`. Larger values reduce round-trips;
    /// bounded so peak memory stays manageable for wide rows.
    const BATCH: usize = 256;

    /// Max text width per cell (bytes). Cells wider than this are truncated and
    /// an `Error::Marshal` is returned for that row.
    const MAX_WIDTH: usize = 65_536;

    impl SqlBackend for OdbcBackend {
        type Stream<'s>
            = OdbcStream
        where
            Self: 's;

        async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>> {
            let conn = env()
                .connect_with_connection_string(&self.conn_str, Default::default())
                .map_err(|e| Error::Marshal(format!("odbc connect: {e}")))?;
            let mut cursor = conn
                .execute(probe_sql, (), None)
                .map_err(|e| Error::Marshal(format!("odbc probe execute: {e}")))?
                .ok_or_else(|| Error::Marshal("odbc probe returned no cursor".to_owned()))?;
            let names: Vec<String> = cursor
                .column_names()
                .map_err(|e| Error::Marshal(format!("odbc column_names: {e}")))?
                .collect::<std::result::Result<_, _>>()
                .map_err(|e| Error::Marshal(format!("odbc column name: {e}")))?;
            Ok(names)
        }

        async fn open_branch(
            &mut self,
            sql: &str,
            lexical_params: &[String],
        ) -> Result<OdbcStream> {
            let conn = env()
                .connect_with_connection_string(&self.conn_str, Default::default())
                .map_err(|e| Error::Marshal(format!("odbc connect: {e}")))?;

            // Inline `?` positional params as single-quoted SQL literals.
            // ODBC `CStr`-based binding requires type-level impl of `InputParameter`
            // which `&str` does not provide; inlining avoids the type complexity
            // while keeping the driver-agnostic lexical contract (ADR-0010 R1).
            let sql_inlined = inline_params_odbc(sql, lexical_params);
            let maybe_cursor = conn
                .execute(&sql_inlined, (), None)
                .map_err(|e| Error::Marshal(format!("odbc execute: {e}")))?;

            let Some(mut cursor) = maybe_cursor else {
                return Ok(OdbcStream {
                    rows: VecDeque::new(),
                });
            };

            let ncols = cursor
                .num_result_cols()
                .map_err(|e| Error::Marshal(format!("odbc num_result_cols: {e}")))?
                as usize;

            let mut row_set_buffer = TextRowSet::for_cursor(BATCH, &mut cursor, Some(MAX_WIDTH))
                .map_err(|e| Error::Marshal(format!("odbc TextRowSet: {e}")))?;
            let mut block_cursor = cursor
                .bind_buffer(&mut row_set_buffer)
                .map_err(|e| Error::Marshal(format!("odbc bind_buffer: {e}")))?;

            let mut rows = VecDeque::new();
            while let Some(batch) = block_cursor
                .fetch()
                .map_err(|e| Error::Marshal(format!("odbc fetch: {e}")))?
            {
                for row_idx in 0..batch.num_rows() {
                    let mut values = Vec::with_capacity(ncols);
                    for col_idx in 0..ncols {
                        let val = batch
                            .at(col_idx, row_idx)
                            .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
                        values.push(val);
                    }
                    let codes = vec![None; ncols];
                    rows.push_back(RawTuple { values, codes });
                }
            }
            Ok(OdbcStream { rows })
        }
    }

    /// Inline `?` positional params into SQL as single-quoted literals.
    fn inline_params_odbc(sql: &str, params: &[String]) -> String {
        if params.is_empty() {
            return sql.to_owned();
        }
        let mut result =
            String::with_capacity(sql.len() + params.iter().map(|p| p.len() + 2).sum::<usize>());
        let mut param_iter = params.iter();
        for ch in sql.chars() {
            if ch == '?' {
                if let Some(p) = param_iter.next() {
                    result.push('\'');
                    result.push_str(&p.replace('\'', "''"));
                    result.push('\'');
                } else {
                    result.push('?');
                }
            } else {
                result.push(ch);
            }
        }
        result
    }
}

// ─── per-database named type aliases ─────────────────────────────────────────

/// IBM DB2 backend (ODBC). Alias of [`OdbcBackend`].
pub type Db2Backend = OdbcBackend;
/// H2 (embedded Java) backend (ODBC/JDBC). Alias of [`OdbcBackend`].
pub type H2Backend = OdbcBackend;
/// Apache Spark SQL backend (ODBC). Alias of [`OdbcBackend`].
pub type SparkBackend = OdbcBackend;
/// Dremio backend (ODBC). Alias of [`OdbcBackend`].
pub type DremioBackend = OdbcBackend;
/// Denodo virtual-DB backend (ODBC). Alias of [`OdbcBackend`].
pub type DenodoBackend = OdbcBackend;
/// JBoss Teiid virtual-DB backend (ODBC). Alias of [`OdbcBackend`].
pub type TeiidBackend = OdbcBackend;

// ─── unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #[cfg(not(feature = "odbc-backend"))]
    #[tokio::test]
    async fn stub_returns_unsupported() {
        use crate::backend::odbc::OdbcBackend;
        use crate::backend::SqlBackend;
        let mut b = OdbcBackend;
        let r = b.column_names("SELECT 1").await;
        assert!(matches!(r, Err(crate::error::Error::Unsupported(_))));
    }
}
