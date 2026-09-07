use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sf_conformance::capability_catalog::{self, Status};
use sf_conformance::capability_model::{CommandMode, Verification};
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
    assert_eq!(loaded.catalog.cells.len(), 101);
    assert_eq!(counts.get(&Status::Admitted).copied().unwrap_or(0), 0);
    assert_eq!(counts.get(&Status::Implemented), Some(&70));
    assert_eq!(counts.get(&Status::Planned), Some(&28));
    assert_eq!(counts.get(&Status::Unsupported), Some(&3));
    assert!(loaded
        .catalog
        .standards
        .iter()
        .all(|standard| standard.url.contains("/TR/") && standard.byte_length > 0));
}

#[test]
fn bounded_slices_do_not_promote_broad_programme_profiles() {
    let loaded = capability_catalog::load(&root()).expect("load tracked catalog");
    for id in [
        "bounded-graceful-shutdown-generic",
        "describe-execution-sqlite",
        "federated-two-source-union-multi-source",
        "generated-qe-per-pr-sqlite",
        "health-readiness-probes-generic",
        "mapping-ontology-semantic-admission-generic",
        "service-description-discovery-generic",
        "verified-source-tls-generic",
    ] {
        let cell = loaded
            .catalog
            .cells
            .iter()
            .find(|cell| cell.id == id)
            .unwrap_or_else(|| panic!("missing {id}"));
        assert_eq!(cell.status, Status::Implemented);
        assert_eq!(cell.verification, Verification::CiRequired);
    }
    for id in ["federation-multi-source", "observability-lifecycle-generic"] {
        let cell = loaded
            .catalog
            .cells
            .iter()
            .find(|cell| cell.id == id)
            .unwrap_or_else(|| panic!("missing {id}"));
        assert_eq!(cell.status, Status::Planned);
        assert!(!cell.advertisable);
    }
}

#[test]
fn capture_supervisor_kernel_does_not_promote_operational_authority() {
    let loaded = capability_catalog::load(&root()).expect("load tracked catalog");
    let cell = loaded
        .catalog
        .cells
        .iter()
        .find(|cell| cell.id == "capture-supervisor-authority-kernel-generic")
        .expect("capture supervisor kernel cell");
    assert_eq!(cell.status, Status::Implemented);
    assert_eq!(cell.verification, Verification::CiRequired);
    assert!(cell.semantic_exact && cell.bounded && !cell.advertisable);
    assert_eq!(
        cell.limitation_ids,
        ["l-capture-supervisor-operational-authority"]
    );
    for id in [
        "e-capture-supervisor-postgresql-contention",
        "e-capture-supervisor-postgresql-differential",
    ] {
        let evidence = loaded
            .catalog
            .evidence
            .iter()
            .find(|item| item.id == id)
            .unwrap();
        assert_eq!(evidence.verification, Verification::LiveOptional);
        assert!(!evidence.required);
    }
}

#[test]
fn describe_profile_is_exact_versioned_and_backend_scoped() {
    let loaded = capability_catalog::load(&root()).expect("load tracked catalog");
    let sqlite = loaded
        .catalog
        .cells
        .iter()
        .find(|cell| cell.id == "describe-execution-sqlite")
        .expect("SQLite DESCRIBE cell");
    assert_eq!(sqlite.status, Status::Implemented);
    assert_eq!(sqlite.verification, Verification::CiRequired);
    assert!(sqlite.semantic_exact);
    assert!(sqlite.bounded);
    assert!(sqlite.advertisable);
    assert_eq!(
        sqlite.evidence_ids,
        [
            "e-describe-compile",
            "e-describe-sqlite",
            "e-describe-variable-inventory",
            "e-describe-wiring",
            "e-query-budget-handler",
            "e-resource-admission",
            "e-resource-profile",
        ]
    );
    assert!(sqlite
        .qualification
        .contains("one parsed target expression"));
    assert!(sqlite.qualification.contains("RDF-graph set union"));
    assert!(sqlite.qualification.contains("retained executor state"));

    for id in ["describe-execution-mysql", "describe-execution-postgresql"] {
        let cell = loaded
            .catalog
            .cells
            .iter()
            .find(|cell| cell.id == id)
            .unwrap_or_else(|| panic!("missing {id}"));
        assert_eq!(cell.status, Status::Planned);
        assert!(!cell.semantic_exact);
        assert!(!cell.bounded);
        assert!(!cell.advertisable);
    }

    let limitation = loaded
        .catalog
        .limitations
        .iter()
        .find(|limitation| limitation.id == "l-describe")
        .expect("DESCRIBE limitation");
    assert!(limitation.release_blocking);

    let compiler_claim = loaded
        .catalog
        .claims
        .iter()
        .find(|claim| claim.id == "claim-compiler-describe")
        .expect("compiler DESCRIBE claim");
    assert_eq!(compiler_claim.cell_ids, ["describe-compilation-compiler"]);
    let runtime_claim = loaded
        .catalog
        .claims
        .iter()
        .find(|claim| claim.id == "claim-sqlite-describe")
        .expect("SQLite DESCRIBE claim");
    assert_eq!(runtime_claim.cell_ids, ["describe-execution-sqlite"]);
    let discovery_claim = loaded
        .catalog
        .claims
        .iter()
        .find(|claim| claim.id == "claim-service-description")
        .expect("Service Description claim");
    assert!(discovery_claim
        .text
        .contains("urn:semantic-fabric:service-description:describe-one-target-one-hop-query-v1"));
}

#[test]
fn static_gold_and_source_evidence_is_not_fused_with_mutable_postgres() {
    let loaded = capability_catalog::load(&root()).expect("load catalog");
    let static_cell = loaded
        .catalog
        .cells
        .iter()
        .find(|cell| cell.id == "mapping-semantic-builder-gold-sealed-source-generic")
        .expect("static gold/source cell");
    assert_eq!(static_cell.backend_id, "generic");
    assert_eq!(static_cell.verification, Verification::SourceOnly);
    assert_eq!(
        static_cell.evidence_ids,
        [
            "e-semantic-builder-gold-external",
            "e-semantic-builder-gold-loader",
            "e-semantic-builder-gold-ontology"
        ]
    );

    let live_cell = loaded
        .catalog
        .cells
        .iter()
        .find(|cell| cell.id == "mapping-product-mock-live-postgresql")
        .expect("mutable PostgreSQL cell");
    assert_eq!(live_cell.backend_id, "postgresql");
    assert_eq!(live_cell.verification, Verification::LiveOptional);
    assert_eq!(
        live_cell.evidence_ids,
        [
            "e-product-mock-live-pg",
            "e-product-mock-schema-live-pg",
            "e-product-mock-serve-live-pg",
            "e-product-mock-serve-support"
        ]
    );

    let external = loaded
        .catalog
        .evidence
        .iter()
        .find(|evidence| evidence.id == "e-semantic-builder-gold-external")
        .expect("external KAT evidence");
    assert_eq!(external.verification, Verification::SourceOnly);
    assert!(!external.required);
    let command = loaded
        .catalog
        .commands
        .iter()
        .find(|command| command.id == "cmd-semantic-builder-gold-external")
        .expect("external KAT command");
    assert_eq!(command.mode, CommandMode::Diagnostic);

    let serve_command = loaded
        .catalog
        .commands
        .iter()
        .find(|command| command.id == "cmd-product-mock-serve-live-pg")
        .expect("live Product Mock serve command");
    assert_eq!(serve_command.mode, CommandMode::Diagnostic);
}

#[test]
fn receipt_commands_are_canonical_and_required() {
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
        (
            "cmd-receipt-postgresql-check",
            "cargo run --locked -p sf-conformance --bin rdb2rdf-execution-receipt \
             -- --backend postgresql --check",
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

    let postgres = mutated(|value| {
        let cell = by_id(array(value, "cells"), "select-execution-postgresql");
        cell["evidenceIds"] = serde_json::json!(["e-receipt-postgresql"]);
        cell["verification"] = Value::String("receipt".to_owned());
    })
    .unwrap_err();
    assert!(postgres.contains("promotes mapping evidence"), "{postgres}");
}

#[test]
fn postgresql_mapping_receipt_does_not_admit_the_backend() {
    let loaded = capability_catalog::load(&root()).expect("load catalog");
    for id in ["mapping-direct-postgresql", "mapping-r2rml-postgresql"] {
        let cell = loaded
            .catalog
            .cells
            .iter()
            .find(|cell| cell.id == id)
            .unwrap_or_else(|| panic!("missing {id}"));
        assert_eq!(cell.status, Status::Implemented);
        assert_eq!(cell.verification, Verification::Receipt);
        assert!(!cell.advertisable);
        assert!(!cell.semantic_exact);
    }
    let generation = loaded
        .catalog
        .cells
        .iter()
        .find(|cell| cell.id == "verified-source-generation-postgresql")
        .expect("PostgreSQL verified-generation cell");
    assert_eq!(generation.status, Status::Implemented);
    assert_eq!(generation.verification, Verification::CiRequired);
    assert!(generation.semantic_exact);
    assert!(generation.bounded);
    assert!(!generation.advertisable);
    assert_eq!(
        generation.limitation_ids,
        ["l-verified-source-generation-promotion"]
    );
    assert_eq!(
        generation.evidence_ids,
        [
            "e-architecture-schema-lifecycle",
            "e-postgresql-observation-qualification-pair",
            "e-postgresql-verified-generation-budget",
            "e-postgresql-verified-generation-budget-expiry",
            "e-postgresql-verified-generation-ci",
            "e-postgresql-verified-generation-core",
            "e-postgresql-verified-generation-lease",
            "e-postgresql-verified-generation-live",
            "e-postgresql-verified-generation-request-route",
            "e-postgresql-verified-generation-runtime-role",
        ]
    );
    let command = loaded
        .catalog
        .commands
        .iter()
        .find(|command| command.id == "cmd-postgresql-verified-generation-live")
        .expect("PostgreSQL verified-generation command");
    assert_eq!(
        command.argv,
        "cargo test --locked -p sf-serve --lib \
         pg_generation::live_tests::verified_generation_lifecycle_is_coherent_and_fail_closed \
         -- --ignored --exact --test-threads=1 --nocapture"
    );
    assert_eq!(command.mode, CommandMode::Required);
    let admission = loaded
        .catalog
        .cells
        .iter()
        .find(|cell| cell.id == "production-source-admission-postgresql")
        .expect("PostgreSQL admission cell");
    assert_eq!(admission.status, Status::Planned);
    assert_eq!(admission.verification, Verification::SourceOnly);
    assert!(!admission.advertisable);
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
