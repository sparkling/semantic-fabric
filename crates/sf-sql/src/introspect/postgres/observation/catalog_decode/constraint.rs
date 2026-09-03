//! PostgreSQL constraint catalogue row decoding and raw evidence adaptation.

use super::super::constraints::{
    Postgres16RawConstraintV1, Postgres16RawEqualityOperatorV1, Postgres16RawForeignKeyV1,
    Postgres16RawIndexPositionV1, Postgres16RawIndexV1, Postgres16RawKeyV1, Postgres16RawUniqueV1,
};
use super::super::trigger_evidence::decode_postgres16_fk_trigger_evidence_v1;
use super::super::{PostgresSchemaIdentityLimitCodeV1, PostgresSchemaIdentityUnavailableV1};
use super::MAX_CATALOG_ARRAY_MEMBERS_V1;
use tokio_postgres::Row;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::introspect::postgres::observation) struct CatalogConstraintRowV1 {
    pub(in crate::introspect::postgres::observation) constraint_oid: u32,
    pub(in crate::introspect::postgres::observation) constraint_kind: String,
    pub(in crate::introspect::postgres::observation) child_oid: u32,
    pub(in crate::introspect::postgres::observation) parent_oid: u32,
    pub(in crate::introspect::postgres::observation) validated: bool,
    pub(in crate::introspect::postgres::observation) deferrable: bool,
    pub(in crate::introspect::postgres::observation) deferred: bool,
    pub(in crate::introspect::postgres::observation) child_key: Option<Vec<i16>>,
    pub(in crate::introspect::postgres::observation) parent_key: Option<Vec<i16>>,
    pub(in crate::introspect::postgres::observation) array_overflow: bool,
    pub(in crate::introspect::postgres::observation) constraint_type_oid: u32,
    pub(in crate::introspect::postgres::observation) parent_constraint_oid: u32,
    pub(in crate::introspect::postgres::observation) constraint_is_local: bool,
    pub(in crate::introspect::postgres::observation) constraint_inheritance_count: i16,
    pub(in crate::introspect::postgres::observation) selected_index_oid: u32,
    pub(in crate::introspect::postgres::observation) observed_index_oid: Option<u32>,
    pub(in crate::introspect::postgres::observation) index_relation_oid: Option<u32>,
    pub(in crate::introspect::postgres::observation) trigger_role_codes: Option<Vec<i16>>,
    pub(in crate::introspect::postgres::observation) trigger_enabled_codes: Option<Vec<i16>>,
    pub(in crate::introspect::postgres::observation) operator_oids: Option<Vec<u32>>,
    pub(in crate::introspect::postgres::observation) search_operator_oids: Option<Vec<u32>>,
    pub(in crate::introspect::postgres::observation) operator_opclass_oids: Option<Vec<u32>>,
    pub(in crate::introspect::postgres::observation) operator_complete: Option<bool>,
    pub(in crate::introspect::postgres::observation) types_and_facets_equal: Option<bool>,
    pub(in crate::introspect::postgres::observation) operator_left_type_oids: Option<Vec<u32>>,
    pub(in crate::introspect::postgres::observation) operator_right_type_oids: Option<Vec<u32>>,
    pub(in crate::introspect::postgres::observation) trigger_overflow: bool,
    pub(in crate::introspect::postgres::observation) index_primary: Option<bool>,
    pub(in crate::introspect::postgres::observation) index_unique: Option<bool>,
    pub(in crate::introspect::postgres::observation) index_valid: Option<bool>,
    pub(in crate::introspect::postgres::observation) index_ready: Option<bool>,
    pub(in crate::introspect::postgres::observation) index_live: Option<bool>,
    pub(in crate::introspect::postgres::observation) index_immediate: Option<bool>,
    pub(in crate::introspect::postgres::observation) index_key_count: Option<i16>,
    pub(in crate::introspect::postgres::observation) index_total_attribute_count: Option<i16>,
    pub(in crate::introspect::postgres::observation) index_all_attnums: Option<Vec<i16>>,
    pub(in crate::introspect::postgres::observation) index_opclass_oids: Option<Vec<u32>>,
    pub(in crate::introspect::postgres::observation) index_opclass_input_type_oids:
        Option<Vec<u32>>,
    pub(in crate::introspect::postgres::observation) index_collation_oids: Option<Vec<u32>>,
    pub(in crate::introspect::postgres::observation) index_position_exact_flags: Option<Vec<bool>>,
    pub(in crate::introspect::postgres::observation) index_nulls_not_distinct: Option<bool>,
    pub(in crate::introspect::postgres::observation) index_access_method: Option<String>,
    pub(in crate::introspect::postgres::observation) index_access_method_type: Option<String>,
    pub(in crate::introspect::postgres::observation) index_expressions_absent: Option<bool>,
    pub(in crate::introspect::postgres::observation) index_predicate_absent: Option<bool>,
    pub(in crate::introspect::postgres::observation) match_code: Option<String>,
    pub(in crate::introspect::postgres::observation) update_action: Option<String>,
    pub(in crate::introspect::postgres::observation) delete_action: Option<String>,
}

impl CatalogConstraintRowV1 {
    /// Convert only rows whose required index proof is present. Foreign keys
    /// additionally require complete, bounded trigger and operator evidence.
    pub(in crate::introspect::postgres::observation) fn into_raw_constraint(
        self,
    ) -> Result<Postgres16RawConstraintV1, PostgresSchemaIdentityUnavailableV1> {
        let child_attnums = self
            .child_key
            .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)?;
        let parent_attnums = self.parent_key.clone().unwrap_or_default();
        let all_attnums = self
            .index_all_attnums
            .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)?;
        let key_count = self
            .index_key_count
            .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)?;
        let key_count_usize = usize::try_from(key_count)
            .ok()
            .filter(|count| *count > 0 && *count <= MAX_CATALOG_ARRAY_MEMBERS_V1)
            .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)?;
        let total_attribute_count = self
            .index_total_attribute_count
            .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)?;
        let opclass_oids = self
            .index_opclass_oids
            .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)?;
        let opclass_input_type_oids = self
            .index_opclass_input_type_oids
            .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)?;
        let collation_oids = self
            .index_collation_oids
            .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)?;
        let position_exact_flags = self
            .index_position_exact_flags
            .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)?;
        if child_attnums.is_empty()
            || child_attnums.len() > MAX_CATALOG_ARRAY_MEMBERS_V1
            || opclass_oids.len() != key_count_usize
            || opclass_input_type_oids.len() != key_count_usize
            || collation_oids.len() != key_count_usize
            || position_exact_flags.len() != key_count_usize
            || all_attnums.len() < key_count_usize
        {
            return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint);
        }
        let key_positions = all_attnums
            .iter()
            .copied()
            .take(key_count_usize)
            .zip(opclass_oids)
            .zip(opclass_input_type_oids)
            .zip(collation_oids)
            .zip(position_exact_flags)
            .map(
                |(
                    (((attnum, opclass_oid), opclass_input_type_oid), collation_oid),
                    default_btree,
                )| Postgres16RawIndexPositionV1 {
                    attnum,
                    opclass_oid,
                    opclass_input_type_oid,
                    collation_oid,
                    default_btree,
                },
            )
            .collect();
        let index = Postgres16RawIndexV1 {
            selected_oid: self.selected_index_oid,
            observed_oid: self
                .observed_index_oid
                .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)?,
            relation_oid: self
                .index_relation_oid
                .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)?,
            key_count,
            total_attribute_count,
            all_attnums,
            key_positions,
            unique: self.index_unique == Some(true),
            primary: self.index_primary == Some(true),
            valid: self.index_valid == Some(true),
            ready: self.index_ready == Some(true),
            live: self.index_live == Some(true),
            immediate: self.index_immediate == Some(true),
            access_method_exact: self.index_access_method.as_deref() == Some("btree")
                && self.index_access_method_type.as_deref() == Some("i"),
            expressions_absent: self.index_expressions_absent == Some(true),
            predicate_absent: self.index_predicate_absent == Some(true),
        };
        if self.constraint_kind == "f" {
            if parent_attnums.is_empty() || parent_attnums.len() != child_attnums.len() {
                return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint);
            }
            let selected = self
                .operator_oids
                .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)?;
            let search = self
                .search_operator_oids
                .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)?;
            let operator_opclasses = self
                .operator_opclass_oids
                .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)?;
            let left_types = self
                .operator_left_type_oids
                .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)?;
            let right_types = self
                .operator_right_type_oids
                .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)?;
            if selected.len() != child_attnums.len()
                || search.len() != selected.len()
                || operator_opclasses.len() != selected.len()
                || left_types.len() != selected.len()
                || right_types.len() != selected.len()
                || self.operator_complete != Some(true)
                || self.types_and_facets_equal != Some(true)
            {
                return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint);
            }
            let operators_exact = self.operator_complete == Some(true);
            let types_and_facets_equal = self.types_and_facets_equal == Some(true);
            let triggers = decode_postgres16_fk_trigger_evidence_v1(
                self.trigger_overflow,
                self.trigger_role_codes,
                self.trigger_enabled_codes,
            )?;
            let triggers_exact = triggers.child_insert_ok
                && triggers.child_update_ok
                && triggers.parent_delete_ok
                && triggers.parent_update_ok;
            let actions_supported = matches!(
                self.update_action.as_deref(),
                Some("a" | "r" | "c" | "n" | "d")
            ) && matches!(
                self.delete_action.as_deref(),
                Some("a" | "r" | "c" | "n" | "d")
            );
            let match_code = self
                .match_code
                .as_deref()
                .and_then(|v| v.chars().next())
                .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)?;
            let equality_operators = selected
                .into_iter()
                .zip(search)
                .zip(operator_opclasses)
                .zip(left_types.into_iter().zip(right_types))
                .map(
                    |(
                        ((selected_oid, search_oid), opclass_oid),
                        (left_type_oid, right_type_oid),
                    )| {
                        Postgres16RawEqualityOperatorV1 {
                            opclass_oid,
                            parent_operand_type_oid: left_type_oid,
                            child_operand_type_oid: right_type_oid,
                            selected_oid,
                            search_oid,
                            is_catalog_equals_bool: operators_exact,
                        }
                    },
                )
                .collect();
            return Ok(Postgres16RawConstraintV1::ForeignKey(
                Postgres16RawForeignKeyV1 {
                    child_oid: self.child_oid,
                    parent_oid: self.parent_oid,
                    child_attnums,
                    parent_attnums,
                    validated: self.validated,
                    match_code,
                    parent_index: index,
                    equality_operators,
                    triggers,
                    types_and_facets_equal,
                    operators_exact,
                    triggers_exact,
                    actions_supported,
                },
            ));
        }
        let key = Postgres16RawKeyV1 {
            relation_oid: self.child_oid,
            attnums: child_attnums,
            validated: self.validated,
            enforced: true,
            index,
        };
        match self.constraint_kind.as_str() {
            "p" if self.index_primary == Some(true) => {
                Ok(Postgres16RawConstraintV1::PrimaryKey(key))
            }
            "u" if self.index_unique == Some(true) => {
                Ok(Postgres16RawConstraintV1::Unique(Postgres16RawUniqueV1 {
                    key,
                    nulls_not_distinct: self.index_nulls_not_distinct == Some(true),
                }))
            }
            _ => Err(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint),
        }
    }
}
pub(in crate::introspect::postgres::observation) fn decode_constraint_row_v1(
    row: &Row,
) -> Result<CatalogConstraintRowV1, PostgresSchemaIdentityUnavailableV1> {
    macro_rules! get {
        ($name:literal, $ty:ty) => {
            row.try_get::<_, $ty>($name)
                .map_err(|_| PostgresSchemaIdentityUnavailableV1::CatalogDecode)?
        };
    }
    let value = CatalogConstraintRowV1 {
        constraint_oid: get!("constraint_oid", u32),
        constraint_kind: get!("contype", String),
        child_oid: get!("child_oid", u32),
        parent_oid: get!("parent_oid", u32),
        validated: get!("convalidated", bool),
        deferrable: get!("condeferrable", bool),
        deferred: get!("condeferred", bool),
        constraint_type_oid: get!("contypid", u32),
        parent_constraint_oid: get!("conparentid", u32),
        constraint_is_local: get!("conislocal", bool),
        constraint_inheritance_count: get!("coninhcount", i16),
        child_key: get!("conkey", Option<Vec<i16>>),
        parent_key: get!("confkey", Option<Vec<i16>>),
        array_overflow: get!("sf_array_overflow", bool),
        selected_index_oid: get!("selected_index_oid", u32),
        observed_index_oid: get!("joined_index_oid", Option<u32>),
        index_relation_oid: get!("index_relation_oid", Option<u32>),
        trigger_role_codes: get!("trigger_role_codes", Option<Vec<i16>>),
        trigger_enabled_codes: get!("trigger_enabled_codes", Option<Vec<i16>>),
        operator_oids: get!("operator_oids", Option<Vec<u32>>),
        search_operator_oids: get!("search_operator_oids", Option<Vec<u32>>),
        operator_opclass_oids: get!("operator_opclass_oids", Option<Vec<u32>>),
        operator_complete: get!("operator_complete", Option<bool>),
        types_and_facets_equal: get!("types_and_facets_equal", Option<bool>),
        operator_left_type_oids: get!("operator_left_type_oids", Option<Vec<u32>>),
        operator_right_type_oids: get!("operator_right_type_oids", Option<Vec<u32>>),
        trigger_overflow: get!("sf_trigger_overflow", bool),
        index_primary: get!("indisprimary", Option<bool>),
        index_unique: get!("indisunique", Option<bool>),
        index_valid: get!("indisvalid", Option<bool>),
        index_ready: get!("indisready", Option<bool>),
        index_live: get!("indislive", Option<bool>),
        index_immediate: get!("indimmediate", Option<bool>),
        index_key_count: get!("indnkeyatts", Option<i16>),
        index_total_attribute_count: get!("indnatts", Option<i16>),
        index_all_attnums: get!("indkey", Option<Vec<i16>>),
        index_opclass_oids: get!("index_opclass_oids", Option<Vec<u32>>),
        index_opclass_input_type_oids: get!("index_opclass_input_type_oids", Option<Vec<u32>>),
        index_collation_oids: get!("index_collation_oids", Option<Vec<u32>>),
        index_position_exact_flags: get!("index_position_exact_flags", Option<Vec<bool>>),
        index_nulls_not_distinct: get!("indnullsnotdistinct", Option<bool>),
        index_access_method: get!("index_access_method", Option<String>),
        index_access_method_type: get!("index_access_method_type", Option<String>),
        index_expressions_absent: get!("index_expressions_absent", Option<bool>),
        index_predicate_absent: get!("index_predicate_absent", Option<bool>),
        match_code: get!("confmatchtype", Option<String>),
        update_action: get!("confupdtype", Option<String>),
        delete_action: get!("confdeltype", Option<String>),
    };
    if value.array_overflow || value.trigger_overflow {
        return Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            PostgresSchemaIdentityLimitCodeV1::KeyMembers,
        ));
    }
    let operators_mismatch = match (
        &value.operator_oids,
        &value.search_operator_oids,
        &value.operator_opclass_oids,
        &value.operator_left_type_oids,
        &value.operator_right_type_oids,
    ) {
        (Some(a), Some(b), Some(c), Some(d), Some(e)) => {
            a.len() != b.len() || a.len() != c.len() || a.len() != d.len() || a.len() != e.len()
        }
        (None, None, None, None, None) => false,
        _ => true,
    };
    let index_positions_mismatch = match (
        &value.index_opclass_oids,
        &value.index_opclass_input_type_oids,
        &value.index_collation_oids,
        &value.index_position_exact_flags,
    ) {
        (Some(a), Some(b), Some(c), Some(d)) => {
            a.len() != b.len() || a.len() != c.len() || a.len() != d.len()
        }
        (None, None, None, None) => false,
        _ => true,
    };
    if value
        .trigger_role_codes
        .as_ref()
        .is_some_and(|v| v.len() > 4)
        || value
            .trigger_enabled_codes
            .as_ref()
            .is_some_and(|v| v.len() > 4)
        || value
            .operator_oids
            .as_ref()
            .is_some_and(|v| v.len() > MAX_CATALOG_ARRAY_MEMBERS_V1)
        || value
            .search_operator_oids
            .as_ref()
            .is_some_and(|v| v.len() > MAX_CATALOG_ARRAY_MEMBERS_V1)
        || value
            .operator_opclass_oids
            .as_ref()
            .is_some_and(|v| v.len() > MAX_CATALOG_ARRAY_MEMBERS_V1)
        || value
            .operator_left_type_oids
            .as_ref()
            .is_some_and(|v| v.len() > MAX_CATALOG_ARRAY_MEMBERS_V1)
        || value
            .operator_right_type_oids
            .as_ref()
            .is_some_and(|v| v.len() > MAX_CATALOG_ARRAY_MEMBERS_V1)
        || value
            .index_all_attnums
            .as_ref()
            .is_some_and(|v| v.len() > MAX_CATALOG_ARRAY_MEMBERS_V1)
        || value
            .index_opclass_oids
            .as_ref()
            .is_some_and(|v| v.len() > MAX_CATALOG_ARRAY_MEMBERS_V1)
        || value
            .index_opclass_input_type_oids
            .as_ref()
            .is_some_and(|v| v.len() > MAX_CATALOG_ARRAY_MEMBERS_V1)
        || value
            .index_collation_oids
            .as_ref()
            .is_some_and(|v| v.len() > MAX_CATALOG_ARRAY_MEMBERS_V1)
        || value
            .index_position_exact_flags
            .as_ref()
            .is_some_and(|v| v.len() > MAX_CATALOG_ARRAY_MEMBERS_V1)
        || operators_mismatch
        || index_positions_mismatch
    {
        return Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            PostgresSchemaIdentityLimitCodeV1::KeyMembers,
        ));
    }
    if !matches!(value.constraint_kind.as_str(), "p" | "u" | "f")
        || value.constraint_oid == 0
        || value.child_oid == 0
        || value.deferrable
        || value.deferred
        || value.constraint_type_oid != 0
        || value.parent_constraint_oid != 0
        || !value.constraint_is_local
        || value.constraint_inheritance_count != 0
        || value.selected_index_oid == 0
        || value.observed_index_oid.is_none()
        || value.index_relation_oid.is_none()
        || value.child_key.as_ref().is_none_or(|v| v.is_empty())
        || value.index_key_count.is_none()
        || value.index_total_attribute_count.is_none()
        || value.index_all_attnums.is_none()
        || value.index_opclass_oids.is_none()
        || value.index_opclass_input_type_oids.is_none()
        || value.index_collation_oids.is_none()
        || value.index_position_exact_flags.is_none()
        || value.index_access_method.as_deref() != Some("btree")
        || value.index_access_method_type.as_deref() != Some("i")
        || value.index_expressions_absent != Some(true)
        || value.index_predicate_absent != Some(true)
        || (value.constraint_kind == "f"
            && (value.parent_oid == 0
                || value.parent_key.as_ref().is_none_or(|key| key.is_empty())
                || value.trigger_role_codes.is_none()
                || value.trigger_enabled_codes.is_none()
                || value.operator_oids.is_none()
                || value.search_operator_oids.is_none()
                || value.operator_opclass_oids.is_none()
                || value.operator_left_type_oids.is_none()
                || value.operator_right_type_oids.is_none()
                || value.operator_complete != Some(true)
                || value.types_and_facets_equal != Some(true)))
    {
        return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint);
    }
    if matches!(value.constraint_kind.as_str(), "p" | "u")
        && (value.parent_oid != 0 || value.parent_key.is_some())
    {
        return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint);
    }
    if value.constraint_kind == "f"
        && (!matches!(value.match_code.as_deref(), Some("s" | "f"))
            || !matches!(
                value.update_action.as_deref(),
                Some("a" | "r" | "c" | "n" | "d")
            )
            || !matches!(
                value.delete_action.as_deref(),
                Some("a" | "r" | "c" | "n" | "d")
            ))
    {
        return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint);
    }
    Ok(value)
}
