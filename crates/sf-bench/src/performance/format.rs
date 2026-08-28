use std::fmt;
use std::str::FromStr;

use super::digest::sha256_hex;
use super::model::{
    BoundaryId, MetricId, PerformanceReceipt, ReceiptKind, RunnerBinding, ScenarioConfig,
    ScenarioObservation, SourceBinding, SourceTree, Unit, MAX_SCENARIOS,
};

const MAGIC: &str = "sf-performance-receipt-v1";
pub const MAX_RECEIPT_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError(pub String);

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FormatError {}

pub fn render_receipt(receipt: &PerformanceReceipt) -> Result<String, FormatError> {
    if receipt.kind == ReceiptKind::Baseline && receipt.source.tree != SourceTree::Clean {
        return Err(FormatError(
            "a committed baseline must bind a clean source tree".into(),
        ));
    }
    let mut output = String::new();
    output.push_str(MAGIC);
    output.push('\n');
    field(&mut output, "kind", receipt.kind);
    field(&mut output, "runner-profile-id", &receipt.runner.profile_id);
    field(
        &mut output,
        "runner-profile-sha256",
        &receipt.runner.profile_sha256,
    );
    field(&mut output, "source-commit", &receipt.source.commit);
    field(&mut output, "source-tree", receipt.source.tree);
    field(
        &mut output,
        "artifact-sha256",
        &receipt.source.artifact_sha256,
    );
    field(
        &mut output,
        "workload-sha256",
        &receipt.source.workload_sha256,
    );
    field(&mut output, "scenario-count", receipt.observations.len());
    for observation in &receipt.observations {
        let raw = observation
            .raw_samples
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        output.push_str(&format!(
            "scenario\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            observation.config.id,
            observation.config.scale,
            observation.config.metric,
            observation.config.boundary,
            observation.config.unit,
            observation.config.warmup_count,
            observation.config.sample_count,
            observation.summary.median,
            observation.summary.p95,
            raw
        ));
    }
    if output.len() > MAX_RECEIPT_BYTES {
        return Err(FormatError("rendered receipt exceeds byte bound".into()));
    }
    Ok(output)
}

pub fn parse_receipt(bytes: &[u8]) -> Result<PerformanceReceipt, FormatError> {
    if bytes.len() > MAX_RECEIPT_BYTES {
        return Err(FormatError("receipt exceeds byte bound".into()));
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| FormatError("receipt is not UTF-8".into()))?;
    if !text.ends_with('\n') || text.contains('\r') {
        return Err(FormatError(
            "receipt must use canonical LF termination".into(),
        ));
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.first() != Some(&MAGIC) || lines.len() < 10 {
        return Err(FormatError("invalid receipt header".into()));
    }

    let kind = parse_field::<ReceiptKind>(&lines, 1, "kind")?;
    let profile_id = text_field(&lines, 2, "runner-profile-id")?;
    let profile_sha256 = text_field(&lines, 3, "runner-profile-sha256")?;
    let commit = text_field(&lines, 4, "source-commit")?;
    let tree = parse_field::<SourceTree>(&lines, 5, "source-tree")?;
    let artifact_sha256 = text_field(&lines, 6, "artifact-sha256")?;
    let workload_sha256 = text_field(&lines, 7, "workload-sha256")?;
    let count = text_field(&lines, 8, "scenario-count")?
        .parse::<usize>()
        .map_err(|_| FormatError("invalid scenario count".into()))?;
    if count == 0 || count > MAX_SCENARIOS || lines.len() != 9 + count {
        return Err(FormatError(
            "scenario count does not match bounded body".into(),
        ));
    }

    let mut observations = Vec::with_capacity(count);
    for line in &lines[9..] {
        observations.push(parse_scenario(line)?);
    }
    let receipt = PerformanceReceipt::new(
        kind,
        RunnerBinding::new(profile_id, profile_sha256).map_err(model_error)?,
        SourceBinding::new(commit, tree, artifact_sha256, workload_sha256).map_err(model_error)?,
        observations,
    )
    .map_err(model_error)?;
    if render_receipt(&receipt)? != text {
        return Err(FormatError(
            "receipt is not in canonical order or form".into(),
        ));
    }
    Ok(receipt)
}

fn parse_scenario(line: &str) -> Result<ScenarioObservation, FormatError> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() != 11 || fields[0] != "scenario" {
        return Err(FormatError("invalid scenario record".into()));
    }
    let scale = parse_number(fields[2], "scale")?;
    let warmup_count = parse_number(fields[6], "warmup count")?;
    let sample_count = parse_number(fields[7], "sample count")?;
    let config = ScenarioConfig::new(
        fields[1],
        scale,
        fields[3].parse::<MetricId>().map_err(model_error)?,
        fields[4].parse::<BoundaryId>().map_err(model_error)?,
        fields[5].parse::<Unit>().map_err(model_error)?,
        warmup_count,
        sample_count,
    )
    .map_err(model_error)?;
    let raw = fields[10]
        .split(',')
        .map(|value| parse_number(value, "raw sample"))
        .collect::<Result<Vec<u64>, _>>()?;
    let observation = ScenarioObservation::new(config, raw).map_err(model_error)?;
    if fields[8] != observation.summary.median.to_string()
        || fields[9] != observation.summary.p95.to_string()
    {
        return Err(FormatError(
            "derived statistics do not match raw samples".into(),
        ));
    }
    Ok(observation)
}

fn field(output: &mut String, key: &str, value: impl fmt::Display) {
    output.push_str(key);
    output.push('\t');
    output.push_str(&value.to_string());
    output.push('\n');
}

fn text_field<'a>(lines: &'a [&str], index: usize, key: &str) -> Result<&'a str, FormatError> {
    let mut parts = lines
        .get(index)
        .ok_or_else(|| FormatError(format!("missing {key}")))?
        .split('\t');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(actual), Some(value), None) if actual == key => Ok(value),
        _ => Err(FormatError(format!("invalid {key} field"))),
    }
}

fn parse_field<T>(lines: &[&str], index: usize, key: &str) -> Result<T, FormatError>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    text_field(lines, index, key)?
        .parse()
        .map_err(|error: T::Err| FormatError(error.to_string()))
}

fn parse_number<T>(value: &str, label: &str) -> Result<T, FormatError>
where
    T: FromStr,
{
    value
        .parse()
        .map_err(|_| FormatError(format!("invalid {label}")))
}

fn model_error(error: impl fmt::Display) -> FormatError {
    FormatError(error.to_string())
}

pub fn receipt_sha256(canonical_receipt: &str) -> String {
    sha256_hex(canonical_receipt.as_bytes())
}

#[derive(Debug, Clone)]
pub struct CommittedBaseline {
    receipt: PerformanceReceipt,
    sha256: String,
}

impl CommittedBaseline {
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        let receipt = parse_receipt(bytes)?;
        if receipt.kind != ReceiptKind::Baseline {
            return Err(FormatError(
                "committed baseline has wrong receipt kind".into(),
            ));
        }
        let canonical =
            std::str::from_utf8(bytes).map_err(|_| FormatError("receipt is not UTF-8".into()))?;
        Ok(Self {
            receipt,
            sha256: receipt_sha256(canonical),
        })
    }

    pub fn receipt(&self) -> &PerformanceReceipt {
        &self.receipt
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}
