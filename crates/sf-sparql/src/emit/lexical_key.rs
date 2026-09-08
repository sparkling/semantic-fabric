//! Only original lexical RDF consumers may use the shared SQLite decoder key.
use super::*;

pub(super) fn column_decode(column: &ColRef, actuals: &ActualColumns) -> Option<SqliteDecode> {
    let source = actuals.get(&column.alias)?;
    source
        .sqlite_columns
        .get(resolve_col(&column.column, Some(&source.columns)))
        .copied()
}

pub(super) fn proven_column(column: &ColRef, actuals: &ActualColumns) -> Option<SqliteDecode> {
    let source = actuals.get(&column.alias)?;
    source
        .lexical_columns
        .get(resolve_col(&column.column, Some(&source.columns)))
        .copied()
}

pub(super) fn expression(raw: String, decode: SqliteDecode, catalog: &ColumnCatalog) -> String {
    catalog
        .lexical_keys
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let padding = decode
        .padding
        .map_or_else(|| "-1".to_owned(), |width| width.to_string());
    let expression = format!(
        "__sf_lexical_key_v1({raw}, {}, {padding})",
        decode.declared_key_code()
    );
    if catalog.suppress_path_collation {
        expression
    } else {
        path_comparison::exact_text(expression, Dialect::Sqlite)
    }
}
