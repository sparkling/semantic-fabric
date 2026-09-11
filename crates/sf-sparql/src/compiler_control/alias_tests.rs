use crate::compiler_control::CompileContext;
use crate::leftjoin::optional_work_test_support::{budget, every_stop};
use crate::{CompilerWorkMode, Error};
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};

#[test]
fn alias_work_exact_ranges_charge_only_requested_identifiers() {
    for count in [1, 2, 64] {
        let c = budget(count as u64);
        let mode = CompilerWorkMode::Metered(CompileContext::new(&c));
        assert_eq!(mode.alias_range(7, count).unwrap(), 7..7 + count);
        assert_eq!(c.consumed(QueryCharge::CompilerWork), count as u64);
        let short = budget(count as u64 - 1);
        assert!(matches!(
            CompilerWorkMode::Metered(CompileContext::new(&short)).alias_range(7, count),
            Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
        ));
        assert_eq!(short.consumed(QueryCharge::CompilerWork), 0);
        every_stop(|c| CompilerWorkMode::Metered(CompileContext::new(c)).alias_range(7, count));
    }
}

#[test]
fn alias_work_impossible_ranges_are_sticky_in_both_modes() {
    for (start, count) in [(usize::MAX, 1), (usize::MAX - 1, 2)] {
        let c = budget(u64::MAX);
        for mode in [
            CompilerWorkMode::Uncontrolled,
            CompilerWorkMode::Metered(CompileContext::new(&c)),
        ] {
            assert!(matches!(
                mode.alias_range(start, count),
                Err(Error::QueryControl(QueryControlError::AccountingOverflow))
            ));
        }
        assert_eq!(c.checkpoint(), Err(QueryControlError::AccountingOverflow));
        assert_eq!(c.consumed(QueryCharge::CompilerWork), 0);
    }
    let c = budget(2);
    assert_eq!(
        CompilerWorkMode::Metered(CompileContext::new(&c))
            .alias_range(usize::MAX - 2, 2)
            .unwrap(),
        usize::MAX - 2..usize::MAX
    );
}

#[test]
fn alias_work_prior_terminal_cause_wins_over_overflow() {
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        let c = budget(2);
        c.terminate(cause);
        assert!(
            matches!(CompilerWorkMode::Metered(CompileContext::new(&c)).alias_range(usize::MAX, 2), Err(Error::QueryControl(e)) if e == cause)
        );
        assert_eq!(c.consumed(QueryCharge::CompilerWork), 0);
    }
}
