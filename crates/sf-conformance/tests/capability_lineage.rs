//! The incremental provenance profile never substitutes for complete ADR-0017.
use sf_conformance::{
    capability_catalog, capability_catalog::Status, capability_model::Verification,
};
use std::path::Path;

#[test]
fn lineage_keeps_full_contract_open_and_only_advertises_the_evidenced_subset() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let catalog = capability_catalog::load(&root).unwrap().catalog;
    for backend in ["mysql", "postgresql", "sqlite"] {
        let partial = catalog
            .cells
            .iter()
            .find(|cell| cell.id == format!("constant-mapping-source-lineage-{backend}"))
            .unwrap();
        assert_eq!(partial.status, Status::Implemented);
        assert_eq!(partial.verification, Verification::CiRequired);
        assert!(partial.semantic_exact && partial.bounded && partial.advertisable);
        for id in [
            "e-query-lineage-graph",
            "e-query-lineage-graph-http",
            "e-query-lineage-graph-stream",
        ] {
            assert!(partial.evidence_ids.iter().any(|evidence| evidence == id));
        }
        if backend != "sqlite" {
            let live = catalog
                .evidence
                .iter()
                .find(|e| e.id == "e-query-lineage-native")
                .unwrap();
            assert!(partial.evidence_ids.contains(&live.id));
            assert!(live.required);
            assert_eq!(
                live.command_id.as_deref(),
                Some("cmd-verified-source-tls-live")
            );
        }
    }
    let full = catalog
        .cells
        .iter()
        .find(|cell| cell.id == "query-lineage-generic")
        .unwrap();
    assert_eq!(full.status, Status::Planned);
    assert!(!full.advertisable);
    assert!(full.qualification.contains("conditional"));
    assert!(
        catalog
            .limitations
            .iter()
            .find(|limit| limit.id == "l-lineage")
            .unwrap()
            .release_blocking
    );
}

#[test]
fn actual_multi_mapping_profiles_bind_required_native_and_bounded_executor_evidence() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let catalog = capability_catalog::load(&root).unwrap().catalog;
    for backend in ["mysql", "postgresql", "sqlite"] {
        let cell = catalog
            .cells
            .iter()
            .find(|cell| cell.id == format!("bounded-mapping-source-lineage-{backend}"))
            .unwrap();
        assert_eq!(cell.status, Status::Implemented);
        assert_eq!(cell.verification, Verification::CiRequired);
        assert!(cell.semantic_exact && cell.bounded && cell.advertisable);
        assert!(cell.limitation_ids.iter().any(|id| id == "l-lineage"));
        for id in [
            "e-query-lineage-multiple-compiler",
            "e-query-lineage-multiple-executor",
            "e-query-lineage-multiple-http",
            "e-query-lineage-multiple-stream",
        ] {
            assert!(cell.evidence_ids.iter().any(|entry| entry == id));
        }
        if backend != "sqlite" {
            let evidence = catalog
                .evidence
                .iter()
                .find(|e| e.id == "e-query-lineage-multiple-native")
                .unwrap();
            assert!(cell.evidence_ids.contains(&evidence.id));
            assert!(evidence.required);
            assert_eq!(
                evidence.command_id.as_deref(),
                Some("cmd-verified-source-tls-live")
            );
            for id in [
                "e-query-lineage-multiple-native-stop",
                "e-query-lineage-multiple-native-stop-oracle",
            ] {
                let proof = catalog.evidence.iter().find(|e| e.id == id).unwrap();
                assert!(cell.evidence_ids.contains(&proof.id));
                assert!(proof.required);
                assert_eq!(proof.verification, Verification::CiRequired);
            }
        }
    }
}

#[test]
fn accepted_release_contract_excludes_research_and_no_prefix_delivery_not_required_checks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let catalog = capability_catalog::load(&root).unwrap().catalog;
    for id in ["l-global-operators", "l-stream-error-atomicity"] {
        let limit = catalog
            .limitations
            .iter()
            .find(|limit| limit.id == id)
            .unwrap();
        assert!(!limit.release_blocking);
        assert_eq!(limit.adr, "ADR-0055");
    }
    for id in [
        "l-query-budget",
        "l-authz",
        "l-production-admission",
        "l-release-artifact",
    ] {
        assert!(
            catalog
                .limitations
                .iter()
                .find(|limit| limit.id == id)
                .unwrap()
                .release_blocking
        );
    }
}

#[test]
fn federated_lineage_binds_actual_public_origins_and_native_stop_evidence() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let catalog = capability_catalog::load(&root).unwrap().catalog;
    let cell = catalog
        .cells
        .iter()
        .find(|c| c.id == "bounded-federated-union-lineage-multi-source")
        .unwrap();
    assert_eq!(cell.status, Status::Implemented);
    assert!(cell.semantic_exact && cell.bounded && cell.advertisable);
    assert!(cell.limitation_ids.iter().any(|id| id == "l-lineage"));
    for (id, command) in [
        (
            "e-query-lineage-federated-http",
            "cmd-federated-union-serve",
        ),
        (
            "e-query-lineage-federated-control",
            "cmd-federated-union-serve",
        ),
        (
            "e-query-lineage-federated-native",
            "cmd-verified-source-tls-live",
        ),
        (
            "e-query-lineage-federated-native-stop",
            "cmd-verified-source-tls-live",
        ),
        (
            "e-query-lineage-inverse-identity",
            "cmd-query-lineage-inverse",
        ),
    ] {
        let proof = catalog.evidence.iter().find(|e| e.id == id).unwrap();
        assert!(cell.evidence_ids.contains(&proof.id));
        assert!(proof.required);
        assert_eq!(proof.verification, Verification::CiRequired);
        assert_eq!(proof.command_id.as_deref(), Some(command));
    }
    assert!(
        !catalog
            .cells
            .iter()
            .find(|c| c.id == "query-lineage-generic")
            .unwrap()
            .advertisable
    );
}
