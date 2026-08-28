use std::fmt;
use std::str::FromStr;

use super::stats::{summarize, SampleSummary};

pub const M0_SAMPLE_COUNT: usize = 50;
pub const MAX_SCENARIOS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelError(pub String);

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ModelError {}

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $value),+ }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = ModelError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    _ => Err(ModelError(format!("unsupported {}: {value}", stringify!($name)))),
                }
            }
        }
    };
}

string_enum!(MetricId {
    LatencyFullResult => "latency.full-result",
    LatencyFirstResult => "latency.first-result",
    HeapRustRequestedLiveDelta => "heap.rust-requested-live-delta",
    RssLinuxProcessPeak => "rss.linux-process-peak",
});

string_enum!(BoundaryId {
    SqliteParseTranslateExecuteCollect => "sqlite.parse-translate-execute-collect",
    SqliteExecuteToFirstTriple => "sqlite.execute-to-first-triple",
    SqliteParseTranslateExecuteDiscard => "sqlite.parse-translate-execute-discard",
    LinuxFreshProcessLifetime => "linux.fresh-process-lifetime",
    HttpPostgresRequestResponse => "http.postgres-request-response",
});

string_enum!(Unit {
    Nanoseconds => "ns",
    Bytes => "bytes",
});

string_enum!(ReceiptKind {
    Baseline => "baseline",
    Candidate => "candidate",
});

string_enum!(SourceTree {
    Clean => "clean",
    Dirty => "dirty",
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerBinding {
    pub profile_id: String,
    pub profile_sha256: String,
}

impl RunnerBinding {
    pub fn new(profile_id: &str, profile_sha256: &str) -> Result<Self, ModelError> {
        validate_id("runner profile", profile_id)?;
        validate_sha256("runner profile", profile_sha256)?;
        Ok(Self {
            profile_id: profile_id.to_owned(),
            profile_sha256: profile_sha256.to_owned(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceBinding {
    pub commit: String,
    pub tree: SourceTree,
    pub artifact_sha256: String,
    pub workload_sha256: String,
}

impl SourceBinding {
    pub fn new(
        commit: &str,
        tree: SourceTree,
        artifact_sha256: &str,
        workload_sha256: &str,
    ) -> Result<Self, ModelError> {
        if !matches!(commit.len(), 40 | 64) || !is_lower_hex(commit) {
            return Err(ModelError(
                "source commit must be 40 or 64 lowercase hex characters".into(),
            ));
        }
        validate_sha256("artifact", artifact_sha256)?;
        validate_sha256("workload", workload_sha256)?;
        Ok(Self {
            commit: commit.to_owned(),
            tree,
            artifact_sha256: artifact_sha256.to_owned(),
            workload_sha256: workload_sha256.to_owned(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioConfig {
    pub id: String,
    pub scale: u32,
    pub metric: MetricId,
    pub boundary: BoundaryId,
    pub unit: Unit,
    pub warmup_count: u16,
    pub sample_count: usize,
}

impl ScenarioConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: &str,
        scale: u32,
        metric: MetricId,
        boundary: BoundaryId,
        unit: Unit,
        warmup_count: u16,
        sample_count: usize,
    ) -> Result<Self, ModelError> {
        validate_id("scenario", id)?;
        if scale == 0 {
            return Err(ModelError("scenario scale must be positive".into()));
        }
        if sample_count != M0_SAMPLE_COUNT {
            return Err(ModelError(format!(
                "M0 scenarios require exactly {M0_SAMPLE_COUNT} samples"
            )));
        }
        let expected_unit = match metric {
            MetricId::LatencyFullResult | MetricId::LatencyFirstResult => Unit::Nanoseconds,
            MetricId::HeapRustRequestedLiveDelta | MetricId::RssLinuxProcessPeak => Unit::Bytes,
        };
        if unit != expected_unit {
            return Err(ModelError(format!(
                "metric {metric} requires unit {expected_unit}"
            )));
        }
        let boundary_matches = match metric {
            MetricId::LatencyFullResult => matches!(
                boundary,
                BoundaryId::SqliteParseTranslateExecuteCollect
                    | BoundaryId::SqliteParseTranslateExecuteDiscard
                    | BoundaryId::HttpPostgresRequestResponse
            ),
            MetricId::LatencyFirstResult => boundary == BoundaryId::SqliteExecuteToFirstTriple,
            MetricId::HeapRustRequestedLiveDelta => {
                boundary == BoundaryId::SqliteParseTranslateExecuteDiscard
            }
            MetricId::RssLinuxProcessPeak => boundary == BoundaryId::LinuxFreshProcessLifetime,
        };
        if !boundary_matches {
            return Err(ModelError(format!(
                "metric {metric} is incompatible with boundary {boundary}"
            )));
        }
        Ok(Self {
            id: id.to_owned(),
            scale,
            metric,
            boundary,
            unit,
            warmup_count,
            sample_count,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioObservation {
    pub config: ScenarioConfig,
    pub raw_samples: Vec<u64>,
    pub summary: SampleSummary,
}

impl ScenarioObservation {
    pub fn new(config: ScenarioConfig, raw_samples: Vec<u64>) -> Result<Self, ModelError> {
        if raw_samples.len() != config.sample_count {
            return Err(ModelError(format!(
                "scenario {} expected {} samples, got {}",
                config.id,
                config.sample_count,
                raw_samples.len()
            )));
        }
        let summary = summarize(&raw_samples).map_err(|error| ModelError(format!("{error:?}")))?;
        Ok(Self {
            config,
            raw_samples,
            summary,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerformanceReceipt {
    pub kind: ReceiptKind,
    pub runner: RunnerBinding,
    pub source: SourceBinding,
    pub observations: Vec<ScenarioObservation>,
}

impl PerformanceReceipt {
    pub fn new(
        kind: ReceiptKind,
        runner: RunnerBinding,
        source: SourceBinding,
        mut observations: Vec<ScenarioObservation>,
    ) -> Result<Self, ModelError> {
        if observations.is_empty() || observations.len() > MAX_SCENARIOS {
            return Err(ModelError(format!(
                "receipt scenario count must be between 1 and {MAX_SCENARIOS}"
            )));
        }
        observations.sort_by(|left, right| left.config.id.cmp(&right.config.id));
        for pair in observations.windows(2) {
            if pair[0].config.id == pair[1].config.id {
                return Err(ModelError(format!(
                    "duplicate scenario: {}",
                    pair[0].config.id
                )));
            }
        }
        Ok(Self {
            kind,
            runner,
            source,
            observations,
        })
    }
}

fn validate_id(label: &str, value: &str) -> Result<(), ModelError> {
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
    {
        return Err(ModelError(format!("invalid {label} id")));
    }
    Ok(())
}

fn validate_sha256(label: &str, value: &str) -> Result<(), ModelError> {
    if value.len() != 64 || !is_lower_hex(value) {
        return Err(ModelError(format!(
            "{label} digest must be 64 lowercase hex characters"
        )));
    }
    Ok(())
}

fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
