//! Real governed SQL-canonicalization child round trips.
//!
//! Every case below spawns an ACTUAL contained child through the production
//! code path (`exercise_sql_canonicalize_for_evidence` calls exactly what
//! serving emission calls) against the real `semantic-fabric` executable, so
//! per-dialect canonical parity, cancellation/deadline, source accounting and
//! process lifecycle are proven end to end rather than from wire-code enums.
#![cfg(all(
    feature = "sql-canonicalize-evidence",
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
))]

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use sf_sparql::exercise_sql_canonicalize_for_evidence as canonicalize;
use sf_sparql::{
    exercise_hostile_sql_canonicalize_for_evidence as hostile_canonicalize,
    exercise_nested_sql_emission_for_evidence as nested_emission, SqlCanonicalizeEvidenceMode,
    SqlCanonicalizeEvidenceState,
};
use sf_sql::source_work::SourceWork;
use sf_sql::Dialect;

fn runtime() -> sf_sparql::ParserRuntime {
    sf_sparql::ParserRuntime::prepare(std::path::Path::new(env!("CARGO_BIN_EXE_semantic-fabric")))
        .expect("prepare held executable")
}

fn budget(source_work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(u64::MAX, source_work, u64::MAX, u64::MAX))
}

/// Minimal single-threaded executor: polls one future to completion on THIS
/// thread with no runtime and no worker pool. Deliberately not Tokio -- a
/// multi-threaded runtime could mask blocking by simply having spare
/// threads, whereas anything that blocks here blocks the only thread there
/// is.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    let mut future = Box::pin(future);
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
    }
}

/// Drive one real round trip to completion on the current thread.
fn run(
    runtime: &sf_sparql::ParserRuntime,
    dialect: Dialect,
    skeleton: &str,
    control: &dyn QueryControl,
) -> sf_sparql::Result<String> {
    block_on(canonicalize(
        runtime,
        dialect,
        skeleton,
        control,
        SourceWork::new(Some(control)),
    ))
}

fn poll_until_request_ready<F: Future>(
    future: std::pin::Pin<&mut F>,
    state: &SqlCanonicalizeEvidenceState,
) {
    let waker = std::task::Waker::noop();
    let mut context = std::task::Context::from_waker(waker);
    let mut future = future;
    for _ in 0..100_000 {
        assert!(
            future.as_mut().poll(&mut context).is_pending(),
            "the hostile child must remain incomplete until containment"
        );
        if state.request_ready() {
            assert_eq!(state.child_is_live(), Some(true));
            return;
        }
    }
    panic!("hostile child did not reach its sealed post-request barrier");
}

/// One representative skeleton per serve-wired dialect, exercising that
/// dialect's own quoting and placeholder style.
fn dialect_cases() -> [(Dialect, &'static str); 3] {
    [
        (
            Dialect::Postgres,
            "SELECT t0.\"value\" AS c0 FROM \"items\" t0 WHERE t0.\"value\" = $1",
        ),
        (
            Dialect::Sqlite,
            "SELECT t0.\"value\" AS c0 FROM \"items\" t0 WHERE t0.\"value\" = ?",
        ),
        (
            Dialect::MySql,
            "SELECT t0.`value` AS c0 FROM `items` t0 WHERE t0.`value` = ?",
        ),
    ]
}

/// The isolated child's canonical text must equal what the in-process
/// `Dialect::emit_via_ast` produces for the same skeleton, byte for byte, on
/// every serve-wired dialect. This is the parity obligation; it needs no
/// database because canonicalization is parse + render only.
#[test]
fn every_serve_wired_dialect_round_trips_byte_identically_to_raw_emission() {
    let runtime = runtime();
    for (dialect, skeleton) in dialect_cases() {
        let control = budget(u64::MAX);
        let isolated = run(&runtime, dialect, skeleton, &control)
            .unwrap_or_else(|error| panic!("{dialect:?} round trip failed: {error:?}"));
        let raw = dialect
            .emit_via_ast(skeleton)
            .unwrap_or_else(|error| panic!("{dialect:?} raw emission failed: {error}"));
        assert_eq!(
            isolated, raw,
            "{dialect:?} isolated canonical text diverged from raw emission"
        );
        // Placeholders and quoting must survive verbatim, not merely parse.
        assert_eq!(
            isolated.matches(dialect.quote_char()).count(),
            raw.matches(dialect.quote_char()).count(),
            "{dialect:?} identifier quoting changed"
        );
    }
}

/// A malformed skeleton is a closed, typed rejection -- not a hang, not a
/// crash, and not a leaked child -- and the next round trip on the same held
/// executable still succeeds.
#[test]
fn malformed_skeleton_is_rejected_and_the_next_round_trip_recovers() {
    let runtime = runtime();
    let control = budget(u64::MAX);
    let rejected = run(&runtime, Dialect::Sqlite, "SELEKT oops FROM", &control);
    assert!(
        rejected.is_err(),
        "a malformed skeleton must not canonicalize: {rejected:?}"
    );

    let recovered = run(
        &runtime,
        Dialect::Sqlite,
        "SELECT 1 AS c0",
        &budget(u64::MAX),
    )
    .expect("a clean round trip must follow a rejected one");
    assert_eq!(recovered, "SELECT 1 AS c0");
}

/// The largest skeleton the immutable transport envelope can carry is
/// actually carried end to end, and one byte more is refused while leaving
/// the held executable usable.
///
/// NOTE on what this can and cannot prove: both a protocol-ceiling refusal
/// and a transport-reservation refusal surface as the same opaque public
/// error (`sql_public_error` deliberately redacts the cause), so this
/// black-box test cannot distinguish *which* boundary refused. The
/// invariant that the protocol ceiling never exceeds the transport envelope
/// -- the property that keeps a refusal from happening mid-transfer -- is
/// enforced at compile time by the static assertions in
/// `sql_canonicalize_protocol`, and is covered behaviourally by
/// `step_reservations_admit_whole_transfers_against_cumulative_caps` in
/// `supervisor::io`'s unit tests.
#[test]
fn the_largest_carriable_skeleton_round_trips_and_one_byte_more_is_refused() {
    const LARGEST_CARRIABLE: usize = 1_048_584; // envelope - Hello - header.
    const PREFIX: &str = "SELECT 1 AS c0 FROM t WHERE '";
    const SUFFIX: &str = "' = ''";
    let runtime = runtime();

    let filler = LARGEST_CARRIABLE - PREFIX.len() - SUFFIX.len();
    let maximal = format!("{PREFIX}{}{SUFFIX}", "x".repeat(filler));
    assert_eq!(maximal.len(), LARGEST_CARRIABLE, "maximal skeleton sizing");
    let carried = run(&runtime, Dialect::Sqlite, &maximal, &budget(u64::MAX))
        .expect("a maximal in-envelope skeleton must be carried end to end");
    assert_eq!(
        carried.len(),
        LARGEST_CARRIABLE,
        "the maximal skeleton must round trip in full"
    );

    let oversized = format!("{PREFIX}{}{SUFFIX}", "x".repeat(filler + 1));
    assert_eq!(oversized.len(), LARGEST_CARRIABLE + 1);
    assert!(
        run(&runtime, Dialect::Sqlite, &oversized, &budget(u64::MAX)).is_err(),
        "a skeleton past the transport envelope must be refused"
    );

    let recovered = run(
        &runtime,
        Dialect::Sqlite,
        "SELECT 1 AS c0",
        &budget(u64::MAX),
    )
    .expect("refusal must not poison the held executable");
    assert_eq!(recovered, "SELECT 1 AS c0");
}

/// A control already cancelled before the call never launches a child, and
/// the caller sees the sticky terminal cause -- not a generic failure.
#[test]
fn a_cancelled_control_refuses_before_launch_with_its_sticky_cause() {
    let runtime = runtime();
    for reason in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        let control = budget(u64::MAX);
        control.terminate(reason);
        let result = run(&runtime, Dialect::Sqlite, "SELECT 1 AS c0", &control);
        match result {
            Err(sf_sparql::Error::QueryControl(actual)) => assert_eq!(
                actual, reason,
                "the sticky terminal cause must survive the isolation boundary"
            ),
            other => panic!("expected the sticky {reason:?}, got {other:?}"),
        }
    }
}

/// Cancellation raised part-way through the round trip (from another thread,
/// while the child is live) is observed by the bounded steps, maps to the
/// sticky cause, and still leaves the held executable usable -- which it
/// could not be if the child had been abandoned unreaped.
#[test]
fn mid_flight_cancellation_is_observed_and_the_child_is_contained() {
    let runtime = runtime();
    for reason in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        let control = Arc::new(budget(u64::MAX));
        let state = SqlCanonicalizeEvidenceState::default();
        let mut future = Box::pin(hostile_canonicalize(
            &runtime,
            SqlCanonicalizeEvidenceMode::HoldAfterRequest,
            &state,
            control.as_ref(),
            SourceWork::new(Some(control.as_ref())),
        ));
        poll_until_request_ready(future.as_mut(), &state);

        // The canonicalizer is Pending on this sole executor thread while the
        // real child is live. A sibling future must still run immediately.
        let progressed = AtomicUsize::new(0);
        let sibling = async { progressed.store(1, Ordering::SeqCst) };
        block_on(sibling);
        assert_eq!(progressed.load(Ordering::SeqCst), 1);

        control.terminate(reason);
        match block_on(future) {
            Err(sf_sparql::Error::QueryControl(actual)) => assert_eq!(actual, reason),
            other => panic!("expected sticky {reason:?}, got {other:?}"),
        }
        assert_eq!(
            state.child_is_live(),
            Some(false),
            "the exact barrier child must be reaped before cancellation returns"
        );
    }

    assert_eq!(
        run(
            &runtime,
            Dialect::Sqlite,
            "SELECT 1 AS c0",
            &budget(u64::MAX),
        )
        .expect("contained cancellation must permit a fresh launch"),
        "SELECT 1 AS c0"
    );
}

#[test]
fn nested_emission_yields_to_a_sibling_while_its_real_child_is_incomplete() {
    let runtime = runtime();
    let control = budget(u64::MAX);
    let state = Arc::new(SqlCanonicalizeEvidenceState::default());
    let mut future = Box::pin(nested_emission(&runtime, Arc::clone(&state), &control));
    poll_until_request_ready(future.as_mut(), &state);

    let progressed = AtomicUsize::new(0);
    block_on(async { progressed.store(1, Ordering::SeqCst) });
    assert_eq!(progressed.load(Ordering::SeqCst), 1);

    control.terminate(QueryControlError::Cancelled);
    match block_on(future) {
        Err(sf_sparql::Error::QueryControl(QueryControlError::Cancelled)) => {}
        other => panic!("nested emission must preserve cancellation, got {other:?}"),
    }
    assert_eq!(state.child_is_live(), Some(false));
}

/// Dropping the in-flight future between bounded steps must still contain and
/// reap the child: the next round trip on the same executable succeeds, which
/// a leaked or unreaped child would jeopardise.
#[test]
fn dropping_the_future_mid_flight_still_reaps_and_capacity_recovers() {
    let runtime = runtime();
    let control = budget(u64::MAX);
    let state = SqlCanonicalizeEvidenceState::default();
    let mut future = Box::pin(hostile_canonicalize(
        &runtime,
        SqlCanonicalizeEvidenceMode::HoldAfterRequest,
        &state,
        &control,
        SourceWork::new(Some(&control)),
    ));
    poll_until_request_ready(future.as_mut(), &state);
    drop(future);
    assert_eq!(
        state.child_is_live(),
        Some(false),
        "dropping the future must synchronously reap the original pidfd child"
    );

    let recovered = run(
        &runtime,
        Dialect::Sqlite,
        "SELECT 1 AS c0",
        &budget(u64::MAX),
    )
    .expect("a dropped future must not strand the held executable");
    assert_eq!(recovered, "SELECT 1 AS c0");
}

#[test]
fn malformed_truncated_and_oversized_child_results_are_contained_and_recover() {
    let runtime = runtime();
    for mode in [
        SqlCanonicalizeEvidenceMode::MalformedResult,
        SqlCanonicalizeEvidenceMode::TruncatedResult,
        SqlCanonicalizeEvidenceMode::OversizedResult,
    ] {
        let control = budget(u64::MAX);
        let state = SqlCanonicalizeEvidenceState::default();
        let result = block_on(hostile_canonicalize(
            &runtime,
            mode,
            &state,
            &control,
            SourceWork::new(Some(&control)),
        ));
        assert!(result.is_err(), "{mode:?} must be rejected");
        assert_eq!(
            state.child_is_live(),
            Some(false),
            "{mode:?} must reap its exact child"
        );
        assert_eq!(
            run(
                &runtime,
                Dialect::Sqlite,
                "SELECT 1 AS c0",
                &budget(u64::MAX),
            )
            .expect("a malformed child result must not poison the next launch"),
            "SELECT 1 AS c0"
        );
    }
}

/// Source accounting is prospective and exact: the measured total succeeds,
/// one unit less fails, and the shortfall is the sticky source-work cause.
#[test]
fn source_accounting_is_exact_and_one_unit_short_fails() {
    let runtime = runtime();
    let skeleton = "SELECT t0.\"value\" AS c0 FROM \"items\" t0";

    let meter = budget(u64::MAX);
    let expected = run(&runtime, Dialect::Sqlite, skeleton, &meter).expect("metered run");
    let total = meter.consumed(QueryCharge::SourceWork);
    let independently_expected =
        88_u64 + 2 * skeleton.len() as u64 + 96 + 4 * expected.len() as u64;
    assert_eq!(
        total, independently_expected,
        "accounting must cover request digest/allocation-copy and all four result-body passes"
    );

    let exact = budget(total);
    assert_eq!(
        run(&runtime, Dialect::Sqlite, skeleton, &exact).expect("exact budget must succeed"),
        expected
    );
    assert_eq!(exact.consumed(QueryCharge::SourceWork), total);

    let short = budget(total - 1);
    match run(&runtime, Dialect::Sqlite, skeleton, &short) {
        Err(sf_sparql::Error::QueryControl(QueryControlError::SourceWorkExceeded)) => {}
        other => panic!("one unit short must fail with SourceWorkExceeded, got {other:?}"),
    }
}

/// Every individual charge boundary is honoured: terminating at the Nth
/// charge always surfaces that exact sticky cause, never a later or generic
/// one, and never a successful result.
struct StopAt {
    budget: QueryBudget,
    calls: AtomicUsize,
    at: usize,
    reason: QueryControlError,
}

impl QueryControl for StopAt {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        self.budget.checkpoint()
    }
    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.budget.terminate(reason)
    }
    fn consume(&self, kind: QueryCharge, units: u64) -> Result<(), QueryControlError> {
        self.budget.consume(kind, units)?;
        if self.calls.fetch_add(1, Ordering::Relaxed) + 1 == self.at {
            self.budget.terminate(self.reason);
        }
        self.budget.checkpoint()
    }
}

#[test]
fn terminating_at_each_charge_preserves_that_exact_cause() {
    let runtime = runtime();
    let skeleton = "SELECT 1 AS c0";

    let meter = StopAt {
        budget: budget(u64::MAX),
        calls: AtomicUsize::new(0),
        at: usize::MAX,
        reason: QueryControlError::Cancelled,
    };
    run(&runtime, Dialect::Sqlite, skeleton, &meter).expect("metered run");
    let charges = meter.calls.load(Ordering::Relaxed);
    assert!(charges > 0, "the round trip must make charges");

    for reason in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for at in 1..=charges {
            let control = StopAt {
                budget: budget(u64::MAX),
                calls: AtomicUsize::new(0),
                at,
                reason,
            };
            match run(&runtime, Dialect::Sqlite, skeleton, &control) {
                Err(sf_sparql::Error::QueryControl(actual)) => assert_eq!(
                    actual, reason,
                    "charge {at} must surface its own sticky cause"
                ),
                other => panic!("charge {at} with {reason:?} must fail, got {other:?}"),
            }
            assert_eq!(control.checkpoint(), Err(reason));
        }
    }
}
