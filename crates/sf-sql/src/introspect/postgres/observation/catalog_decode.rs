//! Decode-neutral bounded catalogue envelopes for PostgreSQL 16 (ADR-0051 §8).
//!
//! SQL adapters must validate these envelopes before retaining rich facts. This
//! module deliberately contains no driver calls and no public/runtime wiring.

use super::relation::{Postgres16AttributeCatalogFactV1, Postgres16RelationCatalogFactV1};
use super::source_type::{
    Postgres16ColumnTypeCatalogFactV1, Postgres16DefaultCollationCatalogFactV1,
};
use super::{
    PostgresSchemaIdentityGuardCodeV1, PostgresSchemaIdentityLimitCodeV1,
    PostgresSchemaIdentityUnavailableV1,
};
use tokio_postgres::Row;

pub(super) const MAX_CATALOG_TEXT_BYTES_V1: usize = 256;
pub(super) const MAX_CATALOG_ARRAY_MEMBERS_V1: usize = 32;
const MAX_RELATION_ROWS_V1: usize = 65_536;
const MAX_ATTRIBUTE_ROWS_V1: usize = 1_048_576;
const MAX_CONSTRAINT_ROWS_V1: usize = 65_536;

fn bounded_rows<T>(
    rows: &[Row],
    cap: usize,
    limit: PostgresSchemaIdentityLimitCodeV1,
    decode: impl Fn(&Row) -> Result<T, PostgresSchemaIdentityUnavailableV1>,
) -> Result<Vec<T>, PostgresSchemaIdentityUnavailableV1> {
    if rows.len() > cap {
        return Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(limit));
    }
    rows.iter().map(decode).collect()
}

pub(super) fn decode_relation_rows_v1(
    rows: &[Row],
) -> Result<Vec<CatalogRelationRowV1>, PostgresSchemaIdentityUnavailableV1> {
    bounded_rows(
        rows,
        MAX_RELATION_ROWS_V1,
        PostgresSchemaIdentityLimitCodeV1::RichRelations,
        decode_relation_row_v1,
    )
}

pub(super) fn decode_attribute_rows_v1(
    rows: &[Row],
) -> Result<Vec<CatalogAttributeRowV1>, PostgresSchemaIdentityUnavailableV1> {
    bounded_rows(
        rows,
        MAX_ATTRIBUTE_ROWS_V1,
        PostgresSchemaIdentityLimitCodeV1::PhysicalAttributes,
        decode_attribute_row_v1,
    )
}

pub(super) fn decode_constraint_rows_v1(
    rows: &[Row],
) -> Result<Vec<CatalogConstraintRowV1>, PostgresSchemaIdentityUnavailableV1> {
    bounded_rows(
        rows,
        MAX_CONSTRAINT_ROWS_V1,
        PostgresSchemaIdentityLimitCodeV1::RawConstraints,
        decode_constraint_row_v1,
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CatalogConstraintRowV1 {
    pub(super) constraint_oid: u32,
    pub(super) constraint_kind: String,
    pub(super) child_oid: u32,
    pub(super) parent_oid: u32,
    pub(super) validated: bool,
    pub(super) deferrable: bool,
    pub(super) deferred: bool,
    pub(super) child_key: Option<Vec<i16>>,
    pub(super) parent_key: Option<Vec<i16>>,
    pub(super) array_overflow: bool,
    pub(super) index_oid: Option<u32>,
    pub(super) trigger_oids: Option<Vec<u32>>,
    pub(super) operator_oids: Option<Vec<u32>>,
    pub(super) search_operator_oids: Option<Vec<u32>>,
    pub(super) operator_complete: Option<bool>,
    pub(super) trigger_overflow: bool,
    pub(super) trigger_shape_valid: Option<bool>,
    pub(super) trigger_all_enabled: Option<bool>,
    pub(super) index_primary: Option<bool>,
    pub(super) index_unique: Option<bool>,
    pub(super) index_valid: Option<bool>,
    pub(super) index_ready: Option<bool>,
    pub(super) index_live: Option<bool>,
    pub(super) index_immediate: Option<bool>,
    pub(super) index_key_count: Option<i16>,
    pub(super) index_key_attnums: Option<Vec<i16>>,
    pub(super) index_nulls_not_distinct: Option<bool>,
    pub(super) index_access_method: Option<String>,
    pub(super) index_opclass_default: Option<bool>,
    pub(super) match_code: Option<String>,
    pub(super) update_action: Option<String>,
    pub(super) delete_action: Option<String>,
}

pub(super) fn decode_constraint_row_v1(
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
        child_key: get!("conkey", Option<Vec<i16>>),
        parent_key: get!("confkey", Option<Vec<i16>>),
        array_overflow: get!("sf_array_overflow", bool),
        index_oid: get!("conindid", Option<u32>),
        trigger_oids: get!("trigger_oids", Option<Vec<u32>>),
        operator_oids: get!("operator_oids", Option<Vec<u32>>),
        search_operator_oids: get!("search_operator_oids", Option<Vec<u32>>),
        operator_complete: get!("operator_complete", Option<bool>),
        trigger_overflow: get!("sf_trigger_overflow", bool),
        trigger_shape_valid: get!("trigger_shape_valid", Option<bool>),
        trigger_all_enabled: get!("trigger_all_enabled", Option<bool>),
        index_primary: get!("indisprimary", Option<bool>),
        index_unique: get!("indisunique", Option<bool>),
        index_valid: get!("indisvalid", Option<bool>),
        index_ready: get!("indisready", Option<bool>),
        index_live: get!("indislive", Option<bool>),
        index_immediate: get!("indimmediate", Option<bool>),
        index_key_count: get!("indnkeyatts", Option<i16>),
        index_key_attnums: get!("indkey", Option<Vec<i16>>),
        index_nulls_not_distinct: get!("indnullsnotdistinct", Option<bool>),
        index_access_method: get!("index_access_method", Option<String>),
        index_opclass_default: get!("index_opclass_default", Option<bool>),
        match_code: get!("confmatchtype", Option<String>),
        update_action: get!("confupdtype", Option<String>),
        delete_action: get!("confdeltype", Option<String>),
    };
    if value.array_overflow || value.trigger_overflow {
        return Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            PostgresSchemaIdentityLimitCodeV1::KeyMembers,
        ));
    }
    let operators_mismatch = match (&value.operator_oids, &value.search_operator_oids) {
        (Some(a), Some(b)) => a.len() != b.len(),
        (None, None) => false,
        _ => true,
    };
    if value.trigger_oids.as_ref().is_some_and(|v| v.len() > 4)
        || value
            .operator_oids
            .as_ref()
            .is_some_and(|v| v.len() > MAX_CATALOG_ARRAY_MEMBERS_V1)
        || value
            .search_operator_oids
            .as_ref()
            .is_some_and(|v| v.len() > MAX_CATALOG_ARRAY_MEMBERS_V1)
        || operators_mismatch
    {
        return Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            PostgresSchemaIdentityLimitCodeV1::KeyMembers,
        ));
    }
    if !matches!(value.constraint_kind.as_str(), "p" | "u" | "f")
        || value.deferrable
        || value.deferred
        || value.child_key.as_ref().is_none_or(|v| v.is_empty())
        || (value.constraint_kind == "f"
            && (value.trigger_oids.is_none()
                || value.operator_oids.is_none()
                || value.search_operator_oids.is_none()
                || value.operator_complete != Some(true)
                || value.trigger_shape_valid != Some(true)
                || value.trigger_all_enabled != Some(true)))
    {
        return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint);
    }
    if matches!(value.constraint_kind.as_str(), "p" | "u")
        && (value.index_access_method.as_deref() != Some("btree")
            || value.index_opclass_default != Some(true))
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CatalogGuardRowV1 {
    pub(super) server_version_num: i32,
    pub(super) server_encoding: String,
    pub(super) client_encoding: String,
    pub(super) max_identifier_length: i32,
    pub(super) max_index_keys: i32,
    pub(super) integer_datetimes: String,
    pub(super) session_replication_role: String,
    pub(super) search_path: String,
    pub(super) database_oid: u32,
    pub(super) database_provider: String,
    pub(super) database_collate: String,
    pub(super) database_ctype: String,
    pub(super) database_icu_locale: Option<String>,
    pub(super) database_icu_rules: Option<String>,
    pub(super) database_recorded_version: Option<String>,
    pub(super) database_actual_version: Option<String>,
    pub(super) public_namespace_count: i64,
    pub(super) current_database_count: i64,
}

impl CatalogGuardRowV1 {
    pub(super) fn validate(&self) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
        match self.server_version_num {
            160_009 | 160_015 => {}
            160_000..=169_999 => {
                return Err(PostgresSchemaIdentityUnavailableV1::UnqualifiedEnginePatch)
            }
            _ => return Err(PostgresSchemaIdentityUnavailableV1::ProfileNotImplemented),
        }
        let exact = [
            (
                self.server_encoding.as_str(),
                "UTF8",
                PostgresSchemaIdentityGuardCodeV1::ServerEncoding,
            ),
            (
                self.client_encoding.as_str(),
                "UTF8",
                PostgresSchemaIdentityGuardCodeV1::ServerEncoding,
            ),
            (
                self.integer_datetimes.as_str(),
                "on",
                PostgresSchemaIdentityGuardCodeV1::IntegerDatetimes,
            ),
            (
                self.session_replication_role.as_str(),
                "origin",
                PostgresSchemaIdentityGuardCodeV1::ReplicationRole,
            ),
            (
                self.search_path.as_str(),
                "pg_catalog,public,pg_temp",
                PostgresSchemaIdentityGuardCodeV1::SearchPath,
            ),
        ];
        if let Some((_, _, code)) = exact
            .into_iter()
            .find(|(actual, expected, _)| actual != expected)
        {
            return Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(code));
        }
        if self.max_identifier_length != 63 {
            return Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
                PostgresSchemaIdentityGuardCodeV1::IdentifierLength,
            ));
        }
        if self.max_index_keys != 32 {
            return Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
                PostgresSchemaIdentityGuardCodeV1::IndexKeyLimit,
            ));
        }
        if self.public_namespace_count != 1 {
            return Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
                PostgresSchemaIdentityGuardCodeV1::PublicNamespace,
            ));
        }
        if self.current_database_count != 1 {
            return Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
                PostgresSchemaIdentityGuardCodeV1::CurrentDatabase,
            ));
        }
        if self.database_oid == 0
            || self
                .database_recorded_version
                .as_ref()
                .zip(self.database_actual_version.as_ref())
                .is_some_and(|(recorded, actual)| recorded != actual)
        {
            return Err(PostgresSchemaIdentityUnavailableV1::IdentityRejected);
        }
        BoundedCatalogTextV1::new(self.database_collate.clone())?;
        BoundedCatalogTextV1::new(self.database_ctype.clone())?;
        one_char(self.database_provider.clone())?;
        for value in [
            self.database_icu_locale.as_deref(),
            self.database_icu_rules.as_deref(),
            self.database_recorded_version.as_deref(),
            self.database_actual_version.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            BoundedCatalogTextV1::new(value.to_owned())?;
        }
        Ok(())
    }
}

pub(super) fn decode_guard_row_v1(
    row: &Row,
) -> Result<CatalogGuardRowV1, PostgresSchemaIdentityUnavailableV1> {
    macro_rules! get {
        ($name:literal, $ty:ty) => {
            row.try_get::<_, $ty>($name)
                .map_err(|_| PostgresSchemaIdentityUnavailableV1::CatalogDecode)?
        };
    }
    let value = CatalogGuardRowV1 {
        server_version_num: get!("server_version_num", i32),
        server_encoding: get!("server_encoding", String),
        client_encoding: get!("client_encoding", String),
        max_identifier_length: get!("max_identifier_length", i32),
        max_index_keys: get!("max_index_keys", i32),
        integer_datetimes: get!("integer_datetimes", String),
        session_replication_role: get!("session_replication_role", String),
        search_path: get!("search_path", String),
        database_oid: get!("database_oid", u32),
        database_provider: get!("database_provider", String),
        database_collate: get!("database_collate", String),
        database_ctype: get!("database_ctype", String),
        database_icu_locale: get!("database_icu_locale", Option<String>),
        database_icu_rules: get!("database_icu_rules", Option<String>),
        database_recorded_version: get!("database_recorded_version", Option<String>),
        database_actual_version: get!("database_actual_version", Option<String>),
        public_namespace_count: get!("public_namespace_count", i64),
        current_database_count: get!("current_database_count", i64),
    };
    value.validate()?;
    Ok(value)
}

pub(super) fn decode_relation_row_v1(
    row: &Row,
) -> Result<CatalogRelationRowV1, PostgresSchemaIdentityUnavailableV1> {
    macro_rules! get {
        ($name:literal, $ty:ty) => {
            row.try_get::<_, $ty>($name)
                .map_err(|_| PostgresSchemaIdentityUnavailableV1::CatalogDecode)?
        };
    }
    let value = CatalogRelationRowV1 {
        relation_oid: get!("relation_oid", u32),
        relation_namespace_oid: get!("relation_namespace_oid", u32),
        joined_namespace_oid: get!("joined_namespace_oid", u32),
        namespace_name: get!("namespace_name", String),
        relation_name: get!("relation_name", String),
        text_overflow: get!("sf_text_overflow", bool),
        relation_kind: one_char(get!("relkind", String))?,
        persistence: one_char(get!("relpersistence", String))?,
        is_partition: get!("relispartition", bool),
        is_shared: get!("relisshared", bool),
        row_security: get!("relrowsecurity", bool),
        force_row_security: get!("relforcerowsecurity", bool),
        of_type_oid: get!("reloftype", u32),
        rewrite_oid: get!("relrewrite", u32),
        access_method_oid: get!("relam", u32),
        joined_access_method_oid: get!("joined_access_method_oid", Option<u32>),
        access_method_namespace: get!("access_method_namespace", Option<String>),
        access_method_name: get!("access_method_name", Option<String>),
        access_method_type: get!("access_method_type", Option<String>)
            .map(one_char)
            .transpose()?,
        inherits_as_child: get!("inherits_as_child", bool),
        inherits_as_parent: get!("inherits_as_parent", bool),
        physical_attribute_count: get!("relnatts", i16),
    };
    value.validate()?;
    Ok(value)
}

pub(super) fn decode_attribute_row_v1(
    row: &Row,
) -> Result<CatalogAttributeRowV1, PostgresSchemaIdentityUnavailableV1> {
    macro_rules! get {
        ($name:literal, $ty:ty) => {
            row.try_get::<_, $ty>($name)
                .map_err(|_| PostgresSchemaIdentityUnavailableV1::CatalogDecode)?
        };
    }
    let value = CatalogAttributeRowV1 {
        relation_oid: get!("relation_oid", u32),
        attribute_number: get!("attnum", i16),
        attribute_name: get!("attribute_name", Option<String>),
        text_overflow: get!("sf_text_overflow", bool),
        is_dropped: get!("attisdropped", bool),
        is_local: get!("attislocal", bool),
        inheritance_count: get!("attinhcount", i16),
        is_not_null: get!("attnotnull", bool),
        attribute_type_oid: get!("atttypid", u32),
        array_dimensions: get!("attndims", i16),
        type_modifier: get!("atttypmod", i32),
        collation_oid: get!("attcollation", u32),
        joined_type_oid: get!("joined_type_oid", Option<u32>),
        joined_type_name: get!("joined_type_name", Option<String>),
        joined_type_namespace_oid: get!("joined_type_namespace_oid", Option<u32>),
        joined_type_kind: get!("joined_type_kind", Option<String>),
        joined_type_is_defined: get!("joined_type_is_defined", Option<bool>),
        joined_type_base_oid: get!("joined_type_base_oid", Option<u32>),
        joined_type_element_oid: get!("joined_type_element_oid", Option<u32>),
        joined_type_relation_oid: get!("joined_type_relation_oid", Option<u32>),
        joined_type_collation_oid: get!("joined_type_collation_oid", Option<u32>),
        joined_collation_oid: get!("joined_collation_oid", Option<u32>),
        joined_collation_name: get!("joined_collation_name", Option<String>),
        joined_collation_namespace_oid: get!("joined_collation_namespace_oid", Option<u32>),
        joined_collation_provider: get!("joined_collation_provider", Option<String>),
        joined_collation_encoding: get!("joined_collation_encoding", Option<i32>),
        joined_collation_is_deterministic: get!("joined_collation_is_deterministic", Option<bool>),
        joined_collation_version: get!("joined_collation_version", Option<String>),
        physical_ordinal: get!("sf_physical_ordinal", i64),
        physical_overflow: get!("sf_physical_overflow", bool),
    };
    value.validate()?;
    Ok(value)
}

fn one_char(value: String) -> Result<char, PostgresSchemaIdentityUnavailableV1> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(PostgresSchemaIdentityUnavailableV1::CatalogDecode);
    };
    if chars.next().is_some() {
        return Err(PostgresSchemaIdentityUnavailableV1::CatalogDecode);
    }
    Ok(first)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CatalogRelationRowV1 {
    pub(super) relation_oid: u32,
    pub(super) relation_namespace_oid: u32,
    pub(super) joined_namespace_oid: u32,
    pub(super) namespace_name: String,
    pub(super) relation_name: String,
    pub(super) text_overflow: bool,
    pub(super) relation_kind: char,
    pub(super) persistence: char,
    pub(super) is_partition: bool,
    pub(super) is_shared: bool,
    pub(super) row_security: bool,
    pub(super) force_row_security: bool,
    pub(super) of_type_oid: u32,
    pub(super) rewrite_oid: u32,
    pub(super) access_method_oid: u32,
    pub(super) joined_access_method_oid: Option<u32>,
    pub(super) access_method_namespace: Option<String>,
    pub(super) access_method_name: Option<String>,
    pub(super) access_method_type: Option<char>,
    pub(super) inherits_as_child: bool,
    pub(super) inherits_as_parent: bool,
    pub(super) physical_attribute_count: i16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CatalogAttributeRowV1 {
    pub(super) relation_oid: u32,
    pub(super) attribute_number: i16,
    pub(super) attribute_name: Option<String>,
    pub(super) text_overflow: bool,
    pub(super) is_dropped: bool,
    pub(super) is_local: bool,
    pub(super) inheritance_count: i16,
    pub(super) is_not_null: bool,
    pub(super) attribute_type_oid: u32,
    pub(super) array_dimensions: i16,
    pub(super) type_modifier: i32,
    pub(super) collation_oid: u32,
    pub(super) joined_type_oid: Option<u32>,
    pub(super) joined_type_name: Option<String>,
    pub(super) joined_type_namespace_oid: Option<u32>,
    pub(super) joined_type_kind: Option<String>,
    pub(super) joined_type_is_defined: Option<bool>,
    pub(super) joined_type_base_oid: Option<u32>,
    pub(super) joined_type_element_oid: Option<u32>,
    pub(super) joined_type_relation_oid: Option<u32>,
    pub(super) joined_type_collation_oid: Option<u32>,
    pub(super) joined_collation_oid: Option<u32>,
    pub(super) joined_collation_name: Option<String>,
    pub(super) joined_collation_namespace_oid: Option<u32>,
    pub(super) joined_collation_provider: Option<String>,
    pub(super) joined_collation_encoding: Option<i32>,
    pub(super) joined_collation_is_deterministic: Option<bool>,
    pub(super) joined_collation_version: Option<String>,
    pub(super) physical_ordinal: i64,
    pub(super) physical_overflow: bool,
}

impl CatalogRelationRowV1 {
    pub(super) fn into_catalog_fact(
        self,
    ) -> Result<Postgres16RelationCatalogFactV1, PostgresSchemaIdentityUnavailableV1> {
        self.validate()?;
        Ok(Postgres16RelationCatalogFactV1 {
            relation_oid: self.relation_oid,
            relation_namespace_oid: self.relation_namespace_oid,
            joined_namespace_oid: self.joined_namespace_oid,
            namespace_name: self.namespace_name.into_boxed_str(),
            relation_name: self.relation_name.into_boxed_str(),
            relation_kind: self.relation_kind,
            persistence: self.persistence,
            is_shared: self.is_shared,
            is_partition: self.is_partition,
            row_security: self.row_security,
            force_row_security: self.force_row_security,
            of_type_oid: self.of_type_oid,
            rewrite_oid: self.rewrite_oid,
            access_method_oid: self.access_method_oid,
            joined_access_method_oid: self.joined_access_method_oid,
            access_method_name: self.access_method_name.map(String::into_boxed_str),
            access_method_type: self.access_method_type,
            inherits_as_child: self.inherits_as_child,
            inherits_as_parent: self.inherits_as_parent,
            physical_attribute_count: self.physical_attribute_count,
        })
    }

    pub(super) fn validate(&self) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
        if self.text_overflow {
            return Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
                PostgresSchemaIdentityLimitCodeV1::TextBytes,
            ));
        }
        BoundedCatalogTextV1::new(self.namespace_name.clone())?;
        BoundedCatalogTextV1::new(self.relation_name.clone())?;
        if self.access_method_namespace.is_some() != self.access_method_name.is_some()
            || self.access_method_name.is_some() != self.access_method_type.is_some()
        {
            return Err(PostgresSchemaIdentityUnavailableV1::IdentityRejected);
        }
        Ok(())
    }
}

impl CatalogAttributeRowV1 {
    pub(super) fn into_catalog_fact(
        self,
        guard: &CatalogGuardRowV1,
    ) -> Result<Postgres16AttributeCatalogFactV1, PostgresSchemaIdentityUnavailableV1> {
        self.validate()?;
        guard.validate()?;
        if self.is_dropped {
            return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedType);
        }
        let type_fact = Postgres16ColumnTypeCatalogFactV1 {
            type_oid: self
                .joined_type_oid
                .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedType)?,
            type_namespace: "pg_catalog".into(),
            type_name: self
                .joined_type_name
                .clone()
                .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedType)?
                .into_boxed_str(),
            type_kind: one_char(
                self.joined_type_kind
                    .clone()
                    .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedType)?,
            )?,
            type_is_defined: self
                .joined_type_is_defined
                .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedType)?,
            type_base_oid: self
                .joined_type_base_oid
                .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedType)?,
            type_element_oid: self
                .joined_type_element_oid
                .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedType)?,
            type_relation_oid: self
                .joined_type_relation_oid
                .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedType)?,
            array_dimensions: self.array_dimensions,
            type_modifier: self.type_modifier,
            collation_oid: self
                .joined_type_collation_oid
                .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedType)?,
        };
        let collation = self
            .joined_collation_oid
            .map(|oid| {
                Ok(Postgres16DefaultCollationCatalogFactV1 {
                    collation_oid: oid,
                    collation_namespace: "pg_catalog".into(),
                    collation_name: self
                        .joined_collation_name
                        .clone()
                        .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedCollation)?
                        .into_boxed_str(),
                    collation_provider: one_char(
                        self.joined_collation_provider
                            .clone()
                            .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedCollation)?,
                    )?,
                    collation_encoding: self
                        .joined_collation_encoding
                        .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedCollation)?,
                    collation_is_deterministic: self
                        .joined_collation_is_deterministic
                        .ok_or(PostgresSchemaIdentityUnavailableV1::UnsupportedCollation)?,
                    database_provider: one_char(guard.database_provider.clone())?,
                    database_collate: guard.database_collate.clone().into_boxed_str(),
                    database_ctype: guard.database_ctype.clone().into_boxed_str(),
                    database_icu_locale: guard
                        .database_icu_locale
                        .clone()
                        .map(String::into_boxed_str),
                    database_icu_rules: guard
                        .database_icu_rules
                        .clone()
                        .map(String::into_boxed_str),
                    database_recorded_version: guard
                        .database_recorded_version
                        .clone()
                        .map(String::into_boxed_str),
                    actual_version: guard
                        .database_actual_version
                        .clone()
                        .map(String::into_boxed_str),
                })
            })
            .transpose()?;
        Ok(Postgres16AttributeCatalogFactV1 {
            relation_oid: self.relation_oid,
            attribute_number: self.attribute_number,
            attribute_name: self
                .attribute_name
                .ok_or(PostgresSchemaIdentityUnavailableV1::IdentityRejected)?
                .into_boxed_str(),
            is_dropped: self.is_dropped,
            is_local: self.is_local,
            inheritance_count: self.inheritance_count,
            is_not_null: self.is_not_null,
            attribute_type_oid: self.attribute_type_oid,
            array_dimensions: self.array_dimensions,
            type_modifier: self.type_modifier,
            collation_oid: self.collation_oid,
            joined_type: Some(type_fact),
            joined_default_collation: collation,
        })
    }

    pub(super) fn validate(&self) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
        if self.text_overflow || self.attribute_number <= 0 {
            return Err(PostgresSchemaIdentityUnavailableV1::IdentityRejected);
        }
        if self.physical_ordinal <= 0 || self.physical_ordinal > 1_601 {
            return Err(PostgresSchemaIdentityUnavailableV1::IdentityRejected);
        }
        if self.physical_overflow {
            return Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
                PostgresSchemaIdentityLimitCodeV1::PhysicalAttributes,
            ));
        }
        if let Some(name) = &self.attribute_name {
            BoundedCatalogTextV1::new(name.clone())?;
        } else if !self.is_dropped {
            return Err(PostgresSchemaIdentityUnavailableV1::IdentityRejected);
        }
        if self.is_dropped {
            if self.attribute_type_oid != 0
                || self.joined_type_oid.is_some()
                || self.joined_collation_oid.is_some()
            {
                return Err(PostgresSchemaIdentityUnavailableV1::IdentityRejected);
            }
        } else if self.joined_type_oid != Some(self.attribute_type_oid)
            || self.joined_type_name.is_none()
            || self.joined_type_namespace_oid.is_none()
            || self.joined_type_kind.is_none()
            || self.joined_type_is_defined.is_none()
            || self.joined_type_base_oid.is_none()
            || self.joined_type_element_oid.is_none()
            || self.joined_type_relation_oid.is_none()
            || self.joined_type_collation_oid.is_none()
            || (self.collation_oid == 0) != self.joined_collation_oid.is_none()
            || (self.collation_oid != 0 && self.joined_collation_oid != Some(self.collation_oid))
            || (self.collation_oid != 0
                && (self.joined_collation_name.is_none()
                    || self.joined_collation_namespace_oid.is_none()
                    || self.joined_collation_provider.is_none()
                    || self.joined_collation_encoding.is_none()
                    || self.joined_collation_is_deterministic.is_none()))
        {
            return Err(PostgresSchemaIdentityUnavailableV1::IdentityRejected);
        }
        if let Some(name) = &self.joined_type_name {
            BoundedCatalogTextV1::new(name.clone())?;
        }
        if let Some(namespace) = &self.joined_type_namespace_oid {
            if *namespace == 0 {
                return Err(PostgresSchemaIdentityUnavailableV1::IdentityRejected);
            }
        }
        if let Some(name) = &self.joined_collation_name {
            BoundedCatalogTextV1::new(name.clone())?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BoundedCatalogTextV1(Box<str>);

impl BoundedCatalogTextV1 {
    pub(super) fn new(value: String) -> Result<Self, PostgresSchemaIdentityUnavailableV1> {
        if value.is_empty() || value.contains('\0') {
            return Err(PostgresSchemaIdentityUnavailableV1::IdentityRejected);
        }
        if value.len() > MAX_CATALOG_TEXT_BYTES_V1 {
            return Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
                PostgresSchemaIdentityLimitCodeV1::TextBytes,
            ));
        }
        Ok(Self(value.into_boxed_str()))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

pub(super) fn validate_catalog_array_len<T>(
    values: &[T],
) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
    if values.is_empty() || values.len() > MAX_CATALOG_ARRAY_MEMBERS_V1 {
        return Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            PostgresSchemaIdentityLimitCodeV1::KeyMembers,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_guard() -> CatalogGuardRowV1 {
        CatalogGuardRowV1 {
            server_version_num: 160_015,
            server_encoding: "UTF8".into(),
            client_encoding: "UTF8".into(),
            max_identifier_length: 63,
            max_index_keys: 32,
            integer_datetimes: "on".into(),
            session_replication_role: "origin".into(),
            search_path: "pg_catalog,public,pg_temp".into(),
            database_oid: 1,
            database_provider: "c".into(),
            database_collate: "C".into(),
            database_ctype: "C".into(),
            database_icu_locale: None,
            database_icu_rules: None,
            database_recorded_version: None,
            database_actual_version: None,
            public_namespace_count: 1,
            current_database_count: 1,
        }
    }

    #[test]
    fn guard_validator_classifies_each_profile_gate() {
        let mut guard = valid_guard();
        assert!(guard.validate().is_ok());
        guard.server_version_num = 160_014;
        assert_eq!(
            guard.validate(),
            Err(PostgresSchemaIdentityUnavailableV1::UnqualifiedEnginePatch)
        );
        guard.server_version_num = 150_010;
        assert_eq!(
            guard.validate(),
            Err(PostgresSchemaIdentityUnavailableV1::ProfileNotImplemented)
        );
        guard = valid_guard();
        guard.max_identifier_length = 62;
        assert_eq!(
            guard.validate(),
            Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
                PostgresSchemaIdentityGuardCodeV1::IdentifierLength
            ))
        );
        guard = valid_guard();
        guard.search_path = "public".into();
        assert_eq!(
            guard.validate(),
            Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
                PostgresSchemaIdentityGuardCodeV1::SearchPath
            ))
        );
        guard = valid_guard();
        guard.public_namespace_count = 2;
        assert_eq!(
            guard.validate(),
            Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
                PostgresSchemaIdentityGuardCodeV1::PublicNamespace
            ))
        );
        guard = valid_guard();
        guard.max_index_keys = 31;
        assert_eq!(
            guard.validate(),
            Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
                PostgresSchemaIdentityGuardCodeV1::IndexKeyLimit
            ))
        );
    }

    #[test]
    fn text_envelope_is_nonempty_nulfree_and_bounded() {
        assert_eq!(
            BoundedCatalogTextV1::new("public".into()).unwrap().as_str(),
            "public"
        );
        assert_eq!(
            BoundedCatalogTextV1::new(String::new()),
            Err(PostgresSchemaIdentityUnavailableV1::IdentityRejected)
        );
        assert_eq!(
            BoundedCatalogTextV1::new("bad\0name".into()),
            Err(PostgresSchemaIdentityUnavailableV1::IdentityRejected)
        );
        assert_eq!(
            BoundedCatalogTextV1::new("x".repeat(MAX_CATALOG_TEXT_BYTES_V1 + 1)),
            Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
                PostgresSchemaIdentityLimitCodeV1::TextBytes
            ))
        );
    }

    #[test]
    fn array_envelope_rejects_empty_and_cap_plus_one() {
        assert!(validate_catalog_array_len(&[1u8; MAX_CATALOG_ARRAY_MEMBERS_V1]).is_ok());
        assert_eq!(
            validate_catalog_array_len::<u8>(&[]),
            Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
                PostgresSchemaIdentityLimitCodeV1::KeyMembers
            ))
        );
        assert_eq!(
            validate_catalog_array_len(&[1u8; MAX_CATALOG_ARRAY_MEMBERS_V1 + 1]),
            Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
                PostgresSchemaIdentityLimitCodeV1::KeyMembers
            ))
        );
    }

    #[test]
    fn typed_rows_reject_overflow_and_incomplete_join_shapes() {
        let mut relation = CatalogRelationRowV1 {
            relation_oid: 1,
            relation_namespace_oid: 2,
            joined_namespace_oid: 2,
            namespace_name: "public".into(),
            relation_name: "orders".into(),
            text_overflow: false,
            relation_kind: 'r',
            persistence: 'p',
            is_partition: false,
            is_shared: false,
            row_security: false,
            force_row_security: false,
            of_type_oid: 0,
            rewrite_oid: 0,
            access_method_oid: 1,
            joined_access_method_oid: Some(1),
            access_method_namespace: Some("pg_catalog".into()),
            access_method_name: Some("heap".into()),
            access_method_type: Some('t'),
            inherits_as_child: false,
            inherits_as_parent: false,
            physical_attribute_count: 1,
        };
        assert!(relation.validate().is_ok());
        relation.text_overflow = true;
        assert!(relation.validate().is_err());
    }

    #[test]
    fn typed_attribute_rows_require_live_names_but_allow_dropped_names_absent() {
        let dropped = CatalogAttributeRowV1 {
            relation_oid: 1,
            attribute_number: 1,
            attribute_name: None,
            text_overflow: false,
            is_dropped: true,
            is_local: false,
            inheritance_count: 9,
            is_not_null: false,
            attribute_type_oid: 0,
            array_dimensions: 0,
            type_modifier: -1,
            collation_oid: 0,
            joined_type_oid: None,
            joined_type_name: None,
            joined_type_namespace_oid: None,
            joined_type_kind: None,
            joined_type_is_defined: None,
            joined_type_base_oid: None,
            joined_type_element_oid: None,
            joined_type_relation_oid: None,
            joined_type_collation_oid: None,
            joined_collation_oid: None,
            joined_collation_name: None,
            joined_collation_namespace_oid: None,
            joined_collation_provider: None,
            joined_collation_encoding: None,
            joined_collation_is_deterministic: None,
            joined_collation_version: None,
            physical_ordinal: 1,
            physical_overflow: false,
        };
        assert!(dropped.validate().is_ok());
        let mut live = dropped.clone();
        live.is_dropped = false;
        assert!(live.validate().is_err());
        let mut overflow = dropped;
        overflow.physical_ordinal = 1_601;
        overflow.physical_overflow = true;
        assert_eq!(
            overflow.validate(),
            Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
                PostgresSchemaIdentityLimitCodeV1::PhysicalAttributes
            ))
        );
    }
}
