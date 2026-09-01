//! Plan cache (ADR-0007 *Performance*) — owned by one immutable
//! [`CompilerBinding`] and keyed on both its opaque compile scope and a
//! **structural hash of the SPARQL algebra**.
//!
//! The scope binds one source ID, mapping, T-Box, observed schema, dialect, and
//! cache. A new binding gets a new process-unique identity; its epoch is an
//! explicit generation marker. No live schema watcher or reload path exists in
//! the current server, so this module does **not** claim that later database DDL
//! bumps the epoch or invalidates an already-running binding.
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

use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

use sf_core::{SourceId, SourceMapping};
use sf_sql::{Dialect, TableSchema};
use spargebra::Query;

use crate::{Plan, Result, Tbox};

/// A compile-binding generation marker.
///
/// The current server constructs one immutable generation and has no live
/// reload/drift detector. A future reload path must build a new binding or bump
/// this marker after observing a coherent replacement snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Epoch(pub u64);

impl Epoch {
    /// Advance the generation, failing closed rather than wrapping to an old
    /// cache namespace.
    pub fn bump(&mut self) {
        self.0 = self.0.checked_add(1).expect("compile epoch exhausted");
    }
}

static NEXT_BINDING_ID: AtomicU64 = AtomicU64::new(1);

/// A process-unique, non-secret identity for one immutable compiler binding.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct CompileBindingId(NonZeroU64);

impl CompileBindingId {
    fn mint() -> Self {
        let value = NEXT_BINDING_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .expect("compiler binding identity exhausted");
        Self(NonZeroU64::new(value).expect("binding IDs start at one"))
    }

    const fn get(self) -> u64 {
        self.0.get()
    }
}

impl fmt::Debug for CompileBindingId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "binding[{}]", self.get())
    }
}

/// Exact cache namespace for one compiler binding and generation.
///
/// Construction is private so callers cannot accidentally reuse an identity for
/// a different mapping/schema/T-Box/backend context. Obtain it from
/// [`CompilerBinding::scope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CompileScope {
    binding: CompileBindingId,
    dialect: Dialect,
    epoch: Epoch,
}

impl CompileScope {
    fn new(binding: CompileBindingId, dialect: Dialect, epoch: Epoch) -> Self {
        Self {
            binding,
            dialect,
            epoch,
        }
    }

    /// Process-local binding identity. This is diagnostic metadata, not a
    /// persistent source identifier or release receipt.
    pub const fn binding_id(self) -> u64 {
        self.binding.get()
    }

    pub const fn dialect(self) -> Dialect {
        self.dialect
    }

    pub const fn epoch(self) -> Epoch {
        self.epoch
    }
}

/// Immutable semantic inputs and cache for one source-local compiler.
///
/// Grouping these values makes dialect/context mismatch unrepresentable on the
/// cached translation path. Creating a replacement mapping, ontology, schema,
/// or backend requires a new binding and therefore a fresh cache namespace.
pub struct CompilerBinding {
    mapping: SourceMapping,
    dialect: Dialect,
    tbox: Tbox,
    schema: Vec<TableSchema>,
    cache: PlanCache<CachedPlan>,
    scope: CompileScope,
}

impl CompilerBinding {
    pub fn new(
        mapping: SourceMapping,
        dialect: Dialect,
        tbox: Tbox,
        schema: Vec<TableSchema>,
        cache_capacity: usize,
    ) -> Self {
        assert!(cache_capacity > 0, "plan cache capacity must be non-zero");
        let scope = CompileScope::new(CompileBindingId::mint(), dialect, Epoch::default());
        Self {
            mapping,
            dialect,
            tbox,
            schema,
            cache: PlanCache::new(cache_capacity),
            scope,
        }
    }

    /// Parse and compile against this binding's inseparable semantic context.
    pub fn compile(&self, sparql: &str) -> Result<Plan> {
        crate::parse_and_translate_cached(sparql, self)
    }

    pub const fn source_id(&self) -> SourceId {
        self.mapping.source_id()
    }

    pub const fn dialect(&self) -> Dialect {
        self.dialect
    }

    pub const fn scope(&self) -> CompileScope {
        self.scope
    }

    pub(crate) fn triples_maps(&self) -> &[sf_core::ir::TriplesMap] {
        self.mapping.triples_maps()
    }

    pub(crate) fn tbox(&self) -> &Tbox {
        &self.tbox
    }

    pub(crate) fn schema(&self) -> &[TableSchema] {
        &self.schema
    }

    pub(crate) fn cache(&self) -> &PlanCache<CachedPlan> {
        &self.cache
    }

    #[cfg(test)]
    pub(crate) fn cache_len(&self) -> usize {
        self.cache.len()
    }
}

/// Cached artifact carrying its own scope as a second fail-closed check against
/// a wrongly inserted value. Kept crate-private so no unverified plan escapes.
#[derive(Clone)]
pub(crate) struct CachedPlan {
    scope: CompileScope,
    plan: Plan,
}

impl CachedPlan {
    pub(crate) const fn new(scope: CompileScope, plan: Plan) -> Self {
        Self { scope, plan }
    }

    pub(crate) const fn scope(&self) -> CompileScope {
        self.scope
    }

    pub(crate) const fn plan(&self) -> &Plan {
        &self.plan
    }
}

impl fmt::Debug for CompilerBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompilerBinding")
            .field("scope", &self.scope)
            .field("source_id", &self.source_id())
            .field("dialect", &self.dialect)
            .field("triples_map_count", &self.mapping.len())
            .field("schema_table_count", &self.schema.len())
            .field("tbox_empty", &self.tbox.is_empty())
            .field("cache_entries", &self.cache.len())
            .finish()
    }
}

/// The structural cache key: `(compile-scope, algebra-hash)` plus the **canonical
/// algebra string** that disambiguates a 64-bit hash collision. `Eq` compares the
/// canonical string, so two distinct queries that happen to share a
/// `structural_hash` in the same scope can never collide onto one plan — closing
/// the hazard ADR-0007 *sharp keying* warns about (a plan for `:a` serving `:b`).
/// `Hash` uses only the fast `(scope, structural_hash)` pre-hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanKey {
    scope: CompileScope,
    structural_hash: u64,
    canonical: String,
}

impl std::hash::Hash for PlanKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.scope.hash(state);
        self.structural_hash.hash(state);
    }
}

/// Compute the structural key for `query` in `scope` (ADR-0007). Conservative:
/// the canonical algebra rendering retains the schema-selecting constants
/// (predicate IRIs, template constants) — and, for now, data constants too — and
/// is also stored verbatim so equality is exact, never hash-only.
pub fn plan_key(query: &Query, scope: CompileScope) -> PlanKey {
    use std::hash::{Hash, Hasher};
    let canonical = query.to_string();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    canonical.hash(&mut h);
    PlanKey {
        scope,
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

    fn scope(dialect: Dialect, epoch: Epoch) -> CompileScope {
        CompileScope::new(CompileBindingId::mint(), dialect, epoch)
    }

    #[test]
    fn same_query_same_key() {
        let scope = scope(Dialect::Sqlite, Epoch(3));
        let a = plan_key(&parse("SELECT * WHERE { ?s ?p ?o }"), scope);
        let b = plan_key(&parse("SELECT * WHERE { ?s ?p ?o }"), scope);
        assert_eq!(a, b);
    }

    #[test]
    fn schema_selecting_constant_changes_key() {
        // A different predicate IRI selects different mapping entries → must not
        // share a plan (ADR-0007 sharp keying).
        let scope = scope(Dialect::Sqlite, Epoch(0));
        let a = plan_key(&parse("SELECT ?x WHERE { ?x <http://ex/a> ?y }"), scope);
        let b = plan_key(&parse("SELECT ?x WHERE { ?x <http://ex/b> ?y }"), scope);
        assert_ne!(a, b);
    }

    #[test]
    fn hash_collision_does_not_serve_the_wrong_plan() {
        // Force a structural_hash collision between two *distinct* queries at the
        // same epoch: equality must still distinguish them (ADR-0007 sharp keying),
        // so the cache returns a miss for the second, never `:a`'s plan for `:b`.
        let cache: PlanCache<u32> = PlanCache::new(8);
        let scope = scope(Dialect::Sqlite, Epoch(0));
        let mut ka = plan_key(&parse("SELECT ?x WHERE { ?x <http://ex/a> ?y }"), scope);
        let mut kb = plan_key(&parse("SELECT ?x WHERE { ?x <http://ex/b> ?y }"), scope);
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
        let binding = CompileBindingId::mint();
        assert_ne!(
            plan_key(&q, CompileScope::new(binding, Dialect::Sqlite, Epoch(1))),
            plan_key(&q, CompileScope::new(binding, Dialect::Sqlite, Epoch(2)))
        );
    }

    #[test]
    #[should_panic(expected = "compile epoch exhausted")]
    fn epoch_exhaustion_fails_instead_of_wrapping() {
        let mut epoch = Epoch(u64::MAX);
        epoch.bump();
    }

    #[test]
    fn dialect_and_binding_identity_are_part_of_the_key() {
        let q = parse("SELECT * WHERE { ?s ?p ?o }");
        let binding = CompileBindingId::mint();
        let sqlite = CompileScope::new(binding, Dialect::Sqlite, Epoch(0));
        let postgres = CompileScope::new(binding, Dialect::Postgres, Epoch(0));
        let other_binding = scope(Dialect::Sqlite, Epoch(0));

        assert_ne!(plan_key(&q, sqlite), plan_key(&q, postgres));
        assert_ne!(plan_key(&q, sqlite), plan_key(&q, other_binding));
    }

    #[test]
    fn cache_round_trips_and_is_bounded() {
        let cache: PlanCache<u32> = PlanCache::new(2);
        let scope = scope(Dialect::Sqlite, Epoch(0));
        let k1 = plan_key(&parse("SELECT * WHERE { ?a ?b ?c }"), scope);
        cache.put(k1.clone(), 10);
        assert_eq!(cache.get(&k1), Some(10));
        // Overflow evicts approximately-LRU, never wholesale (quick_cache).
        cache.put(plan_key(&parse("SELECT * WHERE { ?d ?e ?f }"), scope), 20);
        cache.put(plan_key(&parse("SELECT * WHERE { ?g ?h ?i }"), scope), 30);
        assert!(cache.len() <= 2);
    }

    /// A synthetic key distinguished only by `canonical` (real `plan_key` overkill
    /// for a hit-rate workload of thousands of accesses).
    fn synth_key(scope: CompileScope, id: usize) -> PlanKey {
        PlanKey {
            scope,
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
        let scope = scope(Dialect::Sqlite, Epoch(0));
        let mut hits = 0u32;
        let mut accesses = 0u32;
        for i in 0..ITERS {
            // 2/3 of accesses hit a small, fixed hot set (round-robin); 1/3 hit a
            // much larger cold set that churns through far more distinct keys than
            // fit in `capacity`, forcing the cache to evict repeatedly.
            let key = if i % 3 != 0 {
                synth_key(scope, i % HOT)
            } else {
                synth_key(scope, HOT + (i / 3) % COLD)
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
