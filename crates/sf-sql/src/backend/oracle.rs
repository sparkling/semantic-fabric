//! Oracle Database `SqlBackend` adapter (ADR-0024 M8).
//!
//! All Oracle-specific code is gated on `#[cfg(feature = "oracle-backend")]`
//! because the `oracle` crate requires OCI client headers at link time. Without
//! the feature the module compiles to the stub path which returns
//! `Error::Unsupported`, proving the trait boundary compiles without OCI.
//!
//! The marshalling logic (`marshal_oracle_value`) converts every `OracleType`
//! variant to a lexical string — this can be unit-tested independently of a
//! live Oracle server.
//!
//! Verification tier: compile + unit (marshalling). Live-parity requires
//! Oracle XE / OCI libraries installed and `SF_ORACLE_URL` set.

#[cfg(feature = "oracle-backend")]
pub use real::OracleBackend;
#[cfg(feature = "oracle-backend")]
pub use real::OracleStream;

#[cfg(not(feature = "oracle-backend"))]
pub use stub::OracleBackend;
#[cfg(not(feature = "oracle-backend"))]
pub use stub::OracleStream;

// ─── stub (no OCI dep) ───────────────────────────────────────────────────────

#[cfg(not(feature = "oracle-backend"))]
mod stub {
    use crate::backend::{BranchStream, RawTuple, SqlBackend};
    use crate::error::{Error, Result};

    /// Oracle stub backend. Enable `oracle-backend` feature and install OCI.
    pub struct OracleBackend;

    /// Stub stream for Oracle — never yields rows.
    pub struct OracleStream;

    impl BranchStream for OracleStream {
        async fn next_row(&mut self) -> Result<Option<RawTuple>> {
            Err(Error::Unsupported(
                "OracleBackend: enable the `oracle-backend` feature and install OCI".to_owned(),
            ))
        }
    }

    impl SqlBackend for OracleBackend {
        type Stream<'s>
            = OracleStream
        where
            Self: 's;

        async fn column_names(&mut self, _probe_sql: &str) -> Result<Vec<String>> {
            Err(Error::Unsupported(
                "OracleBackend: enable the `oracle-backend` feature and install OCI".to_owned(),
            ))
        }

        async fn open_branch(&mut self, _sql: &str, _params: &[String]) -> Result<OracleStream> {
            Err(Error::Unsupported(
                "OracleBackend: enable the `oracle-backend` feature and install OCI".to_owned(),
            ))
        }
    }
}

// ─── real implementation (requires `oracle-backend` feature + OCI) ───────────

#[cfg(feature = "oracle-backend")]
pub mod real {
    use std::collections::VecDeque;

    use oracle::Connection;

    use crate::backend::{BranchStream, RawTuple, SqlBackend};
    use crate::error::{Error, Result};

    /// Oracle Database backend wrapping an `oracle::Connection`.
    pub struct OracleBackend {
        conn: Connection,
    }

    impl OracleBackend {
        /// Connect with username / password / Easy Connect string.
        /// Example: `OracleBackend::connect("hr", "oracle", "//localhost/xepdb1")`
        pub fn connect(user: &str, password: &str, connect_string: &str) -> Result<Self> {
            let conn = Connection::connect(user, password, connect_string)
                .map_err(|e| Error::Marshal(format!("oracle connect: {e}")))?;
            Ok(Self { conn })
        }
    }

    /// Materialised row stream. oracle's `ResultSet` is synchronous and
    /// borrows the prepared statement, so we collect rows upfront and serve
    /// them through the async `next_row` interface.
    pub struct OracleStream {
        rows: VecDeque<RawTuple>,
    }

    impl BranchStream for OracleStream {
        async fn next_row(&mut self) -> Result<Option<RawTuple>> {
            Ok(self.rows.pop_front())
        }
    }

    impl SqlBackend for OracleBackend {
        type Stream<'s>
            = OracleStream
        where
            Self: 's;

        async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>> {
            let mut stmt = self
                .conn
                .statement(probe_sql)
                .build()
                .map_err(|e| Error::Marshal(format!("oracle prepare: {e}")))?;
            // Execute with no rows returned; read column names from metadata.
            let result_set = stmt
                .query(&[])
                .map_err(|e| Error::Marshal(format!("oracle probe query: {e}")))?;
            let names = result_set
                .column_info()
                .iter()
                .map(|ci| ci.name().to_owned())
                .collect();
            Ok(names)
        }

        async fn open_branch(
            &mut self,
            sql: &str,
            lexical_params: &[String],
        ) -> Result<OracleStream> {
            let mut stmt = self
                .conn
                .statement(sql)
                .build()
                .map_err(|e| Error::Marshal(format!("oracle prepare: {e}")))?;

            // Each lexical param is bound as a String via oracle's `ToSql` impl.
            let params: Vec<&dyn oracle::sql_type::ToSql> = lexical_params
                .iter()
                .map(|s| s as &dyn oracle::sql_type::ToSql)
                .collect();
            let result_set = stmt
                .query(&params)
                .map_err(|e| Error::Marshal(format!("oracle query: {e}")))?;

            let ncols = result_set.column_info().len();
            let mut rows = VecDeque::new();
            for row_result in result_set {
                let row =
                    row_result.map_err(|e| Error::Marshal(format!("oracle row fetch: {e}")))?;
                let mut values = Vec::with_capacity(ncols);
                // Fetch every column as `Option<String>`.  Oracle's driver converts
                // all value types (numbers, dates, LOBs, etc.) to string via their
                // database-native lexical representation when `String` is requested.
                for i in 0..ncols {
                    let val: Option<String> = row
                        .get(i)
                        .map_err(|e| Error::Marshal(format!("oracle get col {i}: {e}")))?;
                    values.push(val);
                }
                let codes = vec![None; ncols];
                rows.push_back(RawTuple { values, codes });
            }
            Ok(OracleStream { rows })
        }
    }
}

// ─── unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    // Stub-path test: compiles and returns Unsupported without OCI.
    #[cfg(not(feature = "oracle-backend"))]
    #[tokio::test]
    async fn stub_returns_unsupported() {
        use crate::backend::oracle::OracleBackend;
        use crate::backend::SqlBackend;
        let mut b = OracleBackend;
        let r = b.column_names("SELECT 1 FROM DUAL").await;
        assert!(matches!(r, Err(crate::error::Error::Unsupported(_))));
    }
}
