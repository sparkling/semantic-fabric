//! Request-side verified-generation admission before authoritative compilation.

use std::sync::Arc;

use axum::response::Response;

use crate::activation::RuntimeSnapshotLease;
use crate::budget::RequestBudget;
use crate::config::ServeConfig;
use crate::deadline::CompilerReservation;
use crate::pg_generation::{PgGenerationError, VerifiedGenerationLeases};
use crate::problem::{self, ProblemCode};

pub(crate) struct RequestGenerationAdmission {
    generations: VerifiedGenerationLeases,
    compiler: Option<CompilerReservation>,
}

impl RequestGenerationAdmission {
    pub(crate) fn into_parts(self) -> (VerifiedGenerationLeases, Option<CompilerReservation>) {
        (self.generations, self.compiler)
    }
}

/// Discover dependencies without cache authority, then acquire and revalidate
/// every required backend-generation lease before authoritative compilation.
pub(crate) async fn acquire(
    cfg: Arc<ServeConfig>,
    snapshot: &RuntimeSnapshotLease,
    query: &str,
    budget: &RequestBudget,
) -> Result<RequestGenerationAdmission, Response> {
    cfg.query_admission
        .validate(budget)
        .map_err(problem::response)?;
    let source_ids = cfg.query_mode().source_ids().into_iter().flatten();
    if budget.postgres_rls().is_some() && source_ids.clone().any(|id| !snapshot.permits_rls(id)) {
        return Err(crate::pg_rls::denied());
    }
    let requirements = snapshot
        .generation_requirements(source_ids)
        .map_err(response_for_error)?;
    if requirements.is_empty() {
        return Ok(RequestGenerationAdmission {
            generations: VerifiedGenerationLeases::default(),
            compiler: None,
        });
    }

    let compiler = crate::request_compile::preflight(
        cfg.clone(),
        snapshot.clone(),
        query.to_owned(),
        budget.clone(),
    )
    .await?;
    let generations = VerifiedGenerationLeases::acquire(requirements, budget)
        .await
        .map_err(response_for_error)?;
    Ok(RequestGenerationAdmission {
        generations,
        compiler: Some(compiler),
    })
}

pub(crate) fn response_for_error(error: PgGenerationError) -> Response {
    match error {
        PgGenerationError::Control(error) => problem::response_for_control(error),
        PgGenerationError::SourceUnavailable => {
            problem::response_with_retry_after(ProblemCode::SourceUnavailable)
        }
        PgGenerationError::SchemaDrift | PgGenerationError::CapabilityDrift => {
            problem::response_with_retry_after(ProblemCode::SourceUnavailable)
        }
        PgGenerationError::Mapping(_) | PgGenerationError::Internal => {
            problem::response(ProblemCode::Internal)
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, StatusCode};
    use http_body_util::BodyExt;
    use sf_core::query_control::QueryControlError;

    use super::*;
    use crate::telemetry::CorrelationId;
    use crate::{Backend, IntrospectedSource};
    use sf_core::{SourceId, SourceMapping};

    fn config() -> ServeConfig {
        let source = IntrospectedSource::unchecked(
            Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
            Vec::new(),
        );
        ServeConfig::new(
            source,
            SourceMapping::new(SourceId::new(0).unwrap(), Vec::new()),
            crate::test_support::empty_ontology(),
        )
        .unwrap()
    }

    #[test]
    fn request_scoped_drift_does_not_change_runtime_readiness() {
        let config = config();
        let ready = config.runtime_readiness().unwrap();

        for error in [
            PgGenerationError::SchemaDrift,
            PgGenerationError::CapabilityDrift,
        ] {
            let response = response_for_error(error);
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(config.runtime_readiness().unwrap(), ready);
        }
    }

    enum RetryAfter {
        OneSecond,
        Absent,
        Unpinned,
    }

    struct Expected {
        status: StatusCode,
        code: &'static str,
        title: &'static str,
        detail: &'static str,
        retry_after: RetryAfter,
    }

    const SOURCE_UNAVAILABLE: Expected = Expected {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "source-unavailable",
        title: "Service Unavailable",
        detail: "The source is temporarily unavailable.",
        retry_after: RetryAfter::OneSecond,
    };

    const INTERNAL: Expected = Expected {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "internal-error",
        title: "Internal Server Error",
        detail: "The request could not be completed.",
        retry_after: RetryAfter::Absent,
    };

    const TIMEOUT: Expected = Expected {
        status: StatusCode::GATEWAY_TIMEOUT,
        code: "request-timeout",
        title: "Gateway Timeout",
        detail: "The request deadline expired.",
        retry_after: RetryAfter::Absent,
    };

    const BUDGET: Expected = Expected {
        status: StatusCode::TOO_MANY_REQUESTS,
        code: "query-budget-exceeded",
        title: "Too Many Requests",
        detail: "The query exceeded a resource limit.",
        retry_after: RetryAfter::Absent,
    };

    const OVERLOADED: Expected = Expected {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "service-overloaded",
        title: "Service Unavailable",
        detail: "The service is temporarily overloaded.",
        retry_after: RetryAfter::Unpinned,
    };

    fn control_expectations() -> [(QueryControlError, Expected); QueryControlError::VARIANT_COUNT] {
        [
            (QueryControlError::DeadlineExceeded, TIMEOUT),
            (QueryControlError::Cancelled, INTERNAL),
            (QueryControlError::CompilerEnvelopeExceeded, BUDGET),
            (QueryControlError::CompilerResourceExhausted, OVERLOADED),
            (QueryControlError::CompilerWorkExceeded, BUDGET),
            (QueryControlError::SourceWorkExceeded, BUDGET),
            (QueryControlError::ResultItemsExceeded, BUDGET),
            (QueryControlError::SerializedBytesExceeded, BUDGET),
            (QueryControlError::RetainedBytesExceeded, BUDGET),
            (QueryControlError::AccountingOverflow, INTERNAL),
        ]
    }

    fn expected_for_control(error: QueryControlError) -> Expected {
        let mut matches = control_expectations()
            .into_iter()
            .filter(|(candidate, _)| *candidate == error);
        let (_, expected) = matches
            .next()
            .unwrap_or_else(|| panic!("no pinned contract for {error:?}"));
        assert!(matches.next().is_none(), "duplicate contract {error:?}");
        expected
    }

    fn expected_for(error: &PgGenerationError) -> Expected {
        match error {
            PgGenerationError::Control(error) => expected_for_control(*error),
            PgGenerationError::SourceUnavailable
            | PgGenerationError::SchemaDrift
            | PgGenerationError::CapabilityDrift => SOURCE_UNAVAILABLE,
            PgGenerationError::Mapping(_) | PgGenerationError::Internal => INTERNAL,
        }
    }

    fn header(headers: &HeaderMap, name: &str) -> Option<String> {
        headers
            .get(name)
            .map(|value| value.to_str().unwrap().to_owned())
    }

    async fn assert_public_problem(
        mut response: Response,
        expected: &Expected,
        label: &str,
    ) -> (HeaderMap, String) {
        let correlation = CorrelationId::generate();
        let correlation_id = correlation.as_str();
        assert!(
            problem::finalize(&mut response, &correlation).is_some(),
            "{label}: response must carry a pending problem"
        );
        assert_eq!(response.status(), expected.status, "{label}");

        let headers = response.headers().clone();
        for (name, value) in [
            ("content-type", "application/problem+json"),
            ("cache-control", "no-store"),
            ("x-content-type-options", "nosniff"),
            ("x-correlation-id", correlation_id),
        ] {
            assert_eq!(
                header(&headers, name).as_deref(),
                Some(value),
                "{label}: header {name}"
            );
        }
        assert!(headers.get("www-authenticate").is_none(), "{label}");
        let retry = header(&headers, "retry-after");
        match expected.retry_after {
            RetryAfter::OneSecond => assert_eq!(retry.as_deref(), Some("1"), "{label}"),
            RetryAfter::Absent => assert_eq!(retry, None, "{label}"),
            RetryAfter::Unpinned => {}
        }

        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let length = header(&headers, "content-length");
        assert_eq!(length, Some(bytes.len().to_string()), "{label}");
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let expected_json = serde_json::json!({
            "type": format!("urn:semantic-fabric:problem:{}", expected.code),
            "title": expected.title,
            "status": expected.status.as_u16(),
            "detail": expected.detail,
            "instance": format!("urn:semantic-fabric:problem-instance:{correlation_id}"),
            "code": expected.code,
            "correlationId": correlation_id,
        });
        assert_eq!(json, expected_json, "{label}");
        (headers, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn every_generation_error_has_a_pinned_public_problem() {
        let mapping = sf_core::Error::Mapping("synthetic mapping".to_owned());
        let errors = [
            PgGenerationError::Control(QueryControlError::DeadlineExceeded),
            PgGenerationError::SourceUnavailable,
            PgGenerationError::SchemaDrift,
            PgGenerationError::CapabilityDrift,
            PgGenerationError::Mapping(mapping),
            PgGenerationError::Internal,
        ];
        for error in errors {
            let label = format!("{error:?}");
            let expected = expected_for(&error);
            let response = response_for_error(error);
            assert_public_problem(response, &expected, &label).await;
        }
    }

    #[tokio::test]
    async fn every_control_error_has_a_pinned_public_problem() {
        assert_eq!(
            control_expectations().len(),
            QueryControlError::VARIANTS.len()
        );
        for error in QueryControlError::VARIANTS {
            let expected = expected_for_control(error);
            let response = response_for_error(PgGenerationError::from(error));
            assert_public_problem(response, &expected, &format!("{error:?}")).await;
        }
    }

    #[tokio::test]
    async fn mapping_error_body_never_leaks_its_payload() {
        let sentinels = [
            "/opt/synthetic-sentinel/mapping-path.ttl",
            "SELECT synthetic_sentinel_column FROM synthetic_sentinel_table",
            "postgres://sentinel_user:synthetic-sentinel-credential-7f3a@sentinel.invalid/db",
        ];
        let payload = sf_core::Error::Mapping(sentinels.join(" "));
        let rendered_payload = payload.to_string();
        for sentinel in sentinels {
            assert!(rendered_payload.contains(sentinel));
        }

        let response = response_for_error(PgGenerationError::Mapping(payload));
        let label = "Mapping";
        let (headers, body) = assert_public_problem(response, &INTERNAL, label).await;
        let rendered_headers = format!("{headers:?}");
        for sentinel in sentinels {
            assert!(!body.contains(sentinel), "body leaked {sentinel}");
            assert!(
                !rendered_headers.contains(sentinel),
                "headers leaked {sentinel}"
            );
        }
        for fragment in [
            "sentinel",
            "synthetic",
            "credential",
            "postgres",
            "SELECT",
            "mapping error",
        ] {
            assert!(!body.contains(fragment), "body leaked {fragment}");
        }
    }
}
