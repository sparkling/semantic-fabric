//! A process-wide heap high-water-mark probe (ADR-0006 *Streaming & bounded
//! memory*). It is the byte-valued sibling of `sf-core`'s allocation-count probe:
//! where that proves the term-gen path is *alloc-free per row*, this proves the
//! whole streaming path holds a **bounded peak heap** as source data grows — the
//! `O(|T| + |M| + batch)` invariant, independent of result/source size.
//!
//! [`Tracking`] forwards every call to the system allocator and only maintains
//! two atomics: `CURRENT` (live bytes = allocated − freed) and `PEAK` (the
//! high-water of `CURRENT` since the last [`reset_peak`]). It allocates nothing
//! itself, so installing it as the `#[global_allocator]` of a test/bench binary
//! is measurement-safe.
//!
//! ## Usage (test / bench binary root)
//!
//! ```ignore
//! #[global_allocator]
//! static GLOBAL: sf_bench::mem::Tracking = sf_bench::mem::Tracking;
//! ```
//!
//! Then bracket the streaming window: [`reset_peak`] → run → [`window_peak`].
//! The library crate deliberately does **not** install the allocator (so it never
//! perturbs `sf-cli`); only the bench/test roots opt in.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicI64, Ordering};

static CURRENT: AtomicI64 = AtomicI64::new(0);
static PEAK: AtomicI64 = AtomicI64::new(0);

/// A high-water-tracking global allocator. Forwards to [`System`]; the only added
/// work is two relaxed atomic updates, so it does not itself allocate.
pub struct Tracking;

// SAFETY: every method forwards to the System allocator unchanged; the added work
// is non-allocating atomic accounting (cf. sf-core's counting allocator).
unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            record(layout.size() as i64);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        CURRENT.fetch_sub(layout.size() as i64, Ordering::Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = System.realloc(ptr, layout, new_size);
        if !new_ptr.is_null() {
            record(new_size as i64 - layout.size() as i64);
        }
        new_ptr
    }
}

/// Apply a live-bytes delta and lift `PEAK` to the new `CURRENT` if it rose.
fn record(delta: i64) {
    let current = CURRENT.fetch_add(delta, Ordering::Relaxed) + delta;
    let mut peak = PEAK.load(Ordering::Relaxed);
    while current > peak {
        match PEAK.compare_exchange_weak(peak, current, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(observed) => peak = observed,
        }
    }
}

/// Live heap bytes right now (allocated − freed).
pub fn current_bytes() -> i64 {
    CURRENT.load(Ordering::Relaxed)
}

/// Reset the high-water mark to the current live total and return that baseline.
/// Call immediately before the streaming window; pair with [`window_peak`].
pub fn reset_peak() -> i64 {
    let base = CURRENT.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    base
}

/// The absolute high-water of live bytes since the last [`reset_peak`].
pub fn peak_bytes() -> i64 {
    PEAK.load(Ordering::Relaxed)
}

/// Peak **additional** live bytes over `baseline` (the value [`reset_peak`]
/// returned): the engine's working-set high-water during the window, with the
/// pre-existing fixed footprint (`⟨T, M⟩`, statement, buffers) netted out.
pub fn window_peak(baseline: i64) -> i64 {
    (peak_bytes() - baseline).max(0)
}
