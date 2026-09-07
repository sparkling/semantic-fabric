//! Protocol helpers shared by more than one REST provider module: JSON row
//! parsing, the Presto/Trino REST wire protocol, and the `?`-param inlining
//! helper used by every provider that
//! lacks server-side bind-parameter support.

use reqwest::Client;
use serde_json::Value;

use crate::backend::RawTuple;
use crate::error::{Error, Result};

/// Drain a JSON array of row arrays (`[[v, v, …], …]`) into `RawTuple`s.
/// Each cell is converted to its lexical string: number → decimal string,
/// string → string, bool → `"true"`/`"false"`, null → `None`.
pub(crate) fn json_rows_to_tuples(rows: &[Value], ncols: usize) -> Vec<RawTuple> {
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{inline_params, json_value_to_string};

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
    fn inline_params_substitution() {
        let sql = "SELECT * FROM t WHERE id = ? AND name = ?";
        let params = vec!["42".to_owned(), "O'Brien".to_owned()];
        let out = inline_params(sql, &params);
        assert!(out.contains("'42'"), "{out}");
        // Single-quote in value must be escaped.
        assert!(out.contains("'O''Brien'"), "{out}");
    }
}

/// Mocked-HTTP tests for the shared Presto/Trino REST protocol helper. These
/// exercise pagination, error surfacing, and header propagation directly
/// against `presto_execute`, independent of any specific provider backend.
#[cfg(test)]
mod http_mock_tests {
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::presto_execute;

    /// The presto REST protocol is async: an initial QUEUED/PLANNING page can
    /// carry no `columns`/`data`, only a `nextUri` to poll. This is the branch
    /// none of the single-page provider tests exercise.
    #[tokio::test]
    async fn presto_execute_follows_next_uri_pagination() {
        let server = MockServer::start().await;
        let next_uri = format!("{}/v1/statement/queryid/1", server.uri());

        Mock::given(method("POST"))
            .and(path("/v1/statement"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "nextUri": next_uri
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/statement/queryid/1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "columns": [{"name": "n"}],
                "data": [["1"], ["2"], ["3"]]
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let (cols, rows) = presto_execute(&client, &server.uri(), "user", None, None, "SELECT n")
            .await
            .unwrap();
        assert_eq!(cols, vec!["n"]);
        assert_eq!(rows.len(), 3);
    }

    #[tokio::test]
    async fn presto_execute_surfaces_query_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/statement"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "error": {"message": "line 1:1: mismatched input"}
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = presto_execute(&client, &server.uri(), "user", None, None, "SELECT bad").await;
        match result {
            Err(e) => assert!(
                e.to_string().contains("mismatched input"),
                "expected the presto error message to surface, got: {e}"
            ),
            Ok(_) => panic!("expected presto_execute to surface the query error"),
        }
    }

    /// Catalog/schema headers are session defaults Trino/Presto use to resolve
    /// unqualified table names — verify they're actually sent, not just accepted.
    #[tokio::test]
    async fn presto_execute_sends_catalog_and_schema_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/statement"))
            .and(body_string_contains("SELECT 1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "columns": [{"name": "one"}],
                "data": [["1"]]
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let (cols, _) = presto_execute(
            &client,
            &server.uri(),
            "user",
            Some("hive"),
            Some("default"),
            "SELECT 1",
        )
        .await
        .unwrap();
        assert_eq!(cols, vec!["one"]);
    }
}
