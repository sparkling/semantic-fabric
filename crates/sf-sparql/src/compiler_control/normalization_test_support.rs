//! Hand-counted structural costs for the existing A=2/3 nonconstant fan-out
//! fixtures. These never observe a production run or include owned clone work.
//! Prefixes identify the same guarded clone, not an arbitrary total-1 failure.
use crate::iq::node::{IqCond, IqNode};

const V: u64 = std::mem::size_of::<IqNode>() as u64;
const C: u64 = std::mem::size_of::<IqCond>() as u64;
fn vector(arms: u64) -> u64 {
    arms * (1 + V)
}

pub(crate) fn union(arms: usize) -> (u64, u64) {
    assert!(arms >= 2);
    let (mut capacity, mut growth) = (0, 0);
    for occupied in 0..arms {
        if occupied == capacity {
            capacity = (2 * capacity).max(1);
            growth += (capacity + occupied) as u64 * V;
        }
    }
    let a = arms as u64;
    let local = 1 + 6 * a + growth;
    (1 + vector(a) + 2 * a + local, local)
}

pub(crate) fn construction(arms: usize) -> (u64, u64, u64) {
    let a = arms as u64;
    let (input, output) = union(arms);
    let before = 1 + input + 1 + vector(a) + 1;
    // Each completed arm: recursive entry + dispatch + Box; next arm visit.
    let between = 3 + V;
    let total = 1 + input + 1 + vector(a) + a + a * (2 + V) + output;
    (total, before, between)
}

pub(crate) fn filter(arms: usize) -> (u64, u64, u64) {
    let a = arms as u64;
    let (input, output) = union(arms);
    // One condition: Exists(Values). Collection entry/slot, two visits, box.
    let q = 4 + C + V;
    let before = 1 + input + 1 + q + vector(a) + 1;
    let between = 3 + q + V;
    let total = 1 + input + 1 + q + vector(a) + a + a * (2 + q + V) + output;
    (total, before, between)
}

pub(crate) fn inner(arms: usize) -> (u64, u64, u64, u64) {
    let a = arms as u64;
    let (input, output) = union(arms);
    // Two conditions: Sql marker and Exists(Values).
    let q = 6 + 2 * C + V;
    let nested_filter = 3 + q + V;
    let insertion = 2 + 7 * V; // suffix movement + 2->4 requested slots/relocation
    let per_arm = 29 + q + 13 * V; // 3 retained slots + 0->1->2->4 body growth
    let base = 29 + 7 * V + nested_filter + input + vector(a);
    let total = base + a * (1 + insertion + per_arm) + output;
    (total, base, insertion, per_arm)
}

pub(crate) fn left(arms: usize) -> (u64, u64, u64) {
    let a = arms as u64;
    let (input, output) = union(arms);
    let q = 6 + 2 * C + V;
    let nested_filter = 3 + q + V;
    let per_arm = 5 + q + 2 * V;
    // Root + left dispatch + three scope visits (Union, Filter, Extensional).
    let base = 5 + input + nested_filter + q + vector(a);
    (base + a * (1 + per_arm) + output, base, per_arm)
}
