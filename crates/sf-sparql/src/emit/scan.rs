//! Rendering of typed scan relations. No generated SQL is a live source authority.
use super::*;
use crate::iq::{Scan, ScanSource};

pub(super) fn scan_ref(scan: &Scan, dialect: Dialect, catalog: &ColumnCatalog) -> Result<String> {
    let alias = scan.alias;
    match &scan.source {
        ScanSource::Logical(LogicalSource::Table(table)) => {
            Ok(format!("{} t{alias}", dialect.quote_ident(table)))
        }
        ScanSource::Logical(LogicalSource::Query(query)) => Ok(format!("({query}) t{alias}")),
        ScanSource::Path { closure, cte_alias } => {
            let sql = path_as_derived_table_sql(closure, *cte_alias, dialect, catalog)?;
            Ok(format!("({sql}) t{alias}"))
        }
    }
}

pub(super) fn scan_actuals(scan: &Scan, catalog: &ColumnCatalog) -> AliasActuals {
    match &scan.source {
        ScanSource::Logical(source) => source_actuals(source, catalog),
        ScanSource::Path { .. } => AliasActuals {
            source_kind: AliasSourceKind::Derived,
            columns: vec!["sf_s".into(), "sf_o".into()],
        },
    }
}
