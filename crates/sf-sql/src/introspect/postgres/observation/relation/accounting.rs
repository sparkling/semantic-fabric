use sf_core::schema_identity::{QualifiedNameV1, SourceTypeV1, TypeFacetValueV1};

use super::{
    canonical_body_limit, ensure_collection_bound, Postgres16AttributeCatalogFactV1,
    Postgres16RelationCatalogFactV1, PostgresSchemaIdentityLimitCodeV1,
    PostgresSchemaIdentityUnavailableV1,
};

pub(super) fn validate_raw_text_accounting(
    candidates: &[Postgres16RelationCatalogFactV1],
    attributes: &[Postgres16AttributeCatalogFactV1],
    maximum: usize,
) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
    let mut total = 0usize;
    for candidate in candidates {
        add_raw_text(&mut total, &candidate.namespace_name, maximum)?;
        add_raw_text(&mut total, &candidate.relation_name, maximum)?;
        if let Some(value) = candidate.access_method_name.as_deref() {
            add_raw_text(&mut total, value, maximum)?;
        }
    }
    for attribute in attributes {
        add_raw_text(&mut total, &attribute.attribute_name, maximum)?;
        if let Some(fact) = &attribute.joined_type {
            add_raw_text(&mut total, &fact.type_namespace, maximum)?;
            add_raw_text(&mut total, &fact.type_name, maximum)?;
        }
        if let Some(fact) = &attribute.joined_default_collation {
            for value in [
                Some(fact.collation_namespace.as_ref()),
                Some(fact.collation_name.as_ref()),
                Some(fact.database_collate.as_ref()),
                Some(fact.database_ctype.as_ref()),
                fact.database_icu_locale.as_deref(),
                fact.database_icu_rules.as_deref(),
                fact.database_recorded_version.as_deref(),
                fact.actual_version.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                add_raw_text(&mut total, value, maximum)?;
            }
        }
    }
    Ok(())
}

fn add_raw_text(
    total: &mut usize,
    value: &str,
    maximum: usize,
) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
    *total = total
        .checked_add(value.len())
        .ok_or_else(canonical_body_limit)?;
    ensure_collection_bound(
        *total,
        maximum,
        PostgresSchemaIdentityLimitCodeV1::CanonicalBody,
    )
}

pub(super) fn add_semantic_text(
    total: &mut usize,
    value: &str,
    maximum: usize,
) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
    *total = total.checked_add(value.len()).ok_or({
        PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            PostgresSchemaIdentityLimitCodeV1::TextBytes,
        )
    })?;
    ensure_collection_bound(
        *total,
        maximum,
        PostgresSchemaIdentityLimitCodeV1::TextBytes,
    )
}

fn add_qualified_name_text(
    total: &mut usize,
    name: &QualifiedNameV1,
    maximum: usize,
) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
    if let Some(value) = &name.catalog {
        add_semantic_text(total, value.as_str(), maximum)?;
    }
    if let Some(value) = &name.schema {
        add_semantic_text(total, value.as_str(), maximum)?;
    }
    add_semantic_text(total, name.local.as_str(), maximum)
}

pub(super) fn add_source_type_text(
    total: &mut usize,
    source_type: &SourceTypeV1,
    maximum: usize,
) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
    add_qualified_name_text(total, &source_type.native_name, maximum)?;
    for facet in &source_type.facets {
        add_semantic_text(total, facet.key.as_str(), maximum)?;
        match &facet.value {
            TypeFacetValueV1::Text(value) => add_semantic_text(total, value.as_str(), maximum)?,
            TypeFacetValueV1::TypeName(name) => add_qualified_name_text(total, name, maximum)?,
            TypeFacetValueV1::TextList(values) => {
                for value in values {
                    add_semantic_text(total, value.as_str(), maximum)?;
                }
            }
            TypeFacetValueV1::TypeNameList(names) => {
                for name in names {
                    add_qualified_name_text(total, name, maximum)?;
                }
            }
            TypeFacetValueV1::Bool(_)
            | TypeFacetValueV1::U64(_)
            | TypeFacetValueV1::I64(_)
            | TypeFacetValueV1::Digest32(_) => {}
        }
    }
    Ok(())
}
