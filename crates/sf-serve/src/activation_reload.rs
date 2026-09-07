//! The authored coordinator has its own sealed publication capability.
use super::*;

impl RuntimeManager {
    pub(crate) fn activate_authored(
        &self,
        _authority: &crate::reload::ReloadAuthority,
        candidate: crate::reload::AuthoredCandidate,
    ) -> Result<ActivationId, ActivationError> {
        let (expected, snapshot) = candidate.into_parts();
        self.publish(expected, snapshot)
    }

    pub(crate) fn fence_authored(
        &self,
        _authority: &crate::reload::ReloadAuthority,
        expected: RuntimeReadiness,
        cause: ReadinessCause,
    ) -> Result<RuntimeReadiness, ActivationError> {
        if matches!(
            expected,
            RuntimeReadiness::NotReady {
                cause: ReadinessCause::Administrative | ReadinessCause::StateRevisionExhausted,
                ..
            }
        ) {
            return Err(ActivationError::ShuttingDown);
        }
        self.fence(expected, cause)
    }
}
