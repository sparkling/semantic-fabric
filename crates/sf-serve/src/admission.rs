//! Fail-closed serve-lane resource admission (ADR-0038 M1).

use sf_core::query_control::{QueryCharge, QueryControl};
use sf_sparql::resource_profile::SourceSizedState;
use sf_sparql::Plan;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum AdmissionReason {
    #[error("ORDER BY expression evaluation is not production-exact")]
    InexactOrderExpression,
    #[error("source-sized Rust state is not admitted: {0}")]
    SourceSized(SourceSizedState),
}

/// Admit only plans whose current executor path has no source-sized Rust state.
/// The profile returns every reachable kind; the stable enum order selects the
/// primary rejection code while the complete vector remains testable at the plan
/// boundary.
pub(crate) fn admit(
    plan: &Plan,
    maximum_order_rows: usize,
    control: &dyn QueryControl,
) -> sf_sparql::Result<()> {
    // The existing generic expression evaluator is a raw/development surface;
    // it is not yet a SPARQL-error-exact production evaluator. Keep finite
    // source-backed windows from making it newly reachable through serving.
    // LIMIT 0 is safe because execution returns before evaluating any key.
    control.checkpoint()?;
    if plan.limit != Some(0) {
        for key in &plan.order {
            control.consume(QueryCharge::CompilerWork, 1)?;
            control.checkpoint()?;
            if key.expr.is_some() {
                return Err(sf_sparql::Error::Unsupported(
                    AdmissionReason::InexactOrderExpression.to_string(),
                ));
            }
        }
    }
    match plan
        .source_sized_states_with_order_window_and_work_control(maximum_order_rows, control)?
        .into_iter()
        .next()
    {
        Some(state) => Err(sf_sparql::Error::Unsupported(
            AdmissionReason::SourceSized(state).to_string(),
        )),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::query_control::{QueryBudget, QueryControlError, QueryLimits};
    use sf_sparql::parse_and_translate;
    use sf_sql::Dialect;

    fn budget(units: u64) -> QueryBudget {
        QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX))
    }

    #[test]
    fn terminal_control_is_not_relabelled_unsupported() {
        let mut plan = parse_and_translate(
            "SELECT ?x WHERE { VALUES ?x { 1 } } ORDER BY STRLEN(?x) LIMIT 1",
            &[],
            Dialect::Sqlite,
        )
        .unwrap();
        assert!(matches!(
            admit(&plan, 1, &budget(0)),
            Err(sf_sparql::Error::QueryControl(
                QueryControlError::CompilerWorkExceeded
            ))
        ));
        assert!(matches!(
            admit(&plan, 1, &budget(1)),
            Err(sf_sparql::Error::Unsupported(_))
        ));
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            let control = budget(u64::MAX);
            control.terminate(cause);
            assert!(
                matches!(admit(&plan, 1, &control), Err(sf_sparql::Error::QueryControl(actual)) if actual == cause)
            );
        }
        // LIMIT 0 skips expression inspection, but still pays classification.
        plan.limit = Some(0);
        admit(&plan, 0, &budget(u64::MAX)).unwrap();
    }

    #[test]
    fn finite_expression_order_has_its_own_typed_reason() {
        let plan = parse_and_translate(
            "SELECT ?x WHERE { VALUES ?x { \"a\" } } ORDER BY STRLEN(?x) LIMIT 1",
            &[],
            Dialect::Sqlite,
        )
        .unwrap();
        assert!(
            matches!(admit(&plan, 1, &sf_core::query_control::UncontrolledQueryControl),
            Err(sf_sparql::Error::Unsupported(reason)) if reason == AdmissionReason::InexactOrderExpression.to_string())
        );
    }
}
