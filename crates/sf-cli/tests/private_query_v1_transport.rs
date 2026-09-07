#![cfg(all(
    feature = "query-v1-transport-evidence",
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
))]

use std::path::Path;

const BINARY: &str = env!("CARGO_BIN_EXE_semantic-fabric");

#[test]
fn held_binary_completes_parser_free_query_v1_exchange_for_unrelated_sources() {
    for source in ["", "ASK {}", "this is deliberately not SPARQL"] {
        sf_sparql::exercise_synthetic_query_v1_transport_for_evidence(Path::new(BINARY), source)
            .expect("complete parser-free QueryV1 exchange");
    }
}

#[test]
fn transport_accepts_the_exact_source_cap_and_recovers_for_the_next_launch() {
    let full_source = "#".repeat(1 << 20);
    sf_sparql::exercise_synthetic_query_v1_transport_for_evidence(Path::new(BINARY), &full_source)
        .expect("transport the exact one-MiB source cap");

    sf_sparql::exercise_synthetic_query_v1_transport_for_evidence(Path::new(BINARY), "next")
        .expect("a fresh held-binary launch remains healthy");
}

#[test]
fn over_limit_source_fails_before_launch_without_poisoning_recovery() {
    let error = sf_sparql::exercise_synthetic_query_v1_transport_for_evidence(
        Path::new(BINARY),
        &"X".repeat((1 << 20) + 1),
    )
    .expect_err("an over-limit source must fail before a child exists");
    assert_eq!(
        error,
        "parser request preparation: parser protocol source limit exceeded"
    );

    sf_sparql::exercise_synthetic_query_v1_transport_for_evidence(Path::new(BINARY), "recovery")
        .expect("prelaunch refusal cannot poison the next launch");
}

#[test]
fn transport_evidence_rejects_relative_and_parent_traversal_paths_first() {
    for path in [Path::new("semantic-fabric"), Path::new("/tmp/../bin/false")] {
        let error = sf_sparql::exercise_synthetic_query_v1_transport_for_evidence(
            path,
            &"X".repeat((1 << 20) + 1),
        )
        .expect_err("non-normalized evidence path must fail closed");
        assert_eq!(
            error,
            "evidence executable path must be absolute and normalized"
        );
    }
}
