use super::*;

fn fixture_record(addresses: [&str; 3]) -> RecordedPreparedRuntimeObservation {
    let stdout = loader_output("/lib/x86_64-linux-gnu/libc.so.6", addresses);
    let artifact_sha256 = sha256(b"artifact");
    let view = parse_runtime_linkage_view(
        &artifact_sha256,
        "/lib64/ld-linux-x86-64.so.2",
        &["libc.so.6".to_owned()],
        &stdout,
    )
    .unwrap();
    RecordedPreparedRuntimeObservation {
        view,
        bindings: vec![
            binding(
                RecordedRuntimeRole::RootPie,
                "/private/artifact",
                None,
                "/artifact",
                0o444,
                10,
                b"artifact",
            ),
            binding(
                RecordedRuntimeRole::Libc,
                "/lib/x86_64-linux-gnu/libc.so.6",
                Some("libc.so.6"),
                "/lib/x86_64-linux-gnu/libc.so.6",
                0o444,
                11,
                b"libc",
            ),
            binding(
                RecordedRuntimeRole::Loader,
                "/lib64/ld-linux-x86-64.so.2",
                Some("ld-linux-x86-64.so.2"),
                "/lib64/ld-linux-x86-64.so.2",
                0o555,
                12,
                b"loader",
            ),
        ],
        bwrap_sha256: sha256(b"bwrap"),
        bwrap_byte_length: 5,
        bwrap_path: "/usr/bin/bwrap".to_owned(),
        bwrap_executable_policy: "fixture-bwrap-v1".to_owned(),
        stdout_sha256: sha256(&stdout),
        stdout,
    }
}

fn fixture_receipt() -> PreparedRuntimeReceipt {
    PreparedRuntimeReceipt::from_recorded(fixture_record(["1", "2", "3"])).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn binding(
    role: RecordedRuntimeRole,
    logical_path: &str,
    soname: Option<&str>,
    destination: &str,
    mode: u32,
    inode: u64,
    bytes: &[u8],
) -> RecordedBindingIdentity {
    RecordedBindingIdentity {
        object: RecordedObjectIdentity {
            logical_path: logical_path.to_owned(),
            role,
            soname: soname.map(str::to_owned),
            device: 1,
            inode,
            byte_length: bytes.len() as u64,
            sha256: sha256(bytes),
        },
        destination: destination.to_owned(),
        mode,
    }
}

fn loader_output(libc_path: &str, addresses: [&str; 3]) -> Vec<u8> {
    format!(
        "\tlinux-vdso.so.1 (0x{})\n\
         \tlibc.so.6 => {libc_path} (0x{})\n\
         \t/lib64/ld-linux-x86-64.so.2 (0x{})\n",
        addresses[0], addresses[1], addresses[2]
    )
    .into_bytes()
}

#[test]
fn canonical_record_has_a_frozen_digest_and_only_non_authority_metadata() {
    let receipt = fixture_receipt();
    let rendered = render(&receipt).unwrap();
    let parsed = parse(&rendered).unwrap();

    assert_eq!(parsed, receipt);
    assert_eq!(render(&parsed).unwrap(), rendered);
    assert_eq!(
        receipt.receipt_sha256().unwrap(),
        "e3abdef744d7b185f542bfcf748093b67c588a80200e738f3a29a330d0a37388"
    );
    for fixed in [
        "meta\tauthority\tnone\n",
        "meta\tadmission-result\tnot-evaluated\n",
        "meta\trecord-disposition\tnon-admission-only\n",
        "meta\tsemantic-replay-model\tprovider-free-record-validation-no-execution-v1\n",
        &format!("meta\tloader-policy\t{PREPARED_LOADER_POLICY}\n"),
    ] {
        assert!(rendered.contains(fixed), "{fixed:?}");
    }
    for key in NONCLAIM_KEYS {
        assert_eq!(
            rendered
                .lines()
                .filter(|line| *line == format!("meta\t{key}\t{NOT_ATTESTED}"))
                .count(),
            1,
            "{key}"
        );
    }
    assert_eq!(parsed.semantic_replay().unwrap(), receipt.record.view);
}

#[test]
fn debug_redacts_paths_raw_output_and_source_identity() {
    let debug = format!("{:?}", fixture_receipt());
    for secret in [
        "/usr/bin/bwrap",
        "/lib64/ld-linux",
        "linux-vdso",
        "0x1",
        "device",
        "inode",
    ] {
        assert!(!debug.contains(secret), "{secret}");
    }
}

#[test]
fn every_fixed_metadata_field_fails_closed_when_promoted_or_drifted() {
    let canonical = render(&fixture_receipt()).unwrap();
    for (key, value) in fixed_metadata() {
        let needle = format!("meta\t{key}\t{value}\n");
        let replacement = format!("meta\t{key}\tpromoted\n");
        assert_rejected(
            &remint(&replace_once(&canonical, &needle, &replacement)),
            key,
        );
    }

    let deleted = canonical.replacen("meta\tauthority\tnone\n", "", 1);
    assert_rejected(&remint(&deleted), "deleted metadata");
    let duplicate = canonical.replacen(
        "meta\tauthority\tnone\n",
        "meta\tauthority\tnone\nmeta\tauthority\tnone\n",
        1,
    );
    assert_rejected(&remint(&duplicate), "duplicate metadata");
    let unknown = canonical.replacen("view\t", "meta\tunknown-authority\tnone\nview\t", 1);
    assert_rejected(&remint(&unknown), "unknown metadata");
}

#[test]
fn framing_order_and_canonical_number_mutants_fail_closed() {
    let canonical = render(&fixture_receipt()).unwrap();
    let cases = [
        (canonical.replace('\n', "\r\n"), "CRLF"),
        (canonical.trim_end_matches('\n').to_owned(), "missing LF"),
        (format!("{canonical}\n"), "extra LF"),
        (canonical.replacen(HEADER, "wrong-header", 1), "header"),
        (
            canonical.replacen("view\t", "unknown\tx\nview\t", 1),
            "unknown record",
        ),
        (
            canonical.replacen("view\t", "meta\tlate\tnone\nview\t", 1),
            "late metadata",
        ),
        (
            canonical.replacen(
                "tool\tbubblewrap\t/usr/bin/bwrap\t5\t",
                "tool\tbubblewrap\t/usr/bin/bwrap\t05\t",
                1,
            ),
            "leading zero",
        ),
        (
            canonical.replacen("fixture-bwrap-v1", "fixture\u{7f}bwrap-v1", 1),
            "control",
        ),
    ];
    for (mutated, label) in cases {
        assert_rejected(&remint_if_framed(&mutated), label);
    }

    let mut lines = canonical_lines(&canonical);
    let tool = lines
        .iter()
        .position(|line| line.starts_with("tool\t"))
        .unwrap();
    let needed = lines
        .iter()
        .position(|line| line.starts_with("direct-needed\t"))
        .unwrap();
    lines.swap(tool, needed);
    assert_rejected(&seal_lines(lines), "record order");
}

#[test]
fn stale_and_reminted_relational_identity_mutants_fail_closed() {
    let canonical = render(&fixture_receipt()).unwrap();
    for (needle, replacement, label) in [
        ("/usr/bin/bwrap", "/opt/bin/bwrap", "stale tool path"),
        ("fixture-bwrap-v1", "fixture-bwrap-v2", "stale tool policy"),
        (
            "stdout-chunk\t00000000",
            "stdout-chunk\t00000001",
            "stale stdout",
        ),
    ] {
        assert_rejected(&replace_once(&canonical, needle, replacement), label);
    }
    for (needle, replacement, label) in [
        ("binding\troot-pie", "binding\tloader", "root role"),
        ("\t/artifact\t0444\t", "\t/artifact\t0555\t", "root mode"),
        ("some\tlibc.so.6", "some\tlibx.so.6", "SONAME"),
        ("\t1\t11\t4\t", "\t1\t10\t4\t", "source identity"),
        (
            "/lib/x86_64-linux-gnu/libc.so.6\t0444",
            "/lib/libc.so.6\t0444",
            "destination",
        ),
    ] {
        let stale = replace_once(&canonical, needle, replacement);
        assert_rejected(&stale, label);
        assert_rejected(&remint(&stale), label);
    }

    let mut dropped = canonical_lines(&canonical);
    let libc = dropped
        .iter()
        .position(|line| line.contains("some\tlibc.so.6"))
        .unwrap();
    dropped.remove(libc);
    assert_rejected(&seal_lines(dropped), "dropped binding");

    let mut duplicated = canonical_lines(&canonical);
    let root = duplicated
        .iter()
        .position(|line| line.starts_with("binding\troot-pie"))
        .unwrap();
    duplicated.insert(root + 1, duplicated[root].clone());
    assert_rejected(&seal_lines(duplicated), "duplicate binding");

    let mut reordered = canonical_lines(&canonical);
    let root = reordered
        .iter()
        .position(|line| line.starts_with("binding\troot-pie"))
        .unwrap();
    reordered.swap(root, root + 1);
    assert_rejected(&seal_lines(reordered), "reordered binding");

    let artifact = sha256(b"artifact");
    let drifted_view = canonical.replacen(
        &format!("view\t{artifact}\t"),
        &format!("view\t{}\t", sha256(b"other-artifact")),
        1,
    );
    assert_rejected(&remint(&drifted_view), "view artifact digest");
}

#[test]
fn stdout_codec_and_semantic_substitution_mutants_fail_closed() {
    let canonical = render(&fixture_receipt()).unwrap();
    let chunk = canonical
        .lines()
        .find(|line| line.starts_with("stdout-chunk\t"))
        .unwrap();
    let encoded = chunk.rsplit('\t').next().unwrap();
    let uppercase = replace_once(&canonical, encoded, &encoded.to_ascii_uppercase());
    assert_rejected(&remint(&uppercase), "uppercase stdout hex");
    let odd = replace_once(&canonical, encoded, &encoded[..encoded.len() - 1]);
    assert_rejected(&remint(&odd), "odd stdout hex");
    let wrong_index = canonical.replacen("stdout-chunk\t00000000", "stdout-chunk\t00000001", 1);
    assert_rejected(&remint(&wrong_index), "stdout index");
    let wrong_count = canonical.replacen("\t1\t1024\nstdout-chunk", "\t2\t1024\nstdout-chunk", 1);
    assert_rejected(&remint(&wrong_count), "stdout count");
    let wrong_policy = canonical.replacen("\t1\t1024\nstdout-chunk", "\t1\t2048\nstdout-chunk", 1);
    assert_rejected(&remint(&wrong_policy), "stdout chunk policy");

    let mut substituted = fixture_record(["1", "2", "3"]);
    substituted.stdout = loader_output("/usr/lib/x86_64-linux-gnu/libc.so.6", ["1", "2", "3"]);
    substituted.stdout_sha256 = sha256(&substituted.stdout);
    assert!(PreparedRuntimeReceipt::from_recorded(substituted).is_err());
}

#[test]
fn address_only_observations_share_semantics_but_not_record_identity() {
    let first = fixture_receipt();
    let second = PreparedRuntimeReceipt::from_recorded(fixture_record(["a", "b", "c"])).unwrap();
    assert_eq!(
        first.semantic_replay().unwrap(),
        second.semantic_replay().unwrap()
    );
    assert_ne!(first.record_sha256(), second.record_sha256());
    assert_ne!(
        first.receipt_sha256().unwrap(),
        second.receipt_sha256().unwrap()
    );
}

#[test]
fn a_reminted_self_consistent_alternative_stays_non_authoritative() {
    let original = fixture_receipt();
    let mut alternative = original.record.clone();
    alternative.bwrap_path = "/opt/private/bwrap".to_owned();
    alternative.bwrap_sha256 = sha256(b"different-bwrap");
    alternative.bwrap_byte_length = 15;
    alternative.bindings[1].object.byte_length = 14;
    alternative.bindings[1].object.sha256 = sha256(b"different-libc");
    let alternative = PreparedRuntimeReceipt::from_recorded(alternative).unwrap();
    let rendered = render(&alternative).unwrap();

    assert_ne!(
        original.receipt_sha256().unwrap(),
        alternative.receipt_sha256().unwrap()
    );
    assert!(rendered.contains("meta\tauthority\tnone\n"));
    assert!(rendered.contains("meta\tprepared-execution-provenance\tnot-attested\n"));
    assert!(rendered.contains("meta\tloader-output-origin\tnot-attested\n"));
    assert_eq!(parse(&rendered).unwrap(), alternative);
}

#[test]
fn origin_bounds_and_unique_identity_invariants_are_revalidated() {
    let original = fixture_record(["1", "2", "3"]);
    for mutate in [
        |record: &mut RecordedPreparedRuntimeObservation| record.bindings[0].object.device = 0,
        |record: &mut RecordedPreparedRuntimeObservation| record.bindings[0].object.inode = 0,
        |record: &mut RecordedPreparedRuntimeObservation| {
            record.bindings[0].object.byte_length = MAX_RUNTIME_OBJECT_BYTES + 1
        },
        |record: &mut RecordedPreparedRuntimeObservation| {
            record.bwrap_byte_length = MAX_BWRAP_BYTES + 1
        },
    ] {
        let mut mutant = original.clone();
        mutate(&mut mutant);
        assert!(PreparedRuntimeReceipt::from_recorded(mutant).is_err());
    }

    let mut duplicate_source = original.clone();
    duplicate_source.bindings[1].object.device = duplicate_source.bindings[0].object.device;
    duplicate_source.bindings[1].object.inode = duplicate_source.bindings[0].object.inode;
    assert!(PreparedRuntimeReceipt::from_recorded(duplicate_source).is_err());

    let mut duplicate_bytes = original;
    duplicate_bytes.bindings[1].object.byte_length = duplicate_bytes.bindings[0].object.byte_length;
    duplicate_bytes.bindings[1].object.sha256 = duplicate_bytes.bindings[0].object.sha256.clone();
    assert!(PreparedRuntimeReceipt::from_recorded(duplicate_bytes).is_err());
}

#[test]
fn optional_soname_encoding_is_injective_for_a_literal_hyphen() {
    let mut record = fixture_record(["1", "2", "3"]);
    let stdout = b"\tlinux-vdso.so.1 (0x1)\n\t- => /lib/- (0x2)\n\tlibc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x3)\n\t/lib64/ld-linux-x86-64.so.2 (0x4)\n".to_vec();
    record.view = parse_runtime_linkage_view(
        record.view.artifact_sha256(),
        record.view.elf_interpreter(),
        &["-".to_owned(), "libc.so.6".to_owned()],
        &stdout,
    )
    .unwrap();
    record.bindings.insert(
        1,
        binding(
            RecordedRuntimeRole::SharedObject,
            "/lib/-",
            Some("-"),
            "/lib/-",
            0o444,
            13,
            b"hyphen",
        ),
    );
    record.stdout_sha256 = sha256(&stdout);
    record.stdout = stdout;
    let receipt = PreparedRuntimeReceipt::from_recorded(record).unwrap();
    let rendered = render(&receipt).unwrap();
    assert!(rendered.contains("\tsome\t-\t/lib/-\t"));
    assert_eq!(parse(&rendered).unwrap(), receipt);
}

fn assert_rejected(input: &str, label: &str) {
    assert!(parse(input).is_err(), "mutant was accepted: {label}");
}

fn replace_once(input: &str, needle: &str, replacement: &str) -> String {
    assert!(input.contains(needle), "missing mutation needle {needle:?}");
    input.replacen(needle, replacement, 1)
}

fn canonical_lines(input: &str) -> Vec<String> {
    input
        .trim_end_matches('\n')
        .lines()
        .filter(|line| !line.starts_with("receipt-sha256\t"))
        .map(str::to_owned)
        .collect()
}

fn remint_if_framed(input: &str) -> String {
    if input.ends_with('\n') && !input.contains('\r') && !input.ends_with("\n\n") {
        remint(input)
    } else {
        input.to_owned()
    }
}

fn remint(input: &str) -> String {
    seal_lines(canonical_lines(input))
}

fn seal_lines(mut lines: Vec<String>) -> String {
    let record_start = lines
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(index, line)| (!line.starts_with("meta\t")).then_some(index))
        .expect("fixture has observation records");
    let records = format!("{}\n", lines[record_start..].join("\n"));
    let record_digest = domain_sha256(OBSERVATION_DOMAIN, records.as_bytes());
    if let Some(line) = lines
        .iter_mut()
        .find(|line| line.starts_with("meta\tobservation-record-sha256\t"))
    {
        *line = format!("meta\tobservation-record-sha256\t{record_digest}");
    }
    let unsigned = format!("{}\n", lines.join("\n"));
    let receipt_digest = domain_sha256(RECEIPT_DOMAIN, unsigned.as_bytes());
    format!("{unsigned}receipt-sha256\t{receipt_digest}\n")
}
