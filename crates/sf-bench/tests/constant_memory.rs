//! The constant-memory demonstration as a fast `cargo test` (ADR-0006, the
//! differentiator): stream a result-producing OBDA query (the CONSTRUCT dump)
//! against a **file-backed** SQLite source at growing scale factors and assert
//! the engine peak-heap high-water stays BOUNDED (≈ constant, within a small
//! fixed factor) while the streamed-triple count grows ~linearly. This shows the
//! engine working set is `O(|T| + |M| + batch)` — independent of source size:
//! the source DB does the set-work on disk, the engine just streams.
//!
//! This is the same demonstration as `benches/constant_memory.rs`, at small
//! scales so it runs in `cargo test --workspace`. The file holds a single test so
//! the process-wide allocator probe sees no cross-test interference.
//!
//! ## Per-backend re-proof (ADR-0024 M5)
//!
//! [`engine_memory_is_bounded_pg`] / [`_mysql`](engine_memory_is_bounded_mysql)
//! re-prove the same invariant through the driver-agnostic execution core over the
//! **live** PG (server-side `query_raw` cursor) and MySQL (packet-bounded
//! `exec_iter`) adapters — source rows live server-side, so the engine working set
//! is cleanly separable by bracketing only the streaming window. Both SKIP cleanly
//! when no server is reachable (like the differential). Run these serially
//! (`--test-threads=1`) so the process-wide allocator probe is not shared across a
//! parallel test.

use std::sync::Mutex;

use sf_bench::{driver, mem, workload};
use tempfile::TempDir;

#[global_allocator]
static GLOBAL: mem::Tracking = mem::Tracking;

/// Serializes the three tests in this file: they share the process-wide
/// [`mem::Tracking`] allocator state, so running them concurrently (libtest's
/// default) lets one test's allocations pollute another's peak-heap window. Live
/// PG/MySQL connections used to make this harmless in practice (near-instant
/// no-op skips when no server was reachable); a live server turns them into real,
/// memory-allocating work that can overlap the SQLite test's measurement window.
static SERIALIZE: Mutex<()> = Mutex::new(());

fn serialize_guard() -> std::sync::MutexGuard<'static, ()> {
    SERIALIZE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One measured streaming run at a scale: (streamed triples, peak engine bytes).
fn measure(scale: u32) -> (u64, i64) {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("gtfs.db");
    let (conn, _counts) = workload::open_source_db(&path, scale).expect("generate source");
    let maps = driver::mapping().expect("parse mapping");
    let schemas = driver::introspect(&conn).expect("introspect source");

    // Bracket the streaming window: rebaseline the high-water, stream the dump
    // through a discarding sink (bounded memory), read the window peak.
    let base = mem::reset_peak();
    let triples = driver::stream_construct_count(&maps, &conn, &schemas, workload::DUMP_QUERY)
        .expect("stream construct");
    let peak = mem::window_peak(base);
    (triples, peak)
}

#[test]
fn engine_memory_is_bounded_under_growing_source() {
    let _guard = serialize_guard();
    // Small, fast scales (the bench covers 1x/10x/100x).
    let scales = [1u32, 4, 16];
    let mut rows = Vec::new();
    let mut peaks = Vec::new();

    eprintln!(
        "\nADR-0006 constant-memory (test): {:>6} {:>12} {:>16}",
        "scale", "triples", "peak_engine_B"
    );
    for &s in &scales {
        let (triples, peak) = measure(s);
        eprintln!("{s:>30} {triples:>12} {peak:>16}");
        rows.push(triples);
        peaks.push(peak);
    }

    // 1) Result size grows ~linearly with scale (the workload is doing real,
    //    growing work — otherwise the memory claim would be vacuous).
    let row_ratio = rows[scales.len() - 1] as f64 / rows[0].max(1) as f64;
    assert!(
        row_ratio >= 8.0,
        "result rows must grow ~linearly with scale (16x): got {row_ratio:.1}x \
         ({} → {})",
        rows[0],
        rows[scales.len() - 1]
    );

    // 2) Engine peak heap stays bounded — grows far slower than the data. Floor
    //    each peak to damp small-allocation noise, then require the memory growth
    //    factor to be both small in absolute terms AND far below the row growth.
    let floor = 64 * 1024i64; // 64 KiB noise floor
    let eff_min = peaks.iter().copied().min().unwrap().max(floor);
    let eff_max = peaks.iter().copied().max().unwrap().max(floor);
    let mem_ratio = eff_max as f64 / eff_min as f64;

    assert!(
        mem_ratio <= 4.0,
        "engine peak heap must stay ≈ constant across scales: mem_ratio={mem_ratio:.2} \
         (peaks={peaks:?} bytes)"
    );
    assert!(
        mem_ratio <= row_ratio / 2.0,
        "engine memory must grow far slower than source data: mem_ratio={mem_ratio:.2} \
         vs row_ratio={row_ratio:.1} (peaks={peaks:?})"
    );
}

/// The scale factors both live-backend variants sweep (mirrors the SQLite test).
const LIVE_SCALES: [u32; 3] = [1, 4, 16];

/// Shared assertion over a `(triples, peak_bytes, first_result)` sweep: result rows
/// grow ~linearly, engine peak heap stays ≈ constant (bounded) and far below the row
/// growth, and first-result latency stays bounded (does not blow up with source
/// size). `backend` names the arm in failure messages.
fn assert_bounded(backend: &str, rows: &[u64], peaks: &[i64], firsts: &[std::time::Duration]) {
    let n = rows.len();
    let row_ratio = rows[n - 1] as f64 / rows[0].max(1) as f64;
    assert!(
        row_ratio >= 8.0,
        "[{backend}] result rows must grow ~linearly with scale (16x): got {row_ratio:.1}x \
         ({} → {})",
        rows[0],
        rows[n - 1]
    );

    let floor = 64 * 1024i64; // 64 KiB noise floor
    let eff_min = peaks.iter().copied().min().unwrap().max(floor);
    let eff_max = peaks.iter().copied().max().unwrap().max(floor);
    let mem_ratio = eff_max as f64 / eff_min as f64;
    assert!(
        mem_ratio <= 4.0,
        "[{backend}] engine peak heap must stay ≈ constant across scales: \
         mem_ratio={mem_ratio:.2} (peaks={peaks:?} bytes)"
    );
    assert!(
        mem_ratio <= row_ratio / 2.0,
        "[{backend}] engine memory must grow far slower than source data: \
         mem_ratio={mem_ratio:.2} vs row_ratio={row_ratio:.1} (peaks={peaks:?})"
    );

    // Bounded first-result: a streaming server-side cursor yields the first triple
    // in ~constant time regardless of total result size. A weak-but-honest absolute
    // cap at bench scale (a linear buffer-then-scan would grow this with source size
    // and blow the cap well before these scales matter in production).
    let first_max = firsts.iter().copied().max().unwrap();
    assert!(
        first_max < std::time::Duration::from_secs(2),
        "[{backend}] first-result latency must stay bounded under growing source: \
         got {first_max:?} (firsts={firsts:?})"
    );
}

/// PG conninfo: `SF_PG_URL` if set, else the local trust-auth scratch DB
/// (`host=localhost port=5432 user=$USER`, dbname → $USER) — same convention as the
/// PG shootout bench + the differential.
fn pg_conn_str() -> String {
    std::env::var("SF_PG_URL").unwrap_or_else(|_| {
        let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_owned());
        format!("host=localhost port=5432 user={user}")
    })
}

/// ADR-0024 M5: constant engine memory + bounded first-result over the **live PG**
/// server-side `query_raw` cursor (cursor-grade). SKIPs cleanly with no server.
#[test]
fn engine_memory_is_bounded_pg() {
    let _guard = serialize_guard();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio rt");

    let client = match rt.block_on(async {
        let (client, connection) =
            tokio_postgres::connect(&pg_conn_str(), tokio_postgres::NoTls).await?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        Ok::<_, tokio_postgres::Error>(client)
    }) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[constant_memory] no PostgreSQL reachable ({e}) — skipping PG variant");
            return;
        }
    };

    let maps = driver::mapping().expect("parse mapping");
    let (mut rows, mut peaks, mut firsts) = (Vec::new(), Vec::new(), Vec::new());
    eprintln!(
        "\nADR-0024 M5 constant-memory (PG cursor): {:>6} {:>12} {:>16} {:>14}",
        "scale", "triples", "peak_engine_B", "first_result"
    );
    for &s in &LIVE_SCALES {
        rt.block_on(async {
            client
                .batch_execute(workload::PG_SCHEMA_SQL)
                .await
                .expect("PG schema");
            workload::generate_pg(&client, s)
                .await
                .expect("PG generate");
        });
        let schemas = rt
            .block_on(driver::introspect_pg_all(&client))
            .expect("introspect PG");
        let base = mem::reset_peak();
        let (triples, first) = rt
            .block_on(driver::stream_construct_timed_pg(
                &maps,
                &client,
                &schemas,
                workload::DUMP_QUERY,
            ))
            .expect("stream construct PG");
        let peak = mem::window_peak(base);
        eprintln!("{s:>36} {triples:>12} {peak:>16} {first:>14?}");
        rows.push(triples);
        peaks.push(peak);
        firsts.push(first);
    }
    // Best-effort teardown (idempotent DROPs); ignore failures.
    let _ = rt.block_on(client.batch_execute(workload::PG_DROP_SQL));

    assert_bounded("pg", &rows, &peaks, &firsts);
}

/// MySQL URL: `SF_MYSQL_URL` if set, else the `mysql_e2e` container default.
fn mysql_url() -> String {
    std::env::var("SF_MYSQL_URL")
        .unwrap_or_else(|_| "mysql://root:sftest@127.0.0.1:13306/sftest".to_owned())
}

/// ADR-0024 M5: constant engine memory + bounded first-result over the **live MySQL**
/// packet-bounded `exec_iter` stream. NOT cursor-grade (no server-side cursor,
/// §4/§4.2) — the claim is "no client-side full-result buffer + one `RawTuple` in
/// flight," which is what keeps the engine working set bounded here. SKIPs cleanly
/// with no server.
#[test]
fn engine_memory_is_bounded_mysql() {
    let _guard = serialize_guard();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio rt");

    let mut conn = match rt.block_on(async {
        let opts = mysql_async::Opts::from_url(&mysql_url()).ok()?;
        mysql_async::Conn::new(opts).await.ok()
    }) {
        Some(c) => c,
        None => {
            eprintln!("[constant_memory] no MySQL reachable — skipping MySQL variant");
            return;
        }
    };

    let db = format!("sf_cmem_my_{}", std::process::id());
    rt.block_on(async {
        use mysql_async::prelude::Queryable;
        conn.query_drop(format!("DROP DATABASE IF EXISTS {db}"))
            .await
            .expect("drop pre-existing db");
        conn.query_drop(format!("CREATE DATABASE {db}"))
            .await
            .expect("create throwaway db");
        conn.query_drop(format!("USE {db}"))
            .await
            .expect("use throwaway db");
    });

    let maps = driver::mapping().expect("parse mapping");
    let (mut rows, mut peaks, mut firsts) = (Vec::new(), Vec::new(), Vec::new());
    eprintln!(
        "\nADR-0024 M5 constant-memory (MySQL packet-bounded): {:>6} {:>12} {:>16} {:>14}",
        "scale", "triples", "peak_engine_B", "first_result"
    );
    for &s in &LIVE_SCALES {
        rt.block_on(workload::generate_mysql(&mut conn, s))
            .expect("MySQL generate");
        let schemas = rt
            .block_on(driver::introspect_mysql_all(&mut conn))
            .expect("introspect MySQL");
        let base = mem::reset_peak();
        let (triples, first) = rt
            .block_on(driver::stream_construct_timed_mysql(
                &maps,
                &mut conn,
                &schemas,
                workload::DUMP_QUERY,
            ))
            .expect("stream construct MySQL");
        let peak = mem::window_peak(base);
        eprintln!("{s:>52} {triples:>12} {peak:>16} {first:>14?}");
        rows.push(triples);
        peaks.push(peak);
        firsts.push(first);
    }
    // Best-effort teardown; ignore failures.
    let _ = rt.block_on(async {
        use mysql_async::prelude::Queryable;
        conn.query_drop(format!("DROP DATABASE IF EXISTS {db}"))
            .await
    });

    assert_bounded("mysql", &rows, &peaks, &firsts);
}

/// ADR-0006 M4 wave-2 batch restructure: the core O(batch) claim, isolated from
/// the GTFS/wildcard-query test's branch-size complexity. ONE branch (one
/// table, one `rr:template` IRI subject + one column-literal predicate) at two
/// row counts, both comfortably past `sf-sparql::exec_core`'s private
/// `TERM_GEN_BATCH_SIZE` (not importable here — 20k/80k rows are an order of
/// magnitude past any value that const is tuned to). Once a branch's row count
/// exceeds the batch size, engine peak heap is driven ENTIRELY by the batch
/// buffer, never by how many MORE rows are still to come — so peak at 20k rows
/// and peak at 80k rows (4x more data) must be (near-)identical, not merely
/// "within a small ratio" (the GTFS test's looser `4.0` bound exists because
/// ITS 26 branches don't all cross the batch-size threshold at the same scale
/// factor — see `engine_memory_is_bounded_under_growing_source`'s own numbers
/// for that confound; this test is the direct, unconfounded proof).
#[test]
fn engine_memory_is_batch_bounded_past_the_batch_size_threshold() {
    let _guard = serialize_guard();
    let mut peaks = Vec::new();
    for &n in &[20_000i64, 80_000] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("single_branch.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE ITEM (id INTEGER PRIMARY KEY, val TEXT);
             PRAGMA synchronous=OFF; PRAGMA journal_mode=MEMORY;",
        )
        .unwrap();
        {
            let tx = conn.unchecked_transaction().unwrap();
            let mut stmt = tx.prepare("INSERT INTO ITEM VALUES (?1,?2)").unwrap();
            for i in 0..n {
                stmt.execute(rusqlite::params![i, format!("v{i}")]).unwrap();
            }
            drop(stmt);
            tx.commit().unwrap();
        }
        let maps = sf_mapping::parse_r2rml(
            r#"
            @prefix rr: <http://www.w3.org/ns/r2rml#> .
            @prefix : <http://example.org/diag#> .
            <#item> a rr:TriplesMap ;
                rr:logicalTable [ rr:tableName "ITEM" ] ;
                rr:subjectMap [ rr:template "http://example.org/diag/item/{id}" ] ;
                rr:predicateObjectMap [ rr:predicate :val ;
                    rr:objectMap [ rr:column "val" ] ] .
            "#,
        )
        .unwrap();
        let schemas = vec![sf_sql::introspect::introspect_sqlite(&conn, "ITEM").unwrap()];

        let base = mem::reset_peak();
        let plan = sf_sparql::parse_and_translate_with(
            "CONSTRUCT { ?s <http://example.org/diag#val> ?v } WHERE { ?s <http://example.org/diag#val> ?v }",
            &maps,
            sf_sql::Dialect::Sqlite,
            &sf_sparql::Tbox::default(),
            &schemas,
        )
        .unwrap();
        let mut count = 0u64;
        sf_sparql::exec::construct(&plan, &conn, |_triple| count += 1).unwrap();
        let peak = mem::window_peak(base);
        assert_eq!(count, n as u64, "every generated row must reach the sink");
        eprintln!("  [single_branch_batch_bound] n={n} peak_engine_B={peak}");
        peaks.push(peak);
    }
    let (small, large) = (peaks[0], peaks[1]);
    let ratio = large.max(small) as f64 / small.min(large).max(1) as f64;
    assert!(
        ratio <= 1.15,
        "past the batch-size threshold, peak heap must be (near-)identical \
         regardless of how many more rows remain — 4x more data (20k -> 80k \
         rows) moved peak by {ratio:.3}x (peaks={peaks:?} bytes)"
    );
}
