#![cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]

use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits};
use std::sync::Arc;

fn binding() -> sf_sparql::CompilerBinding {
    sf_sparql::CompilerBinding::from_unverified_observation(
        sf_core::SourceMapping::new(sf_core::SourceId::new(0).unwrap(), vec![]),
        sf_serve::BackendKind::Sqlite.dialect(),
        Default::default(),
        vec![],
        Default::default(),
        1,
    )
}

fn budget(work: u64) -> Arc<QueryBudget> {
    Arc::new(QueryBudget::new(QueryLimits::new(
        work,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    )))
}

#[test]
fn describe_direct_and_isolated_parser_have_exact_cold_and_warm_work() {
    let runtime = sf_sparql::ParserRuntime::prepare(std::path::Path::new(env!(
        "CARGO_BIN_EXE_semantic-fabric"
    )))
    .unwrap();
    for source in [
        "SELECT (COUNT(*) AS ?n) WHERE { VALUES ?x { 1 1 2 } }",
        "SELECT (COUNT(*) AS ?n) (COUNT(*) AS ?again) (SUM(?x) AS ?sum) WHERE { VALUES ?x { 1 1 2 } }",
        "DESCRIBE <urn:a>",
        "DESCRIBE <urn:a> WHERE { VALUES ?__sf_cache0 { <urn:c> } }",
    ] {
        let direct = binding();
        let cold = budget(u64::MAX);
        let expected = direct
            .compile_shared_with_work_control(source, cold.as_ref())
            .unwrap();
        let total = cold.consumed(QueryCharge::CompilerWork);
        let warm = budget(u64::MAX);
        assert!(Arc::ptr_eq(
            &expected,
            &direct
                .compile_shared_with_work_control(source, warm.as_ref())
                .unwrap()
        ));
        let warm_total = warm.consumed(QueryCharge::CompilerWork);
        assert!(0 < warm_total && warm_total < total && total <= 1_000_000);
        for _ in 0..16 {
            let isolated = binding();
            let short = budget(total - 1);
            assert!(matches!(
                runtime.with_request(short.clone(), || isolated
                    .compile_shared_with_work_control(source, short.as_ref())),
                Err(sf_sparql::Error::QueryControl(
                    sf_core::query_control::QueryControlError::CompilerWorkExceeded
                ))
            ));
            let paid = budget(total);
            let actual = runtime
                .with_request(paid.clone(), || {
                    isolated.compile_shared_with_work_control(source, paid.as_ref())
                })
                .unwrap();
            assert_eq!(paid.consumed(QueryCharge::CompilerWork), total);
            assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
            let hit_budget = budget(warm_total);
            let hit = runtime
                .with_request(hit_budget.clone(), || {
                    isolated.compile_shared_with_work_control(source, hit_budget.as_ref())
                })
                .unwrap();
            assert!(Arc::ptr_eq(&actual, &hit));
            assert_eq!(hit_budget.consumed(QueryCharge::CompilerWork), warm_total);
        }
    }
}
