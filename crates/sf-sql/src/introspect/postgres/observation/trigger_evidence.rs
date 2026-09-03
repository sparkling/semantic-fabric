//! Pure, bounded PostgreSQL 16 foreign-key trigger evidence decoding.

use super::constraints::Postgres16RawForeignKeyTriggersV1;
use super::{PostgresSchemaIdentityLimitCodeV1, PostgresSchemaIdentityUnavailableV1};

const EXPECTED_TRIGGER_ROLES: usize = 4;

/// Decode one server-bounded FK-trigger aggregate.
///
/// Role codes are an order-independent multiset: child insert, child update,
/// parent delete, and parent update are encoded as `1..=4`. Enabled codes are
/// `1` for `O`/`A` and `0` for `D`/`R` under the guarded origin session role.
pub(super) fn decode_postgres16_fk_trigger_evidence_v1(
    overflow: bool,
    role_codes: Option<Vec<i16>>,
    enabled_codes: Option<Vec<i16>>,
) -> Result<Postgres16RawForeignKeyTriggersV1, PostgresSchemaIdentityUnavailableV1> {
    if overflow {
        return Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            PostgresSchemaIdentityLimitCodeV1::KeyMembers,
        ));
    }

    let (Some(role_codes), Some(enabled_codes)) = (role_codes, enabled_codes) else {
        return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint);
    };
    if role_codes.len() != EXPECTED_TRIGGER_ROLES
        || enabled_codes.len() != EXPECTED_TRIGGER_ROLES
        || role_codes.len() != enabled_codes.len()
    {
        return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint);
    }

    let mut seen_roles = [false; EXPECTED_TRIGGER_ROLES];
    for role in role_codes {
        let index = match role {
            1..=4 => role as usize - 1,
            _ => return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint),
        };
        if seen_roles[index] {
            return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint);
        }
        seen_roles[index] = true;
    }

    let mut all_enabled = true;
    for enabled in enabled_codes {
        match enabled {
            0 => all_enabled = false,
            1 => {}
            _ => return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint),
        }
    }

    Ok(Postgres16RawForeignKeyTriggersV1 {
        child_insert_ok: true,
        child_update_ok: true,
        parent_delete_ok: true,
        parent_update_ok: true,
        all_enabled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_trigger_evidence() -> Postgres16RawForeignKeyTriggersV1 {
        Postgres16RawForeignKeyTriggersV1 {
            child_insert_ok: true,
            child_update_ok: true,
            parent_delete_ok: true,
            parent_update_ok: true,
            all_enabled: true,
        }
    }

    fn assert_unsupported(overflow: bool, roles: Option<Vec<i16>>, enabled: Option<Vec<i16>>) {
        assert_eq!(
            decode_postgres16_fk_trigger_evidence_v1(overflow, roles, enabled),
            Err(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)
        );
    }

    fn next_permutation(values: &mut [i16]) -> bool {
        let Some(pivot) = (0..values.len().saturating_sub(1))
            .rev()
            .find(|&index| values[index] < values[index + 1])
        else {
            return false;
        };
        let successor = (pivot + 1..values.len())
            .rev()
            .find(|&index| values[pivot] < values[index])
            .expect("a pivot has a successor");
        values.swap(pivot, successor);
        values[pivot + 1..].reverse();
        true
    }

    #[test]
    fn should_decode_the_exact_enabled_four_role_multiset() {
        assert_eq!(
            decode_postgres16_fk_trigger_evidence_v1(
                false,
                Some(vec![1, 2, 3, 4]),
                Some(vec![1, 1, 1, 1]),
            ),
            Ok(enabled_trigger_evidence())
        );
    }

    #[test]
    fn should_accept_every_role_permutation_without_relation_coordinates() {
        let mut roles = [1, 2, 3, 4];
        let mut accepted = 0;
        loop {
            assert!(decode_postgres16_fk_trigger_evidence_v1(
                false,
                Some(roles.to_vec()),
                Some(vec![1, 1, 1, 1]),
            )
            .is_ok());
            accepted += 1;
            if !next_permutation(&mut roles) {
                break;
            }
        }
        assert_eq!(accepted, 24);
    }

    #[test]
    fn should_reject_every_missing_role_duplicate_substitution() {
        for missing in 1..=4 {
            for duplicate in 1..=4 {
                if duplicate == missing {
                    continue;
                }
                let mut roles = vec![1, 2, 3, 4];
                roles[(missing - 1) as usize] = duplicate;
                assert_unsupported(false, Some(roles), Some(vec![1, 1, 1, 1]));
            }
        }
    }

    #[test]
    fn should_reject_missing_or_mismatched_evidence_arrays() {
        for (roles, enabled) in [
            (None, Some(vec![1, 1, 1, 1])),
            (Some(vec![1, 2, 3, 4]), None),
            (Some(vec![1, 2, 3]), Some(vec![1, 1, 1])),
            (Some(vec![1, 2, 3, 4]), Some(vec![1, 1, 1])),
            (Some(vec![1, 2, 3]), Some(vec![1, 1, 1, 1])),
        ] {
            assert_unsupported(false, roles, enabled);
        }
    }

    #[test]
    fn should_reject_the_fifth_row_overflow_sentinel() {
        assert_eq!(
            decode_postgres16_fk_trigger_evidence_v1(
                true,
                Some(vec![1, 2, 3, 4, 0]),
                Some(vec![1, 1, 1, 1, -1]),
            ),
            Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
                PostgresSchemaIdentityLimitCodeV1::KeyMembers,
            ))
        );
    }

    #[test]
    fn should_reject_unknown_role_and_enabled_codes() {
        for unknown_role in [-1, 0, 5] {
            assert_unsupported(
                false,
                Some(vec![unknown_role, 2, 3, 4]),
                Some(vec![1, 1, 1, 1]),
            );
        }
        for unknown_enabled in [-1, 2] {
            assert_unsupported(
                false,
                Some(vec![1, 2, 3, 4]),
                Some(vec![1, unknown_enabled, 1, 1]),
            );
        }
    }

    #[test]
    fn should_accept_origin_disabled_codes_as_not_enforced() {
        for enabled in [
            vec![0, 1, 1, 1],
            vec![1, 0, 1, 1],
            vec![1, 1, 0, 1],
            vec![1, 1, 1, 0],
            vec![0, 0, 0, 0],
        ] {
            let decoded = decode_postgres16_fk_trigger_evidence_v1(
                false,
                Some(vec![1, 2, 3, 4]),
                Some(enabled),
            )
            .expect("D/R-derived zero is valid structural evidence");
            assert!(!decoded.all_enabled);
            assert!(decoded.child_insert_ok);
            assert!(decoded.child_update_ok);
            assert!(decoded.parent_delete_ok);
            assert!(decoded.parent_update_ok);
        }
    }
}
