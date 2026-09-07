//! Cross-boundary acceptance evidence for the partial ADR-0011 tracing slice.

use std::future::Future;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Method, Request, Response, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use sf_core::query_control::QueryLimits;
use tower::Service;
use tracing::instrument::WithSubscriber;
use tracing::Dispatch;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::fmt::MakeWriter;

use crate::budget::RequestBudget;
use crate::telemetry::{CorrelationId, RequestTrace};
use crate::{router, terminal_body, Backend, ServeConfig};

const SECRET: &str = "telemetry_acceptance_secret_NEVER_EXPOSE_a831";

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

struct CaptureWriter(Capture);

impl Write for CaptureWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 .0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Capture {
    type Writer = CaptureWriter;

    fn make_writer(&'writer self) -> Self::Writer {
        CaptureWriter(self.clone())
    }
}

async fn capture<T>(future: impl Future<Output = T>) -> (T, String) {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_ansi(false)
        .with_target(false)
        .with_file(false)
        .with_line_number(false)
        .with_thread_ids(false)
        .with_span_events(FmtSpan::CLOSE)
        .with_max_level(tracing::Level::INFO)
        .with_writer(capture.clone())
        .finish();
    let value = future.with_subscriber(Dispatch::new(subscriber)).await;
    let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    (value, output)
}

fn lines(output: &str) -> Vec<Value> {
    output
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON telemetry line"))
        .collect()
}

#[tokio::test(start_paused = true)]
async fn outer_deadline_uses_one_untrusted_correlation_across_every_surface() {
    let ((identity, details), output) = capture(async {
        let mut config = ServeConfig::new_with_unverified_source(
            Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
            Vec::new(),
            crate::test_support::empty_ontology(),
            Vec::new(),
        )
        .unwrap();
        config.timeout = Duration::from_secs(5);
        let mut service = router(Arc::new(config));
        std::future::poll_fn(|cx| service.poll_ready(cx))
            .await
            .unwrap();
        let response = service.call(
            Request::builder()
                .uri("/sparql?query=ASK%20%7B%7D")
                .header(header::ACCEPT, "application/sparql-results+json")
                .header("x-correlation-id", SECRET)
                .body(Body::empty())
                .unwrap(),
        );
        tokio::time::advance(Duration::from_secs(5)).await;
        let response = response.await.unwrap();
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        let identity = response.headers()["x-correlation-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (identity, serde_json::from_slice::<Value>(&body).unwrap())
    })
    .await;

    assert_ne!(identity, SECRET);
    assert_eq!(details["correlationId"], identity);
    assert!(details["instance"].as_str().unwrap().ends_with(&identity));
    assert!(!output.contains(SECRET), "output={output}");
    let records = lines(&output);
    let governance: Vec<_> = records
        .iter()
        .filter(|line| line["event"] == "governance.terminal")
        .collect();
    assert_eq!(governance.len(), 1, "output={output}");
    assert_eq!(governance[0]["correlation_id"], identity);
    let rejection = records
        .iter()
        .find(|line| line["event"] == "request.rejected")
        .unwrap();
    assert_eq!(rejection["correlation_id"], identity);
    let root = records
        .iter()
        .find(|line| line["span"]["name"] == "sf.request")
        .unwrap();
    assert_eq!(root["span"]["correlation_id"], identity);
}

#[tokio::test]
async fn stream_terminals_include_identity_and_use_their_bounded_levels() {
    let ((complete, failed, gone), output) = capture(async {
        let budget = test_budget();
        let complete = budget.correlation_id().as_str().to_owned();
        let (body, task) = terminal_body::spawn(1, budget, |_tx, _budget| async { Ok(()) });
        body.collect().await.unwrap();
        task.await.unwrap();

        let budget = test_budget();
        let failed = budget.correlation_id().as_str().to_owned();
        let (body, task) = terminal_body::spawn(1, budget, |_tx, _budget| async {
            Err(io::Error::other(SECRET))
        });
        assert!(body.collect().await.is_err());
        task.await.unwrap();

        let budget = test_budget();
        let gone = budget.correlation_id().as_str().to_owned();
        let (body, task) = terminal_body::spawn(1, budget, |_tx, _budget| async {
            std::future::pending::<io::Result<()>>().await
        });
        drop(body);
        task.await.unwrap();
        (complete, failed, gone)
    })
    .await;

    assert!(!output.contains(SECRET), "output={output}");
    let records = lines(&output);
    let streams: Vec<_> = records
        .iter()
        .filter(|line| line["event"] == "stream.finished")
        .collect();
    assert_eq!(streams.len(), 3, "output={output}");
    for (identity, outcome, level) in [
        (complete, "complete", "INFO"),
        (failed, "failed", "WARN"),
        (gone, "client_gone", "INFO"),
    ] {
        let event = streams
            .iter()
            .find(|line| line["correlation_id"] == identity)
            .unwrap();
        assert_eq!(event["outcome"], outcome);
        assert_eq!(event["level"], level);
    }
}

#[tokio::test]
async fn protocol_no_body_responses_are_not_classified_as_dropped() {
    for (method, status) in [
        (Method::HEAD, StatusCode::OK),
        (Method::GET, StatusCode::NO_CONTENT),
        (Method::GET, StatusCode::NOT_MODIFIED),
    ] {
        let request_method = method.clone();
        let ((), output) = capture(async move {
            let request = Request::builder()
                .method(request_method)
                .uri("/sparql")
                .body(())
                .unwrap();
            let trace = RequestTrace::new(&request, CorrelationId::generate());
            let response = Response::builder()
                .status(status)
                .body(Body::from("framework-suppressed"))
                .unwrap();
            drop(trace.complete(response));
        })
        .await;
        assert_eq!(
            output
                .matches("\"event\":\"response.body.finished\"")
                .count(),
            1,
            "method={method}, status={status}, output={output}"
        );
        assert!(output.contains("\"outcome\":\"not_applicable\""));
        assert!(!output.contains("\"outcome\":\"dropped\""));
    }
}

fn test_budget() -> RequestBudget {
    RequestBudget::after(
        Duration::from_secs(60),
        QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
    )
}
