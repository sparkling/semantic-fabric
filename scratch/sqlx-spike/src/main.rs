//! sqlx streaming spike — ADR-0024 §8 ("the `sqlx` fallback impl, gated on a
//! streaming spike"). THROWAWAY. Not wired into `exec_core.rs` or any production
//! path. Empirically answers ONE question for BOTH PostgreSQL and MySQL:
//!
//!   Does sqlx's streaming `.fetch()` (returns a `Stream`) give bounded engine
//!   memory + bounded time-to-first-row REGARDLESS of total result size, or does
//!   it secretly buffer the whole resultset client-side?
//!
//! Method: one measurement per process invocation (fresh RSS baseline every time),
//! against a synthetic `id BIGINT, payload TEXT(200 B)` table populated to MAX_ROWS.
//! For each row count we run streaming `.fetch()` AND buffering `.fetch_all()` (the
//! control). If `.fetch()` peak-RSS stays FLAT across scales while `.fetch_all()`
//! peak-RSS grows LINEARLY, streaming is proven. Time-to-first-row that stays flat
//! (vs. scaling with N) is the second, independent streaming signal.
//!
//! CSV out (stdout): engine,mode,rows_req,rows_read,ttfr_ms,total_ms,baseline_rss_mb,peak_rss_mb,rss_at_first_row_mb

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use futures_util::TryStreamExt;
use sqlx::mysql::MySqlConnection;
use sqlx::postgres::PgConnection;
use sqlx::{Connection, Executor, Row};

type BoxErr = Box<dyn std::error::Error>;

const MAX_ROWS: i64 = 2_000_000;
const PAYLOAD_LEN: usize = 200;
const TABLE: &str = "sqlx_spike_bench";

// Peak process RSS (KB) observed by the sampler thread, since the last reset.
static PEAK_KB: AtomicU64 = AtomicU64::new(0);
static SAMPLER_RUN: AtomicBool = AtomicBool::new(true);

/// Own process RSS in KB, read from `ps` (macOS has no /proc). This is the whole
/// process working set — it counts sqlx/driver buffers, not just Rust heap, which
/// is exactly the "engine memory" the ADR-0006/0010 invariant is about.
fn current_rss_kb() -> u64 {
    let pid = std::process::id().to_string();
    if let Ok(o) = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
    {
        if let Ok(s) = String::from_utf8(o.stdout) {
            if let Ok(kb) = s.trim().parse::<u64>() {
                return kb;
            }
        }
    }
    0
}

fn start_sampler() {
    thread::spawn(|| {
        while SAMPLER_RUN.load(Ordering::Relaxed) {
            PEAK_KB.fetch_max(current_rss_kb(), Ordering::Relaxed);
            thread::sleep(Duration::from_millis(20));
        }
    });
}

fn reset_peak() {
    PEAK_KB.store(current_rss_kb(), Ordering::Relaxed);
}
fn peak_mb() -> f64 {
    PEAK_KB.load(Ordering::Relaxed) as f64 / 1024.0
}
fn rss_mb() -> f64 {
    current_rss_kb() as f64 / 1024.0
}

fn pg_url() -> String {
    std::env::var("SPIKE_PG_URL")
        .unwrap_or_else(|_| "postgres://henrik@localhost:5432/gtfs_bench".into())
}
fn mysql_url() -> String {
    std::env::var("SPIKE_MYSQL_URL")
        .unwrap_or_else(|_| "mysql://root:sftest@localhost:13306/sftest".into())
}

// ---- setup -----------------------------------------------------------------

async fn setup_pg() -> Result<(), BoxErr> {
    let mut c = PgConnection::connect(&pg_url()).await?;
    c.execute(format!("DROP TABLE IF EXISTS {TABLE}").as_str()).await?;
    c.execute(format!("CREATE TABLE {TABLE} (id BIGINT NOT NULL, payload TEXT NOT NULL)").as_str())
        .await?;
    let ins = format!(
        "INSERT INTO {TABLE} SELECT g, repeat('x',{PAYLOAD_LEN}) FROM generate_series(1,{MAX_ROWS}) g"
    );
    c.execute(ins.as_str()).await?;
    let n: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {TABLE}"))
        .fetch_one(&mut c)
        .await?;
    eprintln!("[setup pg]    {TABLE} rows={n} payload={PAYLOAD_LEN}B");
    Ok(())
}

async fn setup_mysql() -> Result<(), BoxErr> {
    let mut c = MySqlConnection::connect(&mysql_url()).await?;
    c.execute(format!("DROP TABLE IF EXISTS {TABLE}").as_str()).await?;
    c.execute(format!("CREATE TABLE {TABLE} (id BIGINT NOT NULL, payload TEXT NOT NULL)").as_str())
        .await?;
    // MySQL 8.0 has no generate_series; build N distinct ids from a 7-way cross join
    // of a 10-row seed (10^7 combos), take MAX_ROWS with LIMIT. No ORDER BY.
    let s = "(SELECT 0 AS d UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9)";
    let ins = format!(
        "INSERT INTO {TABLE} (id,payload) \
         SELECT a.d + b.d*10 + c.d*100 + dd.d*1000 + e.d*10000 + f.d*100000 + g.d*1000000, REPEAT('x',{PAYLOAD_LEN}) \
         FROM {s} a CROSS JOIN {s} b CROSS JOIN {s} c CROSS JOIN {s} dd CROSS JOIN {s} e CROSS JOIN {s} f CROSS JOIN {s} g \
         LIMIT {MAX_ROWS}"
    );
    c.execute(ins.as_str()).await?;
    let n: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {TABLE}"))
        .fetch_one(&mut c)
        .await?;
    eprintln!("[setup mysql] {TABLE} rows={n} payload={PAYLOAD_LEN}B");
    Ok(())
}

async fn teardown() {
    if let Ok(mut c) = PgConnection::connect(&pg_url()).await {
        let _ = c.execute(format!("DROP TABLE IF EXISTS {TABLE}").as_str()).await;
    }
    if let Ok(mut c) = MySqlConnection::connect(&mysql_url()).await {
        let _ = c.execute(format!("DROP TABLE IF EXISTS {TABLE}").as_str()).await;
    }
    eprintln!("[teardown] dropped {TABLE} on pg + mysql");
}

// ---- measurement -----------------------------------------------------------

struct Measured {
    rows: i64,
    ttfr_ms: f64,
    total_ms: f64,
    rss_first_mb: f64,
}

fn emit(engine: &str, buffer: bool, n: i64, baseline: f64, m: &Measured) {
    let mode = if buffer { "buffer" } else { "stream" };
    println!(
        "{engine},{mode},{n},{},{:.3},{:.3},{:.1},{:.1},{:.1}",
        m.rows,
        m.ttfr_ms,
        m.total_ms,
        baseline,
        peak_mb(),
        m.rss_first_mb
    );
}

async fn measure_pg(n: i64, buffer: bool) -> Result<(), BoxErr> {
    let mut c = PgConnection::connect(&pg_url()).await?;
    c.execute("SELECT 1").await?; // warm runtime/socket before timing
    let baseline = rss_mb();
    let sql = format!("SELECT id, payload FROM {TABLE} LIMIT {n}");
    reset_peak();
    let t0 = Instant::now();
    let m = if buffer {
        let v = sqlx::query(&sql).fetch_all(&mut c).await?;
        let mut sum = 0usize;
        for r in &v {
            let p: String = r.try_get("payload")?;
            let _id: i64 = r.try_get("id")?;
            sum += p.len();
        }
        std::hint::black_box(sum);
        Measured { rows: v.len() as i64, ttfr_ms: -1.0, total_ms: t0.elapsed().as_secs_f64() * 1e3, rss_first_mb: -1.0 }
    } else {
        let mut st = sqlx::query(&sql).fetch(&mut c);
        let (mut rows, mut sum) = (0i64, 0usize);
        let (mut ttfr, mut rss_first) = (-1.0, -1.0);
        while let Some(row) = st.try_next().await? {
            if rows == 0 {
                ttfr = t0.elapsed().as_secs_f64() * 1e3;
                rss_first = rss_mb();
            }
            let p: String = row.try_get("payload")?;
            let _id: i64 = row.try_get("id")?;
            sum += p.len();
            rows += 1;
        }
        std::hint::black_box(sum);
        Measured { rows, ttfr_ms: ttfr, total_ms: t0.elapsed().as_secs_f64() * 1e3, rss_first_mb: rss_first }
    };
    emit("pg", buffer, n, baseline, &m);
    Ok(())
}

async fn measure_mysql(n: i64, buffer: bool) -> Result<(), BoxErr> {
    let mut c = MySqlConnection::connect(&mysql_url()).await?;
    c.execute("SELECT 1").await?; // warm runtime/socket before timing
    let baseline = rss_mb();
    let sql = format!("SELECT id, payload FROM {TABLE} LIMIT {n}");
    reset_peak();
    let t0 = Instant::now();
    let m = if buffer {
        let v = sqlx::query(&sql).fetch_all(&mut c).await?;
        let mut sum = 0usize;
        for r in &v {
            let p: String = r.try_get("payload")?;
            let _id: i64 = r.try_get("id")?;
            sum += p.len();
        }
        std::hint::black_box(sum);
        Measured { rows: v.len() as i64, ttfr_ms: -1.0, total_ms: t0.elapsed().as_secs_f64() * 1e3, rss_first_mb: -1.0 }
    } else {
        let mut st = sqlx::query(&sql).fetch(&mut c);
        let (mut rows, mut sum) = (0i64, 0usize);
        let (mut ttfr, mut rss_first) = (-1.0, -1.0);
        while let Some(row) = st.try_next().await? {
            if rows == 0 {
                ttfr = t0.elapsed().as_secs_f64() * 1e3;
                rss_first = rss_mb();
            }
            let p: String = row.try_get("payload")?;
            let _id: i64 = row.try_get("id")?;
            sum += p.len();
            rows += 1;
        }
        std::hint::black_box(sum);
        Measured { rows, ttfr_ms: ttfr, total_ms: t0.elapsed().as_secs_f64() * 1e3, rss_first_mb: rss_first }
    };
    emit("mysql", buffer, n, baseline, &m);
    Ok(())
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), BoxErr> {
    let args: Vec<String> = std::env::args().collect();
    start_sampler();
    let cmd = args.get(1).map(String::as_str).unwrap_or("");
    match cmd {
        "setup" => match args.get(2).map(String::as_str) {
            Some("pg") => setup_pg().await?,
            Some("mysql") => setup_mysql().await?,
            _ => eprintln!("usage: setup <pg|mysql>"),
        },
        "teardown" => teardown().await,
        "measure" => {
            let engine = args.get(2).cloned().unwrap_or_default();
            let n: i64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);
            let buffer = args.get(4).map(|s| s == "buffer").unwrap_or(false);
            match engine.as_str() {
                "pg" => measure_pg(n, buffer).await?,
                "mysql" => measure_mysql(n, buffer).await?,
                _ => eprintln!("usage: measure <pg|mysql> <n> <stream|buffer>"),
            }
        }
        _ => eprintln!("usage: <setup|measure|teardown> ..."),
    }
    SAMPLER_RUN.store(false, Ordering::Relaxed);
    Ok(())
}
