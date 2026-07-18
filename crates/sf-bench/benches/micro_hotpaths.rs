//! Micro-benchmarks for the M4 wave-1 perf quick wins: ORDER BY over mixed term
//! kinds (`cmp_term`'s `Term::to_string()` fallback), `COUNT`/`SUM(DISTINCT ?v)`
//! dedup (`rust_agg`), and a GROUP BY forced onto the Rust-level `rust_group` path
//! (ADR-0025 C.6 / M3 fix 1 routing) — three shapes the five GTFS `obda_select_*`
//! queries (`obda_latency.rs`) don't naturally exercise. A small, self-contained,
//! file-backed SQLite fixture (ADR-0006), same `TempDir` pattern as
//! `obda_latency.rs`, engineered so each query provably reaches the intended
//! routing decision (asserted once at setup, not per-iteration).

use criterion::{criterion_group, criterion_main, Criterion};
use rusqlite::Connection;
use sf_core::ir::TriplesMap;
use sf_sparql::{exec, parse_and_translate_with, Tbox};
use sf_sql::{Dialect, TableSchema};
use tempfile::TempDir;

/// Row count for the `ITEM` fixture table — large enough to make sorting /
/// dedup / grouping cost visible (per-query wall-clock, criterion `obda_select_1x`
/// sibling scale).
const N: i64 = 10_000;

/// A minimal R2RML mapping over one `ITEM` table, engineered to hit three
/// specific engine routing decisions — NOT the GTFS workload's five queries,
/// which don't naturally produce these shapes:
///
/// * `:linkTo` (IRI template) / `:labelText` (plain literal column) — a UNION of
///   the two binds `?v` to MIXED term kinds, for the ORDER BY sort bench
///   (`exec_core::cmp_term`).
/// * `:valA` (explicit `xsd:integer`) / `:valU` (same `val` column, no declared
///   datatype) — the `TermSpec` mismatch across the UNION's two arms defeats
///   `try_sql_group_over_union`'s cross-arm type-identity check (`iq/lower.rs`),
///   forcing `COUNT(DISTINCT ?v)` / `SUM(DISTINCT ?v)` onto the Rust-level
///   `rust_group`/`rust_agg` path instead of a pushed-down SQL
///   `COUNT(DISTINCT col)` (ADR-0025 Tier-2 gap 3).
/// * `:valU` alone, `AVG`-aggregated: an undeclared-datatype operand is not
///   PROVABLY `xsd:double`, so M3 fix 1's `avg_needs_exact_decimal` routes even
///   this single-branch GROUP BY onto `rust_group` (ADR-0025 C.6).
const MAPPING_TTL: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix : <http://example.org/bench#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

<#item> a rr:TriplesMap ;
    rr:logicalTable [ rr:tableName "ITEM" ] ;
    rr:subjectMap [ rr:template "http://example.org/bench/item/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate :grp ;
        rr:objectMap [ rr:column "grp" ; rr:datatype xsd:integer ] ] ;
    rr:predicateObjectMap [ rr:predicate :valA ;
        rr:objectMap [ rr:column "val" ; rr:datatype xsd:integer ] ] ;
    rr:predicateObjectMap [ rr:predicate :valU ;
        rr:objectMap [ rr:column "val" ] ] ;
    rr:predicateObjectMap [ rr:predicate :linkTo ;
        rr:objectMap [ rr:template "http://example.org/bench/ref/{id}" ; rr:termType rr:IRI ] ] ;
    rr:predicateObjectMap [ rr:predicate :labelText ;
        rr:objectMap [ rr:column "label" ] ] .
"#;

/// (i) ORDER BY over a variable bound to MIXED term kinds — one UNION arm an
/// IRI, the other a plain literal.
const Q_ORDER_MIXED_KIND: &str = "
PREFIX : <http://example.org/bench#>
SELECT ?v WHERE {
  { ?s :linkTo ?v } UNION { ?s :labelText ?v }
} ORDER BY ?v";

/// (ii) `COUNT(DISTINCT ?v)`/`SUM(DISTINCT ?v)` forced onto `rust_group` by a
/// cross-arm datatype mismatch (`:valA` declared `xsd:integer`, `:valU`
/// undeclared) — `val = id % 1000`, so each of the 1000 distinct values repeats
/// 10x per arm (20x combined): real dedup work, not a no-op DISTINCT.
const Q_DISTINCT_AGG: &str = "
PREFIX : <http://example.org/bench#>
SELECT (COUNT(DISTINCT ?v) AS ?c) (SUM(DISTINCT ?v) AS ?s) WHERE {
  { ?a :valA ?v } UNION { ?b :valU ?v }
}";

/// (iii) GROUP BY + AVG over an undeclared-datatype operand — single-branch, but
/// M3 fix 1 still routes it onto `rust_group` (`avg_needs_exact_decimal`).
const Q_GROUP_AVG_RUST: &str = "
PREFIX : <http://example.org/bench#>
SELECT ?g (AVG(?v) AS ?avg) WHERE { ?s :grp ?g ; :valU ?v } GROUP BY ?g";

struct Fixture {
    _dir: TempDir,
    conn: Connection,
    maps: Vec<TriplesMap>,
    schemas: Vec<TableSchema>,
}

fn fixture() -> Fixture {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("micro.db");
    let conn = Connection::open(&path).expect("open sqlite");
    conn.execute_batch(
        "CREATE TABLE ITEM (id INTEGER PRIMARY KEY, grp INTEGER, val INTEGER, label TEXT);
         PRAGMA synchronous=OFF; PRAGMA journal_mode=MEMORY;",
    )
    .expect("create schema");
    {
        let tx = conn.unchecked_transaction().expect("tx");
        {
            let mut stmt = tx
                .prepare("INSERT INTO ITEM VALUES (?1,?2,?3,?4)")
                .expect("prepare insert");
            for id in 0..N {
                stmt.execute(rusqlite::params![
                    id,
                    id % 100,
                    id % 1000,
                    format!("label-{id}")
                ])
                .expect("insert row");
            }
        }
        tx.commit().expect("commit");
    }
    let maps = sf_mapping::parse_r2rml(MAPPING_TTL).expect("parse mapping");
    let schemas = vec![sf_sql::introspect::introspect_sqlite(&conn, "ITEM").expect("introspect")];
    Fixture {
        _dir: dir,
        conn,
        maps,
        schemas,
    }
}

fn translate(fx: &Fixture, sparql: &str) -> sf_sparql::Plan {
    parse_and_translate_with(
        sparql,
        &fx.maps,
        Dialect::Sqlite,
        &Tbox::default(),
        &fx.schemas,
    )
    .unwrap_or_else(|e| panic!("translate {sparql}: {e}"))
}

fn run(fx: &Fixture, sparql: &str) -> usize {
    exec::select(&translate(fx, sparql), &fx.conn)
        .unwrap_or_else(|e| panic!("exec {sparql}: {e}"))
        .rows
        .len()
}

/// Fail loudly (not silently benchmark the wrong path) if the cross-arm spec
/// mismatch / undeclared-datatype routing tricks above ever stop forcing
/// `rust_group` — a translator change that fixed this would make the "before"
/// half of the receipt meaningless without anyone noticing.
fn assert_routes_to_rust_group(fx: &Fixture, sparql: &str) {
    assert!(
        translate(fx, sparql).rust_group.is_some(),
        "routing assumption broken: {sparql} no longer routes to rust_group"
    );
}

fn bench_order_mixed_kind(c: &mut Criterion) {
    let fx = fixture();
    // Sanity: both UNION arms contributed (mixed IRI/literal kinds actually reach
    // the sort, not just one homogeneous arm).
    assert_eq!(run(&fx, Q_ORDER_MIXED_KIND), (2 * N) as usize);
    c.bench_function("micro_order_mixed_kind", |b| {
        b.iter(|| run(&fx, Q_ORDER_MIXED_KIND));
    });
}

fn bench_distinct_agg(c: &mut Criterion) {
    let fx = fixture();
    assert_routes_to_rust_group(&fx, Q_DISTINCT_AGG);
    assert_eq!(run(&fx, Q_DISTINCT_AGG), 1); // implicit grouping — one result row
    c.bench_function("micro_distinct_agg", |b| {
        b.iter(|| run(&fx, Q_DISTINCT_AGG));
    });
}

fn bench_group_avg_rust(c: &mut Criterion) {
    let fx = fixture();
    assert_routes_to_rust_group(&fx, Q_GROUP_AVG_RUST);
    assert_eq!(run(&fx, Q_GROUP_AVG_RUST), 100); // grp = id % 100 => 100 groups
    c.bench_function("micro_group_avg_rust", |b| {
        b.iter(|| run(&fx, Q_GROUP_AVG_RUST));
    });
}

criterion_group!(
    benches,
    bench_order_mixed_kind,
    bench_distinct_agg,
    bench_group_avg_rust
);
criterion_main!(benches);
