use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use sf_core::TELEMETRY_TARGET;
use tracing::Dispatch;
use tracing_subscriber::fmt::MakeWriter;

use super::{record, AccessDecision};

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

#[test]
fn adopted_access_decision_labels_are_stable() {
    assert_eq!(AccessDecision::VARIANT_COUNT, 3);
    assert_eq!(
        AccessDecision::VARIANTS.map(AccessDecision::as_str),
        ["allow", "deny", "mask"]
    );
}

#[test]
fn real_request_admission_emits_one_redacted_decision_per_attempt() {
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;
    const TOKEN: &str = "test-only-audit-principal-0123456789";
    let capture = Capture::default();
    let dispatch = Dispatch::new(
        tracing_subscriber::fmt()
            .json()
            .flatten_event(true)
            .with_writer(capture.clone())
            .finish(),
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    tracing::dispatcher::with_default(&dispatch, || {
        runtime.block_on(async {
            let mut config = crate::ServeConfig::new_with_unverified_source(
                crate::Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
                vec![],
                crate::test_support::empty_ontology(),
                vec![],
            )
            .unwrap();
            config.set_query_admission(crate::QueryAdmission::Bearer(
                crate::BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
            ));
            let app = crate::router(Arc::new(config));
            for token in ["wrong", TOKEN] {
                let response = app
                    .clone()
                    .oneshot(
                        Request::post("/sparql")
                            .header(header::AUTHORIZATION, format!("Bearer {token}"))
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                drop(response);
            }
        })
    });
    let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(!output.contains(TOKEN) && !output.contains("Bearer") && !output.contains("wrong"));
    let decisions: Vec<Value> = output
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event["event"] == "security.access_decision")
        .map(|event| event["decision"].clone())
        .collect();
    assert_eq!(decisions, vec![Value::from("deny"), Value::from("allow")]);
}

#[test]
fn access_decisions_emit_only_closed_payload_free_fields_on_the_m3_target() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_ansi(false)
        .with_target(true)
        .with_file(false)
        .with_line_number(false)
        .with_thread_ids(false)
        .with_writer(capture.clone())
        .finish();
    let dispatch = Dispatch::new(subscriber);

    tracing::dispatcher::with_default(&dispatch, || {
        record(AccessDecision::Allow);
        record(AccessDecision::Deny);
        record(AccessDecision::Mask);
    });

    let output = String::from_utf8(
        capture
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone(),
    )
    .unwrap();
    let lines: Vec<Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();

    assert_eq!(lines.len(), 3, "output={output}");
    for (line, decision) in lines.iter().zip(["allow", "deny", "mask"]) {
        let mut keys: Vec<&str> = line
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "decision",
                "event",
                "level",
                "schema",
                "target",
                "timestamp"
            ]
        );
        assert_eq!(line["target"], TELEMETRY_TARGET);
        assert_eq!(line["event"], "security.access_decision");
        assert_eq!(line["schema"], "semantic-fabric.telemetry.v1");
        assert_eq!(line["decision"], decision);
    }
}
