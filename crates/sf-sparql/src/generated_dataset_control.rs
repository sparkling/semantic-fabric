//! Bounded, redacted default-graph allowlist and dataset-clause admission.
//! Allowlist entries and query FROM occurrences are bounded independently;
//! request-side visits, byte scans and comparisons are prepaid on the control.

use std::fmt;

use sf_core::query_control::QueryControl;
use spargebra::algebra::QueryDataset;

use super::{refuse, ConstantCoverageError, DatasetRule, GeneratedDatasetError};
use crate::build::control::BuildWork;

/// Most allowlist entries, and most FROM occurrences in one query.
pub const MAX_DATASET_GRAPHS: usize = 64;
/// Most bytes in one graph IRI.
pub const MAX_GRAPH_IRI_BYTES: usize = 4096;
/// Most graph-IRI bytes in one allowlist, and in one query's FROM occurrences.
pub const MAX_DATASET_IRI_BYTES: usize = 65_536;

/// Redacted allowlist construction failure. Carries no IRI.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DatasetAllowlistError {
    #[error("dataset allowlist exceeds its entry bound")]
    TooManyGraphs,
    #[error("dataset allowlist graph IRI exceeds its byte bound")]
    GraphIriTooLong,
    #[error("dataset allowlist exceeds its total byte bound")]
    TotalBytesExceeded,
    #[error("dataset allowlist graph is not an absolute IRI")]
    InvalidGraphIri,
    #[error("dataset allowlist allocation failed")]
    AllocationFailed,
}

/// Embedding-supplied default graphs a generated query may select. Membership
/// is exact IRI string equality. `Debug` reports only the entry count.
#[derive(Clone)]
pub struct DatasetGraphAllowlist {
    graphs: Vec<Box<str>>,
}

impl DatasetGraphAllowlist {
    /// Validate and retain a bounded set of absolute graph IRIs.
    pub fn new<I>(graphs: I) -> Result<Self, DatasetAllowlistError>
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let mut out: Vec<Box<str>> = Vec::new();
        let mut total = 0_usize;
        for entry in graphs {
            let graph = entry.as_ref();
            if out.len() == MAX_DATASET_GRAPHS {
                return Err(DatasetAllowlistError::TooManyGraphs);
            }
            if graph.len() > MAX_GRAPH_IRI_BYTES {
                return Err(DatasetAllowlistError::GraphIriTooLong);
            }
            total += graph.len();
            if total > MAX_DATASET_IRI_BYTES {
                return Err(DatasetAllowlistError::TotalBytesExceeded);
            }
            if !is_absolute_iri(graph) {
                return Err(DatasetAllowlistError::InvalidGraphIri);
            }
            out.try_reserve(1)
                .map_err(|_| DatasetAllowlistError::AllocationFailed)?;
            out.push(Box::from(graph));
        }
        Ok(Self { graphs: out })
    }

    pub fn len(&self) -> usize {
        self.graphs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.graphs.is_empty()
    }

    fn contains(&self, iri: &str, work: BuildWork<'_>) -> Result<bool, GeneratedDatasetError> {
        work.charge(1)?;
        for entry in &self.graphs {
            if same(work, entry, iri)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl fmt::Debug for DatasetGraphAllowlist {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatasetGraphAllowlist")
            .field("entries", &self.graphs.len())
            .finish_non_exhaustive()
    }
}

/// Structural dataset screen before any callback: no FROM NAMED, one to
/// `MAX_DATASET_GRAPHS` bounded absolute IRIs, exactly one distinct graph.
pub(super) fn screen_dataset(
    dataset: &QueryDataset,
    work: BuildWork<'_>,
) -> Result<(), GeneratedDatasetError> {
    work.charge(1)?;
    if dataset
        .named
        .as_ref()
        .is_some_and(|named| !named.is_empty())
    {
        return refuse(DatasetRule::NamedGraphDataset);
    }
    let Some(first) = dataset.default.first() else {
        return refuse(DatasetRule::MissingDataset);
    };
    if dataset.default.len() > MAX_DATASET_GRAPHS {
        return refuse(DatasetRule::DatasetBounds);
    }
    work.charge(dataset.default.len())?;
    let mut total = 0_usize;
    for graph in &dataset.default {
        let iri = graph.as_str();
        if iri.len() > MAX_GRAPH_IRI_BYTES {
            return refuse(DatasetRule::DatasetBounds);
        }
        total += iri.len();
        if total > MAX_DATASET_IRI_BYTES {
            return refuse(DatasetRule::DatasetBounds);
        }
        work.charge(iri.len())?;
        if !is_absolute_iri(iri) {
            return refuse(DatasetRule::InvalidGraphIri);
        }
    }
    for graph in &dataset.default {
        if !same(work, first.as_str(), graph.as_str())? {
            return refuse(DatasetRule::MultipleDefaultGraphs);
        }
    }
    work.checkpoint()?;
    Ok(())
}

/// Admit every FROM occurrence, repeated ones included: fresh allowlist
/// membership, then the caller's graph check. Not-allowlisted and uncovered
/// share one rule so a refusal never discloses allowlist membership. A stopped
/// request stays a typed control error, never a denial.
pub(super) fn admit_occurrences<G>(
    dataset: &QueryDataset,
    allowlist: &DatasetGraphAllowlist,
    control: &dyn QueryControl,
    work: BuildWork<'_>,
    graph_check: &mut G,
) -> Result<(), GeneratedDatasetError>
where
    G: FnMut(&str) -> Result<(), ConstantCoverageError>,
{
    for graph in &dataset.default {
        let iri = graph.as_str();
        if !allowlist.contains(iri, work)? {
            return refuse(DatasetRule::GraphNotAdmitted);
        }
        control.checkpoint().map_err(crate::Error::from)?;
        let verdict = graph_check(iri);
        control.checkpoint().map_err(crate::Error::from)?;
        match verdict {
            Ok(()) => {}
            Err(ConstantCoverageError::Uncovered) => return refuse(DatasetRule::GraphNotAdmitted),
            Err(ConstantCoverageError::Control(cause)) => {
                return Err(crate::Error::QueryControl(control.terminate(cause)).into())
            }
        }
    }
    Ok(())
}

fn same(work: BuildWork<'_>, left: &str, right: &str) -> Result<bool, GeneratedDatasetError> {
    work.charge(1)?;
    if left.len() != right.len() {
        return Ok(false);
    }
    work.charge(left.len())?;
    Ok(left == right)
}

fn is_absolute_iri(value: &str) -> bool {
    oxiri::Iri::parse(value).is_ok()
}
