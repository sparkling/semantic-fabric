//! Snowflake REST backend (SQL API v2).

use std::collections::VecDeque;

use reqwest::Client;
use serde_json::Value;

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};

use super::shared::{inline_params, json_rows_to_tuples};

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
    /// and a ****** Token is read from `SF_SNOWFLAKE_TOKEN` if not set.
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
        .ok_or_else(|| Error::Marshal("snowflake: missing resultSetMetaData.rowType".to_owned()))?;
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::parse_snowflake_response;

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
}

/// Mocked-HTTP test standing in for the live Snowflake cloud endpoint — a
/// local `wiremock` server exercises the real request-building +
/// response-parsing path end to end without needing live cloud credentials.
#[cfg(test)]
mod http_mock_tests {
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::SnowflakeBackend;
    use crate::backend::SqlBackend;

    #[tokio::test]
    async fn snowflake_execute_sql_via_mock_server() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v2/statements"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "resultSetMetaData": {
                    "rowType": [{"name": "ID", "type": "fixed"}, {"name": "NAME", "type": "text"}]
                },
                "data": [["1", "Alice"], ["2", null]]
            })))
            .mount(&server)
            .await;

        let mut backend = SnowflakeBackend::new(server.uri(), "fake-token");
        let cols = backend.column_names("SELECT * FROM t").await.unwrap();
        assert_eq!(cols, vec!["ID", "NAME"]);
    }
}
