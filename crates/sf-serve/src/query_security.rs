//! Query-level admission, not row-level ABAC, sensitivity enforcement or source RLS.
use axum::http::{header, HeaderMap};
use sf_core::security_context::{
    PolicySnapshotId, RequestAttributesIdentity, SecurityContext, SubjectIdentity,
};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::budget::RequestBudget;
use crate::problem::{ProblemCode, StartupCause};
use crate::ServeError;

/// Explicit immutable service-lifetime policy. Rotation requires a new server;
/// a live request never consults a mutable credential or a newer policy.
#[derive(Clone, Debug, Default)]
pub enum QueryAdmission {
    /// No query may execute until the embedding selects an admission profile.
    #[default]
    Deny,
    /// Anyone may read all mapped data. Only for explicitly unprotected development.
    UnrestrictedDevelopment,
    /// The configured service principal may read all mapped data. Not end-user identity.
    Bearer(BearerQueryAdmission),
}

/// A bounded credential digest and provider-neutral, redacted request identity.
#[derive(Clone)]
pub struct BearerQueryAdmission {
    digest: [u8; 32],
    context: SecurityContext,
}

impl std::fmt::Debug for BearerQueryAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BearerQueryAdmission([REDACTED])")
    }
}

impl BearerQueryAdmission {
    /// Configure one service principal, with a distinct identity on credential rotation.
    /// Supply a high-entropy random token; this validates syntax, not randomness.
    pub fn for_service_principal(credential: &str) -> Result<Self, ServeError> {
        if !(32..=1024).contains(&credential.len()) || !valid_token(credential.as_bytes()) {
            return Err(configuration_error());
        }
        let digest: [u8; 32] = Sha256::digest(credential.as_bytes()).into();
        let identity = |domain: &[u8]| -> [u8; 32] {
            let mut hash = Sha256::new();
            hash.update(domain);
            hash.update(digest);
            hash.finalize().into()
        };
        let context = SecurityContext::new(
            PolicySnapshotId::from_digest(identity(b"sf-query-read-all-policy-v1\0"))
                .map_err(|_| configuration_error())?,
            SubjectIdentity::from_digest(identity(b"sf-query-service-principal-v1\0"))
                .map_err(|_| configuration_error())?,
            RequestAttributesIdentity::from_digest(identity(b"sf-query-bearer-attributes-v1\0"))
                .map_err(|_| configuration_error())?,
        );
        Ok(Self { digest, context })
    }

    /// Read a credential through an environment reference, never a literal CLI argument.
    /// Errors and Debug output contain neither the reference nor its value.
    pub fn from_env(name: &str) -> Result<Self, ServeError> {
        let valid = !name.is_empty()
            && name.len() <= 128
            && name.bytes().enumerate().all(|(i, c)| {
                c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit())
            });
        if !valid {
            return Err(configuration_error());
        }
        let credential = std::env::var(name).map_err(|_| configuration_error())?;
        Self::for_service_principal(&credential)
    }
}

fn valid_token(value: &[u8]) -> bool {
    let unpadded = value.strip_suffix(b"=").unwrap_or(value);
    let unpadded = unpadded.strip_suffix(b"=").unwrap_or(unpadded);
    !unpadded.is_empty()
        && unpadded
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || b"-._~+/".contains(c))
}

fn configuration_error() -> ServeError {
    ServeError::new(StartupCause::Configuration {
        error: "query admission credential is missing or invalid".to_owned(),
    })
}

impl QueryAdmission {
    pub(crate) fn authenticate(
        &self,
        headers: &HeaderMap,
    ) -> Result<Option<SecurityContext>, ProblemCode> {
        match self {
            Self::Deny => Err(ProblemCode::AccessDenied),
            Self::UnrestrictedDevelopment => Ok(None),
            Self::Bearer(profile) => {
                let mut values = headers.get_all(header::AUTHORIZATION).iter();
                let value = values.next().ok_or(ProblemCode::Unauthenticated)?;
                if values.next().is_some() || value.as_bytes().len() > 1031 {
                    return Err(ProblemCode::Unauthenticated);
                }
                let value = value.to_str().map_err(|_| ProblemCode::Unauthenticated)?;
                let (scheme, token) = value.split_once(' ').ok_or(ProblemCode::Unauthenticated)?;
                if !scheme.eq_ignore_ascii_case("Bearer") || !valid_token(token.as_bytes()) {
                    return Err(ProblemCode::Unauthenticated);
                }
                let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
                if !bool::from(profile.digest.ct_eq(&digest)) {
                    return Err(ProblemCode::Unauthenticated);
                }
                Ok(Some(profile.context))
            }
        }
    }

    pub(crate) fn validate(&self, budget: &RequestBudget) -> Result<(), ProblemCode> {
        match (self, budget.security_context()) {
            (Self::UnrestrictedDevelopment, None) => Ok(()),
            (Self::Bearer(profile), Some(context))
                if context.matches_policy_snapshot(profile.context.policy_snapshot())
                    && context == profile.context =>
            {
                Ok(())
            }
            _ => Err(ProblemCode::AccessDenied),
        }
    }

    pub(crate) fn policy(&self) -> Option<PolicySnapshotId> {
        match self {
            Self::Bearer(profile) => Some(profile.context.policy_snapshot()),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "query_security_tests.rs"]
mod tests;
