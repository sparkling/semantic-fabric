//! Provider-neutral, non-authorizing request security identity (ADR-0018).
//!
//! These values carry only precomputed, fixed-width identities. They do not
//! authenticate a caller, interpret attributes, select a policy, or authorize
//! data. A runtime snapshot owns the [`PolicySnapshotId`]; each request must
//! supply its own [`SubjectIdentity`] and [`RequestAttributesIdentity`].

use std::fmt;

use sha2::{Digest, Sha256};

const IDENTITY_BYTES: usize = 32;
const CACHE_IDENTITY_DOMAIN: &[u8] = b"semantic-fabric/security-cache-identity/v1";

/// A rejected opaque identity.
///
/// Variants intentionally carry no rejected bytes or caller-controlled text.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SecurityIdentityError {
    #[error("security policy snapshot identity must be explicitly non-zero")]
    EmptyPolicySnapshot,
    #[error("security subject identity must be explicitly non-zero")]
    EmptySubject,
    #[error("security request-attributes identity must be explicitly non-zero")]
    EmptyRequestAttributes,
}

macro_rules! opaque_identity {
    ($(#[$meta:meta])* $name:ident, $error:expr, $display:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Eq, Hash, PartialEq)]
        pub struct $name([u8; IDENTITY_BYTES]);

        impl $name {
            /// Construct from a caller-owned canonical digest.
            ///
            /// The all-zero value is reserved as an invalid sentinel so it
            /// cannot become an implicit anonymous or unrestricted identity.
            pub fn from_digest(
                digest: [u8; IDENTITY_BYTES],
            ) -> Result<Self, SecurityIdentityError> {
                if digest == [0; IDENTITY_BYTES] {
                    return Err($error);
                }
                Ok(Self(digest))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "([redacted])"))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!($display, " [redacted]"))
            }
        }
    };
}

opaque_identity!(
    /// Canonical identity of the policy fixed by one immutable runtime snapshot.
    PolicySnapshotId,
    SecurityIdentityError::EmptyPolicySnapshot,
    "security policy snapshot"
);

opaque_identity!(
    /// Canonical identity assigned to a subject by an external request boundary.
    ///
    /// This value is not authentication proof, carries no provider subject, and
    /// is scoped to one request.
    SubjectIdentity,
    SecurityIdentityError::EmptySubject,
    "security subject"
);

opaque_identity!(
    /// Canonical identity of the request's provider-owned attribute set.
    ///
    /// Semantic Fabric does not interpret the attributes or define their taxonomy.
    RequestAttributesIdentity,
    SecurityIdentityError::EmptyRequestAttributes,
    "security request attributes"
);

/// A derived cache partition that contains no raw policy, subject, or attribute
/// material and cannot be constructed independently of a [`SecurityContext`].
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct SecurityCacheIdentity([u8; IDENTITY_BYTES]);

impl fmt::Debug for SecurityCacheIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecurityCacheIdentity([redacted])")
    }
}

impl fmt::Display for SecurityCacheIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("security cache identity [redacted]")
    }
}

/// Fixed-width request security context.
///
/// Construction is deliberately explicit: there is no `Default`, anonymous,
/// unrestricted, or omitted-attributes value.
///
/// ```compile_fail
/// use sf_core::security_context::SecurityContext;
/// let _implicit: SecurityContext = Default::default();
/// ```
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct SecurityContext {
    policy_snapshot: PolicySnapshotId,
    subject: SubjectIdentity,
    request_attributes: RequestAttributesIdentity,
}

impl SecurityContext {
    /// Bind one request identity to exactly one snapshot-owned policy identity.
    pub const fn new(
        policy_snapshot: PolicySnapshotId,
        subject: SubjectIdentity,
        request_attributes: RequestAttributesIdentity,
    ) -> Self {
        Self {
            policy_snapshot,
            subject,
            request_attributes,
        }
    }

    pub const fn policy_snapshot(self) -> PolicySnapshotId {
        self.policy_snapshot
    }

    pub const fn subject(self) -> SubjectIdentity {
        self.subject
    }

    pub const fn request_attributes(self) -> RequestAttributesIdentity {
        self.request_attributes
    }

    /// Derive a domain-separated cache partition from every context dimension.
    ///
    /// The output is deterministic identity only. It grants no authorization.
    #[must_use]
    pub fn cache_identity(self) -> SecurityCacheIdentity {
        let mut hasher = Sha256::new();
        update_framed(&mut hasher, 0, CACHE_IDENTITY_DOMAIN);
        update_framed(&mut hasher, 1, &self.policy_snapshot.0);
        update_framed(&mut hasher, 2, &self.subject.0);
        update_framed(&mut hasher, 3, &self.request_attributes.0);
        SecurityCacheIdentity(hasher.finalize().into())
    }
}

impl fmt::Debug for SecurityContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "SecurityContext { policy_snapshot: [redacted], subject: [redacted], \
             request_attributes: [redacted] }",
        )
    }
}

impl fmt::Display for SecurityContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("security context [redacted]")
    }
}

fn update_framed(hasher: &mut Sha256, tag: u8, value: &[u8]) {
    hasher.update([tag]);
    hasher.update(
        u64::try_from(value.len())
            .expect("fixed security identity input length fits in u64")
            .to_be_bytes(),
    );
    hasher.update(value);
}
