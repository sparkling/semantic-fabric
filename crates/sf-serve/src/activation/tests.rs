use super::*;

#[test]
fn activation_and_state_counters_fail_closed_at_exhaustion() {
    let activation = ActivationId(u64::MAX);
    assert!(matches!(
        activation.successor(),
        Err(ActivationError::ActivationIdExhausted { current }) if current == activation
    ));

    let revision = RuntimeStateRevision(u64::MAX);
    assert_eq!(
        revision.successor(),
        Err(ActivationError::StateRevisionExhausted { current: revision })
    );

    let state = RuntimeState::NotReady {
        activation_id: ActivationId::INITIAL,
        revision,
        cause: ReadinessCause::SchemaDrift,
    };
    let (terminal, error) = state.not_ready_transition(ReadinessCause::SourceUnavailable);
    assert_eq!(
        terminal.readiness(),
        RuntimeReadiness::NotReady {
            activation_id: ActivationId::INITIAL,
            revision,
            cause: ReadinessCause::StateRevisionExhausted,
        }
    );
    assert_eq!(
        error,
        Some(ActivationError::StateRevisionExhausted { current: revision })
    );
}
