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
    with_mode(raw, decode, false, catalog)
}

pub(super) fn with_mode(
    raw: String,
    decode: SqliteDecode,
    natural: bool,
    catalog: &ColumnCatalog,
) -> String {
    catalog
        .lexical_keys
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let padding = decode
        .padding
        .map_or_else(|| "-1".to_owned(), |width| width.to_string());
    let expression = format!(
        "__sf_lexical_key_v1({raw}, {}, {padding})",
        decode.declared_key_code() + if natural { 16 } else { 0 }
    );
    if catalog.suppress_path_collation {
        expression
    } else {
        path_comparison::exact_text(expression, Dialect::Sqlite)
    }
}

pub(super) fn natural_datatype(raw: &str, decode: SqliteDecode) -> String {
    if let Some(code) = decode.declared {
        return format!("'{}'", code.iri().as_str());
    }
    format!("CASE typeof({raw}) WHEN 'integer' THEN 'http://www.w3.org/2001/XMLSchema#integer' WHEN 'real' THEN 'http://www.w3.org/2001/XMLSchema#double' WHEN 'blob' THEN 'http://www.w3.org/2001/XMLSchema#hexBinary' ELSE 'http://www.w3.org/2001/XMLSchema#string' END")
}

/// An explicit datatype may equal the runtime storage-class type. Select the
/// existing callback's natural mode without changing the literal's datatype.
pub(super) fn typed(
    raw: String,
    decode: SqliteDecode,
    datatype: &str,
    catalog: &ColumnCatalog,
) -> String {
    if let Some(code) = decode.declared {
        return with_mode(raw, decode, code.iri().as_str() == datatype, catalog);
    }
    catalog
        .lexical_keys
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let mode = format!(
        "CASE WHEN {} = {} THEN 16 ELSE 0 END",
        natural_datatype(&raw, decode),
        sql_string_literal(datatype)
    );
    let padding = decode
        .padding
        .map_or_else(|| "-1".to_owned(), |n| n.to_string());
    let expression = format!("__sf_lexical_key_v1({raw}, {mode}, {padding})");
    if catalog.suppress_path_collation {
        expression
    } else {
        path_comparison::exact_text(expression, Dialect::Sqlite)
    }
}
