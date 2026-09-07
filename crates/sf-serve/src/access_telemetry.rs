//! Dormant access-decision telemetry contract (ADR-0018 / ADR-0011).

use sf_core::TELEMETRY_TARGET;

use crate::telemetry::SCHEMA;

macro_rules! define_access_decisions {
    ($($variant:ident => $label:literal),+ $(,)?) => {
        /// Closed decision labels adopted by ADR-0018.
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub(crate) enum AccessDecision {
            $($variant),+
        }

        impl AccessDecision {
            pub(crate) const VARIANT_COUNT: usize = [$(stringify!($variant)),+].len();
            pub(crate) const VARIANTS: [Self; Self::VARIANT_COUNT] = [$(Self::$variant),+];

            pub(crate) const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $label),+
                }
            }
        }
    };
}

define_access_decisions! {
    Allow => "allow",
    Deny => "deny",
    Mask => "mask",
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
