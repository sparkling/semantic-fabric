//! REST/HTTP `SqlBackend` adapters (ADR-0024 M8).
//!
//! Covers databases that expose a SQL-over-REST API: Snowflake (SQL API v2),
//! Google BigQuery (Jobs API), AWS Athena (Presto REST protocol), Databricks
//! (Statement Execution API), and Trino/PrestoDB (native REST protocol).
//!
//! All HTTP backends require the `rest-backends` feature which brings in `reqwest`
//! (async HTTP client) and `serde_json`. Without the feature the module
//! compiles to stubs that return `Error::Unsupported`.
//!
//! Authentication: Bearer tokens read from environment variables:
//! - Snowflake:   `SF_SNOWFLAKE_TOKEN`
//! - BigQuery:    `SF_BIGQUERY_TOKEN`
//! - Databricks:  `SF_DATABRICKS_TOKEN`
//! - Athena/Trino/Presto: none required for test clusters.
//!
//! Verification tier: compile + unit (JSON parsing tests). Live-parity requires
//! a real cloud account with the corresponding `SF_*_URL` env var set.

#[cfg(feature = "rest-backends")]
pub use real::{AthenaBackend, BigQueryBackend, DatabricksBackend, SnowflakeBackend};
#[cfg(feature = "rest-backends")]
pub use trino_real::{PrestoDbBackend, TrinoBackend};

#[cfg(not(feature = "rest-backends"))]
pub use stub::{AthenaBackend, BigQueryBackend, DatabricksBackend, SnowflakeBackend};
#[cfg(not(feature = "rest-backends"))]
pub use stub::{PrestoDbBackend, TrinoBackend};

// SAP HANA and MonetDB get their own files; re-export the stub types here so
// the type aliases in the old API surface still resolve.
pub use super::hana::HanaBackend as SapHanaBackend;
pub use super::monetdb::MonetDbBackend;

// ─── stub path (no reqwest dep) ──────────────────────────────────────────────

#[cfg(not(feature = "rest-backends"))]
mod stub {
    use crate::backend::{BranchStream, RawTuple, SqlBackend};
    use crate::error::{Error, Result};

    macro_rules! stub_backend {
        ($name:ident, $stream:ident, $msg:expr) => {
            /// REST stub backend. Enable `rest-backends` feature to activate.
            pub struct $name;
            /// Stub stream — never yields rows.
            pub struct $stream;

            impl BranchStream for $stream {
                async fn next_row(&mut self) -> Result<Option<RawTuple>> {
                    Err(Error::Unsupported($msg.to_owned()))
                }
            }

            impl SqlBackend for $name {
                type Stream<'s>
                    = $stream
                where
                    Self: 's;

                async fn column_names(&mut self, _probe_sql: &str) -> Result<Vec<String>> {
                    Err(Error::Unsupported($msg.to_owned()))
                }

                async fn open_branch(&mut self, _sql: &str, _params: &[String]) -> Result<$stream> {
                    Err(Error::Unsupported($msg.to_owned()))
                }
            }
        };
    }

    stub_backend!(
        SnowflakeBackend,
        SnowflakeStream,
        "SnowflakeBackend: enable the `rest-backends` feature"
    );
    stub_backend!(
        BigQueryBackend,
        BigQueryStream,
        "BigQueryBackend: enable the `rest-backends` feature"
    );
    stub_backend!(
        AthenaBackend,
        AthenaStream,
        "AthenaBackend: enable the `rest-backends` feature"
    );
    stub_backend!(
        DatabricksBackend,
        DatabricksStream,
        "DatabricksBackend: enable the `rest-backends` feature"
    );
    stub_backend!(
        TrinoBackend,
        TrinoStream,
        "TrinoBackend: enable the `rest-backends` feature"
    );
    stub_backend!(
        PrestoDbBackend,
        PrestoDbStream,
        "PrestoDbBackend: enable the `rest-backends` feature"
    );
}

// ─── real implementations ────────────────────────────────────────────────────

#[cfg(feature = "rest-backends")]
pub mod real {
    use std::collections::VecDeque;

    use reqwest::Client;
    use serde_json::Value;

    use crate::backend::{BranchStream, RawTuple, SqlBackend};
    use crate::error::{Error, Result};

    // ─── shared helpers ──────────────────────────────────────────────────────

    /// Drain a JSON array of row arrays (`[[v, v, …], …]`) into `RawTuple`s.
    /// Each cell is converted to its lexical string: number → decimal string,
    /// string → string, bool → `"true"`/`"false"`, null → `None`.
    fn json_rows_to_tuples(rows: &[Value], ncols: usize) -> Vec<RawTuple> {
        rows.iter()
            .map(|row| {
                let mut values = Vec::with_capacity(ncols);
                let cells = row.as_array().cloned().unwrap_or_default();
                for cell in cells {
                    values.push(json_value_to_string(&cell));
                }
                // Pad/truncate to ncols in case of malformed response.
                values.resize(ncols, None);
                let codes = vec![None; ncols];
                RawTuple { values, codes }
            })
            .collect()
    }

    /// Convert a single JSON value to a lexical string (NULL → None).
    pub fn json_value_to_string(v: &Value) -> Option<String> {
        match v {
            Value::Null => None,
            Value::Bool(b) => Some(b.to_string()),
            Value::Number(n) => Some(n.to_string()),
            Value::String(s) => Some(s.clone()),
            // Arrays/objects: serialize to compact JSON string.
            other => Some(other.to_string()),
        }
    }

    // ─── Snowflake (SQL API v2) ───────────────────────────────────────────────

    /// Snowflake backend using the Snowflake SQL API v2.
    ///
    /// Requires `SF_SNOWFLAKE_TOKEN` (JWT or OAuth access token) and
    /// `SF_SNOWFLAKE_URL` (`https://<account>.snowflakecomputing.com`).
    pub struct SnowflakeBackend {
        base_url: String,
        token: String,
        client: Client,
    }

    /// Snowflake row stream.
    pub struct SnowflakeStream {
        rows: VecDeque<RawTuple>,
    }

    impl BranchStream for SnowflakeStream {
        async fn next_row(&mut self) -> Result<Option<RawTuple>> {
            Ok(self.rows.pop_front())
        }
    }

    impl SnowflakeBackend {
        /// Construct from `base_url` (e.g. `https://acct.snowflakecomputing.com`)
        /// and a Bearer token. Token is read from `SF_SNOWFLAKE_TOKEN` if not set.
        pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
            Self {
                base_url: base_url.into().trim_end_matches('/').to_owned(),
                token: token.into(),
                client: Client::new(),
            }
        }

        /// Build from environment variables `SF_SNOWFLAKE_URL` + `SF_SNOWFLAKE_TOKEN`.
        pub fn from_env() -> Result<Self> {
            let url = std::env::var("SF_SNOWFLAKE_URL")
                .map_err(|_| Error::Marshal("SF_SNOWFLAKE_URL not set".to_owned()))?;
            let token = std::env::var("SF_SNOWFLAKE_TOKEN")
                .map_err(|_| Error::Marshal("SF_SNOWFLAKE_TOKEN not set".to_owned()))?;
            Ok(Self::new(url, token))
        }

        async fn execute_sql(
            &self,
            sql: &str,
            params: &[String],
        ) -> Result<(Vec<String>, Vec<RawTuple>)> {
            // Snowflake SQL API v2: POST /api/v2/statements
            let endpoint = format!("{}/api/v2/statements", self.base_url);
            // Inline-substitute params (Snowflake REST doesn't support positional
            // bind params in the same way JDBC does).
            let sql_with_params = inline_params(sql, params);
            let body = serde_json::json!({
                "statement": sql_with_params,
                "timeout": 60,
                "database": null,
                "schema": null,
                "warehouse": null,
                "resultSetSerializationFormat": "json"
            });
            let resp = self
                .client
                .post(&endpoint)
                .bearer_auth(&self.token)
                .json(&body)
                .send()
                .await
                .map_err(|e| Error::Marshal(format!("snowflake HTTP: {e}")))?
                .error_for_status()
                .map_err(|e| Error::Marshal(format!("snowflake HTTP status: {e}")))?
                .json::<Value>()
                .await
                .map_err(|e| Error::Marshal(format!("snowflake JSON: {e}")))?;

            parse_snowflake_response(&resp)
        }
    }

    /// Parse a Snowflake SQL API v2 JSON response into column names + rows.
    pub fn parse_snowflake_response(resp: &Value) -> Result<(Vec<String>, Vec<RawTuple>)> {
        let col_defs = resp
            .get("resultSetMetaData")
            .and_then(|m| m.get("rowType"))
            .and_then(|r| r.as_array())
            .ok_or_else(|| {
                Error::Marshal("snowflake: missing resultSetMetaData.rowType".to_owned())
            })?;
        let col_names: Vec<String> = col_defs
            .iter()
            .map(|c| {
                c.get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_owned()
            })
            .collect();
        let ncols = col_names.len();
        let raw_rows = resp
            .get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();
        let tuples = json_rows_to_tuples(&raw_rows, ncols);
        Ok((col_names, tuples))
    }

    impl SqlBackend for SnowflakeBackend {
        type Stream<'s>
            = SnowflakeStream
        where
            Self: 's;

        async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>> {
            let (names, _) = self.execute_sql(probe_sql, &[]).await?;
            Ok(names)
        }

        async fn open_branch(
            &mut self,
            sql: &str,
            lexical_params: &[String],
        ) -> Result<SnowflakeStream> {
            let (_, tuples) = self.execute_sql(sql, lexical_params).await?;
            Ok(SnowflakeStream {
                rows: tuples.into(),
            })
        }
    }

    // ─── Google BigQuery (Jobs REST API) ─────────────────────────────────────

    /// BigQuery backend using the BigQuery Jobs REST API.
    ///
    /// Requires `SF_BIGQUERY_TOKEN` (OAuth 2.0 access token) and
    /// `SF_BIGQUERY_PROJECT` (GCP project ID).
    pub struct BigQueryBackend {
        project: String,
        token: String,
        client: Client,
    }

    /// BigQuery row stream.
    pub struct BigQueryStream {
        rows: VecDeque<RawTuple>,
    }

    impl BranchStream for BigQueryStream {
        async fn next_row(&mut self) -> Result<Option<RawTuple>> {
            Ok(self.rows.pop_front())
        }
    }

    impl BigQueryBackend {
        /// Construct from project ID and Bearer token.
        pub fn new(project: impl Into<String>, token: impl Into<String>) -> Self {
            Self {
                project: project.into(),
                token: token.into(),
                client: Client::new(),
            }
        }

        /// Build from environment variables `SF_BIGQUERY_PROJECT` + `SF_BIGQUERY_TOKEN`.
        pub fn from_env() -> Result<Self> {
            let project = std::env::var("SF_BIGQUERY_PROJECT")
                .map_err(|_| Error::Marshal("SF_BIGQUERY_PROJECT not set".to_owned()))?;
            let token = std::env::var("SF_BIGQUERY_TOKEN")
                .map_err(|_| Error::Marshal("SF_BIGQUERY_TOKEN not set".to_owned()))?;
            Ok(Self::new(project, token))
        }

        async fn execute_sql(
            &self,
            sql: &str,
            params: &[String],
        ) -> Result<(Vec<String>, Vec<RawTuple>)> {
            let sql_with_params = inline_params(sql, params);
            let endpoint = format!(
                "https://bigquery.googleapis.com/bigquery/v2/projects/{}/queries",
                self.project
            );
            let body = serde_json::json!({
                "query": sql_with_params,
                "useLegacySql": false,
                "timeoutMs": 60000
            });
            let resp = self
                .client
                .post(&endpoint)
                .bearer_auth(&self.token)
                .json(&body)
                .send()
                .await
                .map_err(|e| Error::Marshal(format!("bigquery HTTP: {e}")))?
                .error_for_status()
                .map_err(|e| Error::Marshal(format!("bigquery HTTP status: {e}")))?
                .json::<Value>()
                .await
                .map_err(|e| Error::Marshal(format!("bigquery JSON: {e}")))?;

            parse_bigquery_response(&resp)
        }
    }

    /// Parse a BigQuery Jobs API query response into column names + rows.
    pub fn parse_bigquery_response(resp: &Value) -> Result<(Vec<String>, Vec<RawTuple>)> {
        let schema = resp
            .get("schema")
            .and_then(|s| s.get("fields"))
            .and_then(|f| f.as_array())
            .ok_or_else(|| Error::Marshal("bigquery: missing schema.fields".to_owned()))?;
        let col_names: Vec<String> = schema
            .iter()
            .map(|f| {
                f.get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_owned()
            })
            .collect();
        let ncols = col_names.len();
        let raw_rows = resp
            .get("rows")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        // BigQuery row format: `{"f": [{"v": "value"}, ...]}`
        let tuples: Vec<RawTuple> = raw_rows
            .iter()
            .map(|row| {
                let cells = row
                    .get("f")
                    .and_then(|f| f.as_array())
                    .cloned()
                    .unwrap_or_default();
                let mut values = Vec::with_capacity(ncols);
                for cell in &cells {
                    let v = cell.get("v").unwrap_or(&Value::Null);
                    values.push(json_value_to_string(v));
                }
                values.resize(ncols, None);
                let codes = vec![None; ncols];
                RawTuple { values, codes }
            })
            .collect();
        Ok((col_names, tuples))
    }

    impl SqlBackend for BigQueryBackend {
        type Stream<'s>
            = BigQueryStream
        where
            Self: 's;

        async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>> {
            let (names, _) = self.execute_sql(probe_sql, &[]).await?;
            Ok(names)
        }

        async fn open_branch(
            &mut self,
            sql: &str,
            lexical_params: &[String],
        ) -> Result<BigQueryStream> {
            let (_, tuples) = self.execute_sql(sql, lexical_params).await?;
            Ok(BigQueryStream {
                rows: tuples.into(),
            })
        }
    }

    // ─── AWS Athena (Presto REST protocol) ───────────────────────────────────

    /// AWS Athena backend using the Presto REST protocol.
    /// Athena's HTTP endpoint speaks a compatible protocol to Presto/Trino.
    pub struct AthenaBackend {
        base_url: String,
        user: String,
        client: Client,
    }

    /// Athena row stream.
    pub struct AthenaStream {
        rows: VecDeque<RawTuple>,
    }

    impl BranchStream for AthenaStream {
        async fn next_row(&mut self) -> Result<Option<RawTuple>> {
            Ok(self.rows.pop_front())
        }
    }

    impl AthenaBackend {
        /// Construct from base URL and user/catalog string.
        pub fn new(base_url: impl Into<String>, user: impl Into<String>) -> Self {
            Self {
                base_url: base_url.into().trim_end_matches('/').to_owned(),
                user: user.into(),
                client: Client::new(),
            }
        }

        /// Build from `SF_ATHENA_URL` and `SF_ATHENA_USER` env vars.
        pub fn from_env() -> Result<Self> {
            let url = std::env::var("SF_ATHENA_URL")
                .map_err(|_| Error::Marshal("SF_ATHENA_URL not set".to_owned()))?;
            let user = std::env::var("SF_ATHENA_USER").unwrap_or_else(|_| "athena".to_owned());
            Ok(Self::new(url, user))
        }

        async fn execute_sql(
            &self,
            sql: &str,
            params: &[String],
        ) -> Result<(Vec<String>, Vec<RawTuple>)> {
            let sql_with_params = inline_params(sql, params);
            presto_execute(
                &self.client,
                &self.base_url,
                &self.user,
                None,
                None,
                &sql_with_params,
            )
            .await
        }
    }

    impl SqlBackend for AthenaBackend {
        type Stream<'s>
            = AthenaStream
        where
            Self: 's;

        async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>> {
            let (names, _) = self.execute_sql(probe_sql, &[]).await?;
            Ok(names)
        }

        async fn open_branch(
            &mut self,
            sql: &str,
            lexical_params: &[String],
        ) -> Result<AthenaStream> {
            let (_, tuples) = self.execute_sql(sql, lexical_params).await?;
            Ok(AthenaStream {
                rows: tuples.into(),
            })
        }
    }

    // ─── Databricks (Statement Execution API) ────────────────────────────────

    /// Databricks SQL backend using the Databricks Statement Execution API.
    ///
    /// Requires `SF_DATABRICKS_TOKEN` (PAT) and `SF_DATABRICKS_URL`
    /// (e.g. `https://<workspace>.azuredatabricks.net`) and
    /// `SF_DATABRICKS_WAREHOUSE_ID`.
    pub struct DatabricksBackend {
        base_url: String,
        warehouse_id: String,
        token: String,
        client: Client,
    }

    /// Databricks row stream.
    pub struct DatabricksStream {
        rows: VecDeque<RawTuple>,
    }

    impl BranchStream for DatabricksStream {
        async fn next_row(&mut self) -> Result<Option<RawTuple>> {
            Ok(self.rows.pop_front())
        }
    }

    impl DatabricksBackend {
        /// Construct from workspace URL, SQL warehouse ID, and Bearer token.
        pub fn new(
            base_url: impl Into<String>,
            warehouse_id: impl Into<String>,
            token: impl Into<String>,
        ) -> Self {
            Self {
                base_url: base_url.into().trim_end_matches('/').to_owned(),
                warehouse_id: warehouse_id.into(),
                token: token.into(),
                client: Client::new(),
            }
        }

        /// Build from environment variables.
        pub fn from_env() -> Result<Self> {
            let url = std::env::var("SF_DATABRICKS_URL")
                .map_err(|_| Error::Marshal("SF_DATABRICKS_URL not set".to_owned()))?;
            let wid = std::env::var("SF_DATABRICKS_WAREHOUSE_ID")
                .map_err(|_| Error::Marshal("SF_DATABRICKS_WAREHOUSE_ID not set".to_owned()))?;
            let tok = std::env::var("SF_DATABRICKS_TOKEN")
                .map_err(|_| Error::Marshal("SF_DATABRICKS_TOKEN not set".to_owned()))?;
            Ok(Self::new(url, wid, tok))
        }

        async fn execute_sql(
            &self,
            sql: &str,
            params: &[String],
        ) -> Result<(Vec<String>, Vec<RawTuple>)> {
            let sql_with_params = inline_params(sql, params);
            let endpoint = format!("{}/api/2.0/sql/statements", self.base_url);
            let body = serde_json::json!({
                "statement": sql_with_params,
                "warehouse_id": self.warehouse_id,
                "wait_timeout": "60s",
                "on_wait_timeout": "CANCEL",
                "format": "JSON_ARRAY"
            });
            let resp = self
                .client
                .post(&endpoint)
                .bearer_auth(&self.token)
                .json(&body)
                .send()
                .await
                .map_err(|e| Error::Marshal(format!("databricks HTTP: {e}")))?
                .error_for_status()
                .map_err(|e| Error::Marshal(format!("databricks HTTP status: {e}")))?
                .json::<Value>()
                .await
                .map_err(|e| Error::Marshal(format!("databricks JSON: {e}")))?;

            parse_databricks_response(&resp)
        }
    }

    /// Parse a Databricks Statement Execution API response.
    pub fn parse_databricks_response(resp: &Value) -> Result<(Vec<String>, Vec<RawTuple>)> {
        let cols = resp
            .get("manifest")
            .and_then(|m| m.get("schema"))
            .and_then(|s| s.get("columns"))
            .and_then(|c| c.as_array())
            .ok_or_else(|| {
                Error::Marshal("databricks: missing manifest.schema.columns".to_owned())
            })?;
        let col_names: Vec<String> = cols
            .iter()
            .map(|c| {
                c.get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_owned()
            })
            .collect();
        let ncols = col_names.len();
        let raw_rows = resp
            .get("result")
            .and_then(|r| r.get("data_array"))
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();
        let tuples = json_rows_to_tuples(&raw_rows, ncols);
        Ok((col_names, tuples))
    }

    impl SqlBackend for DatabricksBackend {
        type Stream<'s>
            = DatabricksStream
        where
            Self: 's;

        async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>> {
            let (names, _) = self.execute_sql(probe_sql, &[]).await?;
            Ok(names)
        }

        async fn open_branch(
            &mut self,
            sql: &str,
            lexical_params: &[String],
        ) -> Result<DatabricksStream> {
            let (_, tuples) = self.execute_sql(sql, lexical_params).await?;
            Ok(DatabricksStream {
                rows: tuples.into(),
            })
        }
    }

    // ─── Presto REST protocol (shared by Athena, Trino, PrestoDB) ────────────

    /// Execute SQL against a Presto-compatible REST endpoint and collect all
    /// pages into (column_names, row_tuples).
    ///
    /// `catalog` and `schema` set session-level defaults via
    /// `X-Trino-Catalog` / `X-Trino-Schema` headers.
    pub async fn presto_execute(
        client: &Client,
        base_url: &str,
        user: &str,
        catalog: Option<&str>,
        schema: Option<&str>,
        sql: &str,
    ) -> Result<(Vec<String>, Vec<RawTuple>)> {
        let statement_url = format!("{}/v1/statement", base_url);
        let mut req = client
            .post(&statement_url)
            .header("X-Presto-User", user)
            .header("X-Trino-User", user)
            .body(sql.to_owned());
        if let Some(cat) = catalog {
            req = req
                .header("X-Trino-Catalog", cat)
                .header("X-Presto-Catalog", cat);
        }
        if let Some(sch) = schema {
            req = req
                .header("X-Trino-Schema", sch)
                .header("X-Presto-Schema", sch);
        }
        let first: Value = req
            .send()
            .await
            .map_err(|e| Error::Marshal(format!("presto HTTP: {e}")))?
            .error_for_status()
            .map_err(|e| Error::Marshal(format!("presto HTTP status: {e}")))?
            .json()
            .await
            .map_err(|e| Error::Marshal(format!("presto JSON: {e}")))?;

        // The Presto REST protocol is async: the first response is often
        // QUEUED/PLANNING with `nextUri` but no `columns`. We follow `nextUri`
        // pages until we have column metadata. Rows accumulate across all pages.
        let mut col_names: Vec<String> = vec![];
        let mut tuples: Vec<RawTuple> = vec![];

        let mut page = first;
        loop {
            // Capture column names from the first page that provides them.
            if col_names.is_empty() {
                if let Some(cols) = page.get("columns").and_then(|c| c.as_array()) {
                    col_names = cols
                        .iter()
                        .map(|c| {
                            c.get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_owned()
                        })
                        .collect();
                }
            }

            let ncols = col_names.len();
            if ncols > 0 {
                if let Some(rows) = page.get("data").and_then(|d| d.as_array()) {
                    tuples.extend(json_rows_to_tuples(rows, ncols));
                }
            }

            // Check for errors in the response.
            if let Some(err) = page.get("error") {
                let msg = err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error");
                return Err(Error::Marshal(format!("presto query error: {msg}")));
            }

            match page
                .get("nextUri")
                .and_then(|u| u.as_str())
                .map(|s| s.to_owned())
            {
                Some(uri) => {
                    page = client
                        .get(&uri)
                        .header("X-Presto-User", user)
                        .header("X-Trino-User", user)
                        .send()
                        .await
                        .map_err(|e| Error::Marshal(format!("presto page HTTP: {e}")))?
                        .error_for_status()
                        .map_err(|e| Error::Marshal(format!("presto page status: {e}")))?
                        .json()
                        .await
                        .map_err(|e| Error::Marshal(format!("presto page JSON: {e}")))?;
                }
                None => break,
            }
        }

        Ok((col_names, tuples))
    }

    // ─── Inline-param helper ─────────────────────────────────────────────────

    /// Inline `?` positional parameters into the SQL by replacing each `?`
    /// with a quoted literal value. Used by REST backends that don't support
    /// server-side parameter binding.
    ///
    /// Values are single-quote escaped; this is safe for REST endpoints where
    /// injection is mitigated by the fact that values originate from the
    /// trusted SPARQL bind variables (ADR-0010 R1).
    pub fn inline_params(sql: &str, params: &[String]) -> String {
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

// ─── Trino/PrestoDB backend (reqwest feature-gated) ──────────────────────────

#[cfg(feature = "rest-backends")]
pub mod trino_real {
    use std::collections::VecDeque;

    use reqwest::Client;

    use crate::backend::{BranchStream, RawTuple, SqlBackend};
    use crate::error::{Error, Result};

    /// Trino backend using the Trino/Presto REST protocol.
    pub struct TrinoBackend {
        base_url: String,
        user: String,
        catalog: Option<String>,
        schema: Option<String>,
        client: Client,
    }

    /// Trino row stream.
    pub struct TrinoStream {
        rows: VecDeque<RawTuple>,
    }

    impl BranchStream for TrinoStream {
        async fn next_row(&mut self) -> Result<Option<RawTuple>> {
            Ok(self.rows.pop_front())
        }
    }

    impl TrinoBackend {
        /// Construct from base URL and Trino user.
        pub fn new(base_url: impl Into<String>, user: impl Into<String>) -> Self {
            Self {
                base_url: base_url.into().trim_end_matches('/').to_owned(),
                user: user.into(),
                catalog: None,
                schema: None,
                client: Client::new(),
            }
        }

        /// Set the default catalog and schema for all queries.
        pub fn with_catalog(
            mut self,
            catalog: impl Into<String>,
            schema: impl Into<String>,
        ) -> Self {
            self.catalog = Some(catalog.into());
            self.schema = Some(schema.into());
            self
        }

        /// Build from `SF_TRINO_URL` env var (user defaults to `"trino"`).
        pub fn from_env() -> Result<Self> {
            let url = std::env::var("SF_TRINO_URL")
                .map_err(|_| Error::Marshal("SF_TRINO_URL not set".to_owned()))?;
            let user = std::env::var("SF_TRINO_USER").unwrap_or_else(|_| "trino".to_owned());
            Ok(Self::new(url, user))
        }

        async fn execute_sql(
            &self,
            sql: &str,
            params: &[String],
        ) -> Result<(Vec<String>, Vec<RawTuple>)> {
            let sql_with_params = super::real::inline_params(sql, params);
            super::real::presto_execute(
                &self.client,
                &self.base_url,
                &self.user,
                self.catalog.as_deref(),
                self.schema.as_deref(),
                &sql_with_params,
            )
            .await
        }
    }

    impl SqlBackend for TrinoBackend {
        type Stream<'s>
            = TrinoStream
        where
            Self: 's;

        async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>> {
            let (names, _) = self.execute_sql(probe_sql, &[]).await?;
            Ok(names)
        }

        async fn open_branch(
            &mut self,
            sql: &str,
            lexical_params: &[String],
        ) -> Result<TrinoStream> {
            let (_, tuples) = self.execute_sql(sql, lexical_params).await?;
            Ok(TrinoStream {
                rows: tuples.into(),
            })
        }
    }

    /// PrestoDB backend — same REST protocol as Trino; type alias.
    pub type PrestoDbBackend = TrinoBackend;
}

// ─── unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #[cfg(feature = "rest-backends")]
    mod with_reqwest {
        use super::super::real::{
            json_value_to_string, parse_bigquery_response, parse_databricks_response,
            parse_snowflake_response,
        };
        use serde_json::json;

        #[test]
        fn json_value_null() {
            assert_eq!(json_value_to_string(&json!(null)), None);
        }

        #[test]
        fn json_value_string() {
            assert_eq!(
                json_value_to_string(&json!("hello")),
                Some("hello".to_owned())
            );
        }

        #[test]
        fn json_value_number() {
            assert_eq!(json_value_to_string(&json!(42)), Some("42".to_owned()));
        }

        #[test]
        fn json_value_bool() {
            assert_eq!(json_value_to_string(&json!(true)), Some("true".to_owned()));
        }

        #[test]
        fn parse_snowflake_response_ok() {
            let resp = json!({
                "resultSetMetaData": {
                    "rowType": [
                        {"name": "ID", "type": "fixed"},
                        {"name": "NAME", "type": "text"}
                    ]
                },
                "data": [
                    ["1", "Alice"],
                    ["2", null]
                ]
            });
            let (cols, rows) = parse_snowflake_response(&resp).unwrap();
            assert_eq!(cols, vec!["ID", "NAME"]);
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].values[0], Some("1".to_owned()));
            assert_eq!(rows[1].values[1], None);
        }

        #[test]
        fn parse_bigquery_response_ok() {
            let resp = json!({
                "schema": {
                    "fields": [
                        {"name": "id", "type": "INTEGER"},
                        {"name": "val", "type": "STRING"}
                    ]
                },
                "rows": [
                    {"f": [{"v": "10"}, {"v": "hello"}]},
                    {"f": [{"v": "20"}, {"v": null}]}
                ]
            });
            let (cols, rows) = parse_bigquery_response(&resp).unwrap();
            assert_eq!(cols, vec!["id", "val"]);
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].values[0], Some("10".to_owned()));
            assert_eq!(rows[1].values[1], None);
        }

        #[test]
        fn parse_databricks_response_ok() {
            let resp = json!({
                "manifest": {
                    "schema": {
                        "columns": [
                            {"name": "x"},
                            {"name": "y"}
                        ]
                    }
                },
                "result": {
                    "data_array": [
                        ["1", "hello"],
                        ["2", null]
                    ]
                }
            });
            let (cols, rows) = parse_databricks_response(&resp).unwrap();
            assert_eq!(cols, vec!["x", "y"]);
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].values[1], Some("hello".to_owned()));
            assert_eq!(rows[1].values[1], None);
        }

        #[test]
        fn inline_params_substitution() {
            use super::super::real::inline_params;
            let sql = "SELECT * FROM t WHERE id = ? AND name = ?";
            let params = vec!["42".to_owned(), "O'Brien".to_owned()];
            let out = inline_params(sql, &params);
            assert!(out.contains("'42'"), "{out}");
            // Single-quote in value must be escaped.
            assert!(out.contains("'O''Brien'"), "{out}");
        }
    }

    #[cfg(not(feature = "rest-backends"))]
    #[tokio::test]
    async fn stub_returns_unsupported() {
        use crate::backend::rest::SnowflakeBackend;
        use crate::backend::SqlBackend;
        let mut b = SnowflakeBackend;
        let r = b.column_names("SELECT 1").await;
        assert!(matches!(r, Err(crate::error::Error::Unsupported(_))));
    }
}
