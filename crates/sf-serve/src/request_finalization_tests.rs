//! Public finalization cutoff is observed independently of total compiler work.
#[test]
fn finalization_refuses_before_source_and_recovers_exact_cold_warm_bags() {
    super::structural_normalization::mapped_process(
        "request_compile::tests::finalization_work::finalization_refuses_before_source_and_recovers_exact_cold_warm_bags",
        "finalize",
    );
}

#[test]
fn aggregate_finalization_refuses_before_source_and_recovers_cold_warm() {
    super::structural_normalization::mapped_query_process(
        "request_compile::tests::finalization_work::aggregate_finalization_refuses_before_source_and_recovers_cold_warm",
        "finalize",
        "SELECT DISTINCT ?value (COUNT(*) AS ?count) (COUNT(*) AS ?again) WHERE { ?item <http://example.test/a> ?value } GROUP BY ?value",
        &["one", "two"],
        Some(&[("one", "2"), ("two", "1")]),
    );
}

#[test]
fn union_aggregate_aliases_preserve_counts_cold_and_warm() {
    super::structural_normalization::mapped_query_process(
        "request_compile::tests::finalization_work::union_aggregate_aliases_preserve_counts_cold_and_warm",
        "finalize",
        "SELECT ?value (COUNT(*) AS ?count) (COUNT(*) AS ?again) WHERE { { ?item <http://example.test/a> ?value } UNION { ?item <http://example.test/a> ?value } } GROUP BY ?value",
        &["one", "two"],
        Some(&[("one", "4"), ("two", "2")]),
    );
}

#[test]
fn construct_finalization_preserves_exact_graph_cold_and_warm() {
    super::structural_normalization::mapped_query_process(
        "request_compile::tests::finalization_work::construct_finalization_preserves_exact_graph_cold_and_warm",
        "finalize",
        "CONSTRUCT { ?item <http://example.test/a> ?value } WHERE { ?item <http://example.test/a> ?value }",
        &[
            "<http://example.test/item/1> <http://example.test/a> \"one\" .",
            "<http://example.test/item/2> <http://example.test/a> \"one\" .",
            "<http://example.test/item/3> <http://example.test/a> \"two\" .",
        ],
        None,
    );
}
