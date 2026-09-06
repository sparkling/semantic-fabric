//! Private, non-authorizing security partition for compiled plans (ADR-0018).
//!
//! This is deliberately additive: the existing product cache remains
//! unchanged, while this seam requires an explicit request context, expected
//! policy snapshot, and distinct cache. Nothing in this module authenticates,
//! authorizes, or activates a serving endpoint.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::Arc;

use sf_core::security_context::{PolicySnapshotId, SecurityCacheIdentity, SecurityContext};
use spargebra::Query;

use super::{CompileProfileId, CompileScope, CompilerBinding};
use crate::Plan;

/// A security-scoped compilation failure carrying no identity material.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SecurityCompileError {
    #[error("request security context does not match the expected policy snapshot")]
    PolicyMismatch,
    #[error("security cache entry does not match the compiler scope")]
    CacheScopeMismatch,
    #[error("security cache entry does not match the compiler profile")]
    CacheProfileMismatch,
    #[error("security cache entry does not match the request security partition")]
    CacheIdentityMismatch,
    #[error(transparent)]
    Compiler(#[from] crate::Error),
}

/// A separate cache key that cannot be constructed without request security
/// identity. Equality retains the complete canonical query and opaque identity,
/// so hash collisions cannot merge query or security partitions.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct SecurityPlanKey {
    scope: CompileScope,
    profile: CompileProfileId,
    security_identity: SecurityCacheIdentity,
    structural_hash: u64,
    canonical: String,
}

impl SecurityPlanKey {
    fn from_query(
        query: &Query,
        scope: CompileScope,
        profile: CompileProfileId,
        security_identity: SecurityCacheIdentity,
    ) -> Self {
        let canonical = query.to_string();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        canonical.hash(&mut hasher);
        Self::from_canonical_with_hash(
            scope,
            profile,
            security_identity,
            hasher.finish(),
            canonical,
        )
    }

    fn from_canonical_with_hash(
        scope: CompileScope,
        profile: CompileProfileId,
        security_identity: SecurityCacheIdentity,
        structural_hash: u64,
        canonical: String,
    ) -> Self {
        Self {
            scope,
            profile,
            security_identity,
            structural_hash,
            canonical,
        }
    }
}

impl Hash for SecurityPlanKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.scope.hash(state);
        self.profile.hash(state);
        self.security_identity.hash(state);
        self.structural_hash.hash(state);
    }
}

impl fmt::Debug for SecurityPlanKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "SecurityPlanKey { scope: [bound], profile: [bound], security_identity: \
             [redacted], query: [redacted] }",
        )
    }
}

/// Cached plan carrying an independent copy of every binding dimension checked
/// on lookup. This turns an internal bad insertion into a closed failure.
#[derive(Clone)]
pub(crate) struct SecurityCachedPlan {
    scope: CompileScope,
    profile: CompileProfileId,
    security_identity: SecurityCacheIdentity,
    plan: Arc<Plan>,
}

impl SecurityCachedPlan {
    fn from_shared(
        scope: CompileScope,
        profile: CompileProfileId,
        security_identity: SecurityCacheIdentity,
        plan: Arc<Plan>,
    ) -> Self {
        Self {
            scope,
            profile,
            security_identity,
            plan,
        }
    }

    fn shared_plan(&self) -> Arc<Plan> {
        Arc::clone(&self.plan)
    }
}

impl fmt::Debug for SecurityCachedPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "SecurityCachedPlan { scope: [bound], profile: [bound], \
             security_identity: [redacted], plan: [redacted] }",
        )
    }
}

/// Explicit, bounded cache for security-scoped compilation.
///
/// It has no `Default`; zero capacity is unrepresentable, and it never aliases
/// the existing product cache.
pub(crate) struct SecurityPlanCache {
    inner: quick_cache::sync::Cache<SecurityPlanKey, SecurityCachedPlan>,
    #[cfg(test)]
    reads: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    writes: std::sync::atomic::AtomicUsize,
}

impl SecurityPlanCache {
    pub(crate) fn new(capacity: NonZeroUsize) -> Self {
        Self {
            inner: quick_cache::sync::Cache::new(capacity.get()),
            #[cfg(test)]
            reads: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            writes: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn get(&self, key: &SecurityPlanKey) -> Option<SecurityCachedPlan> {
        #[cfg(test)]
        self.reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.inner.get(key)
    }

    fn put(&self, key: SecurityPlanKey, plan: SecurityCachedPlan) {
        #[cfg(test)]
        self.writes
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.inner.insert(key, plan);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.inner.len()
    }

    #[cfg(test)]
    fn reset_access_counts(&self) {
        self.reads.store(0, std::sync::atomic::Ordering::Relaxed);
        self.writes.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    #[cfg(test)]
    fn access_counts(&self) -> (usize, usize) {
        (
            self.reads.load(std::sync::atomic::Ordering::Relaxed),
            self.writes.load(std::sync::atomic::Ordering::Relaxed),
        )
    }
}

impl fmt::Debug for SecurityPlanCache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecurityPlanCache { security_identities: [redacted] }")
    }
}

/// Compiler view pinned to one snapshot-owned policy and one explicit cache.
pub(crate) struct SecurityScopedCompiler<'a> {
    binding: &'a CompilerBinding,
    expected_policy: PolicySnapshotId,
    cache: &'a SecurityPlanCache,
}

impl CompilerBinding {
    pub(crate) fn for_security_policy<'a>(
        &'a self,
        expected_policy: PolicySnapshotId,
        cache: &'a SecurityPlanCache,
    ) -> SecurityScopedCompiler<'a> {
        SecurityScopedCompiler {
            binding: self,
            expected_policy,
            cache,
        }
    }
}

impl SecurityScopedCompiler<'_> {
    pub(crate) fn compile_shared(
        &self,
        context: &SecurityContext,
        sparql: &str,
    ) -> Result<Arc<Plan>, SecurityCompileError> {
        if context.policy_snapshot() != self.expected_policy {
            return Err(SecurityCompileError::PolicyMismatch);
        }

        let query = crate::parse_query(sparql)?;
        let profile = CompileProfileId::Uncontrolled;
        let security_identity = context.cache_identity();
        let key =
            SecurityPlanKey::from_query(&query, self.binding.scope(), profile, security_identity);

        if let Some(cached) = self.cache.get(&key) {
            if cached.scope != self.binding.scope() {
                return Err(SecurityCompileError::CacheScopeMismatch);
            }
            if cached.profile != profile {
                return Err(SecurityCompileError::CacheProfileMismatch);
            }
            if cached.security_identity != security_identity {
                return Err(SecurityCompileError::CacheIdentityMismatch);
            }
            return Ok(cached.shared_plan());
        }

        let plan = Arc::new(crate::translate_tree_with_column_type_use(
            &query,
            self.binding.triples_maps(),
            self.binding.tbox(),
            self.binding.dialect(),
            self.binding.schema(),
            self.binding.column_type_use(),
            crate::CompilerWorkMode::Uncontrolled,
        )?);
        self.cache.put(
            key,
            SecurityCachedPlan::from_shared(
                self.binding.scope(),
                profile,
                security_identity,
                Arc::clone(&plan),
            ),
        );
        Ok(plan)
    }

    #[cfg(test)]
    fn expected_policy(&self) -> PolicySnapshotId {
        self.expected_policy
    }
}

#[cfg(test)]
#[path = "cache_security_tests.rs"]
mod tests;
