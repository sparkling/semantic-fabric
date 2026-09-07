use super::context_tree::ContextKind;
use super::qualification::{
    QualificationInputsReceipt, QualifiedContext, QualifiedEdge, QualifiedPackage, ROOT_FEATURE,
};
use super::{OriginKind, PackageRecord, Receipt, TARGET};

fn package_key(name: &str, origin: &str) -> String {
    PackageRecord {
        key: String::new(),
        name: name.to_owned(),
        version: "0.0.0".to_owned(),
        origin_kind: OriginKind::Workspace,
        origin: origin.to_owned(),
        features: Vec::new(),
        edges: Vec::new(),
    }
    .computed_key()
}

fn packages() -> Vec<QualifiedPackage> {
    let root_key = package_key("sf-cli", "crates/sf-cli/Cargo.toml");
    let sparql_key = package_key("sf-sparql", "crates/sf-sparql/Cargo.toml");
    let mut packages = vec![
        QualifiedPackage {
            key: root_key,
            name: "sf-cli".to_owned(),
            version: "0.0.0".to_owned(),
            origin_kind: OriginKind::Workspace,
            origin: "crates/sf-cli/Cargo.toml".to_owned(),
            contexts: vec![QualifiedContext {
                kind: ContextKind::Target,
                features: vec![ROOT_FEATURE.to_owned()],
                edges: vec![QualifiedEdge {
                    alias: "sf_sparql".to_owned(),
                    package_key: sparql_key.clone(),
                    context: ContextKind::Target,
                    kind: "normal".to_owned(),
                    target: None,
                }],
            }],
        },
        QualifiedPackage {
            key: sparql_key,
            name: "sf-sparql".to_owned(),
            version: "0.0.0".to_owned(),
            origin_kind: OriginKind::Workspace,
            origin: "crates/sf-sparql/Cargo.toml".to_owned(),
            contexts: vec![QualifiedContext {
                kind: ContextKind::Target,
                features: vec![ROOT_FEATURE.to_owned()],
                edges: Vec::new(),
            }],
        },
    ];
    packages.sort_by(|left, right| left.key.cmp(&right.key));
    packages
}

fn receipt(packages: Vec<QualifiedPackage>) -> QualificationInputsReceipt {
    QualificationInputsReceipt::from_parts(
        "0".repeat(64),
        "1".repeat(64),
        "2".repeat(64),
        3,
        "3".repeat(64),
        "cargo 1.96.0 (fixture)".to_owned(),
        "rustc 1.96.0 (fixture)".to_owned(),
        TARGET.to_owned(),
        packages,
    )
    .unwrap()
}

#[test]
fn qualification_receipt_round_trips_and_rejects_cross_kind_substitution() {
    let expected = receipt(packages());
    let rendered = super::qualification_format::render(&expected).unwrap();

    assert_eq!(
        super::qualification_format::parse(&rendered).unwrap(),
        expected
    );
    assert!(super::format::parse(&rendered).is_err());

    let default_package = PackageRecord {
        key: package_key("sf-cli", "crates/sf-cli/Cargo.toml"),
        name: "sf-cli".to_owned(),
        version: "0.0.0".to_owned(),
        origin_kind: OriginKind::Workspace,
        origin: "crates/sf-cli/Cargo.toml".to_owned(),
        features: Vec::new(),
        edges: Vec::new(),
    };
    let default = Receipt::from_parts(
        "0".repeat(64),
        "1".repeat(64),
        "cargo fixture",
        "rustc fixture",
        TARGET,
        vec![default_package],
    )
    .unwrap();
    assert!(super::qualification_format::parse(&super::format::render(&default).unwrap()).is_err());
}

#[test]
fn qualification_receipt_fixes_nonauthority_and_full_input_digest() {
    let receipt = receipt(packages());
    let rendered = super::qualification_format::render(&receipt).unwrap();
    let promoted = rendered.replace("meta\tauthority\tnone\n", "meta\tauthority\tattested\n");
    assert!(super::qualification_format::parse(&promoted).is_err());

    let mut changed = receipt.clone();
    changed.target_cfg_sha256 = "4".repeat(64);
    assert_ne!(
        super::qualification::digest_for_test(&receipt),
        super::qualification::digest_for_test(&changed)
    );
}

#[test]
fn qualification_receipt_requires_exact_features_and_native_host() {
    let mut missing = packages();
    missing
        .iter_mut()
        .find(|package| package.name == "sf-cli")
        .unwrap()
        .contexts[0]
        .features
        .clear();
    assert!(QualificationInputsReceipt::from_parts(
        "0".repeat(64),
        "1".repeat(64),
        "2".repeat(64),
        3,
        "3".repeat(64),
        "cargo fixture".to_owned(),
        "rustc fixture".to_owned(),
        TARGET.to_owned(),
        missing,
    )
    .is_err());

    let mut wrong_host = receipt(packages());
    wrong_host.host = "aarch64-unknown-linux-gnu".to_owned();
    let rendered = super::qualification_format::render(&wrong_host).unwrap();
    assert!(super::qualification_format::parse(&rendered).is_err());
}

#[test]
fn qualification_receipt_rejects_impossible_context_transitions() {
    let mut build_to_target = packages();
    let root = build_to_target
        .iter_mut()
        .find(|package| package.name == "sf-cli")
        .unwrap();
    root.contexts[0].edges[0].kind = "build".to_owned();
    assert!(QualificationInputsReceipt::from_parts(
        "0".repeat(64),
        "1".repeat(64),
        "2".repeat(64),
        3,
        "3".repeat(64),
        "cargo fixture".to_owned(),
        "rustc fixture".to_owned(),
        TARGET.to_owned(),
        build_to_target,
    )
    .unwrap_err()
    .contains("build dependency"));

    let mut root_host = packages();
    let root = root_host
        .iter_mut()
        .find(|package| package.name == "sf-cli")
        .unwrap();
    root.contexts.push(QualifiedContext {
        kind: ContextKind::Host,
        features: Vec::new(),
        edges: Vec::new(),
    });
    assert!(QualificationInputsReceipt::from_parts(
        "0".repeat(64),
        "1".repeat(64),
        "2".repeat(64),
        3,
        "3".repeat(64),
        "cargo fixture".to_owned(),
        "rustc fixture".to_owned(),
        TARGET.to_owned(),
        root_host,
    )
    .unwrap_err()
    .contains("root package"));
}
