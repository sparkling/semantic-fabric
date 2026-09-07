//! Query admission and an optional explicit PostgreSQL source-RLS profile.
use axum::http::{header, HeaderMap};
use sf_core::security_context::{
    PolicySnapshotId, RequestAttributesIdentity, SecurityContext, SubjectIdentity,
};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::budget::RequestBudget;
use crate::problem::{ProblemCode, StartupCause};
use crate::ServeError;

#[path = "provisioned_security.rs"]
mod provisioned;
pub use provisioned::{ProvisionedBearerAdmission, ProvisionedBearerSubject};

/// Explicit immutable service-lifetime policy. Rotation requires a new server;
/// a live request never consults a mutable credential or a newer policy.
#[derive(Clone, Debug, Default)]
pub enum QueryAdmission {
    /// No query may execute until the embedding selects an admission profile.
    #[default]
    Deny,
    /// Anyone may read all mapped data. Only for explicitly unprotected development.
    UnrestrictedDevelopment,
    /// Service principal, optionally restricted by explicit PostgreSQL RLS claims.
    /// Without source RLS it may read all mapped data. Not end-user identity.
    Bearer(BearerQueryAdmission),
    /// Explicit subjects with their own trusted PostgreSQL RLS settings.
    /// One immutable registry is shared by all requests; headers cannot supply claims.
    ProvisionedBearers(ProvisionedBearerAdmission),
}

/// Produced only by successful credential verification, retained atomically.
pub(crate) struct AuthenticatedQuery {
    pub(crate) context: SecurityContext,
    pub(crate) rls: Option<std::sync::Arc<crate::PostgresRlsClaims>>,
    pub(crate) portable_rows: Option<std::sync::Arc<crate::PortableRowPolicy>>,
}

/// A bounded credential digest and provider-neutral, redacted request identity.
#[derive(Clone)]
pub struct BearerQueryAdmission {
    digest: [u8; 32],
    context: SecurityContext,
    rls: Option<std::sync::Arc<crate::PostgresRlsClaims>>,
    portable_rows: Option<std::sync::Arc<crate::PortableRowPolicy>>,
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
        Ok(Self {
            digest,
            context,
            rls: None,
            portable_rows: None,
        })
    }

    /// Restrict this principal to database-enforced RLS on authored PostgreSQL
    /// public base-table mappings. Other sources and logical SQL queries deny.
    pub fn with_postgres_rls(
        mut self,
        claims: crate::PostgresRlsClaims,
    ) -> Result<Self, ServeError> {
        if self.portable_rows.is_some() {
            return Err(configuration_error());
        }
        self.context = SecurityContext::new(
            PolicySnapshotId::from_digest(claims.identity(&self.digest, b"sf-pg-rls-policy-v1\0"))
                .map_err(|_| configuration_error())?,
            self.context.subject(),
            RequestAttributesIdentity::from_digest(
                claims.identity(&self.digest, b"sf-pg-rls-attributes-v1\0"),
            )
            .map_err(|_| configuration_error())?,
        );
        self.rls = Some(std::sync::Arc::new(claims));
        Ok(self)
    }

    /// Restrict this principal with portable source/table/column equality rules.
    pub fn with_portable_rows(
        mut self,
        policy: crate::PortableRowPolicy,
    ) -> Result<Self, ServeError> {
        if self.rls.is_some() {
            return Err(configuration_error());
        }
        self.context = SecurityContext::new(
            PolicySnapshotId::from_digest(*policy.identity()).map_err(|_| configuration_error())?,
            self.context.subject(),
            RequestAttributesIdentity::from_digest(*policy.identity())
                .map_err(|_| configuration_error())?,
        );
        self.portable_rows = Some(std::sync::Arc::new(policy));
        Ok(self)
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
    #[cfg(test)]
    pub(crate) fn authenticate(
        &self,
        headers: &HeaderMap,
    ) -> Result<Option<SecurityContext>, ProblemCode> {
        self.admit(headers)
            .map(|admitted| admitted.map(|a| a.context))
    }

    pub(crate) fn admit(
        &self,
        headers: &HeaderMap,
    ) -> Result<Option<AuthenticatedQuery>, ProblemCode> {
        match self {
            Self::Deny => Err(ProblemCode::AccessDenied),
            Self::UnrestrictedDevelopment => Ok(None),
            Self::Bearer(_) | Self::ProvisionedBearers(_) => {
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
                let profile = match self {
                    Self::Bearer(profile) if bool::from(profile.digest.ct_eq(&digest)) => {
                        Some(profile)
                    }
                    Self::ProvisionedBearers(registry) => registry.match_credential(&digest),
                    _ => None,
                }
                .ok_or(ProblemCode::Unauthenticated)?;
                Ok(Some(AuthenticatedQuery {
                    context: profile.context,
                    rls: profile.rls.clone(),
                    portable_rows: profile.portable_rows.clone(),
                }))
            }
        }
    }

    pub(crate) fn validate(&self, budget: &RequestBudget) -> Result<(), ProblemCode> {
        match (self, budget.security_context()) {
            (Self::UnrestrictedDevelopment, None) => Ok(()),
            (Self::Bearer(profile), Some(context))
                if context.matches_policy_snapshot(profile.context.policy_snapshot())
                    && context == profile.context
                    && budget.postgres_rls() == profile.rls.as_ref()
                    && budget.portable_rows() == profile.portable_rows.as_ref() =>
            {
                Ok(())
            }
            (Self::ProvisionedBearers(registry), Some(_)) if registry.matches(budget) => Ok(()),
            _ => Err(ProblemCode::AccessDenied),
        }
    }

    pub(crate) fn policy(&self) -> Option<PolicySnapshotId> {
        match self {
            Self::Bearer(profile) => Some(profile.context.policy_snapshot()),
            Self::ProvisionedBearers(registry) => Some(registry.policy()),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn postgres_rls(&self) -> Option<std::sync::Arc<crate::PostgresRlsClaims>> {
        match self {
            Self::Bearer(profile) => profile.rls.clone(),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "query_security_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "provisioned_security_tests.rs"]
mod provisioned_tests;
