//! Positional row layout is compiler-owned; it does not require executable SQL.
use super::*;

/// Original source columns by emitted position; aggregate values have no raw
/// decoder authority. Shared by metadata and pre-source identity checks.
pub(crate) fn source_projection(
    branch: &Branch,
    distinct: bool,
    dialect: Dialect,
) -> Vec<Option<ColRef>> {
    match &branch.agg {
        Some(agg) if branch.path.is_none() => aggregate_projection(agg, dialect)
            .iter()
            .map(|item| item.source_column().cloned())
            .collect(),
        _ => branch
            .projection_with_distinct(distinct)
            .into_iter()
            .map(Some)
            .collect(),
    }
}

pub(crate) fn projection_layout(b: &Branch, dialect: Dialect) -> Result<Vec<ColRef>> {
    // Keep the same path-before-aggregate precedence as actual emission.
    if b.path.is_some() {
        return Ok(b.projection());
    }
    if let Some(agg) = &b.agg {
        return Ok(aggregate_projection(agg, dialect)
            .iter()
            .map(|item| item.column().clone())
            .collect());
    }
    validate_distinct(b)?;
    Ok(b.projection())
}

pub(super) fn validate_distinct(b: &Branch) -> Result<bool> {
    let term_dedup = crate::cascade::eligible_for_term_dedup(b);
    if b.distinct
        && !term_dedup
        && b.bindings
            .values()
            .any(|def| !crate::cascade::binding_is_injective(def))
    {
        return Err(Error::Unsupported(
            "SELECT DISTINCT over a non-injective term cannot be pushed to raw SQL DISTINCT soundly -> 501 (ADR-0025 C.3)".into()
        ));
    }
    Ok(term_dedup)
}
