//! Public values and hard limits for Observed Schema Identity V1.

use std::fmt;
use std::num::NonZeroU32;

use super::{
    SchemaIdentityErrorV1, SchemaIdentityInvalidCodeV1, SchemaIdentityLimitV1,
    SchemaIdentityLocationV1, SchemaIdentityOperationV1,
};

/// Maximum byte length of a profile ID or facet token.
pub const MAX_PROFILE_TOKEN_BYTES_V1: usize = 64;
/// Maximum byte length of an identifier.
pub const MAX_IDENTIFIER_BYTES_V1: usize = 1_024;
/// Maximum byte length of a facet text value.
pub const MAX_FACET_TEXT_BYTES_V1: usize = 16_384;
/// Maximum number of relations in one observation.
pub const MAX_RELATIONS_V1: usize = 4_096;
/// Maximum number of columns in one relation.
pub const MAX_COLUMNS_PER_RELATION_V1: usize = 4_096;
/// Maximum number of columns across one observation.
pub const MAX_COLUMNS_TOTAL_V1: usize = 65_536;
/// Maximum number of submitted constraint records.
pub const MAX_RAW_CONSTRAINTS_V1: usize = 65_536;
/// Maximum number of canonical constraint records.
pub const MAX_CANONICAL_CONSTRAINTS_V1: usize = 65_536;
/// Maximum arity of one primary key, unique key, or foreign key.
pub const MAX_KEY_MEMBERS_V1: usize = 256;
/// Maximum number of facets on one column type.
pub const MAX_FACETS_PER_COLUMN_V1: usize = 64;
/// Maximum number of facets across one observation.
pub const MAX_FACETS_TOTAL_V1: usize = 262_144;
/// Maximum number of items in one facet list.
pub const MAX_FACET_LIST_ITEMS_V1: usize = 1_024;
/// Maximum number of facet-list items across one observation.
pub const MAX_FACET_LIST_ITEMS_TOTAL_V1: usize = 1_048_576;
/// Maximum cumulative bytes in submitted semantic string occurrences.
pub const MAX_UTF8_PAYLOAD_BYTES_V1: usize = 33_554_432;
/// Maximum canonical body length for each digest.
pub const MAX_CANONICAL_BODY_BYTES_V1: usize = 67_108_864;

fn invalid(code: SchemaIdentityInvalidCodeV1) -> SchemaIdentityErrorV1 {
    SchemaIdentityErrorV1::Invalid {
        code,
        location: SchemaIdentityLocationV1::root(),
    }
}

fn limit(
    kind: SchemaIdentityLimitV1,
    observed: usize,
    maximum: usize,
) -> Result<SchemaIdentityErrorV1, SchemaIdentityErrorV1> {
    Ok(SchemaIdentityErrorV1::LimitExceeded {
        limit: kind,
        observed: u64::try_from(observed).map_err(|_| {
            SchemaIdentityErrorV1::ArithmeticOverflow {
                operation: SchemaIdentityOperationV1::IndexConversion,
            }
        })?,
        maximum: u64::try_from(maximum).map_err(|_| SchemaIdentityErrorV1::ArithmeticOverflow {
            operation: SchemaIdentityOperationV1::IndexConversion,
        })?,
    })
}

fn checked_text(
    value: Box<str>,
    maximum: usize,
    limit_kind: SchemaIdentityLimitV1,
    invalid_code: SchemaIdentityInvalidCodeV1,
    allow_empty: bool,
) -> Result<Box<str>, SchemaIdentityErrorV1> {
    if value.len() > maximum {
        return Err(limit(limit_kind, value.len(), maximum)?);
    }
    if (!allow_empty && value.is_empty()) || value.contains('\0') {
        return Err(invalid(invalid_code));
    }
    Ok(value)
}

fn checked_token(
    value: Box<str>,
    invalid_code: SchemaIdentityInvalidCodeV1,
) -> Result<Box<str>, SchemaIdentityErrorV1> {
    if value.len() > MAX_PROFILE_TOKEN_BYTES_V1 {
        return Err(limit(
            SchemaIdentityLimitV1::ProfileOrTokenBytes,
            value.len(),
            MAX_PROFILE_TOKEN_BYTES_V1,
        )?);
    }
    let bytes = value.as_bytes();
    let endpoint = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    let interior = |byte: u8| endpoint(byte) || matches!(byte, b'.' | b'_' | b'-');
    if bytes.first().is_none_or(|byte| !endpoint(*byte))
        || bytes.last().is_none_or(|byte| !endpoint(*byte))
        || !bytes.iter().copied().all(interior)
    {
        return Err(invalid(invalid_code));
    }
    Ok(value)
}

macro_rules! string_value {
    ($name:ident, $doc:literal, $constructor:expr) => {
        #[doc = $doc]
        #[derive(Clone, PartialEq, Eq, Hash)]
        pub struct $name(Box<str>);

        impl $name {
            /// Validates and owns the exact supplied bytes.
            pub fn new(value: impl Into<Box<str>>) -> Result<Self, SchemaIdentityErrorV1> {
                ($constructor)(value.into()).map(Self)
            }

            /// Returns the exact submitted UTF-8 text.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Returns the exact submitted UTF-8 byte length.
            pub fn byte_len(&self) -> usize {
                self.0.len()
            }

            pub(super) fn as_bytes(&self) -> &[u8] {
                self.0.as_bytes()
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_struct(stringify!($name))
                    .field("bytes", &self.byte_len())
                    .finish_non_exhaustive()
            }
        }
    };
}

string_value!(
    IdentifierV1,
    "A non-empty, NUL-free semantic identifier of at most 1,024 UTF-8 bytes.",
    |value| checked_text(
        value,
        MAX_IDENTIFIER_BYTES_V1,
        SchemaIdentityLimitV1::IdentifierBytes,
        SchemaIdentityInvalidCodeV1::Identifier,
        false,
    )
);
string_value!(
    TextValueV1,
    "A NUL-free facet text value of at most 16,384 UTF-8 bytes.",
    |value| checked_text(
        value,
        MAX_FACET_TEXT_BYTES_V1,
        SchemaIdentityLimitV1::FacetTextValueBytes,
        SchemaIdentityInvalidCodeV1::TextValue,
        true,
    )
);
string_value!(
    ProfileIdV1,
    "An opaque profile domain-separation ID in the V1 token grammar.",
    |value| checked_token(value, SchemaIdentityInvalidCodeV1::ProfileId)
);
string_value!(TokenV1, "A facet key in the V1 token grammar.", |value| {
    checked_token(value, SchemaIdentityInvalidCodeV1::Token)
});

/// The complete input from which the three schema digests are built.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SchemaObservationInputV1 {
    /// Independent profile IDs for each digest domain.
    pub profiles: SchemaProfilesV1,
    /// Observed relations and their columns.
    pub relations: Vec<RelationInputV1>,
    /// Observed constraint facts.
    pub constraints: Vec<ConstraintInputV1>,
}

/// Independent profile IDs for the structural, type, and constraint bodies.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SchemaProfilesV1 {
    /// Structural-body profile ID.
    pub structural: ProfileIdV1,
    /// Type-body profile ID.
    pub types: ProfileIdV1,
    /// Constraint-body profile ID.
    pub constraints: ProfileIdV1,
}

/// An observed relation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RelationInputV1 {
    /// Exact qualified relation name.
    pub name: QualifiedNameV1,
    /// Observed relation kind.
    pub kind: RelationKindV1,
    /// Columns, carrying explicit one-based ordinals.
    pub columns: Vec<ColumnInputV1>,
}

/// An observed column and its lossless source type.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ColumnInputV1 {
    /// One-based ordinal within the relation.
    pub ordinal: NonZeroU32,
    /// Exact column name.
    pub name: IdentifierV1,
    /// Source-native type observation.
    pub source_type: SourceTypeV1,
}

/// A source-native column type and its generic observational facets.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SourceTypeV1 {
    /// Exact qualified native type name.
    pub native_name: QualifiedNameV1,
    /// Backend-neutral observational family.
    pub family: TypeFamilyV1,
    /// Profile-defined observational facets.
    pub facets: Vec<TypeFacetV1>,
}

/// One keyed observational type facet.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TypeFacetV1 {
    /// Facet key.
    pub key: TokenV1,
    /// Typed facet payload.
    pub value: TypeFacetValueV1,
}

/// A generic, lossless facet payload admitted by the V1 grammar.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TypeFacetValueV1 {
    /// Boolean value.
    Bool(bool),
    /// Unsigned integer value.
    U64(u64),
    /// Signed integer value.
    I64(i64),
    /// Bounded textual value.
    Text(TextValueV1),
    /// Qualified type name.
    TypeName(QualifiedNameV1),
    /// Ordered list of bounded textual values.
    TextList(Vec<TextValueV1>),
    /// Ordered list of qualified type names.
    TypeNameList(Vec<QualifiedNameV1>),
    /// Exact 32-octet digest value.
    Digest32([u8; 32]),
}

/// A fully qualified semantic name; absent components remain significant.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QualifiedNameV1 {
    /// Optional catalogue component.
    pub catalog: Option<IdentifierV1>,
    /// Optional schema component.
    pub schema: Option<IdentifierV1>,
    /// Mandatory local component.
    pub local: IdentifierV1,
}

/// Relation kinds admitted by the V1 grammar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RelationKindV1 {
    /// A base table.
    BaseTable,
}

/// Backend-neutral observational type families.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypeFamilyV1 {
    Boolean,
    SignedInteger,
    UnsignedInteger,
    ExactNumeric,
    ApproximateNumeric,
    Character,
    Binary,
    Date,
    Time,
    Timestamp,
    Interval,
    Json,
    Uuid,
    Enum,
    Array,
    Domain,
    DynamicPerValue,
    Opaque,
}

/// An observed constraint fact.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ConstraintInputV1 {
    NotNull {
        column: ColumnRefV1,
        state: ConstraintStateV1,
    },
    PrimaryKey {
        relation: RelationRefV1,
        state: ConstraintStateV1,
        columns: Vec<ColumnKeyV1>,
    },
    UniqueKey {
        relation: RelationRefV1,
        state: ConstraintStateV1,
        nulls: UniqueNullSemanticsV1,
        columns: Vec<ColumnKeyV1>,
    },
    ForeignKey {
        child: RelationRefV1,
        parent: RelationRefV1,
        state: ConstraintStateV1,
        match_kind: ForeignKeyMatchV1,
        pairs: Vec<(ColumnKeyV1, ColumnKeyV1)>,
    },
}

/// Validation and enforcement state of an observed constraint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConstraintStateV1 {
    pub validated: bool,
    pub enforced: bool,
}

/// An exact relation reference.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RelationRefV1 {
    pub name: QualifiedNameV1,
}

/// An exact relation-and-column reference.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ColumnRefV1 {
    pub relation: RelationRefV1,
    pub column: ColumnKeyV1,
}

/// An exact one-based ordinal-and-name column key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ColumnKeyV1 {
    pub ordinal: NonZeroU32,
    pub name: IdentifierV1,
}

/// NULL semantics of an observed unique key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UniqueNullSemanticsV1 {
    NullsDistinct,
    NullsNotDistinct,
    ObservedUnknown,
}

/// Match semantics of an observed foreign key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ForeignKeyMatchV1 {
    Simple,
    Full,
    Partial,
    ObservedUnknown,
}

fn fmt_digest(bytes: &[u8; 32], formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    for byte in bytes {
        fmt::Write::write_fmt(formatter, format_args!("{byte:02x}"))?;
    }
    Ok(())
}

macro_rules! digest_value {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name([u8; 32]);

        impl $name {
            pub(super) const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            /// Returns the digest's exact 32 octets.
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt_digest(&self.0, formatter)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "("))?;
                fmt_digest(&self.0, formatter)?;
                formatter.write_str(")")
            }
        }
    };
}

digest_value!(
    StructuralSchemaDigestV1,
    "Opaque SHA-256 digest of the canonical structural body."
);
digest_value!(
    TypeSchemaDigestV1,
    "Opaque SHA-256 digest of the canonical type body."
);
digest_value!(
    ConstraintSchemaDigestV1,
    "Opaque SHA-256 digest of the canonical constraint body."
);

/// The three independent content digests of one admitted schema observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObservedSchemaIdentityV1 {
    structural: StructuralSchemaDigestV1,
    types: TypeSchemaDigestV1,
    constraints: ConstraintSchemaDigestV1,
}

impl ObservedSchemaIdentityV1 {
    /// Validates and canonically hashes a complete observation without I/O.
    pub fn build(input: SchemaObservationInputV1) -> Result<Self, SchemaIdentityErrorV1> {
        super::build(input)
    }

    pub(super) const fn from_digests(
        structural: StructuralSchemaDigestV1,
        types: TypeSchemaDigestV1,
        constraints: ConstraintSchemaDigestV1,
    ) -> Self {
        Self {
            structural,
            types,
            constraints,
        }
    }

    /// Returns the structural digest.
    pub const fn structural(&self) -> StructuralSchemaDigestV1 {
        self.structural
    }

    /// Returns the type digest.
    pub const fn types(&self) -> TypeSchemaDigestV1 {
        self.types
    }

    /// Returns the constraint digest.
    pub const fn constraints(&self) -> ConstraintSchemaDigestV1 {
        self.constraints
    }
}
