#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

#[path = "fixture.rs"]
mod fixture;
#[path = "r2rml.rs"]
mod r2rml;
#[path = "schema.rs"]
mod schema;

pub use fixture::SyntheticFixture;
#[allow(unused_imports)]
pub use r2rml::extract_r2rml;
#[allow(unused_imports)]
pub use schema::{StyleColumn, StyleForeignKey, StyleSchema};

pub const GOLD_ROOT_ENV: &str = "SF_PRODUCT_MOCK_GOLD_ROOT";
pub const SOURCE_ROOT_ENV: &str = "SF_PRODUCT_MOCK_SOURCE_ROOT";
pub const PG_URL_ENV: &str = "SF_PRODUCT_MOCK_PG_URL";
pub const SOURCE_REVISION: &str = "7c45292fccb8b88afe263e18de6806667ae18573";
pub const MANIFEST_PATH: &str = "candidate-manifest.json";
pub const SNAPSHOT_PATH: &str = "source-snapshot.json";
pub const COVERAGE_PATH: &str = "relational-schema-coverage.json";
pub const MAPPING_PATH: &str = "categories/13-source-mapping/part-001.ttl";
pub const INITIAL_MIGRATION: &str =
    "src/services/ProductDesign/ProductDesign.Infrastructure/Migrations/202607181735_Initial.cs";
pub const FK_MIGRATION: &str = "src/services/ProductDesign/ProductDesign.Infrastructure/Migrations/202607212000_AddIntraContextForeignKeys.cs";

const STYLE_MAP: &str =
    "https://hm.com/ns/semantic-product-mock/source-map/relational/ProductDesign/style";
const STYLE_CLASS: &str = "https://hm.com/ns/semantic-product-mock/product-design/Style";
const STYLE_NUMBER_PREDICATE: &str =
    "https://hm.com/ns/semantic-product-mock/product-design/Style/field/StyleNumber";
const VERSION_PREDICATE: &str =
    "https://hm.com/ns/semantic-product-mock/product-design/Style/field/Version";

#[derive(Clone, Debug)]
pub struct Pin {
    pub path: String,
    pub bytes: u64,
    pub digest: String,
}

#[derive(Clone, Debug)]
pub struct SealPolicy {
    pub manifest_bytes: usize,
    pub manifest_sha256: String,
    pub artifact_count: usize,
    pub artifact_bytes: u64,
    pub snapshot_file_count: usize,
    pub snapshot_file_bytes: u64,
    pub source_pins: Vec<Pin>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoldVertical {
    pub r2rml: String,
    pub style: StyleSchema,
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn pin(path: &str, bytes: &[u8]) -> Pin {
    Pin {
        path: path.to_owned(),
        bytes: bytes.len() as u64,
        digest: sha256(bytes),
    }
}

pub fn external_policy() -> SealPolicy {
    SealPolicy {
        manifest_bytes: 38_321,
        manifest_sha256: "sha256:edad6efab2406c021e85dd55a8d4354f5c115e3b6a5e3f6b7a86f75330f2b1cb"
            .to_owned(),
        artifact_count: 139,
        artifact_bytes: 41_845_098,
        snapshot_file_count: 171,
        snapshot_file_bytes: 1_037_818,
        source_pins: vec![
            Pin {
                path: INITIAL_MIGRATION.to_owned(),
                bytes: 6_712,
                digest: "sha256:e28d426c4fdfd214be10babf61031bd4da838d413db73ff3377e264a019bf09d"
                    .to_owned(),
            },
            Pin {
                path: FK_MIGRATION.to_owned(),
                bytes: 2_821,
                digest: "sha256:c02f140eba11b69ab16f79f3ac31fe5f3c9fc2b34f9a092b1f797a0a6e1bf163"
                    .to_owned(),
            },
        ],
    }
}

pub fn load_external(gold_root: &Path, source_root: &Path) -> Result<GoldVertical, &'static str> {
    let gold_root = external_root(gold_root)?;
    let source_root = external_root(source_root)?;
    if gold_root == source_root {
        return Err("gold and source roots must be distinct");
    }
    let manifest = read_beneath(&gold_root, MANIFEST_PATH)?;
    admit(
        &external_policy(),
        &manifest,
        |path| read_beneath(&gold_root, path),
        |path| read_beneath(&source_root, path),
    )
}

fn external_root(path: &Path) -> Result<PathBuf, &'static str> {
    let root = fs::canonicalize(path).map_err(|_| "external root is unavailable")?;
    if !root.is_dir() {
        return Err("external root is not a directory");
    }
    let crate_root = fs::canonicalize(env!("CARGO_MANIFEST_DIR"))
        .map_err(|_| "conformance root is unavailable")?;
    let repository = crate_root
        .parent()
        .and_then(Path::parent)
        .ok_or("conformance root is invalid")?;
    if root.starts_with(repository) {
        return Err("gold and source roots must be external");
    }
    Ok(root)
}

fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path).components().all(|part| {
            matches!(part, Component::Normal(_))
                && part
                    .as_os_str()
                    .to_str()
                    .is_some_and(|value| !value.is_empty())
        })
}

fn read_beneath(root: &Path, relative: &str) -> Result<Vec<u8>, &'static str> {
    if !safe_relative(relative) {
        return Err("artifact path is invalid");
    }
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path).map_err(|_| "sealed input is unavailable")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("sealed input is not a regular file");
    }
    let canonical = fs::canonicalize(&path).map_err(|_| "sealed input is unavailable")?;
    if !canonical.starts_with(root) {
        return Err("sealed input escaped its root");
    }
    fs::read(canonical).map_err(|_| "sealed input could not be read")
}

pub fn admit<A, S>(
    policy: &SealPolicy,
    manifest_bytes: &[u8],
    mut artifact: A,
    mut source: S,
) -> Result<GoldVertical, &'static str>
where
    A: FnMut(&str) -> Result<Vec<u8>, &'static str>,
    S: FnMut(&str) -> Result<Vec<u8>, &'static str>,
{
    if manifest_bytes.len() != policy.manifest_bytes
        || sha256(manifest_bytes) != policy.manifest_sha256
    {
        return Err("candidate manifest seal mismatch");
    }
    let manifest: Value =
        serde_json::from_slice(manifest_bytes).map_err(|_| "candidate manifest is invalid")?;
    exact_manifest_claims(&manifest)?;
    let files = array(&manifest, "/artifactFiles")?;
    if files.len() != policy.artifact_count {
        return Err("transitive artifact count mismatch");
    }
    let mut seen = BTreeSet::new();
    let mut total = 0_u64;
    let mut selected = BTreeMap::new();
    for entry in files {
        let path = string(entry, "/path")?;
        let bytes = unsigned(entry, "/bytes")?;
        let digest = string(entry, "/digest")?;
        if !safe_relative(path) || !valid_digest(digest) || !seen.insert(path.to_owned()) {
            return Err("transitive artifact descriptor is invalid");
        }
        total = total
            .checked_add(bytes)
            .ok_or("transitive artifact byte count overflow")?;
        let value = artifact(path)?;
        if value.len() as u64 != bytes || sha256(&value) != digest {
            return Err("transitive artifact seal mismatch");
        }
        if [SNAPSHOT_PATH, COVERAGE_PATH, MAPPING_PATH].contains(&path) {
            selected.insert(path.to_owned(), value);
        }
    }
    if total != policy.artifact_bytes {
        return Err("transitive artifact byte count mismatch");
    }
    let snapshot = selected
        .remove(SNAPSHOT_PATH)
        .ok_or("source snapshot artifact is missing")?;
    validate_snapshot(policy, &snapshot, &mut source)?;
    let coverage = selected
        .remove(COVERAGE_PATH)
        .ok_or("relational coverage artifact is missing")?;
    let style = validate_coverage(&coverage)?;
    let mapping = selected
        .remove(MAPPING_PATH)
        .ok_or("source mapping artifact is missing")?;
    let r2rml = r2rml::validate_mapping(&mapping)?;
    Ok(GoldVertical { r2rml, style })
}

fn exact_manifest_claims(manifest: &Value) -> Result<(), &'static str> {
    expect_str(manifest, "/source/pinnedRevision", SOURCE_REVISION)?;
    expect_str(manifest, "/source/admittedView", "exact-committed-tree")?;
    expect_bool(manifest, "/source/mutableHeadAndWorkingTreeExcluded", true)?;
    expect_u64(manifest, "/categoryCount", 14)?;
    expect_str(manifest, "/purpose", "development")?;
    expect_bool(manifest, "/applicability/productionAuthority", false)?;
    expect_bool(
        manifest,
        "/operationalQualification/productionAuthority",
        false,
    )?;
    let category = array(manifest, "/categories")?
        .iter()
        .find(|entry| entry.pointer("/category").and_then(Value::as_u64) == Some(13))
        .ok_or("category 13 is missing")?;
    for (field, expected) in [
        ("relationalSchemaTables", 112),
        ("relationalSchemaColumns", 598),
        ("relationalR2rmlTriplesMaps", 1),
        ("relationalR2rmlPredicateObjectMaps", 2),
        ("relationalR2rmlMappedTables", 1),
        ("relationalR2rmlMappedColumns", 2),
    ] {
        expect_u64(category, &format!("/stats/{field}"), expected)?;
    }
    Ok(())
}

fn validate_snapshot<S>(
    policy: &SealPolicy,
    bytes: &[u8],
    source: &mut S,
) -> Result<(), &'static str>
where
    S: FnMut(&str) -> Result<Vec<u8>, &'static str>,
{
    let snapshot: Value =
        serde_json::from_slice(bytes).map_err(|_| "source snapshot is invalid")?;
    expect_str(&snapshot, "/repository/revision", SOURCE_REVISION)?;
    let files = array(&snapshot, "/files")?;
    if files.len() != policy.snapshot_file_count {
        return Err("source snapshot file count mismatch");
    }
    let mut total = 0_u64;
    let mut paths = BTreeSet::new();
    for entry in files {
        let path = string(entry, "/path")?;
        let count = unsigned(entry, "/bytes")?;
        let digest = string(entry, "/digest")?;
        if !safe_relative(path) || !valid_digest(digest) || !paths.insert(path.to_owned()) {
            return Err("source snapshot descriptor is invalid");
        }
        total = total
            .checked_add(count)
            .ok_or("source snapshot byte count overflow")?;
        let actual = source(path)?;
        if actual.len() as u64 != count || sha256(&actual) != digest {
            return Err("source snapshot file seal mismatch");
        }
    }
    if total != policy.snapshot_file_bytes {
        return Err("source snapshot byte count mismatch");
    }
    for expected in &policy.source_pins {
        let entry = files
            .iter()
            .find(|entry| entry.pointer("/path").and_then(Value::as_str) == Some(&expected.path))
            .ok_or("required source snapshot entry is missing")?;
        if unsigned(entry, "/bytes")? != expected.bytes
            || string(entry, "/digest")? != expected.digest
        {
            return Err("required source snapshot entry mismatch");
        }
    }
    Ok(())
}

fn validate_coverage(bytes: &[u8]) -> Result<StyleSchema, &'static str> {
    let coverage: Value =
        serde_json::from_slice(bytes).map_err(|_| "relational coverage is invalid")?;
    expect_str(&coverage, "/sourceRevision", SOURCE_REVISION)?;
    expect_u64(&coverage, "/relationalSchema/summary/tables", 112)?;
    expect_u64(&coverage, "/relationalSchema/summary/columns", 598)?;
    for (field, expected) in [
        ("mappedTables", 1),
        ("mappedColumns", 2),
        ("unmappedTables", 111),
        ("unmappedColumns", 596),
    ] {
        expect_u64(&coverage, &format!("/relationalR2rml/{field}"), expected)?;
    }
    expect_str(&coverage, "/relationalR2rml/status", "partial")?;
    let bindings = array(&coverage, "/relationalR2rml/bindings")?;
    if bindings.len() != 1 {
        return Err("relational R2RML binding count mismatch");
    }
    let binding = &bindings[0];
    expect_str(binding, "/store", "ProductDesign")?;
    expect_str(binding, "/table", "style")?;
    expect_str(binding, "/sourcePath", INITIAL_MIGRATION)?;
    let mapped: Vec<_> = array(binding, "/columns")?
        .iter()
        .map(|value| string(value, "/column"))
        .collect::<Result<_, _>>()?;
    if mapped != ["style_number", "version"] {
        return Err("relational R2RML mapped columns mismatch");
    }
    let store = array(&coverage, "/relationalSchema/stores")?
        .iter()
        .find(|value| value.pointer("/store").and_then(Value::as_str) == Some("ProductDesign"))
        .ok_or("ProductDesign store is missing")?;
    let style = array(store, "/relations")?
        .iter()
        .find(|value| value.pointer("/name").and_then(Value::as_str) == Some("style"))
        .ok_or("Style relation is missing")?;
    schema::parse_style_schema(style)
}

fn string<'a>(value: &'a Value, pointer: &str) -> Result<&'a str, &'static str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or("sealed JSON string is missing")
}

fn unsigned(value: &Value, pointer: &str) -> Result<u64, &'static str> {
    value
        .pointer(pointer)
        .and_then(Value::as_u64)
        .ok_or("sealed JSON integer is missing")
}

fn boolean(value: &Value, pointer: &str) -> Result<bool, &'static str> {
    value
        .pointer(pointer)
        .and_then(Value::as_bool)
        .ok_or("sealed JSON boolean is missing")
}

fn array<'a>(value: &'a Value, pointer: &str) -> Result<&'a [Value], &'static str> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or("sealed JSON array is missing")
}

fn strings(value: &Value, pointer: &str) -> Result<Vec<String>, &'static str> {
    array(value, pointer)?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or("sealed JSON string array is invalid")
        })
        .collect()
}

fn expect_str(value: &Value, pointer: &str, expected: &str) -> Result<(), &'static str> {
    if string(value, pointer)? == expected {
        Ok(())
    } else {
        Err("sealed JSON string claim mismatch")
    }
}

fn expect_u64(value: &Value, pointer: &str, expected: u64) -> Result<(), &'static str> {
    if unsigned(value, pointer)? == expected {
        Ok(())
    } else {
        Err("sealed JSON integer claim mismatch")
    }
}

fn expect_bool(value: &Value, pointer: &str, expected: bool) -> Result<(), &'static str> {
    if boolean(value, pointer)? == expected {
        Ok(())
    } else {
        Err("sealed JSON boolean claim mismatch")
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn admit_synthetic(fixture: &SyntheticFixture) -> Result<GoldVertical, &'static str> {
    admit(
        &fixture.policy,
        &fixture.manifest,
        |path| {
            fixture
                .artifacts
                .get(path)
                .cloned()
                .ok_or("synthetic artifact is missing")
        },
        |path| {
            fixture
                .sources
                .get(path)
                .cloned()
                .ok_or("synthetic source is missing")
        },
    )
}
