//! Plan cache (ADR-0007 *Performance*) — owned by one immutable
//! [`CompilerBinding`] and keyed on both its opaque compile scope and a
//! **structural hash of the SPARQL algebra**.
//!
//! The scope binds one source ID, mapping, T-Box, compiler-safe schema, dialect,
//! and cache. A new binding gets a process-unique identity and explicit epoch.
//! No live reload path exists; later DDL does not advance an existing binding.
//!
//! **Sharp keying rule (ADR-0007):** parameterise *data* constants but key on
//! *schema-selecting* constants (predicate IRIs and IRI-template constants — the
//! ones that decide which mapping entries/columns to unfold), so a plan compiled
//! for `:a` never serves a `:b` query.
//!
//! v1 keys use the full canonical algebra string, so every constant is keyed.
//! This safely causes only extra misses; data-constant sharing remains deferred.

use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

use sf_core::{SourceId, SourceMapping};
use sf_sql::{Dialect, TableSchema};
use spargebra::Query;

use crate::compiler_schema::{
    ColumnTypeAuthority, ColumnTypeUse, CompilerSchema, ConstraintAuthority,
};
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
    constraint_authority: ConstraintAuthority,
    column_type_authority: ColumnTypeAuthority,
}

impl CompileScope {
    fn new(
        binding: CompileBindingId,
        dialect: Dialect,
        epoch: Epoch,
        constraint_authority: ConstraintAuthority,
        column_type_authority: ColumnTypeAuthority,
    ) -> Self {
        Self {
            binding,
            dialect,
            epoch,
            constraint_authority,
            column_type_authority,
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

    pub const fn constraint_authority(self) -> ConstraintAuthority {
        self.constraint_authority
    }

    pub const fn column_type_authority(self) -> ColumnTypeAuthority {
        self.column_type_authority
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
    schema: CompilerSchema,
    cache: PlanCache<CachedPlan>,
    scope: CompileScope,
}

impl CompilerBinding {
    pub fn new(
        mapping: SourceMapping,
        dialect: Dialect,
        tbox: Tbox,
        schema: CompilerSchema,
        cache_capacity: usize,
    ) -> Self {
        assert!(cache_capacity > 0, "plan cache capacity must be non-zero");
        let scope = CompileScope::new(
            CompileBindingId::mint(),
            dialect,
            Epoch::default(),
            schema.constraint_authority(),
            schema.column_type_authority(),
        );
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

    pub const fn constraint_authority(&self) -> ConstraintAuthority {
        self.schema.constraint_authority()
    }

    pub const fn column_type_authority(&self) -> ColumnTypeAuthority {
        self.schema.column_type_authority()
    }

    pub(crate) fn column_type_use(&self) -> ColumnTypeUse {
        self.column_type_authority().into()
    }

    pub(crate) fn triples_maps(&self) -> &[sf_core::ir::TriplesMap] {
        self.mapping.triples_maps()
    }

    pub(crate) fn tbox(&self) -> &Tbox {
        &self.tbox
    }

    pub(crate) fn schema(&self) -> &[TableSchema] {
        self.schema.tables()
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
            .field("schema", &self.schema)
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
#[path = "cache_tests.rs"]
mod tests;
