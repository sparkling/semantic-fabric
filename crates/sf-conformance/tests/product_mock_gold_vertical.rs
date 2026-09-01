#[path = "product_mock_gold/support.rs"]
mod support;

use std::path::PathBuf;

use serde_json::{json, Value};

fn replace_manifest(fixture: &mut support::SyntheticFixture, pointer: &str, value: Value) {
    let mut manifest: Value =
        serde_json::from_slice(&fixture.manifest).expect("synthetic manifest parses");
    *manifest
        .pointer_mut(pointer)
        .expect("synthetic manifest pointer exists") = value;
    fixture.manifest = serde_json::to_vec(&manifest).expect("synthetic manifest serializes");
    fixture.reseal_manifest();
}

fn replace_artifact(
    fixture: &mut support::SyntheticFixture,
    path: &str,
    pointer: &str,
    value: Value,
) {
    let mut artifact: Value = serde_json::from_slice(
        fixture
            .artifacts
            .get(path)
            .expect("synthetic artifact exists"),
    )
    .expect("synthetic artifact parses");
    *artifact
        .pointer_mut(pointer)
        .expect("synthetic artifact pointer exists") = value;
    fixture.artifacts.insert(
        path.to_owned(),
        serde_json::to_vec(&artifact).expect("synthetic artifact serializes"),
    );
    fixture.reseal_artifact(path);
}

#[test]
fn sealed_product_mock_gold_loader_accepts_a_valid_candidate() {
    let fixture = support::SyntheticFixture::valid();
    let admitted =
        support::admit_synthetic(&fixture).expect("valid synthetic gold candidate is admitted");
    assert_eq!(admitted.style.columns.len(), 5);
    assert_eq!(admitted.style.primary_key, ["style_number"]);
    assert_eq!(admitted.style.foreign_keys.len(), 2);
    assert!(admitted.r2rml.contains(" a rr:TriplesMap ;"));
    assert!(!admitted.r2rml.contains(" a rml:TriplesMap ;"));
    assert!(!admitted.r2rml.contains("rml:logicalSource"));
}

#[test]
fn exact_external_policy_pins_manifest_and_transitive_counts() {
    let policy = support::external_policy();
    assert_eq!(policy.manifest_bytes, 38_321);
    assert_eq!(
        policy.manifest_sha256,
        "sha256:edad6efab2406c021e85dd55a8d4354f5c115e3b6a5e3f6b7a86f75330f2b1cb"
    );
    assert_eq!(policy.artifact_count, 139);
    assert_eq!(policy.artifact_bytes, 41_845_098);
    assert_eq!(policy.snapshot_file_count, 171);
    assert_eq!(policy.snapshot_file_bytes, 1_037_818);
    assert_eq!(policy.source_pins.len(), 2);
    assert_eq!(policy.source_pins[0].bytes, 6_712);
    assert_eq!(policy.source_pins[1].bytes, 2_821);
}

#[test]
fn pure_mutants_cannot_change_outer_manifest_authority_or_counts() {
    let mut changed_byte = support::SyntheticFixture::valid();
    changed_byte.manifest[0] ^= 1;
    assert_eq!(
        support::admit_synthetic(&changed_byte),
        Err("candidate manifest seal mismatch")
    );

    let mut revision = support::SyntheticFixture::valid();
    replace_manifest(&mut revision, "/source/pinnedRevision", json!("mutable"));
    assert_eq!(
        support::admit_synthetic(&revision),
        Err("sealed JSON string claim mismatch")
    );

    let mut count = support::SyntheticFixture::valid();
    count.policy.artifact_count += 1;
    assert_eq!(
        support::admit_synthetic(&count),
        Err("transitive artifact count mismatch")
    );

    let mut total = support::SyntheticFixture::valid();
    total.policy.artifact_bytes += 1;
    assert_eq!(
        support::admit_synthetic(&total),
        Err("transitive artifact byte count mismatch")
    );
}

#[test]
fn pure_mutants_cannot_change_transitive_or_source_bytes() {
    let mut artifact = support::SyntheticFixture::valid();
    artifact
        .artifacts
        .get_mut(support::COVERAGE_PATH)
        .expect("coverage exists")[0] ^= 1;
    assert_eq!(
        support::admit_synthetic(&artifact),
        Err("transitive artifact seal mismatch")
    );

    let mut source = support::SyntheticFixture::valid();
    source
        .sources
        .get_mut(support::INITIAL_MIGRATION)
        .expect("source exists")[0] ^= 1;
    assert_eq!(
        support::admit_synthetic(&source),
        Err("source snapshot file seal mismatch")
    );

    let mut unpinned_source = support::SyntheticFixture::valid();
    unpinned_source
        .sources
        .get_mut("src/unpinned.rs")
        .expect("unpinned source exists")[0] ^= 1;
    assert_eq!(
        support::admit_synthetic(&unpinned_source),
        Err("source snapshot file seal mismatch")
    );

    let mut snapshot_count = support::SyntheticFixture::valid();
    snapshot_count.policy.snapshot_file_count += 1;
    assert_eq!(
        support::admit_synthetic(&snapshot_count),
        Err("source snapshot file count mismatch")
    );
}

#[test]
fn pure_coherently_resealed_claim_mutants_still_fail_structural_oracles() {
    let mut table_gap = support::SyntheticFixture::valid();
    replace_artifact(
        &mut table_gap,
        support::COVERAGE_PATH,
        "/relationalR2rml/unmappedTables",
        json!(110),
    );
    assert_eq!(
        support::admit_synthetic(&table_gap),
        Err("sealed JSON integer claim mismatch")
    );

    let mut column_gap = support::SyntheticFixture::valid();
    replace_artifact(
        &mut column_gap,
        support::COVERAGE_PATH,
        "/relationalR2rml/unmappedColumns",
        json!(595),
    );
    assert_eq!(
        support::admit_synthetic(&column_gap),
        Err("sealed JSON integer claim mismatch")
    );

    let mut column_order = support::SyntheticFixture::valid();
    replace_artifact(
        &mut column_order,
        support::COVERAGE_PATH,
        "/relationalSchema/stores/0/relations/0/columns/0/name",
        json!("season_code"),
    );
    assert_eq!(
        support::admit_synthetic(&column_order),
        Err("Style column contract mismatch")
    );
}

#[test]
fn only_the_r2rml_prefix_before_the_first_rml_map_is_accepted() {
    let mut fixture = support::SyntheticFixture::valid();
    let full = std::str::from_utf8(
        fixture
            .artifacts
            .get(support::MAPPING_PATH)
            .expect("mapping exists"),
    )
    .expect("mapping UTF-8");
    let unbounded_parse = sf_mapping::parse_r2rml(full).expect("RML entries are ignored");
    assert_eq!(unbounded_parse.len(), 1);
    assert!(!unbounded_parse[0].id.contains("first-rml-map"));
    let extracted = support::extract_r2rml(full).expect("bounded R2RML prefix extracts");
    assert_eq!(sf_mapping::parse_r2rml(extracted).unwrap().len(), 1);

    let poisoned = full.replacen(" a rr:TriplesMap ;", " a rml:TriplesMap ;", 1);
    fixture
        .artifacts
        .insert(support::MAPPING_PATH.to_owned(), poisoned.into_bytes());
    fixture.reseal_artifact(support::MAPPING_PATH);
    assert_eq!(
        support::admit_synthetic(&fixture),
        Err("R2RML extraction boundary mismatch")
    );
}

#[test]
fn traversal_and_duplicate_artifact_descriptors_are_rejected_before_reads() {
    let mut traversal = support::SyntheticFixture::valid();
    replace_manifest(
        &mut traversal,
        "/artifactFiles/0/path",
        json!("../candidate-manifest.json"),
    );
    assert_eq!(
        support::admit_synthetic(&traversal),
        Err("transitive artifact descriptor is invalid")
    );

    let mut duplicate = support::SyntheticFixture::valid();
    let mut manifest: Value = serde_json::from_slice(&duplicate.manifest).unwrap();
    manifest["artifactFiles"][1]["path"] = manifest["artifactFiles"][0]["path"].clone();
    duplicate.manifest = serde_json::to_vec(&manifest).unwrap();
    duplicate.reseal_manifest();
    assert_eq!(
        support::admit_synthetic(&duplicate),
        Err("transitive artifact descriptor is invalid")
    );
}

#[test]
#[ignore = "requires exact external semantic-product-mock gold and source roots"]
fn exact_external_product_mock_gold_and_source_are_admitted() {
    let gold = PathBuf::from(
        std::env::var_os(support::GOLD_ROOT_ENV).expect("SF_PRODUCT_MOCK_GOLD_ROOT is required"),
    );
    let source = PathBuf::from(
        std::env::var_os(support::SOURCE_ROOT_ENV)
            .expect("SF_PRODUCT_MOCK_SOURCE_ROOT is required"),
    );
    let admitted = support::load_external(&gold, &source).expect("external seals must match");
    assert_eq!(admitted.style.columns.len(), 5);
    assert_eq!(admitted.style.primary_key, ["style_number"]);
    assert_eq!(admitted.style.foreign_keys.len(), 2);
    assert_eq!(sf_mapping::parse_r2rml(&admitted.r2rml).unwrap().len(), 1);
}
