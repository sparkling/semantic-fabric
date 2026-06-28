//! Pool separation — `tokio` owns async I/O, a **separate** `rayon` pool does
//! CPU-bound term generation (ADR-0006 *Parallelism & dialects*, REQUIRED).
//!
//! Mixing CPU work onto the async runtime's worker threads causes latency spikes,
//! so the discipline is fixed now, before there are callers:
//!
//! * The future OBDA serve endpoint (ADR-0014) runs on **tokio**, which owns all
//!   async source I/O (`tokio-postgres` `query_raw` streaming, the HTTP body).
//! * CPU-bound term generation (`sf-core` `generate_into`) runs on **this
//!   dedicated rayon pool** — never on a tokio worker.
//! * CPU work invoked **from** an async context goes through `spawn_blocking`
//!   (which then dispatches onto [`term_gen_pool`]); the two pools never share
//!   threads.
//!
//! The synchronous SQLite execution path in this wave ([`crate::exec`]) is not
//! async, so it generates terms inline; this module establishes the pool and the
//! `run_cpu` entry point so the separation is structural, not an afterthought.

use std::sync::OnceLock;

use rayon::{ThreadPool, ThreadPoolBuilder};

static TERM_GEN_POOL: OnceLock<ThreadPool> = OnceLock::new();

/// The dedicated CPU pool for term generation. Distinct from any tokio runtime
/// (ADR-0006): tokio worker threads must never run this work directly — an async
/// caller hops here via `spawn_blocking` + [`run_cpu`].
pub fn term_gen_pool() -> &'static ThreadPool {
    TERM_GEN_POOL.get_or_init(|| {
        ThreadPoolBuilder::new()
            .thread_name(|i| format!("sf-termgen-{i}"))
            .build()
            .expect("term-gen rayon pool builds")
    })
}

/// Run a CPU-bound closure on the dedicated term-gen pool, off any async runtime
/// (ADR-0006 pool separation). Async callers wrap this in `spawn_blocking`.
pub fn run_cpu<R, F>(f: F) -> R
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    term_gen_pool().install(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_work_runs_on_the_dedicated_pool() {
        // The closure executes on a sf-termgen worker thread, not the caller's.
        let on_pool = run_cpu(|| {
            std::thread::current()
                .name()
                .map(|n| n.starts_with("sf-termgen-"))
                .unwrap_or(false)
        });
        assert!(on_pool, "term-gen must run on the dedicated rayon pool");
    }

    #[test]
    fn pool_is_a_singleton() {
        let a = term_gen_pool() as *const ThreadPool;
        let b = term_gen_pool() as *const ThreadPool;
        assert_eq!(a, b, "one shared CPU pool, never per-call");
    }
}
