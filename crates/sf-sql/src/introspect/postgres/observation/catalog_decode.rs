//! Decode-neutral bounded catalogue envelopes for PostgreSQL 16 (ADR-0051 §8).
//!
//! SQL adapters must validate these envelopes before retaining rich facts. This
//! module deliberately contains no driver calls and no public/runtime wiring.

use super::{PostgresSchemaIdentityLimitCodeV1, PostgresSchemaIdentityUnavailableV1};

pub(super) const MAX_CATALOG_TEXT_BYTES_V1: usize = 256;
pub(super) const MAX_CATALOG_ARRAY_MEMBERS_V1: usize = 32;

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
