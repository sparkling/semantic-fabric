//! `exec_core` — the driver-agnostic execution core (ADR-0024). One async pull-cursor
//! loop generic over [`SqlBackend`], plus the per-form entry points. The SQLite entry
//! points in [`crate::exec`] delegate here through a `block_on` shim; PostgreSQL /
//! MySQL keep their parallel loops until M3/M4 (design §5).
//!
//! The term-gen helpers (`reconstruct` / `order_cmp` / `eval_expr` / `instantiate` /
//! `Solutions` / `rust_group_result_rows` / `RawRow` and their private helpers) are
//! physically single-homed **here** (ADR-0024 M5, design §2). [`crate::exec`]
//! re-exports `Solutions`; the PostgreSQL / MySQL executors import it by that path.
//!
//! The corrected per-branch sequence (design §2, mirroring the old `exec.rs`
//! SQLite loop): reconstruct → DISTINCT dedup (before slice) → if ordered {buffer,
//! defer} else {streaming OFFSET/LIMIT} → after the loop: sort THEN slice.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::future::Future;

use sf_core::datatype::{self, XsdTypeCode};
use sf_core::ir::{TermMap, TermType};
use sf_core::{Literal, Row, Term, Triple};
use sf_sql::{BranchStream, Dialect, SqlBackend};
use spargebra::algebra::{Expression, Function};

use crate::emit::{self, ColumnCatalog};
use crate::iq::{AggKind, Branch, ColRef, OrderKey, RustAgg, RustGroup, TermDef};
use crate::{Error, Plan, PlanForm, Result};

/// Flatten an `sf-sql` driver error's source chain into the message (the SQLite
/// chain is usually empty, so this is byte-identical to the old `Error::Sql(e)`).
fn map_sql_err(e: sf_sql::Error) -> Error {
    use std::error::Error as _;
    // An uncovered PG result type (adapter `pg_value`) is preserved as a distinct
    // 501 skip — byte-identical to the pre-M3 `exec_pg` path, which returned
    // `sf_sparql::Error::Unsupported` directly from `pg_value` (never `Sql`).
    if let sf_sql::Error::Unsupported(m) = &e {
        return Error::Unsupported(m.clone());
    }
    let mut msg = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        msg.push_str(": ");
        msg.push_str(&s.to_string());
        src = s.source();
    }
    Error::Sql(msg)
}

/// Drive an always-ready future to completion with no runtime (design §5 M2
/// sync↔async bridge). The SQLite backend's stream is fully synchronous, so every
/// `.await` resolves `Ready` immediately; a `noop` waker + poll loop needs no tokio
/// runtime and therefore never nests / panics inside `sf-serve`'s `spawn_blocking`.
pub(crate) fn block_on<F: Future>(fut: F) -> F::Output {
    use std::task::{Context, Poll, Waker};
    let mut cx = Context::from_waker(Waker::noop());
    let mut fut = std::pin::pin!(fut);
    loop {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

/// Evaluate ORDER BY expression keys (e.g. `STRLEN(?n)`) and inject each result as a
/// synthetic binding so `order_cmp` finds it (design §2 — the extraction of the old
/// SQLite-only `exec.rs` injection, now backend-uniform).
fn inject_order_expr_keys(
    order: &[OrderKey],
    bindings: BTreeMap<String, Term>,
) -> BTreeMap<String, Term> {
    if order.iter().any(|k| k.expr.is_some()) {
        let mut b = bindings;
        for key in order {
            if let Some(expr) = &key.expr {
                if let Some(val) = eval_expr(expr, &b) {
                    b.insert(key.var.clone(), val);
                }
            }
        }
        b
    } else {
        bindings
    }
}

/// Iterate every WHERE solution across all branches; dispatch the multi-branch
/// GROUP BY (`rust_group`) to the buffered path, else the streaming branches loop.
async fn for_each_solution<B, F, Fut>(plan: &Plan, b: &mut B, sink: F) -> Result<()>
where
    B: SqlBackend,
    F: FnMut(&Branch, &BTreeMap<String, Term>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    if let Some(rg) = &plan.rust_group {
        return rust_group_execute(plan, b, rg, sink).await;
    }
    for_each_branch_solution(plan, b, sink).await
}

/// Core branches loop — does NOT check `rust_group` (the non-recursive split, so
/// `rust_group_execute` can reuse it to collect inner solutions). One row in flight.
async fn for_each_branch_solution<B, F, Fut>(plan: &Plan, b: &mut B, mut sink: F) -> Result<()>
where
    B: SqlBackend,
    F: FnMut(&Branch, &BTreeMap<String, Term>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let branches = plan.prepared_branches();
    // Catalog: one probe per distinct source; SWALLOW column_names errors (design
    // A4 — a source whose metadata cannot be read is omitted; resolution falls back
    // to the raw identifier).
    let mut catalog = ColumnCatalog::default();
    let mut seen_probe = std::collections::HashSet::new();
    for branch in &branches {
        for (_, source) in branch.alias_sources() {
            let probe = plan.dialect.probe_sql(source);
            if !seen_probe.insert(probe.clone()) {
                continue;
            }
            if let Ok(names) = b.column_names(&probe).await {
                catalog.insert(source, names);
            }
        }
    }
    let multi = branches.len() > 1;
    // DISTINCT over a multi-branch bag-union: SQL dedups only within each branch, so
    // dedup the projected solutions here — before OFFSET/LIMIT (SPARQL evaluates
    // DISTINCT before slicing). The single-branch case pushes DISTINCT into SQL.
    let distinct_vars: Option<Vec<String>> = match (plan.distinct && multi, &plan.form) {
        (true, PlanForm::Select { vars }) => Some(vars.clone()),
        _ => None,
    };
    let mut seen_tuples: std::collections::HashSet<Vec<Option<Term>>> =
        std::collections::HashSet::new();
    let mut seen = 0usize; // solutions observed (for offset)
    let mut emitted = 0usize; // solutions passed downstream (for limit)
                              // ORDER BY is applied HERE for every plan, never in SQL (a SQL ORDER BY inherits
                              // the column's collation/affinity). Buffer, stable-sort via the type-aware
                              // order_cmp, then OFFSET/LIMIT (SPARQL §15: order, then slice).
    let ordered = !plan.order.is_empty();
    let mut buffer: Vec<(usize, BTreeMap<String, Term>)> = Vec::new();
    for (bi, branch) in branches.iter().enumerate() {
        let e = emit::emit_branch_with(branch, plan.dialect, &catalog)?;
        // The ONLY bind site: `e.params` bound as N positional params by the adapter.
        let mut s = b
            .open_branch(&e.sql, &e.params)
            .await
            .map_err(map_sql_err)?;
        while let Some(t) = s.next_row().await.map_err(map_sql_err)? {
            let raw = RawRow {
                schema: &e.projection,
                values: &t.values,
                codes: &t.codes,
            };
            // Reconstruct first: DISTINCT needs the projected terms, and dedup must
            // precede OFFSET/LIMIT (SPARQL order).
            let bindings = reconstruct(branch, &raw)?;
            if multi {
                if let Some(vars) = &distinct_vars {
                    let key: Vec<Option<Term>> =
                        vars.iter().map(|v| bindings.get(v).cloned()).collect();
                    if !seen_tuples.insert(key) {
                        continue; // duplicate projected solution
                    }
                }
            }
            // ORDER BY (any branch count): defer slicing — buffer for the global
            // type-aware sort after every row (OFFSET/LIMIT applied after the sort).
            if ordered {
                let bindings = inject_order_expr_keys(&plan.order, bindings);
                buffer.push((bi, bindings));
                continue;
            }
            // Streaming OFFSET/LIMIT only when SQL didn't apply them (a multi-branch
            // bag-union; a single unordered branch sliced in SQL).
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
    // The buffered bag-union ORDER BY: stable-sort by the keys, then OFFSET/LIMIT.
    if ordered {
        buffer.sort_by(|(_, a), (_, b)| order_cmp(&plan.order, a, b));
        let take = plan.limit.unwrap_or(usize::MAX);
        for (bi, bindings) in buffer.iter().skip(plan.offset).take(take) {
            sink(&branches[*bi], bindings).await?;
        }
    }
    Ok(())
}

/// Multi-branch GROUP BY: collect every inner solution (no DISTINCT/OFFSET/LIMIT/
/// ORDER on the inner — those apply AFTER grouping), then group + aggregate + slice
/// via the backend-independent [`rust_group_result_rows`] and stream the grouped
/// rows. Uses the non-recursive [`for_each_branch_solution`] to avoid a recursive
/// `async fn` monomorphization (design §2).
async fn rust_group_execute<B, F, Fut>(
    plan: &Plan,
    b: &mut B,
    rg: &RustGroup,
    mut sink: F,
) -> Result<()>
where
    B: SqlBackend,
    F: FnMut(&Branch, &BTreeMap<String, Term>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let inner_plan = Plan {
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        rust_group: None, // prevent recursion
        ..plan.clone()
    };
    let mut inner_rows: Vec<BTreeMap<String, Term>> = Vec::new();
    for_each_branch_solution(&inner_plan, b, |_, bindings| {
        inner_rows.push(bindings.clone());
        std::future::ready(Ok(()))
    })
    .await?;

    let dummy = plan.branches.first().cloned().unwrap_or_else(Branch::empty);
    for result in rust_group_result_rows(plan, rg, inner_rows)? {
        sink(&dummy, &result).await?;
    }
    Ok(())
}

// --- per-form entry points (generic over B) ----------------------------------

/// Execute a SELECT, collecting solutions.
pub async fn select<B: SqlBackend>(plan: &Plan, b: &mut B) -> Result<Solutions> {
    let vars = match &plan.form {
        PlanForm::Select { vars } => vars.clone(),
        _ => {
            return Err(Error::Unsupported(
                "select() requires a SELECT plan".to_owned(),
            ))
        }
    };
    let mut rows = Vec::new();
    for_each_solution(plan, b, |_branch, bindings| {
        rows.push(vars.iter().map(|v| bindings.get(v).cloned()).collect());
        std::future::ready(Ok(()))
    })
    .await?;
    Ok(Solutions { vars, rows })
}

/// Stream a SELECT's solutions, invoking `sink` per projected row (one row in flight).
pub async fn select_each<B, S>(plan: &Plan, b: &mut B, mut sink: S) -> Result<()>
where
    B: SqlBackend,
    S: FnMut(&[Option<Term>]) -> Result<()>,
{
    let vars = match &plan.form {
        PlanForm::Select { vars } => vars.clone(),
        _ => {
            return Err(Error::Unsupported(
                "select() requires a SELECT plan".to_owned(),
            ))
        }
    };
    let mut row: Vec<Option<Term>> = Vec::with_capacity(vars.len());
    for_each_solution(plan, b, |_branch, bindings| {
        row.clear();
        row.extend(vars.iter().map(|v| bindings.get(v).cloned()));
        std::future::ready(sink(&row))
    })
    .await
}

/// Stream a SELECT's solutions into an ASYNC sink (per projected row, plan order,
/// `None` = unbound). The PostgreSQL/MySQL serve-lane form: `sink(..).await`
/// backpressures the server-side cursor (ADR-0006 / ADR-0010 §C). SQLite's serve
/// lane keeps the sync [`select_each`]. Written once over the shared core, so it
/// inherits rust_group / DISTINCT / ORDER / OFFSET / LIMIT.
pub async fn select_each_async<B, F, Fut>(plan: &Plan, b: &mut B, mut sink: F) -> Result<()>
where
    B: SqlBackend + Send,
    for<'s> B::Stream<'s>: Send,
    F: FnMut(Vec<Option<Term>>) -> Fut + Send,
    Fut: Future<Output = Result<()>> + Send,
{
    let vars = match &plan.form {
        PlanForm::Select { vars } => vars.clone(),
        _ => {
            return Err(Error::Unsupported(
                "select() requires a SELECT plan".to_owned(),
            ))
        }
    };
    for_each_solution(plan, b, |_branch, bindings| {
        let row: Vec<Option<Term>> = vars.iter().map(|v| bindings.get(v).cloned()).collect();
        sink(row)
    })
    .await
}

/// Stream a CONSTRUCT's per-solution triples into an ASYNC sink (bounded by the
/// template size — never the whole graph). The PostgreSQL/MySQL serve-lane form of
/// [`construct`], written once over the shared core.
pub async fn construct_each_async<B, F, Fut>(plan: &Plan, b: &mut B, mut sink: F) -> Result<()>
where
    B: SqlBackend + Send,
    for<'s> B::Stream<'s>: Send,
    F: FnMut(Vec<Triple>) -> Fut + Send,
    Fut: Future<Output = Result<()>> + Send,
{
    let template = match &plan.form {
        PlanForm::Construct { template } => template.clone(),
        _ => {
            return Err(Error::Unsupported(
                "construct() requires a CONSTRUCT plan".to_owned(),
            ))
        }
    };
    for_each_solution(plan, b, |_branch, bindings| {
        let triples: Vec<Triple> = template
            .iter()
            .filter_map(|tp| instantiate(tp, bindings))
            .collect();
        sink(triples)
    })
    .await
}

/// Execute an ASK — true iff at least one solution exists.
pub async fn ask<B: SqlBackend>(plan: &Plan, b: &mut B) -> Result<bool> {
    let mut any = false;
    for_each_solution(plan, b, |_b, _s| {
        any = true;
        std::future::ready(Ok(()))
    })
    .await?;
    Ok(any)
}

/// Stream the triples of a CONSTRUCT (or the `?s ?p ?o` dump), invoking `sink` per
/// well-formed triple; ill-formed instantiations are skipped. Returns the count.
pub async fn construct<B, S>(plan: &Plan, b: &mut B, mut sink: S) -> Result<u64>
where
    B: SqlBackend,
    S: FnMut(Triple),
{
    let template = match &plan.form {
        PlanForm::Construct { template } => template.clone(),
        _ => {
            return Err(Error::Unsupported(
                "construct() requires a CONSTRUCT plan".to_owned(),
            ))
        }
    };
    let mut count = 0u64;
    for_each_solution(plan, b, |_branch, bindings| {
        for tp in &template {
            if let Some(triple) = instantiate(tp, bindings) {
                count += 1;
                sink(triple);
            }
        }
        std::future::ready(Ok(()))
    })
    .await?;
    Ok(count)
}

/// Collect a CONSTRUCT's triples (test/diagnostic convenience).
pub async fn construct_triples<B: SqlBackend>(plan: &Plan, b: &mut B) -> Result<Vec<Triple>> {
    let mut out = Vec::new();
    construct(plan, b, |t| out.push(t)).await?;
    Ok(out)
}

/// Stream the whole mapping as **quads** (ADR-0005), invoking `sink` per well-formed
/// quad — each triple carries the graph term from the applicable `rr:graphMap`(s).
pub async fn dump_quads_stream<B, S>(
    maps: &[sf_core::ir::TriplesMap],
    b: &mut B,
    dialect: Dialect,
    mut sink: S,
) -> Result<()>
where
    B: SqlBackend,
    S: FnMut(sf_core::Quad),
{
    use crate::dump::{VAR_G, VAR_O, VAR_P, VAR_S};
    use sf_core::GraphName;

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
    for_each_solution(&plan, b, |branch, bindings| {
        let quad = (|| {
            let (Some(s), Some(p), Some(o)) = (
                bindings.get(VAR_S),
                bindings.get(VAR_P),
                bindings.get(VAR_O),
            ) else {
                return None; // a NULL s/p/o column ⇒ no term ⇒ no triple (§11)
            };
            let graph = if branch.bindings.contains_key(VAR_G) {
                match bindings.get(VAR_G) {
                    Some(Term::NamedNode(n)) => GraphName::NamedNode(n.clone()),
                    _ => return None, // graph map yielded no value ⇒ drop this quad
                }
            } else {
                GraphName::DefaultGraph
            };
            Triple::from_terms(s.clone(), p.clone(), o.clone())
                .ok()
                .map(|t| t.in_graph(graph))
        })();
        if let Some(q) = quad {
            sink(q);
        }
        std::future::ready(Ok(()))
    })
    .await
}

/// Collect the mapping-IR quad dump (conformance convenience).
pub async fn dump_quads<B: SqlBackend>(
    maps: &[sf_core::ir::TriplesMap],
    b: &mut B,
    dialect: Dialect,
) -> Result<Vec<sf_core::Quad>> {
    let mut out = Vec::new();
    dump_quads_stream(maps, b, dialect, |q| out.push(q)).await?;
    Ok(out)
}

// --- single-homed term-gen helpers (relocated from exec.rs, ADR-0024 M5) ------

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
                // A NULL SQL aggregate value on the single-branch SQL-pushdown path. ADR-0025
                // C.7: SUM/AVG/COUNT over an EMPTY group ⇒ "0"^^xsd:integer (SPARQL §11); only
                // MIN/MAX of an empty multiset are UNBOUND. This is sound HERE specifically
                // because ADR-0025 C.6 routes any NULLABLE-operand aggregate to `rust_group` —
                // so on this SQL path the operand is MANDATORY (bound in every row), hence a
                // NULL aggregate value means 0 rows (empty group), never "non-empty but all
                // operands unbound" (which must be UNBOUND and is handled correctly by
                // `rust_agg` C.4/C.5). Pre-C.6 this branch conflated the two for AVG.
                return match kind {
                    AggKind::Sum | AggKind::Count | AggKind::Avg => {
                        Ok(Some(natural_literal("0", XsdTypeCode::Integer)?))
                    }
                    AggKind::Min | AggKind::Max => Ok(None),
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

/// A SELECT solution: the projected variables (plan order) paired with each
/// row's bound terms (`None` = unbound).
pub struct Solutions {
    pub vars: Vec<String>,
    pub rows: Vec<Vec<Option<Term>>>,
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

// ---------------------------------------------------------------------------
// Rust-level GROUP BY (multi-branch inner, SPARQL §11)
// ---------------------------------------------------------------------------

/// Group a collected multiset of inner solutions by `rg.keys`, compute each
/// aggregate in `rg.aggs`, then apply the plan's ORDER BY + OFFSET/LIMIT to the
/// grouped rows (SPARQL §15: order, then slice). Returns the final result rows in
/// emit order.
///
/// Shared by the SQLite ([`rust_group_execute`]) and PostgreSQL
/// ([`crate::exec_pg`]) multi-branch GROUP BY paths (ADR-0007): the
/// grouping/aggregation semantics are backend-independent — only the collection of
/// the inner solutions (SQLite `Connection` vs live PostgreSQL cursor) differs.
pub(crate) fn rust_group_result_rows(
    plan: &Plan,
    rg: &RustGroup,
    inner_rows: Vec<BTreeMap<String, Term>>,
) -> Result<Vec<BTreeMap<String, Term>>> {
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

    // Materialise the result row (key vars + aggregates) for every group.
    let mut result_rows: Vec<BTreeMap<String, Term>> = Vec::with_capacity(groups.len());
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
        // ADR-0025 Tier-2 gap 5: post-GROUP-BY expressions over the aggregate outputs
        // (e.g. `COUNT(?x) * 2`). Evaluate each over the row's now-materialised aggregate +
        // group-key bindings via the shared `eval_expr`; an unbound reference yields no
        // binding (SPARQL: the value is unbound), never a wrong answer.
        for (out_var, expr) in &rg.post_exprs {
            if let Some(t) = eval_expr(expr, &result) {
                result.insert(out_var.clone(), t);
            }
        }
        result_rows.push(result);
    }

    // ORDER BY over the grouped rows (if requested), then OFFSET/LIMIT.
    if !plan.order.is_empty() {
        result_rows.sort_by(|a, b| order_cmp(&plan.order, a, b));
    }
    let take = plan.limit.unwrap_or(usize::MAX);
    Ok(result_rows
        .into_iter()
        .skip(plan.offset)
        .take(take)
        .collect())
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
                // COUNT(DISTINCT *) — count DISTINCT whole solutions in the group. A row's
                // canonical key is its (var, term) pairs; `BTreeMap` iterates in sorted key
                // order, so the key is order-independent. Uses the same `Term::to_string`
                // canonicalisation as COUNT(DISTINCT ?v) below (ADR-0025 Tier-2 gap 3).
                None if agg.distinct => {
                    let mut seen: std::collections::HashSet<Vec<(String, String)>> =
                        std::collections::HashSet::new();
                    rows.iter()
                        .filter(|r| {
                            let key: Vec<(String, String)> =
                                r.iter().map(|(k, v)| (k.clone(), v.to_string())).collect();
                            seen.insert(key)
                        })
                        .count()
                }
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
            // ADR-0025 C.5: SUM/AVG/MIN/MAX PROPAGATE an unbound operand — a NON-empty group
            // with ANY row whose operand var is unbound ⇒ the whole aggregate is UNBOUND
            // (SPARQL §11; spareval-confirmed). Only COUNT filters errors. This extends C.4,
            // which handled only the all-unbound case for AVG; the mixed bound+unbound group
            // (and SUM over all-unbound) was still wrongly computed over just the bound rows.
            if !rows.is_empty() && rows.iter().any(|r| r.get(var).is_none()) {
                return Ok(None);
            }
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
            // SPARQL §11.4 / XPath numeric type promotion: any xsd:double operand ⇒ double
            // result; else all-integer ⇒ integer; else decimal. (C.6b: rust_agg previously
            // always emitted integer-or-decimal, losing xsd:double — a datatype =_bag
            // divergence exposed once C.6 routed nullable double-operand SUMs here.)
            if vals.iter().any(|t| is_xsd_double(t)) {
                Ok(Some(double_term(sum)?))
            } else if vals.iter().all(|t| is_xsd_integer(t)) {
                Ok(Some(integer_term(sum as i64)))
            } else {
                Ok(Some(decimal_term(sum)?))
            }
        }
        AggKind::Avg => {
            let Some(var) = &agg.arg_var else {
                return Ok(None);
            };
            // ADR-0025 C.5 (see SUM): any unbound operand row in a non-empty group ⇒ UNBOUND.
            if !rows.is_empty() && rows.iter().any(|r| r.get(var).is_none()) {
                return Ok(None);
            }
            let vals: Vec<&Term> = rows.iter().filter_map(|r| r.get(var)).collect();
            if vals.is_empty() {
                // ADR-0025 C.4: AVG over no bound values. If the GROUP is genuinely EMPTY
                // (0 rows — e.g. implicit grouping over an unmatched pattern), AVG ⇒
                // "0"^^xsd:integer (SPARQL §11, like SUM; spareval-confirmed). But if the
                // group HAS rows and the operand is simply UNBOUND in every one of them
                // (e.g. `AVG(?missing)` over a UNION arm that never binds it), there are no
                // numeric values to average ⇒ the result is UNBOUND, NOT 0. The old
                // `vals.is_empty()` conflated these two — discriminate on `rows`.
                return if rows.is_empty() {
                    Ok(Some(integer_term(0)))
                } else {
                    Ok(None)
                };
            }
            let nums: Vec<f64> = vals.iter().filter_map(|t| numeric_term(t)).collect();
            if nums.is_empty() {
                return Ok(None); // non-numeric operand ⇒ UNBOUND (type error, §11)
            }
            let avg = nums.iter().sum::<f64>() / nums.len() as f64;
            // SPARQL §11.4: AVG of xsd:double values stays xsd:double (else decimal). See the
            // SUM promotion note above (C.6b) — mirrors the SQL path's `avg_result_code`.
            if vals.iter().any(|t| is_xsd_double(t)) {
                Ok(Some(double_term(avg)?))
            } else {
                Ok(Some(decimal_term(avg)?))
            }
        }
        AggKind::Min | AggKind::Max => {
            let Some(var) = &agg.arg_var else {
                return Ok(None);
            };
            // ADR-0025 C.5 (see SUM): any unbound operand row in a non-empty group ⇒ UNBOUND.
            if !rows.is_empty() && rows.iter().any(|r| r.get(var).is_none()) {
                return Ok(None);
            }
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

/// Whether an RDF term is an `xsd:double` (or `xsd:float`, which this codebase folds into
/// double) literal — the promotion signal for SUM/AVG result typing (SPARQL §11.4 / XPath
/// numeric type promotion: any `double` operand makes the aggregate result `double`).
fn is_xsd_double(t: &Term) -> bool {
    match t {
        Term::Literal(l) => matches!(
            l.datatype().as_str(),
            "http://www.w3.org/2001/XMLSchema#double" | "http://www.w3.org/2001/XMLSchema#float"
        ),
        _ => false,
    }
}

/// Build a canonical `xsd:double` literal from an `f64` (via the shared canonicaliser — the
/// oracle's `oxsdatatypes` library — so the lexical form matches, e.g. `1.0E1`).
fn double_term(n: f64) -> Result<Term> {
    natural_literal(&format!("{n}"), XsdTypeCode::Double)
}

/// Build an `xsd:integer` literal from an `i64`.
fn integer_term(n: i64) -> Term {
    Term::Literal(Literal::new_typed_literal(
        n.to_string(),
        sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
    ))
}

/// Build an `xsd:decimal` literal from an `f64`.
fn decimal_term(n: f64) -> Result<Term> {
    // Compact (non-scientific) representation, then run it through the shared XSD-decimal
    // canonicaliser (`oxsdatatypes::Decimal`, the SAME library the oxigraph oracle uses) so
    // the lexical form is canonical and oracle-matching: an integral value renders "30", not
    // "30.0" (RDF term equality is by lexical form, so "30.0"^^decimal ≠ "30"^^decimal would
    // be a =_bag divergence), and trailing zeros trim ("1.50" → "1.5"). Consistent with the
    // SUM / `natural_literal` reconstruction path.
    let raw = if n.fract() == 0.0 {
        format!("{n:.1}")
    } else {
        format!("{n}")
    };
    natural_literal(&raw, XsdTypeCode::Decimal)
}

// --- M2 Send-future / GAT monomorphization gate (design §1 line 103, §5 M2) ----

#[cfg(test)]
mod probe_backend {
    //! Fail-fast device: a throwaway `'static` backend that proves AFIT + GAT + the
    //! generic async sink monomorphize to a **`Send`** future — the one novel
    //! language-feature risk — before any live-DB adapter is exercised.
    use super::*;
    use sf_sql::{BranchStream, RawTuple, SqlBackend};

    struct MockBackend {
        rows: Vec<RawTuple>,
    }
    struct MockStream {
        iter: std::vec::IntoIter<RawTuple>,
    }
    impl BranchStream for MockStream {
        async fn next_row(&mut self) -> sf_sql::Result<Option<RawTuple>> {
            Ok(self.iter.next())
        }
    }
    impl SqlBackend for MockBackend {
        type Stream<'s>
            = MockStream
        where
            Self: 's;
        async fn column_names(&mut self, _probe: &str) -> sf_sql::Result<Vec<String>> {
            Ok(Vec::new())
        }
        async fn open_branch(
            &mut self,
            _sql: &str,
            _params: &[String],
        ) -> sf_sql::Result<MockStream> {
            Ok(MockStream {
                iter: std::mem::take(&mut self.rows).into_iter(),
            })
        }
    }

    fn assert_send<T: Send>(t: T) -> T {
        t
    }

    #[test]
    fn send_future_monomorphizes_and_spawns() {
        // Monomorphizes `run::<MockBackend>` (here: `ask`) and proves the future is
        // `Send + 'static` enough to `tokio::spawn` — the M2 exit gate half (ii).
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let plan = Plan {
            branches: Vec::new(),
            form: PlanForm::Ask,
            distinct: false,
            limit: None,
            offset: 0,
            order: Vec::new(),
            rust_group: None,
            dialect: Dialect::Sqlite,
        };
        rt.block_on(async move {
            let backend = MockBackend { rows: Vec::new() };
            let fut = async move {
                let mut b = backend;
                ask(&plan, &mut b).await
            };
            let joined = tokio::spawn(assert_send(fut)).await.unwrap();
            assert!(!joined.unwrap());
        });
    }
}
