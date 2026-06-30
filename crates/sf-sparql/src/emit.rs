//! Emit — render an optimized [`Branch`] to dialect SQL (ADR-0007 step 6).
//!
//! Two invariants from the substrate (ADR-0010 §A / R1, ADR-0006):
//!
//! * **Values are bound parameters only.** Every constant from the query becomes
//!   a placeholder (`?` / `$n`) and its lexical value is returned in
//!   [`EmittedBranch::params`]; nothing is interpolated into the SQL text.
//! * **AST, not string assembly.** The rendered skeleton is round-tripped through
//!   the `sqlparser` AST via [`sf_sql::Dialect::emit_via_ast`], so the emitted
//!   statement is the `Display` of a parsed tree.
//!
//! Term construction is **not** here — the SELECT projects raw key columns
//! (ADR-0007 lifting); RDF terms are built during reconstruction ([`crate::exec`]).
//!
//! **SQL identifier case-folding (SQL:2008 §5.4 / per-dialect).** An `rr:column` /
//! `rr:sqlQuery` output-column value carries the *author's* identifier. A regular
//! (unquoted) identifier is case-folded by the DBMS — PostgreSQL folds to
//! lowercase — so the mapping's `"StudentId"` must bind to the column the source
//! actually exposes (`studentid`). Each emitted column reference is therefore
//! resolved against the source's *introspected* column names ([`ColumnCatalog`]):
//! an **exact** match wins (a delimited, case-exact identifier), else a **single
//! ASCII-case-insensitive** match (the regular-identifier folding the W3C suite
//! and every dialect honour), else the identifier is emitted as written (the
//! source has no such column — the error surfaces unchanged). Reconstruction is
//! untouched: it reads result columns by position and matches the *raw* IR column
//! strings, so only the rendered SQL text changes here.

use std::collections::HashMap;

use sf_core::ir::{LogicalSource, TermMap};
use sf_sql::Dialect;

use crate::iq::{
    AggCol, AggKind, Aggregation, Branch, ColRef, HopExpr, OrderKey, PathClosure, PathKind,
    SqlCond, StrMatchOp, TermDef,
};
use crate::{Error, Result};

/// The introspected (actual) column names of each logical source, so a mapping's
/// regular-identifier column references resolve to the column the live DBMS truly
/// exposes after its identifier folding (SQL:2008; PostgreSQL lowercases unquoted
/// names). Built by the executor from the connection ([`crate::exec`] /
/// [`crate::exec_pg`]); an empty catalog disables resolution (every reference is
/// emitted as written — the dialect-neutral [`emit_branch`] path).
#[derive(Debug, Default)]
pub struct ColumnCatalog {
    by_source: HashMap<String, Vec<String>>,
}

impl ColumnCatalog {
    /// Record `source`'s actual result-column names (in any order).
    pub fn insert(&mut self, source: &LogicalSource, columns: Vec<String>) {
        self.by_source.insert(source_key(source), columns);
    }

    fn columns(&self, source: &LogicalSource) -> Option<&[String]> {
        self.by_source.get(&source_key(source)).map(Vec::as_slice)
    }
}

/// A collision-free key for a logical source (a table name can never equal an SQL
/// query, but the kind prefix makes that explicit).
fn source_key(source: &LogicalSource) -> String {
    match source {
        LogicalSource::Table(t) => format!("t:{t}"),
        LogicalSource::Query(q) => format!("q:{q}"),
    }
}

/// Resolve a column identifier against a source's actual columns: an exact match
/// (a case-exact / delimited identifier) wins; else a unique ASCII-case-insensitive
/// match (a regular identifier folded by the DBMS); else the identifier as written
/// (no such column — the source surfaces the error). `actual = None` ⇒ unknown
/// source, emit as written.
fn resolve_col<'a>(raw: &'a str, actual: Option<&'a [String]>) -> &'a str {
    let Some(cols) = actual else { return raw };
    if cols.iter().any(|c| c == raw) {
        return raw;
    }
    let mut folded = cols.iter().filter(|c| c.eq_ignore_ascii_case(raw));
    match (folded.next(), folded.next()) {
        (Some(c), None) => c.as_str(),
        _ => raw,
    }
}

/// The actual columns of every scan alias in `b`, keyed by alias, for resolution.
/// For SubPlan-join aliases, the "actual columns" are the projected variable names
/// from the nested Plan's `PlanForm::Select { vars }` (the names the derived table
/// exposes). SubPlan aliases are NOT in `alias_sources()` (they have no catalog
/// entry), so they are wired up here directly.
fn branch_actuals(b: &Branch, catalog: &ColumnCatalog) -> HashMap<usize, Vec<String>> {
    let mut out = HashMap::new();
    for (alias, source) in b.alias_sources() {
        if let Some(cols) = catalog.columns(source) {
            out.insert(alias, cols.to_vec());
        }
    }
    // SubPlan derived-table aliases: their columns are the positional names the
    // inner `emit_branch` assigns (`c0`, `c1`, …), NOT the SPARQL variable names.
    // The outer branch's bindings use `ColRef(sp_alias, "c{i}")` after remapping.
    for sp in &b.subplan_joins {
        if let crate::PlanForm::Select { vars } = &sp.plan.form {
            let positional: Vec<String> = (0..vars.len()).map(|i| format!("c{i}")).collect();
            out.insert(sp.alias, positional);
        }
    }
    out
}

/// A branch rendered to one parameterised SQL `SELECT`.
pub struct EmittedBranch {
    pub sql: String,
    /// The result-set schema: column `i` is `projection[i]` (positional — the
    /// reconstruction reads by position, not by the cosmetic `AS c{i}` label).
    pub projection: Vec<ColRef>,
    /// Bound parameter values, in placeholder order.
    pub params: Vec<String>,
}

/// Render one branch with no column-name resolution (the dialect-neutral path,
/// used where a live catalog is unavailable — every identifier emitted as written).
pub fn emit_branch(b: &Branch, dialect: Dialect) -> Result<EmittedBranch> {
    emit_branch_with(b, dialect, &ColumnCatalog::default())
}

/// Render one branch, resolving each column reference against `catalog` so a
/// mapping's regular identifiers bind to the columns the live source exposes after
/// its identifier folding (see the module docs).
pub fn emit_branch_with(
    b: &Branch,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> Result<EmittedBranch> {
    let actuals = branch_actuals(b, catalog);
    if let Some(pc) = &b.path {
        return emit_path_branch(b, pc, dialect, catalog);
    }
    if let Some(agg) = &b.agg {
        return emit_agg_branch(b, agg, dialect, &actuals);
    }
    let projection = b.projection();
    let mut params = Vec::new();
    let mut pidx = 0usize;

    // FROM (+ LEFT JOIN ON params) MUST render before WHERE so positional
    // placeholders bind in text order. A core-less branch with no SubPlan joins
    // (an inline `VALUES` constant row — all `Const` bindings) renders as a
    // one-row `SELECT <const exprs>` with no FROM. A core-less branch WITH
    // SubPlan joins (ADR-0023 M5 Wave 2: SubPlan anchor) uses the first SubPlan
    // as the FROM anchor.
    let from = if b.core.is_empty() && b.subplan_joins.is_empty() {
        None
    } else {
        Some(render_from(b, dialect, &actuals, &mut params, &mut pidx)?)
    };
    let where_sql = render_where(&b.where_conds, dialect, &actuals, &mut params, &mut pidx);

    let select_list = if projection.is_empty() {
        "1 AS c0".to_owned()
    } else {
        projection
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{} AS c{i}", colref(c, dialect, &actuals)))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let distinct = if b.distinct { "DISTINCT " } else { "" };
    let mut skeleton = match from {
        Some(f) => format!("SELECT {distinct}{select_list} FROM {f}"),
        None => format!("SELECT {distinct}{select_list}"),
    };
    if let Some(w) = where_sql {
        skeleton.push_str(" WHERE ");
        skeleton.push_str(&w);
    }
    // ORDER BY precedes LIMIT/OFFSET (SPARQL §15: order, then slice).
    if let Some(order) = render_order(&b.order, b, dialect, &actuals)? {
        skeleton.push_str(&order);
    }
    if let Some(limit) = b.limit {
        skeleton.push_str(&format!(" LIMIT {limit}"));
    }
    if b.offset > 0 {
        skeleton.push_str(&format!(" OFFSET {}", b.offset));
    }

    let sql = dialect
        .emit_via_ast(&skeleton)
        .map_err(|e| Error::Sql(e.to_string()))?;
    Ok(EmittedBranch {
        sql,
        projection,
        params,
    })
}

/// Render an `ORDER BY` clause that pins the SPARQL value-space order, or `None`
/// when there are no keys. Each key is a bound variable; its `rr:column` term map
/// lowers to a raw column whose SQL order equals the RDF lexical/value order (a
/// homogeneous literal column, or an IRI whose value *is* the column). NULL
/// placement is **always explicit** — `NULLS FIRST` for ASC, `NULLS LAST` for DESC
/// — never the dialect default (PostgreSQL: NULLS LAST; SQLite: NULLS FIRST), so an
/// unbound key sorts first (ASC) / last (DESC) on every engine. A key bound to a
/// non-`rr:column` term (a constructed template IRI, COALESCE, …) cannot be ordered
/// soundly in SQL (the constructed string ≠ the column order) → honest 501.
fn render_order(
    order: &[OrderKey],
    b: &Branch,
    dialect: Dialect,
    actuals: &HashMap<usize, Vec<String>>,
) -> Result<Option<String>> {
    if order.is_empty() {
        return Ok(None);
    }
    let mut terms = Vec::with_capacity(order.len());
    for key in order {
        let def = b.bindings.get(&key.var).ok_or_else(|| {
            Error::Unsupported(format!(
                "ORDER BY ?{} is not a bound variable → 501",
                key.var
            ))
        })?;
        let col = order_column(def).ok_or_else(|| {
            Error::Unsupported(format!(
                "ORDER BY ?{} is not an rr:column term — a constructed/derived sort \
                 key cannot be ordered soundly in SQL → 501",
                key.var
            ))
        })?;
        let dir = if key.descending {
            "DESC NULLS LAST"
        } else {
            "ASC NULLS FIRST"
        };
        terms.push(format!("{} {dir}", colref(&col, dialect, actuals)));
    }
    Ok(Some(format!(" ORDER BY {}", terms.join(", "))))
}

/// The single raw column an ORDER BY key lowers to, iff the key is a bound
/// `rr:column` term map (the SQL-order-sound case). A template / COALESCE / CONCAT /
/// constant has no such column → `None` (the caller defers to 501).
fn order_column(def: &TermDef) -> Option<ColRef> {
    match def {
        TermDef::Derived {
            term_map: TermMap::Column(c, _),
            alias,
        } => Some(ColRef::new(*alias, c.clone())),
        _ => None,
    }
}

/// Render a property-path closure branch to a (possibly `WITH RECURSIVE`) CTE
/// (ADR-0007 *recursive paths compile to source-dialect recursive CTEs*).
///
/// The hop relation ([`HopExpr`]) compiles to a subquery yielding the canonical
/// **raw key columns** `sf_s` / `sf_o` (term-gen lifting; see [`hop_sql`]): a bare
/// predicate is a base scan, and `^p`/`p/q`/`p|q`/`!p` are nested subqueries over
/// the same keys. A second relation reduces it to the distinct reachable node
/// pairs the outer projection reads as `t{alias}` — SPARQL paths are set-semantics
/// over node **pairs**, so this `SELECT DISTINCT sf_s, sf_o` is what keeps
/// `SELECT ?s WHERE {?s P ?o}` a correct bag of `?s` even when `?o` is dropped.
///
/// Per [`PathKind`]:
/// * `One` (`^p`, `p/q`, `p|q`) — just the distinct hop pairs, no recursion; `!p`
///   (NPS) is the bag exception — its `UNION ALL` hop is kept un-`DISTINCT`ed.
/// * `ZeroOrOne` (`p?`) — the hop ∪ the reflexive `(x, x)` pairs over the active
///   graph's nodes (only over a single-predicate bare leaf, `unfold`-enforced).
/// * `OneOrMore` (`P+`) — a `WITH RECURSIVE` closure: the recursive member keeps an
///   `sf_d` depth counter, its body a `UNION` deduped on `(sf_s, sf_o, sf_d)` so a
///   pair revisited around a cycle is NOT collapsed there — `sf_d < max_depth` is
///   the *sole* recursion terminator (ADR-0010; SQLite has no `CYCLE` clause — the
///   later MB-4 wave). The outer `SELECT DISTINCT` collapses the depth dimension.
/// * `ZeroOrMore` (`P*`) — `OneOrMore` plus the reflexive `(x, x)` pairs at depth 0.
///
/// The depth ints and the bound are engine constants (not query data), so — like
/// `LIMIT` and the `ESCAPE '\'` char — they are part of the trusted skeleton, not
/// bound params. RDF terms are still built only at the outer projection
/// ([`crate::exec`]). This wave targets the SQLite dialect; the PostgreSQL
/// `CYCLE`/PG14 variant is the later MB-4 wave.
fn emit_path_branch(
    b: &Branch,
    pc: &PathClosure,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> Result<EmittedBranch> {
    // ORDER BY over a path result is handled at the exec layer (plan.order →
    // Rust-level order_cmp) — Branch.order is always empty here, so no guard needed.
    let projection = b.projection();
    let mut params = Vec::new();
    let mut pidx = 0usize;

    // `cte` is the distinct-pairs relation the outer projection reads (colref binds
    // `t{alias}`). Its columns are the canonical `sf_s` / `sf_o` keys, never base
    // columns, so the outer projection / WHERE resolve against an empty catalog.
    let cte = format!("t{}", pc.alias);
    let outer_actuals: HashMap<usize, Vec<String>> = HashMap::new();
    let hop = hop_sql(&pc.hop, dialect, catalog);
    let (sf_s, sf_o, sf_d) = (
        dialect.quote_ident("sf_s"),
        dialect.quote_ident("sf_o"),
        dialect.quote_ident("sf_d"),
    );

    // The `WITH …` prelude that defines `t{alias}(sf_s, sf_o)` as the distinct
    // reachable node pairs, per path kind.
    let with = match pc.kind {
        PathKind::One => {
            // A negated property set is bag-valued (the hop already emits its own
            // per-predicate `DISTINCT` over a `UNION ALL`); every other length-one
            // composite (`^p`/`p/q`/`p|q`) is set-valued. Omitting / keeping the
            // outer `DISTINCT` accordingly is what matches the oracle's bag vs set.
            let one_distinct = if matches!(pc.hop, HopExpr::Nps(_)) {
                ""
            } else {
                "DISTINCT "
            };
            format!(
                "WITH {cte}({sf_s}, {sf_o}) AS \
                 (SELECT {one_distinct}{sf_s}, {sf_o} FROM ({hop}) hx)"
            )
        }
        PathKind::ZeroOrOne => {
            let refl = reflexive_sql(&pc.hop, dialect, catalog, None)?;
            format!(
                "WITH {cte}({sf_s}, {sf_o}) AS (SELECT DISTINCT {sf_s}, {sf_o} FROM \
                 (SELECT {sf_s}, {sf_o} FROM ({hop}) hx UNION {refl}) z)"
            )
        }
        PathKind::OneOrMore | PathKind::ZeroOrMore => {
            let cte_raw = format!("t{}r", pc.alias);
            let one_hop = format!("SELECT {sf_s}, {sf_o}, 1 AS {sf_d} FROM ({hop}) hx");
            let anchor = if matches!(pc.kind, PathKind::ZeroOrMore) {
                let refl = reflexive_sql(&pc.hop, dialect, catalog, Some(0))?;
                format!("{one_hop} UNION {refl}")
            } else {
                one_hop
            };
            let recursive = format!(
                "SELECT c.{sf_s} AS {sf_s}, h.{sf_o} AS {sf_o}, c.{sf_d} + 1 AS {sf_d} \
                 FROM {cte_raw} c JOIN ({hop}) h ON c.{sf_o} = h.{sf_s} WHERE c.{sf_d} < {max}",
                max = pc.max_depth
            );
            format!(
                "WITH RECURSIVE {cte_raw}({sf_s}, {sf_o}, {sf_d}) AS ({anchor} UNION {recursive}), \
                 {cte}({sf_s}, {sf_o}) AS (SELECT DISTINCT {sf_s}, {sf_o} FROM {cte_raw})"
            )
        }
    };

    let select_list = projection
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{} AS c{i}", colref(c, dialect, &outer_actuals)))
        .collect::<Vec<_>>()
        .join(", ");

    let distinct = if b.distinct { "DISTINCT " } else { "" };
    let mut skeleton = format!("{with} SELECT {distinct}{select_list} FROM {cte}");
    if let Some(w) = render_where(
        &b.where_conds,
        dialect,
        &outer_actuals,
        &mut params,
        &mut pidx,
    ) {
        skeleton.push_str(" WHERE ");
        skeleton.push_str(&w);
    }
    if let Some(limit) = b.limit {
        skeleton.push_str(&format!(" LIMIT {limit}"));
    }
    if b.offset > 0 {
        skeleton.push_str(&format!(" OFFSET {}", b.offset));
    }

    let sql = dialect
        .emit_via_ast(&skeleton)
        .map_err(|e| Error::Sql(e.to_string()))?;
    Ok(EmittedBranch {
        sql,
        projection,
        params,
    })
}

/// Render a GROUP BY + aggregates branch (SPARQL §11) to one parameterised SQL
/// `SELECT … GROUP BY …`. The grouping keys are their **raw key columns** (term
/// construction is rebuilt at reconstruction, ADR-0007), grouped and projected;
/// each aggregate is `COUNT`/`SUM`/`AVG`/`MIN`/`MAX(…)` over its single raw column
/// (or `COUNT(*)`). Implicit grouping (no keys) emits no `GROUP BY`, yielding one
/// row over all inner rows — and one row even when the inner is empty (`COUNT(*)`
/// = 0). The projection is built in lockstep with the SELECT list so reconstruction
/// reads each key/aggregate by position. The aggregate result columns are synthetic
/// (computed in SQL), so their `§10` type is set explicitly at reconstruction (see
/// [`TermDef::Agg`]), never read from a base column.
fn emit_agg_branch(
    b: &Branch,
    agg: &Aggregation,
    dialect: Dialect,
    actuals: &HashMap<usize, Vec<String>>,
) -> Result<EmittedBranch> {
    let mut params = Vec::new();
    let mut pidx = 0usize;

    // FROM (+ LEFT JOIN ON params) renders before WHERE so positional placeholders
    // bind in text order. A core-less inner (an empty BGP) renders without FROM.
    let from = if b.core.is_empty() {
        None
    } else {
        Some(render_from(b, dialect, actuals, &mut params, &mut pidx)?)
    };
    let where_sql = render_where(&b.where_conds, dialect, actuals, &mut params, &mut pidx);

    // The projection + SELECT list, in lockstep: grouping-key raw columns first
    // (also the GROUP BY columns), then each aggregate expression.
    let mut projection: Vec<ColRef> = Vec::new();
    let mut select_items: Vec<String> = Vec::new();
    let mut group_cols: Vec<String> = Vec::new();
    for key in &agg.keys {
        for col in &key.cols {
            let i = projection.len();
            let rendered = colref(col, dialect, actuals);
            select_items.push(format!("{rendered} AS c{i}"));
            group_cols.push(rendered);
            projection.push(col.clone());
        }
    }
    for a in &agg.aggs {
        let i = projection.len();
        let expr = agg_expr_sql(a, dialect, actuals);
        select_items.push(format!("{expr} AS c{i}"));
        projection.push(a.out.clone());
        // AVG result datatype (§11.4) follows the OPERAND numeric type. SQLite's
        // `AVG` always returns a `REAL` (storage class double), erasing an integer/
        // decimal operand's type — so project the operand bare alongside, letting
        // reconstruction read its preserved §10 decltype (SQLite keeps a grouped
        // column's decltype). PostgreSQL's `avg()` already returns the promoted
        // type natively and rejects a bare non-grouped column, so this is
        // SQLite-only (the value is read for its type, never its row).
        if dialect == Dialect::Sqlite && a.kind == AggKind::Avg {
            if let Some(operand) = &a.arg {
                let j = projection.len();
                let rendered = colref(operand, dialect, actuals);
                select_items.push(format!("{rendered} AS c{j}"));
                projection.push(operand.clone());
            }
        }
    }
    let select_list = select_items.join(", ");

    let distinct = if b.distinct { "DISTINCT " } else { "" };
    let mut skeleton = match from {
        Some(f) => format!("SELECT {distinct}{select_list} FROM {f}"),
        None => format!("SELECT {distinct}{select_list}"),
    };
    if let Some(w) = where_sql {
        skeleton.push_str(" WHERE ");
        skeleton.push_str(&w);
    }
    if !group_cols.is_empty() {
        skeleton.push_str(" GROUP BY ");
        skeleton.push_str(&group_cols.join(", "));
    }
    // ORDER BY over an aggregate result is applied in `exec` (never pushed to SQL —
    // it sorts the reconstructed terms type-aware). LIMIT/OFFSET on a single agg
    // branch were pushed by `Plan::prepared_branches` only when unordered.
    if let Some(limit) = b.limit {
        skeleton.push_str(&format!(" LIMIT {limit}"));
    }
    if b.offset > 0 {
        skeleton.push_str(&format!(" OFFSET {}", b.offset));
    }

    let sql = dialect
        .emit_via_ast(&skeleton)
        .map_err(|e| Error::Sql(e.to_string()))?;
    Ok(EmittedBranch {
        sql,
        projection,
        params,
    })
}

/// The SQL for one aggregate output column (`COUNT(*)` / `COUNT`/`SUM`/`AVG`/`MIN`/
/// `MAX(<col>)`, with optional `DISTINCT`). `MIN`/`MAX` ignore DISTINCT (it never
/// changes the extremum), but it is rendered when requested for faithfulness.
fn agg_expr_sql(a: &AggCol, dialect: Dialect, actuals: &HashMap<usize, Vec<String>>) -> String {
    let d = if a.distinct { "DISTINCT " } else { "" };
    let func = match a.kind {
        AggKind::Count => "COUNT",
        AggKind::Sum => "SUM",
        AggKind::Avg => "AVG",
        AggKind::Min => "MIN",
        AggKind::Max => "MAX",
    };
    match &a.arg {
        // COUNT(*) — the only argument-less form (DISTINCT is rejected upstream).
        None => format!("{func}(*)"),
        Some(col) => format!("{func}({d}{})", colref(col, dialect, actuals)),
    }
}

/// Render a [`HopExpr`] to a relation expression yielding the canonical raw key
/// columns `sf_s` / `sf_o` (term-construction lifting, ADR-0007): a bare predicate
/// is a base scan; `^p` swaps the keys; `p/q` joins on the middle node; `p|q` /
/// `!p` set-union the pairs. The result is a `SELECT …` body to be wrapped in
/// `(…) alias` by the caller. Leaf base-column references are resolved against the
/// live catalog (SQL:2008 identifier folding; see the module docs).
fn hop_sql(hop: &HopExpr, dialect: Dialect, catalog: &ColumnCatalog) -> String {
    let (sf_s, sf_o) = (dialect.quote_ident("sf_s"), dialect.quote_ident("sf_o"));
    match hop {
        HopExpr::Pred(rel) => {
            let src = source_sql(&rel.source, dialect);
            let cols = catalog.columns(&rel.source);
            let s = dialect.quote_ident(resolve_col(rel.subj_col.as_ref(), cols));
            let o = dialect.quote_ident(resolve_col(rel.obj_col.as_ref(), cols));
            format!("SELECT h0.{s} AS {sf_s}, h0.{o} AS {sf_o} FROM {src} h0")
        }
        HopExpr::Inverse(inner) => {
            let inner_sql = hop_sql(inner, dialect, catalog);
            format!("SELECT x.{sf_o} AS {sf_s}, x.{sf_s} AS {sf_o} FROM ({inner_sql}) x")
        }
        HopExpr::Seq(a, b) => {
            let a_sql = hop_sql(a, dialect, catalog);
            let b_sql = hop_sql(b, dialect, catalog);
            format!(
                "SELECT a.{sf_s} AS {sf_s}, b.{sf_o} AS {sf_o} \
                 FROM ({a_sql}) a JOIN ({b_sql}) b ON a.{sf_o} = b.{sf_s}"
            )
        }
        HopExpr::Alt(parts) => parts
            .iter()
            .map(|p| {
                let psql = hop_sql(p, dialect, catalog);
                format!("SELECT {sf_s}, {sf_o} FROM ({psql}) u")
            })
            .collect::<Vec<_>>()
            .join(" UNION "),
        // NPS carries BAG semantics (one solution per matching triple): `UNION ALL`
        // over the per-predicate DISTINCT pairs so a pair connected by two
        // complement predicates yields two rows (matching the oracle), while a
        // duplicate row WITHIN one predicate (the same virtual triple) stays
        // collapsed by that predicate's `DISTINCT`. The `PathKind::One` wrapper
        // omits its outer `DISTINCT` to preserve this bag (see `emit_path_branch`).
        HopExpr::Nps(parts) => parts
            .iter()
            .map(|p| {
                let psql = hop_sql(p, dialect, catalog);
                format!("SELECT DISTINCT {sf_s}, {sf_o} FROM ({psql}) u")
            })
            .collect::<Vec<_>>()
            .join(" UNION ALL "),
    }
}

/// The reflexive `(x, x)` pairs over a bare-predicate hop's node set (its subjects
/// ∪ objects) — the ZeroLengthPath component of `P*` / `p?`. `depth` adds a
/// constant `sf_d` column for the recursive (`P*`) anchor. `unfold` only emits
/// reflexive kinds over a single-predicate bare leaf, so a composite hop here is a
/// programming error surfaced as 501.
fn reflexive_sql(
    hop: &HopExpr,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    depth: Option<u32>,
) -> Result<String> {
    let rel = hop.as_pred().ok_or_else(|| {
        Error::Unsupported("reflexive (P*/p?) path over a composite hop → 501".to_owned())
    })?;
    let src = source_sql(&rel.source, dialect);
    let cols = catalog.columns(&rel.source);
    let s = dialect.quote_ident(resolve_col(rel.subj_col.as_ref(), cols));
    let o = dialect.quote_ident(resolve_col(rel.obj_col.as_ref(), cols));
    let (sf_s, sf_o, sf_d) = (
        dialect.quote_ident("sf_s"),
        dialect.quote_ident("sf_o"),
        dialect.quote_ident("sf_d"),
    );
    let dcol = match depth {
        Some(d) => format!(", {d} AS {sf_d}"),
        None => String::new(),
    };
    Ok(format!(
        "SELECT h0.{s} AS {sf_s}, h0.{s} AS {sf_o}{dcol} FROM {src} h0 \
         UNION SELECT h0.{o} AS {sf_s}, h0.{o} AS {sf_o}{dcol} FROM {src} h0"
    ))
}

/// A base source rendered **without** a binding alias (the CTE bodies attach their
/// own `h` / `c` aliases): a quoted table name, or a parenthesised `rr:sqlQuery`.
fn source_sql(source: &LogicalSource, dialect: Dialect) -> String {
    match source {
        LogicalSource::Table(t) => dialect.quote_ident(t),
        LogicalSource::Query(q) => format!("({q})"),
    }
}

fn render_from(
    b: &Branch,
    dialect: Dialect,
    actuals: &HashMap<usize, Vec<String>>,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    // SubPlan derived-table joins (ADR-0023 M5 Wave 2): nested Plans inlined as
    // `(SELECT …) AS t{alias}`. Nested params are spliced at this text-order position
    // (load-bearing for prepared-statement binding — positional order matters).
    //
    // When `core` is non-empty the first core scan is the FROM anchor; SubPlan joins
    // follow as INNER/LEFT JOIN. When `core` is empty AND there are SubPlan joins the
    // first SubPlan becomes the FROM anchor (no CROSS JOIN keyword before it).
    let emit_sp = |sp: &crate::iq::SubPlanJoin,
                   params: &mut Vec<String>,
                   pidx: &mut usize,
                   join_kw: &str|
     -> Result<String> {
        let (nested_sql, nested_params) = emit_subplan_sql(&sp.plan, dialect)?;
        let nested_count = nested_params.len();
        // Rebase Postgres $N placeholders in the nested SQL from $1.. to $(pidx+1)..
        let rebased = rebase_placeholders(&nested_sql, dialect, *pidx)?;
        // Splice nested params into the parent's param vector at this text position.
        params.extend(nested_params);
        // Advance pidx past the nested params so subsequent ON conditions number correctly.
        *pidx += nested_count;
        Ok(format!("{join_kw}({rebased}) t{}", sp.alias))
    };

    let from = if b.core.is_empty() {
        // No base scans: first SubPlan is the FROM anchor (no keyword); remaining join with ON.
        let mut sp_iter = b.subplan_joins.iter();
        let first_sp = sp_iter
            .next()
            .ok_or_else(|| Error::Unsupported("branch with no FROM relation".to_owned()))?;
        let anchor = emit_sp(first_sp, params, pidx, "")?;
        let on_clause = if !first_sp.on.is_empty() {
            // The anchor is a naked derived table — no ON clause in the FROM position;
            // ON-conditions go into WHERE instead. For now emit them into WHERE by
            // pushing to where_conds is not possible here — the only sound path is to
            // require sp.on to be empty for the anchor (lower_as_subplan guarantees this).
            String::new()
        } else {
            String::new()
        };
        let mut from = format!("{}{anchor}", on_clause);
        for sp in sp_iter {
            let join_kw = if sp.left {
                " LEFT JOIN "
            } else {
                " INNER JOIN "
            };
            from.push_str(&emit_sp(sp, params, pidx, join_kw)?);
            if !sp.on.is_empty() {
                from.push_str(" ON ");
                let conds: Vec<&SqlCond> = sp.on.iter().collect();
                from.push_str(&render_conjunction(&conds, dialect, actuals, params, pidx));
            } else {
                from.push_str(" ON 1 = 1");
            }
        }
        from
    } else {
        let mut scans = b.core.iter();
        let first = scans.next().expect("core non-empty — checked above");
        let mut from = scan_ref(&first.source, first.alias, dialect);
        for s in scans {
            from.push_str(" CROSS JOIN ");
            from.push_str(&scan_ref(&s.source, s.alias, dialect));
        }
        for opt in &b.opts {
            from.push_str(" LEFT JOIN ");
            from.push_str(&scan_ref(&opt.scan.source, opt.scan.alias, dialect));
            from.push_str(" ON ");
            let conds: Vec<&SqlCond> = opt.on.iter().chain(opt.extra.iter()).collect();
            from.push_str(&render_conjunction(&conds, dialect, actuals, params, pidx));
        }
        for sp in &b.subplan_joins {
            let join_kw = if sp.left {
                " LEFT JOIN "
            } else {
                " INNER JOIN "
            };
            from.push_str(&emit_sp(sp, params, pidx, join_kw)?);
            if !sp.on.is_empty() {
                from.push_str(" ON ");
                let conds: Vec<&SqlCond> = sp.on.iter().collect();
                from.push_str(&render_conjunction(&conds, dialect, actuals, params, pidx));
            } else {
                from.push_str(" ON 1 = 1");
            }
        }
        from
    };
    Ok(from)
}

/// Render all prepared branches of a nested [`Plan`] to a single SQL SELECT string
/// (for embedding as a derived table). Multi-branch plans become a `UNION ALL`.
/// Returns `(sql_text, params)` — params in text order, placeholders starting from 1.
fn emit_subplan_sql(plan: &crate::Plan, _dialect: Dialect) -> Result<(String, Vec<String>)> {
    let emitted = plan.emitted()?;
    if emitted.is_empty() {
        // Empty inner plan — a values-empty derived table: return a SELECT with no rows.
        // Use a dummy column so it is syntactically valid as a derived table.
        return Ok(("SELECT 1 AS __sf_empty WHERE 1 = 0".to_owned(), Vec::new()));
    }
    if emitted.len() == 1 {
        let e = &emitted[0];
        return Ok((e.sql.clone(), e.params.clone()));
    }
    // Multiple branches: UNION ALL (bag semantics). Each branch's params in text order.
    let mut all_sql = Vec::new();
    let mut all_params = Vec::new();
    for e in &emitted {
        all_sql.push(format!("({})", e.sql));
        all_params.extend(e.params.clone());
    }
    let sql = all_sql.join(" UNION ALL ");
    Ok((sql, all_params))
}

/// Rebase positional `$N` placeholders in `sql` from base 1 to start at `base+1`,
/// for PostgreSQL numbered placeholders. SQLite uses `?` (positional by text order,
/// no numbering), so for SQLite (or when `base == 0`) returns `sql` unchanged.
fn rebase_placeholders(sql: &str, dialect: Dialect, base: usize) -> Result<String> {
    if dialect != Dialect::Postgres || base == 0 {
        return Ok(sql.to_owned());
    }
    // Replace each `$N` → `$(N + base)` by scanning the string bytes.
    let mut out = String::with_capacity(sql.len() + 16);
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            i += 1; // skip '$'
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let n: usize = sql[start..i].parse().map_err(|_| {
                Error::Sql(format!(
                    "rebase_placeholders: non-numeric after $: {}",
                    &sql[start..i]
                ))
            })?;
            out.push('$');
            out.push_str(&(n + base).to_string());
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    Ok(out)
}

fn scan_ref(source: &LogicalSource, alias: usize, dialect: Dialect) -> String {
    match source {
        LogicalSource::Table(t) => format!("{} t{alias}", dialect.quote_ident(t)),
        LogicalSource::Query(q) => format!("({q}) t{alias}"),
    }
}

fn render_where(
    conds: &[SqlCond],
    dialect: Dialect,
    actuals: &HashMap<usize, Vec<String>>,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Option<String> {
    if conds.is_empty() {
        return None;
    }
    let refs: Vec<&SqlCond> = conds.iter().collect();
    Some(render_conjunction(&refs, dialect, actuals, params, pidx))
}

fn render_conjunction(
    conds: &[&SqlCond],
    dialect: Dialect,
    actuals: &HashMap<usize, Vec<String>>,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> String {
    if conds.is_empty() {
        return "1 = 1".to_owned();
    }
    conds
        .iter()
        .map(|c| render_cond(c, dialect, actuals, params, pidx))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn render_cond(
    cond: &SqlCond,
    dialect: Dialect,
    actuals: &HashMap<usize, Vec<String>>,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> String {
    match cond {
        SqlCond::ColEq(a, b) => {
            format!(
                "{} = {}",
                colref(a, dialect, actuals),
                colref(b, dialect, actuals)
            )
        }
        SqlCond::NullSafeEq(a, b) => {
            let (la, lb) = (colref(a, dialect, actuals), colref(b, dialect, actuals));
            format!("({la} = {lb} OR {la} IS NULL OR {lb} IS NULL)")
        }
        SqlCond::Cmp(a, op, val) => {
            params.push(val.clone());
            *pidx += 1;
            format!(
                "{} {} {}",
                colref(a, dialect, actuals),
                op.as_sql(),
                dialect.placeholder(*pidx)
            )
        }
        SqlCond::StrMatch { col, op, param } => {
            // The pattern/regex is a bound parameter (ADR-0010 R1) — never inlined.
            // The `ESCAPE '\'` char is a fixed engine constant (not query data), so
            // it is part of the trusted skeleton, like an identifier.
            params.push(param.clone());
            *pidx += 1;
            let ph = dialect.placeholder(*pidx);
            let c = colref(col, dialect, actuals);
            match op {
                StrMatchOp::Like => format!("{c} LIKE {ph} ESCAPE '\\'"),
                StrMatchOp::RegexMatch => format!("{c} ~ {ph}"),
                StrMatchOp::RegexMatchI => format!("{c} ~* {ph}"),
            }
        }
        SqlCond::IsNotNull(a) => format!("{} IS NOT NULL", colref(a, dialect, actuals)),
        SqlCond::IsNull(a) => format!("{} IS NULL", colref(a, dialect, actuals)),
        SqlCond::Not(c) => format!("(NOT {})", render_cond(c, dialect, actuals, params, pidx)),
        SqlCond::And(cs) => {
            let refs: Vec<&SqlCond> = cs.iter().collect();
            format!(
                "({})",
                render_conjunction(&refs, dialect, actuals, params, pidx)
            )
        }
        SqlCond::Or(cs) => {
            if cs.is_empty() {
                return "1 = 0".to_owned();
            }
            let parts: Vec<String> = cs
                .iter()
                .map(|c| render_cond(c, dialect, actuals, params, pidx))
                .collect();
            format!("({})", parts.join(" OR "))
        }
        // MINUS anti-join (SPARQL §8.3): a correlated `NOT EXISTS` over the right
        // pattern's scans. The inner WHERE renders the right pattern's own
        // conditions plus the shared-variable correlation equalities (which name the
        // outer left aliases), so the whole left row is dropped when a compatible
        // right row exists. A core-less right side (an inline VALUES) renders without
        // FROM. Inner placeholders bind in text order at this position.
        SqlCond::NotExists { scans, conds } | SqlCond::Exists { scans, conds } => {
            let neg = matches!(cond, SqlCond::NotExists { .. });
            let from = scans
                .iter()
                .enumerate()
                .fold(String::new(), |mut acc, (i, s)| {
                    if i > 0 {
                        acc.push_str(" CROSS JOIN ");
                    }
                    acc.push_str(&scan_ref(&s.source, s.alias, dialect));
                    acc
                });
            let refs: Vec<&SqlCond> = conds.iter().collect();
            let where_sql = render_conjunction(&refs, dialect, actuals, params, pidx);
            let kw = if neg { "NOT EXISTS" } else { "EXISTS" };
            if from.is_empty() {
                format!("{kw} (SELECT 1 WHERE {where_sql})")
            } else {
                format!("{kw} (SELECT 1 FROM {from} WHERE {where_sql})")
            }
        }
    }
}

fn colref(c: &ColRef, dialect: Dialect, actuals: &HashMap<usize, Vec<String>>) -> String {
    // The Direct Mapping no-PK blank-node identifier is keyed on the source's
    // physical row id (`sf-mapping`'s synthetic `rowid` column). SQLite exposes
    // that as the `rowid` pseudo-column; PostgreSQL has no `rowid`, so render the
    // equivalent system tuple id `ctid` cast to text (the value is an existential
    // blank-node seed — only per-row uniqueness matters, ADR-0005). Renders as a
    // plain reference for every real column.
    if dialect == Dialect::Postgres && c.column.as_ref() == "rowid" {
        return format!("(t{}.ctid)::text", c.alias);
    }
    let name = resolve_col(&c.column, actuals.get(&c.alias).map(Vec::as_slice));
    format!("t{}.{}", c.alias, dialect.quote_ident(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iq::{Scan, StrMatchOp};
    use sf_core::ir::LogicalSource;

    fn branch_with(cond: SqlCond) -> Branch {
        let mut b = Branch::single(Scan {
            alias: 0,
            source: LogicalSource::Table("emp".to_owned()),
        });
        b.where_conds.push(cond);
        b
    }

    /// A `LIKE` pushdown renders `ESCAPE '\'`, binds its pattern as a parameter,
    /// and round-trips through the AST (ADR-0020 §2 / ADR-0010 R1).
    #[test]
    fn like_renders_escape_and_binds_pattern() {
        let cond = SqlCond::StrMatch {
            col: ColRef::new(0, "name"),
            op: StrMatchOp::Like,
            param: "%a\\%b%".to_owned(),
        };
        let e = emit_branch(&branch_with(cond), Dialect::Sqlite).unwrap();
        let up = e.sql.to_uppercase();
        assert!(up.contains("LIKE") && up.contains("ESCAPE"), "{}", e.sql);
        assert!(e.sql.contains('?'), "bound placeholder: {}", e.sql);
        assert!(
            !e.sql.contains("a%b"),
            "value must not be inlined: {}",
            e.sql
        );
        assert_eq!(e.params, vec!["%a\\%b%".to_owned()]);
    }

    /// Identifier resolution (SQL:2008 folding): an exact match wins (a case-exact /
    /// delimited identifier), else a unique ASCII-case-insensitive match (a regular
    /// identifier the DBMS folded), else the identifier as written.
    #[test]
    fn resolve_col_prefers_exact_then_case_insensitive() {
        let cols = vec!["studentid".to_owned(), "ID".to_owned(), "Name".to_owned()];
        // Regular `StudentId` → no exact, single CI match → the folded actual name.
        assert_eq!(resolve_col("StudentId", Some(&cols)), "studentid");
        // Delimited/case-exact `Name` → exact match wins (never folded away).
        assert_eq!(resolve_col("Name", Some(&cols)), "Name");
        // `ID` exact match.
        assert_eq!(resolve_col("ID", Some(&cols)), "ID");
        // No such column → emitted as written (the source surfaces the error).
        assert_eq!(resolve_col("missing", Some(&cols)), "missing");
        // Unknown source → as written.
        assert_eq!(resolve_col("Whatever", None), "Whatever");
    }

    /// A branch emitted with a catalog resolves its regular-identifier column
    /// reference to the folded column the source actually exposes.
    #[test]
    fn emit_branch_with_resolves_folded_identifier() {
        let mut b = Branch::single(Scan {
            alias: 0,
            source: LogicalSource::Table("Student".to_owned()),
        });
        b.where_conds
            .push(SqlCond::IsNotNull(ColRef::new(0, "StudentId")));
        let mut catalog = ColumnCatalog::default();
        catalog.insert(
            &LogicalSource::Table("Student".to_owned()),
            vec!["studentid".to_owned()],
        );
        let e = emit_branch_with(&b, Dialect::Postgres, &catalog).unwrap();
        assert!(e.sql.contains("\"studentid\""), "{}", e.sql);
        assert!(!e.sql.contains("\"StudentId\""), "{}", e.sql);
        // Reconstruction still keys on the raw IR identifier (position-based read).
        assert_eq!(&*e.projection[0].column, "StudentId");
    }

    /// PostgreSQL regex pushdown renders `~` / `~*` with a numbered, bound param.
    #[test]
    fn pg_regex_renders_operator_and_binds_pattern() {
        let e = emit_branch(
            &branch_with(SqlCond::StrMatch {
                col: ColRef::new(0, "name"),
                op: StrMatchOp::RegexMatchI,
                param: "^a.*".to_owned(),
            }),
            Dialect::Postgres,
        )
        .unwrap();
        assert!(e.sql.contains("~*"), "{}", e.sql);
        assert!(
            e.sql.contains("$1"),
            "numbered bound placeholder: {}",
            e.sql
        );
        assert!(
            !e.sql.contains("^a"),
            "pattern must not be inlined: {}",
            e.sql
        );
        assert_eq!(e.params, vec!["^a.*".to_owned()]);
    }
}
