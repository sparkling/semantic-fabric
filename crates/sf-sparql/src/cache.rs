//! Plan cache (ADR-0007 *Performance*) — keyed on a **structural hash of the
//! SPARQL algebra**, invalidated by a monotonic `⟨T, M⟩` + source-schema
//! **epoch** (bumped on ontology reload, mapping reload, or a schema change). The
//! cache is sized by `⟨T, M⟩`, never by data, so it cannot go stale against the
//! live sources.
//!
//! **Sharp keying rule (ADR-0007):** parameterise *data* constants but key on
//! *schema-selecting* constants (predicate IRIs and IRI-template constants — the
//! ones that decide which mapping entries/columns to unfold), so a plan compiled
//! for `:a` never serves a `:b` query.
//!
//! v1 keying is **conservative**: the structural key is the full canonical
//! algebra string (via `Display`), so *every* constant — including data ones — is
//! in the key. This is strictly safe (it can only cause extra misses, never a
//! wrong hit); the data/schema split that lets two `FILTER(?x = <data>)` queries
//! share one plan is the documented refinement (ADR-0007), tracked here.

use spargebra::Query;

/// A monotonic `⟨T, M⟩` + schema epoch. Bump it whenever the ontology, the
/// mappings, or a source schema changes; all plans from an older epoch are dead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Epoch(pub u64);

impl Epoch {
    pub fn bump(&mut self) {
        self.0 += 1;
    }
}

/// The structural cache key: `(epoch, algebra-hash)` plus the **canonical algebra
/// string** that disambiguates a 64-bit hash collision. `Eq` compares the
/// canonical string, so two distinct queries that happen to share a
/// `structural_hash` at the same epoch can never collide onto one plan — closing
/// the hazard ADR-0007 *sharp keying* warns about (a plan for `:a` serving `:b`).
/// `Hash` uses only the fast `(epoch, structural_hash)` pre-hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanKey {
    pub epoch: u64,
    pub structural_hash: u64,
    pub canonical: String,
}

impl std::hash::Hash for PlanKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.epoch.hash(state);
        self.structural_hash.hash(state);
    }
}

/// Compute the structural key for `query` at `epoch` (ADR-0007). Conservative:
/// the canonical algebra rendering retains the schema-selecting constants
/// (predicate IRIs, template constants) — and, for now, data constants too — and
/// is also stored verbatim so equality is exact, never hash-only.
pub fn plan_key(query: &Query, epoch: Epoch) -> PlanKey {
    use std::hash::{Hash, Hasher};
    let canonical = query.to_string();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    canonical.hash(&mut h);
    PlanKey {
        epoch: epoch.0,
        structural_hash: h.finish(),
        canonical,
    }
}

/// A bounded plan cache. Generic over the cached plan type `P` so the cache does
/// not couple to the (large) plan struct. Bounded by `⟨T, M⟩` size via `capacity`
/// — backed by `quick_cache` (ADR-0007's named production drop-in): an
/// approximately-LRU sharded cache that evicts individual cold entries under
/// pressure, never the whole map at once (the prior `HashMap` + clear-on-overflow
/// collapsed the hit rate to ~0 past `capacity` distinct keys — M4 wave-2 finding 1).
pub struct PlanCache<P> {
    inner: quick_cache::sync::Cache<PlanKey, P>,
}

impl<P: Clone> PlanCache<P> {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: quick_cache::sync::Cache::new(capacity),
        }
    }

    /// Look up a compiled plan.
    pub fn get(&self, key: &PlanKey) -> Option<P> {
        self.inner.get(key)
    }

    /// Insert a compiled plan. Eviction (approximately-LRU, `quick_cache`) drops
    /// individual cold entries as capacity is reached — the cache is
    /// `⟨T, M⟩`-bounded, so eviction rarely fires in practice.
    pub fn put(&self, key: PlanKey, plan: P) {
        self.inner.insert(key, plan);
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spargebra::SparqlParser;

    fn parse(q: &str) -> Query {
        SparqlParser::new().parse_query(q).unwrap()
    }

    #[test]
    fn same_query_same_key() {
        let e = Epoch(3);
        let a = plan_key(&parse("SELECT * WHERE { ?s ?p ?o }"), e);
        let b = plan_key(&parse("SELECT * WHERE { ?s ?p ?o }"), e);
        assert_eq!(a, b);
    }

    #[test]
    fn schema_selecting_constant_changes_key() {
        // A different predicate IRI selects different mapping entries → must not
        // share a plan (ADR-0007 sharp keying).
        let e = Epoch(0);
        let a = plan_key(&parse("SELECT ?x WHERE { ?x <http://ex/a> ?y }"), e);
        let b = plan_key(&parse("SELECT ?x WHERE { ?x <http://ex/b> ?y }"), e);
        assert_ne!(a, b);
    }

    #[test]
    fn hash_collision_does_not_serve_the_wrong_plan() {
        // Force a structural_hash collision between two *distinct* queries at the
        // same epoch: equality must still distinguish them (ADR-0007 sharp keying),
        // so the cache returns a miss for the second, never `:a`'s plan for `:b`.
        let cache: PlanCache<u32> = PlanCache::new(8);
        let mut ka = plan_key(&parse("SELECT ?x WHERE { ?x <http://ex/a> ?y }"), Epoch(0));
        let mut kb = plan_key(&parse("SELECT ?x WHERE { ?x <http://ex/b> ?y }"), Epoch(0));
        // Pin both to the same pre-hash bucket (a forced collision).
        ka.structural_hash = 42;
        kb.structural_hash = 42;
        assert_ne!(
            ka, kb,
            "distinct canonical algebra ⇒ distinct keys despite equal hash"
        );
        cache.put(ka.clone(), 1);
        assert_eq!(cache.get(&ka), Some(1));
        assert_eq!(
            cache.get(&kb),
            None,
            "a collision must not serve the wrong plan"
        );
    }

    #[test]
    fn epoch_bump_invalidates() {
        let q = parse("SELECT * WHERE { ?s ?p ?o }");
        assert_ne!(plan_key(&q, Epoch(1)), plan_key(&q, Epoch(2)));
    }

    #[test]
    fn cache_round_trips_and_is_bounded() {
        let cache: PlanCache<u32> = PlanCache::new(2);
        let k1 = plan_key(&parse("SELECT * WHERE { ?a ?b ?c }"), Epoch(0));
        cache.put(k1.clone(), 10);
        assert_eq!(cache.get(&k1), Some(10));
        // Overflow evicts approximately-LRU, never wholesale (quick_cache).
        cache.put(
            plan_key(&parse("SELECT * WHERE { ?d ?e ?f }"), Epoch(0)),
            20,
        );
        cache.put(
            plan_key(&parse("SELECT * WHERE { ?g ?h ?i }"), Epoch(0)),
            30,
        );
        assert!(cache.len() <= 2);
    }

    /// A synthetic key distinguished only by `canonical` (real `plan_key` overkill
    /// for a hit-rate workload of thousands of accesses).
    fn synth_key(id: usize) -> PlanKey {
        PlanKey {
            epoch: 0,
            structural_hash: id as u64,
            canonical: format!("synthetic-plan-{id}"),
        }
    }

    /// M4 wave-2 finding 1 RECEIPT: a realistic hot/cold workload — a small hot
    /// working set (well within `capacity`) accessed repeatedly, interleaved with
    /// a much larger cold set each touched rarely (so the cache overflows
    /// `capacity` many times over). The prior `HashMap` + clear-on-overflow wipes
    /// the whole map — including the hot set — every time a cold miss pushes it
    /// over capacity, so hot-key hit rate stays near zero; `quick_cache`'s
    /// approximately-LRU eviction should keep the hot set resident and answer most
    /// hot accesses from cache. Get-or-put on every access (the real
    /// `parse_and_translate_cached` call pattern); asserts only a generous
    /// floor, since the interesting number is the OLD-vs-NEW comparison reported
    /// alongside this test, not a tight bound on `quick_cache`'s internals.
    #[test]
    fn hot_working_set_survives_cold_churn_past_capacity() {
        const CAPACITY: usize = 64;
        const HOT: usize = 32;
        const COLD: usize = 128;
        const ITERS: usize = 3000;

        let cache: PlanCache<u32> = PlanCache::new(CAPACITY);
        let mut hits = 0u32;
        let mut accesses = 0u32;
        for i in 0..ITERS {
            // 2/3 of accesses hit a small, fixed hot set (round-robin); 1/3 hit a
            // much larger cold set that churns through far more distinct keys than
            // fit in `capacity`, forcing the cache to evict repeatedly.
            let key = if i % 3 != 0 {
                synth_key(i % HOT)
            } else {
                synth_key(HOT + (i / 3) % COLD)
            };
            accesses += 1;
            if cache.get(&key).is_some() {
                hits += 1;
            } else {
                cache.put(key, i as u32);
            }
        }
        let hit_rate = f64::from(hits) / f64::from(accesses);
        eprintln!(
            "PlanCache hot/cold hit rate over {ITERS} accesses ({HOT} hot + {COLD} cold keys, \
             capacity {CAPACITY}): {hits}/{accesses} = {hit_rate:.3}"
        );
        assert!(
            hit_rate > 0.5,
            "hot working set should survive cold churn past capacity, got hit_rate={hit_rate:.3}"
        );
    }
}
