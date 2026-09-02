//! Pure Observed Schema Identity V1 kernel specified by ADR-0050.

use std::fmt;

mod encode;
mod model;
mod validate;

pub use model::*;

/// Why an otherwise bounded schema observation is invalid.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SchemaIdentityInvalidCodeV1 {
    /// A profile ID has invalid lexical form.
    ProfileId,
    /// A facet token has invalid lexical form.
    Token,
    /// An identifier is empty or contains U+0000.
    Identifier,
    /// A text value contains U+0000.
    TextValue,
    /// Reserved for an untyped adapter; direct typed V1 input cannot construct it.
    RelationKind,
    /// A relation has no columns.
    EmptyRelation,
    /// A PK, UNIQUE, or FK has no members.
    EmptyKey,
    /// Column ordinals are not exactly contiguous from one.
    OrdinalContinuity,
    /// A qualified relation name is repeated.
    DuplicateRelation,
    /// A column ordinal is repeated.
    DuplicateColumnOrdinal,
    /// A column name is repeated within a relation.
    DuplicateColumnName,
    /// A type facet key is repeated within a column.
    DuplicateFacetKey,
    /// A PK or UNIQUE member is repeated.
    DuplicateKeyMember,
    /// An FK pair is repeated.
    DuplicateForeignKeyPair,
    /// An FK child key is repeated.
    DuplicateForeignKeyChild,
    /// An FK parent key is repeated.
    DuplicateForeignKeyParent,
    /// A qualified relation reference does not resolve exactly.
    DanglingRelation,
    /// An ordinal-and-name column reference does not resolve exactly.
    DanglingColumn,
    /// More than one distinct primary key remains after deduplication.
    MultiplePrimaryKey,
    /// A canonical stream emitted a length different from its plan.
    CanonicalLengthMismatch,
}

/// The exact bounded resource whose V1 maximum was exceeded.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SchemaIdentityLimitV1 {
    ProfileOrTokenBytes,
    IdentifierBytes,
    FacetTextValueBytes,
    Relations,
    ColumnsPerRelation,
    ColumnsTotal,
    RawConstraints,
    CanonicalConstraints,
    KeyMembers,
    FacetsPerColumn,
    FacetsTotal,
    FacetListItems,
    FacetListItemsTotal,
    Utf8PayloadBytes,
    StructuralBodyBytes,
    TypeBodyBytes,
    ConstraintBodyBytes,
}

/// The checked operation that could not be represented.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SchemaIdentityOperationV1 {
    IndexConversion,
    U32LengthConversion,
    Utf8PayloadAccounting,
    CanonicalBodyAccounting,
    CanonicalStreamAccounting,
}

/// Zero-based submitted-input coordinates for a redacted validation error.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct SchemaIdentityLocationV1 {
    relation: Option<u32>,
    column: Option<u32>,
    facet: Option<u32>,
    constraint: Option<u32>,
    member: Option<u32>,
}

impl SchemaIdentityLocationV1 {
    /// The observation root, used by standalone scalar constructors.
    pub const fn root() -> Self {
        Self {
            relation: None,
            column: None,
            facet: None,
            constraint: None,
            member: None,
        }
    }
    /// Submitted relation index, if relevant.
    pub const fn relation(self) -> Option<u32> {
        self.relation
    }
    /// Submitted column index, if relevant.
    pub const fn column(self) -> Option<u32> {
        self.column
    }
    /// Submitted facet index, if relevant.
    pub const fn facet(self) -> Option<u32> {
        self.facet
    }
    /// Submitted constraint index, if relevant.
    pub const fn constraint(self) -> Option<u32> {
        self.constraint
    }
    /// Submitted key-member index, if relevant.
    pub const fn member(self) -> Option<u32> {
        self.member
    }

    fn at_relation(relation: u32) -> Self {
        Self {
            relation: Some(relation),
            ..Self::root()
        }
    }
    fn at_column(relation: u32, column: u32) -> Self {
        Self {
            relation: Some(relation),
            column: Some(column),
            ..Self::root()
        }
    }
    fn at_facet(relation: u32, column: u32, facet: u32) -> Self {
        Self {
            relation: Some(relation),
            column: Some(column),
            facet: Some(facet),
            ..Self::root()
        }
    }
    fn at_constraint(constraint: u32) -> Self {
        Self {
            constraint: Some(constraint),
            ..Self::root()
        }
    }
    fn at_member(constraint: u32, member: u32) -> Self {
        Self {
            constraint: Some(constraint),
            member: Some(member),
            ..Self::root()
        }
    }
}

impl fmt::Debug for SchemaIdentityLocationV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for SchemaIdentityLocationV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "r={:?},c={:?},f={:?},k={:?},m={:?}",
            self.relation, self.column, self.facet, self.constraint, self.member
        )
    }
}

/// Closed, redacted failure algebra for Observed Schema Identity V1.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum SchemaIdentityErrorV1 {
    /// A semantic or lexical validity rule failed.
    Invalid {
        code: SchemaIdentityInvalidCodeV1,
        location: SchemaIdentityLocationV1,
    },
    /// A fixed V1 resource maximum was exceeded.
    LimitExceeded {
        limit: SchemaIdentityLimitV1,
        observed: u64,
        maximum: u64,
    },
    /// Checked accounting or width conversion overflowed.
    ArithmeticOverflow {
        operation: SchemaIdentityOperationV1,
    },
}

impl fmt::Display for SchemaIdentityErrorV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid { code, location } => {
                write!(f, "schema identity invalid: {code:?} at {location}")
            }
            Self::LimitExceeded {
                limit,
                observed,
                maximum,
            } => write!(
                f,
                "schema identity limit exceeded: {limit:?} ({observed}>{maximum})"
            ),
            Self::ArithmeticOverflow { operation } => {
                write!(f, "schema identity arithmetic overflow: {operation:?}")
            }
        }
    }
}

impl fmt::Debug for SchemaIdentityErrorV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for SchemaIdentityErrorV1 {}

struct NormalizedObservationV1 {
    profiles: SchemaProfilesV1,
    relations: Vec<NormalizedRelationV1>,
    constraints: Vec<NormalizedConstraintRecordV1>,
}

struct NormalizedRelationV1 {
    input_index: u32,
    name: QualifiedNameV1,
    kind: RelationKindV1,
    columns: Vec<NormalizedColumnV1>,
}

struct NormalizedColumnV1 {
    ordinal: std::num::NonZeroU32,
    name: IdentifierV1,
    source_type: SourceTypeV1,
}

struct NormalizedConstraintRecordV1 {
    input_index: u32,
    value: NormalizedConstraintV1,
}

#[derive(Clone, Eq, PartialEq)]
enum NormalizedConstraintV1 {
    NotNull {
        relation: usize,
        column: usize,
        state: ConstraintStateV1,
    },
    PrimaryKey {
        relation: usize,
        state: ConstraintStateV1,
        columns: Vec<usize>,
    },
    UniqueKey {
        relation: usize,
        state: ConstraintStateV1,
        nulls: UniqueNullSemanticsV1,
        columns: Vec<usize>,
    },
    ForeignKey {
        child: usize,
        parent: usize,
        state: ConstraintStateV1,
        match_kind: ForeignKeyMatchV1,
        pairs: Vec<(usize, usize)>,
    },
}

#[derive(Clone, Copy)]
struct KernelLimitsV1 {
    relations: usize,
    columns_per_relation: usize,
    columns_total: usize,
    raw_constraints: usize,
    canonical_constraints: usize,
    key_members: usize,
    facets_per_column: usize,
    facets_total: usize,
    facet_list_items: usize,
    facet_list_items_total: usize,
    utf8_payload_bytes: usize,
    structural_body_bytes: usize,
    type_body_bytes: usize,
    constraint_body_bytes: usize,
}

impl KernelLimitsV1 {
    const PRODUCTION: Self = Self {
        relations: MAX_RELATIONS_V1,
        columns_per_relation: MAX_COLUMNS_PER_RELATION_V1,
        columns_total: MAX_COLUMNS_TOTAL_V1,
        raw_constraints: MAX_RAW_CONSTRAINTS_V1,
        canonical_constraints: MAX_CANONICAL_CONSTRAINTS_V1,
        key_members: MAX_KEY_MEMBERS_V1,
        facets_per_column: MAX_FACETS_PER_COLUMN_V1,
        facets_total: MAX_FACETS_TOTAL_V1,
        facet_list_items: MAX_FACET_LIST_ITEMS_V1,
        facet_list_items_total: MAX_FACET_LIST_ITEMS_TOTAL_V1,
        utf8_payload_bytes: MAX_UTF8_PAYLOAD_BYTES_V1,
        structural_body_bytes: MAX_CANONICAL_BODY_BYTES_V1,
        type_body_bytes: MAX_CANONICAL_BODY_BYTES_V1,
        constraint_body_bytes: MAX_CANONICAL_BODY_BYTES_V1,
    };
}

#[cfg(test)]
type BodyLimitCaseV1 = (SchemaIdentityLimitV1, fn(&mut KernelLimitsV1));

const fn invalid(
    code: SchemaIdentityInvalidCodeV1,
    location: SchemaIdentityLocationV1,
) -> SchemaIdentityErrorV1 {
    SchemaIdentityErrorV1::Invalid { code, location }
}

const fn limit(limit: SchemaIdentityLimitV1, observed: u64, maximum: u64) -> SchemaIdentityErrorV1 {
    SchemaIdentityErrorV1::LimitExceeded {
        limit,
        observed,
        maximum,
    }
}

const fn overflow(operation: SchemaIdentityOperationV1) -> SchemaIdentityErrorV1 {
    SchemaIdentityErrorV1::ArithmeticOverflow { operation }
}

#[cfg(test)]
fn checked_utf8_add(total: u64, added: usize) -> Result<u64, SchemaIdentityErrorV1> {
    checked_utf8_add_with_limit(total, added, MAX_UTF8_PAYLOAD_BYTES_V1)
}

fn checked_utf8_add_with_limit(
    total: u64,
    added: usize,
    maximum: usize,
) -> Result<u64, SchemaIdentityErrorV1> {
    let length = u64::try_from(added)
        .map_err(|_| overflow(SchemaIdentityOperationV1::Utf8PayloadAccounting))?;
    let total = total
        .checked_add(length)
        .ok_or_else(|| overflow(SchemaIdentityOperationV1::Utf8PayloadAccounting))?;
    let maximum = u64::try_from(maximum)
        .map_err(|_| overflow(SchemaIdentityOperationV1::Utf8PayloadAccounting))?;
    if total > maximum {
        return Err(limit(
            SchemaIdentityLimitV1::Utf8PayloadBytes,
            total,
            maximum,
        ));
    }
    Ok(total)
}

#[cfg(test)]
fn check_body_limit(
    kind: SchemaIdentityLimitV1,
    observed: u64,
) -> Result<(), SchemaIdentityErrorV1> {
    check_body_limit_with_max(kind, observed, MAX_CANONICAL_BODY_BYTES_V1)
}

fn check_body_limit_with_max(
    kind: SchemaIdentityLimitV1,
    observed: u64,
    maximum: usize,
) -> Result<(), SchemaIdentityErrorV1> {
    let maximum = u64::try_from(maximum)
        .map_err(|_| overflow(SchemaIdentityOperationV1::CanonicalBodyAccounting))?;
    if observed > maximum {
        return Err(limit(kind, observed, maximum));
    }
    Ok(())
}

fn checked_body_add(total: u64, added: u64) -> Result<u64, SchemaIdentityErrorV1> {
    total
        .checked_add(added)
        .ok_or_else(|| overflow(SchemaIdentityOperationV1::CanonicalBodyAccounting))
}

fn check_limit(
    limit_kind: SchemaIdentityLimitV1,
    observed: usize,
    maximum: usize,
) -> Result<(), SchemaIdentityErrorV1> {
    if observed > maximum {
        let observed = u64::try_from(observed)
            .map_err(|_| overflow(SchemaIdentityOperationV1::IndexConversion))?;
        let maximum = u64::try_from(maximum)
            .map_err(|_| overflow(SchemaIdentityOperationV1::IndexConversion))?;
        return Err(limit(limit_kind, observed, maximum));
    }
    Ok(())
}

fn build(
    input: SchemaObservationInputV1,
) -> Result<ObservedSchemaIdentityV1, SchemaIdentityErrorV1> {
    encode::build(input)
}

#[cfg(test)]
use encode::build_with_preimages;

#[cfg(test)]
mod tests;
