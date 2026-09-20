//! Bound catalogue lookup, quoting and text-key decoration before native SQL.
use super::*;
use crate::iq::HopRelation;

pub(super) fn quote(name: &str, dialect: Dialect, work: SourceWork<'_>) -> Result<String> {
    // Ident owns a name copy; Display scans it and doubles embedded quotes.
    work.product(name.len(), 2).map_err(error)?;
    let quotes = name
        .bytes()
        .filter(|b| *b == dialect.quote_char() as u8)
        .count();
    work.charge(name.len()).map_err(error)?;
    work.charge(quotes).map_err(error)?;
    work.charge(2).map_err(error)?;
    Ok(dialect.quote_ident(name))
}

fn source_key_paid(source: &LogicalSource, work: SourceWork<'_>) -> Result<String> {
    match source {
        LogicalSource::Table(name) => format::render(work, format_args!("t:{name}")),
        LogicalSource::Query(sql) => format::render(work, format_args!("q:{sql}")),
    }
}

fn endpoint(
    rel: &HopRelation,
    raw: &str,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: SourceWork<'_>,
) -> Result<String> {
    let key = source_key_paid(&rel.source, work)?;
    work.charge(key.len()).map_err(error)?;
    let columns = catalog.by_source.get(&key).map(Vec::as_slice);
    if let Some(columns) = columns {
        // resolve_col can scan once for an exact name, once for a folded name.
        for column in columns {
            work.charge(2).map_err(error)?;
            work.product(raw.len().max(column.len()), 2)
                .map_err(error)?;
        }
    }
    let name = resolve_col(raw, columns);
    let expression =
        if dialect == Dialect::Postgres && physical_row_identifier(&rel.source, raw, dialect) {
            format::render(work, format_args!("(h0.ctid)::text"))?
        } else {
            let name = quote(name, dialect, work)?;
            format::render(work, format_args!("h0.{name}"))?
        };
    work.charge(key.len()).map_err(error)?;
    work.charge(name.len()).map_err(error)?;
    let text = catalog
        .text_by_source
        .get(&key)
        .and_then(|cols| cols.get(name))
        .copied();
    let expression = match (dialect, text) {
        (Dialect::Sqlite, Some(TextKey::SqliteCharacter(width))) => {
            catalog
                .character_keys
                .store(true, std::sync::atomic::Ordering::Relaxed);
            format::render(
                work,
                format_args!("__sf_character_key_v1({expression}, {width})"),
            )?
        }
        (Dialect::Postgres, Some(TextKey::PostgresCharacter)) => format::render(
            work,
            format_args!("(pg_catalog.convert_from(pg_catalog.bpcharsend({expression}), 'UTF8'))"),
        )?,
        _ => expression,
    };
    if !catalog.suppress_path_collation && (dialect == Dialect::Sqlite || text.is_some()) {
        match dialect {
            Dialect::Sqlite => format::render(work, format_args!("({expression} COLLATE BINARY)")),
            Dialect::Postgres => format::render(work, format_args!("({expression} COLLATE \"C\")")),
            Dialect::MySql => format::render(
                work,
                format_args!("(CONVERT({expression} USING utf8mb4) COLLATE utf8mb4_0900_bin)"),
            ),
            _ => Ok(expression),
        }
    } else {
        Ok(expression)
    }
}

fn inputs(
    rel: &HopRelation,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: SourceWork<'_>,
) -> Result<(String, String, String, String, String)> {
    let src = match &rel.source {
        LogicalSource::Table(name) => quote(name, dialect, work)?,
        LogicalSource::Query(sql) => format::render(work, format_args!("({sql})"))?,
    };
    let s = endpoint(rel, &rel.subj_col, dialect, catalog, work)?;
    let o = endpoint(rel, &rel.obj_col, dialect, catalog, work)?;
    Ok((
        src,
        s,
        o,
        quote("sf_s", dialect, work)?,
        quote("sf_o", dialect, work)?,
    ))
}

pub(super) fn relation(
    rel: &HopRelation,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: SourceWork<'_>,
) -> Result<String> {
    let (src, s, o, sf_s, sf_o) = inputs(rel, dialect, catalog, work)?;
    format::render(work, format_args!("SELECT {s} AS {sf_s}, {o} AS {sf_o} FROM {src} h0 WHERE {s} IS NOT NULL AND {o} IS NOT NULL"))
}

pub(super) fn reflexive(
    hop: &HopExpr,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: SourceWork<'_>,
) -> Result<String> {
    let rel = hop.as_pred().ok_or_else(|| {
        Error::Unsupported("reflexive (P*/p?) path over a composite hop -> 501".into())
    })?;
    let (src, s, o, sf_s, sf_o) = inputs(rel, dialect, catalog, work)?;
    format::render(work, format_args!("SELECT {s} AS {sf_s}, {s} AS {sf_o} FROM {src} h0 WHERE {s} IS NOT NULL AND {o} IS NOT NULL UNION SELECT {o} AS {sf_s}, {o} AS {sf_o} FROM {src} h0 WHERE {s} IS NOT NULL AND {o} IS NOT NULL"))
}
