//! The signed AWS JSON 1.1 transport: one POST per operation, bounded retries,
//! and AWS error-shape classification.
//!
//! Every operation is a `POST` to the SAME fixed endpoint with
//! `Content-Type: application/x-amz-json-1.1` and
//! `X-Amz-Target: AmazonAthena.{Operation}`; the operation never appears in the
//! URL. Retries re-sign a byte-identical body with a fresh timestamp.

use std::time::{Instant, SystemTime};

use reqwest::{Client, RequestBuilder, StatusCode};
use serde_json::Value;

use crate::error::{Error, Result};

use super::config::{AthenaConfig, CONTENT_TYPE, TARGET_PREFIX};
use super::credentials::AthenaCredentials;
use super::sign::sign_request;

/// AWS error shapes that mean "slow down" rather than "you are wrong". Athena
/// returns throttling as HTTP **400**, so status alone cannot classify it.
const RETRYABLE_SHAPES: [&str; 2] = ["TooManyRequestsException", "ThrottlingException"];
/// Cap on how much of an unparseable error body is quoted back in diagnostics.
const MAX_BODY_EXCERPT: usize = 512;

/// One attempt's failure plus whether retrying it could help.
struct Attempt {
    retryable: bool,
    error: Error,
}

/// Shared HTTP client + config + credentials for one Athena backend.
pub(crate) struct AthenaSession {
    http: Client,
    url: String,
    pub(crate) config: AthenaConfig,
    pub(crate) credentials: AthenaCredentials,
}

impl AthenaSession {
    pub(crate) fn new(config: AthenaConfig, credentials: AthenaCredentials) -> Result<Self> {
        config.validate()?;
        let http = Client::builder()
            // A SigV4 signature is bound to the configured host. Following a
            // redirect cannot authenticate successfully and could forward the
            // STS session token and SQL body to an untrusted origin.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| Error::Marshal(format!("athena: HTTP client init failed: {e}")))?;
        Ok(Self {
            url: config.request_url(),
            http,
            config,
            credentials,
        })
    }

    /// Build a redacted error. EVERY diagnostic this backend surfaces goes
    /// through here, so no credential component can reach a log line.
    pub(crate) fn err(&self, msg: impl Into<String>) -> Error {
        Error::Marshal(self.credentials.redact(&msg.into()))
    }

    /// Invoke one Athena operation, retrying transport failures, 5xx, and
    /// throttling up to `max_retries` times or until `deadline`.
    pub(crate) async fn call(
        &self,
        operation: &str,
        body: &Value,
        deadline: Instant,
    ) -> Result<Value> {
        let payload = serde_json::to_vec(body)
            .map_err(|e| self.err(format!("athena {operation}: request encode failed: {e}")))?;
        let target = format!("{TARGET_PREFIX}{operation}");

        let mut attempt = 0u32;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(self.err(format!(
                    "athena {operation}: total deadline exceeded before the request was sent"
                )));
            }
            // Fresh signing time per attempt over the byte-identical payload:
            // a SigV4 signature is scoped to its X-Amz-Date.
            let signed = sign_request(
                &self.config,
                &self.credentials,
                &target,
                &payload,
                SystemTime::now(),
            )?;
            let mut request = self
                .http
                .post(&self.url)
                .timeout(self.config.request_timeout.min(remaining))
                .header("content-type", CONTENT_TYPE)
                .header("x-amz-target", &target)
                .body(payload.clone());
            for (name, value) in &signed.headers {
                request = request.header(name.as_str(), value.as_str());
            }

            match self.send(request, operation).await {
                Ok(value) => return Ok(value),
                Err(Attempt { retryable, error }) => {
                    if !retryable || attempt >= self.config.max_retries {
                        return Err(error);
                    }
                    let backoff = self.config.retry_backoff * 2u32.saturating_pow(attempt);
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(error);
                    }
                    tokio::time::sleep(backoff.min(remaining)).await;
                    attempt += 1;
                }
            }
        }
    }

    async fn send(
        &self,
        request: RequestBuilder,
        operation: &str,
    ) -> std::result::Result<Value, Attempt> {
        let response = request.send().await.map_err(|e| Attempt {
            retryable: true,
            error: self.err(format!("athena {operation}: transport failure: {e}")),
        })?;
        let status = response.status();
        let error_type = response
            .headers()
            .get("x-amzn-errortype")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let text = response.text().await.map_err(|e| Attempt {
            retryable: true,
            error: self.err(format!(
                "athena {operation}: response body read failed: {e}"
            )),
        })?;

        if status.is_success() {
            if text.trim().is_empty() {
                // StopQueryExecution and friends legitimately return no body.
                return Ok(Value::Null);
            }
            return serde_json::from_str(&text).map_err(|e| Attempt {
                retryable: false,
                error: self.err(format!("athena {operation}: malformed JSON response: {e}")),
            });
        }
        Err(self.classify(operation, status, error_type.as_deref(), &text))
    }

    fn classify(
        &self,
        operation: &str,
        status: StatusCode,
        error_type: Option<&str>,
        body: &str,
    ) -> Attempt {
        let parsed: Option<Value> = serde_json::from_str(body).ok();
        let shape = error_type
            .map(shape_name)
            .or_else(|| {
                parsed
                    .as_ref()
                    .and_then(|v| v.get("__type").or_else(|| v.get("code")))
                    .and_then(Value::as_str)
                    .map(shape_name)
            })
            .unwrap_or_else(|| "UnknownError".to_owned());
        let message = parsed
            .as_ref()
            .and_then(|v| {
                v.get("message")
                    .or_else(|| v.get("Message"))
                    .or_else(|| v.get("Reason"))
            })
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| excerpt(body));

        // Throttling arrives as HTTP 400, so the shape name — not the status —
        // decides. Auth and validation 4xx are never retried.
        let retryable = status.is_server_error()
            || RETRYABLE_SHAPES
                .iter()
                .any(|s| s.eq_ignore_ascii_case(&shape));
        Attempt {
            retryable,
            error: self.err(format!(
                "athena {operation}: HTTP {} {shape}: {message}",
                status.as_u16()
            )),
        }
    }
}

/// Reduce `com.amazonaws.athena#TooManyRequestsException:` or
/// `TooManyRequestsException:https://...` to the bare shape name.
fn shape_name(raw: &str) -> String {
    let head = raw.split(':').next().unwrap_or(raw);
    let head = head.rsplit(['#', '/']).next().unwrap_or(head);
    head.trim().to_owned()
}

fn excerpt(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "no response body".to_owned();
    }
    match trimmed.char_indices().nth(MAX_BODY_EXCERPT) {
        Some((cut, _)) => format!("{}…", &trimmed[..cut]),
        None => trimmed.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use reqwest::StatusCode;
    use serde_json::json;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::super::config::AthenaConfig;
    use super::super::credentials::AthenaCredentials;
    use super::{shape_name, AthenaSession};

    fn session() -> AthenaSession {
        AthenaSession::new(
            AthenaConfig::new("us-east-1").unwrap(),
            AthenaCredentials::new("AKIDEXAMPLE", "topsecret")
                .unwrap()
                .with_session_token("sessiontoken")
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn shape_names_are_extracted_from_every_aws_spelling() {
        assert_eq!(
            shape_name("com.amazonaws.athena#TooManyRequestsException"),
            "TooManyRequestsException"
        );
        assert_eq!(
            shape_name("TooManyRequestsException:https://docs.example/err"),
            "TooManyRequestsException"
        );
        assert_eq!(
            shape_name("InvalidRequestException"),
            "InvalidRequestException"
        );
    }

    #[test]
    fn throttling_is_retryable_at_http_400_but_validation_is_not() {
        let session = session();
        let throttled = session.classify(
            "StartQueryExecution",
            StatusCode::BAD_REQUEST,
            Some("TooManyRequestsException:"),
            r#"{"__type":"TooManyRequestsException","message":"Rate exceeded"}"#,
        );
        assert!(throttled.retryable);
        assert!(throttled.error.to_string().contains("Rate exceeded"));

        let invalid = session.classify(
            "StartQueryExecution",
            StatusCode::BAD_REQUEST,
            Some("InvalidRequestException:"),
            r#"{"__type":"InvalidRequestException","message":"line 1:8: mismatched input"}"#,
        );
        assert!(!invalid.retryable);

        let unauthorized = session.classify(
            "StartQueryExecution",
            StatusCode::FORBIDDEN,
            None,
            r#"{"__type":"AccessDeniedException","Message":"not authorized"}"#,
        );
        assert!(!unauthorized.retryable);
        assert!(unauthorized
            .error
            .to_string()
            .contains("AccessDeniedException"));

        let server = session.classify("GetQueryResults", StatusCode::BAD_GATEWAY, None, "bad");
        assert!(server.retryable);
    }

    #[test]
    fn classified_errors_redact_credentials_echoed_by_the_service() {
        let session = session();
        let hostile = r#"{"__type":"InvalidRequestException",
            "message":"key AKIDEXAMPLE secret topsecret token sessiontoken"}"#;
        let failure = session.classify(
            "StartQueryExecution",
            StatusCode::BAD_REQUEST,
            None,
            hostile,
        );
        let rendered = failure.error.to_string();
        assert!(!rendered.contains("topsecret"), "{rendered}");
        assert!(!rendered.contains("sessiontoken"), "{rendered}");
        assert!(!rendered.contains("AKIDEXAMPLE"), "{rendered}");
    }

    #[test]
    fn unparseable_error_bodies_are_excerpted_not_dropped() {
        let session = session();
        let failure = session.classify(
            "GetQueryResults",
            StatusCode::BAD_REQUEST,
            None,
            &"z".repeat(4096),
        );
        let rendered = failure.error.to_string();
        assert!(rendered.contains('…'), "{rendered}");
        assert!(
            rendered.len() < 1024,
            "excerpt not bounded: {}",
            rendered.len()
        );
    }

    #[test]
    fn session_rejects_a_config_whose_bounds_conflict() {
        let config = AthenaConfig::new("us-east-1")
            .unwrap()
            .with_total_deadline(std::time::Duration::from_millis(5))
            .unwrap();
        let creds = AthenaCredentials::new("AKID", "secret").unwrap();
        assert!(AthenaSession::new(config, creds).is_err());
    }

    #[tokio::test]
    async fn signed_requests_never_follow_redirects() {
        let destination = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(0)
            .mount(&destination)
            .await;

        let source = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(307).append_header("location", destination.uri()))
            .expect(1)
            .mount(&source)
            .await;

        let config = AthenaConfig::new("us-east-1")
            .unwrap()
            .with_endpoint(source.uri())
            .unwrap()
            .with_max_retries(0)
            .unwrap();
        let session = AthenaSession::new(
            config,
            AthenaCredentials::new("AKIDEXAMPLE", "topsecret")
                .unwrap()
                .with_session_token("sessiontoken")
                .unwrap(),
        )
        .unwrap();
        let error = session
            .call(
                "GetQueryExecution",
                &json!({"QueryExecutionId": "q-1"}),
                Instant::now() + Duration::from_secs(1),
            )
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("HTTP 307"), "{error}");
        source.verify().await;
        destination.verify().await;
    }
}
