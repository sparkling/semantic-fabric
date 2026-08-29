use super::*;

use std::cell::Cell;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::process::Command;

use crate::binary_artifact_receipt::process;
use crate::binary_artifact_receipt::runtime_linkage::object_authority::prepared_probe::{
    ExpectedBwrapIdentity, PreparedRuntimeProbe, PREPARED_LOADER_POLICY,
};
use crate::binary_artifact_receipt::runtime_linkage::prepared_receipt::{parse, render};

fn prepare_fixture(fixture: &Fixture) -> PreparedRuntimeProbe<'_> {
    let held = hold_runtime_inputs(&fixture.pair, &fixture.plan, &fixture.view).unwrap();
    PreparedRuntimeProbe::prepare_with_test_tool(held, fixture_tool_identity(fixture)).unwrap()
}

fn fixture_tool_identity(fixture: &Fixture) -> ExpectedBwrapIdentity {
    let bytes = b"fixture bubblewrap executable";
    ExpectedBwrapIdentity::new(
        fixture.plan.executable(),
        &format!("{:x}", Sha256::digest(bytes)),
        bytes.len() as u64,
        "test-fixture-static-bytes-v1",
    )
    .unwrap()
}

#[test]
fn prepared_policy_uses_only_canonical_sealed_source_copy_bindings() {
    let fixture = Fixture::new("prepared-policy");
    let probe = prepare_fixture(&fixture);
    let arguments: Vec<_> = probe
        .arguments
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        normalize_transfer_fds(&arguments),
        [
            "bwrap",
            "--die-with-parent",
            "--unshare-all",
            "--unshare-user",
            "--unshare-net",
            "--disable-userns",
            "--assert-userns-disabled",
            "--clearenv",
            "--size",
            "134217728",
            "--tmpfs",
            "/",
            "--cap-drop",
            "ALL",
            "--dir",
            "/lib",
            "--dir",
            "/lib/x86_64-linux-gnu",
            "--dir",
            "/lib64",
            "--perms",
            "0444",
            "--ro-bind-data",
            "<sealed-fd>",
            "/artifact",
            "--perms",
            "0444",
            "--ro-bind-data",
            "<sealed-fd>",
            LIBC_PATH,
            "--perms",
            "0555",
            "--ro-bind-data",
            "<sealed-fd>",
            INTERPRETER,
            "--remount-ro",
            "/",
            "--setenv",
            "LC_ALL",
            "C",
            "--chdir",
            "/",
            "--",
            INTERPRETER,
            "--inhibit-cache",
            "--glibc-hwcaps-mask",
            "",
            "--list",
            "/artifact",
        ]
        .map(str::to_owned)
    );

    for required in [
        "--die-with-parent",
        "--unshare-all",
        "--unshare-user",
        "--unshare-net",
        "--disable-userns",
        "--assert-userns-disabled",
        "--clearenv",
        "--remount-ro",
        "--inhibit-cache",
        "--glibc-hwcaps-mask",
    ] {
        assert!(
            arguments.iter().any(|value| value == required),
            "{required}"
        );
    }
    for forbidden in [
        "--ro-bind",
        "--bind",
        "--ro-bind-fd",
        "--proc",
        "--dev",
        "--new-session",
        "--share-net",
    ] {
        assert!(
            !arguments.iter().any(|value| value == forbidden),
            "{forbidden}"
        );
    }
    assert_eq!(arguments.first().map(String::as_str), Some("bwrap"));
    assert_eq!(
        arguments
            .iter()
            .filter(|value| *value == "--ro-bind-data")
            .count(),
        3
    );
    let mut descriptors = BTreeSet::new();
    for index in arguments
        .iter()
        .enumerate()
        .filter_map(|(index, value)| (value == "--ro-bind-data").then_some(index))
    {
        assert_eq!(arguments[index - 2], "--perms");
        assert!(matches!(arguments[index - 1].as_str(), "0444" | "0555"));
        let descriptor: i32 = arguments[index + 1].parse().unwrap();
        assert!((64..=1023).contains(&descriptor));
        assert!(descriptors.insert(descriptor));
    }
    assert!(arguments
        .windows(2)
        .any(|pair| pair == ["--size", "134217728"]));
    assert!(arguments
        .windows(2)
        .any(|pair| pair == ["--list", "/artifact"]));
    assert!(!arguments.iter().any(|value| value.contains("selected")));
    probe.validate_for_test().unwrap();
}

#[test]
fn prepared_policy_drift_and_output_substitution_fail_closed() {
    let drift_fixture = Fixture::new("prepared-drift");
    let mut drifted = prepare_fixture(&drift_fixture);
    let index = drifted
        .arguments
        .iter()
        .position(|argument| argument == "--remount-ro")
        .unwrap();
    drifted.arguments.remove(index);
    assert!(drifted.validate_for_test().is_err());

    let stderr_fixture = Fixture::new("prepared-stderr");
    let stderr = prepare_fixture(&stderr_fixture);
    let error = stderr
        .finish_for_test(synthetic_output(LIBC_PATH), b"diagnostic".to_vec())
        .unwrap_err();
    assert!(error.contains("wrote stderr"), "{error}");

    let malformed_fixture = Fixture::new("prepared-malformed");
    let malformed = prepare_fixture(&malformed_fixture);
    assert!(malformed
        .finish_for_test(b"not loader output\n".to_vec(), Vec::new())
        .is_err());

    let substituted_fixture = Fixture::new("prepared-substitution");
    let substituted = prepare_fixture(&substituted_fixture);
    let error = substituted
        .finish_for_test(
            synthetic_output("/usr/lib/x86_64-linux-gnu/libc.so.6"),
            Vec::new(),
        )
        .unwrap_err();
    assert!(error.contains("differs from candidate"), "{error}");
}

#[test]
fn prepared_policy_rejects_process_arguments_over_the_shared_bound() {
    let fixture = Fixture::new("prepared-argument-bound");
    let mut probe = prepare_fixture(&fixture);
    probe.arguments.push(OsString::from(
        "x".repeat(process::MAX_EXECVEAT_ARGUMENT_BYTES),
    ));
    let error = probe.validate_for_test().unwrap_err();
    assert!(error.contains("argument bytes exceed policy"), "{error}");
}

#[test]
fn prepared_probe_requires_an_independent_exact_tool_expectation() {
    let digest_fixture = Fixture::new("prepared-tool-digest-expectation");
    let held = hold_runtime_inputs(
        &digest_fixture.pair,
        &digest_fixture.plan,
        &digest_fixture.view,
    )
    .unwrap();
    let wrong_digest = ExpectedBwrapIdentity::new(
        digest_fixture.plan.executable(),
        &"0".repeat(64),
        b"fixture bubblewrap executable".len() as u64,
        "test-fixture-static-bytes-v1",
    )
    .unwrap();
    let error = PreparedRuntimeProbe::prepare_with_test_tool(held, wrong_digest).unwrap_err();
    assert!(error.contains("authorized identity"), "{error}");

    let path_fixture = Fixture::new("prepared-tool-path-expectation");
    let held =
        hold_runtime_inputs(&path_fixture.pair, &path_fixture.plan, &path_fixture.view).unwrap();
    let wrong_path = ExpectedBwrapIdentity::new(
        &path_fixture.root.join("other-bwrap"),
        &format!("{:x}", Sha256::digest(b"fixture bubblewrap executable")),
        b"fixture bubblewrap executable".len() as u64,
        "test-fixture-static-bytes-v1",
    )
    .unwrap();
    let error = PreparedRuntimeProbe::prepare_with_test_tool(held, wrong_path).unwrap_err();
    assert!(error.contains("path differs"), "{error}");

    assert!(ExpectedBwrapIdentity::new(
        Path::new("/usr/bin/bwrap"),
        &"0".repeat(64),
        1,
        "Unversioned Policy",
    )
    .is_err());
}

#[test]
fn prepared_probe_rejects_a_swapped_sealed_transfer_capability() {
    let fixture = Fixture::new("prepared-swapped-transfer");
    let mut probe = prepare_fixture(&fixture);
    probe.swap_transfers_for_test(0, 1);
    let error = probe.validate_for_test().unwrap_err();
    assert!(error.contains("transfer differs"), "{error}");
}

#[test]
fn prepared_observation_binds_policy_tool_inputs_and_raw_output() {
    let fixture = Fixture::new("prepared-observation");
    let probe = prepare_fixture(&fixture);
    let stdout = synthetic_output(LIBC_PATH);
    let observation = probe.finish_for_test(stdout.clone(), Vec::new()).unwrap();

    assert_eq!(observation.loader_policy, PREPARED_LOADER_POLICY);
    assert_eq!(observation.view.resolved_objects().len(), 1);
    assert_eq!(observation.bindings.len(), 3);
    assert_eq!(observation.bwrap_byte_length, 29);
    assert_eq!(
        observation.bwrap_sha256,
        format!("{:x}", Sha256::digest(b"fixture bubblewrap executable"))
    );
    assert_eq!(observation.bwrap_path, fixture.plan.executable());
    assert_eq!(
        observation.bwrap_executable_policy,
        "test-fixture-static-bytes-v1"
    );
    assert_eq!(observation.stdout, stdout);
    assert_eq!(
        observation.stdout_sha256,
        format!("{:x}", Sha256::digest(&observation.stdout))
    );
    let artifact_digest = format!("{:x}", Sha256::digest(root_fixture()));
    let loader_digest = format!("{:x}", Sha256::digest(loader_fixture()));
    let libc_digest = format!("{:x}", Sha256::digest(libc_fixture()));
    let bindings: Vec<_> = observation
        .bindings
        .iter()
        .map(|binding| {
            (
                binding.object.role,
                binding.object.logical_path.as_str(),
                binding.object.soname.as_deref(),
                binding.destination.as_str(),
                binding.mode,
                binding.object.sha256.as_str(),
                binding.object.byte_length,
            )
        })
        .collect();
    assert_eq!(
        bindings,
        vec![
            (
                RuntimeElfRole::RootPie,
                fixture.selected.to_str().unwrap(),
                None,
                "/artifact",
                0o444,
                artifact_digest.as_str(),
                root_fixture().len() as u64,
            ),
            (
                RuntimeElfRole::Libc,
                LIBC_PATH,
                Some("libc.so.6"),
                LIBC_PATH,
                0o444,
                libc_digest.as_str(),
                libc_fixture().len() as u64,
            ),
            (
                RuntimeElfRole::Loader,
                INTERPRETER,
                Some("ld-linux-x86-64.so.2"),
                INTERPRETER,
                0o555,
                loader_digest.as_str(),
                loader_fixture().len() as u64,
            ),
        ]
    );
}

#[test]
fn prepared_probe_detects_tool_and_source_replacement_before_execution() {
    let tool_fixture = Fixture::new("prepared-tool-replacement");
    let tool_probe = prepare_fixture(&tool_fixture);
    let tool = tool_fixture.plan.executable();
    fs::remove_file(tool).unwrap();
    regular(tool, b"replacement bubblewrap executable", 0o755);
    let error = tool_probe.validate_for_test().unwrap_err();
    assert!(error.contains("bubblewrap"), "{error}");

    let source_fixture = Fixture::new("prepared-source-replacement");
    let source_probe = prepare_fixture(&source_fixture);
    let mut replacement = libc_fixture();
    replacement.push(0);
    regular(&source_fixture.libc, &replacement, 0o644);
    assert!(source_probe.validate_for_test().is_err());
}

#[test]
fn prepared_execution_fences_before_and_after_the_runner() {
    let pre_fixture = Fixture::new("prepared-pre-fence");
    let pre_probe = prepare_fixture(&pre_fixture);
    let mut changed = libc_fixture();
    changed[0] ^= 1;
    regular(&pre_fixture.libc, &changed, 0o644);
    let calls = Cell::new(0);
    assert!(pre_probe
        .execute_for_test(|_| {
            calls.set(calls.get() + 1);
            unreachable!()
        })
        .is_err());
    assert_eq!(calls.get(), 0);

    let post_fixture = Fixture::new("prepared-post-fence");
    let post_probe = prepare_fixture(&post_fixture);
    let mut changed = libc_fixture();
    changed[0] ^= 1;
    let error = post_probe
        .execute_for_test(|_| {
            regular(&post_fixture.libc, &changed, 0o644);
            Err("runner sentinel".to_owned())
        })
        .unwrap_err();
    assert!(!error.contains("runner sentinel"), "{error}");
}

#[test]
#[ignore = "requires labelled self-hosted Linux bubblewrap and a release-profile binary"]
fn prepared_probe_observes_the_current_release_profile_binary_from_sealed_source_copies() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/semantic-fabric");
    assert!(
        source.is_file(),
        "build the release-profile binary before this gate"
    );
    let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "semantic-fabric-prepared-native-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    directory(&root, 0o700);
    let selected = root.join("selected");
    let linked = root.join("linked");
    fs::copy(&source, &selected).unwrap();
    fs::set_permissions(&selected, fs::Permissions::from_mode(0o755)).unwrap();
    fs::hard_link(&selected, &linked).unwrap();
    let pair = ArtifactPair::bind(&selected, &linked).unwrap();
    let bytes = fs::read(&selected).unwrap();
    let elf = parse_runtime_elf(&bytes, RuntimeElfRole::RootPie).unwrap();
    let interpreter = elf.interpreter().unwrap().to_owned();
    let mut needed = elf.needed().to_vec();
    needed.sort();
    let plan = crate::binary_artifact_receipt::runtime_linkage::plan_runtime_linkage(
        Path::new("/usr/bin/bwrap"),
        &selected,
        &interpreter,
    )
    .unwrap();
    assert!(plan.clears_parent_environment());
    let mut discovery = Command::new(plan.executable());
    discovery.env_clear().args(plan.arguments());
    let output = crate::binary_artifact_receipt::process::run(
        discovery,
        "unauthorized candidate runtime discovery",
        plan.max_stdout_bytes(),
        plan.max_stderr_bytes(),
        plan.timeout(),
    )
    .unwrap();
    assert!(output.stderr.is_empty());
    let view = crate::binary_artifact_receipt::runtime_linkage::parse_runtime_linkage_view(
        pair.sha256(),
        &interpreter,
        &needed,
        &output.stdout,
    )
    .unwrap();
    let held = hold_runtime_inputs(&pair, &plan, &view).unwrap();
    let expected_bwrap = ExpectedBwrapIdentity::new(
        Path::new("/usr/bin/bwrap"),
        "52231e1caf55bcbc667b269f49c63599a6f7db4767ae6a039580d0ff853db712",
        72_160,
        "ubuntu-24.04-bubblewrap-0.9.0-main-elf-only-host-dynamic-closure-unbound-v1",
    )
    .unwrap();
    let observation = held.execute_prepared(expected_bwrap).unwrap();

    assert_eq!(observation.view, view);
    assert_eq!(observation.loader_policy, PREPARED_LOADER_POLICY);
    assert_eq!(
        observation.bindings.len(),
        view.resolved_objects().len() + 2
    );
    assert_eq!(observation.stdout_sha256.len(), 64);
    assert_eq!(
        observation.bwrap_executable_policy,
        "ubuntu-24.04-bubblewrap-0.9.0-main-elf-only-host-dynamic-closure-unbound-v1"
    );
    let receipt = observation.to_non_admission_receipt().unwrap();
    let canonical = render(&receipt).unwrap();
    let replayed = parse(&canonical).unwrap();
    assert_eq!(render(&replayed).unwrap(), canonical);
    assert_eq!(replayed.semantic_replay().unwrap(), view);
    assert!(canonical.contains("meta\tauthority\tnone\n"));
    assert!(canonical.contains("meta\tadmission-result\tnot-evaluated\n"));
    drop(pair);
    fs::remove_dir_all(root).unwrap();
}

fn synthetic_output(libc_path: &str) -> Vec<u8> {
    format!(
        "\tlinux-vdso.so.1 (0x1)\n\
         \tlibc.so.6 => {libc_path} (0x2)\n\
         \t{INTERPRETER} (0x3)\n"
    )
    .into_bytes()
}

fn normalize_transfer_fds(arguments: &[String]) -> Vec<String> {
    let mut normalized = arguments.to_vec();
    for index in 0..normalized.len().saturating_sub(1) {
        if normalized[index] == "--ro-bind-data" {
            normalized[index + 1] = "<sealed-fd>".to_owned();
        }
    }
    normalized
}
