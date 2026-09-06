//! Plan cache (ADR-0007 *Performance*) — owned by one immutable
//! [`CompilerBinding`] and keyed on both its immutable compile scope and a
//! **structural hash of the SPARQL algebra**.
//!
//! The scope binds one source ID, mapping, T-Box, compiler-safe schema, dialect,
//! and cache. The scope carries deterministic content digests and an explicit
//! epoch. No live reload path exists; later DDL does not advance a binding.
//!
//! **Sharp keying rule (ADR-0007):** parameterise *data* constants but key on
//! *schema-selecting* constants (predicate IRIs and IRI-template constants — the
//! ones that decide which mapping entries/columns to unfold), so a plan compiled
//! for `:a` never serves a `:b` query.
//!
//! v1 keys use the full canonical algebra string, so every constant is keyed.
//! This safely causes only extra misses; data-constant sharing remains deferred.

use std::sync::Arc;

use sf_core::{SourceId, SourceMapping};
use sf_sql::{Dialect, TableSchema};
use spargebra::Query;

use crate::compiler_schema::{
    ColumnTypeAuthority, ColumnTypeUse, CompilerSchema, ConstraintAuthority,
};
use crate::runtime_identity::CompileDigests;
use crate::{federation::SourceAffineUnionArm, Plan, Result, Tbox};

#[path = "cache_profile.rs"]
mod profile;
use profile::ProfiledPlanCaches;

#[allow(dead_code)] // Private ADR-0018 seam; request enforcement is a later slice.
#[path = "cache_security.rs"]
mod security;

/// Closed compiler-governance profile used to partition cache authority.
///
/// The governed variant is intentionally dormant until every owned compiler
/// phase is metered. Merely constructing its key grants no governed capability.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum CompileProfileId {
    Uncontrolled,
    #[allow(dead_code)] // Activated only after every owned compiler phase is metered.
    GovernedV1,
}

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

/// Exact cache namespace for one compiler binding and generation.
///
/// Construction is private: the source, mapping, ontology, schema, authority,
/// and capability identity is derived from the binding inputs, never supplied
/// independently by a caller. Obtain it from [`CompilerBinding::scope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CompileScope {
    source_id: SourceId,
    dialect: Dialect,
    epoch: Epoch,
    constraint_authority: ConstraintAuthority,
    column_type_authority: ColumnTypeAuthority,
    digests: CompileDigests,
}

impl CompileScope {
    fn new(
        source_id: SourceId,
        dialect: Dialect,
        epoch: Epoch,
        constraint_authority: ConstraintAuthority,
        column_type_authority: ColumnTypeAuthority,
        digests: CompileDigests,
    ) -> Self {
        Self {
            source_id,
            dialect,
            epoch,
            constraint_authority,
            column_type_authority,
            digests,
        }
    }

    pub const fn source_id(self) -> SourceId {
        self.source_id
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

    pub const fn digests(self) -> CompileDigests {
        self.digests
    }
}

#[cfg(test)]
pub(crate) fn test_scope(source_id: SourceId, dialect: Dialect, epoch: Epoch) -> CompileScope {
    let mapping = SourceMapping::new(source_id, Vec::new());
    let tbox = Tbox::default();
    let schema = CompilerSchema::from_unverified_observation(Vec::new());
    let digests = CompileDigests::from_inputs(
        &mapping,
        &tbox,
        schema.tables(),
        dialect,
        schema.constraint_authority(),
        schema.column_type_authority(),
    );
    CompileScope::new(
        source_id,
        dialect,
        epoch,
        schema.constraint_authority(),
        schema.column_type_authority(),
        digests,
    )
}

/// Immutable semantic inputs and cache for one source-local compiler.
///
/// Grouping these values makes dialect/context mismatch unrepresentable on the
/// cached translation path. Creating a replacement mapping, ontology, schema,
/// or backend requires a new binding and therefore a fresh cache namespace.
/// The supplied capacity remains raw-only while governance is dormant.
pub struct CompilerBinding {
    mapping: SourceMapping,
    dialect: Dialect,
    tbox: Tbox,
    schema: CompilerSchema,
    caches: ProfiledPlanCaches<CachedPlan>,
    scope: CompileScope,
}

#[path = "cache_binding.rs"]
mod binding_extensions;

impl CompilerBinding {
    pub fn new(
        mapping: SourceMapping,
        dialect: Dialect,
        tbox: Tbox,
        schema: CompilerSchema,
        cache_capacity: usize,
    ) -> Self {
        let digests = CompileDigests::from_inputs(
            &mapping,
            &tbox,
            schema.tables(),
            dialect,
            schema.constraint_authority(),
            schema.column_type_authority(),
        );
        Self::from_parts(
            mapping,
            dialect,
            tbox,
            schema,
            Epoch::default(),
            digests,
            cache_capacity,
        )
    }

    /// Build a serving binding from one raw catalogue observation. Identity is
    /// computed before unverified constraints and types are quarantined, so a
    /// replacement observation cannot reuse an old cache scope accidentally.
    pub fn from_unverified_observation(
        mapping: SourceMapping,
        dialect: Dialect,
        tbox: Tbox,
        observed_schema: Vec<TableSchema>,
        epoch: Epoch,
        cache_capacity: usize,
    ) -> Self {
        let constraint_authority = ConstraintAuthority::Unverified;
        let column_type_authority = ColumnTypeAuthority::Unverified;
        let digests = CompileDigests::from_inputs(
            &mapping,
            &tbox,
            &observed_schema,
            dialect,
            constraint_authority,
            column_type_authority,
        );
        let schema = CompilerSchema::from_unverified_observation(observed_schema);
        Self::from_parts(
            mapping,
            dialect,
            tbox,
            schema,
            epoch,
            digests,
            cache_capacity,
        )
    }

    fn from_parts(
        mapping: SourceMapping,
        dialect: Dialect,
        tbox: Tbox,
        schema: CompilerSchema,
        epoch: Epoch,
        digests: CompileDigests,
        cache_capacity: usize,
    ) -> Self {
        let scope = CompileScope::new(
            mapping.source_id(),
            dialect,
            epoch,
            schema.constraint_authority(),
            schema.column_type_authority(),
            digests,
        );
        Self {
            mapping,
            dialect,
            tbox,
            schema,
            caches: ProfiledPlanCaches::uncontrolled_only(cache_capacity),
            scope,
        }
    }

    /// Parse and compile against this binding's inseparable semantic context.
    pub fn compile(&self, sparql: &str) -> Result<Plan> {
        crate::parse_and_translate_cached(sparql, self)
    }

    /// Parse and compile while sharing the cache-owned plan allocation.
    ///
    /// This avoids recursive plan clones on serving cache hits and insertion.
    /// It does not grant governed compilation authority.
    pub fn compile_shared(&self, sparql: &str) -> Result<Arc<Plan>> {
        crate::parse_and_translate_cached_shared(sparql, self)
    }

    /// Parse and compile without reading or populating this binding's cache.
    ///
    /// This narrow seam exists for a serving preflight that must reject invalid
    /// or unsupported work before source I/O, while ensuring that no plan made
    /// before a verified source-generation lease can become authoritative. The
    /// caller must discard the returned plan after structural admission and
    /// compile again through [`Self::compile_shared`] while holding its lease.
    pub fn compile_uncached_shared(&self, sparql: &str) -> Result<Arc<Plan>> {
        let query = crate::parse_query(sparql)?;
        self.compile_parsed_uncached_shared(&query)
    }

    /// Compile one arm from a query that was parsed and structurally admitted
    /// once by the narrow federation boundary.
    pub(crate) fn compile_union_arm_shared(&self, arm: &SourceAffineUnionArm) -> Result<Arc<Plan>> {
        crate::translate_cached_shared(arm.query(), self)
    }

    /// Uncached counterpart used only by the non-authorizing federation
    /// preflight. No result from this method may enter execution or a cache.
    pub(crate) fn compile_union_arm_uncached_shared(
        &self,
        arm: &SourceAffineUnionArm,
    ) -> Result<Arc<Plan>> {
        self.compile_parsed_uncached_shared(arm.query())
    }

    fn compile_parsed_uncached_shared(&self, query: &Query) -> Result<Arc<Plan>> {
        crate::translate_tree_with_column_type_use(
            query,
            self.triples_maps(),
            self.tbox(),
            self.dialect(),
            self.schema(),
            self.column_type_use(),
            crate::CompilerWorkMode::Uncontrolled,
        )
        .map(Arc::new)
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

    pub const fn digests(&self) -> CompileDigests {
        self.scope.digests()
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
        self.cache_for(CompileProfileId::Uncontrolled)
    }

    pub(crate) fn cache_for(&self, profile: CompileProfileId) -> &PlanCache<CachedPlan> {
        self.caches.for_profile(profile)
    }

    #[cfg(test)]
    pub(crate) fn cache_len(&self) -> usize {
        self.cache().len()
    }
}

/// Cached artifact carrying its own scope as a second fail-closed check against
/// a wrongly inserted value. Kept crate-private so no unverified plan escapes.
#[derive(Clone)]
pub(crate) struct CachedPlan {
    scope: CompileScope,
    profile: CompileProfileId,
    plan: Arc<Plan>,
}

impl CachedPlan {
    #[cfg(test)]
    pub(crate) fn new(scope: CompileScope, profile: CompileProfileId, plan: Plan) -> Self {
        Self::from_shared(scope, profile, Arc::new(plan))
    }

    pub(crate) fn from_shared(
        scope: CompileScope,
        profile: CompileProfileId,
        plan: Arc<Plan>,
    ) -> Self {
        Self {
            scope,
            profile,
            plan,
        }
    }

    pub(crate) const fn scope(&self) -> CompileScope {
        self.scope
    }

    pub(crate) const fn profile(&self) -> CompileProfileId {
        self.profile
    }

    pub(crate) fn shared_plan(&self) -> Arc<Plan> {
        Arc::clone(&self.plan)
    }
}

/// The structural cache key: `(compile-scope, compile-profile, algebra-hash)`
/// plus the **canonical algebra string** that disambiguates a 64-bit hash
/// collision. `Eq` compares the canonical string, so two distinct queries that
/// happen to share a `structural_hash` in the same scope/profile can never
/// collide onto one plan — closing the hazard ADR-0007 *sharp keying* warns
/// about (a plan for `:a` serving `:b`). `Hash` uses only the fast
/// `(scope, profile, structural_hash)` pre-hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanKey {
    scope: CompileScope,
    profile: CompileProfileId,
    structural_hash: u64,
    canonical: String,
}

impl PlanKey {
    fn from_canonical(scope: CompileScope, profile: CompileProfileId, canonical: String) -> Self {
        use std::hash::{Hash, Hasher};

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        canonical.hash(&mut hasher);
        Self {
            scope,
            profile,
            structural_hash: hasher.finish(),
            canonical,
        }
    }
}

impl std::hash::Hash for PlanKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.scope.hash(state);
        self.profile.hash(state);
        self.structural_hash.hash(state);
    }
}

/// Compute the structural key for `query` in `scope` (ADR-0007). Conservative:
/// the canonical algebra rendering retains the schema-selecting constants
/// (predicate IRIs, template constants) — and, for now, data constants too — and
/// is also stored verbatim so equality is exact, never hash-only.
pub fn plan_key(query: &Query, scope: CompileScope) -> PlanKey {
    plan_key_for_profile(query, scope, CompileProfileId::Uncontrolled)
}

pub(crate) fn plan_key_for_profile(
    query: &Query,
    scope: CompileScope,
    profile: CompileProfileId,
) -> PlanKey {
    let canonical = query.to_string();
    PlanKey::from_canonical(scope, profile, canonical)
}

#[allow(dead_code)] // Dormant until the controlled compiler path is activated.
#[path = "cache_key.rs"]
pub(crate) mod bounded_key;

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
