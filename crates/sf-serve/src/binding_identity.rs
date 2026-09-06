//! Unforgeable process-local identity for one runtime binding.

use std::sync::Arc;

/// Clones preserve one binding's identity; independent construction never does.
#[derive(Clone)]
pub(crate) struct RuntimeBindingIdentity(Arc<()>);

impl RuntimeBindingIdentity {
    pub(crate) fn fresh() -> Self {
        Self(Arc::new(()))
    }

    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
