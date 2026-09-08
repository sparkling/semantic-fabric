//! AWS JSON 1.1 request bodies and response decoding for Athena.
//!
//! Everything here is pure: build a `serde_json::Value` request, or decode a
//! response `Value`. Errors are returned as plain messages ([`WireResult`]) so
//! that the single call site in the session layer is forced to run them through
//! credential redaction before they become an [`Error`](crate::error::Error) —
//! response bodies are remote-controlled input.

use serde_json::{json, Map, Value};

use crate::backend::RawTuple;

use super::config::{AthenaConfig, MAX_PAGE_SIZE};

/// Athena's documented cap for a `NextToken`.
const MAX_NEXT_TOKEN_LEN: usize = 1024;

/// A decode/validation failure, as an un-redacted message.
pub(crate) type WireResult<T> = std::result::Result<T, String>;

/// Terminal-or-not view of `QueryExecution.Status.State`.
#[derive(Debug)]
pub(crate) enum QueryState {
    /// `QUEUED` / `RUNNING` — keep polling.
    Pending,
    /// `SUCCEEDED` — results are fetchable.
    Succeeded,
    /// `FAILED` — terminal provider failure.
    Failed(String),
    /// `CANCELLED` — terminal provider cancellation.
    Cancelled(String),
}

/// One decoded `GetQueryResults` page.
pub(crate) struct ResultPage {
    pub(crate) columns: Vec<String>,
    pub(crate) rows: Vec<RawTuple>,
    pub(crate) next_token: Option<String>,
}

// ─── Request bodies ──────────────────────────────────────────────────────

/// `StartQueryExecution` body.
///
/// `ExecutionParameters` is omitted entirely when there are no parameters:
/// Athena rejects an empty list.
pub(crate) fn start_query_body(
    config: &AthenaConfig,
    sql: &str,
    client_request_token: &str,
    execution_parameters: &[String],
) -> Value {
    let mut context = Map::new();
    context.insert("Catalog".to_owned(), json!(config.catalog));
    if let Some(database) = &config.database {
        context.insert("Database".to_owned(), json!(database));
    }

    let mut body = Map::new();
    body.insert("QueryString".to_owned(), json!(sql));
    body.insert("ClientRequestToken".to_owned(), json!(client_request_token));
    body.insert("WorkGroup".to_owned(), json!(config.workgroup));
    body.insert("QueryExecutionContext".to_owned(), Value::Object(context));
    if let Some(location) = &config.output_location {
        body.insert(
            "ResultConfiguration".to_owned(),
            json!({ "OutputLocation": location }),
        );
    }
    if !execution_parameters.is_empty() {
        body.insert(
            "ExecutionParameters".to_owned(),
            json!(execution_parameters),
        );
    }
    Value::Object(body)
}

/// Body shared by `GetQueryExecution` and `StopQueryExecution`.
pub(crate) fn query_execution_body(query_execution_id: &str) -> Value {
    json!({ "QueryExecutionId": query_execution_id })
}

/// `GetQueryResults` body. `page_size` is clamped to Athena's 1..=1000 bound as
/// a second line of defence behind `AthenaConfig::with_page_size`.
pub(crate) fn get_query_results_body(
    query_execution_id: &str,
    page_size: u32,
    next_token: Option<&str>,
) -> Value {
    let mut body = Map::new();
    body.insert("QueryExecutionId".to_owned(), json!(query_execution_id));
    body.insert(
        "MaxResults".to_owned(),
        json!(page_size.clamp(1, MAX_PAGE_SIZE)),
    );
    if let Some(token) = next_token {
        body.insert("NextToken".to_owned(), json!(token));
    }
    Value::Object(body)
}

// ─── Response decoding ───────────────────────────────────────────────────

/// Pull `QueryExecutionId` out of a `StartQueryExecution` response.
pub(crate) fn parse_query_execution_id(response: &Value) -> WireResult<String> {
    match response.get("QueryExecutionId").and_then(Value::as_str) {
        Some(id) if !id.is_empty() => Ok(id.to_owned()),
        _ => Err("StartQueryExecution response has no QueryExecutionId".to_owned()),
    }
}

/// Classify `GetQueryExecution` → pending / succeeded / hard failure.
pub(crate) fn parse_query_state(response: &Value) -> WireResult<QueryState> {
    let status = response
        .get("QueryExecution")
        .and_then(|q| q.get("Status"))
        .ok_or_else(|| "GetQueryExecution response has no QueryExecution.Status".to_owned())?;
    let state = status
        .get("State")
        .and_then(Value::as_str)
        .ok_or_else(|| "GetQueryExecution response has no Status.State".to_owned())?;
    let reason = status
        .get("StateChangeReason")
        .and_then(Value::as_str)
        .unwrap_or("no StateChangeReason reported");
    match state {
        "QUEUED" | "RUNNING" => Ok(QueryState::Pending),
        "SUCCEEDED" => Ok(QueryState::Succeeded),
        "FAILED" => Ok(QueryState::Failed(reason.to_owned())),
        "CANCELLED" => Ok(QueryState::Cancelled(reason.to_owned())),
        other => Err(format!("query reported unknown state {other:?}: {reason}")),
    }
}

/// Decode one `GetQueryResults` page.
///
/// `established` carries the column names fixed by the first page; later pages
/// are normalised to that width so a short/long `Data` array can never shift
/// values into the wrong column.
pub(crate) fn parse_result_page(
    response: &Value,
    established: Option<&[String]>,
) -> WireResult<ResultPage> {
    let result_set = response
        .get("ResultSet")
        .ok_or_else(|| "GetQueryResults response has no ResultSet".to_owned())?;
    if !result_set.is_object() {
        return Err("GetQueryResults ResultSet is not an object".to_owned());
    }

    let declared = parse_column_names(result_set)?;
    let columns = match (established, declared) {
        (Some(established), Some(declared)) if declared != established => {
            return Err(format!(
                "GetQueryResults column metadata changed between pages: \
                 expected {established:?}, got {declared:?}"
            ));
        }
        (Some(established), _) => established.to_vec(),
        (None, Some(declared)) => declared,
        (None, None) => {
            return Err("GetQueryResults response has no ResultSetMetadata.ColumnInfo".to_owned());
        }
    };
    let ncols = columns.len();

    let mut rows = Vec::new();
    if let Some(raw_rows) = result_set.get("Rows") {
        let raw_rows = raw_rows
            .as_array()
            .ok_or_else(|| "GetQueryResults ResultSet.Rows is not an array".to_owned())?;
        for raw_row in raw_rows {
            rows.push(parse_row(raw_row, ncols)?);
        }
    }

    Ok(ResultPage {
        columns,
        rows,
        next_token: parse_next_token(response)?,
    })
}

fn parse_column_names(result_set: &Value) -> WireResult<Option<Vec<String>>> {
    let Some(metadata) = result_set.get("ResultSetMetadata") else {
        return Ok(None);
    };
    let metadata = metadata
        .as_object()
        .ok_or_else(|| "GetQueryResults ResultSetMetadata is not an object".to_owned())?;
    let Some(column_info) = metadata.get("ColumnInfo") else {
        return Ok(None);
    };
    let column_info = column_info
        .as_array()
        .ok_or_else(|| "ResultSetMetadata.ColumnInfo is not an array".to_owned())?;
    let mut columns = Vec::with_capacity(column_info.len());
    for (i, column) in column_info.iter().enumerate() {
        // `Name` is the projection name; `Label` is the display alias and is
        // only a sound fallback when the service omitted `Name` entirely.
        let name = column
            .get("Name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .or_else(|| column.get("Label").and_then(Value::as_str))
            .ok_or_else(|| format!("ColumnInfo[{i}] has neither Name nor Label"))?;
        columns.push(name.to_owned());
    }
    Ok(Some(columns))
}

fn parse_row(raw_row: &Value, ncols: usize) -> WireResult<RawTuple> {
    let raw_row = raw_row
        .as_object()
        .ok_or_else(|| "GetQueryResults Row is not an object".to_owned())?;
    let mut values: Vec<Option<String>> = Vec::with_capacity(ncols);
    if let Some(data) = raw_row.get("Data") {
        let data = data
            .as_array()
            .ok_or_else(|| "GetQueryResults Row.Data is not an array".to_owned())?;
        for cell in data {
            let cell = cell
                .as_object()
                .ok_or_else(|| "GetQueryResults Datum is not an object".to_owned())?;
            match cell.get("VarCharValue") {
                // Athena signals SQL NULL by omitting VarCharValue entirely.
                None | Some(Value::Null) => values.push(None),
                Some(Value::String(s)) => values.push(Some(s.clone())),
                Some(other) => {
                    return Err(format!(
                        "GetQueryResults Datum.VarCharValue is not a string: {other}"
                    ))
                }
            }
        }
    }
    // `resize` both pads a short Data array with NULLs and drops any surplus
    // cells, so a malformed row can never shift values into the wrong column.
    values.resize(ncols, None);
    Ok(RawTuple {
        codes: vec![None; ncols],
        values,
    })
}

fn parse_next_token(response: &Value) -> WireResult<Option<String>> {
    match response.get("NextToken") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(token)) => {
            if token.is_empty() || token.len() > MAX_NEXT_TOKEN_LEN {
                return Err(format!(
                    "GetQueryResults NextToken length {} outside 1..={MAX_NEXT_TOKEN_LEN}",
                    token.len()
                ));
            }
            Ok(Some(token.clone()))
        }
        Some(other) => Err(format!(
            "GetQueryResults NextToken is not a string: {other}"
        )),
    }
}

/// Athena repeats the column headers as the first row of the FIRST page for
/// most DML result sets. Drop it only there, and only when every value matches
/// the metadata name exactly — otherwise a legitimate data row whose values
/// happen to look like headers would be silently swallowed.
pub(crate) fn strip_header_row(page: &mut ResultPage) {
    let Some(first) = page.rows.first() else {
        return;
    };
    if first.values.len() != page.columns.len() {
        return;
    }
    let is_header = first
        .values
        .iter()
        .zip(page.columns.iter())
        .all(|(value, column)| value.as_deref() == Some(column.as_str()));
    if is_header {
        page.rows.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::config::AthenaConfig;
    use super::{
        get_query_results_body, parse_query_execution_id, parse_query_state, parse_result_page,
        start_query_body, strip_header_row, QueryState,
    };

    #[test]
    fn start_query_body_uses_pascal_case_aws_shapes() {
        let config = AthenaConfig::new("us-east-1")
            .unwrap()
            .with_database("gtfs")
            .unwrap()
            .with_output_location("s3://bucket/prefix/")
            .unwrap();
        let body = start_query_body(&config, "SELECT ?", "tok", &["'v'".to_owned()]);
        assert_eq!(body["QueryString"], json!("SELECT ?"));
        assert_eq!(body["ClientRequestToken"], json!("tok"));
        assert_eq!(body["WorkGroup"], json!("primary"));
        assert_eq!(body["QueryExecutionContext"]["Database"], json!("gtfs"));
        assert_eq!(
            body["QueryExecutionContext"]["Catalog"],
            json!("AwsDataCatalog")
        );
        assert_eq!(
            body["ResultConfiguration"]["OutputLocation"],
            json!("s3://bucket/prefix/")
        );
        assert_eq!(body["ExecutionParameters"], json!(["'v'"]));
    }

    #[test]
    fn execution_parameters_field_is_omitted_when_empty() {
        let config = AthenaConfig::new("us-east-1").unwrap();
        let body = start_query_body(&config, "SELECT 1", "tok", &[]);
        assert!(body.get("ExecutionParameters").is_none(), "{body}");
        assert!(body.get("ResultConfiguration").is_none(), "{body}");
    }

    #[test]
    fn page_size_is_clamped_into_the_athena_bound() {
        let body = get_query_results_body("q", 5000, Some("tok"));
        assert_eq!(body["MaxResults"], json!(1000));
        assert_eq!(body["NextToken"], json!("tok"));
        let body = get_query_results_body("q", 0, None);
        assert_eq!(body["MaxResults"], json!(1));
        assert!(body.get("NextToken").is_none());
    }

    #[test]
    fn query_states_are_classified() {
        let state = |s: &str| json!({"QueryExecution": {"Status": {"State": s}}});
        assert!(matches!(
            parse_query_state(&state("QUEUED")),
            Ok(QueryState::Pending)
        ));
        assert!(matches!(
            parse_query_state(&state("RUNNING")),
            Ok(QueryState::Pending)
        ));
        assert!(matches!(
            parse_query_state(&state("SUCCEEDED")),
            Ok(QueryState::Succeeded)
        ));
        let failed = json!({"QueryExecution": {"Status": {
            "State": "FAILED", "StateChangeReason": "COLUMN_NOT_FOUND: no such column"
        }}});
        assert!(matches!(
            parse_query_state(&failed),
            Ok(QueryState::Failed(reason)) if reason.contains("COLUMN_NOT_FOUND")
        ));
        let cancelled = json!({"QueryExecution": {"Status": {
            "State": "CANCELLED", "StateChangeReason": "cancelled by caller"
        }}});
        assert!(matches!(
            parse_query_state(&cancelled),
            Ok(QueryState::Cancelled(reason)) if reason.contains("cancelled by caller")
        ));
        assert!(parse_query_state(&state("MOON")).is_err());
        assert!(parse_query_state(&json!({})).is_err());
    }

    #[test]
    fn execution_id_must_be_present() {
        assert_eq!(
            parse_query_execution_id(&json!({"QueryExecutionId": "q-1"})).unwrap(),
            "q-1"
        );
        assert!(parse_query_execution_id(&json!({})).is_err());
        assert!(parse_query_execution_id(&json!({"QueryExecutionId": ""})).is_err());
    }

    #[test]
    fn rows_decode_nulls_and_normalise_width() {
        let response = json!({
            "ResultSet": {
                "ResultSetMetadata": {"ColumnInfo": [{"Name": "a"}, {"Name": "b"}]},
                "Rows": [
                    {"Data": [{"VarCharValue": "a"}, {"VarCharValue": "b"}]},
                    {"Data": [{"VarCharValue": "1"}, {}]},
                    {"Data": [{"VarCharValue": "2"}]},
                    {"Data": [{"VarCharValue": "3"}, {"VarCharValue": "y"}, {"VarCharValue": "z"}]}
                ]
            }
        });
        let mut page = parse_result_page(&response, None).unwrap();
        assert_eq!(page.columns, vec!["a".to_owned(), "b".to_owned()]);
        strip_header_row(&mut page);
        assert_eq!(page.rows.len(), 3, "header row must be dropped");
        assert_eq!(page.rows[0].values, vec![Some("1".to_owned()), None]);
        assert_eq!(page.rows[1].values, vec![Some("2".to_owned()), None]);
        assert_eq!(
            page.rows[2].values,
            vec![Some("3".to_owned()), Some("y".to_owned())]
        );
        assert!(page.next_token.is_none());
    }

    #[test]
    fn header_row_is_only_stripped_on_an_exact_match() {
        let response = json!({
            "ResultSet": {
                "ResultSetMetadata": {"ColumnInfo": [{"Name": "a"}]},
                "Rows": [{"Data": [{"VarCharValue": "A"}]}, {"Data": [{"VarCharValue": "a"}]}]
            }
        });
        let mut page = parse_result_page(&response, None).unwrap();
        strip_header_row(&mut page);
        assert_eq!(
            page.rows.len(),
            2,
            "case-different row is data, not a header"
        );
    }

    #[test]
    fn column_label_is_the_fallback_when_name_is_absent() {
        let response = json!({
            "ResultSet": {"ResultSetMetadata": {"ColumnInfo": [{"Label": "alias"}]}, "Rows": []}
        });
        let page = parse_result_page(&response, None).unwrap();
        assert_eq!(page.columns, vec!["alias".to_owned()]);
    }

    #[test]
    fn later_pages_inherit_the_established_columns() {
        let response = json!({"ResultSet": {"Rows": [{"Data": [{"VarCharValue": "9"}]}]}});
        let established = vec!["n".to_owned()];
        let page = parse_result_page(&response, Some(&established)).unwrap();
        assert_eq!(page.columns, established);
        assert_eq!(page.rows[0].values, vec![Some("9".to_owned())]);
        // Without established columns the same page is a hard error.
        assert!(parse_result_page(&response, None).is_err());

        let changed = json!({
            "ResultSet": {
                "ResultSetMetadata": {"ColumnInfo": [{"Name": "n"}, {"Name": "extra"}]},
                "Rows": [{"Data": [{"VarCharValue": "9"}]}]
            }
        });
        let error = match parse_result_page(&changed, Some(&established)) {
            Err(error) => error,
            Ok(_) => panic!("expected changed page metadata to be rejected"),
        };
        assert!(error.contains("metadata changed"), "{error}");
    }

    #[test]
    fn malformed_shapes_are_hard_errors() {
        assert!(parse_result_page(&json!({}), None).is_err());
        let bad_cell = json!({
            "ResultSet": {
                "ResultSetMetadata": {"ColumnInfo": [{"Name": "a"}]},
                "Rows": [{"Data": [{"VarCharValue": 7}]}]
            }
        });
        assert!(parse_result_page(&bad_cell, None).is_err());
        let long_token = json!({
            "ResultSet": {"ResultSetMetadata": {"ColumnInfo": [{"Name": "a"}]}, "Rows": []},
            "NextToken": "x".repeat(1025)
        });
        assert!(parse_result_page(&long_token, None).is_err());
    }
}
