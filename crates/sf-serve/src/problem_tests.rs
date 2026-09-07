use super::*;

#[test]
fn generated_ids_are_bounded_ascii_and_distinct() {
    let first = generated_correlation_id();
    let second = generated_correlation_id();
    assert_ne!(first, second);
    for id in [first, second] {
        assert_eq!(id.as_str().len(), 36, "id={id:?}");
        assert!(id.as_str().starts_with("sf-"));
        assert!(id
            .as_str()
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-'));
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
fn every_control_error_has_an_explicit_public_mapping() {
    for error in QueryControlError::VARIANTS {
        let mut mappings = CONTROL_PROBLEM_CODES
            .iter()
            .filter(|(candidate, _)| *candidate == error);
        let (_, code) = mappings.next().expect("variant has a public mapping");
        assert!(mappings.next().is_none(), "variant has one public mapping");
        assert_eq!(ProblemCode::from_control(error), *code);
    }
}

#[test]
fn every_problem_code_has_one_stable_status_and_public_value() {
    let cases = [
        (
            ProblemCode::InvalidRequest,
            StatusCode::BAD_REQUEST,
            "invalid-request",
        ),
        (ProblemCode::NotFound, StatusCode::NOT_FOUND, "not-found"),
        (
            ProblemCode::MethodNotAllowed,
            StatusCode::METHOD_NOT_ALLOWED,
            "method-not-allowed",
        ),
        (
            ProblemCode::NotAcceptable,
            StatusCode::NOT_ACCEPTABLE,
            "not-acceptable",
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
            ProblemCode::ServiceOverloaded,
            StatusCode::SERVICE_UNAVAILABLE,
            "service-overloaded",
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
        let details = ProblemDetails::new(code, &generated_correlation_id());
        assert_eq!(details.status, status.as_u16());
        assert_eq!(details.code, value);
        assert!(!details.title.is_empty());
        assert!(!details.detail.is_empty());
    }
    assert_eq!(
        ProblemCode::ServiceOverloaded.detail(),
        "The service is temporarily overloaded."
    );
}

#[test]
fn temporary_unavailability_uses_one_fixed_retry_hint() {
    for code in [
        ProblemCode::ServiceOverloaded,
        ProblemCode::SourceUnavailable,
    ] {
        let response = response_with_retry_after(code);
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers().get(header::RETRY_AFTER).unwrap(), "1");
    }
}

#[test]
fn pending_problem_can_only_be_materialized_once() {
    let mut response = response(ProblemCode::InvalidRequest);
    let correlation = generated_correlation_id();
    assert_eq!(
        finalize(&mut response, &correlation),
        Some(FailureKind::InvalidRequest)
    );
    assert_eq!(finalize(&mut response, &correlation), None);
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
    let request_deadline_source = include_str!("request_deadline.rs");
    let description_source = include_str!("service_description.rs");
    for (name, source) in [
        ("http.rs", http_source),
        ("request_deadline.rs", request_deadline_source),
        ("service_description.rs", description_source),
    ] {
        assert!(!source.contains("err_text("), "source={name}");
        assert!(!source.contains("response_for_status("), "source={name}");
    }
    for (name, source) in [
        ("http.rs", http_source),
        ("request_deadline.rs", request_deadline_source),
    ] {
        assert!(!source.contains("Body::from("), "source={name}");
    }
    assert_eq!(
        http_source.matches("Response::builder()").count(),
        1,
        "only the success response builder belongs in http.rs"
    );
    assert_eq!(
        request_deadline_source
            .matches("Response::builder()")
            .count(),
        0,
        "request_deadline.rs must use the closed problem vocabulary"
    );
}
