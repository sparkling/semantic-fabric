use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, HttpBody};
use axum::http::{header, Method, Request, Response, StatusCode};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError, QueryLimits};
use tower::ServiceExt;

use crate::budget::RequestBudget;
use crate::metrics::{self, MetricsEndpoint, ProductMetricsRecorder};
use crate::telemetry::{CorrelationId, GovernanceOutcome, RequestTrace};
use crate::{
    router, router_with_metrics, Backend, IntrospectedSource, SemanticOntology, ServeConfig,
};

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

fn capture(operation: impl FnOnce()) -> String {
    let recorder = PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full(metrics::QUERY_DURATION_SECONDS.to_owned()),
            metrics::QUERY_DURATION_BUCKETS,
        )
        .expect("fixed histogram buckets")
        .build_recorder();
    let handle = recorder.handle();
    let recorder = ProductMetricsRecorder::new(recorder);
    ::metrics::with_local_recorder(&recorder, operation);
    handle.render()
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
    config.set_query_admission(crate::QueryAdmission::UnrestrictedDevelopment);
    config
}

#[test]
fn query_terminal_metrics_are_exactly_once_for_complete_and_drop() {
    let output = capture(|| {
        metrics::describe_all();

        let request = Request::builder()
            .method(Method::POST)
            .uri("/sparql")
            .body(Body::empty())
            .unwrap();
        let trace = RequestTrace::new(&request, CorrelationId::generate());
        let response = trace.complete(Response::new(Body::empty()));
        assert!(response.body().is_end_stream());
        assert!(response.body().is_end_stream());
        drop(response);

        let request = Request::builder()
            .method(Method::POST)
            .uri("/sparql")
            .body(Body::empty())
            .unwrap();
        let trace = RequestTrace::new(&request, CorrelationId::generate());
        drop(
            trace.complete(Response::new(Body::from_stream(tokio_stream::pending::<
                Result<axum::body::Bytes, std::io::Error>,
            >()))),
        );
    });

    for body in ["complete", "dropped"] {
        let line = output
            .lines()
            .find(|line| {
                line.starts_with("sf_query_total{")
                    && line.contains("status=\"success\"")
                    && line.contains(&format!("body=\"{body}\""))
            })
            .unwrap_or_else(|| panic!("missing {body} series: {output}"));
        assert!(line.ends_with(" 1"), "line={line}");
    }
    assert!(
        output.contains("sf_query_duration_seconds_count{status=\"success\"} 2"),
        "output={output}"
    );
}

#[test]
fn discovery_and_control_responses_are_not_counted_as_queries() {
    let output = capture(|| {
        metrics::describe_all();
        for (method, uri) in [
            (Method::GET, "/sparql"),
            (Method::HEAD, "/sparql"),
            (Method::GET, "/livez"),
            (Method::GET, "/readyz"),
            (Method::GET, "/metrics"),
        ] {
            let request = Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap();
            let trace = RequestTrace::new(&request, CorrelationId::generate());
            let response = trace.complete(Response::new(Body::empty()));
            assert!(response.body().is_end_stream());
        }
    });

    assert!(!output.contains("sf_query_total{"), "output={output}");
    assert!(
        !output.contains("sf_query_duration_seconds_"),
        "output={output}"
    );
}

#[test]
fn governance_terminal_counter_is_closed_and_exactly_once() {
    let output = capture(|| {
        metrics::describe_all();
        let budget = RequestBudget::after_with_telemetry(
            Duration::from_secs(10),
            QueryLimits::new(u64::MAX, 0, u64::MAX, u64::MAX),
            CorrelationId::generate(),
        );
        assert_eq!(
            budget.consume(QueryCharge::SourceWork, 1),
            Err(QueryControlError::SourceWorkExceeded)
        );
        assert_eq!(
            budget.consume(QueryCharge::SourceWork, 1),
            Err(QueryControlError::SourceWorkExceeded)
        );
    });

    assert!(output.contains("sf_governance_rejections_total{reason=\"source_work_exceeded\"} 1"));
}

#[test]
fn recorder_rejects_foreign_names_targets_labels_and_values() {
    const SECRET: &str = "metrics-secret-NEVER-EXPOSE-65e9";
    let output = capture(|| {
        metrics::describe_all();
        ::metrics::counter!(
            target: "foreign_dependency",
            "sf_query_total",
            "status" => "success",
            "body" => "complete"
        )
        .increment(1);
        ::metrics::counter!(
            target: metrics::METRICS_TARGET,
            "foreign_metric_total",
            "secret" => SECRET
        )
        .increment(1);
        ::metrics::counter!(
            target: metrics::METRICS_TARGET,
            "sf_query_total",
            "status" => SECRET,
            "body" => "complete"
        )
        .increment(1);
        ::metrics::histogram!(
            target: metrics::METRICS_TARGET,
            metrics::QUERY_DURATION_SECONDS,
            "status" => "success",
            "query" => SECRET
        )
        .record(1.0);
    });

    assert!(!output.contains(SECRET), "output={output}");
    assert!(!output.contains("foreign_metric"), "output={output}");
    assert!(!output.contains("sf_query_total{"), "output={output}");
    assert!(
        !output.contains("sf_query_duration_seconds_"),
        "output={output}"
    );
}

#[test]
fn governance_metric_reasons_match_the_exhaustive_trace_vocabulary() {
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
        .map(GovernanceOutcome::as_str),
        metrics::GOVERNANCE_REASONS
    );
}

#[tokio::test]
async fn metrics_route_is_opt_in_fixed_and_outside_query_admission() {
    let absent = router(Arc::new(config()))
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(absent.status(), StatusCode::NOT_FOUND);

    let mut enabled_config = config();
    enabled_config.set_max_concurrent_requests(1).unwrap();
    let permits = enabled_config.request_admission_permits();
    let _held = permits
        .clone()
        .try_acquire_owned()
        .expect("saturate query-work admission");
    let endpoint = MetricsEndpoint::new(|| "# TYPE sf_query_total counter\n".to_owned());
    let response = router_with_metrics(Arc::new(enabled_config), endpoint)
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/plain; version=0.0.4; charset=utf-8"
    );
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(permits.available_permits(), 0);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "# TYPE sf_query_total counter\n"
    );
}
