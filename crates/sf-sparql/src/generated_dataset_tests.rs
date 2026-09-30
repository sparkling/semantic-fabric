use std::cell::Cell;
use std::sync::Arc;

use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
    UncontrolledQueryControl,
};
use sf_core::{SourceId, SourceMapping};
use sf_sql::Dialect;
use spargebra::algebra::GraphPattern;
use spargebra::Query;

use super::super::test_support::{isolated, parse_spans};
use super::super::{
    ConstantCoverageError, ConstantOccurrence, ConstantRole, GeneratedCompileError,
    GeneratedDeferred, GeneratedQueryRefusal, ShapeRule,
};
use super::{
    admit, DatasetAllowlistError, DatasetGraphAllowlist, DatasetRule, GeneratedDatasetError,
    MAX_DATASET_GRAPHS, MAX_GRAPH_IRI_BYTES,
};
use crate::{CompilerBinding, Epoch, Error, Plan, Tbox};

type R = DatasetRule;
type Verdict = Result<(), ConstantCoverageError>;

const A: &str = "http://ex/A";
const B: &str = "http://ex/B";
const P: &str = "http://ex/p";
const Q: &str = "http://ex/q";
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const BGP: &str = "SELECT ?o FROM <G> WHERE { ?s <http://ex/p> ?o }";
const VALUES: &str = "SELECT ?x FROM <G> WHERE { VALUES ?x { 1 2 } }";
const FREE: &UncontrolledQueryControl = &UncontrolledQueryControl;
const OK: Verdict = Ok(());
const DENY: Verdict = Err(ConstantCoverageError::Uncovered);
const CANCELLED: QueryControlError = QueryControlError::Cancelled;
const DEADLINE: QueryControlError = QueryControlError::DeadlineExceeded;
const EXCEEDED: QueryControlError = QueryControlError::CompilerWorkExceeded;

const REFUSED: &[(&str, DatasetRule)] = &[
    ("SELECT ?o WHERE { ?s <http://ex/p> ?o }", R::MissingDataset),
    ("SELECT ?o FROM NAMED <G> WHERE { ?s <http://ex/p> ?o }", R::NamedGraphDataset),
    ("SELECT ?o FROM <G> FROM NAMED <G> WHERE { ?s <http://ex/p> ?o }", R::NamedGraphDataset),
    ("SELECT ?o FROM <G> FROM <http://ex/B> WHERE { ?s <http://ex/p> ?o }", R::MultipleDefaultGraphs),
    ("CONSTRUCT { ?s <http://ex/p> ?o } FROM <G> WHERE { ?s <http://ex/p> ?o }", R::QueryForm),
    ("DESCRIBE <http://ex/x> FROM <G>", R::QueryForm),
    ("INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }", R::FormNotAdmitted),
    ("not sparql", R::FormNotAdmitted),
    ("SELECT ?o FROM <G> WHERE { GRAPH <G> { ?s <http://ex/p> ?o } }", R::AuthoredGraph),
    ("SELECT ?o FROM <G> WHERE { GRAPH ?g { ?s <http://ex/p> ?o } }", R::AuthoredGraph),
    ("ASK FROM <G> { SERVICE <http://ex/s> { ?s ?p ?o } }", R::Service),
    ("SELECT ?o FROM <G> WHERE { ?s <http://ex/p>+ ?o }", R::PropertyPath),
    ("SELECT (COUNT(*) AS ?c) FROM <G> WHERE { ?s <http://ex/p> ?o }", R::Aggregate),
    ("SELECT ?s FROM <G> WHERE { ?s <http://ex/p> ?o } GROUP BY ?s", R::Aggregate),
    ("SELECT ?z FROM <G> WHERE { ?s <http://ex/p> ?o BIND(?o AS ?z) }", R::Bind),
    ("SELECT ?o FROM <G> WHERE { ?s <http://ex/p> ?o FILTER EXISTS { ?s <http://ex/q> ?v } }", R::Exists),
    ("SELECT ?o FROM <G> WHERE { ?s <http://ex/p> ?o FILTER NOT EXISTS { ?s <http://ex/q> ?v } }", R::Exists),
    ("SELECT ?o FROM <G> WHERE { ?s <http://ex/p> ?o MINUS { ?s <http://ex/q> ?v } }", R::Minus),
    ("SELECT ?o FROM <G> WHERE { ?s <http://ex/p> ?o } ORDER BY ?o", R::OrderBy),
    ("SELECT ?o FROM <G> WHERE { { SELECT ?o WHERE { ?s <http://ex/p> ?o } } }", R::NestedModifier),
    ("SELECT ?o FROM <G> WHERE { ?s <http://ex/p> ?o OPTIONAL { ?s <http://ex/q> ?v FILTER(?v = 'x') } }", R::OptionalShape),
    ("SELECT ?o FROM <G> WHERE { ?s <http://ex/p> ?o OPTIONAL { { ?s <http://ex/q> ?v } UNION { ?s <http://ex/r> ?v } } }", R::OptionalShape),
    ("SELECT ?o FROM <G> WHERE { ?s <http://ex/p> ?o FILTER(REGEX(?o, 'a')) }", R::UnsupportedExpression),
    ("SELECT ?o FROM <G> WHERE { ?s <http://ex/p> ?o FILTER(?o + 1 > 2) }", R::UnsupportedExpression),
];

#[derive(Clone, Copy)]
enum Mode {
    Cached,
    Uncached,
}

const MODES: [Mode; 2] = [Mode::Cached, Mode::Uncached];

fn binding() -> CompilerBinding {
    CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), vec![]),
        Dialect::Sqlite,
        Tbox::default(),
        vec![],
        Epoch::default(),
        8,
    )
}

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

fn work(control: &QueryBudget) -> u64 {
    control.consumed(QueryCharge::CompilerWork)
}

fn at(query: &str, graph: &str) -> String {
    query.replace("<G>", &format!("<{graph}>"))
}

fn allow(graphs: &[&str]) -> DatasetGraphAllowlist {
    DatasetGraphAllowlist::new(graphs.iter().copied()).unwrap()
}

fn graphs<'a>(calls: &'a Cell<u32>, verdict: Verdict) -> impl FnMut(&str) -> Verdict + 'a {
    move |_| {
        calls.set(calls.get() + 1);
        verdict
    }
}

fn constants<'a>(
    calls: &'a Cell<u32>,
    verdict: Verdict,
) -> impl FnMut(ConstantOccurrence<'_>) -> Verdict + 'a {
    move |_| {
        calls.set(calls.get() + 1);
        verdict
    }
}

fn run<G, F>(
    mode: Mode,
    binding: &CompilerBinding,
    query: &str,
    allowlist: &DatasetGraphAllowlist,
    control: &dyn QueryControl,
    graph: G,
    constant: F,
) -> Result<Arc<Plan>, GeneratedDatasetError>
where
    G: FnMut(&str) -> Verdict,
    F: FnMut(ConstantOccurrence<'_>) -> Verdict,
{
    match mode {
        Mode::Cached => binding
            .compile_shared_with_single_default_dataset(query, allowlist, control, graph, constant),
        Mode::Uncached => binding.compile_uncached_shared_with_single_default_dataset(
            query, allowlist, control, graph, constant,
        ),
    }
}

fn open(
    mode: Mode,
    binding: &CompilerBinding,
    query: &str,
    allowlist: &DatasetGraphAllowlist,
    control: &dyn QueryControl,
) -> Result<Arc<Plan>, GeneratedDatasetError> {
    let graph = |_: &str| OK;
    let constant = |_: ConstantOccurrence<'_>| OK;
    run(mode, binding, query, allowlist, control, graph, constant)
}

fn rule_of(error: &GeneratedDatasetError) -> Option<DatasetRule> {
    match error {
        GeneratedDatasetError::Refused(rule) => Some(*rule),
        _ => None,
    }
}

fn cause_of(error: &GeneratedDatasetError) -> Option<QueryControlError> {
    match error {
        GeneratedDatasetError::Compiler(Error::QueryControl(cause)) => Some(*cause),
        _ => None,
    }
}

fn pattern(query: &Query) -> &GraphPattern {
    match query {
        Query::Select { pattern, .. }
        | Query::Ask { pattern, .. }
        | Query::Construct { pattern, .. }
        | Query::Describe { pattern, .. } => pattern,
    }
}

#[path = "generated_dataset_control_tests.rs"]
mod controls;
#[path = "generated_dataset_semantics_tests.rs"]
mod semantics;
