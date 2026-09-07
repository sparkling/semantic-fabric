//! Fail-closed serve-lane resource admission (ADR-0038 M1).

use sf_sparql::resource_profile::SourceSizedState;
use sf_sparql::Plan;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum AdmissionReason {
    #[error("ORDER BY expression evaluation is not production-exact")]
    InexactOrderExpression,
    #[error("source-sized Rust state is not admitted: {0}")]
    SourceSized(SourceSizedState),
}

/// Typed rejection mapped to 501 before backend acquisition or execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("plan is not admitted: {reason}")]
pub(crate) struct ResourceUnsupported {
    reason: AdmissionReason,
}

impl ResourceUnsupported {
    pub(crate) const fn reason(self) -> AdmissionReason {
        self.reason
    }
}

/// Admit only plans whose current executor path has no source-sized Rust state.
/// The profile returns every reachable kind; the stable enum order selects the
/// primary rejection code while the complete vector remains testable at the plan
/// boundary.
pub(crate) fn admit(plan: &Plan, maximum_order_rows: usize) -> Result<(), ResourceUnsupported> {
    // The existing generic expression evaluator is a raw/development surface;
    // it is not yet a SPARQL-error-exact production evaluator. Keep finite
    // source-backed windows from making it newly reachable through serving.
    // LIMIT 0 is safe because execution returns before evaluating any key.
    if plan.limit != Some(0) && plan.order.iter().any(|key| key.expr.is_some()) {
        return Err(ResourceUnsupported {
            reason: AdmissionReason::InexactOrderExpression,
        });
    }
    match plan
        .source_sized_states_with_order_window(maximum_order_rows)
        .into_iter()
        .next()
    {
        Some(state) => Err(ResourceUnsupported {
            reason: AdmissionReason::SourceSized(state),
        }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_sparql::parse_and_translate;
    use sf_sql::Dialect;

    #[test]
    fn finite_expression_order_has_its_own_typed_reason() {
        let plan = parse_and_translate(
            "SELECT ?x WHERE { VALUES ?x { \"a\" } } ORDER BY STRLEN(?x) LIMIT 1",
            &[],
            Dialect::Sqlite,
        )
        .unwrap();
        assert_eq!(
            admit(&plan, 1).unwrap_err().reason(),
            AdmissionReason::InexactOrderExpression
        );
    }
}
