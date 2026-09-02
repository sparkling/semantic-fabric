use crate::error::{Error, Result};

pub(super) const MAX_LEGACY_RELATIONS_PG16_V1: usize = 4_096;
pub(super) const MAX_LEGACY_ROWS_PER_SET_PG16_V1: usize = 65_536;

#[derive(Clone, Copy)]
pub(super) struct LegacyInputLimitsV1 {
    pub(super) max_relations: usize,
    pub(super) max_name_bytes: usize,
    pub(super) max_total_name_bytes: usize,
}

pub(super) const PRODUCTION_LEGACY_INPUT_LIMITS_V1: LegacyInputLimitsV1 = LegacyInputLimitsV1 {
    max_relations: MAX_LEGACY_RELATIONS_PG16_V1,
    max_name_bytes: 256,
    max_total_name_bytes: 1_048_576,
};

pub(super) fn validate_legacy_table_names<'a>(
    names: impl ExactSizeIterator<Item = &'a str>,
    limits: LegacyInputLimitsV1,
) -> Result<()> {
    if names.len() > limits.max_relations {
        return Err(input_limit("table count"));
    }
    let mut total_name_bytes = 0;
    for name in names {
        if name.len() > limits.max_name_bytes {
            return Err(input_limit("table name"));
        }
        total_name_bytes =
            checked_total_name_bytes(total_name_bytes, name.len(), limits.max_total_name_bytes)?;
    }
    Ok(())
}

pub(super) fn checked_total_name_bytes(
    current: usize,
    next: usize,
    maximum: usize,
) -> Result<usize> {
    current
        .checked_add(next)
        .filter(|total| *total <= maximum)
        .ok_or_else(|| input_limit("table-name bytes"))
}

fn input_limit(resource: &'static str) -> Error {
    Error::Introspection(format!(
        "PostgreSQL introspection {resource} exceeds the configured limit"
    ))
}

#[cfg(test)]
mod tests;
