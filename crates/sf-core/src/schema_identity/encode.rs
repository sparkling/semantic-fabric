use std::cmp::Ordering;

use sha2::{Digest, Sha256};

use super::model::*;
use super::validate::normalize;
use super::{
    check_body_limit_with_max, checked_body_add, invalid, overflow, KernelLimitsV1,
    NormalizedColumnV1, NormalizedConstraintV1, NormalizedObservationV1, NormalizedRelationV1,
    SchemaIdentityErrorV1, SchemaIdentityInvalidCodeV1, SchemaIdentityLimitV1,
    SchemaIdentityLocationV1, SchemaIdentityOperationV1,
};

const STRUCTURAL_DOMAIN: &[u8] = b"semantic-fabric:observed-schema:structural:v1\0";
const TYPE_DOMAIN: &[u8] = b"semantic-fabric:observed-schema:type:v1\0";
const CONSTRAINT_DOMAIN: &[u8] = b"semantic-fabric:observed-schema:constraint:v1\0";

#[cfg(test)]
#[rustfmt::skip]
pub(super) struct CapturedPreimagesV1 {
    pub structural: Vec<u8>, pub types: Vec<u8>, pub constraints: Vec<u8>,
}

#[rustfmt::skip]
pub(super) fn build(input: SchemaObservationInputV1) -> Result<ObservedSchemaIdentityV1, SchemaIdentityErrorV1> {
    build_internal(input, false, KernelLimitsV1::PRODUCTION).map(|(identity, _)| identity) }

#[cfg(test)]
#[rustfmt::skip]
pub(super) fn build_with_preimages(input: SchemaObservationInputV1)
    -> Result<(ObservedSchemaIdentityV1, CapturedPreimagesV1), SchemaIdentityErrorV1> {
    let (identity, captured) = build_internal(input, true, KernelLimitsV1::PRODUCTION)?;
    let [structural, types, constraints] = captured.expect("test capture requested");
    Ok((identity, CapturedPreimagesV1 { structural, types, constraints }))
}

#[cfg(test)]
#[rustfmt::skip]
pub(super) fn build_with_limits(input: SchemaObservationInputV1, limits: KernelLimitsV1)
    -> Result<ObservedSchemaIdentityV1, SchemaIdentityErrorV1> {
    build_internal(input, false, limits).map(|(identity, _)| identity) }

type Captures = Option<[Vec<u8>; 3]>;

#[rustfmt::skip]
struct PlannedObservationV1 { observation: NormalizedObservationV1, lengths: [u64; 3] }

#[rustfmt::skip]
fn build_internal(input: SchemaObservationInputV1, capture: bool, limits: KernelLimitsV1)
    -> Result<(ObservedSchemaIdentityV1, Captures), SchemaIdentityErrorV1> {
    hash_plan(validate_and_plan(input, limits)?, capture) }

#[rustfmt::skip]
fn validate_and_plan(input: SchemaObservationInputV1, limits: KernelLimitsV1)
    -> Result<PlannedObservationV1, SchemaIdentityErrorV1> {
    let observation = normalize(input, limits)?;
    let lengths = [body_length(|sink| write_structural_body(sink, &observation))?,
        body_length(|sink| write_type_body(sink, &observation))?,
        body_length(|sink| write_constraint_body(sink, &observation))?];
    check_body_limit_with_max(SchemaIdentityLimitV1::StructuralBodyBytes,
        lengths[0], limits.structural_body_bytes)?;
    check_body_limit_with_max(SchemaIdentityLimitV1::TypeBodyBytes,
        lengths[1], limits.type_body_bytes)?;
    check_body_limit_with_max(SchemaIdentityLimitV1::ConstraintBodyBytes,
        lengths[2], limits.constraint_body_bytes)?;
    Ok(PlannedObservationV1 { observation, lengths })
}

#[rustfmt::skip]
fn hash_plan(plan: PlannedObservationV1, capture: bool)
    -> Result<(ObservedSchemaIdentityV1, Captures), SchemaIdentityErrorV1> {
    let PlannedObservationV1 { observation, lengths } = plan;
    let (structural, structural_bytes) = digest_body(STRUCTURAL_DOMAIN, lengths[0],
        capture, |sink| write_structural_body(sink, &observation))?;
    let (types, type_bytes) = digest_body(TYPE_DOMAIN, lengths[1],
        capture, |sink| write_type_body(sink, &observation))?;
    let (constraints, constraint_bytes) = digest_body(CONSTRAINT_DOMAIN, lengths[2],
        capture, |sink| write_constraint_body(sink, &observation))?;
    let identity = ObservedSchemaIdentityV1::from_digests(
        StructuralSchemaDigestV1::from_bytes(structural),
        TypeSchemaDigestV1::from_bytes(types),
        ConstraintSchemaDigestV1::from_bytes(constraints),
    );
    let captures = structural_bytes
        .zip(type_bytes)
        .zip(constraint_bytes)
        .map(|((structural, types), constraints)| [structural, types, constraints]);
    Ok((identity, captures))
}

pub(super) trait ByteSink {
    fn write(&mut self, bytes: &[u8]) -> Result<(), SchemaIdentityErrorV1>;
    fn written(&self) -> u64;
}

struct CountingSink(u64);

#[rustfmt::skip]
impl ByteSink for CountingSink {
    fn write(&mut self, bytes: &[u8]) -> Result<(), SchemaIdentityErrorV1> {
        let added = u64::try_from(bytes.len()).map_err(|_|
            overflow(SchemaIdentityOperationV1::CanonicalBodyAccounting))?;
        self.0 = checked_body_add(self.0, added)?; Ok(())
    }
    fn written(&self) -> u64 { self.0 }
}

struct DigestSink {
    hash: Sha256,
    captured: Option<Vec<u8>>,
    written: u64,
}

#[rustfmt::skip]
impl DigestSink {
    fn new(capture: bool, capacity: u64) -> Result<Self, SchemaIdentityErrorV1> {
        let captured = if capture {
            Some(Vec::with_capacity(usize::try_from(capacity).map_err(|_|
                overflow(SchemaIdentityOperationV1::CanonicalStreamAccounting))?))
        } else {
            None
        };
        Ok(Self { hash: Sha256::new(), captured, written: 0 })
    }

    fn finish(self) -> ([u8; 32], Option<Vec<u8>>) {
        (self.hash.finalize().into(), self.captured)
    }
}

#[rustfmt::skip]
impl ByteSink for DigestSink {
    fn write(&mut self, bytes: &[u8]) -> Result<(), SchemaIdentityErrorV1> {
        let added = u64::try_from(bytes.len()).map_err(|_|
            overflow(SchemaIdentityOperationV1::CanonicalStreamAccounting))?;
        self.written = self.written.checked_add(added).ok_or_else(||
            overflow(SchemaIdentityOperationV1::CanonicalStreamAccounting))?;
        self.hash.update(bytes);
        if let Some(captured) = &mut self.captured { captured.extend_from_slice(bytes); }
        Ok(())
    }
    fn written(&self) -> u64 { self.written }
}

#[rustfmt::skip]
fn body_length(write: impl FnOnce(&mut dyn ByteSink) -> Result<(), SchemaIdentityErrorV1>)
    -> Result<u64, SchemaIdentityErrorV1> {
    let mut sink = CountingSink(0);
    write(&mut sink)?;
    Ok(sink.written())
}

#[rustfmt::skip]
pub(super) fn digest_body(domain: &[u8], body_length: u64, capture: bool,
    write: impl FnOnce(&mut dyn ByteSink) -> Result<(), SchemaIdentityErrorV1>)
    -> Result<([u8; 32], Option<Vec<u8>>), SchemaIdentityErrorV1> {
    let capacity = u64::try_from(domain.len()).map_err(|_|
        overflow(SchemaIdentityOperationV1::CanonicalStreamAccounting))?
        .checked_add(8)
        .and_then(|value| value.checked_add(body_length))
        .ok_or_else(|| overflow(SchemaIdentityOperationV1::CanonicalStreamAccounting))?;
    let mut sink = DigestSink::new(capture, capacity)?;
    sink.write(domain)?;
    sink.write(&body_length.to_be_bytes())?;
    let body_start = sink.written();
    write(&mut sink)?;
    if sink.written().checked_sub(body_start) != Some(body_length) {
        return Err(invalid(SchemaIdentityInvalidCodeV1::CanonicalLengthMismatch,
            SchemaIdentityLocationV1::root()));
    }
    Ok(sink.finish())
}

#[rustfmt::skip]
fn write_structural_body(sink: &mut dyn ByteSink, value: &NormalizedObservationV1)
    -> Result<(), SchemaIdentityErrorV1> {
    write_txt_bytes(sink, value.profiles.structural.as_bytes())?;
    write_count(sink, value.relations.len())?;
    for relation in &value.relations {
        write_u8(sink, 0x11)?;
        write_relation_coord(sink, relation)?;
        write_count(sink, relation.columns.len())?;
        for column in &relation.columns {
            write_u8(sink, 0x12)?;
            write_column_key(sink, column)?;
        }
    }
    Ok(())
}

#[rustfmt::skip]
fn write_type_body(sink: &mut dyn ByteSink, value: &NormalizedObservationV1)
    -> Result<(), SchemaIdentityErrorV1> {
    write_txt_bytes(sink, value.profiles.types.as_bytes())?;
    let total = value.relations.iter().try_fold(0usize, |total, relation| {
        total
            .checked_add(relation.columns.len())
            .ok_or_else(|| overflow(SchemaIdentityOperationV1::CanonicalBodyAccounting))
    })?;
    write_count(sink, total)?;
    for relation in &value.relations {
        for column in &relation.columns {
            write_u8(sink, 0x21)?;
            write_relation_coord(sink, relation)?;
            write_column_key(sink, column)?;
            write_qname(sink, &column.source_type.native_name)?;
            write_u8(sink, type_family_tag(column.source_type.family))?;
            write_count(sink, column.source_type.facets.len())?;
            for facet in &column.source_type.facets {
                write_u8(sink, 0x22)?;
                write_txt(sink, facet.key.as_str())?;
                write_facet_value(sink, &facet.value)?;
            }
        }
    }
    Ok(())
}

fn write_constraint_body(
    sink: &mut dyn ByteSink,
    value: &NormalizedObservationV1,
) -> Result<(), SchemaIdentityErrorV1> {
    write_txt_bytes(sink, value.profiles.constraints.as_bytes())?;
    write_count(sink, value.constraints.len())?;
    for record in &value.constraints {
        write_constraint(sink, &record.value, &value.relations)?;
    }
    Ok(())
}

#[rustfmt::skip]
pub(super) fn write_constraint(sink: &mut dyn ByteSink, value: &NormalizedConstraintV1,
    relations: &[NormalizedRelationV1]) -> Result<(), SchemaIdentityErrorV1> {
    match value {
        NormalizedConstraintV1::NotNull { relation, column, state } => {
            write_u8(sink, 0x31)?; write_relation_coord(sink, &relations[*relation])?;
            write_column_key(sink, &relations[*relation].columns[*column])?; write_state(sink, *state)?;
        }
        NormalizedConstraintV1::PrimaryKey { relation, state, columns } => {
            write_u8(sink, 0x32)?; write_relation_coord(sink, &relations[*relation])?;
            write_state(sink, *state)?; write_count(sink, columns.len())?;
            for column in columns { write_column_key(sink, &relations[*relation].columns[*column])?; }
        }
        NormalizedConstraintV1::UniqueKey { relation, state, nulls, columns } => {
            write_u8(sink, 0x33)?; write_relation_coord(sink, &relations[*relation])?;
            write_state(sink, *state)?; write_u8(sink, unique_null_tag(*nulls))?;
            write_count(sink, columns.len())?;
            for column in columns { write_column_key(sink, &relations[*relation].columns[*column])?; }
        }
        NormalizedConstraintV1::ForeignKey { child, parent, state, match_kind, pairs } => {
            write_u8(sink, 0x34)?; write_relation_coord(sink, &relations[*child])?;
            write_relation_coord(sink, &relations[*parent])?; write_state(sink, *state)?;
            write_u8(sink, foreign_match_tag(*match_kind))?; write_count(sink, pairs.len())?;
            for (child_column, parent_column) in pairs {
                write_column_key(sink, &relations[*child].columns[*child_column])?;
                write_column_key(sink, &relations[*parent].columns[*parent_column])?;
            }
        }
    }
    Ok(())
}

#[rustfmt::skip]
fn write_facet_value(sink: &mut dyn ByteSink, value: &TypeFacetValueV1)
    -> Result<(), SchemaIdentityErrorV1> {
    write_u8(sink, facet_value_tag(value))?;
    match value {
        TypeFacetValueV1::Bool(value) => write_bool(sink, *value),
        TypeFacetValueV1::U64(value) => sink.write(&value.to_be_bytes()),
        TypeFacetValueV1::I64(value) => sink.write(&value.to_be_bytes()),
        TypeFacetValueV1::Text(value) => write_txt_bytes(sink, value.as_bytes()),
        TypeFacetValueV1::TypeName(value) => write_qname(sink, value),
        TypeFacetValueV1::TextList(values) => {
            write_count(sink, values.len())?;
            for value in values { write_txt_bytes(sink, value.as_bytes())?; }
            Ok(())
        }
        TypeFacetValueV1::TypeNameList(values) => {
            write_count(sink, values.len())?;
            for value in values { write_qname(sink, value)?; }
            Ok(())
        }
        TypeFacetValueV1::Digest32(value) => sink.write(value),
    }
}

#[rustfmt::skip]
pub(super) fn write_relation_coord(sink: &mut dyn ByteSink, value: &NormalizedRelationV1)
    -> Result<(), SchemaIdentityErrorV1> {
    write_qname(sink, &value.name)?;
    write_u8(sink, relation_kind_tag(value.kind))
}

#[rustfmt::skip]
pub(super) fn write_column_key(sink: &mut dyn ByteSink, value: &NormalizedColumnV1)
    -> Result<(), SchemaIdentityErrorV1> {
    sink.write(&value.ordinal.get().to_be_bytes())?;
    write_txt(sink, value.name.as_str())
}

#[rustfmt::skip]
pub(super) fn write_qname(sink: &mut dyn ByteSink, value: &QualifiedNameV1)
    -> Result<(), SchemaIdentityErrorV1> {
    write_optional_identifier(sink, value.catalog.as_ref())?;
    write_optional_identifier(sink, value.schema.as_ref())?;
    write_txt(sink, value.local.as_str())
}

fn write_optional_identifier(
    sink: &mut dyn ByteSink,
    value: Option<&IdentifierV1>,
) -> Result<(), SchemaIdentityErrorV1> {
    match value {
        None => write_u8(sink, 0),
        Some(value) => {
            write_u8(sink, 1)?;
            write_txt(sink, value.as_str())
        }
    }
}

fn write_state(
    sink: &mut dyn ByteSink,
    value: ConstraintStateV1,
) -> Result<(), SchemaIdentityErrorV1> {
    write_bool(sink, value.validated)?;
    write_bool(sink, value.enforced)
}

fn write_txt(sink: &mut dyn ByteSink, value: &str) -> Result<(), SchemaIdentityErrorV1> {
    write_txt_bytes(sink, value.as_bytes())
}

fn write_txt_bytes(sink: &mut dyn ByteSink, value: &[u8]) -> Result<(), SchemaIdentityErrorV1> {
    write_count(sink, value.len())?;
    sink.write(value)
}

fn write_count(sink: &mut dyn ByteSink, value: usize) -> Result<(), SchemaIdentityErrorV1> {
    let value = u32::try_from(value)
        .map_err(|_| overflow(SchemaIdentityOperationV1::U32LengthConversion))?;
    sink.write(&value.to_be_bytes())
}

fn write_u8(sink: &mut dyn ByteSink, value: u8) -> Result<(), SchemaIdentityErrorV1> {
    sink.write(&[value])
}

fn write_bool(sink: &mut dyn ByteSink, value: bool) -> Result<(), SchemaIdentityErrorV1> {
    write_u8(sink, u8::from(value))
}

#[rustfmt::skip]
pub(super) const fn relation_kind_tag(value: RelationKindV1) -> u8 {
    match value { RelationKindV1::BaseTable => 0x01 }
}
#[rustfmt::skip]
pub(super) const fn unique_null_tag(value: UniqueNullSemanticsV1) -> u8 {
    match value { UniqueNullSemanticsV1::NullsDistinct => 0x01,
        UniqueNullSemanticsV1::NullsNotDistinct => 0x02,
        UniqueNullSemanticsV1::ObservedUnknown => 0x03 }
}
#[rustfmt::skip]
pub(super) const fn foreign_match_tag(value: ForeignKeyMatchV1) -> u8 {
    match value { ForeignKeyMatchV1::Simple => 0x01, ForeignKeyMatchV1::Full => 0x02,
        ForeignKeyMatchV1::Partial => 0x03, ForeignKeyMatchV1::ObservedUnknown => 0x04 }
}
#[rustfmt::skip]
pub(super) const fn type_family_tag(value: TypeFamilyV1) -> u8 {
    match value {
        TypeFamilyV1::Boolean => 0x01, TypeFamilyV1::SignedInteger => 0x02,
        TypeFamilyV1::UnsignedInteger => 0x03, TypeFamilyV1::ExactNumeric => 0x04,
        TypeFamilyV1::ApproximateNumeric => 0x05, TypeFamilyV1::Character => 0x06,
        TypeFamilyV1::Binary => 0x07, TypeFamilyV1::Date => 0x08,
        TypeFamilyV1::Time => 0x09, TypeFamilyV1::Timestamp => 0x0a,
        TypeFamilyV1::Interval => 0x0b, TypeFamilyV1::Json => 0x0c,
        TypeFamilyV1::Uuid => 0x0d, TypeFamilyV1::Enum => 0x0e,
        TypeFamilyV1::Array => 0x0f, TypeFamilyV1::Domain => 0x10,
        TypeFamilyV1::DynamicPerValue => 0x11, TypeFamilyV1::Opaque => 0x7f,
    }
}

#[rustfmt::skip]
pub(super) const fn facet_value_tag(value: &TypeFacetValueV1) -> u8 {
    match value {
        TypeFacetValueV1::Bool(_) => 0x01, TypeFacetValueV1::U64(_) => 0x02,
        TypeFacetValueV1::I64(_) => 0x03, TypeFacetValueV1::Text(_) => 0x04,
        TypeFacetValueV1::TypeName(_) => 0x05, TypeFacetValueV1::TextList(_) => 0x06,
        TypeFacetValueV1::TypeNameList(_) => 0x07, TypeFacetValueV1::Digest32(_) => 0x08,
    }
}

pub(super) fn cmp_qname(left: &QualifiedNameV1, right: &QualifiedNameV1) -> Ordering {
    cmp_optional_txt(left.catalog.as_ref(), right.catalog.as_ref())
        .then_with(|| cmp_optional_txt(left.schema.as_ref(), right.schema.as_ref()))
        .then_with(|| cmp_txt(left.local.as_bytes(), right.local.as_bytes()))
}

pub(super) fn cmp_relation_coord(
    left: &NormalizedRelationV1,
    right: &NormalizedRelationV1,
) -> Ordering {
    cmp_qname(&left.name, &right.name)
        .then_with(|| relation_kind_tag(left.kind).cmp(&relation_kind_tag(right.kind)))
}

pub(super) fn cmp_column_key(left: &NormalizedColumnV1, right: &NormalizedColumnV1) -> Ordering {
    left.ordinal
        .cmp(&right.ordinal)
        .then_with(|| cmp_txt(left.name.as_bytes(), right.name.as_bytes()))
}

#[rustfmt::skip]
pub(super) fn cmp_constraint(left: &NormalizedConstraintV1, right: &NormalizedConstraintV1,
    relations: &[NormalizedRelationV1]) -> Ordering {
    constraint_tag(left)
        .cmp(&constraint_tag(right))
        .then_with(|| match (left, right) {
            (NormalizedConstraintV1::NotNull { relation: lr, column: lc, state: ls },
             NormalizedConstraintV1::NotNull { relation: rr, column: rc, state: rs }) =>
                cmp_relation_coord(&relations[*lr], &relations[*rr])
                .then_with(|| cmp_column_key(&relations[*lr].columns[*lc], &relations[*rr].columns[*rc]))
                .then_with(|| cmp_state(*ls, *rs)),
            (NormalizedConstraintV1::PrimaryKey { relation: lr, state: ls, columns: lc },
             NormalizedConstraintV1::PrimaryKey { relation: rr, state: rs, columns: rc }) =>
                cmp_relation_coord(&relations[*lr], &relations[*rr])
                .then_with(|| cmp_state(*ls, *rs))
                .then_with(|| lc.len().cmp(&rc.len()))
                .then_with(|| cmp_columns(lc, rc, &relations[*lr], &relations[*rr])),
            (NormalizedConstraintV1::UniqueKey { relation: lr, state: ls, nulls: ln, columns: lc },
             NormalizedConstraintV1::UniqueKey { relation: rr, state: rs, nulls: rn, columns: rc }) =>
                cmp_relation_coord(&relations[*lr], &relations[*rr])
                .then_with(|| cmp_state(*ls, *rs))
                .then_with(|| unique_null_tag(*ln).cmp(&unique_null_tag(*rn)))
                .then_with(|| lc.len().cmp(&rc.len()))
                .then_with(|| cmp_columns(lc, rc, &relations[*lr], &relations[*rr])),
            (NormalizedConstraintV1::ForeignKey { child: lc, parent: lp, state: ls,
                match_kind: lm, pairs: lps }, NormalizedConstraintV1::ForeignKey {
                child: rc, parent: rp, state: rs, match_kind: rm, pairs: rps }) =>
                cmp_relation_coord(&relations[*lc], &relations[*rc])
                .then_with(|| cmp_relation_coord(&relations[*lp], &relations[*rp]))
                .then_with(|| cmp_state(*ls, *rs))
                .then_with(|| foreign_match_tag(*lm).cmp(&foreign_match_tag(*rm)))
                .then_with(|| lps.len().cmp(&rps.len()))
                .then_with(|| cmp_pairs(lps, rps, &relations[*lc], &relations[*lp],
                    &relations[*rc], &relations[*rp])),
            _ => Ordering::Equal,
        })
}

#[rustfmt::skip]
fn cmp_columns(left: &[usize], right: &[usize], left_relation: &NormalizedRelationV1,
    right_relation: &NormalizedRelationV1) -> Ordering {
    left.iter().zip(right).find_map(|(left, right)| {
        let order = cmp_column_key(&left_relation.columns[*left], &right_relation.columns[*right]);
        (order != Ordering::Equal).then_some(order)
    }).unwrap_or(Ordering::Equal)
}

#[rustfmt::skip]
fn cmp_pairs(left: &[(usize, usize)], right: &[(usize, usize)],
    left_child: &NormalizedRelationV1, left_parent: &NormalizedRelationV1,
    right_child: &NormalizedRelationV1, right_parent: &NormalizedRelationV1) -> Ordering {
    left.iter().zip(right).find_map(|((lc, lp), (rc, rp))| {
        let order = cmp_column_key(&left_child.columns[*lc], &right_child.columns[*rc])
            .then_with(|| cmp_column_key(&left_parent.columns[*lp], &right_parent.columns[*rp]));
        (order != Ordering::Equal).then_some(order)
    }).unwrap_or(Ordering::Equal)
}

fn cmp_state(left: ConstraintStateV1, right: ConstraintStateV1) -> Ordering {
    left.validated
        .cmp(&right.validated)
        .then_with(|| left.enforced.cmp(&right.enforced))
}

const fn constraint_tag(value: &NormalizedConstraintV1) -> u8 {
    match value {
        NormalizedConstraintV1::NotNull { .. } => 0x31,
        NormalizedConstraintV1::PrimaryKey { .. } => 0x32,
        NormalizedConstraintV1::UniqueKey { .. } => 0x33,
        NormalizedConstraintV1::ForeignKey { .. } => 0x34,
    }
}

fn cmp_optional_txt(left: Option<&IdentifierV1>, right: Option<&IdentifierV1>) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(left), Some(right)) => cmp_txt(left.as_bytes(), right.as_bytes()),
    }
}

fn cmp_txt(left: &[u8], right: &[u8]) -> Ordering {
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}
