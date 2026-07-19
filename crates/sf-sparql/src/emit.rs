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
        return emit_agg_branch(b, agg, dialect, catalog, &actuals);
    }
    // ADR-0025 (C.3): SQL `DISTINCT` dedups RAW columns, so it implements SPARQL DISTINCT
    // (dedup on the RECONSTRUCTED term) only when every projected term is INJECTIVE in its
    // raw columns. A non-injective template (distinct raw tuples → the same RDF term, e.g.
    // `http://ex/{a}{b}` over `(1,23)`/`(12,3)`) would survive SQL DISTINCT as duplicates
    // SPARQL must collapse — a silent wrong answer. Sound 501. Injective templates pass.
    if b.distinct {
        for def in b.bindings.values() {
            if !crate::cascade::binding_is_injective(def) {
                return Err(Error::Unsupported(
                    "SELECT DISTINCT over a non-injective term (a multi-column template that \
                     maps distinct raw tuples to the same RDF term) cannot be pushed to SQL \
                     DISTINCT soundly → 501 (ADR-0025 C.3)"
                        .to_owned(),
                ));
            }
        }
    }
    let projection = b.projection();
    let mut params = Vec::new();
    let mut pidx = 0usize;

    // FROM (+ LEFT JOIN ON params) MUST render before WHERE so positional
    // placeholders bind in text order. A core-less branch with no SubPlan joins
    // AND no opts (an inline `VALUES` constant row — all `Const` bindings)
    // renders as a one-row `SELECT <const exprs>` with no FROM. A core-less
    // branch WITH SubPlan joins (ADR-0023 M5 Wave 2: SubPlan anchor) or opts (a
    // core-less `LeftJoin` left side, e.g. `{} OPTIONAL {...}`) needs
    // `render_from`'s synthetic/SubPlan anchor — omitting it left the opt's own
    // columns referenced with no FROM clause ever introducing their alias.
    let from = if b.core.is_empty() && b.subplan_joins.is_empty() && b.opts.is_empty() {
        None
    } else {
        Some(render_from(
            b,
            dialect,
            catalog,
            &actuals,
            &mut params,
            &mut pidx,
        )?)
    };
    let where_sql = render_where(
        &b.where_conds,
        dialect,
        catalog,
        &actuals,
        &mut params,
        &mut pidx,
    )?;

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
    push_limit_offset(&mut skeleton, b, dialect);

    let sql = dialect
        .emit_via_ast(&skeleton)
        .map_err(|e| Error::Sql(e.to_string()))?;
    Ok(EmittedBranch {
        sql,
        projection,
        params,
    })
}

/// Push a `LIMIT`/`OFFSET` tail onto `skeleton` (called after `WHERE`/`ORDER BY`
/// have already been rendered, per each caller's own SQL clause order). A BARE
/// `OFFSET` (no `LIMIT`) is a genuine SPARQL shape (`OFFSET n` with no `LIMIT`
/// is valid syntax) but not every dialect's grammar accepts a standalone
/// `OFFSET` clause — `Dialect::bare_offset_limit_sentinel` renders an explicit
/// "no limit" `LIMIT` first for the dialects that need one (confirmed live: a
/// bare `OFFSET n` is a SQLite/MySQL syntax error, so this genuinely fixed a
/// live-emission failure, not a hypothetical one).
fn push_limit_offset(skeleton: &mut String, b: &Branch, dialect: Dialect) {
    match b.limit {
        Some(limit) => skeleton.push_str(&format!(" LIMIT {limit}")),
        None if b.offset > 0 => {
            if let Some(sentinel) = dialect.bare_offset_limit_sentinel() {
                skeleton.push_str(&format!(" LIMIT {sentinel}"));
            }
        }
        None => {}
    }
    if b.offset > 0 {
        skeleton.push_str(&format!(" OFFSET {}", b.offset));
    }
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
/// The `WITH …` prelude defining the path closure's distinct-pairs relation
/// `t{alias}(sf_s, sf_o)` — a plain CTE for length-1 shapes, a `WITH RECURSIVE` for `+`/`*`.
/// Shared by [`emit_path_branch`] (a standalone path result) and the `PathExists`
/// correlated-EXISTS emission (ADR-0025 Tier-2 gap 1); both reference `t{alias}.sf_s`/`.sf_o`.
fn path_with_prelude(
    pc: &PathClosure,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> Result<String> {
    let cte = format!("t{}", pc.alias);
    let hop = hop_sql(&pc.hop, dialect, catalog);
    let (sf_s, sf_o, sf_d) = (
        dialect.quote_ident("sf_s"),
        dialect.quote_ident("sf_o"),
        dialect.quote_ident("sf_d"),
    );
    Ok(match pc.kind {
        PathKind::One => {
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
    })
}

/// Render a path closure as a self-contained derived-table SQL string:
/// `{with} SELECT sf_s, sf_o FROM t{cte_alias}` (ADR-0033). `cte_alias` is a
/// FRESH alias for the closure's OWN internal CTE naming, distinct from
/// `pc.alias` — the caller ([`crate::iq::lower::convert_path_branches`]) keeps
/// `pc.alias` as the OUTER `Scan`'s alias, so every pre-existing
/// `TermDef::Derived{alias: pc.alias, column: "sf_s"/"sf_o"}` binding keeps
/// resolving unchanged against this derived table's identically-named output
/// columns — zero cross-tree rewriting. Reuses [`path_with_prelude`] verbatim
/// (only the closure's `alias` is rebased to `cte_alias` first).
pub(crate) fn path_as_derived_table_sql(
    pc: &PathClosure,
    cte_alias: usize,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> Result<String> {
    let mut inner = pc.clone();
    inner.alias = cte_alias;
    let with = path_with_prelude(&inner, dialect, catalog)?;
    let (sf_s, sf_o) = (dialect.quote_ident("sf_s"), dialect.quote_ident("sf_o"));
    Ok(format!("{with} SELECT {sf_s}, {sf_o} FROM t{cte_alias}"))
}

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
    let with = path_with_prelude(pc, dialect, catalog)?;

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
        catalog,
        &outer_actuals,
        &mut params,
        &mut pidx,
    )? {
        skeleton.push_str(" WHERE ");
        skeleton.push_str(&w);
    }
    push_limit_offset(&mut skeleton, b, dialect);

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
    catalog: &ColumnCatalog,
    actuals: &HashMap<usize, Vec<String>>,
) -> Result<EmittedBranch> {
    let mut params = Vec::new();
    let mut pidx = 0usize;

    // FROM (+ LEFT JOIN ON params) renders before WHERE so positional placeholders
    // bind in text order. A core-less inner with no SubPlan join and no opts
    // either (an empty BGP) renders without FROM; a core-less inner WITH a
    // SubPlan join (the SQL agg-over-UNION pushdown: the pooled arms' derived
    // table is the sole FROM relation) or opts (aggregating over `{} OPTIONAL
    // {...}}`) still needs `render_from` — mirrors `emit_branch_with`'s condition.
    let from = if b.core.is_empty() && b.subplan_joins.is_empty() && b.opts.is_empty() {
        None
    } else {
        Some(render_from(
            b,
            dialect,
            catalog,
            actuals,
            &mut params,
            &mut pidx,
        )?)
    };
    let where_sql = render_where(
        &b.where_conds,
        dialect,
        catalog,
        actuals,
        &mut params,
        &mut pidx,
    )?;

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
    push_limit_offset(&mut skeleton, b, dialect);

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
///
/// `HopExpr::Pred` is the ONLY arm that reads raw base columns directly, so it is
/// the ONLY arm that needs a NULL guard: R2RML §11 generates no triple at all when
/// a referenced column is NULL (matching what the non-path `atom()` triple-pattern
/// emission already enforces via its `obj_null_guard`, `unfold.rs`) — without the
/// guard, a NULL-valued subject or object column becomes a phantom one-hop pair
/// that a recursive closure then chains through transitively, poisoning every node
/// that can reach it. Every composite arm (`Inverse`/`Seq`/`Alt`/`Nps`) only ever
/// recomposes an already-guarded inner `hop_sql`'s `sf_s`/`sf_o`, so the leaf guard
/// alone makes every composite sound too — no separate guard needed there.
fn hop_sql(hop: &HopExpr, dialect: Dialect, catalog: &ColumnCatalog) -> String {
    let (sf_s, sf_o) = (dialect.quote_ident("sf_s"), dialect.quote_ident("sf_o"));
    match hop {
        HopExpr::Pred(rel) => {
            let src = source_sql(&rel.source, dialect);
            let cols = catalog.columns(&rel.source);
            let s = dialect.quote_ident(resolve_col(rel.subj_col.as_ref(), cols));
            let o = dialect.quote_ident(resolve_col(rel.obj_col.as_ref(), cols));
            format!(
                "SELECT h0.{s} AS {sf_s}, h0.{o} AS {sf_o} FROM {src} h0 \
                 WHERE h0.{s} IS NOT NULL AND h0.{o} IS NOT NULL"
            )
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
///
/// Reads `subj_col`/`obj_col` directly (like `hop_sql`'s `HopExpr::Pred` leaf, but
/// NOT through it), so it needs the identical NULL guard: a row where EITHER
/// column is NULL generates no triple at all (R2RML §11), hence contributes
/// NEITHER a subject-node NOR an object-node to this predicate's graph — without
/// the guard, a NULL-valued column would seed a phantom `(NULL, NULL)` reflexive
/// pair. Both `UNION` halves share the SAME row-level guard (not a per-column
/// guard on just the column each half projects): a row failing the OTHER column's
/// NULL check still generates no triple, so its own column is not a valid node
/// either.
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
         WHERE h0.{s} IS NOT NULL AND h0.{o} IS NOT NULL \
         UNION SELECT h0.{o} AS {sf_s}, h0.{o} AS {sf_o}{dcol} FROM {src} h0 \
         WHERE h0.{s} IS NOT NULL AND h0.{o} IS NOT NULL"
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
    catalog: &ColumnCatalog,
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
        // No base (core) scans. A synthetic one-row anchor stands in for the
        // `IqNode::True` / empty-BGP identity this branch's own left side represents
        // (SPARQL's `{} OPTIONAL {X}` always has exactly one solution to extend), and
        // EVERY opt / SubPlan attaches to it via its own JOIN — never as a hard FROM
        // anchor that would wrongly drop that guaranteed row on zero matches.
        //
        // Opts render BEFORE SubPlans — the SAME order as the core-bearing branch below
        // — so a SubPlan whose `on` correlates on a prior opt (or earlier SubPlan)
        // references a table already emitted to its LEFT (valid SQL). Previously this
        // path made the FIRST SubPlan the FROM anchor and emitted it with NO `ON` clause
        // AND rendered opts AFTER SubPlans: a SubPlan correlated on an opt then either
        // silently DROPPED its correlation (a wrong answer — an uncorrelated cross join)
        // or referenced an opt emitted to its right (a crash) — both ADR-0007 violations.
        // The uniform "(SELECT 1) anchor, then opts, then SubPlans" order fixes them and
        // matches the already-shipped core-less-plus-opts fix
        // (`bare_group_as_leftjoin_left_no_longer_mis_aliases`). A one-row cross join is
        // `=_bag`-transparent, so the previously-anchor-was-a-SubPlan cases (an
        // uncorrelated `{} OPTIONAL {sub}`, the SQL agg-over-UNION pushdown) keep their
        // meaning — only their cosmetic SQL shape changes.
        let mut from = "(SELECT 1) t_empty".to_owned();
        for opt in &b.opts {
            from.push_str(" LEFT JOIN ");
            from.push_str(&scan_ref(&opt.scan.source, opt.scan.alias, dialect));
            from.push_str(" ON ");
            let conds: Vec<&SqlCond> = opt.on.iter().chain(opt.extra.iter()).collect();
            from.push_str(&render_conjunction(
                &conds, dialect, catalog, actuals, params, pidx,
            )?);
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
                from.push_str(&render_conjunction(
                    &conds, dialect, catalog, actuals, params, pidx,
                )?);
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
            from.push_str(&render_conjunction(
                &conds, dialect, catalog, actuals, params, pidx,
            )?);
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
                from.push_str(&render_conjunction(
                    &conds, dialect, catalog, actuals, params, pidx,
                )?);
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
fn emit_subplan_sql(plan: &crate::Plan, dialect: Dialect) -> Result<(String, Vec<String>)> {
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
    // Multiple branches: `UNION ALL` (bag semantics) by default, or `UNION` (dedup) when the
    // plan carries a DISTINCT (a multi-branch DISTINCT SubPlan, ADR-0025 Tier-2 gap 2 — the
    // pooling requires injective cross-arm reconstruction, so SQL `UNION`'s raw-column dedup
    // equals SPARQL DISTINCT on the reconstructed terms). SQLite's compound-select grammar
    // does NOT accept a parenthesised `select-core` as a UNION operand (`(SELECT …) UNION …`
    // is a syntax error there — the q9 agg-pushdown wave's first live failure); PG/MySQL
    // accept it. So SQLite joins the arms bare.
    let mut all_sql = Vec::new();
    let mut all_params = Vec::new();
    for e in &emitted {
        if dialect == Dialect::Sqlite {
            all_sql.push(e.sql.clone());
        } else {
            all_sql.push(format!("({})", e.sql));
        }
        all_params.extend(e.params.clone());
    }
    let op = if plan.distinct {
        " UNION "
    } else {
        " UNION ALL "
    };
    let sql = all_sql.join(op);
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
    catalog: &ColumnCatalog,
    actuals: &HashMap<usize, Vec<String>>,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<Option<String>> {
    if conds.is_empty() {
        return Ok(None);
    }
    let refs: Vec<&SqlCond> = conds.iter().collect();
    Ok(Some(render_conjunction(
        &refs, dialect, catalog, actuals, params, pidx,
    )?))
}

fn render_conjunction(
    conds: &[&SqlCond],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &HashMap<usize, Vec<String>>,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    if conds.is_empty() {
        return Ok("1 = 1".to_owned());
    }
    Ok(conds
        .iter()
        .map(|c| render_cond(c, dialect, catalog, actuals, params, pidx))
        .collect::<Result<Vec<_>>>()?
        .join(" AND "))
}

fn render_cond(
    cond: &SqlCond,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &HashMap<usize, Vec<String>>,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    Ok(match cond {
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
        SqlCond::Not(c) => format!(
            "(NOT {})",
            render_cond(c, dialect, catalog, actuals, params, pidx)?
        ),
        SqlCond::And(cs) => {
            let refs: Vec<&SqlCond> = cs.iter().collect();
            format!(
                "({})",
                render_conjunction(&refs, dialect, catalog, actuals, params, pidx)?
            )
        }
        SqlCond::Or(cs) => {
            if cs.is_empty() {
                return Ok("1 = 0".to_owned());
            }
            let parts: Vec<String> = cs
                .iter()
                .map(|c| render_cond(c, dialect, catalog, actuals, params, pidx))
                .collect::<Result<Vec<_>>>()?;
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
            let where_sql = render_conjunction(&refs, dialect, catalog, actuals, params, pidx)?;
            let kw = if neg { "NOT EXISTS" } else { "EXISTS" };
            if from.is_empty() {
                format!("{kw} (SELECT 1 WHERE {where_sql})")
            } else {
                format!("{kw} (SELECT 1 FROM {from} WHERE {where_sql})")
            }
        }
        // ADR-0025 Tier-2 gap 1: a correlated [NOT] EXISTS whose inner is a property-path
        // CLOSURE — the recursive-CTE distinct-pairs table `t{pc.alias}(sf_s, sf_o)`. The
        // prelude resolves its own base columns against the live `catalog` (threaded through
        // this render chain), so ALL path kinds — including the reflexive `P*`/`P?`, whose
        // prelude calls the fallible `reflexive_sql` — render here; the `?` propagates any
        // prelude error soundly instead of the old empty-catalog `unwrap_or_default`.
        SqlCond::PathExists { pc, conds, negated } => {
            let with = path_with_prelude(pc, dialect, catalog)?;
            let refs: Vec<&SqlCond> = conds.iter().collect();
            let where_sql = render_conjunction(&refs, dialect, catalog, actuals, params, pidx)?;
            let kw = if *negated { "NOT EXISTS" } else { "EXISTS" };
            format!(
                "{kw} ({with} SELECT 1 FROM t{} WHERE {where_sql})",
                pc.alias
            )
        }
        // Run 4 Wave B3 — `unify::align_templates`'s shape-mismatch fallback:
        // render each side as a SQL string concatenation and compare with
        // `=`. See `render_template_concat`'s doc comment for the
        // dialect-support boundary and the NULL-propagation soundness
        // argument (why a NULL underlying column correctly excludes the row
        // rather than needing special-casing here).
        SqlCond::TemplateEq(sx, a1, sy, a2, encode_iri) => {
            let r1 = render_template_concat(sx, *a1, *encode_iri, dialect, actuals, params, pidx)?;
            let r2 = render_template_concat(sy, *a2, *encode_iri, dialect, actuals, params, pidx)?;
            format!("{r1} = {r2}")
        }
    })
}

/// Render one template's segments as a dialect-appropriate SQL string
/// concatenation — [`SqlCond::TemplateEq`]'s per-side rendering (Run 4 Wave
/// B3). Every `Segment::Literal` is a bound parameter (ADR-0010 R1): the
/// text is mapping-trusted, not query-supplied, but this still follows the
/// blanket "values are bound parameters only" rule rather than hand-rolling
/// SQL string-literal escaping. Every `Segment::Column` renders through the
/// SAME [`colref`] every other `SqlCond` arm uses, additionally wrapped in
/// [`percent_encode_col`] when `encode_iri` (below).
///
/// **Percent-encoding soundness (Run 4 B-repair FIX 2).** `sf_core::ir::
/// Template::expand`'s `encode_iri` flag percent-encodes EVERY column value
/// — never a template's own literal segments — when (and only when) the
/// template constructs an IRI (R2RML §7.3 / RFC 3987); a plain-literal
/// template's expansion is raw, unencoded column text. Before this fix,
/// EVERY `Segment::Column` rendered here as a bare `colref`, regardless of
/// kind — sound for two templates whose column values happen to carry no
/// IRI-encodable character, but wrong in general: a single-column template
/// `http://ex.org/v/{va}` with `va = "X/Y"` expands (RDF) to
/// `http://ex.org/v/X%2FY` (the `/` encoded), while a two-column template
/// `http://ex.org/v/{vb1}/{vb2}` with `vb1="X", vb2="Y"` expands to
/// `http://ex.org/v/X/Y` (the `/` a literal template character) — DIFFERENT
/// IRIs, yet the old raw-concat rendering compared them EQUAL (false
/// positive), and conversely could compare two RDF-equal IRIs unequal.
/// `encode_iri` is one flag for BOTH sides ([`SqlCond::TemplateEq`]'s own
/// doc comment: `align_templates`'s caller only ever builds this variant
/// when both sides share one `TermType`), so each `Segment::Column` here is
/// consistently encoded, or consistently left raw, matching whichever
/// `Template::expand` itself would do for that same template.
///
/// **NULL-propagation soundness.** On every dialect rendered below, `||`
/// (Postgres/SQLite) and `CONCAT(...)` (MySQL) return SQL `NULL` if ANY
/// operand is `NULL` — so `render(t1) = render(t2)` evaluates to UNKNOWN
/// (never `TRUE`) whenever a referenced column is `NULL`, and a WHERE/JOIN
/// condition that is UNKNOWN excludes the row, same as `FALSE`. This is NOT
/// an approximation: `sf_core::term::generate_into`'s `TermMap::Template`
/// arm (`Template::expand`, per R2RML §11) ALREADY treats "any referenced
/// column is NULL" as "this variable is UNBOUND" for that row when
/// RECONSTRUCTING the SAME template in Rust — `sf-core/term.rs`'s
/// `null_in_template_yields_no_term` test locks this in. So a NULL-collapsed
/// concatenation here excludes EXACTLY the rows whose SPARQL-level operand
/// would have been unbound anyway (comparing against an unbound variable is
/// a type error ⇒ the row is excluded from a FILTER, and an unbound shared
/// variable cannot correlate a join either) — the SQL and RDF answers agree
/// by construction, not by coincidence. [`percent_encode_col`]'s own three
/// per-dialect implementations preserve this EXPLICITLY (a `CASE WHEN col IS
/// NULL THEN NULL ELSE …` wrapper, not incidental `NULL`-propagation through
/// some other operator) — see its own doc comment for why an explicit
/// wrapper is required rather than assumed.
///
/// **Dialect support.** Only the three PRODUCTION-WIRED dialects
/// (`sf_sql::Dialect`'s own grouping) are implemented: PostgreSQL/SQLite via
/// ANSI `||`, MySQL via `CONCAT(...)` (`||` is boolean OR there by default,
/// per MySQL's non-default `PIPES_AS_CONCAT` sql_mode). Every OTHER dialect
/// returns `Unsupported` rather than guessing — e.g. SQL Server's own
/// `CONCAT()` function treats `NULL` as an EMPTY STRING (breaking the
/// soundness argument above outright), and `+`, SQL Server's NULL-safe
/// concat operator, is unverified against this rendering; Oracle/DuckDB/etc.
/// are ANSI-`||`-following by reputation but likewise unverified here —
/// "sound over complete", the same bar `str_match`'s PostgreSQL-only `LIKE`
/// pushdown already sets for an analogous dialect-behavior gap.
fn render_template_concat(
    segs: &[sf_core::ir::Segment],
    alias: usize,
    encode_iri: bool,
    dialect: Dialect,
    actuals: &HashMap<usize, Vec<String>>,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    use sf_core::ir::Segment;
    let mut parts = Vec::with_capacity(segs.len());
    for seg in segs {
        parts.push(match seg {
            Segment::Literal(text) => {
                params.push(text.to_string());
                *pidx += 1;
                dialect.placeholder(*pidx)
            }
            Segment::Column(c) => {
                let col = colref(&ColRef::new(alias, c.clone()), dialect, actuals);
                if encode_iri {
                    percent_encode_col(&col, dialect)?
                } else {
                    col
                }
            }
        });
    }
    match dialect {
        Dialect::Postgres | Dialect::Sqlite => Ok(format!("({})", parts.join(" || "))),
        Dialect::MySql => Ok(format!("CONCAT({})", parts.join(", "))),
        other => Err(Error::Unsupported(format!(
            "template-shape-mismatch equality (SQL CONCAT fallback) is not implemented for \
             {other:?} → 501 (never a silently wrong NULL/concat-operator guess)"
        ))),
    }
}

/// Percent-encode `col_sql`'s runtime value EXACTLY the way `sf_core::ir::
/// Template::expand`'s `encode_iri` arm does (`percent_encode_iri`, same
/// file): RFC 3987 *iunreserved* = `ALPHA / DIGIT / "-" / "." / "_" / "~"`
/// passes through; every OTHER byte (the FULL 0x00-0x7F complement, all 62
/// bytes, including every ASCII control byte) becomes `%XX` (uppercase
/// hex); non-ASCII passes through unchanged.
///
/// **History: why this is not a flat `REPLACE` chain.** An earlier version
/// nested one `REPLACE` call per encodable byte — `REPLACE` being ANSI-
/// portable, the obvious building block. That fails for two INDEPENDENT
/// reasons, both found empirically against [`Dialect::emit_via_ast`] (the
/// `sqlparser` AST round-trip every emitted statement goes through):
/// 1. **SQLite** has a hard recursion-depth ceiling (`sqlparser`'s
///    `DEFAULT_REMAINING_DEPTH`) — a flat chain over the full 62-byte set
///    (deeper than the empirically measured ~41-44-level ceiling inside a
///    realistic WHERE clause) errors "recursion limit exceeded" outright.
/// 2. **PostgreSQL** is far worse: not a lower ceiling but EXPONENTIAL
///    parse time in nesting depth, well before any hard limit fires
///    (measured: depth 8 ≈ 14ms, depth 12 ≈ 99ms, depth 20 ≈ over 16
///    SECONDS) — general to nested-function-call parsing in `sqlparser`'s
///    PG dialect (reproduced with a single-argument `UPPER(...)` chain, not
///    just `REPLACE`), so no flat chain wide enough for full coverage is
///    viable there at any practical depth.
///
/// **The fix: per-character SQL, not per-character SQL TEXT NESTING.** Each
/// dialect gets its OWN O(1)-parse-depth encoder — a single `WITH RECURSIVE`
/// (or, for PostgreSQL, `unnest(...) WITH ORDINALITY`) that iterates the
/// STRING'S OWN characters/bytes as ROWS, classifies each with one `CASE`,
/// and reassembles via an ORDER-preserving aggregate (`group_concat`/
/// `string_agg`/`GROUP_CONCAT`, all `... ORDER BY ...` — SQLite 3.44+, this
/// project's bundled 3.46.0 confirmed; PostgreSQL and MySQL support it
/// natively). Parse depth is CONSTANT regardless of the encode-set size or
/// the runtime string length — confirmed fast (single-digit milliseconds)
/// against the SAME realistic OR-IS-NULL-wrapped, multi-column WHERE clause
/// that broke the flat-chain design, for all three dialects.
///
/// **Byte- vs. character-oriented, per dialect — not interchangeable.**
/// SQLite's/MySQL's plain `LENGTH()`/`SUBSTRING()` on a TEXT argument are
/// NOT reliable byte-accurate iterators (SQLite's is character-counting and
/// silently truncates at an embedded NUL, exactly like a C string; MySQL's
/// `LENGTH()` is byte-oriented but its plain `SUBSTRING()` is CHARACTER-
/// oriented — an internally inconsistent pairing that walks past a
/// multi-byte character's true end) — both confirmed by direct, deliberate
/// probing before this design was settled on, both fixed by an explicit
/// `CAST(... AS BLOB)` (SQLite) / `CAST(... AS BINARY)` (MySQL) so every
/// function in the chain is consistently byte-oriented; a non-ASCII
/// multi-byte character is then walked and reassembled ONE RAW BYTE AT A
/// TIME (every continuation/lead byte is ≥ 0x80, so "byte ≥ 0x80 passes
/// through unchanged" correctly reconstructs it without ever needing to
/// understand UTF-8 structure) — confirmed an ISOLATED intermediate byte
/// cast is not independently valid UTF-8, but the FINAL reassembled result
/// is. PostgreSQL's `text` is different on both counts: it cannot contain a
/// NUL byte at all (the server rejects it outright — confirmed live,
/// `ERROR: invalid byte sequence for encoding "UTF8": 0x00` — so there is
/// no NUL case to handle), and `string_to_array(text, NULL)` natively splits
/// by CHARACTER (not byte), which is the natural, already-decoded unit
/// there — `ascii(ch)` gives the correct code point for classification
/// (including non-ASCII), so no BLOB-equivalent cast is needed.
///
/// **NULL-propagation, correctly this time.** `sf_core::term::generate_into`
/// treats "any referenced column is NULL" as "this variable is UNBOUND" for
/// that row (`null_in_template_yields_no_term`, sf-core), so a NULL column
/// must render as SQL NULL — but the natural per-character aggregate
/// (`group_concat`/`string_agg`/`GROUP_CONCAT`) returns NULL over ZERO
/// input rows REGARDLESS of why there were zero rows: a genuinely NULL
/// column (nothing to iterate) and a genuinely EMPTY, non-NULL string
/// (also nothing to iterate, but should encode to `""`, not NULL) are
/// otherwise indistinguishable through the aggregate alone — confirmed by
/// direct probing: an early version without the NULL-vs-empty split below
/// wrongly rendered EACH of "NULL" and "empty string" as if the OTHER,
/// AND wrongly emitted a bare stray `%` for the empty-string case (the
/// iterator's own "at least one row" base case still fired past the end of
/// a zero-length string). Every implementation below is therefore the SAME
/// three-part shape: `CASE WHEN col IS NULL THEN NULL ELSE COALESCE(
/// <per-character aggregate>, '') END` — the outer CASE separates "no
/// value" from "empty value" (COALESCE alone cannot), and COALESCE only
/// then normalizes the (now unambiguous) zero-character case to `''`.
///
/// Every literal in these templates (hex-range bounds, the `-._~` char
/// list, dialect keywords) is a fixed SQL-syntax constant under this
/// module's own control, not query- or mapping-supplied data — inlining
/// them is the "fixed engine constant… part of the trusted skeleton" rule
/// this file's `LIKE ESCAPE '\'` rendering already uses (`render_cond`'s
/// `StrMatch` arm), not a departure from ADR-0010 R1 (which governs values
/// that originate from the SPARQL query or the mapping, neither of which
/// this function ever touches — the ONLY runtime input is the column
/// reference itself, already-resolved SQL text, not a bound value).
fn percent_encode_col(col_sql: &str, dialect: Dialect) -> Result<String> {
    Ok(match dialect {
        Dialect::Sqlite => percent_encode_col_sqlite(col_sql),
        Dialect::MySql => percent_encode_col_mysql(col_sql),
        Dialect::Postgres => percent_encode_col_postgres(col_sql),
        other => {
            return Err(Error::Unsupported(format!(
                "IRI-template percent-encoding is not implemented for {other:?} → 501 \
                 (never a silently wrong un-encoded comparison)"
            )))
        }
    })
}

/// SQLite: `CAST(... AS BLOB)` throughout (byte-oriented `LENGTH`/`substr`,
/// sidestepping TEXT-mode `LENGTH`'s NUL-terminated character counting —
/// see [`percent_encode_col`]'s doc comment). `hex()`, not `unicode()`, for
/// byte classification (`unicode()` reinterprets an isolated non-ASCII byte
/// as a UTF-8 decode attempt and returns the U+FFFD replacement code point
/// for an invalid standalone continuation/lead byte — confirmed live;
/// `hex()` returns the raw byte unconditionally). `group_concat(...ORDER BY
/// n)` needs SQLite ≥ 3.44 (this project's bundled `libsqlite3-sys` ships
/// 3.46.0, confirmed live).
fn percent_encode_col_sqlite(col: &str) -> String {
    format!(
        "(SELECT CASE WHEN {col} IS NULL THEN NULL ELSE COALESCE((\
WITH RECURSIVE seq(n) AS (\
SELECT 1 WHERE LENGTH(CAST({col} AS BLOB)) > 0 \
UNION ALL \
SELECT n + 1 FROM seq WHERE n < LENGTH(CAST({col} AS BLOB))\
) \
SELECT group_concat(\
CASE \
WHEN hex(substr(CAST({col} AS BLOB), n, 1)) BETWEEN '30' AND '39' \
OR hex(substr(CAST({col} AS BLOB), n, 1)) BETWEEN '41' AND '5A' \
OR hex(substr(CAST({col} AS BLOB), n, 1)) BETWEEN '61' AND '7A' \
OR hex(substr(CAST({col} AS BLOB), n, 1)) IN ('2D', '2E', '5F', '7E') \
OR hex(substr(CAST({col} AS BLOB), n, 1)) >= '80' \
THEN CAST(substr(CAST({col} AS BLOB), n, 1) AS TEXT) \
ELSE '%' || hex(substr(CAST({col} AS BLOB), n, 1)) \
END, '' ORDER BY n\
) FROM seq\
), '') END)"
    )
}

/// MySQL: `CAST(... AS BINARY)` throughout — MySQL's `LENGTH()` is
/// byte-oriented but its plain `SUBSTRING()` is CHARACTER-oriented (a
/// confirmed-live, internally inconsistent pairing this cast reconciles,
/// mirroring the SQLite BLOB cast for the identical class of unit
/// mismatch); the pass-through branch and the `%XX` branch are BOTH kept as
/// `BINARY` inside `GROUP_CONCAT` so the aggregate never implicitly
/// re-interprets an in-flight (possibly standalone-invalid) byte as text,
/// and the FINAL aggregated result is converted back to `utf8mb4` once, at
/// the very end (mirrors the SQLite per-byte-cast pattern: only the fully
/// reassembled result needs to be valid UTF-8, confirmed live). The
/// `SET_VAR` optimizer hint raises two session limits for THIS query only
/// (no separate `SET SESSION` statement, so no change to connection
/// setup elsewhere in the codebase): `group_concat_max_len` (MySQL's
/// default of 1024 bytes SILENTLY truncates a longer `GROUP_CONCAT` result
/// with no error — confirmed live — which is exactly the unsound-truncation
/// class this whole fix exists to close, so it cannot be left at the
/// default) and `cte_max_recursion_depth` (default 1000; exceeding it is a
/// query ERROR, not a silent truncation — sound but incomplete for a very
/// long column value — raised anyway for headroom, confirmed live up to a
/// 2000-character input).
fn percent_encode_col_mysql(col: &str) -> String {
    format!(
        "(WITH RECURSIVE seq AS (\
SELECT 1 AS n WHERE LENGTH(CAST({col} AS BINARY)) > 0 \
UNION ALL \
SELECT n + 1 FROM seq WHERE n < LENGTH(CAST({col} AS BINARY))\
) \
SELECT CASE WHEN {col} IS NULL THEN NULL ELSE COALESCE((\
SELECT /*+ SET_VAR(group_concat_max_len = 1000000) SET_VAR(cte_max_recursion_depth = 100000) */ \
CONVERT(CAST(GROUP_CONCAT(\
CASE \
WHEN HEX(SUBSTRING(CAST({col} AS BINARY), n, 1)) BETWEEN '30' AND '39' \
OR HEX(SUBSTRING(CAST({col} AS BINARY), n, 1)) BETWEEN '41' AND '5A' \
OR HEX(SUBSTRING(CAST({col} AS BINARY), n, 1)) BETWEEN '61' AND '7A' \
OR HEX(SUBSTRING(CAST({col} AS BINARY), n, 1)) IN ('2D', '2E', '5F', '7E') \
OR HEX(SUBSTRING(CAST({col} AS BINARY), n, 1)) >= '80' \
THEN SUBSTRING(CAST({col} AS BINARY), n, 1) \
ELSE CAST(CONCAT('%', HEX(SUBSTRING(CAST({col} AS BINARY), n, 1))) AS BINARY) \
END ORDER BY n SEPARATOR ''\
) AS BINARY) USING utf8mb4)\
FROM seq\
), '') END)"
    )
}

/// PostgreSQL: character-oriented (`string_to_array(text, NULL)` natively
/// splits by character — the correct unit for `text`, which is always
/// well-formed and cannot carry an embedded NUL at all, confirmed live).
/// `ascii(ch)` gives the numeric code point for classification (correct
/// for non-ASCII too, unlike a collation-dependent text comparison against
/// `chr(128)` would be). `unnest(...) WITH ORDINALITY` supplies the
/// position `string_agg(... ORDER BY ord)` reassembles by.
fn percent_encode_col_postgres(col: &str) -> String {
    format!(
        "(SELECT CASE WHEN {col}::text IS NULL THEN NULL ELSE COALESCE((\
SELECT string_agg(\
CASE \
WHEN ascii(ch) BETWEEN 48 AND 57 \
OR ascii(ch) BETWEEN 65 AND 90 \
OR ascii(ch) BETWEEN 97 AND 122 \
OR ch IN ('-', '.', '_', '~') \
OR ascii(ch) >= 128 \
THEN ch \
ELSE '%' || UPPER(LPAD(TO_HEX(ascii(ch)), 2, '0')) \
END, '' ORDER BY ord\
) FROM unnest(string_to_array({col}::text, NULL)) WITH ORDINALITY AS t(ch, ord)\
), '') END)"
    )
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

    /// Rust-side reimplementation of `sf_core::ir`'s private `percent_encode_iri`,
    /// for comparison ONLY (that function isn't `pub`) — verified byte-identical
    /// to it via `sf-core`'s own `expand_writes_through_and_percent_encodes_iris`
    /// test's fixtures, reproduced inline below.
    fn reference_encode(value: &str) -> String {
        let mut out = String::new();
        for ch in value.chars() {
            if ch.is_ascii() {
                let byte = ch as u8;
                if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                    out.push(ch);
                } else {
                    out.push_str(&format!("%{byte:02X}"));
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    /// Every dialect's [`percent_encode_col`] must reconstruct EXACTLY what
    /// `sf_core::ir::percent_encode_iri` computes, across the full encodable
    /// byte range (every printable ASCII special AND every control byte,
    /// 0x00-0x1F/0x7F) plus the edge cases design-time probing against a
    /// live SQLite connection found real bugs in: an embedded NUL byte
    /// (SQLite's TEXT-mode `LENGTH` is NUL-terminated and silently
    /// truncates — closed by `CAST(... AS BLOB)`, see
    /// [`percent_encode_col_sqlite`]'s doc comment), an empty-but-non-NULL
    /// string (a naive recursive-CTE base case still fired once past the
    /// end, wrongly emitting a bare `%`), a genuinely NULL column (the
    /// aggregate collapses an empty AND a NULL input to the same result
    /// unless explicitly distinguished), and a multi-byte UTF-8 (CJK)
    /// character (each of its individual bytes is standalone-invalid UTF-8,
    /// exercising the byte-level reassembly path). SQLite only here (no
    /// live-server dependency for a routine `cargo test` run) — PostgreSQL
    /// and MySQL were validated the identical way against live servers
    /// during this fix's development; see [`percent_encode_col_postgres`]/
    /// [`percent_encode_col_mysql`]'s own doc comments for the
    /// dialect-specific bugs their first drafts had (a PG NULL/empty
    /// conflation; a MySQL `LENGTH`-vs-`SUBSTRING` byte/character unit
    /// mismatch; a MySQL `group_concat_max_len`/`cte_max_recursion_depth`
    /// silent-truncation exposure).
    #[test]
    fn percent_encode_col_sqlite_matches_reference_iri_encoding() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (v TEXT)", []).unwrap();

        let mut cases: Vec<Option<String>> = vec![
            Some("a b/c".to_owned()),
            Some("A-z.0_9~".to_owned()),
            Some("".to_owned()),
            None,
            Some("X/Y".to_owned()),
            Some("你好/世界".to_owned()), // non-ASCII must pass through raw
            Some("tab\ttab".to_owned()),  // control byte 0x09
            Some("nul\u{0}nul".to_owned()), // embedded NUL, 0x00
        ];
        for b in 0x20u8..=0x7e {
            cases.push(Some((b as char).to_string())); // every printable ASCII byte
        }
        for b in 0..=0x1fu8 {
            cases.push(Some((b as char).to_string())); // every control byte
        }
        cases.push(Some((0x7fu8 as char).to_string()));

        let sql = percent_encode_col("t.v", Dialect::Sqlite).expect("SQLite is supported");
        let query = format!("SELECT {sql} FROM t");
        let mut mismatches = Vec::new();
        for v in &cases {
            conn.execute("DELETE FROM t", []).unwrap();
            conn.execute("INSERT INTO t (v) VALUES (?1)", [v]).unwrap();
            let got: Option<String> = conn.query_row(&query, [], |r| r.get(0)).unwrap();
            let want = v.as_deref().map(reference_encode);
            if got != want {
                mismatches.push(format!("input={v:?} got={got:?} want={want:?}"));
            }
        }
        assert!(mismatches.is_empty(), "{mismatches:#?}");
    }

    /// A dialect this module does not implement encoding for (Oracle, picked
    /// arbitrarily) declines soundly rather than guessing.
    #[test]
    fn percent_encode_col_unsupported_dialect_is_501() {
        assert!(matches!(
            percent_encode_col("t.v", Dialect::Oracle),
            Err(Error::Unsupported(_))
        ));
    }
}
