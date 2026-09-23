//! Exact/N-1 boundaries and sticky-terminal observation for [`TermWork`].

use super::TermWork;
use crate::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};

fn budget(source_work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(u64::MAX, source_work, u64::MAX, u64::MAX))
}

#[test]
fn uncontrolled_charges_and_checkpoints_always_succeed() {
    let work = TermWork::uncontrolled();
    work.charge(usize::MAX).expect("uncontrolled charge");
    work.product(usize::MAX, 3).expect("uncontrolled product");
    work.checkpoint().expect("uncontrolled checkpoint");
    assert!(work.control().is_none());
}

/// Inclusive limit: spending exactly the budget succeeds, the next unit fails.
#[test]
fn charge_admits_the_exact_budget_and_refuses_one_more() {
    let exact = budget(64);
    let work = TermWork::new(Some(&exact));
    work.charge(64).expect("exactly the budget is admitted");
    assert_eq!(exact.consumed(QueryCharge::SourceWork), 64);
    assert_eq!(
        work.charge(1)
            .expect_err("one unit past the budget refuses"),
        QueryControlError::SourceWorkExceeded
    );
}

#[test]
fn charge_of_one_more_than_the_budget_refuses_and_commits_nothing() {
    let short = budget(64);
    let work = TermWork::new(Some(&short));
    assert_eq!(
        work.charge(65).expect_err("N+1 refuses"),
        QueryControlError::SourceWorkExceeded
    );
    assert_eq!(
        short.consumed(QueryCharge::SourceWork),
        0,
        "a refused charge must not move the counter"
    );
}

#[test]
fn product_charges_count_times_width() {
    let exact = budget(30);
    let work = TermWork::new(Some(&exact));
    work.product(10, 3).expect("10 * 3 fits exactly");
    assert_eq!(exact.consumed(QueryCharge::SourceWork), 30);
    assert_eq!(
        work.product(1, 1).expect_err("past the budget"),
        QueryControlError::SourceWorkExceeded
    );
}

#[test]
fn product_overflow_terminates_rather_than_wrapping() {
    let wide = budget(u64::MAX);
    let work = TermWork::new(Some(&wide));
    assert_eq!(
        work.product(usize::MAX, 2)
            .expect_err("a wrapping product must not become a smaller charge"),
        QueryControlError::AccountingOverflow
    );
}

/// The terminal cause is sticky: once refused, later charges and bare
/// checkpoints report the same original cause, so an in-row stop stays stopped.
#[test]
fn a_terminal_control_is_observed_by_later_charges_and_checkpoints() {
    let exhausted = budget(0);
    let work = TermWork::new(Some(&exhausted));
    assert_eq!(
        work.charge(1).expect_err("zero budget refuses"),
        QueryControlError::SourceWorkExceeded
    );
    assert_eq!(
        work.checkpoint()
            .expect_err("a later checkpoint observes the sticky cause"),
        QueryControlError::SourceWorkExceeded
    );
    assert_eq!(
        work.charge(0)
            .expect_err("even a zero-unit charge observes it"),
        QueryControlError::SourceWorkExceeded
    );
}

#[test]
fn cancellation_is_observed_without_consuming_budget() {
    let live = budget(u64::MAX);
    live.terminate(QueryControlError::Cancelled);
    let work = TermWork::new(Some(&live));
    assert_eq!(
        work.checkpoint().expect_err("cancelled"),
        QueryControlError::Cancelled
    );
    assert_eq!(
        work.charge(1).expect_err("cancelled"),
        QueryControlError::Cancelled
    );
    assert_eq!(live.consumed(QueryCharge::SourceWork), 0);
}

#[test]
fn a_deadline_cause_is_reported_unchanged() {
    let live = budget(u64::MAX);
    live.terminate(QueryControlError::DeadlineExceeded);
    assert_eq!(
        TermWork::new(Some(&live))
            .charge(1)
            .expect_err("deadline is preserved, not relabelled"),
        QueryControlError::DeadlineExceeded
    );
}
