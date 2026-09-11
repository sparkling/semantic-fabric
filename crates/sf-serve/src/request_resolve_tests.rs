//! Actual RESOLVE span admission, not a total-compiler-derived threshold.
#[test]
fn mapped_resolve_refuses_before_source_and_recovers_exact_cold_warm_bags() {
    super::structural_normalization::mapped_process(
        "request_compile::tests::resolve_work::mapped_resolve_refuses_before_source_and_recovers_exact_cold_warm_bags",
        "resolve",
    );
}
