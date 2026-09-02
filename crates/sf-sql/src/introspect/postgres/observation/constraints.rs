//! Pure, bounded PostgreSQL 16 constraint projection (ADR-0051 §7).
//!
//! This module is deliberately dead-staged: SQL decoding and adapter wiring are
//! supplied by a later slice. OIDs are join handles only and never reach the
//! schema identity.
use std::collections::HashSet;

use sf_core::schema_identity::{
    ColumnKeyV1, ConstraintInputV1, ConstraintStateV1, ForeignKeyMatchV1, UniqueNullSemanticsV1,
};

use super::relation::Postgres16NormalizedRelationsV1;
use super::{PostgresSchemaIdentityLimitCodeV1, PostgresSchemaIdentityUnavailableV1};

const MAX_KEY_MEMBERS: usize = 32;
const MAX_CONSTRAINTS: usize = 65_536;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Postgres16RawConstraintV1 {
    NotNull {
        relation_oid: u32,
        attnum: i16,
        validated: bool,
    },
    PrimaryKey(Postgres16RawKeyV1),
    Unique(Postgres16RawUniqueV1),
    ForeignKey(Postgres16RawForeignKeyV1),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Postgres16RawKeyV1 {
    pub(super) relation_oid: u32,
    pub(super) attnums: Vec<i16>,
    pub(super) validated: bool,
    pub(super) enforced: bool,
    pub(super) index: Postgres16RawIndexV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Postgres16RawUniqueV1 {
    pub(super) key: Postgres16RawKeyV1,
    pub(super) nulls_not_distinct: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Postgres16RawForeignKeyV1 {
    pub(super) child_oid: u32,
    pub(super) parent_oid: u32,
    pub(super) child_attnums: Vec<i16>,
    pub(super) parent_attnums: Vec<i16>,
    pub(super) validated: bool,
    pub(super) match_code: char,
    pub(super) parent_index: Postgres16RawIndexV1,
    pub(super) equality_operators: Vec<Postgres16RawEqualityOperatorV1>,
    pub(super) triggers: Postgres16RawForeignKeyTriggersV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Postgres16RawIndexV1 {
    pub(super) relation_oid: u32,
    pub(super) key_attnums: Vec<i16>,
    pub(super) unique: bool,
    pub(super) primary: bool,
    pub(super) valid: bool,
    pub(super) ready: bool,
    pub(super) live: bool,
    pub(super) immediate: bool,
    pub(super) btree_default: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Postgres16RawEqualityOperatorV1 {
    pub(super) child_oid: u32,
    pub(super) parent_oid: u32,
    pub(super) selected_oid: u32,
    pub(super) search_oid: u32,
    pub(super) is_catalog_equals_bool: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Postgres16RawForeignKeyTriggersV1 {
    pub(super) child_insert_ok: bool,
    pub(super) child_update_ok: bool,
    pub(super) parent_delete_ok: bool,
    pub(super) parent_update_ok: bool,
    pub(super) all_enabled: bool,
}

pub(super) fn normalize_postgres16_constraints_v1(
    relations: &Postgres16NormalizedRelationsV1,
    raw: Vec<Postgres16RawConstraintV1>,
) -> Result<Vec<ConstraintInputV1>, PostgresSchemaIdentityUnavailableV1> {
    if raw.len() > MAX_CONSTRAINTS {
        return Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            PostgresSchemaIdentityLimitCodeV1::RawConstraints,
        ));
    }
    let mut out = Vec::with_capacity(raw.len());
    let mut seen = HashSet::with_capacity(raw.len());
    for item in raw {
        let value = match item {
            Postgres16RawConstraintV1::NotNull {
                relation_oid,
                attnum,
                validated,
            } => {
                let (column, coordinate) = relations
                    .live_column_by_number(relation_oid, attnum)
                    .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)?;
                if !coordinate.is_not_null || !validated {
                    return Err(unsupported());
                }
                ConstraintInputV1::NotNull {
                    column: sf_core::schema_identity::ColumnRefV1 {
                        relation: relation_of(relations, relation_oid)?,
                        column: column_key(column),
                    },
                    state: ConstraintStateV1 {
                        validated: true,
                        enforced: true,
                    },
                }
            }
            Postgres16RawConstraintV1::PrimaryKey(key) => key_constraint(relations, key, true)?,
            Postgres16RawConstraintV1::Unique(unique) => {
                let relation = relation_of(relations, unique.key.relation_oid)?;
                let columns = key_columns(relations, unique.key.relation_oid, &unique.key.attnums)?;
                validate_index(&unique.key, true, false)?;
                ConstraintInputV1::UniqueKey {
                    relation,
                    state: state(unique.key.validated, unique.key.enforced),
                    nulls: if unique.nulls_not_distinct {
                        UniqueNullSemanticsV1::NullsNotDistinct
                    } else {
                        UniqueNullSemanticsV1::NullsDistinct
                    },
                    columns,
                }
            }
            Postgres16RawConstraintV1::ForeignKey(fk) => foreign_key(relations, fk)?,
        };
        if !seen.insert(value.clone()) {
            return Err(unsupported());
        }
        out.push(value);
    }
    Ok(out)
}

fn key_constraint(
    r: &Postgres16NormalizedRelationsV1,
    k: Postgres16RawKeyV1,
    primary: bool,
) -> Result<ConstraintInputV1, PostgresSchemaIdentityUnavailableV1> {
    let relation = relation_of(r, k.relation_oid)?;
    let columns = key_columns(r, k.relation_oid, &k.attnums)?;
    validate_index(&k, true, primary)?;
    if primary
        && k.attnums.iter().any(|attnum| {
            r.live_column_by_number(k.relation_oid, *attnum)
                .is_none_or(|(_, coordinate)| !coordinate.is_not_null)
        })
    {
        return Err(unsupported());
    }
    Ok(ConstraintInputV1::PrimaryKey {
        relation,
        state: state(k.validated, k.enforced),
        columns,
    })
}

fn foreign_key(
    r: &Postgres16NormalizedRelationsV1,
    f: Postgres16RawForeignKeyV1,
) -> Result<ConstraintInputV1, PostgresSchemaIdentityUnavailableV1> {
    if f.child_attnums.len() != f.parent_attnums.len()
        || f.child_attnums.is_empty()
        || f.match_code == 'p'
    {
        return Err(unsupported());
    }
    if !matches!(f.match_code, 's' | 'f') || f.equality_operators.len() != f.child_attnums.len() {
        return Err(unsupported());
    }
    let child = relation_of(r, f.child_oid)?;
    let parent = relation_of(r, f.parent_oid)?;
    let child_columns = key_columns(r, f.child_oid, &f.child_attnums)?;
    let parent_columns = key_columns(r, f.parent_oid, &f.parent_attnums)?;
    validate_index_shape(&f.parent_index, f.parent_oid, &f.parent_attnums)?;
    if !f.parent_index.unique || !f.parent_index.immediate {
        return Err(unsupported());
    }
    if !f.triggers.child_insert_ok
        || !f.triggers.child_update_ok
        || !f.triggers.parent_delete_ok
        || !f.triggers.parent_update_ok
    {
        return Err(unsupported());
    }
    if !f.triggers.all_enabled
        || !f.equality_operators.iter().all(|x| {
            x.child_oid == f.child_oid
                && x.parent_oid == f.parent_oid
                && x.selected_oid == x.search_oid
                && x.is_catalog_equals_bool
        })
    {
        return Err(unsupported());
    }
    if duplicate_columns(&child_columns) || duplicate_columns(&parent_columns) {
        return Err(unsupported());
    }
    let pairs = child_columns.into_iter().zip(parent_columns).collect();
    Ok(ConstraintInputV1::ForeignKey {
        child,
        parent,
        state: state(f.validated, f.triggers.all_enabled),
        match_kind: if f.match_code == 's' {
            ForeignKeyMatchV1::Simple
        } else {
            ForeignKeyMatchV1::Full
        },
        pairs,
    })
}

fn key_columns(
    r: &Postgres16NormalizedRelationsV1,
    oid: u32,
    nums: &[i16],
) -> Result<Vec<ColumnKeyV1>, PostgresSchemaIdentityUnavailableV1> {
    if nums.is_empty()
        || nums.len() > MAX_KEY_MEMBERS
        || nums.iter().collect::<HashSet<_>>().len() != nums.len()
    {
        return Err(unsupported());
    }
    nums.iter()
        .map(|n| {
            r.live_column_by_number(oid, *n)
                .map(|(c, _)| column_key(c))
                .ok_or(unsupported())
        })
        .collect()
}
fn validate_index(
    k: &Postgres16RawKeyV1,
    unique: bool,
    primary: bool,
) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
    validate_index_shape(&k.index, k.relation_oid, &k.attnums)?;
    if k.index.unique != unique || k.index.primary != primary || !k.index.immediate {
        return Err(unsupported());
    }
    Ok(())
}
fn validate_index_shape(
    i: &Postgres16RawIndexV1,
    oid: u32,
    nums: &[i16],
) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
    if i.relation_oid != oid
        || i.key_attnums != nums
        || i.key_attnums.is_empty()
        || i.key_attnums.len() > MAX_KEY_MEMBERS
        || !i.valid
        || !i.ready
        || !i.live
        || !i.btree_default
    {
        return Err(unsupported());
    }
    Ok(())
}
fn relation_of(
    r: &Postgres16NormalizedRelationsV1,
    oid: u32,
) -> Result<sf_core::schema_identity::RelationRefV1, PostgresSchemaIdentityUnavailableV1> {
    Ok(sf_core::schema_identity::RelationRefV1 {
        name: r.relation_by_oid(oid).ok_or(unsupported())?.name.clone(),
    })
}
fn column_key(c: &sf_core::schema_identity::ColumnInputV1) -> ColumnKeyV1 {
    ColumnKeyV1 {
        ordinal: c.ordinal,
        name: c.name.clone(),
    }
}
fn state(v: bool, e: bool) -> ConstraintStateV1 {
    ConstraintStateV1 {
        validated: v,
        enforced: e,
    }
}
fn duplicate_columns(c: &[ColumnKeyV1]) -> bool {
    c.iter().map(|x| x.ordinal).collect::<HashSet<_>>().len() != c.len()
}
fn unsupported() -> PostgresSchemaIdentityUnavailableV1 {
    PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint
}
