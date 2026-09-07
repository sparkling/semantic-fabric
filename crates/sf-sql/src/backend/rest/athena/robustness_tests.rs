use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use tokio::time::timeout;
use wiremock::matchers::{header, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::backend::BranchStream;

use super::client::AthenaSession;
use super::wire::{parse_result_page, ResultPage};
use super::{AthenaConfig, AthenaCredentials, AthenaStream};

fn credentials() -> AthenaCredentials {
    AthenaCredentials::new("AKIDEXAMPLE", "topsecret").unwrap()
}

#[tokio::test]
async fn cancelled_page_fetch_preserves_its_continuation_state() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("x-amz-target", "AmazonAthena.GetQueryResults"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(75))
                .set_body_json(json!({
                    "ResultSet": {
                        "Rows": [{"Data": [{"VarCharValue": "resumed"}]}]
                    }
                })),
        )
        .expect(2)
        .mount(&server)
        .await;

    let config = AthenaConfig::new("us-east-1")
        .unwrap()
        .with_endpoint(server.uri())
        .unwrap();
    let session = Arc::new(AthenaSession::new(config, credentials()).unwrap());
    let mut stream = AthenaStream::new(
        session,
        "q-1".to_owned(),
        ResultPage {
            columns: vec!["value".to_owned()],
            rows: Vec::new(),
            next_token: Some("PAGE-2".to_owned()),
        },
        Instant::now() + Duration::from_secs(2),
    );

    assert!(timeout(Duration::from_millis(10), stream.next_row())
        .await
        .is_err());
    let resumed = stream.next_row().await.unwrap().unwrap();
    assert_eq!(resumed.values, vec![Some("resumed".to_owned())]);
    assert!(stream.next_row().await.unwrap().is_none());
    server.verify().await;
}

#[test]
fn malformed_result_objects_are_rejected_instead_of_becoming_eof_or_null() {
    let established = vec!["value".to_owned()];
    for malformed in [
        json!({"ResultSet": null}),
        json!({"ResultSet": {"Rows": [17]}}),
        json!({"ResultSet": {"Rows": [null]}}),
        json!({"ResultSet": {"Rows": [{"Data": [17]}]}}),
        json!({"ResultSet": {"Rows": [{"Data": [null]}]}}),
        json!({"ResultSet": {"ResultSetMetadata": null}}),
    ] {
        assert!(
            parse_result_page(&malformed, Some(&established)).is_err(),
            "accepted malformed page: {malformed}"
        );
    }

    let null_cell = json!({"ResultSet": {"Rows": [{"Data": [{}]}]}});
    let page = parse_result_page(&null_cell, Some(&established)).unwrap();
    assert_eq!(page.rows[0].values, vec![None]);
}

#[tokio::test]
async fn chunked_response_body_is_bounded_without_content_length() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = socket.read(&mut request).unwrap();
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                  Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n\
                  28\r\nxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\r\n\
                  28\r\nyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy\r\n\
                  0\r\n\r\n",
            )
            .unwrap();
    });

    let config = AthenaConfig::new("us-east-1")
        .unwrap()
        .with_endpoint(format!("http://{address}"))
        .unwrap()
        .with_max_response_bytes(64)
        .unwrap()
        .with_max_retries(0)
        .unwrap();
    let session = AthenaSession::new(config, credentials()).unwrap();
    let error = session
        .call(
            "GetQueryResults",
            &json!({"QueryExecutionId": "q-1"}),
            Instant::now() + Duration::from_secs(1),
        )
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("64-byte limit"), "{error}");
    server.join().unwrap();
}
