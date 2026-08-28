use super::*;
use super::{format, model};

fn digest(byte: char) -> String {
    std::iter::repeat_n(byte, 64).collect()
}

fn authority() -> PortableAuthority {
    PortableAuthority {
        git_revision: "0123456789abcdef0123456789abcdef01234567".to_owned(),
        source_date_epoch: 1_787_875_200,
        source_inputs_sha256: digest('1'),
        cargo_lock_sha256: digest('2'),
        rust_toolchain_sha256: digest('3'),
        cargo_home_inputs_sha256: digest('4'),
        cargo_config_set_sha256: digest('5'),
        closure_receipt_sha256: digest('6'),
        current_closure_sha256: digest('7'),
    }
}

fn observation() -> HostObservation {
    HostObservation {
        host_triple: "x86_64-unknown-linux-gnu".to_owned(),
        os_release_sha256: digest('8'),
        environment_sha256: digest('9'),
        link_dependency_file_byte_length: 128,
        link_dependency_file_sha256: digest('a'),
        tools: vec![
            ToolIdentity {
                role: ToolRole::GitMaterializer,
                logical_path: "host-system/usr/bin/git".to_owned(),
                version: "git version 2.53.0 (fixture)".to_owned(),
                byte_length: 10,
                sha256: digest('a'),
            },
            ToolIdentity {
                role: ToolRole::Cargo,
                logical_path: "rust-toolchain/bin/cargo".to_owned(),
                version: "cargo 1.96.0 (fixture)".to_owned(),
                byte_length: 11,
                sha256: digest('b'),
            },
            ToolIdentity {
                role: ToolRole::Rustc,
                logical_path: "rust-toolchain/bin/rustc".to_owned(),
                version: "rustc 1.96.0 (fixture)".to_owned(),
                byte_length: 12,
                sha256: digest('c'),
            },
            ToolIdentity {
                role: ToolRole::Sandbox,
                logical_path: "host-system/usr/bin/bwrap".to_owned(),
                version: "bubblewrap 0.11.0 (fixture)".to_owned(),
                byte_length: 13,
                sha256: digest('d'),
            },
            ToolIdentity {
                role: ToolRole::Linker,
                logical_path: "host-system/usr/bin/cc".to_owned(),
                version: "cc (fixture) 1.0".to_owned(),
                byte_length: 14,
                sha256: digest('d'),
            },
            ToolIdentity {
                role: ToolRole::ElfReader,
                logical_path: "host-system/usr/bin/readelf".to_owned(),
                version: "GNU readelf 2.45 (fixture)".to_owned(),
                byte_length: 15,
                sha256: digest('e'),
            },
        ],
        build_script_events: vec![BuildScriptEvent {
            package_id: "libsqlite3-sys@0.38.0".to_owned(),
            directives_source_byte_length: 32,
            directives_sha256: digest('e'),
            stderr_byte_length: 0,
            stderr_sha256: digest('f'),
            out_tree_file_count: 0,
            out_tree_byte_length: 0,
            out_tree_sha256: digest('0'),
        }],
        final_link_inputs: vec![LinkInput {
            origin: LinkInputOrigin::HostSystem,
            logical_path: "host-system/usr/lib/crt1.o".to_owned(),
            byte_length: 64,
            sha256: digest('f'),
        }],
        artifact: ArtifactObservation {
            byte_length: 1024,
            sha256: digest('0'),
            elf_build_id: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            elf_interpreter: "/lib64/ld-linux-x86-64.so.2".to_owned(),
        },
        dynamic_libraries: vec!["libc.so.6".to_owned(), "libgcc_s.so.1".to_owned()],
    }
}

fn receipt() -> Receipt {
    Receipt::new(authority(), observation()).unwrap()
}

#[test]
fn canonical_receipt_round_trips_and_binds_two_domains() {
    let expected = receipt();
    let rendered = render(&expected).unwrap();
    let parsed = parse(&rendered).unwrap();

    assert_eq!(parsed, expected);
    assert_eq!(render(&parsed).unwrap(), rendered);
    assert!(rendered.starts_with("semantic-fabric-current-sf-cli-artifact-observation-v1\n"));
    for key in model::NONCLAIM_KEYS {
        assert!(rendered.contains(&format!("meta\t{key}\tnot-attested\n")));
    }
}

#[test]
fn same_principal_authority_race_resistance_is_an_explicit_nonclaim() {
    let rendered = render(&receipt()).unwrap();
    assert!(rendered.contains("meta\tsame-principal-authority-race-resistance\tnot-attested\n"));
}

#[test]
fn host_changes_do_not_rewrite_portable_authority_identity() {
    let first = receipt();
    let mut changed_observation = observation();
    changed_observation.artifact.sha256 = digest('a');
    let second = Receipt::new(authority(), changed_observation).unwrap();

    assert_eq!(
        first.portable_authority_sha256(),
        second.portable_authority_sha256()
    );
    assert_ne!(
        first.host_observation_sha256(),
        second.host_observation_sha256()
    );
    assert_ne!(first.receipt_sha256(), second.receipt_sha256());
}

#[test]
fn source_changes_rewrite_only_the_portable_domain_and_aggregate() {
    let first = receipt();
    let mut changed_authority = authority();
    changed_authority.source_inputs_sha256 = digest('a');
    let second = Receipt::new(changed_authority, observation()).unwrap();

    assert_ne!(
        first.portable_authority_sha256(),
        second.portable_authority_sha256()
    );
    assert_eq!(
        first.host_observation_sha256(),
        second.host_observation_sha256()
    );
    assert_ne!(first.receipt_sha256(), second.receipt_sha256());
}

#[test]
fn rejects_duplicate_and_unknown_metadata() {
    let rendered = render(&receipt()).unwrap();
    let duplicate = rendered.replacen(
        "meta\tartifact-class\tcurrent-sf-cli-all-in-one-development\n",
        concat!(
            "meta\tartifact-class\tcurrent-sf-cli-all-in-one-development\n",
            "meta\tartifact-class\tcurrent-sf-cli-all-in-one-development\n"
        ),
        1,
    );
    assert!(parse(&duplicate)
        .unwrap_err()
        .contains("duplicate metadata"));

    let unknown = rendered.replacen(
        "meta\tartifact-class",
        "meta\tunexpected\tvalue\nmeta\tartifact-class",
        1,
    );
    assert!(parse(&unknown)
        .unwrap_err()
        .contains("unknown receipt metadata"));
}

#[test]
fn rejects_crlf_control_bytes_and_missing_final_lf() {
    let rendered = render(&receipt()).unwrap();
    assert!(parse(&rendered.replace('\n', "\r\n"))
        .unwrap_err()
        .contains("LF line endings"));
    let control_error = parse(&rendered.replace("cc (fixture)", "cc\0(fixture)")).unwrap_err();
    assert!(
        control_error.contains("invalid metadata value")
            || control_error.contains("invalid tool version")
    );
    assert!(parse(rendered.trim_end_matches('\n'))
        .unwrap_err()
        .contains("end with one LF"));
}

#[test]
fn rejects_every_attempt_to_promote_a_nonclaim() {
    let rendered = render(&receipt()).unwrap();
    for key in model::NONCLAIM_KEYS {
        let promoted = rendered.replace(
            &format!("meta\t{key}\tnot-attested"),
            &format!("meta\t{key}\tattested"),
        );
        let error = parse(&promoted).unwrap_err();
        assert!(error.contains(key), "{key}: {error}");
    }
}

#[test]
fn rejects_artifact_and_record_digest_drift() {
    let rendered = render(&receipt()).unwrap();
    let changed_artifact = rendered.replacen(&digest('0'), &digest('1'), 1);
    assert!(parse(&changed_artifact)
        .unwrap_err()
        .contains("digest drift"));

    let changed_count = rendered.replace(
        "meta\tdynamic-library-count\t2",
        "meta\tdynamic-library-count\t3",
    );
    assert!(parse(&changed_count)
        .unwrap_err()
        .contains("does not match records"));
}

#[test]
fn rejects_duplicate_or_noncanonical_record_order() {
    let rendered = render(&receipt()).unwrap();
    let duplicate_tool = rendered.replacen(
        "tool\tcargo\trust-toolchain/bin/cargo\tcargo 1.96.0 (fixture)\t11",
        concat!(
            "tool\tcargo\trust-toolchain/bin/cargo\tcargo 1.96.0 (fixture)\t11\t",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n",
            "tool\tcargo\trust-toolchain/bin/cargo\tcargo 1.96.0 (fixture)\t11"
        ),
        1,
    );
    assert!(parse(&duplicate_tool).is_err());

    let reordered = rendered.replace(
        "dynamic-library\tlibc.so.6\ndynamic-library\tlibgcc_s.so.1",
        "dynamic-library\tlibgcc_s.so.1\ndynamic-library\tlibc.so.6",
    );
    assert!(parse(&reordered).unwrap_err().contains("strictly ordered"));
}

#[test]
fn rejects_missing_or_overlarge_required_inventories() {
    let mut missing = observation();
    missing.final_link_inputs.clear();
    assert!(Receipt::new(authority(), missing)
        .unwrap_err()
        .contains("must be observed"));

    let mut overlarge = observation();
    overlarge.dynamic_libraries = (0..=format::MAX_DYNAMIC_LIBRARIES)
        .map(|index| format!("lib{index:03}.so"))
        .collect();
    assert!(Receipt::new(authority(), overlarge)
        .unwrap_err()
        .contains("exceeds bounds"));

    let mut empty_depfile = observation();
    empty_depfile.link_dependency_file_byte_length = 0;
    assert!(Receipt::new(authority(), empty_depfile)
        .unwrap_err()
        .contains("dependency file byte length"));
}

#[test]
fn rejects_unknown_records_and_unsafe_paths() {
    let rendered = render(&receipt()).unwrap();
    let unknown = rendered.replace("host\tx86_64", "mystery\trow\nhost\tx86_64");
    assert!(parse(&unknown)
        .unwrap_err()
        .contains("unknown or malformed"));

    let mut unsafe_observation = observation();
    unsafe_observation.tools[4].logical_path = "../usr/bin/cc".to_owned();
    assert!(Receipt::new(authority(), unsafe_observation)
        .unwrap_err()
        .contains("not normalized"));
}

#[test]
fn retains_empty_build_script_out_tree_and_rejects_omission() {
    let observed = receipt();
    assert_eq!(
        observed.observation.build_script_events[0].out_tree_file_count,
        0
    );
    assert_eq!(
        observed.observation.build_script_events[0].out_tree_byte_length,
        0
    );

    let mut omitted = observation();
    omitted.build_script_events.clear();
    assert!(Receipt::new(authority(), omitted)
        .unwrap_err()
        .contains("build-script events"));
}

#[test]
fn requires_exactly_one_identity_for_every_bound_tool_role() {
    let mut missing = observation();
    missing.tools.pop();
    assert!(Receipt::new(authority(), missing)
        .unwrap_err()
        .contains("tool identities must be exactly"));

    let mut duplicate = observation();
    duplicate.tools[5] = duplicate.tools[4].clone();
    assert!(Receipt::new(authority(), duplicate)
        .unwrap_err()
        .contains("tool identities must be exactly"));
}
