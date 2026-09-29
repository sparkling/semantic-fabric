//! Ordinary SQLite profile: ASK, CONSTRUCT and lineage across schema drift
//! with a sealed pool of two members, on the real ordinary startup fixture.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use sf_core::query_control::QueryLimits;
use sf_core::SourceId;
use tower::ServiceExt;

use super::{replace_with_semantically_different_table, Fixture};
use crate::budget::RequestBudget;

type Config = Arc<crate::ServeConfig>;
type Generation = Arc<crate::sqlite_generation::SqliteGeneration>;

const POOL: usize = 2;
const ATTEMPTS: usize = 6;
/// A held lease keeps its acquisition budget: it must outlive every request
/// deadline the pool proof waits through, or the hold itself would expire.
const HOLD: Duration = Duration::from_secs(60);
const JSON: &str = "application/sparql-results+json";
const NTRIPLES: &str = "application/n-triples";
const ASK_OBSERVED: &str = "ASK { ?p <http://example.test/age> ?age FILTER(?age = 30) }";
const ASK_REPLACED: &str = "ASK { ?p <http://example.test/age> ?age FILTER(?age = 360) }";
const SELECT: &str = "SELECT ?age WHERE { ?p <http://example.test/age> ?age }";
const CONSTRUCT: &str =
    "CONSTRUCT { ?p <http://example.test/age> ?age } WHERE { ?p <http://example.test/age> ?age }";

#[derive(Clone, Copy, Debug)]
enum Form {
    Ask,
    Construct,
    LineageSelect,
    LineageConstruct,
}

#[derive(Clone, Copy, PartialEq)]
enum Era {
    Observed,
    Replaced,
}

impl Form {
    /// One public request of this form, used to inspect the refusal body.
    fn probe(self) -> (&'static str, &'static str) {
        match self {
            Self::Ask => (ASK_OBSERVED, JSON),
            Self::Construct => (CONSTRUCT, NTRIPLES),
            Self::LineageSelect => (SELECT, crate::lineage::MEDIA_TYPE),
            Self::LineageConstruct => (CONSTRUCT, crate::lineage::MEDIA_TYPE),
        }
    }

    fn is_lineage(self) -> bool {
        matches!(self, Self::LineageSelect | Self::LineageConstruct)
    }
}

/// Status and normalized answer, with the raw body kept for failure messages.
struct Reply {
    status: StatusCode,
    answer: Vec<String>,
    body: String,
}

fn reply(status: StatusCode, answer: Vec<String>, body: &[u8]) -> Reply {
    let body = String::from_utf8_lossy(body).into_owned();
    Reply {
        status,
        answer,
        body,
    }
}

fn assert_reply(actual: &Reply, status: StatusCode, answer: &[String], context: &str) {
    assert_eq!(
        (actual.status, actual.answer.as_slice()),
        (status, answer),
        "{context}; body: {}",
        actual.body
    );
}

fn assert_refused(actual: &Reply, context: &str) {
    assert_eq!(
        actual.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{context} must refuse, not answer {:?}; body: {}",
        actual.answer,
        actual.body
    );
}

fn budget(limit: Duration) -> RequestBudget {
    RequestBudget::after(
        limit,
        QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
    )
}

fn sealed_generation(config: &crate::ServeConfig) -> Generation {
    let snapshot = config.lifecycle_runtime().lease().unwrap();
    let mut requirements = snapshot
        .generation_requirements([SourceId::new(0).unwrap()])
        .unwrap();
    match requirements.pop() {
        Some(crate::generation::GenerationRequirement::Sqlite { expected, .. }) => expected,
        _ => panic!("ordinary startup must select the sealed SQLite generation"),
    }
}

/// The activated runtime's lineage identity: request-scoped refusals must not
/// replace, rebuild or retire the runtime that served before the drift.
fn runtime_identity(config: &crate::ServeConfig) -> Vec<u8> {
    let lease = config.lifecycle_runtime().lease().unwrap();
    let identity = lease.snapshot().lineage_identity();
    identity.as_bytes().to_vec()
}

/// The activated runtime's readiness, which must be `Ready` when captured.
fn ready_state(config: &crate::ServeConfig) -> crate::RuntimeReadiness {
    let readiness = config.runtime_readiness().unwrap();
    assert!(
        matches!(readiness, crate::RuntimeReadiness::Ready { .. }),
        "activated runtime must be ready: {readiness:?}"
    );
    readiness
}

/// Readiness through the public accessor and the real `/readyz` route.
async fn assert_readiness(config: &Config, expected: crate::RuntimeReadiness, context: &str) {
    assert_eq!(config.runtime_readiness().unwrap(), expected, "{context}");
    let request = Request::builder()
        .uri("/readyz")
        .body(Body::empty())
        .unwrap();
    let response = crate::router(config.clone())
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{context}");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body, r#"{"status":"ready"}"#, "{context}");
}

async fn call(config: &Config, query: &str, accept: &str) -> (StatusCode, Vec<u8>) {
    let response = crate::router(config.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sparql")
                .header(header::CONTENT_TYPE, "application/sparql-query")
                .header(header::ACCEPT, accept)
                .body(Body::from(query.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await;
    let body = body.map(|body| body.to_bytes().to_vec());
    (status, body.unwrap_or_default())
}

async fn ask_facts(config: &Config) -> Reply {
    let mut facts = Vec::new();
    let mut raw = Vec::new();
    for (label, query) in [("observed", ASK_OBSERVED), ("replaced", ASK_REPLACED)] {
        let (status, body) = call(config, query, JSON).await;
        if status != StatusCode::OK {
            return reply(status, Vec::new(), &body);
        }
        let json: Value = serde_json::from_slice(&body).expect("ASK JSON");
        facts.push(format!("{label}={}", json["boolean"]));
        raw.extend_from_slice(&body);
    }
    reply(StatusCode::OK, facts, &raw)
}

async fn construct_lines(config: &Config) -> Reply {
    let (status, body) = call(config, CONSTRUCT, NTRIPLES).await;
    if status != StatusCode::OK {
        return reply(status, Vec::new(), &body);
    }
    let text = String::from_utf8_lossy(&body);
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    lines.sort();
    reply(status, lines, &body)
}

fn collect_leaves(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.clone()),
        Value::Number(number) => out.push(number.to_string()),
        Value::Array(items) => {
            for item in items {
                collect_leaves(item, out);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                collect_leaves(item, out);
            }
        }
        _ => {}
    }
}

/// A result value appears either as a plain lexical form or as a typed term.
fn mentions(leaves: &[String], age: &str) -> bool {
    let typed = format!("\"{age}\"^^");
    let hit = |leaf: &String| leaf == age || leaf.contains(&typed);
    leaves.iter().any(hit)
}

async fn lineage_ages(config: &Config, query: &str) -> Reply {
    let (status, body) = call(config, query, crate::lineage::MEDIA_TYPE).await;
    if status != StatusCode::OK {
        return reply(status, Vec::new(), &body);
    }
    let records: Vec<Value> = body
        .split(|byte| *byte == 0x1e)
        .filter(|part| !part.is_empty())
        .map(|part| serde_json::from_slice(part).expect("lineage record"))
        .collect();
    assert!(
        records.len() >= 2,
        "lineage stream needs header and complete"
    );
    assert_eq!(records[0]["type"], "header");
    assert_eq!(records[records.len() - 1]["type"], "complete");
    let mut leaves = Vec::new();
    for record in &records[1..records.len() - 1] {
        collect_leaves(record, &mut leaves);
    }
    let seen = ["25", "30", "300", "360"]
        .into_iter()
        .filter(|age| mentions(&leaves, age))
        .map(str::to_owned)
        .collect();
    reply(status, seen, &body)
}

async fn answer(config: &Config, form: Form) -> Reply {
    match form {
        Form::Ask => ask_facts(config).await,
        Form::Construct => construct_lines(config).await,
        Form::LineageSelect => lineage_ages(config, SELECT).await,
        Form::LineageConstruct => lineage_ages(config, CONSTRUCT).await,
    }
}

fn triple(id: u32, age: &str) -> String {
    let integer = "http://www.w3.org/2001/XMLSchema#integer";
    let subject = format!("<http://example.test/person/{id}>");
    format!("{subject} <http://example.test/age> \"{age}\"^^<{integer}> .")
}

fn expected_answer(form: Form, era: Era) -> Vec<String> {
    if matches!(form, Form::Ask) {
        return vec![
            format!("observed={}", era == Era::Observed),
            format!("replaced={}", era == Era::Replaced),
        ];
    }
    let rows = match era {
        Era::Observed => [(1, "30"), (2, "25")],
        Era::Replaced => [(1, "360"), (2, "300")],
    };
    let construct = matches!(form, Form::Construct);
    let mut out: Vec<String> = rows
        .iter()
        .map(|(id, age)| {
            if construct {
                triple(*id, age)
            } else {
                (*age).to_owned()
            }
        })
        .collect();
    out.sort();
    out
}

async fn drift_case(journal: &str, form: Form) {
    let mut fixture = Fixture::new(journal);
    fixture.opts.sqlite_pool_size = POOL;
    if form.is_lineage() {
        // Finite retained allowance for lineage forms only: 1 MiB is above the
        // 294912 bytes `lineage::prepare` can charge (32768 fixed metadata plus
        // 262144 for multi mapping). The parent's 4096 stays for other forms.
        fixture.opts.max_order_bytes = 1 << 20;
    }
    let config = fixture.config().await;
    let expected = sealed_generation(&config);
    let identity = runtime_identity(&config);
    let readiness = ready_state(&config);

    let observed = expected_answer(form, Era::Observed);
    for attempt in 0..ATTEMPTS {
        let context = format!("{form:?} pre-drift attempt {attempt}");
        let actual = answer(&config, form).await;
        assert_reply(&actual, StatusCode::OK, &observed, &context);
    }
    assert_readiness(&config, readiness, "before drift").await;

    replace_with_semantically_different_table(&fixture.writer);

    for attempt in 0..ATTEMPTS {
        let context = format!("{form:?} post-drift attempt {attempt}");
        let actual = answer(&config, form).await;
        assert_refused(&actual, &context);
    }
    let (query, accept) = form.probe();
    let (status, body) = call(&config, query, accept).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let root = fixture.root.to_str().unwrap();
    let leaked = String::from_utf8_lossy(&body).contains(root);
    assert!(!leaked, "problem body must not reveal the source path");

    let started = Instant::now();
    for _ in 0..ATTEMPTS {
        let refused = expected.acquire(&budget(Duration::from_secs(30))).await;
        assert!(
            refused.is_err(),
            "every sealed member must refuse the replaced table"
        );
    }
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "refusals must release members promptly"
    );
    assert_eq!(
        runtime_identity(&config),
        identity,
        "runtime snapshot must stay unchanged by request-scoped refusals"
    );
    assert_readiness(&config, readiness, "after request-scoped refusals").await;

    let rebuilt = fixture.config().await;
    let rebuilt_readiness = ready_state(&rebuilt);
    let recovered = expected_answer(form, Era::Replaced);
    for attempt in 0..ATTEMPTS {
        let context = format!("{form:?} rebuilt attempt {attempt}");
        let actual = answer(&rebuilt, form).await;
        assert_reply(&actual, StatusCode::OK, &recovered, &context);
    }
    let context = format!("{form:?} old runtime after an explicit rebuild");
    let actual = answer(&config, form).await;
    assert_refused(&actual, &context);
    assert_eq!(runtime_identity(&config), identity);
    assert_readiness(&config, readiness, "old runtime after an explicit rebuild").await;
    assert_readiness(&rebuilt, rebuilt_readiness, "rebuilt runtime").await;
}

/// Explicit control: the parent's 4096-byte retained allowance is below the
/// fixed lineage metadata charge, so lineage must refuse by budget there.
async fn lineage_refusal_control(journal: &str) {
    let mut fixture = Fixture::new(journal);
    fixture.opts.sqlite_pool_size = POOL;
    assert_eq!(fixture.opts.max_order_bytes, 4096);
    let config = fixture.config().await;
    for form in [Form::LineageSelect, Form::LineageConstruct] {
        let (query, accept) = form.probe();
        let (status, body) = call(&config, query, accept).await;
        assert_eq!(
            status,
            StatusCode::TOO_MANY_REQUESTS,
            "{form:?} must refuse under the 4096 retained allowance; body: {}",
            String::from_utf8_lossy(&body)
        );
    }
}

async fn pool_case(journal: &str) {
    let mut fixture = Fixture::new(journal);
    fixture.opts.sqlite_pool_size = POOL;
    let config = fixture.config().await;
    let expected = sealed_generation(&config);

    // Hold every member: a further lease cannot be granted within its deadline.
    let first = expected.acquire(&budget(HOLD)).await.unwrap();
    let second = expected.acquire(&budget(HOLD)).await.unwrap();
    let third = expected.acquire(&budget(Duration::from_millis(300))).await;
    assert!(
        third.is_err(),
        "two held leases must exhaust the pool of two"
    );
    first.finish().await.unwrap();

    // One member stays held: repeated requests must reach both the free member
    // (answered) and the held one (refused at the request deadline).
    let mut statuses = Vec::new();
    for _ in 0..POOL * 2 {
        let (status, body) = call(&config, ASK_OBSERVED, JSON).await;
        if status == StatusCode::OK {
            let json: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(json["boolean"], true);
        }
        statuses.push(status);
    }
    let served = statuses
        .iter()
        .filter(|status| **status == StatusCode::OK)
        .count();
    assert!(served > 0, "the free member must serve: {statuses:?}");
    assert!(
        served < statuses.len(),
        "the held member must be selected and refuse: {statuses:?}"
    );

    second.finish().await.unwrap();
    let observed = expected_answer(Form::Ask, Era::Observed);
    for attempt in 0..POOL * 2 {
        let context = format!("released attempt {attempt}");
        let actual = answer(&config, Form::Ask).await;
        assert_reply(&actual, StatusCode::OK, &observed, &context);
    }
}

#[tokio::test]
async fn wal_ask_drift_is_request_scoped_and_rebuild_recovers() {
    drift_case("WAL", Form::Ask).await;
}

#[tokio::test]
async fn wal_construct_drift_is_request_scoped_and_rebuild_recovers() {
    drift_case("WAL", Form::Construct).await;
}

#[tokio::test]
async fn wal_lineage_select_drift_is_request_scoped_and_rebuild_recovers() {
    drift_case("WAL", Form::LineageSelect).await;
}

#[tokio::test]
async fn wal_lineage_construct_drift_is_request_scoped_and_rebuild_recovers() {
    drift_case("WAL", Form::LineageConstruct).await;
}

#[tokio::test]
async fn delete_ask_drift_is_request_scoped_and_rebuild_recovers() {
    drift_case("DELETE", Form::Ask).await;
}

#[tokio::test]
async fn delete_construct_drift_is_request_scoped_and_rebuild_recovers() {
    drift_case("DELETE", Form::Construct).await;
}

#[tokio::test]
async fn delete_lineage_select_drift_is_request_scoped_and_rebuild_recovers() {
    drift_case("DELETE", Form::LineageSelect).await;
}

#[tokio::test]
async fn delete_lineage_construct_drift_is_request_scoped_and_rebuild_recovers() {
    drift_case("DELETE", Form::LineageConstruct).await;
}

#[tokio::test]
async fn wal_lineage_with_the_parent_retained_allowance_is_refused_by_budget() {
    lineage_refusal_control("WAL").await;
}

#[tokio::test]
async fn delete_lineage_with_the_parent_retained_allowance_is_refused_by_budget() {
    lineage_refusal_control("DELETE").await;
}

#[tokio::test]
async fn wal_pool_size_two_selects_every_member_and_recovers() {
    pool_case("WAL").await;
}

#[tokio::test]
async fn delete_pool_size_two_selects_every_member_and_recovers() {
    pool_case("DELETE").await;
}
