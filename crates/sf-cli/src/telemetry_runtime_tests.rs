//! Executable-boundary proof for the exact production telemetry subscriber.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use sf_core::query_control::QueryLimits;
use sf_serve::{router, Backend, IntrospectedSource, SemanticOntology, ServeConfig};
use tower::ServiceExt;
use tracing::instrument::WithSubscriber;
use tracing::Dispatch;
use tracing_subscriber::fmt::MakeWriter;

use super::{production_subscriber, TelemetryLevel};

const INBOUND_SECRET: &str = "inbound-correlation-NEVER-EXPOSE-71a9";
const FOREIGN_SECRET: &str =
    "postgres://foreign-user:foreign-password-NEVER-EXPOSE@db.invalid/private";
const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#Items> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "items" ] ;
  rr:subjectMap [ rr:template "http://example.test/item/{id}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:value ;
    rr:objectMap [ rr:column "value" ]
  ] .
"#;

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

#[test]
fn should_filter_an_actual_request_with_the_exact_production_policy_at_every_level() {
    for level in [
        TelemetryLevel::Off,
        TelemetryLevel::Error,
        TelemetryLevel::Warn,
        TelemetryLevel::Info,
    ] {
        let output = exercise(level);
        assert!(
            !output.contains(INBOUND_SECRET),
            "level={level:?}, {output}"
        );
        assert!(
            !output.contains(FOREIGN_SECRET),
            "level={level:?}, {output}"
        );
        let observed_levels: Vec<_> = output
            .lines()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).unwrap()["level"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert!(
            observed_levels.iter().all(|observed| match level {
                TelemetryLevel::Off => false,
                TelemetryLevel::Error => observed == "ERROR",
                TelemetryLevel::Warn => matches!(observed.as_str(), "ERROR" | "WARN"),
                TelemetryLevel::Info => matches!(observed.as_str(), "ERROR" | "WARN" | "INFO"),
            }),
            "level={level:?}, observed={observed_levels:?}"
        );
        assert_eq!(observed_levels.is_empty(), level == TelemetryLevel::Off);

        let error_visible = level != TelemetryLevel::Off;
        assert_eq!(
            output.contains("\"event\":\"startup.failed\""),
            error_visible,
            "level={level:?}, {output}"
        );

        let warnings_visible = matches!(level, TelemetryLevel::Warn | TelemetryLevel::Info);
        for event in ["governance.terminal", "request.rejected"] {
            assert_eq!(
                output.contains(&format!("\"event\":\"{event}\"")),
                warnings_visible,
                "event={event}, level={level:?}, {output}"
            );
        }

        let info_visible = level == TelemetryLevel::Info;
        for event in ["request.handoff", "response.body.finished"] {
            assert_eq!(
                output.contains(&format!("\"event\":\"{event}\"")),
                info_visible,
                "event={event}, level={level:?}, {output}"
            );
        }
        assert_eq!(
            output.contains("\"name\":\"sf.request\""),
            info_visible,
            "root span, level={level:?}, {output}"
        );
    }
}

fn exercise(level: TelemetryLevel) -> String {
    let capture = Capture::default();
    let subscriber = production_subscriber(level, capture.clone());
    let dispatch = Dispatch::new(subscriber);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(
            async {
                tracing::error!(
                    target: "hostile_foreign_dependency",
                    credential = FOREIGN_SECRET
                );

                let mut config = config();
                config
                    .set_max_query_len(usize::MAX)
                    .expect_err("unrepresentable public limit")
                    .record_telemetry();
                // This subscriber fixture exercises input-budget rejection,
                // before parser/source admission. Isolated parsing is exercised
                // by the real CLI integration tests, not a libtest executable.
                config.query_limits = QueryLimits::new(0, u64::MAX, u64::MAX, u64::MAX);
                let response = router(Arc::new(config))
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/sparql")
                            .header(header::CONTENT_TYPE, "application/sparql-query")
                            .header(header::ACCEPT, "application/sparql-results+json")
                            .header("x-correlation-id", INBOUND_SECRET)
                            .body(Body::from(
                                "ASK { ?item <http://example.test/value> ?value }",
                            ))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
                let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
                let problem: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(problem["code"], "query-budget-exceeded");
                assert_ne!(problem["correlationId"], INBOUND_SECRET);
            }
            .with_subscriber(dispatch),
        );
    let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    output
}

fn config() -> ServeConfig {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT NOT NULL); \
             INSERT INTO items VALUES (1, 'one');",
        )
        .unwrap();
    let source = IntrospectedSource::observe_sqlite(Backend::sqlite(connection)).unwrap();
    let ontology = SemanticOntology::from_turtle(
        "@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> . \
         <http://example.test/value> a rdf:Property .",
    )
    .unwrap();
    let mut config = ServeConfig::from_authored_r2rml(source, MAPPING, ontology).unwrap();
    config.set_query_admission(sf_serve::QueryAdmission::UnrestrictedDevelopment);
    config
}
