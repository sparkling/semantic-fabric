#[path = "product_mock_gold/support.rs"]
mod support;

use std::path::PathBuf;
use std::time::Instant;

use serde_json::{json, Value};
use sf_core::SourceId;

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

fn replace_and_reseal_category_shard(
    fixture: &mut support::SyntheticFixture,
    path: &str,
    from: &str,
    to: &str,
) {
    let source = fixture.artifacts.get(path).expect("synthetic shard exists");
    let source = std::str::from_utf8(source).expect("synthetic shard is UTF-8");
    let changed = source.replacen(from, to, 1);
    assert_ne!(changed, source, "synthetic shard mutation must apply");
    fixture
        .artifacts
        .insert(path.to_owned(), changed.into_bytes());

    let shard = fixture.artifacts.get(path).unwrap();
    let mut category: Value = serde_json::from_slice(
        fixture
            .artifacts
            .get(support::CATEGORY_MAPPING_PATH)
            .expect("Category 13 manifest exists"),
    )
    .unwrap();
    let descriptor = category["shards"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["path"].as_str() == Some(path))
        .expect("synthetic shard descriptor exists");
    descriptor["bytes"] = json!(shard.len());
    descriptor["lines"] =
        json!(shard.iter().filter(|byte| **byte == b'\n').count() + usize::from(!shard.is_empty()));
    descriptor["digest"] = json!(support::sha256(shard));
    fixture.artifacts.insert(
        support::CATEGORY_MAPPING_PATH.to_owned(),
        serde_json::to_vec(&category).unwrap(),
    );
    fixture.reseal_artifact(path);
    fixture.reseal_artifact(support::CATEGORY_MAPPING_PATH);
}

#[test]
fn sealed_product_mock_gold_loader_accepts_a_valid_candidate() {
    let fixture = support::SyntheticFixture::valid();
    let admitted =
        support::admit_synthetic(&fixture).expect("valid synthetic gold candidate is admitted");
    assert_eq!(admitted.style.columns.len(), 5);
    assert_eq!(admitted.style.primary_key, ["style_number"]);
    assert_eq!(admitted.style.foreign_keys.len(), 2);
    sf_serve::SemanticOntology::from_turtle(&admitted.ontology_turtle)
        .expect("assembled semantic document parses as Turtle");
    let mut previous = None;
    for included in [
        "01", "02", "03", "04", "05", "06", "07", "08", "09", "10", "11", "12", "14",
    ] {
        let position = admitted
            .ontology_turtle
            .find(&format!("ontology/category/{included}"))
            .expect("included category marker is retained");
        assert!(previous.is_none_or(|earlier| earlier < position));
        previous = Some(position);
    }
    assert!(!admitted
        .ontology_turtle
        .contains("mapping/category-13/triples-map"));
    assert!(!admitted
        .ontology_turtle
        .contains("http://www.w3.org/ns/r2rml#TriplesMap"));
    assert!(!admitted.ontology_turtle.contains("http://w3id.org/rml/"));
    let mappings = sf_mapping::parse_r2rml(&admitted.r2rml).unwrap();
    assert_eq!(mappings.len(), 148);
    assert_eq!(
        mappings
            .iter()
            .map(|mapping| mapping.predicate_object_maps.len())
            .sum::<usize>(),
        721
    );
    assert!(!admitted.r2rml.contains("http://w3id.org/rml/"));
}

#[test]
fn canonical_ontology_shard_limit_rejects_before_artifact_read() {
    let shard_path = "categories/01-domain-structure/part-001.ttl";
    let mut fixture = support::SyntheticFixture::valid();
    let mut manifest: Value = serde_json::from_slice(&fixture.manifest).unwrap();
    let descriptor = manifest["artifactFiles"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["path"].as_str() == Some(shard_path))
        .expect("synthetic root shard descriptor exists");
    descriptor["bytes"] = json!(64 * 1024 + 1);
    fixture.manifest = serde_json::to_vec(&manifest).unwrap();
    fixture.reseal_manifest();
    fixture.artifacts.remove(shard_path);

    assert_eq!(
        support::admit_synthetic(&fixture),
        Err("canonical ontology shard exceeds its byte limit")
    );
}

#[test]
fn canonical_ontology_requires_its_category_seals_and_manifest_order() {
    let shard_path = "categories/01-domain-structure/part-001.ttl";
    let mut stale_category_seal = support::SyntheticFixture::valid();
    stale_category_seal
        .artifacts
        .get_mut(shard_path)
        .expect("synthetic ontology shard exists")
        .push(b' ');
    stale_category_seal.reseal_artifact(shard_path);
    assert_eq!(
        support::admit_synthetic(&stale_category_seal),
        Err("canonical ontology shard seal mismatch")
    );

    let mut reordered = support::SyntheticFixture::valid();
    replace_artifact(
        &mut reordered,
        "categories/01-domain-structure/category.json",
        "/shards/0/path",
        json!("categories/01-domain-structure/part-002.ttl"),
    );
    assert_eq!(
        support::admit_synthetic(&reordered),
        Err("canonical ontology shard path or order mismatch")
    );
}

#[test]
fn exact_external_policy_pins_manifest_and_transitive_counts() {
    let policy = support::external_policy();
    assert_eq!(policy.manifest_bytes, 63_091);
    assert_eq!(
        policy.manifest_sha256,
        "sha256:bf43a09b9eb952e9708044838c16684351eb85e1049b5dc4a599ce7884d51f5d"
    );
    assert_eq!(policy.artifact_count, 246);
    assert_eq!(policy.artifact_bytes, 69_062_740);
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
        json!(1),
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
        json!(1),
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

    let mut shard_lines = support::SyntheticFixture::valid();
    replace_artifact(
        &mut shard_lines,
        support::CATEGORY_MAPPING_PATH,
        "/shards/0/lines",
        json!(0),
    );
    assert_eq!(
        support::admit_synthetic(&shard_lines),
        Err("source-mapping shard seal mismatch")
    );
}

#[test]
fn rdf_union_extracts_only_the_exact_relational_r2rml_closure() {
    let mut fixture = support::SyntheticFixture::valid();
    let category = fixture
        .artifacts
        .get(support::CATEGORY_MAPPING_PATH)
        .expect("Category 13 manifest exists")
        .clone();
    let shards = fixture
        .artifacts
        .iter()
        .filter(|(path, _)| path.starts_with(support::CATEGORY_SHARD_PREFIX))
        .map(|(path, bytes)| (path.clone(), bytes.clone()))
        .collect();
    let raw_union = fixture
        .artifacts
        .iter()
        .filter(|(path, _)| path.starts_with(support::CATEGORY_SHARD_PREFIX))
        .flat_map(|(_, bytes)| bytes.iter().copied())
        .collect::<Vec<_>>();
    let raw_union = std::str::from_utf8(&raw_union).expect("synthetic shard union is UTF-8");
    assert!(sf_mapping::parse_r2rml(raw_union).is_err());
    let extracted = support::extract_relational_r2rml(&category, &shards)
        .expect("sealed relational RDF closure extracts");
    let mappings = sf_mapping::parse_r2rml(&extracted).unwrap();
    assert_eq!(mappings.len(), 148);
    assert_eq!(
        mappings
            .iter()
            .map(|mapping| mapping.predicate_object_maps.len())
            .sum::<usize>(),
        721
    );
    assert!(!extracted.contains("http://w3id.org/rml/"));

    let shard_path = format!("{}001.ttl", support::CATEGORY_SHARD_PREFIX);
    fixture
        .artifacts
        .get_mut(&shard_path)
        .expect("synthetic shard exists")
        .push(b' ');
    fixture.reseal_artifact(&shard_path);
    assert_eq!(
        support::admit_synthetic(&fixture),
        Err("source-mapping shard seal mismatch")
    );
}

#[test]
fn coherently_resealed_root_or_rml_contamination_mutants_fail_closed() {
    let shard_path = format!("{}001.ttl", support::CATEGORY_SHARD_PREFIX);
    let mut root_count = support::SyntheticFixture::valid();
    replace_and_reseal_category_shard(
        &mut root_count,
        &shard_path,
        "http://www.w3.org/ns/r2rml#TriplesMap",
        "http://w3id.org/rml/TriplesMap",
    );
    assert_eq!(
        support::admit_synthetic(&root_count),
        Err("source-mapping triples-map root count mismatch")
    );

    let mut contamination = support::SyntheticFixture::valid();
    replace_and_reseal_category_shard(
        &mut contamination,
        &shard_path,
        "http://www.w3.org/ns/r2rml#class",
        "http://w3id.org/rml/class",
    );
    assert_eq!(
        support::admit_synthetic(&contamination),
        Err("relational R2RML closure reached an RML term")
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
    let started = Instant::now();
    assert_eq!(admitted.style.columns.len(), 5);
    assert_eq!(admitted.style.primary_key, ["style_number"]);
    assert_eq!(admitted.style.foreign_keys.len(), 2);
    let mapping = sf_mapping::parse_r2rml_for_source(
        &admitted.r2rml,
        SourceId::new(0).expect("fixed source id is valid"),
    )
    .unwrap();
    assert_eq!(mapping.len(), 148);
    assert_eq!(
        mapping
            .triples_maps()
            .iter()
            .map(|mapping| mapping.predicate_object_maps.len())
            .sum::<usize>(),
        721
    );
    let mut closure = sf_validation::parse_turtle_graph(
        &admitted.ontology_turtle,
        sf_validation::DEFAULT_GRAPH_LIMITS,
    )
    .expect("external canonical ontology parses");
    assert_eq!(closure.len(), 47_463);
    let projection = sf_mapping::project_static_to_rdf(&mapping).unwrap();
    assert_eq!(projection.len(), 3_064);
    for triple in projection.iter() {
        closure.insert(triple);
    }
    assert_eq!(closure.len(), 50_527);
    let validation_started = Instant::now();
    let outcome = sf_validation::validate_graph(&closure).unwrap();
    let validation_elapsed = validation_started.elapsed();
    assert_eq!(outcome.violations, 0);
    assert_eq!(outcome.warnings, 0);
    eprintln!(
        "exact static Product Mock M-join-T: total {:?}; validation {:?}",
        started.elapsed(),
        validation_elapsed
    );
    assert!(!admitted
        .ontology_turtle
        .contains("mapping/category-13/triples-map"));
}
