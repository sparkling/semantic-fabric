//! SQL dialect targeting: identifier quoting, bound-parameter placeholders, and
//! SQL emission **through the `sqlparser` AST** (ADR-0006 *Parallelism & dialects*;
//! ADR-0010 §A; ADR-0015).
//!
//! Two correctness properties are load-bearing here:
//!
//! * **Injection-safety (ADR-0010 R1/R2).** Values that originate from the SPARQL
//!   are emitted as **bound-parameter placeholders only** — the builders in this
//!   module take a parameter *count*, never a value, so a value cannot reach the
//!   SQL text. Identifiers are rendered via [`sqlparser::ast::Ident`], whose
//!   `Display` escapes the quote character; identifiers come from the trusted
//!   mapping IR, never user input.
//! * **AST, not string assembly (ADR-0010 §A).** Every emitted statement is the
//!   `Display` of a `sqlparser` AST: [`Dialect::emit_via_ast`] parses the
//!   skeleton with the matching per-DBMS `sqlparser` dialect and re-renders the
//!   parsed tree, so malformed SQL is impossible to emit and the output is
//!   normalised by the same library that parses `rr:sqlQuery`.

use sqlparser::dialect::{
    Dialect as SqlParserDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect,
};

use crate::error::{Error, Result};

/// A SQL dialect target for emission, parsing, and introspection (ADR-0006).
///
/// PostgreSQL is the primary production source and SQLite the embedded / W3C-suite
/// CI source (both first-class). **MySQL is a stub**: emission works, but its
/// `DbTypeMap`, introspection, and driver wiring follow later (ADR-0015 — "MySQL
/// follows"). No columnar/OLAP dialect appears here (ADR-0006 *Confirmation*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    /// PostgreSQL — primary production source; `$n` placeholders, `"`-quoted idents.
    Postgres,
    /// SQLite — embedded / CI source; `?` placeholders, `"`-quoted idents.
    Sqlite,
    /// MySQL — **stub** (emission only); `?` placeholders, backtick-quoted idents.
    MySql,
}

impl Dialect {
    /// The identifier quote character: `"` for PostgreSQL/SQLite, `` ` `` for MySQL.
    pub fn quote_char(self) -> char {
        match self {
            Dialect::Postgres | Dialect::Sqlite => '"',
            Dialect::MySql => '`',
        }
    }

    /// Render `ident` as a quoted SQL identifier via the `sqlparser` AST node,
    /// which escapes any embedded quote character. Identifiers come from the
    /// trusted mapping IR (ADR-0010 R2), so this is hygiene, not authorization.
    pub fn quote_ident(self, ident: &str) -> String {
        sqlparser::ast::Ident::with_quote(self.quote_char(), ident).to_string()
    }

    /// The bound-parameter placeholder for the 1-based `index`-th parameter:
    /// `$index` for PostgreSQL (numbered), `?` for SQLite/MySQL (positional).
    ///
    /// The placeholder is the *only* way a value enters generated SQL (ADR-0010
    /// R1); the value itself is supplied at execution time, never inlined.
    pub fn placeholder(self, index: usize) -> String {
        match self {
            Dialect::Postgres => format!("${index}"),
            Dialect::Sqlite | Dialect::MySql => "?".to_owned(),
        }
    }

    /// The matching `sqlparser` dialect, used to parse `rr:sqlQuery` (ADR-0015)
    /// and to validate/normalise emitted SQL ([`Dialect::emit_via_ast`]).
    pub fn parser_dialect(self) -> Box<dyn SqlParserDialect> {
        match self {
            Dialect::Postgres => Box::new(PostgreSqlDialect {}),
            Dialect::Sqlite => Box::new(SQLiteDialect {}),
            Dialect::MySql => Box::new(MySqlDialect {}),
        }
    }

    /// Parse SQL under this dialect into a `sqlparser` AST (e.g. an `rr:sqlQuery`
    /// R2RML view; ADR-0015). `sqlparser` is *syntax only* — it contributes
    /// nothing to type semantics (those live in `sf-core`, ADR-0015).
    pub fn parse(self, sql: &str) -> Result<Vec<sqlparser::ast::Statement>> {
        sqlparser::parser::Parser::parse_sql(&*self.parser_dialect(), sql)
            .map_err(|e| Error::Emit(e.to_string()))
    }

    /// Emit `skeleton` as canonical SQL by round-tripping it through the
    /// `sqlparser` AST: parse with this dialect, then re-render. The returned
    /// string is the `Display` of a parsed AST (ADR-0010 §A), so it cannot be
    /// syntactically malformed.
    pub fn emit_via_ast(self, skeleton: &str) -> Result<String> {
        let statements = self.parse(skeleton)?;
        Ok(statements
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; "))
    }

    /// Emit a parameterised semi-join **reducer probe** — the `IN`-list reducer
    /// form chosen by the cost planner (`crate::cost::ReducerForm::InList`):
    ///
    /// ```sql
    /// SELECT <project…> FROM <table> WHERE <key> IN (<placeholder…>)
    /// ```
    ///
    /// The shipped key values are bound parameters (`n_keys` placeholders), never
    /// inlined (ADR-0010 R1); identifiers are quoted via [`Dialect::quote_ident`].
    /// The temp-table and Bloom reducer forms emit driver-specific DDL/probes and
    /// are deferred (ADR-0006 *Cross-source semi-join cost*).
    pub fn in_filter_sql(
        self,
        table: &str,
        project: &[&str],
        key: &str,
        n_keys: usize,
    ) -> Result<String> {
        if project.is_empty() {
            return Err(Error::Emit("in_filter_sql: empty projection".to_owned()));
        }
        if n_keys == 0 {
            return Err(Error::Emit(
                "in_filter_sql: zero key placeholders".to_owned(),
            ));
        }
        let cols = project
            .iter()
            .map(|c| self.quote_ident(c))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = (1..=n_keys)
            .map(|i| self.placeholder(i))
            .collect::<Vec<_>>()
            .join(", ");
        let skeleton = format!(
            "SELECT {cols} FROM {table} WHERE {key} IN ({placeholders})",
            table = self.quote_ident(table),
            key = self.quote_ident(key),
        );
        self.emit_via_ast(&skeleton)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_identifiers_per_dialect() {
        assert_eq!(Dialect::Postgres.quote_ident("emp"), "\"emp\"");
        assert_eq!(Dialect::Sqlite.quote_ident("emp"), "\"emp\"");
        assert_eq!(Dialect::MySql.quote_ident("emp"), "`emp`");
    }

    /// An embedded quote is escaped (doubled) by the `sqlparser` `Ident` node —
    /// identifier injection is not expressible (ADR-0010 R2 hygiene).
    #[test]
    fn quoting_escapes_embedded_quote() {
        assert_eq!(Dialect::Postgres.quote_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(Dialect::MySql.quote_ident("a`b"), "`a``b`");
    }

    #[test]
    fn placeholders_are_numbered_for_pg_positional_otherwise() {
        assert_eq!(Dialect::Postgres.placeholder(1), "$1");
        assert_eq!(Dialect::Postgres.placeholder(7), "$7");
        assert_eq!(Dialect::Sqlite.placeholder(1), "?");
        assert_eq!(Dialect::MySql.placeholder(3), "?");
    }

    #[test]
    fn parses_rr_sql_query_under_dialect() {
        let stmts = Dialect::Postgres
            .parse("SELECT id, name FROM emp WHERE dept = $1")
            .unwrap();
        assert_eq!(stmts.len(), 1);
        // Malformed SQL is a clean error, not a panic.
        assert!(Dialect::Sqlite.parse("SELEKT oops").is_err());
    }

    #[test]
    fn in_filter_sql_binds_values_and_quotes_idents_postgres() {
        let sql = Dialect::Postgres
            .in_filter_sql("emp", &["id", "name"], "dept_id", 3)
            .unwrap();
        // Numbered placeholders, double-quoted identifiers, no inlined values.
        assert!(
            sql.contains("$1") && sql.contains("$2") && sql.contains("$3"),
            "{sql}"
        );
        assert!(
            sql.contains("\"emp\"") && sql.contains("\"dept_id\""),
            "{sql}"
        );
        assert!(sql.to_uppercase().contains(" IN ("), "{sql}");
    }

    #[test]
    fn in_filter_sql_uses_positional_placeholders_sqlite() {
        let sql = Dialect::Sqlite
            .in_filter_sql("emp", &["id"], "dept_id", 2)
            .unwrap();
        assert_eq!(sql.matches('?').count(), 2, "{sql}");
        assert!(!sql.contains("$1"), "{sql}");
    }

    /// The emitted reducer is valid SQL (it round-trips through the AST) and is
    /// stable under re-parse — emission is AST-backed, not string assembly.
    #[test]
    fn emitted_reducer_reparses() {
        let sql = Dialect::Postgres
            .in_filter_sql("emp", &["id", "name"], "id", 2)
            .unwrap();
        assert!(Dialect::Postgres.parse(&sql).is_ok(), "{sql}");
    }

    #[test]
    fn in_filter_sql_rejects_degenerate_inputs() {
        assert!(Dialect::Postgres.in_filter_sql("t", &[], "k", 1).is_err());
        assert!(Dialect::Postgres
            .in_filter_sql("t", &["c"], "k", 0)
            .is_err());
    }
}
