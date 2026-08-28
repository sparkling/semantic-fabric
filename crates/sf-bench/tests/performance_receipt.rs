use sf_bench::performance::compare::{compare, ComparisonVerdict};
use sf_bench::performance::config::{parse_scenarios, render_scenarios};
use sf_bench::performance::format::{
    parse_receipt, receipt_sha256, render_receipt, CommittedBaseline, MAX_RECEIPT_BYTES,
};
use sf_bench::performance::model::{
    BoundaryId, MetricId, PerformanceReceipt, ReceiptKind, RunnerBinding, ScenarioConfig,
    ScenarioObservation, SourceBinding, SourceTree, Unit, M0_SAMPLE_COUNT,
};

const PROFILE_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PROFILE_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const ARTIFACT: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const WORKLOAD: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

fn observation() -> ScenarioObservation {
    let config = ScenarioConfig::new(
        "gtfs.sqlite.q1.scale1",
        1,
        MetricId::LatencyFullResult,
        BoundaryId::SqliteParseTranslateExecuteCollect,
        Unit::Nanoseconds,
        3,
        M0_SAMPLE_COUNT,
    )
    .expect("valid scenario");
    ScenarioObservation::new(config, (1..=M0_SAMPLE_COUNT as u64).collect())
        .expect("fifty raw samples")
}

fn receipt(kind: ReceiptKind, profile_sha256: &str, tree: SourceTree) -> PerformanceReceipt {
    PerformanceReceipt::new(
        kind,
        RunnerBinding::new("linux-controlled-v1", profile_sha256).expect("runner binding"),
        SourceBinding::new(
            "0123456789abcdef0123456789abcdef01234567",
            tree,
            ARTIFACT,
            WORKLOAD,
        )
        .expect("source binding"),
        vec![observation()],
    )
    .expect("receipt")
}

#[test]
fn should_require_fifty_samples_in_m0_scenario_config() {
    let result = ScenarioConfig::new(
        "gtfs.sqlite.q1.scale1",
        1,
        MetricId::LatencyFullResult,
        BoundaryId::SqliteParseTranslateExecuteCollect,
        Unit::Nanoseconds,
        3,
        49,
    );

    assert!(result.is_err());
}

#[test]
fn should_reject_metric_boundary_mismatch() {
    let result = ScenarioConfig::new(
        "gtfs.sqlite.construct.first.scale1",
        1,
        MetricId::LatencyFirstResult,
        BoundaryId::SqliteParseTranslateExecuteCollect,
        Unit::Nanoseconds,
        3,
        M0_SAMPLE_COUNT,
    );

    assert!(result.is_err());
}

#[test]
fn should_keep_tracked_scenario_manifest_canonical_and_fifty_sampled() {
    let bytes = include_bytes!("../config/performance-scenarios-v1.tsv");
    let scenarios = parse_scenarios(bytes).expect("tracked scenario manifest");

    assert!(scenarios
        .iter()
        .all(|scenario| scenario.sample_count == M0_SAMPLE_COUNT));
    assert_eq!(
        render_scenarios(&scenarios)
            .expect("render scenarios")
            .as_bytes(),
        bytes
    );
}

#[test]
fn should_round_trip_canonical_receipt_without_changing_bytes() {
    let original = receipt(ReceiptKind::Candidate, PROFILE_A, SourceTree::Clean);
    let text = render_receipt(&original).expect("render canonical receipt");

    let parsed = parse_receipt(text.as_bytes()).expect("parse canonical receipt");

    assert_eq!(render_receipt(&parsed).expect("re-render"), text);
}

#[test]
fn should_reject_receipt_larger_than_parser_bound() {
    let oversized = vec![b'x'; MAX_RECEIPT_BYTES + 1];

    let result = parse_receipt(&oversized);

    assert!(result.is_err());
}

#[test]
fn should_compute_standard_sha256_digest_without_a_provider() {
    assert_eq!(
        receipt_sha256("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn should_reject_tampered_derived_statistic() {
    let text = render_receipt(&receipt(
        ReceiptKind::Candidate,
        PROFILE_A,
        SourceTree::Clean,
    ))
    .expect("render");
    let tampered = text.replacen("\t25.5\t48\t", "\t25\t48\t", 1);

    let result = parse_receipt(tampered.as_bytes());

    assert!(result.is_err());
}

#[test]
fn should_refuse_dirty_receipt_as_committed_baseline() {
    let text = render_receipt(&receipt(
        ReceiptKind::Baseline,
        PROFILE_A,
        SourceTree::Dirty,
    ));

    assert!(text.is_err());
}

#[test]
fn should_make_runner_profile_mismatch_inconclusive() {
    let baseline_text = render_receipt(&receipt(
        ReceiptKind::Baseline,
        PROFILE_A,
        SourceTree::Clean,
    ))
    .expect("baseline");
    let baseline = CommittedBaseline::parse(baseline_text.as_bytes()).expect("immutable baseline");
    let candidate = receipt(ReceiptKind::Candidate, PROFILE_B, SourceTree::Clean);

    let comparison = compare(&baseline, &candidate).expect("comparison receipt");

    assert_eq!(comparison.verdict, ComparisonVerdict::Inconclusive);
}

#[test]
fn should_reference_immutable_baseline_digest_in_comparison() {
    let baseline_text = render_receipt(&receipt(
        ReceiptKind::Baseline,
        PROFILE_A,
        SourceTree::Clean,
    ))
    .expect("baseline");
    let baseline = CommittedBaseline::parse(baseline_text.as_bytes()).expect("immutable baseline");
    let candidate = receipt(ReceiptKind::Candidate, PROFILE_A, SourceTree::Clean);

    let comparison = compare(&baseline, &candidate).expect("comparison receipt");

    assert_eq!(comparison.baseline_sha256, receipt_sha256(&baseline_text));
}
