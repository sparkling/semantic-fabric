use super::*;
use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

#[test]
fn cache_identity_paid_copy_and_rewrite_have_exact_terminal_boundaries() {
    for source in [
        "SELECT (COUNT(*) AS ?count) WHERE { VALUES ?x { 1 2 } }",
        "DESCRIBE <urn:a> <urn:b>",
    ] {
        let query = spargebra::SparqlParser::new().parse_query(source).unwrap();
        let before = format!("{query:?}");
        let envelope = AlgebraEnvelopeV1::validate(&query).unwrap();
        let paid = budget(u64::MAX);
        let expected = controlled(&query, &envelope, &paid).unwrap();
        let work = paid.consumed(QueryCharge::CompilerWork);
        assert!(work > 0);
        assert_eq!(
            controlled(&query, &envelope, &budget(work)).unwrap(),
            expected
        );
        let short = budget(work - 1);
        assert!(matches!(
            controlled(&query, &envelope, &short),
            Err(crate::Error::QueryControl(
                QueryControlError::CompilerWorkExceeded
            ))
        ));
        assert_eq!(
            short.checkpoint(),
            Err(QueryControlError::CompilerWorkExceeded)
        );
        assert_eq!(format!("{query:?}"), before);
    }
}

#[test]
fn cache_identity_every_copy_and_rewrite_charge_preserves_the_first_terminal() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Interrupt {
        budget: QueryBudget,
        at: usize,
        calls: AtomicUsize,
        cause: QueryControlError,
    }
    impl QueryControl for Interrupt {
        fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
            self.budget.checkpoint()
        }
        fn consume(
            &self,
            charge: QueryCharge,
            amount: u64,
        ) -> std::result::Result<(), QueryControlError> {
            self.budget.consume(charge, amount)?;
            if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.at {
                self.budget.terminate(self.cause);
            }
            Ok(())
        }
        fn terminate(&self, cause: QueryControlError) -> QueryControlError {
            self.budget.terminate(cause)
        }
    }
    for source in [
        "SELECT (COUNT(*) AS ?n) WHERE { VALUES ?x { 1 } }",
        "DESCRIBE <urn:a>",
    ] {
        let query = spargebra::SparqlParser::new().parse_query(source).unwrap();
        let before = format!("{query:?}");
        let envelope = AlgebraEnvelopeV1::validate(&query).unwrap();
        let observer = Interrupt {
            budget: budget(u64::MAX),
            at: usize::MAX,
            calls: AtomicUsize::new(0),
            cause: QueryControlError::Cancelled,
        };
        controlled(&query, &envelope, &observer).unwrap();
        let charges = observer.calls.load(Ordering::SeqCst);
        assert!(
            charges > 50,
            "exercise traversal, clone, allocation, comparison and substitution"
        );
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            for at in 1..=charges {
                let observer = Interrupt {
                    budget: budget(u64::MAX),
                    at,
                    calls: AtomicUsize::new(0),
                    cause,
                };
                assert!(
                    matches!(controlled(&query, &envelope, &observer), Err(crate::Error::QueryControl(actual)) if actual == cause)
                );
                assert_eq!(
                    observer.calls.load(Ordering::SeqCst),
                    at,
                    "no paid operation after terminal at {at}"
                );
                assert_eq!(
                    observer.terminate(QueryControlError::AccountingOverflow),
                    cause
                );
                assert_eq!(format!("{query:?}"), before);
            }
        }
    }
}
