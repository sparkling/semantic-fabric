use std::cmp::Ordering;
use std::collections::HashSet;

use super::encode;
use super::model::*;
use super::{
    check_limit, checked_utf8_add_with_limit, invalid, overflow, KernelLimitsV1,
    NormalizedColumnV1, NormalizedConstraintRecordV1, NormalizedConstraintV1,
    NormalizedObservationV1, NormalizedRelationV1, SchemaIdentityErrorV1,
    SchemaIdentityInvalidCodeV1, SchemaIdentityLimitV1, SchemaIdentityLocationV1,
    SchemaIdentityOperationV1,
};

#[rustfmt::skip]
pub(super) fn normalize(input: SchemaObservationInputV1, limits: KernelLimitsV1)
    -> Result<NormalizedObservationV1, SchemaIdentityErrorV1> {
    validate_limits_and_utf8(&input, limits)?;
    let SchemaObservationInputV1 {
        profiles,
        relations,
        constraints,
    } = input;
    let mut relations = relations
        .into_iter()
        .enumerate()
        .map(|(i, relation)| normalize_relation(i, relation))
        .collect::<Result<Vec<_>, _>>()?;
    relations.sort_by(encode::cmp_relation_coord);
    for pair in relations.windows(2) {
        if encode::cmp_qname(&pair[0].name, &pair[1].name) == Ordering::Equal {
            return Err(invalid(
                SchemaIdentityInvalidCodeV1::DuplicateRelation,
                SchemaIdentityLocationV1::at_relation(pair[1].input_index),
            ));
        }
    }
    let mut constraints = constraints
        .into_iter()
        .enumerate()
        .map(|(i, value)| normalize_constraint(i, value, &relations))
        .collect::<Result<Vec<_>, _>>()?;
    constraints.sort_by(|a, b| encode::cmp_constraint(&a.value, &b.value, &relations));
    constraints.dedup_by(|a, b| a.value == b.value);
    check_limit(
        SchemaIdentityLimitV1::CanonicalConstraints,
        constraints.len(),
        limits.canonical_constraints,
    )?;
    let mut primary_keys = HashSet::new();
    for record in &constraints {
        if let NormalizedConstraintV1::PrimaryKey { relation, .. } = record.value {
            if !primary_keys.insert(relation) {
                return Err(invalid(
                    SchemaIdentityInvalidCodeV1::MultiplePrimaryKey,
                    SchemaIdentityLocationV1::at_constraint(record.input_index),
                ));
            }
        }
    }
    Ok(NormalizedObservationV1 {
        profiles,
        relations,
        constraints,
    })
}

fn normalize_relation(
    index: usize,
    relation: RelationInputV1,
) -> Result<NormalizedRelationV1, SchemaIdentityErrorV1> {
    let ri = index_u32(index)?;
    if relation.columns.is_empty() {
        return Err(invalid(
            SchemaIdentityInvalidCodeV1::EmptyRelation,
            SchemaIdentityLocationV1::at_relation(ri),
        ));
    }
    let mut names = HashSet::new();
    let mut ordinals = HashSet::new();
    for (ci, column) in relation.columns.iter().enumerate() {
        if !ordinals.insert(column.ordinal) {
            return Err(invalid(
                SchemaIdentityInvalidCodeV1::DuplicateColumnOrdinal,
                SchemaIdentityLocationV1::at_column(ri, index_u32(ci)?),
            ));
        }
        if !names.insert(column.name.as_str()) {
            return Err(invalid(
                SchemaIdentityInvalidCodeV1::DuplicateColumnName,
                SchemaIdentityLocationV1::at_column(ri, index_u32(ci)?),
            ));
        }
    }
    let mut columns = relation
        .columns
        .into_iter()
        .enumerate()
        .map(|(ci, mut column)| {
            let mut facet_keys = HashSet::new();
            for (fi, facet) in column.source_type.facets.iter().enumerate() {
                if !facet_keys.insert(facet.key.as_str()) {
                    return Err(invalid(
                        SchemaIdentityInvalidCodeV1::DuplicateFacetKey,
                        SchemaIdentityLocationV1::at_facet(ri, index_u32(ci)?, index_u32(fi)?),
                    ));
                }
            }
            column
                .source_type
                .facets
                .sort_by(|a, b| a.key.as_bytes().cmp(b.key.as_bytes()));
            Ok((
                index_u32(ci)?,
                NormalizedColumnV1 {
                    ordinal: column.ordinal,
                    name: column.name,
                    source_type: column.source_type,
                },
            ))
        })
        .collect::<Result<Vec<_>, SchemaIdentityErrorV1>>()?;
    columns.sort_by_key(|(_, column)| column.ordinal);
    for (position, (input_index, column)) in columns.iter().enumerate() {
        let expected = position
            .checked_add(1)
            .ok_or_else(|| overflow(SchemaIdentityOperationV1::IndexConversion))?;
        if column.ordinal.get() != index_u32(expected)? {
            return Err(invalid(
                SchemaIdentityInvalidCodeV1::OrdinalContinuity,
                SchemaIdentityLocationV1::at_column(ri, *input_index),
            ));
        }
    }
    Ok(NormalizedRelationV1 {
        input_index: ri,
        name: relation.name,
        kind: relation.kind,
        columns: columns.into_iter().map(|(_, column)| column).collect(),
    })
}

fn normalize_constraint(
    index: usize,
    value: ConstraintInputV1,
    relations: &[NormalizedRelationV1],
) -> Result<NormalizedConstraintRecordV1, SchemaIdentityErrorV1> {
    let ci = index_u32(index)?;
    let value = match value {
        ConstraintInputV1::NotNull { column, state } => {
            let relation = resolve_relation(&column.relation, relations, ci)?;
            let column = resolve_column(&column.column, relation, relations, ci, None)?;
            NormalizedConstraintV1::NotNull {
                relation,
                column,
                state,
            }
        }
        ConstraintInputV1::PrimaryKey {
            relation,
            state,
            columns,
        } => {
            let relation = resolve_relation(&relation, relations, ci)?;
            let columns = resolve_key(columns, relation, relations, ci)?;
            NormalizedConstraintV1::PrimaryKey {
                relation,
                state,
                columns,
            }
        }
        ConstraintInputV1::UniqueKey {
            relation,
            state,
            nulls,
            columns,
        } => {
            let relation = resolve_relation(&relation, relations, ci)?;
            let mut columns = resolve_key(columns, relation, relations, ci)?;
            columns.sort_by(|a, b| {
                encode::cmp_column_key(
                    &relations[relation].columns[*a],
                    &relations[relation].columns[*b],
                )
            });
            NormalizedConstraintV1::UniqueKey {
                relation,
                state,
                nulls,
                columns,
            }
        }
        ConstraintInputV1::ForeignKey {
            child,
            parent,
            state,
            match_kind,
            pairs,
        } => {
            if pairs.is_empty() {
                return Err(invalid(
                    SchemaIdentityInvalidCodeV1::EmptyKey,
                    SchemaIdentityLocationV1::at_constraint(ci),
                ));
            }
            let child = resolve_relation(&child, relations, ci)?;
            let parent = resolve_relation(&parent, relations, ci)?;
            let mut exact = HashSet::new();
            let mut children = HashSet::new();
            let mut parents = HashSet::new();
            let mut resolved = Vec::with_capacity(pairs.len());
            for (mi, (child_key, parent_key)) in pairs.into_iter().enumerate() {
                let location = SchemaIdentityLocationV1::at_member(ci, index_u32(mi)?);
                let child_column = resolve_column(&child_key, child, relations, ci, Some(mi))?;
                let parent_column = resolve_column(&parent_key, parent, relations, ci, Some(mi))?;
                if !exact.insert((child_column, parent_column)) {
                    return Err(invalid(
                        SchemaIdentityInvalidCodeV1::DuplicateForeignKeyPair,
                        location,
                    ));
                }
                if !children.insert(child_column) {
                    return Err(invalid(
                        SchemaIdentityInvalidCodeV1::DuplicateForeignKeyChild,
                        location,
                    ));
                }
                if !parents.insert(parent_column) {
                    return Err(invalid(
                        SchemaIdentityInvalidCodeV1::DuplicateForeignKeyParent,
                        location,
                    ));
                }
                resolved.push((child_column, parent_column));
            }
            NormalizedConstraintV1::ForeignKey {
                child,
                parent,
                state,
                match_kind,
                pairs: resolved,
            }
        }
    };
    Ok(NormalizedConstraintRecordV1 {
        input_index: ci,
        value,
    })
}

fn resolve_key(
    columns: Vec<ColumnKeyV1>,
    relation: usize,
    relations: &[NormalizedRelationV1],
    constraint: u32,
) -> Result<Vec<usize>, SchemaIdentityErrorV1> {
    if columns.is_empty() {
        return Err(invalid(
            SchemaIdentityInvalidCodeV1::EmptyKey,
            SchemaIdentityLocationV1::at_constraint(constraint),
        ));
    }
    let mut seen = HashSet::new();
    let mut resolved = Vec::with_capacity(columns.len());
    for (mi, key) in columns.into_iter().enumerate() {
        let location = SchemaIdentityLocationV1::at_member(constraint, index_u32(mi)?);
        let column = resolve_column(&key, relation, relations, constraint, Some(mi))?;
        if !seen.insert(column) {
            return Err(invalid(
                SchemaIdentityInvalidCodeV1::DuplicateKeyMember,
                location,
            ));
        }
        resolved.push(column);
    }
    Ok(resolved)
}

fn resolve_relation(
    reference: &RelationRefV1,
    relations: &[NormalizedRelationV1],
    ci: u32,
) -> Result<usize, SchemaIdentityErrorV1> {
    relations
        .binary_search_by(|relation| encode::cmp_qname(&relation.name, &reference.name))
        .map_err(|_| {
            invalid(
                SchemaIdentityInvalidCodeV1::DanglingRelation,
                SchemaIdentityLocationV1::at_constraint(ci),
            )
        })
}

fn resolve_column(
    key: &ColumnKeyV1,
    relation: usize,
    relations: &[NormalizedRelationV1],
    ci: u32,
    member: Option<usize>,
) -> Result<usize, SchemaIdentityErrorV1> {
    let location = match member {
        Some(value) => SchemaIdentityLocationV1::at_member(ci, index_u32(value)?),
        None => SchemaIdentityLocationV1::at_constraint(ci),
    };
    relations[relation]
        .columns
        .binary_search_by_key(&key.ordinal, |column| column.ordinal)
        .ok()
        .filter(|index| relations[relation].columns[*index].name == key.name)
        .ok_or_else(|| invalid(SchemaIdentityInvalidCodeV1::DanglingColumn, location))
}

#[rustfmt::skip]
fn validate_limits_and_utf8(input: &SchemaObservationInputV1, limits: KernelLimitsV1)
    -> Result<u64, SchemaIdentityErrorV1> {
    check_limit(
        SchemaIdentityLimitV1::Relations,
        input.relations.len(),
        limits.relations,
    )?;
    check_limit(
        SchemaIdentityLimitV1::RawConstraints,
        input.constraints.len(),
        limits.raw_constraints,
    )?;
    let mut columns = 0usize;
    let mut facets = 0usize;
    let mut list_items = 0usize;
    let mut utf8 = 0u64;
    add_text(&mut utf8, input.profiles.structural.as_str(), limits)?;
    add_text(&mut utf8, input.profiles.types.as_str(), limits)?;
    add_text(&mut utf8, input.profiles.constraints.as_str(), limits)?;
    for relation in &input.relations {
        add_qname(&mut utf8, &relation.name, limits)?;
        check_limit(
            SchemaIdentityLimitV1::ColumnsPerRelation,
            relation.columns.len(),
            limits.columns_per_relation,
        )?;
        columns = checked_add_usize(columns, relation.columns.len())?;
        check_limit(
            SchemaIdentityLimitV1::ColumnsTotal,
            columns,
            limits.columns_total,
        )?;
        for column in &relation.columns {
            add_text(&mut utf8, column.name.as_str(), limits)?;
            add_qname(&mut utf8, &column.source_type.native_name, limits)?;
            check_limit(
                SchemaIdentityLimitV1::FacetsPerColumn,
                column.source_type.facets.len(),
                limits.facets_per_column,
            )?;
            facets = checked_add_usize(facets, column.source_type.facets.len())?;
            check_limit(
                SchemaIdentityLimitV1::FacetsTotal,
                facets,
                limits.facets_total,
            )?;
            for facet in &column.source_type.facets {
                add_text(&mut utf8, facet.key.as_str(), limits)?;
                count_facet_value(&facet.value, &mut utf8, &mut list_items, limits)?;
            }
        }
    }
    for constraint in &input.constraints {
        count_constraint(constraint, &mut utf8, limits)?;
    }
    Ok(utf8)
}

#[rustfmt::skip]
fn count_facet_value(value: &TypeFacetValueV1, utf8: &mut u64, total: &mut usize,
    limits: KernelLimitsV1) -> Result<(), SchemaIdentityErrorV1> {
    match value {
        TypeFacetValueV1::Text(value) => add_text(utf8, value.as_str(), limits)?,
        TypeFacetValueV1::TypeName(value) => add_qname(utf8, value, limits)?,
        TypeFacetValueV1::TextList(values) => {
            check_limit(
                SchemaIdentityLimitV1::FacetListItems,
                values.len(),
                limits.facet_list_items,
            )?;
            *total = checked_add_usize(*total, values.len())?;
            check_limit(
                SchemaIdentityLimitV1::FacetListItemsTotal,
                *total,
                limits.facet_list_items_total,
            )?;
            for value in values {
                add_text(utf8, value.as_str(), limits)?;
            }
        }
        TypeFacetValueV1::TypeNameList(values) => {
            check_limit(
                SchemaIdentityLimitV1::FacetListItems,
                values.len(),
                limits.facet_list_items,
            )?;
            *total = checked_add_usize(*total, values.len())?;
            check_limit(
                SchemaIdentityLimitV1::FacetListItemsTotal,
                *total,
                limits.facet_list_items_total,
            )?;
            for value in values {
                add_qname(utf8, value, limits)?;
            }
        }
        TypeFacetValueV1::Bool(_)
        | TypeFacetValueV1::U64(_)
        | TypeFacetValueV1::I64(_)
        | TypeFacetValueV1::Digest32(_) => {}
    }
    Ok(())
}

#[rustfmt::skip]
fn count_constraint(value: &ConstraintInputV1, utf8: &mut u64, limits: KernelLimitsV1)
    -> Result<(), SchemaIdentityErrorV1> {
    match value {
        ConstraintInputV1::NotNull { column, .. } => {
            add_qname(utf8, &column.relation.name, limits)?;
            add_text(utf8, column.column.name.as_str(), limits)?;
        }
        ConstraintInputV1::PrimaryKey {
            relation, columns, ..
        }
        | ConstraintInputV1::UniqueKey {
            relation, columns, ..
        } => {
            check_limit(
                SchemaIdentityLimitV1::KeyMembers,
                columns.len(),
                limits.key_members,
            )?;
            add_qname(utf8, &relation.name, limits)?;
            for column in columns {
                add_text(utf8, column.name.as_str(), limits)?;
            }
        }
        ConstraintInputV1::ForeignKey {
            child,
            parent,
            pairs,
            ..
        } => {
            check_limit(
                SchemaIdentityLimitV1::KeyMembers,
                pairs.len(),
                limits.key_members,
            )?;
            add_qname(utf8, &child.name, limits)?;
            add_qname(utf8, &parent.name, limits)?;
            for (child, parent) in pairs {
                add_text(utf8, child.name.as_str(), limits)?;
                add_text(utf8, parent.name.as_str(), limits)?;
            }
        }
    }
    Ok(())
}

#[rustfmt::skip]
fn add_qname(total: &mut u64, value: &QualifiedNameV1, limits: KernelLimitsV1)
    -> Result<(), SchemaIdentityErrorV1> {
    if let Some(value) = &value.catalog {
        add_text(total, value.as_str(), limits)?;
    }
    if let Some(value) = &value.schema {
        add_text(total, value.as_str(), limits)?;
    }
    add_text(total, value.local.as_str(), limits)
}

#[rustfmt::skip]
fn add_text(total: &mut u64, value: &str, limits: KernelLimitsV1)
    -> Result<(), SchemaIdentityErrorV1> {
    *total = checked_utf8_add_with_limit(*total, value.len(), limits.utf8_payload_bytes)?;
    Ok(())
}

fn checked_add_usize(left: usize, right: usize) -> Result<usize, SchemaIdentityErrorV1> {
    left.checked_add(right)
        .ok_or_else(|| overflow(SchemaIdentityOperationV1::IndexConversion))
}

fn index_u32(index: usize) -> Result<u32, SchemaIdentityErrorV1> {
    u32::try_from(index).map_err(|_| overflow(SchemaIdentityOperationV1::IndexConversion))
}

#[cfg(test)]
pub(super) fn submitted_utf8_bytes(
    input: &SchemaObservationInputV1,
) -> Result<u64, SchemaIdentityErrorV1> {
    validate_limits_and_utf8(input, KernelLimitsV1::PRODUCTION)
}
