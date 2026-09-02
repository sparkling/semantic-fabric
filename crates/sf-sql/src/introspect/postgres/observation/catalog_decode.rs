//! Decode-neutral bounded catalogue envelopes for PostgreSQL 16 (ADR-0051 §8).
//!
//! SQL adapters must validate these envelopes before retaining rich facts. This
//! module deliberately contains no driver calls and no public/runtime wiring.

use super::{PostgresSchemaIdentityLimitCodeV1, PostgresSchemaIdentityUnavailableV1};

pub(super) const MAX_CATALOG_TEXT_BYTES_V1: usize = 256;
pub(super) const MAX_CATALOG_ARRAY_MEMBERS_V1: usize = 32;

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
}
