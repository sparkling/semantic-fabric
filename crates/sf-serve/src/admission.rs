//! Fail-closed serve-lane resource admission (ADR-0038 M1).

use sf_sparql::resource_profile::SourceSizedState;
use sf_sparql::Plan;

/// A typed rejection for a compiled plan that would enter a source-sized Rust
/// fallback. `sf-serve` maps this to 501 before backend acquisition or execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("source-sized Rust state is not admitted: {state}")]
pub(crate) struct ResourceUnsupported {
    state: SourceSizedState,
}

impl ResourceUnsupported {
    pub(crate) const fn state(self) -> SourceSizedState {
        self.state
    }
}

/// Admit only plans whose current executor path has no source-sized Rust state.
/// The profile returns every reachable kind; the stable enum order selects the
/// primary rejection code while the complete vector remains testable at the plan
/// boundary.
pub(crate) fn admit(plan: &Plan) -> Result<(), ResourceUnsupported> {
    match plan.source_sized_states().into_iter().next() {
        Some(state) => Err(ResourceUnsupported { state }),
        None => Ok(()),
    }
}
