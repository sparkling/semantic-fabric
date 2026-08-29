use super::*;

use std::cell::Cell;

use crate::binary_artifact_receipt::runtime_linkage::object_authority::prepared_probe::{
    ExpectedBwrapIdentity, ExpectedRuntimeElfPolicy, PreparedRuntimeProbe,
};
use crate::binary_artifact_receipt::RUNTIME_ELF_POLICY;

const PINNED_POLICY_SHA256: &str =
    "cd23f2d883c1e99b655395284e7d803e6d00b9eaf90a417560efca7ffde50b0a";

fn tool_identity(fixture: &Fixture) -> ExpectedBwrapIdentity {
    let bytes = b"fixture bubblewrap executable";
    ExpectedBwrapIdentity::new(
        fixture.plan.executable(),
        &format!("{:x}", Sha256::digest(bytes)),
        bytes.len() as u64,
        "test-fixture-static-bytes-v1",
    )
    .unwrap()
}

fn stale_policies() -> [(&'static str, ExpectedRuntimeElfPolicy); 2] {
    [
        (
            "id",
            ExpectedRuntimeElfPolicy::new("different-runtime-elf-policy-v1", PINNED_POLICY_SHA256)
                .unwrap(),
        ),
        (
            "digest",
            ExpectedRuntimeElfPolicy::new(RUNTIME_ELF_POLICY, &"0".repeat(64)).unwrap(),
        ),
    ]
}

#[test]
fn runtime_elf_policy_id_or_digest_drift_rejects_before_probe_construction() {
    for (field, expected) in stale_policies() {
        let fixture = Fixture::new(&format!("runtime-elf-policy-prepare-{field}"));
        let held = hold_runtime_inputs(&fixture.pair, &fixture.plan, &fixture.view).unwrap();
        let error =
            PreparedRuntimeProbe::prepare_with_test_tool(held, tool_identity(&fixture), expected)
                .unwrap_err();
        assert!(error.contains("pinned expectation"), "{field}: {error}");
    }
}

#[test]
fn runtime_elf_policy_id_or_digest_drift_rejects_before_runner_invocation() {
    for (field, expected) in stale_policies() {
        let fixture = Fixture::new(&format!("runtime-elf-policy-run-{field}"));
        let held = hold_runtime_inputs(&fixture.pair, &fixture.plan, &fixture.view).unwrap();
        let mut probe = PreparedRuntimeProbe::prepare_with_test_tool(
            held,
            tool_identity(&fixture),
            fixture_runtime_elf_policy(),
        )
        .unwrap();
        probe.replace_expected_runtime_elf_policy_for_test(expected);
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
}
