//! Deferred-refusal siblings of single-default-graph dataset admission.
//!
//! Each call parses exactly once through the shared dataset admission. An
//! admitted query is the same normalized query the ordinary entries compile, on
//! the same cached or uncached path, so plans, keys and work are unchanged. A
//! refusal is returned as data next to an optional authorization-only plan,
//! lowered once, uncached, with no key, lookup or insertion:
//!
//! - a screened single default graph refused by allowlist membership, the graph
//!   check or the constant check: the owned original scoped to that requested
//!   graph by the admitted path's bounded rewrite, envelope rechecked, so the
//!   plan keeps exactly that graph's mapped tables. It is never admitted;
//! - any other parsed refusal (no dataset clause, or a dataset, form or pattern
//!   this profile refuses): the owned original exactly as parsed, dataset
//!   clause intact, never stripped or rewritten to look admissible. Its plan
//!   reads exactly what ordinary lowering of that query reads, which is not
//!   necessarily every table its FROM graphs map; no graph is added for it;
//! - a syntax refusal, including any UPDATE: nothing was parsed, so no plan.
//!
//! Only an `Unsupported` lowering becomes an absent plan; every control,
//! mapping, SQL or core failure stays a typed error.
//!
//! A row authorizer must consume the refusal plan BEFORE the refusal is exposed,
//! so a policy denial never reveals refusal detail. The plan is never executed,
//! cached or admitted and carries no identity. These are compiler seams only:
//! not source coverage, mapping proof, receipt or authority. A runtime caller
//! keeps its own receipt and mapping checks.

use std::fmt;
use std::sync::Arc;

use sf_core::query_control::QueryControl;

use super::super::{authorization_only, ConstantCoverageError, ConstantOccurrence};
use super::{
    parse_and_admit_dataset, scope_refused, AuthorizationQuery, DatasetAdmission,
    DatasetGraphAllowlist, DatasetRule,
};
use crate::cache::CompilerBinding;
use crate::Plan;

/// Outcome of deferred-refusal single-default dataset admission.
///
/// `Admitted` is exactly the plan the non-deferred sibling returns. `Refused`
/// carries the named rule and, when the refused query could be lowered, an
/// uncached plan that is row-authorization input ONLY: never execute it, cache
/// it or treat it as admitted.
pub enum GeneratedDatasetDeferred {
    Admitted(Arc<Plan>),
    Refused {
        refusal: DatasetRule,
        authorization_plan: Option<Arc<Plan>>,
    },
}

impl fmt::Debug for GeneratedDatasetDeferred {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::Refused { refusal, .. } = self else {
            return f.write_str("Admitted(<plan>)");
        };
        f.debug_struct("Refused")
            .field("refusal", refusal)
            .finish_non_exhaustive()
    }
}

impl CompilerBinding {
    /// Deferred-refusal sibling of [`Self::compile_shared_with_single_default_dataset`].
    ///
    /// Allowlist membership and both callbacks rerun before every cold or warm
    /// cache lookup. An admitted query takes the unchanged cached path; a refusal
    /// never reads or populates the cache.
    pub fn compile_shared_with_single_default_dataset_deferred<G, F>(
        &self,
        sparql: &str,
        allowlist: &DatasetGraphAllowlist,
        control: &dyn QueryControl,
        graph_check: G,
        constant_check: F,
    ) -> crate::Result<GeneratedDatasetDeferred>
    where
        G: FnMut(&str) -> Result<(), ConstantCoverageError>,
        F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
    {
        match parse_and_admit_dataset(sparql, allowlist, control, graph_check, constant_check)? {
            DatasetAdmission::Admitted(query) => {
                let plan = self.compile_parsed_shared_with_work_control(&query, control)?;
                Ok(GeneratedDatasetDeferred::Admitted(plan))
            }
            DatasetAdmission::Refused { refusal, query } => {
                self.defer_dataset_refusal(refusal, query, control)
            }
        }
    }

    /// Uncached counterpart of
    /// [`Self::compile_shared_with_single_default_dataset_deferred`]. Reads and
    /// populates no cache.
    pub fn compile_uncached_shared_with_single_default_dataset_deferred<G, F>(
        &self,
        sparql: &str,
        allowlist: &DatasetGraphAllowlist,
        control: &dyn QueryControl,
        graph_check: G,
        constant_check: F,
    ) -> crate::Result<GeneratedDatasetDeferred>
    where
        G: FnMut(&str) -> Result<(), ConstantCoverageError>,
        F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
    {
        match parse_and_admit_dataset(sparql, allowlist, control, graph_check, constant_check)? {
            DatasetAdmission::Admitted(query) => {
                let plan =
                    self.compile_parsed_uncached_shared_with_work_control(&query, control)?;
                Ok(GeneratedDatasetDeferred::Admitted(plan))
            }
            DatasetAdmission::Refused { refusal, query } => {
                self.defer_dataset_refusal(refusal, query, control)
            }
        }
    }

    /// Prepare the refused original, then lower it once, uncached, for row
    /// authorization only. Shared by the unscoped and security-scoped entries.
    pub(crate) fn defer_dataset_refusal(
        &self,
        refusal: DatasetRule,
        query: AuthorizationQuery,
        control: &dyn QueryControl,
    ) -> crate::Result<GeneratedDatasetDeferred> {
        let query = match query {
            AuthorizationQuery::Absent => None,
            AuthorizationQuery::Original(query) => Some(query),
            AuthorizationQuery::Scoped(query) => scope_refused(query, control)?,
        };
        let authorization_plan = match query {
            Some(query) => {
                let plan = self.compile_parsed_uncached_shared_with_work_control(&query, control);
                authorization_only(plan)?
            }
            None => None,
        };
        control.checkpoint()?;
        Ok(GeneratedDatasetDeferred::Refused {
            refusal,
            authorization_plan,
        })
    }
}

#[cfg(test)]
#[path = "generated_dataset_deferred_tests.rs"]
mod tests;

#[cfg(test)]
mod test_support {
    use std::cell::Cell;
    use std::sync::Arc;

    use sf_core::ir::{ObjectMap, TriplesMap};
    use sf_core::query_control::{
        QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
        UncontrolledQueryControl,
    };
    use sf_core::{SourceId, SourceMapping};
    use sf_sql::Dialect;

    use crate::cache::generated::{
        ConstantCoverageError, ConstantOccurrence, DatasetGraphAllowlist, DatasetRule,
        GeneratedDatasetDeferred, GeneratedDatasetError,
    };
    use crate::{CompilerBinding, Epoch, Error, Plan, Tbox};

    pub(super) type R = DatasetRule;
    pub(super) type Verdict = Result<(), ConstantCoverageError>;

    pub(super) const A: &str = "http://ex/A";
    pub(super) const B: &str = "http://ex/B";
    /// Base table mapped only into graph A.
    pub(super) const TABLE_A: &str = "graph_a_rows";
    /// Base table mapped only into graph B.
    pub(super) const TABLE_B: &str = "graph_b_rows";
    pub(super) const SELECT: &str = "SELECT ?o FROM <http://ex/A> WHERE { ?s <http://ex/p> ?o }";
    pub(super) const ASK: &str = "ASK FROM <http://ex/A> { ?s <http://ex/p> ?o }";
    pub(super) const VALUES: &str = "SELECT ?x FROM <http://ex/A> WHERE { VALUES ?x { 1 2 } }";
    pub(super) const MISSING: &str = "SELECT ?o WHERE { ?s <http://ex/p> ?o }";
    pub(super) const UPDATE: &str = "INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }";
    pub(super) const REF: &str = "SELECT ?o FROM <http://ex/A> WHERE { ?s <http://ex/ref> ?o }";
    pub(super) const FREE: &UncontrolledQueryControl = &UncontrolledQueryControl;
    pub(super) const OK: Verdict = Ok(());
    pub(super) const DENY: Verdict = Err(ConstantCoverageError::Uncovered);
    pub(super) const CANCELLED: QueryControlError = QueryControlError::Cancelled;
    pub(super) const DEADLINE: QueryControlError = QueryControlError::DeadlineExceeded;
    pub(super) const EXCEEDED: QueryControlError = QueryControlError::CompilerWorkExceeded;

    pub(super) const MAPPING: &str = r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        <http://ex/m> rr:logicalTable [rr:tableName "graph_a_rows"];
          rr:subjectMap [rr:template "http://ex/{s}";rr:graph <http://ex/A>];
          rr:predicateObjectMap [rr:predicate <http://ex/p>;rr:objectMap [rr:column "o"]].
        <http://ex/other> rr:logicalTable [rr:tableName "graph_b_rows"];
          rr:subjectMap [rr:template "http://ex/{s}";rr:graph <http://ex/B>];
          rr:predicateObjectMap [rr:predicate <http://ex/p>;rr:objectMap [rr:column "o"]].
    "#;

    /// The referencing child lives in graph A, so a scoped refusal plan for
    /// `FROM <http://ex/A>` still reaches its broken parent reference.
    pub(super) const REF_MAPPING: &str = r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        <http://ex/parent> rr:logicalTable [rr:tableName "parents"];
          rr:subjectMap [rr:template "http://ex/parent/{id}"];
          rr:predicateObjectMap [rr:predicate <http://ex/name>;rr:objectMap [rr:column "name"]].
        <http://ex/child> rr:logicalTable [rr:tableName "children"];
          rr:subjectMap [rr:template "http://ex/child/{id}";rr:graph <http://ex/A>];
          rr:predicateObjectMap [rr:predicate <http://ex/ref>;
            rr:objectMap [rr:parentTriplesMap <http://ex/parent>;
              rr:joinCondition [rr:child "parent_id";rr:parent "id"]]].
    "#;

    pub(super) const REFUSED: &[(&str, DatasetRule)] = &[
        ("SELECT ?o WHERE { ?s <http://ex/p> ?o }", R::MissingDataset),
        ("SELECT ?o FROM NAMED <G> WHERE { ?s <http://ex/p> ?o }", R::NamedGraphDataset),
        ("SELECT ?o FROM <G> FROM <http://ex/B> WHERE { ?s <http://ex/p> ?o }", R::MultipleDefaultGraphs),
        ("CONSTRUCT { ?s <http://ex/p> ?o } FROM <G> WHERE { ?s <http://ex/p> ?o }", R::QueryForm),
        ("DESCRIBE <http://ex/x> FROM <G>", R::QueryForm),
        ("INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }", R::FormNotAdmitted),
        ("not sparql", R::FormNotAdmitted),
        ("SELECT ?o FROM <G> WHERE { GRAPH <G> { ?s <http://ex/p> ?o } }", R::AuthoredGraph),
        ("ASK FROM <G> { SERVICE <http://ex/s> { ?s ?p ?o } }", R::Service),
        ("SELECT ?o FROM <G> WHERE { ?s <http://ex/p>+ ?o }", R::PropertyPath),
        ("SELECT (COUNT(*) AS ?c) FROM <G> WHERE { ?s <http://ex/p> ?o }", R::Aggregate),
        ("SELECT ?z FROM <G> WHERE { ?s <http://ex/p> ?o BIND(?o AS ?z) }", R::Bind),
        ("SELECT ?o FROM <G> WHERE { ?s <http://ex/p> ?o FILTER EXISTS { ?s <http://ex/q> ?v } }", R::Exists),
        ("SELECT ?o FROM <G> WHERE { ?s <http://ex/p> ?o MINUS { ?s <http://ex/q> ?v } }", R::Minus),
        ("SELECT ?o FROM <G> WHERE { ?s <http://ex/p> ?o } ORDER BY ?o", R::OrderBy),
        ("SELECT ?o FROM <G> WHERE { { SELECT ?o WHERE { ?s <http://ex/p> ?o } } }", R::NestedModifier),
        ("SELECT ?o FROM <G> WHERE { ?s <http://ex/p> ?o FILTER(REGEX(?o, 'a')) }", R::UnsupportedExpression),
    ];

    pub(super) fn bind(maps: Vec<TriplesMap>) -> CompilerBinding {
        CompilerBinding::from_unverified_observation(
            SourceMapping::new(SourceId::new(0).unwrap(), maps),
            Dialect::Sqlite,
            Tbox::default(),
            Vec::new(),
            Epoch::default(),
            8,
        )
    }

    pub(super) fn fixture() -> CompilerBinding {
        bind(sf_mapping::parse_r2rml(MAPPING).unwrap())
    }

    /// A real ref-object map whose parent reference names no map in the binding.
    pub(super) fn broken() -> CompilerBinding {
        let mut maps = sf_mapping::parse_r2rml(REF_MAPPING).unwrap();
        let mut renamed = 0;
        for map in &mut maps {
            for pom in &mut map.predicate_object_maps {
                for object in &mut pom.objects {
                    if let ObjectMap::Ref(reference) = object {
                        reference.parent_triples_map = "http://ex/missing".into();
                        renamed += 1;
                    }
                }
            }
        }
        assert_eq!(renamed, 1);
        bind(maps)
    }

    pub(super) fn budget(work: u64) -> QueryBudget {
        QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
    }

    pub(super) fn work(control: &QueryBudget) -> u64 {
        control.consumed(QueryCharge::CompilerWork)
    }

    pub(super) fn at(template: &str) -> String {
        template.replace("<G>", "<http://ex/A>")
    }

    pub(super) fn allow() -> DatasetGraphAllowlist {
        DatasetGraphAllowlist::new([A, B]).unwrap()
    }

    pub(super) fn pass_graph(_: &str) -> Verdict {
        OK
    }

    pub(super) fn pass_constant(_: ConstantOccurrence<'_>) -> Verdict {
        OK
    }

    pub(super) fn graphs<'a>(
        calls: &'a Cell<u32>,
        verdict: Verdict,
    ) -> impl FnMut(&str) -> Verdict + 'a {
        move |_| {
            calls.set(calls.get() + 1);
            verdict
        }
    }

    pub(super) fn constants<'a>(
        calls: &'a Cell<u32>,
        verdict: Verdict,
    ) -> impl FnMut(ConstantOccurrence<'_>) -> Verdict + 'a {
        move |_| {
            calls.set(calls.get() + 1);
            verdict
        }
    }

    pub(super) fn deferred<G, F>(
        cached: bool,
        binding: &CompilerBinding,
        query: &str,
        list: &DatasetGraphAllowlist,
        control: &dyn QueryControl,
        graph: G,
        constant: F,
    ) -> crate::Result<GeneratedDatasetDeferred>
    where
        G: FnMut(&str) -> Verdict,
        F: FnMut(ConstantOccurrence<'_>) -> Verdict,
    {
        if cached {
            binding.compile_shared_with_single_default_dataset_deferred(
                query, list, control, graph, constant,
            )
        } else {
            binding.compile_uncached_shared_with_single_default_dataset_deferred(
                query, list, control, graph, constant,
            )
        }
    }

    pub(super) fn ordinary<G, F>(
        cached: bool,
        binding: &CompilerBinding,
        query: &str,
        list: &DatasetGraphAllowlist,
        control: &dyn QueryControl,
        graph: G,
        constant: F,
    ) -> Result<Arc<Plan>, GeneratedDatasetError>
    where
        G: FnMut(&str) -> Verdict,
        F: FnMut(ConstantOccurrence<'_>) -> Verdict,
    {
        if cached {
            binding
                .compile_shared_with_single_default_dataset(query, list, control, graph, constant)
        } else {
            binding.compile_uncached_shared_with_single_default_dataset(
                query, list, control, graph, constant,
            )
        }
    }

    pub(super) fn open(
        cached: bool,
        binding: &CompilerBinding,
        query: &str,
        control: &dyn QueryControl,
    ) -> crate::Result<GeneratedDatasetDeferred> {
        deferred(
            cached,
            binding,
            query,
            &allow(),
            control,
            pass_graph,
            pass_constant,
        )
    }

    /// The admitted uncached plan: the oracle for a scoped refusal plan.
    pub(super) fn scoped(binding: &CompilerBinding, query: &str) -> Arc<Plan> {
        let admitted = ordinary(
            false,
            binding,
            query,
            &allow(),
            FREE,
            pass_graph,
            pass_constant,
        );
        admitted.unwrap()
    }

    /// Structural refusal-plan oracle: the ordinary uncached lowering of the
    /// original exactly as parsed. Only a syntax refusal has nothing to lower.
    pub(super) fn original(
        binding: &CompilerBinding,
        query: &str,
    ) -> Option<crate::Result<Arc<Plan>>> {
        crate::parse_query(query).ok()?;
        Some(binding.compile_uncached_shared_with_work_control(query, FREE))
    }

    /// Fixture base tables a plan reads, in fixture order. Read from the plan's
    /// rendering so a base scan behind a derived wrapper (for example the
    /// duplicate-safety DISTINCT wrap) is still observed.
    pub(super) fn tables(plan: &Plan) -> Vec<&'static str> {
        let rendered = format!("{plan:?}");
        [TABLE_A, TABLE_B]
            .into_iter()
            .filter(|table| rendered.contains(*table))
            .collect()
    }

    pub(super) fn admitted(outcome: GeneratedDatasetDeferred) -> Arc<Plan> {
        match outcome {
            GeneratedDatasetDeferred::Admitted(plan) => plan,
            GeneratedDatasetDeferred::Refused { .. } => panic!("expected admission"),
        }
    }

    pub(super) fn refused(outcome: GeneratedDatasetDeferred) -> (DatasetRule, Option<Arc<Plan>>) {
        match outcome {
            GeneratedDatasetDeferred::Refused {
                refusal,
                authorization_plan,
            } => (refusal, authorization_plan),
            GeneratedDatasetDeferred::Admitted(_) => panic!("expected refusal"),
        }
    }

    pub(super) fn rule_of(error: &GeneratedDatasetError) -> Option<DatasetRule> {
        match error {
            GeneratedDatasetError::Refused(rule) => Some(*rule),
            _ => None,
        }
    }

    pub(super) fn cause(error: &Error) -> Option<QueryControlError> {
        match error {
            Error::QueryControl(cause) => Some(*cause),
            _ => None,
        }
    }
}
