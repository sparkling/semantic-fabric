use super::*;
use crate::iq::node::{IqCond, IqNode};
use crate::plan_measure::clone_root::{measure_copy_root, CompilerCloneRootV1};
use sf_core::{
    query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits},
    SourceId,
};

pub(crate) const QUERY: &str =
    "SELECT ?x WHERE { VALUES ?x { 1 2 3 } FILTER EXISTS { VALUES ?inside { 7 } } }";

fn binding() -> CompilerBinding {
    CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), vec![]),
        Dialect::Sqlite,
        Tbox::default(),
        vec![],
        Epoch::default(),
        8,
    )
}

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

fn key_work() -> u64 {
    let query = crate::parse_query(QUERY).unwrap();
    let paid = budget(u64::MAX);
    super::super::bounded_key::plan_key_with_work_control(
        &query,
        binding().scope(),
        CompileProfileId::Uncontrolled,
        &paid,
    )
    .unwrap();
    paid.consumed(QueryCharge::CompilerWork)
}

fn clone_work() -> u64 {
    clone_cost().total_work
}

fn clone_cost() -> crate::plan_measure::test_support::CopyWork {
    let Query::Select { pattern, .. } = crate::parse_query(QUERY).unwrap() else {
        panic!()
    };
    let IqNode::Construction { child, .. } = crate::build::build_tree(&pattern, None).unwrap()
    else {
        panic!()
    };
    let IqNode::Filter { cond, .. } = *child else {
        panic!()
    };
    let [IqCond::Exists(inner)] = cond.as_slice() else {
        panic!()
    };
    measure_copy_root(CompilerCloneRootV1::IqNode(inner)).unwrap()
}

fn build_work() -> u64 {
    let Query::Select { pattern, .. } = crate::parse_query(QUERY).unwrap() else {
        panic!()
    };
    let control = budget(u64::MAX);
    crate::build::build_tree_with_work_control(&pattern, None, &control).unwrap();
    control.consumed(QueryCharge::CompilerWork)
}

#[test]
fn canonical_key_cold_and_warm_paths_require_work_before_cache_access() {
    let query = "SELECT ?x WHERE { VALUES ?x { \"café 東京\" } }";
    for warm in [false, true] {
        let binding = binding();
        if warm {
            binding.compile_shared(query).unwrap();
        }
        assert!(matches!(
            binding.compile_shared_with_work_control(query, &budget(0)),
            Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
        ));
        assert_eq!(binding.cache_len(), usize::from(warm));
    }
}

#[test]
fn raw_populated_entry_is_shared_after_exact_controlled_key_work() {
    let binding = binding();
    let raw = binding.compile_shared(QUERY).unwrap();
    let work = key_work();
    let control = budget(work);
    let controlled = binding
        .compile_shared_with_work_control(QUERY, &control)
        .unwrap();
    assert!(Arc::ptr_eq(&raw, &controlled));
    assert_eq!(control.consumed(QueryCharge::CompilerWork), work);
    assert_eq!(binding.cache_len(), 1);
}

#[test]
fn exact_clone_charge_rejects_failed_misses_and_shares_completed_hits() {
    let binding = binding();
    let work = clone_work();
    let key = key_work();
    let build = build_work();
    let short = budget(key + build + 2 * work - 1);
    assert!(matches!(
        binding.compile_shared_with_work_control(QUERY, &short),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    assert_eq!(
        short.consumed(QueryCharge::CompilerWork),
        key + build + work + clone_cost().measurement_work
    );
    assert_eq!(binding.cache_len(), 0);
    let exact = budget(key + build + 2 * work);
    let plan = binding
        .compile_shared_with_work_control(QUERY, &exact)
        .unwrap();
    assert_eq!(
        exact.consumed(QueryCharge::CompilerWork),
        key + build + 2 * work
    );
    assert_eq!(binding.cache_len(), 1);
    assert_eq!(
        format!("{plan:?}"),
        format!("{:?}", binding.compile_uncached_shared(QUERY).unwrap())
    );
    assert!(matches!(
        binding.compile_shared_with_work_control(QUERY, &budget(key - 1)),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    let hit_control = budget(key);
    let hit = binding
        .compile_shared_with_work_control(QUERY, &hit_control)
        .unwrap();
    assert!(Arc::ptr_eq(&plan, &hit));
    assert!(Arc::ptr_eq(&plan, &binding.compile_shared(QUERY).unwrap()));
    assert_eq!(hit_control.consumed(QueryCharge::CompilerWork), key);
    hit_control.terminate(QueryControlError::Cancelled);
    assert!(matches!(
        binding.compile_shared_with_work_control(QUERY, &hit_control),
        Err(Error::QueryControl(QueryControlError::Cancelled))
    ));
}

#[test]
fn uncached_preflight_charges_each_pass_without_populating_cache() {
    let binding = binding();
    let work = clone_work();
    let build = build_work();
    let total = 2 * build + 4 * work;
    for allowance in [total - 1, total] {
        let control = budget(allowance);
        binding
            .compile_uncached_shared_with_work_control(QUERY, &control)
            .unwrap();
        let second = binding.compile_uncached_shared_with_work_control(QUERY, &control);
        assert_eq!(second.is_ok(), allowance == total);
        assert_eq!(
            control.consumed(QueryCharge::CompilerWork),
            if second.is_ok() {
                total
            } else {
                total - clone_cost().deep_clone_work
            }
        );
        assert_eq!(binding.cache_len(), 0);
    }
}

struct CancelAfterClone(QueryBudget, u64);
impl QueryControl for CancelAfterClone {
    fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
        self.0.checkpoint()
    }
    fn consume(
        &self,
        charge: QueryCharge,
        amount: u64,
    ) -> std::result::Result<(), QueryControlError> {
        self.0.consume(charge, amount)?;
        if charge == QueryCharge::CompilerWork && self.0.consumed(charge) >= self.1 {
            self.0.terminate(QueryControlError::Cancelled);
        }
        Ok(())
    }
    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.0.terminate(reason)
    }
}

#[test]
fn cancellation_between_clone_operations_prevents_cache_insertion() {
    let binding = binding();
    let work = key_work() + build_work() + clone_work();
    let control = CancelAfterClone(budget(u64::MAX), work);
    assert!(matches!(
        binding.compile_shared_with_work_control(QUERY, &control),
        Err(Error::QueryControl(QueryControlError::Cancelled))
    ));
    assert_eq!(control.0.consumed(QueryCharge::CompilerWork), work);
    assert_eq!(binding.cache_len(), 0);
}

#[test]
fn metered_cache_hits_still_check_scope_and_profile() {
    for wrong_profile in [false, true] {
        let binding = binding();
        let parsed = crate::parse_query(QUERY).unwrap();
        let plan = binding.compile_uncached_shared(QUERY).unwrap();
        let scope = if wrong_profile {
            binding.scope()
        } else {
            super::super::test_scope(SourceId::new(1).unwrap(), Dialect::Sqlite, Epoch(1))
        };
        let profile = if wrong_profile {
            CompileProfileId::GovernedV1
        } else {
            CompileProfileId::Uncontrolled
        };
        binding.cache().put(
            super::super::plan_key(&parsed, binding.scope()),
            CachedPlan::from_shared(scope, profile, plan),
        );
        assert!(matches!(
            binding.compile_shared_with_work_control(QUERY, &budget(key_work())),
            Err(Error::Mapping(_))
        ));
    }
}

struct CancelAfterInsertion<'a>(&'a CompilerBinding, QueryBudget);
impl QueryControl for CancelAfterInsertion<'_> {
    fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
        if self.0.cache_len() != 0 {
            self.1.terminate(QueryControlError::Cancelled);
        }
        self.1.checkpoint()
    }
    fn consume(
        &self,
        charge: QueryCharge,
        amount: u64,
    ) -> std::result::Result<(), QueryControlError> {
        self.1.consume(charge, amount)
    }
    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.1.terminate(reason)
    }
}

#[test]
fn cancellation_after_insertion_can_retain_only_the_completed_valid_plan() {
    let binding = binding();
    let control = CancelAfterInsertion(&binding, budget(u64::MAX));
    assert!(matches!(
        binding.compile_shared_with_work_control(QUERY, &control),
        Err(Error::QueryControl(QueryControlError::Cancelled))
    ));
    assert_eq!(binding.cache_len(), 1);
    let cached = binding
        .compile_shared_with_work_control(QUERY, &budget(key_work()))
        .unwrap();
    assert_eq!(
        format!("{cached:?}"),
        format!("{:?}", binding.compile_uncached_shared(QUERY).unwrap())
    );
}
