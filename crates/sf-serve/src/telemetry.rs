//! Closed, payload-free request telemetry vocabulary (ADR-0011, partial M3).

use std::future::Future;

use axum::body::Body;
use axum::http::{HeaderValue, Method, Request, Response, StatusCode};
use sf_core::query_control::QueryControlError;
use sf_core::TELEMETRY_TARGET;
use tracing::{Instrument, Span};

pub(crate) use crate::correlation::CorrelationId;

const SCHEMA: &str = "semantic-fabric.telemetry.v1";
#[derive(Clone, Copy)]
enum RequestMethod {
    Get,
    Head,
    Post,
    Other,
}

impl RequestMethod {
    fn classify(method: &Method) -> Self {
        match *method {
            Method::GET => Self::Get,
            Method::HEAD => Self::Head,
            Method::POST => Self::Post,
            _ => Self::Other,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Head => "head",
            Self::Post => "post",
            Self::Other => "other",
        }
    }

    fn body_disposition(self, status: StatusCode) -> crate::telemetry_body::BodyDisposition {
        if matches!(self, Self::Head)
            || status.is_informational()
            || matches!(status, StatusCode::NO_CONTENT | StatusCode::NOT_MODIFIED)
        {
            crate::telemetry_body::BodyDisposition::ProtocolNoBody
        } else {
            crate::telemetry_body::BodyDisposition::Expected
        }
    }
}

#[derive(Clone, Copy)]
enum RequestRoute {
    Sparql,
    Live,
    Ready,
    Other,
}

impl RequestRoute {
    fn classify(path: &str) -> Self {
        match path {
            "/sparql" => Self::Sparql,
            "/livez" => Self::Live,
            "/readyz" => Self::Ready,
            _ => Self::Other,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Sparql => "sparql",
            Self::Live => "livez",
            Self::Ready => "readyz",
            Self::Other => "other",
        }
    }
}

/// Coarse request stages. Labels are closed and carry no request or source text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Stage {
    RequestAdmission,
    Decode,
    GenerationLease,
    GenerationBuild,
    Compile,
    CompileWorker,
    ShapeAdmission,
    BindExecution,
    Execute,
    ExecuteTask,
    ExecuteStream,
    Serialize,
}

impl Stage {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::RequestAdmission => "request_admission",
            Self::Decode => "decode",
            Self::GenerationLease => "generation_lease",
            Self::GenerationBuild => "generation_build",
            Self::Compile => "compile",
            Self::CompileWorker => "compile_worker",
            Self::ShapeAdmission => "shape_admission",
            Self::BindExecution => "bind_execution",
            Self::Execute => "execute",
            Self::ExecuteTask => "execute_task",
            Self::ExecuteStream => "execute_stream",
            Self::Serialize => "serialize",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FailureKind {
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

impl FailureKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::NotFound => "not_found",
            Self::MethodNotAllowed => "method_not_allowed",
            Self::NotAcceptable => "not_acceptable",
            Self::UnsupportedMediaType => "unsupported_media_type",
            Self::PayloadTooLarge => "payload_too_large",
            Self::UnsupportedQuery => "unsupported_query",
            Self::RequestTimeout => "request_timeout",
            Self::QueryBudgetExceeded => "query_budget_exceeded",
            Self::ServiceOverloaded => "service_overloaded",
            Self::SourceUnavailable => "source_unavailable",
            Self::Internal => "internal",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GovernanceOutcome {
    DeadlineExceeded,
    Cancelled,
    CompilerEnvelopeExceeded,
    CompilerResourceExhausted,
    CompilerWorkExceeded,
    SourceWorkExceeded,
    ResultItemsExceeded,
    SerializedBytesExceeded,
    RetainedBytesExceeded,
    AccountingOverflow,
}

const GOVERNANCE_OUTCOMES: [(QueryControlError, GovernanceOutcome);
    QueryControlError::VARIANT_COUNT] = [
    (
        QueryControlError::DeadlineExceeded,
        GovernanceOutcome::DeadlineExceeded,
    ),
    (QueryControlError::Cancelled, GovernanceOutcome::Cancelled),
    (
        QueryControlError::CompilerEnvelopeExceeded,
        GovernanceOutcome::CompilerEnvelopeExceeded,
    ),
    (
        QueryControlError::CompilerResourceExhausted,
        GovernanceOutcome::CompilerResourceExhausted,
    ),
    (
        QueryControlError::CompilerWorkExceeded,
        GovernanceOutcome::CompilerWorkExceeded,
    ),
    (
        QueryControlError::SourceWorkExceeded,
        GovernanceOutcome::SourceWorkExceeded,
    ),
    (
        QueryControlError::ResultItemsExceeded,
        GovernanceOutcome::ResultItemsExceeded,
    ),
    (
        QueryControlError::SerializedBytesExceeded,
        GovernanceOutcome::SerializedBytesExceeded,
    ),
    (
        QueryControlError::RetainedBytesExceeded,
        GovernanceOutcome::RetainedBytesExceeded,
    ),
    (
        QueryControlError::AccountingOverflow,
        GovernanceOutcome::AccountingOverflow,
    ),
];

impl GovernanceOutcome {
    fn from_error(error: QueryControlError) -> Self {
        GOVERNANCE_OUTCOMES
            .iter()
            .find_map(|(candidate, outcome)| (*candidate == error).then_some(*outcome))
            .expect("QueryControlError telemetry mapping must cover every variant")
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Cancelled => "cancelled",
            Self::CompilerEnvelopeExceeded => "compiler_envelope_exceeded",
            Self::CompilerResourceExhausted => "compiler_resource_exhausted",
            Self::CompilerWorkExceeded => "compiler_work_exceeded",
            Self::SourceWorkExceeded => "source_work_exceeded",
            Self::ResultItemsExceeded => "result_items_exceeded",
            Self::SerializedBytesExceeded => "serialized_bytes_exceeded",
            Self::RetainedBytesExceeded => "retained_bytes_exceeded",
            Self::AccountingOverflow => "accounting_overflow",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StartupFailure {
    Configuration,
    Source,
    Runtime,
}

impl StartupFailure {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Configuration => "startup-configuration",
            Self::Source => "startup-source",
            Self::Runtime => "startup-runtime",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StreamOutcome {
    Complete,
    Failed,
    ClientGone,
}

impl StreamOutcome {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::ClientGone => "client_gone",
        }
    }
}

#[derive(Clone, Copy)]
enum RequestOutcome {
    Success,
    ClientError,
    ServerError,
}

impl RequestOutcome {
    fn from_status(status: StatusCode) -> Self {
        if status.is_server_error() {
            Self::ServerError
        } else if status.is_client_error() {
            Self::ClientError
        } else {
            Self::Success
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::ClientError => "client_error",
            Self::ServerError => "server_error",
        }
    }
}

/// Root request trace. Its generated identity and span remain owned by the body.
pub(crate) struct RequestTrace {
    span: Span,
    correlation: CorrelationId,
    method: RequestMethod,
}

impl RequestTrace {
    pub(crate) fn new<B>(request: &Request<B>, correlation: CorrelationId) -> Self {
        let method = RequestMethod::classify(request.method());
        let route = RequestRoute::classify(request.uri().path());
        let span = tracing::info_span!(
            target: TELEMETRY_TARGET,
            "sf.request",
            schema = SCHEMA,
            method = method.as_str(),
            route = route.as_str(),
            status = tracing::field::Empty,
            outcome = tracing::field::Empty,
            correlation_id = correlation.as_str(),
        );
        Self {
            span,
            correlation,
            method,
        }
    }

    pub(crate) fn span(&self) -> Span {
        self.span.clone()
    }

    pub(crate) fn in_scope<T>(&self, operation: impl FnOnce() -> T) -> T {
        self.span.in_scope(operation)
    }

    pub(crate) fn complete(self, mut response: Response<Body>) -> Response<Body> {
        let problem = crate::problem::finalize(&mut response, &self.correlation);
        response.headers_mut().insert(
            "x-correlation-id",
            HeaderValue::from_str(self.correlation.as_str())
                .expect("generated correlation is an ASCII header value"),
        );
        let status = response.status();
        let body_disposition = self.method.body_disposition(status);
        let outcome = RequestOutcome::from_status(status);
        self.span.record("status", status.as_u16());
        self.span.record("outcome", outcome.as_str());
        if let Some(failure) = problem {
            record_problem(failure, &self.correlation, status, &self.span);
        }
        tracing::info!(
            target: TELEMETRY_TARGET,
            parent: &self.span,
            schema = SCHEMA,
            event = "request.handoff",
            status = status.as_u16(),
            outcome = outcome.as_str(),
            correlation_id = self.correlation.as_str(),
        );

        let (parts, body) = response.into_parts();
        Response::from_parts(
            parts,
            crate::telemetry_body::wrap(body, self.span, self.correlation, body_disposition),
        )
    }
}

pub(crate) fn stage_span(stage: Stage) -> Span {
    tracing::info_span!(
        target: TELEMETRY_TARGET,
        "sf.request.stage",
        schema = SCHEMA,
        stage = stage.as_str(),
    )
}

pub(crate) fn in_stage_sync<T>(stage: Stage, operation: impl FnOnce() -> T) -> T {
    stage_span(stage).in_scope(operation)
}

pub(crate) async fn in_stage<F>(stage: Stage, future: F) -> F::Output
where
    F: Future,
{
    future.instrument(stage_span(stage)).await
}

pub(crate) async fn execute<F>(future: F) -> F::Output
where
    F: Future,
{
    in_stage(Stage::Execute, future).await
}

fn record_problem(
    failure: FailureKind,
    correlation: &CorrelationId,
    status: StatusCode,
    parent: &Span,
) {
    tracing::warn!(
        target: TELEMETRY_TARGET,
        parent: parent,
        schema = SCHEMA,
        event = "request.rejected",
        failure = failure.as_str(),
        status = status.as_u16(),
        correlation_id = correlation.as_str(),
    );
}

pub(crate) fn record_governance_terminal(error: QueryControlError, correlation: &CorrelationId) {
    let outcome = GovernanceOutcome::from_error(error);
    tracing::warn!(
        target: TELEMETRY_TARGET,
        schema = SCHEMA,
        event = "governance.terminal",
        outcome = outcome.as_str(),
        correlation_id = correlation.as_str(),
    );
}

pub(crate) fn record_startup_failure(failure: StartupFailure, correlation: &CorrelationId) {
    tracing::error!(
        target: TELEMETRY_TARGET,
        schema = SCHEMA,
        event = "startup.failed",
        failure = failure.as_str(),
        correlation_id = correlation.as_str(),
    );
}

pub(crate) fn record_stream_outcome(outcome: StreamOutcome, correlation: &CorrelationId) {
    match outcome {
        StreamOutcome::Complete | StreamOutcome::ClientGone => tracing::info!(
            target: TELEMETRY_TARGET,
            schema = SCHEMA,
            event = "stream.finished",
            outcome = outcome.as_str(),
            correlation_id = correlation.as_str(),
        ),
        StreamOutcome::Failed => tracing::warn!(
            target: TELEMETRY_TARGET,
            schema = SCHEMA,
            event = "stream.finished",
            outcome = outcome.as_str(),
            correlation_id = correlation.as_str(),
        ),
    }
}

#[cfg(test)]
pub(crate) fn is_generated_correlation(value: &str) -> bool {
    crate::correlation::is_generated(value)
}
