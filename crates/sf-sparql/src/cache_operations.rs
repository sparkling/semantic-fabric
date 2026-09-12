//! Prospective logical admission around the fixed quick_cache implementation.
//! Geometry is captured before publication, never through request-time lock-taking
//! statistics. Physical allocator and last-Arc destruction are separate concerns.
use std::cell::Cell;
use std::hash::{Hash, Hasher};

use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};

// Eight 32-byte digests plus Hash tags, one security digest, fixed scope
// fields and hash-call/identity overhead fit within 512 logical units.
// Canonical bytes are excluded from Hash and charged separately by Eq.
const FIXED_KEY_WORK: u64 = 512;
const _: () = assert!(usize::BITS <= 64);

#[derive(Clone, Copy, Debug)]
pub(super) struct Geometry {
    residents: u64,
    probes: u64,
}

pub(super) fn new_cache<K: Eq + Hash, V: Clone>(
    capacity: usize,
) -> (quick_cache::sync::Cache<K, V>, Geometry) {
    let inner = quick_cache::sync::Cache::new(capacity);
    // Reserve enough to keep even a churn-triggered rehash below the existing
    // table's half-full threshold. Unit weights, no placeholders and no resize.
    let residents = inner.shard_capacity();
    let reserve = usize::try_from(residents)
        .expect("cache capacity overflow")
        .checked_mul(inner.num_shards())
        .and_then(|n| n.checked_mul(4))
        .expect("cache reservation overflow");
    inner.reserve(reserve);
    // All fresh shards have identical reservation geometry. Token is NonZeroU32
    // in locked quick_cache 0.7.0; allocation bytes include every bucket token.
    let per_shard = inner.memory_used().map / inner.num_shards();
    let probes = (per_shard / std::mem::size_of::<u32>()) as u64 + 1;
    (inner, Geometry { residents, probes })
}

fn charge(control: &dyn QueryControl, units: u64) -> Result<(), QueryControlError> {
    control.checkpoint()?;
    control.consume(QueryCharge::CompilerWork, units)?;
    control.checkpoint()
}

pub(super) trait OperationKey: Hash {
    fn canonical(&self) -> &str;
    fn same_identity(&self, other: &Self) -> bool;
}

/// A fallible comparison must never silently become a successful cache miss.
struct Probe<'a, K> {
    key: &'a K,
    control: &'a dyn QueryControl,
    error: Cell<Option<QueryControlError>>,
}

impl<K: Hash> Hash for Probe<'_, K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

impl<K: OperationKey> quick_cache::Equivalent<K> for Probe<'_, K> {
    fn equivalent(&self, other: &K) -> bool {
        if self.error.get().is_some() {
            return false;
        }
        let compare = || {
            charge(self.control, FIXED_KEY_WORK)?;
            if !self.key.same_identity(other)
                || self.key.canonical().len() != other.canonical().len()
            {
                return Ok(false);
            }
            for (a, b) in self
                .key
                .canonical()
                .as_bytes()
                .chunks(256)
                .zip(other.canonical().as_bytes().chunks(256))
            {
                charge(self.control, a.len() as u64)?;
                if a != b {
                    return Ok(false);
                }
            }
            Ok(true)
        };
        match compare() {
            Ok(equal) => equal,
            Err(error) => {
                self.error.set(Some(error));
                false
            }
        }
    }
}

pub(super) fn lookup<K: Eq + OperationKey, V: Clone>(
    cache: &quick_cache::sync::Cache<K, V>,
    geometry: Geometry,
    key: &K,
    control: &dyn QueryControl,
) -> crate::Result<Option<V>> {
    let units = geometry
        .probes
        .checked_add(FIXED_KEY_WORK)
        .ok_or_else(|| control.terminate(QueryControlError::AccountingOverflow))?;
    charge(control, units)?;
    let probe = Probe {
        key,
        control,
        error: Cell::new(None),
    };
    let value = cache.try_get(&probe).ok().flatten();
    if let Some(error) = probe.error.get() {
        return Err(error.into());
    }
    control.checkpoint()?;
    Ok(value)
}

pub(super) fn insert<K: Eq + OperationKey, V: Clone>(
    cache: &quick_cache::sync::Cache<K, V>,
    geometry: Geometry,
    key: K,
    value: V,
    control: &dyn QueryControl,
) -> crate::Result<()> {
    let r = geometry.residents;
    let h = geometry.probes;
    // Locked quick_cache 0.7 / hashbrown 0.17: ghosts <= R, MAX_F=2,
    // unit weights, no pinned entries/placeholders/resize. At most one resident
    // retires. Tombstone churn CAN rehash in place: <=L placements, each <=H
    // probes plus a fixed-key hash, and an H-sized outer scan. It cannot grow
    // after the constructor reserved at least twice the maximum live slots.
    let units = (|| {
        let l = r.checked_mul(2)?;
        let compare = r.checked_mul(FIXED_KEY_WORK.checked_add(key.canonical().len() as u64)?)?;
        let probes = l.checked_add(4)?.checked_mul(h)?;
        // Only residents hash keys; ghosts return their stored u64 hash.
        let rehash = r
            .checked_mul(FIXED_KEY_WORK)?
            .checked_add(l.checked_mul(2)?)?;
        let clock = r.checked_mul(3)?.checked_add(4)?.checked_mul(32)?;
        FIXED_KEY_WORK
            .checked_add(compare)?
            .checked_add(probes)?
            .checked_add(rehash)?
            .checked_add(clock)
    })()
    .ok_or_else(|| control.terminate(QueryControlError::AccountingOverflow))?;
    charge(control, units)?;
    if r == 0 {
        return Ok(());
    }
    // Keep retired values outside the shard lock but inside request ownership.
    // This bounds logical moves, not physical last-Arc Plan destruction.
    let mut retired = Default::default();
    drop(cache.try_insert_with_lifecycle(key, value, &mut retired));
    control.checkpoint()?;
    drop(retired);
    control.checkpoint()?;
    Ok(())
}

impl OperationKey for super::PlanKey {
    fn canonical(&self) -> &str {
        &self.canonical
    }
    fn same_identity(&self, other: &Self) -> bool {
        self.scope == other.scope
            && self.profile == other.profile
            && self.structural_hash == other.structural_hash
    }
}

impl<P: Clone> super::PlanCache<P> {
    pub(super) fn get_if_uncontended(
        &self,
        key: &super::PlanKey,
        control: &dyn QueryControl,
    ) -> crate::Result<Option<P>> {
        lookup(&self.inner, self.geometry, key, control)
    }
    pub(super) fn put_if_uncontended(
        &self,
        key: super::PlanKey,
        plan: P,
        control: &dyn QueryControl,
    ) -> crate::Result<()> {
        insert(&self.inner, self.geometry, key, plan, control)
    }
}

#[cfg(test)]
#[path = "cache_operations_tests.rs"]
mod tests;
#[cfg(test)]
pub(super) use tests::assert_fixed_hash_bound;

/// Independent cache-only prerequisites; never measures compiler phases.
#[cfg(test)]
pub(crate) fn test_work(capacity: usize, query: &str) -> (u64, u64, u64) {
    use sf_core::query_control::{QueryBudget, QueryLimits};
    let budget = || QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    let scope = super::test_scope(
        sf_core::SourceId::new(0).unwrap(),
        sf_sql::Dialect::Sqlite,
        super::Epoch(0),
    );
    let key = super::plan_key(&crate::parse_query(query).unwrap(), scope);
    let (cache, geometry) = new_cache(capacity);
    let miss = budget();
    assert_eq!(lookup(&cache, geometry, &key, &miss).unwrap(), None);
    let publication = budget();
    insert(&cache, geometry, key.clone(), (), &publication).unwrap();
    let hit = budget();
    assert_eq!(lookup(&cache, geometry, &key, &hit).unwrap(), Some(()));
    (
        miss.consumed(QueryCharge::CompilerWork),
        hit.consumed(QueryCharge::CompilerWork),
        publication.consumed(QueryCharge::CompilerWork),
    )
}
