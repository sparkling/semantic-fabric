#![cfg(all(
    feature = "parser-worker-evidence",
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
))]

use std::path::Path;

const BINARY: &str = env!("CARGO_BIN_EXE_semantic-fabric");

#[test]
fn held_binary_observes_only_the_sealed_real_parser_corpus() {
    let first =
        sf_sparql::exercise_private_parser_observation_corpus_for_evidence(Path::new(BINARY))
            .expect("observe the sealed parser corpus");

    assert_eq!(first.case_count(), 7);
    assert_eq!(first.parsed_count(), 6);
    assert_eq!(first.syntax_rejection_count(), 1);
    assert_eq!(
        first.corpus_digest(),
        [
            58, 112, 21, 141, 12, 37, 190, 214, 3, 79, 90, 157, 105, 171, 4, 140, 234, 219, 157,
            167, 120, 39, 196, 109, 52, 92, 73, 111, 188, 214, 205, 68,
        ]
    );

    let second =
        sf_sparql::exercise_private_parser_observation_corpus_for_evidence(Path::new(BINARY))
            .expect("every case and repeated corpus run must use clean fresh children");
    assert_eq!(second, first);
}

#[test]
fn held_binary_returns_real_query_v1_with_direct_parse_equivalence() {
    let first = sf_sparql::exercise_private_parser_query_v1_corpus_for_evidence(Path::new(BINARY))
        .expect("verify parser-produced QueryV1 corpus");

    assert_eq!(first.case_count(), 7);
    assert_eq!(first.parsed_count(), 6);
    assert_eq!(first.syntax_rejection_count(), 1);

    let second = sf_sparql::exercise_private_parser_query_v1_corpus_for_evidence(Path::new(BINARY))
        .expect("every parser-produced QueryV1 case must use a clean fresh child");
    assert_eq!(second, first);
}

#[test]
fn parser_observation_rejects_unheld_paths_before_any_child_launch() {
    for path in [Path::new("semantic-fabric"), Path::new("/tmp/../bin/false")] {
        let error = sf_sparql::exercise_private_parser_observation_corpus_for_evidence(path)
            .expect_err("non-normalized evidence path must fail closed");
        assert_eq!(
            error,
            "evidence executable path must be absolute and normalized"
        );
    }
}
