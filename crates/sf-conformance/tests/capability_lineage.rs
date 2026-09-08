//! The incremental provenance profile never substitutes for complete ADR-0017.
use sf_conformance::{capability_catalog, capability_catalog::Status};
use std::path::Path;

#[test]
fn lineage_keeps_full_contract_open_and_only_advertises_the_evidenced_subset() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let catalog = capability_catalog::load(&root).unwrap().catalog;
    let partial = catalog
        .cells
        .iter()
        .find(|cell| cell.id == "constant-mapping-source-lineage-sqlite")
        .unwrap();
    assert_eq!(partial.status, Status::Implemented);
    assert!(partial.semantic_exact && partial.bounded && partial.advertisable);
    for id in [
        "e-query-lineage-graph",
        "e-query-lineage-graph-http",
        "e-query-lineage-graph-stream",
    ] {
        assert!(partial.evidence_ids.iter().any(|evidence| evidence == id));
    }
    let full = catalog
        .cells
        .iter()
        .find(|cell| cell.id == "query-lineage-generic")
        .unwrap();
    assert_eq!(full.status, Status::Planned);
    assert!(!full.advertisable);
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
