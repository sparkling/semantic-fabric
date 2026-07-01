//! PostgreSQL execution path (ADR-0006 *Streaming & bounded memory*; ADR-0010 §C;
//! ADR-0015): run the emitted PostgreSQL SQL over a **live** server through the
//! bounded-memory server-side cursor ([`PgRowStream`], `query_raw` — never the
//! buffer-all `query()`), reconstruct `oxrdf` terms through the **single** sf-core
//! term-gen path shared with the SQLite executor ([`crate::exec`], ADR-0003 R3),
//! and stream CONSTRUCT triples / mapping-IR quads.
//!
//! The §10 natural datatype (ADR-0015) is taken from the prepared statement's
//! result-column PostgreSQL types — the catalog authority for a statically typed
//! source — not from the driver's text rendering. PostgreSQL pads `CHARACTER(n)`
//! values itself, so the SQLite `char_pad` step is unnecessary here (the value
//! arrives space-padded), and the §10 cross-dialect consistency clause holds by
//! construction.
//!
//! Values are **bound parameters only** (ADR-0010 R1): the emitted `$n`
//! placeholders are filled from `EmittedBranch::params`, never interpolated.
//!
//! Requires a live server, so this path is exercised by the conformance
//! integration suite (ADR-0012), never the in-crate unit tests.

use std::collections::BTreeMap;
use std::future::Future;

use sf_core::datatype::{self, XsdTypeCode};
use sf_core::ir::LogicalSource;
use sf_core::{GraphName, Quad, Term, Triple};
use sf_sql::stream::PgRowStream;
use sf_sql::Dialect;
use tokio_postgres::types::{ToSql, Type};
use tokio_postgres::{Client, Row as PgRow};

use crate::emit::ColumnCatalog;
use crate::exec::{instantiate, order_cmp, reconstruct, RawRow, Solutions};
use crate::iq::Branch;
use crate::{Error, Plan, PlanForm, Result};

/// Introspect (via prepare-time metadata) the actual result-column names of every
/// source the plan reads, so emission resolves a mapping's regular-identifier
/// column references to the columns PostgreSQL truly exposes after its lowercase
/// identifier folding (see [`crate::emit`]). A source whose metadata cannot be read
/// is omitted (resolution falls back to the raw identifier).
async fn build_catalog_pg(branches: &[Branch], client: &Client, dialect: Dialect) -> ColumnCatalog {
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
            if let Ok(stmt) = client.prepare(&probe).await {
                let names = stmt.columns().iter().map(|c| c.name().to_owned()).collect();
                catalog.insert(source, names);
            }
        }
    }
    catalog
}

/// The §10 natural XSD type implied by a PostgreSQL result-column type
/// (ADR-0015). Text-like types carry no implied datatype (plain literal) and map
/// to [`XsdTypeCode::String`]; an unrecognised type yields `None`, which the
/// reconstruction treats as a plain literal.
fn pg_xsd_code(ty: &Type) -> Option<XsdTypeCode> {
    use XsdTypeCode::*;
    match *ty {
        Type::BOOL => Some(Boolean),
        Type::INT2 | Type::INT4 | Type::INT8 => Some(Integer),
        Type::FLOAT4 | Type::FLOAT8 => Some(Double),
        Type::NUMERIC => Some(Decimal),
        Type::DATE => Some(Date),
        Type::TIME => Some(Time),
        Type::TIMESTAMP | Type::TIMESTAMPTZ => Some(DateTime),
        Type::BYTEA => Some(HexBinary),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::CHAR | Type::UNKNOWN => {
            Some(String)
        }
        _ => None,
    }
}

/// A bound parameter carried as its **lexical SPARQL form** (`&str`), serialised
/// to whatever PostgreSQL type the prepared statement infers for the placeholder.
///
/// All emitted `$n` values arrive as strings (`EmittedBranch::params`), but a
/// FILTER constant compared against a typed column lowers to a bare placeholder —
/// e.g. `FILTER(?d = 1)` over an `INT4` column emits `"direction_id" = $1`, where
/// PostgreSQL infers `$1` as `INT4`. Binding the raw Rust `String` there fails
/// *client-side* (`String` does not `accepts(INT4)`), aborting the already-200
/// response body mid-stream. This wrapper inspects the driver-supplied `ty` at
/// serialise time and parses the lexical form into the native Rust type
/// (delegating to that type's own `ToSql`), so integer/float/boolean placeholders
/// bind correctly. Text-like (and any other) placeholders fall through to the
/// plain string binding — byte-identical to the previous behaviour, so the
/// passing text/string FILTER paths are untouched. Values stay bound parameters
/// (ADR-0010 R1) — never interpolated.
#[derive(Debug)]
struct LexicalParam<'a>(&'a str);

impl ToSql for LexicalParam<'_> {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut bytes::BytesMut,
    ) -> std::result::Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    {
        match *ty {
            Type::BOOL => self.0.parse::<bool>()?.to_sql(ty, out),
            Type::INT2 => self.0.parse::<i16>()?.to_sql(ty, out),
            Type::INT4 => self.0.parse::<i32>()?.to_sql(ty, out),
            Type::INT8 => self.0.parse::<i64>()?.to_sql(ty, out),
            Type::FLOAT4 => self.0.parse::<f32>()?.to_sql(ty, out),
            Type::FLOAT8 => self.0.parse::<f64>()?.to_sql(ty, out),
            // Text-like and everything else: bind the raw lexical string, exactly
            // as the previous `&String` binding did.
            _ => self.0.to_sql(ty, out),
        }
    }

    // Accept every placeholder type; `to_sql` dispatches on the actual `ty`, so
    // the driver never rejects the bind before we get to parse it.
    fn accepts(_ty: &Type) -> bool {
        true
    }

    tokio_postgres::types::to_sql_checked!();
}

/// Extract column `idx` of `row` as its raw lexical string (NULL ⇒ `None`),
/// fetched in the most type-faithful driver form (ADR-0015) — integers/floats as
/// their native Rust type, `bytea` uppercase-hex-encoded, booleans as
/// `true`/`false` (never PostgreSQL's `t`/`f`). XSD-canonicalisation of the
/// lexical form is the downstream sf-core chokepoint's concern. A type the
/// reader does not cover surfaces as a `501` (turned into a documented skip).
fn pg_value(row: &PgRow, idx: usize, ty: &Type) -> Result<Option<String>> {
    let sql_err = |e: tokio_postgres::Error| Error::Sql(e.to_string());
    let s = match *ty {
        Type::BOOL => row
            .try_get::<_, Option<bool>>(idx)
            .map_err(sql_err)?
            .map(|b| b.to_string()),
        Type::INT2 => row
            .try_get::<_, Option<i16>>(idx)
            .map_err(sql_err)?
            .map(|v| v.to_string()),
        Type::INT4 => row
            .try_get::<_, Option<i32>>(idx)
            .map_err(sql_err)?
            .map(|v| v.to_string()),
        Type::INT8 => row
            .try_get::<_, Option<i64>>(idx)
            .map_err(sql_err)?
            .map(|v| v.to_string()),
        Type::FLOAT4 => row
            .try_get::<_, Option<f32>>(idx)
            .map_err(sql_err)?
            .map(|v| v.to_string()),
        Type::FLOAT8 => row
            .try_get::<_, Option<f64>>(idx)
            .map_err(sql_err)?
            .map(|v| v.to_string()),
        Type::BYTEA => row
            .try_get::<_, Option<Vec<u8>>>(idx)
            .map_err(sql_err)?
            .map(|b| {
                let mut out = std::string::String::new();
                datatype::hex_binary_upper(&b, &mut out);
                out
            }),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::CHAR | Type::UNKNOWN => row
            .try_get::<_, Option<std::string::String>>(idx)
            .map_err(sql_err)?,
        _ => {
            return Err(Error::Unsupported(format!(
                "PostgreSQL result type {ty} reconstruction"
            )))
        }
    };
    Ok(s)
}

/// Iterate every WHERE solution across all branches over a live PostgreSQL
/// connection, one row in flight (bounded memory). Mirrors the SQLite
/// [`crate::exec`] core: offset/limit applied in Rust only for the multi-branch
/// (bag-union) case (the single-branch case already pushed them into SQL).
///
/// The `sink` is **async** (awaited per solution): collecting callers return a
/// ready future, while the HTTP streaming layer returns a future that serialises
/// the row and flushes it into the response body — `send().await` there applies
/// backpressure straight back to this `query_raw` cursor, so the whole pipeline
/// stays bounded-memory end to end (ADR-0006 / ADR-0010 §C).
async fn for_each_solution_pg<F, Fut>(plan: &Plan, client: &Client, sink: F) -> Result<()>
where
    F: FnMut(&Branch, &BTreeMap<String, Term>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    // Multi-branch GROUP BY (aggregate over a UNION/VALUES inner that cannot be
    // grouped in SQL — ADR-0007): buffer all inner solutions, then group +
    // aggregate in Rust via the backend-independent helper shared with the SQLite
    // executor ([`crate::exec::rust_group_result_rows`]). This is the async mirror
    // of the SQLite [`crate::exec`] `rust_group_execute` — the collection of inner
    // solutions differs (live PG cursor vs SQLite), the grouping semantics do not.
    if let Some(rg) = &plan.rust_group {
        return rust_group_execute_pg(plan, client, rg, sink).await;
    }
    for_each_branch_solution_pg(plan, client, sink).await
}

/// Core branches loop over a live PostgreSQL connection — does NOT check
/// `rust_group`. Called from both [`for_each_solution_pg`] (when no rust_group is
/// set) and [`rust_group_execute_pg`] (to collect the inner solutions without
/// re-triggering grouping). The async mirror of the SQLite
/// [`crate::exec`] `for_each_branch_solution`.
async fn for_each_branch_solution_pg<F, Fut>(
    plan: &Plan,
    client: &Client,
    mut sink: F,
) -> Result<()>
where
    F: FnMut(&Branch, &BTreeMap<String, Term>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let branches = plan.prepared_branches();
    let catalog = build_catalog_pg(&branches, client, plan.dialect).await;
    let multi = branches.len() > 1;
    // DISTINCT over a multi-branch bag-union: SQL dedups only *within* each branch's
    // SELECT, never *across* the separate per-branch SELECTs, so we dedup the projected
    // solutions here — before OFFSET/LIMIT. The single-branch case pushes DISTINCT into
    // SQL. Mirroring the SQLite executor ([`crate::exec::for_each_solution`]).
    let distinct_vars: Option<Vec<String>> = match (plan.distinct && multi, &plan.form) {
        (true, PlanForm::Select { vars }) => Some(vars.clone()),
        _ => None,
    };
    let mut seen_tuples: std::collections::HashSet<Vec<Option<Term>>> =
        std::collections::HashSet::new();
    let mut seen = 0usize;
    let mut emitted = 0usize;
    // ORDER BY is applied here for every plan (single- and multi-branch), never in
    // SQL: a SQL `ORDER BY` inherits the column's collation/affinity, which can
    // disagree with SPARQL value order. Buffer + stable-sort via the type-aware
    // `order_cmp`, then OFFSET/LIMIT. ORDER BY inherently buffers (the streaming
    // exception), mirroring the SQLite executor.
    let ordered = !plan.order.is_empty();
    let mut buffer: Vec<(usize, BTreeMap<String, Term>)> = Vec::new();
    for (bi, branch) in branches.iter().enumerate() {
        let e = crate::emit::emit_branch_with(branch, plan.dialect, &catalog)?;
        // Each emitted `$n` value is a lexical string, but a FILTER constant may
        // bind against a typed column (INT4/FLOAT8/BOOL/…); `LexicalParam` parses
        // it to the placeholder's inferred PG type at bind time (see its docs).
        let lex: Vec<LexicalParam> = e.params.iter().map(|s| LexicalParam(s)).collect();
        let params: Vec<&(dyn ToSql + Sync)> =
            lex.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
        let mut stream = PgRowStream::open(client, &e.sql, &params)
            .await
            .map_err(map_sql_err)?;
        while let Some(row) = stream.try_next().await.map_err(map_sql_err)? {
            // The emitted SQL projects exactly `e.projection` columns (each `AS
            // c{i}`), so the row's columns align with it positionally.
            let cols = row.columns();
            let mut values = Vec::with_capacity(cols.len());
            let mut codes = Vec::with_capacity(cols.len());
            for (i, col) in cols.iter().enumerate() {
                let ty = col.type_();
                codes.push(pg_xsd_code(ty));
                values.push(pg_value(&row, i, ty)?);
            }
            let raw = RawRow {
                schema: &e.projection,
                values: &values,
                codes: &codes,
            };
            let bindings = reconstruct(branch, &raw)?;
            // DISTINCT dedup for multi-branch bag-union (single-branch dedups in SQL).
            // Applied before ORDER BY buffering so the sorted bag is already deduped.
            if multi {
                if let Some(vars) = &distinct_vars {
                    let key: Vec<Option<Term>> =
                        vars.iter().map(|v| bindings.get(v).cloned()).collect();
                    if !seen_tuples.insert(key) {
                        continue; // duplicate projected solution
                    }
                }
            }
            // ORDER BY (any branch count): buffer for the global type-aware sort after
            // every row, so single- and multi-branch order identically (slice below).
            if ordered {
                buffer.push((bi, bindings));
                continue;
            }
            // Streaming OFFSET/LIMIT only when SQL didn't apply them (multi-branch).
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

/// Multi-branch GROUP BY over a live PostgreSQL connection: collect every inner
/// solution (no DISTINCT/OFFSET/LIMIT/ORDER on the inner — those apply AFTER
/// grouping), then group + aggregate + slice via the backend-independent
/// [`crate::exec::rust_group_result_rows`] and stream the grouped rows. The async
/// mirror of the SQLite [`crate::exec`] `rust_group_execute` (ADR-0007).
async fn rust_group_execute_pg<F, Fut>(
    plan: &Plan,
    client: &Client,
    rg: &crate::iq::RustGroup,
    mut sink: F,
) -> Result<()>
where
    F: FnMut(&Branch, &BTreeMap<String, Term>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    // A no-modifier inner plan: GROUP BY applies AFTER collecting all inner rows,
    // so DISTINCT/OFFSET/LIMIT/ORDER must NOT touch the inner solutions (they are
    // applied to the grouped result rows by the helper below). `rust_group: None`
    // routes through the non-recursive core loop.
    let inner_plan = Plan {
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        rust_group: None,
        ..plan.clone()
    };
    let mut inner_rows: Vec<BTreeMap<String, Term>> = Vec::new();
    for_each_branch_solution_pg(&inner_plan, client, |_, bindings| {
        inner_rows.push(bindings.clone());
        std::future::ready(Ok(()))
    })
    .await?;

    // Choose a dummy branch for the sink call (GROUP BY rows are not from a single
    // branch); mirrors the SQLite path.
    let dummy = plan.branches.first().cloned().unwrap_or_else(Branch::empty);
    for result in crate::exec::rust_group_result_rows(plan, rg, inner_rows)? {
        sink(&dummy, &result).await?;
    }
    Ok(())
}

/// Flatten the error's source chain into the message — `tokio_postgres::Error`'s
/// `Display` is only the kind (`db error`); the server `DbError` detail lives in
/// its `source()`, so surface it for honest conformance reporting.
fn map_sql_err(e: sf_sql::Error) -> Error {
    use std::error::Error as _;
    let mut msg = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        msg.push_str(": ");
        msg.push_str(&s.to_string());
        src = s.source();
    }
    Error::Sql(msg)
}

/// Collect a CONSTRUCT's triples over a live PostgreSQL connection. Streaming is
/// the bounded-memory core ([`for_each_solution_pg`]); this collects for the
/// conformance harness.
pub async fn construct_triples_pg(plan: &Plan, client: &Client) -> Result<Vec<Triple>> {
    let template = match &plan.form {
        PlanForm::Construct { template } => template.clone(),
        _ => {
            return Err(Error::Unsupported(
                "construct() requires a CONSTRUCT plan".to_owned(),
            ))
        }
    };
    let mut out = Vec::new();
    for_each_solution_pg(plan, client, |_branch, bindings| {
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

/// Stream a CONSTRUCT over a live PostgreSQL connection: `sink` is awaited once
/// per solution with that solution's instantiated triples (bounded by the
/// template size — never the whole graph). The async mirror of the streaming
/// SQLite [`crate::exec::construct`] sink; the HTTP layer serialises + flushes
/// each batch into the RDF response body without collecting (ADR-0010 §C).
pub async fn construct_each_pg<F, Fut>(plan: &Plan, client: &Client, mut sink: F) -> Result<()>
where
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
    for_each_solution_pg(plan, client, |_branch, bindings| {
        let triples: Vec<Triple> = template
            .iter()
            .filter_map(|tp| instantiate(tp, bindings))
            .collect();
        sink(triples)
    })
    .await
}

/// Execute a SELECT over a live PostgreSQL connection, collecting solutions —
/// the async mirror of the sync SQLite [`crate::exec::select`] (ADR-0003 R3: the
/// SAME reconstruction). Bounded-memory streaming is the [`for_each_solution_pg`]
/// core (server-side `query_raw` cursor, one row in flight); this collects the
/// projected rows for callers/tests. The identifier case-folding resolution
/// (`build_catalog_pg` / `emit::resolve_col`) is applied inside the core, so the
/// PG path resolves regular-identifier column references through PostgreSQL's
/// lowercase folding exactly as the conformance PG path does.
pub async fn select_pg(plan: &Plan, client: &Client) -> Result<Solutions> {
    let vars = match &plan.form {
        PlanForm::Select { vars } => vars.clone(),
        _ => {
            return Err(Error::Unsupported(
                "select() requires a SELECT plan".to_owned(),
            ))
        }
    };
    let mut rows = Vec::new();
    for_each_solution_pg(plan, client, |_branch, bindings| {
        rows.push(vars.iter().map(|v| bindings.get(v).cloned()).collect());
        std::future::ready(Ok(()))
    })
    .await?;
    Ok(Solutions { vars, rows })
}

/// Stream a SELECT's solution rows over a live PostgreSQL connection into an
/// async `sink`, awaited once per projected row (in `plan` projection order,
/// `None` = unbound). The async mirror of the SQLite [`crate::exec::select_each`]:
/// the HTTP layer serialises + flushes each row into the response body without
/// ever collecting the result set (ADR-0006 / ADR-0010 §C). The caller reads the
/// projected variables from `plan.form` for the result header.
pub async fn select_each_pg<F, Fut>(plan: &Plan, client: &Client, mut sink: F) -> Result<()>
where
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
    for_each_solution_pg(plan, client, |_branch, bindings| {
        let row: Vec<Option<Term>> = vars.iter().map(|v| bindings.get(v).cloned()).collect();
        sink(row)
    })
    .await
}

/// Execute an ASK over a live PostgreSQL connection — true iff at least one
/// solution exists. The async mirror of the sync [`crate::exec::ask`] (same
/// streaming core, same reconstruction).
pub async fn ask_pg(plan: &Plan, client: &Client) -> Result<bool> {
    let mut any = false;
    for_each_solution_pg(plan, client, |_b, _s| {
        any = true;
        std::future::ready(Ok(()))
    })
    .await?;
    Ok(any)
}

/// Collect the mapping-IR **quad** dump over a live PostgreSQL connection
/// (ADR-0005 named-graph conformance) — each triple carries the graph term from
/// the applicable `rr:graphMap`(s), built through the same sf-core term-gen path.
pub async fn dump_quads_pg(
    maps: &[sf_core::ir::TriplesMap],
    client: &Client,
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
    for_each_solution_pg(&plan, client, |branch, bindings| {
        // A NULL s/p/o column ⇒ no term ⇒ no triple (§11); a named-graph branch
        // whose graph map yields no value drops that quad (no default fallback).
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
