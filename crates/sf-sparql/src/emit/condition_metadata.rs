//! Prospective copies and lookups for nested condition scopes.
use super::*;
use sf_sql::source_work::SourceWork;
use source_control::validation_error as error;

pub(super) fn copy(actuals: &ActualColumns, work: SourceWork<'_>) -> Result<ActualColumns> {
    work.product(
        actuals.len(),
        1 + std::mem::size_of::<(usize, AliasActuals)>(),
    )
    .map_err(error)?;
    for source in actuals.values() {
        work.product(source.columns.len(), 1 + std::mem::size_of::<String>())
            .map_err(error)?;
        for name in &source.columns {
            work.charge(name.len()).map_err(error)?;
        }
        source_control::map_copy(&source.datatype_columns, work).map_err(error)?;
        source_control::map_copy(&source.natural_columns, work).map_err(error)?;
        source_control::map_copy(&source.scalar_columns, work).map_err(error)?;
        source_control::map_copy(&source.text_columns, work).map_err(error)?;
        source_control::map_copy(&source.sqlite_columns, work).map_err(error)?;
        source_control::map_copy(&source.lexical_columns, work).map_err(error)?;
        source_control::map_copy(&source.lexical_comparison_columns, work).map_err(error)?;
        for set in [&source.static_iri_columns, &source.iri_unreserved_columns] {
            work.product(set.len(), 1 + std::mem::size_of::<String>())
                .map_err(error)?;
            for name in set {
                work.charge(name.len()).map_err(error)?;
            }
        }
    }
    let result = actuals.clone();
    work.checkpoint().map_err(error)?;
    Ok(result)
}

pub(super) fn install(
    actuals: &mut ActualColumns,
    alias: usize,
    value: AliasActuals,
    work: SourceWork<'_>,
) -> Result<()> {
    work.charge(1).map_err(error)?;
    work.product(
        actuals.len(),
        2 + std::mem::size_of::<(usize, AliasActuals)>(),
    )
    .map_err(error)?;
    work.charge(std::mem::size_of::<(usize, AliasActuals)>())
        .map_err(error)?;
    actuals
        .try_reserve(1)
        .map_err(|_| Error::Sql("condition metadata allocation failed".into()))?;
    actuals.insert(alias, value);
    work.checkpoint().map_err(error)
}

fn column(column: &ColRef, actuals: &ActualColumns, work: SourceWork<'_>) -> Result<()> {
    work.charge(actuals.len()).map_err(error)?;
    work.charge(column.column.len()).map_err(error)?;
    if let Some(source) = actuals.get(&column.alias) {
        for name in &source.columns {
            work.charge(2).map_err(error)?;
            work.product(2, name.len().min(column.column.len()))
                .map_err(error)?;
        }
        let name = resolve_col(&column.column, Some(&source.columns));
        work.charge(name.len()).map_err(error)?;
        // Pay a full lookup envelope; randomized hash iteration never selects
        // which entries receive a charge.
        for key in source
            .natural_columns
            .keys()
            .chain(source.scalar_columns.keys())
        {
            work.charge(1).map_err(error)?;
            work.charge(key.len().min(name.len())).map_err(error)?;
        }
    }
    Ok(())
}

pub(super) fn validation(
    condition: &SqlCond,
    actuals: &ActualColumns,
    work: SourceWork<'_>,
) -> Result<()> {
    match condition {
        SqlCond::DecodedIsNotNull(value) => column(value, actuals, work)?,
        SqlCond::IriCmp(cmp) => {
            for value in cmp.columns() {
                column(value, actuals, work)?;
            }
        }
        SqlCond::LiteralCmp(cmp) => {
            for value in cmp.columns() {
                column(value, actuals, work)?;
            }
        }
        _ => {}
    }
    Ok(())
}
