use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::*;

const ARTIFACT_SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const INTERPRETER: &str = "/lib64/ld-linux-x86-64.so.2";

fn direct_needed() -> Vec<String> {
    [
        "ld-linux-x86-64.so.2",
        "libc.so.6",
        "libgcc_s.so.1",
        "libm.so.6",
    ]
    .map(str::to_owned)
    .to_vec()
}

fn loader_output(address: char) -> Vec<u8> {
    format!(
        "\tlinux-vdso.so.1 (0x{address:0>16})\n\
         \tlibgcc_s.so.1 => /lib/x86_64-linux-gnu/libgcc_s.so.1 (0x{address:0>16})\n\
         \tlibm.so.6 => /lib/x86_64-linux-gnu/libm.so.6 (0x{address:0>16})\n\
         \tlibc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x{address:0>16})\n\
         \t/lib64/ld-linux-x86-64.so.2 (0x{address:0>16})\n"
    )
    .into_bytes()
}

fn parse(bytes: &[u8]) -> Result<RuntimeLinkageView, String> {
    parse_runtime_linkage_view(ARTIFACT_SHA256, INTERPRETER, &direct_needed(), bytes)
}

#[test]
fn canonicalizes_semantic_linkage_independent_of_addresses_and_line_order() {
    let first = parse(&loader_output('1')).unwrap();
    let reordered = b"\tlibm.so.6 => /lib/x86_64-linux-gnu/libm.so.6 (0x2222)\n\
        \t/lib64/ld-linux-x86-64.so.2 (0x3333)\n\
        \tlinux-vdso.so.1 (0x4444)\n\
        \tlibc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x5555)\n\
        \tlibgcc_s.so.1 => /lib/x86_64-linux-gnu/libgcc_s.so.1 (0x6666)\n";
    let second = parse(reordered).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.artifact_sha256(), ARTIFACT_SHA256);
    assert_eq!(first.elf_interpreter(), INTERPRETER);
    assert_eq!(first.loader_policy(), LOADER_POLICY);
    assert_eq!(first.loader_path(), INTERPRETER);
    assert_eq!(first.direct_needed(), direct_needed());
    assert_eq!(first.virtual_objects()[0].name(), "linux-vdso.so.1");
    assert_eq!(
        first
            .resolved_objects()
            .iter()
            .map(|object| (object.soname(), object.resolved_path()))
            .collect::<Vec<_>>(),
        [
            ("libc.so.6", "/lib/x86_64-linux-gnu/libc.so.6"),
            ("libgcc_s.so.1", "/lib/x86_64-linux-gnu/libgcc_s.so.1",),
            ("libm.so.6", "/lib/x86_64-linux-gnu/libm.so.6"),
        ]
    );
}

#[test]
fn retains_well_formed_transitive_objects_without_making_them_direct() {
    let output = String::from_utf8(loader_output('1')).unwrap().replace(
        "\t/lib64/ld-linux-x86-64.so.2",
        "\tlibtransitive.so.1 => /lib/x86_64-linux-gnu/libtransitive.so.1 (0x7777)\n\
         \t/lib64/ld-linux-x86-64.so.2",
    );
    let view = parse(output.as_bytes()).unwrap();

    assert_eq!(view.direct_needed(), direct_needed());
    assert!(view
        .resolved_objects()
        .iter()
        .any(|object| object.soname() == "libtransitive.so.1"));
}

#[test]
fn rejects_invalid_authority_inputs_and_incomplete_direct_resolution() {
    let output = loader_output('1');
    let cases = [
        parse_runtime_linkage_view(
            "A".repeat(64).as_str(),
            INTERPRETER,
            &direct_needed(),
            &output,
        ),
        parse_runtime_linkage_view(&"0".repeat(63), INTERPRETER, &direct_needed(), &output),
        parse_runtime_linkage_view(ARTIFACT_SHA256, "lib64/ld.so", &direct_needed(), &output),
        parse_runtime_linkage_view(
            ARTIFACT_SHA256,
            "/lib64/../lib64/ld-linux-x86-64.so.2",
            &direct_needed(),
            &output,
        ),
        parse_runtime_linkage_view(
            ARTIFACT_SHA256,
            INTERPRETER,
            &["libm.so.6".to_owned(), "libc.so.6".to_owned()],
            &output,
        ),
        parse_runtime_linkage_view(
            ARTIFACT_SHA256,
            INTERPRETER,
            &["libc.so.6".to_owned(), "libc.so.6".to_owned()],
            &output,
        ),
        parse_runtime_linkage_view(
            ARTIFACT_SHA256,
            INTERPRETER,
            &["libmissing.so.1".to_owned()],
            &output,
        ),
        parse_runtime_linkage_view(
            ARTIFACT_SHA256,
            INTERPRETER,
            &["lib/name.so".to_owned()],
            &output,
        ),
        parse_runtime_linkage_view(
            ARTIFACT_SHA256,
            INTERPRETER,
            &["lib name.so".to_owned()],
            &output,
        ),
        parse_runtime_linkage_view(ARTIFACT_SHA256, INTERPRETER, &["x".repeat(129)], &output),
    ];

    for result in cases {
        assert!(result.is_err());
    }
}

#[test]
fn rejects_loader_output_grammar_and_path_spoofs() {
    let valid = String::from_utf8(loader_output('1')).unwrap();
    let invalid = [
        valid.trim_end().to_owned(),
        valid.replace('\n', "\r\n"),
        valid.replace("linux-vdso", "linux-\0vdso"),
        valid.replace("0x0000000000000001", "0X0000000000000001"),
        valid.replace("0x0000000000000001", "0xABC"),
        valid.replace(" (0x0000000000000001)", " (0x1) trailing"),
        valid.replacen('\t', "", 1),
        valid.replace("\tlibm", "\n\tlibm"),
        valid.replace(
            "/lib/x86_64-linux-gnu/libm.so.6",
            "lib/x86_64-linux-gnu/libm.so.6",
        ),
        valid.replace("/lib/x86_64-linux-gnu/libm.so.6", "/lib/../tmp/libm.so.6"),
        valid.replace(
            "/lib/x86_64-linux-gnu/libm.so.6",
            "/lib//x86_64-linux-gnu/libm.so.6",
        ),
        valid.replace("/lib/x86_64-linux-gnu/libm.so.6", "not found"),
        valid.replace(
            "libm.so.6 => /lib/x86_64-linux-gnu/libm.so.6",
            "libm.so.6 => /lib/x86_64-linux-gnu/libm.so.6 => /tmp/spoof",
        ),
        valid.replace("\tlibm.so.6", "libm.so.6"),
        valid.replace("\tlibm.so.6", " libm.so.6"),
        valid.replace("\tlibm.so.6", "\t\tlibm.so.6"),
        valid.replace("\tlibm.so.6", " \tlibm.so.6"),
        valid.replace(
            "libm.so.6 => /lib/x86_64-linux-gnu/libm.so.6",
            "libm.so.6 => /lib",
        ),
    ];

    for output in invalid {
        assert!(parse(output.as_bytes()).is_err(), "accepted {output:?}");
    }
}

#[test]
fn rejects_duplicate_conflicting_missing_and_unknown_loader_records() {
    let valid = String::from_utf8(loader_output('1')).unwrap();
    let libc = "\tlibc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x1)\n";
    let virtual_collision = valid.replace(
        "\t/lib64/ld-linux-x86-64.so.2 (0x0000000000000001)\n",
        "\tlinux-vdso.so.1 => /lib/linux-vdso.so.1 (0x1)\n\
         \t/lib64/ld-linux-x86-64.so.2 (0x0000000000000001)\n",
    );
    let loader_collision = valid.replace(
        "\t/lib64/ld-linux-x86-64.so.2 (0x0000000000000001)\n",
        "\tld-linux-x86-64.so.2 => /lib/ld-linux-x86-64.so.2 (0x1)\n\
         \t/lib64/ld-linux-x86-64.so.2 (0x0000000000000001)\n",
    );
    for output in [&virtual_collision, &loader_collision] {
        assert!(loader_output::parse(output.as_bytes()).is_err());
    }
    let invalid = [
        format!("{valid}{libc}"),
        virtual_collision,
        loader_collision,
        valid.replace(
            "libm.so.6 => /lib/x86_64-linux-gnu/libm.so.6",
            "libother.so.6 => /lib/x86_64-linux-gnu/libc.so.6",
        ),
        valid.replace("\tlinux-vdso.so.1 (0x0000000000000001)\n", ""),
        valid.replace("\t/lib64/ld-linux-x86-64.so.2 (0x0000000000000001)\n", ""),
        valid.replace(
            "\t/lib64/ld-linux-x86-64.so.2 (0x0000000000000001)\n",
            "\t/lib64/ld-linux-x86-64.so.2 (0x1)\n\t/lib64/ld-linux-x86-64.so.2 (0x2)\n",
        ),
        valid.replace(
            "libm.so.6 => /lib/x86_64-linux-gnu/libm.so.6",
            "ld-linux-x86-64.so.2 => /lib/x86_64-linux-gnu/other-ld.so.2",
        ),
        valid.replace(
            "libm.so.6 => /lib/x86_64-linux-gnu/libm.so.6",
            "libm.so.6 => /lib64/ld-linux-x86-64.so.2",
        ),
        valid.replace(
            "libm.so.6 => /lib/x86_64-linux-gnu/libm.so.6",
            "libm.so.6 => /lib/x86_64-linux-gnu/libz.so.1",
        ),
        valid.replace(
            "\t/lib64/ld-linux-x86-64.so.2 (0x0000000000000001)",
            "\t/usr/lib64/ld-linux-x86-64.so.2 (0x0000000000000001)",
        ),
        valid.replace("linux-vdso.so.1", "linux-gate.so.1"),
        format!("{valid}\tunexpected runtime diagnostics\n"),
    ];

    for output in invalid {
        assert!(parse(output.as_bytes()).is_err(), "accepted {output:?}");
    }
}

#[test]
fn rejects_output_and_object_count_overflow() {
    let oversized = vec![b'x'; MAX_LOADER_OUTPUT_BYTES + 1];
    assert!(parse(&oversized).is_err());

    let mut too_many = String::from("\tlinux-vdso.so.1 (0x1)\n");
    for index in 0..=MAX_RESOLVED_OBJECTS {
        too_many.push_str(&format!("\tlib{index}.so => /lib/lib{index}.so (0x1)\n"));
    }
    too_many.push_str("\t/lib64/ld-linux-x86-64.so.2 (0x1)\n");
    assert!(parse(too_many.as_bytes()).is_err());
}

fn synthetic_mounts() -> Vec<RuntimeReadOnlyMount> {
    [
        ("/usr/lib", "/usr/lib"),
        ("/usr/lib", "/lib"),
        ("/usr/lib64", "/usr/lib64"),
        ("/usr/lib64", "/lib64"),
    ]
    .map(|(source, destination)| RuntimeReadOnlyMount {
        source: PathBuf::from(source),
        destination: destination.to_owned(),
    })
    .to_vec()
}

fn plan() -> RuntimeLoaderPlan {
    build_plan(
        Path::new("/usr/bin/bwrap"),
        Path::new("/private/target/semantic-fabric"),
        INTERPRETER,
        synthetic_mounts(),
    )
    .unwrap()
}

fn arguments(plan: &RuntimeLoaderPlan) -> Vec<String> {
    plan.arguments()
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn builds_an_exact_nonexecuting_loader_plan() {
    let plan = plan();
    let argv = arguments(&plan);

    assert_eq!(plan.executable(), Path::new("/usr/bin/bwrap"));
    assert!(plan.clears_parent_environment());
    assert_eq!(plan.max_stdout_bytes(), MAX_LOADER_OUTPUT_BYTES as u64);
    assert_eq!(plan.max_stderr_bytes(), MAX_LOADER_STDERR_BYTES);
    assert_eq!(plan.timeout(), LOADER_TIMEOUT);
    assert!(validate_plan(&plan).is_ok());
    assert_eq!(
        argv,
        [
            "--die-with-parent",
            "--new-session",
            "--unshare-all",
            "--unshare-net",
            "--clearenv",
            "--tmpfs",
            "/",
            "--cap-drop",
            "ALL",
            "--dir",
            "/usr",
            "--ro-bind",
            "/private/target/semantic-fabric",
            "/artifact",
            "--ro-bind",
            "/usr/lib",
            "/lib",
            "--ro-bind",
            "/usr/lib64",
            "/lib64",
            "--ro-bind",
            "/usr/lib",
            "/usr/lib",
            "--ro-bind",
            "/usr/lib64",
            "/usr/lib64",
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
    );
    assert!(argv.windows(2).any(|pair| pair == ["--", INTERPRETER]));
    assert!(argv
        .windows(2)
        .any(|pair| pair == ["--inhibit-cache", "--glibc-hwcaps-mask"]));
    assert!(argv.windows(2).any(|pair| pair == ["--list", "/artifact"]));
    assert!(!argv.iter().any(|value| {
        value.contains("ldd")
            || value == "sh"
            || value.ends_with("/sh")
            || value.starts_with("LD_")
            || value == "/etc"
            || value == "--share-net"
    }));
}

#[test]
fn rejects_capability_and_loader_policy_plan_mutants() {
    let base = plan();
    let mut mutants = Vec::new();
    for needle in [
        OsString::from("--die-with-parent"),
        OsString::from("--new-session"),
        OsString::from("--unshare-all"),
        OsString::from("--unshare-net"),
        OsString::from("--clearenv"),
        OsString::from("--tmpfs"),
        OsString::from("--cap-drop"),
        OsString::from("--setenv"),
        OsString::from("LC_ALL"),
        OsString::from("C"),
        OsString::from("--chdir"),
        OsString::from("--inhibit-cache"),
        OsString::from("--glibc-hwcaps-mask"),
    ] {
        let mut mutant = base.clone();
        mutant.arguments.retain(|argument| argument != &needle);
        mutants.push(mutant);
    }
    let mut writable = base.clone();
    let index = writable
        .arguments
        .iter()
        .position(|argument| argument == "--ro-bind")
        .unwrap();
    writable.arguments[index] = "--bind".into();
    mutants.push(writable);
    let mut shell = base.clone();
    let index = shell
        .arguments
        .iter()
        .position(|argument| argument == INTERPRETER)
        .unwrap();
    shell.arguments[index] = "/bin/sh".into();
    mutants.push(shell);
    let mut ambient = base.clone();
    ambient.arguments.splice(
        3..3,
        [
            "--setenv".into(),
            "LD_PRELOAD".into(),
            "/tmp/evil.so".into(),
        ],
    );
    mutants.push(ambient);
    let mut hwcaps = base.clone();
    let index = hwcaps
        .arguments
        .iter()
        .position(|argument| argument.is_empty())
        .unwrap();
    hwcaps.arguments[index] = "x86-64-v3".into();
    mutants.push(hwcaps);
    let mut trailing = base.clone();
    trailing.arguments.push("--preload".into());
    mutants.push(trailing);
    let mut inherited = base.clone();
    inherited.clear_parent_environment = false;
    mutants.push(inherited);
    let mut unbounded = base;
    unbounded.max_stdout_bytes += 1;
    mutants.push(unbounded);
    let mut stderr = plan();
    stderr.max_stderr_bytes -= 1;
    mutants.push(stderr);
    let mut timeout = plan();
    timeout.timeout += std::time::Duration::from_millis(1);
    mutants.push(timeout);

    for mutant in mutants {
        assert!(validate_plan(&mutant).is_err());
    }
}

fn assert_plan_rejected(bwrap: &str, artifact: &str, mounts: Vec<RuntimeReadOnlyMount>) {
    assert!(build_plan(Path::new(bwrap), Path::new(artifact), INTERPRETER, mounts,).is_err());
}

#[test]
fn rejects_plan_paths_and_mounts_outside_the_runtime_allowlist() {
    for (bwrap, artifact) in [
        ("usr/bin/bwrap", "/private/target/semantic-fabric"),
        ("/usr/bin/bwrap", "private/target/semantic-fabric"),
        ("/usr//bin/bwrap", "/private/target/semantic-fabric"),
        ("//usr/bin/bwrap", "/private/target/semantic-fabric"),
        ("/usr/bin/bwrap", "/private/./target/semantic-fabric"),
        ("/usr/bin/bwrap", "/private//target/semantic-fabric"),
        ("/usr/bin/bwrap/", "/private/target/semantic-fabric"),
    ] {
        assert_plan_rejected(bwrap, artifact, synthetic_mounts());
    }
    let mut mounts = synthetic_mounts();
    mounts.push(RuntimeReadOnlyMount {
        source: "/etc".into(),
        destination: "/etc".to_owned(),
    });
    assert_plan_rejected("/usr/bin/bwrap", "/private/target/semantic-fabric", mounts);
    let mut missing_interpreter = synthetic_mounts();
    missing_interpreter.retain(|mount| mount.destination != "/lib64");
    assert_plan_rejected(
        "/usr/bin/bwrap",
        "/private/target/semantic-fabric",
        missing_interpreter,
    );
    let mut missing_usr_lib = synthetic_mounts();
    missing_usr_lib.retain(|mount| mount.destination != "/usr/lib");
    assert_plan_rejected(
        "/usr/bin/bwrap",
        "/private/target/semantic-fabric",
        missing_usr_lib,
    );
    let mut duplicate = synthetic_mounts();
    duplicate.push(RuntimeReadOnlyMount {
        source: "/other/lib".into(),
        destination: "/lib".to_owned(),
    });
    assert_plan_rejected(
        "/usr/bin/bwrap",
        "/private/target/semantic-fabric",
        duplicate,
    );
    let mut overlap = synthetic_mounts();
    overlap[0].source = "/private".into();
    assert_plan_rejected("/usr/bin/bwrap", "/private/target/semantic-fabric", overlap);
}
