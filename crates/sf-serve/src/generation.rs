//! Backend-specific generation leases bound to the exact activated runtime.

use std::collections::BTreeMap;
use std::sync::Arc;

use sf_core::{schema_identity::ObservedSchemaIdentityV1, SourceId};
use sf_sql::backend::sqlite::SqliteGenerationSchema;

use crate::binding_identity::RuntimeBindingIdentity;
use crate::budget::RequestBudget;
use crate::pg_generation::{
    PgGenerationError, PgGenerationRequirement, VerifiedPostgresGenerationLease,
};
use crate::sqlite_generation::{SqliteGeneration, SqliteRequestLease};

/// Equality evidence for reload; SQLite/MySQL do not claim the PostgreSQL V1 profile.
#[derive(Clone, PartialEq)]
pub(crate) enum GenerationObservation {
    Postgres(ObservedSchemaIdentityV1),
    Sqlite(Arc<SqliteGenerationSchema>),
    Mysql(Arc<crate::mysql_generation::schema::Schema>),
}

pub(crate) enum GenerationRequirement {
    Postgres(PgGenerationRequirement),
    Mysql {
        expected: Arc<crate::mysql_generation::MysqlGeneration>,
        binding_identity: RuntimeBindingIdentity,
    },
    Sqlite {
        expected: Arc<SqliteGeneration>,
        binding_identity: RuntimeBindingIdentity,
    },
}

impl GenerationRequirement {
    pub(crate) fn source_id(&self) -> SourceId {
        match self {
            Self::Postgres(requirement) => requirement.source_id(),
            Self::Sqlite { expected, .. } => expected.source_id(),
            Self::Mysql { expected, .. } => expected.source_id(),
        }
    }

    pub(crate) fn binding_identity(&self) -> &RuntimeBindingIdentity {
        match self {
            Self::Postgres(requirement) => &requirement.binding_identity,
            Self::Sqlite {
                binding_identity, ..
            }
            | Self::Mysql {
                binding_identity, ..
            } => binding_identity,
        }
    }
}

pub(crate) enum VerifiedGenerationLease {
    Postgres(VerifiedPostgresGenerationLease),
    Sqlite(SqliteRequestLease),
    Mysql(crate::mysql_generation::MysqlRequestLease),
}

impl VerifiedGenerationLease {
    fn backend_kind(&self) -> crate::BackendKind {
        match self {
            Self::Postgres(_) => crate::BackendKind::Postgres,
            Self::Sqlite(_) => crate::BackendKind::Sqlite,
            Self::Mysql(_) => crate::BackendKind::MySql,
        }
    }
    #[cfg(test)]
    pub(crate) fn into_postgres(
        self,
    ) -> Result<VerifiedPostgresGenerationLease, PgGenerationError> {
        match self {
            Self::Postgres(lease) => Ok(lease),
            Self::Sqlite(_) | Self::Mysql(_) => Err(PgGenerationError::Internal),
        }
    }

    pub(crate) fn source_id(&self) -> SourceId {
        match self {
            Self::Postgres(lease) => lease.source_id(),
            Self::Sqlite(lease) => lease.source_id(),
            Self::Mysql(lease) => lease.source_id(),
        }
    }

    pub(crate) async fn close(self) -> Result<(), PgGenerationError> {
        match self {
            Self::Postgres(lease) => lease.rollback_bounded().await,
            Self::Sqlite(lease) => lease.finish().await,
            Self::Mysql(lease) => lease.finish().await,
        }
    }
}

type RuntimeBoundGenerationLease = (RuntimeBindingIdentity, VerifiedGenerationLease);

#[derive(Default)]
pub(crate) struct VerifiedGenerationLeases {
    pub(crate) leases: BTreeMap<SourceId, RuntimeBoundGenerationLease>,
}

impl VerifiedGenerationLeases {
    pub(crate) fn matches_backend(
        &self,
        source_id: SourceId,
        identity: &RuntimeBindingIdentity,
        verified: bool,
        backend: crate::BackendKind,
    ) -> bool {
        self.matches(source_id, identity, verified)
            && self
                .leases
                .get(&source_id)
                .is_none_or(|(_, lease)| lease.backend_kind() == backend)
    }
    pub(crate) async fn acquire(
        mut requirements: Vec<GenerationRequirement>,
        budget: &RequestBudget,
    ) -> Result<Self, PgGenerationError> {
        requirements.sort_by_key(GenerationRequirement::source_id);
        if requirements
            .windows(2)
            .any(|pair| pair[0].source_id() == pair[1].source_id())
        {
            return Err(PgGenerationError::Internal);
        }
        let mut leases = Self::default();
        for requirement in requirements {
            let source_id = requirement.source_id();
            let identity = requirement.binding_identity().clone();
            let result = match requirement {
                GenerationRequirement::Postgres(requirement) => {
                    crate::pg_generation::acquire_expected(requirement, budget)
                        .await
                        .map(VerifiedGenerationLease::Postgres)
                }
                GenerationRequirement::Sqlite { expected, .. } => expected
                    .acquire(budget)
                    .await
                    .map(VerifiedGenerationLease::Sqlite),
                GenerationRequirement::Mysql { expected, .. } => expected
                    .acquire(budget)
                    .await
                    .map(VerifiedGenerationLease::Mysql),
            };
            match result {
                Ok(lease) => {
                    leases.leases.insert(source_id, (identity, lease));
                }
                Err(error) => {
                    let _ = leases.finish().await;
                    return Err(error);
                }
            }
        }
        Ok(leases)
    }

    pub(crate) fn take(
        &mut self,
        source_id: SourceId,
        identity: &RuntimeBindingIdentity,
    ) -> Option<VerifiedGenerationLease> {
        if !self.leases.get(&source_id)?.0.ptr_eq(identity) {
            return None;
        }
        let (_, lease) = self.leases.remove(&source_id)?;
        debug_assert_eq!(lease.source_id(), source_id);
        Some(lease)
    }

    pub(crate) fn matches(
        &self,
        source_id: SourceId,
        identity: &RuntimeBindingIdentity,
        verified: bool,
    ) -> bool {
        match self.leases.get(&source_id) {
            Some((actual, _)) => verified && actual.ptr_eq(identity),
            None => !verified,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.leases.is_empty()
    }

    pub(crate) async fn finish(self) -> Result<(), PgGenerationError> {
        let mut first_error = None;
        for (_, (_, lease)) in self.leases {
            let result = lease.close().await;
            if first_error.is_none() {
                first_error = result.err();
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}
