use super::*;

#[test]
fn generated_error_cardinality_includes_the_distinct_compiler_envelope_reason() {
    assert_eq!(QueryControlError::VARIANT_COUNT, 9);
    assert_eq!(QueryControlError::VARIANTS.len(), 9);
    assert_eq!(
        QueryControlError::VARIANTS
            .iter()
            .filter(|error| **error == QueryControlError::CompilerEnvelopeExceeded)
            .count(),
        1
    );
    assert_eq!(
        QueryControlError::VARIANTS
            .iter()
            .filter(|error| **error == QueryControlError::CompilerResourceExhausted)
            .count(),
        1
    );
}

#[test]
fn compiler_envelope_first_cause_is_sticky_through_a_trait_object() {
    let budget = QueryBudget::new(QueryLimits::new(10, 10, 10, 10));
    let control: &dyn QueryControl = &budget;

    assert_eq!(
        control.terminate(QueryControlError::CompilerEnvelopeExceeded),
        QueryControlError::CompilerEnvelopeExceeded
    );
    assert_eq!(
        control.terminate(QueryControlError::CompilerWorkExceeded),
        QueryControlError::CompilerEnvelopeExceeded
    );
    assert_eq!(
        control.consume(QueryCharge::CompilerWork, 1),
        Err(QueryControlError::CompilerEnvelopeExceeded)
    );
    assert_eq!(budget.consumed(QueryCharge::CompilerWork), 0);
}

#[test]
fn an_existing_terminal_reason_wins_over_a_later_envelope_rejection() {
    let budget = QueryBudget::new(QueryLimits::new(10, 10, 10, 10));
    let control: &dyn QueryControl = &budget;

    assert_eq!(
        control.terminate(QueryControlError::Cancelled),
        QueryControlError::Cancelled
    );
    assert_eq!(
        control.terminate(QueryControlError::CompilerEnvelopeExceeded),
        QueryControlError::Cancelled
    );
    assert_eq!(control.checkpoint(), Err(QueryControlError::Cancelled));
}
