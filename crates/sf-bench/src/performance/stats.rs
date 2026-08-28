use std::fmt;

pub const MIN_P95_SAMPLES: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExactMedian {
    doubled_ns: u128,
}

impl ExactMedian {
    pub fn doubled_ns(self) -> u128 {
        self.doubled_ns
    }
}

impl fmt::Display for ExactMedian {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let whole = self.doubled_ns / 2;
        if self.doubled_ns.is_multiple_of(2) {
            write!(f, "{whole}")
        } else {
            write!(f, "{whole}.5")
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleSummary {
    pub median: ExactMedian,
    pub p95: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsError {
    InsufficientSamples { actual: usize, minimum: usize },
}

pub fn summarize(raw: &[u64]) -> Result<SampleSummary, StatsError> {
    if raw.len() < MIN_P95_SAMPLES {
        return Err(StatsError::InsufficientSamples {
            actual: raw.len(),
            minimum: MIN_P95_SAMPLES,
        });
    }

    let mut sorted = raw.to_vec();
    sorted.sort_unstable();
    let middle = sorted.len() / 2;
    let doubled_ns = if sorted.len().is_multiple_of(2) {
        u128::from(sorted[middle - 1]) + u128::from(sorted[middle])
    } else {
        u128::from(sorted[middle]) * 2
    };
    let rank = (95 * sorted.len()).div_ceil(100);

    Ok(SampleSummary {
        median: ExactMedian { doubled_ns },
        p95: sorted[rank - 1],
    })
}
