use std::collections::BTreeMap;
use std::fmt::Write;

use sha2::{Digest, Sha256};

use super::context_tree::ContextKind;
use super::qualification::{
    QualificationInputsReceipt, QualifiedContext, QualifiedEdge, QualifiedPackage,
};
use super::{OriginKind, ROOT_BINARY, ROOT_MANIFEST, ROOT_PACKAGE, TARGET};

const HEADER: &str = "semantic-fabric-parser-worker-qualification-inputs-receipt-v1";
const SCOPE: &str = "locked-parser-worker-cargo-resolution-and-context-inputs";
const EXCLUDED: &str = "registry-source-bytes,build-script-output,proc-macro-output,artifact-bytes,linker-inputs,rust-stdlib,allocator,system-libraries,runtime-linkage,syscalls,containment,witness,sbom,tool-executable-bytes,ambient-cargo-configuration,transient-input-races";
const TARGET_CFG_COMMAND: &str = "rustc --print cfg --target x86_64-unknown-linux-gnu";

pub(super) const MAX_RECEIPT_BYTES: u64 = 4 * 1024 * 1024;
pub(super) const MAX_PACKAGES: usize = 450;
const MAX_LINE_BYTES: usize = 256 * 1024;

pub(super) fn render(receipt: &QualificationInputsReceipt) -> Result<String, String> {
    super::qualification_model::validate_packages(&receipt.packages)?;
    let mut output = String::new();
    writeln!(output, "{HEADER}").expect("String writes cannot fail");
    for (key, value) in metadata(receipt) {
        writeln!(output, "meta\t{key}\t{value}").expect("String writes cannot fail");
    }
    for package in &receipt.packages {
        let contexts: Vec<_> = package
            .contexts
            .iter()
            .map(|context| {
                let edges: Vec<_> = context
                    .edges
                    .iter()
                    .map(|edge| {
                        [
                            edge.alias.clone(),
                            edge.package_key.clone(),
                            edge.context.name().to_owned(),
                            edge.kind.clone(),
                            edge.target.clone().unwrap_or_else(|| "-".to_owned()),
                        ]
                    })
                    .collect();
                (context.kind.name(), &context.features, edges)
            })
            .collect();
        let contexts = serde_json::to_string(&contexts)
            .map_err(|error| format!("serialize qualification package contexts: {error}"))?;
        writeln!(
            output,
            "package\t{}\t{}\t{}\t{}\t{}\t{}",
            package.key,
            package.name,
            package.version,
            package.origin_kind.name(),
            package.origin,
            contexts,
        )
        .expect("String writes cannot fail");
    }
    if output.len() as u64 > MAX_RECEIPT_BYTES
        || output.lines().any(|line| line.len() > MAX_LINE_BYTES)
    {
        return Err("rendered qualification receipt exceeds its bounds".to_owned());
    }
    Ok(output)
}

pub(super) fn parse(input: &str) -> Result<QualificationInputsReceipt, String> {
    if input.len() as u64 > MAX_RECEIPT_BYTES {
        return Err(format!(
            "qualification receipt exceeds {MAX_RECEIPT_BYTES} bytes"
        ));
    }
    if input.lines().any(|line| line.len() > MAX_LINE_BYTES) {
        return Err(format!(
            "qualification receipt line exceeds {MAX_LINE_BYTES} bytes"
        ));
    }
    let mut lines = input.lines().enumerate();
    let Some((_, header)) = lines.next() else {
        return Err("qualification receipt is empty".to_owned());
    };
    if header != HEADER {
        return Err("invalid parser-worker qualification receipt header".to_owned());
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
            ["package", key, name, version, kind, origin, contexts] => {
                if packages.len() == MAX_PACKAGES {
                    return Err(format!(
                        "qualification receipt exceeds {MAX_PACKAGES} packages"
                    ));
                }
                packages.push(parse_package(
                    [key, name, version, kind, origin, contexts],
                    number,
                )?);
            }
            ["meta", ..] => return Err(format!("line {number}: metadata follows packages")),
            _ => return Err(format!("line {number}: malformed qualification record")),
        }
    }
    for (key, expected) in fixed_metadata() {
        expect(&mut metadata, key, &expected)?;
    }
    let cargo_lock_sha256 = take(&mut metadata, "cargo-lock-sha256")?.to_owned();
    let rust_toolchain_sha256 = take(&mut metadata, "rust-toolchain-sha256")?.to_owned();
    let workspace_manifests_sha256 = take(&mut metadata, "workspace-manifests-sha256")?.to_owned();
    let workspace_manifest_count = parse_count(
        take(&mut metadata, "workspace-manifest-count")?,
        "workspace-manifest-count",
    )?;
    let target_cfg_sha256 = take(&mut metadata, "target-cfg-sha256")?.to_owned();
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
    let context_count = parse_count(take(&mut metadata, "context-count")?, "context-count")?;
    let feature_count = parse_count(take(&mut metadata, "feature-count")?, "feature-count")?;
    let edge_count = parse_count(take(&mut metadata, "edge-count")?, "edge-count")?;
    let closure_sha256 = take(&mut metadata, "closure-sha256")?.to_owned();
    let inputs_sha256 = take(&mut metadata, "qualification-inputs-sha256")?.to_owned();
    if let Some(key) = metadata.keys().next() {
        return Err(format!("unknown qualification metadata key {key}"));
    }
    let receipt = QualificationInputsReceipt::from_parts(
        cargo_lock_sha256,
        rust_toolchain_sha256,
        workspace_manifests_sha256,
        workspace_manifest_count,
        target_cfg_sha256,
        cargo_version,
        rustc_version,
        host,
        packages,
    )?;
    let actual_workspace = receipt
        .packages
        .iter()
        .filter(|package| package.origin_kind == OriginKind::Workspace)
        .count();
    let actual_features: usize = receipt
        .packages
        .iter()
        .flat_map(|package| &package.contexts)
        .map(|context| context.features.len())
        .sum();
    if package_count != receipt.package_count()
        || workspace_count != actual_workspace
        || external_count != receipt.package_count() - actual_workspace
        || context_count != receipt.context_count()
        || feature_count != actual_features
        || edge_count != receipt.edge_count()
    {
        return Err("qualification receipt count metadata mismatch".to_owned());
    }
    if closure_sha256 != receipt.closure_sha256 {
        return Err("qualification receipt closure digest mismatch".to_owned());
    }
    if inputs_sha256 != receipt.qualification_inputs_sha256 {
        return Err("qualification receipt full-input digest mismatch".to_owned());
    }
    Ok(receipt)
}

fn parse_package(fields: [&str; 6], line: usize) -> Result<QualifiedPackage, String> {
    let [key, name, version, kind, origin, contexts] = fields;
    type RawContext = (String, Vec<String>, Vec<[String; 5]>);
    let raw_contexts: Vec<RawContext> = serde_json::from_str(contexts)
        .map_err(|error| format!("line {line}: invalid context JSON: {error}"))?;
    let contexts = raw_contexts
        .into_iter()
        .map(|(kind, features, edges)| {
            let edges = edges
                .into_iter()
                .map(|[alias, package_key, context, kind, target]| {
                    Ok(QualifiedEdge {
                        alias,
                        package_key,
                        context: ContextKind::parse(&context)?,
                        kind,
                        target: (target != "-").then_some(target),
                    })
                })
                .collect::<Result<_, String>>()?;
            Ok(QualifiedContext {
                kind: ContextKind::parse(&kind)?,
                features,
                edges,
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(QualifiedPackage {
        key: key.to_owned(),
        name: name.to_owned(),
        version: version.to_owned(),
        origin_kind: OriginKind::parse(kind)?,
        origin: origin.to_owned(),
        contexts,
    })
}

pub(super) fn qualification_inputs_digest(receipt: &QualificationInputsReceipt) -> String {
    let mut digest = Sha256::new();
    digest.update(b"semantic-fabric-parser-worker-qualification-inputs-v1\0");
    digest.update(HEADER.as_bytes());
    digest.update([0]);
    for (key, value) in metadata(receipt) {
        if key == "qualification-inputs-sha256" {
            continue;
        }
        digest.update(key.as_bytes());
        digest.update([0]);
        digest.update(value.as_bytes());
        digest.update([0]);
    }
    format!("{:x}", digest.finalize())
}

fn metadata(receipt: &QualificationInputsReceipt) -> Vec<(&'static str, String)> {
    let workspace = receipt
        .packages
        .iter()
        .filter(|package| package.origin_kind == OriginKind::Workspace)
        .count();
    let features: usize = receipt
        .packages
        .iter()
        .flat_map(|package| &package.contexts)
        .map(|context| context.features.len())
        .sum();
    let mut values = fixed_metadata();
    values.extend([
        ("host", receipt.host.clone()),
        ("cargo-lock-sha256", receipt.cargo_lock_sha256.clone()),
        (
            "rust-toolchain-sha256",
            receipt.rust_toolchain_sha256.clone(),
        ),
        (
            "workspace-manifests-sha256",
            receipt.workspace_manifests_sha256.clone(),
        ),
        (
            "workspace-manifest-count",
            receipt.workspace_manifest_count.to_string(),
        ),
        ("target-cfg-sha256", receipt.target_cfg_sha256.clone()),
        ("cargo-version", receipt.cargo_version.clone()),
        ("rustc-version", receipt.rustc_version.clone()),
        ("package-count", receipt.package_count().to_string()),
        ("workspace-package-count", workspace.to_string()),
        (
            "external-package-count",
            (receipt.package_count() - workspace).to_string(),
        ),
        ("context-count", receipt.context_count().to_string()),
        ("feature-count", features.to_string()),
        ("edge-count", receipt.edge_count().to_string()),
        ("closure-sha256", receipt.closure_sha256.clone()),
        (
            "qualification-inputs-sha256",
            receipt.qualification_inputs_sha256.clone(),
        ),
    ]);
    values
}

fn fixed_metadata() -> Vec<(&'static str, String)> {
    vec![
        ("profile", super::qualification::PROFILE.to_owned()),
        (
            "canonical-path",
            super::qualification::RECEIPT_PATH.to_owned(),
        ),
        ("attestation-scope", SCOPE.to_owned()),
        ("authority", "none".to_owned()),
        ("parser-execution", "not-run".to_owned()),
        ("parser-produced-query-v1", "not-attested".to_owned()),
        ("qualification", "not-attested".to_owned()),
        ("governed-profile", "not-attested".to_owned()),
        ("production-admission", "not-attested".to_owned()),
        ("serving-path", "not-attested".to_owned()),
        ("release-authority", "none".to_owned()),
        ("runtime-linkage", "not-attested".to_owned()),
        ("syscall-profile", "not-attested".to_owned()),
        ("witness", "not-attested".to_owned()),
        ("sbom", "not-attested".to_owned()),
        (
            "tool-identity-scope",
            "reported-version-and-host-only".to_owned(),
        ),
        ("tool-executable-bytes", "not-attested".to_owned()),
        ("ambient-cargo-configuration", "not-attested".to_owned()),
        ("transient-input-race-freedom", "not-attested".to_owned()),
        ("excluded-provenance", EXCLUDED.to_owned()),
        (
            "resolution-command",
            super::qualification::metadata_command_for_receipt(),
        ),
        (
            "context-tree-command",
            super::qualification::context_tree_command_for_receipt(),
        ),
        (
            "command-working-directory",
            "canonical-repository-root".to_owned(),
        ),
        ("target-cfg-command", TARGET_CFG_COMMAND.to_owned()),
        (
            "context-model",
            "target-or-host-by-build-and-proc-macro-edges".to_owned(),
        ),
        ("context-tree-deduplication", "forbidden".to_owned()),
        ("feature-authority", "root-specific-context-tree".to_owned()),
        (
            "metadata-feature-relation",
            "context-tree-features-subset-of-workspace-resolve".to_owned(),
        ),
        ("root-manifest", ROOT_MANIFEST.to_owned()),
        ("root-package", ROOT_PACKAGE.to_owned()),
        ("root-binary", ROOT_BINARY.to_owned()),
        ("target", TARGET.to_owned()),
        ("edge-kinds", "normal,build".to_owned()),
        (
            "feature-mode",
            "no-default-features+parser-worker-evidence".to_owned(),
        ),
        (
            "root-feature",
            super::qualification::ROOT_FEATURE.to_owned(),
        ),
    ]
}

fn take<'a>(metadata: &mut BTreeMap<&'a str, &'a str>, key: &str) -> Result<&'a str, String> {
    metadata
        .remove(key)
        .ok_or_else(|| format!("missing qualification metadata key {key}"))
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
            "qualification metadata {key} is {actual:?}, expected {expected:?}"
        ))
    }
}

fn parse_count(value: &str, key: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("invalid qualification receipt count {key}"))
}
