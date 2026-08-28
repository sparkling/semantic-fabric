use std::fmt::Write as _;

use super::{HostObservation, PortableAuthority, ARTIFACT_PATH};

pub(in crate::binary_artifact_receipt) fn authority_records(
    authority: &PortableAuthority,
) -> String {
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

pub(in crate::binary_artifact_receipt) fn observation_records(
    observation: &HostObservation,
) -> String {
    let mut output = String::new();
    writeln!(
        output,
        "host\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        observation.host_triple,
        observation.os_release_sha256,
        observation.environment_sha256,
        observation.link_dependency_file_byte_length,
        observation.link_dependency_file_sha256,
        observation.link_output_logical_path,
        observation.raw_link_input_count,
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
            "build-script-event\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            item.package_id,
            item.logical_out_dir,
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
    for alias in &observation.final_link_input_aliases {
        writeln!(
            output,
            "link-input-alias\t{}\t{}\t{}\t{}",
            alias.alias_logical_path,
            alias.terminal_logical_path,
            alias.hop_count,
            alias.resolution_sha256,
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
