use sf_core::schema_identity::{
    IdentifierV1, QualifiedNameV1, SourceTypeV1, TextValueV1, TokenV1, TypeFacetV1,
    TypeFacetValueV1, TypeFamilyV1, MAX_FACET_TEXT_BYTES_V1, MAX_IDENTIFIER_BYTES_V1,
};

use super::{PostgresSchemaIdentityLimitCodeV1, PostgresSchemaIdentityUnavailableV1};

#[derive(Clone, Eq, PartialEq)]
pub(super) struct Postgres16ColumnTypeCatalogFactV1 {
    pub(super) type_oid: u32,
    pub(super) type_namespace: Box<str>,
    pub(super) type_name: Box<str>,
    pub(super) type_kind: char,
    pub(super) type_is_defined: bool,
    pub(super) type_base_oid: u32,
    pub(super) type_element_oid: u32,
    pub(super) type_relation_oid: u32,
    pub(super) array_dimensions: i16,
    pub(super) type_modifier: i32,
    /// `pg_type.typcollation`; the attribute's `attcollation` is carried separately.
    pub(super) collation_oid: u32,
}

#[derive(Clone, Eq, PartialEq)]
/// Per-attribute facts from the default-collation join; absent for non-character attributes.
pub(super) struct Postgres16DefaultCollationCatalogFactV1 {
    pub(super) collation_oid: u32,
    pub(super) collation_namespace: Box<str>,
    pub(super) collation_name: Box<str>,
    pub(super) collation_provider: char,
    pub(super) collation_encoding: i32,
    pub(super) collation_is_deterministic: bool,
    pub(super) database_provider: char,
    pub(super) database_collate: Box<str>,
    pub(super) database_ctype: Box<str>,
    pub(super) database_icu_locale: Option<Box<str>>,
    pub(super) database_icu_rules: Option<Box<str>>,
    pub(super) database_recorded_version: Option<Box<str>>,
    pub(super) actual_version: Option<Box<str>>,
}

pub(super) fn normalize_postgres16_source_type_v1(
    fact: &Postgres16ColumnTypeCatalogFactV1,
    joined_default_collation: Option<&Postgres16DefaultCollationCatalogFactV1>,
) -> Result<SourceTypeV1, PostgresSchemaIdentityUnavailableV1> {
    validate_type_text_bounds(fact)?;
    if fact.type_namespace.as_ref() != "pg_catalog"
        || fact.type_kind != 'b'
        || !fact.type_is_defined
        || fact.type_base_oid != 0
        || fact.type_element_oid != 0
        || fact.type_relation_oid != 0
        || fact.array_dimensions != 0
    {
        return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedType);
    }

    let (family, shape) = registered_type(fact.type_oid, &fact.type_name)?;
    let mut facets = normalize_type_modifier(shape, fact.type_modifier)?;
    if family == TypeFamilyV1::Character {
        facets.extend(normalize_collation(
            fact.collation_oid,
            joined_default_collation,
        )?);
    } else if fact.collation_oid != 0 || joined_default_collation.is_some() {
        return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedCollation);
    }

    Ok(SourceTypeV1 {
        native_name: pg_catalog_name(&fact.type_name)?,
        family,
        facets,
    })
}

#[derive(Clone, Copy)]
enum RegisteredTypeShapeV1 {
    Fixed,
    BitWidth(u64),
    Numeric,
    Character { bounded: bool },
    Temporal { with_time_zone: bool },
}

fn registered_type(
    oid: u32,
    name: &str,
) -> Result<(TypeFamilyV1, RegisteredTypeShapeV1), PostgresSchemaIdentityUnavailableV1> {
    use RegisteredTypeShapeV1 as Shape;
    use TypeFamilyV1 as Family;

    match (oid, name) {
        (16, "bool") => Ok((Family::Boolean, Shape::Fixed)),
        (21, "int2") => Ok((Family::SignedInteger, Shape::BitWidth(16))),
        (23, "int4") => Ok((Family::SignedInteger, Shape::BitWidth(32))),
        (20, "int8") => Ok((Family::SignedInteger, Shape::BitWidth(64))),
        (1700, "numeric") => Ok((Family::ExactNumeric, Shape::Numeric)),
        (700, "float4") => Ok((Family::ApproximateNumeric, Shape::BitWidth(32))),
        (701, "float8") => Ok((Family::ApproximateNumeric, Shape::BitWidth(64))),
        (25, "text") => Ok((Family::Character, Shape::Character { bounded: false })),
        (1043, "varchar") | (1042, "bpchar") => {
            Ok((Family::Character, Shape::Character { bounded: true }))
        }
        (17, "bytea") => Ok((Family::Binary, Shape::Fixed)),
        (1082, "date") => Ok((Family::Date, Shape::Fixed)),
        (1083, "time") => Ok((
            Family::Time,
            Shape::Temporal {
                with_time_zone: false,
            },
        )),
        (1266, "timetz") => Ok((
            Family::Time,
            Shape::Temporal {
                with_time_zone: true,
            },
        )),
        (1114, "timestamp") => Ok((
            Family::Timestamp,
            Shape::Temporal {
                with_time_zone: false,
            },
        )),
        (1184, "timestamptz") => Ok((
            Family::Timestamp,
            Shape::Temporal {
                with_time_zone: true,
            },
        )),
        (114, "json") | (3802, "jsonb") => Ok((Family::Json, Shape::Fixed)),
        (2950, "uuid") => Ok((Family::Uuid, Shape::Fixed)),
        _ => Err(PostgresSchemaIdentityUnavailableV1::UnsupportedType),
    }
}

fn normalize_type_modifier(
    shape: RegisteredTypeShapeV1,
    modifier: i32,
) -> Result<Vec<TypeFacetV1>, PostgresSchemaIdentityUnavailableV1> {
    use RegisteredTypeShapeV1 as Shape;

    let unsupported = || PostgresSchemaIdentityUnavailableV1::UnsupportedType;
    match shape {
        Shape::Fixed => {
            if modifier == -1 {
                Ok(Vec::new())
            } else {
                Err(unsupported())
            }
        }
        Shape::BitWidth(width) => {
            if modifier != -1 {
                return Err(unsupported());
            }
            Ok(vec![facet("bit-width", TypeFacetValueV1::U64(width))?])
        }
        Shape::Numeric => normalize_numeric_modifier(modifier),
        Shape::Character { bounded } => normalize_character_modifier(modifier, bounded),
        Shape::Temporal { with_time_zone } => {
            let precision = match modifier {
                -1 => 6,
                0..=6 => modifier as u64,
                _ => return Err(unsupported()),
            };
            Ok(vec![
                facet(
                    "fractional-second-precision",
                    TypeFacetValueV1::U64(precision),
                )?,
                facet("with-time-zone", TypeFacetValueV1::Bool(with_time_zone))?,
            ])
        }
    }
}

fn normalize_character_modifier(
    modifier: i32,
    bounded: bool,
) -> Result<Vec<TypeFacetV1>, PostgresSchemaIdentityUnavailableV1> {
    if !bounded {
        return (modifier == -1)
            .then(Vec::new)
            .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedType);
    }
    if modifier == -1 {
        return Ok(Vec::new());
    }
    let maximum = i64::from(modifier) - 4;
    if !(1..=10_485_760).contains(&maximum) {
        return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedType);
    }
    Ok(vec![facet(
        "character-maximum",
        TypeFacetValueV1::U64(maximum as u64),
    )?])
}

fn normalize_numeric_modifier(
    modifier: i32,
) -> Result<Vec<TypeFacetV1>, PostgresSchemaIdentityUnavailableV1> {
    if modifier == -1 {
        return Ok(Vec::new());
    }
    if modifier < 4 {
        return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedType);
    }
    let payload = i64::from(modifier) - 4;
    let precision = (payload >> 16) & 0xffff;
    let scale = ((payload & 0x7ff) ^ 0x400) - 0x400;
    let reencoded = ((precision << 16) | (scale & 0x7ff)) + 4;
    if !(1..=1000).contains(&precision)
        || !(-1000..=1000).contains(&scale)
        || reencoded != i64::from(modifier)
    {
        return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedType);
    }
    Ok(vec![
        facet("numeric-precision", TypeFacetValueV1::U64(precision as u64))?,
        facet("numeric-scale", TypeFacetValueV1::I64(scale))?,
    ])
}

fn normalize_collation(
    attribute_collation_oid: u32,
    fact: Option<&Postgres16DefaultCollationCatalogFactV1>,
) -> Result<Vec<TypeFacetV1>, PostgresSchemaIdentityUnavailableV1> {
    let fact = fact.ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedCollation)?;
    validate_collation_text_bounds(fact)?;
    if attribute_collation_oid != 100
        || fact.collation_oid != 100
        || fact.collation_namespace.as_ref() != "pg_catalog"
        || fact.collation_name.as_ref() != "default"
        || fact.collation_provider != 'd'
        || fact.collation_encoding != -1
        || !fact.collation_is_deterministic
        || fact.database_recorded_version.as_deref() != fact.actual_version.as_deref()
    {
        return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedCollation);
    }

    let (provider, icu_locale, icu_rules) = match fact.database_provider {
        'c' if fact.database_icu_locale.is_none() && fact.database_icu_rules.is_none() => {
            ("libc", None, None)
        }
        'i' => {
            let locale = fact
                .database_icu_locale
                .as_deref()
                .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedCollation)?;
            ("icu", Some(locale), fact.database_icu_rules.as_deref())
        }
        _ => return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedCollation),
    };

    let mut facets = vec![
        facet(
            "collation-name",
            TypeFacetValueV1::TypeName(pg_catalog_name("default")?),
        )?,
        text_facet("collation-provider", provider)?,
        facet("collation-deterministic", TypeFacetValueV1::Bool(true))?,
        text_facet("collation-collate", &fact.database_collate)?,
        text_facet("collation-ctype", &fact.database_ctype)?,
    ];
    if let Some(locale) = icu_locale {
        facets.push(text_facet("collation-icu-locale", locale)?);
    }
    if let Some(rules) = icu_rules {
        facets.push(text_facet("collation-icu-rules", rules)?);
    }
    if let Some(version) = fact.database_recorded_version.as_deref() {
        facets.push(text_facet("collation-version", version)?);
    }
    Ok(facets)
}

fn validate_type_text_bounds(
    fact: &Postgres16ColumnTypeCatalogFactV1,
) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
    ensure_text_bound(&fact.type_namespace, MAX_IDENTIFIER_BYTES_V1)?;
    ensure_text_bound(&fact.type_name, MAX_IDENTIFIER_BYTES_V1)
}

fn validate_collation_text_bounds(
    fact: &Postgres16DefaultCollationCatalogFactV1,
) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
    ensure_text_bound(&fact.collation_namespace, MAX_IDENTIFIER_BYTES_V1)?;
    ensure_text_bound(&fact.collation_name, MAX_IDENTIFIER_BYTES_V1)?;
    ensure_text_bound(&fact.database_collate, MAX_FACET_TEXT_BYTES_V1)?;
    ensure_text_bound(&fact.database_ctype, MAX_FACET_TEXT_BYTES_V1)?;
    for value in [
        fact.database_icu_locale.as_deref(),
        fact.database_icu_rules.as_deref(),
        fact.database_recorded_version.as_deref(),
        fact.actual_version.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        ensure_text_bound(value, MAX_FACET_TEXT_BYTES_V1)?;
    }
    Ok(())
}

fn ensure_text_bound(
    value: &str,
    maximum: usize,
) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
    if value.len() <= maximum {
        Ok(())
    } else {
        Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            PostgresSchemaIdentityLimitCodeV1::TextBytes,
        ))
    }
}

fn pg_catalog_name(local: &str) -> Result<QualifiedNameV1, PostgresSchemaIdentityUnavailableV1> {
    Ok(QualifiedNameV1 {
        catalog: None,
        schema: Some(identifier("pg_catalog")?),
        local: identifier(local)?,
    })
}

fn identifier(value: &str) -> Result<IdentifierV1, PostgresSchemaIdentityUnavailableV1> {
    IdentifierV1::new(value).map_err(|_| PostgresSchemaIdentityUnavailableV1::IdentityRejected)
}

fn facet(
    key: &'static str,
    value: TypeFacetValueV1,
) -> Result<TypeFacetV1, PostgresSchemaIdentityUnavailableV1> {
    Ok(TypeFacetV1 {
        key: TokenV1::new(key)
            .map_err(|_| PostgresSchemaIdentityUnavailableV1::IdentityRejected)?,
        value,
    })
}

fn text_facet(
    key: &'static str,
    value: &str,
) -> Result<TypeFacetV1, PostgresSchemaIdentityUnavailableV1> {
    facet(
        key,
        TypeFacetValueV1::Text(
            TextValueV1::new(value)
                .map_err(|_| PostgresSchemaIdentityUnavailableV1::IdentityRejected)?,
        ),
    )
}

#[cfg(test)]
mod adversarial_tests;
#[cfg(test)]
mod tests;
