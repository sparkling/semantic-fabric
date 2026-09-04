#![cfg(all(
    feature = "parser-worker-evidence",
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
))]

use std::path::Path;

const BINARY: &str = env!("CARGO_BIN_EXE_semantic-fabric");

#[test]
fn held_binary_reaches_control_ready_only_after_candidate_confinement() {
    sf_sparql::exercise_private_parser_worker_handshake_for_evidence(Path::new(BINARY), "ASK {}")
        .expect("complete held-binary Hello/Ready/EOF exchange");
}

#[test]
fn evidence_seam_rejects_relative_and_parent_traversal_paths() {
    for path in [Path::new("semantic-fabric"), Path::new("/tmp/../bin/false")] {
        let error = sf_sparql::exercise_private_parser_worker_handshake_for_evidence(
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

#[test]
fn evidence_prepares_exact_source_boundaries_without_sending_a_request() {
    let full_source = "#".repeat(1 << 20);
    sf_sparql::exercise_private_parser_worker_handshake_for_evidence(
        Path::new(BINARY),
        &full_source,
    )
    .expect("the complete one-MiB source is prepared before the control-only launch");

    let error = sf_sparql::exercise_private_parser_worker_handshake_for_evidence(
        Path::new(BINARY),
        &(full_source + "X"),
    )
    .expect_err("a source above one MiB must fail before launch");
    assert_eq!(
        error,
        "parser request preparation: parser protocol source limit exceeded"
    );
}
