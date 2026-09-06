//! Process-boundary structured subscriber initialization (ADR-0011, partial M3).

use std::fmt;

use tracing::{Level, Metadata};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;
use tracing_subscriber::util::SubscriberInitExt;

const PRODUCT_TARGET: &str = "semantic_fabric::telemetry";

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

pub(super) fn init() -> Result<(), InitError> {
    let formatter = tracing_subscriber::fmt::layer()
        .json()
        .flatten_event(true)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_target(false)
        .with_file(false)
        .with_line_number(false)
        .with_thread_ids(false)
        .with_span_events(FmtSpan::CLOSE);
    tracing_subscriber::registry()
        .with(formatter.with_filter(tracing_subscriber::filter::filter_fn(is_product_metadata)))
        .try_init()
        .map_err(redact_init_error)
}

fn is_product_metadata(metadata: &Metadata<'_>) -> bool {
    metadata.target() == PRODUCT_TARGET
        && matches!(*metadata.level(), Level::ERROR | Level::WARN | Level::INFO)
}

fn redact_init_error<T>(_error: T) -> InitError {
    InitError
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    use tracing::Dispatch;
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
    fn subscriber_filter_rejects_foreign_targets_and_debug_events() {
        const SEEDED_CREDENTIAL: &str =
            "postgres://telemetry-user:seeded-password-NEVER-EXPOSE@db.invalid/product";
        let capture = Capture::default();
        let formatter = tracing_subscriber::fmt::layer()
            .without_time()
            .with_ansi(false)
            .with_writer(capture.clone());
        let subscriber = tracing_subscriber::registry().with(
            formatter.with_filter(tracing_subscriber::filter::filter_fn(is_product_metadata)),
        );
        let dispatch = Dispatch::new(subscriber);
        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!(target: PRODUCT_TARGET, event = "allowed");
            tracing::debug!(target: PRODUCT_TARGET, secret = "debug-secret");
            tracing::error!(target: "foreign_dependency", secret = SEEDED_CREDENTIAL);
        });
        let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(output.contains("allowed"), "output={output}");
        assert!(!output.contains("debug-secret"), "output={output}");
        assert!(!output.contains(SEEDED_CREDENTIAL), "output={output}");
    }
}
