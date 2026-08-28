use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::model::{
    validate_sha256, validate_text, ArtifactObservation, BuildScriptEvent, HostObservation,
    LinkInput, LinkInputOrigin, PortableAuthority, Receipt, ToolIdentity, ToolRole, ARTIFACT_CLASS,
    ARTIFACT_PATH, ATTESTATION_SCOPE, AUTHORITY_MODEL, BUILD_SCRIPT_DIRECTIVES_FORMAT,
    BUILD_SCRIPT_OUT_TREE_FORMAT, COMMAND_TEMPLATE, ENVIRONMENT_DIGEST_FORMAT, ENVIRONMENT_POLICY,
    HEADER, NONCLAIM_KEYS, NOT_ATTESTED, PROFILE, ROOT_BINARY, ROOT_MANIFEST, ROOT_PACKAGE,
    SOURCE_DATE_EPOCH_LAW, TARGET,
};

pub const MAX_RECEIPT_BYTES: usize = 4 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 4096;
pub(super) const MAX_BUILD_SCRIPT_EVENTS: usize = 2048;
pub(super) const MAX_BUILD_SCRIPT_TREE_FILES: u64 = 65_536;
pub(super) const MAX_FINAL_LINK_INPUTS: usize = 16_384;
pub(super) const MAX_DYNAMIC_LIBRARIES: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    Host,
    Tool,
    BuildScript,
    LinkInput,
    Artifact,
    DynamicLibrary,
}

pub fn render(receipt: &Receipt) -> Result<String, String> {
    receipt.validate()?;
    let mut output = String::new();
    writeln!(output, "{HEADER}").expect("String writes cannot fail");
    for (key, value) in metadata(receipt) {
        writeln!(output, "meta\t{key}\t{value}").expect("String writes cannot fail");
    }
    output.push_str(&observation_records(&receipt.observation));
    validate_text_shape(&output)?;
    Ok(output)
}

pub fn parse(input: &str) -> Result<Receipt, String> {
    validate_text_shape(input)?;
    if input.contains('\r') {
        return Err("receipt must use LF line endings".to_owned());
    }
    if !input.ends_with('\n') {
        return Err("receipt must end with one LF".to_owned());
    }
    let mut lines = input.lines().enumerate();
    if lines.next().map(|(_, line)| line) != Some(HEADER) {
        return Err("invalid current sf-cli artifact observation header".to_owned());
    }
    let mut metadata = BTreeMap::new();
    let mut stage = None;
    let mut host = None;
    let mut tools = Vec::new();
    let mut build_script_events = Vec::new();
    let mut final_link_inputs = Vec::new();
    let mut artifact = None;
    let mut dynamic_libraries = Vec::new();
    for (index, line) in lines {
        let number = index + 1;
        let fields: Vec<_> = line.split('\t').collect();
        match fields.as_slice() {
            ["meta", key, value] if stage.is_none() => {
                validate_text("metadata key", key, 64)?;
                validate_text("metadata value", value, 1024)?;
                if metadata.insert(*key, *value).is_some() {
                    return Err(format!("line {number}: duplicate metadata key {key}"));
                }
            }
            ["host", host_triple, os_release, environment, link_depfile_length, link_depfile] => {
                advance(&mut stage, Stage::Host, number)?;
                if host
                    .replace((
                        *host_triple,
                        *os_release,
                        *environment,
                        parse_length(link_depfile_length, number)?,
                        *link_depfile,
                    ))
                    .is_some()
                {
                    return Err(format!("line {number}: duplicate host observation"));
                }
            }
            ["tool", role, path, version, length, digest] => {
                advance(&mut stage, Stage::Tool, number)?;
                if tools.len() == 6 {
                    return Err(format!("line {number}: too many tool identities"));
                }
                tools.push(ToolIdentity {
                    role: ToolRole::parse(role)
                        .ok_or_else(|| format!("line {number}: unknown tool role {role:?}"))?,
                    logical_path: (*path).to_owned(),
                    version: (*version).to_owned(),
                    byte_length: parse_length(length, number)?,
                    sha256: (*digest).to_owned(),
                });
            }
            ["build-script-event", package, directives_length, directives_digest, stderr_length, stderr_digest, out_file_count, out_tree_length, out_tree_digest] =>
            {
                advance(&mut stage, Stage::BuildScript, number)?;
                if build_script_events.len() == MAX_BUILD_SCRIPT_EVENTS {
                    return Err(format!("line {number}: too many build-script events"));
                }
                build_script_events.push(BuildScriptEvent {
                    package_id: (*package).to_owned(),
                    directives_source_byte_length: parse_length(directives_length, number)?,
                    directives_sha256: (*directives_digest).to_owned(),
                    stderr_byte_length: parse_length(stderr_length, number)?,
                    stderr_sha256: (*stderr_digest).to_owned(),
                    out_tree_file_count: parse_length(out_file_count, number)?,
                    out_tree_byte_length: parse_length(out_tree_length, number)?,
                    out_tree_sha256: (*out_tree_digest).to_owned(),
                });
            }
            ["link-input", origin, path, length, digest] => {
                advance(&mut stage, Stage::LinkInput, number)?;
                if final_link_inputs.len() == MAX_FINAL_LINK_INPUTS {
                    return Err(format!("line {number}: too many final link inputs"));
                }
                final_link_inputs.push(LinkInput {
                    origin: LinkInputOrigin::parse(origin)
                        .ok_or_else(|| format!("line {number}: unknown link input origin"))?,
                    logical_path: (*path).to_owned(),
                    byte_length: parse_length(length, number)?,
                    sha256: (*digest).to_owned(),
                });
            }
            ["artifact", path, length, digest, "0755", "elf64", "little", "x86-64", "system-v", interpreter, build_id] =>
            {
                advance(&mut stage, Stage::Artifact, number)?;
                if *path != ARTIFACT_PATH {
                    return Err(format!("line {number}: unexpected artifact logical path"));
                }
                if artifact.is_some() {
                    return Err(format!("line {number}: duplicate artifact observation"));
                }
                artifact = Some(ArtifactObservation {
                    byte_length: parse_length(length, number)?,
                    sha256: (*digest).to_owned(),
                    elf_build_id: (*build_id).to_owned(),
                    elf_interpreter: (*interpreter).to_owned(),
                });
            }
            ["artifact", ..] => {
                return Err(format!("line {number}: invalid fixed ELF artifact shape"));
            }
            ["dynamic-library", soname] => {
                advance(&mut stage, Stage::DynamicLibrary, number)?;
                if dynamic_libraries.len() == MAX_DYNAMIC_LIBRARIES {
                    return Err(format!("line {number}: too many dynamic libraries"));
                }
                dynamic_libraries.push((*soname).to_owned());
            }
            ["meta", ..] => return Err(format!("line {number}: metadata follows records")),
            _ => return Err(format!("line {number}: unknown or malformed record")),
        }
    }
    let authority = PortableAuthority {
        git_revision: take(&mut metadata, "git-revision")?.to_owned(),
        source_date_epoch: parse_u64_metadata(
            take(&mut metadata, "source-date-epoch")?,
            "source-date-epoch",
        )?,
        source_inputs_sha256: take(&mut metadata, "source-inputs-sha256")?.to_owned(),
        cargo_lock_sha256: take(&mut metadata, "cargo-lock-sha256")?.to_owned(),
        rust_toolchain_sha256: take(&mut metadata, "rust-toolchain-sha256")?.to_owned(),
        cargo_home_inputs_sha256: take(&mut metadata, "cargo-home-inputs-sha256")?.to_owned(),
        cargo_config_set_sha256: take(&mut metadata, "cargo-config-set-sha256")?.to_owned(),
        closure_receipt_sha256: take(&mut metadata, "closure-receipt-sha256")?.to_owned(),
        current_closure_sha256: take(&mut metadata, "current-closure-sha256")?.to_owned(),
    };
    let (
        host_triple,
        os_release_sha256,
        environment_sha256,
        link_dependency_file_byte_length,
        link_dependency_file_sha256,
    ) = host.ok_or_else(|| "missing host observation".to_owned())?;
    let observation = HostObservation {
        host_triple: host_triple.to_owned(),
        os_release_sha256: os_release_sha256.to_owned(),
        environment_sha256: environment_sha256.to_owned(),
        link_dependency_file_byte_length,
        link_dependency_file_sha256: link_dependency_file_sha256.to_owned(),
        tools,
        build_script_events,
        final_link_inputs,
        artifact: artifact.ok_or_else(|| "missing artifact observation".to_owned())?,
        dynamic_libraries,
    };
    let counts = [
        ("tool-count", observation.tools.len()),
        (
            "build-script-event-count",
            observation.build_script_events.len(),
        ),
        (
            "final-link-input-count",
            observation.final_link_inputs.len(),
        ),
        ("dynamic-library-count", observation.dynamic_libraries.len()),
    ];
    for (key, observed) in counts {
        if parse_count(take(&mut metadata, key)?, key)? != observed {
            return Err(format!("receipt {key} does not match records"));
        }
    }
    enforce_fixed_metadata(&mut metadata)?;
    let expected_authority = take(&mut metadata, "portable-authority-sha256")?.to_owned();
    let expected_observation = take(&mut metadata, "host-observation-sha256")?.to_owned();
    let expected_receipt = take(&mut metadata, "receipt-sha256")?.to_owned();
    for (label, digest) in [
        ("portable authority", expected_authority.as_str()),
        ("host observation", expected_observation.as_str()),
        ("receipt", expected_receipt.as_str()),
    ] {
        validate_sha256(label, digest)?;
    }
    if let Some(key) = metadata.keys().next() {
        return Err(format!("unknown receipt metadata key {key}"));
    }
    let receipt = Receipt::new(authority, observation)?;
    if expected_authority != receipt.portable_authority_sha256()
        || expected_observation != receipt.host_observation_sha256()
        || expected_receipt != receipt.receipt_sha256()
    {
        return Err("artifact observation digest drift".to_owned());
    }
    if render(&receipt)? != input {
        return Err("receipt is valid but not in canonical generated form".to_owned());
    }
    Ok(receipt)
}

pub(super) fn authority_records(authority: &PortableAuthority) -> String {
    format!(
        "git-revision\t{}\nsource-date-epoch\t{}\nsource-inputs-sha256\t{}\ncargo-lock-sha256\t{}\nrust-toolchain-sha256\t{}\ncargo-home-inputs-sha256\t{}\ncargo-config-set-sha256\t{}\nclosure-receipt-sha256\t{}\ncurrent-closure-sha256\t{}\n",
        authority.git_revision,
        authority.source_date_epoch,
        authority.source_inputs_sha256,
        authority.cargo_lock_sha256,
        authority.rust_toolchain_sha256,
        authority.cargo_home_inputs_sha256,
        authority.cargo_config_set_sha256,
        authority.closure_receipt_sha256,
        authority.current_closure_sha256,
    )
}

pub(super) fn observation_records(observation: &HostObservation) -> String {
    let mut output = String::new();
    writeln!(
        output,
        "host\t{}\t{}\t{}\t{}\t{}",
        observation.host_triple,
        observation.os_release_sha256,
        observation.environment_sha256,
        observation.link_dependency_file_byte_length,
        observation.link_dependency_file_sha256,
    )
    .expect("String writes cannot fail");
    for tool in &observation.tools {
        writeln!(
            output,
            "tool\t{}\t{}\t{}\t{}\t{}",
            tool.role.name(),
            tool.logical_path,
            tool.version,
            tool.byte_length,
            tool.sha256
        )
        .expect("String writes cannot fail");
    }
    for item in &observation.build_script_events {
        writeln!(
            output,
            "build-script-event\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            item.package_id,
            item.directives_source_byte_length,
            item.directives_sha256,
            item.stderr_byte_length,
            item.stderr_sha256,
            item.out_tree_file_count,
            item.out_tree_byte_length,
            item.out_tree_sha256
        )
        .expect("String writes cannot fail");
    }
    for item in &observation.final_link_inputs {
        writeln!(
            output,
            "link-input\t{}\t{}\t{}\t{}",
            item.origin.name(),
            item.logical_path,
            item.byte_length,
            item.sha256
        )
        .expect("String writes cannot fail");
    }
    let artifact = &observation.artifact;
    writeln!(
        output,
        "artifact\t{ARTIFACT_PATH}\t{}\t{}\t0755\telf64\tlittle\tx86-64\tsystem-v\t{}\t{}",
        artifact.byte_length, artifact.sha256, artifact.elf_interpreter, artifact.elf_build_id
    )
    .expect("String writes cannot fail");
    for library in &observation.dynamic_libraries {
        writeln!(output, "dynamic-library\t{library}").expect("String writes cannot fail");
    }
    output
}

fn metadata(receipt: &Receipt) -> Vec<(&'static str, String)> {
    let authority = &receipt.authority;
    let observation = &receipt.observation;
    let mut values = vec![
        ("artifact-class", ARTIFACT_CLASS.to_owned()),
        ("attestation-scope", ATTESTATION_SCOPE.to_owned()),
        ("authority-model", AUTHORITY_MODEL.to_owned()),
        ("hash-algorithm", "sha256".to_owned()),
        ("root-manifest", ROOT_MANIFEST.to_owned()),
        ("root-package", ROOT_PACKAGE.to_owned()),
        ("root-binary", ROOT_BINARY.to_owned()),
        ("profile", PROFILE.to_owned()),
        ("target", TARGET.to_owned()),
        ("build-command-template", COMMAND_TEMPLATE.to_owned()),
        ("environment-policy", ENVIRONMENT_POLICY.to_owned()),
        (
            "environment-digest-format",
            ENVIRONMENT_DIGEST_FORMAT.to_owned(),
        ),
        ("source-date-epoch-law", SOURCE_DATE_EPOCH_LAW.to_owned()),
        (
            "build-script-directives-format",
            BUILD_SCRIPT_DIRECTIVES_FORMAT.to_owned(),
        ),
        (
            "build-script-out-tree-format",
            BUILD_SCRIPT_OUT_TREE_FORMAT.to_owned(),
        ),
        ("git-revision", authority.git_revision.clone()),
        ("source-date-epoch", authority.source_date_epoch.to_string()),
        (
            "source-inputs-sha256",
            authority.source_inputs_sha256.clone(),
        ),
        ("cargo-lock-sha256", authority.cargo_lock_sha256.clone()),
        (
            "rust-toolchain-sha256",
            authority.rust_toolchain_sha256.clone(),
        ),
        (
            "cargo-home-inputs-sha256",
            authority.cargo_home_inputs_sha256.clone(),
        ),
        (
            "cargo-config-set-sha256",
            authority.cargo_config_set_sha256.clone(),
        ),
        (
            "closure-receipt-sha256",
            authority.closure_receipt_sha256.clone(),
        ),
        (
            "current-closure-sha256",
            authority.current_closure_sha256.clone(),
        ),
    ];
    values.extend(NONCLAIM_KEYS.map(|key| (key, NOT_ATTESTED.to_owned())));
    values.extend([
        ("tool-count", observation.tools.len().to_string()),
        (
            "build-script-event-count",
            observation.build_script_events.len().to_string(),
        ),
        (
            "final-link-input-count",
            observation.final_link_inputs.len().to_string(),
        ),
        (
            "dynamic-library-count",
            observation.dynamic_libraries.len().to_string(),
        ),
        (
            "portable-authority-sha256",
            receipt.portable_authority_sha256(),
        ),
        ("host-observation-sha256", receipt.host_observation_sha256()),
        ("receipt-sha256", receipt.receipt_sha256()),
    ]);
    values
}

fn enforce_fixed_metadata<'a>(metadata: &mut BTreeMap<&'a str, &'a str>) -> Result<(), String> {
    for (key, expected) in [
        ("artifact-class", ARTIFACT_CLASS),
        ("attestation-scope", ATTESTATION_SCOPE),
        ("authority-model", AUTHORITY_MODEL),
        ("hash-algorithm", "sha256"),
        ("root-manifest", ROOT_MANIFEST),
        ("root-package", ROOT_PACKAGE),
        ("root-binary", ROOT_BINARY),
        ("profile", PROFILE),
        ("target", TARGET),
        ("build-command-template", COMMAND_TEMPLATE),
        ("environment-policy", ENVIRONMENT_POLICY),
        ("environment-digest-format", ENVIRONMENT_DIGEST_FORMAT),
        ("source-date-epoch-law", SOURCE_DATE_EPOCH_LAW),
        (
            "build-script-directives-format",
            BUILD_SCRIPT_DIRECTIVES_FORMAT,
        ),
        ("build-script-out-tree-format", BUILD_SCRIPT_OUT_TREE_FORMAT),
    ] {
        expect(metadata, key, expected)?;
    }
    for key in NONCLAIM_KEYS {
        expect(metadata, key, NOT_ATTESTED)?;
    }
    Ok(())
}

fn advance(stage: &mut Option<Stage>, next: Stage, line: usize) -> Result<(), String> {
    if stage.is_some_and(|current| current > next) {
        return Err(format!(
            "line {line}: record groups are not in canonical order"
        ));
    }
    *stage = Some(next);
    Ok(())
}

fn validate_text_shape(input: &str) -> Result<(), String> {
    if input.is_empty() || input.len() > MAX_RECEIPT_BYTES {
        return Err("artifact observation receipt size is outside bounds".to_owned());
    }
    if input.lines().any(|line| line.len() > MAX_LINE_BYTES) {
        return Err(format!("receipt line exceeds {MAX_LINE_BYTES} bytes"));
    }
    Ok(())
}

fn parse_length(value: &str, line: usize) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("line {line}: invalid byte length"))
}

fn parse_count(value: &str, key: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("invalid receipt count {key}"))
}

fn parse_u64_metadata(value: &str, key: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("invalid receipt integer {key}"))
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
