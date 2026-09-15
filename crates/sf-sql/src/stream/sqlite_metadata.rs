//! SQLite's declared-type walk stops at COLLATE even though COLLATE does not
//! change values or types. Recover explicitly aliased column metadata, never execute the
//! comparison-free statement or use it for result evaluation.
use std::ops::ControlFlow;

use sqlparser::ast::{Expr, Select, SelectItem, Value, VisitMut, VisitorMut};
use sqlparser::dialect::SQLiteDialect;
use sqlparser::parser::Parser;

use crate::error::{Error, Result};
use crate::source_work::SourceWork;

pub(crate) fn recover_collated_decltypes_controlled(
    conn: &rusqlite::Connection,
    sql: &str,
    declared: Vec<Option<String>>,
    work: SourceWork<'_>,
) -> Result<Vec<Option<String>>> {
    work.checkpoint()?;
    let mut collated = false;
    for window in sql.as_bytes().windows(7) {
        work.charge(7)?;
        if window.eq_ignore_ascii_case(b"collate") {
            collated = true;
            break;
        }
    }
    if !collated {
        return Ok(declared);
    }
    // Input admission only: parser-internal allocation/traversal is a separate
    // remaining obligation, not covered by the visitor/output charges below.
    work.charge(sql.len())?;
    let mut statements = Parser::parse_sql(&SQLiteDialect {}, sql)
        .map_err(|_| Error::Emit("SQLite collated metadata parse failed".into()))?;
    work.checkpoint()?;
    if statements.len() != 1 {
        return Err(Error::Emit("SQLite metadata requires one statement".into()));
    }
    let mut visitor = MetadataVisitor {
        changed: false,
        unnamed_expression: false,
        work,
    };
    if let ControlFlow::Break(error) = statements.visit(&mut visitor) {
        return Err(error);
    }
    if !visitor.changed {
        return Ok(declared);
    }
    if visitor.unnamed_expression {
        return Err(Error::Unsupported(
            "collated metadata recovery requires named expression outputs".into(),
        ));
    }
    let rendered = render_metadata(&statements[0], work)?;
    work.checkpoint()?;
    let recovered = conn.prepare(&rendered)?;
    work.checkpoint()?;
    if recovered.column_count() != declared.len() {
        return Err(Error::Emit("SQLite metadata projection mismatch".into()));
    }
    // A recursive CTE may synthesize TEXT instead of None after losing origin;
    // the transparent, name-preserving projection supplies the declaration.
    work.product(
        recovered.column_count(),
        std::mem::size_of::<rusqlite::Column<'_>>(),
    )?;
    let columns = recovered.columns();
    let mut output = work.vector(columns.len())?;
    for column in columns {
        work.charge(1)?;
        output.push(
            column
                .decl_type()
                .map(|decl| work.string(decl))
                .transpose()?,
        );
    }
    work.checkpoint()?;
    Ok(output)
}

struct MetadataVisitor<'a> {
    changed: bool,
    unnamed_expression: bool,
    work: SourceWork<'a>,
}
impl VisitorMut for MetadataVisitor<'_> {
    type Break = Error;
    fn pre_visit_expr(&mut self, _: &mut Expr) -> ControlFlow<Error> {
        match self.work.charge(1) {
            Ok(()) => ControlFlow::Continue(()),
            Err(error) => ControlFlow::Break(error),
        }
    }
    fn pre_visit_select(&mut self, select: &mut Select) -> ControlFlow<Error> {
        match self.prepare_select(select) {
            Ok(()) => ControlFlow::Continue(()),
            Err(error) => ControlFlow::Break(error),
        }
    }
}
impl MetadataVisitor<'_> {
    fn prepare_select(&mut self, select: &mut Select) -> Result<()> {
        self.work.charge(1)?;
        for item in &mut select.projection {
            self.work.charge(1)?;
            match item {
                SelectItem::ExprWithAlias { expr, .. } => {
                    // Only transparent wrappers around a column, never functions,
                    // casts, CASE, or a renamed implicit output.
                    let mut leaf = &*expr;
                    while let Expr::Nested(inner) | Expr::Collate { expr: inner, .. } = leaf {
                        self.work.charge(1)?;
                        leaf = inner;
                    }
                    if matches!(leaf, Expr::Identifier(_) | Expr::CompoundIdentifier(_)) {
                        strip_collation(expr, &mut self.changed, self.work)?;
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
        self.work.checkpoint()
    }
}
fn strip_collation(
    mut expression: &mut Expr,
    changed: &mut bool,
    work: SourceWork<'_>,
) -> Result<()> {
    loop {
        work.charge(1)?;
        match expression {
            Expr::Nested(inner) => expression = inner,
            Expr::Collate { .. } => {
                let Expr::Collate { expr, .. } =
                    std::mem::replace(expression, Expr::value(Value::Null))
                else {
                    unreachable!()
                };
                *expression = *expr;
                *changed = true;
            }
            _ => return work.checkpoint(),
        }
    }
}

/// Pay emitted bytes and capacity/movement before growing the output. The AST
/// formatter remains the semantic oracle; it cannot append after refusal.
fn render_metadata(statement: &impl std::fmt::Display, work: SourceWork<'_>) -> Result<String> {
    struct Output<'a> {
        bytes: crate::source_work::SourceVec<u8>,
        work: SourceWork<'a>,
        error: Option<Error>,
    }
    impl std::fmt::Write for Output<'_> {
        fn write_str(&mut self, value: &str) -> std::fmt::Result {
            if let Err(error) = self.bytes.extend_bytes(value.as_bytes(), self.work) {
                self.error = Some(error);
                return Err(std::fmt::Error);
            }
            Ok(())
        }
    }
    let mut output = Output {
        bytes: Default::default(),
        work,
        error: None,
    };
    if std::fmt::write(&mut output, format_args!("{statement}")).is_err() {
        return Err(output
            .error
            .unwrap_or_else(|| Error::Emit("SQLite metadata formatting failed".into())));
    }
    work.charge(output.bytes.as_slice().len())?; // UTF-8 validation scans bytes.
    let rendered = String::from_utf8(output.bytes.into_vec())
        .map_err(|_| Error::Emit("invalid metadata SQL UTF-8".into()))?;
    work.checkpoint()?;
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::sqlite_column_decltypes;
    use sf_core::query_control::{
        QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
    };

    #[test]
    fn controlled_recovery_has_exact_budget_and_sticky_stop_boundaries() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Stop {
            budget: QueryBudget,
            calls: AtomicUsize,
            at: usize,
            cause: QueryControlError,
        }
        impl QueryControl for Stop {
            fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
                self.budget.checkpoint()
            }
            fn consume(
                &self,
                kind: QueryCharge,
                units: u64,
            ) -> std::result::Result<(), QueryControlError> {
                self.budget.consume(kind, units)?;
                if self.calls.fetch_add(1, Ordering::Relaxed) + 1 == self.at {
                    self.budget.terminate(self.cause);
                }
                Ok(())
            }
            fn terminate(&self, reason: QueryControlError) -> QueryControlError {
                self.budget.terminate(reason)
            }
        }
        let budget =
            |units| QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX));
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(d DATE);").unwrap();
        let sql =
            "SELECT (d COLLATE BINARY) COLLATE NOCASE AS d, abs(-9223372036854775808) AS n FROM t";
        let run = |control: &dyn QueryControl| {
            recover_collated_decltypes_controlled(
                &conn,
                sql,
                vec![None, None],
                SourceWork::new(Some(control)),
            )
        };
        let expected = vec![Some("DATE".into()), None];
        let counted = Stop {
            budget: budget(u64::MAX),
            calls: AtomicUsize::new(0),
            at: usize::MAX,
            cause: QueryControlError::Cancelled,
        };
        assert_eq!(run(&counted).unwrap(), expected);
        let total = counted.budget.consumed(QueryCharge::SourceWork);
        assert_eq!(counted.budget.consumed(QueryCharge::CompilerWork), 0);
        assert_eq!(run(&budget(total)).unwrap(), expected);
        assert!(matches!(
            run(&budget(total - 1)),
            Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
        ));
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            for at in 1..=counted.calls.load(Ordering::Relaxed) {
                let stopped = Stop {
                    budget: budget(u64::MAX),
                    calls: AtomicUsize::new(0),
                    at,
                    cause,
                };
                assert!(
                    matches!(run(&stopped), Err(Error::QueryControl(actual)) if actual == cause),
                    "charge {at}"
                );
            }
        }
        assert_eq!(run(&budget(total)).unwrap(), expected);
    }

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
