//! Shared fixtures for the generated-query HTTP profile tests. Configs are built
//! with `ServeConfig::new`, which installs the explicitly named unit-test-only
//! in-process parser fixture; the isolated-parser regression suites stay separate.

use std::future::Future;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use http_body_util::BodyExt;
use sf_core::{SourceId, SourceMapping};
use tower::ServiceExt;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

use crate::{
    Backend, IntrospectedSource, QueryAdmission, QueryShapeProfile, RuntimeSource, ServeConfig,
    SqlitePool,
};

pub(crate) const HEADER: &str = "x-semantic-fabric-profile";
pub(crate) const SELECT: &str = "SELECT ?n WHERE { ?p a <http://ex/Person> ; <http://ex/name> ?n }";
pub(crate) const ASK: &str = "ASK { ?p <http://ex/name> ?n }";
pub(crate) const UNKNOWN_SELECT: &str = "SELECT ?n WHERE { ?p <http://ex/unknown> ?n }";
pub(crate) const CONSTRUCT: &str =
    "CONSTRUCT { ?p <http://ex/name> ?n } WHERE { ?p <http://ex/name> ?n }";

pub(crate) const MAPPING: &str = "@prefix rr: <http://www.w3.org/ns/r2rml#> . \
    <#p> a rr:TriplesMap; rr:logicalTable [ rr:tableName 'people' ]; \
    rr:subjectMap [ rr:template 'http://ex/person/{id}'; rr:class <http://ex/Person> ]; \
    rr:predicateObjectMap [ rr:predicate <http://ex/name>; \
    rr:objectMap [ rr:column 'name' ] ] .";

pub(crate) fn source() -> (IntrospectedSource, SqlitePool) {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE people(id INTEGER, name TEXT, tenant TEXT); \
             INSERT INTO people VALUES (1, 'Alice', 'a'), (2, 'Bob', 'b');",
        )
        .unwrap();
    let backend = Backend::sqlite(connection);
    let Backend::Sqlite(pool) = &backend else {
        unreachable!()
    };
    let pool = pool.clone();
    (IntrospectedSource::observe_sqlite(backend).unwrap(), pool)
}

pub(crate) fn mapping(index: usize) -> SourceMapping {
    sf_mapping::parse_r2rml_for_source(MAPPING, SourceId::new(index).unwrap()).unwrap()
}

pub(crate) fn ontology(extra: &[&str]) -> crate::SemanticOntology {
    let mut properties = vec!["http://ex/name"];
    properties.extend_from_slice(extra);
    crate::test_support::ontology(&["http://ex/Person"], &properties)
}

pub(crate) fn open() -> QueryAdmission {
    QueryAdmission::UnrestrictedDevelopment
}

pub(crate) fn build(
    profile: QueryShapeProfile,
    admission: QueryAdmission,
    tweak: impl FnOnce(&mut ServeConfig),
) -> (Arc<ServeConfig>, SqlitePool) {
    let (observed, pool) = source();
    let mut cfg = ServeConfig::new(observed, mapping(0), ontology(&[])).unwrap();
    cfg.set_query_admission(admission);
    cfg.set_query_shape_profile(profile);
    tweak(&mut cfg);
    (Arc::new(cfg), pool)
}

pub(crate) fn config(
    profile: QueryShapeProfile,
    admission: QueryAdmission,
) -> (Arc<ServeConfig>, SqlitePool) {
    build(profile, admission, |_| {})
}

pub(crate) fn federated(profile: QueryShapeProfile) -> Arc<ServeConfig> {
    let sources = [0_usize, 1].map(|index| {
        let (observed, _) = source();
        RuntimeSource::new(observed, mapping(index))
    });
    let mut cfg = ServeConfig::new_federated(sources, ontology(&[])).unwrap();
    cfg.set_query_admission(open());
    cfg.set_query_shape_profile(profile);
    Arc::new(cfg)
}

pub(crate) fn post(
    query: &str,
    token: Option<&str>,
    accept: Option<&str>,
    extra: &[(&str, &str)],
) -> Request<Body> {
    let mut builder =
        Request::post("/sparql").header(header::CONTENT_TYPE, "application/sparql-query");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(accept) = accept {
        builder = builder.header(header::ACCEPT, accept);
    }
    for (name, value) in extra {
        builder = builder.header(*name, *value);
    }
    builder.body(Body::from(query.to_owned())).unwrap()
}

pub(crate) struct Reply {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
}

impl Reply {
    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap()
    }

    pub(crate) fn identity(&self) -> Option<String> {
        self.headers
            .get(HEADER)
            .map(|value| value.to_str().unwrap().to_owned())
    }
}

pub(crate) async fn send(cfg: &Arc<ServeConfig>, request: Request<Body>) -> Reply {
    let response = crate::router(cfg.clone()).oneshot(request).await.unwrap();
    let (parts, body) = response.into_parts();
    let body = body.collect().await.unwrap().to_bytes().to_vec();
    Reply {
        status: parts.status,
        headers: parts.headers,
        body,
    }
}

pub(crate) fn assert_wire(wire: &str) {
    assert!(wire.starts_with("sfgp1:"), "{wire}");
    assert_eq!(wire.len(), 70, "{wire}");
    let lower_hex = |byte: u8| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte);
    assert!(wire[6..].bytes().all(lower_hex), "{wire}");
}

/// A typed redacted generated-profile refusal: 501 with a citable rule, no
/// identity, finalized problem headers and no query/IRI/mapping disclosure.
pub(crate) fn assert_refusal(reply: &Reply, rule: &str) {
    assert_eq!(reply.status, StatusCode::NOT_IMPLEMENTED, "{rule}");
    assert!(reply.identity().is_none(), "{rule}");
    let content_type = reply.headers.get(header::CONTENT_TYPE).unwrap();
    assert_eq!(content_type, "application/problem+json");
    let nosniff = reply.headers.get("x-content-type-options").unwrap();
    assert_eq!(nosniff, "nosniff");
    assert!(reply.headers.contains_key("x-correlation-id"));
    let length = reply.headers.get(header::CONTENT_LENGTH).unwrap();
    assert_eq!(length.to_str().unwrap(), reply.body.len().to_string());
    let json = reply.json();
    assert_eq!(json["code"], "unsupported-query", "{rule}");
    assert_eq!(json["rule"], rule);
    let text = reply.text();
    for leak in ["http://ex", "Person", "unknown", "people"] {
        assert!(!text.contains(leak), "{rule}: {text}");
    }
}

/// Compiler parse-stage spans observed by the child's global subscriber.
static PARSES: AtomicUsize = AtomicUsize::new(0);

struct StageVisitor(bool);

impl Visit for StageVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "stage" && value == "parse" {
            self.0 = true;
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "stage" {
            let text = format!("{value:?}");
            if text == "parse" || text == "\"parse\"" {
                self.0 = true;
            }
        }
    }
}

/// Counts only `stage = "parse"` spans; spans run on blocking compiler workers,
/// so a thread-local default dispatcher cannot observe them.
struct ParseCounter {
    next: AtomicU64,
}

impl Subscriber for ParseCounter {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, span: &Attributes<'_>) -> Id {
        let mut stage = StageVisitor(false);
        span.record(&mut stage);
        if stage.0 {
            PARSES.fetch_add(1, Ordering::SeqCst);
        }
        Id::from_u64(self.next.fetch_add(1, Ordering::Relaxed))
    }

    fn record(&self, _: &Id, values: &Record<'_>) {
        let mut stage = StageVisitor(false);
        values.record(&mut stage);
        if stage.0 {
            PARSES.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, _: &Event<'_>) {}

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}

pub(crate) fn reset_parse_count() {
    PARSES.store(0, Ordering::SeqCst);
}

pub(crate) fn parse_count() -> usize {
    PARSES.load(Ordering::SeqCst)
}

/// Route one request and return the compiler parses it performed.
pub(crate) async fn counted(cfg: &Arc<ServeConfig>, request: Request<Body>) -> (Reply, usize) {
    reset_parse_count();
    let reply = send(cfg, request).await;
    (reply, parse_count())
}

/// Run `test` alone in a child test process that owns the global tracing
/// subscriber, so no concurrently running test can add or hide a parse span.
pub(crate) fn isolated(test: impl FnOnce()) {
    const CHILD: &str = "SF_GENERATED_HTTP_PARSE_CHILD";
    const COMPLETED: i32 = 77;
    let thread = std::thread::current();
    let name = thread.name().expect("named Rust test thread").to_owned();
    if std::env::var(CHILD).as_deref() == Ok(name.as_str()) {
        let counter = ParseCounter {
            next: AtomicU64::new(1),
        };
        tracing::subscriber::set_global_default(counter)
            .expect("the child owns the global subscriber");
        test();
        std::process::exit(COMPLETED);
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name.as_str(), "--nocapture", "--test-threads=1"])
        .env(CHILD, &name)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(
                status.code(),
                Some(COMPLETED),
                "child must execute all assertions; zero matching tests cannot pass"
            );
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("generated HTTP parse-count child exceeded its bound");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(crate) fn isolated_async<F, Fut>(test: F)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    isolated(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        runtime.block_on(test());
    });
}
