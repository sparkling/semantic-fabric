//! Independent two-source fixture for ordinary SQLite startup: isolated
//! file-backed databases, authored mappings, the real startup path, query
//! builders and expected bags. Limits are the finite product defaults.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use sf_core::query_control::QueryLimits;
use tower::ServiceExt;

pub(super) type Config = Arc<crate::ServeConfig>;
pub(super) type Row = (i64, &'static str, &'static str, &'static str);

pub(super) const LEFT: usize = 0;
pub(super) const RIGHT: usize = 1;
pub(super) const EX: &str = "http://example.test/";
pub(super) const ITEM: &str = "http://example.test/item/";
pub(super) const MISSING: &str = "-";
/// The retained-byte ceiling of the first failing fixture.
pub(super) const SMALL_RETAINED: u64 = 4096;
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

pub(super) const INITIAL_LEFT: &[Row] = &[
    (1, "l1", "k1", "alpha"),
    (2, "l2", "k1", "shared"),
    (3, "l3", "k9", "shared"),
];
pub(super) const INITIAL_RIGHT: &[Row] = &[
    (1, "r1", "k1", "shared"),
    (2, "r2", "k1", "delta"),
    (3, "r3", "k8", "echo"),
];
pub(super) const NEW_LEFT: &[Row] = &[
    (1, "l4", "k1", "pi"),
    (2, "l5", "k8", "shared"),
    (3, "l6", "k8", "rho"),
];
pub(super) const NEW_RIGHT: &[Row] = &[
    (1, "r4", "k9", "shared"),
    (2, "r5", "k1", "romeo"),
    (3, "r6", "k9", "sierra"),
];

const SCHEMA: &str = "CREATE TABLE items(pk INTEGER PRIMARY KEY, id TEXT NOT NULL, \
    join_key TEXT NOT NULL, label TEXT NOT NULL);";

/// Same table name and mapped columns, different table: compiled SQL still
/// runs, so only the sealed generation can tell the request it changed.
const DRIFTED: &str = "DROP TABLE items; CREATE TABLE items(pk INTEGER PRIMARY KEY, \
    id TEXT NOT NULL, join_key TEXT NOT NULL, label TEXT NOT NULL, unit TEXT);";

fn ontology() -> String {
    let prefix = "@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n";
    let mut out = String::from(prefix);
    for name in ["label", "left", "right", "leftLabel", "rightLabel"] {
        out.push_str(&format!("<{EX}{name}> a rdf:Property .\n"));
    }
    out
}

fn pom(predicate: &str, column: &str) -> String {
    let object = format!("rr:column \"{column}\" ; rr:datatype <{XSD_STRING}>");
    let head = format!("rr:predicateObjectMap [ rr:predicate <{EX}{predicate}>");
    format!("{head} ; rr:objectMap [ {object} ] ]")
}

/// Each source owns its key and label predicates; both also map the shared
/// `label`, which keeps the original single-pattern control meaningful.
fn mapping(key: &str, label: &str) -> String {
    let poms = [
        pom(key, "join_key"),
        pom(label, "label"),
        pom("label", "label"),
    ];
    let body = poms.join(" ;\n");
    format!(
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> a rr:TriplesMap ; rr:logicalTable [ rr:tableName "items" ] ;
rr:subjectMap [ rr:template "{ITEM}{{id}}" ] ;
{body} ."#
    )
}

fn insert(conn: &rusqlite::Connection, rows: &[Row]) {
    for (pk, id, key, label) in rows {
        conn.execute(
            "INSERT INTO items(pk, id, join_key, label) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![pk, id, key, label],
        )
        .unwrap();
    }
}

/// One isolated directory holding one file-backed SQLite source.
fn seed(dir: &Path, journal: &str, rows: &[Row]) -> (PathBuf, rusqlite::Connection) {
    std::fs::create_dir(dir).unwrap();
    let db = dir.join("source.db");
    let writer = rusqlite::Connection::open(&db).unwrap();
    let pragma = format!("PRAGMA journal_mode={journal};");
    writer.execute_batch(&pragma).unwrap();
    let mode: String = writer
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert!(mode.eq_ignore_ascii_case(journal), "{journal}: {mode}");
    writer.execute_batch(SCHEMA).unwrap();
    insert(&writer, rows);
    (db, writer)
}

pub(super) struct Fixture {
    root: PathBuf,
    writers: [rusqlite::Connection; 2],
    pub(super) opts: crate::run::ServeOptions,
}

impl Fixture {
    pub(super) fn new(journals: [&str; 2]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let name = format!(
            "sf-ordinary-pair-{}-{}-{}",
            std::process::id(),
            since.as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(name);
        std::fs::create_dir(&root).unwrap();
        let (left, lw) = seed(&root.join("left"), journals[LEFT], INITIAL_LEFT);
        let (right, rw) = seed(&root.join("right"), journals[RIGHT], INITIAL_RIGHT);
        let write = |name: &str, text: String| std::fs::write(root.join(name), text).unwrap();
        write("ontology.ttl", ontology());
        write("left.ttl", mapping("left", "leftLabel"));
        write("right.ttl", mapping("right", "rightLabel"));
        let opts = options(&root, &left, &right);
        Self {
            root,
            writers: [lw, rw],
            opts,
        }
    }

    pub(super) fn drift(&self, side: usize, rows: &[Row]) {
        let writer = &self.writers[side];
        writer.execute_batch(DRIFTED).unwrap();
        insert(writer, rows);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Created exclusively by this fixture.
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn sqlite(db: &Path) -> crate::SourceRef {
    crate::SourceRef::inline(format!("sqlite:{}", db.display()))
}

fn r2rml(root: &Path, name: &str) -> crate::MappingRef {
    crate::MappingRef::r2rml_file(root.join(name).to_string_lossy())
}

/// ORDINARY two-source profile: no verified-generation requirement, reload
/// disabled, and every governance limit at its finite product default.
fn options(root: &Path, left: &Path, right: &Path) -> crate::run::ServeOptions {
    let extra = crate::run::AdditionalSourceOptions {
        source: sqlite(right),
        mapping: r2rml(root, "right.ttl"),
    };
    let limits = crate::DEFAULT_QUERY_LIMITS;
    crate::run::ServeOptions {
        query_shape_profile: crate::QueryShapeProfile::Ordinary,
        query_admission: crate::QueryAdmission::UnrestrictedDevelopment,
        source: sqlite(left),
        mapping: r2rml(root, "left.ttl"),
        additional_source: Some(extra),
        ontology_path: root.join("ontology.ttl").to_string_lossy().into_owned(),
        bind: "127.0.0.1:0".into(),
        timeout: Duration::from_secs(5),
        max_query_len: 4096,
        max_concurrent_requests: crate::config::DEFAULT_MAX_CONCURRENT_REQUESTS,
        max_compiler_work: limits.max_compiler_work(),
        max_source_work: limits.max_source_work(),
        max_result_items: limits.max_result_items(),
        max_order_rows: crate::config::DEFAULT_MAX_ORDER_ROWS,
        max_order_bytes: crate::config::DEFAULT_MAX_ORDER_BYTES,
        max_serialized_bytes: limits.max_serialized_bytes(),
        pg_pool_size: 1,
        pg_pool_wait: Duration::from_secs(1),
        sqlite_pool_size: 1,
        shutdown_timeout: Duration::from_secs(1),
        reload_interval: Duration::ZERO,
        require_verified_generation: false,
        metrics: None,
    }
}

/// Tuner changing only the retained-byte ceiling; every other limit stays.
pub(super) fn retained(bytes: u64) -> impl FnOnce(&mut crate::ServeConfig) {
    move |config| config.query_limits = config.query_limits.with_max_retained_bytes(bytes)
}

/// Every limit the first failing fixture set; the negative witness.
pub(super) fn old_fixture_limits(config: &mut crate::ServeConfig) {
    let base = crate::DEFAULT_QUERY_LIMITS;
    let (compiler, source) = (base.max_compiler_work(), base.max_source_work());
    let limits = QueryLimits::new(compiler, source, 1000, 1 << 20);
    config.set_max_order_rows(100);
    config.query_limits = limits.with_max_retained_bytes(SMALL_RETAINED);
}

/// The real ordinary startup path for the two-source profile.
pub(super) async fn build(opts: &crate::run::ServeOptions) -> Config {
    build_tuned(opts, |_| {}).await
}

/// Same startup path; `tune` runs before the config is shared.
pub(super) async fn build_tuned(
    opts: &crate::run::ServeOptions,
    tune: impl FnOnce(&mut crate::ServeConfig),
) -> Config {
    let primary = opts.source.resolve().unwrap().prepare().unwrap();
    let extra = opts.additional_source.as_ref().unwrap();
    let additional = extra.source.resolve().unwrap().prepare().unwrap();
    let built = crate::startup::build_config(opts, primary, Some(additional)).await;
    let (mut config, _) = match built {
        Ok(built) => built,
        Err(error) => panic!("ordinary startup {}: {error:?}", error.code()),
    };
    config.use_in_process_test_parser();
    tune(&mut config);
    Arc::new(config)
}

/// Status and full body; a body stream failure is reported, never unwrapped.
pub(super) async fn ask(config: &Config, query: &str) -> (StatusCode, String) {
    let request = Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, "application/sparql-results+json")
        .body(Body::from(query.to_owned()))
        .unwrap();
    let app = crate::router(config.clone());
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = match response.into_body().collect().await {
        Ok(body) => String::from_utf8_lossy(&body.to_bytes()).into_owned(),
        Err(error) => format!("<body stream failed after {status}: {error}>"),
    };
    (status, body)
}

/// The real `/readyz` route of the built router.
pub(super) async fn readyz(config: &Config) -> (StatusCode, String) {
    let request = Request::builder()
        .uri("/readyz")
        .body(Body::empty())
        .unwrap();
    let app = crate::router(config.clone());
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

fn cell(row: &serde_json::Value, vars: &[&str]) -> String {
    let mut values = Vec::new();
    for var in vars {
        values.push(row[*var]["value"].as_str().unwrap_or(MISSING));
    }
    values.join("|")
}

pub(super) fn sorted(mut rows: Vec<String>) -> Vec<String> {
    rows.sort();
    rows
}

/// The public problem code, or `MISSING` for a non-problem body.
pub(super) fn problem_code(body: &str) -> String {
    let json: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    json["code"].as_str().unwrap_or(MISSING).to_owned()
}

/// The full answer bag, sorted but never deduplicated.
pub(super) fn bag(body: &str, vars: &[&str]) -> Option<Vec<String>> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let rows = json["results"]["bindings"].as_array()?;
    Some(sorted(rows.iter().map(|row| cell(row, vars)).collect()))
}

pub(super) fn iri(id: &str) -> String {
    format!("{ITEM}{id}")
}

pub(super) fn triple(subject: &str, predicate: &str, object: &str) -> String {
    format!("{subject} <{EX}{predicate}> {object}")
}

pub(super) fn select(vars: &str, body: &str) -> String {
    format!("SELECT {vars} WHERE {{ {body} }}")
}

fn either(left: &str, right: &str) -> String {
    format!("{{ {left} }} UNION {{ {right} }}")
}

/// Two explicit source-affine label arms binding the same variables.
pub(super) fn labels(vars: &str) -> String {
    let left = triple("?s", "leftLabel", "?o");
    let right = triple("?s", "rightLabel", "?o");
    select(vars, &either(&left, &right))
}

/// Distinct variables per arm: bound domains prove which source answered.
pub(super) fn affine(left_predicate: &str, right_predicate: &str) -> String {
    let left = triple("?s", left_predicate, "?l");
    let right = triple("?s", right_predicate, "?r");
    select("?s ?l ?r", &either(&left, &right))
}

/// One shared object variable over arbitrary arm predicates.
pub(super) fn same_var(left_predicate: &str, right_predicate: &str) -> String {
    let left = triple("?s", left_predicate, "?o");
    let right = triple("?s", right_predicate, "?o");
    select("?s ?o", &either(&left, &right))
}

pub(super) fn join_query() -> String {
    let left = triple("?left", "left", "?key");
    let right = triple("?right", "right", "?key");
    select("?left ?right", &format!("{left} . {right}"))
}

fn column(row: &Row, labels: bool) -> &'static str {
    [row.2, row.3][usize::from(labels)]
}

pub(super) fn union_expected(left: &[Row], right: &[Row]) -> Vec<String> {
    let mut out = Vec::new();
    for (_, id, _, label) in left.iter().chain(right) {
        out.push(format!("{}|{label}", iri(id)));
    }
    sorted(out)
}

/// Projection keeps one solution per distinct triple, so repeated labels stay.
pub(super) fn labels_expected(left: &[Row], right: &[Row]) -> Vec<String> {
    let mut out = Vec::new();
    for (_, _, _, label) in left.iter().chain(right) {
        out.push((*label).to_owned());
    }
    sorted(out)
}

pub(super) fn affine_expected(left: &[Row], right: &[Row], labels: bool) -> Vec<String> {
    let mut out = Vec::new();
    for row in left {
        let value = column(row, labels);
        out.push(format!("{}|{value}|{MISSING}", iri(row.1)));
    }
    for row in right {
        let value = column(row, labels);
        out.push(format!("{}|{MISSING}|{value}", iri(row.1)));
    }
    sorted(out)
}

pub(super) fn join_expected(left: &[Row], right: &[Row]) -> Vec<String> {
    let mut out = Vec::new();
    for (_, l, left_key, _) in left {
        for (_, r, right_key, _) in right {
            if left_key == right_key {
                out.push(format!("{}|{}", iri(l), iri(r)));
            }
        }
    }
    sorted(out)
}
