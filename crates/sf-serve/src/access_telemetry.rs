//! Dormant access-decision telemetry contract (ADR-0018 / ADR-0011).

use sf_core::TELEMETRY_TARGET;

const SCHEMA: &str = "semantic-fabric.telemetry.v1";

/// Closed decision labels adopted by ADR-0018.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccessDecision {
    Allow,
    Deny,
    Mask,
}

impl AccessDecision {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Mask => "mask",
        }
    }
}

/// Emit only the closed decision label on M3's exact telemetry target.
///
/// There is intentionally no request-context or policy payload parameter and
/// no call site: the function records a decision made elsewhere; it neither
/// makes nor enforces one.
pub(crate) fn record(decision: AccessDecision) {
    tracing::info!(
        target: TELEMETRY_TARGET,
        schema = SCHEMA,
        event = "security.access_decision",
        decision = decision.as_str(),
    );
}

#[cfg(test)]
#[path = "access_telemetry_tests.rs"]
mod tests;
