//! SQLite's declared-type walk stops at COLLATE even though COLLATE does not
//! change values or types. Recover explicitly aliased column metadata, never execute the
//! comparison-free statement or use it for result evaluation.
use std::ops::ControlFlow;

use sqlparser::ast::{Expr, Select, SelectItem, Value, VisitMut, VisitorMut};
use sqlparser::dialect::SQLiteDialect;
use sqlparser::parser::Parser;

use crate::error::{Error, Result};

pub(super) fn recover_collated_decltypes(
    conn: &rusqlite::Connection,
    sql: &str,
    declared: Vec<Option<String>>,
) -> Result<Vec<Option<String>>> {
    if !sql
        .as_bytes()
        .windows(7)
        .any(|w| w.eq_ignore_ascii_case(b"collate"))
    {
        return Ok(declared);
    }
    let mut statements = Parser::parse_sql(&SQLiteDialect {}, sql)
        .map_err(|_| Error::Emit("SQLite collated metadata parse failed".into()))?;
    if statements.len() != 1 {
        return Err(Error::Emit("SQLite metadata requires one statement".into()));
    }
    let mut visitor = MetadataVisitor::default();
    let _: ControlFlow<()> = statements.visit(&mut visitor);
    if !visitor.changed {
        return Ok(declared);
    }
    if visitor.unnamed_expression {
        return Err(Error::Unsupported(
            "collated metadata recovery requires named expression outputs".into(),
        ));
    }
    let recovered = conn.prepare(&statements[0].to_string())?;
    if recovered.column_count() != declared.len() {
        return Err(Error::Emit("SQLite metadata projection mismatch".into()));
    }
    // A recursive CTE may synthesize TEXT instead of None after losing origin;
    // the transparent, name-preserving projection supplies the declaration.
    Ok(recovered
        .columns()
        .iter()
        .map(|c| c.decl_type().map(str::to_owned))
        .collect())
}

#[derive(Default)]
struct MetadataVisitor {
    changed: bool,
    unnamed_expression: bool,
}
impl VisitorMut for MetadataVisitor {
    type Break = ();
    fn pre_visit_select(&mut self, select: &mut Select) -> ControlFlow<()> {
        for item in &mut select.projection {
            match item {
                SelectItem::ExprWithAlias { expr, .. } => {
                    // Only transparent wrappers around a column, never functions,
                    // casts, CASE, or a renamed implicit output.
                    let mut leaf = &*expr;
                    while let Expr::Nested(inner) | Expr::Collate { expr: inner, .. } = leaf {
                        leaf = inner;
                    }
                    if matches!(leaf, Expr::Identifier(_) | Expr::CompoundIdentifier(_)) {
                        strip_collation(expr, &mut self.changed);
                    }
                }
                SelectItem::UnnamedExpr(expr)
                    if !matches!(expr, Expr::Identifier(_) | Expr::CompoundIdentifier(_)) =>
                {
                    self.unnamed_expression = true
                }
                _ => {}
            }
        }
        ControlFlow::Continue(())
    }
}
fn strip_collation(expression: &mut Expr, changed: &mut bool) {
    match expression {
        Expr::Nested(inner) => strip_collation(inner, changed),
        Expr::Collate { .. } => {
            let Expr::Collate { expr, .. } =
                std::mem::replace(expression, Expr::value(Value::Null))
            else {
                unreachable!()
            };
            *expression = *expr;
            *changed = true;
            strip_collation(expression, changed);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use crate::stream::sqlite_column_decltypes;

    #[test]
    fn transparent_collation_preserves_types_but_computation_does_not_invent_them() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(c CHARACTER(4), d DATE, b BLOB); INSERT INTO t VALUES('a','2026-09-08',X'80');").unwrap();
        let actual = sqlite_column_decltypes(&conn, "WITH x AS (SELECT (c COLLATE NOCASE) COLLATE BINARY AS c, d COLLATE BINARY AS d, b COLLATE BINARY AS b FROM t) SELECT c,d,b,c || ' COLLATE BINARY' AS joined, 'COLLATE BINARY' AS quoted, 1 AS n FROM x").unwrap();
        assert_eq!(
            actual,
            vec![
                Some("CHARACTER(4)".into()),
                Some("DATE".into()),
                Some("BLOB".into()),
                None,
                None,
                None
            ]
        );
    }

    #[test]
    fn metadata_recovery_never_evaluates_the_projection() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(d DATE); INSERT INTO t VALUES('2026-09-08');")
            .unwrap();
        // abs(i64::MIN) errors if evaluated. Preparing metadata must not run it.
        assert_eq!(
            sqlite_column_decltypes(
                &conn,
                "SELECT d COLLATE BINARY AS d, abs(-9223372036854775808) AS overflow FROM t"
            )
            .unwrap(),
            vec![Some("DATE".into()), None]
        );
        assert!(conn
            .query_row("SELECT abs(-9223372036854775808)", [], |row| row
                .get::<_, i64>(0))
            .is_err());
    }

    #[test]
    fn implicit_names_and_quoted_literal_resolution_are_never_rewritten() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(d DATE);").unwrap();
        for query in [
            "SELECT x.d FROM (SELECT d COLLATE BINARY FROM t) x",
            "SELECT \"d COLLATE BINARY\" FROM (SELECT d COLLATE BINARY FROM t)",
        ] {
            assert_eq!(sqlite_column_decltypes(&conn, query).unwrap(), vec![None]);
        }
        assert!(matches!(
            sqlite_column_decltypes(
                &conn,
                "SELECT d COLLATE BINARY AS fixed, x.\"1  +  2\" FROM t, (SELECT 1  +  2) x"
            ),
            Err(crate::Error::Unsupported(_))
        ));
    }
}
