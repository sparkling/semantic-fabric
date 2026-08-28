use std::process::Command;

use sf_bench::performance::format::{receipt_sha256, render_receipt};
use sf_bench::performance::model::{
    BoundaryId, MetricId, PerformanceReceipt, ReceiptKind, RunnerBinding, ScenarioConfig,
    ScenarioObservation, SourceBinding, SourceTree, Unit, M0_SAMPLE_COUNT,
};

const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn receipt(kind: ReceiptKind) -> String {
    let config = ScenarioConfig::new(
        "synthetic.latency.scale1",
        1,
        MetricId::LatencyFullResult,
        BoundaryId::SqliteParseTranslateExecuteCollect,
        Unit::Nanoseconds,
        3,
        M0_SAMPLE_COUNT,
    )
    .unwrap();
    let observation =
        ScenarioObservation::new(config, (1..=M0_SAMPLE_COUNT as u64).collect()).unwrap();
    render_receipt(
        &PerformanceReceipt::new(
            kind,
            RunnerBinding::new("controlled-linux-v1", DIGEST).unwrap(),
            SourceBinding::new(
                "0123456789abcdef0123456789abcdef01234567",
                SourceTree::Clean,
                DIGEST,
                DIGEST,
            )
            .unwrap(),
            vec![observation],
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn should_check_tracked_scenario_manifest_without_capturing() {
    let path = format!(
        "{}/config/performance-scenarios-v1.tsv",
        env!("CARGO_MANIFEST_DIR")
    );

    let output = Command::new(env!("CARGO_BIN_EXE_sf-performance-receipt"))
        .args(["check-scenarios", &path])
        .output()
        .expect("run receipt CLI");

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "scenarios\t17\n");
}

#[test]
fn should_check_committed_baseline_without_modifying_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("baseline.tsv");
    let baseline = receipt(ReceiptKind::Baseline);
    std::fs::write(&path, &baseline).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_sf-performance-receipt"))
        .arg("check-baseline")
        .arg(&path)
        .output()
        .expect("run receipt CLI");

    assert!(output.status.success());
    assert_eq!(std::fs::read_to_string(path).unwrap(), baseline);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("baseline-sha256\t{}\n", receipt_sha256(&baseline))
    );
}

#[test]
fn should_emit_ephemeral_comparison_that_references_baseline_digest() {
    let dir = tempfile::tempdir().unwrap();
    let baseline_path = dir.path().join("baseline.tsv");
    let candidate_path = dir.path().join("candidate.tsv");
    let baseline = receipt(ReceiptKind::Baseline);
    std::fs::write(&baseline_path, &baseline).unwrap();
    std::fs::write(&candidate_path, receipt(ReceiptKind::Candidate)).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_sf-performance-receipt"))
        .arg("compare")
        .arg(&baseline_path)
        .arg(&candidate_path)
        .output()
        .expect("run receipt CLI");
    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(output.status.success());
    assert!(stdout.starts_with("sf-performance-comparison-v1\n"));
    assert!(stdout.contains(&format!("baseline-sha256\t{}\n", receipt_sha256(&baseline))));
}
