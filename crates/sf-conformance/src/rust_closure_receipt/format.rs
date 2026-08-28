use std::collections::BTreeMap;
use std::fmt::Write;

use super::{
    Edge, OriginKind, PackageRecord, Receipt, ROOT_BINARY, ROOT_MANIFEST, ROOT_PACKAGE, TARGET,
};

const HEADER: &str = "semantic-fabric-rust-closure-receipt-v1";
const SCOPE: &str = "locked-dependency-resolution-and-current-default-sf-cli-package-closure";
const EXCLUDED: &str = "binary-bytes,build-script-output,linker-inputs,system-libraries";
const COMMAND: &str = "cargo metadata --locked --offline --format-version 1 --manifest-path crates/sf-cli/Cargo.toml --filter-platform x86_64-unknown-linux-gnu";
const CLOSURE_COMMAND: &str = "cargo tree --locked --offline -p sf-cli -e normal,build --target x86_64-unknown-linux-gnu --prefix none --format {p}<TAB>{f}";
const TARGET_CFG_COMMAND: &str = "rustc --print cfg --target x86_64-unknown-linux-gnu";
pub(super) const MAX_RECEIPT_BYTES: u64 = 2 * 1024 * 1024;
pub(super) const MAX_PACKAGES: usize = 450;
const MAX_LINE_BYTES: usize = 64 * 1024;

pub(super) fn render(receipt: &Receipt) -> Result<String, String> {
    super::validate_packages(&receipt.packages)?;
    let mut output = String::new();
    writeln!(output, "{HEADER}").expect("String writes cannot fail");
    for (key, value) in metadata(receipt) {
        writeln!(output, "meta\t{key}\t{value}").expect("String writes cannot fail");
    }
    for package in &receipt.packages {
        let features = serde_json::to_string(&package.features)
            .map_err(|error| format!("serialize package features: {error}"))?;
        let edges: Vec<_> = package
            .edges
            .iter()
            .map(|edge| {
                [
                    edge.alias.clone(),
                    edge.package_key.clone(),
                    edge.kind.clone(),
                    edge.target.clone().unwrap_or_else(|| "-".to_owned()),
                ]
            })
            .collect();
        let edges = serde_json::to_string(&edges)
            .map_err(|error| format!("serialize package edges: {error}"))?;
        writeln!(
            output,
            "package\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            package.key,
            package.name,
            package.version,
            package.origin_kind.name(),
            package.origin,
            features,
            edges,
        )
        .expect("String writes cannot fail");
    }
    if output.len() as u64 > MAX_RECEIPT_BYTES
        || output.lines().any(|line| line.len() > MAX_LINE_BYTES)
    {
        return Err("rendered receipt exceeds its bounds".to_owned());
    }
    Ok(output)
}

pub(super) fn parse(input: &str) -> Result<Receipt, String> {
    if input.len() as u64 > MAX_RECEIPT_BYTES {
        return Err(format!("receipt exceeds {MAX_RECEIPT_BYTES} bytes"));
    }
    if input.lines().any(|line| line.len() > MAX_LINE_BYTES) {
        return Err(format!("receipt line exceeds {MAX_LINE_BYTES} bytes"));
    }
    let mut lines = input.lines().enumerate();
    let Some((_, header)) = lines.next() else {
        return Err("receipt is empty".to_owned());
    };
    if header != HEADER {
        return Err("invalid Rust closure receipt header".to_owned());
    }
    let mut metadata = BTreeMap::new();
    let mut packages = Vec::new();
    for (index, line) in lines {
        let number = index + 1;
        let fields: Vec<_> = line.split('\t').collect();
        match fields.as_slice() {
            ["meta", key, value] if packages.is_empty() => {
                if metadata.insert(*key, *value).is_some() {
                    return Err(format!("line {number}: duplicate metadata key {key}"));
                }
            }
            ["package", key, name, version, kind, origin, features, edges] => {
                if packages.len() == MAX_PACKAGES {
                    return Err(format!("receipt exceeds {MAX_PACKAGES} packages"));
                }
                packages.push(parse_package(
                    [key, name, version, kind, origin, features, edges],
                    number,
                )?);
            }
            ["meta", ..] => return Err(format!("line {number}: metadata follows packages")),
            _ => return Err(format!("line {number}: malformed receipt record")),
        }
    }
    expect(&mut metadata, "attestation-scope", SCOPE)?;
    expect(&mut metadata, "excluded-provenance", EXCLUDED)?;
    expect(&mut metadata, "production-admission", "not-attested")?;
    expect(&mut metadata, "resolution-command", COMMAND)?;
    expect(&mut metadata, "closure-command", CLOSURE_COMMAND)?;
    expect(&mut metadata, "target-cfg-command", TARGET_CFG_COMMAND)?;
    expect(&mut metadata, "root-manifest", ROOT_MANIFEST)?;
    expect(&mut metadata, "root-package", ROOT_PACKAGE)?;
    expect(&mut metadata, "root-binary", ROOT_BINARY)?;
    expect(&mut metadata, "target", TARGET)?;
    expect(&mut metadata, "edge-kinds", "normal,build")?;
    expect(&mut metadata, "feature-mode", "default")?;
    let cargo_lock_sha256 = take(&mut metadata, "cargo-lock-sha256")?.to_owned();
    let rust_toolchain_sha256 = take(&mut metadata, "rust-toolchain-sha256")?.to_owned();
    let cargo_version = take(&mut metadata, "cargo-version")?.to_owned();
    let rustc_version = take(&mut metadata, "rustc-version")?.to_owned();
    let host = take(&mut metadata, "host")?.to_owned();
    let package_count = parse_count(take(&mut metadata, "package-count")?, "package-count")?;
    let workspace_count = parse_count(
        take(&mut metadata, "workspace-package-count")?,
        "workspace-package-count",
    )?;
    let external_count = parse_count(
        take(&mut metadata, "external-package-count")?,
        "external-package-count",
    )?;
    let feature_count = parse_count(take(&mut metadata, "feature-count")?, "feature-count")?;
    let edge_count = parse_count(take(&mut metadata, "edge-count")?, "edge-count")?;
    let closure_sha256 = take(&mut metadata, "closure-sha256")?.to_owned();
    if let Some(key) = metadata.keys().next() {
        return Err(format!("unknown receipt metadata key {key}"));
    }
    let receipt = Receipt::from_parts(
        cargo_lock_sha256,
        rust_toolchain_sha256,
        &cargo_version,
        &rustc_version,
        &host,
        packages,
    )?;
    if package_count != receipt.package_count()
        || workspace_count != receipt.workspace_package_count()
        || external_count != receipt.package_count() - receipt.workspace_package_count()
        || feature_count != receipt.feature_count()
        || edge_count != receipt.edge_count()
    {
        return Err("receipt count metadata mismatch".to_owned());
    }
    if closure_sha256 != receipt.closure_sha256 {
        return Err("receipt closure digest mismatch".to_owned());
    }
    Ok(receipt)
}

fn parse_package(fields: [&str; 7], line: usize) -> Result<PackageRecord, String> {
    let [key, name, version, kind, origin, features, edges] = fields;
    let features: Vec<String> = serde_json::from_str(features)
        .map_err(|error| format!("line {line}: invalid feature JSON: {error}"))?;
    let raw_edges: Vec<[String; 4]> = serde_json::from_str(edges)
        .map_err(|error| format!("line {line}: invalid edge JSON: {error}"))?;
    let edges = raw_edges
        .into_iter()
        .map(|[alias, package_key, kind, target]| Edge {
            alias,
            package_key,
            kind,
            target: (target != "-").then_some(target),
        })
        .collect();
    Ok(PackageRecord {
        key: key.to_owned(),
        name: name.to_owned(),
        version: version.to_owned(),
        origin_kind: OriginKind::parse(kind)?,
        origin: origin.to_owned(),
        features,
        edges,
    })
}

fn metadata(receipt: &Receipt) -> Vec<(&'static str, String)> {
    vec![
        ("attestation-scope", SCOPE.to_owned()),
        ("excluded-provenance", EXCLUDED.to_owned()),
        ("production-admission", "not-attested".to_owned()),
        ("resolution-command", COMMAND.to_owned()),
        ("closure-command", CLOSURE_COMMAND.to_owned()),
        ("target-cfg-command", TARGET_CFG_COMMAND.to_owned()),
        ("root-manifest", ROOT_MANIFEST.to_owned()),
        ("root-package", ROOT_PACKAGE.to_owned()),
        ("root-binary", ROOT_BINARY.to_owned()),
        ("target", TARGET.to_owned()),
        ("host", receipt.host.clone()),
        ("edge-kinds", "normal,build".to_owned()),
        ("feature-mode", "default".to_owned()),
        ("cargo-lock-sha256", receipt.cargo_lock_sha256.clone()),
        (
            "rust-toolchain-sha256",
            receipt.rust_toolchain_sha256.clone(),
        ),
        ("cargo-version", receipt.cargo_version.clone()),
        ("rustc-version", receipt.rustc_version.clone()),
        ("package-count", receipt.package_count().to_string()),
        (
            "workspace-package-count",
            receipt.workspace_package_count().to_string(),
        ),
        (
            "external-package-count",
            (receipt.package_count() - receipt.workspace_package_count()).to_string(),
        ),
        ("feature-count", receipt.feature_count().to_string()),
        ("edge-count", receipt.edge_count().to_string()),
        ("closure-sha256", receipt.closure_sha256.clone()),
    ]
}

fn take<'a>(metadata: &mut BTreeMap<&'a str, &'a str>, key: &str) -> Result<&'a str, String> {
    metadata
        .remove(key)
        .ok_or_else(|| format!("missing receipt metadata key {key}"))
}

fn expect<'a>(
    metadata: &mut BTreeMap<&'a str, &'a str>,
    key: &str,
    expected: &str,
) -> Result<(), String> {
    let actual = take(metadata, key)?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "receipt metadata {key} is {actual:?}, expected {expected:?}"
        ))
    }
}

fn parse_count(value: &str, key: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("invalid receipt count {key}"))
}
