use super::*;

use std::cell::Cell;
use std::ffi::OsString;
use std::path::Path;

use crate::binary_artifact_receipt::process;
use crate::binary_artifact_receipt::runtime_linkage::bwrap_resolution_inventory::{
    parse, render, BWRAP_HOST_RESOLUTION_POLICY, RESOLUTION_RELATION,
};
use crate::binary_artifact_receipt::runtime_linkage::object_authority::prepared_probe::{
    observe_bwrap_host_resolution_with_test_tool, ExpectedRuntimeElfPolicy,
};
use crate::binary_artifact_receipt::runtime_linkage::{
    LOADER_TIMEOUT, MAX_LOADER_OUTPUT_BYTES, MAX_LOADER_STDERR_BYTES,
};

fn synthetic_output() -> Vec<u8> {
    format!(
        "\tlinux-vdso.so.1 (0x1)\n\
         \tlibselinux.so.1 => /lib/x86_64-linux-gnu/libselinux.so.1 (0x2)\n\
         \tlibcap.so.2 => /lib/x86_64-linux-gnu/libcap.so.2 (0x3)\n\
         \tlibc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x4)\n\
         \tlibpcre2-8.so.0 => /lib/x86_64-linux-gnu/libpcre2-8.so.0 (0x5)\n\
         \t{INTERPRETER} (0x6)\n"
    )
    .into_bytes()
}

#[test]
fn observer_uses_the_exact_counterfactual_plan_and_builds_a_replayable_inventory() {
    let fixture = Fixture::new("bwrap-host-resolution-plan");
    let calls = Cell::new(0);
    let observation = observe_bwrap_host_resolution_with_test_tool(
        fixture_bwrap_identity(fixture.plan.executable()),
        fixture_runtime_elf_policy(),
        |plan| {
            calls.set(calls.get() + 1);
            assert_eq!(plan.executable(), Path::new(INTERPRETER));
            assert_eq!(
                plan.arguments(),
                [
                    OsString::from("--inhibit-cache"),
                    OsString::from("--glibc-hwcaps-mask"),
                    OsString::from(""),
                    OsString::from("--list"),
                    fixture.plan.executable().as_os_str().to_owned(),
                ]
            );
            assert!(plan.clears_parent_environment());
            assert_eq!(plan.environment(), [("LC_ALL", "C")]);
            assert_eq!(plan.current_directory(), Path::new("/"));
            assert_eq!(plan.max_stdout_bytes(), MAX_LOADER_OUTPUT_BYTES as u64);
            assert_eq!(plan.max_stderr_bytes(), MAX_LOADER_STDERR_BYTES);
            assert_eq!(plan.timeout(), LOADER_TIMEOUT);
            Ok(process::Output {
                stdout: synthetic_output(),
                stderr: Vec::new(),
            })
        },
    )
    .unwrap();
    assert_eq!(calls.get(), 1);
    assert_eq!(observation.policy(), BWRAP_HOST_RESOLUTION_POLICY);
    assert_eq!(observation.relation(), RESOLUTION_RELATION);
    let inventory = observation.to_non_authority_inventory().unwrap();
    let canonical = render(&inventory).unwrap();
    let replayed = parse(&canonical).unwrap();
    assert_eq!(render(&replayed).unwrap(), canonical);
    assert_eq!(
        replayed.view().direct_needed(),
        ["libc.so.6", "libcap.so.2", "libselinux.so.1"]
    );
}

#[test]
fn policy_drift_rejects_before_the_runner_and_path_replacement_wins_after_it() {
    let stale = Fixture::new("bwrap-host-resolution-stale-policy");
    let calls = Cell::new(0);
    let error = observe_bwrap_host_resolution_with_test_tool(
        fixture_bwrap_identity(stale.plan.executable()),
        ExpectedRuntimeElfPolicy::new("stale-policy-v1", &"0".repeat(64)).unwrap(),
        |_| {
            calls.set(calls.get() + 1);
            unreachable!()
        },
    )
    .unwrap_err();
    assert!(error.contains("runtime ELF policy"), "{error}");
    assert_eq!(calls.get(), 0);

    let replaced = Fixture::new("bwrap-host-resolution-post-fence");
    let path = replaced.plan.executable().to_path_buf();
    let error = observe_bwrap_host_resolution_with_test_tool(
        fixture_bwrap_identity(&path),
        fixture_runtime_elf_policy(),
        |_| {
            std::fs::remove_file(&path).unwrap();
            regular(&path, &fixture_bwrap_bytes(), 0o755);
            Err("runner sentinel".to_owned())
        },
    )
    .unwrap_err();
    assert!(error.contains("bubblewrap"), "{error}");
    assert!(!error.contains("runner sentinel"), "{error}");
}

#[test]
fn stderr_and_semantically_substituted_loader_output_fail_closed() {
    for (name, stdout, stderr) in [
        ("stderr", synthetic_output(), b"diagnostic".to_vec()),
        ("malformed", b"not loader output\n".to_vec(), Vec::new()),
        (
            "missing-direct",
            synthetic_output()
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.starts_with(b"\tlibcap.so.2"))
                .flat_map(|line| line.iter().copied().chain(std::iter::once(b'\n')))
                .collect(),
            Vec::new(),
        ),
        (
            "substituted-loader",
            synthetic_output()
                .windows(INTERPRETER.len())
                .position(|window| window == INTERPRETER.as_bytes())
                .map(|offset| {
                    let mut bytes = synthetic_output();
                    bytes.splice(
                        offset..offset + INTERPRETER.len(),
                        b"/lib64/ld-linux-x86-64.so.3".iter().copied(),
                    );
                    bytes
                })
                .unwrap(),
            Vec::new(),
        ),
    ] {
        let fixture = Fixture::new(&format!("bwrap-host-resolution-{name}"));
        assert!(observe_bwrap_host_resolution_with_test_tool(
            fixture_bwrap_identity(fixture.plan.executable()),
            fixture_runtime_elf_policy(),
            |_| Ok(process::Output { stdout, stderr }),
        )
        .is_err());
    }
}

pub(super) fn run_native_inventory() {
    let observation = observe_bwrap_host_resolution(
        super::prepared_seccomp::native_bwrap(),
        super::prepared_seccomp::native_runtime_elf(),
    )
    .unwrap();
    let inventory = observation.to_non_authority_inventory().unwrap();
    let canonical = render(&inventory).unwrap();
    let replayed = parse(&canonical).unwrap();
    assert_eq!(render(&replayed).unwrap(), canonical);
    assert_eq!(
        replayed.view().bwrap_sha256(),
        inventory.view().bwrap_sha256()
    );
}
