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
/// (`TermWork::uncontrolled()`): SELECT/CONSTRUCT fell 27_985 -> 27_857 and ASK's
/// ceiling 27_919 -> 27_855. Each assertion below therefore fails if that specific call
/// site stops charging the caller's control, rather than merely proving that
/// some source work is charged somewhere in a query.
const SELECT_EXACT_SOURCE_WORK: u64 = 27_985;
/// ASK's ceiling: the most source work it can spend. Unlike SELECT/CONSTRUCT,
/// ASK's total is NOT exact on the SQLite owned-worker path, and this test
/// asserts a bounded window rather than a single boundary. The worker thread
/// decodes and charges rows ahead of the consumer across a capacity-1 channel
/// (`sf-sql` `backend/sqlite/owned.rs`); ASK stops after its first solution
/// and drops the receiver, so how many prefetched rows were already charged is
/// a thread-scheduling race. Measured over 30 runs each: ASK completed at N-1
/// 9/30, N-4 11/30, N-8 2/30, N-16 0/30. Forcing a delay before each worker
/// row made it fully deterministic, and the variance was identical with this
/// slice's reconstruction charge disconnected, so the race is pre-existing and
/// not reconstruction's. SELECT drains every row and is exact (0/20).
const ASK_MAX_SOURCE_WORK: u64 = 27_919;
/// The widest observed prefetch race. Below `ASK_MAX_SOURCE_WORK - ASK_RACE_WINDOW`
/// ASK must ALWAYS refuse. Disconnecting reconstruction's charge drops ASK to
/// 27_855 (64 below the ceiling), well outside this window, so the bounded
/// assertion still fails if reconstruction stops charging.
const ASK_RACE_WINDOW: u64 = 16;
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

/// `ASK` buffers, so its refusal is the ordinary typed problem response.
/// Bounded, not exact: see `ASK_MAX_SOURCE_WORK` for the measured race.
#[tokio::test]
async fn ask_refuses_below_its_reconstruction_window_then_answers_exactly() {
    let floor = ASK_MAX_SOURCE_WORK - ASK_RACE_WINDOW;
    assert!(
        completes(ASK_TERMS, ASK_MAX_SOURCE_WORK).await,
        "ASK must always complete at its measured ceiling {ASK_MAX_SOURCE_WORK}"
    );
    // Below the race window ASK must always refuse, with the typed problem.
    assert_budget_problem(
        router(Arc::new(source_capped(floor - 1)))
            .oneshot(authenticated(ASK_TERMS))
            .await
            .unwrap(),
    )
    .await;
    let json: serde_json::Value =
        serde_json::from_slice(&body(ASK_TERMS, ASK_MAX_SOURCE_WORK).await).unwrap();
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
    let cfg = Arc::new(source_capped(ASK_MAX_SOURCE_WORK));
    assert_budget_problem(
        router(Arc::new(source_capped(
            ASK_MAX_SOURCE_WORK - ASK_RACE_WINDOW - 1,
        )))
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
