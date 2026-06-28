//! Build the in-memory SQLite fixture for one W3C case from its `create.sql`, and
//! introspect its schema for the Direct Mapping auto-generated-R2RML path
//! (ADR-0005; introspection lives in `sf-sql`, ADR-0006).

use rusqlite::Connection;
use sf_sql::introspect::introspect_sqlite;
use sf_sql::TableSchema;

/// Load `create.sql` (DDL + INSERTs) into a fresh in-memory SQLite database.
/// A DDL the embedded SQLite cannot accept (a dialect feature) surfaces as an
/// error the caller turns into a documented skip.
pub fn load(create_sql: &str) -> Result<Connection, String> {
    let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
    conn.execute_batch(create_sql)
        .map_err(|e| format!("create.sql load failed: {e}"))?;
    Ok(conn)
}

/// Introspect every base table (schema order from `sqlite_master`), for Direct
/// Mapping. Foreign-key targets are resolved by name against this set.
pub fn introspect_all(conn: &Connection) -> Result<Vec<TableSchema>, String> {
    let names = table_names(conn)?;
    let mut schemas = Vec::with_capacity(names.len());
    for name in names {
        schemas.push(introspect_sqlite(conn, &name).map_err(|e| e.to_string())?);
    }
    Ok(schemas)
}

fn table_names(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut names = Vec::new();
    for r in rows {
        names.push(r.map_err(|e| e.to_string())?);
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_introspects() {
        let conn = load(
            "CREATE TABLE \"Student\" (\"ID\" INTEGER PRIMARY KEY, \"Name\" VARCHAR(50));
             INSERT INTO \"Student\" VALUES (10, 'Venus');",
        )
        .unwrap();
        let schemas = introspect_all(&conn).unwrap();
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].name, "Student");
        assert_eq!(schemas[0].primary_key, vec!["ID".to_owned()]);
    }

    #[test]
    fn rejects_unloadable_ddl() {
        assert!(load("CREATE TABLE bad (").is_err());
    }
}
