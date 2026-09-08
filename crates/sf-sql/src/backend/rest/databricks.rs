//! Databricks REST backend (Statement Execution API).

use std::collections::VecDeque;

use reqwest::Client;
use serde_json::Value;

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};

use super::shared::{inline_params, json_rows_to_tuples};

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
    /// Construct from workspace URL, SQL warehouse ID, and ******
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
        .ok_or_else(|| Error::Marshal("databricks: missing manifest.schema.columns".to_owned()))?;
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::parse_databricks_response;

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
}

/// Mocked-HTTP test standing in for the live Databricks cloud endpoint — a
/// local `wiremock` server exercises the real request-building +
/// response-parsing path end to end without needing live cloud credentials.
#[cfg(test)]
mod http_mock_tests {
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::DatabricksBackend;
    use crate::backend::SqlBackend;

    #[tokio::test]
    async fn databricks_execute_sql_via_mock_server() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/2.0/sql/statements"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "manifest": {"schema": {"columns": [{"name": "x"}, {"name": "y"}]}},
                "result": {"data_array": [["1", "hello"], ["2", null]]}
            })))
            .mount(&server)
            .await;

        let mut backend = DatabricksBackend::new(server.uri(), "warehouse-123", "fake-token");
        let cols = backend.column_names("SELECT * FROM t").await.unwrap();
        assert_eq!(cols, vec!["x", "y"]);
    }
}
