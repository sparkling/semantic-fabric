//! Decode-neutral bounded catalogue envelopes for PostgreSQL 16 (ADR-0051 §8).
//!
//! SQL adapters validate these envelopes before retaining rich facts. Driver
//! calls stay in the observation boundary; the decoders remain deterministic.

mod constraint;
mod guard;
mod relation_attribute;

pub(super) use constraint::{decode_constraint_row_v1, CatalogConstraintRowV1};
pub(super) use guard::decode_guard_row_v1;
pub(super) use relation_attribute::{
    decode_attribute_row_v1, decode_relation_row_v1, CatalogAttributeRowV1, CatalogRelationRowV1,
};

use super::constraints::Postgres16RawConstraintV1;
use super::{PostgresSchemaIdentityLimitCodeV1, PostgresSchemaIdentityUnavailableV1};
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

pub(super) fn adapt_constraint_rows_v1(
    rows: Vec<CatalogConstraintRowV1>,
) -> Result<Vec<Postgres16RawConstraintV1>, PostgresSchemaIdentityUnavailableV1> {
    rows.into_iter()
        .map(CatalogConstraintRowV1::into_raw_constraint)
        .collect()
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
mod tests;
