//! Deterministic M0 dependency-resolution and current binary-closure receipt.

mod authority;
mod features;
mod format;
mod metadata;
mod origin;
mod platform;
mod process;
#[cfg(test)]
mod tests;
mod tree;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use sha2::{Digest, Sha256};

pub const RECEIPT_PATH: &str = "tests/rust-dependency-closure.tsv";
pub const ROOT_MANIFEST: &str = "crates/sf-cli/Cargo.toml";
pub const ROOT_PACKAGE: &str = "sf-cli";
pub const ROOT_BINARY: &str = "semantic-fabric";
pub const TARGET: &str = "x86_64-unknown-linux-gnu";
const MAX_METADATA_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TREE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_TOOL_OUTPUT_BYTES: u64 = 64 * 1024;
const MAX_CFG_BYTES: u64 = 256 * 1024;
const TOOL_TIMEOUT: Duration = Duration::from_secs(10);
const CARGO_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OriginKind {
    Workspace,
    Registry,
    Git,
}

impl OriginKind {
    fn name(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Registry => "registry",
            Self::Git => "git",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "workspace" => Ok(Self::Workspace),
            "registry" => Ok(Self::Registry),
            "git" => Ok(Self::Git),
            _ => Err(format!("invalid package origin kind {value:?}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Edge {
    pub alias: String,
    pub package_key: String,
    pub kind: String,
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PackageRecord {
    pub key: String,
    pub name: String,
    pub version: String,
    pub origin_kind: OriginKind,
    pub origin: String,
    pub features: Vec<String>,
    pub edges: Vec<Edge>,
}

impl PackageRecord {
    fn computed_key(&self) -> String {
        let mut digest = Sha256::new();
        for value in [
            self.origin_kind.name(),
            self.name.as_str(),
            self.version.as_str(),
            self.origin.as_str(),
        ] {
            digest.update(value.as_bytes());
            digest.update([0]);
        }
        format!("{:x}", digest.finalize())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    cargo_lock_sha256: String,
    rust_toolchain_sha256: String,
    cargo_version: String,
    rustc_version: String,
    host: String,
    closure_sha256: String,
    packages: Vec<PackageRecord>,
}

impl Receipt {
    fn from_parts(
        cargo_lock_sha256: String,
        rust_toolchain_sha256: String,
        cargo_version: &str,
        rustc_version: &str,
        host: &str,
        packages: Vec<PackageRecord>,
    ) -> Result<Self, String> {
        validate_sha256("Cargo.lock", &cargo_lock_sha256)?;
        validate_sha256("rust-toolchain.toml", &rust_toolchain_sha256)?;
        validate_text("cargo version", cargo_version)?;
        validate_text("rustc version", rustc_version)?;
        validate_text("host", host)?;
        validate_packages(&packages)?;
        let closure_sha256 = closure_digest(&packages);
        Ok(Self {
            cargo_lock_sha256,
            rust_toolchain_sha256,
            cargo_version: cargo_version.to_owned(),
            rustc_version: rustc_version.to_owned(),
            host: host.to_owned(),
            closure_sha256,
            packages,
        })
    }

    pub fn package_count(&self) -> usize {
        self.packages.len()
    }

    pub fn workspace_package_count(&self) -> usize {
        self.packages
            .iter()
            .filter(|package| package.origin_kind == OriginKind::Workspace)
            .count()
    }

    pub fn feature_count(&self) -> usize {
        self.packages
            .iter()
            .map(|package| package.features.len())
            .sum()
    }

    pub fn edge_count(&self) -> usize {
        self.packages
            .iter()
            .map(|package| package.edges.len())
            .sum()
    }

    pub fn cargo_lock_sha256(&self) -> &str {
        &self.cargo_lock_sha256
    }

    pub fn closure_sha256(&self) -> &str {
        &self.closure_sha256
    }
}

pub fn generate(repo_root: &Path) -> Result<String, String> {
    format::render(&capture(repo_root)?)
}

pub fn check(repo_root: &Path, receipt_path: &Path) -> Result<Receipt, String> {
    let (expected, expected_bytes) = load(receipt_path)?;
    let observed = capture(repo_root)?;
    authority::ensure_unchanged(receipt_path, &expected_bytes, format::MAX_RECEIPT_BYTES)?;
    if expected != observed {
        return Err(format!(
            "Rust dependency/closure receipt drift: recorded lock={} closure={}, actual lock={} closure={}",
            expected.cargo_lock_sha256,
            expected.closure_sha256,
            observed.cargo_lock_sha256,
            observed.closure_sha256,
        ));
    }
    Ok(expected)
}

fn load(path: &Path) -> Result<(Receipt, Vec<u8>), String> {
    let bytes = authority::read(path, format::MAX_RECEIPT_BYTES)?;
    let text = String::from_utf8(bytes)
        .map_err(|error| format!("receipt {} is not UTF-8: {error}", path.display()))?;
    let receipt = format::parse(&text)?;
    if format::render(&receipt)? != text {
        return Err("receipt is valid but not in canonical generated form".to_owned());
    }
    Ok((receipt, text.into_bytes()))
}

fn capture(repo_root: &Path) -> Result<Receipt, String> {
    let canonical_root = fs::canonicalize(repo_root)
        .map_err(|error| format!("canonicalize repository root: {error}"))?;
    let lock = fs::read(canonical_root.join("Cargo.lock"))
        .map_err(|error| format!("read Cargo.lock: {error}"))?;
    let toolchain = fs::read(canonical_root.join("rust-toolchain.toml"))
        .map_err(|error| format!("read rust-toolchain.toml: {error}"))?;
    let cargo_version = command_line("cargo", &["-V"])?;
    let rustc_version = command_line("rustc", &["-V"])?;
    let host = tool_host("rustc", &["-vV"])?;
    let cargo_host = tool_host("cargo", &["-Vv"])?;
    if host != cargo_host || host != TARGET {
        return Err(format!(
            "receipt requires native host/target {TARGET}; rustc={host}, cargo={cargo_host}"
        ));
    }
    let metadata = cargo_metadata(&canonical_root)?;
    let tree = cargo_tree(&canonical_root)?;
    let target_cfg = rustc_target_cfg()?;
    let target = platform::TargetContext::parse(TARGET, &target_cfg)?;
    let packages = metadata::parse(&metadata, &tree, &canonical_root, &target)?;
    let receipt = Receipt::from_parts(
        sha256(&lock),
        sha256(&toolchain),
        &cargo_version,
        &rustc_version,
        &host,
        packages,
    )?;
    let lock_after = fs::read(canonical_root.join("Cargo.lock"))
        .map_err(|error| format!("re-read Cargo.lock: {error}"))?;
    let toolchain_after = fs::read(canonical_root.join("rust-toolchain.toml"))
        .map_err(|error| format!("re-read rust-toolchain.toml: {error}"))?;
    if lock != lock_after || toolchain != toolchain_after {
        return Err("dependency or toolchain input changed during capture".to_owned());
    }
    Ok(receipt)
}

fn cargo_metadata(repo_root: &Path) -> Result<String, String> {
    let manifest = repo_root.join(ROOT_MANIFEST);
    let mut command = Command::new("cargo");
    command
        .current_dir(repo_root)
        .args([
            "metadata",
            "--locked",
            "--offline",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(&manifest)
        .args(["--filter-platform", TARGET]);
    process::output(command, "cargo metadata", MAX_METADATA_BYTES, CARGO_TIMEOUT)
}

fn cargo_tree(repo_root: &Path) -> Result<String, String> {
    let mut command = Command::new("cargo");
    command.current_dir(repo_root).args([
        "tree",
        "--locked",
        "--offline",
        "-p",
        ROOT_PACKAGE,
        "-e",
        "normal,build",
        "--target",
        TARGET,
        "--prefix",
        "none",
        "--format",
        "{p}\t{f}",
    ]);
    process::output(command, "cargo tree", MAX_TREE_BYTES, CARGO_TIMEOUT)
}

fn command_line(program: &str, arguments: &[&str]) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(arguments);
    let output = process::output(
        command,
        &format!("{program} version"),
        MAX_TOOL_OUTPUT_BYTES,
        TOOL_TIMEOUT,
    )?;
    let line = output.trim();
    validate_text(&format!("{program} version"), line)?;
    Ok(line.to_owned())
}

fn tool_host(program: &str, arguments: &[&str]) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(arguments);
    let output = process::output(
        command,
        &format!("{program} verbose version"),
        MAX_TOOL_OUTPUT_BYTES,
        TOOL_TIMEOUT,
    )?;
    let hosts: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_prefix("host: "))
        .collect();
    match hosts.as_slice() {
        [host] => Ok((*host).to_owned()),
        _ => Err(format!("{program} verbose version has no unique host")),
    }
}

fn rustc_target_cfg() -> Result<String, String> {
    let mut command = Command::new("rustc");
    command.args(["--print", "cfg", "--target", TARGET]);
    process::output(command, "rustc target cfg", MAX_CFG_BYTES, TOOL_TIMEOUT)
}

fn validate_packages(packages: &[PackageRecord]) -> Result<(), String> {
    if packages.is_empty() || packages.len() > format::MAX_PACKAGES {
        return Err(format!("invalid package count {}", packages.len()));
    }
    let keys: BTreeSet<_> = packages.iter().map(|package| &package.key).collect();
    if keys.len() != packages.len() {
        return Err("duplicate package key".to_owned());
    }
    if !packages.windows(2).all(|pair| pair[0].key < pair[1].key) {
        return Err("packages are not in canonical key order".to_owned());
    }
    for package in packages {
        validate_sha256("package key", &package.key)?;
        if package.computed_key() != package.key {
            return Err(format!("package key mismatch for {}", package.name));
        }
        validate_text("package name", &package.name)?;
        validate_text("package version", &package.version)?;
        validate_text("package origin", &package.origin)?;
        validate_sorted("features", &package.features)?;
        if !package.edges.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err(format!("edges for {} are not canonical", package.name));
        }
        for edge in &package.edges {
            validate_text("dependency alias", &edge.alias)?;
            validate_sha256("dependency package key", &edge.package_key)?;
            if !keys.contains(&edge.package_key) {
                return Err(format!("edge from {} leaves the closure", package.name));
            }
            if !matches!(edge.kind.as_str(), "normal" | "build") {
                return Err(format!("invalid dependency kind {:?}", edge.kind));
            }
            if let Some(target) = &edge.target {
                validate_text("dependency target", target)?;
            }
        }
    }
    Ok(())
}

fn validate_sorted(label: &str, values: &[String]) -> Result<(), String> {
    if !values.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(format!("{label} are not sorted and unique"));
    }
    for value in values {
        validate_text(label, value)?;
    }
    Ok(())
}

fn validate_text(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 4096 || value.bytes().any(|byte| byte.is_ascii_control()) {
        Err(format!("invalid {label}"))
    } else {
        Ok(())
    }
}

fn validate_sha256(label: &str, value: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(format!("invalid SHA-256 for {label}"))
    }
}

fn closure_digest(packages: &[PackageRecord]) -> String {
    let mut digest = Sha256::new();
    for package in packages {
        for value in [
            package.key.as_str(),
            package.name.as_str(),
            package.version.as_str(),
            package.origin_kind.name(),
            package.origin.as_str(),
        ] {
            digest.update(value.as_bytes());
            digest.update([0]);
        }
        for feature in &package.features {
            digest.update(b"feature\0");
            digest.update(feature.as_bytes());
            digest.update([0]);
        }
        for edge in &package.edges {
            digest.update(b"edge\0");
            for value in [
                edge.alias.as_str(),
                edge.package_key.as_str(),
                edge.kind.as_str(),
                edge.target.as_deref().unwrap_or("-"),
            ] {
                digest.update(value.as_bytes());
                digest.update([0]);
            }
        }
    }
    format!("{:x}", digest.finalize())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
