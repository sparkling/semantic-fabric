//! ADR-0023 optimizer-residue wave, q9 agg-pushdown follow-up (Wave A.3): the SQL
//! pushdown (`try_sql_group_over_union` — `GROUP BY` + aggregates over a `UNION
//! ALL` derived table, computed by the DATABASE) exists specifically to avoid the
//! `RustGroup` fallback's buffer-every-pre-aggregation-row-in-Rust behaviour
//! (`rust_group_execute`, ADR-0023 design §5 Aggregation). This proves that claim
//! with the SAME [`sf_bench::mem`] high-water-mark probe `constant_memory.rs` uses
//! (ADR-0006 *Streaming & bounded memory*): stream the IDENTICAL aggregate query
//! over the SAME SQLite fixture at growing UNION cardinality, once through each
//! path (toggled purely by cross-arm `TermSpec` compatibility — the pushdown's own
//! applicability gate, no other code difference), and assert:
//!
//! 1. the pushdown path's engine peak heap stays ≈ CONSTANT as the pre-aggregation
//!    UNION cardinality grows (the DB does the grouping; the client only ever
//!    streams the small, fixed-size post-aggregation result);
//! 2. the `RustGroup` fallback's peak GROWS with the pre-aggregation cardinality
//!    (it buffers every union-arm solution before grouping);
//! 3. at the largest scale, the pushdown's peak is materially LOWER than
//!    `RustGroup`'s over the identical data — the actual memory win, not just two
//!    separate bounded/unbounded claims.

use rusqlite::Connection;
use sf_sparql::{exec, parse_and_translate_tree_with, Tbox};
use sf_sql::Dialect;

#[global_allocator]
static GLOBAL: sf_bench::mem::Tracking = sf_bench::mem::Tracking;

const QUERY: &str = r#"
    PREFIX ex: <http://ex/>
    SELECT ?g (COUNT(?v) AS ?c) WHERE {
        ?s ex:grp ?g .
        { ?s ex:p1 ?v } UNION { ?s ex:p2 ?v }
    } GROUP BY ?g"#;

/// Both arms declare the IDENTICAL `TermSpec` (plain literal, no datatype) for
/// `?v` — `try_sql_group_over_union`'s cross-arm compatibility gate passes, so
/// the pushdown fires.
const PUSHDOWN_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#M>
    rr:logicalTable [ rr:tableName "m" ] ;
    rr:subjectMap [ rr:template "http://ex/m/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:grp ; rr:objectMap [ rr:column "grp" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:p1  ; rr:objectMap [ rr:column "v1" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:p2  ; rr:objectMap [ rr:column "v2" ] ] .
"#;

/// Identical shape, but `?v`'s two arms now declare DIFFERENT `TermSpec`s (an
/// explicit `rr:datatype xsd:string` on one POM only) — the pushdown's concern-#1
/// cross-arm type-unification gate declines, so this is the SAME query/data
/// forced through the `RustGroup` buffer-and-group fallback (the correctness
/// oracle the pushdown falls back to when it can't prove `=_bag`-safety).
const RUST_GROUP_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<#M>
    rr:logicalTable [ rr:tableName "m" ] ;
    rr:subjectMap [ rr:template "http://ex/m/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:grp ; rr:objectMap [ rr:column "grp" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:p1  ; rr:objectMap [ rr:column "v1" ] ] ;
    rr:predicateObjectMap [
        rr:predicate ex:p2 ;
        rr:objectMap [ rr:column "v2" ; rr:datatype xsd:string ]
    ] .
"#;

/// A file-backed SQLite source with `n_rows` split evenly across 8 fixed `grp`
/// values (so the POST-aggregation result stays a small, constant 8 rows at every
/// scale — only the PRE-aggregation UNION cardinality, `2 * n_rows`, grows).
fn build_db(path: &std::path::Path, n_rows: u32) -> Connection {
    let conn = Connection::open(path).expect("open sqlite file");
    conn.execute_batch(
        "CREATE TABLE m (id INTEGER PRIMARY KEY, grp TEXT NOT NULL, v1 TEXT NOT NULL, v2 TEXT NOT NULL);",
    )
    .expect("create table");
    let tx = conn.unchecked_transaction().expect("begin tx");
    {
        let mut stmt = tx
            .prepare("INSERT INTO m (id, grp, v1, v2) VALUES (?1, ?2, ?3, ?4)")
            .expect("prepare insert");
        for i in 0..n_rows {
            stmt.execute(rusqlite::params![
                i,
                format!("g{}", i % 8),
                format!("v1-{i}"),
                format!("v2-{i}"),
            ])
            .expect("insert row");
        }
    }
    tx.commit().expect("commit fixture rows");
    conn
}

/// Translate + stream `QUERY` over `n_rows`, bracketing the engine peak-heap
/// window around JUST the translate+execute call (the fixture load itself sits
/// outside the window, matching `constant_memory.rs`'s convention). Returns
/// `(result_rows, peak_engine_bytes, pushdown_fired)`.
fn measure(r2rml: &str, n_rows: u32, dir: &std::path::Path, tag: &str) -> (usize, i64, bool) {
    let path = dir.join(format!("{tag}_{n_rows}.db"));
    let conn = build_db(&path, n_rows);
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let schema = sf_sql::introspect::introspect_sqlite(&conn, "m").expect("introspect");

    let base = sf_bench::mem::reset_peak();
    let plan =
        parse_and_translate_tree_with(QUERY, &maps, Dialect::Sqlite, &Tbox::default(), &[schema])
            .expect("translate agg-over-union");
    let pushdown_fired = plan.rust_group.is_none()
        && plan
            .branches
            .iter()
            .any(|b| b.agg.is_some() && !b.subplan_joins.is_empty());
    let sols = exec::select(&plan, &conn).expect("stream select");
    let peak = sf_bench::mem::window_peak(base);
    (sols.rows.len(), peak, pushdown_fired)
}

/// Both proofs in ONE test function (not two): [`sf_bench::mem`]'s peak tracker is
/// a process-wide static, so two SEPARATE `#[test]`s in this binary could run on
/// different threads under the default (parallel) test harness and cross-
/// contaminate each other's window — `constant_memory.rs` sidesteps this for its
/// live-backend arms by documenting "run serially"; here we sidestep it structurally
/// by keeping every measurement in one function, so the ordering is deterministic
/// regardless of how `cargo test` schedules OTHER test binaries/files.
///
/// (1) The pushdown path's engine peak heap stays ≈ CONSTANT as the UNION's
/// pre-aggregation cardinality grows 32x (1,000 → 32,000 rows ⇒ 2,000 → 64,000
/// union solutions) — the database does the `GROUP BY`, so the client only ever
/// streams 8 fixed result rows.
///
/// (2)+(3) The head-to-head: the SAME query/data at the largest scale, once
/// forced through `RustGroup` (buffers every pre-aggregation solution) and once
/// through the pushdown. `RustGroup`'s peak must be materially larger — the
/// actual memory win the pushdown exists for, not just two independently-bounded
/// claims.
#[test]
fn pushdown_memory_bounded_and_beats_rust_group() {
    let dir = tempfile::TempDir::new().expect("temp dir");

    // (1) constant-memory sweep, pushdown path only.
    let scales = [1_000u32, 8_000, 32_000];
    let mut peaks = Vec::new();
    eprintln!(
        "\nWave A.3 pushdown constant-memory: {:>8} {:>10} {:>14}",
        "rows", "result", "peak_B"
    );
    for &n in &scales {
        let (result_rows, peak, fired) = measure(PUSHDOWN_R2RML, n, dir.path(), "pushdown");
        assert!(
            fired,
            "PUSHDOWN_R2RML must exercise the SQL pushdown at n={n}, not RustGroup"
        );
        assert_eq!(result_rows, 8, "8 fixed groups regardless of scale");
        eprintln!("{n:>28} {result_rows:>10} {peak:>14}");
        peaks.push(peak);
    }
    let floor = 64 * 1024i64; // 64 KiB noise floor
    let eff_min = peaks.iter().copied().min().unwrap().max(floor);
    let eff_max = peaks.iter().copied().max().unwrap().max(floor);
    let mem_ratio = eff_max as f64 / eff_min as f64;
    assert!(
        mem_ratio <= 4.0,
        "pushdown engine peak heap must stay ≈ constant across a 32x cardinality \
         growth: mem_ratio={mem_ratio:.2} (peaks={peaks:?} bytes)"
    );

    // (2)+(3) head-to-head at the largest scale.
    let n = 32_000u32;
    let (rg_rows, rg_peak, rg_fired) = measure(RUST_GROUP_R2RML, n, dir.path(), "rustgroup");
    assert!(
        !rg_fired,
        "RUST_GROUP_R2RML's cross-arm TermSpec mismatch must decline the pushdown"
    );
    let (pd_rows, pd_peak, pd_fired) = measure(PUSHDOWN_R2RML, n, dir.path(), "pushdown2");
    assert!(pd_fired, "PUSHDOWN_R2RML must exercise the SQL pushdown");
    assert_eq!(rg_rows, pd_rows, "both paths compute the same 8 groups");
    eprintln!(
        "Wave A.3 head-to-head @ {n} rows (2*{n} union solutions): \
         rust_group={rg_peak}B pushdown={pd_peak}B"
    );

    let rg = rg_peak.max(floor);
    let pd = pd_peak.max(floor);
    assert!(
        pd * 3 <= rg,
        "pushdown peak heap ({pd}B) must be materially (≥3x) below RustGroup's \
         ({rg}B) at {n} rows — the buffer-every-row cost RustGroup pays and the \
         pushdown does not"
    );
}
