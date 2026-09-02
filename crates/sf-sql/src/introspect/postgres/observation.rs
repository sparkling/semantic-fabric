//! Opaque PostgreSQL observed-schema contract (ADR-0051).

use std::fmt;

use sf_core::schema_identity::ObservedSchemaIdentityV1;

use crate::schema::TableSchema;

pub const POSTGRES16_PUBLIC_STRUCTURAL_PROFILE_ID_V1: &str =
    "io.github.sparkling.semantic-fabric.pg16-pb.structural-v1";
pub const POSTGRES16_PUBLIC_TYPE_PROFILE_ID_V1: &str =
    "io.github.sparkling.semantic-fabric.pg16-pb.type-v1";
pub const POSTGRES16_PUBLIC_CONSTRAINT_PROFILE_ID_V1: &str =
    "io.github.sparkling.semantic-fabric.pg16-pb.constraint-v1";

/// A registered PostgreSQL-16 observation that callers cannot construct.
///
/// ```compile_fail
/// use sf_sql::introspect::Postgres16PublicObservedSchemaV1;
/// let _forged = Postgres16PublicObservedSchemaV1 {};
/// ```
#[derive(Eq, PartialEq)]
pub struct Postgres16PublicObservedSchemaV1 {
    identity: ObservedSchemaIdentityV1,
}

impl Postgres16PublicObservedSchemaV1 {
    /// Returns the non-authorizing content identity carried by this observation.
    pub const fn identity(&self) -> &ObservedSchemaIdentityV1 {
        &self.identity
    }
}

impl fmt::Debug for Postgres16PublicObservedSchemaV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Postgres16PublicObservedSchemaV1")
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PostgresSchemaIdentityGuardCodeV1 {
    ServerEncoding,
    IndexKeyLimit,
    IntegerDatetimes,
    ReplicationRole,
    PublicNamespace,
    CurrentDatabase,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PostgresSchemaIdentityLimitCodeV1 {
    RichRelations,
    PhysicalAttributes,
    LiveColumns,
    RawConstraints,
    KeyMembers,
    Facets,
    TextBytes,
    CanonicalBody,
}

/// Closed and identifier-free reason why schema identity is unavailable.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub enum PostgresSchemaIdentityUnavailableV1 {
    ProfileNotImplemented,
    UnqualifiedEnginePatch,
    GuardUnsupported(PostgresSchemaIdentityGuardCodeV1),
    LegacyCoordinateMismatch,
    CatalogQuery,
    CatalogDecode,
    LimitExceeded(PostgresSchemaIdentityLimitCodeV1),
    UnsupportedRelation,
    UnsupportedType,
    UnsupportedCollation,
    UnsupportedConstraint,
    IdentityRejected,
}

impl fmt::Display for PostgresSchemaIdentityUnavailableV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PostgreSQL schema identity unavailable: ")?;
        formatter.write_str(match self {
            Self::ProfileNotImplemented => "profile not implemented",
            Self::UnqualifiedEnginePatch => "engine patch not qualified",
            Self::GuardUnsupported(code) => guard_message(*code),
            Self::LegacyCoordinateMismatch => "legacy coordinates differ",
            Self::CatalogQuery => "catalogue query failed",
            Self::CatalogDecode => "catalogue decode failed",
            Self::LimitExceeded(code) => limit_message(*code),
            Self::UnsupportedRelation => "relation unsupported",
            Self::UnsupportedType => "type unsupported",
            Self::UnsupportedCollation => "collation unsupported",
            Self::UnsupportedConstraint => "constraint unsupported",
            Self::IdentityRejected => "identity input rejected",
        })
    }
}

impl fmt::Debug for PostgresSchemaIdentityUnavailableV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl std::error::Error for PostgresSchemaIdentityUnavailableV1 {}

const fn guard_message(code: PostgresSchemaIdentityGuardCodeV1) -> &'static str {
    match code {
        PostgresSchemaIdentityGuardCodeV1::ServerEncoding => "server encoding unsupported",
        PostgresSchemaIdentityGuardCodeV1::IndexKeyLimit => "index key limit unsupported",
        PostgresSchemaIdentityGuardCodeV1::IntegerDatetimes => "integer datetimes unsupported",
        PostgresSchemaIdentityGuardCodeV1::ReplicationRole => "replication role unsupported",
        PostgresSchemaIdentityGuardCodeV1::PublicNamespace => "public namespace unsupported",
        PostgresSchemaIdentityGuardCodeV1::CurrentDatabase => "current database unsupported",
    }
}

const fn limit_message(code: PostgresSchemaIdentityLimitCodeV1) -> &'static str {
    match code {
        PostgresSchemaIdentityLimitCodeV1::RichRelations => "rich relation limit exceeded",
        PostgresSchemaIdentityLimitCodeV1::PhysicalAttributes => {
            "physical attribute limit exceeded"
        }
        PostgresSchemaIdentityLimitCodeV1::LiveColumns => "live column limit exceeded",
        PostgresSchemaIdentityLimitCodeV1::RawConstraints => "raw constraint limit exceeded",
        PostgresSchemaIdentityLimitCodeV1::KeyMembers => "key member limit exceeded",
        PostgresSchemaIdentityLimitCodeV1::Facets => "facet limit exceeded",
        PostgresSchemaIdentityLimitCodeV1::TextBytes => "text byte limit exceeded",
        PostgresSchemaIdentityLimitCodeV1::CanonicalBody => "canonical body limit exceeded",
    }
}

/// Identity availability carried inside one committed PostgreSQL snapshot.
pub enum PostgresSchemaIdentityAvailabilityV1 {
    Available(Postgres16PublicObservedSchemaV1),
    Unavailable(PostgresSchemaIdentityUnavailableV1),
}

impl PostgresSchemaIdentityAvailabilityV1 {
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available(_))
    }

    pub const fn identity(&self) -> Option<&ObservedSchemaIdentityV1> {
        match self {
            Self::Available(observation) => Some(observation.identity()),
            Self::Unavailable(_) => None,
        }
    }

    pub const fn unavailable_reason(&self) -> Option<PostgresSchemaIdentityUnavailableV1> {
        match self {
            Self::Available(_) => None,
            Self::Unavailable(reason) => Some(*reason),
        }
    }
}

impl fmt::Debug for PostgresSchemaIdentityAvailabilityV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Available(_) => {
                formatter.write_str("Available(Postgres16PublicObservedSchemaV1)")
            }
            Self::Unavailable(reason) => {
                formatter.debug_tuple("Unavailable").field(reason).finish()
            }
        }
    }
}

/// One committed legacy projection plus its inseparable identity availability.
///
/// ```compile_fail
/// use sf_sql::introspect::Postgres16PublicObservedSnapshotV1;
/// let _forged = Postgres16PublicObservedSnapshotV1 {};
/// ```
pub struct Postgres16PublicObservedSnapshotV1 {
    legacy_tables: Vec<TableSchema>,
    availability: PostgresSchemaIdentityAvailabilityV1,
}

impl Postgres16PublicObservedSnapshotV1 {
    pub fn legacy_tables(&self) -> &[TableSchema] {
        &self.legacy_tables
    }

    pub const fn availability(&self) -> &PostgresSchemaIdentityAvailabilityV1 {
        &self.availability
    }

    /// Preserves both the legacy projection and its identity availability.
    pub fn into_parts(self) -> (Vec<TableSchema>, PostgresSchemaIdentityAvailabilityV1) {
        (self.legacy_tables, self.availability)
    }

    /// Discards identity availability and returns only the compatibility DTOs.
    pub fn into_legacy_tables(self) -> Vec<TableSchema> {
        self.legacy_tables
    }
}

impl fmt::Debug for Postgres16PublicObservedSnapshotV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Postgres16PublicObservedSnapshotV1")
            .field("legacy_table_count", &self.legacy_tables.len())
            .field("availability", &self.availability)
            .finish()
    }
}

#[cfg(test)]
mod tests;
