use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};

use super::*;
use crate::compile_envelope::{CompileEnvelopeError, CompileEnvelopeLimit};
use crate::plan_measure::PlanMeasureV1;
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

fn plan_measure(deep_clone_work: u64) -> PlanMeasureV1 {
    PlanMeasureV1 {
        nodes: 0,
        collection_slots: 0,
        payload_bytes: 0,
        deep_clone_work,
        max_depth: 0,
        max_pending_items: 0,
    }
}

fn assert_copy<T: Copy>() {}

#[test]
fn compile_context_is_copy_and_preserves_the_request_identity() {
    assert_copy::<CompileContext<'_>>();

    let budget = budget(3);
    let context = CompileContext::new(&budget);
    let copied = context;

    context.reserve_checked_sum(&[1]).unwrap();
    copied.reserve_checked_product(&[2, 1]).unwrap();

    assert_eq!(budget.consumed(QueryCharge::CompilerWork), 3);
    assert_eq!(budget.consumed(QueryCharge::SourceWork), 0);
}

#[test]
fn compile_context_reservations_cover_zero_exact_n_and_n_plus_one() {
    let zero = budget(0);
    let zero_context = CompileContext::new(&zero);
    assert_eq!(zero_context.reserve_checked_sum(&[]).unwrap(), 0);
    assert_eq!(
        zero_context
            .reserve_measured_clone(&plan_measure(0))
            .unwrap(),
        0
    );
    assert_eq!(zero.consumed(QueryCharge::CompilerWork), 0);

    let exact = budget(6);
    let exact_context = CompileContext::new(&exact);
    assert_eq!(exact_context.reserve_checked_product(&[2, 3]).unwrap(), 6);
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), 6);

    let short = budget(5);
    let short_context = CompileContext::new(&short);
    assert_control_error(
        short_context
            .reserve_checked_sum(&[2, 4])
            .expect_err("N+1 prospective work must fail"),
        QueryControlError::CompilerWorkExceeded,
    );
    assert_eq!(short.consumed(QueryCharge::CompilerWork), 0);
}

#[test]
fn compile_context_reserves_the_exact_measured_clone_work() {
    let exact = budget(7);
    let exact_context = CompileContext::new(&exact);
    let measure = plan_measure(7);

    assert_eq!(exact_context.reserve_measured_clone(&measure).unwrap(), 7);
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), 7);
    assert_eq!(exact.consumed(QueryCharge::SourceWork), 0);

    let short = budget(6);
    let short_context = CompileContext::new(&short);
    assert_control_error(
        short_context
            .reserve_measured_clone(&measure)
            .expect_err("the exact clone measure exceeds the budget"),
        QueryControlError::CompilerWorkExceeded,
    );
    assert_eq!(short.consumed(QueryCharge::CompilerWork), 0);
}

#[test]
fn compile_context_preserves_a_pre_existing_terminal_cause() {
    let budget = budget(10);
    budget.terminate(QueryControlError::DeadlineExceeded);
    let context = CompileContext::new(&budget);

    assert_control_error(
        context
            .reserve_measured_clone(&plan_measure(1))
            .expect_err("a terminal request cannot reserve clone work"),
        QueryControlError::DeadlineExceeded,
    );
    assert_control_error(
        context
            .checkpoint()
            .expect_err("the same terminal cause remains visible"),
        QueryControlError::DeadlineExceeded,
    );
    assert_eq!(budget.consumed(QueryCharge::CompilerWork), 0);
}

#[test]
fn compile_context_maps_arithmetic_and_counter_overflow_to_query_control() {
    let arithmetic = budget(u64::MAX);
    let arithmetic_context = CompileContext::new(&arithmetic);
    assert_control_error(
        arithmetic_context
            .reserve_checked_sum(&[u64::MAX, 1])
            .expect_err("prospective arithmetic must be checked"),
        QueryControlError::AccountingOverflow,
    );
    assert_eq!(arithmetic.consumed(QueryCharge::CompilerWork), 0);

    let product = budget(u64::MAX);
    let product_context = CompileContext::new(&product);
    assert_control_error(
        product_context
            .reserve_checked_product(&[usize::MAX; 8])
            .expect_err("prospective product arithmetic must be checked"),
        QueryControlError::AccountingOverflow,
    );
    assert_eq!(product.consumed(QueryCharge::CompilerWork), 0);

    let counter = budget(u64::MAX);
    let counter_context = CompileContext::new(&counter);
    counter_context
        .reserve_checked_sum(&[u64::MAX])
        .expect("the inclusive maximum fits");
    assert_control_error(
        counter_context
            .reserve_measured_clone(&plan_measure(1))
            .expect_err("the shared compiler-work counter must not wrap"),
        QueryControlError::AccountingOverflow,
    );
    assert_eq!(counter.consumed(QueryCharge::CompilerWork), u64::MAX);
    assert_eq!(
        counter.checkpoint(),
        Err(QueryControlError::AccountingOverflow)
    );
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
