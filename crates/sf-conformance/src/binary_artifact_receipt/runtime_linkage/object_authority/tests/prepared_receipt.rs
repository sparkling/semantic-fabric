use super::*;

use crate::binary_artifact_receipt::runtime_linkage::object_authority::prepared_probe::{
    ExpectedBwrapIdentity, PreparedRuntimeObservation, PreparedRuntimeProbe,
};
use crate::binary_artifact_receipt::runtime_linkage::prepared_receipt::{parse, render};

fn observation(name: &str) -> PreparedRuntimeObservation {
    let fixture = Fixture::new(name);
    let held = hold_runtime_inputs(&fixture.pair, &fixture.plan, &fixture.view).unwrap();
    let tool_bytes = b"fixture bubblewrap executable";
    let expected = ExpectedBwrapIdentity::new(
        fixture.plan.executable(),
        &format!("{:x}", Sha256::digest(tool_bytes)),
        tool_bytes.len() as u64,
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
    .finish_for_test(synthetic_output(), Vec::new())
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

#[test]
fn canonical_receipt_roundtrips_and_replays_without_execution() {
    let expected = observation("receipt-roundtrip");
    assert!(!expected.bwrap_path.exists());
    let receipt = expected.to_non_admission_receipt().unwrap();
    let bytes = render(&receipt).unwrap();
    let parsed = parse(&bytes).unwrap();

    assert_eq!(render(&parsed).unwrap(), bytes);
    assert_eq!(parsed, receipt);
    assert_eq!(parsed.semantic_replay().unwrap(), expected.view);
}

#[test]
fn live_conversion_rejects_prepared_policy_detachment() {
    let mut observation = observation("receipt-policy-drift");
    observation.loader_policy = "drifted-prepared-policy-v1";
    assert!(observation.to_non_admission_receipt().is_err());
}

#[test]
fn live_conversion_rejects_runtime_elf_policy_detachment() {
    let mut policy_observation = observation("receipt-runtime-elf-policy-drift");
    policy_observation.runtime_elf_policy = "drifted-runtime-elf-policy-v1";
    assert!(policy_observation.to_non_admission_receipt().is_err());

    let mut digest_observation = observation("receipt-runtime-elf-digest-drift");
    digest_observation.runtime_elf_policy_sha256 = "0".repeat(64);
    assert!(digest_observation.to_non_admission_receipt().is_err());
}

#[test]
fn live_conversion_rejects_seccomp_policy_detachment_without_serializing_it() {
    let mut id_observation = observation("receipt-seccomp-id-drift");
    id_observation.seccomp_policy = "drifted-seccomp-policy-v1";
    assert!(id_observation.to_non_admission_receipt().is_err());

    let mut digest_observation = observation("receipt-seccomp-digest-drift");
    digest_observation.seccomp_policy_sha256 = "0".repeat(64);
    assert!(digest_observation.to_non_admission_receipt().is_err());

    let mut length_observation = observation("receipt-seccomp-length-drift");
    length_observation.seccomp_policy_byte_length += 8;
    assert!(length_observation.to_non_admission_receipt().is_err());
}
