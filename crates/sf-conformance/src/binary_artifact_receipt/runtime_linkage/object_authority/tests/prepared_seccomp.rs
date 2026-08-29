use super::*;

use std::cell::Cell;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd};
use std::process::Command;

use crate::binary_artifact_receipt::process;
use crate::binary_artifact_receipt::runtime_linkage::object_authority::prepared_probe::{
    seccomp_bytes_for_test, seccomp_identity_for_test, validate_seccomp_bytes_for_test,
    ExpectedBwrapIdentity, ExpectedPreparedSeccompPolicy, ExpectedRuntimeElfPolicy,
    PreparedRuntimeProbe, PREPARED_LOADER_POLICY, PREPARED_SECCOMP_POLICY,
};
use crate::binary_artifact_receipt::runtime_linkage::prepared_receipt::{parse, render};
use crate::binary_artifact_receipt::RUNTIME_ELF_POLICY;

const RET_KILL_PROCESS: u32 = 0x8000_0000;
const RET_ALLOW: u32 = 0x7fff_0000;
const ARCH_X86_64: u32 = 0xc000_003e;

#[test]
fn seccomp_program_is_exact_default_kill_and_argument_restricted() {
    let bytes = seccomp_bytes_for_test();
    let identity = seccomp_identity_for_test();
    validate_seccomp_bytes_for_test(&bytes).unwrap();
    assert_eq!(identity.id(), PREPARED_SECCOMP_POLICY);
    assert_eq!(identity.byte_length(), bytes.len() as u64);
    assert_eq!(identity.sha256(), format!("{:x}", Sha256::digest(&bytes)));
    assert_eq!(
        identity.sha256(),
        "0092c69f902c071515f2f82c5aff75bf63f065148f1c0fb51af414787338e80a"
    );
    let ordinary = [0, 3, 5, 12, 17, 21, 59, 61, 158, 218, 231, 273, 334];
    for syscall in ordinary {
        assert_eq!(evaluate(&bytes, ARCH_X86_64, syscall, [0; 6]), RET_ALLOW);
    }
    let mut arguments = [0; 6];
    arguments[0] = 4;
    assert_eq!(evaluate(&bytes, ARCH_X86_64, 1, arguments), RET_ALLOW);
    arguments[0] = 1;
    assert_eq!(
        evaluate(&bytes, ARCH_X86_64, 1, arguments),
        RET_KILL_PROCESS
    );
    assert_eq!(evaluate(&bytes, ARCH_X86_64, 20, arguments), RET_ALLOW);
    arguments[0] = 2;
    assert_eq!(
        evaluate(&bytes, ARCH_X86_64, 20, arguments),
        RET_KILL_PROCESS
    );
    arguments = [0; 6];
    arguments[2] = libc::O_RDONLY as u64 | libc::O_CLOEXEC as u64;
    assert_eq!(evaluate(&bytes, ARCH_X86_64, 257, arguments), RET_ALLOW);
    for forbidden in [
        libc::O_WRONLY,
        libc::O_RDWR,
        libc::O_CREAT,
        libc::O_TRUNC,
        libc::O_TMPFILE,
    ] {
        arguments[2] = forbidden as u64;
        assert_eq!(
            evaluate(&bytes, ARCH_X86_64, 257, arguments),
            RET_KILL_PROCESS
        );
    }
    arguments[2] = (libc::PROT_READ | libc::PROT_EXEC) as u64;
    assert_eq!(evaluate(&bytes, ARCH_X86_64, 9, arguments), RET_ALLOW);
    arguments[2] = (libc::PROT_READ | libc::PROT_WRITE) as u64;
    assert_eq!(evaluate(&bytes, ARCH_X86_64, 9, arguments), RET_ALLOW);
    arguments[2] = (libc::PROT_WRITE | libc::PROT_EXEC) as u64;
    assert_eq!(
        evaluate(&bytes, ARCH_X86_64, 9, arguments),
        RET_KILL_PROCESS
    );

    for forbidden in [
        10, 15, 41, 56, 57, 58, 101, 157, 165, 272, 298, 308, 317, 321, 322, 425, 435,
    ] {
        assert_eq!(
            evaluate(&bytes, ARCH_X86_64, forbidden, [0; 6]),
            RET_KILL_PROCESS
        );
    }
    assert_eq!(evaluate(&bytes, 0, 0, [0; 6]), RET_KILL_PROCESS);
    assert_eq!(
        evaluate(&bytes, ARCH_X86_64, 0x4000_003b, [0; 6]),
        RET_KILL_PROCESS
    );
}

#[test]
fn every_seccomp_byte_and_structural_mutant_fails_closed() {
    let bytes = seccomp_bytes_for_test();
    for index in 0..bytes.len() {
        for bit in 0..8 {
            let mut mutant = bytes.clone();
            mutant[index] ^= 1 << bit;
            assert!(
                validate_seccomp_bytes_for_test(&mutant).is_err(),
                "{index}:{bit}"
            );
        }
    }
    assert!(validate_seccomp_bytes_for_test(&bytes[..bytes.len() - 1]).is_err());
    let mut trailing = bytes.clone();
    trailing.extend_from_slice(&[0; 8]);
    assert!(validate_seccomp_bytes_for_test(&trailing).is_err());

    let mut default_allow = bytes.clone();
    let end = default_allow.len();
    default_allow[end - 4..].copy_from_slice(&RET_ALLOW.to_le_bytes());
    assert!(validate_seccomp_bytes_for_test(&default_allow).is_err());

    let mut unbounded_jump = bytes;
    unbounded_jump[8 + 2] = u8::MAX;
    let error = validate_seccomp_bytes_for_test(&unbounded_jump).unwrap_err();
    assert!(error.contains("jump target"), "{error}");
}

#[test]
fn prepared_probe_fences_policy_identity_descriptor_and_argument_drift() {
    let actual = seccomp_identity_for_test();
    let stale = [
        (
            "id",
            ExpectedPreparedSeccompPolicy::new(
                "drifted-prepared-seccomp-policy-v1",
                actual.sha256(),
                actual.byte_length(),
            )
            .unwrap(),
        ),
        (
            "digest",
            ExpectedPreparedSeccompPolicy::new(actual.id(), &"0".repeat(64), actual.byte_length())
                .unwrap(),
        ),
        (
            "length",
            ExpectedPreparedSeccompPolicy::new(
                actual.id(),
                actual.sha256(),
                actual.byte_length() + 8,
            )
            .unwrap(),
        ),
    ];
    for (field, expected) in stale {
        let fixture = Fixture::new(&format!("seccomp-stale-{field}"));
        let mut probe = prepare(&fixture);
        probe.replace_expected_seccomp_policy_for_test(expected);
        let calls = Cell::new(0);
        let error = probe
            .execute_for_test(|_| {
                calls.set(calls.get() + 1);
                unreachable!()
            })
            .unwrap_err();
        assert!(error.contains("pinned expectation"), "{field}: {error}");
        assert_eq!(calls.get(), 0, "{field}");
    }

    let mutable_fixture = Fixture::new("seccomp-mutable-transfer");
    let mut mutable = prepare(&mutable_fixture);
    mutable.replace_seccomp_transfer_for_test(unsealed_high_transfer(&seccomp_bytes_for_test()));
    let error = mutable.validate_for_test().unwrap_err();
    assert!(error.contains("descriptor policy"), "{error}");

    let wrong_bytes_fixture = Fixture::new("seccomp-sealed-wrong-bytes");
    let mut wrong_bytes = prepare(&wrong_bytes_fixture);
    let mut mutant = seccomp_bytes_for_test();
    mutant[0] ^= 1;
    wrong_bytes.replace_seccomp_transfer_for_test(sealed_high_transfer(&mutant));
    let calls = Cell::new(0);
    let error = wrong_bytes
        .execute_for_test(|_| {
            calls.set(calls.get() + 1);
            unreachable!()
        })
        .unwrap_err();
    assert!(error.contains("bytes changed"), "{error}");
    assert_eq!(calls.get(), 0);
    let post_run_fixture = Fixture::new("seccomp-post-run-descriptor-drift");
    let post_run = prepare(&post_run_fixture);
    let seccomp_fd = post_run.seccomp_fd_for_test();
    let error = post_run
        .execute_for_test(|_| {
            // SAFETY: mutate only this test-owned live descriptor after the
            // pre-run fence, proving the independent post-run fence observes it.
            let flags = unsafe { libc::fcntl(seccomp_fd, libc::F_GETFD) };
            assert!(flags >= 0);
            assert_eq!(
                unsafe { libc::fcntl(seccomp_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) },
                0
            );
            Ok(process::Output {
                stdout: synthetic_output(),
                stderr: Vec::new(),
            })
        })
        .unwrap_err();
    assert!(error.contains("descriptor policy"), "{error}");
    assert_argument_mutant_rejected("missing", |arguments| {
        let index = arguments
            .iter()
            .position(|value| value == "--seccomp")
            .unwrap();
        arguments.drain(index..=index + 1);
    });
    assert_argument_mutant_rejected("duplicate", |arguments| {
        let index = arguments
            .iter()
            .position(|value| value == "--seccomp")
            .unwrap();
        let pair = [arguments[index].clone(), arguments[index + 1].clone()];
        arguments.splice(index..index, pair);
    });
    assert_argument_mutant_rejected("reordered", |arguments| {
        let index = arguments
            .iter()
            .position(|value| value == "--seccomp")
            .unwrap();
        let pair: Vec<_> = arguments.drain(index..=index + 1).collect();
        let command = arguments.iter().position(|value| value == "--").unwrap() + 1;
        arguments.splice(command..command, pair);
    });
    assert_argument_mutant_rejected("add-seccomp", |arguments| {
        let command = arguments.iter().position(|value| value == "--").unwrap();
        arguments.splice(
            command..command,
            [OsString::from("--add-seccomp-fd"), OsString::from("999")],
        );
    });
}

#[test]
fn prepared_observation_binds_live_seccomp_but_receipt_v1_retains_nonclaim() {
    let fixture = Fixture::new("seccomp-observation");
    let probe = prepare(&fixture);
    let seccomp_fd = probe.seccomp_fd_for_test();
    let observation = probe
        .execute_for_test(|request| {
            assert_eq!(request.data_fds.last(), Some(&seccomp_fd));
            assert_eq!(
                request
                    .data_fds
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .len(),
                request.data_fds.len()
            );
            let index = request
                .argv
                .iter()
                .position(|value| value == "--seccomp")
                .unwrap();
            assert_eq!(
                request.argv[index + 1],
                OsString::from(seccomp_fd.to_string())
            );
            assert_eq!(request.argv[index + 2], "--");
            Ok(process::Output {
                stdout: synthetic_output(),
                stderr: Vec::new(),
            })
        })
        .unwrap();
    let identity = seccomp_identity_for_test();
    assert_eq!(observation.seccomp_policy, identity.id());
    assert_eq!(observation.seccomp_policy_sha256, identity.sha256());
    assert_eq!(
        observation.seccomp_policy_byte_length,
        identity.byte_length()
    );

    let canonical = render(&observation.to_non_admission_receipt().unwrap()).unwrap();
    assert!(canonical.contains("meta\ttarget-seccomp-or-syscall-trace\tnot-attested\n"));
    assert!(!canonical.contains(PREPARED_SECCOMP_POLICY));
    assert_eq!(render(&parse(&canonical).unwrap()).unwrap(), canonical);
}

#[test]
#[ignore = "requires labelled self-hosted Linux bubblewrap"]
fn prepared_seccomp_native_canary_kills_a_forbidden_socket_syscall() {
    super::prepared_seccomp_canary::run_native_canary();
}

pub(super) fn run_native_prepared_probe() {
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
    let mut discovery = Command::new(plan.executable());
    discovery.env_clear().args(plan.arguments());
    let output = process::run(
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
    let observation = hold_runtime_inputs(&pair, &plan, &view)
        .unwrap()
        .execute_prepared(native_bwrap(), native_runtime_elf(), native_seccomp())
        .unwrap();
    assert_eq!(observation.view, view);
    assert_eq!(observation.loader_policy, PREPARED_LOADER_POLICY);
    assert_eq!(observation.runtime_elf_policy, RUNTIME_ELF_POLICY);
    assert_eq!(observation.seccomp_policy, PREPARED_SECCOMP_POLICY);
    assert_eq!(
        observation.seccomp_policy_sha256,
        seccomp_identity_for_test().sha256()
    );
    assert_eq!(
        observation.bindings.len(),
        view.resolved_objects().len() + 2
    );
    assert_eq!(observation.stdout_sha256.len(), 64);
    let canonical = render(&observation.to_non_admission_receipt().unwrap()).unwrap();
    let replayed = parse(&canonical).unwrap();
    assert_eq!(render(&replayed).unwrap(), canonical);
    assert_eq!(replayed.semantic_replay().unwrap(), view);
    assert!(canonical.contains("meta\tauthority\tnone\n"));
    drop(pair);
    fs::remove_dir_all(root).unwrap();
}

fn prepare(fixture: &Fixture) -> PreparedRuntimeProbe<'_> {
    let held = hold_runtime_inputs(&fixture.pair, &fixture.plan, &fixture.view).unwrap();
    let tool = b"fixture bubblewrap executable";
    let expected = ExpectedBwrapIdentity::new(
        fixture.plan.executable(),
        &format!("{:x}", Sha256::digest(tool)),
        tool.len() as u64,
        "test-fixture-static-bytes-v1",
    )
    .unwrap();
    PreparedRuntimeProbe::prepare_with_test_tool(
        held,
        expected,
        fixture_runtime_elf_policy(),
        fixture_seccomp_policy(),
    )
    .unwrap()
}

fn assert_argument_mutant_rejected(name: &str, mutate: impl FnOnce(&mut Vec<OsString>)) {
    let fixture = Fixture::new(&format!("seccomp-argument-{name}"));
    let mut probe = prepare(&fixture);
    mutate(&mut probe.arguments);
    let error = probe.validate_for_test().unwrap_err();
    assert!(error.contains("argument policy drift"), "{name}: {error}");
}

pub(super) fn native_bwrap() -> ExpectedBwrapIdentity {
    ExpectedBwrapIdentity::new(
        Path::new("/usr/bin/bwrap"),
        "52231e1caf55bcbc667b269f49c63599a6f7db4767ae6a039580d0ff853db712",
        72_160,
        "ubuntu-24.04-bubblewrap-0.9.0-main-elf-only-host-dynamic-closure-unbound-v1",
    )
    .unwrap()
}

fn native_runtime_elf() -> ExpectedRuntimeElfPolicy {
    ExpectedRuntimeElfPolicy::new(
        "elf64-le-x86_64-closed-dynamic-tags-safe-search-flags-v1",
        "cd23f2d883c1e99b655395284e7d803e6d00b9eaf90a417560efca7ffde50b0a",
    )
    .unwrap()
}

pub(super) fn native_seccomp() -> ExpectedPreparedSeccompPolicy {
    ExpectedPreparedSeccompPolicy::new(
        "x86_64-prepared-loader-late-cbpf-default-kill-v1",
        "0092c69f902c071515f2f82c5aff75bf63f065148f1c0fb51af414787338e80a",
        440,
    )
    .unwrap()
}

fn synthetic_output() -> Vec<u8> {
    format!(
        "\tlinux-vdso.so.1 (0x1)\n\
         \tlibc.so.6 => {LIBC_PATH} (0x2)\n\
         \t{INTERPRETER} (0x3)\n"
    )
    .into_bytes()
}

fn unsealed_high_transfer(bytes: &[u8]) -> File {
    let source = unsealed_memfd(bytes);
    duplicate_high(&source)
}

fn sealed_high_transfer(bytes: &[u8]) -> File {
    let source = unsealed_memfd(bytes);
    let seals = libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
    // SAFETY: fcntl applies seals to the live owned memfd.
    assert_eq!(
        unsafe { libc::fcntl(source.as_raw_fd(), libc::F_ADD_SEALS, seals) },
        0
    );
    duplicate_high(&source)
}

fn duplicate_high(source: &File) -> File {
    // SAFETY: F_DUPFD_CLOEXEC creates a new independently owned descriptor.
    let descriptor = unsafe { libc::fcntl(source.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 64) };
    assert!((64..=1023).contains(&descriptor));
    // SAFETY: ownership of the new descriptor transfers to File.
    unsafe { File::from_raw_fd(descriptor) }
}

fn unsealed_memfd(bytes: &[u8]) -> File {
    // SAFETY: the static name is NUL terminated and the returned descriptor is owned.
    let descriptor = unsafe {
        libc::memfd_create(
            c"semantic-fabric-seccomp-test".as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    assert!(descriptor >= 0);
    // SAFETY: ownership of the new descriptor transfers to File.
    let mut file = unsafe { File::from_raw_fd(descriptor) };
    file.write_all(bytes).unwrap();
    file
}

fn evaluate(bytes: &[u8], arch: u32, syscall: u32, arguments: [u64; 6]) -> u32 {
    let mut accumulator = 0u32;
    let mut pc = 0usize;
    loop {
        let filter = &bytes[pc * 8..pc * 8 + 8];
        let code = u16::from_le_bytes([filter[0], filter[1]]);
        let jt = usize::from(filter[2]);
        let jf = usize::from(filter[3]);
        let k = u32::from_le_bytes([filter[4], filter[5], filter[6], filter[7]]);
        match code {
            0x20 => {
                accumulator = match k {
                    0 => syscall,
                    4 => arch,
                    offset if offset >= 16 && (offset - 16).is_multiple_of(8) => {
                        arguments[((offset - 16) / 8) as usize] as u32
                    }
                    _ => panic!("unexpected seccomp data offset {k}"),
                };
                pc += 1;
            }
            0x54 => {
                accumulator &= k;
                pc += 1;
            }
            0x15 => pc += 1 + if accumulator == k { jt } else { jf },
            0x35 => pc += 1 + if accumulator >= k { jt } else { jf },
            0x06 => return k,
            _ => panic!("unexpected cBPF instruction {code:#x}"),
        }
    }
}
