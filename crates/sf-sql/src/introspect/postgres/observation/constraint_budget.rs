//! Combined NOT NULL and catalogue-constraint capture budget (ADR-0051 §8).

use sf_core::schema_identity::MAX_RAW_CONSTRAINTS_V1;

use super::{PostgresSchemaIdentityLimitCodeV1, PostgresSchemaIdentityUnavailableV1};

pub(super) fn constraint_catalog_budget_v1(
    not_null_count: usize,
) -> Result<(usize, i64), PostgresSchemaIdentityUnavailableV1> {
    let catalog_cap = MAX_RAW_CONSTRAINTS_V1.checked_sub(not_null_count).ok_or(
        PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            PostgresSchemaIdentityLimitCodeV1::RawConstraints,
        ),
    )?;
    Ok((catalog_cap, catalog_cap as i64 + 1))
}

#[cfg(test)]
mod tests {
    use super::super::{PostgresSchemaIdentityLimitCodeV1, PostgresSchemaIdentityUnavailableV1};
    use super::constraint_catalog_budget_v1;
    use sf_core::schema_identity::MAX_RAW_CONSTRAINTS_V1;

    #[test]
    fn combined_constraint_budget_reserves_exact_cap_plus_one_sentinel() {
        assert_eq!(
            constraint_catalog_budget_v1(0),
            Ok((MAX_RAW_CONSTRAINTS_V1, MAX_RAW_CONSTRAINTS_V1 as i64 + 1))
        );
        assert_eq!(
            constraint_catalog_budget_v1(MAX_RAW_CONSTRAINTS_V1 - 1),
            Ok((1, 2))
        );
        assert_eq!(
            constraint_catalog_budget_v1(MAX_RAW_CONSTRAINTS_V1),
            Ok((0, 1))
        );
    }

    #[test]
    fn not_null_count_over_combined_cap_fails_before_catalogue_io() {
        assert_eq!(
            constraint_catalog_budget_v1(MAX_RAW_CONSTRAINTS_V1 + 1),
            Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
                PostgresSchemaIdentityLimitCodeV1::RawConstraints,
            ))
        );
    }
}
