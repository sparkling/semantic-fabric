//! Mocked-HTTP integration coverage for the real AWS Athena backend.
//!
//! A local `wiremock` server stands in for the Athena JSON 1.1 endpoint, so the
//! full request-building → SigV4-signing → polling → lazy-paging path runs end
//! to end without live AWS credentials. Request bodies and responses use the
//! real PascalCase AWS shapes, and matchers assert on `X-Amz-Target` exactly as
//! Athena routes operations.
//!
//! The Presto/Trino REST regression suite lives with the `presto`/`shared`
//! modules; Athena no longer speaks that protocol.

#![cfg(feature = "athena-backend")]

use std::time::Duration;

use serde_json::{json, Value};
use sf_sql::backend::rest::{AthenaBackend, AthenaConfig, AthenaCredentials};
use sf_sql::backend::{BranchStream, SqlBackend};
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const QUERY_ID: &str = "11111111-2222-3333-4444-555555555555";
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
const SESSION_TOKEN: &str = "FQoGZXIvYXdzEXAMPLESESSIONTOKEN";

fn target(operation: &str) -> impl wiremock::Match {
    header("x-amz-target", format!("AmazonAthena.{operation}").as_str())
}

fn athena_backend(server: &MockServer) -> AthenaBackend {
    let config = AthenaConfig::new("us-east-1")
        .unwrap()
        .with_endpoint(server.uri())
        .unwrap()
        .with_database("gtfs")
        .unwrap()
        .with_output_location("s3://results/prefix/")
        .unwrap()
        .with_poll_interval(Duration::from_millis(1))
        .unwrap()
        .with_retry_backoff(Duration::from_millis(1))
        .unwrap()
        .with_page_size(2)
        .unwrap();
    let credentials = AthenaCredentials::new("AKIDEXAMPLE", SECRET_KEY)
        .unwrap()
        .with_session_token(SESSION_TOKEN)
        .unwrap();
    AthenaBackend::new(config, credentials).unwrap()
}

/// `{"ResultSet": {...}}` with the header row Athena prepends to page 1.
fn result_page(columns: &[&str], rows: &[&[&str]], next_token: Option<&str>) -> Value {
    let column_info: Vec<Value> = columns.iter().map(|c| json!({"Name": c})).collect();
    let rows: Vec<Value> = rows
        .iter()
        .map(|row| json!({"Data": row.iter().map(|v| json!({"VarCharValue": v})).collect::<Vec<_>>()}))
        .collect();
    let mut page = json!({
        "ResultSet": {
            "ResultSetMetadata": {"ColumnInfo": column_info},
            "Rows": rows
        }
    });
    if let Some(token) = next_token {
        page["NextToken"] = json!(token);
    }
    page
}

fn succeeded() -> Value {
    json!({"QueryExecution": {"QueryExecutionId": QUERY_ID,
        "Status": {"State": "SUCCEEDED"}}})
}

async fn mount_start(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/"))
        .and(target("StartQueryExecution"))
        .and(header("content-type", "application/x-amz-json-1.1"))
        .and(header_exists("authorization"))
        .and(header_exists("x-amz-date"))
        .and(header("x-amz-security-token", SESSION_TOKEN))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "QueryExecutionId": QUERY_ID
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn start_poll_and_fetch_signs_every_request_and_returns_rows() {
    let server = MockServer::start().await;
    mount_start(&server).await;
    Mock::given(method("POST"))
        .and(target("GetQueryExecution"))
        .and(header_exists("authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_json(succeeded()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(target("GetQueryResults"))
        .and(header_exists("authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_json(result_page(
            &["route_id", "route_name"],
            &[
                &["route_id", "route_name"],
                &["R1", "Red Line"],
                &["R2", "Blue Line"],
            ],
            None,
        )))
        .mount(&server)
        .await;

    let mut backend = athena_backend(&server);
    let mut stream = backend
        .open_branch(
            "SELECT route_id, route_name FROM routes WHERE agency = ?",
            &["MBTA".to_owned()],
        )
        .await
        .unwrap();

    let first = stream.next_row().await.unwrap().unwrap();
    assert_eq!(
        first.values,
        vec![Some("R1".to_owned()), Some("Red Line".to_owned())]
    );
    let second = stream.next_row().await.unwrap().unwrap();
    assert_eq!(
        second.values,
        vec![Some("R2".to_owned()), Some("Blue Line".to_owned())]
    );
    assert!(stream.next_row().await.unwrap().is_none());

    // The header row must have been dropped, and the parameter must have gone
    // out as ExecutionParameters — never spliced into QueryString.
    let start = start_request_body(&server).await;
    assert_eq!(start["ExecutionParameters"], json!(["'MBTA'"]));
    assert_eq!(
        start["QueryString"],
        json!("SELECT route_id, route_name FROM routes WHERE agency = ?")
    );
    assert_eq!(start["QueryExecutionContext"]["Database"], json!("gtfs"));
    assert_eq!(
        start["ResultConfiguration"]["OutputLocation"],
        json!("s3://results/prefix/")
    );
    let token = start["ClientRequestToken"].as_str().unwrap();
    assert!((32..=128).contains(&token.len()), "token {token:?}");
}

#[tokio::test]
async fn column_names_reads_metadata_off_the_first_page() {
    let server = MockServer::start().await;
    mount_start(&server).await;
    Mock::given(method("POST"))
        .and(target("GetQueryExecution"))
        .respond_with(ResponseTemplate::new(200).set_body_json(succeeded()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(target("GetQueryResults"))
        .respond_with(ResponseTemplate::new(200).set_body_json(result_page(
            &["route_id", "route_name"],
            &[&["route_id", "route_name"]],
            None,
        )))
        .mount(&server)
        .await;

    let mut backend = athena_backend(&server);
    let columns = backend
        .column_names("SELECT * FROM \"routes\" LIMIT 0")
        .await
        .unwrap();
    assert_eq!(columns, vec!["route_id", "route_name"]);
}

#[tokio::test]
async fn second_page_is_fetched_only_after_the_first_one_drains() {
    let server = MockServer::start().await;
    mount_start(&server).await;
    Mock::given(method("POST"))
        .and(target("GetQueryExecution"))
        .respond_with(ResponseTemplate::new(200).set_body_json(succeeded()))
        .mount(&server)
        .await;
    // Page 1 (no NextToken echoed back) → hands out a token for page 2.
    Mock::given(method("POST"))
        .and(target("GetQueryResults"))
        .and(no_next_token())
        .respond_with(ResponseTemplate::new(200).set_body_json(result_page(
            &["n"],
            &[&["n"], &["1"], &["2"]],
            Some("PAGE2TOKEN"),
        )))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(target("GetQueryResults"))
        .and(next_token_is("PAGE2TOKEN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(result_page(
            &["n"],
            &[&["3"]],
            None,
        )))
        .mount(&server)
        .await;

    let mut backend = athena_backend(&server);
    let mut stream = backend.open_branch("SELECT n FROM t", &[]).await.unwrap();

    assert_eq!(
        page_two_requests(&server).await,
        0,
        "page 2 fetched eagerly"
    );
    assert_eq!(row(&mut stream).await, "1");
    assert_eq!(
        page_two_requests(&server).await,
        0,
        "page 2 fetched while page 1 still had rows"
    );
    assert_eq!(row(&mut stream).await, "2");
    assert_eq!(
        page_two_requests(&server).await,
        0,
        "page 2 must not be fetched until a row is actually pulled past page 1"
    );
    // Page 1 is drained only now, so the next pull must trigger exactly one fetch.
    assert_eq!(row(&mut stream).await, "3");
    assert_eq!(page_two_requests(&server).await, 1);
    assert!(stream.next_row().await.unwrap().is_none());
    assert_eq!(page_two_requests(&server).await, 1, "paging did not stop");
}

#[tokio::test]
async fn failed_state_surfaces_the_state_change_reason() {
    let server = MockServer::start().await;
    mount_start(&server).await;
    Mock::given(method("POST"))
        .and(target("GetQueryExecution"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "QueryExecution": {"QueryExecutionId": QUERY_ID, "Status": {
                "State": "FAILED",
                "StateChangeReason": "COLUMN_NOT_FOUND: line 1:8: Column 'nope' cannot be resolved"
            }}
        })))
        .mount(&server)
        .await;

    let mut backend = athena_backend(&server);
    let error = backend
        .open_branch("SELECT nope FROM t", &[])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("FAILED"), "{error}");
    assert!(error.contains("COLUMN_NOT_FOUND"), "{error}");
}

#[tokio::test]
async fn hostile_error_body_cannot_leak_credentials_into_diagnostics() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(target("StartQueryExecution"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "__type": "com.amazonaws.athena#InvalidRequestException",
            "message": format!(
                "rejected credentials AKIDEXAMPLE / {SECRET_KEY} / {SESSION_TOKEN}"
            )
        })))
        .mount(&server)
        .await;

    let mut backend = athena_backend(&server);
    let error = backend
        .open_branch("SELECT 1", &[])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("InvalidRequestException"), "{error}");
    assert!(!error.contains(SECRET_KEY), "secret leaked: {error}");
    assert!(!error.contains(SESSION_TOKEN), "token leaked: {error}");
    assert!(!error.contains("AKIDEXAMPLE"), "access key leaked: {error}");
}

#[tokio::test]
async fn throttling_is_retried_but_validation_failures_are_not() {
    let server = MockServer::start().await;
    // Athena reports throttling as HTTP 400, so only the shape distinguishes it.
    Mock::given(method("POST"))
        .and(target("StartQueryExecution"))
        .respond_with(
            ResponseTemplate::new(400)
                .append_header("x-amzn-ErrorType", "TooManyRequestsException:")
                .set_body_json(json!({
                    "__type": "TooManyRequestsException", "message": "Rate exceeded"
                })),
        )
        .expect(4) // initial attempt + the default 3 retries
        .mount(&server)
        .await;

    let mut backend = athena_backend(&server);
    let error = backend
        .open_branch("SELECT 1", &[])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("TooManyRequestsException"), "{error}");
    server.verify().await;
    let retry_tokens: std::collections::HashSet<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| is_target(request, "StartQueryExecution"))
        .map(|request| {
            body_of(request)["ClientRequestToken"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        retry_tokens.len(),
        1,
        "retries of one logical execution must reuse its idempotency token"
    );
    drop(server);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(target("StartQueryExecution"))
        .respond_with(
            ResponseTemplate::new(400)
                .append_header("x-amzn-ErrorType", "InvalidRequestException:")
                .set_body_json(json!({
                    "__type": "InvalidRequestException", "message": "bad SQL"
                })),
        )
        .expect(1) // no retry for a validation error
        .mount(&server)
        .await;

    let mut strict = athena_backend(&server);
    assert!(strict.open_branch("SELECT 1", &[]).await.is_err());
    server.verify().await;
}

#[tokio::test]
async fn repeated_next_token_is_rejected_instead_of_looping() {
    let server = MockServer::start().await;
    mount_start(&server).await;
    Mock::given(method("POST"))
        .and(target("GetQueryExecution"))
        .respond_with(ResponseTemplate::new(200).set_body_json(succeeded()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(target("GetQueryResults"))
        .and(no_next_token())
        .respond_with(ResponseTemplate::new(200).set_body_json(result_page(
            &["n"],
            &[&["n"], &["1"]],
            Some("STUCK"),
        )))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(target("GetQueryResults"))
        .and(next_token_is("STUCK"))
        .respond_with(ResponseTemplate::new(200).set_body_json(result_page(
            &["n"],
            &[&["2"]],
            Some("STUCK"),
        )))
        .expect(1)
        .mount(&server)
        .await;

    let mut backend = athena_backend(&server);
    let mut stream = backend.open_branch("SELECT n FROM t", &[]).await.unwrap();
    assert_eq!(row(&mut stream).await, "1");
    assert_eq!(row(&mut stream).await, "2");
    let error = match stream.next_row().await {
        Err(error) => error.to_string(),
        Ok(_) => panic!("expected a repeated NextToken error"),
    };
    assert!(error.contains("cyclic NextToken"), "{error}");
    server.verify().await;
}

#[tokio::test]
async fn deadline_overrun_while_polling_issues_stop_query_execution() {
    let server = MockServer::start().await;
    mount_start(&server).await;
    // Never leaves RUNNING, so the total deadline is what ends the poll loop.
    Mock::given(method("POST"))
        .and(target("GetQueryExecution"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"QueryExecution": {"Status": {"State": "RUNNING"}}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(target("StopQueryExecution"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    let config = AthenaConfig::new("us-east-1")
        .unwrap()
        .with_endpoint(server.uri())
        .unwrap()
        .with_total_deadline(Duration::from_millis(50))
        .unwrap()
        .with_request_timeout(Duration::from_millis(50))
        .unwrap()
        .with_poll_interval(Duration::from_millis(1))
        .unwrap();
    let credentials = AthenaCredentials::new("AKIDEXAMPLE", SECRET_KEY)
        .unwrap()
        .with_session_token(SESSION_TOKEN)
        .unwrap();
    let mut backend = AthenaBackend::new(config, credentials).unwrap();

    let error = backend
        .open_branch("SELECT 1", &[])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("deadline exceeded"), "{error}");
    assert!(error.contains("StopQueryExecution"), "{error}");
    server.verify().await;
}

// ─── helpers ─────────────────────────────────────────────────────────────

async fn row(stream: &mut impl BranchStream) -> String {
    stream.next_row().await.unwrap().unwrap().values[0]
        .clone()
        .unwrap()
}

fn body_of(request: &Request) -> Value {
    serde_json::from_slice(&request.body).expect("request body is JSON")
}

fn is_target(request: &Request, operation: &str) -> bool {
    request
        .headers
        .get("x-amz-target")
        .and_then(|v| v.to_str().ok())
        == Some(format!("AmazonAthena.{operation}").as_str())
}

async fn start_request_body(server: &MockServer) -> Value {
    let requests = server.received_requests().await.unwrap();
    let start = requests
        .iter()
        .find(|r| is_target(r, "StartQueryExecution"))
        .expect("no StartQueryExecution request recorded");
    body_of(start)
}

async fn page_two_requests(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| is_target(r, "GetQueryResults") && body_of(r).get("NextToken").is_some())
        .count()
}

fn no_next_token() -> impl wiremock::Match {
    |request: &Request| body_of(request).get("NextToken").is_none()
}

fn next_token_is(expected: &'static str) -> impl wiremock::Match {
    move |request: &Request| {
        body_of(request).get("NextToken").and_then(Value::as_str) == Some(expected)
    }
}
