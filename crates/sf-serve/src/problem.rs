//! Closed public-error vocabulary and the sole response redaction boundary.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::Response;
use serde::Serialize;
use sf_core::query_control::QueryControlError;
use sf_sparql::Error as SparqlError;

static NEXT_CORRELATION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProblemCode {
    InvalidRequest,
    UnsupportedMediaType,
    PayloadTooLarge,
    UnsupportedQuery,
    RequestTimeout,
    QueryBudgetExceeded,
    SourceUnavailable,
    Internal,
}

impl ProblemCode {
    fn from_sparql(error: &SparqlError) -> Self {
        match error {
            SparqlError::Parse(_) => Self::InvalidRequest,
            SparqlError::Unsupported(_) => Self::UnsupportedQuery,
            SparqlError::Mapping(_) | SparqlError::Sql(_) | SparqlError::Core(_) => Self::Internal,
            SparqlError::QueryControl(error) => Self::from_control(*error),
        }
    }

    fn from_control(error: QueryControlError) -> Self {
        match error {
            QueryControlError::DeadlineExceeded => Self::RequestTimeout,
            QueryControlError::SourceWorkExceeded
            | QueryControlError::ResultItemsExceeded
            | QueryControlError::SerializedBytesExceeded => Self::QueryBudgetExceeded,
            QueryControlError::Cancelled | QueryControlError::AccountingOverflow => Self::Internal,
            _ => Self::Internal,
        }
    }

    fn value(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid-request",
            Self::UnsupportedMediaType => "unsupported-media-type",
            Self::PayloadTooLarge => "payload-too-large",
            Self::UnsupportedQuery => "unsupported-query",
            Self::RequestTimeout => "request-timeout",
            Self::QueryBudgetExceeded => "query-budget-exceeded",
            Self::SourceUnavailable => "source-unavailable",
            Self::Internal => "internal-error",
        }
    }

    fn status(self) -> StatusCode {
        match self {
            Self::InvalidRequest => StatusCode::BAD_REQUEST,
            Self::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnsupportedQuery => StatusCode::NOT_IMPLEMENTED,
            Self::RequestTimeout => StatusCode::GATEWAY_TIMEOUT,
            Self::QueryBudgetExceeded => StatusCode::TOO_MANY_REQUESTS,
            Self::SourceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn title(self) -> &'static str {
        self.status()
            .canonical_reason()
            .unwrap_or("Internal Server Error")
    }

    fn detail(self) -> &'static str {
        match self {
            Self::InvalidRequest => "The request is invalid.",
            Self::UnsupportedMediaType => "The request Content-Type is not supported.",
            Self::PayloadTooLarge => "The query exceeds the configured byte limit.",
            Self::UnsupportedQuery => "The requested query or execution shape is not supported.",
            Self::RequestTimeout => "The request deadline expired.",
            Self::QueryBudgetExceeded => "The query exceeded a configured resource limit.",
            Self::SourceUnavailable => "The source is temporarily unavailable.",
            Self::Internal => "The request could not be completed.",
        }
    }
}

#[derive(Serialize)]
struct ProblemDetails {
    #[serde(rename = "type")]
    kind: String,
    title: &'static str,
    status: u16,
    detail: &'static str,
    instance: String,
    code: &'static str,
    #[serde(rename = "correlationId")]
    correlation_id: String,
}

impl ProblemDetails {
    fn new(code: ProblemCode) -> Self {
        let correlation_id = generated_correlation_id();
        Self {
            kind: format!("urn:semantic-fabric:problem:{}", code.value()),
            title: code.title(),
            status: code.status().as_u16(),
            detail: code.detail(),
            instance: format!("urn:semantic-fabric:problem-instance:{correlation_id}"),
            code: code.value(),
            correlation_id,
        }
    }
}

fn generated_correlation_id() -> String {
    let sequence = NEXT_CORRELATION_ID.fetch_add(1, Ordering::Relaxed);
    format!("sf-{:08x}-{sequence:016x}", std::process::id())
}

pub(crate) fn response(code: ProblemCode) -> Response {
    let details = ProblemDetails::new(code);
    let correlation_id = details.correlation_id.clone();
    let body = serde_json::to_vec(&details).expect("fixed problem details must serialize");
    Response::builder()
        .status(code.status())
        .header(header::CONTENT_TYPE, "application/problem+json")
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-content-type-options", "nosniff")
        .header("x-correlation-id", correlation_id)
        .body(Body::from(body))
        .expect("static problem response builder")
}

pub(crate) fn response_for_sparql(error: &SparqlError) -> Response {
    response(ProblemCode::from_sparql(error))
}

pub(crate) fn response_for_control(error: QueryControlError) -> Response {
    response(ProblemCode::from_control(error))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartupCode {
    Configuration,
    Source,
    Runtime,
}

impl StartupCode {
    fn value(self) -> &'static str {
        match self {
            Self::Configuration => "startup-configuration",
            Self::Source => "startup-source",
            Self::Runtime => "startup-runtime",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Self::Configuration => "startup configuration is invalid",
            Self::Source => "source initialization failed",
            Self::Runtime => "service startup failed",
        }
    }
}

#[derive(Debug)]
pub(crate) enum StartupCause {
    Configuration { error: String },
    Runtime { error: String },
    MappingRead { path: String, error: String },
    MappingParse { error: String },
    OntologyRead { path: String, error: String },
    OntologyParse { error: String },
    SourceSpec { spec: String, error: String },
    SourceConnect { spec: String, error: String },
    Schema { spec: String, error: String },
    Bind { bind: String, error: String },
    Server { error: String },
}

impl StartupCause {
    fn code(&self) -> StartupCode {
        match self {
            Self::Configuration { .. }
            | Self::MappingRead { .. }
            | Self::MappingParse { .. }
            | Self::OntologyRead { .. }
            | Self::OntologyParse { .. } => StartupCode::Configuration,
            Self::SourceSpec { .. } | Self::SourceConnect { .. } | Self::Schema { .. } => {
                StartupCode::Source
            }
            Self::Runtime { .. } | Self::Bind { .. } | Self::Server { .. } => StartupCode::Runtime,
        }
    }
}

impl fmt::Display for StartupCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration { error } => write!(formatter, "configuration: {error}"),
            Self::Runtime { error } => write!(formatter, "runtime: {error}"),
            Self::MappingRead { path, error } => {
                write!(formatter, "mapping read {path:?}: {error}")
            }
            Self::MappingParse { error } => write!(formatter, "mapping parse: {error}"),
            Self::OntologyRead { path, error } => {
                write!(formatter, "ontology read {path:?}: {error}")
            }
            Self::OntologyParse { error } => write!(formatter, "ontology parse: {error}"),
            Self::SourceSpec { spec, error } => {
                write!(formatter, "source specification {spec:?}: {error}")
            }
            Self::SourceConnect { spec, error } => {
                write!(formatter, "source connection {spec:?}: {error}")
            }
            Self::Schema { spec, error } => {
                write!(formatter, "source schema {spec:?}: {error}")
            }
            Self::Bind { bind, error } => write!(formatter, "bind {bind:?}: {error}"),
            Self::Server { error } => write!(formatter, "server: {error}"),
        }
    }
}

/// Opaque startup error. Its public format is redacted; the typed cause remains
/// inside `sf-serve` for a future internal tracing sink.
pub struct ServeError {
    code: StartupCode,
    correlation_id: String,
    cause: StartupCause,
}

impl ServeError {
    pub(crate) fn new(cause: StartupCause) -> Self {
        Self {
            code: cause.code(),
            correlation_id: generated_correlation_id(),
            cause,
        }
    }

    /// Stable, non-sensitive classification suitable for a CLI error surface.
    pub fn code(&self) -> &'static str {
        self.code.value()
    }

    /// Bounded generated identifier suitable for support correlation.
    pub fn correlation_id(&self) -> &str {
        &self.correlation_id
    }

    #[allow(dead_code)]
    pub(crate) fn internal_cause(&self) -> &StartupCause {
        &self.cause
    }
}

impl fmt::Display for ServeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: {} (correlation {})",
            self.code.value(),
            self.code.detail(),
            self.correlation_id
        )
    }
}

impl fmt::Debug for ServeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServeError")
            .field("code", &self.code.value())
            .field("correlation_id", &self.correlation_id)
            .field("cause", &"<redacted>")
            .finish()
    }
}

impl std::error::Error for ServeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_are_bounded_ascii_and_distinct() {
        let first = generated_correlation_id();
        let second = generated_correlation_id();
        assert_ne!(first, second);
        for id in [first, second] {
            assert!(id.len() <= 32, "id={id:?}");
            assert!(id.starts_with("sf-"));
            assert!(id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
        }
    }

    #[test]
    fn sparql_mapping_borrows_and_does_not_discard_the_typed_cause() {
        let sentinel = "typed_secret_cause";
        let error = SparqlError::Sql(sentinel.to_owned());
        assert_eq!(ProblemCode::from_sparql(&error), ProblemCode::Internal);
        assert!(error.to_string().contains(sentinel));
    }

    #[test]
    fn every_problem_code_has_one_stable_status_and_public_value() {
        let cases = [
            (
                ProblemCode::InvalidRequest,
                StatusCode::BAD_REQUEST,
                "invalid-request",
            ),
            (
                ProblemCode::UnsupportedMediaType,
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported-media-type",
            ),
            (
                ProblemCode::PayloadTooLarge,
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload-too-large",
            ),
            (
                ProblemCode::UnsupportedQuery,
                StatusCode::NOT_IMPLEMENTED,
                "unsupported-query",
            ),
            (
                ProblemCode::RequestTimeout,
                StatusCode::GATEWAY_TIMEOUT,
                "request-timeout",
            ),
            (
                ProblemCode::QueryBudgetExceeded,
                StatusCode::TOO_MANY_REQUESTS,
                "query-budget-exceeded",
            ),
            (
                ProblemCode::SourceUnavailable,
                StatusCode::SERVICE_UNAVAILABLE,
                "source-unavailable",
            ),
            (
                ProblemCode::Internal,
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal-error",
            ),
        ];

        for (code, status, value) in cases {
            assert_eq!(code.status(), status);
            assert_eq!(code.value(), value);
            let details = ProblemDetails::new(code);
            assert_eq!(details.status, status.as_u16());
            assert_eq!(details.code, value);
            assert!(!details.title.is_empty());
            assert!(!details.detail.is_empty());
        }
    }

    #[test]
    fn startup_public_formats_redact_the_retained_typed_cause() {
        let sentinel = "startup_secret_cause";
        let error = ServeError::new(StartupCause::SourceSpec {
            spec: format!("mysql://user:{sentinel}@host/db"),
            error: "invalid URL".to_owned(),
        });
        assert!(error.internal_cause().to_string().contains(sentinel));
        assert_eq!(error.code(), "startup-source");
        assert!(!error.to_string().contains(sentinel));
        assert!(!format!("{error:?}").contains(sentinel));
    }

    #[test]
    fn public_http_call_sites_cannot_accept_raw_error_strings() {
        let http_source = include_str!("http.rs");
        assert!(!http_source.contains("err_text("));
        assert!(!http_source.contains("response_for_status("));
        assert!(!http_source.contains("Body::from("));
        assert_eq!(
            http_source.matches("Response::builder()").count(),
            1,
            "only the success response builder belongs in http.rs"
        );
    }
}
