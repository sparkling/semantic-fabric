use std::collections::BTreeSet;
use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use http_body_util::BodyExt;
use oxrdf::{NamedNode, NamedOrBlankNode, Term};
use oxttl::TurtleParser;
use sf_core::ir::{LogicalSource, SubjectMap, TermMap, TriplesMap};
use sf_core::{SourceId, SourceMapping};
use sf_sparql::Tbox;
use tokio_stream::StreamExt;
use tower::ServiceExt;

use super::{SINGLE_SOURCE, TWO_SOURCE};
use crate::{router, Backend, IntrospectedSource, ReadinessCause, RuntimeSource, ServeConfig};

const SECRET: &str = "do-not-expose-password-or-source-path";
const ENDPOINT: &str = "https://endpoint.example/sparql";
const SD: &str = "http://www.w3.org/ns/sparql-service-description#";
const SF: &str = "urn:semantic-fabric:service-description:";

fn sensitive_mapping(source_id: usize) -> SourceMapping {
    SourceMapping::new(
        SourceId::new(source_id).unwrap(),
        vec![TriplesMap {
            id: format!("file:///private/{SECRET}/{source_id}.ttl"),
            source: LogicalSource::Query(format!("SELECT '{SECRET}' AS credential")),
            subject: SubjectMap {
                term: TermMap::Constant(
                    NamedNode::new(format!("urn:test:subject:{source_id}"))
                        .unwrap()
                        .into(),
                ),
                classes: Vec::new(),
                graphs: Vec::new(),
            },
            predicate_object_maps: Vec::new(),
        }],
    )
}

fn sensitive_tbox() -> Tbox {
    let mut tbox = Tbox::new();
    tbox.add_subclass(format!("urn:test:{SECRET}"), "urn:test:private-class");
    tbox
}

fn runtime_source(source_id: usize) -> RuntimeSource {
    RuntimeSource::new(
        IntrospectedSource::unchecked(
            Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
            Vec::new(),
        ),
        sensitive_mapping(source_id),
    )
}

fn single_config() -> Arc<ServeConfig> {
    let mut config = ServeConfig::new(
        IntrospectedSource::unchecked(
            Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
            Vec::new(),
        ),
        sensitive_mapping(0),
        sensitive_tbox(),
    );
    config.set_max_concurrent_requests(1).unwrap();
    Arc::new(config)
}

fn two_source_config() -> Arc<ServeConfig> {
    Arc::new(
        ServeConfig::new_federated([runtime_source(3), runtime_source(7)], sensitive_tbox())
            .unwrap(),
    )
}

fn request(method: Method, accept: Option<&str>, body: Body) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri("/sparql")
        .header(header::HOST, format!("{SECRET}.invalid"));
    if let Some(accept) = accept {
        builder = builder.header(header::ACCEPT, accept);
    }
    builder.body(body).unwrap()
}

async fn send(config: Arc<ServeConfig>, request: Request<Body>) -> (StatusCode, HeaderMap, Bytes) {
    let response = router(config).oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, body)
}

fn parsed(document: &[u8]) -> Vec<oxrdf::Triple> {
    TurtleParser::new()
        .with_base_iri(ENDPOINT)
        .unwrap()
        .for_slice(document)
        .collect::<Result<_, _>>()
        .unwrap()
}

fn object_iris(triples: &[oxrdf::Triple], predicate: &str) -> BTreeSet<String> {
    triples
        .iter()
        .filter(|triple| triple.predicate.as_str() == predicate)
        .filter_map(|triple| match &triple.object {
            Term::NamedNode(node) => Some(node.as_str().to_owned()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn queryless_get_is_stable_redacted_rdf_with_a_matching_endpoint() {
    let config = single_config();
    let (status, headers, first) =
        send(config.clone(), request(Method::GET, None, Body::empty())).await;
    let (_, _, second) = send(
        config,
        request(Method::GET, Some("TEXT/TURTLE; q=0.5"), Body::empty()),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "text/turtle; charset=utf-8");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(headers[header::VARY], "Accept");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(
        headers[header::CONTENT_LENGTH],
        SINGLE_SOURCE.len().to_string()
    );
    assert_eq!(first, Bytes::from_static(SINGLE_SOURCE.as_bytes()));
    assert_eq!(
        first, second,
        "request metadata must not perturb fixed bytes"
    );
    assert!(first.len() < 2_048, "description must remain bounded");
    assert!(!first
        .windows(SECRET.len())
        .any(|part| part == SECRET.as_bytes()));

    let triples = parsed(&first);
    assert_eq!(triples.len(), 19);
    assert!(triples.iter().any(|triple| {
        matches!(&triple.subject, NamedOrBlankNode::NamedNode(node) if node.as_str() == ENDPOINT)
            && triple.predicate == oxrdf::vocab::rdf::TYPE
            && matches!(&triple.object, Term::NamedNode(node) if node.as_str() == format!("{SD}Service"))
    }));
    assert_eq!(
        object_iris(&triples, &format!("{SD}endpoint")),
        BTreeSet::from([ENDPOINT.to_owned()])
    );
    assert_eq!(
        object_iris(&triples, &format!("{SD}supportedLanguage")),
        BTreeSet::from([format!("{SF}bounded-read-query-v1")])
    );
    assert_eq!(
        object_iris(&triples, &format!("{SD}feature")),
        BTreeSet::from([
            format!("{SF}ask-query"),
            format!("{SF}construct-query"),
            format!("{SF}describe-one-target-one-hop-query-v1"),
            format!("{SF}select-query"),
        ])
    );
    assert_eq!(
        object_iris(&triples, &format!("{SD}resultFormat")),
        BTreeSet::from([
            "http://www.w3.org/ns/formats/JSON-LD".to_owned(),
            "http://www.w3.org/ns/formats/N-Triples".to_owned(),
            "http://www.w3.org/ns/formats/SPARQL_Results_CSV".to_owned(),
            "http://www.w3.org/ns/formats/SPARQL_Results_JSON".to_owned(),
            "http://www.w3.org/ns/formats/SPARQL_Results_TSV".to_owned(),
            "http://www.w3.org/ns/formats/SPARQL_Results_XML".to_owned(),
            "http://www.w3.org/ns/formats/Turtle".to_owned(),
        ])
    );
}

#[tokio::test]
async fn two_source_mode_adds_only_the_exact_union_feature() {
    let (_, _, single) = send(
        single_config(),
        request(Method::GET, Some("text/turtle"), Body::empty()),
    )
    .await;
    let config = two_source_config();
    let (_, _, first) = send(
        config.clone(),
        request(Method::GET, Some("*/*"), Body::empty()),
    )
    .await;
    let (_, _, second) = send(config, request(Method::GET, Some("text/*"), Body::empty())).await;

    assert_eq!(first, Bytes::from_static(TWO_SOURCE.as_bytes()));
    assert_eq!(first, second);
    assert_ne!(first, single);
    let triples = parsed(&first);
    assert_eq!(triples.len(), 12);
    let features = object_iris(&triples, &format!("{SD}feature"));
    assert_eq!(features.len(), 2);
    assert_eq!(
        object_iris(&triples, &format!("{SD}supportedLanguage")),
        BTreeSet::from([format!("{SF}two-source-select-union-query-v1")])
    );
    assert_eq!(
        object_iris(&triples, &format!("{SD}resultFormat")),
        BTreeSet::from([
            "http://www.w3.org/ns/formats/SPARQL_Results_CSV".to_owned(),
            "http://www.w3.org/ns/formats/SPARQL_Results_JSON".to_owned(),
            "http://www.w3.org/ns/formats/SPARQL_Results_TSV".to_owned(),
            "http://www.w3.org/ns/formats/SPARQL_Results_XML".to_owned(),
        ])
    );
    assert!(features.contains(&format!("{SF}source-affine-two-arm-select-union-v1")));
    let text = String::from_utf8(first.to_vec()).unwrap();
    for forbidden in [
        SECRET,
        "SPARQL11Query",
        "SPARQL11Update",
        "BasicFederatedQuery",
        "production-admission",
        "ask-query",
        "construct-query",
        "describe-one-target-one-hop-query-v1",
        "formats/Turtle",
        "formats/N-Triples",
        "formats/JSON-LD",
        "SourceId(3)",
        "SourceId(7)",
    ] {
        assert!(
            !text.contains(forbidden),
            "leaked or overclaimed {forbidden:?}"
        );
    }
}

#[tokio::test]
async fn accept_and_method_contract_is_exact() {
    for accept in [
        None,
        Some("text/turtle"),
        Some("text/*;q=0.2"),
        Some("application/ld+json;q=1, */*;q=0.1"),
    ] {
        let (status, _, body) =
            send(single_config(), request(Method::GET, accept, Body::empty())).await;
        assert_eq!(status, StatusCode::OK, "accept={accept:?}");
        assert_eq!(body, SINGLE_SOURCE);
    }

    for accept in [
        "application/ld+json",
        "text/turtle;q=0",
        "text/turtle;q=0, */*;q=1",
        "text/turtle;q=2",
    ] {
        let (status, headers, body) = send(
            single_config(),
            request(Method::GET, Some(accept), Body::empty()),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_ACCEPTABLE, "accept={accept:?}");
        assert_eq!(headers[header::CONTENT_TYPE], "application/problem+json");
        let problem: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(problem["code"], "not-acceptable");
        assert!(!body
            .windows(accept.len())
            .any(|part| part == accept.as_bytes()));
    }

    let (status, headers, body) = send(
        single_config(),
        request(Method::HEAD, Some("text/turtle"), Body::empty()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_LENGTH],
        SINGLE_SOURCE.len().to_string()
    );
    assert!(body.is_empty());

    let (status, _, _) = send(single_config(), request(Method::POST, None, Body::empty())).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let (status, headers, _) =
        send(single_config(), request(Method::PUT, None, Body::empty())).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(headers[header::ALLOW], "GET,HEAD,POST");
}

#[tokio::test]
async fn discovery_bypasses_query_admission_and_runtime_readiness() {
    let config = single_config();
    let activation = config.runtime_readiness().unwrap().activation_id();
    config
        .mark_runtime_not_ready(activation, ReadinessCause::SourceUnavailable)
        .unwrap();
    let held = config
        .request_admission_permits()
        .try_acquire_owned()
        .unwrap();
    let polls = Arc::new(AtomicUsize::new(0));
    let observed = polls.clone();
    let stream = tokio_stream::iter([Ok::<_, Infallible>(Bytes::from_static(b"ignored"))]).map(
        move |chunk| {
            observed.fetch_add(1, Ordering::SeqCst);
            chunk
        },
    );
    let (status, _, body) = send(
        config.clone(),
        request(Method::GET, None, Body::from_stream(stream)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, SINGLE_SOURCE);
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    assert_eq!(config.available_request_permits(), 0);

    let query = Request::builder()
        .uri("/sparql?query=ASK%20%7B%7D")
        .body(Body::empty())
        .unwrap();
    let (status, _, _) = send(config.clone(), query).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    config.begin_shutdown();
    let (status, _, body) = send(config.clone(), request(Method::GET, None, Body::empty())).await;
    assert_eq!(status, StatusCode::OK, "control metadata survives draining");
    assert_eq!(body, SINGLE_SOURCE);
    drop(held);
    assert_eq!(config.available_request_permits(), 1);
}
