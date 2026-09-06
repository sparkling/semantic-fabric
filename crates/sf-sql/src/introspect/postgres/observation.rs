//! Opaque PostgreSQL observed-schema contract (ADR-0051).

use std::fmt;

use tokio_postgres::{GenericClient, Row};

use sf_core::schema_identity::{
    ConstraintInputV1, ObservedSchemaIdentityV1, ProfileIdV1, RelationInputV1,
    SchemaIdentityErrorV1, SchemaIdentityLimitV1, SchemaObservationInputV1, SchemaProfilesV1,
    MAX_RELATIONS_V1,
};

use tokio_postgres::types::Type;

use crate::schema::TableSchema;

use super::legacy_query::{query_bounded_mapped, TypedQueryParameter};

#[allow(dead_code)]
mod catalog_decode;
#[allow(dead_code)]
mod catalog_sql;
#[allow(dead_code)]
mod constraint_budget;
#[allow(dead_code)]
mod constraints;
#[allow(dead_code)]
mod direct_mapping;
#[allow(dead_code)]
mod relation;
#[allow(dead_code)]
mod source_type;
#[allow(dead_code)]
mod trigger_evidence;

pub(super) async fn qualify_profile_guard<C>(
    client: &C,
) -> Result<(), PostgresSchemaIdentityUnavailableV1>
where
    C: GenericClient + Sync,
{
    let row: Row = client
        .query_one(catalog_sql::RICH_GUARD_SQL_V1, &[])
        .await
        .map_err(|_| PostgresSchemaIdentityUnavailableV1::CatalogQuery)?;
    catalog_decode::decode_guard_row_v1(&row).map(|_| ())
}

pub const POSTGRES16_PUBLIC_STRUCTURAL_PROFILE_ID_V1: &str =
    "io.github.sparkling.semantic-fabric.pg16-pb.structural-v1";
pub const POSTGRES16_PUBLIC_TYPE_PROFILE_ID_V1: &str =
    "io.github.sparkling.semantic-fabric.pg16-pb.type-v1";
pub const POSTGRES16_PUBLIC_CONSTRAINT_PROFILE_ID_V1: &str =
    "io.github.sparkling.semantic-fabric.pg16-pb.constraint-v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RegisteredPostgresObservationProfileV1 {
    Postgres16PublicBaseV1,
}

impl RegisteredPostgresObservationProfileV1 {
    fn profiles(self) -> Result<SchemaProfilesV1, PostgresSchemaIdentityUnavailableV1> {
        match self {
            Self::Postgres16PublicBaseV1 => Ok(SchemaProfilesV1 {
                structural: registered_profile_id(POSTGRES16_PUBLIC_STRUCTURAL_PROFILE_ID_V1)?,
                types: registered_profile_id(POSTGRES16_PUBLIC_TYPE_PROFILE_ID_V1)?,
                constraints: registered_profile_id(POSTGRES16_PUBLIC_CONSTRAINT_PROFILE_ID_V1)?,
            }),
        }
    }
}

fn select_registered_profile_v1(
    server_version_num: i32,
) -> Result<RegisteredPostgresObservationProfileV1, PostgresSchemaIdentityUnavailableV1> {
    match server_version_num {
        160_009 | 160_015 => Ok(RegisteredPostgresObservationProfileV1::Postgres16PublicBaseV1),
        160_000..=169_999 => Err(PostgresSchemaIdentityUnavailableV1::UnqualifiedEnginePatch),
        _ => Err(PostgresSchemaIdentityUnavailableV1::ProfileNotImplemented),
    }
}

// Kept private; the public snapshot exposes its result only after the guarded
// collector succeeds, while exact patch qualification remains a release gate.
#[allow(dead_code)]
fn build_registered_observation(
    server_version_num: i32,
    relations: Vec<RelationInputV1>,
    constraints: Vec<ConstraintInputV1>,
) -> Result<Postgres16PublicObservedSchemaV1, PostgresSchemaIdentityUnavailableV1> {
    let profiles = select_registered_profile_v1(server_version_num)?.profiles()?;
    let direct_mapping_tables =
        direct_mapping::project_direct_mapping_tables_v1(&relations, &constraints)?;
    let identity = ObservedSchemaIdentityV1::build(SchemaObservationInputV1 {
        profiles,
        relations,
        constraints,
    })
    .map_err(map_schema_identity_error_v1)?;
    Ok(Postgres16PublicObservedSchemaV1 {
        identity,
        direct_mapping_tables,
    })
}

/// Assemble a registered observation from the private normalized relation
/// graph and bounded raw constraint evidence. This is the single pure seam
/// that the eventual SQL snapshot adapter must call.
#[allow(dead_code)]
fn build_registered_observation_from_raw(
    server_version_num: i32,
    relations: relation::Postgres16NormalizedRelationsV1,
    raw_constraints: Vec<constraints::Postgres16RawConstraintV1>,
) -> Result<Postgres16PublicObservedSchemaV1, PostgresSchemaIdentityUnavailableV1> {
    let constraints =
        constraints::normalize_postgres16_constraints_v1(&relations, raw_constraints)?;
    build_registered_observation(server_version_num, relations.into_relations(), constraints)
}

/// Capture and assemble the rich profile inside one caller-owned snapshot.
/// Every row is decoded and bounded before the pure normalizers run.
#[allow(dead_code)]
pub(super) async fn capture_registered_observation<C>(
    client: &C,
    schema_name: &str,
    legacy_tables: &[TableSchema],
) -> Result<Postgres16PublicObservedSchemaV1, PostgresSchemaIdentityUnavailableV1>
where
    C: GenericClient + Sync,
{
    let guard_row = client
        .query_one(catalog_sql::RICH_GUARD_SQL_V1, &[])
        .await
        .map_err(|_| PostgresSchemaIdentityUnavailableV1::CatalogQuery)?;
    let guard = catalog_decode::decode_guard_row_v1(&guard_row)?;
    let server_version_num = guard.server_version_num;
    let text_limit = catalog_decode::MAX_CATALOG_TEXT_BYTES_V1 as i32;
    let relation_limit = MAX_RELATIONS_V1 as i64 + 1;
    let relations = query_bounded_mapped(
        client,
        catalog_sql::RICH_RELATIONS_SQL_V1,
        &[
            TypedQueryParameter::new(&schema_name, Type::TEXT),
            TypedQueryParameter::new(&text_limit, Type::INT4),
            TypedQueryParameter::new(&relation_limit, Type::INT8),
        ],
        MAX_RELATIONS_V1,
        |_| PostgresSchemaIdentityUnavailableV1::CatalogQuery,
        || {
            PostgresSchemaIdentityUnavailableV1::LimitExceeded(
                PostgresSchemaIdentityLimitCodeV1::RichRelations,
            )
        },
        |row| catalog_decode::decode_relation_row_v1(&row)?.into_catalog_fact(),
    )
    .await?;
    let attribute_limit = relation::MAX_PHYSICAL_ATTRIBUTES_TOTAL_PG16_V1 as i64 + 1;
    let attributes = query_bounded_mapped(
        client,
        catalog_sql::RICH_ATTRIBUTES_SQL_V1,
        &[
            TypedQueryParameter::new(&schema_name, Type::TEXT),
            TypedQueryParameter::new(&text_limit, Type::INT4),
            TypedQueryParameter::new(&attribute_limit, Type::INT8),
        ],
        relation::MAX_PHYSICAL_ATTRIBUTES_TOTAL_PG16_V1,
        |_| PostgresSchemaIdentityUnavailableV1::CatalogQuery,
        || {
            PostgresSchemaIdentityUnavailableV1::LimitExceeded(
                PostgresSchemaIdentityLimitCodeV1::PhysicalAttributes,
            )
        },
        |row| catalog_decode::decode_attribute_row_v1(&row)?.into_catalog_fact(&guard),
    )
    .await?;
    let normalized = relation::normalize_postgres16_relations_v1(relations, attributes)?;
    relation::compare_postgres16_legacy_coordinates_v1(legacy_tables, &normalized)?;
    let mut raw_constraints = constraints::observed_not_null_constraints_v1(&normalized)?;
    let (remaining_constraints, constraint_limit) =
        constraint_budget::constraint_catalog_budget_v1(raw_constraints.len())?;
    let mut catalog_constraints = query_bounded_mapped(
        client,
        catalog_sql::RICH_CONSTRAINTS_SQL_V1,
        &[
            TypedQueryParameter::new(&schema_name, Type::TEXT),
            TypedQueryParameter::new(&constraint_limit, Type::INT8),
        ],
        remaining_constraints,
        |_| PostgresSchemaIdentityUnavailableV1::CatalogQuery,
        || {
            PostgresSchemaIdentityUnavailableV1::LimitExceeded(
                PostgresSchemaIdentityLimitCodeV1::RawConstraints,
            )
        },
        |row| catalog_decode::decode_constraint_row_v1(&row)?.into_raw_constraint(),
    )
    .await?;
    raw_constraints.append(&mut catalog_constraints);
    build_registered_observation_from_raw(server_version_num, normalized, raw_constraints)
}

fn registered_profile_id(
    value: &'static str,
) -> Result<ProfileIdV1, PostgresSchemaIdentityUnavailableV1> {
    ProfileIdV1::new(value).map_err(|_| PostgresSchemaIdentityUnavailableV1::IdentityRejected)
}

fn map_schema_identity_error_v1(
    error: SchemaIdentityErrorV1,
) -> PostgresSchemaIdentityUnavailableV1 {
    match error {
        SchemaIdentityErrorV1::LimitExceeded { limit, .. } => {
            match map_schema_identity_limit_v1(limit) {
                Some(code) => PostgresSchemaIdentityUnavailableV1::LimitExceeded(code),
                None => PostgresSchemaIdentityUnavailableV1::IdentityRejected,
            }
        }
        SchemaIdentityErrorV1::Invalid { .. }
        | SchemaIdentityErrorV1::ArithmeticOverflow { .. } => {
            PostgresSchemaIdentityUnavailableV1::IdentityRejected
        }
    }
}

const fn map_schema_identity_limit_v1(
    limit: SchemaIdentityLimitV1,
) -> Option<PostgresSchemaIdentityLimitCodeV1> {
    use PostgresSchemaIdentityLimitCodeV1 as PostgresLimit;
    use SchemaIdentityLimitV1 as KernelLimit;

    match limit {
        KernelLimit::ProfileOrTokenBytes
        | KernelLimit::IdentifierBytes
        | KernelLimit::FacetTextValueBytes => None,
        KernelLimit::Relations => Some(PostgresLimit::RichRelations),
        KernelLimit::ColumnsPerRelation | KernelLimit::ColumnsTotal => {
            Some(PostgresLimit::LiveColumns)
        }
        KernelLimit::RawConstraints | KernelLimit::CanonicalConstraints => {
            Some(PostgresLimit::RawConstraints)
        }
        KernelLimit::KeyMembers => Some(PostgresLimit::KeyMembers),
        KernelLimit::FacetsPerColumn
        | KernelLimit::FacetsTotal
        | KernelLimit::FacetListItems
        | KernelLimit::FacetListItemsTotal => Some(PostgresLimit::Facets),
        KernelLimit::Utf8PayloadBytes => Some(PostgresLimit::TextBytes),
        KernelLimit::StructuralBodyBytes
        | KernelLimit::TypeBodyBytes
        | KernelLimit::ConstraintBodyBytes => Some(PostgresLimit::CanonicalBody),
    }
}

/// A registered PostgreSQL-16 observation that callers cannot construct.
///
/// ```compile_fail
/// use sf_sql::introspect::Postgres16PublicObservedSchemaV1;
/// let _forged = Postgres16PublicObservedSchemaV1 {};
/// ```
#[derive(Eq, PartialEq)]
pub struct Postgres16PublicObservedSchemaV1 {
    identity: ObservedSchemaIdentityV1,
    direct_mapping_tables: Vec<TableSchema>,
}

impl Postgres16PublicObservedSchemaV1 {
    /// Returns the non-authorizing content identity carried by this observation.
    pub const fn identity(&self) -> &ObservedSchemaIdentityV1 {
        &self.identity
    }

    /// Returns the Direct-Mapping DTOs derived from the exact admitted rich
    /// relation, type, and constraint facts carried by this observation.
    pub fn direct_mapping_tables(&self) -> &[TableSchema] {
        &self.direct_mapping_tables
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
    IdentifierLength,
    IndexKeyLimit,
    IntegerDatetimes,
    ReplicationRole,
    SearchPath,
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
        PostgresSchemaIdentityGuardCodeV1::IdentifierLength => "identifier length unsupported",
        PostgresSchemaIdentityGuardCodeV1::IndexKeyLimit => "index key limit unsupported",
        PostgresSchemaIdentityGuardCodeV1::IntegerDatetimes => "integer datetimes unsupported",
        PostgresSchemaIdentityGuardCodeV1::ReplicationRole => "replication role unsupported",
        PostgresSchemaIdentityGuardCodeV1::SearchPath => "search path unsupported",
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
    pub(super) fn available(
        legacy_tables: Vec<TableSchema>,
        observation: Postgres16PublicObservedSchemaV1,
    ) -> Self {
        Self {
            legacy_tables,
            availability: PostgresSchemaIdentityAvailabilityV1::Available(observation),
        }
    }

    pub(super) fn unavailable(
        legacy_tables: Vec<TableSchema>,
        reason: PostgresSchemaIdentityUnavailableV1,
    ) -> Self {
        Self {
            legacy_tables,
            availability: PostgresSchemaIdentityAvailabilityV1::Unavailable(reason),
        }
    }

    pub fn legacy_tables(&self) -> &[TableSchema] {
        &self.legacy_tables
    }

    pub const fn availability(&self) -> &PostgresSchemaIdentityAvailabilityV1 {
        &self.availability
    }

    /// Returns the rich-authority Direct-Mapping projection when identity is
    /// available. Legacy DTOs are deliberately not substituted on failure.
    pub fn direct_mapping_tables(&self) -> Option<&[TableSchema]> {
        match &self.availability {
            PostgresSchemaIdentityAvailabilityV1::Available(observation) => {
                Some(observation.direct_mapping_tables())
            }
            PostgresSchemaIdentityAvailabilityV1::Unavailable(_) => None,
        }
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
