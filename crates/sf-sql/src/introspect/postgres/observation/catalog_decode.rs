//! Decode-neutral bounded catalogue envelopes for PostgreSQL 16 (ADR-0051 §8).
//!
//! SQL adapters must validate these envelopes before retaining rich facts. This
//! module deliberately contains no driver calls and no public/runtime wiring.

use super::{PostgresSchemaIdentityLimitCodeV1, PostgresSchemaIdentityUnavailableV1};
use tokio_postgres::Row;

pub(super) const MAX_CATALOG_TEXT_BYTES_V1: usize = 256;
pub(super) const MAX_CATALOG_ARRAY_MEMBERS_V1: usize = 32;

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
    pub(super) public_namespace_count: i64,
    pub(super) current_database_count: i64,
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
    Ok(CatalogGuardRowV1 {
        server_version_num: get!("server_version_num", i32),
        server_encoding: get!("server_encoding", String),
        client_encoding: get!("client_encoding", String),
        max_identifier_length: get!("max_identifier_length", i32),
        max_index_keys: get!("max_index_keys", i32),
        integer_datetimes: get!("integer_datetimes", String),
        session_replication_role: get!("session_replication_role", String),
        search_path: get!("search_path", String),
        public_namespace_count: get!("public_namespace_count", i64),
        current_database_count: get!("current_database_count", i64),
    })
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
    pub(super) row_security: bool,
    pub(super) force_row_security: bool,
    pub(super) of_type_oid: u32,
    pub(super) rewrite_oid: u32,
    pub(super) access_method_oid: u32,
    pub(super) joined_access_method_oid: Option<u32>,
    pub(super) access_method_namespace: Option<String>,
    pub(super) access_method_name: Option<String>,
    pub(super) access_method_type: Option<char>,
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
}

impl CatalogRelationRowV1 {
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
    pub(super) fn validate(&self) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
        if self.text_overflow || self.attribute_number <= 0 {
            return Err(PostgresSchemaIdentityUnavailableV1::IdentityRejected);
        }
        if let Some(name) = &self.attribute_name {
            BoundedCatalogTextV1::new(name.clone())?;
        } else if !self.is_dropped {
            return Err(PostgresSchemaIdentityUnavailableV1::IdentityRejected);
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
            row_security: false,
            force_row_security: false,
            of_type_oid: 0,
            rewrite_oid: 0,
            access_method_oid: 1,
            joined_access_method_oid: Some(1),
            access_method_namespace: Some("pg_catalog".into()),
            access_method_name: Some("heap".into()),
            access_method_type: Some('t'),
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
        };
        assert!(dropped.validate().is_ok());
        let mut live = dropped.clone();
        live.is_dropped = false;
        assert!(live.validate().is_err());
    }
}
