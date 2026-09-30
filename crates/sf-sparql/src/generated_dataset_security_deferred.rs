//! Deferred-refusal single-default dataset admission over the security cache.
//!
//! A wrong policy snapshot fails before any checkpoint, parse or callback. An
//! admitted query takes the unchanged policy/subject/attributes-partitioned
//! cache path. A refusal is data: through the shared `defer_dataset_refusal`,
//! the original parsed query (scoped to its requested graph after a screened
//! denial, lowered exactly as parsed after any other parsed refusal, absent
//! only after a syntax refusal) is lowered once, uncached, bypassing both the
//! security and the unscoped cache, as row-authorization input ONLY. The row
//! authorizer must consume that plan before the refusal is exposed; it is never
//! executed, cached or admitted and mints no identity. Admission and control
//! failures keep the `Dataset` partition; compile failures, including refusal
//! lowering, keep `Security`.

use sf_core::query_control::QueryControl;
use sf_core::security_context::SecurityContext;

use super::super::{SecurityCompileError, SecurityScopedCompiler};
use super::SecurityDatasetCompileError;
use crate::cache::generated::{
    parse_and_admit_single_default_dataset, ConstantCoverageError, ConstantOccurrence,
    DatasetAdmission, DatasetGraphAllowlist, GeneratedDatasetDeferred, GeneratedDatasetError,
};

impl SecurityScopedCompiler<'_> {
    /// Deferred-refusal sibling of
    /// [`Self::compile_shared_with_single_default_dataset`]. Parses once; both
    /// callbacks rerun before every cold or warm security-cache lookup.
    pub fn compile_shared_with_single_default_dataset_deferred<G, F>(
        &self,
        context: &SecurityContext,
        sparql: &str,
        allowlist: &DatasetGraphAllowlist,
        control: &dyn QueryControl,
        graph_check: G,
        constant_check: F,
    ) -> Result<GeneratedDatasetDeferred, SecurityDatasetCompileError>
    where
        G: FnMut(&str) -> Result<(), ConstantCoverageError>,
        F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
    {
        if !context.matches_policy_snapshot(self.expected_policy) {
            return Err(SecurityCompileError::PolicyMismatch.into());
        }
        let admission = parse_and_admit_single_default_dataset(
            sparql,
            allowlist,
            control,
            graph_check,
            constant_check,
        )
        .map_err(GeneratedDatasetError::Compiler)?;
        match admission {
            DatasetAdmission::Admitted(query) => {
                let plan = self.compile_parsed_impl(context, &query, Some(control))?;
                Ok(GeneratedDatasetDeferred::Admitted(plan))
            }
            DatasetAdmission::Refused { refusal, query } => {
                let deferred = self
                    .binding
                    .defer_dataset_refusal(refusal, query, control)
                    .map_err(SecurityCompileError::Compiler)?;
                Ok(deferred)
            }
        }
    }
}

#[cfg(test)]
#[path = "generated_dataset_security_deferred_tests.rs"]
mod tests;

#[cfg(test)]
mod test_support {
    use std::num::NonZeroUsize;
    use std::sync::Arc;

    use sf_core::ir::{ObjectMap, TriplesMap};
    use sf_core::query_control::{
        QueryBudget, QueryCharge, QueryControl, QueryControlError as Stop, QueryLimits,
        UncontrolledQueryControl,
    };
    use sf_core::security_context::{
        PolicySnapshotId, RequestAttributesIdentity, SecurityContext, SubjectIdentity,
    };
    use sf_core::{SourceId, SourceMapping};
    use sf_sql::Dialect;

    use crate::cache::generated::{
        ConstantCoverageError, ConstantOccurrence, DatasetGraphAllowlist, DatasetRule,
        GeneratedDatasetDeferred, GeneratedDatasetError,
    };
    use crate::cache::{SecurityCompileError, SecurityDatasetCompileError as E, SecurityPlanCache};
    use crate::{CompilerBinding, Epoch, Error, Plan, Tbox};

    pub(super) type Verdict = Result<(), ConstantCoverageError>;

    pub(super) const A: &str = "http://ex/A";
    pub(super) const B: &str = "http://ex/B";
    /// Base table mapped only into graph A.
    pub(super) const TABLE_A: &str = "graph_a_rows";
    /// Base table mapped only into graph B.
    pub(super) const TABLE_B: &str = "graph_b_rows";
    pub(super) const SELECT: &str = "SELECT ?o FROM <http://ex/A> WHERE { ?s <http://ex/p> ?o }";
    pub(super) const ASK: &str = "ASK FROM <http://ex/A> { ?s <http://ex/p> ?o }";
    pub(super) const MISSING: &str = "SELECT * WHERE {}";
    pub(super) const NAMED: &str = "SELECT * FROM NAMED <http://secret.invalid/g> WHERE {}";
    pub(super) const MULTIPLE: &str =
        "SELECT ?o FROM <http://ex/A> FROM <http://ex/B> WHERE { ?s <http://ex/p> ?o }";
    pub(super) const MULTIPLE_REF: &str =
        "SELECT ?o FROM <http://ex/A> FROM <http://ex/B> WHERE { ?s <http://ex/ref> ?o }";
    pub(super) const CONSTRUCT: &str =
        "CONSTRUCT { ?s <http://ex/p> ?o } FROM <http://ex/A> WHERE { ?s <http://ex/p> ?o }";
    pub(super) const UPDATE: &str = "INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }";
    pub(super) const REF: &str = "SELECT ?o FROM <http://ex/A> WHERE { ?s <http://ex/ref> ?o }";
    pub(super) const FREE: &UncontrolledQueryControl = &UncontrolledQueryControl;
    pub(super) const OK: Verdict = Ok(());
    pub(super) const DENY: Verdict = Err(ConstantCoverageError::Uncovered);

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

    pub(super) fn policy(value: u8) -> PolicySnapshotId {
        PolicySnapshotId::from_digest([value; 32]).unwrap()
    }

    pub(super) fn context(p: u8, s: u8, a: u8) -> SecurityContext {
        SecurityContext::new(
            policy(p),
            SubjectIdentity::from_digest([s; 32]).unwrap(),
            RequestAttributesIdentity::from_digest([a; 32]).unwrap(),
        )
    }

    pub(super) fn budget(work: u64) -> QueryBudget {
        QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
    }

    pub(super) fn work(control: &QueryBudget) -> u64 {
        control.consumed(QueryCharge::CompilerWork)
    }

    pub(super) fn cache() -> SecurityPlanCache {
        SecurityPlanCache::new(NonZeroUsize::new(8).unwrap())
    }

    pub(super) fn allow() -> DatasetGraphAllowlist {
        DatasetGraphAllowlist::new([A, B]).unwrap()
    }

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

    pub(super) fn pass_graph(_: &str) -> Verdict {
        OK
    }

    pub(super) fn pass_constant(_: ConstantOccurrence<'_>) -> Verdict {
        OK
    }

    /// The admitted uncached plan: the oracle for a scoped refusal plan.
    pub(super) fn scoped(binding: &CompilerBinding, query: &str) -> Arc<Plan> {
        let admitted = binding.compile_uncached_shared_with_single_default_dataset(
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

    pub(super) fn run<G, F>(
        binding: &CompilerBinding,
        cache: &SecurityPlanCache,
        who: &SecurityContext,
        query: &str,
        control: &dyn QueryControl,
        graph: G,
        constant: F,
    ) -> Result<GeneratedDatasetDeferred, E>
    where
        G: FnMut(&str) -> Verdict,
        F: FnMut(ConstantOccurrence<'_>) -> Verdict,
    {
        let compiler = binding.for_security_policy(who.policy_snapshot(), cache);
        compiler.compile_shared_with_single_default_dataset_deferred(
            who,
            query,
            &allow(),
            control,
            graph,
            constant,
        )
    }

    pub(super) fn existing<G, F>(
        binding: &CompilerBinding,
        cache: &SecurityPlanCache,
        who: &SecurityContext,
        query: &str,
        control: &dyn QueryControl,
        graph: G,
        constant: F,
    ) -> Result<Arc<Plan>, E>
    where
        G: FnMut(&str) -> Verdict,
        F: FnMut(ConstantOccurrence<'_>) -> Verdict,
    {
        let compiler = binding.for_security_policy(who.policy_snapshot(), cache);
        compiler.compile_shared_with_single_default_dataset(
            who,
            query,
            &allow(),
            control,
            graph,
            constant,
        )
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

    pub(super) fn rule_of(error: &E) -> Option<DatasetRule> {
        match error {
            E::Dataset(GeneratedDatasetError::Refused(rule)) => Some(*rule),
            _ => None,
        }
    }

    pub(super) fn cause(error: &E) -> Option<Stop> {
        match error {
            E::Dataset(GeneratedDatasetError::Compiler(Error::QueryControl(cause)))
            | E::Security(SecurityCompileError::Compiler(Error::QueryControl(cause))) => {
                Some(*cause)
            }
            _ => None,
        }
    }
}
