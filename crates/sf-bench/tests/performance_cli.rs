use std::path::{Path, PathBuf};
use std::process::Command;

use sf_bench::performance::config::parse_scenarios;
use sf_bench::performance::format::{receipt_sha256, render_receipt};
use sf_bench::performance::model::{
    PerformanceReceipt, ReceiptKind, RunnerBinding, ScenarioObservation, SourceBinding, SourceTree,
    M0_SAMPLE_COUNT,
};
use sf_bench::performance::paths::{BASELINE_PATH, CANDIDATE_PATH, PROFILE_PATH, SCENARIOS_PATH};
use sf_bench::performance::profile::{render_profile, RunnerProfile};
use sf_bench::performance::workload_runner::workload_sha256;

const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn receipt(kind: ReceiptKind, value: u64, profile_sha256: &str, workload: &str) -> String {
    let observations = parse_scenarios(include_bytes!("../config/performance-scenarios-v1.tsv"))
        .unwrap()
        .into_iter()
        .map(|config| ScenarioObservation::new(config, vec![value; M0_SAMPLE_COUNT]).unwrap())
        .collect();
    render_receipt(
        &PerformanceReceipt::new(
            kind,
            RunnerBinding::new("controlled-linux-test-v1", profile_sha256).unwrap(),
            SourceBinding::new(
                "0123456789abcdef0123456789abcdef01234567",
                SourceTree::Clean,
                DIGEST,
                workload,
            )
            .unwrap(),
            observations,
        )
        .unwrap(),
    )
    .unwrap()
}

fn repository_fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    dir
}

fn write_fixed(root: &Path, relative: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    path
}

fn write_authorities(root: &Path) -> (String, String) {
    let manifest = include_bytes!("../config/performance-scenarios-v1.tsv");
    write_fixed(root, SCENARIOS_PATH, manifest);
    let profile = RunnerProfile {
        profile_id: "controlled-linux-test-v1".into(),
        controlled: true,
        os: "linux".into(),
        architecture: "x86_64".into(),
        kernel_release: "6.12.1-controlled".into(),
        cpu_model: "Synthetic CPU".into(),
        online_cpus: "0-7".into(),
        allowed_cpus: "6-7".into(),
        isolated_cpus: "6-7".into(),
        scaling_governor: "performance".into(),
        turbo: "disabled".into(),
        swap_total_kib: 0,
        mem_total_kib: 67_108_864,
        load1_limit_milli: 250,
        build_profile: "release".into(),
    };
    let profile_text = render_profile(&profile).unwrap();
    write_fixed(root, PROFILE_PATH, profile_text);
    (
        profile.digest().unwrap(),
        workload_sha256(manifest).unwrap(),
    )
}

fn run(root: &Path, command: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sf-performance-receipt"))
        .arg(command)
        .current_dir(root)
        .output()
        .unwrap()
}

#[test]
fn should_check_the_fixed_tracked_scenario_manifest_without_capturing() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let output = run(root, "check-scenarios");

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "scenarios\t17\n");
}

#[test]
fn should_check_the_fixed_baseline_without_modifying_it() {
    let dir = repository_fixture();
    let (profile, workload) = write_authorities(dir.path());
    let baseline = receipt(ReceiptKind::Baseline, 100, &profile, &workload);
    let path = write_fixed(dir.path(), BASELINE_PATH, &baseline);

    let output = run(dir.path(), "check-baseline");

    assert!(output.status.success());
    assert_eq!(std::fs::read_to_string(path).unwrap(), baseline);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("baseline-sha256\t{}\n", receipt_sha256(&baseline))
    );
}

#[test]
fn should_compare_only_the_fixed_baseline_and_candidate_paths() {
    let dir = repository_fixture();
    let (profile, workload) = write_authorities(dir.path());
    let baseline = receipt(ReceiptKind::Baseline, 100, &profile, &workload);
    write_fixed(dir.path(), BASELINE_PATH, &baseline);
    write_fixed(
        dir.path(),
        CANDIDATE_PATH,
        receipt(ReceiptKind::Candidate, 105, &profile, &workload),
    );

    let output = run(dir.path(), "compare");
    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(output.status.success());
    assert!(stdout.starts_with("sf-performance-comparison-v2\n"));
    assert!(stdout.contains("verdict\tpass\n"));
    assert!(stdout.contains(&format!("baseline-sha256\t{}\n", receipt_sha256(&baseline))));
}

#[test]
fn should_exit_nonzero_when_comparison_reports_regression() {
    let dir = repository_fixture();
    let (profile, workload) = write_authorities(dir.path());
    write_fixed(
        dir.path(),
        BASELINE_PATH,
        receipt(ReceiptKind::Baseline, 100, &profile, &workload),
    );
    write_fixed(
        dir.path(),
        CANDIDATE_PATH,
        receipt(ReceiptKind::Candidate, 106, &profile, &workload),
    );

    let output = run(dir.path(), "compare");

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("verdict\tregression\n"));
}

#[test]
fn should_reject_a_baseline_copied_into_the_candidate_path() {
    let dir = repository_fixture();
    let (profile, workload) = write_authorities(dir.path());
    let baseline = receipt(ReceiptKind::Baseline, 100, &profile, &workload);
    write_fixed(dir.path(), BASELINE_PATH, &baseline);
    write_fixed(dir.path(), CANDIDATE_PATH, &baseline);

    let output = run(dir.path(), "compare");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("wrong receipt kind"));
}

#[test]
fn should_reject_a_receipt_bound_to_a_stale_workload() {
    let dir = repository_fixture();
    let (profile, _workload) = write_authorities(dir.path());
    write_fixed(
        dir.path(),
        BASELINE_PATH,
        receipt(ReceiptKind::Baseline, 100, &profile, DIGEST),
    );

    let output = run(dir.path(), "check-baseline");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("workload binding"));
}

#[test]
fn should_fail_before_capture_when_the_fixed_profile_is_missing() {
    let dir = repository_fixture();
    write_fixed(
        dir.path(),
        SCENARIOS_PATH,
        include_bytes!("../config/performance-scenarios-v1.tsv"),
    );

    let output = run(dir.path(), "capture-baseline");

    assert_eq!(output.status.code(), Some(2));
    assert!(!dir.path().join(BASELINE_PATH).exists());
}

#[test]
fn should_refuse_to_overwrite_an_existing_baseline_before_capture() {
    let dir = repository_fixture();
    let original = b"do-not-overwrite\n";
    let path = write_fixed(dir.path(), BASELINE_PATH, original);

    let output = run(dir.path(), "capture-baseline");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
fn should_reject_legacy_arbitrary_path_arguments() {
    let dir = repository_fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_sf-performance-receipt"))
        .args(["check-baseline", "/tmp/untrusted-baseline.tsv"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
}
