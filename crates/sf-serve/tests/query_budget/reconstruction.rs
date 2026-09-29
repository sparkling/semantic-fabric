//! Public receipts for per-row RDF reconstruction under a request's
//! source-work budget.
//!
//! Row decoding is charged by the backends; this module covers what happens
//! after a row is decoded — building each solution's terms. A source budget
//! that stops short of the reconstruction a query needs must stop that request,
//! and the same query must then answer exactly, at unchanged defaults, once the
//! budget admits it. Refusal and recovery are both required: a refusal alone
//! could come from any earlier stage, and a success alone proves nothing about
//! the cap.
//!
//! **How a refusal is observable depends on the form, and this module asserts
//! the existing contract rather than changing it (ADR-0010).** `ASK` buffers its
//! single answer, so an exhausted budget surfaces as the ordinary
//! `query-budget-exceeded` problem response. `SELECT` and `CONSTRUCT` stream:
//! the `200` status and headers are committed before any row is reconstructed,
//! so a budget exhausted *during* reconstruction can only appear as the stable
//! `result stream failed` body-stream error after that status. That post-200
//! signalling limitation is ADR-0010's accepted position; these tests pin the
//! behavior that exists.

use super::*;

/// The fixture's two rows, reconstructed through an `rr:template` subject and
/// an `rr:column` object, so each solution performs real template expansion,
/// percent-encoding and literal building rather than copying a constant.
const SELECT_TERMS: &str = "SELECT ?s ?value WHERE { ?s <http://example.test/value> ?value }";
const ASK_TERMS: &str = "ASK { ?s <http://example.test/value> ?value }";
const CONSTRUCT_TERMS: &str = "CONSTRUCT { ?s <http://example.test/value> ?value } \
     WHERE { ?s <http://example.test/value> ?value }";

/// A config whose compiler allowance is unbounded and whose *source* allowance
/// is `source_work` — isolating the budget this slice governs.
fn source_capped(source_work: u64) -> ServeConfig {
    let mut cfg = config(QueryLimits::new(u64::MAX, source_work, u64::MAX, u64::MAX));
    cfg.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    cfg
}

/// Did this request deliver a complete body? `false` covers both observable
/// refusal shapes: a pre-status problem response, and a streaming form's
/// post-status body failure.
async fn completes(query: &str, source_work: u64) -> bool {
    let response = router(Arc::new(source_capped(source_work)))
        .oneshot(authenticated(query))
        .await
        .unwrap();
    if response.status() != StatusCode::OK {
        return false;
    }
    response.into_body().collect().await.is_ok()
}

/// The exact source-work totals these three queries spend, PINNED.
///
/// A bisection that simply finds "whatever the minimum is" would pass whether
/// or not per-row reconstruction is charged at all, so these are fixed numbers,
/// measured with the real charge in place and then re-measured with the
/// reconstruction call site's control temporarily disconnected
/// (`TermWork::uncontrolled()`): SELECT/CONSTRUCT fell 27_985 -> 27_857, 64
/// units per reconstructed row. Each assertion below therefore fails if that
/// specific call site stops charging the caller's control, rather than merely
/// proving that some source work is charged somewhere in a query.
const SELECT_EXACT_SOURCE_WORK: u64 = 27_985;
/// ASK's exact total. ASK stops after its first solution, and the executor
/// opens its SQLite owned-worker branch with the early-stop demand hint, so the
/// worker decodes only the one row ASK pulls. Before that hint the worker also
/// decoded and billed the second row speculatively, racing ASK's return (the
/// old 27_919 ceiling with a 16-unit race window). That second row's decode is
/// 79 units (`sf-sql` `backend/sqlite/decode.rs`): value and code slot vectors
/// `2 x (1 + 24)` and `2 x (1 + 1)`, the `id` cell `1 + 20`, and the `'two'`
/// cell `1 + 3`; so ASK now spends 27_919 - 79. SELECT agrees: 27_985 less 2
/// (SELECT's second-row and EOF pulls), 64 (its second row's reconstruction)
/// and 79. Disconnecting reconstruction's charge drops ASK by its one row's 64
/// units, far beyond the one-unit boundary asserted here.
const ASK_EXACT_SOURCE_WORK: u64 = 27_840;
const CONSTRUCT_EXACT_SOURCE_WORK: u64 = 27_985;

/// Assert the pinned total really is this query's boundary: it completes at
/// `exact` and does not at `exact - 1`.
async fn assert_exact_boundary(query: &str, exact: u64) {
    assert!(
        completes(query, exact).await,
        "{query} must complete at its pinned exact source-work total {exact}"
    );
    assert!(
        !completes(query, exact - 1).await,
        "{query} must NOT complete one unit below its pinned total {exact} — \
         if it does, per-row reconstruction is charging less than when this \
         number was measured"
    );
}

async fn body(query: &str, source_work: u64) -> axum::body::Bytes {
    let response = router(Arc::new(source_capped(source_work)))
        .oneshot(authenticated(query))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{query}");
    response
        .into_body()
        .collect()
        .await
        .expect("the admitted budget streams a complete body")
        .to_bytes()
}

/// A streaming form one unit short of its reconstruction cost fails its body
/// stream (ADR-0010's post-200 limitation), and answers exactly at the cost.
#[tokio::test]
async fn select_fails_its_stream_one_unit_short_then_answers_exactly() {
    let exact = SELECT_EXACT_SOURCE_WORK;
    assert_exact_boundary(SELECT_TERMS, exact).await;
    let short = router(Arc::new(source_capped(exact - 1)))
        .oneshot(authenticated(SELECT_TERMS))
        .await
        .unwrap();
    assert_eq!(
        short.status(),
        StatusCode::OK,
        "the status is committed before rows reconstruct (ADR-0010)"
    );
    assert!(
        short.into_body().collect().await.is_err(),
        "one unit short of reconstruction must not deliver a complete body"
    );

    let json: serde_json::Value = serde_json::from_slice(&body(SELECT_TERMS, exact).await).unwrap();
    let rows = json["results"]["bindings"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let mut pairs: Vec<(&str, &str)> = rows
        .iter()
        .map(|row| {
            assert_eq!(row["s"]["type"], "uri");
            assert_eq!(row["value"]["type"], "literal");
            (
                row["s"]["value"].as_str().unwrap(),
                row["value"]["value"].as_str().unwrap(),
            )
        })
        .collect();
    pairs.sort();
    assert_eq!(
        pairs,
        [
            ("http://example.test/item/1", "one"),
            ("http://example.test/item/2", "two"),
        ]
    );
}

/// `ASK` buffers, so its refusal is the ordinary typed problem response. Its
/// total is exact because only demanded rows are decoded; the boundary repeats
/// so a reintroduced prefetch race cannot pass by scheduling luck.
#[tokio::test]
async fn ask_refuses_one_unit_short_then_answers_exactly() {
    let exact = ASK_EXACT_SOURCE_WORK;
    for _ in 0..4 {
        assert_exact_boundary(ASK_TERMS, exact).await;
    }
    assert_budget_problem(
        router(Arc::new(source_capped(exact - 1)))
            .oneshot(authenticated(ASK_TERMS))
            .await
            .unwrap(),
    )
    .await;
    let json: serde_json::Value = serde_json::from_slice(&body(ASK_TERMS, exact).await).unwrap();
    assert_eq!(json["boolean"], true);
}

#[tokio::test]
async fn construct_fails_its_stream_one_unit_short_then_answers_exactly() {
    let exact = CONSTRUCT_EXACT_SOURCE_WORK;
    assert_exact_boundary(CONSTRUCT_TERMS, exact).await;
    let short = router(Arc::new(source_capped(exact - 1)))
        .oneshot(authenticated(CONSTRUCT_TERMS))
        .await
        .unwrap();
    assert_eq!(short.status(), StatusCode::OK);
    assert!(short.into_body().collect().await.is_err());

    let bytes = body(CONSTRUCT_TERMS, exact).await;
    let graph = std::str::from_utf8(&bytes).unwrap();
    assert!(graph.contains("http://example.test/item/1"), "{graph}");
    assert!(graph.contains("http://example.test/item/2"), "{graph}");
    assert!(graph.contains("\"one\""), "{graph}");
    assert!(graph.contains("\"two\""), "{graph}");
}

/// The cap is per request: a refused request leaves no residue that degrades
/// the next one against the same configuration.
#[tokio::test]
async fn a_refused_request_does_not_degrade_the_next_one() {
    let cfg = Arc::new(source_capped(ASK_EXACT_SOURCE_WORK));
    assert_budget_problem(
        router(Arc::new(source_capped(ASK_EXACT_SOURCE_WORK - 1)))
            .oneshot(authenticated(ASK_TERMS))
            .await
            .unwrap(),
    )
    .await;
    for _ in 0..3 {
        let response = router(cfg.clone())
            .oneshot(authenticated(ASK_TERMS))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["boolean"], true);
    }
}
