//! `exec_core` — the driver-agnostic execution core (ADR-0024). One async pull-cursor
//! loop generic over [`SqlBackend`], plus the per-form entry points. The SQLite entry
//! points in [`crate::exec`] delegate here through a `block_on` shim; PostgreSQL /
//! MySQL keep their parallel loops until M3/M4 (design §5).
//!
//! The term-gen helpers (`reconstruct` / `order_cmp` / `eval_expr` / `instantiate` /
//! `Solutions` / `rust_group_result_rows` / `RawRow`) are reused **in place** from
//! [`crate::exec`] (reuse-first; the PostgreSQL / MySQL executors import the same
//! symbols from there byte-unchanged). The single-home physical relocation is a
//! later cleanup (design §2) — it is not required by the M2 exit gate.
//!
//! The corrected per-branch sequence (design §2, mirroring the old `exec.rs`
//! SQLite loop): reconstruct → DISTINCT dedup (before slice) → if ordered {buffer,
//! defer} else {streaming OFFSET/LIMIT} → after the loop: sort THEN slice.

use std::collections::BTreeMap;
use std::future::Future;

use sf_core::{Term, Triple};
use sf_sql::{BranchStream, Dialect, SqlBackend};

use crate::emit::{self, ColumnCatalog};
use crate::exec::{
    eval_expr, instantiate, order_cmp, reconstruct, rust_group_result_rows, RawRow, Solutions,
};
use crate::iq::{Branch, OrderKey, RustGroup};
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
    B: SqlBackend,
    F: FnMut(Vec<Option<Term>>) -> Fut,
    Fut: Future<Output = Result<()>>,
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
    B: SqlBackend,
    F: FnMut(Vec<Triple>) -> Fut,
    Fut: Future<Output = Result<()>>,
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
