//! Bounded, transaction-local SQLite schema facts for authored serving.
//! This is not an ADR-0050 ObservedSchemaIdentity V1 profile.

use crate::source_work::{SourceVec, SourceWork};
use crate::{Column, Error, Result, TableSchema};
use rusqlite::Connection;

const MAX_OBJECTS: usize = 8192;
const MAX_TABLES: usize = 4096;
const MAX_COLUMNS: usize = 65536;
const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct SqliteGenerationSchema {
    objects: Vec<[String; 4]>,
    tables: Vec<TableSchema>,
    cookie: i64,
    journal: String,
}

impl SqliteGenerationSchema {
    pub fn tables(&self) -> &[TableSchema] {
        &self.tables
    }

    pub(super) fn observe(conn: &Connection, work: SourceWork<'_>) -> Result<Self> {
        work.checkpoint()?;
        let journal: String = conn.query_row("PRAGMA main.journal_mode", [], |row| row.get(0))?;
        if !matches!(journal.as_str(), "wal" | "delete") {
            return Err(invalid());
        }
        for pragma in ["PRAGMA read_uncommitted", "PRAGMA writable_schema"] {
            if conn.query_row(pragma, [], |r| r.get::<_, i64>(0))? != 0 {
                return Err(invalid());
            }
        }
        let mut databases = conn.prepare("PRAGMA database_list")?;
        let mut rows = databases.query([])?;
        let mut main = false;
        while let Some(row) = rows.next()? {
            work.charge(1)?;
            let name = row.get_ref(1)?.as_str().map_err(|_| invalid())?;
            if name == "main" {
                main = !row.get_ref(2)?.as_str().map_err(|_| invalid())?.is_empty();
            } else if name != "temp" {
                return Err(invalid());
            }
        }
        if !main
            || conn.query_row("SELECT count(*) FROM temp.sqlite_schema", [], |r| {
                r.get::<_, i64>(0)
            })? != 0
        {
            return Err(invalid());
        }
        // The first main-schema read establishes the transaction's snapshot.
        let cookie = conn.query_row("PRAGMA main.schema_version", [], |r| r.get(0))?;
        let mut statement =
            conn.prepare("SELECT type, name, tbl_name, coalesce(sql,'') FROM main.sqlite_schema")?;
        let mut rows = statement.query([])?;
        let mut objects = SourceVec::default();
        let mut tables = SourceVec::default();
        let mut bytes = 0usize;
        let mut columns = 0usize;
        while let Some(row) = rows.next()? {
            if objects.as_slice().len() == MAX_OBJECTS {
                return Err(invalid());
            }
            work.charge(std::mem::size_of::<[String; 4]>())?;
            let mut object: [String; 4] = std::array::from_fn(|_| String::new());
            for (i, value) in object.iter_mut().enumerate() {
                let text = row.get_ref(i)?.as_str().map_err(|_| invalid())?;
                bytes = bytes
                    .checked_add(text.len())
                    .filter(|n| *n <= MAX_BYTES)
                    .ok_or_else(invalid)?;
                *value = work.string(text)?;
            }
            if object[0] == "table" && !object[1].starts_with("sqlite_") {
                if tables.as_slice().len() == MAX_TABLES {
                    return Err(invalid());
                }
                // Virtual tables may execute external code outside this database snapshot.
                let mut flags = conn.prepare(
                    "SELECT type FROM pragma_table_list WHERE schema='main' AND name=?1",
                )?;
                let kind: String = flags.query_row([&object[1]], |r| r.get(0))?;
                if kind != "table" {
                    return Err(invalid());
                }
                work.charge(std::mem::size_of::<TableSchema>())?;
                let mut table = TableSchema::new(work.string(&object[1])?);
                let mut info = conn.prepare(
                    "SELECT name, type, \"notnull\" FROM pragma_table_xinfo(?1, 'main')",
                )?;
                let mut fields = info.query([&object[1]])?;
                let mut table_columns = SourceVec::default();
                while let Some(field) = fields.next()? {
                    columns += 1;
                    if columns > MAX_COLUMNS || table_columns.as_slice().len() == 4096 {
                        return Err(invalid());
                    }
                    let name = field.get_ref(0)?.as_str().map_err(|_| invalid())?;
                    let kind = field.get_ref(1)?.as_str().map_err(|_| invalid())?;
                    bytes = bytes
                        .checked_add(name.len())
                        .and_then(|n| n.checked_add(kind.len()))
                        .filter(|n| *n <= MAX_BYTES)
                        .ok_or_else(invalid)?;
                    work.charge(std::mem::size_of::<Column>())?;
                    table_columns.push(
                        Column::new(
                            work.string(name)?,
                            work.string(kind)?,
                            field.get::<_, i64>(2)? != 0,
                        ),
                        work,
                    )?;
                }
                // Authored serving quarantines constraint authority. Exact schema SQL
                // still binds every key/collation/trigger definition for drift checks.
                table.columns = table_columns.into_vec();
                tables.push(table, work)?;
            }
            objects.push(object, work)?;
        }
        let mut objects = objects.into_vec();
        // Sort only after enforcing bounds, without a source-side SQL sorter.
        work.product(bytes, objects.as_slice().len().max(1).ilog2() as usize + 1)?;
        objects.sort_unstable();
        let mut tables = tables.into_vec();
        tables.sort_by(|a, b| a.name.cmp(&b.name));
        work.checkpoint()?;
        Ok(Self {
            objects,
            tables,
            cookie,
            journal,
        })
    }
}

fn invalid() -> Error {
    Error::Introspection("SQLite generation profile or schema bound exceeded".into())
}
