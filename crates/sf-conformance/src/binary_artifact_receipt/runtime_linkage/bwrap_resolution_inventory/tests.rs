use super::super::prepared_receipt::{
    render as render_prepared, PreparedRuntimeReceipt, RecordedBindingIdentity,
    RecordedObjectIdentity, RecordedPreparedRuntimeObservation, RecordedRuntimeRole,
};
use super::*;

const BWRAP_PATH: &str = "/usr/bin/bwrap";
const INTERPRETER: &str = "/lib64/ld-linux-x86-64.so.2";
const BWRAP_SHA256: &str = "52231e1caf55bcbc667b269f49c63599a6f7db4767ae6a039580d0ff853db712";
const RUNTIME_ELF_SHA256: &str = "cd23f2d883c1e99b655395284e7d803e6d00b9eaf90a417560efca7ffde50b0a";

fn stdout() -> Vec<u8> {
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

fn fixture_record() -> RecordedBwrapHostResolution {
    let stdout = stdout();
    let direct_needed = ["libc.so.6", "libcap.so.2", "libselinux.so.1"]
        .map(str::to_owned)
        .to_vec();
    let view = super::super::parse_runtime_linkage_view(
        BWRAP_SHA256,
        INTERPRETER,
        &direct_needed,
        &stdout,
    )
    .unwrap();
    RecordedBwrapHostResolution {
        bwrap_path: BWRAP_PATH.to_owned(),
        bwrap_sha256: BWRAP_SHA256.to_owned(),
        bwrap_byte_length: 72_160,
        bwrap_executable_policy:
            "ubuntu-24.04-bubblewrap-0.9.0-main-elf-only-host-dynamic-closure-unbound-v1".to_owned(),
        runtime_elf_policy: "elf64-le-x86_64-closed-dynamic-tags-safe-search-flags-v1".to_owned(),
        runtime_elf_policy_sha256: RUNTIME_ELF_SHA256.to_owned(),
        view: BwrapHostResolutionView::from_runtime(view),
        stdout_sha256: sha256(&stdout),
        stdout,
    }
}

fn fixture() -> BwrapHostResolutionInventory {
    BwrapHostResolutionInventory::from_recorded(fixture_record()).unwrap()
}

#[test]
fn canonical_inventory_round_trips_with_explicit_non_authority() {
    let inventory = fixture();
    let canonical = render(&inventory).unwrap();
    assert!(canonical.starts_with(&format!("{HEADER}\n")));
    assert!(canonical.contains("meta\tauthority\tnone\n"));
    assert!(canonical.contains(&format!(
        "meta\tresolution-relation\t{RESOLUTION_RELATION}\n"
    )));
    assert!(canonical.contains("meta\tresolution-target-exact-byte-consumption\tnot-attested\n"));
    assert_eq!(render(&parse(&canonical).unwrap()).unwrap(), canonical);
    assert_eq!(&inventory.semantic_replay().unwrap(), inventory.view());
}

#[test]
fn prepared_receipt_and_resolution_inventory_reject_each_others_kind() {
    let canonical = render(&fixture()).unwrap();
    assert!(super::super::prepared_receipt::parse(&canonical).is_err());
    assert!(parse(&render_prepared(&prepared_fixture()).unwrap()).is_err());
}

#[test]
fn inventory_digest_and_nonclaim_set_are_frozen() {
    let inventory = fixture();
    assert_eq!(
        inventory.inventory_sha256().unwrap(),
        "b9cfb64ff7e57df3241ab35c3234230fdf081abe289c5753b6fd5a629bd4ffb6"
    );
    let canonical = render(&inventory).unwrap();
    for key in NONCLAIM_KEYS {
        assert!(
            canonical.contains(&format!("meta\t{key}\t{NOT_ATTESTED}\n")),
            "{key}"
        );
    }
    assert_eq!(
        canonical
            .lines()
            .filter(|line| line.starts_with("meta\t") && line.ends_with("\tnot-attested"))
            .count(),
        NONCLAIM_KEYS.len()
    );
}

#[test]
fn record_validation_rejects_tool_policy_and_raw_output_drift() {
    let mut digest = fixture_record();
    digest.bwrap_sha256 = "0".repeat(64);
    assert!(BwrapHostResolutionInventory::from_recorded(digest).is_err());

    let mut runtime_policy = fixture_record();
    runtime_policy.runtime_elf_policy_sha256 = "A".repeat(64);
    assert!(BwrapHostResolutionInventory::from_recorded(runtime_policy).is_err());

    let mut path = fixture_record();
    path.bwrap_path.push('\t');
    assert!(BwrapHostResolutionInventory::from_recorded(path).is_err());

    let mut stdout = fixture_record();
    stdout.stdout[0] ^= 1;
    assert!(BwrapHostResolutionInventory::from_recorded(stdout).is_err());
}

#[test]
fn reminted_structural_mutants_still_fail_closed() {
    let canonical = render(&fixture()).unwrap();
    let deleted_nonclaim = remint(&canonical.replacen(
        "meta\tresolution-target-exact-byte-consumption\tnot-attested\n",
        "",
        1,
    ));
    let duplicate_metadata = remint(&canonical.replacen(
        "meta\tauthority\tnone\n",
        "meta\tauthority\tnone\nmeta\tauthority\tnone\n",
        1,
    ));
    let unknown_record =
        remint(&canonical.replacen("tool\tbubblewrap", "unknown\tvalue\ntool\tbubblewrap", 1));
    let leading_zero = remint(&canonical.replacen("\t72160\t", "\t072160\t", 1));
    let reordered_direct = remint(&canonical.replacen(
        "direct-needed\tlibc.so.6\ndirect-needed\tlibcap.so.2\n",
        "direct-needed\tlibcap.so.2\ndirect-needed\tlibc.so.6\n",
        1,
    ));
    let substituted_resolution = remint(&canonical.replacen(
        "resolved\tlibc.so.6\t/lib/x86_64-linux-gnu/libc.so.6\n",
        "resolved\tlibc.so.6\t/usr/lib/x86_64-linux-gnu/libc.so.6\n",
        1,
    ));
    let malformed_chunk = remint(&uppercase_first_chunk_hex(&canonical));
    for (name, mutant) in [
        ("deleted-nonclaim", deleted_nonclaim),
        ("duplicate-metadata", duplicate_metadata),
        ("unknown-record", unknown_record),
        ("leading-zero", leading_zero),
        ("reordered-direct", reordered_direct),
        ("substituted-resolution", substituted_resolution),
        ("malformed-chunk", malformed_chunk),
    ] {
        assert!(parse(&mutant).is_err(), "{name}");
    }
}

#[test]
fn self_consistent_address_remint_remains_explicitly_non_authorizing() {
    let baseline = fixture();
    let mut record = fixture_record();
    let text = String::from_utf8(record.stdout.clone()).unwrap();
    record.stdout = text.replace("(0x1)", "(0xa)").into_bytes();
    record.stdout_sha256 = sha256(&record.stdout);
    let reminted = BwrapHostResolutionInventory::from_recorded(record).unwrap();
    assert_eq!(reminted.view(), baseline.view());
    let canonical = render(&reminted).unwrap();
    assert!(canonical.contains("meta\tauthority\tnone\n"));
    assert!(canonical.contains("meta\treplay-execution\tnot-attested\n"));
}

fn remint(input: &str) -> String {
    let body = input
        .strip_suffix('\n')
        .unwrap()
        .rsplit_once('\n')
        .unwrap()
        .0;
    let unsigned = format!("{body}\n");
    let digest = domain_sha256(INVENTORY_DOMAIN, unsigned.as_bytes());
    format!("{unsigned}inventory-sha256\t{digest}\n")
}

fn uppercase_first_chunk_hex(input: &str) -> String {
    let mut output = input.to_owned();
    let start = output.find("stdout-chunk\t00000000\t").unwrap() + "stdout-chunk\t00000000\t".len();
    let end = output[start..].find('\n').unwrap() + start;
    let relative = output[start..end]
        .bytes()
        .position(|byte| (b'a'..=b'f').contains(&byte))
        .unwrap();
    output.replace_range(start + relative..start + relative + 1, "A");
    output
}

fn prepared_fixture() -> PreparedRuntimeReceipt {
    let resolution = fixture_record();
    let view = resolution.view.0.clone();
    let mut bindings = vec![RecordedBindingIdentity {
        object: RecordedObjectIdentity {
            logical_path: "/tmp/artifact".to_owned(),
            role: RecordedRuntimeRole::RootPie,
            soname: None,
            device: 1,
            inode: 1,
            byte_length: 1_001,
            sha256: BWRAP_SHA256.to_owned(),
        },
        destination: "/artifact".to_owned(),
        mode: 0o444,
    }];
    for (index, object) in view.resolved_objects().iter().enumerate() {
        bindings.push(RecordedBindingIdentity {
            object: RecordedObjectIdentity {
                logical_path: object.resolved_path().to_owned(),
                role: if object.soname() == "libc.so.6" {
                    RecordedRuntimeRole::Libc
                } else {
                    RecordedRuntimeRole::SharedObject
                },
                soname: Some(object.soname().to_owned()),
                device: index as u64 + 2,
                inode: index as u64 + 2,
                byte_length: index as u64 + 1_002,
                sha256: format!("{:064x}", index + 2),
            },
            destination: object.resolved_path().to_owned(),
            mode: 0o444,
        });
    }
    bindings.push(RecordedBindingIdentity {
        object: RecordedObjectIdentity {
            logical_path: INTERPRETER.to_owned(),
            role: RecordedRuntimeRole::Loader,
            soname: Some("ld-linux-x86-64.so.2".to_owned()),
            device: 100,
            inode: 100,
            byte_length: 2_000,
            sha256: format!("{:064x}", 100),
        },
        destination: INTERPRETER.to_owned(),
        mode: 0o555,
    });
    bindings.sort_by(|left, right| left.destination.cmp(&right.destination));
    PreparedRuntimeReceipt::from_recorded(RecordedPreparedRuntimeObservation {
        view,
        bindings,
        bwrap_sha256: BWRAP_SHA256.to_owned(),
        bwrap_byte_length: 72_160,
        bwrap_path: BWRAP_PATH.to_owned(),
        bwrap_executable_policy:
            "ubuntu-24.04-bubblewrap-0.9.0-main-elf-only-host-dynamic-closure-unbound-v1"
                .to_owned(),
        stdout: resolution.stdout,
        stdout_sha256: resolution.stdout_sha256,
    })
    .unwrap()
}
