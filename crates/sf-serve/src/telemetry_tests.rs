use std::future::Future;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::http::{header, Method, Request, Response, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use sf_core::query_control::{QueryControl, QueryControlError, QueryLimits};
use tower::ServiceExt;
use tracing::instrument::WithSubscriber;
use tracing::Dispatch;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::fmt::MakeWriter;

use crate::budget::RequestBudget;
use crate::problem::{self, ProblemCode, ServeError, StartupCause};
use crate::telemetry::{
    self, CorrelationId, FailureKind, GovernanceOutcome, RequestTrace, Stage, StartupFailure,
    StreamOutcome,
};
use crate::{router, Backend, ServeConfig};

const SECRET: &str = "telemetry_secret_NEVER_EXPOSE_174b";
const QUERY: &str = "https%3A%2F%2Fsecret.example%2Fprivate";

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

struct CaptureWriter(Capture);

impl Write for CaptureWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
             .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend_from_slice(bytes);
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

impl Capture {
    fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
        )
        .expect("telemetry is UTF-8 JSON")
    }
}

fn subscriber(capture: Capture) -> impl tracing::Subscriber + Send + Sync {
    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_ansi(false)
        .with_target(false)
        .with_file(false)
        .with_line_number(false)
        .with_thread_ids(false)
        .with_span_events(FmtSpan::CLOSE)
        .with_max_level(tracing::Level::INFO)
        .with_writer(capture)
        .finish()
}

fn capture(operation: impl FnOnce()) -> String {
    let capture = Capture::default();
    let dispatch = Dispatch::new(subscriber(capture.clone()));
    tracing::dispatcher::with_default(&dispatch, operation);
    capture.text()
}

async fn capture_async<T>(future: impl Future<Output = T>) -> (T, String) {
    let capture = Capture::default();
    let dispatch = Dispatch::new(subscriber(capture.clone()));
    let value = future.with_subscriber(dispatch).await;
    (value, capture.text())
}

fn json_lines(output: &str) -> Vec<Value> {
    output
        .lines()
        .map(|line| serde_json::from_str(line).expect("each telemetry line is JSON"))
        .collect()
}

#[tokio::test]
async fn outer_identity_overrides_inbound_value_and_materializes_the_problem() {
    let ((correlation, body), output) = capture_async(async {
        let request = Request::builder()
            .method(Method::from_bytes(b"PRIVATE-METHOD").unwrap())
            .uri(format!("/{SECRET}?query={QUERY}"))
            .header("x-correlation-id", SECRET)
            .body(Body::empty())
            .unwrap();
        let correlation = CorrelationId::generate();
        let trace = RequestTrace::new(&request, correlation.clone());
        let response = trace.in_scope(|| {
            telemetry::in_stage_sync(Stage::Decode, || {
                problem::response(ProblemCode::InvalidRequest)
            })
        });
        assert!(response.headers().get("x-correlation-id").is_none());
        let response = trace.complete(response);
        assert_eq!(response.headers()["x-correlation-id"], correlation.as_str());
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (correlation, body)
    })
    .await;

    assert!(!output.contains(SECRET), "output={output}");
    assert!(!output.contains(QUERY), "output={output}");
    assert!(!output.contains("PRIVATE-METHOD"), "output={output}");
    let details: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(details["correlationId"], correlation.as_str());
    assert!(details["instance"]
        .as_str()
        .unwrap()
        .ends_with(correlation.as_str()));

    let lines = json_lines(&output);
    let rejected = lines
        .iter()
        .find(|line| line["event"] == "request.rejected")
        .expect("bounded rejection event");
    assert_eq!(rejected["failure"], "invalid_request");
    assert_eq!(rejected["correlation_id"], correlation.as_str());
    let handoff = lines
        .iter()
        .find(|line| line["event"] == "request.handoff")
        .expect("request handoff event");
    assert_eq!(handoff["correlation_id"], correlation.as_str());
    assert!(output.contains("\"method\":\"other\""));
    assert!(output.contains("\"route\":\"other\""));
}

#[tokio::test]
async fn early_outer_service_return_uses_one_identity_in_span_header_and_body() {
    let (identity, output) = capture_async(async {
        let config = ServeConfig::new_with_unverified_source(
            Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
            Vec::new(),
            crate::test_support::empty_ontology(),
            Vec::new(),
        )
        .unwrap();
        let response = router(Arc::new(config))
            .oneshot(
                Request::builder()
                    .uri("/sparql")
                    .header(header::ACCEPT, "application/json")
                    .header("x-correlation-id", SECRET)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
        let identity = response.headers()["x-correlation-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let details: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(details["correlationId"], identity);
        identity
    })
    .await;

    assert_ne!(identity, SECRET);
    assert!(!output.contains(SECRET), "output={output}");
    let root = json_lines(&output)
        .into_iter()
        .find(|line| line["span"]["name"] == "sf.request")
        .expect("root request span close event");
    assert_eq!(root["span"]["correlation_id"], identity);
    assert_eq!(root["span"]["method"], "get");
    assert_eq!(root["span"]["route"], "sparql");
}

#[tokio::test(start_paused = true)]
async fn deadline_replacement_records_only_the_winning_problem_and_terminal() {
    let ((), output) = capture_async(async {
        let correlation = CorrelationId::generate();
        let request = Request::builder()
            .uri("/sparql")
            .body(Body::empty())
            .unwrap();
        let trace = RequestTrace::new(&request, correlation.clone());
        let budget = RequestBudget::after_with_telemetry(
            Duration::from_secs(5),
            QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
            correlation,
        );
        let prepared = problem::response(ProblemCode::InvalidRequest);
        tokio::time::advance(Duration::from_secs(5)).await;
        let response = match budget.run_until_deadline(async move { prepared }).await {
            Ok(response) => response,
            Err(error) => problem::response_for_control(error),
        };
        let response = trace.complete(response);
        let details: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(details["code"], "request-timeout");
    })
    .await;

    assert_eq!(output.matches("\"event\":\"request.rejected\"").count(), 1);
    assert_eq!(
        output.matches("\"event\":\"governance.terminal\"").count(),
        1
    );
    assert!(output.contains("\"failure\":\"request_timeout\""));
    assert!(output.contains("\"outcome\":\"deadline_exceeded\""));
    assert!(!output.contains("\"failure\":\"invalid_request\""));
}

#[test]
fn sticky_terminal_emits_once_even_when_later_callers_repeat_or_replace_it() {
    let capture = Capture::default();
    let dispatch = Dispatch::new(subscriber(capture.clone()));
    tracing::dispatcher::with_default(&dispatch, || {
        let budget = RequestBudget::after_with_telemetry(
            Duration::from_secs(60),
            QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
            CorrelationId::generate(),
        );
        assert_eq!(
            budget.terminate(QueryControlError::SourceWorkExceeded),
            QueryControlError::SourceWorkExceeded
        );
        std::thread::scope(|scope| {
            for reason in [
                QueryControlError::Cancelled,
                QueryControlError::DeadlineExceeded,
                QueryControlError::AccountingOverflow,
            ] {
                let budget = budget.clone();
                let dispatch = dispatch.clone();
                scope.spawn(move || {
                    tracing::dispatcher::with_default(&dispatch, || {
                        assert_eq!(
                            budget.terminate(reason),
                            QueryControlError::SourceWorkExceeded
                        );
                    });
                });
            }
        });
    });
    let output = capture.text();
    assert_eq!(
        output.matches("\"event\":\"governance.terminal\"").count(),
        1
    );
    assert!(output.contains("\"outcome\":\"source_work_exceeded\""));
}

#[tokio::test]
async fn body_completion_error_and_drop_each_emit_one_redacted_terminal_event() {
    for (body, expected) in [
        (Body::from("complete"), "complete"),
        (
            Body::from_stream(tokio_stream::iter([Err::<Bytes, io::Error>(
                io::Error::other(SECRET),
            )])),
            "error",
        ),
    ] {
        let ((), output) = capture_async(async move {
            let request = Request::builder().uri("/sparql").body(()).unwrap();
            let trace = RequestTrace::new(&request, CorrelationId::generate());
            let response = trace.complete(Response::new(body));
            let _ = response.into_body().collect().await;
        })
        .await;
        assert_eq!(
            output
                .matches("\"event\":\"response.body.finished\"")
                .count(),
            1,
            "output={output}"
        );
        assert!(output.contains(&format!("\"outcome\":\"{expected}\"")));
        assert!(!output.contains(SECRET), "output={output}");
    }

    let ((), output) = capture_async(async {
        let request = Request::builder().uri("/sparql").body(()).unwrap();
        let trace = RequestTrace::new(&request, CorrelationId::generate());
        let body = Body::from_stream(tokio_stream::iter([Ok::<Bytes, io::Error>(
            Bytes::from_static(b"unpolled"),
        )]));
        drop(trace.complete(Response::new(body)));
    })
    .await;
    assert_eq!(
        output
            .matches("\"event\":\"response.body.finished\"")
            .count(),
        1,
        "output={output}"
    );
    assert!(output.contains("\"outcome\":\"dropped\""));
}

#[test]
fn startup_failure_event_cannot_reach_the_retained_internal_cause() {
    let error = ServeError::new(StartupCause::SourceSpec {
        spec: format!("mysql://user:{SECRET}@host/db"),
        error: format!("connector echoed {SECRET}"),
    });
    let correlation = error.correlation_id().to_owned();
    let output = capture(|| error.record_telemetry());

    assert!(!output.contains(SECRET), "output={output}");
    let lines = json_lines(&output);
    assert_eq!(lines.len(), 1, "output={output}");
    assert_eq!(lines[0]["event"], "startup.failed");
    assert_eq!(lines[0]["failure"], "startup-source");
    assert_eq!(lines[0]["correlation_id"], correlation);
}

#[test]
fn telemetry_vocabulary_is_closed_and_low_cardinality() {
    assert_eq!(
        [
            Stage::RequestAdmission,
            Stage::Decode,
            Stage::GenerationLease,
            Stage::GenerationBuild,
            Stage::Compile,
            Stage::CompileWorker,
            Stage::ShapeAdmission,
            Stage::BindExecution,
            Stage::Execute,
            Stage::ExecuteTask,
            Stage::ExecuteStream,
            Stage::Serialize,
        ]
        .map(Stage::as_str),
        [
            "request_admission",
            "decode",
            "generation_lease",
            "generation_build",
            "compile",
            "compile_worker",
            "shape_admission",
            "bind_execution",
            "execute",
            "execute_task",
            "execute_stream",
            "serialize",
        ]
    );
    assert_eq!(
        [
            FailureKind::InvalidRequest,
            FailureKind::NotFound,
            FailureKind::MethodNotAllowed,
            FailureKind::NotAcceptable,
            FailureKind::UnsupportedMediaType,
            FailureKind::PayloadTooLarge,
            FailureKind::UnsupportedQuery,
            FailureKind::RequestTimeout,
            FailureKind::QueryBudgetExceeded,
            FailureKind::ServiceOverloaded,
            FailureKind::SourceUnavailable,
            FailureKind::Internal,
        ]
        .map(FailureKind::as_str)
        .len(),
        12
    );
    assert_eq!(
        [
            GovernanceOutcome::DeadlineExceeded,
            GovernanceOutcome::Cancelled,
            GovernanceOutcome::CompilerEnvelopeExceeded,
            GovernanceOutcome::CompilerResourceExhausted,
            GovernanceOutcome::CompilerWorkExceeded,
            GovernanceOutcome::SourceWorkExceeded,
            GovernanceOutcome::ResultItemsExceeded,
            GovernanceOutcome::SerializedBytesExceeded,
            GovernanceOutcome::RetainedBytesExceeded,
            GovernanceOutcome::AccountingOverflow,
        ]
        .map(GovernanceOutcome::as_str)
        .len(),
        QueryControlError::VARIANT_COUNT
    );
    assert_eq!(
        [
            StartupFailure::Configuration,
            StartupFailure::Source,
            StartupFailure::Runtime,
        ]
        .map(StartupFailure::as_str),
        ["startup-configuration", "startup-source", "startup-runtime"]
    );
    assert_eq!(
        [
            StreamOutcome::Complete,
            StreamOutcome::Failed,
            StreamOutcome::ClientGone,
        ]
        .map(StreamOutcome::as_str),
        ["complete", "failed", "client_gone"]
    );
}

#[test]
fn only_the_exact_generated_correlation_shape_is_accepted() {
    let valid = CorrelationId::generate();
    assert!(telemetry::is_generated_correlation(valid.as_str()));
    for invalid in [
        "sf-000000000000000-0000000000000000",
        "sf-00000000000000000-0000000000000000",
        "sf-0000000000000000_0000000000000000",
        "sf-000000000000000g-0000000000000000",
        "sf-000000000000000A-0000000000000000",
        "external-correlation",
        SECRET,
    ] {
        assert!(!telemetry::is_generated_correlation(invalid), "{invalid:?}");
    }
}
