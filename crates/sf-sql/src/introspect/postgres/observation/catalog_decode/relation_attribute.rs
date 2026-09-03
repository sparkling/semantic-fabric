//! PostgreSQL relation and attribute catalogue decoding.

use super::super::relation::{Postgres16AttributeCatalogFactV1, Postgres16RelationCatalogFactV1};
use super::super::source_type::{
    Postgres16ColumnTypeCatalogFactV1, Postgres16DefaultCollationCatalogFactV1,
};
use super::super::{PostgresSchemaIdentityLimitCodeV1, PostgresSchemaIdentityUnavailableV1};
use super::guard::CatalogGuardRowV1;
use super::{one_char, BoundedCatalogTextV1};
use tokio_postgres::Row;

pub(in crate::introspect::postgres::observation) fn decode_relation_row_v1(
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

pub(in crate::introspect::postgres::observation) fn decode_attribute_row_v1(
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::introspect::postgres::observation) struct CatalogRelationRowV1 {
    pub(in crate::introspect::postgres::observation) relation_oid: u32,
    pub(in crate::introspect::postgres::observation) relation_namespace_oid: u32,
    pub(in crate::introspect::postgres::observation) joined_namespace_oid: u32,
    pub(in crate::introspect::postgres::observation) namespace_name: String,
    pub(in crate::introspect::postgres::observation) relation_name: String,
    pub(in crate::introspect::postgres::observation) text_overflow: bool,
    pub(in crate::introspect::postgres::observation) relation_kind: char,
    pub(in crate::introspect::postgres::observation) persistence: char,
    pub(in crate::introspect::postgres::observation) is_partition: bool,
    pub(in crate::introspect::postgres::observation) is_shared: bool,
    pub(in crate::introspect::postgres::observation) row_security: bool,
    pub(in crate::introspect::postgres::observation) force_row_security: bool,
    pub(in crate::introspect::postgres::observation) of_type_oid: u32,
    pub(in crate::introspect::postgres::observation) rewrite_oid: u32,
    pub(in crate::introspect::postgres::observation) access_method_oid: u32,
    pub(in crate::introspect::postgres::observation) joined_access_method_oid: Option<u32>,
    pub(in crate::introspect::postgres::observation) access_method_namespace: Option<String>,
    pub(in crate::introspect::postgres::observation) access_method_name: Option<String>,
    pub(in crate::introspect::postgres::observation) access_method_type: Option<char>,
    pub(in crate::introspect::postgres::observation) inherits_as_child: bool,
    pub(in crate::introspect::postgres::observation) inherits_as_parent: bool,
    pub(in crate::introspect::postgres::observation) physical_attribute_count: i16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::introspect::postgres::observation) struct CatalogAttributeRowV1 {
    pub(in crate::introspect::postgres::observation) relation_oid: u32,
    pub(in crate::introspect::postgres::observation) attribute_number: i16,
    pub(in crate::introspect::postgres::observation) attribute_name: Option<String>,
    pub(in crate::introspect::postgres::observation) text_overflow: bool,
    pub(in crate::introspect::postgres::observation) is_dropped: bool,
    pub(in crate::introspect::postgres::observation) is_local: bool,
    pub(in crate::introspect::postgres::observation) inheritance_count: i16,
    pub(in crate::introspect::postgres::observation) is_not_null: bool,
    pub(in crate::introspect::postgres::observation) attribute_type_oid: u32,
    pub(in crate::introspect::postgres::observation) array_dimensions: i16,
    pub(in crate::introspect::postgres::observation) type_modifier: i32,
    pub(in crate::introspect::postgres::observation) collation_oid: u32,
    pub(in crate::introspect::postgres::observation) joined_type_oid: Option<u32>,
    pub(in crate::introspect::postgres::observation) joined_type_name: Option<String>,
    pub(in crate::introspect::postgres::observation) joined_type_namespace_oid: Option<u32>,
    pub(in crate::introspect::postgres::observation) joined_type_kind: Option<String>,
    pub(in crate::introspect::postgres::observation) joined_type_is_defined: Option<bool>,
    pub(in crate::introspect::postgres::observation) joined_type_base_oid: Option<u32>,
    pub(in crate::introspect::postgres::observation) joined_type_element_oid: Option<u32>,
    pub(in crate::introspect::postgres::observation) joined_type_relation_oid: Option<u32>,
    pub(in crate::introspect::postgres::observation) joined_type_collation_oid: Option<u32>,
    pub(in crate::introspect::postgres::observation) joined_collation_oid: Option<u32>,
    pub(in crate::introspect::postgres::observation) joined_collation_name: Option<String>,
    pub(in crate::introspect::postgres::observation) joined_collation_namespace_oid: Option<u32>,
    pub(in crate::introspect::postgres::observation) joined_collation_provider: Option<String>,
    pub(in crate::introspect::postgres::observation) joined_collation_encoding: Option<i32>,
    pub(in crate::introspect::postgres::observation) joined_collation_is_deterministic:
        Option<bool>,
    pub(in crate::introspect::postgres::observation) joined_collation_version: Option<String>,
    pub(in crate::introspect::postgres::observation) physical_ordinal: i64,
    pub(in crate::introspect::postgres::observation) physical_overflow: bool,
}

impl CatalogRelationRowV1 {
    pub(in crate::introspect::postgres::observation) fn into_catalog_fact(
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

    pub(in crate::introspect::postgres::observation) fn validate(
        &self,
    ) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
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
    pub(in crate::introspect::postgres::observation) fn into_catalog_fact(
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

    pub(in crate::introspect::postgres::observation) fn validate(
        &self,
    ) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
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
