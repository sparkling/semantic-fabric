use std::collections::{BTreeMap, HashMap, HashSet};
use std::num::NonZeroU32;

use sf_core::schema_identity::{
    ColumnInputV1, IdentifierV1, QualifiedNameV1, RelationInputV1, RelationKindV1,
    MAX_CANONICAL_BODY_BYTES_V1, MAX_COLUMNS_TOTAL_V1, MAX_IDENTIFIER_BYTES_V1, MAX_RELATIONS_V1,
    MAX_UTF8_PAYLOAD_BYTES_V1,
};

use super::source_type::{
    normalize_postgres16_source_type_v1, Postgres16ColumnTypeCatalogFactV1,
    Postgres16DefaultCollationCatalogFactV1,
};
use super::{PostgresSchemaIdentityLimitCodeV1, PostgresSchemaIdentityUnavailableV1};

use self::accounting::{add_semantic_text, add_source_type_text, validate_raw_text_accounting};
mod accounting;
mod legacy;
pub(super) use legacy::compare_postgres16_legacy_coordinates_v1;
pub(super) const MAX_PHYSICAL_ATTRIBUTES_PER_RELATION_PG16_V1: usize = 1_600;
pub(super) const MAX_PHYSICAL_ATTRIBUTES_TOTAL_PG16_V1: usize = MAX_COLUMNS_TOTAL_V1;

#[derive(Clone, Eq, PartialEq)]
pub(super) struct Postgres16RelationCatalogFactV1 {
    pub(super) relation_oid: u32,
    pub(super) relation_namespace_oid: u32,
    pub(super) joined_namespace_oid: u32,
    pub(super) namespace_name: Box<str>,
    pub(super) relation_name: Box<str>,
    pub(super) relation_kind: char,
    pub(super) persistence: char,
    pub(super) is_shared: bool,
    pub(super) is_partition: bool,
    pub(super) row_security: bool,
    pub(super) force_row_security: bool,
    pub(super) of_type_oid: u32,
    pub(super) rewrite_oid: u32,
    pub(super) access_method_oid: u32,
    pub(super) joined_access_method_oid: Option<u32>,
    pub(super) access_method_name: Option<Box<str>>,
    pub(super) access_method_type: Option<char>,
    pub(super) inherits_as_child: bool,
    pub(super) inherits_as_parent: bool,
    pub(super) physical_attribute_count: i16,
}

#[derive(Clone, Eq, PartialEq)]
pub(super) struct Postgres16AttributeCatalogFactV1 {
    pub(super) relation_oid: u32,
    pub(super) attribute_number: i16,
    pub(super) attribute_name: Box<str>,
    pub(super) is_dropped: bool,
    pub(super) is_local: bool,
    pub(super) inheritance_count: i16,
    pub(super) is_not_null: bool,
    pub(super) attribute_type_oid: u32,
    pub(super) array_dimensions: i16,
    pub(super) type_modifier: i32,
    pub(super) collation_oid: u32,
    pub(super) joined_type: Option<Postgres16ColumnTypeCatalogFactV1>,
    pub(super) joined_default_collation: Option<Postgres16DefaultCollationCatalogFactV1>,
}

#[derive(Clone, Eq, PartialEq)]
pub(super) struct Postgres16NormalizedRelationsV1 {
    pub(super) relations: Vec<RelationInputV1>,
    pub(super) coordinates_by_relation_oid: BTreeMap<u32, Postgres16RelationCoordinateV1>,
}

#[derive(Clone, Eq, PartialEq)]
pub(super) struct Postgres16RelationCoordinateV1 {
    pub(super) relation_index: usize,
    pub(super) attributes_by_number: BTreeMap<i16, Postgres16AttributeCoordinateV1>,
}

#[derive(Clone, Eq, PartialEq)]
pub(super) enum Postgres16AttributeCoordinateV1 {
    Dropped,
    Live(Postgres16ColumnCoordinateV1),
}

#[derive(Clone, Eq, PartialEq)]
pub(super) struct Postgres16ColumnCoordinateV1 {
    pub(super) column_index: usize,
    pub(super) type_oid: u32,
    pub(super) collation_oid: u32,
    pub(super) is_not_null: bool,
}

impl Postgres16NormalizedRelationsV1 {
    pub(super) fn relation_by_oid(&self, relation_oid: u32) -> Option<&RelationInputV1> {
        let coordinate = self.coordinates_by_relation_oid.get(&relation_oid)?;
        self.relations.get(coordinate.relation_index)
    }

    pub(super) fn attribute_by_number(
        &self,
        relation_oid: u32,
        attribute_number: i16,
    ) -> Option<&Postgres16AttributeCoordinateV1> {
        self.coordinates_by_relation_oid
            .get(&relation_oid)?
            .attributes_by_number
            .get(&attribute_number)
    }

    pub(super) fn live_column_by_number(
        &self,
        relation_oid: u32,
        attribute_number: i16,
    ) -> Option<(&ColumnInputV1, &Postgres16ColumnCoordinateV1)> {
        let relation_coordinate = self.coordinates_by_relation_oid.get(&relation_oid)?;
        let Postgres16AttributeCoordinateV1::Live(column_coordinate) = relation_coordinate
            .attributes_by_number
            .get(&attribute_number)?
        else {
            return None;
        };
        let column = self
            .relations
            .get(relation_coordinate.relation_index)?
            .columns
            .get(column_coordinate.column_index)?;
        Some((column, column_coordinate))
    }

    pub(super) fn into_relations(self) -> Vec<RelationInputV1> {
        self.relations
    }
}

#[derive(Clone, Copy)]
struct Postgres16RelationLimitsV1 {
    relations: usize,
    physical_per_relation: usize,
    physical_total: usize,
    live_total: usize,
    raw_text_bytes: usize,
    semantic_text_bytes: usize,
}

const PRODUCTION_LIMITS: Postgres16RelationLimitsV1 = Postgres16RelationLimitsV1 {
    relations: MAX_RELATIONS_V1,
    physical_per_relation: MAX_PHYSICAL_ATTRIBUTES_PER_RELATION_PG16_V1,
    physical_total: MAX_PHYSICAL_ATTRIBUTES_TOTAL_PG16_V1,
    live_total: MAX_COLUMNS_TOTAL_V1,
    raw_text_bytes: MAX_CANONICAL_BODY_BYTES_V1,
    semantic_text_bytes: MAX_UTF8_PAYLOAD_BYTES_V1,
};

struct PendingNormalizedRelationV1 {
    relation_oid: u32,
    relation: RelationInputV1,
    attributes_by_number: BTreeMap<i16, Postgres16AttributeCoordinateV1>,
}

pub(super) fn normalize_postgres16_relations_v1(
    candidates: Vec<Postgres16RelationCatalogFactV1>,
    attributes: Vec<Postgres16AttributeCatalogFactV1>,
) -> Result<Postgres16NormalizedRelationsV1, PostgresSchemaIdentityUnavailableV1> {
    normalize_postgres16_relations_with_limits_v1(candidates, attributes, PRODUCTION_LIMITS)
}

fn normalize_postgres16_relations_with_limits_v1(
    candidates: Vec<Postgres16RelationCatalogFactV1>,
    attributes: Vec<Postgres16AttributeCatalogFactV1>,
    limits: Postgres16RelationLimitsV1,
) -> Result<Postgres16NormalizedRelationsV1, PostgresSchemaIdentityUnavailableV1> {
    ensure_collection_bound(
        candidates.len(),
        limits.relations,
        PostgresSchemaIdentityLimitCodeV1::RichRelations,
    )?;
    ensure_collection_bound(
        attributes.len(),
        limits.physical_total,
        PostgresSchemaIdentityLimitCodeV1::PhysicalAttributes,
    )?;
    validate_raw_text_accounting(&candidates, &attributes, limits.raw_text_bytes)?;

    let mut relation_positions = HashMap::with_capacity(candidates.len());
    let mut relation_names = HashSet::with_capacity(candidates.len());
    let mut declared_physical_total = 0usize;
    let mut physical_counts = Vec::with_capacity(candidates.len());
    for (index, candidate) in candidates.iter().enumerate() {
        let physical = validate_relation_candidate(candidate, limits.physical_per_relation)?;
        declared_physical_total = declared_physical_total
            .checked_add(physical)
            .ok_or_else(physical_attribute_limit)?;
        ensure_collection_bound(
            declared_physical_total,
            limits.physical_total,
            PostgresSchemaIdentityLimitCodeV1::PhysicalAttributes,
        )?;
        if relation_positions
            .insert(candidate.relation_oid, index)
            .is_some()
            || !relation_names.insert(candidate.relation_name.as_ref())
        {
            return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedRelation);
        }
        physical_counts.push(physical);
    }
    if attributes.len() != declared_physical_total {
        return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedRelation);
    }

    let mut slots: Vec<Vec<Option<Postgres16AttributeCatalogFactV1>>> = physical_counts
        .into_iter()
        .map(|count| std::iter::repeat_with(|| None).take(count).collect())
        .collect();
    for attribute in attributes {
        ensure_text_bound(&attribute.attribute_name, MAX_IDENTIFIER_BYTES_V1)?;
        let relation_index = relation_positions
            .get(&attribute.relation_oid)
            .copied()
            .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedRelation)?;
        let attribute_number = usize::try_from(attribute.attribute_number)
            .ok()
            .filter(|number| *number > 0)
            .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedRelation)?;
        let slot = slots[relation_index]
            .get_mut(attribute_number - 1)
            .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedRelation)?;
        if slot.replace(attribute).is_some() {
            return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedRelation);
        }
    }

    let mut semantic_text_bytes = 0usize;
    let mut live_total = 0usize;
    let mut pending = Vec::with_capacity(candidates.len());
    for (candidate, relation_slots) in candidates.into_iter().zip(slots) {
        let relation_attributes = relation_slots
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedRelation)?;
        let (normalized, live) = normalize_relation(
            candidate,
            relation_attributes,
            &mut semantic_text_bytes,
            limits.semantic_text_bytes,
        )?;
        live_total = live_total.checked_add(live).ok_or_else(live_column_limit)?;
        ensure_collection_bound(
            live_total,
            limits.live_total,
            PostgresSchemaIdentityLimitCodeV1::LiveColumns,
        )?;
        pending.push(normalized);
    }

    pending.sort_by(|left, right| {
        left.relation
            .name
            .local
            .as_str()
            .as_bytes()
            .cmp(right.relation.name.local.as_str().as_bytes())
    });
    let mut relations = Vec::with_capacity(pending.len());
    let mut coordinates_by_relation_oid = BTreeMap::new();
    for (relation_index, normalized) in pending.into_iter().enumerate() {
        if coordinates_by_relation_oid
            .insert(
                normalized.relation_oid,
                Postgres16RelationCoordinateV1 {
                    relation_index,
                    attributes_by_number: normalized.attributes_by_number,
                },
            )
            .is_some()
        {
            return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedRelation);
        }
        relations.push(normalized.relation);
    }
    Ok(Postgres16NormalizedRelationsV1 {
        relations,
        coordinates_by_relation_oid,
    })
}

fn validate_relation_candidate(
    candidate: &Postgres16RelationCatalogFactV1,
    physical_limit: usize,
) -> Result<usize, PostgresSchemaIdentityUnavailableV1> {
    ensure_text_bound(&candidate.namespace_name, MAX_IDENTIFIER_BYTES_V1)?;
    validate_identifier(&candidate.relation_name)?;
    if let Some(name) = candidate.access_method_name.as_deref() {
        ensure_text_bound(name, MAX_IDENTIFIER_BYTES_V1)?;
    }
    let physical = usize::try_from(candidate.physical_attribute_count)
        .map_err(|_| PostgresSchemaIdentityUnavailableV1::UnsupportedRelation)?;
    ensure_collection_bound(
        physical,
        physical_limit,
        PostgresSchemaIdentityLimitCodeV1::PhysicalAttributes,
    )?;
    if candidate.relation_oid == 0
        || candidate.relation_namespace_oid == 0
        || candidate.relation_namespace_oid != candidate.joined_namespace_oid
        || candidate.namespace_name.as_ref() != "public"
        || candidate.relation_kind != 'r'
        || candidate.persistence != 'p'
        || candidate.is_shared
        || candidate.is_partition
        || candidate.row_security
        || candidate.force_row_security
        || candidate.of_type_oid != 0
        || candidate.rewrite_oid != 0
        || candidate.access_method_oid == 0
        || candidate.joined_access_method_oid != Some(candidate.access_method_oid)
        || candidate.access_method_name.as_deref() != Some("heap")
        || candidate.access_method_type != Some('t')
        || candidate.inherits_as_child
        || candidate.inherits_as_parent
    {
        return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedRelation);
    }
    Ok(physical)
}

fn normalize_relation(
    candidate: Postgres16RelationCatalogFactV1,
    attributes: Vec<Postgres16AttributeCatalogFactV1>,
    semantic_text_bytes: &mut usize,
    semantic_text_limit: usize,
) -> Result<(PendingNormalizedRelationV1, usize), PostgresSchemaIdentityUnavailableV1> {
    let mut live_names = HashSet::new();
    let mut live_count = 0usize;
    for attribute in &attributes {
        if attribute.is_dropped {
            if attribute.attribute_type_oid != 0 || attribute.joined_type.is_some() {
                return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedType);
            }
            if attribute.joined_default_collation.is_some() {
                return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedCollation);
            }
            continue;
        }
        validate_identifier(&attribute.attribute_name)?;
        if !attribute.is_local || attribute.inheritance_count != 0 {
            return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedRelation);
        }
        let joined_type = attribute
            .joined_type
            .as_ref()
            .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedType)?;
        if attribute.attribute_type_oid == 0
            || joined_type.type_oid != attribute.attribute_type_oid
            || joined_type.array_dimensions != attribute.array_dimensions
            || joined_type.type_modifier != attribute.type_modifier
        {
            return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedType);
        }
        if joined_type.collation_oid != attribute.collation_oid {
            return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedCollation);
        }
        if !live_names.insert(attribute.attribute_name.as_ref()) {
            return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedRelation);
        }
        live_count = live_count.checked_add(1).ok_or_else(live_column_limit)?;
    }
    if live_count == 0 {
        return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedRelation);
    }

    let relation_name = IdentifierV1::new(candidate.relation_name)
        .map_err(|_| PostgresSchemaIdentityUnavailableV1::IdentityRejected)?;
    add_semantic_text(semantic_text_bytes, "public", semantic_text_limit)?;
    add_semantic_text(
        semantic_text_bytes,
        relation_name.as_str(),
        semantic_text_limit,
    )?;

    let mut columns = Vec::with_capacity(live_count);
    let mut attributes_by_number = BTreeMap::new();
    for attribute in attributes {
        if attribute.is_dropped {
            attributes_by_number.insert(
                attribute.attribute_number,
                Postgres16AttributeCoordinateV1::Dropped,
            );
            continue;
        }
        let joined_type = attribute
            .joined_type
            .as_ref()
            .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedType)?;
        let source_type = normalize_postgres16_source_type_v1(
            joined_type,
            attribute.joined_default_collation.as_ref(),
        )?;
        let column_name = IdentifierV1::new(attribute.attribute_name)
            .map_err(|_| PostgresSchemaIdentityUnavailableV1::IdentityRejected)?;
        add_semantic_text(
            semantic_text_bytes,
            column_name.as_str(),
            semantic_text_limit,
        )?;
        add_source_type_text(semantic_text_bytes, &source_type, semantic_text_limit)?;
        let column_index = columns.len();
        let ordinal = u32::try_from(column_index + 1)
            .ok()
            .and_then(NonZeroU32::new)
            .ok_or(PostgresSchemaIdentityUnavailableV1::IdentityRejected)?;
        let coordinate = Postgres16ColumnCoordinateV1 {
            column_index,
            type_oid: attribute.attribute_type_oid,
            collation_oid: attribute.collation_oid,
            is_not_null: attribute.is_not_null,
        };
        attributes_by_number.insert(
            attribute.attribute_number,
            Postgres16AttributeCoordinateV1::Live(coordinate),
        );
        columns.push(ColumnInputV1 {
            ordinal,
            name: column_name,
            source_type,
        });
    }
    let relation = RelationInputV1 {
        name: QualifiedNameV1 {
            catalog: None,
            schema: Some(
                IdentifierV1::new("public")
                    .map_err(|_| PostgresSchemaIdentityUnavailableV1::IdentityRejected)?,
            ),
            local: relation_name,
        },
        kind: RelationKindV1::BaseTable,
        columns,
    };
    Ok((
        PendingNormalizedRelationV1 {
            relation_oid: candidate.relation_oid,
            relation,
            attributes_by_number,
        },
        live_count,
    ))
}

fn ensure_collection_bound(
    observed: usize,
    maximum: usize,
    code: PostgresSchemaIdentityLimitCodeV1,
) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
    if observed <= maximum {
        Ok(())
    } else {
        Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(code))
    }
}

fn ensure_text_bound(
    value: &str,
    maximum: usize,
) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
    ensure_collection_bound(
        value.len(),
        maximum,
        PostgresSchemaIdentityLimitCodeV1::TextBytes,
    )
}

fn validate_identifier(value: &str) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
    ensure_text_bound(value, MAX_IDENTIFIER_BYTES_V1)?;
    if value.is_empty() || value.contains('\0') {
        Err(PostgresSchemaIdentityUnavailableV1::IdentityRejected)
    } else {
        Ok(())
    }
}

fn canonical_body_limit() -> PostgresSchemaIdentityUnavailableV1 {
    PostgresSchemaIdentityUnavailableV1::LimitExceeded(
        PostgresSchemaIdentityLimitCodeV1::CanonicalBody,
    )
}

fn physical_attribute_limit() -> PostgresSchemaIdentityUnavailableV1 {
    PostgresSchemaIdentityUnavailableV1::LimitExceeded(
        PostgresSchemaIdentityLimitCodeV1::PhysicalAttributes,
    )
}

fn live_column_limit() -> PostgresSchemaIdentityUnavailableV1 {
    PostgresSchemaIdentityUnavailableV1::LimitExceeded(
        PostgresSchemaIdentityLimitCodeV1::LiveColumns,
    )
}

#[cfg(test)]
mod adversarial_tests;
#[cfg(test)]
mod tests;
