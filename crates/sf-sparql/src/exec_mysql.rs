//! MySQL execution path (WS-D dialect breadth; ADR-0006 *Streaming & bounded memory*;
//! ADR-0015): run the emitted MySQL SQL over a **live** server through the
//! `mysql_async` async driver, reconstruct `oxrdf` terms through the **single**
//! sf-core term-gen path shared with the SQLite and PostgreSQL executors
//! ([`crate::exec`], ADR-0003 R3).
//!
//! MySQL's text protocol does not provide a server-side cursor equivalent to
//! PostgreSQL's `query_raw`; the driver fetches the result in wire packets and
//! delivers rows asynchronously. For virtualisation workloads (read-only, bounded
//! by mapping size × result pages) this satisfies ADR-0010 §C's bounded-memory
//! requirement at the packet level.
//!
//! XSD-type inference is conservative for v1: without prepared-statement wire types
//! (MySQL's text protocol does not carry them per-row), all values arrive as
//! `Option<String>` and are treated as plain literals (XsdTypeCode = None). The
//! mapping's `rr:datatype` / `rr:language` declarations still apply through
//! sf-core's normal term-gen chokepoint. Full type inference (via introspection →
//! column type → XsdTypeCode) is a planned ADR-0014 follow-up.
//!
//! Values are **bound parameters only** (ADR-0010 R1): the emitted `?` placeholders
//! are filled from `EmittedBranch::params`, never interpolated into SQL.
//!
//! Requires a live server; exercised by the integration suite (ADR-0012).

use std::collections::BTreeMap;
use std::future::Future;

use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Params, Row, Value};
use sf_core::ir::LogicalSource;
use sf_core::{GraphName, Quad, Term, Triple};
use sf_sql::Dialect;

use crate::emit::ColumnCatalog;
use crate::exec::{instantiate, order_cmp, reconstruct, RawRow, Solutions};
use crate::iq::Branch;
use crate::{Error, Plan, PlanForm, Result};

/// Build a [`ColumnCatalog`] by probing the result-column names of each source
/// in the plan's branches via a zero-row `LIMIT 0` probe query. MySQL column names
/// are case-insensitive by default; the canonical form returned by the driver is
/// used for `emit::resolve_col`.
async fn build_catalog_mysql(
    branches: &[Branch],
    conn: &mut Conn,
    dialect: Dialect,
) -> ColumnCatalog {
    let mut catalog = ColumnCatalog::default();
    let mut seen = std::collections::HashSet::new();
    for branch in branches {
        for (_, source) in branch.alias_sources() {
            let probe = match source {
                LogicalSource::Table(t) => {
                    format!("SELECT * FROM {} LIMIT 0", dialect.quote_ident(t))
                }
                LogicalSource::Query(q) => q.clone(),
            };
            if !seen.insert(probe.clone()) {
                continue;
            }
            if let Ok(stmt) = conn.prep(&probe).await {
                let names: Vec<String> = stmt
                    .columns()
                    .iter()
                    .map(|c| c.name_str().into_owned())
                    .collect();
                catalog.insert(source, names);
            }
        }
    }
    catalog
}

/// Convert a single MySQL [`Value`] cell to a raw lexical [`String`] (NULL → `None`).
/// All wire types are converted via their natural Rust representation and then
/// formatted as strings — the same principle as the PostgreSQL text-protocol path.
///
/// **Bytes:** only valid UTF-8 sequences are accepted; non-UTF-8 bytes (BLOB /
/// VARBINARY) yield `None` (unbound) rather than a silently-corrupted literal.
/// Callers that need binary BLOB values should add an `rr:datatype` declaration
/// and a custom term-gen hook (ADR-0014 follow-up).
///
/// **Date midnight:** `Value::Date` with all-zero time fields is produced by both
/// MySQL `DATE` columns and `DATETIME`/`TIMESTAMP` columns whose value is exactly
/// midnight. Without per-column wire-type metadata (available from `stmt.columns()`
/// but not yet threaded through the v1 executor) the two are indistinguishable.
/// The emitted lexical form `"YYYY-MM-DD"` is correct for `DATE` columns mapped with
/// `rr:datatype xsd:date`; for `DATETIME` midnight mapped with `rr:datatype xsd:dateTime`
/// this produces an invalid xsd:dateTime lexical form — a known v1 limitation tracked
/// under ADR-0014.
fn mysql_value_to_string(v: Value) -> Option<String> {
    use mysql_async::Value::*;
    match v {
        NULL => None,
        // Reject non-UTF-8 bytes rather than silently corrupting BLOB data.
        Bytes(b) => String::from_utf8(b).ok(),
        Int(i) => Some(i.to_string()),
        UInt(u) => Some(u.to_string()),
        Float(f) => Some(f.to_string()),
        Double(d) => Some(d.to_string()),
        Date(y, mo, d, h, mi, s, us) => {
            if h == 0 && mi == 0 && s == 0 && us == 0 {
                Some(format!("{y:04}-{mo:02}-{d:02}"))
            } else if us == 0 {
                Some(format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}"))
            } else {
                Some(format!(
                    "{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{us:06}"
                ))
            }
        }
        Time(neg, days, h, mi, s, us) => {
            let sign = if neg { "-" } else { "" };
            let total_h = days * 24 + u32::from(h);
            if us == 0 {
                Some(format!("{sign}{total_h:02}:{mi:02}:{s:02}"))
            } else {
                Some(format!("{sign}{total_h:02}:{mi:02}:{s:02}.{us:06}"))
            }
        }
    }
}

/// Fetch all rows for `sql` from a MySQL connection, returning each as a
/// `Vec<Option<String>>` (NULL → None). Values are always bound parameters —
/// never interpolated (ADR-0010 R1). Uses the binary protocol (prepared statements)
/// for parameter safety; values are decoded via [`mysql_value_to_string`].
async fn fetch_rows(
    conn: &mut Conn,
    sql: &str,
    params: Vec<Value>,
) -> Result<Vec<Vec<Option<String>>>> {
    let stmt = conn
        .prep(sql)
        .await
        .map_err(|e| Error::Sql(e.to_string()))?;
    let result_rows: Vec<Row> = conn
        .exec(stmt, Params::Positional(params))
        .await
        .map_err(|e| Error::Sql(e.to_string()))?;
    let mut out = Vec::with_capacity(result_rows.len());
    for mut row in result_rows {
        let ncols = row.len();
        let mut vals = Vec::with_capacity(ncols);
        for i in 0..ncols {
            let v: Value = row.take(i).unwrap_or(Value::NULL);
            vals.push(mysql_value_to_string(v));
        }
        out.push(vals);
    }
    Ok(out)
}

/// Iterate every WHERE solution across all branches over a live MySQL connection.
/// Results are buffered per-branch (MySQL text protocol does not support server-side
/// cursors). ORDER BY is applied here for type-aware SPARQL ordering; OFFSET/LIMIT
/// follow. Mirrors the PostgreSQL executor ([`crate::exec_pg`]).
///
/// Note on memory: MySQL's text protocol fetches results per-packet from the server
/// and delivers them to the driver buffer. For virtualisation workloads (read-only,
/// bounded by mapping × filter selectivity) this is bounded in practice. A proper
/// server-side cursor (MySQL 8.0+ `CURSOR FOR`) is a planned ADR-0014 follow-up.
async fn for_each_solution_mysql<F, Fut>(plan: &Plan, conn: &mut Conn, mut sink: F) -> Result<()>
where
    F: FnMut(&Branch, &BTreeMap<String, Term>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let branches = plan.prepared_branches();
    let catalog = build_catalog_mysql(&branches, conn, plan.dialect).await;
    let multi = branches.len() > 1;
    let distinct_vars: Option<Vec<String>> = match (plan.distinct && multi, &plan.form) {
        (true, PlanForm::Select { vars }) => Some(vars.clone()),
        _ => None,
    };
    let mut seen_tuples: std::collections::HashSet<Vec<Option<Term>>> =
        std::collections::HashSet::new();
    let mut seen = 0usize;
    let mut emitted = 0usize;
    let ordered = !plan.order.is_empty();
    let mut buffer: Vec<(usize, BTreeMap<String, Term>)> = Vec::new();
    for (bi, branch) in branches.iter().enumerate() {
        let e = crate::emit::emit_branch_with(branch, plan.dialect, &catalog)?;
        let params: Vec<Value> = e.params.iter().map(|s| Value::from(s.as_str())).collect();
        let rows = fetch_rows(conn, &e.sql, params).await?;
        for row_vals in rows {
            // v1: treat all values as plain literals (text protocol, no per-row types).
            let codes: Vec<Option<sf_core::datatype::XsdTypeCode>> = vec![None; row_vals.len()];
            let raw = RawRow {
                schema: &e.projection,
                values: &row_vals,
                codes: &codes,
            };
            let bindings = reconstruct(branch, &raw)?;
            if multi {
                if let Some(vars) = &distinct_vars {
                    let key: Vec<Option<Term>> =
                        vars.iter().map(|v| bindings.get(v).cloned()).collect();
                    if !seen_tuples.insert(key) {
                        continue;
                    }
                }
            }
            if ordered {
                buffer.push((bi, bindings));
                continue;
            }
            if multi {
                if seen < plan.offset {
                    seen += 1;
                    continue;
                }
                if let Some(limit) = plan.limit {
                    if emitted >= limit {
                        break;
                    }
                }
            }
            emitted += 1;
            sink(branch, &bindings).await?;
        }
    }
    // ORDER BY: stable-sort by type-aware key, then OFFSET/LIMIT.
    if ordered {
        buffer.sort_by(|(_, a), (_, b)| order_cmp(&plan.order, a, b));
        let take = plan.limit.unwrap_or(usize::MAX);
        for (bi, bindings) in buffer.iter().skip(plan.offset).take(take) {
            sink(&branches[*bi], bindings).await?;
        }
    }
    Ok(())
}

/// Collect a CONSTRUCT's triples over a live MySQL connection.
pub async fn construct_triples_mysql(plan: &Plan, conn: &mut Conn) -> Result<Vec<Triple>> {
    let template = match &plan.form {
        PlanForm::Construct { template } => template.clone(),
        _ => {
            return Err(Error::Unsupported(
                "construct() requires a CONSTRUCT plan".to_owned(),
            ))
        }
    };
    let mut out = Vec::new();
    for_each_solution_mysql(plan, conn, |_branch, bindings| {
        for tp in &template {
            if let Some(triple) = instantiate(tp, bindings) {
                out.push(triple);
            }
        }
        std::future::ready(Ok(()))
    })
    .await?;
    Ok(out)
}

/// Execute a SELECT over a live MySQL connection, collecting solutions.
pub async fn select_mysql(plan: &Plan, conn: &mut Conn) -> Result<Solutions> {
    let vars = match &plan.form {
        PlanForm::Select { vars } => vars.clone(),
        _ => {
            return Err(Error::Unsupported(
                "select() requires a SELECT plan".to_owned(),
            ))
        }
    };
    let mut rows = Vec::new();
    for_each_solution_mysql(plan, conn, |_branch, bindings| {
        rows.push(vars.iter().map(|v| bindings.get(v).cloned()).collect());
        std::future::ready(Ok(()))
    })
    .await?;
    Ok(Solutions { vars, rows })
}

/// Execute an ASK over a live MySQL connection — true iff at least one solution exists.
pub async fn ask_mysql(plan: &Plan, conn: &mut Conn) -> Result<bool> {
    let mut any = false;
    for_each_solution_mysql(plan, conn, |_b, _s| {
        any = true;
        std::future::ready(Ok(()))
    })
    .await?;
    Ok(any)
}

/// Collect the mapping-IR **quad** dump over a live MySQL connection.
pub async fn dump_quads_mysql(
    maps: &[sf_core::ir::TriplesMap],
    conn: &mut Conn,
    dialect: Dialect,
) -> Result<Vec<Quad>> {
    use crate::dump::{VAR_G, VAR_O, VAR_P, VAR_S};

    let plan = Plan {
        branches: crate::dump::build_branches(maps),
        form: PlanForm::Select { vars: Vec::new() },
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        rust_group: None,
        dialect,
    };
    let mut out = Vec::new();
    for_each_solution_mysql(&plan, conn, |branch, bindings| {
        if let (Some(s), Some(p), Some(o)) = (
            bindings.get(VAR_S),
            bindings.get(VAR_P),
            bindings.get(VAR_O),
        ) {
            let graph = if branch.bindings.contains_key(VAR_G) {
                match bindings.get(VAR_G) {
                    Some(Term::NamedNode(n)) => Some(GraphName::NamedNode(n.clone())),
                    _ => None,
                }
            } else {
                Some(GraphName::DefaultGraph)
            };
            if let Some(graph) = graph {
                if let Ok(triple) = Triple::from_terms(s.clone(), p.clone(), o.clone()) {
                    out.push(triple.in_graph(graph));
                }
            }
        }
        std::future::ready(Ok(()))
    })
    .await?;
    Ok(out)
}
