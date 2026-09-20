//! Immutable operator-provisioned identities; native source policies enforce rows.
use super::*;
use std::collections::BTreeSet;
use std::sync::Arc;

#[path = "provisioned_security_env.rs"]
mod environment;

/// An opaque stable subject, one credential, and explicit database policy inputs.
/// Construct from trusted provisioning only, never client-supplied claims. No
/// tenant, clearance, or business vocabulary is inferred by Semantic Fabric.
#[derive(Clone)]
pub struct ProvisionedBearerSubject {
    subject: [u8; 32],
    attributes: [u8; 32],
    principal: BearerQueryAdmission,
}

impl std::fmt::Debug for ProvisionedBearerSubject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProvisionedBearerSubject([REDACTED])")
    }
}

impl ProvisionedBearerSubject {
    /// Bind a high-entropy credential to a stable operator-owned opaque subject
    /// reference (1–128 ASCII identifier bytes) and PostgreSQL RLS settings.
    /// Rotation preserves subject identity; registry policy identity changes.
    pub fn postgres_rls(
        subject_ref: &str,
        credential: &str,
        claims: crate::PostgresRlsClaims,
    ) -> Result<Self, ServeError> {
        let subject = subject_identity(subject_ref)?;
        let attributes = claims.identity(&[0; 32], b"sf-provisioned-rls-attributes-v1\0");
        let principal =
            BearerQueryAdmission::for_service_principal(credential)?.with_postgres_rls(claims)?;
        Ok(Self {
            subject,
            attributes,
            principal,
        })
    }

    /// Bind a credential and opaque subject to a portable row allowlist.
    pub fn portable_rows(
        subject_ref: &str,
        credential: &str,
        policy: crate::PortableRowPolicy,
    ) -> Result<Self, ServeError> {
        let subject = subject_identity(subject_ref)?;
        let attributes = *policy.identity();
        let principal =
            BearerQueryAdmission::for_service_principal(credential)?.with_portable_rows(policy)?;
        Ok(Self {
            subject,
            attributes,
            principal,
        })
    }
}

fn subject_identity(subject_ref: &str) -> Result<[u8; 32], ServeError> {
    if subject_ref.is_empty()
        || subject_ref.len() > 128
        || !subject_ref
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
    {
        return Err(invalid_registry());
    }
    let mut hash = Sha256::new();
    hash.update(b"sf-provisioned-subject-v1\0");
    hash.update((subject_ref.len() as u64).to_be_bytes());
    hash.update(subject_ref.as_bytes());
    Ok(hash.finalize().into())
}

/// Bounded service-lifetime registry. Every subject must have one explicit
/// PostgreSQL-RLS or portable-row policy; there is no read-all/anonymous fallback.
#[derive(Clone)]
pub struct ProvisionedBearerAdmission {
    subjects: Arc<[BearerQueryAdmission]>,
    policy: PolicySnapshotId,
}

impl std::fmt::Debug for ProvisionedBearerAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProvisionedBearerAdmission([REDACTED])")
    }
}

impl ProvisionedBearerAdmission {
    /// Reject empty/oversized registries, duplicate subjects, and credentials
    /// shared by multiple subjects. Canonical order is independent of input order.
    pub fn new(mut subjects: Vec<ProvisionedBearerSubject>) -> Result<Self, ServeError> {
        if subjects.is_empty() || subjects.len() > 256 {
            return Err(invalid_registry());
        }
        subjects.sort_unstable_by_key(|s| s.subject);
        let mut credentials = BTreeSet::new();
        if subjects.windows(2).any(|s| s[0].subject == s[1].subject)
            || subjects
                .iter()
                .any(|s| !credentials.insert(s.principal.digest))
        {
            return Err(invalid_registry());
        }
        let mut hash = Sha256::new();
        hash.update(b"sf-provisioned-admission-policy-v2\0");
        hash.update((subjects.len() as u64).to_be_bytes());
        for subject in &subjects {
            hash.update(subject.subject);
            hash.update(subject.principal.digest);
            hash.update(subject.attributes);
        }
        let policy = PolicySnapshotId::from_digest(hash.finalize().into())
            .map_err(|_| invalid_registry())?;
        let subjects = subjects
            .into_iter()
            .map(|mut s| {
                s.principal.context = SecurityContext::new(
                    policy,
                    SubjectIdentity::from_digest(s.subject).map_err(|_| invalid_registry())?,
                    RequestAttributesIdentity::from_digest(s.attributes)
                        .map_err(|_| invalid_registry())?,
                );
                Ok(s.principal)
            })
            .collect::<Result<Vec<_>, ServeError>>()?;
        Ok(Self {
            subjects: subjects.into(),
            policy,
        })
    }

    pub(super) fn policy(&self) -> PolicySnapshotId {
        self.policy
    }

    pub(super) fn only_portable_rows(&self) -> bool {
        self.subjects
            .iter()
            .all(|subject| subject.rls.is_none() && subject.portable_rows.is_some())
    }

    pub(super) fn portable_columns_exist(
        &self,
        source: sf_core::SourceId,
        tables: &crate::portable_rows::MappedPolicyColumns<'_>,
    ) -> bool {
        self.subjects.iter().all(|subject| {
            subject
                .portable_rows
                .as_ref()
                .is_none_or(|policy| policy.mapped_columns_exist(source, tables))
        })
    }

    pub(super) fn match_credential(&self, digest: &[u8; 32]) -> Option<&BearerQueryAdmission> {
        let mut selected = None;
        // Visit every bounded entry: do not short-circuit on a matching position.
        for subject in self.subjects.iter() {
            if bool::from(subject.digest.ct_eq(digest)) {
                selected = Some(subject);
            }
        }
        selected
    }

    pub(super) fn matches(&self, budget: &RequestBudget) -> bool {
        self.subjects.iter().any(|subject| {
            budget.security_context() == Some(subject.context)
                && budget.postgres_rls() == subject.rls.as_ref()
                && budget.portable_rows() == subject.portable_rows.as_ref()
        })
    }
}

fn invalid_registry() -> ServeError {
    ServeError::new(StartupCause::Configuration {
        error: "provisioned query identity registry is missing or invalid".into(),
    })
}
