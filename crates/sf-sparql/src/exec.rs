//! Execute — run the emitted SQL on a live SQLite source, reconstruct `oxrdf`
//! bindings/triples, and stream results (ADR-0006 *Streaming & bounded memory*;
//! ADR-0007 step 7).
//!
//! The reconstruction is the **single term-gen path** (ADR-0003 R3): the SQL
//! projects raw key columns and `sf-core`'s `generate_into` materialises the RDF
//! term per output position — terms are built here, in the outermost projection,
//! never inside a join/filter (ADR-0007 lifting). Streaming uses `sf-sql`'s
//! bounded SQLite cursor ([`sf_sql::sqlite_for_each`]) — one row in flight, so
//! memory is independent of result size. CPU-bound term-gen belongs on the
//! dedicated rayon pool ([`crate::pool`]); the sync SQLite path here generates
//! inline (no async runtime to protect — ADR-0006).

use std::cmp::Ordering;
use std::collections::BTreeMap;

use rusqlite::types::ValueRef;
use rusqlite::Connection;
use sf_core::datatype::{self, XsdTypeCode};
use sf_core::ir::{TermMap, TermType};
use sf_core::{Literal, Row, Term, Triple};

use sf_core::ir::LogicalSource;
use sf_sql::Dialect;

use spargebra::algebra::{Expression, Function};

use crate::emit::{ColumnCatalog, EmittedBranch};
use crate::iq::{AggKind, Branch, ColRef, OrderKey, RustAgg, RustGroup, TermDef};
use crate::{Error, Plan, PlanForm, Result};

/// Introspect the actual result-column names of every source the plan reads, so
/// emission can resolve a mapping's regular-identifier column references to the
/// columns SQLite truly exposes (see [`crate::emit`]). A source whose metadata
/// cannot be read is simply omitted (resolution falls back to the raw identifier).
fn build_catalog(branches: &[Branch], conn: &Connection, dialect: Dialect) -> ColumnCatalog {
    let mut catalog = ColumnCatalog::default();
    let mut seen = std::collections::HashSet::new();
    for branch in branches {
        for (_, source) in branch.alias_sources() {
            let probe = match source {
                LogicalSource::Table(t) => format!("SELECT * FROM {}", dialect.quote_ident(t)),
                LogicalSource::Query(q) => q.clone(),
            };
            if !seen.insert(probe.clone()) {
                continue;
            }
            if let Ok(names) = sf_sql::sqlite_column_names(conn, &probe) {
                catalog.insert(source, names);
            }
        }
    }
    catalog
}

/// Each projected column's R2RML §10 natural XSD type from the emitted SQL's
/// **declared** result-column types (ADR-0015), in projection order. SQLite traces
/// decl types back through `rr:sqlQuery` views and derived tables, so this covers
/// base tables and views alike; a computed/expression column (`COUNT(…)`,
/// `a || b`) has no decl type — `None` — and falls back to its per-value storage
/// class at extraction ([`storage_class_code`]).
fn declared_codes(e: &EmittedBranch, conn: &Connection) -> Vec<Option<XsdTypeCode>> {
    match sf_sql::sqlite_column_decltypes(conn, &e.sql) {
        Ok(decltypes) => decltypes
            .iter()
            .map(|d| d.as_deref().and_then(datatype::natural_xsd))
            .collect(),
        Err(_) => vec![None; e.projection.len()],
    }
}

/// Each projected column's fixed `CHARACTER(n)` blank-pad length, in projection
/// order (`None` unless the column is a fixed-length char type with an explicit
/// length). A `CHARACTER(n)` value carries SQL fixed-length semantics — it is
/// space-padded to `n` (PostgreSQL does this; SQLite stores it unpadded). Capture
/// `n` so the extractor can right-pad SQLite's value, keeping the natural RDF
/// literal consistent across source dialects (ADR-0015 §10 consistency clause).
fn declared_char_pads(e: &EmittedBranch, conn: &Connection) -> Vec<Option<usize>> {
    match sf_sql::sqlite_column_decltypes(conn, &e.sql) {
        Ok(decltypes) => decltypes
            .iter()
            .map(|d| d.as_deref().and_then(char_pad_len))
            .collect(),
        Err(_) => vec![None; e.projection.len()],
    }
}

/// The fixed `CHARACTER(n)` pad length, if `decl` is a fixed-length char type
/// (`CHAR` / `CHARACTER` / `NCHAR`) with an explicit `(n)` — never a *varying*
/// type (`VARCHAR`, `CHARACTER VARYING`) and never an unsized one.
fn char_pad_len(decl: &str) -> Option<usize> {
    let open = decl.find('(')?;
    let close = decl[open..].find(')')? + open;
    let name: String = decl[..open]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase();
    if !matches!(name.as_str(), "CHAR" | "CHARACTER" | "NCHAR") {
        return None;
    }
    decl[open + 1..close].trim().parse::<usize>().ok()
}

/// The §10 type implied by a value's SQLite storage class — the affinity fallback
/// for a column with no declared type (ADR-0015 *SQLite — the special hazard*):
/// `INTEGER → xsd:integer`, `REAL → xsd:double`, `BLOB → xsd:hexBinary`; text /
/// NULL carry no implied type (plain literal).
fn storage_class_code(v: &ValueRef<'_>) -> Option<XsdTypeCode> {
    match v {
        ValueRef::Integer(_) => Some(XsdTypeCode::Integer),
        ValueRef::Real(_) => Some(XsdTypeCode::Double),
        ValueRef::Blob(_) => Some(XsdTypeCode::HexBinary),
        ValueRef::Text(_) | ValueRef::Null => None,
    }
}

/// One projected result row's raw column values plus each value's resolved §10
/// type (declared type, else storage-class fallback), addressed by [`ColRef`].
/// `pub(crate)` so the PostgreSQL executor ([`crate::exec_pg`]) drives the same
/// single term-gen path (ADR-0003 R3) with PG-extracted values.
pub(crate) struct RawRow<'a> {
    pub(crate) schema: &'a [ColRef],
    pub(crate) values: &'a [Option<String>],
    pub(crate) codes: &'a [Option<XsdTypeCode>],
}

impl RawRow<'_> {
    /// The resolved §10 XSD type of `column` under `alias`, if any.
    fn code_for(&self, alias: usize, column: &str) -> Option<XsdTypeCode> {
        self.schema
            .iter()
            .position(|c| c.alias == alias && &*c.column == column)
            .and_then(|i| self.codes[i])
    }
}

/// A view of a [`RawRow`] scoped to one scan alias, so a mapping term map's
/// column lookups resolve to that scan's projected columns ([`sf_core::Row`]).
struct AliasRow<'a> {
    raw: &'a RawRow<'a>,
    alias: usize,
}

impl Row for AliasRow<'_> {
    fn value(&self, column: &str) -> Option<&str> {
        self.raw
            .schema
            .iter()
            .position(|c| c.alias == self.alias && &*c.column == column)
            .and_then(|i| self.raw.values[i].as_deref())
    }
}

/// Materialise a term definition into an `oxrdf` term, or `None` if a referenced
/// column is NULL/absent (R2RML §11: no value ⇒ no term ⇒ unbound).
fn build_term(def: &TermDef, raw: &RawRow<'_>) -> Result<Option<Term>> {
    match def {
        TermDef::Const(t) => Ok(Some(t.clone())),
        TermDef::Derived { term_map, alias } => derived_term(term_map, *alias, raw),
        // R2 COALESCE: the preserved (left) side wins when bound; otherwise the
        // optional (right) value (ADR-0007). `None` from `left` = its source
        // columns were NULL (the optional did not match), so fall back to `right`.
        TermDef::Coalesce(l, r) => match build_term(l, raw)? {
            Some(t) => Ok(Some(t)),
            None => build_term(r, raw),
        },
        // BIND(CONCAT(…)) — SPARQL §17.4.5.4. Every operand must be a string literal
        // (xsd:string, simple, or lang-tagged); an unbound / IRI / blank-node operand
        // or a non-string *typed* literal is an expression error, so the BIND variable
        // is left unbound (Ok(None)) — never a wrong value. The result carries the
        // common language tag iff every operand shares it, else a simple literal.
        TermDef::Concat(parts) => {
            let mut s = String::new();
            let mut common_lang: Option<Option<String>> = None; // unset | mixed | lang
            for p in parts {
                let Some(Term::Literal(l)) = build_term(p, raw)? else {
                    return Ok(None);
                };
                let lang = l.language();
                if lang.is_none() && l.datatype() != sf_core::vocab::xsd::STRING {
                    return Ok(None); // a non-string typed literal ⇒ type error
                }
                s.push_str(l.value());
                let this = lang.map(str::to_owned);
                common_lang = Some(match common_lang {
                    None => this,                       // first operand sets it
                    Some(prev) if prev == this => prev, // still consistent
                    Some(_) => None,                    // diverged ⇒ no common tag
                });
            }
            let term = match common_lang.flatten() {
                Some(lang) => Literal::new_language_tagged_literal(s, lang)
                    .map_err(|e| Error::Core(e.to_string()))?,
                None => Literal::new_simple_literal(s),
            };
            Ok(Some(Term::Literal(term)))
        }
        // An aggregate result (SPARQL §11): the value is the SQL aggregate computed
        // at `col`. A NULL value is an empty multiset: SUM (and COUNT, defensively —
        // SQL `COUNT` never NULLs) over an empty multiset is `"0"^^xsd:integer`,
        // while AVG/MIN/MAX (and SAMPLE) are UNBOUND (§11). The §10 type is
        // `fixed_type` when the function pins it (COUNT ⇒ integer), else the
        // column's resolved decltype/storage class (SUM/MIN/MAX keep the source
        // numeric type). AVG (§11.4) follows the OPERAND numeric type under XPath
        // promotion — resolved from `operand`'s §10 type, since SQLite's `AVG`
        // always yields a REAL (the operand is projected bare on SQLite; on PG it is
        // absent and `avg()`'s own promoted result type is used).
        TermDef::Agg {
            col,
            kind,
            operand,
            fixed_type,
        } => {
            let row = AliasRow {
                raw,
                alias: col.alias,
            };
            let Some(value) = row.value(&col.column) else {
                return match kind {
                    AggKind::Sum | AggKind::Count => {
                        Ok(Some(natural_literal("0", XsdTypeCode::Integer)?))
                    }
                    AggKind::Avg | AggKind::Min | AggKind::Max => Ok(None),
                };
            };
            let code = match kind {
                AggKind::Avg => {
                    let operand_code = operand
                        .as_ref()
                        .and_then(|o| raw.code_for(o.alias, &o.column))
                        .or_else(|| raw.code_for(col.alias, &col.column))
                        .unwrap_or(XsdTypeCode::Decimal);
                    avg_result_code(operand_code)
                }
                _ => fixed_type
                    .or_else(|| raw.code_for(col.alias, &col.column))
                    .unwrap_or(XsdTypeCode::String),
            };
            Ok(Some(natural_literal(value, code)?))
        }
    }
}

/// Build a derived term, applying the R2RML §10 natural datatype mapping
/// (ADR-0015) when — and only when — the term map is a column-valued literal with
/// no explicit `rr:datatype` / `rr:language`. Templates, IRIs, blank nodes, and
/// explicitly-typed/lang-tagged literals go through the plain `sf-core` term-gen
/// path unchanged.
fn derived_term(term_map: &TermMap, alias: usize, raw: &RawRow<'_>) -> Result<Option<Term>> {
    if let TermMap::Column(col, spec) = term_map {
        if spec.term_type == TermType::Literal && spec.datatype.is_none() && spec.language.is_none()
        {
            let row = AliasRow { raw, alias };
            let Some(value) = row.value(col) else {
                return Ok(None);
            };
            let code = raw.code_for(alias, col).unwrap_or(XsdTypeCode::String);
            return Ok(Some(natural_literal(value, code)?));
        }
    }
    let row = AliasRow { raw, alias };
    sf_core::term::generate(term_map, &row).map_err(|e| Error::Core(e.to_string()))
}

/// Produce the RDF literal for a value under its §10 natural XSD type, in the
/// XSD-canonical lexical form (ADR-0015 chokepoint, `sf_core::datatype`).
/// `HexBinary` values arrive already uppercase-hex-encoded from blob extraction.
fn natural_literal(value: &str, code: XsdTypeCode) -> Result<Term> {
    let literal = match code {
        XsdTypeCode::String => Literal::new_simple_literal(value),
        XsdTypeCode::HexBinary => Literal::new_typed_literal(value, code.iri()),
        _ => {
            let mut buf = String::new();
            datatype::canonical_lexical(value, code, &mut buf)
                .map_err(|e| Error::Core(e.to_string()))?;
            Literal::new_typed_literal(buf, code.iri())
        }
    };
    Ok(Term::Literal(literal))
}

/// The §10 result datatype of `AVG(operand)` (SPARQL §11.4: AVG = SUM/COUNT under
/// XPath numeric type promotion). The result follows the operand numeric type:
/// `xsd:double` is preserved (so is `xsd:float`, which this codebase folds into
/// `xsd:double`); `xsd:integer` and `xsd:decimal` promote to `xsd:decimal`.
fn avg_result_code(operand: XsdTypeCode) -> XsdTypeCode {
    match operand {
        XsdTypeCode::Double => XsdTypeCode::Double,
        _ => XsdTypeCode::Decimal,
    }
}

/// Reconstruct all bound variables of `branch` for one raw row. `pub(crate)` so
/// the PostgreSQL executor reuses the identical reconstruction (ADR-0003 R3).
pub(crate) fn reconstruct(branch: &Branch, raw: &RawRow<'_>) -> Result<BTreeMap<String, Term>> {
    let mut out = BTreeMap::new();
    for (var, def) in &branch.bindings {
        if let Some(term) = build_term(def, raw)? {
            out.insert(var.clone(), term);
        }
    }
    Ok(out)
}

/// Read a SQLite value as its lexical string (NULL ⇒ `None`). Datatype
/// canonicalisation (R2RML §10) is `sf-core`'s concern and explicit `rr:datatype`
/// values pass through verbatim (ADR-0015); this is the raw lexical extraction.
fn lexical(v: ValueRef<'_>) -> Result<Option<String>> {
    Ok(match v {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Text(t) => Some(
            std::str::from_utf8(t)
                .map_err(|e| Error::Core(format!("non-UTF8 text column: {e}")))?
                .to_owned(),
        ),
        ValueRef::Blob(_) => {
            return Err(Error::Unsupported("BLOB column reconstruction".to_owned()))
        }
    })
}

/// Extract a column value with its target §10 type in view: a `BLOB` feeding an
/// `xsd:hexBinary` column is uppercase-hex-encoded here (where the raw bytes are
/// available — ADR-0015), so the literal builder sees its canonical lexical form;
/// every other storage class is read by [`lexical`]. A blob in a non-hexBinary
/// position is still unsupported (501).
fn lexical_typed(v: ValueRef<'_>, code: Option<XsdTypeCode>) -> Result<Option<String>> {
    if let ValueRef::Blob(bytes) = v {
        if code == Some(XsdTypeCode::HexBinary) {
            let mut out = String::new();
            datatype::hex_binary_upper(bytes, &mut out);
            return Ok(Some(out));
        }
    }
    lexical(v)
}

/// Iterate every WHERE solution across all branches, applying offset/limit in
/// Rust for the multi-branch (bag-union) case (the single-branch case pushes them
/// into SQL — see [`Plan::prepared_branches`]).
fn for_each_solution(
    plan: &Plan,
    conn: &Connection,
    mut sink: impl FnMut(&Branch, &BTreeMap<String, Term>) -> Result<()>,
) -> Result<()> {
    // Multi-branch GROUP BY: buffer all inner solutions, group and aggregate in
    // Rust, then stream the grouped result rows.
    if let Some(rg) = &plan.rust_group {
        return rust_group_execute(plan, conn, rg, &mut sink);
    }
    for_each_branch_solution(plan, conn, &mut sink)
}

/// Core branches loop — does NOT check `rust_group`. Called from both
/// `for_each_solution` (when no rust_group is set) and `rust_group_execute`
/// (to collect the inner solutions without triggering re-grouping).
fn for_each_branch_solution(
    plan: &Plan,
    conn: &Connection,
    sink: &mut impl FnMut(&Branch, &BTreeMap<String, Term>) -> Result<()>,
) -> Result<()> {
    let branches = plan.prepared_branches();
    let catalog = build_catalog(&branches, conn, plan.dialect);
    let multi = branches.len() > 1;
    // DISTINCT over a multi-branch bag-union: SQL dedups only *within* each branch's
    // SELECT, never *across* the separate per-branch SELECTs (UNION arms / VALUES
    // rows), so we dedup the projected solutions here — before OFFSET/LIMIT, since
    // SPARQL evaluates DISTINCT before slicing. The single-branch case pushes DISTINCT
    // into SQL (cascade pass 6 / the branch `distinct` flag); CONSTRUCT/dump never set
    // it. Bounded-memory caveat: DISTINCT inherently buffers the seen key set.
    let distinct_vars: Option<Vec<String>> = match (plan.distinct && multi, &plan.form) {
        (true, PlanForm::Select { vars }) => Some(vars.clone()),
        _ => None,
    };
    let mut seen_tuples: std::collections::HashSet<Vec<Option<Term>>> =
        std::collections::HashSet::new();
    let mut seen = 0usize; // solutions observed (for offset)
    let mut emitted = 0usize; // solutions passed downstream (for limit)
                              // ORDER BY is applied HERE for every plan (single- and multi-branch alike), never
                              // in SQL: a SQL `ORDER BY` inherits the column's collation/affinity, which can
                              // disagree with SPARQL value order (NOCASE text, non-C locale, temporal types).
                              // So buffer every (DISTINCT-deduped) solution, stable-sort by the keys via the
                              // type-aware `order_cmp`, then OFFSET/LIMIT (SPARQL §15: order, then slice).
                              // Bounded-memory caveat: ORDER BY inherently buffers — the one exception to the
                              // streaming contract (ADR-0006), memory grows with the result size.
    let ordered = !plan.order.is_empty();
    let mut buffer: Vec<(usize, BTreeMap<String, Term>)> = Vec::new();
    for (bi, branch) in branches.iter().enumerate() {
        let e = crate::emit::emit_branch_with(branch, plan.dialect, &catalog)?;
        let declared = declared_codes(&e, conn);
        let char_pads = declared_char_pads(&e, conn);
        let params = rusqlite::params_from_iter(e.params.iter());
        let mut err: Option<Error> = None;
        sf_sql::sqlite_for_each(conn, &e.sql, params, |row| {
            if err.is_some() {
                return Ok(());
            }
            let mut values = Vec::with_capacity(e.projection.len());
            let mut codes = Vec::with_capacity(e.projection.len());
            for (i, &decl_code) in declared.iter().enumerate() {
                let v = match row.get_ref(i).map_err(|e| Error::Sql(e.to_string())) {
                    Ok(v) => v,
                    Err(x) => {
                        err = Some(x);
                        return Ok(());
                    }
                };
                // §10 type: the declared decl type, else the value's storage class.
                let code = decl_code.or_else(|| storage_class_code(&v));
                match lexical_typed(v, code) {
                    Ok(mut text) => {
                        // R2RML §10 / ADR-0015: blank-pad a fixed-length CHAR(n)
                        // value to `n` so SQLite matches the SQL-standard value.
                        if let (Some(n), Some(s)) = (char_pads[i], text.as_mut()) {
                            for _ in s.chars().count()..n {
                                s.push(' ');
                            }
                        }
                        values.push(text);
                        codes.push(code);
                    }
                    Err(x) => {
                        err = Some(x);
                        return Ok(());
                    }
                }
            }
            let raw = RawRow {
                schema: &e.projection,
                values: &values,
                codes: &codes,
            };
            // Reconstruct first: DISTINCT needs the projected terms to dedup, and the
            // dedup must precede OFFSET/LIMIT (SPARQL order). Single-branch plans had
            // DISTINCT/OFFSET/LIMIT pushed into SQL, so they reconstruct + sink only.
            let bindings = match reconstruct(branch, &raw) {
                Ok(b) => b,
                Err(x) => {
                    err = Some(x);
                    return Ok(());
                }
            };
            // DISTINCT dedup only for a multi-branch bag-union (a single branch dedups
            // in SQL). Applied before ORDER BY.
            if multi {
                if let Some(vars) = &distinct_vars {
                    let key: Vec<Option<Term>> =
                        vars.iter().map(|v| bindings.get(v).cloned()).collect();
                    if !seen_tuples.insert(key) {
                        return Ok(()); // duplicate projected solution ⇒ not a new one
                    }
                }
            }
            // ORDER BY (any branch count): defer slicing — buffer for the global
            // type-aware sort after every row is read, so single- and multi-branch
            // order identically (OFFSET/LIMIT applied after the sort, below).
            // For expression keys (e.g. STRLEN(?n)), evaluate the expression now
            // and inject the result as a synthetic binding so order_cmp finds it.
            if ordered {
                let bindings = if plan.order.iter().any(|k| k.expr.is_some()) {
                    let mut b = bindings;
                    for key in &plan.order {
                        if let Some(expr) = &key.expr {
                            if let Some(val) = eval_expr(expr, &b) {
                                b.insert(key.var.clone(), val);
                            }
                        }
                    }
                    b
                } else {
                    bindings
                };
                buffer.push((bi, bindings));
                return Ok(());
            }
            // Streaming OFFSET/LIMIT only when SQL didn't apply them (a multi-branch
            // bag-union; a single unordered branch sliced in SQL).
            if multi {
                if seen < plan.offset {
                    seen += 1;
                    return Ok(());
                }
                if let Some(limit) = plan.limit {
                    if emitted >= limit {
                        return Ok(());
                    }
                }
            }
            emitted += 1;
            if let Err(x) = sink(branch, &bindings) {
                err = Some(x);
            }
            Ok(())
        })
        .map_err(map_sql_err)?;
        if let Some(x) = err {
            return Err(x);
        }
    }
    // The buffered bag-union ORDER BY: stable-sort by the keys (a stable sort keeps
    // the input bag order for equal keys), then OFFSET/LIMIT, then sink.
    if ordered {
        buffer.sort_by(|(_, a), (_, b)| order_cmp(&plan.order, a, b));
        let take = plan.limit.unwrap_or(usize::MAX);
        for (bi, bindings) in buffer.iter().skip(plan.offset).take(take) {
            sink(&branches[*bi], bindings)?;
        }
    }
    Ok(())
}

/// Compare two solutions by the ORDER BY keys (SPARQL §15.1), honoring each key's
/// direction with explicit UNBOUND placement: an unbound key sorts FIRST for ASC
/// and LAST for DESC — matching the SQL `NULLS FIRST/LAST` the single-branch path
/// emits, so single- and multi-branch orderings agree. Bound terms order
/// blank-node < IRI < literal; numeric-typed literals compare by value (so
/// xsd:integer 2 < 10, not lexical "10" < "2"). `pub(crate)` so the PostgreSQL
/// executor sorts its bag-union identically.
pub(crate) fn order_cmp(
    order: &[OrderKey],
    a: &BTreeMap<String, Term>,
    b: &BTreeMap<String, Term>,
) -> Ordering {
    for key in order {
        let ord = match (a.get(&key.var), b.get(&key.var)) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => {
                if key.descending {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            (Some(_), None) => {
                if key.descending {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
            (Some(x), Some(y)) => {
                let c = cmp_term(x, y);
                if key.descending {
                    c.reverse()
                } else {
                    c
                }
            }
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

/// SPARQL term order extended to a total order for sorting: blank node < IRI <
/// literal; within a kind by value.
fn cmp_term(a: &Term, b: &Term) -> Ordering {
    fn rank(t: &Term) -> u8 {
        match t {
            Term::BlankNode(_) => 0,
            Term::NamedNode(_) => 1,
            Term::Literal(_) => 2,
            _ => 3, // quoted triple (RDF-star) — sorts last, by lexical form
        }
    }
    match (a, b) {
        (Term::BlankNode(x), Term::BlankNode(y)) => x.as_str().cmp(y.as_str()),
        (Term::NamedNode(x), Term::NamedNode(y)) => x.as_str().cmp(y.as_str()),
        (Term::Literal(x), Term::Literal(y)) => cmp_literal(x, y),
        _ => rank(a)
            .cmp(&rank(b))
            .then_with(|| a.to_string().cmp(&b.to_string())),
    }
}

/// Compare two literals: numerically when both carry a numeric XSD datatype, else
/// by lexical value, then datatype IRI, then language tag.
fn cmp_literal(x: &Literal, y: &Literal) -> Ordering {
    if let (Some(nx), Some(ny)) = (numeric_value(x), numeric_value(y)) {
        return nx.partial_cmp(&ny).unwrap_or(Ordering::Equal);
    }
    x.value()
        .cmp(y.value())
        .then_with(|| x.datatype().as_str().cmp(y.datatype().as_str()))
        .then_with(|| x.language().unwrap_or("").cmp(y.language().unwrap_or("")))
}

/// The `f64` value of a numeric-XSD-typed literal, else `None` (a non-numeric
/// datatype is ordered lexically, never coerced).
fn numeric_value(l: &Literal) -> Option<f64> {
    const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
    let local = l.datatype().as_str().strip_prefix(XSD)?;
    let numeric = matches!(
        local,
        "integer"
            | "decimal"
            | "double"
            | "float"
            | "long"
            | "int"
            | "short"
            | "byte"
            | "nonNegativeInteger"
            | "nonPositiveInteger"
            | "negativeInteger"
            | "positiveInteger"
            | "unsignedLong"
            | "unsignedInt"
            | "unsignedShort"
            | "unsignedByte"
    );
    if numeric {
        l.value().parse::<f64>().ok()
    } else {
        None
    }
}

fn map_sql_err(e: sf_sql::Error) -> Error {
    Error::Sql(e.to_string())
}

// ---------------------------------------------------------------------------
// SPARQL expression evaluator for ORDER BY expression keys
// ---------------------------------------------------------------------------
// Evaluates a SPARQL expression against a solution binding map, returning the
// result as an RDF term, or `None` on type error / unbound input. Covers the
// subset needed for ORDER BY expression keys: arithmetic, string built-ins,
// IF, BOUND, comparisons, COALESCE, boolean connectives. On unsupported
// sub-expressions returns None (unbound), which ORDER BY treats as sorting
// first/last per direction — sound but never silently wrong.

/// Evaluate a SPARQL expression to an RDF term, or `None` if indeterminate.
pub(crate) fn eval_expr(expr: &Expression, b: &BTreeMap<String, Term>) -> Option<Term> {
    match expr {
        Expression::Variable(v) => b.get(v.as_str()).cloned(),
        Expression::NamedNode(n) => Some(Term::NamedNode(n.clone())),
        Expression::Literal(l) => Some(Term::Literal(l.clone())),
        Expression::Bound(v) => {
            let bound = b.contains_key(v.as_str());
            Some(Term::Literal(Literal::new_typed_literal(
                if bound { "true" } else { "false" },
                sf_core::NamedNode::new("http://www.w3.org/2001/XMLSchema#boolean").ok()?,
            )))
        }
        Expression::If(cond, then, els) => {
            if eval_bool(cond, b)? {
                eval_expr(then, b)
            } else {
                eval_expr(els, b)
            }
        }
        Expression::Coalesce(args) => args.iter().find_map(|a| eval_expr(a, b)),
        Expression::Not(a) => {
            let v = eval_bool(a, b)?;
            bool_literal(!v)
        }
        Expression::And(a, c) => {
            let av = eval_bool(a, b)?;
            let bv = eval_bool(c, b)?;
            bool_literal(av && bv)
        }
        Expression::Or(a, c) => {
            let av = eval_bool(a, b)?;
            let bv = eval_bool(c, b)?;
            bool_literal(av || bv)
        }
        Expression::Equal(a, c) => {
            let cmp = cmp_option(eval_expr(a, b).as_ref(), eval_expr(c, b).as_ref());
            bool_literal(matches!(cmp, Some(Ordering::Equal)))
        }
        Expression::Less(a, c) => bool_literal(matches!(
            cmp_option(eval_expr(a, b).as_ref(), eval_expr(c, b).as_ref()),
            Some(Ordering::Less)
        )),
        Expression::Greater(a, c) => bool_literal(matches!(
            cmp_option(eval_expr(a, b).as_ref(), eval_expr(c, b).as_ref()),
            Some(Ordering::Greater)
        )),
        Expression::LessOrEqual(a, c) => bool_literal(matches!(
            cmp_option(eval_expr(a, b).as_ref(), eval_expr(c, b).as_ref()),
            Some(Ordering::Less | Ordering::Equal)
        )),
        Expression::GreaterOrEqual(a, c) => bool_literal(matches!(
            cmp_option(eval_expr(a, b).as_ref(), eval_expr(c, b).as_ref()),
            Some(Ordering::Greater | Ordering::Equal)
        )),
        Expression::Add(a, c) => num_binop(eval_expr(a, b)?, eval_expr(c, b)?, |x, y| x + y),
        Expression::Subtract(a, c) => num_binop(eval_expr(a, b)?, eval_expr(c, b)?, |x, y| x - y),
        Expression::Multiply(a, c) => num_binop(eval_expr(a, b)?, eval_expr(c, b)?, |x, y| x * y),
        Expression::Divide(a, c) => {
            let bv = term_to_f64(&eval_expr(c, b)?)?;
            if bv == 0.0 {
                return None;
            }
            num_binop(
                eval_expr(a, b)?,
                Term::Literal(Literal::new_simple_literal("0")),
                |x, _| x / bv,
            )
        }
        Expression::UnaryMinus(a) => {
            let v = term_to_f64(&eval_expr(a, b)?)?;
            f64_to_term(-v)
        }
        Expression::FunctionCall(func, args) => eval_function(func, args, b),
        _ => None,
    }
}

fn eval_bool(expr: &Expression, b: &BTreeMap<String, Term>) -> Option<bool> {
    match eval_expr(expr, b)? {
        Term::Literal(l) => {
            const XSD_BOOL: &str = "http://www.w3.org/2001/XMLSchema#boolean";
            if l.datatype().as_str() == XSD_BOOL {
                Some(l.value() == "true")
            } else {
                // Effective boolean value per SPARQL §17.2.2
                let v = l.value();
                Some(!v.is_empty() && v != "0" && v != "0.0" && v != "false")
            }
        }
        _ => None,
    }
}

fn bool_literal(v: bool) -> Option<Term> {
    Some(Term::Literal(Literal::new_typed_literal(
        if v { "true" } else { "false" },
        sf_core::NamedNode::new("http://www.w3.org/2001/XMLSchema#boolean").ok()?,
    )))
}

fn term_to_f64(t: &Term) -> Option<f64> {
    match t {
        Term::Literal(l) => l.value().parse::<f64>().ok(),
        _ => None,
    }
}

fn f64_to_term(v: f64) -> Option<Term> {
    let code = if v.fract() == 0.0 && v.abs() < 1e15 {
        XsdTypeCode::Integer
    } else {
        XsdTypeCode::Double
    };
    natural_literal(&v.to_string(), code).ok()
}

fn num_binop(a: Term, b: Term, op: impl Fn(f64, f64) -> f64) -> Option<Term> {
    let av = term_to_f64(&a)?;
    let bv = term_to_f64(&b)?;
    f64_to_term(op(av, bv))
}

fn cmp_option(a: Option<&Term>, b: Option<&Term>) -> Option<Ordering> {
    Some(cmp_term(a?, b?))
}

fn eval_function(func: &Function, args: &[Expression], b: &BTreeMap<String, Term>) -> Option<Term> {
    fn str_val(t: &Term) -> Option<String> {
        match t {
            Term::Literal(l) => Some(l.value().to_owned()),
            _ => None,
        }
    }
    match func {
        Function::StrLen => {
            let t = eval_expr(args.first()?, b)?;
            let s = str_val(&t)?;
            natural_literal(&s.chars().count().to_string(), XsdTypeCode::Integer).ok()
        }
        Function::UCase => {
            let t = eval_expr(args.first()?, b)?;
            Some(Term::Literal(Literal::new_simple_literal(
                str_val(&t)?.to_uppercase(),
            )))
        }
        Function::LCase => {
            let t = eval_expr(args.first()?, b)?;
            Some(Term::Literal(Literal::new_simple_literal(
                str_val(&t)?.to_lowercase(),
            )))
        }
        Function::Str => {
            let t = eval_expr(args.first()?, b)?;
            let s = match &t {
                Term::Literal(l) => l.value().to_owned(),
                Term::NamedNode(n) => n.as_str().to_owned(),
                _ => return None,
            };
            Some(Term::Literal(Literal::new_simple_literal(s)))
        }
        Function::Concat => {
            let mut result = String::new();
            for arg in args {
                let t = eval_expr(arg, b)?;
                result.push_str(&str_val(&t)?);
            }
            Some(Term::Literal(Literal::new_simple_literal(result)))
        }
        Function::Lang => {
            let t = eval_expr(args.first()?, b)?;
            let lang = match &t {
                Term::Literal(l) => l.language().unwrap_or("").to_owned(),
                _ => String::new(),
            };
            Some(Term::Literal(Literal::new_simple_literal(lang)))
        }
        Function::Datatype => {
            let t = eval_expr(args.first()?, b)?;
            match &t {
                Term::Literal(l) => Some(Term::NamedNode(
                    sf_core::NamedNode::new(l.datatype().as_str()).ok()?,
                )),
                _ => None,
            }
        }
        Function::Abs => {
            let t = eval_expr(args.first()?, b)?;
            let v = term_to_f64(&t)?;
            f64_to_term(v.abs())
        }
        Function::Floor => {
            let t = eval_expr(args.first()?, b)?;
            let v = term_to_f64(&t)?;
            f64_to_term(v.floor())
        }
        Function::Ceil => {
            let t = eval_expr(args.first()?, b)?;
            let v = term_to_f64(&t)?;
            f64_to_term(v.ceil())
        }
        Function::Round => {
            let t = eval_expr(args.first()?, b)?;
            let v = term_to_f64(&t)?;
            f64_to_term(v.round())
        }
        _ => None,
    }
}

/// Stream the triples of a `CONSTRUCT` (or the `?s ?p ?o` dump), invoking `sink`
/// per well-formed triple. Ill-formed instantiations (e.g. a literal subject) are
/// skipped, per SPARQL CONSTRUCT semantics.
pub fn construct(plan: &Plan, conn: &Connection, mut sink: impl FnMut(Triple)) -> Result<u64> {
    let template = match &plan.form {
        PlanForm::Construct { template } => template.clone(),
        _ => {
            return Err(Error::Unsupported(
                "construct() requires a CONSTRUCT plan".to_owned(),
            ))
        }
    };
    let mut count = 0u64;
    for_each_solution(plan, conn, |_branch, bindings| {
        for tp in &template {
            if let Some(triple) = instantiate(tp, bindings) {
                count += 1;
                sink(triple);
            }
        }
        Ok(())
    })?;
    Ok(count)
}

/// Collect a CONSTRUCT's triples (test/diagnostic convenience; the streaming
/// [`construct`] is the bounded-memory API).
pub fn construct_triples(plan: &Plan, conn: &Connection) -> Result<Vec<Triple>> {
    let mut out = Vec::new();
    construct(plan, conn, |t| out.push(t))?;
    Ok(out)
}

/// Stream the whole mapping as **quads** (ADR-0005 named-graph conformance),
/// invoking `sink` per well-formed quad. Distinct from the `?s ?p ?o` CONSTRUCT
/// dump: it walks the mapping IR ([`crate::dump`]) so each triple carries the
/// graph term from the applicable `rr:graphMap`(s), built through the *same*
/// `sf-core` term-gen path (datatype §10 included — no drift). Bounded-memory:
/// one row in flight via [`for_each_solution`]. A triple whose subject/predicate/
/// object column is NULL is dropped (R2RML §11); a named-graph branch whose graph
/// map produces no value drops that quad (no silent default-graph fallback).
pub fn dump_quads_stream(
    maps: &[sf_core::ir::TriplesMap],
    conn: &Connection,
    dialect: sf_sql::Dialect,
    mut sink: impl FnMut(sf_core::Quad),
) -> Result<()> {
    use crate::dump::{VAR_G, VAR_O, VAR_P, VAR_S};
    use sf_core::GraphName;

    let plan = Plan {
        branches: crate::dump::build_branches(maps),
        form: PlanForm::Select { vars: Vec::new() }, // unused; we read bindings directly
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        rust_group: None,
        dialect,
    };
    for_each_solution(&plan, conn, |branch, bindings| {
        let (Some(s), Some(p), Some(o)) = (
            bindings.get(VAR_S),
            bindings.get(VAR_P),
            bindings.get(VAR_O),
        ) else {
            return Ok(()); // a NULL s/p/o column ⇒ no term ⇒ no triple (§11)
        };
        // A named-graph branch declares `g` in its bindings *definition*; the
        // reconstructed value is present only if the graph map produced a term.
        let graph = if branch.bindings.contains_key(VAR_G) {
            match bindings.get(VAR_G) {
                Some(Term::NamedNode(n)) => GraphName::NamedNode(n.clone()),
                _ => return Ok(()), // graph map yielded no value ⇒ drop this quad
            }
        } else {
            GraphName::DefaultGraph
        };
        if let Ok(triple) = Triple::from_terms(s.clone(), p.clone(), o.clone()) {
            sink(triple.in_graph(graph));
        }
        Ok(())
    })
}

/// Collect the mapping-IR quad dump (conformance convenience; the streaming
/// [`dump_quads_stream`] is the bounded-memory API).
pub fn dump_quads(
    maps: &[sf_core::ir::TriplesMap],
    conn: &Connection,
    dialect: sf_sql::Dialect,
) -> Result<Vec<sf_core::Quad>> {
    let mut out = Vec::new();
    dump_quads_stream(maps, conn, dialect, |q| out.push(q))?;
    Ok(out)
}

/// A SELECT solution: the projected variables (plan order) paired with each
/// row's bound terms (`None` = unbound).
pub struct Solutions {
    pub vars: Vec<String>,
    pub rows: Vec<Vec<Option<Term>>>,
}

/// Stream a SELECT's solutions, invoking `sink` per projected row (in projection
/// order, `None` = unbound) — one row in flight (bounded memory), the streaming
/// core behind [`select`]. The HTTP layer drives this to serialise + flush each
/// row into the response body without ever collecting the result set (ADR-0010
/// §C). The `&[Option<Term>]` slice is reused across rows (a fixed budget).
pub fn select_each(
    plan: &Plan,
    conn: &Connection,
    mut sink: impl FnMut(&[Option<Term>]) -> Result<()>,
) -> Result<()> {
    let vars = match &plan.form {
        PlanForm::Select { vars } => vars.clone(),
        _ => {
            return Err(Error::Unsupported(
                "select() requires a SELECT plan".to_owned(),
            ))
        }
    };
    let mut row: Vec<Option<Term>> = Vec::with_capacity(vars.len());
    for_each_solution(plan, conn, |_branch, bindings| {
        row.clear();
        row.extend(vars.iter().map(|v| bindings.get(v).cloned()));
        sink(&row)
    })
}

/// Execute a SELECT, collecting solutions (bounded-memory streaming is the
/// `for_each_solution` core; this collects for callers/tests).
pub fn select(plan: &Plan, conn: &Connection) -> Result<Solutions> {
    let vars = match &plan.form {
        PlanForm::Select { vars } => vars.clone(),
        _ => {
            return Err(Error::Unsupported(
                "select() requires a SELECT plan".to_owned(),
            ))
        }
    };
    let mut rows = Vec::new();
    for_each_solution(plan, conn, |_branch, bindings| {
        rows.push(vars.iter().map(|v| bindings.get(v).cloned()).collect());
        Ok(())
    })?;
    Ok(Solutions { vars, rows })
}

/// Execute an ASK — true iff at least one solution exists.
pub fn ask(plan: &Plan, conn: &Connection) -> Result<bool> {
    let mut any = false;
    for_each_solution(plan, conn, |_b, _s| {
        any = true;
        Ok(())
    })?;
    Ok(any)
}

/// Instantiate a CONSTRUCT-template triple against a solution; `None` if any
/// variable is unbound or the triple would be ill-formed. `pub(crate)` so the
/// PostgreSQL executor instantiates CONSTRUCT templates identically.
pub(crate) fn instantiate(
    tp: &spargebra::term::TriplePattern,
    bindings: &BTreeMap<String, Term>,
) -> Option<Triple> {
    use spargebra::term::{NamedNodePattern, TermPattern};
    let term = |p: &TermPattern| -> Option<Term> {
        match p {
            TermPattern::Variable(v) => bindings.get(v.as_str()).cloned(),
            TermPattern::NamedNode(n) => Some(Term::NamedNode(n.clone())),
            TermPattern::Literal(l) => Some(Term::Literal(l.clone())),
            TermPattern::BlankNode(b) => Some(Term::BlankNode(b.clone())),
            _ => None,
        }
    };
    let subject = term(&tp.subject)?;
    let predicate = match &tp.predicate {
        NamedNodePattern::NamedNode(n) => Term::NamedNode(n.clone()),
        NamedNodePattern::Variable(v) => bindings.get(v.as_str()).cloned()?,
    };
    let object = term(&tp.object)?;
    Triple::from_terms(subject, predicate, object).ok()
}

/// Serialise triples as N-Triples 1.2 (ADR-0019 G1: triple-term graphs serialise
/// as N-Triples/Turtle, not JSON-LD). One triple per line; streamed.
pub fn write_ntriples(triples: &[Triple]) -> String {
    let mut out = String::new();
    for t in triples {
        out.push_str(&t.to_string());
        out.push_str(" .\n");
    }
    out
}

// ---------------------------------------------------------------------------
// Rust-level GROUP BY (multi-branch inner, SPARQL §11)
// ---------------------------------------------------------------------------

/// Buffer all solutions from every inner branch, apply GROUP BY and aggregation
/// in Rust, and stream the resulting group rows.  Called by [`for_each_solution`]
/// when `plan.rust_group` is set (a UNION/VALUES inner that cannot be grouped in
/// SQL — see ADR-0007 and [`crate::unfold::Unfolder::group`]).
fn rust_group_execute(
    plan: &Plan,
    conn: &Connection,
    rg: &RustGroup,
    sink: &mut impl FnMut(&Branch, &BTreeMap<String, Term>) -> Result<()>,
) -> Result<()> {
    // Build a no-modifier plan for the inner execution (GROUP BY applies AFTER
    // collecting all inner solutions, so DISTINCT/OFFSET/LIMIT must not be
    // applied to the inner rows; ORDER BY is handled after grouping below).
    let inner_plan = Plan {
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        rust_group: None, // prevent recursion
        ..plan.clone()
    };

    // Collect all inner solutions (use for_each_branch_solution to avoid the
    // rust_group check — the inner_plan has rust_group: None, but calling
    // for_each_solution would create a recursive monomorphization that the
    // compiler cannot handle; for_each_branch_solution is the non-recursive core).
    let mut inner_rows: Vec<BTreeMap<String, Term>> = Vec::new();
    for_each_branch_solution(&inner_plan, conn, &mut |_, bindings| {
        inner_rows.push(bindings.clone());
        Ok(())
    })?;

    // Group by the key variable values, preserving insertion order for stable output.
    // Use a Vec for ordering + a HashMap for O(1) group lookup.
    type GroupKey = Vec<Option<Term>>;
    type GroupRows = Vec<BTreeMap<String, Term>>;
    #[allow(clippy::type_complexity)]
    let mut groups: Vec<(GroupKey, GroupRows)> = Vec::new();
    let mut key_index: std::collections::HashMap<Vec<Option<Term>>, usize> =
        std::collections::HashMap::new();

    for row in inner_rows {
        let key: Vec<Option<Term>> = rg.keys.iter().map(|k| row.get(k).cloned()).collect();
        if let Some(&idx) = key_index.get(&key) {
            groups[idx].1.push(row);
        } else {
            let idx = groups.len();
            key_index.insert(key.clone(), idx);
            groups.push((key, vec![row]));
        }
    }

    // Implicit grouping (no key variables): always produce exactly one group,
    // even over an empty inner (COUNT(*) ⇒ 0, AVG/MIN/MAX ⇒ UNBOUND — §11).
    if rg.keys.is_empty() && groups.is_empty() {
        groups.push((vec![], vec![]));
    }

    // Choose a dummy branch for the sink call (SELECT uses `_branch` only for
    // CONSTRUCT template lookup; GROUP BY results do not come from a single branch).
    let dummy = plan.branches.first().cloned().unwrap_or_else(Branch::empty);

    // Apply ORDER BY (if requested) over the grouped result rows.
    if !plan.order.is_empty() {
        // Build the result rows first, then sort.
        let mut result_rows: Vec<BTreeMap<String, Term>> = Vec::new();
        for (key_vals, group_rows) in &groups {
            let mut result = BTreeMap::new();
            for (k, val) in rg.keys.iter().zip(key_vals.iter()) {
                if let Some(t) = val {
                    result.insert(k.clone(), t.clone());
                }
            }
            for agg_spec in &rg.aggs {
                if let Some(t) = rust_agg(agg_spec, group_rows)? {
                    result.insert(agg_spec.out_var.clone(), t);
                }
            }
            result_rows.push(result);
        }
        result_rows.sort_by(|a, b| order_cmp(&plan.order, a, b));
        let take = plan.limit.unwrap_or(usize::MAX);
        for result in result_rows.into_iter().skip(plan.offset).take(take) {
            sink(&dummy, &result)?;
        }
        return Ok(());
    }

    // Streaming path (no ORDER BY): OFFSET/LIMIT applied row by row.
    let mut seen = 0usize;
    let mut emitted = 0usize;
    for (key_vals, group_rows) in &groups {
        if seen < plan.offset {
            seen += 1;
            continue;
        }
        if let Some(limit) = plan.limit {
            if emitted >= limit {
                break;
            }
        }
        let mut result = BTreeMap::new();
        for (k, val) in rg.keys.iter().zip(key_vals.iter()) {
            if let Some(t) = val {
                result.insert(k.clone(), t.clone());
            }
        }
        for agg_spec in &rg.aggs {
            if let Some(t) = rust_agg(agg_spec, group_rows)? {
                result.insert(agg_spec.out_var.clone(), t);
            }
        }
        emitted += 1;
        sink(&dummy, &result)?;
    }
    Ok(())
}

/// Compute one aggregate over a group of solutions. Returns `None` for
/// UNBOUND (AVG/MIN/MAX over an empty multiset — SPARQL §11).
fn rust_agg(agg: &RustAgg, rows: &[BTreeMap<String, Term>]) -> Result<Option<Term>> {
    // Collect bound numeric values of the argument variable.
    let _bound_vals: Vec<&Term> = match &agg.arg_var {
        None => rows.iter().flat_map(|r| r.values()).collect(), // COUNT(*) — not used for numerics
        Some(var) => rows.iter().filter_map(|r| r.get(var)).collect(),
    };

    match agg.kind {
        AggKind::Count => {
            let count = match &agg.arg_var {
                None => rows.len(), // COUNT(*)
                Some(var) => {
                    if agg.distinct {
                        let mut seen: std::collections::HashSet<String> =
                            std::collections::HashSet::new();
                        rows.iter()
                            .filter_map(|r| r.get(var))
                            .filter(|t| seen.insert(t.to_string()))
                            .count()
                    } else {
                        rows.iter().filter(|r| r.contains_key(var.as_str())).count()
                    }
                }
            };
            Ok(Some(Term::Literal(Literal::new_typed_literal(
                count.to_string(),
                sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
            ))))
        }
        AggKind::Sum => {
            let Some(var) = &agg.arg_var else {
                return Ok(None);
            };
            let vals: Vec<&Term> = rows.iter().filter_map(|r| r.get(var)).collect();
            if vals.is_empty() {
                // SUM over empty multiset ⇒ "0"^^xsd:integer (SPARQL §11).
                return Ok(Some(Term::Literal(Literal::new_typed_literal(
                    "0",
                    sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
                ))));
            }
            let nums: Vec<f64> = vals.iter().filter_map(|t| numeric_term(t)).collect();
            if nums.len() < vals.len() {
                return Ok(None); // non-numeric operand ⇒ UNBOUND (type error)
            }
            let sum: f64 = nums.iter().sum();
            if vals.iter().all(|t| is_xsd_integer(t)) {
                Ok(Some(integer_term(sum as i64)))
            } else {
                Ok(Some(decimal_term(sum)))
            }
        }
        AggKind::Avg => {
            let Some(var) = &agg.arg_var else {
                return Ok(None);
            };
            let vals: Vec<&Term> = rows.iter().filter_map(|r| r.get(var)).collect();
            if vals.is_empty() {
                // AVG over an empty multiset ⇒ "0"^^xsd:integer (SPARQL §11, like SUM —
                // NOT UNBOUND; the spareval oracle confirms 0).
                return Ok(Some(integer_term(0)));
            }
            let nums: Vec<f64> = vals.iter().filter_map(|t| numeric_term(t)).collect();
            if nums.is_empty() {
                return Ok(None); // non-numeric operand ⇒ UNBOUND (type error, §11)
            }
            let avg = nums.iter().sum::<f64>() / nums.len() as f64;
            Ok(Some(decimal_term(avg)))
        }
        AggKind::Min | AggKind::Max => {
            let Some(var) = &agg.arg_var else {
                return Ok(None);
            };
            let vals: Vec<&Term> = rows.iter().filter_map(|r| r.get(var)).collect();
            if vals.is_empty() {
                return Ok(None); // UNBOUND for empty multiset (§11)
            }
            let result = if agg.kind == AggKind::Min {
                vals.iter().min_by(|a, b| cmp_term(a, b))
            } else {
                vals.iter().max_by(|a, b| cmp_term(a, b))
            };
            Ok(result.map(|t| (*t).clone()))
        }
    }
}

/// Extract the `f64` numeric value of an RDF term (returns `None` for
/// non-numeric-typed literals and non-literals).
fn numeric_term(t: &Term) -> Option<f64> {
    match t {
        Term::Literal(l) => numeric_value(l),
        _ => None,
    }
}

/// Whether an RDF term is an `xsd:integer`-typed literal.
fn is_xsd_integer(t: &Term) -> bool {
    match t {
        Term::Literal(l) => l.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#integer",
        _ => false,
    }
}

/// Build an `xsd:integer` literal from an `i64`.
fn integer_term(n: i64) -> Term {
    Term::Literal(Literal::new_typed_literal(
        n.to_string(),
        sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
    ))
}

/// Build an `xsd:decimal` literal from an `f64`.
fn decimal_term(n: f64) -> Term {
    // Use a compact decimal representation (avoid scientific notation).
    let s = if n.fract() == 0.0 {
        format!("{n:.1}")
    } else {
        format!("{n}")
    };
    Term::Literal(Literal::new_typed_literal(
        s,
        sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#decimal"),
    ))
}
