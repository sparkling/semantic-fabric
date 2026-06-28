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

use sf_core::ir::LogicalSource;
use sf_sql::Dialect;

use crate::iq::{Branch, ColRef, PathClosure, SqlCond, StrMatchOp};
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
        return emit_path_branch(b, pc, dialect, &actuals);
    }
    let projection = b.projection();
    let mut params = Vec::new();
    let mut pidx = 0usize;

    // FROM (+ LEFT JOIN ON params) MUST render before WHERE so positional
    // placeholders bind in text order.
    let from = render_from(b, dialect, &actuals, &mut params, &mut pidx)?;
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
    let mut skeleton = format!("SELECT {distinct}{select_list} FROM {from}");
    if let Some(w) = where_sql {
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

/// Render a recursive property-path closure branch to a `WITH RECURSIVE` CTE
/// (ADR-0007 *recursive paths compile to source-dialect recursive CTEs*).
///
/// The CTE iterates the one-hop relation over **raw key columns** (term-gen
/// lifting): the recursive member `t{alias}r` projects `sf_s` / `sf_o` (the two
/// keys) plus an `sf_d` depth counter. Its body is a **`UNION`** (it dedups on the
/// `(sf_s, sf_o, sf_d)` triple), but because `sf_d` is part of that key a pair
/// reached at several depths — or revisited around a cycle — is NOT collapsed
/// there: `sf_d < max_depth` is the *sole* recursion terminator (ADR-0010 depth
/// backstop; SQLite has no `CYCLE` clause — that is the later MB-4 wave). SPARQL
/// `P+`/`P*` are set-semantics over node **pairs**, so a second non-recursive CTE
/// `t{alias}` = `SELECT DISTINCT sf_s, sf_o` collapses the depth dimension to the
/// distinct reachable pairs the outer projection then reads. Deduping the *pair*
/// (not the projected columns) is what keeps `SELECT ?s WHERE {?s p+ ?o}` a
/// correct bag of `?s` even when `?o` is dropped. `P+` anchors on the one-hop
/// pairs (depth 1); `P*` additionally seeds the reflexive `(x, x)` pairs over the
/// hop's node set (subjects ∪ objects) at depth 0 — sound only for a
/// single-predicate active graph, which `unfold` enforces (else `P*` → 501). The
/// depth ints and the bound are engine constants (not query data), so — like
/// `LIMIT` and the `ESCAPE '\'` char — they are part of the trusted skeleton, not
/// bound params. RDF terms are still built only at the outer projection
/// ([`crate::exec`]).
///
/// This wave targets the SQLite dialect; the PostgreSQL `CYCLE`/PG14 variant is
/// the later MB-4 wave.
fn emit_path_branch(
    b: &Branch,
    pc: &PathClosure,
    dialect: Dialect,
    actuals: &HashMap<usize, Vec<String>>,
) -> Result<EmittedBranch> {
    let projection = b.projection();
    let mut params = Vec::new();
    let mut pidx = 0usize;

    // `cte` is the distinct-pairs relation the outer projection reads (colref binds
    // `t{alias}`); `cte_raw` is the depth-tracked recursive closure behind it.
    let cte = format!("t{}", pc.alias);
    let cte_raw = format!("t{}r", pc.alias);
    let src = source_sql(&pc.hop.source, dialect);
    let s_col = dialect.quote_ident(&pc.hop.subj_col);
    let o_col = dialect.quote_ident(&pc.hop.obj_col);
    let (sf_s, sf_o, sf_d) = (
        dialect.quote_ident("sf_s"),
        dialect.quote_ident("sf_o"),
        dialect.quote_ident("sf_d"),
    );

    // Anchor: P+ = the one-hop pairs (depth 1); P* additionally = the reflexive
    // (x, x) pairs over every node appearing as a hop subject or object (depth 0).
    let anchor = if pc.reflexive {
        format!(
            "SELECT h.{s_col} AS {sf_s}, h.{s_col} AS {sf_o}, 0 AS {sf_d} FROM {src} h \
             UNION SELECT h.{o_col} AS {sf_s}, h.{o_col} AS {sf_o}, 0 AS {sf_d} FROM {src} h"
        )
    } else {
        format!("SELECT h.{s_col} AS {sf_s}, h.{o_col} AS {sf_o}, 1 AS {sf_d} FROM {src} h")
    };
    // Recursive step: extend a reached pair by one hop (raw-key equality), bounded.
    let recursive = format!(
        "SELECT c.{sf_s} AS {sf_s}, h.{o_col} AS {sf_o}, c.{sf_d} + 1 AS {sf_d} \
         FROM {cte_raw} c JOIN {src} h ON c.{sf_o} = h.{s_col} WHERE c.{sf_d} < {max}",
        max = pc.max_depth
    );

    let select_list = projection
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{} AS c{i}", colref(c, dialect, actuals)))
        .collect::<Vec<_>>()
        .join(", ");

    let distinct = if b.distinct { "DISTINCT " } else { "" };
    // The recursive closure keeps `sf_d` (so cycles/multi-depth pairs are NOT
    // collapsed there); a second non-recursive CTE reduces it to the DISTINCT
    // reachable node pairs — SPARQL `P+`/`P*` set-semantics — which the outer
    // projection reads as `t{alias}`.
    let mut skeleton = format!(
        "WITH RECURSIVE {cte_raw}({sf_s}, {sf_o}, {sf_d}) AS ({anchor} UNION {recursive}), \
         {cte}({sf_s}, {sf_o}) AS (SELECT DISTINCT {sf_s}, {sf_o} FROM {cte_raw}) \
         SELECT {distinct}{select_list} FROM {cte}"
    );
    if let Some(w) = render_where(&b.where_conds, dialect, actuals, &mut params, &mut pidx) {
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
