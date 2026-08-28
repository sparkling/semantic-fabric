use std::path::{Component, Path};

use sha2::{Digest, Sha256};

pub const HEADER: &str = "semantic-fabric-current-sf-cli-artifact-observation-v1";
pub const ARTIFACT_CLASS: &str = "current-sf-cli-all-in-one-development";
pub const ATTESTATION_SCOPE: &str =
    "exact-single-builder-current-sf-cli-bytes-and-observed-build-provenance";
pub const AUTHORITY_MODEL: &str = "portable-authority-plus-host-specific-observation";
pub const ROOT_MANIFEST: &str = "crates/sf-cli/Cargo.toml";
pub const ROOT_PACKAGE: &str = "sf-cli";
pub const ROOT_BINARY: &str = "semantic-fabric";
pub const PROFILE: &str = "release";
pub const TARGET: &str = "x86_64-unknown-linux-gnu";
pub const COMMAND_TEMPLATE: &str = "cargo rustc --locked --offline --release -p sf-cli --bin semantic-fabric --target x86_64-unknown-linux-gnu --target-dir <TARGET_DIR> --message-format=json-render-diagnostics -- -C linker=<LINKER> -C link-arg=-Wl,--dependency-file=<LINK_DEPFILE>";
pub const ARTIFACT_PATH: &str = "target/x86_64-unknown-linux-gnu/release/semantic-fabric";
pub const BUILD_SCRIPT_DIRECTIVES_FORMAT: &str =
    "semantic-fabric-canonical-build-script-directives-v1";
pub const BUILD_SCRIPT_OUT_TREE_FORMAT: &str = "semantic-fabric-build-script-out-tree-v1";
pub const ENVIRONMENT_POLICY: &str = "semantic-fabric-env-clear-exact-v1:CARGO_HOME,CARGO_INCREMENTAL,CARGO_NET_OFFLINE,HOME,LC_ALL,PATH,RUSTC,RUSTUP_HOME,SOURCE_DATE_EPOCH,TMPDIR,TZ";
pub const ENVIRONMENT_DIGEST_FORMAT: &str =
    "semantic-fabric-sorted-env-name-nul-value-nul-sha256-v1";
pub const SOURCE_DATE_EPOCH_LAW: &str = "git-commit-committer-timestamp-seconds-v1";
pub const NOT_ATTESTED: &str = "not-attested";

pub const NONCLAIM_KEYS: [&str; 16] = [
    "production-minimality",
    "reproducibility",
    "sbom",
    "build-script-input-closure",
    "dynamic-runtime-portability",
    "dynamic-runtime-resolution",
    "backend-admission",
    "production-admission",
    "release-readiness",
    "adr-0039-acceptance",
    "flagless-build-byte-equality",
    "linker-tool-closure",
    "tool-execution-closure",
    "final-link-depfile-authorship",
    "signing",
    "slsa-provenance",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableAuthority {
    pub git_revision: String,
    pub source_date_epoch: u64,
    pub source_inputs_sha256: String,
    pub cargo_lock_sha256: String,
    pub rust_toolchain_sha256: String,
    pub cargo_home_inputs_sha256: String,
    pub cargo_config_set_sha256: String,
    pub closure_receipt_sha256: String,
    pub current_closure_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolRole {
    GitMaterializer,
    Cargo,
    Rustc,
    Sandbox,
    Linker,
    ElfReader,
}

impl ToolRole {
    pub fn name(self) -> &'static str {
        match self {
            Self::GitMaterializer => "git-materializer",
            Self::Cargo => "cargo",
            Self::Rustc => "rustc",
            Self::Sandbox => "sandbox",
            Self::Linker => "linker",
            Self::ElfReader => "elf-reader",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "git-materializer" => Some(Self::GitMaterializer),
            "cargo" => Some(Self::Cargo),
            "rustc" => Some(Self::Rustc),
            "sandbox" => Some(Self::Sandbox),
            "linker" => Some(Self::Linker),
            "elf-reader" => Some(Self::ElfReader),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ToolIdentity {
    pub role: ToolRole,
    pub logical_path: String,
    pub version: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BuildScriptEvent {
    pub package_id: String,
    pub directives_source_byte_length: u64,
    pub directives_sha256: String,
    pub stderr_byte_length: u64,
    pub stderr_sha256: String,
    pub out_tree_file_count: u64,
    pub out_tree_byte_length: u64,
    pub out_tree_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LinkInputOrigin {
    Workspace,
    CargoRegistry,
    RustSysroot,
    BuildOutput,
    HostSystem,
}

impl LinkInputOrigin {
    pub fn name(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::CargoRegistry => "cargo-registry",
            Self::RustSysroot => "rust-sysroot",
            Self::BuildOutput => "build-output",
            Self::HostSystem => "host-system",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "workspace" => Some(Self::Workspace),
            "cargo-registry" => Some(Self::CargoRegistry),
            "rust-sysroot" => Some(Self::RustSysroot),
            "build-output" => Some(Self::BuildOutput),
            "host-system" => Some(Self::HostSystem),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LinkInput {
    pub origin: LinkInputOrigin,
    pub logical_path: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactObservation {
    pub byte_length: u64,
    pub sha256: String,
    pub elf_build_id: String,
    pub elf_interpreter: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostObservation {
    pub host_triple: String,
    pub os_release_sha256: String,
    pub environment_sha256: String,
    pub link_dependency_file_byte_length: u64,
    pub link_dependency_file_sha256: String,
    pub tools: Vec<ToolIdentity>,
    pub build_script_events: Vec<BuildScriptEvent>,
    pub final_link_inputs: Vec<LinkInput>,
    pub artifact: ArtifactObservation,
    pub dynamic_libraries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub authority: PortableAuthority,
    pub observation: HostObservation,
}

impl Receipt {
    pub fn new(authority: PortableAuthority, observation: HostObservation) -> Result<Self, String> {
        let receipt = Self {
            authority,
            observation,
        };
        receipt.validate()?;
        Ok(receipt)
    }

    pub fn portable_authority_sha256(&self) -> String {
        domain_sha256(
            b"semantic-fabric:portable-authority:v1",
            super::format::authority_records(&self.authority).as_bytes(),
        )
    }

    pub fn host_observation_sha256(&self) -> String {
        domain_sha256(
            b"semantic-fabric:host-observation:v1",
            super::format::observation_records(&self.observation).as_bytes(),
        )
    }

    pub fn receipt_sha256(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"semantic-fabric:artifact-observation-receipt:v1\0");
        digest.update(self.portable_authority_sha256().as_bytes());
        digest.update([0]);
        digest.update(self.host_observation_sha256().as_bytes());
        format!("{:x}", digest.finalize())
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        self.authority.validate()?;
        self.observation.validate()
    }
}

impl PortableAuthority {
    fn validate(&self) -> Result<(), String> {
        validate_git_revision(&self.git_revision)?;
        if self.source_date_epoch == 0 || self.source_date_epoch > i64::MAX as u64 {
            return Err("SOURCE_DATE_EPOCH is outside supported Unix timestamp bounds".to_owned());
        }
        for (label, value) in [
            ("source inputs", self.source_inputs_sha256.as_str()),
            ("Cargo.lock", self.cargo_lock_sha256.as_str()),
            ("rust-toolchain.toml", self.rust_toolchain_sha256.as_str()),
            (
                "controlled Cargo home",
                self.cargo_home_inputs_sha256.as_str(),
            ),
            (
                "Cargo configuration set",
                self.cargo_config_set_sha256.as_str(),
            ),
            ("closure receipt", self.closure_receipt_sha256.as_str()),
            ("current closure", self.current_closure_sha256.as_str()),
        ] {
            validate_sha256(label, value)?;
        }
        Ok(())
    }
}

impl HostObservation {
    fn validate(&self) -> Result<(), String> {
        if self.host_triple != TARGET {
            return Err(format!("host triple must be exactly {TARGET}"));
        }
        validate_sha256("os-release", &self.os_release_sha256)?;
        validate_sha256("build environment", &self.environment_sha256)?;
        validate_sha256(
            "final link dependency file",
            &self.link_dependency_file_sha256,
        )?;
        validate_nonempty_file(
            self.link_dependency_file_byte_length,
            "final link dependency file",
        )?;
        if self.build_script_events.len() > super::format::MAX_BUILD_SCRIPT_EVENTS
            || self.final_link_inputs.len() > super::format::MAX_FINAL_LINK_INPUTS
            || self.dynamic_libraries.len() > super::format::MAX_DYNAMIC_LIBRARIES
        {
            return Err("host observation inventory count exceeds bounds".to_owned());
        }
        validate_tools(&self.tools)?;
        validate_sorted(&self.build_script_events, "build-script events")?;
        validate_sorted(&self.final_link_inputs, "final link inputs")?;
        if self.build_script_events.is_empty() || self.final_link_inputs.is_empty() {
            return Err("build-script events and final link inputs must be observed".to_owned());
        }
        for event in &self.build_script_events {
            validate_text("build-script package id", &event.package_id, 512)?;
            validate_file_length(
                event.directives_source_byte_length,
                "build-script directives source",
            )?;
            validate_sha256("build-script directives", &event.directives_sha256)?;
            validate_file_length(event.stderr_byte_length, "build-script stderr")?;
            validate_sha256("build-script stderr", &event.stderr_sha256)?;
            if event.out_tree_file_count > super::format::MAX_BUILD_SCRIPT_TREE_FILES
                || event.out_tree_byte_length > 4 * 1024 * 1024 * 1024
            {
                return Err("build-script OUT tree is outside bounds".to_owned());
            }
            validate_sha256("build-script OUT tree", &event.out_tree_sha256)?;
        }
        for input in &self.final_link_inputs {
            validate_logical_path(&input.logical_path)?;
            validate_nonempty_file(input.byte_length, "final link input")?;
            validate_sha256("final link input", &input.sha256)?;
        }
        self.artifact.validate()?;
        validate_dynamic_libraries(&self.dynamic_libraries)
    }
}

impl ArtifactObservation {
    fn validate(&self) -> Result<(), String> {
        validate_nonempty_file(self.byte_length, "binary artifact")?;
        validate_sha256("binary artifact", &self.sha256)?;
        validate_hex("ELF build id", &self.elf_build_id, 16, 128)?;
        validate_text("ELF interpreter", &self.elf_interpreter, 512)?;
        let mut components = Path::new(&self.elf_interpreter).components();
        if !matches!(components.next(), Some(Component::RootDir))
            || components.any(|component| !matches!(component, Component::Normal(_)))
            || self.elf_interpreter.contains("//")
            || self.elf_interpreter.contains('\\')
        {
            return Err("ELF interpreter must be a normalized absolute path".to_owned());
        }
        Ok(())
    }
}

fn validate_tools(tools: &[ToolIdentity]) -> Result<(), String> {
    const EXPECTED: [ToolRole; 6] = [
        ToolRole::GitMaterializer,
        ToolRole::Cargo,
        ToolRole::Rustc,
        ToolRole::Sandbox,
        ToolRole::Linker,
        ToolRole::ElfReader,
    ];
    if tools.iter().map(|tool| tool.role).collect::<Vec<_>>() != EXPECTED {
        return Err(
            "tool identities must be exactly git-materializer, cargo, rustc, sandbox, linker, elf-reader in order"
                .to_owned(),
        );
    }
    for tool in tools {
        validate_logical_path(&tool.logical_path)?;
        validate_text("tool version", &tool.version, 512)?;
        validate_nonempty_file(tool.byte_length, "tool executable")?;
        validate_sha256("tool executable", &tool.sha256)?;
    }
    Ok(())
}

fn validate_dynamic_libraries(libraries: &[String]) -> Result<(), String> {
    if libraries.is_empty() {
        return Err("declared dynamic library observation must not be empty".to_owned());
    }
    validate_sorted(libraries, "dynamic libraries")?;
    for library in libraries {
        if library.len() > 128
            || !library
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
        {
            return Err(format!("invalid dynamic library name {library:?}"));
        }
    }
    Ok(())
}

fn validate_sorted<T: Ord>(values: &[T], label: &str) -> Result<(), String> {
    if !values.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(format!("{label} are not strictly ordered"));
    }
    Ok(())
}

pub(crate) fn validate_logical_path(value: &str) -> Result<(), String> {
    validate_text("logical path", value, 1024)?;
    let path = Path::new(value);
    if path.is_absolute()
        || value.contains("//")
        || value.contains('\\')
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("logical path is not normalized: {value:?}"));
    }
    Ok(())
}

pub(crate) fn validate_text(label: &str, value: &str, max: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || !byte.is_ascii())
    {
        Err(format!("invalid {label}"))
    } else {
        Ok(())
    }
}

pub(crate) fn validate_sha256(label: &str, value: &str) -> Result<(), String> {
    validate_hex(label, value, 64, 64)
}

fn validate_git_revision(value: &str) -> Result<(), String> {
    if matches!(value.len(), 40 | 64) {
        validate_hex("git revision", value, value.len(), value.len())
    } else {
        Err("git revision must be a full SHA-1 or SHA-256 object id".to_owned())
    }
}

fn validate_hex(label: &str, value: &str, min: usize, max: usize) -> Result<(), String> {
    if value.len() >= min
        && value.len() <= max
        && value.len().is_multiple_of(2)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(format!("invalid lowercase hexadecimal value for {label}"))
    }
}

fn validate_nonempty_file(length: u64, label: &str) -> Result<(), String> {
    if length == 0 || length > 4 * 1024 * 1024 * 1024 {
        Err(format!("{label} byte length is outside bounds"))
    } else {
        Ok(())
    }
}

fn validate_file_length(length: u64, label: &str) -> Result<(), String> {
    if length > 4 * 1024 * 1024 * 1024 {
        Err(format!("{label} byte length is outside bounds"))
    } else {
        Ok(())
    }
}

fn domain_sha256(domain: &[u8], bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain);
    digest.update([0]);
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}
