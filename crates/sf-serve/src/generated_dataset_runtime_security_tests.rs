use super::super::deferred_test_controls::{context, policy, request};
use super::tests::{binding, pinned, SELECT_DATASET};
use super::{GeneratedDatasetRuntimeError as E, *};
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError as Stop};
use sf_sparql::cache::generated::{DatasetRule, GeneratedDatasetError};
use std::sync::Arc;

#[test]
fn generated_dataset_secured_select_ask_keep_exact_owner_and_context() {
    use super::super::tests::{assert_answers, ASK, SELECT};
    use super::tests::ASK_DATASET;
    let binding = binding();
    let who = context(2, 3);
    for (query, ordinary) in [(SELECT_DATASET, SELECT), (ASK_DATASET, ASK)] {
        let compile = || {
            binding
                .compile_generated_single_default_secured(
                    query,
                    &pinned(),
                    &request(Some(who), u64::MAX),
                    policy(1),
                )
                .unwrap()
        };
        let cold = compile();
        let warm = compile();
        assert!(Arc::ptr_eq(&cold.plan.plan, &warm.plan.plan));
        for compiled in [cold, warm] {
            assert_eq!(compiled.plan.security, Some(who));
            assert_eq!(compiled.plan.scope, binding.scope());
            assert_eq!(compiled.plan.source_id(), binding.source_id());
            assert_answers(&binding, ordinary, compiled.plan);
        }
        let other = super::tests::binding();
        assert!(other.prepare_execution(compile().plan).is_err());
    }
}

#[test]
fn generated_dataset_secured_matching_warm_entries_still_require_coverage() {
    let binding = binding();
    let who = context(2, 3);
    for (query, graph, rule) in [
        (
            SELECT_DATASET.replace("<http://ex/g>", "<http://ex/unknown>"),
            "http://ex/unknown",
            DatasetRule::GraphNotAdmitted,
        ),
        (
            SELECT_DATASET.replace("<http://ex/name>", "<http://ex/unknown>"),
            "http://ex/g",
            DatasetRule::ConstantCoverage,
        ),
    ] {
        let list = DatasetGraphAllowlist::new([graph]).unwrap();
        let raw = || {
            binding
                .compiler
                .for_security_policy(policy(1), &binding.security_cache)
                .compile_shared_with_single_default_dataset(
                    &who,
                    &query,
                    &list,
                    &request(Some(who), u64::MAX),
                    |_| Ok(()),
                    |_| Ok(()),
                )
                .unwrap()
        };
        let first = raw();
        assert!(Arc::ptr_eq(&first, &raw()));
        let pinned = PinnedGraphAllowlist::new([graph]).unwrap();
        for _ in 0..2 {
            assert!(matches!(binding.compile_generated_single_default_secured(
                &query, &pinned, &request(Some(who), u64::MAX), policy(1)),
                Err(E::Dataset(GeneratedDatasetError::Refused(found))) if found == rule));
        }
        assert!(Arc::ptr_eq(&first, &raw()));
    }
}

fn stopped(result: Result<GeneratedCompiled, E>, expected: Stop) {
    let found = match result {
        Err(E::Dataset(GeneratedDatasetError::Compiler(sf_sparql::Error::QueryControl(cause))))
        | Err(E::Security(SecurityCompileError::Compiler(sf_sparql::Error::QueryControl(cause)))) => {
            cause
        }
        other => panic!("expected stopped compilation: {other:?}"),
    };
    assert_eq!(found, expected);
}

#[test]
fn generated_dataset_secured_cold_warm_exact_limits_and_sticky_recovery() {
    for warm in [false, true] {
        let prepare = || {
            let binding = binding();
            if warm {
                binding
                    .compile_generated_single_default_secured(
                        SELECT_DATASET,
                        &pinned(),
                        &request(Some(context(2, 3)), u64::MAX),
                        policy(1),
                    )
                    .unwrap();
            }
            binding
        };
        let compile = |binding: &RuntimeBinding, budget: &RequestBudget| {
            binding.compile_generated_single_default_secured(
                SELECT_DATASET,
                &pinned(),
                budget,
                policy(1),
            )
        };
        let measured = request(Some(context(2, 3)), u64::MAX);
        compile(&prepare(), &measured).unwrap();
        let n = measured.consumed(QueryCharge::CompilerWork);
        assert!(n > 0);
        compile(&prepare(), &request(Some(context(2, 3)), n)).unwrap();
        let binding = prepare();
        let short = request(Some(context(2, 3)), n - 1);
        stopped(compile(&binding, &short), Stop::CompilerWorkExceeded);
        stopped(compile(&binding, &short), Stop::CompilerWorkExceeded);
        compile(&binding, &request(Some(context(2, 3)), u64::MAX)).unwrap();
        for cause in [Stop::Cancelled, Stop::DeadlineExceeded] {
            let budget = request(Some(context(2, 3)), u64::MAX);
            budget.terminate(cause);
            stopped(compile(&binding, &budget), cause);
            stopped(compile(&binding, &budget), cause);
            assert_eq!(budget.checkpoint(), Err(cause));
            compile(&binding, &request(Some(context(2, 3)), u64::MAX)).unwrap();
        }
    }
}

#[test]
fn generated_dataset_security_is_policy_first_and_cache_partitioned() {
    let binding = binding();
    let pinned = pinned();
    let empty = request(None, u64::MAX);
    empty.terminate(Stop::Cancelled);
    assert!(matches!(
        binding.compile_generated_single_default_secured("invalid", &pinned, &empty, policy(1)),
        Err(E::MissingSecurityContext)
    ));
    let stopped = request(Some(context(2, 3)), u64::MAX);
    stopped.terminate(Stop::Cancelled);
    assert!(matches!(
        binding.compile_generated_single_default_secured("invalid", &pinned, &stopped, policy(2)),
        Err(E::PolicyMismatch)
    ));
    assert_eq!(stopped.consumed(QueryCharge::CompilerWork), 0);
    assert!(matches!(
        binding.compile_generated_single_default_secured(
            SELECT_DATASET,
            &pinned,
            &stopped,
            policy(1)
        ),
        Err(E::Dataset(GeneratedDatasetError::Compiler(
            sf_sparql::Error::QueryControl(Stop::Cancelled)
        )))
    ));
    let compile = |who| {
        binding
            .compile_generated_single_default_secured(
                SELECT_DATASET,
                &pinned,
                &request(Some(who), u64::MAX),
                policy(1),
            )
            .unwrap()
    };
    let first = compile(context(2, 3));
    let warm = compile(context(2, 3));
    assert!(Arc::ptr_eq(&first.plan.plan, &warm.plan.plan));
    assert_eq!(first.identity, warm.identity);
    for other in [context(3, 3), context(2, 4)] {
        let compiled = compile(other.clone());
        assert!(!Arc::ptr_eq(&first.plan.plan, &compiled.plan.plan));
        assert_eq!(compiled.plan.security, Some(other));
    }
    let unprotected = binding
        .compile_generated_single_default(
            SELECT_DATASET,
            &pinned,
            &sf_core::query_control::UncontrolledQueryControl,
        )
        .unwrap();
    assert!(!Arc::ptr_eq(&first.plan.plan, &unprotected.plan.plan));
    let other_policy = sf_core::security_context::SecurityContext::new(
        policy(2),
        sf_core::security_context::SubjectIdentity::from_digest([2; 32]).unwrap(),
        sf_core::security_context::RequestAttributesIdentity::from_digest([3; 32]).unwrap(),
    );
    let other = binding
        .compile_generated_single_default_secured(
            SELECT_DATASET,
            &pinned,
            &request(Some(other_policy), u64::MAX),
            policy(2),
        )
        .unwrap();
    assert!(!Arc::ptr_eq(&first.plan.plan, &other.plan.plan));
}

#[test]
fn generated_dataset_security_rechecks_graph_and_constant_on_cache_hits() {
    let binding = binding();
    let pinned = pinned();
    let compile = |query: &str, list: &PinnedGraphAllowlist| {
        binding.compile_generated_single_default_secured(
            query,
            list,
            &request(Some(context(2, 3)), u64::MAX),
            policy(1),
        )
    };
    compile(SELECT_DATASET, &pinned).unwrap();
    let empty = PinnedGraphAllowlist::new([] as [&str; 0]).unwrap();
    assert!(matches!(
        compile(SELECT_DATASET, &empty),
        Err(E::Dataset(GeneratedDatasetError::Refused(
            DatasetRule::GraphNotAdmitted
        )))
    ));
    let unknown = SELECT_DATASET.replace("<http://ex/name>", "<http://ex/unknown>");
    assert!(matches!(
        compile(&unknown, &pinned),
        Err(E::Dataset(GeneratedDatasetError::Refused(
            DatasetRule::ConstantCoverage
        )))
    ));
    compile(SELECT_DATASET, &pinned).unwrap();
}
