use std::fmt;

use super::format::{receipt_sha256, render_receipt, CommittedBaseline};
use super::model::{PerformanceReceipt, ReceiptKind, ScenarioObservation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonVerdict {
    Comparable,
    Inconclusive,
}

impl ComparisonVerdict {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Comparable => "comparable",
            Self::Inconclusive => "inconclusive",
        }
    }
}

impl fmt::Display for ComparisonVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioComparison {
    pub scenario_id: String,
    pub baseline_median_x2: u128,
    pub candidate_median_x2: u128,
    pub baseline_p95: u64,
    pub candidate_p95: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComparisonReceipt {
    pub baseline_sha256: String,
    pub candidate_sha256: String,
    pub verdict: ComparisonVerdict,
    pub reason: String,
    pub scenarios: Vec<ScenarioComparison>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareError(pub String);

impl fmt::Display for CompareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CompareError {}

pub fn compare(
    baseline: &CommittedBaseline,
    candidate: &PerformanceReceipt,
) -> Result<ComparisonReceipt, CompareError> {
    if candidate.kind != ReceiptKind::Candidate {
        return Err(CompareError(
            "comparison input must be a candidate receipt".into(),
        ));
    }
    let candidate_text =
        render_receipt(candidate).map_err(|error| CompareError(error.to_string()))?;
    let mut output = ComparisonReceipt {
        baseline_sha256: baseline.sha256().to_owned(),
        candidate_sha256: receipt_sha256(&candidate_text),
        verdict: ComparisonVerdict::Comparable,
        reason: "exact runner profile and workload binding match".into(),
        scenarios: Vec::new(),
    };
    let expected = baseline.receipt();
    if expected.runner.profile_sha256 != candidate.runner.profile_sha256
        || expected.runner.profile_id != candidate.runner.profile_id
    {
        output.verdict = ComparisonVerdict::Inconclusive;
        output.reason = "runner profile mismatch".into();
        return Ok(output);
    }
    if expected.source.workload_sha256 != candidate.source.workload_sha256 {
        output.verdict = ComparisonVerdict::Inconclusive;
        output.reason = "workload digest mismatch".into();
        return Ok(output);
    }
    if expected.observations.len() != candidate.observations.len() {
        output.verdict = ComparisonVerdict::Inconclusive;
        output.reason = "scenario set mismatch".into();
        return Ok(output);
    }
    for (left, right) in expected.observations.iter().zip(&candidate.observations) {
        if !same_scenario(left, right) {
            output.verdict = ComparisonVerdict::Inconclusive;
            output.reason = "scenario configuration mismatch".into();
            output.scenarios.clear();
            return Ok(output);
        }
        output.scenarios.push(ScenarioComparison {
            scenario_id: left.config.id.clone(),
            baseline_median_x2: left.summary.median.doubled_ns(),
            candidate_median_x2: right.summary.median.doubled_ns(),
            baseline_p95: left.summary.p95,
            candidate_p95: right.summary.p95,
        });
    }
    Ok(output)
}

fn same_scenario(left: &ScenarioObservation, right: &ScenarioObservation) -> bool {
    left.config == right.config
}

pub fn render_comparison(comparison: &ComparisonReceipt) -> Result<String, CompareError> {
    if comparison.reason.contains(['\t', '\n', '\r']) {
        return Err(CompareError("comparison reason is not TSV-safe".into()));
    }
    let mut output = String::from("sf-performance-comparison-v1\n");
    output.push_str(&format!(
        "baseline-sha256\t{}\ncandidate-sha256\t{}\nverdict\t{}\nreason\t{}\nscenario-count\t{}\n",
        comparison.baseline_sha256,
        comparison.candidate_sha256,
        comparison.verdict,
        comparison.reason,
        comparison.scenarios.len()
    ));
    for scenario in &comparison.scenarios {
        output.push_str(&format!(
            "scenario\t{}\t{}\t{}\t{}\t{}\n",
            scenario.scenario_id,
            scenario.baseline_median_x2,
            scenario.candidate_median_x2,
            scenario.baseline_p95,
            scenario.candidate_p95
        ));
    }
    Ok(output)
}
