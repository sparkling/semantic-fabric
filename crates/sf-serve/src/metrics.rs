//! Closed, payload-free metric vocabulary for the partial ADR-0011 control plane.

use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::http::{header, Response, StatusCode};
use metrics::{
    describe_counter, describe_histogram, Counter, Gauge, Histogram, Key, KeyName, Metadata,
    Recorder, SharedString, Unit,
};

pub const QUERY_DURATION_SECONDS: &str = "sf_query_duration_seconds";
pub const QUERY_DURATION_BUCKETS: &[f64] = &[
    0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
    120.0, 300.0,
];

pub const METRICS_TARGET: &str = "semantic_fabric::metrics";
const QUERY_TOTAL: &str = "sf_query_total";
const GOVERNANCE_REJECTIONS_TOTAL: &str = "sf_governance_rejections_total";
const PROMETHEUS_MEDIA_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

const STATUSES: &[&str] = &["success", "client_error", "server_error"];
const BODY_OUTCOMES: &[&str] = &["complete", "error", "dropped", "not_applicable"];
pub(crate) const GOVERNANCE_REASONS: &[&str] = &[
    "deadline_exceeded",
    "cancelled",
    "compiler_envelope_exceeded",
    "compiler_resource_exhausted",
    "compiler_work_exceeded",
    "source_work_exceeded",
    "result_items_exceeded",
    "serialized_bytes_exceeded",
    "retained_bytes_exceeded",
    "accounting_overflow",
];

/// Register the exact metric names and help text after installing a recorder.
pub fn describe_all() {
    describe_counter!(
        QUERY_TOTAL,
        Unit::Count,
        "Terminal query attempts by closed HTTP status and body outcome"
    );
    describe_histogram!(
        QUERY_DURATION_SECONDS,
        Unit::Seconds,
        "Query ingress-to-body-terminal duration in seconds"
    );
    describe_counter!(
        GOVERNANCE_REJECTIONS_TOTAL,
        Unit::Count,
        "Sticky request-governance terminal decisions by closed reason"
    );
}

/// Body-lifetime observation prepared at response handoff.
#[derive(Clone, Copy)]
pub(crate) struct QueryTerminal {
    started: Instant,
    status: &'static str,
}

impl QueryTerminal {
    pub(crate) const fn new(started: Instant, status: &'static str) -> Self {
        Self { started, status }
    }

    pub(crate) fn record(self, body: &'static str) {
        metrics::counter!(target: METRICS_TARGET, QUERY_TOTAL, "status" => self.status, "body" => body)
            .increment(1);
        metrics::histogram!(target: METRICS_TARGET, QUERY_DURATION_SECONDS, "status" => self.status)
            .record(self.started.elapsed().as_secs_f64());
    }
}

pub(crate) fn record_governance_rejection(reason: &'static str) {
    metrics::counter!(target: METRICS_TARGET, GOVERNANCE_REJECTIONS_TOTAL, "reason" => reason)
        .increment(1);
}

/// Fail-closed recorder that admits only this module's fixed metric schema.
pub struct ProductMetricsRecorder<R> {
    inner: R,
}

impl<R> ProductMetricsRecorder<R> {
    pub const fn new(inner: R) -> Self {
        Self { inner }
    }
}

impl<R: Recorder> Recorder for ProductMetricsRecorder<R> {
    fn describe_counter(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        if matches!(key.as_str(), QUERY_TOTAL | GOVERNANCE_REJECTIONS_TOTAL) {
            self.inner.describe_counter(key, unit, description);
        }
    }

    fn describe_gauge(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}

    fn describe_histogram(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        if key.as_str() == QUERY_DURATION_SECONDS {
            self.inner.describe_histogram(key, unit, description);
        }
    }

    fn register_counter(&self, key: &Key, metadata: &Metadata<'_>) -> Counter {
        if metadata.target() == METRICS_TARGET && valid_counter_key(key) {
            self.inner.register_counter(key, metadata)
        } else {
            Counter::noop()
        }
    }

    fn register_gauge(&self, _key: &Key, _metadata: &Metadata<'_>) -> Gauge {
        Gauge::noop()
    }

    fn register_histogram(&self, key: &Key, metadata: &Metadata<'_>) -> Histogram {
        if metadata.target() == METRICS_TARGET
            && key.name() == QUERY_DURATION_SECONDS
            && labels_match(key, &[("status", STATUSES)])
        {
            self.inner.register_histogram(key, metadata)
        } else {
            Histogram::noop()
        }
    }
}

fn valid_counter_key(key: &Key) -> bool {
    match key.name() {
        QUERY_TOTAL => labels_match(key, &[("status", STATUSES), ("body", BODY_OUTCOMES)]),
        GOVERNANCE_REJECTIONS_TOTAL => labels_match(key, &[("reason", GOVERNANCE_REASONS)]),
        _ => false,
    }
}

fn labels_match(key: &Key, expected: &[(&str, &[&str])]) -> bool {
    if key.labels().count() != expected.len() {
        return false;
    }
    expected.iter().all(|(expected_key, allowed)| {
        let mut matches = key.labels().filter(|label| label.key() == *expected_key);
        matches
            .next()
            .is_some_and(|label| allowed.contains(&label.value()))
            && matches.next().is_none()
    })
}

/// Cloneable renderer for an explicitly enabled Prometheus control endpoint.
#[derive(Clone)]
pub struct MetricsEndpoint {
    render: Arc<dyn Fn() -> String + Send + Sync>,
}

impl MetricsEndpoint {
    pub fn new(render: impl Fn() -> String + Send + Sync + 'static) -> Self {
        Self {
            render: Arc::new(render),
        }
    }

    pub(crate) fn response(&self) -> Response<Body> {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, PROMETHEUS_MEDIA_TYPE)
            .header(header::CACHE_CONTROL, "no-store")
            .header("x-content-type-options", "nosniff")
            .body(Body::from((self.render)()))
            .expect("fixed metrics response is valid")
    }
}

pub(crate) fn is_path(path: &str) -> bool {
    path == "/metrics"
}
