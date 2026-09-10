use sf_core::ir::LogicalSource;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};

use super::*;
use crate::compile_envelope::{CompileEnvelopeError, CompileEnvelopeLimit};
use crate::iq::node::{IqCond, IqNode};
use crate::iq::{Branch, Scan, TermDef};
use crate::plan_measure::clone_root::{
    measure_compiler_clone_collection_v1, CompilerCloneCollectionV1,
};
use crate::plan_measure::{PlanMeasureError, PlanMeasureLimit};
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

fn branch_forest() -> Vec<Branch> {
    vec![Branch::single(Scan {
        alias: 1,
        source: (LogicalSource::Table("source".to_owned())).into(),
    })]
}

fn nested_iq_conditions() -> Vec<IqCond> {
    vec![IqCond::Exists(Box::new(IqNode::Values {
        vars: vec!["inside".into()],
        rows: vec![vec![Some(TermDef::Const(
            spargebra::term::NamedNode::new("http://example.test/inside")
                .unwrap()
                .into(),
        ))]],
    }))]
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
    assert!(zero_context.clone_branch_forest(&[]).unwrap().is_empty());
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
fn compile_context_binds_exact_measure_reservation_and_one_clone() {
    let source = branch_forest();
    let measure =
        measure_compiler_clone_collection_v1(CompilerCloneCollectionV1::Branches(&source)).unwrap();
    let exact = budget(measure.deep_clone_work);
    let exact_context = CompileContext::new(&exact);
    let source_allocation = source.as_ptr();

    let cloned = exact_context.clone_branch_forest(&source).unwrap();
    assert_eq!(cloned.len(), source.len());
    assert_eq!(cloned[0].core[0].alias, source[0].core[0].alias);
    match (
        cloned[0].core[0].source.logical(),
        source[0].core[0].source.logical(),
    ) {
        (Some(LogicalSource::Table(actual)), Some(LogicalSource::Table(expected))) => {
            assert_eq!(actual, expected)
        }
        _ => panic!("branch forest fixture must retain its table source"),
    }
    assert_ne!(cloned.as_ptr(), source_allocation);
    assert_eq!(
        exact.consumed(QueryCharge::CompilerWork),
        measure.deep_clone_work
    );
    assert_eq!(exact.consumed(QueryCharge::SourceWork), 0);

    let short = budget(measure.deep_clone_work - 1);
    let short_context = CompileContext::new(&short);
    assert_control_error(
        short_context
            .clone_branch_forest(&source)
            .expect_err("the exact clone measure exceeds the budget"),
        QueryControlError::CompilerWorkExceeded,
    );
    assert_eq!(short.consumed(QueryCharge::CompilerWork), 0);
}

#[test]
fn branch_copy_reservation_binds_the_source_and_precedes_the_operation() {
    let source = branch_forest().remove(0);
    let work = measure_compiler_clone_root_v1(CompilerCloneRootV1::Branch(&source))
        .unwrap()
        .deep_clone_work;
    for allowance in [work - 1, work] {
        let control = budget(allowance);
        let called = std::cell::Cell::new(false);
        let result = CompileContext::new(&control).with_reserved_branch_copy(&source, |measured| {
            assert!(std::ptr::eq(measured, &source));
            assert_eq!(control.consumed(QueryCharge::CompilerWork), work);
            called.set(true);
            Ok(measured.core[0].clone())
        });
        assert_eq!(called.get(), allowance == work);
        if allowance == work {
            assert_eq!(
                format!("{:?}", result.unwrap()),
                format!("{:?}", source.core[0])
            );
        } else {
            assert_control_error(result.unwrap_err(), QueryControlError::CompilerWorkExceeded);
            assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
        }
    }
}

#[test]
fn branch_copy_observes_cancellation_before_and_after_the_operation() {
    let source = Branch::empty();
    for (already_cancelled, operation_fails) in [(true, false), (false, false), (false, true)] {
        let control = budget(u64::MAX);
        if already_cancelled {
            control.terminate(QueryControlError::Cancelled);
        }
        let called = std::cell::Cell::new(false);
        let result = CompileContext::new(&control).with_reserved_branch_copy(&source, |_| {
            called.set(true);
            control.terminate(QueryControlError::Cancelled);
            if operation_fails {
                Err(Error::Unsupported("fixture merge rejection".into()))
            } else {
                Ok(())
            }
        });
        assert_control_error(result.unwrap_err(), QueryControlError::Cancelled);
        assert_eq!(called.get(), !already_cancelled);
        assert_eq!(
            control.consumed(QueryCharge::CompilerWork),
            u64::from(!already_cancelled)
        );
    }
    let control = budget(u64::MAX);
    assert!(matches!(
        CompileContext::new(&control).with_reserved_branch_copy(&source, |_| {
            Err::<(), _>(Error::Unsupported("fixture merge rejection".into()))
        }),
        Err(Error::Unsupported(_))
    ));
    assert_eq!(control.checkpoint(), Ok(()));
}

#[test]
fn compile_context_binds_exact_iq_condition_measure_to_one_clone() {
    let source = nested_iq_conditions();
    let measure =
        measure_compiler_clone_collection_v1(CompilerCloneCollectionV1::IqConditions(&source))
            .unwrap();
    let exact = budget(measure.deep_clone_work);
    let source_allocation = source.as_ptr();

    let cloned = CompileContext::new(&exact)
        .clone_iq_conditions(&source)
        .unwrap();

    assert_eq!(format!("{cloned:?}"), format!("{source:?}"));
    assert_ne!(cloned.as_ptr(), source_allocation);
    assert_eq!(
        exact.consumed(QueryCharge::CompilerWork),
        measure.deep_clone_work
    );

    let short = budget(measure.deep_clone_work - 1);
    assert_control_error(
        CompileContext::new(&short)
            .clone_iq_conditions(&source)
            .expect_err("N-1 must reject before cloning the condition forest"),
        QueryControlError::CompilerWorkExceeded,
    );
    assert_eq!(source.as_ptr(), source_allocation);
    assert_eq!(short.consumed(QueryCharge::CompilerWork), 0);
}

#[test]
fn compile_context_preserves_a_pre_existing_terminal_cause() {
    let budget = budget(10);
    budget.terminate(QueryControlError::DeadlineExceeded);
    let context = CompileContext::new(&budget);

    assert_control_error(
        context
            .clone_branch_forest(&branch_forest())
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
            .clone_branch_forest(&branch_forest())
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
fn compile_context_preserves_measurement_failure_classification() {
    let allocation = budget(10);
    let allocation_context = CompileContext::new(&allocation);
    assert_control_error(
        allocation_context.measurement_error(PlanMeasureError::AllocationFailed),
        QueryControlError::CompilerResourceExhausted,
    );
    assert_eq!(
        allocation.checkpoint(),
        Err(QueryControlError::CompilerResourceExhausted)
    );
    assert_eq!(allocation.consumed(QueryCharge::CompilerWork), 0);

    let envelope = budget(10);
    let envelope_context = CompileContext::new(&envelope);
    assert_control_error(
        envelope_context.measurement_error(PlanMeasureError::LimitExceeded {
            dimension: PlanMeasureLimit::Nodes,
            observed: 2,
            maximum: 1,
        }),
        QueryControlError::CompilerEnvelopeExceeded,
    );
    assert_eq!(
        envelope.checkpoint(),
        Err(QueryControlError::CompilerEnvelopeExceeded)
    );
    assert_eq!(envelope.consumed(QueryCharge::CompilerWork), 0);
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
