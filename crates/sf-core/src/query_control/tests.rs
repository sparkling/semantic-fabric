use super::*;

#[test]
fn compiler_work_limit_is_exposed_and_has_an_exact_sticky_boundary() {
    let limits = QueryLimits::new(3, 4, 5, 6);
    assert_eq!(limits.max_compiler_work(), 3);

    let budget = QueryBudget::new(limits);
    budget.consume(QueryCharge::CompilerWork, 3).unwrap();
    assert_eq!(budget.consumed(QueryCharge::CompilerWork), 3);
    assert_eq!(
        budget.consume(QueryCharge::CompilerWork, 1),
        Err(QueryControlError::CompilerWorkExceeded)
    );
    assert_eq!(
        budget.consume(QueryCharge::SourceWork, 1),
        Err(QueryControlError::CompilerWorkExceeded)
    );
    assert_eq!(budget.consumed(QueryCharge::SourceWork), 0);
}

#[test]
fn compiler_work_arithmetic_overflow_fails_closed() {
    let budget = QueryBudget::new(QueryLimits::new(u64::MAX, 1, 1, 1));
    budget.consume(QueryCharge::CompilerWork, u64::MAX).unwrap();

    assert_eq!(
        budget.consume(QueryCharge::CompilerWork, 1),
        Err(QueryControlError::AccountingOverflow)
    );
    assert_eq!(budget.consumed(QueryCharge::CompilerWork), u64::MAX);
    assert_eq!(
        budget.consume(QueryCharge::ResultItems, 1),
        Err(QueryControlError::AccountingOverflow)
    );
}

#[test]
fn concurrent_compiler_consumers_cannot_overshoot() {
    let budget = QueryBudget::new(QueryLimits::new(10_000, 1, 1, 1));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let budget = budget.clone();
            scope.spawn(
                move || {
                    while budget.consume(QueryCharge::CompilerWork, 1).is_ok() {}
                },
            );
        }
    });

    assert_eq!(budget.consumed(QueryCharge::CompilerWork), 10_000);
    assert_eq!(
        budget.checkpoint(),
        Err(QueryControlError::CompilerWorkExceeded)
    );
}

#[test]
fn inclusive_limit_succeeds_then_first_failure_is_sticky() {
    let budget = QueryBudget::new(QueryLimits::new(u64::MAX, 3, 2, 5));
    budget.consume(QueryCharge::SourceWork, 3).unwrap();
    assert_eq!(budget.consumed(QueryCharge::SourceWork), 3);
    assert_eq!(
        budget.consume(QueryCharge::SourceWork, 1),
        Err(QueryControlError::SourceWorkExceeded)
    );
    assert_eq!(
        budget.consume(QueryCharge::ResultItems, 1),
        Err(QueryControlError::SourceWorkExceeded)
    );
    assert_eq!(budget.consumed(QueryCharge::ResultItems), 0);
}

#[test]
fn every_dimension_has_an_exact_typed_boundary() {
    for (charge, expected) in [
        (
            QueryCharge::CompilerWork,
            QueryControlError::CompilerWorkExceeded,
        ),
        (
            QueryCharge::SourceWork,
            QueryControlError::SourceWorkExceeded,
        ),
        (
            QueryCharge::ResultItems,
            QueryControlError::ResultItemsExceeded,
        ),
        (
            QueryCharge::SerializedBytes,
            QueryControlError::SerializedBytesExceeded,
        ),
        (
            QueryCharge::RetainedBytes,
            QueryControlError::RetainedBytesExceeded,
        ),
    ] {
        let budget = QueryBudget::new(QueryLimits::new(1, 1, 1, 1).with_max_retained_bytes(1));
        budget.consume(charge, 1).unwrap();
        assert_eq!(budget.consume(charge, 1), Err(expected));
    }
}

#[test]
fn arithmetic_overflow_fails_closed() {
    let budget = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, 1, 1));
    budget.consume(QueryCharge::SourceWork, u64::MAX).unwrap();
    assert_eq!(
        budget.consume(QueryCharge::SourceWork, 1),
        Err(QueryControlError::AccountingOverflow)
    );
    assert_eq!(budget.consumed(QueryCharge::SourceWork), u64::MAX);
}

#[test]
fn concurrent_consumers_cannot_overshoot() {
    let budget = QueryBudget::new(QueryLimits::new(u64::MAX, 10_000, 1, 1));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let budget = budget.clone();
            scope.spawn(
                move || {
                    while budget.consume(QueryCharge::SourceWork, 1).is_ok() {}
                },
            );
        }
    });
    assert_eq!(budget.consumed(QueryCharge::SourceWork), 10_000);
    assert_eq!(
        budget.checkpoint(),
        Err(QueryControlError::SourceWorkExceeded)
    );
}

#[test]
fn cancellation_wins_and_remains_the_terminal_reason() {
    let budget = QueryBudget::new(QueryLimits::new(u64::MAX, 10, 10, 10));
    assert_eq!(
        budget.terminate(QueryControlError::Cancelled),
        QueryControlError::Cancelled
    );
    assert_eq!(
        budget.terminate(QueryControlError::DeadlineExceeded),
        QueryControlError::Cancelled
    );
    assert_eq!(budget.checkpoint(), Err(QueryControlError::Cancelled));
}

#[test]
fn trait_object_termination_reuses_the_budget_first_cause() {
    let budget = QueryBudget::new(QueryLimits::new(u64::MAX, 10, 10, 10));
    let control: &dyn QueryControl = &budget;

    assert_eq!(
        control.terminate(QueryControlError::CompilerWorkExceeded),
        QueryControlError::CompilerWorkExceeded
    );
    assert_eq!(
        control.terminate(QueryControlError::Cancelled),
        QueryControlError::CompilerWorkExceeded
    );
    assert_eq!(
        control.checkpoint(),
        Err(QueryControlError::CompilerWorkExceeded)
    );
    assert_eq!(
        <UncontrolledQueryControl as QueryControl>::terminate(
            &UncontrolledQueryControl,
            QueryControlError::Cancelled,
        ),
        QueryControlError::Cancelled
    );
}

#[test]
fn an_in_flight_charge_linearizes_before_termination() {
    let budget = QueryBudget::new(QueryLimits::new(u64::MAX, 1, 1, 1));
    let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
    let release = std::sync::Arc::new(std::sync::Barrier::new(2));

    std::thread::scope(|scope| {
        let charge_budget = budget.clone();
        let charge_entered = entered.clone();
        let charge_release = release.clone();
        let charge = scope.spawn(move || {
            charge_budget.consume_with_hook(QueryCharge::SourceWork, 1, || {
                charge_entered.wait();
                charge_release.wait();
            })
        });
        entered.wait();
        let terminate_budget = budget.clone();
        let terminate =
            scope.spawn(move || terminate_budget.terminate(QueryControlError::Cancelled));
        release.wait();

        assert_eq!(charge.join().unwrap(), Ok(()));
        assert_eq!(terminate.join().unwrap(), QueryControlError::Cancelled);
    });
    assert_eq!(budget.consumed(QueryCharge::SourceWork), 1);
    assert_eq!(budget.checkpoint(), Err(QueryControlError::Cancelled));
}

#[test]
fn a_charge_rejected_after_termination_changes_no_counter() {
    let budget = QueryBudget::new(QueryLimits::new(u64::MAX, 1, 1, 1));
    budget.terminate(QueryControlError::Cancelled);

    assert_eq!(
        budget.consume(QueryCharge::SourceWork, 1),
        Err(QueryControlError::Cancelled)
    );
    assert_eq!(budget.consumed(QueryCharge::SourceWork), 0);
}
