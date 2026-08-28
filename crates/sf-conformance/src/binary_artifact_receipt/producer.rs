//! End-to-end producer for one host-observed current `sf-cli` artifact receipt.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::rust_closure_receipt::{self, ControlledCheckRequest};

use super::model::{
    ArtifactObservation, BuildScriptEvent, HostObservation, LinkInput, LinkInputAlias,
    PortableAuthority, Receipt, ToolIdentity, ToolRole,
};
use super::{
    artifact_pair::ArtifactPair,
    authority, capture as build_script_capture, cargo, elf, linker,
    producer_paths::{SandboxPathMap, Workspace},
    sandbox, source,
};

const ROOT_PACKAGE_ID: &str = "path+file:///workspace/crates/sf-cli#0.0.0";
const OS_RELEASE: &str = "/usr/lib/os-release";
const MAX_LOCK_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TOOLCHAIN_BYTES: u64 = 1024 * 1024;
const MAX_CLOSURE_RECEIPT_BYTES: u64 = 2 * 1024 * 1024;

/// Exact host inputs for a fresh, external current-`sf-cli` observation.
#[derive(Debug, Clone, Copy)]
pub struct CaptureRequest<'a> {
    pub repository: &'a Path,
    pub git: &'a Path,
    pub bwrap: &'a Path,
    pub toolchain_root: &'a Path,
    pub cargo_home: &'a Path,
    pub readelf: &'a Path,
    pub scratch_root: &'a Path,
}

/// Builds and observes the current broad development binary. It neither writes
/// a receipt nor upgrades any production, release, or reproducibility claim.
pub fn capture(request: &CaptureRequest<'_>) -> Result<Receipt, String> {
    validate_bound_file(request.git, "Git materializer")?;
    validate_bound_file(request.bwrap, "bubblewrap executable")?;
    validate_bound_file(request.readelf, "readelf executable")?;
    validate_bound_file(Path::new(sandbox::LINKER), "linker executable")?;

    let workspace = Workspace::prepare(
        request.repository,
        request.scratch_root,
        request.toolchain_root,
        request.cargo_home,
    )?;
    let cargo_path = request.toolchain_root.join("bin/cargo");
    let rustc_path = request.toolchain_root.join("bin/rustc");
    let cargo_registry = request.cargo_home.join("registry");
    let tools = BoundTools::identify(
        request.git,
        &cargo_path,
        &rustc_path,
        request.bwrap,
        request.readelf,
    )?;
    let os_release = authority::read(
        Path::new(OS_RELEASE),
        1024 * 1024,
        "fixed /usr/lib/os-release",
    )?;

    workspace.assert_current()?;
    let snapshot = source::materialize(request.git, request.repository, &workspace.source)?;
    workspace.assert_current()?;
    let inputs = SourceInputs::read(&workspace.source)?;
    let cargo_config_set = source::empty_cargo_config_set(&workspace.source, request.cargo_home)?;
    let cargo_home_inputs = source::cargo_home_inputs(&cargo_registry)?;
    let closure = rust_closure_receipt::check_with_tools(&ControlledCheckRequest {
        materialized_source: &workspace.source,
        cargo: &cargo_path,
        rustc: &rustc_path,
        toolchain_root: request.toolchain_root,
        cargo_home: request.cargo_home,
        temporary_dir: &workspace.temporary,
        source_date_epoch: snapshot.source_date_epoch,
    })?;

    workspace.assert_current()?;
    let (build_output, plan) = sandbox::execute(&sandbox::Request {
        bwrap: request.bwrap,
        source: &workspace.source,
        toolchain: request.toolchain_root,
        cargo_registry: &cargo_registry,
        target: &workspace.target,
        source_date_epoch: snapshot.source_date_epoch,
    })?;
    workspace.assert_current()?;
    tools.require_sandbox_identity(&plan)?;
    let messages = cargo::parse_sandbox_stdout(&build_output.stdout, ROOT_PACKAGE_ID)?;
    if messages.package_id != ROOT_PACKAGE_ID {
        return Err("Cargo selected an unexpected root package identity".to_owned());
    }
    let path_map = SandboxPathMap::new(
        &workspace.source,
        &cargo_registry,
        request.toolchain_root,
        &workspace.target,
        &plan.system_mounts,
    )?;
    workspace.assert_current()?;
    let artifact_path = path_map.map_target(&messages.logical_artifact)?;
    let build_scripts = build_script_events(build_script_capture::inventory(
        &workspace.target,
        &path_map,
        &messages.build_scripts,
    )?)?;
    let dependency_file = linker::capture(&workspace.target.join("final-link.d"), &path_map)?;
    let artifact_pair = ArtifactPair::bind(&artifact_path, &dependency_file.output_path)?;
    let link_inputs = link_inputs(&dependency_file.inputs)?;
    let link_input_aliases = link_input_aliases(&dependency_file.aliases);
    let (readelf_identity, elf_observation) = elf::inspect(&artifact_pair, request.readelf)?;
    tools.require_readelf_identity(&readelf_identity)?;
    workspace.assert_current()?;
    artifact_pair.assert_current()?;
    dependency_file.assert_current(&path_map)?;

    stabilize(
        request,
        &snapshot,
        &inputs,
        &tools,
        &os_release,
        &cargo_config_set,
        &cargo_home_inputs,
    )?;
    workspace.assert_current()?;
    artifact_pair.assert_current()?;
    dependency_file.assert_current(&path_map)?;

    Receipt::new(
        PortableAuthority {
            git_revision: snapshot.revision,
            source_date_epoch: snapshot.source_date_epoch,
            source_inputs_sha256: snapshot.source_inputs_sha256,
            cargo_lock_sha256: inputs.cargo_lock.sha256.clone(),
            rust_toolchain_sha256: inputs.rust_toolchain.sha256.clone(),
            cargo_home_inputs_sha256: cargo_home_inputs,
            cargo_config_set_sha256: cargo_config_set,
            closure_receipt_sha256: inputs.closure_receipt.sha256.clone(),
            current_closure_sha256: closure.closure_sha256().to_owned(),
        },
        HostObservation {
            host_triple: tools.rustc.host.clone(),
            os_release_sha256: os_release.sha256,
            environment_sha256: plan.environment.sha256(),
            link_dependency_file_byte_length: dependency_file.byte_length,
            link_dependency_file_sha256: dependency_file.sha256,
            link_output_logical_path: dependency_file.receipt_output,
            raw_link_input_count: dependency_file.raw_input_count as u64,
            tools: tools.receipt_identities(),
            build_script_events: build_scripts,
            final_link_inputs: link_inputs,
            final_link_input_aliases: link_input_aliases,
            artifact: ArtifactObservation {
                byte_length: elf_observation.artifact_size,
                sha256: elf_observation.artifact_sha256,
                elf_build_id: elf_observation.build_id,
                elf_interpreter: elf_observation.interpreter,
            },
            dynamic_libraries: elf_observation.needed,
        },
    )
}

fn stabilize(
    request: &CaptureRequest<'_>,
    snapshot: &source::Snapshot,
    inputs: &SourceInputs,
    tools: &BoundTools,
    os_release: &authority::AuthorityFile,
    cargo_config_set: &str,
    cargo_home_inputs: &str,
) -> Result<(), String> {
    source::assert_unchanged(request.git, request.repository, snapshot)?;
    inputs.ensure_unchanged(&snapshot.materialized_root)?;
    if source::empty_cargo_config_set(&snapshot.materialized_root, request.cargo_home)?
        != cargo_config_set
    {
        return Err("Cargo configuration authority changed during capture".to_owned());
    }
    if source::cargo_home_inputs(&request.cargo_home.join("registry"))? != cargo_home_inputs {
        return Err("controlled Cargo registry changed during capture".to_owned());
    }
    let os_release_after = authority::read(
        Path::new(OS_RELEASE),
        1024 * 1024,
        "fixed /usr/lib/os-release",
    )?;
    if os_release.bytes != os_release_after.bytes {
        return Err("fixed /usr/lib/os-release changed during capture".to_owned());
    }
    let after = BoundTools::identify(
        request.git,
        &request.toolchain_root.join("bin/cargo"),
        &request.toolchain_root.join("bin/rustc"),
        request.bwrap,
        request.readelf,
    )?;
    if tools != &after {
        return Err("one or more bound tools changed during capture".to_owned());
    }
    Ok(())
}

struct SourceInputs {
    cargo_lock: authority::AuthorityFile,
    rust_toolchain: authority::AuthorityFile,
    closure_receipt: authority::AuthorityFile,
}

impl SourceInputs {
    fn read(source: &Path) -> Result<Self, String> {
        Ok(Self {
            cargo_lock: authority::read(
                &source.join("Cargo.lock"),
                MAX_LOCK_BYTES,
                "materialized Cargo.lock",
            )?,
            rust_toolchain: authority::read(
                &source.join("rust-toolchain.toml"),
                MAX_TOOLCHAIN_BYTES,
                "materialized rust-toolchain.toml",
            )?,
            closure_receipt: authority::read(
                &source.join(rust_closure_receipt::RECEIPT_PATH),
                MAX_CLOSURE_RECEIPT_BYTES,
                "materialized closure receipt",
            )?,
        })
    }

    fn ensure_unchanged(&self, source: &Path) -> Result<(), String> {
        let after = Self::read(source)?;
        for (label, before, current) in [
            ("Cargo.lock", &self.cargo_lock, &after.cargo_lock),
            (
                "rust-toolchain.toml",
                &self.rust_toolchain,
                &after.rust_toolchain,
            ),
            (
                "closure receipt",
                &self.closure_receipt,
                &after.closure_receipt,
            ),
        ] {
            if before.bytes != current.bytes {
                return Err(format!("materialized {label} changed during capture"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BoundTools {
    git: cargo::Executable,
    cargo: cargo::Executable,
    rustc: cargo::RustcIdentity,
    bwrap: cargo::Executable,
    linker: cargo::Executable,
    readelf: cargo::Executable,
}

impl BoundTools {
    fn identify(
        git: &Path,
        cargo_path: &Path,
        rustc: &Path,
        bwrap: &Path,
        readelf: &Path,
    ) -> Result<Self, String> {
        Ok(Self {
            git: cargo::identify(git, "Git materializer")?,
            cargo: cargo::identify(cargo_path, "Cargo executable")?,
            rustc: cargo::identify_rustc(rustc)?,
            bwrap: cargo::identify(bwrap, "bubblewrap executable")?,
            linker: cargo::identify(Path::new(sandbox::LINKER), "linker executable")?,
            readelf: cargo::identify(readelf, "readelf executable")?,
        })
    }

    fn require_sandbox_identity(&self, plan: &sandbox::Plan) -> Result<(), String> {
        if plan.executable != self.bwrap.path
            || plan.bwrap_sha256 != self.bwrap.sha256
            || plan.bwrap_byte_length != self.bwrap.byte_length
            || plan.bwrap_version != self.bwrap.version
        {
            Err("sandbox plan does not bind the pre-observed bubblewrap tool".to_owned())
        } else {
            Ok(())
        }
    }

    fn require_readelf_identity(&self, observed: &elf::ReadelfIdentity) -> Result<(), String> {
        if observed.path != self.readelf.path
            || observed.sha256 != self.readelf.sha256
            || observed.size != self.readelf.byte_length
            || observed.version != self.readelf.version
        {
            Err("ELF observation does not bind the pre-observed readelf tool".to_owned())
        } else {
            Ok(())
        }
    }

    fn receipt_identities(&self) -> Vec<ToolIdentity> {
        [
            (ToolRole::GitMaterializer, "host-tools/git", &self.git),
            (ToolRole::Cargo, "toolchain/bin/cargo", &self.cargo),
            (
                ToolRole::Rustc,
                "toolchain/bin/rustc",
                &self.rustc.executable,
            ),
            (ToolRole::Sandbox, "host-tools/bwrap", &self.bwrap),
            (
                ToolRole::Linker,
                "host-system/usr/bin/x86_64-linux-gnu-gcc-13",
                &self.linker,
            ),
            (ToolRole::ElfReader, "host-tools/readelf", &self.readelf),
        ]
        .into_iter()
        .map(|(role, logical_path, tool)| ToolIdentity {
            role,
            logical_path: logical_path.to_owned(),
            version: tool.version.clone(),
            byte_length: tool.byte_length,
            sha256: tool.sha256.clone(),
        })
        .collect()
    }
}

fn build_script_events(
    inventories: Vec<build_script_capture::BuildScriptInventory>,
) -> Result<Vec<BuildScriptEvent>, String> {
    let mut events = Vec::with_capacity(inventories.len());
    for inventory in inventories {
        events.push(BuildScriptEvent {
            package_id: inventory.package_id,
            logical_out_dir: inventory.receipt_out_dir,
            directives_source_byte_length: inventory.directives_bytes,
            directives_sha256: inventory.directives_sha256,
            stderr_byte_length: inventory.stderr_bytes,
            stderr_sha256: inventory.stderr_sha256,
            out_tree_file_count: inventory.out_file_count as u64,
            out_tree_byte_length: inventory.out_bytes,
            out_tree_sha256: inventory.out_tree_sha256,
        });
    }
    events.sort();
    Ok(events)
}

fn link_inputs(inputs: &[linker::Input]) -> Result<Vec<LinkInput>, String> {
    let mut receipt_paths = BTreeSet::new();
    let mut observed = Vec::with_capacity(inputs.len());
    for input in inputs {
        if !receipt_paths.insert(input.receipt_path.clone()) {
            return Err("link input receipt identity collision".to_owned());
        }
        observed.push(LinkInput {
            origin: input.origin,
            logical_path: input.receipt_path.clone(),
            byte_length: input.byte_length,
            sha256: input.sha256.clone(),
        });
    }
    observed.sort();
    Ok(observed)
}

fn link_input_aliases(aliases: &[linker::InputAlias]) -> Vec<LinkInputAlias> {
    aliases
        .iter()
        .map(|alias| LinkInputAlias {
            alias_logical_path: alias.alias_receipt_path.clone(),
            terminal_logical_path: alias.terminal_receipt_path.clone(),
            hop_count: alias.hop_count,
            resolution_sha256: alias.resolution_sha256.clone(),
        })
        .collect()
}

fn validate_bound_file(path: &Path, label: &str) -> Result<PathBuf, String> {
    validate_absolute_normal(path, label)?;
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("canonicalize {label} {}: {error}", path.display()))?;
    if canonical != path {
        return Err(format!("{label} path is not canonical"));
    }
    let _ = authority::digest(path, 2 * 1024 * 1024 * 1024, label)?;
    Ok(canonical)
}

fn validate_absolute_normal(path: &Path, label: &str) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
    {
        Err(format!("{label} must be absolute and normalized"))
    } else {
        Ok(())
    }
}
