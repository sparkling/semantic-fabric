//! Pass (3) of the optimizer cascade — functional-dependency inference
//! (transitive closure). Kept in its own file to hold `cascade/mod.rs` within
//! the size budget; the result gates pass (4) FK/PK join elimination.

use sf_core::ir::LogicalSource;
use sf_sql::TableSchema;

use crate::iq::{Branch, ColRef, SqlCond};

/// The functional dependencies that hold over a branch's row stream. An entry
/// `(det, alias)` means **`det` determines every column of scan `alias`** (a
/// superkey of that scan's projection). Built for pass (4): a join may be
/// eliminated only once uniqueness is proven, and uniqueness is exactly "the
/// join column is a key" — an FD whose determinant is that column.
#[derive(Debug, Default)]
pub struct Fds {
    /// `(det, alias)`: `det` functionally determines all columns of `alias`.
    pub(super) deps: Vec<(ColRef, usize)>,
}

impl Fds {
    pub(super) fn has(&self, det: &ColRef, alias: usize) -> bool {
        self.deps.iter().any(|(d, a)| d == det && *a == alias)
    }

    pub(super) fn add(&mut self, det: ColRef, alias: usize) -> bool {
        if self.has(&det, alias) {
            false
        } else {
            self.deps.push((det, alias));
            true
        }
    }

    /// Is `c` a key — does it determine its own scan's whole row? This is the
    /// uniqueness precondition pass (4) consults.
    pub fn is_key(&self, c: &ColRef) -> bool {
        self.has(c, c.alias)
    }
}

/// Derive the FD set with its **transitive closure** (ADR-0007 step iii — "FD
/// inference, transitive closure, through unions, *must precede* FK/PK join
/// elimination"). Seeds each single-column unique key as a key→row FD, then
/// closes to a fixpoint under two sound rules:
///
/// * **equality** — for a core key equality `ColEq(a, b)` (`a` and `b` hold the
///   same value on every surviving row), anything `a` determines `b` also
///   determines, and vice-versa.
/// * **transitivity** — if `x` determines all of scan `m` and a column `y` of
///   `m` determines scan `n`, then `x` determines `n`.
///
/// "Through unions" is honoured at the branch granularity: each UNION arm is a
/// separate [`Branch`], so its FDs are inferred independently and a join is
/// eliminated per-arm only on that arm's proven keys.
pub fn infer_functional_dependencies(b: &Branch, schema: &[TableSchema]) -> Fds {
    let mut fds = Fds::default();
    // Seed: every single-column unique key (PK or UNIQUE) determines its row.
    for scan in &b.core {
        if let LogicalSource::Table(t) = &scan.source {
            if let Some(ts) = schema.iter().find(|s| &s.name == t) {
                for col in single_col_keys(ts) {
                    fds.add(ColRef::new(scan.alias, col), scan.alias);
                }
            }
        }
    }
    // Closure to a fixpoint.
    loop {
        let mut changed = false;
        for cond in &b.where_conds {
            if let SqlCond::ColEq(a, c) = cond {
                changed |= propagate_eq(&mut fds, a, c);
                changed |= propagate_eq(&mut fds, c, a);
            }
        }
        // transitivity — snapshot the current edges to avoid borrow conflicts.
        let snapshot: Vec<(ColRef, usize)> = fds.deps.clone();
        for (x, m) in &snapshot {
            for (y, n) in &snapshot {
                if y.alias == *m && fds.add(x.clone(), *n) {
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    fds
}

/// Equality rule: given `ColEq(from, to)` (equal values), copy every FD whose
/// determinant is `from` onto `to`. Returns whether anything was added.
fn propagate_eq(fds: &mut Fds, from: &ColRef, to: &ColRef) -> bool {
    let targets: Vec<usize> = fds
        .deps
        .iter()
        .filter(|(d, _)| d == from)
        .map(|(_, a)| *a)
        .collect();
    let mut changed = false;
    for a in targets {
        changed |= fds.add(to.clone(), a);
    }
    changed
}

/// The single-column unique keys of a table (the determinants that fix a row):
/// a single-column primary key, plus any single-column `UNIQUE` constraint.
pub fn single_col_keys(ts: &TableSchema) -> Vec<String> {
    let mut keys = Vec::new();
    if ts.primary_key.len() == 1 {
        keys.push(ts.primary_key[0].clone());
    }
    for u in &ts.unique {
        if u.len() == 1 && !keys.contains(&u[0]) {
            keys.push(u[0].clone());
        }
    }
    keys
}
