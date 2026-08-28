use std::fmt;

use super::format::{receipt_sha256, render_receipt, CommittedBaseline};
use super::model::{MetricId, PerformanceReceipt, ReceiptKind, ScenarioObservation};

pub const MEDIAN_REGRESSION_LIMIT_PERCENT: u8 = 5;
pub const P95_REGRESSION_LIMIT_PERCENT: u8 = 10;
pub const MEMORY_SCALING_LIMIT_PERCENT: u8 = 10;
const REGRESSION_LIMIT_ORIGIN: &str = "ADR-0038-M4";
const MEMORY_SCALING_LIMIT_ORIGIN: &str = "ADR-0038-M1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonVerdict {
    Pass,
    Regression,
    Inconclusive,
}

impl ComparisonVerdict {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Regression => "regression",
            Self::Inconclusive => "inconclusive",
        }
    }
}

impl fmt::Display for ComparisonVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateVerdict {
    Pass,
    Regression,
}

impl fmt::Display for GateVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pass => "pass",
            Self::Regression => "regression",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioComparison {
    pub scenario_id: String,
    pub baseline_median_x2: u128,
    pub candidate_median_x2: u128,
    pub baseline_p95: u64,
    pub candidate_p95: u64,
    pub median_verdict: GateVerdict,
    pub p95_verdict: GateVerdict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalingComparison {
    pub metric: MetricId,
    pub scale_from: u32,
    pub scale_to: u32,
    pub median_from_x2: u128,
    pub median_to_x2: u128,
    pub p95_from: u64,
    pub p95_to: u64,
    pub median_verdict: GateVerdict,
    pub p95_verdict: GateVerdict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComparisonReceipt {
    pub baseline_sha256: String,
    pub candidate_sha256: String,
    pub verdict: ComparisonVerdict,
    pub reason: String,
    pub scenarios: Vec<ScenarioComparison>,
    pub scaling: Vec<ScalingComparison>,
    pub informational: Vec<String>,
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
        verdict: ComparisonVerdict::Pass,
        reason: "all authoritative regression and memory-scaling gates passed".into(),
        scenarios: Vec::new(),
        scaling: Vec::new(),
        informational: vec![
            "fresh-process RSS excludes relational fixture generation but includes worker startup, mapping/schema setup, execution, and teardown"
                .into(),
            "median and p95 baseline-relative thresholds are the future ADR-0038 M4 acceptance limits; this M0 tool records machinery, not a passed M4 gate"
                .into(),
            "latency cross-scale boundedness has no programme-authoritative limit; baseline-relative gates only"
                .into(),
            "fresh-process RSS samples do not establish soak monotonicity; soak comparison is unmeasured"
                .into(),
            "heap and RSS 10x-to-100x scaling apply the ADR-0038 M1 10% limit independently to median and p95; both gate"
                .into(),
            "raw samples retain variance evidence, but no numeric variance threshold is authoritative; variance is informational"
                .into(),
        ],
    };
    let expected = baseline.receipt();
    if expected.runner != candidate.runner {
        return Ok(inconclusive(output, "runner profile mismatch"));
    }
    if expected.source.workload_sha256 != candidate.source.workload_sha256 {
        return Ok(inconclusive(output, "workload digest mismatch"));
    }
    if expected.observations.len() != candidate.observations.len() {
        return Ok(inconclusive(output, "scenario set mismatch"));
    }
    for (left, right) in expected.observations.iter().zip(&candidate.observations) {
        if left.config != right.config {
            return Ok(inconclusive(output, "scenario configuration mismatch"));
        }
        output.scenarios.push(compare_scenario(left, right));
    }
    if let Err(reason) = add_memory_scaling(&mut output, candidate) {
        return Ok(inconclusive(output, reason));
    }
    let regressed = output.scenarios.iter().any(|scenario| {
        scenario.median_verdict == GateVerdict::Regression
            || scenario.p95_verdict == GateVerdict::Regression
    }) || output.scaling.iter().any(|scaling| {
        scaling.median_verdict == GateVerdict::Regression
            || scaling.p95_verdict == GateVerdict::Regression
    });
    if regressed {
        output.verdict = ComparisonVerdict::Regression;
        output.reason =
            "one or more authoritative regression or memory-scaling gates failed".into();
    }
    Ok(output)
}

fn compare_scenario(
    baseline: &ScenarioObservation,
    candidate: &ScenarioObservation,
) -> ScenarioComparison {
    ScenarioComparison {
        scenario_id: baseline.config.id.clone(),
        baseline_median_x2: baseline.summary.median.doubled_ns(),
        candidate_median_x2: candidate.summary.median.doubled_ns(),
        baseline_p95: baseline.summary.p95,
        candidate_p95: candidate.summary.p95,
        median_verdict: gate(
            candidate.summary.median.doubled_ns(),
            baseline.summary.median.doubled_ns(),
            MEDIAN_REGRESSION_LIMIT_PERCENT,
        ),
        p95_verdict: gate(
            u128::from(candidate.summary.p95),
            u128::from(baseline.summary.p95),
            P95_REGRESSION_LIMIT_PERCENT,
        ),
    }
}

fn add_memory_scaling(
    output: &mut ComparisonReceipt,
    candidate: &PerformanceReceipt,
) -> Result<(), &'static str> {
    for metric in [
        MetricId::HeapRustRequestedLiveDelta,
        MetricId::RssLinuxProcessPeak,
    ] {
        let from = candidate.observations.iter().find(|observation| {
            observation.config.metric == metric && observation.config.scale == 10
        });
        let to = candidate.observations.iter().find(|observation| {
            observation.config.metric == metric && observation.config.scale == 100
        });
        match (from, to) {
            (Some(from), Some(to)) => output.scaling.push(ScalingComparison {
                metric,
                scale_from: 10,
                scale_to: 100,
                median_from_x2: from.summary.median.doubled_ns(),
                median_to_x2: to.summary.median.doubled_ns(),
                p95_from: from.summary.p95,
                p95_to: to.summary.p95,
                median_verdict: gate(
                    to.summary.median.doubled_ns(),
                    from.summary.median.doubled_ns(),
                    MEMORY_SCALING_LIMIT_PERCENT,
                ),
                p95_verdict: gate(
                    u128::from(to.summary.p95),
                    u128::from(from.summary.p95),
                    MEMORY_SCALING_LIMIT_PERCENT,
                ),
            }),
            (None, None) => output.informational.push(format!(
                "{metric} 10x-to-100x scaling gate is not applicable to this scenario set"
            )),
            _ => return Err("memory scaling scenario set is incomplete"),
        }
    }
    Ok(())
}

fn gate(candidate: u128, baseline: u128, limit_percent: u8) -> GateVerdict {
    if within_percent(candidate, baseline, limit_percent) {
        GateVerdict::Pass
    } else {
        GateVerdict::Regression
    }
}

pub fn within_percent(candidate: u128, baseline: u128, limit_percent: u8) -> bool {
    candidate * 100 <= baseline * (100 + u128::from(limit_percent))
}

fn inconclusive(mut output: ComparisonReceipt, reason: &str) -> ComparisonReceipt {
    output.verdict = ComparisonVerdict::Inconclusive;
    output.reason = reason.into();
    output.scenarios.clear();
    output.scaling.clear();
    output
}

pub fn render_comparison(comparison: &ComparisonReceipt) -> Result<String, CompareError> {
    validate_tsv("reason", &comparison.reason)?;
    let mut output = String::from("sf-performance-comparison-v2\n");
    output.push_str(&format!(
        "baseline-sha256\t{}\ncandidate-sha256\t{}\nverdict\t{}\nreason\t{}\nmedian-limit-percent\t{}\np95-limit-percent\t{}\nregression-limit-origin\t{}\nmemory-scaling-limit-percent\t{}\nmemory-scaling-limit-origin\t{}\nscenario-count\t{}\n",
        comparison.baseline_sha256,
        comparison.candidate_sha256,
        comparison.verdict,
        comparison.reason,
        MEDIAN_REGRESSION_LIMIT_PERCENT,
        P95_REGRESSION_LIMIT_PERCENT,
        REGRESSION_LIMIT_ORIGIN,
        MEMORY_SCALING_LIMIT_PERCENT,
        MEMORY_SCALING_LIMIT_ORIGIN,
        comparison.scenarios.len()
    ));
    for scenario in &comparison.scenarios {
        output.push_str(&format!(
            "scenario\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            scenario.scenario_id,
            scenario.baseline_median_x2,
            scenario.candidate_median_x2,
            scenario.baseline_p95,
            scenario.candidate_p95,
            scenario.median_verdict,
            scenario.p95_verdict
        ));
    }
    output.push_str(&format!("scaling-count\t{}\n", comparison.scaling.len()));
    for scaling in &comparison.scaling {
        output.push_str(&format!(
            "scaling\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            scaling.metric,
            scaling.scale_from,
            scaling.scale_to,
            scaling.median_from_x2,
            scaling.median_to_x2,
            scaling.p95_from,
            scaling.p95_to,
            scaling.median_verdict,
            scaling.p95_verdict
        ));
    }
    output.push_str(&format!(
        "informational-count\t{}\n",
        comparison.informational.len()
    ));
    for note in &comparison.informational {
        validate_tsv("informational note", note)?;
        output.push_str(&format!("informational\t{note}\n"));
    }
    Ok(output)
}

fn validate_tsv(label: &str, value: &str) -> Result<(), CompareError> {
    if value.contains(['\t', '\n', '\r']) {
        Err(CompareError(format!("comparison {label} is not TSV-safe")))
    } else {
        Ok(())
    }
}
