use super::*;
use sf_core::query_control::{QueryBudget, QueryLimits, UncontrolledQueryControl};

fn fixture() -> ParserRuntime {
    // Only scope restoration is exercised with this non-parser fixture.
    from_prepared_for_test(
        PreparedParserExecutable::from_file_for_evidence(std::fs::File::open("/bin/cat").unwrap())
            .unwrap(),
    )
}

fn budget() -> QueryBudget {
    QueryBudget::new(QueryLimits::new(100, 100, 100, 100))
}

#[test]
fn request_scope_restores_after_success_error_cancellation_and_unwinding() {
    let runtime = fixture();
    let outer = Arc::new(budget());
    runtime
        .with_request(outer.clone(), || {
            assert_eq!(poll_timeout(100), 10);
            std::thread::spawn(|| assert_eq!(poll_timeout(100), 100))
                .join()
                .unwrap();
            runtime.with_request(Arc::new(budget()), || Ok(()))?;
            let error: crate::Result<()> = runtime.with_request(Arc::new(budget()), || {
                Err(crate::Error::Parse("fixture".into()))
            });
            assert!(matches!(error, Err(crate::Error::Parse(_))));
            let inner = Arc::new(budget());
            let error = runtime.with_request(inner.clone(), || {
                inner.terminate(QueryControlError::Cancelled);
                Ok(())
            });
            assert!(matches!(
                error,
                Err(crate::Error::QueryControl(QueryControlError::Cancelled))
            ));
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _: crate::Result<()> =
                    runtime.with_request(Arc::new(budget()), || panic!("fixture panic"));
            }));
            assert!(panic.is_err());
            REQUEST.with(|slot| {
                let scope = slot.borrow();
                let expected: Arc<dyn QueryControl> = outer.clone();
                assert!(Arc::ptr_eq(&scope.as_ref().unwrap().control, &expected));
            });
            checkpoint().unwrap();
            Ok(())
        })
        .unwrap();
    assert_eq!(poll_timeout(100), 100);
    assert!(REQUEST.with(|slot| slot.borrow().is_none()));
}

#[test]
fn terminal_request_does_not_enter_compiler_scope() {
    let runtime = fixture();
    let control = Arc::new(budget());
    control.terminate(QueryControlError::Cancelled);
    let result: crate::Result<()> = runtime.with_request(control, || panic!("must not start work"));
    assert!(matches!(
        result,
        Err(crate::Error::QueryControl(QueryControlError::Cancelled))
    ));
    assert!(REQUEST.with(|slot| slot.borrow().is_none()));
}

#[test]
fn syntax_envelope_deadline_and_infrastructure_errors_are_distinct_and_redacted() {
    use super::super::parse_protocol::ParseRejectionV1 as Rejection;
    let control = UncontrolledQueryControl;
    assert!(matches!(
        public_error(SupervisorError::ParseRejected(Rejection::Syntax), &control),
        crate::Error::Parse(_)
    ));
    for (error, expected) in [
        (
            SupervisorError::ParseRejected(Rejection::QueryEnvelope),
            QueryControlError::CompilerEnvelopeExceeded,
        ),
        (
            SupervisorError::ParseRejected(Rejection::ResourceExhausted),
            QueryControlError::CompilerResourceExhausted,
        ),
        (
            SupervisorError::DeadlineExceeded,
            QueryControlError::DeadlineExceeded,
        ),
    ] {
        assert!(
            matches!(public_error(error, &control), crate::Error::QueryControl(actual) if actual == expected)
        );
    }
    for error in [
        SupervisorError::InvalidState("private details"),
        SupervisorError::InvalidExecutable("private path"),
    ] {
        let mapped = public_error(error, &control);
        assert!(matches!(mapped, crate::Error::Mapping(_)));
        assert_eq!(mapped.to_string(), "mapping error: isolated parser failed");
    }
}
