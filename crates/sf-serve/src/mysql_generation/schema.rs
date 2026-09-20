//! Bounded, conservative metadata captured only after the complete MDL barrier.
use mysql_async::{prelude::Queryable, Conn, Params, Value};
use sf_core::{Column, TableSchema};
use sf_sql::source_work::{SourceVec, SourceWork};

use crate::pg_generation::PgGenerationError;

const MAX_COLUMNS: usize = 1024;
const MAX_SCHEMA_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Schema {
    pub(crate) session: Vec<Vec<Option<String>>>,
    pub(crate) tables: Vec<TableSchema>,
    facts: Vec<Vec<Vec<Option<String>>>>,
    comparison_work: usize,
}

impl Schema {
    pub(crate) fn matches(
        &self,
        other: &Self,
        work: SourceWork<'_>,
    ) -> Result<bool, PgGenerationError> {
        work.charge(self.comparison_work + other.comparison_work)
            .map_err(sql_error)?;
        Ok(self == other)
    }

    pub(crate) fn compiler_tables(
        &self,
        work: SourceWork<'_>,
    ) -> Result<Vec<TableSchema>, PgGenerationError> {
        let mut tables = work.vector(self.tables.len()).map_err(sql_error)?;
        for source in &self.tables {
            let mut table = TableSchema::new(work.string(&source.name).map_err(sql_error)?);
            table.columns = work.vector(source.columns.len()).map_err(sql_error)?;
            for column in &source.columns {
                table.columns.push(Column::new(
                    work.string(&column.name).map_err(sql_error)?,
                    work.string(&column.sql_type).map_err(sql_error)?,
                    column.not_null,
                ));
            }
            tables.push(table);
        }
        Ok(tables)
    }
}

pub(crate) fn sql_error(error: sf_sql::Error) -> PgGenerationError {
    match error {
        sf_sql::Error::QueryControl(error) => PgGenerationError::Control(error),
        _ => PgGenerationError::SourceUnavailable,
    }
}

pub(crate) fn identifier(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name
            .bytes()
            .enumerate()
            .all(|(i, c)| c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}

/// SQL projections must cap every field at its admitted width plus one. Read
/// one native row at a time and charge copies/growth before owning metadata.
pub(crate) async fn rows(
    conn: &mut Conn,
    sql: &str,
    params: Params,
    cap: usize,
    widths: &[usize],
    work: SourceWork<'_>,
    bytes: &mut usize,
) -> Result<Vec<Vec<Option<String>>>, PgGenerationError> {
    work.charge(1).map_err(sql_error)?;
    let mut cursor = conn
        .exec_iter(sql, params)
        .await
        .map_err(|_| PgGenerationError::SourceUnavailable)?;
    let mut output = SourceVec::default();
    while let Some(row) = cursor
        .next()
        .await
        .map_err(|_| PgGenerationError::SourceUnavailable)?
    {
        if output.as_slice().len() == cap || row.len() != widths.len() {
            return Err(PgGenerationError::CapabilityDrift);
        }
        let mut fields = work.vector(widths.len()).map_err(sql_error)?;
        for (i, width) in widths.iter().enumerate() {
            fields.push(field(row.as_ref(i), *width, bytes, work)?);
        }
        output.push(fields, work).map_err(sql_error)?;
    }
    cursor
        .drop_result()
        .await
        .map_err(|_| PgGenerationError::SourceUnavailable)?;
    Ok(output.into_vec())
}

fn field(
    value: Option<&Value>,
    width: usize,
    bytes: &mut usize,
    work: SourceWork<'_>,
) -> Result<Option<String>, PgGenerationError> {
    match value {
        Some(Value::Bytes(value)) if value.len() <= width => {
            let next = bytes
                .checked_add(value.len())
                .filter(|bytes| *bytes <= MAX_SCHEMA_BYTES)
                .ok_or(PgGenerationError::CapabilityDrift)?;
            work.charge(value.len()).map_err(sql_error)?;
            let text =
                std::str::from_utf8(value).map_err(|_| PgGenerationError::CapabilityDrift)?;
            let owned = work.string(text).map_err(sql_error)?;
            *bytes = next;
            Ok(Some(owned))
        }
        Some(Value::NULL) => Ok(None),
        _ => Err(PgGenerationError::CapabilityDrift),
    }
}

pub(crate) fn text(row: &[Option<String>], index: usize) -> Result<&str, PgGenerationError> {
    row.get(index)
        .and_then(Option::as_deref)
        .ok_or(PgGenerationError::CapabilityDrift)
}

const TABLE_SQL: &str = "SELECT LEFT(CAST(TABLE_NAME AS BINARY),64), \
    LEFT(CAST(TABLE_TYPE AS BINARY),33), LEFT(CAST(ENGINE AS BINARY),65), \
    LEFT(CAST(TABLE_COLLATION AS BINARY),65), LEFT(CAST(CREATE_OPTIONS AS BINARY),2049) \
    FROM information_schema.TABLES WHERE TABLE_SCHEMA=DATABASE() \
    AND BINARY TABLE_NAME=BINARY ? LIMIT 2";

const COLUMN_SQL: &str = "SELECT LEFT(CAST(COLUMN_NAME AS BINARY),257), \
    LEFT(CAST(COLUMN_TYPE AS BINARY),4097), LEFT(CAST(IS_NULLABLE AS BINARY),4), \
    LEFT(CAST(CHARACTER_SET_NAME AS BINARY),65), LEFT(CAST(COLLATION_NAME AS BINARY),65), \
    LEFT(CAST(EXTRA AS BINARY),257), LEFT(CAST(GENERATION_EXPRESSION AS BINARY),4097), \
    LEFT(CAST(COLUMN_DEFAULT AS BINARY),4097) \
    FROM information_schema.COLUMNS WHERE TABLE_SCHEMA=DATABASE() \
    AND BINARY TABLE_NAME=BINARY ? ORDER BY ORDINAL_POSITION LIMIT 1025";

pub(crate) async fn capture(
    conn: &mut Conn,
    names: &[String],
    work: SourceWork<'_>,
) -> Result<Schema, PgGenerationError> {
    if names.is_empty() || names.len() > 256 || names.iter().any(|name| !identifier(name)) {
        return Err(PgGenerationError::CapabilityDrift);
    }
    let mut bytes = 0usize;
    let session = super::session::capture(conn, work, &mut bytes).await?;
    let mut tables = work.vector(names.len()).map_err(sql_error)?;
    let mut facts = work.vector(names.len() * 2).map_err(sql_error)?;
    for name in names {
        let parameter = || -> Result<Params, PgGenerationError> {
            let mut values = work.vector(1).map_err(sql_error)?;
            values.push(Value::from(work.string(name).map_err(sql_error)?));
            Ok(Params::Positional(values))
        };
        let header = rows(
            conn,
            TABLE_SQL,
            parameter()?,
            1,
            &[63, 32, 64, 64, 2048],
            work,
            &mut bytes,
        )
        .await?;
        let row = header.first().ok_or(PgGenerationError::CapabilityDrift)?;
        if text(row, 0)? != name || text(row, 1)? != "BASE TABLE" || text(row, 2)? != "InnoDB" {
            return Err(PgGenerationError::CapabilityDrift);
        }
        let columns = rows(
            conn,
            COLUMN_SQL,
            parameter()?,
            MAX_COLUMNS,
            &[256, 4096, 3, 64, 64, 256, 4096, 4096],
            work,
            &mut bytes,
        )
        .await?;
        if columns.is_empty() {
            return Err(PgGenerationError::CapabilityDrift);
        }
        let mut table = TableSchema::new(work.string(name).map_err(sql_error)?);
        table.columns = work.vector(columns.len()).map_err(sql_error)?;
        for row in &columns {
            let nullable = text(row, 2)?;
            if !matches!(nullable, "YES" | "NO") {
                return Err(PgGenerationError::CapabilityDrift);
            }
            table.columns.push(Column::new(
                work.string(text(row, 0)?).map_err(sql_error)?,
                work.string(text(row, 1)?).map_err(sql_error)?,
                nullable == "NO",
            ));
        }
        // No keys/statistics are promoted into optimizer authority by this profile.
        tables.push(table);
        facts.push(header);
        facts.push(columns);
    }
    let comparison_work = bytes * 3 + names.len() * (MAX_COLUMNS * 16 + 4096);
    Ok(Schema {
        session,
        tables,
        facts,
        comparison_work,
    })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
