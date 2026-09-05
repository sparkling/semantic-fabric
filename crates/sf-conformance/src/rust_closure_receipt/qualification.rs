//! Parser-free Cargo inputs for later ADR-0053 parser-profile qualification.

use std::fs;
use std::path::Path;
use std::time::Duration;

use super::context_tree::{ContextKind, ParsedTree};
use super::OriginKind;

pub const RECEIPT_PATH: &str = "tests/rust-parser-worker-qualification-inputs-v1.tsv";
pub const PROFILE: &str = "parser-worker-qualification-inputs-v1";
pub const ROOT_FEATURE: &str = "parser-worker-evidence";

const MAX_INPUT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CONTEXT_TREE_BYTES: u64 = 64 * 1024 * 1024;
const CONTEXT_TREE_TIMEOUT: Duration = Duration::from_secs(90);
const METADATA_ARGUMENTS: &[&str] = &[
    "metadata",
    "--locked",
    "--offline",
    "--format-version",
    "1",
    "--manifest-path",
    super::ROOT_MANIFEST,
    "--filter-platform",
    super::TARGET,
    "--no-default-features",
    "--features",
    ROOT_FEATURE,
];
const CONTEXT_TREE_ARGUMENTS: &[&str] = &[
    "tree",
    "--locked",
    "--offline",
    "--manifest-path",
    super::ROOT_MANIFEST,
    "-p",
    super::ROOT_PACKAGE,
    "-e",
    "normal,build",
    "--target",
    super::TARGET,
    "--charset",
    "ascii",
    "--no-dedupe",
    "--prefix",
    "indent",
    "--format",
    "{p}\t{f}",
    "--no-default-features",
    "--features",
    ROOT_FEATURE,
];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct QualifiedEdge {
    pub(super) alias: String,
    pub(super) package_key: String,
    pub(super) context: ContextKind,
    pub(super) kind: String,
    pub(super) target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct QualifiedContext {
    pub(super) kind: ContextKind,
    pub(super) features: Vec<String>,
    pub(super) edges: Vec<QualifiedEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QualifiedPackage {
    pub(super) key: String,
    pub(super) name: String,
    pub(super) version: String,
    pub(super) origin_kind: OriginKind,
    pub(super) origin: String,
    pub(super) contexts: Vec<QualifiedContext>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualificationInputsReceipt {
    pub(super) cargo_lock_sha256: String,
    pub(super) rust_toolchain_sha256: String,
    pub(super) workspace_manifests_sha256: String,
    pub(super) workspace_manifest_count: usize,
    pub(super) target_cfg_sha256: String,
    pub(super) cargo_version: String,
    pub(super) rustc_version: String,
    pub(super) host: String,
    pub(super) closure_sha256: String,
    pub(super) qualification_inputs_sha256: String,
    pub(super) packages: Vec<QualifiedPackage>,
}

impl QualificationInputsReceipt {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn from_parts(
        cargo_lock_sha256: String,
        rust_toolchain_sha256: String,
        workspace_manifests_sha256: String,
        workspace_manifest_count: usize,
        target_cfg_sha256: String,
        cargo_version: String,
        rustc_version: String,
        host: String,
        packages: Vec<QualifiedPackage>,
    ) -> Result<Self, String> {
        for (label, digest) in [
            ("Cargo.lock", &cargo_lock_sha256),
            ("rust-toolchain.toml", &rust_toolchain_sha256),
            ("workspace manifests", &workspace_manifests_sha256),
            ("target cfg", &target_cfg_sha256),
        ] {
            super::validate_sha256(label, digest)?;
        }
        if workspace_manifest_count == 0 {
            return Err("qualification receipt has no workspace manifests".to_owned());
        }
        super::validate_text("Cargo version", &cargo_version)?;
        super::validate_text("rustc version", &rustc_version)?;
        super::validate_text("host", &host)?;
        if host != super::TARGET {
            return Err(format!(
                "qualification receipt host {host:?} does not equal target {:?}",
                super::TARGET
            ));
        }
        super::qualification_model::validate_packages(&packages)?;
        let closure_sha256 = super::qualification_model::closure_digest(&packages);
        let mut receipt = Self {
            cargo_lock_sha256,
            rust_toolchain_sha256,
            workspace_manifests_sha256,
            workspace_manifest_count,
            target_cfg_sha256,
            cargo_version,
            rustc_version,
            host,
            closure_sha256,
            qualification_inputs_sha256: String::new(),
            packages,
        };
        receipt.qualification_inputs_sha256 =
            super::qualification_format::qualification_inputs_digest(&receipt);
        Ok(receipt)
    }

    pub fn package_count(&self) -> usize {
        self.packages.len()
    }

    pub fn context_count(&self) -> usize {
        self.packages
            .iter()
            .map(|package| package.contexts.len())
            .sum()
    }

    pub fn edge_count(&self) -> usize {
        self.packages
            .iter()
            .flat_map(|package| &package.contexts)
            .map(|context| context.edges.len())
            .sum()
    }

    pub fn closure_sha256(&self) -> &str {
        &self.closure_sha256
    }

    pub fn qualification_inputs_sha256(&self) -> &str {
        &self.qualification_inputs_sha256
    }
}

pub fn generate(repo_root: &Path) -> Result<String, String> {
    super::qualification_format::render(&capture(repo_root)?)
}

pub fn check(repo_root: &Path, receipt_path: &Path) -> Result<QualificationInputsReceipt, String> {
    let (expected, expected_bytes) = load(receipt_path)?;
    let observed = capture(repo_root)?;
    super::authority::ensure_unchanged(
        receipt_path,
        &expected_bytes,
        super::qualification_format::MAX_RECEIPT_BYTES,
    )?;
    if expected != observed {
        return Err(format!(
            "parser-worker qualification-input drift: recorded closure={} inputs={}, actual closure={} inputs={}",
            expected.closure_sha256,
            expected.qualification_inputs_sha256,
            observed.closure_sha256,
            observed.qualification_inputs_sha256,
        ));
    }
    Ok(expected)
}

fn load(path: &Path) -> Result<(QualificationInputsReceipt, Vec<u8>), String> {
    let bytes = super::authority::read(path, super::qualification_format::MAX_RECEIPT_BYTES)?;
    let text = String::from_utf8(bytes).map_err(|error| {
        format!(
            "qualification receipt {} is not UTF-8: {error}",
            path.display()
        )
    })?;
    let receipt = super::qualification_format::parse(&text)?;
    if super::qualification_format::render(&receipt)? != text {
        return Err("qualification receipt is not in canonical generated form".to_owned());
    }
    Ok((receipt, text.into_bytes()))
}

fn capture(repo_root: &Path) -> Result<QualificationInputsReceipt, String> {
    let root = fs::canonicalize(repo_root)
        .map_err(|error| format!("canonicalize repository root: {error}"))?;
    let lock = super::authority::read(&root.join("Cargo.lock"), MAX_INPUT_BYTES)?;
    let toolchain = super::authority::read(&root.join("rust-toolchain.toml"), MAX_INPUT_BYTES)?;
    let manifests = super::workspace_manifests::Snapshot::capture(&root)?;
    let cargo_version = super::command_line("cargo", &["-V"])?;
    let rustc_version = super::command_line("rustc", &["-V"])?;
    let host = super::tool_host("rustc", &["-vV"])?;
    let cargo_host = super::tool_host("cargo", &["-Vv"])?;
    if host != cargo_host || host != super::TARGET {
        return Err(format!(
            "qualification receipt requires native host/target {}; rustc={host}, cargo={cargo_host}",
            super::TARGET
        ));
    }
    let raw_metadata = cargo_metadata(&root)?;
    let raw_tree = cargo_context_tree(&root)?;
    let raw_cfg = super::rustc_target_cfg()?;
    let canonical_cfg = super::platform::canonical_qualification_cfg(&raw_cfg)?;
    let target = super::platform::TargetContext::parse(super::TARGET, &canonical_cfg)?;
    let ParsedTree {
        aggregate,
        contexts,
        edges,
    } = super::context_tree::parse(&raw_tree)?;
    let package_records =
        super::metadata::parse_with_tree(&raw_metadata, aggregate, &root, &target)?;
    super::resolved_features::validate(&raw_metadata, &package_records)?;
    manifests.ensure_covers(&package_records)?;
    let packages = super::qualification_model::bind_contexts(package_records, contexts, edges)?;
    let receipt = QualificationInputsReceipt::from_parts(
        super::sha256(&lock),
        super::sha256(&toolchain),
        manifests.sha256().to_owned(),
        manifests.file_count(),
        super::sha256(canonical_cfg.as_bytes()),
        cargo_version,
        rustc_version,
        host,
        packages,
    )?;
    let lock_after = super::authority::read(&root.join("Cargo.lock"), MAX_INPUT_BYTES)?;
    let toolchain_after =
        super::authority::read(&root.join("rust-toolchain.toml"), MAX_INPUT_BYTES)?;
    manifests.ensure_unchanged(&root)?;
    if lock != lock_after || toolchain != toolchain_after {
        return Err("qualification dependency input changed during capture".to_owned());
    }
    Ok(receipt)
}

fn cargo_metadata(root: &Path) -> Result<String, String> {
    let mut command = super::plain_cargo_command(root);
    command.args(METADATA_ARGUMENTS);
    super::process::output(
        command,
        "parser-worker Cargo metadata",
        super::MAX_METADATA_BYTES,
        super::CARGO_TIMEOUT,
    )
}

fn cargo_context_tree(root: &Path) -> Result<String, String> {
    let mut command = super::plain_cargo_command(root);
    command.args(CONTEXT_TREE_ARGUMENTS);
    super::process::output(
        command,
        "parser-worker context Cargo tree",
        MAX_CONTEXT_TREE_BYTES,
        CONTEXT_TREE_TIMEOUT,
    )
}

pub(super) fn metadata_command_for_receipt() -> String {
    command_for_receipt("cargo", METADATA_ARGUMENTS)
}

pub(super) fn context_tree_command_for_receipt() -> String {
    command_for_receipt("cargo", CONTEXT_TREE_ARGUMENTS)
}

fn command_for_receipt(program: &str, arguments: &[&str]) -> String {
    std::iter::once(program.to_owned())
        .chain(
            arguments
                .iter()
                .map(|argument| argument.replace('\t', "<TAB>")),
        )
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
pub(super) fn digest_for_test(receipt: &QualificationInputsReceipt) -> String {
    super::qualification_format::qualification_inputs_digest(receipt)
}
