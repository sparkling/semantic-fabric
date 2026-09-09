//! Decoded template projections retain generated-text, not source-column, authority.
use super::*;
use sf_core::ir::{Template, TermSpec, TermType};

pub(super) fn output_decode(
    term: &TermMap,
    dialect: Dialect,
    inner: &AliasActuals,
) -> Option<SqliteDecode> {
    match term {
        TermMap::Column(raw, _) => inner
            .sqlite_columns
            .get(resolve_col(raw, Some(&inner.columns)))
            .copied(),
        TermMap::Template(_, spec)
            if dialect == Dialect::Sqlite && spec.term_type == TermType::Iri =>
        {
            Some(SqliteDecode {
                declared: Some(sf_core::datatype::XsdTypeCode::String),
                padding: None,
            })
        }
        _ => None,
    }
}

pub(super) fn render(
    recipe: &Template,
    spec: &TermSpec,
    alias: usize,
    distinct: bool,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
) -> Result<String> {
    let iri = spec.term_type == TermType::Iri;
    let mut decoded = HashMap::new();
    if dialect == Dialect::Sqlite && iri {
        for segment in recipe.segments() {
            let Segment::Column(name) = segment else {
                continue;
            };
            let column = ColRef::new(alias, name.clone());
            if let Some(decode) = lexical_key::column_decode(&column, actuals) {
                decoded.insert(
                    name.as_ref(),
                    lexical_key::expression(colref(&column, dialect, actuals), decode, catalog),
                );
            } else if distinct {
                return Err(Error::Unsupported(
                    "rendered IRI dedup requires each live SQLite decoder".into(),
                ));
            }
        }
    }
    let expression = render_template_inline(recipe.segments(), iri, dialect, |name| {
        decoded.get(name).cloned().unwrap_or_else(|| {
            path_comparison::rdf_column(&ColRef::new(alias, name), dialect, catalog, actuals)
        })
    })?;
    if distinct && iri && dialect == Dialect::Sqlite && spec.base.is_none() {
        // Final key validation is required even when COUNT hides the generated term.
        Ok(format!("__sf_iri_key_v1({expression}, NULL)"))
    } else {
        Ok(expression)
    }
}

pub(super) fn distinct_key(term: &TermMap, expression: &str, dialect: Dialect) -> Result<String> {
    if supports_distinct(term) && dialect == Dialect::Sqlite {
        Ok(path_comparison::exact_text(expression.to_owned(), dialect))
    } else {
        Err(Error::Unsupported("distinct computed scan recipe".into()))
    }
}

pub(super) fn supports_distinct(term: &TermMap) -> bool {
    matches!(term, TermMap::Template(_, spec) if spec.term_type == TermType::Iri && spec.base.is_none())
}
