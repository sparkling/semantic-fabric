//! Opaque, process-local correlation identities generated at request ingress.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use serde::Serialize;

static NEXT_NONCE: AtomicU64 = AtomicU64::new(1);
static HASHERS: OnceLock<(RandomState, RandomState)> = OnceLock::new();

/// Generated support identifier. Construction is private so telemetry never
/// accepts request-controlled correlation text.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct CorrelationId(String);

impl CorrelationId {
    pub(crate) fn generate() -> Self {
        let hashers = HASHERS.get_or_init(|| (RandomState::new(), RandomState::new()));
        let nonce = NEXT_NONCE.fetch_add(1, Ordering::Relaxed);
        let opaque = |state: &RandomState, domain: u8| {
            let mut hasher = state.build_hasher();
            domain.hash(&mut hasher);
            nonce.hash(&mut hasher);
            hasher.finish()
        };
        Self(format!(
            "sf-{:016x}-{:016x}",
            opaque(&hashers.0, 0),
            opaque(&hashers.1, 1)
        ))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
pub(crate) fn is_generated(value: &str) -> bool {
    value.len() == 36
        && value.as_bytes().get(..3) == Some(b"sf-")
        && value.as_bytes().get(19) == Some(&b'-')
        && value.bytes().enumerate().all(|(index, byte)| {
            index < 3 || index == 19 || byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
        })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn generated_identities_have_one_opaque_shape_and_are_unique() {
        let identities: HashSet<_> = (0..4_096).map(|_| CorrelationId::generate().0).collect();
        assert_eq!(identities.len(), 4_096);
        assert!(identities.iter().all(|identity| is_generated(identity)));
    }
}
