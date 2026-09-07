//! Process-boundary Prometheus recorder installation (ADR-0011, partial M3).

use std::fmt;

use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusRecorder};
use sf_serve::{
    describe_metrics, MetricsEndpoint, ProductMetricsRecorder, QUERY_DURATION_BUCKETS,
    QUERY_DURATION_SECONDS,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct InitError;

impl fmt::Display for InitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("metrics initialization failed")
    }
}

impl std::error::Error for InitError {}

pub(super) fn init(enabled: bool) -> Result<Option<MetricsEndpoint>, InitError> {
    if !enabled {
        return Ok(None);
    }
    let recorder = build_recorder()?;
    let handle = recorder.handle();
    ::metrics::set_global_recorder(ProductMetricsRecorder::new(recorder)).map_err(|_| InitError)?;
    describe_metrics();
    Ok(Some(MetricsEndpoint::new(move || handle.render())))
}

fn build_recorder() -> Result<PrometheusRecorder, InitError> {
    PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full(QUERY_DURATION_SECONDS.to_owned()),
            QUERY_DURATION_BUCKETS,
        )
        .map(PrometheusBuilder::build_recorder)
        .map_err(|_| InitError)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_uses_fixed_prometheus_buckets() {
        let recorder = build_recorder().unwrap();
        let handle = recorder.handle();
        let recorder = ProductMetricsRecorder::new(recorder);
        ::metrics::with_local_recorder(&recorder, || {
            describe_metrics();
            ::metrics::histogram!(
                target: sf_serve::METRICS_TARGET,
                QUERY_DURATION_SECONDS,
                "status" => "success"
            )
            .record(0.5);
        });
        let rendered = handle.render();
        assert!(rendered.contains("sf_query_duration_seconds_bucket"));
        assert!(rendered.contains("le=\"0.5\""));
    }
}
