//! Real-Router tests for the opt-in generated-query HTTP profile.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use sf_core::query_control::{QueryLimits, UncontrolledQueryControl};
use sf_core::SourceId;
use sf_sparql::Epoch;

use crate::budget::RequestBudget;
use crate::generated_http_test_support::*;
use crate::{QueryShapeProfile, RuntimeSnapshot, RuntimeSource, ServeConfig};

const GENERATED: QueryShapeProfile = QueryShapeProfile::GeneratedSelectAsk;
const ORDINARY: QueryShapeProfile = QueryShapeProfile::Ordinary;
const LINEAGE: &str = "application/vnd.semantic-fabric.lineage+json-seq";
const MIXED_LINEAGE: &str = "Application/Vnd.Semantic-Fabric.Lineage+json-seq;q=0.5";
const DESCRIBE: &str = "DESCRIBE <http://ex/person/1>";
const SERVICE: &str = "ASK { SERVICE <http://ex/s> { ?s ?p ?o } }";
const FROM: &str = "SELECT * FROM <http://ex/g> WHERE { ?s ?p ?o }";
const FROM_NAMED: &str = "ASK FROM NAMED <http://ex/g> { ?s ?p ?o }";
const UPDATE: &str = "INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }";

#[tokio::test]
async fn select_and_ask_return_one_stable_identity_cold_and_warm() {
    let (cfg, _) = config(GENERATED, open());
    let mut seen = Vec::new();
    for query in [SELECT, ASK, SELECT, ASK] {
        let reply = send(&cfg, post(query, None, None, &[])).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        if query == SELECT {
            let text = reply.text();
            let both = text.contains("Alice") && text.contains("Bob");
            assert!(both, "{text}");
        } else {
            assert_eq!(reply.json()["boolean"], true);
        }
        seen.push(reply.identity().expect("identity header"));
    }
    assert_wire(&seen[0]);
    assert!(seen.iter().all(|wire| wire == &seen[0]));
}

#[tokio::test]
async fn negotiation_and_correlation_survive_identity_attachment() {
    let (cfg, _) = config(GENERATED, open());
    let reply = send(&cfg, post(SELECT, None, Some("text/csv"), &[])).await;
    assert_eq!(reply.status, StatusCode::OK);
    let content_type = reply.headers.get("content-type").unwrap();
    assert!(content_type.to_str().unwrap().starts_with("text/csv"));
    assert!(reply.text().contains("Alice"));
    assert!(reply.headers.contains_key("x-correlation-id"));
    assert_wire(&reply.identity().unwrap());
}

#[tokio::test]
async fn uncovered_constant_is_refused_by_the_profile_but_empty_200_by_default() {
    let (generated, _) = config(GENERATED, open());
    let reply = send(&generated, post(UNKNOWN_SELECT, None, None, &[])).await;
    assert_refusal(&reply, "coverage-refused");

    let (ordinary, _) = config(ORDINARY, open());
    let reply = send(&ordinary, post(UNKNOWN_SELECT, None, None, &[])).await;
    assert_eq!(reply.status, StatusCode::OK);
    let json = reply.json();
    assert_eq!(json["results"]["bindings"], serde_json::json!([]));
    assert!(reply.identity().is_none());
}

#[tokio::test]
async fn indeterminate_template_refuses_before_source_admission() {
    use crate::semantic_admission::generated_mapping_coverage::{ConstantRole, Coverage};
    use crate::semantic_admission::{MappingOrigin, ValidatedMapping};
    let (observed, pool) = source();
    let text = MAPPING.replace("person/{id}", "person/{id}{tenant}");
    let mapping = || sf_mapping::parse_r2rml_for_source(&text, SourceId::new(0).unwrap()).unwrap();
    let terms = ontology(&[]);
    let validated =
        ValidatedMapping::validate(mapping(), MappingOrigin::Authored, &terms, &observed).unwrap();
    assert_eq!(
        validated
            .coverage()
            .classify(
                ConstantRole::Subject,
                "http://ex/person/1a",
                &UncontrolledQueryControl
            )
            .unwrap(),
        Coverage::Indeterminate
    );
    let mut cfg = ServeConfig::new(observed, mapping(), terms).unwrap();
    cfg.set_query_admission(open());
    cfg.set_query_shape_profile(GENERATED);
    let cfg = Arc::new(cfg);
    let held = pool.pick_owned().acquire().await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    pool.set_admission_pending_observer(move || {
        observed.fetch_add(1, Ordering::SeqCst);
    });
    let query = "ASK { <http://ex/person/1a> <http://ex/name> ?n }";
    let reply = tokio::time::timeout(
        Duration::from_secs(2),
        send(&cfg, post(query, None, None, &[])),
    )
    .await
    .expect("indeterminate refusal must not wait for source");
    assert_refusal(&reply, "coverage-refused");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop(held);
}

#[tokio::test]
async fn execution_start_failure_has_no_issued_identity() {
    for query in [ASK, SELECT] {
        execution_start_failure(query).await;
    }
}

async fn execution_start_failure(query: &str) {
    let (cfg, pool) = config(GENERATED, open());
    let warm = send(&cfg, post(query, None, None, &[])).await;
    assert_eq!(warm.status, StatusCode::OK);
    assert!(warm.identity().is_some());
    pool.pick()
        .lock()
        .unwrap()
        .execute_batch("DROP TABLE people")
        .unwrap();
    let lease = cfg.runtime_lease().unwrap();
    let binding = lease
        .snapshot()
        .registry()
        .binding(SourceId::new(0).unwrap())
        .unwrap();
    assert!(binding
        .compile_generated(query, &UncontrolledQueryControl)
        .is_ok());
    let reply = send(&cfg, post(query, None, None, &[])).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(reply.identity().is_none());
    assert_eq!(reply.json()["code"], "internal-error");
    assert!(reply.json().get("rule").is_none());
    assert_eq!(reply.headers["content-type"], "application/problem+json");
    assert_eq!(
        reply.headers["content-length"].to_str().unwrap(),
        reply.body.len().to_string()
    );
    assert!(reply.headers.contains_key("x-correlation-id"));
    for secret in ["people", "http://ex", "SELECT", "no such table"] {
        assert!(!reply.text().contains(secret));
    }
}

#[tokio::test]
async fn issued_identity_waits_for_one_chunk_not_stream_completion() {
    use axum::body::{Body, Bytes};
    use http_body_util::BodyExt;
    use tokio_stream::wrappers::ReceiverStream;

    let (cfg, _) = config(GENERATED, open());
    let lease = cfg.runtime_lease().unwrap();
    let compiled = lease
        .snapshot()
        .registry()
        .binding(SourceId::new(0).unwrap())
        .unwrap()
        .compile_generated(SELECT, &UncontrolledQueryControl)
        .unwrap();
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    sender
        .send(Ok::<_, std::io::Error>(Bytes::from_static(b"prefix")))
        .await
        .unwrap();
    let response = axum::response::Response::new(Body::from_stream(ReceiverStream::new(receiver)));
    let budget = cfg.request_budget();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        crate::generated_request::attach(response, &compiled.identity, &budget),
    )
    .await
    .expect("must not wait for sender EOF");
    assert_eq!(response.headers()[HEADER], compiled.identity.wire());
    let mut body = response.into_body();
    assert_eq!(
        body.frame().await.unwrap().unwrap().into_data().unwrap(),
        b"prefix"[..]
    );
    assert!(!sender.is_closed());
    drop(body);
    assert!(
        sender.is_closed(),
        "dropping wrapped body retains cancellation custody"
    );
}

#[tokio::test]
async fn identity_start_gate_keeps_original_control_refusal() {
    use axum::body::Body;
    let (cfg, _) = config(GENERATED, open());
    let lease = cfg.runtime_lease().unwrap();
    let compiled = lease
        .snapshot()
        .registry()
        .binding(SourceId::new(0).unwrap())
        .unwrap()
        .compile_generated(SELECT, &UncontrolledQueryControl)
        .unwrap();
    let budget = RequestBudget::after(
        std::time::Duration::ZERO,
        sf_core::query_control::QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
    );
    let response = crate::generated_request::attach(
        axum::response::Response::new(Body::from("prefix")),
        &compiled.identity,
        &budget,
    )
    .await;
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert!(!response.headers().contains_key(HEADER));
}

#[tokio::test]
async fn a_default_cache_warmed_uncovered_query_is_still_refused() {
    let (cfg, _) = config(GENERATED, open());
    let lease = cfg.runtime_lease().unwrap();
    let source_id = SourceId::new(0).unwrap();
    let warmed = lease.compile(source_id, UNKNOWN_SELECT, &UncontrolledQueryControl);
    assert!(warmed.is_ok());
    drop(lease);
    for _ in 0..2 {
        let reply = send(&cfg, post(UNKNOWN_SELECT, None, None, &[])).await;
        assert_refusal(&reply, "coverage-refused");
    }
}

#[tokio::test]
async fn unsupported_forms_are_refused_with_named_rules() {
    let cases = [
        (CONSTRUCT, "construct-form"),
        (DESCRIBE, "describe-form"),
        (SERVICE, "service-in-pattern"),
        (FROM, "dataset-clause"),
        (FROM_NAMED, "dataset-clause"),
        (UPDATE, "form-not-admitted"),
        ("not sparql", "form-not-admitted"),
    ];
    let (cfg, _) = config(GENERATED, open());
    for (query, rule) in cases {
        let reply = send(&cfg, post(query, None, None, &[])).await;
        assert_refusal(&reply, rule);
    }
}

#[tokio::test]
async fn federation_and_lineage_are_refused_explicitly() {
    let cfg = federated(GENERATED);
    let reply = send(&cfg, post(SELECT, None, None, &[])).await;
    assert_refusal(&reply, "federation-unsupported");

    let (cfg, _) = config(GENERATED, open());
    for accept in [LINEAGE, MIXED_LINEAGE] {
        let reply = send(&cfg, post(SELECT, None, Some(accept), &[])).await;
        assert_refusal(&reply, "lineage-unsupported");
    }
}

#[tokio::test]
async fn resource_causes_keep_their_typed_status_and_are_not_form_refusals() {
    let limits = QueryLimits::new(1, u64::MAX, u64::MAX, u64::MAX);
    let (cfg, _) = build(GENERATED, open(), |cfg| cfg.query_limits = limits);
    let reply = send(&cfg, post(SELECT, None, None, &[])).await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    let json = reply.json();
    assert_eq!(json["code"], "query-budget-exceeded");
    assert!(json.get("rule").is_none());
    assert!(reply.identity().is_none());
}

#[tokio::test]
async fn refusals_complete_while_the_source_is_held_and_never_reach_it() {
    let (cfg, pool) = config(GENERATED, open());
    let held = pool.pick_owned().acquire().await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    pool.set_admission_pending_observer(move || {
        observed.fetch_add(1, Ordering::SeqCst);
    });
    for query in [UNKNOWN_SELECT, CONSTRUCT] {
        let request = send(&cfg, post(query, None, None, &[]));
        let reply = tokio::time::timeout(Duration::from_secs(2), request)
            .await
            .expect("refusal must not wait for the source");
        assert_eq!(reply.status, StatusCode::NOT_IMPLEMENTED);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop(held);
}

#[tokio::test]
async fn a_missing_parser_fails_closed_without_identity() {
    let (observed, _) = source();
    let terms = ontology(&[]);
    let mut cfg = ServeConfig::from_authored_r2rml(observed, MAPPING, terms).unwrap();
    cfg.set_query_admission(open());
    cfg.set_query_shape_profile(GENERATED);
    let reply = send(&Arc::new(cfg), post(SELECT, None, None, &[])).await;
    assert!(reply.status.is_server_error(), "{}", reply.status);
    assert!(reply.identity().is_none());
}

#[tokio::test]
async fn direct_preflight_refuses_without_leaking_compiler_capacity() {
    let (cfg, _) = config(GENERATED, open());
    let lease = cfg.runtime_lease().unwrap();
    let query = UNKNOWN_SELECT.to_owned();
    let budget = cfg.request_budget();
    let refused = crate::request_compile::preflight(cfg.clone(), lease.clone(), query, budget);
    let refused = refused.await.err().expect("refused");
    assert_eq!(refused.status(), StatusCode::NOT_IMPLEMENTED);

    let budget = cfg.request_budget();
    let admitted = crate::request_compile::preflight(cfg.clone(), lease, SELECT.into(), budget);
    let reservation = admitted.await.expect("covered preflight is admitted");
    drop(reservation);
    let permits = cfg.compiler_permits().acquire_many_owned(4);
    let recovered = tokio::time::timeout(Duration::from_secs(1), permits).await;
    drop(recovered.expect("compiler slots recover").unwrap());
}

#[tokio::test]
async fn an_old_lease_keeps_its_identity_while_new_requests_get_the_new_one() {
    let (cfg, _) = config(GENERATED, open());
    let old = cfg.runtime_lease().unwrap();
    let first = send(&cfg, post(SELECT, None, None, &[])).await;
    let first = first.identity().unwrap();
    let (replacement, _) = source();
    let candidate = RuntimeSource::new(replacement, mapping(0));
    let terms = ontology(&["http://ex/extra"]);
    let snapshot = RuntimeSnapshot::single(Epoch(1), terms, candidate).unwrap();
    let ready = cfg.runtime_readiness().unwrap();
    cfg.activate_snapshot(ready, snapshot).unwrap();

    let budget = cfg.request_budget();
    let query = SELECT.to_owned();
    let pinned = crate::generated_request::compile(cfg.clone(), old, query, budget, None);
    let (_plan, pinned) = pinned.await.unwrap();
    assert_eq!(pinned.wire(), first);

    let after = send(&cfg, post(SELECT, None, None, &[])).await;
    let after = after.identity().unwrap();
    assert_ne!(after, first);
    assert_wire(&after);
    let again = send(&cfg, post(SELECT, None, None, &[])).await;
    assert_eq!(again.identity().unwrap(), after);
}

#[tokio::test]
async fn the_default_profile_serves_unchanged_and_issues_no_identity() {
    assert_eq!(QueryShapeProfile::default(), ORDINARY);
    let (cfg, _) = config(ORDINARY, open());
    assert_eq!(cfg.query_shape_profile(), ORDINARY);
    for query in [SELECT, ASK, CONSTRUCT] {
        let reply = send(&cfg, post(query, None, None, &[])).await;
        assert_eq!(reply.status, StatusCode::OK, "{query}");
        assert!(reply.identity().is_none());
    }
}

#[test]
fn every_generated_compiler_pass_parses_exactly_once() {
    isolated_async(|| async {
        let (cfg, _) = config(GENERATED, open());
        let ok = StatusCode::OK;
        let refused = StatusCode::NOT_IMPLEMENTED;
        let requests = [
            (SELECT, ok),
            (SELECT, ok),
            (ASK, ok),
            (UNKNOWN_SELECT, refused),
            (UNKNOWN_SELECT, refused),
            (CONSTRUCT, refused),
            (DESCRIBE, refused),
            (FROM, refused),
            (UPDATE, refused),
            ("not sparql", refused),
        ];
        for (query, status) in requests {
            let (reply, parsed) = counted(&cfg, post(query, None, None, &[])).await;
            assert_eq!(reply.status, status, "{query}");
            assert_eq!(parsed, 1, "{query}");
        }
        let (reply, parsed) = counted(&cfg, post(SELECT, None, Some(LINEAGE), &[])).await;
        assert_refusal(&reply, "lineage-unsupported");
        assert_eq!(parsed, 0);
        let federation = federated(GENERATED);
        let (reply, parsed) = counted(&federation, post(SELECT, None, None, &[])).await;
        assert_refusal(&reply, "federation-unsupported");
        assert_eq!(parsed, 0);

        let passes = [
            (SELECT, true),
            (ASK, true),
            (UNKNOWN_SELECT, false),
            (CONSTRUCT, false),
            ("not sparql", false),
        ];
        for (query, admitted) in passes {
            let lease = cfg.runtime_lease().unwrap();
            let budget = cfg.request_budget();
            reset_parse_count();
            let preflight = crate::request_compile::preflight(
                cfg.clone(),
                lease.clone(),
                query.to_owned(),
                budget.clone(),
            )
            .await;
            assert_eq!(parse_count(), 1, "{query} preflight");
            let reservation = match preflight {
                Ok(reservation) => reservation,
                Err(response) => {
                    assert!(!admitted, "{query}");
                    assert_eq!(response.status(), refused, "{query}");
                    continue;
                }
            };
            assert!(admitted, "{query}");
            reset_parse_count();
            let compiled = crate::generated_request::compile(
                cfg.clone(),
                lease,
                query.to_owned(),
                budget,
                Some(reservation),
            )
            .await;
            assert!(compiled.is_ok(), "{query}");
            assert_eq!(parse_count(), 1, "{query} authoritative");
        }
        for query in [UNKNOWN_SELECT, CONSTRUCT, UPDATE, "not sparql"] {
            let lease = cfg.runtime_lease().unwrap();
            let budget = cfg.request_budget();
            reset_parse_count();
            let compiled = crate::generated_request::compile(
                cfg.clone(),
                lease,
                query.to_owned(),
                budget,
                None,
            )
            .await;
            let response = compiled.err().expect("refused");
            assert_eq!(response.status(), refused, "{query}");
            assert_eq!(parse_count(), 1, "{query} authoritative");
        }
    });
}
