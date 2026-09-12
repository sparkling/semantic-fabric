use super::*;
use sf_core::query_control::{QueryBudget, QueryLimits};

pub(crate) fn assert_fixed_hash_bound<K: Hash>(key: &K) {
    #[derive(Default)]
    struct Count(u64);
    impl Hasher for Count {
        fn finish(&self) -> u64 {
            0
        }
        fn write(&mut self, bytes: &[u8]) {
            self.0 += bytes.len() as u64 + 1;
        }
    }
    let mut count = Count::default();
    key.hash(&mut count);
    assert!(count.0 <= FIXED_KEY_WORK, "fixed hash cost {}", count.0);
}

#[test]
fn fixed_hash_excludes_large_canonical_and_overflow_never_publishes() {
    let scope = super::super::test_scope(
        sf_core::SourceId::new(0).unwrap(),
        sf_sql::Dialect::Sqlite,
        super::super::Epoch(0),
    );
    let key = super::super::PlanKey::from_canonical(
        scope,
        super::super::CompileProfileId::Uncontrolled,
        "x".repeat(100_000),
    );
    assert_fixed_hash_bound(&key);
    let (cache, _) = new_cache(8);
    for geometry in [
        Geometry {
            residents: u64::MAX,
            probes: 1,
        },
        Geometry {
            residents: 1,
            probes: u64::MAX,
        },
    ] {
        let control = budget(u64::MAX);
        assert!(matches!(
            insert(&cache, geometry, key.clone(), (), &control),
            Err(crate::Error::QueryControl(
                QueryControlError::AccountingOverflow
            ))
        ));
        assert_eq!(cache.get(&key), None);
        assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
        assert_eq!(
            control.checkpoint(),
            Err(QueryControlError::AccountingOverflow)
        );
    }
    let control = budget(u64::MAX);
    assert!(lookup(
        &cache,
        Geometry {
            residents: 1,
            probes: u64::MAX
        },
        &key,
        &control
    )
    .is_err());
    assert_eq!(
        control.checkpoint(),
        Err(QueryControlError::AccountingOverflow)
    );
    let (disabled, geometry) = new_cache(0);
    let control = budget(u64::MAX);
    insert(&disabled, geometry, key.clone(), (), &control).unwrap();
    assert!(control.consumed(QueryCharge::CompilerWork) > 0);
    assert_eq!(
        lookup(&disabled, geometry, &key, &budget(u64::MAX)).unwrap(),
        None
    );
}

#[derive(Clone, Eq, PartialEq)]
struct Key(String);
impl Hash for Key {
    fn hash<H: Hasher>(&self, state: &mut H) {
        0u64.hash(state);
    }
}
impl OperationKey for Key {
    fn canonical(&self) -> &str {
        &self.0
    }
    fn same_identity(&self, _: &Self) -> bool {
        true
    }
}
fn budget(units: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX))
}

#[test]
fn collisions_are_exact_and_comparison_refusal_is_not_a_miss() {
    let (cache, geometry) = new_cache(8);
    let first = Key(format!("{}a", "x".repeat(513)));
    let second = Key(format!("{}b", "x".repeat(513)));
    cache.insert(first.clone(), 1u32);
    cache.insert(second.clone(), 2u32);
    let control = budget(u64::MAX);
    assert_eq!(
        lookup(&cache, geometry, &second, &control).unwrap(),
        Some(2)
    );
    let exact = control.consumed(QueryCharge::CompilerWork);
    assert_eq!(
        lookup(&cache, geometry, &second, &budget(exact)).unwrap(),
        Some(2)
    );
    assert!(matches!(
        lookup(&cache, geometry, &second, &budget(exact - 1)),
        Err(crate::Error::QueryControl(
            QueryControlError::CompilerWorkExceeded
        ))
    ));
    assert_eq!(cache.get(&first), Some(1));
    assert_eq!(cache.get(&second), Some(2));
}

#[test]
fn insertion_admission_is_exact_and_precedes_publication() {
    let (cache, geometry) = new_cache(8);
    let key = Key("new".into());
    let control = budget(u64::MAX);
    insert(&cache, geometry, key.clone(), 7u32, &control).unwrap();
    let exact = control.consumed(QueryCharge::CompilerWork);
    for limit in [exact - 1, exact] {
        let (cache, geometry) = new_cache(8);
        let result = insert(&cache, geometry, key.clone(), 7u32, &budget(limit));
        assert_eq!(result.is_ok(), limit == exact);
        assert_eq!(cache.get(&key), (limit == exact).then_some(7));
    }
}

struct Stop {
    budget: QueryBudget,
    after: u64,
    calls: std::sync::atomic::AtomicU64,
    cause: QueryControlError,
}
impl QueryControl for Stop {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        self.budget.checkpoint()
    }
    fn consume(&self, kind: QueryCharge, units: u64) -> Result<(), QueryControlError> {
        self.budget.consume(kind, units)?;
        if self
            .calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1
            == self.after
        {
            self.budget.terminate(self.cause);
        }
        Ok(())
    }
    fn terminate(&self, cause: QueryControlError) -> QueryControlError {
        self.budget.terminate(cause)
    }
}

#[test]
fn sticky_stops_at_every_comparison_cut_and_before_insert() {
    let (cache, geometry) = new_cache(8);
    let key = Key("x".repeat(700));
    cache.insert(key.clone(), 4u32);
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        // Envelope, fixed identity, then three canonical chunks.
        for after in 1..=5 {
            let control = Stop {
                budget: budget(u64::MAX),
                after,
                calls: Default::default(),
                cause,
            };
            assert!(matches!(lookup(&cache, geometry, &key, &control),
                Err(crate::Error::QueryControl(actual)) if actual == cause));
        }
        let control = Stop {
            budget: budget(u64::MAX),
            after: 1,
            calls: Default::default(),
            cause,
        };
        assert!(
            matches!(insert(&cache, geometry, key.clone(), 9u32, &control),
            Err(crate::Error::QueryControl(actual)) if actual == cause)
        );
        assert_eq!(cache.get(&key), Some(4));
    }
}

#[test]
fn reserved_geometry_survives_collision_and_diverse_key_churn() {
    use super::super::{CompileProfileId, Epoch, PlanKey};
    let scope = super::super::test_scope(
        sf_core::SourceId::new(0).unwrap(),
        sf_sql::Dialect::Sqlite,
        Epoch(0),
    );
    for capacity in [1, 8, 33, 65] {
        let (cache, geometry) = new_cache(capacity);
        let before = cache.memory_used();
        for index in 0..2000 {
            let mut key = PlanKey::from_canonical(
                scope,
                CompileProfileId::Uncontrolled,
                format!("key-{index}"),
            );
            if index % 2 == 0 {
                key.structural_hash = 0;
            }
            insert(&cache, geometry, key.clone(), index, &budget(u64::MAX)).unwrap();
            for _ in 0..3 {
                assert_eq!(cache.get(&key), Some(index));
            }
        }
        let after = cache.memory_used();
        // LinkedSlab reports initialized length, not its reserved allocation.
        // Only the map metric exposes allocation capacity for this assertion.
        assert_eq!(after.map, before.map);
        assert!(after.entries > 0);
    }
}
