use std::fmt::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use sf_core::query_control::{QueryBudget, QueryLimits};

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

pub(crate) fn query_work(query: &Query) -> u64 {
    let control = budget(u64::MAX);
    plan_key_with_work_control(query, scope(), CompileProfileId::Uncontrolled, &control).unwrap();
    control.consumed(QueryCharge::CompilerWork)
}

fn scope() -> CompileScope {
    super::super::test_scope(
        sf_core::SourceId::new(0).unwrap(),
        sf_sql::Dialect::Sqlite,
        super::super::Epoch(0),
    )
}

#[test]
fn canonical_identity_and_exact_hash_budget_match_all_query_forms() {
    for source in [
        "PREFIX ex: <http://example.test/> SELECT DISTINCT ?s ?o WHERE { ?s ex:p ?o OPTIONAL { ?s ex:q ?q } FILTER(?o != ?q) } ORDER BY ?s LIMIT 5",
        "ASK FROM <http://example.test/graph> WHERE { ?s ?p \"café 東京\" }",
        "CONSTRUCT { ?s <http://example.test/p> ?o } WHERE { ?s <http://example.test/p> ?o }",
        "DESCRIBE ?s WHERE { ?s a <http://example.test/Thing> }",
        "SELECT (?x+1 AS ?a) (?a+2 AS ?b) WHERE { VALUES ?x { 1 2 } }",
        "SELECT (COUNT(*) AS ?n) WHERE { VALUES ?x { 1 2 } }",
        "DESCRIBE <urn:a> <urn:b>",
    ] {
        let query = spargebra::SparqlParser::new().parse_query(source).unwrap();
        let work = query_work(&query);
        for profile in [CompileProfileId::Uncontrolled, CompileProfileId::GovernedV1] {
            let paid = budget(work);
            let key = plan_key_with_work_control(&query, scope(), profile, &paid).unwrap();
            assert_eq!(key, super::super::plan_key_for_profile(&query, scope(), profile));
            assert_eq!(paid.consumed(QueryCharge::CompilerWork), work);
            let short = budget(work - 1);
            assert!(matches!(
                plan_key_with_work_control(&query, scope(), profile, &short),
                Err(crate::Error::QueryControl(QueryControlError::CompilerWorkExceeded))
            ));
            assert_eq!(short.consumed(QueryCharge::CompilerWork), work - key.canonical.len() as u64,
                "N-1 fails before the one full-content hash");
        }
    }
}

#[test]
fn fragments_growth_and_slack_are_prepaid_independently_of_allocator_capacity() {
    let control = budget(u64::MAX);
    let calls = AtomicUsize::new(0);
    let mut writer = BoundedWriter::new(usize::MAX, |output: &mut String, additional| {
        let index = calls.fetch_add(1, Ordering::SeqCst);
        match index {
            0 => assert_eq!(control.consumed(QueryCharge::CompilerWork), 1 + 64),
            1 => assert_eq!(
                control.consumed(QueryCharge::CompilerWork),
                65 + 64 + 128 + 64
            ),
            _ => panic!("unexpected growth"),
        }
        // Deliberate excess capacity must not change the logical work schedule.
        output.try_reserve_exact(additional + 1024).map_err(|_| ())
    });
    writer.control = Some(&control);
    for _ in 0..65 {
        writer.write_str("x").unwrap();
    }
    assert_eq!(writer.output, "x".repeat(65));
    assert_eq!(writer.requested_capacity, 128);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(control.consumed(QueryCharge::CompilerWork), 321);
}

#[test]
fn utf8_and_growth_failure_never_append_unpaid_payload() {
    for (allowance, succeeds) in [(65, false), (66, true)] {
        let control = budget(allowance);
        let calls = AtomicUsize::new(0);
        let mut writer = BoundedWriter::new(usize::MAX, |output: &mut String, additional| {
            calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(control.consumed(QueryCharge::CompilerWork), 66);
            output.try_reserve_exact(additional).map_err(|_| ())
        });
        writer.control = Some(&control);
        let result = writer.write_str("λ");
        assert_eq!(result.is_ok(), succeeds);
        assert_eq!(writer.output, if succeeds { "λ" } else { "" });
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(succeeds));
        if !succeeds {
            assert_eq!(
                writer.finish(result),
                Err(BoundedCacheKeyError::Control(
                    QueryControlError::CompilerWorkExceeded
                ))
            );
        }
    }
}

struct CancelOnCharge(QueryBudget);
impl QueryControl for CancelOnCharge {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        self.0.checkpoint()
    }
    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), QueryControlError> {
        self.0.consume(charge, amount)?;
        self.0.terminate(QueryControlError::Cancelled);
        Ok(())
    }
    fn terminate(&self, cause: QueryControlError) -> QueryControlError {
        self.0.terminate(cause)
    }
}

#[test]
fn cancellation_and_allocation_errors_keep_exact_redacted_causes() {
    let cancelled = CancelOnCharge(budget(u64::MAX));
    let mut writer =
        BoundedWriter::new(usize::MAX, |_: &mut String, _| panic!("unpaid allocation"));
    writer.control = Some(&cancelled);
    let result = writer.write_str("private query");
    assert_eq!(
        writer.finish(result),
        Err(BoundedCacheKeyError::Control(QueryControlError::Cancelled))
    );
    let control = budget(u64::MAX);
    let mut writer = BoundedWriter::new(usize::MAX, |_: &mut String, _| Err(()));
    writer.control = Some(&control);
    let result = writer.write_str("private query");
    assert_eq!(
        writer.finish(result),
        Err(BoundedCacheKeyError::AllocationFailed)
    );
}

#[test]
fn structural_envelope_rejects_before_recursive_display() {
    let query = spargebra::SparqlParser::new()
        .parse_query("SELECT * WHERE {}")
        .unwrap();
    let mut query = query;
    let Query::Select { pattern, .. } = &mut query else {
        panic!()
    };
    for _ in 0..130 {
        *pattern = spargebra::algebra::GraphPattern::Distinct {
            inner: Box::new(pattern.clone()),
        };
    }
    let control = budget(u64::MAX);
    assert!(matches!(
        plan_key_with_work_control(&query, scope(), CompileProfileId::Uncontrolled, &control),
        Err(crate::Error::QueryControl(
            QueryControlError::CompilerEnvelopeExceeded
        ))
    ));
    assert!(
        control.consumed(QueryCharge::CompilerWork) > 1,
        "the rejecting iterative walk is also metered"
    );
}

#[test]
fn preparation_is_prepaid_before_output_and_preserves_terminal_cause() {
    use crate::compile_envelope::algebra::AlgebraEnvelopeV1;
    let query = spargebra::SparqlParser::new()
        .parse_query("SELECT (?x+1 AS ?y) WHERE { VALUES ?x { 1 } }")
        .unwrap();
    let walk = budget(u64::MAX);
    let envelope = AlgebraEnvelopeV1::validate_with_control(&query, &walk).unwrap();
    let preparation = budget(u64::MAX);
    envelope.charge_canonical_preparation(&preparation).unwrap();
    let walked = 1 + walk.consumed(QueryCharge::CompilerWork);
    let prepared = walked + preparation.consumed(QueryCharge::CompilerWork);
    let short = budget(prepared - 1);
    assert!(matches!(
        plan_key_with_work_control(&query, scope(), CompileProfileId::Uncontrolled, &short),
        Err(crate::Error::QueryControl(
            QueryControlError::CompilerWorkExceeded
        ))
    ));
    assert_eq!(
        short.consumed(QueryCharge::CompilerWork),
        walked,
        "rejection precedes the atomic preparation charge, output and hash"
    );

    struct StopAfterPreparation(QueryBudget, u64, QueryControlError);
    impl QueryControl for StopAfterPreparation {
        fn checkpoint(&self) -> Result<(), QueryControlError> {
            self.0.checkpoint()
        }
        fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), QueryControlError> {
            self.0.consume(charge, amount)?;
            if self.0.consumed(QueryCharge::CompilerWork) == self.1 {
                self.0.terminate(self.2);
            }
            Ok(())
        }
        fn terminate(&self, cause: QueryControlError) -> QueryControlError {
            self.0.terminate(cause)
        }
    }
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        let control = StopAfterPreparation(budget(u64::MAX), prepared, cause);
        assert!(matches!(
            plan_key_with_work_control(&query, scope(), CompileProfileId::Uncontrolled, &control),
            Err(crate::Error::QueryControl(actual)) if actual == cause
        ));
        assert_eq!(
            control.0.consumed(QueryCharge::CompilerWork),
            prepared,
            "the first output fragment and hash must not run after cancellation"
        );
    }
}
