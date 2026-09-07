//! Unforgeable activation candidate produced only after complete validation.

use crate::snapshot::RuntimeSnapshot;
use crate::RuntimeReadiness;

use super::builder::ValidatedCandidateParts;

/// Whole runtime state whose fallible construction completed off-path.
///
/// The fields and constructor are private to the lifecycle boundary. Runtime
/// publication consumes this value; no raw [`RuntimeSnapshot`] can cross the
/// production activation API.
pub(crate) struct ValidatedRuntimeCandidate {
    expected: RuntimeReadiness,
    snapshot: RuntimeSnapshot,
}

impl ValidatedRuntimeCandidate {
    pub(super) fn from_validated(parts: ValidatedCandidateParts) -> Self {
        let (expected, snapshot) = parts.into_parts();
        Self { expected, snapshot }
    }

    #[cfg(test)]
    pub(super) const fn from_test(expected: RuntimeReadiness, snapshot: RuntimeSnapshot) -> Self {
        Self { expected, snapshot }
    }

    pub(crate) const fn expected(&self) -> RuntimeReadiness {
        self.expected
    }

    pub(crate) fn into_snapshot(self) -> RuntimeSnapshot {
        self.snapshot
    }
}
