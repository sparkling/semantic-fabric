//! Google BigQuery REST backend (Jobs API).

use std::collections::VecDeque;

use reqwest::Client;
use serde_json::Value;

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};

use super::shared::{inline_params, json_value_to_string};

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
    /// Construct from project ID and ******
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::parse_bigquery_response;

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
}

// BigQuery hardcodes `bigquery.googleapis.com` and can't be redirected to a
// local mock server without a code change, so unlike the other providers
// there is no `http_mock_tests` module here.
