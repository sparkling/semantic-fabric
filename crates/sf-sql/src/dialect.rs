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
    BigQueryDialect, DatabricksDialect, Dialect as SqlParserDialect, DuckDbDialect, GenericDialect,
    MsSqlDialect, MySqlDialect, OracleDialect, PostgreSqlDialect, RedshiftSqlDialect,
    SQLiteDialect, SnowflakeDialect, SparkSqlDialect,
};

use crate::error::{Error, Result};

/// A SQL dialect target for emission, parsing, and introspection (ADR-0006 / ADR-0024 M8).
///
/// The three original dialects (Postgres, Sqlite, MySql) are production-wired. Every
/// other variant has an associated [`Dialect::placeholder`], [`Dialect::quote_char`],
/// and [`Dialect::parser_dialect`] implementation, so SQL can be emitted for all of
/// them today. Live driver wiring is tiered:
///
/// * **Live-wired**: Postgres, Sqlite, MySql, DuckDb (embedded, requires `duckdb-backend` feature)
/// * **Wire-compatible**: Redshift (thin alias over PG wire)
/// * **Scaffolded**: all others (compile + return `Error::Unsupported` at runtime)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    // --- original three (production-wired) ------------------------------------
    /// PostgreSQL — primary production source; `$n` placeholders, `"`-quoted idents.
    Postgres,
    /// SQLite — embedded / CI source; `?` placeholders, `"`-quoted idents.
    Sqlite,
    /// MySQL — `?` placeholders, backtick-quoted idents.
    MySql,

    // --- PG-wire-compatible ---------------------------------------------------
    /// AWS Redshift — PG wire protocol; `$n` placeholders, `"`-quoted idents.
    Redshift,

    // --- native-driver dialects (ADR-0024 M8) ---------------------------------
    /// DuckDB — embedded OLAP; `$n` placeholders, `"`-quoted idents.
    DuckDb,
    /// Microsoft SQL Server — TDS protocol; `@Pn` placeholders, `"`-quoted idents.
    SqlServer,
    /// Oracle Database — OCI/JDBC; `:n` placeholders, `"`-quoted idents.
    Oracle,
    /// SAP HANA — JDBC/ODBC; `?` placeholders, `"`-quoted idents (PG-ish dialect).
    SapHana,
    /// MonetDB — MAPI; `?` placeholders, `"`-quoted idents (PG-ish dialect).
    MonetDb,

    // --- REST / HTTP (scaffolded) --------------------------------------------
    /// Snowflake — REST; `?` placeholders, `"`-quoted idents.
    Snowflake,
    /// Google BigQuery — REST; `?` placeholders, backtick-quoted idents.
    BigQuery,
    /// AWS Athena — Presto/Trino-based; `?` placeholders, `"`-quoted idents.
    Athena,
    /// Databricks — Spark SQL; `?` placeholders, backtick-quoted idents.
    Databricks,
    /// Trino — distributed query; `?` placeholders, `"`-quoted idents.
    Trino,
    /// PrestoDB — distributed query; `?` placeholders, `"`-quoted idents.
    PrestoDB,

    // --- ODBC / generic (scaffolded) -----------------------------------------
    /// IBM DB2 — ODBC/JDBC; `?` placeholders, `"`-quoted idents.
    Db2,
    /// H2 (embedded Java) — JDBC; `?` placeholders, `"`-quoted idents.
    H2,
    /// Apache Spark SQL — `?` placeholders, backtick-quoted idents.
    Spark,
    /// Dremio — SQL over REST; `?` placeholders, `"`-quoted idents.
    Dremio,
    /// Denodo — virtual DB; `?` placeholders, `"`-quoted idents.
    Denodo,
    /// JBoss Teiid — virtual DB; `?` placeholders, `"`-quoted idents.
    Teiid,
}

impl Dialect {
    /// The identifier quote character.
    pub fn quote_char(self) -> char {
        match self {
            // backtick group
            Dialect::MySql | Dialect::BigQuery | Dialect::Databricks | Dialect::Spark => '`',
            // double-quote for everything else
            _ => '"',
        }
    }

    /// Render `ident` as a quoted SQL identifier via the `sqlparser` AST node,
    /// which escapes any embedded quote character. Identifiers come from the
    /// trusted mapping IR (ADR-0010 R2), so this is hygiene, not authorization.
    pub fn quote_ident(self, ident: &str) -> String {
        sqlparser::ast::Ident::with_quote(self.quote_char(), ident).to_string()
    }

    /// Prepare-only metadata probe for a source's result-column names. `LIMIT 0` is
    /// uniform across all supported dialects: column metadata is available at prepare
    /// time regardless of LIMIT, and the statement never executes.
    pub fn probe_sql(&self, source: &sf_core::ir::LogicalSource) -> String {
        use sf_core::ir::LogicalSource;
        match source {
            LogicalSource::Table(t) => format!("SELECT * FROM {} LIMIT 0", self.quote_ident(t)),
            LogicalSource::Query(q) => q.clone(),
        }
    }

    /// The bound-parameter placeholder for the 1-based `index`-th parameter.
    ///
    /// | Style   | Dialects                                    |
    /// |---------|---------------------------------------------|
    /// | `$n`    | Postgres, Redshift, DuckDb                  |
    /// | `@Pn`   | SqlServer                                   |
    /// | `:n`    | Oracle                                      |
    /// | `?`     | everything else (positional)                |
    ///
    /// The placeholder is the *only* way a value enters generated SQL (ADR-0010 R1).
    pub fn placeholder(self, index: usize) -> String {
        match self {
            Dialect::Postgres | Dialect::Redshift | Dialect::DuckDb => format!("${index}"),
            Dialect::SqlServer => format!("@P{index}"),
            Dialect::Oracle => format!(":{index}"),
            _ => "?".to_owned(),
        }
    }

    /// The "no limit" sentinel to render as `LIMIT <sentinel>` immediately before
    /// a BARE `OFFSET n` (no LIMIT otherwise present) — `None` for a dialect whose
    /// grammar accepts a standalone `OFFSET` clause, needing no sentinel at all.
    ///
    /// | Sentinel                       | Dialects            | Why                                  |
    /// |---------------------------------|----------------------|---------------------------------------|
    /// | `Some("-1")`                    | Sqlite               | SQLite's own "`-1` = unbounded" idiom — confirmed live: a bare `OFFSET n` is a syntax error otherwise |
    /// | `Some("18446744073709551615")`  | MySql                | `2^64 - 1`, MySQL's own documented "no limit" idiom (MySQL has no negative-LIMIT convention) — confirmed live: a bare `OFFSET n` is a syntax error otherwise |
    /// | `None`                          | Postgres, Redshift   | ANSI-standard: a standalone `OFFSET` is valid SQL — confirmed live for Postgres; Redshift is PG-wire-compatible |
    /// | `None`                          | everything else      | not live-tested either way; keep the pre-existing bare-`OFFSET` behavior rather than guess |
    pub fn bare_offset_limit_sentinel(self) -> Option<&'static str> {
        match self {
            Dialect::Sqlite => Some("-1"),
            Dialect::MySql => Some("18446744073709551615"),
            _ => None,
        }
    }

    /// The matching `sqlparser` dialect, used to parse `rr:sqlQuery` (ADR-0015)
    /// and to validate/normalise emitted SQL ([`Dialect::emit_via_ast`]).
    pub fn parser_dialect(self) -> Box<dyn SqlParserDialect> {
        match self {
            Dialect::Postgres | Dialect::SapHana | Dialect::MonetDb => {
                Box::new(PostgreSqlDialect {})
            }
            Dialect::Sqlite => Box::new(SQLiteDialect {}),
            Dialect::MySql => Box::new(MySqlDialect {}),
            Dialect::Redshift => Box::new(RedshiftSqlDialect {}),
            Dialect::DuckDb => Box::new(DuckDbDialect {}),
            Dialect::SqlServer => Box::new(MsSqlDialect {}),
            Dialect::Oracle => Box::new(OracleDialect {}),
            Dialect::Snowflake => Box::new(SnowflakeDialect {}),
            Dialect::BigQuery => Box::new(BigQueryDialect {}),
            Dialect::Databricks => Box::new(DatabricksDialect {}),
            Dialect::Spark => Box::new(SparkSqlDialect {}),
            Dialect::Athena
            | Dialect::Trino
            | Dialect::PrestoDB
            | Dialect::Dremio
            | Dialect::Db2
            | Dialect::H2
            | Dialect::Denodo
            | Dialect::Teiid => Box::new(GenericDialect {}),
        }
    }

    /// Parse SQL under this dialect into a `sqlparser` AST.
    pub fn parse(self, sql: &str) -> Result<Vec<sqlparser::ast::Statement>> {
        sqlparser::parser::Parser::parse_sql(&*self.parser_dialect(), sql)
            .map_err(|e| Error::Emit(e.to_string()))
    }

    /// Emit `skeleton` as canonical SQL by round-tripping it through the
    /// `sqlparser` AST (ADR-0010 §A).
    pub fn emit_via_ast(self, skeleton: &str) -> Result<String> {
        let statements = self.parse(skeleton)?;
        Ok(statements
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; "))
    }

    /// Emit a parameterised semi-join **reducer probe** (ADR-0006 / ADR-0010 R1).
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
        assert_eq!(Dialect::DuckDb.quote_ident("emp"), "\"emp\"");
        assert_eq!(Dialect::BigQuery.quote_ident("emp"), "`emp`");
        assert_eq!(Dialect::SqlServer.quote_ident("emp"), "\"emp\"");
    }

    /// An embedded quote is escaped (doubled) by the `sqlparser` `Ident` node.
    #[test]
    fn quoting_escapes_embedded_quote() {
        assert_eq!(Dialect::Postgres.quote_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(Dialect::MySql.quote_ident("a`b"), "`a``b`");
    }

    #[test]
    fn placeholders_numbered_or_positional() {
        assert_eq!(Dialect::Postgres.placeholder(1), "$1");
        assert_eq!(Dialect::Postgres.placeholder(7), "$7");
        assert_eq!(Dialect::Redshift.placeholder(3), "$3");
        assert_eq!(Dialect::DuckDb.placeholder(2), "$2");
        assert_eq!(Dialect::SqlServer.placeholder(1), "@P1");
        assert_eq!(Dialect::Oracle.placeholder(2), ":2");
        assert_eq!(Dialect::Sqlite.placeholder(1), "?");
        assert_eq!(Dialect::MySql.placeholder(3), "?");
        assert_eq!(Dialect::Snowflake.placeholder(5), "?");
    }

    #[test]
    fn parses_rr_sql_query_under_dialect() {
        let stmts = Dialect::Postgres
            .parse("SELECT id, name FROM emp WHERE dept = $1")
            .unwrap();
        assert_eq!(stmts.len(), 1);
        assert!(Dialect::Sqlite.parse("SELEKT oops").is_err());
    }

    #[test]
    fn in_filter_sql_binds_values_and_quotes_idents_postgres() {
        let sql = Dialect::Postgres
            .in_filter_sql("emp", &["id", "name"], "dept_id", 3)
            .unwrap();
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

    #[test]
    fn probe_sql_limits_table_and_passes_query_through() {
        use sf_core::ir::LogicalSource;
        let table = LogicalSource::Table("emp".to_owned());
        assert_eq!(
            Dialect::Sqlite.probe_sql(&table),
            "SELECT * FROM \"emp\" LIMIT 0"
        );
        let query = LogicalSource::Query("SELECT 1 AS x".to_owned());
        assert_eq!(Dialect::Postgres.probe_sql(&query), "SELECT 1 AS x");
    }

    #[test]
    fn new_dialects_have_correct_quote_char() {
        // double-quote group
        for d in [
            Dialect::Redshift,
            Dialect::DuckDb,
            Dialect::SqlServer,
            Dialect::Oracle,
            Dialect::SapHana,
            Dialect::MonetDb,
            Dialect::Snowflake,
            Dialect::Athena,
            Dialect::Trino,
            Dialect::PrestoDB,
            Dialect::Db2,
            Dialect::H2,
            Dialect::Dremio,
            Dialect::Denodo,
            Dialect::Teiid,
        ] {
            assert_eq!(d.quote_char(), '"', "{d:?} should use double-quote");
        }
        // backtick group
        for d in [Dialect::BigQuery, Dialect::Databricks, Dialect::Spark] {
            assert_eq!(d.quote_char(), '`', "{d:?} should use backtick");
        }
    }
}
