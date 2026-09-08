//! Bound compiler work control, semantic identity and redacted diagnostics.

use std::fmt;
use std::sync::Arc;

use sf_core::query_control::QueryControl;
use sf_core::SourceMapping;
use sf_sql::{Dialect, TableSchema};
use spargebra::Query;

use super::{CachedPlan, CompileProfileId, CompilerBinding, Epoch};
use crate::compiler_control::CompileContext;
use crate::compiler_schema::{ColumnTypeAuthority, CompilerSchema, ConstraintAuthority};
use crate::runtime_identity::{CompileDigests, SemanticIdentity};
use crate::{CompilerWorkMode, Error, Plan, Result, Tbox};

impl CompilerBinding {
    /// Carry request control into the already-metered normalization, lowering
    /// and nested-cascade operations on a cache miss. Parsing, key rendering,
    /// build/resolve and cache destruction are not fully governed by this seam.
    /// Cache identity and semantics are unchanged: this does not activate
    /// `GovernedV1`. A shared hit performs no recursive plan clone to charge.
    /// Measurement limits protect each performed clone, not whole-plan admission.
    pub fn compile_shared_with_work_control(
        &self,
        sparql: &str,
        control: &dyn QueryControl,
    ) -> Result<Arc<Plan>> {
        control.checkpoint()?;
        let query = crate::parse_query(sparql)?;
        self.compile_parsed_shared_with_work_control(&query, control)
    }

    /// Uncached counterpart for structural preflight. The request retains the
    /// same cumulative control for its later authoritative compilation.
    pub fn compile_uncached_shared_with_work_control(
        &self,
        sparql: &str,
        control: &dyn QueryControl,
    ) -> Result<Arc<Plan>> {
        control.checkpoint()?;
        let query = crate::parse_query(sparql)?;
        self.compile_parsed_uncached_shared_with_work_control(&query, control)
    }

    pub(crate) fn compile_parsed_shared_with_work_control(
        &self,
        query: &Query,
        control: &dyn QueryControl,
    ) -> Result<Arc<Plan>> {
        control.checkpoint()?;
        let profile = CompileProfileId::Uncontrolled;
        let key = super::plan_key_for_profile(query, self.scope(), profile);
        control.checkpoint()?;
        if let Some(cached) = self.cache().get(&key) {
            if cached.scope() != self.scope() || cached.profile() != profile {
                return Err(Error::Mapping(
                    "compiled-plan cache identity mismatch".into(),
                ));
            }
            control.checkpoint()?;
            return Ok(cached.shared_plan());
        }
        let plan = self.compile_parsed_uncached_shared_with_work_control(query, control)?;
        control.checkpoint()?;
        self.cache().put(
            key,
            CachedPlan::from_shared(self.scope(), profile, Arc::clone(&plan)),
        );
        control.checkpoint()?;
        Ok(plan)
    }

    pub(crate) fn compile_parsed_uncached_shared_with_work_control(
        &self,
        query: &Query,
        control: &dyn QueryControl,
    ) -> Result<Arc<Plan>> {
        control.checkpoint()?;
        let result = self.compile_parsed_with_work_mode(
            query,
            CompilerWorkMode::Metered(CompileContext::new(control)),
        );
        control.checkpoint()?;
        result
    }

    pub(super) fn compile_parsed_with_work_mode(
        &self,
        query: &Query,
        work: CompilerWorkMode<'_>,
    ) -> Result<Arc<Plan>> {
        crate::translate_tree_with_column_type_use(
            query,
            self.triples_maps(),
            self.tbox(),
            self.dialect(),
            self.schema(),
            self.column_type_use(),
            work,
        )
        .map(Arc::new)
    }

    /// Prove the constant mapping origin of every emitted solution for the
    /// initial on-demand lineage profile. This is a query/input proof, NOT an
    /// execution capability for an independently mutable `Plan`.
    ///
    /// Exactly one authored map, no referencing object maps, and nonempty
    /// mandatory BGPs (possibly joined/unioned/projected/deduplicated/sliced).
    /// Saturation preserves the authored map identity. Every witness therefore
    /// has this same origin, even where semantic dedup erases witness identity.
    /// Other shapes reject; never substitute a list of candidate maps.
    pub fn constant_mapping_origin(
        &self,
        sparql: &str,
        control: &dyn sf_core::query_control::QueryControl,
    ) -> crate::Result<String> {
        use sf_core::{ir::ObjectMap, query_control::QueryCharge};
        use spargebra::{algebra::GraphPattern, Query};
        let unsupported = || crate::Error::Unsupported("lineage profile is not admitted".into());
        control.checkpoint()?;
        let [map] = self.mapping.triples_maps() else {
            return Err(unsupported());
        };
        if map.id.is_empty()
            || map.id.len() > 1024
            || map
                .predicate_object_maps
                .iter()
                .flat_map(|pom| &pom.objects)
                .any(|object| matches!(object, ObjectMap::Ref(_)))
        {
            return Err(unsupported());
        }
        let query = crate::parse_query(sparql)?;
        let (pattern, mut work) = match &query {
            Query::Select {
                pattern,
                dataset: None,
                ..
            } => (pattern, 0),
            Query::Construct {
                pattern,
                dataset: None,
                template,
                ..
            } => {
                // Template size is part of lineage eligibility, not merely WHERE
                // algebra size. This is not a total parser/term-recursion bound.
                if template.len() > 256 {
                    return Err(unsupported());
                }
                control.consume(QueryCharge::CompilerWork, template.len() as u64)?;
                (pattern, template.len())
            }
            _ => return Err(unsupported()),
        };
        let mut pending = vec![pattern];
        while let Some(pattern) = pending.pop() {
            control.checkpoint()?;
            control.consume(QueryCharge::CompilerWork, 1)?;
            work += 1;
            if work > 256 {
                return Err(unsupported());
            }
            match pattern {
                GraphPattern::Bgp { patterns } if !patterns.is_empty() => {
                    work = work.saturating_add(patterns.len());
                    if work > 256 {
                        return Err(unsupported());
                    }
                    control.consume(QueryCharge::CompilerWork, patterns.len() as u64)?;
                }
                GraphPattern::Project { inner, .. }
                | GraphPattern::Distinct { inner }
                | GraphPattern::Reduced { inner }
                | GraphPattern::Slice { inner, .. } => pending.push(inner),
                GraphPattern::Join { left, right } | GraphPattern::Union { left, right } => {
                    pending.push(left);
                    pending.push(right);
                }
                _ => return Err(unsupported()),
            }
        }
        Ok(map.id.clone())
    }

    /// Build a binding with externally supplied semantic identity. The exact
    /// document and admission digests partition every plan/cache entry, but the
    /// caller remains responsible for enforcing its opaque admission boundary.
    pub fn from_observation_with_semantic_identity(
        mapping: SourceMapping,
        dialect: Dialect,
        tbox: Tbox,
        observed_schema: Vec<TableSchema>,
        epoch: Epoch,
        semantic: SemanticIdentity,
        cache_capacity: usize,
    ) -> Self {
        let constraint_authority = ConstraintAuthority::Unverified;
        let column_type_authority = ColumnTypeAuthority::Unverified;
        let digests = CompileDigests::from_inputs_with_semantic_identity(
            &mapping,
            &observed_schema,
            dialect,
            constraint_authority,
            column_type_authority,
            semantic,
        );
        let schema = CompilerSchema::from_unverified_observation(observed_schema);
        Self::from_parts(
            mapping,
            dialect,
            tbox,
            schema,
            epoch,
            digests,
            cache_capacity,
        )
    }
}

impl fmt::Debug for CompilerBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompilerBinding")
            .field("scope", &self.scope)
            .field("source_id", &self.source_id())
            .field("dialect", &self.dialect)
            .field("triples_map_count", &self.mapping.len())
            .field("schema", &self.schema)
            .field("tbox_empty", &self.tbox.is_empty())
            .field("cache_entries", &self.cache().len())
            .finish()
    }
}

#[cfg(test)]
mod work_tests {
    use super::*;
    use crate::iq::node::{IqCond, IqNode};
    use crate::plan_measure::clone_root::{measure_compiler_clone_root_v1, CompilerCloneRootV1};
    use sf_core::{
        query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits},
        SourceId,
    };

    pub(crate) const QUERY: &str =
        "SELECT ?x WHERE { VALUES ?x { 1 2 3 } FILTER EXISTS { VALUES ?inside { 7 } } }";

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

    fn clone_work() -> u64 {
        let Query::Select { pattern, .. } = crate::parse_query(QUERY).unwrap() else {
            panic!()
        };
        let IqNode::Construction { child, .. } = crate::build::build_tree(&pattern, None).unwrap()
        else {
            panic!()
        };
        let IqNode::Filter { cond, .. } = *child else {
            panic!()
        };
        let [IqCond::Exists(inner)] = cond.as_slice() else {
            panic!()
        };
        measure_compiler_clone_root_v1(CompilerCloneRootV1::IqNode(inner))
            .unwrap()
            .deep_clone_work
    }

    #[test]
    fn exact_clone_charge_rejects_failed_misses_and_shares_completed_hits() {
        let binding = binding();
        let work = clone_work();
        let short = budget(2 * work - 1);
        assert!(matches!(
            binding.compile_shared_with_work_control(QUERY, &short),
            Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
        ));
        assert_eq!(short.consumed(QueryCharge::CompilerWork), work);
        assert_eq!(binding.cache_len(), 0);
        let exact = budget(2 * work);
        let plan = binding
            .compile_shared_with_work_control(QUERY, &exact)
            .unwrap();
        assert_eq!(exact.consumed(QueryCharge::CompilerWork), 2 * work);
        assert_eq!(binding.cache_len(), 1);
        assert_eq!(
            format!("{plan:?}"),
            format!("{:?}", binding.compile_uncached_shared(QUERY).unwrap())
        );
        let hit_control = budget(0);
        let hit = binding
            .compile_shared_with_work_control(QUERY, &hit_control)
            .unwrap();
        assert!(Arc::ptr_eq(&plan, &hit));
        assert!(Arc::ptr_eq(&plan, &binding.compile_shared(QUERY).unwrap()));
        assert_eq!(hit_control.consumed(QueryCharge::CompilerWork), 0);
        hit_control.terminate(QueryControlError::Cancelled);
        assert!(matches!(
            binding.compile_shared_with_work_control(QUERY, &hit_control),
            Err(Error::QueryControl(QueryControlError::Cancelled))
        ));
    }

    #[test]
    fn uncached_preflight_charges_each_pass_without_populating_cache() {
        let binding = binding();
        let work = clone_work();
        for allowance in [4 * work - 1, 4 * work] {
            let control = budget(allowance);
            binding
                .compile_uncached_shared_with_work_control(QUERY, &control)
                .unwrap();
            let second = binding.compile_uncached_shared_with_work_control(QUERY, &control);
            assert_eq!(second.is_ok(), allowance == 4 * work);
            assert_eq!(
                control.consumed(QueryCharge::CompilerWork),
                if second.is_ok() { 4 * work } else { 3 * work }
            );
            assert_eq!(binding.cache_len(), 0);
        }
    }

    struct CancelAfterClone(QueryBudget);
    impl QueryControl for CancelAfterClone {
        fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
            self.0.checkpoint()
        }
        fn consume(
            &self,
            charge: QueryCharge,
            amount: u64,
        ) -> std::result::Result<(), QueryControlError> {
            self.0.consume(charge, amount)?;
            if charge == QueryCharge::CompilerWork && amount > 0 {
                self.0.terminate(QueryControlError::Cancelled);
            }
            Ok(())
        }
        fn terminate(&self, reason: QueryControlError) -> QueryControlError {
            self.0.terminate(reason)
        }
    }

    #[test]
    fn cancellation_between_clone_operations_prevents_cache_insertion() {
        let binding = binding();
        let control = CancelAfterClone(budget(u64::MAX));
        assert!(matches!(
            binding.compile_shared_with_work_control(QUERY, &control),
            Err(Error::QueryControl(QueryControlError::Cancelled))
        ));
        assert_eq!(control.0.consumed(QueryCharge::CompilerWork), clone_work());
        assert_eq!(binding.cache_len(), 0);
    }

    #[test]
    fn metered_cache_hits_still_check_scope_and_profile() {
        for wrong_profile in [false, true] {
            let binding = binding();
            let parsed = crate::parse_query(QUERY).unwrap();
            let plan = binding.compile_uncached_shared(QUERY).unwrap();
            let scope = if wrong_profile {
                binding.scope()
            } else {
                super::super::test_scope(SourceId::new(1).unwrap(), Dialect::Sqlite, Epoch(1))
            };
            let profile = if wrong_profile {
                CompileProfileId::GovernedV1
            } else {
                CompileProfileId::Uncontrolled
            };
            binding.cache().put(
                super::super::plan_key(&parsed, binding.scope()),
                CachedPlan::from_shared(scope, profile, plan),
            );
            assert!(matches!(
                binding.compile_shared_with_work_control(QUERY, &budget(0)),
                Err(Error::Mapping(_))
            ));
        }
    }

    struct CancelAfterInsertion<'a>(&'a CompilerBinding, QueryBudget);
    impl QueryControl for CancelAfterInsertion<'_> {
        fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
            if self.0.cache_len() != 0 {
                self.1.terminate(QueryControlError::Cancelled);
            }
            self.1.checkpoint()
        }
        fn consume(
            &self,
            charge: QueryCharge,
            amount: u64,
        ) -> std::result::Result<(), QueryControlError> {
            self.1.consume(charge, amount)
        }
        fn terminate(&self, reason: QueryControlError) -> QueryControlError {
            self.1.terminate(reason)
        }
    }

    #[test]
    fn cancellation_after_insertion_can_retain_only_the_completed_valid_plan() {
        let binding = binding();
        let control = CancelAfterInsertion(&binding, budget(u64::MAX));
        assert!(matches!(
            binding.compile_shared_with_work_control(QUERY, &control),
            Err(Error::QueryControl(QueryControlError::Cancelled))
        ));
        assert_eq!(binding.cache_len(), 1);
        let cached = binding
            .compile_shared_with_work_control(QUERY, &budget(0))
            .unwrap();
        assert_eq!(
            format!("{cached:?}"),
            format!("{:?}", binding.compile_uncached_shared(QUERY).unwrap())
        );
    }
}
