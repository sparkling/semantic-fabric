//! Process-boundary structured subscriber initialization (ADR-0011, partial M3).

use std::fmt;

use clap::ValueEnum;
use sf_core::TELEMETRY_TARGET;
use tracing::{Level, Metadata};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::{Context, Filter};
use tracing_subscriber::prelude::*;
use tracing_subscriber::util::SubscriberInitExt;

/// Closed operator-controlled ceiling for product telemetry. Arbitrary filter
/// directives are deliberately not accepted at the public CLI boundary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(super) enum TelemetryLevel {
    Off,
    Error,
    Warn,
    #[default]
    Info,
}

impl TelemetryLevel {
    const fn allows(self, level: &Level) -> bool {
        match self {
            Self::Off => false,
            Self::Error => matches!(*level, Level::ERROR),
            Self::Warn => matches!(*level, Level::ERROR | Level::WARN),
            Self::Info => matches!(*level, Level::ERROR | Level::WARN | Level::INFO),
        }
    }

    const fn max_level(self) -> LevelFilter {
        match self {
            Self::Off => LevelFilter::OFF,
            Self::Error => LevelFilter::ERROR,
            Self::Warn => LevelFilter::WARN,
            Self::Info => LevelFilter::INFO,
        }
    }
}

/// Deliberately opaque: subscriber-install errors may contain foreign text and
/// are not part of the public startup vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct InitError;

impl fmt::Display for InitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("structured telemetry initialization failed")
    }
}

impl std::error::Error for InitError {}

pub(super) fn init(level: TelemetryLevel) -> Result<(), InitError> {
    production_subscriber(level, std::io::stderr)
        .try_init()
        .map_err(redact_init_error)
}

fn production_subscriber<W>(
    level: TelemetryLevel,
    writer: W,
) -> impl tracing::Subscriber + Send + Sync
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    let formatter = tracing_subscriber::fmt::layer()
        .json()
        .flatten_event(true)
        .with_writer(writer)
        .with_ansi(false)
        .with_target(false)
        .with_file(false)
        .with_line_number(false)
        .with_thread_ids(false)
        .with_span_events(FmtSpan::CLOSE);
    tracing_subscriber::registry().with(formatter.with_filter(product_filter(level)))
}

#[derive(Clone, Copy)]
struct ProductFilter {
    level: TelemetryLevel,
    max_level: LevelFilter,
}

impl<S> Filter<S> for ProductFilter
where
    S: tracing::Subscriber,
{
    fn enabled(&self, metadata: &Metadata<'_>, context: &Context<'_, S>) -> bool {
        is_product_metadata(metadata, self.level)
            && <LevelFilter as Filter<S>>::enabled(&self.max_level, metadata, context)
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(self.max_level)
    }
}

fn product_filter(level: TelemetryLevel) -> ProductFilter {
    ProductFilter {
        level,
        max_level: level.max_level(),
    }
}

fn is_product_metadata(metadata: &Metadata<'_>, level: TelemetryLevel) -> bool {
    metadata.target() == TELEMETRY_TARGET && level.allows(metadata.level())
}

fn redact_init_error<T>(_error: T) -> InitError {
    InitError
}

#[cfg(test)]
#[path = "telemetry_runtime_tests.rs"]
mod runtime_tests;

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    use tracing::{Dispatch, Subscriber};
    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

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
    fn subscriber_install_failure_never_forwards_foreign_text() {
        let secret = "telemetry_secret_NEVER_EXPOSE_91f2";
        let error = redact_init_error(secret);
        assert_eq!(
            error.to_string(),
            "structured telemetry initialization failed"
        );
        assert!(!error.to_string().contains(secret));
        assert_eq!(format!("{error:?}"), "InitError");
    }

    #[test]
    fn subscriber_filter_applies_closed_levels_to_events_and_spans() {
        const SEEDED_CREDENTIAL: &str =
            "postgres://telemetry-user:seeded-password-NEVER-EXPOSE@db.invalid/product";
        for (level, expected) in [
            (TelemetryLevel::Off, [false, false, false]),
            (TelemetryLevel::Error, [true, false, false]),
            (TelemetryLevel::Warn, [true, true, false]),
            (TelemetryLevel::Info, [true, true, true]),
        ] {
            let capture = Capture::default();
            let formatter = tracing_subscriber::fmt::layer()
                .without_time()
                .with_ansi(false)
                .with_span_events(FmtSpan::NEW)
                .with_writer(capture.clone());
            let subscriber =
                tracing_subscriber::registry().with(formatter.with_filter(product_filter(level)));
            assert_eq!(subscriber.max_level_hint(), Some(level.max_level()));
            let dispatch = Dispatch::new(subscriber);
            tracing::dispatcher::with_default(&dispatch, || {
                tracing::error!(target: TELEMETRY_TARGET, event = "product.error.event");
                let _error = tracing::error_span!(
                    target: TELEMETRY_TARGET,
                    "product.error.span"
                )
                .entered();
                tracing::warn!(target: TELEMETRY_TARGET, event = "product.warn.event");
                let _warn =
                    tracing::warn_span!(target: TELEMETRY_TARGET, "product.warn.span").entered();
                tracing::info!(target: TELEMETRY_TARGET, event = "product.info.event");
                let _info =
                    tracing::info_span!(target: TELEMETRY_TARGET, "product.info.span").entered();
                tracing::debug!(target: TELEMETRY_TARGET, secret = "debug-secret");
                tracing::error!(target: "foreign_dependency", secret = SEEDED_CREDENTIAL);
                let _foreign = tracing::error_span!(
                    target: "sf_sql",
                    "foreign.span",
                    secret = SEEDED_CREDENTIAL
                )
                .entered();
            });
            let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
            for (name, allowed) in [
                ("product.error", expected[0]),
                ("product.warn", expected[1]),
                ("product.info", expected[2]),
            ] {
                assert_eq!(
                    output.contains(name),
                    allowed,
                    "level={level:?}, output={output}"
                );
            }
            assert!(!output.contains("debug-secret"), "output={output}");
            assert!(!output.contains(SEEDED_CREDENTIAL), "output={output}");
            assert!(!output.contains("foreign.span"), "output={output}");
        }
    }

    #[test]
    fn every_product_emitter_uses_the_single_shared_target() {
        for (name, source, expected_sites) in [
            (
                "sf-serve events and spans",
                include_str!("../../sf-serve/src/telemetry.rs"),
                8,
            ),
            (
                "sf-serve response body",
                include_str!("../../sf-serve/src/telemetry_body.rs"),
                1,
            ),
            (
                "sf-sparql compiler",
                include_str!("../../sf-sparql/src/compiler_telemetry.rs"),
                1,
            ),
        ] {
            assert_eq!(
                source.matches("target: TELEMETRY_TARGET").count(),
                expected_sites,
                "{name}"
            );
            assert!(!source.contains("target: \"semantic_fabric::telemetry\""));
        }
    }
}
