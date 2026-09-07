//! Trino/PrestoDB REST backend (native Presto REST protocol).

use std::collections::VecDeque;

use reqwest::Client;

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};

use super::shared::{inline_params, presto_execute};

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
    pub fn with_catalog(mut self, catalog: impl Into<String>, schema: impl Into<String>) -> Self {
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
        let sql_with_params = inline_params(sql, params);
        presto_execute(
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

    async fn open_branch(&mut self, sql: &str, lexical_params: &[String]) -> Result<TrinoStream> {
        let (_, tuples) = self.execute_sql(sql, lexical_params).await?;
        Ok(TrinoStream {
            rows: tuples.into(),
        })
    }
}

/// PrestoDB backend — same REST protocol as Trino; type alias.
pub type PrestoDbBackend = TrinoBackend;

/// Mocked-HTTP test standing in for the live Trino cloud endpoint — a local
/// `wiremock` server exercises the real request-building +
/// response-parsing path end to end without needing live cloud credentials.
#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::TrinoBackend;
    use crate::backend::SqlBackend;

    #[tokio::test]
    async fn trino_execute_sql_via_mock_server() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/statement"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "columns": [{"name": "a"}, {"name": "b"}],
                "data": [["1", "2"]]
            })))
            .mount(&server)
            .await;

        let mut backend = TrinoBackend::new(server.uri(), "trino-user");
        let cols = backend.column_names("SELECT * FROM t").await.unwrap();
        assert_eq!(cols, vec!["a", "b"]);
    }
}
