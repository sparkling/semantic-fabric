use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};

use super::*;
use crate::compile_envelope::{CompileEnvelopeError, CompileEnvelopeLimit};
use crate::Error;

fn budget(max_compiler_work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(
        max_compiler_work,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    ))
}

fn assert_control_error(error: Error, expected: QueryControlError) {
    match error {
        Error::QueryControl(actual) => assert_eq!(actual, expected),
        other => panic!("expected query-control error, got {other:?}"),
    }
}

fn envelope_error() -> CompileEnvelopeError {
    CompileEnvelopeError::LimitExceeded {
        dimension: CompileEnvelopeLimit::Tokens,
        observed: 2,
        maximum: 1,
    }
}

#[test]
fn should_reserve_exact_compiler_work_then_fail_at_n_plus_one() {
    let budget = budget(7);
    let control: &dyn QueryControl = &budget;
    let meter = CompileMeter::new(control);

    meter.reserve_work(7).expect("the inclusive limit fits");
    assert_eq!(budget.consumed(QueryCharge::CompilerWork), 7);
    assert_eq!(budget.consumed(QueryCharge::SourceWork), 0);

    assert_control_error(
        meter.reserve_work(1).expect_err("N+1 must fail"),
        QueryControlError::CompilerWorkExceeded,
    );
    assert_eq!(budget.consumed(QueryCharge::CompilerWork), 7);
}

#[test]
fn should_forward_checkpoint_through_the_same_trait_object() {
    let budget = budget(10);
    let control: &dyn QueryControl = &budget;
    let meter = CompileMeter::new(control);
    budget.terminate(QueryControlError::Cancelled);

    assert_control_error(
        meter.checkpoint().expect_err("cancelled control must stop"),
        QueryControlError::Cancelled,
    );
}

#[test]
fn should_compute_checked_units_sum_and_product_without_charging() {
    let budget = budget(100);
    let meter = CompileMeter::new(&budget);

    assert_eq!(meter.checked_usize(7).unwrap(), 7);
    assert_eq!(meter.checked_sum(&[2, 3, 5]).unwrap(), 10);
    assert_eq!(meter.checked_product(&[2, 3, 5]).unwrap(), 30);
    assert_eq!(meter.checked_product(&[2, 0, usize::MAX]).unwrap(), 0);
    assert_eq!(meter.checked_product(&[]).unwrap(), 1);
    assert_eq!(budget.consumed(QueryCharge::CompilerWork), 0);
}

#[test]
fn should_precharge_a_checked_product_at_its_exact_boundary() {
    let exact = budget(24);
    let exact_meter = CompileMeter::new(&exact);
    assert_eq!(exact_meter.precharge_product(&[2, 3, 4]).unwrap(), 24);
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), 24);

    let short = budget(23);
    let short_meter = CompileMeter::new(&short);
    assert_control_error(
        short_meter
            .precharge_product(&[2, 3, 4])
            .expect_err("prospective product exceeds the limit"),
        QueryControlError::CompilerWorkExceeded,
    );
    assert_eq!(short.consumed(QueryCharge::CompilerWork), 0);
}

#[test]
fn should_seal_sum_overflow_before_any_compiler_work() {
    let budget = budget(u64::MAX);
    let meter = CompileMeter::new(&budget);

    assert_control_error(
        meter
            .checked_sum(&[u64::MAX, 1])
            .expect_err("sum must overflow"),
        QueryControlError::AccountingOverflow,
    );
    assert_eq!(budget.consumed(QueryCharge::CompilerWork), 0);
    assert_eq!(
        budget.checkpoint(),
        Err(QueryControlError::AccountingOverflow)
    );
}

#[test]
fn should_seal_product_overflow_before_precharging_work() {
    let budget = budget(u64::MAX);
    let meter = CompileMeter::new(&budget);

    assert_control_error(
        meter
            .precharge_product(&[usize::MAX; 8])
            .expect_err("product must overflow"),
        QueryControlError::AccountingOverflow,
    );
    assert_eq!(budget.consumed(QueryCharge::CompilerWork), 0);
    assert_eq!(
        budget.checkpoint(),
        Err(QueryControlError::AccountingOverflow)
    );
}

#[test]
fn should_seal_a_distinct_envelope_reason_through_a_trait_object() {
    let budget = budget(10);
    let control: &dyn QueryControl = &budget;
    let meter = CompileMeter::new(control);

    assert_control_error(
        meter
            .reject_envelope(envelope_error())
            .expect_err("fixed envelope rejection must fail"),
        QueryControlError::CompilerEnvelopeExceeded,
    );
    assert_eq!(budget.consumed(QueryCharge::CompilerWork), 0);
    assert_control_error(
        meter
            .reserve_work(1)
            .expect_err("first terminal cause is sticky"),
        QueryControlError::CompilerEnvelopeExceeded,
    );
}

#[test]
fn should_preserve_an_earlier_terminal_over_envelope_rejection() {
    let budget = budget(10);
    budget.terminate(QueryControlError::DeadlineExceeded);
    let meter = CompileMeter::new(&budget);

    assert_control_error(
        meter
            .reject_envelope(envelope_error())
            .expect_err("the request is already terminal"),
        QueryControlError::DeadlineExceeded,
    );
}
