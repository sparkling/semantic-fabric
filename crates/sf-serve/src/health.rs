//! Fixed, redacted process-liveness and runtime-readiness responses.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::Response;

use crate::{RuntimeReadiness, ServeConfig};

const HEALTH_MEDIA_TYPE: &str = "application/json";
const LIVE_BODY: &str = r#"{"status":"live"}"#;
const READY_BODY: &str = r#"{"status":"ready"}"#;
const NOT_READY_BODY: &str = r#"{"status":"not-ready"}"#;
const INVALID_BODY: &str = r#"{"status":"invalid"}"#;

pub(crate) const fn is_health_path(path: &str) -> bool {
    matches!(path.as_bytes(), b"/livez" | b"/readyz")
}

/// Event-loop liveness only. This intentionally does not inspect runtime or source state.
pub(crate) async fn live() -> Response {
    response(StatusCode::OK, LIVE_BODY, false)
}

/// Runtime readiness only. Snapshot construction establishes structural validity;
/// this read never acquires or polls a source connection.
pub(crate) async fn ready(State(config): State<Arc<ServeConfig>>) -> Response {
    readiness_response(config.runtime_readiness())
}

fn readiness_response(readiness: Result<RuntimeReadiness, crate::ActivationError>) -> Response {
    match readiness {
        Ok(RuntimeReadiness::Ready { .. }) => response(StatusCode::OK, READY_BODY, false),
        Ok(RuntimeReadiness::NotReady { .. }) => {
            response(StatusCode::SERVICE_UNAVAILABLE, NOT_READY_BODY, true)
        }
        Err(_) => response(StatusCode::INTERNAL_SERVER_ERROR, INVALID_BODY, false),
    }
}

fn response(status: StatusCode, body: &'static str, retry: bool) -> Response {
    let mut response = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, HEALTH_MEDIA_TYPE)
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-content-type-options", "nosniff")
        .body(Body::from(body))
        .expect("static health response builder");
    if retry {
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ActivationError;

    #[tokio::test]
    async fn invalid_runtime_state_is_redacted_and_non_successful() {
        let response = readiness_response(Err(ActivationError::StatePoisoned));
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.headers()[header::CONTENT_TYPE], HEALTH_MEDIA_TYPE);
    }
}
