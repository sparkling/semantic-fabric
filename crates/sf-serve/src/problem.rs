use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::Response;
use serde::Serialize;
use sf_core::query_control::QueryControlError::{
    self, CompilerEnvelopeExceeded, CompilerResourceExhausted,
};
use sf_sparql::Error as SparqlError;

static NEXT_CORRELATION_ID: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProblemCode {
    InvalidRequest,
    NotFound,
    MethodNotAllowed,
    NotAcceptable,
    UnsupportedMediaType,
    PayloadTooLarge,
    UnsupportedQuery,
    RequestTimeout,
    QueryBudgetExceeded,
    ServiceOverloaded,
    SourceUnavailable,
    Internal,
}
const CONTROL_PROBLEM_CODES: [(QueryControlError, ProblemCode); QueryControlError::VARIANT_COUNT] = [
    (
        QueryControlError::DeadlineExceeded,
        ProblemCode::RequestTimeout,
    ),
    (QueryControlError::Cancelled, ProblemCode::Internal),
    (CompilerEnvelopeExceeded, ProblemCode::QueryBudgetExceeded),
    (CompilerResourceExhausted, ProblemCode::ServiceOverloaded),
    (
        QueryControlError::CompilerWorkExceeded,
        ProblemCode::QueryBudgetExceeded,
    ),
    (
        QueryControlError::SourceWorkExceeded,
        ProblemCode::QueryBudgetExceeded,
    ),
    (
        QueryControlError::ResultItemsExceeded,
        ProblemCode::QueryBudgetExceeded,
    ),
    (
        QueryControlError::SerializedBytesExceeded,
        ProblemCode::QueryBudgetExceeded,
    ),
    (
        QueryControlError::RetainedBytesExceeded,
        ProblemCode::QueryBudgetExceeded,
    ),
    (QueryControlError::AccountingOverflow, ProblemCode::Internal),
];
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
        CONTROL_PROBLEM_CODES
            .iter()
            .find_map(|(candidate, code)| (*candidate == error).then_some(*code))
            .expect("QueryControlError mapping table must cover every variant")
    }

    fn value(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid-request",
            Self::NotFound => "not-found",
            Self::MethodNotAllowed => "method-not-allowed",
            Self::NotAcceptable => "not-acceptable",
            Self::UnsupportedMediaType => "unsupported-media-type",
            Self::PayloadTooLarge => "payload-too-large",
            Self::UnsupportedQuery => "unsupported-query",
            Self::RequestTimeout => "request-timeout",
            Self::QueryBudgetExceeded => "query-budget-exceeded",
            Self::ServiceOverloaded => "service-overloaded",
            Self::SourceUnavailable => "source-unavailable",
            Self::Internal => "internal-error",
        }
    }

    fn status(self) -> StatusCode {
        match self {
            Self::InvalidRequest => StatusCode::BAD_REQUEST,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            Self::NotAcceptable => StatusCode::NOT_ACCEPTABLE,
            Self::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnsupportedQuery => StatusCode::NOT_IMPLEMENTED,
            Self::RequestTimeout => StatusCode::GATEWAY_TIMEOUT,
            Self::QueryBudgetExceeded => StatusCode::TOO_MANY_REQUESTS,
            Self::ServiceOverloaded | Self::SourceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
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
            Self::NotFound => "The requested resource was not found.",
            Self::MethodNotAllowed => "The request method is not supported for this resource.",
            Self::NotAcceptable => "The requested response representation is not available.",
            Self::UnsupportedMediaType => "The request Content-Type is not supported.",
            Self::PayloadTooLarge => "The request body or query exceeds the configured byte limit.",
            Self::UnsupportedQuery => "The requested query or execution shape is not supported.",
            Self::RequestTimeout => "The request deadline expired.",
            Self::QueryBudgetExceeded => "The query exceeded a resource limit.",
            Self::ServiceOverloaded => "The service is temporarily overloaded.",
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

/// Build a temporary-unavailability response with the shared fixed retry hint.
pub(crate) fn response_with_retry_after(code: ProblemCode) -> Response {
    debug_assert_eq!(code.status(), StatusCode::SERVICE_UNAVAILABLE);
    let mut response = response(code);
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
    response
}

pub(crate) fn response_for_sparql(error: &SparqlError) -> Response {
    response(ProblemCode::from_sparql(error))
}

pub(crate) fn response_for_control(error: QueryControlError) -> Response {
    response(ProblemCode::from_control(error))
}

pub(crate) async fn not_found() -> Response {
    response(ProblemCode::NotFound)
}

pub(crate) async fn method_not_allowed() -> Response {
    response(ProblemCode::MethodNotAllowed)
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
#[path = "problem_tests.rs"]
mod tests;
