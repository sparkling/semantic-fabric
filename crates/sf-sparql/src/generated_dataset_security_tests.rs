use std::cell::Cell;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use rusqlite::Connection;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError as Stop, QueryLimits,
};
use sf_core::security_context::{
    PolicySnapshotId, RequestAttributesIdentity, SecurityContext, SubjectIdentity,
};
use sf_core::{SourceId, SourceMapping};
use sf_sql::Dialect;

use super::{SecurityCompileError, SecurityDatasetCompileError as E};
use crate::cache::generated::tests::{isolated, parse_spans};
use crate::cache::generated::{
    ConstantCoverageError, DatasetGraphAllowlist, DatasetRule, GeneratedDatasetError,
};
use crate::cache::SecurityPlanCache;
use crate::{exec, CompilerBinding, Epoch, Error, Plan, Tbox};

const A: &str = "http://ex/A";
const B: &str = "http://ex/B";
const SELECT: &str = "SELECT ?o FROM <http://ex/A> WHERE { ?s <http://ex/p> ?o }";
const ASK: &str = "ASK FROM <http://ex/A> { ?s <http://ex/p> ?o }";
const MISSING: &str = "SELECT * WHERE {}";

fn policy(value: u8) -> PolicySnapshotId {
    PolicySnapshotId::from_digest([value; 32]).unwrap()
}
fn context(p: u8, s: u8, a: u8) -> SecurityContext {
    SecurityContext::new(
        policy(p),
        SubjectIdentity::from_digest([s; 32]).unwrap(),
        RequestAttributesIdentity::from_digest([a; 32]).unwrap(),
    )
}
fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}
fn cache() -> SecurityPlanCache {
    SecurityPlanCache::new(NonZeroUsize::new(8).unwrap())
}
fn allow() -> DatasetGraphAllowlist {
    DatasetGraphAllowlist::new([A, B]).unwrap()
}
fn fixture() -> (CompilerBinding, Connection) {
    let mapping = sf_mapping::parse_r2rml(
        r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        <http://ex/m> rr:logicalTable [rr:tableName "items"];
          rr:subjectMap [rr:template "http://ex/{s}";rr:graph <http://ex/A>];
          rr:predicateObjectMap [rr:predicate <http://ex/p>;rr:objectMap [rr:column "o"]].
        <http://ex/other> rr:logicalTable [rr:tableName "other"];
          rr:subjectMap [rr:template "http://ex/{s}";rr:graph <http://ex/B>];
          rr:predicateObjectMap [rr:predicate <http://ex/p>;rr:objectMap [rr:column "o"]].
    "#,
    )
    .unwrap();
    let binding = CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), mapping),
        Dialect::Sqlite,
        Tbox::default(),
        Vec::new(),
        Epoch::default(),
        8,
    );
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE items(s TEXT,o TEXT); INSERT INTO items VALUES('s1','a'),('s2','a'); CREATE TABLE other(s TEXT,o TEXT); INSERT INTO other VALUES('s1','b');").unwrap();
    (binding, db)
}
fn compile(
    binding: &CompilerBinding,
    cache: &SecurityPlanCache,
    who: &SecurityContext,
    query: &str,
    control: &dyn QueryControl,
) -> Result<Arc<Plan>, E> {
    binding
        .for_security_policy(who.policy_snapshot(), cache)
        .compile_shared_with_single_default_dataset(
            who,
            query,
            &allow(),
            control,
            |_| Ok(()),
            |_| Ok(()),
        )
}
fn bag(plan: &Plan, db: &Connection) -> Vec<Vec<Option<String>>> {
    let mut rows: Vec<_> = exec::select(plan, db)
        .unwrap()
        .rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|v| v.as_ref().map(ToString::to_string))
                .collect::<Vec<_>>()
        })
        .collect();
    rows.sort();
    rows
}
fn cause(error: &E) -> Option<Stop> {
    match error {
        E::Dataset(GeneratedDatasetError::Compiler(Error::QueryControl(cause)))
        | E::Security(SecurityCompileError::Compiler(Error::QueryControl(cause))) => Some(*cause),
        _ => None,
    }
}
fn rule(error: &E) -> Option<DatasetRule> {
    match error {
        E::Dataset(GeneratedDatasetError::Refused(rule)) => Some(*rule),
        _ => None,
    }
}

#[test]
fn generated_dataset_security_preserves_exact_select_bags_and_ask() {
    let (binding, db) = fixture();
    let cache = cache();
    let who = context(1, 2, 3);
    for (graph, expected) in [
        (A, vec![vec![Some("\"a\"".into())]; 2]),
        (B, vec![vec![Some("\"b\"".into())]]),
    ] {
        let query = SELECT.replace(A, graph);
        let raw = binding
            .compile_uncached_shared_with_single_default_dataset(
                &query,
                &allow(),
                &budget(u64::MAX),
                |_| Ok(()),
                |_| Ok(()),
            )
            .unwrap();
        for _ in 0..2 {
            let protected = compile(&binding, &cache, &who, &query, &budget(u64::MAX)).unwrap();
            assert_eq!(bag(&protected, &db), expected);
            assert_eq!(bag(&protected, &db), bag(&raw, &db));
            assert_eq!(format!("{protected:?}"), format!("{raw:?}"));
        }
    }
    for (query, expected) in [
        (ASK, true),
        ("ASK FROM <http://ex/A> { ?s <http://ex/absent> ?o }", false),
    ] {
        let raw = binding
            .compile_uncached_shared_with_single_default_dataset(
                query,
                &allow(),
                &budget(u64::MAX),
                |_| Ok(()),
                |_| Ok(()),
            )
            .unwrap();
        for _ in 0..2 {
            let protected = compile(&binding, &cache, &who, query, &budget(u64::MAX)).unwrap();
            assert_eq!(exec::ask(&protected, &db).unwrap(), expected);
            assert_eq!(
                exec::ask(&protected, &db).unwrap(),
                exec::ask(&raw, &db).unwrap()
            );
            assert_eq!(format!("{protected:?}"), format!("{raw:?}"));
        }
    }
    assert_eq!(binding.cache_len(), 0);
}

#[test]
fn generated_dataset_security_context_partitions_reuse_and_never_uses_raw_cache() {
    let (binding, _) = fixture();
    let cache = cache();
    let who = context(1, 2, 3);
    let first = compile(&binding, &cache, &who, SELECT, &budget(u64::MAX)).unwrap();
    let same = compile(&binding, &cache, &who, SELECT, &budget(u64::MAX)).unwrap();
    assert!(Arc::ptr_eq(&first, &same));
    for other in [context(4, 2, 3), context(1, 4, 3), context(1, 2, 4)] {
        let next = compile(&binding, &cache, &other, SELECT, &budget(u64::MAX)).unwrap();
        assert!(!Arc::ptr_eq(&first, &next));
        let repeated = compile(&binding, &cache, &other, SELECT, &budget(u64::MAX)).unwrap();
        assert!(Arc::ptr_eq(&next, &repeated));
    }
    assert_eq!(cache.len(), 4);
    assert_eq!(binding.cache_len(), 0);
    let raw = binding
        .compile_shared_with_single_default_dataset(
            SELECT,
            &allow(),
            &budget(u64::MAX),
            |_| Ok(()),
            |_| Ok(()),
        )
        .unwrap();
    assert!(!Arc::ptr_eq(&raw, &first));
    let again = compile(&binding, &cache, &who, SELECT, &budget(u64::MAX)).unwrap();
    assert!(Arc::ptr_eq(&first, &again));
    assert_eq!(binding.cache_len(), 1);
    assert_eq!(cache.len(), 4);
}

#[test]
fn generated_dataset_security_wrong_policy_is_first_and_unpaid() {
    isolated(|| {
        let (binding, _) = fixture();
        let cache = cache();
        let control = ObservedControl::new(None);
        control.terminate(Stop::Cancelled);
        let (result, parses) = parse_spans(|| {
            binding
                .for_security_policy(policy(1), &cache)
                .compile_shared_with_single_default_dataset(
                    &context(4, 2, 3),
                    "not sparql",
                    &allow(),
                    &control,
                    |_| panic!("graph visited"),
                    |_| panic!("constant visited"),
                )
        });
        assert!(matches!(
            result,
            Err(E::Security(SecurityCompileError::PolicyMismatch))
        ));
        assert_eq!(parses, 0);
        assert_eq!(control.checkpoints.load(Ordering::SeqCst), 0);
        assert_eq!(control.budget.consumed(QueryCharge::CompilerWork), 0);
        assert_eq!(control.budget.terminal(), Some(Stop::Cancelled));
        assert_eq!(cache.access_counts(), (0, 0));
        assert_eq!(cache.len(), 0);
        assert_eq!(binding.cache_len(), 0);
    });
}

#[test]
fn generated_dataset_security_warm_hits_rerun_both_callbacks_before_lookup() {
    for warm in [false, true] {
        let (binding, _) = fixture();
        let cache = cache();
        let who = context(1, 2, 3);
        let compiler = binding.for_security_policy(policy(1), &cache);
        let saved = warm.then(|| {
            let calls = Cell::new((0, 0));
            let mut first = None;
            for _ in 0..2 {
                let before = cache.access_counts();
                let plan = compiler
                    .compile_shared_with_single_default_dataset(
                        &who,
                        SELECT,
                        &allow(),
                        &budget(u64::MAX),
                        |_| {
                            assert_eq!(cache.access_counts(), before);
                            let (g, c) = calls.get();
                            calls.set((g + 1, c));
                            Ok(())
                        },
                        |_| {
                            assert_eq!(cache.access_counts(), before);
                            let (g, c) = calls.get();
                            calls.set((g, c + 1));
                            Ok(())
                        },
                    )
                    .unwrap();
                if let Some(previous) = &first {
                    assert!(Arc::ptr_eq(previous, &plan));
                }
                first = Some(plan);
            }
            assert_eq!(calls.get(), (2, 2));
            first.unwrap()
        });
        let accesses = cache.access_counts();
        let entries = cache.len();
        for graph_denied in [true, false] {
            let error = compiler
                .compile_shared_with_single_default_dataset(
                    &who,
                    SELECT,
                    &allow(),
                    &budget(u64::MAX),
                    |_| {
                        assert_eq!(cache.access_counts(), accesses);
                        if graph_denied {
                            Err(ConstantCoverageError::Uncovered)
                        } else {
                            Ok(())
                        }
                    },
                    |_| {
                        assert!(!graph_denied);
                        assert_eq!(cache.access_counts(), accesses);
                        Err(ConstantCoverageError::Uncovered)
                    },
                )
                .unwrap_err();
            let expected = if graph_denied {
                DatasetRule::GraphNotAdmitted
            } else {
                DatasetRule::ConstantCoverage
            };
            assert_eq!(rule(&error), Some(expected));
            assert_eq!(cache.access_counts(), accesses);
            assert_eq!(cache.len(), entries);
        }
        let changed = DatasetGraphAllowlist::new([B]).unwrap();
        let result = compiler.compile_shared_with_single_default_dataset(
            &who,
            SELECT,
            &changed,
            &budget(u64::MAX),
            |_| panic!("not allowlisted"),
            |_| panic!("not allowlisted"),
        );
        let error = result.unwrap_err();
        assert_eq!(rule(&error), Some(DatasetRule::GraphNotAdmitted));
        assert_eq!(cache.access_counts(), accesses);
        assert_eq!(cache.len(), entries);
        assert_eq!(binding.cache_len(), 0);
        if let Some(saved) = saved {
            let again = compile(&binding, &cache, &who, SELECT, &budget(u64::MAX)).unwrap();
            assert!(Arc::ptr_eq(&saved, &again));
        }
    }
}

#[test]
fn generated_dataset_security_refusals_do_not_populate_caches_or_leak_text() {
    let (binding, _) = fixture();
    let cache = cache();
    let who = context(1, 2, 3);
    for (query, expected) in [
        (MISSING, DatasetRule::MissingDataset),
        (
            "SELECT * FROM NAMED <http://secret.invalid/g> WHERE {}",
            DatasetRule::NamedGraphDataset,
        ),
        (
            "SELECT * FROM <http://secret.invalid/g> WHERE {}",
            DatasetRule::GraphNotAdmitted,
        ),
        (
            "SELECT * FROM <http://ex/A> FROM <http://ex/B> WHERE {}",
            DatasetRule::MultipleDefaultGraphs,
        ),
        ("not sparql", DatasetRule::FormNotAdmitted),
    ] {
        let error = compile(&binding, &cache, &who, query, &budget(u64::MAX)).unwrap_err();
        assert!(matches!(&error, E::Dataset(GeneratedDatasetError::Refused(r)) if *r == expected));
        assert!(!format!("{error} {error:?}").contains("secret.invalid"));
        assert_eq!(cache.access_counts(), (0, 0));
        assert_eq!(cache.len(), 0);
        assert_eq!(binding.cache_len(), 0);
    }
}

#[test]
fn generated_dataset_security_parses_once_cold_warm_and_refused() {
    isolated(|| {
        let (binding, _) = fixture();
        let cache = cache();
        let who = context(1, 2, 3);
        for _ in 0..2 {
            let (result, count) =
                parse_spans(|| compile(&binding, &cache, &who, SELECT, &budget(u64::MAX)));
            result.unwrap();
            assert_eq!(count, 1);
        }
        let accesses = cache.access_counts();
        let (result, count) =
            parse_spans(|| compile(&binding, &cache, &who, MISSING, &budget(u64::MAX)));
        let error = result.unwrap_err();
        assert_eq!(rule(&error), Some(DatasetRule::MissingDataset));
        assert_eq!(count, 1);
        assert_eq!(cache.access_counts(), accesses);
        assert_eq!(cache.len(), 1);
        assert_eq!(binding.cache_len(), 0);
    });
}

struct ObservedControl {
    budget: QueryBudget,
    checkpoints: AtomicUsize,
    stop_after_entry: Option<Stop>,
}
impl ObservedControl {
    fn new(stop_after_entry: Option<Stop>) -> Self {
        Self {
            budget: budget(u64::MAX),
            checkpoints: AtomicUsize::new(0),
            stop_after_entry,
        }
    }
}
impl QueryControl for ObservedControl {
    fn checkpoint(&self) -> Result<(), Stop> {
        if self.checkpoints.fetch_add(1, Ordering::SeqCst) > 0 {
            if let Some(cause) = self.stop_after_entry {
                self.budget.terminate(cause);
            }
        }
        self.budget.checkpoint()
    }
    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), Stop> {
        self.budget.consume(charge, amount)
    }
    fn terminate(&self, reason: Stop) -> Stop {
        self.budget.terminate(reason)
    }
}

#[test]
fn generated_dataset_security_malformed_parse_keeps_sticky_control() {
    for stop in [Stop::Cancelled, Stop::DeadlineExceeded] {
        let (binding, _) = fixture();
        let cache = cache();
        let control = ObservedControl::new(Some(stop));
        let error =
            compile(&binding, &cache, &context(1, 2, 3), "not sparql", &control).unwrap_err();
        assert_eq!(cause(&error), Some(stop));
        assert_eq!(control.budget.terminal(), Some(stop));
        assert_eq!(control.checkpoints.load(Ordering::SeqCst), 2);
        assert_eq!(cache.access_counts(), (0, 0));
        assert_eq!(cache.len(), 0);
        assert_eq!(binding.cache_len(), 0);
    }
}

#[test]
fn generated_dataset_security_exact_work_boundary_and_callback_stop() {
    let (binding, _) = fixture();
    let who = context(1, 2, 3);
    let measured = budget(u64::MAX);
    compile(&binding, &cache(), &who, SELECT, &measured).unwrap();
    let total = measured.consumed(QueryCharge::CompilerWork);
    assert!(total > 0);
    compile(&binding, &cache(), &who, SELECT, &budget(total)).unwrap();
    let short_cache = cache();
    let short = budget(total - 1);
    let error = compile(&binding, &short_cache, &who, SELECT, &short).unwrap_err();
    assert_eq!(cause(&error), Some(Stop::CompilerWorkExceeded));
    assert_eq!(short.terminal(), Some(Stop::CompilerWorkExceeded));
    assert_eq!(short_cache.len(), 0);
    for stop in [Stop::Cancelled, Stop::DeadlineExceeded] {
        for warm in [false, true] {
            let cache = cache();
            let saved =
                warm.then(|| compile(&binding, &cache, &who, SELECT, &budget(u64::MAX)).unwrap());
            let accesses = cache.access_counts();
            let entries = cache.len();
            let stopped = budget(u64::MAX);
            stopped.terminate(stop);
            let error = compile(&binding, &cache, &who, SELECT, &stopped).unwrap_err();
            assert_eq!(cause(&error), Some(stop));
            for graph in [true, false] {
                let control = budget(u64::MAX);
                let result = binding
                    .for_security_policy(policy(1), &cache)
                    .compile_shared_with_single_default_dataset(
                        &who,
                        SELECT,
                        &allow(),
                        &control,
                        |_| {
                            if graph {
                                control.terminate(stop);
                            }
                            Ok(())
                        },
                        |_| {
                            if !graph {
                                control.terminate(stop);
                            }
                            Err(ConstantCoverageError::Uncovered)
                        },
                    );
                assert_eq!(cause(&result.unwrap_err()), Some(stop));
                assert_eq!(control.terminal(), Some(stop));
            }
            assert_eq!(cache.access_counts(), accesses);
            assert_eq!(cache.len(), entries);
            if let Some(saved) = saved {
                let again = compile(&binding, &cache, &who, SELECT, &budget(u64::MAX)).unwrap();
                assert!(Arc::ptr_eq(&saved, &again));
            }
        }
    }
    assert_eq!(binding.cache_len(), 0);
}
