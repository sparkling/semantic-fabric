//! SAP HANA `SqlBackend` adapter (ADR-0024 M8).
//!
//! Uses the `hdbconnect` crate (synchronous pure-Rust HANA driver). SAP HANA
//! speaks a proprietary TCP protocol (SQLDBC/HANA Protocol); hdbconnect handles
//! the wire layer.
//!
//! Requires the `hana-backend` feature. Without it the module compiles to a
//! stub returning `Error::Unsupported`. A live SAP HANA server is needed for
//! end-to-end testing; the marshalling tests run without a server.
//!
//! Value marshalling: `HdbValue` is converted to a lexical string via its
//! `Display` implementation, which hdbconnect guarantees to produce the HANA
//! lexical form for each type (ISO dates, ISO times, decimal strings, etc.).
//! NULL variants are converted to `None`.
//!
//! Verification tier: compile + unit (marshalling). Live-parity requires a
//! SAP HANA instance and `SF_HANA_URL` env var set.

#[cfg(feature = "hana-backend")]
pub use real::HanaBackend;
#[cfg(feature = "hana-backend")]
pub use real::HanaStream;

#[cfg(not(feature = "hana-backend"))]
pub use stub::HanaBackend;
#[cfg(not(feature = "hana-backend"))]
pub use stub::HanaStream;

// ─── stub (no hdbconnect dep) ────────────────────────────────────────────────

#[cfg(not(feature = "hana-backend"))]
mod stub {
    use crate::backend::{BranchStream, RawTuple, SqlBackend};
    use crate::error::{Error, Result};

    /// SAP HANA stub backend. Enable `hana-backend` feature to activate.
    pub struct HanaBackend;

    /// Stub stream for SAP HANA — never yields rows.
    pub struct HanaStream;

    impl BranchStream for HanaStream {
        async fn next_row(&mut self) -> Result<Option<RawTuple>> {
            Err(Error::Unsupported(
                "HanaBackend: enable the `hana-backend` feature".to_owned(),
            ))
        }
    }

    impl SqlBackend for HanaBackend {
        type Stream<'s>
            = HanaStream
        where
            Self: 's;

        async fn column_names(&mut self, _probe_sql: &str) -> Result<Vec<String>> {
            Err(Error::Unsupported(
                "HanaBackend: enable the `hana-backend` feature".to_owned(),
            ))
        }

        async fn open_branch(&mut self, _sql: &str, _params: &[String]) -> Result<HanaStream> {
            Err(Error::Unsupported(
                "HanaBackend: enable the `hana-backend` feature".to_owned(),
            ))
        }
    }
}

// ─── real implementation (requires `hana-backend` feature) ───────────────────

#[cfg(feature = "hana-backend")]
pub mod real {
    use std::collections::VecDeque;

    use hdbconnect::{Connection, HdbValue};

    use crate::backend::{BranchStream, RawTuple, SqlBackend};
    use crate::error::{Error, Result};

    /// SAP HANA backend wrapping an `hdbconnect::Connection`.
    pub struct HanaBackend {
        conn: Connection,
    }

    impl HanaBackend {
        /// Connect using an `hdbconnect` URL.
        /// Example: `"hdbsql://user:pass@localhost:39013/SYSTEMDB"`
        pub fn connect(url: &str) -> Result<Self> {
            let conn =
                Connection::new(url).map_err(|e| Error::Marshal(format!("hana connect: {e}")))?;
            Ok(Self { conn })
        }
    }

    /// Materialised row stream. `hdbconnect` is synchronous, so rows are
    /// collected upfront and served through the async interface.
    pub struct HanaStream {
        rows: VecDeque<RawTuple>,
    }

    impl BranchStream for HanaStream {
        async fn next_row(&mut self) -> Result<Option<RawTuple>> {
            Ok(self.rows.pop_front())
        }
    }

    impl SqlBackend for HanaBackend {
        type Stream<'s>
            = HanaStream
        where
            Self: 's;

        async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>> {
            let result_set = self
                .conn
                .query(probe_sql)
                .map_err(|e| Error::Marshal(format!("hana probe query: {e}")))?;
            let meta = result_set.metadata();
            let names = meta.iter().map(|c| c.displayname().to_owned()).collect();
            Ok(names)
        }

        async fn open_branch(
            &mut self,
            sql: &str,
            lexical_params: &[String],
        ) -> Result<HanaStream> {
            // Inline params: hdbconnect's serde API requires Serialize tuples
            // matched to the server's parameter metadata; inlining avoids the
            // round-trip metadata fetch and type negotiation for string-typed
            // lexical values (same approach as MonetDB).
            let sql_inlined = inline_params_hana(sql, lexical_params);
            let result_set = self
                .conn
                .query(&sql_inlined)
                .map_err(|e| Error::Marshal(format!("hana query: {e}")))?;

            let ncols = result_set.metadata().len();
            let mut rows = VecDeque::new();
            for row_result in result_set {
                let row = row_result.map_err(|e| Error::Marshal(format!("hana row: {e}")))?;
                let mut values = Vec::with_capacity(ncols);
                for hdb_val in row {
                    values.push(marshal_hdb_value(hdb_val));
                }
                values.resize(ncols, None);
                let codes = vec![None; ncols];
                rows.push_back(RawTuple { values, codes });
            }
            Ok(HanaStream { rows })
        }
    }

    /// Marshal an `HdbValue` to a lexical `Option<String>`.
    /// NULL → `None`. BINARY/GEOMETRY/POINT → uppercase hex.
    /// All other types → their HANA lexical form via `Display`.
    pub fn marshal_hdb_value(v: HdbValue) -> Option<String> {
        match v {
            HdbValue::NULL => None,
            // Binary types: encode as uppercase hex.
            HdbValue::BINARY(bytes)
            | HdbValue::GEOMETRY(bytes)
            | HdbValue::POINT(bytes)
            | HdbValue::DBSTRING(bytes) => {
                let mut out = String::with_capacity(bytes.len() * 2);
                for byte in &bytes {
                    use std::fmt::Write;
                    let _ = write!(out, "{byte:02X}");
                }
                Some(out)
            }
            // All other types: use Display which produces HANA's lexical form.
            other => Some(other.to_string()),
        }
    }

    /// Inline `?` positional params into SQL as single-quoted literals.
    fn inline_params_hana(sql: &str, params: &[String]) -> String {
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

// ─── unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #[cfg(feature = "hana-backend")]
    mod with_driver {
        use super::super::real::marshal_hdb_value;
        use hdbconnect::HdbValue;

        #[test]
        fn marshal_null() {
            assert_eq!(marshal_hdb_value(HdbValue::NULL), None);
        }

        #[test]
        fn marshal_string() {
            assert_eq!(
                marshal_hdb_value(HdbValue::STRING("hello".to_owned())),
                Some("hello".to_owned())
            );
        }

        #[test]
        fn marshal_binary_hex() {
            assert_eq!(
                marshal_hdb_value(HdbValue::BINARY(vec![0xAB, 0xCD])),
                Some("ABCD".to_owned())
            );
        }
    }

    #[cfg(not(feature = "hana-backend"))]
    #[tokio::test]
    async fn stub_returns_unsupported() {
        use crate::backend::hana::HanaBackend;
        use crate::backend::SqlBackend;
        let mut b = HanaBackend;
        let r = b.column_names("SELECT 1 FROM DUMMY").await;
        assert!(matches!(r, Err(crate::error::Error::Unsupported(_))));
    }
}
