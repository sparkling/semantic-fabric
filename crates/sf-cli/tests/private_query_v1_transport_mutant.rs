#![cfg(all(
    feature = "query-v1-transport-mutant-evidence",
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
))]

use std::path::Path;

use sf_sparql::QueryV1TransportMutant;

const BINARY: &str = env!("CARGO_BIN_EXE_semantic-fabric");

const MUTANTS: [QueryV1TransportMutant; 10] = [
    QueryV1TransportMutant::WrongNonceExitZero,
    QueryV1TransportMutant::WrongSourceDigestExitZero,
    QueryV1TransportMutant::WrongPayloadDigestExitZero,
    QueryV1TransportMutant::SelfConsistentDigestOverInvalidQueryV1ExitZero,
    QueryV1TransportMutant::WrongCorrelationThenExit78,
    QueryV1TransportMutant::WrongCorrelationThenDeadlineStall,
    QueryV1TransportMutant::TrailingOutput,
    QueryV1TransportMutant::ExactOutputCap,
    QueryV1TransportMutant::OutputCapPlusOne,
    QueryV1TransportMutant::RequestFrameAllocationRefusal,
];

#[test]
fn held_binary_exhibits_each_exact_mutant_then_allows_a_clean_normal_launch() {
    for mutant in MUTANTS {
        sf_sparql::exercise_synthetic_query_v1_transport_mutant_for_evidence(
            Path::new(BINARY),
            "unparsed mutant evidence source",
            mutant,
        )
        .expect("observe the mutant's exact contained outcome");

        #[cfg(feature = "query-v1-transport-evidence")]
        sf_sparql::exercise_synthetic_query_v1_transport_for_evidence(
            Path::new(BINARY),
            "clean next normal launch",
        )
        .expect("a contained mutant cannot poison the next normal launch");
    }
}

#[test]
fn mutant_transport_accepts_its_exact_source_cap_and_rejects_cap_plus_one() {
    let exact_source = "#".repeat((1 << 20) - 2);
    sf_sparql::exercise_synthetic_query_v1_transport_mutant_for_evidence(
        Path::new(BINARY),
        &exact_source,
        QueryV1TransportMutant::WrongNonceExitZero,
    )
    .expect("transport the exact mutant source ceiling");

    let error = sf_sparql::exercise_synthetic_query_v1_transport_mutant_for_evidence(
        Path::new(BINARY),
        &"X".repeat((1 << 20) - 1),
        QueryV1TransportMutant::WrongNonceExitZero,
    )
    .expect_err("two directive bytes reduce the mutant source ceiling by exactly two");
    assert_eq!(
        error,
        "parser request preparation: parser protocol source limit exceeded"
    );
}

#[test]
fn corrupt_requests_remain_alive_and_silent_until_exact_eof() {
    sf_sparql::exercise_synthetic_query_v1_request_eof_order_for_evidence(Path::new(BINARY))
        .expect("nonce, source-digest, and UTF-8 validation must be strictly post-EOF");
}

#[test]
fn malformed_directives_exit_silently_then_allow_a_clean_launch() {
    sf_sparql::exercise_synthetic_query_v1_malformed_directives_for_evidence(Path::new(BINARY))
        .expect("short, long, and unknown directives must be silent, reaped rejections");
}

#[test]
fn mutant_evidence_rejects_paths_before_preparing_an_oversized_request() {
    for path in [Path::new("semantic-fabric"), Path::new("/tmp/../bin/false")] {
        let error = sf_sparql::exercise_synthetic_query_v1_transport_mutant_for_evidence(
            path,
            &"X".repeat((1 << 20) - 1),
            QueryV1TransportMutant::WrongNonceExitZero,
        )
        .expect_err("non-normalized evidence paths must fail closed first");
        assert_eq!(
            error,
            "evidence executable path must be absolute and normalized"
        );
    }
}
