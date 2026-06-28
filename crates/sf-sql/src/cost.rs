//! Cross-source semi-join **cost planner** (ADR-0006 *Cross-source semi-join
//! cost*).
//!
//! The cross-source semi-join — combining tables that live in *different*
//! relational databases — is the engine's one genuinely in-process join
//! decision. Everything else is pushed into a single source's SQL (ADR-0006
//! *Relational execution*). Because of that, this planner is **cost-driven from
//! the start**: a foundational, baked-in decision, since retrofitting cost once
//! the planner has callers is expensive.
//!
//! Given each side's distinct-key cardinality (from source catalogs / sketches —
//! see [`crate::schema`], ADR-0015), [`plan_semijoin`] decides three things:
//!
//! * **Side selection** — ship the *smaller distinct-key* side's keys to the
//!   larger source. "Smaller" is by **distinct-key cardinality**, not raw row
//!   count (ADR-0006).
//! * **Reducer form & sizing** — `IN`-list vs temp-table vs Bloom filter, chosen
//!   by the shipped distinct-key count, each kept inside a **fixed memory
//!   budget** so the reducer never breaks the bounded-memory invariant (ADR-0010).
//! * **Skip-if-unselective gate** — if the estimated post-reduction *survival
//!   ratio* is ≈ 1 the reducer would eliminate almost nothing, so skip the
//!   round-trip and stream-merge the inputs directly (ADR-0006).
//!
//! Estimation inputs beyond catalog stats — HLL/Bloom distinct-count sketches
//! where catalogs are thin, and at most one cached `EXPLAIN (FORMAT JSON)` leaf
//! probe — are deferred (ADR-0006 *Cross-source semi-join cost*); this module
//! consumes the resulting [`SideStats`] estimates and owns the *decision*.

/// Which input of the cross-source join a [`Plan`] refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    /// The other side.
    pub fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

/// Cardinality estimates for one join input, on its join key.
///
/// `distinct_keys` (distinct values of the join key) drives both side selection
/// and the survival-ratio estimate; `rows` is retained for context/diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SideStats {
    /// Distinct values of the join key on this side (the selectivity driver).
    pub distinct_keys: u64,
    /// Total rows on this side.
    pub rows: u64,
}

impl SideStats {
    /// A side with `distinct_keys` distinct join-key values over `rows` rows.
    pub fn new(distinct_keys: u64, rows: u64) -> Self {
        Self {
            distinct_keys,
            rows,
        }
    }
}

/// The chosen reducer representation for the shipped key set. Each form is
/// bounded in memory: `InList`/`TempTable` carry an explicit shipped-key count,
/// and `Bloom` is sized under a hard bit cap ([`CostConfig::bloom_max_bits`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReducerForm {
    /// Few keys → a bound `IN ($1, $2, …)` list of `keys` parameters.
    InList { keys: u64 },
    /// More keys → ship them as a temp table and join on the larger source.
    TempTable { keys: u64 },
    /// Many keys → a fixed-size Bloom filter (`bits` long, `hashes` hash fns),
    /// sized under the bit cap so memory stays bounded even as `keys` grows.
    Bloom { keys: u64, bits: u64, hashes: u8 },
}

/// The planner's decision for a cross-source join.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Plan {
    /// The reducer would eliminate almost nothing (survival ratio ≈ 1): skip it
    /// and stream-merge the inputs directly (ADR-0006 skip-if-unselective gate).
    SkipMerge {
        /// Estimated fraction of probe rows that would survive reduction (≈ 1).
        survival_ratio: f64,
    },
    /// Ship `build`'s keys (as `reducer`) to `probe`'s source, then merge.
    SemiJoin {
        /// The side whose distinct keys are shipped (the smaller distinct side).
        build: Side,
        /// The larger-distinct side the reducer is applied against.
        probe: Side,
        /// How the shipped key set is represented and sized.
        reducer: ReducerForm,
        /// Estimated fraction of probe rows surviving reduction (< the skip gate).
        survival_ratio: f64,
    },
}

/// Tunable thresholds for the cost model. [`Default`] bakes in the production
/// defaults; tests override them to exercise each branch deterministically.
#[derive(Debug, Clone, Copy)]
pub struct CostConfig {
    /// Shipped distinct keys ≤ this → `IN`-list reducer.
    pub in_list_max: u64,
    /// Shipped distinct keys ≤ this (and > `in_list_max`) → temp-table reducer;
    /// above it → Bloom filter.
    pub temp_table_max: u64,
    /// Survival ratio ≥ this → skip the reducer (it is unselective; ≈ 1).
    pub skip_ratio: f64,
    /// Target Bloom false-positive rate used to size the filter.
    pub bloom_target_fp: f64,
    /// Hard cap on Bloom filter size in bits — the bounded-memory floor
    /// (ADR-0010): sizing never exceeds this regardless of key count.
    pub bloom_max_bits: u64,
}

impl Default for CostConfig {
    fn default() -> Self {
        Self {
            in_list_max: 128,
            temp_table_max: 100_000,
            skip_ratio: 0.9,
            bloom_target_fp: 0.01,
            bloom_max_bits: 8 * 1024 * 1024, // 1 MiB — bounded regardless of |keys|
        }
    }
}

/// Plan a cross-source semi-join from the two sides' cardinality estimates.
///
/// Side selection ships the smaller distinct-key side; the survival-ratio gate
/// may turn the plan into [`Plan::SkipMerge`]; otherwise the reducer form is
/// chosen by the shipped distinct-key count.
pub fn plan_semijoin(left: SideStats, right: SideStats, cfg: &CostConfig) -> Plan {
    // Side selection (ADR-0006): build = the smaller distinct-key side — we ship
    // the fewer keys, and by containment its keys sit inside the probe's domain.
    let (build, build_stats, probe_stats) = if left.distinct_keys <= right.distinct_keys {
        (Side::Left, left, right)
    } else {
        (Side::Right, right, left)
    };
    let probe = build.opposite();

    // Survival-ratio estimate. Under containment (build has ≤ distinct keys, so
    // build keys ⊆ probe key domain) and uniform rows-per-key on the probe, the
    // fraction of probe rows surviving the reducer ≈ build_distinct /
    // probe_distinct, capped at 1.
    let survival_ratio = survival_ratio(build_stats.distinct_keys, probe_stats.distinct_keys);

    // Skip-if-unselective: ≈ 1 means the reducer earns nothing.
    if survival_ratio >= cfg.skip_ratio {
        return Plan::SkipMerge { survival_ratio };
    }

    let reducer = choose_reducer(build_stats.distinct_keys, cfg);
    Plan::SemiJoin {
        build,
        probe,
        reducer,
        survival_ratio,
    }
}

/// Fraction of probe rows expected to survive shipping `build_distinct` keys to a
/// probe with `probe_distinct` distinct keys (capped at 1; a zero/absent probe
/// estimate is treated as "no reduction").
fn survival_ratio(build_distinct: u64, probe_distinct: u64) -> f64 {
    if probe_distinct == 0 {
        return 1.0;
    }
    (build_distinct as f64 / probe_distinct as f64).min(1.0)
}

/// Pick the reducer form by shipped distinct-key count, keeping each form within
/// the memory budget.
fn choose_reducer(keys: u64, cfg: &CostConfig) -> ReducerForm {
    if keys <= cfg.in_list_max {
        ReducerForm::InList { keys }
    } else if keys <= cfg.temp_table_max {
        ReducerForm::TempTable { keys }
    } else {
        let (bits, hashes) = bloom_sizing(keys, cfg.bloom_target_fp, cfg.bloom_max_bits);
        ReducerForm::Bloom { keys, bits, hashes }
    }
}

/// Standard Bloom sizing — `m = ⌈-n·ln(p) / (ln 2)²⌉`, `k = round((m/n)·ln 2)` —
/// then **clamp `m` to `max_bits`** so memory stays bounded (ADR-0010). When the
/// cap binds, the realised false-positive rate rises but memory does not.
fn bloom_sizing(n: u64, target_fp: f64, max_bits: u64) -> (u64, u8) {
    let n_f = n.max(1) as f64;
    let ln2 = std::f64::consts::LN_2;
    let ideal_bits = (-n_f * target_fp.ln() / (ln2 * ln2)).ceil();
    let bits = (ideal_bits.max(1.0) as u64).min(max_bits.max(1));
    let hashes = (((bits as f64 / n_f) * ln2).round() as i64).clamp(1, 30) as u8;
    (bits, hashes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR-0006 *Confirmation*: "the cross-source semi-join planner selects side,
    /// reducer form, and skip-vs-reduce from catalog/sketch estimates —
    /// unit-tested against synthetic cardinalities (small, large, and ≈ 1
    /// reduction)." This is that test.
    #[test]
    fn plans_small_large_and_near_one_reduction() {
        let cfg = CostConfig::default();

        // --- small reduction: very selective, few keys → SemiJoin + IN-list ---
        // 10 distinct keys shipped against 1,000,000 → survival ≈ 1e-5.
        let small = plan_semijoin(
            SideStats::new(10, 50),
            SideStats::new(1_000_000, 5_000_000),
            &cfg,
        );
        match small {
            Plan::SemiJoin {
                build,
                probe,
                reducer,
                survival_ratio,
            } => {
                assert_eq!(build, Side::Left, "ship the smaller distinct-key side");
                assert_eq!(probe, Side::Right);
                assert_eq!(reducer, ReducerForm::InList { keys: 10 });
                assert!(
                    survival_ratio < 0.001,
                    "should be very selective: {survival_ratio}"
                );
            }
            other => panic!("expected selective SemiJoin, got {other:?}"),
        }

        // --- large reduction: still selective but many keys → SemiJoin + Bloom,
        //     sized under the hard bit cap (bounded memory). ---
        let large = plan_semijoin(
            SideStats::new(5_000_000, 20_000_000),
            SideStats::new(100_000_000, 400_000_000),
            &cfg,
        );
        match large {
            Plan::SemiJoin {
                build,
                reducer,
                survival_ratio,
                ..
            } => {
                assert_eq!(build, Side::Left);
                match reducer {
                    ReducerForm::Bloom { keys, bits, hashes } => {
                        assert_eq!(keys, 5_000_000);
                        assert_eq!(
                            bits, cfg.bloom_max_bits,
                            "Bloom must be capped → bounded memory"
                        );
                        assert!(hashes >= 1);
                    }
                    other => panic!("expected Bloom for 5M keys, got {other:?}"),
                }
                assert!(
                    survival_ratio < cfg.skip_ratio,
                    "still worth reducing: {survival_ratio}"
                );
            }
            other => panic!("expected SemiJoin, got {other:?}"),
        }

        // --- near-1 reduction: build ≈ probe distinct → skip the reducer ---
        let near_one = plan_semijoin(
            SideStats::new(950_000, 1_000_000),
            SideStats::new(1_000_000, 1_000_000),
            &cfg,
        );
        match near_one {
            Plan::SkipMerge { survival_ratio } => {
                assert!(
                    survival_ratio >= cfg.skip_ratio,
                    "≈1 survival → skip: {survival_ratio}"
                );
            }
            other => panic!("expected SkipMerge for ≈1 reduction, got {other:?}"),
        }
    }

    /// Side selection is by distinct-key cardinality, not raw rows: a side with
    /// *more rows* but *fewer distinct keys* is still the build side.
    #[test]
    fn side_selection_uses_distinct_keys_not_rows() {
        let cfg = CostConfig::default();
        // Right: many rows, few distinct keys → it is the build (shipped) side.
        let plan = plan_semijoin(
            SideStats::new(900, 1_000),      // left: 900 distinct
            SideStats::new(50, 100_000_000), // right: 50 distinct, huge rows
            &cfg,
        );
        match plan {
            Plan::SemiJoin {
                build,
                probe,
                reducer,
                ..
            } => {
                assert_eq!(
                    build,
                    Side::Right,
                    "fewer distinct keys wins, despite more rows"
                );
                assert_eq!(probe, Side::Left);
                assert_eq!(reducer, ReducerForm::InList { keys: 50 });
            }
            other => panic!("expected SemiJoin, got {other:?}"),
        }
    }

    /// The middle reducer band → temp table.
    #[test]
    fn medium_distinct_count_selects_temp_table() {
        let cfg = CostConfig::default();
        let plan = plan_semijoin(
            SideStats::new(10_000, 20_000), // > in_list_max (128), ≤ temp_table_max
            SideStats::new(50_000_000, 200_000_000),
            &cfg,
        );
        match plan {
            Plan::SemiJoin { reducer, .. } => {
                assert_eq!(reducer, ReducerForm::TempTable { keys: 10_000 });
            }
            other => panic!("expected SemiJoin/TempTable, got {other:?}"),
        }
    }

    /// Exactly-equal distinct counts → survival ratio 1.0 → skip.
    #[test]
    fn equal_distinct_counts_skip() {
        let cfg = CostConfig::default();
        let plan = plan_semijoin(
            SideStats::new(1_000, 2_000),
            SideStats::new(1_000, 9_000),
            &cfg,
        );
        assert!(matches!(plan, Plan::SkipMerge { .. }));
    }

    /// Bloom sizing is bounded: arbitrarily many keys never exceed the bit cap,
    /// and the cap binds well before pathological sizes.
    #[test]
    fn bloom_sizing_is_bounded_by_cap() {
        let cap = 1_000_000;
        let (bits_small, k_small) = bloom_sizing(1_000, 0.01, cap);
        assert!(bits_small <= cap && bits_small > 0);
        assert!(k_small >= 1);

        let (bits_huge, _) = bloom_sizing(u64::MAX, 0.0001, cap);
        assert_eq!(
            bits_huge, cap,
            "sizing must clamp to the cap for huge key sets"
        );
    }

    /// The IN-list boundary is inclusive of `in_list_max`.
    #[test]
    fn in_list_boundary_is_inclusive() {
        let cfg = CostConfig::default();
        assert_eq!(
            choose_reducer(cfg.in_list_max, &cfg),
            ReducerForm::InList {
                keys: cfg.in_list_max
            }
        );
        assert_eq!(
            choose_reducer(cfg.in_list_max + 1, &cfg),
            ReducerForm::TempTable {
                keys: cfg.in_list_max + 1
            }
        );
    }
}
