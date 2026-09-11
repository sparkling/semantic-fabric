use super::*;
use crate::iq::TermDef;
use crate::unify::Unify;
use sf_core::query_control::{QueryBudget, QueryLimits};

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

#[test]
fn unification_arithmetic_and_refusal_are_sticky_before_reservation() {
    for (left, right) in [(u64::MAX, 1), (u64::MAX, 0), ((u64::MAX - 511) / 64, 0)] {
        let control = budget(u64::MAX);
        assert!(matches!(
            CompileContext::new(&control).reserve_unification_work(left, right),
            Err(Error::QueryControl(QueryControlError::AccountingOverflow))
        ));
        assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
        assert_eq!(
            control.checkpoint(),
            Err(QueryControlError::AccountingOverflow)
        );
    }
}

#[test]
fn unification_measurement_and_logical_operation_reservation_are_separate() {
    let left = TermDef::Const(sf_core::Literal::from("東京").into());
    let right = TermDef::Const(sf_core::Literal::from("東京").into());
    let measured = budget(u64::MAX);
    let cx = CompileContext::new(&measured);
    let a = cx
        .measure_root(CompilerCloneRootV1::TermDef(&left))
        .unwrap();
    let b = cx
        .measure_root(CompilerCloneRootV1::TermDef(&right))
        .unwrap();
    let prefix = measured.consumed(QueryCharge::CompilerWork);
    let expected = prefix + 512 + 64 * (a.deep_clone_work + b.deep_clone_work);
    let exact = budget(expected);
    assert!(
        matches!(CompileContext::new(&exact).unify_terms(&left, &right).unwrap(), Unify::Sat(conds) if conds.is_empty())
    );
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), expected);
    let short = budget(expected - 1);
    assert!(matches!(
        CompileContext::new(&short).unify_terms(&left, &right),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    assert_eq!(short.consumed(QueryCharge::CompilerWork), prefix);
}
