use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sf_conformance::capability_catalog::{self, Status};
use sf_conformance::capability_model::CommandMode;
use sf_conformance::capability_render;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn catalog_bytes() -> Vec<u8> {
    fs::read(root().join(capability_catalog::CATALOG_PATH)).expect("read catalog")
}

fn mutated(mutator: impl FnOnce(&mut Value)) -> Result<capability_catalog::Catalog, String> {
    let mut value: Value = serde_json::from_slice(&catalog_bytes()).expect("parse mutation source");
    mutator(&mut value);
    capability_catalog::parse_and_validate(
        &root(),
        &serde_json::to_vec(&value).expect("serialize mutation"),
    )
}

fn array<'a>(value: &'a mut Value, key: &str) -> &'a mut Vec<Value> {
    value[key].as_array_mut().expect("catalog array")
}

fn by_id<'a>(values: &'a mut [Value], id: &str) -> &'a mut Value {
    values
        .iter_mut()
        .find(|value| value["id"] == id)
        .expect("catalog id")
}

#[test]
fn tracked_catalog_is_strict_evidence_bound_and_has_zero_admissions() {
    let loaded = capability_catalog::load(&root()).expect("load tracked catalog");
    let counts = capability_catalog::status_counts(&loaded.catalog);
    assert_eq!(loaded.catalog.cells.len(), 74);
    assert_eq!(counts.get(&Status::Admitted).copied().unwrap_or(0), 0);
    assert_eq!(counts.get(&Status::Implemented), Some(&38));
    assert_eq!(counts.get(&Status::Planned), Some(&34));
    assert_eq!(counts.get(&Status::Unsupported), Some(&2));
    assert!(loaded
        .catalog
        .standards
        .iter()
        .all(|standard| standard.url.contains("/TR/") && standard.byte_length > 0));
}

#[test]
fn sqlite_regression_receipt_commands_are_canonical_and_required() {
    let loaded = capability_catalog::load(&root()).expect("load tracked catalog");
    for (id, argv) in [
        (
            "cmd-protocol-regression-sqlite",
            "cargo run --locked --offline -p sf-conformance --features evidence-receipts \
             --bin sparql-protocol-regression-baseline -- --check",
        ),
        (
            "cmd-query-regression-sqlite",
            "cargo run --locked --offline -p sf-conformance --features evidence-receipts \
             --bin sparql-query-regression-baseline -- --check",
        ),
    ] {
        let command = loaded
            .catalog
            .commands
            .iter()
            .find(|command| command.id == id)
            .unwrap_or_else(|| panic!("missing {id}"));
        assert_eq!(command.argv, argv);
        assert_eq!(command.mode, CommandMode::Required);
    }
}

#[test]
fn generated_json_markdown_and_readme_are_exact() {
    let repository = root();
    let loaded = capability_catalog::load(&repository).expect("load catalog");
    let readme =
        fs::read_to_string(repository.join(capability_render::README_PATH)).expect("read README");
    let expected = capability_render::render(&loaded, &readme).expect("render artifacts");
    assert_eq!(
        fs::read(repository.join(capability_render::GENERATED_JSON_PATH)).unwrap(),
        expected.json.as_bytes()
    );
    assert_eq!(
        fs::read(repository.join(capability_render::GENERATED_MARKDOWN_PATH)).unwrap(),
        expected.markdown.as_bytes()
    );
    assert_eq!(readme.as_bytes(), expected.readme.as_bytes());
}

#[test]
fn production_check_path_is_read_only() {
    let repository = root();
    let paths = [
        capability_render::GENERATED_JSON_PATH,
        capability_render::GENERATED_MARKDOWN_PATH,
        capability_render::README_PATH,
    ];
    let before: Vec<_> = paths
        .iter()
        .map(|path| fs::read(repository.join(path)).expect("read before"))
        .collect();
    let output = Command::new(env!("CARGO_BIN_EXE_capability-matrix"))
        .arg("--check")
        .output()
        .expect("execute checker");
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let after: Vec<_> = paths
        .iter()
        .map(|path| fs::read(repository.join(path)).expect("read after"))
        .collect();
    assert_eq!(before, after);
}

#[test]
fn unknown_fields_and_missing_cross_product_cells_fail_closed() {
    let unknown = mutated(|value| {
        value
            .as_object_mut()
            .unwrap()
            .insert("unknown".to_owned(), Value::Bool(true));
    })
    .unwrap_err();
    assert!(unknown.contains("unknown field"), "{unknown}");

    let missing = mutated(|value| {
        array(value, "cells").retain(|cell| cell["id"] != "ask-execution-sqlite");
    })
    .unwrap_err();
    assert!(missing.contains("cross-product"), "{missing}");
}

#[test]
fn evidence_drift_and_non_normalized_paths_fail_closed() {
    let digest = mutated(|value| {
        by_id(array(value, "evidence"), "e-inventory")["sha256"] = Value::String("0".repeat(64));
    })
    .unwrap_err();
    assert!(digest.contains("digest mismatch"), "{digest}");

    let path = mutated(|value| {
        by_id(array(value, "evidence"), "e-inventory")["path"] =
            Value::String("tests/capabilities/../w3c/rdb2rdf/inventory.tsv".to_owned());
    })
    .unwrap_err();
    assert!(path.contains("not normalized"), "{path}");
}

#[test]
fn mapping_evidence_cannot_promote_query_or_protocol_cells() {
    let error = mutated(|value| {
        let cell = by_id(array(value, "cells"), "ask-execution-sqlite");
        cell["evidenceIds"] = serde_json::json!(["e-receipt-sqlite"]);
        cell["verification"] = Value::String("receipt".to_owned());
    })
    .unwrap_err();
    assert!(error.contains("promotes mapping evidence"), "{error}");
}

#[test]
fn production_admission_and_public_claims_cannot_self_promote() {
    let admission = mutated(|value| {
        let cell = by_id(array(value, "cells"), "production-source-admission-sqlite");
        cell["status"] = Value::String("admitted".to_owned());
        cell["verification"] = Value::String("receipt".to_owned());
        cell["semanticExact"] = Value::Bool(true);
        cell["bounded"] = Value::Bool(true);
        cell["advertisable"] = Value::Bool(true);
    })
    .unwrap_err();
    assert!(admission.contains("production law"), "{admission}");

    let claim = mutated(|value| {
        by_id(array(value, "claims"), "claim-compiler-describe")["cellIds"] =
            serde_json::json!(["runtime-source-path-mysql"]);
    })
    .unwrap_err();
    assert!(claim.contains("non-advertisable"), "{claim}");
}

#[test]
fn forbidden_unqualified_manual_readme_claim_fails_generation() {
    let loaded = capability_catalog::load(&root()).expect("load catalog");
    let readme = fs::read_to_string(root().join(capability_render::README_PATH)).unwrap();
    let poisoned = format!(
        "{readme}\nThe public serving path currently admits **SQLite, PostgreSQL, and MySQL**.\n"
    );
    let error = capability_render::render(&loaded, &poisoned).unwrap_err();
    assert!(error.contains("forbidden unqualified"), "{error}");
}

#[test]
fn evidence_paths_resolve_inside_repository() {
    let loaded = capability_catalog::load(&root()).expect("load catalog");
    for evidence in &loaded.catalog.evidence {
        let path = Path::new(&evidence.path);
        assert!(!path.is_absolute());
        assert!(root().join(path).is_file(), "missing {}", evidence.path);
    }
}
