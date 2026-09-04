#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::path::Path;

const BINARY: &str = env!("CARGO_BIN_EXE_semantic-fabric");

#[test]
fn held_binary_reaches_control_ready_only_after_candidate_confinement() {
    sf_sparql::exercise_private_parser_worker_handshake_for_evidence(Path::new(BINARY))
        .expect("complete held-binary Hello/Ready/EOF exchange");
}

#[test]
fn evidence_seam_rejects_relative_and_parent_traversal_paths() {
    for path in [Path::new("semantic-fabric"), Path::new("/tmp/../bin/false")] {
        let error = sf_sparql::exercise_private_parser_worker_handshake_for_evidence(path)
            .expect_err("non-normalized evidence path must fail closed");
        assert_eq!(
            error,
            "evidence executable path must be absolute and normalized"
        );
    }
}
