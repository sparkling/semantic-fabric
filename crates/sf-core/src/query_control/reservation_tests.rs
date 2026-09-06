use std::sync::{Arc, Barrier};

use super::*;
use crate::query_control::{QueryCharge, QueryControl, QueryLimits};

fn limits(shape: ReservationLimits) -> QueryLimits {
    QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX).with_reservation_limits(shape)
}

fn exact_limits(n: u64) -> QueryLimits {
    limits(ReservationLimits::new(n, n, n, n, n))
}

#[test]
fn exact_all_dimension_reservation_succeeds_and_n_plus_one_is_rollback_pure() {
    let budget = QueryBudget::new(exact_limits(7));
    let exact = ReservationShape::new(7, 7, 7, 7, 7);
    let token = budget.reserve(exact).unwrap();
    assert_eq!(token.shape(), exact);
    assert_eq!(budget.reserved(), exact);

    assert_eq!(
        budget
            .reserve(ReservationShape::new(0, 0, 0, 0, 1))
            .unwrap_err(),
        ReservationError::OperatorTasksExceeded
    );
    assert_eq!(
        budget.reserved(),
        exact,
        "rejection must not partially mutate"
    );
    assert_eq!(
        budget.checkpoint(),
        Ok(()),
        "capacity rejection is retryable"
    );

    drop(token);
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
    assert!(budget.reserve(exact).is_ok(), "Drop must restore capacity");
}

#[test]
fn every_reservation_dimension_has_an_exact_typed_boundary() {
    let cases = [
        (
            ReservationShape::new(2, 0, 0, 0, 0),
            ReservationError::RetainedBytesExceeded,
        ),
        (
            ReservationShape::new(0, 2, 0, 0, 0),
            ReservationError::SpillBytesExceeded,
        ),
        (
            ReservationShape::new(0, 0, 2, 0, 0),
            ReservationError::SpillFilesExceeded,
        ),
        (
            ReservationShape::new(0, 0, 0, 2, 0),
            ReservationError::FileDescriptorsExceeded,
        ),
        (
            ReservationShape::new(0, 0, 0, 0, 2),
            ReservationError::OperatorTasksExceeded,
        ),
    ];
    for (requested, expected) in cases {
        let budget = QueryBudget::new(exact_limits(1));
        assert_eq!(budget.reserve(requested).unwrap_err(), expected);
        assert_eq!(budget.reserved(), ReservationShape::ZERO);
    }
}

#[test]
fn late_dimension_failure_rolls_back_every_earlier_dimension() {
    let budget = QueryBudget::new(exact_limits(5));
    let baseline = ReservationShape::new(1, 1, 1, 1, 1);
    let _held = budget.reserve(baseline).unwrap();

    assert_eq!(
        budget
            .reserve(ReservationShape::new(2, 2, 2, 2, 5))
            .unwrap_err(),
        ReservationError::OperatorTasksExceeded
    );
    assert_eq!(budget.reserved(), baseline);
}

#[test]
fn arithmetic_overflow_is_rejected_without_mutation() {
    let budget = QueryBudget::new(exact_limits(u64::MAX));
    let held = budget
        .reserve(ReservationShape::new(u64::MAX, 0, 0, 0, 0))
        .unwrap();

    assert_eq!(
        budget
            .reserve(ReservationShape::for_retained_bytes(1))
            .unwrap_err(),
        ReservationError::AccountingOverflow
    );
    assert_eq!(
        budget.reserved(),
        ReservationShape::new(u64::MAX, 0, 0, 0, 0)
    );
    drop(held);
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
}

#[test]
fn clones_share_one_capacity_authority_and_release_is_exactly_once() {
    let budget = QueryBudget::new(exact_limits(3));
    let clone = budget.clone();
    let token = budget
        .reserve(ReservationShape::new(3, 3, 3, 3, 3))
        .unwrap();
    assert_eq!(
        clone
            .reserve(ReservationShape::new(0, 0, 0, 0, 1))
            .unwrap_err(),
        ReservationError::OperatorTasksExceeded
    );

    token.release();
    assert_eq!(clone.reserved(), ReservationShape::ZERO);
    let second = clone.reserve(ReservationShape::new(3, 3, 3, 3, 3)).unwrap();
    drop(second);
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
}

#[test]
fn concurrent_reservations_linearize_without_overshoot() {
    const THREADS: usize = 24;
    const CAPACITY: u64 = 7;
    let budget = QueryBudget::new(limits(ReservationLimits::new(
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        CAPACITY,
    )));
    let attempted = Arc::new(Barrier::new(THREADS + 1));
    let release = Arc::new(Barrier::new(THREADS + 1));

    let successes = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(THREADS);
        for _ in 0..THREADS {
            let budget = budget.clone();
            let attempted = Arc::clone(&attempted);
            let release = Arc::clone(&release);
            handles.push(scope.spawn(move || {
                let token = budget.reserve(ReservationShape::new(0, 0, 0, 0, 1));
                attempted.wait();
                release.wait();
                token.is_ok()
            }));
        }
        attempted.wait();
        assert_eq!(budget.reserved().operator_tasks(), CAPACITY);
        release.wait();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|success| *success)
            .count()
    });

    assert_eq!(successes, CAPACITY as usize);
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
}

#[test]
fn retained_charges_and_reservations_share_one_limit() {
    let budget = QueryBudget::new(limits(ReservationLimits::new(
        10,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    )));
    budget.consume(QueryCharge::RetainedBytes, 4).unwrap();
    let held = budget
        .reserve(ReservationShape::for_retained_bytes(6))
        .unwrap();

    assert_eq!(
        budget
            .reserve(ReservationShape::for_retained_bytes(1))
            .unwrap_err(),
        ReservationError::RetainedBytesExceeded
    );
    assert_eq!(budget.consumed(QueryCharge::RetainedBytes), 4);
    assert_eq!(budget.reserved().retained_bytes(), 6);

    drop(held);
    assert!(budget
        .reserve(ReservationShape::for_retained_bytes(6))
        .is_ok());
}

#[test]
fn live_reservation_prevents_cumulative_retained_charge_overshoot() {
    let budget = QueryBudget::new(limits(ReservationLimits::new(
        10,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    )));
    let _held = budget
        .reserve(ReservationShape::for_retained_bytes(7))
        .unwrap();

    assert_eq!(budget.consume(QueryCharge::RetainedBytes, 3), Ok(()));
    assert_eq!(
        budget.consume(QueryCharge::RetainedBytes, 1),
        Err(QueryControlError::RetainedBytesExceeded)
    );
    assert_eq!(budget.consumed(QueryCharge::RetainedBytes), 3);
}

#[test]
fn terminal_budget_never_admits_a_new_reservation() {
    let budget = QueryBudget::new(exact_limits(1));
    budget.terminate(QueryControlError::Cancelled);

    assert_eq!(
        budget.reserve(ReservationShape::ZERO).unwrap_err(),
        ReservationError::QueryTerminated(QueryControlError::Cancelled)
    );
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
}

#[test]
fn in_flight_reservation_linearizes_wholly_before_termination() {
    let budget = QueryBudget::new(exact_limits(1));
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let terminator_started = Arc::new(Barrier::new(2));

    std::thread::scope(|scope| {
        let reserve_budget = budget.clone();
        let reserve_entered = Arc::clone(&entered);
        let reserve_release = Arc::clone(&release);
        let reservation = scope.spawn(move || {
            reserve_budget.reserve_with_hook(ReservationShape::new(0, 0, 0, 0, 1), || {
                reserve_entered.wait();
                reserve_release.wait();
            })
        });

        entered.wait();
        let terminate_budget = budget.clone();
        let terminate_started = Arc::clone(&terminator_started);
        let termination = scope.spawn(move || {
            terminate_started.wait();
            terminate_budget.terminate(QueryControlError::Cancelled)
        });
        terminator_started.wait();
        release.wait();

        let token = reservation.join().unwrap().unwrap();
        assert_eq!(termination.join().unwrap(), QueryControlError::Cancelled);
        assert_eq!(
            budget.reserve(ReservationShape::ZERO).unwrap_err(),
            ReservationError::QueryTerminated(QueryControlError::Cancelled)
        );
        assert_eq!(budget.reserved().operator_tasks(), 1);
        drop(token);
    });
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
}

#[test]
fn retained_consume_cannot_race_past_an_in_flight_reservation() {
    let budget = QueryBudget::new(limits(ReservationLimits::new(
        10,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    )));
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));

    std::thread::scope(|scope| {
        let reserve_budget = budget.clone();
        let reserve_entered = Arc::clone(&entered);
        let reserve_release = Arc::clone(&release);
        let reservation = scope.spawn(move || {
            reserve_budget.reserve_with_hook(ReservationShape::for_retained_bytes(7), || {
                reserve_entered.wait();
                reserve_release.wait();
            })
        });

        entered.wait();
        let consume_budget = budget.clone();
        let consumption =
            scope.spawn(move || consume_budget.consume(QueryCharge::RetainedBytes, 4));
        release.wait();

        let token = reservation.join().unwrap().unwrap();
        assert_eq!(
            consumption.join().unwrap(),
            Err(QueryControlError::RetainedBytesExceeded)
        );
        assert_eq!(budget.reserved().retained_bytes(), 7);
        assert_eq!(budget.consumed(QueryCharge::RetainedBytes), 0);
        drop(token);
    });
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
}

#[test]
fn reservation_errors_are_static_and_redacted() {
    for error in [
        ReservationError::QueryTerminated(QueryControlError::Cancelled),
        ReservationError::RetainedBytesExceeded,
        ReservationError::SpillBytesExceeded,
        ReservationError::SpillFilesExceeded,
        ReservationError::FileDescriptorsExceeded,
        ReservationError::OperatorTasksExceeded,
        ReservationError::AccountingOverflow,
    ] {
        let rendered = error.to_string();
        assert!(!rendered.contains("7"));
        assert!(!rendered.contains("path"));
    }
}
