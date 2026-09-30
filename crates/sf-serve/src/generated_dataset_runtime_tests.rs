use super::super::tests::{assert_answers, bind, bind_default, mapping_text, ASK, FREE, SELECT};
use super::{GeneratedDatasetRuntimeError as E, *};
use crate::semantic_admission::MappingOrigin;
use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError as Stop, QueryLimits};
use sf_sparql::cache::generated::{DatasetRule, GeneratedDatasetError};
use std::sync::Arc;

fn route(
    binding: &RuntimeBinding,
    preflight: bool,
    control: &dyn QueryControl,
) -> Result<Arc<Plan>, E> {
    if preflight {
        binding.preflight_generated_single_default(SELECT_DATASET, &pinned(), control)
    } else {
        binding
            .compile_generated_single_default(SELECT_DATASET, &pinned(), control)
            .map(|compiled| compiled.plan.plan)
    }
}

#[test]
fn generated_dataset_matching_warm_entries_do_not_bypass_coverage() {
    let binding = binding();
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
        let allowlist = DatasetGraphAllowlist::new([graph]).unwrap();
        let warm = || {
            binding
                .compiler
                .compile_shared_with_single_default_dataset(
                    &query,
                    &allowlist,
                    FREE,
                    |_| Ok(()),
                    |_| Ok(()),
                )
                .unwrap()
        };
        let first = warm();
        assert!(Arc::ptr_eq(&first, &warm()));
        let list = PinnedGraphAllowlist::new([graph]).unwrap();
        for error in [
            binding
                .compile_generated_single_default(&query, &list, FREE)
                .unwrap_err(),
            binding
                .preflight_generated_single_default(&query, &list, FREE)
                .unwrap_err(),
        ] {
            assert!(
                matches!(error, E::Dataset(GeneratedDatasetError::Refused(found)) if found == rule)
            );
        }
        assert!(Arc::ptr_eq(&first, &warm()));
    }
}

#[test]
fn generated_dataset_preflight_does_not_populate_cache_or_change_v1_bytes() {
    let control = || QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    let pristine = control();
    binding()
        .compile_generated_single_default(SELECT_DATASET, &pinned(), &pristine)
        .unwrap();
    let binding = binding();
    let original = binding.generated.identity().wire();
    let first = binding
        .preflight_generated_single_default(SELECT_DATASET, &pinned(), FREE)
        .unwrap();
    let second = binding
        .preflight_generated_single_default(SELECT_DATASET, &pinned(), FREE)
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    let after_preflight = control();
    let admitted = binding
        .compile_generated_single_default(SELECT_DATASET, &pinned(), &after_preflight)
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &admitted.plan.plan));
    assert!(!Arc::ptr_eq(&second, &admitted.plan.plan));
    assert_eq!(
        pristine.consumed(QueryCharge::CompilerWork),
        after_preflight.consumed(QueryCharge::CompilerWork)
    );
    assert_ne!(original, admitted.identity.wire());
    assert_eq!(original, binding.generated.identity().wire());
    let before = binding.compile_generated(ASK, FREE).unwrap();
    let bytes = before.identity.wire();
    binding
        .compile_generated_single_default(ASK_DATASET, &pinned(), FREE)
        .unwrap();
    let after = binding.compile_generated(ASK, FREE).unwrap();
    assert_eq!(bytes.as_bytes(), after.identity.wire().as_bytes());
    assert_eq!(bytes.as_bytes(), original.as_bytes());
    assert!(Arc::ptr_eq(&before.plan.plan, &after.plan.plan));
}

#[test]
fn generated_dataset_all_unscoped_routes_have_exact_work_and_midwork_stops() {
    use super::super::deferred_test_controls::Trip;
    use std::sync::atomic::Ordering;
    let budget = |work| QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX));
    for (warm, preflight) in [(false, false), (true, false), (false, true)] {
        let prepare = || {
            let binding = binding();
            if warm {
                route(&binding, false, FREE).unwrap();
            }
            binding
        };
        let measured = budget(u64::MAX);
        route(&prepare(), preflight, &measured).unwrap();
        let n = measured.consumed(QueryCharge::CompilerWork);
        assert!(n > 0);
        route(&prepare(), preflight, &budget(n)).unwrap();
        let short = budget(n - 1);
        assert!(matches!(
            route(&prepare(), preflight, &short),
            Err(E::Dataset(GeneratedDatasetError::Compiler(
                sf_sparql::Error::QueryControl(Stop::CompilerWorkExceeded)
            )))
        ));
        assert_eq!(short.terminal(), Some(Stop::CompilerWorkExceeded));
        let measured = Trip::new(u64::MAX, Stop::Cancelled);
        route(&prepare(), preflight, &measured).unwrap();
        let calls = measured.calls.load(Ordering::SeqCst);
        for cause in [Stop::Cancelled, Stop::DeadlineExceeded] {
            for at in [1, calls / 2, calls] {
                let binding = prepare();
                let stopped = Trip::new(at.max(1), cause);
                assert!(
                    matches!(route(&binding, preflight, &stopped), Err(E::Dataset(GeneratedDatasetError::Compiler(sf_sparql::Error::QueryControl(found)))) if found == cause)
                );
                assert_eq!(stopped.checkpoint(), Err(cause));
                assert!(
                    matches!(route(&binding, preflight, &stopped), Err(E::Dataset(GeneratedDatasetError::Compiler(sf_sparql::Error::QueryControl(found)))) if found == cause)
                );
                route(&binding, preflight, FREE).unwrap();
            }
        }
    }
}

pub(super) const SELECT_DATASET: &str =
    "SELECT ?n FROM <http://ex/g> WHERE { ?p a <http://ex/Person>; <http://ex/name> ?n }";
pub(super) const ASK_DATASET: &str = "ASK FROM <http://ex/g> { ?p <http://ex/name> ?n }";

pub(super) fn binding() -> RuntimeBinding {
    let mapping = mapping_text(true, true).replace(
        "; rr:graphMap [ rr:constant <http://ex/g> ]",
        "; rr:graph <http://ex/g>, <http://ex/h>",
    );
    bind(&mapping, MappingOrigin::Authored, &[])
}

pub(super) fn pinned() -> PinnedGraphAllowlist {
    PinnedGraphAllowlist::new(["http://ex/g", "http://ex/h"]).unwrap()
}

#[test]
fn generated_dataset_errors_redact_compiler_details_without_losing_cause() {
    for error in [
        E::Dataset(GeneratedDatasetError::Compiler(sf_sparql::Error::Mapping(
            "private-mapping-detail".into(),
        ))),
        E::Security(SecurityCompileError::Compiler(sf_sparql::Error::Mapping(
            "private-mapping-detail".into(),
        ))),
    ] {
        assert!(!format!("{error} {error:?}").contains("private-mapping-detail"));
        let cause = match error {
            E::Dataset(GeneratedDatasetError::Compiler(cause))
            | E::Security(SecurityCompileError::Compiler(cause)) => cause,
            _ => panic!("typed compiler cause lost"),
        };
        assert!(
            matches!(cause, sf_sparql::Error::Mapping(message) if message == "private-mapping-detail")
        );
    }
}

#[test]
fn generated_dataset_cold_warm_and_uncached_are_exact_and_bound() {
    let binding = binding();
    let pinned = pinned();
    for (query, expected) in [(SELECT_DATASET, SELECT), (ASK_DATASET, ASK)] {
        let cold = binding
            .compile_generated_single_default(query, &pinned, FREE)
            .unwrap();
        let warm = binding
            .compile_generated_single_default(query, &pinned, FREE)
            .unwrap();
        assert!(Arc::ptr_eq(&cold.plan.plan, &warm.plan.plan));
        assert_eq!(cold.identity, warm.identity);
        assert_ne!(cold.identity, binding.generated.identity());
        let uncached = binding
            .preflight_generated_single_default(query, &pinned, FREE)
            .unwrap();
        assert!(!Arc::ptr_eq(&cold.plan.plan, &uncached));
        assert_answers(&binding, expected, cold.plan);
        assert_answers(&binding, expected, warm.plan);
    }
    let g = binding
        .compile_generated_single_default(SELECT_DATASET, &pinned, FREE)
        .unwrap();
    let h = binding
        .compile_generated_single_default(
            &SELECT_DATASET.replace("<http://ex/g>", "<http://ex/h>"),
            &pinned,
            FREE,
        )
        .unwrap();
    assert!(!Arc::ptr_eq(&g.plan.plan, &h.plan.plan));
    assert_answers(&binding, SELECT, h.plan);
}

#[test]
fn generated_dataset_rechecks_allowlist_and_mapping_on_warm_paths() {
    let binding = binding();
    let pinned = pinned();
    binding
        .compile_generated_single_default(SELECT_DATASET, &pinned, FREE)
        .unwrap();
    let empty = PinnedGraphAllowlist::new([] as [&str; 0]).unwrap();
    let unknown = PinnedGraphAllowlist::new(["http://ex/unknown"]).unwrap();
    for (query, list, rule) in [
        (
            SELECT_DATASET.to_owned(),
            &empty,
            DatasetRule::GraphNotAdmitted,
        ),
        (
            SELECT_DATASET.replace("<http://ex/g>", "<http://ex/unknown>"),
            &unknown,
            DatasetRule::GraphNotAdmitted,
        ),
        (
            SELECT_DATASET.replace("<http://ex/name>", "<http://ex/unknown>"),
            &pinned,
            DatasetRule::ConstantCoverage,
        ),
    ] {
        for error in [
            binding
                .compile_generated_single_default(&query, list, FREE)
                .unwrap_err(),
            binding
                .preflight_generated_single_default(&query, list, FREE)
                .unwrap_err(),
        ] {
            assert!(
                matches!(error, E::Dataset(GeneratedDatasetError::Refused(found)) if found == rule)
            );
            assert!(!format!("{error:?} {error}").contains("http://ex"));
        }
    }
}

#[test]
fn generated_dataset_receipt_mismatch_precedes_parser_and_stopped_control() {
    let mut binding = binding();
    binding.generated = bind_default().generated;
    let control = QueryBudget::new(QueryLimits::new(0, 0, 0, 0));
    control.terminate(Stop::Cancelled);
    assert!(matches!(
        binding.compile_generated_single_default("invalid", &pinned(), &control),
        Err(E::Receipt(_))
    ));
    assert!(matches!(
        binding.preflight_generated_single_default("invalid", &pinned(), &control),
        Err(E::Receipt(_))
    ));
}

#[test]
fn generated_dataset_uncached_exact_work_sticky_stop_and_recovery() {
    let binding = binding();
    let pinned = pinned();
    let budget = |work| QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX));
    let measured = budget(u64::MAX);
    binding
        .preflight_generated_single_default(SELECT_DATASET, &pinned, &measured)
        .unwrap();
    let total = measured.consumed(QueryCharge::CompilerWork);
    assert!(total > 0);
    binding
        .preflight_generated_single_default(SELECT_DATASET, &pinned, &budget(total))
        .unwrap();
    let short = budget(total - 1);
    assert!(matches!(
        binding.preflight_generated_single_default(SELECT_DATASET, &pinned, &short),
        Err(E::Dataset(GeneratedDatasetError::Compiler(
            sf_sparql::Error::QueryControl(Stop::CompilerWorkExceeded)
        )))
    ));
    for reason in [Stop::Cancelled, Stop::DeadlineExceeded] {
        let stopped = budget(u64::MAX);
        stopped.terminate(reason);
        assert!(
            matches!(binding.compile_generated_single_default(SELECT_DATASET, &pinned, &stopped), Err(E::Dataset(GeneratedDatasetError::Compiler(sf_sparql::Error::QueryControl(found)))) if found == reason)
        );
    }
    binding
        .compile_generated_single_default(SELECT_DATASET, &pinned, FREE)
        .unwrap();
}
