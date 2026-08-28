#![cfg(feature = "evidence-receipts")]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use sf_conformance::binary_artifact_receipt::{
    load_external, parse, render, write_new_external, ArtifactObservation, BuildScriptEvent,
    HostObservation, LinkInput, LinkInputOrigin, PortableAuthority, Receipt, ToolIdentity,
    ToolRole,
};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

fn digest(byte: char) -> String {
    std::iter::repeat_n(byte, 64).collect()
}

fn receipt() -> Receipt {
    let authority = PortableAuthority {
        git_revision: "0123456789abcdef0123456789abcdef01234567".to_owned(),
        source_date_epoch: 1_787_875_200,
        source_inputs_sha256: digest('1'),
        cargo_lock_sha256: digest('2'),
        rust_toolchain_sha256: digest('3'),
        cargo_home_inputs_sha256: digest('4'),
        cargo_config_set_sha256: digest('5'),
        closure_receipt_sha256: digest('6'),
        current_closure_sha256: digest('7'),
    };
    let tool = |role, logical_path: &str, version: &str, byte_length, sha| ToolIdentity {
        role,
        logical_path: logical_path.to_owned(),
        version: version.to_owned(),
        byte_length,
        sha256: digest(sha),
    };
    let observation = HostObservation {
        host_triple: "x86_64-unknown-linux-gnu".to_owned(),
        os_release_sha256: digest('8'),
        environment_sha256: digest('9'),
        link_dependency_file_byte_length: 128,
        link_dependency_file_sha256: digest('a'),
        tools: vec![
            tool(
                ToolRole::GitMaterializer,
                "host-system/usr/bin/git",
                "git version 2.53.0 (fixture)",
                10,
                'a',
            ),
            tool(
                ToolRole::Cargo,
                "rust-toolchain/bin/cargo",
                "cargo 1.96.0 (fixture)",
                11,
                'b',
            ),
            tool(
                ToolRole::Rustc,
                "rust-toolchain/bin/rustc",
                "rustc 1.96.0 (fixture)",
                12,
                'c',
            ),
            tool(
                ToolRole::Sandbox,
                "host-system/usr/bin/bwrap",
                "bubblewrap 0.11.0 (fixture)",
                13,
                'd',
            ),
            tool(
                ToolRole::Linker,
                "host-system/usr/bin/cc",
                "cc (fixture) 1.0",
                14,
                'd',
            ),
            tool(
                ToolRole::ElfReader,
                "host-system/usr/bin/readelf",
                "GNU readelf 2.45 (fixture)",
                15,
                'e',
            ),
        ],
        build_script_events: vec![BuildScriptEvent {
            package_id: "libsqlite3-sys@0.38.0".to_owned(),
            logical_out_dir: "build-output/release/build/libsqlite3-sys-fixture/out".to_owned(),
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
    };
    Receipt::new(authority, observation).expect("fixture is valid")
}

fn private_directory(name: &str) -> PathBuf {
    let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "semantic-fabric-artifact-cli-{name}-{}-{serial}",
        std::process::id()
    ));
    fs::create_dir(&path).expect("create private test directory");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("make test directory private");
    }
    path
}

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_current-sf-cli-artifact-observation"))
        .args(arguments)
        .output()
        .expect("run artifact observation CLI")
}

#[test]
fn canonical_render_parse_and_external_io_round_trip() {
    let expected = receipt();
    let rendered = render(&expected).expect("render canonical receipt");
    assert_eq!(parse(&rendered).expect("parse canonical receipt"), expected);

    let root = private_directory("external");
    let repository = root.join("repository");
    let output = root.join("output");
    fs::create_dir(&repository).expect("create repository directory");
    fs::create_dir(&output).expect("create output directory");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for directory in [&repository, &output] {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
                .expect("make authority directory private");
        }
    }
    let path = output.join("receipt.tsv");
    write_new_external(&repository, &path, &expected).expect("write external receipt");
    assert_eq!(
        load_external(&repository, &path).expect("load external receipt"),
        expected
    );
    assert!(write_new_external(&repository, &path, &expected).is_err());
    fs::remove_dir_all(root).expect("remove test directory");
}

#[test]
fn help_is_explicit_and_cannot_be_combined() {
    let help = run(&["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--capture"));
    assert!(String::from_utf8_lossy(&help.stdout).contains("--verify"));

    let combined = run(&["--help", "--verify"]);
    assert!(!combined.status.success());
    assert!(String::from_utf8_lossy(&combined.stderr).contains("cannot be combined"));
}

#[test]
fn invalid_arguments_fail_before_any_capture_or_receipt_load() {
    let cases: &[(&[&str], &str)] = &[
        (&[], "choose exactly one"),
        (&["--verify"], "missing required argument"),
        (&["--verify", "--verify"], "choose exactly one"),
        (&["--capture", "--verify"], "choose exactly one"),
        (&["--verify", "--unknown", "/tmp/value"], "unknown argument"),
        (
            &[
                "--verify",
                "--repository",
                "/tmp/repository",
                "--repository",
                "/tmp/other",
                "--input",
                "/tmp/receipt.tsv",
            ],
            "duplicate argument",
        ),
        (
            &[
                "--verify",
                "--repository",
                "/tmp/repository",
                "--input",
                "relative.tsv",
            ],
            "absolute, normalized",
        ),
        (&["--capture"], "missing required argument"),
        (
            &["--capture", "--unknown", "/tmp/value"],
            "unknown argument",
        ),
        (
            &[
                "--capture",
                "--repository",
                "/tmp/repository",
                "--repository",
                "/tmp/other",
            ],
            "duplicate argument",
        ),
        (
            &["--capture", "--repository", "relative"],
            "absolute, normalized",
        ),
    ];
    for (arguments, expected_error) in cases {
        let output = run(arguments);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success() && stderr.contains(expected_error),
            "invalid arguments had the wrong outcome: {arguments:?}: {stderr}"
        );
    }
}
