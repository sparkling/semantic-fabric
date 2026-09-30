//! Opt-in single-default-graph generated admission (an ADR-0056 prerequisite).
//!
//! A SELECT/ASK query whose dataset clause names exactly one distinct default
//! graph from a bounded, validated embedding allowlist is admitted and compiled
//! through the existing parsed entry points. Each call checkpoints, parses once
//! through `crate::parse_query` and validates the original algebra envelope
//! before any new recursion or copy. Dataset and pattern shape are screened
//! next; every unsupported construct is a named, redacted refusal before any
//! callback, key, cache lookup or lowering. Every FROM occurrence then passes
//! fresh allowlist membership and the caller's graph check, and the caller's
//! constant check runs over the original parsed patterns. Only then is the
//! dataset normalized to its one default graph and each nonempty BGP scoped to
//! that constant GRAPH; VALUES and empty BGPs stay unchanged. The normalized
//! query keys the existing cache, so graph choices never share an entry, and no
//! verdict is ever cached. Named/variable GRAPH and EXISTS remain later work.
//! The `_deferred` siblings share this one parse and report refusals as data,
//! with the original query kept for authorization-only lowering.
//! This is not mapping proof, row authority, receipt or issued identity.

use std::fmt;
use std::sync::Arc;

use sf_core::query_control::QueryControl;
use spargebra::algebra::{GraphPattern, QueryDataset};
use spargebra::term::NamedNode;
use spargebra::Query;

use super::{
    admit_parsed, ConstantCoverageError, ConstantOccurrence, GeneratedCompileError,
    GeneratedQueryRefusal, ShapeRule,
};
use crate::build::control::BuildWork;
use crate::cache::CompilerBinding;
use crate::compile_envelope::algebra::AlgebraEnvelopeV1;
use crate::compiler_control::CompileContext;
use crate::{CompilerWorkMode, Plan};

#[path = "generated_dataset_control.rs"]
mod dataset_control;
#[path = "generated_dataset_deferred.rs"]
mod deferred;
#[path = "generated_dataset_walk.rs"]
mod walk;

pub use dataset_control::{
    DatasetAllowlistError, DatasetGraphAllowlist, MAX_DATASET_GRAPHS, MAX_DATASET_IRI_BYTES,
    MAX_GRAPH_IRI_BYTES,
};
pub use deferred::GeneratedDatasetDeferred;

/// Named, redacted refusal rule. Carries no query text, IRI or allowlist entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DatasetRule {
    /// Input is not a parseable query.
    FormNotAdmitted,
    /// CONSTRUCT or DESCRIBE.
    QueryForm,
    MissingDataset,
    NamedGraphDataset,
    MultipleDefaultGraphs,
    DatasetBounds,
    InvalidGraphIri,
    /// Not allowlisted or refused by the graph check; the two are not distinguished.
    GraphNotAdmitted,
    ConstantCoverage,
    AuthoredGraph,
    Service,
    PropertyPath,
    Aggregate,
    Bind,
    Exists,
    Minus,
    OrderBy,
    NestedModifier,
    OptionalShape,
    UnsupportedExpression,
    RdfStar,
    Unclassified,
    /// A rule reported by the unchanged generated-query screen.
    Shape(ShapeRule),
}

impl DatasetRule {
    pub const fn code(self) -> &'static str {
        match self {
            Self::FormNotAdmitted => "form-not-admitted",
            Self::QueryForm => "query-form",
            Self::MissingDataset => "missing-dataset",
            Self::NamedGraphDataset => "named-graph-dataset",
            Self::MultipleDefaultGraphs => "multiple-default-graphs",
            Self::DatasetBounds => "dataset-bounds",
            Self::InvalidGraphIri => "invalid-graph-iri",
            Self::GraphNotAdmitted => "graph-not-admitted",
            Self::ConstantCoverage => "constant-coverage",
            Self::AuthoredGraph => "authored-graph",
            Self::Service => "service-in-pattern",
            Self::PropertyPath => "property-path",
            Self::Aggregate => "aggregate",
            Self::Bind => "bind",
            Self::Exists => "exists",
            Self::Minus => "minus",
            Self::OrderBy => "order-by",
            Self::NestedModifier => "nested-modifier",
            Self::OptionalShape => "optional-shape",
            Self::UnsupportedExpression => "unsupported-expression",
            Self::RdfStar => "rdf-star-unsupported",
            Self::Unclassified => "unclassified-form",
            Self::Shape(rule) => rule.code(),
        }
    }
}

impl fmt::Display for DatasetRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "generated-dataset admission refused: {}", self.code())
    }
}

impl std::error::Error for DatasetRule {}

/// Failure of single-default-graph admission. Refusals and compiler or
/// request-control failures stay distinct.
#[derive(Debug, thiserror::Error)]
pub enum GeneratedDatasetError {
    #[error(transparent)]
    Refused(#[from] DatasetRule),
    #[error(transparent)]
    Compiler(#[from] crate::Error),
}

fn refuse<T>(rule: DatasetRule) -> Result<T, GeneratedDatasetError> {
    Err(GeneratedDatasetError::Refused(rule))
}

fn dataset_slot(query: &mut Query) -> &mut Option<QueryDataset> {
    match query {
        Query::Select { dataset, .. }
        | Query::Construct { dataset, .. }
        | Query::Describe { dataset, .. }
        | Query::Ask { dataset, .. } => dataset,
    }
}

fn pattern_slot(query: &mut Query) -> &mut GraphPattern {
    match query {
        Query::Select { pattern, .. }
        | Query::Construct { pattern, .. }
        | Query::Describe { pattern, .. }
        | Query::Ask { pattern, .. } => pattern,
    }
}

fn from_constant(error: GeneratedCompileError) -> GeneratedDatasetError {
    match error {
        GeneratedCompileError::Refused(GeneratedQueryRefusal::CoverageRefused) => {
            DatasetRule::ConstantCoverage.into()
        }
        GeneratedCompileError::Refused(GeneratedQueryRefusal::Rule(rule)) => {
            DatasetRule::Shape(rule).into()
        }
        GeneratedCompileError::Compiler(error) => error.into(),
    }
}

/// The ORIGINAL parsed query a refusal may lower, uncached, solely as
/// row-authorization input. It is never admitted, cached or identified.
pub(crate) enum AuthorizationQuery {
    /// A syntax refusal parsed nothing, so nothing may be lowered.
    Absent,
    /// Any other structural refusal: the original exactly as parsed, dataset
    /// clause intact, lowered as ordinary compilation would lower it. It is
    /// never stripped or rewritten into an admissible-looking query.
    Original(Query),
    /// Dataset and pattern screens passed and a check refused the request: the
    /// original, restored exactly, still to be scoped to its requested graph.
    Scoped(Query),
}

/// Parse-stage result of dataset admission. A refusal keeps what one parse may
/// lower once for row authorization; nothing is normalized for it yet.
pub(crate) enum DatasetAdmission {
    Admitted(Query),
    Refused {
        refusal: DatasetRule,
        query: AuthorizationQuery,
    },
}

/// Checkpoint, parse once, screen, run every check, then normalize. Returns the
/// normalized query; no key, cache or lowering work has happened yet.
pub(crate) fn admit<G, F>(
    sparql: &str,
    allowlist: &DatasetGraphAllowlist,
    control: &dyn QueryControl,
    graph_check: G,
    constant_check: F,
) -> Result<Query, GeneratedDatasetError>
where
    G: FnMut(&str) -> Result<(), ConstantCoverageError>,
    F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
{
    match parse_and_admit_dataset(sparql, allowlist, control, graph_check, constant_check)? {
        DatasetAdmission::Admitted(query) => Ok(query),
        DatasetAdmission::Refused { refusal, .. } => refuse(refusal),
    }
}

/// [`admit`] reporting refusals as data. Control, resource, envelope and
/// compiler failures stay typed errors and never become refusals.
pub(crate) fn parse_and_admit_dataset<G, F>(
    sparql: &str,
    allowlist: &DatasetGraphAllowlist,
    control: &dyn QueryControl,
    graph_check: G,
    constant_check: F,
) -> crate::Result<DatasetAdmission>
where
    G: FnMut(&str) -> Result<(), ConstantCoverageError>,
    F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
{
    control.checkpoint()?;
    let parsed = crate::parse_query(sparql);
    // Parsing observes no control; a stop raised meanwhile outranks its outcome.
    control.checkpoint()?;
    let mut query = match parsed {
        Ok(query) => query,
        // Only a syntax failure is a form refusal; limit and control errors stay typed.
        Err(crate::Error::Parse(_)) => {
            let refusal = DatasetRule::FormNotAdmitted;
            let query = AuthorizationQuery::Absent;
            return Ok(DatasetAdmission::Refused { refusal, query });
        }
        Err(error) => return Err(error),
    };
    // The original envelope bounds every later recursion and copy below.
    AlgebraEnvelopeV1::validate_with_control(&query, control)?;
    let work = BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control)));
    match screen(&query, work) {
        Ok(()) => {}
        Err(GeneratedDatasetError::Refused(refusal)) => {
            // Kept exactly as parsed: a refusal is never made to look admissible.
            let query = AuthorizationQuery::Original(query);
            return Ok(DatasetAdmission::Refused { refusal, query });
        }
        Err(GeneratedDatasetError::Compiler(error)) => return Err(error),
    }
    let checked = check(
        &mut query,
        allowlist,
        control,
        work,
        graph_check,
        constant_check,
    );
    let graph = match checked {
        Ok(graph) => graph,
        Err(GeneratedDatasetError::Refused(refusal)) => {
            let query = AuthorizationQuery::Scoped(query);
            return Ok(DatasetAdmission::Refused { refusal, query });
        }
        Err(GeneratedDatasetError::Compiler(error)) => return Err(error),
    };
    match normalize(&mut query, &graph, work) {
        Ok(()) => {}
        // Unreachable after the screen. A partly rewritten query is no longer
        // the original, so no authorization plan may be derived from it.
        Err(GeneratedDatasetError::Refused(refusal)) => {
            let query = AuthorizationQuery::Absent;
            return Ok(DatasetAdmission::Refused { refusal, query });
        }
        Err(GeneratedDatasetError::Compiler(error)) => return Err(error),
    }
    AlgebraEnvelopeV1::validate_with_control(&query, control)?;
    control.checkpoint()?;
    Ok(DatasetAdmission::Admitted(query))
}

/// Structural screen of the original parsed query: form, dataset clause and
/// pattern shape, before any callback.
fn screen(query: &Query, work: BuildWork<'_>) -> Result<(), GeneratedDatasetError> {
    let (dataset, pattern) = match query {
        Query::Select {
            dataset, pattern, ..
        }
        | Query::Ask {
            dataset, pattern, ..
        } => (dataset.as_ref(), pattern),
        Query::Construct { .. } | Query::Describe { .. } => return refuse(DatasetRule::QueryForm),
    };
    let Some(dataset) = dataset else {
        return refuse(DatasetRule::MissingDataset);
    };
    dataset_control::screen_dataset(dataset, work)?;
    walk::screen(pattern, work)
}

/// Run every check over the screened original. Returns the selected graph;
/// the query is left exactly as parsed.
fn check<G, F>(
    query: &mut Query,
    allowlist: &DatasetGraphAllowlist,
    control: &dyn QueryControl,
    work: BuildWork<'_>,
    mut graph_check: G,
    constant_check: F,
) -> Result<NamedNode, GeneratedDatasetError>
where
    G: FnMut(&str) -> Result<(), ConstantCoverageError>,
    F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
{
    let Some(dataset) = query.dataset() else {
        return refuse(DatasetRule::MissingDataset);
    };
    dataset_control::admit_occurrences(dataset, allowlist, control, work, &mut graph_check)?;
    let graph = work.copied(&dataset.default[0])?;
    // The unchanged screen refuses any dataset clause, so the owned dataset is
    // moved out only for the constant check over the original patterns and is
    // restored before either outcome propagates.
    let taken = dataset_slot(query).take();
    let admitted = admit_parsed(query, control, constant_check);
    *dataset_slot(query) = taken;
    // A callback that stopped the request never becomes a denial or admission.
    control.checkpoint().map_err(crate::Error::from)?;
    admitted.map_err(from_constant)?;
    Ok(graph)
}

/// Deduplicate the admitted repeated FROM occurrences, keep named metadata
/// exactly, and scope each nonempty default-context BGP to the selected graph.
fn normalize(
    query: &mut Query,
    graph: &NamedNode,
    work: BuildWork<'_>,
) -> Result<(), GeneratedDatasetError> {
    work.charge(1)?;
    if let Some(dataset) = dataset_slot(query) {
        dataset.default.truncate(1);
    }
    let pattern = pattern_slot(query);
    let owned = std::mem::take(pattern);
    *pattern = walk::rewrite(owned, graph, work)?;
    work.checkpoint()?;
    Ok(())
}

/// Scope a screened, refused original to its one requested default graph with
/// the admitted path's bounded rewrite, then recheck the envelope. The result
/// is authorization-only lowering input: it keeps exactly the requested graph's
/// mapped dependencies and never admits, caches or identifies anything.
pub(crate) fn scope_refused(
    mut query: Query,
    control: &dyn QueryControl,
) -> crate::Result<Option<Query>> {
    let work = BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control)));
    let Some(first) = query.dataset().and_then(|dataset| dataset.default.first()) else {
        return Ok(None);
    };
    let graph = work.copied(first)?;
    match normalize(&mut query, &graph, work) {
        Ok(()) => {}
        // Unreachable after both screens; a partial rewrite is no plan input.
        Err(GeneratedDatasetError::Refused(_)) => return Ok(None),
        Err(GeneratedDatasetError::Compiler(error)) => return Err(error),
    }
    AlgebraEnvelopeV1::validate_with_control(&query, control)?;
    Ok(Some(query))
}

impl CompilerBinding {
    /// Cached compile of a SELECT/ASK query whose dataset selects one allowlisted
    /// default graph. `graph_check` sees each FROM occurrence's exact IRI and
    /// `constant_check` each original pattern constant; both, plus allowlist
    /// membership, rerun on every call before key construction and cache lookup,
    /// so a warm entry is returned only after fresh checks. Verdicts are never
    /// cached. The request control is used unwrapped.
    pub fn compile_shared_with_single_default_dataset<G, F>(
        &self,
        sparql: &str,
        allowlist: &DatasetGraphAllowlist,
        control: &dyn QueryControl,
        graph_check: G,
        constant_check: F,
    ) -> Result<Arc<Plan>, GeneratedDatasetError>
    where
        G: FnMut(&str) -> Result<(), ConstantCoverageError>,
        F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
    {
        let query = admit(sparql, allowlist, control, graph_check, constant_check)?;
        let plan = self.compile_parsed_shared_with_work_control(&query, control)?;
        Ok(plan)
    }

    /// Uncached counterpart of [`Self::compile_shared_with_single_default_dataset`].
    /// Reads and populates no cache.
    pub fn compile_uncached_shared_with_single_default_dataset<G, F>(
        &self,
        sparql: &str,
        allowlist: &DatasetGraphAllowlist,
        control: &dyn QueryControl,
        graph_check: G,
        constant_check: F,
    ) -> Result<Arc<Plan>, GeneratedDatasetError>
    where
        G: FnMut(&str) -> Result<(), ConstantCoverageError>,
        F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
    {
        let query = admit(sparql, allowlist, control, graph_check, constant_check)?;
        let plan = self.compile_parsed_uncached_shared_with_work_control(&query, control)?;
        Ok(plan)
    }
}

#[cfg(test)]
#[path = "generated_dataset_tests.rs"]
mod tests;
