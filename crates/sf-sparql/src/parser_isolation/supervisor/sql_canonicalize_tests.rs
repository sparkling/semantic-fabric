//! Containment-precedence and bounded-step lifecycle evidence for the
//! governed SQL-canonicalization peer, observed inside the supervisor where
//! a cleanup failure is distinguishable from the cause that triggered it.

use std::sync::atomic::{AtomicUsize, Ordering};

use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};

use super::super::linux::spawn_fixture;
use super::super::{v1_limits, SupervisorError};
use super::contained_checkpoint;
use crate::parser_isolation::supervisor::executable::PreparedParserExecutable;

/// A control that is already terminal, so `contained_checkpoint` must take
/// its failure path on the first call.
struct Cancelled(QueryControlError);

impl QueryControl for Cancelled {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        Err(self.0)
    }
    fn terminate(&self, _: QueryControlError) -> QueryControlError {
        self.0
    }
    fn consume(&self, _: QueryCharge, _: u64) -> Result<(), QueryControlError> {
        Err(self.0)
    }
}

fn prepared_cat() -> PreparedParserExecutable {
    PreparedParserExecutable::from_file_for_test(
        std::fs::File::open("/bin/cat").expect("open cat fixture"),
        true,
    )
    .expect("prepare cat fixture")
}

/// A mid-flight checkpoint failure must CONTAIN the live child -- signal,
/// sweep and exactly reap it -- before returning, rather than returning the
/// bare cause and leaving cleanup to the `Drop` backstop (which cannot
/// surface its own failure). Observed here as: after the call, the process
/// has been reaped (`child` taken) rather than still live.
#[test]
fn a_checkpoint_failure_contains_and_reaps_before_returning() {
    for reason in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        let executable = prepared_cat();
        let mut process = spawn_fixture(&executable, v1_limits(), &[b"cat", b"-"])
            .expect("launch bounded fixture");
        assert!(process.child.is_some(), "fixture must start live");

        let error = contained_checkpoint(&mut process, &Cancelled(reason))
            .expect_err("a terminal control must fail the checkpoint");

        // The cause is preserved when containment itself succeeds...
        assert!(
            matches!(error, SupervisorError::RequestControl(actual) if actual == reason),
            "expected the sticky {reason:?}, got {error:?}"
        );
        // ...and, decisively, the child was reaped INSIDE the call, not left
        // for `Drop`. A bare `checkpoint()?` would leave `child` populated.
        assert!(
            process.child.is_none(),
            "the child must be contained and reaped before returning"
        );
    }
}

/// Containment is attempted exactly once per failed checkpoint and leaves the
/// process in a terminal state, so a second checkpoint on an already-reaped
/// process surfaces the invalid-state containment error rather than silently
/// succeeding or double-reaping.
#[test]
fn containment_is_terminal_and_not_repeated() {
    let executable = prepared_cat();
    let mut process =
        spawn_fixture(&executable, v1_limits(), &[b"cat", b"-"]).expect("launch fixture");
    let control = Cancelled(QueryControlError::Cancelled);

    contained_checkpoint(&mut process, &control).expect_err("first checkpoint fails");
    assert!(process.child.is_none(), "first call reaps");

    // A second attempt must surface the containment failure (already reaped)
    // in preference to the original cause, proving cleanup-failure precedence.
    let second = contained_checkpoint(&mut process, &control)
        .expect_err("a second checkpoint on a reaped process still fails");
    assert!(
        matches!(second, SupervisorError::InvalidState(_)),
        "a containment failure must win over the original cause, got {second:?}"
    );
}

/// The bounded-step loops must never block: each step polls for at most
/// `STEP_TIMEOUT_MS`, so a silent peer yields control promptly instead of
/// holding the caller's thread for the child's whole wall ceiling.
#[test]
fn a_bounded_step_returns_promptly_against_a_silent_peer() {
    let executable = prepared_cat();
    let mut process =
        spawn_fixture(&executable, v1_limits(), &[b"cat", b"-"]).expect("launch fixture");

    // `cat` echoes nothing until written to, so a read step must come back
    // Pending quickly rather than waiting out the worker's wall deadline.
    let mut buffer = [0_u8; 1];
    let mut transfer = process.reserve_read(buffer.len()).expect("admitted");
    let started = std::time::Instant::now();
    let outcome = process
        .read_step_contained(&mut buffer, &mut transfer, super::STEP_TIMEOUT_MS)
        .expect("a silent peer is Pending, not an error");
    let elapsed = started.elapsed();

    assert_eq!(outcome, super::StepOutcome::Pending);
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "one bounded step must not block the caller: {elapsed:?}"
    );
    // The worker's own wall ceiling is far larger, so this really did come
    // back on the step cap rather than the lifetime.
    assert!(
        v1_limits().values().wall_time_millis >= 1_000,
        "the wall ceiling must dwarf the step cap for this to be meaningful"
    );

    let _ = process.terminate_and_reap();
}
