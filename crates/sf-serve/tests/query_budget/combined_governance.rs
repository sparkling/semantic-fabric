//! Combined public governance qualification for ordinary and bearer requests.
//!
//! "Ordinary" is the credential-free development admission; "bearer" is the
//! service-principal credential. Both run the same SQLite fixtures through the
//! actual public router.
//!
//! Duplicate RDF answers are exact on one router, cold and then warm, at the
//! unchanged serving defaults: SELECT returns six literal identical bindings
//! and CONSTRUCT, requested as N-Triples, returns the literal template triple
//! once per solution. Streaming CONSTRUCT performs no graph-level duplicate
//! collapse, so six solutions serialize six identical statements. ASK only
//! answers `true`; it is not duplicate evidence.
//!
//! In every refusal fixture only the source-work allowance is finite, so a
//! refusal can only come from the source-work stage. SELECT and CONSTRUCT
//! stream, so their refusal is the accepted post-200 `result stream failed`
//! body error (ADR-0010); ASK buffers and answers the typed
//! `query-budget-exceeded` problem. Recovery then answers exactly on the same
//! `ServeConfig`, hence the same SQLite connection and warm plan cache. A
//! router cannot change its limits, so recovery at a raised allowance uses a
//! new router over that same configuration; same-router recovery is shown with
//! a cheaper query under the unchanged cap.
//!
//! The bearer one-unit pins mirror `reconstruction.rs`, where they were
//! measured cold with bearer admission on a fresh connection; each is asserted
//! only against the first request on its own fresh connection. Recovery at a
//! pin may run warm or after earlier requests on that connection, which never
//! spends more.
//!
//! Ordinary admission has its own literal pins, measured by the public
//! diagnostic and kept as separate cold and warm values: cold is a fresh
//! connection, warm is a fresh connection whose plan cache one admitted answer
//! of the same query has populated. Each pin is a pair: one unit short refuses
//! in the form's shape on one such state, and the exact allowance answers
//! literally on another. No boundary is discovered at run time, so a charge
//! that silently shrinks or disappears fails the exact or one-short assertion.
//!
//! The warm compiler path is proven to be a cache hit: with the same source
//! budget, a finite compiler allowance that cannot compile cold refuses a cold
//! connection and admits the warm one over the same configuration and cache.
//!
//! This is combined evidence only, not blanket G1/G5 closure and not native
//! PostgreSQL or MySQL qualification.

use sf_serve::RequestDeadlineService;

use super::*;

const DUPLICATE_MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#Edges> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "edges" ] ;
  rr:subjectMap [ rr:template "http://example.test/node/{s}" ] ;
  rr:predicateObjectMap [ rr:predicate ex:a, ex:b, ex:c, ex:d, ex:e, ex:f ;
    rr:objectMap [ rr:template "http://example.test/node/{o}" ] ] .
"#;

// One source row, six mapped predicates: the negated path yields six
// solutions with the identical (node/1, node/2) projection.
const DUPLICATE_SELECT: &str = "SELECT ?s ?o WHERE { ?s !<urn:absent> ?o }";
const DUPLICATE_ASK: &str = "ASK { ?s !<urn:absent> ?o }";
const DUPLICATE_CONSTRUCT: &str = "CONSTRUCT { ?s <http://example.test/a> ?o } \
     WHERE { ?s !<urn:absent> ?o }";
const DUPLICATE_TRIPLE: &str =
    "<http://example.test/node/1> <http://example.test/a> <http://example.test/node/2>";

const SELECT_ROWS: &str = "SELECT ?s ?value WHERE { ?s <http://example.test/value> ?value }";
const ASK_ROWS: &str = "ASK { ?s <http://example.test/value> ?value }";
const CONSTRUCT_ROWS: &str = "CONSTRUCT { ?s <http://example.test/value> ?value } \
     WHERE { ?s <http://example.test/value> ?value }";
const ROWS: [&str; 3] = [SELECT_ROWS, ASK_ROWS, CONSTRUCT_ROWS];
const ROW_TRIPLES: [&str; 2] = [
    "<http://example.test/item/1> <http://example.test/value> \"one\"",
    "<http://example.test/item/2> <http://example.test/value> \"two\"",
];

// Exact bearer source-work totals, mirroring the pins in reconstruction.rs.
const SELECT_EXACT: u64 = 27_985;
const ASK_EXACT: u64 = 27_840;
const CONSTRUCT_EXACT: u64 = 27_985;

// Measured ordinary source-work boundaries: (query, cold, warm).
const ORDINARY_PINS: [(&str, u64, u64); 3] = [
    (SELECT_ROWS, 27_985, 27_985),
    (ASK_ROWS, 27_840, 27_840),
    (CONSTRUCT_ROWS, 27_985, 27_985),
];

const N_TRIPLES: &str = "application/n-triples";

fn with_admission(mut cfg: ServeConfig, bearer: bool) -> ServeConfig {
    if bearer {
        cfg.set_query_admission(QueryAdmission::Bearer(
            BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
        ));
    }
    cfg
}

/// CONSTRUCT asks for line-oriented N-Triples so its graph can be read exactly.
fn send_request(bearer: bool, query: &str) -> Request<Body> {
    let mut req = if bearer {
        authenticated(query)
    } else {
        request(query)
    };
    if query.starts_with("CONSTRUCT") {
        req.headers_mut()
            .insert(header::ACCEPT, N_TRIPLES.parse().unwrap());
    }
    req
}

async fn send(app: &RequestDeadlineService, bearer: bool, query: &str) -> axum::response::Response {
    app.clone()
        .oneshot(send_request(bearer, query))
        .await
        .unwrap()
}

async fn ok_bytes(response: axum::response::Response) -> axum::body::Bytes {
    assert_eq!(response.status(), StatusCode::OK);
    let collected = response.into_body().collect().await;
    collected.expect("complete body").to_bytes()
}

async fn ok_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = ok_bytes(response).await;
    serde_json::from_slice(&bytes).unwrap()
}

/// Every statement of a complete N-Triples body, in serialized order, as
/// `subject predicate object` without the terminating dot. Any line that is
/// not one such statement fails with the whole body.
async fn ok_triples(response: axum::response::Response) -> Vec<String> {
    let content_type = response.headers()[header::CONTENT_TYPE].clone();
    let bytes = ok_bytes(response).await;
    let body = std::str::from_utf8(&bytes).unwrap();
    let media = content_type.to_str().unwrap();
    assert!(media.starts_with(N_TRIPLES), "{media}: {body}");
    let mut triples = Vec::new();
    for line in body.lines().filter(|line| !line.is_empty()) {
        let statement = line.strip_suffix(" .").expect(body);
        let terms: Vec<&str> = statement.split_whitespace().collect();
        assert_eq!(terms.len(), 3, "{body}");
        triples.push(terms.join(" "));
    }
    triples
}

async fn assert_stream_failure(response: axum::response::Response) {
    assert_eq!(response.status(), StatusCode::OK);
    let error = response.into_body().collect().await.unwrap_err();
    assert_eq!(error.to_string(), "result stream failed");
}

fn serving_defaults() -> QueryLimits {
    QueryLimits::new(1_000_000, 1_000_000, 100_000, 64 * 1024 * 1024)
        .with_max_retained_bytes(64 * 1024 * 1024)
}

fn source_only(source_work: u64) -> QueryLimits {
    QueryLimits::new(u64::MAX, source_work, u64::MAX, u64::MAX)
}

fn governed(bearer: bool, source_work: u64) -> ServeConfig {
    with_admission(config(source_only(source_work)), bearer)
}

/// Raise the limits of the same configuration, and so the same SQLite
/// connection and plan cache, once every router over it has been dropped.
fn relimit(cfg: &mut Arc<ServeConfig>, limits: QueryLimits) {
    let cfg = Arc::get_mut(cfg).expect("config is unshared");
    cfg.query_limits = limits;
}

fn duplicate_config(bearer: bool) -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE edges (s INTEGER, o INTEGER); INSERT INTO edges VALUES (1, 2);",
    )
    .unwrap();
    let cfg = support::serve_config(Backend::sqlite(conn), DUPLICATE_MAPPING);
    with_admission(cfg, bearer)
}

fn duplicate_pairs() -> serde_json::Value {
    let pair = serde_json::json!({
        "s": {"type": "uri", "value": "http://example.test/node/1"},
        "o": {"type": "uri", "value": "http://example.test/node/2"}
    });
    serde_json::json!(vec![pair; 6])
}

async fn assert_duplicate_round(app: &RequestDeadlineService, bearer: bool) {
    let select = ok_json(send(app, bearer, DUPLICATE_SELECT).await).await;
    assert_eq!(select["results"]["bindings"], duplicate_pairs());
    let ask = ok_json(send(app, bearer, DUPLICATE_ASK).await).await;
    assert_eq!(ask["boolean"], true);
    let graph = ok_triples(send(app, bearer, DUPLICATE_CONSTRUCT).await).await;
    assert_eq!(graph, [DUPLICATE_TRIPLE; 6], "bearer={bearer}");
}

#[tokio::test]
async fn duplicate_answers_are_exact_cold_and_warm_at_default_limits() {
    for bearer in [false, true] {
        let cfg = duplicate_config(bearer);
        assert_eq!(cfg.query_limits, serving_defaults());
        let app = router(Arc::new(cfg));
        // Cold, then warm on the same router and plan cache.
        assert_duplicate_round(&app, bearer).await;
        assert_duplicate_round(&app, bearer).await;
    }
}

fn assert_rows_select(json: &serde_json::Value) {
    let rows = json["results"]["bindings"].as_array().unwrap();
    let mut pairs: Vec<(&str, &str)> = Vec::new();
    for row in rows {
        assert_eq!(row["s"]["type"], "uri");
        assert_eq!(row["value"]["type"], "literal");
        let subject = row["s"]["value"].as_str().unwrap();
        let value = row["value"]["value"].as_str().unwrap();
        pairs.push((subject, value));
    }
    pairs.sort();
    assert_eq!(
        pairs,
        [
            ("http://example.test/item/1", "one"),
            ("http://example.test/item/2", "two"),
        ]
    );
}

/// The exact literal answer of one of the two-row fixture's queries.
async fn assert_rows_answer(app: &RequestDeadlineService, bearer: bool, query: &str) {
    let response = send(app, bearer, query).await;
    if query == CONSTRUCT_ROWS {
        let mut graph = ok_triples(response).await;
        graph.sort();
        assert_eq!(graph, ROW_TRIPLES);
        return;
    }
    let json = ok_json(response).await;
    if query == ASK_ROWS {
        assert_eq!(json["boolean"], true);
    } else {
        assert_rows_select(&json);
    }
}

/// The form's accepted source-work refusal: a typed problem for buffered ASK,
/// the stable post-200 stream failure for streaming SELECT and CONSTRUCT.
async fn assert_refused(app: &RequestDeadlineService, bearer: bool, query: &str) {
    let response = send(app, bearer, query).await;
    if query == ASK_ROWS {
        assert_budget_problem(response).await;
    } else {
        assert_stream_failure(response).await;
    }
}

#[tokio::test]
async fn zero_source_allowance_refuses_every_form_then_the_same_connection_recovers() {
    for bearer in [false, true] {
        let mut cfg = Arc::new(governed(bearer, 0));
        let app = router(cfg.clone());
        // Cold, then warm: the refused compiles populate the plan cache.
        for _ in 0..2 {
            for query in ROWS {
                assert_refused(&app, bearer, query).await;
            }
        }
        drop(app);
        relimit(&mut cfg, serving_defaults());
        let app = router(cfg.clone());
        for _ in 0..2 {
            for query in ROWS {
                assert_rows_answer(&app, bearer, query).await;
            }
        }
    }
}

#[tokio::test]
async fn bearer_pins_refuse_one_unit_short_then_recover_on_the_same_connection() {
    for (query, exact) in [
        (SELECT_ROWS, SELECT_EXACT),
        (ASK_ROWS, ASK_EXACT),
        (CONSTRUCT_ROWS, CONSTRUCT_EXACT),
    ] {
        let mut cfg = Arc::new(governed(true, exact - 1));
        let app = router(cfg.clone());
        // The pin was measured on a cold first request, exactly this one.
        assert_refused(&app, true, query).await;
        drop(app);
        relimit(&mut cfg, source_only(exact));
        let app = router(cfg.clone());
        for _ in 0..2 {
            assert_rows_answer(&app, true, query).await;
        }
    }
}

#[tokio::test]
async fn one_short_streams_refuse_and_the_same_router_still_answers_ask() {
    for (query, exact) in [
        (SELECT_ROWS, SELECT_EXACT),
        (CONSTRUCT_ROWS, CONSTRUCT_EXACT),
    ] {
        let app = router(Arc::new(governed(true, exact - 1)));
        // The first request on this fresh connection, as its pin was measured.
        assert_refused(&app, true, query).await;
        // ASK's pin is 144 units below this cap; its warm repeat never spends more.
        for _ in 0..2 {
            assert_rows_answer(&app, true, ASK_ROWS).await;
        }
    }
}

/// A new ordinary connection capped at `source_work`, with its router. Cold
/// is untouched; warm first answers `query` once without a source cap, then
/// applies the cap to that same configuration, connection and plan cache.
async fn ordinary_state(
    query: &str,
    warm: bool,
    source_work: u64,
) -> (Arc<ServeConfig>, RequestDeadlineService) {
    if !warm {
        let cfg = Arc::new(governed(false, source_work));
        return (cfg.clone(), router(cfg));
    }
    let mut cfg = Arc::new(governed(false, u64::MAX));
    let app = router(cfg.clone());
    assert_rows_answer(&app, false, query).await;
    drop(app);
    relimit(&mut cfg, source_only(source_work));
    (cfg.clone(), router(cfg))
}

/// One unit short refuses in the form's shape, and the exact allowance answers
/// literally on a separate, equivalently prepared connection.
async fn assert_ordinary_pair(query: &str, warm: bool, exact: u64) {
    let (mut cfg, app) = ordinary_state(query, warm, exact - 1).await;
    assert_refused(&app, false, query).await;
    // Full recovery needs the raised allowance: a new router over this same
    // configuration and connection, not a raised limit on the refused router.
    drop(app);
    relimit(&mut cfg, source_only(exact));
    let app = router(cfg.clone());
    assert_rows_answer(&app, false, query).await;

    let (_cfg, app) = ordinary_state(query, warm, exact).await;
    assert_rows_answer(&app, false, query).await;
}

#[tokio::test]
async fn ordinary_forms_refuse_one_unit_short_cold_and_warm_then_answer_exactly() {
    for (query, cold, warm) in ORDINARY_PINS {
        assert_ordinary_pair(query, false, cold).await;
        assert_ordinary_pair(query, true, warm).await;
    }
}

#[tokio::test]
async fn ordinary_one_short_streams_refuse_and_the_same_router_still_answers_ask() {
    let ask = ORDINARY_PINS[1].1;
    for (query, cold, _) in [ORDINARY_PINS[0], ORDINARY_PINS[2]] {
        assert!(
            ask < cold,
            "{query}: cap {} cannot admit ASK {ask}",
            cold - 1
        );
        let (_cfg, app) = ordinary_state(query, false, cold - 1).await;
        // The first request on this fresh connection, as its pin was measured.
        assert_refused(&app, false, query).await;
        // Same router, unchanged cap: ASK's cold total fits, its repeat never spends more.
        for _ in 0..2 {
            assert_rows_answer(&app, false, ASK_ROWS).await;
        }
    }
}

#[tokio::test]
async fn ordinary_warm_compiler_path_is_a_cache_hit_under_a_cold_refusing_allowance() {
    let maps = sf_mapping::parse_r2rml(MAPPING).unwrap();
    for (query, _, warm) in ORDINARY_PINS {
        let len = query.len() as u64;
        let paid = cache_key::warm_work(query) + cache_key::admission_work(query, &maps);
        let compiler = len + paid;
        assert!(
            compiler < len + cache_key::fixture_compile_work(query),
            "{query}: compiler allowance {compiler} could compile cold"
        );
        let limits = QueryLimits::new(compiler, warm, u64::MAX, u64::MAX);

        // Cold: the same limits refuse before any source work is spent.
        let app = router(Arc::new(config(limits)));
        assert_budget_problem(send(&app, false, query).await).await;

        // Warm: the same configuration and plan cache, primed once unlimited,
        // then admitted under exactly those limits.
        let mut cfg = Arc::new(governed(false, u64::MAX));
        let app = router(cfg.clone());
        assert_rows_answer(&app, false, query).await;
        drop(app);
        relimit(&mut cfg, limits);
        let app = router(cfg.clone());
        assert_rows_answer(&app, false, query).await;
    }
}
