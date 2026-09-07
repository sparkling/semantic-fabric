//! Request-budget unit tests kept separate from the production module size boundary.

use std::time::Duration;

use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError, QueryLimits};

use crate::budget::RequestBudget;

#[test]
fn trait_object_termination_delegates_to_the_sticky_request_identity() {
    let budget = RequestBudget::after(
        Duration::from_secs(60),
        QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
    );
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
        budget.checkpoint(),
        Err(QueryControlError::CompilerWorkExceeded)
    );
}

#[tokio::test(start_paused = true)]
async fn cancellation_wakes_a_pending_phase_and_is_sticky() {
    let budget = RequestBudget::after(
        Duration::from_secs(60),
        QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
    );
    let waiter = tokio::spawn({
        let budget = budget.clone();
        async move { budget.run(std::future::pending::<()>()).await }
    });
    tokio::task::yield_now().await;

    assert_eq!(budget.cancel(), QueryControlError::Cancelled);
    assert_eq!(
        waiter.await.expect("waiter task"),
        Err(QueryControlError::Cancelled)
    );
    assert_eq!(budget.checkpoint(), Err(QueryControlError::Cancelled));
}

#[tokio::test(start_paused = true)]
async fn limit_failure_wakes_an_independent_pending_phase() {
    let budget = RequestBudget::after(
        Duration::from_secs(60),
        QueryLimits::new(u64::MAX, 0, u64::MAX, u64::MAX),
    );
    let waiter = tokio::spawn({
        let budget = budget.clone();
        async move { budget.run(std::future::pending::<()>()).await }
    });
    tokio::task::yield_now().await;

    assert_eq!(
        budget.consume(QueryCharge::SourceWork, 1),
        Err(QueryControlError::SourceWorkExceeded)
    );
    assert_eq!(budget.consumed(QueryCharge::SourceWork), 0);
    assert_eq!(
        waiter.await.expect("waiter task"),
        Err(QueryControlError::SourceWorkExceeded)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn handler_output_is_rechecked_against_the_absolute_deadline() {
    let budget = RequestBudget::after(
        Duration::from_millis(1),
        QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
    );
    let result = budget
        .run_until_deadline(async {
            std::thread::sleep(Duration::from_millis(20));
            "response"
        })
        .await;
    assert_eq!(result, Err(QueryControlError::DeadlineExceeded));
}

#[tokio::test(start_paused = true)]
async fn ready_handoff_before_deadline_ignores_a_sticky_resource_failure() {
    let budget = RequestBudget::after(
        Duration::from_secs(5),
        QueryLimits::new(u64::MAX, 0, u64::MAX, u64::MAX),
    );
    assert_eq!(
        budget.consume(QueryCharge::SourceWork, 1),
        Err(QueryControlError::SourceWorkExceeded)
    );
    assert_eq!(
        budget.run_until_deadline(async { "response" }).await,
        Ok("response")
    );
    assert_eq!(
        budget.checkpoint(),
        Err(QueryControlError::SourceWorkExceeded)
    );
}

#[tokio::test(start_paused = true)]
async fn expired_handoff_reports_deadline_while_accounting_keeps_first_cause() {
    let budget = RequestBudget::after(
        Duration::from_secs(5),
        QueryLimits::new(u64::MAX, 0, u64::MAX, u64::MAX),
    );
    assert_eq!(
        budget.consume(QueryCharge::SourceWork, 1),
        Err(QueryControlError::SourceWorkExceeded)
    );
    tokio::time::advance(Duration::from_secs(5)).await;
    assert_eq!(
        budget.run_until_deadline(async { "response" }).await,
        Err(QueryControlError::DeadlineExceeded)
    );
    assert_eq!(
        budget.checkpoint(),
        Err(QueryControlError::SourceWorkExceeded)
    );
}

#[tokio::test(start_paused = true)]
async fn pending_handoff_reports_deadline_after_an_earlier_resource_failure() {
    let budget = RequestBudget::after(
        Duration::from_secs(5),
        QueryLimits::new(u64::MAX, 0, u64::MAX, u64::MAX),
    );
    assert_eq!(
        budget.consume(QueryCharge::SourceWork, 1),
        Err(QueryControlError::SourceWorkExceeded)
    );
    let waiter = tokio::spawn({
        let budget = budget.clone();
        async move {
            budget
                .run_until_deadline(std::future::pending::<()>())
                .await
        }
    });
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(5)).await;
    assert_eq!(
        waiter.await.expect("waiter task"),
        Err(QueryControlError::DeadlineExceeded)
    );
    assert_eq!(
        budget.checkpoint(),
        Err(QueryControlError::SourceWorkExceeded)
    );
}

#[tokio::test(start_paused = true)]
async fn unrepresentable_handoff_deadline_remains_accounting_overflow() {
    let budget = RequestBudget::after(
        Duration::MAX,
        QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
    );
    assert_eq!(
        budget.run_until_deadline(async { "response" }).await,
        Err(QueryControlError::AccountingOverflow)
    );
    assert_eq!(
        budget.checkpoint(),
        Err(QueryControlError::AccountingOverflow)
    );
}

#[tokio::test(start_paused = true)]
async fn cancellation_at_or_after_deadline_records_deadline() {
    let expired = RequestBudget::after(
        Duration::from_secs(5),
        QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
    );
    tokio::time::advance(Duration::from_secs(5)).await;
    assert_eq!(expired.cancel(), QueryControlError::DeadlineExceeded);

    let live = RequestBudget::after(
        Duration::from_secs(5),
        QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
    );
    assert_eq!(live.cancel(), QueryControlError::Cancelled);
}

#[test]
fn ask_preflight_rejects_zero_capacity_without_consuming() {
    let budget = RequestBudget::after(
        Duration::from_secs(5),
        QueryLimits::new(u64::MAX, u64::MAX, 0, u64::MAX),
    );
    assert_eq!(
        budget.preflight_ask_result(),
        Err(QueryControlError::ResultItemsExceeded)
    );
    assert_eq!(budget.consumed(QueryCharge::ResultItems), 0);
}

#[test]
fn ask_preflight_leaves_positive_capacity_for_the_executor() {
    let budget = RequestBudget::after(
        Duration::from_secs(5),
        QueryLimits::new(u64::MAX, u64::MAX, 1, u64::MAX),
    );
    assert_eq!(budget.preflight_ask_result(), Ok(()));
    assert_eq!(budget.consumed(QueryCharge::ResultItems), 0);
}
