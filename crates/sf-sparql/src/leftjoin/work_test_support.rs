use crate::{Error, Result};
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use std::fmt::Debug;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

pub(crate) fn exact_and_short<T: Debug>(raw: T, run: impl Fn(&dyn QueryControl) -> Result<T>) {
    let observed = budget(u64::MAX);
    let value = run(&observed).unwrap();
    assert_eq!(format!("{value:?}"), format!("{raw:?}"));
    let work = observed.consumed(QueryCharge::CompilerWork);
    assert!(work > 0);
    let exact = budget(work);
    assert_eq!(format!("{:?}", run(&exact).unwrap()), format!("{raw:?}"));
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), work);
    let short = budget(work - 1);
    assert!(matches!(
        run(&short),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    assert_eq!(
        short.checkpoint(),
        Err(QueryControlError::CompilerWorkExceeded)
    );
    assert_eq!(observed.consumed(QueryCharge::SourceWork), 0);
}

struct StopAtCharge {
    budget: QueryBudget,
    calls: AtomicUsize,
    stop: usize,
    cause: QueryControlError,
}
impl QueryControl for StopAtCharge {
    fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
        self.budget.checkpoint()
    }
    fn consume(
        &self,
        charge: QueryCharge,
        amount: u64,
    ) -> std::result::Result<(), QueryControlError> {
        self.budget.consume(charge, amount)?;
        if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.stop {
            self.budget.terminate(self.cause);
        }
        Ok(())
    }
    fn terminate(&self, cause: QueryControlError) -> QueryControlError {
        self.budget.terminate(cause)
    }
}

pub(crate) fn every_stop<T: Debug>(run: impl Fn(&dyn QueryControl) -> Result<T>) {
    let observed = StopAtCharge {
        budget: budget(u64::MAX),
        calls: AtomicUsize::new(0),
        stop: usize::MAX,
        cause: QueryControlError::Cancelled,
    };
    run(&observed).unwrap();
    assert!(observed.calls.load(Ordering::SeqCst) > 0);
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for stop in 1..=observed.calls.load(Ordering::SeqCst) {
            let control = StopAtCharge {
                budget: budget(u64::MAX),
                calls: AtomicUsize::new(0),
                stop,
                cause,
            };
            let result = run(&control);
            assert!(
                matches!(&result, Err(Error::QueryControl(e)) if *e == cause),
                "stop {stop}: {result:?}"
            );
            assert_eq!(control.calls.load(Ordering::SeqCst), stop);
            assert_eq!(
                control.terminate(QueryControlError::CompilerWorkExceeded),
                cause
            );
        }
    }
}
