use std::path::Path;

use super::{
    format, metadata, plain_cargo_command, platform, Edge, OriginKind, PackageRecord, Receipt,
    TARGET,
};

fn package(name: &str, features: &[&str], edges: Vec<Edge>) -> PackageRecord {
    let mut package = PackageRecord {
        key: String::new(),
        name: name.to_owned(),
        version: "1.0.0".to_owned(),
        origin_kind: OriginKind::Registry,
        origin: "registry+https://github.com/rust-lang/crates.io-index".to_owned(),
        features: features.iter().map(|value| (*value).to_owned()).collect(),
        edges,
    };
    package.key = package.computed_key();
    package
}

fn receipt(mut packages: Vec<PackageRecord>) -> Receipt {
    packages.sort_by(|left, right| left.key.cmp(&right.key));
    Receipt::from_parts(
        "0".repeat(64),
        "1".repeat(64),
        "cargo 1.96.0 (30a34c682 2026-05-25)",
        "rustc 1.96.0 (ac68faa20 2026-05-25)",
        "x86_64-unknown-linux-gnu",
        packages,
    )
    .unwrap()
}

fn linux_target() -> platform::TargetContext {
    platform::TargetContext::parse(
        TARGET,
        "target_arch=\"x86_64\"\ntarget_os=\"linux\"\nunix\n",
    )
    .unwrap()
}

#[test]
fn cargo_receipt_commands_override_ambient_colour_for_machine_output() {
    let command = plain_cargo_command(Path::new("/tmp/checkout-a"));
    let colour = command
        .get_envs()
        .find_map(|(name, value)| (name == "CARGO_TERM_COLOR").then_some(value))
        .flatten();

    assert_eq!(colour, Some(std::ffi::OsStr::new("never")));
}

#[test]
fn should_round_trip_one_canonical_package_row_per_package() {
    let dependency = package("beta", &["default"], Vec::new());
    let root = package(
        "alpha",
        &["default", "std"],
        vec![Edge {
            alias: "beta".to_owned(),
            package_key: dependency.key.clone(),
            kind: "normal".to_owned(),
            target: None,
        }],
    );
    let expected = receipt(vec![root, dependency]);

    let rendered = format::render(&expected).unwrap();
    let parsed = format::parse(&rendered).unwrap();

    assert_eq!(parsed, expected);
    assert_eq!(
        rendered
            .lines()
            .filter(|line| line.starts_with("package\t"))
            .count(),
        2
    );
}

#[test]
fn should_reject_noncanonical_feature_order() {
    let invalid = package("alpha", &["std", "default"], Vec::new());

    let error = Receipt::from_parts(
        "0".repeat(64),
        "1".repeat(64),
        "cargo 1.96.0 (30a34c682 2026-05-25)",
        "rustc 1.96.0 (ac68faa20 2026-05-25)",
        "x86_64-unknown-linux-gnu",
        vec![invalid],
    )
    .unwrap_err();

    assert!(error.contains("features"), "{error}");
}

#[test]
fn should_normalize_checkout_paths_and_exclude_dev_only_edges() {
    let first = metadata::parse(
        &metadata_fixture("/tmp/checkout-a"),
        tree_fixture(),
        Path::new("/tmp/checkout-a"),
        &linux_target(),
    )
    .unwrap();
    let second = metadata::parse(
        &metadata_fixture("/different/checkout-b"),
        tree_fixture(),
        Path::new("/different/checkout-b"),
        &linux_target(),
    )
    .unwrap();

    assert_eq!(first, second);
    assert_eq!(first.len(), 2);
    let root = first
        .iter()
        .find(|package| package.name == "sf-cli")
        .unwrap();
    assert_eq!(root.origin, "crates/sf-cli/Cargo.toml");
    assert_eq!(root.edges.len(), 1);
    assert_eq!(root.edges[0].kind, "build");
}

#[test]
fn should_reject_external_path_dependencies() {
    let raw = metadata_fixture("/tmp/checkout-a").replace(
        "/tmp/checkout-a/crates/dep/Cargo.toml",
        "/tmp/external/dep/Cargo.toml",
    );

    let error = metadata::parse(
        &raw,
        tree_fixture(),
        Path::new("/tmp/checkout-a"),
        &linux_target(),
    )
    .unwrap_err();

    assert!(error.contains("outside repository"), "{error}");
}

#[test]
fn should_limit_workspace_wide_features_to_the_root_package_tree() {
    let raw = metadata_fixture("/tmp/checkout-a").replace(
        r#""features":["default"],"deps""#,
        r#""features":["default","inactive"],"deps""#,
    );

    let packages = metadata::parse(
        &raw,
        tree_fixture(),
        Path::new("/tmp/checkout-a"),
        &linux_target(),
    )
    .unwrap();

    assert_eq!(packages.len(), 2);
    assert!(packages.iter().all(|package| package.name != "inactive"));
    let root = packages
        .iter()
        .find(|package| package.name == "sf-cli")
        .unwrap();
    assert_eq!(root.features, ["default"]);
}

#[test]
fn should_emit_only_dependency_kinds_active_for_the_target() {
    let raw = metadata_fixture("/tmp/checkout-a").replace(
        r#"[{"kind":"build","target":null}]"#,
        r#"[{"kind":"build","target":null},{"kind":"build","target":"cfg(windows)"}]"#,
    );

    let packages = metadata::parse(
        &raw,
        tree_fixture(),
        Path::new("/tmp/checkout-a"),
        &linux_target(),
    )
    .unwrap();
    let root = packages
        .iter()
        .find(|package| package.name == "sf-cli")
        .unwrap();

    assert_eq!(root.edges.len(), 1);
    assert_eq!(root.edges[0].target, None);
}

#[test]
fn should_preserve_an_active_target_expression_on_the_edge() {
    let raw = metadata_fixture("/tmp/checkout-a").replace(
        r#"[{"kind":"build","target":null}]"#,
        r#"[{"kind":"build","target":"cfg(unix)"}]"#,
    );

    let packages = metadata::parse(
        &raw,
        tree_fixture(),
        Path::new("/tmp/checkout-a"),
        &linux_target(),
    )
    .unwrap();
    let root = packages
        .iter()
        .find(|package| package.name == "sf-cli")
        .unwrap();

    assert_eq!(root.edges.len(), 1);
    assert_eq!(root.edges[0].target.as_deref(), Some("cfg(unix)"));
}

#[test]
fn should_reject_an_active_metadata_dependency_missing_from_cargo_tree() {
    let error = metadata::parse(
        &metadata_fixture("/tmp/checkout-a"),
        "sf-cli v0.0.0\tdefault\n",
        Path::new("/tmp/checkout-a"),
        &linux_target(),
    )
    .unwrap_err();

    assert!(error.contains("absent from cargo tree"), "{error}");
}

#[test]
fn should_reject_an_additional_root_binary() {
    let raw = metadata_fixture("/tmp/checkout-a").replace(
        r#"}],"dependencies":[{"name":"dep""#,
        r#"},{"kind":["bin"],"name":"other","src_path":"/tmp/checkout-a/crates/sf-cli/src/other.rs","required-features":[]}],"dependencies":[{"name":"dep""#,
    );

    let error = metadata::parse(
        &raw,
        tree_fixture(),
        Path::new("/tmp/checkout-a"),
        &linux_target(),
    )
    .unwrap_err();

    assert!(error.contains("exactly one binary"), "{error}");
}

#[test]
fn should_reject_a_root_binary_whose_required_feature_is_not_active() {
    let raw = metadata_fixture("/tmp/checkout-a").replace(
        r#""required-features":[]"#,
        r#""required-features":["missing"]"#,
    );

    let error = metadata::parse(
        &raw,
        tree_fixture(),
        Path::new("/tmp/checkout-a"),
        &linux_target(),
    )
    .unwrap_err();

    assert!(error.contains("required features"), "{error}");
}

fn tree_fixture() -> &'static str {
    "sf-cli v0.0.0\tdefault\ndep v1.0.0\t\n"
}

fn metadata_fixture(root: &str) -> String {
    format!(
        r#"{{
          "packages": [
            {{"name":"sf-cli","version":"0.0.0","id":"path+file://{root}/crates/sf-cli#0.0.0","source":null,"manifest_path":"{root}/crates/sf-cli/Cargo.toml","targets":[{{"kind":["bin"],"name":"semantic-fabric","src_path":"{root}/crates/sf-cli/src/main.rs","required-features":[]}}],"dependencies":[{{"name":"dep","rename":null,"optional":false,"kind":"build","target":null}},{{"name":"dev-only","rename":null,"optional":false,"kind":"dev","target":null}},{{"name":"inactive","rename":null,"optional":true,"kind":null,"target":null}}],"features":{{"inactive":["dep:inactive"]}}}},
            {{"name":"dep","version":"1.0.0","id":"path+file://{root}/crates/dep#1.0.0","source":null,"manifest_path":"{root}/crates/dep/Cargo.toml","targets":[{{"kind":["lib"],"name":"dep","src_path":"{root}/crates/dep/src/lib.rs","required-features":[]}}],"dependencies":[],"features":{{}}}},
            {{"name":"dev-only","version":"1.0.0","id":"registry+https://github.com/rust-lang/crates.io-index#dev-only@1.0.0","source":"registry+https://github.com/rust-lang/crates.io-index","manifest_path":"/cache/dev-only/Cargo.toml","targets":[{{"kind":["lib"],"name":"dev_only","src_path":"/cache/dev-only/src/lib.rs","required-features":[]}}],"dependencies":[],"features":{{}}}},
            {{"name":"inactive","version":"1.0.0","id":"registry+https://github.com/rust-lang/crates.io-index#inactive@1.0.0","source":"registry+https://github.com/rust-lang/crates.io-index","manifest_path":"/cache/inactive/Cargo.toml","targets":[{{"kind":["lib"],"name":"inactive","src_path":"/cache/inactive/src/lib.rs","required-features":[]}}],"dependencies":[],"features":{{}}}}
          ],
          "workspace_members": ["path+file://{root}/crates/sf-cli#0.0.0","path+file://{root}/crates/dep#1.0.0"],
          "workspace_default_members": ["path+file://{root}/crates/sf-cli#0.0.0"],
          "resolve": {{"root":"path+file://{root}/crates/sf-cli#0.0.0","nodes":[
            {{"id":"path+file://{root}/crates/sf-cli#0.0.0","features":["default"],"deps":[
              {{"name":"dep","pkg":"path+file://{root}/crates/dep#1.0.0","dep_kinds":[{{"kind":"build","target":null}}]}},
              {{"name":"dev_only","pkg":"registry+https://github.com/rust-lang/crates.io-index#dev-only@1.0.0","dep_kinds":[{{"kind":"dev","target":null}}]}},
              {{"name":"inactive","pkg":"registry+https://github.com/rust-lang/crates.io-index#inactive@1.0.0","dep_kinds":[{{"kind":null,"target":null}}]}}
            ]}},
            {{"id":"path+file://{root}/crates/dep#1.0.0","features":[],"deps":[]}},
            {{"id":"registry+https://github.com/rust-lang/crates.io-index#dev-only@1.0.0","features":[],"deps":[]}},
            {{"id":"registry+https://github.com/rust-lang/crates.io-index#inactive@1.0.0","features":[],"deps":[]}}
          ]}}
        }}"#
    )
}
