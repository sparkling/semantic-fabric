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
    Branch, ColRef, HopExpr, OrderKey, PathClosure, PathKind, SqlCond, StrMatchOp, TermDef,
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
fn branch_actuals(b: &Branch, catalog: &ColumnCatalog) -> HashMap<usize, Vec<String>> {
    let mut out = HashMap::new();
    for (alias, source) in b.alias_sources() {
        if let Some(cols) = catalog.columns(source) {
            out.insert(alias, cols.to_vec());
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
    let projection = b.projection();
    let mut params = Vec::new();
    let mut pidx = 0usize;

    // FROM (+ LEFT JOIN ON params) MUST render before WHERE so positional
    // placeholders bind in text order. A core-less branch (an inline `VALUES`
    // constant row — all `Const` bindings, no scan) renders as a one-row
    // `SELECT <const exprs>` with no FROM.
    let from = if b.core.is_empty() {
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
    // A path closure binds its endpoints to constructed template IRIs over the
    // canonical `sf_s`/`sf_o` keys — never a plain `rr:column` — so SQL-ordering by
    // a raw key would not match the IRI order. ORDER BY over a property-path result
    // is therefore deferred → 501 (never a silently wrong order), not dropped.
    if !b.order.is_empty() {
        return Err(Error::Unsupported(
            "ORDER BY over a property-path result is deferred → 501".to_owned(),
        ));
    }
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
    let mut scans = b.core.iter();
    let first = scans
        .next()
        .ok_or_else(|| Error::Unsupported("branch with no FROM relation".to_owned()))?;
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
    Ok(from)
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
