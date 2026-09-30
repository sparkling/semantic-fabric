use super::*;
use crate::generated_http_test_support::{
    config, open, post, send, ASK, CONSTRUCT, SELECT, UNKNOWN_SELECT,
};
use crate::{QueryAdmission, QueryShapeProfile};
use axum::body::Body;
use sf_core::query_control::{QueryLimits, UncontrolledQueryControl};
use std::time::Duration;

const GENERATED: QueryShapeProfile = QueryShapeProfile::GeneratedSelectAsk;
const EXECUTION: &str = execution::EXECUTION_HEADER;

#[tokio::test]
async fn generated_http_success_carries_exact_query_pair_on_cold_and_warm() {
    let (cfg, _) = config(GENERATED, open());
    let mut first = None;
    for query in [SELECT, SELECT, ASK] {
        let reply = send(
            &cfg,
            post(
                query,
                None,
                None,
                &[(EXECUTION, "spoof"), (PROFILE_HEADER, "spoof")],
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        let pair = (
            reply.headers[PROFILE_HEADER].clone(),
            reply.headers[EXECUTION].clone(),
        );
        if let Some((profile, execution)) = &first {
            assert_eq!(profile, &pair.0);
            if query == SELECT {
                assert_eq!(execution, &pair.1);
            } else {
                assert_ne!(execution, &pair.1);
            }
        } else {
            first = Some(pair);
        }
    }
}

#[tokio::test]
async fn ordinary_denied_and_refused_responses_have_neither_header() {
    for (profile, admission, query, status) in [
        (QueryShapeProfile::Ordinary, open(), SELECT, StatusCode::OK),
        (
            GENERATED,
            QueryAdmission::Deny,
            SELECT,
            StatusCode::FORBIDDEN,
        ),
        (GENERATED, open(), CONSTRUCT, StatusCode::NOT_IMPLEMENTED),
        (
            GENERATED,
            open(),
            UNKNOWN_SELECT,
            StatusCode::NOT_IMPLEMENTED,
        ),
    ] {
        let (cfg, _) = config(profile, admission);
        let reply = send(&cfg, post(query, None, None, &[(EXECUTION, "spoof")])).await;
        assert_eq!(reply.status, status, "{}", reply.text());
        assert!(!reply.headers.contains_key(EXECUTION));
        assert!(!reply.headers.contains_key(PROFILE_HEADER));
    }
}

fn issued(cfg: &ServeConfig) -> GeneratedResponseIdentity {
    let lease = cfg.runtime_lease().unwrap();
    let binding = lease
        .snapshot()
        .registry()
        .binding(SourceId::new(0).unwrap())
        .unwrap();
    let compiled = binding
        .compile_generated(SELECT, &UncontrolledQueryControl)
        .unwrap();
    execution::mint(
        binding,
        cfg,
        SELECT,
        compiled.identity,
        &cfg.request_budget(),
    )
    .unwrap()
}

#[tokio::test]
async fn first_chunk_failure_or_cancellation_does_not_issue_either_identity() {
    let (cfg, _) = config(GENERATED, open());
    let pair = issued(&cfg);
    for body in [
        Body::empty(),
        Body::from_stream(tokio_stream::iter([Err::<axum::body::Bytes, _>(
            std::io::Error::other("private diagnostic"),
        )])),
    ] {
        let mut response = Response::new(body);
        response
            .headers_mut()
            .insert(EXECUTION, HeaderValue::from_static("spoof"));
        let result = attach_issued(response, &pair, &cfg.request_budget()).await;
        assert_eq!(result.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!result.headers().contains_key(EXECUTION));
        assert!(!result.headers().contains_key(PROFILE_HEADER));
    }
    let budget = RequestBudget::after(
        Duration::ZERO,
        QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
    );
    let result = attach_issued(Response::new(Body::from("prefix")), &pair, &budget).await;
    assert_eq!(result.status(), StatusCode::GATEWAY_TIMEOUT);
    assert!(!result.headers().contains_key(EXECUTION));
    assert!(!result.headers().contains_key(PROFILE_HEADER));
}

#[tokio::test]
async fn pair_attaches_after_first_chunk_without_waiting_for_complete_stream() {
    use http_body_util::BodyExt;
    use tokio_stream::wrappers::ReceiverStream;
    let (cfg, _) = config(GENERATED, open());
    let pair = issued(&cfg);
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    sender
        .send(Ok::<_, std::io::Error>(axum::body::Bytes::from_static(
            b"prefix",
        )))
        .await
        .unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        attach_issued(
            Response::new(Body::from_stream(ReceiverStream::new(receiver))),
            &pair,
            &cfg.request_budget(),
        ),
    )
    .await
    .expect("first chunk, not full stream");
    assert_eq!(result.headers()[EXECUTION], pair.execution_wire());
    assert_eq!(result.headers()[PROFILE_HEADER], pair.profile().wire());
    let mut body = result.into_body();
    assert_eq!(
        body.frame().await.unwrap().unwrap().into_data().unwrap(),
        b"prefix"[..]
    );
    assert!(!sender.is_closed());
    drop(body);
    assert!(sender.is_closed());
}

fn neither(response: &Response) {
    assert!(!response.headers().contains_key(EXECUTION));
    assert!(!response.headers().contains_key(PROFILE_HEADER));
}

#[tokio::test]
async fn actual_cancellation_does_not_issue_headers_and_late_error_stays_error() {
    use http_body_util::BodyExt;
    use sf_core::query_control::QueryControlError;
    let (cfg, _) = config(GENERATED, open());
    let pair = issued(&cfg);
    let budget = cfg.request_budget();
    budget.terminate(QueryControlError::Cancelled);
    let response = attach_issued(Response::new(Body::from("prefix")), &pair, &budget).await;
    assert!(!response.status().is_success());
    neither(&response);
    assert_eq!(budget.checkpoint(), Err(QueryControlError::Cancelled));

    let stream = tokio_stream::iter([
        Ok(axum::body::Bytes::from_static(b"prefix")),
        Err(std::io::Error::other("private late source error")),
    ]);
    let response = attach_issued(
        Response::new(Body::from_stream(stream)),
        &pair,
        &cfg.request_budget(),
    )
    .await;
    assert!(response.headers().contains_key(EXECUTION));
    let mut body = response.into_body();
    assert!(body.frame().await.unwrap().is_ok());
    assert!(
        body.frame().await.unwrap().is_err(),
        "identity is not completion proof"
    );
}

#[tokio::test]
async fn real_backend_start_failure_and_budget_refusal_have_neither_header() {
    use crate::generated_http_test_support::build;
    for query in [SELECT, ASK] {
        let (cfg, pool) = config(GENERATED, open());
        let warm = send(&cfg, post(query, None, None, &[])).await;
        assert_eq!(warm.status, StatusCode::OK);
        pool.pick()
            .lock()
            .unwrap()
            .execute_batch("DROP TABLE people")
            .unwrap();
        let result = send(&cfg, post(query, None, None, &[])).await;
        assert_eq!(result.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!result.headers.contains_key(EXECUTION));
        assert!(!result.headers.contains_key(PROFILE_HEADER));
    }
    let (cfg, _) = build(GENERATED, open(), |cfg| {
        cfg.query_limits = QueryLimits::new(0, u64::MAX, u64::MAX, u64::MAX);
    });
    let result = send(&cfg, post(SELECT, None, None, &[])).await;
    assert!(!result.status.is_success());
    assert_eq!(result.json()["code"], "query-budget-exceeded");
    assert!(!result.headers.contains_key(EXECUTION));
    assert!(!result.headers.contains_key(PROFILE_HEADER));
    let (cfg, _) = config(GENERATED, open());
    let result = send(
        &cfg,
        post(
            "SELECT * WHERE { SERVICE <http://ex/service> { ?s ?p ?o } }",
            None,
            None,
            &[],
        ),
    )
    .await;
    assert_eq!(result.status, StatusCode::NOT_IMPLEMENTED);
    assert!(!result.headers.contains_key(EXECUTION));
    assert!(!result.headers.contains_key(PROFILE_HEADER));
}

#[tokio::test]
async fn secured_missing_wrong_context_and_row_denial_have_neither_header() {
    use crate::{
        PortableRowPolicy, PortableRowRule, ProvisionedBearerAdmission, ProvisionedBearerSubject,
    };
    use axum::http::{header, HeaderMap};
    const TOKEN: &str = "test-only-execution-binding-token-0123456789";
    let admission = |table: &str| {
        let rows =
            PortableRowPolicy::new(vec![PortableRowRule::new(0, table, "tenant", "a").unwrap()])
                .unwrap();
        let subject = ProvisionedBearerSubject::portable_rows("fixture", TOKEN, rows).unwrap();
        QueryAdmission::ProvisionedBearers(ProvisionedBearerAdmission::new(vec![subject]).unwrap())
    };
    let (cfg, _) = config(GENERATED, admission("people"));
    let (wrong, _) = config(GENERATED, admission("other"));
    for token in [None, Some("wrong-test-token")] {
        let reply = send(&cfg, post(SELECT, token, None, &[])).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        assert!(!reply.headers.contains_key(EXECUTION));
        assert!(!reply.headers.contains_key(PROFILE_HEADER));
    }
    let reply = send(&wrong, post(SELECT, Some(TOKEN), None, &[])).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert!(!reply.headers.contains_key(EXECUTION));
    assert!(!reply.headers.contains_key(PROFILE_HEADER));
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {TOKEN}")).unwrap(),
    );
    let mut wrong_budget = cfg.request_budget();
    wrong_budget
        .retain_authenticated(wrong.query_admission.admit(&headers).unwrap())
        .unwrap();
    for budget in [cfg.request_budget(), wrong_budget] {
        let result = compile(
            cfg.clone(),
            cfg.runtime_lease().unwrap(),
            SELECT.into(),
            budget,
            None,
        )
        .await;
        let response = result.err().expect("missing/wrong bound context denied");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        neither(&response);
    }
}
